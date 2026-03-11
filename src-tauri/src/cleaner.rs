//! Game directory cleaner for pre-install preparation.
//!
//! Scans the game data directory against the baseline snapshot to identify
//! non-stock files (leftover mods, loose scripts, textures, etc.) and provides
//! options to clean them before a fresh collection install.
//!
//! The cleaner leverages the existing integrity snapshot system
//! (`game_file_snapshots`) rather than maintaining a hardcoded vanilla file
//! list, making it game-agnostic.

use std::collections::HashSet;
use std::fs;
use std::io;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use log::{info, warn};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use walkdir::WalkDir;

use crate::baselines;
use crate::database::ModDatabase;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum CleanerError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("WalkDir error: {0}")]
    WalkDir(#[from] walkdir::Error),

    #[error("No baseline snapshot exists for {0}/{1}. Run the game once to create a snapshot, then try again.")]
    NoSnapshot(String, String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, CleanerError>;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Report from scanning the game directory for non-stock files.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CleanReport {
    /// Files present on disk that are NOT in the baseline snapshot.
    pub non_stock_files: Vec<NonStockFile>,
    /// Total size of all non-stock files in bytes.
    pub total_size: u64,
    /// Number of files in the baseline snapshot.
    pub snapshot_file_count: usize,
    /// Number of files currently on disk.
    pub disk_file_count: usize,
    /// Files that are tracked in the deployment manifest (managed by Corkscrew).
    pub managed_count: usize,
    /// Files that are NOT tracked — true orphans from manual installs or other tools.
    pub orphaned_count: usize,
    /// ENB-related files detected (d3d11.dll, enbseries/, etc.).
    pub enb_files: Vec<String>,
    /// Save-related files detected (excluded from cleaning by default).
    pub save_files: Vec<String>,
}

/// A single non-stock file with metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NonStockFile {
    /// Path relative to the data directory.
    pub relative_path: String,
    /// File size in bytes.
    pub size: u64,
    /// Whether this file is tracked in the deployment manifest.
    pub is_managed: bool,
    /// File category (plugin, mesh, texture, script, bsa, enb, other).
    pub category: String,
}

/// Options for the clean operation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CleanOptions {
    /// Remove loose mod files (meshes, textures, scripts, plugins).
    pub remove_loose_files: bool,
    /// Remove non-stock BSA/BA2 archives.
    pub remove_archives: bool,
    /// Remove ENB files (d3d11.dll, enbseries/, etc.).
    pub remove_enb: bool,
    /// Remove save files (.ess, .skse cosaves).
    pub remove_saves: bool,
    /// Remove SKSE files (skse64_loader, DLLs, SKSE/Plugins/).
    pub remove_skse: bool,
    /// Only remove unmanaged/orphaned files (skip files tracked in manifest).
    pub orphans_only: bool,
    /// Preview what would be removed without actually deleting.
    pub dry_run: bool,
    /// Glob patterns to exclude from cleaning (e.g., "SKSE/Plugins/*").
    pub exclude_patterns: Vec<String>,
}

impl Default for CleanOptions {
    fn default() -> Self {
        Self {
            remove_loose_files: true,
            remove_archives: true,
            remove_enb: false,
            remove_saves: false,
            remove_skse: false,
            orphans_only: false,
            dry_run: false,
            exclude_patterns: Vec::new(),
        }
    }
}

/// Result of a clean operation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CleanResult {
    /// Files that were removed (or would be removed in dry_run mode).
    pub removed_files: Vec<String>,
    /// Files that were skipped due to exclude patterns or options.
    pub skipped_files: Vec<String>,
    /// Total bytes freed (or that would be freed).
    pub bytes_freed: u64,
    /// Whether this was a dry run.
    pub dry_run: bool,
}

// ---------------------------------------------------------------------------
// ENB / save detection patterns
// ---------------------------------------------------------------------------

