//! Tauri commands for native macOS game discovery.
//!
//! - `rescan_native_games`: aggregate scan of Steam mac, GOG mac, and
//!   `/Applications`, persists each discovered app to the `games` table,
//!   and returns the full candidate list.

use tauri::State;

use crate::native_scanner::{scan_all_native, NativeAppCandidate};
use crate::AppState;

/// Aggregate scan: `/Applications` + Steam mac + GOG mac.
///
/// Deduplicates by canonicalized bundle path (Steam wins over GOG wins over
/// `/Applications`). Each discovered candidate is persisted to the `games`
/// table via `upsert_game` using the bundle identifier as the stable
/// `game_id`. Per-game native plugins (Task 3+) may later rename the
/// `game_id` to a friendlier key.
///
/// Returns the full list of candidates including their architecture and
/// sandbox metadata so the frontend can display discovery results.
#[tauri::command]
pub async fn rescan_native_games(
    state: State<'_, AppState>,
) -> Result<Vec<NativeAppCandidate>, String> {
    let candidates = tokio::task::spawn_blocking(scan_all_native)
        .await
        .map_err(|e| format!("native scan task failed: {e}"))?;

    let db = state.db.clone();
    for c in &candidates {
        let game = crate::database::PersistedGame {
            game_id: c.info.bundle_identifier.clone(),
            runtime: crate::runtime::GameRuntime::Native(crate::runtime::NativeContext {
                app_bundle_path: c.bundle_path.clone(),
                game_data_root: c.bundle_path.join("Contents/MacOS"),
                architecture: c.architecture,
                sandboxed: c.sandboxed,
                source: c.source,
            }),
        };
        let db_clone = db.clone();
        let game_clone = game.clone();
        tokio::task::spawn_blocking(move || db_clone.upsert_game(&game_clone))
            .await
            .map_err(|e| format!("db task failed: {e}"))?
            .map_err(|e| format!("upsert_game failed: {e}"))?;
    }

    Ok(candidates)
}
