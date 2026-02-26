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
// Filesystem helpers
// ---------------------------------------------------------------------------

/// Check whether two paths reside on the same filesystem (device).
/// Returns `false` if either path doesn't exist or metadata can't be read.
#[cfg(unix)]
pub fn same_filesystem(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(ma), Ok(mb)) => ma.dev() == mb.dev(),
        _ => false,
    }
}

#[cfg(not(unix))]
pub fn same_filesystem(_a: &Path, _b: &Path) -> bool {
    false // assume different on non-Unix; always copy
}

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
///
/// Uses parallel file I/O via rayon for maximum throughput on multi-core
/// systems. Conflict resolution uses bulk-loaded in-memory lookups, and
/// deployment entries are batch-inserted in a single transaction.
pub fn deploy_mod(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    mod_id: i64,
    staging_path: &Path,
    data_dir: &Path,
    files: &[String],
) -> Result<DeployResult> {
    use rayon::prelude::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    if !staging_path.exists() {
        return Err(DeployerError::StagingNotFound(staging_path.to_path_buf()));
    }

    let can_hardlink = same_filesystem(staging_path, data_dir);
    if !can_hardlink {
        debug!(
            "Staging ({}) and data_dir ({}) are on different filesystems — will use copy",
            staging_path.display(),
            data_dir.display()
        );
        let deploy_size = crate::disk_budget::dir_size(staging_path);
        crate::disk_budget::check_space_guard(data_dir, deploy_size)
            .map_err(DeployerError::Other)?;
    }

    let mod_info = db
        .get_mod(mod_id)
        .map_err(|e| DeployerError::Database(e.to_string()))?
        .ok_or_else(|| DeployerError::Database(format!("Mod {} not found", mod_id)))?;
    let my_priority = mod_info.install_priority;

    // Batch-load existing deployment manifest + mod priorities into memory
    // to avoid per-file database round-trips during conflict resolution.
    let manifest = db
        .get_deployment_manifest(game_id, bottle_name)
        .map_err(|e| DeployerError::Database(e.to_string()))?;
    let deployed_map: std::collections::HashMap<&str, i64> = manifest
        .iter()
        .map(|e| (e.relative_path.as_str(), e.mod_id))
        .collect();
    let priorities = db
        .get_all_mod_priorities()
        .map_err(|e| DeployerError::Database(e.to_string()))?;

    let deployed_count = AtomicUsize::new(0);
    let skipped_count = AtomicUsize::new(0);
    let missing_count = AtomicUsize::new(0);
    let fallback_used = AtomicBool::new(false);
    let staging_str = staging_path.to_string_lossy().to_string();

    // Phase 1: Parallel file I/O — resolve conflicts, then hardlink or copy.
    // Collect successful deployments for batch database insert.
    let results: Vec<Option<(String, &str)>> = files
        .par_iter()
        .map(|rel_path| {
            let src = staging_path.join(rel_path);
            let dst = data_dir.join(rel_path);

            if !src.exists() {
                missing_count.fetch_add(1, Ordering::Relaxed);
                if missing_count.load(Ordering::Relaxed) <= 5 {
                    warn!(
                        "Deploy: source file not found in staging: {} (mod {})",
                        src.display(),
                        mod_id
                    );
                }
                return None;
            }

            // Conflict resolution via in-memory lookup
            if let Some(&owner_mod_id) = deployed_map.get(rel_path.as_str()) {
                if owner_mod_id == mod_id {
                    return None; // already deployed by us
                }
                let owner_priority = priorities.get(&owner_mod_id).copied().unwrap_or(0);
                if owner_priority > my_priority as i64 {
                    skipped_count.fetch_add(1, Ordering::Relaxed);
                    return None;
                }
                // We win — remove existing file
                if dst.exists() {
                    let _ = fs::remove_file(&dst);
                }
            }

            if let Some(parent) = dst.parent() {
                let _ = fs::create_dir_all(parent); // idempotent, safe for parallel calls
            }

            // Prevent symlink-following attacks
            if dst.exists() {
                if let Ok(meta) = fs::symlink_metadata(&dst) {
                    if meta.file_type().is_symlink() {
                        warn!("Skipping deployment to symlink target: {}", dst.display());
                        skipped_count.fetch_add(1, Ordering::Relaxed);
                        return None;
                    }
                }
            }

            let method = if can_hardlink {
                match fs::hard_link(&src, &dst) {
                    Ok(_) => "hardlink",
                    Err(e) => {
                        warn!(
                            "Hardlink failed for {} → {}: {} (falling back to copy)",
                            src.display(),
                            dst.display(),
                            e
                        );
                        if let Err(copy_err) = fs::copy(&src, &dst) {
                            warn!(
                                "Copy also failed for {} → {}: {}",
                                src.display(),
                                dst.display(),
                                copy_err
                            );
                            return None;
                        }
                        fallback_used.store(true, Ordering::Relaxed);
                        "copy"
                    }
                }
            } else {
                if let Err(copy_err) = fs::copy(&src, &dst) {
                    warn!(
                        "Copy failed for {} → {}: {}",
                        src.display(),
                        dst.display(),
                        copy_err
                    );
                    return None;
                }
                fallback_used.store(true, Ordering::Relaxed);
                "copy"
            };

            deployed_count.fetch_add(1, Ordering::Relaxed);
            Some((rel_path.clone(), method))
        })
        .collect();

    // Phase 2: Batch-insert all deployment entries in a single transaction.
    let batch: Vec<(&str, &str, i64, &str, &str, &str)> = results
        .iter()
        .filter_map(|opt| {
            opt.as_ref().map(|(rel_path, method)| {
                (
                    game_id,
                    bottle_name,
                    mod_id,
                    rel_path.as_str(),
                    staging_str.as_str(),
                    *method,
                )
            })
        })
        .collect();

    if !batch.is_empty() {
        db.batch_add_deployment_entries(&batch)
            .map_err(|e| DeployerError::Database(e.to_string()))?;
    }

    let final_deployed = deployed_count.load(Ordering::Relaxed);
    let final_skipped = skipped_count.load(Ordering::Relaxed);
    let final_missing = missing_count.load(Ordering::Relaxed);
    let final_fallback = fallback_used.load(Ordering::Relaxed);

    if final_missing > 5 {
        warn!(
            "Deploy mod {}: {} additional source files not found in staging (suppressed)",
            mod_id,
            final_missing - 5
        );
    }

    info!(
        "Deployed mod {} ({} files, {} skipped, {} missing, hardlink fallback: {})",
        mod_id, final_deployed, final_skipped, final_missing, final_fallback
    );

    // If we expected to deploy files but none succeeded, something is wrong
    if final_deployed == 0 && !files.is_empty() {
        warn!(
            "Deploy mod {}: 0 of {} files deployed! staging_path={}, data_dir={}, exists=({}, {})",
            mod_id,
            files.len(),
            staging_path.display(),
            data_dir.display(),
            staging_path.exists(),
            data_dir.exists(),
        );
        return Err(DeployerError::Other(format!(
            "0 of {} files deployed — staging may be missing or paths may not match \
             (staging exists: {}, data_dir exists: {}, missing: {})",
            files.len(),
            staging_path.exists(),
            data_dir.exists(),
            final_missing,
        )));
    }

    Ok(DeployResult {
        deployed_count: final_deployed,
        skipped_count: final_skipped,
        fallback_used: final_fallback,
    })
}

