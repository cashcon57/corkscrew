//! Grand Theft Auto V game plugin.
//!
//! Implements [`crate::games::GamePlugin`] for detecting and managing
//! GTA V installations inside Wine bottles. Supports:
//! - Steam (AppID 271590, common dir "Grand Theft Auto V")
//! - Rockstar Games Launcher (`Rockstar Games/Grand Theft Auto V/`)
//! - Epic Games Store (`Epic Games/GTAV/`)
//!
//! ## Phase 1 scope
//!
//! This plugin covers the foundation: ASI Loader / ScriptHookV /
//! ScriptHookVDotNet / LUA Plugin scripts, plus `dlcpacks/` add-on
//! recognition. Out of scope (deferred): `.rpf` archive editing,
//! `.oiv` package parsing, FiveM (separate phases).
//!
//! ## Layout
//!
//! GTA V's mod stack is unusual — almost everything lives in the game
//! root next to `GTA5.exe`:
//!
//! - `dinput8.dll` (ASI Loader, Alexander Blade)
//! - `ScriptHookV.dll`
//! - `*.asi` plugins (auto-loaded by the ASI loader)
//! - `ScriptHookVDotNet.asi` + `ScriptHookVDotNet*.dll`
//! - `scripts/` — host folder for SHVDN C# scripts and LUA Plugin scripts
//! - `dlcpacks/` — DLC add-on content; each subfolder contains a `dlc.rpf`
//!
//! There is no engine "data" subdirectory like Skyrim — `data_dir` is the
//! game root. Routing of individual archives is handled by the mod-type
//! registry in [`crate::mod_types`].
//!
//! ## Steam launcher stub
//!
//! On Steam installs there are two executables:
//! - `PlayGTAV.exe` — Rockstar's Social Club login / launcher stub.
//! - `GTA5.exe` — the actual game.
//!
//! When `PlayGTAV.exe` is present we return it from [`launch_executable`]
//! (mirrors the Hogwarts Legacy pattern) so that Social Club / Steam DRM
//! gets a chance to run before the game starts.

use std::fs;
use std::path::{Path, PathBuf};

use crate::bottles::Bottle;
use crate::games::{DetectedGame, GamePlugin};
use crate::runtime::{GameRuntime, WineContext};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Known executable names for GTA V (checked case-insensitively).
const EXECUTABLES: &[&str] = &["GTA5.exe", "PlayGTAV.exe", "gtav.exe"];

/// Relative path from `drive_c` to the default Steam library `common` dir.
const STEAM_COMMON: &[&str] = &["Program Files (x86)", "Steam", "steamapps", "common"];

/// Steam common directory name for GTA V.
const STEAM_GAME_DIR: &str = "Grand Theft Auto V";

/// Rockstar Games Launcher install paths (relative to `drive_c`).
const ROCKSTAR_PATHS: &[&[&str]] = &[
    &["Program Files", "Rockstar Games", "Grand Theft Auto V"],
    &["Program Files (x86)", "Rockstar Games", "Grand Theft Auto V"],
];

/// Epic Games Store install paths (relative to `drive_c`). Epic uses the
/// short name "GTAV" rather than the long Steam form.
const EPIC_PATHS: &[&[&str]] = &[
    &["Program Files", "Epic Games", "GTAV"],
    &["Program Files (x86)", "Epic Games", "GTAV"],
];

/// Additional non-Steam / non-Epic install paths. Covers manual installs,
/// drag-drop, and the "Enhanced" edition naming. Each is anchored by
/// `has_any_executable` to prevent false-positives.
const NON_STEAM_PATHS: &[&[&str]] = &[
    // "Enhanced" edition naming (newer PC release).
    &["Program Files (x86)", "Grand Theft Auto V Enhanced"],
    &["Program Files", "Grand Theft Auto V Enhanced"],
    &["Games", "Grand Theft Auto V Enhanced"],
    &["Grand Theft Auto V Enhanced"],
    // Classic naming.
    &["Program Files (x86)", "Grand Theft Auto V"],
    &["Program Files", "Grand Theft Auto V"],
    &["Games", "Grand Theft Auto V"],
    &["Grand Theft Auto V"],
];

