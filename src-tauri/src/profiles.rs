//! Mod profiles system.
//!
//! Profiles allow users to save and switch between different sets of enabled
//! mods, mod priorities, and plugin load orders per game/bottle. Switching
//! profiles triggers a full purge-and-redeploy cycle.

use std::path::{Path, PathBuf};

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
pub fn list_profiles(db: &ModDatabase, game_id: &str, bottle_name: &str) -> Result<Vec<Profile>> {
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

/// Delete a profile by ID (works for active or inactive profiles).
pub fn delete_profile(db: &ModDatabase, profile_id: i64) -> Result<()> {
    let conn = db.conn().map_err(|e| ProfileError::Other(e.to_string()))?;
    let rows = conn.execute("DELETE FROM profiles WHERE id = ?1", params![profile_id])?;
    if rows == 0 {
        return Err(ProfileError::NotFound(profile_id));
    }
    Ok(())
}

/// Deactivate a profile (set is_active = 0).
pub fn deactivate_profile(db: &ModDatabase, game_id: &str, bottle_name: &str) -> Result<()> {
    let conn = db.conn().map_err(|e| ProfileError::Other(e.to_string()))?;
    conn.execute(
        "UPDATE profiles SET is_active = 0
         WHERE game_id = ?1 AND bottle_name = ?2 AND is_active = 1",
        params![game_id, bottle_name],
    )?;
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
///
/// Both updates run inside a transaction so that a crash between them cannot
/// leave every profile inactive.
pub fn set_active_profile(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    profile_id: i64,
) -> Result<()> {
    let conn = db.conn().map_err(|e| ProfileError::Other(e.to_string()))?;
    let tx = conn.unchecked_transaction()?;

    // Deactivate all profiles for this game/bottle
    tx.execute(
        "UPDATE profiles SET is_active = 0
         WHERE game_id = ?1 AND bottle_name = ?2",
        params![game_id, bottle_name],
    )?;

    // Activate the target
    let rows = tx.execute(
        "UPDATE profiles SET is_active = 1 WHERE id = ?1",
        params![profile_id],
    )?;

    if rows == 0 {
        // Roll back the deactivation — tx drops without commit
        return Err(ProfileError::NotFound(profile_id));
    }

    tx.commit()?;
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
// Wine prefix case-variant file sync
// ---------------------------------------------------------------------------

/// Write a file to both case variants in a Wine prefix for compatibility.
/// Wine is inconsistent about whether it reads `plugins.txt` or `Plugins.txt`.
pub fn write_case_variants(dir: &Path, filename: &str, content: &[u8]) -> Result<()> {
    // Write lowercase version
    let lower_path = dir.join(filename.to_lowercase());
    std::fs::write(&lower_path, content)
        .map_err(|e| ProfileError::Other(format!("Failed to write {}: {}", lower_path.display(), e)))?;

    // Write original case version (if different)
    let original_path = dir.join(filename);
    if original_path != lower_path {
        std::fs::write(&original_path, content)
            .map_err(|e| ProfileError::Other(format!("Failed to write {}: {}", original_path.display(), e)))?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// MO2 profile import (Wabbajack modlists)
// ---------------------------------------------------------------------------

/// An MO2 profile discovered in a Wabbajack install directory.
#[derive(Debug, Clone, Serialize)]
pub struct Mo2Profile {
    pub name: String,
    pub dir: PathBuf,
    pub has_modlist: bool,
    pub has_plugins: bool,
    pub mod_count: usize,
    pub plugin_count: usize,
}

/// Scan a Wabbajack install directory for MO2 profiles.
///
/// MO2 stores profiles at `<install>/profiles/<name>/` with files like
/// `modlist.txt`, `plugins.txt`, and INI files.
pub fn scan_mo2_profiles(install_dir: &Path) -> Vec<Mo2Profile> {
    let profiles_dir = install_dir.join("profiles");
    if !profiles_dir.is_dir() {
        return Vec::new();
    }

    let mut profiles = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&profiles_dir) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let dir = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();

            let modlist_path = dir.join("modlist.txt");
            let plugins_path = dir.join("plugins.txt");

            let has_modlist = modlist_path.exists();
            let has_plugins = plugins_path.exists();

            // Skip empty/invalid profile dirs
            if !has_modlist && !has_plugins {
                continue;
            }

            let mod_count = if has_modlist {
                parse_mo2_modlist(&modlist_path).len()
            } else {
                0
            };

            let plugin_count = if has_plugins {
                parse_mo2_plugins(&plugins_path).len()
            } else {
                0
            };

            profiles.push(Mo2Profile {
                name,
                dir,
                has_modlist,
                has_plugins,
                mod_count,
                plugin_count,
            });
        }
    }

    profiles.sort_by(|a, b| a.name.cmp(&b.name));
    log::info!(
        "Found {} MO2 profiles in {}",
        profiles.len(),
        install_dir.display()
    );
    profiles
}

/// Parse an MO2 `modlist.txt` file.
///
/// Format: one entry per line.
/// - `+ModName` = enabled
/// - `-ModName` = disabled
/// - `*ModName` = unmanaged/separator (skip)
/// - Lines starting with `#` are comments
///
/// Returns entries in order (first = highest priority in MO2's convention).
pub fn parse_mo2_modlist(path: &Path) -> Vec<(String, bool)> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Failed to read MO2 modlist.txt: {}", e);
            return Vec::new();
        }
    };

    let mut entries = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('+') {
            entries.push((line[1..].to_string(), true));
        } else if line.starts_with('-') {
            entries.push((line[1..].to_string(), false));
        }
        // Skip '*' (separators/unmanaged) and other prefixes
    }

    entries
}

