//! Crimson Desert (native macOS) game plugin.
//!
//! Pearl Abyss's open-world action RPG, native Apple Silicon build
//! released March 19, 2026 (macOS 15.0+ required). Bundle identifier
//! `com.pearlabyss.CrimsonDesert`, Steam App ID `3321460`.
//!
//! Architecture: Apple Silicon ONLY. Intel users on Rosetta will NOT
//! see this plugin produce a detection — the spike confirmed no Intel
//! binary slice is shipped.
//!
//! Modding posture: Pearl Abyss unofficially tolerates client-side
//! mods. The game ships Denuvo Anti-Tamper (DRM only — no XignCode/EQU8
//! anti-cheat). Mods carry user-acceptance risk under the EULA but
//! there are no documented ban incidents and the offline mode insulates
//! the user from server-side detection.
//!
//! Mod architecture: PAZ overlay groups numbered `0036+` (vanilla ships
//! `0000`–`0035`). A mod ships a pre-built overlay directory containing
//! `<group>/0.paz` + `<group>/0.pamt` and the registration is appended
//! to `meta/0.papgt`. This Corkscrew plugin implements the deploy
//! orchestration; the PAZ format itself is opaque to us — we never read
//! or write inside `.paz` archives. Format-aware mod tooling (JSON
//! byte-patches that need PAZ extraction) is a separate future project,
//! not Phase 1.
//!
//! DEPLOY IS BLOCKED on verifying the PAZ overlay tree location: does
//! it live at `<game_install>/Paz/` (writable, no signing impact) or
//! inside `.app/Contents/Resources/` (writing breaks code signing)?
//! Until verified on a real install, deploy_native returns a typed
//! BLOCKED error explaining the situation.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::bottles::Bottle;
use crate::database::ModDatabase;
use crate::deployer::{DeployResult, DeployerError};
use crate::games::{DetectedGame, GamePlugin};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const CD_BUNDLE_IDENTIFIER: &str = "com.pearlabyss.CrimsonDesert";

/// Candidate executable names inside `Contents/MacOS/`.
///
/// The authoritative name is an open spike question (see spec open question #2).
/// These cover known possibilities from community tools and Pearl Abyss naming
/// conventions — `CrimsonDesert_Steam` is a community-observed Steam wrapper
/// variant that may or may not exist. Refine after verifying a real install.
///
/// TODO: verify with real install — run `ls "<app>/Contents/MacOS/"`.
const CD_BUNDLE_EXECUTABLE: &str = "Crimson Desert";

const CD_STEAM_APP_ID: &str = "3321460";

// ---------------------------------------------------------------------------
// Plugin struct
// ---------------------------------------------------------------------------

pub struct CrimsonDesertNativePlugin;

// ---------------------------------------------------------------------------
// Detection helper (testable)
// ---------------------------------------------------------------------------

