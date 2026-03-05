//! SKSE (Skyrim Script Extender) detection and installation.
//!
//! Provides utilities for:
//! - Detecting whether SKSE is installed in a Skyrim SE game directory
//! - Auto-downloading the correct SKSE build for the user's game version
//! - Installing SKSE files from a local or downloaded archive
//! - Persisting per-game SKSE preference in the Corkscrew config
//!
//! SKSE is distributed via GitHub releases (ianpatt/skse64) with no
//! redistribution restrictions. Each release targets a specific Skyrim version.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use std::sync::Arc;

use crate::config;
use crate::database::ModDatabase;
use crate::downgrader;
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

/// GitHub repo for SKSE64 releases.
const SKSE_GITHUB_REPO: &str = "ianpatt/skse64";

// ---------------------------------------------------------------------------
// SKSE Version ↔ Game Version Database
// ---------------------------------------------------------------------------

/// Static mapping of SKSE versions to their target Skyrim game versions.
#[allow(dead_code)]
struct SkseVersionEntry {
    /// SKSE tag (e.g., "v2.2.6")
    tag: &'static str,
    /// Target Skyrim game version (e.g., "1.6.1170")
    game_version: &'static str,
    /// Whether this is the GOG variant (separate asset)
    has_gog_variant: bool,
}

/// Known SKSE releases mapped to their target Skyrim versions.
/// Ordered newest-first for priority matching.
const SKSE_VERSION_DB: &[SkseVersionEntry] = &[
    SkseVersionEntry {
        tag: "v2.2.6",
        game_version: "1.6.1170",
        has_gog_variant: false,
    },
    SkseVersionEntry {
        tag: "v2.2.5",
        game_version: "1.6.1130",
        has_gog_variant: false,
    },
    SkseVersionEntry {
        tag: "v2.2.4",
        game_version: "1.6.1130",
        has_gog_variant: false,
    },
    SkseVersionEntry {
        tag: "v2.2.3",
        game_version: "1.6.640",
        has_gog_variant: true,
    },
    SkseVersionEntry {
        tag: "v2.2.2",
        game_version: "1.6.640",
        has_gog_variant: false,
    },
    SkseVersionEntry {
        tag: "v2.2.1",
        game_version: "1.6.640",
        has_gog_variant: false,
    },
    SkseVersionEntry {
        tag: "v2.2.0",
        game_version: "1.6.629",
        has_gog_variant: false,
    },
    SkseVersionEntry {
        tag: "v2.1.5",
        game_version: "1.6.353",
        has_gog_variant: false,
    },
    SkseVersionEntry {
        tag: "v2.1.4",
        game_version: "1.6.342",
        has_gog_variant: false,
    },
    SkseVersionEntry {
        tag: "v2.1.3",
        game_version: "1.6.323",
        has_gog_variant: false,
    },
    SkseVersionEntry {
        tag: "v2.1.2",
        game_version: "1.6.318",
        has_gog_variant: false,
    },
    SkseVersionEntry {
        tag: "v2.1.1",
        game_version: "1.6.318",
        has_gog_variant: false,
    },
    SkseVersionEntry {
        tag: "v2.1.0",
        game_version: "1.6.317",
        has_gog_variant: false,
    },
    SkseVersionEntry {
        tag: "v2.0.20",
        game_version: "1.5.97",
        has_gog_variant: false,
    },
];

/// Available SKSE builds for a specific game version, returned to frontend.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkseAvailableBuilds {
    /// The detected or provided game version string.
    pub game_version: String,
    /// Whether the game version is SE (1.5.x) or AE (1.6.x).
    pub edition: String,
    /// Recommended SKSE build (best match for the game version).
    pub recommended: Option<SkseBuild>,
    /// All compatible builds, newest first.
    pub all_builds: Vec<SkseBuild>,
}

/// A single SKSE build that can be downloaded.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkseBuild {
    /// SKSE version tag (e.g., "v2.2.6")
    pub tag: String,
    /// SKSE version without prefix (e.g., "2.2.6")
    pub version: String,
    /// Target game version (e.g., "1.6.1170")
    pub target_game_version: String,
    /// Direct download URL from GitHub releases
    pub download_url: String,
    /// Asset filename (e.g., "skse64_2_02_06.7z")
    pub filename: String,
    /// Whether this is the recommended build for the detected game version
    pub is_recommended: bool,
}

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
            let stem = name.trim_start_matches("skse64_").trim_end_matches(".dll");

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
// Uninstall
// ---------------------------------------------------------------------------

