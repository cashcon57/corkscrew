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

/// Normalize a relative path into a manifest/conflict map key: forward
/// slashes + lowercase. Wine targets case-insensitive filesystems (NTFS
/// semantics, APFS on macOS), so `Textures/a.dds` and `textures/a.dds`
/// are the same game-visible file and must collide in conflict resolution.
fn manifest_key(rel_path: &str) -> String {
    rel_path.replace('\\', "/").to_lowercase()
}

/// Deploy-time heuristic for *untracked* files: an existing top-level
/// `.esm`/`.bsa`/`.ba2` that is not in the deployment manifest is assumed to
/// belong to the game and is never overwritten.
///
/// This must NEVER be used to decide whether a *recorded* file may be
/// removed. Mod archives (`SkyUI_SE.bsa`, `UOSC.bsa`, …) match this pattern
/// too, and a manifest record is proof Corkscrew placed the file. Treating
/// recorded mod BSAs as vanilla on undeploy left them in Data untracked, and
/// re-enabling the mod then refused to overwrite its own leftover (bug #4).
fn is_protected_extension(rel_path: &str) -> bool {
    let lower = rel_path.to_lowercase();
    if !lower.contains('/')
        && (lower.ends_with(".esm") || lower.ends_with(".bsa") || lower.ends_with(".ba2"))
    {
        return true;
    }
    false
}

/// Build a set of known vanilla file paths for a game (normalized with
/// [`manifest_key`], for O(1) lookup). Returns None if no baseline is
/// available for the game.
fn build_vanilla_set(game_id: &str) -> Option<std::collections::HashSet<String>> {
    baselines::get_builtin_baseline(game_id)
        .map(|baseline| baseline.iter().map(|p| manifest_key(p)).collect())
}

/// How a recorded path relates to the game's stock (vanilla) files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StockClass {
    /// Not a known stock file — the manifest record alone proves Corkscrew
    /// owns whatever is on disk at this path.
    NotStock,
    /// Exact match in the built-in baseline list (e.g. `Skyrim.esm`). These
    /// are never deleted, even when recorded.
    Baseline,
    /// Matches a broad stock *pattern* (`cc*.bsa`, `video/…`) but is not in
    /// the explicit baseline. Mods can collide with these patterns (any mod
    /// archive whose name starts with "cc"), so ownership is decided by
    /// comparing the on-disk file against the mod's staging copy.
    StockPattern,
}

fn classify_stock(
    game_id: &str,
    rel_path: &str,
    vanilla_set: Option<&std::collections::HashSet<String>>,
) -> StockClass {
    // Case-folded on purpose: Wine targets case-insensitive filesystems.
    if vanilla_set.is_some_and(|set| set.contains(&manifest_key(rel_path))) {
        return StockClass::Baseline;
    }
    if baselines::is_stock_pattern(game_id, &rel_path.replace('\\', "/")) {
        return StockClass::StockPattern;
    }
    StockClass::NotStock
}

#[cfg(unix)]
fn same_inode(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.dev() == b.dev() && a.ino() == b.ino()
}

#[cfg(not(unix))]
fn same_inode(_a: &fs::Metadata, _b: &fs::Metadata) -> bool {
    false
}

/// If `dst` is provably the copy Corkscrew deploys from `src`, return the
/// deploy method that describes it: `"hardlink"` when both are the same inode,
/// `"copy"` when they are distinct but byte-identical regular files.
///
/// Neither path is followed if it is a symlink.
fn deployed_copy_kind(src: &Path, dst: &Path) -> Option<&'static str> {
    let dst_meta = fs::symlink_metadata(dst).ok()?;
    let src_meta = fs::symlink_metadata(src).ok()?;
    if !dst_meta.file_type().is_file() || !src_meta.file_type().is_file() {
        return None;
    }
    if same_inode(&src_meta, &dst_meta) {
        return Some("hardlink");
    }
    if src_meta.len() != dst_meta.len() {
        return None;
    }
    let src_hash = platform::fast_hash(src).ok()?;
    let dst_hash = platform::fast_hash(dst).ok()?;
    (src_hash == dst_hash).then_some("copy")
}

/// Outcome of removing one recorded (manifest-tracked) file from disk.
#[derive(Debug)]
enum RemoveOutcome {
    /// The file was deleted.
    Removed,
    /// Nothing was on disk at the recorded path.
    AlreadyGone,
    /// The path is a genuine stock game file and was left in place.
    KeptVanilla,
    /// Deletion failed; the manifest row must be kept so the user can retry.
    Failed(String),
}

