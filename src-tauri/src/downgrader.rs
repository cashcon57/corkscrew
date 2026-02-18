//! Skyrim SE version detection and downgrade support.
//!
//! Provides utilities for:
//! - Detecting whether a Skyrim SE installation is the classic SE build (v1.5.97)
//!   or the newer Anniversary Edition (v1.6.x+)
//! - Creating a "Stock Game" copy that isolates the game from Steam auto-updates
//! - Managing Stock Game paths in the Corkscrew configuration
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
    /// Path to the Stock Game copy, if one has been created.
    pub stock_game_path: Option<String>,
}

// ---------------------------------------------------------------------------
// Known version data
// ---------------------------------------------------------------------------

/// Target SE version that most SKSE mods are built for.
const TARGET_VERSION: &str = "1.5.97";

/// Known SHA-256 hash of SkyrimSE.exe v1.5.97.0 (the final pre-AE build).
/// This is the most widely-modded version.
const HASH_SE_1_5_97: &str =
    "1a0b7a68e0ba935c26a8e220ea63b2edb68e20fbb3e41f52fc1e4f2790a198e6";

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

    // Check for existing Stock Game path.
    let stock_game_path = get_stock_game_path_from_config(game_path);

    Ok(DowngradeStatus {
        current_version,
        target_version: TARGET_VERSION.to_string(),
        is_downgraded,
        stock_game_path: stock_game_path.map(|p| p.to_string_lossy().into_owned()),
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
// Stock Game creation
// ---------------------------------------------------------------------------

/// Create a "Stock Game" copy of the entire game installation.
///
/// Copies the full game directory to `target_dir/StockGame/`, providing an
/// isolated copy that is immune to Steam auto-updates. This is a common
/// technique in the Skyrim modding community.
///
/// Returns the path to the created Stock Game directory.
///
/// # Arguments
///
/// * `game_path` - The original game installation directory.
/// * `target_dir` - The parent directory where `StockGame/` will be created.
pub fn create_stock_game(game_path: &Path, target_dir: &Path) -> Result<PathBuf> {
    let stock_game_path = target_dir.join("StockGame");

    if stock_game_path.exists() {
        warn!(
            "Stock Game directory already exists: {}. It will be replaced.",
            stock_game_path.display()
        );
        fs::remove_dir_all(&stock_game_path)?;
    }

    info!(
        "Creating Stock Game copy: {} -> {}",
        game_path.display(),
        stock_game_path.display()
    );

    fs::create_dir_all(&stock_game_path)?;

    // Walk the source directory and copy everything.
    let mut files_copied: u64 = 0;
    let mut bytes_copied: u64 = 0;

    for entry in WalkDir::new(game_path)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let relative = entry
            .path()
            .strip_prefix(game_path)
            .map_err(|e| DowngraderError::Other(e.to_string()))?;

        let dest = stock_game_path.join(relative);

        if entry.file_type().is_dir() {
            fs::create_dir_all(&dest)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            let size = fs::copy(entry.path(), &dest)?;
            files_copied += 1;
            bytes_copied += size;

            if files_copied % 100 == 0 {
                debug!(
                    "Stock Game progress: {} files ({:.1} MB)",
                    files_copied,
                    bytes_copied as f64 / (1024.0 * 1024.0)
                );
            }
        }
        // Symlinks are skipped — they can cause issues across Wine prefixes.
    }

    let total_mb = bytes_copied as f64 / (1024.0 * 1024.0);
    info!(
        "Stock Game created: {} files, {:.1} MB at {}",
        files_copied, total_mb, stock_game_path.display()
    );

    // Store the Stock Game path in config.
    store_stock_game_path(game_path, &stock_game_path);

    Ok(stock_game_path)
}

// ---------------------------------------------------------------------------
// Stock Game path management
// ---------------------------------------------------------------------------

/// Retrieve the stored Stock Game path for a given game installation.
///
/// Uses `game_id` and `bottle_name` derived from the game path if available,
/// or falls back to a path-based config key.
pub fn get_stock_game_path(game_id: &str, bottle_name: &str) -> Option<PathBuf> {
    let key = format!("stock_game_{}_{}", game_id, bottle_name);
    config::get_config_value(&key)
        .ok()
        .flatten()
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

/// Internal: look up Stock Game path using a path-based config key.
fn get_stock_game_path_from_config(game_path: &Path) -> Option<PathBuf> {
    let key = stock_game_config_key(game_path);
    config::get_config_value(&key)
        .ok()
        .flatten()
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

/// Store the Stock Game path in the configuration.
fn store_stock_game_path(game_path: &Path, stock_game_path: &Path) {
    let key = stock_game_config_key(game_path);
    let value = stock_game_path.to_string_lossy();
    if let Err(e) = config::set_config_value(&key, &value) {
        warn!("Failed to store Stock Game path in config: {}", e);
    }
}

/// Generate a config key for the Stock Game path based on the game installation path.
///
/// Uses a sanitized version of the game path to create a unique key.
fn stock_game_config_key(game_path: &Path) -> String {
    let path_str = game_path.to_string_lossy();
    // Create a simple hash-like key from the path to avoid special characters.
    let sanitized: String = path_str
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("stock_game_path_{}", sanitized)
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
            stock_game_path: Some("/path/to/StockGame".to_string()),
        };

        let json = serde_json::to_string(&status).unwrap();
        let deserialized: DowngradeStatus = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.current_version, "1.5.97");
        assert_eq!(deserialized.target_version, "1.5.97");
        assert!(deserialized.is_downgraded);
        assert_eq!(
            deserialized.stock_game_path,
            Some("/path/to/StockGame".to_string())
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
    fn create_stock_game_copies_files() {
        let tmp = tempfile::tempdir().unwrap();

        // Create a fake game directory.
        let game_dir = tmp.path().join("game");
        let game_data = game_dir.join("Data").join("Meshes");
        fs::create_dir_all(&game_data).unwrap();
        fs::write(game_dir.join("SkyrimSE.exe"), b"fake exe").unwrap();
        fs::write(game_dir.join("SkyrimSE.ini"), b"fake ini").unwrap();
        fs::write(game_data.join("mesh.nif"), b"fake mesh").unwrap();

        // Create Stock Game.
        let target = tmp.path().join("mods");
        let stock_path = create_stock_game(&game_dir, &target).unwrap();

        assert_eq!(stock_path, target.join("StockGame"));
        assert!(stock_path.join("SkyrimSE.exe").exists());
        assert!(stock_path.join("SkyrimSE.ini").exists());
        assert!(stock_path.join("Data").join("Meshes").join("mesh.nif").exists());

        // Verify file contents.
        assert_eq!(
            fs::read_to_string(stock_path.join("SkyrimSE.exe")).unwrap(),
            "fake exe"
        );
    }

    #[test]
    fn create_stock_game_replaces_existing() {
        let tmp = tempfile::tempdir().unwrap();

        // Create a fake game directory.
        let game_dir = tmp.path().join("game");
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join("SkyrimSE.exe"), b"original").unwrap();

        let target = tmp.path().join("mods");

        // Create Stock Game twice.
        let path1 = create_stock_game(&game_dir, &target).unwrap();
        assert_eq!(
            fs::read_to_string(path1.join("SkyrimSE.exe")).unwrap(),
            "original"
        );

        // Update the source and re-create.
        fs::write(game_dir.join("SkyrimSE.exe"), b"updated").unwrap();
        let path2 = create_stock_game(&game_dir, &target).unwrap();
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
    fn stock_game_config_key_is_deterministic() {
        let path = Path::new("/home/user/.wine/drive_c/Games/Skyrim");
        let key1 = stock_game_config_key(path);
        let key2 = stock_game_config_key(path);
        assert_eq!(key1, key2);
        assert!(key1.starts_with("stock_game_path_"));
        // Should not contain slashes or other special characters.
        assert!(!key1[16..].contains('/'));
    }

    #[test]
    fn get_stock_game_path_returns_none_for_nonexistent() {
        let result = get_stock_game_path("skyrimse", "nonexistent_bottle_xyz");
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
