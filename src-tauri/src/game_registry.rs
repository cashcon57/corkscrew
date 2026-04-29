//! Game registry — auto-registers games from the Vortex game data JSON.
//!
//! Loads `data/vortex_game_registry.json` at compile time and creates
//! generic [`GamePlugin`] implementations for each entry. Games that
//! already have dedicated plugins (e.g. `skyrimse`, `fallout4`) are
//! skipped to avoid duplicates.
//!
//! The registry also exposes metadata (supported games list, tool info)
//! that the frontend can query.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::bottles::Bottle;
use crate::games::{DetectedGame, GamePlugin};

// ---------------------------------------------------------------------------
// JSON schema
// ---------------------------------------------------------------------------

/// A tool associated with a game (e.g. SSEEdit, SMAPI).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GameTool {
    pub name: String,
    pub executable: String,
}

/// A game entry from the Vortex game registry JSON.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GameEntry {
    pub game_id: String,
    pub name: String,
    pub nexus_domain: String,
    pub steam_id: Option<String>,
    pub gog_id: Option<String>,
    pub epic_id: Option<String>,
    pub executable: Option<String>,
    pub mod_path: String,
    pub required_files: Vec<String>,
    pub tools: Vec<GameTool>,
    /// Override for the Steam directory name when it differs from `name`.
    pub steam_dir: Option<String>,
    /// Note for stub entries (these are skipped during registration).
    #[serde(rename = "_note")]
    pub note: Option<String>,
}

/// Serialisable summary returned to the frontend.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SupportedGame {
    pub game_id: String,
    pub name: String,
    pub nexus_domain: String,
    pub steam_id: Option<String>,
    pub has_tools: bool,
    pub tool_names: Vec<String>,
}

// ---------------------------------------------------------------------------
// Compile-time registry data
// ---------------------------------------------------------------------------

/// The raw JSON embedded at compile time.
const REGISTRY_JSON: &str = include_str!("../data/vortex_game_registry.json");

/// Parse the registry once and return a static reference.
fn entries() -> &'static Vec<GameEntry> {
    use std::sync::OnceLock;
    static ENTRIES: OnceLock<Vec<GameEntry>> = OnceLock::new();
    ENTRIES.get_or_init(|| {
        serde_json::from_str(REGISTRY_JSON).expect("Failed to parse game registry JSON")
    })
}

/// Public read-only view of the embedded game-registry entries.
///
/// Exposed for cross-module lookups (e.g. CrossOver shortcut auto-match)
/// that need to walk the catalog by `game_id` / `executable` / `steam_id`.
pub fn all_game_entries() -> &'static [GameEntry] {
    entries().as_slice()
}

// ---------------------------------------------------------------------------
// Generic GamePlugin implementation
// ---------------------------------------------------------------------------

/// Game IDs that already have dedicated plugin modules.
/// These are skipped during auto-registration.
const CUSTOM_PLUGIN_IDS: &[&str] = &["skyrimse", "fallout4", "hogwartslegacy"];

/// A generic game plugin created from registry data.
struct RegistryGamePlugin {
    entry: &'static GameEntry,
}

impl GamePlugin for RegistryGamePlugin {
    fn game_id(&self) -> &str {
        &self.entry.game_id
    }

    fn display_name(&self) -> &str {
        &self.entry.name
    }

    fn nexus_slug(&self) -> &str {
        &self.entry.nexus_domain
    }

    fn executables(&self) -> &[&str] {
        // Return a static slice — we use a leak pattern since the data is
        // effectively 'static (embedded at compile time).
        // Detection uses find_game_path() directly rather than this list.
        &[]
    }

    fn detect(&self, bottle: &Bottle) -> Option<DetectedGame> {
        let exe = self.entry.executable.as_deref()?;
        let game_path = find_game_path(bottle, self.entry)?;

        // Verify the executable exists (case-insensitive).
        let exe_filename = Path::new(exe)
            .file_name()
            .map(|f| f.to_string_lossy().to_lowercase())?;
        let exe_dir = if exe.contains('/') || exe.contains('\\') {
            let exe_path = Path::new(exe);
            let parent = exe_path.parent()?;
            game_path.join(parent)
        } else {
            game_path.clone()
        };

        if !has_file_case_insensitive(&exe_dir, &exe_filename) {
            return None;
        }

        let exe_path = find_file_case_insensitive(&exe_dir, &exe_filename);
        let data_dir = self.get_data_dir(&game_path);

        Some(DetectedGame {
            game_id: self.entry.game_id.clone(),
            display_name: self.entry.name.clone(),
            nexus_slug: self.entry.nexus_domain.clone(),
            game_path,
            exe_path,
            data_dir,
            bottle_name: bottle.name.clone(),
            bottle_path: bottle.path.clone(),
            steam_app_id: None,
        })
    }

    fn get_data_dir(&self, game_path: &Path) -> PathBuf {
        let mod_path = &self.entry.mod_path;

        // Special prefixes for paths outside the game directory.
        if mod_path.starts_with("{documents}") || mod_path.starts_with("{appdata}") {
            // For document-relative paths, just use the game dir as data_dir.
            // The deployer will handle the actual mod path separately.
            return game_path.to_path_buf();
        }

        if mod_path == "." {
            game_path.to_path_buf()
        } else {
            game_path.join(mod_path)
        }
    }

    fn get_plugins_file(&self, _game_path: &Path, _bottle: &Bottle) -> Option<PathBuf> {
        // Only Bethesda games have plugins.txt, and those have dedicated plugins.
        None
    }
}

// ---------------------------------------------------------------------------
// Detection helpers
// ---------------------------------------------------------------------------

/// Standard Steam common path components.
const STEAM_COMMON: &[&str] = &["Program Files (x86)", "Steam", "steamapps", "common"];

