//! The Sims 4 game plugin.
//!
//! The Sims 4 (Electronic Arts) is moddable on macOS via CrossOver / Wine.
//! Unlike most games, mods do NOT live inside the game install directory.
//! They are deployed to **`Documents/Electronic Arts/The Sims 4/Mods/`**
//! under the user's documents tree.
//!
//! ## Layout
//!
//! - Game install: `<bottle>/drive_c/.../The Sims 4/Game/Bin/TS4_x64.exe`
//!   (Steam path: `<steam common>/The Sims 4/Game/Bin/TS4_x64.exe`,
//!   EA App path: typically `Program Files/EA Games/The Sims 4/Game/Bin/`).
//! - Mods deploy: `<documents>/Electronic Arts/The Sims 4/Mods/`
//! - `Resource.cfg`: lives at the root of `Mods/` and controls how deep
//!   the game scans subdirectories for `.package` files. Corkscrew creates
//!   a sensible default (5-deep recursion) on first install; an existing
//!   `Resource.cfg` is never overwritten — users frequently customize it.
//!
//! ## Mod file types
//!
//! - `.package`  — binary CC archives (custom content, gameplay tuning)
//! - `.ts4script` — Python script mods (loaded by name from `Mods/`)
//! - `.zip` / `.7z` — common shipping format; nested CC archives extract
//!   into a per-mod subfolder.
//!
//! ## No load order
//!
//! The Sims 4 has no ESP/plugin load-order system. `get_plugins_file`
//! returns `None` and the Load Order page in the UI is hidden via
//! `GAMES_WITHOUT_PLUGINS`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::bottles::Bottle;
use crate::games::{DetectedGame, GamePlugin};
use crate::runtime::{GameRuntime, WineContext};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Known executable names for The Sims 4 (case-insensitive).
const EXECUTABLES: &[&str] = &["TS4_x64.exe", "TS4.exe"];

/// Steam App ID for The Sims 4.
const STEAM_APP_ID: &str = "1222670";

/// Directory the executable lives in, relative to the game root.
const EXE_RELATIVE_DIR: &[&str] = &["Game", "Bin"];

/// Steam's `steamapps/common` location inside a bottle.
const STEAM_COMMON: &[&str] = &["Program Files (x86)", "Steam", "steamapps", "common"];

/// Known directory names inside Steam's common folder.
const STEAM_GAME_DIRS: &[&str] = &["The Sims 4"];

/// EA App / Origin install paths to check.
const EA_PATHS: &[&[&str]] = &[
    &["Program Files", "EA Games", "The Sims 4"],
    &["Program Files (x86)", "EA Games", "The Sims 4"],
    &["Program Files", "Origin Games", "The Sims 4"],
    &["Program Files (x86)", "Origin Games", "The Sims 4"],
    &["Program Files", "Electronic Arts", "The Sims 4"],
    &["Program Files (x86)", "Electronic Arts", "The Sims 4"],
];

/// Additional non-EA / non-Steam install paths (manual installs, drag-drop).
/// Each is validated by `has_real_executable` to prevent false-positives.
const NON_STEAM_PATHS: &[&[&str]] = &[
    &["Program Files (x86)", "The Sims 4"],
    &["Program Files", "The Sims 4"],
    &["Games", "The Sims 4"],
    // Top-level drag-drop convention.
    &["The Sims 4"],
];

/// Xbox Game Pass install path.
const XBOX_GAMES_PATH: &[&str] = &["XboxGames", "The Sims 4", "Content"];

/// Default `Resource.cfg` content (5-deep recursion — community standard).
pub const DEFAULT_RESOURCE_CFG: &str = "Priority 500\n\
DirectoryFiles enabled autoupdate\n\
PackedFile *.package\n\
PackedFile */*.package\n\
PackedFile */*/*.package\n\
PackedFile */*/*/*.package\n\
PackedFile */*/*/*/*.package\n\
PackedFile */*/*/*/*/*.package\n";

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

/// Game plugin for The Sims 4 (Electronic Arts).
pub struct Sims4Plugin;

impl GamePlugin for Sims4Plugin {
    fn game_id(&self) -> &str {
        "sims4"
    }

    fn display_name(&self) -> &str {
        "The Sims 4"
    }

    fn nexus_slug(&self) -> &str {
        "thesims4"
    }

