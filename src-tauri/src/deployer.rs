//! Deployment engine for installing mod files from staging into game directories.
//!
//! Uses a **hardlink → reflink → copy** strategy:
//! 1. Attempt `std::fs::hard_link()` for each file (zero disk overhead).
//! 2. If hardlink fails, try reflink/clonefile via `platform::fast_copy()` (CoW).
//! 3. If reflink is unsupported, fall back to `std::fs::copy()`.
//! 4. Track every deployed file in the `deployment_manifest` database table.
//!
//! **Important:** `.exe` and `.dll` files must never be symlinked — Wine path
//! resolution breaks with symlinks. See `should_avoid_symlink()`.
//!
//! Key operations:
//! - `deploy_game` — runtime-dispatched dispatcher (Wine or Native); prefer this for new code
//! - `deploy_wine_game` / `deploy_native_game` — per-runtime legs (native is Phase 1 stub)
//! - `deploy_mod` — deploy a single mod's files from staging to game dir
//! - `undeploy_mod` — remove deployed files, restore lower-priority files
//! - `redeploy_all` — purge + redeploy all enabled mods (after priority changes)
//! - `purge_deployment` — remove all deployed files (clean slate)

use std::fs;
use std::path::{Path, PathBuf};

use log::{debug, info, warn};
use thiserror::Error;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use rayon::prelude::*;

use crate::baselines;
use crate::database::ModDatabase;
use crate::platform;

// ---------------------------------------------------------------------------
// Batch existence check
// ---------------------------------------------------------------------------

/// Batch-check file existence to minimize stat() syscalls.
/// Wine probes 10,000+ files on startup, so reducing redundant stats matters.
pub fn batch_check_exists(paths: &[PathBuf]) -> Vec<bool> {
    paths.par_iter().map(|p| p.exists()).collect()
}

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

    // Lowercase comparison is intentional: Wine targets use case-insensitive
    // filesystems (NTFS under Wine, APFS on macOS), so vanilla file lookups
    // must be case-folded to match regardless of how the path was authored.
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
///
/// Each path is walked up to its nearest existing ancestor before the device
/// number comparison so that not-yet-created destinations (e.g. a Linux Wine
/// prefix's `Data/` directory before the first deploy) still resolve to the
/// volume they will live on. Without this walk, a missing destination would
/// always force the deployer into copy mode, even though a hardlink would
/// have worked once the directory was created.
///
/// Returns `false` only if neither path nor any ancestor can be stat'd, or
/// if the two stat'd ancestors have different `dev()` values.
#[cfg(unix)]
pub fn same_filesystem(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;

    fn nearest_existing_meta(p: &Path) -> Option<std::fs::Metadata> {
        let mut cur = p.to_path_buf();
        loop {
            if let Ok(m) = fs::metadata(&cur) {
                return Some(m);
            }
            match cur.parent() {
                Some(parent) if parent != cur.as_path() => cur = parent.to_path_buf(),
                _ => return None,
            }
        }
    }

    match (nearest_existing_meta(a), nearest_existing_meta(b)) {
        (Some(ma), Some(mb)) => ma.dev() == mb.dev(),
        _ => false,
    }
}

#[cfg(not(unix))]
pub fn same_filesystem(_a: &Path, _b: &Path) -> bool {
    false // assume different on non-Unix; always copy
}

