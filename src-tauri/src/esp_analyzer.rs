// SPDX-License-Identifier: GPL-3.0-or-later
//! ESP/ESM/ESL record-level conflict detection.
//!
//! Uses the `esplugin` crate (by Ortham, same author as libloot) to parse
//! Bethesda plugin files and detect which plugins override the same records.
//! This enables the UI to show record-level conflicts beyond simple file-level
//! overlap that the deployer already handles.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use esplugin::{GameId, ParseOptions, Plugin};
use serde::Serialize;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Summary of a single parsed plugin's record counts and metadata.
#[derive(Debug, Clone, Serialize)]
pub struct PluginRecordSummary {
    /// Plugin filename (e.g. "Unofficial Skyrim Special Edition Patch.esp").
    pub plugin_name: String,
    /// Master files this plugin depends on.
    pub masters: Vec<String>,
    /// Total record + group count reported in the plugin header.
    pub record_and_group_count: Option<u32>,
    /// Number of records that override records defined in master plugins.
    pub override_record_count: usize,
    /// Whether the plugin has the ESM flag set.
    pub is_master: bool,
    /// Whether the plugin has the ESL (light) flag set.
    pub is_light: bool,
    /// Header version (e.g. 0.94 for Skyrim LE, 1.70 for Skyrim SE).
    pub header_version: Option<f32>,
}

/// A detected record-level conflict: two or more plugins that modify overlapping
/// records (same FormIDs).
#[derive(Debug, Clone, Serialize)]
pub struct RecordConflict {
    /// The pair of plugins that overlap.
    pub plugin_a: String,
    pub plugin_b: String,
    /// Number of records (FormIDs) shared between the two plugins.
    pub overlap_count: usize,
}

// ---------------------------------------------------------------------------
// GameId mapping
// ---------------------------------------------------------------------------

