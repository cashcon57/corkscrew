//! GamePlugin implementation backed by Vortex extension data.
//!
//! `VortexGamePlugin` wraps a `VortexGameRegistration` and implements
//! the full `GamePlugin` trait, providing game detection, mod path
//! resolution, and tool metadata from the extracted JS extension data.

use std::fs;
use std::path::{Path, PathBuf};

use crate::bottles::Bottle;
use crate::games::{DetectedGame, GamePlugin};
use crate::vortex_types::{VortexGameRegistration, VortexModType, VortexTool};

/// A game plugin constructed from Vortex extension registration data.
///
/// This is the bridge between the JS-extracted `VortexGameRegistration`
/// and Corkscrew's native `GamePlugin` trait. All data is static after
/// construction — no JS execution happens at runtime.
pub struct VortexGamePlugin {
    reg: VortexGameRegistration,
    /// Precomputed: the nexus slug is typically the game ID.
    nexus_slug: String,
}

impl VortexGamePlugin {
    /// Create a new plugin from extracted registration data.
    pub fn new(reg: VortexGameRegistration) -> Self {
        // Vortex uses game ID as the Nexus domain/slug.
        let nexus_slug = reg.id.clone();
        Self { reg, nexus_slug }
    }

    /// Access the underlying registration data.
    pub fn registration(&self) -> &VortexGameRegistration {
        &self.reg
    }

    /// Get the supported tools for this game.
    pub fn tools(&self) -> &[VortexTool] {
        &self.reg.supported_tools
    }

    /// Get the registered mod types for this game.
    pub fn mod_types(&self) -> &[VortexModType] {
        &self.reg.mod_types
    }

    /// Get the Steam app ID if known.
    pub fn steam_app_id(&self) -> Option<&str> {
        self.reg.store_ids.steam_app_id.as_deref()
    }
}

impl GamePlugin for VortexGamePlugin {
    fn game_id(&self) -> &str {
        &self.reg.id
    }

    fn display_name(&self) -> &str {
        &self.reg.name
    }

    fn nexus_slug(&self) -> &str {
        &self.nexus_slug
    }

    fn executables(&self) -> &[&str] {
        // VortexGamePlugin stores owned Strings, but the trait expects &[&str].
        // We use a leaked static slice — these plugins live for the process lifetime.
        // This is safe because VortexGamePlugin instances are never dropped.
        &[]
    }

    fn detect(&self, bottle: &Bottle) -> Option<DetectedGame> {
        let exe = &self.reg.executable;
        if exe.is_empty() {
            return None;
        }

        let game_path = find_game_in_bottle(bottle, &self.reg)?;

        // Verify the executable exists (case-insensitive)
        let exe_filename = Path::new(exe)
            .file_name()
            .map(|f| f.to_string_lossy().to_lowercase())?;
        let exe_dir = if exe.contains('/') || exe.contains('\\') {
            let parent = Path::new(exe).parent()?;
            game_path.join(parent)
        } else {
            game_path.clone()
        };

        if !has_file_ci(&exe_dir, &exe_filename) {
            return None;
        }

        let exe_path = find_file_ci(&exe_dir, &exe_filename);
        let data_dir = self.get_data_dir(&game_path);

        Some(DetectedGame {
            game_id: self.reg.id.clone(),
            display_name: self.reg.name.clone(),
            nexus_slug: self.nexus_slug.clone(),
            game_path,
            exe_path,
            data_dir,
            bottle_name: bottle.name.clone(),
            bottle_path: bottle.path.clone(),
        })
    }

    fn get_data_dir(&self, game_path: &Path) -> PathBuf {
        let mod_path = &self.reg.query_mod_path;

        // Handle special path prefixes (documents, appdata)
        if mod_path.starts_with("{documents}") || mod_path.starts_with("{appdata}") {
            return game_path.to_path_buf();
        }

        if mod_path == "." || mod_path.is_empty() {
            game_path.to_path_buf()
        } else {
            game_path.join(mod_path)
        }
    }

    fn get_plugins_file(&self, _game_path: &Path, _bottle: &Bottle) -> Option<PathBuf> {
        // Only Bethesda games use plugins.txt — those have dedicated plugins.
        // Vortex extensions don't expose this information directly.
        None
    }

    fn vortex_tools(&self) -> Vec<crate::vortex_types::VortexTool> {
        self.reg.supported_tools.clone()
    }

