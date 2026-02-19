use std::collections::HashMap;
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
    pub name: String,
    pub version: String,
    pub archive_name: String,
    pub installed_files: Vec<String>,
    pub installed_at: String,
    pub enabled: bool,
    pub staging_path: Option<String>,
    pub install_priority: i32,
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
}

// ---------------------------------------------------------------------------
// FileConflict
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileConflict {
    pub relative_path: String,
    pub mods: Vec<ConflictModInfo>,
    pub winner_mod_id: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConflictModInfo {
    pub mod_id: i64,
    pub mod_name: String,
    pub priority: i32,
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

impl ModDatabase {
    /// Open (or create) the database at `db_path`, creating parent
    /// directories as needed, and run all pending schema migrations.
    pub fn new(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path)?;

        // Enable WAL mode for better concurrent-read performance.
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;

        // Enable foreign key enforcement.
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;

        // Run schema migrations
        crate::migrations::migrate(&conn).map_err(|e| {
            DatabaseError::Other(format!("Schema migration failed: {}", e))
        })?;

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

    // -- helpers ------------------------------------------------------------

    /// Build an `InstalledMod` from the current row of a prepared statement.
    /// Column order must match [`Self::SELECT_COLUMNS`].
    fn row_to_mod(row: &rusqlite::Row<'_>) -> rusqlite::Result<InstalledMod> {
        let files_json: String = row.get(7)?;
        let installed_files: Vec<String> =
            serde_json::from_str(&files_json).unwrap_or_default();
        let enabled_int: i64 = row.get(9)?;

        Ok(InstalledMod {
            id: row.get(0)?,
            game_id: row.get(1)?,
            bottle_name: row.get(2)?,
            nexus_mod_id: row.get(3)?,
            nexus_file_id: row.get(10)?,
            source_url: row.get(11)?,
            name: row.get(4)?,
            version: row.get(5)?,
            archive_name: row.get(6)?,
            installed_files,
            installed_at: row.get(8)?,
            enabled: enabled_int != 0,
            staging_path: row.get(12)?,
            install_priority: row.get(13)?,
        })
    }

    /// The column list used in every SELECT on `installed_mods`.
    const SELECT_COLUMNS: &'static str =
        "id, game_id, bottle_name, nexus_mod_id, name, version, \
         archive_name, installed_files, installed_at, enabled, \
         nexus_file_id, source_url, staging_path, install_priority";

    // -- public API ---------------------------------------------------------

    /// Insert a new mod record and return its auto-generated row id.
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

        let conn = self.conn.lock().unwrap();
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
    pub fn remove_mod(&self, mod_id: i64) -> Result<Option<InstalledMod>> {
        let existing = self.get_mod(mod_id)?;
        if existing.is_some() {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "DELETE FROM installed_mods WHERE id = ?1",
                params![mod_id],
            )?;
        }
        Ok(existing)
    }

    /// Fetch a single mod by its primary key.
    pub fn get_mod(&self, mod_id: i64) -> Result<Option<InstalledMod>> {
        let conn = self.conn.lock().unwrap();
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

    /// List every mod installed for a given game + bottle combination.
    pub fn list_mods(
        &self,
        game_id: &str,
        bottle_name: &str,
    ) -> Result<Vec<InstalledMod>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM installed_mods \
             WHERE game_id = ?1 AND bottle_name = ?2 \
             ORDER BY installed_at DESC",
            Self::SELECT_COLUMNS,
        ))?;

        let rows = stmt.query_map(
            params![game_id, bottle_name],
            Self::row_to_mod,
        )?;

        let mut mods = Vec::new();
        for row in rows {
            mods.push(row?);
        }
        Ok(mods)
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
        let conn = self.conn.lock().unwrap();
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
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE installed_mods SET staging_path = ?1 WHERE id = ?2",
            params![staging_path, mod_id],
        )?;
        Ok(())
    }

    /// Set the install priority for a mod.
    pub fn set_mod_priority(&self, mod_id: i64, priority: i32) -> Result<()> {
        let conn = self.conn.lock().unwrap();
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
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "UPDATE installed_mods SET install_priority = ?1
             WHERE id = ?2 AND game_id = ?3 AND bottle_name = ?4",
        )?;

        for (i, mod_id) in ordered_mod_ids.iter().enumerate() {
            stmt.execute(params![i as i32, mod_id, game_id, bottle_name])?;
        }
        Ok(())
    }

    // -- Deployment manifest ------------------------------------------------

    /// Add a deployment manifest entry.
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
        let conn = self.conn.lock().unwrap();
        let deployed_at = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT OR REPLACE INTO deployment_manifest
                (game_id, bottle_name, mod_id, relative_path, staging_path, deploy_method, sha256, deployed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![game_id, bottle_name, mod_id, relative_path, staging_path, deploy_method, sha256, deployed_at],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Remove all deployment manifest entries for a mod.
    pub fn remove_deployment_entries_for_mod(&self, mod_id: i64) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();

        // First collect the relative paths
        let mut stmt = conn.prepare(
            "SELECT relative_path FROM deployment_manifest WHERE mod_id = ?1",
        )?;
        let paths: Vec<String> = stmt
            .query_map(params![mod_id], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        // Then delete
        conn.execute(
            "DELETE FROM deployment_manifest WHERE mod_id = ?1",
            params![mod_id],
        )?;

        Ok(paths)
    }

    /// Get deployment manifest for a game/bottle.
    pub fn get_deployment_manifest(
        &self,
        game_id: &str,
        bottle_name: &str,
    ) -> Result<Vec<DeploymentEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT dm.id, dm.game_id, dm.bottle_name, dm.mod_id, dm.relative_path,
                    dm.staging_path, dm.deploy_method, dm.sha256, dm.deployed_at,
                    COALESCE(im.name, 'Unknown') as mod_name
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
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT dm.id, dm.game_id, dm.bottle_name, dm.mod_id, dm.relative_path,
                    dm.staging_path, dm.deploy_method, dm.sha256, dm.deployed_at,
                    COALESCE(im.name, 'Unknown') as mod_name
             FROM deployment_manifest dm
             LEFT JOIN installed_mods im ON dm.mod_id = im.id
             WHERE dm.game_id = ?1 AND dm.bottle_name = ?2 AND dm.relative_path = ?3",
        )?;

        let mut rows = stmt.query_map(
            params![game_id, bottle_name, relative_path],
            |row| {
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
                })
            },
        )?;

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
        let conn = self.conn.lock().unwrap();
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
    pub fn get_file_hashes(
        &self,
        mod_id: i64,
    ) -> Result<Vec<(String, String, u64)>> {
        let conn = self.conn.lock().unwrap();
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

    // -- Mod field updates --------------------------------------------------

    /// Update the installed_files JSON for a mod.
    pub fn update_installed_files(&self, mod_id: i64, files: &[String]) -> Result<()> {
        let files_json = serde_json::to_string(files)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE installed_mods SET installed_files = ?1 WHERE id = ?2",
            params![files_json, mod_id],
        )?;
        Ok(())
    }

    /// Get the next priority value for a game/bottle (current max + 1, or 0 if empty).
    pub fn get_next_priority(&self, game_id: &str, bottle_name: &str) -> Result<i32> {
        let conn = self.conn.lock().unwrap();
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
    /// Returns files where multiple enabled mods want the same path.
    pub fn find_all_conflicts(
        &self,
        game_id: &str,
        bottle_name: &str,
    ) -> Result<Vec<FileConflict>> {
        let mods = self.list_mods(game_id, bottle_name)?;
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
                // Winner is the mod with highest priority
                let winner_mod_id = mods_info
                    .iter()
                    .max_by_key(|m| m.priority)
                    .map(|m| m.mod_id)
                    .unwrap_or(0);

                conflicts.push(FileConflict {
                    relative_path: path,
                    mods: mods_info,
                    winner_mod_id,
                });
            }
        }

        conflicts.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        Ok(conflicts)
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
            .add_mod("skyrim", "default", Some(1234), "Cool Armor", "1.0", "cool_armor.zip", &files)
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

        db.add_mod("skyrim", "default", None, "Mod A", "1.0", "a.zip", &[]).unwrap();
        db.add_mod("skyrim", "default", None, "Mod B", "1.0", "b.zip", &[]).unwrap();
        db.add_mod("fallout4", "default", None, "Mod C", "1.0", "c.zip", &[]).unwrap();

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

        let id_a = db.add_mod("skyrim", "default", None, "A", "1.0", "a.zip", &files_a).unwrap();
        let id_b = db.add_mod("skyrim", "default", None, "B", "1.0", "b.zip", &files_b).unwrap();

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
        db.add_mod("skyrim", "default", None, "Existing Mod", "1.0", "e.zip", &files).unwrap();

        let new_files = vec!["shared.txt".to_string(), "brand_new.txt".to_string()];
        let conflicts = db.find_conflicts("skyrim", "default", &new_files).unwrap();

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts["shared.txt"], "Existing Mod");
    }

    #[test]
    fn test_set_enabled() {
        let (db, _tmp) = test_db();

        let id = db.add_mod("skyrim", "default", None, "Toggle Me", "1.0", "t.zip", &[]).unwrap();
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
}
