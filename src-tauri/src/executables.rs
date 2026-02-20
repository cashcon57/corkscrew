//! Custom executable management for games.
//!
//! Allows users to register custom executables per game/bottle combination
//! (e.g., a manually installed SKSE loader, an alternative game launcher,
//! or a different Skyrim executable).

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::database::ModDatabase;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CustomExecutable {
    pub id: i64,
    pub game_id: String,
    pub bottle_name: String,
    pub name: String,
    pub exe_path: String,
    pub working_dir: Option<String>,
    pub args: Option<String>,
    pub is_default: bool,
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

/// Create the `custom_executables` table if it does not exist.
pub fn init_schema(db: &ModDatabase) -> Result<(), String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS custom_executables (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            game_id     TEXT    NOT NULL,
            bottle_name TEXT    NOT NULL,
            name        TEXT    NOT NULL,
            exe_path    TEXT    NOT NULL,
            working_dir TEXT,
            args        TEXT,
            is_default  INTEGER NOT NULL DEFAULT 0,
            UNIQUE(game_id, bottle_name, name)
        );

        CREATE INDEX IF NOT EXISTS idx_custom_exe_game_bottle
            ON custom_executables (game_id, bottle_name);",
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// CRUD operations
// ---------------------------------------------------------------------------

/// Add a custom executable. Returns its ID.
pub fn add_executable(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    name: &str,
    exe_path: &str,
    working_dir: Option<&str>,
    args: Option<&str>,
) -> Result<i64, String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO custom_executables
            (game_id, bottle_name, name, exe_path, working_dir, args, is_default)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
        params![game_id, bottle_name, name, exe_path, working_dir, args],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

/// Remove a custom executable by ID.
pub fn remove_executable(db: &ModDatabase, exe_id: i64) -> Result<(), String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM custom_executables WHERE id = ?1",
        params![exe_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// List all custom executables for a game/bottle.
pub fn list_executables(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
) -> Result<Vec<CustomExecutable>, String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, game_id, bottle_name, name, exe_path, working_dir, args, is_default
             FROM custom_executables
             WHERE game_id = ?1 AND bottle_name = ?2
             ORDER BY name",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map(params![game_id, bottle_name], |row| {
            let is_default_int: i64 = row.get(7)?;
            Ok(CustomExecutable {
                id: row.get(0)?,
                game_id: row.get(1)?,
                bottle_name: row.get(2)?,
                name: row.get(3)?,
                exe_path: row.get(4)?,
                working_dir: row.get(5)?,
                args: row.get(6)?,
                is_default: is_default_int != 0,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut exes = Vec::new();
    for row in rows {
        exes.push(row.map_err(|e| e.to_string())?);
    }
    Ok(exes)
}

/// Set a specific executable as the default for its game/bottle.
/// Clears default from all others in the same game/bottle.
pub fn set_default_executable(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    exe_id: i64,
) -> Result<(), String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    // Clear existing default
    conn.execute(
        "UPDATE custom_executables SET is_default = 0
         WHERE game_id = ?1 AND bottle_name = ?2",
        params![game_id, bottle_name],
    )
    .map_err(|e| e.to_string())?;
    // Set new default
    conn.execute(
        "UPDATE custom_executables SET is_default = 1 WHERE id = ?1",
        params![exe_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Clear the default executable for a game/bottle (revert to built-in).
pub fn clear_default_executable(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
) -> Result<(), String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE custom_executables SET is_default = 0
         WHERE game_id = ?1 AND bottle_name = ?2",
        params![game_id, bottle_name],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Get the default custom executable for a game/bottle, if one is set.
pub fn get_default_executable(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
) -> Result<Option<CustomExecutable>, String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, game_id, bottle_name, name, exe_path, working_dir, args, is_default
             FROM custom_executables
             WHERE game_id = ?1 AND bottle_name = ?2 AND is_default = 1
             LIMIT 1",
        )
        .map_err(|e| e.to_string())?;

    let mut rows = stmt
        .query_map(params![game_id, bottle_name], |row| {
            let is_default_int: i64 = row.get(7)?;
            Ok(CustomExecutable {
                id: row.get(0)?,
                game_id: row.get(1)?,
                bottle_name: row.get(2)?,
                name: row.get(3)?,
                exe_path: row.get(4)?,
                working_dir: row.get(5)?,
                args: row.get(6)?,
                is_default: is_default_int != 0,
            })
        })
        .map_err(|e| e.to_string())?;

    match rows.next() {
        Some(row) => Ok(Some(row.map_err(|e| e.to_string())?)),
        None => Ok(None),
    }
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
    fn add_and_list_executables() {
        let (db, _tmp) = test_db();

        let id = add_executable(
            &db,
            "skyrimse",
            "Gaming",
            "SKSE Manual",
            "/path/to/skse64_loader.exe",
            None,
            None,
        )
        .unwrap();
        assert!(id > 0);

        let exes = list_executables(&db, "skyrimse", "Gaming").unwrap();
        assert_eq!(exes.len(), 1);
        assert_eq!(exes[0].name, "SKSE Manual");
        assert_eq!(exes[0].exe_path, "/path/to/skse64_loader.exe");
        assert!(!exes[0].is_default);
    }

    #[test]
    fn set_and_get_default() {
        let (db, _tmp) = test_db();

        let id1 = add_executable(
            &db,
            "skyrimse",
            "Gaming",
            "Exe A",
            "/path/a.exe",
            None,
            None,
        )
        .unwrap();
        let _id2 = add_executable(
            &db,
            "skyrimse",
            "Gaming",
            "Exe B",
            "/path/b.exe",
            None,
            None,
        )
        .unwrap();

        // No default initially
        let def = get_default_executable(&db, "skyrimse", "Gaming").unwrap();
        assert!(def.is_none());

        // Set default
        set_default_executable(&db, "skyrimse", "Gaming", id1).unwrap();
        let def = get_default_executable(&db, "skyrimse", "Gaming").unwrap();
        assert!(def.is_some());
        assert_eq!(def.unwrap().name, "Exe A");
    }

    #[test]
    fn clear_default() {
        let (db, _tmp) = test_db();

        let id = add_executable(
            &db,
            "skyrimse",
            "Gaming",
            "Test",
            "/path/test.exe",
            None,
            None,
        )
        .unwrap();

        set_default_executable(&db, "skyrimse", "Gaming", id).unwrap();
        assert!(get_default_executable(&db, "skyrimse", "Gaming")
            .unwrap()
            .is_some());

        clear_default_executable(&db, "skyrimse", "Gaming").unwrap();
        assert!(get_default_executable(&db, "skyrimse", "Gaming")
            .unwrap()
            .is_none());
    }

    #[test]
    fn remove_executable_works() {
        let (db, _tmp) = test_db();

        let id = add_executable(
            &db,
            "skyrimse",
            "Gaming",
            "To Remove",
            "/path/remove.exe",
            None,
            None,
        )
        .unwrap();

        assert_eq!(
            list_executables(&db, "skyrimse", "Gaming").unwrap().len(),
            1
        );
        remove_executable(&db, id).unwrap();
        assert_eq!(
            list_executables(&db, "skyrimse", "Gaming").unwrap().len(),
            0
        );
    }

    #[test]
    fn different_game_bottles_are_isolated() {
        let (db, _tmp) = test_db();

        add_executable(&db, "skyrimse", "Bottle1", "A", "/a.exe", None, None).unwrap();
        add_executable(&db, "skyrimse", "Bottle2", "B", "/b.exe", None, None).unwrap();
        add_executable(&db, "fallout4", "Bottle1", "C", "/c.exe", None, None).unwrap();

        assert_eq!(
            list_executables(&db, "skyrimse", "Bottle1").unwrap().len(),
            1
        );
        assert_eq!(
            list_executables(&db, "skyrimse", "Bottle2").unwrap().len(),
            1
        );
        assert_eq!(
            list_executables(&db, "fallout4", "Bottle1").unwrap().len(),
            1
        );
    }
}