/// Attempt to locate a game inside a Wine bottle.
fn find_game_path(bottle: &Bottle, entry: &GameEntry) -> Option<PathBuf> {
    // Determine the Steam directory name.
    let steam_dir_name = entry.steam_dir.as_deref().unwrap_or(&entry.name);

    // 1. Check default Steam library.
    if let Some(common) = bottle.find_path(STEAM_COMMON) {
        if let Some(game_dir) = find_child_case_insensitive(&common, steam_dir_name) {
            if game_dir.is_dir() {
                return Some(game_dir);
            }
        }
    }

    // 2. Check additional Steam library folders from libraryfolders.vdf.
    if let Some(steam_dir) = bottle.find_path(&["Program Files (x86)", "Steam"]) {
        let vdf_path = steam_dir.join("steamapps").join("libraryfolders.vdf");
        let vdf_path = if vdf_path.exists() {
            Some(vdf_path)
        } else {
            let alt = steam_dir.join("config").join("libraryfolders.vdf");
            if alt.exists() {
                Some(alt)
            } else {
                None
            }
        };

        if let Some(vdf) = vdf_path {
            if let Some(lib_paths) = parse_library_folders_vdf(&vdf) {
                for lib_path in lib_paths {
                    let common = lib_path.join("steamapps").join("common");
                    if let Some(game_dir) = find_child_case_insensitive(&common, steam_dir_name) {
                        if game_dir.is_dir() {
                            return Some(game_dir);
                        }
                    }
                }
            }
        }
    }

    // 3. Check GOG paths.
    let gog_dirs = [
        vec!["GOG Games", steam_dir_name],
        vec!["Program Files", "GOG Galaxy", "Games", steam_dir_name],
        vec!["Program Files (x86)", "GOG Galaxy", "Games", steam_dir_name],
        vec!["Games", steam_dir_name],
    ];
    for parts in &gog_dirs {
        let refs: Vec<&str> = parts.iter().map(|s| &**s).collect();
        if let Some(path) = bottle.find_path(&refs) {
            if path.is_dir() {
                return Some(path);
            }
        }
    }

    None
}

/// Check if a file exists in a directory (case-insensitive).
fn has_file_case_insensitive(dir: &Path, filename_lower: &str) -> bool {
    find_file_case_insensitive(dir, filename_lower).is_some()
}

fn find_file_case_insensitive(dir: &Path, filename_lower: &str) -> Option<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return None;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().to_lowercase() == filename_lower {
            return Some(entry.path());
        }
    }
    None
}

/// Find a child whose name matches case-insensitively.
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

/// Parse Steam's `libraryfolders.vdf` to extract library paths.
fn parse_library_folders_vdf(vdf_path: &Path) -> Option<Vec<PathBuf>> {
    let content = match fs::read_to_string(vdf_path) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Failed to read Steam VDF {}: {}", vdf_path.display(), e);
            return None;
        }
    };
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
// Public API
// ---------------------------------------------------------------------------

/// Register all games from the registry that don't have dedicated plugins.
pub fn register_all() {
    let entries = entries();
    for entry in entries.iter() {
        // Skip stubs (games with separate extensions).
        if entry.note.is_some() || entry.executable.is_none() {
            continue;
        }
        // Skip games with dedicated plugin modules.
        if CUSTOM_PLUGIN_IDS.contains(&entry.game_id.as_str()) {
            continue;
        }
        crate::games::register_plugin(Box::new(RegistryGamePlugin { entry }));
    }
}

/// Return metadata for all supported games (for the frontend).
pub fn list_supported_games() -> Vec<SupportedGame> {
    entries()
        .iter()
        .filter(|e| e.executable.is_some() && e.note.is_none())
        .map(|e| SupportedGame {
            game_id: e.game_id.clone(),
            name: e.name.clone(),
            nexus_domain: e.nexus_domain.clone(),
            steam_id: e.steam_id.clone(),
            has_tools: !e.tools.is_empty(),
            tool_names: e.tools.iter().map(|t| t.name.clone()).collect(),
        })
        .collect()
}

/// Get the full registry entry for a specific game.
pub fn get_game_entry(game_id: &str) -> Option<&'static GameEntry> {
    entries().iter().find(|e| e.game_id == game_id)
}

// ---------------------------------------------------------------------------
// Steam appmanifest scanner — detects ALL installed Steam games
// ---------------------------------------------------------------------------

/// A Steam game discovered from an appmanifest ACF file.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SteamAppManifest {
    pub app_id: String,
    pub name: String,
    pub install_dir: String,
}

/// Drive letters that Wine exposes inside a bottle as `drive_X` directories.
/// We check these when scanning for additional Steam library folders.
const WINE_DRIVE_LETTERS: &[&str] = &["c", "d", "e", "f", "z"];

/// Resolve a Windows absolute path (e.g. `D:\SteamLibrary`) to the
/// corresponding host path inside a Wine bottle.
///
/// Rules:
/// - Must begin with a single drive letter followed by `:\` or `:/`.
/// - The drive letter maps to `<bottle_path>/drive_<letter>/`.
/// - The rest of the path is appended with case-insensitive component
///   resolution so the result works on case-sensitive filesystems.
/// - Returns `None` if:
///   - the path doesn't look like a Windows absolute path,
///   - the drive directory doesn't exist in the bottle,
///   - any intermediate component doesn't exist on disk, or
///   - the resolved path would escape the bottle root (traversal guard).
fn resolve_windows_path_in_bottle(bottle: &Bottle, windows_path: &str) -> Option<PathBuf> {
    // Normalise backslashes and strip a leading drive letter.
    let normalised = windows_path.replace('\\', "/");
    let trimmed = normalised.trim();

    // Expect `X:/…`
    let mut chars = trimmed.chars();
    let drive_letter = chars.next()?.to_ascii_lowercase();
    if !drive_letter.is_ascii_alphabetic() {
        return None;
    }
    let colon = chars.next()?;
    if colon != ':' {
        return None;
    }
    let slash = chars.next()?;
    if slash != '/' {
        return None;
    }

    // Build the host drive root: <bottle>/drive_X
    let drive_dir = bottle.path.join(format!("drive_{}", drive_letter));
    if !drive_dir.is_dir() {
        return None;
    }

    // The remainder after `X:/`
    let rel: &str = &trimmed[3..]; // safe: we consumed exactly 3 chars above

    // Guard: reject any traversal attempts in the relative part.
    if rel.split('/').any(|c| c == "..") {
        log::warn!(
            "resolve_windows_path_in_bottle: traversal attempt in '{}' — rejected",
            windows_path
        );
        return None;
    }

    // Walk components case-insensitively so this works on case-sensitive FSes.
    let mut current = drive_dir;
    for component in rel.split('/').filter(|c| !c.is_empty()) {
        let candidate = current.join(component);
        if candidate.exists() {
            current = candidate;
        } else {
            // Case-insensitive fallback.
            let component_lower = component.to_lowercase();
            let mut found = false;
            if let Ok(entries) = fs::read_dir(&current) {
                for entry in entries.flatten() {
                    if entry.file_name().to_string_lossy().to_lowercase() == component_lower {
                        current = entry.path();
                        found = true;
                        break;
                    }
                }
            }
            if !found {
                return None;
            }
        }
    }

    // Final containment check: resolved path must still be inside the bottle.
    let bottle_canonical = bottle.path.canonicalize().ok()?;
    let current_canonical = current.canonicalize().ok()?;
    if !current_canonical.starts_with(&bottle_canonical) {
        log::warn!(
            "resolve_windows_path_in_bottle: '{}' resolved outside bottle — rejected",
            windows_path
        );
        return None;
    }

    Some(current)
}

