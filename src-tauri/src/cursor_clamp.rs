//! Legacy cursor fix — replaced by Wine registry keys in display_fix.rs.
//!
//! The old approach used invasive macOS hacks (Dock suppression, Hot Corner
//! zeroing, CGEventTap Y-clamping) which prevented users from accessing their
//! desktop while a game was running and required crash recovery.
//!
//! The new approach sets Wine registry keys (DXGrab, MouseWarpOverride,
//! CaptureDisplaysForFullscreen) that fix the double-cursor bug at the Wine
//! driver level — no runtime state, no system-wide side effects, no
//! Accessibility permission needed.
//!
//! This module is kept for ONE release cycle to clean up leftover Dock/Hot
//! Corner state from users upgrading from previous versions. After that it
//! can be fully removed.

// --- Non-macOS: all no-ops ---

#[cfg(not(target_os = "macos"))]
pub fn recover_dock_if_needed() {}

// --- macOS: crash recovery only ---

#[cfg(target_os = "macos")]
pub fn recover_dock_if_needed() {
    use log::{info, warn};

    let delay = read_dock_autohide_delay();
    let backup = corner_backup_path();
    let corners_backup = std::fs::read_to_string(&backup).ok();

    if delay != 86400.0 && corners_backup.is_none() {
        return;
    }

    warn!("cursor_clamp: detected leftover suppression from previous version — cleaning up");

    // Restore Hot Corners from backup file
    if let Some(originals) = corners_backup {
        let values: Vec<i32> = originals
            .trim()
            .split(',')
            .filter_map(|s| s.parse().ok())
            .collect();

        if values.len() == 4 {
            info!(
                "cursor_clamp: recovering Hot Corners from backup ({})",
                originals.trim()
            );
            for (key, val) in CORNER_KEYS.iter().zip(values.iter()) {
                if *val == 0 {
                    let _ = std::process::Command::new("defaults")
                        .args(["delete", "com.apple.dock", key])
                        .status();
                } else {
                    let _ = std::process::Command::new("defaults")
                        .args(["write", "com.apple.dock", key, "-int", &val.to_string()])
                        .status();
                }
            }
        }
        let _ = std::fs::remove_file(&backup);
    }

    // Restore Dock
    if delay == 86400.0 {
        let _ = std::process::Command::new("defaults")
            .args(["write", "com.apple.dock", "autohide", "-bool", "false"])
            .status();
        let _ = std::process::Command::new("defaults")
            .args(["delete", "com.apple.dock", "autohide-delay"])
            .status();
    }

    let _ = std::process::Command::new("killall")
        .arg("Dock")
        .status();
    info!("cursor_clamp: cleaned up leftover state from previous version");
}

#[cfg(target_os = "macos")]
const CORNER_KEYS: [&str; 4] = [
    "wvous-tl-corner",
    "wvous-tr-corner",
    "wvous-bl-corner",
    "wvous-br-corner",
];

#[cfg(target_os = "macos")]
fn corner_backup_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join(".corkscrew-hot-corners-backup")
}

#[cfg(target_os = "macos")]
fn read_dock_autohide_delay() -> f64 {
    std::process::Command::new("defaults")
        .args(["read", "com.apple.dock", "autohide-delay"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .parse::<f64>()
                    .ok()
            } else {
                None
            }
        })
        .unwrap_or(0.0)
}
