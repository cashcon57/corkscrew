//! FromSoftware games (Mod Engine 2) — batch-registered via a spec table.
//!
//! All FromSoft titles supported here share the same modding architecture:
//! - **Mod Engine 2** loader (a launcher-side DLL injector that points the game
//!   at a `mod/<modname>/` overlay)
//! - Per-mod folder convention `<game>/mod/<modname>/`
//! - EAC anti-cheat for online play; Mod Engine 2 disables it for offline use
//! - No ESP/ESL/ESM stack → Load Order page hidden
//! - Steam as primary distribution (DS:R / AC6 also on GOG)
//! - Nexus Mods as canonical mod catalog
//!
//! Mirrors the parameterized-spec pattern in [`thunderstore_games`] — one
//! generic plugin keyed by a spec, registered N times. Adding a FromSoft
//! game = add one row to [`SPECS`].
//!
//! The Mod Engine 2 mod-type entry that routes archives into per-mod
//! subdirectories lives in `mod_types.rs` (handled separately). This module
//! exposes the data dir as `<game>/mod/` and opts out of legacy data-dir
//! merging via [`use_legacy_data_dir`] returning `false`.
//!
//! [`thunderstore_games`]: crate::plugins::thunderstore_games
//! [`use_legacy_data_dir`]: crate::games::GamePlugin::use_legacy_data_dir

use std::fs;
use std::path::{Path, PathBuf};

use crate::bottles::Bottle;
use crate::games::{DetectedGame, GamePlugin};
use crate::runtime::{GameRuntime, WineContext};

const STEAM_COMMON: &[&str] = &["Program Files (x86)", "Steam", "steamapps", "common"];

/// Generic GOG installation roots scanned for FromSoft games. Sekiro and
/// Elden Ring aren't on GOG, but the scan is harmless — it only matches
/// existing directories that also contain the spec's executable.
const GOG_ROOTS: &[&[&str]] = &[
    &["GOG Games"],
    &["Program Files", "GOG Galaxy", "Games"],
    &["Program Files (x86)", "GOG Galaxy", "Games"],
];

/// Static spec for one FromSoftware game. Adding a game = add a row here.
pub struct FromSoftGameSpec {
    pub game_id: &'static str,
    pub display_name: &'static str,
    /// Slug used with the NexusMods API.
    pub nexus_slug: &'static str,
    /// Steam AppID (for `steam://` URL launch).
    pub steam_app_id: &'static str,
    /// Steam common-directory names to scan (case-insensitive).
    pub steam_dirs: &'static [&'static str],
    /// Known executable names inside the game directory (case-insensitive).
    pub executables: &'static [&'static str],
    /// Mod folder relative to the game root. Mod Engine 2 expects mods at
    /// `<game>/mod/<modname>/`. We expose `<game>/mod/` as the data dir
    /// (Phase 1).
    pub mod_dir: &'static [&'static str],
    /// Whether this game is verified working under our test bottles. Marks
    /// the game as Experimental in the support-tier UI when false.
    pub verified: bool,
}

pub const SPECS: &[FromSoftGameSpec] = &[
    FromSoftGameSpec {
        game_id: "sekiro",
        display_name: "Sekiro: Shadows Die Twice",
        nexus_slug: "sekiro",
        steam_app_id: "814380",
        steam_dirs: &["SEKIRO Shadows Die Twice", "Sekiro Shadows Die Twice"],
        executables: &["sekiro.exe"],
        mod_dir: &["mod"],
        verified: false,
    },
    FromSoftGameSpec {
        game_id: "eldenring",
        display_name: "Elden Ring",
        nexus_slug: "eldenring",
        steam_app_id: "1245620",
        steam_dirs: &["ELDEN RING", "Elden Ring"],
        executables: &["eldenring.exe", "start_protected_game.exe"],
        mod_dir: &["mod"],
        verified: false,
    },
    FromSoftGameSpec {
        game_id: "darksouls3",
        display_name: "Dark Souls III",
        nexus_slug: "darksouls3",
        steam_app_id: "374320",
        steam_dirs: &["DARK SOULS III"],
        executables: &["DarkSoulsIII.exe"],
        mod_dir: &["mod"],
        verified: false,
    },
    FromSoftGameSpec {
        game_id: "darksouls_remastered",
        display_name: "Dark Souls: Remastered",
        nexus_slug: "darksoulsremastered",
        steam_app_id: "570940",
        steam_dirs: &["DARK SOULS REMASTERED"],
        executables: &["DarkSoulsRemastered.exe"],
        mod_dir: &["mod"],
        verified: false,
    },
    FromSoftGameSpec {
        game_id: "armoredcore6",
        display_name: "Armored Core VI: Fires of Rubicon",
        nexus_slug: "armoredcore6firesofrubicon",
        steam_app_id: "1888160",
        steam_dirs: &["ARMORED CORE VI FIRES OF RUBICON"],
        executables: &["armoredcore6.exe", "start_protected_game.exe"],
        mod_dir: &["mod"],
        verified: false,
    },
];

