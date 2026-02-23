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
use thiserror::Error;
use walkdir::WalkDir;

use crate::collections::CollectionManifest;
use crate::wabbajack::ParsedModlist;

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
    /// Whether this tool can be auto-installed from GitHub.
    pub can_auto_install: bool,
    /// GitHub "owner/repo" for tools that support auto-install.
    pub github_repo: Option<String>,
    /// Direct download URL for tools not on GitHub.
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
}

/// Known tool signatures for detecting required tools in collections/wabbajack lists.
pub const TOOL_SIGNATURES: &[ToolSignature] = &[
    ToolSignature {
        tool_id: "skse",
        tool_name: "SKSE64",
        nexus_mod_ids: &[30379],
        name_patterns: &["skse64", "skse_"],
    },
    ToolSignature {
        tool_id: "sseedit",
        tool_name: "SSEEdit (xEdit)",
        nexus_mod_ids: &[164],
        name_patterns: &["sseedit", "xedit", "tes5edit"],
    },
    ToolSignature {
        tool_id: "bodyslide",
        tool_name: "BodySlide & Outfit Studio",
        nexus_mod_ids: &[201],
        name_patterns: &["bodyslide", "outfit studio"],
    },
    ToolSignature {
        tool_id: "nemesis",
        tool_name: "Nemesis",
        nexus_mod_ids: &[60033],
        name_patterns: &["nemesis unlimited behavior"],
    },
    ToolSignature {
        tool_id: "fnis",
        tool_name: "FNIS",
        nexus_mod_ids: &[3038],
        name_patterns: &["fnis", "generatefnis"],
    },
    ToolSignature {
        tool_id: "pandora",
        tool_name: "Pandora Behaviour Engine+",
        nexus_mod_ids: &[],
        name_patterns: &["pandora behaviour", "pandora behavior"],
    },
    ToolSignature {
        tool_id: "dyndolod",
        tool_name: "DynDOLOD",
        nexus_mod_ids: &[68518, 32382],
        name_patterns: &["dyndolod"],
    },
    ToolSignature {
        tool_id: "wryebash",
        tool_name: "Wrye Bash",
        nexus_mod_ids: &[],
        name_patterns: &["wrye bash", "wryebash"],
    },
    ToolSignature {
        tool_id: "cao",
        tool_name: "Cathedral Assets Optimizer",
        nexus_mod_ids: &[],
        name_patterns: &["cathedral assets optimizer"],
    },
    ToolSignature {
        tool_id: "bethini",
        tool_name: "BethINI Pie",
        nexus_mod_ids: &[631],
        name_patterns: &["bethini"],
    },
    ToolSignature {
        tool_id: "nifoptimizer",
        tool_name: "SSE NIF Optimizer",
        nexus_mod_ids: &[],
        name_patterns: &["nif optimizer", "nifoptimizer"],
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
        || detected_tools.iter().any(|t| t.id == "pandora" && t.detected_path.is_some());

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

/// Built-in tool definitions for Skyrim SE modding.
fn builtin_tools() -> Vec<ModTool> {
    vec![
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
            download_url: None,
            license: "Proprietary".into(),
            wine_notes: Some("Works under Wine/Proton".into()),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("https://skse.silverlock.org/".into()),
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
            ],
            detected_path: None,
            requires_wine: true,
            category: "Cleaning".into(),
            can_auto_install: true,
            github_repo: Some("TES5Edit/TES5Edit".into()),
            download_url: None,
            license: "MPL-2.0".into(),
            wine_notes: Some("Works well under Wine/Proton".into()),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("https://ko-fi.com/elminsterau".into()),
        },
        ModTool {
            id: "pandora".into(),
            name: "Pandora Behaviour Engine+".into(),
            description: "Animation engine — replaces FNIS and Nemesis with better Wine support"
                .into(),
            exe_names: vec![
                "Pandora Behaviour Engine.exe".into(),
                "Pandora.exe".into(),
            ],
            detected_path: None,
            requires_wine: true,
            category: "Animation".into(),
            can_auto_install: true,
            github_repo: Some("Monitor221hz/Pandora-Behaviour-Engine-Plus".into()),
            download_url: None,
            license: "MIT".into(),
            wine_notes: Some("Works under Wine/Proton; backwards-compatible with FNIS and Nemesis animation mods".into()),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("https://www.patreon.com/monitorhz".into()),
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
            github_repo: Some("ousnius/BodySlide-and-Outfit-Studio".into()),
            download_url: None,
            license: "GPL-3.0".into(),
            wine_notes: Some("Works under Wine with some setup".into()),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: None,
        },
        ModTool {
            id: "cao".into(),
            name: "Cathedral Assets Optimizer".into(),
            description: "Texture and mesh optimization".into(),
            exe_names: vec!["Cathedral Assets Optimizer.exe".into()],
            detected_path: None,
            requires_wine: true,
            category: "Optimization".into(),
            can_auto_install: true,
            github_repo: Some("Guekka/Cathedral-Assets-Optimizer".into()),
            download_url: None,
            license: "MPL-2.0".into(),
            wine_notes: Some("Generally works under Wine".into()),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("https://github.com/sponsors/Guekka".into()),
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
            github_repo: Some("ousnius/SSE-NIF-Optimizer".into()),
            download_url: None,
            license: "GPL-3.0".into(),
            wine_notes: None,
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: None,
        },
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
            download_url: None,
            license: "GPL-3.0".into(),
            wine_notes: Some("Native Linux support since v312; also works via Wine".into()),
            wine_compat: "good".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: None,
        },
        // ---- Tools with limited Wine compatibility ----
        ModTool {
            id: "bethini".into(),
            name: "BethINI Pie".into(),
            description: "INI configuration optimizer".into(),
            exe_names: vec!["BethINI.exe".into(), "Bethini Pie.exe".into()],
            detected_path: None,
            requires_wine: true,
            category: "INI".into(),
            can_auto_install: false,
            github_repo: None,
            download_url: Some("https://www.nexusmods.com/site/mods/631".into()),
            license: "CC BY-NC-SA 4.0".into(),
            wine_notes: Some("Python-based; may work under Wine. Corkscrew's built-in INI editor provides similar functionality natively.".into()),
            wine_compat: "limited".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: None,
        },
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
            download_url: Some("https://www.nexusmods.com/skyrimspecialedition/mods/68518".into()),
            license: "Proprietary".into(),
            wine_notes: Some("Texconv issues under Wine; limited functionality on Linux".into()),
            wine_compat: "limited".into(),
            recommended_alternative: None,
            recommended_ini_edits: vec![],
            support_url: Some("https://ko-fi.com/sheson".into()),
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
            download_url: Some("https://www.nexusmods.com/skyrimspecialedition/mods/60033".into()),
            license: "GPL-3.0".into(),
            wine_notes: Some("Poor Wine compatibility. Use Pandora instead — it is backwards-compatible with Nemesis animation mods.".into()),
            wine_compat: "not_recommended".into(),
            recommended_alternative: Some("pandora".into()),
            recommended_ini_edits: vec![],
            support_url: Some("https://www.patreon.com/shikyokira".into()),
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
            download_url: Some("https://www.nexusmods.com/skyrimspecialedition/mods/3038".into()),
            license: "Proprietary".into(),
            wine_notes: Some(
                "Deprecated and poor Wine compatibility. Use Pandora instead — it is backwards-compatible with FNIS animation mods.".into(),
            ),
            wine_compat: "not_recommended".into(),
            recommended_alternative: Some("pandora".into()),
            recommended_ini_edits: vec![],
            support_url: None,
        },
    ]
}

