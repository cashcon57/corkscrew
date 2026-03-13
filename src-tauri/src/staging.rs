//! Staging folder management for mod files.
//!
//! Instead of installing mod files directly into the game directory, they are
//! first extracted into a per-mod staging folder. Deployment (hardlink/copy)
//! then links files from staging into the game directory. This enables:
//!
//! - Non-destructive enable/disable (remove links, keep staging)
//! - Rollback and re-deployment
//! - File integrity verification via SHA-256 hashes
//! - Conflict resolution (multiple mods can coexist in staging)

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use log::{debug, info};
use rayon::prelude::*;
use thiserror::Error;
use walkdir::WalkDir;

use crate::config;
use crate::installer;
use crate::platform;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum StagingError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("Installer error: {0}")]
    Installer(#[from] installer::InstallerError),

    #[error("WalkDir error: {0}")]
    WalkDir(#[from] walkdir::Error),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, StagingError>;

/// File extensions that indicate root-level files (game executables, loaders,
/// DLL injectors) which should be deployed alongside the game executable
/// rather than into the Data/ directory.
const ROOT_FILE_EXTENSIONS: &[&str] = &["exe", "dll"];

/// When `find_data_root()` resolves to a `Data/` subdirectory, check its
/// parent for sibling files that look like root-level content (.exe, .dll).
/// If found, copy them into a `Root/` folder inside `staging_dir` so that
/// the collection installer's existing Root folder detection picks them up.
///
/// This handles archives like SKSE which contain both root executables and
/// a `Data/` directory with scripts/plugins:
/// ```text
/// skse64_2_02_06/
///   skse64_loader.exe   → staging/Root/skse64_loader.exe
///   skse64_*.dll        → staging/Root/skse64_*.dll
///   Data/
///     Scripts/...       → staging/Scripts/...
///     SKSE/...          → staging/SKSE/...
/// ```
fn preserve_root_files(data_root: &Path, staging_dir: &Path) {
    // Only applies when find_data_root returned a "Data" subdirectory.
    let dir_name = data_root
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase());
    if dir_name.as_deref() != Some("data") {
        return;
    }

    let parent = match data_root.parent() {
        Some(p) => p,
        None => return,
    };

    let entries = match fs::read_dir(parent) {
        Ok(e) => e,
        Err(_) => return,
    };

    let root_staging = staging_dir.join("Root");
    let mut count = 0u32;

    for entry in entries.flatten() {
        // Only interested in files (not directories like Data/).
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let ext = name_str
            .rsplit('.')
            .next()
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        if ROOT_FILE_EXTENSIONS.contains(&ext.as_str()) {
            if count == 0 {
                let _ = fs::create_dir_all(&root_staging);
            }
            let dest = root_staging.join(&name);
            if fs::rename(entry.path(), &dest).is_err() {
                let _ = fs::copy(entry.path(), &dest);
            }
            info!(
                "Preserved root file: {} → Root/{}",
                name_str,
                name.to_string_lossy()
            );
            count += 1;
        }
    }

    if count > 0 {
        info!(
            "Preserved {} root files from sibling of Data/ directory",
            count
        );
    }
}

// ---------------------------------------------------------------------------
// Path safety
// ---------------------------------------------------------------------------

/// Check whether a relative path is safe (no directory traversal, no absolute
/// paths). Returns `true` for safe paths, `false` for anything suspicious.
///
/// Used to validate paths from untrusted sources (FOMOD XML, collection
/// manifests, archive entries) before joining them with a base directory.
pub fn is_safe_relative_path(path: &str) -> bool {
    !path.contains("..")
        && !path.starts_with('/')
        && !path.starts_with('\\')
        && !path.contains(":/")
        && !path.contains(":\\")
        && !path.contains('\0')
        && !path.starts_with("\\\\?\\")
        // Reject Windows drive letters (e.g. "C:", "D:")
        && !(path.len() >= 2
            && path.as_bytes()[0].is_ascii_alphabetic()
            && path.as_bytes()[1] == b':')
}

// ---------------------------------------------------------------------------
// StagingResult
// ---------------------------------------------------------------------------

/// Result of staging a mod archive.
pub struct StagingResult {
    /// Absolute path to the mod's staging directory.
    pub staging_path: PathBuf,
    /// Relative file paths within the staging directory.
    pub files: Vec<String>,
    /// File hashes: (relative_path, sha256_hex, file_size).
    pub hashes: Vec<(String, String, u64)>,
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Returns the base staging directory for all mods.
/// Uses the configured override if set, otherwise falls back to the default
/// location under the platform data directory.
pub fn staging_root() -> PathBuf {
    if let Ok(cfg) = config::get_config() {
        if let Some(ref dir) = cfg.staging_dir {
            if !dir.is_empty() {
                return PathBuf::from(dir);
            }
        }
    }
    config::data_dir().join("staging")
}

/// Returns the staging directory for a specific game/bottle.
pub fn staging_base_dir(game_id: &str, bottle_name: &str) -> PathBuf {
    staging_root()
        .join(game_id)
        .join(sanitize_name(bottle_name))
}

/// Returns the staging directory for a specific mod.
pub fn mod_staging_dir(game_id: &str, bottle_name: &str, mod_id: i64, mod_name: &str) -> PathBuf {
    staging_base_dir(game_id, bottle_name).join(format!("{}_{}", mod_id, sanitize_name(mod_name)))
}

/// Sanitize a name for use as a directory component.
fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Staging operations
// ---------------------------------------------------------------------------

/// Stage a mod: extract the archive into a staging folder, find the data root,
/// and compute SHA-256 hashes for all files.
///
/// The staging directory is: `<staging_root>/<game_id>/<bottle>/<mod_id>_<name>/`
///
/// Returns the staging result with file list and hashes.
pub fn stage_mod(
    archive_path: &Path,
    game_id: &str,
    bottle_name: &str,
    mod_id: i64,
    mod_name: &str,
) -> Result<StagingResult> {
    // Estimate ~3x archive size for extracted content.
    let archive_size = fs::metadata(archive_path).map(|m| m.len()).unwrap_or(0);
    let estimated_extracted = archive_size.saturating_mul(3);
    let staging_root = staging_root();
    crate::disk_budget::check_space_guard(&staging_root, estimated_extracted)
        .map_err(StagingError::Other)?;

    let staging_dir = mod_staging_dir(game_id, bottle_name, mod_id, mod_name);

    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir)?;
    }
    fs::create_dir_all(&staging_dir)?;

    let temp_dir = std::env::temp_dir().join(format!("corkscrew_stage_{}", std::process::id()));
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir)?;
    }
    fs::create_dir_all(&temp_dir)?;

    let _temp_guard = TempDirGuard(temp_dir.clone());

    info!(
        "Staging mod '{}' from {} -> {}",
        mod_name,
        archive_path.display(),
        staging_dir.display()
    );

    installer::extract_archive(archive_path, &temp_dir)?;

    let data_root = installer::find_data_root(&temp_dir);
    debug!("Data root for staging: {}", data_root.display());

    // If find_data_root resolved to a Data/ subdirectory, preserve sibling
    // .exe/.dll files as Root/ content for game-root deployment.
    preserve_root_files(&data_root, &staging_dir);

    // Detect the optimal copy method once for the entire batch.
    let copy_method = platform::detect_copy_method(&data_root, &staging_dir);
    debug!("Staging copy method: {:?}", copy_method);

    // Collect all file entries first, then process in parallel
    let entries: Vec<_> = WalkDir::new(&data_root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| !installer::is_junk_file(e.path()))
        .collect();

    // Parallel copy + hash: each file is read once, written + hashed simultaneously
    let results: Vec<std::result::Result<(String, String, u64), StagingError>> = entries
        .par_iter()
        .map(|entry| {
            let abs_src = entry.path();
            let relative = abs_src
                .strip_prefix(&data_root)
                .map_err(|e| StagingError::Other(e.to_string()))?;

            let dest_path = staging_dir.join(relative);
            if let Some(parent) = dest_path.parent() {
                let _ = fs::create_dir_all(parent);
            }

            let (hash, file_size) = copy_and_hash(abs_src, &dest_path, copy_method)?;
            let rel_str = relative.to_string_lossy().replace('\\', "/");
            Ok((rel_str, hash, file_size))
        })
        .collect();

    let mut files: Vec<String> = Vec::with_capacity(results.len());
    let mut hashes: Vec<(String, String, u64)> = Vec::with_capacity(results.len());

    for result in results {
        let (rel_str, hash, file_size) = result?;
        files.push(rel_str.clone());
        hashes.push((rel_str, hash, file_size));
    }

    info!(
        "Staged {} files for mod '{}' at {}",
        files.len(),
        mod_name,
        staging_dir.display()
    );

    Ok(StagingResult {
        staging_path: staging_dir,
        files,
        hashes,
    })
}