/// Parse an MO2 `plugins.txt` file.
///
/// Format: one plugin per line.
/// - `*PluginName.esp` = enabled
/// - `PluginName.esp` (no prefix) = disabled
/// - Lines starting with `#` are comments
pub fn parse_mo2_plugins(path: &Path) -> Vec<(String, bool)> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Failed to read MO2 plugins.txt: {}", e);
            return Vec::new();
        }
    };

    let mut entries = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('*') {
            entries.push((line[1..].to_string(), true));
        } else {
            entries.push((line.to_string(), false));
        }
    }

    entries
}

/// Import MO2 profiles from a Wabbajack install into Corkscrew's profile system.
///
/// For each MO2 profile directory found:
/// 1. Parse `modlist.txt` → map mod names to installed mod IDs → set enable/priority
/// 2. Parse `plugins.txt` → import as plugin load order
/// 3. Create a Corkscrew profile with this state
///
/// Returns the names of successfully imported profiles.
pub fn import_mo2_profiles(
    db: &ModDatabase,
    install_dir: &Path,
    game_id: &str,
    bottle_name: &str,
    modlist_name: &str,
) -> Vec<String> {
    let mo2_profiles = scan_mo2_profiles(install_dir);
    if mo2_profiles.is_empty() {
        log::debug!("No MO2 profiles found in {}", install_dir.display());
        return Vec::new();
    }

    // Get all installed mods for name matching
    let installed_mods = db
        .list_mods(game_id, bottle_name)
        .unwrap_or_default();

    let mut imported = Vec::new();

    for mo2_profile in &mo2_profiles {
        // Build profile name: "ModlistName - ProfileName"
        let profile_name = if mo2_profiles.len() == 1 {
            format!("wj:{}", modlist_name)
        } else {
            format!("wj:{} - {}", modlist_name, mo2_profile.name)
        };

        let profile_id = match create_profile(db, game_id, bottle_name, &profile_name) {
            Ok(id) => id,
            Err(ProfileError::DuplicateName(_)) => {
                log::info!("Profile '{}' already exists, skipping", profile_name);
                continue;
            }
            Err(e) => {
                log::warn!("Failed to create profile '{}': {}", profile_name, e);
                continue;
            }
        };

        // Import mod states from modlist.txt
        if mo2_profile.has_modlist {
            let mo2_mods = parse_mo2_modlist(&mo2_profile.dir.join("modlist.txt"));

            // MO2 modlist.txt lists mods top-to-bottom = highest to lowest priority.
            // We reverse so index 0 = lowest priority (matching our priority convention).
            let mut mod_states = Vec::new();
            let total = mo2_mods.len() as i32;

            for (i, (mod_name, enabled)) in mo2_mods.iter().enumerate() {
                // Try to match MO2 mod name to an installed Corkscrew mod.
                // MO2 mod names often match the staging folder name which we use as mod name.
                let mod_name_lower = mod_name.to_lowercase();
                if let Some(installed) = installed_mods.iter().find(|m| {
                    let installed_lower = m.name.to_lowercase();
                    // Prefer exact match, fall back to substring containment
                    installed_lower == mod_name_lower
                        || installed_lower.contains(&mod_name_lower)
                        || mod_name_lower.contains(&installed_lower)
                }) {
                    log::debug!(
                        "MO2 mod '{}' matched to installed mod '{}' (id={})",
                        mod_name, installed.name, installed.id
                    );
                    mod_states.push(ProfileModState {
                        mod_id: installed.id,
                        enabled: *enabled,
                        priority: total - i as i32, // Higher number = higher priority
                    });
                }
            }

            if !mod_states.is_empty() {
                if let Err(e) = save_mod_states(db, profile_id, &mod_states) {
                    log::warn!("Failed to save mod states for '{}': {}", profile_name, e);
                } else {
                    log::info!(
                        "Imported {} mod states for profile '{}'",
                        mod_states.len(),
                        profile_name
                    );
                }
            }
        }

        // Import plugin states from plugins.txt
        if mo2_profile.has_plugins {
            let mo2_plugins = parse_mo2_plugins(&mo2_profile.dir.join("plugins.txt"));

            let plugin_states: Vec<ProfilePluginState> = mo2_plugins
                .iter()
                .enumerate()
                .map(|(i, (filename, enabled))| ProfilePluginState {
                    plugin_filename: filename.clone(),
                    enabled: *enabled,
                    load_index: i as i32,
                })
                .collect();

            if !plugin_states.is_empty() {
                if let Err(e) = save_plugin_states(db, profile_id, &plugin_states) {
                    log::warn!(
                        "Failed to save plugin states for '{}': {}",
                        profile_name,
                        e
                    );
                } else {
                    log::info!(
                        "Imported {} plugin states for profile '{}'",
                        plugin_states.len(),
                        profile_name
                    );
                }
            }
        }

        imported.push(profile_name.clone());
        log::info!(
            "Created profile '{}' from MO2 profile '{}' ({} mods, {} plugins)",
            profile_name,
            mo2_profile.name,
            mo2_profile.mod_count,
            mo2_profile.plugin_count,
        );
    }

    // If only one profile was imported, activate it automatically
    if imported.len() == 1 {
        if let Ok(profiles) = list_profiles(db, game_id, bottle_name) {
            if let Some(p) = profiles.iter().find(|p| p.name == imported[0]) {
                let _ = set_active_profile(db, game_id, bottle_name, p.id);
                log::info!("Auto-activated profile '{}'", imported[0]);
            }
        }
    }

    imported
}