/// Look up a tool definition by ID.
fn find_tool_def(tool_id: &str) -> Result<ModTool> {
    builtin_tools()
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
    let mut tools = builtin_tools();

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
            // Pandora releases may include source zips — prefer the release archive
            archives
                .iter()
                .filter(|(_, n)| {
                    !n.contains("src") && !n.contains("source") && !n.contains("linux")
                        && (n.contains("pandora") || n.contains("release"))
                })
                .collect()
        }
        "wryebash" => archives
            .iter()
            .filter(|(_, n)| {
                (n.contains("standalone") || n.contains("wrye"))
                    && !n.contains("src")
                    && !n.contains("source")
            })
            .collect(),
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

/// Download and install a tool from GitHub releases into the game's Tools directory.
///
/// Returns the path to the installed tool's executable.
pub async fn install_tool(tool_id: &str, game_data_dir: &Path) -> Result<String> {
    let tool_def = find_tool_def(tool_id)?;

    if !tool_def.can_auto_install {
        return Err(ToolError::NoAutoInstall(tool_id.to_string()));
    }

    let github_repo = tool_def
        .github_repo
        .as_ref()
        .ok_or_else(|| ToolError::NoAutoInstall(tool_id.to_string()))?;

    info!(
        "Installing mod tool '{}' from GitHub: {}",
        tool_id, github_repo
    );

    // 1. Fetch latest release from GitHub API
    let api_url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        github_repo
    );

    let client = reqwest::Client::builder()
        .user_agent(format!("Corkscrew/{}", env!("CARGO_PKG_VERSION")))
        .build()?;

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

    // 2. Pick the best asset
    let asset = pick_asset(tool_id, &release.assets)
        .ok_or_else(|| ToolError::GitHub("No suitable archive found in release".into()))?;

    info!("Downloading asset: {} ({} bytes)", asset.name, asset.size);

    // 3. Download the archive to a temp file
    let bytes = client
        .get(&asset.browser_download_url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    // 4. Prepare target directory
    let tools_dir = tools_install_dir(game_data_dir)?;
    let tool_dir = tools_dir.join(&tool_def.id);
    if tool_dir.exists() {
        fs::remove_dir_all(&tool_dir)?;
    }
    fs::create_dir_all(&tool_dir)?;

    // 5. Extract archive
    let asset_lower = asset.name.to_lowercase();
    if asset_lower.ends_with(".zip") {
        extract_zip(&bytes, &tool_dir)?;
    } else if asset_lower.ends_with(".7z") {
        extract_7z(&bytes, &tool_dir)?;
    } else {
        return Err(ToolError::Other(format!(
            "Unsupported archive format: {}",
            asset.name
        )));
    }

    // 6. Flatten single-directory archives (if archive contains just one folder)
    flatten_single_dir(&tool_dir)?;

    // 7. Find the executable in the extracted files
    let exe_path = find_tool_exe(&tool_def, &tool_dir).ok_or(ToolError::ExeNotFound)?;

    info!("Tool '{}' installed to: {}", tool_id, exe_path.display());

    Ok(exe_path.to_string_lossy().to_string())
}

