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

    // Persist the canonicalized paths rather than the raw strings so any
    // `..` redundancy in the caller-supplied path is normalized before the
    // value lands in the custom_games DB. The canonical forms passed all
    // the containment checks above, so they are guaranteed to live inside
    // the bottle.
    let custom = CustomGame {
        game_id,
        display_name,
        nexus_slug,
        game_path: canon_game_path.display().to_string(),
        exe_path: Some(canon_exe.display().to_string()),
        // Default mod deployment dir = game_path. Bethesda titles override
        // this elsewhere via a per-plugin module; for arbitrary games this
        // is the safe default.
        data_dir: canon_game_path.display().to_string(),
        bottle_name: bottle.name.clone(),
        bottle_path: bottle.path.display().to_string(),
        steam_app_id,
    };

    save_custom_game(&state.db, &custom)
}

/// Probe a user-selected directory for any known game executables.
///
/// Returns all matching games so the frontend can auto-fill the "Add Game"
/// form (game_id, display_name, nexus_slug, exe_path). Returns an empty
/// list when no known game is found — the user must fill the form manually.
#[tauri::command]
pub async fn identify_game_at_path(
    path: String,
) -> Result<Vec<crate::games::GameIdentification>, String> {
    let dir = std::path::PathBuf::from(path);
    tokio::task::spawn_blocking(move || Ok(crate::games::identify_at_path(&dir)))
        .await
        .map_err(|e| format!("identify task failed: {e}"))?
}

/// Remove a custom-added game from the database. Scoped by (game_id,
/// bottle_name) — the same game_id can legitimately exist in multiple
/// bottles, so a card-level Remove must not blow them all away.
#[tauri::command]
pub async fn remove_custom_game_cmd(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        crate::game_registry::remove_custom_game_for_bottle(&db, &game_id, &bottle_name)
    })
    .await
    .map_err(|e| format!("remove task failed: {e}"))?
}

/// Update the game_path / exe_path / data_dir for an existing custom game.
/// Re-validates that paths exist and stay inside the bottle (same checks as
/// `register_unregistered_game`) before writing.
#[tauri::command]
pub async fn update_custom_game_paths_cmd(
    game_id: String,
    bottle_name: String,
    game_path: String,
    exe_path: String,
    data_dir: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
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

    // Default data_dir to game_path when not supplied — matches the
    // behaviour of `register_unregistered_game`.
    let data_dir_str = data_dir.unwrap_or_else(|| canon_game_path.display().to_string());

    let db = state.db.clone();
    let game_path_owned = canon_game_path.display().to_string();
    let exe_path_owned = canon_exe.display().to_string();
    tokio::task::spawn_blocking(move || {
        crate::game_registry::update_custom_game_paths(
            &db,
            &game_id,
            &bottle_name,
            &game_path_owned,
            &exe_path_owned,
            &data_dir_str,
        )
    })
    .await
    .map_err(|e| format!("update task failed: {e}"))?
}