/// Deploy a mod atomically: if deployment fails partway through, roll back
/// any partially deployed files so the game directory is not left in a
/// broken state.
pub fn deploy_mod_atomic(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    mod_id: i64,
    staging_path: &Path,
    data_dir: &Path,
    files: &[String],
) -> Result<DeployResult> {
    match deploy_mod(
        db,
        game_id,
        bottle_name,
        mod_id,
        staging_path,
        data_dir,
        files,
    ) {
        Ok(result) => Ok(result),
        Err(e) => {
            warn!(
                "deploy_mod failed for mod {}, rolling back partially deployed files: {}",
                mod_id, e
            );
            let _ = undeploy_mod(db, game_id, bottle_name, mod_id, data_dir);
            Err(e)
        }
    }
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
    // Query manifest paths FIRST without deleting entries.
    // Entries are only deleted after files are successfully removed,
    // preventing orphaned files if removal fails partway through.
    let manifest_paths = db
        .get_deployment_paths_for_mod(mod_id)
        .map_err(|e| DeployerError::Database(e.to_string()))?;

    let mut actually_removed = Vec::new();
    let mut errors = Vec::new();
    let mut restore_failures = Vec::new();

    for rel_path in &manifest_paths {
        let file_path = data_dir.join(rel_path);

        if file_path.exists() {
            match fs::remove_file(&file_path) {
                Ok(()) => {
                    actually_removed.push(rel_path.clone());
                    prune_empty_dirs(&file_path, data_dir);
                }
                Err(e) => {
                    errors.push(format!("{}: {}", rel_path, e));
                    continue;
                }
            }
        } else {
            // File already gone — still count as "removed" for manifest cleanup
            actually_removed.push(rel_path.clone());
        }

        // Restore next-priority mod's version of this file if applicable.
        // This runs BEFORE manifest deletion so that on failure, the manifest
        // still tracks the file until cleanup completes.
        if let Err(e) = restore_next_winner(db, game_id, bottle_name, rel_path, data_dir) {
            warn!("Failed to restore winner for {}: {}", rel_path, e);
            restore_failures.push(rel_path.clone());
        }
    }

    // Delete manifest entries after all file operations are complete.
    // Even if some restorations failed, we still clean the manifest to avoid
    // stale entries — but log a warning about potential orphans.
    if !restore_failures.is_empty() {
        warn!(
            "Mod {} undeploy: {} file(s) could not be restored from lower-priority mods, \
             potential orphans: {:?}",
            mod_id,
            restore_failures.len(),
            restore_failures,
        );
    }
    let _ = db.remove_deployment_entries_for_mod(mod_id);

    info!(
        "Undeployed mod {} ({}/{} files removed, {} errors, {} restore failures)",
        mod_id,
        actually_removed.len(),
        manifest_paths.len(),
        errors.len(),
        restore_failures.len(),
    );

    if !errors.is_empty() {
        warn!("Undeploy errors for mod {}: {:?}", mod_id, errors);
    }

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
    redeploy_all_with_progress(
        db,
        game_id,
        bottle_name,
        data_dir,
        None::<fn(usize, usize, &str, usize, usize)>,
    )
}

