//! Mod profiles system.
//!
//! Profiles allow users to save and switch between different sets of enabled
//! mods, mod priorities, and plugin load orders per game/bottle. Switching
//! profiles triggers a full purge-and-redeploy cycle.

use std::path::Path;

use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::database::ModDatabase;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("Profile not found: {0}")]
    NotFound(i64),

    #[error("Profile name already exists: {0}")]
    DuplicateName(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, ProfileError>;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    pub id: i64,
    pub game_id: String,
    pub bottle_name: String,
    pub name: String,
    pub is_active: bool,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileModState {
    pub mod_id: i64,
    pub enabled: bool,
    pub priority: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfilePluginState {
    pub plugin_filename: String,
    pub enabled: bool,
    pub load_index: i32,
}

// ---------------------------------------------------------------------------
// Schema initialization
// ---------------------------------------------------------------------------

/// Create the profile tables. Called once during app startup.
pub fn init_schema(db: &ModDatabase) -> std::result::Result<(), String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    init_schema_with_conn(&conn).map_err(|e| e.to_string())
}

fn init_schema_with_conn(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS profiles (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            game_id     TEXT    NOT NULL,
            bottle_name TEXT    NOT NULL,
            name        TEXT    NOT NULL,
            is_active   INTEGER NOT NULL DEFAULT 0,
            created_at  TEXT    NOT NULL,
            UNIQUE(game_id, bottle_name, name)
        );

        CREATE TABLE IF NOT EXISTS profile_mods (
            profile_id INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
            mod_id     INTEGER NOT NULL,
            enabled    INTEGER NOT NULL DEFAULT 1,
            priority   INTEGER NOT NULL DEFAULT 0,
            UNIQUE(profile_id, mod_id)
        );

        CREATE TABLE IF NOT EXISTS profile_plugins (
            profile_id      INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
            plugin_filename TEXT    NOT NULL,
            enabled         INTEGER NOT NULL DEFAULT 1,
            load_index      INTEGER NOT NULL,
            UNIQUE(profile_id, plugin_filename)
        );",
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// CRUD operations
// ---------------------------------------------------------------------------

/// Create a new profile. Returns its ID.
pub fn create_profile(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    name: &str,
) -> Result<i64> {
    let conn = db.conn().map_err(|e| ProfileError::Other(e.to_string()))?;
    let created_at = Utc::now().to_rfc3339();

    match conn.execute(
        "INSERT INTO profiles (game_id, bottle_name, name, is_active, created_at)
         VALUES (?1, ?2, ?3, 0, ?4)",
        params![game_id, bottle_name, name, created_at],
    ) {
        Ok(_) => Ok(conn.last_insert_rowid()),
        Err(e) => {
            if e.to_string().contains("UNIQUE constraint failed") {
                Err(ProfileError::DuplicateName(name.to_string()))
            } else {
                Err(ProfileError::Sqlite(e))
            }
        }
    }
}

/// List all profiles for a game/bottle.
pub fn list_profiles(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
) -> Result<Vec<Profile>> {
    let conn = db.conn().map_err(|e| ProfileError::Other(e.to_string()))?;
    let mut stmt = conn.prepare(
        "SELECT id, game_id, bottle_name, name, is_active, created_at
         FROM profiles
         WHERE game_id = ?1 AND bottle_name = ?2
         ORDER BY created_at ASC",
    )?;

    let rows = stmt.query_map(params![game_id, bottle_name], |row| {
        let is_active_int: i64 = row.get(4)?;
        Ok(Profile {
            id: row.get(0)?,
            game_id: row.get(1)?,
            bottle_name: row.get(2)?,
            name: row.get(3)?,
            is_active: is_active_int != 0,
            created_at: row.get(5)?,
        })
    })?;

    let mut profiles = Vec::new();
    for row in rows {
        profiles.push(row?);
    }
    Ok(profiles)
}

/// Delete a profile by ID.
pub fn delete_profile(db: &ModDatabase, profile_id: i64) -> Result<()> {
    let conn = db.conn().map_err(|e| ProfileError::Other(e.to_string()))?;
    let rows = conn.execute(
        "DELETE FROM profiles WHERE id = ?1",
        params![profile_id],
    )?;
    if rows == 0 {
        return Err(ProfileError::NotFound(profile_id));
    }
    Ok(())
}

/// Rename a profile.
pub fn rename_profile(db: &ModDatabase, profile_id: i64, new_name: &str) -> Result<()> {
    let conn = db.conn().map_err(|e| ProfileError::Other(e.to_string()))?;
    match conn.execute(
        "UPDATE profiles SET name = ?1 WHERE id = ?2",
        params![new_name, profile_id],
    ) {
        Ok(0) => Err(ProfileError::NotFound(profile_id)),
        Ok(_) => Ok(()),
        Err(e) => {
            if e.to_string().contains("UNIQUE constraint failed") {
                Err(ProfileError::DuplicateName(new_name.to_string()))
            } else {
                Err(ProfileError::Sqlite(e))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Snapshot: save current mod/plugin state into a profile
// ---------------------------------------------------------------------------

/// Save the current mod states into a profile.
/// This replaces any existing mod states in the profile.
pub fn save_mod_states(
    db: &ModDatabase,
    profile_id: i64,
    states: &[ProfileModState],
) -> Result<()> {
    let conn = db.conn().map_err(|e| ProfileError::Other(e.to_string()))?;

    conn.execute(
        "DELETE FROM profile_mods WHERE profile_id = ?1",
        params![profile_id],
    )?;

    let mut stmt = conn.prepare(
        "INSERT INTO profile_mods (profile_id, mod_id, enabled, priority)
         VALUES (?1, ?2, ?3, ?4)",
    )?;

    for s in states {
        stmt.execute(params![profile_id, s.mod_id, s.enabled as i64, s.priority])?;
    }

    Ok(())
}

/// Save the current plugin states into a profile.
pub fn save_plugin_states(
    db: &ModDatabase,
    profile_id: i64,
    states: &[ProfilePluginState],
) -> Result<()> {
    let conn = db.conn().map_err(|e| ProfileError::Other(e.to_string()))?;

    conn.execute(
        "DELETE FROM profile_plugins WHERE profile_id = ?1",
        params![profile_id],
    )?;

    let mut stmt = conn.prepare(
        "INSERT INTO profile_plugins (profile_id, plugin_filename, enabled, load_index)
         VALUES (?1, ?2, ?3, ?4)",
    )?;

    for s in states {
        stmt.execute(params![
            profile_id,
            s.plugin_filename,
            s.enabled as i64,
            s.load_index,
        ])?;
    }

    Ok(())
}

/// Get saved mod states for a profile.
pub fn get_mod_states(db: &ModDatabase, profile_id: i64) -> Result<Vec<ProfileModState>> {
    let conn = db.conn().map_err(|e| ProfileError::Other(e.to_string()))?;
    let mut stmt = conn.prepare(
        "SELECT mod_id, enabled, priority FROM profile_mods
         WHERE profile_id = ?1 ORDER BY priority ASC",
    )?;

    let rows = stmt.query_map(params![profile_id], |row| {
        let enabled_int: i64 = row.get(1)?;
        Ok(ProfileModState {
            mod_id: row.get(0)?,
            enabled: enabled_int != 0,
            priority: row.get(2)?,
        })
    })?;

    let mut states = Vec::new();
    for row in rows {
        states.push(row?);
    }
    Ok(states)
}

/// Get saved plugin states for a profile.
pub fn get_plugin_states(db: &ModDatabase, profile_id: i64) -> Result<Vec<ProfilePluginState>> {
    let conn = db.conn().map_err(|e| ProfileError::Other(e.to_string()))?;
    let mut stmt = conn.prepare(
        "SELECT plugin_filename, enabled, load_index FROM profile_plugins
         WHERE profile_id = ?1 ORDER BY load_index ASC",
    )?;

    let rows = stmt.query_map(params![profile_id], |row| {
        let enabled_int: i64 = row.get(1)?;
        Ok(ProfilePluginState {
            plugin_filename: row.get(0)?,
            enabled: enabled_int != 0,
            load_index: row.get(2)?,
        })
    })?;

    let mut states = Vec::new();
    for row in rows {
        states.push(row?);
    }
    Ok(states)
}

// ---------------------------------------------------------------------------
// Profile activation
// ---------------------------------------------------------------------------

/// Set which profile is active for a game/bottle (deactivates all others).
pub fn set_active_profile(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    profile_id: i64,
) -> Result<()> {
    let conn = db.conn().map_err(|e| ProfileError::Other(e.to_string()))?;

    // Deactivate all profiles for this game/bottle
    conn.execute(
        "UPDATE profiles SET is_active = 0
         WHERE game_id = ?1 AND bottle_name = ?2",
        params![game_id, bottle_name],
    )?;

    // Activate the target
    let rows = conn.execute(
        "UPDATE profiles SET is_active = 1 WHERE id = ?1",
        params![profile_id],
    )?;

    if rows == 0 {
        return Err(ProfileError::NotFound(profile_id));
    }

    Ok(())
}

/// Get the currently active profile for a game/bottle (if any).
pub fn get_active_profile(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
) -> Result<Option<Profile>> {
    let conn = db.conn().map_err(|e| ProfileError::Other(e.to_string()))?;
    let mut stmt = conn.prepare(
        "SELECT id, game_id, bottle_name, name, is_active, created_at
         FROM profiles
         WHERE game_id = ?1 AND bottle_name = ?2 AND is_active = 1
         LIMIT 1",
    )?;

    let mut rows = stmt.query_map(params![game_id, bottle_name], |row| {
        Ok(Profile {
            id: row.get(0)?,
            game_id: row.get(1)?,
            bottle_name: row.get(2)?,
            name: row.get(3)?,
            is_active: true,
            created_at: row.get(5)?,
        })
    })?;

    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

/// Snapshot the current live state into a profile.
///
/// Reads installed_mods (enabled, priority) and the plugins file to
/// populate profile_mods and profile_plugins.
pub fn snapshot_current_state(
    db: &ModDatabase,
    profile_id: i64,
    game_id: &str,
    bottle_name: &str,
    plugins_file: Option<&Path>,
) -> Result<()> {
    // Save mod states
    let mods = db
        .list_mods(game_id, bottle_name)
        .map_err(|e| ProfileError::Other(e.to_string()))?;

    let mod_states: Vec<ProfileModState> = mods
        .iter()
        .map(|m| ProfileModState {
            mod_id: m.id,
            enabled: m.enabled,
            priority: m.install_priority,
        })
        .collect();

    save_mod_states(db, profile_id, &mod_states)?;

    // Save plugin states
    if let Some(pf) = plugins_file {
        if pf.exists() {
            let entries = crate::plugins::skyrim_plugins::read_plugins_txt(pf)
                .map_err(|e| ProfileError::Other(e.to_string()))?;

            let plugin_states: Vec<ProfilePluginState> = entries
                .iter()
                .enumerate()
                .map(|(i, e)| ProfilePluginState {
                    plugin_filename: e.filename.clone(),
                    enabled: e.enabled,
                    load_index: i as i32,
                })
                .collect();

            save_plugin_states(db, profile_id, &plugin_states)?;
        }
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
        let db_path = tmp.path().join("test.db");
        let db = ModDatabase::new(&db_path).unwrap();
        init_schema(&db).unwrap();
        (db, tmp)
    }

    #[test]
    fn create_and_list_profiles() {
        let (db, _tmp) = test_db();

        let id1 = create_profile(&db, "skyrimse", "Gaming", "Default").unwrap();
        let id2 = create_profile(&db, "skyrimse", "Gaming", "Modded").unwrap();

        let profiles = list_profiles(&db, "skyrimse", "Gaming").unwrap();
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].id, id1);
        assert_eq!(profiles[0].name, "Default");
        assert_eq!(profiles[1].id, id2);
        assert_eq!(profiles[1].name, "Modded");
    }

    #[test]
    fn duplicate_name_errors() {
        let (db, _tmp) = test_db();

        create_profile(&db, "skyrimse", "Gaming", "Default").unwrap();
        let result = create_profile(&db, "skyrimse", "Gaming", "Default");
        assert!(matches!(result, Err(ProfileError::DuplicateName(_))));
    }

    #[test]
    fn delete_profile_removes_it() {
        let (db, _tmp) = test_db();

        let id = create_profile(&db, "skyrimse", "Gaming", "Test").unwrap();
        delete_profile(&db, id).unwrap();

        let profiles = list_profiles(&db, "skyrimse", "Gaming").unwrap();
        assert!(profiles.is_empty());
    }

    #[test]
    fn rename_profile_works() {
        let (db, _tmp) = test_db();

        let id = create_profile(&db, "skyrimse", "Gaming", "Old Name").unwrap();
        rename_profile(&db, id, "New Name").unwrap();

        let profiles = list_profiles(&db, "skyrimse", "Gaming").unwrap();
        assert_eq!(profiles[0].name, "New Name");
    }

    #[test]
    fn save_and_get_mod_states() {
        let (db, _tmp) = test_db();
        let profile_id = create_profile(&db, "skyrimse", "Gaming", "Test").unwrap();

        let states = vec![
            ProfileModState { mod_id: 1, enabled: true, priority: 0 },
            ProfileModState { mod_id: 2, enabled: false, priority: 1 },
        ];

        save_mod_states(&db, profile_id, &states).unwrap();
        let loaded = get_mod_states(&db, profile_id).unwrap();

        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].mod_id, 1);
        assert!(loaded[0].enabled);
        assert_eq!(loaded[1].mod_id, 2);
        assert!(!loaded[1].enabled);
    }

    #[test]
    fn save_and_get_plugin_states() {
        let (db, _tmp) = test_db();
        let profile_id = create_profile(&db, "skyrimse", "Gaming", "Test").unwrap();

        let states = vec![
            ProfilePluginState {
                plugin_filename: "Skyrim.esm".to_string(),
                enabled: true,
                load_index: 0,
            },
            ProfilePluginState {
                plugin_filename: "SkyUI_SE.esp".to_string(),
                enabled: true,
                load_index: 1,
            },
        ];

        save_plugin_states(&db, profile_id, &states).unwrap();
        let loaded = get_plugin_states(&db, profile_id).unwrap();

        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].plugin_filename, "Skyrim.esm");
        assert_eq!(loaded[1].plugin_filename, "SkyUI_SE.esp");
    }

    #[test]
    fn set_active_profile_deactivates_others() {
        let (db, _tmp) = test_db();

        let id1 = create_profile(&db, "skyrimse", "Gaming", "A").unwrap();
        let id2 = create_profile(&db, "skyrimse", "Gaming", "B").unwrap();

        set_active_profile(&db, "skyrimse", "Gaming", id1).unwrap();
        let active = get_active_profile(&db, "skyrimse", "Gaming").unwrap();
        assert_eq!(active.unwrap().id, id1);

        set_active_profile(&db, "skyrimse", "Gaming", id2).unwrap();
        let active = get_active_profile(&db, "skyrimse", "Gaming").unwrap();
        assert_eq!(active.unwrap().id, id2);

        // id1 should no longer be active
        let profiles = list_profiles(&db, "skyrimse", "Gaming").unwrap();
        let p1 = profiles.iter().find(|p| p.id == id1).unwrap();
        assert!(!p1.is_active);
    }
}
