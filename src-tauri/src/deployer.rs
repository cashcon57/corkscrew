//! Deployment engine for installing mod files from staging into game directories.
//!
//! Uses a **hardlink-first, copy-fallback** strategy:
//! 1. Attempt `std::fs::hard_link()` for each file (zero disk overhead).
//! 2. If that fails (cross-volume, unsupported FS), fall back to `std::fs::copy()`.
//! 3. Track every deployed file in the `deployment_manifest` database table.
//!
//! Key operations:
//! - `deploy_mod` — deploy a single mod's files from staging to game dir
//! - `undeploy_mod` — remove deployed files, restore lower-priority files
//! - `redeploy_all` — purge + redeploy all enabled mods (after priority changes)
//! - `purge_deployment` — remove all deployed files (clean slate)

use std::fs;
use std::path::{Path, PathBuf};

use log::{debug, info, warn};
use thiserror::Error;

use crate::database::ModDatabase;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum DeployerError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Staging directory not found: {0}")]
    StagingNotFound(PathBuf),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, DeployerError>;

// ---------------------------------------------------------------------------
// DeployResult
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct DeployResult {
    /// Number of files successfully deployed.
    pub deployed_count: usize,
    /// Number of files skipped due to higher-priority conflicts.
    pub skipped_count: usize,
    /// Whether any files fell back to copy (hardlinks not supported).
    pub fallback_used: bool,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Test whether hardlinks work between staging_dir and data_dir.
pub fn test_hardlink_support(staging_dir: &Path, data_dir: &Path) -> bool {
    let test_src = staging_dir.join(".corkscrew_hardlink_test");
    let test_dst = data_dir.join(".corkscrew_hardlink_test");

    if fs::write(&test_src, b"test").is_err() {
        return false;
    }

    let result = fs::hard_link(&test_src, &test_dst).is_ok();

    let _ = fs::remove_file(&test_src);
    let _ = fs::remove_file(&test_dst);

    result
}

/// Deploy a single mod's files from staging to data_dir.
/// Higher-priority mods win file conflicts.
pub fn deploy_mod(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    mod_id: i64,
    staging_path: &Path,
    data_dir: &Path,
    files: &[String],
) -> Result<DeployResult> {
    if !staging_path.exists() {
        return Err(DeployerError::StagingNotFound(staging_path.to_path_buf()));
    }

    // Hardlinks are free; only check space for copy-based deployment.
    if !test_hardlink_support(staging_path, data_dir) {
        let deploy_size = crate::disk_budget::dir_size(staging_path);
        crate::disk_budget::check_space_guard(data_dir, deploy_size)
            .map_err(DeployerError::Other)?;
    }

    let mod_info = db
        .get_mod(mod_id)
        .map_err(|e| DeployerError::Database(e.to_string()))?
        .ok_or_else(|| DeployerError::Database(format!("Mod {} not found", mod_id)))?;
    let my_priority = mod_info.install_priority;

    let mut deployed_count = 0;
    let mut skipped_count = 0;
    let mut fallback_used = false;

    for rel_path in files {
        let src = staging_path.join(rel_path);
        let dst = data_dir.join(rel_path);

        if !src.exists() {
            warn!("Staging file missing, skipping: {}", src.display());
            continue;
        }

        let existing = db
            .get_deployed_file(game_id, bottle_name, rel_path)
            .map_err(|e| DeployerError::Database(e.to_string()))?;

        if let Some(entry) = &existing {
            if entry.mod_id == mod_id {
                continue;
            }

            let other_mod = db
                .get_mod(entry.mod_id)
                .map_err(|e| DeployerError::Database(e.to_string()))?;

            if let Some(other) = other_mod {
                if other.install_priority > my_priority {
                    debug!(
                        "Skipping {} (owned by higher-priority mod '{}')",
                        rel_path, other.name
                    );
                    skipped_count += 1;
                    continue;
                }
            }

            if dst.exists() {
                fs::remove_file(&dst)?;
            }
        }

        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }

        // Prevent symlink-following attacks
        if dst.exists() && fs::symlink_metadata(&dst)?.file_type().is_symlink() {
            warn!("Skipping deployment to symlink target: {}", dst.display());
            skipped_count += 1;
            continue;
        }

        let method = match fs::hard_link(&src, &dst) {
            Ok(_) => "hardlink",
            Err(_) => {
                fs::copy(&src, &dst)?;
                fallback_used = true;
                "copy"
            }
        };

        db.add_deployment_entry(
            game_id,
            bottle_name,
            mod_id,
            rel_path,
            &staging_path.to_string_lossy(),
            method,
            None, // SHA-256 computed at staging time, not deployment
        )
        .map_err(|e| DeployerError::Database(e.to_string()))?;

        deployed_count += 1;
    }

    info!(
        "Deployed mod {} ({} files, {} skipped, hardlink fallback: {})",
        mod_id, deployed_count, skipped_count, fallback_used
    );

    Ok(DeployResult {
        deployed_count,
        skipped_count,
        fallback_used,
    })
}

