//! Modlist import/export for sharing mod configurations between users.
//!
//! This module handles exporting the current mod configuration to a portable
//! JSON manifest and importing manifests to plan or apply mod installations.
//! The manifest format includes all installed mods, their metadata, and the
//! plugin load order, enabling users to share their exact setup.

use std::fs;
use std::path::Path;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::database::ModDatabase;
use crate::plugins::skyrim_plugins::PluginEntry;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Current format version for exported modlists.
const FORMAT_VERSION: u32 = 1;

/// Application version string embedded in exports.
const APP_VERSION: &str = "0.4.0-alpha";

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ModlistError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Incompatible format version: {0}")]
    IncompatibleVersion(u32),
}

// ---------------------------------------------------------------------------
// Exported types
// ---------------------------------------------------------------------------

/// A complete exported modlist manifest, portable and shareable.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExportedModlist {
    pub format_version: u32,
    pub app_version: String,
    pub exported_at: String,
    pub game_id: String,
    pub game_name: String,
    pub mod_count: usize,
    pub mods: Vec<ExportedMod>,
    pub plugin_order: Vec<ExportedPlugin>,
    pub notes: Option<String>,
}

/// A single mod entry in an exported modlist.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExportedMod {
    pub name: String,
    pub version: String,
    pub enabled: bool,
    pub priority: i32,
    pub nexus_mod_id: Option<i64>,
    pub nexus_file_id: Option<i64>,
    pub archive_name: String,
    pub source_url: Option<String>,
    #[serde(default = "default_source_type")]
    pub source_type: String,
    pub installed_files: Vec<String>,
    pub fomod_selections: Option<serde_json::Value>,
}

fn default_source_type() -> String {
    "manual".to_string()
}

/// A single plugin entry in an exported modlist.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExportedPlugin {
    pub filename: String,
    pub enabled: bool,
}

// ---------------------------------------------------------------------------
// Import planning types
// ---------------------------------------------------------------------------

/// The result of analysing a modlist against the current installation state.
/// Returned by `plan_import` for the UI to display before making changes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportPlan {
    pub game_id: String,
    pub total_mods: usize,
    pub nexus_mods: usize,
    pub manual_mods: usize,
    pub already_installed: usize,
    pub mods: Vec<ImportModStatus>,
    pub plugin_order: Vec<ExportedPlugin>,
}

/// Per-mod status in an import plan.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportModStatus {
    pub name: String,
    pub version: String,
    pub status: ImportStatus,
    pub nexus_mod_id: Option<i64>,
    pub nexus_file_id: Option<i64>,
    pub source_type: String,
    pub existing_mod_id: Option<i64>,
}

/// Classification of how an imported mod relates to the current installation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum ImportStatus {
    /// Exact match (name+version or nexus ID+version) already installed.
    AlreadyInstalled,
    /// Same mod is installed but with a different version.
    VersionMismatch,
    /// Has a Nexus mod ID so it can be downloaded automatically.
    CanAutoDownload,
    /// No Nexus ID and no source URL; user must download manually.
    NeedsManualDownload,
}

// ---------------------------------------------------------------------------
// Diff types
// ---------------------------------------------------------------------------

/// A comparison between two modlist states, showing what changed.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModlistDiff {
    /// Mods present in the imported list but not in the current state.
    pub added: Vec<String>,
    /// Mods present in the current state but not in the imported list.
    pub removed: Vec<String>,
    /// Mods whose version changed: (name, old_version, new_version).
    pub version_changed: Vec<(String, String, String)>,
    /// Mods whose priority changed: (name, old_priority, new_priority).
    pub priority_changed: Vec<(String, i32, i32)>,
    /// Mods whose enabled state changed: (name, old_enabled, new_enabled).
    pub enabled_changed: Vec<(String, bool, bool)>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Export the current mod configuration to a portable JSON manifest.
