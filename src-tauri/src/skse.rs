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
pub fn find_file_case_insensitive(dir: &Path, target: &str) -> Option<PathBuf> {
    crate::fs_ci::find_file_ci(dir, target)
}

/// Find a subdirectory by name (case-insensitive) in a directory.
fn find_subdirectory_case_insensitive(dir: &Path, target: &str) -> Option<PathBuf> {
    crate::fs_ci::find_dir_ci(dir, target)
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

/// Stock SSE Engine Fixes files that crash under Wine and are replaced by
/// SSE Engine Fixes for Wine: `(relative_path, deploy_target)`.
///
/// 1. `SKSE/Plugins/EngineFixes.dll` (Data) — the original SKSE plugin; it
///    also force-closes the game when its preloader is missing.
/// 2. `d3dx9_42.dll` (game root) — the preloader, which hooks during DLL load
///    and crashes Wine ~63s into launch.
const STOCK_ENGINE_FIXES_FILES: &[(&str, &str)] = &[
    ("SKSE/Plugins/EngineFixes.dll", "data"),
    ("d3dx9_42.dll", "root"),
];

/// The stock Engine Fixes config. Patched (deployed copy only) while the Wine
/// fork is active; re-linked from staging when it isn't.
const STOCK_ENGINE_FIXES_TOML: (&str, &str) = ("SKSE/Plugins/EngineFixes.toml", "data");

/// Skyrim runtimes the Wine fork's SKSE plugin declares compatibility with.
/// Must match `CompatibleVersions` in SSEEngineFixesForWine `src/main.cpp`
/// (currently `RUNTIME_SSE_1_6_1170` only). SKSE refuses to load the fork on
/// any other runtime — notably 1.7.x, which also rejects the stock plugin.
const ENGINE_FIXES_WINE_SUPPORTED_RUNTIMES: &[&str] = &["1.6.1170"];

/// Whether the Wine fork can load on the given Skyrim runtime version
/// (as reported by [`downgrader::detect_skyrim_version`], e.g. `1.6.1170`).
pub fn engine_fixes_wine_supports_runtime(version: &str) -> bool {
    let version = version.trim();
    ENGINE_FIXES_WINE_SUPPORTED_RUNTIMES.iter().any(|v| {
        version == *v
            || version
                .strip_prefix(v)
                .is_some_and(|rest| rest.starts_with('.') || rest.starts_with(' '))
    })
}

/// Detect the Skyrim runtime version for Engine Fixes gating. `None` when the
/// executable can't be found or classified.
pub fn skyrim_runtime_version(game_path: &Path) -> Option<String> {
    downgrader::detect_skyrim_version(game_path)
        .map(|s| s.current_version)
        .map_err(|e| debug!("Engine Fixes: could not detect Skyrim version: {}", e))
        .ok()
}

/// What [`prepare_engine_fixes`] decided and did.
#[derive(Debug, Default, Clone)]
pub struct EngineFixesPlan {
    /// The caller should now deploy SSE Engine Fixes for Wine
    /// (`install_engine_fixes_wine{,_blocking}`).
    pub install_wine_fork: bool,
    /// Informational: which stock files were withheld or restored this run.
    pub notice: Option<String>,
    /// User-actionable problem (e.g. unsupported game runtime).
    pub warning: Option<String>,
}

/// Apply the "SSE Engine Fixes for Wine" option to the current deployment.
///
/// Called after every deploy and before launch so the deployed state never
/// depends on which code path ran last. Staging is never modified.
///
/// - **Option on + supported runtime (1.6.1170, macOS):** the stock
///   `EngineFixes.dll` and `d3dx9_42.dll` preloader are withheld from the
///   deployment (deployed copies removed, manifest rows dropped; staging
///   intact). The caller then installs the Wine fork.
/// - **Option on + unsupported/unknown runtime:** the fork can't load, so
///   nothing is withheld or installed and a warning is returned.
/// - **Option off (or Linux, where the fork isn't needed):** the fork DLL is
///   removed and the stock files are re-deployed from their mod's staging
///   copy, so turning the option off restores stock behaviour.
///
/// Also undoes the staging mutation of earlier versions, which renamed the
/// stock DLLs to `.dll.disabled` inside every mod's staging directory.
pub fn prepare_engine_fixes(
    db: &Arc<ModDatabase>,
    game_id: &str,
    bottle_name: &str,
    data_dir: &Path,
    game_path: &Path,
    wine_fork_requested: bool,
    game_version: Option<&str>,
) -> EngineFixesPlan {
    let mut plan = EngineFixesPlan::default();
    let mut notices: Vec<String> = Vec::new();

    let healed = restore_legacy_disabled_in_staging(db, game_id, bottle_name);
    if healed > 0 {
        info!(
            "Engine Fixes: restored {} stock file(s) earlier versions renamed inside staging",
            healed
        );
    }

    // The fork targets macOS CrossOver Wine bugs; Proton/native Wine on
    // Linux doesn't need it (install is a no-op there).
    let wine_host = cfg!(not(target_os = "linux"));
    let fork_active = wine_fork_requested
        && wine_host
        && match game_version {
            Some(v) if engine_fixes_wine_supports_runtime(v) => true,
            Some(v) => {
                plan.warning = Some(format!(
                    "SSE Engine Fixes for Wine was not deployed: it supports Skyrim {} only, \
                     and this game is {}. The stock Engine Fixes files are deployed instead.",
                    ENGINE_FIXES_WINE_SUPPORTED_RUNTIMES.join(", "),
                    v
                ));
                false
            }
            None => {
                plan.warning = Some(
                    "SSE Engine Fixes for Wine was not deployed: Corkscrew could not detect the \
                     Skyrim version. The stock Engine Fixes files are deployed instead."
                        .to_string(),
                );
                false
            }
        };
    if let Some(ref w) = plan.warning {
        warn!("Engine Fixes: {}", w);
    }

    if fork_active {
        match crate::deployer::exclude_deployed_paths(
            db,
            game_id,
            bottle_name,
            data_dir,
            game_path,
            STOCK_ENGINE_FIXES_FILES,
        ) {
            Ok(removed) if !removed.is_empty() => notices.push(format!(
                "SSE Engine Fixes for Wine is active, so the stock Engine Fixes files that crash \
                 under Wine were not deployed ({}). Turn the option off in Settings to use them.",
                removed.join(", ")
            )),
            Ok(_) => {}
            Err(e) => warn!("Engine Fixes: could not withhold stock files: {}", e),
        }
        // Files Corkscrew doesn't track (manual installs, pre-manifest
        // deploys) are renamed, recoverably, exactly as before.
        let renamed = disable_untracked_stock_engine_fixes(data_dir, game_path);
        if !renamed.is_empty() {
            notices.push(format!(
                "Renamed untracked stock Engine Fixes file(s) to .disabled: {}",
                renamed.join(", ")
            ));
        }

        // Deployed copy only: patch_engine_fixes_toml writes temp + rename,
        // which replaces the hardlink instead of writing through it.
        let deployed_toml = data_dir.join(STOCK_ENGINE_FIXES_TOML.0);
        if deployed_toml.is_file() {
            match patch_engine_fixes_toml(&deployed_toml) {
                Ok(true) => info!("Engine Fixes: patched {}", deployed_toml.display()),
                Ok(false) => {}
                Err(e) => warn!("Engine Fixes: failed to patch {}: {}", deployed_toml.display(), e),
            }
        }

        // SkyrimSoulsRE doesn't recognise the fork's DLL name and shows a
        // false "Engine Fixes not installed" dialog that blocks startup.
        patch_skyrim_souls_re_ini(data_dir);
        plan.install_wine_fork = true;
    } else {
        if !wine_fork_requested && remove_engine_fixes_wine(db, game_id, bottle_name, data_dir) {
            notices.push(
                "Removed SSE Engine Fixes for Wine (option is off); using the stock Engine Fixes."
                    .to_string(),
            );
        }
        // The fork isn't active, so the stock files must be deployed exactly
        // as their mod ships them (undoing any earlier withholding/patching).
        let mut paths: Vec<(&str, &str)> = STOCK_ENGINE_FIXES_FILES.to_vec();
        paths.push(STOCK_ENGINE_FIXES_TOML);
        match crate::deployer::ensure_paths_deployed(
            db,
            game_id,
            bottle_name,
            data_dir,
            game_path,
            &paths,
        ) {
            Ok(restored) if !restored.is_empty() => notices.push(format!(
                "Restored stock Engine Fixes file(s): {}",
                restored.join(", ")
            )),
            Ok(_) => {}
            Err(e) => warn!("Engine Fixes: could not restore stock files: {}", e),
        }
        let unrenamed = restore_untracked_stock_engine_fixes(data_dir, game_path);
        if !unrenamed.is_empty() {
            notices.push(format!(
                "Re-enabled stock Engine Fixes file(s): {}",
                unrenamed.join(", ")
            ));
        }
    }

    if !notices.is_empty() {
        let notice = notices.join(" ");
        info!("Engine Fixes: {}", notice);
        plan.notice = Some(notice);
    }
    plan
}

/// Blocking post-deploy / pre-launch entry point: read the option, detect the
/// runtime (unless `game_version` is supplied), apply
/// [`prepare_engine_fixes`], and install the Wine fork when it should be
/// active. `context` prefixes log lines ("Redeploy", "Pre-launch", …).
///
/// Returns user-facing messages (warnings first, then notices).
pub fn apply_engine_fixes_blocking(
    db: &Arc<ModDatabase>,
    game_id: &str,
    bottle_name: &str,
    data_dir: &Path,
    game_path: &Path,
    game_version: Option<&str>,
    context: &str,
) -> Vec<String> {
    let requested = config::engine_fixes_wine_enabled();
    let detected;
    let version = match game_version {
        Some(v) => Some(v),
        None if requested => {
            detected = skyrim_runtime_version(game_path);
            detected.as_deref()
        }
        None => None,
    };
    let plan = prepare_engine_fixes(
        db, game_id, bottle_name, data_dir, game_path, requested, version,
    );
    let mut messages: Vec<String> = plan.warning.into_iter().collect();
    if plan.install_wine_fork {
        match install_engine_fixes_wine_blocking(data_dir) {
            Ok(true) => info!("{}: deployed SSE Engine Fixes for Wine", context),
            Ok(false) => debug!("{}: SSE Engine Fixes for Wine already up to date", context),
            Err(e) => {
                warn!("{}: could not deploy SSE Engine Fixes for Wine: {}", context, e);
                messages.push(format!("Could not deploy SSE Engine Fixes for Wine: {}", e));
            }
        }
    } else if !requested {
        debug!("{}: SSE Engine Fixes for Wine is off; stock Engine Fixes in use", context);
    }
    messages.extend(plan.notice);
    messages
}

/// Deployed locations of the stock Engine Fixes files.
fn stock_engine_fixes_deployed_paths(data_dir: &Path, game_path: &Path) -> Vec<PathBuf> {
    STOCK_ENGINE_FIXES_FILES
        .iter()
        .map(|(rel, target)| {
            if *target == "root" {
                game_path.join(rel)
            } else {
                data_dir.join(rel)
            }
        })
        .collect()
}

fn disabled_sibling(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".disabled");
    path.with_file_name(name)
}

