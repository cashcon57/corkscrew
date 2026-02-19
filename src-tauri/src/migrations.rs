//! Database schema versioning and migration system.
//!
//! Each migration is a function that transforms the schema from version N to
//! N+1. Migrations run inside transactions so that a failed migration leaves
//! the database unchanged.

use rusqlite::{params, Connection};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MigrationError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("Migration failed (v{from} -> v{to}): {reason}")]
    Failed { from: u32, to: u32, reason: String },
}

pub type Result<T> = std::result::Result<T, MigrationError>;

/// The current target schema version. Bump this when adding a new migration.
const TARGET_VERSION: u32 = 2;

/// Get the current schema version (0 if no version table exists).
pub fn current_version(conn: &Connection) -> Result<u32> {
    // Check if schema_version table exists
    let exists: bool = conn
        .prepare("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='schema_version'")?
        .query_row([], |row| row.get::<_, i64>(0))?
        > 0;

    if !exists {
        return Ok(0);
    }

    let version: u32 = conn
        .prepare("SELECT version FROM schema_version LIMIT 1")?
        .query_row([], |row| row.get(0))
        .unwrap_or(0);

    Ok(version)
}

/// Run all pending migrations to bring the schema up to date.
pub fn migrate(conn: &Connection) -> Result<()> {
    let mut version = current_version(conn)?;

    if version == 0 {
        // Fresh database or pre-migration database
        migrate_v0_to_v1(conn)?;
        version = 1;
    }

    if version == 1 {
        migrate_v1_to_v2(conn)?;
        version = 2;
    }

    let _ = version; // suppress unused warning when TARGET_VERSION == current
    debug_assert!(version == TARGET_VERSION);

    Ok(())
}

/// Migration 0 → 1: Baseline schema.
///
/// Creates the schema_version table and the original installed_mods table.
/// If installed_mods already exists (pre-migration database), we just add
/// the version tracking.
fn migrate_v0_to_v1(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    // Create version tracking
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER NOT NULL DEFAULT 0
        );",
    )?;

    // Insert version row if it doesn't exist
    let count: i64 = tx
        .prepare("SELECT count(*) FROM schema_version")?
        .query_row([], |row| row.get(0))?;
    if count == 0 {
        tx.execute("INSERT INTO schema_version (version) VALUES (1)", [])?;
    } else {
        tx.execute("UPDATE schema_version SET version = 1", [])?;
    }

    // Create the baseline installed_mods table (idempotent)
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS installed_mods (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            game_id         TEXT    NOT NULL,
            bottle_name     TEXT    NOT NULL,
            nexus_mod_id    INTEGER,
            name            TEXT    NOT NULL,
            version         TEXT    NOT NULL,
            archive_name    TEXT    NOT NULL,
            installed_files TEXT    NOT NULL,
            installed_at    TEXT    NOT NULL,
            enabled         INTEGER NOT NULL DEFAULT 1
        );

        CREATE INDEX IF NOT EXISTS idx_installed_mods_game_bottle
            ON installed_mods (game_id, bottle_name);",
    )?;

    tx.commit()?;
    log::info!("Migration 0 → 1 complete (baseline schema)");
    Ok(())
}