/// Full redeploy with optional progress callback.
///
/// The callback receives `(current_index, total_mods, mod_name, files_deployed, total_files)`
/// during deployment, allowing the frontend to display a smooth progress indicator.
pub fn redeploy_all_with_progress<F>(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    data_dir: &Path,
    on_progress: Option<F>,
) -> Result<DeployResult>
where
    F: Fn(usize, usize, &str, usize, usize),
{
    // Check disk space if any staging dir is on a different filesystem than data_dir
    // (hardlinks won't work cross-filesystem, so copies will consume space).
    let staging_root = crate::staging::staging_base_dir(game_id, bottle_name);
    if !same_filesystem(&staging_root, data_dir) {
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

    let total = enabled_mods.len();
    let mut total_deployed = 0;
    let mut total_skipped = 0;
    let mut any_fallback = false;

    // Pre-count total files for accurate progress reporting
    let total_files: usize = enabled_mods
        .iter()
        .filter_map(|m| m.staging_path.as_ref())
        .filter_map(|p| crate::staging::list_staging_files(Path::new(p)).ok())
        .map(|f| f.len())
        .sum();
    let mut files_so_far: usize = 0;

    for (i, m) in enabled_mods.iter().enumerate() {
        if let Some(ref on_progress) = on_progress {
            on_progress(i, total, &m.name, files_so_far, total_files);
        }

        if let Some(ref staging_path_str) = m.staging_path {
            let staging_path = PathBuf::from(staging_path_str);
            if staging_path.exists() {
                let files = crate::staging::list_staging_files(&staging_path)
                    .map_err(|e| DeployerError::Other(e.to_string()))?;

                let file_count = files.len();
                let result = deploy_mod(
                    db,
                    game_id,
                    bottle_name,
                    m.id,
                    &staging_path,
                    data_dir,
                    &files,
                )?;

                files_so_far += file_count;
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
        let Some(staging_ref) = winner.staging_path.as_ref() else {
            return Ok(());
        };
        let staging_path = PathBuf::from(staging_ref);
        let src = staging_path.join(rel_path);
        let dst = data_dir.join(rel_path);

        if src.exists() {
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent)?;
            }

            let can_hardlink = same_filesystem(&staging_path, data_dir);
            let method = if can_hardlink {
                match fs::hard_link(&src, &dst) {
                    Ok(_) => "hardlink",
                    Err(e) => {
                        warn!("Hardlink failed in restore_next_winner: {}", e);
                        fs::copy(&src, &dst)?;
                        "copy"
                    }
                }
            } else {
                fs::copy(&src, &dst)?;
                "copy"
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

    #[test]
    fn same_filesystem_same_tmpdir() {
        let tmp = TempDir::new().unwrap();
        let dir_a = tmp.path().join("a");
        let dir_b = tmp.path().join("b");
        fs::create_dir_all(&dir_a).unwrap();
        fs::create_dir_all(&dir_b).unwrap();

        assert!(same_filesystem(&dir_a, &dir_b));
    }

    #[test]
    fn same_filesystem_nonexistent_returns_false() {
        let tmp = TempDir::new().unwrap();
        let real = tmp.path().join("real");
        fs::create_dir_all(&real).unwrap();
        let fake = PathBuf::from("/nonexistent/corkscrew_test_xyz");

        assert!(!same_filesystem(&real, &fake));
    }
}