// ---------------------------------------------------------------------------
// Profile save management
// ---------------------------------------------------------------------------

/// Get the directory where a profile's saves are backed up.
///
/// Layout: `<staging_root>/saves/<game_id>/<sanitized_bottle>/<profile_id>/`
pub fn profile_saves_dir(game_id: &str, bottle_name: &str, profile_id: i64) -> std::path::PathBuf {
    let sanitized_bottle = bottle_name.replace(['/', '\\', ' '], "_");
    crate::config::data_dir()
        .join("saves")
        .join(game_id)
        .join(sanitized_bottle)
        .join(profile_id.to_string())
}

/// Backup saves from the game save directory to the profile's save backup dir.
///
/// This copies all save files from `saves_dir` into the profile-specific backup.
/// Existing backup files for this profile are replaced.
pub fn backup_saves(
    profile_id: i64,
    game_id: &str,
    bottle_name: &str,
    saves_dir: &Path,
) -> Result<usize> {
    use std::fs;
    use walkdir::WalkDir;

    if !saves_dir.exists() {
        log::info!(
            "Saves dir does not exist for {}/{}, nothing to backup",
            game_id,
            bottle_name
        );
        return Ok(0);
    }

    let backup_dir = profile_saves_dir(game_id, bottle_name, profile_id);

    // Clear existing backup
    if backup_dir.exists() {
        fs::remove_dir_all(&backup_dir)
            .map_err(|e| ProfileError::Other(format!("Failed to clear save backup: {}", e)))?;
    }
    fs::create_dir_all(&backup_dir)
        .map_err(|e| ProfileError::Other(format!("Failed to create save backup dir: {}", e)))?;

    let mut count = 0;
    for entry in WalkDir::new(saves_dir).into_iter().filter_map(|e| e.ok()) {
        let src = entry.path();
        let relative = match src.strip_prefix(saves_dir) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let dest = backup_dir.join(relative);

        if entry.file_type().is_dir() {
            let _ = fs::create_dir_all(&dest);
        } else if entry.file_type().is_file() {
            if let Some(parent) = dest.parent() {
                let _ = fs::create_dir_all(parent);
            }
            fs::copy(src, &dest).map_err(|e| {
                ProfileError::Other(format!("Failed to copy save {}: {}", src.display(), e))
            })?;
            count += 1;
        }
    }

    log::info!(
        "Backed up {} save files for profile {} ({}/{})",
        count,
        profile_id,
        game_id,
        bottle_name
    );

    Ok(count)
}

