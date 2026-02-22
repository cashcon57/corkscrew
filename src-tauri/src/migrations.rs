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
const TARGET_VERSION: u32 = 10;

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

    if version == 2 {
        migrate_v2_to_v3(conn)?;
        version = 3;
    }

    if version == 3 {
        migrate_v3_to_v4(conn)?;
        version = 4;
    }

    if version == 4 {
        migrate_v4_to_v5(conn)?;
        version = 5;
    }

    if version == 5 {
        migrate_v5_to_v6(conn)?;
        version = 6;
    }

    if version == 6 {
        migrate_v6_to_v7(conn)?;
        version = 7;
    }

    if version == 7 {
        migrate_v7_to_v8(conn)?;
        version = 8;
    }

    if version == 8 {
        migrate_v8_to_v9(conn)?;
        version = 9;
    }

    if version == 9 {
        migrate_v9_to_v10(conn)?;
        version = 10;
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

/// Migration 2 → 3: Collection tracking.
///
/// Adds a collection_name column to installed_mods so mods can be associated
/// with the NexusMods collection they were installed from.
fn migrate_v2_to_v3(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    match tx.execute_batch("ALTER TABLE installed_mods ADD COLUMN collection_name TEXT") {
        Ok(_) => {}
        Err(e) => {
            let msg = e.to_string();
            if !msg.contains("duplicate column") {
                return Err(MigrationError::Sqlite(e));
            }
        }
    }

    tx.execute("UPDATE schema_version SET version = 3", [])?;
    tx.commit()?;
    log::info!("Migration 2 → 3 complete (collection_name column)");
    Ok(())
}

/// Migration 3 → 4: Download registry, notes & tags.
///
/// Creates the download_registry and download_collection_refs tables for
/// shared download deduplication across collections. Also adds user_notes
/// and user_tags columns to installed_mods.
fn migrate_v3_to_v4(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    // Download registry for deduplication
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS download_registry (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            archive_path    TEXT    NOT NULL,
            archive_name    TEXT    NOT NULL,
            nexus_mod_id    INTEGER,
            nexus_file_id   INTEGER,
            sha256          TEXT,
            file_size       INTEGER NOT NULL DEFAULT 0,
            downloaded_at   TEXT    NOT NULL,
            UNIQUE(archive_path)
        );

        CREATE INDEX IF NOT EXISTS idx_download_registry_nexus
            ON download_registry (nexus_mod_id, nexus_file_id);",
    )?;

    // Tracks which collections reference which downloads
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS download_collection_refs (
            download_id     INTEGER NOT NULL REFERENCES download_registry(id) ON DELETE CASCADE,
            collection_name TEXT    NOT NULL,
            game_id         TEXT    NOT NULL,
            bottle_name     TEXT    NOT NULL,
            UNIQUE(download_id, collection_name, game_id, bottle_name)
        );",
    )?;

    // Mod notes and tags
    let new_columns = [
        "ALTER TABLE installed_mods ADD COLUMN user_notes TEXT",
        "ALTER TABLE installed_mods ADD COLUMN user_tags TEXT", // JSON array
    ];

    for sql in &new_columns {
        match tx.execute_batch(sql) {
            Ok(_) => {}
            Err(e) => {
                let msg = e.to_string();
                if !msg.contains("duplicate column") {
                    return Err(MigrationError::Sqlite(e));
                }
            }
        }
    }

    tx.execute("UPDATE schema_version SET version = 4", [])?;
    tx.commit()?;
    log::info!("Migration 3 → 4 complete (download registry, notes & tags)");
    Ok(())
}

