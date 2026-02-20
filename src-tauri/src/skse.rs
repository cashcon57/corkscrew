//! SKSE (Skyrim Script Extender) detection and installation.
//!
//! Provides utilities for:
//! - Detecting whether SKSE is installed in a Skyrim SE game directory
//! - Installing SKSE files from a user-provided local archive
//! - Persisting per-game SKSE preference in the Corkscrew config
//!
//! **NOTE:** SKSE's license prohibits automated redistribution. We do NOT
//! auto-download SKSE. Instead, the user downloads the archive manually from
//! the official site and we install from their local copy.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config;
use crate::installer;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum SkseError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("Archive extraction failed: {0}")]
    Extraction(String),

    #[error("SKSE root directory not found in extracted archive")]
    SkseRootNotFound,

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, SkseError>;

// ---------------------------------------------------------------------------
// SkseStatus
// ---------------------------------------------------------------------------

/// Status of SKSE in a game installation, suitable for the Tauri frontend.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkseStatus {
    /// Whether SKSE appears to be installed (loader exe found).
    pub installed: bool,
    /// Path to the SKSE loader executable, if found.
    pub loader_path: Option<String>,
    /// Detected SKSE version (e.g. "2.2.6"), if a versioned DLL was found.
    pub version: Option<String>,
    /// Whether the user has opted to use SKSE for launching.
    pub use_skse: bool,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// The SKSE loader executable name (Skyrim SE 64-bit).
const SKSE_LOADER: &str = "skse64_loader.exe";

/// Official SKSE download page.
const SKSE_URL: &str = "https://skse.silverlock.org/";

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Detect whether SKSE is installed in the given game directory.
///
/// Scans for `skse64_loader.exe` (case-insensitive) and attempts to extract
/// the version number from any `skse64_X_Y_Z.dll` file present.
pub fn detect_skse(game_path: &Path) -> SkseStatus {
    let loader = find_file_case_insensitive(game_path, SKSE_LOADER);
    let version = detect_skse_version(game_path);

    let installed = loader.is_some();
    let loader_path = loader.map(|p| p.to_string_lossy().into_owned());

    SkseStatus {
        installed,
        loader_path,
        version,
        use_skse: false, // Caller should overlay the preference from config.
    }
}