    fn executables(&self) -> &[&str] {
        EXECUTABLES
    }

    fn detect_wine(&self, bottle: &Bottle) -> Option<DetectedGame> {
        let game_path = find_game_path(bottle)?;
        if !has_real_executable(&game_path) {
            return None;
        }

        let exe_path = find_executable(&game_path);
        let data_dir = find_or_create_sims4_mods_dir(bottle).unwrap_or_else(|_| {
            // If the documents dir can't be created (permissions, fresh
            // bottle, etc.) fall back to the conventional Wine path so
            // detection still completes — first install will retry.
            bottle
                .documents_dir()
                .join("Electronic Arts")
                .join("The Sims 4")
                .join("Mods")
        });

        // Best-effort: ensure Resource.cfg exists. Failures are non-fatal
        // (the user may not have write permission yet).
        let _ = ensure_resource_cfg(&data_dir);

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

    /// Mods deploy to the user's Documents folder, NOT the game install.
    /// This is the default Sims 4 mod path. Note: detection populates
    /// `DetectedGame.data_dir` directly; `get_data_dir(game_path)` here
    /// has no access to the bottle, so we return a sentinel inside the
    /// game tree. Callers should prefer `DetectedGame.data_dir`.
    fn get_data_dir(&self, game_path: &Path) -> PathBuf {
        // Fallback only — real path lives in `DetectedGame.data_dir`.
        game_path.join("Mods")
    }

    /// The Sims 4 has no plugin/load-order file.
    fn get_plugins_file(&self, _game_path: &Path, _bottle: &Bottle) -> Option<PathBuf> {
        None
    }

    fn get_saves_dir(&self, _game_path: &Path, bottle: &Bottle) -> Option<PathBuf> {
        Some(
            bottle
                .documents_dir()
                .join("Electronic Arts")
                .join("The Sims 4")
                .join("saves"),
        )
    }

    fn steam_launch_id(&self) -> Option<&str> {
        Some(STEAM_APP_ID)
    }

    /// Use mod-type detection — Sims 4 archives are routed by the
    /// `Sims4_Package` and `Sims4_Script` entries in [`crate::mod_types`].
    fn use_legacy_data_dir(&self) -> bool {
        false
    }

    fn categorize_mod_file(&self, rel_path: &str) -> Option<String> {
        let lower = rel_path.replace('\\', "/").to_lowercase();
        if lower.ends_with(".package") {
            return Some("package".into());
        }
        if lower.ends_with(".ts4script") {
            return Some("script".into());
        }
        if lower.ends_with(".zip") || lower.ends_with(".7z") {
            return Some("archive".into());
        }
        if lower.ends_with(".cfg") {
            return Some("config".into());
        }
        None
    }

    fn protected_root_extensions(&self) -> Vec<&str> {
        // The data dir is `Documents/.../Mods/`, which contains user
        // content only. These extensions identify game files (in case
        // a future change ever points the cleaner at the game install
        // tree); never delete them.
        vec![".exe", ".dll", ".bin", ".dat"]
    }

    fn critical_files(&self) -> Vec<&str> {
        // No SKSE-equivalent. The Mods/Resource.cfg file is always
        // recreated by `ensure_resource_cfg` on next launch, so it
        // doesn't need hard protection here.
        vec![]
    }

    fn save_file_patterns(&self) -> Vec<&str> {
        // Sims 4 saves live alongside Mods/ but in a `saves/` sibling.
        // The data_dir for this plugin is just `Mods/`, so save files
        // shouldn't normally appear there — list anyway for safety.
        vec![".save", ".ver", "saves/"]
    }
}

// ---------------------------------------------------------------------------
// Mods directory helpers
// ---------------------------------------------------------------------------

/// Locate the Sims 4 Mods directory, creating it (and the parent path) if
/// it doesn't yet exist.
///
/// Returns the absolute path to `Documents/Electronic Arts/The Sims 4/Mods/`.
/// The default `Resource.cfg` is laid down on creation via
/// [`ensure_resource_cfg`].
pub fn find_or_create_sims4_mods_dir(bottle: &Bottle) -> io::Result<PathBuf> {
    let mods_dir = bottle
        .documents_dir()
        .join("Electronic Arts")
        .join("The Sims 4")
        .join("Mods");

    if !mods_dir.exists() {
        fs::create_dir_all(&mods_dir)?;
    }

    // Idempotent — leaves an existing Resource.cfg untouched.
    let _ = ensure_resource_cfg(&mods_dir);

    Ok(mods_dir)
}

/// Ensure a `Resource.cfg` file exists at the root of `mods_dir` with
/// the community-standard 5-deep recursion config.
///
/// **Never** overwrites an existing file — users frequently customize
/// priority, depth, or exclude lists.
pub fn ensure_resource_cfg(mods_dir: &Path) -> io::Result<()> {
    if !mods_dir.exists() {
        fs::create_dir_all(mods_dir)?;
    }
    let cfg_path = mods_dir.join("Resource.cfg");
    if cfg_path.exists() {
        return Ok(());
    }
    fs::write(&cfg_path, DEFAULT_RESOURCE_CFG)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Detection helpers
// ---------------------------------------------------------------------------

fn find_game_path(bottle: &Bottle) -> Option<PathBuf> {
    if let Some(p) = check_steam_default(bottle) {
        return Some(p);
    }
    if let Some(p) = check_steam_library_folders(bottle) {
        return Some(p);
    }
    if let Some(p) = check_ea_paths(bottle) {
        return Some(p);
    }
    if let Some(p) = check_non_steam_paths(bottle) {
        return Some(p);
    }
    if let Some(p) = check_xbox_games_path(bottle) {
        return Some(p);
    }
    check_user_documents_paths(bottle)
}

fn check_steam_default(bottle: &Bottle) -> Option<PathBuf> {
    let common = bottle.find_path(STEAM_COMMON)?;
    for name in STEAM_GAME_DIRS {
        if let Some(dir) = find_child_case_insensitive(&common, name) {
            if dir.is_dir() {
                return Some(dir);
            }
        }
    }
    None
}

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
        for name in STEAM_GAME_DIRS {
            if let Some(dir) = find_child_case_insensitive(&common, name) {
                if dir.is_dir() {
                    return Some(dir);
                }
            }
        }
    }
    None
}

