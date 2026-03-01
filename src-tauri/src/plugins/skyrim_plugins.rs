//! Plugin load order management for Bethesda games.
//!
//! Handles reading, writing, and synchronising `plugins.txt` and
//! `loadorder.txt` files used by Bethesda games (Skyrim SE, Fallout 4, etc.)
//! to determine which plugins are active and in what order they are loaded.
//!
//! ## File formats
//!
//! **`plugins.txt`**:
//! - Lines starting with `#` are comments.
//! - Lines starting with `*` denote an enabled plugin (the `*` is stripped).
//! - All other non-empty lines are disabled plugins.
//!
//! **`loadorder.txt`**:
//! - Plain list of plugin filenames, one per line, in load order.
//! - Comments (`#`) and blank lines are ignored.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Skyrim SE implicit plugins — always loaded by the engine in this order.
pub const SKYRIM_SE_IMPLICIT_PLUGINS: &[&str] = &[
    "Skyrim.esm",
    "Update.esm",
    "Dawnguard.esm",
    "HearthFires.esm",
    "Dragonborn.esm",
];

/// Fallout 4 implicit plugins — always loaded by the engine in this order.
pub const FALLOUT4_IMPLICIT_PLUGINS: &[&str] = &[
    "Fallout4.esm",
    "DLCRobot.esm",
    "DLCworkshop01.esm",
    "DLCworkshop02.esm",
    "DLCworkshop03.esm",
    "DLCCoast.esm",
    "DLCNukaWorld.esm",
];

/// Legacy alias for backwards compatibility.
pub const IMPLICIT_PLUGINS: &[&str] = SKYRIM_SE_IMPLICIT_PLUGINS;

/// Get the implicit plugins list for a given game ID.
pub fn implicit_plugins_for_game(game_id: &str) -> &'static [&'static str] {
    match game_id {
        "skyrimse" => SKYRIM_SE_IMPLICIT_PLUGINS,
        "fallout4" => FALLOUT4_IMPLICIT_PLUGINS,
        _ => &[],
    }
}

/// Game IDs that support Bethesda-style plugin load order.
pub fn supports_plugin_order(game_id: &str) -> bool {
    matches!(game_id, "skyrimse" | "fallout4")
}

/// Returns the implicit (always-loaded) plugins for a given game.
pub fn get_implicit_plugins(game_id: &str) -> &'static [&'static str] {
    match game_id {
        "skyrimse" => SKYRIM_SE_IMPLICIT_PLUGINS,
        "fallout4" => FALLOUT4_IMPLICIT_PLUGINS,
        _ => &[],
    }
}

// ---------------------------------------------------------------------------
// PluginEntry
// ---------------------------------------------------------------------------

/// A single entry in the plugin load order.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginEntry {
    /// Filename of the plugin (e.g. `"MyMod.esp"`).
    pub filename: String,
    /// Whether the plugin is enabled (active).
    pub enabled: bool,
}

impl PluginEntry {
    /// Returns `true` if this plugin has the `.esl` extension (ESL light plugin).
    pub fn is_esl(&self) -> bool {
        self.filename
            .rsplit('.')
            .next()
            .map(|ext| ext.eq_ignore_ascii_case("esl"))
            .unwrap_or(false)
    }

