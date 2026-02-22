//! Mod version rollback and snapshot system.
//!
//! Keeps previous versions of mod staging directories so users can roll back
//! a mod update to a prior version. Also supports creating full mod
//! configuration snapshots (all mods, their versions, enabled states, and
//! priorities) for a game/bottle, enabling restore-to-known-good-state
//! workflows.

use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::database::ModDatabase;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A saved version of a single mod.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModVersion {
    pub id: i64,
    pub mod_id: i64,
    pub version: String,
    pub staging_path: String,
    pub archive_name: String,
    pub created_at: String,
    pub is_current: bool,
}

/// A full snapshot of all mod states for a game/bottle.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModSnapshot {
    pub id: i64,
    pub game_id: String,
    pub bottle_name: String,
    pub name: String,
    pub description: Option<String>,
    pub mod_states: Vec<ModSnapshotEntry>,
    pub created_at: String,
}

/// One mod's state within a snapshot.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModSnapshotEntry {
    pub mod_id: i64,
    pub mod_name: String,
    pub version: String,
    pub enabled: bool,
    pub priority: i32,
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

/// Create the rollback tables if they do not exist.
pub fn init_schema(db: &ModDatabase) -> Result<(), rusqlite::Error> {
    let conn = db.conn().map_err(|_| {
        rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
            Some("Failed to lock database".to_string()),
        )
    })?;
    init_schema_with_conn(&conn)
}

