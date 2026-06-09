//! Paralives (native macOS) game plugin.
//!
//! Paralives is a Unity (IL2CPP) life-sim by Paralives Studio with first-
//! class official mod support: feature mods, asset mods, and a Steam
//! Workshop integration. Mods drop into the Unity persistent data path
//! at `~/Library/Application Support/com.Paralives.Paralives/Mods/`,
//! which is OUTSIDE the `.app` bundle — bundle code signing is preserved.
//!
//! Supported mod formats (all data-only, cross-platform):
//! `.fbx`, `.obj`, `.png`, `.jpg`, `.jpeg`, `.catalog`, `.ogg`, `.wav`,
//! `.json`, `.ttf`.
//!
//! UNSUPPORTED (refused at deploy time):
//! - `.exe`, `.dll`, `winhttp.dll`, `BepInEx/` payloads — these are
//!   Windows-only loaders OR require `codesign --remove-signature` on
//!   the .app bundle (BepInEx-mac). Both are deal-breakers we do NOT
//!   ship by default; Phase 1 Corkscrew supports official data mods only.
//!
//! BepInEx-style script mods are out of scope for the default deploy path.
//! The macOS BepInEx build requires `codesign --remove-signature` on the
//! .app bundle, which invalidates Apple Developer ID signing — an operation
//! Corkscrew does NOT automate. Users who want BepInEx script mods should
//! install BepInEx manually with explicit understanding of the signing
//! trade-off.
//!
//! Apple Silicon only (ARM64 native). Steam App ID 1118520.
//! CFBundleIdentifier best guess: `com.Paralives.Paralives` — TODO verify
//! with a real install by reading
//! `Paralives.app/Contents/Info.plist` CFBundleIdentifier.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::bottles::Bottle;
use crate::database::ModDatabase;
use crate::deployer::{DeployResult, DeployerError};
use crate::games::{DetectedGame, GamePlugin, LoadOrderKind};
use crate::staging::is_safe_relative_path;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// CFBundleIdentifier for Paralives.
///
/// Best guess derived from Unity's `Application.persistentDataPath` formula
/// (`com.<CompanyName>.<ProductName>`) and community-confirmed path at
/// `~/Library/Application Support/com.Paralives.Paralives/`.
/// TODO: verify with a real install.
const PARALIVES_BUNDLE_IDENTIFIER: &str = "com.Paralives.Paralives";

/// Executable name inside `Paralives.app/Contents/MacOS/`.
/// Inferred from Unity's ProductName convention; TODO verify with real install.
const PARALIVES_BUNDLE_EXECUTABLE: &str = "Paralives";

/// Steam App ID for Paralives.
const PARALIVES_STEAM_APP_ID: &str = "1118520";

/// Bottle sentinel for native mods (no Wine bottle).
const PARALIVES_NATIVE_BOTTLE_SENTINEL: &str = "";

// ---------------------------------------------------------------------------
// Plugin struct
// ---------------------------------------------------------------------------

/// Game plugin for Paralives (native macOS).
///
/// Deploys official data mods (feature mods, asset mods) to the Unity
/// persistent data path at `~/Library/Application Support/com.Paralives.Paralives/Mods/`.
/// The .app bundle is never touched; code signing is preserved.
pub struct ParalivesNativePlugin;

// ---------------------------------------------------------------------------
// Path resolution helpers
// ---------------------------------------------------------------------------

/// Returns the Paralives mods directory for the current user.
///
/// Resolves to `~/Library/Application Support/com.Paralives.Paralives/Mods/`
/// via `dirs::home_dir()`. The path is returned even if it does not yet exist
/// on disk — callers create it before use.
///
/// TODO: verify with a real install that this path is the one Paralives
/// actually uses. The in-game Mods button opens the folder on disk — check
/// which path it points to on first launch.
pub fn resolve_mods_dir() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    home.join("Library/Application Support/com.Paralives.Paralives/Mods")
}

// ---------------------------------------------------------------------------
// Detection helper (pure function for testability)
// ---------------------------------------------------------------------------