/// Undeploy a single mod: remove all its deployed files from data_dir.
///
/// If a lower-priority mod also has a file at the same path, that mod's file
/// will be re-deployed (the "next winner" takes over).
pub fn undeploy_mod(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    mod_id: i64,
    data_dir: &Path,
) -> Result<Vec<String>> {
    let removed_paths = db
        .remove_deployment_entries_for_mod(mod_id)
        .map_err(|e| DeployerError::Database(e.to_string()))?;

    let mut actually_removed = Vec::new();

    for rel_path in &removed_paths {
        let file_path = data_dir.join(rel_path);

        if file_path.exists() {
            fs::remove_file(&file_path)?;
            actually_removed.push(rel_path.clone());

            prune_empty_dirs(&file_path, data_dir);
        }

        restore_next_winner(db, game_id, bottle_name, rel_path, data_dir)?;
    }

    info!(
        "Undeployed mod {} ({}/{} files removed)",
        mod_id,
        actually_removed.len(),
        removed_paths.len()
    );

    Ok(actually_removed)
}

/// Full redeploy: purge everything from data_dir that's in the manifest,
/// then redeploy all enabled mods in priority order.
pub fn redeploy_all(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    data_dir: &Path,
) -> Result<DeployResult> {
    if !test_hardlink_support(data_dir, data_dir) {
        let total_staging: u64 = db
            .list_mods(game_id, bottle_name)
            .unwrap_or_default()
            .iter()
            .filter(|m| m.enabled)
            .filter_map(|m| m.staging_path.as_ref())
            .map(|p| crate::disk_budget::dir_size(std::path::Path::new(p)))
            .sum();
        crate::disk_budget::check_space_guard(data_dir, total_staging)
            .map_err(DeployerError::Other)?;
    }

    purge_deployment(db, game_id, bottle_name, data_dir)?;

    let mods = db
        .list_mods(game_id, bottle_name)
        .map_err(|e| DeployerError::Database(e.to_string()))?;

    let mut enabled_mods: Vec<_> = mods.into_iter().filter(|m| m.enabled).collect();
    enabled_mods.sort_by_key(|m| m.install_priority);

    let mut total_deployed = 0;
    let mut total_skipped = 0;
    let mut any_fallback = false;

    for m in &enabled_mods {
        if let Some(ref staging_path_str) = m.staging_path {
            let staging_path = PathBuf::from(staging_path_str);
            if staging_path.exists() {
                let files = crate::staging::list_staging_files(&staging_path)
                    .map_err(|e| DeployerError::Other(e.to_string()))?;

                let result = deploy_mod(
                    db,
                    game_id,
                    bottle_name,
                    m.id,
                    &staging_path,
                    data_dir,
                    &files,
                )?;

                total_deployed += result.deployed_count;
                total_skipped += result.skipped_count;
                any_fallback = any_fallback || result.fallback_used;
            }
        }
        // Legacy mods (no staging_path) are skipped during redeploy
    }

    info!(
        "Full redeploy for {}/{}: {} files deployed, {} skipped",
        game_id, bottle_name, total_deployed, total_skipped
    );

    Ok(DeployResult {
        deployed_count: total_deployed,
        skipped_count: total_skipped,
        fallback_used: any_fallback,
    })
}

