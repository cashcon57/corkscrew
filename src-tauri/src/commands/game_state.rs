//! Game state management: locks, deploy journal, version pinning, rollback/snapshots, and directory cleaning.

use crate::bottles;
use crate::cleaner;
use crate::deploy_journal;
use crate::deployer;
use crate::game_lock;
use crate::games;
use crate::plugins;
use crate::rollback;
use crate::rollback::{ModSnapshot, ModVersion};
use crate::{AppState, auto_snapshot_before_destructive, resolve_game};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tauri::State;

// --- Known-but-uninstalled games ---

/// A game we know about (via the bundled Vortex game registry or the
/// curated Vortex extension index) but that isn't installed in any
/// detected bottle. Surfaced in the game-selector dropdown when the user
/// has opted in to "show uninstalled games", so users can preview which
/// titles Corkscrew would support if they installed them.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KnownUninstalledGame {
    pub game_id: String,
    pub name: String,
    pub nexus_slug: String,
    /// Where this entry came from. Useful for the frontend to differentiate
    /// "supported via plugin" from "supported via fetched Vortex extension".
    pub source: String,
}

#[tauri::command]
pub async fn list_known_uninstalled_games_cmd(
    _state: State<'_, AppState>,
) -> Result<Vec<KnownUninstalledGame>, String> {
    // Source 3: full NexusMods games catalog (via /v1/games.json). Async +
    // network — fetched here, then merged with the bundled sources inside
    // the blocking task. Falls back to a stale cache, then to None when the
    // user is signed out and the cache is empty/expired.
    let nm_games = match crate::nexus_games_index::get_games().await {
        Ok(v) => Some(v),
        Err(e) => {
            log::warn!("nexus_games_index::get_games failed: {e}; falling back to bundled sources");
            crate::nexus_games_index::load_stale()
        }
    };

    tokio::task::spawn_blocking(move || {
        // Build the set of (game_id, nexus_slug) that are currently installed
        // in *any* detected bottle. We dedupe on both keys so an installed
        // game never reappears under "uninstalled" because of a slug mismatch.
        let bottles_list = bottles::detect_bottles();
        let mut installed_game_ids: HashSet<String> = HashSet::new();
        let mut installed_nexus_slugs: HashSet<String> = HashSet::new();
        for b in &bottles_list {
            for g in games::detect_games(b) {
                installed_game_ids.insert(g.game_id.clone());
                if !g.nexus_slug.is_empty() {
                    installed_nexus_slugs.insert(g.nexus_slug.clone());
                }
            }
        }

        let mut out: Vec<KnownUninstalledGame> = Vec::new();
        let mut seen_slugs: HashSet<String> = HashSet::new();

        // Source 1: bundled Vortex game registry (the 85-game catalog).
        for entry in crate::game_registry::all_game_entries() {
            // Skip stub entries — they're listed but not actually supported.
            if entry.note.is_some() {
                continue;
            }
            if installed_game_ids.contains(&entry.game_id)
                || installed_nexus_slugs.contains(&entry.nexus_domain)
            {
                continue;
            }
            if !seen_slugs.insert(entry.nexus_domain.clone()) {
                continue;
            }
            out.push(KnownUninstalledGame {
                game_id: entry.game_id.clone(),
                name: entry.name.clone(),
                nexus_slug: entry.nexus_domain.clone(),
                source: "vortex_registry".to_string(),
            });
        }

        // Source 2: curated Vortex extension index (Steam-app-id keyed).
        // Some games (e.g. Cyberpunk 2077) live here but not in source 1.
        for ext in crate::vortex_index::all_entries() {
            if installed_nexus_slugs.contains(&ext.nexus_slug) {
                continue;
            }
            if !seen_slugs.insert(ext.nexus_slug.clone()) {
                continue;
            }
            out.push(KnownUninstalledGame {
                game_id: ext.id.clone(),
                name: ext.name.clone(),
                nexus_slug: ext.nexus_slug.clone(),
                source: "vortex_extension_index".to_string(),
            });
        }

        // Source 3: full NexusMods catalog. ~3000 games — fills the long tail
        // beyond the ~85 covered by the bundled sources above.
        if let Some(games) = nm_games {
            for g in games {
                if installed_nexus_slugs.contains(&g.domain_name) {
                    continue;
                }
                if !seen_slugs.insert(g.domain_name.clone()) {
                    continue;
                }
                out.push(KnownUninstalledGame {
                    game_id: g.domain_name.clone(),
                    name: g.name,
                    nexus_slug: g.domain_name,
                    source: "nexusmods".to_string(),
                });
            }
        }

        // Sort alphabetically by display name for predictable UI.
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Ok::<_, String>(out)
    })
    .await
    .map_err(crate::format_join_error)?
}