/// Core detection logic. Accepts a pre-scanned candidate list so unit tests
/// can inject synthetic candidates without touching the real filesystem.
fn detect_from_candidates(
    candidates: Vec<crate::native_scanner::NativeAppCandidate>,
) -> Vec<DetectedGame> {
    candidates
        .into_iter()
        // Sandbox refusal: App Store sandboxed bundles cannot be modded.
        // The sandboxed game writes into its own container; Corkscrew cannot
        // reach that container without violating the App Sandbox.
        .filter(|c| !c.sandboxed)
        // Apple Silicon only — no Intel build exists. If we somehow encounter
        // an Intel-only candidate (unexpected), skip it rather than producing a
        // DetectedGame that would fail at deploy time.
        .filter(|c| c.architecture != crate::runtime::Architecture::IntelOnly)
        .filter(|c| {
            // Case-insensitive — Info.plist authors are inconsistent about casing.
            c.info.bundle_identifier.eq_ignore_ascii_case(CD_BUNDLE_IDENTIFIER)
                || c.info.bundle_executable.eq_ignore_ascii_case(CD_BUNDLE_EXECUTABLE)
                // Also accept the no-space community variant.
                || c.info.bundle_executable.eq_ignore_ascii_case("CrimsonDesert")
        })
        .map(|c| {
            // Game install root: for Steam, the .app lives inside the Steam
            // common dir, e.g. `common/Crimson Desert/CrimsonDesert_Steam.app`.
            // The PAZ group tree is expected at the *parent* of the .app bundle
            // (the install root), not inside the bundle itself.
            //
            // OPEN QUESTION (spike §10.3 / §10.5): if PAZ groups turn out to
            // live inside `Contents/Resources/`, this mapping must be revised.
            // Until then, install_root = .app parent is the working assumption.
            let install_root = c
                .bundle_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| c.bundle_path.clone());

            let game_path = install_root.clone();

            // Executable path inside the bundle's MacOS directory.
            let exe_path = Some(
                c.bundle_path
                    .join("Contents")
                    .join("MacOS")
                    .join(&c.info.bundle_executable),
            );

            // PAZ overlay tree lives at <game_install>/Paz/ — pending verification.
            // See spec open question #5: if groups are inside Contents/Resources/
            // this breaks code signing and the deploy path must be redesigned.
            let data_dir = game_path.join("Paz");

            let steam_app_id = if c.source == crate::runtime::NativeSource::Steam {
                Some(CD_STEAM_APP_ID.to_string())
            } else {
                None
            };

            DetectedGame {
                game_id: "crimson_desert_native".to_string(),
                display_name: "Crimson Desert".to_string(),
                nexus_slug: "crimsondesert".to_string(),
                game_path,
                exe_path,
                data_dir,
                runtime: crate::runtime::GameRuntime::Native(crate::runtime::NativeContext {
                    app_bundle_path: c.bundle_path,
                    game_data_root: install_root,
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
// GamePlugin impl
// ---------------------------------------------------------------------------

impl GamePlugin for CrimsonDesertNativePlugin {
    fn game_id(&self) -> &str {
        "crimson_desert_native"
    }

    fn display_name(&self) -> &str {
        "Crimson Desert (Native)"
    }

    fn nexus_slug(&self) -> &str {
        "crimsondesert"
    }

    fn executables(&self) -> &[&str] {
        // Executable names inside Contents/MacOS/. Space variant is the
        // typical Pearl Abyss macOS naming; no-space variant is a community-
        // observed fallback. See spike open question #2 for TODO.
        &["Crimson Desert", "CrimsonDesert"]
    }

    /// Wine detection is not applicable — this is a native-only plugin.
    fn detect_wine(&self, _bottle: &Bottle) -> Option<DetectedGame> {
        None
    }

    fn detect_native(&self) -> Vec<DetectedGame> {
        detect_from_candidates(crate::native_scanner::scan_all_native())
    }

    /// Returns the PAZ overlay tree root relative to the game install path.
    ///
    /// Working assumption: `<game_install>/Paz/`. If PAZ groups live inside
    /// `.app/Contents/Resources/` (spike open question #5), this must be
    /// updated — writing into the bundle would break code signing and require
    /// a different deploy strategy.
    fn get_data_dir(&self, game_path: &Path) -> PathBuf {
        game_path.join("Paz")
    }

    /// Crimson Desert has no plugin/load-order manifest.
    fn get_plugins_file(&self, _game_path: &Path, _bottle: &Bottle) -> Option<PathBuf> {
        None
    }

    /// Native deploy — BLOCKED pending PAZ overlay tree location verification.
    ///
    /// The spike's most critical open question (§10.5) asks whether PAZ overlay
    /// groups live at `<game_install>/Paz/` (safe — outside the .app bundle) or
    /// inside `.app/Contents/Resources/` (unsafe — writing there invalidates the
    /// Apple Developer ID signature). Until verified on a real install, this
    /// method returns a typed error rather than silently deploying to the wrong
    /// location or breaking game signing.
    ///
    /// When the deploy path is verified:
    /// 1. Remove this Err block.
    /// 2. Implement the PAZ overlay copy + `meta/0.papgt` registration.
    ///    See spec §9 (Step 4) for the algorithm.
    /// 3. Set `VERIFIED = true` in this file.
    /// 4. Update docs/superpowers/plans/2026-05-02-crimson-desert-native-spec.md
    ///    to mark open question #5 as resolved.
    fn deploy_native(
        &self,
        detected: &DetectedGame,
        _db: &Arc<ModDatabase>,
    ) -> std::result::Result<DeployResult, DeployerError> {
        let native = detected
            .runtime
            .native()
            .ok_or_else(|| DeployerError::Other("expected native runtime".into()))?;

        // Sandbox refusal at the deploy layer (belt-and-suspenders — detection
        // already filters sandboxed candidates, but a DetectedGame could arrive
        // via a manual path).
        if native.sandboxed {
            return Err(DeployerError::Other(format!(
                "native modding refused for sandboxed app: {}. \
                 Use the Steam version of Crimson Desert instead.",
                native.app_bundle_path.display()
            )));
        }

        // Apple Silicon-only check. No Intel build of Crimson Desert exists;
        // if IntelOnly somehow slips through, reject it explicitly so the error
        // is legible rather than silently deploying to a broken state.
        if native.architecture == crate::runtime::Architecture::IntelOnly {
            return Err(DeployerError::Other(
                "Crimson Desert is Apple Silicon only — Intel binary detected (unexpected). \
                 Rosetta 2 is not a supported mod target for this plugin."
                    .into(),
            ));
        }

        // BLOCKED: PAZ overlay location unverified.
        //
        // The spike (docs/superpowers/plans/2026-05-02-crimson-desert-native-spec.md,
        // open question #5) asks whether overlay groups live at <game_install>/Paz/
        // (writable, safe) or inside .app/Contents/Resources/ (breaks code signing).
        // Do NOT remove this Err until that question is answered on a real install.
        Err(DeployerError::Other(
            "Crimson Desert native deploy blocked: PAZ overlay tree location unverified. \
             Open question: do overlay groups live at <game_install>/Paz/ (safe, outside .app) \
             or inside .app/Contents/Resources/ (breaks Apple Developer ID code signing)? \
             Verify on a real install before enabling deploy. \
             See docs/superpowers/plans/2026-05-02-crimson-desert-native-spec.md §10.5."
                .into(),
        ))
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub fn register() {
    crate::games::register_plugin(Arc::new(CrimsonDesertNativePlugin));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_scanner::NativeAppCandidate;
    use crate::plist::InfoPlist;
    use crate::runtime::{Architecture, GameRuntime, NativeContext, NativeSource};
    use std::path::PathBuf;

    // ── Test helpers ────────────────────────────────────────────────────────

    /// Build a synthetic `NativeAppCandidate` for use in unit tests.
    ///
    /// The detection logic reads `bundle_identifier` and `bundle_executable`
    /// from `info`, and `sandboxed` / `source` / `architecture` from the
    /// candidate directly. The remaining `InfoPlist` fields are unused in
    /// detection and are given zero-value stubs here.
    fn fake_candidate(
        bundle_path: &str,
        bundle_id: &str,
        exe_name: &str,
        sandboxed: bool,
        source: NativeSource,
        arch: Architecture,
    ) -> NativeAppCandidate {
        NativeAppCandidate {
            bundle_path: PathBuf::from(bundle_path),
            info: InfoPlist {
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

    /// Build a synthetic `DetectedGame` with a (non-sandboxed) native runtime.
    fn fake_detected(game_path: &str) -> DetectedGame {
        DetectedGame {
            game_id: "crimson_desert_native".into(),
            display_name: "Crimson Desert".into(),
            nexus_slug: "crimsondesert".into(),
            game_path: PathBuf::from(game_path),
            exe_path: None,
            data_dir: PathBuf::from(game_path).join("Paz"),
            runtime: GameRuntime::Native(NativeContext {
                app_bundle_path: PathBuf::from(format!("{}/Crimson Desert.app", game_path)),
                game_data_root: PathBuf::from(game_path),
                architecture: Architecture::AppleSilicon,
                sandboxed: false,
                source: NativeSource::Steam,
            }),
            steam_app_id: Some(CD_STEAM_APP_ID.into()),
        }
    }

    // ── Metadata ─────────────────────────────────────────────────────────────

    #[test]
    fn crimson_desert_plugin_metadata() {
        let p = CrimsonDesertNativePlugin;
        assert_eq!(p.game_id(), "crimson_desert_native");
        assert_eq!(p.display_name(), "Crimson Desert (Native)");
        assert_eq!(p.nexus_slug(), "crimsondesert");
        assert!(!p.executables().is_empty());
    }

    // ── Detection filter: bundle identifier ──────────────────────────────────

    #[test]
    fn detect_native_filters_by_bundle_id() {
        let candidates = vec![
            fake_candidate(
                "/Applications/Crimson Desert.app",
                "com.pearlabyss.CrimsonDesert",
                "Crimson Desert",
                false,
                NativeSource::Steam,
                Architecture::AppleSilicon,
            ),
            fake_candidate(
                "/Applications/Other.app",
                "com.other.app",
                "Other",
                false,
                NativeSource::SystemApplications,
                Architecture::AppleSilicon,
            ),
        ];
        let result = detect_from_candidates(candidates);
        assert_eq!(result.len(), 1, "only the CD candidate should match");
        assert_eq!(result[0].game_id, "crimson_desert_native");
    }

    // ── Detection filter: Intel-only skipped ────────────────────────────────

    #[test]
    fn detect_native_skips_intel_only_candidates() {
        let candidates = vec![fake_candidate(
            "/Applications/Crimson Desert.app",
            "com.pearlabyss.CrimsonDesert",
            "Crimson Desert",
            false,
            NativeSource::Steam,
            Architecture::IntelOnly,
        )];
        let result = detect_from_candidates(candidates);
        assert_eq!(
            result.len(),
            0,
            "Intel-only Crimson Desert should not match — no Intel build exists"
        );
    }

    // ── Detection filter: sandboxed skipped ─────────────────────────────────

    #[test]
    fn detect_native_skips_sandboxed() {
        let candidates = vec![fake_candidate(
            "/Applications/Crimson Desert.app",
            "com.pearlabyss.CrimsonDesert",
            "Crimson Desert",
            true, // sandboxed = App Store version
            NativeSource::AppStore,
            Architecture::AppleSilicon,
        )];
        let result = detect_from_candidates(candidates);
        assert_eq!(result.len(), 0, "sandboxed App Store version must be filtered");
    }

    // ── Detection: Steam app ID propagation ─────────────────────────────────

    #[test]
    fn detect_native_populates_steam_app_id_for_steam_source() {
        let candidates = vec![fake_candidate(
            "/Applications/Crimson Desert.app",
            "com.pearlabyss.CrimsonDesert",
            "Crimson Desert",
            false,
            NativeSource::Steam,
            Architecture::AppleSilicon,
        )];
        let result = detect_from_candidates(candidates);
        assert_eq!(
            result[0].steam_app_id.as_deref(),
            Some("3321460"),
            "Steam source must set steam_app_id"
        );
    }

    #[test]
    fn detect_native_omits_steam_app_id_for_manual_source() {
        let candidates = vec![fake_candidate(
            "/Applications/Crimson Desert.app",
            "com.pearlabyss.CrimsonDesert",
            "Crimson Desert",
            false,
            NativeSource::Manual,
            Architecture::AppleSilicon,
        )];
        let result = detect_from_candidates(candidates);
        assert_eq!(
            result[0].steam_app_id, None,
            "Manual source must not set steam_app_id"
        );
    }

    // ── Detection: executable-name fallback ────────────────────────────────

    #[test]
    fn detect_native_falls_back_to_executable_name() {
        // Unrecognised bundle ID, but bundle_executable matches CD_BUNDLE_EXECUTABLE.
        let candidates = vec![fake_candidate(
            "/Applications/CD.app",
            "com.unknown.foo",
            "Crimson Desert",
            false,
            NativeSource::Steam,
            Architecture::AppleSilicon,
        )];
        let result = detect_from_candidates(candidates);
        assert_eq!(result.len(), 1, "executable-name fallback should produce a match");
    }

    // ── Data dir ─────────────────────────────────────────────────────────────

    #[test]
    fn data_dir_is_paz_subdir_of_install_root() {
        let p = CrimsonDesertNativePlugin;
        let install_root = PathBuf::from(
            "/Users/x/Library/Application Support/Steam/steamapps/common/Crimson Desert",
        );
        assert_eq!(
            p.get_data_dir(&install_root),
            install_root.join("Paz"),
            "data_dir must be <install_root>/Paz"
        );
    }

    // ── Deploy blocked ────────────────────────────────────────────────────────

    #[test]
    fn deploy_native_returns_blocked_error_until_path_verified() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(
            crate::database::ModDatabase::new(&tmp.path().join("test.db"))
                .expect("test db"),
        );
        let detected = fake_detected("/fake/install");
        let result = CrimsonDesertNativePlugin.deploy_native(&detected, &db);
        assert!(result.is_err(), "deploy_native must return Err when deploy is blocked");
        let err = format!("{:?}", result.unwrap_err());
        assert!(
            err.to_lowercase().contains("blocked")
                || err.to_lowercase().contains("unverified")
                || err.to_lowercase().contains("paz"),
            "expected a BLOCKED/unverified/PAZ error message, got: {}",
            err
        );
    }

    #[test]
    fn deploy_native_refuses_sandboxed_game() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(
            crate::database::ModDatabase::new(&tmp.path().join("test.db"))
                .expect("test db"),
        );
        let detected = DetectedGame {
            game_id: "crimson_desert_native".into(),
            display_name: "Crimson Desert".into(),
            nexus_slug: "crimsondesert".into(),
            game_path: PathBuf::from("/fake"),
            exe_path: None,
            data_dir: PathBuf::from("/fake/Paz"),
            runtime: GameRuntime::Native(NativeContext {
                app_bundle_path: PathBuf::from(
                    "/Library/Containers/com.pearlabyss.CrimsonDesert/Crimson Desert.app",
                ),
                game_data_root: PathBuf::from("/fake"),
                architecture: Architecture::AppleSilicon,
                sandboxed: true, // App Store sandboxed
                source: NativeSource::AppStore,
            }),
            steam_app_id: None,
        };
        let result = CrimsonDesertNativePlugin.deploy_native(&detected, &db);
        assert!(result.is_err(), "deploy_native must refuse sandboxed game");
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("sandboxed"),
            "error must mention 'sandboxed': {msg}"
        );
    }

    #[test]
    fn deploy_native_refuses_intel_only_architecture() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(
            crate::database::ModDatabase::new(&tmp.path().join("test.db"))
                .expect("test db"),
        );
        let detected = DetectedGame {
            game_id: "crimson_desert_native".into(),
            display_name: "Crimson Desert".into(),
            nexus_slug: "crimsondesert".into(),
            game_path: PathBuf::from("/fake"),
            exe_path: None,
            data_dir: PathBuf::from("/fake/Paz"),
            runtime: GameRuntime::Native(NativeContext {
                app_bundle_path: PathBuf::from("/Applications/Crimson Desert.app"),
                game_data_root: PathBuf::from("/fake"),
                architecture: Architecture::IntelOnly, // should not exist
                sandboxed: false,
                source: NativeSource::Steam,
            }),
            steam_app_id: Some(CD_STEAM_APP_ID.into()),
        };
        let result = CrimsonDesertNativePlugin.deploy_native(&detected, &db);
        assert!(result.is_err(), "deploy_native must refuse Intel-only architecture");
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.to_lowercase().contains("intel") || msg.to_lowercase().contains("silicon"),
            "error must mention Intel/Silicon architecture: {msg}"
        );
    }
}
