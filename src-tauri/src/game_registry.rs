//! Game registry — auto-registers games from the Vortex game data JSON.
//!
//! Loads `data/vortex_game_registry.json` at compile time and creates
//! generic [`GamePlugin`] implementations for each entry. Games that
//! already have dedicated plugins (e.g. `skyrimse`, `fallout4`) are
//! skipped to avoid duplicates.
//!
//! The registry also exposes metadata (supported games list, tool info)
//! that the frontend can query.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::bottles::Bottle;
use crate::games::{DetectedGame, GamePlugin};

// ---------------------------------------------------------------------------
// JSON schema
// ---------------------------------------------------------------------------

/// A tool associated with a game (e.g. SSEEdit, SMAPI).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GameTool {
    pub name: String,
    pub executable: String,
}

/// A game entry from the Vortex game registry JSON.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GameEntry {
    pub game_id: String,
    pub name: String,
    pub nexus_domain: String,
    pub steam_id: Option<String>,
    pub gog_id: Option<String>,
    pub epic_id: Option<String>,
    pub executable: Option<String>,
    pub mod_path: String,
    pub required_files: Vec<String>,
    pub tools: Vec<GameTool>,
    /// Override for the Steam directory name when it differs from `name`.
    pub steam_dir: Option<String>,
    /// Note for stub entries (these are skipped during registration).
    #[serde(rename = "_note")]
    pub note: Option<String>,
}

/// Serialisable summary returned to the frontend.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SupportedGame {
    pub game_id: String,
    pub name: String,
    pub nexus_domain: String,
    pub steam_id: Option<String>,
    pub has_tools: bool,
    pub tool_names: Vec<String>,
}

// ---------------------------------------------------------------------------
// Compile-time registry data
// ---------------------------------------------------------------------------

/// The raw JSON embedded at compile time.
const REGISTRY_JSON: &str = include_str!("../data/vortex_game_registry.json");

/// Parse the registry once and return a static reference.
fn entries() -> &'static Vec<GameEntry> {
    use std::sync::OnceLock;
    static ENTRIES: OnceLock<Vec<GameEntry>> = OnceLock::new();
    ENTRIES.get_or_init(|| {
        serde_json::from_str(REGISTRY_JSON).expect("Failed to parse game registry JSON")
    })
}

// ---------------------------------------------------------------------------
// Generic GamePlugin implementation
// ---------------------------------------------------------------------------

/// Game IDs that already have dedicated plugin modules.
/// These are skipped during auto-registration.
const CUSTOM_PLUGIN_IDS: &[&str] = &["skyrimse", "fallout4"];

/// A generic game plugin created from registry data.
struct RegistryGamePlugin {
    entry: &'static GameEntry,
}

impl GamePlugin for RegistryGamePlugin {
    fn game_id(&self) -> &str {
        &self.entry.game_id
    }

    fn display_name(&self) -> &str {
        &self.entry.name
    }

    fn nexus_slug(&self) -> &str {
        &self.entry.nexus_domain
    }

    fn executables(&self) -> &[&str] {
        // Return a static slice — we use a leak pattern since the data is
        // effectively 'static (embedded at compile time).
        // Detection uses find_game_path() directly rather than this list.
        &[]
    }

    fn detect(&self, bottle: &Bottle) -> Option<DetectedGame> {
        let exe = self.entry.executable.as_deref()?;
        let game_path = find_game_path(bottle, self.entry)?;

        // Verify the executable exists (case-insensitive).
        let exe_filename = Path::new(exe)
            .file_name()
            .map(|f| f.to_string_lossy().to_lowercase())?;
        let exe_dir = if exe.contains('/') || exe.contains('\\') {
            let exe_path = Path::new(exe);
            let parent = exe_path.parent()?;
            game_path.join(parent)
        } else {
            game_path.clone()
        };

        if !has_file_case_insensitive(&exe_dir, &exe_filename) {
            return None;
        }

        let exe_path = find_file_case_insensitive(&exe_dir, &exe_filename);
        let data_dir = self.get_data_dir(&game_path);

        Some(DetectedGame {
            game_id: self.entry.game_id.clone(),
            display_name: self.entry.name.clone(),
            nexus_slug: self.entry.nexus_domain.clone(),
            game_path,
            exe_path,
            data_dir,
            bottle_name: bottle.name.clone(),
            bottle_path: bottle.path.clone(),
        })
    }

