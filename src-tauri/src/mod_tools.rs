//! Mod tools registry — detection, installation, and launching of common modding tools.
//!
//! Provides a registry of known modding tools (SSEEdit, BethINI, DynDOLOD, etc.)
//! with automatic detection, GitHub-based auto-installation, and Wine-based launching.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use log::{debug, info};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use thiserror::Error;
use walkdir::WalkDir;

use crate::collections::CollectionManifest;
use crate::wabbajack::ParsedModlist;

// ---------------------------------------------------------------------------
// Progress Events
// ---------------------------------------------------------------------------

/// Progress event emitted during tool installation.
#[derive(Clone, Debug, Serialize)]
pub struct ToolInstallProgress {
    pub tool_id: String,
    pub phase: String,
    pub detail: String,
}

/// Emit a tool install progress event (best-effort, ignores errors).
fn emit_progress(app: &AppHandle, tool_id: &str, phase: &str, detail: &str) {
    let _ = app.emit(
        "tool-install-progress",
        ToolInstallProgress {
            tool_id: tool_id.to_string(),
            phase: phase.to_string(),
            detail: detail.to_string(),
        },
    );
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("Tool '{0}' not found in registry")]
    NotFound(String),

    #[error("Tool '{0}' has no auto-install source")]
    NoAutoInstall(String),

    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("ZIP extraction error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("7z extraction error: {0}")]
    SevenZ(String),

    #[error("Tool executable not found after installation")]
    ExeNotFound,

    #[error("GitHub API error: {0}")]
    GitHub(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, ToolError>;

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
    /// Whether this tool can be auto-installed (from GitHub or NexusMods).
    pub can_auto_install: bool,
    /// GitHub "owner/repo" for tools that support auto-install.
    pub github_repo: Option<String>,
    /// NexusMods mod ID for tools distributed via Nexus (premium auto-download).
    pub nexus_mod_id: Option<i64>,
    /// NexusMods game domain slug (e.g. "skyrimspecialedition") for Nexus downloads.
    pub nexus_game_slug: Option<String>,
    /// Direct download URL for tools not on GitHub or Nexus.
    pub download_url: Option<String>,
    /// Software license identifier.
    pub license: String,
    /// Wine compatibility notes.
    pub wine_notes: Option<String>,
    /// Wine compatibility level: "good", "limited", or "not_recommended".
    pub wine_compat: String,
    /// Alternative tool ID to recommend (e.g., Nemesis → Pandora).
    pub recommended_alternative: Option<String>,
    /// INI edits to apply when this tool is installed/run.
    pub recommended_ini_edits: Vec<IniEdit>,
    /// Ko-fi, Patreon, or other support/donation URL for the tool author.
    pub support_url: Option<String>,
    /// If true, this tool requires an additional explicit user opt-in
    /// (beyond the experimental-tool warning) before it appears or is
    /// installable. Used for sources that host adult / NSFW content
    /// (e.g. LoversLab), which are gated behind a per-game config flag
    /// such as `enable_adult_content_for_sims4`.
    #[serde(default)]
    pub requires_explicit_opt_in: bool,
}

/// A recommended INI edit for a tool.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IniEdit {
    /// INI file name (e.g., "Skyrim.ini", "SkyrimPrefs.ini").
    pub file: String,
    /// INI section (e.g., "General", "Animation").
    pub section: String,
    /// Key name.
    pub key: String,
    /// Recommended value.
    pub value: String,
    /// Description of what this edit does.
    pub description: String,
}

// ---------------------------------------------------------------------------
// Tool Requirement Signatures
// ---------------------------------------------------------------------------

/// Maps known Nexus mod IDs and archive name patterns to a Corkscrew tool ID.
pub struct ToolSignature {
    pub tool_id: &'static str,
    pub tool_name: &'static str,
    pub nexus_mod_ids: &'static [i64],
    pub name_patterns: &'static [&'static str],
    /// Game IDs this tool applies to. Empty = all games.
    pub game_ids: &'static [&'static str],
}

/// Known tool signatures for detecting required tools in collections/wabbajack lists.
pub const TOOL_SIGNATURES: &[ToolSignature] = &[
    ToolSignature {
        tool_id: "skse",
        tool_name: "SKSE64",
        nexus_mod_ids: &[30379],
        name_patterns: &["skse64", "skse_"],
        game_ids: &["skyrimse"],
    },
    ToolSignature {
        tool_id: "sseedit",
        tool_name: "SSEEdit (xEdit)",
        nexus_mod_ids: &[164],
        name_patterns: &["sseedit", "xedit", "tes5edit"],
        game_ids: &["skyrimse"],
    },
    ToolSignature {
        tool_id: "bodyslide",
        tool_name: "BodySlide & Outfit Studio",
        nexus_mod_ids: &[201],
        name_patterns: &["bodyslide", "outfit studio"],
        game_ids: &["skyrimse"],
    },
    ToolSignature {
        tool_id: "nemesis",
        tool_name: "Nemesis",
        nexus_mod_ids: &[60033],
        name_patterns: &["nemesis unlimited behavior"],
        game_ids: &["skyrimse"],
    },
    ToolSignature {
        tool_id: "fnis",
        tool_name: "FNIS",
        nexus_mod_ids: &[3038],
        name_patterns: &["fnis", "generatefnis"],
        game_ids: &["skyrimse"],
    },
    ToolSignature {
        tool_id: "pandora",
        tool_name: "Pandora Behaviour Engine+",
        nexus_mod_ids: &[],
        name_patterns: &["pandora behaviour", "pandora behavior"],
        game_ids: &["skyrimse"],
    },
    ToolSignature {
        tool_id: "dyndolod",
        tool_name: "DynDOLOD",
        nexus_mod_ids: &[68518, 32382],
        name_patterns: &["dyndolod"],
        game_ids: &["skyrimse"],
    },
    ToolSignature {
        tool_id: "wryebash",
        tool_name: "Wrye Bash",
        nexus_mod_ids: &[],
        name_patterns: &["wrye bash", "wryebash"],
        game_ids: &["skyrimse", "fallout4"],
    },
    ToolSignature {
        tool_id: "cao",
        tool_name: "Cathedral Assets Optimizer",
        nexus_mod_ids: &[],
        name_patterns: &["cathedral assets optimizer"],
        game_ids: &["skyrimse"],
    },
    ToolSignature {
        tool_id: "bethini",
        tool_name: "BethINI Pie",
        nexus_mod_ids: &[631],
        name_patterns: &["bethini"],
        game_ids: &["skyrimse", "fallout4"],
    },
    ToolSignature {
        tool_id: "nifoptimizer",
        tool_name: "SSE NIF Optimizer",
        nexus_mod_ids: &[],
        name_patterns: &["nif optimizer", "nifoptimizer"],
        game_ids: &["skyrimse"],
    },
    // -- Fallout 4 tools --
    ToolSignature {
        tool_id: "f4se",
        tool_name: "F4SE",
        nexus_mod_ids: &[42147],
        name_patterns: &["f4se_", "f4se"],
        game_ids: &["fallout4"],
    },
    ToolSignature {
        tool_id: "fo4edit",
        tool_name: "FO4Edit (xEdit)",
        nexus_mod_ids: &[2737],
        name_patterns: &["fo4edit"],
        game_ids: &["fallout4"],
    },
    ToolSignature {
        tool_id: "bodyslide_fo4",
        tool_name: "BodySlide & Outfit Studio (FO4)",
        nexus_mod_ids: &[25],
        name_patterns: &[],
        game_ids: &["fallout4"],
    },
    // -- Hogwarts Legacy tools --
    ToolSignature {
        tool_id: "ue4ss",
        tool_name: "RE-UE4SS",
        nexus_mod_ids: &[942],
        name_patterns: &["ue4ss", "re-ue4ss"],
        game_ids: &["hogwartslegacy"],
    },
    // NOTE: HLModMerger (Nexus mod 178) is NOT listed here — Corkscrew has a
    // native PAK database merger (hl_merger.rs) that replaces it entirely.
    // No external tool needed.
    // -- GTA V tools --
    ToolSignature {
        tool_id: "gtav-asi-loader",
        tool_name: "ASI Loader",
        nexus_mod_ids: &[],
        name_patterns: &["asi loader", "asiloader", "alexander blade", "dinput8"],
        game_ids: &["gtav"],
    },
    ToolSignature {
        tool_id: "scripthookv",
        tool_name: "ScriptHookV",
        nexus_mod_ids: &[],
        name_patterns: &["scripthookv", "script hook v"],
        game_ids: &["gtav"],
    },
    ToolSignature {
        tool_id: "shvdn",
        tool_name: "ScriptHookVDotNet",
        nexus_mod_ids: &[],
        name_patterns: &["scripthookvdotnet", "shvdn", "script hook v dot net"],
        game_ids: &["gtav"],
    },
    ToolSignature {
        tool_id: "gtav-lua-plugin",
        tool_name: "LUA Plugin for Script Hook V",
        nexus_mod_ids: &[],
        name_patterns: &["lua plugin", "lua-plugin"],
        game_ids: &["gtav"],
    },
    ToolSignature {
        tool_id: "openiv",
        tool_name: "OpenIV",
        nexus_mod_ids: &[],
        name_patterns: &["openiv", "open iv"],
        game_ids: &["gtav"],
    },
];

/// A tool detected as required by a collection or wabbajack modlist.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequiredTool {
    pub tool_id: String,
    pub tool_name: String,
    pub can_auto_install: bool,
    pub is_detected: bool,
    pub wine_compat: String,
    pub recommended_alternative: Option<String>,
    pub download_url: Option<String>,
}

// ---------------------------------------------------------------------------
// Tool Requirement Detection
// ---------------------------------------------------------------------------

/// Tools that are integrated into Corkscrew and should never appear as required.
const INTEGRATED_TOOLS: &[&str] = &["loot"];

/// Detect tool requirements from a NexusMods collection manifest.
pub fn detect_required_tools_collection(
    manifest: &CollectionManifest,
    game_data_dir: &Path,
    game_id: &str,
) -> Vec<RequiredTool> {
    let mut matched_ids: HashSet<String> = HashSet::new();
    let mut results: Vec<RequiredTool> = Vec::new();
    let detected = detect_tools(game_data_dir);

    for mod_entry in &manifest.mods {
        let name_lower = mod_entry.name.to_lowercase();

        for sig in TOOL_SIGNATURES {
            if matched_ids.contains(sig.tool_id) {
                continue;
            }
            // Skip tools that don't apply to this game
            if !sig.game_ids.is_empty() && !sig.game_ids.contains(&game_id) {
                continue;
            }
            // Skip tools integrated into Corkscrew
            if INTEGRATED_TOOLS.contains(&sig.tool_id) {
                continue;
            }

            let id_match = mod_entry
                .source
                .mod_id
                .map(|id| sig.nexus_mod_ids.contains(&id))
                .unwrap_or(false);

            let name_match = sig.name_patterns.iter().any(|p| name_lower.contains(p));

            if id_match || name_match {
                matched_ids.insert(sig.tool_id.to_string());
                results.push(build_required_tool(sig, &detected));
            }
        }
    }

    suppress_replaced_tools(&mut results, &detected);
    results
}

/// Detect tool requirements from a parsed Wabbajack modlist.
pub fn detect_required_tools_wabbajack(
    modlist: &ParsedModlist,
    game_data_dir: &Path,
) -> Vec<RequiredTool> {
    let mut matched_ids: HashSet<String> = HashSet::new();
    let mut results: Vec<RequiredTool> = Vec::new();
    let detected = detect_tools(game_data_dir);

    for archive in &modlist.archives {
        let name_lower = archive.name.to_lowercase();

        for sig in TOOL_SIGNATURES {
            if matched_ids.contains(sig.tool_id) {
                continue;
            }
            // Skip tools integrated into Corkscrew
            if INTEGRATED_TOOLS.contains(&sig.tool_id) {
                continue;
            }

            let id_match = archive
                .nexus_mod_id
                .map(|id| sig.nexus_mod_ids.contains(&id))
                .unwrap_or(false);

            let name_match = sig.name_patterns.iter().any(|p| name_lower.contains(p));

            if id_match || name_match {
                matched_ids.insert(sig.tool_id.to_string());
                results.push(build_required_tool(sig, &detected));
            }
        }
    }

    suppress_replaced_tools(&mut results, &detected);
    results
}

