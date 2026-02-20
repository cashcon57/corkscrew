//! Mod tools registry — detection, installation, and launching of common modding tools.
//!
//! Provides a registry of known modding tools (SSEEdit, BethINI, DynDOLOD, etc.)
//! with automatic detection, GitHub-based auto-installation, and Wine-based launching.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use log::{debug, info};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use walkdir::WalkDir;

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
        "loot" => archives
            .iter()
            .filter(|(_, n)| n.contains("win") && (n.contains("64") || n.contains("x64")))
            .collect(),
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
        .user_agent("Corkscrew-ModManager/1.0")
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

/// Remove an installed tool from the game's Tools directory.
pub fn uninstall_tool(tool_id: &str, game_data_dir: &Path) -> Result<()> {
    let game_dir = game_data_dir.parent().unwrap_or(game_data_dir);
    let tool_dir = game_dir.join(TOOLS_DIR).join(tool_id);

    if tool_dir.exists() {
        info!(
            "Uninstalling tool '{}' from: {}",
            tool_id,
            tool_dir.display()
        );
        fs::remove_dir_all(&tool_dir)?;
    } else {
        debug!("Tool directory does not exist: {}", tool_dir.display());
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Reinstallation
// ---------------------------------------------------------------------------

/// Reinstall a tool by uninstalling and re-installing from GitHub.
///
/// Returns the path to the newly installed tool's executable.
pub async fn reinstall_tool(tool_id: &str, game_data_dir: &Path) -> Result<String> {
    info!("Reinstalling mod tool '{}'", tool_id);
    uninstall_tool(tool_id, game_data_dir)?;
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
                log::warn!("Failed to apply INI edit [{}]{}: {}", edit.section, edit.key, e);
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
            Some(&format!("Tool: {} | Bottle: {}", tool_id, result.bottle_name)),
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
        assert!(tools.len() >= 10);
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

        // Should not error even if tool dir doesn't exist
        assert!(uninstall_tool("sseedit", &data_dir).is_ok());
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
}
