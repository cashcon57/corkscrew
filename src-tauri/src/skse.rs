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
    let build_str: String = parts[2].chars().take_while(|c| c.is_ascii_digit()).collect();
    let build: u16 = build_str.parse().ok()?;
    Some(((major as u32) << 24) | ((minor as u32) << 16) | ((build as u32) << 4))
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
    let version_data = read_version_data_pe64(&bytes)
        .or_else(|| read_version_data_pe32(&bytes));

    let data = match version_data {
        Some(d) => d,
        None => return SksePluginCompat::NoVersionData,
    };

    // Check version independence flags
    if data.version_independence & VERSION_INDEPENDENT_ADDRESS_LIBRARY != 0
        || data.version_independence & VERSION_INDEPENDENT_SIGNATURES != 0
    {
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
        SksePluginCompat::Incompatible { supports: supported }
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
    if name.is_empty() { None } else { Some(name) }
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
    let se_lib = data_dir.join("SKSE").join("Plugins").join("versionlib-1-5-97-0.bin");
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
                    let plugin_name = get_skse_plugin_name(dll_path)
                        .unwrap_or_else(|| dll_name.clone());
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
        total_plugins, address_library_version, address_library_matches, warnings.len()
    );

    SksePluginScanResult {
        total_plugins,
        address_library_version,
        address_library_matches,
        warnings,
    }
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
        assert!(matches!(result, SksePluginCompat::NoVersionData | SksePluginCompat::ParseError(_)));
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
}
