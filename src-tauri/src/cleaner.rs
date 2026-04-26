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
    /// Remove save files (game-specific patterns: .ess/.skse for Skyrim, .sav for HL, etc.).
    pub remove_saves: bool,
    /// Remove script extender / framework files (DLLs, SKSE/Plugins/, etc.).
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

/// Returns true if a file is a critical game file that must never be deleted.
///
/// Queries the game's plugin for critical file names and protected root
/// extensions. Falls back to a built-in Bethesda list when no plugin is
/// registered (safety net for generic registry games).
fn is_critical_file(game_id: &str, rel_path: &str) -> bool {
    let lower = rel_path.to_lowercase();

    // Query the game plugin for critical files and protected extensions.
    // Convert to owned Strings to avoid lifetime issues with the plugin lock.
    let (critical, protected_ext): (Vec<String>, Vec<String>) =
        crate::games::with_plugin(game_id, |p| {
            (
                p.critical_files().into_iter().map(|s| s.to_string()).collect(),
                p.protected_root_extensions()
                    .into_iter()
                    .map(|s| s.to_string())
                    .collect(),
            )
        })
        .unwrap_or_else(|| {
            // Fallback for games with no dedicated plugin: use legacy Bethesda
            // protection as a safe default since many registry games are Bethesda.
            (
                vec![
                    "skyrim.esm",
                    "update.esm",
                    "dawnguard.esm",
                    "hearthfires.esm",
                    "dragonborn.esm",
                    "fallout4.esm",
                    "dlcrobot.esm",
                    "dlcworkshop01.esm",
                    "dlcworkshop02.esm",
                    "dlcworkshop03.esm",
                    "dlccoast.esm",
                    "dlcnukaworld.esm",
                ]
                .into_iter()
                .map(|s| s.to_string())
                .collect(),
                vec![".esm", ".bsa", ".ba2"]
                    .into_iter()
                    .map(|s| s.to_string())
                    .collect(),
            )
        });

    // Check exact critical filenames (top-level master files)
    for pattern in &critical {
        if lower == pattern.as_str() {
            return true;
        }
    }

    // Root-level files with protected extensions
    if !lower.contains('/') {
        for ext in &protected_ext {
            if lower.ends_with(ext.as_str()) {
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

/// Returns save file patterns for a game. Queries the game plugin first,
/// falls back to common Bethesda save patterns for unregistered games.
fn save_patterns_for_game(game_id: &str) -> Vec<String> {
    let fallback = || vec![".ess".into(), ".skse".into(), "saves/".into()];
    crate::games::with_plugin(game_id, |p| {
        let patterns = p.save_file_patterns();
        if patterns.is_empty() {
            fallback()
        } else {
            patterns.into_iter().map(|s| s.to_string()).collect()
        }
    })
    .unwrap_or_else(fallback)
}

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

    // Load deployment manifest paths into a HashSet, lowercased so the
    // comparison against on-disk paths is case-insensitive. Linux ext4 is
    // case-sensitive, but Wine's NTFS/APFS-target case-insensitive mod
    // sources mean the manifest and the disk can disagree on case and
    // produce phantom "orphan" files.
    let mut manifest_stmt = conn.prepare(
        "SELECT relative_path FROM deployment_manifest
         WHERE game_id = ?1 AND bottle_name = ?2",
    )?;
    let managed_paths: HashSet<String> = manifest_stmt
        .query_map(params![game_id, bottle_name], |row| row.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .map(|p| p.to_lowercase())
        .collect();

    // Load game-specific save patterns once before the scan loop.
    let game_save_patterns = save_patterns_for_game(game_id);

    let mut non_stock_files = Vec::new();
    let mut enb_files = Vec::new();
    let mut save_files = Vec::new();
    let mut total_size: u64 = 0;
    let mut disk_file_count = 0usize;
    let mut managed_count = 0usize;
    let mut orphaned_count = 0usize;

    for entry in WalkDir::new(data_dir).into_iter().filter_map(|e| e.ok()) {
        let file_type = entry.file_type();

        // WalkDir defaults to follow_links=false, which yields symlink entries
        // but reports file_type().is_file() == false for them. On Steam Deck
        // microSD setups, mod files presented via symlinks would be silently
        // skipped. Treat symlinks pointing to files as files for the purposes
        // of orphan detection — log them so users understand what's scanned.
        let is_symlink_to_file = if file_type.is_symlink() {
            match fs::metadata(entry.path()) {
                Ok(meta) => meta.is_file(),
                Err(e) => {
                    warn!(
                        "cleaner: skipping unreadable symlink {}: {}",
                        entry.path().display(),
                        e
                    );
                    false
                }
            }
        } else {
            false
        };

        if !file_type.is_file() && !is_symlink_to_file {
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
        if is_critical_file(game_id, &rel_str) {
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

        let is_save = is_save_file_with_patterns(&rel_str, &game_save_patterns);
        if is_save {
            save_files.push(rel_str.clone());
        }

        let file_size = fs::metadata(abs_path).map(|m| m.len()).unwrap_or(0);
        // Case-insensitive match (managed_paths was lowercased on load).
        let is_managed = managed_paths.contains(&rel_str.to_lowercase());
        let is_enb = is_enb_file(&rel_str);
        let category = if is_save {
            "save".to_string()
        } else {
            categorize_file(game_id, &rel_str)
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
        if is_critical_file(game_id, &file.relative_path) {
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
            "skse" | "framework" => !options.remove_skse,
            "bsa" | "ba2" | "pak" => !options.remove_archives,
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
///
/// Queries the game plugin first for game-specific categorization, then
/// falls back to generic heuristics.
pub fn categorize_file(game_id: &str, rel_path: &str) -> String {
    // ENB detection is universal across all games
    if is_enb_file(rel_path) {
        return "enb".to_string();
    }

    // Ask the game plugin for a game-specific category
    if let Some(cat) = crate::games::with_plugin(game_id, |p| p.categorize_mod_file(rel_path)) {
        if let Some(category) = cat {
            return category;
        }
    }

    // Generic fallback for games without a dedicated plugin
    let lower = rel_path.to_lowercase();

    if lower.ends_with(".bsa") || lower.ends_with(".ba2") {
        return "bsa".to_string();
    }
    if lower.ends_with(".esp") || lower.ends_with(".esm") || lower.ends_with(".esl") {
        return "plugin".to_string();
    }
    if lower.ends_with(".pak") || lower.ends_with(".ucas") || lower.ends_with(".utoc") {
        return "pak".to_string();
    }
    if lower.ends_with(".dll") {
        return "framework".to_string();
    }
    if lower.contains("meshes/") || lower.ends_with(".nif") {
        return "mesh".to_string();
    }
    if lower.contains("textures/") || lower.ends_with(".dds") {
        return "texture".to_string();
    }
    if lower.contains("scripts/") || lower.ends_with(".pex") || lower.ends_with(".psc") {
        return "script".to_string();
    }
    if lower.contains("sound/")
        || lower.contains("music/")
        || lower.ends_with(".wav")
        || lower.ends_with(".xwm")
        || lower.ends_with(".fuz")
    {
        return "sound".to_string();
    }
    if lower.contains("interface/") || lower.ends_with(".swf") {
        return "interface".to_string();
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

/// Check if a file matches any of the provided save patterns.
fn is_save_file_with_patterns(rel_path: &str, patterns: &[String]) -> bool {
    let lower = rel_path.to_lowercase();
    for pattern in patterns {
        if pattern.ends_with('/') {
            if lower.starts_with(pattern.as_str())
                || lower.contains(&format!("/{}", pattern))
            {
                return true;
            }
        } else if lower.ends_with(pattern.as_str()) {
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
        // Generic fallback (no plugin registered for "testgame")
        assert_eq!(categorize_file("testgame", "mod.esp"), "plugin");
        assert_eq!(categorize_file("testgame", "mod.esm"), "plugin");
        assert_eq!(categorize_file("testgame", "meshes/armor.nif"), "mesh");
        assert_eq!(categorize_file("testgame", "textures/body.dds"), "texture");
        assert_eq!(categorize_file("testgame", "scripts/main.pex"), "script");
        assert_eq!(categorize_file("testgame", "d3d11.dll"), "enb");
        assert_eq!(categorize_file("testgame", "mod.bsa"), "bsa");
        assert_eq!(categorize_file("testgame", "readme.txt"), "other");
        // Generic fallback handles UE PAK files
        assert_eq!(categorize_file("testgame", "MyMod_P.pak"), "pak");
        // Generic fallback handles DLLs as framework (when not ENB)
        assert_eq!(categorize_file("testgame", "mods/plugin.dll"), "framework");
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
        // Use a non-registered game ID so we get the legacy Bethesda fallback
        let gid = "unknowngame_crit";

        // Master ESM files are always critical (via fallback)
        assert!(is_critical_file(gid, "Skyrim.esm"));
        assert!(is_critical_file(gid, "skyrim.esm")); // case-insensitive
        assert!(is_critical_file(gid, "SKYRIM.ESM")); // all caps
        assert!(is_critical_file(gid, "Update.esm"));
        assert!(is_critical_file(gid, "Dawnguard.esm"));
        assert!(is_critical_file(gid, "Dragonborn.esm"));
        assert!(is_critical_file(gid, "HearthFires.esm"));
        assert!(is_critical_file(gid, "Fallout4.esm"));

        // Top-level .esm/.bsa/.ba2 files are protected (via fallback)
        assert!(is_critical_file(gid, "SomeOther.esm"));
        assert!(is_critical_file(gid, "Skyrim - Textures0.bsa"));
        assert!(is_critical_file(gid, "Dawnguard.bsa"));

        // Subdirectory .esm files are NOT protected (mod-specific)
        assert!(!is_critical_file(gid, "mods/something.esm"));

        // Regular mod files are not critical
        assert!(!is_critical_file(gid, "mod.esp"));
        assert!(!is_critical_file(gid, "textures/something.dds"));
        assert!(!is_critical_file(gid, "meshes/armor.nif"));
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

    #[test]
    fn hogwarts_legacy_categorization() {
        // Register the Hogwarts Legacy plugin for this test
        crate::plugins::hogwarts_legacy::register();

        assert_eq!(
            categorize_file("hogwartslegacy", "MyMod_P.pak"),
            "pak"
        );
        assert_eq!(
            categorize_file("hogwartslegacy", "Scripts/main.lua"),
            "script"
        );
        assert_eq!(
            categorize_file("hogwartslegacy", "intro.bk2"),
            "movie"
        );
        assert_eq!(
            categorize_file("hogwartslegacy", "ue4ss.dll"),
            // ENB check runs first and d3d11.dll matches, but ue4ss.dll doesn't
            "framework"
        );
        assert_eq!(
            categorize_file("hogwartslegacy", "settings.ini"),
            "config"
        );
    }

    #[test]
    fn hogwarts_legacy_cleaner_scan() {
        crate::plugins::hogwarts_legacy::register();
        let (db, tmp) = test_db();
        let data_dir = tmp.path().join("mods");
        fs::create_dir_all(&data_dir).unwrap();

        // Simulate vanilla state with a placeholder file (the ~mods dir would
        // normally be empty, but the snapshot needs at least one file to
        // distinguish "snapshot exists with 0 stock files" from "no snapshot").
        fs::write(data_dir.join(".gitkeep"), b"").unwrap();
        integrity::create_game_snapshot(&db, "hogwartslegacy", "Gaming", &data_dir).unwrap();

        // Add mod files after snapshot
        fs::write(data_dir.join("CoolMod_P.pak"), b"pak mod").unwrap();
        fs::write(data_dir.join("config.ini"), b"settings").unwrap();

        let report =
            scan_game_directory(&db, "hogwartslegacy", "Gaming", &data_dir).unwrap();

        assert_eq!(report.non_stock_files.len(), 2);
        let cats: Vec<&str> = report
            .non_stock_files
            .iter()
            .map(|f| f.category.as_str())
            .collect();
        assert!(cats.contains(&"pak"), "PAK files should be categorized as pak");
        assert!(
            cats.contains(&"config"),
            "INI files should be categorized as config"
        );
    }

    #[cfg(unix)]
    #[test]
    fn scan_includes_symlinked_files() {
        use std::os::unix::fs::symlink;

        let (db, tmp) = test_db();
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();

        // Stock file → snapshot it
        fs::write(data_dir.join("Skyrim.esm"), b"master").unwrap();
        integrity::create_game_snapshot(&db, "skyrimse", "Gaming", &data_dir).unwrap();

        // External file presented via symlink (Steam Deck microSD pattern).
        let external = tmp.path().join("microsd");
        fs::create_dir_all(&external).unwrap();
        fs::write(external.join("mod.esp"), b"mod").unwrap();
        symlink(external.join("mod.esp"), data_dir.join("mod.esp")).unwrap();

        let report = scan_game_directory(&db, "skyrimse", "Gaming", &data_dir).unwrap();
        // Symlinked mod.esp should be flagged as orphaned/non-stock,
        // not silently skipped.
        assert!(
            report.non_stock_files.iter().any(|f| f.relative_path == "mod.esp"),
            "symlinked file should be detected as non-stock; got {:?}",
            report.non_stock_files.iter().map(|f| &f.relative_path).collect::<Vec<_>>()
        );
    }

    #[test]
    fn hogwarts_legacy_no_false_critical_protection() {
        crate::plugins::hogwarts_legacy::register();

        // HL has no critical files — nothing should be falsely protected
        assert!(!is_critical_file("hogwartslegacy", "CoolMod_P.pak"));
        assert!(!is_critical_file("hogwartslegacy", "ue4ss.dll"));
        assert!(!is_critical_file("hogwartslegacy", "settings.ini"));
        // Skyrim ESMs should NOT be protected in HL context
        assert!(!is_critical_file("hogwartslegacy", "Skyrim.esm"));
    }
}
