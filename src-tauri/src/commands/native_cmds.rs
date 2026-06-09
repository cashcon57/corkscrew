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

/// Install BepInEx 6.x IL2CPP macOS ARM64 into a Paralives game install directory.
///
/// Downloads the latest BepInEx 6.x macOS ARM64 release from GitHub, extracts
/// it into `game_install_dir`, and removes the Apple Developer ID signature from
/// `app_bundle_path` so BepInEx's doorstop loader can inject into the game.
///
/// This is a TRUST-BOUNDARY MUTATION — the caller MUST gate this behind the
/// consent dialog in the frontend before invoking. The frontend consent flow is
/// the only permitted entry point.
///
/// # Arguments
///
/// * `game_install_dir` — absolute path to the Steam install directory that
///   CONTAINS `Paralives.app` (e.g. `.../steamapps/common/Paralives`).
/// * `app_bundle_path` — absolute path to `Paralives.app` itself. Its parent
///   must equal `game_install_dir`.
#[tauri::command]
pub async fn install_paralives_bepinex(
    game_install_dir: String,
    app_bundle_path: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let install_dir = std::path::PathBuf::from(game_install_dir);
    let bundle = std::path::PathBuf::from(app_bundle_path);
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        crate::paralives_bepinex::install(&install_dir, &bundle, &db)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Uninstall BepInEx from a Paralives game install directory.
///
/// Removes BepInEx marker files/directories. Does NOT restore the `.app`
/// signature — the user must use Steam's "Verify integrity of game files"
/// or reinstall the game.
///
/// Idempotent: if BepInEx is not installed, returns Ok immediately.
#[tauri::command]
pub async fn uninstall_paralives_bepinex(
    game_install_dir: String,
) -> Result<(), String> {
    let install_dir = std::path::PathBuf::from(game_install_dir);
    tokio::task::spawn_blocking(move || {
        crate::paralives_bepinex::uninstall(&install_dir)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------
// Manual native game add (file-picker flow)
// ---------------------------------------------------------------------------

/// Result of manually adding a native game via file picker. The frontend
/// uses `matched_plugin` to decide whether to prompt the user to use
/// the matched plugin or register as a generic unsupported game.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ManualNativeAddResult {
    pub candidate: crate::native_scanner::NativeAppCandidate,
    pub matched_plugin: Option<crate::native_scanner::KnownNativePluginMatch>,
}

/// Validate a user-supplied `.app` path and detect whether it matches
/// one of our known native plugins.
///
/// This is the first step of the manual-add flow: the frontend calls this
/// after the user has selected a `.app` bundle via the file picker.
/// If `matched_plugin` is `Some`, the frontend should prompt the user to
/// confirm use of the matched plugin. If it is `None`, register as
/// generic unsupported.
#[tauri::command]
pub async fn add_manual_native_game(app_path: String) -> Result<ManualNativeAddResult, String> {
    let path = std::path::PathBuf::from(app_path);
    let candidate = crate::native_scanner::validate_manual_native_app(&path)?;
    let matched_plugin = crate::native_scanner::match_known_native_plugin(&candidate);
    Ok(ManualNativeAddResult {
        candidate,
        matched_plugin,
    })
}

/// Register a manually-added native game in the database.
///
/// Called after the user has responded to the plugin-match confirmation:
/// - `use_plugin_game_id = Some("paralives_native")` → register under
///   that plugin's `game_id` so it inherits deploy/manifest logic.
/// - `use_plugin_game_id = None` → register as a generic unsupported
///   native game; `game_id` is derived from `bundle_identifier`.
///
/// Returns a [`crate::games::DetectedGame`] that the frontend can push into
/// the game selector immediately (no re-scan required).
///
/// # Persistence
///
/// The game is persisted to the `games` table via `upsert_game`, so it
/// survives app restarts. On the next `getAllGames()` refresh, the game
/// will appear via `detect_native_games()` (plugin-matched) or via the
/// stored `PersistedGame` (generic).
#[tauri::command]
pub async fn register_manual_native_game(
    state: State<'_, AppState>,
    candidate: crate::native_scanner::NativeAppCandidate,
    use_plugin_game_id: Option<String>,
) -> Result<crate::games::DetectedGame, String> {
    let game_id = use_plugin_game_id
        .clone()
        .unwrap_or_else(|| format!("manual:{}", candidate.info.bundle_identifier));

    // Build display_name and nexus_slug. For plugin-matched games, consult
    // the registered plugin for authoritative metadata. Fall back to bundle
    // info if the plugin isn't registered (should not happen in production).
    let (display_name, nexus_slug) = if let Some(ref plugin_id) = use_plugin_game_id {
        crate::games::with_plugin(plugin_id, |p| {
            (p.display_name().to_owned(), p.nexus_slug().to_owned())
        })
        .unwrap_or_else(|| (candidate.info.bundle_executable.clone(), String::new()))
    } else {
        (candidate.info.bundle_executable.clone(), String::new())
    };

    let game_path = candidate.bundle_path.clone();
    let data_root = candidate.bundle_path.join("Contents/MacOS");

    let runtime = crate::runtime::GameRuntime::Native(crate::runtime::NativeContext {
        app_bundle_path: candidate.bundle_path.clone(),
        game_data_root: data_root.clone(),
        architecture: candidate.architecture,
        sandboxed: candidate.sandboxed,
        source: crate::runtime::NativeSource::Manual,
    });

    let persisted = crate::database::PersistedGame {
        game_id: game_id.clone(),
        runtime: runtime.clone(),
    };

    let db = state.db.clone();
    let persisted_clone = persisted.clone();
    tokio::task::spawn_blocking(move || db.upsert_game(&persisted_clone))
        .await
        .map_err(|e| format!("db task failed: {e}"))?
        .map_err(|e| format!("upsert_game failed: {e}"))?;

    let detected = crate::games::DetectedGame {
        game_id,
        display_name,
        nexus_slug,
        game_path,
        exe_path: None,
        data_dir: data_root,
        runtime,
        steam_app_id: None,
    };

    Ok(detected)
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
