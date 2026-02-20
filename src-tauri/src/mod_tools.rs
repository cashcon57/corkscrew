//! Mod tools registry — detection and launching of common modding tools.
//!
//! Provides a registry of known modding tools (SSEEdit, BethINI, DynDOLOD, etc.)
//! with automatic detection and Wine-based launching.

use std::path::Path;

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModTool {
    pub id: String,
    pub name: String,
    pub description: String,
    pub exe_names: Vec<String>,
    pub detected_path: Option<String>,
    pub requires_wine: bool,
    pub category: String,
}

/// Built-in tool definitions for Skyrim SE modding.
fn builtin_tools() -> Vec<ModTool> {
    vec![
        ModTool {
            id: "sseedit".into(),
            name: "SSEEdit (xEdit)".into(),
            description: "Plugin cleaning and conflict resolution".into(),
            exe_names: vec![
                "SSEEdit.exe".into(),
                "SSEEdit64.exe".into(),
                "xEdit.exe".into(),
            ],
            detected_path: None,
            requires_wine: true,
            category: "Cleaning".into(),
        },
        ModTool {
            id: "bethini".into(),
            name: "BethINI / BethINI Pie".into(),
            description: "INI configuration optimizer".into(),
            exe_names: vec!["BethINI.exe".into(), "Bethini Pie.exe".into()],
            detected_path: None,
            requires_wine: true,
            category: "INI".into(),
        },
        ModTool {
            id: "dyndolod".into(),
            name: "DynDOLOD".into(),
            description: "Dynamic LOD generation".into(),
            exe_names: vec!["DynDOLOD.exe".into(), "DynDOLODx64.exe".into()],
            detected_path: None,
            requires_wine: true,
            category: "LOD".into(),
        },
        ModTool {
            id: "bodyslide".into(),
            name: "BodySlide & Outfit Studio".into(),
            description: "Body and outfit customization".into(),
            exe_names: vec!["BodySlide.exe".into(), "OutfitStudio.exe".into()],
            detected_path: None,
            requires_wine: true,
            category: "Body".into(),
        },
        ModTool {
            id: "nemesis".into(),
            name: "Nemesis".into(),
            description: "Animation engine (FNIS replacement)".into(),
            exe_names: vec!["Nemesis Unlimited Behavior Engine.exe".into()],
            detected_path: None,
            requires_wine: true,
            category: "Animation".into(),
        },
        ModTool {
            id: "fnis".into(),
            name: "FNIS".into(),
            description: "Legacy animation framework".into(),
            exe_names: vec!["GenerateFNISforUsers.exe".into()],
            detected_path: None,
            requires_wine: true,
            category: "Animation".into(),
        },
        ModTool {
            id: "wryebash".into(),
            name: "Wrye Bash".into(),
            description: "Bashed patch creation".into(),
            exe_names: vec!["Wrye Bash.exe".into()],
            detected_path: None,
            requires_wine: true,
            category: "Patching".into(),
        },
        ModTool {
            id: "cao".into(),
            name: "Cathedral Assets Optimizer".into(),
            description: "Texture and mesh optimization".into(),
            exe_names: vec!["Cathedral Assets Optimizer.exe".into()],
            detected_path: None,
            requires_wine: true,
            category: "Optimization".into(),
        },
    ]
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Scan the game data directory (and parent) for known modding tools.
/// Returns the list of built-in tools with `detected_path` populated where found.
pub fn detect_tools(game_data_dir: &Path) -> Vec<ModTool> {
    let mut tools = builtin_tools();

    // Directories to search: game dir, parent of game dir, and common tool locations
    let game_dir = game_data_dir.parent().unwrap_or(game_data_dir);
    let search_roots: Vec<&Path> = vec![game_dir, game_data_dir];

    // Build a case-insensitive exe name set for matching
    let exe_names_lower: Vec<Vec<String>> = tools
        .iter()
        .map(|t| t.exe_names.iter().map(|n| n.to_lowercase()).collect())
        .collect();

    // Walk max 3 levels deep to keep it fast
    for root in &search_roots {
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(root)
            .max_depth(3)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let file_name = entry.file_name().to_string_lossy().to_lowercase();
            for (i, names) in exe_names_lower.iter().enumerate() {
                if tools[i].detected_path.is_some() {
                    continue; // Already found
                }
                if names.iter().any(|n| n == &file_name) {
                    tools[i].detected_path = Some(entry.path().to_string_lossy().to_string());
                }
            }
        }
    }

    tools
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_tools_not_empty() {
        let tools = builtin_tools();
        assert!(tools.len() >= 8);
        assert!(tools.iter().any(|t| t.id == "sseedit"));
        assert!(tools.iter().any(|t| t.id == "bethini"));
    }

    #[test]
    fn test_detect_tools_no_crash_on_missing_dir() {
        let tools = detect_tools(Path::new("/nonexistent/path"));
        assert!(tools.iter().all(|t| t.detected_path.is_none()));
    }
}