///
/// Queries all installed mods from the database for the given game/bottle,
/// pairs them with the current plugin load order, and builds a complete
/// `ExportedModlist` ready to be serialized and shared.
pub fn export_modlist(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    plugins: &[PluginEntry],
    notes: Option<&str>,
) -> Result<ExportedModlist, ModlistError> {
    let installed = db
        .list_mods(game_id, bottle_name)
        .map_err(|e| ModlistError::Database(e.to_string()))?;

    let mods: Vec<ExportedMod> = installed
        .iter()
        .map(|m| ExportedMod {
            name: m.name.clone(),
            version: m.version.clone(),
            enabled: m.enabled,
            priority: m.install_priority,
            nexus_mod_id: m.nexus_mod_id,
            nexus_file_id: m.nexus_file_id,
            archive_name: m.archive_name.clone(),
            source_url: m.source_url.clone(),
            source_type: m.source_type.clone(),
            installed_files: m.installed_files.clone(),
            fomod_selections: None,
        })
        .collect();

    let plugin_order: Vec<ExportedPlugin> = plugins
        .iter()
        .map(|p| ExportedPlugin {
            filename: p.filename.clone(),
            enabled: p.enabled,
        })
        .collect();

    Ok(ExportedModlist {
        format_version: FORMAT_VERSION,
        app_version: APP_VERSION.to_string(),
        exported_at: Utc::now().to_rfc3339(),
        game_id: game_id.to_string(),
        game_name: game_display_name(game_id).to_string(),
        mod_count: mods.len(),
        mods,
        plugin_order,
        notes: notes.map(|s| s.to_string()),
    })
}

/// Write an exported modlist to a file as pretty-printed JSON.
pub fn write_modlist_file(modlist: &ExportedModlist, path: &Path) -> Result<(), ModlistError> {
    let json = serde_json::to_string_pretty(modlist)?;
    fs::write(path, json)?;
    Ok(())
}

/// Read a modlist file from disk and deserialize it.
pub fn read_modlist_file(path: &Path) -> Result<ExportedModlist, ModlistError> {
    let contents = fs::read_to_string(path)?;
    let modlist: ExportedModlist = serde_json::from_str(&contents)?;
    Ok(modlist)
}

/// Analyse what importing a modlist would do without making any changes.
///
/// For each mod in the import, checks whether it is already installed
/// (by nexus_mod_id or name+version match), has a version mismatch,
/// can be auto-downloaded from Nexus, or requires manual action.
pub fn plan_import(
    db: &ModDatabase,
    modlist: &ExportedModlist,
    game_id: &str,
    bottle_name: &str,
) -> Result<ImportPlan, ModlistError> {
    let installed = db
        .list_mods(game_id, bottle_name)
        .map_err(|e| ModlistError::Database(e.to_string()))?;

    let mut mod_statuses = Vec::new();
    let mut nexus_count = 0usize;
    let mut manual_count = 0usize;
    let mut already_count = 0usize;

    for em in &modlist.mods {
        // Try to find an existing match.
        let existing = find_existing_mod(&installed, em);

        let (status, existing_mod_id) = match existing {
            Some((id, true)) => {
                // Exact match: same version.
                already_count += 1;
                (ImportStatus::AlreadyInstalled, Some(id))
            }
            Some((id, false)) => {
                // Same mod but different version.
                (ImportStatus::VersionMismatch, Some(id))
            }
            None => {
                // Not installed at all.
                if em.nexus_mod_id.is_some() {
                    nexus_count += 1;
                    (ImportStatus::CanAutoDownload, None)
                } else {
                    manual_count += 1;
                    (ImportStatus::NeedsManualDownload, None)
                }
            }
        };

        mod_statuses.push(ImportModStatus {
            name: em.name.clone(),
            version: em.version.clone(),
            status,
            nexus_mod_id: em.nexus_mod_id,
            nexus_file_id: em.nexus_file_id,
            source_type: em.source_type.clone(),
            existing_mod_id,
        });
    }

    Ok(ImportPlan {
        game_id: game_id.to_string(),
        total_mods: modlist.mods.len(),
        nexus_mods: nexus_count,
        manual_mods: manual_count,
        already_installed: already_count,
        mods: mod_statuses,
        plugin_order: modlist.plugin_order.clone(),
    })
}

