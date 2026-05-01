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

use log::debug;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use crate::bottles::Bottle;
use crate::games::{DetectedGame, GamePlugin};
use crate::runtime::{GameRuntime, WineContext};
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

/// Additional non-Steam / non-Epic install paths. Each is anchored by the
/// `Phoenix/Binaries/Win64/HogwartsLegacy.exe` check to prevent false
/// positives from empty directories.
const NON_STEAM_PATHS: &[&[&str]] = &[
    &["Program Files (x86)", "Hogwarts Legacy"],
    &["Program Files", "Hogwarts Legacy"],
    &["Games", "Hogwarts Legacy"],
    // Top-level drag-drop convention.
    &["Hogwarts Legacy"],
    // GOG (Hogwarts Legacy is available on GOG).
    &["GOG Games", "Hogwarts Legacy"],
    &["Program Files", "GOG Galaxy", "Games", "Hogwarts Legacy"],
    &["Program Files (x86)", "GOG Galaxy", "Games", "Hogwarts Legacy"],
];

/// Xbox Game Pass install path.
const XBOX_GAMES_PATH: &[&str] = &["XboxGames", "Hogwarts Legacy", "Content"];

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

    fn detect_wine(&self, bottle: &Bottle) -> Option<DetectedGame> {
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
            runtime: GameRuntime::Wine(WineContext {
                bottle_name: bottle.name.clone(),
                bottle_path: bottle.path.clone(),
                source: bottle.source.clone(),
            }),
            steam_app_id: None,
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

    fn detect_mod_type_from_files(&self, files: &[String]) -> Option<String> {
        detect_hl_mod_type(files)
    }

    fn detect_game_version(&self, game_path: &Path) -> Option<String> {
        read_da_version(game_path)
    }

    fn on_mod_deployed(
        &self,
        game_path: &Path,
        mod_type: Option<&str>,
        _deployed_files: &[String],
    ) {
        if mod_type == Some("hl-lua-mod") {
            if let Err(e) = sync_mods_txt(game_path) {
                debug!("Failed to sync Mods.txt after deploy: {e}");
            }
        }
    }

    fn on_mod_undeployed(
        &self,
        game_path: &Path,
        mod_type: Option<&str>,
        undeployed_files: &[String],
    ) {
        // Sync Mods.txt on any undeploy if it might have contained Lua mods.
        // When mod_type is None (e.g. uninstall), check file list for Lua patterns.
        let is_lua = mod_type == Some("hl-lua-mod")
            || (mod_type.is_none()
                && undeployed_files
                    .iter()
                    .any(|f| f.to_lowercase().contains("main.lua")));
        if is_lua {
            if let Err(e) = sync_mods_txt(game_path) {
                debug!("Failed to sync Mods.txt after undeploy: {e}");
            }
        }
    }

    fn steam_launch_id(&self) -> Option<&str> {
        Some("990080")
    }

    fn launch_executable(&self, game_path: &std::path::Path) -> Option<std::path::PathBuf> {
        // Launch the ROOT launcher (not the Phoenix binary) — the root launcher
        // invokes Steam for DRM authentication before starting the real game.
        // Direct launch of the Phoenix binary fails because Steam isn't running.
        find_file_case_insensitive(game_path, "hogwartslegacy.exe")
    }

    fn critical_files(&self) -> Vec<&str> {
        // The ~mods data dir doesn't contain any vanilla files — all vanilla
        // content lives in non-mod PAK files outside this directory. Nothing
        // in the mod directory needs hard protection.
        vec![]
    }

    fn protected_root_extensions(&self) -> Vec<&str> {
        // No root-level extension protection needed — the data dir only
        // contains mod PAK files (vanilla PAKs are elsewhere).
        vec![]
    }

    fn save_file_patterns(&self) -> Vec<&str> {
        // HL saves are in AppData, not the game/data directory. Include .sav
        // just in case someone copies saves into the game tree.
        vec![".sav"]
    }

    fn categorize_mod_file(&self, rel_path: &str) -> Option<String> {
        let lower = rel_path.to_lowercase();

        if lower.ends_with(".pak") || lower.ends_with(".ucas") || lower.ends_with(".utoc") {
            return Some("pak".into());
        }
        if lower.ends_with(".lua") {
            return Some("script".into());
        }
        if lower.ends_with(".bk2") {
            return Some("movie".into());
        }
        if lower.ends_with(".dll") {
            return Some("framework".into());
        }
        if lower.ends_with(".ini") || lower.ends_with(".cfg") || lower.ends_with(".toml") {
            return Some("config".into());
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Mod type auto-detection
// ---------------------------------------------------------------------------

/// Known UE4SS/engine root DLLs and files.
const ENGINE_ROOT_FILES: &[&str] = &[
    "ue4ss.dll",
    "xinput1_3.dll",
    "dwmapi.dll",
    "reshade-shaders",
    "reshade.ini",
    "ue4ss-settings.ini",
    "d3d11.dll",
    "dxgi.dll",
];

/// Detect the Hogwarts Legacy mod type from a list of staged file paths.
///
/// Heuristics (checked in priority order):
/// 1. Any file matching `*/Scripts/main.lua` → Lua mod
/// 2. Any `.pak` in a `LogicMods` directory or marker files → Logic mod
/// 3. All files are `.bk2` → Movie mod
/// 4. Known engine root DLLs (ue4ss.dll, xinput1_3.dll, etc.) → Engine root
/// 5. Default → PAK mod
fn detect_hl_mod_type(files: &[String]) -> Option<String> {
    if files.is_empty() {
        return None;
    }

    let lower_files: Vec<String> = files.iter().map(|f| f.to_lowercase().replace('\\', "/")).collect();

    // 1. Lua mod: contains Scripts/main.lua pattern
    if lower_files.iter().any(|f| f.ends_with("/scripts/main.lua") || f == "scripts/main.lua") {
        return Some("hl-lua-mod".into());
    }

    // 2. Logic/Blueprint mod: .pak files in LogicMods or marker files
    let logic_markers = [".ue4sslogicmod", "ue4sslogicmod.info", ".logicmod"];
    if lower_files.iter().any(|f| {
        logic_markers.iter().any(|m| f.ends_with(m))
            || (f.contains("logicmods/") && f.ends_with(".pak"))
    }) {
        return Some("hl-logic-mod".into());
    }

    // 3. Movie mod: all content files are .bk2
    let content_files: Vec<&String> = lower_files
        .iter()
        .filter(|f| !f.ends_with('/'))
        .collect();
    if !content_files.is_empty() && content_files.iter().all(|f| f.ends_with(".bk2")) {
        return Some("hogwarts-modtype-movies".into());
    }

    // 4. Engine root: known DLLs/files
    if lower_files.iter().any(|f| {
        let basename = f.rsplit('/').next().unwrap_or(f);
        ENGINE_ROOT_FILES.iter().any(|root| *root == basename)
    }) {
        return Some("hl-engine-root".into());
    }

    // 5. Default: PAK mod (if any .pak files present)
    if lower_files.iter().any(|f| f.ends_with(".pak")) {
        return Some("hogwarts-PAK-modtype".into());
    }

    None
}

// ---------------------------------------------------------------------------
// Game version detection
// ---------------------------------------------------------------------------

/// Read the game version from `Phoenix/Content/Data/Version/DA_Version.txt`.
///
/// The file contains a single integer (e.g. `1233043`) representing the
/// build version. Returns `None` if the file doesn't exist or can't be read.
fn read_da_version(game_path: &Path) -> Option<String> {
    // Case-insensitive traversal for Wine/APFS
    let mut dir = game_path.to_path_buf();
    for component in &["Phoenix", "Content", "Data", "Version"] {
        dir = find_child_case_insensitive(&dir, component)?;
    }
    let version_file = find_file_case_insensitive(&dir, "DA_Version.txt")?;
    let content = fs::read_to_string(&version_file).ok()?;
    let version = content.trim().to_string();
    if version.is_empty() {
        None
    } else {
        Some(version)
    }
}

// ---------------------------------------------------------------------------
// Lua Mods.txt management
// ---------------------------------------------------------------------------

/// Path to the UE4SS Mods directory (relative components from game root).
const MODS_DIR_COMPONENTS: &[&str] = &["Phoenix", "Binaries", "Win64", "Mods"];

/// Reserved UE4SS mod directories that should not be toggled by the user.
/// These are part of UE4SS itself, not user Lua mods.
const UE4SS_BUILTINS: &[&str] = &[
    "shared",
    "bpmodloadmod",
    "bpmodloadermod",
    "consolecreatormod",
    "consoleenablermod",
    "cheatmanagerenablermod",
    "keybindmanager",
    "linetracemod",
    "jsbpmod",
    "objectdumpermod",
    "usettingsmod",
    "splashscreendumpermod",
];

/// Sync `Mods.txt` to match the currently deployed Lua mods.
///
/// Scans the `Phoenix/Binaries/Win64/Mods/` directory for mod folders
/// containing `Scripts/main.lua`. Each discovered mod is written as
/// `ModName : 1` (enabled). UE4SS built-in mod entries are preserved
/// with their existing enable/disable state.
///
/// This is resilient to:
/// - Repeated enable/disable cycles (regenerates from disk state)
/// - Missing Mods directory (creates if needed after UE4SS is installed)
/// - Corrupt/truncated Mods.txt (completely regenerated)
/// - Extra whitespace or inconsistent formatting in existing file
pub fn sync_mods_txt(game_path: &Path) -> Result<(), String> {
    let mods_dir = resolve_case_insensitive_path(game_path, MODS_DIR_COMPONENTS)
        .ok_or("UE4SS Mods directory not found — is UE4SS installed?")?;

    let mods_txt = mods_dir.join("Mods.txt");

    // 1. Read existing entries to preserve built-in mod states
    let mut builtin_states: Vec<(String, bool)> = Vec::new();
    if mods_txt.exists() {
        if let Ok(file) = fs::File::open(&mods_txt) {
            let reader = BufReader::new(file);
            for line in reader.lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(_) => continue,
                };
                if let Some((name, enabled)) = parse_mods_txt_line(&line) {
                    if UE4SS_BUILTINS.contains(&name.to_lowercase().as_str()) {
                        builtin_states.push((name, enabled));
                    }
                }
            }
        }
    }

    // 2. Scan for deployed user Lua mods (folders with Scripts/main.lua)
    let mut user_mods: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(&mods_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if UE4SS_BUILTINS.contains(&name.to_lowercase().as_str()) {
                continue;
            }
            // Check for Scripts/main.lua (case-insensitive)
            let mod_dir = entry.path();
            if !mod_dir.is_dir() {
                continue;
            }
            let has_main_lua = find_child_case_insensitive(&mod_dir, "Scripts")
                .and_then(|scripts_dir| find_file_case_insensitive(&scripts_dir, "main.lua"))
                .is_some();
            if has_main_lua {
                user_mods.push(name);
            }
        }
    }
    user_mods.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));

    // 3. Write Mods.txt — built-ins first (preserved state), then user mods (enabled)
    let mut output = String::new();
    output.push_str("; Managed by Corkscrew — do not edit while the mod manager is running.\n");
    output.push_str("; Built-in UE4SS mods:\n");
    for (name, enabled) in &builtin_states {
        output.push_str(&format!("{} : {}\n", name, if *enabled { 1 } else { 0 }));
    }
    if !user_mods.is_empty() {
        output.push_str("\n; User Lua mods:\n");
        for name in &user_mods {
            output.push_str(&format!("{} : 1\n", name));
        }
    }

    // 4. Atomic write via temp file + rename
    let tmp_path = mods_txt.with_extension("txt.tmp");
    let mut file = fs::File::create(&tmp_path).map_err(|e| format!("Failed to write Mods.txt: {e}"))?;
    file.write_all(output.as_bytes()).map_err(|e| format!("Failed to write Mods.txt: {e}"))?;
    file.sync_all().map_err(|e| format!("Sync failed: {e}"))?;
    fs::rename(&tmp_path, &mods_txt).map_err(|e| format!("Failed to rename Mods.txt: {e}"))?;

    debug!(
        "Synced Mods.txt: {} built-in entries, {} user mods",
        builtin_states.len(),
        user_mods.len()
    );
    Ok(())
}

