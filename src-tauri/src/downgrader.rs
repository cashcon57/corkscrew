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
use std::io;
use std::path::{Path, PathBuf};

use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
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

    let (current_version, is_downgraded) = classify_exe(&exe_path)?;

    // Check for existing downgrade copy path.
    let downgrade_path = get_downgrade_path_from_config(game_path);

    Ok(DowngradeStatus {
        current_version,
        target_version: TARGET_VERSION.to_string(),
        is_downgraded,
        downgrade_path: downgrade_path.map(|p| p.to_string_lossy().into_owned()),
    })
}

/// Read the FileVersion (e.g. "1.5.97") from a PE executable's version resource.
///
/// This is authoritative — Bethesda stamps every SkyrimSE.exe with its build
/// number, so it works for versions we have no hash table entry for, and for
/// GOG/Epic builds whose hashes differ from the Steam depot.
fn pe_file_version(exe_path: &Path) -> Option<String> {
    use pelite::pe64::{Pe, PeFile};

    let bytes = fs::read(exe_path).ok()?;
    let pe = PeFile::from_bytes(&bytes).ok()?;
    let resources = pe.resources().ok()?;
    let info = resources.version_info().ok()?;
    let fixed = info.fixed()?;
    let v = fixed.dwFileVersion;
    Some(format!("{}.{}.{}", v.Major, v.Minor, v.Patch))
}

/// Classify a SkyrimSE.exe as (version string, is at target SE version).
///
/// Primary: the PE version resource. Fallback: size/hash heuristics for
/// executables whose version resource cannot be read.
fn classify_exe(exe_path: &Path) -> Result<(String, bool)> {
    if let Some(version) = pe_file_version(exe_path) {
        let is_se = version.starts_with(TARGET_VERSION);
        info!("SkyrimSE.exe PE version resource: {}", version);
        return Ok((version, is_se));
    }

    // Fallback: file size + hash heuristics.
    let metadata = fs::metadata(exe_path)?;
    let file_size = metadata.len();
    debug!("SkyrimSE.exe file size: {} bytes", file_size);

    let hash = compute_sha256(exe_path)?;
    if hash == HASH_SE_1_5_97 || file_size == SIZE_SE_1_5_97 {
        return Ok(("1.5.97".to_string(), true));
    }
    if file_size > SIZE_AE_THRESHOLD {
        let version = identify_ae_version(&hash, file_size);
        info!("Detected Anniversary Edition: {}", version);
        return Ok((version, false));
    }
    if file_size < SIZE_SE_1_5_97 {
        info!(
            "SkyrimSE.exe is smaller than expected ({} bytes, hash: {})",
            file_size, hash
        );
        return Ok((format!("Unknown ({}B)", file_size), false));
    }
    Ok((identify_ae_version(&hash, file_size), false))
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
/// Delegates to `platform::fast_hash`, which memory-maps files larger than
/// 1 MiB (multi-GB game executables hash at full disk speed) and uses
/// buffered reads for smaller ones.
fn compute_sha256(path: &Path) -> Result<String> {
    Ok(crate::platform::fast_hash(path)?)
}

// ---------------------------------------------------------------------------
// Version cache
// ---------------------------------------------------------------------------

/// Steam depot constants for Skyrim SE downgrade.
pub const SKYRIM_APP_ID: u32 = 489830;
/// Depot 489833 is "Skyrim Special Edition exe" — it contains only SkyrimSE.exe.
pub const SKYRIM_DEPOT_ID: u32 = 489833;
/// Manifest for the v1.5.97.0 executable in depot 489833 (the Nov 20 2019
/// build, the last before AE). This is the manifest every community
/// downgrade guide uses with `download_depot 489830 489833 <manifest>`.
pub const SKYRIM_SE_MANIFEST: &str = "2289561010626853674";

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
    /// The console commands to paste, one per line.
    pub command: String,
    /// Each `download_depot` command (one per depot).
    #[serde(default)]
    pub commands: Vec<String>,
    /// The Skyrim version these depots install.
    #[serde(default)]
    pub target_version: String,
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
    let (version, is_se) = classify_exe(&exe_path)?;

    // Determine cache subdirectory
    let subdir = if is_se { "se" } else { "ae" };

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

    // Determine version — depot download should be 1.5.97
    let (version, is_se) = classify_exe(depot_exe_path)?;
    let subdir = if is_se { "se" } else { "ae" };

    let cache_dir = version_cache_dir(game_id).join(subdir).join(&version);
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

/// All Steam installations whose `steamapps/content/` may hold a depot
/// download: the bottle's Windows Steam plus the host's native Steam.
///
/// The `steam://open/console` automation opens the NATIVE Steam client, so
/// its `download_depot` output lands in the native Steam directory — while a
/// user pasting the command into the bottle's Windows Steam produces files
/// inside the bottle. Both must be searched.
///
/// Steam Play prefixes (Proton, NotProton) and GameHub containers hold only
/// a Steam bridge or GameHub's own client, which Corkscrew can't drive, so
/// for those only the host Steam is searched.
pub fn depot_search_dirs(bottle: &crate::bottles::Bottle) -> Vec<PathBuf> {
    let mut dirs_found = Vec::new();
    let own_windows_steam = !bottle.is_steam_compat_prefix() && bottle.source != "GameHub";
    if own_windows_steam {
        if let Some(dir) = find_steam_dir(&bottle.path) {
            dirs_found.push(dir);
        }
    }
    if let Some(home) = dirs::home_dir() {
        let xdg_data_home = crate::proton::xdg_data_home_env();
        for candidate in native_steam_candidates(&home, xdg_data_home.as_deref()) {
            if candidate.exists() && !dirs_found.iter().any(|d| same_canonical(d, &candidate)) {
                dirs_found.push(candidate);
            }
        }
    }
    dirs_found
}

/// Host Steam roots that `download_depot` (run in the native client) may
/// write to: macOS, native Linux and its `~/.steam` symlinks, both Flatpak
/// layouts, Snap, and a custom `$XDG_DATA_HOME` Steam root. Callers skip
/// missing dirs and aliases of the same install.
fn native_steam_candidates(home: &Path, xdg_data_home: Option<&std::ffi::OsStr>) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = [
        "Library/Application Support/Steam", // macOS
        ".steam/steam",
        ".local/share/Steam",
        ".steam/root",
        ".var/app/com.valvesoftware.Steam/data/Steam",
        ".var/app/com.valvesoftware.Steam/.local/share/Steam",
        "snap/steam/common/.local/share/Steam",
    ]
    .iter()
    .map(|c| home.join(c))
    .collect();
    candidates.extend(crate::proton::xdg_steam_root(xdg_data_home));
    candidates
}

