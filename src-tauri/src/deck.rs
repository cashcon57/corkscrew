//! Steam Deck-specific utilities and optimizations.
//!
//! The Steam Deck is THE Linux modding platform: 16GB shared RAM, 1280x800
//! screen, gamepad input, sleep/wake interrupts. This module provides
//! Deck-aware defaults and utilities.

use log::info;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Steam Deck hardware profile.
#[derive(Debug, Clone, Serialize)]
pub struct DeckProfile {
    /// Whether we're running on a Steam Deck
    pub is_deck: bool,
    /// Total RAM in MB (typically 16384)
    pub total_ram_mb: u64,
    /// Whether an SD card is mounted
    pub sd_card_mounted: bool,
    /// SD card mount path (if mounted)
    pub sd_card_path: Option<PathBuf>,
    /// SD card free space in bytes
    pub sd_card_free_bytes: Option<u64>,
    /// Internal storage free space in bytes
    pub internal_free_bytes: u64,
    /// Whether we're in Gaming Mode (vs Desktop Mode)
    pub gaming_mode: bool,
}

/// Detect Steam Deck profile.
pub fn detect_deck_profile() -> DeckProfile {
    let is_deck = is_steam_deck();

    let total_ram_mb = get_total_ram_mb();
    let sd_card = detect_sd_card();
    let internal_free = get_free_space(Path::new("/home")).unwrap_or(0);
    let gaming_mode = is_gaming_mode();

    let profile = DeckProfile {
        is_deck,
        total_ram_mb,
        sd_card_mounted: sd_card.is_some(),
        sd_card_path: sd_card.as_ref().map(|(p, _)| p.clone()),
        sd_card_free_bytes: sd_card.map(|(_, free)| free),
        internal_free_bytes: internal_free,
        gaming_mode,
    };

    if is_deck {
        info!(
            "Steam Deck detected: {}MB RAM, SD card: {}, Gaming Mode: {}",
            total_ram_mb, profile.sd_card_mounted, gaming_mode
        );
    }

    profile
}

/// Memory-conscious defaults for Steam Deck.
#[derive(Debug, Clone, Serialize)]
pub struct DeckDefaults {
    /// Number of rayon threads (lower on Deck to save RAM)
    pub rayon_threads: usize,
    /// Download concurrency (lower on Deck)
    pub download_concurrency: usize,
    /// Whether to always use BSA cache (disk) instead of RAM
    pub always_use_bsa_cache: bool,
    /// Whether to run malloc_trim aggressively between phases
    pub aggressive_malloc_trim: bool,
    /// Minimum touch target size in pixels
    pub min_touch_target_px: u32,
}

/// Get recommended defaults based on whether we're on Deck.
pub fn get_defaults() -> DeckDefaults {
    if is_steam_deck() {
        DeckDefaults {
            rayon_threads: 4,
            download_concurrency: 4,
            always_use_bsa_cache: true,
            aggressive_malloc_trim: true,
            min_touch_target_px: 48,
        }
    } else {
        DeckDefaults {
            rayon_threads: std::thread::available_parallelism()
                .map(|n| n.get().min(8))
                .unwrap_or(4),
            download_concurrency: 8,
            always_use_bsa_cache: false,
            aggressive_malloc_trim: false,
            min_touch_target_px: 32,
        }
    }
}

/// Check if running on Steam Deck.
///
/// Uses multiple detection methods:
/// 1. DMI board vendor (most reliable)
/// 2. /etc/os-release SteamOS marker
/// 3. Steam Deck-specific hardware paths
fn is_steam_deck() -> bool {
    // Method 1: DMI board vendor
    if let Ok(vendor) = std::fs::read_to_string("/sys/devices/virtual/dmi/id/board_vendor") {
        if vendor.trim() == "Valve" {
            return true;
        }
    }

    // Method 2: SteamOS in os-release
    if let Ok(os_release) = std::fs::read_to_string("/etc/os-release") {
        if os_release.contains("SteamOS") || os_release.contains("steamos") {
            return true;
        }
    }

    // Method 3: Deck-specific device
    if Path::new("/sys/class/hwmon").is_dir() {
        if let Ok(entries) = std::fs::read_dir("/sys/class/hwmon") {
            for entry in entries.flatten() {
                let name_path = entry.path().join("name");
                if let Ok(name) = std::fs::read_to_string(&name_path) {
                    if name.trim() == "jupiter" || name.trim() == "galileo" {
                        return true;
                    }
                }
            }
        }
    }

    false
}