fn init_schema_with_conn(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS mod_versions (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            mod_id       INTEGER NOT NULL,
            version      TEXT    NOT NULL,
            staging_path TEXT    NOT NULL,
            archive_name TEXT    NOT NULL,
            is_current   INTEGER NOT NULL DEFAULT 0,
            created_at   TEXT    NOT NULL,
            FOREIGN KEY (mod_id) REFERENCES installed_mods(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_mod_versions_mod_id
            ON mod_versions (mod_id);

        CREATE TABLE IF NOT EXISTS mod_snapshots (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            game_id     TEXT NOT NULL,
            bottle_name TEXT NOT NULL,
            name        TEXT NOT NULL,
            description TEXT,
            created_at  TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_mod_snapshots_game_bottle
            ON mod_snapshots (game_id, bottle_name);

        CREATE TABLE IF NOT EXISTS snapshot_entries (
            snapshot_id INTEGER NOT NULL,
            mod_id      INTEGER NOT NULL,
            mod_name    TEXT    NOT NULL,
            version     TEXT    NOT NULL,
            enabled     INTEGER NOT NULL,
            priority    INTEGER NOT NULL,
            FOREIGN KEY (snapshot_id) REFERENCES mod_snapshots(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_snapshot_entries_snapshot
            ON snapshot_entries (snapshot_id);",
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Cleanup
// ---------------------------------------------------------------------------

/// Remove on-disk staging directories for all saved versions of a mod.
/// Call this *before* `remove_mod()` so the mod_versions rows still exist.
pub fn cleanup_mod_version_staging(db: &ModDatabase, mod_id: i64) -> Result<(), String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT staging_path FROM mod_versions WHERE mod_id = ?1")
        .map_err(|e| format!("Failed to query mod versions: {}", e))?;
    let paths: Vec<String> = stmt
        .query_map(params![mod_id], |row| row.get(0))
        .map_err(|e| format!("Failed to read mod version rows: {}", e))?
        .filter_map(|r| r.ok())
        .collect();
    drop(stmt);
    drop(conn);

    for path in &paths {
        let p = std::path::Path::new(path);
        if p.exists() {
            let _ = std::fs::remove_dir_all(p);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Mod version management
// ---------------------------------------------------------------------------

/// Save the current staging directory as a versioned entry.
///
/// If this is the first version saved for the mod, it is automatically
/// marked as current. Otherwise, the new version is saved but not marked
/// current (call `rollback_to_version` to switch).
///
/// Returns the new version's row ID.
pub fn save_mod_version(
    db: &ModDatabase,
    mod_id: i64,
    version: &str,
    staging_path: &str,
    archive_name: &str,
) -> Result<i64, String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    let created_at = Utc::now().to_rfc3339();

    // Check if there are any existing versions for this mod.
    let existing_count: i64 = conn
        .prepare("SELECT count(*) FROM mod_versions WHERE mod_id = ?1")
        .map_err(|e| format!("Failed to prepare count: {}", e))?
        .query_row(params![mod_id], |row| row.get(0))
        .map_err(|e| format!("Failed to count versions: {}", e))?;

    let is_current = if existing_count == 0 { 1i64 } else { 0i64 };

    conn.execute(
        "INSERT INTO mod_versions
            (mod_id, version, staging_path, archive_name, is_current, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            mod_id,
            version,
            staging_path,
            archive_name,
            is_current,
            created_at
        ],
    )
    .map_err(|e| format!("Failed to save mod version: {}", e))?;

    Ok(conn.last_insert_rowid())
}

/// List all saved versions for a mod, ordered by creation time (newest first).
pub fn list_mod_versions(db: &ModDatabase, mod_id: i64) -> Result<Vec<ModVersion>, String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, mod_id, version, staging_path, archive_name, is_current, created_at
             FROM mod_versions
             WHERE mod_id = ?1
             ORDER BY created_at DESC",
        )
        .map_err(|e| format!("Failed to prepare statement: {}", e))?;

    let rows = stmt
        .query_map(params![mod_id], |row| {
            let is_current_int: i64 = row.get(5)?;
            Ok(ModVersion {
                id: row.get(0)?,
                mod_id: row.get(1)?,
                version: row.get(2)?,
                staging_path: row.get(3)?,
                archive_name: row.get(4)?,
                is_current: is_current_int != 0,
                created_at: row.get(6)?,
            })
        })
        .map_err(|e| format!("Failed to query versions: {}", e))?;

    let mut versions = Vec::new();
    for row in rows {
        versions.push(row.map_err(|e| format!("Failed to read row: {}", e))?);
    }
    Ok(versions)
}

/// Roll back to a specific saved version.
///
/// Marks the target version as current and clears the `is_current` flag on
/// all other versions for the same mod. Returns the version that was
/// activated.
///
/// Note: this function only updates the database. The caller is responsible
/// for actually swapping the staging directory content on disk (e.g. by
/// re-deploying from the returned `staging_path`).
pub fn rollback_to_version(
    db: &ModDatabase,
    mod_id: i64,
    version_id: i64,
) -> Result<ModVersion, String> {
    let conn = db.conn().map_err(|e| e.to_string())?;

    // Verify the target version exists and belongs to this mod.
    let target: ModVersion = conn
        .prepare(
            "SELECT id, mod_id, version, staging_path, archive_name, is_current, created_at
             FROM mod_versions
             WHERE id = ?1 AND mod_id = ?2",
        )
        .map_err(|e| format!("Failed to prepare query: {}", e))?
        .query_row(params![version_id, mod_id], |row| {
            let is_current_int: i64 = row.get(5)?;
            Ok(ModVersion {
                id: row.get(0)?,
                mod_id: row.get(1)?,
                version: row.get(2)?,
                staging_path: row.get(3)?,
                archive_name: row.get(4)?,
                is_current: is_current_int != 0,
                created_at: row.get(6)?,
            })
        })
        .map_err(|_| format!("Version {} not found for mod {}", version_id, mod_id))?;

    // Clear is_current on all versions for this mod.
    conn.execute(
        "UPDATE mod_versions SET is_current = 0 WHERE mod_id = ?1",
        params![mod_id],
    )
    .map_err(|e| format!("Failed to clear current flags: {}", e))?;

    // Mark the target version as current.
    conn.execute(
        "UPDATE mod_versions SET is_current = 1 WHERE id = ?1",
        params![version_id],
    )
    .map_err(|e| format!("Failed to set current version: {}", e))?;

    Ok(ModVersion {
        is_current: true,
        ..target
    })
}

/// Delete old versions for a mod, keeping only the most recent `keep_count`.
///
/// The currently active version is always kept regardless of `keep_count`.
/// Returns the number of versions deleted.
pub fn cleanup_old_versions(
    db: &ModDatabase,
    mod_id: i64,
    keep_count: usize,
) -> Result<usize, String> {
    let conn = db.conn().map_err(|e| e.to_string())?;

    // Get all versions ordered by creation time, newest first.
    let mut stmt = conn
        .prepare(
            "SELECT id, is_current FROM mod_versions
             WHERE mod_id = ?1
             ORDER BY created_at DESC",
        )
        .map_err(|e| format!("Failed to prepare query: {}", e))?;

    let all_versions: Vec<(i64, bool)> = stmt
        .query_map(params![mod_id], |row| {
            let is_current_int: i64 = row.get(1)?;
            Ok((row.get(0)?, is_current_int != 0))
        })
        .map_err(|e| format!("Failed to query versions: {}", e))?
        .filter_map(|r| r.ok())
        .collect();

    if all_versions.len() <= keep_count {
        return Ok(0);
    }

    // Determine which IDs to delete: everything beyond keep_count, except
    // the current version which is always retained.
    //
    // Pre-reserve a slot for the current version so it counts toward
    // keep_count regardless of its position in the chronological order.
    let has_current = all_versions.iter().any(|(_, c)| *c);
    let mut kept = if has_current { 1usize } else { 0usize };
    let mut to_delete: Vec<i64> = Vec::new();

    for (id, is_current) in &all_versions {
        if *is_current {
            // Already counted above; always keep.
            continue;
        }
        if kept < keep_count {
            kept += 1;
        } else {
            to_delete.push(*id);
        }
    }

    let deleted = to_delete.len();
    for id in to_delete {
        conn.execute("DELETE FROM mod_versions WHERE id = ?1", params![id])
            .map_err(|e| format!("Failed to delete version {}: {}", id, e))?;
    }

    Ok(deleted)
}

// ---------------------------------------------------------------------------
// Snapshot management
// ---------------------------------------------------------------------------

/// Create a full mod configuration snapshot for a game/bottle.
///
/// Reads all currently installed mods and saves their names, versions,
/// enabled states, and priorities into a new snapshot. Returns the
/// snapshot's row ID.
pub fn create_snapshot(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    name: &str,
    description: Option<&str>,
) -> Result<i64, String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    let created_at = Utc::now().to_rfc3339();

    // Insert the snapshot header.
    conn.execute(
        "INSERT INTO mod_snapshots (game_id, bottle_name, name, description, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![game_id, bottle_name, name, description, created_at],
    )
    .map_err(|e| format!("Failed to create snapshot: {}", e))?;

    let snapshot_id = conn.last_insert_rowid();

    // Read all installed mods for this game/bottle and save their states.
    let mut mod_stmt = conn
        .prepare(
            "SELECT id, name, version, enabled, install_priority
             FROM installed_mods
             WHERE game_id = ?1 AND bottle_name = ?2
             ORDER BY install_priority ASC",
        )
        .map_err(|e| format!("Failed to query mods: {}", e))?;

    let mods: Vec<(i64, String, String, bool, i32)> = mod_stmt
        .query_map(params![game_id, bottle_name], |row| {
            let enabled_int: i64 = row.get(3)?;
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                enabled_int != 0,
                row.get(4)?,
            ))
        })
        .map_err(|e| format!("Failed to read mods: {}", e))?
        .filter_map(|r| r.ok())
        .collect();

    let mut insert_stmt = conn
        .prepare(
            "INSERT INTO snapshot_entries
                (snapshot_id, mod_id, mod_name, version, enabled, priority)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .map_err(|e| format!("Failed to prepare insert: {}", e))?;

    for (mod_id, mod_name, version, enabled, priority) in &mods {
        insert_stmt
            .execute(params![
                snapshot_id,
                mod_id,
                mod_name,
                version,
                *enabled as i64,
                priority,
            ])
            .map_err(|e| format!("Failed to insert snapshot entry: {}", e))?;
    }

    Ok(snapshot_id)
}

/// List all snapshots for a game/bottle.
///
/// Returns snapshots without their mod_states populated (use
/// `get_snapshot_states` to load the entries for a specific snapshot).
pub fn list_snapshots(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
) -> Result<Vec<ModSnapshot>, String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, game_id, bottle_name, name, description, created_at
             FROM mod_snapshots
             WHERE game_id = ?1 AND bottle_name = ?2
             ORDER BY created_at DESC",
        )
        .map_err(|e| format!("Failed to prepare query: {}", e))?;

    let rows = stmt
        .query_map(params![game_id, bottle_name], |row| {
            Ok(ModSnapshot {
                id: row.get(0)?,
                game_id: row.get(1)?,
                bottle_name: row.get(2)?,
                name: row.get(3)?,
                description: row.get(4)?,
                mod_states: Vec::new(), // populated on demand
                created_at: row.get(5)?,
            })
        })
        .map_err(|e| format!("Failed to query snapshots: {}", e))?;

    let mut snapshots = Vec::new();
    for row in rows {
        snapshots.push(row.map_err(|e| format!("Failed to read row: {}", e))?);
    }
    Ok(snapshots)
}

/// Get the mod state entries for a specific snapshot.
pub fn get_snapshot_states(
    db: &ModDatabase,
    snapshot_id: i64,
) -> Result<Vec<ModSnapshotEntry>, String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT mod_id, mod_name, version, enabled, priority
             FROM snapshot_entries
             WHERE snapshot_id = ?1
             ORDER BY priority ASC",
        )
        .map_err(|e| format!("Failed to prepare query: {}", e))?;

    let rows = stmt
        .query_map(params![snapshot_id], |row| {
            let enabled_int: i64 = row.get(3)?;
            Ok(ModSnapshotEntry {
                mod_id: row.get(0)?,
                mod_name: row.get(1)?,
                version: row.get(2)?,
                enabled: enabled_int != 0,
                priority: row.get(4)?,
            })
        })
        .map_err(|e| format!("Failed to query entries: {}", e))?;

    let mut entries = Vec::new();
    for row in rows {
        entries.push(row.map_err(|e| format!("Failed to read row: {}", e))?);
    }
    Ok(entries)
}