/// Critical game files that must NEVER be deleted by the cleaner, regardless of
/// baseline or snapshot state. This is a hard safety rail to prevent catastrophic
/// game directory destruction.
///
/// Patterns are matched case-insensitively against relative paths.
const CRITICAL_FILE_PATTERNS: &[&str] = &[
    // Skyrim SE / AE master files
    "skyrim.esm",
    "update.esm",
    "dawnguard.esm",
    "hearthfires.esm",
    "dragonborn.esm",
    // Fallout 4 master files
    "fallout4.esm",
    "dlcrobot.esm",
    "dlcworkshop01.esm",
    "dlcworkshop02.esm",
    "dlcworkshop03.esm",
    "dlccoast.esm",
    "dlcnukaworld.esm",
];

/// File extensions that should never be deleted from a game Data directory
/// unless they are confirmed mod files (tracked in deployment manifest AND
/// have a staging counterpart).
const PROTECTED_EXTENSIONS: &[&str] = &[".esm", ".bsa", ".ba2"];

/// Returns true if a file is a critical game file that must never be deleted.
fn is_critical_file(rel_path: &str) -> bool {
    let lower = rel_path.to_lowercase();
    // Check exact critical filenames (top-level master files)
    for pattern in CRITICAL_FILE_PATTERNS {
        if lower == *pattern {
            return true;
        }
    }
    // Any .esm/.bsa/.ba2 file at the root level (not in a subdirectory) is
    // likely a vanilla game file and should be protected
    if !lower.contains('/') {
        for ext in PROTECTED_EXTENSIONS {
            if lower.ends_with(ext) {
                return true;
            }
        }
    }
    false
}

/// Known ENB-related files and directories (case-insensitive check).
const ENB_PATTERNS: &[&str] = &[
    "d3d11.dll",
    "d3d9.dll",
    "d3dcompiler_46e.dll",
    "enbseries",
    "enblocal.ini",
    "enbseries.ini",
    "enbadaptation.fx",
    "enbbloom.fx",
    "enbeffect.fx",
    "enbeffectprepass.fx",
    "enblens.fx",
    "enbpalette",
];