/// Purge all deployed files from data_dir (clean slate).
/// Only removes files tracked in the deployment manifest.
pub fn purge_deployment(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    data_dir: &Path,
) -> Result<Vec<String>> {
    let manifest = db
        .get_deployment_manifest(game_id, bottle_name)
        .map_err(|e| DeployerError::Database(e.to_string()))?;

    let mut removed = Vec::new();

    for entry in &manifest {
        // Legacy direct-installed files are not ours to purge
        if entry.deploy_method == "direct" {
            continue;
        }

        let file_path = data_dir.join(&entry.relative_path);
        if file_path.exists() {
            if let Err(e) = fs::remove_file(&file_path) {
                warn!("Failed to purge {}: {}", file_path.display(), e);
            } else {
                removed.push(entry.relative_path.clone());
                prune_empty_dirs(&file_path, data_dir);
            }
        }

        db.remove_deployment_entries_for_mod(entry.mod_id)
            .map_err(|e| DeployerError::Database(e.to_string()))?;
    }

    info!(
        "Purged deployment for {}/{}: {} files removed",
        game_id,
        bottle_name,
        removed.len()
    );

    Ok(removed)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Walk up from a removed file and prune empty directories up to (not including)
/// `stop_at`.
fn prune_empty_dirs(removed_file: &Path, stop_at: &Path) {
    let mut current = removed_file.parent().map(|p| p.to_path_buf());
    while let Some(dir) = current {
        if dir == stop_at {
            break;
        }
        let is_empty = fs::read_dir(&dir)
            .map(|mut rd| rd.next().is_none())
            .unwrap_or(false);
        if is_empty {
            debug!("Pruning empty directory: {}", dir.display());
            let _ = fs::remove_dir(&dir);
            current = dir.parent().map(|p| p.to_path_buf());
        } else {
            break;
        }
    }
}

/// Check if another enabled mod has a file at this path and re-deploy it.
fn restore_next_winner(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    rel_path: &str,
    data_dir: &Path,
) -> Result<()> {
    let mods = db
        .list_mods(game_id, bottle_name)
        .map_err(|e| DeployerError::Database(e.to_string()))?;

    let mut candidates: Vec<_> = mods
        .iter()
        .filter(|m| {
            m.enabled
                && m.staging_path.is_some()
                && m.installed_files.contains(&rel_path.to_string())
        })
        .collect();

    candidates.sort_by(|a, b| b.install_priority.cmp(&a.install_priority));

    if let Some(winner) = candidates.first() {
        let staging_path = PathBuf::from(winner.staging_path.as_ref().unwrap());
        let src = staging_path.join(rel_path);
        let dst = data_dir.join(rel_path);

        if src.exists() {
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent)?;
            }

            let method = match fs::hard_link(&src, &dst) {
                Ok(_) => "hardlink",
                Err(_) => {
                    fs::copy(&src, &dst)?;
                    "copy"
                }
            };

            db.add_deployment_entry(
                game_id,
                bottle_name,
                winner.id,
                rel_path,
                &staging_path.to_string_lossy(),
                method,
                None,
            )
            .map_err(|e| DeployerError::Database(e.to_string()))?;

            debug!(
                "Restored {} from mod '{}' (priority {})",
                rel_path, winner.name, winner.install_priority
            );
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::ModDatabase;
    use crate::executables;
    use std::fs;
    use tempfile::TempDir;

    fn setup() -> (ModDatabase, TempDir, PathBuf, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let db = ModDatabase::new(&db_path).unwrap();
        executables::init_schema(&db).unwrap();

        let staging = tmp.path().join("staging");
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&staging).unwrap();
        fs::create_dir_all(&data_dir).unwrap();

        (db, tmp, staging, data_dir)
    }

    fn create_staging_file(staging: &Path, rel_path: &str, content: &[u8]) {
        let full = staging.join(rel_path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, content).unwrap();
    }

    #[test]
    fn deploy_creates_files_in_data_dir() {
        let (db, _tmp, staging, data_dir) = setup();

        let mod_id = db
            .add_mod(
                "skyrimse",
                "Gaming",
                None,
                "TestMod",
                "1.0",
                "test.zip",
                &["meshes/test.nif".to_string(), "mod.esp".to_string()],
            )
            .unwrap();

        create_staging_file(&staging, "meshes/test.nif", b"nif data");
        create_staging_file(&staging, "mod.esp", b"esp data");

        let files = vec!["meshes/test.nif".to_string(), "mod.esp".to_string()];
        let result = deploy_mod(
            &db, "skyrimse", "Gaming", mod_id, &staging, &data_dir, &files,
        )
        .unwrap();

        assert_eq!(result.deployed_count, 2);
        assert_eq!(result.skipped_count, 0);
        assert!(data_dir.join("meshes/test.nif").exists());
        assert!(data_dir.join("mod.esp").exists());
    }

    #[test]
    fn undeploy_removes_files() {
        let (db, _tmp, staging, data_dir) = setup();

        let files = vec!["test.esp".to_string()];
        let mod_id = db
            .add_mod(
                "skyrimse", "Gaming", None, "TestMod", "1.0", "test.zip", &files,
            )
            .unwrap();

        create_staging_file(&staging, "test.esp", b"esp");

        deploy_mod(
            &db, "skyrimse", "Gaming", mod_id, &staging, &data_dir, &files,
        )
        .unwrap();
        assert!(data_dir.join("test.esp").exists());

        let removed = undeploy_mod(&db, "skyrimse", "Gaming", mod_id, &data_dir).unwrap();
        assert_eq!(removed.len(), 1);
        assert!(!data_dir.join("test.esp").exists());
    }

    #[test]
    fn higher_priority_wins_deployment() {
        let (db, _tmp, staging1, data_dir) = setup();
        let staging2 = staging1.parent().unwrap().join("staging2");
        fs::create_dir_all(&staging2).unwrap();

        let files = vec!["shared.esp".to_string()];

        // Low priority mod
        let mod1 = db
            .add_mod(
                "skyrimse", "Gaming", None, "LowPri", "1.0", "low.zip", &files,
            )
            .unwrap();
        db.set_mod_priority(mod1, 0).unwrap();
        create_staging_file(&staging1, "shared.esp", b"low priority data");

        // High priority mod
        let mod2 = db
            .add_mod(
                "skyrimse", "Gaming", None, "HighPri", "1.0", "high.zip", &files,
            )
            .unwrap();
        db.set_mod_priority(mod2, 10).unwrap();
        create_staging_file(&staging2, "shared.esp", b"high priority data");

        deploy_mod(
            &db, "skyrimse", "Gaming", mod1, &staging1, &data_dir, &files,
        )
        .unwrap();

        deploy_mod(
            &db, "skyrimse", "Gaming", mod2, &staging2, &data_dir, &files,
        )
        .unwrap();

        let content = fs::read_to_string(data_dir.join("shared.esp")).unwrap();
        assert_eq!(content, "high priority data");
    }

    #[test]
    fn test_hardlink_support_on_same_volume() {
        let tmp = TempDir::new().unwrap();
        let dir_a = tmp.path().join("a");
        let dir_b = tmp.path().join("b");
        fs::create_dir_all(&dir_a).unwrap();
        fs::create_dir_all(&dir_b).unwrap();

        assert!(test_hardlink_support(&dir_a, &dir_b));
    }
}
