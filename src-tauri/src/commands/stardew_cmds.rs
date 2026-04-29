//! Tauri commands for Stardew Valley (native) mod status analysis.
//!
//! [`get_stardew_mod_status`] surfaces unmet SMAPI dependencies and UniqueID
//! conflicts so the frontend can display warnings on the Stardew mods page.

use tauri::State;

use crate::plugins::stardew_valley_native::StardewModStatus;
use crate::AppState;

/// Return per-mod dependency and conflict status for all installed
/// Stardew Valley (native) mods.
///
/// # Phase 1 stub
///
/// The real implementation reads enabled mods from the database for
/// `game_id = "stardew_valley_native"`, opens each mod's
/// `<staging_dir>/manifest.json`, parses it with `parse_manifest`, builds
/// a `Vec<InstalledModInfo>`, and calls `analyze_mod_status`. This wiring
/// is deferred to Phase 5 (Native Mode UI shell) when the Stardew mods
/// page that will consume this data is built. The analyzer itself is fully
/// implemented and tested in `stardew_valley_native.rs`.
///
/// Returns an empty list until the DB integration is wired.
#[tauri::command]
pub async fn get_stardew_mod_status(
    _state: State<'_, AppState>,
) -> Result<Vec<StardewModStatus>, String> {
    // TODO (Phase 5): wire real DB read + manifest parsing.
    //   1. db.list_mods("stardew_valley_native", "") → enabled mods
    //   2. For each mod with a staging_path: open <staging_path>/manifest.json,
    //      call parse_manifest(), build InstalledModInfo.
    //   3. Call analyze_mod_status(&infos) and return the result.
    Ok(Vec::new())
}