/// If Pandora is detected/installable, suppress Nemesis and FNIS from required tools
/// since Pandora is backwards-compatible with both.
fn suppress_replaced_tools(results: &mut Vec<RequiredTool>, detected_tools: &[ModTool]) {
    let pandora_available = results.iter().any(|t| t.tool_id == "pandora")
        || detected_tools
            .iter()
            .any(|t| t.id == "pandora" && t.detected_path.is_some());

    if pandora_available {
        results.retain(|t| t.tool_id != "nemesis" && t.tool_id != "fnis");
    }
}

/// Build a RequiredTool from a signature, enriching with builtin tool metadata.
fn build_required_tool(sig: &ToolSignature, detected_tools: &[ModTool]) -> RequiredTool {
    let builtin = builtin_tools().into_iter().find(|t| t.id == sig.tool_id);

    let is_detected = detected_tools
        .iter()
        .any(|t| t.id == sig.tool_id && t.detected_path.is_some());

    if let Some(tool) = builtin {
        RequiredTool {
            tool_id: sig.tool_id.to_string(),
            tool_name: tool.name,
            can_auto_install: tool.can_auto_install,
            is_detected,
            wine_compat: tool.wine_compat,
            recommended_alternative: tool.recommended_alternative,
            download_url: tool.download_url,
        }
    } else {
        // Tool in signatures but not in builtin registry (e.g. SKSE)
        RequiredTool {
            tool_id: sig.tool_id.to_string(),
            tool_name: sig.tool_name.to_string(),
            can_auto_install: false,
            is_detected,
            wine_compat: "good".to_string(),
            recommended_alternative: None,
            download_url: None,
        }
    }
}

/// Minimal GitHub release JSON shape.
#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

// ---------------------------------------------------------------------------
// Tool Registry
// ---------------------------------------------------------------------------

/// Built-in tool definitions, filtered by game ID.
///
/// Returns the tools relevant to the specified game. Shared tools (Wrye Bash,
/// BethINI, CAO) appear for all supported games.
fn builtin_tools_for_game(game_id: &str) -> Vec<ModTool> {
    let mut tools = match game_id {
        "skyrimse" => skyrim_se_tools(),
        "fallout4" => fallout4_tools(),
        "hogwartslegacy" => hogwarts_legacy_tools(),
        "hades2" => hades2_tools(),
        "crimsondesert" => crimson_desert_tools(),
        "sims4" => sims4_tools_filtered(),
        "gtav" => gtav_tools(),
        "genshin" => genshin_tools(),
        "sekiro" | "eldenring" | "darksouls3" | "darksouls_remastered" | "armoredcore6" => {
            fromsoft_tools()
        }
        _ => vec![],
    };

    // Merge tools from Vortex extensions (if this game has a Vortex plugin).
    if let Some(vortex_tools) = crate::games::with_plugin(game_id, |p| p.vortex_tools()) {
        for vt in &vortex_tools {
            // Skip tools that already exist in the builtin list (by id or exe name).
            if !tools.iter().any(|t| {
                t.id == vt.id
                    || t.exe_names
                        .iter()
                        .any(|e| e.eq_ignore_ascii_case(&vt.executable))
            }) {
                tools.push(vortex_tool_to_mod_tool(vt));
            }
        }
    }

    tools
}

/// Convert a Vortex extension tool definition into a [`ModTool`].
fn vortex_tool_to_mod_tool(vt: &crate::vortex_types::VortexTool) -> ModTool {
    ModTool {
        id: vt.id.clone(),
        name: vt.name.clone(),
        description: format!("{} (from Vortex extension)", vt.name),
        exe_names: vec![vt.executable.clone()],
        detected_path: None,
        requires_wine: true,
        category: "Extension Tool".into(),
        can_auto_install: false,
        github_repo: None,
        nexus_mod_id: None,
        nexus_game_slug: None,
        download_url: None,
        license: "Unknown".into(),
        wine_notes: None,
        wine_compat: "unknown".into(),
        recommended_alternative: None,
        recommended_ini_edits: vec![],
        support_url: None,
        requires_explicit_opt_in: false,
    }
}

/// Backwards-compatible alias — returns Skyrim SE tools.
fn builtin_tools() -> Vec<ModTool> {
    skyrim_se_tools()
}

/// All builtin tools across all games (for tool detection by signature).
fn all_builtin_tools() -> Vec<ModTool> {
    let mut tools = skyrim_se_tools();
    for tool in fallout4_tools() {
        if !tools.iter().any(|t| t.id == tool.id) {
            tools.push(tool);
        }
    }
    for tool in hogwarts_legacy_tools() {
        if !tools.iter().any(|t| t.id == tool.id) {
            tools.push(tool);
        }
    }
    for tool in hades2_tools() {
        if !tools.iter().any(|t| t.id == tool.id) {
            tools.push(tool);
        }
    }
    for tool in crimson_desert_tools() {
        if !tools.iter().any(|t| t.id == tool.id) {
            tools.push(tool);
        }
    }
    for tool in sims4_tools() {
        if !tools.iter().any(|t| t.id == tool.id) {
            tools.push(tool);
        }
    }
    for tool in gtav_tools() {
        if !tools.iter().any(|t| t.id == tool.id) {
            tools.push(tool);
        }
    }
    for tool in genshin_tools() {
        if !tools.iter().any(|t| t.id == tool.id) {
            tools.push(tool);
        }
    }
    for tool in fromsoft_tools() {
        if !tools.iter().any(|t| t.id == tool.id) {
            tools.push(tool);
        }
    }
    tools
}

/// The Sims 4 tools, filtered by the user's adult-content opt-in.
///
/// Tools with `requires_explicit_opt_in == true` (e.g. the LoversLab
/// link entry) are only included when the user has flipped the
/// `enable_adult_content_for_sims4` flag in their config. The base
/// tools (MC Command Center, Sims 4 Studio) always appear.
fn sims4_tools_filtered() -> Vec<ModTool> {
    let opted_in = crate::config::get_config()
        .map(|c| c.enable_adult_content_for_sims4)
        .unwrap_or(false);
    sims4_tools()
        .into_iter()
        .filter(|t| !t.requires_explicit_opt_in || opted_in)
        .collect()
}

// ---------------------------------------------------------------------------
// Shared tools (used by both Skyrim SE and Fallout 4)
// ---------------------------------------------------------------------------

fn shared_tools() -> Vec<ModTool> {
    vec![
        ModTool {
            id: "wryebash".into(),
            name: "Wrye Bash".into(),
            description: "Bashed patch creation and leveled list merging".into(),
            exe_names: vec!["Wrye Bash.exe".into()],
            detected_path: None,
            requires_wine: true,
            category: "Patching".into(),
            can_auto_install: true,
            github_repo: Some("wrye-bash/wrye-bash".into()),
            nexus_mod_id: None,
            nexus_game_slug: None,
            download_url: None,
            license: "GPL-3.0".into(),
            wine_notes: Some("Native Linux support since v312; also works via Wine".into()),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: None,
            requires_explicit_opt_in: false,
        },
        ModTool {
            id: "bethini".into(),
            name: "BethINI Pie".into(),
            description: "INI configuration optimizer".into(),
            exe_names: vec!["BethINI.exe".into(), "Bethini Pie.exe".into()],
            detected_path: None,
            requires_wine: true,
            category: "INI".into(),
            can_auto_install: true,
            github_repo: None,
            nexus_mod_id: Some(631),
            nexus_game_slug: Some("site".into()),
            download_url: Some("https://www.nexusmods.com/site/mods/631".into()),
            license: "CC BY-NC-SA 4.0".into(),
            wine_notes: Some("Python-based; may work under Wine. Corkscrew's built-in INI editor provides similar functionality natively.".into()),
            wine_compat: "limited".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: None,
            requires_explicit_opt_in: false,
        },
    ]
}

// ---------------------------------------------------------------------------
// Skyrim SE tools
// ---------------------------------------------------------------------------

fn skyrim_se_tools() -> Vec<ModTool> {
    let mut tools = vec![
        // ---- Frameworks ----
        ModTool {
            id: "skse".into(),
            name: "SKSE64".into(),
            description: "Skyrim Script Extender — required by most Skyrim mods".into(),
            exe_names: vec!["skse64_loader.exe".into()],
            detected_path: None,
            requires_wine: true,
            category: "Framework".into(),
            can_auto_install: true,
            github_repo: Some("ianpatt/skse64".into()),
            nexus_mod_id: None,
            nexus_game_slug: None,
            download_url: None,
            license: "Proprietary".into(),
            wine_notes: Some("Works under Wine/Proton".into()),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("https://skse.silverlock.org/".into()),
            requires_explicit_opt_in: false,
        },
        // ---- Recommended tools (good Wine compatibility) ----
        ModTool {
            id: "sseedit".into(),
            name: "SSEEdit (xEdit)".into(),
            description: "Plugin cleaning and conflict resolution".into(),
            exe_names: vec![
                "SSEEdit.exe".into(),
                "SSEEdit64.exe".into(),
                "xEdit.exe".into(),
                "xTESEdit.exe".into(),
                "xTESEdit64.exe".into(),
            ],
            detected_path: None,
            requires_wine: true,
            category: "Cleaning".into(),
            can_auto_install: true,
            github_repo: Some("TES5Edit/TES5Edit".into()),
            nexus_mod_id: Some(164),
            nexus_game_slug: Some("skyrimspecialedition".into()),
            download_url: None,
            license: "MPL-2.0".into(),
            wine_notes: Some("Works well under Wine/Proton".into()),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("https://ko-fi.com/elminsterau".into()),
            requires_explicit_opt_in: false,
        },
        ModTool {
            id: "pandora".into(),
            name: "Pandora Behaviour Engine+".into(),
            description: "Animation engine — replaces FNIS and Nemesis with better Wine support"
                .into(),
            exe_names: vec![
                "Pandora Behaviour Engine+.exe".into(),
                "Pandora Behaviour Engine.exe".into(),
                "Pandora.exe".into(),
            ],
            detected_path: None,
            requires_wine: true,
            category: "Animation".into(),
            can_auto_install: true,
            github_repo: Some("Monitor221hz/Pandora-Behaviour-Engine-Plus".into()),
            nexus_mod_id: None,
            nexus_game_slug: None,
            download_url: None,
            license: "MIT".into(),
            wine_notes: Some("Works under Wine/Proton; backwards-compatible with FNIS and Nemesis animation mods".into()),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("https://www.patreon.com/monitorhz".into()),
            requires_explicit_opt_in: false,
        },
        ModTool {
            id: "bodyslide".into(),
            name: "BodySlide & Outfit Studio".into(),
            description: "Body and outfit customization".into(),
            exe_names: vec!["BodySlide.exe".into(), "OutfitStudio.exe".into()],
            detected_path: None,
            requires_wine: true,
            category: "Body".into(),
            can_auto_install: true,
            github_repo: None,
            nexus_mod_id: Some(201),
            nexus_game_slug: Some("skyrimspecialedition".into()),
            download_url: Some("https://www.nexusmods.com/skyrimspecialedition/mods/201".into()),
            license: "GPL-3.0".into(),
            wine_notes: Some("Works under Wine with some setup".into()),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: None,
            requires_explicit_opt_in: false,
        },
        ModTool {
            id: "cao".into(),
            name: "Cathedral Assets Optimizer".into(),
            description: "Texture and mesh optimization".into(),
            exe_names: vec![
                "Cathedral Assets Optimizer.exe".into(),
                "Cathedral_Assets_Optimizer.exe".into(),
            ],
            detected_path: None,
            requires_wine: true,
            category: "Optimization".into(),
            can_auto_install: true,
            github_repo: None,
            nexus_mod_id: Some(23316),
            nexus_game_slug: Some("skyrimspecialedition".into()),
            download_url: Some("https://www.nexusmods.com/skyrimspecialedition/mods/23316".into()),
            license: "MPL-2.0".into(),
            wine_notes: Some("Generally works under Wine".into()),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("https://github.com/sponsors/Guekka".into()),
            requires_explicit_opt_in: false,
        },
        ModTool {
            id: "nifoptimizer".into(),
            name: "SSE NIF Optimizer".into(),
            description: "NIF mesh optimization for SSE".into(),
            exe_names: vec!["SSE NIF Optimizer.exe".into()],
            detected_path: None,
            requires_wine: true,
            category: "Optimization".into(),
            can_auto_install: true,
            github_repo: None,
            nexus_mod_id: Some(4089),
            nexus_game_slug: Some("skyrimspecialedition".into()),
            download_url: Some("https://www.nexusmods.com/skyrimspecialedition/mods/4089".into()),
            license: "GPL-3.0".into(),
            wine_notes: None,
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: None,
            requires_explicit_opt_in: false,
        },
        // ---- Tools with limited Wine compatibility ----
        ModTool {
            id: "dyndolod".into(),
            name: "DynDOLOD".into(),
            description: "Dynamic LOD generation".into(),
            exe_names: vec!["DynDOLOD.exe".into(), "DynDOLODx64.exe".into()],
            detected_path: None,
            requires_wine: true,
            category: "LOD".into(),
            can_auto_install: false,
            github_repo: None,
            nexus_mod_id: None,
            nexus_game_slug: None,
            download_url: Some("https://www.nexusmods.com/skyrimspecialedition/mods/68518".into()),
            license: "Proprietary".into(),
            wine_notes: Some("Texconv issues under Wine; limited functionality on Linux".into()),
            wine_compat: "limited".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("https://ko-fi.com/sheson".into()),
            requires_explicit_opt_in: false,
        },
        // ---- Not recommended via Wine — use Pandora instead ----
        ModTool {
            id: "nemesis".into(),
            name: "Nemesis".into(),
            description: "Animation engine (FNIS replacement)".into(),
            exe_names: vec!["Nemesis Unlimited Behavior Engine.exe".into()],
            detected_path: None,
            requires_wine: true,
            category: "Animation".into(),
            can_auto_install: false,
            github_repo: None,
            nexus_mod_id: None,
            nexus_game_slug: None,
            download_url: Some("https://www.nexusmods.com/skyrimspecialedition/mods/60033".into()),
            license: "GPL-3.0".into(),
            wine_notes: Some("Poor Wine compatibility. Use Pandora instead — it is backwards-compatible with Nemesis animation mods.".into()),
            wine_compat: "not_recommended".into(),
            recommended_alternative: Some("pandora".into()),
            recommended_ini_edits: vec![],
            support_url: Some("https://www.patreon.com/shikyokira".into()),
            requires_explicit_opt_in: false,
        },
        ModTool {
            id: "fnis".into(),
            name: "FNIS".into(),
            description: "Legacy animation framework (deprecated)".into(),
            exe_names: vec!["GenerateFNISforUsers.exe".into()],
            detected_path: None,
            requires_wine: true,
            category: "Animation".into(),
            can_auto_install: false,
            github_repo: None,
            nexus_mod_id: None,
            nexus_game_slug: None,
            download_url: Some("https://www.nexusmods.com/skyrimspecialedition/mods/3038".into()),
            license: "Proprietary".into(),
            wine_notes: Some(
                "Deprecated and poor Wine compatibility. Use Pandora instead — it is backwards-compatible with FNIS animation mods.".into(),
            ),
            wine_compat: "not_recommended".into(),
            recommended_alternative: Some("pandora".into()),
            recommended_ini_edits: vec![],
            support_url: None,
            requires_explicit_opt_in: false,
        },
    ];
    tools.extend(shared_tools());
    tools
}

