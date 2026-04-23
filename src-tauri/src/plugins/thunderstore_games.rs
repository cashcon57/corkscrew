//! Thunderstore games (BepInEx / Unity) — batch-registered via a spec table.
//!
//! All Thunderstore-supported games share the same modding pattern:
//! - **Unity** runtime
//! - **BepInEx** loader (DLL at game root as proxy, mods in `BepInEx/plugins/`)
//! - No ESP/ESL/ESM stack → Load Order page hidden
//! - Steam as primary distribution
//! - Thunderstore as canonical mod catalog
//!
//! Instead of a file-per-game copy-paste (see hades2.rs, crimson_desert.rs),
//! we define one generic plugin keyed by a spec and register N instances.
//! Adding a new Thunderstore game = add one row to `SPECS`.
//!
//! Ports the shape of what r2modmac does on macOS-native, adapted for
//! Windows-via-Wine. Full Thunderstore browse/install UI lands in session 2.

use std::fs;
use std::path::{Path, PathBuf};

use crate::bottles::Bottle;
use crate::games::{DetectedGame, GamePlugin};

const STEAM_COMMON: &[&str] = &["Program Files (x86)", "Steam", "steamapps", "common"];

/// Static spec for one Thunderstore game. Adding a game = add a row here.
pub struct ThunderstoreGameSpec {
    pub game_id: &'static str,
    pub display_name: &'static str,
    /// Slug used with the NexusMods API (rare for Thunderstore games, but set
    /// for ones that also exist on Nexus, e.g. Palworld, Valheim).
    pub nexus_slug: &'static str,
    /// Thunderstore community identifier (e.g. `"lethal-company"`).
    pub thunderstore_community: &'static str,
    /// Steam AppID (for `steam://` URL launch).
    pub steam_app_id: &'static str,
    /// Steam common-directory names to scan (case-insensitive).
    pub steam_dirs: &'static [&'static str],
    /// Known executable names inside the game directory.
    pub executables: &'static [&'static str],
    /// Mod directory relative to game root. For BepInEx games this is
    /// usually `"BepInEx/plugins"`.
    pub mod_dir: &'static [&'static str],
}

pub const SPECS: &[ThunderstoreGameSpec] = &[
    ThunderstoreGameSpec {
        game_id: "silksong",
        display_name: "Hollow Knight: Silksong",
        nexus_slug: "hollowknightsilksong",
        thunderstore_community: "hollow-knight-silksong",
        steam_app_id: "1030300",
        steam_dirs: &["Hollow Knight Silksong", "Silksong"],
        executables: &["Hollow Knight Silksong.exe", "Silksong.exe"],
        mod_dir: &["BepInEx", "plugins"],
    },
    ThunderstoreGameSpec {
        game_id: "riskofrain2",
        display_name: "Risk of Rain 2",
        nexus_slug: "riskofrain2",
        thunderstore_community: "riskofrain2",
        steam_app_id: "632360",
        steam_dirs: &["Risk of Rain 2"],
        executables: &["Risk of Rain 2.exe"],
        mod_dir: &["BepInEx", "plugins"],
    },
    ThunderstoreGameSpec {
        game_id: "lethalcompany",
        display_name: "Lethal Company",
        nexus_slug: "lethalcompany",
        thunderstore_community: "lethal-company",
        steam_app_id: "1966720",
        steam_dirs: &["Lethal Company"],
        executables: &["Lethal Company.exe"],
        mod_dir: &["BepInEx", "plugins"],
    },
    ThunderstoreGameSpec {
        game_id: "contentwarning",
        display_name: "Content Warning",
        nexus_slug: "contentwarning",
        thunderstore_community: "content-warning",
        steam_app_id: "2881650",
        steam_dirs: &["Content Warning"],
        executables: &["Content Warning.exe"],
        mod_dir: &["BepInEx", "plugins"],
    },
    ThunderstoreGameSpec {
        game_id: "repo",
        display_name: "R.E.P.O.",
        nexus_slug: "repo",
        thunderstore_community: "repo",
        steam_app_id: "3241660",
        steam_dirs: &["REPO", "R.E.P.O."],
        executables: &["REPO.exe", "R.E.P.O..exe"],
        mod_dir: &["BepInEx", "plugins"],
    },
    ThunderstoreGameSpec {
        game_id: "palworld",
        display_name: "Palworld",
        nexus_slug: "palworld",
        thunderstore_community: "palworld",
        steam_app_id: "1623730",
        steam_dirs: &["Palworld"],
        executables: &["Palworld.exe", "Palworld-Win64-Shipping.exe"],
        // Palworld uses UE5, not BepInEx; mods land at game root for UE loaders.
        mod_dir: &[],
    },
    ThunderstoreGameSpec {
        game_id: "valheim",
        display_name: "Valheim",
        nexus_slug: "valheim",
        thunderstore_community: "valheim",
        steam_app_id: "892970",
        steam_dirs: &["Valheim"],
        executables: &["valheim.exe"],
        mod_dir: &["BepInEx", "plugins"],
    },
];