/// Whether two existing paths resolve to the same directory (symlink aliases
/// such as `~/.steam/steam` → `~/.local/share/Steam`).
fn same_canonical(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
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

/// Get the depot download commands and info for the frontend wizard.
pub fn get_depot_download_info(
    game_id: &str,
    bottle: &crate::bottles::Bottle,
    target: Option<&str>,
) -> Result<DepotDownloadInfo> {
    if game_id != "skyrimse" {
        return Err(DowngraderError::Other(
            "Depot download is only supported for Skyrim SE".to_string(),
        ));
    }

    let steam_dir = depot_search_dirs(bottle).into_iter().next().ok_or_else(|| {
        DowngraderError::Other(
            "No Steam installation found (checked the bottle and the native Steam client)"
                .to_string(),
        )
    })?;

    let build = downgrade_target(target);
    let commands = download_depot_commands(build);
    let expected_path = get_depot_download_path(&steam_dir, SKYRIM_APP_ID, SKYRIM_DEPOT_ID)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();

    Ok(DepotDownloadInfo {
        command: commands.join("\n"),
        commands,
        target_version: build.version.to_string(),
        steam_uri: "steam://open/console".to_string(),
        expected_path: expected_path.to_string_lossy().into_owned(),
    })
}

/// Apply a depot download to a game installation.
///
/// Copies all files from `depot_dir` (downloaded by DepotDownloader) over the
/// game installation at `game_path`. Backs up the current game files first.
/// Works for ANY game, not just Skyrim SE.
pub fn apply_depot_to_game(
    game_path: &Path,
    depot_dir: &Path,
    game_id: &str,
) -> Result<u64> {
    use walkdir::WalkDir;

    if !depot_dir.exists() || !depot_dir.is_dir() {
        return Err(DowngraderError::Other(format!(
            "Depot directory not found: {}",
            depot_dir.display()
        )));
    }

    // Back up current version before overwriting
    info!(
        "Applying depot to game: {} → {}",
        depot_dir.display(),
        game_path.display()
    );
    match cache_current_version(game_path, game_id) {
        Ok(cached) => info!("Backed up current version: {}", cached.version),
        Err(e) => warn!("Failed to back up current version: {} (continuing)", e),
    }

    // Copy all files from depot over game installation
    let mut files_copied: u64 = 0;
    for entry in WalkDir::new(depot_dir).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }

        let rel_path = match entry.path().strip_prefix(depot_dir) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let dest = game_path.join(rel_path);
        if let Some(parent) = dest.parent() {
            if !parent.exists() {
                let _ = std::fs::create_dir_all(parent);
            }
        }

        match std::fs::copy(entry.path(), &dest) {
            Ok(_) => files_copied += 1,
            Err(e) => warn!(
                "Failed to copy depot file {} → {}: {}",
                entry.path().display(),
                dest.display(),
                e
            ),
        }
    }

    info!(
        "Applied depot to game {}: {} files copied",
        game_id, files_copied
    );
    Ok(files_copied)
}