/// Save file extensions and directories (case-insensitive).
const SAVE_PATTERNS: &[&str] = &[
    ".ess",   // Skyrim saves
    ".skse",  // SKSE co-saves
    "saves/", // Save directory
];

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Scan the game data directory for non-stock files.
///
/// Compares all files on disk against the baseline snapshot stored in
/// `game_file_snapshots`. Any file not in the snapshot is considered
/// non-stock. Also cross-references the `deployment_manifest` to
/// distinguish managed files from orphans.
pub fn scan_game_directory(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    data_dir: &Path,
) -> Result<CleanReport> {
    let conn = db.conn().map_err(|e| CleanerError::Other(e.to_string()))?;

    // Load snapshot paths into a HashSet for O(1) lookup.
    // Normalize to lowercase for case-insensitive comparison — file systems
    // under Wine/CrossOver may differ in casing from our baseline.
    let mut stmt = conn.prepare(
        "SELECT relative_path FROM game_file_snapshots
         WHERE game_id = ?1 AND bottle_name = ?2",
    )?;
    let snapshot_paths: HashSet<String> = stmt
        .query_map(params![game_id, bottle_name], |row| row.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .map(|p| p.to_lowercase())
        .collect();

    // Fall back to built-in baseline if no user-created snapshot exists
    let using_builtin = snapshot_paths.is_empty();
    let snapshot_paths = if using_builtin {
        match baselines::get_builtin_baseline(game_id) {
            Some(baseline) => {
                info!(
                    "No snapshot for {}/{}; using built-in baseline ({} stock files)",
                    game_id,
                    bottle_name,
                    baseline.len()
                );
                // Lowercase the built-in baseline too for case-insensitive matching
                baseline.into_iter().map(|p| p.to_lowercase()).collect()
            }
            None => {
                return Err(CleanerError::NoSnapshot(
                    game_id.to_string(),
                    bottle_name.to_string(),
                ));
            }
        }
    } else {
        snapshot_paths
    };

    // Load deployment manifest paths into a HashSet
    let mut manifest_stmt = conn.prepare(
        "SELECT relative_path FROM deployment_manifest
         WHERE game_id = ?1 AND bottle_name = ?2",
    )?;
    let managed_paths: HashSet<String> = manifest_stmt
        .query_map(params![game_id, bottle_name], |row| row.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .collect();

    let mut non_stock_files = Vec::new();
    let mut enb_files = Vec::new();
    let mut save_files = Vec::new();
    let mut total_size: u64 = 0;
    let mut disk_file_count = 0usize;
    let mut managed_count = 0usize;
    let mut orphaned_count = 0usize;

    for entry in WalkDir::new(data_dir).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }

        let abs_path = entry.path();
        let relative = match abs_path.strip_prefix(data_dir) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let rel_str = relative.to_string_lossy().replace('\\', "/");
        disk_file_count += 1;

        // SAFETY: Critical game files are NEVER flagged as non-stock,
        // regardless of baseline or snapshot state.
        if is_critical_file(&rel_str) {
            continue;
        }

        // Skip files that are in the baseline snapshot (case-insensitive)
        if snapshot_paths.contains(&rel_str.to_lowercase()) {
            continue;
        }

        // Also check stock patterns (catches CC content, video files, etc.)
        // Apply this check regardless of whether using built-in baseline —
        // stock patterns should always be protected.
        if baselines::is_stock_pattern(game_id, &rel_str) {
            continue;
        }

        let is_save = is_save_file(&rel_str);
        if is_save {
            save_files.push(rel_str.clone());
        }

        let file_size = fs::metadata(abs_path).map(|m| m.len()).unwrap_or(0);
        let is_managed = managed_paths.contains(&rel_str);
        let is_enb = is_enb_file(&rel_str);
        let category = if is_save {
            "save".to_string()
        } else {
            categorize_file(&rel_str)
        };

        if is_enb {
            enb_files.push(rel_str.clone());
        }

        if is_managed {
            managed_count += 1;
        } else {
            orphaned_count += 1;
        }

        total_size += file_size;

        non_stock_files.push(NonStockFile {
            relative_path: rel_str,
            size: file_size,
            is_managed,
            category,
        });
    }

    Ok(CleanReport {
        non_stock_files,
        total_size,
        snapshot_file_count: snapshot_paths.len(),
        disk_file_count,
        managed_count,
        orphaned_count,
        enb_files,
        save_files,
    })
}