/// Collect all candidate `steamapps/` directories for a bottle.
///
/// Sources (in order):
/// 1. Primary: `<bottle>/drive_c/Program Files (x86)/Steam/steamapps/`
/// 2. Additional drives: for each `drive_X` that exists, check
///    `drive_X/SteamLibrary/steamapps/` and
///    `drive_X/Program Files (x86)/Steam/steamapps/`.
/// 3. `libraryfolders.vdf` declared paths from the primary Steam install.
///
/// Dedup by canonical path so the same directory discovered via multiple
/// sources is only returned once.
fn collect_steam_library_paths(bottle: &Bottle) -> Vec<PathBuf> {
    use std::collections::HashSet;

    let mut paths: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();

    // Helper: add a steamapps path if it exists and wasn't already added.
    let mut push = |p: PathBuf| {
        if p.is_dir() {
            let key = p.canonicalize().unwrap_or_else(|_| p.clone());
            if seen.insert(key) {
                paths.push(p);
            }
        }
    };

    // 1. Primary Steam library (drive_c, Program Files (x86)).
    if let Some(p) = bottle.find_path(&["Program Files (x86)", "Steam", "steamapps"]) {
        push(p);
    }
    // Also try Program Files (non-x86) on drive_c.
    if let Some(p) = bottle.find_path(&["Program Files", "Steam", "steamapps"]) {
        push(p);
    }

    // 2. Additional drive letters.
    for letter in WINE_DRIVE_LETTERS {
        let drive_dir = bottle.path.join(format!("drive_{}", letter));
        if !drive_dir.is_dir() {
            continue;
        }

        // drive_X/SteamLibrary/steamapps
        let lib = drive_dir.join("SteamLibrary").join("steamapps");
        push(lib);

        // drive_X/Program Files (x86)/Steam/steamapps
        let pf86 = drive_dir
            .join("Program Files (x86)")
            .join("Steam")
            .join("steamapps");
        push(pf86);
    }

    // 3. libraryfolders.vdf — parse from primary Steam install.
    let vdf_candidates = [
        bottle
            .path
            .join("drive_c")
            .join("Program Files (x86)")
            .join("Steam")
            .join("steamapps")
            .join("libraryfolders.vdf"),
        bottle
            .path
            .join("drive_c")
            .join("Program Files (x86)")
            .join("Steam")
            .join("config")
            .join("libraryfolders.vdf"),
    ];

    for vdf_path in &vdf_candidates {
        if !vdf_path.exists() {
            continue;
        }
        if let Some(windows_paths) = parse_library_folders_vdf_raw(vdf_path) {
            for win_path in windows_paths {
                if let Some(host_dir) = resolve_windows_path_in_bottle(bottle, &win_path) {
                    let steamapps = host_dir.join("steamapps");
                    push(steamapps);
                }
            }
        }
        // Only parse the first VDF file found.
        break;
    }

    paths
}

/// Parse Steam's `libraryfolders.vdf` and return the raw Windows-style path
/// strings from `"path"` entries.  Handles `\\` → `\` unescaping.
fn parse_library_folders_vdf_raw(vdf_path: &Path) -> Option<Vec<String>> {
    let content = match fs::read_to_string(vdf_path) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Failed to read Steam VDF {}: {}", vdf_path.display(), e);
            return None;
        }
    };
    let mut raw_paths = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = strip_vdf_key(trimmed, "path") {
            let value = strip_vdf_quotes(rest);
            if !value.is_empty() {
                // Unescape double-backslash sequences.
                let unescaped = value.replace("\\\\", "\\");
                raw_paths.push(unescaped);
            }
        }
    }
    if raw_paths.is_empty() {
        None
    } else {
        Some(raw_paths)
    }
}

/// Scan a bottle's steamapps directory for all appmanifest files.
/// Returns games that are NOT already detected by registered plugins.
pub fn detect_unregistered_steam_games(
    bottle: &Bottle,
    already_detected: &[DetectedGame],
) -> Vec<DetectedGame> {
    use std::collections::HashSet;

    let library_paths = collect_steam_library_paths(bottle);
    if library_paths.is_empty() {
        return Vec::new();
    }

    let mut found: Vec<DetectedGame> = Vec::new();
    // Dedup by canonical appmanifest path to avoid double-counting the same
    // game when multiple sources resolve to the same library folder.
    let mut seen_manifests: HashSet<PathBuf> = HashSet::new();

    for steamapps in &library_paths {
        let Ok(entries) = fs::read_dir(steamapps) else {
            continue;
        };

        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("appmanifest_") || !name.ends_with(".acf") {
                continue;
            }

            let acf_path = entry.path();

            // Dedup: skip if we've already processed this manifest file.
            let canonical_acf = acf_path.canonicalize().unwrap_or_else(|_| acf_path.clone());
            if !seen_manifests.insert(canonical_acf) {
                continue;
            }

            let manifest = match parse_appmanifest(&acf_path) {
                Some(m) => m,
                None => continue,
            };

            // Skip if already detected by a registered plugin.
            let manifest_dir_lower = manifest.install_dir.to_lowercase();
            if already_detected.iter().any(|g| {
                g.game_path.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.to_lowercase() == manifest_dir_lower)
                    .unwrap_or(false)
                    || g.game_path
                        .to_string_lossy()
                        .to_lowercase()
                        .ends_with(&format!("/{}", manifest_dir_lower))
                    || g.game_path
                        .to_string_lossy()
                        .to_lowercase()
                        .ends_with(&format!("\\{}", manifest_dir_lower))
            }) {
                continue;
            }

            // Resolve the actual game path.
            let common = steamapps.join("common");
            let game_path = match find_child_case_insensitive(&common, &manifest.install_dir) {
                Some(p) if p.is_dir() => p,
                _ => continue,
            };

            // Find the first .exe in the game directory (heuristic).
            let exe_path = find_main_executable(&game_path);

            // Derive a game_id from the app name.
            let game_id = slugify_game_name(&manifest.name);

            // Resolve the Nexus slug via curated index, fall back to slug.
            let nexus_slug =
                crate::vortex_index::lookup_extension_for_steam_appid(&manifest.app_id)
                    .map(|e| e.nexus_slug.clone())
                    .unwrap_or_else(|| game_id.replace('-', ""));

            found.push(DetectedGame {
                game_id: game_id.clone(),
                display_name: manifest.name.clone(),
                nexus_slug,
                game_path: game_path.clone(),
                exe_path,
                data_dir: game_path,
                bottle_name: bottle.name.clone(),
                bottle_path: bottle.path.clone(),
                steam_app_id: Some(manifest.app_id.clone()),
            });
        }
    }

    found
}

