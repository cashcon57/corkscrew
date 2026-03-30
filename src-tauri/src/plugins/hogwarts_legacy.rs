//! Hogwarts Legacy game plugin.
//!
//! Implements [`crate::games::GamePlugin`] for detecting and managing
//! Hogwarts Legacy installations inside Wine bottles. Supports detection
//! via Steam library folders (including additional library paths parsed
//! from `libraryfolders.vdf`) and Epic Games Store installation paths.
//!
//! Hogwarts Legacy is an Unreal Engine 5 game with the internal project
//! name "Phoenix". The directory structure places the real executable at
//! `Phoenix/Binaries/Win64/HogwartsLegacy.exe` rather than the game root.
//! Pak mods deploy to `Phoenix/Content/Paks/~mods/` (the `~` prefix
//! ensures mods load after base game files in alphanumeric order).

use std::fs;
use std::path::{Path, PathBuf};

use crate::bottles::Bottle;
use crate::games::{DetectedGame, GamePlugin};
use crate::vortex_types::VortexModType;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Known executable name for Hogwarts Legacy (checked case-insensitively).
/// Note: There is a root launcher AND a real exe at Phoenix/Binaries/Win64/.
/// We only list one name since both are `HogwartsLegacy.exe`.
const EXECUTABLES: &[&str] = &["HogwartsLegacy.exe"];

/// Relative path components from `drive_c` to the default Steam library.
const STEAM_COMMON: &[&str] = &["Program Files (x86)", "Steam", "steamapps", "common"];

/// The game's directory name inside a Steam library.
const STEAM_GAME_DIR: &str = "Hogwarts Legacy";

/// Epic Games Store installation paths to check (relative to `drive_c`).
const EPIC_PATHS: &[&[&str]] = &[
    &["Program Files", "Epic Games", "Hogwarts Legacy"],
    &["Program Files (x86)", "Epic Games", "Hogwarts Legacy"],
];

/// Subdirectory containing the real game executable.
const PHOENIX_BIN_DIR: &[&str] = &["Phoenix", "Binaries", "Win64"];

// ---------------------------------------------------------------------------
// HogwartsLegacyPlugin
// ---------------------------------------------------------------------------

/// Game plugin for Hogwarts Legacy (Unreal Engine 5, project "Phoenix").
pub struct HogwartsLegacyPlugin;

impl GamePlugin for HogwartsLegacyPlugin {
    fn game_id(&self) -> &str {
        "hogwartslegacy"
    }

    fn display_name(&self) -> &str {
        "Hogwarts Legacy"
    }

    fn nexus_slug(&self) -> &str {
        "hogwartslegacy"
    }

    fn executables(&self) -> &[&str] {
        EXECUTABLES
    }

    fn detect(&self, bottle: &Bottle) -> Option<DetectedGame> {
        let game_path = find_game_path(bottle)?;

        // Verify the real executable exists in Phoenix/Binaries/Win64/
        // (the root launcher alone is not sufficient).
        if !has_real_executable(&game_path) {
            return None;
        }

        let data_dir = self.get_data_dir(&game_path);
        let exe_path = find_executable(&game_path);

        Some(DetectedGame {
            game_id: self.game_id().to_string(),
            display_name: self.display_name().to_string(),
            nexus_slug: self.nexus_slug().to_string(),
            game_path,
            exe_path,
            data_dir,
            bottle_name: bottle.name.clone(),
            bottle_path: bottle.path.clone(),
        })
    }

    fn get_data_dir(&self, game_path: &Path) -> PathBuf {
        game_path
            .join("Phoenix")
            .join("Content")
            .join("Paks")
            .join("~mods")
    }

    fn get_plugins_file(&self, _game_path: &Path, _bottle: &Bottle) -> Option<PathBuf> {
        None // Not a Bethesda game — no plugin load order.
    }

    fn get_saves_dir(&self, _game_path: &Path, bottle: &Bottle) -> Option<PathBuf> {
        let local = bottle.appdata_local();
        Some(
            local
                .join("Hogwarts Legacy")
                .join("Saved")
                .join("SaveGames"),
        )
    }

