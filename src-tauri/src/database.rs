use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Error, Debug)]
pub enum DatabaseError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Mod not found: {0}")]
    ModNotFound(i64),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, DatabaseError>;

// ---------------------------------------------------------------------------
// InstalledMod
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstalledMod {
    pub id: i64,
    pub game_id: String,
    pub bottle_name: String,
    pub nexus_mod_id: Option<i64>,
    pub nexus_file_id: Option<i64>,
    pub source_url: Option<String>,
    pub source_type: String,
    pub name: String,
    pub version: String,
    pub archive_name: String,
    pub installed_files: Vec<String>,
    pub file_count: usize,
    pub installed_at: String,
    pub enabled: bool,
    pub staging_path: Option<String>,
    pub install_priority: i32,
    pub collection_name: Option<String>,
    pub user_notes: Option<String>,
    pub user_tags: Vec<String>,
    pub auto_category: Option<String>,
    pub collection_optional: bool,
}

/// Lightweight mod summary for list views — omits `installed_files` to avoid
/// serializing hundreds of thousands of file paths across 1900+ mods.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModSummary {
    pub id: i64,
    pub game_id: String,
    pub bottle_name: String,
    pub nexus_mod_id: Option<i64>,
    pub nexus_file_id: Option<i64>,
    pub source_url: Option<String>,
    pub source_type: String,
    pub name: String,
    pub version: String,
    pub archive_name: String,
    pub file_count: usize,
    pub installed_at: String,
    pub enabled: bool,
    pub staging_path: Option<String>,
    pub install_priority: i32,
    pub collection_name: Option<String>,
    pub user_notes: Option<String>,
    pub user_tags: Vec<String>,
    pub auto_category: Option<String>,
    pub collection_optional: bool,
}

// ---------------------------------------------------------------------------
// DeploymentEntry
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeploymentEntry {
    pub id: i64,
    pub game_id: String,
    pub bottle_name: String,
    pub mod_id: i64,
    pub relative_path: String,
    pub staging_path: String,
    pub deploy_method: String,
    pub sha256: Option<String>,
    pub deployed_at: String,
    pub mod_name: String,
    /// "data" (default — game Data/ folder) or "root" (game root folder).
    pub deploy_target: String,
}

// ---------------------------------------------------------------------------
// DownloadRecord
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DownloadRecord {
    pub id: i64,
    pub archive_path: String,
    pub archive_name: String,
    pub nexus_mod_id: Option<i64>,
    pub nexus_file_id: Option<i64>,
    pub sha256: Option<String>,
    pub file_size: i64,
    pub downloaded_at: String,
}

// ---------------------------------------------------------------------------
// CollectionSummary
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CollectionSummary {
    pub name: String,
    pub mod_count: usize,
    pub enabled_count: usize,
    pub slug: Option<String>,
    pub author: Option<String>,
    pub image_url: Option<String>,
    pub game_domain: Option<String>,
    pub installed_revision: Option<u32>,
    pub original_mod_count: Option<usize>,
    #[serde(default)]
    pub game_versions: Vec<String>,
}