/// Filter `native_scanner` candidates to Paralives installs and produce
/// `DetectedGame` entries. Pure function — the `GamePlugin::detect_native`
/// impl wraps this with the actual scanner call.
///
/// Accepts candidates matching:
/// - `bundle_identifier == "com.Paralives.Paralives"` (primary), OR
/// - `bundle_executable == "Paralives"` (fallback for non-standard packaging).
///
/// Sandboxed candidates are always rejected.
fn detect_from_candidates(
    candidates: Vec<crate::native_scanner::NativeAppCandidate>,
) -> Vec<DetectedGame> {
    candidates
        .into_iter()
        .filter(|c| !c.sandboxed)
        .filter(|c| {
            c.info.bundle_identifier == PARALIVES_BUNDLE_IDENTIFIER
                || c.info.bundle_executable == PARALIVES_BUNDLE_EXECUTABLE
        })
        .map(|c| {
            let game_path = c.bundle_path.join("Contents").join("MacOS");
            let exe_path = Some(game_path.join("Paralives"));
            let data_dir = resolve_mods_dir();
            let steam_app_id = if c.source == crate::runtime::NativeSource::Steam {
                Some(PARALIVES_STEAM_APP_ID.to_string())
            } else {
                None
            };
            DetectedGame {
                game_id: "paralives_native".to_string(),
                display_name: "Paralives".to_string(),
                nexus_slug: "paralives".to_string(),
                game_path: game_path.clone(),
                exe_path,
                data_dir,
                runtime: crate::runtime::GameRuntime::Native(crate::runtime::NativeContext {
                    app_bundle_path: c.bundle_path,
                    game_data_root: game_path,
                    architecture: c.architecture,
                    sandboxed: false,
                    source: c.source,
                }),
                steam_app_id,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Inner (testable) deploy function
// ---------------------------------------------------------------------------

/// Core deployment logic for Paralives native mods.
///
/// Takes an explicit `mods_dir` argument rather than resolving from HOME
/// so that unit tests can redirect to tempdirs.
///
/// Algorithm:
/// 1. Verify `detected.runtime` is [`crate::runtime::GameRuntime::Native`]
///    and the game is not sandboxed.
/// 2. Create `mods_dir` if absent.
/// 3. Snapshot the current state (best-effort).
/// 4. Walk enabled mods; for each file in a mod's staging dir:
///    a. Reject Windows-only / BepInEx artifacts (`.exe`, `.dll`,
///       `winhttp.dll`, `doorstop_config.ini`, `BepInEx/` paths).
///    b. Validate the relative path (no traversal, no null bytes).
///    c. Copy (hardlink-first) the file to `mods_dir/<relative_path>`.
/// 5. Return [`DeployResult`] with the deployed file count.
pub fn deploy_native_inner(
    detected: &DetectedGame,
    db: &Arc<ModDatabase>,
    mods_dir: &Path,
) -> Result<DeployResult, DeployerError> {
    // 1. Reject non-native and sandboxed games.
    let native = detected
        .runtime
        .native()
        .ok_or_else(|| DeployerError::Other("expected native runtime for Paralives deploy".into()))?;
    if native.sandboxed {
        return Err(DeployerError::Other(format!(
            "native modding refused for sandboxed app: {}",
            native.app_bundle_path.display()
        )));
    }

    // 2. Create the mods directory.
    std::fs::create_dir_all(mods_dir)
        .map_err(|e| DeployerError::Other(format!("create mods_dir: {}", e)))?;

    // 3. Pre-deploy snapshot (best-effort — failure must not abort deploy).
    if let Err(e) = crate::rollback::create_native_snapshot(
        db,
        &detected.game_id,
        "paralives-deploy",
        &format!("Paralives deploy to {}", mods_dir.display()),
    ) {
        log::warn!("snapshot before Paralives deploy failed: {}", e);
    }

    // 4. Walk enabled mods from the database.
    let enabled_mods = db
        .list_mods(&detected.game_id, PARALIVES_NATIVE_BOTTLE_SENTINEL)
        .map_err(|e| DeployerError::Database(e.to_string()))?;

    // Canonicalise mods_dir once for destination-escape checks.
    let canonical_mods_dir = mods_dir
        .canonicalize()
        .unwrap_or_else(|_| mods_dir.to_path_buf());

    let mut deployed_count = 0usize;
    let mut fallback_used = false;

    for installed_mod in enabled_mods.iter().filter(|m| m.enabled) {
        let staging_dir = match &installed_mod.staging_path {
            Some(p) => PathBuf::from(p),
            None => {
                log::warn!(
                    "paralives deploy_native: mod '{}' has no staging path, skipping",
                    installed_mod.name
                );
                continue;
            }
        };

        if !staging_dir.exists() {
            log::warn!(
                "paralives deploy_native: staging dir missing for mod '{}': {}",
                installed_mod.name,
                staging_dir.display()
            );
            continue;
        }

        for entry in walkdir::WalkDir::new(&staging_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();
            let rel = match path.strip_prefix(&staging_dir) {
                Ok(r) => r,
                Err(_) => continue,
            };
            let rel_str = rel.to_string_lossy();

            // a. Reject Windows-only / BepInEx-style artifacts.
            if rejects_paralives_artifact(&rel_str) {
                return Err(DeployerError::Other(format!(
                    "unsupported Paralives artifact (Windows-only or BepInEx loader) \
                     in mod '{}': {}",
                    installed_mod.name, rel_str
                )));
            }

            // b. Validate relative path safety (no traversal, null bytes, drive letters).
            if !is_safe_relative_path(&rel_str) {
                return Err(DeployerError::Other(format!(
                    "unsafe mod path in mod '{}': {}",
                    installed_mod.name, rel_str
                )));
            }

            let dest = mods_dir.join(rel);

            // c. Post-join canonicalization: verify dest stays inside mods_dir.
            let dest_parent = dest.parent().unwrap_or(mods_dir);
            std::fs::create_dir_all(dest_parent)
                .map_err(|e| DeployerError::Other(format!("create dest parent: {}", e)))?;
            let canonical_parent = dest_parent
                .canonicalize()
                .unwrap_or_else(|_| dest_parent.to_path_buf());
            if !canonical_parent.starts_with(&canonical_mods_dir) {
                return Err(DeployerError::Other(format!(
                    "destination escapes mods_dir: {}",
                    dest.display()
                )));
            }

            // Remove existing file so hardlink does not fail with EEXIST.
            if dest.exists() {
                let _ = std::fs::remove_file(&dest);
            }

            if std::fs::hard_link(path, &dest).is_err() {
                std::fs::copy(path, &dest)
                    .map_err(|e| DeployerError::Other(format!("copy mod file: {}", e)))?;
                fallback_used = true;
            }

            deployed_count += 1;
        }
    }

    Ok(DeployResult {
        deployed_count,
        skipped_count: 0,
        fallback_used,
    })
}

// ---------------------------------------------------------------------------
// Artifact rejection predicate
// ---------------------------------------------------------------------------

/// Returns `true` if the relative file path is a Windows-only loader,
/// BepInEx payload, or other artifact that Paralives' macOS data-mod
/// runtime cannot use.
///
/// Rejected patterns:
/// - `.exe` — Windows executables
/// - `.dll` — Windows DLLs (BepInEx plugin assemblies, loaders)
/// - `winhttp.dll` — the BepInEx Doorstop hook DLL (Windows-specific)
/// - `doorstop_config.ini` — BepInEx Doorstop configuration (Windows)
/// - Any path containing `BepInEx/` — BepInEx plugin layout
///
/// Note: `.dll` is rejected broadly because Paralives' official macOS
/// mod runtime loads only data-only formats (see module doc). When/if
/// Corkscrew adds a dedicated BepInEx-mac plugin, it will handle `.dll`
/// through its own specialised pipeline.
pub fn rejects_paralives_artifact(rel_path: &str) -> bool {
    let lower = rel_path.replace('\\', "/").to_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);

    // Windows binaries
    if name.ends_with(".exe") || name.ends_with(".dll") {
        return true;
    }
    // BepInEx loader markers
    if name == "winhttp.dll" || name == "doorstop_config.ini" {
        return true;
    }
    // BepInEx directory layout
    if lower.contains("/bepinex/") || lower.starts_with("bepinex/") {
        return true;
    }

    false
}

// ---------------------------------------------------------------------------
// GamePlugin impl
// ---------------------------------------------------------------------------

impl GamePlugin for ParalivesNativePlugin {
    fn game_id(&self) -> &str {
        "paralives_native"
    }

    fn display_name(&self) -> &str {
        "Paralives (Native)"
    }

    fn nexus_slug(&self) -> &str {
        "paralives"
    }

    fn executables(&self) -> &[&str] {
        &[PARALIVES_BUNDLE_EXECUTABLE]
    }

    fn detect_wine(&self, _bottle: &Bottle) -> Option<DetectedGame> {
        // Paralives is Apple Silicon-only; Wine/CrossOver not applicable.
        None
    }

    fn detect_native(&self) -> Vec<DetectedGame> {
        detect_from_candidates(crate::native_scanner::scan_all_native())
    }

    /// Returns the mods directory for Paralives.
    ///
    /// Note: the true Paralives mods directory is at the Unity persistent data
    /// path (`~/Library/Application Support/com.Paralives.Paralives/Mods/`),
    /// independent of the .app bundle location. Use [`resolve_mods_dir`] for
    /// deployment; this method is used by the generic mod install pipeline.
    fn get_data_dir(&self, _game_path: &Path) -> PathBuf {
        resolve_mods_dir()
    }

    /// Paralives has no `plugins.txt`-style load order file.
    ///
    /// Mod activation is managed by the in-game Mods menu; Corkscrew's role
    /// is file deployment only.
    fn get_plugins_file(&self, _game_path: &Path, _bottle: &Bottle) -> Option<PathBuf> {
        None
    }

    /// Paralives has no load order — data mods are not ordered.
    fn load_order_kind(&self, _game_path: &Path) -> LoadOrderKind {
        LoadOrderKind::None
    }

    /// Deploy all staged Paralives mods into the Unity persistent data mods dir.
    ///
    /// Thin wrapper that resolves the canonical macOS path via
    /// [`resolve_mods_dir`] and delegates all work to the testable
    /// [`deploy_native_inner`].
    fn deploy_native(
        &self,
        detected: &DetectedGame,
        db: &Arc<ModDatabase>,
    ) -> std::result::Result<DeployResult, DeployerError> {
        deploy_native_inner(detected, db, &resolve_mods_dir())
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub fn register() {
    crate::games::register_plugin(Arc::new(ParalivesNativePlugin));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::with_plugin;
    use crate::runtime::{Architecture, NativeSource};

    // ── Test infrastructure ─────────────────────────────────────────────────

    /// Build a synthetic `NativeAppCandidate` for use in unit tests.
    ///
    /// Only `bundle_identifier`, `bundle_executable`, `sandboxed`, `source`,
    /// and `architecture` are read by the detection logic — other InfoPlist
    /// fields are left as `None`.
    fn fake_candidate(
        bundle_path: &str,
        bundle_id: &str,
        exe_name: &str,
        sandboxed: bool,
        source: NativeSource,
        arch: Architecture,
    ) -> crate::native_scanner::NativeAppCandidate {
        crate::native_scanner::NativeAppCandidate {
            bundle_path: std::path::PathBuf::from(bundle_path),
            info: crate::plist::InfoPlist {
                bundle_identifier: bundle_id.to_string(),
                bundle_executable: exe_name.to_string(),
                short_version: None,
                category: None,
            },
            architecture: arch,
            source,
            sandboxed,
        }
    }

    /// Build a synthetic `DetectedGame` with a native runtime for Paralives.
    fn fake_detected_native(bundle_path: &Path) -> DetectedGame {
        let game_path = bundle_path.join("Contents").join("MacOS");
        DetectedGame {
            game_id: "paralives_native".into(),
            display_name: "Paralives".into(),
            nexus_slug: "paralives".into(),
            game_path: game_path.clone(),
            exe_path: Some(game_path.join("Paralives")),
            data_dir: resolve_mods_dir(),
            runtime: crate::runtime::GameRuntime::Native(crate::runtime::NativeContext {
                app_bundle_path: bundle_path.to_path_buf(),
                game_data_root: game_path,
                architecture: Architecture::AppleSilicon,
                sandboxed: false,
                source: NativeSource::Steam,
            }),
            steam_app_id: Some(PARALIVES_STEAM_APP_ID.to_string()),
        }
    }

    // ── 1. Plugin metadata ──────────────────────────────────────────────────

    #[test]
    fn paralives_plugin_metadata() {
        let plugin = ParalivesNativePlugin;
        assert_eq!(plugin.game_id(), "paralives_native");
        assert_eq!(plugin.display_name(), "Paralives (Native)");
        assert_eq!(plugin.nexus_slug(), "paralives");
    }

    // ── 2. Detection: filter by bundle identifier ───────────────────────────

    #[test]
    fn paralives_detect_native_filters_by_bundle_id() {
        let candidates = vec![
            fake_candidate(
                "/Users/user/Library/Application Support/Steam/steamapps/common/Paralives/Paralives.app",
                PARALIVES_BUNDLE_IDENTIFIER,
                "Paralives",
                false,
                NativeSource::Steam,
                Architecture::AppleSilicon,
            ),
            fake_candidate(
                "/Applications/OtherGame.app",
                "com.other.game",
                "OtherGame",
                false,
                NativeSource::SystemApplications,
                Architecture::Universal,
            ),
        ];

        let results = detect_from_candidates(candidates);
        assert_eq!(results.len(), 1, "only the Paralives bundle should match");
        assert_eq!(results[0].game_id, "paralives_native");
        assert_eq!(results[0].display_name, "Paralives");
    }

    // ── 3. Detection: executable-name fallback ──────────────────────────────

    #[test]
    fn paralives_detect_native_falls_back_to_executable_name() {
        let candidates = vec![fake_candidate(
            "/Users/user/Games/Paralives/Paralives.app",
            "com.unknown.paralives", // non-standard bundle id
            "Paralives",             // matching executable name
            false,
            NativeSource::Manual,
            Architecture::AppleSilicon,
        )];

        let results = detect_from_candidates(candidates);
        assert_eq!(
            results.len(),
            1,
            "executable-name fallback should match non-standard bundle"
        );
        assert_eq!(results[0].game_id, "paralives_native");
    }

    // ── 4. Detection: sandboxed candidates skipped ─────────────────────────

    #[test]
    fn paralives_detect_native_skips_sandboxed() {
        let candidates = vec![
            fake_candidate(
                "/Applications/Paralives.app",
                PARALIVES_BUNDLE_IDENTIFIER,
                "Paralives",
                true, // sandboxed
                NativeSource::SystemApplications,
                Architecture::AppleSilicon,
            ),
            fake_candidate(
                "/Users/user/Games/Paralives.app",
                PARALIVES_BUNDLE_IDENTIFIER,
                "Paralives",
                false, // not sandboxed
                NativeSource::Steam,
                Architecture::AppleSilicon,
            ),
        ];

        let results = detect_from_candidates(candidates);
        assert_eq!(results.len(), 1, "sandboxed candidate must be skipped");
        let ctx = match &results[0].runtime {
            crate::runtime::GameRuntime::Native(n) => n,
            _ => panic!("expected Native runtime"),
        };
        assert!(!ctx.sandboxed);
    }

    // ── 5. Detection: Steam source populates steam_app_id ──────────────────

    #[test]
    fn paralives_detect_native_populates_steam_app_id_for_steam_source() {
        let candidates = vec![fake_candidate(
            "/Users/user/Library/Application Support/Steam/steamapps/common/Paralives/Paralives.app",
            PARALIVES_BUNDLE_IDENTIFIER,
            "Paralives",
            false,
            NativeSource::Steam,
            Architecture::AppleSilicon,
        )];

        let results = detect_from_candidates(candidates);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].steam_app_id,
            Some(PARALIVES_STEAM_APP_ID.to_string()),
            "Steam source must set steam_app_id"
        );
    }

    // ── 6. Detection: non-Steam sources omit steam_app_id ──────────────────

    #[test]
    fn paralives_detect_native_omits_steam_app_id_for_other_sources() {
        let non_steam_sources = [
            NativeSource::Gog,
            NativeSource::Manual,
            NativeSource::SystemApplications,
        ];

        for source in non_steam_sources {
            let candidates = vec![fake_candidate(
                "/Applications/Paralives.app",
                PARALIVES_BUNDLE_IDENTIFIER,
                "Paralives",
                false,
                source,
                Architecture::AppleSilicon,
            )];
            let results = detect_from_candidates(candidates);
            assert_eq!(results.len(), 1);
            assert!(
                results[0].steam_app_id.is_none(),
                "source {:?} must not set steam_app_id",
                source
            );
        }
    }

    // ── 7. resolve_mods_dir returns persistent data path ───────────────────

    #[test]
    fn paralives_resolve_mods_dir_returns_persistent_data_path() {
        let mods_dir = resolve_mods_dir();
        let home = dirs::home_dir().expect("home dir must be available in test environment");
        let expected = home.join("Library/Application Support/com.Paralives.Paralives/Mods");
        assert_eq!(
            mods_dir, expected,
            "mods dir must be the Unity persistent data path"
        );
    }

    // ── 8. Artifact rejection: Windows .exe ─────────────────────────────────

    #[test]
    fn rejects_paralives_artifact_rejects_windows_exe() {
        assert!(
            rejects_paralives_artifact("CheatMod.exe"),
            ".exe must be rejected"
        );
        assert!(
            rejects_paralives_artifact("subdir/launcher.exe"),
            ".exe in subdirectory must be rejected"
        );
    }

    // ── 9. Artifact rejection: .dll ─────────────────────────────────────────

    #[test]
    fn rejects_paralives_artifact_rejects_dll() {
        assert!(
            rejects_paralives_artifact("CheatMod.dll"),
            ".dll must be rejected"
        );
        assert!(
            rejects_paralives_artifact("BepInEx/plugins/CheatMod.dll"),
            ".dll in BepInEx layout must be rejected"
        );
    }

    // ── 10. Artifact rejection: winhttp.dll loader ──────────────────────────

    #[test]
    fn rejects_paralives_artifact_rejects_winhttp_loader() {
        assert!(
            rejects_paralives_artifact("winhttp.dll"),
            "winhttp.dll must be rejected"
        );
        assert!(
            rejects_paralives_artifact("WINHTTP.DLL"),
            "winhttp.dll (uppercase) must be rejected"
        );
        assert!(
            rejects_paralives_artifact("doorstop_config.ini"),
            "doorstop_config.ini must be rejected"
        );
    }

    // ── 11. Artifact rejection: BepInEx path ────────────────────────────────

    #[test]
    fn rejects_paralives_artifact_rejects_bepinex_path() {
        assert!(
            rejects_paralives_artifact("BepInEx/plugins/SomeMod.dll"),
            "BepInEx/ path must be rejected"
        );
        assert!(
            rejects_paralives_artifact("BEPINEX/core/BepInEx.dll"),
            "BepInEx/ path (uppercase) must be rejected"
        );
        // Windows path separator normalised to /
        assert!(
            rejects_paralives_artifact("BepInEx\\plugins\\mod.dll"),
            "BepInEx\\ (Windows separator) must be rejected"
        );
    }

    // ── 12. Artifact acceptance: official data-mod formats ──────────────────

    #[test]
    fn rejects_paralives_artifact_accepts_png_fbx_json_etc() {
        let accepted = [
            "textures/skin_01.png",
            "models/chair.fbx",
            "models/table.obj",
            "ui/button.jpg",
            "ui/icon.jpeg",
            "animations/walk.catalog",
            "audio/ambient.ogg",
            "audio/click.wav",
            "config/traits.json",
            "fonts/custom.ttf",
        ];
        for path in &accepted {
            assert!(
                !rejects_paralives_artifact(path),
                "'{}' should be accepted as a valid Paralives data-mod format",
                path
            );
        }
    }

    // ── deploy_native_inner: sandbox refusal ────────────────────────────────

    #[test]
    fn paralives_deploy_native_refuses_sandboxed_game() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp.path().join("Mods");

        let db_path = tmp.path().join("test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());

        let detected = DetectedGame {
            game_id: "paralives_native".into(),
            display_name: "Paralives".into(),
            nexus_slug: "paralives".into(),
            game_path: tmp.path().to_path_buf(),
            exe_path: None,
            data_dir: resolve_mods_dir(),
            runtime: crate::runtime::GameRuntime::Native(crate::runtime::NativeContext {
                app_bundle_path: tmp.path().to_path_buf(),
                game_data_root: tmp.path().to_path_buf(),
                architecture: Architecture::AppleSilicon,
                sandboxed: true, // sandboxed
                source: NativeSource::Steam,
            }),
            steam_app_id: Some(PARALIVES_STEAM_APP_ID.to_string()),
        };

        let result = deploy_native_inner(&detected, &db, &mods_dir);
        assert!(result.is_err(), "must refuse sandboxed game");
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("sandboxed"),
            "error must mention 'sandboxed': {msg}"
        );
        assert!(
            !mods_dir.exists(),
            "mods_dir must not be created for sandboxed game"
        );
    }

    // ── deploy_native_inner: snapshot created ───────────────────────────────

    #[test]
    fn paralives_deploy_native_creates_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp.path().join("Mods");

        let db_path = tmp.path().join("test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());
        crate::rollback::init_schema(&db).unwrap();

        // Add a staging mod with a simple data file.
        let staging_dir = tmp.path().join("staging");
        std::fs::create_dir_all(&staging_dir).unwrap();
        std::fs::write(staging_dir.join("mod_data.json"), b"{}").unwrap();

        let mod_id = db
            .add_mod(
                "paralives_native",
                "",
                None,
                "Test Mod",
                "1.0.0",
                "test_mod.zip",
                &[],
            )
            .unwrap();
        db.set_staging_path(mod_id, &staging_dir.to_string_lossy())
            .unwrap();

        let detected = fake_detected_native(tmp.path());
        deploy_native_inner(&detected, &db, &mods_dir).unwrap();

        let snapshots =
            crate::rollback::list_snapshots(&db, "paralives_native", "").unwrap();
        assert!(
            !snapshots.is_empty(),
            "deploy_native_inner should create a snapshot"
        );
        assert_eq!(
            snapshots[0].name, "paralives-deploy",
            "snapshot name must be 'paralives-deploy'"
        );
    }

    // ── deploy_native_inner: happy path ─────────────────────────────────────

    #[test]
    fn paralives_deploy_native_copies_files_to_mods_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp.path().join("Mods");

        let db_path = tmp.path().join("test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());

        let staging_dir = tmp.path().join("staging");
        std::fs::create_dir_all(staging_dir.join("textures")).unwrap();
        std::fs::write(staging_dir.join("config.json"), b"{}").unwrap();
        std::fs::write(staging_dir.join("textures/skin.png"), b"PNG").unwrap();

        let mod_id = db
            .add_mod(
                "paralives_native",
                "",
                None,
                "Asset Mod",
                "1.0.0",
                "asset_mod.zip",
                &[],
            )
            .unwrap();
        db.set_staging_path(mod_id, &staging_dir.to_string_lossy())
            .unwrap();

        let detected = fake_detected_native(tmp.path());
        let result = deploy_native_inner(&detected, &db, &mods_dir).unwrap();

        assert_eq!(result.deployed_count, 2, "config.json + textures/skin.png");
        assert!(mods_dir.join("config.json").exists());
        assert!(mods_dir.join("textures/skin.png").exists());
    }

    // ── deploy_native_inner: reject BepInEx artifact ────────────────────────

    #[test]
    fn paralives_deploy_native_refuses_bepinex_dll() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp.path().join("Mods");

        let db_path = tmp.path().join("test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());

        let staging_dir = tmp.path().join("staging");
        std::fs::create_dir_all(staging_dir.join("BepInEx/plugins")).unwrap();
        std::fs::write(
            staging_dir.join("BepInEx/plugins/ScriptMod.dll"),
            b"MZ fake dll",
        )
        .unwrap();

        let mod_id = db
            .add_mod(
                "paralives_native",
                "",
                None,
                "Script Mod",
                "1.0.0",
                "script_mod.zip",
                &[],
            )
            .unwrap();
        db.set_staging_path(mod_id, &staging_dir.to_string_lossy())
            .unwrap();

        let detected = fake_detected_native(tmp.path());
        let result = deploy_native_inner(&detected, &db, &mods_dir);
        assert!(result.is_err(), "BepInEx dll must be rejected");
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("unsupported Paralives artifact"),
            "error must mention unsupported artifact: {msg}"
        );
    }

    // ── Plugin registration ─────────────────────────────────────────────────

    #[test]
    fn paralives_native_plugin_registers() {
        crate::games::register_plugin(std::sync::Arc::new(ParalivesNativePlugin));
        let result = with_plugin("paralives_native", |p| p.display_name().to_owned());
        assert_eq!(result, Some("Paralives (Native)".to_owned()));
    }
}
