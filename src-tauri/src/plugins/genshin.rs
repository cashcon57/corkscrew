//! Genshin Impact game plugin.
//!
//! Genshin Impact (HoYoverse) is moddable on macOS via `gamesir-labs` Wine and
//! HoYoPlay launcher patches. The mod stack is built around **3DMigoto**, a
//! DirectX 11 hook that intercepts draw calls; the Genshin-specific fork is
//! **GIMI** (Genshin Impact Model Importer).
//!
//! ## Anti-cheat reality
//!
//! HoYo's HoyoProtect *can* detect 3DMigoto loader injection. Bans are rare
//! for cosmetic-only mods (texture/character swaps) but the risk is real.
//! The frontend gates installs through `AntiCheatWarningDialog` so the user
//! has to acknowledge the risk explicitly. See
//! `src/lib/components/AntiCheatWarningDialog.svelte`.
//!
//! ## Mods directory
//!
//! GIMI loads mods from `<game>/Mods/`. The directory does **not** ship with
//! the vanilla game — GIMI creates it on first launch, and we auto-create it
//! when the user runs their first install via [`find_or_create_genshin_mods_dir`].
//!
//! ## Detection
//!
//! Genshin can land in three layouts inside a Wine bottle:
//!
//! 1. **Steam** — `Program Files (x86)/Steam/steamapps/common/Genshin Impact/Genshin Impact game/`
//!    (Genshin's Steam release uses a nested `Genshin Impact game` directory).
//! 2. **HoYoPlay launcher** — `Program Files/HoYoPlay/games/Genshin Impact game/`
//!    (HoYo's official cross-platform launcher; the path is plural-less).
//! 3. **Standalone install** — `Program Files/Genshin Impact/Genshin Impact Game/`
//!    (legacy installs from before the HoYoPlay rebrand).
//!
//! The CN client uses `YuanShen.exe` instead of `GenshinImpact.exe`; both are
//! recognised here.

use std::fs;
use std::path::{Path, PathBuf};

use crate::bottles::Bottle;
use crate::games::{DetectedGame, GamePlugin};
use crate::runtime::{GameRuntime, WineContext};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Known executable filenames (case-insensitive). `YuanShen.exe` is the CN
/// client; the global build uses `GenshinImpact.exe`.
const EXECUTABLES: &[&str] = &["GenshinImpact.exe", "YuanShen.exe"];

/// Steam App ID for Genshin Impact (global release).
const STEAM_APP_ID: &str = "1517290";

/// Relative path components from `drive_c` to the default Steam library.
const STEAM_COMMON: &[&str] = &["Program Files (x86)", "Steam", "steamapps", "common"];

/// Steam game directory name. The actual executable lives one level deeper at
/// `<this>/Genshin Impact game/`.
const STEAM_GAME_DIR: &str = "Genshin Impact";

/// Inner directory under the Steam install that contains the executable.
/// Steam ships Genshin with this nested layout.
const STEAM_INNER_DIR: &str = "Genshin Impact game";

/// HoYoPlay launcher install paths to check (relative to `drive_c`).
/// HoYoPlay is HoYo's official cross-platform launcher; it scans for game
/// installs under `<launcher>/games/`.
const HOYOPLAY_PATHS: &[&[&str]] = &[
    &["Program Files", "HoYoPlay", "games", "Genshin Impact game"],
    &["Program Files (x86)", "HoYoPlay", "games", "Genshin Impact game"],
];

/// Standalone (legacy) install paths to check.
const STANDALONE_PATHS: &[&[&str]] = &[
    &["Program Files", "Genshin Impact", "Genshin Impact Game"],
    &["Program Files (x86)", "Genshin Impact", "Genshin Impact Game"],
    // Some users install the CN client via miHoYo's launcher to a `YuanShen`
    // root; the inner game folder is "YuanShen Game".
    &["Program Files", "miHoYo", "Genshin Impact", "Genshin Impact Game"],
    &[
        "Program Files (x86)",
        "miHoYo",
        "Genshin Impact",
        "Genshin Impact Game",
    ],
];