/// Detect SD card mount point and free space.
fn detect_sd_card() -> Option<(PathBuf, u64)> {
    // Steam Deck mounts SD card at /run/media/mmcblk0p1
    let deck_sd = PathBuf::from("/run/media/mmcblk0p1");
    if deck_sd.is_dir() {
        if let Some(free) = get_free_space(&deck_sd) {
            return Some((deck_sd, free));
        }
    }

    // Also check /run/media/{user}/ for non-Deck Linux
    if let Some(home) = dirs::home_dir() {
        if let Some(user) = home.file_name() {
            let media_dir = PathBuf::from("/run/media").join(user);
            if media_dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&media_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_dir() {
                            if let Some(free) = get_free_space(&path) {
                                return Some((path, free));
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

/// Get free space on a path's filesystem.
fn get_free_space(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;

        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };

        let result = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
        if result == 0 {
            Some(stat.f_bavail as u64 * stat.f_frsize as u64)
        } else {
            None
        }
    }

    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// Get total system RAM in MB.
fn get_total_ram_mb() -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") {
            for line in meminfo.lines() {
                if line.starts_with("MemTotal:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if let Some(kb_str) = parts.get(1) {
                        if let Ok(kb) = kb_str.parse::<u64>() {
                            return kb / 1024;
                        }
                    }
                }
            }
        }
        0
    }

    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("sysctl")
            .arg("-n")
            .arg("hw.memsize")
            .output()
            .ok();
        if let Some(out) = output {
            if let Ok(s) = String::from_utf8(out.stdout) {
                if let Ok(bytes) = s.trim().parse::<u64>() {
                    return bytes / (1024 * 1024);
                }
            }
        }
        0
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        0
    }
}

/// Check if running in Steam Gaming Mode (vs Desktop Mode).
fn is_gaming_mode() -> bool {
    // In Gaming Mode, the session is managed by gamescope
    std::env::var("GAMESCOPE_WAYLAND_DISPLAY").is_ok()
        || std::env::var("SteamGamepadUI").is_ok()
}

/// Check if a deployment would cross device boundaries.
///
/// Returns true if source and dest are on different filesystems,
/// which means hardlinks won't work and copies are needed.
pub fn is_cross_device(source: &Path, dest: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        let source_dev = source.metadata().map(|m| m.dev()).ok();
        // If dest doesn't exist yet, check its parent directory
        let dest_dev = dest
            .metadata()
            .or_else(|_| {
                dest.parent()
                    .map(|p| p.metadata())
                    .unwrap_or_else(|| dest.metadata())
            })
            .map(|m| m.dev())
            .ok();

        match (source_dev, dest_dev) {
            (Some(s), Some(d)) => s != d,
            _ => false, // Can't determine — assume same device
        }
    }

    #[cfg(not(unix))]
    {
        let _ = (source, dest);
        false
    }
}

/// Get recommended staging/download location for Steam Deck.
///
/// Prefers internal SSD for staging (faster I/O), SD card for downloads (more space).
pub fn get_recommended_paths(game_dir: &Path) -> (PathBuf, PathBuf) {
    let profile = detect_deck_profile();

    let default_staging = game_dir
        .parent()
        .unwrap_or(game_dir)
        .join("corkscrew_staging");
    let default_download = dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("corkscrew/downloads");

    if profile.is_deck {
        // Prefer internal SSD for staging (fast random I/O)
        let staging = default_staging;

        // Use SD card for downloads if available and has enough space
        let download = if let (Some(sd_path), Some(sd_free)) =
            (&profile.sd_card_path, profile.sd_card_free_bytes)
        {
            if sd_free > 10_000_000_000 {
                // >10GB free
                sd_path.join("corkscrew/downloads")
            } else {
                default_download
            }
        } else {
            default_download
        };

        (staging, download)
    } else {
        (default_staging, default_download)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_defaults_non_deck() {
        // On CI/dev machines, should get standard defaults
        let defaults = get_defaults();
        assert!(defaults.rayon_threads >= 1);
        assert!(defaults.download_concurrency >= 1);
    }

    #[test]
    fn test_detect_profile() {
        let profile = detect_deck_profile();
        // Should at least not crash
        assert!(profile.total_ram_mb > 0 || !profile.is_deck);
    }

    #[test]
    fn test_cross_device_same() {
        let tmp = std::env::temp_dir();
        assert!(!is_cross_device(&tmp, &tmp));
    }

    #[test]
    fn test_get_total_ram_mb() {
        let ram = get_total_ram_mb();
        // On any real machine, should be > 0
        assert!(ram > 0);
    }

    #[test]
    fn test_get_free_space_tmp() {
        let free = get_free_space(Path::new("/tmp"));
        // /tmp should exist and have free space on any system
        assert!(free.is_some());
        assert!(free.unwrap() > 0);
    }

    #[test]
    fn test_recommended_paths() {
        let game_dir = PathBuf::from("/tmp/test_game");
        let (staging, download) = get_recommended_paths(&game_dir);
        // Should return valid paths regardless of platform
        assert!(!staging.as_os_str().is_empty());
        assert!(!download.as_os_str().is_empty());
    }

    #[test]
    fn test_deck_defaults_serializable() {
        let defaults = get_defaults();
        let json = serde_json::to_string(&defaults).unwrap();
        assert!(json.contains("rayon_threads"));
        assert!(json.contains("download_concurrency"));
    }

    #[test]
    fn test_deck_profile_serializable() {
        let profile = detect_deck_profile();
        let json = serde_json::to_string(&profile).unwrap();
        assert!(json.contains("is_deck"));
        assert!(json.contains("total_ram_mb"));
    }
}
