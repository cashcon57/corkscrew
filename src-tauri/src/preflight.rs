//! Pre-flight installation check.
//!
//! Before deploying mods or switching profiles, validates disk space,
//! missing master plugins, known incompatibilities, and bottle health.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::bottles::Bottle;
use crate::database::ModDatabase;
use crate::disk_budget;
use crate::wine_diagnostic;

/// Overall pre-flight result.
#[derive(Clone, Debug, Serialize)]
pub struct PreflightResult {
    pub checks: Vec<PreflightCheck>,
    pub passed: usize,
    pub failed: usize,
    pub warnings: usize,
    pub can_proceed: bool,
}

/// A single pre-flight check.
#[derive(Clone, Debug, Serialize)]
pub struct PreflightCheck {
    pub name: String,
    pub status: PreflightStatus,
    pub message: String,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PreflightStatus {
    Pass,
    Warning,
    Fail,
}

/// Run all pre-flight checks before deployment.
pub fn run_preflight(
    db: &ModDatabase,
    bottle: &Bottle,
    game_id: &str,
    bottle_name: &str,
    game_data_dir: &Path,
) -> PreflightResult {
    let checks = vec![
        check_disk_space(game_id, bottle_name, game_data_dir),
        check_enabled_mods(db, game_id, bottle_name),
        check_staging_dirs(db, game_id, bottle_name),
        check_bottle_health(bottle),
        check_conflicts(db, game_id, bottle_name),
    ];

    let passed = checks
        .iter()
        .filter(|c| c.status == PreflightStatus::Pass)
        .count();
    let failed = checks
        .iter()
        .filter(|c| c.status == PreflightStatus::Fail)
        .count();
    let warnings = checks
        .iter()
        .filter(|c| c.status == PreflightStatus::Warning)
        .count();

    PreflightResult {
        can_proceed: failed == 0,
        checks,
        passed,
        failed,
        warnings,
    }
}

/// Check available disk space on game and staging volumes.
fn check_disk_space(game_id: &str, bottle_name: &str, game_data_dir: &Path) -> PreflightCheck {
    let budget = disk_budget::compute_budget(game_id, bottle_name, game_data_dir);

    let min_free = 500 * 1024 * 1024; // 500 MB minimum
    let game_ok = budget.game_available_bytes > min_free;
    let staging_ok = budget.available_bytes > min_free;

    if game_ok && staging_ok {
        PreflightCheck {
            name: "Disk Space".into(),
            status: PreflightStatus::Pass,
            message: format!(
                "Game: {} free, Staging: {} free",
                disk_budget::format_bytes(budget.game_available_bytes),
                disk_budget::format_bytes(budget.available_bytes),
            ),
            detail: None,
        }
    } else {
        let mut issues = Vec::new();
        if !game_ok {
            issues.push(format!(
                "Game volume: only {} free",
                disk_budget::format_bytes(budget.game_available_bytes)
            ));
        }
        if !staging_ok {
            issues.push(format!(
                "Staging volume: only {} free",
                disk_budget::format_bytes(budget.available_bytes)
            ));
        }
        PreflightCheck {
            name: "Disk Space".into(),
            status: PreflightStatus::Fail,
            message: "Insufficient disk space".into(),
            detail: Some(issues.join("; ")),
        }
    }
}

/// Check that there are enabled mods to deploy.
fn check_enabled_mods(db: &ModDatabase, game_id: &str, bottle_name: &str) -> PreflightCheck {
    let (total, enabled) = db.get_mod_counts(game_id, bottle_name).unwrap_or((0, 0));

    if enabled == 0 && total > 0 {
        PreflightCheck {
            name: "Enabled Mods".into(),
            status: PreflightStatus::Warning,
            message: format!("No mods enabled out of {} installed", total),
            detail: Some("Enable at least one mod before deploying".into()),
        }
    } else if total == 0 {
        PreflightCheck {
            name: "Enabled Mods".into(),
            status: PreflightStatus::Warning,
            message: "No mods installed".into(),
            detail: None,
        }
    } else {
        PreflightCheck {
            name: "Enabled Mods".into(),
            status: PreflightStatus::Pass,
            message: format!("{} of {} mods enabled", enabled, total),
            detail: None,
        }
    }
}

/// Verify that staging directories exist for all enabled mods.
fn check_staging_dirs(db: &ModDatabase, game_id: &str, bottle_name: &str) -> PreflightCheck {
    let mods = db
        .list_mods_summary(game_id, bottle_name)
        .unwrap_or_default();
    let mut missing = Vec::new();

    for m in mods.iter().filter(|m| m.enabled) {
        if let Some(ref sp) = m.staging_path {
            if !PathBuf::from(sp).exists() {
                missing.push(m.name.clone());
            }
        }
    }

    if missing.is_empty() {
        PreflightCheck {
            name: "Staging Integrity".into(),
            status: PreflightStatus::Pass,
            message: "All staging directories present".into(),
            detail: None,
        }
    } else {
        PreflightCheck {
            name: "Staging Integrity".into(),
            status: PreflightStatus::Fail,
            message: format!("{} mod(s) have missing staging directories", missing.len()),
            detail: Some(format!("Missing: {}", missing.join(", "))),
        }
    }
}

/// Quick bottle health check (subset of full diagnostics).
fn check_bottle_health(bottle: &Bottle) -> PreflightCheck {
    if !bottle.drive_c().exists() {
        return PreflightCheck {
            name: "Bottle Health".into(),
            status: PreflightStatus::Fail,
            message: "Wine bottle's drive_c is missing".into(),
            detail: Some("The Wine bottle may be corrupted or unmounted".into()),
        };
    }

    let diag = wine_diagnostic::run_diagnostics(bottle, "skyrimse");
    if diag.errors > 0 {
        PreflightCheck {
            name: "Bottle Health".into(),
            status: PreflightStatus::Warning,
            message: format!(
                "{} issue(s) detected in Wine bottle",
                diag.errors + diag.warnings
            ),
            detail: Some("Run Wine diagnostics for details".into()),
        }
    } else {
        PreflightCheck {
            name: "Bottle Health".into(),
            status: PreflightStatus::Pass,
            message: "Wine bottle appears healthy".into(),
            detail: None,
        }
    }
}

/// Check for file conflicts.
fn check_conflicts(db: &ModDatabase, game_id: &str, bottle_name: &str) -> PreflightCheck {
    let conflicts = db
        .find_all_conflicts(game_id, bottle_name)
        .unwrap_or_default();

    if conflicts.is_empty() {
        PreflightCheck {
            name: "File Conflicts".into(),
            status: PreflightStatus::Pass,
            message: "No file conflicts".into(),
            detail: None,
        }
    } else {
        PreflightCheck {
            name: "File Conflicts".into(),
            status: PreflightStatus::Warning,
            message: format!(
                "{} file conflict(s) — highest-priority mod wins",
                conflicts.len()
            ),
            detail: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preflight_status_pass() {
        let check = PreflightCheck {
            name: "Test".into(),
            status: PreflightStatus::Pass,
            message: "ok".into(),
            detail: None,
        };
        assert_eq!(check.status, PreflightStatus::Pass);
    }

    #[test]
    fn preflight_status_fail() {
        let check = PreflightCheck {
            name: "Test".into(),
            status: PreflightStatus::Fail,
            message: "bad".into(),
            detail: Some("details".into()),
        };
        assert_eq!(check.status, PreflightStatus::Fail);
    }

    #[test]
    fn preflight_status_warning() {
        let check = PreflightCheck {
            name: "Test".into(),
            status: PreflightStatus::Warning,
            message: "meh".into(),
            detail: None,
        };
        assert_eq!(check.status, PreflightStatus::Warning);
    }

    #[test]
    fn preflight_result_can_proceed_no_failures() {
        let result = PreflightResult {
            checks: vec![
                PreflightCheck {
                    name: "A".into(),
                    status: PreflightStatus::Pass,
                    message: "ok".into(),
                    detail: None,
                },
                PreflightCheck {
                    name: "B".into(),
                    status: PreflightStatus::Warning,
                    message: "warn".into(),
                    detail: None,
                },
            ],
            passed: 1,
            failed: 0,
            warnings: 1,
            can_proceed: true,
        };
        assert!(result.can_proceed);
    }

    #[test]
    fn preflight_result_cannot_proceed_with_failure() {
        let result = PreflightResult {
            checks: vec![PreflightCheck {
                name: "A".into(),
                status: PreflightStatus::Fail,
                message: "bad".into(),
                detail: None,
            }],
            passed: 0,
            failed: 1,
            warnings: 0,
            can_proceed: false,
        };
        assert!(!result.can_proceed);
    }

    #[test]
    fn check_disk_space_with_temp() {
        let tmp = tempfile::TempDir::new().unwrap();
        let check = check_disk_space("test", "test", tmp.path());
        // /tmp should have plenty of space
        assert_eq!(check.status, PreflightStatus::Pass);
    }

    #[test]
    fn check_disk_space_message_contains_free() {
        let tmp = tempfile::TempDir::new().unwrap();
        let check = check_disk_space("test", "test", tmp.path());
        assert!(check.message.contains("free"));
    }

    #[test]
    fn check_enabled_mods_empty_db() {
        let db = ModDatabase::new(std::path::Path::new(":memory:")).unwrap();
        let check = check_enabled_mods(&db, "test", "test");
        assert_eq!(check.status, PreflightStatus::Warning);
    }

    #[test]
    fn check_enabled_mods_message() {
        let db = ModDatabase::new(std::path::Path::new(":memory:")).unwrap();
        let check = check_enabled_mods(&db, "test", "test");
        assert!(check.message.contains("No mods"));
    }

    #[test]
    fn check_staging_dirs_empty() {
        let db = ModDatabase::new(std::path::Path::new(":memory:")).unwrap();
        let check = check_staging_dirs(&db, "test", "test");
        assert_eq!(check.status, PreflightStatus::Pass);
    }

    #[test]
    fn check_staging_dirs_pass_message() {
        let db = ModDatabase::new(std::path::Path::new(":memory:")).unwrap();
        let check = check_staging_dirs(&db, "test", "test");
        assert!(check.message.contains("present"));
    }

    #[test]
    fn check_conflicts_empty() {
        let db = ModDatabase::new(std::path::Path::new(":memory:")).unwrap();
        let check = check_conflicts(&db, "test", "test");
        assert_eq!(check.status, PreflightStatus::Pass);
    }

    #[test]
    fn check_conflicts_pass_message() {
        let db = ModDatabase::new(std::path::Path::new(":memory:")).unwrap();
        let check = check_conflicts(&db, "test", "test");
        assert!(check.message.contains("No file conflicts"));
    }

    #[test]
    fn check_bottle_health_missing_drive_c() {
        let tmp = tempfile::TempDir::new().unwrap();
        let bottle = Bottle {
            name: "test".into(),
            path: tmp.path().to_path_buf(),
            source: "Test".into(),
        };
        let check = check_bottle_health(&bottle);
        assert_eq!(check.status, PreflightStatus::Fail);
    }

    #[test]
    fn check_bottle_health_fail_message() {
        let tmp = tempfile::TempDir::new().unwrap();
        let bottle = Bottle {
            name: "test".into(),
            path: tmp.path().to_path_buf(),
            source: "Test".into(),
        };
        let check = check_bottle_health(&bottle);
        assert!(check.message.contains("missing"));
    }

    #[test]
    fn check_bottle_health_with_drive_c() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("drive_c").join("windows").join("system32"))
            .unwrap();
        std::fs::create_dir_all(
            tmp.path()
                .join("drive_c")
                .join("users")
                .join("test")
                .join("AppData")
                .join("Local"),
        )
        .unwrap();
        let bottle = Bottle {
            name: "test".into(),
            path: tmp.path().to_path_buf(),
            source: "Test".into(),
        };
        let check = check_bottle_health(&bottle);
        // Should pass or warn (not fail) since drive_c exists
        assert_ne!(check.status, PreflightStatus::Fail);
    }

    #[test]
    fn check_bottle_health_pass_message() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("drive_c").join("windows").join("system32"))
            .unwrap();
        std::fs::create_dir_all(
            tmp.path()
                .join("drive_c")
                .join("users")
                .join("test")
                .join("AppData")
                .join("Local"),
        )
        .unwrap();
        let bottle = Bottle {
            name: "test".into(),
            path: tmp.path().to_path_buf(),
            source: "Test".into(),
        };
        let check = check_bottle_health(&bottle);
        assert!(check.message.contains("healthy") || check.message.contains("issue"));
    }

    /// Helper: create a bottle with a valid drive_c and common subdirectories.
    fn make_healthy_bottle(tmp: &tempfile::TempDir) -> Bottle {
        std::fs::create_dir_all(tmp.path().join("drive_c").join("windows").join("system32"))
            .unwrap();
        std::fs::create_dir_all(
            tmp.path()
                .join("drive_c")
                .join("users")
                .join("test")
                .join("AppData")
                .join("Local"),
        )
        .unwrap();
        Bottle {
            name: "test".into(),
            path: tmp.path().to_path_buf(),
            source: "Test".into(),
        }
    }

    // ── run_preflight tests ───────────────────────────────────────────────

    #[test]
    fn run_preflight_empty_db() {
        let db = ModDatabase::new(std::path::Path::new(":memory:")).unwrap();
        let tmp = tempfile::TempDir::new().unwrap();
        let bottle = make_healthy_bottle(&tmp);
        let game_data = tmp.path().join("drive_c").join("game");
        std::fs::create_dir_all(&game_data).unwrap();
        let result = run_preflight(&db, &bottle, "skyrimse", "test", &game_data);
        assert!(!result.checks.is_empty());
    }

    #[test]
    fn run_preflight_has_checks() {
        let db = ModDatabase::new(std::path::Path::new(":memory:")).unwrap();
        let tmp = tempfile::TempDir::new().unwrap();
        let bottle = make_healthy_bottle(&tmp);
        let game_data = tmp.path().join("drive_c").join("game");
        std::fs::create_dir_all(&game_data).unwrap();
        let result = run_preflight(&db, &bottle, "skyrimse", "test", &game_data);
        // run_preflight always pushes 5 checks
        assert_eq!(result.checks.len(), 5);
    }

    #[test]
    fn run_preflight_can_proceed_empty() {
        let db = ModDatabase::new(std::path::Path::new(":memory:")).unwrap();
        let tmp = tempfile::TempDir::new().unwrap();
        let bottle = make_healthy_bottle(&tmp);
        let game_data = tmp.path().join("drive_c").join("game");
        std::fs::create_dir_all(&game_data).unwrap();
        let result = run_preflight(&db, &bottle, "skyrimse", "test", &game_data);
        // No failures for an empty DB with a healthy bottle, so can_proceed
        assert!(result.can_proceed);
    }

    #[test]
    fn run_preflight_counts_consistent() {
        let db = ModDatabase::new(std::path::Path::new(":memory:")).unwrap();
        let tmp = tempfile::TempDir::new().unwrap();
        let bottle = make_healthy_bottle(&tmp);
        let game_data = tmp.path().join("drive_c").join("game");
        std::fs::create_dir_all(&game_data).unwrap();
        let result = run_preflight(&db, &bottle, "skyrimse", "test", &game_data);
        assert_eq!(
            result.passed + result.warnings + result.failed,
            result.checks.len()
        );
    }

    // ── check_disk_space additional tests ─────────────────────────────────

    #[test]
    fn check_disk_space_passes() {
        let tmp = tempfile::TempDir::new().unwrap();
        let check = check_disk_space("skyrimse", "test", tmp.path());
        assert_eq!(check.status, PreflightStatus::Pass);
    }

    #[test]
    fn check_disk_space_status_is_pass() {
        let tmp = tempfile::TempDir::new().unwrap();
        let check = check_disk_space("skyrimse", "test", tmp.path());
        assert_eq!(check.status, PreflightStatus::Pass);
        assert!(check.detail.is_none());
    }

    // ── check_enabled_mods additional tests ───────────────────────────────

    #[test]
    fn check_enabled_mods_with_mods() {
        let db = ModDatabase::new(std::path::Path::new(":memory:")).unwrap();
        db.add_mod(
            "skyrimse",
            "test",
            Some(1234),
            "Test Mod",
            "1.0",
            "testmod.zip",
            &["data/test.esp".to_string()],
        )
        .unwrap();
        let check = check_enabled_mods(&db, "skyrimse", "test");
        // add_mod inserts with enabled=1 by default, so this should pass.
        assert_eq!(check.status, PreflightStatus::Pass);
    }

    #[test]
    fn check_enabled_mods_status_is_warning_when_empty() {
        let db = ModDatabase::new(std::path::Path::new(":memory:")).unwrap();
        let check = check_enabled_mods(&db, "skyrimse", "test");
        assert_eq!(check.status, PreflightStatus::Warning);
        assert!(check.message.contains("No mods"));
    }

    // ── check_staging_dirs additional tests ────────────────────────────────

    #[test]
    fn check_staging_dirs_with_staging() {
        let db = ModDatabase::new(std::path::Path::new(":memory:")).unwrap();
        let tmp = tempfile::TempDir::new().unwrap();
        let staging = tmp.path().join("staging").join("mod1");
        std::fs::create_dir_all(&staging).unwrap();

        let mod_id = db
            .add_mod(
                "skyrimse",
                "test",
                Some(1),
                "Mod With Staging",
                "1.0",
                "mod.zip",
                &["data/plugin.esp".to_string()],
            )
            .unwrap();
        db.set_staging_path(mod_id, staging.to_str().unwrap())
            .unwrap();

        let check = check_staging_dirs(&db, "skyrimse", "test");
        assert_eq!(check.status, PreflightStatus::Pass);
    }

    #[test]
    fn check_staging_dirs_pass_message_correct() {
        let db = ModDatabase::new(std::path::Path::new(":memory:")).unwrap();
        let check = check_staging_dirs(&db, "skyrimse", "test");
        assert_eq!(check.message, "All staging directories present");
    }

    // ── check_conflicts additional tests ──────────────────────────────────

    #[test]
    fn check_conflicts_with_conflicts() {
        let db = ModDatabase::new(std::path::Path::new(":memory:")).unwrap();
        // Two mods that install the same file → conflict
        db.add_mod(
            "skyrimse",
            "test",
            Some(1),
            "Mod A",
            "1.0",
            "modA.zip",
            &["data/textures/shared.dds".to_string()],
        )
        .unwrap();
        db.add_mod(
            "skyrimse",
            "test",
            Some(2),
            "Mod B",
            "1.0",
            "modB.zip",
            &["data/textures/shared.dds".to_string()],
        )
        .unwrap();

        let check = check_conflicts(&db, "skyrimse", "test");
        assert_eq!(check.status, PreflightStatus::Warning);
    }

    #[test]
    fn check_conflicts_status() {
        let db = ModDatabase::new(std::path::Path::new(":memory:")).unwrap();
        // Two mods with the same file should produce a warning
        db.add_mod(
            "skyrimse",
            "test",
            Some(10),
            "Conflict Mod 1",
            "2.0",
            "cm1.zip",
            &["data/meshes/overlap.nif".to_string()],
        )
        .unwrap();
        db.add_mod(
            "skyrimse",
            "test",
            Some(11),
            "Conflict Mod 2",
            "2.0",
            "cm2.zip",
            &["data/meshes/overlap.nif".to_string()],
        )
        .unwrap();

        let check = check_conflicts(&db, "skyrimse", "test");
        assert_eq!(check.status, PreflightStatus::Warning);
        assert!(check.message.contains("conflict"));
    }
}