// ---------------------------------------------------------------------------
// Generic plugin impl
// ---------------------------------------------------------------------------

pub struct FromSoftPlugin {
    spec: &'static FromSoftGameSpec,
}

impl FromSoftPlugin {
    pub fn new(spec: &'static FromSoftGameSpec) -> Self {
        Self { spec }
    }

    /// Whether this game has been end-to-end tested under a real bottle.
    /// Frontend can use this to mark the game as Experimental.
    pub fn verified(&self) -> bool {
        self.spec.verified
    }
}

impl GamePlugin for FromSoftPlugin {
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

    fn detect_wine(&self, bottle: &Bottle) -> Option<DetectedGame> {
        let game_path = find_game_path(bottle, self.spec)?;
        let exe_path = find_executable(&game_path, self.spec.executables)?;

        // Mod Engine 2 won't load mods if `<game>/mod/` doesn't exist on first
        // install. Auto-create on detection so a fresh game gets a usable
        // staging dir without the user having to know to mkdir manually.
        let data_dir = find_or_create_modengine_mod_dir(&game_path);

        Some(DetectedGame {
            game_id: self.game_id().to_string(),
            display_name: self.display_name().to_string(),
            nexus_slug: self.nexus_slug().to_string(),
            game_path,
            exe_path: Some(exe_path),
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

    /// Route through `mod_types` so Mod Engine 2 archives land at
    /// `<game>/mod/<modname>/` instead of merging into `<game>/mod/`.
    fn use_legacy_data_dir(&self) -> bool {
        false
    }

    fn categorize_mod_file(&self, rel_path: &str) -> Option<String> {
        // Normalize separators: Wine archives may ship with `\` paths.
        let lower = rel_path.replace('\\', "/").to_lowercase();
        // FromSoft archive containers (binders) and DCX-compressed assets.
        if lower.ends_with(".bnd") || lower.ends_with(".dcx") {
            return Some("data".into());
        }
        if lower.ends_with(".dll") {
            return Some("library".into());
        }
        if lower.ends_with(".ini") {
            return Some("config".into());
        }
        None
    }

    fn protected_root_extensions(&self) -> Vec<&str> {
        // Vanilla game artifacts that must never be deleted from the game
        // root by the cleaner. `.bdt`/`.bhd` are FromSoft's split-archive
        // header/data pair; `.regulation` covers regulation.bin and friends.
        vec![".exe", ".dll", ".bdt", ".bhd", ".regulation"]
    }

    fn critical_files(&self) -> Vec<&str> {
        // Per-spec critical files: at minimum every spec exe + the master
        // gameplay archive (regulation.bin) that Mod Engine 2 patches over.
        let mut v: Vec<&str> = self.spec.executables.to_vec();
        v.push("regulation.bin");
        v
    }
}

// ---------------------------------------------------------------------------
// Public helpers
// ---------------------------------------------------------------------------

/// Resolve `<game_path>/mod` and create it if missing. Idempotent.
///
/// Mod Engine 2 expects this directory to exist before it will load any
/// mods, and a fresh install of the game has no `mod/` folder. We create
/// it on detection so the user never sees an "install failed because
/// directory is missing" path.
pub fn find_or_create_modengine_mod_dir(game_path: &Path) -> PathBuf {
    let dir = game_path.join("mod");
    if !dir.exists() {
        // Best-effort creation. If this fails (read-only volume, perms),
        // the install pipeline will surface a clearer error later — there's
        // nothing useful we can do here, and we still want to return the
        // canonical path so callers can show it in the UI.
        let _ = fs::create_dir_all(&dir);
    }
    dir
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub fn register_all() {
    for spec in SPECS {
        crate::games::register_plugin(Box::new(FromSoftPlugin::new(spec)));
    }
}

/// Return all game_ids that follow the FromSoft / Mod Engine 2 pattern.
/// Frontend uses this to gate the Load Order page.
pub fn game_ids() -> Vec<&'static str> {
    SPECS.iter().map(|s| s.game_id).collect()
}

// ---------------------------------------------------------------------------
// Detection helpers (shared across all FromSoft games)
// ---------------------------------------------------------------------------

fn find_game_path(bottle: &Bottle, spec: &FromSoftGameSpec) -> Option<PathBuf> {
    if let Some(p) = check_steam_default(bottle, spec) {
        return Some(p);
    }
    if let Some(p) = check_steam_library_folders(bottle, spec) {
        return Some(p);
    }
    check_gog_paths(bottle, spec)
}

fn check_steam_default(bottle: &Bottle, spec: &FromSoftGameSpec) -> Option<PathBuf> {
    let common = bottle.find_path(STEAM_COMMON)?;
    for name in spec.steam_dirs {
        if let Some(dir) = find_child_ci(&common, name) {
            if dir.is_dir() && find_executable(&dir, spec.executables).is_some() {
                return Some(dir);
            }
            // For ER / AC6, the launcher exe lives in `Game/` subdir alongside
            // the start_protected_game.exe at game root. The first match
            // (root) succeeds when `start_protected_game.exe` is in our spec,
            // but if a publisher reshuffles, also probe a `Game/` child.
            if dir.is_dir() {
                let sub = dir.join("Game");
                if sub.is_dir() && find_executable(&sub, spec.executables).is_some() {
                    return Some(sub);
                }
                return Some(dir);
            }
        }
    }
    None
}

fn check_steam_library_folders(bottle: &Bottle, spec: &FromSoftGameSpec) -> Option<PathBuf> {
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
                if dir.is_dir() && find_executable(&dir, spec.executables).is_some() {
                    return Some(dir);
                }
                if dir.is_dir() {
                    let sub = dir.join("Game");
                    if sub.is_dir() && find_executable(&sub, spec.executables).is_some() {
                        return Some(sub);
                    }
                }
            }
        }
    }
    None
}

