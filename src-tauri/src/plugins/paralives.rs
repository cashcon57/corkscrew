//! Paralives support.
//!
//! Paralives has two mod channels:
//! - official content mods created by in-game Modding Tools / Steam Workshop;
//! - community script mods loaded through BepInEx 5 Mono x64.
//!
//! This plugin focuses on the script-mod path Corkscrew can fully automate for
//! Wine/Proton: route DLL mods to `BepInEx/plugins/<mod>/`, auto-install the
//! BepInEx bootstrap, and apply the required `winhttp=n,b` launch override.

use std::fs;
use std::path::{Path, PathBuf};

use crate::bottles::Bottle;
use crate::games::{DetectedGame, GamePlugin};
use crate::runtime::{GameRuntime, WineContext};
use crate::vortex_types::VortexModType;

const STEAM_APP_ID: &str = "1118520";
const STEAM_COMMON: &[&str] = &["Program Files (x86)", "Steam", "steamapps", "common"];
const STEAM_DIRS: &[&str] = &["Paralives"];
const EXECUTABLES: &[&str] = &["Paralives.exe"];

fn paralives_data_mods_dir(bottle: &Bottle) -> PathBuf {
    let users = bottle.drive_c().join("users");
    if let Ok(entries) = fs::read_dir(&users) {
        let user_dirs: Vec<_> = entries.flatten().collect();
        for preferred in ["steamuser", "crossover"] {
            for entry in &user_dirs {
                if entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(preferred)
                {
                    return entry
                        .path()
                        .join("AppData")
                        .join("LocalLow")
                        .join("Paralives")
                        .join("Paralives");
                }
            }
        }
    }
    bottle
        .drive_c()
        .join("users")
        .join("steamuser")
        .join("AppData")
        .join("LocalLow")
        .join("Paralives")
        .join("Paralives")
}

pub struct ParalivesPlugin;

impl GamePlugin for ParalivesPlugin {
    fn game_id(&self) -> &str {
        "paralives"
    }

    fn display_name(&self) -> &str {
        "Paralives"
    }

    fn nexus_slug(&self) -> &str {
        "paralives"
    }

    fn executables(&self) -> &[&str] {
        EXECUTABLES
    }

    fn detect_wine(&self, bottle: &Bottle) -> Option<DetectedGame> {
        let game_path = find_game_path(bottle)?;
        let exe_path = find_executable(&game_path, EXECUTABLES);
        if exe_path.is_none() {
            return None;
        }
        let data_dir = paralives_data_mods_dir(bottle);
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
            steam_app_id: Some(STEAM_APP_ID.to_string()),
            is_custom: false,
        })
    }

    fn get_data_dir(&self, game_path: &Path) -> PathBuf {
        game_path.to_path_buf()
    }

    fn get_plugins_file(&self, _game_path: &Path, _bottle: &Bottle) -> Option<PathBuf> {
        None
    }

    fn steam_launch_id(&self) -> Option<&str> {
        Some(STEAM_APP_ID)
    }

    fn use_legacy_data_dir(&self) -> bool {
        false
    }

    fn vortex_mod_types(&self) -> Vec<VortexModType> {
        vec![
            VortexModType {
                id: "Paralives_BepInEx".into(),
                priority: 90,
                target_path: "BepInEx/plugins".into(),
            },
            VortexModType {
                id: "Paralives_BepInExBootstrap".into(),
                priority: 95,
                target_path: ".".into(),
            },
            VortexModType {
                id: "Paralives_DataMod".into(),
                priority: 50,
                target_path: ".".into(),
            },
        ]
    }

    fn detect_mod_type_from_files(&self, files: &[String]) -> Option<String> {
        let normalized: Vec<String> = files
            .iter()
            .map(|f| f.replace('\\', "/").to_ascii_lowercase())
            .collect();

        if normalized.iter().any(|f| f == "winhttp.dll")
            && normalized.iter().any(|f| f.starts_with("bepinex/core/"))
        {
            return Some("Paralives_BepInExBootstrap".into());
        }

        if normalized
            .iter()
            .any(|f| f.starts_with("bepinexpack/winhttp.dll"))
            || normalized
                .iter()
                .any(|f| f.starts_with("bepinexpack/bepinex/"))
        {
            return Some("Paralives_BepInExBootstrap".into());
        }

        let dlls: Vec<&String> = normalized.iter().filter(|f| f.ends_with(".dll")).collect();
        if dlls.is_empty() {
            let official_asset_exts = [
                ".catalog", ".fbx", ".jpg", ".jpeg", ".mp3", ".obj", ".ogg", ".png", ".ttf",
                ".txt", ".wav",
            ];
            if normalized
                .iter()
                .any(|f| official_asset_exts.iter().any(|ext| f.ends_with(ext)))
            {
                return Some("Paralives_DataMod".into());
            }
            return None;
        }

        // Common Paralives Nexus shape: a single `ModName.dll` at archive root.
        // Also support the already-expanded BepInEx/plugins forms.
        let bepinex_shaped = normalized.iter().any(|f| f.starts_with("bepinex/plugins/"))
            || normalized.iter().any(|f| f.starts_with("plugins/"));
        let loose_root_dll = dlls.iter().any(|f| !f.contains('/'));

        if bepinex_shaped || loose_root_dll {
            Some("Paralives_BepInEx".into())
        } else {
            None
        }
    }

    fn protected_root_extensions(&self) -> Vec<&str> {
        vec![".exe", ".dll"]
    }

    fn categorize_mod_file(&self, rel_path: &str) -> Option<String> {
        let lower = rel_path.replace('\\', "/").to_ascii_lowercase();
        if lower.ends_with(".dll") {
            return Some("plugin".into());
        }
        if lower.ends_with(".cfg") || lower.ends_with(".json") || lower.ends_with(".xml") {
            return Some("config".into());
        }
        None
    }
}