// ---------------------------------------------------------------------------
// NotificationEntry
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NotificationEntry {
    pub id: i64,
    pub level: String,
    pub message: String,
    pub detail: Option<String>,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// FileConflict
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileConflict {
    pub relative_path: String,
    pub mods: Vec<ConflictModInfo>,
    pub winner_mod_id: i64,
    /// True when ALL conflicting mods belong to the same collection.
    /// These are "expected" overlaps resolved by the collection author's priority order.
    pub same_collection: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConflictModInfo {
    pub mod_id: i64,
    pub mod_name: String,
    pub priority: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CollectionInstallCheckpoint {
    pub id: i64,
    pub collection_name: String,
    pub game_id: String,
    pub bottle_name: String,
    #[serde(skip_serializing)]
    pub manifest_json: String,
    pub status: String,
    pub total_mods: i64,
    pub completed_mods: i64,
    pub failed_mods: i64,
    pub skipped_mods: i64,
    pub mod_statuses: String,
    pub error_message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

// ---------------------------------------------------------------------------
// ModDatabase
// ---------------------------------------------------------------------------

/// Wraps a `rusqlite::Connection` in a `Mutex` so the struct is `Send`.
/// rusqlite `Connection` is not `Send` by default on every platform, so the
/// mutex guarantees exclusive access and satisfies the trait bound.
pub struct ModDatabase {
    conn: Mutex<Connection>,
}

/// Detect if a path is on a network filesystem where WAL mode may fail.
/// Copy the database file to a timestamped sibling before a schema migration
/// that rebuilds tables. Best-effort: the caller logs and continues on failure.
///
/// File pattern: `<db>.pre-v{target}-{YYYYMMDD-HHMMSS}.backup`. Backups live
/// next to the DB so they're easy to spot and easy to swap back manually.
fn snapshot_db_file(db_path: &Path, from_version: u32) -> std::io::Result<()> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup_name = format!(
        "{}.pre-v{}-from-v{}-ts{}.backup",
        db_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("mods.db"),
        crate::migrations::TARGET_VERSION,
        from_version,
        secs,
    );
    let backup_path = db_path.with_file_name(backup_name);

    std::fs::copy(db_path, &backup_path)?;
    log::info!(
        "Pre-migration DB snapshot: {} -> {}",
        db_path.display(),
        backup_path.display()
    );
    Ok(())
}

fn is_network_filesystem(path: &Path) -> bool {
    #[cfg(target_os = "linux")]
    {
        // Try /proc/mounts first (more reliable than stat on all Linux distros)
        if let Ok(mounts) = std::fs::read_to_string("/proc/mounts") {
            let path_str = path.to_string_lossy();
            for line in mounts.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 3 {
                    let mount_point = parts[1];
                    let fs_type = parts[2].to_lowercase();
                    if path_str.starts_with(mount_point)
                        && matches!(
                            fs_type.as_str(),
                            "nfs" | "nfs4" | "cifs" | "smb" | "smb2" | "9p" | "fuse.sshfs"
                        )
                    {
                        return true;
                    }
                }
            }
            return false;
        }
        // Fallback to GNU stat (may not exist on all systems)
        let output = std::process::Command::new("stat")
            .arg("-f")
            .arg("-c")
            .arg("%T")
            .arg(path)
            .output()
            .ok();
        if let Some(out) = output {
            let fs_type = String::from_utf8_lossy(&out.stdout)
                .trim()
                .to_lowercase()
                .to_string();
            return matches!(
                fs_type.as_str(),
                "nfs" | "cifs" | "smb" | "smb2" | "9p" | "fuse.sshfs"
            );
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
    }
    false
}

impl ModDatabase {
    /// Open (or create) the database at `db_path`, creating parent
    /// directories as needed, and run all pending schema migrations.
    pub fn new(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path)?;

        // Set restrictive permissions on the database file (owner-only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(db_path, std::fs::Permissions::from_mode(0o600));
        }

        // Enable WAL mode for better concurrent-read performance.
        // On network filesystems WAL can fail, so fall back to DELETE mode.
        if is_network_filesystem(db_path) {
            log::warn!("Database on network filesystem — using exclusive locking instead of WAL");
            conn.execute_batch("PRAGMA journal_mode = DELETE; PRAGMA locking_mode = EXCLUSIVE;")?;
        } else {
            conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        }

        // Enable foreign key enforcement.
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;

        // Wait if the database is locked by another connection. Linux —
        // and Steam Deck in particular — can hit longer contention windows
        // when deploy + scan + LOOT sort overlap under load, so give Linux
        // a more generous budget. macOS keeps the original 5 s.
        #[cfg(target_os = "linux")]
        let busy_timeout = std::time::Duration::from_secs(15);
        #[cfg(not(target_os = "linux"))]
        let busy_timeout = std::time::Duration::from_secs(5);
        conn.busy_timeout(busy_timeout)?;

        // If a migration is about to rebuild the deployment_manifest table
        // (v23 → v24), copy the DB file to a timestamped backup first so
        // the rebuild can be recovered from if anything goes wrong. Only
        // takes a snapshot when there's actually data to protect: a brand
        // new DB at version 0 doesn't get one.
        let starting_version = crate::migrations::current_version(&conn)
            .map_err(|e| DatabaseError::Other(format!("Read schema version failed: {}", e)))?;
        if starting_version > 0 && starting_version < crate::migrations::TARGET_VERSION {
            if let Err(e) = snapshot_db_file(db_path, starting_version) {
                log::warn!(
                    "Pre-migration DB snapshot failed (continuing anyway): {}",
                    e
                );
            }
        }

        // Run schema migrations
        crate::migrations::migrate(&conn)
            .map_err(|e| DatabaseError::Other(format!("Schema migration failed: {}", e)))?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    // -- connection access ---------------------------------------------------

    /// Obtain a lock on the underlying database connection.
    ///
    /// This is used by other modules (e.g. `executables`) that need to run
    /// their own SQL against the same database.
    pub fn conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|e| DatabaseError::Other(format!("Failed to lock database: {}", e)))
    }

    /// Run PRAGMA integrity_check and return the result string.
    pub fn execute_pragma_integrity_check(&self) -> Result<String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let result: String = conn
            .prepare("PRAGMA integrity_check")?
            .query_row([], |row| row.get(0))?;
        Ok(result)
    }

    // -- helpers ------------------------------------------------------------

    /// Build an `InstalledMod` from the current row of a prepared statement.
    /// Column order must match [`Self::SELECT_COLUMNS`].
    fn row_to_mod(row: &rusqlite::Row<'_>) -> rusqlite::Result<InstalledMod> {
        let files_json: String = row.get(7)?;
        let installed_files: Vec<String> = serde_json::from_str(&files_json).unwrap_or_else(|e| {
            log::warn!("Failed to parse installed_files JSON: {}", e);
            Vec::new()
        });
        let enabled_int: i64 = row.get(9)?;

        let tags_json: Option<String> = row.get(16)?;
        let user_tags: Vec<String> = tags_json
            .map(|s| {
                serde_json::from_str(&s).unwrap_or_else(|e| {
                    log::warn!("Failed to parse user_tags JSON: {}", e);
                    Vec::new()
                })
            })
            .unwrap_or_default();

        Ok(InstalledMod {
            id: row.get(0)?,
            game_id: row.get(1)?,
            bottle_name: row.get(2)?,
            nexus_mod_id: row.get(3)?,
            nexus_file_id: row.get(10)?,
            source_url: row.get(11)?,
            source_type: row
                .get::<_, Option<String>>(18)?
                .unwrap_or_else(|| "manual".to_string()),
            name: row.get(4)?,
            version: row.get(5)?,
            archive_name: row.get(6)?,
            file_count: installed_files.len(),
            installed_files,
            installed_at: row.get(8)?,
            enabled: enabled_int != 0,
            staging_path: row.get(12)?,
            install_priority: row.get(13)?,
            collection_name: row.get(14)?,
            user_notes: row.get(15)?,
            user_tags,
            auto_category: row.get(17)?,
            collection_optional: row.get::<_, i32>(19).unwrap_or(0) != 0,
        })
    }

    /// Build a `ModSummary` from the current row — same column order as
    /// [`Self::SELECT_COLUMNS`] but counts files instead of parsing the JSON array.
    fn row_to_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<ModSummary> {
        let files_json: String = row.get(7)?;
        // Fast count: count commas in the JSON array + 1, or 0 for "[]"
        let file_count = if files_json.len() <= 2 {
            0
        } else {
            files_json.matches(',').count() + 1
        };
        let enabled_int: i64 = row.get(9)?;

        let tags_json: Option<String> = row.get(16)?;
        let user_tags: Vec<String> = tags_json
            .map(|s| {
                serde_json::from_str(&s).unwrap_or_else(|e| {
                    log::warn!("Failed to parse user_tags JSON: {}", e);
                    Vec::new()
                })
            })
            .unwrap_or_default();

        Ok(ModSummary {
            id: row.get(0)?,
            game_id: row.get(1)?,
            bottle_name: row.get(2)?,
            nexus_mod_id: row.get(3)?,
            nexus_file_id: row.get(10)?,
            source_url: row.get(11)?,
            source_type: row
                .get::<_, Option<String>>(18)?
                .unwrap_or_else(|| "manual".to_string()),
            name: row.get(4)?,
            version: row.get(5)?,
            archive_name: row.get(6)?,
            file_count,
            installed_at: row.get(8)?,
            enabled: enabled_int != 0,
            staging_path: row.get(12)?,
            install_priority: row.get(13)?,
            collection_name: row.get(14)?,
            user_notes: row.get(15)?,
            user_tags,
            auto_category: row.get(17)?,
            collection_optional: row.get::<_, i32>(19).unwrap_or(0) != 0,
        })
    }

    /// The column list used in every SELECT on `installed_mods`.
    const SELECT_COLUMNS: &'static str = "id, game_id, bottle_name, nexus_mod_id, name, version, \
         archive_name, installed_files, installed_at, enabled, \
         nexus_file_id, source_url, staging_path, install_priority, \
         collection_name, user_notes, user_tags, auto_category, source_type, \
         collection_optional";

    // -- public API ---------------------------------------------------------

    /// Insert a new mod record and return its auto-generated row id.
    #[allow(clippy::too_many_arguments)]
    pub fn add_mod(
        &self,
        game_id: &str,
        bottle_name: &str,
        nexus_mod_id: Option<i64>,
        name: &str,
        version: &str,
        archive_name: &str,
        installed_files: &[String],
    ) -> Result<i64> {
        let files_json = serde_json::to_string(installed_files)?;
        let installed_at = Utc::now().to_rfc3339();

        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO installed_mods
                (game_id, bottle_name, nexus_mod_id, name, version,
                 archive_name, installed_files, installed_at, enabled)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1)",
            params![
                game_id,
                bottle_name,
                nexus_mod_id,
                name,
                version,
                archive_name,
                files_json,
                installed_at,
            ],
        )?;

        Ok(conn.last_insert_rowid())
    }

    /// Delete a mod by id. Returns `Ok(Some(mod))` with the removed record,
    /// or `Ok(None)` if no mod with that id exists.
    ///
    /// All related rows (installed_mods, profile_mods, conflict_rules) are
    /// deleted inside a single transaction to prevent inconsistent state if
    /// the app crashes mid-operation.
    pub fn remove_mod(&self, mod_id: i64) -> Result<Option<InstalledMod>> {
        let existing = self.get_mod(mod_id)?;
        if existing.is_some() {
            let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
            let tx = conn.unchecked_transaction()?;
            tx.execute("DELETE FROM installed_mods WHERE id = ?1", params![mod_id])?;
            // Clean up profile_mods references (table may not exist if profiles not initialized)
            let _ = tx.execute(
                "DELETE FROM profile_mods WHERE mod_id = ?1",
                params![mod_id],
            );
            // Clean up conflict rules involving this mod
            let _ = tx.execute(
                "DELETE FROM conflict_rules WHERE winner_mod_id = ?1 OR loser_mod_id = ?1",
                params![mod_id],
            );
            tx.commit()?;
        }
        Ok(existing)
    }

    /// Find mod IDs matching a name, game, and bottle (for deduplication on re-install).
    pub fn find_mods_by_name(
        &self,
        game_id: &str,
        bottle_name: &str,
        name: &str,
    ) -> Result<Vec<i64>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id FROM installed_mods WHERE game_id = ?1 AND bottle_name = ?2 AND name = ?3",
        )?;
        let rows = stmt.query_map(params![game_id, bottle_name, name], |row| {
            row.get::<_, i64>(0)
        })?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(row?);
        }
        Ok(ids)
    }

    /// Fetch a single mod by its primary key.
    pub fn get_mod(&self, mod_id: i64) -> Result<Option<InstalledMod>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM installed_mods WHERE id = ?1",
            Self::SELECT_COLUMNS,
        ))?;

        let mut rows = stmt.query_map(params![mod_id], Self::row_to_mod)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Get install_priority for all mods, keyed by mod id.
    pub fn get_all_mod_priorities(&self) -> Result<std::collections::HashMap<i64, i64>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare("SELECT id, install_priority FROM installed_mods")?;
        let map = stmt
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(map)
    }

    /// List every mod installed for a given game + bottle combination.
    pub fn list_mods(&self, game_id: &str, bottle_name: &str) -> Result<Vec<InstalledMod>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM installed_mods \
             WHERE game_id = ?1 AND bottle_name = ?2 \
             ORDER BY installed_at DESC",
            Self::SELECT_COLUMNS,
        ))?;

        let rows = stmt.query_map(params![game_id, bottle_name], Self::row_to_mod)?;

        let mut mods = Vec::new();
        for row in rows {
            mods.push(row?);
        }
        Ok(mods)
    }

    /// Fast summary list — same as `list_mods` but returns `ModSummary` with
    /// `file_count` instead of the full `installed_files` array.
    pub fn list_mods_summary(&self, game_id: &str, bottle_name: &str) -> Result<Vec<ModSummary>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM installed_mods \
             WHERE game_id = ?1 AND bottle_name = ?2 \
             ORDER BY installed_at DESC",
            Self::SELECT_COLUMNS,
        ))?;

        let rows = stmt.query_map(params![game_id, bottle_name], Self::row_to_summary)?;

        let mut mods = Vec::new();
        for row in rows {
            mods.push(row?);
        }
        Ok(mods)
    }

    /// Ultra-lightweight mod counts — just `SELECT COUNT(*)` queries, no row
    /// deserialization at all.  Returns `(total_mods, total_enabled)`.
    pub fn get_mod_counts(&self, game_id: &str, bottle_name: &str) -> Result<(usize, usize)> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM installed_mods WHERE game_id = ?1 AND bottle_name = ?2",
            params![game_id, bottle_name],
            |row| row.get(0),
        )?;
        let enabled: i64 = conn.query_row(
            "SELECT COUNT(*) FROM installed_mods WHERE game_id = ?1 AND bottle_name = ?2 AND enabled = 1",
            params![game_id, bottle_name],
            |row| row.get(0),
        )?;
        Ok((total as usize, enabled as usize))
    }

    /// Lightweight deployment manifest count — avoids deserializing all rows.
    pub fn get_deployment_count(&self, game_id: &str, bottle_name: &str) -> Result<usize> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM deployment_manifest WHERE game_id = ?1 AND bottle_name = ?2",
            params![game_id, bottle_name],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Return a map of **file path -> mod id** for every file installed by
    /// any mod in the given game + bottle.
    pub fn get_all_installed_files(
        &self,
        game_id: &str,
        bottle_name: &str,
    ) -> Result<HashMap<String, i64>> {
        let mods = self.list_mods(game_id, bottle_name)?;
        let mut map = HashMap::new();
        for m in mods {
            for file in &m.installed_files {
                map.insert(file.clone(), m.id);
            }
        }
        Ok(map)
    }

    /// Given a set of file paths that a new mod wants to install, return a
    /// map of **file path -> owning mod name** for every conflict (i.e. the
    /// file is already claimed by an existing mod).
    pub fn find_conflicts(
        &self,
        game_id: &str,
        bottle_name: &str,
        new_files: &[String],
    ) -> Result<HashMap<String, String>> {
        let mods = self.list_mods(game_id, bottle_name)?;
        let mut conflicts = HashMap::new();

        // Build a quick lookup: file -> mod name
        let mut file_owners: HashMap<&str, &str> = HashMap::new();
        for m in &mods {
            for file in &m.installed_files {
                file_owners.insert(file.as_str(), m.name.as_str());
            }
        }

        for file in new_files {
            if let Some(owner) = file_owners.get(file.as_str()) {
                conflicts.insert(file.clone(), (*owner).to_string());
            }
        }

        Ok(conflicts)
    }

    /// Enable or disable a mod.
    pub fn set_enabled(&self, mod_id: i64, enabled: bool) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let rows_changed = conn.execute(
            "UPDATE installed_mods SET enabled = ?1 WHERE id = ?2",
            params![enabled as i64, mod_id],
        )?;

        if rows_changed == 0 {
            return Err(DatabaseError::ModNotFound(mod_id));
        }
        Ok(())
    }

    /// Set the staging path for a mod.
    pub fn set_staging_path(&self, mod_id: i64, staging_path: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE installed_mods SET staging_path = ?1 WHERE id = ?2",
            params![staging_path, mod_id],
        )?;
        Ok(())
    }

    /// Set Nexus Mods IDs for a mod (used when installing from collections).
    pub fn set_nexus_ids(
        &self,
        mod_id: i64,
        nexus_mod_id: i64,
        nexus_file_id: Option<i64>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE installed_mods SET nexus_mod_id = ?1, nexus_file_id = ?2 WHERE id = ?3",
            params![nexus_mod_id, nexus_file_id, mod_id],
        )?;
        Ok(())
    }

    /// Set the source URL for a mod.
    pub fn set_source_url(&self, mod_id: i64, url: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE installed_mods SET source_url = ?1 WHERE id = ?2",
            params![url, mod_id],
        )?;
        Ok(())
    }

    /// Set the source type and optionally the source URL for a mod.
    pub fn set_mod_source(
        &self,
        mod_id: i64,
        source_type: &str,
        source_url: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE installed_mods SET source_type = ?1, source_url = ?2 WHERE id = ?3",
            params![source_type, source_url, mod_id],
        )?;
        Ok(())
    }

    /// Tag a mod as belonging to a NexusMods collection.
    pub fn set_collection_name(&self, mod_id: i64, collection_name: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE installed_mods SET collection_name = ?1 WHERE id = ?2",
            params![collection_name, mod_id],
        )?;
        Ok(())
    }

    /// Mark a mod as optional within its collection.
    pub fn set_collection_optional(&self, mod_id: i64, optional: bool) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE installed_mods SET collection_optional = ?1 WHERE id = ?2",
            params![optional as i32, mod_id],
        )?;
        Ok(())
    }

    /// Set the install priority for a mod.
    pub fn set_mod_priority(&self, mod_id: i64, priority: i32) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE installed_mods SET install_priority = ?1 WHERE id = ?2",
            params![priority, mod_id],
        )?;
        Ok(())
    }

    /// Reorder mod priorities for a game/bottle based on the given ID order.
    /// The first ID in the list gets priority 0, the second gets 1, etc.
    pub fn reorder_priorities(
        &self,
        game_id: &str,
        bottle_name: &str,
        ordered_mod_ids: &[i64],
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        // Use a transaction so partial reorder failures roll back cleanly.
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<()> {
            let mut stmt = conn.prepare(
                "UPDATE installed_mods SET install_priority = ?1
                 WHERE id = ?2 AND game_id = ?3 AND bottle_name = ?4",
            )?;
            for (i, mod_id) in ordered_mod_ids.iter().enumerate() {
                stmt.execute(params![i as i32, mod_id, game_id, bottle_name])?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                conn.execute_batch("COMMIT")?;
                Ok(())
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    // -- Deployment manifest ------------------------------------------------

    /// Add a deployment manifest entry.
    #[allow(clippy::too_many_arguments)]
    pub fn add_deployment_entry(
        &self,
        game_id: &str,
        bottle_name: &str,
        mod_id: i64,
        relative_path: &str,
        staging_path: &str,
        deploy_method: &str,
        sha256: Option<&str>,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let deployed_at = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT OR REPLACE INTO deployment_manifest
                (game_id, bottle_name, mod_id, relative_path, staging_path, deploy_method, sha256, deployed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![game_id, bottle_name, mod_id, relative_path, staging_path, deploy_method, sha256, deployed_at],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Batch-insert deployment entries in a single transaction for maximum throughput.
    ///
    /// `deploy_target` is now part of the UNIQUE key (see migration v24);
    /// each call writes the explicit value passed in instead of inferring it
    /// from a sibling row by relative_path. This prevents target drift when
    /// two mods legitimately share a relative_path under different targets
    /// (e.g. `data/` vs `BepInEx/plugins/`).
    pub fn batch_add_deployment_entries(
        &self,
        entries: &[(
            &str, // game_id
            &str, // bottle_name
            i64,  // mod_id
            &str, // relative_path
            &str, // staging_path
            &str, // deploy_method
            &str, // deploy_target
        )],
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let deployed_at = chrono::Utc::now().to_rfc3339();
        let tx = conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO deployment_manifest
                    (game_id, bottle_name, mod_id, relative_path, staging_path, deploy_method, sha256, deployed_at, deploy_target)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8)",
            )?;
            for (game_id, bottle_name, mod_id, rel_path, staging_path, method, deploy_target) in entries {
                stmt.execute(params![
                    game_id,
                    bottle_name,
                    mod_id,
                    rel_path,
                    staging_path,
                    method,
                    deployed_at,
                    deploy_target,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Remove all deployment manifest entries for a mod.
    pub fn remove_deployment_entries_for_mod(&self, mod_id: i64) -> Result<Vec<String>> {
        let paths_with_target = self.get_deployment_paths_for_mod(mod_id)?;
        let paths: Vec<String> = paths_with_target.into_iter().map(|(p, _)| p).collect();

        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "DELETE FROM deployment_manifest WHERE mod_id = ?1",
            params![mod_id],
        )?;

        Ok(paths)
    }

    /// Bulk-delete deployment manifest entries for a set of mod IDs in one
    /// transaction.  Returns all removed relative paths.
    pub fn bulk_remove_deployment_entries(&self, mod_ids: &[i64]) -> Result<Vec<(String, String)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        // Collect all paths + deploy targets first
        let placeholders: String = mod_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let query = format!(
            "SELECT relative_path, deploy_target FROM deployment_manifest WHERE mod_id IN ({})",
            placeholders
        );
        let mut stmt = conn.prepare(&query)?;
        let params: Vec<&dyn rusqlite::types::ToSql> = mod_ids
            .iter()
            .map(|id| id as &dyn rusqlite::types::ToSql)
            .collect();
        let paths: Vec<(String, String)> = stmt
            .query_map(params.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)
                        .unwrap_or_else(|_| "data".to_string()),
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();
        // Delete in one shot
        let delete_query = format!(
            "DELETE FROM deployment_manifest WHERE mod_id IN ({})",
            placeholders
        );
        let mut del_stmt = conn.prepare(&delete_query)?;
        let del_params: Vec<&dyn rusqlite::types::ToSql> = mod_ids
            .iter()
            .map(|id| id as &dyn rusqlite::types::ToSql)
            .collect();
        del_stmt.execute(del_params.as_slice())?;
        Ok(paths)
    }

    /// Bulk-remove mods from the database in one transaction.
    pub fn bulk_remove_mods(&self, mod_ids: &[i64]) -> Result<usize> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let placeholders: String = mod_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        // Clean related tables first (same as remove_mod but in batch).
        // Table names are hardcoded constants — NOT user input.
        const RELATED_TABLES: &[&str] = &[
            "file_hashes",
            "deployment_manifest",
            "mod_dependencies",
            "profile_mods",
        ];
        for table in RELATED_TABLES {
            let q = format!("DELETE FROM {} WHERE mod_id IN ({})", table, placeholders);
            let mut stmt = conn.prepare(&q)?;
            let p: Vec<&dyn rusqlite::types::ToSql> = mod_ids
                .iter()
                .map(|id| id as &dyn rusqlite::types::ToSql)
                .collect();
            let _ = stmt.execute(p.as_slice());
        }
        let q = format!("DELETE FROM installed_mods WHERE id IN ({})", placeholders);
        let mut stmt = conn.prepare(&q)?;
        let p: Vec<&dyn rusqlite::types::ToSql> = mod_ids
            .iter()
            .map(|id| id as &dyn rusqlite::types::ToSql)
            .collect();
        let deleted = stmt.execute(p.as_slice())?;
        Ok(deleted)
    }

    /// Get deployed file paths for a mod without deleting the manifest entries.
    /// Returns `(relative_path, deploy_target)` tuples.
    pub fn get_deployment_paths_for_mod(&self, mod_id: i64) -> Result<Vec<(String, String)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT relative_path, deploy_target FROM deployment_manifest WHERE mod_id = ?1",
        )?;
        let paths: Vec<(String, String)> = stmt
            .query_map(params![mod_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)
                        .unwrap_or_else(|_| "data".to_string()),
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(paths)
    }

    /// Set deploy_target for all deployment entries of a mod.
    pub fn set_deploy_target_for_mod(&self, mod_id: i64, target: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE deployment_manifest SET deploy_target = ?1 WHERE mod_id = ?2",
            params![target, mod_id],
        )?;
        Ok(())
    }

    /// Get the deploy_target for a mod (from its deployment manifest entries).
    /// Returns "data" if no entries exist.
    pub fn get_deploy_target_for_mod(&self, mod_id: i64) -> Result<String> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let result: Option<String> = conn
            .query_row(
                "SELECT deploy_target FROM deployment_manifest WHERE mod_id = ?1 LIMIT 1",
                params![mod_id],
                |row| row.get(0),
            )
            .ok();
        Ok(result.unwrap_or_else(|| "data".to_string()))
    }

    /// Get deployment manifest for a game/bottle.
    pub fn get_deployment_manifest(
        &self,
        game_id: &str,
        bottle_name: &str,
    ) -> Result<Vec<DeploymentEntry>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT dm.id, dm.game_id, dm.bottle_name, dm.mod_id, dm.relative_path,
                    dm.staging_path, dm.deploy_method, dm.sha256, dm.deployed_at,
                    COALESCE(im.name, 'Unknown') as mod_name,
                    dm.deploy_target
             FROM deployment_manifest dm
             LEFT JOIN installed_mods im ON dm.mod_id = im.id
             WHERE dm.game_id = ?1 AND dm.bottle_name = ?2
             ORDER BY dm.relative_path",
        )?;

        let rows = stmt.query_map(params![game_id, bottle_name], |row| {
            Ok(DeploymentEntry {
                id: row.get(0)?,
                game_id: row.get(1)?,
                bottle_name: row.get(2)?,
                mod_id: row.get(3)?,
                relative_path: row.get(4)?,
                staging_path: row.get(5)?,
                deploy_method: row.get(6)?,
                sha256: row.get(7)?,
                deployed_at: row.get(8)?,
                mod_name: row.get(9)?,
                deploy_target: row
                    .get::<_, String>(10)
                    .unwrap_or_else(|_| "data".to_string()),
            })
        })?;

        let mut entries = Vec::new();
        for row in rows {
            entries.push(row?);
        }
        Ok(entries)
    }

    /// Get the deployment entry for a specific file path.
    pub fn get_deployed_file(
        &self,
        game_id: &str,
        bottle_name: &str,
        relative_path: &str,
    ) -> Result<Option<DeploymentEntry>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT dm.id, dm.game_id, dm.bottle_name, dm.mod_id, dm.relative_path,
                    dm.staging_path, dm.deploy_method, dm.sha256, dm.deployed_at,
                    COALESCE(im.name, 'Unknown') as mod_name,
                    dm.deploy_target
             FROM deployment_manifest dm
             LEFT JOIN installed_mods im ON dm.mod_id = im.id
             WHERE dm.game_id = ?1 AND dm.bottle_name = ?2 AND dm.relative_path = ?3",
        )?;

        let mut rows = stmt.query_map(params![game_id, bottle_name, relative_path], |row| {
            Ok(DeploymentEntry {
                id: row.get(0)?,
                game_id: row.get(1)?,
                bottle_name: row.get(2)?,
                mod_id: row.get(3)?,
                relative_path: row.get(4)?,
                staging_path: row.get(5)?,
                deploy_method: row.get(6)?,
                sha256: row.get(7)?,
                deployed_at: row.get(8)?,
                mod_name: row.get(9)?,
                deploy_target: row
                    .get::<_, String>(10)
                    .unwrap_or_else(|_| "data".to_string()),
            })
        })?;

        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    // -- File hashes --------------------------------------------------------

    /// Store file hashes for a mod's staging files.
    pub fn store_file_hashes(
        &self,
        mod_id: i64,
        hashes: &[(String, String, u64)], // (relative_path, sha256, file_size)
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "INSERT OR REPLACE INTO file_hashes (mod_id, relative_path, sha256, file_size)
             VALUES (?1, ?2, ?3, ?4)",
        )?;

        for (path, hash, size) in hashes {
            stmt.execute(params![mod_id, path, hash, *size as i64])?;
        }
        Ok(())
    }

    /// Get file hashes for a mod.
    pub fn get_file_hashes(&self, mod_id: i64) -> Result<Vec<(String, String, u64)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT relative_path, sha256, file_size FROM file_hashes WHERE mod_id = ?1",
        )?;

        let rows = stmt.query_map(params![mod_id], |row| {
            let size: i64 = row.get(2)?;
            Ok((row.get(0)?, row.get(1)?, size as u64))
        })?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    // -- Incremental deployment helpers --------------------------------------

    /// Return the deployment manifest as a HashMap keyed by relative_path
    /// for efficient diff computation during incremental deployment.
    pub fn get_deployment_manifest_map(
        &self,
        game_id: &str,
        bottle_name: &str,
    ) -> Result<HashMap<String, DeploymentEntry>> {
        let entries = self.get_deployment_manifest(game_id, bottle_name)?;
        let map = entries
            .into_iter()
            .map(|e| (e.relative_path.clone(), e))
            .collect();
        Ok(map)
    }

    /// Batch-insert deployment entries WITH sha256 values in a single transaction.
    ///
    /// Used by incremental deployment to record new/updated entries with their
    /// hash values from the file_hashes table. `deploy_target` is required and
    /// part of the UNIQUE key as of migration v24.
    ///
    /// Tuple fields:
    /// (game_id, bottle_name, mod_id, relative_path, staging_path,
    ///  deploy_method, sha256, deploy_target)
    #[allow(clippy::type_complexity)]
    pub fn batch_add_deployment_entries_with_hashes(
        &self,
        entries: &[(
            &str,         // game_id
            &str,         // bottle_name
            i64,          // mod_id
            &str,         // relative_path
            &str,         // staging_path
            &str,         // deploy_method
            Option<&str>, // sha256
            &str,         // deploy_target
        )],
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let deployed_at = chrono::Utc::now().to_rfc3339();
        let tx = conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO deployment_manifest
                    (game_id, bottle_name, mod_id, relative_path, staging_path, deploy_method, sha256, deployed_at, deploy_target)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            for (game_id, bottle_name, mod_id, rel_path, staging_path, method, sha256, deploy_target) in entries {
                stmt.execute(params![
                    game_id,
                    bottle_name,
                    mod_id,
                    rel_path,
                    staging_path,
                    method,
                    sha256,
                    deployed_at,
                    deploy_target,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Batch-remove deployment manifest entries by relative paths for a specific
    /// game/bottle in a single transaction.
    pub fn batch_remove_deployment_entries(
        &self,
        game_id: &str,
        bottle_name: &str,
        paths: &[&str],
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "DELETE FROM deployment_manifest
                 WHERE game_id = ?1 AND bottle_name = ?2 AND relative_path = ?3",
            )?;
            for path in paths {
                stmt.execute(params![game_id, bottle_name, path])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Batch-fetch file hashes for multiple mods in a single query.
    /// Returns a map of `(mod_id, relative_path) -> sha256`.
    pub fn get_file_hashes_bulk(&self, mod_ids: &[i64]) -> Result<HashMap<(i64, String), String>> {
        if mod_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());

        // Build dynamic IN clause with positional params
        let placeholders: Vec<String> = (1..=mod_ids.len()).map(|i| format!("?{}", i)).collect();
        let sql = format!(
            "SELECT mod_id, relative_path, sha256 FROM file_hashes WHERE mod_id IN ({})",
            placeholders.join(", ")
        );

        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> = mod_ids
            .iter()
            .map(|id| id as &dyn rusqlite::ToSql)
            .collect();

        let rows = stmt.query_map(params.as_slice(), |row| {
            let mod_id: i64 = row.get(0)?;
            let path: String = row.get(1)?;
            let sha256: String = row.get(2)?;
            Ok((mod_id, path, sha256))
        })?;

        let mut result = HashMap::new();
        for row in rows {
            let (mod_id, path, sha256) = row?;
            result.insert((mod_id, path), sha256);
        }
        Ok(result)
    }

    /// Alias for `get_file_hashes_bulk` — used by incremental deployment.
    pub fn get_file_hashes_for_mods(
        &self,
        mod_ids: &[i64],
    ) -> Result<HashMap<(i64, String), String>> {
        self.get_file_hashes_bulk(mod_ids)
    }

    // -- Mod field updates --------------------------------------------------

    /// Update the installed_files JSON for a mod.
    pub fn update_installed_files(&self, mod_id: i64, files: &[String]) -> Result<()> {
        let files_json = serde_json::to_string(files)?;
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE installed_mods SET installed_files = ?1 WHERE id = ?2",
            params![files_json, mod_id],
        )?;
        Ok(())
    }

    /// Get the next priority value for a game/bottle (current max + 1, or 0 if empty).
    pub fn get_next_priority(&self, game_id: &str, bottle_name: &str) -> Result<i32> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let max_priority: Option<i32> = conn
            .prepare(
                "SELECT MAX(install_priority) FROM installed_mods
                 WHERE game_id = ?1 AND bottle_name = ?2",
            )?
            .query_row(params![game_id, bottle_name], |row| row.get(0))?;
        Ok(max_priority.map_or(0, |p| p + 1))
    }

    // -- Conflicts ----------------------------------------------------------

    /// Find all file conflicts for a game/bottle.
    /// Tags each conflict with `same_collection` when all involved mods share
    /// the same collection — these are expected overlaps managed by the author.
    pub fn find_all_conflicts(
        &self,
        game_id: &str,
        bottle_name: &str,
    ) -> Result<Vec<FileConflict>> {
        let mods = self.list_mods(game_id, bottle_name)?;

        // Build mod_id → collection_name lookup
        let mod_collection: HashMap<i64, Option<String>> = mods
            .iter()
            .map(|m| (m.id, m.collection_name.clone()))
            .collect();

        let mut file_to_mods: HashMap<String, Vec<ConflictModInfo>> = HashMap::new();

        for m in &mods {
            if !m.enabled {
                continue;
            }
            for file in &m.installed_files {
                file_to_mods
                    .entry(file.clone())
                    .or_default()
                    .push(ConflictModInfo {
                        mod_id: m.id,
                        mod_name: m.name.clone(),
                        priority: m.install_priority,
                    });
            }
        }

        let mut conflicts = Vec::new();
        for (path, mods_info) in file_to_mods {
            if mods_info.len() > 1 {
                // Determine same_collection: all mods share one non-null collection name
                let collections: HashSet<Option<&str>> = mods_info
                    .iter()
                    .map(|m| mod_collection.get(&m.mod_id).and_then(|c| c.as_deref()))
                    .collect();
                let same_collection =
                    collections.len() == 1 && collections.iter().next().unwrap().is_some();

                let winner_mod_id = mods_info
                    .iter()
                    .max_by_key(|m| m.priority)
                    .map(|m| m.mod_id)
                    .unwrap_or(0);

                conflicts.push(FileConflict {
                    relative_path: path,
                    mods: mods_info,
                    winner_mod_id,
                    same_collection,
                });
            }
        }

        // Filter out resolved conflicts (covered by conflict_rules).
        let resolved = self.get_resolved_pairs(game_id, bottle_name)?;
        if !resolved.is_empty() {
            conflicts.retain(|c| {
                !c.mods
                    .iter()
                    .filter(|m| m.mod_id != c.winner_mod_id)
                    .all(|loser| resolved.contains(&(c.winner_mod_id, loser.mod_id)))
            });
        }

        conflicts.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        Ok(conflicts)
    }

    // -- Conflict rules ------------------------------------------------------

    /// Record that a conflict between two mods has been resolved.
    pub fn add_conflict_rule(
        &self,
        game_id: &str,
        bottle_name: &str,
        winner_mod_id: i64,
        loser_mod_id: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT OR REPLACE INTO conflict_rules
             (game_id, bottle_name, winner_mod_id, loser_mod_id, file_pattern)
             VALUES (?1, ?2, ?3, ?4, NULL)",
            params![game_id, bottle_name, winner_mod_id, loser_mod_id],
        )?;
        Ok(())
    }

    /// Get all resolved (winner, loser) pairs for a game/bottle.
    pub fn get_resolved_pairs(
        &self,
        game_id: &str,
        bottle_name: &str,
    ) -> Result<HashSet<(i64, i64)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT winner_mod_id, loser_mod_id FROM conflict_rules
             WHERE game_id = ?1 AND bottle_name = ?2",
        )?;
        let rows = stmt.query_map(params![game_id, bottle_name], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })?;
        let mut pairs = HashSet::new();
        for row in rows {
            pairs.insert(row?);
        }
        Ok(pairs)
    }

    /// Remove all conflict rules involving a specific mod (e.g. on uninstall).
    pub fn clear_conflict_rules_for_mod(&self, mod_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "DELETE FROM conflict_rules WHERE winner_mod_id = ?1 OR loser_mod_id = ?1",
            params![mod_id],
        )?;
        Ok(())
    }

    // -- Collection queries --------------------------------------------------

    /// List mods belonging to a specific collection.
    pub fn list_mods_by_collection(
        &self,
        game_id: &str,
        bottle_name: &str,
        collection_name: &str,
    ) -> Result<Vec<InstalledMod>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let sql = format!(
            "SELECT {} FROM installed_mods
             WHERE game_id = ?1 AND bottle_name = ?2 AND collection_name = ?3
             ORDER BY install_priority ASC",
            Self::SELECT_COLUMNS
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(
            params![game_id, bottle_name, collection_name],
            Self::row_to_mod,
        )?;
        let mut mods = Vec::new();
        for row in rows {
            mods.push(row?);
        }
        Ok(mods)
    }

    /// List all installed collections for a game/bottle.
    /// Returns (collection_name, total_count, enabled_count).
    pub fn list_installed_collections(
        &self,
        game_id: &str,
        bottle_name: &str,
    ) -> Result<Vec<(String, usize, usize)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT collection_name,
                    COUNT(*) as total,
                    SUM(CASE WHEN enabled = 1 THEN 1 ELSE 0 END) as enabled
             FROM installed_mods
             WHERE game_id = ?1 AND bottle_name = ?2 AND collection_name IS NOT NULL
             GROUP BY collection_name
             ORDER BY collection_name",
        )?;
        let rows = stmt.query_map(params![game_id, bottle_name], |row| {
            let name: String = row.get(0)?;
            let total: i64 = row.get(1)?;
            let enabled: i64 = row.get(2)?;
            Ok((name, total as usize, enabled as usize))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    // -- Download registry ---------------------------------------------------

    /// Register a downloaded archive in the download registry.
    #[allow(clippy::too_many_arguments)]
    pub fn register_download(
        &self,
        archive_path: &str,
        archive_name: &str,
        nexus_mod_id: Option<i64>,
        nexus_file_id: Option<i64>,
        sha256: Option<&str>,
        file_size: i64,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let downloaded_at = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT OR REPLACE INTO download_registry
                (archive_path, archive_name, nexus_mod_id, nexus_file_id, sha256, file_size, downloaded_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![archive_path, archive_name, nexus_mod_id, nexus_file_id, sha256, file_size, downloaded_at],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Find a download by Nexus mod and file IDs.
    pub fn find_download_by_nexus_ids(
        &self,
        nexus_mod_id: i64,
        nexus_file_id: i64,
    ) -> Result<Option<DownloadRecord>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, archive_path, archive_name, nexus_mod_id, nexus_file_id,
                    sha256, file_size, downloaded_at
             FROM download_registry
             WHERE nexus_mod_id = ?1 AND nexus_file_id = ?2",
        )?;
        let mut rows = stmt.query_map(params![nexus_mod_id, nexus_file_id], |row| {
            Ok(DownloadRecord {
                id: row.get(0)?,
                archive_path: row.get(1)?,
                archive_name: row.get(2)?,
                nexus_mod_id: row.get(3)?,
                nexus_file_id: row.get(4)?,
                sha256: row.get(5)?,
                file_size: row.get(6)?,
                downloaded_at: row.get(7)?,
            })
        })?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Find a download by archive name.
    pub fn find_download_by_name(&self, archive_name: &str) -> Result<Option<DownloadRecord>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, archive_path, archive_name, nexus_mod_id, nexus_file_id,
                    sha256, file_size, downloaded_at
             FROM download_registry
             WHERE archive_name = ?1",
        )?;
        let mut rows = stmt.query_map(params![archive_name], |row| {
            Ok(DownloadRecord {
                id: row.get(0)?,
                archive_path: row.get(1)?,
                archive_name: row.get(2)?,
                nexus_mod_id: row.get(3)?,
                nexus_file_id: row.get(4)?,
                sha256: row.get(5)?,
                file_size: row.get(6)?,
                downloaded_at: row.get(7)?,
            })
        })?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Add a reference linking a download to a collection.
    pub fn add_download_collection_ref(
        &self,
        download_id: i64,
        collection_name: &str,
        game_id: &str,
        bottle_name: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT OR IGNORE INTO download_collection_refs
                (download_id, collection_name, game_id, bottle_name)
             VALUES (?1, ?2, ?3, ?4)",
            params![download_id, collection_name, game_id, bottle_name],
        )?;
        Ok(())
    }

    /// Check if a download is only referenced by one collection.
    pub fn is_download_unique_to_collection(
        &self,
        download_id: i64,
        collection_name: &str,
    ) -> Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let count: i64 = conn
            .prepare(
                "SELECT COUNT(DISTINCT collection_name) FROM download_collection_refs
                 WHERE download_id = ?1 AND collection_name != ?2",
            )?
            .query_row(params![download_id, collection_name], |row| row.get(0))?;
        Ok(count == 0)
    }

    /// Remove a collection's reference to a download.
    pub fn remove_download_collection_ref(
        &self,
        download_id: i64,
        collection_name: &str,
        game_id: &str,
        bottle_name: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "DELETE FROM download_collection_refs
             WHERE download_id = ?1 AND collection_name = ?2 AND game_id = ?3 AND bottle_name = ?4",
            params![download_id, collection_name, game_id, bottle_name],
        )?;
        Ok(())
    }

    /// Get the total size of downloads unique to a collection (i.e. not shared with other collections).
    pub fn collection_unique_download_size(
        &self,
        game_id: &str,
        bottle_name: &str,
        collection_name: &str,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        // Sum file_size for downloads that are ONLY referenced by this collection
        let size: i64 = conn
            .prepare(
                "SELECT COALESCE(SUM(dr.file_size), 0)
             FROM download_registry dr
             JOIN download_collection_refs dcr ON dcr.download_id = dr.id
             WHERE dcr.collection_name = ?1
               AND dcr.game_id = ?2
               AND dcr.bottle_name = ?3
               AND dr.id NOT IN (
                   SELECT download_id FROM download_collection_refs
                   WHERE collection_name != ?1
               )",
            )?
            .query_row(params![collection_name, game_id, bottle_name], |row| {
                row.get(0)
            })?;
        Ok(size)
    }

    /// Get all downloads unique to a collection (not shared with any other collection).
    /// Returns (download_id, archive_path) pairs for deletion.
    pub fn get_unique_downloads_for_collection(
        &self,
        game_id: &str,
        bottle_name: &str,
        collection_name: &str,
    ) -> Result<Vec<(i64, String)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT dr.id, dr.archive_path
             FROM download_registry dr
             JOIN download_collection_refs dcr ON dcr.download_id = dr.id
             WHERE dcr.collection_name = ?1
               AND dcr.game_id = ?2
               AND dcr.bottle_name = ?3
               AND dr.id NOT IN (
                   SELECT download_id FROM download_collection_refs
                   WHERE collection_name != ?1
               )",
        )?;
        let rows = stmt.query_map(params![collection_name, game_id, bottle_name], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Remove all collection refs for a given collection.
    pub fn remove_all_collection_download_refs(
        &self,
        collection_name: &str,
        game_id: &str,
        bottle_name: &str,
    ) -> Result<usize> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let deleted = conn.execute(
            "DELETE FROM download_collection_refs
             WHERE collection_name = ?1 AND game_id = ?2 AND bottle_name = ?3",
            params![collection_name, game_id, bottle_name],
        )?;
        Ok(deleted)
    }

    /// Batch-check which (nexus_mod_id, nexus_file_id) pairs exist in the download registry.
    /// Returns the subset of input pairs that have a matching downloaded file on disk.
    pub fn batch_check_cached_files(&self, pairs: &[(i64, i64)]) -> Result<Vec<(i64, i64)>> {
        if pairs.is_empty() {
            return Ok(Vec::new());
        }

        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());

        // Load all cached (mod_id, file_id, archive_path) tuples
        let mut cached: std::collections::HashSet<(i64, i64)> = std::collections::HashSet::new();
        let mut stale_ids: Vec<i64> = Vec::new();

        let mut stmt = conn.prepare(
            "SELECT id, nexus_mod_id, nexus_file_id, archive_path
             FROM download_registry
             WHERE nexus_mod_id IS NOT NULL AND nexus_file_id IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        for (id, mod_id, file_id, path) in rows.flatten() {
            if std::path::Path::new(&path).exists() {
                cached.insert((mod_id, file_id));
            } else {
                stale_ids.push(id);
            }
        }

        // Clean up stale entries whose files no longer exist on disk
        if !stale_ids.is_empty() {
            for id in &stale_ids {
                let _ = conn.execute("DELETE FROM download_registry WHERE id = ?1", params![id]);
            }
        }

        // Return only the input pairs that exist in the cache
        let matched: Vec<(i64, i64)> = pairs
            .iter()
            .filter(|p| cached.contains(p))
            .cloned()
            .collect();

        Ok(matched)
    }

    /// Delete orphaned download_registry rows that have no collection references.
    pub fn cleanup_orphaned_downloads(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let deleted = conn.execute(
            "DELETE FROM download_registry WHERE id NOT IN
             (SELECT DISTINCT download_id FROM download_collection_refs)",
            [],
        )?;
        Ok(deleted)
    }

    /// Delete a specific download_registry entry by ID.
    ///
    /// Both the registry row and its collection refs are deleted in a single
    /// transaction to prevent orphaned refs on partial failure.
    pub fn delete_download_record(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.unchecked_transaction()?;
        tx.execute("DELETE FROM download_registry WHERE id = ?1", params![id])?;
        // Also clean up any collection refs pointing to this download
        tx.execute(
            "DELETE FROM download_collection_refs WHERE download_id = ?1",
            params![id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Get all distinct archive_name values from installed_mods.
    pub fn get_all_archive_names(&self) -> Result<HashSet<String>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt =
            conn.prepare("SELECT DISTINCT archive_name FROM installed_mods WHERE archive_name != ''")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut names = HashSet::new();
        for row in rows {
            names.insert(row?);
        }
        Ok(names)
    }

    /// Get all download records from the download_registry.
    pub fn get_all_download_records(&self) -> Result<Vec<DownloadRecord>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, archive_path, archive_name, nexus_mod_id, nexus_file_id,
                    sha256, file_size, downloaded_at
             FROM download_registry",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(DownloadRecord {
                id: row.get(0)?,
                archive_path: row.get(1)?,
                archive_name: row.get(2)?,
                nexus_mod_id: row.get(3)?,
                nexus_file_id: row.get(4)?,
                sha256: row.get(5)?,
                file_size: row.get(6)?,
                downloaded_at: row.get(7)?,
            })
        })?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        Ok(records)
    }

    /// Get all download IDs that have collection references.
    pub fn get_download_ids_with_refs(&self) -> Result<HashSet<i64>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt =
            conn.prepare("SELECT DISTINCT download_id FROM download_collection_refs")?;
        let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
        let mut ids = HashSet::new();
        for row in rows {
            ids.insert(row?);
        }
        Ok(ids)
    }

    // -- Notes & tags --------------------------------------------------------

    /// Set user notes for a mod.
    pub fn set_user_notes(&self, mod_id: i64, notes: Option<&str>) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE installed_mods SET user_notes = ?1 WHERE id = ?2",
            params![notes, mod_id],
        )?;
        Ok(())
    }

    /// Set user tags for a mod (stored as JSON array).
    pub fn set_user_tags(&self, mod_id: i64, tags: &[String]) -> Result<()> {
        let tags_json = serde_json::to_string(tags)?;
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE installed_mods SET user_tags = ?1 WHERE id = ?2",
            params![tags_json, mod_id],
        )?;
        Ok(())
    }

    /// Get all unique user tags for a game/bottle.
    pub fn get_all_user_tags(&self, game_id: &str, bottle_name: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT user_tags FROM installed_mods
             WHERE game_id = ?1 AND bottle_name = ?2 AND user_tags IS NOT NULL",
        )?;
        let rows = stmt.query_map(params![game_id, bottle_name], |row| row.get::<_, String>(0))?;

        let mut all_tags = std::collections::HashSet::new();
        for row in rows {
            let tags_json = row?;
            if let Ok(tags) = serde_json::from_str::<Vec<String>>(&tags_json) {
                for tag in tags {
                    all_tags.insert(tag);
                }
            }
        }
        let mut sorted: Vec<String> = all_tags.into_iter().collect();
        sorted.sort();
        Ok(sorted)
    }
}