/// Stage a mod from a pre-extracted directory instead of an archive.
///
/// Skips the archive extraction step (which was already done concurrently)
/// and copies files from the pre-extracted directory into the staging folder.
///
/// When `skip_hash` is true, files are copied without computing SHA-256 hashes,
/// which eliminates a full re-read of every file on CoW filesystems (APFS/Btrfs).
pub fn stage_mod_from_extracted(
    extracted_dir: &Path,
    game_id: &str,
    bottle_name: &str,
    mod_id: i64,
    mod_name: &str,
) -> Result<StagingResult> {
    stage_mod_from_extracted_opts(extracted_dir, game_id, bottle_name, mod_id, mod_name, false)
}

/// Stage a mod from a pre-extracted directory with optional hash skipping.
pub fn stage_mod_from_extracted_opts(
    extracted_dir: &Path,
    game_id: &str,
    bottle_name: &str,
    mod_id: i64,
    mod_name: &str,
    skip_hash: bool,
) -> Result<StagingResult> {
    let staging_dir = mod_staging_dir(game_id, bottle_name, mod_id, mod_name);

    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir)?;
    }
    fs::create_dir_all(&staging_dir)?;

    let data_root = installer::find_data_root(extracted_dir);
    debug!(
        "Data root for pre-extracted staging: {}",
        data_root.display()
    );

    // If find_data_root resolved to a Data/ subdirectory, preserve sibling
    // .exe/.dll files as Root/ content for game-root deployment.
    preserve_root_files(&data_root, &staging_dir);

    // Detect the optimal copy method once for the entire batch.
    let copy_method = platform::detect_copy_method(&data_root, &staging_dir);
    debug!(
        "Pre-extracted staging copy method: {:?} (skip_hash={})",
        copy_method, skip_hash
    );

    // Collect all file entries first, then process in parallel
    let entries: Vec<_> = WalkDir::new(&data_root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| !installer::is_junk_file(e.path()))
        .collect();

    // Parallel copy (+ optional hash)
    let results: Vec<std::result::Result<(String, String, u64), StagingError>> = entries
        .par_iter()
        .map(|entry| {
            let abs_src = entry.path();
            let relative = abs_src
                .strip_prefix(&data_root)
                .map_err(|e| StagingError::Other(e.to_string()))?;

            let dest_path = staging_dir.join(relative);
            if let Some(parent) = dest_path.parent() {
                let _ = fs::create_dir_all(parent); // idempotent, safe for parallel
            }

            let (hash, file_size) = if skip_hash {
                copy_no_hash(abs_src, &dest_path, copy_method)?
            } else {
                copy_and_hash(abs_src, &dest_path, copy_method)?
            };
            let rel_str = relative.to_string_lossy().replace('\\', "/");
            Ok((rel_str, hash, file_size))
        })
        .collect();

    let mut files: Vec<String> = Vec::with_capacity(results.len());
    let mut hashes: Vec<(String, String, u64)> = Vec::with_capacity(results.len());

    for result in results {
        let (rel_str, hash, file_size) = result?;
        files.push(rel_str.clone());
        hashes.push((rel_str, hash, file_size));
    }

    info!(
        "Staged {} files for mod '{}' from pre-extracted dir at {} (skip_hash={})",
        files.len(),
        mod_name,
        staging_dir.display(),
        skip_hash,
    );

    Ok(StagingResult {
        staging_path: staging_dir,
        files,
        hashes,
    })
}