/// Detect the SKSE version by finding a `skse64_X_Y_Z.dll` file.
///
/// Specifically looks for DLLs matching the pattern `skse64_\d+_\d+_\d+.dll`
/// but **excludes** the Steam loader variant (`skse64_steam_loader.dll`).
#[allow(dead_code)]
fn detect_skse_version(game_path: &Path) -> Option<String> {
    let entries = fs::read_dir(game_path).ok()?;

    for entry in entries.flatten() {
        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_lowercase();

        // Skip the steam loader — we want the main SKSE DLL.
        if name.contains("steam_loader") {
            continue;
        }

        // Match pattern: skse64_X_Y_Z.dll
        if name.starts_with("skse64_") && name.ends_with(".dll") {
            let stem = name
                .trim_start_matches("skse64_")
                .trim_end_matches(".dll");

            // Parse "X_Y_Z" into "X.Y.Z"
            let parts: Vec<&str> = stem.split('_').collect();
            if parts.len() == 3 && parts.iter().all(|p| p.parse::<u32>().is_ok()) {
                let version = parts.join(".");
                debug!("Detected SKSE version: {} from file: {}", version, name);
                return Some(version);
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Install from local archive
// ---------------------------------------------------------------------------

/// Return the official SKSE download page URL so the frontend can open it.
pub fn skse_download_url() -> &'static str {
    SKSE_URL
}

/// Install SKSE from a user-provided local archive (.7z or .zip).
///
/// The user downloads the archive manually from <https://skse.silverlock.org/>
/// and then points Corkscrew at the file. This respects SKSE's redistribution
/// license.
///
/// 1. Extracts the archive to a temporary directory.
/// 2. Copies SKSE files into the game directory.
/// 3. Cleans up the temp directory.
///
/// Returns an updated [`SkseStatus`] reflecting the new installation.
pub fn install_skse_from_archive(
    game_path: &Path,
    archive_path: &Path,
) -> Result<SkseStatus> {
    if !archive_path.exists() {
        return Err(SkseError::Io(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Archive not found: {}", archive_path.display()),
        )));
    }

    info!(
        "Installing SKSE from local archive: {}",
        archive_path.display()
    );

    // 1. Extract the archive to a temp directory.
    let extract_dir = std::env::temp_dir().join("corkscrew_skse_extract");
    if extract_dir.exists() {
        fs::remove_dir_all(&extract_dir)?;
    }
    fs::create_dir_all(&extract_dir)?;

    installer::extract_archive(archive_path, &extract_dir).map_err(|e| {
        SkseError::Extraction(format!("Failed to extract SKSE archive: {}", e))
    })?;

    // 2. Install SKSE files into the game directory.
    install_skse_files(&extract_dir, game_path)?;

    // 3. Clean up extraction directory.
    if let Err(e) = fs::remove_dir_all(&extract_dir) {
        warn!("Failed to clean up SKSE extraction dir: {}", e);
    }

    // 4. Return updated status.
    let status = detect_skse(game_path);
    info!(
        "SKSE installation complete: installed={}, version={:?}",
        status.installed, status.version
    );

    Ok(status)
}

/// Install SKSE files from an extracted archive directory into the game.
///
/// Expects the extracted archive to contain a single top-level directory
/// (e.g. `skse64_2_02_06/`) which in turn contains the SKSE files.
///
/// Copies:
/// - `.exe` and `.dll` files from the SKSE root -> game root
/// - `Data/` subfolder contents -> game's `Data/` directory
pub fn install_skse_files(extracted_dir: &Path, game_path: &Path) -> Result<()> {
    // Find the SKSE root directory (the single top-level folder in the archive).
    let skse_root = find_skse_root(extracted_dir)?;

    info!(
        "Installing SKSE from {} to {}",
        skse_root.display(),
        game_path.display()
    );

    // Copy .exe and .dll files from SKSE root to game root.
    let entries = fs::read_dir(&skse_root).map_err(SkseError::Io)?;
    let mut files_copied = 0u32;

    for entry in entries.flatten() {
        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name.ends_with(".exe") || name.ends_with(".dll") {
            let dest = game_path.join(entry.file_name());
            fs::copy(entry.path(), &dest)?;
            debug!("Copied {} -> {}", entry.path().display(), dest.display());
            files_copied += 1;
        }
    }

    info!("Copied {} exe/dll files to game root", files_copied);

    // Copy Data/ subfolder if present.
    let skse_data = find_subdirectory_case_insensitive(&skse_root, "data");
    if let Some(data_src) = skse_data {
        let data_dst = game_path.join("Data");
        fs::create_dir_all(&data_dst)?;
        copy_dir_recursive(&data_src, &data_dst)?;
        info!(
            "Copied SKSE Data directory: {} -> {}",
            data_src.display(),
            data_dst.display()
        );
    } else {
        debug!("No Data/ subfolder found in SKSE archive");
    }

    Ok(())
}

/// Locate the SKSE root directory inside an extracted archive.
///
/// Most SKSE archives contain a single top-level directory like
/// `skse64_2_02_06/`. This function finds it.
fn find_skse_root(extracted_dir: &Path) -> Result<PathBuf> {
    let entries: Vec<_> = fs::read_dir(extracted_dir)
        .map_err(SkseError::Io)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
        .filter(|e| {
            !e.file_name()
                .to_string_lossy()
                .starts_with('.')
        })
        .collect();

    // If there's exactly one top-level directory, that's our SKSE root.
    if entries.len() == 1 {
        return Ok(entries[0].path());
    }

    // If there are multiple directories, look for one starting with "skse".
    for entry in &entries {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name.starts_with("skse") {
            return Ok(entry.path());
        }
    }

    // Check if the extracted_dir itself contains SKSE files directly.
    if find_file_case_insensitive(extracted_dir, SKSE_LOADER).is_some() {
        return Ok(extracted_dir.to_path_buf());
    }

    Err(SkseError::SkseRootNotFound)
}

// ---------------------------------------------------------------------------
// Config preferences
// ---------------------------------------------------------------------------

/// Read the user's SKSE preference for a specific game+bottle combination.
///
/// Stores the preference under the config key `skse_enabled_{game_id}_{bottle_name}`.
pub fn get_skse_preference(game_id: &str, bottle_name: &str) -> bool {
    let key = format!("skse_enabled_{}_{}", game_id, bottle_name);
    config::get_config_value(&key)
        .ok()
        .flatten()
        .map(|v| v == "true")
        .unwrap_or(false)
}

/// Store the user's SKSE preference for a specific game+bottle combination.
pub fn set_skse_preference(game_id: &str, bottle_name: &str, enabled: bool) -> Result<()> {
    let key = format!("skse_enabled_{}_{}", game_id, bottle_name);
    let value = if enabled { "true" } else { "false" };
    config::set_config_value(&key, value).map_err(|e| SkseError::Config(e.to_string()))
}

// ---------------------------------------------------------------------------
// SKSE ↔ Game Version Compatibility
// ---------------------------------------------------------------------------

/// Result of checking SKSE + game version compatibility.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkseCompatibility {
    /// Overall compatibility verdict.
    pub compatible: bool,
    /// The detected SKSE version, if any.
    pub skse_version: Option<String>,
    /// The detected game version string.
    pub game_version: String,
    /// The expected game version range for this SKSE version (min, max).
    pub expected_game_versions: Option<(String, String)>,
    /// Human-readable summary.
    pub message: String,
    /// Severity: "ok", "warning", or "error".
    pub severity: String,
}

/// Map a detected SKSE version to the Skyrim game version range it supports.
///
/// Returns `Some((min_game_version, max_game_version))` for known SKSE builds.
pub fn skse_game_compatibility(skse_version: &str) -> Option<(&'static str, &'static str)> {
    let parts: Vec<u32> = skse_version
        .split('.')
        .filter_map(|p| p.parse().ok())
        .collect();

    if parts.len() < 3 {
        return None;
    }

    let (major, minor, patch) = (parts[0], parts[1], parts[2]);

    match (major, minor) {
        // SKSE 2.0.x and 2.1.x target Skyrim SE 1.5.97
        (2, 0) | (2, 1) => Some(("1.5.97", "1.5.97")),
        // SKSE 2.2.x targets AE versions
        (2, 2) => {
            if patch <= 2 {
                // 2.2.0 – 2.2.2 → AE 1.6.317 – 1.6.659
                Some(("1.6.317", "1.6.659"))
            } else {
                // 2.2.3+ → AE 1.6.1130+
                Some(("1.6.1130", "1.6.1170"))
            }
        }
        _ => None,
    }
}

/// Classify whether "Skyrim SE 1.5.97" or "Anniversary Edition" from a
/// [`DowngradeStatus`] version string.
fn classify_game_version(version_str: &str) -> &str {
    if version_str.contains("1.5.97") {
        "SE"
    } else if version_str.contains("Anniversary")
        || version_str.contains("1.6")
    {
        "AE"
    } else {
        "Unknown"
    }
}

/// Run a combined SKSE + game version compatibility check.
///
/// Takes the SKSE detection result and the downgrade/version detection result
/// and produces a single compatibility verdict.
pub fn check_skse_compatibility(
    skse_status: &SkseStatus,
    downgrade_status: &crate::downgrader::DowngradeStatus,
) -> SkseCompatibility {
    let game_version = downgrade_status.current_version.clone();

    // Case 1: SKSE is not installed at all
    if !skse_status.installed {
        return SkseCompatibility {
            compatible: false,
            skse_version: None,
            game_version,
            expected_game_versions: None,
            message: "SKSE is not installed. Download it from the official site and install from the archive.".into(),
            severity: "error".into(),
        };
    }

    // Case 2: SKSE is installed but we couldn't detect the version
    let skse_ver = match &skse_status.version {
        Some(v) => v.clone(),
        None => {
            return SkseCompatibility {
                compatible: true,
                skse_version: None,
                game_version,
                expected_game_versions: None,
                message: "SKSE is installed but the version could not be determined. Verify manually that it matches your game version.".into(),
                severity: "warning".into(),
            };
        }
    };

    // Case 3: SKSE version detected — check against game version
    let expected = skse_game_compatibility(&skse_ver);
    let game_class = classify_game_version(&game_version).to_string();

    match expected {
        Some((min_ver, max_ver)) => {
            let expected_class = if min_ver == "1.5.97" { "SE" } else { "AE" };

            if game_class == expected_class || game_class == "Unknown" {
                let message = format!(
                    "SKSE {} is compatible with Skyrim {} ({} – {}).",
                    skse_ver, game_class, min_ver, max_ver
                );
                SkseCompatibility {
                    compatible: true,
                    skse_version: Some(skse_ver),
                    game_version,
                    expected_game_versions: Some((min_ver.into(), max_ver.into())),
                    message,
                    severity: "ok".into(),
                }
            } else {
                let message = format!(
                    "SKSE {} targets Skyrim {} ({} – {}), but you have Skyrim {}. Install the correct SKSE build for your game version.",
                    skse_ver, expected_class, min_ver, max_ver, game_class
                );
                SkseCompatibility {
                    compatible: false,
                    skse_version: Some(skse_ver),
                    game_version,
                    expected_game_versions: Some((min_ver.into(), max_ver.into())),
                    message,
                    severity: "error".into(),
                }
            }
        }
        None => {
            let message = format!(
                "SKSE {} is installed. Could not determine expected game version range — verify compatibility manually.",
                skse_ver
            );
            SkseCompatibility {
                compatible: true,
                skse_version: Some(skse_ver),
                game_version,
                expected_game_versions: None,
                message,
                severity: "warning".into(),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Find a file by name (case-insensitive) in a directory (non-recursive).
#[allow(dead_code)]
pub fn find_file_case_insensitive(dir: &Path, target: &str) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }

    let target_lower = target.to_lowercase();

    // Fast path: try exact match.
    let exact = dir.join(target);
    if exact.exists() {
        return Some(exact);
    }

    // Case-insensitive scan.
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name == target_lower {
                return Some(entry.path());
            }
        }
    }

    None
}

/// Find a subdirectory by name (case-insensitive) in a directory.
#[allow(dead_code)]
fn find_subdirectory_case_insensitive(dir: &Path, target: &str) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }

    let target_lower = target.to_lowercase();

    // Fast path: try exact match.
    let exact = dir.join(target);
    if exact.is_dir() {
        return Some(exact);
    }

    // Case-insensitive scan.
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name == target_lower {
                return Some(entry.path());
            }
        }
    }

    None
}

