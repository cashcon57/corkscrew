//! Fallout 4 game plugin.
//!
//! Implements [`crate::games::GamePlugin`] for detecting and managing
//! Fallout 4 installations inside Wine bottles. Supports detection via
//! Steam library folders (including additional library paths parsed from
//! `libraryfolders.vdf`) and GOG installation paths.

use std::fs;
use std::path::{Path, PathBuf};

use crate::bottles::Bottle;
use crate::games::{DetectedGame, GamePlugin};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Known executable names for Fallout 4 (checked case-insensitively).
const EXECUTABLES: &[&str] = &["Fallout4.exe", "Fallout4Launcher.exe"];

/// Relative path components from `drive_c` to the default Steam library.
const STEAM_COMMON: &[&str] = &["Program Files (x86)", "Steam", "steamapps", "common"];

/// The game's directory name inside a Steam library.
const STEAM_GAME_DIR: &str = "Fallout 4";

/// GOG installation paths to check (relative to `drive_c`).
const GOG_PATHS: &[&[&str]] = &[
    &["GOG Games", "Fallout 4"],
    &["GOG Games", "Fallout 4 GOTY"],
    &["Program Files", "GOG Galaxy", "Games", "Fallout 4"],
    &["Program Files (x86)", "GOG Galaxy", "Games", "Fallout 4"],
    &["Games", "Fallout 4"],
];

/// The `plugins.txt` path relative to `AppData\Local`.
const PLUGINS_TXT_RELATIVE: &[&str] = &["Fallout4", "plugins.txt"];

// ---------------------------------------------------------------------------
// Fallout4Plugin
// ---------------------------------------------------------------------------

/// Game plugin for Fallout 4.
pub struct Fallout4Plugin;

impl GamePlugin for Fallout4Plugin {
    fn game_id(&self) -> &str {
        "fallout4"
    }

    fn display_name(&self) -> &str {
        "Fallout 4"
    }

    fn nexus_slug(&self) -> &str {
        "fallout4"
    }

    fn executables(&self) -> &[&str] {
        EXECUTABLES
    }

    fn detect(&self, bottle: &Bottle) -> Option<DetectedGame> {
        let game_path = find_game_path(bottle)?;

        // Verify at least one known executable exists (case-insensitive).
        if !has_executable(&game_path) {
            return None;
        }

        let data_dir = self.get_data_dir(&game_path);

        Some(DetectedGame {
            game_id: self.game_id().to_string(),
            display_name: self.display_name().to_string(),
            nexus_slug: self.nexus_slug().to_string(),
            game_path,
            data_dir,
            bottle_name: bottle.name.clone(),
            bottle_path: bottle.path.clone(),
        })
    }

    fn get_data_dir(&self, game_path: &Path) -> PathBuf {
        game_path.join("Data")
    }

    fn get_plugins_file(&self, _game_path: &Path, bottle: &Bottle) -> Option<PathBuf> {
        let local = bottle.appdata_local();
        let mut path = local;
        for component in PLUGINS_TXT_RELATIVE {
            path = path.join(component);
        }
        Some(path)
    }