// --- Native Game Commands ---

/// Accept a user-supplied `.app` path (from a file picker) and return a
/// fully-populated [`NativeAppCandidate`].
///
/// Validation is delegated to `native_scanner::validate_manual_native_app`,
/// which checks the extension, existence, and `Info.plist` readability.
/// This command is stateless — it does not mutate the database or game
/// registry; callers should follow up with a registry registration if
/// they intend to track the game.
#[tauri::command]
pub async fn add_native_game_manually(
    app_path: String,
) -> Result<crate::native_scanner::NativeAppCandidate, String> {
    let path = std::path::PathBuf::from(&app_path);
    crate::native_scanner::validate_manual_native_app(&path)
}

// --- Game Lock Commands ---

#[tauri::command]
pub async fn get_game_lock_status(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Option<game_lock::GameLock>, String> {
    Ok(state.game_locks.get(&game_id, &bottle_name))
}

#[tauri::command]
pub async fn get_all_game_locks(
    state: State<'_, AppState>,
) -> Result<Vec<game_lock::GameLock>, String> {
    Ok(state.game_locks.all_locks())
}

#[tauri::command]
pub async fn force_unlock_game(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    Ok(state.game_locks.force_unlock(&game_id, &bottle_name))
}


// --- Deploy Journal Commands ---

#[tauri::command]
pub async fn get_deploy_journal_status() -> Result<Vec<deploy_journal::JournalEntry>, String> {
    Ok(deploy_journal::get_incomplete())
}

#[tauri::command]
pub async fn heal_deployment(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let bottle = bottles::find_bottle_by_name(&bottle_name)
            .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))?;
        let game = games::detect_games(&bottle)
            .into_iter()
            .find(|g| g.game_id == game_id)
            .ok_or_else(|| format!("Game '{}' not found in bottle '{}'", game_id, bottle_name))?;
        let data_dir = PathBuf::from(&game.data_dir);

        deployer::redeploy_all(&db, &game_id, &bottle_name, &data_dir, &game.game_path)
            .map_err(|e| format!("Heal redeploy failed: {e}"))?;

        log::info!("heal_deployment: redeployed {}/{}", game_id, bottle_name);
        Ok(())
    })
    .await
    .map_err(crate::format_join_error)?
}


// --- Game Version Pinning ---

#[tauri::command]
pub async fn get_pinned_game_version(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    state
        .db
        .get_pinned_game_version(&game_id, &bottle_name)
        .map_err(|e| format!("Failed to get pinned version: {}", e))
}

#[tauri::command]
pub async fn pin_game_version(
    game_id: String,
    bottle_name: String,
    version: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .db
        .set_pinned_game_version(&game_id, &bottle_name, &version)
        .map_err(|e| format!("Failed to pin version: {}", e))
}


// --- Mod Rollback & Snapshots ---

