//! Tauri commands for native macOS game discovery and native window effects.
//!
//! - `rescan_native_games`: aggregate scan of Steam mac, GOG mac, and
//!   `/Applications`, persists each discovered app to the `games` table,
//!   and returns the full candidate list.
//! - `apply_native_window_effect`: set the Liquid Glass variant on the main
//!   window at runtime — used by the native-mode toggle.

#[cfg(target_os = "macos")]
use tauri::{Manager, State};

#[cfg(target_os = "macos")]
use crate::native_scanner::{scan_all_native, NativeAppCandidate};
#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
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

// ---------------------------------------------------------------------------
// SMAPI (Stardew Valley) install / uninstall / status
// ---------------------------------------------------------------------------

/// Wire-format status for SMAPI installed in a Stardew Valley `.app` bundle.
///
/// Surfaced by [`get_stardew_smapi_status`] for the frontend to render the
/// "SMAPI installed / not installed / version" state.
#[cfg(target_os = "macos")]
#[derive(Clone, Debug, serde::Serialize)]
pub struct SmapiStatusDto {
    /// True iff both SMAPI markers (renamed vanilla launcher + SMAPI
    /// executable) are present in the bundle's `Contents/MacOS/`.
    pub installed: bool,
    /// Installed SMAPI version as read from its `.deps.json` files, when
    /// available. `None` if either the file is absent or no version key
    /// could be extracted.
    pub version: Option<String>,
    /// Absolute path to the `.app` bundle that was inspected. Useful for
    /// the frontend to display in tooltips / error messages.
    pub bundle_path: String,
    /// True iff the bundle is sandboxed (Mac App Store or
    /// `/System/Applications/`). SMAPI install is refused for sandboxed
    /// bundles regardless of marker presence.
    pub sandboxed: bool,
}

/// Resolve a native Stardew Valley game and return its `.app` bundle path.
///
/// Returns `Err` with a clean message when the game id does not resolve, has
/// no native runtime, or the bundle is sandboxed.
#[cfg(target_os = "macos")]
fn resolve_stardew_native_bundle(game_id: &str) -> Result<std::path::PathBuf, String> {
    let (_opt_bottle, game, _data_dir) = crate::resolve_game_any_runtime(game_id, "")?;
    let native = game
        .runtime
        .native()
        .ok_or_else(|| "SMAPI install requires a native macOS Stardew Valley install".to_string())?;
    if native.sandboxed {
        return Err(
            "SMAPI cannot install into a sandboxed (Mac App Store / system) bundle".to_string(),
        );
    }
    Ok(native.app_bundle_path.clone())
}

/// Install SMAPI into the user's native Stardew Valley `.app` bundle.
///
/// Downloads the latest SMAPI installer from GitHub
/// (`Pathoschild/SMAPI/releases/latest`), unpacks it into a temp directory,
/// and applies it via [`crate::smapi::install`]. Snapshots are created
/// automatically as part of the lower-level install pipeline. The
/// `com.apple.quarantine` xattr is cleared on success so Gatekeeper does
/// not block the SMAPI launcher.
///
/// macOS-only. Returns the resolved SMAPI tag name on success (e.g.
/// `"4.1.10"`) so the frontend can surface "SMAPI X.Y.Z installed".
#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn install_stardew_smapi(
    game_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let bundle = resolve_stardew_native_bundle(&game_id)?;
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        crate::smapi::fetch_and_install(&bundle, &db).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("SMAPI install task failed: {e}"))?
}

/// Uninstall SMAPI from the user's native Stardew Valley `.app` bundle.
///
/// Reverses [`install_stardew_smapi`]: removes SMAPI markers, restores the
/// vanilla launcher from `StardewValley-original`, and preserves the user's
/// `Mods/` directory. Idempotent — calling on a vanilla bundle is a no-op.
///
/// macOS-only. Returns a short human-readable status string.
#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn uninstall_stardew_smapi(
    game_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let bundle = resolve_stardew_native_bundle(&game_id)?;
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        crate::smapi::uninstall(&bundle, &db).map_err(|e| e.to_string())?;
        Ok::<String, String>("SMAPI uninstalled".to_string())
    })
    .await
    .map_err(|e| format!("SMAPI uninstall task failed: {e}"))?
}

/// Report SMAPI install state for the user's native Stardew Valley install.
///
/// Read-only. Safe to call even when SMAPI is not installed — returns
/// `installed: false` with `version: None`.
///
/// macOS-only.
#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn get_stardew_smapi_status(
    game_id: String,
) -> Result<SmapiStatusDto, String> {
    let (_opt_bottle, game, _data_dir) = crate::resolve_game_any_runtime(&game_id, "")?;
    let native = game
        .runtime
        .native()
        .ok_or_else(|| "SMAPI status requires a native macOS Stardew Valley install".to_string())?;
    let bundle = native.app_bundle_path.clone();
    let sandboxed = native.sandboxed;
    let installed = crate::smapi::is_installed(&bundle);
    let version = crate::smapi::installed_version(&bundle);
    Ok(SmapiStatusDto {
        installed,
        version,
        bundle_path: bundle.display().to_string(),
        sandboxed,
    })
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
#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn get_bg3se_status(app_bundle: String) -> Result<crate::bg3se::Bg3seStatus, String> {
    let path = std::path::PathBuf::from(app_bundle);
    Ok(crate::bg3se::detect(&path))
}

// ---------------------------------------------------------------------------
// BG3SE install / uninstall (research-blocker stubs — see bg3se.rs)
// ---------------------------------------------------------------------------