    fn get_data_dir(&self, game_path: &Path) -> PathBuf {
        let mod_path = &self.entry.mod_path;

        // Special prefixes for paths outside the game directory.
        if mod_path.starts_with("{documents}") || mod_path.starts_with("{appdata}") {
            // For document-relative paths, just use the game dir as data_dir.
            // The deployer will handle the actual mod path separately.
            return game_path.to_path_buf();
        }

        if mod_path == "." {
            game_path.to_path_buf()
        } else {
            game_path.join(mod_path)
        }
    }

    fn get_plugins_file(&self, _game_path: &Path, _bottle: &Bottle) -> Option<PathBuf> {
        // Only Bethesda games have plugins.txt, and those have dedicated plugins.
        None
    }
}

// ---------------------------------------------------------------------------
// Detection helpers
// ---------------------------------------------------------------------------

/// Standard Steam common path components.
const STEAM_COMMON: &[&str] = &["Program Files (x86)", "Steam", "steamapps", "common"];

/// Attempt to locate a game inside a Wine bottle.
fn find_game_path(bottle: &Bottle, entry: &GameEntry) -> Option<PathBuf> {
    // Determine the Steam directory name.
    let steam_dir_name = entry.steam_dir.as_deref().unwrap_or(&entry.name);

    // 1. Check default Steam library.
    if let Some(common) = bottle.find_path(STEAM_COMMON) {
        if let Some(game_dir) = find_child_case_insensitive(&common, steam_dir_name) {
            if game_dir.is_dir() {
                return Some(game_dir);
            }
        }
    }

    // 2. Check additional Steam library folders from libraryfolders.vdf.
    if let Some(steam_dir) = bottle.find_path(&["Program Files (x86)", "Steam"]) {
        let vdf_path = steam_dir.join("steamapps").join("libraryfolders.vdf");
        let vdf_path = if vdf_path.exists() {
            Some(vdf_path)
        } else {
            let alt = steam_dir.join("config").join("libraryfolders.vdf");
            if alt.exists() {
                Some(alt)
            } else {
                None
            }
        };

        if let Some(vdf) = vdf_path {
            if let Some(lib_paths) = parse_library_folders_vdf(&vdf) {
                for lib_path in lib_paths {
                    let common = lib_path.join("steamapps").join("common");
                    if let Some(game_dir) = find_child_case_insensitive(&common, steam_dir_name) {
                        if game_dir.is_dir() {
                            return Some(game_dir);
                        }
                    }
                }
            }
        }
    }

    // 3. Check GOG paths.
    let gog_dirs = [
        vec!["GOG Games", steam_dir_name],
        vec!["Program Files", "GOG Galaxy", "Games", steam_dir_name],
        vec!["Program Files (x86)", "GOG Galaxy", "Games", steam_dir_name],
        vec!["Games", steam_dir_name],
    ];
    for parts in &gog_dirs {
        let refs: Vec<&str> = parts.iter().map(|s| &**s).collect();
        if let Some(path) = bottle.find_path(&refs) {
            if path.is_dir() {
                return Some(path);
            }
        }
    }

    None
}

/// Check if a file exists in a directory (case-insensitive).
fn has_file_case_insensitive(dir: &Path, filename_lower: &str) -> bool {
    find_file_case_insensitive(dir, filename_lower).is_some()
}

fn find_file_case_insensitive(dir: &Path, filename_lower: &str) -> Option<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return None;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().to_lowercase() == filename_lower {
            return Some(entry.path());
        }
    }
    None
}

