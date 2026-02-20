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
        },
        ModTool {
            id: "loot".into(),
            name: "LOOT".into(),
            description: "Load order optimization (standalone GUI)".into(),
            exe_names: vec!["LOOT.exe".into()],
            detected_path: None,
            requires_wine: true,
            category: "Load Order".into(),
            can_auto_install: true,
            github_repo: Some("loot/loot".into()),
            download_url: None,
            license: "GPL-3.0".into(),
            wine_notes: Some("Native Linux builds available on Flathub".into()),
        },
        // --- Tools that cannot be auto-installed (proprietary / NC license) ---
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
            wine_notes: Some("Python-based; may work under Wine".into()),
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
        },
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
            wine_notes: Some("Poor Wine compatibility; consider Pandora as alternative".into()),
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
                "Works with workarounds; deprecated in favor of Nemesis/Pandora".into(),
            ),
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
        assert!(tools.iter().any(|t| t.id == "loot"));
        assert!(tools.iter().any(|t| t.id == "nifoptimizer"));
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