/// Compare two modlists and report all differences.
///
/// Compares `current` (the user's current state) against `imported` (the
/// incoming modlist) and produces a diff of added, removed, version-changed,
/// priority-changed, and enabled-changed mods.
pub fn diff_modlists(current: &ExportedModlist, imported: &ExportedModlist) -> ModlistDiff {
    use std::collections::HashMap;

    // Index current mods by name (lowercase for case-insensitive matching).
    let current_map: HashMap<String, &ExportedMod> = current
        .mods
        .iter()
        .map(|m| (m.name.to_lowercase(), m))
        .collect();

    let imported_map: HashMap<String, &ExportedMod> = imported
        .mods
        .iter()
        .map(|m| (m.name.to_lowercase(), m))
        .collect();

    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut version_changed = Vec::new();
    let mut priority_changed = Vec::new();
    let mut enabled_changed = Vec::new();

    // Mods in imported but not in current => added.
    // Mods in both => check for changes.
    for im in &imported.mods {
        let key = im.name.to_lowercase();
        match current_map.get(&key) {
            None => {
                added.push(im.name.clone());
            }
            Some(cm) => {
                if cm.version != im.version {
                    version_changed.push((im.name.clone(), cm.version.clone(), im.version.clone()));
                }
                if cm.priority != im.priority {
                    priority_changed.push((im.name.clone(), cm.priority, im.priority));
                }
                if cm.enabled != im.enabled {
                    enabled_changed.push((im.name.clone(), cm.enabled, im.enabled));
                }
            }
        }
    }

    // Mods in current but not in imported => removed.
    for cm in &current.mods {
        let key = cm.name.to_lowercase();
        if !imported_map.contains_key(&key) {
            removed.push(cm.name.clone());
        }
    }

    ModlistDiff {
        added,
        removed,
        version_changed,
        priority_changed,
        enabled_changed,
    }
}

/// Validate a modlist file for structural correctness and game compatibility.
///
/// Checks that the format version is supported and that the modlist targets
/// the expected game.
pub fn validate_modlist(modlist: &ExportedModlist, game_id: &str) -> Result<(), ModlistError> {
    if modlist.format_version != FORMAT_VERSION {
        return Err(ModlistError::IncompatibleVersion(modlist.format_version));
    }

    if modlist.game_id != game_id {
        return Err(ModlistError::Validation(format!(
            "Modlist is for '{}' ({}), but current game is '{}' ({})",
            modlist.game_name,
            modlist.game_id,
            game_display_name(game_id),
            game_id,
        )));
    }

    if modlist.mod_count != modlist.mods.len() {
        return Err(ModlistError::Validation(format!(
            "Modlist declares {} mods but contains {}",
            modlist.mod_count,
            modlist.mods.len(),
        )));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Map a game_id to a human-readable display name.
fn game_display_name(game_id: &str) -> &str {
    match game_id {
        "skyrimse" => "Skyrim Special Edition",
        "skyrim" => "Skyrim (Classic)",
        "fallout4" => "Fallout 4",
        "falloutnv" => "Fallout: New Vegas",
        "oblivion" => "The Elder Scrolls IV: Oblivion",
        "morrowind" => "The Elder Scrolls III: Morrowind",
        "starfield" => "Starfield",
        "enderal" => "Enderal",
        "enderalse" => "Enderal Special Edition",
        _ => game_id,
    }
}

/// Result of executing a modlist import.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportResult {
    /// How many already-installed mods had their priority/enabled state updated.
    pub mods_updated: usize,
    /// How many mods were already installed and needed no changes.
    pub mods_skipped: usize,
    /// Mods that need to be downloaded (not yet installed).
    pub mods_to_download: Vec<ImportDownloadAction>,
    /// Errors encountered while applying changes.
    pub errors: Vec<String>,
}

/// A mod that the user needs to download to complete the import.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportDownloadAction {
    pub name: String,
    pub version: String,
    pub nexus_mod_id: Option<i64>,
    pub nexus_file_id: Option<i64>,
    pub source_type: String,
    pub source_url: Option<String>,
}