/// Automate the depot download by opening Steam console and typing the command.
/// macOS only — uses osascript to send keystrokes to Steam.
/// Returns Ok(true) if the command was sent successfully, Ok(false) if automation unavailable.
///
/// On Linux there is no equivalent (no AppleScript, no portable Steam-window
/// keystroke automation). Instead of silently returning Ok(false), we surface
/// a clear error pointing the user at the existing DepotDownloader integration
/// and giving them the exact Steam console command they can paste manually.
pub fn send_depot_command_to_steam(target: Option<&str>) -> std::result::Result<bool, String> {
    let commands = download_depot_commands(downgrade_target(target));
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

        // Type each depot command via AppleScript keystrokes. Steam queues
        // the downloads and runs them one after another.
        let typed: String = commands
            .iter()
            .map(|c| format!("keystroke \"{c}\"\n    delay 0.3\n    keystroke return\n    delay 1\n"))
            .collect();
        let script = format!(
            "tell application \"Steam\" to activate\n\
             delay 1\n\
             tell application \"System Events\"\n    {typed}end tell"
        );

        let output = ProcessCommand::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|e| format!("AppleScript execution failed: {}", e))?;

        if output.status.success() {
            info!("Sent {} depot download commands to the Steam console", commands.len());
            Ok(true)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("AppleScript automation failed: {}", stderr);
            Ok(false)
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let message = format!(
            "Steam console automation is not supported on this platform. \
             Open the Steam console (steam://open/console) and run these commands: {}",
            commands.join(" ; ")
        );
        warn!("{}", message);
        Err(message)
    }
}

// ---------------------------------------------------------------------------
// Full (three-depot) downgrade
// ---------------------------------------------------------------------------

/// Depot 489831: core BSAs and the free Creation Club content.
pub const SKYRIM_DEPOT_CORE: u32 = 489831;
/// Depot 489832: master files, the remaining BSAs, launcher and default INIs.
pub const SKYRIM_DEPOT_DATA: u32 = 489832;

/// One Steam build of Skyrim SE: the manifest of each content depot.
///
/// Since 1.6.1130 Bethesda patches change the master files and BSAs as well
/// as `SkyrimSE.exe` (1.7.x also changed the Address Library format), so
/// swapping only the exe leaves the old exe running newer data. A correct
/// downgrade replaces all three depots.
#[derive(Clone, Copy, Debug)]
pub struct SkyrimBuild {
    pub version: &'static str,
    /// `(depot, manifest)` for the core, data and exe depots.
    pub manifests: [(u32, &'static str); 3],
}

/// Known Skyrim SE Steam builds (manifest IDs from the community manifest
/// list, Nexus article 6536, cross-checked with Wabbajack's indexed game
/// files and this Mac's Steam depotcache).
pub const SKYRIM_BUILDS: &[SkyrimBuild] = &[
    SkyrimBuild {
        version: "1.7.104",
        manifests: [
            (SKYRIM_DEPOT_CORE, "4940892828028256588"),
            (SKYRIM_DEPOT_DATA, "5728778377666085157"),
            (SKYRIM_DEPOT_ID, "4886117324142477814"),
        ],
    },
    SkyrimBuild {
        version: "1.6.1170",
        manifests: [
            (SKYRIM_DEPOT_CORE, "8442952117333549665"),
            (SKYRIM_DEPOT_DATA, "8042843504692938467"),
            (SKYRIM_DEPOT_ID, "1914580699073641964"),
        ],
    },
    SkyrimBuild {
        version: "1.5.97",
        manifests: [
            (SKYRIM_DEPOT_CORE, "7848722008564294070"),
            (SKYRIM_DEPOT_DATA, "8702665189575304780"),
            (SKYRIM_DEPOT_ID, SKYRIM_SE_MANIFEST),
        ],
    },
];

/// The known build for a version string such as "1.6.1170" or
/// "1.5.97 (Special Edition)".
pub fn skyrim_build(version: &str) -> Option<&'static SkyrimBuild> {
    let wanted = version.split_whitespace().next().unwrap_or(version);
    SKYRIM_BUILDS.iter().find(|b| b.version == wanted)
}

/// The downgrade target for a request: a known build version, or the
/// classic SE build (1.5.97) by default.
pub fn downgrade_target(requested: Option<&str>) -> &'static SkyrimBuild {
    requested
        .and_then(skyrim_build)
        .or_else(|| skyrim_build(TARGET_VERSION))
        .expect("1.5.97 is in SKYRIM_BUILDS")
}