/// Extract a ZIP archive from bytes into the target directory.
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
        }
    }

    Ok(())
}

/// Extract a 7z archive from bytes into the target directory.
fn extract_7z(data: &[u8], target: &Path) -> Result<()> {
    // Write to temp file since sevenz-rust needs a path
    let tmp_path = target.join("__download.7z");
    fs::write(&tmp_path, data)?;

    sevenz_rust::decompress_file(&tmp_path, target)
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

    Ok(())
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
pub async fn reinstall_tool(tool_id: &str, game_data_dir: &Path) -> Result<String> {
    info!("Reinstalling mod tool '{}'", tool_id);
    uninstall_tool(tool_id, game_data_dir, None)?;
    install_tool(tool_id, game_data_dir).await
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
    crate::launcher::launch_game(bottle, exe_path, exe_path.parent()).map_err(|e| e.to_string())
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
    fn test_auto_install_tools_have_github_repo() {
        for tool in builtin_tools() {
            if tool.can_auto_install {
                assert!(
                    tool.github_repo.is_some(),
                    "Tool '{}' can auto-install but has no github_repo",
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
                })
                .collect(),
            plugins: vec![],
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
        }
    }

    #[test]
    fn test_detect_collection_tools_by_mod_id() {
        let manifest =
            mock_collection_manifest(vec![("SKSE64", Some(30379)), ("SkyUI", Some(12604))]);
        let tools = detect_required_tools_collection(&manifest, Path::new("/nonexistent"));
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool_id, "skse");
    }

    #[test]
    fn test_detect_collection_tools_by_name() {
        let manifest = mock_collection_manifest(vec![
            ("Nemesis Unlimited Behavior Engine", None),
            ("SkyUI", None),
        ]);
        let tools = detect_required_tools_collection(&manifest, Path::new("/nonexistent"));
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool_id, "nemesis");
        assert_eq!(tools[0].recommended_alternative.as_deref(), Some("pandora"));
    }

    #[test]
    fn test_detect_collection_tools_dedup() {
        // Same tool matched by both mod_id and name
        let manifest = mock_collection_manifest(vec![("SSEEdit xEdit", Some(164))]);
        let tools = detect_required_tools_collection(&manifest, Path::new("/nonexistent"));
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
        let tools = detect_required_tools_collection(&manifest, Path::new("/nonexistent"));
        assert!(tools.is_empty());
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
        let tools = detect_required_tools_collection(&manifest, Path::new("/nonexistent"));
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
        let tools = detect_required_tools_collection(&manifest, Path::new("/nonexistent"));
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
        let tools = detect_required_tools_collection(&manifest, Path::new("/nonexistent"));
        assert!(tools.iter().any(|t| t.tool_id == "pandora"));
        assert!(!tools.iter().any(|t| t.tool_id == "nemesis"));
        assert!(!tools.iter().any(|t| t.tool_id == "fnis"));
    }

    #[test]
    fn test_skse_detected_enriched_from_builtin() {
        let manifest = mock_collection_manifest(vec![("SKSE64", Some(30379))]);
        let tools = detect_required_tools_collection(&manifest, Path::new("/nonexistent"));
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool_id, "skse");
        assert!(tools[0].can_auto_install);
        assert_eq!(tools[0].tool_name, "SKSE64");
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