#[tauri::command]
pub async fn save_mod_version_cmd(
    mod_id: i64,
    version: String,
    staging_path: String,
    archive_name: String,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        rollback::save_mod_version(&db, mod_id, &version, &staging_path, &archive_name)
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn list_mod_versions_cmd(
    mod_id: i64,
    state: State<'_, AppState>,
) -> Result<Vec<ModVersion>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || rollback::list_mod_versions(&db, mod_id))
        .await
        .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn rollback_mod_version(
    mod_id: i64,
    version_id: i64,
    game_id: String,
    bottle_name: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<ModVersion, String> {
    crate::check_game_lock(&state.game_locks, &game_id, &bottle_name)?;
    let _guard =
        crate::DeployGuard::try_acquire(state.deploy_in_progress.clone(), app.clone())?;
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        // 1. Update database to mark the target version as current
        let version = rollback::rollback_to_version(&db, mod_id, version_id)?;

        // 2. Resolve game paths for undeploy + redeploy
        let (_bottle, game, data_dir) = crate::resolve_game(&game_id, &bottle_name)?;
        let game_path = game.game_path.clone();

        // 3. Undeploy current files for this mod
        let _ = deployer::undeploy_mod(&db, &game_id, &bottle_name, mod_id, &data_dir, &game_path);

        // 4. Redeploy from the rolled-back version's staging path
        let staging_path = Path::new(&version.staging_path);
        if staging_path.exists() {
            // Get the file list from the mod's DB record
            let files = db
                .get_mod(mod_id)
                .ok()
                .flatten()
                .map(|m| m.installed_files)
                .unwrap_or_default();

            if !files.is_empty() {
                // Preserve original deploy target on rollback. Rollback
                // restores a prior version of the same mod, so the
                // deployment destination shouldn't change.
                let mod_target = db
                    .get_deploy_target_for_mod(mod_id)
                    .unwrap_or_else(|_| "data".to_string());
                let _ = deployer::deploy_mod_atomic(
                    &db,
                    &game_id,
                    &bottle_name,
                    mod_id,
                    staging_path,
                    &data_dir,
                    &files,
                    &game_path,
                    &mod_target,
                );
            }
        }

        Ok(version)
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn cleanup_mod_versions(
    mod_id: i64,
    keep_count: usize,
    state: State<'_, AppState>,
) -> Result<usize, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || rollback::cleanup_old_versions(&db, mod_id, keep_count))
        .await
        .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn create_mod_snapshot(
    game_id: String,
    bottle_name: String,
    name: String,
    description: Option<String>,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        rollback::create_snapshot(&db, &game_id, &bottle_name, &name, description.as_deref())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn list_mod_snapshots(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<ModSnapshot>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || rollback::list_snapshots(&db, &game_id, &bottle_name))
        .await
        .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn delete_mod_snapshot(snapshot_id: i64, state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || rollback::delete_snapshot(&db, snapshot_id))
        .await
        .map_err(crate::format_join_error)?
}


// --- Game Directory Cleaner ---

#[tauri::command]
pub async fn scan_game_directory(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<cleaner::CleanReport, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
        cleaner::scan_game_directory(&db, &game_id, &bottle_name, &data_dir)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn clean_game_directory(
    game_id: String,
    bottle_name: String,
    options: cleaner::CleanOptions,
    state: State<'_, AppState>,
) -> Result<cleaner::CleanResult, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;

        if !options.dry_run {
            auto_snapshot_before_destructive(
                &db,
                &game_id,
                &bottle_name,
                "Before clean game directory",
            );
        }

        let result =
            cleaner::clean_game_directory(&db, &game_id, &bottle_name, &data_dir, &options)
                .map_err(|e| e.to_string())?;

        // After a full clean (not orphans-only), reset plugins.txt to vanilla state
        // so the load order doesn't show stale entries for removed plugins
        if !options.dry_run && !options.orphans_only && !result.removed_files.is_empty() {
            if let Some(plugins_file) = games::with_plugin(&game_id, |plugin| {
                plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
            })
            .flatten()
            {
                // Build vanilla plugin list from stock ESMs still on disk
                let vanilla_entries: Vec<plugins::skyrim_plugins::PluginEntry> =
                    plugins::skyrim_plugins::get_implicit_plugins(&game_id)
                        .iter()
                        .filter(|name| data_dir.join(name).exists())
                        .map(|name| plugins::skyrim_plugins::PluginEntry {
                            filename: name.to_string(),
                            enabled: true,
                        })
                        .collect();
                let _ = plugins::skyrim_plugins::write_plugins_txt(&plugins_file, &vanilla_entries);
                log::info!(
                    "Reset plugins.txt to {} vanilla entries after clean",
                    vanilla_entries.len()
                );

                // Also reset loadorder.txt if it exists alongside plugins.txt
                if let Some(parent) = plugins_file.parent() {
                    let loadorder_file = parent.join("loadorder.txt");
                    if loadorder_file.exists() {
                        let _ = std::fs::remove_file(&loadorder_file);
                        log::info!("Removed stale loadorder.txt after clean");
                    }
                }
            }
        }

        Ok(result)
    })
    .await
    .map_err(crate::format_join_error)?
}