/// Migration 4 → 5: Dependencies, FOMOD recipes, game sessions.
///
/// Adds tables for mod dependency tracking, FOMOD choice replay,
/// game session stability tracking, and INI tweak presets.
fn migrate_v4_to_v5(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    // Mod dependency graph
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS mod_dependencies (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            game_id         TEXT    NOT NULL,
            bottle_name     TEXT    NOT NULL,
            mod_id          INTEGER NOT NULL REFERENCES installed_mods(id) ON DELETE CASCADE,
            depends_on_id   INTEGER REFERENCES installed_mods(id) ON DELETE CASCADE,
            nexus_dep_id    INTEGER,
            dep_name        TEXT    NOT NULL,
            relationship    TEXT    NOT NULL DEFAULT 'requires',
            created_at      TEXT    NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_mod_deps_mod
            ON mod_dependencies (mod_id);
        CREATE INDEX IF NOT EXISTS idx_mod_deps_target
            ON mod_dependencies (depends_on_id);",
    )?;

    // FOMOD recipes (saved installer selections)
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS fomod_recipes (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            mod_id          INTEGER NOT NULL REFERENCES installed_mods(id) ON DELETE CASCADE,
            mod_name        TEXT    NOT NULL,
            installer_hash  TEXT,
            selections_json TEXT    NOT NULL,
            created_at      TEXT    NOT NULL,
            UNIQUE(mod_id)
        );",
    )?;

    // Game sessions for stability tracking
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS game_sessions (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            game_id         TEXT    NOT NULL,
            bottle_name     TEXT    NOT NULL,
            profile_name    TEXT,
            started_at      TEXT    NOT NULL,
            ended_at        TEXT,
            duration_secs   INTEGER,
            clean_exit      INTEGER,
            crash_log_path  TEXT,
            notes           TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_sessions_game
            ON game_sessions (game_id, bottle_name);",
    )?;

    // Mod changes per session
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_mod_changes (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id      INTEGER NOT NULL REFERENCES game_sessions(id) ON DELETE CASCADE,
            mod_id          INTEGER,
            mod_name        TEXT    NOT NULL,
            change_type     TEXT    NOT NULL,
            detail          TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_session_changes
            ON session_mod_changes (session_id);",
    )?;

    // INI presets
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS ini_presets (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            name            TEXT    NOT NULL,
            game_id         TEXT    NOT NULL,
            description     TEXT,
            settings_json   TEXT    NOT NULL,
            is_builtin      INTEGER NOT NULL DEFAULT 0,
            created_at      TEXT    NOT NULL,
            UNIQUE(name, game_id)
        );",
    )?;

    tx.execute("UPDATE schema_version SET version = 5", [])?;
    tx.commit()?;
    log::info!("Migration 4 → 5 complete (dependencies, FOMOD recipes, sessions, INI presets)");
    Ok(())
}

/// Migration 5 → 6: Collection metadata.
///
/// Stores rich metadata about installed collections (slug, author, image_url,
/// manifest JSON snapshot) for the My Collections redesign and diff system.
fn migrate_v5_to_v6(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS collection_metadata (
            id                  INTEGER PRIMARY KEY AUTOINCREMENT,
            collection_name     TEXT NOT NULL,
            game_id             TEXT NOT NULL,
            bottle_name         TEXT NOT NULL,
            slug                TEXT,
            author              TEXT,
            description         TEXT,
            game_domain         TEXT,
            image_url           TEXT,
            installed_revision  INTEGER,
            total_mods          INTEGER,
            installed_at        TEXT NOT NULL,
            manifest_json       TEXT,
            UNIQUE(collection_name, game_id, bottle_name)
        );

        CREATE INDEX IF NOT EXISTS idx_collection_meta_game
            ON collection_metadata (game_id, bottle_name);",
    )?;

    tx.execute("UPDATE schema_version SET version = 6", [])?;
    tx.commit()?;
    log::info!("Migration 5 → 6 complete (collection metadata)");
    Ok(())
}

/// Migration 6 → 7: Auto-category + notification log.
///
/// Adds auto_category column to installed_mods for heuristic-based mod
/// classification, and creates a notification_log table for persistent
/// UI notifications.
fn migrate_v6_to_v7(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    // Add auto_category column
    match tx.execute_batch("ALTER TABLE installed_mods ADD COLUMN auto_category TEXT") {
        Ok(_) => {}
        Err(e) => {
            let msg = e.to_string();
            if !msg.contains("duplicate column") {
                return Err(MigrationError::Sqlite(e));
            }
        }
    }

    // Persistent notification log
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS notification_log (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            level       TEXT    NOT NULL,
            message     TEXT    NOT NULL,
            detail      TEXT,
            created_at  TEXT    NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_notif_created
            ON notification_log (created_at);",
    )?;

    tx.execute("UPDATE schema_version SET version = 7", [])?;
    tx.commit()?;
    log::info!("Migration 6 → 7 complete (auto_category, notification_log)");
    Ok(())
}