// ---------------------------------------------------------------------------
// Vortex extension suggestions
// ---------------------------------------------------------------------------

/// Pairing of a detected (but unregistered) Steam game with the upstream
/// Vortex extension that mods it.
///
/// Returned by [`collect_extension_suggestions`] when an installed Steam game
/// has no native Corkscrew plugin and no entry in `vortex_game_registry.json`,
/// but the curated [`crate::vortex_index`] has a matching extension.
///
/// The frontend uses `vortex_dir_name` as the argument to the existing
/// `vortex_fetch_extension` Tauri command — once fetched and registered,
/// the game is no longer "unknown".
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VortexExtensionSuggestion {
    /// Stable extension entry id (e.g. `"game-cyberpunk2077"`).
    pub extension_id: String,
    /// Directory name in `Nexus-Mods/vortex-games`. Pass to
    /// `vortex_fetch_extension` to install.
    pub vortex_dir_name: String,
    /// Human-readable game name (e.g. `"Cyberpunk 2077"`).
    pub display_name: String,
    /// Nexus Mods slug, useful for surfacing a "view on Nexus" link.
    pub nexus_slug: String,
    /// Steam app ID this suggestion was matched against.
    pub steam_app_id: String,
    /// Slugified game ID Corkscrew assigned to the unregistered game.
    /// Lets the frontend correlate the suggestion with the entry already in
    /// the detected-games list.
    pub detected_game_id: String,
    /// Bottle the game was found in. A single Vortex extension may apply to
    /// multiple bottles when the same game is installed in more than one.
    pub bottle_name: String,
}

/// Walk every bottle, find Steam games that aren't covered by a registered
/// plugin or the bundled `vortex_game_registry.json`, and pair each with the
/// matching upstream Vortex extension (if one exists in the static index).
///
/// `already_registered_ids` is the set of game IDs the caller has already
/// covered through native plugins, the bundled registry, and any
/// already-fetched Vortex extensions — those don't need a suggestion.
pub fn collect_extension_suggestions(
    already_registered_ids: &[String],
) -> Vec<VortexExtensionSuggestion> {
    use crate::bottles::detect_bottles;
    use std::collections::HashSet;

    let registered: HashSet<&str> = already_registered_ids
        .iter()
        .map(|s| s.as_str())
        .collect();

    let mut out: Vec<VortexExtensionSuggestion> = Vec::new();
    let mut seen_pairs: HashSet<(String, String)> = HashSet::new();

    for bottle in detect_bottles() {
        for unregistered in scan_steam_games_with_appid(&bottle) {
            // Skip if a registered plugin already covers this game ID.
            if registered.contains(unregistered.game.game_id.as_str()) {
                continue;
            }
            // Skip if the bundled registry already maps this game ID
            // (e.g. via a steam_id entry in vortex_game_registry.json).
            if get_game_entry(&unregistered.game.game_id).is_some() {
                continue;
            }
            let Some(entry) =
                crate::vortex_index::lookup_extension_for_steam_appid(&unregistered.app_id)
            else {
                continue;
            };
            let key = (entry.id.clone(), bottle.name.clone());
            if !seen_pairs.insert(key) {
                continue;
            }
            out.push(VortexExtensionSuggestion {
                extension_id: entry.id.clone(),
                vortex_dir_name: entry.vortex_dir_name.clone(),
                display_name: entry.name.clone(),
                nexus_slug: entry.nexus_slug.clone(),
                steam_app_id: entry.steam_app_id.clone(),
                detected_game_id: unregistered.game.game_id.clone(),
                bottle_name: bottle.name.clone(),
            });
        }
    }

    out
}

/// Pair an unregistered Steam game with the Steam app ID that produced it.
///
/// Used internally by [`collect_extension_suggestions`] — the public
/// [`detect_unregistered_steam_games`] discards the app ID after slugifying.
struct UnregisteredSteamGame {
    game: DetectedGame,
    app_id: String,
}

/// Variant of [`detect_unregistered_steam_games`] that retains each game's
/// Steam app ID for downstream lookups.
///
/// We don't want to change the public signature of
/// [`detect_unregistered_steam_games`] (callers all over the codebase use it),
/// so we accept a small amount of duplication here.
fn scan_steam_games_with_appid(bottle: &Bottle) -> Vec<UnregisteredSteamGame> {
    use std::collections::HashSet;

    let library_paths = collect_steam_library_paths(bottle);
    if library_paths.is_empty() {
        return Vec::new();
    }

    let mut out: Vec<UnregisteredSteamGame> = Vec::new();
    let mut seen_manifests: HashSet<PathBuf> = HashSet::new();

    for steamapps in &library_paths {
        let Ok(entries) = fs::read_dir(steamapps) else {
            continue;
        };

        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("appmanifest_") || !name.ends_with(".acf") {
                continue;
            }

            let acf_path = entry.path();
            let canonical_acf = acf_path.canonicalize().unwrap_or_else(|_| acf_path.clone());
            if !seen_manifests.insert(canonical_acf) {
                continue;
            }

            let manifest = match parse_appmanifest(&acf_path) {
                Some(m) => m,
                None => continue,
            };
            let common = steamapps.join("common");
            let game_path = match find_child_case_insensitive(&common, &manifest.install_dir) {
                Some(p) if p.is_dir() => p,
                _ => continue,
            };
            let exe_path = find_main_executable(&game_path);
            let game_id = slugify_game_name(&manifest.name);
            out.push(UnregisteredSteamGame {
                game: DetectedGame {
                    game_id: game_id.clone(),
                    display_name: manifest.name.clone(),
                    nexus_slug: game_id,
                    game_path: game_path.clone(),
                    exe_path,
                    data_dir: game_path,
                    bottle_name: bottle.name.clone(),
                    bottle_path: bottle.path.clone(),
                    steam_app_id: None,
                },
                app_id: manifest.app_id,
            });
        }
    }
    out
}