/// Stage a mod by extracting an archive directly into the staging directory.
///
/// This is the fast path for collection installs:
/// 1. Extract archive into a temp subdir within the staging folder
/// 2. Find the data root inside the extracted content
/// 3. Move (rename) files from data root to staging root (same FS = instant)
/// 4. Optionally skip SHA-256 hashing (controlled by `skip_hash`)
///
/// This eliminates the temp dir → copy → staging pipeline, saving one full
/// write pass of all extracted data.
pub fn stage_mod_extract_direct(
    archive_path: &Path,
    game_id: &str,
    bottle_name: &str,
    mod_id: i64,
    mod_name: &str,
    skip_hash: bool,
) -> Result<StagingResult> {
    let staging_dir = mod_staging_dir(game_id, bottle_name, mod_id, mod_name);

    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir)?;
    }
    fs::create_dir_all(&staging_dir)?;

    // Extract into a temp subdir within staging (same filesystem for instant rename).
    let extract_subdir = staging_dir.join("__extract_tmp");
    fs::create_dir_all(&extract_subdir)?;

    info!(
        "Direct-staging mod '{}' from {} -> {}",
        mod_name,
        archive_path.display(),
        staging_dir.display()
    );

    installer::extract_archive(archive_path, &extract_subdir)?;

    // Find the data root (unwrap nested single-folder archives).
    let data_root = installer::find_data_root(&extract_subdir);
    debug!("Data root for direct-staging: {}", data_root.display());

    // If find_data_root resolved to a Data/ subdirectory, preserve sibling
    // .exe/.dll files as Root/ content for game-root deployment.
    preserve_root_files(&data_root, &staging_dir);

    // Move files from data_root → staging_dir (same filesystem = rename is instant).
    // We move each entry from data_root directly into staging_dir.
    let entries_to_move: Vec<_> = fs::read_dir(&data_root)?.filter_map(|e| e.ok()).collect();

    for entry in &entries_to_move {
        let dest = staging_dir.join(entry.file_name());
        // Rename is instant on the same filesystem.
        if fs::rename(entry.path(), &dest).is_err() {
            // Fallback: if rename fails (shouldn't happen, same FS), do a copy.
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                copy_dir_recursive(&entry.path(), &dest)?;
            } else {
                fs::copy(entry.path(), &dest)?;
            }
        }
    }

    // Remove the temp extraction subdir (now empty or containing only leftovers).
    let _ = fs::remove_dir_all(&extract_subdir);

    // Walk the staging dir to collect file list and optionally hash.
    let file_entries: Vec<_> = WalkDir::new(&staging_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| !installer::is_junk_file(e.path()))
        .collect();

    let mut files: Vec<String> = Vec::with_capacity(file_entries.len());
    let mut hashes: Vec<(String, String, u64)> = Vec::with_capacity(file_entries.len());

    if skip_hash {
        // Fast path: just collect file paths and sizes, no SHA-256.
        for entry in &file_entries {
            let abs = entry.path();
            let relative = abs
                .strip_prefix(&staging_dir)
                .map_err(|e| StagingError::Other(e.to_string()))?;
            let rel_str = relative.to_string_lossy().replace('\\', "/");
            let size = fs::metadata(abs).map(|m| m.len()).unwrap_or(0);
            files.push(rel_str.clone());
            hashes.push((rel_str, String::new(), size));
        }
    } else {
        // Full path: parallel hash computation via rayon.
        let results: Vec<std::result::Result<(String, String, u64), StagingError>> = file_entries
            .par_iter()
            .map(|entry| {
                let abs = entry.path();
                let relative = abs
                    .strip_prefix(&staging_dir)
                    .map_err(|e| StagingError::Other(e.to_string()))?;
                let rel_str = relative.to_string_lossy().replace('\\', "/");
                let hash = compute_sha256(abs)?;
                let size = fs::metadata(abs).map(|m| m.len()).unwrap_or(0);
                Ok((rel_str, hash, size))
            })
            .collect();

        for result in results {
            let (rel_str, hash, size) = result?;
            files.push(rel_str.clone());
            hashes.push((rel_str, hash, size));
        }
    }

    info!(
        "Direct-staged {} files for mod '{}' at {} (skip_hash={})",
        files.len(),
        mod_name,
        staging_dir.display(),
        skip_hash,
    );

    Ok(StagingResult {
        staging_path: staging_dir,
        files,
        hashes,
    })
}