/// Restore saves from a profile's backup into the game save directory.
///
/// This clears the current save directory and copies the backed-up files in.
/// If the profile has no backed-up saves, the save directory is left untouched.
pub fn restore_saves(
    profile_id: i64,
    game_id: &str,
    bottle_name: &str,
    saves_dir: &Path,
) -> Result<usize> {
    use std::fs;
    use walkdir::WalkDir;

    let backup_dir = profile_saves_dir(game_id, bottle_name, profile_id);
    if !backup_dir.exists() {
        log::info!(
            "No save backup exists for profile {} ({}/{})",
            profile_id,
            game_id,
            bottle_name
        );
        return Ok(0);
    }

    // Clear current saves
    if saves_dir.exists() {
        fs::remove_dir_all(saves_dir)
            .map_err(|e| ProfileError::Other(format!("Failed to clear save dir: {}", e)))?;
    }
    fs::create_dir_all(saves_dir)
        .map_err(|e| ProfileError::Other(format!("Failed to create save dir: {}", e)))?;

    let mut count = 0;
    for entry in WalkDir::new(&backup_dir).into_iter().filter_map(|e| e.ok()) {
        let src = entry.path();
        let relative = match src.strip_prefix(&backup_dir) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let dest = saves_dir.join(relative);

        if entry.file_type().is_dir() {
            let _ = fs::create_dir_all(&dest);
        } else if entry.file_type().is_file() {
            if let Some(parent) = dest.parent() {
                let _ = fs::create_dir_all(parent);
            }
            fs::copy(src, &dest).map_err(|e| {
                ProfileError::Other(format!("Failed to restore save {}: {}", src.display(), e))
            })?;
            count += 1;
        }
    }

    log::info!(
        "Restored {} save files for profile {} ({}/{})",
        count,
        profile_id,
        game_id,
        bottle_name
    );

    Ok(count)
}

/// Get info about a profile's backed-up saves.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileSaveInfo {
    pub profile_id: i64,
    pub file_count: usize,
    pub total_size: u64,
    pub has_backup: bool,
}