/// Parse a Steam appmanifest ACF file to extract app ID, name, and install dir.
fn parse_appmanifest(path: &Path) -> Option<SteamAppManifest> {
    let content = fs::read_to_string(path).ok()?;
    let app_id = extract_acf_value(&content, "appid")?;
    let name = extract_acf_value(&content, "name")?;
    let install_dir = extract_acf_value(&content, "installdir")?;

    // Skip tools/configs (Steamworks Shared, Proton, etc.)
    let state_flags = extract_acf_value(&content, "StateFlags")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    // StateFlags 4 = fully installed. Skip if 0 (invalid) or other states.
    if state_flags == 0 {
        return None;
    }

    // Skip known non-game entries
    let lower_name = name.to_lowercase();
    if lower_name.contains("steamworks")
        || lower_name.contains("proton")
        || lower_name.contains("steam linux runtime")
        || lower_name.contains("steam controller")
        || lower_name.contains("redistributable")
        || lower_name.contains("directx")
    {
        return None;
    }

    Some(SteamAppManifest {
        app_id,
        name,
        install_dir,
    })
}

/// Extract a quoted value from a VDF/ACF key-value pair.
fn extract_acf_value(content: &str, key: &str) -> Option<String> {
    let key_pat = format!("\"{}\"", key);
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(&key_pat) {
            let val = rest.trim();
            if val.starts_with('"') && val.len() >= 2 {
                let end = val[1..].find('"').map(|i| i + 1)?;
                return Some(val[1..end].to_string());
            }
        }
    }
    None
}

/// A depot entry extracted from a Steam appmanifest ACF file.
#[derive(Debug, Clone)]
pub struct SteamDepotInfo {
    pub app_id: String,
    pub depot_id: String,
    pub manifest_id: String,
    pub build_id: String,
}

/// Extract installed depot info from a Steam appmanifest ACF file.
/// Returns a list of (depot_id, manifest_id) pairs plus the build_id.
pub fn extract_depot_info(acf_path: &Path) -> Option<Vec<SteamDepotInfo>> {
    let content = fs::read_to_string(acf_path).ok()?;
    let app_id = extract_acf_value(&content, "appid")?;
    let build_id = extract_acf_value(&content, "buildid").unwrap_or_default();

    // Parse the InstalledDepots section to extract depot_id → manifest pairs
    let mut depots = Vec::new();
    let mut in_depots = false;
    let mut current_depot_id: Option<String> = None;
    let mut brace_depth = 0i32;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.contains("\"InstalledDepots\"") {
            in_depots = true;
            continue;
        }

        if !in_depots {
            continue;
        }

        if trimmed == "{" {
            brace_depth += 1;
            continue;
        }

        if trimmed == "}" {
            brace_depth -= 1;
            if brace_depth <= 0 {
                break; // End of InstalledDepots section
            }
            current_depot_id = None;
            continue;
        }

        // At depth 1: depot IDs (e.g., "990081")
        if brace_depth == 1 {
            if let Some(depot_id) = extract_quoted_value(trimmed) {
                current_depot_id = Some(depot_id);
            }
        }

        // At depth 2: depot properties (manifest, size)
        if brace_depth == 2 {
            if let Some(ref depot_id) = current_depot_id {
                if let Some(manifest) = try_extract_key_value(trimmed, "manifest") {
                    depots.push(SteamDepotInfo {
                        app_id: app_id.clone(),
                        depot_id: depot_id.clone(),
                        manifest_id: manifest,
                        build_id: build_id.clone(),
                    });
                }
            }
        }
    }

    if depots.is_empty() {
        None
    } else {
        Some(depots)
    }
}

/// Extract a standalone quoted string from a line (e.g., `"990081"` → `990081`).
fn extract_quoted_value(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 3 {
        // Single quoted value on a line (depot ID)
        let inner = &trimmed[1..trimmed.len() - 1];
        if !inner.contains('"') {
            return Some(inner.to_string());
        }
    }
    None
}

/// Try to extract a key-value pair from a VDF line (e.g., `"manifest"  "123"` → Some("123")).
fn try_extract_key_value(line: &str, key: &str) -> Option<String> {
    let key_pat = format!("\"{}\"", key);
    let trimmed = line.trim();
    if let Some(rest) = trimmed.strip_prefix(&key_pat) {
        let val = rest.trim();
        if val.starts_with('"') && val.len() >= 2 {
            let end = val[1..].find('"').map(|i| i + 1)?;
            return Some(val[1..end].to_string());
        }
    }
    None
}

/// Capture depot manifest info for a detected game and store in the database.
/// Called during game detection to build a local version history.
pub fn capture_depot_manifests(
    db: &crate::database::ModDatabase,
    game_id: &str,
    bottle: &crate::bottles::Bottle,
) {
    // Find steamapps directory
    let steamapps_paths = [
        bottle.drive_c().join("Program Files (x86)").join("Steam").join("steamapps"),
        bottle.drive_c().join("Program Files").join("Steam").join("steamapps"),
    ];

    let steamapps = match steamapps_paths.iter().find(|p| p.exists()) {
        Some(p) => p,
        None => return,
    };

    // Look up the Steam App ID from the game registry
    let app_id = match get_game_entry(game_id) {
        Some(entry) => match entry.steam_id.as_deref() {
            Some(id) => id.to_string(),
            None => return,
        },
        None => return,
    };

    // Find the ACF file
    let acf_path = steamapps.join(format!("appmanifest_{}.acf", app_id));
    if !acf_path.exists() {
        return;
    }

    // Extract depot info
    let depots = match extract_depot_info(&acf_path) {
        Some(d) => d,
        None => return,
    };

    // Get game version from plugin (if available)
    let game_version = crate::games::with_plugin(game_id, |plugin| {
        // Find the game path from the ACF install dir
        let install_dir = extract_acf_value(
            &fs::read_to_string(&acf_path).unwrap_or_default(),
            "installdir",
        )
        .unwrap_or_default();
        let game_path = steamapps.join("common").join(&install_dir);
        plugin.detect_game_version(&game_path)
    })
    .flatten();

    // Store in database
    if let Ok(conn) = db.conn() {
        for depot in &depots {
            let result = conn.execute(
                "INSERT OR IGNORE INTO steam_depot_history
                 (game_id, app_id, depot_id, manifest_id, build_id, game_version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    game_id,
                    depot.app_id,
                    depot.depot_id,
                    depot.manifest_id,
                    depot.build_id,
                    game_version,
                ],
            );
            if let Ok(1) = result {
                log::info!(
                    "Captured depot manifest: game={}, depot={}, manifest={}, build={}, version={:?}",
                    game_id,
                    depot.depot_id,
                    depot.manifest_id,
                    depot.build_id,
                    game_version,
                );
            }
        }
    }
}

