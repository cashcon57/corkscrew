//! Hades II game plugin.
//!
//! Hades II (Supergiant) uses a Lua-based mod ecosystem via the community
//! **ModImporter** framework. Mods are installed as named subdirectories under
//! `Content/Mods/<ModName>/`, NOT as flat file replacements. We map
//! [`get_data_dir`] to `Content/Mods` so that Nexus archives shipped with a
//! `<ModName>/` top-level folder land exactly where ModImporter expects.
//!
//! There is no plugin/load-order system (no ESP/BSA equivalent), so
//! [`get_plugins_file`] returns `None` and the Load Order UI is hidden for
//! this game.

use std::fs;
use std::path::{Path, PathBuf};

use crate::bottles::Bottle;
use crate::games::{DetectedGame, GamePlugin};
use crate::runtime::{GameRuntime, WineContext};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Known executable names for Hades II (case-insensitive).
const EXECUTABLES: &[&str] = &["Hades2.exe", "HadesII.exe"];

/// Relative path components from `drive_c` to the default Steam library.
const STEAM_COMMON: &[&str] = &["Program Files (x86)", "Steam", "steamapps", "common"];

/// Known directory names inside Steam's common folder.
const STEAM_GAME_DIRS: &[&str] = &["Hades II", "HadesII", "Hades2"];

/// GOG installation paths (Hades II is not on GOG at time of writing, but we
/// check anyway in case that changes).
const GOG_PATHS: &[&[&str]] = &[
    &["GOG Games", "Hades II"],
    &["GOG Games", "HadesII"],
    &["Program Files", "GOG Galaxy", "Games", "Hades II"],
    &["Program Files (x86)", "GOG Galaxy", "Games", "Hades II"],
    &["Games", "Hades II"],
];

/// Additional non-Steam / non-GOG install paths (manual installs, drag-drop).
/// Each is anchored by the executable check to prevent false-positives.
const NON_STEAM_PATHS: &[&[&str]] = &[
    &["Program Files (x86)", "Hades II"],
    &["Program Files", "Hades II"],
    &["Games", "Hades II"],
    // Top-level drag-drop convention.
    &["Hades II"],
];

/// Xbox Game Pass install path.
const XBOX_GAMES_PATH: &[&str] = &["XboxGames", "Hades II", "Content"];

/// Steam App ID for Hades II (early access).
const STEAM_APP_ID: &str = "1145350";

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

pub struct Hades2Plugin;

impl GamePlugin for Hades2Plugin {
    fn game_id(&self) -> &str {
        "hades2"
    }

    fn display_name(&self) -> &str {
        "Hades II"
    }

    fn nexus_slug(&self) -> &str {
        "hades2"
    }

    fn executables(&self) -> &[&str] {
        EXECUTABLES
    }