/// Execute a modlist import: apply priority/enabled changes for already-installed
/// mods and return a list of download actions for missing mods.
///
/// This does NOT trigger any downloads — the frontend is responsible for that
/// (respecting NexusMods free-user restrictions).
pub fn execute_import(
    db: &ModDatabase,
    modlist: &ExportedModlist,
    game_id: &str,
    bottle_name: &str,
) -> Result<ImportResult, ModlistError> {
    let installed = db
        .list_mods(game_id, bottle_name)
        .map_err(|e| ModlistError::Database(e.to_string()))?;

    let mut mods_updated = 0usize;
    let mut mods_skipped = 0usize;
    let mut mods_to_download = Vec::new();
    let mut errors = Vec::new();

    for (i, em) in modlist.mods.iter().enumerate() {
        let target_priority = em.priority.max(i as i32);

        match find_existing_mod(&installed, em) {
            Some((mod_id, _version_matches)) => {
                // Mod is installed — update priority and enabled state
                let mut changed = false;

                if let Err(e) = db.set_mod_priority(mod_id, target_priority) {
                    errors.push(format!("{}: failed to set priority: {}", em.name, e));
                } else {
                    changed = true;
                }

                // Get current state to check if enabled needs toggling
                if let Ok(Some(current)) = db.get_mod(mod_id) {
                    if current.enabled != em.enabled {
                        if let Err(e) = db.set_enabled(mod_id, em.enabled) {
                            errors.push(format!("{}: failed to toggle: {}", em.name, e));
                        } else {
                            changed = true;
                        }
                    }
                }

                if changed {
                    mods_updated += 1;
                } else {
                    mods_skipped += 1;
                }
            }
            None => {
                // Mod not installed — add to download list
                mods_to_download.push(ImportDownloadAction {
                    name: em.name.clone(),
                    version: em.version.clone(),
                    nexus_mod_id: em.nexus_mod_id,
                    nexus_file_id: em.nexus_file_id,
                    source_type: em.source_type.clone(),
                    source_url: em.source_url.clone(),
                });
            }
        }
    }

    Ok(ImportResult {
        mods_updated,
        mods_skipped,
        mods_to_download,
        errors,
    })
}