/// Rename stock Engine Fixes files that Corkscrew does NOT track (still on
/// disk after the tracked copies were withheld) to `.dll.disabled`.
fn disable_untracked_stock_engine_fixes(data_dir: &Path, game_path: &Path) -> Vec<String> {
    let mut renamed = Vec::new();
    for path in stock_engine_fixes_deployed_paths(data_dir, game_path) {
        match fs::symlink_metadata(&path) {
            Ok(m) if m.file_type().is_file() => {}
            _ => continue,
        }
        let disabled = disabled_sibling(&path);
        match fs::rename(&path, &disabled) {
            Ok(()) => {
                info!("Engine Fixes: disabled untracked {}", path.display());
                renamed.push(path.file_name().unwrap_or_default().to_string_lossy().into_owned());
            }
            Err(e) => warn!("Engine Fixes: failed to disable {}: {}", path.display(), e),
        }
    }
    renamed
}

/// Undo [`disable_untracked_stock_engine_fixes`] (and the deployed-side
/// renames of earlier versions): bring `.dll.disabled` back when nothing is
/// deployed at the original path; drop it when it's a stale duplicate of the
/// file that is now deployed there.
fn restore_untracked_stock_engine_fixes(data_dir: &Path, game_path: &Path) -> Vec<String> {
    let mut restored = Vec::new();
    for path in stock_engine_fixes_deployed_paths(data_dir, game_path) {
        let disabled = disabled_sibling(&path);
        match fs::symlink_metadata(&disabled) {
            Ok(m) if m.file_type().is_file() => {}
            _ => continue,
        }
        match fs::symlink_metadata(&path) {
            Err(_) => match fs::rename(&disabled, &path) {
                Ok(()) => {
                    info!("Engine Fixes: re-enabled {}", path.display());
                    restored
                        .push(path.file_name().unwrap_or_default().to_string_lossy().into_owned());
                }
                Err(e) => warn!("Engine Fixes: failed to re-enable {}: {}", path.display(), e),
            },
            Ok(_) if files_identical(&disabled, &path) => {
                if let Err(e) = fs::remove_file(&disabled) {
                    debug!("Engine Fixes: could not remove stale {}: {}", disabled.display(), e);
                }
            }
            Ok(_) => {}
        }
    }
    restored
}