fn check_ea_paths(bottle: &Bottle) -> Option<PathBuf> {
    for parts in EA_PATHS {
        if let Some(p) = bottle.find_path(parts) {
            if p.is_dir() && has_real_executable(&p) {
                return Some(p);
            }
        }
    }
    None
}

/// Check generic non-Steam / non-EA install paths, anchored by the real
/// executable check.
fn check_non_steam_paths(bottle: &Bottle) -> Option<PathBuf> {
    for parts in NON_STEAM_PATHS {
        if let Some(p) = bottle.find_path(parts) {
            if p.is_dir() && has_real_executable(&p) {
                return Some(p);
            }
        }
    }
    None
}

/// Check Xbox Game Pass install location.
fn check_xbox_games_path(bottle: &Bottle) -> Option<PathBuf> {
    if let Some(p) = bottle.find_path(XBOX_GAMES_PATH) {
        if p.is_dir() && has_real_executable(&p) {
            return Some(p);
        }
    }
    None
}

/// Probe `<bottle>/drive_c/users/<user>/Documents/Games/The Sims 4/` and
/// `<bottle>/drive_c/users/<user>/Documents/The Sims 4/` for game installs.
///
/// CrossOver typically symlinks the bottle's `Documents` directory to the
/// host `~/Documents`, so games kept at `~/Documents/Games/<name>/` on the
/// macOS host are reachable through the bottle filesystem.
///
/// Sims 4 uses a `Game/Bin/` sub-directory for the executable. Validation
/// requires `TS4_x64.exe` in `<root>/Game/Bin/` (mirrors `has_real_executable`).
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
            for name in STEAM_GAME_DIRS {
                if let Some(dir) = find_child_case_insensitive(root, name) {
                    if dir.is_dir() && has_real_executable(&dir) {
                        return Some(dir);
                    }
                }
            }
            // Broad fallback: scan every subdir of this root and check for the
            // game's executables. Catches non-standard folder names that won't
            // match STEAM_GAME_DIRS.
            if let Ok(entries) = fs::read_dir(root) {
                for entry in entries.flatten() {
                    let dir = entry.path();
                    if !dir.is_dir() {
                        continue;
                    }
                    if has_real_executable(&dir) {
                        return Some(dir);
                    }
                }
            }
        }
    }
    None
}