/// Try to find an existing installed mod that matches an exported mod entry.
///
/// Returns `Some((mod_id, version_matches))` if a match is found, where
/// `version_matches` is `true` when the version strings are identical.
///
/// Match priority:
/// 1. Same `nexus_mod_id` (strongest signal)
/// 2. Same name (case-insensitive) + same version
/// 3. Same name (case-insensitive) with different version
fn find_existing_mod(
    installed: &[crate::database::InstalledMod],
    exported: &ExportedMod,
) -> Option<(i64, bool)> {
    // First try matching by Nexus mod ID (most reliable).
    if let Some(nexus_id) = exported.nexus_mod_id {
        for m in installed {
            if m.nexus_mod_id == Some(nexus_id) {
                let version_match = m.version == exported.version;
                return Some((m.id, version_match));
            }
        }
    }

    // Fall back to name matching (case-insensitive).
    let target_name = exported.name.to_lowercase();
    for m in installed {
        if m.name.to_lowercase() == target_name {
            let version_match = m.version == exported.version;
            return Some((m.id, version_match));
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::ModDatabase;
    use tempfile::TempDir;

    /// Helper: create a database inside a temporary directory.
    fn test_db() -> (ModDatabase, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test_modlist.db");
        let db = ModDatabase::new(&db_path).unwrap();
        (db, tmp)
    }

    /// Helper: build a minimal ExportedModlist for testing.
    fn sample_modlist(game_id: &str, mods: Vec<ExportedMod>) -> ExportedModlist {
        let mod_count = mods.len();
        ExportedModlist {
            format_version: FORMAT_VERSION,
            app_version: APP_VERSION.to_string(),
            exported_at: Utc::now().to_rfc3339(),
            game_id: game_id.to_string(),
            game_name: game_display_name(game_id).to_string(),
            mod_count,
            mods,
            plugin_order: vec![ExportedPlugin {
                filename: "Skyrim.esm".to_string(),
                enabled: true,
            }],
            notes: Some("Test modlist".to_string()),
        }
    }

    /// Helper: build a sample ExportedMod.
    fn sample_exported_mod(name: &str, version: &str, nexus_mod_id: Option<i64>) -> ExportedMod {
        ExportedMod {
            name: name.to_string(),
            version: version.to_string(),
            enabled: true,
            priority: 0,
            nexus_mod_id,
            nexus_file_id: None,
            archive_name: format!("{}.zip", name.to_lowercase().replace(' ', "_")),
            source_url: None,
            source_type: if nexus_mod_id.is_some() { "nexus".to_string() } else { "manual".to_string() },
            installed_files: vec![format!(
                "data/meshes/{}.nif",
                name.to_lowercase().replace(' ', "_")
            )],
            fomod_selections: None,
        }
    }

    // -- Test: export creates valid JSON ------------------------------------

    #[test]
    fn test_export_creates_valid_json() {
        let (db, _tmp) = test_db();

        let files = vec![
            "data/meshes/armor.nif".to_string(),
            "data/textures/armor.dds".to_string(),
        ];
        db.add_mod(
            "skyrimse",
            "default",
            Some(1234),
            "Cool Armor",
            "2.1",
            "cool_armor.zip",
            &files,
        )
        .unwrap();

        let plugins = vec![
            PluginEntry {
                filename: "Skyrim.esm".to_string(),
                enabled: true,
            },
            PluginEntry {
                filename: "CoolArmor.esp".to_string(),
                enabled: true,
            },
        ];

        let modlist =
            export_modlist(&db, "skyrimse", "default", &plugins, Some("My setup")).unwrap();

        // Serialize to JSON and parse back to verify it is valid.
        let json = serde_json::to_string_pretty(&modlist).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["format_version"], 1);
        assert_eq!(parsed["app_version"], APP_VERSION);
        assert_eq!(parsed["game_id"], "skyrimse");
        assert_eq!(parsed["game_name"], "Skyrim Special Edition");
        assert_eq!(parsed["mod_count"], 1);
        assert_eq!(parsed["mods"][0]["name"], "Cool Armor");
        assert_eq!(parsed["mods"][0]["version"], "2.1");
        assert_eq!(parsed["mods"][0]["nexus_mod_id"], 1234);
        assert_eq!(parsed["plugin_order"].as_array().unwrap().len(), 2);
        assert_eq!(parsed["notes"], "My setup");
    }

    // -- Test: round-trip (export -> write -> read -> compare) --------------

    #[test]
    fn test_roundtrip_write_read() {
        let (db, tmp) = test_db();

        db.add_mod(
            "skyrimse",
            "default",
            Some(5678),
            "Magic Overhaul",
            "3.0",
            "magic_overhaul.7z",
            &["data/scripts/magic.pex".to_string()],
        )
        .unwrap();
        db.add_mod(
            "skyrimse",
            "default",
            None,
            "Texture Pack",
            "1.2",
            "textures.zip",
            &["data/textures/landscape.dds".to_string()],
        )
        .unwrap();

        let plugins = vec![
            PluginEntry {
                filename: "Skyrim.esm".to_string(),
                enabled: true,
            },
            PluginEntry {
                filename: "MagicOverhaul.esp".to_string(),
                enabled: true,
            },
        ];

        let original = export_modlist(
            &db,
            "skyrimse",
            "default",
            &plugins,
            Some("Round trip test"),
        )
        .unwrap();

        let file_path = tmp.path().join("modlist.json");
        write_modlist_file(&original, &file_path).unwrap();

        let loaded = read_modlist_file(&file_path).unwrap();

        assert_eq!(loaded.format_version, original.format_version);
        assert_eq!(loaded.app_version, original.app_version);
        assert_eq!(loaded.game_id, original.game_id);
        assert_eq!(loaded.game_name, original.game_name);
        assert_eq!(loaded.mod_count, original.mod_count);
        assert_eq!(loaded.mods.len(), original.mods.len());
        assert_eq!(loaded.plugin_order.len(), original.plugin_order.len());
        assert_eq!(loaded.notes, original.notes);

        // Verify each mod survived the round trip.
        for (orig, read) in original.mods.iter().zip(loaded.mods.iter()) {
            assert_eq!(orig.name, read.name);
            assert_eq!(orig.version, read.version);
            assert_eq!(orig.enabled, read.enabled);
            assert_eq!(orig.priority, read.priority);
            assert_eq!(orig.nexus_mod_id, read.nexus_mod_id);
            assert_eq!(orig.archive_name, read.archive_name);
            assert_eq!(orig.installed_files, read.installed_files);
        }
    }

    // -- Test: import plan classifies mods correctly ------------------------

    #[test]
    fn test_import_plan_classification() {
        let (db, _tmp) = test_db();

        // Install two mods in the DB.
        db.add_mod(
            "skyrimse",
            "default",
            Some(100),
            "Already Here",
            "1.0",
            "already_here.zip",
            &[],
        )
        .unwrap();
        db.add_mod(
            "skyrimse",
            "default",
            Some(200),
            "Version Mismatch",
            "1.0",
            "version_mismatch.zip",
            &[],
        )
        .unwrap();

        // Build an imported modlist with four mods:
        let mods = vec![
            // 1. Exact match by nexus_mod_id + version
            sample_exported_mod("Already Here", "1.0", Some(100)),
            // 2. Same nexus_mod_id, different version
            sample_exported_mod("Version Mismatch", "2.0", Some(200)),
            // 3. Not installed, has nexus ID -> CanAutoDownload
            sample_exported_mod("New Nexus Mod", "1.0", Some(300)),
            // 4. Not installed, no nexus ID -> NeedsManualDownload
            sample_exported_mod("Manual Only Mod", "1.0", None),
        ];

        let modlist = sample_modlist("skyrimse", mods);
        let plan = plan_import(&db, &modlist, "skyrimse", "default").unwrap();

        assert_eq!(plan.total_mods, 4);
        assert_eq!(plan.already_installed, 1);
        assert_eq!(plan.nexus_mods, 1);
        assert_eq!(plan.manual_mods, 1);

        assert_eq!(plan.mods[0].status, ImportStatus::AlreadyInstalled);
        assert!(plan.mods[0].existing_mod_id.is_some());

        assert_eq!(plan.mods[1].status, ImportStatus::VersionMismatch);
        assert!(plan.mods[1].existing_mod_id.is_some());

        assert_eq!(plan.mods[2].status, ImportStatus::CanAutoDownload);
        assert!(plan.mods[2].existing_mod_id.is_none());

        assert_eq!(plan.mods[3].status, ImportStatus::NeedsManualDownload);
        assert!(plan.mods[3].existing_mod_id.is_none());
    }

    // -- Test: diff detection -----------------------------------------------

    #[test]
    fn test_diff_detection() {
        let current = sample_modlist(
            "skyrimse",
            vec![
                {
                    let mut m = sample_exported_mod("Shared Mod", "1.0", Some(10));
                    m.priority = 0;
                    m.enabled = true;
                    m
                },
                sample_exported_mod("Removed Mod", "1.0", Some(20)),
                {
                    let mut m = sample_exported_mod("Version Changed", "1.0", Some(30));
                    m.priority = 2;
                    m
                },
                {
                    let mut m = sample_exported_mod("Priority Changed", "1.0", Some(40));
                    m.priority = 3;
                    m.enabled = true;
                    m
                },
                {
                    let mut m = sample_exported_mod("Enabled Changed", "1.0", Some(50));
                    m.enabled = true;
                    m
                },
            ],
        );

        let imported = sample_modlist(
            "skyrimse",
            vec![
                {
                    let mut m = sample_exported_mod("Shared Mod", "1.0", Some(10));
                    m.priority = 0;
                    m.enabled = true;
                    m
                },
                sample_exported_mod("Added Mod", "1.0", Some(60)),
                {
                    let mut m = sample_exported_mod("Version Changed", "2.0", Some(30));
                    m.priority = 2;
                    m
                },
                {
                    let mut m = sample_exported_mod("Priority Changed", "1.0", Some(40));
                    m.priority = 10;
                    m.enabled = true;
                    m
                },
                {
                    let mut m = sample_exported_mod("Enabled Changed", "1.0", Some(50));
                    m.enabled = false;
                    m
                },
            ],
        );

        let diff = diff_modlists(&current, &imported);

        assert_eq!(diff.added, vec!["Added Mod"]);
        assert_eq!(diff.removed, vec!["Removed Mod"]);
        assert_eq!(
            diff.version_changed,
            vec![(
                "Version Changed".to_string(),
                "1.0".to_string(),
                "2.0".to_string()
            )]
        );
        assert_eq!(
            diff.priority_changed,
            vec![("Priority Changed".to_string(), 3, 10)]
        );
        assert_eq!(
            diff.enabled_changed,
            vec![("Enabled Changed".to_string(), true, false)]
        );
    }

    // -- Test: validation catches wrong game --------------------------------

    #[test]
    fn test_validate_catches_wrong_game() {
        let modlist = sample_modlist("skyrimse", vec![]);

        // Should pass for the correct game.
        assert!(validate_modlist(&modlist, "skyrimse").is_ok());

        // Should fail for a different game.
        let result = validate_modlist(&modlist, "fallout4");
        assert!(result.is_err());
        match result {
            Err(ModlistError::Validation(msg)) => {
                assert!(
                    msg.contains("skyrimse"),
                    "Error should mention the modlist game_id"
                );
                assert!(
                    msg.contains("fallout4"),
                    "Error should mention the target game_id"
                );
            }
            other => panic!("Expected Validation error, got: {:?}", other),
        }

        // Should fail for incompatible format version.
        let mut bad_version = sample_modlist("skyrimse", vec![]);
        bad_version.format_version = 99;
        let result = validate_modlist(&bad_version, "skyrimse");
        assert!(result.is_err());
        match result {
            Err(ModlistError::IncompatibleVersion(v)) => assert_eq!(v, 99),
            other => panic!("Expected IncompatibleVersion error, got: {:?}", other),
        }

        // Should fail for mismatched mod count.
        let mut bad_count =
            sample_modlist("skyrimse", vec![sample_exported_mod("Test", "1.0", None)]);
        bad_count.mod_count = 5; // Claims 5 but only has 1.
        let result = validate_modlist(&bad_count, "skyrimse");
        assert!(result.is_err());
        match result {
            Err(ModlistError::Validation(msg)) => {
                assert!(msg.contains("5"), "Error should mention declared count");
                assert!(msg.contains("1"), "Error should mention actual count");
            }
            other => panic!("Expected Validation error, got: {:?}", other),
        }
    }

    // -- Test: empty modlist handling ---------------------------------------

    #[test]
    fn test_empty_modlist_handling() {
        let (db, tmp) = test_db();

        // Export with no mods and no plugins.
        let modlist = export_modlist(&db, "skyrimse", "default", &[], None).unwrap();
        assert_eq!(modlist.mod_count, 0);
        assert!(modlist.mods.is_empty());
        assert!(modlist.plugin_order.is_empty());
        assert!(modlist.notes.is_none());

        // Round-trip an empty modlist through disk.
        let file_path = tmp.path().join("empty.json");
        write_modlist_file(&modlist, &file_path).unwrap();
        let loaded = read_modlist_file(&file_path).unwrap();
        assert_eq!(loaded.mod_count, 0);
        assert!(loaded.mods.is_empty());

        // Validate passes.
        validate_modlist(&loaded, "skyrimse").unwrap();

        // Import plan on empty modlist.
        let plan = plan_import(&db, &loaded, "skyrimse", "default").unwrap();
        assert_eq!(plan.total_mods, 0);
        assert_eq!(plan.already_installed, 0);
        assert_eq!(plan.nexus_mods, 0);
        assert_eq!(plan.manual_mods, 0);

        // Diff of two empty modlists.
        let diff = diff_modlists(&modlist, &loaded);
        assert!(diff.added.is_empty());
        assert!(diff.removed.is_empty());
        assert!(diff.version_changed.is_empty());
        assert!(diff.priority_changed.is_empty());
        assert!(diff.enabled_changed.is_empty());
    }

    // -- Test: name-based matching fallback ---------------------------------

    #[test]
    fn test_import_plan_name_matching_fallback() {
        let (db, _tmp) = test_db();

        // Install a mod without a Nexus ID.
        db.add_mod(
            "skyrimse",
            "default",
            None,
            "Handcrafted Mod",
            "1.0",
            "handcrafted.zip",
            &[],
        )
        .unwrap();

        let mods = vec![
            // Same name, same version, no nexus ID -> AlreadyInstalled via name match
            sample_exported_mod("Handcrafted Mod", "1.0", None),
            // Same name, different version, no nexus ID -> VersionMismatch via name match
            sample_exported_mod("Handcrafted Mod v2", "2.0", None),
        ];

        // Manually fix the second mod's name to match the installed one for
        // the version mismatch test.
        let mut mods = mods;
        mods[1].name = "Handcrafted Mod".to_string();
        mods[1].version = "2.0".to_string();

        // Since both have the same name, the second occurrence tests
        // the version mismatch path. We build two separate modlists to
        // test each independently.

        let modlist_exact = sample_modlist("skyrimse", vec![mods[0].clone()]);
        let plan_exact = plan_import(&db, &modlist_exact, "skyrimse", "default").unwrap();
        assert_eq!(plan_exact.mods[0].status, ImportStatus::AlreadyInstalled);

        let modlist_mismatch = sample_modlist("skyrimse", vec![mods[1].clone()]);
        let plan_mismatch = plan_import(&db, &modlist_mismatch, "skyrimse", "default").unwrap();
        assert_eq!(plan_mismatch.mods[0].status, ImportStatus::VersionMismatch);
    }

    // -- Test: game display name mapping ------------------------------------

    #[test]
    fn test_game_display_name() {
        assert_eq!(game_display_name("skyrimse"), "Skyrim Special Edition");
        assert_eq!(game_display_name("fallout4"), "Fallout 4");
        assert_eq!(game_display_name("unknown_game"), "unknown_game");
    }
}
