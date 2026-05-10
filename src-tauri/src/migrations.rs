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
pub const TARGET_VERSION: u32 = 24;

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

    if version == 10 {
        migrate_v10_to_v11(conn)?;
        version = 11;
    }

    if version == 11 {
        migrate_v11_to_v12(conn)?;
        version = 12;
    }

    if version == 12 {
        migrate_v12_to_v13(conn)?;
        version = 13;
    }

    if version == 13 {
        migrate_v13_to_v14(conn)?;
        version = 14;
    }

    if version == 14 {
        migrate_v14_to_v15(conn)?;
        version = 15;
    }

    if version == 15 {
        migrate_v15_to_v16(conn)?;
        version = 16;
    }

    if version == 16 {
        migrate_v16_to_v17(conn)?;
        version = 17;
    }

    if version == 17 {
        migrate_v17_to_v18(conn)?;
        version = 18;
    }

    if version == 18 {
        migrate_v18_to_v19(conn)?;
        version = 19;
    }

    if version == 19 {
        migrate_v19_to_v20(conn)?;
        version = 20;
    }

    if version == 20 {
        migrate_v20_to_v21(conn)?;
        version = 21;
    }

    if version == 21 {
        migrate_v21_to_v22(conn)?;
        version = 22;
    }

    if version == 22 {
        migrate_v22_to_v23(conn)?;
        version = 23;
    }

    if version == 23 {
        migrate_v23_to_v24(conn)?;
        version = 24;
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
                // Ignore "duplicate column name" errors (column already exists).
                // Match the exact SQLite error phrasing to avoid masking other errors.
                let msg = e.to_string();
                if msg.contains("duplicate column name") {
                    log::warn!("Column already exists, skipping: {}", sql);
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

    match tx.execute_batch(
        "ALTER TABLE installed_mods ADD COLUMN source_type TEXT NOT NULL DEFAULT 'manual'",
    ) {
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

/// Migration 10 → 11: Collection install checkpoints.
///
/// Creates a table to track in-progress collection installations so they can
/// be resumed after interruption. Each row stores the full manifest JSON and
/// per-mod completion status as a JSON object.
fn migrate_v10_to_v11(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS collection_install_checkpoints (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            collection_name TEXT    NOT NULL,
            game_id         TEXT    NOT NULL,
            bottle_name     TEXT    NOT NULL,
            manifest_json   TEXT    NOT NULL,
            status          TEXT    NOT NULL DEFAULT 'in_progress',
            total_mods      INTEGER NOT NULL DEFAULT 0,
            completed_mods  INTEGER NOT NULL DEFAULT 0,
            failed_mods     INTEGER NOT NULL DEFAULT 0,
            skipped_mods    INTEGER NOT NULL DEFAULT 0,
            mod_statuses    TEXT    NOT NULL DEFAULT '{}',
            error_message   TEXT,
            created_at      TEXT    NOT NULL DEFAULT (datetime('now')),
            updated_at      TEXT    NOT NULL DEFAULT (datetime('now')),
            UNIQUE(collection_name, game_id, bottle_name)
        );

        CREATE INDEX IF NOT EXISTS idx_checkpoint_game_bottle
            ON collection_install_checkpoints (game_id, bottle_name, status);",
    )?;

    tx.execute("UPDATE schema_version SET version = 11", [])?;
    tx.commit()?;
    log::info!("Migration 10 → 11 complete (collection install checkpoints)");
    Ok(())
}

/// Migration 11 → 12: Pinned game versions.
///
/// Stores the last-known game executable version per game/bottle so Corkscrew
/// can warn users when Steam silently updates their game and breaks SKSE mods.
fn migrate_v11_to_v12(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS pinned_game_versions (
            game_id     TEXT NOT NULL,
            bottle_name TEXT NOT NULL,
            version     TEXT NOT NULL,
            pinned_at   TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (game_id, bottle_name)
        );",
    )?;

    tx.execute("UPDATE schema_version SET version = 12", [])?;
    tx.commit()?;
    log::info!("Migration 11 → 12 complete (pinned game versions)");
    Ok(())
}

/// Migration 12 → 13: Incremental deployment index.
///
/// Adds a compound index on deployment_manifest(game_id, bottle_name, mod_id)
/// for fast lookups during incremental deployment diff computation.
fn migrate_v12_to_v13(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    tx.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_manifest_game_bottle_mod
            ON deployment_manifest (game_id, bottle_name, mod_id);",
    )?;

    tx.execute("UPDATE schema_version SET version = 13", [])?;
    tx.commit()?;
    log::info!("Migration 12 → 13 complete (incremental deployment index)");
    Ok(())
}

