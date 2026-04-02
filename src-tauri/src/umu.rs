//! UMU (Universal Game Launcher) integration for Linux/Steam Deck.
//!
//! When `umu-run` is available, Corkscrew can route game launches through
//! Valve's Proton runtime with automatic protonfixes, pressure-vessel
//! containerization, and the umu-database game ID mapping.
//!
//! Reference: https://github.com/Open-Wine-Components/umu-launcher

use std::path::PathBuf;
use std::process::Command;

use log::info;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Status of UMU availability on the system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UmuStatus {
    /// Whether umu-run was found on the system.
    pub available: bool,
    /// Path to the umu-run binary, if found.
    pub binary_path: Option<String>,
    /// UMU version string, if detectable.
    pub version: Option<String>,
    /// Whether UMU is recommended for this system (Linux only).
    pub recommended: bool,
}

/// UMU game ID mapping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UmuGameId {
    /// The UMU game identifier (e.g., "umu-489830" for Skyrim SE).
    pub umu_id: String,
    /// The store this game was purchased from.
    pub store: String,
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Detect whether umu-run is available on the system.
///
/// Searches PATH and known installation locations.
pub fn detect_umu() -> UmuStatus {
    // UMU is Linux-only
    if cfg!(not(target_os = "linux")) {
        return UmuStatus {
            available: false,
            binary_path: None,
            version: None,
            recommended: false,
        };
    }

    // Search for umu-run in PATH
    let binary_path = find_umu_binary();
    let available = binary_path.is_some();

    let version = binary_path.as_ref().and_then(|path| {
        Command::new(path)
            .arg("--version")
            .output()
            .ok()
            .and_then(|o| {
                let stdout = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if stdout.is_empty() { None } else { Some(stdout) }
            })
    });

    UmuStatus {
        available,
        binary_path: binary_path.map(|p| p.to_string_lossy().into_owned()),
        version,
        recommended: available, // If available on Linux, recommend it
    }
}

/// Find the umu-run binary in PATH or known locations.
fn find_umu_binary() -> Option<PathBuf> {
    // Check PATH first
    if let Ok(output) = Command::new("which").arg("umu-run").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
    }

    // Known installation locations
    let known_paths = [
        PathBuf::from("/usr/bin/umu-run"),
        PathBuf::from("/usr/local/bin/umu-run"),
        dirs::home_dir()
            .map(|h| h.join(".local/bin/umu-run"))
            .unwrap_or_default(),
        dirs::home_dir()
            .map(|h| h.join(".local/share/umu/umu-run"))
            .unwrap_or_default(),
    ];

    known_paths.into_iter().find(|p| p.exists())
}

// ---------------------------------------------------------------------------
// Game ID mapping
// ---------------------------------------------------------------------------

/// Map a Corkscrew game_id to a UMU game identifier.
///
/// UMU uses Steam AppIDs prefixed with "umu-" as identifiers.
/// This allows protonfixes to look up and apply per-game fixes.
pub fn game_to_umu_id(game_id: &str, steam_app_id: Option<u32>) -> Option<UmuGameId> {
    // If we have a Steam AppID, use it directly
    if let Some(app_id) = steam_app_id {
        return Some(UmuGameId {
            umu_id: format!("umu-{}", app_id),
            store: "steam".into(),
        });
    }

    // Fallback: known game mappings
    let (umu_id, store) = match game_id {
        "skyrimse" => ("umu-489830", "steam"),
        "skyrim" => ("umu-72850", "steam"),
        "skyrimvr" => ("umu-611670", "steam"),
        "fallout4" => ("umu-377160", "steam"),
        "fallout4vr" => ("umu-611660", "steam"),
        "falloutnv" => ("umu-22380", "steam"),
        "fallout3" => ("umu-22300", "steam"),
        "oblivion" => ("umu-22330", "steam"),
        "morrowind" => ("umu-22320", "steam"),
        "starfield" => ("umu-1716740", "steam"),
        "enderal" => ("umu-976620", "steam"),
        _ => return None,
    };

    Some(UmuGameId {
        umu_id: umu_id.to_string(),
        store: store.to_string(),
    })
}

/// Build the environment variables needed to launch a game through umu-run.
///
/// The returned vars should be set before calling `umu-run <exe_path>`.
pub fn build_umu_env(
    umu_game_id: &UmuGameId,
    proton_path: Option<&str>,
) -> Vec<(String, String)> {
    let mut env = vec![
        ("GAMEID".into(), umu_game_id.umu_id.clone()),
        ("STORE".into(), umu_game_id.store.clone()),
    ];

    // Optionally specify a Proton version
    if let Some(proton) = proton_path {
        env.push(("PROTONPATH".into(), proton.into()));
    }

    info!(
        "UMU launch env: GAMEID={} STORE={}",
        umu_game_id.umu_id, umu_game_id.store
    );

    env
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_mapping_covers_bethesda_titles() {
        assert!(game_to_umu_id("skyrimse", None).is_some());
        assert!(game_to_umu_id("fallout4", None).is_some());
        assert!(game_to_umu_id("oblivion", None).is_some());
        assert!(game_to_umu_id("morrowind", None).is_some());

        let sse = game_to_umu_id("skyrimse", None).unwrap();
        assert_eq!(sse.umu_id, "umu-489830");
        assert_eq!(sse.store, "steam");
    }

    #[test]
    fn game_mapping_returns_none_for_unknown() {
        assert!(game_to_umu_id("unknowngame", None).is_none());
    }

    #[test]
    fn steam_app_id_overrides_builtin() {
        let result = game_to_umu_id("skyrimse", Some(99999));
        assert_eq!(result.unwrap().umu_id, "umu-99999");
    }

    #[test]
    fn build_env_includes_gameid_and_store() {
        let game_id = UmuGameId {
            umu_id: "umu-489830".into(),
            store: "steam".into(),
        };
        let env = build_umu_env(&game_id, None);
        assert!(env.iter().any(|(k, v)| k == "GAMEID" && v == "umu-489830"));
        assert!(env.iter().any(|(k, v)| k == "STORE" && v == "steam"));
    }

    #[test]
    fn build_env_with_proton_path() {
        let game_id = UmuGameId {
            umu_id: "umu-489830".into(),
            store: "steam".into(),
        };
        let env = build_umu_env(&game_id, Some("/path/to/proton"));
        assert!(env.iter().any(|(k, v)| k == "PROTONPATH" && v == "/path/to/proton"));
    }

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn detect_umu_not_available_on_non_linux() {
        let status = detect_umu();
        assert!(!status.available);
        assert!(!status.recommended);
    }
}
