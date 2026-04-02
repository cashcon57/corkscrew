//! Headless execution configuration for modding tools under Wine.
//!
//! Builds command-line configurations for automatable modding tasks (xEdit
//! QuickAutoClean, BodySlide BatchBuild, Synthesis RunPatcher, Wrye Bash
//! BuildPatch). Does NOT execute Wine commands directly — callers should pass
//! the resulting [`TaskConfig`] to `mod_tools::launch_tool_with_logging()`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::wine_dll_overrides::{DllOverride, get_tool_overrides};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A modding task that can be run headlessly (no GUI interaction required).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AutomatableTask {
    /// xEdit QuickAutoClean — clean dirty plugin records.
    XEditQuickAutoClean {
        /// The plugin filename to clean (e.g. "Update.esm").
        plugin_name: String,
    },
    /// BodySlide BatchBuild — generate all body/outfit meshes.
    BodySlideBatchBuild,
    /// Synthesis RunPatcher — execute a Synthesis patcher profile.
    SynthesisRunPatcher {
        /// Profile name to run (e.g. "Default").
        profile_name: String,
    },
    /// Wrye Bash BuildPatch — generate a Bashed Patch.
    WryeBashBuildPatch,
}

impl AutomatableTask {
    /// Human-readable display name for the task.
    pub fn display_name(&self) -> &str {
        match self {
            Self::XEditQuickAutoClean { .. } => "xEdit QuickAutoClean",
            Self::BodySlideBatchBuild => "BodySlide Batch Build",
            Self::SynthesisRunPatcher { .. } => "Synthesis Run Patcher",
            Self::WryeBashBuildPatch => "Wrye Bash Build Patch",
        }
    }

    /// The tool registry ID that corresponds to this task.
    pub fn tool_id(&self) -> &str {
        match self {
            Self::XEditQuickAutoClean { .. } => "sseedit",
            Self::BodySlideBatchBuild => "bodyslide",
            Self::SynthesisRunPatcher { .. } => "synthesis",
            Self::WryeBashBuildPatch => "wryebash",
        }
    }
}

/// Configuration for running a modding tool headlessly under Wine.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskConfig {
    /// Corkscrew tool registry ID (e.g. "sseedit", "bodyslide").
    pub tool_id: String,
    /// Game identifier (e.g. "skyrimse", "fallout4").
    pub game_id: String,
    /// Wine bottle name.
    pub bottle_name: String,
    /// Path to the tool executable.
    pub exe_path: PathBuf,
    /// Command-line arguments for headless operation.
    pub args: Vec<String>,
    /// Maximum time to allow the tool to run before considering it hung.
    pub timeout_secs: u64,
    /// Wine DLL overrides required for this tool.
    pub dll_overrides: HashMap<String, DllOverride>,
}

/// Result of a completed automation task.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskResult {
    /// Whether the task completed successfully (exit code 0).
    pub success: bool,
    /// Captured stdout from the process.
    pub stdout: String,
    /// Captured stderr from the process.
    pub stderr: String,
    /// Process exit code (None if killed by timeout or signal).
    pub exit_code: Option<i32>,
    /// Wall-clock duration of execution in milliseconds.
    pub duration_ms: u64,
}

// ---------------------------------------------------------------------------
// Task configuration builders
// ---------------------------------------------------------------------------

/// Default timeout for each task type.
fn default_timeout(task: &AutomatableTask) -> Duration {
    match task {
        AutomatableTask::XEditQuickAutoClean { .. } => Duration::from_secs(300),  // 5 min
        AutomatableTask::BodySlideBatchBuild => Duration::from_secs(600),         // 10 min
        AutomatableTask::SynthesisRunPatcher { .. } => Duration::from_secs(900),  // 15 min
        AutomatableTask::WryeBashBuildPatch => Duration::from_secs(600),          // 10 min
    }
}