/// Recursively copy a directory (fallback if rename fails across filesystems).
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let dest = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&entry.path(), &dest)?;
        } else {
            fs::copy(entry.path(), &dest)?;
        }
    }
    Ok(())
}

/// Remove a mod's staging directory entirely.
pub fn remove_staging(staging_path: &Path) -> Result<()> {
    if staging_path.exists() {
        fs::remove_dir_all(staging_path)?;
        info!("Removed staging directory: {}", staging_path.display());
    }
    Ok(())
}

/// Verify staging file integrity by recomputing hashes.
/// Returns a list of files whose hash doesn't match (empty = all good).
pub fn verify_staging_integrity(
    staging_path: &Path,
    expected_hashes: &[(String, String, u64)],
) -> Result<Vec<String>> {
    let mut mismatched = Vec::new();

    for (rel_path, expected_hash, _expected_size) in expected_hashes {
        // Reject path traversal attempts
        if !is_safe_relative_path(rel_path) {
            log::warn!("Skipping integrity check for unsafe path: {}", rel_path);
            mismatched.push(rel_path.clone());
            continue;
        }

        let full_path = staging_path.join(rel_path);

        if !full_path.exists() {
            mismatched.push(rel_path.clone());
            continue;
        }

        let actual_hash = compute_sha256(&full_path)?;
        if actual_hash != *expected_hash {
            mismatched.push(rel_path.clone());
        }
    }

    Ok(mismatched)
}

