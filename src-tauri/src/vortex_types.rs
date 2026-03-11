//! Data model structs for extracted Vortex game extension data.
//!
//! These mirror the registration data from Vortex's `context.registerGame()`,
//! `context.registerModType()`, and `context.registerInstaller()` calls.
//! All structs are serializable for JSON caching.

use serde::{Deserialize, Serialize};

/// Tool associated with a game (e.g. SKSE64, F4SE, REDmod, SMAPI).
///
/// Extracted from Vortex extension's `supportedTools` array.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VortexTool {
    pub id: String,
    pub name: String,
    /// Relative path to the tool executable (e.g. `"skse64_loader.exe"`).
    pub executable: String,
    #[serde(default)]
    pub required_files: Vec<String>,
    /// Short name for compact display.
    pub short_name: Option<String>,
    /// Whether the exe path is relative to the game directory.
    #[serde(default)]
    pub relative: bool,
    /// Whether to run this tool exclusively (no other tools concurrently).
    #[serde(default)]
    pub exclusive: bool,
    /// Whether this is the default launch tool.
    #[serde(default)]
    pub default_primary: bool,
    /// Launch parameters.
    #[serde(default)]
    pub parameters: Vec<String>,
}

/// A mod type defining an alternative install path for a game.
///
/// Some games have multiple mod types with different deploy targets:
/// e.g. Witcher 3 has DLC mods, menu mods, and root mods each going
/// to different directories.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VortexModType {
    pub id: String,
    pub priority: i32,
    /// The resolved install path (relative to game directory).
    /// Extracted by calling the mod type's `getPath()` function.
    pub target_path: String,
}

/// Metadata for a registered custom installer.
///
/// We capture the ID and priority but cannot statically extract the
/// JS callback logic. The raw JS source is cached separately for
/// potential runtime execution.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VortexInstallerMeta {
    pub id: String,
    pub priority: i32,
}

/// Store-based game discovery query arguments.
///
/// Modern Vortex extensions use `queryArgs` with store-specific IDs
/// instead of the legacy `queryPath` function.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StoreIds {
    pub steam_app_id: Option<String>,
    pub gog_app_id: Option<String>,
    pub epic_app_id: Option<String>,
    pub xbox_id: Option<String>,
}

/// Full game registration data extracted from a Vortex extension.
///
/// This is the primary output of executing an extension's `index.js`
/// in the QuickJS sandbox. Contains everything needed to create a
/// `GamePlugin` implementation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VortexGameRegistration {
    /// Game identifier (e.g. `"skyrimse"`, `"witcher3"`).
    /// Also used as the Nexus Mods domain/slug.
    pub id: String,
    /// Display name (e.g. `"The Elder Scrolls V: Skyrim Special Edition"`).
    pub name: String,
    /// Main game executable filename (e.g. `"SkyrimSE.exe"`).
    pub executable: String,
    /// Files that must exist to validate the game directory.
    #[serde(default)]
    pub required_files: Vec<String>,
    /// Mod deployment directory relative to game path (e.g. `"Data"`, `"Mods"`, `"."`).
    pub query_mod_path: String,
    /// Whether mods merge into a single directory.
    #[serde(default = "default_true")]
    pub merge_mods: bool,
    /// Store IDs for game detection.
    #[serde(default)]
    pub store_ids: StoreIds,
    /// Additional details from the extension (hashFiles, ignoreConflicts, etc.).
    #[serde(default)]
    pub details: serde_json::Value,
    /// Environment variables (e.g. SteamAPPId).
    #[serde(default)]
    pub environment: serde_json::Value,
    /// Tools associated with this game.
    #[serde(default)]
    pub supported_tools: Vec<VortexTool>,
    /// Custom mod types with alternative install paths.
    #[serde(default)]
    pub mod_types: Vec<VortexModType>,
    /// Custom installer metadata.
    #[serde(default)]
    pub installers: Vec<VortexInstallerMeta>,
    /// Whether this is a stub (registerGameStub) with limited data.
    #[serde(default)]
    pub is_stub: bool,
    /// Name of the Steam directory (if different from display name).
    pub steam_dir_name: Option<String>,
}

fn default_true() -> bool {
    true
}

/// All data captured from executing a Vortex extension.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CapturedRegistrations {
    pub game: Option<VortexGameRegistration>,
    pub mod_types: Vec<VortexModType>,
    pub installers: Vec<VortexInstallerMeta>,
}

/// Raw extension source files fetched from GitHub.
pub struct ExtensionSource {
    /// The main index.js content.
    pub index_js: String,
    /// The info.json content (if available).
    pub info_json: Option<String>,
    /// SHA256 hash of index.js for cache invalidation.
    pub source_hash: String,
    /// Additional JS files (for relative requires).
    pub extra_files: std::collections::HashMap<String, String>,
}

/// Summary of a cached extension for UI display.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExtensionSummary {
    pub game_id: String,
    pub name: String,
    pub version: Option<String>,
    pub is_stub: bool,
    pub fetched_at: String,
    pub tool_count: usize,
    pub mod_type_count: usize,
}