/// Map a Corkscrew game_id string to an esplugin `GameId`.
pub fn esplugin_game_id(game_id: &str) -> Option<GameId> {
    match game_id {
        "skyrimse" | "skyrimspecialedition" => Some(GameId::SkyrimSE),
        "skyrim" | "skyrimle" => Some(GameId::Skyrim),
        "fallout4" => Some(GameId::Fallout4),
        "falloutnv" | "fallout_nv" => Some(GameId::FalloutNV),
        "fallout3" => Some(GameId::Fallout3),
        "oblivion" => Some(GameId::Oblivion),
        "morrowind" => Some(GameId::Morrowind),
        "starfield" => Some(GameId::Starfield),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Core analysis functions
// ---------------------------------------------------------------------------

/// Parse a single plugin and return a [`PluginRecordSummary`].
///
/// `data_dir` is the game's Data folder (where plugin files live).
/// `plugin_name` is the filename (e.g. `"Skyrim.esm"`).
pub fn analyze_plugin(
    data_dir: &Path,
    plugin_name: &str,
    game_id: GameId,
) -> Result<PluginRecordSummary> {
    let plugin_path = data_dir.join(plugin_name);
    if !plugin_path.is_file() {
        bail!(
            "Plugin file not found: {}",
            plugin_path.display()
        );
    }

    let mut plugin = Plugin::new(game_id, &plugin_path);
    plugin
        .parse_file(ParseOptions::whole_plugin())
        .with_context(|| format!("Failed to parse plugin '{}'", plugin_name))?;

    let masters = plugin
        .masters()
        .with_context(|| format!("Failed to read masters for '{}'", plugin_name))?;

    let override_count = plugin
        .count_override_records()
        .with_context(|| format!("Failed to count overrides for '{}'", plugin_name))?;

    Ok(PluginRecordSummary {
        plugin_name: plugin_name.to_string(),
        masters,
        record_and_group_count: plugin.record_and_group_count(),
        override_record_count: override_count,
        is_master: plugin.is_master_file(),
        is_light: plugin.is_light_plugin(),
        header_version: plugin.header_version(),
    })
}

/// Detect record-level conflicts across multiple plugins.
///
/// Returns a list of [`RecordConflict`] entries for every pair of plugins that
/// share at least one overridden FormID. Plugins are loaded from `data_dir`.
///
/// This uses esplugin's `overlaps_with` and `overlap_size` which compare the
/// actual FormIDs modified by each plugin — not just file-level conflicts.
pub fn detect_record_conflicts(
    data_dir: &Path,
    plugin_names: &[String],
    game_id: GameId,
) -> Result<Vec<RecordConflict>> {
    if plugin_names.is_empty() {
        return Ok(Vec::new());
    }

    // Parse all plugins up-front.
    let mut parsed: Vec<(String, Plugin)> = Vec::with_capacity(plugin_names.len());
    for name in plugin_names {
        let path = data_dir.join(name);
        if !path.is_file() {
            log::warn!("Skipping missing plugin for conflict scan: {}", name);
            continue;
        }
        let mut plugin = Plugin::new(game_id, &path);
        if let Err(e) = plugin.parse_file(ParseOptions::whole_plugin()) {
            log::warn!("Skipping unparseable plugin '{}': {:#}", name, e);
            continue;
        }
        parsed.push((name.clone(), plugin));
    }

    // For Morrowind/Starfield we need to resolve record IDs first.
    if matches!(game_id, GameId::Morrowind | GameId::Starfield) {
        let plugin_refs: Vec<&Plugin> = parsed.iter().map(|(_, p)| p).collect();
        let metadata = esplugin::plugins_metadata(&plugin_refs)
            .context("Failed to build plugin metadata for record ID resolution")?;
        for (_, plugin) in &mut parsed {
            plugin
                .resolve_record_ids(&metadata)
                .context("Failed to resolve record IDs")?;
        }
    }

    // Pairwise overlap detection.
    let mut conflicts = Vec::new();
    for i in 0..parsed.len() {
        for j in (i + 1)..parsed.len() {
            let overlaps = parsed[i]
                .1
                .overlaps_with(&parsed[j].1)
                .unwrap_or(false);
            if overlaps {
                // Get the actual count of overlapping records.
                let other_ref = &parsed[j].1;
                let count = parsed[i]
                    .1
                    .overlap_size(&[other_ref])
                    .unwrap_or(0);
                if count > 0 {
                    conflicts.push(RecordConflict {
                        plugin_a: parsed[i].0.clone(),
                        plugin_b: parsed[j].0.clone(),
                        overlap_count: count,
                    });
                }
            }
        }
    }

    Ok(conflicts)
}

/// Batch-analyze all plugins and return summaries keyed by plugin name.
pub fn analyze_all_plugins(
    data_dir: &Path,
    plugin_names: &[String],
    game_id: GameId,
) -> Result<HashMap<String, PluginRecordSummary>> {
    let mut results = HashMap::with_capacity(plugin_names.len());
    for name in plugin_names {
        match analyze_plugin(data_dir, name, game_id) {
            Ok(summary) => {
                results.insert(name.clone(), summary);
            }
            Err(e) => {
                log::warn!("Failed to analyze plugin '{}': {:#}", name, e);
            }
        }
    }
    Ok(results)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Helper: create a temporary directory with no plugins.
    fn empty_data_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("Failed to create temp dir")
    }

    #[test]
    fn test_esplugin_game_id_mapping() {
        assert_eq!(esplugin_game_id("skyrimse"), Some(GameId::SkyrimSE));
        assert_eq!(
            esplugin_game_id("skyrimspecialedition"),
            Some(GameId::SkyrimSE)
        );
        assert_eq!(esplugin_game_id("skyrim"), Some(GameId::Skyrim));
        assert_eq!(esplugin_game_id("skyrimle"), Some(GameId::Skyrim));
        assert_eq!(esplugin_game_id("fallout4"), Some(GameId::Fallout4));
        assert_eq!(esplugin_game_id("falloutnv"), Some(GameId::FalloutNV));
        assert_eq!(esplugin_game_id("fallout3"), Some(GameId::Fallout3));
        assert_eq!(esplugin_game_id("oblivion"), Some(GameId::Oblivion));
        assert_eq!(esplugin_game_id("morrowind"), Some(GameId::Morrowind));
        assert_eq!(esplugin_game_id("starfield"), Some(GameId::Starfield));
        assert_eq!(esplugin_game_id("unknown_game"), None);
    }

    #[test]
    fn test_analyze_plugin_missing_file() {
        let dir = empty_data_dir();
        let result = analyze_plugin(dir.path(), "nonexistent.esp", GameId::SkyrimSE);
        assert!(result.is_err());
        let err_msg = format!("{:#}", result.unwrap_err());
        assert!(
            err_msg.contains("not found"),
            "Expected 'not found' in error: {}",
            err_msg
        );
    }

    #[test]
    fn test_detect_conflicts_empty_list() {
        let dir = empty_data_dir();
        let result =
            detect_record_conflicts(dir.path(), &[], GameId::SkyrimSE).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_detect_conflicts_skips_missing() {
        let dir = empty_data_dir();
        let names = vec!["missing_a.esp".to_string(), "missing_b.esp".to_string()];
        let result =
            detect_record_conflicts(dir.path(), &names, GameId::SkyrimSE).unwrap();
        assert!(
            result.is_empty(),
            "Should return no conflicts for missing files"
        );
    }

    #[test]
    fn test_analyze_all_plugins_skips_missing() {
        let dir = empty_data_dir();
        let names = vec!["missing.esp".to_string()];
        let result =
            analyze_all_plugins(dir.path(), &names, GameId::SkyrimSE).unwrap();
        assert!(
            result.is_empty(),
            "Should skip missing plugins gracefully"
        );
    }

    /// Create a minimal valid Skyrim SE plugin (TES4 header only).
    /// This is the smallest file esplugin will accept as a valid plugin.
    fn write_minimal_sse_plugin(dir: &Path, name: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        // Minimal TES4 record for Skyrim SE:
        // Record type: TES4 (4 bytes)
        // Data size: 0 (4 bytes LE)
        // Flags: 0 (4 bytes LE) — no ESM flag
        // FormID: 0 (4 bytes LE)
        // Version control: 0 (4 bytes LE)
        // Form version: 44 (2 bytes LE) — Skyrim SE
        // Padding: 0 (2 bytes LE)
        // HEDR subrecord: type "HEDR" (4 bytes), size 12 (2 bytes LE)
        //   version: 1.70 f32 LE, record count: 0 u32 LE, next object id: 0x800 u32 LE
        let mut data: Vec<u8> = Vec::new();
        // TES4 record header
        data.extend_from_slice(b"TES4"); // type
        data.extend_from_slice(&36u32.to_le_bytes()); // data size (HEDR sub = 18 bytes + padding)
        data.extend_from_slice(&0u32.to_le_bytes()); // flags
        data.extend_from_slice(&0u32.to_le_bytes()); // formid
        data.extend_from_slice(&0u32.to_le_bytes()); // version control
        data.extend_from_slice(&44u16.to_le_bytes()); // form version (SSE = 44)
        data.extend_from_slice(&0u16.to_le_bytes()); // padding

        // HEDR subrecord
        data.extend_from_slice(b"HEDR"); // type
        data.extend_from_slice(&12u16.to_le_bytes()); // size
        data.extend_from_slice(&1.7f32.to_le_bytes()); // version
        data.extend_from_slice(&0u32.to_le_bytes()); // record count
        data.extend_from_slice(&0x800u32.to_le_bytes()); // next object id

        // CNAM subrecord (author) — needed to fill declared data size
        data.extend_from_slice(b"CNAM"); // type
        data.extend_from_slice(&5u16.to_le_bytes()); // size
        data.extend_from_slice(b"Test\0"); // null-terminated author string

        // SNAM subrecord (description)
        data.extend_from_slice(b"SNAM"); // type
        data.extend_from_slice(&1u16.to_le_bytes()); // size
        data.extend_from_slice(b"\0"); // null-terminated empty string

        // Fix up the data size: everything after the 24-byte record header
        let data_size = (data.len() - 24) as u32;
        data[4..8].copy_from_slice(&data_size.to_le_bytes());

        fs::write(&path, &data).expect("Failed to write test plugin");
        path
    }

    #[test]
    fn test_analyze_minimal_plugin() {
        let dir = empty_data_dir();
        let _path = write_minimal_sse_plugin(dir.path(), "test.esp");
        let summary =
            analyze_plugin(dir.path(), "test.esp", GameId::SkyrimSE).unwrap();
        assert_eq!(summary.plugin_name, "test.esp");
        assert!(!summary.is_master);
        assert!(!summary.is_light);
        assert!(summary.masters.is_empty());
        assert_eq!(summary.override_record_count, 0);
    }

    #[test]
    fn test_analyze_minimal_plugin_master_flag() {
        let dir = empty_data_dir();
        let path = write_minimal_sse_plugin(dir.path(), "test.esm");
        // Set the ESM flag (bit 0 of the flags field at offset 8)
        let mut data = fs::read(&path).unwrap();
        let flags = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
        let new_flags = flags | 0x1; // ESM flag
        data[8..12].copy_from_slice(&new_flags.to_le_bytes());
        fs::write(&path, &data).unwrap();

        let summary =
            analyze_plugin(dir.path(), "test.esm", GameId::SkyrimSE).unwrap();
        assert!(summary.is_master, "Plugin with ESM flag should report is_master=true");
    }

    #[test]
    fn test_detect_no_conflicts_between_independent_plugins() {
        let dir = empty_data_dir();
        write_minimal_sse_plugin(dir.path(), "a.esp");
        write_minimal_sse_plugin(dir.path(), "b.esp");

        let names = vec!["a.esp".to_string(), "b.esp".to_string()];
        let conflicts =
            detect_record_conflicts(dir.path(), &names, GameId::SkyrimSE).unwrap();
        assert!(
            conflicts.is_empty(),
            "Empty plugins should have no record conflicts"
        );
    }

    #[test]
    fn test_analyze_all_plugins_returns_all_valid() {
        let dir = empty_data_dir();
        write_minimal_sse_plugin(dir.path(), "a.esp");
        write_minimal_sse_plugin(dir.path(), "b.esp");

        let names = vec![
            "a.esp".to_string(),
            "b.esp".to_string(),
            "missing.esp".to_string(),
        ];
        let results =
            analyze_all_plugins(dir.path(), &names, GameId::SkyrimSE).unwrap();
        assert_eq!(results.len(), 2, "Should have 2 valid summaries");
        assert!(results.contains_key("a.esp"));
        assert!(results.contains_key("b.esp"));
        assert!(!results.contains_key("missing.esp"));
    }

    #[test]
    fn test_record_conflict_serializes() {
        let conflict = RecordConflict {
            plugin_a: "a.esp".to_string(),
            plugin_b: "b.esp".to_string(),
            overlap_count: 42,
        };
        let json = serde_json::to_string(&conflict).unwrap();
        assert!(json.contains("\"plugin_a\":\"a.esp\""));
        assert!(json.contains("\"overlap_count\":42"));
    }

    #[test]
    fn test_plugin_record_summary_serializes() {
        let summary = PluginRecordSummary {
            plugin_name: "test.esp".to_string(),
            masters: vec!["Skyrim.esm".to_string()],
            record_and_group_count: Some(100),
            override_record_count: 5,
            is_master: false,
            is_light: true,
            header_version: Some(1.7),
        };
        let json = serde_json::to_string(&summary).unwrap();
        assert!(json.contains("\"is_light\":true"));
        assert!(json.contains("\"override_record_count\":5"));
    }

    #[test]
    fn test_all_game_ids_are_mapped() {
        // Verify every esplugin GameId has a mapping
        let mappings = [
            ("skyrimse", GameId::SkyrimSE),
            ("skyrim", GameId::Skyrim),
            ("fallout4", GameId::Fallout4),
            ("falloutnv", GameId::FalloutNV),
            ("fallout3", GameId::Fallout3),
            ("oblivion", GameId::Oblivion),
            ("morrowind", GameId::Morrowind),
            ("starfield", GameId::Starfield),
        ];
        for (key, expected) in &mappings {
            assert_eq!(
                esplugin_game_id(key),
                Some(*expected),
                "Mapping failed for '{}'",
                key
            );
        }
    }
}
