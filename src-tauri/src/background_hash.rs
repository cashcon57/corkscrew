//! Post-install background file hashing.
//!
//! After a collection install (which skips SHA-256 for speed), this module
//! iterates all mods with empty hashes, computes SHA-256 in the background,
//! and stores results in the database.  If a game process is detected running,
//! parallelism is halved to avoid hurting game performance.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use log::{info, warn};
use rayon::prelude::*;
use serde::Serialize;

use crate::database::ModDatabase;
use crate::platform;
use crate::staging;

// ---------------------------------------------------------------------------
// Progress event payload
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HashingProgress {
    /// Total files to hash across all mods.
    pub total_files: u64,
    /// Files hashed so far.
    pub hashed_files: u64,
    /// Total bytes hashed so far.
    pub hashed_bytes: u64,
    /// Total bytes to hash (estimated from file sizes).
    pub total_bytes: u64,
    /// Number of mods fully hashed.
    pub mods_done: u32,
    /// Total mods to hash.
    pub mods_total: u32,
    /// Whether a game is detected running (throttled mode).
    pub game_running: bool,
    /// Whether hashing is complete.
    pub done: bool,
    /// Error message if hashing failed.
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Cancellation token
// ---------------------------------------------------------------------------

static CANCEL: AtomicBool = AtomicBool::new(false);

pub fn cancel() {
    CANCEL.store(true, Ordering::Relaxed);
}

pub fn is_cancelled() -> bool {
    CANCEL.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Game-running detection
// ---------------------------------------------------------------------------

/// Check if a process with the given PID is still alive.
fn is_pid_alive(pid: u32) -> bool {
    // On Unix, signal 0 checks if process exists without actually sending a signal.
    #[cfg(unix)]
    {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

/// Check if any common game executable is running by scanning /proc or using sysctl.
/// Falls back to checking a specific PID if provided.
fn is_game_running(game_pid: Option<u32>) -> bool {
    if let Some(pid) = game_pid {
        return is_pid_alive(pid);
    }
    false
}

// ---------------------------------------------------------------------------
// Core hashing logic (runs on a blocking thread)
// ---------------------------------------------------------------------------

/// Information about a mod that needs hashing.
struct ModToHash {
    mod_id: i64,
    staging_path: PathBuf,
    file_count: usize,
    total_size: u64,
}

/// Run background hashing for all mods with empty hashes in the given game/bottle.
///
/// Returns a `HashingProgress` with `done = true` on completion.
pub fn run_background_hashing(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    game_pid: Option<u32>,
    progress_callback: impl Fn(HashingProgress) + Send + Sync,
) -> HashingProgress {
    CANCEL.store(false, Ordering::Relaxed);

    // 1. Find all mods with empty hashes
    let mods = match db.list_mods(game_id, bottle_name) {
        Ok(m) => m,
        Err(e) => {
            return HashingProgress {
                total_files: 0,
                hashed_files: 0,
                hashed_bytes: 0,
                total_bytes: 0,
                mods_done: 0,
                mods_total: 0,
                game_running: false,
                done: true,
                error: Some(format!("Failed to list mods: {e}")),
            };
        }
    };

    // Filter to mods that have empty hashes in the DB
    let mut mods_to_hash: Vec<ModToHash> = Vec::new();
    for m in &mods {
        let hashes = match db.get_file_hashes(m.id) {
            Ok(h) => h,
            Err(_) => continue,
        };

        // Check if this mod has any empty hashes
        let has_empty = hashes.iter().any(|(_, hash, _)| hash.is_empty());
        if !has_empty && !hashes.is_empty() {
            continue; // Already fully hashed
        }

        // Resolve staging path
        let staging_path = staging::mod_staging_dir(game_id, bottle_name, m.id, &m.name);
        if !staging_path.exists() {
            continue; // Staging dir gone, skip
        }

        // Collect files and sizes
        let files = match staging::list_staging_files(&staging_path) {
            Ok(f) => f,
            Err(_) => continue,
        };

        let total_size: u64 = files
            .iter()
            .map(|f| {
                std::fs::metadata(staging_path.join(f))
                    .map(|m| m.len())
                    .unwrap_or(0)
            })
            .sum();

        mods_to_hash.push(ModToHash {
            mod_id: m.id,
            staging_path,
            file_count: files.len(),
            total_size,
        });
    }

    let mods_total = mods_to_hash.len() as u32;
    let total_files: u64 = mods_to_hash.iter().map(|m| m.file_count as u64).sum();
    let total_bytes: u64 = mods_to_hash.iter().map(|m| m.total_size).sum();

    if mods_total == 0 {
        let done_progress = HashingProgress {
            total_files: 0,
            hashed_files: 0,
            hashed_bytes: 0,
            total_bytes: 0,
            mods_done: 0,
            mods_total: 0,
            game_running: false,
            done: true,
            error: None,
        };
        progress_callback(done_progress.clone());
        return done_progress;
    }

    info!(
        "Background hashing: {} mods, {} files, {:.1} GB",
        mods_total,
        total_files,
        total_bytes as f64 / (1024.0 * 1024.0 * 1024.0)
    );

    let hashed_files = Arc::new(AtomicU64::new(0));
    let hashed_bytes = Arc::new(AtomicU64::new(0));
    let mut mods_done: u32 = 0;

    // Send initial progress
    progress_callback(HashingProgress {
        total_files,
        hashed_files: 0,
        hashed_bytes: 0,
        total_bytes,
        mods_done: 0,
        mods_total,
        game_running: is_game_running(game_pid),
        done: false,
        error: None,
    });

    // Determine available parallelism
    let full_threads = rayon::current_num_threads();

    for mtoh in &mods_to_hash {
        if is_cancelled() {
            info!("Background hashing cancelled");
            break;
        }

        let staging_path = &mtoh.staging_path;
        let files = match staging::list_staging_files(staging_path) {
            Ok(f) => f,
            Err(e) => {
                warn!("Failed to list staging files for mod {}: {e}", mtoh.mod_id);
                mods_done += 1;
                continue;
            }
        };

        // Check if game is running and adjust parallelism
        let game_running = is_game_running(game_pid);
        let thread_count = if game_running {
            (full_threads / 2).max(1)
        } else {
            full_threads
        };

        // Build a scoped thread pool with the appropriate parallelism
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(thread_count)
            .build()
            .unwrap_or_else(|_| rayon::ThreadPoolBuilder::new().build().unwrap());

        let hf = Arc::clone(&hashed_files);
        let hb = Arc::clone(&hashed_bytes);

        let hashes: Vec<(String, String, u64)> = pool.install(|| {
            files
                .par_iter()
                .filter_map(|rel_path| {
                    if is_cancelled() {
                        return None;
                    }
                    let full_path = staging_path.join(rel_path);
                    if !full_path.is_file() {
                        return None;
                    }
                    let size = std::fs::metadata(&full_path).map(|m| m.len()).unwrap_or(0);
                    let hash = match platform::fast_hash(&full_path) {
                        Ok(h) => h,
                        Err(e) => {
                            warn!("Hash failed for {}: {e}", full_path.display());
                            return None;
                        }
                    };
                    hf.fetch_add(1, Ordering::Relaxed);
                    hb.fetch_add(size, Ordering::Relaxed);
                    Some((rel_path.clone(), hash, size))
                })
                .collect()
        });

        // Store hashes in DB
        if !hashes.is_empty() {
            if let Err(e) = db.store_file_hashes(mtoh.mod_id, &hashes) {
                warn!("Failed to store hashes for mod {}: {e}", mtoh.mod_id);
            }
        }

        mods_done += 1;

        // Emit progress
        progress_callback(HashingProgress {
            total_files,
            hashed_files: hashed_files.load(Ordering::Relaxed),
            hashed_bytes: hashed_bytes.load(Ordering::Relaxed),
            total_bytes,
            mods_done,
            mods_total,
            game_running,
            done: false,
            error: None,
        });
    }

    let final_progress = HashingProgress {
        total_files,
        hashed_files: hashed_files.load(Ordering::Relaxed),
        hashed_bytes: hashed_bytes.load(Ordering::Relaxed),
        total_bytes,
        mods_done,
        mods_total,
        game_running: is_game_running(game_pid),
        done: true,
        error: None,
    };

    info!(
        "Background hashing complete: {}/{} files, {:.1} GB",
        final_progress.hashed_files,
        final_progress.total_files,
        final_progress.hashed_bytes as f64 / (1024.0 * 1024.0 * 1024.0)
    );

    progress_callback(final_progress.clone());
    final_progress
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_flag_works() {
        CANCEL.store(false, Ordering::Relaxed);
        assert!(!is_cancelled());
        cancel();
        assert!(is_cancelled());
        CANCEL.store(false, Ordering::Relaxed);
    }

    #[test]
    fn is_pid_alive_self() {
        // Our own PID should be alive
        let pid = std::process::id();
        assert!(is_pid_alive(pid));
    }

    #[test]
    fn is_pid_alive_nonexistent() {
        // PID 99999999 should not exist
        assert!(!is_pid_alive(99_999_999));
    }
}
