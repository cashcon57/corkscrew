//! LOOT integration for plugin sorting and metadata.
//!
//! Wraps [`libloot`] to provide automatic plugin load-order sorting using
//! LOOT masterlists, and exposes plugin-level warnings/messages to the UI.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use libloot::{EvalMode, GameType, MergeMode};
use serde::{Deserialize, Serialize};

use crate::bottles::Bottle;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Result of a LOOT sort operation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SortResult {
    /// Plugin filenames in the new sorted order.
    pub sorted_order: Vec<String>,
    /// Number of plugins that changed position.
    pub plugins_moved: usize,
    /// Warnings/messages from LOOT for all plugins.
    pub warnings: Vec<PluginWarning>,
}

/// A single LOOT warning or message about a plugin.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginWarning {
    pub plugin_name: String,
    /// "info", "warn", or "error"
    pub level: String,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Game type mapping
// ---------------------------------------------------------------------------

/// Map a Corkscrew `game_id` to a libloot `GameType`.
pub fn game_type_for(game_id: &str) -> Option<GameType> {
    match game_id {
        "skyrimse" => Some(GameType::SkyrimSE),
        "skyrim" => Some(GameType::Skyrim),
        "skyrimvr" => Some(GameType::SkyrimVR),
        "fallout4" => Some(GameType::Fallout4),
        "fallout4vr" => Some(GameType::Fallout4VR),
        "falloutnv" => Some(GameType::FalloutNV),
        "fallout3" => Some(GameType::Fallout3),
        "oblivion" => Some(GameType::Oblivion),
        "morrowind" => Some(GameType::Morrowind),
        "starfield" => Some(GameType::Starfield),
        _ => None,
    }
}

/// Map a Corkscrew `game_id` to the LOOT masterlist GitHub repository name.
fn masterlist_repo(game_id: &str) -> Option<&'static str> {
    match game_id {
        "skyrimse" => Some("skyrimse"),
        "skyrim" => Some("skyrim"),
        "skyrimvr" => Some("skyrimvr"),
        "fallout4" => Some("fallout4"),
        "fallout4vr" => Some("fallout4vr"),
        "falloutnv" => Some("falloutnv"),
        "fallout3" => Some("fallout3"),
        "oblivion" => Some("oblivion"),
        "morrowind" => Some("morrowind"),
        "starfield" => Some("starfield"),
        _ => None,
    }
}

/// Raw GitHub URL for a game's LOOT masterlist.
fn masterlist_url(game_id: &str) -> Option<String> {
    masterlist_repo(game_id).map(|repo| {
        format!(
            "https://raw.githubusercontent.com/loot/{}/v0.21/masterlist.yaml",
            repo
        )
    })
}

/// Raw GitHub URL for the LOOT prelude file.
fn prelude_url() -> &'static str {
    "https://raw.githubusercontent.com/loot/prelude/v0.21/prelude.yaml"
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/// Base cache directory for LOOT data.
fn loot_cache_dir() -> PathBuf {
    crate::config::data_dir().join("loot")
}

/// Path to the cached masterlist for a game.
pub fn masterlist_path(game_id: &str) -> PathBuf {
    loot_cache_dir().join(game_id).join("masterlist.yaml")
}

/// Path to the cached LOOT prelude.
pub fn prelude_path() -> PathBuf {
    loot_cache_dir().join("prelude.yaml")
}

/// Resolve the `local_path` for a game inside a Wine bottle.
///
/// For Skyrim SE this is `<AppData/Local>/Skyrim Special Edition/`.
/// This is where `plugins.txt` and `loadorder.txt` live.
pub fn local_game_path(bottle: &Bottle, game_id: &str) -> Option<PathBuf> {
    let local = bottle.appdata_local();
    let subfolder = match game_id {
        "skyrimse" => "Skyrim Special Edition",
        "skyrim" => "Skyrim",
        "skyrimvr" => "Skyrim VR",
        "fallout4" => "Fallout4",
        "fallout4vr" => "Fallout4VR",
        "falloutnv" => "FalloutNV",
        "fallout3" => "Fallout3",
        "oblivion" => "Oblivion",
        "morrowind" => "Morrowind",
        "starfield" => "Starfield",
        _ => return None,
    };
    Some(local.join(subfolder))
}