/// Build the CLI arguments for a given automatable task.
fn build_args(task: &AutomatableTask, game_data_dir: &Path, game_id: &str) -> Vec<String> {
    match task {
        AutomatableTask::XEditQuickAutoClean { plugin_name } => {
            let mut args = vec![
                "-quickautoclean".to_string(),
                plugin_name.clone(),
            ];
            // xEdit needs to know the data directory
            args.push("-D:".to_string() + &game_data_dir.to_string_lossy());
            // Pass the game mode based on game_id
            if let Some(mode) = xedit_game_mode(game_id) {
                args.push(format!("-{}", mode));
            }
            args
        }
        AutomatableTask::BodySlideBatchBuild => {
            vec!["-gbuild".to_string()]
        }
        AutomatableTask::SynthesisRunPatcher { profile_name } => {
            vec![
                "run-patcher".to_string(),
                "--profile".to_string(),
                profile_name.clone(),
                "--data-folder-path".to_string(),
                game_data_dir.to_string_lossy().into_owned(),
            ]
        }
        AutomatableTask::WryeBashBuildPatch => {
            let mut args = vec![
                "--no-gui".to_string(),
                "--build-patch".to_string(),
            ];
            // Tell Wrye Bash where game data lives
            args.push("--gamePath".to_string());
            // Wrye Bash expects the game root (parent of Data/)
            let game_root = game_data_dir
                .parent()
                .unwrap_or(game_data_dir);
            args.push(game_root.to_string_lossy().into_owned());
            args
        }
    }
}

/// Map game ID to xEdit's internal game mode flag.
fn xedit_game_mode(game_id: &str) -> Option<&'static str> {
    match game_id {
        "skyrimse" | "skyrimspecialedition" => Some("sse"),
        "skyrim" => Some("tes5"),
        "fallout4" => Some("fo4"),
        "falloutnv" | "falloutnewvegas" => Some("fnv"),
        "fallout3" => Some("fo3"),
        "oblivion" => Some("tes4"),
        _ => None,
    }
}

/// Build a complete [`TaskConfig`] for the given automatable task.
///
/// # Arguments
///
/// * `task` — The automation task to configure.
/// * `game_data_dir` — Path to the game's Data directory (inside the Wine prefix).
/// * `game_id` — Corkscrew game identifier (e.g. "skyrimse").
/// * `bottle_name` — Name of the Wine bottle.
/// * `exe_path` — Path to the tool's executable.
pub fn build_task_config(
    task: &AutomatableTask,
    game_data_dir: &Path,
    game_id: &str,
    bottle_name: &str,
    exe_path: &Path,
) -> TaskConfig {
    let args = build_args(task, game_data_dir, game_id);
    let dll_overrides = get_tool_overrides(task.tool_id());
    let timeout = default_timeout(task);

    TaskConfig {
        tool_id: task.tool_id().to_string(),
        game_id: game_id.to_string(),
        bottle_name: bottle_name.to_string(),
        exe_path: exe_path.to_path_buf(),
        args,
        timeout_secs: timeout.as_secs(),
        dll_overrides,
    }
}

// ---------------------------------------------------------------------------
// Detection heuristics
// ---------------------------------------------------------------------------

/// A detected task that should be run based on the installed mod list.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DetectedTask {
    /// The task that should be run.
    pub task: AutomatableTask,
    /// Human-readable reason why this task was detected as needed.
    pub reason: String,
    /// Mod names that triggered the detection.
    pub triggering_mods: Vec<String>,
}

/// Keywords that indicate BodySlide output is needed.
const BODYSLIDE_KEYWORDS: &[&str] = &[
    "body", "outfit", "armor", "cbbe", "bodyslide", "bhunp", "himbo",
    "3ba", "3bbb", "unp", "physics",
];

/// Keywords that indicate a Synthesis patcher is needed.
const SYNTHESIS_KEYWORDS: &[&str] = &[
    "synthesis", "patcher", "synthpatch",
];

/// Plugins known to have dirty records that benefit from xEdit cleaning.
/// This is a subset from the LOOT masterlist — the most commonly dirty
/// official master files across Bethesda games.
const KNOWN_DIRTY_PLUGINS: &[&str] = &[
    "update.esm",
    "dawnguard.esm",
    "hearthfires.esm",
    "dragonborn.esm",
    "unofficial skyrim special edition patch.esp",
    "dlcrobot.esm",
    "dlcworkshop01.esm",
    "dlcworkshop02.esm",
    "dlcworkshop03.esm",
    "dlccoast.esm",
    "dlcnukaworld.esm",
];