/// Additional generic non-Steam paths. These are checked AFTER the
/// HoYoPlay and standalone paths, and only for installs that don't follow
/// Genshin's usual nested `Genshin Impact/Genshin Impact game/` layout.
/// Each path points directly at the inner game directory that contains
/// the executable.
const NON_STEAM_PATHS: &[&[&str]] = &[
    // Drag-drop of the inner "Genshin Impact game" folder directly.
    &["Program Files (x86)", "Genshin Impact game"],
    &["Program Files", "Genshin Impact game"],
    &["Games", "Genshin Impact game"],
    &["Genshin Impact game"],
];

/// Xbox Game Pass install path.
const XBOX_GAMES_PATH: &[&str] = &["XboxGames", "Genshin Impact game", "Content"];

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

pub struct GenshinPlugin;

impl GamePlugin for GenshinPlugin {
    fn game_id(&self) -> &str {
        "genshin"
    }

    fn display_name(&self) -> &str {
        "Genshin Impact"
    }

    fn nexus_slug(&self) -> &str {
        "genshinimpact"
    }

    fn executables(&self) -> &[&str] {
        EXECUTABLES
    }

    fn detect_wine(&self, bottle: &Bottle) -> Option<DetectedGame> {
        let game_path = find_game_path(bottle)?;
        let exe_path = find_executable(&game_path);
        // Require the real executable to be present; an empty directory with
        // the right name should not register as a detected install.
        exe_path.as_ref()?;

        let data_dir = self.get_data_dir(&game_path);

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

    /// GIMI loads mods from `<game>/Mods/`. The directory is created by GIMI
    /// (or by Corkscrew via [`find_or_create_genshin_mods_dir`]); the vanilla
    /// install does not ship one.
    fn get_data_dir(&self, game_path: &Path) -> PathBuf {
        game_path.join("Mods")
    }

    fn get_plugins_file(&self, _game_path: &Path, _bottle: &Bottle) -> Option<PathBuf> {
        None
    }

    fn get_saves_dir(&self, _game_path: &Path, _bottle: &Bottle) -> Option<PathBuf> {
        // Genshin saves are stored server-side. There is no local save dir.
        None
    }

    /// Opt out of legacy data-dir merging. Genshin/GIMI mods each occupy
    /// their own `<modname>/` subfolder under `Mods/`, which is exactly what
    /// the [`crate::mod_types::GIMI_Mod`] entry resolves to. The default
    /// (`true`) would dump every archive into `Mods/` directly, mixing files
    /// across mods and breaking 3DMigoto's per-mod scoping.
    fn use_legacy_data_dir(&self) -> bool {
        false
    }

    fn steam_launch_id(&self) -> Option<&str> {
        Some(STEAM_APP_ID)
    }

    fn critical_files(&self) -> Vec<&str> {
        vec![
            "GenshinImpact.exe",
            "YuanShen.exe",
            "UnityPlayer.dll",
        ]
    }

    fn protected_root_extensions(&self) -> Vec<&str> {
        // Anything ending with these in the data dir root must never be
        // touched by the cleaner — they are vanilla game payload, not mod
        // content. Note that the data dir is `Mods/`, which only legitimately
        // contains mod subfolders; these are belt-and-braces protection in
        // case a future change repoints the cleaner at the game root.
        vec![".exe", ".pck", ".dat"]
    }

    fn save_file_patterns(&self) -> Vec<&str> {
        // Genshin has no local saves, but we list `.dat` defensively to keep
        // any cached game state out of the cleaner's deletion path.
        vec![".dat"]
    }

    fn categorize_mod_file(&self, rel_path: &str) -> Option<String> {
        let lower = rel_path.to_lowercase();

        // 3DMigoto config files.
        if lower.ends_with(".ini") {
            return Some("config".into());
        }
        // Texture replacements.
        if lower.ends_with(".dds") || lower.ends_with(".png") {
            return Some("texture".into());
        }
        // 3DMigoto buffer / index files — unique to GIMI/3DMigoto-style mods.
        if lower.ends_with(".buf") || lower.ends_with(".ib") {
            return Some("buffer".into());
        }
        if lower.ends_with(".dll") {
            return Some("library".into());
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub fn register() {
    crate::games::register_plugin(std::sync::Arc::new(GenshinPlugin));
}

// ---------------------------------------------------------------------------
// Mods/ directory helper
// ---------------------------------------------------------------------------

/// Ensure the GIMI `Mods/` directory exists for the given game install.
///
/// Vanilla Genshin does not ship a `Mods/` directory — GIMI creates it on
/// first run, and we create it here so that the first install works whether
/// or not the user has launched GIMI yet. Idempotent: returns `Ok(path)` if
/// the directory already exists.
///
/// Returns the absolute path to the created (or pre-existing) directory.
pub fn find_or_create_genshin_mods_dir(game_path: &Path) -> std::io::Result<PathBuf> {
    let mods_dir = game_path.join("Mods");
    if !mods_dir.exists() {
        fs::create_dir_all(&mods_dir)?;
    }
    Ok(mods_dir)
}

// ---------------------------------------------------------------------------
// Detection helpers
// ---------------------------------------------------------------------------

/// Attempt to locate the Genshin Impact installation directory inside a bottle.
///
/// Order: Steam default → Steam library folders (libraryfolders.vdf) →
/// HoYoPlay launcher → standalone/miHoYo install paths → generic non-Steam
/// paths → Xbox Game Pass.
fn find_game_path(bottle: &Bottle) -> Option<PathBuf> {
    if let Some(p) = check_steam_default(bottle) {
        return Some(p);
    }
    if let Some(p) = check_steam_library_folders(bottle) {
        return Some(p);
    }
    if let Some(p) = check_hoyoplay_paths(bottle) {
        return Some(p);
    }
    if let Some(p) = check_standalone_paths(bottle) {
        return Some(p);
    }
    if let Some(p) = check_non_steam_paths(bottle) {
        return Some(p);
    }
    check_xbox_games_path(bottle)
}

/// Check Steam's default common folder. The Steam release uses a nested
/// `Genshin Impact/Genshin Impact game/` layout; we descend into the inner
/// directory if present.
fn check_steam_default(bottle: &Bottle) -> Option<PathBuf> {
    let common = bottle.find_path(STEAM_COMMON)?;
    let outer = find_child_case_insensitive(&common, STEAM_GAME_DIR)?;
    if !outer.is_dir() {
        return None;
    }
    descend_into_inner_game_dir(&outer)
}

/// Parse `libraryfolders.vdf` and check each library path.
fn check_steam_library_folders(bottle: &Bottle) -> Option<PathBuf> {
    let steam_dir = bottle.find_path(&["Program Files (x86)", "Steam"])?;
    let vdf_primary = steam_dir.join("steamapps").join("libraryfolders.vdf");
    let vdf_alt = steam_dir.join("config").join("libraryfolders.vdf");
    let vdf_path = if vdf_primary.exists() {
        vdf_primary
    } else if vdf_alt.exists() {
        vdf_alt
    } else {
        return None;
    };

    let lib_paths = parse_library_folders_vdf(&vdf_path)?;
    for lib in lib_paths {
        let common = lib.join("steamapps").join("common");
        if let Some(outer) = find_child_case_insensitive(&common, STEAM_GAME_DIR) {
            if outer.is_dir() {
                if let Some(inner) = descend_into_inner_game_dir(&outer) {
                    return Some(inner);
                }
            }
        }
    }
    None
}

/// Check HoYoPlay launcher install paths.
fn check_hoyoplay_paths(bottle: &Bottle) -> Option<PathBuf> {
    for parts in HOYOPLAY_PATHS {
        if let Some(p) = bottle.find_path(parts) {
            if p.is_dir() {
                return Some(p);
            }
        }
    }
    None
}

/// Check legacy / standalone install paths.
fn check_standalone_paths(bottle: &Bottle) -> Option<PathBuf> {
    for parts in STANDALONE_PATHS {
        if let Some(p) = bottle.find_path(parts) {
            if p.is_dir() && find_executable(&p).is_some() {
                return Some(p);
            }
        }
    }
    None
}

/// Check generic non-Steam paths (direct drag-drop of the inner game dir).
fn check_non_steam_paths(bottle: &Bottle) -> Option<PathBuf> {
    for parts in NON_STEAM_PATHS {
        if let Some(p) = bottle.find_path(parts) {
            if p.is_dir() && find_executable(&p).is_some() {
                return Some(p);
            }
        }
    }
    None
}

/// Check Xbox Game Pass install location.
fn check_xbox_games_path(bottle: &Bottle) -> Option<PathBuf> {
    if let Some(p) = bottle.find_path(XBOX_GAMES_PATH) {
        if p.is_dir() && find_executable(&p).is_some() {
            return Some(p);
        }
    }
    None
}

/// Given an outer `Genshin Impact/` directory, descend into the inner
/// `Genshin Impact game/` (or `YuanShen Game/`) directory if present. Falls
/// back to the outer directory if no inner directory exists — some pre-launch
/// installs may not have created it yet.
fn descend_into_inner_game_dir(outer: &Path) -> Option<PathBuf> {
    if let Some(inner) = find_child_case_insensitive(outer, STEAM_INNER_DIR) {
        if inner.is_dir() {
            return Some(inner);
        }
    }
    if let Some(inner) = find_child_case_insensitive(outer, "YuanShen Game") {
        if inner.is_dir() {
            return Some(inner);
        }
    }
    // No inner dir — fall back to outer in case the user launched the game
    // through a custom layout. Detection will only succeed if an executable
    // is later found at this level.
    Some(outer.to_path_buf())
}

/// Find a Genshin executable inside `game_path`, preferring `GenshinImpact.exe`
/// over `YuanShen.exe`.
fn find_executable(game_path: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(game_path).ok()?;
    let exe_lower: Vec<String> = EXECUTABLES.iter().map(|e| e.to_lowercase()).collect();
    let mut best: Option<(usize, PathBuf)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if let Some(idx) = exe_lower.iter().position(|e| e == &name) {
            match &best {
                Some((cur, _)) if idx >= *cur => {}
                _ => best = Some((idx, entry.path())),
            }
        }
    }
    best.map(|(_, p)| p)
}

/// Find a child entry of `parent` whose name matches `target` case-insensitively.
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
        let plugin = GenshinPlugin;
        assert_eq!(plugin.game_id(), "genshin");
        assert_eq!(plugin.display_name(), "Genshin Impact");
        assert_eq!(plugin.nexus_slug(), "genshinimpact");
        assert_eq!(plugin.executables(), &["GenshinImpact.exe", "YuanShen.exe"]);
        assert_eq!(plugin.steam_launch_id(), Some("1517290"));
        assert!(!plugin.use_legacy_data_dir());
    }

    #[test]
    fn data_dir_is_mods_subfolder() {
        let plugin = GenshinPlugin;
        let game_path = PathBuf::from("/fake/Genshin Impact game");
        assert_eq!(
            plugin.get_data_dir(&game_path),
            PathBuf::from("/fake/Genshin Impact game/Mods"),
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
        let plugin = GenshinPlugin;
        assert!(plugin
            .get_plugins_file(Path::new("/fake"), &bottle)
            .is_none());
    }

    #[test]
    fn no_saves_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = Bottle {
            name: "Test".into(),
            path: tmp.path().to_path_buf(),
            source: "Test".into(),
        };
        let plugin = GenshinPlugin;
        assert!(plugin.get_saves_dir(Path::new("/fake"), &bottle).is_none());
    }

    #[test]
    fn categorize_mod_file_extensions() {
        let plugin = GenshinPlugin;
        assert_eq!(plugin.categorize_mod_file("merged.ini"), Some("config".into()));
        assert_eq!(plugin.categorize_mod_file("char.dds"), Some("texture".into()));
        assert_eq!(plugin.categorize_mod_file("preview.png"), Some("texture".into()));
        assert_eq!(plugin.categorize_mod_file("vb.buf"), Some("buffer".into()));
        assert_eq!(plugin.categorize_mod_file("ib.IB"), Some("buffer".into()));
        assert_eq!(plugin.categorize_mod_file("loader.dll"), Some("library".into()));
        assert_eq!(plugin.categorize_mod_file("readme.txt"), None);
    }

    #[test]
    fn protected_root_and_critical_files() {
        let plugin = GenshinPlugin;
        let critical = plugin.critical_files();
        assert!(critical.contains(&"GenshinImpact.exe"));
        assert!(critical.contains(&"YuanShen.exe"));
        assert!(critical.contains(&"UnityPlayer.dll"));

        let protected = plugin.protected_root_extensions();
        assert!(protected.contains(&".exe"));
        assert!(protected.contains(&".pck"));
        assert!(protected.contains(&".dat"));
    }

    /// Helper to lay out a Genshin install rooted at `game_root`.
    fn create_genshin_install(bottle_path: &Path, game_root: &Path, exe_name: &str) {
        fs::create_dir_all(game_root).unwrap();
        fs::write(game_root.join(exe_name), b"fake").unwrap();
        // Ensure drive_c exists for the bottle.
        fs::create_dir_all(bottle_path.join("drive_c")).unwrap();
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

        let plugin = GenshinPlugin;
        assert!(plugin.detect_wine(&bottle).is_none());
    }

    #[test]
    fn detect_finds_steam_install_with_inner_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let inner_dir = bottle_path
            .join("drive_c")
            .join("Program Files (x86)")
            .join("Steam")
            .join("steamapps")
            .join("common")
            .join("Genshin Impact")
            .join("Genshin Impact game");

        create_genshin_install(&bottle_path, &inner_dir, "GenshinImpact.exe");

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };

