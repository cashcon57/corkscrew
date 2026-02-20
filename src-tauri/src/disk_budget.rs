//! Disk space budget and impact preview.
//!
//! Calculates staging directory size, deployment overhead, and available space.
//! Reports whether hardlinks are used (zero extra cost) or copies (2x cost).

use std::path::Path;

use serde::Serialize;
use walkdir::WalkDir;

use crate::deployer;
use crate::staging;

/// Minimum free disk space (5 GB) required before any write operation.
/// Filling a drive on SteamOS can cause boot loops; on macOS it degrades badly.
pub const MIN_FREE_BYTES: u64 = 5 * 1024 * 1024 * 1024;

/// Check whether a write of `bytes_needed` to the volume containing `path`
/// would leave at least `MIN_FREE_BYTES` free. Returns `Ok(())` on success,
/// or an `Err` describing the shortfall.
pub fn check_space_guard(path: &Path, bytes_needed: u64) -> std::result::Result<(), String> {
    let avail = available_space(path);
    if avail == 0 {
        return Ok(());
    }
    let remaining_after = avail.saturating_sub(bytes_needed);
    if remaining_after < MIN_FREE_BYTES {
        Err(format!(
            "Not enough disk space. This operation needs {} but only {} is available \
             ({} free after write, minimum {} required to keep your system safe).",
            format_bytes(bytes_needed),
            format_bytes(avail),
            format_bytes(remaining_after),
            format_bytes(MIN_FREE_BYTES),
        ))
    } else {
        Ok(())
    }
}

/// Disk budget summary for a game/bottle.
#[derive(Clone, Debug, Serialize)]
pub struct DiskBudget {
    /// Total size of all staging directories in bytes.
    pub staging_bytes: u64,
    /// Number of staged mod folders.
    pub staging_count: usize,
    /// Total deployed bytes (0 for hardlinks, staging_bytes for copies).
    pub deployment_bytes: u64,
    /// Whether hardlinks are used (true) or copies (false).
    pub uses_hardlinks: bool,
    /// Available bytes on the staging volume.
    pub available_bytes: u64,
    /// Available bytes on the game volume.
    pub game_available_bytes: u64,
    /// Total combined disk impact (staging + deployment overhead).
    pub total_impact_bytes: u64,
}

/// Calculate the total size of a directory tree.
pub fn dir_size(path: &Path) -> u64 {
    if !path.exists() {
        return 0;
    }
    WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

/// Get available disk space on the volume containing `path`.
pub fn available_space(path: &Path) -> u64 {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::mem::MaybeUninit;

        let path_str = path.to_string_lossy();
        let c_path = match CString::new(path_str.as_bytes()) {
            Ok(p) => p,
            Err(_) => return 0,
        };

        unsafe {
            let mut stat = MaybeUninit::<libc::statvfs>::uninit();
            if libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) == 0 {
                let stat = stat.assume_init();
                stat.f_bavail as u64 * stat.f_frsize
            } else {
                0
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        0
    }
}

/// Estimate disk impact of installing an archive of the given size.
pub fn estimate_install_impact(archive_size: u64, game_data_dir: &Path) -> InstallImpact {
    // Rough estimate: extracted size is ~2-3x archive size for typical mods
    let estimated_extracted = archive_size * 3;
    let uses_hardlinks = deployer::test_hardlink_support(game_data_dir, game_data_dir);
    let deployment_cost = if uses_hardlinks {
        0
    } else {
        estimated_extracted
    };

    InstallImpact {
        archive_size,
        estimated_staging_bytes: estimated_extracted,
        deployment_bytes: deployment_cost,
        total_bytes: estimated_extracted + deployment_cost,
        uses_hardlinks,
        game_available_bytes: available_space(game_data_dir),
    }
}

/// Pre-install size estimate for a single mod.
#[derive(Clone, Debug, Serialize)]
pub struct InstallImpact {
    pub archive_size: u64,
    pub estimated_staging_bytes: u64,
    pub deployment_bytes: u64,
    pub total_bytes: u64,
    pub uses_hardlinks: bool,
    pub game_available_bytes: u64,
}

/// Compute the full disk budget for a game/bottle.
pub fn compute_budget(game_id: &str, bottle_name: &str, game_data_dir: &Path) -> DiskBudget {
    let game_staging = staging::staging_base_dir(game_id, bottle_name);
    let staging_root = staging::staging_root();

    let mut staging_bytes = 0u64;
    let mut staging_count = 0usize;

    if game_staging.exists() {
        if let Ok(entries) = std::fs::read_dir(&game_staging) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    staging_bytes += dir_size(&entry.path());
                    staging_count += 1;
                }
            }
        }
    }

    let uses_hardlinks = deployer::test_hardlink_support(game_data_dir, game_data_dir);
    let deployment_bytes = if uses_hardlinks { 0 } else { staging_bytes };
    let total_impact = staging_bytes + deployment_bytes;

    DiskBudget {
        staging_bytes,
        staging_count,
        deployment_bytes,
        uses_hardlinks,
        available_bytes: available_space(&staging_root),
        game_available_bytes: available_space(game_data_dir),
        total_impact_bytes: total_impact,
    }
}