fn files_identical(a: &Path, b: &Path) -> bool {
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(ma), Ok(mb)) if ma.len() == mb.len() => {
            matches!(
                (crate::platform::fast_hash(a), crate::platform::fast_hash(b)),
                (Ok(ha), Ok(hb)) if ha == hb
            )
        }
        _ => false,
    }
}

/// Earlier versions renamed the stock DLLs to `.dll.disabled` inside every
/// mod's *staging* directory, which made turning the option off unable to
/// restore them. Rename them back. Returns the number restored.
fn restore_legacy_disabled_in_staging(
    db: &Arc<ModDatabase>,
    game_id: &str,
    bottle_name: &str,
) -> usize {
    let Ok(mods) = db.list_mods(game_id, bottle_name) else {
        return 0;
    };
    let mut healed = 0;
    for m in &mods {
        let Some(ref sp) = m.staging_path else {
            continue;
        };
        let sp = PathBuf::from(sp);
        for (rel, _) in STOCK_ENGINE_FIXES_FILES {
            let original = sp.join(rel);
            let disabled = disabled_sibling(&original);
            let is_file = |p: &Path| {
                fs::symlink_metadata(p)
                    .map(|m| m.file_type().is_file())
                    .unwrap_or(false)
            };
            if is_file(&disabled) && fs::symlink_metadata(&original).is_err() {
                match fs::rename(&disabled, &original) {
                    Ok(()) => healed += 1,
                    Err(e) => warn!(
                        "Engine Fixes: could not restore {} in staging: {}",
                        original.display(),
                        e
                    ),
                }
            }
        }
    }
    healed
}

