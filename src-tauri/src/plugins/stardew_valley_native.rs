//! Stardew Valley (native macOS) game plugin.
//!
//! Detects ConcernedApe's macOS-native Stardew Valley install and
//! provides game-specific metadata for SMAPI mod management. Detection
//! walks `native_scanner::scan_all_native()` results and filters to
//! those matching the Stardew bundle identifier (or the executable name
//! as a fallback for GOG variants).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;
use thiserror::Error;

use crate::bottles::Bottle;
use crate::database::ModDatabase;
use crate::deployer::{DeployResult, DeployerError};
use crate::games::{DetectedGame, GamePlugin};

// ---------------------------------------------------------------------------
// SMAPI manifest types
// ---------------------------------------------------------------------------

/// Error type for manifest parsing operations.
#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("manifest file not found: {0}")]
    NotFound(String),
    #[error("malformed manifest: {0}")]
    Malformed(String),
}

/// A SMAPI mod dependency entry from `manifest.json`.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SdvDependency {
    /// The unique identifier of the required mod.
    #[serde(rename = "UniqueID")]
    pub unique_id: String,
    /// Whether this dependency is required for the mod to function.
    /// Defaults to `true` per SMAPI convention when omitted.
    #[serde(default = "default_required")]
    pub is_required: bool,
    /// Optional minimum version of the dependency required.
    pub minimum_version: Option<String>,
}

fn default_required() -> bool {
    true
}

/// Parsed representation of a SMAPI mod's `manifest.json`.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SdvModManifest {
    /// The human-readable mod name (from `Name`).
    pub name: String,
    /// The mod version string (from `Version`).
    pub version: String,
    /// The globally-unique mod identifier (from `UniqueID`).
    ///
    /// SMAPI requires this field; parsing fails if it is absent.
    #[serde(rename = "UniqueID")]
    pub unique_id: String,
    /// Optional list of mod dependencies (from `Dependencies`).
    #[serde(default)]
    pub dependencies: Vec<SdvDependency>,
    /// The minimum SMAPI API version required by this mod.
    pub minimum_api_version: Option<String>,
}

/// Parse a SMAPI mod's `manifest.json`.
///
/// Returns [`ManifestError::NotFound`] when the file does not exist and
/// [`ManifestError::Malformed`] for I/O errors, JSON parse failures, or
/// missing required fields (UniqueID is required per SMAPI convention).
pub fn parse_manifest(path: &Path) -> Result<SdvModManifest, ManifestError> {
    if !path.exists() {
        return Err(ManifestError::NotFound(path.display().to_string()));
    }
    let contents = std::fs::read_to_string(path)
        .map_err(|e| ManifestError::Malformed(format!("read failed: {}", e)))?;
    serde_json::from_str(&contents)
        .map_err(|e| ManifestError::Malformed(format!("json parse failed: {}", e)))
}

/// Return the canonical SMAPI mods directory for a detected Stardew Valley game.
///
/// SMAPI's installer places mods at `<gameDir>/Mods/`. For the native macOS
/// install, that resolves to `<app_bundle>/Contents/MacOS/Mods/`. Whether
/// SMAPI's runtime also reads from `~/Library/Application Support/StardewValley/Mods/`
/// is an open spike TODO; this function returns the bundle-internal path only.
pub fn resolve_mods_dir(detected: &DetectedGame) -> PathBuf {
    detected.game_path.join("Mods")
}

/// Game plugin for Stardew Valley (native macOS).
///
/// Stardew Valley ships a native universal binary (Apple Silicon +
/// Intel) on Steam and GOG. Mods are managed via SMAPI which patches
/// the `Contents/MacOS/StardewValley` launcher script in the .app
/// bundle to invoke a SMAPI-aware launcher that loads mods from
/// `Contents/MacOS/Mods/`.
pub struct StardewValleyNativePlugin;

const EXECUTABLES: &[&str] = &["StardewValley"];

const STARDEW_BUNDLE_IDENTIFIER: &str = "com.chucklefish.stardewvalley";
const STARDEW_BUNDLE_EXECUTABLE: &str = "StardewValley";
const STARDEW_STEAM_APP_ID: &str = "413150";