// ---------------------------------------------------------------------------
// Collection metadata
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CollectionMetadata {
    pub id: i64,
    pub collection_name: String,
    pub game_id: String,
    pub bottle_name: String,
    pub slug: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    pub game_domain: Option<String>,
    pub image_url: Option<String>,
    pub installed_revision: Option<u32>,
    pub total_mods: Option<usize>,
    pub installed_at: String,
    pub manifest_json: Option<String>,
}

impl ModDatabase {
    /// Save or update collection metadata (upsert by collection_name + game_id + bottle_name).
    pub fn save_collection_metadata(&self, meta: &CollectionMetadata) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT OR REPLACE INTO collection_metadata
                (collection_name, game_id, bottle_name, slug, author, description,
                 game_domain, image_url, installed_revision, total_mods, installed_at, manifest_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                meta.collection_name,
                meta.game_id,
                meta.bottle_name,
                meta.slug,
                meta.author,
                meta.description,
                meta.game_domain,
                meta.image_url,
                meta.installed_revision.map(|v| v as i64),
                meta.total_mods.map(|v| v as i64),
                meta.installed_at,
                meta.manifest_json,
            ],
        )?;
        Ok(())
    }

    /// Get metadata for a specific installed collection.
    pub fn get_collection_metadata(
        &self,
        game_id: &str,
        bottle_name: &str,
        collection_name: &str,
    ) -> Result<Option<CollectionMetadata>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, collection_name, game_id, bottle_name, slug, author, description,
                    game_domain, image_url, installed_revision, total_mods, installed_at, manifest_json
             FROM collection_metadata
             WHERE game_id = ?1 AND bottle_name = ?2 AND collection_name = ?3",
        )?;
        let result = stmt.query_row(params![game_id, bottle_name, collection_name], |row| {
            Ok(CollectionMetadata {
                id: row.get(0)?,
                collection_name: row.get(1)?,
                game_id: row.get(2)?,
                bottle_name: row.get(3)?,
                slug: row.get(4)?,
                author: row.get(5)?,
                description: row.get(6)?,
                game_domain: row.get(7)?,
                image_url: row.get(8)?,
                installed_revision: row.get::<_, Option<i64>>(9)?.map(|v| v as u32),
                total_mods: row.get::<_, Option<i64>>(10)?.map(|v| v as usize),
                installed_at: row.get(11)?,
                manifest_json: row.get(12)?,
            })
        });
        match result {
            Ok(meta) => Ok(Some(meta)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// List all collection metadata for a game/bottle.
    pub fn list_collection_metadata(
        &self,
        game_id: &str,
        bottle_name: &str,
    ) -> Result<Vec<CollectionMetadata>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, collection_name, game_id, bottle_name, slug, author, description,
                    game_domain, image_url, installed_revision, total_mods, installed_at, manifest_json
             FROM collection_metadata
             WHERE game_id = ?1 AND bottle_name = ?2
             ORDER BY installed_at DESC",
        )?;
        let rows = stmt.query_map(params![game_id, bottle_name], |row| {
            Ok(CollectionMetadata {
                id: row.get(0)?,
                collection_name: row.get(1)?,
                game_id: row.get(2)?,
                bottle_name: row.get(3)?,
                slug: row.get(4)?,
                author: row.get(5)?,
                description: row.get(6)?,
                game_domain: row.get(7)?,
                image_url: row.get(8)?,
                installed_revision: row.get::<_, Option<i64>>(9)?.map(|v| v as u32),
                total_mods: row.get::<_, Option<i64>>(10)?.map(|v| v as usize),
                installed_at: row.get(11)?,
                manifest_json: row.get(12)?,
            })
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Remove collection metadata.
    pub fn remove_collection_metadata(
        &self,
        game_id: &str,
        bottle_name: &str,
        collection_name: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "DELETE FROM collection_metadata
             WHERE game_id = ?1 AND bottle_name = ?2 AND collection_name = ?3",
            params![game_id, bottle_name, collection_name],
        )?;
        Ok(())
    }

    // -- Auto-category -------------------------------------------------------

    /// Classify a mod into a category based on its installed file paths.
    pub fn classify_mod_category(installed_files: &[String]) -> Option<String> {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        let mut has_plugin_file = false;

        for file in installed_files {
            let lower = file.to_lowercase();

            // Track whether the mod ships any .esp/.esm/.esl
            if lower.ends_with(".esp") || lower.ends_with(".esm") || lower.ends_with(".esl") {
                has_plugin_file = true;
            }

            let cat = if lower.contains("skse/plugins/") || lower.contains("skse\\plugins\\") {
                "Framework"
            } else if lower.contains("enbseries")
                || lower.starts_with("d3d")
                || lower.contains("shaderfx")
                || lower.contains("reshade")
            {
                "Lighting & Weather"
            } else if lower.contains("textures/")
                || lower.contains("textures\\")
                || lower.ends_with(".dds")
            {
                "Texture"
            } else if lower.contains("meshes/")
                || lower.contains("meshes\\")
                || lower.ends_with(".nif")
            {
                "Model & Mesh"
            } else if lower.contains("interface/")
                || lower.contains("interface\\")
                || lower.ends_with(".swf")
            {
                "UI"
            } else if lower.contains("sound/")
                || lower.contains("sound\\")
                || lower.contains("music/")
                || lower.contains("music\\")
            {
                "Audio"
            } else if lower.contains("animations/")
                || lower.contains("animations\\")
                || lower.ends_with(".hkx")
            {
                "Animation"
            } else if (lower.contains("scripts/") || lower.contains("scripts\\"))
                && lower.ends_with(".pex")
            {
                "Script"
            } else if lower.contains("seq/") || lower.contains("seq\\") {
                "Gameplay"
            } else {
                "Misc"
            };
            *counts.entry(cat).or_insert(0) += 1;
        }

        // Pick the dominant non-Misc category
        let best = counts
            .into_iter()
            .filter(|(cat, _)| *cat != "Misc")
            .max_by_key(|(_, count)| *count)
            .map(|(cat, _)| cat.to_string());

        match best {
            Some(cat) => {
                // If the only content category is Script and there's a plugin file,
                // the mod is really a gameplay mod that happens to use scripts.
                if cat == "Script" && has_plugin_file {
                    Some("Gameplay".to_string())
                } else {
                    Some(cat)
                }
            }
            None => {
                // No strong content signal — if it has a plugin file it's a gameplay mod
                if has_plugin_file {
                    Some("Gameplay".to_string())
                } else {
                    Some("Misc".to_string())
                }
            }
        }
    }

    /// Set the auto_category for a mod.
    pub fn set_auto_category(&self, mod_id: i64, category: Option<&str>) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let rows = conn.execute(
            "UPDATE installed_mods SET auto_category = ?1 WHERE id = ?2",
            params![category, mod_id],
        )?;
        if rows == 0 {
            return Err(DatabaseError::ModNotFound(mod_id));
        }
        Ok(())
    }

    /// Get all distinct categories with counts for a game/bottle.
    pub fn get_categories(&self, game_id: &str, bottle_name: &str) -> Result<Vec<(String, usize)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT COALESCE(auto_category, 'Uncategorized'), COUNT(*)
             FROM installed_mods
             WHERE game_id = ?1 AND bottle_name = ?2
             GROUP BY COALESCE(auto_category, 'Uncategorized')
             ORDER BY COUNT(*) DESC",
        )?;

        let rows = stmt.query_map(params![game_id, bottle_name], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
        })?;

        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Reclassify auto_category for all mods (updates stale names too).
    pub fn backfill_categories(&self, game_id: &str, bottle_name: &str) -> Result<usize> {
        let mods = self.list_mods(game_id, bottle_name)?;
        let mut updated = 0;
        for m in &mods {
            if let Some(cat) = Self::classify_mod_category(&m.installed_files) {
                if m.auto_category.as_deref() != Some(&cat) {
                    self.set_auto_category(m.id, Some(&cat))?;
                    updated += 1;
                }
            }
        }
        Ok(updated)
    }

    /// Backfill source_type for mods that have nexus_mod_id but source_type
    /// is still "manual" (legacy data from before source tracking was wired up).
    pub fn backfill_source_types(&self, game_id: &str, bottle_name: &str) -> Result<usize> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let updated = conn.execute(
            "UPDATE installed_mods SET source_type = 'nexus'
             WHERE game_id = ?1 AND bottle_name = ?2
               AND nexus_mod_id IS NOT NULL
               AND (source_type = 'manual' OR source_type IS NULL)",
            params![game_id, bottle_name],
        )?;
        Ok(updated)
    }

    // -- Notification log ----------------------------------------------------

    /// Log a notification to the persistent notification table.
    pub fn log_notification(&self, level: &str, message: &str, detail: Option<&str>) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO notification_log (level, message, detail, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![level, message, detail, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// Get recent notifications, most recent first.
    pub fn get_notifications(&self, limit: usize) -> Result<Vec<NotificationEntry>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, level, message, detail, created_at
             FROM notification_log
             ORDER BY created_at DESC
             LIMIT ?1",
        )?;

        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(NotificationEntry {
                id: row.get(0)?,
                level: row.get(1)?,
                message: row.get(2)?,
                detail: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?;

        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Clear all notifications.
    pub fn clear_notifications(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute("DELETE FROM notification_log", [])?;
        Ok(())
    }

    /// Get notification count.
    pub fn notification_count(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let count: i64 = conn
            .prepare("SELECT COUNT(*) FROM notification_log")?
            .query_row([], |row| row.get(0))?;
        Ok(count as usize)
    }
}

// ---------------------------------------------------------------------------
// Error Event Diagnostics
// ---------------------------------------------------------------------------

/// A structured error event for proactive diagnostics.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ErrorEvent {
    pub id: i64,
    pub timestamp: String,
    pub module: String,
    pub error_type: String,
    pub message: String,
    pub count: i64,
    pub first_seen: String,
    pub last_seen: String,
    pub resolved: bool,
}

impl ModDatabase {
    /// Record a diagnostic error event. If the same (module, error_type, message) was seen
    /// within the last hour and is unresolved, increment its count; otherwise insert a new row.
    pub fn record_error_event(
        &self,
        module: &str,
        error_type: &str,
        message: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());

        // Try to find an existing unresolved row within the last hour
        let existing: Option<i64> = conn
            .prepare(
                "SELECT id FROM error_events
                 WHERE module = ?1 AND error_type = ?2 AND message = ?3
                   AND resolved = 0 AND last_seen > datetime('now', '-1 hour')
                 LIMIT 1",
            )?
            .query_row(params![module, error_type, message], |row| row.get(0))
            .ok();

        if let Some(id) = existing {
            conn.execute(
                "UPDATE error_events SET count = count + 1, last_seen = datetime('now') WHERE id = ?1",
                params![id],
            )?;
        } else {
            conn.execute(
                "INSERT INTO error_events (module, error_type, message) VALUES (?1, ?2, ?3)",
                params![module, error_type, message],
            )?;
        }
        Ok(())
    }

    /// Get a summary of top unresolved error events by count.
    pub fn get_error_summary(&self, limit: u32) -> Result<Vec<ErrorEvent>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, timestamp, module, error_type, message, count, first_seen, last_seen, resolved
             FROM error_events
             WHERE resolved = 0
             ORDER BY count DESC, last_seen DESC
             LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit], |row| {
                Ok(ErrorEvent {
                    id: row.get(0)?,
                    timestamp: row.get(1)?,
                    module: row.get(2)?,
                    error_type: row.get(3)?,
                    message: row.get(4)?,
                    count: row.get(5)?,
                    first_seen: row.get(6)?,
                    last_seen: row.get(7)?,
                    resolved: row.get::<_, i64>(8)? != 0,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Mark an error event as resolved.
    pub fn resolve_error_event(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE error_events SET resolved = 1 WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Download Queue Persistence
// ---------------------------------------------------------------------------

impl ModDatabase {
    /// Save a queue item to the database.
    pub fn save_queue_item(&self, item: &crate::download_queue::QueueItem) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let status_str = match item.status {
            crate::download_queue::DownloadStatus::Pending => "pending",
            crate::download_queue::DownloadStatus::Downloading => "downloading",
            crate::download_queue::DownloadStatus::Completed => "completed",
            crate::download_queue::DownloadStatus::Failed => "failed",
            crate::download_queue::DownloadStatus::Cancelled => "cancelled",
        };
        conn.execute(
            "INSERT OR REPLACE INTO download_queue
                (id, mod_name, file_name, status, error, attempt, max_attempts,
                 downloaded_bytes, total_bytes, nexus_mod_id, nexus_file_id,
                 url, game_slug, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, datetime('now'))",
            params![
                item.id as i64,
                item.mod_name,
                item.file_name,
                status_str,
                item.error,
                item.attempt,
                item.max_attempts,
                item.downloaded_bytes as i64,
                item.total_bytes as i64,
                item.nexus_mod_id,
                item.nexus_file_id,
                item.url,
                item.game_slug,
            ],
        )?;
        Ok(())
    }

    /// Load all non-completed/non-cancelled queue items from the database.
    /// Items that were "downloading" are reset to "pending" since the download
    /// was interrupted by the app closing.
    pub fn load_queue_items(&self) -> Result<Vec<crate::download_queue::QueueItem>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, mod_name, file_name, status, error, attempt, max_attempts,
                    downloaded_bytes, total_bytes, nexus_mod_id, nexus_file_id, url, game_slug
             FROM download_queue
             WHERE status NOT IN ('completed', 'cancelled')
             ORDER BY id",
        )?;

        let items = stmt
            .query_map([], |row| {
                let status_str: String = row.get(3)?;
                let status = match status_str.as_str() {
                    "downloading" | "pending" => crate::download_queue::DownloadStatus::Pending,
                    "failed" => crate::download_queue::DownloadStatus::Failed,
                    _ => crate::download_queue::DownloadStatus::Pending,
                };
                Ok(crate::download_queue::QueueItem {
                    id: row.get::<_, i64>(0)? as u64,
                    mod_name: row.get(1)?,
                    file_name: row.get(2)?,
                    status,
                    error: row.get(4)?,
                    attempt: row.get(5)?,
                    max_attempts: row.get(6)?,
                    downloaded_bytes: 0, // Reset on restart
                    total_bytes: row.get::<_, i64>(8)? as u64,
                    nexus_mod_id: row.get(9)?,
                    nexus_file_id: row.get(10)?,
                    url: row.get(11)?,
                    game_slug: row.get(12)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        // Reset any "downloading" items back to "pending" in the DB
        conn.execute(
            "UPDATE download_queue SET status = 'pending', downloaded_bytes = 0 WHERE status = 'downloading'",
            [],
        )?;

        Ok(items)
    }

    /// Remove completed and cancelled queue items from the database.
    pub fn clear_finished_queue_items(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let count = conn.execute(
            "DELETE FROM download_queue WHERE status IN ('completed', 'cancelled')",
            [],
        )?;
        Ok(count)
    }

    // -----------------------------------------------------------------------
    // Wabbajack install pipeline
    // -----------------------------------------------------------------------

    /// Create a new Wabbajack install record.
    pub fn create_wj_install(
        &self,
        modlist_name: &str,
        modlist_version: &str,
        game_type: u32,
        install_dir: &str,
        total_archives: usize,
        total_directives: usize,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        // Mark any existing pending/failed/downloading/extracting installs for this
        // modlist as cancelled to prevent duplicates on resume/retry.
        conn.execute(
            "UPDATE wabbajack_installs SET status = 'cancelled'
             WHERE modlist_name = ?1 AND status IN ('pending', 'failed', 'downloading', 'extracting', 'processing')",
            params![modlist_name],
        )?;
        conn.execute(
            "INSERT INTO wabbajack_installs
                (modlist_name, modlist_version, game_type, install_dir, status,
                 total_archives, total_directives)
             VALUES (?1, ?2, ?3, ?4, 'pending', ?5, ?6)",
            params![
                modlist_name,
                modlist_version,
                game_type,
                install_dir,
                total_archives as i64,
                total_directives as i64,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Update the status of a Wabbajack install.
    pub fn update_wj_install_status(
        &self,
        install_id: i64,
        status: &str,
        error_message: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE wabbajack_installs SET status = ?1, error_message = ?2,
                    updated_at = datetime('now') WHERE id = ?3",
            params![status, error_message, install_id],
        )?;
        Ok(())
    }

    /// Update the completed archive count for a Wabbajack install.
    pub fn update_wj_install_archive_progress(
        &self,
        install_id: i64,
        completed: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE wabbajack_installs SET completed_archives = ?1,
                    updated_at = datetime('now') WHERE id = ?2",
            params![completed, install_id],
        )?;
        Ok(())
    }

    /// Update the completed directive count for a Wabbajack install.
    pub fn update_wj_install_directive_progress(
        &self,
        install_id: i64,
        completed: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE wabbajack_installs SET completed_directives = ?1,
                    updated_at = datetime('now') WHERE id = ?2",
            params![completed, install_id],
        )?;
        Ok(())
    }

    /// Insert or update archive download status for a Wabbajack install.
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_wj_archive_status(
        &self,
        install_id: i64,
        archive_hash: &str,
        archive_name: &str,
        source_type: &str,
        status: &str,
        download_path: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO wabbajack_archive_status
                (install_id, archive_hash, archive_name, source_type, status, download_path, error_message)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(install_id, archive_hash)
             DO UPDATE SET status = ?5, download_path = ?6, error_message = ?7",
            params![
                install_id,
                archive_hash,
                archive_name,
                source_type,
                status,
                download_path,
                error_message,
            ],
        )?;
        Ok(())
    }

    /// List all archive statuses for a Wabbajack install.
    #[allow(clippy::type_complexity)]
    pub fn list_wj_archive_status(
        &self,
        install_id: i64,
    ) -> Result<Vec<(String, String, String, Option<String>)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT archive_hash, archive_name, status, download_path
             FROM wabbajack_archive_status WHERE install_id = ?1",
        )?;
        let rows = stmt
            .query_map(params![install_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Look up a file in the download registry by xxhash64.
    pub fn find_download_by_xxhash(&self, xxhash64: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let result = conn
            .prepare("SELECT file_path FROM download_registry WHERE xxhash64 = ?1 LIMIT 1")?
            .query_row(params![xxhash64], |row| row.get::<_, String>(0))
            .ok();
        Ok(result)
    }

    /// Set the xxhash64 for a download registry entry.
    pub fn set_download_xxhash(&self, file_path: &str, xxhash64: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE download_registry SET xxhash64 = ?1 WHERE file_path = ?2",
            params![xxhash64, file_path],
        )?;
        Ok(())
    }

    /// Get the install status for a Wabbajack install.
    #[allow(clippy::type_complexity)]
    pub fn get_wj_install_status(
        &self,
        install_id: i64,
    ) -> Result<Option<(String, i64, i64, i64, i64, Option<String>)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let result = conn
            .prepare(
                "SELECT status, total_archives, completed_archives,
                        total_directives, completed_directives, error_message
                 FROM wabbajack_installs WHERE id = ?1",
            )?
            .query_row(params![install_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })
            .ok();
        Ok(result)
    }

    // -----------------------------------------------------------------------
    // Collection Install Checkpoints
    // -----------------------------------------------------------------------

    /// Create or replace a collection install checkpoint.
    pub fn create_collection_checkpoint(
        &self,
        collection_name: &str,
        game_id: &str,
        bottle_name: &str,
        manifest_json: &str,
        total_mods: usize,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT OR REPLACE INTO collection_install_checkpoints
                (collection_name, game_id, bottle_name, manifest_json, status,
                 total_mods, completed_mods, failed_mods, skipped_mods, mod_statuses,
                 created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 'in_progress', ?5, 0, 0, 0, '{}',
                     datetime('now'), datetime('now'))",
            params![
                collection_name,
                game_id,
                bottle_name,
                manifest_json,
                total_mods as i64,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Update the status of a single mod within a checkpoint.
    pub fn update_checkpoint_mod_status(
        &self,
        checkpoint_id: i64,
        mod_index: usize,
        status: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.unchecked_transaction()?;

        // Read current mod_statuses JSON
        let current_json: String = tx
            .prepare("SELECT mod_statuses FROM collection_install_checkpoints WHERE id = ?1")?
            .query_row(params![checkpoint_id], |row| row.get(0))
            .unwrap_or_else(|_| "{}".to_string());

        let mut statuses: std::collections::HashMap<String, String> =
            serde_json::from_str(&current_json).unwrap_or_default();

        // Check if this index was already counted (for counter adjustment)
        let prev_status = statuses.get(&mod_index.to_string()).cloned();
        statuses.insert(mod_index.to_string(), status.to_string());

        let new_json = serde_json::to_string(&statuses).unwrap_or_else(|_| "{}".to_string());

        // Calculate counter deltas
        let is_complete = |s: &str| matches!(s, "installed" | "already_installed");
        let is_failed = |s: &str| s == "failed";
        let is_skipped = |s: &str| matches!(s, "skipped" | "user_action");

        let mut completed_delta: i64 = 0;
        let mut failed_delta: i64 = 0;
        let mut skipped_delta: i64 = 0;

        // Subtract previous status contribution
        if let Some(ref prev) = prev_status {
            if is_complete(prev) {
                completed_delta -= 1;
            }
            if is_failed(prev) {
                failed_delta -= 1;
            }
            if is_skipped(prev) {
                skipped_delta -= 1;
            }
        }

        // Add new status contribution
        if is_complete(status) {
            completed_delta += 1;
        }
        if is_failed(status) {
            failed_delta += 1;
        }
        if is_skipped(status) {
            skipped_delta += 1;
        }

        tx.execute(
            "UPDATE collection_install_checkpoints
             SET mod_statuses = ?1,
                 completed_mods = MAX(0, completed_mods + ?2),
                 failed_mods = MAX(0, failed_mods + ?3),
                 skipped_mods = MAX(0, skipped_mods + ?4),
                 updated_at = datetime('now')
             WHERE id = ?5",
            params![
                new_json,
                completed_delta,
                failed_delta,
                skipped_delta,
                checkpoint_id
            ],
        )?;

        tx.commit()?;
        Ok(())
    }

    /// Get the active (in_progress) checkpoint for a game/bottle pair.
    pub fn get_active_checkpoint(
        &self,
        game_id: &str,
        bottle_name: &str,
    ) -> Result<Option<CollectionInstallCheckpoint>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let result = conn
            .prepare(
                "SELECT id, collection_name, game_id, bottle_name, manifest_json,
                        status, total_mods, completed_mods, failed_mods, skipped_mods,
                        mod_statuses, error_message, created_at, updated_at
                 FROM collection_install_checkpoints
                 WHERE game_id = ?1 AND bottle_name = ?2 AND status = 'in_progress'
                 ORDER BY updated_at DESC LIMIT 1",
            )?
            .query_row(params![game_id, bottle_name], |row| {
                Ok(CollectionInstallCheckpoint {
                    id: row.get(0)?,
                    collection_name: row.get(1)?,
                    game_id: row.get(2)?,
                    bottle_name: row.get(3)?,
                    manifest_json: row.get(4)?,
                    status: row.get(5)?,
                    total_mods: row.get(6)?,
                    completed_mods: row.get(7)?,
                    failed_mods: row.get(8)?,
                    skipped_mods: row.get(9)?,
                    mod_statuses: row.get(10)?,
                    error_message: row.get(11)?,
                    created_at: row.get(12)?,
                    updated_at: row.get(13)?,
                })
            })
            .ok();
        Ok(result)
    }

    /// Get a checkpoint by its ID (regardless of status).
    pub fn get_active_checkpoint_by_id(
        &self,
        checkpoint_id: i64,
    ) -> Result<Option<CollectionInstallCheckpoint>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let result = conn
            .prepare(
                "SELECT id, collection_name, game_id, bottle_name, manifest_json,
                        status, total_mods, completed_mods, failed_mods, skipped_mods,
                        mod_statuses, error_message, created_at, updated_at
                 FROM collection_install_checkpoints
                 WHERE id = ?1 AND status = 'in_progress'",
            )?
            .query_row(params![checkpoint_id], |row| {
                Ok(CollectionInstallCheckpoint {
                    id: row.get(0)?,
                    collection_name: row.get(1)?,
                    game_id: row.get(2)?,
                    bottle_name: row.get(3)?,
                    manifest_json: row.get(4)?,
                    status: row.get(5)?,
                    total_mods: row.get(6)?,
                    completed_mods: row.get(7)?,
                    failed_mods: row.get(8)?,
                    skipped_mods: row.get(9)?,
                    mod_statuses: row.get(10)?,
                    error_message: row.get(11)?,
                    created_at: row.get(12)?,
                    updated_at: row.get(13)?,
                })
            })
            .ok();
        Ok(result)
    }

    /// Get all in-progress checkpoints (across all games/bottles).
    /// Used at app startup to detect interrupted installs.
    pub fn get_all_active_checkpoints(&self) -> Result<Vec<CollectionInstallCheckpoint>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, collection_name, game_id, bottle_name, manifest_json,
                    status, total_mods, completed_mods, failed_mods, skipped_mods,
                    mod_statuses, error_message, created_at, updated_at
             FROM collection_install_checkpoints
             WHERE status = 'in_progress'
             ORDER BY updated_at DESC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(CollectionInstallCheckpoint {
                    id: row.get(0)?,
                    collection_name: row.get(1)?,
                    game_id: row.get(2)?,
                    bottle_name: row.get(3)?,
                    manifest_json: row.get(4)?,
                    status: row.get(5)?,
                    total_mods: row.get(6)?,
                    completed_mods: row.get(7)?,
                    failed_mods: row.get(8)?,
                    skipped_mods: row.get(9)?,
                    mod_statuses: row.get(10)?,
                    error_message: row.get(11)?,
                    created_at: row.get(12)?,
                    updated_at: row.get(13)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Mark a checkpoint as completed.
    pub fn complete_checkpoint(&self, checkpoint_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE collection_install_checkpoints
             SET status = 'completed', updated_at = datetime('now')
             WHERE id = ?1",
            params![checkpoint_id],
        )?;
        Ok(())
    }

    /// Mark a checkpoint as abandoned (user chose to dismiss).
    pub fn abandon_checkpoint(&self, checkpoint_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE collection_install_checkpoints
             SET status = 'abandoned', updated_at = datetime('now')
             WHERE id = ?1",
            params![checkpoint_id],
        )?;
        Ok(())
    }

    /// Delete all checkpoints for a given collection (used when collection is deleted).
    pub fn delete_collection_checkpoints(
        &self,
        collection_name: &str,
        game_id: &str,
        bottle_name: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "DELETE FROM collection_install_checkpoints
             WHERE collection_name = ?1 AND game_id = ?2 AND bottle_name = ?3",
            params![collection_name, game_id, bottle_name],
        )?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Pinned game versions
    // -----------------------------------------------------------------------

    /// Get the pinned (last-known) game version for a game/bottle pair.
    pub fn get_pinned_game_version(
        &self,
        game_id: &str,
        bottle_name: &str,
    ) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let result = conn
            .query_row(
                "SELECT version FROM pinned_game_versions
                 WHERE game_id = ?1 AND bottle_name = ?2",
                params![game_id, bottle_name],
                |row| row.get(0),
            )
            .ok();
        Ok(result)
    }

    /// Set (upsert) the pinned game version for a game/bottle pair.
    pub fn set_pinned_game_version(
        &self,
        game_id: &str,
        bottle_name: &str,
        version: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO pinned_game_versions (game_id, bottle_name, version, pinned_at)
             VALUES (?1, ?2, ?3, datetime('now'))
             ON CONFLICT(game_id, bottle_name)
             DO UPDATE SET version = excluded.version, pinned_at = excluded.pinned_at",
            params![game_id, bottle_name, version],
        )?;
        Ok(())
    }

    /// List all pending (incomplete) Wabbajack installs.
    #[allow(clippy::type_complexity)]
    pub fn list_pending_wj_installs(
        &self,
    ) -> Result<
        Vec<(
            i64,
            String,
            String,
            String,
            i64,
            i64,
            i64,
            i64,
            Option<String>,
        )>,
    > {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, modlist_name, modlist_version, status,
                    total_archives, completed_archives,
                    total_directives, completed_directives,
                    error_message
             FROM wabbajack_installs
             WHERE status NOT IN ('completed', 'cancelled')
             ORDER BY updated_at DESC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }
    /// Find Wabbajack installs that were left in an active state (downloading,
    /// extracting, processing, deploying) — these are orphans from a previous
    /// crash or forced quit.  Returns (id, install_dir, status).
    pub fn get_stale_wj_installs(&self) -> Result<Vec<(i64, String, String)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, install_dir, status
             FROM wabbajack_installs
             WHERE status IN ('downloading', 'extracting', 'processing', 'deploying')
             ORDER BY id",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }
}

// ---------------------------------------------------------------------------
// Chat history persistence
// ---------------------------------------------------------------------------

impl ModDatabase {
    /// Save a chat message to the persistent history.
    ///
    /// Only user and assistant messages should be persisted — system and tool
    /// messages are ephemeral context rebuilt each session.
    pub fn save_chat_message(
        &self,
        game_id: &str,
        bottle_name: &str,
        msg: &crate::llm_chat::ChatMessage,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tool_calls_json = msg
            .tool_calls
            .as_ref()
            .map(|tc| serde_json::to_string(tc).unwrap_or_default());
        let mentioned_mods_json = msg
            .mentioned_mods
            .as_ref()
            .map(|mm| serde_json::to_string(mm).unwrap_or_default());
        conn.execute(
            "INSERT INTO chat_history (game_id, bottle_name, role, content, tool_calls, mentioned_mods)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                game_id,
                bottle_name,
                msg.role,
                msg.content,
                tool_calls_json,
                mentioned_mods_json,
            ],
        )?;
        let row_id = conn.last_insert_rowid();

        // Auto-prune: keep at most 500 messages per game/bottle pair
        conn.execute(
            "DELETE FROM chat_history WHERE game_id = ?1 AND bottle_name = ?2
             AND id NOT IN (
                SELECT id FROM chat_history
                WHERE game_id = ?1 AND bottle_name = ?2
                ORDER BY id DESC LIMIT 500
             )",
            params![game_id, bottle_name],
        )?;

        Ok(row_id)
    }

    /// Load the most recent chat messages for a game/bottle pair.
    ///
    /// Returns up to `limit` messages ordered oldest-first (chronological),
    /// suitable for prepending into a new session after the system prompt.
    pub fn load_chat_history(
        &self,
        game_id: &str,
        bottle_name: &str,
        limit: usize,
    ) -> Result<Vec<crate::llm_chat::ChatMessage>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT role, content, tool_calls, mentioned_mods, created_at
             FROM chat_history
             WHERE game_id = ?1 AND bottle_name = ?2
             ORDER BY id DESC
             LIMIT ?3",
        )?;
        let rows: Vec<crate::llm_chat::ChatMessage> = stmt
            .query_map(params![game_id, bottle_name, limit as i64], |row| {
                let role: String = row.get(0)?;
                let content: String = row.get(1)?;
                let tool_calls_json: Option<String> = row.get(2)?;
                let mentioned_mods_json: Option<String> = row.get(3)?;
                let created_at: Option<String> = row.get(4)?;

                let tool_calls = tool_calls_json.and_then(|j| serde_json::from_str(&j).ok());
                let mentioned_mods =
                    mentioned_mods_json.and_then(|j| serde_json::from_str(&j).ok());

                Ok(crate::llm_chat::ChatMessage {
                    role,
                    content,
                    tool_calls,
                    mentioned_mods,
                    timestamp: created_at,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        // Reverse so messages are in chronological order (oldest first)
        let mut messages = rows;
        messages.reverse();
        Ok(messages)
    }

    /// Delete all chat history for a game/bottle pair.
    pub fn clear_chat_history(&self, game_id: &str, bottle_name: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "DELETE FROM chat_history WHERE game_id = ?1 AND bottle_name = ?2",
            params![game_id, bottle_name],
        )?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Wabbajack directive resume tracking
// ---------------------------------------------------------------------------

impl ModDatabase {
    /// Initialize directive statuses for a new WJ install.
    pub fn init_wj_directive_statuses(
        &self,
        install_id: &str,
        directives: &[(usize, &str)], // (index, directive_type)
    ) -> std::result::Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;

        // Clear any prior statuses for this install (crash recovery)
        tx.execute(
            "DELETE FROM wj_directive_status WHERE install_id = ?1",
            params![install_id],
        )
        .map_err(|e| e.to_string())?;

        let mut stmt = tx
            .prepare(
                "INSERT INTO wj_directive_status (install_id, directive_index, directive_type, status, updated_at)
                 VALUES (?1, ?2, ?3, 'pending', datetime('now'))",
            )
            .map_err(|e| e.to_string())?;

        for (index, dtype) in directives {
            stmt.execute(params![install_id, *index as i64, dtype])
                .map_err(|e| e.to_string())?;
        }
        drop(stmt);
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Mark a directive as processing.
    pub fn mark_directive_processing(
        &self,
        install_id: &str,
        directive_index: usize,
    ) -> std::result::Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE wj_directive_status SET status = 'processing', updated_at = datetime('now')
             WHERE install_id = ?1 AND directive_index = ?2",
            params![install_id, directive_index as i64],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Mark a directive as completed.
    pub fn mark_directive_completed(
        &self,
        install_id: &str,
        directive_index: usize,
    ) -> std::result::Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE wj_directive_status SET status = 'completed', updated_at = datetime('now')
             WHERE install_id = ?1 AND directive_index = ?2",
            params![install_id, directive_index as i64],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Mark a directive as failed.
    pub fn mark_directive_failed(
        &self,
        install_id: &str,
        directive_index: usize,
    ) -> std::result::Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE wj_directive_status SET status = 'failed', updated_at = datetime('now')
             WHERE install_id = ?1 AND directive_index = ?2",
            params![install_id, directive_index as i64],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Reset all 'processing' directives back to 'pending' (crash recovery).
    pub fn reset_processing_directives(
        &self,
        install_id: &str,
    ) -> std::result::Result<usize, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let count = conn
            .execute(
                "UPDATE wj_directive_status SET status = 'pending', updated_at = datetime('now')
                 WHERE install_id = ?1 AND status = 'processing'",
                params![install_id],
            )
            .map_err(|e| e.to_string())?;
        Ok(count)
    }

    /// Get set of completed directive indices for an install.
    pub fn get_completed_directive_indices(
        &self,
        install_id: &str,
    ) -> std::result::Result<HashSet<usize>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT directive_index FROM wj_directive_status
                 WHERE install_id = ?1 AND status = 'completed'",
            )
            .map_err(|e| e.to_string())?;
        let indices = stmt
            .query_map(params![install_id], |row| Ok(row.get::<_, i64>(0)? as usize))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(indices)
    }

    /// Clean up directive statuses for a completed/cancelled install.
    pub fn cleanup_directive_statuses(
        &self,
        install_id: &str,
    ) -> std::result::Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM wj_directive_status WHERE install_id = ?1",
            params![install_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Nexus URL cache
// ---------------------------------------------------------------------------

impl ModDatabase {
    /// Cache a Nexus download URL with expiry.
    pub fn cache_nexus_url(
        &self,
        game_domain: &str,
        mod_id: i64,
        file_id: i64,
        url: &str,
        expires_at: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT OR REPLACE INTO nexus_url_cache (game_domain, mod_id, file_id, url, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![game_domain, mod_id, file_id, url, expires_at],
        )?;
        Ok(())
    }

    /// Get a cached Nexus URL if it hasn't expired.
    pub fn get_cached_nexus_url(
        &self,
        game_domain: &str,
        mod_id: i64,
        file_id: i64,
    ) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let result = conn
            .prepare(
                "SELECT url FROM nexus_url_cache
                 WHERE game_domain = ?1 AND mod_id = ?2 AND file_id = ?3
                 AND datetime(expires_at) > datetime('now')",
            )?
            .query_row(params![game_domain, mod_id, file_id], |row| row.get(0))
            .ok();
        Ok(result)
    }
}

// -----------------------------------------------------------------------
// Patch basis cache (on ModDatabase — persistent across installs)
// -----------------------------------------------------------------------

impl ModDatabase {
    /// Cache patch basis file hashes for quick lookup.
    pub fn cache_patch_basis_hash(
        &self,
        modlist: &str,
        file_path: &str,
        quick_hash: u64,
        full_hash: &str,
        file_size: u64,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        // SQLite stores integers as i64. Values > i64::MAX wrap to negative but
        // round-trip correctly through Rust's `as u64` bit-preserving cast.
        // Do NOT filter on this column in SQL — use Rust-side comparison only.
        conn.execute(
            "INSERT OR REPLACE INTO patch_basis_cache (modlist, file_path, quick_hash, full_hash, file_size)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![modlist, file_path, quick_hash as i64, full_hash, file_size as i64],
        )?;
        Ok(())
    }

    /// Get a cached patch basis hash entry.
    /// Returns (quick_hash, full_hash, file_size) if found.
    pub fn get_cached_patch_basis(
        &self,
        modlist: &str,
        file_path: &str,
    ) -> Result<Option<(u64, String, u64)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        // SQLite stores integers as i64. Values > i64::MAX wrap to negative but
        // round-trip correctly through Rust's `as u64` bit-preserving cast.
        // Do NOT filter on this column in SQL — use Rust-side comparison only.
        let result = conn
            .prepare(
                "SELECT quick_hash, full_hash, file_size FROM patch_basis_cache
                 WHERE modlist = ?1 AND file_path = ?2",
            )?
            .query_row(params![modlist, file_path], |row| {
                Ok((
                    row.get::<_, i64>(0)? as u64,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? as u64,
                ))
            })
            .ok();
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Per-modlist config cache
// ---------------------------------------------------------------------------

impl ModDatabase {
    /// Save modlist install configuration for future reinstalls.
    pub fn save_modlist_config(
        &self,
        name: &str,
        version: &str,
        config_json: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT OR REPLACE INTO modlist_config (name, version, config_json)
             VALUES (?1, ?2, ?3)",
            params![name, version, config_json],
        )?;
        Ok(())
    }

    /// Get a saved modlist configuration.
    pub fn get_modlist_config(
        &self,
        name: &str,
        version: &str,
    ) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let result = conn
            .prepare(
                "SELECT config_json FROM modlist_config WHERE name = ?1 AND version = ?2",
            )?
            .query_row(params![name, version], |row| row.get(0))
            .ok();
        Ok(result)
    }

    /// Get any saved config for a modlist (any version, newest first).
    pub fn get_latest_modlist_config(
        &self,
        name: &str,
    ) -> Result<Option<(String, String)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let result = conn
            .prepare(
                "SELECT version, config_json FROM modlist_config
                 WHERE name = ?1 ORDER BY saved_at DESC LIMIT 1",
            )?
            .query_row(params![name], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .ok();
        Ok(result)
    }
}

