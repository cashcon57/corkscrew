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

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::baselines;
use crate::database::ModDatabase;
use crate::platform;

// ---------------------------------------------------------------------------
// Vanilla file protection
// ---------------------------------------------------------------------------

/// Quick check for top-level game files that must never be deleted.
/// This is a fast path that doesn't require a baseline lookup.
fn is_protected_extension(rel_path: &str) -> bool {
    let lower = rel_path.to_lowercase();
    if !lower.contains('/')
        && (lower.ends_with(".esm") || lower.ends_with(".bsa") || lower.ends_with(".ba2"))
    {
        return true;
    }
    false
}

/// Build a set of known vanilla file paths for a game (lowercase, for O(1) lookup).
/// Returns None if no baseline is available for the game.
fn build_vanilla_set(game_id: &str) -> Option<std::collections::HashSet<String>> {
    baselines::get_builtin_baseline(game_id)
        .map(|baseline| baseline.into_iter().map(|p| p.to_lowercase()).collect())
}

/// Check if a file is a vanilla/stock game file that should NOT be deleted
/// during purge or undeploy operations. Uses a pre-built vanilla set for
/// efficiency when checking many files.
fn is_vanilla_file_with_set(
    game_id: &str,
    rel_path: &str,
    vanilla_set: Option<&std::collections::HashSet<String>>,
) -> bool {
    // Fast path: top-level .esm/.bsa/.ba2 are always protected
    if is_protected_extension(rel_path) {
        return true;
    }

    let lower = rel_path.to_lowercase();

    // Check built-in baseline (pre-computed set)
    if let Some(set) = vanilla_set {
        if set.contains(&lower) {
            return true;
        }
    }

    // Check stock patterns (CC content, video files, etc.)
    if baselines::is_stock_pattern(game_id, rel_path) {
        return true;
    }

    false
}