/// Steam console commands that download every depot of `build`.
pub fn download_depot_commands(build: &SkyrimBuild) -> Vec<String> {
    build
        .manifests
        .iter()
        .map(|(depot, manifest)| format!("download_depot {SKYRIM_APP_ID} {depot} {manifest}"))
        .collect()
}

/// Seconds a depot folder must go without writes before it counts as
/// finished; Steam writes files in place while `download_depot` runs.
const DEPOT_SETTLE_SECS: u64 = 60;

/// Whether every depot of `build` has been downloaded by `download_depot`
/// into one of `steam_dirs`. Returns the three depot directories (core, data,
/// exe) or a reason the set isn't ready.
///
/// Checks: Steam fetched the exact manifests (`depotcache/<depot>_<manifest>.manifest`),
/// the exe depot's `SkyrimSE.exe` is the target version, the data depot has
/// `Data/Skyrim.esm`, the core depot has BSAs, and nothing changed in the
/// last minute.
pub fn full_depot_set_ready(
    steam_dirs: &[PathBuf],
    build: &SkyrimBuild,
) -> std::result::Result<Vec<PathBuf>, String> {
    let mut dirs = Vec::with_capacity(3);
    for (depot, manifest) in build.manifests {
        let found = steam_dirs.iter().find_map(|steam| {
            let dir = get_depot_download_path(steam, SKYRIM_APP_ID, depot);
            let fetched = steam
                .join("depotcache")
                .join(format!("{depot}_{manifest}.manifest"))
                .is_file();
            (dir.is_dir() && fetched).then_some(dir)
        });
        dirs.push(found.ok_or_else(|| {
            format!("depot {depot} (manifest {manifest}) hasn't been downloaded yet")
        })?);
    }

    let exe = find_skyrim_exe(&dirs[2]).map_err(|_| "SkyrimSE.exe isn't in the exe depot yet".to_string())?;
    let (version, _) = classify_exe(&exe).map_err(|e| e.to_string())?;
    if version.split_whitespace().next() != Some(build.version) {
        return Err(format!(
            "the downloaded SkyrimSE.exe is {version}, not {}",
            build.version
        ));
    }
    if !dirs[1].join("Data").join("Skyrim.esm").is_file() {
        return Err("Data/Skyrim.esm isn't in the data depot yet".to_string());
    }
    let has_bsa = fs::read_dir(dirs[0].join("Data"))
        .map(|rd| {
            rd.flatten().any(|e| {
                e.path()
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("bsa"))
            })
        })
        .unwrap_or(false);
    if !has_bsa {
        return Err("the core depot has no BSAs yet".to_string());
    }

    let settle = std::time::Duration::from_secs(DEPOT_SETTLE_SECS);
    for dir in &dirs {
        if let Some(age) = newest_file_age(dir) {
            if age < settle {
                return Err(format!("{} is still being written", dir.display()));
            }
        }
    }
    Ok(dirs)
}

/// Time since the most recent file modification under `dir`.
fn newest_file_age(dir: &Path) -> Option<std::time::Duration> {
    let now = std::time::SystemTime::now();
    WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok()?.modified().ok())
        .filter_map(|t| now.duration_since(t).ok())
        .min()
}