// ---------------------------------------------------------------------------
// Fallout 4 tools
// ---------------------------------------------------------------------------

fn fallout4_tools() -> Vec<ModTool> {
    let mut tools = vec![
        // ---- Frameworks ----
        ModTool {
            id: "f4se".into(),
            name: "F4SE".into(),
            description: "Fallout 4 Script Extender — required by most Fallout 4 mods".into(),
            exe_names: vec!["f4se_loader.exe".into()],
            detected_path: None,
            requires_wine: true,
            category: "Framework".into(),
            can_auto_install: true,
            github_repo: Some("ianpatt/f4se".into()),
            nexus_mod_id: None,
            nexus_game_slug: None,
            download_url: None,
            license: "Proprietary".into(),
            wine_notes: Some("Works under Wine/Proton".into()),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("https://f4se.silverlock.org/".into()),
            requires_explicit_opt_in: false,
        },
        // ---- Recommended tools ----
        ModTool {
            id: "fo4edit".into(),
            name: "FO4Edit (xEdit)".into(),
            description: "Plugin cleaning and conflict resolution for Fallout 4".into(),
            exe_names: vec![
                "FO4Edit.exe".into(),
                "FO4Edit64.exe".into(),
                "xEdit.exe".into(),
                "xTESEdit.exe".into(),
                "xTESEdit64.exe".into(),
            ],
            detected_path: None,
            requires_wine: true,
            category: "Cleaning".into(),
            can_auto_install: true,
            github_repo: Some("TES5Edit/TES5Edit".into()),
            nexus_mod_id: Some(2737),
            nexus_game_slug: Some("fallout4".into()),
            download_url: None,
            license: "MPL-2.0".into(),
            wine_notes: Some("Works well under Wine/Proton".into()),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("https://ko-fi.com/elminsterau".into()),
            requires_explicit_opt_in: false,
        },
        ModTool {
            id: "bodyslide_fo4".into(),
            name: "BodySlide & Outfit Studio".into(),
            description: "Body and outfit customization for Fallout 4".into(),
            exe_names: vec!["BodySlide.exe".into(), "OutfitStudio.exe".into()],
            detected_path: None,
            requires_wine: true,
            category: "Body".into(),
            can_auto_install: true,
            github_repo: None,
            nexus_mod_id: Some(25),
            nexus_game_slug: Some("fallout4".into()),
            download_url: Some("https://www.nexusmods.com/fallout4/mods/25".into()),
            license: "GPL-3.0".into(),
            wine_notes: Some("Works under Wine with some setup".into()),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: None,
            requires_explicit_opt_in: false,
        },
    ];
    tools.extend(shared_tools());
    tools
}

// ---------------------------------------------------------------------------
// Hogwarts Legacy tools
// ---------------------------------------------------------------------------

fn hogwarts_legacy_tools() -> Vec<ModTool> {
    vec![
        ModTool {
            id: "ue4ss".into(),
            name: "RE-UE4SS".into(),
            description: "Unreal Engine scripting system — enables Lua and Blueprint mods".into(),
            exe_names: vec!["UE4SS.dll".into(), "xinput1_3.dll".into()],
            detected_path: None,
            requires_wine: true,
            category: "Framework".into(),
            can_auto_install: true,
            github_repo: Some("UE4SS-RE/RE-UE4SS".into()),
            nexus_mod_id: Some(942),
            nexus_game_slug: Some("hogwartslegacy".into()),
            download_url: None,
            license: "MIT".into(),
            wine_notes: Some("DLL injection works under Wine/Proton".into()),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("https://github.com/UE4SS-RE/RE-UE4SS".into()),
            requires_explicit_opt_in: false,
        },
        // NOTE: HLModMerger removed — Corkscrew has native PAK database merging
        // via hl_merger.rs. The external Windows tool is not needed.
        ModTool {
            id: "hl-mod-merger-native".into(),
            name: "PAK Database Merger (built-in)".into(),
            description: "Automatically merges conflicting PhoenixShipData.sqlite — no external tool needed".into(),
            exe_names: vec![],
            detected_path: None,
            requires_wine: false,
            category: "Utility".into(),
            can_auto_install: false,
            github_repo: None,
            nexus_mod_id: None,
            nexus_game_slug: None,
            download_url: None,
            license: "GPL-3.0".into(),
            wine_notes: None,
            wine_compat: "native".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: None,
            requires_explicit_opt_in: false,
        },
    ]
}

// ---------------------------------------------------------------------------
// Hades II tools
// ---------------------------------------------------------------------------

fn hades2_tools() -> Vec<ModTool> {
    vec![
        ModTool {
            id: "modimporter".into(),
            name: "ModImporter".into(),
            description:
                "Official Supergiant community mod loader for Hades II. Installs mods from Content/Mods/ into the game's Lua scripting system."
                    .into(),
            exe_names: vec![
                // ModImporter is distributed as a Python script / compiled binary.
                // Presence of either indicates an install.
                "modimporter.exe".into(),
                "modimporter.py".into(),
            ],
            detected_path: None,
            requires_wine: true,
            category: "Framework".into(),
            can_auto_install: true,
            github_repo: Some("SGG-Modding/ModImporter".into()),
            nexus_mod_id: None,
            nexus_game_slug: Some("hades2".into()),
            download_url: None,
            license: "MIT".into(),
            wine_notes: Some(
                "Runs as a Windows binary inside the bottle; no special prefix setup required."
                    .into(),
            ),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("https://github.com/SGG-Modding/ModImporter".into()),
            requires_explicit_opt_in: false,
        },
        ModTool {
            id: "modutil".into(),
            name: "ModUtil".into(),
            description:
                "Runtime Lua merging library for Hades II mods. Required by most script mods."
                    .into(),
            exe_names: vec!["ModUtil".into()],
            detected_path: None,
            requires_wine: false,
            category: "Framework".into(),
            can_auto_install: true,
            github_repo: Some("SGG-Modding/ModUtil".into()),
            nexus_mod_id: None,
            nexus_game_slug: Some("hades2".into()),
            download_url: None,
            license: "MIT".into(),
            wine_notes: Some("Pure Lua — no Wine concerns.".into()),
            wine_compat: "native".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("https://github.com/SGG-Modding/ModUtil".into()),
            requires_explicit_opt_in: false,
        },
    ]
}

// ---------------------------------------------------------------------------
// Crimson Desert tools
// ---------------------------------------------------------------------------

fn crimson_desert_tools() -> Vec<ModTool> {
    vec![
        ModTool {
            id: "ultimate-asi-loader".into(),
            name: "Ultimate ASI Loader".into(),
            description:
                "Sideloads ASI/DLL mods next to the game exe. Works under Wine/CrossOver with a renamed proxy DLL."
                    .into(),
            // Common proxy names the loader ships under. Presence of any
            // indicates an install.
            exe_names: vec![
                "dinput8.dll".into(),
                "xinput1_3.dll".into(),
                "version.dll".into(),
                "winmm.dll".into(),
            ],
            detected_path: None,
            requires_wine: true,
            category: "Framework".into(),
            can_auto_install: true,
            github_repo: Some("ThirteenAG/Ultimate-ASI-Loader".into()),
            nexus_mod_id: None,
            nexus_game_slug: Some("crimsondesert".into()),
            download_url: None,
            license: "Unlicense".into(),
            wine_notes: Some(
                "Loads as a DLL proxy inside the bottle. No special Wine configuration needed."
                    .into(),
            ),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("https://github.com/ThirteenAG/Ultimate-ASI-Loader".into()),
            requires_explicit_opt_in: false,
        },
        ModTool {
            id: "reshade".into(),
            name: "ReShade".into(),
            description:
                "Post-processing shader injector. Works under DXVK/Wine for DX11/DX12 titles."
                    .into(),
            exe_names: vec!["ReShade64.dll".into(), "dxgi.dll".into(), "d3d11.dll".into()],
            detected_path: None,
            requires_wine: true,
            category: "Graphics".into(),
            can_auto_install: false,
            github_repo: None,
            nexus_mod_id: None,
            nexus_game_slug: None,
            download_url: Some("https://reshade.me/".into()),
            license: "BSD-3-Clause".into(),
            wine_notes: Some(
                "ReShade runs under DXVK; installer is Windows-only — install it through the bottle."
                    .into(),
            ),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("https://reshade.me/".into()),
            requires_explicit_opt_in: false,
        },
    ]
}

// ---------------------------------------------------------------------------
// The Sims 4 tools
// ---------------------------------------------------------------------------