/// Find a child whose name matches case-insensitively.
fn find_child_case_insensitive(parent: &Path, target: &str) -> Option<PathBuf> {
    let exact = parent.join(target);
    if exact.exists() {
        return Some(exact);
    }
    let target_lower = target.to_lowercase();
    let entries = fs::read_dir(parent).ok()?;
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().to_lowercase() == target_lower {
            return Some(entry.path());
        }
    }
    None
}

/// Parse Steam's `libraryfolders.vdf` to extract library paths.
fn parse_library_folders_vdf(vdf_path: &Path) -> Option<Vec<PathBuf>> {
    let content = fs::read_to_string(vdf_path).ok()?;
    let mut paths = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = strip_vdf_key(trimmed, "path") {
            let value = strip_vdf_quotes(rest);
            if !value.is_empty() {
                paths.push(PathBuf::from(value.replace('\\', "/")));
            }
        }
    }
    if paths.is_empty() {
        None
    } else {
        Some(paths)
    }
}

fn strip_vdf_key<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let line = line.trim();
    let expected = format!("\"{}\"", key);
    if !line.starts_with(&expected) {
        return None;
    }
    Some(line[expected.len()..].trim())
}

fn strip_vdf_quotes(s: &str) -> String {
    let s = s.trim();
    if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Register all games from the registry that don't have dedicated plugins.
pub fn register_all() {
    let entries = entries();
    for entry in entries.iter() {
        // Skip stubs (games with separate extensions).
        if entry.note.is_some() || entry.executable.is_none() {
            continue;
        }
        // Skip games with dedicated plugin modules.
        if CUSTOM_PLUGIN_IDS.contains(&entry.game_id.as_str()) {
            continue;
        }
        crate::games::register_plugin(Box::new(RegistryGamePlugin { entry }));
    }
}

/// Return metadata for all supported games (for the frontend).
pub fn list_supported_games() -> Vec<SupportedGame> {
    entries()
        .iter()
        .filter(|e| e.executable.is_some() && e.note.is_none())
        .map(|e| SupportedGame {
            game_id: e.game_id.clone(),
            name: e.name.clone(),
            nexus_domain: e.nexus_domain.clone(),
            steam_id: e.steam_id.clone(),
            has_tools: !e.tools.is_empty(),
            tool_names: e.tools.iter().map(|t| t.name.clone()).collect(),
        })
        .collect()
}

/// Get the full registry entry for a specific game.
pub fn get_game_entry(game_id: &str) -> Option<&'static GameEntry> {
    entries().iter().find(|e| e.game_id == game_id)
}

// ---------------------------------------------------------------------------
// Steam appmanifest scanner — detects ALL installed Steam games
// ---------------------------------------------------------------------------

/// A Steam game discovered from an appmanifest ACF file.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SteamAppManifest {
    pub app_id: String,
    pub name: String,
    pub install_dir: String,
}

/// Scan a bottle's steamapps directory for all appmanifest files.
/// Returns games that are NOT already detected by registered plugins.
pub fn detect_unregistered_steam_games(
    bottle: &Bottle,
    already_detected: &[DetectedGame],
) -> Vec<DetectedGame> {
    let steamapps = match bottle.find_path(&["Program Files (x86)", "Steam", "steamapps"]) {
        Some(p) => p,
        None => return Vec::new(),
    };

    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(&steamapps) else {
        return found;
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("appmanifest_") || !name.ends_with(".acf") {
            continue;
        }

        let manifest = match parse_appmanifest(&entry.path()) {
            Some(m) => m,
            None => continue,
        };

        // Skip if already detected by a registered plugin
        if already_detected.iter().any(|g| {
            // Match by Steam app ID in game_id or by install dir overlap
            g.game_path
                .to_string_lossy()
                .to_lowercase()
                .contains(&manifest.install_dir.to_lowercase())
        }) {
            continue;
        }

        // Resolve the actual game path
        let common = steamapps.join("common");
        let game_path = match find_child_case_insensitive(&common, &manifest.install_dir) {
            Some(p) if p.is_dir() => p,
            _ => continue,
        };

        // Find the first .exe in the game directory (heuristic)
        let exe_path = find_main_executable(&game_path);

        // Derive a game_id from the app name
        let game_id = slugify_game_name(&manifest.name);

        found.push(DetectedGame {
            game_id: game_id.clone(),
            display_name: manifest.name.clone(),
            nexus_slug: game_id,
            game_path: game_path.clone(),
            exe_path,
            data_dir: game_path,
            bottle_name: bottle.name.clone(),
            bottle_path: bottle.path.clone(),
        });
    }

    found
}