/// Remove SKSE files from the game directory.
///
/// Removes:
/// - `skse64_loader.exe`
/// - All `skse64_*.dll` files in the game root
/// - `Data/SKSE/` directory (SKSE scripts and plugins)
/// - `Data/Scripts/SKSE/` if it exists
pub fn uninstall_skse(game_path: &Path) -> Result<SkseStatus> {
    info!("Uninstalling SKSE from {}", game_path.display());

    let mut removed = 0u32;

    // Remove SKSE executables and DLLs from game root
    if let Ok(entries) = fs::read_dir(game_path) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name == "skse64_loader.exe"
                || (name.starts_with("skse64_") && name.ends_with(".dll"))
            {
                if let Err(e) = fs::remove_file(entry.path()) {
                    warn!("Failed to remove {}: {}", entry.path().display(), e);
                } else {
                    info!("Removed {}", entry.path().display());
                    removed += 1;
                }
            }
        }
    }

    // Remove Data/SKSE/ directory (SKSE plugins and scripts)
    let data_skse = game_path.join("Data").join("SKSE");
    if data_skse.exists() {
        match fs::remove_dir_all(&data_skse) {
            Ok(()) => {
                info!("Removed {}", data_skse.display());
                removed += 1;
            }
            Err(e) => warn!("Failed to remove {}: {}", data_skse.display(), e),
        }
    }

    info!("SKSE uninstall complete: {} items removed", removed);

    // Return updated status
    Ok(detect_skse(game_path))
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
pub fn install_skse_from_archive(game_path: &Path, archive_path: &Path) -> Result<SkseStatus> {
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

    installer::extract_archive(archive_path, &extract_dir)
        .map_err(|e| SkseError::Extraction(format!("Failed to extract SKSE archive: {}", e)))?;

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

// ---------------------------------------------------------------------------
// Auto-download from GitHub
// ---------------------------------------------------------------------------

/// Convert an SKSE tag like "v2.2.6" to an asset filename like "skse64_2_02_06.7z".
fn tag_to_asset_name(tag: &str) -> Option<String> {
    let version = tag.strip_prefix('v')?;
    let parts: Vec<u32> = version.split('.').filter_map(|p| p.parse().ok()).collect();
    if parts.len() != 3 {
        return None;
    }
    Some(format!(
        "skse64_{}_{:02}_{:02}.7z",
        parts[0], parts[1], parts[2]
    ))
}

/// Parse a game version string like "1.6.1170" into a comparable tuple.
fn parse_game_version(v: &str) -> Option<(u32, u32, u32)> {
    // Strip any suffix text (e.g., "1.5.97 (Special Edition)")
    let clean = v.split_whitespace().next().unwrap_or(v);
    let parts: Vec<u32> = clean.split('.').filter_map(|p| p.parse().ok()).collect();
    if parts.len() >= 3 {
        Some((parts[0], parts[1], parts[2]))
    } else {
        None
    }
}

/// Get available SKSE builds for a given game version.
///
/// Uses the static version database to find compatible builds.
/// The recommended build is the newest SKSE that targets a game version
/// less than or equal to the user's game version.
pub fn get_available_skse_builds(game_version: &str) -> SkseAvailableBuilds {
    let edition = if game_version.contains("1.5") {
        "SE"
    } else if game_version.contains("1.6") {
        "AE"
    } else {
        "Unknown"
    };

    let user_ver = parse_game_version(game_version);

    // If the version string is unparseable (e.g. "1.6.x") but clearly AE,
    // assume the latest game version so we recommend the newest SKSE for AE.
    let effective_ver = user_ver.or_else(|| {
        if edition == "AE" {
            log::info!(
                "Game version '{}' unparseable — assuming latest AE for SKSE matching",
                game_version
            );
            Some((1, 6, u32::MAX))
        } else {
            None
        }
    });

    let mut all_builds: Vec<SkseBuild> = Vec::new();
    let mut recommended: Option<SkseBuild> = None;

    for entry in SKSE_VERSION_DB {
        let filename = match tag_to_asset_name(entry.tag) {
            Some(f) => f,
            None => continue,
        };

        let download_url = format!(
            "https://github.com/{}/releases/download/{}/{}",
            SKSE_GITHUB_REPO, entry.tag, filename
        );

        let version = entry.tag.strip_prefix('v').unwrap_or(entry.tag).to_string();

        // Determine if this build is compatible with the user's game version.
        // SKSE is forward-compatible within a version range, so we pick the
        // newest SKSE whose target game version <= user's game version.
        let target_ver = parse_game_version(entry.game_version);
        let compatible = match (effective_ver, target_ver) {
            (Some(u), Some(t)) => t <= u,
            _ => false,
        };

        let build = SkseBuild {
            tag: entry.tag.to_string(),
            version,
            target_game_version: entry.game_version.to_string(),
            download_url,
            filename,
            is_recommended: false,
        };

        if compatible && recommended.is_none() {
            let mut rec = build.clone();
            rec.is_recommended = true;
            recommended = Some(rec.clone());
            all_builds.push(rec);
        } else {
            all_builds.push(build);
        }
    }

    SkseAvailableBuilds {
        game_version: game_version.to_string(),
        edition: edition.to_string(),
        recommended,
        all_builds,
    }
}

/// Download and install the recommended SKSE build for the user's game version.
///
/// Fetches the .7z archive from GitHub releases, extracts it, and copies
/// the SKSE files into the game directory.
pub async fn install_skse_auto(game_path: &Path, game_version: &str) -> Result<SkseStatus> {
    let builds = get_available_skse_builds(game_version);
    let build = builds.recommended.ok_or_else(|| {
        SkseError::Other(format!(
            "No compatible SKSE build found for game version {}",
            game_version
        ))
    })?;

    info!(
        "Auto-downloading SKSE {} for game version {} from {}",
        build.tag, game_version, build.download_url
    );

    let client = reqwest::Client::builder()
        .user_agent(format!("Corkscrew/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| SkseError::Other(format!("HTTP client error: {}", e)))?;

    let bytes = client
        .get(&build.download_url)
        .send()
        .await
        .map_err(|e| SkseError::Other(format!("Download failed: {}", e)))?
        .error_for_status()
        .map_err(|e| SkseError::Other(format!("Download failed: {}", e)))?
        .bytes()
        .await
        .map_err(|e| SkseError::Other(format!("Download failed: {}", e)))?;

    info!(
        "Downloaded {} ({} bytes), extracting...",
        build.filename,
        bytes.len()
    );

    // Write to a temp file and extract
    let extract_dir = std::env::temp_dir().join("corkscrew_skse_extract");
    if extract_dir.exists() {
        fs::remove_dir_all(&extract_dir)?;
    }
    fs::create_dir_all(&extract_dir)?;

    // SKSE archives are .7z — write to temp then extract
    let tmp_archive = extract_dir.join(&build.filename);
    fs::write(&tmp_archive, &bytes)?;

    installer::extract_archive(&tmp_archive, &extract_dir)
        .map_err(|e| SkseError::Extraction(format!("Failed to extract SKSE archive: {}", e)))?;

    // Remove the archive file after extraction
    let _ = fs::remove_file(&tmp_archive);

    // Install the files
    install_skse_files(&extract_dir, game_path)?;

    // Clean up
    if let Err(e) = fs::remove_dir_all(&extract_dir) {
        warn!("Failed to clean up SKSE extraction dir: {}", e);
    }

    let status = detect_skse(game_path);
    info!(
        "SKSE auto-install complete: installed={}, version={:?}",
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
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
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
    } else if version_str.contains("Anniversary") || version_str.contains("1.6") {
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
// SKSE plugin DLL version compatibility (PE parsing)
// ---------------------------------------------------------------------------

/// SKSE plugin version data struct, matching the C++ `SKSEPluginVersionData`
/// from SKSE64's PluginAPI.h. Exported by SKSE plugins as `SKSEPlugin_Version`.
/// Total size: 0x350 (848 bytes).
#[repr(C)]
#[derive(Copy, Clone)]
struct SKSEPluginVersionData {
    data_version: u32,              // 0x000 — must be 1
    plugin_version: u32,            // 0x004
    name: [u8; 256],                // 0x008
    author: [u8; 256],              // 0x108
    support_email: [u8; 252],       // 0x208
    version_independence_ex: u32,   // 0x304
    version_independence: u32,      // 0x308
    compatible_versions: [u32; 16], // 0x30C — zero-terminated list
    se_version_required: u32,       // 0x34C
}

// Safety: SKSEPluginVersionData is #[repr(C)] with only primitive fields.
unsafe impl pelite::Pod for SKSEPluginVersionData {}

/// Version independence flags from SKSE64 PluginAPI.h.
const VERSION_INDEPENDENT_ADDRESS_LIBRARY: u32 = 1 << 0;
const VERSION_INDEPENDENT_SIGNATURES: u32 = 1 << 1;

/// Result of checking an SKSE plugin DLL's compatibility with a game version.
#[derive(Clone, Debug)]
pub enum SksePluginCompat {
    /// DLL explicitly lists this game version as compatible.
    Compatible,
    /// DLL does not list this game version. `supports` has human-readable versions it does support.
    Incompatible { supports: Vec<String> },
    /// DLL uses Address Library or signature scanning — works with any game version.
    VersionIndependent,
    /// DLL has no `SKSEPlugin_Version` export (legacy plugin).
    NoVersionData,
    /// Could not parse the DLL.
    ParseError(String),
}

/// Encode a game version string like "1.6.1170" into SKSE's runtime version u32.
/// Format: `(major << 24) | (minor << 16) | (build << 4)`.
pub fn parse_game_version_to_runtime(version_str: &str) -> Option<u32> {
    let parts: Vec<&str> = version_str.split('.').collect();
    if parts.len() < 3 {
        return None;
    }
    let major: u8 = parts[0].parse().ok()?;
    let minor: u8 = parts[1].parse().ok()?;
    // Build might have non-numeric suffix (e.g., "1170") — parse digits only
    let build_str: String = parts[2]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    match build_str.parse::<u16>() {
        Ok(build) => Some(((major as u32) << 24) | ((minor as u32) << 16) | ((build as u32) << 4)),
        Err(_) => {
            // Wildcard version like "1.6.x (Anniversary Edition, ~35.4 MB)" —
            // the exe hash didn't match any known version but file size indicates AE.
            // Use 1.6.1170 as representative AE runtime for compat checks.
            if major == 1 && minor == 6 {
                info!(
                    "parse_game_version_to_runtime: wildcard '{}' → treating as AE 1.6.1170",
                    version_str
                );
                Some((1u32 << 24) | (6u32 << 16) | (1170u32 << 4))
            } else {
                None
            }
        }
    }
}

/// Decode SKSE runtime version u32 back to human-readable string.
fn decode_runtime_version(v: u32) -> String {
    let major = (v >> 24) & 0xFF;
    let minor = (v >> 16) & 0xFF;
    let build = (v >> 4) & 0xFFF;
    format!("{}.{}.{}", major, minor, build)
}

/// Check an SKSE plugin DLL's compatibility with a specific game runtime version.
///
/// Parses the PE export table to find the `SKSEPlugin_Version` data export,
/// reads the `SKSEPluginVersionData` struct, and checks whether the DLL supports
/// the given game version — the same check SKSE performs at load time.
pub fn check_skse_dll_compat(dll_path: &Path, game_runtime_version: u32) -> SksePluginCompat {
    let bytes = match std::fs::read(dll_path) {
        Ok(b) => b,
        Err(e) => return SksePluginCompat::ParseError(format!("read error: {}", e)),
    };

    // Try PE64 first (most SKSE plugins are 64-bit), fall back to PE32
    let version_data = read_version_data_pe64(&bytes).or_else(|| read_version_data_pe32(&bytes));

    let data = match version_data {
        Some(d) => d,
        None => return SksePluginCompat::NoVersionData,
    };

    // Check version independence flags
    if data.version_independence & VERSION_INDEPENDENT_SIGNATURES != 0 {
        // Signature scanning — truly version-independent, works with any game version
        return SksePluginCompat::VersionIndependent;
    }

    if data.version_independence & VERSION_INDEPENDENT_ADDRESS_LIBRARY != 0 {
        // "Uses Address Library" — the plugin resolves offsets at runtime via an
        // Address Library database file. Both SE and AE plugins set this flag with
        // data_version=1 and empty compatible_versions, so we CANNOT distinguish
        // SE-only from AE-compatible plugins via PE metadata alone.
        // Return VersionIndependent here; the fix_skse_plugin_conflicts() function
        // uses a file-comparison approach to detect and swap mismatched DLLs.
        return SksePluginCompat::VersionIndependent;
    }

    // Check compatible_versions array (zero-terminated)
    let mut supported = Vec::new();
    for &ver in &data.compatible_versions {
        if ver == 0 {
            break;
        }
        if ver == game_runtime_version {
            return SksePluginCompat::Compatible;
        }
        supported.push(decode_runtime_version(ver));
    }

    if supported.is_empty() {
        // No versions listed and not version-independent — treat as unknown
        SksePluginCompat::NoVersionData
    } else {
        SksePluginCompat::Incompatible {
            supports: supported,
        }
    }
}

/// Extract the plugin name from an SKSE plugin DLL's version data.
pub fn get_skse_plugin_name(dll_path: &Path) -> Option<String> {
    let bytes = std::fs::read(dll_path).ok()?;
    let data = read_version_data_pe64(&bytes).or_else(|| read_version_data_pe32(&bytes))?;
    let name = std::str::from_utf8(&data.name)
        .ok()?
        .trim_end_matches('\0')
        .to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Try to read SKSEPluginVersionData from a PE64 DLL.
fn read_version_data_pe64(bytes: &[u8]) -> Option<SKSEPluginVersionData> {
    use pelite::pe64::{Pe, PeFile};
    let pe = PeFile::from_bytes(bytes).ok()?;
    let exports = pe.exports().ok()?.by().ok()?;
    let export = exports.name("SKSEPlugin_Version").ok()?;
    let rva = export.symbol()?;
    let data: &SKSEPluginVersionData = pe.derva(rva).ok()?;
    if data.data_version == 0 || data.data_version > 2 {
        return None;
    }
    Some(*data)
}

/// Try to read SKSEPluginVersionData from a PE32 DLL.
fn read_version_data_pe32(bytes: &[u8]) -> Option<SKSEPluginVersionData> {
    use pelite::pe32::{Pe, PeFile};
    let pe = PeFile::from_bytes(bytes).ok()?;
    let exports = pe.exports().ok()?.by().ok()?;
    let export = exports.name("SKSEPlugin_Version").ok()?;
    let rva = export.symbol()?;
    let data: &SKSEPluginVersionData = pe.derva(rva).ok()?;
    if data.data_version == 0 || data.data_version > 2 {
        return None;
    }
    Some(*data)
}

// ---------------------------------------------------------------------------
// Post-install SKSE plugin scan
// ---------------------------------------------------------------------------

/// Warning about a specific SKSE plugin DLL.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SksePluginWarning {
    /// Name of the DLL file.
    pub dll_name: String,
    /// Description of the issue.
    pub warning: String,
}

/// Results of scanning SKSE plugins for compatibility issues.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SksePluginScanResult {
    /// Total number of DLL files found in the plugins directory.
    pub total_plugins: usize,
    /// Address Library version detected ("SE" for 1.5.97, "AE" for 1.6.x, or "unknown").
    pub address_library_version: String,
    /// Whether the Address Library matches the game version.
    pub address_library_matches: bool,
    /// Specific DLL warnings.
    pub warnings: Vec<SksePluginWarning>,
}

/// Scan SKSE plugins directory for compatibility issues.
///
/// Checks:
/// - Number of DLL plugins present
/// - Address Library version files (`versionlib-1-5-97-0.bin` for SE, `versionlib-1-6-*.bin` for AE)
/// - Whether Address Library matches the game version
/// - Each DLL's PE-exported `SKSEPlugin_Version` data for version compatibility
pub fn scan_skse_plugins(data_dir: &Path, game_version: &str) -> SksePluginScanResult {
    let plugins_dir = data_dir.join("SKSE").join("Plugins");
    let mut total_plugins = 0;
    let mut warnings = Vec::new();
    let mut dll_paths: Vec<PathBuf> = Vec::new();

    // Collect DLL plugins
    if plugins_dir.exists() {
        if let Ok(entries) = fs::read_dir(&plugins_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if name.ends_with(".dll") {
                    total_plugins += 1;
                    dll_paths.push(entry.path());
                }
            }
        }
    }

    // Check Address Library version
    let is_se = game_version.starts_with("1.5");
    let se_lib = data_dir
        .join("SKSE")
        .join("Plugins")
        .join("versionlib-1-5-97-0.bin");
    let has_se_lib = se_lib.exists();

    // Check for any AE address library file
    let mut has_ae_lib = false;
    if plugins_dir.exists() {
        if let Ok(entries) = fs::read_dir(&plugins_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if name.starts_with("versionlib-1-6-") && name.ends_with(".bin") {
                    has_ae_lib = true;
                    break;
                }
            }
        }
    }

    let address_library_version = if has_se_lib && has_ae_lib {
        "both".to_string()
    } else if has_se_lib {
        "SE".to_string()
    } else if has_ae_lib {
        "AE".to_string()
    } else {
        "none".to_string()
    };

    let address_library_matches = if !has_se_lib && !has_ae_lib {
        true // No Address Library present — nothing to mismatch
    } else if is_se {
        has_se_lib
    } else {
        has_ae_lib
    };

    if !address_library_matches {
        let expected = if is_se { "SE (1.5.97)" } else { "AE (1.6.x)" };
        let found = if has_se_lib { "SE" } else { "AE" };
        warnings.push(SksePluginWarning {
            dll_name: "Address Library".to_string(),
            warning: format!(
                "Address Library is for {} but your game is {}. SKSE plugins may crash.",
                found, expected
            ),
        });
    }

    // Per-DLL version compatibility check (if we can parse the game version)
    if let Some(runtime) = parse_game_version_to_runtime(game_version) {
        for dll_path in &dll_paths {
            let dll_name = dll_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            match check_skse_dll_compat(dll_path, runtime) {
                SksePluginCompat::Incompatible { supports } => {
                    let plugin_name =
                        get_skse_plugin_name(dll_path).unwrap_or_else(|| dll_name.clone());
                    warnings.push(SksePluginWarning {
                        dll_name: dll_name.clone(),
                        warning: format!(
                            "{}: incompatible (built for {}, game is {})",
                            plugin_name,
                            supports.join("/"),
                            game_version,
                        ),
                    });
                }
                SksePluginCompat::ParseError(e) => {
                    debug!("Could not parse SKSE plugin '{}': {}", dll_name, e);
                }
                _ => {} // Compatible, VersionIndependent, NoVersionData — all fine
            }
        }
    }

    info!(
        "SKSE plugin scan: {} plugins, Address Library: {} (matches: {}), {} warnings",
        total_plugins,
        address_library_version,
        address_library_matches,
        warnings.len()
    );

    SksePluginScanResult {
        total_plugins,
        address_library_version,
        address_library_matches,
        warnings,
    }
}

// ---------------------------------------------------------------------------
// Post-deploy SKSE plugin conflict fixer
// ---------------------------------------------------------------------------

/// Scan deployed SKSE plugin DLLs and fix version incompatibilities by swapping
/// in alternative builds from other installed mods' staging directories.
///
/// **Strategy:** Since SE and AE Address Library plugins have identical PE metadata
/// (both set data_version=1 + Address Library flag with empty compatible_versions),
/// we cannot distinguish them via static analysis. Instead we use two approaches:
///
/// 1. **PE compat check** — for DLLs with explicit compatible_versions, swap if
///    the game version isn't listed.
/// 2. **Alternative swap** — for Address Library DLLs (VersionIndependent), check
///    if another mod provides a DIFFERENT build of the same DLL (different file
///    size). If so, on AE the current DLL might be an SE build. Swap in the
///    alternative. This correctly handles cases like BehaviorDataInjector where
///    an SE-only mod (1.3M) and a Universal Support mod (1.4M) both provide the
///    same DLL.
///
/// Check if a mod name suggests it's the AE-compatible build of an SKSE plugin.
/// Used to add directional preference when both SE and AE Address Library DLLs
/// are available — prevents oscillating swaps between launches.
fn is_ae_preferred_mod_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("universal")
        || lower.contains("anniversary")
        || lower.contains(" ae")
        || lower.contains("ae ")
        || lower.contains("(ae)")
        || lower.contains("[ae]")
}

/// Runs during collection install and before every Skyrim SE launch.
/// Returns the number of DLLs swapped.
pub fn fix_skse_plugin_conflicts(
    db: &Arc<ModDatabase>,
    game_id: &str,
    bottle_name: &str,
    data_dir: &Path,
    game_path: &Path,
) -> usize {
    if game_id != "skyrimse" {
        return 0;
    }

    // Detect game version
    let ds = match downgrader::detect_skyrim_version(game_path) {
        Ok(status) => status,
        Err(e) => {
            debug!("SKSE plugin fix: could not detect game version: {}", e);
            return 0;
        }
    };
    let runtime = match parse_game_version_to_runtime(&ds.current_version) {
        Some(v) => v,
        None => {
            debug!(
                "SKSE plugin fix: could not parse game version '{}'",
                ds.current_version
            );
            return 0;
        }
    };

    let ae_cutoff = (1u32 << 24) | (6u32 << 16) | (629u32 << 4); // 1.6.629
    let is_ae = runtime >= ae_cutoff;

    let plugins_dir = data_dir.join("SKSE").join("Plugins");
    if !plugins_dir.exists() {
        return 0;
    }

    // Collect deployed DLLs
    let deployed_dlls: Vec<PathBuf> = match fs::read_dir(&plugins_dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_type().map(|ft| ft.is_file()).unwrap_or(false)
                    && e.file_name()
                        .to_string_lossy()
                        .to_lowercase()
                        .ends_with(".dll")
            })
            .map(|e| e.path())
            .collect(),
        Err(_) => return 0,
    };

    if deployed_dlls.is_empty() {
        return 0;
    }

    let all_mods = match db.list_mods(game_id, bottle_name) {
        Ok(mods) => mods,
        Err(_) => return 0,
    };

    let manifest_map = match db.get_deployment_manifest_map(game_id, bottle_name) {
        Ok(m) => m,
        Err(_) => return 0,
    };

    let mut fixes = 0usize;

    for dll_path in &deployed_dlls {
        let dll_name = match dll_path.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => continue,
        };

        let compat = check_skse_dll_compat(dll_path, runtime);

        // Determine if this DLL needs fixing
        let needs_fix = match &compat {
            // Explicit incompatibility from compatible_versions check
            SksePluginCompat::Incompatible { .. } => true,
            // Address Library DLL on AE — might be SE build, check for a BETTER alternative.
            // Uses directional preference to prevent oscillating swaps between launches:
            // prefer mods with "Universal"/"AE" in name, then larger DLLs as fallback.
            SksePluginCompat::VersionIndependent if is_ae => {
                let deployed_size = fs::metadata(dll_path).map(|m| m.len()).unwrap_or(0);
                let rel_skse = format!("SKSE/Plugins/{}", dll_name);
                let current_owner_id = manifest_map.get(&rel_skse).map(|e| e.mod_id);

                // If current owner mod already has an AE-indicating name, don't swap
                let current_mod_name = current_owner_id
                    .and_then(|id| all_mods.iter().find(|m| m.id == id))
                    .map(|m| m.name.as_str())
                    .unwrap_or("");
                if is_ae_preferred_mod_name(current_mod_name) {
                    false
                } else {
                    // Look for a preferred alternative (AE-named or larger)
                    all_mods.iter().any(|m| {
                        if Some(m.id) == current_owner_id || !m.enabled {
                            return false;
                        }
                        let staging = match &m.staging_path {
                            Some(s) => PathBuf::from(s),
                            None => return false,
                        };
                        let candidate = staging.join("SKSE").join("Plugins").join(&dll_name);
                        let alt_size = if let Ok(meta) = fs::metadata(&candidate) {
                            meta.len()
                        } else {
                            find_file_case_insensitive(
                                &staging.join("SKSE").join("Plugins"),
                                &dll_name,
                            )
                            .and_then(|p| fs::metadata(&p).ok())
                            .map(|meta| meta.len())
                            .unwrap_or(0)
                        };
                        if alt_size == 0 || alt_size == deployed_size {
                            return false;
                        }

                        // Swap only toward AE-preferred mods, or larger DLLs as tiebreaker
                        is_ae_preferred_mod_name(&m.name) || alt_size > deployed_size
                    })
                }
            }
            _ => false,
        };

        if !needs_fix {
            continue;
        }

        let reason = match &compat {
            SksePluginCompat::Incompatible { supports } => {
                format!("incompatible (supports: {})", supports.join(", "))
            }
            SksePluginCompat::VersionIndependent => {
                "Address Library plugin with different-sized alternative available".to_string()
            }
            _ => "unknown".to_string(),
        };

        info!(
            "SKSE plugin fix: '{}' — {}, searching for alternative...",
            dll_name, reason
        );

        let rel_skse = format!("SKSE/Plugins/{}", dll_name);
        let current_owner_id = manifest_map.get(&rel_skse).map(|e| e.mod_id);
        let deployed_size = fs::metadata(dll_path).map(|m| m.len()).unwrap_or(0);

        // Search all mods' staging directories for an alternative
        let mut best_candidate: Option<(PathBuf, i64, String)> = None;
        let mut best_is_ae_preferred = false;

        for m in &all_mods {
            if Some(m.id) == current_owner_id {
                continue;
            }
            if !m.enabled {
                continue;
            }

            let staging = match &m.staging_path {
                Some(s) => PathBuf::from(s),
                None => continue,
            };

            let candidate_path = staging.join("SKSE").join("Plugins").join(&dll_name);
            let candidate = if candidate_path.exists() {
                Some(candidate_path)
            } else {
                find_file_case_insensitive(&staging.join("SKSE").join("Plugins"), &dll_name)
            };

            let candidate = match candidate {
                Some(p) => p,
                None => continue,
            };

            let alt_compat = check_skse_dll_compat(&candidate, runtime);
            let alt_size = fs::metadata(&candidate).map(|m| m.len()).unwrap_or(0);

            match alt_compat {
                SksePluginCompat::Compatible => {
                    // Explicit version match — best possible
                    best_candidate = Some((candidate, m.id, "compatible".to_string()));
                    break;
                }
                SksePluginCompat::VersionIndependent => {
                    // Both are VersionIndependent (Address Library) — use directional
                    // preference to pick the right one and prevent oscillation:
                    // 1. Prefer AE-named mods ("Universal", "AE", "Anniversary")
                    // 2. Fallback: prefer larger DLLs (AE builds tend to be larger)
                    if alt_size != deployed_size && alt_size > 0 {
                        let alt_is_ae = is_ae_preferred_mod_name(&m.name);
                        if alt_is_ae || alt_size > deployed_size {
                            let quality = format!(
                                "alternative build ({}KB vs deployed {}KB)",
                                alt_size / 1024,
                                deployed_size / 1024
                            );
                            // AE-named mods beat non-AE; among same tier, first wins
                            if best_candidate.is_none() || (alt_is_ae && !best_is_ae_preferred) {
                                best_candidate = Some((candidate, m.id, quality));
                                best_is_ae_preferred = alt_is_ae;
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        if let Some((src_path, new_owner_id, quality)) = best_candidate {
            let new_owner_name = all_mods
                .iter()
                .find(|m| m.id == new_owner_id)
                .map(|m| m.name.as_str())
                .unwrap_or("unknown");

            info!(
                "SKSE plugin fix: swapping '{}' — deploying {} from mod '{}' (id {})",
                dll_name, quality, new_owner_name, new_owner_id
            );

            if let Err(e) = fs::remove_file(dll_path) {
                warn!(
                    "SKSE plugin fix: failed to remove '{}': {}",
                    dll_path.display(),
                    e
                );
                continue;
            }

            if let Err(e) = fs::copy(&src_path, dll_path) {
                warn!(
                    "SKSE plugin fix: failed to copy '{}' -> '{}': {}",
                    src_path.display(),
                    dll_path.display(),
                    e
                );
                continue;
            }

            let _ = db.add_deployment_entry(
                game_id,
                bottle_name,
                new_owner_id,
                &rel_skse,
                &src_path.to_string_lossy(),
                "copy",
                None,
            );

            fixes += 1;
        } else {
            warn!(
                "SKSE plugin fix: no alternative found for '{}' ({})",
                dll_name, reason
            );
        }
    }

    fixes
}

// ---------------------------------------------------------------------------
// EngineFixes Wine compatibility
// ---------------------------------------------------------------------------

/// Disable all EngineFixes patches that are incompatible with Wine/CrossOver.
///
/// Engine Fixes' code-patching mechanism (hooking game functions during the
/// d3dx9_42.dll preload phase) crashes under Wine ~63 seconds into launch
/// during InitTESThread.  This affects ALL [Fixes] and [Patches] — even
/// trivial ones like MaxStdIO — not just the memory manager.
///
/// The DLL itself loads fine; it's the actual hooks that are the problem.
/// We disable every boolean setting in [Fixes], [Patches], [MemoryManager],
/// and [Warnings], then force `bDisableTBB = true` in [Debug].
///
/// We patch every `EngineFixes.toml` we can find — both in the deployed
/// game Data directory and in every staging directory that contains one —
/// so the fix persists across redeploys.
///
/// Returns the number of TOML files patched.
pub fn fix_engine_fixes_for_wine(
    data_dir: &Path,
    db: &Arc<ModDatabase>,
    game_id: &str,
    bottle_name: &str,
) -> usize {
    let mut patched = 0;

    // Disable the d3dx9_42.dll preloader — it crashes Wine because it hooks
    // during DLL load, before the SKSE plugin phase.  We rename it rather
    // than delete so it can be restored if the user ever switches to native
    // Windows.  Check both the deployed game root AND every staging dir that
    // ships the preloader (collections often include it as a root-deploy file).
    disable_engine_fixes_preloader(data_dir, db, game_id, bottle_name);

    // Collect all paths to check: deployed + every staging dir
    let mut toml_paths: Vec<PathBuf> = Vec::new();

    // 1. Deployed copy
    let deployed = data_dir
        .join("SKSE")
        .join("Plugins")
        .join("EngineFixes.toml");
    if deployed.exists() {
        toml_paths.push(deployed);
    }

    // 2. Staging copies from all installed mods
    if let Ok(mods) = db.list_mods(game_id, bottle_name) {
        for m in &mods {
            if let Some(ref sp) = m.staging_path {
                let staging_toml = PathBuf::from(sp)
                    .join("SKSE")
                    .join("Plugins")
                    .join("EngineFixes.toml");
                if staging_toml.exists() {
                    toml_paths.push(staging_toml);
                }
            }
        }
    }

    for path in &toml_paths {
        match patch_engine_fixes_toml(path) {
            Ok(true) => {
                info!("EngineFixes Wine fix: patched {}", path.display());
                patched += 1;
            }
            Ok(false) => {
                debug!("EngineFixes Wine fix: already patched {}", path.display());
            }
            Err(e) => {
                warn!(
                    "EngineFixes Wine fix: failed to patch {}: {}",
                    path.display(),
                    e
                );
            }
        }
    }

    patched
}

/// Disable the original Engine Fixes files that are incompatible with Wine.
///
/// Two files must be neutralized:
/// 1. `d3dx9_42.dll` (game root) — preloader that hooks during DLL load,
///    crashes Wine ~63s into launch.
/// 2. `EngineFixes.dll` (Data/SKSE/Plugins/) — the original SKSE plugin
///    that checks for the preloader and force-closes the game if it's missing.
///
/// Both are renamed to `.dll.disabled` (recoverable, not deleted).
/// We also disable these in staging dirs so redeploys don't bring them back.
fn disable_engine_fixes_preloader(
    data_dir: &Path,
    db: &Arc<ModDatabase>,
    game_id: &str,
    bottle_name: &str,
) {
    // Helper: rename a DLL to .disabled if it exists
    let disable_dll = |path: &Path| {
        if path.exists() {
            let disabled = path.with_extension("dll.disabled");
            match fs::rename(path, &disabled) {
                Ok(()) => info!("EngineFixes Wine fix: disabled {}", path.display()),
                Err(e) => warn!(
                    "EngineFixes Wine fix: failed to disable {}: {}",
                    path.display(),
                    e
                ),
            }
        }
    };

    // 1. Deployed copies
    if let Some(game_root) = data_dir.parent() {
        // Preloader in game root
        disable_dll(&game_root.join("d3dx9_42.dll"));
    }
    // Original SKSE plugin in Data/SKSE/Plugins/
    disable_dll(
        &data_dir
            .join("SKSE")
            .join("Plugins")
            .join("EngineFixes.dll"),
    );

    // 2. Staging copies — collections often include both files
    if let Ok(mods) = db.list_mods(game_id, bottle_name) {
        for m in &mods {
            if let Some(ref sp) = m.staging_path {
                let sp = PathBuf::from(sp);
                // Root-deployed preloader
                disable_dll(&sp.join("d3dx9_42.dll"));
                // SKSE plugin
                disable_dll(&sp.join("SKSE").join("Plugins").join("EngineFixes.dll"));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Wine-incompatible SKSE plugin management
// ---------------------------------------------------------------------------

/// SKSE plugins that are known to be incompatible with Wine and must be disabled.
/// Each entry is (dll_name, reason) — the DLL is renamed to `.dll.disabled`.
const WINE_INCOMPATIBLE_PLUGINS: &[(&str, &str)] = &[(
    "CrashLogger.dll",
    "CrashLogger's VEH handler conflicts with Wine exception handling, causing CTDs",
)];

/// Disable SKSE plugins that are known to be incompatible with Wine.
///
/// Returns a list of `(dll_name, reason)` for each plugin that was actually disabled
/// during this call (i.e., plugins already disabled are not re-reported).
pub fn disable_wine_incompatible_plugins(
    data_dir: &Path,
    db: &Arc<ModDatabase>,
    game_id: &str,
    bottle_name: &str,
) -> Vec<(String, String)> {
    let mut disabled = Vec::new();
    let plugins_dir = data_dir.join("SKSE").join("Plugins");

    for &(dll_name, reason) in WINE_INCOMPATIBLE_PLUGINS {
        // 1. Deployed copy
        let deployed = plugins_dir.join(dll_name);
        if deployed.exists() {
            let target = deployed.with_extension("dll.disabled");
            match fs::rename(&deployed, &target) {
                Ok(()) => {
                    info!("Wine compat: disabled {} — {}", dll_name, reason);
                    disabled.push((dll_name.to_string(), reason.to_string()));
                }
                Err(e) => warn!("Wine compat: failed to disable {}: {}", dll_name, e),
            }
        }

        // 2. Staging copies — prevent redeploys from restoring them
        if let Ok(mods) = db.list_mods(game_id, bottle_name) {
            for m in &mods {
                if let Some(ref sp) = m.staging_path {
                    let staged = PathBuf::from(sp)
                        .join("SKSE")
                        .join("Plugins")
                        .join(dll_name);
                    if staged.exists() {
                        let target = staged.with_extension("dll.disabled");
                        if let Err(e) = fs::rename(&staged, &target) {
                            warn!("Wine compat: failed to disable staged {}: {}", dll_name, e);
                        }
                    }
                }
            }
        }
    }

    disabled
}

/// List all Wine-incompatible plugins that are currently disabled (`.dll.disabled`).
/// Returns `(dll_name, reason)` pairs.
pub fn list_disabled_wine_plugins(data_dir: &Path) -> Vec<(String, String)> {
    let plugins_dir = data_dir.join("SKSE").join("Plugins");
    let mut result = Vec::new();

    for &(dll_name, reason) in WINE_INCOMPATIBLE_PLUGINS {
        let disabled_path = plugins_dir.join(dll_name.replace(".dll", ".dll.disabled"));
        if disabled_path.exists() {
            result.push((dll_name.to_string(), reason.to_string()));
        }
    }

    result
}

/// Re-enable a previously disabled Wine-incompatible plugin.
/// Returns `Ok(true)` if restored, `Ok(false)` if the disabled file wasn't found.
pub fn reenable_wine_plugin(data_dir: &Path, dll_name: &str) -> std::result::Result<bool, String> {
    // Validate it's a known plugin (prevent arbitrary file renames)
    if !WINE_INCOMPATIBLE_PLUGINS
        .iter()
        .any(|&(n, _)| n == dll_name)
    {
        return Err(format!(
            "'{}' is not a known Wine-incompatible plugin",
            dll_name
        ));
    }

    let plugins_dir = data_dir.join("SKSE").join("Plugins");
    let disabled_path = plugins_dir.join(dll_name.replace(".dll", ".dll.disabled"));
    let enabled_path = plugins_dir.join(dll_name);

    if disabled_path.exists() {
        fs::rename(&disabled_path, &enabled_path)
            .map_err(|e| format!("Failed to re-enable {}: {}", dll_name, e))?;
        info!("Wine compat: re-enabled {} (user override)", dll_name);
        Ok(true)
    } else {
        Ok(false)
    }
}

/// All boolean keys in EngineFixes.toml that must be set to `false` under Wine.
/// Every [Fixes], [Patches], [MemoryManager], and [Warnings] boolean is included
/// because Engine Fixes' hooking mechanism itself crashes under Wine.
const WINE_DISABLE_KEYS: &[&str] = &[
    // [Fixes]
    "bArcheryDownwardAiming",
    "bAnimationLoadSignedCrash",
    "bBethesdaNetCrash",
    "bBGSKeywordFormLoadCrash",
    "bBSLightingAmbientSpecular",
    "bBSLightingShaderForceAlphaTest",
    "bBSLightingShaderParallaxBug",
    "bBSLightingShaderPropertyShadowMap",
    "bBSTempEffectNiRTTI",
    "bCalendarSkipping",
    "bCellInit",
    "bClimateLoad",
    "bConjurationEnchantAbsorbs",
    "bCreateArmorNodeNullPtrCrash",
    "bDoublePerkApply",
    "bESLCELLLoadBug",
    "bEquipShoutEventSpam",
    "bFaceGenMorphDataHeadNullPtrCrash",
    "bGetKeywordItemCount",
    "bGHeapLeakDetectionCrash",
    "bGlobalTime",
    "bInitializeHitDataNullPtrCrash",
    "bLipSync",
    "bMemoryAccessErrors",
    "bMO5STypo",
    "bMusicOverlap",
    "bNiControllerNoTarget",
    "bNullProcessCrash",
    "bPerkFragmentIsRunning",
    "bPrecomputedPaths",
    "bRemovedSpellBook",
    "bSaveScreenshots",
    "bSavedHavokDataLoadInit",
    "bShadowSceneNodeNullPtrCrash",
    "bTextureLoadCrash",
    "bTorchLandscape",
    "bTreeReflections",
    "bVerticalLookSensitivity",
    "bWeaponBlockScaling",
    // [Patches]
    "bDisableChargenPrecache",
    "bDisableSnowFlag",
    "bEnableAchievementsWithMods",
    "bFormCaching",
    "bINISettingCollection",
    "bMaxStdIO",
    "bRegularQuicksaves",
    "bSafeExit",
    "bSaveAddedSoundCategories",
    "bScrollingDoesntSwitchPOV",
    "bTreeLodReferenceCaching",
    "bWaterflowAnimation",
    // [MemoryManager]
    "bOverrideMemoryManager",
    "bOverrideScrapHeap",
    "bOverrideScaleformAllocator",
    "bOverrideRenderPassCache",
    "bOverrideHavokMemorySystem",
    "bReplaceImports",
    // [Warnings]
    "bTextureLoadFailed",
    "bPrecomputedPathHasErrors",
    "bRefHandleLimit",
];

/// Patch a single EngineFixes.toml for Wine compatibility:
///   - Set all boolean `[Fixes]`/`[Patches]`/`[MemoryManager]`/`[Warnings]` keys to false
///   - Set `bDisableTBB = true` in `[Debug]` (force CRT allocator instead of TBB)
///
/// Returns `Ok(true)` if the file was modified, `Ok(false)` if already correct.
fn patch_engine_fixes_toml(path: &Path) -> std::result::Result<bool, String> {
    let content =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {}", path.display(), e))?;

    let mut new_content = content.clone();
    let mut changed = false;

    for key in WINE_DISABLE_KEYS {
        let pattern = format!("{} = true", key);
        if new_content.contains(&pattern) {
            let replacement = format!("{} = false", key);
            new_content = new_content.replacen(&pattern, &replacement, 1);
            changed = true;
        }
    }

    // Force CRT allocator — TBB is Wine-incompatible
    if new_content.contains("bDisableTBB = false") {
        new_content = new_content.replacen("bDisableTBB = false", "bDisableTBB = true", 1);
        changed = true;
    }

    if changed {
        // Atomic write: temp file + rename
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, &new_content)
            .map_err(|e| format!("write {}: {}", tmp.display(), e))?;
        std::fs::rename(&tmp, path)
            .map_err(|e| format!("rename {} -> {}: {}", tmp.display(), path.display(), e))?;
    }

    Ok(changed)
}

// ---------------------------------------------------------------------------
// SSE Engine Fixes for Wine — auto-download and deploy
// ---------------------------------------------------------------------------

/// GitHub repo for SSE Engine Fixes for Wine releases.
const ENGINE_FIXES_WINE_REPO: &str = "cashcon57/SSEEngineFixesForWine";

/// DLL name of the Wine-compatible Engine Fixes plugin.
/// Prefixed with "0_" so SKSE loads it before all letter-named plugins,
/// ensuring the editor ID cache fix runs before other plugins' kDataLoaded handlers.
const ENGINE_FIXES_WINE_DLL: &str = "0_SSEEngineFixesForWine.dll";

/// TOML config name for the Wine-compatible Engine Fixes plugin.
const ENGINE_FIXES_WINE_TOML: &str = "SSEEngineFixesForWine.toml";

/// Version marker file — stores the deployed release tag for auto-update.
const ENGINE_FIXES_WINE_VERSION: &str = "SSEEngineFixesForWine.version";

/// Check if SSE Engine Fixes for Wine is already deployed.
pub fn is_engine_fixes_wine_installed(data_dir: &Path) -> bool {
    data_dir
        .join("SKSE")
        .join("Plugins")
        .join(ENGINE_FIXES_WINE_DLL)
        .exists()
}

/// Download and deploy the latest SSE Engine Fixes for Wine release.
///
/// This is the Wine-compatible replacement for SSE Engine Fixes.
/// It downloads the AE build (1.6.1170) from GitHub and deploys
/// `0_SSEEngineFixesForWine.dll` + `SSEEngineFixesForWine.toml`
/// to `Data/SKSE/Plugins/`.
///
/// Returns Ok(true) if newly installed, Ok(false) if already present.
pub async fn install_engine_fixes_wine(data_dir: &Path) -> Result<bool> {
    let plugins_dir = data_dir.join("SKSE").join("Plugins");

    let client = reqwest::Client::builder()
        .user_agent(format!("Corkscrew/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| SkseError::Other(format!("HTTP client error: {}", e)))?;

    // Fetch latest release from GitHub API
    let url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        ENGINE_FIXES_WINE_REPO
    );
    let release: serde_json::Value = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| SkseError::Other(format!("GitHub API request failed: {}", e)))?
        .error_for_status()
        .map_err(|e| SkseError::Other(format!("GitHub API error: {}", e)))?
        .json()
        .await
        .map_err(|e| SkseError::Other(format!("GitHub API JSON parse error: {}", e)))?;

    let tag = release["tag_name"].as_str().unwrap_or("unknown");

    // Check if already deployed at the latest version
    let version_file = plugins_dir.join(ENGINE_FIXES_WINE_VERSION);
    if plugins_dir.join(ENGINE_FIXES_WINE_DLL).exists() {
        if let Ok(deployed_tag) = fs::read_to_string(&version_file) {
            if deployed_tag.trim() == tag {
                debug!(
                    "SSE Engine Fixes for Wine {} already deployed, skipping",
                    tag
                );
                return Ok(false);
            }
            info!(
                "SSE Engine Fixes for Wine update available: {} -> {}",
                deployed_tag.trim(),
                tag
            );
        } else {
            info!(
                "SSE Engine Fixes for Wine deployed without version marker, updating to {}",
                tag
            );
        }
    }

    info!(
        "Downloading SSE Engine Fixes for Wine {} from GitHub...",
        tag
    );

    // Find the AE zip asset (SSEEngineFixesForWine-AE-1.6.1170.zip)
    let assets = release["assets"]
        .as_array()
        .ok_or_else(|| SkseError::Other("No assets in release".into()))?;

    let ae_asset = assets
        .iter()
        .find(|a| {
            a["name"]
                .as_str()
                .map(|n| n.contains("AE") && n.ends_with(".zip"))
                .unwrap_or(false)
        })
        .ok_or_else(|| SkseError::Other("No AE zip asset found in release".into()))?;

    let download_url = ae_asset["browser_download_url"]
        .as_str()
        .ok_or_else(|| SkseError::Other("No download URL for asset".into()))?;

    // Download the zip
    let bytes = client
        .get(download_url)
        .send()
        .await
        .map_err(|e| SkseError::Other(format!("Download failed: {}", e)))?
        .error_for_status()
        .map_err(|e| SkseError::Other(format!("Download failed: {}", e)))?
        .bytes()
        .await
        .map_err(|e| SkseError::Other(format!("Download failed: {}", e)))?;

    info!(
        "Downloaded SSE Engine Fixes for Wine ({} bytes), extracting...",
        bytes.len()
    );

    // Extract to temp directory
    let extract_dir = std::env::temp_dir().join("corkscrew_engine_fixes_wine");
    if extract_dir.exists() {
        fs::remove_dir_all(&extract_dir)?;
    }
    fs::create_dir_all(&extract_dir)?;

    let tmp_zip = extract_dir.join("SSEEngineFixesForWine.zip");
    fs::write(&tmp_zip, &bytes)?;

    installer::extract_archive(&tmp_zip, &extract_dir)
        .map_err(|e| SkseError::Extraction(format!("Failed to extract: {}", e)))?;
    let _ = fs::remove_file(&tmp_zip);

    // Find and copy the DLL and TOML into the game's plugins directory
    fs::create_dir_all(&plugins_dir)?;

    let mut found_dll = false;
    let mut found_toml = false;

    // Walk extracted directory to find our files (they're in SKSE/Plugins/ inside the zip)
    for entry in walkdir::WalkDir::new(&extract_dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.eq_ignore_ascii_case(ENGINE_FIXES_WINE_DLL) {
                fs::copy(path, plugins_dir.join(ENGINE_FIXES_WINE_DLL))?;
                found_dll = true;
                info!("Deployed {}", ENGINE_FIXES_WINE_DLL);
                // Clean up old-named DLL from pre-v1.3.0 (was SSEEngineFixesForWine.dll)
                let old_dll = plugins_dir.join("SSEEngineFixesForWine.dll");
                if old_dll.exists() {
                    let _ = fs::remove_file(&old_dll);
                    info!("Removed old SSEEngineFixesForWine.dll (renamed to 0_ prefix)");
                }
            } else if name.eq_ignore_ascii_case(ENGINE_FIXES_WINE_TOML) {
                // Only copy TOML if it doesn't already exist (preserve user config)
                let dest = plugins_dir.join(ENGINE_FIXES_WINE_TOML);
                if !dest.exists() {
                    fs::copy(path, &dest)?;
                    info!("Deployed {} (default config)", ENGINE_FIXES_WINE_TOML);
                } else {
                    debug!(
                        "{} already exists, preserving user config",
                        ENGINE_FIXES_WINE_TOML
                    );
                }
                found_toml = true;
            }
        }
    }

    // Clean up
    if let Err(e) = fs::remove_dir_all(&extract_dir) {
        warn!("Failed to clean up extraction dir: {}", e);
    }

    if !found_dll {
        return Err(SkseError::Other(
            "0_SSEEngineFixesForWine.dll not found in release archive".into(),
        ));
    }

    if !found_toml {
        warn!(
            "SSEEngineFixesForWine.toml not found in release archive; DLL deployed without config"
        );
    }

    // Write version marker for future update checks
    if let Err(e) = fs::write(&version_file, tag) {
        warn!("Failed to write Engine Fixes Wine version marker: {}", e);
    }

    info!("SSE Engine Fixes for Wine {} installed successfully", tag);
    Ok(true)
}

/// Blocking variant of `install_engine_fixes_wine` for use in sync contexts
/// (pre-launch, deploy commands). Uses reqwest::blocking internally.
///
/// Returns Ok(true) if newly installed, Ok(false) if already present.
pub fn install_engine_fixes_wine_blocking(data_dir: &Path) -> Result<bool> {
    let plugins_dir = data_dir.join("SKSE").join("Plugins");

    let client = reqwest::blocking::Client::builder()
        .user_agent(format!("Corkscrew/{}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| SkseError::Other(format!("HTTP client error: {}", e)))?;

    let url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        ENGINE_FIXES_WINE_REPO
    );
    let release: serde_json::Value = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .map_err(|e| SkseError::Other(format!("GitHub API request failed: {}", e)))?
        .error_for_status()
        .map_err(|e| SkseError::Other(format!("GitHub API error: {}", e)))?
        .json()
        .map_err(|e| SkseError::Other(format!("GitHub API JSON parse error: {}", e)))?;

    let tag = release["tag_name"].as_str().unwrap_or("unknown");

    // Check if already deployed at the latest version
    let version_file = plugins_dir.join(ENGINE_FIXES_WINE_VERSION);
    if plugins_dir.join(ENGINE_FIXES_WINE_DLL).exists() {
        if let Ok(deployed_tag) = fs::read_to_string(&version_file) {
            if deployed_tag.trim() == tag {
                debug!(
                    "SSE Engine Fixes for Wine {} already deployed, skipping",
                    tag
                );
                return Ok(false);
            }
            info!(
                "SSE Engine Fixes for Wine update available: {} -> {}",
                deployed_tag.trim(),
                tag
            );
        } else {
            // DLL exists but no version marker — legacy install, re-deploy to update
            info!(
                "SSE Engine Fixes for Wine deployed without version marker, updating to {}",
                tag
            );
        }
    }

    info!(
        "Downloading SSE Engine Fixes for Wine {} from GitHub (blocking)...",
        tag
    );

    let assets = release["assets"]
        .as_array()
        .ok_or_else(|| SkseError::Other("No assets in release".into()))?;

    let ae_asset = assets
        .iter()
        .find(|a| {
            a["name"]
                .as_str()
                .map(|n| n.contains("AE") && n.ends_with(".zip"))
                .unwrap_or(false)
        })
        .ok_or_else(|| SkseError::Other("No AE zip asset found in release".into()))?;

    let download_url = ae_asset["browser_download_url"]
        .as_str()
        .ok_or_else(|| SkseError::Other("No download URL for asset".into()))?;
    info!(
        "Downloading SSE Engine Fixes for Wine {} from {}",
        tag, download_url
    );

    let bytes = client
        .get(download_url)
        .send()
        .map_err(|e| SkseError::Other(format!("Download failed: {}", e)))?
        .error_for_status()
        .map_err(|e| SkseError::Other(format!("Download failed: {}", e)))?
        .bytes()
        .map_err(|e| SkseError::Other(format!("Download failed: {}", e)))?;

    info!(
        "Downloaded SSE Engine Fixes for Wine ({} bytes), extracting...",
        bytes.len()
    );

    let extract_dir = std::env::temp_dir().join("corkscrew_engine_fixes_wine");
    if extract_dir.exists() {
        fs::remove_dir_all(&extract_dir)?;
    }
    fs::create_dir_all(&extract_dir)?;

    let tmp_zip = extract_dir.join("SSEEngineFixesForWine.zip");
    fs::write(&tmp_zip, &bytes)?;

    installer::extract_archive(&tmp_zip, &extract_dir)
        .map_err(|e| SkseError::Extraction(format!("Failed to extract: {}", e)))?;
    let _ = fs::remove_file(&tmp_zip);

    fs::create_dir_all(&plugins_dir)?;

    let mut found_dll = false;
    let mut found_toml = false;

    for entry in walkdir::WalkDir::new(&extract_dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.eq_ignore_ascii_case(ENGINE_FIXES_WINE_DLL) {
                fs::copy(path, plugins_dir.join(ENGINE_FIXES_WINE_DLL))?;
                found_dll = true;
                info!("Deployed {}", ENGINE_FIXES_WINE_DLL);
                // Clean up old-named DLL from pre-v1.3.0
                let old_dll = plugins_dir.join("SSEEngineFixesForWine.dll");
                if old_dll.exists() {
                    let _ = fs::remove_file(&old_dll);
                    info!("Removed old SSEEngineFixesForWine.dll (renamed to 0_ prefix)");
                }
            } else if name.eq_ignore_ascii_case(ENGINE_FIXES_WINE_TOML) {
                let dest = plugins_dir.join(ENGINE_FIXES_WINE_TOML);
                if !dest.exists() {
                    fs::copy(path, &dest)?;
                    info!("Deployed {} (default config)", ENGINE_FIXES_WINE_TOML);
                }
                found_toml = true;
            }
        }
    }

    if let Err(e) = fs::remove_dir_all(&extract_dir) {
        warn!("Failed to clean up extraction dir: {}", e);
    }

    if !found_dll {
        return Err(SkseError::Other(
            "0_SSEEngineFixesForWine.dll not found in release archive".into(),
        ));
    }

    if !found_toml {
        warn!(
            "SSEEngineFixesForWine.toml not found in release archive; DLL deployed without config"
        );
    }

    // Write version marker for future update checks
    if let Err(e) = fs::write(&version_file, tag) {
        warn!("Failed to write Engine Fixes Wine version marker: {}", e);
    }

    info!("SSE Engine Fixes for Wine {} installed successfully", tag);
    Ok(true)
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
        assert_eq!(skse_game_compatibility("2.1.5"), Some(("1.5.97", "1.5.97")));
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
        assert_eq!(fs::read_to_string(dst.join("file1.txt")).unwrap(), "hello");
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
        fs::write(skse_root.join("skse64_steam_loader.dll"), b"steam loader").unwrap();
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
        assert!(game_dir
            .join("Data")
            .join("Scripts")
            .join("SKSE.pex")
            .exists());
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

    // --- Auto-download tests ---

    #[test]
    fn tag_to_asset_name_works() {
        assert_eq!(
            tag_to_asset_name("v2.2.6"),
            Some("skse64_2_02_06.7z".into())
        );
        assert_eq!(
            tag_to_asset_name("v2.0.20"),
            Some("skse64_2_00_20.7z".into())
        );
        assert_eq!(
            tag_to_asset_name("v2.1.0"),
            Some("skse64_2_01_00.7z".into())
        );
        assert_eq!(tag_to_asset_name("invalid"), None);
    }

    #[test]
    fn parse_game_version_works() {
        assert_eq!(parse_game_version("1.6.1170"), Some((1, 6, 1170)));
        assert_eq!(parse_game_version("1.5.97"), Some((1, 5, 97)));
        assert_eq!(
            parse_game_version("1.5.97 (Special Edition)"),
            Some((1, 5, 97))
        );
        assert_eq!(parse_game_version("invalid"), None);
    }

    #[test]
    fn get_builds_for_ae_latest() {
        let builds = get_available_skse_builds("1.6.1170");
        assert_eq!(builds.edition, "AE");
        let rec = builds.recommended.unwrap();
        assert_eq!(rec.tag, "v2.2.6");
        assert!(rec.is_recommended);
        assert!(rec.download_url.contains("skse64_2_02_06.7z"));
    }

    #[test]
    fn get_builds_for_se() {
        let builds = get_available_skse_builds("1.5.97 (Special Edition)");
        assert_eq!(builds.edition, "SE");
        let rec = builds.recommended.unwrap();
        assert_eq!(rec.tag, "v2.0.20");
        assert_eq!(rec.target_game_version, "1.5.97");
    }

    #[test]
    fn get_builds_for_ae_1130() {
        let builds = get_available_skse_builds("1.6.1130");
        let rec = builds.recommended.unwrap();
        // Should recommend v2.2.5 or v2.2.4 (both target 1.6.1130)
        assert!(rec.tag == "v2.2.5" || rec.tag == "v2.2.4");
    }

    #[test]
    fn get_builds_for_old_ae() {
        let builds = get_available_skse_builds("1.6.640");
        let rec = builds.recommended.unwrap();
        // Should recommend v2.2.3 (newest targeting 1.6.640)
        assert_eq!(rec.tag, "v2.2.3");
    }

    #[test]
    fn get_builds_unknown_version() {
        let builds = get_available_skse_builds("invalid-version");
        assert!(builds.recommended.is_none());
    }

    // --- SKSE plugin version compat tests ---

    #[test]
    fn runtime_version_encoding() {
        // 1.5.97 → (1 << 24) | (5 << 16) | (97 << 4)
        assert_eq!(parse_game_version_to_runtime("1.5.97"), Some(0x01050610));
        // 1.6.1170 → (1 << 24) | (6 << 16) | (1170 << 4)
        assert_eq!(parse_game_version_to_runtime("1.6.1170"), Some(0x01064920));
        // 1.6.640 → (1 << 24) | (6 << 16) | (640 << 4)
        assert_eq!(parse_game_version_to_runtime("1.6.640"), Some(0x01062800));
        // Wildcard AE version (exe hash unknown) — should resolve to 1.6.1170
        assert_eq!(
            parse_game_version_to_runtime("1.6.x (Anniversary Edition, ~35.4 MB)"),
            Some(0x01064920)
        );
        assert_eq!(
            parse_game_version_to_runtime("1.6.x (Anniversary Edition, ~50.1 MB)"),
            Some(0x01064920)
        );
        // Invalid
        assert_eq!(parse_game_version_to_runtime("invalid"), None);
        assert_eq!(parse_game_version_to_runtime("1.2"), None);
    }

    #[test]
    fn runtime_version_roundtrip() {
        let v = parse_game_version_to_runtime("1.6.1170").unwrap();
        assert_eq!(decode_runtime_version(v), "1.6.1170");

        let v = parse_game_version_to_runtime("1.5.97").unwrap();
        assert_eq!(decode_runtime_version(v), "1.5.97");
    }

    #[test]
    fn check_compat_non_pe_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dll = tmp.path().join("fake.dll");
        fs::write(&dll, b"this is not a PE file").unwrap();
        let runtime = parse_game_version_to_runtime("1.6.1170").unwrap();
        let result = check_skse_dll_compat(&dll, runtime);
        // Should return NoVersionData (graceful) since it can't parse as PE
        assert!(matches!(
            result,
            SksePluginCompat::NoVersionData | SksePluginCompat::ParseError(_)
        ));
    }

    #[test]
    fn check_compat_missing_file() {
        let result = check_skse_dll_compat(Path::new("/nonexistent/fake.dll"), 0x01064920);
        assert!(matches!(result, SksePluginCompat::ParseError(_)));
    }

    #[test]
    fn all_version_db_entries_have_valid_assets() {
        for entry in SKSE_VERSION_DB {
            let asset = tag_to_asset_name(entry.tag);
            assert!(
                asset.is_some(),
                "SKSE_VERSION_DB entry {} has invalid tag",
                entry.tag
            );
        }
    }

    #[test]
    fn address_library_plugins_both_version_independent() {
        // Both SE-only and AE Address Library plugins have identical PE metadata:
        // data_version=1, version_independence=0x1, compatible_versions=[]
        // Both should return VersionIndependent from check_skse_dll_compat.
        // The fix_skse_plugin_conflicts() function uses file-size comparison
        // to detect and swap mismatched DLLs on AE.

        let ae_runtime = parse_game_version_to_runtime("1.6.1170").unwrap();

        // If the real BDI DLLs are on disk, verify both return VersionIndependent
        let se_dll = std::path::Path::new(
            "/Users/cashconway/Library/Application Support/CrossOver/Bottles/Steam/drive_c/\
             Program Files (x86)/Steam/steamapps/common/Skyrim Special Edition/Data/SKSE/Plugins/\
             BehaviorDataInjector.dll",
        );
        let ae_dll = std::path::Path::new(
            "/Users/cashconway/Library/Application Support/corkscrew/staging/skyrimse/Steam/\
             28438_Behavior_Data_Injector_Universal_Support/SKSE/Plugins/BehaviorDataInjector.dll",
        );
        if se_dll.exists() {
            let compat = check_skse_dll_compat(se_dll, ae_runtime);
            assert!(
                matches!(compat, SksePluginCompat::VersionIndependent),
                "SE BDI should be VersionIndependent (Address Library), got {:?}",
                compat
            );
        }
        if ae_dll.exists() {
            let compat = check_skse_dll_compat(ae_dll, ae_runtime);
            assert!(
                matches!(compat, SksePluginCompat::VersionIndependent),
                "AE BDI should be VersionIndependent (Address Library), got {:?}",
                compat
            );
            // Log file sizes — may or may not differ depending on mod versions
            if se_dll.exists() {
                let se_size = fs::metadata(se_dll).unwrap().len();
                let ae_size = fs::metadata(ae_dll).unwrap().len();
                if se_size != ae_size {
                    eprintln!(
                        "SE size: {}KB, AE size: {}KB (different — swap detection works)",
                        se_size / 1024,
                        ae_size / 1024
                    );
                } else {
                    eprintln!(
                        "SE size: {}KB, AE size: {}KB (same — swap relies on mod name heuristic)",
                        se_size / 1024,
                        ae_size / 1024
                    );
                }
            }
        }
    }
}
