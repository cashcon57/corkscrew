//! Tauri commands for native macOS game discovery and native window effects.
//!
//! - `rescan_native_games`: aggregate scan of Steam mac, GOG mac, and
//!   `/Applications`, persists each discovered app to the `games` table,
//!   and returns the full candidate list.
//! - `apply_native_window_effect`: set the Liquid Glass variant on the main
//!   window at runtime — used by the native-mode toggle.

use tauri::{Manager, State};

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

/// Apply a Liquid Glass intensity variant to the main window at runtime.
///
/// Called by the native-mode toggle to ratchet the glass effect up when
/// entering native mode and back down when leaving.
///
/// Three intensity levels:
/// - `"default"` → `Regular` (variant 0) — the default startup state
/// - `"medium"`  → `Sidebar` (variant 16) — moderate vibrancy
/// - `"high"`    → `Inspector` (variant 18) — deepest M5-style glass
///
/// Cross-platform safety: the underlying plugin is a safe no-op on
/// Windows and Linux — no `#[cfg]` guard is required at call sites.
#[tauri::command]
pub async fn apply_native_window_effect(
    intensity: String,
    app: tauri::AppHandle,
) -> Result<(), String> {
    use tauri_plugin_liquid_glass::{GlassMaterialVariant, LiquidGlassConfig, LiquidGlassExt};

    let variant = match intensity.as_str() {
        "high" => GlassMaterialVariant::Inspector,
        "medium" => GlassMaterialVariant::Sidebar,
        _ => GlassMaterialVariant::Regular,
    };

    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "no main window".to_string())?;

    app.liquid_glass()
        .set_effect(
            &window,
            LiquidGlassConfig {
                enabled: true,
                variant,
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?;

    Ok(())
}

/// Get BepInEx (Paralives script-mod runtime) status for a given game
/// install directory. Read-only — no mutation. Install/uninstall live
/// in Layer 3 of the Paralives BepInEx integration.
///
/// Pass the absolute path to the game's Steam install directory (the directory
/// that CONTAINS `Paralives.app`, not the bundle itself), e.g.
/// `/Users/user/Library/Application Support/Steam/steamapps/common/Paralives`.
#[tauri::command]
pub async fn get_paralives_bepinex_status(
    game_install_dir: String,
) -> Result<crate::paralives_bepinex::ParalivesBepInExStatus, String> {
    let path = std::path::PathBuf::from(game_install_dir);
    Ok(crate::paralives_bepinex::detect(&path))
}

/// Return the BG3 Script Extender detection status for the given `.app` bundle.
///
/// Pass the absolute path to the `.app` bundle directory (e.g.
/// `/Applications/Baldurs Gate 3.app`). The command is read-only and
/// performs no installation or modification.
///
/// Returns [`crate::bg3se::Bg3seStatus`] — see that struct for field semantics,
/// especially `mac_supported` which distinguishes a correctly-installed macOS
/// dylib from a mis-dropped Windows `DWrite.dll`.
#[tauri::command]
pub async fn get_bg3se_status(app_bundle: String) -> Result<crate::bg3se::Bg3seStatus, String> {
    let path = std::path::PathBuf::from(app_bundle);
    Ok(crate::bg3se::detect(&path))
}
