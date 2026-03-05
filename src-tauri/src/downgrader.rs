//! Skyrim SE version detection and downgrade support.
//!
//! Provides utilities for:
//! - Detecting whether a Skyrim SE installation is the classic SE build (v1.5.97)
//!   or the newer Anniversary Edition (v1.6.x+)
//! - Creating a downgrade copy that isolates the game from Steam auto-updates
//! - Managing downgrade copy paths in the Corkscrew configuration
//!
//! Version detection uses a combination of SHA-256 hashing and file size
//! heuristics, since the SkyrimSE.exe binary differs significantly between
//! the SE and AE builds.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use walkdir::WalkDir;

use crate::config;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum DowngraderError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("SkyrimSE.exe not found in: {0}")]
    ExeNotFound(PathBuf),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("WalkDir error: {0}")]
    WalkDir(#[from] walkdir::Error),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, DowngraderError>;

// ---------------------------------------------------------------------------
// DowngradeStatus
// ---------------------------------------------------------------------------

/// Version and downgrade status for a Skyrim SE installation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DowngradeStatus {
    /// The detected current version (e.g. "1.5.97" or "1.6.x (Anniversary Edition)").
    pub current_version: String,
    /// The target version for modding compatibility.
    pub target_version: String,
    /// Whether the game is already at the target (downgraded/SE) version.
    pub is_downgraded: bool,
    /// Path to the downgrade copy, if one has been created.
    pub downgrade_path: Option<String>,
}

// ---------------------------------------------------------------------------
// Known version data
// ---------------------------------------------------------------------------

/// Target SE version that most SKSE mods are built for.
const TARGET_VERSION: &str = "1.5.97";

/// Known SHA-256 hash of SkyrimSE.exe v1.5.97.0 (the final pre-AE build).
/// This is the most widely-modded version.
const HASH_SE_1_5_97: &str = "1a0b7a68e0ba935c26a8e220ea63b2edb68e20fbb3e41f52fc1e4f2790a198e6";

/// File size of SkyrimSE.exe v1.5.97.0 in bytes.
/// Used as a fast heuristic before falling back to SHA-256.
const SIZE_SE_1_5_97: u64 = 29_210_112;

/// Approximate threshold: AE executables are significantly larger.
/// Any SkyrimSE.exe above this size is almost certainly AE (v1.6.x).
const SIZE_AE_THRESHOLD: u64 = 50_000_000;

/// Known hashes for specific AE versions (for informational display).
/// These are best-effort and not exhaustive.
const KNOWN_AE_VERSIONS: &[(&str, &str)] = &[
    // (hash, version_string)
    (
        "af3a59f8f447b4434a4be361f888d10e7005e9d8cf2e5e68af826e18a2635a0e",
        "1.6.640",
    ),
    (
        "07e6ed30e13a09b81e89c2a8c1cd012e5ecdfe65ee7bc53f8e37e4b0bec0de39",
        "1.6.659",
    ),
    (
        "3b6abc6f8b1660be727adf26ee4b0148fce22ed39a29e91e89a4d0b2536efdf5",
        "1.6.1130",
    ),
    (
        "a72582c3127d6d1e81e31aa29ef68d0b9a1002ba94e6e4c8e2dbf1b14aed6e2e",
        "1.6.1170",
    ),
];

// ---------------------------------------------------------------------------
// Version detection
// ---------------------------------------------------------------------------