/// List all files in a staging directory (relative paths).
pub fn list_staging_files(staging_path: &Path) -> Result<Vec<String>> {
    if !staging_path.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    for entry in WalkDir::new(staging_path)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        if installer::is_junk_file(entry.path()) {
            continue;
        }

        if let Ok(relative) = entry.path().strip_prefix(staging_path) {
            let rel_str = relative.to_string_lossy().replace('\\', "/");
            files.push(rel_str);
        }
    }

    Ok(files)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Compute the SHA-256 hash of a file, returning the hex string.
///
/// Uses memory-mapped I/O for files larger than 1 MiB, falling back to
/// buffered 128 KiB reads for smaller files.
pub fn compute_sha256(path: &Path) -> Result<String> {
    platform::fast_hash(path).map_err(StagingError::Io)
}

/// Copy a file and compute its SHA-256 hash.
///
/// Uses platform-optimized copy (clonefile on macOS APFS, reflink on Linux
/// Btrfs/XFS) when available, falling back to a single-pass buffered
/// copy+hash for standard filesystems.
fn copy_and_hash(
    src: &Path,
    dst: &Path,
    copy_method: platform::FsCopyMethod,
) -> Result<(String, u64)> {
    platform::fast_copy_and_hash(src, dst, copy_method).map_err(StagingError::Io)
}

/// Copy a file without computing SHA-256 hash (fast path for collection installs).
///
/// Returns ("", file_size) — empty hash string indicates hash was skipped.
fn copy_no_hash(
    src: &Path,
    dst: &Path,
    copy_method: platform::FsCopyMethod,
) -> Result<(String, u64)> {
    platform::fast_copy(src, dst, copy_method).map_err(StagingError::Io)?;
    let size = fs::metadata(dst).map(|m| m.len()).unwrap_or(0);
    Ok((String::new(), size))
}