/// Remove the deployed SSE Engine Fixes for Wine plugin (and its version
/// marker; the user's TOML config is kept). Skipped when the user installed
/// the fork as a regular, manifest-tracked mod. Returns true if removed.
fn remove_engine_fixes_wine(
    db: &Arc<ModDatabase>,
    game_id: &str,
    bottle_name: &str,
    data_dir: &Path,
) -> bool {
    let plugins_dir = data_dir.join("SKSE").join("Plugins");
    let dll = plugins_dir.join(ENGINE_FIXES_WINE_DLL);
    if !dll.is_file() {
        return false;
    }
    let tracked = db
        .get_deployment_manifest(game_id, bottle_name)
        .map(|entries| {
            entries.iter().any(|e| {
                e.deploy_target == "data"
                    && e.relative_path
                        .replace('\\', "/")
                        .eq_ignore_ascii_case(&format!("SKSE/Plugins/{}", ENGINE_FIXES_WINE_DLL))
            })
        })
        .unwrap_or(true);
    if tracked {
        debug!("Engine Fixes: {} is a managed mod file; leaving it", ENGINE_FIXES_WINE_DLL);
        return false;
    }
    match fs::remove_file(&dll) {
        Ok(()) => {
            let _ = fs::remove_file(plugins_dir.join(ENGINE_FIXES_WINE_VERSION));
            info!("Engine Fixes: removed {}", dll.display());
            true
        }
        Err(e) => {
            warn!("Engine Fixes: failed to remove {}: {}", dll.display(), e);
            false
        }
    }
}

