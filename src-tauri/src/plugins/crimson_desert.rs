//! Crimson Desert (Windows via Wine) game plugin.
//!
//! Pearl Abyss, Steam AppID `3321460`, March 2026 release. Runs on macOS via
//! CrossOver/Wine. Corkscrew targets the Windows build; the native macOS build
//! is handled by CDM2 and is out of scope here.
//!
//! Mod landscape (Windows):
//! - **ASI mods** — DLLs loaded via ultimate-asi-loader (installed as
//!   `xinput1_3.dll` or similar sideload). Works under Wine.
//! - **Data-only mods** — JSON patches, BNK audio, `.pathc` repacks,
//!   `.bsdiff`/`.xdelta` binary patches. Require per-format tooling Corkscrew
//!   does not yet have; flagged as future work.
//! - **No plugin/load-order system.** No ESP/ESL/ESM stack.
//!
//! `get_data_dir` is set to the **game root** so ASI sideload DLLs and loose
//! file drops land next to the exe. Per-format mod tooling (pathc/BNK/JSON
//! overlays) is deferred — see the CDM2 project for reference implementations.

use std::fs;
use std::path::{Path, PathBuf};

use crate::bottles::Bottle;
use crate::games::{DetectedGame, GamePlugin};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Known executable names (case-insensitive). Exact exe not publicly
/// documented pre-release; we match a few sensible variants Pearl Abyss uses
/// in related titles.
const EXECUTABLES: &[&str] = &["CrimsonDesert.exe", "Crimson Desert.exe", "CD.exe"];

const STEAM_COMMON: &[&str] = &["Program Files (x86)", "Steam", "steamapps", "common"];

const STEAM_GAME_DIRS: &[&str] = &["Crimson Desert"];

const STEAM_APP_ID: &str = "3321460";

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

pub struct CrimsonDesertPlugin;

impl GamePlugin for CrimsonDesertPlugin {
    fn game_id(&self) -> &str {
        "crimsondesert"
    }

    fn display_name(&self) -> &str {
        "Crimson Desert"
    }

    fn nexus_slug(&self) -> &str {
        "crimsondesert"
    }

    fn executables(&self) -> &[&str] {
        EXECUTABLES
    }