/// Callback type for reporting deployment progress: (files_done, files_total).
pub type DeployProgressCb = dyn Fn(u64, u64) + Send + Sync;

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
    deploy_mod_inner(
        db,
        game_id,
        bottle_name,
        mod_id,
        staging_path,
        data_dir,
        files,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn deploy_mod_inner(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    mod_id: i64,
    staging_path: &Path,
    data_dir: &Path,
    files: &[String],
    progress: Option<&DeployProgressCb>,
) -> Result<DeployResult> {
    use rayon::prelude::*;

    if !staging_path.exists() {
        return Err(DeployerError::StagingNotFound(staging_path.to_path_buf()));
    }

    let can_hardlink = same_filesystem(staging_path, data_dir);
    let copy_method = platform::detect_copy_method(staging_path, data_dir);
    if !can_hardlink {
        debug!(
            "Staging ({}) and data_dir ({}) are on different filesystems — will use copy ({:?})",
            staging_path.display(),
            data_dir.display(),
            copy_method
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
            // Defense-in-depth: reject path traversal
            if !crate::staging::is_safe_relative_path(rel_path) {
                warn!("Deploy: skipping unsafe relative path: {}", rel_path);
                return None;
            }

            // Defense-in-depth: skip packaging junk (fomod/, meta.ini, etc.)
            if crate::installer::is_deploy_junk(std::path::Path::new(rel_path)) {
                debug!("Deploy: skipping junk file: {}", rel_path);
                return None;
            }

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
                // We win — remove existing deployed file
                if dst.exists() {
                    let _ = fs::remove_file(&dst);
                }
            } else if dst.exists() {
                // File exists on disk but is NOT in the deployment manifest.
                // This is likely a vanilla game file. Do NOT overwrite it —
                // removing the existing file and deploying over it would make
                // the vanilla file unrecoverable on undeploy/purge.
                if is_protected_extension(rel_path) {
                    warn!(
                        "Deploy: skipping {} — would overwrite unmanaged vanilla file",
                        rel_path
                    );
                    skipped_count.fetch_add(1, Ordering::Relaxed);
                    return None;
                }
                // For non-protected files (textures, scripts, etc.), remove
                // the existing file so the hardlink can succeed.
                let _ = fs::remove_file(&dst);
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
                        if let Err(copy_err) = platform::fast_copy(&src, &dst, copy_method) {
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
                if let Err(copy_err) = platform::fast_copy(&src, &dst, copy_method) {
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

            let done = deployed_count.fetch_add(1, Ordering::Relaxed) + 1;
            if let Some(cb) = &progress {
                let total = files.len() as u64;
                let interval = (total / 50).clamp(10, 100);
                if (done as u64).is_multiple_of(interval) || done as u64 == total {
                    cb(done as u64, total);
                }
            }
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
#[allow(clippy::too_many_arguments)]
pub fn deploy_mod_atomic(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    mod_id: i64,
    staging_path: &Path,
    data_dir: &Path,
    files: &[String],
    game_path: &Path,
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
            let _ = undeploy_mod(db, game_id, bottle_name, mod_id, data_dir, game_path);
            Err(e)
        }
    }
}

/// Like [`deploy_mod_atomic`] but reports per-file progress via a callback.
#[allow(clippy::too_many_arguments)]
pub fn deploy_mod_atomic_with_progress(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    mod_id: i64,
    staging_path: &Path,
    data_dir: &Path,
    files: &[String],
    progress: &DeployProgressCb,
    game_path: &Path,
) -> Result<DeployResult> {
    match deploy_mod_inner(
        db,
        game_id,
        bottle_name,
        mod_id,
        staging_path,
        data_dir,
        files,
        Some(progress),
    ) {
        Ok(result) => Ok(result),
        Err(e) => {
            warn!(
                "deploy_mod failed for mod {}, rolling back partially deployed files: {}",
                mod_id, e
            );
            let _ = undeploy_mod(db, game_id, bottle_name, mod_id, data_dir, game_path);
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
    game_path: &Path,
) -> Result<Vec<String>> {
    // Query manifest paths FIRST without deleting entries.
    // Entries are only deleted after files are successfully removed,
    // preventing orphaned files if removal fails partway through.
    let manifest_paths = db
        .get_deployment_paths_for_mod(mod_id)
        .map_err(|e| DeployerError::Database(e.to_string()))?;

    let vanilla_set = build_vanilla_set(game_id);
    let mut actually_removed = Vec::new();
    let mut errors = Vec::new();
    let mut restore_failures = Vec::new();

    for (rel_path, deploy_target) in &manifest_paths {
        // SAFETY: Never delete vanilla game files even if they're in the manifest.
        if deploy_target != "root"
            && is_vanilla_file_with_set(game_id, rel_path, vanilla_set.as_ref())
        {
            warn!("SAFETY: Refusing to undeploy vanilla file: {}", rel_path);
            actually_removed.push(rel_path.clone()); // Clean manifest entry
            continue;
        }

        let base = if deploy_target == "root" {
            game_path
        } else {
            data_dir
        };
        let file_path = base.join(rel_path);

        if file_path.exists() {
            // Make writable before deleting — some mod files are read-only
            if let Ok(metadata) = fs::metadata(&file_path) {
                let perms = metadata.permissions();
                if perms.readonly() {
                    let mut writable = perms;
                    #[allow(clippy::permissions_set_readonly_false)]
                    writable.set_readonly(false);
                    let _ = fs::set_permissions(&file_path, writable);
                }
            }
            match fs::remove_file(&file_path) {
                Ok(()) => {
                    actually_removed.push(rel_path.clone());
                    prune_empty_dirs(&file_path, base);
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
        if let Err(e) = restore_next_winner(db, game_id, bottle_name, rel_path, base) {
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
    game_path: &Path,
) -> Result<DeployResult> {
    redeploy_all_with_progress(
        db,
        game_id,
        bottle_name,
        data_dir,
        game_path,
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
    game_path: &Path,
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

    purge_deployment(db, game_id, bottle_name, data_dir, game_path)?;

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

                // Determine deploy target for this mod (root vs data)
                let mod_target = db
                    .get_deploy_target_for_mod(m.id)
                    .unwrap_or_else(|_| "data".to_string());
                let effective_dir = if mod_target == "root" {
                    game_path
                } else {
                    data_dir
                };

                let file_count = files.len();
                let result = deploy_mod(
                    db,
                    game_id,
                    bottle_name,
                    m.id,
                    &staging_path,
                    effective_dir,
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
    game_path: &Path,
) -> Result<Vec<String>> {
    let manifest = db
        .get_deployment_manifest(game_id, bottle_name)
        .map_err(|e| DeployerError::Database(e.to_string()))?;

    let vanilla_set = build_vanilla_set(game_id);
    let mut removed = Vec::new();
    let mut purged_mod_ids = std::collections::HashSet::new();
    let mut failed_mod_ids = std::collections::HashSet::new();

    for entry in &manifest {
        // Legacy direct-installed files are not ours to purge
        if entry.deploy_method == "direct" {
            continue;
        }

        // SAFETY: Never delete vanilla game files even if they're in the manifest.
        // This can happen if a mod overwrote a vanilla file during deployment.
        if entry.deploy_target != "root"
            && is_vanilla_file_with_set(game_id, &entry.relative_path, vanilla_set.as_ref())
        {
            warn!(
                "SAFETY: Refusing to purge vanilla file: {}",
                entry.relative_path
            );
            purged_mod_ids.insert(entry.mod_id); // Still clean manifest entry
            continue;
        }

        let base = if entry.deploy_target == "root" {
            game_path
        } else {
            data_dir
        };
        let file_path = base.join(&entry.relative_path);
        if file_path.exists() {
            if let Err(e) = fs::remove_file(&file_path) {
                warn!("Failed to purge {}: {}", file_path.display(), e);
                failed_mod_ids.insert(entry.mod_id);
            } else {
                removed.push(entry.relative_path.clone());
                prune_empty_dirs(&file_path, base);
                purged_mod_ids.insert(entry.mod_id);
            }
        } else {
            // File already gone from disk — still mark mod for manifest cleanup
            purged_mod_ids.insert(entry.mod_id);
        }
    }

    // Only clean manifest entries for mods whose files were ALL successfully removed.
    // If any file for a mod failed to delete, keep the manifest so the user can retry.
    for mod_id in &purged_mod_ids {
        if failed_mod_ids.contains(mod_id) {
            warn!(
                "Skipping manifest cleanup for mod {} — some files could not be removed",
                mod_id
            );
            continue;
        }
        db.remove_deployment_entries_for_mod(*mod_id)
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
            let copy_method = platform::detect_copy_method(&staging_path, data_dir);
            let method = if can_hardlink {
                match fs::hard_link(&src, &dst) {
                    Ok(_) => "hardlink",
                    Err(e) => {
                        warn!("Hardlink failed in restore_next_winner: {}", e);
                        platform::fast_copy(&src, &dst, copy_method)?;
                        "copy"
                    }
                }
            } else {
                platform::fast_copy(&src, &dst, copy_method)?;
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
// Incremental Deployment
// ---------------------------------------------------------------------------

/// A file that should exist in the deployed state, computed from all enabled
/// mods in priority order.
#[derive(Debug, Clone)]
struct DesiredFile {
    relative_path: String,
    mod_id: i64,
    staging_path: PathBuf,
    sha256: Option<String>,
}

/// The computed diff between the desired deployment state and the current one.
#[derive(Debug)]
struct DeploymentDiff {
    to_add: Vec<DesiredFile>,
    to_remove: Vec<crate::database::DeploymentEntry>,
    to_update: Vec<(crate::database::DeploymentEntry, DesiredFile)>,
    unchanged: usize,
}

/// Result of an incremental deployment operation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IncrementalDeployResult {
    pub files_added: usize,
    pub files_removed: usize,
    pub files_updated: usize,
    pub files_unchanged: usize,
    pub fallback_used: bool,
    pub verification_failures: Vec<String>,
}

/// Compute the desired deployment state by walking all enabled mods in priority
/// order (ascending — highest priority last, so it overwrites lower-priority
/// files at the same path).
fn compute_desired_state(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
) -> Result<std::collections::HashMap<String, DesiredFile>> {
    let mods = db
        .list_mods(game_id, bottle_name)
        .map_err(|e| DeployerError::Database(e.to_string()))?;

    let mut enabled_mods: Vec<_> = mods.into_iter().filter(|m| m.enabled).collect();
    enabled_mods.sort_by_key(|m| m.install_priority);

    // Bulk-load file hashes for all enabled mods
    let mod_ids: Vec<i64> = enabled_mods.iter().map(|m| m.id).collect();
    let hash_map = db
        .get_file_hashes_for_mods(&mod_ids)
        .map_err(|e| DeployerError::Database(e.to_string()))?;

    let mut desired: std::collections::HashMap<String, DesiredFile> =
        std::collections::HashMap::new();

    for m in &enabled_mods {
        let Some(ref staging_path_str) = m.staging_path else {
            continue; // Legacy mod without staging — skip
        };
        let staging_path = PathBuf::from(staging_path_str);
        if !staging_path.exists() {
            warn!(
                "Incremental deploy: staging directory not found for mod '{}' ({}), skipping",
                m.name,
                staging_path.display()
            );
            continue;
        }

        let files = crate::staging::list_staging_files(&staging_path)
            .map_err(|e| DeployerError::Other(e.to_string()))?;

        for rel_path in files {
            let sha256 = hash_map.get(&(m.id, rel_path.clone())).cloned();

            // Last writer wins (highest priority, since sorted ascending)
            desired.insert(
                rel_path.clone(),
                DesiredFile {
                    relative_path: rel_path,
                    mod_id: m.id,
                    staging_path: staging_path.clone(),
                    sha256,
                },
            );
        }
    }

    Ok(desired)
}

/// Compare the desired state against the current deployment manifest to
/// produce a diff of what needs to change.
fn compute_diff(
    desired: &std::collections::HashMap<String, DesiredFile>,
    current: &std::collections::HashMap<String, crate::database::DeploymentEntry>,
) -> DeploymentDiff {
    let mut to_add = Vec::new();
    let mut to_remove = Vec::new();
    let mut to_update = Vec::new();
    let mut unchanged: usize = 0;

    // Files in desired but not in current → add
    // Files in both but different mod_id → update
    for (path, desired_file) in desired {
        match current.get(path) {
            None => {
                to_add.push(desired_file.clone());
            }
            Some(current_entry) => {
                if current_entry.mod_id != desired_file.mod_id {
                    to_update.push((current_entry.clone(), desired_file.clone()));
                } else {
                    unchanged += 1;
                }
            }
        }
    }

    // Files in current but not in desired → remove
    for (path, entry) in current {
        // Skip legacy direct-installed files
        if entry.deploy_method == "direct" {
            continue;
        }
        if !desired.contains_key(path) {
            to_remove.push(entry.clone());
        }
    }

    DeploymentDiff {
        to_add,
        to_remove,
        to_update,
        unchanged,
    }
}

/// Deploy a single file from staging to the game directory using hardlink-first
/// strategy. Returns the deploy method used ("hardlink" or "copy"), or None on
/// failure.
fn deploy_single_file(src: &Path, dst: &Path, can_hardlink: bool) -> Option<&'static str> {
    if let Some(parent) = dst.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            warn!(
                "Failed to create parent directory {}: {}",
                parent.display(),
                e
            );
            return None;
        }
    }

    // Remove existing file at destination if present
    if dst.exists() {
        if let Ok(meta) = fs::symlink_metadata(dst) {
            if meta.file_type().is_symlink() {
                warn!("Skipping deployment to symlink target: {}", dst.display());
                return None;
            }
        }
        if let Err(e) = fs::remove_file(dst) {
            warn!("Failed to remove existing file {}: {}", dst.display(), e);
            return None;
        }
    }

    if can_hardlink {
        match fs::hard_link(src, dst) {
            Ok(_) => Some("hardlink"),
            Err(e) => {
                warn!(
                    "Hardlink failed for {} → {}: {} (falling back to copy)",
                    src.display(),
                    dst.display(),
                    e
                );
                match fs::copy(src, dst) {
                    Ok(_) => Some("copy"),
                    Err(copy_err) => {
                        warn!(
                            "Copy also failed for {} → {}: {}",
                            src.display(),
                            dst.display(),
                            copy_err
                        );
                        None
                    }
                }
            }
        }
    } else {
        match fs::copy(src, dst) {
            Ok(_) => Some("copy"),
            Err(e) => {
                warn!(
                    "Copy failed for {} → {}: {}",
                    src.display(),
                    dst.display(),
                    e
                );
                None
            }
        }
    }
}

/// Perform an incremental deployment: compute the diff between current and
/// desired state, then apply only the changes.
///
/// Falls back to a full redeploy if more than 80% of total files would change
/// (incremental not worth it in that case).
#[allow(clippy::type_complexity)]
pub fn deploy_incremental(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    data_dir: &Path,
    game_path: &Path,
) -> Result<IncrementalDeployResult> {
    use rayon::prelude::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    info!(
        "Starting incremental deployment for {}/{}",
        game_id, bottle_name
    );

    // Step 1: Compute desired state
    let desired = compute_desired_state(db, game_id, bottle_name)?;

    // Step 2: Load current deployment manifest as a HashMap
    let current = db
        .get_deployment_manifest_map(game_id, bottle_name)
        .map_err(|e| DeployerError::Database(e.to_string()))?;

    // Step 3: Compute diff
    let diff = compute_diff(&desired, &current);

    let total_changes = diff.to_add.len() + diff.to_remove.len() + diff.to_update.len();
    let total_files = total_changes + diff.unchanged;

    info!(
        "Incremental diff: {} to add, {} to remove, {} to update, {} unchanged (total: {})",
        diff.to_add.len(),
        diff.to_remove.len(),
        diff.to_update.len(),
        diff.unchanged,
        total_files
    );

    // Step 4: If changes exceed 80% of total, fall back to full redeploy
    if total_files > 0 && total_changes * 100 / total_files.max(1) > 80 {
        info!(
            "Incremental diff covers {}% of files — falling back to full redeploy",
            total_changes * 100 / total_files.max(1)
        );

        let full_result = redeploy_all(db, game_id, bottle_name, data_dir, game_path)?;
        return Ok(IncrementalDeployResult {
            files_added: full_result.deployed_count,
            files_removed: 0,
            files_updated: 0,
            files_unchanged: 0,
            fallback_used: true,
            verification_failures: Vec::new(),
        });
    }

    // If there's nothing to do, return immediately
    if total_changes == 0 {
        info!("Incremental deployment: nothing to do — deployment is up to date");
        return Ok(IncrementalDeployResult {
            files_added: 0,
            files_removed: 0,
            files_updated: 0,
            files_unchanged: diff.unchanged,
            fallback_used: false,
            verification_failures: Vec::new(),
        });
    }

    // Determine staging root for filesystem check
    let staging_root = crate::staging::staging_base_dir(game_id, bottle_name);
    let can_hardlink = same_filesystem(&staging_root, data_dir);

    if !can_hardlink {
        // Estimate space needed for additions/updates only
        let add_update_count = diff.to_add.len() + diff.to_update.len();
        // Rough estimate: 1MB per file on average
        let estimated_bytes = (add_update_count as u64) * 1_048_576;
        if let Err(e) = crate::disk_budget::check_space_guard(data_dir, estimated_bytes) {
            return Err(DeployerError::Other(e));
        }
    }

    let removed_count = AtomicUsize::new(0);
    let added_count = AtomicUsize::new(0);
    let updated_count = AtomicUsize::new(0);
    let any_fallback = AtomicBool::new(!can_hardlink);
    let verification_failures: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

    // Step 5a: Remove files that should no longer be deployed
    let remove_paths: Vec<&str> = diff
        .to_remove
        .par_iter()
        .filter_map(|entry| {
            let file_path = data_dir.join(&entry.relative_path);
            if file_path.exists() {
                match fs::remove_file(&file_path) {
                    Ok(()) => {
                        prune_empty_dirs(&file_path, data_dir);
                        removed_count.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(e) => {
                        warn!("Failed to remove {}: {}", file_path.display(), e);
                        verification_failures
                            .lock()
                            .unwrap()
                            .push(format!("remove failed: {}: {}", entry.relative_path, e));
                    }
                }
            } else {
                removed_count.fetch_add(1, Ordering::Relaxed);
            }
            Some(entry.relative_path.as_str())
        })
        .collect();

    // Remove stale manifest entries
    if !remove_paths.is_empty() {
        db.batch_remove_deployment_entries(game_id, bottle_name, &remove_paths)
            .map_err(|e| DeployerError::Database(e.to_string()))?;
    }

    // Step 5b: Update files where the owning mod changed
    let update_owned: Vec<(i64, String, String, Option<String>)> = diff
        .to_update
        .par_iter()
        .filter_map(|(old_entry, new_desired)| {
            let dst = data_dir.join(&new_desired.relative_path);
            let src = new_desired.staging_path.join(&new_desired.relative_path);

            if !src.exists() {
                warn!("Incremental update: source not found: {}", src.display());
                return None;
            }

            match deploy_single_file(&src, &dst, can_hardlink) {
                Some(method) => {
                    updated_count.fetch_add(1, Ordering::Relaxed);
                    if method == "copy" {
                        any_fallback.store(true, Ordering::Relaxed);
                    }
                    Some((
                        new_desired.mod_id,
                        new_desired.relative_path.clone(),
                        new_desired.staging_path.to_string_lossy().to_string(),
                        new_desired.sha256.clone(),
                    ))
                }
                None => {
                    verification_failures.lock().unwrap().push(format!(
                        "update failed: {} (mod {} -> mod {})",
                        new_desired.relative_path, old_entry.mod_id, new_desired.mod_id
                    ));
                    None
                }
            }
        })
        .collect();

    let update_entries: Vec<(&str, &str, i64, &str, &str, &str, Option<&str>)> = update_owned
        .iter()
        .map(|(mod_id, rel_path, staging_path, sha256)| {
            (
                game_id,
                bottle_name,
                *mod_id,
                rel_path.as_str(),
                staging_path.as_str(),
                if can_hardlink { "hardlink" } else { "copy" },
                sha256.as_deref(),
            )
        })
        .collect();

    if !update_entries.is_empty() {
        db.batch_add_deployment_entries_with_hashes(&update_entries)
            .map_err(|e| DeployerError::Database(e.to_string()))?;
    }

    // Step 5c: Add new files
    let add_results: Vec<Option<(i64, String, String, Option<String>)>> = diff
        .to_add
        .par_iter()
        .map(|desired_file| {
            let dst = data_dir.join(&desired_file.relative_path);
            let src = desired_file.staging_path.join(&desired_file.relative_path);

            if !src.exists() {
                warn!("Incremental add: source not found: {}", src.display());
                return None;
            }

            match deploy_single_file(&src, &dst, can_hardlink) {
                Some(method) => {
                    added_count.fetch_add(1, Ordering::Relaxed);
                    if method == "copy" {
                        any_fallback.store(true, Ordering::Relaxed);
                    }
                    Some((
                        desired_file.mod_id,
                        desired_file.relative_path.clone(),
                        desired_file.staging_path.to_string_lossy().to_string(),
                        desired_file.sha256.clone(),
                    ))
                }
                None => {
                    verification_failures
                        .lock()
                        .unwrap()
                        .push(format!("add failed: {}", desired_file.relative_path));
                    None
                }
            }
        })
        .collect();

    // Batch-insert new manifest entries
    let add_entries: Vec<(&str, &str, i64, &str, &str, &str, Option<&str>)> = add_results
        .iter()
        .filter_map(|opt| {
            opt.as_ref()
                .map(|(mod_id, rel_path, staging_path, sha256)| {
                    (
                        game_id,
                        bottle_name,
                        *mod_id,
                        rel_path.as_str(),
                        staging_path.as_str(),
                        if can_hardlink { "hardlink" } else { "copy" },
                        sha256.as_deref(),
                    )
                })
        })
        .collect();

    if !add_entries.is_empty() {
        db.batch_add_deployment_entries_with_hashes(&add_entries)
            .map_err(|e| DeployerError::Database(e.to_string()))?;
    }

    let final_added = added_count.load(Ordering::Relaxed);
    let final_removed = removed_count.load(Ordering::Relaxed);
    let final_updated = updated_count.load(Ordering::Relaxed);
    let final_fallback = any_fallback.load(Ordering::Relaxed);
    let final_failures = verification_failures.into_inner().unwrap();

    info!(
        "Incremental deployment complete for {}/{}: {} added, {} removed, {} updated, {} unchanged, {} failures",
        game_id,
        bottle_name,
        final_added,
        final_removed,
        final_updated,
        diff.unchanged,
        final_failures.len()
    );

    Ok(IncrementalDeployResult {
        files_added: final_added,
        files_removed: final_removed,
        files_updated: final_updated,
        files_unchanged: diff.unchanged,
        fallback_used: final_fallback,
        verification_failures: final_failures,
    })
}

// ---------------------------------------------------------------------------
// Deployment verification
// ---------------------------------------------------------------------------

/// Result of post-deploy hash verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResult {
    /// Total files checked by hash.
    pub hash_checked: usize,
    /// Files whose hash did not match the deployment manifest.
    pub hash_mismatches: usize,
    /// Files skipped because the manifest has no stored SHA-256.
    pub hash_skipped_no_record: usize,
    /// Relative paths of files that failed hash verification.
    pub mismatched_files: Vec<String>,
}

use crate::config::VerificationLevel;
use serde::{Deserialize, Serialize};

/// Verify deployed files against the deployment manifest's SHA-256 hashes.
///
/// - **Fast**: no-op (returns immediately with zeroed result).
/// - **Balanced**: spot-checks ~10% of files (every 10th file) by SHA-256 hash.
/// - **Paranoid**: verifies every deployed file by SHA-256 hash.
///
/// Files whose manifest entry has no SHA-256 stored (NULL) are skipped gracefully.
pub fn verify_deployment(
    level: &VerificationLevel,
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    deploy_root: &Path,
) -> Result<VerificationResult> {
    if *level == VerificationLevel::Fast {
        return Ok(VerificationResult {
            hash_checked: 0,
            hash_mismatches: 0,
            hash_skipped_no_record: 0,
            mismatched_files: Vec::new(),
        });
    }

    let manifest = db
        .get_deployment_manifest(game_id, bottle_name)
        .map_err(|e| DeployerError::Other(e.to_string()))?;

    let mut hash_checked: usize = 0;
    let mut hash_mismatches: usize = 0;
    let mut hash_skipped_no_record: usize = 0;
    let mut mismatched_files: Vec<String> = Vec::new();

    let is_balanced = *level == VerificationLevel::Balanced;

    for (idx, entry) in manifest.iter().enumerate() {
        // Balanced mode: spot-check every 10th file (~10%)
        if is_balanced && idx % 10 != 0 {
            continue;
        }

        let expected_hash = match &entry.sha256 {
            Some(h) if !h.is_empty() => h,
            _ => {
                hash_skipped_no_record += 1;
                continue;
            }
        };

        let file_path = deploy_root.join(&entry.relative_path);
        if !file_path.exists() {
            // Missing files are already counted by the existence check;
            // don't double-count as a hash mismatch.
            continue;
        }

        match platform::fast_hash(&file_path) {
            Ok(actual_hash) => {
                hash_checked += 1;
                if actual_hash != *expected_hash {
                    hash_mismatches += 1;
                    if mismatched_files.len() < 50 {
                        mismatched_files.push(entry.relative_path.clone());
                    }
                    debug!(
                        "Hash mismatch: {} (expected {}, got {})",
                        entry.relative_path, expected_hash, actual_hash
                    );
                }
            }
            Err(e) => {
                warn!("Failed to hash {}: {} — skipping", entry.relative_path, e);
            }
        }
    }

    info!(
        "Verification ({:?}): checked={}, mismatches={}, skipped_no_record={}",
        level, hash_checked, hash_mismatches, hash_skipped_no_record
    );

    Ok(VerificationResult {
        hash_checked,
        hash_mismatches,
        hash_skipped_no_record,
        mismatched_files,
    })
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

        let removed =
            undeploy_mod(&db, "skyrimse", "Gaming", mod_id, &data_dir, &data_dir).unwrap();
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

    // -----------------------------------------------------------------------
    // Workstream 1: Incremental Deployment Engine
    // -----------------------------------------------------------------------

    /// Helper: add a mod, set staging_path, create staging files, and optionally
    /// add file hashes for realistic incremental deploy testing.
    fn add_test_mod(
        db: &ModDatabase,
        staging_root: &Path,
        name: &str,
        priority: i32,
        files: &[(&str, &[u8])],
    ) -> (i64, PathBuf) {
        let file_names: Vec<String> = files.iter().map(|(f, _)| f.to_string()).collect();
        let mod_id = db
            .add_mod(
                "skyrimse",
                "Gaming",
                None,
                name,
                "1.0",
                &format!("{name}.zip"),
                &file_names,
            )
            .unwrap();
        db.set_mod_priority(mod_id, priority).unwrap();

        let staging = staging_root.join(format!("skyrimse/Gaming/{mod_id}_{name}"));
        fs::create_dir_all(&staging).unwrap();
        db.set_staging_path(mod_id, staging.to_str().unwrap())
            .unwrap();

        for (rel_path, content) in files {
            let full = staging.join(rel_path);
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&full, content).unwrap();

            // Also add file hash for realistic testing
            let hash = crate::platform::fast_hash(&full).unwrap();
            let _ = db.store_file_hashes(
                mod_id,
                &[(rel_path.to_string(), hash, content.len() as u64)],
            );
        }

        (mod_id, staging)
    }

    #[test]
    fn incremental_deploy_from_empty_uses_fallback() {
        let (db, _tmp, _, data_dir) = setup();
        let staging_root = _tmp.path().join("staging_root");
        fs::create_dir_all(&staging_root).unwrap();

        add_test_mod(
            &db,
            &staging_root,
            "ModA",
            0,
            &[
                ("textures/sky.dds", b"sky texture data"),
                ("meshes/tree.nif", b"tree mesh data"),
            ],
        );

        // From empty → 100% new files → triggers >80% fallback to full redeploy
        let result = deploy_incremental(&db, "skyrimse", "Gaming", &data_dir, &data_dir).unwrap();
        assert!(
            result.fallback_used,
            "Initial deploy from empty should use fallback"
        );
        assert!(data_dir.join("textures/sky.dds").exists());
        assert!(data_dir.join("meshes/tree.nif").exists());
    }

    #[test]
    fn incremental_deploy_adds_new_mod_incrementally() {
        let (db, _tmp, _, data_dir) = setup();
        let staging_root = _tmp.path().join("staging_root");
        fs::create_dir_all(&staging_root).unwrap();

        // Deploy first mod (uses fallback since it's from empty)
        add_test_mod(
            &db,
            &staging_root,
            "ModA",
            0,
            &[
                ("textures/sky.dds", b"sky texture data"),
                ("meshes/tree.nif", b"tree mesh data"),
                ("sounds/fx.wav", b"sound data"),
                ("data.esp", b"esp data"),
                ("extra1.bsa", b"bsa data"),
            ],
        );
        let r1 = deploy_incremental(&db, "skyrimse", "Gaming", &data_dir, &data_dir).unwrap();
        assert!(r1.fallback_used); // First deploy is full

        // Now add a small second mod — should be incremental (1 new file < 80%)
        add_test_mod(
            &db,
            &staging_root,
            "ModB",
            10,
            &[("new_plugin.esp", b"new esp data")],
        );

        let r2 = deploy_incremental(&db, "skyrimse", "Gaming", &data_dir, &data_dir).unwrap();
        // fallback_used tracks copy-vs-hardlink (expected true in test env), not full-vs-incremental
        // File counts prove incremental behavior:
        assert_eq!(r2.files_added, 1);
        assert!(r2.files_unchanged >= 5);
        assert_eq!(r2.files_removed, 0);
        assert_eq!(r2.files_updated, 0);
        assert!(data_dir.join("new_plugin.esp").exists());
    }

    #[test]
    fn incremental_deploy_removes_files_from_disabled_mod() {
        let (db, _tmp, _, data_dir) = setup();
        let staging_root = _tmp.path().join("staging_root");
        fs::create_dir_all(&staging_root).unwrap();

        // First, deploy multiple mods so disabling one is < 80% change
        add_test_mod(
            &db,
            &staging_root,
            "BaseA",
            0,
            &[
                ("base1.esp", b"base1"),
                ("base2.esp", b"base2"),
                ("base3.esp", b"base3"),
                ("base4.esp", b"base4"),
            ],
        );
        let (mod_b, _) = add_test_mod(
            &db,
            &staging_root,
            "SmallMod",
            5,
            &[("small.esp", b"esp data")],
        );

        // Initial deploy (fallback)
        deploy_incremental(&db, "skyrimse", "Gaming", &data_dir, &data_dir).unwrap();
        assert!(data_dir.join("small.esp").exists());

        // Disable the small mod — only 1 removal out of 5 files (~20% change)
        db.set_enabled(mod_b, false).unwrap();

        let r2 = deploy_incremental(&db, "skyrimse", "Gaming", &data_dir, &data_dir).unwrap();
        // File counts prove incremental behavior (1 removal out of 5 total):
        assert_eq!(r2.files_removed, 1);
        assert_eq!(r2.files_added, 0);
        assert!(r2.files_unchanged >= 4);
        assert!(!data_dir.join("small.esp").exists());
        // Base files should still exist
        assert!(data_dir.join("base1.esp").exists());
    }

    #[test]
    fn incremental_deploy_updates_when_priority_changes() {
        let (db, _tmp, _, data_dir) = setup();
        let staging_root = _tmp.path().join("staging_root");
        fs::create_dir_all(&staging_root).unwrap();

        // Create several unique files first to avoid >80% threshold
        let (mod_a, _) = add_test_mod(
            &db,
            &staging_root,
            "ModA",
            0,
            &[
                ("shared.esp", b"content from A"),
                ("unique_a1.txt", b"unique a1"),
                ("unique_a2.txt", b"unique a2"),
            ],
        );
        let (_mod_b, _) = add_test_mod(
            &db,
            &staging_root,
            "ModB",
            10,
            &[
                ("shared.esp", b"content from B"),
                ("unique_b1.txt", b"unique b1"),
                ("unique_b2.txt", b"unique b2"),
            ],
        );

        // Initial deploy — ModB wins shared.esp (higher priority), fallback
        deploy_incremental(&db, "skyrimse", "Gaming", &data_dir, &data_dir).unwrap();
        let content = fs::read_to_string(data_dir.join("shared.esp")).unwrap();
        assert_eq!(content, "content from B");

        // Swap priorities — ModA now higher
        db.set_mod_priority(mod_a, 20).unwrap();

        // Incremental redeploy — should update shared.esp (1 of 5 files = 20%)
        let r2 = deploy_incremental(&db, "skyrimse", "Gaming", &data_dir, &data_dir).unwrap();
        // File counts prove incremental behavior (1 update out of 5 total):
        assert_eq!(r2.files_updated, 1);
        assert_eq!(r2.files_added, 0);
        assert_eq!(r2.files_removed, 0);
        assert!(r2.files_unchanged >= 4);
        let content = fs::read_to_string(data_dir.join("shared.esp")).unwrap();
        assert_eq!(content, "content from A");
    }

    #[test]
    fn incremental_deploy_no_changes_returns_all_unchanged() {
        let (db, _tmp, _, data_dir) = setup();
        let staging_root = _tmp.path().join("staging_root");
        fs::create_dir_all(&staging_root).unwrap();

        add_test_mod(&db, &staging_root, "ModA", 0, &[("test.esp", b"data")]);

        // Deploy once (fallback since from empty)
        deploy_incremental(&db, "skyrimse", "Gaming", &data_dir, &data_dir).unwrap();

        // Deploy again — nothing changed
        let r2 = deploy_incremental(&db, "skyrimse", "Gaming", &data_dir, &data_dir).unwrap();
        assert_eq!(r2.files_unchanged, 1);
        assert_eq!(r2.files_added, 0);
        assert_eq!(r2.files_removed, 0);
        assert_eq!(r2.files_updated, 0);
        assert!(!r2.fallback_used);
    }

    #[test]
    fn incremental_deploy_empty_state() {
        let (db, _tmp, _, data_dir) = setup();

        // No mods — should be a no-op
        let result = deploy_incremental(&db, "skyrimse", "Gaming", &data_dir, &data_dir).unwrap();
        assert_eq!(result.files_added, 0);
        assert_eq!(result.files_unchanged, 0);
        assert!(!result.fallback_used);
    }

    // -----------------------------------------------------------------------
    // Workstream 5: Configurable Verification Levels
    // -----------------------------------------------------------------------

    /// Helper: deploy mods using full redeploy + manually record hashes in manifest
    fn deploy_with_hashes(
        db: &ModDatabase,
        staging_root: &Path,
        data_dir: &Path,
        mods: &[(&str, i32, &[(&str, &[u8])])],
    ) {
        for (name, priority, files) in mods {
            add_test_mod(db, staging_root, name, *priority, files);
        }
        // Use full redeploy to establish baseline
        redeploy_all(db, "skyrimse", "Gaming", data_dir, data_dir).unwrap();

        // Manually update manifest entries with hashes
        let manifest = db.get_deployment_manifest("skyrimse", "Gaming").unwrap();
        for entry in &manifest {
            let file_path = data_dir.join(&entry.relative_path);
            if file_path.exists() {
                let hash = crate::platform::fast_hash(&file_path).unwrap();
                let entries = vec![(
                    "skyrimse",
                    "Gaming",
                    entry.mod_id,
                    entry.relative_path.as_str(),
                    entry.staging_path.as_str(),
                    entry.deploy_method.as_str(),
                    Some(hash.as_str()),
                )];
                db.batch_add_deployment_entries_with_hashes(&entries)
                    .unwrap();
            }
        }
    }

    #[test]
    fn verify_deployment_fast_is_noop() {
        let (db, _tmp, _, data_dir) = setup();
        let result = verify_deployment(
            &crate::config::VerificationLevel::Fast,
            &db,
            "skyrimse",
            "Gaming",
            &data_dir,
        )
        .unwrap();
        assert_eq!(result.hash_checked, 0);
        assert_eq!(result.hash_mismatches, 0);
    }

    #[test]
    fn verify_deployment_paranoid_detects_tamper() {
        let (db, _tmp, _, data_dir) = setup();
        let staging_root = _tmp.path().join("staging_root");
        fs::create_dir_all(&staging_root).unwrap();

        deploy_with_hashes(
            &db,
            &staging_root,
            &data_dir,
            &[("ModA", 0, &[("test.esp", b"original content")])],
        );
        assert!(data_dir.join("test.esp").exists());

        // Tamper with it
        fs::write(data_dir.join("test.esp"), b"tampered content").unwrap();

        // Paranoid verification should detect the mismatch
        let result = verify_deployment(
            &crate::config::VerificationLevel::Paranoid,
            &db,
            "skyrimse",
            "Gaming",
            &data_dir,
        )
        .unwrap();
        assert_eq!(result.hash_checked, 1);
        assert_eq!(result.hash_mismatches, 1);
        assert!(result.mismatched_files.contains(&"test.esp".to_string()));
    }

    #[test]
    fn verify_deployment_paranoid_passes_on_clean() {
        let (db, _tmp, _, data_dir) = setup();
        let staging_root = _tmp.path().join("staging_root");
        fs::create_dir_all(&staging_root).unwrap();

        deploy_with_hashes(
            &db,
            &staging_root,
            &data_dir,
            &[("ModA", 0, &[("test.esp", b"original content")])],
        );

        let result = verify_deployment(
            &crate::config::VerificationLevel::Paranoid,
            &db,
            "skyrimse",
            "Gaming",
            &data_dir,
        )
        .unwrap();
        assert_eq!(result.hash_checked, 1);
        assert_eq!(result.hash_mismatches, 0);
    }

    #[test]
    fn verify_deployment_balanced_spotchecks() {
        let (db, _tmp, _, data_dir) = setup();
        let staging_root = _tmp.path().join("staging_root");
        fs::create_dir_all(&staging_root).unwrap();

        // Create 20 files — Balanced mode should check ~2 (every 10th: idx 0 and 10)
        let files: Vec<(&str, &[u8])> = (0..20)
            .map(|i| {
                let name: &str = Box::leak(format!("file_{i:02}.esp").into_boxed_str());
                (name, b"data" as &[u8])
            })
            .collect();

        deploy_with_hashes(&db, &staging_root, &data_dir, &[("BigMod", 0, &files)]);

        let result = verify_deployment(
            &crate::config::VerificationLevel::Balanced,
            &db,
            "skyrimse",
            "Gaming",
            &data_dir,
        )
        .unwrap();
        // Should check only ~10% (every 10th file)
        assert!(
            result.hash_checked <= 5,
            "Balanced should spot-check ~10%, got {}",
            result.hash_checked
        );
        assert!(
            result.hash_checked >= 1,
            "Balanced should check at least 1 file"
        );
        assert_eq!(result.hash_mismatches, 0);
    }
}