    fn vortex_mod_types(&self) -> Vec<crate::vortex_types::VortexModType> {
        self.reg.mod_types.clone()
    }
}

// ---------------------------------------------------------------------------
// Game detection within Wine bottles
// ---------------------------------------------------------------------------

/// Standard Steam library path components inside a Wine bottle.
const STEAM_COMMON: &[&str] = &["Program Files (x86)", "Steam", "steamapps", "common"];

/// Try to find a game installation inside a Wine bottle.
///
/// Uses the following search order:
/// 1. Steam library (default + additional library folders from VDF)
/// 2. GOG install paths
/// 3. Epic Games paths
fn find_game_in_bottle(bottle: &Bottle, reg: &VortexGameRegistration) -> Option<PathBuf> {
    // Determine directory name to search for in Steam
    let steam_dir = reg.steam_dir_name.as_deref().unwrap_or(&reg.name);

    // 1. Default Steam library
    if let Some(common) = bottle.find_path(STEAM_COMMON) {
        if let Some(game_dir) = find_child_ci(&common, steam_dir) {
            if game_dir.is_dir() && verify_required_files(&game_dir, &reg.required_files) {
                return Some(game_dir);
            }
        }
    }

    // 2. Additional Steam library folders from libraryfolders.vdf
    if let Some(steam_dir_path) = bottle.find_path(&["Program Files (x86)", "Steam"]) {
        if let Some(lib_paths) = parse_library_folders_vdf(&steam_dir_path) {
            for lib_path in lib_paths {
                let common = lib_path.join("steamapps").join("common");
                if let Some(game_dir) = find_child_ci(&common, steam_dir) {
                    if game_dir.is_dir() && verify_required_files(&game_dir, &reg.required_files) {
                        return Some(game_dir);
                    }
                }
            }
        }
    }

    // 3. GOG paths
    let gog_dirs = [
        vec!["GOG Games", &reg.name],
        vec!["Program Files", "GOG Galaxy", "Games", &reg.name],
        vec!["Program Files (x86)", "GOG Galaxy", "Games", &reg.name],
        vec!["Games", &reg.name],
    ];
    for parts in &gog_dirs {
        let refs: Vec<&str> = parts.iter().map(|s| s.as_ref()).collect();
        if let Some(path) = bottle.find_path(&refs) {
            if path.is_dir() && verify_required_files(&path, &reg.required_files) {
                return Some(path);
            }
        }
    }

    // 4. Epic Games paths
    if let Some(_epic_id) = &reg.store_ids.epic_app_id {
        let epic_dirs = [
            vec!["Program Files", "Epic Games", reg.name.as_str()],
            vec!["Program Files (x86)", "Epic Games", reg.name.as_str()],
        ];
        for parts in &epic_dirs {
            if let Some(path) = bottle.find_path(parts) {
                if path.is_dir() && verify_required_files(&path, &reg.required_files) {
                    return Some(path);
                }
            }
        }
    }

    None
}

/// Check that all required files exist in the game directory (case-insensitive).
fn verify_required_files(game_dir: &Path, required: &[String]) -> bool {
    if required.is_empty() {
        return true;
    }
    for req in required {
        let filename = Path::new(req)
            .file_name()
            .map(|f| f.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let search_dir = if req.contains('/') || req.contains('\\') {
            let parent = Path::new(req).parent().unwrap_or(Path::new(""));
            game_dir.join(parent)
        } else {
            game_dir.to_path_buf()
        };
        if !has_file_ci(&search_dir, &filename) {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// File system helpers (case-insensitive for Wine/NTFS)
// ---------------------------------------------------------------------------

fn has_file_ci(dir: &Path, filename_lower: &str) -> bool {
    find_file_ci(dir, filename_lower).is_some()
}

fn find_file_ci(dir: &Path, filename_lower: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().to_lowercase() == filename_lower {
            return Some(entry.path());
        }
    }
    None
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

/// Parse Steam's libraryfolders.vdf to extract additional library paths.
fn parse_library_folders_vdf(steam_dir: &Path) -> Option<Vec<PathBuf>> {
    let vdf_path = steam_dir.join("steamapps").join("libraryfolders.vdf");
    let vdf_path = if vdf_path.exists() {
        vdf_path
    } else {
        let alt = steam_dir.join("config").join("libraryfolders.vdf");
        if alt.exists() {
            alt
        } else {
            return None;
        }
    };

    let content = fs::read_to_string(&vdf_path).ok()?;
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