/// What a full downgrade changed, saved next to the backup so it can be undone.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FullDowngradeRecord {
    pub game_path: String,
    pub from_version: String,
    pub to_version: String,
    pub created_at: String,
    /// Game-relative paths whose originals were moved into the backup.
    pub replaced: Vec<String>,
    /// Game-relative paths that didn't exist before the downgrade.
    pub added: Vec<String>,
    /// Files outside the game folder moved aside (absolute path, backup-relative path).
    pub moved_aside: Vec<(String, String)>,
}

const DOWNGRADE_RECORD: &str = "downgrade.json";

/// Directory for backups of one game install, keyed by its canonical path.
fn install_backup_dir(game_path: &Path, game_id: &str) -> PathBuf {
    use sha2::{Digest, Sha256};
    let canonical = game_path.canonicalize().unwrap_or_else(|_| game_path.to_path_buf());
    let key = Sha256::digest(canonical.to_string_lossy().as_bytes());
    let key: String = key.iter().take(6).map(|b| format!("{b:02x}")).collect();
    version_cache_dir(game_id).join("full-backups").join(key)
}

/// Move `from` to `to`, falling back to copy + delete across filesystems.
fn move_file(from: &Path, to: &Path) -> io::Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) => {
            fs::copy(from, to)?;
            fs::remove_file(from)
        }
    }
}

/// Whether two files have identical contents (size first, then hash).
fn same_contents(a: &Path, b: &Path) -> bool {
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(ma), Ok(mb)) if ma.len() == mb.len() => {
            matches!((compute_sha256(a), compute_sha256(b)), (Ok(x), Ok(y)) if x == y)
        }
        _ => false,
    }
}

/// Replace a game install with the files of three downloaded depots.
///
/// Every game file the depots overwrite is first moved (not copied) into
/// `backup_root`, so the downgrade can be undone with
/// [`restore_full_downgrade`] and costs no extra disk space on the same
/// volume. Identical files are left alone. `aside` lists files outside the
/// game folder to move into the backup too (a 1.7.x `ContentCatalog.txt`
/// crashes older builds). Symlinks in the game folder are never followed or
/// overwritten.
pub fn apply_full_downgrade(
    game_path: &Path,
    depot_dirs: &[PathBuf],
    backup_root: &Path,
    from_version: &str,
    to_version: &str,
    aside: &[PathBuf],
) -> Result<FullDowngradeRecord> {
    if backup_root.join(DOWNGRADE_RECORD).exists() {
        return Err(DowngraderError::Other(format!(
            "a downgrade backup already exists at {}; restore it first",
            backup_root.display()
        )));
    }
    let game_files = backup_root.join("game");
    let mut record = FullDowngradeRecord {
        game_path: game_path.to_string_lossy().into_owned(),
        from_version: from_version.to_string(),
        to_version: to_version.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        ..Default::default()
    };

    // Save the record even when a step fails part way, so whatever was
    // already moved into the backup can still be restored.
    let outcome = (|| -> Result<()> {
        for depot in depot_dirs {
            for entry in WalkDir::new(depot).into_iter().filter_map(|e| e.ok()) {
                // Depot contents come from Steam, but never follow links out of it.
                if !entry.file_type().is_file() {
                    continue;
                }
                let Ok(rel) = entry.path().strip_prefix(depot) else {
                    continue;
                };
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                if !crate::staging::is_safe_relative_path(&rel_str) {
                    warn!("Skipping unsafe depot path {}", rel_str);
                    continue;
                }
                let dest = game_path.join(rel);
                let dest_meta = fs::symlink_metadata(&dest);
                if dest_meta.as_ref().is_ok_and(|m| m.file_type().is_symlink()) {
                    warn!("Not overwriting symlink {}", dest.display());
                    continue;
                }
                if dest_meta.is_ok() {
                    if same_contents(entry.path(), &dest) {
                        continue;
                    }
                    move_file(&dest, &game_files.join(rel))?;
                    record.replaced.push(rel_str);
                } else {
                    record.added.push(rel_str);
                }
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(entry.path(), &dest)?;
            }
        }

        for (i, path) in aside.iter().enumerate() {
            if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file()) {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let rel = format!("aside/{i}-{name}");
                move_file(path, &backup_root.join(&rel))?;
                record.moved_aside.push((path.to_string_lossy().into_owned(), rel));
            }
        }
        Ok(())
    })();
    let changed = !(record.replaced.is_empty() && record.added.is_empty() && record.moved_aside.is_empty());
    if changed || outcome.is_ok() {
        fs::create_dir_all(backup_root)?;
        fs::write(
            backup_root.join(DOWNGRADE_RECORD),
            serde_json::to_vec_pretty(&record).map_err(|e| DowngraderError::Other(e.to_string()))?,
        )?;
    }
    outcome?;
    info!(
        "Full downgrade {} -> {}: {} files replaced, {} added, {} moved aside (backup {})",
        from_version,
        to_version,
        record.replaced.len(),
        record.added.len(),
        record.moved_aside.len(),
        backup_root.display()
    );
    Ok(record)
}