/// Delete a snapshot and all its entries (cascaded via foreign key).
pub fn delete_snapshot(db: &ModDatabase, snapshot_id: i64) -> Result<(), String> {
    let conn = db.conn().map_err(|e| e.to_string())?;

    // Manually delete entries first in case FK cascade is not enabled.
    conn.execute(
        "DELETE FROM snapshot_entries WHERE snapshot_id = ?1",
        params![snapshot_id],
    )
    .map_err(|e| format!("Failed to delete snapshot entries: {}", e))?;

    let rows = conn
        .execute(
            "DELETE FROM mod_snapshots WHERE id = ?1",
            params![snapshot_id],
        )
        .map_err(|e| format!("Failed to delete snapshot: {}", e))?;

    if rows == 0 {
        return Err(format!("Snapshot with ID {} not found", snapshot_id));
    }

    Ok(())
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
        let db_path = tmp.path().join("test_rollback.db");
        let db = ModDatabase::new(&db_path).unwrap();
        init_schema(&db).unwrap();
        (db, tmp)
    }

    /// Helper: insert a fake mod for testing. Returns the mod ID.
    fn insert_test_mod(db: &ModDatabase, name: &str) -> i64 {
        db.add_mod("skyrimse", "Gaming", None, name, "1.0", "test.zip", &[])
            .unwrap()
    }

    #[test]
    fn test_schema_initialization() {
        let (db, _tmp) = test_db();
        // Schema should be initialized; verify by listing versions/snapshots.
        let versions = list_mod_versions(&db, 999).unwrap();
        assert!(versions.is_empty());

        let snapshots = list_snapshots(&db, "skyrimse", "Gaming").unwrap();
        assert!(snapshots.is_empty());

        // Calling init_schema again should be idempotent.
        init_schema(&db).unwrap();
    }

    #[test]
    fn test_save_and_list_versions() {
        let (db, _tmp) = test_db();
        let mod_id = insert_test_mod(&db, "Test Mod");

        let v1_id = save_mod_version(&db, mod_id, "1.0", "/staging/v1", "mod_v1.zip").unwrap();
        let v2_id = save_mod_version(&db, mod_id, "2.0", "/staging/v2", "mod_v2.zip").unwrap();

        assert!(v1_id > 0);
        assert!(v2_id > 0);
        assert_ne!(v1_id, v2_id);

        let versions = list_mod_versions(&db, mod_id).unwrap();
        assert_eq!(versions.len(), 2);

        // Newest first.
        assert_eq!(versions[0].version, "2.0");
        assert_eq!(versions[1].version, "1.0");

        // First saved version should be current.
        assert!(versions[1].is_current, "First version should be current");
        assert!(
            !versions[0].is_current,
            "Second version should not be current"
        );
    }

    #[test]
    fn test_rollback_marks_correct_version() {
        let (db, _tmp) = test_db();
        let mod_id = insert_test_mod(&db, "Rollback Mod");

        let v1_id = save_mod_version(&db, mod_id, "1.0", "/staging/v1", "v1.zip").unwrap();
        let v2_id = save_mod_version(&db, mod_id, "2.0", "/staging/v2", "v2.zip").unwrap();
        let _v3_id = save_mod_version(&db, mod_id, "3.0", "/staging/v3", "v3.zip").unwrap();

        // Initially v1 is current. Roll back to v2.
        let activated = rollback_to_version(&db, mod_id, v2_id).unwrap();
        assert!(activated.is_current);
        assert_eq!(activated.version, "2.0");
        assert_eq!(activated.staging_path, "/staging/v2");

        // Verify only v2 is current.
        let versions = list_mod_versions(&db, mod_id).unwrap();
        for v in &versions {
            if v.id == v2_id {
                assert!(v.is_current, "v2 should be current");
            } else {
                assert!(
                    !v.is_current,
                    "v{} (id={}) should not be current",
                    v.version, v.id
                );
            }
        }

        // Roll back to v1.
        let activated = rollback_to_version(&db, mod_id, v1_id).unwrap();
        assert_eq!(activated.version, "1.0");
        assert!(activated.is_current);

        // Non-existent version should error.
        assert!(rollback_to_version(&db, mod_id, 99999).is_err());
    }

    #[test]
    fn test_cleanup_keeps_only_n_versions() {
        let (db, _tmp) = test_db();
        let mod_id = insert_test_mod(&db, "Cleanup Mod");

        for i in 1..=5 {
            save_mod_version(
                &db,
                mod_id,
                &format!("{}.0", i),
                &format!("/staging/v{}", i),
                &format!("v{}.zip", i),
            )
            .unwrap();
        }

        // 5 versions exist, keep 2.
        let deleted = cleanup_old_versions(&db, mod_id, 2).unwrap();
        assert_eq!(deleted, 3, "Should have deleted 3 versions");

        let remaining = list_mod_versions(&db, mod_id).unwrap();
        assert_eq!(remaining.len(), 2);

        // The current version (v1, the first saved) should always be kept.
        assert!(
            remaining.iter().any(|v| v.is_current),
            "Current version must be retained"
        );
    }

    #[test]
    fn test_cleanup_noop_when_fewer_than_keep() {
        let (db, _tmp) = test_db();
        let mod_id = insert_test_mod(&db, "Few Versions Mod");

        save_mod_version(&db, mod_id, "1.0", "/staging/v1", "v1.zip").unwrap();
        save_mod_version(&db, mod_id, "2.0", "/staging/v2", "v2.zip").unwrap();

        let deleted = cleanup_old_versions(&db, mod_id, 5).unwrap();
        assert_eq!(deleted, 0);
    }

    #[test]
    fn test_snapshot_create_list_get_states() {
        let (db, _tmp) = test_db();

        // Insert some mods.
        let _m1 = insert_test_mod(&db, "Mod Alpha");
        let m2 = insert_test_mod(&db, "Mod Beta");

        // Disable the second mod.
        db.set_enabled(m2, false).unwrap();

        // Create a snapshot.
        let snap_id = create_snapshot(
            &db,
            "skyrimse",
            "Gaming",
            "Before Update",
            Some("Stable config before major update"),
        )
        .unwrap();
        assert!(snap_id > 0);

        // List snapshots.
        let snapshots = list_snapshots(&db, "skyrimse", "Gaming").unwrap();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].name, "Before Update");
        assert_eq!(
            snapshots[0].description.as_deref(),
            Some("Stable config before major update")
        );

        // Get snapshot states.
        let states = get_snapshot_states(&db, snap_id).unwrap();
        assert_eq!(states.len(), 2);

        let alpha = states.iter().find(|s| s.mod_name == "Mod Alpha").unwrap();
        assert!(alpha.enabled);

        let beta = states.iter().find(|s| s.mod_name == "Mod Beta").unwrap();
        assert!(!beta.enabled);
    }

    #[test]
    fn test_delete_snapshot() {
        let (db, _tmp) = test_db();
        insert_test_mod(&db, "Some Mod");

        let snap_id = create_snapshot(&db, "skyrimse", "Gaming", "Temp", None).unwrap();

        // Entries should exist.
        let states = get_snapshot_states(&db, snap_id).unwrap();
        assert!(!states.is_empty());

        // Delete.
        delete_snapshot(&db, snap_id).unwrap();

        // Snapshot should be gone.
        let snapshots = list_snapshots(&db, "skyrimse", "Gaming").unwrap();
        assert!(snapshots.is_empty());

        // Entries should be gone too.
        let states = get_snapshot_states(&db, snap_id).unwrap();
        assert!(states.is_empty());

        // Deleting again should error.
        assert!(delete_snapshot(&db, snap_id).is_err());
    }

    #[test]
    fn test_snapshot_different_games_are_separate() {
        let (db, _tmp) = test_db();

        db.add_mod(
            "skyrimse",
            "Gaming",
            None,
            "Skyrim Mod",
            "1.0",
            "a.zip",
            &[],
        )
        .unwrap();
        db.add_mod(
            "fallout4",
            "Gaming",
            None,
            "Fallout Mod",
            "1.0",
            "b.zip",
            &[],
        )
        .unwrap();

        create_snapshot(&db, "skyrimse", "Gaming", "Skyrim Snap", None).unwrap();
        create_snapshot(&db, "fallout4", "Gaming", "Fallout Snap", None).unwrap();

        let skyrim_snaps = list_snapshots(&db, "skyrimse", "Gaming").unwrap();
        assert_eq!(skyrim_snaps.len(), 1);
        assert_eq!(skyrim_snaps[0].name, "Skyrim Snap");

        let fallout_snaps = list_snapshots(&db, "fallout4", "Gaming").unwrap();
        assert_eq!(fallout_snaps.len(), 1);
        assert_eq!(fallout_snaps[0].name, "Fallout Snap");
    }

    #[test]
    fn test_version_for_nonexistent_mod() {
        let (db, _tmp) = test_db();
        // Saving a version for a mod that doesn't exist should still work at
        // the DB level (FK enforcement may or may not be on depending on the
        // connection). The important thing is list returns empty for a mod
        // with no versions.
        let versions = list_mod_versions(&db, 99999).unwrap();
        assert!(versions.is_empty());
    }
}