/// Migration 1 → 2: Enhanced mod tracking.
///
/// Adds new columns to installed_mods and creates deployment_manifest,
/// file_hashes, and conflict_rules tables.
fn migrate_v1_to_v2(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    // Add new columns to installed_mods (SQLite ALTER TABLE ADD COLUMN is safe)
    // Each ALTER is wrapped in its own block to handle "duplicate column" gracefully.
    let new_columns = [
        "ALTER TABLE installed_mods ADD COLUMN nexus_file_id INTEGER",
        "ALTER TABLE installed_mods ADD COLUMN source_url TEXT",
        "ALTER TABLE installed_mods ADD COLUMN staging_path TEXT",
        "ALTER TABLE installed_mods ADD COLUMN install_priority INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE installed_mods ADD COLUMN fomod_selections TEXT",
    ];

    for sql in &new_columns {
        match tx.execute_batch(sql) {
            Ok(_) => {}
            Err(e) => {
                // Ignore "duplicate column name" errors (column already exists)
                let msg = e.to_string();
                if msg.contains("duplicate column") {
                    log::debug!("Column already exists, skipping: {}", sql);
                } else {
                    return Err(MigrationError::Sqlite(e));
                }
            }
        }
    }

    // Create deployment_manifest table
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS deployment_manifest (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            game_id       TEXT    NOT NULL,
            bottle_name   TEXT    NOT NULL,
            mod_id        INTEGER NOT NULL REFERENCES installed_mods(id) ON DELETE CASCADE,
            relative_path TEXT    NOT NULL,
            staging_path  TEXT    NOT NULL,
            deploy_method TEXT    NOT NULL,
            sha256        TEXT,
            deployed_at   TEXT    NOT NULL,
            UNIQUE(game_id, bottle_name, relative_path)
        );

        CREATE INDEX IF NOT EXISTS idx_manifest_game_bottle
            ON deployment_manifest (game_id, bottle_name);
        CREATE INDEX IF NOT EXISTS idx_manifest_mod
            ON deployment_manifest (mod_id);",
    )?;

    // Create file_hashes table
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS file_hashes (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            mod_id        INTEGER NOT NULL REFERENCES installed_mods(id) ON DELETE CASCADE,
            relative_path TEXT    NOT NULL,
            sha256        TEXT    NOT NULL,
            file_size     INTEGER NOT NULL,
            UNIQUE(mod_id, relative_path)
        );",
    )?;

    // Create conflict_rules table
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS conflict_rules (
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            game_id        TEXT    NOT NULL,
            bottle_name    TEXT    NOT NULL,
            winner_mod_id  INTEGER NOT NULL,
            loser_mod_id   INTEGER NOT NULL,
            file_pattern   TEXT,
            UNIQUE(game_id, bottle_name, winner_mod_id, loser_mod_id, file_pattern)
        );",
    )?;

    // Backfill deployment_manifest for existing mods (legacy migration).
    // Existing mods have files directly in the game dir — mark as 'direct'.
    let rows: Vec<(i64, String, String, String, String)> = {
        let mut stmt = tx.prepare(
            "SELECT id, game_id, bottle_name, installed_files, installed_at
             FROM installed_mods
             WHERE staging_path IS NULL",
        )?;

        let mapped = stmt.query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get::<_, String>(3)?,
                row.get(4)?,
            ))
        })?;
        let collected: Vec<_> = mapped.filter_map(|r| r.ok()).collect();
        collected
    }; // stmt dropped here

    for (mod_id, game_id, bottle_name, files_json, installed_at) in &rows {
        let files: Vec<String> = serde_json::from_str(files_json).unwrap_or_default();
        for file_path in &files {
            // Ignore errors from duplicate entries (UNIQUE constraint)
            let _ = tx.execute(
                "INSERT OR IGNORE INTO deployment_manifest
                    (game_id, bottle_name, mod_id, relative_path, staging_path, deploy_method, deployed_at)
                 VALUES (?1, ?2, ?3, ?4, '', 'direct', ?5)",
                params![game_id, bottle_name, mod_id, file_path, installed_at],
            );
        }
    }

    // Update version
    tx.execute("UPDATE schema_version SET version = 2", [])?;

    tx.commit()?;
    log::info!("Migration 1 → 2 complete (deployment manifest, file hashes, conflict rules)");
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn memory_db() -> Connection {
        Connection::open_in_memory().unwrap()
    }

    #[test]
    fn fresh_database_migrates_to_latest() {
        let conn = memory_db();
        assert_eq!(current_version(&conn).unwrap(), 0);

        migrate(&conn).unwrap();
        assert_eq!(current_version(&conn).unwrap(), TARGET_VERSION);

        // Verify tables exist
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert!(tables.contains(&"installed_mods".to_string()));
        assert!(tables.contains(&"deployment_manifest".to_string()));
        assert!(tables.contains(&"file_hashes".to_string()));
        assert!(tables.contains(&"conflict_rules".to_string()));
        assert!(tables.contains(&"schema_version".to_string()));
    }

    #[test]
    fn migration_is_idempotent() {
        let conn = memory_db();
        migrate(&conn).unwrap();
        // Running again should not fail
        migrate(&conn).unwrap();
        assert_eq!(current_version(&conn).unwrap(), TARGET_VERSION);
    }

    #[test]
    fn pre_existing_database_migrates_correctly() {
        let conn = memory_db();

        // Simulate a pre-migration database (just installed_mods, no schema_version)
        conn.execute_batch(
            "CREATE TABLE installed_mods (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                game_id         TEXT NOT NULL,
                bottle_name     TEXT NOT NULL,
                nexus_mod_id    INTEGER,
                name            TEXT NOT NULL,
                version         TEXT NOT NULL,
                archive_name    TEXT NOT NULL,
                installed_files TEXT NOT NULL,
                installed_at    TEXT NOT NULL,
                enabled         INTEGER NOT NULL DEFAULT 1
            );",
        )
        .unwrap();

        // Add a test mod
        conn.execute(
            "INSERT INTO installed_mods
                (game_id, bottle_name, name, version, archive_name, installed_files, installed_at, enabled)
             VALUES ('skyrimse', 'Gaming', 'Test Mod', '1.0', 'test.zip', '[\"meshes/test.nif\"]', '2024-01-01T00:00:00Z', 1)",
            [],
        )
        .unwrap();

        assert_eq!(current_version(&conn).unwrap(), 0);

        // Migrate
        migrate(&conn).unwrap();
        assert_eq!(current_version(&conn).unwrap(), TARGET_VERSION);

        // Verify new columns exist
        let staging: Option<String> = conn
            .prepare("SELECT staging_path FROM installed_mods WHERE id = 1")
            .unwrap()
            .query_row([], |row| row.get(0))
            .unwrap();
        assert!(staging.is_none()); // Should be NULL for legacy mods

        // Verify deployment_manifest was backfilled
        let count: i64 = conn
            .prepare("SELECT count(*) FROM deployment_manifest WHERE mod_id = 1")
            .unwrap()
            .query_row([], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);

        // Verify the backfilled entry
        let (method, path): (String, String) = conn
            .prepare("SELECT deploy_method, relative_path FROM deployment_manifest WHERE mod_id = 1")
            .unwrap()
            .query_row([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap();
        assert_eq!(method, "direct");
        assert_eq!(path, "meshes/test.nif");
    }

    #[test]
    fn installed_mods_new_columns_have_defaults() {
        let conn = memory_db();
        migrate(&conn).unwrap();

        // Insert a mod without the new columns
        conn.execute(
            "INSERT INTO installed_mods
                (game_id, bottle_name, name, version, archive_name, installed_files, installed_at, enabled)
             VALUES ('skyrimse', 'Gaming', 'Test', '1.0', 'test.zip', '[]', '2024-01-01T00:00:00Z', 1)",
            [],
        )
        .unwrap();

        // Verify defaults
        let priority: i64 = conn
            .prepare("SELECT install_priority FROM installed_mods WHERE id = 1")
            .unwrap()
            .query_row([], |row| row.get(0))
            .unwrap();
        assert_eq!(priority, 0);
    }
}