// ---------------------------------------------------------------------------
// Masterlist management
// ---------------------------------------------------------------------------

/// Download the LOOT masterlist for a game from GitHub.
///
/// Also downloads the prelude file if it doesn't exist yet.
/// Returns the path to the downloaded masterlist.
pub async fn update_masterlist(game_id: &str) -> Result<PathBuf> {
    let url = masterlist_url(game_id)
        .with_context(|| format!("No LOOT masterlist available for game '{}'", game_id))?;

    let ml_path = masterlist_path(game_id);
    download_file(&url, &ml_path)
        .await
        .with_context(|| format!("Failed to download masterlist for '{}'", game_id))?;

    // Also ensure prelude is cached
    let pl_path = prelude_path();
    if !pl_path.exists() {
        download_file(prelude_url(), &pl_path)
            .await
            .context("Failed to download LOOT prelude")?;
    }

    Ok(ml_path)
}

/// Download a file from a URL to a local path.
async fn download_file(url: &str, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
    }

    let response = reqwest::get(url)
        .await
        .with_context(|| format!("HTTP request failed for {}", url))?;

    if !response.status().is_success() {
        anyhow::bail!("Failed to download {}: HTTP {}", url, response.status());
    }

    let bytes = response
        .bytes()
        .await
        .context("Failed to read response body")?;

    std::fs::write(dest, &bytes)
        .with_context(|| format!("Failed to write file: {}", dest.display()))?;

    log::info!("Downloaded {} -> {}", url, dest.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// Sorting
// ---------------------------------------------------------------------------

/// Sort plugins using LOOT for a given game.
///
/// This function:
/// 1. Creates a libloot Game handle pointing at the Wine bottle paths.
/// 2. Loads the masterlist (downloading if needed).
/// 3. Loads plugin headers from the data directory.
/// 4. Runs LOOT's topological sort.
/// 5. Collects warnings/messages for each plugin.
/// 6. Returns the sorted order + warnings.
pub fn sort_plugins(
    game_id: &str,
    game_path: &Path,
    data_dir: &Path,
    local_path: &Path,
) -> Result<SortResult> {
    let game_type = game_type_for(game_id)
        .with_context(|| format!("Unsupported game for LOOT: '{}'", game_id))?;

    // Create libloot Game handle with explicit local path (Wine/CrossOver)
    let mut game = libloot::Game::with_local_path(game_type, game_path, local_path)
        .map_err(|e| anyhow::anyhow!("Failed to create LOOT game handle: {:?}", e))?;

    // Load masterlist if cached
    let ml_path = masterlist_path(game_id);
    let pl_path = prelude_path();
    {
        let db_arc = game.database();
        let mut db = db_arc
            .write()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        if ml_path.exists() {
            if pl_path.exists() {
                db.load_masterlist_with_prelude(&ml_path, &pl_path)
                    .map_err(|e| anyhow::anyhow!("Failed to load masterlist: {:?}", e))?;
            } else {
                db.load_masterlist(&ml_path)
                    .map_err(|e| anyhow::anyhow!("Failed to load masterlist: {:?}", e))?;
            }
            log::info!("Loaded LOOT masterlist from {}", ml_path.display());
        } else {
            log::warn!(
                "No masterlist cached for '{}' — sorting without metadata",
                game_id
            );
        }
    }

    // Discover and load plugin headers
    let plugin_paths = discover_plugin_paths(data_dir)?;
    let path_refs: Vec<&Path> = plugin_paths.iter().map(|p| p.as_path()).collect();
    game.load_plugin_headers(&path_refs)
        .map_err(|e| anyhow::anyhow!("Failed to load plugin headers: {:?}", e))?;

    // Load current load order state from disk
    game.load_current_load_order_state()
        .map_err(|e| anyhow::anyhow!("Failed to load load order state: {:?}", e))?;

    // Get current order for comparison
    let current_order: Vec<String> = game.load_order().iter().map(|s| s.to_string()).collect();

    // Get plugin names for sorting
    let plugin_names: Vec<&str> = current_order.iter().map(|s| s.as_str()).collect();

    // Sort
    let sorted = game
        .sort_plugins(&plugin_names)
        .map_err(|e| anyhow::anyhow!("LOOT sort failed: {:?}", e))?;

    // Count how many plugins moved
    let plugins_moved = current_order
        .iter()
        .zip(sorted.iter())
        .filter(|(a, b)| a != b)
        .count();

    // Collect warnings
    let warnings = collect_plugin_warnings(&game, &sorted);

    Ok(SortResult {
        sorted_order: sorted,
        plugins_moved,
        warnings,
    })
}

/// Get LOOT messages for a single plugin.
pub fn get_plugin_messages(
    game_id: &str,
    game_path: &Path,
    data_dir: &Path,
    local_path: &Path,
    plugin_name: &str,
) -> Result<Vec<PluginWarning>> {
    let game_type = game_type_for(game_id)
        .with_context(|| format!("Unsupported game for LOOT: '{}'", game_id))?;

    let mut game = libloot::Game::with_local_path(game_type, game_path, local_path)
        .map_err(|e| anyhow::anyhow!("Failed to create LOOT game handle: {:?}", e))?;

    // Load masterlist
    let ml_path = masterlist_path(game_id);
    let pl_path = prelude_path();
    {
        let db_arc = game.database();
        let mut db = db_arc
            .write()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        if ml_path.exists() {
            if pl_path.exists() {
                let _ = db.load_masterlist_with_prelude(&ml_path, &pl_path);
            } else {
                let _ = db.load_masterlist(&ml_path);
            }
        }
    }

    // Load plugin headers
    let plugin_path = data_dir.join(plugin_name);
    if plugin_path.exists() {
        let _ = game.load_plugin_headers(&[plugin_path.as_path()]);
    }

    let warnings = collect_single_plugin_warnings(&game, plugin_name);
    Ok(warnings)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Find all plugin files in the data directory.
fn discover_plugin_paths(data_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    let entries = std::fs::read_dir(data_dir)
        .with_context(|| format!("Failed to read data directory: {}", data_dir.display()))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if let Some(ext) = path.extension() {
            let ext_lower = ext.to_string_lossy().to_lowercase();
            if matches!(ext_lower.as_str(), "esp" | "esm" | "esl") {
                paths.push(path);
            }
        }
    }

    Ok(paths)
}

/// Collect LOOT messages for all plugins.
fn collect_plugin_warnings(game: &libloot::Game, plugin_names: &[String]) -> Vec<PluginWarning> {
    let db_arc = game.database();
    let Ok(db) = db_arc.read() else {
        return Vec::new();
    };

    let mut warnings = Vec::new();

    for name in plugin_names {
        if let Ok(Some(metadata)) =
            db.plugin_metadata(name, MergeMode::WithUserMetadata, EvalMode::DoNotEvaluate)
        {
            for msg in metadata.messages() {
                let level = match msg.message_type() {
                    libloot::metadata::MessageType::Say => "info",
                    libloot::metadata::MessageType::Warn => "warn",
                    libloot::metadata::MessageType::Error => "error",
                };

                // Get the English content (first content entry or default)
                let text = msg
                    .content()
                    .first()
                    .map(|c| c.text().to_string())
                    .unwrap_or_default();

                if !text.is_empty() {
                    warnings.push(PluginWarning {
                        plugin_name: name.clone(),
                        level: level.to_string(),
                        message: text,
                    });
                }
            }
        }
    }

    warnings
}

/// Collect LOOT messages for a single plugin.
fn collect_single_plugin_warnings(game: &libloot::Game, plugin_name: &str) -> Vec<PluginWarning> {
    let db_arc = game.database();
    let Ok(db) = db_arc.read() else {
        return Vec::new();
    };

    let mut warnings = Vec::new();

    if let Ok(Some(metadata)) = db.plugin_metadata(
        plugin_name,
        MergeMode::WithUserMetadata,
        EvalMode::DoNotEvaluate,
    ) {
        for msg in metadata.messages() {
            let level = match msg.message_type() {
                libloot::metadata::MessageType::Say => "info",
                libloot::metadata::MessageType::Warn => "warn",
                libloot::metadata::MessageType::Error => "error",
            };

            let text = msg
                .content()
                .first()
                .map(|c| c.text().to_string())
                .unwrap_or_default();

            if !text.is_empty() {
                warnings.push(PluginWarning {
                    plugin_name: plugin_name.to_string(),
                    level: level.to_string(),
                    message: text,
                });
            }
        }
    }

    // Also check if the plugin has dirty info
    if let Ok(Some(metadata)) = db.plugin_metadata(
        plugin_name,
        MergeMode::WithUserMetadata,
        EvalMode::DoNotEvaluate,
    ) {
        for dirty in metadata.dirty_info() {
            let text = format!(
                "Contains {} ITM records. Clean with xEdit.",
                dirty.itm_count()
            );
            warnings.push(PluginWarning {
                plugin_name: plugin_name.to_string(),
                level: "warn".to_string(),
                message: text,
            });
        }
    }

    warnings
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_type_mapping_covers_known_games() {
        assert!(matches!(
            game_type_for("skyrimse"),
            Some(GameType::SkyrimSE)
        ));
        assert!(matches!(
            game_type_for("fallout4"),
            Some(GameType::Fallout4)
        ));
        assert!(matches!(
            game_type_for("oblivion"),
            Some(GameType::Oblivion)
        ));
        assert!(game_type_for("unknown_game").is_none());
    }

    #[test]
    fn masterlist_urls_are_valid() {
        let url = masterlist_url("skyrimse").unwrap();
        assert!(url.contains("loot/skyrimse"));
        assert!(url.ends_with("masterlist.yaml"));

        assert!(masterlist_url("unknown").is_none());
    }

    #[test]
    fn masterlist_cache_paths_are_reasonable() {
        let path = masterlist_path("skyrimse");
        assert!(path.to_string_lossy().contains("loot"));
        assert!(path.to_string_lossy().contains("skyrimse"));
        assert!(path.to_string_lossy().ends_with("masterlist.yaml"));
    }

    #[test]
    fn local_game_path_returns_correct_paths() {
        let bottle = crate::bottles::Bottle {
            name: "Test".into(),
            path: PathBuf::from("/fake/bottle"),
            source: "Test".into(),
        };
        let path = local_game_path(&bottle, "skyrimse");
        assert!(path.is_some());
        let path = path.unwrap();
        assert!(path.to_string_lossy().contains("Skyrim Special Edition"));

        assert!(local_game_path(&bottle, "unknown").is_none());
    }

    #[test]
    fn discover_plugin_paths_finds_plugins() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path();

        std::fs::write(data_dir.join("Skyrim.esm"), b"fake").unwrap();
        std::fs::write(data_dir.join("MyMod.esp"), b"fake").unwrap();
        std::fs::write(data_dir.join("Light.esl"), b"fake").unwrap();
        std::fs::write(data_dir.join("readme.txt"), b"not a plugin").unwrap();

        let paths = discover_plugin_paths(data_dir).unwrap();
        assert_eq!(paths.len(), 3);
    }
}