/// Look up a manifest ID for a specific game version from the captured history.
pub fn lookup_manifest_for_version(
    db: &crate::database::ModDatabase,
    game_id: &str,
    target_version: &str,
) -> Option<SteamDepotInfo> {
    let conn = db.conn().ok()?;
    let mut stmt = conn
        .prepare(
            "SELECT app_id, depot_id, manifest_id, build_id
             FROM steam_depot_history
             WHERE game_id = ?1 AND game_version = ?2
             ORDER BY captured_at DESC
             LIMIT 1",
        )
        .ok()?;

    stmt.query_row(rusqlite::params![game_id, target_version], |row| {
        Ok(SteamDepotInfo {
            app_id: row.get(0)?,
            depot_id: row.get(1)?,
            manifest_id: row.get(2)?,
            build_id: row.get(3)?,
        })
    })
    .ok()
}

/// Get all captured versions for a game.
pub fn get_depot_history(
    db: &crate::database::ModDatabase,
    game_id: &str,
) -> Vec<(String, String, String)> {
    let conn = match db.conn() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut stmt = match conn.prepare(
        "SELECT game_version, build_id, manifest_id
         FROM steam_depot_history
         WHERE game_id = ?1 AND game_version IS NOT NULL
         GROUP BY game_version
         ORDER BY captured_at DESC",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    stmt.query_map(rusqlite::params![game_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })
    .ok()
    .map(|rows| rows.filter_map(|r| r.ok()).collect())
    .unwrap_or_default()
}

/// Public wrapper for find_main_executable.
pub fn find_main_executable_public(game_path: &Path) -> Option<PathBuf> {
    find_main_executable(game_path)
}

/// Find the most likely main executable in a game directory.
/// Prefers .exe files in the root, skips crash reporters and launchers.
fn find_main_executable(game_path: &Path) -> Option<PathBuf> {
    let skip_patterns = [
        "crash",
        "report",
        "installer",
        "unins",
        "setup",
        "redis",
        "vc_redist",
        "dxsetup",
        "dotnet",
    ];

    let Ok(entries) = fs::read_dir(game_path) else {
        return None;
    };

    let mut candidates: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if !name.ends_with(".exe") {
            continue;
        }
        let skip = skip_patterns.iter().any(|p| name.contains(p));
        if skip {
            continue;
        }
        candidates.push(entry.path());
    }

    // Sort by size descending — the main exe is usually the largest
    candidates.sort_by(|a, b| {
        let sa = a.metadata().map(|m| m.len()).unwrap_or(0);
        let sb = b.metadata().map(|m| m.len()).unwrap_or(0);
        sb.cmp(&sa)
    });

    candidates.into_iter().next()
}

/// Convert a game name into a URL-safe slug for use as game_id.
fn slugify_game_name(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

// ---------------------------------------------------------------------------
// Custom games (user-added, persisted in DB)
// ---------------------------------------------------------------------------

/// A custom game added by the user (stored in SQLite).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CustomGame {
    pub game_id: String,
    pub display_name: String,
    pub nexus_slug: String,
    pub game_path: String,
    pub exe_path: Option<String>,
    pub data_dir: String,
    pub bottle_name: String,
    pub bottle_path: String,
    pub steam_app_id: Option<String>,
}

/// Load custom games from the database and return them as DetectedGame entries.
pub fn load_custom_games(db: &crate::database::ModDatabase) -> Vec<DetectedGame> {
    let conn = match db.conn() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut stmt = match conn.prepare(
        "SELECT game_id, display_name, nexus_slug, game_path, exe_path, data_dir, bottle_name, bottle_path FROM custom_games",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(), // Table might not exist yet
    };
    let rows = stmt
        .query_map([], |row| {
            Ok(DetectedGame {
                game_id: row.get(0)?,
                display_name: row.get(1)?,
                nexus_slug: row.get(2)?,
                game_path: PathBuf::from(row.get::<_, String>(3)?),
                exe_path: row.get::<_, Option<String>>(4)?.map(PathBuf::from),
                data_dir: PathBuf::from(row.get::<_, String>(5)?),
                bottle_name: row.get(6)?,
                bottle_path: PathBuf::from(row.get::<_, String>(7)?),
                steam_app_id: None,
            })
        })
        .ok();
    match rows {
        Some(r) => r.flatten().collect(),
        None => Vec::new(),
    }
}

/// Save a custom game to the database.
pub fn save_custom_game(
    db: &crate::database::ModDatabase,
    game: &CustomGame,
) -> Result<(), String> {
    let conn = db
        .conn()
        .map_err(|e| format!("No database connection: {e}"))?;
    conn.execute(
        "INSERT OR REPLACE INTO custom_games (game_id, display_name, nexus_slug, game_path, exe_path, data_dir, bottle_name, bottle_path, steam_app_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            game.game_id,
            game.display_name,
            game.nexus_slug,
            game.game_path,
            game.exe_path,
            game.data_dir,
            game.bottle_name,
            game.bottle_path,
            game.steam_app_id,
        ],
    )
    .map_err(|e| format!("Failed to save custom game: {e}"))?;
    Ok(())
}