        let plugin = GenshinPlugin;
        let detected = plugin.detect_wine(&bottle).expect("should detect Steam install");
        assert_eq!(detected.game_id, "genshin");
        assert_eq!(detected.game_path, inner_dir);
        assert!(detected.exe_path.unwrap().ends_with("GenshinImpact.exe"));
    }

    #[test]
    fn detect_finds_hoyoplay_install() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = bottle_path
            .join("drive_c")
            .join("Program Files")
            .join("HoYoPlay")
            .join("games")
            .join("Genshin Impact game");

        create_genshin_install(&bottle_path, &game_dir, "GenshinImpact.exe");

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };

        let plugin = GenshinPlugin;
        let detected = plugin
            .detect_wine(&bottle)
            .expect("should detect HoYoPlay install");
        assert_eq!(detected.game_id, "genshin");
        assert_eq!(detected.game_path, game_dir);
    }

    #[test]
    fn detect_finds_standalone_install() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = bottle_path
            .join("drive_c")
            .join("Program Files")
            .join("Genshin Impact")
            .join("Genshin Impact Game");

        create_genshin_install(&bottle_path, &game_dir, "GenshinImpact.exe");

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };

        let plugin = GenshinPlugin;
        let detected = plugin
            .detect_wine(&bottle)
            .expect("should detect standalone install");
        assert_eq!(detected.game_path, game_dir);
    }

    #[test]
    fn detect_recognises_cn_yuanshen_exe() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = bottle_path
            .join("drive_c")
            .join("Program Files")
            .join("HoYoPlay")
            .join("games")
            .join("Genshin Impact game");

        create_genshin_install(&bottle_path, &game_dir, "YuanShen.exe");

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };

        let plugin = GenshinPlugin;
        let detected = plugin
            .detect_wine(&bottle)
            .expect("should detect CN client via YuanShen.exe");
        assert_eq!(detected.game_id, "genshin");
        assert!(
            detected
                .exe_path
                .unwrap()
                .to_string_lossy()
                .to_lowercase()
                .ends_with("yuanshen.exe")
        );
    }

    #[test]
    fn detect_skips_directory_without_executable() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = bottle_path
            .join("drive_c")
            .join("Program Files")
            .join("HoYoPlay")
            .join("games")
            .join("Genshin Impact game");

        // Create the directory but NO executable.
        fs::create_dir_all(&game_dir).unwrap();
        fs::create_dir_all(bottle_path.join("drive_c")).unwrap();

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };

        let plugin = GenshinPlugin;
        assert!(
            plugin.detect_wine(&bottle).is_none(),
            "empty install dir must not register"
        );
    }

    #[test]
    fn detected_data_dir_points_to_mods_subfolder() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = bottle_path
            .join("drive_c")
            .join("Program Files")
            .join("HoYoPlay")
            .join("games")
            .join("Genshin Impact game");

        create_genshin_install(&bottle_path, &game_dir, "GenshinImpact.exe");

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };

        let plugin = GenshinPlugin;
        let detected = plugin.detect_wine(&bottle).unwrap();
        assert_eq!(detected.data_dir, game_dir.join("Mods"));
    }

    #[test]
    fn find_or_create_genshin_mods_dir_creates_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let game_path = tmp.path().join("Genshin Impact game");
        fs::create_dir_all(&game_path).unwrap();

        let mods = find_or_create_genshin_mods_dir(&game_path).unwrap();
        assert_eq!(mods, game_path.join("Mods"));
        assert!(mods.is_dir(), "Mods/ should have been created");
    }

    #[test]
    fn find_or_create_genshin_mods_dir_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let game_path = tmp.path().join("Genshin Impact game");
        let mods_dir = game_path.join("Mods");
        fs::create_dir_all(&mods_dir).unwrap();
        // Plant a sentinel file so we can verify the directory wasn't recreated.
        fs::write(mods_dir.join("sentinel.txt"), b"existing").unwrap();

        let result = find_or_create_genshin_mods_dir(&game_path).unwrap();
        assert_eq!(result, mods_dir);
        assert!(
            mods_dir.join("sentinel.txt").exists(),
            "existing contents must be preserved"
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
        "path"		"D:\Games\Steam"
    }
}
"#,
        )
        .unwrap();

        let paths = parse_library_folders_vdf(&vdf).unwrap();
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[1], PathBuf::from("D:/Games/Steam"));
    }

    fn make_genshin_at(bottle_path: &Path, subpath: &[&str]) -> PathBuf {
        let mut game_dir = bottle_path.join("drive_c");
        for p in subpath {
            game_dir = game_dir.join(p);
        }
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join("GenshinImpact.exe"), b"fake").unwrap();
        fs::create_dir_all(bottle_path.join("drive_c")).unwrap();
        game_dir
    }

    #[test]
    fn detect_finds_game_in_program_files_x86_direct() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = make_genshin_at(&bottle_path, &["Program Files (x86)", "Genshin Impact game"]);
        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        let detected = GenshinPlugin.detect_wine(&bottle).expect("non-Steam detect");
        assert_eq!(detected.game_id, "genshin");
        assert_eq!(detected.game_path, game_dir);
    }

    #[test]
    fn detect_finds_game_in_games_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        make_genshin_at(&bottle_path, &["Games", "Genshin Impact game"]);
        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        assert!(GenshinPlugin.detect_wine(&bottle).is_some());
    }

    #[test]
    fn detect_finds_game_at_drive_c_root() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        make_genshin_at(&bottle_path, &["Genshin Impact game"]);
        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        assert!(GenshinPlugin.detect_wine(&bottle).is_some(), "top-level drag-drop");
    }

    #[test]
    fn detect_finds_game_in_xbox_games() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = bottle_path
            .join("drive_c")
            .join("XboxGames")
            .join("Genshin Impact game")
            .join("Content");
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join("GenshinImpact.exe"), b"fake").unwrap();
        fs::create_dir_all(bottle_path.join("drive_c")).unwrap();

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        let detected = GenshinPlugin.detect_wine(&bottle).expect("Xbox Game Pass detect");
        assert_eq!(detected.game_id, "genshin");
        assert!(detected.game_path.ends_with("Content"));
    }

    #[test]
    fn detect_non_steam_requires_exe() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        // Create the directory but no exe.
        let game_dir = bottle_path
            .join("drive_c")
            .join("Games")
            .join("Genshin Impact game");
        fs::create_dir_all(&game_dir).unwrap();
        fs::create_dir_all(bottle_path.join("drive_c")).unwrap();

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        assert!(
            GenshinPlugin.detect_wine(&bottle).is_none(),
            "empty dir must not detect"
        );
    }
}
