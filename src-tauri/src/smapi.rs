//! SMAPI (Stardew Modding API) detection — read-only.
//!
//! SMAPI is the canonical mod loader for Stardew Valley. Its installer
//! mutates the game's .app bundle by renaming the vanilla
//! `Contents/MacOS/StardewValley` launcher to `StardewValley-original`
//! and dropping a new launcher that loads SMAPI's runtime. We detect
//! presence by checking for those markers.
//!
//! Mutation logic (install / uninstall) lives in Tasks 3.4 and 3.5.
//! This module is read-only.

use std::fs;
use std::io;
use std::path::Path;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SmapiError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),

    #[error("{0}")]
    Other(String),
}

/// Marker file: SMAPI renames the vanilla launcher to this when installed.
const VANILLA_LAUNCHER_RENAMED: &str = "StardewValley-original";

/// Marker file: SMAPI's main executable, present only when SMAPI is installed.
const SMAPI_EXECUTABLE: &str = "StardewModdingAPI";

/// Detect whether SMAPI is installed in a Stardew Valley `.app` bundle.
///
/// Returns true iff BOTH the renamed vanilla launcher (`StardewValley-original`)
/// AND the SMAPI executable (`StardewModdingAPI`) exist in the bundle's
/// `Contents/MacOS` directory. Either marker alone indicates a partial /
/// broken install — return false so the caller can prompt re-install.
pub fn is_installed(app_bundle: &Path) -> bool {
    let macos = app_bundle.join("Contents").join("MacOS");
    macos.join(VANILLA_LAUNCHER_RENAMED).exists() && macos.join(SMAPI_EXECUTABLE).exists()
}

/// Read the installed SMAPI version, if available.
///
/// Looks at the deps.json file SMAPI's installer drops alongside the
/// SMAPI executable. Returns None if the file is missing, malformed,
/// or doesn't contain a recognizable version string.
pub fn installed_version(app_bundle: &Path) -> Option<String> {
    let macos = app_bundle.join("Contents").join("MacOS");
    let candidates = [
        macos.join("StardewModdingAPI.deps.json"),
        macos.join("smapi-internal/Stardew Valley.deps.json"),
    ];
    for path in &candidates {
        if let Ok(contents) = fs::read_to_string(path) {
            if let Some(version) = extract_version_from_deps_json(&contents) {
                return Some(version);
            }
        }
    }
    None
}

/// Extract a SMAPI version from a deps.json blob. The format is .NET's
/// dependency JSON; SMAPI's assembly version typically appears under
/// targets.<framework>.StardewModdingAPI/<version>. We do a forgiving
/// scan: parse JSON, walk to find a key that looks like
/// "StardewModdingAPI/<version>" and pull the version part.
fn extract_version_from_deps_json(contents: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(contents).ok()?;
    // Check libraries section first (more reliable — it's a flat map of
    // "AssemblyName/version" → metadata, present in all .NET deps.json files).
    if let Some(libraries) = value.get("libraries").and_then(|v| v.as_object()) {
        for key in libraries.keys() {
            if let Some(rest) = key.strip_prefix("StardewModdingAPI/") {
                return Some(rest.to_string());
            }
        }
    }
    // Fall back to scanning targets.* for any "StardewModdingAPI/X.Y.Z" key.
    if let Some(targets) = value.get("targets").and_then(|v| v.as_object()) {
        for target_obj in targets.values() {
            if let Some(target) = target_obj.as_object() {
                for key in target.keys() {
                    if let Some(rest) = key.strip_prefix("StardewModdingAPI/") {
                        return Some(rest.to_string());
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_macos_dir(dir: &Path) -> std::path::PathBuf {
        let macos = dir.join("Stardew Valley.app/Contents/MacOS");
        fs::create_dir_all(&macos).expect("mkdir");
        macos
    }

    #[test]
    fn is_installed_returns_false_for_vanilla_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let macos = make_macos_dir(dir.path());
        // Vanilla: only StardewValley present, no SMAPI markers.
        fs::write(macos.join("StardewValley"), b"#!/bin/bash\n").unwrap();
        let bundle = dir.path().join("Stardew Valley.app");
        assert!(!is_installed(&bundle));
    }

    #[test]
    fn is_installed_returns_true_when_both_markers_exist() {
        let dir = tempfile::tempdir().unwrap();
        let macos = make_macos_dir(dir.path());
        fs::write(macos.join("StardewValley-original"), b"#!/bin/bash\n").unwrap();
        fs::write(macos.join("StardewModdingAPI"), b"smapi binary").unwrap();
        let bundle = dir.path().join("Stardew Valley.app");
        assert!(is_installed(&bundle));
    }

    #[test]
    fn is_installed_returns_false_when_only_renamed_launcher_exists() {
        let dir = tempfile::tempdir().unwrap();
        let macos = make_macos_dir(dir.path());
        fs::write(macos.join("StardewValley-original"), b"#!/bin/bash\n").unwrap();
        // No StardewModdingAPI — partial install.
        let bundle = dir.path().join("Stardew Valley.app");
        assert!(!is_installed(&bundle));
    }

    #[test]
    fn is_installed_returns_false_when_only_smapi_executable_exists() {
        let dir = tempfile::tempdir().unwrap();
        let macos = make_macos_dir(dir.path());
        fs::write(macos.join("StardewModdingAPI"), b"smapi binary").unwrap();
        // No renamed launcher — partial install.
        let bundle = dir.path().join("Stardew Valley.app");
        assert!(!is_installed(&bundle));
    }

    #[test]
    fn installed_version_returns_none_for_uninstalled() {
        let dir = tempfile::tempdir().unwrap();
        make_macos_dir(dir.path());
        let bundle = dir.path().join("Stardew Valley.app");
        assert_eq!(installed_version(&bundle), None);
    }

    #[test]
    fn installed_version_extracts_version_from_deps_json() {
        let dir = tempfile::tempdir().unwrap();
        let macos = make_macos_dir(dir.path());
        let deps = serde_json::json!({
            "libraries": {
                "StardewModdingAPI/4.1.10": {
                    "type": "project",
                    "serviceable": false,
                    "sha512": ""
                }
            }
        });
        fs::write(macos.join("StardewModdingAPI.deps.json"), deps.to_string()).unwrap();
        let bundle = dir.path().join("Stardew Valley.app");
        assert_eq!(installed_version(&bundle), Some("4.1.10".to_string()));
    }

    #[test]
    fn installed_version_returns_none_for_malformed_deps_json() {
        let dir = tempfile::tempdir().unwrap();
        let macos = make_macos_dir(dir.path());
        fs::write(macos.join("StardewModdingAPI.deps.json"), b"not valid json {").unwrap();
        let bundle = dir.path().join("Stardew Valley.app");
        assert_eq!(installed_version(&bundle), None);
    }
}