/// Check if a profile has backed-up saves and get stats.
pub fn get_profile_save_info(profile_id: i64, game_id: &str, bottle_name: &str) -> ProfileSaveInfo {
    let backup_dir = profile_saves_dir(game_id, bottle_name, profile_id);

    if !backup_dir.exists() {
        return ProfileSaveInfo {
            profile_id,
            file_count: 0,
            total_size: 0,
            has_backup: false,
        };
    }

    let mut file_count = 0;
    let mut total_size: u64 = 0;
    for entry in walkdir::WalkDir::new(&backup_dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            file_count += 1;
            total_size += entry.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }

    ProfileSaveInfo {
        profile_id,
        file_count,
        total_size,
        has_backup: file_count > 0,
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
            ProfileModState {
                mod_id: 1,
                enabled: true,
                priority: 0,
            },
            ProfileModState {
                mod_id: 2,
                enabled: false,
                priority: 1,
            },
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

    #[test]
    fn parse_mo2_modlist_basic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("modlist.txt");
        std::fs::write(
            &path,
            "# This file was automatically generated by Mod Organizer.\n\
             +SkyUI\n\
             -Immersive Armors\n\
             +USSEP\n\
             *Separator_Visuals\n\
             +RaceMenu\n",
        )
        .unwrap();

        let entries = parse_mo2_modlist(&path);
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0], ("SkyUI".to_string(), true));
        assert_eq!(entries[1], ("Immersive Armors".to_string(), false));
        assert_eq!(entries[2], ("USSEP".to_string(), true));
        assert_eq!(entries[3], ("RaceMenu".to_string(), true));
    }

    #[test]
    fn parse_mo2_plugins_basic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plugins.txt");
        std::fs::write(
            &path,
            "# This file is used by Mod Organizer.\n\
             *Skyrim.esm\n\
             *Update.esm\n\
             *Dawnguard.esm\n\
             SomeDisabled.esp\n\
             *SkyUI_SE.esp\n",
        )
        .unwrap();

        let entries = parse_mo2_plugins(&path);
        assert_eq!(entries.len(), 5);
        assert_eq!(entries[0], ("Skyrim.esm".to_string(), true));
        assert_eq!(entries[3], ("SomeDisabled.esp".to_string(), false));
        assert_eq!(entries[4], ("SkyUI_SE.esp".to_string(), true));
    }

    #[test]
    fn scan_mo2_profiles_finds_profiles() {
        let dir = tempfile::tempdir().unwrap();
        let profiles_dir = dir.path().join("profiles");

        // Create a Default profile
        let default_dir = profiles_dir.join("Default");
        std::fs::create_dir_all(&default_dir).unwrap();
        std::fs::write(default_dir.join("modlist.txt"), "+SkyUI\n+USSEP\n").unwrap();
        std::fs::write(
            default_dir.join("plugins.txt"),
            "*Skyrim.esm\n*SkyUI_SE.esp\n",
        )
        .unwrap();

        // Create a Performance profile
        let perf_dir = profiles_dir.join("Performance");
        std::fs::create_dir_all(&perf_dir).unwrap();
        std::fs::write(perf_dir.join("modlist.txt"), "+SkyUI\n-USSEP\n").unwrap();

        // Create an empty dir (should be skipped)
        std::fs::create_dir_all(profiles_dir.join("Empty")).unwrap();

        let profiles = scan_mo2_profiles(dir.path());
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].name, "Default");
        assert!(profiles[0].has_modlist);
        assert!(profiles[0].has_plugins);
        assert_eq!(profiles[0].mod_count, 2);
        assert_eq!(profiles[0].plugin_count, 2);
        assert_eq!(profiles[1].name, "Performance");
        assert!(profiles[1].has_modlist);
        assert!(!profiles[1].has_plugins);
    }

    #[test]
    fn parse_mo2_modlist_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("modlist.txt");
        std::fs::write(&path, "# Empty\n").unwrap();
        assert!(parse_mo2_modlist(&path).is_empty());
    }

    #[test]
    fn parse_mo2_modlist_missing_file() {
        let path = Path::new("/nonexistent/modlist.txt");
        assert!(parse_mo2_modlist(path).is_empty());
    }
}
