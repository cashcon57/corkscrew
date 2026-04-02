//! Mod profile management: create, switch, save snapshots, and manage saves.

use crate::plugins;
use crate::profiles;
use crate::config;
use crate::deployer;
use crate::games;
use crate::plugins::skyrim_plugins::{PluginEntry};
use crate::profiles::{Profile};
use crate::{AppState, DeployGuard, check_game_lock, resolve_game};
use std::path::Path;
use tauri::{AppHandle, State};

// --- Profiles ---

#[tauri::command]
pub async fn list_profiles_cmd(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<Profile>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        profiles::list_profiles(&db, &game_id, &bottle_name).map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn create_profile_cmd(
    game_id: String,
    bottle_name: String,
    name: String,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        profiles::create_profile(&db, &game_id, &bottle_name, &name).map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn delete_profile_cmd(profile_id: i64, state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        profiles::delete_profile(&db, profile_id).map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn deactivate_profile_cmd(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        profiles::deactivate_profile(&db, &game_id, &bottle_name).map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn rename_profile_cmd(
    profile_id: i64,
    new_name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        profiles::rename_profile(&db, profile_id, &new_name).map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn save_profile_snapshot(
    profile_id: i64,
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        // Determine plugins file path (for Bethesda games with plugin load order)
        let plugins_file = if plugins::skyrim_plugins::supports_plugin_order(&game_id) {
            let (bottle, game, _) = resolve_game(&game_id, &bottle_name)?;

            games::with_plugin(&game_id, |plugin| {
                plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
            })
            .flatten()
        } else {
            None
        };

        profiles::snapshot_current_state(
            &db,
            profile_id,
            &game_id,
            &bottle_name,
            plugins_file.as_deref(),
        )
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn activate_profile(
    profile_id: i64,
    game_id: String,
    bottle_name: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    check_game_lock(&state.game_locks, &game_id, &bottle_name)?;
    let _guard = DeployGuard::new(state.deploy_in_progress.clone(), app.clone());
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        // Look up the game
        let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;

        // Check if per-profile saves is enabled
        let saves_enabled = config::get_config_value("profile_saves_enabled")
            .ok()
            .flatten()
            .map(|v| v == "true")
            .unwrap_or(false);

        // Resolve saves directory for the game
        let saves_dir = if saves_enabled {
            games::with_plugin(&game_id, |plugin| {
                plugin.get_saves_dir(Path::new(&game.game_path), &bottle)
            })
            .flatten()
        } else {
            None
        };

        // 1. Save current state to the currently active profile (if any)
        if let Ok(Some(current_active)) = profiles::get_active_profile(&db, &game_id, &bottle_name)
        {
            let plugins_file = if plugins::skyrim_plugins::supports_plugin_order(&game_id) {
                games::with_plugin(&game_id, |plugin| {
                    plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
                })
                .flatten()
            } else {
                None
            };

            let _ = profiles::snapshot_current_state(
                &db,
                current_active.id,
                &game_id,
                &bottle_name,
                plugins_file.as_deref(),
            );

            // Backup current saves for the outgoing profile
            if let Some(ref sd) = saves_dir {
                let _ = profiles::backup_saves(current_active.id, &game_id, &bottle_name, sd);
            }
        }

        // 2. Purge current deployment
        let _ = deployer::purge_deployment(&db, &game_id, &bottle_name, &data_dir, &game.game_path);

        // 3. Load target profile state
        let mod_states = profiles::get_mod_states(&db, profile_id).map_err(|e| e.to_string())?;

        // 4. Apply mod enabled states and priorities
        for ms in &mod_states {
            let _ = db.set_enabled(ms.mod_id, ms.enabled);
            let _ = db.set_mod_priority(ms.mod_id, ms.priority);
        }

        // 5. Redeploy enabled mods
        let _ = deployer::redeploy_all(&db, &game_id, &bottle_name, &data_dir, &game.game_path);

        // 6. Restore saves for the incoming profile
        if let Some(ref sd) = saves_dir {
            let _ = profiles::restore_saves(profile_id, &game_id, &bottle_name, sd);
        }

        // 7. Apply plugin states
        let plugin_states =
            profiles::get_plugin_states(&db, profile_id).map_err(|e| e.to_string())?;

        if !plugin_states.is_empty() && plugins::skyrim_plugins::supports_plugin_order(&game_id) {
            let plugins_file = games::with_plugin(&game_id, |plugin| {
                plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
            })
            .flatten();

            if let Some(pf) = plugins_file {
                let loadorder_file = pf
                    .parent()
                    .map(|p| p.join("loadorder.txt"))
                    .unwrap_or_else(|| pf.with_file_name("loadorder.txt"));

                let entries: Vec<PluginEntry> = plugin_states
                    .iter()
                    .map(|ps| PluginEntry {
                        filename: ps.plugin_filename.clone(),
                        enabled: ps.enabled,
                    })
                    .collect();

                let _ = plugins::skyrim_plugins::apply_load_order(&pf, &loadorder_file, &entries);
            }
        }

        // 8. Sync plugins to ensure plugins.txt matches on-disk state
        let _ = crate::sync_plugins_for_game(&game, &bottle);

        // 9. Mark profile as active
        profiles::set_active_profile(&db, &game_id, &bottle_name, profile_id)
            .map_err(|e| e.to_string())?;

        Ok(())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn get_profile_save_info(
    profile_id: i64,
    game_id: String,
    bottle_name: String,
) -> Result<profiles::ProfileSaveInfo, String> {
    tokio::task::spawn_blocking(move || {
        Ok(profiles::get_profile_save_info(
            profile_id,
            &game_id,
            &bottle_name,
        ))
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn backup_profile_saves(
    profile_id: i64,
    game_id: String,
    bottle_name: String,
) -> Result<usize, String> {
    tokio::task::spawn_blocking(move || {
        let (bottle, game, _) = resolve_game(&game_id, &bottle_name)?;
        let saves_dir = games::with_plugin(&game_id, |plugin| {
            plugin.get_saves_dir(Path::new(&game.game_path), &bottle)
        })
        .flatten()
        .ok_or("Game does not have a known saves directory")?;

        profiles::backup_saves(profile_id, &game_id, &bottle_name, &saves_dir)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn restore_profile_saves(
    profile_id: i64,
    game_id: String,
    bottle_name: String,
) -> Result<usize, String> {
    tokio::task::spawn_blocking(move || {
        let (bottle, game, _) = resolve_game(&game_id, &bottle_name)?;
        let saves_dir = games::with_plugin(&game_id, |plugin| {
            plugin.get_saves_dir(Path::new(&game.game_path), &bottle)
        })
        .flatten()
        .ok_or("Game does not have a known saves directory")?;

        profiles::restore_saves(profile_id, &game_id, &bottle_name, &saves_dir)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}

