//! Game detection within Wine bottles.
//!
//! Ported from legacy-python/games.py. Provides a trait-based plugin system
//! for detecting games inside Wine bottles managed by CrossOver, Whisky, etc.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::bottles::Bottle;

// ---------------------------------------------------------------------------
// GamePlugin trait
// ---------------------------------------------------------------------------

/// Interface for game-specific mod management logic.
///
/// Each supported game implements this trait and registers itself with the
/// global plugin registry via [`register_plugin`]. The trait is object-safe
/// and requires `Send + Sync` so trait objects can be shared across Tauri's
/// async runtime threads.
pub trait GamePlugin: Send + Sync {
    /// Unique identifier for this game (e.g. `"skyrimse"`).
    fn game_id(&self) -> &str;

    /// Human-readable name shown in the UI (e.g. `"Skyrim Special Edition"`).
    fn display_name(&self) -> &str;

    /// Nexus Mods slug used in API requests (e.g. `"skyrimspecialedition"`).
    fn nexus_slug(&self) -> &str;

    /// Executable filenames used to verify a game installation.
    fn executables(&self) -> &[&str];

    /// Attempt to locate this game inside `bottle`. Returns `Some(DetectedGame)`
    /// if the game is found, or `None` otherwise.
    fn detect(&self, bottle: &Bottle) -> Option<DetectedGame>;

    /// Return the directory where mods should be deployed for a given
    /// `game_path` (e.g. `<game_path>/Data` for Bethesda titles).
    fn get_data_dir(&self, game_path: &Path) -> PathBuf;

    /// Return the path to the plugin load-order file (e.g. `plugins.txt`),
    /// if applicable for this game. Not all games use plugin files.
    fn get_plugins_file(&self, game_path: &Path, bottle: &Bottle) -> Option<PathBuf>;

    /// Return the directory where game saves are stored, if known.
    /// Used for per-profile save management.
    fn get_saves_dir(&self, _game_path: &Path, _bottle: &Bottle) -> Option<PathBuf> {
        None // Default: game doesn't have a known saves directory
    }

    /// Return Vortex-sourced tool definitions, if this plugin was loaded from
    /// a Vortex extension. Non-Vortex plugins return an empty list.
    fn vortex_tools(&self) -> Vec<crate::vortex_types::VortexTool> {
        vec![]
    }

    /// Return Vortex-registered mod types for this game.
    ///
    /// Each mod type maps a type ID (e.g. `"w3modRoot"`) to a target path
    /// relative to the game directory. Used by the collection installer to
    /// route mods to the correct subdirectory.
    fn vortex_mod_types(&self) -> Vec<crate::vortex_types::VortexModType> {
        vec![]
    }

    /// Auto-detect mod type from staged file list. Returns the mod type ID
    /// (matching an entry from [`vortex_mod_types`]) if detection succeeds.
    /// Used during manual installs to route mods to the correct directory.
    fn detect_mod_type_from_files(&self, _files: &[String]) -> Option<String> {
        None
    }

    /// Detect the installed game version, if possible.
    /// Returns a version string (e.g. `"1233043"` for HL).
    fn detect_game_version(&self, _game_path: &Path) -> Option<String> {
        None
    }

    /// Post-deploy hook called after a mod is deployed. Game plugins can use
    /// this to update game-specific config files (e.g. Mods.txt for HL Lua mods).
    fn on_mod_deployed(
        &self,
        _game_path: &Path,
        _mod_type: Option<&str>,
        _deployed_files: &[String],
    ) {
    }

    /// Post-undeploy hook called after a mod is undeployed.
    fn on_mod_undeployed(
        &self,
        _game_path: &Path,
        _mod_type: Option<&str>,
        _undeployed_files: &[String],
    ) {
    }

    /// Return a Steam App ID for protocol launch (`steam://rungameid/{id}`).
    /// When set, the launcher uses the Steam protocol URL instead of launching
    /// the exe directly, avoiding Steam's "custom arguments" dialog.
    /// Return `None` to launch the exe directly (default).
    fn steam_launch_id(&self) -> Option<&str> {
        None
    }

    /// Return the executable to use for game launch, if different from the
    /// detection executable. Some games (e.g. Hogwarts Legacy) have a root
    /// launcher stub that invokes Steam/DRM, while the real game binary is
    /// in a subdirectory. Detection uses the real binary, but launch must
    /// use the root launcher for DRM to work.
    /// Return `None` to use the detected exe_path (default behavior).
    fn launch_executable(&self, _game_path: &Path) -> Option<PathBuf> {
        None
    }

