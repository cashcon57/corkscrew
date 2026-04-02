//! Plugin load order: LOOT sorting and custom ordering rules.

use crate::loot;
use crate::loot_rules;
use crate::plugins;
use crate::games;
use crate::loot::{MasterlistStatus, PluginWarning, SortResult};
use crate::loot_rules::{PluginRule};
use crate::plugins::skyrim_plugins::{PluginEntry};
use crate::{AppState, resolve_game};
use std::path::{Path, PathBuf};
use tauri::State;

// --- LOOT & Plugin Management ---

#[tauri::command]
pub async fn sort_plugins_loot(game_id: String, bottle_name: String) -> Result<SortResult, String> {
    let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;
    let game_path = PathBuf::from(&game.game_path);
    let local_path = loot::local_game_path(&bottle, &game_id)
        .ok_or_else(|| format!("Cannot determine local path for game '{}'", game_id))?;

    // Sort using LOOT
    let sort_result = loot::sort_plugins(&game_id, &game_path, &data_dir, &local_path)
        .map_err(|e| e.to_string())?;

    // Apply the sorted order to disk
    if sort_result.plugins_moved > 0 {
        let plugins_file = games::with_plugin(&game_id, |plugin| {
            plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
        })
        .flatten()
        .ok_or_else(|| "Could not determine plugins file location".to_string())?;

        let loadorder_file = plugins_file
            .parent()
            .map(|p| p.join("loadorder.txt"))
            .unwrap_or_else(|| plugins_file.with_file_name("loadorder.txt"));

        // Build PluginEntry list from sorted order, preserving enabled state
        let existing = if plugins_file.exists() {
            plugins::skyrim_plugins::read_plugins_txt(&plugins_file).unwrap_or_default()
        } else {
            Vec::new()
        };

        let enabled_map: std::collections::HashMap<String, bool> = existing
            .iter()
            .map(|e| (e.filename.to_lowercase(), e.enabled))
            .collect();

        let ordered_entries: Vec<PluginEntry> = sort_result
            .sorted_order
            .iter()
            .map(|name| PluginEntry {
                filename: name.clone(),
                enabled: enabled_map
                    .get(&name.to_lowercase())
                    .copied()
                    .unwrap_or(false),
            })
            .collect();

        plugins::skyrim_plugins::apply_load_order(&plugins_file, &loadorder_file, &ordered_entries)
            .map_err(|e| e.to_string())?;
    }

    Ok(sort_result)
}

#[tauri::command]
pub async fn update_loot_masterlist(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    loot::update_masterlist(&game_id, Some(&state.loot_masterlist_checked))
        .await
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn force_refresh_loot_masterlist(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    loot::force_refresh_masterlist(&game_id, Some(&state.loot_masterlist_checked))
        .await
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn reorder_plugins_cmd(
    game_id: String,
    bottle_name: String,
    ordered_plugins: Vec<String>,
) -> Result<Vec<PluginEntry>, String> {
    tokio::task::spawn_blocking(move || {
        let (bottle, game, _) = resolve_game(&game_id, &bottle_name)?;

        let plugins_file = games::with_plugin(&game_id, |plugin| {
            plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
        })
        .flatten()
        .ok_or_else(|| "Could not determine plugins file location".to_string())?;

        let loadorder_file = plugins_file
            .parent()
            .map(|p| p.join("loadorder.txt"))
            .unwrap_or_else(|| plugins_file.with_file_name("loadorder.txt"));

        plugins::skyrim_plugins::reorder_plugins(&plugins_file, &loadorder_file, &ordered_plugins)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn toggle_plugin_cmd(
    game_id: String,
    bottle_name: String,
    plugin_name: String,
    enabled: bool,
) -> Result<Vec<PluginEntry>, String> {
    tokio::task::spawn_blocking(move || {
        let (bottle, game, _) = resolve_game(&game_id, &bottle_name)?;

        let plugins_file = games::with_plugin(&game_id, |plugin| {
            plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
        })
        .flatten()
        .ok_or_else(|| "Could not determine plugins file location".to_string())?;

        let loadorder_file = plugins_file
            .parent()
            .map(|p| p.join("loadorder.txt"))
            .unwrap_or_else(|| plugins_file.with_file_name("loadorder.txt"));

        plugins::skyrim_plugins::toggle_plugin(
            &plugins_file,
            &loadorder_file,
            &plugin_name,
            enabled,
        )
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn move_plugin_cmd(
    game_id: String,
    bottle_name: String,
    plugin_name: String,
    new_index: usize,
) -> Result<Vec<PluginEntry>, String> {
    tokio::task::spawn_blocking(move || {
        let (bottle, game, _) = resolve_game(&game_id, &bottle_name)?;

        let plugins_file = games::with_plugin(&game_id, |plugin| {
            plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
        })
        .flatten()
        .ok_or_else(|| "Could not determine plugins file location".to_string())?;

        let loadorder_file = plugins_file
            .parent()
            .map(|p| p.join("loadorder.txt"))
            .unwrap_or_else(|| plugins_file.with_file_name("loadorder.txt"));

        plugins::skyrim_plugins::move_plugin(
            &plugins_file,
            &loadorder_file,
            &plugin_name,
            new_index,
        )
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn get_plugin_messages(
    game_id: String,
    bottle_name: String,
    plugin_name: String,
) -> Result<Vec<PluginWarning>, String> {
    tokio::task::spawn_blocking(move || {
        let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;

        let game_path = PathBuf::from(&game.game_path);
        let local_path = loot::local_game_path(&bottle, &game_id)
            .ok_or_else(|| format!("Cannot determine local path for game '{}'", game_id))?;

        loot::get_plugin_messages(&game_id, &game_path, &data_dir, &local_path, &plugin_name)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}


#[tauri::command]
pub async fn get_masterlist_status(game_id: String) -> Result<MasterlistStatus, String> {
    Ok(loot::get_masterlist_status(&game_id))
}

// --- Plugin Load Order Rules ---

#[tauri::command]
pub async fn add_plugin_rule(
    game_id: String,
    bottle_name: String,
    plugin_name: String,
    rule_type: loot_rules::PluginRuleType,
    reference_plugin: String,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        loot_rules::add_rule(
            &db,
            &game_id,
            &bottle_name,
            &plugin_name,
            rule_type,
            &reference_plugin,
        )
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn remove_plugin_rule(rule_id: i64, state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || loot_rules::remove_rule(&db, rule_id))
        .await
        .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn list_plugin_rules(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<PluginRule>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || loot_rules::list_rules(&db, &game_id, &bottle_name))
        .await
        .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn clear_plugin_rules(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || loot_rules::clear_rules(&db, &game_id, &bottle_name))
        .await
        .map_err(crate::format_join_error)?
}