/// RAII guard that removes a temporary directory when dropped.
struct TempDirGuard(PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        if self.0.exists() {
            if let Err(e) = fs::remove_dir_all(&self.0) {
                log::warn!("Failed to clean up temp dir {}: {}", self.0.display(), e);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn sanitize_name_replaces_special_chars() {
        assert_eq!(sanitize_name("My Cool Mod!"), "My_Cool_Mod_");
        assert_eq!(sanitize_name("mod-name_v1.0"), "mod-name_v1.0");
        assert_eq!(sanitize_name("a/b\\c"), "a_b_c");
    }

    #[test]
    fn staging_dir_layout() {
        let dir = mod_staging_dir("skyrimse", "Gaming Bottle", 42, "SkyUI");
        let dir_str = dir.to_string_lossy();
        assert!(dir_str.contains("staging"));
        assert!(dir_str.contains("skyrimse"));
        assert!(dir_str.contains("Gaming_Bottle"));
        assert!(dir_str.contains("42_SkyUI"));
    }

    #[test]
    fn compute_sha256_works() {
        let tmp = tempfile::tempdir().unwrap();
        let file_path = tmp.path().join("test.txt");
        fs::write(&file_path, b"hello world").unwrap();

        let hash = compute_sha256(&file_path).unwrap();
        // Known SHA-256 of "hello world"
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn list_staging_files_returns_empty_for_nonexistent() {
        let files = list_staging_files(Path::new("/tmp/corkscrew_nonexistent_staging")).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn list_staging_files_finds_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("staging");
        fs::create_dir_all(dir.join("meshes")).unwrap();
        fs::write(dir.join("meshes/test.nif"), b"nif").unwrap();
        fs::write(dir.join("mod.esp"), b"esp").unwrap();

        let files = list_staging_files(&dir).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.contains(&"meshes/test.nif".to_string()));
        assert!(files.contains(&"mod.esp".to_string()));
    }

    #[test]
    fn verify_integrity_detects_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let staging = tmp.path().join("staging");
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("test.esp"), b"original").unwrap();

        let original_hash = compute_sha256(&staging.join("test.esp")).unwrap();
        let hashes = vec![("test.esp".to_string(), original_hash, 8)];

        let bad = verify_staging_integrity(&staging, &hashes).unwrap();
        assert!(bad.is_empty());

        fs::write(staging.join("test.esp"), b"corrupted").unwrap();
        let bad = verify_staging_integrity(&staging, &hashes).unwrap();
        assert_eq!(bad.len(), 1);
        assert_eq!(bad[0], "test.esp");
    }

    #[test]
    fn verify_integrity_detects_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let staging = tmp.path();

        let hashes = vec![("missing.esp".to_string(), "abc123".to_string(), 100)];
        let bad = verify_staging_integrity(staging, &hashes).unwrap();
        assert_eq!(bad.len(), 1);
    }

    #[test]
    fn preserve_root_files_creates_root_dir() {
        // Simulate an SKSE-like archive: Data/ dir + sibling .exe/.dll files.
        let tmp = tempfile::tempdir().unwrap();
        let archive_root = tmp.path().join("skse64_2_02_06");
        let data_subdir = archive_root.join("Data");
        let scripts_dir = data_subdir.join("Scripts");
        fs::create_dir_all(&scripts_dir).unwrap();
        fs::write(scripts_dir.join("SKSE.pex"), b"pex").unwrap();

        // Root-level files alongside Data/
        fs::write(archive_root.join("skse64_loader.exe"), b"loader").unwrap();
        fs::write(archive_root.join("skse64_1_6_1170.dll"), b"dll").unwrap();
        fs::write(archive_root.join("skse64_readme.txt"), b"readme").unwrap(); // not .exe/.dll

        let staging = tmp.path().join("staging");
        fs::create_dir_all(&staging).unwrap();

        // Call preserve_root_files with data_root pointing to Data/
        preserve_root_files(&data_subdir, &staging);

        // Root/ should exist with .exe and .dll files
        let root_dir = staging.join("Root");
        assert!(root_dir.is_dir());
        assert!(root_dir.join("skse64_loader.exe").exists());
        assert!(root_dir.join("skse64_1_6_1170.dll").exists());
        // .txt should NOT be preserved (only .exe/.dll)
        assert!(!root_dir.join("skse64_readme.txt").exists());
    }

    #[test]
    fn preserve_root_files_noop_when_not_data_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let non_data = tmp.path().join("meshes");
        fs::create_dir_all(&non_data).unwrap();

        let staging = tmp.path().join("staging");
        fs::create_dir_all(&staging).unwrap();

        preserve_root_files(&non_data, &staging);

        // Root/ should NOT be created
        assert!(!staging.join("Root").exists());
    }
}