    // -- Game directory cleaner support --

    /// File names (case-insensitive) that must NEVER be deleted by the cleaner.
    /// These are critical game files like master ESMs or core PAK files.
    /// Default: empty (rely on snapshot comparison only).
    fn critical_files(&self) -> Vec<&str> {
        vec![]
    }

    /// File extensions (case-insensitive, including the dot) that should never
    /// be deleted from the root of the data directory. Files in subdirectories
    /// with these extensions are NOT protected.
    /// Default: empty.
    fn protected_root_extensions(&self) -> Vec<&str> {
        vec![]
    }

    /// Patterns for detecting save files in the data directory
    /// (case-insensitive). Extension patterns start with `.` (e.g. `".ess"`),
    /// directory patterns end with `/` (e.g. `"saves/"`).
    /// Default: empty (saves assumed to be outside data dir).
    fn save_file_patterns(&self) -> Vec<&str> {
        vec![]
    }

    /// Categorize a mod file by its relative path within the data directory.
    /// Returns a category string (e.g. `"plugin"`, `"texture"`, `"pak"`)
    /// used by the cleaner for selective removal. Return `None` to fall
    /// through to the default categorizer.
    fn categorize_mod_file(&self, _rel_path: &str) -> Option<String> {
        None
    }
}

// ---------------------------------------------------------------------------
// DetectedGame
// ---------------------------------------------------------------------------

/// A game found inside a Wine bottle.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DetectedGame {
    /// Identifier matching [`GamePlugin::game_id`].
    pub game_id: String,
    /// Human-readable name matching [`GamePlugin::display_name`].
    pub display_name: String,
    /// Nexus Mods slug matching [`GamePlugin::nexus_slug`].
    pub nexus_slug: String,
    /// Absolute path to the game installation directory inside the bottle.
    pub game_path: PathBuf,
    /// Absolute path to the main game executable (e.g. SkyrimSE.exe).
    pub exe_path: Option<PathBuf>,
    /// Absolute path to the data/mod deployment directory.
    pub data_dir: PathBuf,
    /// Name of the bottle containing this game.
    pub bottle_name: String,
    /// Absolute path to the bottle root.
    pub bottle_path: PathBuf,
}

// ---------------------------------------------------------------------------
// Plugin registry
// ---------------------------------------------------------------------------

/// Thread-safe storage for registered game plugins.
type PluginRegistry = Mutex<Vec<Box<dyn GamePlugin + Send + Sync>>>;

/// Global plugin registry, initialised on first access.
fn registry() -> &'static PluginRegistry {
    static REGISTRY: OnceLock<PluginRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Vec::new()))
}

/// Register a game plugin with the global registry.
///
/// Plugins are typically registered at application startup. Duplicate
/// registrations (same `game_id`) are silently ignored.
pub fn register_plugin(plugin: Box<dyn GamePlugin + Send + Sync>) {
    let mut plugins = registry().lock().unwrap_or_else(|e| e.into_inner());
    // Prevent duplicate registrations.
    let id = plugin.game_id().to_owned();
    if plugins.iter().any(|p| p.game_id() == id) {
        return;
    }
    plugins.push(plugin);
}

/// Scan a single bottle for all recognized games.
///
/// Runs registered game plugins first, then scans Steam appmanifest files
/// to pick up any installed games that lack a dedicated plugin or registry entry.
pub fn detect_games(bottle: &Bottle) -> Vec<DetectedGame> {
    let plugins = registry().lock().unwrap_or_else(|e| e.into_inner());
    let mut found = Vec::new();
    for plugin in plugins.iter() {
        if let Some(detected) = plugin.detect(bottle) {
            found.push(detected);
        }
    }
    drop(plugins); // release lock before scanning appmanifests

    // Discover any Steam games not covered by registered plugins
    let unregistered = crate::game_registry::detect_unregistered_steam_games(bottle, &found);
    found.extend(unregistered);

    found
}

/// Scan **all** discoverable bottles for all recognized games.
pub fn detect_all_games() -> Vec<DetectedGame> {
    use crate::bottles::detect_bottles;

    let mut found = Vec::new();
    for bottle in detect_bottles() {
        found.extend(detect_games(&bottle));
    }
    found
}

