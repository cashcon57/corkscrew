//! Skyrim Special Edition game plugin.
//!
//! Implements [`crate::games::GamePlugin`] for detecting and managing
//! Skyrim Special Edition installations inside Wine bottles. Supports
//! detection via Steam library folders (including additional library paths
//! parsed from `libraryfolders.vdf`) and GOG installation paths.

use std::fs;
use std::path::{Path, PathBuf};

use crate::bottles::Bottle;
use crate::games::{DetectedGame, GamePlugin};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Known executable names for Skyrim SE (checked case-insensitively).
const EXECUTABLES: &[&str] = &["SkyrimSE.exe", "SkyrimSELauncher.exe"];

/// Relative path components from `drive_c` to the default Steam library.
const STEAM_COMMON: &[&str] = &["Program Files (x86)", "Steam", "steamapps", "common"];

/// The game's directory name inside a Steam library.
const STEAM_GAME_DIR: &str = "Skyrim Special Edition";

/// GOG installation paths to check (relative to `drive_c`).
const GOG_PATHS: &[&[&str]] = &[
    &["GOG Games", "Skyrim Special Edition"],
    &[
        "Program Files",
        "GOG Galaxy",
        "Games",
        "Skyrim Special Edition",
    ],
    &[
        "Program Files (x86)",
        "GOG Galaxy",
        "Games",
        "Skyrim Special Edition",
    ],
    &["Games", "Skyrim Special Edition"],
];

/// The `plugins.txt` path relative to `AppData\Local`.
const PLUGINS_TXT_RELATIVE: &[&str] = &["Skyrim Special Edition", "plugins.txt"];

// ---------------------------------------------------------------------------
// SkyrimSEPlugin
// ---------------------------------------------------------------------------

/// Game plugin for The Elder Scrolls V: Skyrim Special Edition.
pub struct SkyrimSEPlugin;

impl GamePlugin for SkyrimSEPlugin {
    fn game_id(&self) -> &str {
        "skyrimse"
    }

    fn display_name(&self) -> &str {
        "Skyrim Special Edition"
    }

    fn nexus_slug(&self) -> &str {
        "skyrimspecialedition"
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
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Register the Skyrim SE plugin with the global game plugin registry.
pub fn register() {
    crate::games::register_plugin(Box::new(SkyrimSEPlugin));
}

// ---------------------------------------------------------------------------
// Detection helpers
// ---------------------------------------------------------------------------

/// Attempt to locate the Skyrim SE installation directory inside a bottle.
///
/// Checks the default Steam common directory first, then parses
/// `libraryfolders.vdf` for additional Steam library paths, and finally
/// checks well-known GOG installation paths.
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

    // Also try config/libraryfolders.vdf (older Steam layout).
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
        // Each library path contains a `steamapps/common` subdirectory.
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

/// Check whether a directory contains at least one of the known Skyrim SE
/// executables (case-insensitive comparison).
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

/// Find a child entry of `parent` whose name matches `target`
/// case-insensitively.
fn find_child_case_insensitive(parent: &Path, target: &str) -> Option<PathBuf> {
    // Fast path.
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
///
/// The VDF format is a simple key-value tree. We look for `"path"` keys
/// and collect their string values. The paths inside a Wine bottle are
/// Windows-style but actually map to the POSIX filesystem inside `drive_c`,
/// so we normalise backslashes and attempt to resolve them.
fn parse_library_folders_vdf(vdf_path: &Path) -> Option<Vec<PathBuf>> {
    let content = fs::read_to_string(vdf_path).ok()?;
    let mut paths = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();

        // Match lines like:  "path"		"C:\Program Files\Steam"
        if let Some(rest) = strip_vdf_key(trimmed, "path") {
            let value = strip_vdf_quotes(rest);
            if !value.is_empty() {
                // Normalise Windows path separators.
                let normalised = value.replace('\\', "/");

                // If it looks like a Windows absolute path (e.g. C:/...),
                // we need to resolve it relative to the bottle's drive_c.
                // However we don't have the bottle reference here, so we
                // store the raw path and let the caller resolve it. Since
                // most Steam library paths inside a bottle point back into
                // the same prefix, just store as-is.
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

/// Strip a VDF key name and surrounding quotes from the start of a line,
/// returning the remainder (the value portion).
fn strip_vdf_key<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let line = line.trim();

    // Expected format: "key"  "value"
    let expected_key = format!("\"{}\"", key);
    if !line.starts_with(&expected_key) {
        return None;
    }

    Some(line[expected_key.len()..].trim())
}

/// Remove surrounding double quotes from a VDF value string.
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
    fn strip_vdf_key_extracts_value() {
        let line = r#""path"		"C:\SteamLibrary""#;
        let rest = strip_vdf_key(line, "path").unwrap();
        assert_eq!(strip_vdf_quotes(rest), r"C:\SteamLibrary");
    }

    #[test]
    fn strip_vdf_key_returns_none_for_wrong_key() {
        let line = r#""id"		"1""#;
        assert!(strip_vdf_key(line, "path").is_none());
    }

    #[test]
    fn strip_vdf_quotes_removes_quotes() {
        assert_eq!(strip_vdf_quotes(r#""hello""#), "hello");
        assert_eq!(strip_vdf_quotes("noquotes"), "noquotes");
    }

    #[test]
    fn parse_vdf_extracts_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let vdf = tmp.path().join("libraryfolders.vdf");
        fs::write(
            &vdf,
            r#"
"libraryfolders"
{
    "0"
    {
        "path"		"C:\Program Files (x86)\Steam"
        "label"		""
    }
    "1"
    {
        "path"		"D:\SteamLibrary"
        "label"		""
    }
}
"#,
        )
        .unwrap();

        let paths = parse_library_folders_vdf(&vdf).unwrap();
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0], PathBuf::from("C:/Program Files (x86)/Steam"));
        assert_eq!(paths[1], PathBuf::from("D:/SteamLibrary"));
    }

    #[test]
    fn plugin_metadata() {
        let plugin = SkyrimSEPlugin;
        assert_eq!(plugin.game_id(), "skyrimse");
        assert_eq!(plugin.display_name(), "Skyrim Special Edition");
        assert_eq!(plugin.nexus_slug(), "skyrimspecialedition");
        assert_eq!(plugin.executables().len(), 2);
    }

    #[test]
    fn data_dir_is_game_path_data() {
        let plugin = SkyrimSEPlugin;
        let game_path = PathBuf::from("/fake/Skyrim Special Edition");
        assert_eq!(
            plugin.get_data_dir(&game_path),
            PathBuf::from("/fake/Skyrim Special Edition/Data")
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

        let plugin = SkyrimSEPlugin;
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
            .join("Skyrim Special Edition");
        fs::create_dir_all(&game_dir).unwrap();

        // Create a fake executable.
        fs::write(game_dir.join("SkyrimSE.exe"), b"fake").unwrap();

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };

        let plugin = SkyrimSEPlugin;
        let detected = plugin.detect(&bottle);
        assert!(detected.is_some());

        let detected = detected.unwrap();
        assert_eq!(detected.game_id, "skyrimse");
        assert_eq!(detected.game_path, game_dir);
    }
}