    fn vortex_mod_types(&self) -> Vec<VortexModType> {
        vec![
            // Standard pak mods — handled by get_data_dir() as the default,
            // but also registered as a type for explicit collection metadata.
            VortexModType {
                id: "hogwarts-PAK-modtype".into(),
                priority: 25,
                target_path: "Phoenix/Content/Paks/~mods".into(),
            },
            // UE4SS framework, ReShade, and other exe-directory mods.
            VortexModType {
                id: "hl-engine-root".into(),
                priority: 30,
                target_path: "Phoenix/Binaries/Win64".into(),
            },
            // UE4SS Lua script mods (each mod is a subfolder with Scripts/main.lua).
            VortexModType {
                id: "hl-lua-mod".into(),
                priority: 35,
                target_path: "Phoenix/Binaries/Win64/Mods".into(),
            },
            // UE4SS Blueprint/LogicMods (loaded by Apparate/UE4SS loader).
            VortexModType {
                id: "hl-logic-mod".into(),
                priority: 40,
                target_path: "Phoenix/Content/Paks/LogicMods".into(),
            },
            // Movie replacement mods (.bk2 files).
            VortexModType {
                id: "hogwarts-modtype-movies".into(),
                priority: 95,
                target_path: "Phoenix/Content".into(),
            },
        ]
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Register the Hogwarts Legacy plugin with the global game plugin registry.
pub fn register() {
    crate::games::register_plugin(Box::new(HogwartsLegacyPlugin));
}

// ---------------------------------------------------------------------------
// Detection helpers
// ---------------------------------------------------------------------------

/// Attempt to locate the Hogwarts Legacy installation directory inside a bottle.
///
/// Checks the default Steam common directory first, then parses
/// `libraryfolders.vdf` for additional Steam library paths, and finally
/// checks Epic Games Store installation paths.
fn find_game_path(bottle: &Bottle) -> Option<PathBuf> {
    // 1. Default Steam library location.
    if let Some(path) = check_steam_default(bottle) {
        return Some(path);
    }

    // 2. Additional Steam library folders from libraryfolders.vdf.
    if let Some(path) = check_steam_library_folders(bottle) {
        return Some(path);
    }

    // 3. Epic Games Store installation paths.
    if let Some(path) = check_epic_paths(bottle) {
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
        let common = lib_path.join("steamapps").join("common");
        if let Some(game_dir) = find_child_case_insensitive(&common, STEAM_GAME_DIR) {
            if game_dir.is_dir() {
                return Some(game_dir);
            }
        }
    }

    None
}

/// Check well-known Epic Games Store installation directories.
fn check_epic_paths(bottle: &Bottle) -> Option<PathBuf> {
    for parts in EPIC_PATHS {
        if let Some(path) = bottle.find_path(parts) {
            if path.is_dir() {
                return Some(path);
            }
        }
    }
    None
}

/// Check whether the *real* executable exists at `Phoenix/Binaries/Win64/`.
/// The root launcher alone is not sufficient to confirm a valid installation.
fn has_real_executable(game_path: &Path) -> bool {
    let bin_dir = resolve_phoenix_bin_dir(game_path);
    find_file_case_insensitive(&bin_dir, "hogwartslegacy.exe").is_some()
}

/// Find the main game executable, preferring the real exe in Phoenix/Binaries/Win64/
/// over the root launcher.
fn find_executable(game_path: &Path) -> Option<PathBuf> {
    // Prefer the real exe in Phoenix/Binaries/Win64/.
    let bin_dir = resolve_phoenix_bin_dir(game_path);
    if let Some(exe) = find_file_case_insensitive(&bin_dir, "hogwartslegacy.exe") {
        return Some(exe);
    }

    // Fall back to root launcher.
    find_file_case_insensitive(game_path, "hogwartslegacy.exe")
}

/// Build the path to `Phoenix/Binaries/Win64` with case-insensitive traversal.
fn resolve_phoenix_bin_dir(game_path: &Path) -> PathBuf {
    let mut dir = game_path.to_path_buf();
    for component in PHOENIX_BIN_DIR {
        if let Some(child) = find_child_case_insensitive(&dir, component) {
            dir = child;
        } else {
            // If case-insensitive lookup fails, use the exact component
            // (will likely fail downstream, but that's fine — detection
            // returns None).
            dir = dir.join(component);
        }
    }
    dir
}

/// Find a file inside `dir` whose name matches `target` case-insensitively.
fn find_file_case_insensitive(dir: &Path, target: &str) -> Option<PathBuf> {
    let exact = dir.join(target);
    if exact.exists() {
        return Some(exact);
    }

    let target_lower = target.to_lowercase();
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().to_lowercase() == target_lower {
            return Some(entry.path());
        }
    }
    None
}

/// Find a child entry of `parent` whose name matches `target`
/// case-insensitively.
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
        let plugin = HogwartsLegacyPlugin;
        assert_eq!(plugin.game_id(), "hogwartslegacy");
        assert_eq!(plugin.display_name(), "Hogwarts Legacy");
        assert_eq!(plugin.nexus_slug(), "hogwartslegacy");
        assert_eq!(plugin.executables(), &["HogwartsLegacy.exe"]);
    }

    #[test]
    fn data_dir_is_paks_tilde_mods() {
        let plugin = HogwartsLegacyPlugin;
        let game_path = PathBuf::from("/fake/Hogwarts Legacy");
        assert_eq!(
            plugin.get_data_dir(&game_path),
            PathBuf::from("/fake/Hogwarts Legacy/Phoenix/Content/Paks/~mods")
        );
    }

    #[test]
    fn no_plugins_file() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = Bottle {
            name: "Test".into(),
            path: tmp.path().to_path_buf(),
            source: "Test".into(),
        };
        let plugin = HogwartsLegacyPlugin;
        assert!(plugin
            .get_plugins_file(Path::new("/fake"), &bottle)
            .is_none());
    }

    #[test]
    fn saves_dir_in_localappdata() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        fs::create_dir_all(bottle_path.join("drive_c")).unwrap();

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };

        let plugin = HogwartsLegacyPlugin;
        let saves = plugin.get_saves_dir(Path::new("/fake"), &bottle);
        assert!(saves.is_some());
        let saves = saves.unwrap();
        assert!(saves.to_string_lossy().contains("Hogwarts Legacy"));
        assert!(saves.to_string_lossy().contains("SaveGames"));
    }

    #[test]
    fn vortex_mod_types_defined() {
        let plugin = HogwartsLegacyPlugin;
        let types = plugin.vortex_mod_types();
        assert_eq!(types.len(), 5);

        let pak = types
            .iter()
            .find(|t| t.id == "hogwarts-PAK-modtype")
            .unwrap();
        assert_eq!(pak.target_path, "Phoenix/Content/Paks/~mods");

        let engine_root = types.iter().find(|t| t.id == "hl-engine-root").unwrap();
        assert_eq!(engine_root.target_path, "Phoenix/Binaries/Win64");

        let lua_mod = types.iter().find(|t| t.id == "hl-lua-mod").unwrap();
        assert_eq!(lua_mod.target_path, "Phoenix/Binaries/Win64/Mods");

        let logic_mod = types.iter().find(|t| t.id == "hl-logic-mod").unwrap();
        assert_eq!(logic_mod.target_path, "Phoenix/Content/Paks/LogicMods");

        let movies = types
            .iter()
            .find(|t| t.id == "hogwarts-modtype-movies")
            .unwrap();
        assert_eq!(movies.target_path, "Phoenix/Content");
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

        let plugin = HogwartsLegacyPlugin;
        assert!(plugin.detect(&bottle).is_none());
    }

    /// Helper to create the full Hogwarts Legacy directory structure in a
    /// temporary bottle, including the real executable in Phoenix/Binaries/Win64/.
    fn create_hl_install(bottle_path: &Path, game_root: &Path) {
        let bin_dir = game_root.join("Phoenix").join("Binaries").join("Win64");
        fs::create_dir_all(&bin_dir).unwrap();
        fs::write(bin_dir.join("HogwartsLegacy.exe"), b"fake").unwrap();
        // Ensure drive_c exists for the bottle.
        fs::create_dir_all(bottle_path.join("drive_c")).unwrap();
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
            .join("Hogwarts Legacy");

        create_hl_install(&bottle_path, &game_dir);

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };

        let plugin = HogwartsLegacyPlugin;
        let detected = plugin.detect(&bottle);
        assert!(detected.is_some());

        let detected = detected.unwrap();
        assert_eq!(detected.game_id, "hogwartslegacy");
        assert_eq!(detected.game_path, game_dir);
    }

    #[test]
    fn detect_finds_game_in_epic_path() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = bottle_path
            .join("drive_c")
            .join("Program Files")
            .join("Epic Games")
            .join("Hogwarts Legacy");

        create_hl_install(&bottle_path, &game_dir);

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };

        let plugin = HogwartsLegacyPlugin;
        let detected = plugin.detect(&bottle);
        assert!(detected.is_some());
        assert_eq!(detected.unwrap().game_id, "hogwartslegacy");
    }

    #[test]
    fn detect_requires_real_exe_not_root_launcher() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = bottle_path
            .join("drive_c")
            .join("Program Files (x86)")
            .join("Steam")
            .join("steamapps")
            .join("common")
            .join("Hogwarts Legacy");
        fs::create_dir_all(&game_dir).unwrap();
        fs::create_dir_all(bottle_path.join("drive_c")).unwrap();

        // Only create the root launcher, NOT the real exe in Phoenix/Binaries/Win64/.
        fs::write(game_dir.join("HogwartsLegacy.exe"), b"launcher").unwrap();

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };

        let plugin = HogwartsLegacyPlugin;
        // Should NOT detect — root launcher alone is insufficient.
        assert!(plugin.detect(&bottle).is_none());
    }

    #[test]
    fn exe_path_prefers_real_exe() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = bottle_path
            .join("drive_c")
            .join("Program Files (x86)")
            .join("Steam")
            .join("steamapps")
            .join("common")
            .join("Hogwarts Legacy");

        create_hl_install(&bottle_path, &game_dir);
        // Also create the root launcher.
        fs::write(game_dir.join("HogwartsLegacy.exe"), b"launcher").unwrap();

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };

        let plugin = HogwartsLegacyPlugin;
        let detected = plugin.detect(&bottle).unwrap();

        // exe_path should point to the real exe in Phoenix/Binaries/Win64/.
        let exe_path = detected.exe_path.unwrap();
        assert!(
            exe_path.to_string_lossy().contains("Phoenix"),
            "exe_path should be in Phoenix/Binaries/Win64, got: {}",
            exe_path.display()
        );
    }

    #[test]
    fn strip_vdf_key_extracts_value() {
        let line = r#""path"		"C:\SteamLibrary""#;
        let rest = strip_vdf_key(line, "path").unwrap();
        assert_eq!(strip_vdf_quotes(rest), r"C:\SteamLibrary");
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
}