    /// Returns `true` if this plugin has the `.esm` extension (master file).
    pub fn is_esm(&self) -> bool {
        self.filename
            .rsplit('.')
            .next()
            .map(|ext| ext.eq_ignore_ascii_case("esm"))
            .unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------
// plugins.txt
// ---------------------------------------------------------------------------

/// Read a `plugins.txt` file and return the plugin entries.
///
/// Lines starting with `#` are comments and are skipped. Lines starting with
/// `*` indicate an enabled plugin. All other non-empty lines are treated as
/// disabled plugins.
pub fn read_plugins_txt(path: &Path) -> Result<Vec<PluginEntry>> {
    let file = fs::File::open(path)
        .with_context(|| format!("Failed to open plugins.txt: {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut entries = Vec::new();

    for line in reader.lines() {
        let line = line.with_context(|| "Failed to read line from plugins.txt")?;
        let trimmed = line.trim();

        // Skip empty lines and comments.
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if let Some(name) = trimmed.strip_prefix('*') {
            let name = name.trim();
            if !name.is_empty() {
                entries.push(PluginEntry {
                    filename: name.to_string(),
                    enabled: true,
                });
            }
        } else {
            entries.push(PluginEntry {
                filename: trimmed.to_string(),
                enabled: false,
            });
        }
    }

    Ok(entries)
}

/// Write plugin entries to a `plugins.txt` file.
///
/// Enabled plugins are prefixed with `*`. A header comment is written at
/// the top of the file.
pub fn write_plugins_txt(path: &Path, entries: &[PluginEntry]) -> Result<()> {
    // Ensure parent directory exists.
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
    }

    let mut file = fs::File::create(path)
        .with_context(|| format!("Failed to create plugins.txt: {}", path.display()))?;

    writeln!(file, "# This file is used to determine plugin load order.")?;
    writeln!(file, "# Managed by Corkscrew.")?;

    for entry in entries {
        if entry.enabled {
            writeln!(file, "*{}", entry.filename)?;
        } else {
            writeln!(file, "{}", entry.filename)?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// loadorder.txt
// ---------------------------------------------------------------------------

/// Read a `loadorder.txt` file and return the plugin filenames in order.
///
/// Comments (`#`) and blank lines are ignored.
pub fn read_loadorder_txt(path: &Path) -> Result<Vec<String>> {
    let file = fs::File::open(path)
        .with_context(|| format!("Failed to open loadorder.txt: {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut plugins = Vec::new();

    for line in reader.lines() {
        let line = line.with_context(|| "Failed to read line from loadorder.txt")?;
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        plugins.push(trimmed.to_string());
    }

    Ok(plugins)
}

/// Write plugin filenames to a `loadorder.txt` file in the given order.
pub fn write_loadorder_txt(path: &Path, plugins: &[String]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
    }

    let mut file = fs::File::create(path)
        .with_context(|| format!("Failed to create loadorder.txt: {}", path.display()))?;

    for plugin in plugins {
        writeln!(file, "{}", plugin)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Discover all plugin files (`.esp`, `.esm`, `.esl`) in the given data
/// directory.
///
/// Returns a sorted list of filenames found on disk.
pub fn discover_plugins(data_dir: &Path) -> Result<Vec<String>> {
    let mut plugins = Vec::new();

    let entries = fs::read_dir(data_dir)
        .with_context(|| format!("Failed to read data directory: {}", data_dir.display()))?;

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if is_plugin_file(&name) {
            plugins.push(name);
        }
    }

    plugins.sort_by_key(|a| a.to_lowercase());
    Ok(plugins)
}

/// Check whether a filename has a recognised plugin extension.
fn is_plugin_file(filename: &str) -> bool {
    let ext = filename.rsplit('.').next().unwrap_or("");
    matches!(ext.to_lowercase().as_str(), "esp" | "esm" | "esl")
}

// ---------------------------------------------------------------------------
// Synchronisation
// ---------------------------------------------------------------------------

/// Synchronise the on-disk plugin state with `plugins.txt` and
/// `loadorder.txt`.
///
/// This function:
/// 1. Discovers all plugins present in `data_dir`.
/// 2. Reads the existing `plugins.txt` (if present) to preserve
///    enabled/disabled state.
/// 3. Adds any newly discovered plugins as disabled.
/// 4. Removes entries for plugins no longer on disk.
/// 5. Ensures implicit (base game) plugins are present and enabled.
/// 6. Writes the updated `plugins.txt` and `loadorder.txt`.
pub fn sync_plugins(
    data_dir: &Path,
    plugins_file: &Path,
    loadorder_file: &Path,
    game_implicit_plugins: &[&str],
) -> Result<()> {
    let on_disk = discover_plugins(data_dir)?;

    // Build a set of filenames on disk for quick lookup (case-insensitive).
    let on_disk_lower: Vec<String> = on_disk.iter().map(|s| s.to_lowercase()).collect();

    // Read existing state if available.
    let existing = if plugins_file.exists() {
        read_plugins_txt(plugins_file).unwrap_or_default()
    } else {
        Vec::new()
    };

    // Save the existing order before consuming into map.
    let existing_order: Vec<String> = existing.iter().map(|e| e.filename.clone()).collect();

    // Build a map of existing entries (lowercase key -> PluginEntry).
    let mut existing_map: std::collections::HashMap<String, PluginEntry> = existing
        .into_iter()
        .map(|e| (e.filename.to_lowercase(), e))
        .collect();

    // Ensure implicit plugins are present and enabled.
    for &implicit in game_implicit_plugins {
        let key = implicit.to_lowercase();
        if on_disk_lower.contains(&key) {
            existing_map
                .entry(key)
                .and_modify(|e| e.enabled = true)
                .or_insert(PluginEntry {
                    filename: implicit.to_string(),
                    enabled: true,
                });
        }
    }

    // Build the final ordered list.
    let mut result: Vec<PluginEntry> = Vec::new();
    let mut added: std::collections::HashSet<String> = std::collections::HashSet::new();

    // First add implicit plugins in their canonical order.
    for &implicit in game_implicit_plugins {
        let key = implicit.to_lowercase();
        if on_disk_lower.contains(&key) {
            // Use the on-disk filename casing.
            let real_name = on_disk
                .iter()
                .find(|n| n.to_lowercase() == key)
                .cloned()
                .unwrap_or_else(|| implicit.to_string());
            result.push(PluginEntry {
                filename: real_name,
                enabled: true,
            });
            added.insert(key.clone());
            existing_map.remove(&key);
        }
    }

    // Then add remaining plugins from the EXISTING order (preserving the
    // order from plugins.txt — e.g. collection-defined load order).
    // Only include plugins that are still on disk.
    for existing_name in &existing_order {
        let key = existing_name.to_lowercase();

        // Skip implicit plugins already added.
        if added.contains(&key) {
            continue;
        }

        // Skip plugins no longer on disk.
        if !on_disk_lower.contains(&key) {
            existing_map.remove(&key);
            continue;
        }

        let base_enabled = if let Some(existing_entry) = existing_map.remove(&key) {
            existing_entry.enabled
        } else {
            true
        };

        // ESM and ESL files are always loaded by the engine — force them enabled.
        // All on-disk plugins should be enabled: if a file is deployed, it should
        // be active.  The only "disabled" entries should be for plugins listed in
        // plugins.txt that no longer have a file on disk (and those are already
        // skipped above at line 345).
        let is_master = key.ends_with(".esm") || key.ends_with(".esl");
        result.push(PluginEntry {
            filename: existing_name.clone(),
            enabled: if is_master || on_disk_lower.contains(&key) {
                true
            } else {
                base_enabled
            },
        });
        added.insert(key);
    }

    // Finally, append any NEW on-disk plugins not in the existing file.
    for plugin_name in &on_disk {
        let key = plugin_name.to_lowercase();

        if added.contains(&key) {
            continue;
        }

        // ESM and ESL files are always loaded by the engine — force them enabled.
        let is_master = key.ends_with(".esm") || key.ends_with(".esl");
        result.push(PluginEntry {
            filename: plugin_name.clone(),
            enabled: if is_master { true } else { true }, // new plugins default enabled
        });
        added.insert(key);
    }

    // Write updated files.
    write_plugins_txt(plugins_file, &result)?;

    let load_order: Vec<String> = result.iter().map(|e| e.filename.clone()).collect();
    write_loadorder_txt(loadorder_file, &load_order)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Reorder / Toggle / Move
// ---------------------------------------------------------------------------

/// Apply a new load order by rewriting both `plugins.txt` and `loadorder.txt`.
///
/// `ordered_plugins` contains `PluginEntry` items in the desired order.
/// This replaces the current files on disk.
pub fn apply_load_order(
    plugins_file: &Path,
    loadorder_file: &Path,
    ordered_plugins: &[PluginEntry],
) -> Result<()> {
    write_plugins_txt(plugins_file, ordered_plugins)?;
    let load_order: Vec<String> = ordered_plugins.iter().map(|e| e.filename.clone()).collect();
    write_loadorder_txt(loadorder_file, &load_order)?;
    Ok(())
}

/// Reorder plugins from a list of filenames, preserving enabled state.
///
/// Any plugins not mentioned in `ordered_names` are appended at the end
/// in their original relative order.
pub fn reorder_plugins(
    plugins_file: &Path,
    loadorder_file: &Path,
    ordered_names: &[String],
) -> Result<Vec<PluginEntry>> {
    let existing = if plugins_file.exists() {
        read_plugins_txt(plugins_file)?
    } else {
        Vec::new()
    };

    // Build a map of existing state
    let state_map: std::collections::HashMap<String, bool> = existing
        .iter()
        .map(|e| (e.filename.to_lowercase(), e.enabled))
        .collect();

    let mut result: Vec<PluginEntry> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Add plugins in the requested order
    for name in ordered_names {
        let key = name.to_lowercase();
        if seen.contains(&key) {
            continue;
        }
        seen.insert(key.clone());

        let enabled = state_map.get(&key).copied().unwrap_or(false);
        result.push(PluginEntry {
            filename: name.clone(),
            enabled,
        });
    }

    // Append any remaining plugins not in the ordered list
    for entry in &existing {
        let key = entry.filename.to_lowercase();
        if !seen.contains(&key) {
            seen.insert(key);
            result.push(entry.clone());
        }
    }

    apply_load_order(plugins_file, loadorder_file, &result)?;
    Ok(result)
}

/// Toggle the enabled state of a single plugin.
///
/// Returns the updated full plugin list.
pub fn toggle_plugin(
    plugins_file: &Path,
    loadorder_file: &Path,
    plugin_name: &str,
    enabled: bool,
) -> Result<Vec<PluginEntry>> {
    let mut entries = if plugins_file.exists() {
        read_plugins_txt(plugins_file)?
    } else {
        Vec::new()
    };

    let target_lower = plugin_name.to_lowercase();

    // ESM and ESL files are always loaded by the engine — refuse to disable them.
    let is_master = target_lower.ends_with(".esm") || target_lower.ends_with(".esl");
    let effective_enabled = if is_master { true } else { enabled };

    let mut found = false;

    for entry in &mut entries {
        if entry.filename.to_lowercase() == target_lower {
            entry.enabled = effective_enabled;
            found = true;
            break;
        }
    }

    if !found {
        entries.push(PluginEntry {
            filename: plugin_name.to_string(),
            enabled: effective_enabled,
        });
    }

    apply_load_order(plugins_file, loadorder_file, &entries)?;
    Ok(entries)
}

/// Move a plugin to a new position in the load order.
///
/// Returns the updated full plugin list.
pub fn move_plugin(
    plugins_file: &Path,
    loadorder_file: &Path,
    plugin_name: &str,
    new_index: usize,
) -> Result<Vec<PluginEntry>> {
    let mut entries = if plugins_file.exists() {
        read_plugins_txt(plugins_file)?
    } else {
        Vec::new()
    };

    let target_lower = plugin_name.to_lowercase();

    // Find and remove the plugin from its current position
    let pos = entries
        .iter()
        .position(|e| e.filename.to_lowercase() == target_lower);

    if let Some(current_pos) = pos {
        let entry = entries.remove(current_pos);
        let insert_at = new_index.min(entries.len());
        entries.insert(insert_at, entry);
    }

    apply_load_order(plugins_file, loadorder_file, &entries)?;
    Ok(entries)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn plugin_entry_is_esl() {
        let entry = PluginEntry {
            filename: "MyMod.esl".into(),
            enabled: true,
        };
        assert!(entry.is_esl());
        assert!(!entry.is_esm());
    }

    #[test]
    fn plugin_entry_is_esm() {
        let entry = PluginEntry {
            filename: "Skyrim.esm".into(),
            enabled: true,
        };
        assert!(entry.is_esm());
        assert!(!entry.is_esl());
    }

    #[test]
    fn plugin_entry_esp_is_neither() {
        let entry = PluginEntry {
            filename: "MyMod.esp".into(),
            enabled: false,
        };
        assert!(!entry.is_esm());
        assert!(!entry.is_esl());
    }

    #[test]
    fn read_write_plugins_txt_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("plugins.txt");

        let entries = vec![
            PluginEntry {
                filename: "Skyrim.esm".into(),
                enabled: true,
            },
            PluginEntry {
                filename: "MyMod.esp".into(),
                enabled: false,
            },
            PluginEntry {
                filename: "CoolMod.esp".into(),
                enabled: true,
            },
        ];

        write_plugins_txt(&path, &entries).unwrap();
        let read_back = read_plugins_txt(&path).unwrap();

        assert_eq!(read_back.len(), 3);
        assert_eq!(read_back[0].filename, "Skyrim.esm");
        assert!(read_back[0].enabled);
        assert_eq!(read_back[1].filename, "MyMod.esp");
        assert!(!read_back[1].enabled);
        assert_eq!(read_back[2].filename, "CoolMod.esp");
        assert!(read_back[2].enabled);
    }

    #[test]
    fn read_plugins_txt_skips_comments() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("plugins.txt");
        fs::write(
            &path,
            "# Comment line\n*Skyrim.esm\n# Another comment\nMyMod.esp\n\n",
        )
        .unwrap();

        let entries = read_plugins_txt(&path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].filename, "Skyrim.esm");
        assert!(entries[0].enabled);
        assert_eq!(entries[1].filename, "MyMod.esp");
        assert!(!entries[1].enabled);
    }

    #[test]
    fn read_write_loadorder_txt_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("loadorder.txt");

        let plugins = vec![
            "Skyrim.esm".to_string(),
            "Update.esm".to_string(),
            "MyMod.esp".to_string(),
        ];

        write_loadorder_txt(&path, &plugins).unwrap();
        let read_back = read_loadorder_txt(&path).unwrap();

        assert_eq!(read_back, plugins);
    }

    #[test]
    fn discover_plugins_finds_plugin_files() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("Data");
        fs::create_dir_all(&data_dir).unwrap();

        // Create some plugin files and a non-plugin file.
        fs::write(data_dir.join("Skyrim.esm"), b"fake").unwrap();
        fs::write(data_dir.join("MyMod.esp"), b"fake").unwrap();
        fs::write(data_dir.join("Light.esl"), b"fake").unwrap();
        fs::write(data_dir.join("readme.txt"), b"not a plugin").unwrap();
        fs::write(data_dir.join("texture.dds"), b"not a plugin").unwrap();

        let plugins = discover_plugins(&data_dir).unwrap();
        assert_eq!(plugins.len(), 3);

        let names: Vec<&str> = plugins.iter().map(|s| s.as_str()).collect();
        assert!(names.contains(&"Skyrim.esm"));
        assert!(names.contains(&"MyMod.esp"));
        assert!(names.contains(&"Light.esl"));
    }

    #[test]
    fn is_plugin_file_detects_extensions() {
        assert!(is_plugin_file("Skyrim.esm"));
        assert!(is_plugin_file("MyMod.ESP"));
        assert!(is_plugin_file("Light.esl"));
        assert!(!is_plugin_file("readme.txt"));
        assert!(!is_plugin_file("texture.dds"));
        assert!(!is_plugin_file("no_extension"));
    }

    #[test]
    fn sync_plugins_creates_files() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("Data");
        fs::create_dir_all(&data_dir).unwrap();

        // Create base game plugins and a user mod.
        fs::write(data_dir.join("Skyrim.esm"), b"fake").unwrap();
        fs::write(data_dir.join("Update.esm"), b"fake").unwrap();
        fs::write(data_dir.join("Dawnguard.esm"), b"fake").unwrap();
        fs::write(data_dir.join("HearthFires.esm"), b"fake").unwrap();
        fs::write(data_dir.join("Dragonborn.esm"), b"fake").unwrap();
        fs::write(data_dir.join("UserMod.esp"), b"fake").unwrap();

        let plugins_file = tmp.path().join("plugins.txt");
        let loadorder_file = tmp.path().join("loadorder.txt");

        sync_plugins(
            &data_dir,
            &plugins_file,
            &loadorder_file,
            SKYRIM_SE_IMPLICIT_PLUGINS,
        )
        .unwrap();

        // Verify plugins.txt was created.
        let entries = read_plugins_txt(&plugins_file).unwrap();

        // Implicit plugins should be first and enabled.
        assert_eq!(entries[0].filename, "Skyrim.esm");
        assert!(entries[0].enabled);
        assert_eq!(entries[4].filename, "Dragonborn.esm");
        assert!(entries[4].enabled);

        // User mod should be present and enabled by default (matches Vortex/MO2 behavior).
        let user_mod = entries
            .iter()
            .find(|e| e.filename == "UserMod.esp")
            .unwrap();
        assert!(user_mod.enabled);

        // Verify loadorder.txt was created.
        let load_order = read_loadorder_txt(&loadorder_file).unwrap();
        assert_eq!(load_order.len(), entries.len());
    }

    #[test]
    fn sync_plugins_preserves_enabled_state() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("Data");
        fs::create_dir_all(&data_dir).unwrap();

        fs::write(data_dir.join("Skyrim.esm"), b"fake").unwrap();
        fs::write(data_dir.join("UserMod.esp"), b"fake").unwrap();

        let plugins_file = tmp.path().join("plugins.txt");
        let loadorder_file = tmp.path().join("loadorder.txt");

        // Pre-populate plugins.txt with UserMod enabled.
        write_plugins_txt(
            &plugins_file,
            &[
                PluginEntry {
                    filename: "Skyrim.esm".into(),
                    enabled: true,
                },
                PluginEntry {
                    filename: "UserMod.esp".into(),
                    enabled: true,
                },
            ],
        )
        .unwrap();

        sync_plugins(
            &data_dir,
            &plugins_file,
            &loadorder_file,
            SKYRIM_SE_IMPLICIT_PLUGINS,
        )
        .unwrap();

        let entries = read_plugins_txt(&plugins_file).unwrap();
        let user_mod = entries
            .iter()
            .find(|e| e.filename == "UserMod.esp")
            .unwrap();
        assert!(
            user_mod.enabled,
            "UserMod.esp should remain enabled after sync"
        );
    }

    #[test]
    fn sync_plugins_removes_missing_plugins() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("Data");
        fs::create_dir_all(&data_dir).unwrap();

        fs::write(data_dir.join("Skyrim.esm"), b"fake").unwrap();

        let plugins_file = tmp.path().join("plugins.txt");
        let loadorder_file = tmp.path().join("loadorder.txt");

        // Pre-populate with a plugin that no longer exists on disk.
        write_plugins_txt(
            &plugins_file,
            &[
                PluginEntry {
                    filename: "Skyrim.esm".into(),
                    enabled: true,
                },
                PluginEntry {
                    filename: "DeletedMod.esp".into(),
                    enabled: true,
                },
            ],
        )
        .unwrap();

        sync_plugins(
            &data_dir,
            &plugins_file,
            &loadorder_file,
            SKYRIM_SE_IMPLICIT_PLUGINS,
        )
        .unwrap();

        let entries = read_plugins_txt(&plugins_file).unwrap();
        assert!(
            !entries.iter().any(|e| e.filename == "DeletedMod.esp"),
            "Deleted plugin should be removed from plugins.txt"
        );
    }

    #[test]
    fn toggle_plugin_enables_and_disables() {
        let tmp = tempfile::tempdir().unwrap();
        let pf = tmp.path().join("plugins.txt");
        let lo = tmp.path().join("loadorder.txt");

        write_plugins_txt(
            &pf,
            &[
                PluginEntry {
                    filename: "Skyrim.esm".into(),
                    enabled: true,
                },
                PluginEntry {
                    filename: "MyMod.esp".into(),
                    enabled: false,
                },
            ],
        )
        .unwrap();

        // Enable MyMod.esp
        let result = toggle_plugin(&pf, &lo, "MyMod.esp", true).unwrap();
        let my_mod = result.iter().find(|e| e.filename == "MyMod.esp").unwrap();
        assert!(my_mod.enabled);

        // Disable MyMod.esp
        let result = toggle_plugin(&pf, &lo, "MyMod.esp", false).unwrap();
        let my_mod = result.iter().find(|e| e.filename == "MyMod.esp").unwrap();
        assert!(!my_mod.enabled);
    }

    #[test]
    fn move_plugin_changes_position() {
        let tmp = tempfile::tempdir().unwrap();
        let pf = tmp.path().join("plugins.txt");
        let lo = tmp.path().join("loadorder.txt");

        write_plugins_txt(
            &pf,
            &[
                PluginEntry {
                    filename: "A.esm".into(),
                    enabled: true,
                },
                PluginEntry {
                    filename: "B.esp".into(),
                    enabled: true,
                },
                PluginEntry {
                    filename: "C.esp".into(),
                    enabled: true,
                },
            ],
        )
        .unwrap();

        // Move C to position 1 (between A and B)
        let result = move_plugin(&pf, &lo, "C.esp", 1).unwrap();
        assert_eq!(result[0].filename, "A.esm");
        assert_eq!(result[1].filename, "C.esp");
        assert_eq!(result[2].filename, "B.esp");
    }

    #[test]
    fn reorder_plugins_preserves_enabled_state() {
        let tmp = tempfile::tempdir().unwrap();
        let pf = tmp.path().join("plugins.txt");
        let lo = tmp.path().join("loadorder.txt");

        write_plugins_txt(
            &pf,
            &[
                PluginEntry {
                    filename: "A.esm".into(),
                    enabled: true,
                },
                PluginEntry {
                    filename: "B.esp".into(),
                    enabled: false,
                },
                PluginEntry {
                    filename: "C.esp".into(),
                    enabled: true,
                },
            ],
        )
        .unwrap();

        let new_order = vec![
            "C.esp".to_string(),
            "A.esm".to_string(),
            "B.esp".to_string(),
        ];
        let result = reorder_plugins(&pf, &lo, &new_order).unwrap();

        assert_eq!(result[0].filename, "C.esp");
        assert!(result[0].enabled);
        assert_eq!(result[1].filename, "A.esm");
        assert!(result[1].enabled);
        assert_eq!(result[2].filename, "B.esp");
        assert!(!result[2].enabled); // preserves disabled state
    }

    #[test]
    fn sync_plugins_forces_esm_enabled() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("Data");
        fs::create_dir_all(&data_dir).unwrap();

        fs::write(data_dir.join("Skyrim.esm"), b"fake").unwrap();
        fs::write(data_dir.join("ModMaster.esm"), b"fake").unwrap();
        fs::write(data_dir.join("UserMod.esp"), b"fake").unwrap();

        let pf = tmp.path().join("plugins.txt");
        let lo = tmp.path().join("loadorder.txt");

        // Pre-populate with the mod ESM disabled (as if from an old plugins.txt).
        write_plugins_txt(
            &pf,
            &[
                PluginEntry {
                    filename: "Skyrim.esm".into(),
                    enabled: true,
                },
                PluginEntry {
                    filename: "ModMaster.esm".into(),
                    enabled: false,
                },
                PluginEntry {
                    filename: "UserMod.esp".into(),
                    enabled: false,
                },
            ],
        )
        .unwrap();

        sync_plugins(&data_dir, &pf, &lo, SKYRIM_SE_IMPLICIT_PLUGINS).unwrap();
        let entries = read_plugins_txt(&pf).unwrap();

        let master = entries
            .iter()
            .find(|e| e.filename == "ModMaster.esm")
            .unwrap();
        assert!(master.enabled, "ESM files must be forced enabled by sync");

        // On-disk ESPs should be enabled — if the file is deployed, it should
        // be active.  (Changed from the old behaviour that preserved disabled
        // state, which left 1500+ collection plugins inactive.)
        let esp = entries
            .iter()
            .find(|e| e.filename == "UserMod.esp")
            .unwrap();
        assert!(esp.enabled, "On-disk ESP should be enabled by sync");
    }

