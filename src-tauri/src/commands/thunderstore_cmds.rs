//! Thunderstore catalog + install commands.

use std::path::PathBuf;
use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::database::ModDatabase;
use crate::thunderstore::{self, Community, InstallReport, Package};
use crate::{resolve_game, AppState, DeployGuard};

#[tauri::command]
pub async fn thunderstore_list_communities() -> Result<Vec<Community>, String> {
    thunderstore::list_communities().await
}

#[tauri::command]
pub async fn thunderstore_list_packages(community: String) -> Result<Vec<Package>, String> {
    thunderstore::list_packages(&community).await
}

/// Install a single Thunderstore package version (no dependency resolution).
/// For the closure variant see [`thunderstore_install_with_dependencies`].
///
/// This command takes its own `DeployGuard`. Internal callers that already
/// hold a guard (e.g. the dependency-closure installer) should use
/// [`install_one_no_guard`] instead so the deploy flag does not flicker
/// between iterations.
#[tauri::command]
pub async fn thunderstore_install_package(
    app: AppHandle,
    community: String,
    full_name: String,
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<InstallReport, String> {
    let _guard = DeployGuard::new(state.deploy_in_progress.clone(), app.clone());
    install_one_no_guard(
        state.db.clone(),
        &community,
        &full_name,
        &game_id,
        &bottle_name,
    )
    .await
}

/// Install one Thunderstore package version without creating a `DeployGuard`.
/// Caller must hold an outer guard for the duration of the install. Used by
/// the dep-closure installer to keep the deploy flag steady across the whole
/// operation.
async fn install_one_no_guard(
    db: Arc<ModDatabase>,
    community: &str,
    full_name: &str,
    game_id: &str,
    bottle_name: &str,
) -> Result<InstallReport, String> {
    let packages = thunderstore::list_packages(community).await?;
    let version = thunderstore::find_version(&packages, full_name)
        .ok_or_else(|| format!("version {} not found in {}", full_name, community))?
        .clone();

    let zip_path = thunderstore::download_version(community, &version).await?;

    let data_dir: PathBuf = {
        let (_bottle, game, _path) = resolve_game(game_id, bottle_name)?;
        game.data_dir.clone()
    };

    let install_full_name = full_name.to_string();
    let report = tokio::task::spawn_blocking(move || {
        thunderstore::install_version(&zip_path, &data_dir, &install_full_name)
    })
    .await
    .map_err(|e| format!("join: {e}"))??;

    // Register with the mod database so it appears on the Mods page.
    // Use the sanitized `mod_subdir` for the archive_name so the DB record
    // matches the actual cached zip filename on disk (sanitize_segment may
    // remap characters that aren't ASCII alphanumeric/`-`/`_`/`.`).
    let archive_name = format!("{}.zip", report.mod_subdir);
    let report_clone = report.clone();
    let version_number = version.version_number.clone();
    let db_game_id = game_id.to_string();
    let db_bottle_name = bottle_name.to_string();
    tokio::task::spawn_blocking(move || {
        db.add_mod(
            &db_game_id,
            &db_bottle_name,
            None,
            &report_clone.full_name,
            &version_number,
            &archive_name,
            &report_clone.installed_files,
        )
    })
    .await
    .map_err(|e| format!("join: {e}"))?
    .map_err(|e| format!("db: {e}"))?;

    Ok(report)
}

/// Install a package version plus its full dependency closure. Returns one
/// report per installed package, root last.
///
/// Holds a single outer `DeployGuard` for the whole closure so the deploy
/// flag does not flicker on each per-package iteration.
#[tauri::command]
pub async fn thunderstore_install_with_dependencies(
    app: AppHandle,
    community: String,
    full_name: String,
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<InstallReport>, String> {
    let packages = thunderstore::list_packages(&community).await?;
    let root = thunderstore::find_version(&packages, &full_name)
        .ok_or_else(|| format!("version {} not found in {}", full_name, community))?
        .clone();

    // Resolved deps, root last so BepInEx etc. install before the plugin that
    // depends on them.
    let mut ordered: Vec<String> = thunderstore::resolve_dependencies(&packages, &root)
        .into_iter()
        .map(|v| v.full_name.clone())
        .collect();
    ordered.push(root.full_name.clone());

    let _guard = DeployGuard::new(state.deploy_in_progress.clone(), app.clone());
    let db = state.db.clone();
    let mut reports = Vec::with_capacity(ordered.len());
    for name in ordered {
        let report =
            install_one_no_guard(db.clone(), &community, &name, &game_id, &bottle_name).await?;
        reports.push(report);
    }
    Ok(reports)
}