/// Parse a `Mods.txt` line like `"ModName : 1"` or `"ModName : 0"`.
fn parse_mods_txt_line(line: &str) -> Option<(String, bool)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
        return None;
    }
    let parts: Vec<&str> = line.splitn(2, ':').collect();
    if parts.len() != 2 {
        return None;
    }
    let name = parts[0].trim().to_string();
    let enabled = parts[1].trim() == "1";
    if name.is_empty() {
        return None;
    }
    Some((name, enabled))
}

// ---------------------------------------------------------------------------
// Movie (.bk2) file matching
// ---------------------------------------------------------------------------

/// Scan the game's `Phoenix/Content/Movies/` tree and find the correct
/// subdirectory for each `.bk2` file by matching filenames.
///
/// Returns a map of `staged_relative_path -> correct_relative_deploy_path`
/// (relative to `Phoenix/Content`). Files that don't match any game directory
/// are mapped to `Movies/` as a fallback.
pub fn match_bk2_files(
    game_path: &Path,
    staged_files: &[String],
) -> Vec<(String, String)> {
    let bk2_files: Vec<&String> = staged_files
        .iter()
        .filter(|f| f.to_lowercase().ends_with(".bk2"))
        .collect();

    if bk2_files.is_empty() {
        return Vec::new();
    }

    // Build index of all .bk2 files in game's Movies directory tree
    let movies_dir = resolve_case_insensitive_path(
        game_path,
        &["Phoenix", "Content", "Movies"],
    );

    let mut game_bk2_index: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    if let Some(ref movies) = movies_dir {
        index_bk2_files(movies, movies, &mut game_bk2_index);
    }

    let mut mappings = Vec::new();
    for staged_file in bk2_files {
        let basename = staged_file
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(staged_file)
            .to_lowercase();

        if let Some(game_rel_path) = game_bk2_index.get(&basename) {
            // Found matching file in game — deploy to its directory
            mappings.push((staged_file.clone(), format!("Movies/{}", game_rel_path)));
        } else {
            // No match — deploy directly to Movies/
            let filename = staged_file.rsplit(['/', '\\']).next().unwrap_or(staged_file);
            mappings.push((staged_file.clone(), format!("Movies/{}", filename)));
        }
    }

    mappings
}