    #[test]
    fn sync_plugins_forces_esl_enabled() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("Data");
        fs::create_dir_all(&data_dir).unwrap();

        fs::write(data_dir.join("Skyrim.esm"), b"fake").unwrap();
        fs::write(data_dir.join("LightPatch.esl"), b"fake").unwrap();

        let pf = tmp.path().join("plugins.txt");
        let lo = tmp.path().join("loadorder.txt");

        // Pre-populate with the ESL disabled.
        write_plugins_txt(
            &pf,
            &[
                PluginEntry {
                    filename: "Skyrim.esm".into(),
                    enabled: true,
                },
                PluginEntry {
                    filename: "LightPatch.esl".into(),
                    enabled: false,
                },
            ],
        )
        .unwrap();

        sync_plugins(&data_dir, &pf, &lo, SKYRIM_SE_IMPLICIT_PLUGINS).unwrap();
        let entries = read_plugins_txt(&pf).unwrap();

        let esl = entries
            .iter()
            .find(|e| e.filename == "LightPatch.esl")
            .unwrap();
        assert!(esl.enabled, "ESL files must be forced enabled by sync");
    }

    #[test]
    fn toggle_plugin_refuses_to_disable_esm() {
        let tmp = tempfile::tempdir().unwrap();
        let pf = tmp.path().join("plugins.txt");
        let lo = tmp.path().join("loadorder.txt");

        write_plugins_txt(
            &pf,
            &[PluginEntry {
                filename: "ModMaster.esm".into(),
                enabled: true,
            }],
        )
        .unwrap();

        // Try to disable an ESM — should remain enabled.
        let result = toggle_plugin(&pf, &lo, "ModMaster.esm", false).unwrap();
        let entry = result
            .iter()
            .find(|e| e.filename == "ModMaster.esm")
            .unwrap();
        assert!(entry.enabled, "ESM must stay enabled even when toggled off");
    }

    #[test]
    fn toggle_plugin_refuses_to_disable_esl() {
        let tmp = tempfile::tempdir().unwrap();
        let pf = tmp.path().join("plugins.txt");
        let lo = tmp.path().join("loadorder.txt");

        write_plugins_txt(
            &pf,
            &[PluginEntry {
                filename: "Light.esl".into(),
                enabled: true,
            }],
        )
        .unwrap();

        let result = toggle_plugin(&pf, &lo, "Light.esl", false).unwrap();
        let entry = result.iter().find(|e| e.filename == "Light.esl").unwrap();
        assert!(entry.enabled, "ESL must stay enabled even when toggled off");
    }

    #[test]
    fn toggle_plugin_allows_disabling_esp() {
        let tmp = tempfile::tempdir().unwrap();
        let pf = tmp.path().join("plugins.txt");
        let lo = tmp.path().join("loadorder.txt");

        write_plugins_txt(
            &pf,
            &[PluginEntry {
                filename: "UserMod.esp".into(),
                enabled: true,
            }],
        )
        .unwrap();

        let result = toggle_plugin(&pf, &lo, "UserMod.esp", false).unwrap();
        let entry = result.iter().find(|e| e.filename == "UserMod.esp").unwrap();
        assert!(!entry.enabled, "ESP files can be disabled normally");
    }
}