    fn detect_wine(&self, bottle: &Bottle) -> Option<DetectedGame> {
        let game_path = find_game_path(bottle)?;
        if find_executable(&game_path).is_none() {
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

    /// Mods land under `<game>/Content/Mods/`. Each mod ships as a named
    /// subdirectory; the deployer preserves archive-relative paths, so an
    /// archive containing `MyMod/modfile.txt` deploys to
    /// `Content/Mods/MyMod/modfile.txt`.
    fn get_data_dir(&self, game_path: &Path) -> PathBuf {
        game_path.join("Content").join("Mods")
    }

    /// Hades II has no plugin/load-order file.
    fn get_plugins_file(&self, _game_path: &Path, _bottle: &Bottle) -> Option<PathBuf> {
        None
    }

    fn get_saves_dir(&self, _game_path: &Path, bottle: &Bottle) -> Option<PathBuf> {
        // Saves live under %USERPROFILE%\Saved Games\Hades II inside the bottle.
        // Walk drive_c/users/* looking for the first user with a Saved Games dir.
        let users = bottle.users_dir();
        if users.exists() {
            if let Ok(entries) = fs::read_dir(&users) {
                for entry in entries.flatten() {
                    let user_dir = entry.path();
                    if !user_dir.is_dir() {
                        continue;
                    }
                    let candidate = user_dir.join("Saved Games").join("Hades II");
                    if candidate.exists() {
                        return Some(candidate);
                    }
                }
            }
        }
        // Fallback: CrossOver convention.
        Some(
            users
                .join("crossover")
                .join("Saved Games")
                .join("Hades II"),
        )
    }

    fn steam_launch_id(&self) -> Option<&str> {
        Some(STEAM_APP_ID)
    }

    fn critical_files(&self) -> Vec<&str> {
        // Never let the cleaner delete these: ModImporter's entry points and
        // the vanilla RomFileSystem it patches. Patterns are relative paths
        // matched case-insensitively against lowercased rel paths (cleaner
        // does `lower == pattern.as_str()`), so patterns themselves must be
        // lowercase. Since `data_dir` for Hades II is `Content/Mods` and the
        // vanilla files live in `Content/Scripts/`, a cleaner walking the
        // mod directory will not encounter them — these entries are a
        // conservative belt-and-braces protection in case a future change
        // points the cleaner at the game root.
        vec![
            "content/scripts/romfilesystem.lua",
            "content/scripts/game.lua",
            "content/scripts/main.lua",
        ]
    }

    fn categorize_mod_file(&self, rel_path: &str) -> Option<String> {
        // Normalize separators: Wine archives may ship with `\` paths, but our
        // substring checks assume `/` boundaries.
        let lower = rel_path.replace('\\', "/").to_lowercase();
        if lower.ends_with(".lua") {
            return Some("script".into());
        }
        if lower.ends_with(".sjson") {
            return Some("data".into());
        }
        if lower.ends_with(".png") || lower.ends_with(".dds") || lower.contains("/textures/") {
            return Some("texture".into());
        }
        if lower.ends_with(".ogg") || lower.ends_with(".wav") || lower.contains("/audio/") {
            return Some("sound".into());
        }
        None
    }

    fn save_file_patterns(&self) -> Vec<&str> {
        vec![".sav", ".ctrls", "Saved Games/Hades II"]
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub fn register() {
    crate::games::register_plugin(std::sync::Arc::new(Hades2Plugin));
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
    if let Some(p) = check_gog_paths(bottle) {
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

fn check_gog_paths(bottle: &Bottle) -> Option<PathBuf> {
    for parts in GOG_PATHS {
        if let Some(p) = bottle.find_path(parts) {
            if p.is_dir() && find_executable(&p).is_some() {
                return Some(p);
            }
        }
    }
    None
}

/// Check generic non-Steam / non-GOG install paths, anchored by the executable.
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

/// Probe `<bottle>/drive_c/users/<user>/Documents/Games/Hades II/` and
/// `<bottle>/drive_c/users/<user>/Documents/Hades II/` for game installs.
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
            for name in STEAM_GAME_DIRS {
                if let Some(dir) = find_child_case_insensitive(root, name) {
                    if dir.is_dir() && find_executable(&dir).is_some() {
                        return Some(dir);
                    }
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
    let exe_lower: Vec<String> = EXECUTABLES.iter().map(|e| e.to_lowercase()).collect();
    // Collect every match with its preference index, then return the one with
    // the lowest index. Iteration order over `read_dir` is not deterministic,
    // so picking by preference rank rather than first-found is required.
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_metadata() {
        let p = Hades2Plugin;
        assert_eq!(p.game_id(), "hades2");
        assert_eq!(p.display_name(), "Hades II");
        assert_eq!(p.nexus_slug(), "hades2");
        assert!(p.executables().iter().any(|e| *e == "Hades2.exe"));
    }

    #[test]
    fn data_dir_points_to_content_mods() {
        let p = Hades2Plugin;
        let got = p.get_data_dir(&PathBuf::from("/fake/Hades II"));
        assert_eq!(got, PathBuf::from("/fake/Hades II/Content/Mods"));
    }

    #[test]
    fn no_plugins_file() {
        let p = Hades2Plugin;
        let bottle = Bottle {
            name: "T".into(),
            path: PathBuf::from("/tmp"),
            source: "Test".into(),
        };
        assert!(p.get_plugins_file(&PathBuf::from("/fake"), &bottle).is_none());
    }

    #[test]
    fn steam_launch_id_set() {
        assert_eq!(Hades2Plugin.steam_launch_id(), Some("1145350"));
    }

    #[test]
    fn detect_none_for_empty_bottle() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("Bottle");
        fs::create_dir_all(bottle_path.join("drive_c")).unwrap();
        let bottle = Bottle {
            name: "T".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        assert!(Hades2Plugin.detect_wine(&bottle).is_none());
    }

    #[test]
    fn detect_finds_game_in_steam_common() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("Bottle");
        let game_dir = bottle_path
            .join("drive_c")
            .join("Program Files (x86)")
            .join("Steam")
            .join("steamapps")
            .join("common")
            .join("Hades II");
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join("Hades2.exe"), b"fake").unwrap();

        let bottle = Bottle {
            name: "T".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        let detected = Hades2Plugin.detect_wine(&bottle).expect("detection");
        assert_eq!(detected.game_id, "hades2");
        assert_eq!(detected.data_dir, game_dir.join("Content").join("Mods"));
    }

    #[test]
    fn categorize_file_by_extension() {
        let p = Hades2Plugin;
        assert_eq!(p.categorize_mod_file("MyMod/modfile.lua"), Some("script".into()));
        assert_eq!(p.categorize_mod_file("MyMod/data.sjson"), Some("data".into()));
        assert_eq!(p.categorize_mod_file("MyMod/textures/foo.png"), Some("texture".into()));
        assert_eq!(p.categorize_mod_file("MyMod/audio/boom.ogg"), Some("sound".into()));
        assert_eq!(p.categorize_mod_file("MyMod/README.md"), None);
    }

    fn make_hades2_at(bottle_path: &Path, subpath: &[&str]) -> PathBuf {
        let mut game_dir = bottle_path.join("drive_c");
        for p in subpath {
            game_dir = game_dir.join(p);
        }
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join("Hades2.exe"), b"fake").unwrap();
        game_dir
    }

    #[test]
    fn detect_finds_game_in_program_files() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("Bottle");
        let game_dir = make_hades2_at(&bottle_path, &["Program Files", "Hades II"]);
        let bottle = Bottle {
            name: "T".into(),
            path: bottle_path,
            source: "T".into(),
        };
        let detected = Hades2Plugin.detect_wine(&bottle).expect("non-Steam detect");
        assert_eq!(detected.game_id, "hades2");
        assert_eq!(detected.game_path, game_dir);
    }

    #[test]
    fn detect_finds_game_in_games_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("Bottle");
        make_hades2_at(&bottle_path, &["Games", "Hades II"]);
        let bottle = Bottle {
            name: "T".into(),
            path: bottle_path,
            source: "T".into(),
        };
        assert!(Hades2Plugin.detect_wine(&bottle).is_some());
    }

    #[test]
    fn detect_finds_game_at_drive_c_root() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("Bottle");
        make_hades2_at(&bottle_path, &["Hades II"]);
        let bottle = Bottle {
            name: "T".into(),
            path: bottle_path,
            source: "T".into(),
        };
        assert!(Hades2Plugin.detect_wine(&bottle).is_some(), "top-level drag-drop");
    }

    #[test]
    fn detect_finds_game_in_xbox_games() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("Bottle");
        let game_dir = bottle_path
            .join("drive_c")
            .join("XboxGames")
            .join("Hades II")
            .join("Content");
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join("Hades2.exe"), b"fake").unwrap();

        let bottle = Bottle {
            name: "T".into(),
            path: bottle_path,
            source: "T".into(),
        };
        let detected = Hades2Plugin.detect_wine(&bottle).expect("Xbox Game Pass detect");
        assert_eq!(detected.game_id, "hades2");
        assert!(detected.game_path.ends_with("Content"));
    }

    #[test]
    fn detect_non_steam_requires_exe() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("Bottle");
        let game_dir = bottle_path.join("drive_c").join("Games").join("Hades II");
        fs::create_dir_all(&game_dir).unwrap();

        let bottle = Bottle {
            name: "T".into(),
            path: bottle_path,
            source: "T".into(),
        };
        assert!(
            Hades2Plugin.detect_wine(&bottle).is_none(),
            "empty dir must not detect"
        );
    }

    #[test]
    fn detect_finds_game_in_documents_games() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("Bottle");
        // Layout: <bottle>/drive_c/users/crossover/Documents/Games/Hades II/
        let game_dir = bottle_path
            .join("drive_c")
            .join("users")
            .join("crossover")
            .join("Documents")
            .join("Games")
            .join("Hades II");
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join("Hades2.exe"), b"fake").unwrap();

        let bottle = Bottle {
            name: "T".into(),
            path: bottle_path,
            source: "T".into(),
        };
        let detected = Hades2Plugin
            .detect_wine(&bottle)
            .expect("should detect Documents/Games install");
        assert_eq!(detected.game_id, "hades2");
        assert_eq!(detected.game_path, game_dir);
    }
}