/// v13 → v14: Add `deploy_target` column to deployment_manifest.
/// Values: "data" (default, normal Data/ folder) or "root" (game root folder).
fn migrate_v13_to_v14(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    tx.execute_batch(
        "ALTER TABLE deployment_manifest
            ADD COLUMN deploy_target TEXT NOT NULL DEFAULT 'data';",
    )?;

    tx.execute("UPDATE schema_version SET version = 14", [])?;
    tx.commit()?;
    log::info!("Migration 13 → 14 complete (deploy_target column)");
    Ok(())
}

/// v14 → v15: Add `collection_optional` column to installed_mods.
/// Tracks whether a mod was an optional pick in its parent collection.
/// Backfills from stored manifest_json in collection_metadata.
fn migrate_v14_to_v15(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    // Add the column (INTEGER: 0 = required, 1 = optional)
    tx.execute_batch(
        "ALTER TABLE installed_mods
            ADD COLUMN collection_optional INTEGER NOT NULL DEFAULT 0;",
    )?;

    // Backfill from stored collection manifests.
    // For each collection with a manifest_json, parse it and mark matching
    // installed mods as optional based on mod name matching.
    {
        let mut meta_stmt = tx.prepare(
            "SELECT collection_name, game_id, bottle_name, manifest_json
             FROM collection_metadata
             WHERE manifest_json IS NOT NULL",
        )?;
        let rows: Vec<(String, String, String, String)> = meta_stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();

        for (coll_name, game_id, bottle_name, manifest_json) in &rows {
            // Parse manifest to find optional mod names
            if let Ok(manifest) = serde_json::from_str::<serde_json::Value>(manifest_json) {
                if let Some(mods) = manifest.get("mods").and_then(|v| v.as_array()) {
                    for m in mods {
                        let is_optional =
                            m.get("optional").and_then(|v| v.as_bool()).unwrap_or(false);
                        if is_optional {
                            if let Some(mod_name) = m.get("name").and_then(|v| v.as_str()) {
                                // Mark matching installed mods as optional
                                let _ = tx.execute(
                                    "UPDATE installed_mods
                                     SET collection_optional = 1
                                     WHERE collection_name = ?1
                                       AND game_id = ?2
                                       AND bottle_name = ?3
                                       AND name = ?4",
                                    params![coll_name, game_id, bottle_name, mod_name],
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    tx.execute("UPDATE schema_version SET version = 15", [])?;
    tx.commit()?;
    log::info!("Migration 14 → 15 complete (collection_optional column + backfill)");
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Migration 15 → 16: Vortex extension cache table.
fn migrate_v15_to_v16(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    crate::vortex_registry::create_table(&tx).map_err(|e| MigrationError::Failed {
        from: 15,
        to: 16,
        reason: e.to_string(),
    })?;

    tx.execute("UPDATE schema_version SET version = 16", [])?;
    tx.commit()?;
    log::info!("Migration 15 → 16 complete (vortex extension cache)");
    Ok(())
}

/// Migration 16 → 17: Custom games table for user-added games.
fn migrate_v16_to_v17(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS custom_games (
            game_id TEXT PRIMARY KEY,
            display_name TEXT NOT NULL,
            nexus_slug TEXT NOT NULL DEFAULT '',
            game_path TEXT NOT NULL,
            exe_path TEXT,
            data_dir TEXT NOT NULL,
            bottle_name TEXT NOT NULL,
            bottle_path TEXT NOT NULL,
            steam_app_id TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )?;

    tx.execute("UPDATE schema_version SET version = 17", [])?;
    tx.commit()?;
    log::info!("Migration 16 → 17 complete (custom games table)");
    Ok(())
}

/// Migration 17 -> 18: Chat history persistence.
///
/// Creates a table to store LLM chat messages per game/bottle so conversation
/// context survives app restarts and model reloads.
fn migrate_v17_to_v18(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS chat_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            game_id TEXT NOT NULL,
            bottle_name TEXT NOT NULL,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            tool_calls TEXT,
            mentioned_mods TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_chat_history_game
            ON chat_history (game_id, bottle_name);",
    )?;

    tx.execute("UPDATE schema_version SET version = 18", [])?;
    tx.commit()?;
    log::info!("Migration 17 → 18 complete (chat history table)");
    Ok(())
}

/// Migration 18 -> 19: Shader conversion tracking.
fn migrate_v18_to_v19(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS shader_conversions (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            game_id         TEXT NOT NULL,
            bottle_name     TEXT NOT NULL,
            snapshot_id     INTEGER NOT NULL,
            status          TEXT NOT NULL DEFAULT 'completed',
            disabled_mods   TEXT NOT NULL DEFAULT '[]',
            swapped_mods    TEXT NOT NULL DEFAULT '[]',
            enb_installed   INTEGER NOT NULL DEFAULT 0,
            created_at      TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_shader_conversions_game_bottle
            ON shader_conversions (game_id, bottle_name);",
    )?;

    tx.execute("UPDATE schema_version SET version = 19", [])?;
    tx.commit()?;
    log::info!("Migration 18 → 19 complete (shader conversions table)");
    Ok(())
}

/// Migration 19 -> 20: Wabbajack directive-level resume tracking.
///
/// Creates a table to track individual directive completion status during
/// Wabbajack installs, enabling resume after crash or cancellation without
/// re-processing already-completed directives.
fn migrate_v19_to_v20(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS wj_directive_status (
            install_id      TEXT    NOT NULL,
            directive_index INTEGER NOT NULL,
            directive_type  TEXT    NOT NULL,
            status          TEXT    NOT NULL DEFAULT 'pending',
            updated_at      TEXT    NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (install_id, directive_index)
        );

        CREATE INDEX IF NOT EXISTS idx_wj_directive_status_install
            ON wj_directive_status (install_id);

        CREATE TABLE IF NOT EXISTS nexus_url_cache (
            game_domain TEXT NOT NULL,
            mod_id INTEGER NOT NULL,
            file_id INTEGER NOT NULL,
            url TEXT NOT NULL,
            cached_at TEXT NOT NULL DEFAULT (datetime('now')),
            expires_at TEXT NOT NULL,
            PRIMARY KEY (game_domain, mod_id, file_id)
        );

        CREATE TABLE IF NOT EXISTS patch_basis_cache (
            modlist TEXT NOT NULL,
            file_path TEXT NOT NULL,
            quick_hash INTEGER NOT NULL,
            full_hash TEXT NOT NULL,
            file_size INTEGER NOT NULL,
            cached_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (modlist, file_path)
        );

        CREATE TABLE IF NOT EXISTS modlist_config (
            name TEXT NOT NULL,
            version TEXT NOT NULL,
            config_json TEXT NOT NULL,
            saved_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (name, version)
        );",
    )?;

    tx.execute("UPDATE schema_version SET version = 20", [])?;
    tx.commit()?;
    log::info!("Migration 19 → 20 complete (wj_directive_status + cache tables)");
    Ok(())
}

/// Migration 20 → 21: Structured error tracking table for proactive diagnostics.
fn migrate_v20_to_v21(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS error_events (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp   TEXT    NOT NULL DEFAULT (datetime('now')),
            module      TEXT    NOT NULL,
            error_type  TEXT    NOT NULL,
            message     TEXT    NOT NULL,
            count       INTEGER NOT NULL DEFAULT 1,
            first_seen  TEXT    NOT NULL DEFAULT (datetime('now')),
            last_seen   TEXT    NOT NULL DEFAULT (datetime('now')),
            resolved    INTEGER NOT NULL DEFAULT 0
        );

        CREATE INDEX IF NOT EXISTS idx_error_events_module
            ON error_events (module, error_type);

        CREATE INDEX IF NOT EXISTS idx_error_events_last_seen
            ON error_events (last_seen DESC);",
    )?;

    tx.execute("UPDATE schema_version SET version = 21", [])?;
    tx.commit()?;
    log::info!("Migration 20 → 21 complete (error_events table)");
    Ok(())
}

/// Migration 21 → 22: Steam depot manifest history for game version rollback.
///
/// Auto-captures (game_version, build_id, manifest_id, depot_id) from Steam
/// ACF files every time a game is detected. Enables automated downgrade to
/// any previously-seen version without needing external manifest databases.
fn migrate_v21_to_v22(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS steam_depot_history (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            game_id     TEXT NOT NULL,
            app_id      TEXT NOT NULL,
            depot_id    TEXT NOT NULL,
            manifest_id TEXT NOT NULL,
            build_id    TEXT,
            game_version TEXT,
            captured_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(game_id, depot_id, manifest_id)
        );",
    )?;

    tx.execute("UPDATE schema_version SET version = 22", [])?;
    tx.commit()?;
    log::info!("Migration 21 → 22 complete (steam_depot_history table)");
    Ok(())
}

// ---------------------------------------------------------------------------
// Idempotency helpers (used by v23+)
// ---------------------------------------------------------------------------

/// Returns `true` if `column` exists in `table`.
fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info(`{}`)", table))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Add `column` to `table` if it does not already exist.
///
/// **SAFETY:** `ddl` is interpolated verbatim into the SQL string. Callers
/// MUST pass a literal column definition (`"runtime TEXT NOT NULL DEFAULT 'wine'"`),
/// never a value derived from user input or external config. `table` is
/// quoted with backticks but `ddl` is not — the column-definition grammar
/// is too rich to safely escape.
fn add_column_if_missing(conn: &Connection, table: &str, column: &str, ddl: &str) -> Result<()> {
    if !column_exists(conn, table, column)? {
        conn.execute(&format!("ALTER TABLE `{}` ADD COLUMN {}", table, ddl), [])?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Migration 22 → 23
// ---------------------------------------------------------------------------

/// Migration 22 → 23: Runtime discriminator + native game columns on `games`.
///
/// Introduces the `runtime` column so every game row is explicitly labelled
/// `'wine'` or `'native'`. Six native-specific columns are added as nullable
/// (or NOT NULL with a safe default); they are NULL for all existing Wine rows.
///
/// The `games` table itself is created here if it does not yet exist (it was not
/// part of the pre-23 schema — games were detected at runtime, not persisted).
/// Existing databases that were upgraded from v22 will have no rows in `games`,
/// and all six new columns are added idempotently via `add_column_if_missing`.
///
/// `bottle_name` and `bottle_path` intentionally remain NOT NULL in this
/// migration (SQLite cannot drop NOT NULL via ALTER). Native rows should pass
/// empty strings until a future v24 rebuild makes those columns nullable.
fn migrate_v22_to_v23(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    // Create the games table if this is a fresh database or an older schema
    // that never persisted games rows.
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS games (
            game_id     TEXT    NOT NULL PRIMARY KEY,
            bottle_name TEXT    NOT NULL DEFAULT '',
            bottle_path TEXT    NOT NULL DEFAULT ''
        );",
    )?;

    // Add runtime discriminator. Existing rows default to 'wine'.
    add_column_if_missing(
        &tx,
        "games",
        "runtime",
        "runtime TEXT NOT NULL DEFAULT 'wine'",
    )?;

    // Native game fields — NULL for Wine games.
    add_column_if_missing(&tx, "games", "native_app_path", "native_app_path TEXT")?;
    add_column_if_missing(&tx, "games", "native_data_root", "native_data_root TEXT")?;
    add_column_if_missing(
        &tx,
        "games",
        "native_architecture",
        "native_architecture TEXT",
    )?;
    add_column_if_missing(
        &tx,
        "games",
        "native_sandboxed",
        "native_sandboxed INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(&tx, "games", "native_source", "native_source TEXT")?;

    tx.execute("UPDATE schema_version SET version = 23", [])?;

    tx.commit()?;
    log::info!("Migration 22 → 23 complete (runtime column + native game fields)");
    Ok(())
}

/// Migration v23 → v24: extend `deployment_manifest` UNIQUE constraint to
/// include `deploy_target`.
///
/// **Why.** Prior to v24 the table's UNIQUE constraint was
/// `(game_id, bottle_name, relative_path)`. With BepInEx / UE / Vortex
/// routing, two mods can legitimately land the same `relative_path` under
/// different deploy targets (e.g. `data/` vs `BepInEx/plugins/`). The
/// previous schema collapsed those rows; insert-or-replace also dropped
/// the original deploy_target via COALESCE-by-path. v24 rebuilds the
/// table with `UNIQUE(game_id, bottle_name, deploy_target, relative_path)`
/// so callers can pass `deploy_target` explicitly and the manifest stops
/// drifting.
///
/// SQLite requires a full table rebuild to change a UNIQUE constraint:
/// new table → copy rows → drop old → rename → recreate indexes.
fn migrate_v23_to_v24(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    tx.execute_batch(
        "CREATE TABLE deployment_manifest_v24 (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            game_id       TEXT    NOT NULL,
            bottle_name   TEXT    NOT NULL,
            mod_id        INTEGER NOT NULL REFERENCES installed_mods(id) ON DELETE CASCADE,
            relative_path TEXT    NOT NULL,
            staging_path  TEXT    NOT NULL,
            deploy_method TEXT    NOT NULL,
            sha256        TEXT,
            deployed_at   TEXT    NOT NULL,
            deploy_target TEXT    NOT NULL DEFAULT 'data',
            UNIQUE(game_id, bottle_name, deploy_target, relative_path)
        );",
    )?;

    // Copy every existing row, preserving the deploy_target value added in v14.
    tx.execute_batch(
        "INSERT INTO deployment_manifest_v24
            (id, game_id, bottle_name, mod_id, relative_path, staging_path,
             deploy_method, sha256, deployed_at, deploy_target)
         SELECT id, game_id, bottle_name, mod_id, relative_path, staging_path,
                deploy_method, sha256, deployed_at,
                COALESCE(deploy_target, 'data')
         FROM deployment_manifest;",
    )?;

    tx.execute_batch(
        "DROP TABLE deployment_manifest;
         ALTER TABLE deployment_manifest_v24 RENAME TO deployment_manifest;
         CREATE INDEX IF NOT EXISTS idx_manifest_game_bottle
             ON deployment_manifest (game_id, bottle_name);
         CREATE INDEX IF NOT EXISTS idx_manifest_mod
             ON deployment_manifest (mod_id);
         CREATE INDEX IF NOT EXISTS idx_manifest_game_bottle_mod
             ON deployment_manifest (game_id, bottle_name, mod_id);
         CREATE INDEX IF NOT EXISTS idx_manifest_target
             ON deployment_manifest (game_id, bottle_name, deploy_target);",
    )?;

    tx.execute("UPDATE schema_version SET version = 24", [])?;

    tx.commit()?;
    log::info!(
        "Migration 23 → 24 complete (deployment_manifest UNIQUE now includes deploy_target)"
    );
    Ok(())
}

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
        assert!(tables.contains(&"error_events".to_string()));
    }

    #[test]
    fn error_events_table_works() {
        let conn = memory_db();
        migrate(&conn).unwrap();

        // Insert an error event
        conn.execute(
            "INSERT INTO error_events (module, error_type, message) VALUES (?1, ?2, ?3)",
            rusqlite::params!["test_module", "test_error", "something broke"],
        )
        .unwrap();

        // Verify it was inserted with defaults
        let (count, resolved): (i64, i64) = conn
            .prepare("SELECT count, resolved FROM error_events WHERE module = 'test_module'")
            .unwrap()
            .query_row([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(resolved, 0);

        // Verify indexes exist
        let indexes: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='error_events'")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(indexes.iter().any(|n| n.contains("module")));
        assert!(indexes.iter().any(|n| n.contains("last_seen")));
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

    #[test]
    fn v13_creates_deployment_manifest_index() {
        let conn = memory_db();
        migrate(&conn).unwrap();
        assert_eq!(current_version(&conn).unwrap(), TARGET_VERSION);

        // Verify the compound index exists
        let index_exists: bool = conn
            .prepare(
                "SELECT COUNT(*) > 0 FROM sqlite_master
                 WHERE type = 'index' AND name = 'idx_manifest_game_bottle_mod'",
            )
            .unwrap()
            .query_row([], |row| row.get(0))
            .unwrap();
        assert!(index_exists, "v13 compound index should exist");
    }

    #[test]
    fn v23_adds_runtime_column_with_wine_default() {
        let conn = memory_db();
        migrate(&conn).unwrap();
        assert_eq!(current_version(&conn).unwrap(), TARGET_VERSION);

        // Insert a row using the v22 column set — runtime should default to 'wine'.
        conn.execute(
            "INSERT INTO games (game_id, bottle_name, bottle_path) VALUES ('test', 'b', '/p')",
            [],
        )
        .unwrap();

        let runtime: String = conn
            .query_row(
                "SELECT runtime FROM games WHERE game_id = 'test'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(runtime, "wine");
    }

    /// End-to-end smoke for the v23→v24 migration WITH real-shaped data,
    /// going through `ModDatabase::new` so we also exercise the pre-migration
    /// DB-file snapshot path. Builds a synthetic v23 DB on disk, populates
    /// it with rows that would have collided under the old UNIQUE constraint,
    /// then runs the production open path and verifies row preservation +
    /// backup-file creation.
    #[test]
    fn v24_full_open_path_with_synthetic_v23_db() {
        use crate::database::ModDatabase;
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("mods.db");

        // Build a v23-only DB by stopping the chain at 23.
        {
            let conn = Connection::open(&db_path).unwrap();
            migrate_v0_to_v1(&conn).unwrap();
            migrate_v1_to_v2(&conn).unwrap();
            migrate_v2_to_v3(&conn).unwrap();
            migrate_v3_to_v4(&conn).unwrap();
            migrate_v4_to_v5(&conn).unwrap();
            migrate_v5_to_v6(&conn).unwrap();
            migrate_v6_to_v7(&conn).unwrap();
            migrate_v7_to_v8(&conn).unwrap();
            migrate_v8_to_v9(&conn).unwrap();
            migrate_v9_to_v10(&conn).unwrap();
            migrate_v10_to_v11(&conn).unwrap();
            migrate_v11_to_v12(&conn).unwrap();
            migrate_v12_to_v13(&conn).unwrap();
            migrate_v13_to_v14(&conn).unwrap();
            migrate_v14_to_v15(&conn).unwrap();
            migrate_v15_to_v16(&conn).unwrap();
            migrate_v16_to_v17(&conn).unwrap();
            migrate_v17_to_v18(&conn).unwrap();
            migrate_v18_to_v19(&conn).unwrap();
            migrate_v19_to_v20(&conn).unwrap();
            migrate_v20_to_v21(&conn).unwrap();
            migrate_v21_to_v22(&conn).unwrap();
            migrate_v22_to_v23(&conn).unwrap();
            assert_eq!(current_version(&conn).unwrap(), 23);

            // Insert one mod and two manifest rows that map the SAME
            // relative_path under different deploy_targets. Under the old
            // v23 UNIQUE this would have collapsed to one; v24 should
            // preserve both.
            conn.execute(
                "INSERT INTO installed_mods (id, game_id, bottle_name, name, version, archive_name, installed_files, installed_at)
                 VALUES (42, 'skyrimse', 'b', 'TestMod', '1.0', 'm.zip', '[\"foo.dll\"]', '2026-01-01')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO deployment_manifest
                    (game_id, bottle_name, mod_id, relative_path, staging_path, deploy_method, deployed_at, deploy_target)
                 VALUES ('skyrimse', 'b', 42, 'foo.dll', '/s/a', 'hardlink', '2026-01-01', 'data')",
                [],
            ).unwrap();
            // Note: this row will COLLIDE with the one above under the v23
            // UNIQUE — INSERT OR REPLACE would lose it. Insert with a
            // different relative_path so both survive the synthetic setup,
            // then assert the post-migration state allows the previously-
            // forbidden combination.
            conn.execute(
                "INSERT INTO deployment_manifest
                    (game_id, bottle_name, mod_id, relative_path, staging_path, deploy_method, deployed_at, deploy_target)
                 VALUES ('skyrimse', 'b', 42, 'bar.dll', '/s/b', 'hardlink', '2026-01-01', 'root')",
                [],
            ).unwrap();
        }

        let pre_size = std::fs::metadata(&db_path).unwrap().len();

        // Now run the production open path (triggers backup + migration).
        let db = ModDatabase::new(&db_path).expect("ModDatabase::new must succeed");
        drop(db);

        // 1. Schema is at TARGET_VERSION.
        let conn = Connection::open(&db_path).unwrap();
        assert_eq!(current_version(&conn).unwrap(), TARGET_VERSION);

        // 2. Both manifest rows survived.
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM deployment_manifest", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 2, "manifest rows must survive rebuild");

        // 3. New UNIQUE: insert a duplicate-by-path row under a different
        //    deploy_target — must succeed.
        let insert = conn.execute(
            "INSERT INTO deployment_manifest
                (game_id, bottle_name, mod_id, relative_path, staging_path, deploy_method, deployed_at, deploy_target)
             VALUES ('skyrimse', 'b', 42, 'foo.dll', '/s/c', 'hardlink', '2026-01-02', 'root')",
            [],
        );
        assert!(insert.is_ok(), "v24 UNIQUE must permit different deploy_target");

        // 4. Backup file exists next to the DB.
        let parent = db_path.parent().unwrap();
        let entries: Vec<_> = std::fs::read_dir(parent)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        let backup_found = entries.iter().any(|name| name.contains(".pre-v") && name.ends_with(".backup"));
        assert!(
            backup_found,
            "expected a pre-migration backup file next to the DB, found: {:?}",
            entries
        );

        // Sanity: backup file size matches pre-migration size.
        let backup_name = entries
            .iter()
            .find(|n| n.contains(".pre-v") && n.ends_with(".backup"))
            .unwrap();
        let backup_size = std::fs::metadata(parent.join(backup_name)).unwrap().len();
        assert_eq!(
            backup_size, pre_size,
            "backup size must match pre-migration DB size"
        );

        eprintln!(
            "v24 full-open smoke: migrated and backed up {} ({} bytes)",
            backup_name, backup_size
        );
    }

    /// One-off smoke test: if a real user DB is sitting at the well-known
    /// location, copy it to a scratch path and run the v24 migration against
    /// it to verify row preservation + final schema. Always passes on CI
    /// (file doesn't exist there); locally validates against actual data.
    #[test]
    fn v24_real_db_smoke() {
        let candidate = "/tmp/corkscrew-migration-test.db";
        if !std::path::Path::new(candidate).exists() {
            eprintln!("(skipping v24_real_db_smoke — no scratch DB at {})", candidate);
            return;
        }
        let conn = Connection::open(candidate).expect("open scratch DB");
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();

        let pre_v = current_version(&conn).unwrap();
        let pre_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM deployment_manifest", [], |r| r.get(0))
            .unwrap();
        eprintln!("smoke before: version={} rows={}", pre_v, pre_count);

        migrate(&conn).expect("migration must succeed on real DB");

        let post_v = current_version(&conn).unwrap();
        let post_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM deployment_manifest", [], |r| r.get(0))
            .unwrap();
        eprintln!("smoke after:  version={} rows={}", post_v, post_count);

        assert_eq!(post_v, TARGET_VERSION, "schema should be at TARGET_VERSION");
        assert_eq!(
            pre_count, post_count,
            "row count must survive deployment_manifest rebuild"
        );
    }

    #[test]
    fn v24_deployment_manifest_unique_includes_deploy_target() {
        let conn = memory_db();
        migrate(&conn).unwrap();

        // Add a fake mod so the FK constraint is satisfied. Use only columns
        // that have been present since v1 + use NULL/empty for everything else
        // so this test isn't coupled to later schema additions.
        conn.execute_batch(
            "INSERT INTO installed_mods
                (id, game_id, bottle_name, name, version, archive_name, installed_files, installed_at)
             VALUES (1, 'skyrimse', 'b', 'M', '1', 'a.zip', '[]', '2026-01-01');",
        ).unwrap();

        // Two rows that previously would have collided on
        // UNIQUE(game_id, bottle_name, relative_path). Under v24 they are
        // distinguishable by deploy_target and must both succeed.
        conn.execute(
            "INSERT INTO deployment_manifest
                (game_id, bottle_name, mod_id, relative_path, staging_path, deploy_method, deployed_at, deploy_target)
             VALUES ('skyrimse', 'b', 1, 'foo.dll', '/s', 'hardlink', 't', 'data')",
            [],
        )
        .unwrap();

        let inserted_root = conn.execute(
            "INSERT INTO deployment_manifest
                (game_id, bottle_name, mod_id, relative_path, staging_path, deploy_method, deployed_at, deploy_target)
             VALUES ('skyrimse', 'b', 1, 'foo.dll', '/s2', 'hardlink', 't', 'root')",
            [],
        );
        assert!(
            inserted_root.is_ok(),
            "v24 UNIQUE constraint must allow same relative_path under a different deploy_target"
        );

        // Indexes from earlier migrations must still exist after the table rebuild.
        for idx in &[
            "idx_manifest_game_bottle",
            "idx_manifest_mod",
            "idx_manifest_game_bottle_mod",
            "idx_manifest_target",
        ] {
            let exists: bool = conn
                .prepare("SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='index' AND name = ?1")
                .unwrap()
                .query_row([idx], |r| r.get(0))
                .unwrap();
            assert!(exists, "missing index after v24 rebuild: {}", idx);
        }
    }

    #[test]
    fn v23_native_columns_nullable() {
        let conn = memory_db();
        migrate(&conn).unwrap();
        assert_eq!(current_version(&conn).unwrap(), TARGET_VERSION);

        conn.execute(
            "INSERT INTO games (game_id, runtime, native_app_path, native_architecture)
             VALUES ('sdv', 'native', '/Applications/Stardew Valley.app', 'apple_silicon')",
            [],
        )
        .unwrap();

        let arch: String = conn
            .query_row(
                "SELECT native_architecture FROM games WHERE game_id = 'sdv'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(arch, "apple_silicon");

        // native_data_root and native_source should be nullable (NULL for this row)
        let data_root: Option<String> = conn
            .query_row(
                "SELECT native_data_root FROM games WHERE game_id = 'sdv'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            data_root.is_none(),
            "native_data_root should be NULL when not provided"
        );
    }

    #[test]
    fn v23_migration_is_idempotent() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        // Calling the v23 migration directly a second time must not error.
        migrate_v22_to_v23(&conn).unwrap();
        assert_eq!(current_version(&conn).unwrap(), 23);
        // And a row insertable under v23's column set still works.
        conn.execute(
            "INSERT INTO games (game_id, runtime) VALUES ('idemp', 'wine')",
            [],
        ).unwrap();
    }
}