/// Analyze installed mods and return a list of automation tasks that should
/// be run.
///
/// This is a heuristic — it errs on the side of suggesting tasks rather than
/// missing them. The user should confirm before execution.
///
/// # Arguments
///
/// * `installed_mods` — Names of installed mods (from the mod database).
/// * `game_id` — Corkscrew game identifier.
pub fn detect_required_tasks(
    installed_mods: &[String],
    game_id: &str,
) -> Vec<DetectedTask> {
    let mut tasks = Vec::new();

    // --- BodySlide detection ---
    let bodyslide_triggers: Vec<String> = installed_mods
        .iter()
        .filter(|name| {
            let lower = name.to_lowercase();
            BODYSLIDE_KEYWORDS.iter().any(|kw| lower.contains(kw))
        })
        .cloned()
        .collect();

    if !bodyslide_triggers.is_empty() {
        tasks.push(DetectedTask {
            task: AutomatableTask::BodySlideBatchBuild,
            reason: format!(
                "Found {} mod(s) that likely require BodySlide output generation",
                bodyslide_triggers.len()
            ),
            triggering_mods: bodyslide_triggers,
        });
    }

    // --- Synthesis detection ---
    let synthesis_triggers: Vec<String> = installed_mods
        .iter()
        .filter(|name| {
            let lower = name.to_lowercase();
            SYNTHESIS_KEYWORDS.iter().any(|kw| lower.contains(kw))
        })
        .cloned()
        .collect();

    if !synthesis_triggers.is_empty() {
        tasks.push(DetectedTask {
            task: AutomatableTask::SynthesisRunPatcher {
                profile_name: "Default".to_string(),
            },
            reason: format!(
                "Found {} mod(s) referencing Synthesis patching",
                synthesis_triggers.len()
            ),
            triggering_mods: synthesis_triggers,
        });
    }

    // --- xEdit dirty plugin detection ---
    // Only applicable to Bethesda games
    let bethesda_games = [
        "skyrimse", "skyrimspecialedition", "skyrim", "fallout4",
        "falloutnv", "falloutnewvegas", "fallout3", "oblivion",
    ];
    if bethesda_games.contains(&game_id) {
        for dirty_plugin in KNOWN_DIRTY_PLUGINS {
            let plugin_lower = dirty_plugin.to_lowercase();
            // Check if any installed mod name matches the dirty plugin
            // (mods are often named after the plugin they contain)
            let matching: Vec<String> = installed_mods
                .iter()
                .filter(|name| name.to_lowercase().contains(&plugin_lower))
                .cloned()
                .collect();

            if !matching.is_empty() {
                tasks.push(DetectedTask {
                    task: AutomatableTask::XEditQuickAutoClean {
                        plugin_name: dirty_plugin.to_string(),
                    },
                    reason: format!(
                        "'{}' is known to have dirty records (LOOT masterlist)",
                        dirty_plugin
                    ),
                    triggering_mods: matching,
                });
            }
        }
    }

    tasks
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // -- AutomatableTask basics --

    #[test]
    fn test_task_display_names() {
        let tasks = [
            (AutomatableTask::XEditQuickAutoClean { plugin_name: "Update.esm".into() },
             "xEdit QuickAutoClean"),
            (AutomatableTask::BodySlideBatchBuild,
             "BodySlide Batch Build"),
            (AutomatableTask::SynthesisRunPatcher { profile_name: "Default".into() },
             "Synthesis Run Patcher"),
            (AutomatableTask::WryeBashBuildPatch,
             "Wrye Bash Build Patch"),
        ];
        for (task, expected) in &tasks {
            assert_eq!(task.display_name(), *expected);
        }
    }

    #[test]
    fn test_task_tool_ids() {
        assert_eq!(
            AutomatableTask::XEditQuickAutoClean { plugin_name: "x".into() }.tool_id(),
            "sseedit"
        );
        assert_eq!(AutomatableTask::BodySlideBatchBuild.tool_id(), "bodyslide");
        assert_eq!(
            AutomatableTask::SynthesisRunPatcher { profile_name: "x".into() }.tool_id(),
            "synthesis"
        );
        assert_eq!(AutomatableTask::WryeBashBuildPatch.tool_id(), "wryebash");
    }

    // -- build_task_config --

    #[test]
    fn test_xedit_config_skyrimse() {
        let data_dir = PathBuf::from("/bottles/Skyrim/drive_c/games/Skyrim/Data");
        let exe = PathBuf::from("/tools/SSEEdit.exe");
        let task = AutomatableTask::XEditQuickAutoClean {
            plugin_name: "Update.esm".to_string(),
        };

        let config = build_task_config(&task, &data_dir, "skyrimse", "Skyrim", &exe);

        assert_eq!(config.tool_id, "sseedit");
        assert_eq!(config.game_id, "skyrimse");
        assert_eq!(config.bottle_name, "Skyrim");
        assert!(config.args.contains(&"-quickautoclean".to_string()));
        assert!(config.args.contains(&"Update.esm".to_string()));
        // Should have -sse game mode flag
        assert!(config.args.iter().any(|a| a == "-sse"));
        // Should have data dir argument
        assert!(config.args.iter().any(|a| a.starts_with("-D:")));
        assert_eq!(config.timeout_secs, 300);
        // xEdit DLL overrides (comdlg32, etc.)
        assert!(config.dll_overrides.contains_key("comdlg32"));
    }

    #[test]
    fn test_xedit_config_fallout4() {
        let data_dir = PathBuf::from("/bottles/FO4/drive_c/games/Fallout4/Data");
        let exe = PathBuf::from("/tools/FO4Edit.exe");
        let task = AutomatableTask::XEditQuickAutoClean {
            plugin_name: "DLCRobot.esm".to_string(),
        };

        let config = build_task_config(&task, &data_dir, "fallout4", "FO4", &exe);

        assert!(config.args.iter().any(|a| a == "-fo4"));
    }

    #[test]
    fn test_bodyslide_config() {
        let data_dir = PathBuf::from("/bottles/Skyrim/drive_c/games/Skyrim/Data");
        let exe = PathBuf::from("/tools/BodySlide.exe");

        let config = build_task_config(
            &AutomatableTask::BodySlideBatchBuild,
            &data_dir,
            "skyrimse",
            "Skyrim",
            &exe,
        );

        assert_eq!(config.tool_id, "bodyslide");
        assert_eq!(config.args, vec!["-gbuild"]);
        assert_eq!(config.timeout_secs, 600);
        // BodySlide needs d3d11 native
        assert!(config.dll_overrides.contains_key("d3d11"));
    }

    #[test]
    fn test_synthesis_config() {
        let data_dir = PathBuf::from("/bottles/Skyrim/drive_c/games/Skyrim/Data");
        let exe = PathBuf::from("/tools/Synthesis.exe");

        let config = build_task_config(
            &AutomatableTask::SynthesisRunPatcher {
                profile_name: "MyProfile".to_string(),
            },
            &data_dir,
            "skyrimse",
            "Skyrim",
            &exe,
        );

        assert_eq!(config.tool_id, "synthesis");
        assert!(config.args.contains(&"run-patcher".to_string()));
        assert!(config.args.contains(&"--profile".to_string()));
        assert!(config.args.contains(&"MyProfile".to_string()));
        assert!(config.args.contains(&"--data-folder-path".to_string()));
        assert_eq!(config.timeout_secs, 900);
        // Synthesis needs mscoree native (.NET)
        assert!(config.dll_overrides.contains_key("mscoree"));
    }

    #[test]
    fn test_wryebash_config() {
        let data_dir = PathBuf::from("/bottles/Skyrim/drive_c/games/Skyrim/Data");
        let exe = PathBuf::from("/tools/Wrye Bash.exe");

        let config = build_task_config(
            &AutomatableTask::WryeBashBuildPatch,
            &data_dir,
            "skyrimse",
            "Skyrim",
            &exe,
        );

        assert_eq!(config.tool_id, "wryebash");
        assert!(config.args.contains(&"--no-gui".to_string()));
        assert!(config.args.contains(&"--build-patch".to_string()));
        assert!(config.args.contains(&"--gamePath".to_string()));
        // Game path should be parent of Data dir
        let game_root = "/bottles/Skyrim/drive_c/games/Skyrim";
        assert!(config.args.contains(&game_root.to_string()));
        assert_eq!(config.timeout_secs, 600);
    }

    // -- xedit_game_mode --

    #[test]
    fn test_xedit_game_modes() {
        assert_eq!(xedit_game_mode("skyrimse"), Some("sse"));
        assert_eq!(xedit_game_mode("skyrimspecialedition"), Some("sse"));
        assert_eq!(xedit_game_mode("skyrim"), Some("tes5"));
        assert_eq!(xedit_game_mode("fallout4"), Some("fo4"));
        assert_eq!(xedit_game_mode("falloutnv"), Some("fnv"));
        assert_eq!(xedit_game_mode("fallout3"), Some("fo3"));
        assert_eq!(xedit_game_mode("oblivion"), Some("tes4"));
        assert_eq!(xedit_game_mode("hogwartslegacy"), None);
    }

    // -- detect_required_tasks --

    #[test]
    fn test_detect_bodyslide_needed() {
        let mods = vec![
            "SkyUI".to_string(),
            "CBBE 3BBB Advanced".to_string(),
            "Immersive Armors".to_string(),
        ];

        let tasks = detect_required_tasks(&mods, "skyrimse");
        let bodyslide = tasks.iter().find(|t| t.task == AutomatableTask::BodySlideBatchBuild);
        assert!(bodyslide.is_some(), "Should detect BodySlide need from CBBE mod");
        let bs = bodyslide.unwrap();
        assert!(bs.triggering_mods.iter().any(|m| m.contains("CBBE")));
    }

    #[test]
    fn test_detect_bodyslide_multiple_triggers() {
        let mods = vec![
            "CBBE".to_string(),
            "HIMBO".to_string(),
            "Immersive Armors SE".to_string(),
            "BodySlide Presets".to_string(),
        ];

        let tasks = detect_required_tasks(&mods, "skyrimse");
        let bodyslide = tasks.iter().find(|t| t.task == AutomatableTask::BodySlideBatchBuild);
        assert!(bodyslide.is_some());
        // All four should trigger (cbbe, himbo, armor, bodyslide)
        assert!(bodyslide.unwrap().triggering_mods.len() >= 3);
    }

    #[test]
    fn test_detect_synthesis_needed() {
        let mods = vec![
            "SkyUI".to_string(),
            "Synthesis Output".to_string(),
            "USSEP".to_string(),
        ];

        let tasks = detect_required_tasks(&mods, "skyrimse");
        let synthesis = tasks.iter().find(|t| matches!(
            &t.task,
            AutomatableTask::SynthesisRunPatcher { .. }
        ));
        assert!(synthesis.is_some(), "Should detect Synthesis need");
    }

    #[test]
    fn test_detect_xedit_dirty_plugins() {
        let mods = vec![
            "Update.esm".to_string(),
            "Dawnguard.esm".to_string(),
            "SkyUI".to_string(),
        ];

        let tasks = detect_required_tasks(&mods, "skyrimse");
        let xedit_tasks: Vec<_> = tasks
            .iter()
            .filter(|t| matches!(&t.task, AutomatableTask::XEditQuickAutoClean { .. }))
            .collect();
        assert!(xedit_tasks.len() >= 2, "Should detect Update.esm and Dawnguard.esm as dirty");
    }

    #[test]
    fn test_detect_no_xedit_for_non_bethesda() {
        let mods = vec![
            "Update.esm".to_string(),
        ];

        let tasks = detect_required_tasks(&mods, "hogwartslegacy");
        let xedit_tasks: Vec<_> = tasks
            .iter()
            .filter(|t| matches!(&t.task, AutomatableTask::XEditQuickAutoClean { .. }))
            .collect();
        assert!(xedit_tasks.is_empty(), "Should NOT suggest xEdit for non-Bethesda games");
    }

    #[test]
    fn test_detect_nothing_needed() {
        let mods = vec![
            "SkyUI".to_string(),
            "USSEP".to_string(),
            "Address Library".to_string(),
        ];

        let tasks = detect_required_tasks(&mods, "skyrimse");
        // SkyUI and USSEP shouldn't trigger BodySlide or Synthesis
        let bodyslide = tasks.iter().find(|t| t.task == AutomatableTask::BodySlideBatchBuild);
        let synthesis = tasks.iter().find(|t| matches!(
            &t.task,
            AutomatableTask::SynthesisRunPatcher { .. }
        ));
        assert!(bodyslide.is_none());
        assert!(synthesis.is_none());
    }

    #[test]
    fn test_detect_case_insensitive() {
        let mods = vec!["cbbe body".to_string()];
        let tasks = detect_required_tasks(&mods, "skyrimse");
        let bodyslide = tasks.iter().find(|t| t.task == AutomatableTask::BodySlideBatchBuild);
        assert!(bodyslide.is_some(), "Detection should be case-insensitive");
    }

    // -- TaskConfig serialization --

    #[test]
    fn test_task_config_serializes() {
        let config = TaskConfig {
            tool_id: "sseedit".to_string(),
            game_id: "skyrimse".to_string(),
            bottle_name: "Skyrim".to_string(),
            exe_path: PathBuf::from("/tools/SSEEdit.exe"),
            args: vec!["-quickautoclean".to_string(), "Update.esm".to_string()],
            timeout_secs: 300,
            dll_overrides: HashMap::new(),
        };

        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("sseedit"));
        assert!(json.contains("quickautoclean"));

        let deserialized: TaskConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.tool_id, "sseedit");
        assert_eq!(deserialized.args.len(), 2);
    }

    #[test]
    fn test_task_result_serializes() {
        let result = TaskResult {
            success: true,
            stdout: "Cleaned 42 records".to_string(),
            stderr: String::new(),
            exit_code: Some(0),
            duration_ms: 12345,
        };

        let json = serde_json::to_string(&result).unwrap();
        let deserialized: TaskResult = serde_json::from_str(&json).unwrap();
        assert!(deserialized.success);
        assert_eq!(deserialized.exit_code, Some(0));
        assert_eq!(deserialized.duration_ms, 12345);
    }
}