fn check_gog_paths(bottle: &Bottle, spec: &FromSoftGameSpec) -> Option<PathBuf> {
    for root in GOG_ROOTS {
        let Some(root_path) = bottle.find_path(root) else {
            continue;
        };
        for name in spec.steam_dirs {
            if let Some(dir) = find_child_ci(&root_path, name) {
                if dir.is_dir() && find_executable(&dir, spec.executables).is_some() {
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
    // Prefer earlier entries in `exes` over later ones (so the real game
    // exe wins over `start_protected_game.exe` when both are present).
    let mut best: Option<(usize, PathBuf)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
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

    fn spec_for(id: &str) -> &'static FromSoftGameSpec {
        SPECS.iter().find(|s| s.game_id == id).expect("spec exists")
    }

    fn make_bottle(path: PathBuf) -> Bottle {
        Bottle {
            name: "T".into(),
            path,
            source: "T".into(),
        }
    }

    #[test]
    fn every_spec_has_valid_fields() {
        for s in SPECS {
            assert!(!s.game_id.is_empty(), "game_id empty in {:?}", s.display_name);
            assert!(!s.display_name.is_empty(), "display_name empty");
            assert!(!s.nexus_slug.is_empty(), "nexus_slug empty");
            assert!(!s.steam_app_id.is_empty(), "steam id empty");
            assert!(!s.steam_dirs.is_empty(), "steam_dirs empty");
            assert!(!s.executables.is_empty(), "executables empty");
            assert!(!s.mod_dir.is_empty(), "mod_dir empty");
        }
    }

    #[test]
    fn no_duplicate_game_ids() {
        let mut ids: Vec<&str> = SPECS.iter().map(|s| s.game_id).collect();
        ids.sort();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "duplicate game_id in SPECS");
    }

    #[test]
    fn no_duplicate_steam_app_ids() {
        let mut ids: Vec<&str> = SPECS.iter().map(|s| s.steam_app_id).collect();
        ids.sort();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "duplicate steam_app_id in SPECS");
    }

    #[test]
    fn all_specs_unverified_until_real_bottle_test() {
        // Sanity check on the support-tier intent: nothing is `verified` until
        // a human actually plays the modded game end-to-end. If you flip a
        // spec to `verified: true`, update this test to allow it.
        for s in SPECS {
            assert!(
                !s.verified,
                "{} marked verified — update test if intentional",
                s.game_id
            );
        }
    }

    #[test]
    fn plugin_metadata_round_trip_per_spec() {
        for spec in SPECS {
            let p = FromSoftPlugin::new(spec);
            assert_eq!(p.game_id(), spec.game_id);
            assert_eq!(p.display_name(), spec.display_name);
            assert_eq!(p.nexus_slug(), spec.nexus_slug);
            assert_eq!(p.executables(), spec.executables);
            assert_eq!(p.steam_launch_id(), Some(spec.steam_app_id));
            assert_eq!(p.verified(), spec.verified);
            assert!(!p.use_legacy_data_dir(), "{} must opt out of legacy data dir", spec.game_id);
        }
    }

    #[test]
    fn data_dir_resolves_to_mod_subdir() {
        for spec in SPECS {
            let p = FromSoftPlugin::new(spec);
            let got = p.get_data_dir(&PathBuf::from("/fake/Game"));
            assert_eq!(
                got,
                PathBuf::from("/fake/Game/mod"),
                "{} data_dir mismatch",
                spec.game_id
            );
        }
    }

    #[test]
    fn no_plugins_file_for_any_spec() {
        let b = make_bottle(PathBuf::from("/tmp"));
        for spec in SPECS {
            let p = FromSoftPlugin::new(spec);
            assert!(
                p.get_plugins_file(&PathBuf::from("/fake"), &b).is_none(),
                "{} should have no plugin file",
                spec.game_id
            );
        }
    }

    #[test]
    fn protected_root_extensions_cover_fromsoft_archives() {
        let p = FromSoftPlugin::new(spec_for("eldenring"));
        let exts = p.protected_root_extensions();
        for needed in [".exe", ".dll", ".bdt", ".bhd", ".regulation"] {
            assert!(exts.contains(&needed), "missing protected ext {}", needed);
        }
    }

    #[test]
    fn critical_files_include_exes_and_regulation() {
        for spec in SPECS {
            let p = FromSoftPlugin::new(spec);
            let crit = p.critical_files();
            assert!(crit.contains(&"regulation.bin"), "{} missing regulation.bin", spec.game_id);
            for exe in spec.executables {
                assert!(
                    crit.contains(exe),
                    "{} critical_files missing exe {}",
                    spec.game_id,
                    exe
                );
            }
        }
    }

    #[test]
    fn categorize_mod_file_buckets() {
        let p = FromSoftPlugin::new(spec_for("eldenring"));
        assert_eq!(p.categorize_mod_file("parts/foo.bnd"), Some("data".into()));
        assert_eq!(p.categorize_mod_file("chr/c0000.dcx"), Some("data".into()));
        assert_eq!(p.categorize_mod_file("modengine2.dll"), Some("library".into()));
        assert_eq!(p.categorize_mod_file("config\\engine.ini"), Some("config".into()));
        assert_eq!(p.categorize_mod_file("readme.md"), None);
    }

    #[test]
    fn find_or_create_modengine_mod_dir_creates_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().join("Sekiro");
        fs::create_dir_all(&game).unwrap();
        assert!(!game.join("mod").exists());

        let got = find_or_create_modengine_mod_dir(&game);
        assert_eq!(got, game.join("mod"));
        assert!(got.is_dir(), "mod dir should have been created");
    }

    #[test]
    fn find_or_create_modengine_mod_dir_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().join("EldenRing");
        let mod_dir = game.join("mod");
        fs::create_dir_all(&mod_dir).unwrap();
        // Drop a sentinel file so we can verify the dir wasn't recreated.
        fs::write(mod_dir.join("sentinel"), b"x").unwrap();

        let got = find_or_create_modengine_mod_dir(&game);
        assert_eq!(got, mod_dir);
        assert!(got.is_dir());
        assert!(mod_dir.join("sentinel").exists(), "existing dir was clobbered");
    }

    fn make_steam_game(bottle_path: &Path, steam_dir: &str, exe: &str) -> PathBuf {
        let game_dir = bottle_path
            .join("drive_c")
            .join("Program Files (x86)")
            .join("Steam")
            .join("steamapps")
            .join("common")
            .join(steam_dir);
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join(exe), b"fake").unwrap();
        game_dir
    }

    #[test]
    fn detect_sekiro_in_steam_common() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("Bottle");
        let game_dir = make_steam_game(&bottle_path, "SEKIRO Shadows Die Twice", "sekiro.exe");

        let b = make_bottle(bottle_path);
        let p = FromSoftPlugin::new(spec_for("sekiro"));
        let d = p.detect_wine(&b).expect("detection");
        assert_eq!(d.game_id, "sekiro");
        assert_eq!(d.game_path, game_dir);
        assert_eq!(d.data_dir, game_dir.join("mod"));
        // detect() must auto-create the mod folder.
        assert!(game_dir.join("mod").is_dir());
    }

    #[test]
    fn detect_eldenring_picks_real_exe_over_protected_launcher() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("Bottle");
        let game_dir = make_steam_game(&bottle_path, "ELDEN RING", "eldenring.exe");
        // ER ships both real exe + start_protected_game.exe; we should pick
        // the earlier-listed `eldenring.exe`.
        fs::write(game_dir.join("start_protected_game.exe"), b"fake").unwrap();

        let b = make_bottle(bottle_path);
        let p = FromSoftPlugin::new(spec_for("eldenring"));
        let d = p.detect_wine(&b).expect("detection");
        assert_eq!(d.game_id, "eldenring");
        assert_eq!(
            d.exe_path.as_deref(),
            Some(game_dir.join("eldenring.exe").as_path())
        );
    }

    #[test]
    fn detect_dark_souls_3() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("Bottle");
        let _ = make_steam_game(&bottle_path, "DARK SOULS III", "DarkSoulsIII.exe");

        let b = make_bottle(bottle_path);
        let p = FromSoftPlugin::new(spec_for("darksouls3"));
        assert!(p.detect_wine(&b).is_some());
    }

    #[test]
    fn detect_dark_souls_remastered() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("Bottle");
        let _ = make_steam_game(
            &bottle_path,
            "DARK SOULS REMASTERED",
            "DarkSoulsRemastered.exe",
        );

        let b = make_bottle(bottle_path);
        let p = FromSoftPlugin::new(spec_for("darksouls_remastered"));
        assert!(p.detect_wine(&b).is_some());
    }

    #[test]
    fn detect_armored_core_6() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("Bottle");
        let _ = make_steam_game(
            &bottle_path,
            "ARMORED CORE VI FIRES OF RUBICON",
            "armoredcore6.exe",
        );

        let b = make_bottle(bottle_path);
        let p = FromSoftPlugin::new(spec_for("armoredcore6"));
        let d = p.detect_wine(&b).expect("detection");
        assert_eq!(d.nexus_slug, "armoredcore6firesofrubicon");
    }

    #[test]
    fn detect_returns_none_when_exe_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("Bottle");
        // Create dir but no exe.
        let game_dir = bottle_path
            .join("drive_c")
            .join("Program Files (x86)")
            .join("Steam")
            .join("steamapps")
            .join("common")
            .join("ELDEN RING");
        fs::create_dir_all(&game_dir).unwrap();

        let b = make_bottle(bottle_path);
        let p = FromSoftPlugin::new(spec_for("eldenring"));
        assert!(p.detect_wine(&b).is_none());
    }

    #[test]
    fn game_ids_all_present() {
        let ids = game_ids();
        assert!(ids.contains(&"sekiro"));
        assert!(ids.contains(&"eldenring"));
        assert!(ids.contains(&"darksouls3"));
        assert!(ids.contains(&"darksouls_remastered"));
        assert!(ids.contains(&"armoredcore6"));
        assert_eq!(ids.len(), SPECS.len());
    }
}
