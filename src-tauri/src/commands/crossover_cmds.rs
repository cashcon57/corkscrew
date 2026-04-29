//! Tauri commands for CrossOver shortcut auto-discovery.
//!
//! - `list_unregistered_crossover_games`: scan all bottles for `.lnk`
//!   shortcuts that look like games but aren't yet registered.
//! - `register_unregistered_game`: persist a shortcut to the
//!   `custom_games` DB so it shows up in the game selector on next scan.

use std::path::Path;

use tauri::State;

use crate::bottles::detect_bottles;
use crate::crossover_shortcuts::{self, UnregisteredGame};
use crate::game_registry::{save_custom_game, CustomGame};
use crate::games::detect_all_games_with_custom;
use crate::AppState;

/// Scan all detected bottles and return shortcuts that look like games but
/// aren't already registered (neither auto-detected by a plugin nor saved
/// as a custom game).
#[tauri::command]
pub async fn list_unregistered_crossover_games(
    state: State<'_, AppState>,
) -> Result<Vec<UnregisteredGame>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let bottles = detect_bottles();
        let already = detect_all_games_with_custom(&db);
        crossover_shortcuts::list_unregistered_games(&bottles, &already)
    })
    .await
    .map_err(|e| format!("crossover scan task failed: {e}"))
}

/// Persist a discovered shortcut as a custom game in the database.
///
/// The frontend supplies the (possibly auto-matched, possibly user-edited)
/// metadata. We re-validate `game_path` and `exe_path` exist on disk
/// before writing.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn register_unregistered_game(
    bottle_name: String,
    game_id: String,
    display_name: String,
    nexus_slug: String,
    steam_app_id: Option<String>,
    game_path: String,
    exe_path: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if game_id.trim().is_empty() {
        return Err("game_id must not be empty".into());
    }
    if display_name.trim().is_empty() {
        return Err("display_name must not be empty".into());
    }
    let game_path_p = Path::new(&game_path);
    let exe_path_p = Path::new(&exe_path);

    if !game_path_p.is_dir() {
        return Err(format!("game_path does not exist: {}", game_path));
    }
    if !exe_path_p.is_file() {
        return Err(format!("exe_path does not exist: {}", exe_path));
    }

    let bottle = crate::bottles::find_bottle_by_name(&bottle_name)
        .ok_or_else(|| format!("Bottle not found: {bottle_name}"))?;

    // Sanity: the exe must live inside the bottle's root (any drive_X is
    // allowed — Wine supports multi-drive bottles, and ME2 .bat launchers /
    // games may legitimately live on drive_d, drive_e, etc). Reject anything
    // outside the bottle root to prevent registering arbitrary host paths.
    let canon_bottle = std::fs::canonicalize(&bottle.path)
        .map_err(|e| format!("canonicalize bottle failed: {e}"))?;
    let canon_exe = std::fs::canonicalize(exe_path_p)
        .map_err(|e| format!("canonicalize exe failed: {e}"))?;
    if !canon_exe.starts_with(&canon_bottle) {
        return Err("exe_path must live inside the bottle".into());
    }
    let canon_game_path = std::fs::canonicalize(game_path_p)
        .map_err(|e| format!("canonicalize game_path failed: {e}"))?;
    if !canon_game_path.starts_with(&canon_bottle) {
        return Err("game_path must live inside the bottle".into());
    }

    let custom = CustomGame {
        game_id,
        display_name,
        nexus_slug,
        game_path: game_path_p.display().to_string(),
        exe_path: Some(exe_path_p.display().to_string()),
        // Default mod deployment dir = game_path. Bethesda titles override
        // this elsewhere via a per-plugin module; for arbitrary games this
        // is the safe default.
        data_dir: game_path_p.display().to_string(),
        bottle_name: bottle.name.clone(),
        bottle_path: bottle.path.display().to_string(),
        steam_app_id,
    };

    save_custom_game(&state.db, &custom)
}