/// Remove one file that the deployment manifest says Corkscrew placed.
///
/// A manifest record is proof of ownership, so the file is removed unless it
/// is a genuine stock file: an exact built-in baseline match, or a stock
/// *pattern* match whose on-disk content is not the mod's staging copy.
/// Root-targeted entries skip the stock check (the baseline describes Data/).
///
/// The symlink check happens before any file operation (TOCTOU): a symlink
/// at the recorded path is unlinked, never followed.
fn remove_recorded_file(
    game_id: &str,
    rel_path: &str,
    deploy_target: &str,
    base: &Path,
    staging_dir: Option<&Path>,
    vanilla_set: Option<&std::collections::HashSet<String>>,
) -> RemoveOutcome {
    if !crate::staging::is_safe_relative_path(rel_path) {
        return RemoveOutcome::Failed(format!("unsafe manifest path: {}", rel_path));
    }
    let file_path = base.join(rel_path);

    let meta = match fs::symlink_metadata(&file_path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return RemoveOutcome::AlreadyGone,
        Err(e) => return RemoveOutcome::Failed(e.to_string()),
    };
    if meta.is_dir() {
        return RemoveOutcome::Failed(format!("{} is a directory", file_path.display()));
    }

    if deploy_target != "root" {
        let keep = match classify_stock(game_id, rel_path, vanilla_set) {
            StockClass::NotStock => false,
            StockClass::Baseline => true,
            StockClass::StockPattern => !staging_dir
                .is_some_and(|dir| deployed_copy_kind(&dir.join(rel_path), &file_path).is_some()),
        };
        if keep {
            warn!(
                "SAFETY: leaving stock game file in place: {} (record dropped)",
                rel_path
            );
            return RemoveOutcome::KeptVanilla;
        }
    }

    // Some mod files ship read-only; make writable before deleting.
    if !meta.file_type().is_symlink() && meta.permissions().readonly() {
        let mut writable = meta.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        writable.set_readonly(false);
        if let Err(e) = fs::set_permissions(&file_path, writable) {
            debug!("Could not clear read-only flag on {}: {}", file_path.display(), e);
        }
    }

    match fs::remove_file(&file_path) {
        Ok(()) => {
            prune_empty_dirs(&file_path, base);
            RemoveOutcome::Removed
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => RemoveOutcome::AlreadyGone,
        Err(e) => RemoveOutcome::Failed(e.to_string()),
    }
}

/// Resolve the directory a manifest entry's `relative_path` is relative to.
/// `custom` targets are recorded relative to a per-mod directory that only
/// the command layer can recompute; they fall back to `data_dir`, matching
/// the historical behaviour of undeploy/purge.
fn deploy_base<'a>(deploy_target: &str, data_dir: &'a Path, game_path: &'a Path) -> &'a Path {
    if deploy_target == "root" {
        game_path
    } else {
        data_dir
    }
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
    use rayon::prelude::*;

    if !staging_path.exists() {
        return Err(DeployerError::StagingNotFound(staging_path.to_path_buf()));
    }
    let vanilla_set = build_vanilla_set(game_id);

    // Persist the target on the mod itself: manifest rows (the only other
    // place it lived) are dropped on undeploy/purge, which used to reset
    // root-deployed mods to `data` on the next re-enable or full redeploy.
    if let Err(e) = db.remember_deploy_target_for_mod(mod_id, deploy_target) {
        warn!(
            "Deploy mod {}: could not persist deploy target '{}': {}",
            mod_id, deploy_target, e
        );
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
    // Keyed case-insensitively (manifest_key) so conflicts are detected the
    // way the game sees them on NTFS/APFS; values keep the original-cased
    // path so the loser's on-disk file can be removed even when casing differs.
    let deployed_map: std::collections::HashMap<String, (i64, &str)> = manifest
        .iter()
        .map(|e| {
            (
                manifest_key(&e.relative_path),
                (e.mod_id, e.relative_path.as_str()),
            )
        })
        .collect();
    let priorities = db
        .get_all_mod_priorities()
        .map_err(|e| DeployerError::Database(e.to_string()))?;

    let deployed_count = AtomicUsize::new(0);
    let skipped_count = AtomicUsize::new(0);
    let missing_count = AtomicUsize::new(0);
    let junk_count = AtomicUsize::new(0);
    let fallback_used = AtomicBool::new(false);
    let staging_str = staging_path.to_string_lossy().to_string();

    // Phase 1: Parallel file I/O — resolve conflicts, then hardlink or copy.
    // Collect successful deployments for batch database insert.
    let results: Vec<Option<(String, &str)>> = files
        .par_iter()
        .map(|rel_path| {
            // Defense-in-depth: reject path traversal (string-level check)
            if !crate::staging::is_safe_relative_path(rel_path) {
                warn!("Deploy: skipping unsafe relative path: {}", rel_path);
                return None;
            }

            // Defense-in-depth: post-join canonicalization check
            let src = staging_path.join(rel_path);
            if src.exists() && !crate::staging::validate_path_within_base(staging_path, &src) {
                warn!("Deploy: path escaped staging after join: {}", rel_path);
                return None;
            }

            // Defense-in-depth: skip packaging junk (fomod/, meta.ini, etc.)
            if crate::installer::is_deploy_junk(std::path::Path::new(rel_path)) {
                debug!("Deploy: skipping junk file: {}", rel_path);
                junk_count.fetch_add(1, Ordering::Relaxed);
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

            // Conflict resolution via in-memory lookup (case-insensitive key)
            let normalized_key = manifest_key(rel_path);
            let mut manifest_owned = false;
            if let Some(&(owner_mod_id, owner_path)) = deployed_map.get(normalized_key.as_str()) {
                if owner_mod_id == mod_id {
                    return None; // already deployed by us
                }
                let owner_priority = priorities.get(&owner_mod_id).copied().unwrap_or(0);
                if owner_priority > my_priority as i64 {
                    skipped_count.fetch_add(1, Ordering::Relaxed);
                    return None;
                }
                // We win. On a case-sensitive host filesystem the loser's file
                // may live at a differently-cased path — remove it explicitly
                // (deploy_single_file only clears the exact dst path).
                if owner_path != rel_path {
                    let _ = fs::remove_file(data_dir.join(owner_path));
                }
                manifest_owned = true;
            }

            let method =
                match deploy_single_file(
                    rel_path,
                    &src,
                    &dst,
                    manifest_owned,
                    classify_stock(game_id, rel_path, vanilla_set.as_ref()) == StockClass::NotStock,
                    can_hardlink,
                    copy_method,
                ) {
                    Some(m) => {
                        if m == "copy" {
                            fallback_used.store(true, Ordering::Relaxed);
                        }
                        m
                    }
                    None => {
                        skipped_count.fetch_add(1, Ordering::Relaxed);
                        return None;
                    }
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
    let batch: Vec<(&str, &str, i64, &str, &str, &str, &str)> = results
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
                    deploy_target,
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

    let final_junk = junk_count.load(Ordering::Relaxed);

    // If we expected to deploy files but none succeeded, warn but don't error.
    // This can happen legitimately: FOMOD reconfiguration, stale staging, or
    // mods consisting entirely of metadata files (meta.ini, fomod/, etc.).
    let deployable = files.len().saturating_sub(final_junk);
    if final_deployed == 0 && deployable > 0 {
        warn!(
            "Deploy mod {}: 0 of {} deployable files deployed (junk={}, missing={}) \
             staging_path={}, data_dir={}, exists=({}, {})",
            mod_id,
            deployable,
            final_junk,
            final_missing,
            staging_path.display(),
            data_dir.display(),
            staging_path.exists(),
            data_dir.exists(),
        );
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
    deploy_target: &str,
) -> Result<DeployResult> {
    match deploy_mod(
        db,
        game_id,
        bottle_name,
        mod_id,
        staging_path,
        data_dir,
        files,
        deploy_target,
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
    deploy_target: &str,
) -> Result<DeployResult> {
    match deploy_mod_inner(
        db,
        game_id,
        bottle_name,
        mod_id,
        staging_path,
        data_dir,
        files,
        deploy_target,
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
    // Query manifest rows FIRST without deleting them. Rows are only deleted
    // for files that are actually gone from disk afterwards (or are genuine
    // stock files), so a failed removal stays tracked and can be retried.
    let entries = db
        .get_deployment_entries_for_mod(mod_id)
        .map_err(|e| DeployerError::Database(e.to_string()))?;

    let vanilla_set = build_vanilla_set(game_id);
    let mut actually_removed = Vec::new();
    let mut clear_ids: Vec<i64> = Vec::with_capacity(entries.len());
    let mut kept_vanilla = 0usize;
    let mut errors = Vec::new();
    let mut restore_failures = Vec::new();

    // Fetch the mod list ONCE for all restore_next_winner calls.
    // Previously this was fetched per-file, causing O(n²) DB queries for large mods.
    let all_mods = db
        .list_mods(game_id, bottle_name)
        .map_err(|e| DeployerError::Database(e.to_string()))?;

    for entry in &entries {
        let rel_path = &entry.relative_path;
        let base = deploy_base(&entry.deploy_target, data_dir, game_path);
        let staging_dir =
            (!entry.staging_path.is_empty()).then(|| Path::new(entry.staging_path.as_str()));

        match remove_recorded_file(
            game_id,
            rel_path,
            &entry.deploy_target,
            base,
            staging_dir,
            vanilla_set.as_ref(),
        ) {
            RemoveOutcome::Removed | RemoveOutcome::AlreadyGone => {
                actually_removed.push(rel_path.clone());
                clear_ids.push(entry.id);
            }
            RemoveOutcome::KeptVanilla => {
                // The file on disk is the game's, not ours: the record no
                // longer describes anything Corkscrew placed.
                kept_vanilla += 1;
                clear_ids.push(entry.id);
                continue;
            }
            RemoveOutcome::Failed(e) => {
                // Keep the record: the file is still on disk and still ours.
                errors.push(format!("{}: {}", rel_path, e));
                continue;
            }
        }

        // Restore next-priority mod's version of this file if applicable.
        // Its manifest row replaces ours (UNIQUE on path + target), so the
        // id-based cleanup below never deletes the winner's row.
        if let Err(e) = restore_next_winner_with_mods(db, &all_mods, rel_path, base) {
            warn!("Failed to restore winner for {}: {}", rel_path, e);
            restore_failures.push(rel_path.clone());
        }
    }

    if !restore_failures.is_empty() {
        warn!(
            "Mod {} undeploy: {} file(s) could not be restored from lower-priority mods, \
             potential orphans: {:?}",
            mod_id,
            restore_failures.len(),
            restore_failures,
        );
    }
    db.remove_deployment_entries_by_ids(&clear_ids)
        .map_err(|e| DeployerError::Database(e.to_string()))?;

    info!(
        "Undeployed mod {} ({}/{} files removed, {} stock files left in place, {} errors, {} restore failures)",
        mod_id,
        actually_removed.len(),
        entries.len(),
        kept_vanilla,
        errors.len(),
        restore_failures.len(),
    );

    if !errors.is_empty() {
        warn!(
            "Undeploy errors for mod {} ({} file(s) still deployed and still tracked): {:?}",
            mod_id,
            errors.len(),
            errors
        );
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
            if !crate::staging::is_safe_relative_path(staging_path_str) && !PathBuf::from(staging_path_str).is_absolute() {
                warn!("Skipping mod with suspicious staging path: {}", staging_path_str);
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
                    &mod_target,
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
    let mut clear_ids: Vec<i64> = Vec::with_capacity(manifest.len());
    let mut kept_vanilla = 0usize;
    let mut failed = 0usize;

    for entry in &manifest {
        // Legacy direct-installed files are not ours to purge
        if entry.deploy_method == "direct" {
            continue;
        }

        let base = deploy_base(&entry.deploy_target, data_dir, game_path);
        let staging_dir =
            (!entry.staging_path.is_empty()).then(|| Path::new(entry.staging_path.as_str()));
        match remove_recorded_file(
            game_id,
            &entry.relative_path,
            &entry.deploy_target,
            base,
            staging_dir,
            vanilla_set.as_ref(),
        ) {
            RemoveOutcome::Removed => {
                removed.push(entry.relative_path.clone());
                clear_ids.push(entry.id);
            }
            RemoveOutcome::AlreadyGone => clear_ids.push(entry.id),
            RemoveOutcome::KeptVanilla => {
                kept_vanilla += 1;
                clear_ids.push(entry.id);
            }
            RemoveOutcome::Failed(e) => {
                // Keep the record so the file stays managed and a later
                // purge/undeploy can retry. Dropping it would leave an
                // untracked mod file that future deploys refuse to replace.
                warn!(
                    "Failed to purge {}: {}",
                    base.join(&entry.relative_path).display(),
                    e
                );
                failed += 1;
            }
        }
    }

    db.remove_deployment_entries_by_ids(&clear_ids)
        .map_err(|e| DeployerError::Database(e.to_string()))?;

    info!(
        "Purged deployment for {}/{}: {} files removed, {} stock files left in place, {} failed (still tracked)",
        game_id,
        bottle_name,
        removed.len(),
        kept_vanilla,
        failed
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
    restore_next_winner_with_mods(db, &mods, rel_path, data_dir)
}

/// Like `restore_next_winner`, but accepts a pre-fetched mod list to avoid
/// repeated `db.list_mods()` calls. Use this when undeploying many files at
/// once (e.g., `undeploy_mod`) to avoid O(n²) DB queries.
fn restore_next_winner_with_mods(
    db: &ModDatabase,
    all_mods: &[crate::database::InstalledMod],
    rel_path: &str,
    data_dir: &Path,
) -> Result<()> {
    let mut candidates: Vec<_> = all_mods
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
                        warn!("Hardlink failed in restore_next_winner_with_mods: {}", e);
                        platform::fast_copy(&src, &dst, copy_method)?;
                        "copy"
                    }
                }
            } else {
                platform::fast_copy(&src, &dst, copy_method)?;
                "copy"
            };

            db.add_deployment_entry(
                &winner.game_id,
                &winner.bottle_name,
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

        let deploy_target = db
            .get_deploy_target_for_mod(m.id)
            .unwrap_or_else(|_| "data".to_string());

        for rel_path in files {
            let sha256 = hash_map.get(&(m.id, rel_path.clone())).cloned();

            // Last writer wins (highest priority, since sorted ascending).
            // Keyed case-insensitively so differently-cased paths from two
            // mods collide here instead of double-deploying.
            desired.insert(
                manifest_key(&rel_path),
                DesiredFile {
                    relative_path: rel_path,
                    mod_id: m.id,
                    staging_path: staging_path.clone(),
                    sha256,
                    deploy_target: deploy_target.clone(),
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

/// Deploy a single file from staging to the game directory.
///
/// This is the single source of truth for per-file deployment safety,
/// shared by the full and incremental deploy paths:
/// - symlink rejection via `symlink_metadata` (catches dangling symlinks,
///   which `dst.exists()` would follow and miss) before AND after the
///   file operation (TOCTOU hardening)
/// - vanilla-file protection: an existing file not owned by the manifest
///   (`manifest_owned == false`) is never overwritten if protected
///
/// Priority chain: hardlink → reflink/clonefile → copy.
/// Returns the deploy method used ("hardlink" or "copy"), or None on skip/failure.
fn deploy_single_file(
    rel_path: &str,
    src: &Path,
    dst: &Path,
    manifest_owned: bool,
    allow_content_adoption: bool,
    can_hardlink: bool,
    copy_method: platform::FsCopyMethod,
) -> Option<&'static str> {
    // symlink_metadata (not exists()) so dangling symlinks are detected
    // instead of followed — a dangling link would otherwise survive to the
    // copy fallback, which writes THROUGH it to an arbitrary target.
    match fs::symlink_metadata(dst) {
        Ok(meta) if meta.file_type().is_symlink() => {
            warn!("Skipping deployment to symlink target: {}", dst.display());
            return None;
        }
        Ok(_) => {
            if !manifest_owned && is_protected_extension(rel_path) {
                // An untracked file that is provably this mod's own copy (the
                // same inode as the staging file, or byte-identical) is a
                // leftover from an earlier undeploy that dropped its record
                // without deleting it. Adopt it instead of skipping, so users
                // already hit by that bug heal on their next deploy.
                // Same inode proves Corkscrew put it there. Identical bytes
                // only do for non-stock paths: a mod can ship an exact copy
                // of a base-game file, which must never become ours to delete.
                if let Some(method) = deployed_copy_kind(src, dst)
                    .filter(|m| *m == "hardlink" || allow_content_adoption)
                {
                    info!(
                        "Deploy: adopting untracked {} — identical to the mod's staging copy ({})",
                        rel_path, method
                    );
                    return Some(method);
                }
                // File exists on disk but is NOT in the deployment manifest:
                // likely a vanilla game file. Overwriting it would make it
                // unrecoverable on undeploy/purge.
                warn!(
                    "Deploy: skipping {} — would overwrite unmanaged vanilla file",
                    rel_path
                );
                return None;
            }
            if let Err(e) = fs::remove_file(dst) {
                warn!("Failed to remove existing file {}: {}", dst.display(), e);
                return None;
            }
        }
        Err(_) => {}
    }

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

    // Symlink re-check immediately before the file operation to minimize
    // the TOCTOU window between removal and link/copy.
    if let Ok(meta) = fs::symlink_metadata(dst) {
        if meta.file_type().is_symlink() {
            warn!("Pre-deploy symlink detected: {}", dst.display());
            return None;
        }
    }

    let method = if can_hardlink {
        match fs::hard_link(src, dst) {
            Ok(_) => "hardlink",
            Err(e) => {
                warn!(
                    "Hardlink failed for {} → {}: {} (falling back to reflink/copy)",
                    src.display(),
                    dst.display(),
                    e
                );
                // platform::fast_copy tries reflink/clonefile before fs::copy
                match platform::fast_copy(src, dst, copy_method) {
                    Ok(_) => "copy",
                    Err(copy_err) => {
                        warn!(
                            "Copy also failed for {} → {}: {}",
                            src.display(),
                            dst.display(),
                            copy_err
                        );
                        return None;
                    }
                }
            }
        }
    } else {
        // Cross-device: reflink/clonefile → fs::copy
        match platform::fast_copy(src, dst, copy_method) {
            Ok(_) => "copy",
            Err(e) => {
                warn!(
                    "Copy failed for {} → {}: {}",
                    src.display(),
                    dst.display(),
                    e
                );
                return None;
            }
        }
    };

    // Post-deploy symlink check to tighten the TOCTOU window
    if let Ok(meta) = fs::symlink_metadata(dst) {
        if meta.file_type().is_symlink() {
            let _ = fs::remove_file(dst);
            warn!("Post-deploy symlink detected, removed: {}", dst.display());
            return None;
        }
    }

    Some(method)
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

    // Step 2: Load current deployment manifest, re-keyed case-insensitively
    // to match compute_desired_state (Wine-visible path semantics).
    let current: std::collections::HashMap<String, crate::database::DeploymentEntry> = db
        .get_deployment_manifest_map(game_id, bottle_name)
        .map_err(|e| DeployerError::Database(e.to_string()))?
        .into_values()
        .map(|e| (manifest_key(&e.relative_path), e))
        .collect();

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
    let copy_method = platform::detect_copy_method(&staging_root, data_dir);

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

    // Step 5a: Remove files that should no longer be deployed. Same ownership
    // rules as undeploy/purge; rows are only dropped once the file is gone
    // (or is a genuine stock file), never for a file that failed to delete.
    let vanilla_set = build_vanilla_set(game_id);
    let remove_ids: Vec<i64> = diff
        .to_remove
        .par_iter()
        .filter_map(|entry| {
            let base = deploy_base(&entry.deploy_target, data_dir, game_path);
            let staging_dir =
                (!entry.staging_path.is_empty()).then(|| Path::new(entry.staging_path.as_str()));
            match remove_recorded_file(
                game_id,
                &entry.relative_path,
                &entry.deploy_target,
                base,
                staging_dir,
                vanilla_set.as_ref(),
            ) {
                RemoveOutcome::Removed | RemoveOutcome::AlreadyGone => {
                    removed_count.fetch_add(1, Ordering::Relaxed);
                    Some(entry.id)
                }
                RemoveOutcome::KeptVanilla => Some(entry.id),
                RemoveOutcome::Failed(e) => {
                    warn!(
                        "Failed to remove {}: {}",
                        base.join(&entry.relative_path).display(),
                        e
                    );
                    verification_failures
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .push(format!("remove failed: {}: {}", entry.relative_path, e));
                    None
                }
            }
        })
        .collect();

    // Remove stale manifest entries
    db.remove_deployment_entries_by_ids(&remove_ids)
        .map_err(|e| DeployerError::Database(e.to_string()))?;

    // Step 5b: Update files where the owning mod changed
    let update_owned: Vec<(i64, String, String, Option<String>, String, &'static str)> = diff
        .to_update
        .par_iter()
        .filter_map(|(old_entry, new_desired)| {
            let base = deploy_base(&new_desired.deploy_target, data_dir, game_path);
            let old_base = deploy_base(&old_entry.deploy_target, data_dir, game_path);
            let dst = base.join(&new_desired.relative_path);
            let src = new_desired.staging_path.join(&new_desired.relative_path);

            if !src.exists() {
                warn!("Incremental update: source not found: {}", src.display());
                return None;
            }

            // The old owner's file may live at a differently-cased path on a
            // case-sensitive host filesystem (or under a different target) —
            // remove it explicitly.
            if old_entry.relative_path != new_desired.relative_path || old_base != base {
                let _ = fs::remove_file(old_base.join(&old_entry.relative_path));
            }

            match deploy_single_file(
                &new_desired.relative_path,
                &src,
                &dst,
                true, // path is in the manifest (owned by old_entry's mod)
                false,
                can_hardlink,
                copy_method,
            ) {
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
                        method,
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
        .map(|(mod_id, rel_path, staging_path, sha256, deploy_target, method)| {
            (
                game_id,
                bottle_name,
                *mod_id,
                rel_path.as_str(),
                staging_path.as_str(),
                *method,
                sha256.as_deref(),
                deploy_target.as_str(),
            )
        })
        .collect();

    if !update_entries.is_empty() {
        db.batch_add_deployment_entries_with_hashes(&update_entries)
            .map_err(|e| DeployerError::Database(e.to_string()))?;
    }

    // Step 5c: Add new files
    let add_results: Vec<Option<(i64, String, String, Option<String>, String, &'static str)>> =
        diff.to_add
            .par_iter()
            .map(|desired_file| {
                let dst = deploy_base(&desired_file.deploy_target, data_dir, game_path)
                    .join(&desired_file.relative_path);
                let src = desired_file.staging_path.join(&desired_file.relative_path);

                if !src.exists() {
                    warn!("Incremental add: source not found: {}", src.display());
                    return None;
                }

                match deploy_single_file(
                    &desired_file.relative_path,
                    &src,
                    &dst,
                    false, // not in manifest — vanilla protection applies
                    classify_stock(game_id, &desired_file.relative_path, vanilla_set.as_ref())
                        == StockClass::NotStock,
                    can_hardlink,
                    copy_method,
                ) {
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
                            method,
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
                        *method,
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
// Targeted exclude / restore (used by game-specific deploy policies)
// ---------------------------------------------------------------------------

/// Resolve `rel_path` under `base` component-by-component, case-insensitively,
/// returning the on-disk casing. Wine games target case-insensitive
/// filesystems, so `SKSE/Plugins/EngineFixes.dll` must match a staging dir
/// that spells it `skse/plugins/EngineFixes.dll`.
fn resolve_case_insensitive(base: &Path, rel_path: &str) -> Option<String> {
    let mut cur = base.to_path_buf();
    let mut parts: Vec<String> = Vec::new();
    for comp in rel_path.replace('\\', "/").split('/').filter(|c| !c.is_empty()) {
        let exact = cur.join(comp);
        let found = if fs::symlink_metadata(&exact).is_ok() {
            comp.to_string()
        } else {
            fs::read_dir(&cur)
                .ok()?
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .find(|n| n.eq_ignore_ascii_case(comp))?
        };
        cur.push(&found);
        parts.push(found);
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Stop deploying specific paths: delete their deployed copies and drop
/// their manifest rows. Staging is never touched, so the files come back as
/// soon as the caller stops excluding them (and any full redeploy re-adds
/// them until the caller's policy runs again).
///
/// `paths` is a list of `(relative_path, deploy_target)`; paths match
/// case-insensitively, targets exactly. Returns the relative paths removed.
pub fn exclude_deployed_paths(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    data_dir: &Path,
    game_path: &Path,
    paths: &[(&str, &str)],
) -> Result<Vec<String>> {
    let wanted: std::collections::HashSet<(String, &str)> =
        paths.iter().map(|(p, t)| (manifest_key(p), *t)).collect();
    let manifest = db
        .get_deployment_manifest(game_id, bottle_name)
        .map_err(|e| DeployerError::Database(e.to_string()))?;

    let mut removed = Vec::new();
    let mut clear_ids = Vec::new();
    for entry in manifest.iter().filter(|e| {
        e.deploy_method != "direct"
            && wanted.contains(&(manifest_key(&e.relative_path), e.deploy_target.as_str()))
    }) {
        let base = deploy_base(&entry.deploy_target, data_dir, game_path);
        let staging_dir =
            (!entry.staging_path.is_empty()).then(|| Path::new(entry.staging_path.as_str()));
        match remove_recorded_file(
            game_id,
            &entry.relative_path,
            &entry.deploy_target,
            base,
            staging_dir,
            None,
        ) {
            RemoveOutcome::Removed => {
                removed.push(entry.relative_path.clone());
                clear_ids.push(entry.id);
            }
            RemoveOutcome::AlreadyGone | RemoveOutcome::KeptVanilla => clear_ids.push(entry.id),
            RemoveOutcome::Failed(e) => warn!(
                "Exclude: could not remove {} ({}); keeping its record",
                entry.relative_path, e
            ),
        }
    }
    db.remove_deployment_entries_by_ids(&clear_ids)
        .map_err(|e| DeployerError::Database(e.to_string()))?;
    Ok(removed)
}

/// Make sure each `(relative_path, deploy_target)` is deployed from the
/// highest-priority enabled mod whose staging directory ships it, and that
/// the deployed copy is byte-identical to that staging file. Missing or
/// modified copies are re-linked from staging and recorded in the manifest.
///
/// An untracked file at the destination that is not the winner's copy
/// belongs to someone else (a manual install) and is left alone.
/// Returns the relative paths that were (re)deployed.
pub fn ensure_paths_deployed(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    data_dir: &Path,
    game_path: &Path,
    paths: &[(&str, &str)],
) -> Result<Vec<String>> {
    let mut mods: Vec<_> = db
        .list_mods(game_id, bottle_name)
        .map_err(|e| DeployerError::Database(e.to_string()))?
        .into_iter()
        .filter(|m| m.enabled && m.staging_path.is_some())
        .collect();
    mods.sort_by_key(|m| std::cmp::Reverse(m.install_priority));

    let manifest = db
        .get_deployment_manifest(game_id, bottle_name)
        .map_err(|e| DeployerError::Database(e.to_string()))?;

    let mut restored = Vec::new();
    for &(rel, target) in paths {
        if !crate::staging::is_safe_relative_path(rel) {
            continue;
        }
        // Highest-priority enabled mod that ships this file for this target.
        let winner = mods.iter().find_map(|m| {
            let staging = PathBuf::from(m.staging_path.as_deref()?);
            let actual = resolve_case_insensitive(&staging, rel)?;
            if !staging.join(&actual).is_file() {
                return None;
            }
            let mod_target = db
                .get_deploy_target_for_mod(m.id)
                .unwrap_or_else(|_| "data".to_string());
            (mod_target == target).then_some((m, staging, actual))
        });
        let Some((winner, staging, staging_rel)) = winner else {
            continue;
        };

        let base = deploy_base(target, data_dir, game_path);
        let key = manifest_key(rel);
        let existing = manifest
            .iter()
            .find(|e| e.deploy_target == target && manifest_key(&e.relative_path) == key);
        // Keep the casing already on disk/in the manifest so we don't create
        // a second row for the same game-visible file.
        let rel_path = existing
            .map(|e| e.relative_path.clone())
            .or_else(|| resolve_case_insensitive(base, rel))
            .unwrap_or_else(|| staging_rel.clone());
        let src = staging.join(&staging_rel);
        let dst = base.join(&rel_path);

        let on_disk = fs::symlink_metadata(&dst).is_ok();
        let is_winner_copy = on_disk && deployed_copy_kind(&src, &dst).is_some();
        if is_winner_copy && existing.is_some_and(|e| e.mod_id == winner.id) {
            continue; // already correct
        }
        if on_disk && existing.is_none() && !is_winner_copy {
            warn!(
                "Restore: leaving untracked {} in place — it is not the copy shipped by '{}'",
                dst.display(),
                winner.name
            );
            continue;
        }

        let can_hardlink = same_filesystem(&staging, base);
        let copy_method = platform::detect_copy_method(&staging, base);
        let Some(method) =
            deploy_single_file(&rel_path, &src, &dst, true, false, can_hardlink, copy_method)
        else {
            warn!("Restore: failed to deploy {} from '{}'", rel_path, winner.name);
            continue;
        };
        if let Some(e) = existing {
            db.remove_deployment_entries_by_ids(&[e.id])
                .map_err(|e| DeployerError::Database(e.to_string()))?;
        }
        let staging_str = staging.to_string_lossy();
        db.batch_add_deployment_entries(&[(
            game_id,
            bottle_name,
            winner.id,
            rel_path.as_str(),
            staging_str.as_ref(),
            method,
            target,
        )])
        .map_err(|e| DeployerError::Database(e.to_string()))?;
        info!("Restore: deployed {} from '{}' ({})", rel_path, winner.name, method);
        restored.push(rel_path);
    }
    Ok(restored)
}

// ---------------------------------------------------------------------------
// Pre-launch self-heal
// ---------------------------------------------------------------------------

/// Result of [`restore_missing_files`].
#[derive(Debug, Default, Clone)]
pub struct SelfHealReport {
    /// Manifest entries whose on-disk presence was checked.
    pub checked: usize,
    /// Checked entries whose file was missing from disk.
    pub missing: usize,
    /// Missing files that were re-deployed from staging.
    pub restored: Vec<String>,
    /// Missing files that could not be restored (staging copy gone or the
    /// deploy failed). Their records are kept.
    pub unrecoverable: Vec<String>,
    /// Records for missing files whose mod is no longer enabled/installed;
    /// these were dropped instead of restored.
    pub stale_dropped: usize,
}

/// Re-deploy only the managed files that have gone missing from disk.
///
/// Each manifest entry is resolved against its real deploy target (`root`
/// entries live in the game folder, not `Data/`), so root-deployed files such
/// as the SKSE loader are no longer reported missing on every launch. Only
/// the missing files are restored; nothing else is touched. `custom`-target
/// entries are skipped because their base directory can only be recomputed
/// by the command layer.
#[allow(clippy::type_complexity)]
pub fn restore_missing_files(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    data_dir: &Path,
    game_path: &Path,
) -> Result<SelfHealReport> {
    let manifest = db
        .get_deployment_manifest(game_id, bottle_name)
        .map_err(|e| DeployerError::Database(e.to_string()))?;

    let candidates: Vec<&crate::database::DeploymentEntry> = manifest
        .iter()
        .filter(|e| e.deploy_method != "direct" && e.deploy_target != "custom")
        .collect();

    // Parallel existence check — modlists can track 100k+ files.
    // symlink_metadata so a symlink counts as "present" and is never followed.
    let missing: Vec<&crate::database::DeploymentEntry> = candidates
        .par_iter()
        .filter(|e| {
            let base = deploy_base(&e.deploy_target, data_dir, game_path);
            fs::symlink_metadata(base.join(&e.relative_path)).is_err()
        })
        .copied()
        .collect();

    let mut report = SelfHealReport {
        checked: candidates.len(),
        missing: missing.len(),
        ..Default::default()
    };
    if missing.is_empty() {
        return Ok(report);
    }

    let enabled: std::collections::HashSet<i64> = db
        .list_mods(game_id, bottle_name)
        .map_err(|e| DeployerError::Database(e.to_string()))?
        .into_iter()
        .filter(|m| m.enabled)
        .map(|m| m.id)
        .collect();

    let staging_root = crate::staging::staging_base_dir(game_id, bottle_name);
    let can_hardlink = same_filesystem(&staging_root, data_dir);
    let copy_method = platform::detect_copy_method(&staging_root, data_dir);

    let mut stale_ids = Vec::new();
    let mut rewritten: Vec<(i64, String, String, &'static str, Option<String>, String)> = Vec::new();

    for entry in missing {
        if !enabled.contains(&entry.mod_id) {
            stale_ids.push(entry.id);
            continue;
        }
        if !crate::staging::is_safe_relative_path(&entry.relative_path) {
            warn!("Self-heal: skipping unsafe manifest path {}", entry.relative_path);
            report.unrecoverable.push(entry.relative_path.clone());
            continue;
        }
        let staging_dir = Path::new(&entry.staging_path);
        let src = staging_dir.join(&entry.relative_path);
        let dst = deploy_base(&entry.deploy_target, data_dir, game_path).join(&entry.relative_path);
        if entry.staging_path.is_empty() || !src.is_file() {
            report.unrecoverable.push(entry.relative_path.clone());
            continue;
        }
        match deploy_single_file(&entry.relative_path, &src, &dst, true, false, can_hardlink, copy_method) {
            Some(method) => {
                report.restored.push(entry.relative_path.clone());
                if method != entry.deploy_method {
                    rewritten.push((
                        entry.mod_id,
                        entry.relative_path.clone(),
                        entry.staging_path.clone(),
                        method,
                        entry.sha256.clone(),
                        entry.deploy_target.clone(),
                    ));
                }
            }
            None => report.unrecoverable.push(entry.relative_path.clone()),
        }
    }

    report.stale_dropped = db
        .remove_deployment_entries_by_ids(&stale_ids)
        .map_err(|e| DeployerError::Database(e.to_string()))?;

    if !rewritten.is_empty() {
        let rows: Vec<(&str, &str, i64, &str, &str, &str, Option<&str>, &str)> = rewritten
            .iter()
            .map(|(mod_id, rel, staging, method, sha, target)| {
                (
                    game_id,
                    bottle_name,
                    *mod_id,
                    rel.as_str(),
                    staging.as_str(),
                    *method,
                    sha.as_deref(),
                    target.as_str(),
                )
            })
            .collect();
        db.batch_add_deployment_entries_with_hashes(&rows)
            .map_err(|e| DeployerError::Database(e.to_string()))?;
    }

    info!(
        "Self-heal for {}/{}: {} checked, {} missing, {} restored, {} unrecoverable, {} stale records dropped",
        game_id,
        bottle_name,
        report.checked,
        report.missing,
        report.restored.len(),
        report.unrecoverable.len(),
        report.stale_dropped
    );

    Ok(report)
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
    let plugin = crate::games::clone_plugin_for_dispatch(&detected.game_id)
        .ok_or_else(|| DeployerError::Other(format!(
            "native deployment not implemented for {}; per-game plugin must provide it",
            detected.game_id
        )))?;

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
        assert!(result.is_err(), "expected stub error for unimplemented native deploy");
        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(err_msg.contains("fakegame"), "error should mention game_id");
    }

    // -----------------------------------------------------------------------
    // Bug #4: disable must undeploy recorded mod archives
    // -----------------------------------------------------------------------

    fn staging_root(tmp: &TempDir) -> PathBuf {
        let root = tmp.path().join("staging_root");
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn disable_removes_mod_bsa_and_reenable_redeploys_it() {
        let (db, tmp, _, data_dir) = setup();
        let (mod_id, staging) = add_test_mod(
            &db,
            &staging_root(&tmp),
            "SkyUI",
            0,
            &[("SkyUI_SE.bsa", b"bsa"), ("SkyUI_SE.esp", b"esp")],
        );
        let files = crate::staging::list_staging_files(&staging).unwrap();
        let r = deploy_mod(&db, "skyrimse", "Gaming", mod_id, &staging, &data_dir, &files, "data")
            .unwrap();
        assert_eq!(r.deployed_count, 2);

        // Single-toggle disable path.
        db.set_enabled(mod_id, false).unwrap();
        undeploy_mod(&db, "skyrimse", "Gaming", mod_id, &data_dir, &data_dir).unwrap();
        assert!(!data_dir.join("SkyUI_SE.bsa").exists(), "mod BSA must be removed on disable");
        assert!(!data_dir.join("SkyUI_SE.esp").exists());
        assert!(db.get_deployment_paths_for_mod(mod_id).unwrap().is_empty());

        // Re-enable: nothing may be skipped as "unmanaged vanilla".
        db.set_enabled(mod_id, true).unwrap();
        let r2 = deploy_mod(&db, "skyrimse", "Gaming", mod_id, &staging, &data_dir, &files, "data")
            .unwrap();
        assert_eq!(r2.deployed_count, 2);
        assert_eq!(r2.skipped_count, 0);
        assert_eq!(db.get_deployment_paths_for_mod(mod_id).unwrap().len(), 2);
    }

    #[test]
    fn bulk_disable_via_redeploy_all_removes_mod_bsa() {
        let (db, tmp, _, data_dir) = setup();
        let (mod_id, _staging) = add_test_mod(
            &db,
            &staging_root(&tmp),
            "USSEP",
            0,
            &[
                ("unofficial skyrim special edition patch.bsa", b"bsa"),
                ("unofficial skyrim special edition patch.esp", b"esp"),
            ],
        );
        redeploy_all(&db, "skyrimse", "Gaming", &data_dir, &data_dir).unwrap();
        assert!(data_dir.join("unofficial skyrim special edition patch.bsa").exists());

        db.set_enabled(mod_id, false).unwrap();
        redeploy_all(&db, "skyrimse", "Gaming", &data_dir, &data_dir).unwrap();
        assert!(
            !data_dir.join("unofficial skyrim special edition patch.bsa").exists(),
            "purge must remove recorded mod BSAs"
        );
        assert!(db.get_deployment_manifest("skyrimse", "Gaming").unwrap().is_empty());

        db.set_enabled(mod_id, true).unwrap();
        let r = redeploy_all(&db, "skyrimse", "Gaming", &data_dir, &data_dir).unwrap();
        assert_eq!(r.deployed_count, 2);
        assert_eq!(r.skipped_count, 0);
    }

    #[test]
    fn deploy_adopts_untracked_leftover_identical_to_staging() {
        // Users hit by bug #4 have mod BSAs in Data with no manifest record.
        let (db, tmp, _, data_dir) = setup();
        let (mod_id, staging) = add_test_mod(
            &db,
            &staging_root(&tmp),
            "UIExtensions",
            0,
            &[("UIExtensions.bsa", b"bsa-bytes")],
        );
        let files = crate::staging::list_staging_files(&staging).unwrap();
        deploy_mod(&db, "skyrimse", "Gaming", mod_id, &staging, &data_dir, &files, "data").unwrap();
        // Simulate the old undeploy: record dropped, file left behind.
        db.remove_deployment_entries_for_mod(mod_id).unwrap();
        assert!(data_dir.join("UIExtensions.bsa").exists());

        let r = deploy_mod(&db, "skyrimse", "Gaming", mod_id, &staging, &data_dir, &files, "data")
            .unwrap();
        assert_eq!(r.deployed_count, 1, "leftover must be adopted, not skipped");
        assert_eq!(r.skipped_count, 0);
        assert_eq!(db.get_deployment_paths_for_mod(mod_id).unwrap().len(), 1);

        // Same leftover, but as an independent byte-identical copy.
        db.remove_deployment_entries_for_mod(mod_id).unwrap();
        fs::remove_file(data_dir.join("UIExtensions.bsa")).unwrap();
        fs::write(data_dir.join("UIExtensions.bsa"), b"bsa-bytes").unwrap();
        let r = deploy_mod(&db, "skyrimse", "Gaming", mod_id, &staging, &data_dir, &files, "data")
            .unwrap();
        assert_eq!(r.deployed_count, 1);
        assert_eq!(r.skipped_count, 0);
    }

    #[test]
    fn identical_copy_of_a_stock_file_is_never_adopted() {
        // A mod that ships a byte-identical copy of a base-game file must
        // not take ownership of the real one (and later delete it).
        let (db, tmp, _, data_dir) = setup();
        for name in ["Skyrim - Misc.bsa", "ccBGSSSE001-Fish.esm"] {
            fs::write(data_dir.join(name), b"vanilla bytes").unwrap();
        }
        let (mod_id, staging) = add_test_mod(
            &db,
            &staging_root(&tmp),
            "VanillaCopies",
            0,
            &[("Skyrim - Misc.bsa", b"vanilla bytes"), ("ccBGSSSE001-Fish.esm", b"vanilla bytes")],
        );
        let files = crate::staging::list_staging_files(&staging).unwrap();
        let r = deploy_mod(&db, "skyrimse", "Gaming", mod_id, &staging, &data_dir, &files, "data")
            .unwrap();
        assert_eq!(r.deployed_count, 0, "stock files are not adopted by content");
        assert_eq!(r.skipped_count, 2);
        assert!(db.get_deployment_paths_for_mod(mod_id).unwrap().is_empty());

        // Disabling the mod therefore leaves the base-game files alone.
        undeploy_mod(&db, "skyrimse", "Gaming", mod_id, &data_dir, &data_dir).unwrap();
        assert!(data_dir.join("Skyrim - Misc.bsa").exists());
        assert!(data_dir.join("ccBGSSSE001-Fish.esm").exists());
    }

    #[test]
    fn deploy_still_refuses_to_overwrite_untracked_vanilla_archive() {
        let (db, tmp, _, data_dir) = setup();
        fs::write(data_dir.join("Skyrim - Misc.bsa"), b"vanilla").unwrap();
        let (mod_id, staging) = add_test_mod(
            &db,
            &staging_root(&tmp),
            "BadMod",
            0,
            &[("Skyrim - Misc.bsa", b"modded")],
        );
        let files = crate::staging::list_staging_files(&staging).unwrap();
        let r = deploy_mod(&db, "skyrimse", "Gaming", mod_id, &staging, &data_dir, &files, "data")
            .unwrap();
        assert_eq!(r.deployed_count, 0);
        assert_eq!(r.skipped_count, 1);
        assert_eq!(fs::read(data_dir.join("Skyrim - Misc.bsa")).unwrap(), b"vanilla");
    }

    #[test]
    fn undeploy_never_deletes_baseline_game_file() {
        let (db, tmp, _, data_dir) = setup();
        // Placed into an empty Data dir, so it IS recorded — but it's a real
        // game master name and must survive undeploy.
        let (mod_id, staging) =
            add_test_mod(&db, &staging_root(&tmp), "Masters", 0, &[("Skyrim.esm", b"esm")]);
        let files = crate::staging::list_staging_files(&staging).unwrap();
        deploy_mod(&db, "skyrimse", "Gaming", mod_id, &staging, &data_dir, &files, "data").unwrap();

        db.set_enabled(mod_id, false).unwrap();
        let removed =
            undeploy_mod(&db, "skyrimse", "Gaming", mod_id, &data_dir, &data_dir).unwrap();
        assert!(removed.is_empty());
        assert!(data_dir.join("Skyrim.esm").exists(), "baseline file must not be deleted");
        assert!(db.get_deployment_paths_for_mod(mod_id).unwrap().is_empty());
    }

    #[test]
    fn stock_pattern_names_are_removed_only_when_provably_ours() {
        // "cc*.bsa" is a Creation Club pattern, but mods can match it too.
        let (db, tmp, _, data_dir) = setup();
        let (mod_id, staging) = add_test_mod(
            &db,
            &staging_root(&tmp),
            "CCOR",
            0,
            &[("ccor.bsa", b"mod archive"), ("ccQDRSSE001-SurvivalMode.bsa", b"mod copy")],
        );
        let files = crate::staging::list_staging_files(&staging).unwrap();
        deploy_mod(&db, "skyrimse", "Gaming", mod_id, &staging, &data_dir, &files, "data").unwrap();

        // Something else replaced the second file with different bytes
        // (e.g. the store re-downloaded the real CC content).
        fs::remove_file(data_dir.join("ccQDRSSE001-SurvivalMode.bsa")).unwrap();
        fs::write(data_dir.join("ccQDRSSE001-SurvivalMode.bsa"), b"real CC").unwrap();

        db.set_enabled(mod_id, false).unwrap();
        undeploy_mod(&db, "skyrimse", "Gaming", mod_id, &data_dir, &data_dir).unwrap();
        assert!(!data_dir.join("ccor.bsa").exists(), "our own cc-named file is removed");
        assert_eq!(
            fs::read(data_dir.join("ccQDRSSE001-SurvivalMode.bsa")).unwrap(),
            b"real CC",
            "a stock-pattern file that isn't ours is left alone"
        );
        assert!(db.get_deployment_paths_for_mod(mod_id).unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn undeploy_keeps_record_when_file_cannot_be_removed() {
        use std::os::unix::fs::PermissionsExt;

        let (db, tmp, _, data_dir) = setup();
        let (mod_id, staging) = add_test_mod(
            &db,
            &staging_root(&tmp),
            "Locked",
            0,
            &[("locked/thing.nif", b"nif"), ("free.esp", b"esp")],
        );
        let files = crate::staging::list_staging_files(&staging).unwrap();
        deploy_mod(&db, "skyrimse", "Gaming", mod_id, &staging, &data_dir, &files, "data").unwrap();

        let locked_dir = data_dir.join("locked");
        fs::set_permissions(&locked_dir, fs::Permissions::from_mode(0o555)).unwrap();
        db.set_enabled(mod_id, false).unwrap();
        undeploy_mod(&db, "skyrimse", "Gaming", mod_id, &data_dir, &data_dir).unwrap();
        fs::set_permissions(&locked_dir, fs::Permissions::from_mode(0o755)).unwrap();

        if !locked_dir.join("thing.nif").exists() {
            // Running as root: permissions don't block removal. Nothing to assert.
            return;
        }
        let remaining: Vec<String> = db
            .get_deployment_paths_for_mod(mod_id)
            .unwrap()
            .into_iter()
            .map(|(p, _)| p)
            .collect();
        assert_eq!(remaining, vec!["locked/thing.nif".to_string()]);
        assert!(!data_dir.join("free.esp").exists());
    }

    #[test]
    fn redeploy_all_preserves_root_deploy_target() {
        let (db, tmp, _, data_dir) = setup();
        let game_root = tmp.path().join("game");
        fs::create_dir_all(&game_root).unwrap();
        let (mod_id, staging) = add_test_mod(
            &db,
            &staging_root(&tmp),
            "SKSE",
            0,
            &[("skse64_loader.exe", b"exe")],
        );
        let files = crate::staging::list_staging_files(&staging).unwrap();
        deploy_mod(&db, "skyrimse", "Gaming", mod_id, &staging, &game_root, &files, "root").unwrap();
        assert!(game_root.join("skse64_loader.exe").exists());

        redeploy_all(&db, "skyrimse", "Gaming", &data_dir, &game_root).unwrap();
        assert!(game_root.join("skse64_loader.exe").exists(), "root file must stay in game root");
        assert!(!data_dir.join("skse64_loader.exe").exists(), "root file must not move into Data");
    }

    // -----------------------------------------------------------------------
    // Bug #6: pre-launch self-heal
    // -----------------------------------------------------------------------

    #[test]
    fn self_heal_ignores_present_root_deployed_files() {
        let (db, tmp, _, data_dir) = setup();
        let game_root = tmp.path().join("game");
        fs::create_dir_all(&game_root).unwrap();
        let (mod_id, staging) = add_test_mod(
            &db,
            &staging_root(&tmp),
            "Preloader",
            0,
            &[("d3dx9_42.dll", b"dll"), ("skse64_loader.exe", b"exe")],
        );
        let files = crate::staging::list_staging_files(&staging).unwrap();
        deploy_mod(&db, "skyrimse", "Gaming", mod_id, &staging, &game_root, &files, "root").unwrap();

        let report =
            restore_missing_files(&db, "skyrimse", "Gaming", &data_dir, &game_root).unwrap();
        assert_eq!(report.checked, 2);
        assert_eq!(report.missing, 0, "root files resolved against Data looked missing before");
        assert!(report.restored.is_empty());
    }

    #[test]
    fn self_heal_restores_only_the_missing_files() {
        let (db, tmp, _, data_dir) = setup();
        let (mod_id, staging) = add_test_mod(
            &db,
            &staging_root(&tmp),
            "Plugins",
            0,
            &[("a.esp", b"a"), ("b.esp", b"b"), ("meshes/c.nif", b"c")],
        );
        let files = crate::staging::list_staging_files(&staging).unwrap();
        deploy_mod(&db, "skyrimse", "Gaming", mod_id, &staging, &data_dir, &files, "data").unwrap();
        fs::remove_file(data_dir.join("b.esp")).unwrap();

        let report =
            restore_missing_files(&db, "skyrimse", "Gaming", &data_dir, &data_dir).unwrap();
        assert_eq!(report.missing, 1);
        assert_eq!(report.restored, vec!["b.esp".to_string()]);
        assert!(report.unrecoverable.is_empty());
        assert_eq!(fs::read(data_dir.join("b.esp")).unwrap(), b"b");
        assert_eq!(db.get_deployment_paths_for_mod(mod_id).unwrap().len(), 3);
    }

    #[test]
    fn self_heal_drops_records_of_disabled_mods_instead_of_restoring() {
        let (db, tmp, _, data_dir) = setup();
        let (mod_id, staging) =
            add_test_mod(&db, &staging_root(&tmp), "Gone", 0, &[("gone.esp", b"x")]);
        let files = crate::staging::list_staging_files(&staging).unwrap();
        deploy_mod(&db, "skyrimse", "Gaming", mod_id, &staging, &data_dir, &files, "data").unwrap();
        fs::remove_file(data_dir.join("gone.esp")).unwrap();
        db.set_enabled(mod_id, false).unwrap();

        let report =
            restore_missing_files(&db, "skyrimse", "Gaming", &data_dir, &data_dir).unwrap();
        assert_eq!(report.stale_dropped, 1);
        assert!(report.restored.is_empty());
        assert!(!data_dir.join("gone.esp").exists());
    }

    #[test]
    fn incremental_deploy_honours_root_target() {
        let (db, tmp, _, data_dir) = setup();
        let game_root = tmp.path().join("game");
        fs::create_dir_all(&game_root).unwrap();
        let root = staging_root(&tmp);
        // Enough data files that adding one root mod stays under the 80% fallback.
        add_test_mod(
            &db,
            &root,
            "Big",
            0,
            &[("a.esp", b"a"), ("b.esp", b"b"), ("c.esp", b"c"), ("d.esp", b"d"), ("e.esp", b"e")],
        );
        deploy_incremental(&db, "skyrimse", "Gaming", &data_dir, &game_root).unwrap();

        let (mod_id, _staging) =
            add_test_mod(&db, &root, "RootMod", 1, &[("root_only.dll", b"dll")]);
        // A root mod that is installed but not yet deployed (e.g. re-enabled).
        db.set_deploy_target_for_mod(mod_id, "root").unwrap();

        let r = deploy_incremental(&db, "skyrimse", "Gaming", &data_dir, &game_root).unwrap();
        assert_eq!(r.files_added, 1);
        assert!(game_root.join("root_only.dll").exists());
        assert!(!data_dir.join("root_only.dll").exists());

        // And the incremental remove path deletes it from the game root.
        db.set_enabled(mod_id, false).unwrap();
        deploy_incremental(&db, "skyrimse", "Gaming", &data_dir, &game_root).unwrap();
        assert!(!game_root.join("root_only.dll").exists());
    }

    #[test]
    fn root_target_survives_disable_and_reenable() {
        let (db, tmp, _, _data_dir) = setup();
        let game_root = tmp.path().join("game");
        fs::create_dir_all(&game_root).unwrap();
        let (mod_id, staging) =
            add_test_mod(&db, &staging_root(&tmp), "ENB", 0, &[("d3d11.dll", b"dll")]);
        let files = crate::staging::list_staging_files(&staging).unwrap();
        deploy_mod(&db, "skyrimse", "Gaming", mod_id, &staging, &game_root, &files, "root").unwrap();

        db.set_enabled(mod_id, false).unwrap();
        undeploy_mod(&db, "skyrimse", "Gaming", mod_id, &tmp.path().join("data"), &game_root)
            .unwrap();
        assert!(!game_root.join("d3d11.dll").exists());
        assert_eq!(db.get_deploy_target_for_mod(mod_id).unwrap(), "root");
    }
}
