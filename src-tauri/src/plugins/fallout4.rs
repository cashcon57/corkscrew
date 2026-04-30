//! Fallout 4 game plugin.
//!
//! Implements [`crate::games::GamePlugin`] for detecting and managing
//! Fallout 4 installations inside Wine bottles. Supports detection via
//! Steam library folders (including additional library paths parsed from
//! `libraryfolders.vdf`) and GOG installation paths.

use std::fs;
use std::path::{Path, PathBuf};

use crate::bottles::Bottle;
use crate::games::{DetectedGame, GamePlugin, LoadOrderKind};
use crate::runtime::{GameRuntime, WineContext};

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

/// Additional non-Steam, non-GOG install paths. Each is anchored by the real
/// executable check to prevent false-positives from empty directories.
const NON_STEAM_PATHS: &[&[&str]] = &[
    &["Program Files (x86)", "Fallout 4"],
    &["Program Files", "Fallout 4"],
    &["Games", "Fallout 4"],
    // Top-level drag-drop convention.
    &["Fallout 4"],
];

/// Xbox Game Pass install path.
const XBOX_GAMES_PATH: &[&str] = &["XboxGames", "Fallout 4", "Content"];

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

    fn detect_wine(&self, bottle: &Bottle) -> Option<DetectedGame> {
        let game_path = find_game_path(bottle)?;

        // Verify at least one known executable exists (case-insensitive).
        if !has_executable(&game_path) {
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
            runtime: GameRuntime::Wine(WineContext {
                bottle_name: bottle.name.clone(),
                bottle_path: bottle.path.clone(),
                source: bottle.source.clone(),
            }),
            steam_app_id: None,
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

    fn critical_files(&self) -> Vec<&str> {
        vec![
            "fallout4.esm",
            "dlcrobot.esm",
            "dlcworkshop01.esm",
            "dlcworkshop02.esm",
            "dlcworkshop03.esm",
            "dlccoast.esm",
            "dlcnukaworld.esm",
        ]
    }

    fn protected_root_extensions(&self) -> Vec<&str> {
        vec![".esm", ".bsa", ".ba2"]
    }

    fn save_file_patterns(&self) -> Vec<&str> {
        vec![".fos", ".f4se", "saves/"]
    }

    fn load_order_kind(&self, _game_path: &Path) -> LoadOrderKind {
        LoadOrderKind::Plugins
    }

    fn categorize_mod_file(&self, rel_path: &str) -> Option<String> {
        let lower = rel_path.to_lowercase();

        if lower.ends_with(".esp") || lower.ends_with(".esm") || lower.ends_with(".esl") {
            return Some("plugin".into());
        }
        if lower.ends_with(".bsa") || lower.ends_with(".ba2") {
            return Some("bsa".into());
        }
        if lower.contains("meshes/") || lower.ends_with(".nif") {
            return Some("mesh".into());
        }
        if lower.contains("textures/") || lower.ends_with(".dds") {
            return Some("texture".into());
        }
        if lower.contains("scripts/") || lower.ends_with(".pex") || lower.ends_with(".psc") {
            return Some("script".into());
        }
        if lower.contains("sound/")
            || lower.contains("music/")
            || lower.ends_with(".wav")
            || lower.ends_with(".xwm")
            || lower.ends_with(".fuz")
        {
            return Some("sound".into());
        }
        if lower.contains("interface/") || lower.ends_with(".swf") {
            return Some("interface".into());
        }
        if lower.contains("f4se/") || lower.ends_with(".dll") {
            return Some("skse".into());
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Register the Fallout 4 plugin with the global game plugin registry.
pub fn register() {
    crate::games::register_plugin(std::sync::Arc::new(Fallout4Plugin));
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

    // 4. Generic non-Steam paths (manual installs, drag-drop).
    if let Some(path) = check_non_steam_paths(bottle) {
        return Some(path);
    }

    // 5. Xbox Game Pass install.
    if let Some(path) = check_xbox_games_path(bottle) {
        return Some(path);
    }

    // 6. Host Documents folder (via CrossOver symlink).
    check_user_documents_paths(bottle)
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

/// Check all Steam library folders (including non-C: drives) for the game.
///
/// Delegates to [`crate::game_registry::collect_steam_library_paths`] which
/// resolves Windows-style VDF path strings against every `drive_X` directory
/// present in the bottle, rather than assuming paths are under `drive_c`.
fn check_steam_library_folders(bottle: &Bottle) -> Option<PathBuf> {
    let library_paths = crate::game_registry::collect_steam_library_paths(bottle);
    for steamapps in library_paths {
        let common = steamapps.join("common");
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
            if path.is_dir() && has_executable(&path) {
                return Some(path);
            }
        }
    }
    None
}

/// Check generic non-Steam / non-GOG install paths, anchored by the real
/// executable to prevent false-positives from empty directories.
fn check_non_steam_paths(bottle: &Bottle) -> Option<PathBuf> {
    for parts in NON_STEAM_PATHS {
        if let Some(path) = bottle.find_path(parts) {
            if path.is_dir() && has_executable(&path) {
                return Some(path);
            }
        }
    }
    None
}

/// Check Xbox Game Pass install location.
fn check_xbox_games_path(bottle: &Bottle) -> Option<PathBuf> {
    if let Some(path) = bottle.find_path(XBOX_GAMES_PATH) {
        if path.is_dir() && has_executable(&path) {
            return Some(path);
        }
    }
    None
}

/// Check whether a directory contains at least one known executable.
fn has_executable(game_path: &Path) -> bool {
    find_executable(game_path).is_some()
}

/// Probe `<bottle>/drive_c/users/<user>/Documents/Games/Fallout 4/` and
/// `<bottle>/drive_c/users/<user>/Documents/Fallout 4/` for game installs.
///
/// CrossOver typically symlinks the bottle's `Documents` directory to the
/// host `~/Documents`, so games kept at `~/Documents/Games/<name>/` on the
/// macOS host are reachable through the bottle filesystem.
fn check_user_documents_paths(bottle: &Bottle) -> Option<PathBuf> {
    let users_dir = bottle.users_dir();
    let Ok(entries) = fs::read_dir(&users_dir) else {
        return None;
    };
    for user_entry in entries.flatten() {
        let user_dir = user_entry.path();
        if !user_dir.is_dir() {
            continue;
        }
        let docs = user_dir.join("Documents");
        if !docs.is_dir() {
            continue;
        }
        // Two roots: Documents/Games/ (convention) and Documents/ directly.
        let roots = [docs.join("Games"), docs];
        for root in &roots {
            if !root.is_dir() {
                continue;
            }
            if let Some(dir) = find_child_case_insensitive(root, STEAM_GAME_DIR) {
                if dir.is_dir() && has_executable(&dir) {
                    return Some(dir);
                }
            }
        }
    }
    None
}

fn find_executable(game_path: &Path) -> Option<PathBuf> {
    let Ok(entries) = fs::read_dir(game_path) else {
        return None;
    };

    let exe_names_lower: Vec<String> = EXECUTABLES.iter().map(|e| e.to_lowercase()).collect();
    let mut found: Option<PathBuf> = None;

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if let Some(idx) = exe_names_lower.iter().position(|e| e == &name) {
            if idx == 0 {
                return Some(entry.path());
            }
            if found.is_none() {
                found = Some(entry.path());
            }
        }
    }

    found
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
        assert!(plugin.detect_wine(&bottle).is_none());
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
        let detected = plugin.detect_wine(&bottle);
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
        let detected = plugin.detect_wine(&bottle);
        assert!(detected.is_some());
        assert_eq!(detected.unwrap().game_id, "fallout4");
    }

    fn make_fo4_bottle_with_game(subpath: &[&str]) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let mut game_dir = bottle_path.join("drive_c");
        for p in subpath {
            game_dir = game_dir.join(p);
        }
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join("Fallout4.exe"), b"fake").unwrap();
        (tmp, bottle_path)
    }

    #[test]
    fn detect_finds_game_in_program_files_x86() {
        let (_tmp, bottle_path) =
            make_fo4_bottle_with_game(&["Program Files (x86)", "Fallout 4"]);
        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        let detected = Fallout4Plugin.detect_wine(&bottle).expect("non-Steam detect");
        assert_eq!(detected.game_id, "fallout4");
    }

    #[test]
    fn detect_finds_game_in_games_dir() {
        let (_tmp, bottle_path) = make_fo4_bottle_with_game(&["Games", "Fallout 4"]);
        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        assert!(Fallout4Plugin.detect_wine(&bottle).is_some());
    }

    #[test]
    fn detect_finds_game_at_drive_c_root() {
        let (_tmp, bottle_path) = make_fo4_bottle_with_game(&["Fallout 4"]);
        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        assert!(
            Fallout4Plugin.detect_wine(&bottle).is_some(),
            "top-level drag-drop install"
        );
    }

    #[test]
    fn detect_finds_game_in_xbox_games() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = bottle_path
            .join("drive_c")
            .join("XboxGames")
            .join("Fallout 4")
            .join("Content");
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join("Fallout4.exe"), b"fake").unwrap();

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        let detected = Fallout4Plugin.detect_wine(&bottle).expect("Game Pass detect");
        assert_eq!(detected.game_id, "fallout4");
        assert!(detected.game_path.ends_with("Content"));
    }

    #[test]
    fn detect_non_steam_requires_exe() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = bottle_path
            .join("drive_c")
            .join("Program Files")
            .join("Fallout 4");
        fs::create_dir_all(&game_dir).unwrap();

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        assert!(
            Fallout4Plugin.detect_wine(&bottle).is_none(),
            "empty dir must not detect"
        );
    }

    #[test]
    fn detect_finds_game_in_documents_games() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        // Layout: <bottle>/drive_c/users/crossover/Documents/Games/Fallout 4/Fallout4.exe
        let game_dir = bottle_path
            .join("drive_c")
            .join("users")
            .join("crossover")
            .join("Documents")
            .join("Games")
            .join("Fallout 4");
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join("Fallout4.exe"), b"fake").unwrap();

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        let detected = Fallout4Plugin
            .detect_wine(&bottle)
            .expect("should detect Documents/Games install");
        assert_eq!(detected.game_id, "fallout4");
        assert_eq!(detected.game_path, game_dir);
    }
}
