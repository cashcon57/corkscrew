//! Baldur's Gate 3 (native macOS) game plugin.
//!
//! Detects Larian's macOS-native BG3 build (shipped Sept 2024) and
//! provides game-specific metadata for .pak-based mod management. The
//! native install on mac differs from Windows in path conventions:
//! mods live at `~/Library/Application Support/Larian Studios/
//! Baldur's Gate 3/Mods/` (NOT inside the .app bundle), and the load
//! order is encoded as `<region id="ModuleSettings">` in
//! `modsettings.lsx`.
//!
//! Real detection (Task 4.2+), .pak parsing (Task 4.3), deploy +
//! modsettings.lsx editing (Task 4.4), and load-order UI (Task 4.5)
//! all build on this scaffold.

use std::path::{Path, PathBuf};

use crate::bottles::Bottle;
use crate::games::{DetectedGame, GamePlugin};

/// Game plugin for Baldur's Gate 3 (native macOS).
///
/// BG3 ships a macOS-native build via Steam. Mods are distributed as
/// `.pak` files and are managed through `modsettings.lsx` (an XML-based
/// load-order manifest), not a plain-text `plugins.txt`. The actual
/// mods directory on macOS is
/// `~/Library/Application Support/Larian Studios/Baldur's Gate 3/Mods/`
/// — independent of the .app bundle location. The `get_data_dir`
/// implementation below uses `<game_path>/Mods` as a placeholder until
/// the Task 4.0 spike finalises the abstraction needed to handle the
/// Library-rooted path.
pub struct BaldursGate3NativePlugin;

/// Candidate executable names for BG3 on macOS.
///
/// The authoritative macOS executable name is an open spike question
/// (Task 4.0). These names cover the known possibilities:
/// - `"Baldur's Gate 3"` — likely launcher / wrapper name
/// - `"bg3"` — Vulkan backend executable (known from Linux)
/// - `"bg3_dx11"` — DX11-compat backend (placeholder; may not exist on macOS)
///
/// Refine after Task 4.0 spike returns confirmed macOS executable names.
const EXECUTABLES: &[&str] = &["Baldur's Gate 3", "bg3", "bg3_dx11"];

impl GamePlugin for BaldursGate3NativePlugin {
    fn game_id(&self) -> &str {
        "baldurs_gate_3_native"
    }

    fn display_name(&self) -> &str {
        "Baldur's Gate 3 (Native)"
    }

    fn nexus_slug(&self) -> &str {
        "baldursgate3"
    }

    fn executables(&self) -> &[&str] {
        EXECUTABLES
    }

    /// Detection is a stub for this scaffold task.
    ///
    /// Real detection (bundle identifier lookup, Steam appmanifest scan)
    /// arrives in Task 4.2. Returns empty until then.
    fn detect_native(&self) -> Vec<DetectedGame> {
        Vec::new()
    }

    /// Returns the BG3 mods directory relative to `game_path`.
    ///
    /// Placeholder: returns `<game_path>/Mods`. The real BG3 mods directory
    /// on macOS is `~/Library/Application Support/Larian Studios/Baldur's
    /// Gate 3/Mods/`, which is NOT a function of the .app bundle path.
    /// The abstraction will be revised once the Task 4.0 spike confirms
    /// the correct path model.
    fn get_data_dir(&self, game_path: &Path) -> PathBuf {
        game_path.join("Mods")
    }

    /// BG3 has no `plugins.txt` load-order file.
    ///
    /// Load order is stored as XML in `modsettings.lsx`
    /// (`<region id="ModuleSettings">`). Access to that file requires a
    /// different abstraction; see Task 4.5.
    fn get_plugins_file(&self, _game_path: &Path, _bottle: &Bottle) -> Option<PathBuf> {
        None
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub fn register() {
    crate::games::register_plugin(std::sync::Arc::new(BaldursGate3NativePlugin));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::with_plugin;

    #[test]
    fn bg3_native_plugin_registers() {
        crate::games::register_plugin(std::sync::Arc::new(BaldursGate3NativePlugin));
        let result = with_plugin("baldurs_gate_3_native", |p| p.display_name().to_owned());
        assert_eq!(result, Some("Baldur's Gate 3 (Native)".to_owned()));
    }

    #[test]
    fn bg3_get_data_dir_returns_mods_subfolder() {
        let plugin = BaldursGate3NativePlugin;
        let p = Path::new("/Applications/Baldurs Gate 3.app/Contents/MacOS");
        assert_eq!(plugin.get_data_dir(p), p.join("Mods"));
    }

    #[test]
    fn bg3_detect_native_is_empty_in_scaffold() {
        let plugin = BaldursGate3NativePlugin;
        assert!(plugin.detect_native().is_empty());
    }
}