/// Verify that `<game>/Game/Bin/TS4_x64.exe` (or `TS4.exe`) exists.
fn has_real_executable(game_path: &Path) -> bool {
    let bin_dir = resolve_bin_dir(game_path);
    EXECUTABLES
        .iter()
        .any(|exe| find_file_case_insensitive(&bin_dir, exe).is_some())
}

fn find_executable(game_path: &Path) -> Option<PathBuf> {
    let bin_dir = resolve_bin_dir(game_path);
    for exe in EXECUTABLES {
        if let Some(p) = find_file_case_insensitive(&bin_dir, exe) {
            return Some(p);
        }
    }
    // Fallback: very old / non-standard layouts ship the exe at root.
    for exe in EXECUTABLES {
        if let Some(p) = find_file_case_insensitive(game_path, exe) {
            return Some(p);
        }
    }
    None
}

fn resolve_bin_dir(game_path: &Path) -> PathBuf {
    let mut dir = game_path.to_path_buf();
    for component in EXE_RELATIVE_DIR {
        if let Some(child) = find_child_case_insensitive(&dir, component) {
            dir = child;
        } else {
            dir = dir.join(component);
        }
    }
    dir
}

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
// Steam libraryfolders.vdf parsing
// ---------------------------------------------------------------------------

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
// Registration
// ---------------------------------------------------------------------------