/// Undo [`apply_full_downgrade`]: remove files it added and move the
/// originals back. The backup is deleted once everything is restored.
pub fn restore_full_downgrade(backup_root: &Path) -> Result<FullDowngradeRecord> {
    let record: FullDowngradeRecord = serde_json::from_slice(&fs::read(
        backup_root.join(DOWNGRADE_RECORD),
    )?)
    .map_err(|e| DowngraderError::Other(format!("unreadable downgrade record: {e}")))?;
    let game_path = PathBuf::from(&record.game_path);

    for rel in record.added.iter().chain(record.replaced.iter()) {
        if !crate::staging::is_safe_relative_path(rel) {
            return Err(DowngraderError::Other(format!("unsafe path in record: {rel}")));
        }
    }
    for rel in &record.added {
        let path = game_path.join(rel);
        if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_file()) {
            fs::remove_file(&path)?;
        }
    }
    for rel in &record.replaced {
        let backup = backup_root.join("game").join(rel);
        let dest = game_path.join(rel);
        if fs::symlink_metadata(&dest).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(DowngraderError::Other(format!("{} is now a symlink", dest.display())));
        }
        if backup.is_file() {
            if dest.exists() {
                fs::remove_file(&dest)?;
            }
            move_file(&backup, &dest)?;
        }
    }
    for (original, rel) in &record.moved_aside {
        let backup = backup_root.join(rel);
        if backup.is_file() && !Path::new(original).exists() {
            move_file(&backup, Path::new(original))?;
        }
    }
    fs::remove_dir_all(backup_root)?;
    info!(
        "Restored Skyrim {} from downgrade backup ({} files)",
        record.from_version,
        record.replaced.len()
    );
    Ok(record)
}

/// The backup directory for this install, if a full downgrade can be undone.
pub fn full_downgrade_backup(game_path: &Path, game_id: &str) -> Option<PathBuf> {
    let dir = install_backup_dir(game_path, game_id);
    dir.join(DOWNGRADE_RECORD).is_file().then_some(dir)
}

/// Backup location for a new full downgrade of this install.
pub fn new_full_downgrade_backup(game_path: &Path, game_id: &str) -> PathBuf {
    install_backup_dir(game_path, game_id)
}

