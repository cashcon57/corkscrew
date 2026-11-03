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
    /// Detected SKSE version (e.g. "2.2.6"), read from the SKSE DLL's
    /// version resource.
    pub version: Option<String>,
    /// The Skyrim runtime the installed SKSE was built for (e.g. "1.6.1170"),
    /// from its `skse64_<runtime>.dll` name. SKSE only starts on this exact
    /// SkyrimSE.exe version.
    #[serde(default)]
    pub runtime: Option<String>,
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

/// Where an SKSE build can be downloaded from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SkseSource {
    /// A release asset on GitHub (`ianpatt/skse64`), which Corkscrew can
    /// download automatically.
    GitHub,
    /// Published only on Nexus Mods (mod 30379). Corkscrew never automates
    /// Nexus downloads outside the Premium API, so the user downloads it and
    /// installs it from the archive.
    Nexus,
    /// Tagged in the source repo but with no known public download.
    Unpublished,
}

/// SKSE's Nexus Mods files page (the only download for builds after 2.2.6).
pub const SKSE_NEXUS_FILES_URL: &str =
    "https://www.nexusmods.com/skyrimspecialedition/mods/30379?tab=files";

/// One SKSE build and the single Skyrim runtime it supports.
struct SkseVersionEntry {
    /// SKSE tag (e.g., "v2.2.6")
    tag: &'static str,
    /// The Skyrim runtime this build was made for. SKSE's loader refuses to
    /// start on any other SkyrimSE.exe version.
    game_version: &'static str,
    source: SkseSource,
}

/// Known SKSE releases and the exact game version each targets, newest first.
/// Targets come from `CURRENT_RELEASE_RUNTIME` in each tag's
/// `skse64_common/skse_version.h` (ianpatt/skse64).
const SKSE_VERSION_DB: &[SkseVersionEntry] = &[
    SkseVersionEntry {
        tag: "v2.3.1",
        game_version: "1.7.104",
        source: SkseSource::Nexus,
    },
    SkseVersionEntry {
        tag: "v2.3.0",
        game_version: "1.7.99",
        source: SkseSource::Nexus,
    },
    SkseVersionEntry {
        tag: "v2.2.8",
        game_version: "1.6.1170",
        source: SkseSource::Unpublished,
    },
    SkseVersionEntry {
        tag: "v2.2.7",
        game_version: "1.6.1170",
        source: SkseSource::Unpublished,
    },
    SkseVersionEntry {
        tag: "v2.2.6",
        game_version: "1.6.1170",
        source: SkseSource::GitHub,
    },
    SkseVersionEntry {
        tag: "v2.2.5",
        game_version: "1.6.1130",
        source: SkseSource::GitHub,
    },
    SkseVersionEntry {
        tag: "v2.2.4",
        game_version: "1.6.1130",
        source: SkseSource::GitHub,
    },
    SkseVersionEntry {
        tag: "v2.2.3",
        game_version: "1.6.640",
        source: SkseSource::GitHub,
    },
    SkseVersionEntry {
        tag: "v2.2.2",
        game_version: "1.6.640",
        source: SkseSource::GitHub,
    },
    SkseVersionEntry {
        tag: "v2.2.1",
        game_version: "1.6.640",
        source: SkseSource::GitHub,
    },
    SkseVersionEntry {
        tag: "v2.2.0",
        game_version: "1.6.629",
        source: SkseSource::GitHub,
    },
    SkseVersionEntry {
        tag: "v2.1.5",
        game_version: "1.6.353",
        source: SkseSource::GitHub,
    },
    SkseVersionEntry {
        tag: "v2.1.4",
        game_version: "1.6.342",
        source: SkseSource::GitHub,
    },
    SkseVersionEntry {
        tag: "v2.1.3",
        game_version: "1.6.323",
        source: SkseSource::GitHub,
    },
    SkseVersionEntry {
        tag: "v2.1.2",
        game_version: "1.6.318",
        source: SkseSource::GitHub,
    },
    SkseVersionEntry {
        tag: "v2.1.1",
        game_version: "1.6.318",
        source: SkseSource::GitHub,
    },
    SkseVersionEntry {
        tag: "v2.1.0",
        game_version: "1.6.317",
        source: SkseSource::GitHub,
    },
    SkseVersionEntry {
        tag: "v2.0.20",
        game_version: "1.5.97",
        source: SkseSource::GitHub,
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
    /// True when Corkscrew can download and install this build itself
    /// (GitHub release). False for Nexus-only builds, whose `download_url`
    /// is the Nexus files page.
    pub auto_install: bool,
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Detect whether SKSE is installed in the given game directory.
///
/// Looks for `skse64_loader.exe` (case-insensitive) and the runtime DLL
/// `skse64_<X>_<Y>_<Z>.dll`, whose name gives the game version it supports
/// and whose version resource gives the SKSE version.
pub fn detect_skse(game_path: &Path) -> SkseStatus {
    let loader = find_file_case_insensitive(game_path, SKSE_LOADER);
    let runtime_dll = find_skse_runtime_dll(game_path);

    SkseStatus {
        installed: loader.is_some(),
        loader_path: loader.map(|p| p.to_string_lossy().into_owned()),
        version: runtime_dll.as_ref().and_then(|(path, _)| skse_dll_version(path)),
        runtime: runtime_dll.map(|(_, runtime)| runtime),
        use_skse: false, // Caller should overlay the preference from config.
    }
}

/// Find the SKSE runtime DLL (`skse64_1_6_1170.dll`) and the game version
/// its name encodes ("1.6.1170").
fn find_skse_runtime_dll(game_path: &Path) -> Option<(PathBuf, String)> {
    let entries = fs::read_dir(game_path).ok()?;
    for entry in entries.flatten() {
        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_lowercase();
        let Some(stem) = name
            .strip_prefix("skse64_")
            .and_then(|n| n.strip_suffix(".dll"))
        else {
            continue;
        };
        let parts: Vec<&str> = stem.split('_').collect();
        if parts.len() == 3 && parts.iter().all(|p| p.parse::<u32>().is_ok()) {
            let runtime = parts.join(".");
            debug!("Found SKSE runtime DLL {} (game {})", name, runtime);
            return Some((entry.path(), runtime));
        }
    }
    None
}

/// SKSE version from a DLL's version resource. SKSE stamps FileVersion as
/// `0.<major>.<minor>.<beta>` (SKSE 2.2.6 → 0.2.2.6).
fn skse_dll_version(dll: &Path) -> Option<String> {
    use pelite::pe64::{Pe, PeFile};
    let bytes = fs::read(dll).ok()?;
    let pe = PeFile::from_bytes(&bytes).ok()?;
    let fixed = pe.resources().ok()?.version_info().ok()?.fixed()?;
    let v = fixed.dwFileVersion;
    if v.Major == 0 {
        Some(format!("{}.{}.{}", v.Minor, v.Patch, v.Build))
    } else {
        Some(format!("{}.{}.{}", v.Major, v.Minor, v.Patch))
    }
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

    // If a Steam Play / GameHub launch put the loader in the launcher's
    // place, put the real launcher back first.
    if let Some(swap) = crate::launcher_swap::for_game("skyrimse") {
        if let Err(e) = crate::launcher_swap::restore(game_path, swap) {
            warn!("Couldn't restore {}: {}", swap.launcher, e);
        }
    }

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
/// SKSE supports exactly one game runtime per build, so the recommended build
/// is the newest one whose target equals the game version. Older builds are
/// listed but never recommended for a different runtime.
pub fn get_available_skse_builds(game_version: &str) -> SkseAvailableBuilds {
    let user_ver = parse_game_version(game_version);
    let edition = match user_ver {
        Some((1, 5, _)) => "SE",
        Some((1, minor, _)) if minor >= 6 => "AE",
        _ => "Unknown",
    };

    let mut all_builds: Vec<SkseBuild> = Vec::new();
    let mut recommended: Option<SkseBuild> = None;

    for entry in SKSE_VERSION_DB {
        if entry.source == SkseSource::Unpublished {
            continue;
        }
        let Some(filename) = tag_to_asset_name(entry.tag) else {
            continue;
        };
        let (download_url, auto_install) = match entry.source {
            SkseSource::GitHub => (
                format!(
                    "https://github.com/{}/releases/download/{}/{}",
                    SKSE_GITHUB_REPO, entry.tag, filename
                ),
                true,
            ),
            _ => (SKSE_NEXUS_FILES_URL.to_string(), false),
        };

        let exact = user_ver.is_some() && parse_game_version(entry.game_version) == user_ver;
        let build = SkseBuild {
            tag: entry.tag.to_string(),
            version: entry.tag.strip_prefix('v').unwrap_or(entry.tag).to_string(),
            target_game_version: entry.game_version.to_string(),
            download_url,
            filename,
            is_recommended: exact && recommended.is_none(),
            auto_install,
        };
        if build.is_recommended {
            recommended = Some(build.clone());
        }
        all_builds.push(build);
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
            "Corkscrew doesn't know an SKSE build for Skyrim {game_version}. \
             SKSE only runs on the exact game version it was built for; check {SKSE_URL} \
             for a matching build, then install it from the archive."
        ))
    })?;
    if !build.auto_install {
        return Err(SkseError::Other(format!(
            "SKSE {} (for Skyrim {}) is only published on Nexus Mods. Download it from {} \
             and use \"Install from archive\".",
            build.version, game_version, build.download_url
        )));
    }

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
        "SKSE auto-install complete: installed={}, version={:?}, runtime={:?}",
        status.installed, status.version, status.runtime
    );
    if status.runtime.as_deref().and_then(parse_game_version) != parse_game_version(game_version) {
        return Err(SkseError::Other(format!(
            "Installed SKSE {} is for Skyrim {}, not your game version {}.",
            status.version.as_deref().unwrap_or("(unknown)"),
            status.runtime.as_deref().unwrap_or("(unknown)"),
            game_version
        )));
    }

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