/// Filter `native_scanner` candidates to Stardew Valley installs and produce
/// `DetectedGame` entries. Pure function for testability — the public
/// `detect_native` impl wraps this with the actual scanner call.
fn detect_from_candidates(
    candidates: Vec<crate::native_scanner::NativeAppCandidate>,
) -> Vec<DetectedGame> {
    candidates
        .into_iter()
        .filter(|c| !c.sandboxed)
        .filter(|c| {
            c.info.bundle_identifier == STARDEW_BUNDLE_IDENTIFIER
                || c.info.bundle_executable == STARDEW_BUNDLE_EXECUTABLE
        })
        .map(|c| {
            let game_path = c.bundle_path.join("Contents").join("MacOS");
            let exe_path = Some(game_path.join("StardewValley"));
            let data_dir = game_path.join("Mods");
            let steam_app_id = if c.source == crate::runtime::NativeSource::Steam {
                Some(STARDEW_STEAM_APP_ID.to_string())
            } else {
                None
            };
            DetectedGame {
                game_id: "stardew_valley_native".to_string(),
                display_name: "Stardew Valley".to_string(),
                nexus_slug: "stardewvalley".to_string(),
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

impl GamePlugin for StardewValleyNativePlugin {
    fn game_id(&self) -> &str {
        "stardew_valley_native"
    }

    fn display_name(&self) -> &str {
        "Stardew Valley (Native)"
    }

    fn nexus_slug(&self) -> &str {
        "stardewvalley"
    }

    fn executables(&self) -> &[&str] {
        EXECUTABLES
    }

    fn detect_native(&self) -> Vec<DetectedGame> {
        detect_from_candidates(crate::native_scanner::scan_all_native())
    }

    fn get_data_dir(&self, game_path: &Path) -> PathBuf {
        // SMAPI loads mods from <game_path>/Mods (which is
        // <app_bundle>/Contents/MacOS/Mods on the native install).
        game_path.join("Mods")
    }

    fn get_plugins_file(&self, _game_path: &Path, _bottle: &Bottle) -> Option<PathBuf> {
        // Stardew has no plugins.txt-style load order; SMAPI loads all
        // mod folders alphabetically.
        None
    }

    /// Deploy all staged Stardew Valley mods into SMAPI's per-mod folder layout.
    ///
    /// SMAPI expects each mod in its own directory under `<game>/Mods/<UniqueID>/`.
    /// This method:
    /// 1. Lists all enabled mods registered in the DB for this game.
    /// 2. For each mod with a staging directory, reads `manifest.json` to
    ///    extract the mod's `UniqueID`.
    /// 3. Validates the `UniqueID` is safe (no path traversal, no embedded
    ///    separators) and resolves to a path inside `<mods_dir>`.
    /// 4. Creates `<mods_dir>/<UniqueID>/` and copies (or hardlinks if same
    ///    volume) staged files into it.
    fn deploy_native(
        &self,
        detected: &DetectedGame,
        db: &Arc<ModDatabase>,
    ) -> std::result::Result<DeployResult, DeployerError> {
        use crate::staging::is_safe_relative_path;
        use std::fs;

        let mods_dir = resolve_mods_dir(detected);
        fs::create_dir_all(&mods_dir)?;

        // Native games have no bottle; use "" as the bottle_name sentinel.
        let enabled_mods = db
            .list_mods(&detected.game_id, "")
            .map_err(|e| DeployerError::Database(e.to_string()))?;

        let mut deployed_count = 0usize;
        let mut fallback_used = false;

        for m in enabled_mods.iter().filter(|m| m.enabled) {
            let staging_path = match &m.staging_path {
                Some(p) => PathBuf::from(p),
                None => {
                    log::warn!(
                        "stardew deploy_native: mod {} has no staging path, skipping",
                        m.name
                    );
                    continue;
                }
            };

            if !staging_path.exists() {
                log::warn!(
                    "stardew deploy_native: staging dir missing for mod {}: {}",
                    m.name,
                    staging_path.display()
                );
                continue;
            }

            // Read manifest.json from the staging root.
            let manifest_path = staging_path.join("manifest.json");
            let manifest = parse_manifest(&manifest_path).map_err(|e| {
                DeployerError::Other(format!(
                    "mod '{}': manifest.json error: {}",
                    m.name, e
                ))
            })?;

            let unique_id = &manifest.unique_id;

            // Safety: reject UniqueIDs that would escape <mods_dir>.
            // SMAPI UniqueIDs are dotted identifiers like "AuthorName.ModName"
            // and must never contain path separators or traversal sequences.
            if !is_safe_relative_path(unique_id) || unique_id.contains('/') || unique_id.contains('\\') {
                return Err(DeployerError::Other(format!(
                    "mod '{}': UniqueID '{}' contains unsafe path characters",
                    m.name, unique_id
                )));
            }

            let target_dir = mods_dir.join(unique_id);

            // Post-join safety: ensure target_dir is inside mods_dir.
            // We can't use canonicalize on a path that may not exist yet,
            // so we normalise via components instead.
            let normalised_target = normalise_path(&target_dir);
            let normalised_mods = normalise_path(&mods_dir);
            if !normalised_target.starts_with(&normalised_mods) {
                return Err(DeployerError::Other(format!(
                    "mod '{}': resolved target '{}' escapes mods dir '{}'",
                    m.name,
                    target_dir.display(),
                    mods_dir.display()
                )));
            }

            fs::create_dir_all(&target_dir)?;

            // Copy every file from the staging root into the target dir.
            // Hardlink when on the same filesystem (zero extra disk space).
            let can_hardlink = crate::deployer::same_filesystem(&staging_path, &target_dir);

            for entry in walkdir::WalkDir::new(&staging_path)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
            {
                let rel = entry
                    .path()
                    .strip_prefix(&staging_path)
                    .expect("walkdir entry must be inside staging root");

                let dst = target_dir.join(rel);
                if let Some(parent) = dst.parent() {
                    fs::create_dir_all(parent)?;
                }

                if can_hardlink {
                    match fs::hard_link(entry.path(), &dst) {
                        Ok(()) => {}
                        Err(_) => {
                            // Hardlink failed (e.g. destination already exists from a
                            // previous deploy pass). Remove and retry, then copy.
                            let _ = fs::remove_file(&dst);
                            match fs::hard_link(entry.path(), &dst) {
                                Ok(()) => {}
                                Err(_) => {
                                    fs::copy(entry.path(), &dst)?;
                                    fallback_used = true;
                                }
                            }
                        }
                    }
                } else {
                    // Different filesystem — copy directly.
                    let _ = fs::remove_file(&dst); // overwrite if present
                    fs::copy(entry.path(), &dst)?;
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
}

/// Collapse `.` and `..` components from a `Path` without touching the
/// filesystem. Used for pre-existence path safety checks where
/// [`std::fs::canonicalize`] cannot be used (target may not exist yet).
fn normalise_path(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub fn register() {
    crate::games::register_plugin(Box::new(StardewValleyNativePlugin));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::with_plugin;
    use crate::runtime::{Architecture, NativeSource};

    /// Build a synthetic `NativeAppCandidate` for use in unit tests.
    ///
    /// The detection logic only reads `bundle_identifier` and
    /// `bundle_executable` from `info`, and `sandboxed` / `source` /
    /// `architecture` from the candidate itself — other InfoPlist fields
    /// (short_version, category) are left as `None`.
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

    #[test]
    fn stardew_valley_native_plugin_registers() {
        crate::games::register_plugin(Box::new(StardewValleyNativePlugin));
        let result = with_plugin("stardew_valley_native", |p| p.display_name().to_owned());
        assert_eq!(result, Some("Stardew Valley (Native)".to_owned()));
    }

    #[test]
    fn stardew_get_data_dir_returns_mods_subfolder() {
        let plugin = StardewValleyNativePlugin;
        let game_path = Path::new("/Applications/Stardew Valley.app/Contents/MacOS");
        assert_eq!(plugin.get_data_dir(game_path), game_path.join("Mods"));
    }

    /// Passing no candidates should yield no DetectedGame entries.
    #[test]
    fn stardew_detect_native_is_empty_for_no_candidates() {
        assert!(detect_from_candidates(vec![]).is_empty());
    }

    /// Only candidates with the chucklefish bundle identifier are kept;
    /// unrelated bundle identifiers are filtered out regardless of executable
    /// name.
    #[test]
    fn stardew_detect_native_filters_by_bundle_id() {
        let candidates = vec![
            fake_candidate(
                "/Applications/Stardew Valley.app",
                "com.chucklefish.stardewvalley",
                "StardewValley",
                false,
                NativeSource::SystemApplications,
                Architecture::Universal,
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
        assert_eq!(results.len(), 1, "only the chucklefish bundle should match");
        assert_eq!(results[0].game_id, "stardew_valley_native");
        assert_eq!(results[0].display_name, "Stardew Valley");
    }

    /// A candidate whose bundle_identifier does NOT match the chucklefish id
    /// but whose bundle_executable is "StardewValley" (GOG variant scenario)
    /// should still produce a DetectedGame.
    #[test]
    fn stardew_detect_native_falls_back_to_executable_name() {
        let candidates = vec![fake_candidate(
            "/Users/user/Games/Stardew Valley/Stardew Valley.app",
            "com.gog.stardewvalley", // different bundle id
            "StardewValley",         // matching executable name
            false,
            NativeSource::Gog,
            Architecture::Universal,
        )];

        let results = detect_from_candidates(candidates);
        assert_eq!(
            results.len(),
            1,
            "executable-name fallback should match GOG variant"
        );
        assert_eq!(results[0].game_id, "stardew_valley_native");
    }

    /// Sandboxed candidates are skipped even if the bundle identifier matches.
    #[test]
    fn stardew_detect_native_skips_sandboxed() {
        let candidates = vec![
            fake_candidate(
                "/Applications/Stardew Valley.app",
                "com.chucklefish.stardewvalley",
                "StardewValley",
                true, // sandboxed
                NativeSource::SystemApplications,
                Architecture::Universal,
            ),
            fake_candidate(
                "/Applications/Stardew Valley Normal.app",
                "com.chucklefish.stardewvalley",
                "StardewValley",
                false, // not sandboxed
                NativeSource::SystemApplications,
                Architecture::Universal,
            ),
        ];

        let results = detect_from_candidates(candidates);
        assert_eq!(results.len(), 1, "sandboxed candidate must be skipped");
        // The unsandboxed one should be present.
        let ctx = match &results[0].runtime {
            crate::runtime::GameRuntime::Native(n) => n,
            _ => panic!("expected Native runtime"),
        };
        assert!(!ctx.sandboxed);
    }

    /// Steam-sourced candidates get `steam_app_id = Some("413150")`.
    #[test]
    fn stardew_detect_native_populates_steam_app_id_for_steam_source() {
        let candidates = vec![fake_candidate(
            "/Users/user/Library/Application Support/Steam/steamapps/common/Stardew Valley/Stardew Valley.app",
            "com.chucklefish.stardewvalley",
            "StardewValley",
            false,
            NativeSource::Steam,
            Architecture::Universal,
        )];

        let results = detect_from_candidates(candidates);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].steam_app_id,
            Some("413150".to_string()),
            "Steam source must set steam_app_id to Stardew's appid"
        );
    }

    /// Non-Steam sources (GOG, Manual, SystemApplications) must NOT set
    /// steam_app_id so that Steam-specific logic downstream cannot be
    /// accidentally triggered.
    #[test]
    fn stardew_detect_native_omits_steam_app_id_for_other_sources() {
        let non_steam_sources = [
            NativeSource::Gog,
            NativeSource::Manual,
            NativeSource::SystemApplications,
        ];

        for source in non_steam_sources {
            let candidates = vec![fake_candidate(
                "/Applications/Stardew Valley.app",
                "com.chucklefish.stardewvalley",
                "StardewValley",
                false,
                source,
                Architecture::Universal,
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

    // -----------------------------------------------------------------------
    // SMAPI manifest parser tests
    // -----------------------------------------------------------------------

    #[test]
    fn parse_manifest_extracts_required_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.json");
        std::fs::write(
            &path,
            r#"{
                "Name": "Test Mod",
                "Author": "tester",
                "Version": "1.0.0",
                "Description": "Hello",
                "UniqueID": "tester.TestMod",
                "EntryDll": "TestMod.dll"
            }"#,
        )
        .unwrap();
        let m = parse_manifest(&path).unwrap();
        assert_eq!(m.name, "Test Mod");
        assert_eq!(m.version, "1.0.0");
        assert_eq!(m.unique_id, "tester.TestMod");
        assert!(m.dependencies.is_empty());
        assert!(m.minimum_api_version.is_none());
    }

    #[test]
    fn parse_manifest_handles_dependencies() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.json");
        std::fs::write(
            &path,
            r#"{
                "Name": "Dep Mod",
                "Version": "0.1.0",
                "UniqueID": "tester.DepMod",
                "Dependencies": [
                    { "UniqueID": "Pathoschild.ContentPatcher" },
                    { "UniqueID": "tester.OtherMod", "IsRequired": false, "MinimumVersion": "1.2.3" }
                ]
            }"#,
        )
        .unwrap();
        let m = parse_manifest(&path).unwrap();
        assert_eq!(m.dependencies.len(), 2);
        assert_eq!(m.dependencies[0].unique_id, "Pathoschild.ContentPatcher");
        assert!(m.dependencies[0].is_required, "default IsRequired = true");
        assert!(!m.dependencies[1].is_required);
        assert_eq!(
            m.dependencies[1].minimum_version.as_deref(),
            Some("1.2.3")
        );
    }

    #[test]
    fn parse_manifest_handles_minimum_api_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.json");
        std::fs::write(
            &path,
            r#"{
                "Name": "API Mod",
                "Version": "0.1.0",
                "UniqueID": "tester.ApiMod",
                "MinimumApiVersion": "4.0.0"
            }"#,
        )
        .unwrap();
        let m = parse_manifest(&path).unwrap();
        assert_eq!(m.minimum_api_version.as_deref(), Some("4.0.0"));
    }

    #[test]
    fn parse_manifest_returns_not_found_for_missing_file() {
        let result = parse_manifest(Path::new("/nonexistent/manifest.json"));
        assert!(matches!(result, Err(ManifestError::NotFound(_))));
    }

    #[test]
    fn parse_manifest_returns_malformed_for_bad_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.json");
        std::fs::write(&path, b"not json {").unwrap();
        let result = parse_manifest(&path);
        assert!(matches!(result, Err(ManifestError::Malformed(_))));
    }

    #[test]
    fn parse_manifest_returns_malformed_for_missing_unique_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.json");
        std::fs::write(
            &path,
            r#"{
                "Name": "No ID",
                "Version": "0.1.0"
            }"#,
        )
        .unwrap();
        let result = parse_manifest(&path);
        assert!(matches!(result, Err(ManifestError::Malformed(_))));
    }

    #[test]
    fn resolve_mods_dir_returns_game_path_mods_for_native() {
        let game_path =
            std::path::PathBuf::from("/Applications/Stardew Valley.app/Contents/MacOS");
        let detected = DetectedGame {
            game_id: "stardew_valley_native".into(),
            display_name: "Stardew Valley".into(),
            nexus_slug: "stardewvalley".into(),
            game_path: game_path.clone(),
            exe_path: None,
            data_dir: game_path.join("Mods"),
            runtime: crate::runtime::GameRuntime::Native(crate::runtime::NativeContext {
                app_bundle_path: std::path::PathBuf::from("/Applications/Stardew Valley.app"),
                game_data_root: game_path.clone(),
                architecture: crate::runtime::Architecture::AppleSilicon,
                sandboxed: false,
                source: crate::runtime::NativeSource::Steam,
            }),
            steam_app_id: Some("413150".into()),
        };
        assert_eq!(resolve_mods_dir(&detected), game_path.join("Mods"));
    }

    // -----------------------------------------------------------------------
    // deploy_native tests (Task 3.7)
    // -----------------------------------------------------------------------

    /// Build a synthetic `DetectedGame` with a native runtime rooted at `game_path`.
    fn fake_detected_game(game_path: &std::path::Path) -> DetectedGame {
        DetectedGame {
            game_id: "stardew_valley_native".into(),
            display_name: "Stardew Valley".into(),
            nexus_slug: "stardewvalley".into(),
            game_path: game_path.to_path_buf(),
            exe_path: Some(game_path.join("StardewValley")),
            data_dir: game_path.join("Mods"),
            runtime: crate::runtime::GameRuntime::Native(crate::runtime::NativeContext {
                app_bundle_path: game_path
                    .parent()
                    .and_then(|p| p.parent())
                    .unwrap_or(game_path)
                    .to_path_buf(),
                game_data_root: game_path.to_path_buf(),
                architecture: Architecture::Universal,
                sandboxed: false,
                source: NativeSource::Steam,
            }),
            steam_app_id: Some("413150".into()),
        }
    }

    /// Happy-path: staged mod with a valid manifest.json is deployed into
    /// `<mods_dir>/<UniqueID>/` and all staged files appear there.
    #[test]
    fn stardew_deploy_native_creates_mod_folder_with_unique_id() {
        use std::sync::Arc;
        let tmp = tempfile::tempdir().unwrap();

        // Set up a fake "game" directory and staging directory.
        let game_path = tmp.path().join("MacOS");
        let staging_dir = tmp.path().join("staging").join("my_mod");
        std::fs::create_dir_all(&game_path).unwrap();
        std::fs::create_dir_all(&staging_dir).unwrap();

        // Write a valid SMAPI manifest.json in the staging root.
        std::fs::write(
            staging_dir.join("manifest.json"),
            r#"{"Name":"My Mod","Version":"1.0.0","UniqueID":"tester.MyMod"}"#,
        )
        .unwrap();
        // Write an additional staged file.
        std::fs::write(staging_dir.join("MyMod.dll"), b"fake dll").unwrap();

        // Set up an in-memory DB and add the mod with the staging path.
        let db_path = tmp.path().join("test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());
        let mod_id = db
            .add_mod(
                "stardew_valley_native",
                "",
                None,
                "My Mod",
                "1.0.0",
                "MyMod.zip",
                &[],
            )
            .unwrap();
        db.set_staging_path(mod_id, &staging_dir.to_string_lossy())
            .unwrap();

        let detected = fake_detected_game(&game_path);
        let plugin = StardewValleyNativePlugin;
        let result = plugin.deploy_native(&detected, &db).unwrap();

        // Both files should have been deployed.
        assert_eq!(result.deployed_count, 2, "manifest.json + MyMod.dll");

        // Verify the per-mod folder was created with the correct UniqueID.
        let expected_dir = game_path.join("Mods").join("tester.MyMod");
        assert!(
            expected_dir.exists(),
            "per-mod folder <mods_dir>/tester.MyMod/ must exist"
        );
        assert!(
            expected_dir.join("manifest.json").exists(),
            "manifest.json must be present in target dir"
        );
        assert!(
            expected_dir.join("MyMod.dll").exists(),
            "MyMod.dll must be present in target dir"
        );
    }

    /// A UniqueID that contains a path traversal sequence (`..`) must be
    /// rejected with an error; no files should be written outside `<mods_dir>`.
    #[test]
    fn stardew_deploy_native_refuses_path_outside_mods_dir() {
        use std::sync::Arc;
        let tmp = tempfile::tempdir().unwrap();

        let game_path = tmp.path().join("MacOS");
        let staging_dir = tmp.path().join("staging").join("evil_mod");
        std::fs::create_dir_all(&game_path).unwrap();
        std::fs::create_dir_all(&staging_dir).unwrap();

        // A manifest whose UniqueID contains path-traversal characters.
        std::fs::write(
            staging_dir.join("manifest.json"),
            r#"{"Name":"Evil","Version":"1.0.0","UniqueID":"../../../etc/evil"}"#,
        )
        .unwrap();

        let db_path = tmp.path().join("test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());
        let mod_id = db
            .add_mod(
                "stardew_valley_native",
                "",
                None,
                "Evil Mod",
                "1.0.0",
                "evil.zip",
                &[],
            )
            .unwrap();
        db.set_staging_path(mod_id, &staging_dir.to_string_lossy())
            .unwrap();

        let detected = fake_detected_game(&game_path);
        let plugin = StardewValleyNativePlugin;
        let result = plugin.deploy_native(&detected, &db);

        assert!(
            result.is_err(),
            "path-traversal UniqueID must be rejected"
        );
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("unsafe path characters") || msg.contains("escapes mods dir"),
            "error should mention unsafe path or escape: {msg}"
        );
    }

    /// A staged mod directory that has no `manifest.json` must cause an error
    /// rather than being silently skipped, so missing manifests are caught early.
    #[test]
    fn stardew_deploy_native_returns_err_for_missing_manifest() {
        use std::sync::Arc;
        let tmp = tempfile::tempdir().unwrap();

        let game_path = tmp.path().join("MacOS");
        let staging_dir = tmp.path().join("staging").join("no_manifest");
        std::fs::create_dir_all(&game_path).unwrap();
        std::fs::create_dir_all(&staging_dir).unwrap();

        // Write a file but NOT a manifest.json.
        std::fs::write(staging_dir.join("SomeMod.dll"), b"bytes").unwrap();

        let db_path = tmp.path().join("test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());
        let mod_id = db
            .add_mod(
                "stardew_valley_native",
                "",
                None,
                "No Manifest Mod",
                "1.0.0",
                "nomanifest.zip",
                &[],
            )
            .unwrap();
        db.set_staging_path(mod_id, &staging_dir.to_string_lossy())
            .unwrap();

        let detected = fake_detected_game(&game_path);
        let plugin = StardewValleyNativePlugin;
        let result = plugin.deploy_native(&detected, &db);

        assert!(
            result.is_err(),
            "missing manifest.json must return an error"
        );
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("manifest.json"),
            "error should mention manifest.json: {msg}"
        );
    }

    // -----------------------------------------------------------------------
    // Path shape tests (existing)
    // -----------------------------------------------------------------------

    /// Verify path shapes: game_path, exe_path, data_dir all derive from
    /// <bundle>/Contents/MacOS as the spec requires.
    #[test]
    fn stardew_detect_native_path_shapes_are_correct() {
        let bundle = "/Applications/Stardew Valley.app";
        let candidates = vec![fake_candidate(
            bundle,
            "com.chucklefish.stardewvalley",
            "StardewValley",
            false,
            NativeSource::Steam,
            Architecture::AppleSilicon,
        )];

        let results = detect_from_candidates(candidates);
        assert_eq!(results.len(), 1);
        let game = &results[0];

        let expected_game_path =
            std::path::PathBuf::from(bundle).join("Contents").join("MacOS");
        assert_eq!(game.game_path, expected_game_path);
        assert_eq!(
            game.exe_path,
            Some(expected_game_path.join("StardewValley"))
        );
        assert_eq!(game.data_dir, expected_game_path.join("Mods"));

        // NativeContext carries the bundle path and architecture from the candidate.
        let ctx = match &game.runtime {
            crate::runtime::GameRuntime::Native(n) => n,
            _ => panic!("expected Native runtime"),
        };
        assert_eq!(ctx.app_bundle_path, std::path::PathBuf::from(bundle));
        assert_eq!(ctx.game_data_root, expected_game_path);
        assert_eq!(ctx.architecture, Architecture::AppleSilicon);
        assert_eq!(ctx.source, NativeSource::Steam);
        assert!(!ctx.sandboxed);
    }
}
