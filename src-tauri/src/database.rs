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
    pub name: String,
    pub version: String,
    pub archive_name: String,
    pub installed_files: Vec<String>,
    pub installed_at: String,
    pub enabled: bool,
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
    /// directories as needed, and initialise the schema.
    pub fn new(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path)?;

        // Enable WAL mode for better concurrent-read performance.
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;

        Self::init_schema(&conn)?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Create the `installed_mods` table and associated index if they do not
    /// already exist.
    fn init_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(
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
        Ok(())
    }

    // -- helpers ------------------------------------------------------------

    /// Build an `InstalledMod` from the current row of a prepared statement.
    /// Column order must match the SELECT used by the caller.
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
            name: row.get(4)?,
            version: row.get(5)?,
            archive_name: row.get(6)?,
            installed_files,
            installed_at: row.get(8)?,
            enabled: enabled_int != 0,
        })
    }

    /// The column list used in every SELECT on `installed_mods`.
    const SELECT_COLUMNS: &'static str =
        "id, game_id, bottle_name, nexus_mod_id, name, version, \
         archive_name, installed_files, installed_at, enabled";

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