/// Detect the Skyrim SE version by examining SkyrimSE.exe.
///
/// Uses a combination of file size and SHA-256 hashing to determine whether
/// the installation is the classic SE build (v1.5.97) or an Anniversary
/// Edition build (v1.6.x+).
///
/// Also checks the configuration for a previously stored Stock Game path.
pub fn detect_skyrim_version(game_path: &Path) -> Result<DowngradeStatus> {
    let exe_path = find_skyrim_exe(game_path)?;

    info!("Detecting Skyrim version from: {}", exe_path.display());

    // Get file metadata for size-based heuristic.
    let metadata = fs::metadata(&exe_path)?;
    let file_size = metadata.len();

    debug!("SkyrimSE.exe file size: {} bytes", file_size);

    // Fast path: check file size first.
    let (current_version, is_downgraded) = if file_size == SIZE_SE_1_5_97 {
        // Exact size match — very likely v1.5.97, but verify with hash.
        let hash = compute_sha256(&exe_path)?;
        if hash == HASH_SE_1_5_97 {
            info!("Confirmed Skyrim SE v1.5.97 via SHA-256");
            ("1.5.97".to_string(), true)
        } else {
            // Same size but different hash — unusual, treat as unknown SE-era build.
            info!(
                "SkyrimSE.exe matches v1.5.97 size but has unexpected hash: {}",
                hash
            );
            ("1.5.97 (unverified)".to_string(), true)
        }
    } else if file_size > SIZE_AE_THRESHOLD {
        // Clearly an AE build — the executable is much larger.
        let hash = compute_sha256(&exe_path)?;
        let version = identify_ae_version(&hash, file_size);
        info!("Detected Anniversary Edition: {}", version);
        (version, false)
    } else if file_size < SIZE_SE_1_5_97 {
        // Smaller than expected — could be an older SE build or something unusual.
        let hash = compute_sha256(&exe_path)?;
        if hash == HASH_SE_1_5_97 {
            ("1.5.97".to_string(), true)
        } else {
            info!(
                "SkyrimSE.exe is smaller than expected ({} bytes, hash: {})",
                file_size, hash
            );
            (format!("Unknown ({}B)", file_size), false)
        }
    } else {
        // Between SE and AE sizes — hash to determine.
        let hash = compute_sha256(&exe_path)?;
        if hash == HASH_SE_1_5_97 {
            ("1.5.97".to_string(), true)
        } else {
            let version = identify_ae_version(&hash, file_size);
            (version, false)
        }
    };

    // Check for existing downgrade copy path.
    let downgrade_path = get_downgrade_path_from_config(game_path);

    Ok(DowngradeStatus {
        current_version,
        target_version: TARGET_VERSION.to_string(),
        is_downgraded,
        downgrade_path: downgrade_path.map(|p| p.to_string_lossy().into_owned()),
    })
}

/// Identify a specific AE version from its hash, or return a generic label.
fn identify_ae_version(hash: &str, file_size: u64) -> String {
    for &(known_hash, version) in KNOWN_AE_VERSIONS {
        if hash == known_hash {
            return version.to_string();
        }
    }

    // Unknown AE build — include approximate size for debugging.
    let size_mb = file_size as f64 / (1024.0 * 1024.0);
    format!("1.6.x (Anniversary Edition, ~{:.1} MB)", size_mb)
}

/// Find the SkyrimSE.exe file in the game directory (case-insensitive).
fn find_skyrim_exe(game_path: &Path) -> Result<PathBuf> {
    let target = "skyrimse.exe";

    // Try exact match first.
    let exact = game_path.join("SkyrimSE.exe");
    if exact.exists() {
        return Ok(exact);
    }

    // Case-insensitive scan of the game directory.
    if let Ok(entries) = fs::read_dir(game_path) {
        for entry in entries.flatten() {
            if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if name == target {
                    return Ok(entry.path());
                }
            }
        }
    }

    Err(DowngraderError::ExeNotFound(game_path.to_path_buf()))
}

/// Compute the SHA-256 hash of a file.
///
/// Reads the file in chunks to handle large executables without loading
/// the entire file into memory at once.
fn compute_sha256(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];

    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    let hash = hasher.finalize();
    Ok(format!("{:x}", hash))
}

// ---------------------------------------------------------------------------
// Version cache
// ---------------------------------------------------------------------------

/// Steam depot constants for Skyrim SE downgrade.
pub const SKYRIM_APP_ID: u32 = 489830;
pub const SKYRIM_DEPOT_ID: u32 = 489833;
pub const SKYRIM_SE_MANIFEST: &str = "4063321535627579835";

/// A cached game executable version.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CachedVersion {
    /// Version string (e.g. "1.5.97" or "1.6.1170").
    pub version: String,
    /// Full path to the cached SkyrimSE.exe.
    pub exe_path: String,
    /// SHA-256 hash of the cached executable.
    pub hash: String,
    /// ISO 8601 timestamp of when this version was cached.
    pub cached_at: String,
}

