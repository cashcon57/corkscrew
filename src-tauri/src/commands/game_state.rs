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
use std::path::{Path, PathBuf};
use tauri::State;

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
    state: State<'_, AppState>,
) -> Result<ModVersion, String> {
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
                let _ = deployer::deploy_mod_atomic(
                    &db,
                    &game_id,
                    &bottle_name,
                    mod_id,
                    staging_path,
                    &data_dir,
                    &files,
                    &game_path,
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