/// Scan all bottles and include custom (user-added) games from the database.
pub fn detect_all_games_with_custom(db: &crate::database::ModDatabase) -> Vec<DetectedGame> {
    let mut found = detect_all_games();

    // Add custom games from DB
    let custom = crate::game_registry::load_custom_games(db);
    for cg in custom {
        // Don't duplicate if auto-detection already found it
        if !found.iter().any(|g| g.game_id == cg.game_id) {
            found.push(cg);
        }
    }

    // Auto-capture Steam depot manifests for all detected games.
    // This builds a local version history so we can offer automated
    // downgrades when a collection requires an older game version.
    for game in &found {
        let bottle = crate::bottles::Bottle {
            name: game.bottle_name.clone(),
            path: game.bottle_path.clone(),
            source: String::new(),
        };
        crate::game_registry::capture_depot_manifests(db, &game.game_id, &bottle);
    }

    found
}

/// Look up a registered plugin by its game id.
///
/// The returned reference borrows from the `MutexGuard`, so the caller
/// cannot hold it across an await point. For short-lived, synchronous
/// lookups this is fine; if you need to call methods on the plugin later,
/// copy the data you need while the lock is held.
///
/// # Usage
///
/// ```ignore
/// if let Some(result) = with_plugin("skyrimse", |plugin| {
///     plugin.display_name().to_owned()
/// }) {
///     println!("Found: {result}");
/// }
/// ```
pub fn with_plugin<F, R>(game_id: &str, f: F) -> Option<R>
where
    F: FnOnce(&dyn GamePlugin) -> R,
{
    let plugins = registry().lock().unwrap_or_else(|e| e.into_inner());
    plugins
        .iter()
        .find(|p| p.game_id() == game_id)
        .map(|p| f(p.as_ref()))
}

/// Look up a registered plugin by its game id and return a reference to it.
///
/// **Important**: This acquires the registry mutex lock. The returned guard
/// keeps the lock held until it is dropped. Prefer [`with_plugin`] for
/// scoped access, or copy data out quickly.
pub fn get_plugin_for_game(game_id: &str) -> Option<PluginRef> {
    let plugins = registry().lock().unwrap_or_else(|e| e.into_inner());
    // Check if any plugin matches before constructing the ref.
    let index = plugins.iter().position(|p| p.game_id() == game_id)?;
    Some(PluginRef {
        guard: plugins,
        index,
    })
}

/// A handle that keeps the registry lock held while providing access to a
/// single plugin. Dereferences to `&dyn GamePlugin`.
pub struct PluginRef {
    guard: std::sync::MutexGuard<'static, Vec<Box<dyn GamePlugin + Send + Sync>>>,
    index: usize,
}

impl std::ops::Deref for PluginRef {
    type Target = dyn GamePlugin;

    fn deref(&self) -> &Self::Target {
        self.guard[self.index].as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Minimal plugin used only in tests.
    struct TestPlugin;

    impl GamePlugin for TestPlugin {
        fn game_id(&self) -> &str {
            "testgame"
        }
        fn display_name(&self) -> &str {
            "Test Game"
        }
        fn nexus_slug(&self) -> &str {
            "testgame"
        }
        fn executables(&self) -> &[&str] {
            &["test.exe"]
        }
        fn detect(&self, _bottle: &Bottle) -> Option<DetectedGame> {
            None
        }
        fn get_data_dir(&self, game_path: &Path) -> PathBuf {
            game_path.join("Data")
        }
        fn get_plugins_file(&self, _game_path: &Path, _bottle: &Bottle) -> Option<PathBuf> {
            None
        }
    }

    #[test]
    fn register_and_lookup() {
        register_plugin(Box::new(TestPlugin));
        let result = with_plugin("testgame", |p| p.display_name().to_owned());
        assert_eq!(result, Some("Test Game".to_owned()));
    }

    #[test]
    fn duplicate_registration_ignored() {
        register_plugin(Box::new(TestPlugin));
        register_plugin(Box::new(TestPlugin));
        let plugins = registry().lock().unwrap_or_else(|e| e.into_inner());
        let count = plugins.iter().filter(|p| p.game_id() == "testgame").count();
        assert_eq!(count, 1);
    }

    #[test]
    fn unknown_game_returns_none() {
        assert!(with_plugin("nonexistent", |p| p.game_id().to_owned()).is_none());
    }
}