/// Tool registry for The Sims 4.
///
/// Includes the community-essential script-mod framework (MC Command
/// Center), the de-facto custom-content authoring tool (Sims 4 Studio),
/// and a link-only entry for LoversLab — gated behind both the
/// experimental-warning dialog and the
/// `enable_adult_content_for_sims4` config flag.
fn sims4_tools() -> Vec<ModTool> {
    vec![
        // MC Command Center — by Deaderpool. Community essential. Hosted
        // on Patreon (free tier OK, no NM mirror).
        ModTool {
            id: "mccc".into(),
            name: "MC Command Center".into(),
            description:
                "Community-essential script mod by Deaderpool. Adds gameplay control, story progression tuning, and a long tail of quality-of-life fixes."
                    .into(),
            // Loaded by exact filename from Mods/. The pack ships a
            // `.ts4script` archive plus several `.package` files.
            exe_names: vec!["mc_cmd_center.ts4script".into(), "mc_settings.ts4script".into()],
            detected_path: None,
            requires_wine: false,
            category: "Framework".into(),
            can_auto_install: false,
            github_repo: None,
            nexus_mod_id: None,
            nexus_game_slug: None,
            download_url: Some("https://www.patreon.com/MCCC".into()),
            license: "Shareware".into(),
            wine_notes: Some(
                "Pure Python script mod — runs inside the game, no Wine interaction needed."
                    .into(),
            ),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("https://www.patreon.com/MCCC".into()),
            requires_explicit_opt_in: false,
        },
        // Sims 4 Studio — CC creation tool (Windows GUI app).
        ModTool {
            id: "sims4-studio".into(),
            name: "Sims 4 Studio".into(),
            description:
                "Custom-content creation and editing tool for The Sims 4. Free download from sims4studio.com."
                    .into(),
            exe_names: vec!["Sims4Studio.exe".into()],
            detected_path: None,
            requires_wine: true,
            category: "Tool".into(),
            can_auto_install: false,
            github_repo: None,
            nexus_mod_id: None,
            nexus_game_slug: None,
            download_url: Some("https://sims4studio.com/".into()),
            license: "Freeware".into(),
            wine_notes: Some(
                "Windows GUI app; runs under Wine/CrossOver. Requires .NET runtime in the bottle."
                    .into(),
            ),
            wine_compat: "limited".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("https://sims4studio.com/".into()),
            requires_explicit_opt_in: false,
        },
        // LoversLab — adult content (NSFW). Link-only; gated behind
        // explicit user opt-in via `enable_adult_content_for_sims4`.
        ModTool {
            id: "loverslab-sims4".into(),
            name: "LoversLab — The Sims 4".into(),
            description:
                "Community hub for adult / NSFW Sims 4 mods. External link only — Corkscrew does not download from LoversLab automatically."
                    .into(),
            exe_names: vec![],
            detected_path: None,
            requires_wine: false,
            category: "Adult Content".into(),
            can_auto_install: false,
            github_repo: None,
            nexus_mod_id: None,
            nexus_game_slug: None,
            download_url: Some(
                "https://www.loverslab.com/files/category/150-the-sims-4/".into(),
            ),
            license: "Varies".into(),
            wine_notes: None,
            wine_compat: "unknown".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: None,
            requires_explicit_opt_in: true,
        },
    ]
}

// ---------------------------------------------------------------------------
// GTA V tools
// ---------------------------------------------------------------------------

fn gtav_tools() -> Vec<ModTool> {
    vec![
        // ---- Frameworks ----
        ModTool {
            id: "gtav-asi-loader".into(),
            name: "ASI Loader".into(),
            description:
                "Alexander Blade's ASI Loader. Drops in as `dinput8.dll` next to GTA5.exe and auto-loads any `.asi` plugins from the game directory."
                    .into(),
            exe_names: vec!["dinput8.dll".into()],
            detected_path: None,
            requires_wine: true,
            category: "Framework".into(),
            can_auto_install: true,
            github_repo: None,
            nexus_mod_id: None,
            nexus_game_slug: None,
            download_url: Some("http://www.dev-c.com/gtav/scripthookv/".into()),
            license: "Freeware".into(),
            wine_notes: Some(
                "DLL proxy load works under Wine/CrossOver. Bundled with the ScriptHookV release archive."
                    .into(),
            ),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("http://www.dev-c.com/gtav/scripthookv/".into()),
            requires_explicit_opt_in: false,
        },
        ModTool {
            id: "scripthookv".into(),
            name: "ScriptHookV".into(),
            description:
                "Alexander Blade's native scripting library — required by virtually every C++ GTA V mod. Loads via the ASI Loader."
                    .into(),
            exe_names: vec!["ScriptHookV.dll".into()],
            detected_path: None,
            requires_wine: true,
            category: "Framework".into(),
            can_auto_install: true,
            github_repo: None,
            nexus_mod_id: None,
            nexus_game_slug: None,
            download_url: Some("http://www.dev-c.com/gtav/scripthookv/".into()),
            license: "Freeware".into(),
            wine_notes: Some(
                "Story mode only — never use online. Works under Wine when paired with the ASI Loader."
                    .into(),
            ),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("http://www.dev-c.com/gtav/scripthookv/".into()),
            requires_explicit_opt_in: false,
        },
        ModTool {
            id: "shvdn".into(),
            name: "ScriptHookVDotNet".into(),
            description:
                ".NET (C#/VB) scripting layer for GTA V. Requires ScriptHookV; ships ScriptHookVDotNet.asi plus a per-runtime DLL set."
                    .into(),
            exe_names: vec![
                "ScriptHookVDotNet.asi".into(),
                "ScriptHookVDotNet2.dll".into(),
                "ScriptHookVDotNet3.dll".into(),
            ],
            detected_path: None,
            requires_wine: true,
            category: "Framework".into(),
            can_auto_install: true,
            github_repo: Some("scripthookvdotnet/scripthookvdotnet".into()),
            nexus_mod_id: None,
            nexus_game_slug: None,
            download_url: None,
            license: "zlib".into(),
            wine_notes: Some(
                "Needs the in-bottle .NET runtime that the C# scripts target (typically .NET Framework 4.8). Install via winetricks if missing."
                    .into(),
            ),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("https://github.com/scripthookvdotnet/scripthookvdotnet".into()),
            requires_explicit_opt_in: false,
        },
        ModTool {
            id: "gtav-lua-plugin".into(),
            name: "LUA Plugin for Script Hook V".into(),
            description:
                "Headscript's Lua scripting plugin for GTA V. Drops `LUA Plugin.asi` into the game root and reads scripts from `scripts/addins/`."
                    .into(),
            exe_names: vec!["LUA Plugin.asi".into()],
            detected_path: None,
            requires_wine: true,
            category: "Framework".into(),
            can_auto_install: false,
            github_repo: None,
            nexus_mod_id: None,
            nexus_game_slug: None,
            download_url: Some(
                "https://www.gta5-mods.com/tools/lua-plugin-for-script-hook-v".into(),
            ),
            license: "Freeware".into(),
            wine_notes: Some(
                "Loads as an ASI plugin — no separate Wine setup needed beyond the ASI Loader."
                    .into(),
            ),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some(
                "https://www.gta5-mods.com/tools/lua-plugin-for-script-hook-v".into(),
            ),
            requires_explicit_opt_in: false,
        },
        // ---- External tool: OpenIV (link-only) ----
        ModTool {
            id: "openiv".into(),
            name: "OpenIV".into(),
            description:
                "external tool, install separately for .rpf editing. Closed-source archive editor used for vehicle / ped / map replacements. Phase 1 of Corkscrew does not parse .rpf or .oiv archives."
                    .into(),
            exe_names: vec!["OpenIV.exe".into()],
            detected_path: None,
            requires_wine: true,
            category: "Tool".into(),
            can_auto_install: false,
            github_repo: None,
            nexus_mod_id: None,
            nexus_game_slug: None,
            download_url: Some("https://openiv.com/".into()),
            license: "Proprietary".into(),
            wine_notes: Some(
                "Native installer is Windows-only. Run inside the bottle if needed; Corkscrew will not auto-install it."
                    .into(),
            ),
            wine_compat: "limited".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("https://openiv.com/".into()),
            requires_explicit_opt_in: false,
        },
    ]
}

// ---------------------------------------------------------------------------
// FromSoft tools (Sekiro / Elden Ring / DS3 / DS:R / AC6)
// ---------------------------------------------------------------------------
//
// Mod Engine 2 is the canonical FromSoft mod loader — a `dinput8.dll`
// proxy + injection framework. Mods install as `<game>/mod/<modname>/`
// subdirectories which the loader merges into the game's virtual file
// system at runtime. The launcher `modengine2_launcher.exe` patches
// `start_protected_game.exe` to disable EAC for offline modding.
//
// Works under Wine/CrossOver because it's a pure DLL proxy — no
// kernel-mode anti-cheat shenanigans involved.

fn fromsoft_tools() -> Vec<ModTool> {
    vec![
        ModTool {
            id: "modengine2".into(),
            name: "Mod Engine 2".into(),
            description: "FromSoft mod loader for Sekiro, Elden Ring, Dark Souls 3, Dark Souls: Remastered, and Armored Core VI. Disables EAC for offline modding.".into(),
            exe_names: vec![
                "modengine2_launcher.exe".into(),
                "launchmod_eldenring.bat".into(),
                "launchmod_sekiro.bat".into(),
                "launchmod_darksouls3.bat".into(),
                "launchmod_armoredcore6.bat".into(),
            ],
            detected_path: None,
            requires_wine: true,
            category: "Framework".into(),
            can_auto_install: true,
            github_repo: Some("soulsmods/ModEngine2".into()),
            nexus_mod_id: None,
            nexus_game_slug: None,
            download_url: None,
            license: "BSD-3-Clause".into(),
            wine_notes: Some("Loads as a dinput8.dll proxy. Patches start_protected_game.exe to disable EAC for offline play. Works under Wine/CrossOver.".into()),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("https://github.com/soulsmods/ModEngine2".into()),
            requires_explicit_opt_in: false,
        },
        ModTool {
            // Repacker for FromSoft archive formats (BND/BHD/BDT/DCX/PARAM/etc).
            // Essential for any FromSoft mod author; users tend to want it
            // alongside Mod Engine 2 for inspecting/repacking shipped mods.
            id: "witchybnd".into(),
            name: "WitchyBND".into(),
            description: "FromSoft archive repacker (BND/DCX/PARAM/MSB/etc). Essential for inspecting and repacking FromSoft mods.".into(),
            exe_names: vec!["WitchyBND.exe".into()],
            detected_path: None,
            requires_wine: true,
            category: "Tool".into(),
            can_auto_install: true,
            github_repo: Some("ividyon/WitchyBND".into()),
            nexus_mod_id: None,
            nexus_game_slug: None,
            download_url: None,
            license: "BSD-3-Clause".into(),
            wine_notes: Some("Pure .NET CLI/GUI tool — runs cleanly under Wine. No native deps.".into()),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("https://github.com/ividyon/WitchyBND".into()),
            requires_explicit_opt_in: false,
        },
        ModTool {
            // Successor to DSMapStudio. Param/map/event editor across all five
            // FromSoft titles we support.
            id: "smithbox".into(),
            name: "Smithbox".into(),
            description: "Param, map, and event editor for FromSoft games. Successor to DSMapStudio.".into(),
            exe_names: vec!["Smithbox.exe".into()],
            detected_path: None,
            requires_wine: true,
            category: "Editor".into(),
            can_auto_install: true,
            github_repo: Some("vawser/Smithbox".into()),
            nexus_mod_id: None,
            nexus_game_slug: None,
            download_url: None,
            license: "MIT".into(),
            wine_notes: Some("Veldrid renderer; works under DXVK. May need d3dcompiler_47 in the bottle.".into()),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("https://github.com/vawser/Smithbox".into()),
            requires_explicit_opt_in: false,
        },
        ModTool {
            // Param editor specifically tuned for Elden Ring's param schema.
            id: "yapped_rune_bear".into(),
            name: "Yapped Rune Bear".into(),
            description: "Param editor specifically tuned for Elden Ring. Read/write regulation.bin params with named field metadata.".into(),
            exe_names: vec!["Yapped.exe".into(), "Yapped Rune Bear.exe".into()],
            detected_path: None,
            requires_wine: true,
            category: "Editor".into(),
            can_auto_install: true,
            github_repo: Some("vawser/Yapped-Rune-Bear".into()),
            nexus_mod_id: None,
            nexus_game_slug: None,
            download_url: None,
            license: "MIT".into(),
            wine_notes: Some("WinForms .NET app — runs cleanly under Wine.".into()),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("https://github.com/vawser/Yapped-Rune-Bear".into()),
            requires_explicit_opt_in: false,
        },
        ModTool {
            // Animation viewer/editor for FromSoft games (HKX animations).
            id: "dsanimstudio".into(),
            name: "DSAnimStudio".into(),
            description: "Animation viewer and editor for FromSoft games (HKX/TAE). Used for animation mod authoring.".into(),
            exe_names: vec!["DSAnimStudio.exe".into()],
            detected_path: None,
            requires_wine: true,
            category: "Editor".into(),
            can_auto_install: true,
            github_repo: Some("Meowmaritus/DSAnimStudio".into()),
            nexus_mod_id: None,
            nexus_game_slug: None,
            download_url: None,
            license: "MIT".into(),
            wine_notes: Some("MonoGame DX11 renderer; works under DXVK with d3dcompiler_47 in the bottle.".into()),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("https://github.com/Meowmaritus/DSAnimStudio".into()),
            requires_explicit_opt_in: false,
        },
    ]
}