/// Creations catalog files written by Skyrim 1.6.1130+ that older builds
/// can't read: next to the game and in the prefix's `AppData/Local`.
pub fn content_catalog_candidates(game_path: &Path, appdata_local: &Path) -> Vec<PathBuf> {
    vec![
        game_path.join("ContentCatalog.txt"),
        appdata_local.join("Skyrim Special Edition").join("ContentCatalog.txt"),
    ]
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

    // ---- Full (three-depot) downgrade ----

    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn builds_cover_all_three_depots_and_default_to_1_5_97() {
        for build in SKYRIM_BUILDS {
            let depots: Vec<u32> = build.manifests.iter().map(|(d, _)| *d).collect();
            assert_eq!(depots, vec![SKYRIM_DEPOT_CORE, SKYRIM_DEPOT_DATA, SKYRIM_DEPOT_ID]);
        }
        assert_eq!(downgrade_target(None).version, "1.5.97");
        assert_eq!(downgrade_target(Some("full")).version, "1.5.97");
        assert_eq!(downgrade_target(Some("1.6.1170")).version, "1.6.1170");
        assert_eq!(skyrim_build("1.5.97 (Special Edition)").unwrap().version, "1.5.97");
        assert_eq!(
            download_depot_commands(skyrim_build("1.6.1170").unwrap()),
            vec![
                "download_depot 489830 489831 8442952117333549665",
                "download_depot 489830 489832 8042843504692938467",
                "download_depot 489830 489833 1914580699073641964",
            ]
        );
    }

    #[test]
    fn depot_set_not_ready_until_steam_fetched_every_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let steam = tmp.path().join("Steam");
        let build = skyrim_build("1.6.1170").unwrap();
        for (depot, _) in build.manifests {
            fs::create_dir_all(get_depot_download_path(&steam, SKYRIM_APP_ID, depot)).unwrap();
        }
        // Content folders alone (e.g. left over from another manifest) don't count.
        let err = full_depot_set_ready(&[steam.clone()], build).unwrap_err();
        assert!(err.contains("489831"), "{err}");

        for (depot, manifest) in build.manifests {
            write(&steam.join(format!("depotcache/{depot}_{manifest}.manifest")), "x");
        }
        // Manifests fetched, but the exe depot has no SkyrimSE.exe yet.
        let err = full_depot_set_ready(&[steam.clone()], build).unwrap_err();
        assert!(err.contains("SkyrimSE.exe"), "{err}");

        // A non-PE exe can't be the target build.
        write(
            &get_depot_download_path(&steam, SKYRIM_APP_ID, SKYRIM_DEPOT_ID).join("SkyrimSE.exe"),
            "not a pe",
        );
        let err = full_depot_set_ready(&[steam], build).unwrap_err();
        assert!(err.contains("not 1.6.1170"), "{err}");
    }

    fn fake_install(root: &Path) -> (PathBuf, Vec<PathBuf>, PathBuf) {
        let game = root.join("Skyrim Special Edition");
        write(&game.join("SkyrimSE.exe"), "exe 1.7.104");
        write(&game.join("Data/Skyrim.esm"), "esm 1.7.104");
        write(&game.join("Data/Skyrim - Misc.bsa"), "same in both");
        write(&game.join("Data/MyMod.esp"), "a mod file, not in any depot");

        let core = root.join("depot_489831");
        write(&core.join("Data/Skyrim - Misc.bsa"), "same in both");
        write(&core.join("Data/Skyrim - Old.bsa"), "only in old build");
        let data = root.join("depot_489832");
        write(&data.join("Data/Skyrim.esm"), "esm 1.6.1170");
        let exe = root.join("depot_489833");
        write(&exe.join("SkyrimSE.exe"), "exe 1.6.1170");

        let catalog = root.join("AppData/Local/Skyrim Special Edition/ContentCatalog.txt");
        write(&catalog, "1.7 creations catalog");
        (game, vec![core, data, exe], catalog)
    }

    #[test]
    fn full_downgrade_applies_every_depot_and_restores_exactly() {
        let tmp = tempfile::tempdir().unwrap();
        let (game, depots, catalog) = fake_install(tmp.path());
        let backup = tmp.path().join("backup");

        let record = apply_full_downgrade(
            &game, &depots, &backup, "1.7.104", "1.6.1170", &[catalog.clone()],
        )
        .unwrap();

        assert_eq!(read(&game.join("SkyrimSE.exe")), "exe 1.6.1170");
        assert_eq!(read(&game.join("Data/Skyrim.esm")), "esm 1.6.1170");
        assert_eq!(read(&game.join("Data/Skyrim - Old.bsa")), "only in old build");
        assert_eq!(read(&game.join("Data/MyMod.esp")), "a mod file, not in any depot");
        assert!(!catalog.exists(), "1.7 ContentCatalog.txt moved aside");
        let mut replaced = record.replaced.clone();
        replaced.sort();
        assert_eq!(replaced, vec!["Data/Skyrim.esm", "SkyrimSE.exe"], "identical files untouched");
        assert_eq!(record.added, vec!["Data/Skyrim - Old.bsa"]);
        assert_eq!(read(&backup.join("game/SkyrimSE.exe")), "exe 1.7.104");

        // A second downgrade can't overwrite the only backup.
        assert!(apply_full_downgrade(&game, &depots, &backup, "1.6.1170", "1.5.97", &[]).is_err());

        restore_full_downgrade(&backup).unwrap();
        assert_eq!(read(&game.join("SkyrimSE.exe")), "exe 1.7.104");
        assert_eq!(read(&game.join("Data/Skyrim.esm")), "esm 1.7.104");
        assert!(!game.join("Data/Skyrim - Old.bsa").exists());
        assert_eq!(read(&game.join("Data/MyMod.esp")), "a mod file, not in any depot");
        assert_eq!(read(&catalog), "1.7 creations catalog");
        assert!(!backup.exists(), "backup removed after a full restore");
    }

    #[cfg(unix)]
    #[test]
    fn full_downgrade_never_overwrites_through_a_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let (game, depots, _) = fake_install(tmp.path());
        let outside = tmp.path().join("outside.esm");
        write(&outside, "outside the game");
        fs::remove_file(game.join("Data/Skyrim.esm")).unwrap();
        std::os::unix::fs::symlink(&outside, game.join("Data/Skyrim.esm")).unwrap();

        apply_full_downgrade(&game, &depots, &tmp.path().join("backup"), "1.7.104", "1.6.1170", &[])
            .unwrap();
        assert_eq!(read(&outside), "outside the game");
    }

    #[test]
    fn steam_play_and_gamehub_prefixes_only_search_host_steam() {
        let tmp = tempfile::tempdir().unwrap();
        let pfx = tmp.path().join("steamapps/compatdata/489830/pfx");
        let bridge = pfx.join("drive_c/Program Files (x86)/Steam");
        fs::create_dir_all(&bridge).unwrap();
        for source in ["NotProton", "Proton", "GameHub"] {
            let bottle = crate::bottles::Bottle {
                name: "Skyrim".into(),
                path: pfx.clone(),
                source: source.into(),
            };
            assert!(
                !depot_search_dirs(&bottle).iter().any(|d| d == &bridge),
                "{source} must not use the in-prefix Steam"
            );
        }
        let crossover = crate::bottles::Bottle {
            name: "Steam".into(),
            path: pfx.clone(),
            source: "CrossOver".into(),
        };
        assert_eq!(depot_search_dirs(&crossover).first(), Some(&bridge));
    }

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

    #[test]
    fn native_steam_candidates_cover_flatpak_snap_and_xdg() {
        let home = Path::new("/home/deck");
        let xdg = std::ffi::OsStr::new("/data/xdg");
        let c = native_steam_candidates(home, Some(xdg));
        for expected in [
            "/home/deck/.steam/steam",
            "/home/deck/.local/share/Steam",
            "/home/deck/.steam/root",
            "/home/deck/.var/app/com.valvesoftware.Steam/data/Steam",
            "/home/deck/.var/app/com.valvesoftware.Steam/.local/share/Steam",
            "/home/deck/snap/steam/common/.local/share/Steam",
            "/data/xdg/Steam",
        ] {
            assert!(c.contains(&PathBuf::from(expected)), "missing {expected}");
        }
        // Relative XDG values are ignored per the XDG Base Directory spec.
        let rel = native_steam_candidates(home, Some(std::ffi::OsStr::new("rel/xdg")));
        assert!(!rel.iter().any(|p| p.ends_with("rel/xdg/Steam")));
    }

    #[cfg(unix)]
    #[test]
    fn same_canonical_collapses_symlink_alias() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real");
        fs::create_dir_all(&real).unwrap();
        let alias = tmp.path().join("alias");
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        assert!(same_canonical(&real, &alias));
        assert!(!same_canonical(&real, tmp.path()));
    }
}

#[cfg(test)]
mod live_tests {
    /// Live check on this machine's real bottles (read-only):
    /// `cargo test --lib live_downgrade_targets -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_downgrade_targets() {
        crate::plugins::skyrim_se::register();
        for bottle in crate::bottles::detect_bottles() {
            let games = crate::games::detect_games(&bottle);
            eprintln!("BOTTLE {} [{}] {} -> {:?}", bottle.name, bottle.source, bottle.path.display(),
                games.iter().map(|g| g.game_id.clone()).collect::<Vec<_>>());
            for game in games {
                if game.game_id != "skyrimse" {
                    continue;
                }
                let path = std::path::Path::new(&game.game_path);
                eprintln!(
                    "{} [{}] game={} version={:?} depot_dirs={:?} backup={}",
                    bottle.name,
                    bottle.source,
                    game.game_path.display(),
                    super::detect_skyrim_version(path).map(|s| s.current_version).ok(),
                    super::depot_search_dirs(&bottle),
                    super::new_full_downgrade_backup(path, "skyrimse").display(),
                );
            }
        }
    }
}