/// Install BG3SE into the given Baldur's Gate 3 `.app` bundle.
///
/// **Stub.** Returns the [`crate::bg3se::BG3SE_INSTALL_BLOCKER`] error
/// because the upstream install layout has not been verified. The frontend
/// should surface the message verbatim and direct the user to the manual
/// install instructions.
///
/// macOS-only.
#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn install_bg3se(
    app_bundle: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let bundle = std::path::PathBuf::from(app_bundle);
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || crate::bg3se::install(&bundle, &db))
        .await
        .map_err(|e| format!("BG3SE install task failed: {e}"))?
}

/// Uninstall BG3SE from the given Baldur's Gate 3 `.app` bundle.
///
/// **Stub.** Returns the [`crate::bg3se::BG3SE_INSTALL_BLOCKER`] error.
///
/// macOS-only.
#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn uninstall_bg3se(
    app_bundle: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let bundle = std::path::PathBuf::from(app_bundle);
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || crate::bg3se::uninstall(&bundle, &db))
        .await
        .map_err(|e| format!("BG3SE uninstall task failed: {e}"))?
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(all(test, target_os = "macos"))]
mod smapi_cmd_tests {
    use std::path::Path;
    use std::sync::Arc;

    /// Construct an in-memory `ModDatabase` with the rollback schema initialised.
    fn make_db(dir: &Path) -> Arc<crate::database::ModDatabase> {
        let db_path = dir.join("native-cmds-test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());
        crate::rollback::init_schema(&db).unwrap();
        db
    }

    /// `install_stardew_smapi` MUST refuse with a clean error when the supplied
    /// `game_id` does not resolve to a native runtime (this is the runtime gate;
    /// no HTTP traffic should occur).
    ///
    /// We exercise the underlying resolver helper directly because the
    /// `#[tauri::command]` wrapper requires a Tauri State, which is awkward to
    /// fabricate in unit tests. The resolver is the only path-checking step in
    /// the command — if it errors, the install never runs.
    #[test]
    fn install_stardew_smapi_refuses_when_no_native_runtime() {
        // A game_id that is guaranteed to not resolve to a native runtime.
        let result = super::resolve_stardew_native_bundle("definitely_not_a_real_game_id");
        assert!(result.is_err(), "must error when no native runtime resolves");
        let msg = result.unwrap_err();
        // Confirm the error is the resolver's friendly message rather than a
        // panic / unwrap failure.
        assert!(
            msg.to_lowercase().contains("native")
                || msg.to_lowercase().contains("not found"),
            "error must reference the native-runtime requirement: {msg}"
        );
    }

    /// `get_stardew_smapi_status` MUST report `installed = true` once SMAPI
    /// markers are present in a fake bundle. We bypass the full `resolve` step
    /// by calling the underlying `smapi::is_installed` + `installed_version`
    /// helpers directly to test the wire shape.
    #[test]
    fn get_stardew_smapi_status_reports_installed_after_install() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("Stardew Valley.app");
        let macos = bundle.join("Contents/MacOS");
        std::fs::create_dir_all(&macos).unwrap();

        // Pre-state: no SMAPI markers → not installed.
        assert!(!crate::smapi::is_installed(&bundle));

        // Drop the two markers SMAPI install creates.
        std::fs::write(macos.join("StardewValley-original"), b"vanilla").unwrap();
        std::fs::write(macos.join("StardewModdingAPI"), b"smapi binary").unwrap();
        // Add a deps.json so the version probe succeeds.
        std::fs::write(
            macos.join("StardewModdingAPI.deps.json"),
            br#"{"libraries":{"StardewModdingAPI/4.1.10":{}}}"#,
        )
        .unwrap();

        // Build the wire payload directly (same shape `get_stardew_smapi_status`
        // would produce given a resolved bundle path).
        let dto = super::SmapiStatusDto {
            installed: crate::smapi::is_installed(&bundle),
            version: crate::smapi::installed_version(&bundle),
            bundle_path: bundle.display().to_string(),
            sandboxed: crate::native_scanner::is_sandboxed(&bundle),
        };

        assert!(dto.installed, "installed must be true after marker writes");
        assert_eq!(dto.version.as_deref(), Some("4.1.10"));
        assert!(!dto.sandboxed);

        // Smoke-test serialisation so the wire shape stays compatible with
        // the frontend.
        let json = serde_json::to_string(&dto).unwrap();
        assert!(json.contains("\"installed\":true"));
        assert!(json.contains("\"version\":\"4.1.10\""));

        // Sanity: silence unused warnings on the DB helper if no further use.
        let _ = make_db(dir.path());
    }
}

#[cfg(all(test, target_os = "macos"))]
mod bg3se_cmd_tests {
    use std::path::Path;
    use std::sync::Arc;

    fn make_db(dir: &Path) -> Arc<crate::database::ModDatabase> {
        let db_path = dir.join("bg3se-cmds-test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());
        crate::rollback::init_schema(&db).unwrap();
        db
    }

    /// Install must return the documented blocker error string verbatim so the
    /// UI can render a deterministic message until research lands.
    #[test]
    fn install_bg3se_returns_blocker_stub_error() {
        let dir = tempfile::tempdir().unwrap();
        let db = make_db(dir.path());
        let bundle = dir.path().join("Baldurs Gate 3.app");
        std::fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();

        let result = crate::bg3se::install(&bundle, &db);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), crate::bg3se::BG3SE_INSTALL_BLOCKER);
    }

    /// Uninstall is symmetrically blocked.
    #[test]
    fn uninstall_bg3se_returns_blocker_stub_error() {
        let dir = tempfile::tempdir().unwrap();
        let db = make_db(dir.path());
        let bundle = dir.path().join("Baldurs Gate 3.app");
        std::fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();

        let result = crate::bg3se::uninstall(&bundle, &db);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), crate::bg3se::BG3SE_INSTALL_BLOCKER);
    }
}