/// Information needed to run the Steam depot download command.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DepotDownloadInfo {
    /// The console command to paste (e.g. "download_depot 489830 489833 ...").
    pub command: String,
    /// Steam URI to open the console.
    pub steam_uri: String,
    /// The expected download path after completion.
    pub expected_path: String,
}

/// Returns the version cache directory for a given game.
///
/// Layout: `{data_dir}/game_versions/{game_id}/`
///   - `ae/{version}/SkyrimSE.exe` — backed-up AE executables
///   - `se/1.5.97/SkyrimSE.exe` — cached SE executable
fn version_cache_dir(game_id: &str) -> PathBuf {
    config::data_dir().join("game_versions").join(game_id)
}

/// Cache the current game executable before modifying it.
///
/// Detects the version, copies the exe to the version cache, and returns
/// the cached version info.
pub fn cache_current_version(game_path: &Path, game_id: &str) -> Result<CachedVersion> {
    let exe_path = find_skyrim_exe(game_path)?;
    let hash = compute_sha256(&exe_path)?;
    let metadata = fs::metadata(&exe_path)?;
    let file_size = metadata.len();

    // Determine version string
    let version = if hash == HASH_SE_1_5_97 || file_size == SIZE_SE_1_5_97 {
        "1.5.97".to_string()
    } else {
        identify_ae_version(&hash, file_size)
    };

    // Determine cache subdirectory
    let subdir = if version == "1.5.97" || version == "1.5.97 (unverified)" {
        "se"
    } else {
        "ae"
    };

    let cache_dir = version_cache_dir(game_id).join(subdir).join(&version);
    fs::create_dir_all(&cache_dir)?;

    let dest = cache_dir.join("SkyrimSE.exe");
    fs::copy(&exe_path, &dest)?;

    let now = chrono::Utc::now().to_rfc3339();
    let cached = CachedVersion {
        version: version.clone(),
        exe_path: dest.to_string_lossy().into_owned(),
        hash,
        cached_at: now,
    };

    info!("Cached game version {} at {}", version, dest.display());
    Ok(cached)
}

/// List all cached versions for a game.
pub fn list_cached_versions(game_id: &str) -> Vec<CachedVersion> {
    let base = version_cache_dir(game_id);
    let mut versions = Vec::new();

    for subdir in &["se", "ae"] {
        let dir = base.join(subdir);
        if !dir.exists() {
            continue;
        }
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                    continue;
                }
                let version = entry.file_name().to_string_lossy().into_owned();
                let exe = entry.path().join("SkyrimSE.exe");
                if !exe.exists() {
                    continue;
                }
                let hash = compute_sha256(&exe).unwrap_or_default();
                // Try to get modification time as cached_at
                let cached_at = fs::metadata(&exe)
                    .and_then(|m| m.modified())
                    .map(|t| {
                        let dt: chrono::DateTime<chrono::Utc> = t.into();
                        dt.to_rfc3339()
                    })
                    .unwrap_or_default();

                versions.push(CachedVersion {
                    version,
                    exe_path: exe.to_string_lossy().into_owned(),
                    hash,
                    cached_at,
                });
            }
        }
    }

    versions
}

/// Swap the game executable to a specific cached version.
///
/// Copies the cached exe over the current game exe.
pub fn swap_to_version(
    game_path: &Path,
    game_id: &str,
    target_version: &str,
) -> Result<DowngradeStatus> {
    // Find the cached version
    let cached = list_cached_versions(game_id);
    let target = cached
        .iter()
        .find(|c| c.version == target_version)
        .ok_or_else(|| {
            DowngraderError::Other(format!(
                "Version {} is not cached. Download it first.",
                target_version
            ))
        })?;

    let exe_path = find_skyrim_exe(game_path)?;
    let cached_exe = Path::new(&target.exe_path);

    if !cached_exe.exists() {
        return Err(DowngraderError::Other(format!(
            "Cached exe not found at: {}",
            target.exe_path
        )));
    }

    // Copy cached version over current
    fs::copy(cached_exe, &exe_path)?;

    info!("Swapped game to version {} from cache", target_version);

    // Return updated status
    detect_skyrim_version(game_path)
}