// -----------------------------------------------------------------------
// BSA extraction cache (temporary — aggressive pragmas for speed)
// -----------------------------------------------------------------------

/// Open or create the BSA cache database (separate from main DB for isolation).
/// Uses aggressive pragmas since this is temporary data only.
pub fn open_bsa_cache(cache_dir: &Path) -> std::result::Result<rusqlite::Connection, String> {
    std::fs::create_dir_all(cache_dir).map_err(|e| e.to_string())?;
    let db_path = cache_dir.join("bsa_cache.db");
    let conn = rusqlite::Connection::open(&db_path).map_err(|e| e.to_string())?;

    // Aggressive pragmas — this is throwaway temp data
    conn.execute_batch(
        "PRAGMA journal_mode = OFF;
         PRAGMA synchronous = OFF;
         PRAGMA cache_size = -65536;
         PRAGMA temp_store = MEMORY;",
    )
    .map_err(|e| e.to_string())?;

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS bsa_cache (
            bsa_path TEXT NOT NULL,
            file_path TEXT NOT NULL,
            data BLOB NOT NULL,
            PRIMARY KEY (bsa_path, file_path)
        )",
    )
    .map_err(|e| e.to_string())?;

    Ok(conn)
}

/// Insert a file into the BSA cache.
pub fn bsa_cache_insert(
    conn: &rusqlite::Connection,
    bsa_path: &str,
    file_path: &str,
    data: &[u8],
) -> std::result::Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO bsa_cache (bsa_path, file_path, data) VALUES (?1, ?2, ?3)",
        params![bsa_path, file_path, data],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Get a file from the BSA cache.