/// Recursively copy the contents of `src` into `dst`.
///
/// Creates `dst` and any necessary parent directories. Existing files
/// in `dst` will be overwritten.
#[allow(dead_code)]
pub fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    if !src.is_dir() {
        return Err(SkseError::Io(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Source directory does not exist: {}", src.display()),
        )));
    }

    fs::create_dir_all(dst)?;

    for entry in fs::read_dir(src)?.flatten() {
        let entry_path = entry.path();
        let dest_path = dst.join(entry.file_name());

        if entry_path.is_dir() {
            copy_dir_recursive(&entry_path, &dest_path)?;
        } else {
            fs::copy(&entry_path, &dest_path)?;
            debug!(
                "Copied: {} -> {}",
                entry_path.display(),
                dest_path.display()
            );
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn detect_skse_not_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let status = detect_skse(tmp.path());
        assert!(!status.installed);
        assert!(status.loader_path.is_none());
        assert!(status.version.is_none());
    }

    #[test]
    fn detect_skse_installed_with_version() {
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path();

        // Create fake SKSE files.
        fs::write(game_dir.join("skse64_loader.exe"), b"fake loader").unwrap();
        fs::write(game_dir.join("skse64_2_2_6.dll"), b"fake dll").unwrap();
        fs::write(
            game_dir.join("skse64_steam_loader.dll"),
            b"fake steam loader",
        )
        .unwrap();

        let status = detect_skse(game_dir);
        assert!(status.installed);
        assert!(status.loader_path.is_some());
        assert_eq!(status.version, Some("2.2.6".to_string()));
    }

    #[test]
    fn detect_skse_installed_without_version_dll() {
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path();

        // Only the loader, no versioned DLL.
        fs::write(game_dir.join("skse64_loader.exe"), b"fake loader").unwrap();

        let status = detect_skse(game_dir);
        assert!(status.installed);
        assert!(status.version.is_none());
    }

    #[test]
    fn detect_skse_case_insensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path();

        // Use different casing.
        fs::write(game_dir.join("SKSE64_Loader.exe"), b"fake").unwrap();
        fs::write(game_dir.join("SKSE64_2_2_6.dll"), b"fake").unwrap();

        let status = detect_skse(game_dir);
        assert!(status.installed);
        assert_eq!(status.version, Some("2.2.6".to_string()));
    }

    #[test]
    fn skse_compat_se_versions() {
        // SKSE 2.0.x and 2.1.x should target SE 1.5.97
        assert_eq!(
            skse_game_compatibility("2.0.20"),
            Some(("1.5.97", "1.5.97"))
        );
        assert_eq!(
            skse_game_compatibility("2.1.5"),
            Some(("1.5.97", "1.5.97"))
        );
    }

    #[test]
    fn skse_compat_ae_early_versions() {
        // SKSE 2.2.0–2.2.2 should target AE 1.6.317–1.6.659
        assert_eq!(
            skse_game_compatibility("2.2.0"),
            Some(("1.6.317", "1.6.659"))
        );
        assert_eq!(
            skse_game_compatibility("2.2.2"),
            Some(("1.6.317", "1.6.659"))
        );
    }

    #[test]
    fn skse_compat_ae_late_versions() {
        // SKSE 2.2.3+ should target AE 1.6.1130+
        assert_eq!(
            skse_game_compatibility("2.2.3"),
            Some(("1.6.1130", "1.6.1170"))
        );
        assert_eq!(
            skse_game_compatibility("2.2.6"),
            Some(("1.6.1130", "1.6.1170"))
        );
    }

    #[test]
    fn skse_compat_unknown_version() {
        assert_eq!(skse_game_compatibility("3.0.0"), None);
        assert_eq!(skse_game_compatibility("invalid"), None);
    }

    #[test]
    fn check_compat_no_skse() {
        let skse_status = SkseStatus {
            installed: false,
            loader_path: None,
            version: None,
            use_skse: false,
        };
        let downgrade_status = crate::downgrader::DowngradeStatus {
            current_version: "1.5.97 (Special Edition)".into(),
            target_version: "1.5.97".into(),
            is_downgraded: true,
            downgrade_path: None,
        };

        let result = check_skse_compatibility(&skse_status, &downgrade_status);
        assert!(!result.compatible);
        assert_eq!(result.severity, "error");
    }

    #[test]
    fn check_compat_matching_se() {
        let skse_status = SkseStatus {
            installed: true,
            loader_path: Some("/game/skse64_loader.exe".into()),
            version: Some("2.0.20".into()),
            use_skse: true,
        };
        let downgrade_status = crate::downgrader::DowngradeStatus {
            current_version: "1.5.97 (Special Edition)".into(),
            target_version: "1.5.97".into(),
            is_downgraded: true,
            downgrade_path: None,
        };

        let result = check_skse_compatibility(&skse_status, &downgrade_status);
        assert!(result.compatible);
        assert_eq!(result.severity, "ok");
    }

    #[test]
    fn check_compat_mismatch_ae_skse_on_se_game() {
        let skse_status = SkseStatus {
            installed: true,
            loader_path: Some("/game/skse64_loader.exe".into()),
            version: Some("2.2.6".into()),
            use_skse: true,
        };
        let downgrade_status = crate::downgrader::DowngradeStatus {
            current_version: "1.5.97 (Special Edition)".into(),
            target_version: "1.5.97".into(),
            is_downgraded: true,
            downgrade_path: None,
        };

        let result = check_skse_compatibility(&skse_status, &downgrade_status);
        assert!(!result.compatible);
        assert_eq!(result.severity, "error");
    }

    #[test]
    fn skse_download_url_is_valid() {
        let url = skse_download_url();
        assert!(url.starts_with("https://"));
        assert!(url.contains("skse"));
    }

    #[test]
    fn find_file_case_insensitive_works() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("SomeFile.TXT"), b"hello").unwrap();

        let result = find_file_case_insensitive(tmp.path(), "somefile.txt");
        assert!(result.is_some());
    }

    #[test]
    fn find_file_case_insensitive_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let result = find_file_case_insensitive(tmp.path(), "nonexistent.txt");
        assert!(result.is_none());
    }

    #[test]
    fn copy_dir_recursive_works() {
        let tmp = tempfile::tempdir().unwrap();

        // Create source tree.
        let src = tmp.path().join("src");
        let src_sub = src.join("subdir");
        fs::create_dir_all(&src_sub).unwrap();
        fs::write(src.join("file1.txt"), b"hello").unwrap();
        fs::write(src_sub.join("file2.txt"), b"world").unwrap();

        // Copy to destination.
        let dst = tmp.path().join("dst");
        copy_dir_recursive(&src, &dst).unwrap();

        // Verify.
        assert!(dst.join("file1.txt").exists());
        assert!(dst.join("subdir").join("file2.txt").exists());
        assert_eq!(
            fs::read_to_string(dst.join("file1.txt")).unwrap(),
            "hello"
        );
        assert_eq!(
            fs::read_to_string(dst.join("subdir").join("file2.txt")).unwrap(),
            "world"
        );
    }

    #[test]
    fn find_skse_root_single_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let skse_dir = tmp.path().join("skse64_2_02_06");
        fs::create_dir_all(&skse_dir).unwrap();
        fs::write(skse_dir.join("skse64_loader.exe"), b"fake").unwrap();

        let root = find_skse_root(tmp.path()).unwrap();
        assert_eq!(root, skse_dir);
    }

    #[test]
    fn find_skse_root_multiple_dirs_picks_skse() {
        let tmp = tempfile::tempdir().unwrap();
        let skse_dir = tmp.path().join("skse64_2_02_06");
        let other_dir = tmp.path().join("readme_files");
        fs::create_dir_all(&skse_dir).unwrap();
        fs::create_dir_all(&other_dir).unwrap();

        let root = find_skse_root(tmp.path()).unwrap();
        assert_eq!(root, skse_dir);
    }

    #[test]
    fn install_skse_files_copies_correctly() {
        let tmp = tempfile::tempdir().unwrap();

        // Create a fake extracted SKSE archive.
        let extracted = tmp.path().join("extracted");
        let skse_root = extracted.join("skse64_2_02_06");
        let skse_data = skse_root.join("Data").join("Scripts");
        fs::create_dir_all(&skse_data).unwrap();
        fs::write(skse_root.join("skse64_loader.exe"), b"loader").unwrap();
        fs::write(skse_root.join("skse64_2_2_6.dll"), b"main dll").unwrap();
        fs::write(
            skse_root.join("skse64_steam_loader.dll"),
            b"steam loader",
        )
        .unwrap();
        fs::write(skse_data.join("SKSE.pex"), b"script").unwrap();

        // Create a fake game directory.
        let game_dir = tmp.path().join("game");
        fs::create_dir_all(&game_dir).unwrap();

        // Install.
        install_skse_files(&extracted, &game_dir).unwrap();

        // Verify exe/dll files were copied to game root.
        assert!(game_dir.join("skse64_loader.exe").exists());
        assert!(game_dir.join("skse64_2_2_6.dll").exists());
        assert!(game_dir.join("skse64_steam_loader.dll").exists());

        // Verify Data/Scripts was copied.
        assert!(game_dir.join("Data").join("Scripts").join("SKSE.pex").exists());
    }

    #[test]
    fn skse_status_serializes() {
        let status = SkseStatus {
            installed: true,
            loader_path: Some("/game/skse64_loader.exe".to_string()),
            version: Some("2.2.6".to_string()),
            use_skse: true,
        };

        let json = serde_json::to_string(&status).unwrap();
        let deserialized: SkseStatus = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.installed, true);
        assert_eq!(
            deserialized.loader_path,
            Some("/game/skse64_loader.exe".to_string())
        );
        assert_eq!(deserialized.version, Some("2.2.6".to_string()));
        assert_eq!(deserialized.use_skse, true);
    }
}