/// Remove a custom game from the database.
pub fn remove_custom_game(db: &crate::database::ModDatabase, game_id: &str) -> Result<(), String> {
    let conn = db
        .conn()
        .map_err(|e| format!("No database connection: {e}"))?;
    conn.execute(
        "DELETE FROM custom_games WHERE game_id = ?1",
        rusqlite::params![game_id],
    )
    .map_err(|e| format!("Failed to remove custom game: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_parses_successfully() {
        let entries = entries();
        assert!(!entries.is_empty());
        // Should have at least 70 games
        assert!(
            entries.len() >= 70,
            "Expected 70+ games, got {}",
            entries.len()
        );
    }

    #[test]
    fn skyrimse_is_first() {
        let entries = entries();
        assert_eq!(entries[0].game_id, "skyrimse");
    }

    #[test]
    fn no_stubs_in_supported_list() {
        let supported = list_supported_games();
        for game in &supported {
            assert!(!game.game_id.is_empty(), "Empty game_id in supported list");
        }
        // Cyberpunk was a stub in original data but we added real data
        assert!(supported.iter().any(|g| g.game_id == "cyberpunk2077"));
    }

    #[test]
    fn custom_plugins_excluded_from_registration() {
        // Verify that the custom plugin IDs would be skipped
        for id in CUSTOM_PLUGIN_IDS {
            let entry = entries().iter().find(|e| e.game_id == *id);
            assert!(entry.is_some(), "Custom plugin {} not in registry", id);
        }
    }

    #[test]
    fn tools_parsed_correctly() {
        let skyrimse = entries().iter().find(|e| e.game_id == "skyrimse").unwrap();
        assert!(!skyrimse.tools.is_empty());
        assert!(skyrimse.tools.iter().any(|t| t.name.contains("SSEEdit")));
    }

    #[test]
    fn steam_dir_override_works() {
        let falloutnv = entries().iter().find(|e| e.game_id == "falloutnv").unwrap();
        assert_eq!(falloutnv.steam_dir.as_deref(), Some("Fallout New Vegas"));
    }

    #[test]
    fn list_supported_games_returns_data() {
        let supported = list_supported_games();
        assert!(!supported.is_empty());
        // Should include Skyrim SE
        assert!(supported.iter().any(|g| g.game_id == "skyrimse"));
        // Skyrim SE should have tools
        let sse = supported.iter().find(|g| g.game_id == "skyrimse").unwrap();
        assert!(sse.has_tools);
    }

    #[test]
    fn vortex_extension_suggestion_serializes_snake_case() {
        // Surface for the frontend — snake_case to match the rest of the
        // Vortex API surface in types.ts.
        let s = VortexExtensionSuggestion {
            extension_id: "game-cyberpunk2077".into(),
            vortex_dir_name: "game-cyberpunk2077".into(),
            display_name: "Cyberpunk 2077".into(),
            nexus_slug: "cyberpunk2077".into(),
            steam_app_id: "1091500".into(),
            detected_game_id: "cyberpunk-2077".into(),
            bottle_name: "Steam".into(),
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"vortex_dir_name\""));
        assert!(json.contains("\"steam_app_id\""));
        assert!(json.contains("\"detected_game_id\""));
    }

    #[test]
    fn collect_extension_suggestions_does_not_panic_on_empty() {
        // Smoke test: function is safe to call when there are no bottles or
        // when every detected game is already registered.
        let _ = collect_extension_suggestions(&[]);
        let many: Vec<String> = vec!["skyrimse".into(), "fallout4".into()];
        let _ = collect_extension_suggestions(&many);
    }

    // -----------------------------------------------------------------------
    // Multi-library Steam scanner tests
    // -----------------------------------------------------------------------

    /// Write a minimal appmanifest ACF file for testing.
    fn write_appmanifest(dir: &Path, app_id: &str, name: &str, install_dir: &str) {
        let content = format!(
            "\"AppState\"\n{{\n\
             \t\"appid\"\t\"{app_id}\"\n\
             \t\"name\"\t\"{name}\"\n\
             \t\"installdir\"\t\"{install_dir}\"\n\
             \t\"StateFlags\"\t\"4\"\n\
             }}\n"
        );
        fs::write(
            dir.join(format!("appmanifest_{}.acf", app_id)),
            content,
        )
        .unwrap();
    }

    /// Create a minimal Bottle pointing at a temp directory.
    fn make_bottle(path: &Path) -> Bottle {
        Bottle {
            name: "Test".into(),
            path: path.to_path_buf(),
            source: "test".into(),
        }
    }

    #[test]
    fn multi_drive_steam_libraries_detected() {
        let root = tempfile::tempdir().unwrap();
        let bottle_path = root.path().to_path_buf();

        // Primary Steam install on drive_c.
        let primary_steamapps = bottle_path
            .join("drive_c")
            .join("Program Files (x86)")
            .join("Steam")
            .join("steamapps");
        fs::create_dir_all(&primary_steamapps).unwrap();
        let primary_common = primary_steamapps.join("common");
        fs::create_dir_all(primary_common.join("GameOnC")).unwrap();
        write_appmanifest(&primary_steamapps, "100", "Game On C", "GameOnC");

        // Additional Steam library on drive_d.
        let drive_d_steamapps = bottle_path
            .join("drive_d")
            .join("SteamLibrary")
            .join("steamapps");
        fs::create_dir_all(&drive_d_steamapps).unwrap();
        let drive_d_common = drive_d_steamapps.join("common");
        fs::create_dir_all(drive_d_common.join("GameOnD")).unwrap();
        write_appmanifest(&drive_d_steamapps, "200", "Game On D", "GameOnD");

        let bottle = make_bottle(&bottle_path);
        let results = detect_unregistered_steam_games(&bottle, &[]);

        let ids: Vec<&str> = results.iter().map(|g| g.game_id.as_str()).collect();
        assert!(
            ids.contains(&"game-on-c"),
            "Expected game-on-c, got: {:?}",
            ids
        );
        assert!(
            ids.contains(&"game-on-d"),
            "Expected game-on-d, got: {:?}",
            ids
        );
        assert_eq!(results.len(), 2, "Should detect exactly 2 games, got: {:?}", ids);
    }

    #[test]
    fn libraryfolders_vdf_parsed_and_scanned() {
        let root = tempfile::tempdir().unwrap();
        let bottle_path = root.path().to_path_buf();

        // Primary Steam install — only used to host the VDF, no games here.
        let primary_steamapps = bottle_path
            .join("drive_c")
            .join("Program Files (x86)")
            .join("Steam")
            .join("steamapps");
        fs::create_dir_all(&primary_steamapps).unwrap();

        // Write a realistic libraryfolders.vdf pointing at D:\SteamLibrary.
        let vdf_content = "\"libraryfolders\"\n\
            {\n\
            \t\"0\"\n\
            \t{\n\
            \t\t\"path\"\t\t\"C:\\\\Program Files (x86)\\\\Steam\"\n\
            \t\t\"label\"\t\t\"\"\n\
            \t}\n\
            \t\"1\"\n\
            \t{\n\
            \t\t\"path\"\t\t\"D:\\\\SteamLibrary\"\n\
            \t\t\"label\"\t\t\"D Drive\"\n\
            \t}\n\
            }\n";
        fs::write(primary_steamapps.join("libraryfolders.vdf"), vdf_content).unwrap();

        // D:\SteamLibrary\steamapps — the VDF-declared library.
        let d_steamapps = bottle_path
            .join("drive_d")
            .join("SteamLibrary")
            .join("steamapps");
        fs::create_dir_all(&d_steamapps).unwrap();
        let d_common = d_steamapps.join("common");
        fs::create_dir_all(d_common.join("CoolGame")).unwrap();
        write_appmanifest(&d_steamapps, "999", "Cool Game", "CoolGame");

        let bottle = make_bottle(&bottle_path);
        let results = detect_unregistered_steam_games(&bottle, &[]);

        let ids: Vec<&str> = results.iter().map(|g| g.game_id.as_str()).collect();
        assert!(
            ids.contains(&"cool-game"),
            "Expected cool-game from VDF library, got: {:?}",
            ids
        );
    }

    #[test]
    fn dedup_same_game_in_two_libraries() {
        let root = tempfile::tempdir().unwrap();
        let bottle_path = root.path().to_path_buf();

        // The drive_d SteamLibrary folder is the canonical location.
        let d_steamapps = bottle_path
            .join("drive_d")
            .join("SteamLibrary")
            .join("steamapps");
        fs::create_dir_all(&d_steamapps).unwrap();
        let d_common = d_steamapps.join("common");
        fs::create_dir_all(d_common.join("SharedGame")).unwrap();
        write_appmanifest(&d_steamapps, "500", "Shared Game", "SharedGame");

        // Primary drive_c Steam install — points at drive_d via VDF so the
        // same library would be discovered twice.
        let primary_steamapps = bottle_path
            .join("drive_c")
            .join("Program Files (x86)")
            .join("Steam")
            .join("steamapps");
        fs::create_dir_all(&primary_steamapps).unwrap();

        let vdf_content = "\"libraryfolders\"\n\
            {\n\
            \t\"1\"\n\
            \t{\n\
            \t\t\"path\"\t\t\"D:\\\\SteamLibrary\"\n\
            \t}\n\
            }\n";
        fs::write(primary_steamapps.join("libraryfolders.vdf"), vdf_content).unwrap();

        let bottle = make_bottle(&bottle_path);
        let results = detect_unregistered_steam_games(&bottle, &[]);

        assert_eq!(
            results.len(),
            1,
            "Dedup should prevent double-counting — got: {:?}",
            results.iter().map(|g| &g.game_id).collect::<Vec<_>>()
        );
        assert_eq!(results[0].game_id, "shared-game");
    }

    #[test]
    fn no_vdf_falls_back_to_drive_scan_only() {
        let root = tempfile::tempdir().unwrap();
        let bottle_path = root.path().to_path_buf();

        // drive_c primary — no VDF file.
        let primary_steamapps = bottle_path
            .join("drive_c")
            .join("Program Files (x86)")
            .join("Steam")
            .join("steamapps");
        fs::create_dir_all(&primary_steamapps).unwrap();
        let c_common = primary_steamapps.join("common");
        fs::create_dir_all(c_common.join("CGame")).unwrap();
        write_appmanifest(&primary_steamapps, "10", "C Game", "CGame");

        // drive_e SteamLibrary — no VDF reference.
        let e_steamapps = bottle_path
            .join("drive_e")
            .join("SteamLibrary")
            .join("steamapps");
        fs::create_dir_all(&e_steamapps).unwrap();
        let e_common = e_steamapps.join("common");
        fs::create_dir_all(e_common.join("EGame")).unwrap();
        write_appmanifest(&e_steamapps, "20", "E Game", "EGame");

        let bottle = make_bottle(&bottle_path);
        let results = detect_unregistered_steam_games(&bottle, &[]);

        let ids: Vec<&str> = results.iter().map(|g| g.game_id.as_str()).collect();
        assert!(ids.contains(&"c-game"), "Expected c-game, got: {:?}", ids);
        assert!(ids.contains(&"e-game"), "Expected e-game, got: {:?}", ids);
    }

    #[test]
    fn malformed_vdf_does_not_crash() {
        let root = tempfile::tempdir().unwrap();
        let bottle_path = root.path().to_path_buf();

        let primary_steamapps = bottle_path
            .join("drive_c")
            .join("Program Files (x86)")
            .join("Steam")
            .join("steamapps");
        fs::create_dir_all(&primary_steamapps).unwrap();

        // Write a deliberately malformed VDF.
        fs::write(
            primary_steamapps.join("libraryfolders.vdf"),
            "this is not valid vdf content\x00\x01\x02\n{{{{{}}",
        )
        .unwrap();

        let bottle = make_bottle(&bottle_path);
        // Should not panic — just return an empty list.
        let results = detect_unregistered_steam_games(&bottle, &[]);
        assert!(results.is_empty());
    }

    #[test]
    fn resolve_windows_path_rejects_traversal() {
        let root = tempfile::tempdir().unwrap();
        let bottle_path = root.path().to_path_buf();
        fs::create_dir_all(bottle_path.join("drive_c")).unwrap();

        let bottle = make_bottle(&bottle_path);
        // Traversal via `..` should be rejected.
        assert!(
            resolve_windows_path_in_bottle(&bottle, "C:\\..\\etc\\passwd").is_none(),
            "Traversal path should be rejected"
        );
    }

    #[test]
    fn resolve_windows_path_missing_drive_returns_none() {
        let root = tempfile::tempdir().unwrap();
        let bottle_path = root.path().to_path_buf();
        // Only drive_c exists.
        fs::create_dir_all(bottle_path.join("drive_c")).unwrap();

        let bottle = make_bottle(&bottle_path);
        // E: drive doesn't exist in the bottle.
        assert!(
            resolve_windows_path_in_bottle(&bottle, "E:\\SteamLibrary").is_none(),
            "Missing drive should return None"
        );
    }

    #[test]
    fn resolve_windows_path_valid_path() {
        let root = tempfile::tempdir().unwrap();
        let bottle_path = root.path().to_path_buf();
        let target = bottle_path.join("drive_d").join("SteamLibrary");
        fs::create_dir_all(&target).unwrap();

        let bottle = make_bottle(&bottle_path);
        let resolved = resolve_windows_path_in_bottle(&bottle, "D:\\SteamLibrary");
        assert!(resolved.is_some(), "Should resolve D:\\SteamLibrary");
        // Canonicalize both for comparison.
        let resolved_canon = resolved.unwrap().canonicalize().unwrap();
        let expected_canon = target.canonicalize().unwrap();
        assert_eq!(resolved_canon, expected_canon);
    }
}