/// Migration 7 → 8: Add source_type column for multi-source mod support.
///
/// Tracks where each mod came from: "nexus", "direct", "loverslab", "moddb",
/// "curseforge", or "manual". Backfills existing mods: those with a nexus_mod_id
/// get "nexus", all others get "manual".
fn migrate_v7_to_v8(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    match tx.execute_batch("ALTER TABLE installed_mods ADD COLUMN source_type TEXT NOT NULL DEFAULT 'manual'") {
        Ok(_) => {}
        Err(e) => {
            let msg = e.to_string();
            if !msg.contains("duplicate column") {
                return Err(MigrationError::Sqlite(e));
            }
        }
    }

    // Backfill: mods with nexus_mod_id get "nexus"
    tx.execute(
        "UPDATE installed_mods SET source_type = 'nexus' WHERE nexus_mod_id IS NOT NULL",
        [],
    )?;

    tx.execute("UPDATE schema_version SET version = 8", [])?;
    tx.commit()?;
    log::info!("Migration 7 → 8 complete (source_type column)");
    Ok(())
}

/// Migration 8 → 9: Persistent download queue table.
///
/// Stores download queue items so they survive app restarts. Items with status
/// "downloading" are reset to "pending" on load (since the download was
/// interrupted).
fn migrate_v8_to_v9(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS download_queue (
            id INTEGER PRIMARY KEY,
            mod_name TEXT NOT NULL,
            file_name TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            error TEXT,
            attempt INTEGER NOT NULL DEFAULT 0,
            max_attempts INTEGER NOT NULL DEFAULT 3,
            downloaded_bytes INTEGER NOT NULL DEFAULT 0,
            total_bytes INTEGER NOT NULL DEFAULT 0,
            nexus_mod_id INTEGER,
            nexus_file_id INTEGER,
            url TEXT,
            game_slug TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    )?;

    tx.execute("UPDATE schema_version SET version = 9", [])?;
    tx.commit()?;
    log::info!("Migration 8 → 9 complete (download_queue table)");
    Ok(())
}

/// Migration 9 → 10: Wabbajack install pipeline tables.
///
/// Creates tables for tracking Wabbajack modlist installations and per-archive
/// download status. Also adds xxhash64 and file_path columns to the download
/// registry for shared download cache lookups.
fn migrate_v9_to_v10(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    // Wabbajack install tracking
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS wabbajack_installs (
            id                    INTEGER PRIMARY KEY AUTOINCREMENT,
            modlist_name          TEXT    NOT NULL,
            modlist_version       TEXT    NOT NULL DEFAULT '',
            game_type             INTEGER NOT NULL DEFAULT 0,
            install_dir           TEXT    NOT NULL,
            status                TEXT    NOT NULL DEFAULT 'pending',
            total_archives        INTEGER NOT NULL DEFAULT 0,
            completed_archives    INTEGER NOT NULL DEFAULT 0,
            total_directives      INTEGER NOT NULL DEFAULT 0,
            completed_directives  INTEGER NOT NULL DEFAULT 0,
            error_message         TEXT,
            created_at            TEXT    NOT NULL DEFAULT (datetime('now')),
            updated_at            TEXT    NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS wabbajack_archive_status (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            install_id    INTEGER NOT NULL REFERENCES wabbajack_installs(id) ON DELETE CASCADE,
            archive_hash  TEXT    NOT NULL,
            archive_name  TEXT    NOT NULL,
            source_type   TEXT    NOT NULL DEFAULT '',
            status        TEXT    NOT NULL DEFAULT 'pending',
            download_path TEXT,
            error_message TEXT,
            UNIQUE(install_id, archive_hash)
        );

        CREATE INDEX IF NOT EXISTS idx_wj_archive_status_install
            ON wabbajack_archive_status (install_id);",
    )?;

    // Add xxhash64 and file_path columns to download_registry for shared cache
    let has_xxhash: bool = tx
        .prepare("SELECT 1 FROM pragma_table_info('download_registry') WHERE name = 'xxhash64'")?
        .exists([])?;

    if !has_xxhash {
        tx.execute_batch(
            "ALTER TABLE download_registry ADD COLUMN xxhash64 TEXT;
             ALTER TABLE download_registry ADD COLUMN file_path TEXT;
             CREATE INDEX IF NOT EXISTS idx_download_registry_xxhash
                ON download_registry (xxhash64);",
        )?;
    }

    tx.execute("UPDATE schema_version SET version = 10", [])?;
    tx.commit()?;
    log::info!("Migration 9 → 10 complete (wabbajack install pipeline tables)");
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
        assert!(tables.contains(&"download_registry".to_string()));
        assert!(tables.contains(&"download_collection_refs".to_string()));
        assert!(tables.contains(&"collection_metadata".to_string()));
        assert!(tables.contains(&"notification_log".to_string()));
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
            .prepare(
                "SELECT deploy_method, relative_path FROM deployment_manifest WHERE mod_id = 1",
            )
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