// ---------------------------------------------------------------------------
// Generic plugin impl
// ---------------------------------------------------------------------------

pub struct ThunderstorePlugin {
    spec: &'static ThunderstoreGameSpec,
}

impl ThunderstorePlugin {
    pub fn new(spec: &'static ThunderstoreGameSpec) -> Self {
        Self { spec }
    }

    /// Return the Thunderstore community id for this game. Useful for the
    /// frontend to fetch the catalog without hardcoding a map.
    pub fn thunderstore_community(&self) -> &'static str {
        self.spec.thunderstore_community
    }
}

impl GamePlugin for ThunderstorePlugin {
    fn game_id(&self) -> &str {
        self.spec.game_id
    }
    fn display_name(&self) -> &str {
        self.spec.display_name
    }
    fn nexus_slug(&self) -> &str {
        self.spec.nexus_slug
    }
    fn executables(&self) -> &[&str] {
        self.spec.executables
    }

    fn detect(&self, bottle: &Bottle) -> Option<DetectedGame> {
        let game_path = find_game_path(bottle, self.spec)?;
        if find_executable(&game_path, self.spec.executables).is_none() {
            return None;
        }
        let exe_path = find_executable(&game_path, self.spec.executables);
        let data_dir = self.get_data_dir(&game_path);
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
        let mut d = game_path.to_path_buf();
        for part in self.spec.mod_dir {
            d = d.join(part);
        }
        d
    }

    fn get_plugins_file(&self, _game_path: &Path, _bottle: &Bottle) -> Option<PathBuf> {
        None
    }

    fn steam_launch_id(&self) -> Option<&str> {
        Some(self.spec.steam_app_id)
    }

    fn categorize_mod_file(&self, rel_path: &str) -> Option<String> {
        let lower = rel_path.to_lowercase();
        if lower.ends_with(".dll") {
            return Some("plugin".into());
        }
        if lower.ends_with(".pak") {
            return Some("pak".into());
        }
        if lower.ends_with(".json") || lower.ends_with(".cfg") || lower.ends_with(".yml") {
            return Some("config".into());
        }
        if lower.ends_with(".png") || lower.ends_with(".dds") || lower.contains("/textures/") {
            return Some("texture".into());
        }
        None
    }

    fn protected_root_extensions(&self) -> Vec<&str> {
        vec![".exe", ".dll", ".pak"]
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub fn register_all() {
    for spec in SPECS {
        crate::games::register_plugin(Box::new(ThunderstorePlugin::new(spec)));
    }
}

/// Return all game_ids that follow the Thunderstore/BepInEx pattern.
/// Frontend uses this to gate the Load Order page.
pub fn game_ids() -> Vec<&'static str> {
    SPECS.iter().map(|s| s.game_id).collect()
}

/// Look up the Thunderstore community id for a registered game_id.
pub fn community_for(game_id: &str) -> Option<&'static str> {
    SPECS
        .iter()
        .find(|s| s.game_id == game_id)
        .map(|s| s.thunderstore_community)
}

// ---------------------------------------------------------------------------
// Detection helpers (shared across all Thunderstore games)
// ---------------------------------------------------------------------------

fn find_game_path(bottle: &Bottle, spec: &ThunderstoreGameSpec) -> Option<PathBuf> {
    if let Some(p) = check_steam_default(bottle, spec) {
        return Some(p);
    }
    check_steam_library_folders(bottle, spec)
}

fn check_steam_default(bottle: &Bottle, spec: &ThunderstoreGameSpec) -> Option<PathBuf> {
    let common = bottle.find_path(STEAM_COMMON)?;
    for name in spec.steam_dirs {
        if let Some(dir) = find_child_ci(&common, name) {
            if dir.is_dir() {
                return Some(dir);
            }
        }
    }
    None
}

