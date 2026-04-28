//! Tauri commands for FromSoft Mod Engine 2 + regulation.bin + save backup.
//!
//! Shared resolution pattern: every command takes `(game_id, bottle_name)`
//! and resolves to `(Bottle, DetectedGame, game_data_dir)` via [`resolve_game`].
//! ME2 config lives at `<game_path>/modengine2/`, so we use `game.game_path`.

use std::path::PathBuf;

use tauri::State;

use crate::fromsoft_saves;
use crate::modengine2_config::{self as me2, ModEngine2Config};
use crate::regulation_conflicts::{self, RegulationConflict};
use crate::{resolve_game, AppState};

// ---------------------------------------------------------------------------
// modengine2.toml editor
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_modengine2_config(
    game_id: String,
    bottle_name: String,
) -> Result<ModEngine2Config, String> {
    tokio::task::spawn_blocking(move || {
        let (_, game, _) = resolve_game(&game_id, &bottle_name)?;
        me2::load_config(&game.game_path, &game_id)
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn save_modengine2_config(
    game_id: String,
    bottle_name: String,
    config: ModEngine2Config,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let (_, game, _) = resolve_game(&game_id, &bottle_name)?;
        me2::save_config(&game.game_path, &game_id, &config)
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn add_mod_to_modengine2(
    game_id: String,
    bottle_name: String,
    name: String,
    path: String,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let (_, game, _) = resolve_game(&game_id, &bottle_name)?;
        let mut cfg = me2::load_config(&game.game_path, &game_id)?;
        me2::add_mod(&mut cfg, &name, &path);
        me2::save_config(&game.game_path, &game_id, &cfg)
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn remove_mod_from_modengine2(
    game_id: String,
    bottle_name: String,
    name: String,
) -> Result<bool, String> {
    tokio::task::spawn_blocking(move || {
        let (_, game, _) = resolve_game(&game_id, &bottle_name)?;
        let mut cfg = me2::load_config(&game.game_path, &game_id)?;
        let removed = me2::remove_mod(&mut cfg, &name);
        if removed {
            me2::save_config(&game.game_path, &game_id, &cfg)?;
        }
        Ok::<bool, String>(removed)
    })
    .await
    .map_err(crate::format_join_error)?
}

// ---------------------------------------------------------------------------
// regulation.bin conflict detection
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_regulation_conflicts(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<RegulationConflict>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        regulation_conflicts::detect_regulation_conflicts(&db, &game_id, &bottle_name)
    })
    .await
    .map_err(crate::format_join_error)?
}

// ---------------------------------------------------------------------------
// FromSoft save backups
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn list_fromsoft_saves(
    game_id: String,
    bottle_name: String,
) -> Result<Vec<fromsoft_saves::SaveFile>, String> {
    tokio::task::spawn_blocking(move || {
        let (bottle, _, _) = resolve_game(&game_id, &bottle_name)?;
        Ok::<Vec<_>, String>(fromsoft_saves::list_saves(&bottle, &game_id))
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn get_fromsoft_saves_dir(
    game_id: String,
    bottle_name: String,
) -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || {
        let (bottle, _, _) = resolve_game(&game_id, &bottle_name)?;
        let dir: Option<PathBuf> = fromsoft_saves::find_fromsoft_saves_dir(&bottle, &game_id);
        Ok::<Option<String>, String>(dir.map(|p| p.to_string_lossy().to_string()))
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn backup_fromsoft_saves(
    game_id: String,
    bottle_name: String,
    max_backups: Option<usize>,
) -> Result<usize, String> {
    let max = max_backups.unwrap_or(fromsoft_saves::DEFAULT_MAX_BACKUPS);
    tokio::task::spawn_blocking(move || {
        let (bottle, _, _) = resolve_game(&game_id, &bottle_name)?;
        fromsoft_saves::backup_saves_before_launch(&bottle, &game_id, max)
    })
    .await
    .map_err(crate::format_join_error)?
}
