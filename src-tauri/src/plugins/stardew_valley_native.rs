//! Stardew Valley (native macOS) game plugin.
//!
//! Detects ConcernedApe's macOS-native Stardew Valley install and
//! provides game-specific metadata for SMAPI mod management. The actual
//! detection logic walks `native_scanner` results in Task 3.2; this
//! task is the plugin scaffold.

use std::path::{Path, PathBuf};

use crate::bottles::Bottle;
use crate::games::{DetectedGame, GamePlugin};

/// Game plugin for Stardew Valley (native macOS).
///
/// Stardew Valley ships a native universal binary (Apple Silicon +
/// Intel) on Steam and GOG. Mods are managed via SMAPI which patches
/// the `Contents/MacOS/StardewValley` launcher script in the .app
/// bundle to invoke a SMAPI-aware launcher that loads mods from
/// `Contents/MacOS/Mods/`.
pub struct StardewValleyNativePlugin;

const EXECUTABLES: &[&str] = &["StardewValley"];

impl GamePlugin for StardewValleyNativePlugin {
    fn game_id(&self) -> &str {
        "stardew_valley_native"
    }

    fn display_name(&self) -> &str {
        "Stardew Valley (Native)"
    }

    fn nexus_slug(&self) -> &str {
        "stardewvalley"
    }

    fn executables(&self) -> &[&str] {
        EXECUTABLES
    }

    fn detect_native(&self) -> Vec<DetectedGame> {
        // Real impl lands in Task 3.2. Empty vec for the scaffold.
        Vec::new()
    }

    fn get_data_dir(&self, game_path: &Path) -> PathBuf {
        // SMAPI loads mods from <game_path>/Mods (which is
        // <app_bundle>/Contents/MacOS/Mods on the native install).
        game_path.join("Mods")
    }

    fn get_plugins_file(&self, _game_path: &Path, _bottle: &Bottle) -> Option<PathBuf> {
        // Stardew has no plugins.txt-style load order; SMAPI loads all
        // mod folders alphabetically.
        None
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub fn register() {
    crate::games::register_plugin(Box::new(StardewValleyNativePlugin));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::with_plugin;

    #[test]
    fn stardew_valley_native_plugin_registers() {
        crate::games::register_plugin(Box::new(StardewValleyNativePlugin));
        let result = with_plugin("stardew_valley_native", |p| p.display_name().to_owned());
        assert_eq!(result, Some("Stardew Valley (Native)".to_owned()));
    }

    #[test]
    fn stardew_get_data_dir_returns_mods_subfolder() {
        let plugin = StardewValleyNativePlugin;
        let game_path = Path::new("/Applications/Stardew Valley.app/Contents/MacOS");
        assert_eq!(plugin.get_data_dir(game_path), game_path.join("Mods"));
    }

    #[test]
    fn stardew_detect_native_is_empty_in_scaffold() {
        let plugin = StardewValleyNativePlugin;
        assert!(plugin.detect_native().is_empty());
    }
}