/// Format bytes into human-readable string.
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn dir_size_empty_directory() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(dir_size(tmp.path()), 0);
    }

    #[test]
    fn dir_size_with_files() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("a.txt"), "hello").unwrap();
        fs::write(tmp.path().join("b.txt"), "world!").unwrap();
        assert_eq!(dir_size(tmp.path()), 11); // 5 + 6
    }

    #[test]
    fn dir_size_nested_files() {
        let tmp = TempDir::new().unwrap();
        let sub = tmp.path().join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("file.bin"), vec![0u8; 1000]).unwrap();
        assert_eq!(dir_size(tmp.path()), 1000);
    }

    #[test]
    fn dir_size_nonexistent_returns_zero() {
        assert_eq!(dir_size(Path::new("/nonexistent/path/abc123")), 0);
    }

    #[test]
    fn format_bytes_various_sizes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(1048576), "1.0 MB");
        assert_eq!(format_bytes(1073741824), "1.0 GB");
    }

    #[test]
    fn format_bytes_large_values() {
        assert_eq!(format_bytes(10 * 1024 * 1024 * 1024), "10.0 GB");
        assert_eq!(format_bytes(2 * 1024 * 1024), "2.0 MB");
    }

    #[test]
    fn available_space_returns_value() {
        // Should return something > 0 for a real path
        let space = available_space(Path::new("/tmp"));
        assert!(space > 0, "Expected some available space on /tmp");
    }

    #[test]
    fn available_space_nonexistent_returns_zero() {
        let space = available_space(Path::new("/nonexistent/volume/xyz"));
        assert_eq!(space, 0);
    }

    #[test]
    fn estimate_install_impact_values() {
        let tmp = TempDir::new().unwrap();
        let impact = estimate_install_impact(1_000_000, tmp.path());
        assert_eq!(impact.archive_size, 1_000_000);
        assert_eq!(impact.estimated_staging_bytes, 3_000_000);
        // total should be staging + deployment
        assert_eq!(
            impact.total_bytes,
            impact.estimated_staging_bytes + impact.deployment_bytes
        );
    }

    #[test]
    fn estimate_install_impact_hardlink_zero_deploy_cost() {
        let tmp = TempDir::new().unwrap();
        let impact = estimate_install_impact(500_000, tmp.path());
        if impact.uses_hardlinks {
            assert_eq!(impact.deployment_bytes, 0);
        } else {
            assert_eq!(impact.deployment_bytes, impact.estimated_staging_bytes);
        }
    }

    #[test]
    fn disk_budget_empty_staging() {
        let tmp = TempDir::new().unwrap();
        let budget = compute_budget("testgame", "testbottle", tmp.path());
        assert_eq!(budget.staging_bytes, 0);
        assert_eq!(budget.staging_count, 0);
    }

    #[test]
    fn disk_budget_total_impact_correct() {
        let tmp = TempDir::new().unwrap();
        let budget = compute_budget("testgame", "testbottle", tmp.path());
        assert_eq!(
            budget.total_impact_bytes,
            budget.staging_bytes + budget.deployment_bytes
        );
    }

    #[test]
    fn space_guard_allows_small_write() {
        // Writing 1 byte to /tmp should always pass (plenty of space)
        assert!(check_space_guard(Path::new("/tmp"), 1).is_ok());
    }

    #[test]
    fn space_guard_rejects_impossible_write() {
        // Requesting more bytes than any drive could have
        let result = check_space_guard(Path::new("/tmp"), u64::MAX);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("Not enough disk space"));
    }

    #[test]
    fn space_guard_allows_nonexistent_path() {
        // Non-existent path returns 0 available space — guard allows it (can't determine)
        assert!(check_space_guard(Path::new("/nonexistent/xyz"), 1000).is_ok());
    }

    #[test]
    fn min_free_bytes_is_5gb() {
        assert_eq!(MIN_FREE_BYTES, 5 * 1024 * 1024 * 1024);
    }
}