/// Patch `SkyrimSoulsRE.ini` to set `bHideEngineFixesWarning = true`.
///
/// Our Wine fork uses a different DLL name (`0_SSEEngineFixesForWine.dll`)
/// which SkyrimSoulsRE doesn't recognise, causing a false warning dialog
/// that blocks game startup.  Returns 1 if patched, 0 otherwise.
///
/// Writes temp + rename so a hardlinked deployed copy is replaced rather
/// than written through (which would modify the mod's staging file).
fn patch_skyrim_souls_re_ini(data_dir: &Path) -> usize {
    let ini_path = data_dir
        .join("SKSE")
        .join("Plugins")
        .join("SkyrimSoulsRE.ini");

    if !ini_path.exists() {
        return 0;
    }

    let contents = match fs::read_to_string(&ini_path) {
        Ok(c) => c,
        Err(e) => {
            warn!(
                "SkyrimSoulsRE fix: failed to read {}: {}",
                ini_path.display(),
                e
            );
            return 0;
        }
    };

    let needle = "bHideEngineFixesWarning = false";
    let replacement = "bHideEngineFixesWarning = true";

    if !contents.contains(needle) {
        debug!(
            "SkyrimSoulsRE fix: already patched or key not found in {}",
            ini_path.display()
        );
        return 0;
    }

    let patched = contents.replace(needle, replacement);
    let tmp = ini_path.with_extension("ini.tmp");
    match fs::write(&tmp, patched).and_then(|()| fs::rename(&tmp, &ini_path)) {
        Ok(()) => {
            info!(
                "SkyrimSoulsRE fix: set bHideEngineFixesWarning = true in {}",
                ini_path.display()
            );
            1
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            warn!(
                "SkyrimSoulsRE fix: failed to write {}: {}",
                ini_path.display(),
                e
            );
            0
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
/// On Linux, this is a no-op — SSEEngineFixesForWine addresses macOS-specific
/// Wine bugs that don't exist in Proton/native Linux Wine.
pub async fn install_engine_fixes_wine(data_dir: &Path) -> Result<bool> {
    #[cfg(target_os = "linux")]
    {
        info!("Skipping SSEEngineFixesForWine install on Linux (macOS-only fix)");
        return Ok(false);
    }
    let plugins_dir = data_dir.join("SKSE").join("Plugins");

    // Connect timeout matches the blocking twin (install_engine_fixes_wine_blocking):
    // a stalled GitHub call must not hang a collection install indefinitely.
    let client = reqwest::Client::builder()
        .user_agent(format!("Corkscrew/{}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(10))
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
    #[cfg(target_os = "linux")]
    {
        info!("Skipping SSEEngineFixesForWine install on Linux (macOS-only fix)");
        return Ok(false);
    }

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
mod engine_fixes_policy_tests {
    use super::*;
    use crate::deployer;
    use tempfile::TempDir;

    #[test]
    fn wine_fork_runtime_gate_matches_compatible_versions() {
        assert!(engine_fixes_wine_supports_runtime("1.6.1170"));
        assert!(engine_fixes_wine_supports_runtime("1.6.1170.0"));
        assert!(!engine_fixes_wine_supports_runtime("1.7.104"));
        assert!(!engine_fixes_wine_supports_runtime("1.5.97"));
        assert!(!engine_fixes_wine_supports_runtime("1.6.640"));
        assert!(!engine_fixes_wine_supports_runtime("1.6.11700"));
        assert!(!engine_fixes_wine_supports_runtime("1.6.x (Anniversary Edition, ~37.1 MB)"));
    }

    struct Fixture {
        _tmp: TempDir,
        db: Arc<ModDatabase>,
        data: PathBuf,
        root: PathBuf,
        ef_staging: PathBuf,
        ef_mod: i64,
    }

    /// One "Engine Fixes" mod (SKSE plugin + toml, Data target) and one
    /// "Engine Fixes part 2" mod (preloader, root target), both deployed.
    fn fixture() -> Fixture {
        let tmp = TempDir::new().unwrap();
        let db = Arc::new(ModDatabase::new(&tmp.path().join("t.db")).unwrap());
        let root = tmp.path().join("game");
        let data = root.join("Data");
        fs::create_dir_all(&data).unwrap();

        let mk = |name: &str, files: &[(&str, &[u8])]| -> (i64, PathBuf) {
            let id = db
                .add_mod("skyrimse", "Gaming", None, name, "1", "a.zip", &[])
                .unwrap();
            let staging = tmp.path().join("staging").join(format!("{id}"));
            for (rel, bytes) in files {
                let p = staging.join(rel);
                fs::create_dir_all(p.parent().unwrap()).unwrap();
                fs::write(p, bytes).unwrap();
            }
            db.set_staging_path(id, staging.to_str().unwrap()).unwrap();
            (id, staging)
        };
        let (ef_mod, ef_staging) = mk(
            "Engine Fixes",
            &[
                ("SKSE/Plugins/EngineFixes.dll", b"stock plugin"),
                ("SKSE/Plugins/EngineFixes.toml", b"bMemoryManager = true\n"),
            ],
        );
        let (pre_mod, pre_staging) = mk("Engine Fixes part 2", &[("d3dx9_42.dll", b"preloader")]);

        let files = crate::staging::list_staging_files(&ef_staging).unwrap();
        deployer::deploy_mod(&db, "skyrimse", "Gaming", ef_mod, &ef_staging, &data, &files, "data")
            .unwrap();
        let files = crate::staging::list_staging_files(&pre_staging).unwrap();
        deployer::deploy_mod(&db, "skyrimse", "Gaming", pre_mod, &pre_staging, &root, &files, "root")
            .unwrap();

        Fixture { _tmp: tmp, db, data, root, ef_staging, ef_mod }
    }

    fn recorded(f: &Fixture, rel: &str) -> bool {
        f.db.get_deployment_manifest("skyrimse", "Gaming")
            .unwrap()
            .iter()
            .any(|e| e.relative_path.eq_ignore_ascii_case(rel))
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn option_off_after_on_redeploys_stock_engine_fixes() {
        let f = fixture();
        let plugin = f.data.join("SKSE/Plugins/EngineFixes.dll");

        // On, supported runtime: stock files withheld, staging untouched.
        let plan = prepare_engine_fixes(
            &f.db, "skyrimse", "Gaming", &f.data, &f.root, true, Some("1.6.1170"),
        );
        assert!(plan.install_wine_fork);
        assert!(plan.notice.is_some(), "withholding must be reported");
        assert!(!plugin.exists());
        assert!(!f.root.join("d3dx9_42.dll").exists());
        assert!(!recorded(&f, "SKSE/Plugins/EngineFixes.dll"), "record must be dropped");
        assert!(f.ef_staging.join("SKSE/Plugins/EngineFixes.dll").is_file(), "staging untouched");
        assert!(!f.ef_staging.join("SKSE/Plugins/EngineFixes.dll.disabled").exists());
        assert_eq!(
            fs::read(f.ef_staging.join("SKSE/Plugins/EngineFixes.toml")).unwrap(),
            b"bMemoryManager = true\n",
            "staging TOML must not be patched"
        );

        // Pretend the fork was installed by the caller.
        let fork = f.data.join("SKSE/Plugins").join(ENGINE_FIXES_WINE_DLL);
        fs::write(&fork, b"fork").unwrap();

        // Off: stock files come back from staging, fork removed.
        let plan = prepare_engine_fixes(&f.db, "skyrimse", "Gaming", &f.data, &f.root, false, None);
        assert!(!plan.install_wine_fork);
        assert_eq!(fs::read(&plugin).unwrap(), b"stock plugin");
        assert!(recorded(&f, "SKSE/Plugins/EngineFixes.dll"));
        assert_eq!(fs::read(f.root.join("d3dx9_42.dll")).unwrap(), b"preloader");
        assert!(!fork.exists(), "fork must not load alongside stock Engine Fixes");

        // And it's stable: a second pass changes nothing.
        let plan = prepare_engine_fixes(&f.db, "skyrimse", "Gaming", &f.data, &f.root, false, None);
        assert!(plan.notice.is_none(), "{:?}", plan.notice);

        // A full redeploy keeps the root preloader in the game root.
        deployer::redeploy_all(&f.db, "skyrimse", "Gaming", &f.data, &f.root).unwrap();
        assert!(f.root.join("d3dx9_42.dll").exists());
        assert!(!f.data.join("d3dx9_42.dll").exists());
    }

    #[test]
    fn unsupported_runtime_keeps_stock_files_and_warns() {
        let f = fixture();
        let plan = prepare_engine_fixes(
            &f.db, "skyrimse", "Gaming", &f.data, &f.root, true, Some("1.7.104"),
        );
        assert!(!plan.install_wine_fork);
        let warning = plan.warning.unwrap_or_default();
        if cfg!(not(target_os = "linux")) {
            assert!(warning.contains("1.7.104"), "{warning}");
        }
        assert!(f.data.join("SKSE/Plugins/EngineFixes.dll").exists());
        assert!(f.root.join("d3dx9_42.dll").exists());
    }

    #[test]
    fn legacy_staging_rename_is_undone_and_stock_redeployed() {
        let f = fixture();
        // State left by older versions: deployed copy AND staging copy renamed,
        // manifest row still pointing at EngineFixes.dll.
        let deployed = f.data.join("SKSE/Plugins/EngineFixes.dll");
        let staged = f.ef_staging.join("SKSE/Plugins/EngineFixes.dll");
        fs::rename(&deployed, disabled_sibling(&deployed)).unwrap();
        fs::rename(&staged, disabled_sibling(&staged)).unwrap();
        assert!(recorded(&f, "SKSE/Plugins/EngineFixes.dll"));

        prepare_engine_fixes(&f.db, "skyrimse", "Gaming", &f.data, &f.root, false, None);
        assert!(staged.is_file(), "staging copy restored");
        assert!(!disabled_sibling(&staged).exists());
        assert_eq!(fs::read(&deployed).unwrap(), b"stock plugin");
        assert!(!disabled_sibling(&deployed).exists(), "stale .disabled duplicate removed");
        let entries = f.db.get_deployment_entries_for_mod(f.ef_mod).unwrap();
        assert_eq!(
            entries
                .iter()
                .filter(|e| e.relative_path.eq_ignore_ascii_case("SKSE/Plugins/EngineFixes.dll"))
                .count(),
            1,
            "exactly one manifest row for the plugin"
        );
    }
}

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
