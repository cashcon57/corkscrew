//! Game file integrity detection.
//!
//! Creates snapshots of the game's data directory (vanilla files) so that
//! Corkscrew can detect modified/unknown/missing files. This helps users
//! determine if their game installation is clean before modding.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;

use log::info;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use walkdir::WalkDir;

use crate::database::ModDatabase;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum IntegrityError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("WalkDir error: {0}")]
    WalkDir(#[from] walkdir::Error),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, IntegrityError>;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntegrityReport {
    /// Files that existed in the snapshot but have a different hash now.
    pub modified_files: Vec<String>,
    /// Files present on disk that are not in the snapshot (likely mod files).
    pub unknown_files: Vec<String>,
    /// Files in the snapshot that no longer exist on disk.
    pub missing_files: Vec<String>,
    /// Total files scanned.
    pub total_scanned: usize,
}

// ---------------------------------------------------------------------------
// Schema initialization
// ---------------------------------------------------------------------------

pub fn init_schema(db: &ModDatabase) -> std::result::Result<(), String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    init_schema_with_conn(&conn).map_err(|e| e.to_string())
}

fn init_schema_with_conn(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS game_file_snapshots (
            game_id       TEXT NOT NULL,
            bottle_name   TEXT NOT NULL,
            relative_path TEXT NOT NULL,
            sha256        TEXT NOT NULL,
            file_size     INTEGER NOT NULL,
            UNIQUE(game_id, bottle_name, relative_path)
        );",
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Snapshot creation
// ---------------------------------------------------------------------------

/// Create a snapshot of the game's data directory.
///
/// This scans all files under `data_dir`, computes their SHA-256 hashes,
/// and stores them in the database. Existing snapshot entries for this
/// game/bottle are replaced.
pub fn create_game_snapshot(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    data_dir: &Path,
) -> Result<usize> {
    let conn = db.conn().map_err(|e| IntegrityError::Other(e.to_string()))?;

    // Clear existing snapshot
    conn.execute(
        "DELETE FROM game_file_snapshots WHERE game_id = ?1 AND bottle_name = ?2",
        params![game_id, bottle_name],
    )?;

    let mut count = 0usize;
    let mut stmt = conn.prepare(
        "INSERT INTO game_file_snapshots (game_id, bottle_name, relative_path, sha256, file_size)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;

    for entry in WalkDir::new(data_dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }

        let abs_path = entry.path();
        let relative = match abs_path.strip_prefix(data_dir) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let rel_str = relative.to_string_lossy().replace('\\', "/");
        let hash = compute_sha256(abs_path)?;
        let file_size = fs::metadata(abs_path)?.len();

        stmt.execute(params![
            game_id,
            bottle_name,
            rel_str,
            hash,
            file_size as i64,
        ])?;

        count += 1;
    }

    info!(
        "Created game snapshot for {}/{}: {} files",
        game_id, bottle_name, count
    );

    Ok(count)
}

/// Check game file integrity against a saved snapshot.
pub fn check_game_integrity(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    data_dir: &Path,
) -> Result<IntegrityReport> {
    let conn = db.conn().map_err(|e| IntegrityError::Other(e.to_string()))?;

    // Load snapshot into a map
    let mut stmt = conn.prepare(
        "SELECT relative_path, sha256 FROM game_file_snapshots
         WHERE game_id = ?1 AND bottle_name = ?2",
    )?;

    let mut snapshot: HashMap<String, String> = HashMap::new();
    let rows = stmt.query_map(params![game_id, bottle_name], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    for row in rows {
        let (path, hash) = row?;
        snapshot.insert(path, hash);
    }

    if snapshot.is_empty() {
        return Err(IntegrityError::Other(
            "No snapshot exists for this game. Create one first.".to_string(),
        ));
    }

    let mut modified = Vec::new();
    let mut unknown = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut total_scanned = 0usize;

    for entry in WalkDir::new(data_dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }

        let abs_path = entry.path();
        let relative = match abs_path.strip_prefix(data_dir) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let rel_str = relative.to_string_lossy().replace('\\', "/");
        total_scanned += 1;
        seen.insert(rel_str.clone());

        match snapshot.get(&rel_str) {
            Some(expected_hash) => {
                let actual_hash = compute_sha256(abs_path)?;
                if actual_hash != *expected_hash {
                    modified.push(rel_str);
                }
            }
            None => {
                unknown.push(rel_str);
            }
        }
    }

    // Find missing files (in snapshot but not on disk)
    let missing: Vec<String> = snapshot
        .keys()
        .filter(|k| !seen.contains(*k))
        .cloned()
        .collect();

    Ok(IntegrityReport {
        modified_files: modified,
        unknown_files: unknown,
        missing_files: missing,
        total_scanned,
    })
}

/// Check if a snapshot exists for a game/bottle.
pub fn has_snapshot(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
) -> Result<bool> {
    let conn = db.conn().map_err(|e| IntegrityError::Other(e.to_string()))?;
    let count: i64 = conn
        .prepare("SELECT count(*) FROM game_file_snapshots WHERE game_id = ?1 AND bottle_name = ?2")?
        .query_row(params![game_id, bottle_name], |row| row.get(0))?;
    Ok(count > 0)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn compute_sha256(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)?;
    let result = hasher.finalize();
    Ok(format!("{:x}", result))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_db() -> (ModDatabase, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let db = ModDatabase::new(&db_path).unwrap();
        init_schema(&db).unwrap();
        (db, tmp)
    }

    #[test]
    fn create_and_check_snapshot() {
        let (db, tmp) = test_db();

        // Create a fake game data dir
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(data_dir.join("meshes")).unwrap();
        fs::write(data_dir.join("meshes/test.nif"), b"mesh data").unwrap();
        fs::write(data_dir.join("test.esp"), b"plugin data").unwrap();

        // Create snapshot
        let count = create_game_snapshot(&db, "skyrimse", "Gaming", &data_dir).unwrap();
        assert_eq!(count, 2);

        // Check integrity — should be clean
        let report = check_game_integrity(&db, "skyrimse", "Gaming", &data_dir).unwrap();
        assert!(report.modified_files.is_empty());
        assert!(report.unknown_files.is_empty());
        assert!(report.missing_files.is_empty());
        assert_eq!(report.total_scanned, 2);
    }

    #[test]
    fn detects_modified_file() {
        let (db, tmp) = test_db();

        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(data_dir.join("test.esp"), b"original").unwrap();

        create_game_snapshot(&db, "skyrimse", "Gaming", &data_dir).unwrap();

        // Modify the file
        fs::write(data_dir.join("test.esp"), b"modified").unwrap();

        let report = check_game_integrity(&db, "skyrimse", "Gaming", &data_dir).unwrap();
        assert_eq!(report.modified_files.len(), 1);
        assert_eq!(report.modified_files[0], "test.esp");
    }

    #[test]
    fn detects_unknown_file() {
        let (db, tmp) = test_db();

        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(data_dir.join("test.esp"), b"data").unwrap();

        create_game_snapshot(&db, "skyrimse", "Gaming", &data_dir).unwrap();

        // Add a new file
        fs::write(data_dir.join("mod.esp"), b"new mod").unwrap();

        let report = check_game_integrity(&db, "skyrimse", "Gaming", &data_dir).unwrap();
        assert_eq!(report.unknown_files.len(), 1);
        assert_eq!(report.unknown_files[0], "mod.esp");
    }

    #[test]
    fn detects_missing_file() {
        let (db, tmp) = test_db();

        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(data_dir.join("test.esp"), b"data").unwrap();

        create_game_snapshot(&db, "skyrimse", "Gaming", &data_dir).unwrap();

        // Delete the file
        fs::remove_file(data_dir.join("test.esp")).unwrap();

        let report = check_game_integrity(&db, "skyrimse", "Gaming", &data_dir).unwrap();
        assert_eq!(report.missing_files.len(), 1);
        assert_eq!(report.missing_files[0], "test.esp");
    }

    #[test]
    fn has_snapshot_works() {
        let (db, tmp) = test_db();

        assert!(!has_snapshot(&db, "skyrimse", "Gaming").unwrap());

        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(data_dir.join("test.esp"), b"data").unwrap();

        create_game_snapshot(&db, "skyrimse", "Gaming", &data_dir).unwrap();

        assert!(has_snapshot(&db, "skyrimse", "Gaming").unwrap());
    }
}