// ---------------------------------------------------------------------------
// Genshin Impact tools
// ---------------------------------------------------------------------------
//
// Genshin uses 3DMigoto (a generic DX11 hook) plus the GIMI fork (Genshin
// Impact Model Importer) for character/weapon swaps. GIMI is the
// recommended loader for users; raw 3DMigoto is listed for transparency
// but not auto-installed because it requires manual `d3dx.ini` editing
// for each game.

fn genshin_tools() -> Vec<ModTool> {
    vec![
        ModTool {
            id: "gimi".into(),
            name: "GIMI (Genshin Impact Model Importer)".into(),
            description:
                "3DMigoto fork preconfigured for Genshin Impact. Loads character, weapon, and texture mods from `<game>/Mods/`."
                    .into(),
            // GIMI ships the 3DMigoto loader as `d3d11.dll` plus the
            // 3DMigoto launcher executable. Presence of either indicates
            // an install.
            exe_names: vec![
                "d3d11.dll".into(),
                "3DMigoto Loader.exe".into(),
                "GIMI.exe".into(),
            ],
            detected_path: None,
            requires_wine: true,
            category: "Framework".into(),
            can_auto_install: true,
            // SilentNightSound's GIMI-Package is the canonical distribution
            // most modlists target. Forks exist; if the canonical repo
            // moves, swap this entry.
            github_repo: Some("SilentNightSound/GIMI-Package".into()),
            nexus_mod_id: None,
            nexus_game_slug: Some("genshinimpact".into()),
            download_url: None,
            license: "GPL-3.0".into(),
            wine_notes: Some(
                "DLL injection works under Wine/CrossOver; 3DMigoto's DX11 hook is well-supported by DXVK."
                    .into(),
            ),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("https://github.com/SilentNightSound/GIMI-Package".into()),
            requires_explicit_opt_in: false,
        },
        ModTool {
            id: "3dmigoto".into(),
            name: "3DMigoto".into(),
            description:
                "Upstream DX11 hook that GIMI is built on. Listed for reference — most users want GIMI, not raw 3DMigoto."
                    .into(),
            exe_names: vec!["3DMigoto Loader.exe".into(), "d3d11.dll".into()],
            detected_path: None,
            requires_wine: true,
            category: "Framework".into(),
            // Info-only: 3DMigoto requires per-game `d3dx.ini` configuration
            // that GIMI already supplies. Users almost always want GIMI.
            can_auto_install: false,
            github_repo: Some("bo3b/3Dmigoto".into()),
            nexus_mod_id: None,
            nexus_game_slug: None,
            download_url: Some("https://github.com/bo3b/3Dmigoto".into()),
            license: "BSD-3-Clause".into(),
            wine_notes: Some(
                "Generic 3DMigoto build is not preconfigured for Genshin. Use GIMI instead unless you know why you want the raw loader."
                    .into(),
            ),
            wine_compat: "good".into(),
            recommended_alternative: Some("gimi".into()),
            recommended_ini_edits: vec![],
            support_url: Some("https://github.com/bo3b/3Dmigoto".into()),
            requires_explicit_opt_in: false,
        },
    ]
}

/// Look up a tool definition by ID (searches all games).
fn find_tool_def(tool_id: &str) -> Result<ModTool> {
    all_builtin_tools()
        .into_iter()
        .find(|t| t.id == tool_id)
        .ok_or_else(|| ToolError::NotFound(tool_id.to_string()))
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Standard tools installation directory relative to game root.
const TOOLS_DIR: &str = "Tools";

/// Scan the game data directory (and parent) for known modding tools.
/// Returns the list of built-in tools with `detected_path` populated where found.
pub fn detect_tools(game_data_dir: &Path) -> Vec<ModTool> {
    detect_tools_for_game(game_data_dir, "skyrimse")
}

/// Scan the game data directory for modding tools relevant to the specified game.
pub fn detect_tools_for_game(game_data_dir: &Path, game_id: &str) -> Vec<ModTool> {
    let mut tools = builtin_tools_for_game(game_id);

    let game_dir = game_data_dir.parent().unwrap_or(game_data_dir);
    let tools_dir = game_dir.join(TOOLS_DIR);
    let mut search_roots: Vec<&Path> = vec![game_dir, game_data_dir];
    if tools_dir.exists() {
        search_roots.push(&tools_dir);
    }

    let exe_names_lower: Vec<Vec<String>> = tools
        .iter()
        .map(|t| t.exe_names.iter().map(|n| n.to_lowercase()).collect())
        .collect();

    for root in &search_roots {
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(root)
            .max_depth(4)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let file_name = entry.file_name().to_string_lossy().to_lowercase();
            for (i, names) in exe_names_lower.iter().enumerate() {
                if tools[i].detected_path.is_some() {
                    continue;
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
// Installation (GitHub releases)
// ---------------------------------------------------------------------------

/// Get the tools install directory for a game. Creates it if needed.
fn tools_install_dir(game_data_dir: &Path) -> io::Result<PathBuf> {
    let game_dir = game_data_dir.parent().unwrap_or(game_data_dir);
    let dir = game_dir.join(TOOLS_DIR);
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Pick the best asset from a GitHub release for a given tool.
/// Prefers 64-bit Windows archives (zip > 7z).
fn pick_asset<'a>(tool_id: &str, assets: &'a [GitHubAsset]) -> Option<&'a GitHubAsset> {
    let lower_assets: Vec<(usize, String)> = assets
        .iter()
        .enumerate()
        .map(|(i, a)| (i, a.name.to_lowercase()))
        .collect();

    // Filter to archives only
    let archives: Vec<(usize, &String)> = lower_assets
        .iter()
        .filter(|(_, n)| n.ends_with(".zip") || n.ends_with(".7z"))
        .map(|(i, n)| (*i, n))
        .collect();

    if archives.is_empty() {
        return None;
    }

    // Tool-specific heuristics
    let preferred: Vec<&(usize, &String)> = match tool_id {
        "sseedit" => archives
            .iter()
            .filter(|(_, n)| n.contains("sse") || n.contains("xedit"))
            .collect(),
        "skse" => {
            // SKSE releases have archives for different game versions; pick the largest .7z
            let sevenz: Vec<&(usize, &String)> = archives
                .iter()
                .filter(|(_, n)| n.ends_with(".7z"))
                .collect();
            if sevenz.is_empty() {
                archives.iter().collect()
            } else {
                // Return the largest archive (most likely the full release, not a delta)
                return sevenz
                    .iter()
                    .max_by_key(|(i, _)| assets[*i].size)
                    .map(|(i, _)| &assets[*i]);
            }
        }
        "pandora" => {
            // Pandora has multiple variants: self-contained (no suffix) and _net (needs .NET).
            // Prefer the self-contained one (largest, no _net/_arm64/_x86 suffix) for Wine compat.
            let self_contained: Vec<&(usize, &String)> = archives
                .iter()
                .filter(|(_, n)| {
                    n.contains("pandora")
                        && !n.contains("_net")
                        && !n.contains("arm64")
                        && !n.contains("x86")
                        && !n.contains("src")
                        && !n.contains("source")
                })
                .collect();
            if !self_contained.is_empty() {
                return self_contained
                    .iter()
                    .max_by_key(|(i, _)| assets[*i].size)
                    .map(|(i, _)| &assets[*i]);
            }
            // Fallback: any pandora archive
            archives
                .iter()
                .filter(|(_, n)| n.contains("pandora") && !n.contains("src"))
                .collect()
        }
        "wryebash" => {
            // Wrye Bash 312+ ships native Linux releases. Prefer those when
            // running on Linux so we don't install Windows-only Python builds
            // that need Wine to run a Python interpreter inside Wine.
            #[cfg(target_os = "linux")]
            {
                let linux_assets: Vec<&(usize, &String)> = archives
                    .iter()
                    .filter(|(_, n)| {
                        n.contains("linux")
                            && !n.contains("src")
                            && !n.contains("source")
                    })
                    .collect();
                if !linux_assets.is_empty() {
                    return linux_assets
                        .iter()
                        .max_by_key(|(i, _)| assets[*i].size)
                        .map(|(i, _)| &assets[*i]);
                }
            }
            archives
                .iter()
                .filter(|(_, n)| {
                    (n.contains("standalone") || n.contains("wrye"))
                        && !n.contains("src")
                        && !n.contains("source")
                        && !n.contains("linux") // Windows fallback shouldn't grab Linux build
                })
                .collect()
        }
        _ => archives
            .iter()
            .filter(|(_, n)| !n.contains("src") && !n.contains("source") && !n.contains("linux"))
            .collect(),
    };

    let candidates = if preferred.is_empty() {
        &archives
    } else {
        // Convert to same type - just use the preferred list
        return preferred
            .iter()
            // Prefer .zip over .7z
            .min_by_key(|(_, n)| if n.ends_with(".zip") { 0 } else { 1 })
            .map(|(i, _)| &assets[*i]);
    };

    candidates
        .iter()
        .min_by_key(|(_, n)| if n.ends_with(".zip") { 0 } else { 1 })
        .map(|(i, _)| &assets[*i])
}

/// Download and install a tool from GitHub releases or NexusMods into the game's
/// Tools directory.
///
/// Tries GitHub first (if `github_repo` is set). If that fails, falls back to
/// NexusMods (if `nexus_mod_id` is set and user has a Nexus API key with premium).
///
/// Returns the path to the installed tool's executable.
pub async fn install_tool(tool_id: &str, game_data_dir: &Path, app: &AppHandle) -> Result<String> {
    let tool_def = find_tool_def(tool_id)?;

    if !tool_def.can_auto_install {
        return Err(ToolError::NoAutoInstall(tool_id.to_string()));
    }

    let client = reqwest::Client::builder()
        .user_agent(format!("Corkscrew/{}", env!("CARGO_PKG_VERSION")))
        .build()?;

    let has_nexus = tool_def.nexus_mod_id.is_some() && tool_def.nexus_game_slug.is_some();

    // --- Phase 1: Download ---
    emit_progress(app, tool_id, "downloading", "Fetching from GitHub...");

    let (archive_bytes, archive_name, version_tag) =
        if let Some(github_repo) = &tool_def.github_repo {
            match install_tool_from_github(tool_id, github_repo, &client).await {
                Ok(result) => result,
                Err(gh_err) => {
                    if has_nexus {
                        let mod_id = tool_def.nexus_mod_id.unwrap();
                        let game_slug = tool_def.nexus_game_slug.as_ref().unwrap();
                        info!(
                            "GitHub install failed for '{}': {}. Trying NexusMods fallback...",
                            tool_id, gh_err
                        );
                        emit_progress(
                            app,
                            tool_id,
                            "downloading",
                            "GitHub unavailable, downloading from NexusMods...",
                        );
                        install_tool_from_nexus(tool_id, mod_id, game_slug, &client).await?
                    } else {
                        return Err(gh_err);
                    }
                }
            }
        } else if let (Some(mod_id), Some(game_slug)) =
            (&tool_def.nexus_mod_id, &tool_def.nexus_game_slug)
        {
            emit_progress(app, tool_id, "downloading", "Downloading from NexusMods...");
            install_tool_from_nexus(tool_id, *mod_id, game_slug, &client).await?
        } else {
            return Err(ToolError::NoAutoInstall(tool_id.to_string()));
        };

    // --- Phase 2: Extract ---
    let size_mb = archive_bytes.len() as f64 / 1_048_576.0;
    emit_progress(
        app,
        tool_id,
        "extracting",
        &format!("Extracting {} ({:.1} MB)...", archive_name, size_mb),
    );

    let tools_dir = tools_install_dir(game_data_dir)?;
    let tool_dir = tools_dir.join(&tool_def.id);
    if tool_dir.exists() {
        fs::remove_dir_all(&tool_dir)?;
    }
    fs::create_dir_all(&tool_dir)?;

    let name_lower = archive_name.to_lowercase();
    if name_lower.ends_with(".zip") {
        extract_zip(&archive_bytes, &tool_dir)?;
    } else if name_lower.ends_with(".7z") {
        extract_7z(&archive_bytes, &tool_dir)?;
    } else {
        return Err(ToolError::Other(format!(
            "Unsupported archive format: {}",
            archive_name
        )));
    }

    // Flatten single-directory archives (if archive contains just one folder)
    flatten_single_dir(&tool_dir)?;

    // --- Phase 3: Verify ---
    emit_progress(app, tool_id, "verifying", "Looking for executable...");

    // Tool-specific post-install: rename xEdit executables for game mode detection.
    // The prefix is everything before "Edit" — e.g. "SSE" produces "SSEEdit.exe".
    if tool_id == "sseedit" {
        rename_xedit_for_game(&tool_dir, "SSE");
    } else if tool_id == "fo4edit" {
        rename_xedit_for_game(&tool_dir, "FO4");
    }

    let exe_path = find_tool_exe(&tool_def, &tool_dir).ok_or(ToolError::ExeNotFound)?;

    // Write version marker for future update checks
    write_version_marker(&tool_dir, &version_tag);

    emit_progress(app, tool_id, "done", "Installed successfully");
    info!("Tool '{}' installed to: {}", tool_id, exe_path.display());

    Ok(exe_path.to_string_lossy().to_string())
}

/// Rename xEdit's generic executables to game-specific names.
///
/// xEdit 4.0.4+ ships as `xTESEdit.exe` / `xFOEdit.exe` and uses its own
/// filename to detect which game to target. We rename to `{prefix}Edit.exe`
/// so it auto-detects the correct game mode.
fn rename_xedit_for_game(tool_dir: &Path, prefix: &str) {
    let renames = [
        ("xTESEdit.exe", format!("{}Edit.exe", prefix)),
        ("xTESEdit64.exe", format!("{}Edit64.exe", prefix)),
        ("xFOEdit.exe", format!("{}Edit.exe", prefix)),
        ("xFOEdit64.exe", format!("{}Edit64.exe", prefix)),
    ];
    for (from, to) in &renames {
        let src = tool_dir.join(from);
        let dst = tool_dir.join(to);
        if src.exists() && !dst.exists() {
            if let Err(e) = fs::rename(&src, &dst) {
                info!("Could not rename {} to {}: {}", from, to, e);
            } else {
                info!("Renamed {} to {} for game mode detection", from, to);
            }
        }
    }
}

/// Download tool archive from GitHub releases.
/// Returns (bytes, filename, version_tag).
async fn install_tool_from_github(
    tool_id: &str,
    github_repo: &str,
    client: &reqwest::Client,
) -> Result<(Vec<u8>, String, String)> {
    info!(
        "Installing mod tool '{}' from GitHub: {}",
        tool_id, github_repo
    );

    let api_url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        github_repo
    );

    let release: GitHubRelease = client
        .get(&api_url)
        .send()
        .await?
        .error_for_status()
        .map_err(|e| ToolError::GitHub(format!("Failed to fetch release: {}", e)))?
        .json()
        .await?;

    info!(
        "Found release {} with {} assets",
        release.tag_name,
        release.assets.len()
    );

    let asset = pick_asset(tool_id, &release.assets)
        .ok_or_else(|| ToolError::GitHub("No suitable archive found in release".into()))?;

    info!("Downloading asset: {} ({} bytes)", asset.name, asset.size);

    let bytes = client
        .get(&asset.browser_download_url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?
        .to_vec();

    Ok((bytes, asset.name.clone(), release.tag_name))
}

/// Download tool archive from NexusMods (requires premium).
/// Returns (bytes, filename, version_string).
async fn install_tool_from_nexus(
    tool_id: &str,
    mod_id: i64,
    game_slug: &str,
    client: &reqwest::Client,
) -> Result<(Vec<u8>, String, String)> {
    info!(
        "Installing mod tool '{}' from NexusMods: {}/mods/{}",
        tool_id, game_slug, mod_id
    );

    // Use unified auth (OAuth or API key)
    let auth_method = crate::oauth::get_auth_method_refreshed().await;
    let nexus = crate::nexus::NexusClient::from_auth_method(&auth_method).map_err(|e| {
        ToolError::Other(format!(
            "NexusMods sign-in required to auto-install this tool. \
             Sign in via Settings → Authentication. ({})",
            e
        ))
    })?;

    // Check premium status — NexusMods compliance: free users cannot automate downloads
    if !nexus.is_premium().await {
        return Err(ToolError::Other(
            "NexusMods Premium required to auto-install this tool. \
             Free users can download it manually from the NexusMods website."
                .into(),
        ));
    }

    // Get the mod's files list and pick the latest file
    let files = nexus
        .get_mod_files(game_slug, mod_id)
        .await
        .map_err(|e| ToolError::Other(format!("Failed to fetch mod files: {}", e)))?;

    // NexusMods file categories:
    // 1 = MAIN, 2 = UPDATE, 3 = OPTIONAL, 4 = OLD_VERSION, 5 = MISCELLANEOUS, 6 = ARCHIVED
    // Try MAIN first, then UPDATE, then any non-old/archived category
    let pick_latest = |category_ids: &[i64]| -> Option<&serde_json::Value> {
        files
            .iter()
            .filter(|f| {
                f.get("category_id")
                    .and_then(|v| v.as_i64())
                    .map(|c| category_ids.contains(&c))
                    .unwrap_or(false)
            })
            .max_by_key(|f| {
                f.get("uploaded_timestamp")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0)
            })
    };

    let main_file = pick_latest(&[1])
        .or_else(|| pick_latest(&[2]))
        .or_else(|| pick_latest(&[1, 2, 3, 5]))
        .ok_or_else(|| {
            ToolError::Other(format!(
                "No installable file found on NexusMods for {}/mods/{}",
                game_slug, mod_id
            ))
        })?;

    let file_id = main_file
        .get("file_id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| ToolError::Other("Missing file_id in NexusMods response".into()))?;

    let file_name = main_file
        .get("file_name")
        .and_then(|v| v.as_str())
        .unwrap_or("download.zip")
        .to_string();

    // Use the file's uploaded_timestamp as version marker for NexusMods tools
    let version_marker = main_file
        .get("uploaded_timestamp")
        .and_then(|v| v.as_i64())
        .map(|t| t.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    info!("Found NexusMods file: {} (id: {})", file_name, file_id);

    // Get download links (premium-only automated download)
    let links = nexus
        .get_download_links(game_slug, mod_id, file_id, None, None)
        .await
        .map_err(|e| ToolError::Other(format!("Failed to get download links: {}", e)))?;

    let download_url = links
        .first()
        .map(|l| l.uri.clone())
        .ok_or_else(|| ToolError::Other("No download links returned".into()))?;

    // Download the file
    let bytes = client
        .get(&download_url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?
        .to_vec();

    info!(
        "Downloaded {} ({} bytes) from NexusMods",
        file_name,
        bytes.len()
    );

    Ok((bytes, file_name, version_marker))
}

/// Extract a ZIP archive from bytes into the target directory.
///
/// On Unix, preserves the file mode bits stored in the ZIP's external attributes
/// so executables (LOOT, Pandora, ASI Loader, etc. distributed via GitHub) launch
/// without manual `chmod`. Falls back to `0o755` for files when no Unix mode is
/// stored in the entry — most modding tools' archives lack metadata when packed
/// on Windows but still need to be executable under Wine/native.
fn extract_zip(data: &[u8], target: &Path) -> Result<()> {
    let cursor = io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor)?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.enclosed_name().map(|n| n.to_path_buf());
        let Some(name) = name else { continue };

        let out_path = target.join(&name);

        if file.is_dir() {
            fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut outfile = fs::File::create(&out_path)?;
            io::copy(&mut file, &mut outfile)?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                // Default to 0o755 so files pulled from Windows-packed zips
                // (which often have no Unix mode) are still executable.
                let mode = file.unix_mode().unwrap_or(0o755);
                if let Err(e) = fs::set_permissions(&out_path, fs::Permissions::from_mode(mode)) {
                    log::warn!(
                        "Failed to set mode {:o} on {}: {}",
                        mode,
                        out_path.display(),
                        e
                    );
                }
            }
        }
    }

    Ok(())
}

/// Extract a 7z archive from bytes into the target directory.
///
/// `sevenz_rust2::decompress_file` does not preserve mode bits, so on Unix we
/// walk the extracted tree and `chmod 0o755` any file that looks executable:
/// `*.exe`, files without an extension, or files with the ELF magic header.
fn extract_7z(data: &[u8], target: &Path) -> Result<()> {
    // Write to temp file since sevenz-rust needs a path
    let tmp_path = target.join("__download.7z");
    fs::write(&tmp_path, data)?;

    sevenz_rust2::decompress_file(&tmp_path, target)
        .map_err(|e| ToolError::SevenZ(e.to_string()))?;

    // Clean up temp file
    let _ = fs::remove_file(&tmp_path);

    // Validate extracted files stay within the target directory (path traversal check)
    let canonical_target = target
        .canonicalize()
        .unwrap_or_else(|_| target.to_path_buf());
    for entry in WalkDir::new(target).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            let path = entry.into_path();
            if let Ok(canonical) = path.canonicalize() {
                if !canonical.starts_with(&canonical_target) {
                    log::warn!(
                        "Removing 7z entry outside target directory: {}",
                        canonical.display()
                    );
                    let _ = fs::remove_file(&path);
                }
            }
        }
    }

    // On Unix, mark plausible executables 0o755 since 7z extraction lost mode bits.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for entry in WalkDir::new(target).into_iter().filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if looks_executable(path) {
                if let Err(e) = fs::set_permissions(path, fs::Permissions::from_mode(0o755)) {
                    log::warn!(
                        "Failed to chmod 0o755 {}: {}",
                        path.display(),
                        e
                    );
                }
            }
        }
    }

    Ok(())
}