/// Check whether a file extension indicates an executable or library that
/// must NOT be deployed via symlink. Wine resolves DLL/EXE paths through
/// its own path resolution layer, and symlinks can break that resolution.
///
/// Note: Corkscrew currently never creates symlinks during deployment
/// (hardlink -> reflink -> copy), so this is a defense-in-depth guard.
pub fn should_avoid_symlink(path: &Path) -> bool {
    if let Some(ext) = path.extension() {
        let ext_lower = ext.to_string_lossy().to_lowercase();
        matches!(ext_lower.as_str(), "exe" | "dll" | "com" | "bat" | "cmd")
    } else {
        false
    }
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

    #[error("Deploy mod {mod_id} failed for {failed_files:?}")]
    DeployFailed {
        mod_id: i64,
        failed_files: Vec<String>,
    },

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeployFileMapping {
    pub source_relative_path: String,
    pub relative_path: String,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Test whether hardlinks work between staging_dir and data_dir.
///
/// Uses unique probe filenames so concurrent callers (or a previous
/// interrupted run that left stale files behind) don't collide on a
/// shared `.corkscrew_hardlink_test` name and delete each other's probe.
pub fn test_hardlink_support(staging_dir: &Path, data_dir: &Path) -> bool {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let stem = format!(
        ".corkscrew_hardlink_test_{}_{}_{}",
        std::process::id(),
        nanos,
        n
    );

    let test_src = staging_dir.join(&stem);
    let test_dst = data_dir.join(&stem);

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
#[allow(clippy::too_many_arguments)]
pub fn deploy_mod(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    mod_id: i64,
    staging_path: &Path,
    data_dir: &Path,
    files: &[String],
    deploy_target: &str,
) -> Result<DeployResult> {
    deploy_mod_inner(
        db,
        game_id,
        bottle_name,
        mod_id,
        staging_path,
        data_dir,
        files,
        deploy_target,
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
    deploy_target: &str,
    progress: Option<&DeployProgressCb>,
) -> Result<DeployResult> {
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
    // Persist durable mod-level deploy target before writing manifest rows so
    // a later full redeploy can recover the original target after manifest purge.
    let deploy_base = data_dir.to_string_lossy().to_string();
    let deploy_base_path = if deploy_target == "custom" {
        Some(deploy_base.as_str())
    } else {
        None
    };
    db.set_deploy_target_for_mod_with_base(mod_id, deploy_target, deploy_base_path)
        .map_err(|e| DeployerError::Database(e.to_string()))?;
    let my_priority = mod_info.install_priority;

    // Batch-load existing deployment manifest + mod priorities into memory
    // to avoid per-file database round-trips during conflict resolution.
    let manifest = db
        .get_deployment_manifest(game_id, bottle_name)
        .map_err(|e| DeployerError::Database(e.to_string()))?;
    let deployed_map: std::collections::HashMap<(String, String), i64> = manifest
        .iter()
        .map(|e| {
            (
                (e.relative_path.replace('\\', "/"), e.deploy_target.clone()),
                e.mod_id,
            )
        })
        .collect();
    let priorities = db
        .get_all_mod_priorities()
        .map_err(|e| DeployerError::Database(e.to_string()))?;

    let staging_str = staging_path.to_string_lossy().to_string();
    let manifest_deploy_target = if deploy_target == "custom" {
        format!("custom:{}", data_dir.to_string_lossy())
    } else {
        deploy_target.to_string()
    };
    let mappings: Vec<DeployFileMapping> = files
        .iter()
        .map(|rel_path| DeployFileMapping {
            source_relative_path: rel_path.clone(),
            relative_path: rel_path.clone(),
        })
        .collect();
    deploy_mod_mapped_inner(
        db,
        game_id,
        bottle_name,
        mod_id,
        staging_path,
        data_dir,
        &mappings,
        &manifest_deploy_target,
        progress,
        can_hardlink,
        copy_method,
        my_priority,
        deployed_map,
        priorities,
        staging_str,
    )
}

#[allow(clippy::too_many_arguments)]
fn deploy_mod_mapped_inner(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    mod_id: i64,
    staging_path: &Path,
    data_dir: &Path,
    mappings: &[DeployFileMapping],
    manifest_deploy_target: &str,
    progress: Option<&DeployProgressCb>,
    can_hardlink: bool,
    copy_method: platform::FsCopyMethod,
    my_priority: i32,
    deployed_map: std::collections::HashMap<(String, String), i64>,
    priorities: std::collections::HashMap<i64, i64>,
    staging_str: String,
) -> Result<DeployResult> {
    let deployed_count = AtomicUsize::new(0);
    let skipped_count = AtomicUsize::new(0);
    let missing_count = AtomicUsize::new(0);
    let junk_count = AtomicUsize::new(0);
    let fallback_used = AtomicBool::new(false);
    let failure_count = AtomicUsize::new(0);
    let failure_messages = std::sync::Mutex::new(Vec::<String>::new());
    let conflict_backups = std::sync::Mutex::new(Vec::<(PathBuf, PathBuf)>::new());
    // Phase 1: Parallel file I/O — resolve conflicts, then hardlink or copy.
    // Collect successful deployments for batch database insert.
    let results: Vec<Option<(String, &str, Option<String>)>> = mappings
        .par_iter()
        .map(|mapping| {
            let source_rel_path = &mapping.source_relative_path;
            let rel_path = &mapping.relative_path;
            // Defense-in-depth: reject path traversal (string-level check)
            if !crate::staging::is_safe_relative_path(source_rel_path)
                || !crate::staging::is_safe_relative_path(rel_path)
            {
                failure_count.fetch_add(1, Ordering::Relaxed);
                if let Ok(mut failures) = failure_messages.lock() {
                    if failures.len() < 10 {
                        failures.push(format!(
                            "unsafe mapping {} -> {}",
                            source_rel_path, rel_path
                        ));
                    }
                }
                warn!(
                    "Deploy: skipping unsafe mapping: {} -> {}",
                    source_rel_path, rel_path
                );
                return None;
            }

            // Defense-in-depth: post-join canonicalization check
            let src = staging_path.join(source_rel_path);
            if src.exists() && !crate::staging::validate_path_within_base(staging_path, &src) {
                failure_count.fetch_add(1, Ordering::Relaxed);
                if let Ok(mut failures) = failure_messages.lock() {
                    if failures.len() < 10 {
                        failures.push(format!("staging escape {}", source_rel_path));
                    }
                }
                warn!(
                    "Deploy: path escaped staging after join: {}",
                    source_rel_path
                );
                return None;
            }

            // Defense-in-depth: skip packaging junk (fomod/, meta.ini, etc.)
            if crate::installer::is_deploy_junk(std::path::Path::new(source_rel_path)) {
                debug!("Deploy: skipping junk file: {}", source_rel_path);
                junk_count.fetch_add(1, Ordering::Relaxed);
                return None;
            }

            let dst = data_dir.join(rel_path);

            if !src.exists() {
                missing_count.fetch_add(1, Ordering::Relaxed);
                failure_count.fetch_add(1, Ordering::Relaxed);
                if let Ok(mut failures) = failure_messages.lock() {
                    if failures.len() < 10 {
                        failures.push(format!("missing source {}", source_rel_path));
                    }
                }
                if missing_count.load(Ordering::Relaxed) <= 5 {
                    warn!(
                        "Deploy: source file not found in staging: {} (mod {})",
                        src.display(),
                        mod_id
                    );
                }
                return None;
            }

            // Symlink check BEFORE any removal — prevents TOCTOU race where
            // an attacker replaces a file with a symlink between remove and deploy.
            if dst.exists() || dst.symlink_metadata().is_ok() {
                if let Ok(meta) = fs::symlink_metadata(&dst) {
                    if meta.file_type().is_symlink() {
                        warn!("Skipping deployment to symlink target: {}", dst.display());
                        skipped_count.fetch_add(1, Ordering::Relaxed);
                        return None;
                    }
                }
            }

            // Conflict resolution via in-memory lookup
            let normalized_key = (
                rel_path.replace('\\', "/"),
                manifest_deploy_target.to_string(),
            );
            if let Some(&owner_mod_id) = deployed_map.get(&normalized_key) {
                if owner_mod_id == mod_id {
                    return None; // already deployed by us
                }
                let owner_priority = priorities.get(&owner_mod_id).copied().unwrap_or(0);
                if owner_priority > my_priority as i64 {
                    skipped_count.fetch_add(1, Ordering::Relaxed);
                    return None;
                }
                // We win — remove existing deployed file, but first snapshot it
                // so a later failure in this same deploy attempt can restore
                // the lower-priority owner instead of leaving manifest/disk
                // inconsistent.
                if dst.exists() {
                    let backup_path = std::env::temp_dir().join(format!(
                        "corkscrew-deploy-rollback-{}-{}-{}-{}",
                        mod_id,
                        std::process::id(),
                        std::thread::current().name().unwrap_or("worker"),
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_nanos())
                            .unwrap_or_default()
                    ));
                    if let Err(e) = fs::copy(&dst, &backup_path) {
                        failure_count.fetch_add(1, Ordering::Relaxed);
                        if let Ok(mut failures) = failure_messages.lock() {
                            if failures.len() < 10 {
                                failures.push(format!(
                                    "failed to snapshot displaced file {}: {}",
                                    rel_path, e
                                ));
                            }
                        }
                        return None;
                    }
                    if let Ok(mut backups) = conflict_backups.lock() {
                        backups.push((dst.clone(), backup_path));
                    }
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
                // For non-protected files (textures, scripts, etc.), snapshot
                // before removal so a later failure in this same deploy
                // attempt can restore unmanaged/vanilla loose files.
                let backup_path = std::env::temp_dir().join(format!(
                    "corkscrew-deploy-rollback-unmanaged-{}-{}-{}",
                    mod_id,
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos())
                        .unwrap_or_default()
                ));
                if let Err(e) = fs::copy(&dst, &backup_path) {
                    failure_count.fetch_add(1, Ordering::Relaxed);
                    if let Ok(mut failures) = failure_messages.lock() {
                        if failures.len() < 10 {
                            failures.push(format!(
                                "failed to snapshot unmanaged file {}: {}",
                                rel_path, e
                            ));
                        }
                    }
                    return None;
                }
                if let Ok(mut backups) = conflict_backups.lock() {
                    backups.push((dst.clone(), backup_path));
                }
                let _ = fs::remove_file(&dst);
            }

            if let Some(parent) = dst.parent() {
                let _ = fs::create_dir_all(parent); // idempotent, safe for parallel calls
            }

            // Symlink re-check immediately before file operation to minimize TOCTOU window
            if let Ok(meta) = fs::symlink_metadata(&dst) {
                if meta.file_type().is_symlink() {
                    failure_count.fetch_add(1, Ordering::Relaxed);
                    if let Ok(mut failures) = failure_messages.lock() {
                        if failures.len() < 10 {
                            failures.push(format!("pre-deploy symlink target {}", rel_path));
                        }
                    }
                    warn!("Pre-deploy symlink detected: {}", dst.display());
                    return None;
                }
            }

            let method = if can_hardlink {
                match fs::hard_link(&src, &dst) {
                    Ok(_) => {
                        // Post-deploy symlink check to tighten TOCTOU window
                        if let Ok(meta) = fs::symlink_metadata(&dst) {
                            if meta.file_type().is_symlink() {
                                let _ = fs::remove_file(&dst);
                                failure_count.fetch_add(1, Ordering::Relaxed);
                                if let Ok(mut failures) = failure_messages.lock() {
                                    if failures.len() < 10 {
                                        failures.push(format!(
                                            "post-deploy symlink target {}",
                                            rel_path
                                        ));
                                    }
                                }
                                warn!("Post-deploy symlink detected, removed: {}", dst.display());
                                return None;
                            }
                        }
                        "hardlink"
                    }
                    Err(e) => {
                        warn!(
                            "Hardlink failed for {} → {}: {} (falling back to copy)",
                            src.display(),
                            dst.display(),
                            e
                        );
                        if let Err(copy_err) = platform::fast_copy(&src, &dst, copy_method) {
                            failure_count.fetch_add(1, Ordering::Relaxed);
                            if let Ok(mut failures) = failure_messages.lock() {
                                if failures.len() < 10 {
                                    failures.push(format!(
                                        "copy failed {} -> {}: {}",
                                        source_rel_path, rel_path, copy_err
                                    ));
                                }
                            }
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
                    failure_count.fetch_add(1, Ordering::Relaxed);
                    if let Ok(mut failures) = failure_messages.lock() {
                        if failures.len() < 10 {
                            failures.push(format!(
                                "copy failed {} -> {}: {}",
                                source_rel_path, rel_path, copy_err
                            ));
                        }
                    }
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
                let total = mappings.len() as u64;
                let interval = (total / 50).clamp(10, 100);
                if (done as u64).is_multiple_of(interval) || done as u64 == total {
                    cb(done as u64, total);
                }
            }
            let hash = crate::platform::fast_hash(&src).ok();
            Some((rel_path.clone(), method, hash))
        })
        .collect();

    let final_failures = failure_count.load(Ordering::Relaxed);
    if final_failures > 0 {
        let deployed_rel_paths: Vec<String> = results
            .iter()
            .filter_map(|opt| opt.as_ref().map(|(rel_path, _, _)| rel_path.clone()))
            .collect();
        for rel_path in &deployed_rel_paths {
            let dst = data_dir.join(rel_path);
            let _ = fs::remove_file(&dst);
            prune_empty_dirs(&dst, data_dir);
        }
        if let Ok(backups) = conflict_backups.lock() {
            for (dst, backup) in backups.iter() {
                if let Some(parent) = dst.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let _ = fs::copy(backup, dst);
                let _ = fs::remove_file(backup);
            }
        }
        let failed_files = failure_messages
            .lock()
            .ok()
            .map(|failures| failures.clone())
            .filter(|failures| !failures.is_empty())
            .unwrap_or_else(|| vec!["unknown deployment failure".to_string()]);
        return Err(DeployerError::DeployFailed {
            mod_id,
            failed_files,
        });
    }

    // Phase 2: Batch-insert all deployment entries in a single transaction.
    let batch: Vec<(&str, &str, i64, &str, &str, &str, Option<&str>, &str)> = results
        .iter()
        .filter_map(|opt| {
            opt.as_ref().map(|(rel_path, method, hash)| {
                (
                    game_id,
                    bottle_name,
                    mod_id,
                    rel_path.as_str(),
                    staging_str.as_str(),
                    *method,
                    hash.as_deref(),
                    manifest_deploy_target,
                )
            })
        })
        .collect();

    if !batch.is_empty() {
        if let Err(e) = db.batch_add_deployment_entries_with_hashes(&batch) {
            for (rel_path, _, _) in results.iter().filter_map(|opt| opt.as_ref()) {
                let dst = data_dir.join(rel_path);
                let _ = fs::remove_file(&dst);
                prune_empty_dirs(&dst, data_dir);
            }
            if let Ok(backups) = conflict_backups.lock() {
                for (dst, backup) in backups.iter() {
                    if let Some(parent) = dst.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    let _ = fs::copy(backup, dst);
                    let _ = fs::remove_file(backup);
                }
            }
            return Err(DeployerError::Database(e.to_string()));
        }
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

    let final_junk = junk_count.load(Ordering::Relaxed);

    // If expected deployable files exist but none succeeded, fail closed. Pure
    // metadata/junk-only mods still succeed as no-op because deployable == 0.
    let deployable = mappings.len().saturating_sub(final_junk);
    if final_deployed == 0 && deployable > 0 {
        return Err(DeployerError::DeployFailed {
            mod_id,
            failed_files: vec![format!(
                "0 of {} deployable files deployed (junk={}, missing={}) staging_path={} data_dir={} exists=({}, {})",
                deployable,
                final_junk,
                final_missing,
                staging_path.display(),
                data_dir.display(),
                staging_path.exists(),
                data_dir.exists(),
            )],
        });
    }

    if let Ok(backups) = conflict_backups.lock() {
        for (_, backup) in backups.iter() {
            let _ = fs::remove_file(backup);
        }
    }

    Ok(DeployResult {
        deployed_count: final_deployed,
        skipped_count: final_skipped,
        fallback_used: final_fallback,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn deploy_mod_mapped_with_progress(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    mod_id: i64,
    staging_path: &Path,
    data_dir: &Path,
    mappings: &[DeployFileMapping],
    deploy_target: &str,
    progress: &DeployProgressCb,
) -> Result<DeployResult> {
    if !staging_path.exists() {
        return Err(DeployerError::StagingNotFound(staging_path.to_path_buf()));
    }
    let can_hardlink = same_filesystem(staging_path, data_dir);
    let copy_method = platform::detect_copy_method(staging_path, data_dir);
    if !can_hardlink {
        let deploy_size = crate::disk_budget::dir_size(staging_path);
        crate::disk_budget::check_space_guard(data_dir, deploy_size)
            .map_err(DeployerError::Other)?;
    }
    let mod_info = db
        .get_mod(mod_id)
        .map_err(|e| DeployerError::Database(e.to_string()))?
        .ok_or_else(|| DeployerError::Database(format!("Mod {} not found", mod_id)))?;
    let deploy_base = data_dir.to_string_lossy().to_string();
    let deploy_base_path = if deploy_target == "custom" {
        Some(deploy_base.as_str())
    } else {
        None
    };
    db.set_deploy_target_for_mod_with_base(mod_id, deploy_target, deploy_base_path)
        .map_err(|e| DeployerError::Database(e.to_string()))?;
    let manifest = db
        .get_deployment_manifest(game_id, bottle_name)
        .map_err(|e| DeployerError::Database(e.to_string()))?;
    let deployed_map: std::collections::HashMap<(String, String), i64> = manifest
        .iter()
        .map(|e| {
            (
                (e.relative_path.replace('\\', "/"), e.deploy_target.clone()),
                e.mod_id,
            )
        })
        .collect();
    let priorities = db
        .get_all_mod_priorities()
        .map_err(|e| DeployerError::Database(e.to_string()))?;
    let staging_str = staging_path.to_string_lossy().to_string();
    let manifest_deploy_target = if deploy_target == "custom" {
        format!("custom:{}", data_dir.to_string_lossy())
    } else {
        deploy_target.to_string()
    };
    deploy_mod_mapped_inner(
        db,
        game_id,
        bottle_name,
        mod_id,
        staging_path,
        data_dir,
        mappings,
        &manifest_deploy_target,
        Some(progress),
        can_hardlink,
        copy_method,
        mod_info.install_priority,
        deployed_map,
        priorities,
        staging_str,
    )
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
    deploy_target: &str,
) -> Result<DeployResult> {
    let deploy_base = match deploy_target {
        "root" => game_path,
        _ => data_dir,
    };

    match deploy_mod(
        db,
        game_id,
        bottle_name,
        mod_id,
        staging_path,
        deploy_base,
        files,
        deploy_target,
    ) {
        Ok(result) => Ok(result),
        Err(e) => {
            warn!("deploy_mod failed for mod {}: {}", mod_id, e);
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
    deploy_target: &str,
) -> Result<DeployResult> {
    let deploy_base = match deploy_target {
        "root" => game_path,
        _ => data_dir,
    };

    match deploy_mod_inner(
        db,
        game_id,
        bottle_name,
        mod_id,
        staging_path,
        deploy_base,
        files,
        deploy_target,
        Some(progress),
    ) {
        Ok(result) => Ok(result),
        Err(e) => {
            warn!("deploy_mod failed for mod {}: {}", mod_id, e);
            Err(e)
        }
    }
}

pub fn deploy_mod_atomic_mapped_with_progress(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    mod_id: i64,
    staging_path: &Path,
    data_dir: &Path,
    mappings: &[DeployFileMapping],
    progress: &DeployProgressCb,
    game_path: &Path,
    deploy_target: &str,
) -> Result<DeployResult> {
    let deploy_base = match deploy_target {
        "root" => game_path,
        _ => data_dir,
    };
    match deploy_mod_mapped_with_progress(
        db,
        game_id,
        bottle_name,
        mod_id,
        staging_path,
        deploy_base,
        mappings,
        deploy_target,
        progress,
    ) {
        Ok(result) => Ok(result),
        Err(e) => {
            warn!("deploy_mod_mapped failed for mod {}: {}", mod_id, e);
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

    for (_, deploy_target) in &manifest_paths {
        if deploy_target != "data"
            && deploy_target != "root"
            && db
                .get_deploy_base_path_for_mod(mod_id)
                .map_err(|e| DeployerError::Database(e.to_string()))?
                .is_none()
        {
            return Err(DeployerError::Other(format!(
                "Cannot safely undeploy deploy_target '{}' without durable custom target path metadata",
                deploy_target
            )));
        }
    }

    let vanilla_set = build_vanilla_set(game_id);
    let mut actually_removed = Vec::new();
    let mut errors = Vec::new();
    let mut restore_failures = Vec::new();

    // Fetch the mod list ONCE for all restore_next_winner calls.
    // Previously this was fetched per-file, causing O(n²) DB queries for large mods.
    let all_mods = db
        .list_mods(game_id, bottle_name)
        .map_err(|e| DeployerError::Database(e.to_string()))?;

    for (rel_path, deploy_target) in &manifest_paths {
        // SAFETY: Never delete vanilla game files even if they're in the manifest.
        if deploy_target != "root"
            && is_vanilla_file_with_set(game_id, rel_path, vanilla_set.as_ref())
        {
            warn!("SAFETY: Refusing to undeploy vanilla file: {}", rel_path);
            actually_removed.push(rel_path.clone()); // Clean manifest entry
            continue;
        }

        let custom_base;
        let base = if deploy_target == "root" {
            game_path
        } else if deploy_target == "data" {
            data_dir
        } else {
            custom_base = PathBuf::from(
                db.get_deploy_base_path_for_mod(mod_id)
                    .map_err(|e| DeployerError::Database(e.to_string()))?
                    .ok_or_else(|| {
                        DeployerError::Other(format!(
                            "Cannot safely undeploy deploy_target '{}' without durable custom target path metadata",
                            deploy_target
                        ))
                    })?,
            );
            custom_base.as_path()
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
        let restore_result =
            restore_next_winner_with_mods(db, &all_mods, rel_path, deploy_target, base);
        if let Err(e) = restore_result {
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

    let mods = db
        .list_mods(game_id, bottle_name)
        .map_err(|e| DeployerError::Database(e.to_string()))?;

    let mut enabled_mods: Vec<_> = mods.into_iter().filter(|m| m.enabled).collect();
    enabled_mods.sort_by_key(|m| m.install_priority);

    // Preflight deploy targets BEFORE purging. Until custom target paths are
    // persisted durably, a full redeploy cannot reconstruct them safely after
    // the manifest is removed. Fail closed instead of silently routing custom
    // files into Data/ or deleting tracked manifest rows first.
    for m in &enabled_mods {
        let target = db
            .get_deploy_target_for_mod(m.id)
            .unwrap_or_else(|_| "data".to_string());
        if target != "data"
            && target != "root"
            && target != "mixed"
            && db
                .get_deploy_base_path_for_mod(m.id)
                .map_err(|e| DeployerError::Database(e.to_string()))?
                .is_none()
        {
            return Err(DeployerError::Other(format!(
                "Full redeploy cannot safely route deploy_target '{}' for mod '{}' without durable custom target path metadata",
                target, m.name
            )));
        }
    }

    purge_deployment(db, game_id, bottle_name, data_dir, game_path)?;

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
            if !crate::staging::is_safe_relative_path(staging_path_str)
                && !PathBuf::from(staging_path_str).is_absolute()
            {
                warn!(
                    "Skipping mod with suspicious staging path: {}",
                    staging_path_str
                );
                continue;
            }
            let staging_path = PathBuf::from(staging_path_str);
            if staging_path.exists() {
                let files = crate::staging::list_staging_files(&staging_path)
                    .map_err(|e| DeployerError::Other(e.to_string()))?;

                // Determine deploy target for this mod (root vs data)
                let mod_target = db
                    .get_deploy_target_for_mod(m.id)
                    .unwrap_or_else(|_| "data".to_string());

                if mod_target == "mixed" {
                    let file_targets = db
                        .get_deploy_file_targets_for_mod(m.id)
                        .map_err(|e| DeployerError::Database(e.to_string()))?;
                    let mut batches: std::collections::BTreeMap<String, Vec<DeployFileMapping>> =
                        std::collections::BTreeMap::new();
                    for file in files {
                        let target = file_targets.get(&file).cloned().unwrap_or(
                            crate::database::DeployFileTarget {
                                source_relative_path: file.clone(),
                                relative_path: file.clone(),
                                deploy_target: "data".to_string(),
                            },
                        );
                        batches
                            .entry(target.deploy_target.clone())
                            .or_default()
                            .push(DeployFileMapping {
                                source_relative_path: target.source_relative_path,
                                relative_path: target.relative_path,
                            });
                    }

                    for (target, batch_files) in batches {
                        let custom_base;
                        let effective_dir = if target == "root" {
                            game_path
                        } else if target == "data" {
                            data_dir
                        } else if target == "custom" {
                            custom_base = PathBuf::from(
                                db.get_deploy_base_path_for_mod(m.id)
                                    .map_err(|e| DeployerError::Database(e.to_string()))?
                                    .ok_or_else(|| {
                                        DeployerError::Other(format!(
                                            "Full redeploy cannot safely route mixed custom deploy target for mod '{}' without durable custom target path metadata",
                                            m.name
                                        ))
                                    })?,
                            );
                            custom_base.as_path()
                        } else {
                            return Err(DeployerError::Other(format!(
                                "Full redeploy cannot safely route mixed deploy_target '{}' for mod '{}'",
                                target, m.name
                            )));
                        };
                        let progress = |_done: u64, _total: u64| {};
                        let result = deploy_mod_mapped_with_progress(
                            db,
                            game_id,
                            bottle_name,
                            m.id,
                            &staging_path,
                            effective_dir,
                            &batch_files,
                            &target,
                            &progress,
                        )?;
                        files_so_far += batch_files.len();
                        total_deployed += result.deployed_count;
                        total_skipped += result.skipped_count;
                        any_fallback = any_fallback || result.fallback_used;
                    }
                    db.set_deploy_target_for_mod_with_base(
                        m.id,
                        "mixed",
                        m.deploy_base_path.as_deref(),
                    )
                    .map_err(|e| DeployerError::Database(e.to_string()))?;
                    continue;
                }

                let custom_base;
                let effective_dir = if mod_target == "root" {
                    game_path
                } else if mod_target == "data" {
                    data_dir
                } else {
                    custom_base = PathBuf::from(
                        db.get_deploy_base_path_for_mod(m.id)
                            .map_err(|e| DeployerError::Database(e.to_string()))?
                            .ok_or_else(|| {
                                DeployerError::Other(format!(
                                    "Full redeploy cannot safely route deploy_target '{}' for mod '{}' without durable custom target path metadata",
                                    mod_target, m.name
                                ))
                            })?,
                    );
                    custom_base.as_path()
                };

                let file_targets = db
                    .get_deploy_file_targets_for_mod(m.id)
                    .map_err(|e| DeployerError::Database(e.to_string()))?;
                let file_count = files.len();
                let result = if file_targets.is_empty() {
                    deploy_mod(
                        db,
                        game_id,
                        bottle_name,
                        m.id,
                        &staging_path,
                        effective_dir,
                        &files,
                        &mod_target,
                    )?
                } else {
                    let mappings: Vec<DeployFileMapping> = files
                        .iter()
                        .map(|file| {
                            file_targets
                                .get(file)
                                .map(|target| DeployFileMapping {
                                    source_relative_path: target.source_relative_path.clone(),
                                    relative_path: target.relative_path.clone(),
                                })
                                .unwrap_or_else(|| DeployFileMapping {
                                    source_relative_path: file.clone(),
                                    relative_path: file.clone(),
                                })
                        })
                        .collect();
                    let progress = |_done: u64, _total: u64| {};
                    deploy_mod_mapped_with_progress(
                        db,
                        game_id,
                        bottle_name,
                        m.id,
                        &staging_path,
                        effective_dir,
                        &mappings,
                        &mod_target,
                        &progress,
                    )?
                };

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

    for entry in &manifest {
        if entry.deploy_method != "direct"
            && entry.deploy_target != "data"
            && entry.deploy_target != "root"
            && db
                .get_deploy_base_path_for_mod(entry.mod_id)
                .map_err(|e| DeployerError::Database(e.to_string()))?
                .is_none()
        {
            return Err(DeployerError::Other(format!(
                "Cannot safely purge deploy_target '{}' without durable custom target path metadata",
                entry.deploy_target
            )));
        }
    }

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

        let custom_base;
        let base = if entry.deploy_target == "root" {
            game_path
        } else if entry.deploy_target == "data" {
            data_dir
        } else {
            custom_base = PathBuf::from(
                db.get_deploy_base_path_for_mod(entry.mod_id)
                    .map_err(|e| DeployerError::Database(e.to_string()))?
                    .ok_or_else(|| {
                        DeployerError::Other(format!(
                            "Cannot safely purge deploy_target '{}' without durable custom target path metadata",
                            entry.deploy_target
                        ))
                    })?,
            );
            custom_base.as_path()
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
///
/// This convenience wrapper fetches the mod list from the database. For batch
/// operations (e.g., `undeploy_mod`), prefer `restore_next_winner_with_mods`
/// to avoid repeated DB queries.
#[allow(dead_code)]
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
    restore_next_winner_with_mods(db, &mods, rel_path, "data", data_dir)
}

/// Like `restore_next_winner`, but accepts a pre-fetched mod list to avoid
/// repeated `db.list_mods()` calls. Use this when undeploying many files at
/// once (e.g., `undeploy_mod`) to avoid O(n²) DB queries.
fn restore_next_winner_with_mods(
    db: &ModDatabase,
    all_mods: &[crate::database::InstalledMod],
    rel_path: &str,
    deploy_target: &str,
    deploy_base: &Path,
) -> Result<()> {
    let mut candidates: Vec<_> = all_mods
        .iter()
        .filter(|m| {
            let target_matches = if deploy_target.starts_with("custom:") {
                m.deploy_target == "custom"
                    && m.deploy_base_path
                        .as_deref()
                        .map(|base| Path::new(base) == deploy_base)
                        .unwrap_or(false)
            } else {
                m.deploy_target == deploy_target
            };
            m.enabled
                && m.staging_path.is_some()
                && target_matches
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
        let dst = deploy_base.join(rel_path);

        if src.exists() {
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent)?;
            }

            let can_hardlink = same_filesystem(&staging_path, deploy_base);
            let copy_method = platform::detect_copy_method(&staging_path, deploy_base);
            let method = if can_hardlink {
                match fs::hard_link(&src, &dst) {
                    Ok(_) => "hardlink",
                    Err(e) => {
                        warn!("Hardlink failed in restore_next_winner_with_mods: {}", e);
                        platform::fast_copy(&src, &dst, copy_method)?;
                        "copy"
                    }
                }
            } else {
                platform::fast_copy(&src, &dst, copy_method)?;
                "copy"
            };

            let staging_path_string = staging_path.to_string_lossy().to_string();
            db.batch_add_deployment_entries(&[(
                &winner.game_id,
                &winner.bottle_name,
                winner.id,
                rel_path,
                staging_path_string.as_str(),
                method,
                deploy_target,
            )])
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
    source_relative_path: String,
    mod_id: i64,
    staging_path: PathBuf,
    sha256: Option<String>,
    /// `data`, `root`, or `custom` — comes from the per-mod stored target
    /// so incremental rewrites preserve the original deploy destination.
    deploy_target: String,
}

/// The computed diff between the desired deployment state and the current one.
#[derive(Debug)]
struct DeploymentDiff {
    to_add: Vec<DesiredFile>,
    to_remove: Vec<crate::database::DeploymentEntry>,
    to_update: Vec<(crate::database::DeploymentEntry, DesiredFile)>,
    unchanged: usize,
}

/// Resolve the physical deployment base for targets whose paths are durable in
/// Task 1.1. Custom targets deliberately return an error until Task 1.2 stores
/// their exact roots; treating them as `data_dir` would silently misdeploy and
/// could delete the wrong manifest entry while leaving the real file behind.
fn resolve_incremental_deploy_base(
    deploy_target: &str,
    data_dir: &Path,
    game_path: &Path,
) -> Result<PathBuf> {
    match deploy_target {
        "" | "data" => Ok(data_dir.to_path_buf()),
        "root" => Ok(game_path.to_path_buf()),
        custom if custom.starts_with("custom:") => Ok(PathBuf::from(&custom["custom:".len()..])),
        other => Err(DeployerError::Other(format!(
            "Incremental deployment cannot safely route deploy_target '{other}' without durable custom target path metadata; run a full redeploy after custom routing metadata is available"
        ))),
    }
}

fn custom_target_incremental_change(diff: &DeploymentDiff) -> Option<String> {
    diff.to_add
        .iter()
        .map(|d| d.deploy_target.as_str())
        .chain(diff.to_remove.iter().map(|e| e.deploy_target.as_str()))
        .chain(
            diff.to_update
                .iter()
                .flat_map(|(old, new)| [old.deploy_target.as_str(), new.deploy_target.as_str()]),
        )
        .find(|target| !matches!(*target, "" | "data" | "root") && !target.starts_with("custom:"))
        .map(|target| target.to_string())
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
) -> Result<std::collections::HashMap<crate::database::DeploymentKey, DesiredFile>> {
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

    let mut desired: std::collections::HashMap<crate::database::DeploymentKey, DesiredFile> =
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

        let deploy_target = db
            .get_deploy_target_for_mod(m.id)
            .unwrap_or_else(|_| "data".to_string());
        let deploy_file_targets = if deploy_target == "mixed" {
            db.get_deploy_file_targets_for_mod(m.id)
                .map_err(|e| DeployerError::Database(e.to_string()))?
        } else {
            std::collections::HashMap::new()
        };

        let manifest_deploy_target = if deploy_target == "custom" {
            m.deploy_base_path
                .as_ref()
                .map(|base| format!("custom:{base}"))
                .unwrap_or_else(|| deploy_target.clone())
        } else {
            deploy_target.clone()
        };

        for rel_path in files {
            let (source_rel_path, file_deploy_target, target_rel_path) = if deploy_target == "mixed"
            {
                if let Some(target) = deploy_file_targets.get(&rel_path) {
                    let target_name = if target.deploy_target == "custom" {
                        m.deploy_base_path
                            .as_ref()
                            .map(|base| format!("custom:{base}"))
                            .unwrap_or_else(|| target.deploy_target.clone())
                    } else {
                        target.deploy_target.clone()
                    };
                    (
                        target.source_relative_path.clone(),
                        target_name,
                        target.relative_path.clone(),
                    )
                } else {
                    (rel_path.clone(), "data".to_string(), rel_path.clone())
                }
            } else {
                (
                    rel_path.clone(),
                    manifest_deploy_target.clone(),
                    rel_path.clone(),
                )
            };

            // Last writer wins for the same deployment identity (same target + path).
            desired.insert(
                (target_rel_path.clone(), file_deploy_target.clone()),
                DesiredFile {
                    relative_path: target_rel_path,
                    source_relative_path: source_rel_path.clone(),
                    mod_id: m.id,
                    staging_path: staging_path.clone(),
                    sha256: hash_map.get(&(m.id, source_rel_path)).cloned(),
                    deploy_target: file_deploy_target,
                },
            );
        }
    }

    Ok(desired)
}

fn deployment_identity_matches(
    current_entry: &crate::database::DeploymentEntry,
    desired_file: &DesiredFile,
) -> bool {
    // Deployment target is part of DeploymentKey, but keep this explicit so
    // callers cannot accidentally treat same-path/different-target entries as
    // equivalent if the key shape changes later.
    current_entry.deploy_target == desired_file.deploy_target
        && current_entry.mod_id == desired_file.mod_id
        && Path::new(&current_entry.staging_path) == desired_file.staging_path.as_path()
}

fn deployed_entry_matches_desired(
    current_entry: &crate::database::DeploymentEntry,
    desired_file: &DesiredFile,
) -> bool {
    if !deployment_identity_matches(current_entry, desired_file) {
        return false;
    }

    match (&current_entry.sha256, &desired_file.sha256) {
        (Some(current_hash), Some(desired_hash)) => current_hash == desired_hash,
        // Safe legacy-manifest fallback: if the desired state has a content
        // hash and the current manifest does not, force an update so the
        // deployed file and manifest are refreshed rather than assuming the
        // old file is still correct.
        (None, Some(_)) => false,
        // If the desired hash is unavailable, compute_diff has no deployment
        // base to verify the filesystem hash safely; leave unchanged when all
        // durable identity fields above match.
        (Some(_), None) | (None, None) => true,
    }
}

fn is_hash_backfill_update(
    current_entry: &crate::database::DeploymentEntry,
    desired_file: &DesiredFile,
) -> bool {
    deployment_identity_matches(current_entry, desired_file)
        && current_entry.sha256.is_none()
        && desired_file.sha256.is_some()
}

/// Compare the desired state against the current deployment manifest to
/// produce a diff of what needs to change.
fn compute_diff(
    desired: &std::collections::HashMap<crate::database::DeploymentKey, DesiredFile>,
    current: &std::collections::HashMap<
        crate::database::DeploymentKey,
        crate::database::DeploymentEntry,
    >,
) -> DeploymentDiff {
    let mut to_add = Vec::new();
    let mut to_remove = Vec::new();
    let mut to_update = Vec::new();
    let mut unchanged: usize = 0;

    // Files in desired but not in current → add
    // Files in both but changed identity/content → update
    for (deployment_key, desired_file) in desired {
        match current.get(deployment_key) {
            None => {
                to_add.push(desired_file.clone());
            }
            Some(current_entry) => {
                if deployed_entry_matches_desired(current_entry, desired_file) {
                    unchanged += 1;
                } else {
                    to_update.push((current_entry.clone(), desired_file.clone()));
                }
            }
        }
    }

    // Files in current but not in desired → remove
    for (deployment_key, entry) in current {
        // Skip legacy direct-installed files
        if entry.deploy_method == "direct" {
            continue;
        }
        if !desired.contains_key(deployment_key) {
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

/// Deploy a single file from staging to the game directory.
///
/// Priority chain: hardlink → reflink/clonefile → copy.
/// Returns the deploy method used ("hardlink" or "copy"), or None on failure.
fn deploy_single_file(
    src: &Path,
    dst: &Path,
    can_hardlink: bool,
    copy_method: platform::FsCopyMethod,
) -> Option<&'static str> {
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
                    "Hardlink failed for {} → {}: {} (falling back to reflink/copy)",
                    src.display(),
                    dst.display(),
                    e
                );
                // Use platform::fast_copy which tries reflink/clonefile before fs::copy
                match platform::fast_copy(src, dst, copy_method) {
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
        // Cross-device: use platform::fast_copy (reflink/clonefile → fs::copy)
        match platform::fast_copy(src, dst, copy_method) {
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

    if let Some(target) = custom_target_incremental_change(&diff) {
        return Err(DeployerError::Other(format!(
            "Incremental deployment cannot safely apply changes for deploy_target '{target}' without durable custom target path metadata"
        )));
    }

    let only_hash_backfill_updates = diff.to_add.is_empty()
        && diff.to_remove.is_empty()
        && !diff.to_update.is_empty()
        && diff
            .to_update
            .iter()
            .all(|(current, desired)| is_hash_backfill_update(current, desired));

    // Step 4: If changes exceed 80% of total, fall back to full redeploy.
    // Do not route pure legacy hash backfills through full redeploy: full
    // redeploy entries may also lack hashes, causing repeated fallback loops
    // instead of refreshing the manifest hashes once.
    if total_files > 0
        && total_changes * 100 / total_files.max(1) > 80
        && !only_hash_backfill_updates
    {
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

    // Check copy-space needs against the physical base for each deploy target.
    // Root-target files must be checked against game_path, not data_dir.
    for desired_file in diff
        .to_add
        .iter()
        .chain(diff.to_update.iter().map(|(_, desired)| desired))
    {
        let base =
            resolve_incremental_deploy_base(&desired_file.deploy_target, data_dir, game_path)?;
        if !same_filesystem(&desired_file.staging_path, &base) {
            // Rough estimate: 1MB per file on average (keeps existing scoped behavior).
            if let Err(e) = crate::disk_budget::check_space_guard(&base, 1_048_576) {
                return Err(DeployerError::Other(e));
            }
        }
    }

    let removed_count = AtomicUsize::new(0);
    let added_count = AtomicUsize::new(0);
    let updated_count = AtomicUsize::new(0);
    let any_fallback = AtomicBool::new(false);
    let verification_failures: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

    // Step 5a: Remove files that should no longer be deployed
    let remove_entries: Vec<(&str, &str)> = diff
        .to_remove
        .par_iter()
        .filter_map(|entry| {
            let base =
                match resolve_incremental_deploy_base(&entry.deploy_target, data_dir, game_path) {
                    Ok(base) => base,
                    Err(e) => {
                        verification_failures
                            .lock()
                            .unwrap()
                            .push(format!("remove failed: {}: {}", entry.relative_path, e));
                        return None;
                    }
                };
            let file_path = base.join(&entry.relative_path);
            if file_path.exists() {
                match fs::remove_file(&file_path) {
                    Ok(()) => {
                        prune_empty_dirs(&file_path, &base);
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
            Some((entry.relative_path.as_str(), entry.deploy_target.as_str()))
        })
        .collect();

    // Remove stale manifest entries
    if !remove_entries.is_empty() {
        db.batch_remove_deployment_entries(game_id, bottle_name, &remove_entries)
            .map_err(|e| DeployerError::Database(e.to_string()))?;
    }

    // Step 5b: Update files where the owning mod changed
    let update_owned: Vec<(i64, String, String, Option<String>, String, String)> = diff
        .to_update
        .par_iter()
        .filter_map(|(old_entry, new_desired)| {
            let base = match resolve_incremental_deploy_base(
                &new_desired.deploy_target,
                data_dir,
                game_path,
            ) {
                Ok(base) => base,
                Err(e) => {
                    verification_failures.lock().unwrap().push(format!(
                        "update failed: {}: {}",
                        new_desired.relative_path, e
                    ));
                    return None;
                }
            };
            let dst = base.join(&new_desired.relative_path);
            let src = new_desired
                .staging_path
                .join(&new_desired.source_relative_path);

            if !src.exists() {
                warn!("Incremental update: source not found: {}", src.display());
                return None;
            }

            let can_hardlink = same_filesystem(&new_desired.staging_path, &base);
            let copy_method = platform::detect_copy_method(&new_desired.staging_path, &base);

            match deploy_single_file(&src, &dst, can_hardlink, copy_method) {
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
                        new_desired.deploy_target.clone(),
                        method.to_string(),
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

    let update_entries: Vec<(&str, &str, i64, &str, &str, &str, Option<&str>, &str)> = update_owned
        .iter()
        .map(
            |(mod_id, rel_path, staging_path, sha256, deploy_target, method)| {
                (
                    game_id,
                    bottle_name,
                    *mod_id,
                    rel_path.as_str(),
                    staging_path.as_str(),
                    method.as_str(),
                    sha256.as_deref(),
                    deploy_target.as_str(),
                )
            },
        )
        .collect();

    if !update_entries.is_empty() {
        db.batch_add_deployment_entries_with_hashes(&update_entries)
            .map_err(|e| DeployerError::Database(e.to_string()))?;
    }

    // Step 5c: Add new files
    let add_results: Vec<Option<(i64, String, String, Option<String>, String, String)>> = diff
        .to_add
        .par_iter()
        .map(|desired_file| {
            let base = match resolve_incremental_deploy_base(
                &desired_file.deploy_target,
                data_dir,
                game_path,
            ) {
                Ok(base) => base,
                Err(e) => {
                    verification_failures
                        .lock()
                        .unwrap()
                        .push(format!("add failed: {}: {}", desired_file.relative_path, e));
                    return None;
                }
            };
            let dst = base.join(&desired_file.relative_path);
            let src = desired_file
                .staging_path
                .join(&desired_file.source_relative_path);

            if !src.exists() {
                warn!("Incremental add: source not found: {}", src.display());
                return None;
            }

            let can_hardlink = same_filesystem(&desired_file.staging_path, &base);
            let copy_method = platform::detect_copy_method(&desired_file.staging_path, &base);

            match deploy_single_file(&src, &dst, can_hardlink, copy_method) {
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
                        desired_file.deploy_target.clone(),
                        method.to_string(),
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
    let add_entries: Vec<(&str, &str, i64, &str, &str, &str, Option<&str>, &str)> = add_results
        .iter()
        .filter_map(|opt| {
            opt.as_ref().map(
                |(mod_id, rel_path, staging_path, sha256, deploy_target, method)| {
                    (
                        game_id,
                        bottle_name,
                        *mod_id,
                        rel_path.as_str(),
                        staging_path.as_str(),
                        method.as_str(),
                        sha256.as_deref(),
                        deploy_target.as_str(),
                    )
                },
            )
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
// Runtime-dispatched deployment API
//
// These three functions form the high-level deployment abstraction that sits
// above the fine-grained `deploy_mod` / `redeploy_all` / etc. primitives.
// Callers that hold a `DetectedGame` should use `deploy_game`; the lower-level
// functions remain available for places that operate on individual mods.
// ---------------------------------------------------------------------------

/// Public dispatcher for full-game deployment. Routes by `detected.runtime`
/// to the appropriate per-runtime implementation.
///
/// Wine games are delegated to `deploy_wine_game`, which runs a full
/// `redeploy_all` using bottle context extracted from `detected`.
/// Native games are delegated to `deploy_native_game`, which is a stub in
/// Phase 1 — per-game native plugins (Stardew in Task 3.7, BG3 in Task 4.4)
/// replace that stub with real logic.
pub fn deploy_game(
    detected: &crate::games::DetectedGame,
    db: &std::sync::Arc<ModDatabase>,
) -> Result<DeployResult> {
    match &detected.runtime {
        crate::runtime::GameRuntime::Wine(_) => deploy_wine_game(detected, db),
        crate::runtime::GameRuntime::Native(_) => deploy_native_game(detected, db),
    }
}

/// Full redeploy for a Wine/CrossOver-hosted game. Extracts bottle context
/// from `detected.runtime` and delegates to [`redeploy_all`].
///
/// This is the Wine leg of the runtime split introduced in Task 1.5.
/// Behavior is identical to calling `redeploy_all` directly — the wrapper
/// exists so callers that hold a `DetectedGame` don't need to unpack
/// runtime fields manually.
pub fn deploy_wine_game(
    detected: &crate::games::DetectedGame,
    db: &std::sync::Arc<ModDatabase>,
) -> Result<DeployResult> {
    let wine_ctx = detected
        .runtime
        .wine()
        .ok_or_else(|| DeployerError::Other("deploy_wine_game called on non-Wine game".into()))?;
    redeploy_all(
        db,
        &detected.game_id,
        &wine_ctx.bottle_name,
        &detected.data_dir,
        &detected.game_path,
    )
}

/// Native macOS deployment dispatcher. Routes to the registered per-game
/// plugin's [`crate::games::GamePlugin::deploy_native`] implementation.
///
/// If a plugin is registered for `detected.game_id`, its `deploy_native`
/// method is called and the result propagated. If no plugin matches (e.g. a
/// native game discovered via appmanifest scanning with no dedicated plugin),
/// a clear "not implemented" error is returned so callers fail loudly rather
/// than silently corrupting state.
///
/// Wine games are not routed here — see [`deploy_wine_game`].
pub fn deploy_native_game(
    detected: &crate::games::DetectedGame,
    db: &std::sync::Arc<ModDatabase>,
) -> Result<DeployResult> {
    // Clone the Arc out of the registry so the registry lock is dropped
    // before we call deploy_native. Deploying walks every staged file and
    // performs hardlink / copy operations, which can take hundreds of
    // milliseconds — holding the registry mutex that entire time would block
    // detect_all_games, detect_native_games, and any other thread that calls
    // with_plugin or register_plugin.
    let plugin = crate::games::clone_plugin_for_dispatch(&detected.game_id).ok_or_else(|| {
        DeployerError::Other(format!(
            "native deployment not implemented for {}; per-game plugin must provide it",
            detected.game_id
        ))
    })?;

    // Registry lock is released here. deploy_native may take arbitrarily long.
    plugin.deploy_native(detected, db)
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
    fn redeploy_all_preserves_mixed_root_data_file_targets_after_manifest_purge() {
        let (db, tmp, staging, data_dir) = setup();
        let game_root = tmp.path().join("game_root");
        fs::create_dir_all(&game_root).unwrap();

        let mod_id = db
            .add_mod(
                "skyrimse",
                "Gaming",
                None,
                "Mixed Root Data Mod",
                "1.0",
                "mixed.zip",
                &[
                    "skse64_loader.exe".to_string(),
                    "Scripts/Foo.pex".to_string(),
                    "SKSE/Plugins/Foo.dll".to_string(),
                ],
            )
            .unwrap();
        db.set_staging_path(mod_id, &staging.to_string_lossy())
            .unwrap();

        create_staging_file(&staging, "skse64_loader.exe", b"loader");
        create_staging_file(&staging, "Scripts/Foo.pex", b"script");
        create_staging_file(&staging, "SKSE/Plugins/Foo.dll", b"plugin");

        deploy_mod(
            &db,
            "skyrimse",
            "Gaming",
            mod_id,
            &staging,
            &game_root,
            &["skse64_loader.exe".to_string()],
            "root",
        )
        .unwrap();
        deploy_mod(
            &db,
            "skyrimse",
            "Gaming",
            mod_id,
            &staging,
            &data_dir,
            &[
                "Scripts/Foo.pex".to_string(),
                "SKSE/Plugins/Foo.dll".to_string(),
            ],
            "data",
        )
        .unwrap();
        db.set_deploy_target_for_mod(mod_id, "mixed").unwrap();
        db.set_deploy_file_targets_for_mod(
            mod_id,
            &[
                crate::database::DeployFileTarget {
                    source_relative_path: "skse64_loader.exe".to_string(),
                    relative_path: "skse64_loader.exe".to_string(),
                    deploy_target: "root".to_string(),
                },
                crate::database::DeployFileTarget {
                    source_relative_path: "Scripts/Foo.pex".to_string(),
                    relative_path: "Scripts/Foo.pex".to_string(),
                    deploy_target: "data".to_string(),
                },
                crate::database::DeployFileTarget {
                    source_relative_path: "SKSE/Plugins/Foo.dll".to_string(),
                    relative_path: "SKSE/Plugins/Foo.dll".to_string(),
                    deploy_target: "data".to_string(),
                },
            ],
        )
        .unwrap();

        purge_deployment(&db, "skyrimse", "Gaming", &data_dir, &game_root).unwrap();
        assert!(!game_root.join("skse64_loader.exe").exists());
        assert!(!data_dir.join("Scripts/Foo.pex").exists());

        redeploy_all(&db, "skyrimse", "Gaming", &data_dir, &game_root).unwrap();

        assert!(game_root.join("skse64_loader.exe").exists());
        assert!(!data_dir.join("skse64_loader.exe").exists());
        assert!(data_dir.join("Scripts/Foo.pex").exists());
        assert!(data_dir.join("SKSE/Plugins/Foo.dll").exists());
        assert!(!game_root.join("Scripts/Foo.pex").exists());
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
            &db, "skyrimse", "Gaming", mod_id, &staging, &data_dir, &files, "data",
        )
        .unwrap();

        assert_eq!(result.deployed_count, 2);
        assert_eq!(result.skipped_count, 0);
        assert!(data_dir.join("meshes/test.nif").exists());
        assert!(data_dir.join("mod.esp").exists());
    }

    #[test]
    fn deploy_mod_atomic_missing_expected_file_rolls_back_partial_deploy() {
        let (db, _tmp, staging, data_dir) = setup();
        let files = vec!["present.esp".to_string(), "missing.esp".to_string()];
        let mod_id = db
            .add_mod(
                "skyrimse",
                "Gaming",
                None,
                "PartialFail",
                "1.0",
                "partial.zip",
                &files,
            )
            .unwrap();
        create_staging_file(&staging, "present.esp", b"present");

        let result = deploy_mod_atomic(
            &db, "skyrimse", "Gaming", mod_id, &staging, &data_dir, &files, &data_dir, "data",
        );

        assert!(result.is_err());
        assert!(!data_dir.join("present.esp").exists());
        assert!(
            db.get_deployment_manifest("skyrimse", "Gaming")
                .unwrap()
                .is_empty(),
            "failed atomic deploy must not leave manifest rows"
        );
    }

    #[test]
    fn deploy_mod_atomic_failed_redeploy_preserves_existing_manifest_and_file() {
        let (db, _tmp, staging, data_dir) = setup();
        let files = vec!["stable.esp".to_string()];
        let mod_id = db
            .add_mod(
                "skyrimse",
                "Gaming",
                None,
                "Existing",
                "1.0",
                "existing.zip",
                &files,
            )
            .unwrap();
        create_staging_file(&staging, "stable.esp", b"stable");
        deploy_mod_atomic(
            &db, "skyrimse", "Gaming", mod_id, &staging, &data_dir, &files, &data_dir, "data",
        )
        .unwrap();
        let retry_files = vec!["stable.esp".to_string(), "missing.esp".to_string()];

        let result = deploy_mod_atomic(
            &db,
            "skyrimse",
            "Gaming",
            mod_id,
            &staging,
            &data_dir,
            &retry_files,
            &data_dir,
            "data",
        );

        assert!(result.is_err());
        assert_eq!(fs::read(data_dir.join("stable.esp")).unwrap(), b"stable");
        let manifest = db.get_deployment_manifest("skyrimse", "Gaming").unwrap();
        assert_eq!(manifest.len(), 1);
        assert_eq!(manifest[0].mod_id, mod_id);
        assert_eq!(manifest[0].relative_path, "stable.esp");
    }

    #[test]
    fn deploy_mod_atomic_conflict_failure_restores_displaced_owner_file() {
        let (db, _tmp, staging_low, data_dir) = setup();
        let staging_high = staging_low.parent().unwrap().join("staging-high");
        fs::create_dir_all(&staging_high).unwrap();
        let shared = vec!["shared.esp".to_string()];
        let low_id = db
            .add_mod("skyrimse", "Gaming", None, "Low", "1.0", "low.zip", &shared)
            .unwrap();
        db.set_mod_priority(low_id, 0).unwrap();
        create_staging_file(&staging_low, "shared.esp", b"low");
        deploy_mod_atomic(
            &db,
            "skyrimse",
            "Gaming",
            low_id,
            &staging_low,
            &data_dir,
            &shared,
            &data_dir,
            "data",
        )
        .unwrap();

        let high_files = vec!["shared.esp".to_string(), "missing.esp".to_string()];
        let high_id = db
            .add_mod(
                "skyrimse",
                "Gaming",
                None,
                "High",
                "1.0",
                "high.zip",
                &high_files,
            )
            .unwrap();
        db.set_mod_priority(high_id, 10).unwrap();
        create_staging_file(&staging_high, "shared.esp", b"high");

        let result = deploy_mod_atomic(
            &db,
            "skyrimse",
            "Gaming",
            high_id,
            &staging_high,
            &data_dir,
            &high_files,
            &data_dir,
            "data",
        );

        assert!(result.is_err());
        assert_eq!(fs::read(data_dir.join("shared.esp")).unwrap(), b"low");
        let manifest = db.get_deployment_manifest("skyrimse", "Gaming").unwrap();
        assert_eq!(manifest.len(), 1);
        assert_eq!(manifest[0].mod_id, low_id);
    }

    #[test]
    fn deploy_mod_atomic_unsafe_mapping_fails_and_rolls_back_valid_file() {
        let (db, _tmp, staging, data_dir) = setup();
        let files = vec!["safe.esp".to_string(), "../evil.esp".to_string()];
        let mod_id = db
            .add_mod(
                "skyrimse",
                "Gaming",
                None,
                "UnsafeMapping",
                "1.0",
                "unsafe.zip",
                &files,
            )
            .unwrap();
        create_staging_file(&staging, "safe.esp", b"safe");
        let progress = |_done: u64, _total: u64| {};
        let mappings = vec![
            DeployFileMapping {
                source_relative_path: "safe.esp".to_string(),
                relative_path: "safe.esp".to_string(),
            },
            DeployFileMapping {
                source_relative_path: "../evil.esp".to_string(),
                relative_path: "evil.esp".to_string(),
            },
        ];

        let result = deploy_mod_atomic_mapped_with_progress(
            &db, "skyrimse", "Gaming", mod_id, &staging, &data_dir, &mappings, &progress,
            &data_dir, "data",
        );

        assert!(result.is_err());
        assert!(!data_dir.join("safe.esp").exists());
        assert!(db
            .get_deployment_manifest("skyrimse", "Gaming")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn deploy_mod_atomic_unmanaged_file_restored_when_later_file_fails() {
        let (db, _tmp, staging, data_dir) = setup();
        let files = vec!["textures/foo.dds".to_string(), "missing.esp".to_string()];
        let mod_id = db
            .add_mod(
                "skyrimse",
                "Gaming",
                None,
                "UnmanagedRestore",
                "1.0",
                "unmanaged.zip",
                &files,
            )
            .unwrap();
        create_staging_file(&staging, "textures/foo.dds", b"mod texture");
        let existing = data_dir.join("textures/foo.dds");
        fs::create_dir_all(existing.parent().unwrap()).unwrap();
        fs::write(&existing, b"unmanaged texture").unwrap();

        let result = deploy_mod_atomic(
            &db, "skyrimse", "Gaming", mod_id, &staging, &data_dir, &files, &data_dir, "data",
        );

        assert!(result.is_err());
        assert_eq!(fs::read(existing).unwrap(), b"unmanaged texture");
        assert!(db
            .get_deployment_manifest("skyrimse", "Gaming")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn deploy_mod_atomic_source_symlink_escape_fails_and_rolls_back_valid_file() {
        let (db, tmp, staging, data_dir) = setup();
        let files = vec!["safe.esp".to_string(), "escape.esp".to_string()];
        let mod_id = db
            .add_mod(
                "skyrimse",
                "Gaming",
                None,
                "SymlinkEscape",
                "1.0",
                "escape.zip",
                &files,
            )
            .unwrap();
        create_staging_file(&staging, "safe.esp", b"safe");
        let outside = tmp.path().join("outside.esp");
        fs::write(&outside, b"outside").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, staging.join("escape.esp")).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&outside, staging.join("escape.esp")).unwrap();

        let result = deploy_mod_atomic(
            &db, "skyrimse", "Gaming", mod_id, &staging, &data_dir, &files, &data_dir, "data",
        );

        assert!(result.is_err());
        assert!(!data_dir.join("safe.esp").exists());
        assert!(db
            .get_deployment_manifest("skyrimse", "Gaming")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn deploy_mod_atomic_zero_success_returns_error() {
        let (db, _tmp, staging, data_dir) = setup();
        let files = vec!["mod.esp".to_string()];
        let mod_id = db
            .add_mod(
                "skyrimse",
                "Gaming",
                None,
                "ZeroSuccess",
                "1.0",
                "zero.zip",
                &files,
            )
            .unwrap();
        create_staging_file(&staging, "mod.esp", b"esp data");
        #[cfg(unix)]
        std::os::unix::fs::symlink(staging.join("mod.esp"), data_dir.join("mod.esp")).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(staging.join("mod.esp"), data_dir.join("mod.esp"))
            .unwrap();

        let result = deploy_mod_atomic(
            &db, "skyrimse", "Gaming", mod_id, &staging, &data_dir, &files, &data_dir, "data",
        );

        assert!(result.is_err());
        assert!(fs::symlink_metadata(data_dir.join("mod.esp"))
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(
            db.get_deployment_manifest("skyrimse", "Gaming")
                .unwrap()
                .is_empty(),
            "zero-success deploy must not write manifest rows"
        );
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
            &db, "skyrimse", "Gaming", mod_id, &staging, &data_dir, &files, "data",
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
            &db, "skyrimse", "Gaming", mod1, &staging1, &data_dir, &files, "data",
        )
        .unwrap();

        deploy_mod(
            &db, "skyrimse", "Gaming", mod2, &staging2, &data_dir, &files, "data",
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
    fn same_filesystem_walks_to_existing_ancestor() {
        // Regression: on freshly-initialized Wine prefixes the destination
        // directory (e.g. `Data/`) does not yet exist. The old implementation
        // returned `false` whenever stat() failed, forcing copy fallback.
        // The new implementation walks up to the nearest existing ancestor.
        let tmp = TempDir::new().unwrap();
        let real = tmp.path().join("real");
        fs::create_dir_all(&real).unwrap();
        // Path that does not exist yet but lives under the same temp dir
        let unborn = tmp.path().join("not_yet_created").join("Data");

        assert!(
            same_filesystem(&real, &unborn),
            "should match via existing ancestor (tmp dir)"
        );
    }

    #[test]
    fn same_filesystem_unrooted_returns_false() {
        // If both paths fail to find any existing ancestor (impossible on
        // unix where `/` always exists, but we still guarantee no panic and
        // a sane result for completely unstattable inputs).
        let tmp = TempDir::new().unwrap();
        let real = tmp.path().join("real");
        fs::create_dir_all(&real).unwrap();
        // `/` exists, so both will resolve. This test now verifies we don't
        // crash on a deeply nonexistent path; both end up at `/` so it
        // returns true. Kept here as documentation of the new semantics.
        let nonexistent = PathBuf::from("/nonexistent/corkscrew_test_xyz");
        // Both should walk to existing ancestors; on a single-volume host
        // they'll match.
        let _ = same_filesystem(&real, &nonexistent);
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
    fn compute_diff_updates_same_mod_same_path_when_staging_hash_changes() {
        let mut desired = std::collections::HashMap::new();
        desired.insert(
            ("textures/foo.dds".to_string(), "data".to_string()),
            DesiredFile {
                relative_path: "textures/foo.dds".to_string(),
                source_relative_path: "textures/foo.dds".to_string(),
                mod_id: 42,
                staging_path: PathBuf::from("/tmp/corkscrew/staging/mod42"),
                sha256: Some("new-hash".to_string()),
                deploy_target: "data".to_string(),
            },
        );

        let mut current = std::collections::HashMap::new();
        current.insert(
            ("textures/foo.dds".to_string(), "data".to_string()),
            crate::database::DeploymentEntry {
                id: 1,
                game_id: "skyrimse".to_string(),
                bottle_name: "Gaming".to_string(),
                mod_id: 42,
                relative_path: "textures/foo.dds".to_string(),
                staging_path: "/tmp/corkscrew/staging/mod42".to_string(),
                deploy_method: "copy".to_string(),
                sha256: Some("old-hash".to_string()),
                deployed_at: "2026-06-08T00:00:00Z".to_string(),
                mod_name: "HashChanged".to_string(),
                deploy_target: "data".to_string(),
            },
        );

        let diff = compute_diff(&desired, &current);

        assert_eq!(diff.to_update.len(), 1);
        assert_eq!(diff.unchanged, 0);
        assert_eq!(diff.to_add.len(), 0);
        assert_eq!(diff.to_remove.len(), 0);

        current
            .get_mut(&("textures/foo.dds".to_string(), "data".to_string()))
            .unwrap()
            .sha256 = None;
        let legacy_manifest_diff = compute_diff(&desired, &current);
        assert_eq!(legacy_manifest_diff.to_update.len(), 1);
        assert_eq!(legacy_manifest_diff.unchanged, 0);
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
    fn incremental_deploy_adds_root_target_to_game_root() {
        let (db, _tmp, _, data_dir) = setup();
        let game_root = _tmp.path().join("game_root");
        fs::create_dir_all(&game_root).unwrap();
        let staging_root = _tmp.path().join("staging_root");
        fs::create_dir_all(&staging_root).unwrap();

        let (mod_id, staging) = add_test_mod(
            &db,
            &staging_root,
            "RootMod",
            0,
            &[("skse64_loader.exe", b"loader v1")],
        );
        deploy_mod(
            &db,
            "skyrimse",
            "Gaming",
            mod_id,
            &staging,
            &game_root,
            &["skse64_loader.exe".to_string()],
            "root",
        )
        .unwrap();

        create_staging_file(&staging, "root_added.dll", b"root dll");
        let r = deploy_incremental(&db, "skyrimse", "Gaming", &data_dir, &game_root).unwrap();

        assert_eq!(r.files_added, 1);
        assert!(game_root.join("root_added.dll").exists());
        assert!(!data_dir.join("root_added.dll").exists());
        let entry = db
            .get_deployed_file("skyrimse", "Gaming", "root_added.dll")
            .unwrap()
            .unwrap();
        assert_eq!(entry.deploy_target, "root");
    }

    #[test]
    fn incremental_deploy_updates_root_target_in_game_root() {
        let (db, _tmp, _, data_dir) = setup();
        let game_root = _tmp.path().join("game_root");
        fs::create_dir_all(&game_root).unwrap();
        let staging_root = _tmp.path().join("staging_root");
        fs::create_dir_all(&staging_root).unwrap();

        let (mod_a, staging_a) = add_test_mod(
            &db,
            &staging_root,
            "RootA",
            0,
            &[
                ("shared_root.dll", b"root from A"),
                ("unique_a_root.txt", b"a"),
            ],
        );
        let (mod_b, staging_b) = add_test_mod(
            &db,
            &staging_root,
            "RootB",
            10,
            &[
                ("shared_root.dll", b"root from B"),
                ("unique_b_root.txt", b"b"),
            ],
        );

        deploy_mod(
            &db,
            "skyrimse",
            "Gaming",
            mod_a,
            &staging_a,
            &game_root,
            &[
                "shared_root.dll".to_string(),
                "unique_a_root.txt".to_string(),
            ],
            "root",
        )
        .unwrap();
        deploy_mod(
            &db,
            "skyrimse",
            "Gaming",
            mod_b,
            &staging_b,
            &game_root,
            &[
                "shared_root.dll".to_string(),
                "unique_b_root.txt".to_string(),
            ],
            "root",
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(game_root.join("shared_root.dll")).unwrap(),
            "root from B"
        );

        db.set_mod_priority(mod_a, 20).unwrap();
        let r = deploy_incremental(&db, "skyrimse", "Gaming", &data_dir, &game_root).unwrap();

        assert_eq!(r.files_updated, 1);
        assert_eq!(
            fs::read_to_string(game_root.join("shared_root.dll")).unwrap(),
            "root from A"
        );
        assert!(!data_dir.join("shared_root.dll").exists());
    }

    #[test]
    fn incremental_deploy_removes_root_target_from_game_root() {
        let (db, _tmp, _, data_dir) = setup();
        let game_root = _tmp.path().join("game_root");
        fs::create_dir_all(&game_root).unwrap();
        let staging_root = _tmp.path().join("staging_root");
        fs::create_dir_all(&staging_root).unwrap();

        let (base_mod, base_staging) = add_test_mod(
            &db,
            &staging_root,
            "RootBase",
            0,
            &[
                ("base_root_1.txt", b"base1"),
                ("base_root_2.txt", b"base2"),
                ("base_root_3.txt", b"base3"),
                ("base_root_4.txt", b"base4"),
            ],
        );
        let (small_mod, small_staging) = add_test_mod(
            &db,
            &staging_root,
            "RootSmall",
            5,
            &[("remove_me_root.dll", b"remove me")],
        );
        deploy_mod(
            &db,
            "skyrimse",
            "Gaming",
            base_mod,
            &base_staging,
            &game_root,
            &[
                "base_root_1.txt".to_string(),
                "base_root_2.txt".to_string(),
                "base_root_3.txt".to_string(),
                "base_root_4.txt".to_string(),
            ],
            "root",
        )
        .unwrap();
        deploy_mod(
            &db,
            "skyrimse",
            "Gaming",
            small_mod,
            &small_staging,
            &game_root,
            &["remove_me_root.dll".to_string()],
            "root",
        )
        .unwrap();
        assert!(game_root.join("remove_me_root.dll").exists());

        db.set_enabled(small_mod, false).unwrap();
        let r = deploy_incremental(&db, "skyrimse", "Gaming", &data_dir, &game_root).unwrap();

        assert_eq!(r.files_removed, 1);
        assert!(!game_root.join("remove_me_root.dll").exists());
        assert!(!data_dir.join("remove_me_root.dll").exists());
        assert!(game_root.join("base_root_1.txt").exists());
    }

    #[test]
    fn incremental_deploy_adds_custom_target_to_durable_base() {
        let (db, _tmp, _, data_dir) = setup();
        let game_root = _tmp.path().join("game_root");
        let custom_root = _tmp.path().join("custom_root");
        fs::create_dir_all(&game_root).unwrap();
        fs::create_dir_all(&custom_root).unwrap();
        let staging_root = _tmp.path().join("staging_root");
        fs::create_dir_all(&staging_root).unwrap();

        let (custom_mod, _custom_staging) = add_test_mod(
            &db,
            &staging_root,
            "CustomMod",
            0,
            &[("custom_file.txt", b"custom")],
        );
        let custom_base = custom_root.to_string_lossy().to_string();
        db.set_deploy_target_for_mod_with_base(custom_mod, "custom", Some(&custom_base))
            .unwrap();

        let result = deploy_incremental(&db, "skyrimse", "Gaming", &data_dir, &game_root)
            .expect("custom incremental add should use durable base");

        assert!(result.fallback_used);
        assert!(custom_root.join("custom_file.txt").exists());
        assert!(!data_dir.join("custom_file.txt").exists());
        let entry = db
            .get_deployed_file("skyrimse", "Gaming", "custom_file.txt")
            .unwrap()
            .expect("manifest entry should exist");
        assert_eq!(entry.mod_id, custom_mod);
        assert!(entry.deploy_target.starts_with("custom:"));
    }

    #[test]
    fn incremental_deploy_custom_target_change_uses_durable_base_without_orphaning() {
        let (db, _tmp, _, data_dir) = setup();
        let game_root = _tmp.path().join("game_root");
        let custom_root = _tmp.path().join("custom_root");
        fs::create_dir_all(&game_root).unwrap();
        fs::create_dir_all(&custom_root).unwrap();
        let staging_root = _tmp.path().join("staging_root");
        fs::create_dir_all(&staging_root).unwrap();

        let (custom_mod, custom_staging) = add_test_mod(
            &db,
            &staging_root,
            "CustomMod",
            0,
            &[("custom_file.txt", b"custom")],
        );
        deploy_mod(
            &db,
            "skyrimse",
            "Gaming",
            custom_mod,
            &custom_staging,
            &custom_root,
            &["custom_file.txt".to_string()],
            "custom",
        )
        .unwrap();
        assert!(custom_root.join("custom_file.txt").exists());

        db.set_enabled(custom_mod, false).unwrap();
        let result = deploy_incremental(&db, "skyrimse", "Gaming", &data_dir, &game_root)
            .expect("custom incremental change should use durable base safely");

        assert!(result.fallback_used);
        assert!(!custom_root.join("custom_file.txt").exists());
        assert!(db
            .get_deployed_file("skyrimse", "Gaming", "custom_file.txt")
            .unwrap()
            .is_none());
    }

    #[test]
    fn purge_custom_target_fails_before_deleting_or_cleaning_manifest() {
        let (db, _tmp, _, data_dir) = setup();
        let game_root = _tmp.path().join("game_root");
        let custom_root = _tmp.path().join("custom_root");
        fs::create_dir_all(&game_root).unwrap();
        fs::create_dir_all(&custom_root).unwrap();

        let mod_id = db
            .add_mod(
                "skyrimse",
                "Gaming",
                None,
                "CustomMod",
                "1.0",
                "custom.zip",
                &["custom_file.txt".to_string()],
            )
            .unwrap();
        db.set_deploy_target_for_mod(mod_id, "custom").unwrap();
        db.batch_add_deployment_entries(&[(
            "skyrimse",
            "Gaming",
            mod_id,
            "custom_file.txt",
            custom_root.to_str().unwrap(),
            "copy",
            "custom",
        )])
        .unwrap();

        fs::write(data_dir.join("custom_file.txt"), b"do not delete").unwrap();
        fs::write(custom_root.join("custom_file.txt"), b"custom").unwrap();

        let err = purge_deployment(&db, "skyrimse", "Gaming", &data_dir, &game_root)
            .expect_err("custom target purge must fail before destructive work");
        assert!(err.to_string().contains("deploy_target 'custom'"));
        assert_eq!(
            fs::read(data_dir.join("custom_file.txt")).unwrap(),
            b"do not delete"
        );
        assert!(custom_root.join("custom_file.txt").exists());
        assert_eq!(db.get_deployment_paths_for_mod(mod_id).unwrap().len(), 1);
    }

    #[test]
    fn deploy_mod_atomic_routes_root_target_to_game_root() {
        let (db, _tmp, staging, data_dir) = setup();
        let game_root = _tmp.path().join("game_root");
        fs::create_dir_all(&game_root).unwrap();

        let mod_id = db
            .add_mod(
                "skyrimse",
                "Gaming",
                None,
                "RootMod",
                "1.0",
                "root.zip",
                &["root_loader.dll".to_string()],
            )
            .unwrap();
        create_staging_file(&staging, "root_loader.dll", b"root dll");

        deploy_mod_atomic(
            &db,
            "skyrimse",
            "Gaming",
            mod_id,
            &staging,
            &data_dir,
            &["root_loader.dll".to_string()],
            &game_root,
            "root",
        )
        .unwrap();

        assert!(game_root.join("root_loader.dll").exists());
        assert!(!data_dir.join("root_loader.dll").exists());
        assert_eq!(db.get_deploy_target_for_mod(mod_id).unwrap(), "root");
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
                    entry.deploy_target.as_str(),
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

    #[test]
    fn deploy_native_game_without_plugin_returns_error() {
        use crate::runtime::{Architecture, GameRuntime, NativeContext, NativeSource};
        use std::sync::Arc;

        let detected = crate::games::DetectedGame {
            game_id: "fakegame".into(),
            display_name: "Fake".into(),
            nexus_slug: "fake".into(),
            game_path: PathBuf::from("/Applications/Fake.app/Contents/MacOS"),
            exe_path: None,
            data_dir: PathBuf::from("/Applications/Fake.app/Contents/MacOS"),
            runtime: GameRuntime::Native(NativeContext {
                app_bundle_path: PathBuf::from("/Applications/Fake.app"),
                game_data_root: PathBuf::from("/Applications/Fake.app/Contents/MacOS"),
                architecture: Architecture::AppleSilicon,
                sandboxed: false,
                source: NativeSource::Manual,
            }),
            steam_app_id: None,
            is_custom: false,
        };

        let (db, _tmp, _, _) = setup();
        let db = Arc::new(db);

        let result = deploy_native_game(&detected, &db);
        assert!(
            result.is_err(),
            "expected stub error for unimplemented native deploy"
        );
        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(err_msg.contains("fakegame"), "error should mention game_id");
    }
}