pub fn bsa_cache_get(
    conn: &rusqlite::Connection,
    bsa_path: &str,
    file_path: &str,
) -> std::result::Result<Option<Vec<u8>>, String> {
    let result = conn
        .prepare("SELECT data FROM bsa_cache WHERE bsa_path = ?1 AND file_path = ?2")
        .map_err(|e| e.to_string())?
        .query_row(params![bsa_path, file_path], |row| row.get(0))
        .ok();
    Ok(result)
}

/// Clear all entries for a specific BSA.
pub fn bsa_cache_clear(
    conn: &rusqlite::Connection,
    bsa_path: &str,
) -> std::result::Result<(), String> {
    conn.execute(
        "DELETE FROM bsa_cache WHERE bsa_path = ?1",
        params![bsa_path],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Clear the entire BSA cache (call after install completes).
pub fn bsa_cache_clear_all(conn: &rusqlite::Connection) -> std::result::Result<(), String> {
    conn.execute("DELETE FROM bsa_cache", [])
        .map_err(|e| e.to_string())?;
    // Reclaim space
    conn.execute("VACUUM", [])
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Games Table API (PersistedGame — Wine + Native)
// ---------------------------------------------------------------------------

/// Persistence shape for the `games` table. Distinct from `DetectedGame`
/// (transient runtime struct passed to frontend). `PersistedGame` is what we
/// serialize to and deserialize from the `games` table introduced in v23.
#[derive(Clone, Debug)]
pub struct PersistedGame {
    pub game_id: String,
    pub runtime: crate::runtime::GameRuntime,
}

// -- private helpers --------------------------------------------------------

fn arch_to_db_str(a: crate::runtime::Architecture) -> &'static str {
    use crate::runtime::Architecture;
    match a {
        Architecture::AppleSilicon => "apple_silicon",
        Architecture::IntelOnly => "intel_only",
        Architecture::Universal => "universal",
        Architecture::Unknown => "unknown",
    }
}

fn source_to_db_str(s: crate::runtime::NativeSource) -> &'static str {
    use crate::runtime::NativeSource;
    match s {
        NativeSource::SystemApplications => "system_applications",
        NativeSource::Steam => "steam",
        NativeSource::Gog => "gog",
        NativeSource::Manual => "manual",
        NativeSource::AppStore => "app_store",
    }
}

#[allow(clippy::too_many_arguments)]
fn row_to_runtime(
    runtime_str: &str,
    bottle_name: Option<String>,
    bottle_path: Option<String>,
    native_app_path: Option<String>,
    native_data_root: Option<String>,
    native_architecture: Option<String>,
    native_sandboxed: i64,
    native_source: Option<String>,
) -> Result<crate::runtime::GameRuntime> {
    use crate::runtime::{Architecture, GameRuntime, NativeContext, NativeSource, WineContext};
    match runtime_str {
        "wine" => Ok(GameRuntime::Wine(WineContext {
            bottle_name: bottle_name.unwrap_or_default(),
            bottle_path: std::path::PathBuf::from(bottle_path.unwrap_or_default()),
            source: String::new(), // games table doesn't store wine source; acceptable
        })),
        "native" => {
            let arch = match native_architecture.as_deref() {
                Some("apple_silicon") => Architecture::AppleSilicon,
                Some("intel_only") => Architecture::IntelOnly,
                Some("universal") => Architecture::Universal,
                _ => Architecture::Unknown,
            };
            let source = match native_source.as_deref() {
                Some("system_applications") => NativeSource::SystemApplications,
                Some("steam") => NativeSource::Steam,
                Some("gog") => NativeSource::Gog,
                Some("manual") => NativeSource::Manual,
                Some("app_store") => NativeSource::AppStore,
                _ => NativeSource::Manual, // safe fallback
            };
            Ok(GameRuntime::Native(NativeContext {
                app_bundle_path: std::path::PathBuf::from(
                    native_app_path.unwrap_or_default(),
                ),
                game_data_root: std::path::PathBuf::from(
                    native_data_root.unwrap_or_default(),
                ),
                architecture: arch,
                sandboxed: native_sandboxed != 0,
                source,
            }))
        }
        other => Err(DatabaseError::Other(format!(
            "unknown runtime discriminator: {}",
            other
        ))),
    }
}

// -- impl ModDatabase -------------------------------------------------------

impl ModDatabase {
    /// Insert or update a row in the `games` table. Idempotent — calling
    /// twice with the same `game_id` replaces the row, doesn't insert a
    /// duplicate.
    pub fn upsert_game(&self, game: &PersistedGame) -> Result<()> {
        use crate::runtime::GameRuntime;
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());

        let (runtime_str, bottle_name, bottle_path, native_app_path, native_data_root,
             native_architecture, native_sandboxed, native_source) = match &game.runtime {
            GameRuntime::Wine(w) => (
                "wine",
                Some(w.bottle_name.clone()),
                Some(w.bottle_path.to_string_lossy().into_owned()),
                None::<String>,
                None::<String>,
                None::<String>,
                0i64,
                None::<String>,
            ),
            GameRuntime::Native(n) => (
                "native",
                None::<String>,
                None::<String>,
                Some(n.app_bundle_path.to_string_lossy().into_owned()),
                Some(n.game_data_root.to_string_lossy().into_owned()),
                Some(arch_to_db_str(n.architecture).to_string()),
                if n.sandboxed { 1i64 } else { 0i64 },
                Some(source_to_db_str(n.source).to_string()),
            ),
        };

        conn.execute(
            "INSERT OR REPLACE INTO games
                (game_id, runtime, bottle_name, bottle_path,
                 native_app_path, native_data_root, native_architecture,
                 native_sandboxed, native_source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                game.game_id,
                runtime_str,
                bottle_name.unwrap_or_default(),
                bottle_path.unwrap_or_default(),
                native_app_path,
                native_data_root,
                native_architecture,
                native_sandboxed,
                native_source,
            ],
        )?;
        Ok(())
    }

    /// Read all persisted games. Returns an empty `Vec` if the table is empty.
    pub fn list_persisted_games(&self) -> Result<Vec<PersistedGame>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT game_id, runtime, bottle_name, bottle_path,
                    native_app_path, native_data_root, native_architecture,
                    native_sandboxed, native_source
             FROM games",
        )?;
        let rows = stmt
            .query_map([], |row| {
                let game_id: String = row.get(0)?;
                let runtime_str: String = row.get(1)?;
                let bottle_name: Option<String> = row.get::<_, Option<String>>(2)?;
                let bottle_path: Option<String> = row.get::<_, Option<String>>(3)?;
                let native_app_path: Option<String> = row.get(4)?;
                let native_data_root: Option<String> = row.get(5)?;
                let native_architecture: Option<String> = row.get(6)?;
                let native_sandboxed: i64 = row.get::<_, Option<i64>>(7)?.unwrap_or(0);
                let native_source: Option<String> = row.get(8)?;
                Ok((
                    game_id,
                    runtime_str,
                    bottle_name,
                    bottle_path,
                    native_app_path,
                    native_data_root,
                    native_architecture,
                    native_sandboxed,
                    native_source,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let mut games = Vec::with_capacity(rows.len());
        for (game_id, runtime_str, bottle_name, bottle_path, native_app_path,
             native_data_root, native_architecture, native_sandboxed, native_source) in rows
        {
            let runtime = row_to_runtime(
                &runtime_str,
                bottle_name,
                bottle_path,
                native_app_path,
                native_data_root,
                native_architecture,
                native_sandboxed,
                native_source,
            )?;
            games.push(PersistedGame { game_id, runtime });
        }
        Ok(games)
    }

    /// Read a single game by id. Returns `None` if not present.
    pub fn get_persisted_game(&self, game_id: &str) -> Result<Option<PersistedGame>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT game_id, runtime, bottle_name, bottle_path,
                    native_app_path, native_data_root, native_architecture,
                    native_sandboxed, native_source
             FROM games WHERE game_id = ?1",
        )?;

        match stmt.query_row(params![game_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<i64>>(7)?.unwrap_or(0),
                row.get::<_, Option<String>>(8)?,
            ))
        }) {
            Ok((game_id, runtime_str, bottle_name, bottle_path, native_app_path,
                native_data_root, native_architecture, native_sandboxed, native_source)) => {
                let runtime = row_to_runtime(
                    &runtime_str,
                    bottle_name,
                    bottle_path,
                    native_app_path,
                    native_data_root,
                    native_architecture,
                    native_sandboxed,
                    native_source,
                )?;
                Ok(Some(PersistedGame { game_id, runtime }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Delete a game by id. No-op if not present.
    pub fn delete_persisted_game(&self, game_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute("DELETE FROM games WHERE game_id = ?1", params![game_id])?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Helper: create a database inside a temporary directory.
    fn test_db() -> (ModDatabase, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("corkscrew.db");
        let db = ModDatabase::new(&db_path).unwrap();
        (db, tmp)
    }

    #[test]
    fn test_add_and_get_mod() {
        let (db, _tmp) = test_db();

        let files = vec![
            "data/meshes/armor.nif".to_string(),
            "data/textures/armor.dds".to_string(),
        ];

        let id = db
            .add_mod(
                "skyrim",
                "default",
                Some(1234),
                "Cool Armor",
                "1.0",
                "cool_armor.zip",
                &files,
            )
            .unwrap();

        let m = db.get_mod(id).unwrap().expect("mod should exist");
        assert_eq!(m.name, "Cool Armor");
        assert_eq!(m.version, "1.0");
        assert_eq!(m.game_id, "skyrim");
        assert_eq!(m.bottle_name, "default");
        assert_eq!(m.nexus_mod_id, Some(1234));
        assert_eq!(m.installed_files, files);
        assert!(m.enabled);
    }

    #[test]
    fn test_remove_mod() {
        let (db, _tmp) = test_db();

        let id = db
            .add_mod("skyrim", "default", None, "Test Mod", "2.0", "test.7z", &[])
            .unwrap();

        let removed = db.remove_mod(id).unwrap();
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().name, "Test Mod");

        // Should be gone now.
        assert!(db.get_mod(id).unwrap().is_none());

        // Removing a non-existent mod returns None.
        assert!(db.remove_mod(id).unwrap().is_none());
    }

    #[test]
    fn test_list_mods() {
        let (db, _tmp) = test_db();

        db.add_mod("skyrim", "default", None, "Mod A", "1.0", "a.zip", &[])
            .unwrap();
        db.add_mod("skyrim", "default", None, "Mod B", "1.0", "b.zip", &[])
            .unwrap();
        db.add_mod("fallout4", "default", None, "Mod C", "1.0", "c.zip", &[])
            .unwrap();

        let skyrim_mods = db.list_mods("skyrim", "default").unwrap();
        assert_eq!(skyrim_mods.len(), 2);

        let fallout_mods = db.list_mods("fallout4", "default").unwrap();
        assert_eq!(fallout_mods.len(), 1);
        assert_eq!(fallout_mods[0].name, "Mod C");
    }

    #[test]
    fn test_get_all_installed_files() {
        let (db, _tmp) = test_db();

        let files_a = vec!["a.txt".to_string(), "b.txt".to_string()];
        let files_b = vec!["c.txt".to_string()];

        let id_a = db
            .add_mod("skyrim", "default", None, "A", "1.0", "a.zip", &files_a)
            .unwrap();
        let id_b = db
            .add_mod("skyrim", "default", None, "B", "1.0", "b.zip", &files_b)
            .unwrap();

        let all = db.get_all_installed_files("skyrim", "default").unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all["a.txt"], id_a);
        assert_eq!(all["b.txt"], id_a);
        assert_eq!(all["c.txt"], id_b);
    }

    #[test]
    fn test_find_conflicts() {
        let (db, _tmp) = test_db();

        let files = vec!["shared.txt".to_string(), "unique_a.txt".to_string()];
        db.add_mod(
            "skyrim",
            "default",
            None,
            "Existing Mod",
            "1.0",
            "e.zip",
            &files,
        )
        .unwrap();

        let new_files = vec!["shared.txt".to_string(), "brand_new.txt".to_string()];
        let conflicts = db.find_conflicts("skyrim", "default", &new_files).unwrap();

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts["shared.txt"], "Existing Mod");
    }

    #[test]
    fn test_set_enabled() {
        let (db, _tmp) = test_db();

        let id = db
            .add_mod("skyrim", "default", None, "Toggle Me", "1.0", "t.zip", &[])
            .unwrap();
        assert!(db.get_mod(id).unwrap().unwrap().enabled);

        db.set_enabled(id, false).unwrap();
        assert!(!db.get_mod(id).unwrap().unwrap().enabled);

        db.set_enabled(id, true).unwrap();
        assert!(db.get_mod(id).unwrap().unwrap().enabled);
    }

    #[test]
    fn test_set_enabled_nonexistent() {
        let (db, _tmp) = test_db();
        let result = db.set_enabled(999, true);
        assert!(result.is_err());
    }

    #[test]
    fn test_creates_parent_dirs() {
        let tmp = TempDir::new().unwrap();
        let nested = tmp.path().join("a").join("b").join("c").join("mods.db");
        let _db = ModDatabase::new(&nested).unwrap();
        assert!(nested.exists());
    }

    // -----------------------------------------------------------------------
    // Workstream 1/3: Incremental deployment + conflict DB helpers
    // -----------------------------------------------------------------------

    #[test]
    fn test_get_deployment_manifest_map() {
        let (db, _tmp) = test_db();
        let mod_id = db
            .add_mod("skyrimse", "Gaming", None, "Mod", "1.0", "m.zip", &[])
            .unwrap();

        db.add_deployment_entry(
            "skyrimse",
            "Gaming",
            mod_id,
            "test.esp",
            "/staging/test.esp",
            "hardlink",
            None,
        )
        .unwrap();
        db.add_deployment_entry(
            "skyrimse",
            "Gaming",
            mod_id,
            "meshes/a.nif",
            "/staging/meshes/a.nif",
            "copy",
            None,
        )
        .unwrap();

        let map = db
            .get_deployment_manifest_map("skyrimse", "Gaming")
            .unwrap();
        assert_eq!(map.len(), 2);
        assert!(map.contains_key("test.esp"));
        assert!(map.contains_key("meshes/a.nif"));
        assert_eq!(map["test.esp"].mod_id, mod_id);
    }

    #[test]
    fn test_batch_add_deployment_entries_with_hashes() {
        let (db, _tmp) = test_db();
        let mod_id = db
            .add_mod("skyrimse", "Gaming", None, "Mod", "1.0", "m.zip", &[])
            .unwrap();

        let entries = vec![
            (
                "skyrimse",
                "Gaming",
                mod_id,
                "file1.esp",
                "/staging/file1.esp",
                "hardlink",
                Some("abc123"),
                "data",
            ),
            (
                "skyrimse",
                "Gaming",
                mod_id,
                "file2.esp",
                "/staging/file2.esp",
                "copy",
                None,
                "data",
            ),
        ];
        let entries_ref: Vec<(&str, &str, i64, &str, &str, &str, Option<&str>, &str)> = entries
            .iter()
            .map(|(a, b, c, d, e, f, g, h)| (*a, *b, *c, *d, *e, *f, g.as_deref(), *h))
            .collect();
        db.batch_add_deployment_entries_with_hashes(&entries_ref)
            .unwrap();

        let manifest = db.get_deployment_manifest("skyrimse", "Gaming").unwrap();
        assert_eq!(manifest.len(), 2);

        let file1 = manifest
            .iter()
            .find(|e| e.relative_path == "file1.esp")
            .unwrap();
        assert_eq!(file1.sha256.as_deref(), Some("abc123"));

        let file2 = manifest
            .iter()
            .find(|e| e.relative_path == "file2.esp")
            .unwrap();
        assert!(file2.sha256.is_none());
    }

    #[test]
    fn test_batch_remove_deployment_entries() {
        let (db, _tmp) = test_db();
        let mod_id = db
            .add_mod("skyrimse", "Gaming", None, "Mod", "1.0", "m.zip", &[])
            .unwrap();

        db.add_deployment_entry(
            "skyrimse", "Gaming", mod_id, "a.esp", "/s/a.esp", "hardlink", None,
        )
        .unwrap();
        db.add_deployment_entry(
            "skyrimse", "Gaming", mod_id, "b.esp", "/s/b.esp", "hardlink", None,
        )
        .unwrap();
        db.add_deployment_entry(
            "skyrimse", "Gaming", mod_id, "c.esp", "/s/c.esp", "hardlink", None,
        )
        .unwrap();

        db.batch_remove_deployment_entries("skyrimse", "Gaming", &["a.esp", "c.esp"])
            .unwrap();

        let manifest = db.get_deployment_manifest("skyrimse", "Gaming").unwrap();
        assert_eq!(manifest.len(), 1);
        assert_eq!(manifest[0].relative_path, "b.esp");
    }

    #[test]
    fn test_get_file_hashes_bulk() {
        let (db, _tmp) = test_db();
        let mod1 = db
            .add_mod("skyrimse", "Gaming", None, "Mod1", "1.0", "m1.zip", &[])
            .unwrap();
        let mod2 = db
            .add_mod("skyrimse", "Gaming", None, "Mod2", "1.0", "m2.zip", &[])
            .unwrap();

        db.store_file_hashes(mod1, &[("textures/sky.dds".into(), "hash_a".into(), 1024)])
            .unwrap();
        db.store_file_hashes(
            mod2,
            &[
                ("textures/sky.dds".into(), "hash_b".into(), 2048),
                ("meshes/tree.nif".into(), "hash_c".into(), 512),
            ],
        )
        .unwrap();

        let hashes = db.get_file_hashes_bulk(&[mod1, mod2]).unwrap();
        assert_eq!(hashes.len(), 3);
        assert_eq!(hashes[&(mod1, "textures/sky.dds".to_string())], "hash_a");
        assert_eq!(hashes[&(mod2, "textures/sky.dds".to_string())], "hash_b");
        assert_eq!(hashes[&(mod2, "meshes/tree.nif".to_string())], "hash_c");
    }

    #[test]
    fn test_get_file_hashes_bulk_empty() {
        let (db, _tmp) = test_db();
        let hashes = db.get_file_hashes_bulk(&[]).unwrap();
        assert!(hashes.is_empty());
    }

    #[test]
    fn test_get_file_hashes_for_mods_aliases_bulk() {
        let (db, _tmp) = test_db();
        let mod1 = db
            .add_mod("skyrimse", "Gaming", None, "Mod1", "1.0", "m1.zip", &[])
            .unwrap();
        db.store_file_hashes(mod1, &[("test.esp".into(), "hash_x".into(), 100)])
            .unwrap();

        // Both methods should return the same result
        let bulk = db.get_file_hashes_bulk(&[mod1]).unwrap();
        let for_mods = db.get_file_hashes_for_mods(&[mod1]).unwrap();
        assert_eq!(bulk, for_mods);
    }

    #[test]
    fn test_patch_basis_cache_roundtrip() {
        let (db, _tmp) = test_db();
        db.cache_patch_basis_hash("GTS", "textures/sky.dds", 12345, "abc123==", 4096)
            .unwrap();
        let cached = db.get_cached_patch_basis("GTS", "textures/sky.dds").unwrap();
        assert!(cached.is_some());
        let (qh, fh, sz) = cached.unwrap();
        assert_eq!(qh, 12345);
        assert_eq!(fh, "abc123==");
        assert_eq!(sz, 4096);
    }

    #[test]
    fn test_patch_basis_cache_miss() {
        let (db, _tmp) = test_db();
        let cached = db.get_cached_patch_basis("GTS", "nonexistent.dds").unwrap();
        assert!(cached.is_none());
    }

    #[test]
    fn test_patch_basis_cache_overwrite() {
        let (db, _tmp) = test_db();
        db.cache_patch_basis_hash("GTS", "test.esp", 111, "old_hash", 100)
            .unwrap();
        db.cache_patch_basis_hash("GTS", "test.esp", 222, "new_hash", 200)
            .unwrap();
        let (qh, fh, sz) = db.get_cached_patch_basis("GTS", "test.esp").unwrap().unwrap();
        assert_eq!(qh, 222);
        assert_eq!(fh, "new_hash");
        assert_eq!(sz, 200);
    }

    #[test]
    fn test_bsa_cache_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let conn = open_bsa_cache(tmp.path()).unwrap();

        bsa_cache_insert(&conn, "Skyrim.bsa", "textures/sky.dds", b"pixel_data").unwrap();
        let data = bsa_cache_get(&conn, "Skyrim.bsa", "textures/sky.dds").unwrap();
        assert_eq!(data, Some(b"pixel_data".to_vec()));
    }

    #[test]
    fn test_bsa_cache_miss() {
        let tmp = tempfile::TempDir::new().unwrap();
        let conn = open_bsa_cache(tmp.path()).unwrap();
        let data = bsa_cache_get(&conn, "Skyrim.bsa", "nonexistent.dds").unwrap();
        assert!(data.is_none());
    }

    #[test]
    fn test_bsa_cache_clear_specific() {
        let tmp = tempfile::TempDir::new().unwrap();
        let conn = open_bsa_cache(tmp.path()).unwrap();

        bsa_cache_insert(&conn, "A.bsa", "file1.dds", b"data1").unwrap();
        bsa_cache_insert(&conn, "B.bsa", "file2.dds", b"data2").unwrap();
        bsa_cache_clear(&conn, "A.bsa").unwrap();

        assert!(bsa_cache_get(&conn, "A.bsa", "file1.dds").unwrap().is_none());
        assert!(bsa_cache_get(&conn, "B.bsa", "file2.dds").unwrap().is_some());
    }

    #[test]
    fn test_bsa_cache_clear_all() {
        let tmp = tempfile::TempDir::new().unwrap();
        let conn = open_bsa_cache(tmp.path()).unwrap();

        bsa_cache_insert(&conn, "A.bsa", "file1.dds", b"data1").unwrap();
        bsa_cache_insert(&conn, "B.bsa", "file2.dds", b"data2").unwrap();
        bsa_cache_clear_all(&conn).unwrap();

        assert!(bsa_cache_get(&conn, "A.bsa", "file1.dds").unwrap().is_none());
        assert!(bsa_cache_get(&conn, "B.bsa", "file2.dds").unwrap().is_none());
    }

    // -----------------------------------------------------------------------
    // PersistedGame CRUD tests (games table API)
    // -----------------------------------------------------------------------

    fn make_wine_game(game_id: &str) -> PersistedGame {
        use crate::runtime::{GameRuntime, WineContext};
        PersistedGame {
            game_id: game_id.to_string(),
            runtime: GameRuntime::Wine(WineContext {
                bottle_name: "GTS".to_string(),
                bottle_path: std::path::PathBuf::from("/Users/x/Bottles/GTS"),
                source: String::new(),
            }),
        }
    }

    fn make_native_game(game_id: &str) -> PersistedGame {
        use crate::runtime::{Architecture, GameRuntime, NativeContext, NativeSource};
        PersistedGame {
            game_id: game_id.to_string(),
            runtime: GameRuntime::Native(NativeContext {
                app_bundle_path: std::path::PathBuf::from(
                    "/Applications/Stardew Valley.app",
                ),
                game_data_root: std::path::PathBuf::from(
                    "/Applications/Stardew Valley.app/Contents/MacOS",
                ),
                architecture: Architecture::AppleSilicon,
                sandboxed: false,
                source: NativeSource::Steam,
            }),
        }
    }

    #[test]
    fn upsert_and_list_round_trips_wine_game() {
        let (db, _tmp) = test_db();
        let game = make_wine_game("skyrimse");
        db.upsert_game(&game).unwrap();

        let all = db.list_persisted_games().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].game_id, "skyrimse");
        let w = all[0].runtime.wine().expect("should be Wine runtime");
        assert_eq!(w.bottle_name, "GTS");
        assert_eq!(w.bottle_path, std::path::PathBuf::from("/Users/x/Bottles/GTS"));
    }

    #[test]
    fn upsert_and_list_round_trips_native_game() {
        use crate::runtime::{Architecture, NativeSource};
        let (db, _tmp) = test_db();
        let game = make_native_game("stardew_valley_native");
        db.upsert_game(&game).unwrap();

        let all = db.list_persisted_games().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].game_id, "stardew_valley_native");
        let n = all[0].runtime.native().expect("should be Native runtime");
        assert_eq!(n.architecture, Architecture::AppleSilicon);
        assert_eq!(n.source, NativeSource::Steam);
        assert!(!n.sandboxed);
        assert_eq!(
            n.app_bundle_path,
            std::path::PathBuf::from("/Applications/Stardew Valley.app")
        );
        assert_eq!(
            n.game_data_root,
            std::path::PathBuf::from("/Applications/Stardew Valley.app/Contents/MacOS")
        );
    }

    #[test]
    fn upsert_is_idempotent_replaces_not_duplicates() {
        use crate::runtime::{GameRuntime, WineContext};
        let (db, _tmp) = test_db();
        db.upsert_game(&make_wine_game("skyrimse")).unwrap();
        // Second upsert with updated bottle_name — must replace, not duplicate.
        let updated = PersistedGame {
            game_id: "skyrimse".to_string(),
            runtime: GameRuntime::Wine(WineContext {
                bottle_name: "NewBottle".to_string(),
                bottle_path: std::path::PathBuf::from("/Users/x/Bottles/NewBottle"),
                source: String::new(),
            }),
        };
        db.upsert_game(&updated).unwrap();

        let all = db.list_persisted_games().unwrap();
        assert_eq!(all.len(), 1, "upsert must not duplicate rows");
        let w = all[0].runtime.wine().unwrap();
        assert_eq!(w.bottle_name, "NewBottle");
    }

    #[test]
    fn get_persisted_game_returns_some_after_upsert() {
        let (db, _tmp) = test_db();
        db.upsert_game(&make_native_game("sdv")).unwrap();
        let result = db.get_persisted_game("sdv").unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().game_id, "sdv");
    }

    #[test]
    fn get_persisted_game_returns_none_for_unknown_id() {
        let (db, _tmp) = test_db();
        let result = db.get_persisted_game("does_not_exist").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn get_persisted_game_propagates_unknown_runtime_discriminator() {
        let (db, _tmp) = test_db();
        // Insert a row with a bogus runtime discriminator that row_to_runtime rejects.
        let conn = db.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO games (game_id, runtime) VALUES ('weird', 'martian')",
            [],
        ).unwrap();
        drop(conn);

        let result = db.get_persisted_game("weird");
        assert!(
            result.is_err(),
            "should propagate unknown discriminator error, not silently return None"
        );
    }

    #[test]
    fn delete_persisted_game_removes_row() {
        let (db, _tmp) = test_db();
        db.upsert_game(&make_wine_game("skyrimse")).unwrap();
        assert!(db.get_persisted_game("skyrimse").unwrap().is_some());
        db.delete_persisted_game("skyrimse").unwrap();
        assert!(db.get_persisted_game("skyrimse").unwrap().is_none());

        // Also verify list is empty.
        assert!(db.list_persisted_games().unwrap().is_empty());
    }

    #[test]
    fn delete_persisted_game_is_noop_for_unknown_id() {
        let (db, _tmp) = test_db();
        // Must not error on missing id.
        db.delete_persisted_game("nonexistent").unwrap();
    }
}