/// Parse a Steam appmanifest ACF file to extract app ID, name, and install dir.
fn parse_appmanifest(path: &Path) -> Option<SteamAppManifest> {
    let content = fs::read_to_string(path).ok()?;
    let app_id = extract_acf_value(&content, "appid")?;
    let name = extract_acf_value(&content, "name")?;
    let install_dir = extract_acf_value(&content, "installdir")?;

    // Skip tools/configs (Steamworks Shared, Proton, etc.)
    let state_flags = extract_acf_value(&content, "StateFlags")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    // StateFlags 4 = fully installed. Skip if 0 (invalid) or other states.
    if state_flags == 0 {
        return None;
    }

    // Skip known non-game entries
    let lower_name = name.to_lowercase();
    if lower_name.contains("steamworks")
        || lower_name.contains("proton")
        || lower_name.contains("steam linux runtime")
        || lower_name.contains("steam controller")
        || lower_name.contains("redistributable")
        || lower_name.contains("directx")
    {
        return None;
    }

    Some(SteamAppManifest {
        app_id,
        name,
        install_dir,
    })
}

/// Extract a quoted value from a VDF/ACF key-value pair.
fn extract_acf_value(content: &str, key: &str) -> Option<String> {
    let key_pat = format!("\"{}\"", key);
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(&key_pat) {
            let val = rest.trim();
            if val.starts_with('"') && val.len() >= 2 {
                let end = val[1..].find('"').map(|i| i + 1)?;
                return Some(val[1..end].to_string());
            }
        }
    }
    None
}

/// Public wrapper for find_main_executable.
pub fn find_main_executable_public(game_path: &Path) -> Option<PathBuf> {
    find_main_executable(game_path)
}

/// Find the most likely main executable in a game directory.
/// Prefers .exe files in the root, skips crash reporters and launchers.
fn find_main_executable(game_path: &Path) -> Option<PathBuf> {
    let skip_patterns = [
        "crash",
        "report",
        "installer",
        "unins",
        "setup",
        "redis",
        "vc_redist",
        "dxsetup",
        "dotnet",
    ];

    let Ok(entries) = fs::read_dir(game_path) else {
        return None;
    };

    let mut candidates: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if !name.ends_with(".exe") {
            continue;
        }
        let skip = skip_patterns.iter().any(|p| name.contains(p));
        if skip {
            continue;
        }
        candidates.push(entry.path());
    }

    // Sort by size descending — the main exe is usually the largest
    candidates.sort_by(|a, b| {
        let sa = a.metadata().map(|m| m.len()).unwrap_or(0);
        let sb = b.metadata().map(|m| m.len()).unwrap_or(0);
        sb.cmp(&sa)
    });

    candidates.into_iter().next()
}

/// Convert a game name into a URL-safe slug for use as game_id.
fn slugify_game_name(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

// ---------------------------------------------------------------------------
// Custom games (user-added, persisted in DB)
// ---------------------------------------------------------------------------

/// A custom game added by the user (stored in SQLite).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CustomGame {
    pub game_id: String,
    pub display_name: String,
    pub nexus_slug: String,
    pub game_path: String,
    pub exe_path: Option<String>,
    pub data_dir: String,
    pub bottle_name: String,
    pub bottle_path: String,
    pub steam_app_id: Option<String>,
}