fn check_steam_library_folders(bottle: &Bottle, spec: &ThunderstoreGameSpec) -> Option<PathBuf> {
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
        for name in spec.steam_dirs {
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
    let lower: Vec<String> = exes.iter().map(|e| e.to_lowercase()).collect();
    let mut found: Option<PathBuf> = None;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if let Some(idx) = lower.iter().position(|e| e == &name) {
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

fn find_child_ci(parent: &Path, target: &str) -> Option<PathBuf> {
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

    fn silksong_spec() -> &'static ThunderstoreGameSpec {
        SPECS.iter().find(|s| s.game_id == "silksong").unwrap()
    }

    #[test]
    fn every_spec_has_valid_fields() {
        for s in SPECS {
            assert!(!s.game_id.is_empty(), "game_id empty in {:?}", s.display_name);
            assert!(!s.thunderstore_community.is_empty(), "community empty");
            assert!(!s.steam_app_id.is_empty(), "steam id empty");
            assert!(!s.steam_dirs.is_empty(), "steam_dirs empty");
            assert!(!s.executables.is_empty(), "executables empty");
        }
    }

    #[test]
    fn no_duplicate_game_ids() {
        let mut ids: Vec<&str> = SPECS.iter().map(|s| s.game_id).collect();
        ids.sort();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len());
    }

    #[test]
    fn silksong_plugin_metadata() {
        let p = ThunderstorePlugin::new(silksong_spec());
        assert_eq!(p.game_id(), "silksong");
        assert_eq!(p.display_name(), "Hollow Knight: Silksong");
        assert_eq!(p.thunderstore_community(), "hollow-knight-silksong");
        assert_eq!(p.steam_launch_id(), Some("1030300"));
    }

    #[test]
    fn data_dir_uses_mod_dir_components() {
        let p = ThunderstorePlugin::new(silksong_spec());
        let got = p.get_data_dir(&PathBuf::from("/fake/Silksong"));
        assert_eq!(got, PathBuf::from("/fake/Silksong/BepInEx/plugins"));
    }

    #[test]
    fn data_dir_empty_mod_dir_stays_root() {
        // Palworld: empty mod_dir → data_dir = game root
        let spec = SPECS.iter().find(|s| s.game_id == "palworld").unwrap();
        let p = ThunderstorePlugin::new(spec);
        let got = p.get_data_dir(&PathBuf::from("/fake/Palworld"));
        assert_eq!(got, PathBuf::from("/fake/Palworld"));
    }

    #[test]
    fn no_plugins_file_for_any_spec() {
        let b = Bottle {
            name: "T".into(),
            path: PathBuf::from("/tmp"),
            source: "T".into(),
        };
        for spec in SPECS {
            let p = ThunderstorePlugin::new(spec);
            assert!(
                p.get_plugins_file(&PathBuf::from("/fake"), &b).is_none(),
                "{} should have no plugin file",
                spec.game_id
            );
        }
    }

    #[test]
    fn community_for_lookup() {
        assert_eq!(community_for("silksong"), Some("hollow-knight-silksong"));
        assert_eq!(community_for("lethalcompany"), Some("lethal-company"));
        assert_eq!(community_for("nonexistent"), None);
    }

    #[test]
    fn game_ids_all_present() {
        let ids = game_ids();
        assert!(ids.contains(&"silksong"));
        assert!(ids.contains(&"riskofrain2"));
        assert!(ids.contains(&"lethalcompany"));
        assert!(ids.contains(&"contentwarning"));
    }

    #[test]
    fn detect_silksong_in_steam_common() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("Bottle");
        let game_dir = bottle_path
            .join("drive_c")
            .join("Program Files (x86)")
            .join("Steam")
            .join("steamapps")
            .join("common")
            .join("Hollow Knight Silksong");
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join("Hollow Knight Silksong.exe"), b"fake").unwrap();

        let b = Bottle {
            name: "T".into(),
            path: bottle_path,
            source: "T".into(),
        };
        let p = ThunderstorePlugin::new(silksong_spec());
        let d = p.detect(&b).expect("detection");
        assert_eq!(d.game_id, "silksong");
        assert_eq!(
            d.data_dir,
            game_dir.join("BepInEx").join("plugins")
        );
    }

    #[test]
    fn categorize_mod_file_buckets() {
        let p = ThunderstorePlugin::new(silksong_spec());
        assert_eq!(p.categorize_mod_file("x/MyMod.dll"), Some("plugin".into()));
        assert_eq!(p.categorize_mod_file("x/data.cfg"), Some("config".into()));
        assert_eq!(p.categorize_mod_file("x/textures/foo.png"), Some("texture".into()));
        assert_eq!(p.categorize_mod_file("x/readme.md"), None);
    }
}