pub fn register() {
    crate::games::register_plugin(std::sync::Arc::new(ParalivesPlugin));
}

fn find_game_path(bottle: &Bottle) -> Option<PathBuf> {
    if let Some(p) = check_steam_default(bottle) {
        return Some(p);
    }
    check_steam_library_folders(bottle)
}

fn check_steam_default(bottle: &Bottle) -> Option<PathBuf> {
    let common = bottle.find_path(STEAM_COMMON)?;
    for name in STEAM_DIRS {
        if let Some(dir) = find_child_ci(&common, name) {
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
        for name in STEAM_DIRS {
            if let Some(dir) = find_child_ci(&common, name) {
                if dir.is_dir() {
                    return Some(dir);
                }
            }
        }
    }
    None
}

fn find_executable(game_path: &Path, exes: &[&str]) -> Option<PathBuf> {
    let Ok(entries) = fs::read_dir(game_path) else {
        return None;
    };
    let lower: Vec<String> = exes.iter().map(|e| e.to_ascii_lowercase()).collect();
    let mut best: Option<(usize, PathBuf)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if let Some(idx) = lower.iter().position(|e| e == &name) {
            match &best {
                Some((cur, _)) if idx >= *cur => {}
                _ => best = Some((idx, entry.path())),
            }
        }
    }
    best.map(|(_, p)| p)
}

fn find_child_ci(parent: &Path, target: &str) -> Option<PathBuf> {
    let exact = parent.join(target);
    if exact.exists() {
        return Some(exact);
    }
    let target_lower = target.to_ascii_lowercase();
    let entries = fs::read_dir(parent).ok()?;
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().to_ascii_lowercase() == target_lower {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_is_correct() {
        let plugin = ParalivesPlugin;
        assert_eq!(plugin.game_id(), "paralives");
        assert_eq!(plugin.nexus_slug(), "paralives");
        assert_eq!(plugin.steam_launch_id(), Some("1118520"));
        assert!(!plugin.use_legacy_data_dir());
    }

    #[test]
    fn detects_loose_bepinex_script_mod() {
        let plugin = ParalivesPlugin;
        assert_eq!(
            plugin.detect_mod_type_from_files(&["ModOrganizer.dll".into()]),
            Some("Paralives_BepInEx".into())
        );
    }

    #[test]
    fn detects_expanded_bepinex_script_mod() {
        let plugin = ParalivesPlugin;
        assert_eq!(
            plugin.detect_mod_type_from_files(&[
                "BepInEx/plugins/ModOrganizer/ModOrganizer.dll".into()
            ]),
            Some("Paralives_BepInEx".into())
        );
    }

    #[test]
    fn detects_bepinex_bootstrap() {
        let plugin = ParalivesPlugin;
        assert_eq!(
            plugin.detect_mod_type_from_files(&[
                "winhttp.dll".into(),
                "BepInEx/core/BepInEx.dll".into(),
            ]),
            Some("Paralives_BepInExBootstrap".into())
        );
    }

    #[test]
    fn detects_official_asset_mods() {
        let plugin = ParalivesPlugin;
        assert_eq!(
            plugin.detect_mod_type_from_files(&["chair.fbx".into(), "chair.png".into()]),
            Some("Paralives_DataMod".into())
        );
    }
}