    fn get_saves_dir(&self, _game_path: &Path, bottle: &Bottle) -> Option<PathBuf> {
        let docs = bottle.documents_dir();
        let saves = docs.join("My Games").join("Fallout4").join("Saves");
        Some(saves)
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Register the Fallout 4 plugin with the global game plugin registry.
pub fn register() {
    crate::games::register_plugin(Box::new(Fallout4Plugin));
}

// ---------------------------------------------------------------------------
// Detection helpers
// ---------------------------------------------------------------------------

/// Attempt to locate the Fallout 4 installation directory inside a bottle.
fn find_game_path(bottle: &Bottle) -> Option<PathBuf> {
    // 1. Default Steam library location.
    if let Some(path) = check_steam_default(bottle) {
        return Some(path);
    }

    // 2. Additional Steam library folders from libraryfolders.vdf.
    if let Some(path) = check_steam_library_folders(bottle) {
        return Some(path);
    }

    // 3. GOG installation paths.
    if let Some(path) = check_gog_paths(bottle) {
        return Some(path);
    }

    None
}

/// Check the default Steam common directory.
fn check_steam_default(bottle: &Bottle) -> Option<PathBuf> {
    let path = bottle.find_path(STEAM_COMMON)?;
    let game_dir = find_child_case_insensitive(&path, STEAM_GAME_DIR)?;
    if game_dir.is_dir() {
        Some(game_dir)
    } else {
        None
    }
}

/// Parse `libraryfolders.vdf` and check each library for the game.
fn check_steam_library_folders(bottle: &Bottle) -> Option<PathBuf> {
    let steam_dir = bottle.find_path(&["Program Files (x86)", "Steam"])?;
    let vdf_path = steam_dir.join("steamapps").join("libraryfolders.vdf");

    let vdf_path = if vdf_path.exists() {
        vdf_path
    } else {
        let alt = steam_dir.join("config").join("libraryfolders.vdf");
        if alt.exists() {
            alt
        } else {
            return None;
        }
    };

    let library_paths = parse_library_folders_vdf(&vdf_path)?;

    for lib_path in library_paths {
        let common = lib_path.join("steamapps").join("common");
        if let Some(game_dir) = find_child_case_insensitive(&common, STEAM_GAME_DIR) {
            if game_dir.is_dir() {
                return Some(game_dir);
            }
        }
    }

    None
}

/// Check well-known GOG installation directories.
fn check_gog_paths(bottle: &Bottle) -> Option<PathBuf> {
    for parts in GOG_PATHS {
        if let Some(path) = bottle.find_path(parts) {
            if path.is_dir() {
                return Some(path);
            }
        }
    }
    None
}

/// Check whether a directory contains at least one known executable.
fn has_executable(game_path: &Path) -> bool {
    let Ok(entries) = fs::read_dir(game_path) else {
        return false;
    };

    let exe_names_lower: Vec<String> = EXECUTABLES.iter().map(|e| e.to_lowercase()).collect();

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if exe_names_lower.iter().any(|e| e == &name) {
            return true;
        }
    }

    false
}

/// Find a child entry whose name matches `target` case-insensitively.
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

/// Parse Steam's `libraryfolders.vdf` to extract additional library paths.
fn parse_library_folders_vdf(vdf_path: &Path) -> Option<Vec<PathBuf>> {
    let content = fs::read_to_string(vdf_path).ok()?;
    let mut paths = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = strip_vdf_key(trimmed, "path") {
            let value = strip_vdf_quotes(rest);
            if !value.is_empty() {
                let normalised = value.replace('\\', "/");
                paths.push(PathBuf::from(normalised));
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
    let expected_key = format!("\"{}\"", key);
    if !line.starts_with(&expected_key) {
        return None;
    }
    Some(line[expected_key.len()..].trim())
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_metadata() {
        let plugin = Fallout4Plugin;
        assert_eq!(plugin.game_id(), "fallout4");
        assert_eq!(plugin.display_name(), "Fallout 4");
        assert_eq!(plugin.nexus_slug(), "fallout4");
        assert_eq!(plugin.executables().len(), 2);
    }

    #[test]
    fn data_dir_is_game_path_data() {
        let plugin = Fallout4Plugin;
        let game_path = PathBuf::from("/fake/Fallout 4");
        assert_eq!(
            plugin.get_data_dir(&game_path),
            PathBuf::from("/fake/Fallout 4/Data")
        );
    }

    #[test]
    fn detect_returns_none_for_empty_bottle() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        fs::create_dir_all(bottle_path.join("drive_c")).unwrap();

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };

        let plugin = Fallout4Plugin;
        assert!(plugin.detect(&bottle).is_none());
    }

    #[test]
    fn detect_finds_game_in_steam_default() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = bottle_path
            .join("drive_c")
            .join("Program Files (x86)")
            .join("Steam")
            .join("steamapps")
            .join("common")
            .join("Fallout 4");
        fs::create_dir_all(&game_dir).unwrap();

        // Create a fake executable.
        fs::write(game_dir.join("Fallout4.exe"), b"fake").unwrap();

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };

        let plugin = Fallout4Plugin;
        let detected = plugin.detect(&bottle);
        assert!(detected.is_some());

        let detected = detected.unwrap();
        assert_eq!(detected.game_id, "fallout4");
        assert_eq!(detected.game_path, game_dir);
    }

    #[test]
    fn detect_finds_game_in_gog_path() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = bottle_path
            .join("drive_c")
            .join("GOG Games")
            .join("Fallout 4");
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join("Fallout4.exe"), b"fake").unwrap();

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };

        let plugin = Fallout4Plugin;
        let detected = plugin.detect(&bottle);
        assert!(detected.is_some());
        assert_eq!(detected.unwrap().game_id, "fallout4");
    }
}