/// Xbox Game Pass install path (uses "Grand Theft Auto V" naming).
const XBOX_GAMES_PATHS: &[&[&str]] = &[
    &["XboxGames", "Grand Theft Auto V Enhanced", "Content"],
    &["XboxGames", "Grand Theft Auto V", "Content"],
];

// ---------------------------------------------------------------------------
// GtaVPlugin
// ---------------------------------------------------------------------------

/// Game plugin for Grand Theft Auto V.
pub struct GtaVPlugin;

impl GamePlugin for GtaVPlugin {
    fn game_id(&self) -> &str {
        "gtav"
    }

    fn display_name(&self) -> &str {
        "Grand Theft Auto V"
    }

    fn nexus_slug(&self) -> &str {
        "grandtheftautov"
    }

    fn executables(&self) -> &[&str] {
        EXECUTABLES
    }

    fn detect_wine(&self, bottle: &Bottle) -> Option<DetectedGame> {
        let game_path = find_game_path(bottle)?;

        // Verify at least one of the known executables actually exists.
        if !has_any_executable(&game_path) {
            return None;
        }

        let data_dir = self.get_data_dir(&game_path);
        let exe_path = find_primary_executable(&game_path);

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
            is_custom: false,
        })
    }

    fn get_data_dir(&self, game_path: &Path) -> PathBuf {
        // GTA V has no engine "data" directory — everything (ASI, SHV,
        // SHVDN, LUA Plugin scripts, dlcpacks/) deploys to the game root.
        // mod_types routes individual archives within this root.
        game_path.to_path_buf()
    }

    fn use_legacy_data_dir(&self) -> bool {
        // GTA V mods come in many shapes (ASI, .NET, Lua, dlcpacks).
        // Defer to the mod-types registry instead of merging everything
        // into a single Bethesda-style data directory.
        false
    }

    fn get_plugins_file(&self, _game_path: &Path, _bottle: &Bottle) -> Option<PathBuf> {
        None // Not a Bethesda game — no plugin load order.
    }

    fn get_saves_dir(&self, _game_path: &Path, bottle: &Bottle) -> Option<PathBuf> {
        // GTA V saves live under Documents in the user's profile, but
        // Rockstar buries them in a hashed profile dir. We point at the
        // parent directory and let consumers walk it.
        let documents = bottle
            .drive_c()
            .join("users")
            .join("crossover")
            .join("Documents")
            .join("Rockstar Games")
            .join("GTA V");
        Some(documents.join("Profiles"))
    }

    fn steam_launch_id(&self) -> Option<&str> {
        Some("271590")
    }

    fn launch_executable(&self, game_path: &Path) -> Option<PathBuf> {
        // Prefer Rockstar's launcher stub when present — it handles
        // Social Club / Steam DRM authentication. Fall back to GTA5.exe
        // for installs without the launcher (Rockstar Launcher direct
        // installs and some Epic copies).
        if let Some(stub) = find_file_case_insensitive(game_path, "PlayGTAV.exe") {
            return Some(stub);
        }
        find_file_case_insensitive(game_path, "GTA5.exe")
    }

    fn critical_files(&self) -> Vec<&str> {
        // Root-level vanilla files that the cleaner must NEVER delete.
        // common.rpf and x64.rpf are the master archives; deleting them
        // bricks the install. GTAVLauncher.exe is the Rockstar launcher.
        vec![
            "GTA5.exe",
            "PlayGTAV.exe",
            "GTAVLauncher.exe",
            "common.rpf",
            "x64.rpf",
        ]
    }

    fn protected_root_extensions(&self) -> Vec<&str> {
        // Vanilla game executables, DLLs, and master archives must not be
        // touched at the game root. ASI files ARE mod artifacts so we do
        // NOT protect `.asi` here.
        vec![".exe", ".dll", ".rpf"]
    }

    fn save_file_patterns(&self) -> Vec<&str> {
        // SGTA00010 etc. are extension-less. No reliable in-tree pattern.
        vec![]
    }

    fn categorize_mod_file(&self, rel_path: &str) -> Option<String> {
        let lower = rel_path.to_lowercase();
        if lower.ends_with(".asi") {
            return Some("asi".into());
        }
        if lower.ends_with(".dll") {
            return Some("library".into());
        }
        if lower.ends_with(".lua") {
            return Some("script".into());
        }
        if lower.ends_with(".oiv") || lower.ends_with(".rpf") {
            return Some("archive".into());
        }
        if lower.ends_with(".ini") || lower.ends_with(".cfg") || lower.ends_with(".xml") {
            return Some("config".into());
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Detection helpers
// ---------------------------------------------------------------------------

/// Locate the GTA V install inside a bottle.
fn find_game_path(bottle: &Bottle) -> Option<PathBuf> {
    // 1. Default Steam library.
    if let Some(path) = check_steam_default(bottle) {
        return Some(path);
    }
    // 2. Additional Steam libraries from `libraryfolders.vdf`.
    if let Some(path) = check_steam_library_folders(bottle) {
        return Some(path);
    }
    // 3. Rockstar Games Launcher install.
    if let Some(path) = check_paths(bottle, ROCKSTAR_PATHS) {
        return Some(path);
    }
    // 4. Epic Games Store install.
    if let Some(path) = check_paths(bottle, EPIC_PATHS) {
        return Some(path);
    }
    // 5. Generic non-Steam paths (manual installs, drag-drop, Enhanced edition).
    if let Some(path) = check_non_steam_paths(bottle) {
        return Some(path);
    }
    // 6. Xbox Game Pass installs.
    if let Some(path) = check_xbox_games_paths(bottle) {
        return Some(path);
    }
    // 7. Host Documents folder (via CrossOver symlink).
    check_user_documents_paths(bottle)
}

fn check_steam_default(bottle: &Bottle) -> Option<PathBuf> {
    let common = bottle.find_path(STEAM_COMMON)?;
    let game_dir = find_child_case_insensitive(&common, STEAM_GAME_DIR)?;
    if game_dir.is_dir() {
        Some(game_dir)
    } else {
        None
    }
}

fn check_steam_library_folders(bottle: &Bottle) -> Option<PathBuf> {
    let steam_dir = bottle.find_path(&["Program Files (x86)", "Steam"])?;
    let primary = steam_dir.join("steamapps").join("libraryfolders.vdf");
    let vdf_path = if primary.exists() {
        primary
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

fn check_paths(bottle: &Bottle, candidates: &[&[&str]]) -> Option<PathBuf> {
    for parts in candidates {
        if let Some(path) = bottle.find_path(parts) {
            if path.is_dir() && has_any_executable(&path) {
                return Some(path);
            }
        }
    }
    None
}

/// Check generic non-Steam / non-launcher install paths, anchored by the
/// executable check.
fn check_non_steam_paths(bottle: &Bottle) -> Option<PathBuf> {
    for parts in NON_STEAM_PATHS {
        if let Some(path) = bottle.find_path(parts) {
            if path.is_dir() && has_any_executable(&path) {
                return Some(path);
            }
        }
    }
    None
}

/// Check Xbox Game Pass install locations.
fn check_xbox_games_paths(bottle: &Bottle) -> Option<PathBuf> {
    for parts in XBOX_GAMES_PATHS {
        if let Some(path) = bottle.find_path(parts) {
            if path.is_dir() && has_any_executable(&path) {
                return Some(path);
            }
        }
    }
    None
}

/// Probe `<bottle>/drive_c/users/<user>/Documents/Games/Grand Theft Auto V/` and
/// `<bottle>/drive_c/users/<user>/Documents/Grand Theft Auto V/` for GTA V installs.
///
/// CrossOver typically symlinks the bottle's `Documents` directory to the
/// host `~/Documents`, so games kept at `~/Documents/Games/<name>/` on the
/// macOS host are reachable through the bottle filesystem.
fn check_user_documents_paths(bottle: &Bottle) -> Option<PathBuf> {
    let users_dir = bottle.users_dir();
    let Ok(entries) = fs::read_dir(&users_dir) else {
        return None;
    };
    // All GTA V directory names we scan for (Steam + Enhanced naming).
    const GAME_DIRS: &[&str] = &[
        "Grand Theft Auto V Enhanced",
        "Grand Theft Auto V",
        "GTAV",
    ];
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
            for name in GAME_DIRS {
                if let Some(dir) = find_child_case_insensitive(root, name) {
                    if dir.is_dir() && has_any_executable(&dir) {
                        return Some(dir);
                    }
                }
            }
            // Broad fallback: scan every subdir of this root and check for the
            // game's executables. Catches non-standard folder names that won't
            // match GAME_DIRS.
            if let Ok(entries) = fs::read_dir(root) {
                for entry in entries.flatten() {
                    let dir = entry.path();
                    if !dir.is_dir() {
                        continue;
                    }
                    if has_any_executable(&dir) {
                        return Some(dir);
                    }
                }
            }
        }
    }
    None
}

/// Verify at least one known executable exists in the game root.
fn has_any_executable(game_path: &Path) -> bool {
    EXECUTABLES
        .iter()
        .any(|name| find_file_case_insensitive(game_path, name).is_some())
}

/// Find the canonical detection executable, preferring `GTA5.exe`
/// (the real game binary) over `PlayGTAV.exe` (the launcher stub).
fn find_primary_executable(game_path: &Path) -> Option<PathBuf> {
    if let Some(exe) = find_file_case_insensitive(game_path, "GTA5.exe") {
        return Some(exe);
    }
    if let Some(exe) = find_file_case_insensitive(game_path, "PlayGTAV.exe") {
        return Some(exe);
    }
    find_file_case_insensitive(game_path, "gtav.exe")
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

fn find_file_case_insensitive(dir: &Path, target: &str) -> Option<PathBuf> {
    crate::fs_ci::find_file_ci(dir, target)
}

fn find_child_case_insensitive(parent: &Path, target: &str) -> Option<PathBuf> {
    crate::fs_ci::find_child_ci(parent, target)
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
// Registration
// ---------------------------------------------------------------------------

/// Register the GTA V plugin with the global game plugin registry.
pub fn register() {
    crate::games::register_plugin(std::sync::Arc::new(GtaVPlugin));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_metadata() {
        let plugin = GtaVPlugin;
        assert_eq!(plugin.game_id(), "gtav");
        assert_eq!(plugin.display_name(), "Grand Theft Auto V");
        assert_eq!(plugin.nexus_slug(), "grandtheftautov");
        assert_eq!(plugin.executables(), &["GTA5.exe", "PlayGTAV.exe", "gtav.exe"]);
        assert_eq!(plugin.steam_launch_id(), Some("271590"));
        assert!(!plugin.use_legacy_data_dir());
    }

    #[test]
    fn data_dir_is_game_root() {
        let plugin = GtaVPlugin;
        let game_path = PathBuf::from("/fake/Grand Theft Auto V");
        assert_eq!(plugin.get_data_dir(&game_path), game_path);
    }

    #[test]
    fn no_plugins_file() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = Bottle {
            name: "Test".into(),
            path: tmp.path().to_path_buf(),
            source: "Test".into(),
        };
        let plugin = GtaVPlugin;
        assert!(plugin
            .get_plugins_file(Path::new("/fake"), &bottle)
            .is_none());
    }

    #[test]
    fn critical_files_includes_master_rpfs() {
        let plugin = GtaVPlugin;
        let crit = plugin.critical_files();
        assert!(crit.contains(&"common.rpf"));
        assert!(crit.contains(&"x64.rpf"));
        assert!(crit.contains(&"GTA5.exe"));
        assert!(crit.contains(&"PlayGTAV.exe"));
        assert!(crit.contains(&"GTAVLauncher.exe"));
    }

    #[test]
    fn protected_root_extensions_includes_rpf() {
        let plugin = GtaVPlugin;
        let exts = plugin.protected_root_extensions();
        assert!(exts.contains(&".exe"));
        assert!(exts.contains(&".dll"));
        assert!(exts.contains(&".rpf"));
        // .asi is a mod artifact — must NOT be protected.
        assert!(!exts.contains(&".asi"));
    }

    #[test]
    fn categorize_mod_files() {
        let plugin = GtaVPlugin;
        assert_eq!(plugin.categorize_mod_file("MyMod.asi").as_deref(), Some("asi"));
        assert_eq!(
            plugin.categorize_mod_file("ScriptHookVDotNet.dll").as_deref(),
            Some("library")
        );
        assert_eq!(
            plugin.categorize_mod_file("scripts/addins/menyoo.lua").as_deref(),
            Some("script")
        );
        assert_eq!(
            plugin.categorize_mod_file("dlcpacks/foo/dlc.rpf").as_deref(),
            Some("archive")
        );
        assert_eq!(plugin.categorize_mod_file("pack.oiv").as_deref(), Some("archive"));
        assert_eq!(plugin.categorize_mod_file("config.ini").as_deref(), Some("config"));
        assert!(plugin.categorize_mod_file("readme.txt").is_none());
    }

    /// Helper: write a fake GTA5.exe (and optionally PlayGTAV.exe) into
    /// the given game directory, plus the bottle's drive_c.
    fn create_gtav_install(bottle_path: &Path, game_dir: &Path, with_play_stub: bool) {
        fs::create_dir_all(game_dir).unwrap();
        fs::create_dir_all(bottle_path.join("drive_c")).unwrap();
        fs::write(game_dir.join("GTA5.exe"), b"fake").unwrap();
        if with_play_stub {
            fs::write(game_dir.join("PlayGTAV.exe"), b"fake").unwrap();
        }
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
        let plugin = GtaVPlugin;
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
            .join("Grand Theft Auto V");
        create_gtav_install(&bottle_path, &game_dir, true);

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        let plugin = GtaVPlugin;
        let detected = plugin.detect_wine(&bottle).expect("Steam install should detect");
        assert_eq!(detected.game_id, "gtav");
        assert_eq!(detected.display_name, "Grand Theft Auto V");
        assert_eq!(detected.nexus_slug, "grandtheftautov");
        assert_eq!(detected.game_path, game_dir);
    }

    #[test]
    fn detect_finds_game_in_rockstar_path() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = bottle_path
            .join("drive_c")
            .join("Program Files")
            .join("Rockstar Games")
            .join("Grand Theft Auto V");
        create_gtav_install(&bottle_path, &game_dir, false);

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        let plugin = GtaVPlugin;
        let detected = plugin.detect_wine(&bottle).expect("Rockstar install should detect");
        assert_eq!(detected.game_id, "gtav");
    }

    #[test]
    fn detect_finds_game_in_epic_path() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = bottle_path
            .join("drive_c")
            .join("Program Files")
            .join("Epic Games")
            .join("GTAV");
        create_gtav_install(&bottle_path, &game_dir, false);

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        let plugin = GtaVPlugin;
        let detected = plugin.detect_wine(&bottle).expect("Epic install should detect");
        assert_eq!(detected.game_id, "gtav");
    }

    #[test]
    fn launch_executable_prefers_play_stub_on_steam() {
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path().join("Grand Theft Auto V");
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join("GTA5.exe"), b"real").unwrap();
        fs::write(game_dir.join("PlayGTAV.exe"), b"stub").unwrap();

        let plugin = GtaVPlugin;
        let launched = plugin.launch_executable(&game_dir).unwrap();
        assert_eq!(
            launched.file_name().unwrap().to_string_lossy().to_lowercase(),
            "playgtav.exe"
        );
    }

    #[test]
    fn launch_executable_falls_back_to_gta5_when_no_stub() {
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path().join("Grand Theft Auto V");
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join("GTA5.exe"), b"real").unwrap();

        let plugin = GtaVPlugin;
        let launched = plugin.launch_executable(&game_dir).unwrap();
        assert_eq!(
            launched.file_name().unwrap().to_string_lossy().to_lowercase(),
            "gta5.exe"
        );
    }

    #[test]
    fn detect_executable_path_prefers_real_exe() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = bottle_path
            .join("drive_c")
            .join("Program Files (x86)")
            .join("Steam")
            .join("steamapps")
            .join("common")
            .join("Grand Theft Auto V");
        create_gtav_install(&bottle_path, &game_dir, true);

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        let plugin = GtaVPlugin;
        let detected = plugin.detect_wine(&bottle).unwrap();
        let exe = detected.exe_path.unwrap();
        // exe_path (detection) should be GTA5.exe — the real binary.
        assert_eq!(
            exe.file_name().unwrap().to_string_lossy().to_lowercase(),
            "gta5.exe"
        );
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
    }
    "1"
    {
        "path"		"D:\SteamLibrary"
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

    fn make_gtav_at(bottle_path: &Path, subpath: &[&str]) -> PathBuf {
        let mut game_dir = bottle_path.join("drive_c");
        for p in subpath {
            game_dir = game_dir.join(p);
        }
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join("GTA5.exe"), b"fake").unwrap();
        game_dir
    }

    #[test]
    fn detect_finds_game_in_program_files_x86() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = make_gtav_at(&bottle_path, &["Program Files (x86)", "Grand Theft Auto V"]);
        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        let detected = GtaVPlugin.detect_wine(&bottle).expect("non-Steam detect");
        assert_eq!(detected.game_id, "gtav");
        assert_eq!(detected.game_path, game_dir);
    }

    #[test]
    fn detect_finds_enhanced_edition() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = make_gtav_at(
            &bottle_path,
            &["Program Files", "Grand Theft Auto V Enhanced"],
        );
        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        let detected = GtaVPlugin.detect_wine(&bottle).expect("Enhanced edition detect");
        assert_eq!(detected.game_id, "gtav");
        assert_eq!(detected.game_path, game_dir);
    }

    #[test]
    fn detect_finds_game_in_games_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        make_gtav_at(&bottle_path, &["Games", "Grand Theft Auto V"]);
        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        assert!(GtaVPlugin.detect_wine(&bottle).is_some());
    }

    #[test]
    fn detect_finds_game_at_drive_c_root() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        make_gtav_at(&bottle_path, &["Grand Theft Auto V"]);
        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        assert!(GtaVPlugin.detect_wine(&bottle).is_some(), "top-level drag-drop");
    }

    #[test]
    fn detect_finds_game_in_xbox_games() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = bottle_path
            .join("drive_c")
            .join("XboxGames")
            .join("Grand Theft Auto V")
            .join("Content");
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join("GTA5.exe"), b"fake").unwrap();

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        let detected = GtaVPlugin.detect_wine(&bottle).expect("Xbox Game Pass detect");
        assert_eq!(detected.game_id, "gtav");
        assert!(detected.game_path.ends_with("Content"));
    }

    #[test]
    fn detect_non_steam_requires_exe() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = bottle_path
            .join("drive_c")
            .join("Program Files")
            .join("Grand Theft Auto V");
        fs::create_dir_all(&game_dir).unwrap();

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        assert!(
            GtaVPlugin.detect_wine(&bottle).is_none(),
            "empty dir must not detect"
        );
    }

    #[test]
    fn detect_finds_game_in_documents_games() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        // Layout: <bottle>/drive_c/users/crossover/Documents/Games/Grand Theft Auto V/
        let game_dir = bottle_path
            .join("drive_c")
            .join("users")
            .join("crossover")
            .join("Documents")
            .join("Games")
            .join("Grand Theft Auto V");
        create_gtav_install(&bottle_path, &game_dir, false);

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        let detected = GtaVPlugin
            .detect_wine(&bottle)
            .expect("should detect Documents/Games install");
        assert_eq!(detected.game_id, "gtav");
        assert_eq!(detected.game_path, game_dir);
    }
}