/// Heuristic check for files that should be executable after extraction.
///
/// Used by the 7z extractor on Unix because `sevenz_rust2` doesn't preserve mode
/// bits. Matches Windows binaries (`.exe`), extensionless tools, and ELF binaries
/// (detected via the magic header `7f 45 4c 46`).
#[cfg(unix)]
fn looks_executable(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    // Windows binaries — needed for Wine launches.
    if name.to_ascii_lowercase().ends_with(".exe") {
        return true;
    }

    // Extensionless files are typically Unix tools.
    if !name.contains('.') {
        return true;
    }

    // ELF magic header (0x7F 'E' 'L' 'F').
    if let Ok(mut file) = fs::File::open(path) {
        use std::io::Read;
        let mut buf = [0u8; 4];
        if file.read_exact(&mut buf).is_ok() && buf == [0x7f, 0x45, 0x4c, 0x46] {
            return true;
        }
    }

    false
}

/// If the extracted directory contains a single subdirectory, move its contents up.
fn flatten_single_dir(dir: &Path) -> io::Result<()> {
    let entries: Vec<_> = fs::read_dir(dir)?.filter_map(|e| e.ok()).collect();

    if entries.len() == 1 && entries[0].file_type()?.is_dir() {
        let sub = entries[0].path();
        let tmp = dir.join("__flatten_tmp");
        fs::rename(&sub, &tmp)?;
        // Move everything from tmp into parent
        for entry in fs::read_dir(&tmp)?.filter_map(|e| e.ok()) {
            let dest = dir.join(entry.file_name());
            fs::rename(entry.path(), dest)?;
        }
        fs::remove_dir_all(&tmp)?;
    }

    Ok(())
}