/// The Skyrim game version a given SKSE version supports.
///
/// Returns `Some((game_version, game_version))` — each SKSE build supports a
/// single runtime; the pair is kept for the existing min/max callers.
pub fn skse_game_compatibility(skse_version: &str) -> Option<(&'static str, &'static str)> {
    let wanted = parse_game_version(skse_version)?;
    SKSE_VERSION_DB
        .iter()
        .find(|e| parse_game_version(e.tag.trim_start_matches('v')) == Some(wanted))
        .map(|e| (e.game_version, e.game_version))
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

    // Case 2: SKSE is installed but its runtime DLL wasn't found.
    let Some(runtime) = skse_status.runtime.clone() else {
        return SkseCompatibility {
            compatible: true,
            skse_version: skse_status.version.clone(),
            game_version,
            expected_game_versions: None,
            message: "SKSE is installed but its runtime DLL (skse64_<version>.dll) wasn't found. Verify manually that it matches your game version.".into(),
            severity: "warning".into(),
        };
    };

    // Case 3: SKSE only starts on the exact game version it was built for.
    let skse_label = skse_status
        .version
        .as_deref()
        .map(|v| format!("SKSE {v}"))
        .unwrap_or_else(|| "SKSE".to_string());
    let expected = Some((runtime.clone(), runtime.clone()));
    let game = parse_game_version(&game_version);
    if game.is_some() && game == parse_game_version(&runtime) {
        return SkseCompatibility {
            compatible: true,
            skse_version: skse_status.version.clone(),
            message: format!("{skse_label} matches your game version ({runtime})."),
            game_version,
            expected_game_versions: expected,
            severity: "ok".into(),
        };
    }

    let fix = match get_available_skse_builds(&game_version).recommended {
        Some(b) if b.auto_install => format!(" Install SKSE {} instead.", b.version),
        Some(b) => format!(
            " Install SKSE {} instead (download it from {}).",
            b.version, b.download_url
        ),
        None => format!(" Check {SKSE_URL} for a build that supports your game version."),
    };
    SkseCompatibility {
        compatible: false,
        skse_version: skse_status.version.clone(),
        message: format!(
            "{skse_label} is built for Skyrim {runtime}, but your game is {game_version}. \
             SKSE won't start on a different game version.{fix}"
        ),
        game_version,
        expected_game_versions: expected,
        severity: "error".into(),
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

/// `versionIndependence` flags from SKSE64 PluginAPI.h.
const VERSION_INDEPENDENT_ADDRESS_LIBRARY: u32 = 1 << 0;
const VERSION_INDEPENDENT_SIGNATURES: u32 = 1 << 1;
const VERSION_INDEPENDENT_STRUCTS_POST_629: u32 = 1 << 2;
const KNOWN_VERSION_INDEPENDENT: u32 = VERSION_INDEPENDENT_ADDRESS_LIBRARY
    | VERSION_INDEPENDENT_SIGNATURES
    | VERSION_INDEPENDENT_STRUCTS_POST_629;

/// `versionIndependenceEx` flags from SKSE64 PluginAPI.h.
const VERSION_INDEPENDENT_EX_NO_STRUCT_USE: u32 = 1 << 0;
const VERSION_INDEPENDENT_EX_ADDRESS_LIBRARY_V5: u32 = 1 << 1;

/// `RUNTIME_VERSION_*` values from skse64_common/skse_version.h.
const RUNTIME_VERSION_1_6_317: u32 = 0x010613D0;
const RUNTIME_VERSION_1_6_629: u32 = 0x01062750;
const RUNTIME_VERSION_1_7_99: u32 = 0x01070630;

/// PE build-time window (Unix seconds) that SKSE 2.3.x treats as "built
/// against the pre-1.7.99 Address Library encoding": from the founding of
/// Bethesda to 2025-05-26 GMT (see commonlib-shared 804f07c).
const OLD_ADDRESS_LIBRARY_BUILD_TIME: std::ops::Range<u32> = 520_128_000..1_748_217_600;

/// SKSE's own rejection messages, verbatim from PluginManager.cpp so the scan
/// matches what the user later sees in skse64.log.
const SKSE_MSG_BAD_VERSION_DATA: &str = "disabled, bad version data";
const SKSE_MSG_NO_NAME: &str = "disabled, no name specified";
const SKSE_MSG_UNKNOWN_INDEPENDENCE: &str = "disabled, unsupported version independence method";
const SKSE_MSG_INCOMPATIBLE: &str = "disabled, incompatible with current version of the game";
const SKSE_MSG_ADDRESS_LIBRARY: &str = "disabled, address library needs to be updated";
const SKSE_MSG_RECOMPILE: &str = "must be recompiled for new address library";
const SKSE_MSG_PRE_629_ONLY: &str = "disabled, only compatible with versions earlier than 1.6.629";
const SKSE_MSG_NEWER_SKSE: &str = "disabled, requires newer script extender";

/// Result of checking an SKSE plugin DLL's compatibility with a game version.
#[derive(Clone, Debug)]
pub enum SksePluginCompat {
    /// DLL explicitly lists this game version as compatible.
    Compatible,
    /// DLL does not list this game version. `supports` has human-readable versions it does support.
    Incompatible { supports: Vec<String> },
    /// SKSE will refuse to load this DLL for another reason (`reason` is
    /// SKSE's own log message, e.g. "must be recompiled for new address
    /// library"). `supports` lists any versions the DLL does declare.
    Rejected { reason: String, supports: Vec<String> },
    /// DLL uses Address Library or signature scanning — works with any game version.
    VersionIndependent,
    /// DLL has no `SKSEPlugin_Version` export (legacy plugin).
    NoVersionData,
    /// Could not parse the DLL.
    ParseError(String),
}

/// The fields of `SKSEPluginVersionData` that SKSE's load check reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PluginVersionFields {
    pub data_version: u32,
    pub has_name: bool,
    pub version_independence_ex: u32,
    pub version_independence: u32,
    /// `compatibleVersions` up to (not including) its zero terminator.
    pub compatible_versions: Vec<u32>,
    pub se_version_required: u32,
}

impl From<&SKSEPluginVersionData> for PluginVersionFields {
    fn from(d: &SKSEPluginVersionData) -> Self {
        Self {
            data_version: d.data_version,
            has_name: d.name[0] != 0,
            version_independence_ex: d.version_independence_ex,
            version_independence: d.version_independence,
            compatible_versions: d
                .compatible_versions
                .iter()
                .copied()
                .take_while(|&v| v != 0)
                .collect(),
            se_version_required: d.se_version_required,
        }
    }
}

/// Outcome of SKSE's plugin load check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PluginLoadVerdict {
    /// SKSE loads the plugin, either through a version-independence claim or
    /// because `compatibleVersions` lists the runtime.
    Loads { version_independent: bool },
    /// SKSE refuses the plugin with this log message.
    Rejected(&'static str),
}

/// Port of `PluginManager::CheckPluginCompatibility` (ianpatt/skse64 v2.3.1,
/// skse64/PluginManager.cpp) for AE runtimes (1.6.317+), where SKSE loads
/// plugins through `SKSEPlugin_Version`.
///
/// * `build_time` is the DLL's PE `FileHeader.TimeDateStamp`.
/// * `address_library_present`: `Data/SKSE/Plugins/versionlib-M-m-b-0.bin`
///   exists for exactly this runtime.
/// * `skse_packed`: installed SKSE as `PACKED_SKSE_VERSION`; `None` skips the
///   `seVersionRequired` check.
///
/// The 1.7.99 Address Library re-encoding check only exists in SKSE 2.3.x,
/// which only supports 1.7.x runtimes, so it is applied for runtime >= 1.7.99.
pub(crate) fn skse_plugin_load_verdict(
    v: &PluginVersionFields,
    build_time: u32,
    runtime: u32,
    address_library_present: bool,
    skse_packed: Option<u32>,
) -> PluginLoadVerdict {
    use PluginLoadVerdict::{Loads, Rejected};

    if v.data_version == 0 {
        return Rejected(SKSE_MSG_BAD_VERSION_DATA);
    }
    if !v.has_name {
        return Rejected(SKSE_MSG_NO_NAME);
    }
    // Unknown flags: a future breaking API change in the runtime.
    if v.version_independence & !KNOWN_VERSION_INDEPENDENT != 0 {
        return Rejected(SKSE_MSG_UNKNOWN_INDEPENDENCE);
    }

    let mut version_independent = v.version_independence
        & (VERSION_INDEPENDENT_ADDRESS_LIBRARY | VERSION_INDEPENDENT_SIGNATURES)
        != 0;
    let mut error = SKSE_MSG_INCOMPATIBLE;

    if v.version_independence & VERSION_INDEPENDENT_ADDRESS_LIBRARY != 0 {
        if !address_library_present {
            return Rejected(SKSE_MSG_ADDRESS_LIBRARY);
        }
        // The Address Library changed encoding incompatibly in 1.7.99+.
        if runtime >= RUNTIME_VERSION_1_7_99
            && v.version_independence_ex & VERSION_INDEPENDENT_EX_ADDRESS_LIBRARY_V5 == 0
            && OLD_ADDRESS_LIBRARY_BUILD_TIME.contains(&build_time)
        {
            error = SKSE_MSG_RECOMPILE;
            version_independent = false;
        }
    }

    // 1.6.629+ changed structure sizes; the plugin must say which it uses.
    if version_independent
        && runtime >= RUNTIME_VERSION_1_6_629
        && v.version_independence & VERSION_INDEPENDENT_STRUCTS_POST_629 == 0
        && v.version_independence_ex & VERSION_INDEPENDENT_EX_NO_STRUCT_USE == 0
    {
        return Rejected(SKSE_MSG_PRE_629_ONLY);
    }

    if !version_independent && !v.compatible_versions.contains(&runtime) {
        return Rejected(error);
    }

    if let Some(skse) = skse_packed {
        if v.se_version_required > skse {
            return Rejected(SKSE_MSG_NEWER_SKSE);
        }
    }

    Loads {
        version_independent,
    }
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

/// Encode an SKSE64 release version ("2.3.1") as SKSE's `PACKED_SKSE_VERSION`.
/// Returns `None` for anything that isn't a 2.x SKSE64 version (e.g. a game
/// runtime string such as "1.7.104").
fn parse_skse_version_packed(version: &str) -> Option<u32> {
    let mut parts = version.trim().split('.').map(|p| p.parse::<u32>().ok());
    let major = parts.next()??;
    let minor = parts.next()??;
    let beta = parts.next().unwrap_or(Some(0))?;
    if major != 2 || minor > 0xFF || beta > 0xFFF {
        return None;
    }
    Some((major << 24) | (minor << 16) | (beta << 4))
}

/// Decode SKSE runtime version u32 back to human-readable string.
fn decode_runtime_version(v: u32) -> String {
    let major = (v >> 24) & 0xFF;
    let minor = (v >> 16) & 0xFF;
    let build = (v >> 4) & 0xFFF;
    format!("{}.{}.{}", major, minor, build)
}

/// Address Library database filename SKSE (or Address Library itself) loads
/// for a runtime: `version-1-5-97-0.bin` on SE, `versionlib-M-m-b-0.bin` on AE.
fn address_library_filenames(runtime: u32) -> Vec<String> {
    let (major, minor, build) = ((runtime >> 24) & 0xFF, (runtime >> 16) & 0xFF, (runtime >> 4) & 0xFFF);
    let ae = format!("versionlib-{major}-{minor}-{build}-0.bin");
    if runtime < RUNTIME_VERSION_1_6_317 {
        vec![format!("version-{major}-{minor}-{build}-0.bin"), ae]
    } else {
        vec![ae]
    }
}

/// Check an SKSE plugin DLL's compatibility with a specific game runtime version.
///
/// Parses the PE export table to find the `SKSEPlugin_Version` data export,
/// reads the `SKSEPluginVersionData` struct, and runs SKSE's own load check
/// (see [`skse_plugin_load_verdict`]). Assumes the Address Library for the
/// runtime is present and skips the SKSE-version requirement; use
/// [`scan_skse_plugins`] for the full check.
pub fn check_skse_dll_compat(dll_path: &Path, game_runtime_version: u32) -> SksePluginCompat {
    check_skse_dll_compat_with(dll_path, game_runtime_version, true, None)
}

fn check_skse_dll_compat_with(
    dll_path: &Path,
    runtime: u32,
    address_library_present: bool,
    skse_packed: Option<u32>,
) -> SksePluginCompat {
    let bytes = match std::fs::read(dll_path) {
        Ok(b) => b,
        Err(e) => return SksePluginCompat::ParseError(format!("read error: {}", e)),
    };
    match read_plugin_binary_info(&bytes) {
        Some(info) => classify_plugin(&info, runtime, address_library_present, skse_packed),
        None => SksePluginCompat::NoVersionData,
    }
}

/// Map SKSE's verdict onto [`SksePluginCompat`].
fn classify_plugin(
    info: &PluginBinaryInfo,
    runtime: u32,
    address_library_present: bool,
    skse_packed: Option<u32>,
) -> SksePluginCompat {
    let fields = &info.fields;
    let supports: Vec<String> = fields
        .compatible_versions
        .iter()
        .map(|&v| decode_runtime_version(v))
        .collect();

    // SE (1.5.x) SKSE loads plugins through SKSEPlugin_Query and never runs
    // the AE version check; keep the simple flag/list reading there.
    if runtime < RUNTIME_VERSION_1_6_317 {
        if fields.version_independence
            & (VERSION_INDEPENDENT_SIGNATURES | VERSION_INDEPENDENT_ADDRESS_LIBRARY)
            != 0
        {
            return SksePluginCompat::VersionIndependent;
        }
        if fields.compatible_versions.contains(&runtime) {
            return SksePluginCompat::Compatible;
        }
        return if supports.is_empty() {
            SksePluginCompat::NoVersionData
        } else {
            SksePluginCompat::Incompatible { supports }
        };
    }

    match skse_plugin_load_verdict(
        fields,
        info.build_time,
        runtime,
        address_library_present,
        skse_packed,
    ) {
        PluginLoadVerdict::Loads {
            version_independent: true,
        } => SksePluginCompat::VersionIndependent,
        PluginLoadVerdict::Loads {
            version_independent: false,
        } => SksePluginCompat::Compatible,
        PluginLoadVerdict::Rejected(msg) if msg == SKSE_MSG_INCOMPATIBLE && !supports.is_empty() => {
            SksePluginCompat::Incompatible { supports }
        }
        PluginLoadVerdict::Rejected(msg) => SksePluginCompat::Rejected {
            reason: msg.to_string(),
            supports,
        },
    }
}

/// Extract the plugin name from an SKSE plugin DLL's version data.
pub fn get_skse_plugin_name(dll_path: &Path) -> Option<String> {
    let bytes = std::fs::read(dll_path).ok()?;
    read_plugin_binary_info(&bytes).and_then(|info| info.name)
}

/// What the scan needs from a plugin DLL.
struct PluginBinaryInfo {
    fields: PluginVersionFields,
    name: Option<String>,
    /// PE `FileHeader.TimeDateStamp` (what SKSE's `GetBuildTime` reads).
    build_time: u32,
}

fn read_plugin_binary_info(bytes: &[u8]) -> Option<PluginBinaryInfo> {
    let (data, build_time) = read_version_data_pe64(bytes).or_else(|| read_version_data_pe32(bytes))?;
    let name_len = data.name.iter().position(|&b| b == 0).unwrap_or(data.name.len());
    let name = String::from_utf8_lossy(&data.name[..name_len]).trim().to_string();
    Some(PluginBinaryInfo {
        fields: PluginVersionFields::from(&data),
        name: (!name.is_empty()).then_some(name),
        build_time,
    })
}

/// Try to read SKSEPluginVersionData and the PE build time from a PE64 DLL.
fn read_version_data_pe64(bytes: &[u8]) -> Option<(SKSEPluginVersionData, u32)> {
    use pelite::pe64::{Pe, PeFile};
    let pe = PeFile::from_bytes(bytes).ok()?;
    let exports = pe.exports().ok()?.by().ok()?;
    let export = exports.name("SKSEPlugin_Version").ok()?;
    let rva = export.symbol()?;
    let data: &SKSEPluginVersionData = pe.derva(rva).ok()?;
    if data.data_version > 2 {
        return None;
    }
    Some((*data, pe.file_header().TimeDateStamp))
}

/// Try to read SKSEPluginVersionData and the PE build time from a PE32 DLL.
fn read_version_data_pe32(bytes: &[u8]) -> Option<(SKSEPluginVersionData, u32)> {
    use pelite::pe32::{Pe, PeFile};
    let pe = PeFile::from_bytes(bytes).ok()?;
    let exports = pe.exports().ok()?.by().ok()?;
    let export = exports.name("SKSEPlugin_Version").ok()?;
    let rva = export.symbol()?;
    let data: &SKSEPluginVersionData = pe.derva(rva).ok()?;
    if data.data_version > 2 {
        return None;
    }
    Some((*data, pe.file_header().TimeDateStamp))
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
/// - The Address Library database for exactly this runtime
///   (`versionlib-M-m-b-0.bin`, or `version-1-5-97-0.bin` on SE)
/// - Each DLL's PE-exported `SKSEPlugin_Version` data, run through SKSE's own
///   load check (see [`skse_plugin_load_verdict`])
///
/// `skse_version` is the installed SKSE64 release ("2.3.1"); when unknown the
/// plugins' `seVersionRequired` is not checked.
pub fn scan_skse_plugins(
    data_dir: &Path,
    game_version: &str,
    skse_version: Option<&str>,
) -> SksePluginScanResult {
    let plugins_dir = data_dir.join("SKSE").join("Plugins");
    let mut warnings = Vec::new();
    let mut dll_paths: Vec<PathBuf> = Vec::new();
    let mut bin_names: Vec<String> = Vec::new();

    if let Ok(entries) = fs::read_dir(&plugins_dir) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name.ends_with(".dll") {
                dll_paths.push(entry.path());
            } else if name.ends_with(".bin")
                && (name.starts_with("versionlib-") || name.starts_with("version-"))
            {
                bin_names.push(name);
            }
        }
    }
    dll_paths.sort();
    let total_plugins = dll_paths.len();

    let has_se_lib = bin_names
        .iter()
        .any(|n| n == "version-1-5-97-0.bin" || n == "versionlib-1-5-97-0.bin");
    let has_ae_lib = bin_names
        .iter()
        .any(|n| n.starts_with("versionlib-1-6-") || n.starts_with("versionlib-1-7-"));
    let address_library_version = match (has_se_lib, has_ae_lib) {
        (true, true) => "both",
        (true, false) => "SE",
        (false, true) => "AE",
        (false, false) => "none",
    }
    .to_string();

    let runtime = parse_game_version_to_runtime(game_version);
    let skse_packed = skse_version.and_then(parse_skse_version_packed);

    // SKSE opens the database for exactly the running game version.
    let expected_libs = runtime.map(address_library_filenames).unwrap_or_default();
    let has_exact_lib = expected_libs.iter().any(|f| bin_names.contains(f));

    let mut address_library_users: Vec<String> = Vec::new();
    if let Some(runtime) = runtime {
        for dll_path in &dll_paths {
            let dll_name = dll_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let bytes = match fs::read(dll_path) {
                Ok(b) => b,
                Err(e) => {
                    debug!("Could not read SKSE plugin '{}': {}", dll_name, e);
                    continue;
                }
            };
            let Some(info) = read_plugin_binary_info(&bytes) else {
                continue; // No SKSEPlugin_Version export (legacy / helper DLL)
            };
            let plugin_name = info.name.clone().unwrap_or_else(|| dll_name.clone());
            if info.fields.version_independence & VERSION_INDEPENDENT_ADDRESS_LIBRARY != 0 {
                address_library_users.push(plugin_name.clone());
            }

            // A missing Address Library is reported once below, not per plugin.
            match classify_plugin(&info, runtime, true, skse_packed) {
                SksePluginCompat::Incompatible { supports } => {
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
                SksePluginCompat::Rejected { reason, .. } => {
                    let hint = if reason == SKSE_MSG_RECOMPILE {
                        format!(
                            " — this build predates the Skyrim {} Address Library; install an updated version of the mod",
                            game_version
                        )
                    } else {
                        String::new()
                    };
                    warnings.push(SksePluginWarning {
                        dll_name: dll_name.clone(),
                        warning: format!(
                            "{}: SKSE will not load it ({}){}",
                            plugin_name, reason, hint
                        ),
                    });
                }
                _ => {}
            }
        }
    }

    let address_library_matches = match runtime {
        Some(_) => has_exact_lib || (address_library_users.is_empty() && bin_names.is_empty()),
        // Unknown game version: fall back to the SE/AE family comparison.
        None => {
            let is_se = game_version.starts_with("1.5");
            (!has_se_lib && !has_ae_lib) || if is_se { has_se_lib } else { has_ae_lib }
        }
    };

    if !address_library_matches {
        let expected = expected_libs
            .first()
            .cloned()
            .unwrap_or_else(|| "a matching versionlib-*.bin".to_string());
        let found = if bin_names.is_empty() {
            "none installed".to_string()
        } else {
            format!("found {}", bin_names.join(", "))
        };
        let affected = if address_library_users.is_empty() {
            String::new()
        } else {
            format!(
                " SKSE will disable {} Address Library plugin(s): {}.",
                address_library_users.len(),
                address_library_users.join(", ")
            )
        };
        warnings.insert(
            0,
            SksePluginWarning {
                dll_name: "Address Library".to_string(),
                warning: format!(
                    "Address Library for Skyrim {} is missing (needs SKSE/Plugins/{}; {}).{}",
                    game_version, expected, found, affected
                ),
            },
        );
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
            // Explicit incompatibility from compatible_versions check, or
            // SKSE's load check rejects it (e.g. pre-1.7.99 Address Library build)
            SksePluginCompat::Incompatible { .. } | SksePluginCompat::Rejected { .. } => true,
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
            SksePluginCompat::Rejected { reason, .. } => reason.clone(),
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
// Retired SSE Engine Fixes for Wine: migration back to stock Engine Fixes
// ---------------------------------------------------------------------------
//
// Corkscrew used to deploy a Wine fork of SSE Engine Fixes
// (cashcon57/SSEEngineFixesForWine), because the stock d3dx9_42.dll
// preloader crashed under Wine. Depending on the version, Corkscrew
// downloaded `0_SSEEngineFixesForWine.dll` into `Data/SKSE/Plugins`, renamed
// the stock `EngineFixes.dll` / `d3dx9_42.dll` to `.disabled` (in Data, the
// game root and every mod's staging folder) or withheld them from the
// deployment, and switched every patch in `EngineFixes.toml` off.
//
// The fork was retired on 2026-11-03: stock Engine Fixes 7.0.21+ (which uses
// SKSE's preloader on Skyrim 1.7+) loads under Wine/CrossOver/NotProton with
// every patch, including the TBB memory manager, and the fork only ever
// supported Skyrim 1.6.1170. [`migrate_from_engine_fixes_wine`] undoes all of
// the above after every deploy and before launch; on an install the fork
// never touched it finds nothing to do and changes nothing.

/// Stock SSE Engine Fixes files that older versions renamed or withheld:
/// `(relative_path, deploy_target)`.
const STOCK_ENGINE_FIXES_FILES: &[(&str, &str)] = &[
    ("SKSE/Plugins/EngineFixes.dll", "data"),
    ("d3dx9_42.dll", "root"),
];

/// The stock Engine Fixes config, which older versions patched.
const STOCK_ENGINE_FIXES_TOML: (&str, &str) = ("SKSE/Plugins/EngineFixes.toml", "data");

/// Files Corkscrew's fork installer put in `Data/SKSE/Plugins`: the plugin,
/// its pre-1.3.0 name, its default config and Corkscrew's version marker.
/// The fork wrote its logs to the SKSE log folder, not Data.
const LEGACY_WINE_FORK_FILES: &[&str] = &[
    "0_SSEEngineFixesForWine.dll",
    "SSEEngineFixesForWine.dll",
    "SSEEngineFixesForWine.toml",
    "SSEEngineFixesForWine.version",
];

/// Settings that are on in the stock `EngineFixes.toml` and that the old Wine
/// patch always switched off (alongside setting `bDisableTBB = true`). All of
/// them off together with TBB disabled identifies a config Corkscrew patched;
/// nobody tunes Engine Fixes that way by hand.
const LEGACY_WINE_TOML_SIGNATURE: &[&str] = &[
    "bOverrideMemoryManager",
    "bOverrideScrapHeap",
    "bReplaceImports",
    "bFormCaching",
    "bMaxStdIO",
    "bSafeExit",
    "bMemoryAccessErrors",
];

/// What [`migrate_from_engine_fixes_wine`] found and did.
#[derive(Debug, Default, Clone)]
pub struct EngineFixesMigration {
    /// Something the user has to fix (e.g. reinstall Engine Fixes).
    pub warnings: Vec<String>,
    /// Informational: what was removed or restored this run.
    pub notices: Vec<String>,
}

impl EngineFixesMigration {
    /// User-facing messages, warnings first.
    pub fn into_messages(self) -> Vec<String> {
        let mut messages = self.warnings;
        messages.extend(self.notices);
        messages
    }
}

/// Undo everything the retired SSE Engine Fixes for Wine integration changed,
/// so the stock SSE Engine Fixes is deployed exactly as its mod ships it.
///
/// Runs after deploys and before launch (callers hold the deploy context);
/// it is idempotent and only touches files the integration created or
/// changed:
///
/// - `.disabled` renames of `EngineFixes.dll` / `d3dx9_42.dll` in staging and
///   in the game folder are renamed back (a stale duplicate is dropped).
/// - Withheld stock files are re-deployed from their mod's staging copy and
///   recorded in the deployment manifest.
/// - The fork's files in `Data/SKSE/Plugins` are deleted unless a mod the
///   user installed owns them (manifest-tracked).
/// - A deployed `EngineFixes.toml` that Corkscrew patched is re-linked from
///   the mod's staging copy. A staging copy patched by older versions can't
///   be recovered, so the user is told to reinstall Engine Fixes.
///
/// No mod changes state, so no rollback snapshot is taken.
pub fn migrate_from_engine_fixes_wine(
    db: &Arc<ModDatabase>,
    game_id: &str,
    bottle_name: &str,
    data_dir: &Path,
    game_path: &Path,
) -> EngineFixesMigration {
    let mut out = EngineFixesMigration::default();

    let healed = restore_legacy_disabled_in_staging(db, game_id, bottle_name);
    if healed > 0 {
        info!(
            "Engine Fixes: restored {} stock file(s) earlier versions renamed inside staging",
            healed
        );
    }

    match remove_legacy_wine_fork(db, game_id, bottle_name, data_dir) {
        ForkCleanup::Removed(files) => out.notices.push(format!(
            "Removed the retired SSE Engine Fixes for Wine ({}). Stock SSE Engine Fixes now \
             works under Wine and is used instead.",
            files.join(", ")
        )),
        ForkCleanup::OwnedByMod => out.warnings.push(
            "SSE Engine Fixes for Wine is installed as a mod. It is no longer needed (stock \
             SSE Engine Fixes now works under Wine) and shouldn't load alongside it: disable \
             or uninstall it."
                .to_string(),
        ),
        ForkCleanup::NotPresent => {}
    }

    match crate::deployer::ensure_paths_deployed(
        db,
        game_id,
        bottle_name,
        data_dir,
        game_path,
        STOCK_ENGINE_FIXES_FILES,
    ) {
        Ok(restored) if !restored.is_empty() => out.notices.push(format!(
            "Restored stock Engine Fixes file(s): {}",
            restored.join(", ")
        )),
        Ok(_) => {}
        Err(e) => warn!("Engine Fixes: could not restore stock files: {}", e),
    }
    let unrenamed = restore_untracked_stock_engine_fixes(data_dir, game_path);
    if !unrenamed.is_empty() {
        out.notices.push(format!(
            "Re-enabled stock Engine Fixes file(s): {}",
            unrenamed.join(", ")
        ));
    }

    repair_engine_fixes_toml(db, game_id, bottle_name, data_dir, game_path, &mut out);

    for w in &out.warnings {
        warn!("Engine Fixes: {}", w);
    }
    for n in &out.notices {
        info!("Engine Fixes: {}", n);
    }
    out
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

/// Undo the deployed-side `.dll.disabled` renames of earlier versions: bring
/// the file back when nothing is deployed at the original path; drop it when
/// it's a stale duplicate of the file that is now deployed there.
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
/// mod's *staging* directory, so redeploys couldn't bring them back. Rename
/// them back. Returns the number restored.
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

/// Outcome of [`remove_legacy_wine_fork`].
#[derive(Debug, PartialEq, Eq)]
enum ForkCleanup {
    NotPresent,
    /// Deleted these files (names as found on disk).
    Removed(Vec<String>),
    /// The fork DLL belongs to a mod the user installed; left in place.
    OwnedByMod,
}

/// Delete the fork's files from `Data/SKSE/Plugins` (looked up
/// case-insensitively). Files recorded in the deployment manifest belong to
/// a user-installed mod and are kept; if the plugin itself is one of them,
/// nothing is touched. When the manifest can't be read nothing is deleted.
fn remove_legacy_wine_fork(
    db: &Arc<ModDatabase>,
    game_id: &str,
    bottle_name: &str,
    data_dir: &Path,
) -> ForkCleanup {
    let Some(plugins_dir) = crate::fs_ci::find_dir_ci(data_dir, "SKSE")
        .and_then(|skse| crate::fs_ci::find_dir_ci(&skse, "Plugins"))
    else {
        return ForkCleanup::NotPresent;
    };
    let present: Vec<PathBuf> = LEGACY_WINE_FORK_FILES
        .iter()
        .filter_map(|name| crate::fs_ci::find_file_ci(&plugins_dir, name))
        .collect();
    if present.is_empty() {
        return ForkCleanup::NotPresent;
    }

    let manifest = match db.get_deployment_manifest(game_id, bottle_name) {
        Ok(m) => m,
        Err(e) => {
            warn!(
                "Engine Fixes: could not read the deployment manifest, leaving SSE Engine Fixes \
                 for Wine files in place: {}",
                e
            );
            return ForkCleanup::NotPresent;
        }
    };
    let owned_by_mod = |name: &str| {
        let rel = format!("skse/plugins/{}", name.to_ascii_lowercase());
        manifest.iter().any(|e| {
            e.deploy_target == "data"
                && e.relative_path.replace('\\', "/").to_ascii_lowercase() == rel
        })
    };
    let file_name = |p: &Path| p.file_name().unwrap_or_default().to_string_lossy().into_owned();

    if present.iter().any(|p| {
        let name = file_name(p);
        name.eq_ignore_ascii_case(LEGACY_WINE_FORK_FILES[0]) && owned_by_mod(&name)
    }) {
        debug!("Engine Fixes: SSE Engine Fixes for Wine is a managed mod file; leaving it");
        return ForkCleanup::OwnedByMod;
    }

    let mut removed = Vec::new();
    for path in present {
        let name = file_name(&path);
        if owned_by_mod(&name) {
            debug!("Engine Fixes: {} belongs to a mod; leaving it", name);
            continue;
        }
        // remove_file deletes a symlink itself, never its target, so
        // nothing outside Data can be deleted here.
        match fs::remove_file(&path) {
            Ok(()) => {
                info!("Engine Fixes: removed {}", path.display());
                removed.push(name);
            }
            Err(e) => warn!("Engine Fixes: failed to remove {}: {}", path.display(), e),
        }
    }
    if removed.is_empty() {
        ForkCleanup::NotPresent
    } else {
        ForkCleanup::Removed(removed)
    }
}

/// Whether `content` is an `EngineFixes.toml` that Corkscrew's old Wine patch
/// rewrote: `bDisableTBB = true` and every [`LEGACY_WINE_TOML_SIGNATURE`]
/// setting off.
fn is_legacy_wine_patched_toml(content: &str) -> bool {
    let mut settings = std::collections::HashMap::new();
    for line in content.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if let Some((key, value)) = line.split_once('=') {
            settings.insert(key.trim(), value.trim());
        }
    }
    settings.get("bDisableTBB") == Some(&"true")
        && LEGACY_WINE_TOML_SIGNATURE
            .iter()
            .all(|k| settings.get(k) == Some(&"false"))
}

fn is_legacy_wine_patched_toml_file(path: &Path) -> bool {
    // A real EngineFixes.toml is a few KB; don't read anything unexpected.
    const MAX_LEN: u64 = 256 * 1024;
    match fs::metadata(path) {
        Ok(m) if m.is_file() && m.len() <= MAX_LEN => {}
        _ => return false,
    }
    fs::read_to_string(path)
        .map(|c| is_legacy_wine_patched_toml(&c))
        .unwrap_or(false)
}

/// `<base>/SKSE/Plugins/EngineFixes.toml`, resolved case-insensitively.
fn find_engine_fixes_toml(base: &Path) -> Option<PathBuf> {
    let skse = crate::fs_ci::find_dir_ci(base, "SKSE")?;
    let plugins = crate::fs_ci::find_dir_ci(&skse, "Plugins")?;
    crate::fs_ci::find_file_ci(&plugins, "EngineFixes.toml")
}

/// Re-link a Corkscrew-patched deployed `EngineFixes.toml` from its mod's
/// staging copy, and warn when no unpatched copy exists to restore.
fn repair_engine_fixes_toml(
    db: &Arc<ModDatabase>,
    game_id: &str,
    bottle_name: &str,
    data_dir: &Path,
    game_path: &Path,
    out: &mut EngineFixesMigration,
) {
    let deployed_patched =
        || find_engine_fixes_toml(data_dir).is_some_and(|p| is_legacy_wine_patched_toml_file(&p));

    if deployed_patched() {
        // Only when Corkscrew patched it: a config the user edited by hand
        // (copy deploys) is left alone.
        match crate::deployer::ensure_paths_deployed(
            db,
            game_id,
            bottle_name,
            data_dir,
            game_path,
            &[STOCK_ENGINE_FIXES_TOML],
        ) {
            Ok(restored) if !restored.is_empty() => out.notices.push(
                "Restored the stock EngineFixes.toml settings (an older Corkscrew version had \
                 switched its patches off for SSE Engine Fixes for Wine)."
                    .to_string(),
            ),
            Ok(_) => {}
            Err(e) => warn!("Engine Fixes: could not restore EngineFixes.toml: {}", e),
        }
    }

    let patched_in_staging: Vec<String> = db
        .list_mods(game_id, bottle_name)
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m.enabled)
        .filter(|m| {
            m.staging_path
                .as_deref()
                .and_then(|sp| find_engine_fixes_toml(Path::new(sp)))
                .is_some_and(|p| is_legacy_wine_patched_toml_file(&p))
        })
        .map(|m| m.name)
        .collect();

    if !patched_in_staging.is_empty() {
        out.warnings.push(format!(
            "EngineFixes.toml in '{}' has every Engine Fixes patch switched off: an older \
             Corkscrew version changed it for the retired SSE Engine Fixes for Wine, and the \
             original settings can't be recovered. Reinstall SSE Engine Fixes to get them back.",
            patched_in_staging.join("', '")
        ));
    } else if deployed_patched() {
        out.warnings.push(
            "EngineFixes.toml in Data/SKSE/Plugins has every Engine Fixes patch switched off \
             (an older Corkscrew version changed it for the retired SSE Engine Fixes for Wine) \
             and there is no original copy to restore. Reinstall SSE Engine Fixes to get its \
             default settings back."
                .to_string(),
        );
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod engine_fixes_migration_tests {
    use super::*;
    use crate::deployer;
    use tempfile::TempDir;

    /// Trimmed-down stock EngineFixes.toml (same layout as upstream 7.0.x).
    const STOCK_TOML: &str = "\
[Fixes]
bArcheryDownwardAiming = true                   # fixes a bug
bMemoryAccessErrors = true                      # fixes errors
bStuckMouseButtons = true                       # added in 7.x
[Patches]
bDisableChargenPrecache = false                 # off by default
bFormCaching = true                             # speedup
bMaxStdIO = true                                # handles
bSafeExit = true                                # no hang on exit
[MemoryManager]
bOverrideMemoryManager = true                   # malloc/free
bOverrideScrapHeap = true                       # malloc/free
bReplaceImports = true                          # CRT imports
[Debug]
bDisableTBB = false                             # use CRT allocator instead of tbb
";

    /// What the retired Wine patch made of [`STOCK_TOML`]: every listed
    /// boolean off (keys it didn't know, like bStuckMouseButtons, untouched)
    /// and TBB disabled.
    fn wine_patched(toml: &str) -> String {
        let mut s = toml.to_string();
        for key in [
            "bArcheryDownwardAiming",
            "bMemoryAccessErrors",
            "bFormCaching",
            "bMaxStdIO",
            "bSafeExit",
            "bOverrideMemoryManager",
            "bOverrideScrapHeap",
            "bReplaceImports",
        ] {
            s = s.replacen(&format!("{key} = true"), &format!("{key} = false"), 1);
        }
        s.replacen("bDisableTBB = false", "bDisableTBB = true", 1)
    }

    /// Write via temp + rename so a hardlinked deployed copy is replaced,
    /// not written through to staging (as the old patcher did).
    fn replace_file(path: &Path, content: &[u8]) {
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, content).unwrap();
        fs::rename(&tmp, path).unwrap();
    }

    struct Fixture {
        tmp: TempDir,
        db: Arc<ModDatabase>,
        data: PathBuf,
        root: PathBuf,
        ef_staging: PathBuf,
        ef_mod: i64,
    }

    impl Fixture {
        fn add_mod(&self, name: &str, files: &[(&str, &[u8])], target: &str) -> (i64, PathBuf) {
            let id = self
                .db
                .add_mod("skyrimse", "Gaming", None, name, "1", "a.zip", &[])
                .unwrap();
            let staging = self.tmp.path().join("staging").join(format!("{id}"));
            for (rel, bytes) in files {
                let p = staging.join(rel);
                fs::create_dir_all(p.parent().unwrap()).unwrap();
                fs::write(p, bytes).unwrap();
            }
            self.db.set_staging_path(id, staging.to_str().unwrap()).unwrap();
            let base = if target == "root" { &self.root } else { &self.data };
            let list = crate::staging::list_staging_files(&staging).unwrap();
            deployer::deploy_mod(&self.db, "skyrimse", "Gaming", id, &staging, base, &list, target)
                .unwrap();
            (id, staging)
        }

        fn recorded(&self, rel: &str) -> bool {
            self.db
                .get_deployment_manifest("skyrimse", "Gaming")
                .unwrap()
                .iter()
                .any(|e| e.relative_path.eq_ignore_ascii_case(rel))
        }

        fn migrate(&self) -> EngineFixesMigration {
            migrate_from_engine_fixes_wine(&self.db, "skyrimse", "Gaming", &self.data, &self.root)
        }

        fn plugins(&self) -> PathBuf {
            self.data.join("SKSE/Plugins")
        }
    }

    /// One "Engine Fixes" mod (SKSE plugin + toml, Data target) and one
    /// "Engine Fixes part 2" mod (preloader, root target), both deployed.
    fn fixture() -> Fixture {
        let tmp = TempDir::new().unwrap();
        let db = Arc::new(ModDatabase::new(&tmp.path().join("t.db")).unwrap());
        let root = tmp.path().join("game");
        let data = root.join("Data");
        fs::create_dir_all(&data).unwrap();
        let mut f = Fixture { tmp, db, data, root, ef_staging: PathBuf::new(), ef_mod: 0 };
        let (ef_mod, ef_staging) = f.add_mod(
            "Engine Fixes",
            &[
                ("SKSE/Plugins/EngineFixes.dll", b"stock plugin"),
                ("SKSE/Plugins/EngineFixes.toml", STOCK_TOML.as_bytes()),
            ],
            "data",
        );
        f.add_mod("Engine Fixes part 2", &[("d3dx9_42.dll", b"preloader")], "root");
        f.ef_mod = ef_mod;
        f.ef_staging = ef_staging;
        f
    }

    /// The deployed state the "option on" path of earlier versions left
    /// behind: stock DLLs withheld (deleted, records dropped), deployed TOML
    /// patched, fork files installed.
    fn simulate_wine_fork_deployed(f: &Fixture) {
        let manifest = f.db.get_deployment_manifest("skyrimse", "Gaming").unwrap();
        let mut ids = Vec::new();
        for e in &manifest {
            let rel = e.relative_path.to_ascii_lowercase();
            if rel == "skse/plugins/enginefixes.dll" || rel == "d3dx9_42.dll" {
                let base = if e.deploy_target == "root" { &f.root } else { &f.data };
                fs::remove_file(base.join(&e.relative_path)).unwrap();
                ids.push(e.id);
            }
        }
        assert_eq!(ids.len(), 2);
        f.db.remove_deployment_entries_by_ids(&ids).unwrap();
        replace_file(
            &f.plugins().join("EngineFixes.toml"),
            wine_patched(STOCK_TOML).as_bytes(),
        );
        for name in LEGACY_WINE_FORK_FILES {
            fs::write(f.plugins().join(name), b"fork").unwrap();
        }
    }

    #[test]
    fn signature_matches_only_the_wine_patched_toml() {
        assert!(!is_legacy_wine_patched_toml(STOCK_TOML));
        assert!(is_legacy_wine_patched_toml(&wine_patched(STOCK_TOML)));
        // A user who only switched TBB off is not mistaken for the patch.
        assert!(!is_legacy_wine_patched_toml(
            &STOCK_TOML.replacen("bDisableTBB = false", "bDisableTBB = true", 1)
        ));
        assert!(!is_legacy_wine_patched_toml(""));
    }

    #[test]
    fn fork_is_removed_and_stock_engine_fixes_restored() {
        let f = fixture();
        simulate_wine_fork_deployed(&f);

        let out = f.migrate();
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        assert!(!out.notices.is_empty(), "the cleanup must be reported");

        for name in LEGACY_WINE_FORK_FILES {
            assert!(!f.plugins().join(name).exists(), "{name} must be removed");
        }
        assert_eq!(fs::read(f.plugins().join("EngineFixes.dll")).unwrap(), b"stock plugin");
        assert_eq!(fs::read(f.root.join("d3dx9_42.dll")).unwrap(), b"preloader");
        assert!(!f.data.join("d3dx9_42.dll").exists(), "preloader stays in the game root");
        assert!(f.recorded("SKSE/Plugins/EngineFixes.dll"));
        assert!(f.recorded("d3dx9_42.dll"));
        assert_eq!(
            fs::read_to_string(f.plugins().join("EngineFixes.toml")).unwrap(),
            STOCK_TOML,
            "patched deployed TOML re-linked from staging"
        );
        assert_eq!(
            fs::read_to_string(f.ef_staging.join("SKSE/Plugins/EngineFixes.toml")).unwrap(),
            STOCK_TOML,
            "staging untouched"
        );
        let toml_rows = f
            .db
            .get_deployment_entries_for_mod(f.ef_mod)
            .unwrap()
            .into_iter()
            .filter(|e| e.relative_path.eq_ignore_ascii_case("SKSE/Plugins/EngineFixes.toml"))
            .count();
        assert_eq!(toml_rows, 1, "exactly one manifest row for the TOML");

        // Idempotent: a second pass finds nothing to do.
        let again = f.migrate();
        assert!(again.notices.is_empty(), "{:?}", again.notices);
        assert!(again.warnings.is_empty(), "{:?}", again.warnings);
    }

    #[test]
    fn untouched_install_is_left_alone() {
        let f = fixture();
        let out = f.migrate();
        assert!(out.notices.is_empty() && out.warnings.is_empty(), "{out:?}");
        assert_eq!(fs::read(f.plugins().join("EngineFixes.dll")).unwrap(), b"stock plugin");
    }

    #[test]
    fn fork_installed_as_a_mod_is_kept_with_a_warning() {
        let f = fixture();
        f.add_mod(
            "SSE Engine Fixes for Wine",
            &[
                ("SKSE/Plugins/0_SSEEngineFixesForWine.dll", b"fork"),
                ("SKSE/Plugins/SSEEngineFixesForWine.toml", b"cfg"),
            ],
            "data",
        );
        let out = f.migrate();
        assert!(f.plugins().join("0_SSEEngineFixesForWine.dll").is_file());
        assert!(f.plugins().join("SSEEngineFixesForWine.toml").is_file());
        assert_eq!(out.warnings.len(), 1, "{:?}", out.warnings);
        assert!(out.warnings[0].contains("installed as a mod"));
    }

    #[test]
    fn fork_files_are_found_case_insensitively() {
        let f = fixture();
        let dll = f.plugins().join("0_sseenginefixesforwine.DLL");
        fs::write(&dll, b"fork").unwrap();
        let out = f.migrate();
        assert!(!dll.exists());
        assert_eq!(out.notices.len(), 1, "{:?}", out.notices);
    }

    #[test]
    fn toml_patched_in_staging_warns_to_reinstall() {
        let f = fixture();
        // Older versions patched the staging copy too; the original is gone.
        let staged = f.ef_staging.join("SKSE/Plugins/EngineFixes.toml");
        let deployed = f.plugins().join("EngineFixes.toml");
        replace_file(&staged, wine_patched(STOCK_TOML).as_bytes());
        replace_file(&deployed, wine_patched(STOCK_TOML).as_bytes());

        let out = f.migrate();
        assert_eq!(out.warnings.len(), 1, "{:?}", out.warnings);
        assert!(out.warnings[0].contains("'Engine Fixes'"), "{}", out.warnings[0]);
        assert!(out.warnings[0].contains("Reinstall"), "{}", out.warnings[0]);
        // Nothing is invented: both copies stay as they are.
        assert!(is_legacy_wine_patched_toml_file(&staged));
        assert!(is_legacy_wine_patched_toml_file(&deployed));
    }

    #[test]
    fn user_edited_toml_is_not_overwritten() {
        let f = fixture();
        let deployed = f.plugins().join("EngineFixes.toml");
        let edited = STOCK_TOML.replacen("bSafeExit = true", "bSafeExit = false", 1);
        replace_file(&deployed, edited.as_bytes());
        let out = f.migrate();
        assert!(out.notices.is_empty() && out.warnings.is_empty(), "{out:?}");
        assert_eq!(fs::read_to_string(&deployed).unwrap(), edited);
    }

    #[test]
    fn legacy_staging_rename_is_undone_and_stock_redeployed() {
        let f = fixture();
        // State left by older versions: deployed copy AND staging copy renamed,
        // manifest row still pointing at EngineFixes.dll.
        let deployed = f.plugins().join("EngineFixes.dll");
        let staged = f.ef_staging.join("SKSE/Plugins/EngineFixes.dll");
        fs::rename(&deployed, disabled_sibling(&deployed)).unwrap();
        fs::rename(&staged, disabled_sibling(&staged)).unwrap();
        assert!(f.recorded("SKSE/Plugins/EngineFixes.dll"));

        f.migrate();
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

        // A full redeploy keeps the root preloader in the game root.
        deployer::redeploy_all(&f.db, "skyrimse", "Gaming", &f.data, &f.root).unwrap();
        assert!(f.root.join("d3dx9_42.dll").exists());
        assert!(!f.data.join("d3dx9_42.dll").exists());
    }

    /// The fork is retired: nothing in the backend may fetch its releases.
    #[test]
    fn no_code_downloads_the_wine_fork() {
        let needles = [
            ["SSEEngineFixes", "ForWine/releases"].concat(),
            ["install_engine_fixes", "_wine"].concat(),
        ];
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for entry in walkdir::WalkDir::new(&src).into_iter().filter_map(|e| e.ok()) {
            if entry.path().extension().is_some_and(|e| e == "rs") {
                let text = fs::read_to_string(entry.path()).unwrap();
                for needle in &needles {
                    assert!(
                        !text.contains(needle.as_str()),
                        "{} still references `{}`",
                        entry.path().display(),
                        needle
                    );
                }
            }
        }
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
        // SKSE names its runtime DLL after the game version it supports.
        fs::write(game_dir.join("skse64_1_6_1170.dll"), b"fake dll").unwrap();
        fs::write(
            game_dir.join("skse64_steam_loader.dll"),
            b"fake steam loader",
        )
        .unwrap();

        let status = detect_skse(game_dir);
        assert!(status.installed);
        assert!(status.loader_path.is_some());
        assert_eq!(status.runtime, Some("1.6.1170".to_string()));
        // Not a real PE, so the SKSE version resource can't be read.
        assert_eq!(status.version, None);
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
        fs::write(game_dir.join("SKSE64_1_7_104.dll"), b"fake").unwrap();

        let status = detect_skse(game_dir);
        assert!(status.installed);
        assert_eq!(status.runtime, Some("1.7.104".to_string()));
    }

    #[test]
    fn skse_compat_maps_each_build_to_its_single_runtime() {
        // Targets from CURRENT_RELEASE_RUNTIME in each tag's skse_version.h.
        for (skse, game) in [
            ("2.0.20", "1.5.97"),
            ("2.1.0", "1.6.317"),
            ("2.1.5", "1.6.353"),
            ("2.2.0", "1.6.629"),
            ("2.2.3", "1.6.640"),
            ("2.2.5", "1.6.1130"),
            ("2.2.6", "1.6.1170"),
            ("2.2.8", "1.6.1170"),
            ("2.3.0", "1.7.99"),
            ("2.3.1", "1.7.104"),
        ] {
            assert_eq!(skse_game_compatibility(skse), Some((game, game)), "SKSE {skse}");
        }
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
            runtime: None,
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
            runtime: Some("1.5.97".into()),
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
            runtime: Some("1.6.1170".into()),
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

    /// Live check against a real SKSE install (not redistributable, so not a
    /// fixture): `SKSE_TEST_DIR=<dir with skse64_loader.exe + skse64_*.dll>
    /// cargo test --lib skse_live -- --ignored`.
    #[test]
    #[ignore]
    fn skse_live_detects_real_version_and_runtime() {
        let dir = std::env::var("SKSE_TEST_DIR").expect("set SKSE_TEST_DIR");
        let status = detect_skse(Path::new(&dir));
        eprintln!("{status:?}");
        assert!(status.installed);
        assert!(status.version.is_some() && status.runtime.is_some());
        assert_eq!(
            skse_game_compatibility(status.version.as_deref().unwrap()).map(|(g, _)| g.to_string()),
            status.runtime
        );
    }

    /// The reported case: SKSE 2.2.6 (for 1.6.1170) on a 1.7.104 game. The
    /// old check classed 1.7.x as "Unknown" and called it compatible.
    #[test]
    fn check_compat_rejects_1_6_skse_on_1_7_game_and_names_the_nexus_build() {
        let skse_status = SkseStatus {
            installed: true,
            loader_path: Some("/game/skse64_loader.exe".into()),
            version: Some("2.2.6".into()),
            runtime: Some("1.6.1170".into()),
            use_skse: true,
        };
        let downgrade_status = crate::downgrader::DowngradeStatus {
            current_version: "1.7.104".into(),
            target_version: "1.5.97".into(),
            is_downgraded: false,
            downgrade_path: None,
        };
        let result = check_skse_compatibility(&skse_status, &downgrade_status);
        assert!(!result.compatible);
        assert_eq!(result.severity, "error");
        assert!(result.message.contains("SKSE 2.3.1"), "{}", result.message);
        assert!(result.message.contains(SKSE_NEXUS_FILES_URL), "{}", result.message);
    }

    #[test]
    fn check_compat_accepts_exact_runtime_match_on_1_7() {
        let skse_status = SkseStatus {
            installed: true,
            loader_path: Some("/game/skse64_loader.exe".into()),
            version: Some("2.3.1".into()),
            runtime: Some("1.7.104".into()),
            use_skse: true,
        };
        let downgrade_status = crate::downgrader::DowngradeStatus {
            current_version: "1.7.104".into(),
            target_version: "1.5.97".into(),
            is_downgraded: false,
            downgrade_path: None,
        };
        let result = check_skse_compatibility(&skse_status, &downgrade_status);
        assert!(result.compatible);
        assert_eq!(result.severity, "ok");
    }

    #[test]
    fn builds_for_1_7_104_recommend_nexus_only_2_3_1() {
        let builds = get_available_skse_builds("1.7.104");
        assert_eq!(builds.edition, "AE");
        let rec = builds.recommended.expect("2.3.1 targets 1.7.104");
        assert_eq!(rec.version, "2.3.1");
        assert!(!rec.auto_install);
        assert_eq!(rec.download_url, SKSE_NEXUS_FILES_URL);
        // No GitHub build may be offered as a fallback for another runtime.
        assert!(builds.all_builds.iter().filter(|b| b.is_recommended).count() == 1);
    }

    #[test]
    fn builds_for_unlisted_runtime_recommend_nothing() {
        // Before the fix, any newer game got the newest older SKSE (2.2.6).
        assert!(get_available_skse_builds("1.7.200").recommended.is_none());
        assert!(get_available_skse_builds("1.6.1171").recommended.is_none());
    }

    #[test]
    fn auto_install_refuses_nexus_only_build_with_instructions() {
        let tmp = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let err = rt
            .block_on(install_skse_auto(tmp.path(), "1.7.104"))
            .expect_err("2.3.1 is Nexus-only");
        let msg = err.to_string();
        assert!(msg.contains("Nexus") && msg.contains("Install from archive"), "{msg}");
        assert!(fs::read_dir(tmp.path()).unwrap().next().is_none(), "nothing installed");
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
            runtime: None,
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
            // An SE-era build may lack the 1.6.629 struct-layout flag, in
            // which case SKSE (and so check_skse_dll_compat) rejects it.
            let compat = check_skse_dll_compat(se_dll, ae_runtime);
            assert!(
                matches!(
                    compat,
                    SksePluginCompat::VersionIndependent | SksePluginCompat::Rejected { .. }
                ),
                "SE BDI should be VersionIndependent or Rejected, got {:?}",
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

    // --- SKSE load-check port (PluginManager.cpp CheckPluginCompatibility) ---

    const RT_1_6_640: u32 = 0x01062800;
    const RT_1_6_1170: u32 = 0x01064920;
    const RT_1_7_104: u32 = 0x01070680;
    const SKSE_2_3_1: u32 = 0x02030010;
    /// 2024-01-01 — inside SKSE's "old Address Library" build window.
    const BUILT_2024: u32 = 1_704_067_200;
    /// 2025-06-01 — after the 2025-05-26 cutoff.
    const BUILT_2025_06: u32 = 1_748_736_000;

    fn plugin(vi: u32, vix: u32, compat: &[u32]) -> PluginVersionFields {
        PluginVersionFields {
            data_version: 1,
            has_name: true,
            version_independence_ex: vix,
            version_independence: vi,
            compatible_versions: compat.to_vec(),
            se_version_required: 0,
        }
    }

    #[test]
    fn skse_load_verdict_table() {
        use PluginLoadVerdict::{Loads, Rejected};
        const AL: u32 = VERSION_INDEPENDENT_ADDRESS_LIBRARY;
        const SIG: u32 = VERSION_INDEPENDENT_SIGNATURES;
        const P629: u32 = VERSION_INDEPENDENT_STRUCTS_POST_629;
        const NOSTRUCT: u32 = VERSION_INDEPENDENT_EX_NO_STRUCT_USE;
        const ALV5: u32 = VERSION_INDEPENDENT_EX_ADDRESS_LIBRARY_V5;
        let loads_vi = Loads { version_independent: true };
        let loads_list = Loads { version_independent: false };

        // (case, fields, build_time, runtime, address library present, expected)
        let cases: Vec<(&str, PluginVersionFields, u32, u32, bool, PluginLoadVerdict)> = vec![
            ("bad version data", PluginVersionFields { data_version: 0, ..plugin(AL | P629, 0, &[]) },
                BUILT_2024, RT_1_6_1170, true, Rejected(SKSE_MSG_BAD_VERSION_DATA)),
            ("no name", PluginVersionFields { has_name: false, ..plugin(AL | P629, 0, &[]) },
                BUILT_2024, RT_1_6_1170, true, Rejected(SKSE_MSG_NO_NAME)),
            ("unknown independence bit", plugin(AL | P629 | (1 << 3), 0, &[]),
                BUILT_2024, RT_1_6_1170, true, Rejected(SKSE_MSG_UNKNOWN_INDEPENDENCE)),
            ("AL plugin on 1.6.1170 loads", plugin(AL | P629, 0, &[]),
                BUILT_2024, RT_1_6_1170, true, loads_vi.clone()),
            ("AL plugin without its versionlib", plugin(AL | P629, 0, &[]),
                BUILT_2024, RT_1_6_1170, false, Rejected(SKSE_MSG_ADDRESS_LIBRARY)),
            ("signature plugin ignores versionlib", plugin(SIG | P629, 0, &[]),
                BUILT_2024, RT_1_6_1170, false, loads_vi.clone()),
            ("pre-2025-05-26 AL build on 1.7.104", plugin(AL | P629, 0, &[]),
                BUILT_2024, RT_1_7_104, true, Rejected(SKSE_MSG_RECOMPILE)),
            ("pre-cutoff build that lists 1.7.104", plugin(AL | P629, 0, &[RT_1_7_104]),
                BUILT_2024, RT_1_7_104, true, loads_list.clone()),
            ("pre-cutoff build with AddressLibraryV5", plugin(AL | P629, ALV5, &[]),
                BUILT_2024, RT_1_7_104, true, loads_vi.clone()),
            ("post-cutoff build without V5 flag", plugin(AL | P629, 0, &[]),
                BUILT_2025_06, RT_1_7_104, true, loads_vi.clone()),
            ("zero timestamp is outside the window", plugin(AL | P629, 0, &[]),
                0, RT_1_7_104, true, loads_vi.clone()),
            ("pre-cutoff signature-only build is unaffected", plugin(SIG | P629, 0, &[]),
                BUILT_2024, RT_1_7_104, true, loads_vi.clone()),
            ("recompile check is 1.7.99+ only", plugin(AL | P629, 0, &[]),
                BUILT_2024, RT_1_6_1170, true, loads_vi.clone()),
            ("independent but pre-629 structs", plugin(AL, 0, &[]),
                BUILT_2025_06, RT_1_6_1170, true, Rejected(SKSE_MSG_PRE_629_ONLY)),
            ("pre-629 structs ok with NoStructUse", plugin(AL, NOSTRUCT, &[]),
                BUILT_2025_06, RT_1_6_1170, true, loads_vi.clone()),
            ("pre-629 structs fine on 1.6.353", plugin(AL, 0, &[]),
                BUILT_2024, 0x01061610, true, loads_vi.clone()),
            ("version list match", plugin(0, 0, &[RT_1_6_640, RT_1_6_1170]),
                BUILT_2024, RT_1_6_1170, true, loads_list.clone()),
            ("version list miss", plugin(0, 0, &[RT_1_6_640]),
                BUILT_2024, RT_1_6_1170, true, Rejected(SKSE_MSG_INCOMPATIBLE)),
            ("no claims at all", plugin(0, 0, &[]),
                BUILT_2024, RT_1_6_1170, true, Rejected(SKSE_MSG_INCOMPATIBLE)),
            ("StructsPost629 alone is not independence", plugin(P629, 0, &[]),
                BUILT_2024, RT_1_6_1170, true, Rejected(SKSE_MSG_INCOMPATIBLE)),
        ];

        for (name, fields, build_time, runtime, al, expected) in cases {
            assert_eq!(
                skse_plugin_load_verdict(&fields, build_time, runtime, al, Some(SKSE_2_3_1)),
                expected,
                "case: {name}"
            );
        }
    }

    #[test]
    fn skse_load_verdict_checks_required_skse_version() {
        let mut p = plugin(
            VERSION_INDEPENDENT_ADDRESS_LIBRARY | VERSION_INDEPENDENT_STRUCTS_POST_629,
            0,
            &[],
        );
        p.se_version_required = 0x02030020; // 2.3.2
        assert_eq!(
            skse_plugin_load_verdict(&p, BUILT_2025_06, RT_1_6_1170, true, Some(SKSE_2_3_1)),
            PluginLoadVerdict::Rejected(SKSE_MSG_NEWER_SKSE)
        );
        p.se_version_required = SKSE_2_3_1;
        assert_eq!(
            skse_plugin_load_verdict(&p, BUILT_2025_06, RT_1_6_1170, true, Some(SKSE_2_3_1)),
            PluginLoadVerdict::Loads { version_independent: true }
        );
        // Unknown SKSE version: the requirement is not checked.
        p.se_version_required = 0x02050000;
        assert_eq!(
            skse_plugin_load_verdict(&p, BUILT_2025_06, RT_1_6_1170, true, None),
            PluginLoadVerdict::Loads { version_independent: true }
        );
    }

    #[test]
    fn classify_maps_recompile_verdict_to_rejected() {
        let info = PluginBinaryInfo {
            fields: plugin(
                VERSION_INDEPENDENT_ADDRESS_LIBRARY | VERSION_INDEPENDENT_STRUCTS_POST_629,
                0,
                &[],
            ),
            name: Some("ScrambledBugs".into()),
            build_time: BUILT_2024,
        };
        match classify_plugin(&info, RT_1_7_104, true, None) {
            SksePluginCompat::Rejected { reason, .. } => assert_eq!(reason, SKSE_MSG_RECOMPILE),
            other => panic!("expected Rejected, got {other:?}"),
        }
        assert!(matches!(
            classify_plugin(&info, RT_1_6_1170, true, None),
            SksePluginCompat::VersionIndependent
        ));
        // SE keeps the plain flag reading (SKSE 2.0.x loads via SKSEPlugin_Query).
        assert!(matches!(
            classify_plugin(&info, 0x01050610, true, None),
            SksePluginCompat::VersionIndependent
        ));
        let listed = PluginBinaryInfo {
            fields: plugin(0, 0, &[RT_1_6_640]),
            name: None,
            build_time: 0,
        };
        match classify_plugin(&listed, RT_1_6_1170, true, None) {
            SksePluginCompat::Incompatible { supports } => assert_eq!(supports, vec!["1.6.640"]),
            other => panic!("expected Incompatible, got {other:?}"),
        }
    }

    #[test]
    fn skse_version_packing() {
        assert_eq!(parse_skse_version_packed("2.3.1"), Some(SKSE_2_3_1));
        assert_eq!(parse_skse_version_packed("2.2.6"), Some(0x02020060));
        assert_eq!(parse_skse_version_packed("2.0.20"), Some(0x02000140));
        // A game runtime string is not an SKSE version.
        assert_eq!(parse_skse_version_packed("1.7.104"), None);
        assert_eq!(parse_skse_version_packed("garbage"), None);
    }

    #[test]
    fn address_library_filename_is_exact_runtime() {
        assert_eq!(address_library_filenames(RT_1_7_104), vec!["versionlib-1-7-104-0.bin"]);
        assert_eq!(address_library_filenames(RT_1_6_1170), vec!["versionlib-1-6-1170-0.bin"]);
        assert_eq!(
            address_library_filenames(0x01050610),
            vec!["version-1-5-97-0.bin", "versionlib-1-5-97-0.bin"]
        );
    }

    #[test]
    fn scan_requires_exact_address_library_for_runtime() {
        let tmp = tempfile::tempdir().unwrap();
        let plugins = tmp.path().join("SKSE").join("Plugins");
        fs::create_dir_all(&plugins).unwrap();
        fs::write(plugins.join("versionlib-1-6-1170-0.bin"), b"x").unwrap();

        // 1.7.104 game with only the 1.6.1170 database: mismatch.
        let r = scan_skse_plugins(tmp.path(), "1.7.104", None);
        assert_eq!(r.address_library_version, "AE");
        assert!(!r.address_library_matches);
        assert_eq!(r.warnings[0].dll_name, "Address Library");
        assert!(
            r.warnings[0].warning.contains("versionlib-1-7-104-0.bin"),
            "{}",
            r.warnings[0].warning
        );

        // 1.6.1170 game: exact match.
        let r = scan_skse_plugins(tmp.path(), "1.6.1170", None);
        assert!(r.address_library_matches);
        assert!(r.warnings.is_empty());

        // 1.6.640 game: a different AE database does not count.
        assert!(!scan_skse_plugins(tmp.path(), "1.6.640", None).address_library_matches);

        // File name match is case-insensitive.
        fs::write(plugins.join("VersionLib-1-7-104-0.bin"), b"x").unwrap();
        assert!(scan_skse_plugins(tmp.path(), "1.7.104", None).address_library_matches);

        // No database and no plugins at all: nothing to mismatch.
        let empty = tempfile::tempdir().unwrap();
        assert!(scan_skse_plugins(empty.path(), "1.7.104", None).address_library_matches);
    }
}