/// Clean non-stock files from the game directory based on provided options.
pub fn clean_game_directory(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    data_dir: &Path,
    options: &CleanOptions,
) -> Result<CleanResult> {
    // First, scan to get the full report
    let report = scan_game_directory(db, game_id, bottle_name, data_dir)?;

    let mut removed_files = Vec::new();
    let mut skipped_files = Vec::new();
    let mut bytes_freed: u64 = 0;

    for file in &report.non_stock_files {
        // SAFETY: Double-check critical files even if they somehow made it
        // into the non_stock list. This is the last line of defense.
        if is_critical_file(&file.relative_path) {
            warn!(
                "SAFETY: Refusing to delete critical file: {}",
                file.relative_path
            );
            skipped_files.push(file.relative_path.clone());
            continue;
        }

        // Check exclude patterns
        if matches_exclude_pattern(&file.relative_path, &options.exclude_patterns) {
            skipped_files.push(file.relative_path.clone());
            continue;
        }

        // Check orphans_only filter
        if options.orphans_only && file.is_managed {
            skipped_files.push(file.relative_path.clone());
            continue;
        }

        // Check category filters
        let dominated_by_category = match file.category.as_str() {
            "enb" => !options.remove_enb,
            "save" => !options.remove_saves,
            "skse" => !options.remove_skse,
            "bsa" | "ba2" => !options.remove_archives,
            _ => !options.remove_loose_files,
        };

        if dominated_by_category {
            skipped_files.push(file.relative_path.clone());
            continue;
        }

        // This file should be removed
        let abs_path = data_dir.join(&file.relative_path);

        if options.dry_run {
            removed_files.push(file.relative_path.clone());
            bytes_freed += file.size;
        } else if abs_path.exists() {
            // Make file writable before deleting — some mod files are read-only
            if let Ok(metadata) = fs::metadata(&abs_path) {
                let perms = metadata.permissions();
                if perms.readonly() {
                    #[cfg(unix)]
                    {
                        let mut writable = perms;
                        writable.set_mode(0o644);
                        let _ = fs::set_permissions(&abs_path, writable);
                    }
                    #[cfg(not(unix))]
                    {
                        let mut writable = perms;
                        writable.set_readonly(false);
                        let _ = fs::set_permissions(&abs_path, writable);
                    }
                }
            }
            // Also make parent directory writable — deletion requires write on parent
            if let Some(parent) = abs_path.parent() {
                if let Ok(dir_meta) = fs::metadata(parent) {
                    let dir_perms = dir_meta.permissions();
                    if dir_perms.readonly() {
                        #[cfg(unix)]
                        {
                            let mut writable = dir_perms;
                            writable.set_mode(0o755);
                            let _ = fs::set_permissions(parent, writable);
                        }
                        #[cfg(not(unix))]
                        {
                            let mut writable = dir_perms;
                            writable.set_readonly(false);
                            let _ = fs::set_permissions(parent, writable);
                        }
                    }
                }
            }
            match fs::remove_file(&abs_path) {
                Ok(()) => {
                    removed_files.push(file.relative_path.clone());
                    bytes_freed += file.size;
                    // Prune empty parent directories
                    prune_empty_dirs(&abs_path, data_dir);
                }
                Err(e) => {
                    warn!("Failed to remove {}: {}", abs_path.display(), e);
                    skipped_files.push(file.relative_path.clone());
                }
            }
        } else {
            // File was in scan but doesn't exist at constructed path
            warn!(
                "File from scan not found at constructed path: {}",
                abs_path.display()
            );
            skipped_files.push(file.relative_path.clone());
        }
    }

    // If not dry_run and we removed managed files, also clear their manifest entries
    if !options.dry_run && !options.orphans_only {
        // Clear deployment manifest for this game/bottle since we're cleaning everything
        let conn = db.conn().map_err(|e| CleanerError::Other(e.to_string()))?;
        conn.execute(
            "DELETE FROM deployment_manifest WHERE game_id = ?1 AND bottle_name = ?2",
            params![game_id, bottle_name],
        )?;

        // Also clear installed_files arrays in installed_mods and disable mods
        conn.execute(
            "UPDATE installed_mods SET enabled = 0 WHERE game_id = ?1 AND bottle_name = ?2",
            params![game_id, bottle_name],
        )?;
    }

    if !options.dry_run {
        info!(
            "Cleaned game directory for {}/{}: {} files removed ({} bytes freed), {} skipped",
            game_id,
            bottle_name,
            removed_files.len(),
            bytes_freed,
            skipped_files.len()
        );
    }

    Ok(CleanResult {
        removed_files,
        skipped_files,
        bytes_freed,
        dry_run: options.dry_run,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Categorize a file based on its extension/path.
pub fn categorize_file(rel_path: &str) -> String {
    let lower = rel_path.to_lowercase();

    if is_enb_file(rel_path) {
        return "enb".to_string();
    }

    // BSA/BA2 archives
    if lower.ends_with(".bsa") || lower.ends_with(".ba2") {
        return "bsa".to_string();
    }

    // Plugin files
    if lower.ends_with(".esp") || lower.ends_with(".esm") || lower.ends_with(".esl") {
        return "plugin".to_string();
    }

    // Meshes
    if lower.contains("meshes/") || lower.ends_with(".nif") {
        return "mesh".to_string();
    }

    // Textures
    if lower.contains("textures/") || lower.ends_with(".dds") {
        return "texture".to_string();
    }

    // Scripts
    if lower.contains("scripts/") || lower.ends_with(".pex") || lower.ends_with(".psc") {
        return "script".to_string();
    }

    // Sound/music
    if lower.contains("sound/")
        || lower.contains("music/")
        || lower.ends_with(".wav")
        || lower.ends_with(".xwm")
        || lower.ends_with(".fuz")
    {
        return "sound".to_string();
    }

    // Interface/UI
    if lower.contains("interface/") || lower.ends_with(".swf") {
        return "interface".to_string();
    }

    // SKSE plugins
    if lower.contains("skse/") || lower.ends_with(".dll") {
        return "skse".to_string();
    }

    "other".to_string()
}

/// Check if a file is ENB-related (case-insensitive).
fn is_enb_file(rel_path: &str) -> bool {
    let lower = rel_path.to_lowercase();
    for pattern in ENB_PATTERNS {
        if lower.starts_with(pattern) || lower.contains(&format!("/{}", pattern)) {
            return true;
        }
    }
    false
}

/// Check if a file is a save file.
fn is_save_file(rel_path: &str) -> bool {
    let lower = rel_path.to_lowercase();
    for pattern in SAVE_PATTERNS {
        if pattern.ends_with('/') {
            if lower.starts_with(pattern) || lower.contains(&format!("/{}", pattern)) {
                return true;
            }
        } else if lower.ends_with(pattern) {
            return true;
        }
    }
    false
}

/// Check if a file matches any exclude pattern.
/// Supports simple glob-like matching: * matches any sequence of non-/ characters.
fn matches_exclude_pattern(rel_path: &str, patterns: &[String]) -> bool {
    let lower = rel_path.to_lowercase();
    for pattern in patterns {
        let pat_lower = pattern.to_lowercase().replace('\\', "/");
        if simple_glob_match(&pat_lower, &lower) {
            return true;
        }
    }
    false
}

/// Simple glob matcher supporting * as wildcard for any sequence of characters.
fn simple_glob_match(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        // No wildcards — exact match or prefix
        return text == pattern || text.starts_with(pattern);
    }

    let mut pos = 0;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        match text[pos..].find(part) {
            Some(found) => {
                // First part must match at start
                if i == 0 && found != 0 {
                    return false;
                }
                pos += found + part.len();
            }
            None => return false,
        }
    }

    // Last part must match at end if pattern doesn't end with *
    if !pattern.ends_with('*') {
        if let Some(last_part) = parts.last() {
            if !last_part.is_empty() {
                return text.ends_with(last_part);
            }
        }
    }

    true
}

/// Walk up from a removed file and prune empty directories up to (not including)
/// `stop_at`.
fn prune_empty_dirs(removed_file: &Path, stop_at: &Path) {
    let mut current = removed_file.parent().map(|p| p.to_path_buf());
    while let Some(dir) = current {
        if dir == stop_at || !dir.starts_with(stop_at) {
            break;
        }
        // Try to remove — will only succeed if empty
        if fs::remove_dir(&dir).is_err() {
            break;
        }
        current = dir.parent().map(|p| p.to_path_buf());
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrity;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn test_db() -> (ModDatabase, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let db = ModDatabase::new(&db_path).unwrap();
        integrity::init_schema(&db).unwrap();
        (db, tmp)
    }

    #[test]
    fn scan_identifies_non_stock_files() {
        let (db, tmp) = test_db();
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(data_dir.join("meshes")).unwrap();

        // Stock files
        fs::write(data_dir.join("Skyrim.esm"), b"master").unwrap();
        fs::write(data_dir.join("meshes/vanilla.nif"), b"mesh").unwrap();

        // Create baseline snapshot
        integrity::create_game_snapshot(&db, "skyrimse", "Gaming", &data_dir).unwrap();

        // Add non-stock files
        fs::write(data_dir.join("mod.esp"), b"mod plugin").unwrap();
        fs::write(data_dir.join("meshes/modded.nif"), b"modded mesh").unwrap();

        let report = scan_game_directory(&db, "skyrimse", "Gaming", &data_dir).unwrap();
        assert_eq!(report.non_stock_files.len(), 2);
        assert_eq!(report.snapshot_file_count, 2);
        assert_eq!(report.disk_file_count, 4);
        assert_eq!(report.orphaned_count, 2);
        assert!(report.total_size > 0);
    }

    #[test]
    fn scan_detects_enb_files() {
        let (db, tmp) = test_db();
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();

        fs::write(data_dir.join("Skyrim.esm"), b"master").unwrap();
        integrity::create_game_snapshot(&db, "skyrimse", "Gaming", &data_dir).unwrap();

        // Add ENB files
        fs::write(data_dir.join("d3d11.dll"), b"enb").unwrap();
        fs::create_dir_all(data_dir.join("enbseries")).unwrap();
        fs::write(data_dir.join("enbseries/effect.fx"), b"fx").unwrap();

        let report = scan_game_directory(&db, "skyrimse", "Gaming", &data_dir).unwrap();
        assert_eq!(report.enb_files.len(), 2);
    }

    #[test]
    fn scan_reports_save_files() {
        let (db, tmp) = test_db();
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();

        fs::write(data_dir.join("Skyrim.esm"), b"master").unwrap();
        integrity::create_game_snapshot(&db, "skyrimse", "Gaming", &data_dir).unwrap();

        // Add save files
        fs::write(data_dir.join("quicksave.ess"), b"save").unwrap();
        fs::write(data_dir.join("quicksave.skse"), b"cosave").unwrap();

        // Add a mod file
        fs::write(data_dir.join("mod.esp"), b"mod").unwrap();

        let report = scan_game_directory(&db, "skyrimse", "Gaming", &data_dir).unwrap();
        assert_eq!(report.save_files.len(), 2);
        // Saves are now included in non_stock_files (category "save") for opt-in removal
        assert_eq!(report.non_stock_files.len(), 3); // mod.esp + 2 saves
        let save_count = report
            .non_stock_files
            .iter()
            .filter(|f| f.category == "save")
            .count();
        assert_eq!(save_count, 2);
    }

    #[test]
    fn clean_dry_run_removes_nothing() {
        let (db, tmp) = test_db();
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();

        fs::write(data_dir.join("Skyrim.esm"), b"master").unwrap();
        integrity::create_game_snapshot(&db, "skyrimse", "Gaming", &data_dir).unwrap();

        fs::write(data_dir.join("mod.esp"), b"mod").unwrap();

        let options = CleanOptions {
            dry_run: true,
            ..Default::default()
        };

        let result = clean_game_directory(&db, "skyrimse", "Gaming", &data_dir, &options).unwrap();
        assert_eq!(result.removed_files.len(), 1);
        assert!(result.dry_run);

        // File should still exist
        assert!(data_dir.join("mod.esp").exists());
    }

    #[test]
    fn clean_removes_non_stock_files() {
        let (db, tmp) = test_db();
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(data_dir.join("meshes")).unwrap();

        fs::write(data_dir.join("Skyrim.esm"), b"master").unwrap();
        integrity::create_game_snapshot(&db, "skyrimse", "Gaming", &data_dir).unwrap();

        fs::write(data_dir.join("mod.esp"), b"mod plugin").unwrap();
        fs::write(data_dir.join("meshes/modded.nif"), b"modded").unwrap();

        let options = CleanOptions::default();
        let result = clean_game_directory(&db, "skyrimse", "Gaming", &data_dir, &options).unwrap();

        assert_eq!(result.removed_files.len(), 2);
        assert!(!result.dry_run);
        assert!(!data_dir.join("mod.esp").exists());
        assert!(!data_dir.join("meshes/modded.nif").exists());
        // Stock file should remain
        assert!(data_dir.join("Skyrim.esm").exists());
    }

    #[test]
    fn clean_respects_exclude_patterns() {
        let (db, tmp) = test_db();
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(data_dir.join("MyMod/Config")).unwrap();

        fs::write(data_dir.join("Skyrim.esm"), b"master").unwrap();
        integrity::create_game_snapshot(&db, "skyrimse", "Gaming", &data_dir).unwrap();

        fs::write(data_dir.join("mod.esp"), b"mod").unwrap();
        fs::write(data_dir.join("MyMod/Config/important.ini"), b"keep this").unwrap();

        let options = CleanOptions {
            exclude_patterns: vec!["MyMod/Config/*".to_string()],
            ..Default::default()
        };

        let result = clean_game_directory(&db, "skyrimse", "Gaming", &data_dir, &options).unwrap();

        assert_eq!(result.removed_files.len(), 1); // Only mod.esp
        assert_eq!(result.skipped_files.len(), 1); // Excluded config
        assert!(data_dir.join("MyMod/Config/important.ini").exists());
    }

    #[test]
    fn clean_skips_enb_by_default() {
        let (db, tmp) = test_db();
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();

        fs::write(data_dir.join("Skyrim.esm"), b"master").unwrap();
        integrity::create_game_snapshot(&db, "skyrimse", "Gaming", &data_dir).unwrap();

        fs::write(data_dir.join("d3d11.dll"), b"enb").unwrap();
        fs::write(data_dir.join("mod.esp"), b"mod").unwrap();

        let options = CleanOptions::default(); // remove_enb = false
        let result = clean_game_directory(&db, "skyrimse", "Gaming", &data_dir, &options).unwrap();

        assert_eq!(result.removed_files.len(), 1); // Only mod.esp
        assert!(data_dir.join("d3d11.dll").exists()); // ENB preserved
    }

    #[test]
    fn no_snapshot_returns_error() {
        let (db, _tmp) = test_db();
        let data_dir = PathBuf::from("/nonexistent");

        // Use an unknown game ID so there's no built-in baseline to fall back on
        let result = scan_game_directory(&db, "unknowngame", "Gaming", &data_dir);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("No baseline snapshot"));
    }

    #[test]
    fn skyrim_uses_builtin_baseline_when_no_snapshot() {
        let (db, tmp) = test_db();
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();

        // Stock file — should NOT appear in report
        fs::write(data_dir.join("Skyrim.esm"), b"master").unwrap();
        // Non-stock file — should appear
        fs::write(data_dir.join("mod.esp"), b"mod").unwrap();

        // No snapshot created — should fall back to built-in baseline
        let report = scan_game_directory(&db, "skyrimse", "Gaming", &data_dir).unwrap();
        assert_eq!(report.non_stock_files.len(), 1);
        assert_eq!(report.non_stock_files[0].relative_path, "mod.esp");
    }

    #[test]
    fn categorize_file_works() {
        assert_eq!(categorize_file("mod.esp"), "plugin");
        assert_eq!(categorize_file("mod.esm"), "plugin");
        assert_eq!(categorize_file("meshes/armor.nif"), "mesh");
        assert_eq!(categorize_file("textures/body.dds"), "texture");
        assert_eq!(categorize_file("scripts/main.pex"), "script");
        assert_eq!(categorize_file("d3d11.dll"), "enb");
        assert_eq!(categorize_file("mod.bsa"), "bsa");
        assert_eq!(categorize_file("readme.txt"), "other");
    }

    #[test]
    fn glob_matching_works() {
        assert!(simple_glob_match("SKSE/Plugins/*", "SKSE/Plugins/test.dll"));
        assert!(simple_glob_match("*.esp", "mod.esp"));
        assert!(!simple_glob_match("*.esp", "mod.esm"));
        assert!(simple_glob_match("meshes/*", "meshes/armor.nif"));
        assert!(!simple_glob_match("meshes/*", "textures/body.dds"));
    }

    #[test]
    fn critical_file_detection() {
        // Master ESM files are always critical
        assert!(is_critical_file("Skyrim.esm"));
        assert!(is_critical_file("skyrim.esm")); // case-insensitive
        assert!(is_critical_file("SKYRIM.ESM")); // all caps
        assert!(is_critical_file("Update.esm"));
        assert!(is_critical_file("Dawnguard.esm"));
        assert!(is_critical_file("Dragonborn.esm"));
        assert!(is_critical_file("HearthFires.esm"));
        assert!(is_critical_file("Fallout4.esm"));

        // Top-level .esm/.bsa/.ba2 files are protected
        assert!(is_critical_file("SomeOther.esm"));
        assert!(is_critical_file("Skyrim - Textures0.bsa"));
        assert!(is_critical_file("Dawnguard.bsa"));

        // Subdirectory .esm files are NOT protected (mod-specific)
        assert!(!is_critical_file("mods/something.esm"));

        // Regular mod files are not critical
        assert!(!is_critical_file("mod.esp"));
        assert!(!is_critical_file("textures/something.dds"));
        assert!(!is_critical_file("meshes/armor.nif"));
    }

    #[test]
    fn cleaner_never_deletes_critical_files() {
        let (db, tmp) = test_db();
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();

        // Write both stock and non-stock files
        fs::write(data_dir.join("Skyrim.esm"), b"master").unwrap();
        fs::write(data_dir.join("Update.esm"), b"update").unwrap();
        fs::write(data_dir.join("Skyrim - Textures0.bsa"), b"textures").unwrap();
        fs::write(data_dir.join("mod.esp"), b"mod").unwrap();
        fs::write(data_dir.join("modname.bsa"), b"mod archive").unwrap();

        // Use an unknown game ID with no baseline — critical file protection
        // should still prevent deletion of .esm and .bsa files
        integrity::create_game_snapshot(&db, "testgame", "Gaming", &data_dir).unwrap();

        // Now add the "non-stock" files after snapshot
        fs::write(data_dir.join("extra.esp"), b"extra").unwrap();

        // Even with no baseline, master files must survive
        let options = CleanOptions::default();
        let result = clean_game_directory(&db, "testgame", "Gaming", &data_dir, &options).unwrap();

        // extra.esp should be removed (it was added after snapshot)
        assert!(result.removed_files.contains(&"extra.esp".to_string()));
        // Stock files must still exist
        assert!(data_dir.join("Skyrim.esm").exists());
        assert!(data_dir.join("Update.esm").exists());
        assert!(data_dir.join("Skyrim - Textures0.bsa").exists());
    }

    #[test]
    fn case_insensitive_baseline_matching() {
        let (db, tmp) = test_db();
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();

        // Write stock file with different casing
        fs::write(data_dir.join("skyrim.esm"), b"master").unwrap();
        fs::write(data_dir.join("mod.esp"), b"mod").unwrap();

        // No snapshot — falls back to built-in baseline which has "Skyrim.esm"
        let report = scan_game_directory(&db, "skyrimse", "Gaming", &data_dir).unwrap();

        // skyrim.esm should NOT appear (case-insensitive match with baseline)
        let paths: Vec<&str> = report
            .non_stock_files
            .iter()
            .map(|f| f.relative_path.as_str())
            .collect();
        assert!(
            !paths.contains(&"skyrim.esm"),
            "skyrim.esm should be recognized as stock"
        );
        assert!(paths.contains(&"mod.esp"), "mod.esp should be non-stock");
    }
}