pub fn register() {
    crate::games::register_plugin(std::sync::Arc::new(Sims4Plugin));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bottle(path: PathBuf) -> Bottle {
        Bottle {
            name: "TestBottle".into(),
            path,
            source: "Test".into(),
        }
    }

    /// Create a fake Sims 4 install at `game_root`, with the executable
    /// laid out at `Game/Bin/TS4_x64.exe`. Also ensures `drive_c` exists
    /// on the bottle.
    fn create_sims4_install(bottle_path: &Path, game_root: &Path) {
        let bin_dir = game_root.join("Game").join("Bin");
        fs::create_dir_all(&bin_dir).unwrap();
        fs::write(bin_dir.join("TS4_x64.exe"), b"fake").unwrap();
        fs::create_dir_all(bottle_path.join("drive_c")).unwrap();
    }

    /// Create the standard CrossOver-style user `Documents` directory on
    /// `bottle_path` so `bottle.documents_dir()` returns it.
    fn create_user_documents(bottle_path: &Path) -> PathBuf {
        let docs = bottle_path
            .join("drive_c")
            .join("users")
            .join("crossover")
            .join("Documents");
        fs::create_dir_all(&docs).unwrap();
        docs
    }

    #[test]
    fn plugin_metadata() {
        let plugin = Sims4Plugin;
        assert_eq!(plugin.game_id(), "sims4");
        assert_eq!(plugin.display_name(), "The Sims 4");
        assert_eq!(plugin.nexus_slug(), "thesims4");
        assert_eq!(plugin.executables(), &["TS4_x64.exe", "TS4.exe"]);
        assert_eq!(plugin.steam_launch_id(), Some("1222670"));
        assert!(!plugin.use_legacy_data_dir());
        assert!(plugin
            .get_plugins_file(Path::new("/fake"), &make_bottle("/tmp".into()))
            .is_none());
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
            .join("The Sims 4");
        create_sims4_install(&bottle_path, &game_dir);
        create_user_documents(&bottle_path);

        let bottle = make_bottle(bottle_path);
        let detected = Sims4Plugin.detect_wine(&bottle).expect("should detect");
        assert_eq!(detected.game_id, "sims4");
        assert_eq!(detected.game_path, game_dir);
        // data_dir lives outside the game install — under Documents.
        let s = detected.data_dir.to_string_lossy().replace('\\', "/");
        assert!(s.contains("Documents"));
        assert!(s.contains("The Sims 4"));
        assert!(s.ends_with("/Mods"));
    }

    #[test]
    fn detect_finds_game_in_ea_path() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = bottle_path
            .join("drive_c")
            .join("Program Files")
            .join("EA Games")
            .join("The Sims 4");
        create_sims4_install(&bottle_path, &game_dir);
        create_user_documents(&bottle_path);

        let bottle = make_bottle(bottle_path);
        let detected = Sims4Plugin.detect_wine(&bottle).expect("should detect");
        assert_eq!(detected.game_id, "sims4");
        assert_eq!(detected.game_path, game_dir);
    }

    #[test]
    fn detect_returns_none_when_no_install() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        fs::create_dir_all(bottle_path.join("drive_c")).unwrap();

        let bottle = make_bottle(bottle_path);
        assert!(Sims4Plugin.detect_wine(&bottle).is_none());
    }

    #[test]
    fn detect_requires_real_exe_in_game_bin() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = bottle_path
            .join("drive_c")
            .join("Program Files (x86)")
            .join("Steam")
            .join("steamapps")
            .join("common")
            .join("The Sims 4");
        // Create the dir but NOT the Game/Bin/TS4_x64.exe.
        fs::create_dir_all(&game_dir).unwrap();
        fs::create_dir_all(bottle_path.join("drive_c")).unwrap();

        let bottle = make_bottle(bottle_path);
        assert!(Sims4Plugin.detect_wine(&bottle).is_none());
    }

    #[test]
    fn data_dir_resolves_to_documents_mods() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let docs = create_user_documents(&bottle_path);

        let bottle = make_bottle(bottle_path);
        let mods = find_or_create_sims4_mods_dir(&bottle).unwrap();

        let expected = docs.join("Electronic Arts").join("The Sims 4").join("Mods");
        assert_eq!(mods, expected);
        assert!(mods.exists(), "Mods directory should be created");
    }

    #[test]
    fn find_or_create_sims4_mods_dir_creates_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        // No Documents directory anywhere — fallback path should still work.
        fs::create_dir_all(bottle_path.join("drive_c")).unwrap();

        let bottle = make_bottle(bottle_path);
        let mods = find_or_create_sims4_mods_dir(&bottle).unwrap();
        assert!(mods.exists());
        assert!(mods.ends_with(Path::new("Mods")));
    }

    #[test]
    fn ensure_resource_cfg_creates_default() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp.path().join("Mods");
        fs::create_dir_all(&mods_dir).unwrap();

        ensure_resource_cfg(&mods_dir).unwrap();

        let cfg_path = mods_dir.join("Resource.cfg");
        let content = fs::read_to_string(&cfg_path).unwrap();
        assert!(content.contains("Priority 500"));
        assert!(content.contains("DirectoryFiles enabled autoupdate"));
        assert!(content.contains("PackedFile *.package"));
        // 5-deep recursion config ships six PackedFile lines.
        assert_eq!(content.matches("PackedFile").count(), 6);
    }

    #[test]
    fn ensure_resource_cfg_idempotent_on_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp.path().join("Mods");
        fs::create_dir_all(&mods_dir).unwrap();

        // Pretend a user customized the file.
        let user_cfg = "Priority 999\n# my custom config\n";
        fs::write(mods_dir.join("Resource.cfg"), user_cfg).unwrap();

        ensure_resource_cfg(&mods_dir).unwrap();

        let content = fs::read_to_string(mods_dir.join("Resource.cfg")).unwrap();
        assert_eq!(content, user_cfg, "Existing Resource.cfg must not be overwritten");
    }

    #[test]
    fn ensure_resource_cfg_creates_parent_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp.path().join("Nested").join("Mods");
        // Don't pre-create — ensure_resource_cfg should mkdir -p.
        ensure_resource_cfg(&mods_dir).unwrap();

        assert!(mods_dir.exists());
        assert!(mods_dir.join("Resource.cfg").exists());
    }

    #[test]
    fn categorize_mod_file_routes_correctly() {
        let plugin = Sims4Plugin;
        assert_eq!(
            plugin.categorize_mod_file("MyMod/awesome.package"),
            Some("package".into())
        );
        assert_eq!(
            plugin.categorize_mod_file("Author/Mod/script.ts4script"),
            Some("script".into())
        );
        assert_eq!(
            plugin.categorize_mod_file("nested\\mods\\hair.PACKAGE"),
            Some("package".into()),
        );
        assert_eq!(
            plugin.categorize_mod_file("packed.zip"),
            Some("archive".into())
        );
        assert_eq!(
            plugin.categorize_mod_file("packed.7z"),
            Some("archive".into())
        );
        assert_eq!(
            plugin.categorize_mod_file("Resource.cfg"),
            Some("config".into())
        );
        assert_eq!(plugin.categorize_mod_file("readme.txt"), None);
    }

    #[test]
    fn protected_root_extensions_includes_game_files() {
        let plugin = Sims4Plugin;
        let exts = plugin.protected_root_extensions();
        assert!(exts.contains(&".exe"));
        assert!(exts.contains(&".dll"));
        assert!(exts.contains(&".bin"));
        assert!(exts.contains(&".dat"));
    }

    #[test]
    fn critical_files_is_empty() {
        let plugin = Sims4Plugin;
        assert!(plugin.critical_files().is_empty());
    }

    #[test]
    fn saves_dir_under_documents() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let docs = create_user_documents(&bottle_path);

        let bottle = make_bottle(bottle_path);
        let plugin = Sims4Plugin;
        let saves = plugin
            .get_saves_dir(Path::new("/fake/game"), &bottle)
            .expect("saves dir");
        let expected = docs.join("Electronic Arts").join("The Sims 4").join("saves");
        assert_eq!(saves, expected);
    }

    fn make_sims4_at(bottle_path: &Path, subpath: &[&str]) -> PathBuf {
        let mut game_root = bottle_path.join("drive_c");
        for p in subpath {
            game_root = game_root.join(p);
        }
        let bin_dir = game_root.join("Game").join("Bin");
        fs::create_dir_all(&bin_dir).unwrap();
        fs::write(bin_dir.join("TS4_x64.exe"), b"fake").unwrap();
        game_root
    }

    #[test]
    fn detect_finds_game_in_program_files_x86() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = make_sims4_at(&bottle_path, &["Program Files (x86)", "The Sims 4"]);
        create_user_documents(&bottle_path);

        let bottle = make_bottle(bottle_path);
        let detected = Sims4Plugin.detect_wine(&bottle).expect("non-Steam detect");
        assert_eq!(detected.game_id, "sims4");
        assert_eq!(detected.game_path, game_dir);
    }

    #[test]
    fn detect_finds_game_in_games_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        make_sims4_at(&bottle_path, &["Games", "The Sims 4"]);
        create_user_documents(&bottle_path);

        let bottle = make_bottle(bottle_path);
        assert!(Sims4Plugin.detect_wine(&bottle).is_some());
    }

    #[test]
    fn detect_finds_game_at_drive_c_root() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        make_sims4_at(&bottle_path, &["The Sims 4"]);
        create_user_documents(&bottle_path);

        let bottle = make_bottle(bottle_path);
        assert!(Sims4Plugin.detect_wine(&bottle).is_some(), "top-level drag-drop");
    }

    #[test]
    fn detect_finds_game_in_xbox_games() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        // XboxGames/<GameDir>/Content is the game root.
        let game_dir = bottle_path
            .join("drive_c")
            .join("XboxGames")
            .join("The Sims 4")
            .join("Content");
        let bin_dir = game_dir.join("Game").join("Bin");
        fs::create_dir_all(&bin_dir).unwrap();
        fs::write(bin_dir.join("TS4_x64.exe"), b"fake").unwrap();
        create_user_documents(&bottle_path);

        let bottle = make_bottle(bottle_path);
        let detected = Sims4Plugin.detect_wine(&bottle).expect("Xbox Game Pass detect");
        assert_eq!(detected.game_id, "sims4");
        assert_eq!(detected.game_path, game_dir);
    }

    #[test]
    fn detect_non_steam_requires_exe() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        // Create the dir but no exe.
        let game_dir = bottle_path
            .join("drive_c")
            .join("Program Files")
            .join("The Sims 4");
        fs::create_dir_all(&game_dir).unwrap();

        let bottle = make_bottle(bottle_path);
        assert!(
            Sims4Plugin.detect_wine(&bottle).is_none(),
            "empty dir must not detect"
        );
    }

    #[test]
    fn detect_finds_game_in_documents_games() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        // Layout: <bottle>/drive_c/users/crossover/Documents/Games/The Sims 4/
        // with the real exe at Game/Bin/TS4_x64.exe
        let game_dir = bottle_path
            .join("drive_c")
            .join("users")
            .join("crossover")
            .join("Documents")
            .join("Games")
            .join("The Sims 4");
        create_sims4_install(&bottle_path, &game_dir);
        // Also create the Documents dir so data_dir can be resolved.
        create_user_documents(&bottle_path);

        let bottle = make_bottle(bottle_path);
        let detected = Sims4Plugin
            .detect_wine(&bottle)
            .expect("should detect Documents/Games install");
        assert_eq!(detected.game_id, "sims4");
        assert_eq!(detected.game_path, game_dir);
    }
}