/// Import a depot-downloaded exe into the version cache.
///
/// Copies the exe from the depot download location into the SE cache.
pub fn import_depot_exe(depot_exe_path: &Path, game_id: &str) -> Result<CachedVersion> {
    if !depot_exe_path.exists() {
        return Err(DowngraderError::Other(format!(
            "Depot exe not found at: {}",
            depot_exe_path.display()
        )));
    }

    let hash = compute_sha256(depot_exe_path)?;
    let metadata = fs::metadata(depot_exe_path)?;
    let file_size = metadata.len();

    // Determine version — depot download should be 1.5.97
    let version = if hash == HASH_SE_1_5_97 || file_size == SIZE_SE_1_5_97 {
        "1.5.97".to_string()
    } else {
        identify_ae_version(&hash, file_size)
    };

    let cache_dir = version_cache_dir(game_id).join("se").join(&version);
    fs::create_dir_all(&cache_dir)?;

    let dest = cache_dir.join("SkyrimSE.exe");
    fs::copy(depot_exe_path, &dest)?;

    let now = chrono::Utc::now().to_rfc3339();
    let cached = CachedVersion {
        version: version.clone(),
        exe_path: dest.to_string_lossy().into_owned(),
        hash,
        cached_at: now,
    };

    info!(
        "Imported depot exe as version {} at {}",
        version,
        dest.display()
    );
    Ok(cached)
}

// ---------------------------------------------------------------------------
// Steam depot detection
// ---------------------------------------------------------------------------

/// Find the Steam installation directory within a Wine bottle.
pub fn find_steam_dir(bottle_path: &Path) -> Option<PathBuf> {
    // Common Steam paths within Wine bottles
    let candidates = [
        bottle_path.join("drive_c/Program Files (x86)/Steam"),
        bottle_path.join("drive_c/Program Files/Steam"),
    ];

    for path in &candidates {
        if path.exists() {
            return Some(path.clone());
        }
    }

    None
}

/// Get the expected depot download path after `download_depot` completes.
///
/// Returns: `{steam_dir}/steamapps/content/app_{app_id}/depot_{depot_id}/`
pub fn get_depot_download_path(steam_dir: &Path, app_id: u32, depot_id: u32) -> PathBuf {
    steam_dir
        .join("steamapps")
        .join("content")
        .join(format!("app_{}", app_id))
        .join(format!("depot_{}", depot_id))
}

/// Check if depot files have been downloaded.
///
/// Returns the path to SkyrimSE.exe if found in the depot content directory.
pub fn check_depot_downloaded(steam_dir: &Path, app_id: u32, depot_id: u32) -> Option<PathBuf> {
    let depot_dir = get_depot_download_path(steam_dir, app_id, depot_id);

    if !depot_dir.exists() {
        return None;
    }

    // Look for SkyrimSE.exe (case-insensitive)
    let target = "skyrimse.exe";
    if let Ok(entries) = fs::read_dir(&depot_dir) {
        for entry in entries.flatten() {
            if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if name == target {
                    return Some(entry.path());
                }
            }
        }
    }

    None
}

/// Get the depot download command and info for the frontend wizard.
pub fn get_depot_download_info(game_id: &str, bottle_path: &Path) -> Result<DepotDownloadInfo> {
    if game_id != "skyrimse" {
        return Err(DowngraderError::Other(
            "Depot download is only supported for Skyrim SE".to_string(),
        ));
    }

    let steam_dir = find_steam_dir(bottle_path).ok_or_else(|| {
        DowngraderError::Other("Steam directory not found in the bottle".to_string())
    })?;

    let expected_path = get_depot_download_path(&steam_dir, SKYRIM_APP_ID, SKYRIM_DEPOT_ID);

    Ok(DepotDownloadInfo {
        command: format!(
            "download_depot {} {} {}",
            SKYRIM_APP_ID, SKYRIM_DEPOT_ID, SKYRIM_SE_MANIFEST
        ),
        steam_uri: "steam://open/console".to_string(),
        expected_path: expected_path.to_string_lossy().into_owned(),
    })
}