/// Recursively index all `.bk2` files under a directory.
/// Maps `lowercase_filename -> relative_path_from_root`.
fn index_bk2_files(
    dir: &Path,
    root: &Path,
    index: &mut std::collections::HashMap<String, String>,
) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            index_bk2_files(&path, root, index);
        } else if let Some(name) = path.file_name() {
            let name_str = name.to_string_lossy();
            if name_str.to_lowercase().ends_with(".bk2") {
                if let Ok(rel) = path.strip_prefix(root) {
                    index.insert(
                        name_str.to_lowercase(),
                        rel.to_string_lossy().replace('\\', "/"),
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Resolve a path with case-insensitive traversal for each component.
fn resolve_case_insensitive_path(base: &Path, components: &[&str]) -> Option<PathBuf> {
    let mut dir = base.to_path_buf();
    for component in components {
        dir = find_child_case_insensitive(&dir, component)?;
    }
    Some(dir)
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Register the Hogwarts Legacy plugin with the global game plugin registry.
pub fn register() {
    crate::games::register_plugin(std::sync::Arc::new(HogwartsLegacyPlugin));
}

// ---------------------------------------------------------------------------
// Detection helpers
// ---------------------------------------------------------------------------

/// Attempt to locate the Hogwarts Legacy installation directory inside a bottle.
///
/// Checks the default Steam common directory first, then parses
/// `libraryfolders.vdf` for additional Steam library paths, checks Epic Games
/// Store installation paths, and finally falls back to generic non-Steam
/// locations (GOG, manual installs, Game Pass, etc.).
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

    // 4. Generic non-Steam / GOG / manual install paths.
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

/// Parse `libraryfolders.vdf` and check each library for the game.
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

/// Check well-known Epic Games Store installation directories.
fn check_epic_paths(bottle: &Bottle) -> Option<PathBuf> {
    for parts in EPIC_PATHS {
        if let Some(path) = bottle.find_path(parts) {
            if path.is_dir() && has_real_executable(&path) {
                return Some(path);
            }
        }
    }
    None
}

/// Check generic non-Steam / non-Epic install paths (GOG, manual, drag-drop).
/// Every candidate is validated against `has_real_executable` to prevent
/// false-positives from empty directories.
fn check_non_steam_paths(bottle: &Bottle) -> Option<PathBuf> {
    for parts in NON_STEAM_PATHS {
        if let Some(path) = bottle.find_path(parts) {
            if path.is_dir() && has_real_executable(&path) {
                return Some(path);
            }
        }
    }
    None
}

/// Check Xbox Game Pass install location.
fn check_xbox_games_path(bottle: &Bottle) -> Option<PathBuf> {
    if let Some(path) = bottle.find_path(XBOX_GAMES_PATH) {
        if path.is_dir() && has_real_executable(&path) {
            return Some(path);
        }
    }
    None
}

/// Probe `<bottle>/drive_c/users/<user>/Documents/Games/Hogwarts Legacy/` and
/// `<bottle>/drive_c/users/<user>/Documents/Hogwarts Legacy/` for game installs.
///
/// CrossOver typically symlinks the bottle's `Documents` directory to the
/// host `~/Documents`, so games kept at `~/Documents/Games/<name>/` on the
/// macOS host are reachable through the bottle filesystem.
///
/// Validation requires the real executable in `Phoenix/Binaries/Win64/` to
/// prevent false-positives from empty directories (mirrors `has_real_executable`).
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
                if dir.is_dir() && has_real_executable(&dir) {
                    return Some(dir);
                }
            }
            // Broad fallback: scan every subdir of this root and check for the
            // game's executables. Catches non-standard folder names that won't
            // match STEAM_GAME_DIR.
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

// ---------------------------------------------------------------------------
// PakChunk Conflict Detection
// ---------------------------------------------------------------------------

/// A group of PAK files that share the same pakchunk number (conflict).
#[derive(Clone, Debug, serde::Serialize)]
pub struct PakChunkConflict {
    pub chunk_number: u32,
    pub files: Vec<String>,
}

/// Scan the `~mods` directory for PAK files with conflicting pakchunk numbers.
///
/// UE5 PAK files use naming like `pakchunk5-ModName_P.pak`. Two mods using
/// the same chunk number will conflict and crash the game.
pub fn scan_pakchunk_conflicts(mods_dir: &Path) -> Vec<PakChunkConflict> {
    let mut chunks: std::collections::HashMap<u32, Vec<String>> =
        std::collections::HashMap::new();

    let entries = match fs::read_dir(mods_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.to_lowercase().ends_with(".pak") {
            continue;
        }
        // Extract chunk number from "pakchunkN-..." pattern
        let lower = name.to_lowercase();
        if let Some(rest) = lower.strip_prefix("pakchunk") {
            if let Some(num_str) = rest.split(|c: char| !c.is_ascii_digit()).next() {
                if let Ok(num) = num_str.parse::<u32>() {
                    chunks.entry(num).or_default().push(name);
                }
            }
        }
    }

    // Only return groups with 2+ files (actual conflicts)
    let mut conflicts: Vec<PakChunkConflict> = chunks
        .into_iter()
        .filter(|(_, files)| files.len() > 1)
        .map(|(chunk_number, mut files)| {
            files.sort();
            PakChunkConflict {
                chunk_number,
                files,
            }
        })
        .collect();
    conflicts.sort_by_key(|c| c.chunk_number);
    conflicts
}

// ---------------------------------------------------------------------------
// UE4SS Detection
// ---------------------------------------------------------------------------

/// Check if RE-UE4SS is installed for Hogwarts Legacy.
pub fn is_ue4ss_installed(game_path: &Path) -> bool {
    let win64 = game_path.join("Phoenix").join("Binaries").join("Win64");
    // Check for any of the common UE4SS files
    for name in &["UE4SS.dll", "xinput1_3.dll", "dwmapi.dll"] {
        if crate::skse::find_file_case_insensitive(&win64, name).is_some() {
            return true;
        }
    }
    false
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
        assert!(plugin.detect_wine(&bottle).is_none());
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
        let detected = plugin.detect_wine(&bottle);
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
        let detected = plugin.detect_wine(&bottle);
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
        assert!(plugin.detect_wine(&bottle).is_none());
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
        let detected = plugin.detect_wine(&bottle).unwrap();

        // exe_path should point to the real exe in Phoenix/Binaries/Win64/.
        let exe_path = detected.exe_path.unwrap();
        assert!(
            exe_path.to_string_lossy().contains("Phoenix"),
            "exe_path should be in Phoenix/Binaries/Win64, got: {}",
            exe_path.display()
        );
    }

    #[test]
    fn detect_finds_game_in_program_files() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = bottle_path
            .join("drive_c")
            .join("Program Files")
            .join("Hogwarts Legacy");
        create_hl_install(&bottle_path, &game_dir);

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        let plugin = HogwartsLegacyPlugin;
        let detected = plugin.detect_wine(&bottle).expect("Program Files detect");
        assert_eq!(detected.game_id, "hogwartslegacy");
        assert_eq!(detected.game_path, game_dir);
    }

    #[test]
    fn detect_finds_game_in_games_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = bottle_path
            .join("drive_c")
            .join("Games")
            .join("Hogwarts Legacy");
        create_hl_install(&bottle_path, &game_dir);

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        let plugin = HogwartsLegacyPlugin;
        assert!(plugin.detect_wine(&bottle).is_some());
    }

    #[test]
    fn detect_finds_game_at_drive_c_root() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = bottle_path.join("drive_c").join("Hogwarts Legacy");
        create_hl_install(&bottle_path, &game_dir);

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        let plugin = HogwartsLegacyPlugin;
        assert!(plugin.detect_wine(&bottle).is_some(), "top-level drag-drop");
    }

    #[test]
    fn detect_finds_game_in_xbox_games() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = bottle_path
            .join("drive_c")
            .join("XboxGames")
            .join("Hogwarts Legacy")
            .join("Content");
        create_hl_install(&bottle_path, &game_dir);

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        let plugin = HogwartsLegacyPlugin;
        let detected = plugin.detect_wine(&bottle).expect("Xbox Game Pass detect");
        assert_eq!(detected.game_id, "hogwartslegacy");
        assert!(detected.game_path.ends_with("Content"));
    }

    #[test]
    fn detect_non_steam_requires_phoenix_exe() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        let game_dir = bottle_path
            .join("drive_c")
            .join("Program Files (x86)")
            .join("Hogwarts Legacy");
        // Only create the directory, no exe.
        fs::create_dir_all(&game_dir).unwrap();
        fs::create_dir_all(bottle_path.join("drive_c")).unwrap();

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        let plugin = HogwartsLegacyPlugin;
        assert!(plugin.detect_wine(&bottle).is_none(), "empty dir must not detect");
    }

    #[test]
    fn detect_finds_game_in_documents_games() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().join("TestBottle");
        // Layout: <bottle>/drive_c/users/crossover/Documents/Games/Hogwarts Legacy/
        // with the real exe at Phoenix/Binaries/Win64/HogwartsLegacy.exe
        let game_dir = bottle_path
            .join("drive_c")
            .join("users")
            .join("crossover")
            .join("Documents")
            .join("Games")
            .join("Hogwarts Legacy");
        create_hl_install(&bottle_path, &game_dir);

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };
        let plugin = HogwartsLegacyPlugin;
        let detected = plugin
            .detect_wine(&bottle)
            .expect("should detect Documents/Games install");
        assert_eq!(detected.game_id, "hogwartslegacy");
        assert_eq!(detected.game_path, game_dir);
    }

    // --- Mod type auto-detection tests ---

    #[test]
    fn detect_lua_mod() {
        let files = vec![
            "MyMod/Scripts/main.lua".to_string(),
            "MyMod/Scripts/helper.lua".to_string(),
        ];
        assert_eq!(detect_hl_mod_type(&files), Some("hl-lua-mod".into()));
    }

    #[test]
    fn detect_lua_mod_case_insensitive() {
        let files = vec!["CoolMod/SCRIPTS/Main.lua".to_string()];
        assert_eq!(detect_hl_mod_type(&files), Some("hl-lua-mod".into()));
    }

    #[test]
    fn detect_logic_mod_by_marker() {
        let files = vec![
            "MyBPMod.pak".to_string(),
            ".ue4sslogicmod".to_string(),
        ];
        assert_eq!(detect_hl_mod_type(&files), Some("hl-logic-mod".into()));
    }

    #[test]
    fn detect_logic_mod_by_info_file() {
        let files = vec![
            "SomeMod/SomeMod.pak".to_string(),
            "SomeMod/ue4sslogicmod.info".to_string(),
        ];
        assert_eq!(detect_hl_mod_type(&files), Some("hl-logic-mod".into()));
    }

    #[test]
    fn detect_logic_mod_by_path() {
        let files = vec!["LogicMods/MyMod.pak".to_string()];
        assert_eq!(detect_hl_mod_type(&files), Some("hl-logic-mod".into()));
    }

    #[test]
    fn detect_movie_mod() {
        let files = vec![
            "intro_movie.bk2".to_string(),
            "outro_movie.bk2".to_string(),
        ];
        assert_eq!(
            detect_hl_mod_type(&files),
            Some("hogwarts-modtype-movies".into())
        );
    }

    #[test]
    fn detect_engine_root_ue4ss() {
        let files = vec![
            "UE4SS.dll".to_string(),
            "UE4SS-settings.ini".to_string(),
        ];
        assert_eq!(detect_hl_mod_type(&files), Some("hl-engine-root".into()));
    }

    #[test]
    fn detect_engine_root_reshade() {
        let files = vec![
            "dxgi.dll".to_string(),
            "reshade.ini".to_string(),
            "reshade-shaders".to_string(),
        ];
        assert_eq!(detect_hl_mod_type(&files), Some("hl-engine-root".into()));
    }

    #[test]
    fn detect_pak_mod() {
        let files = vec![
            "MyTextures_P.pak".to_string(),
            "MyTextures_P.utoc".to_string(),
        ];
        assert_eq!(
            detect_hl_mod_type(&files),
            Some("hogwarts-PAK-modtype".into())
        );
    }

    #[test]
    fn detect_empty_files() {
        assert_eq!(detect_hl_mod_type(&[]), None);
    }

    #[test]
    fn detect_unknown_files() {
        let files = vec!["readme.txt".to_string(), "config.json".to_string()];
        assert_eq!(detect_hl_mod_type(&files), None);
    }

    #[test]
    fn detect_mixed_bk2_and_pak_not_movie() {
        // Mixed archives with both .bk2 and .pak should NOT be classified as movie
        let files = vec![
            "intro.bk2".to_string(),
            "textures.pak".to_string(),
        ];
        // Should detect as PAK (bk2 are not ALL files)
        assert_eq!(
            detect_hl_mod_type(&files),
            Some("hogwarts-PAK-modtype".into())
        );
    }

    #[test]
    fn lua_takes_priority_over_pak() {
        // Lua mod with a pak file included
        let files = vec![
            "MyMod/Scripts/main.lua".to_string(),
            "MyMod/Data.pak".to_string(),
        ];
        assert_eq!(detect_hl_mod_type(&files), Some("hl-lua-mod".into()));
    }

    // --- Game version detection tests ---

    #[test]
    fn read_da_version_from_file() {
        let tmp = tempfile::tempdir().unwrap();
        let game_path = tmp.path();
        let version_dir = game_path
            .join("Phoenix")
            .join("Content")
            .join("Data")
            .join("Version");
        fs::create_dir_all(&version_dir).unwrap();
        fs::write(version_dir.join("DA_Version.txt"), "1233043\n").unwrap();

        assert_eq!(read_da_version(game_path), Some("1233043".into()));
    }

    #[test]
    fn read_da_version_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(read_da_version(tmp.path()), None);
    }

    #[test]
    fn read_da_version_empty_file() {
        let tmp = tempfile::tempdir().unwrap();
        let version_dir = tmp
            .path()
            .join("Phoenix")
            .join("Content")
            .join("Data")
            .join("Version");
        fs::create_dir_all(&version_dir).unwrap();
        fs::write(version_dir.join("DA_Version.txt"), "  \n").unwrap();

        assert_eq!(read_da_version(tmp.path()), None);
    }

    // --- Mods.txt management tests ---

    #[test]
    fn parse_mods_txt_line_enabled() {
        let (name, enabled) = parse_mods_txt_line("MyMod : 1").unwrap();
        assert_eq!(name, "MyMod");
        assert!(enabled);
    }

    #[test]
    fn parse_mods_txt_line_disabled() {
        let (name, enabled) = parse_mods_txt_line("SomeMod : 0").unwrap();
        assert_eq!(name, "SomeMod");
        assert!(!enabled);
    }

    #[test]
    fn parse_mods_txt_line_comment() {
        assert!(parse_mods_txt_line("; This is a comment").is_none());
        assert!(parse_mods_txt_line("# This too").is_none());
    }

    #[test]
    fn parse_mods_txt_line_empty() {
        assert!(parse_mods_txt_line("").is_none());
        assert!(parse_mods_txt_line("  ").is_none());
    }

    #[test]
    fn parse_mods_txt_line_extra_spaces() {
        let (name, enabled) = parse_mods_txt_line("  CoolMod  :  1  ").unwrap();
        assert_eq!(name, "CoolMod");
        assert!(enabled);
    }

    #[test]
    fn sync_mods_txt_creates_file() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp
            .path()
            .join("Phoenix")
            .join("Binaries")
            .join("Win64")
            .join("Mods");
        fs::create_dir_all(&mods_dir).unwrap();

        // Create a user Lua mod
        let mod_dir = mods_dir.join("TestLuaMod");
        fs::create_dir_all(mod_dir.join("Scripts")).unwrap();
        fs::write(mod_dir.join("Scripts").join("main.lua"), "-- test").unwrap();

        sync_mods_txt(tmp.path()).unwrap();

        let content = fs::read_to_string(mods_dir.join("Mods.txt")).unwrap();
        assert!(content.contains("TestLuaMod : 1"));
    }

    #[test]
    fn sync_mods_txt_preserves_builtins() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp
            .path()
            .join("Phoenix")
            .join("Binaries")
            .join("Win64")
            .join("Mods");
        fs::create_dir_all(&mods_dir).unwrap();

        // Write existing Mods.txt with a builtin disabled
        fs::write(
            mods_dir.join("Mods.txt"),
            "ConsoleEnablerMod : 0\nCheatManagerEnablerMod : 1\n",
        )
        .unwrap();

        sync_mods_txt(tmp.path()).unwrap();

        let content = fs::read_to_string(mods_dir.join("Mods.txt")).unwrap();
        assert!(content.contains("ConsoleEnablerMod : 0"));
        assert!(content.contains("CheatManagerEnablerMod : 1"));
    }

    #[test]
    fn sync_mods_txt_removes_undeployed_mods() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp
            .path()
            .join("Phoenix")
            .join("Binaries")
            .join("Win64")
            .join("Mods");
        fs::create_dir_all(&mods_dir).unwrap();

        // Create a mod, sync, then remove the mod and sync again
        let mod_dir = mods_dir.join("TempMod");
        fs::create_dir_all(mod_dir.join("Scripts")).unwrap();
        fs::write(mod_dir.join("Scripts").join("main.lua"), "-- temp").unwrap();

        sync_mods_txt(tmp.path()).unwrap();
        let content = fs::read_to_string(mods_dir.join("Mods.txt")).unwrap();
        assert!(content.contains("TempMod : 1"));

        // Remove the mod
        fs::remove_dir_all(&mod_dir).unwrap();

        // Sync again — TempMod should be gone
        sync_mods_txt(tmp.path()).unwrap();
        let content = fs::read_to_string(mods_dir.join("Mods.txt")).unwrap();
        assert!(!content.contains("TempMod"));
    }

    #[test]
    fn sync_mods_txt_no_ue4ss_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let result = sync_mods_txt(tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn sync_mods_txt_ignores_non_lua_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp
            .path()
            .join("Phoenix")
            .join("Binaries")
            .join("Win64")
            .join("Mods");
        fs::create_dir_all(&mods_dir).unwrap();

        // Create a directory without Scripts/main.lua
        fs::create_dir_all(mods_dir.join("NotALuaMod")).unwrap();
        fs::write(mods_dir.join("NotALuaMod").join("readme.txt"), "hi").unwrap();

        sync_mods_txt(tmp.path()).unwrap();

        let content = fs::read_to_string(mods_dir.join("Mods.txt")).unwrap();
        assert!(!content.contains("NotALuaMod"));
    }

    // --- Movie .bk2 matching tests ---

    #[test]
    fn match_bk2_finds_game_files() {
        let tmp = tempfile::tempdir().unwrap();
        let game_path = tmp.path();
        let movies_dir = game_path.join("Phoenix").join("Content").join("Movies");
        let sub_dir = movies_dir.join("Cinematics");
        fs::create_dir_all(&sub_dir).unwrap();
        fs::write(sub_dir.join("intro.bk2"), b"video").unwrap();

        let staged = vec!["intro.bk2".to_string()];
        let mappings = match_bk2_files(game_path, &staged);
        assert_eq!(mappings.len(), 1);
        assert_eq!(mappings[0].1, "Movies/Cinematics/intro.bk2");
    }

    #[test]
    fn match_bk2_fallback_to_movies_root() {
        let tmp = tempfile::tempdir().unwrap();
        let game_path = tmp.path();
        let movies_dir = game_path.join("Phoenix").join("Content").join("Movies");
        fs::create_dir_all(&movies_dir).unwrap();

        // No matching game file
        let staged = vec!["custom_video.bk2".to_string()];
        let mappings = match_bk2_files(game_path, &staged);
        assert_eq!(mappings.len(), 1);
        assert_eq!(mappings[0].1, "Movies/custom_video.bk2");
    }

    #[test]
    fn match_bk2_case_insensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let game_path = tmp.path();
        let movies_dir = game_path.join("Phoenix").join("Content").join("Movies");
        fs::create_dir_all(&movies_dir).unwrap();
        fs::write(movies_dir.join("Logo.bk2"), b"video").unwrap();

        let staged = vec!["logo.BK2".to_string()];
        let mappings = match_bk2_files(game_path, &staged);
        assert_eq!(mappings.len(), 1);
        assert_eq!(mappings[0].1, "Movies/Logo.bk2");
    }

    #[test]
    fn match_bk2_no_bk2_files() {
        let tmp = tempfile::tempdir().unwrap();
        let staged = vec!["texture.pak".to_string()];
        let mappings = match_bk2_files(tmp.path(), &staged);
        assert!(mappings.is_empty());
    }

    // --- Trait method tests ---

    #[test]
    fn detect_mod_type_via_trait() {
        let plugin = HogwartsLegacyPlugin;
        let files = vec!["MyMod/Scripts/main.lua".to_string()];
        assert_eq!(
            plugin.detect_mod_type_from_files(&files),
            Some("hl-lua-mod".into())
        );
    }

    #[test]
    fn detect_game_version_via_trait() {
        let tmp = tempfile::tempdir().unwrap();
        let version_dir = tmp
            .path()
            .join("Phoenix")
            .join("Content")
            .join("Data")
            .join("Version");
        fs::create_dir_all(&version_dir).unwrap();
        fs::write(version_dir.join("DA_Version.txt"), "1235957").unwrap();

        let plugin = HogwartsLegacyPlugin;
        assert_eq!(
            plugin.detect_game_version(tmp.path()),
            Some("1235957".into())
        );
    }

    #[test]
    fn test_pakchunk_conflict_detection() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp.path();

        // Create test files with conflicting chunk numbers
        fs::write(mods_dir.join("pakchunk5-ModA_P.pak"), b"fake").unwrap();
        fs::write(mods_dir.join("pakchunk5-ModB_P.pak"), b"fake").unwrap();
        fs::write(mods_dir.join("pakchunk10-ModC_P.pak"), b"fake").unwrap();
        fs::write(mods_dir.join("pakchunk20-ModD_P.pak"), b"fake").unwrap();
        fs::write(mods_dir.join("pakchunk20-ModE_P.pak"), b"fake").unwrap();
        fs::write(mods_dir.join("pakchunk20-ModF_P.pak"), b"fake").unwrap();
        fs::write(mods_dir.join("SomeOtherMod_P.pak"), b"fake").unwrap();

        let conflicts = scan_pakchunk_conflicts(mods_dir);
        assert_eq!(conflicts.len(), 2);

        let chunk5 = conflicts.iter().find(|c| c.chunk_number == 5).unwrap();
        assert_eq!(chunk5.files.len(), 2);

        let chunk20 = conflicts.iter().find(|c| c.chunk_number == 20).unwrap();
        assert_eq!(chunk20.files.len(), 3);
    }

    #[test]
    fn test_pakchunk_no_conflicts() {
        let tmp = tempfile::tempdir().unwrap();
        let mods_dir = tmp.path();

        fs::write(mods_dir.join("pakchunk5-ModA_P.pak"), b"fake").unwrap();
        fs::write(mods_dir.join("pakchunk10-ModB_P.pak"), b"fake").unwrap();

        let conflicts = scan_pakchunk_conflicts(mods_dir);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn test_ue4ss_detection() {
        let tmp = tempfile::tempdir().unwrap();
        let win64 = tmp.path().join("Phoenix").join("Binaries").join("Win64");
        fs::create_dir_all(&win64).unwrap();

        assert!(!is_ue4ss_installed(tmp.path()));

        fs::write(win64.join("UE4SS.dll"), b"fake").unwrap();
        assert!(is_ue4ss_installed(tmp.path()));
    }
}