/// Search the tool directory for the tool's known executable names.
fn find_tool_exe(tool: &ModTool, tool_dir: &Path) -> Option<PathBuf> {
    let exe_lower: Vec<String> = tool.exe_names.iter().map(|n| n.to_lowercase()).collect();

    for entry in WalkDir::new(tool_dir)
        .max_depth(3)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if exe_lower.iter().any(|n| n == &name) {
                return Some(entry.into_path());
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Uninstallation
// ---------------------------------------------------------------------------

/// Remove an installed tool. First tries the canonical Tools directory, then
/// falls back to the `detected_path` (the exe found by `detect_tools`).
pub fn uninstall_tool(
    tool_id: &str,
    game_data_dir: &Path,
    detected_path: Option<&str>,
) -> Result<()> {
    let game_dir = game_data_dir.parent().unwrap_or(game_data_dir);
    let tool_dir = game_dir.join(TOOLS_DIR).join(tool_id);

    // Try canonical Tools/<id>/ directory first
    if tool_dir.exists() {
        info!(
            "Uninstalling tool '{}' from: {}",
            tool_id,
            tool_dir.display()
        );
        fs::remove_dir_all(&tool_dir)?;
        return Ok(());
    }

    // Fallback: remove via detected exe path
    if let Some(path_str) = detected_path {
        let exe_path = Path::new(path_str);
        if exe_path.exists() {
            info!(
                "Uninstalling tool '{}' via detected path: {}",
                tool_id,
                exe_path.display()
            );
            fs::remove_file(exe_path)?;

            // If the parent is a tool-specific folder (not game root), remove if empty
            if let Some(parent) = exe_path.parent() {
                if parent != game_dir && parent != game_data_dir {
                    let _ = fs::remove_dir(parent); // only removes empty dirs
                }
            }
            return Ok(());
        }
    }

    Err(ToolError::Other(format!(
        "Tool '{}' not found — neither in Tools directory nor at detected path",
        tool_id
    )))
}

// ---------------------------------------------------------------------------
// Reinstallation
// ---------------------------------------------------------------------------

/// Reinstall a tool by uninstalling and re-installing from GitHub.
///
/// Returns the path to the newly installed tool's executable.
pub async fn reinstall_tool(
    tool_id: &str,
    game_data_dir: &Path,
    app: &AppHandle,
) -> Result<String> {
    info!("Reinstalling mod tool '{}'", tool_id);
    uninstall_tool(tool_id, game_data_dir, None)?;
    install_tool(tool_id, game_data_dir, app).await
}

// ---------------------------------------------------------------------------
// Update Check
// ---------------------------------------------------------------------------

/// Result of checking for a tool update.
#[derive(Clone, Debug, Serialize)]
pub struct ToolUpdateInfo {
    pub tool_id: String,
    pub tool_name: String,
    pub latest_version: String,
    pub update_available: bool,
}

/// Check for an available update for a tool by querying GitHub releases or
/// NexusMods file timestamps.
///
/// For tools with a GitHub repo, compares the latest release tag to the
/// currently installed version (stored in a `.version` marker file inside the
/// tool directory).
///
/// For NexusMods-only tools, compares the latest file upload timestamp to the
/// one stored during installation.
pub async fn check_tool_update(tool_id: &str, game_data_dir: &Path) -> Result<ToolUpdateInfo> {
    let tool_def = find_tool_def(tool_id)?;

    if !tool_def.can_auto_install {
        return Ok(ToolUpdateInfo {
            tool_id: tool_id.to_string(),
            tool_name: tool_def.name.clone(),
            latest_version: "N/A".to_string(),
            update_available: false,
        });
    }

    let client = reqwest::Client::builder()
        .user_agent(format!("Corkscrew/{}", env!("CARGO_PKG_VERSION")))
        .build()?;

    // Read the installed version marker
    let game_dir = game_data_dir.parent().unwrap_or(game_data_dir);
    let tool_dir = game_dir.join(TOOLS_DIR).join(&tool_def.id);
    let version_file = tool_dir.join(".version");
    let installed_version = fs::read_to_string(&version_file)
        .unwrap_or_default()
        .trim()
        .to_string();

    // Check GitHub first
    if let Some(github_repo) = &tool_def.github_repo {
        let api_url = format!(
            "https://api.github.com/repos/{}/releases/latest",
            github_repo
        );
        match client.get(&api_url).send().await {
            Ok(resp) => {
                if let Ok(release) = resp.json::<GitHubRelease>().await {
                    let latest = release.tag_name.clone();
                    let update_available =
                        !installed_version.is_empty() && installed_version != latest;
                    return Ok(ToolUpdateInfo {
                        tool_id: tool_id.to_string(),
                        tool_name: tool_def.name.clone(),
                        latest_version: latest,
                        update_available,
                    });
                }
            }
            Err(e) => {
                info!("GitHub check failed for '{}': {}", tool_id, e);
            }
        }
    }

    // Fallback: NexusMods — check latest file upload timestamp
    if let (Some(mod_id), Some(game_slug)) = (&tool_def.nexus_mod_id, &tool_def.nexus_game_slug) {
        let auth_method = crate::oauth::get_auth_method_refreshed().await;
        if let Ok(nexus) = crate::nexus::NexusClient::from_auth_method(&auth_method) {
            if let Ok(files) = nexus.get_mod_files(game_slug, *mod_id).await {
                // Find latest file upload timestamp
                let latest_ts = files
                    .iter()
                    .filter(|f| {
                        f.get("category_id")
                            .and_then(|v| v.as_i64())
                            .map(|c| [1, 2, 3, 5].contains(&c))
                            .unwrap_or(false)
                    })
                    .filter_map(|f| f.get("uploaded_timestamp").and_then(|v| v.as_i64()))
                    .max()
                    .unwrap_or(0);

                let latest_version_name = files
                    .iter()
                    .filter(|f| {
                        f.get("uploaded_timestamp")
                            .and_then(|v| v.as_i64())
                            .map(|t| t == latest_ts)
                            .unwrap_or(false)
                    })
                    .filter_map(|f| f.get("version").and_then(|v| v.as_str()))
                    .next()
                    .unwrap_or("unknown")
                    .to_string();

                let update_available =
                    !installed_version.is_empty() && installed_version != latest_ts.to_string();

                return Ok(ToolUpdateInfo {
                    tool_id: tool_id.to_string(),
                    tool_name: tool_def.name.clone(),
                    latest_version: latest_version_name,
                    update_available,
                });
            }
        }
    }

    Ok(ToolUpdateInfo {
        tool_id: tool_id.to_string(),
        tool_name: tool_def.name.clone(),
        latest_version: "Unable to check".to_string(),
        update_available: false,
    })
}

/// Write a version marker file after successful tool installation.
fn write_version_marker(tool_dir: &Path, version: &str) {
    let marker = tool_dir.join(".version");
    let _ = fs::write(&marker, version);
}

// ---------------------------------------------------------------------------
// INI Edit Application
// ---------------------------------------------------------------------------

/// Apply the recommended INI edits for a given tool to the game's INI files.
///
/// Returns the number of edits applied.
pub fn apply_tool_ini_edits(tool_id: &str, game_data_dir: &Path) -> Result<usize> {
    let tool_def = find_tool_def(tool_id)?;

    if tool_def.recommended_ini_edits.is_empty() {
        return Ok(0);
    }

    let game_dir = game_data_dir.parent().unwrap_or(game_data_dir);
    let mut applied = 0;

    for edit in &tool_def.recommended_ini_edits {
        let ini_path = game_dir.join(&edit.file);
        if !ini_path.exists() {
            debug!("INI file not found for edit: {}", ini_path.display());
            continue;
        }

        match apply_single_ini_edit(&ini_path, &edit.section, &edit.key, &edit.value) {
            Ok(true) => {
                info!(
                    "Applied INI edit: [{}]{} = {} in {}",
                    edit.section, edit.key, edit.value, edit.file
                );
                applied += 1;
            }
            Ok(false) => {
                debug!("INI edit already applied: [{}]{}", edit.section, edit.key);
            }
            Err(e) => {
                log::warn!(
                    "Failed to apply INI edit [{}]{}: {}",
                    edit.section,
                    edit.key,
                    e
                );
            }
        }
    }

    Ok(applied)
}

/// Apply a single key=value edit to an INI file within [section].
/// Returns Ok(true) if the edit was applied, Ok(false) if already set.
fn apply_single_ini_edit(
    ini_path: &Path,
    section: &str,
    key: &str,
    value: &str,
) -> std::result::Result<bool, std::io::Error> {
    let content = fs::read_to_string(ini_path)?;
    let section_header = format!("[{}]", section);
    let key_lower = key.to_lowercase();

    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    let mut in_section = false;
    let mut found = false;
    let mut section_end = lines.len();

    for (i, line) in lines.iter_mut().enumerate() {
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case(&section_header) {
            in_section = true;
            continue;
        }
        if in_section && trimmed.starts_with('[') {
            section_end = i;
            break;
        }
        if in_section {
            if let Some(eq_pos) = trimmed.find('=') {
                let k = trimmed[..eq_pos].trim().to_lowercase();
                if k == key_lower {
                    let existing_val = trimmed[eq_pos + 1..].trim();
                    if existing_val == value {
                        return Ok(false); // Already set
                    }
                    *line = format!("{}={}", key, value);
                    found = true;
                    break;
                }
            }
        }
    }

    if !found {
        // If section exists, insert the key at the end of it
        let mut section_found = false;
        for line in &lines {
            if line.trim().eq_ignore_ascii_case(&section_header) {
                section_found = true;
                break;
            }
        }

        if section_found {
            lines.insert(section_end, format!("{}={}", key, value));
        } else {
            // Add section at the end
            lines.push(String::new());
            lines.push(section_header);
            lines.push(format!("{}={}", key, value));
        }
    }

    fs::write(ini_path, lines.join("\n"))?;
    Ok(true)
}

// ---------------------------------------------------------------------------
// Launching
// ---------------------------------------------------------------------------

/// Launch a detected mod tool through Wine.
///
/// Uses the bottle's Wine binary to execute the tool. The tool must already
/// be detected (have a `detected_path`).
pub fn launch_tool(
    exe_path: &Path,
    bottle: &crate::bottles::Bottle,
) -> std::result::Result<crate::launcher::LaunchResult, String> {
    crate::launcher::launch_game(bottle, exe_path, exe_path.parent(), None, None).map_err(|e| e.to_string())
}

/// Launch a tool and log the result to the notification/crash log system.
///
/// Returns the launch result and logs any crash or error to the database.
pub fn launch_tool_with_logging(
    exe_path: &Path,
    bottle: &crate::bottles::Bottle,
    tool_id: &str,
    tool_name: &str,
    db: &crate::database::ModDatabase,
) -> std::result::Result<crate::launcher::LaunchResult, String> {
    let result = launch_tool(exe_path, bottle)?;

    if result.success {
        let _ = db.log_notification(
            "info",
            &format!("Launched {}", tool_name),
            Some(&format!(
                "Tool: {} | Bottle: {}",
                tool_id, result.bottle_name
            )),
        );
    } else {
        let _ = db.log_notification(
            "error",
            &format!("{} failed to launch", tool_name),
            Some(&format!(
                "Tool: {} | Bottle: {} | Check Wine compatibility",
                tool_id, result.bottle_name
            )),
        );
    }

    Ok(result)
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
        assert!(tools.len() >= 11);
        assert!(tools.iter().any(|t| t.id == "skse"));
        assert!(tools.iter().any(|t| t.id == "sseedit"));
        assert!(tools.iter().any(|t| t.id == "bethini"));
        assert!(tools.iter().any(|t| t.id == "pandora"));
        assert!(tools.iter().any(|t| t.id == "nifoptimizer"));
        // LOOT should NOT be in registry (integrated into Corkscrew)
        assert!(!tools.iter().any(|t| t.id == "loot"));
    }

    #[test]
    fn test_wine_compat_field_values() {
        for tool in builtin_tools() {
            assert!(
                ["good", "limited", "not_recommended"].contains(&tool.wine_compat.as_str()),
                "Tool '{}' has invalid wine_compat: '{}'",
                tool.id,
                tool.wine_compat
            );
        }
    }

    #[test]
    fn test_not_recommended_tools_have_alternative() {
        for tool in builtin_tools() {
            if tool.wine_compat == "not_recommended" {
                assert!(
                    tool.recommended_alternative.is_some(),
                    "Tool '{}' is not_recommended but has no recommended_alternative",
                    tool.id
                );
            }
        }
    }

    #[test]
    fn test_pandora_replaces_nemesis_and_fnis() {
        let tools = builtin_tools();
        let nemesis = tools.iter().find(|t| t.id == "nemesis").unwrap();
        let fnis = tools.iter().find(|t| t.id == "fnis").unwrap();
        assert_eq!(nemesis.recommended_alternative.as_deref(), Some("pandora"));
        assert_eq!(fnis.recommended_alternative.as_deref(), Some("pandora"));
        assert_eq!(nemesis.wine_compat, "not_recommended");
        assert_eq!(fnis.wine_compat, "not_recommended");
    }

    #[test]
    fn test_auto_install_tools_have_install_source() {
        for tool in builtin_tools() {
            if tool.can_auto_install {
                let has_source = tool.github_repo.is_some()
                    || (tool.nexus_mod_id.is_some() && tool.nexus_game_slug.is_some());
                assert!(
                    has_source,
                    "Tool '{}' can auto-install but has no github_repo or nexus_mod_id",
                    tool.id
                );
            }
        }
    }

    #[test]
    fn test_non_auto_install_tools_have_download_url() {
        for tool in builtin_tools() {
            if !tool.can_auto_install {
                assert!(
                    tool.download_url.is_some(),
                    "Tool '{}' cannot auto-install but has no download_url",
                    tool.id
                );
            }
        }
    }

    #[test]
    fn test_all_tools_have_license() {
        for tool in builtin_tools() {
            assert!(
                !tool.license.is_empty(),
                "Tool '{}' has empty license",
                tool.id
            );
        }
    }

    #[test]
    fn test_detect_tools_no_crash_on_missing_dir() {
        let tools = detect_tools(Path::new("/nonexistent/path"));
        assert!(tools.iter().all(|t| t.detected_path.is_none()));
    }

    #[test]
    fn test_find_tool_def() {
        assert!(find_tool_def("sseedit").is_ok());
        assert!(find_tool_def("nonexistent").is_err());
    }

    #[test]
    fn test_pick_asset_prefers_zip() {
        let assets = vec![
            GitHubAsset {
                name: "tool.7z".into(),
                browser_download_url: "https://example.com/tool.7z".into(),
                size: 1000,
            },
            GitHubAsset {
                name: "tool.zip".into(),
                browser_download_url: "https://example.com/tool.zip".into(),
                size: 1200,
            },
        ];
        let picked = pick_asset("cao", &assets).unwrap();
        assert!(picked.name.ends_with(".zip"));
    }

    #[test]
    fn test_pick_asset_filters_source() {
        let assets = vec![
            GitHubAsset {
                name: "tool-source.zip".into(),
                browser_download_url: "https://example.com/source.zip".into(),
                size: 5000,
            },
            GitHubAsset {
                name: "tool-release.zip".into(),
                browser_download_url: "https://example.com/release.zip".into(),
                size: 2000,
            },
        ];
        let picked = pick_asset("cao", &assets).unwrap();
        assert_eq!(picked.name, "tool-release.zip");
    }

    #[test]
    fn test_flatten_single_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Create: root/SubFolder/file.exe
        let sub = root.join("SubFolder");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("file.exe"), b"test").unwrap();

        flatten_single_dir(root).unwrap();

        // file.exe should now be at root level
        assert!(root.join("file.exe").exists());
        assert!(!root.join("SubFolder").exists());
    }

    #[test]
    fn test_uninstall_tool_no_crash_on_missing() {
        let tmp = tempfile::tempdir().unwrap();
        // game_data_dir would be tmp/Data, game_dir would be tmp/
        let data_dir = tmp.path().join("Data");
        fs::create_dir_all(&data_dir).unwrap();

        // Should error when tool dir doesn't exist and no detected path
        assert!(uninstall_tool("sseedit", &data_dir, None).is_err());
    }

    #[test]
    fn test_extract_zip_roundtrip() {
        use std::io::Write;

        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("extracted");
        fs::create_dir_all(&target).unwrap();

        // Create a minimal zip in memory
        let mut buf = io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buf);
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file("test.txt", options).unwrap();
            writer.write_all(b"hello world").unwrap();
            writer.finish().unwrap();
        }

        let data = buf.into_inner();
        extract_zip(&data, &target).unwrap();

        let content = fs::read_to_string(target.join("test.txt")).unwrap();
        assert_eq!(content, "hello world");
    }

    // --- Tool Requirement Detection Tests ---

    fn mock_collection_manifest(mods: Vec<(&str, Option<i64>)>) -> CollectionManifest {
        use crate::collections::{CollectionModEntry, CollectionSource};
        CollectionManifest {
            name: "Test Collection".to_string(),
            slug: Some("test-collection".to_string()),
            author: "Test".to_string(),
            description: "Test".to_string(),
            game_domain: "skyrimspecialedition".to_string(),
            image_url: None,
            revision: None,
            mod_rules: vec![],
            install_instructions: None,
            mods: mods
                .into_iter()
                .map(|(name, mod_id)| CollectionModEntry {
                    name: name.to_string(),
                    version: "1.0".to_string(),
                    optional: false,
                    source: CollectionSource {
                        source_type: "nexus".to_string(),
                        url: None,
                        instructions: None,
                        mod_id,
                        file_id: Some(1),
                        update_policy: None,
                        md5: None,
                        file_size: None,
                    },
                    choices: None,
                    patches: None,
                    instructions: None,
                    phase: None,
                    file_overrides: vec![],
                    install_disabled: false,
                    mod_type: None,
                    details: None,
                })
                .collect(),
            plugins: vec![],
            game_versions: vec![],
            ini_tweaks: vec![],
            bundled_files: std::collections::HashMap::new(),
        }
    }

    fn mock_parsed_modlist(archives: Vec<(&str, Option<i64>)>) -> ParsedModlist {
        use crate::wabbajack::ArchiveSummary;
        ParsedModlist {
            name: "Test Modlist".to_string(),
            author: "Test".to_string(),
            description: "Test".to_string(),
            version: "1.0".to_string(),
            game_type: 1,
            game_name: "Skyrim Special Edition".to_string(),
            is_nsfw: false,
            archive_count: archives.len(),
            total_download_size: 0,
            directive_count: 0,
            directive_breakdown: std::collections::HashMap::new(),
            archives: archives
                .into_iter()
                .map(|(name, nexus_mod_id)| ArchiveSummary {
                    name: name.to_string(),
                    size: 1000,
                    source_type: if nexus_mod_id.is_some() {
                        "Nexus".to_string()
                    } else {
                        "HTTP".to_string()
                    },
                    nexus_mod_id,
                    nexus_file_id: Some(1),
                })
                .collect(),
            required_game_version: None,
        }
    }

    #[test]
    fn test_detect_collection_tools_by_mod_id() {
        let manifest =
            mock_collection_manifest(vec![("SKSE64", Some(30379)), ("SkyUI", Some(12604))]);
        let tools = detect_required_tools_collection(&manifest, Path::new("/nonexistent"), "skyrimse");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool_id, "skse");
    }

    #[test]
    fn test_detect_collection_tools_by_name() {
        let manifest = mock_collection_manifest(vec![
            ("Nemesis Unlimited Behavior Engine", None),
            ("SkyUI", None),
        ]);
        let tools = detect_required_tools_collection(&manifest, Path::new("/nonexistent"), "skyrimse");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool_id, "nemesis");
        assert_eq!(tools[0].recommended_alternative.as_deref(), Some("pandora"));
    }

    #[test]
    fn test_detect_collection_tools_dedup() {
        // Same tool matched by both mod_id and name
        let manifest = mock_collection_manifest(vec![("SSEEdit xEdit", Some(164))]);
        let tools = detect_required_tools_collection(&manifest, Path::new("/nonexistent"), "skyrimse");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool_id, "sseedit");
    }

    #[test]
    fn test_detect_wabbajack_tools_by_mod_id() {
        let modlist = mock_parsed_modlist(vec![
            ("skse64_2_02_06.7z", Some(30379)),
            ("SkyUI_5_2_SE.7z", Some(12604)),
        ]);
        let tools = detect_required_tools_wabbajack(&modlist, Path::new("/nonexistent"));
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool_id, "skse");
    }

    #[test]
    fn test_detect_wabbajack_tools_by_name() {
        let modlist =
            mock_parsed_modlist(vec![("skse64_2_02_06.7z", None), ("random_mod.zip", None)]);
        let tools = detect_required_tools_wabbajack(&modlist, Path::new("/nonexistent"));
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool_id, "skse");
    }

    #[test]
    fn test_detect_no_tools() {
        let manifest = mock_collection_manifest(vec![("SkyUI", Some(12604)), ("USSEP", Some(266))]);
        let tools = detect_required_tools_collection(&manifest, Path::new("/nonexistent"), "skyrimse");
        assert!(tools.is_empty());
    }

    #[test]
    fn test_non_skyrim_game_skips_skyrim_tools() {
        // BodySlide (mod_id 201) is a Skyrim tool — should NOT match for Hogwarts Legacy
        let manifest = mock_collection_manifest(vec![
            ("BodySlide and Outfit Studio", Some(201)),
            ("SSEEdit", Some(164)),
        ]);
        let tools = detect_required_tools_collection(&manifest, Path::new("/nonexistent"), "hogwartslegacy");
        assert!(tools.is_empty(), "Skyrim tools should not appear for hogwartslegacy");
    }

    #[test]
    fn test_tool_signatures_cover_all_builtins() {
        // Every builtin tool should have a matching signature
        let tools = builtin_tools();
        for tool in &tools {
            assert!(
                TOOL_SIGNATURES.iter().any(|s| s.tool_id == tool.id),
                "Builtin tool '{}' has no entry in TOOL_SIGNATURES",
                tool.id
            );
        }
    }

    #[test]
    fn test_required_tool_enriches_from_builtin() {
        let manifest = mock_collection_manifest(vec![("SSEEdit", Some(164))]);
        let tools = detect_required_tools_collection(&manifest, Path::new("/nonexistent"), "skyrimse");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool_name, "SSEEdit (xEdit)");
        assert!(tools[0].can_auto_install);
        assert_eq!(tools[0].wine_compat, "good");
    }

    #[test]
    fn test_skse_in_builtin_can_auto_install() {
        let tools = builtin_tools();
        let skse = tools.iter().find(|t| t.id == "skse").unwrap();
        assert!(skse.can_auto_install);
        assert_eq!(skse.github_repo.as_deref(), Some("ianpatt/skse64"));
    }

    #[test]
    fn test_integrated_tools_not_detected() {
        // LOOT should never appear in required tools even if a mod name matches
        let manifest = mock_collection_manifest(vec![
            ("LOOT - Load Order Optimization Tool", None),
            ("SSEEdit", Some(164)),
        ]);
        let tools = detect_required_tools_collection(&manifest, Path::new("/nonexistent"), "skyrimse");
        assert!(!tools.iter().any(|t| t.tool_id == "loot"));
        assert!(tools.iter().any(|t| t.tool_id == "sseedit"));
    }

    #[test]
    fn test_pandora_suppresses_nemesis_and_fnis() {
        let manifest = mock_collection_manifest(vec![
            ("Pandora Behaviour Engine", None),
            ("Nemesis Unlimited Behavior Engine", None),
            ("FNIS", None),
        ]);
        let tools = detect_required_tools_collection(&manifest, Path::new("/nonexistent"), "skyrimse");
        assert!(tools.iter().any(|t| t.tool_id == "pandora"));
        assert!(!tools.iter().any(|t| t.tool_id == "nemesis"));
        assert!(!tools.iter().any(|t| t.tool_id == "fnis"));
    }

    #[test]
    fn test_skse_detected_enriched_from_builtin() {
        let manifest = mock_collection_manifest(vec![("SKSE64", Some(30379))]);
        let tools = detect_required_tools_collection(&manifest, Path::new("/nonexistent"), "skyrimse");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool_id, "skse");
        assert!(tools[0].can_auto_install);
        assert_eq!(tools[0].tool_name, "SKSE64");
    }

    #[test]
    fn test_fromsoft_tools_registry_complete() {
        // All five S-class FromSoft tools must be wired with the canonical IDs
        // the frontend / install pipeline references.
        let tools = fromsoft_tools();
        let ids: Vec<&str> = tools.iter().map(|t| t.id.as_str()).collect();
        for needed in [
            "modengine2",
            "witchybnd",
            "smithbox",
            "yapped_rune_bear",
            "dsanimstudio",
        ] {
            assert!(ids.contains(&needed), "FromSoft tool {} missing", needed);
        }
        // Each must declare a GitHub repo for auto-install + a license string.
        for t in &tools {
            assert!(
                t.github_repo.is_some(),
                "FromSoft tool {} missing github_repo",
                t.id
            );
            assert!(!t.license.is_empty(), "FromSoft tool {} missing license", t.id);
            assert!(t.requires_wine, "FromSoft tool {} should require wine", t.id);
        }
    }

    #[test]
    fn test_fromsoft_tools_categorized() {
        let tools = fromsoft_tools();
        // ME2 is the loader framework; the other four are tooling.
        let me2 = tools.iter().find(|t| t.id == "modengine2").unwrap();
        assert_eq!(me2.category, "Framework");
        let witchy = tools.iter().find(|t| t.id == "witchybnd").unwrap();
        assert_eq!(witchy.category, "Tool");
        for editor_id in ["smithbox", "yapped_rune_bear", "dsanimstudio"] {
            let t = tools.iter().find(|t| t.id == editor_id).unwrap();
            assert_eq!(t.category, "Editor", "{} should be Editor", editor_id);
        }
    }

    #[test]
    fn test_pick_asset_skse_prefers_7z() {
        let assets = vec![
            GitHubAsset {
                name: "skse64_2_02_06.7z".into(),
                browser_download_url: "https://example.com/skse.7z".into(),
                size: 5000,
            },
            GitHubAsset {
                name: "Source code (zip)".into(),
                browser_download_url: "https://example.com/source.zip".into(),
                size: 10000,
            },
        ];
        let picked = pick_asset("skse", &assets).unwrap();
        assert!(picked.name.contains("skse64"));
    }
}