/// Full downgrade flow: cache current AE exe → import depot SE exe → swap.
pub fn apply_depot_downgrade(
    game_path: &Path,
    depot_exe: &Path,
    game_id: &str,
) -> Result<DowngradeStatus> {
    // Step 1: Cache current version before overwriting
    info!("Caching current game version before downgrade...");
    match cache_current_version(game_path, game_id) {
        Ok(cached) => info!("Cached current version: {}", cached.version),
        Err(e) => warn!("Failed to cache current version (continuing anyway): {}", e),
    }

    // Step 2: Import the depot exe into cache
    info!("Importing depot exe into version cache...");
    let imported = import_depot_exe(depot_exe, game_id)?;
    info!("Imported depot version: {}", imported.version);

    // Step 3: Swap to the imported version
    info!("Swapping game to downgraded version...");
    swap_to_version(game_path, game_id, &imported.version)
}

/// Automate the depot download by opening Steam console and typing the command.
/// macOS only — uses osascript to send keystrokes to Steam.
/// Returns Ok(true) if the command was sent successfully, Ok(false) if automation unavailable.
pub fn send_depot_command_to_steam() -> std::result::Result<bool, String> {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command as ProcessCommand;

        // Open Steam console via protocol handler
        ProcessCommand::new("open")
            .arg("steam://open/console")
            .spawn()
            .map_err(|e| format!("Failed to open Steam console: {}", e))?;

        // Wait for console to open
        std::thread::sleep(std::time::Duration::from_secs(3));

        // Type the depot command via AppleScript keystroke
        let depot_command = format!(
            "download_depot {} {} {}",
            SKYRIM_APP_ID, SKYRIM_DEPOT_ID, SKYRIM_SE_MANIFEST
        );
        let script = format!(
            "tell application \"Steam\" to activate\n\
             delay 1\n\
             tell application \"System Events\"\n\
                 keystroke \"{}\"\n\
                 delay 0.3\n\
                 keystroke return\n\
             end tell",
            depot_command
        );

        let output = ProcessCommand::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|e| format!("AppleScript execution failed: {}", e))?;

        if output.status.success() {
            info!("Depot download command sent to Steam console via AppleScript");
            Ok(true)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("AppleScript automation failed: {}", stderr);
            Ok(false)
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// Downgrade copy creation (legacy)
// ---------------------------------------------------------------------------

/// Create a downgrade copy of the entire game installation.
///
/// Copies the full game directory to `target_dir/DowngradedGame/`, providing
/// an isolated copy that is immune to Steam auto-updates. The copy can then
/// be patched to the target version for mod compatibility.
///
/// Returns the path to the created downgrade copy directory.
///
/// # Arguments
///
/// * `game_path` - The original game installation directory.
/// * `target_dir` - The parent directory where `DowngradedGame/` will be created.
pub fn create_downgrade_copy(game_path: &Path, target_dir: &Path) -> Result<PathBuf> {
    let downgrade_path = target_dir.join("DowngradedGame");

    if downgrade_path.exists() {
        warn!(
            "Downgrade copy already exists: {}. It will be replaced.",
            downgrade_path.display()
        );
        fs::remove_dir_all(&downgrade_path)?;
    }

    info!(
        "Creating downgrade copy: {} -> {}",
        game_path.display(),
        downgrade_path.display()
    );

    fs::create_dir_all(&downgrade_path)?;

    // Walk the source directory and copy everything.
    let mut files_copied: u64 = 0;
    let mut bytes_copied: u64 = 0;

    for entry in WalkDir::new(game_path).into_iter().filter_map(|e| e.ok()) {
        let relative = entry
            .path()
            .strip_prefix(game_path)
            .map_err(|e| DowngraderError::Other(e.to_string()))?;

        let dest = downgrade_path.join(relative);

        if entry.file_type().is_dir() {
            fs::create_dir_all(&dest)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            let size = fs::copy(entry.path(), &dest)?;
            files_copied += 1;
            bytes_copied += size;

            if files_copied.is_multiple_of(100) {
                debug!(
                    "Downgrade copy progress: {} files ({:.1} MB)",
                    files_copied,
                    bytes_copied as f64 / (1024.0 * 1024.0)
                );
            }
        }
        // Symlinks are skipped — they can cause issues across Wine prefixes.
    }

    let total_mb = bytes_copied as f64 / (1024.0 * 1024.0);
    info!(
        "Downgrade copy created: {} files, {:.1} MB at {}",
        files_copied,
        total_mb,
        downgrade_path.display()
    );

    // Store the downgrade copy path in config.
    store_downgrade_path(game_path, &downgrade_path);

    Ok(downgrade_path)
}

// ---------------------------------------------------------------------------
// Downgrade path management
// ---------------------------------------------------------------------------

/// Retrieve the stored downgrade copy path for a given game installation.
///
/// Uses `game_id` and `bottle_name` to look up the config key.
pub fn get_downgrade_path(game_id: &str, bottle_name: &str) -> Option<PathBuf> {
    let key = format!("downgrade_{}_{}", game_id, bottle_name);
    config::get_config_value(&key)
        .ok()
        .flatten()
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

/// Internal: look up downgrade copy path using a path-based config key.
fn get_downgrade_path_from_config(game_path: &Path) -> Option<PathBuf> {
    let key = downgrade_config_key(game_path);
    config::get_config_value(&key)
        .ok()
        .flatten()
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

/// Store the downgrade copy path in the configuration.
fn store_downgrade_path(game_path: &Path, downgrade_path: &Path) {
    let key = downgrade_config_key(game_path);
    let value = downgrade_path.to_string_lossy();
    if let Err(e) = config::set_config_value(&key, &value) {
        warn!("Failed to store downgrade path in config: {}", e);
    }
}

/// Generate a config key for the downgrade path based on the game installation path.
///
/// Uses a sanitized version of the game path to create a unique key.
fn downgrade_config_key(game_path: &Path) -> String {
    let path_str = game_path.to_string_lossy();
    let sanitized: String = path_str
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    format!("downgrade_path_{}", sanitized)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn downgrade_status_serializes() {
        let status = DowngradeStatus {
            current_version: "1.5.97".to_string(),
            target_version: "1.5.97".to_string(),
            is_downgraded: true,
            downgrade_path: Some("/path/to/DowngradedGame".to_string()),
        };

        let json = serde_json::to_string(&status).unwrap();
        let deserialized: DowngradeStatus = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.current_version, "1.5.97");
        assert_eq!(deserialized.target_version, "1.5.97");
        assert!(deserialized.is_downgraded);
        assert_eq!(
            deserialized.downgrade_path,
            Some("/path/to/DowngradedGame".to_string())
        );
    }

    #[test]
    fn detect_version_missing_exe() {
        let tmp = tempfile::tempdir().unwrap();
        let result = detect_skyrim_version(tmp.path());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, DowngraderError::ExeNotFound(_)));
    }

    #[test]
    fn detect_version_small_exe_unknown() {
        let tmp = tempfile::tempdir().unwrap();
        // Create a tiny fake SkyrimSE.exe — should be detected as unknown.
        fs::write(tmp.path().join("SkyrimSE.exe"), b"tiny fake exe").unwrap();

        let result = detect_skyrim_version(tmp.path());
        assert!(result.is_ok());
        let status = result.unwrap();
        // A 14-byte file won't match any known version.
        assert!(
            status.current_version.contains("Unknown")
                || status.current_version.contains("unverified"),
            "Expected unknown version, got: {}",
            status.current_version
        );
    }

    #[test]
    fn detect_version_case_insensitive_exe() {
        let tmp = tempfile::tempdir().unwrap();
        // Use lowercase filename.
        fs::write(tmp.path().join("skyrimse.exe"), b"fake exe data").unwrap();

        let result = detect_skyrim_version(tmp.path());
        assert!(result.is_ok());
    }

    #[test]
    fn compute_sha256_correct() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("test.bin");
        fs::write(&file, b"hello world").unwrap();

        let hash = compute_sha256(&file).unwrap();
        // SHA-256 of "hello world" is a well-known value.
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn compute_sha256_empty_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("empty.bin");
        fs::write(&file, b"").unwrap();

        let hash = compute_sha256(&file).unwrap();
        // SHA-256 of empty string.
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn create_downgrade_copy_copies_files() {
        let tmp = tempfile::tempdir().unwrap();

        // Create a fake game directory.
        let game_dir = tmp.path().join("game");
        let game_data = game_dir.join("Data").join("Meshes");
        fs::create_dir_all(&game_data).unwrap();
        fs::write(game_dir.join("SkyrimSE.exe"), b"fake exe").unwrap();
        fs::write(game_dir.join("SkyrimSE.ini"), b"fake ini").unwrap();
        fs::write(game_data.join("mesh.nif"), b"fake mesh").unwrap();

        // Create downgrade copy.
        let target = tmp.path().join("mods");
        let downgrade_path = create_downgrade_copy(&game_dir, &target).unwrap();

        assert_eq!(downgrade_path, target.join("DowngradedGame"));
        assert!(downgrade_path.join("SkyrimSE.exe").exists());
        assert!(downgrade_path.join("SkyrimSE.ini").exists());
        assert!(downgrade_path
            .join("Data")
            .join("Meshes")
            .join("mesh.nif")
            .exists());

        // Verify file contents.
        assert_eq!(
            fs::read_to_string(downgrade_path.join("SkyrimSE.exe")).unwrap(),
            "fake exe"
        );
    }

    #[test]
    fn create_downgrade_copy_replaces_existing() {
        let tmp = tempfile::tempdir().unwrap();

        // Create a fake game directory.
        let game_dir = tmp.path().join("game");
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join("SkyrimSE.exe"), b"original").unwrap();

        let target = tmp.path().join("mods");

        // Create downgrade copy twice.
        let path1 = create_downgrade_copy(&game_dir, &target).unwrap();
        assert_eq!(
            fs::read_to_string(path1.join("SkyrimSE.exe")).unwrap(),
            "original"
        );

        // Update the source and re-create.
        fs::write(game_dir.join("SkyrimSE.exe"), b"updated").unwrap();
        let path2 = create_downgrade_copy(&game_dir, &target).unwrap();
        assert_eq!(path1, path2);
        assert_eq!(
            fs::read_to_string(path2.join("SkyrimSE.exe")).unwrap(),
            "updated"
        );
    }

    #[test]
    fn identify_ae_version_known_hash() {
        let version = identify_ae_version(
            "af3a59f8f447b4434a4be361f888d10e7005e9d8cf2e5e68af826e18a2635a0e",
            65_000_000,
        );
        assert_eq!(version, "1.6.640");
    }

    #[test]
    fn identify_ae_version_unknown_hash() {
        let version = identify_ae_version("0000000000000000000000000000000000", 65_000_000);
        assert!(version.contains("Anniversary Edition"));
        assert!(version.contains("~62.0 MB") || version.contains("61."));
    }

    #[test]
    fn downgrade_config_key_is_deterministic() {
        let path = Path::new("/home/user/.wine/drive_c/Games/Skyrim");
        let key1 = downgrade_config_key(path);
        let key2 = downgrade_config_key(path);
        assert_eq!(key1, key2);
        assert!(key1.starts_with("downgrade_path_"));
        // Should not contain slashes or other special characters.
        assert!(!key1[15..].contains('/'));
    }

    #[test]
    fn get_downgrade_path_returns_none_for_nonexistent() {
        let result = get_downgrade_path("skyrimse", "nonexistent_bottle_xyz");
        assert!(result.is_none());
    }

    #[test]
    fn find_skyrim_exe_finds_exact() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("SkyrimSE.exe"), b"exe").unwrap();

        let result = find_skyrim_exe(tmp.path());
        assert!(result.is_ok());
        assert!(result.unwrap().ends_with("SkyrimSE.exe"));
    }

    #[test]
    fn find_skyrim_exe_case_insensitive() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("SKYRIMSE.EXE"), b"exe").unwrap();

        let result = find_skyrim_exe(tmp.path());
        assert!(result.is_ok());
    }

    #[test]
    fn find_skyrim_exe_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let result = find_skyrim_exe(tmp.path());
        assert!(result.is_err());
    }
}