    fn detect(&self, bottle: &Bottle) -> Option<DetectedGame> {
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
            bottle_name: bottle.name.clone(),
            bottle_path: bottle.path.clone(),
        })
    }

    /// Deploy to the game root directory. This is the correct target for ASI
    /// sideload DLLs (ultimate-asi-loader, ReShade) and loose file drops.
    /// Data-only mod formats (pathc/BNK/JSON patches) require custom tooling
    /// outside the hardlink deployer.
    fn get_data_dir(&self, game_path: &Path) -> PathBuf {
        game_path.to_path_buf()
    }

    /// Crimson Desert has no plugin/load-order system.
    fn get_plugins_file(&self, _game_path: &Path, _bottle: &Bottle) -> Option<PathBuf> {
        None
    }

    fn steam_launch_id(&self) -> Option<&str> {
        Some(STEAM_APP_ID)
    }

    /// Protect the core game binaries + Pearl Abyss container formats from the
    /// cleaner. `.pathc` / `.pamt` / `.papgt` are the archive containers.
    fn protected_root_extensions(&self) -> Vec<&str> {
        vec![".exe", ".dll", ".pathc", ".pamt", ".papgt"]
    }

    fn categorize_mod_file(&self, rel_path: &str) -> Option<String> {
        let lower = rel_path.to_lowercase();
        if lower.ends_with(".asi") || lower.ends_with(".dll") {
            return Some("asi".into());
        }
        if lower.ends_with(".bnk") {
            return Some("sound".into());
        }
        if lower.ends_with(".pathc") || lower.ends_with(".pamt") || lower.ends_with(".papgt") {
            return Some("archive".into());
        }
        if lower.ends_with(".json") {
            return Some("data".into());
        }
        if lower.ends_with(".bsdiff") || lower.ends_with(".xdelta") {
            return Some("patch".into());
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub fn register() {
    crate::games::register_plugin(Box::new(CrimsonDesertPlugin));
}

// ---------------------------------------------------------------------------
// Detection helpers
// ---------------------------------------------------------------------------

fn find_game_path(bottle: &Bottle) -> Option<PathBuf> {
    if let Some(p) = check_steam_default(bottle) {
        return Some(p);
    }
    check_steam_library_folders(bottle)
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

fn find_executable(game_path: &Path) -> Option<PathBuf> {
    // Check game root first.
    if let Some(p) = find_executable_in(game_path) {
        return Some(p);
    }
    // Pearl Abyss titles sometimes nest the real binary in `bin64/` or similar.
    for sub in ["bin64", "binaries", "Bin", "Binaries"] {
        let nested = game_path.join(sub);
        if let Some(p) = find_executable_in(&nested) {
            return Some(p);
        }
    }
    None
}

fn find_executable_in(dir: &Path) -> Option<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return None;
    };
    let exe_lower: Vec<String> = EXECUTABLES.iter().map(|e| e.to_lowercase()).collect();
    let mut found: Option<PathBuf> = None;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if let Some(idx) = exe_lower.iter().position(|e| e == &name) {
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
        let p = CrimsonDesertPlugin;
        assert_eq!(p.game_id(), "crimsondesert");
        assert_eq!(p.display_name(), "Crimson Desert");
        assert_eq!(p.nexus_slug(), "crimsondesert");
        assert!(!p.executables().is_empty());
    }

    #[test]
    fn data_dir_is_game_root() {
        let p = CrimsonDesertPlugin;
        let g = PathBuf::from("/fake/Crimson Desert");
        assert_eq!(p.get_data_dir(&g), g);
    }

    #[test]
    fn no_plugins_file() {
        let p = CrimsonDesertPlugin;
        let b = Bottle {
            name: "T".into(),
            path: PathBuf::from("/tmp"),
            source: "T".into(),
        };
        assert!(p.get_plugins_file(&PathBuf::from("/fake"), &b).is_none());
    }

    #[test]
    fn steam_launch_id_set() {
        assert_eq!(CrimsonDesertPlugin.steam_launch_id(), Some("3321460"));
    }

    #[test]
    fn categorize_mod_file_recognizes_formats() {
        let p = CrimsonDesertPlugin;
        assert_eq!(p.categorize_mod_file("x/mod.asi"), Some("asi".into()));
        assert_eq!(p.categorize_mod_file("x/mod.dll"), Some("asi".into()));
        assert_eq!(p.categorize_mod_file("x/sound.bnk"), Some("sound".into()));
        assert_eq!(p.categorize_mod_file("x/0.pathc"), Some("archive".into()));
        assert_eq!(p.categorize_mod_file("x/patch.bsdiff"), Some("patch".into()));
        assert_eq!(p.categorize_mod_file("x/data.json"), Some("data".into()));
        assert_eq!(p.categorize_mod_file("x/readme.md"), None);
    }

    #[test]
    fn detect_none_for_empty_bottle() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("Bottle");
        fs::create_dir_all(bottle_path.join("drive_c")).unwrap();
        let b = Bottle {
            name: "T".into(),
            path: bottle_path,
            source: "T".into(),
        };
        assert!(CrimsonDesertPlugin.detect(&b).is_none());
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
            .join("Crimson Desert");
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join("CrimsonDesert.exe"), b"fake").unwrap();

        let b = Bottle {
            name: "T".into(),
            path: bottle_path,
            source: "T".into(),
        };
        let d = CrimsonDesertPlugin.detect(&b).expect("detection");
        assert_eq!(d.game_id, "crimsondesert");
        assert_eq!(d.data_dir, game_dir);
    }

    #[test]
    fn detect_finds_exe_in_bin64_subdir() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("Bottle");
        let game_dir = bottle_path
            .join("drive_c")
            .join("Program Files (x86)")
            .join("Steam")
            .join("steamapps")
            .join("common")
            .join("Crimson Desert");
        fs::create_dir_all(game_dir.join("bin64")).unwrap();
        fs::write(game_dir.join("bin64").join("CrimsonDesert.exe"), b"fake").unwrap();

        let b = Bottle {
            name: "T".into(),
            path: bottle_path,
            source: "T".into(),
        };
        let d = CrimsonDesertPlugin.detect(&b).expect("detection");
        assert_eq!(d.game_id, "crimsondesert");
    }
}