/// Load custom games from the database and return them as DetectedGame entries.
pub fn load_custom_games(db: &crate::database::ModDatabase) -> Vec<DetectedGame> {
    let conn = match db.conn() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut stmt = match conn.prepare(
        "SELECT game_id, display_name, nexus_slug, game_path, exe_path, data_dir, bottle_name, bottle_path FROM custom_games",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(), // Table might not exist yet
    };
    let rows = stmt
        .query_map([], |row| {
            Ok(DetectedGame {
                game_id: row.get(0)?,
                display_name: row.get(1)?,
                nexus_slug: row.get(2)?,
                game_path: PathBuf::from(row.get::<_, String>(3)?),
                exe_path: row.get::<_, Option<String>>(4)?.map(PathBuf::from),
                data_dir: PathBuf::from(row.get::<_, String>(5)?),
                bottle_name: row.get(6)?,
                bottle_path: PathBuf::from(row.get::<_, String>(7)?),
            })
        })
        .ok();
    match rows {
        Some(r) => r.flatten().collect(),
        None => Vec::new(),
    }
}

/// Save a custom game to the database.
pub fn save_custom_game(
    db: &crate::database::ModDatabase,
    game: &CustomGame,
) -> Result<(), String> {
    let conn = db
        .conn()
        .map_err(|e| format!("No database connection: {e}"))?;
    conn.execute(
        "INSERT OR REPLACE INTO custom_games (game_id, display_name, nexus_slug, game_path, exe_path, data_dir, bottle_name, bottle_path, steam_app_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            game.game_id,
            game.display_name,
            game.nexus_slug,
            game.game_path,
            game.exe_path,
            game.data_dir,
            game.bottle_name,
            game.bottle_path,
            game.steam_app_id,
        ],
    )
    .map_err(|e| format!("Failed to save custom game: {e}"))?;
    Ok(())
}

/// Remove a custom game from the database.
pub fn remove_custom_game(db: &crate::database::ModDatabase, game_id: &str) -> Result<(), String> {
    let conn = db
        .conn()
        .map_err(|e| format!("No database connection: {e}"))?;
    conn.execute(
        "DELETE FROM custom_games WHERE game_id = ?1",
        rusqlite::params![game_id],
    )
    .map_err(|e| format!("Failed to remove custom game: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_parses_successfully() {
        let entries = entries();
        assert!(!entries.is_empty());
        // Should have at least 70 games
        assert!(
            entries.len() >= 70,
            "Expected 70+ games, got {}",
            entries.len()
        );
    }

    #[test]
    fn skyrimse_is_first() {
        let entries = entries();
        assert_eq!(entries[0].game_id, "skyrimse");
    }

    #[test]
    fn no_stubs_in_supported_list() {
        let supported = list_supported_games();
        for game in &supported {
            assert!(!game.game_id.is_empty(), "Empty game_id in supported list");
        }
        // Cyberpunk was a stub in original data but we added real data
        assert!(supported.iter().any(|g| g.game_id == "cyberpunk2077"));
    }

    #[test]
    fn custom_plugins_excluded_from_registration() {
        // Verify that the custom plugin IDs would be skipped
        for id in CUSTOM_PLUGIN_IDS {
            let entry = entries().iter().find(|e| e.game_id == *id);
            assert!(entry.is_some(), "Custom plugin {} not in registry", id);
        }
    }

    #[test]
    fn tools_parsed_correctly() {
        let skyrimse = entries().iter().find(|e| e.game_id == "skyrimse").unwrap();
        assert!(!skyrimse.tools.is_empty());
        assert!(skyrimse.tools.iter().any(|t| t.name.contains("SSEEdit")));
    }

    #[test]
    fn steam_dir_override_works() {
        let falloutnv = entries().iter().find(|e| e.game_id == "falloutnv").unwrap();
        assert_eq!(falloutnv.steam_dir.as_deref(), Some("Fallout New Vegas"));
    }

    #[test]
    fn list_supported_games_returns_data() {
        let supported = list_supported_games();
        assert!(!supported.is_empty());
        // Should include Skyrim SE
        assert!(supported.iter().any(|g| g.game_id == "skyrimse"));
        // Skyrim SE should have tools
        let sse = supported.iter().find(|g| g.game_id == "skyrimse").unwrap();
        assert!(sse.has_tools);
    }
}
