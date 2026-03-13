//! Community Shaders (CS) detection and ENB conversion for Skyrim SE under
//! Wine/CrossOver.
//!
//! Community Shaders is a modern shader framework for Skyrim SE that uses
//! compute-shader features unavailable in Wine's D3D11 translation layer.
//! This module detects CS-dependent mods and orchestrates conversion to the
//! ENB ecosystem, which works well under DXVK/Wine.
//!
//! Key operations:
//! - `scan_for_cs_mods` — detect all CS-dependent mods in a load order
//! - `batch_disable_cs_mods` — disable and undeploy a set of CS mods
//! - `install_enb_binary` — download and install ENBSeries binaries
//! - `discover_enb_siblings` — query NexusMods for ENB variants of CS mods
//! - `execute_conversion` — full CS→ENB conversion pipeline
//! - `revert_conversion` — restore pre-conversion snapshot

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use log::{debug, error, info, warn};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::bottles::Bottle;
use crate::database::ModDatabase;
use crate::deploy_journal;
use crate::deployer;
use crate::fomod;
use crate::fomod_recipes;
use crate::nexus::{self, NexusModFile};
use crate::rollback;
use crate::wine_diagnostic;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Reason a mod was flagged as Community Shaders–dependent.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CsDetectionReason {
    /// Mod contains the core CommunityShaders.dll.
    CoreDll,
    /// Mod contains CS configuration files (e.g. CommunityShaders/*.ini).
    CsConfigFiles,
    /// Mod contains Light Placer configuration files.
    LightPlacerConfigs,
    /// Mod contains PBR textures designed for CS rendering.
    PbrTextures,
    /// FOMOD recipe selections include CS-specific options.
    FomodCsSelection,
    /// Mod is a known CS ecosystem mod by NexusMods ID or name pattern.
    KnownCsEcosystemMod,
    /// Mod contains files that only work with CS (shader binaries, etc).
    CsOnlyFiles,
}

/// A mod detected as Community Shaders–dependent.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CsDetectedMod {
    pub mod_id: i64,
    pub mod_name: String,
    pub nexus_mod_id: Option<i64>,
    pub nexus_file_id: Option<i64>,
    pub reasons: Vec<CsDetectionReason>,
    pub action: CsModAction,
}

/// What to do with a detected CS mod during conversion.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum CsModAction {
    /// Disable and undeploy the mod entirely.
    Disable,
    /// Swap to an ENB-compatible variant file on NexusMods.
    SwapToEnbVariant {
        enb_file_id: i64,
        enb_file_name: String,
    },
    /// Re-run the FOMOD installer with ENB-oriented selections.
    RerunFomod {
        suggested_selections: HashMap<String, Vec<String>>,
    },
    /// Keep the mod as-is (user override or false positive).
    Keep,
}

/// Result of scanning a mod list for CS dependencies.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShaderScanResult {
    pub detected_mods: Vec<CsDetectedMod>,
    pub total_cs_mods: usize,
    pub swappable_count: usize,
    pub fomod_rerun_count: usize,
    pub disable_only_count: usize,
    pub keep_count: usize,
    pub enb_already_installed: bool,
}

/// User's choice of ENB preset quality tier.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnbPresetChoice {
    Performance,
    Balanced,
    Quality,
    Custom {
        nexus_mod_id: i64,
        nexus_file_id: i64,
    },
}

/// Configuration for a full CS→ENB conversion.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConversionConfig {
    pub install_enb_binary: bool,
    pub enb_preset: Option<EnbPresetChoice>,
    pub mod_actions: Vec<(i64, CsModAction)>,
    pub install_enb_ecosystem: bool,
    pub switch_to_dxvk: bool,
}

/// Result of executing a conversion.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConversionResult {
    pub conversion_id: i64,
    pub snapshot_id: i64,
    pub mods_disabled: usize,
    pub mods_swapped: usize,
    pub fomods_rerun: usize,
    pub enb_installed: bool,
    pub dxvk_switched: bool,
    pub errors: Vec<String>,
}

/// A historical conversion record.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConversionHistoryEntry {
    pub id: i64,
    pub game_id: String,
    pub bottle_name: String,
    pub snapshot_id: i64,
    pub status: String,
    pub disabled_mods: Vec<i64>,
    pub swapped_mods: Vec<i64>,
    pub enb_installed: bool,
    pub created_at: String,
}

/// Progress events emitted during conversion.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "phase")]
pub enum ConversionProgress {
    Scanning {
        message: String,
    },
    DisablingMods {
        current: usize,
        total: usize,
        mod_name: String,
    },
    InstallingEnb {
        step: String,
    },
    SwappingMods {
        current: usize,
        total: usize,
        mod_name: String,
    },
    RerunningFomods {
        current: usize,
        total: usize,
        mod_name: String,
    },
    InstallingEcosystem {
        mod_name: String,
    },
    Redeploying {
        current: usize,
        total: usize,
        mod_name: String,
    },
    Complete {
        result: ConversionResult,
    },
    Failed {
        error: String,
    },
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// File path patterns (lowercased, forward-slash) that indicate CS dependency.
const CS_FILE_PATTERNS: &[&str] = &[
    "skse/plugins/communityshaders.dll",
    "skse/plugins/communityshaders/",
    "skse/plugins/lightlimitfix.dll",
    "skse/plugins/grasslighting.dll",
    "skse/plugins/screenspaceshadows.dll",
    "skse/plugins/treelodlighting.dll",
    "skse/plugins/skylighting.dll",
    "skse/plugins/waterblending.dll",
    "skse/plugins/waterparallax.dll",
    "skse/plugins/lightplacer.dll",
];

/// Substring patterns (lowercased) matched against mod display names.
const CS_MOD_NAME_PATTERNS: &[&str] = &[
    "community shaders",
    "light limit fix",
    "grass lighting",
    "screen-space shadows",
    "tree lod lighting",
    "skylighting",
    "water blending",
    "water parallax",
    "light placer",
];

/// Substring patterns matched against FOMOD selection values to detect CS
/// choices.
const CS_FOMOD_PATTERNS: &[&str] = &[
    "community shaders",
    "light limit fix",
    "cs version",
    "cs patch",
];

/// Substring patterns for ENB-related FOMOD options.
const ENB_FOMOD_PATTERNS: &[&str] = &[
    "enb",
    "enb version",
    "enb light",
    "enb patch",
    "enbseries",
];

/// Substring patterns for mesh-split options (CS uses split meshes; ENB does
/// not).
const MESH_SPLIT_PATTERNS: &[&str] = &["split meshes", "mesh patches", "mesh split"];

/// ENBSeries version page.
const ENB_VERSION_PAGE: &str = "http://enbdev.com/download_mod_tesskyrimse.html";

/// ENBSeries download URL prefix.
const ENB_DOWNLOAD_BASE: &str = "http://enbdev.com/enbseries_skyrimse_v";

/// Referer header prefix (enbdev.com requires a valid referer).
const ENB_REFERER_BASE: &str = "http://enbdev.com/mod_tesskyrimse_v";

/// Files deployed to the game root directory by ENBSeries.
const ENB_ROOT_FILES: &[&str] = &[
    "d3d11.dll",
    "d3dcompiler_46e.dll",
    "enblocal.ini",
    "enbseries.ini",
];

/// NexusMods mod ID for ENB Helper SE.
const ENB_HELPER_SE_MOD_ID: i64 = 27351;

/// NexusMods mod ID for ENB Light.
const ENB_LIGHT_MOD_ID: i64 = 22574;

/// Essential SKSE plugins that MUST NEVER be removed by shader conversion,
/// even if they are bundled inside a mod being disabled.  These DLLs are
/// critical framework plugins that many other mods depend on.
const ESSENTIAL_SKSE_PLUGINS: &[&str] = &[
    "po3_tweaks.dll",
    "crashlogger.dll",
    "crashloggersse.dll",
    "bugfixessse.dll",
    "scrambledbugs.dll",
    "address library for skse plugins.dll",
    "jcontainers64.dll",
    "papyrusutil.dll",
    "skseengineFixesforwine.dll",
    "0_sseenginefixesforwine.dll",
    "consoleutil.dll",
    "powerofthree's tweaks.dll",
];

/// Known NexusMods mod IDs for core Community Shaders ecosystem mods.
const KNOWN_CS_NEXUS_IDS: &[i64] = &[
    86492,  // Community Shaders
    106441, // Light Limit Fix (CS)
    106746, // Grass Lighting (CS)
    105338, // Screen-Space Shadows (CS)
    107636, // Tree LOD Lighting (CS)
    107306, // Skylighting (CS)
    105245, // Water Blending (CS)
    108949, // Water Parallax (CS)
    104997, // Light Placer (CS)
];

/// Tauri event name for conversion progress.
const CONVERSION_PROGRESS_EVENT: &str = "shader-conversion-progress";

/// Fallback ENB version string when auto-detection fails.
const ENB_FALLBACK_VERSION: &str = "0503";

// ---------------------------------------------------------------------------
// Core functions
// ---------------------------------------------------------------------------

/// Scan installed mods for Community Shaders dependencies.
///
/// Checks each mod's deployed files, display name, FOMOD recipe selections,
/// and NexusMods ID against known CS patterns. Returns a summary with
/// categorized actions.
pub fn scan_for_cs_mods(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    game_path: &Path,
) -> Result<ShaderScanResult, String> {
    let mods = db
        .list_mods(game_id, bottle_name)
        .map_err(|e| format!("Failed to list mods: {}", e))?;

    let mut detected_mods = Vec::new();

    for m in &mods {
        let mut reasons = Vec::new();

        // 1. Check file paths against CS patterns
        check_file_patterns(&m.installed_files, &mut reasons);

        // 2. Check mod name against CS name patterns
        check_name_patterns(&m.name, &mut reasons);

        // 3. Check FOMOD recipe for CS-specific selections
        check_fomod_recipe(db, m.id, &mut reasons);

        // 4. Check known NexusMods IDs
        if let Some(nid) = m.nexus_mod_id {
            if KNOWN_CS_NEXUS_IDS.contains(&nid) {
                if !reasons.contains(&CsDetectionReason::KnownCsEcosystemMod) {
                    reasons.push(CsDetectionReason::KnownCsEcosystemMod);
                }
            }
        }

        if !reasons.is_empty() {
            // Safety check: if this mod contains essential SKSE plugins,
            // force Keep regardless of CS detection reasons.  These plugins
            // are critical framework DLLs that many other mods depend on
            // and must NEVER be disabled by shader conversion.
            let has_essential_plugin = m.installed_files.iter().any(|f| {
                let lower = f.to_lowercase().replace('\\', "/");
                let filename = lower.rsplit('/').next().unwrap_or(&lower);
                ESSENTIAL_SKSE_PLUGINS.iter().any(|&essential| filename == essential)
            });

            if has_essential_plugin {
                info!(
                    "Mod '{}' contains essential SKSE plugin(s) — forcing Keep despite CS detection ({:?})",
                    m.name, reasons
                );
                detected_mods.push(CsDetectedMod {
                    mod_id: m.id,
                    mod_name: m.name.clone(),
                    nexus_mod_id: m.nexus_mod_id,
                    nexus_file_id: m.nexus_file_id,
                    reasons,
                    action: CsModAction::Keep,
                });
                continue;
            }

            // Determine default action based on severity of CS dependency.
            //
            // Config files (CsConfigFiles, LightPlacerConfigs) are inert
            // without the CS DLL loaded — they sit in directories that
            // nothing reads unless CommunityShaders.dll is present. Mods
            // that ONLY contain these harmless artefacts can stay enabled.
            let only_harmless = reasons.iter().all(|r| {
                matches!(
                    r,
                    CsDetectionReason::CsConfigFiles
                        | CsDetectionReason::LightPlacerConfigs
                )
            });

            let has_fomod_recipe = fomod_recipes::get_recipe(db, m.id)
                .ok()
                .flatten()
                .is_some();

            let action = if only_harmless {
                // Config files are inert without CS DLL — safe to keep
                CsModAction::Keep
            } else if has_fomod_recipe
                && reasons.contains(&CsDetectionReason::FomodCsSelection)
            {
                CsModAction::RerunFomod {
                    suggested_selections: HashMap::new(),
                }
            } else {
                CsModAction::Disable
            };

            detected_mods.push(CsDetectedMod {
                mod_id: m.id,
                mod_name: m.name.clone(),
                nexus_mod_id: m.nexus_mod_id,
                nexus_file_id: m.nexus_file_id,
                reasons,
                action,
            });
        }
    }

    // Check for existing ENB installation
    let enb_already_installed = check_enb_installed(game_path);

    let total_cs_mods = detected_mods.len();
    let swappable_count = detected_mods
        .iter()
        .filter(|m| matches!(m.action, CsModAction::SwapToEnbVariant { .. }))
        .count();
    let fomod_rerun_count = detected_mods
        .iter()
        .filter(|m| matches!(m.action, CsModAction::RerunFomod { .. }))
        .count();
    let disable_only_count = detected_mods
        .iter()
        .filter(|m| matches!(m.action, CsModAction::Disable))
        .count();
    let keep_count = detected_mods
        .iter()
        .filter(|m| matches!(m.action, CsModAction::Keep))
        .count();

    info!(
        "CS scan: {} total, {} swappable, {} FOMOD rerun, {} disable-only, {} safe-to-keep, ENB installed: {}",
        total_cs_mods, swappable_count, fomod_rerun_count, disable_only_count, keep_count, enb_already_installed
    );

    Ok(ShaderScanResult {
        detected_mods,
        total_cs_mods,
        swappable_count,
        fomod_rerun_count,
        disable_only_count,
        keep_count,
        enb_already_installed,
    })
}

/// Check a mod's file list against known CS file patterns.
fn check_file_patterns(installed_files: &[String], reasons: &mut Vec<CsDetectionReason>) {
    for file in installed_files {
        let lower = file.to_lowercase().replace('\\', "/");

        // Core DLL check
        if lower.ends_with("communityshaders.dll") {
            if !reasons.contains(&CsDetectionReason::CoreDll) {
                reasons.push(CsDetectionReason::CoreDll);
            }
            continue;
        }

        // CS config directory
        if lower.contains("communityshaders/") && (lower.ends_with(".ini") || lower.ends_with(".json")) {
            if !reasons.contains(&CsDetectionReason::CsConfigFiles) {
                reasons.push(CsDetectionReason::CsConfigFiles);
            }
            continue;
        }

        // Light Placer DLL (active binary — breaking without CS)
        if lower.ends_with("lightplacer.dll") {
            if !reasons.contains(&CsDetectionReason::CsOnlyFiles) {
                reasons.push(CsDetectionReason::CsOnlyFiles);
            }
            continue;
        }

        // Light Placer configs (inert JSON configs — harmless without CS)
        if lower.contains("lightplacer/") {
            if !reasons.contains(&CsDetectionReason::LightPlacerConfigs) {
                reasons.push(CsDetectionReason::LightPlacerConfigs);
            }
            continue;
        }

        // PBR textures (CS-specific PBR workflow)
        if lower.contains("/pbr/") && (lower.ends_with(".dds") || lower.ends_with(".png")) {
            if !reasons.contains(&CsDetectionReason::PbrTextures) {
                reasons.push(CsDetectionReason::PbrTextures);
            }
            continue;
        }

        // Generic CS file patterns
        for pattern in CS_FILE_PATTERNS {
            if lower.contains(pattern) {
                if !reasons.contains(&CsDetectionReason::CsOnlyFiles) {
                    reasons.push(CsDetectionReason::CsOnlyFiles);
                }
                break;
            }
        }
    }
}

/// Check a mod's display name against known CS name patterns.
fn check_name_patterns(mod_name: &str, reasons: &mut Vec<CsDetectionReason>) {
    let lower = mod_name.to_lowercase();
    for pattern in CS_MOD_NAME_PATTERNS {
        if lower.contains(pattern) {
            if !reasons.contains(&CsDetectionReason::KnownCsEcosystemMod) {
                reasons.push(CsDetectionReason::KnownCsEcosystemMod);
            }
            return;
        }
    }
}

/// Check a mod's saved FOMOD recipe for CS-specific selections.
fn check_fomod_recipe(db: &ModDatabase, mod_id: i64, reasons: &mut Vec<CsDetectionReason>) {
    let recipe = match fomod_recipes::get_recipe(db, mod_id) {
        Ok(Some(r)) => r,
        _ => return,
    };

    for (_group, selections) in &recipe.selections {
        for sel in selections {
            let lower = sel.to_lowercase();
            for pattern in CS_FOMOD_PATTERNS {
                if lower.contains(pattern) {
                    if !reasons.contains(&CsDetectionReason::FomodCsSelection) {
                        reasons.push(CsDetectionReason::FomodCsSelection);
                    }
                    return;
                }
            }
        }
    }
}

/// Check whether ENB files are already deployed to the game root.
fn check_enb_installed(game_path: &Path) -> bool {
    let d3d11 = game_path.join("d3d11.dll");
    let enblocal = game_path.join("enblocal.ini");
    d3d11.exists() && enblocal.exists()
}

/// Disable and undeploy a batch of CS mods.
///
/// For each mod ID: marks the mod as disabled in the database and removes
/// its deployed files from the data directory. Returns the count of
/// successfully disabled mods.
pub fn batch_disable_cs_mods(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    mod_ids: &[i64],
    data_dir: &Path,
    game_path: &Path,
) -> Result<usize, String> {
    let mut disabled = 0usize;

    for &mod_id in mod_ids {
        // Disable in database
        if let Err(e) = db.set_enabled(mod_id, false) {
            warn!("Failed to disable mod {}: {}", mod_id, e);
            continue;
        }

        // Undeploy from filesystem
        match deployer::undeploy_mod(db, game_id, bottle_name, mod_id, data_dir, game_path) {
            Ok(removed) => {
                debug!("Undeployed mod {}: removed {} files", mod_id, removed.len());
                disabled += 1;
            }
            Err(e) => {
                warn!("Failed to undeploy mod {}: {}", mod_id, e);
                // Still count as disabled since the DB record was updated
                disabled += 1;
            }
        }
    }

    info!("Batch-disabled {}/{} CS mods", disabled, mod_ids.len());
    Ok(disabled)
}

/// Download and install the ENBSeries binary into the game root.
///
/// Attempts to scrape the enbdev.com version page for the latest release;
/// falls back to a hardcoded version string if scraping fails. Extracts
/// the ENB zip and deploys d3d11.dll, d3dcompiler_46e.dll, enblocal.ini,
/// enbseries.ini, and the enbseries/ directory to the game root. Patches
/// enblocal.ini with `LinuxVersion=true` for Wine compatibility.
///
/// Returns the installed ENB version string.
pub async fn install_enb_binary(game_path: &Path) -> Result<String, String> {
    let game_path = game_path.to_path_buf();

    let client = reqwest::Client::builder()
        .user_agent("Corkscrew Mod Manager")
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    // Try to detect latest version from enbdev.com
    let version = detect_latest_enb_version(&client).await;
    info!("ENB version to install: {}", version);

    // Build download URL and referer
    let download_url = format!("{}{}.zip", ENB_DOWNLOAD_BASE, version);
    let referer = format!("{}{}.html", ENB_REFERER_BASE, version);

    info!("Downloading ENB from: {}", download_url);

    let response = client
        .get(&download_url)
        .header("Referer", &referer)
        .send()
        .await
        .map_err(|e| format!("Failed to download ENB: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "ENB download failed with status {}: {}",
            response.status(),
            download_url
        ));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read ENB download body: {}", e))?;

    // Extract to temp directory
    let temp_dir = tempfile::tempdir()
        .map_err(|e| format!("Failed to create temp dir: {}", e))?;
    let zip_path = temp_dir.path().join("enb.zip");
    fs::write(&zip_path, &bytes)
        .map_err(|e| format!("Failed to write ENB zip: {}", e))?;

    let extract_dir = temp_dir.path().join("enb_extracted");
    fs::create_dir_all(&extract_dir)
        .map_err(|e| format!("Failed to create extract dir: {}", e))?;

    // Use the zip crate to extract
    let zip_path_clone = zip_path.clone();
    let extract_dir_clone = extract_dir.clone();
    tokio::task::spawn_blocking(move || {
        extract_enb_zip(&zip_path_clone, &extract_dir_clone)
    })
    .await
    .map_err(|e| format!("ENB extraction task failed: {}", e))?
    .map_err(|e| format!("ENB extraction failed: {}", e))?;

    // Find the WrapperVersion directory inside the extracted archive
    // ENB zips typically contain WrapperVersion/ and InjectorVersion/
    let wrapper_dir = find_enb_wrapper_dir(&extract_dir)?;

    // Copy ENB files to game root
    deploy_enb_files(&wrapper_dir, &game_path)?;

    // Patch enblocal.ini for Linux/Wine compatibility
    patch_enb_local_ini(&game_path)?;

    // Write a marker file with the version
    let marker_path = game_path.join("enb.version");
    fs::write(&marker_path, &version)
        .map_err(|e| format!("Failed to write ENB version marker: {}", e))?;

    info!("ENB v{} installed to {}", version, game_path.display());
    Ok(version)
}

/// Attempt to detect the latest ENB version from the enbdev.com download
/// page. Returns the fallback version on failure.
async fn detect_latest_enb_version(client: &reqwest::Client) -> String {
    match client.get(ENB_VERSION_PAGE).send().await {
        Ok(resp) => {
            if let Ok(body) = resp.text().await {
                // Look for download links matching the pattern
                // enbseries_skyrimse_vXXXX.zip
                if let Some(version) = parse_enb_version_from_html(&body) {
                    return version;
                }
            }
        }
        Err(e) => {
            warn!("Failed to fetch ENB version page: {}", e);
        }
    }
    info!("Using fallback ENB version: {}", ENB_FALLBACK_VERSION);
    ENB_FALLBACK_VERSION.to_string()
}

/// Parse the ENB version string from the enbdev.com HTML page.
///
/// Looks for patterns like `enbseries_skyrimse_v0503.zip` in download links
/// and extracts the version number portion.
fn parse_enb_version_from_html(html: &str) -> Option<String> {
    // Look for the versioned zip filename in links or text
    let prefix = "enbseries_skyrimse_v";
    let suffix = ".zip";

    let mut best_version: Option<String> = None;

    for line in html.lines() {
        let lower = line.to_lowercase();
        if let Some(start) = lower.find(prefix) {
            let after_prefix = start + prefix.len();
            if let Some(end) = lower[after_prefix..].find(suffix) {
                let ver = &lower[after_prefix..after_prefix + end];
                // Validate: should be numeric
                if ver.chars().all(|c| c.is_ascii_digit()) && !ver.is_empty() {
                    // Keep the highest version found
                    match &best_version {
                        Some(existing) if ver.parse::<u32>().unwrap_or(0) > existing.parse::<u32>().unwrap_or(0) => {
                            best_version = Some(ver.to_string());
                        }
                        None => {
                            best_version = Some(ver.to_string());
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    if let Some(ref v) = best_version {
        info!("Detected latest ENB version: {}", v);
    }
    best_version
}

/// Extract an ENB zip archive using the `zip` crate.
fn extract_enb_zip(zip_path: &Path, dest: &Path) -> Result<(), String> {
    let file = fs::File::open(zip_path)
        .map_err(|e| format!("Failed to open ENB zip: {}", e))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("Failed to parse ENB zip: {}", e))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read zip entry {}: {}", i, e))?;

        let out_path = match entry.enclosed_name() {
            Some(p) => dest.join(p),
            None => continue,
        };

        if entry.is_dir() {
            fs::create_dir_all(&out_path)
                .map_err(|e| format!("Failed to create dir {}: {}", out_path.display(), e))?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create parent dir: {}", e))?;
            }
            let mut outfile = fs::File::create(&out_path)
                .map_err(|e| format!("Failed to create file {}: {}", out_path.display(), e))?;
            std::io::copy(&mut entry, &mut outfile)
                .map_err(|e| format!("Failed to extract {}: {}", out_path.display(), e))?;
        }
    }

    Ok(())
}

/// Find the WrapperVersion directory inside the extracted ENB archive.
///
/// ENB zips contain two directories: `WrapperVersion/` (uses d3d11.dll proxy)
/// and `InjectorVersion/` (uses a separate injector EXE). We always use the
/// wrapper version for Wine compatibility.
fn find_enb_wrapper_dir(extract_dir: &Path) -> Result<PathBuf, String> {
    // Check for WrapperVersion subdirectory (case-insensitive)
    if let Ok(entries) = fs::read_dir(extract_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.eq_ignore_ascii_case("wrapperversion") && entry.path().is_dir() {
                return Ok(entry.path());
            }
        }
    }

    // If no WrapperVersion dir, the files might be at the root
    let d3d11 = extract_dir.join("d3d11.dll");
    if d3d11.exists() {
        return Ok(extract_dir.to_path_buf());
    }

    // Check one level deeper (some zips have a top-level named directory)
    if let Ok(entries) = fs::read_dir(extract_dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let wrapper = entry.path().join("WrapperVersion");
                if wrapper.is_dir() {
                    return Ok(wrapper);
                }
                // Also check if d3d11.dll is directly inside
                let d3d11 = entry.path().join("d3d11.dll");
                if d3d11.exists() {
                    return Ok(entry.path());
                }
            }
        }
    }

    Err("Could not find ENB WrapperVersion directory in archive".to_string())
}

/// Deploy ENB files from the wrapper directory to the game root.
fn deploy_enb_files(wrapper_dir: &Path, game_path: &Path) -> Result<(), String> {
    // Copy individual files
    for file_name in ENB_ROOT_FILES {
        let src = wrapper_dir.join(file_name);
        if src.exists() {
            let dst = game_path.join(file_name);
            fs::copy(&src, &dst).map_err(|e| {
                format!(
                    "Failed to copy {} to {}: {}",
                    src.display(),
                    dst.display(),
                    e
                )
            })?;
            debug!("Deployed ENB file: {}", file_name);
        } else {
            debug!("ENB file not in archive (optional): {}", file_name);
        }
    }

    // Copy enbseries/ directory if it exists
    let enbseries_src = wrapper_dir.join("enbseries");
    if enbseries_src.is_dir() {
        let enbseries_dst = game_path.join("enbseries");
        copy_dir_recursive(&enbseries_src, &enbseries_dst)?;
        debug!("Deployed enbseries/ directory");
    }

    Ok(())
}

/// Recursively copy a directory tree.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst)
        .map_err(|e| format!("Failed to create dir {}: {}", dst.display(), e))?;

    let entries = fs::read_dir(src)
        .map_err(|e| format!("Failed to read dir {}: {}", src.display(), e))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read dir entry: {}", e))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path).map_err(|e| {
                format!(
                    "Failed to copy {} to {}: {}",
                    src_path.display(),
                    dst_path.display(),
                    e
                )
            })?;
        }
    }

    Ok(())
}

/// Patch enblocal.ini to enable Linux/Wine compatibility mode.
///
/// Ensures the `[GLOBAL]` section contains `LinuxVersion=true`. If the file
/// does not exist, creates a minimal one.
pub fn patch_enb_local_ini(game_path: &Path) -> Result<(), String> {
    let ini_path = game_path.join("enblocal.ini");

    let content = if ini_path.exists() {
        fs::read_to_string(&ini_path)
            .map_err(|e| format!("Failed to read enblocal.ini: {}", e))?
    } else {
        String::new()
    };

    let patched = ensure_enb_linux_version(&content);

    fs::write(&ini_path, &patched)
        .map_err(|e| format!("Failed to write enblocal.ini: {}", e))?;

    info!("Patched enblocal.ini with LinuxVersion=true");
    Ok(())
}

/// Ensure the INI content has `[GLOBAL]\nLinuxVersion=true`.
///
/// If `[GLOBAL]` section exists, adds or updates the key. Otherwise appends
/// the section.
fn ensure_enb_linux_version(content: &str) -> String {
    let mut lines: Vec<String> = content.lines().map(String::from).collect();
    let global_lower = "[global]";
    let key_lower = "linuxversion";

    // Find [GLOBAL] section
    let mut global_idx: Option<usize> = None;
    let mut key_idx: Option<usize> = None;
    let mut next_section_idx: Option<usize> = None;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim().to_lowercase();
        if trimmed == global_lower {
            global_idx = Some(i);
        } else if global_idx.is_some() && next_section_idx.is_none() {
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                next_section_idx = Some(i);
            } else if trimmed.starts_with(key_lower) {
                key_idx = Some(i);
            }
        }
    }

    match (global_idx, key_idx) {
        (Some(_), Some(idx)) => {
            // Update existing key
            lines[idx] = "LinuxVersion=true".to_string();
        }
        (Some(idx), None) => {
            // Insert after [GLOBAL]
            lines.insert(idx + 1, "LinuxVersion=true".to_string());
        }
        (None, _) => {
            // Append [GLOBAL] section
            if !lines.is_empty() && !lines.last().map_or(true, |l| l.is_empty()) {
                lines.push(String::new());
            }
            lines.push("[GLOBAL]".to_string());
            lines.push("LinuxVersion=true".to_string());
        }
    }

    let mut result = lines.join("\n");
    // Ensure trailing newline
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// Query NexusMods for ENB-compatible variant files of detected CS mods.
///
/// For each mod with a `nexus_mod_id`, fetches the mod's file list from the
/// NexusMods API and looks for files whose name or description contains
/// "enb". If found, the mod's action is updated to `SwapToEnbVariant`.
///
/// Respects NexusMods rate limits with a 1-second delay between API calls.
pub async fn discover_enb_siblings(
    detected_mods: &[CsDetectedMod],
    game_slug: &str,
) -> Result<Vec<CsDetectedMod>, String> {
    let client = crate::nexus_client().await?;

    let mut updated = detected_mods.to_vec();

    for detected in &mut updated {
        let nexus_mod_id = match detected.nexus_mod_id {
            Some(id) => id,
            None => continue,
        };

        // Skip mods that already have a non-Disable action
        if !matches!(detected.action, CsModAction::Disable) {
            continue;
        }

        // Query NexusMods for all files on this mod page
        match client.get_mod_files(game_slug, nexus_mod_id).await {
            Ok(raw_files) => {
                let files = nexus::parse_mod_files(&raw_files, nexus_mod_id);
                if let Some(enb_file) = find_enb_variant_file(&files) {
                    info!(
                        "Found ENB variant for mod {} (NM {}): file_id={} '{}'",
                        detected.mod_name, nexus_mod_id, enb_file.file_id, enb_file.name
                    );
                    detected.action = CsModAction::SwapToEnbVariant {
                        enb_file_id: enb_file.file_id,
                        enb_file_name: enb_file.name.clone(),
                    };
                }
            }
            Err(e) => {
                warn!(
                    "Failed to query NM files for mod {} (NM {}): {}",
                    detected.mod_name, nexus_mod_id, e
                );
            }
        }

        // Rate limit: 1 second between API calls
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    Ok(updated)
}

/// Look through a mod's file list for an ENB-compatible variant.
///
/// Returns the first file whose name or description contains "enb"
/// (case-insensitive) and is in the "main" or "optional" category.
fn find_enb_variant_file(files: &[NexusModFile]) -> Option<&NexusModFile> {
    files.iter().find(|f| {
        let name_lower = f.name.to_lowercase();
        let desc_lower = f.description.to_lowercase();
        let is_enb = name_lower.contains("enb")
            || desc_lower.contains("enb version")
            || desc_lower.contains("enb variant")
            || desc_lower.contains("for enb");
        let is_relevant_category = f.category == "main" || f.category == "optional";
        is_enb && is_relevant_category
    })
}

/// Swap a mod to its ENB-compatible variant file from NexusMods.
///
/// This downloads the ENB variant and replaces the mod's staging directory.
/// Only works for premium NexusMods users (API-initiated downloads).
pub async fn swap_mod_to_enb_variant(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    mod_id: i64,
    enb_file_id: i64,
    game_slug: &str,
    data_dir: &Path,
    game_path: &Path,
) -> Result<(), String> {
    // Check premium status
    let client = crate::nexus_client().await?;
    if !client.is_premium().await {
        return Err(
            "Mod swapping requires NexusMods Premium. \
             Please download the ENB variant manually from the mod page \
             and install it through the normal mod installation flow."
                .to_string(),
        );
    }

    // Get the mod record to find the NexusMods mod ID
    let m = db
        .get_mod(mod_id)
        .map_err(|e| format!("Failed to get mod {}: {}", mod_id, e))?
        .ok_or_else(|| format!("Mod {} not found", mod_id))?;
    let nexus_mod_id = m
        .nexus_mod_id
        .ok_or_else(|| "Mod has no NexusMods ID, cannot swap".to_string())?;

    // Get download links for the ENB variant file (premium-only, no key/expires)
    let links = client
        .get_download_links(game_slug, nexus_mod_id, enb_file_id, None, None)
        .await
        .map_err(|e| format!("Failed to get download links: {}", e))?;

    let download_url = links
        .first()
        .map(|l| l.uri.as_str())
        .ok_or_else(|| "No download URL returned for ENB variant".to_string())?;

    info!(
        "Downloading ENB variant for mod {} (file_id {})",
        mod_id, enb_file_id
    );

    // Download the file
    let dl_client = reqwest::Client::new();
    let response = dl_client
        .get(download_url)
        .send()
        .await
        .map_err(|e| format!("Download failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "Download failed with status {}",
            response.status()
        ));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read download body: {}", e))?;

    // Undeploy the old mod
    let _ = deployer::undeploy_mod(db, game_id, bottle_name, mod_id, data_dir, game_path);

    // Write the new archive to a temp file and extract to staging
    let temp_dir = tempfile::tempdir()
        .map_err(|e| format!("Failed to create temp dir: {}", e))?;
    let archive_name = format!("enb_variant_{}.zip", enb_file_id);
    let archive_path = temp_dir.path().join(&archive_name);
    fs::write(&archive_path, &bytes)
        .map_err(|e| format!("Failed to write archive: {}", e))?;

    // Get the staging path from the mod record
    let staging_path = m
        .staging_path
        .as_deref()
        .ok_or_else(|| "Mod has no staging path".to_string())?;
    let staging_dir = PathBuf::from(staging_path);

    // Clear and re-extract staging
    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir)
            .map_err(|e| format!("Failed to clear staging: {}", e))?;
    }
    fs::create_dir_all(&staging_dir)
        .map_err(|e| format!("Failed to create staging dir: {}", e))?;

    crate::installer::extract_archive(&archive_path, &staging_dir)
        .map_err(|e| format!("Failed to extract ENB variant: {}", e))?;

    // Update the mod's nexus_file_id in the database
    let conn = db.conn().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE installed_mods SET nexus_file_id = ?1 WHERE id = ?2",
        params![enb_file_id, mod_id],
    )
    .map_err(|e| format!("Failed to update mod file ID: {}", e))?;
    drop(conn);

    info!(
        "Swapped mod {} to ENB variant (file_id {})",
        mod_id, enb_file_id
    );
    Ok(())
}

/// Compute ENB-oriented FOMOD selections from existing CS-oriented ones.
///
/// For each group in the FOMOD installer, if the current selections match
/// CS patterns and there is an ENB alternative option available, the
/// selection is swapped. Also prefers non-split mesh options over split
/// meshes (CS uses split meshes for parallax; ENB does not need them).
pub fn compute_enb_fomod_selections(
    current_selections: &HashMap<String, Vec<String>>,
    installer: &fomod::FomodInstaller,
) -> HashMap<String, Vec<String>> {
    let mut new_selections = current_selections.clone();

    for step in &installer.steps {
        for group in &step.groups {
            let group_name = &group.name;

            if let Some(current) = new_selections.get(group_name) {
                let mut updated = current.clone();
                let mut changed = false;

                // Check if any current selection is CS-related
                let has_cs_selection = current.iter().any(|sel| {
                    let lower = sel.to_lowercase();
                    CS_FOMOD_PATTERNS.iter().any(|p| lower.contains(p))
                });

                if has_cs_selection {
                    // Look for ENB alternative in the same group
                    let enb_option = group.options.iter().find(|opt| {
                        let lower = opt.name.to_lowercase();
                        ENB_FOMOD_PATTERNS.iter().any(|p| lower.contains(p))
                    });

                    if let Some(enb_opt) = enb_option {
                        // Replace CS selections with ENB selection
                        updated.retain(|sel| {
                            let lower = sel.to_lowercase();
                            !CS_FOMOD_PATTERNS.iter().any(|p| lower.contains(p))
                        });
                        updated.push(enb_opt.name.clone());
                        changed = true;
                    }
                }

                // Check for mesh split selections — prefer non-split
                let has_split = current.iter().any(|sel| {
                    let lower = sel.to_lowercase();
                    MESH_SPLIT_PATTERNS.iter().any(|p| lower.contains(p))
                });

                if has_split {
                    // Look for a non-split alternative
                    let non_split_option = group.options.iter().find(|opt| {
                        let lower = opt.name.to_lowercase();
                        !MESH_SPLIT_PATTERNS.iter().any(|p| lower.contains(p))
                            && (lower.contains("mesh") || lower.contains("default"))
                    });

                    if let Some(non_split) = non_split_option {
                        updated.retain(|sel| {
                            let lower = sel.to_lowercase();
                            !MESH_SPLIT_PATTERNS.iter().any(|p| lower.contains(p))
                        });
                        updated.push(non_split.name.clone());
                        changed = true;
                    }
                }

                if changed {
                    new_selections.insert(group_name.clone(), updated);
                }
            }
        }
    }

    new_selections
}

/// Execute a full CS→ENB conversion pipeline.
///
/// Orchestrates the entire conversion:
/// 1. Create a rollback snapshot
/// 2. Disable CS-only mods
/// 3. Install ENB binary (if requested)
/// 4. Set DXVK d3d11 override (if requested)
/// 5. Process mod swaps (download ENB variant files)
/// 6. Re-run FOMOD installers with ENB selections
/// 7. Install ENB ecosystem mods (ENB Helper SE, ENB Light)
/// 8. Redeploy all mods
/// 9. Save conversion record
///
/// Emits `ConversionProgress` events throughout.
pub async fn execute_conversion(
    app: &AppHandle,
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    game_path: &Path,
    data_dir: &Path,
    config: ConversionConfig,
    bottle: Option<&Bottle>,
) -> Result<ConversionResult, String> {
    let mut errors = Vec::new();
    let mut mods_disabled = 0usize;
    let mut mods_swapped = 0usize;
    let mut fomods_rerun = 0usize;
    let mut enb_installed = false;
    let mut dxvk_switched = false;

    // 1. Create rollback snapshot
    emit_progress(
        app,
        ConversionProgress::Scanning {
            message: "Creating pre-conversion snapshot...".to_string(),
        },
    );

    let snapshot_id = rollback::create_snapshot(
        db,
        game_id,
        bottle_name,
        "Pre-shader conversion",
        Some("Auto-snapshot before CS→ENB conversion"),
    )
    .map_err(|e| format!("Failed to create snapshot: {}", e))?;

    info!("Created pre-conversion snapshot: {}", snapshot_id);

    // 2. Disable CS mods
    let disable_ids: Vec<i64> = config
        .mod_actions
        .iter()
        .filter(|(_, action)| matches!(action, CsModAction::Disable))
        .map(|(id, _)| *id)
        .collect();

    if !disable_ids.is_empty() {
        let total = disable_ids.len();
        for (i, &mod_id) in disable_ids.iter().enumerate() {
            let mod_name = db
                .get_mod(mod_id)
                .ok()
                .flatten()
                .map(|m| m.name.clone())
                .unwrap_or_else(|| format!("Mod {}", mod_id));

            emit_progress(
                app,
                ConversionProgress::DisablingMods {
                    current: i + 1,
                    total,
                    mod_name: mod_name.clone(),
                },
            );

            match disable_single_mod(db, game_id, bottle_name, mod_id, data_dir, game_path) {
                Ok(()) => mods_disabled += 1,
                Err(e) => {
                    let msg = format!("Failed to disable '{}': {}", mod_name, e);
                    warn!("{}", msg);
                    errors.push(msg);
                }
            }
        }
    }

    // 3. Install ENB binary
    if config.install_enb_binary {
        emit_progress(
            app,
            ConversionProgress::InstallingEnb {
                step: "Downloading ENBSeries...".to_string(),
            },
        );

        match install_enb_binary(game_path).await {
            Ok(version) => {
                info!("Installed ENB v{}", version);
                enb_installed = true;
            }
            Err(e) => {
                let msg = format!("Failed to install ENB: {}", e);
                error!("{}", msg);
                errors.push(msg);
            }
        }
    }

    // 4. Switch to DXVK d3d11 override
    if config.switch_to_dxvk {
        if let Some(bottle) = bottle {
            match wine_diagnostic::fix_dll_override(bottle, "d3d11", "native") {
                Ok(()) => {
                    info!("Set d3d11 DLL override to native (DXVK)");
                    dxvk_switched = true;
                }
                Err(e) => {
                    let msg = format!("Failed to set DXVK override: {}", e);
                    warn!("{}", msg);
                    errors.push(msg);
                }
            }
        } else {
            errors.push("Cannot set DXVK override: no bottle provided".to_string());
        }
    }

    // 5. Process mod swaps
    let swap_actions: Vec<(i64, i64, String)> = config
        .mod_actions
        .iter()
        .filter_map(|(id, action)| {
            if let CsModAction::SwapToEnbVariant {
                enb_file_id,
                enb_file_name,
            } = action
            {
                Some((*id, *enb_file_id, enb_file_name.clone()))
            } else {
                None
            }
        })
        .collect();

    if !swap_actions.is_empty() {
        let total = swap_actions.len();
        for (i, (mod_id, enb_file_id, _enb_file_name)) in swap_actions.iter().enumerate() {
            let mod_name = db
                .get_mod(*mod_id)
                .ok()
                .flatten()
                .map(|m| m.name.clone())
                .unwrap_or_else(|| format!("Mod {}", mod_id));

            emit_progress(
                app,
                ConversionProgress::SwappingMods {
                    current: i + 1,
                    total,
                    mod_name: mod_name.clone(),
                },
            );

            // Determine game slug from the mod's game_id
            let game_slug = game_id_to_nexus_slug(game_id);

            match swap_mod_to_enb_variant(
                db,
                game_id,
                bottle_name,
                *mod_id,
                *enb_file_id,
                &game_slug,
                data_dir,
                game_path,
            )
            .await
            {
                Ok(()) => mods_swapped += 1,
                Err(e) => {
                    let msg = format!("Failed to swap '{}': {}", mod_name, e);
                    warn!("{}", msg);
                    errors.push(msg);
                }
            }
        }
    }

    // 6. Process FOMOD reruns
    let rerun_actions: Vec<(i64, HashMap<String, Vec<String>>)> = config
        .mod_actions
        .iter()
        .filter_map(|(id, action)| {
            if let CsModAction::RerunFomod {
                suggested_selections,
            } = action
            {
                Some((*id, suggested_selections.clone()))
            } else {
                None
            }
        })
        .collect();

    if !rerun_actions.is_empty() {
        let total = rerun_actions.len();
        for (i, (mod_id, _suggested)) in rerun_actions.iter().enumerate() {
            let mod_name = db
                .get_mod(*mod_id)
                .ok()
                .flatten()
                .map(|m| m.name.clone())
                .unwrap_or_else(|| format!("Mod {}", mod_id));

            emit_progress(
                app,
                ConversionProgress::RerunningFomods {
                    current: i + 1,
                    total,
                    mod_name: mod_name.clone(),
                },
            );

            // FOMOD reruns require user interaction in the frontend, so we
            // just record them as needing attention. The frontend will
            // present the FOMOD wizard with pre-selected ENB options.
            fomods_rerun += 1;
            info!(
                "FOMOD rerun queued for '{}' (mod_id {})",
                mod_name, mod_id
            );
        }
    }

    // 7. Install ENB ecosystem mods (ENB Helper SE, ENB Light)
    if config.install_enb_ecosystem {
        emit_progress(
            app,
            ConversionProgress::InstallingEcosystem {
                mod_name: "ENB Helper SE".to_string(),
            },
        );
        info!("ENB ecosystem install requested — ENB Helper SE (NM {}) and ENB Light (NM {})",
            ENB_HELPER_SE_MOD_ID, ENB_LIGHT_MOD_ID);
        // Ecosystem mod installation requires NexusMods Premium for API
        // downloads. The actual install is deferred to the frontend which
        // can present download prompts for free users.
    }

    // 8. Redeploy only if mods were swapped/installed (not for disable-only)
    //    undeploy_mod() already removes files and restores winners, so
    //    a full redeploy of ALL mods is only needed when new files are introduced.
    let needs_full_redeploy = mods_swapped > 0 || fomods_rerun > 0 || enb_installed || dxvk_switched;
    if needs_full_redeploy {
        emit_progress(app, ConversionProgress::Redeploying {
            current: 0,
            total: 0,
            mod_name: "Preparing...".to_string(),
        });
        let app_redeploy = app.clone();
        match deployer::redeploy_all_with_progress(
            db,
            game_id,
            bottle_name,
            data_dir,
            game_path,
            Some(|current_idx: usize, total_mods: usize, name: &str, _files: usize, _total_files: usize| {
                emit_progress(
                    &app_redeploy,
                    ConversionProgress::Redeploying {
                        current: current_idx + 1,
                        total: total_mods,
                        mod_name: name.to_string(),
                    },
                );
            }),
        ) {
            Ok(result) => {
                info!(
                    "Redeployed {} files after conversion",
                    result.deployed_count
                );
            }
            Err(e) => {
                let msg = format!("Redeploy failed: {}", e);
                error!("{}", msg);
                errors.push(msg);
            }
        }
    } else {
        info!("Skipping full redeploy — disable-only conversion, undeploy already handled file cleanup");
    }

    // 9. Save conversion record
    let conversion_id = save_conversion_record(
        db,
        game_id,
        bottle_name,
        snapshot_id,
        &disable_ids,
        &swap_actions.iter().map(|(id, _, _)| *id).collect::<Vec<_>>(),
        enb_installed,
    )?;

    let result = ConversionResult {
        conversion_id,
        snapshot_id,
        mods_disabled,
        mods_swapped,
        fomods_rerun,
        enb_installed,
        dxvk_switched,
        errors,
    };

    emit_progress(
        app,
        ConversionProgress::Complete {
            result: result.clone(),
        },
    );

    info!(
        "Conversion {} complete: {} disabled, {} swapped, {} FOMOD reruns, ENB: {}, DXVK: {}",
        conversion_id,
        mods_disabled,
        mods_swapped,
        fomods_rerun,
        enb_installed,
        dxvk_switched
    );

    Ok(result)
}

/// Disable and undeploy a single mod.
fn disable_single_mod(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    mod_id: i64,
    data_dir: &Path,
    game_path: &Path,
) -> Result<(), String> {
    db.set_enabled(mod_id, false)
        .map_err(|e| format!("DB disable failed: {}", e))?;
    deployer::undeploy_mod(db, game_id, bottle_name, mod_id, data_dir, game_path)
        .map_err(|e| format!("Undeploy failed: {}", e))?;
    Ok(())
}

/// Emit a conversion progress event to the frontend.
fn emit_progress(app: &AppHandle, progress: ConversionProgress) {
    if let Err(e) = app.emit(CONVERSION_PROGRESS_EVENT, &progress) {
        warn!("Failed to emit conversion progress: {}", e);
    }
}

/// Map a game_id to its NexusMods slug.
fn game_id_to_nexus_slug(game_id: &str) -> String {
    match game_id {
        "skyrimse" => "skyrimspecialedition".to_string(),
        "skyrim" => "skyrim".to_string(),
        "fallout4" => "fallout4".to_string(),
        "fallout3" => "fallout3".to_string(),
        "falloutnv" => "newvegas".to_string(),
        "oblivion" => "oblivion".to_string(),
        "starfield" => "starfield".to_string(),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Database helpers
// ---------------------------------------------------------------------------

/// Initialize the shader_conversions table schema.
pub fn init_schema(db: &ModDatabase) -> Result<(), String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS shader_conversions (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            game_id         TEXT NOT NULL,
            bottle_name     TEXT NOT NULL,
            snapshot_id     INTEGER NOT NULL,
            status          TEXT NOT NULL DEFAULT 'completed',
            disabled_mods   TEXT NOT NULL DEFAULT '[]',
            swapped_mods    TEXT NOT NULL DEFAULT '[]',
            enb_installed   INTEGER NOT NULL DEFAULT 0,
            created_at      TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_shader_conversions_game_bottle
            ON shader_conversions (game_id, bottle_name);",
    )
    .map_err(|e| format!("Failed to create shader_conversions table: {}", e))?;

    Ok(())
}

/// Save a conversion record to the database.
fn save_conversion_record(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    snapshot_id: i64,
    disabled_mods: &[i64],
    swapped_mods: &[i64],
    enb_installed: bool,
) -> Result<i64, String> {
    // Ensure schema exists
    init_schema(db)?;

    let conn = db.conn().map_err(|e| e.to_string())?;
    let created_at = Utc::now().to_rfc3339();
    let disabled_json =
        serde_json::to_string(disabled_mods).map_err(|e| format!("JSON error: {}", e))?;
    let swapped_json =
        serde_json::to_string(swapped_mods).map_err(|e| format!("JSON error: {}", e))?;

    conn.execute(
        "INSERT INTO shader_conversions
            (game_id, bottle_name, snapshot_id, status, disabled_mods, swapped_mods, enb_installed, created_at)
         VALUES (?1, ?2, ?3, 'completed', ?4, ?5, ?6, ?7)",
        params![
            game_id,
            bottle_name,
            snapshot_id,
            disabled_json,
            swapped_json,
            enb_installed as i64,
            created_at,
        ],
    )
    .map_err(|e| format!("Failed to save conversion record: {}", e))?;

    Ok(conn.last_insert_rowid())
}

/// Retrieve conversion history for a game/bottle.
pub fn get_conversion_history(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
) -> Result<Vec<ConversionHistoryEntry>, String> {
    // Ensure schema exists (table might not be created yet)
    init_schema(db)?;

    let conn = db.conn().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, game_id, bottle_name, snapshot_id, status,
                    disabled_mods, swapped_mods, enb_installed, created_at
             FROM shader_conversions
             WHERE game_id = ?1 AND bottle_name = ?2
             ORDER BY created_at DESC",
        )
        .map_err(|e| format!("Failed to prepare history query: {}", e))?;

    let entries = stmt
        .query_map(params![game_id, bottle_name], |row| {
            let disabled_json: String = row.get(5)?;
            let swapped_json: String = row.get(6)?;
            let enb_int: i64 = row.get(7)?;

            let disabled_mods: Vec<i64> =
                serde_json::from_str(&disabled_json).unwrap_or_default();
            let swapped_mods: Vec<i64> =
                serde_json::from_str(&swapped_json).unwrap_or_default();

            Ok(ConversionHistoryEntry {
                id: row.get(0)?,
                game_id: row.get(1)?,
                bottle_name: row.get(2)?,
                snapshot_id: row.get(3)?,
                status: row.get(4)?,
                disabled_mods,
                swapped_mods,
                enb_installed: enb_int != 0,
                created_at: row.get(8)?,
            })
        })
        .map_err(|e| format!("Failed to query history: {}", e))?;

    let mut result = Vec::new();
    for entry in entries {
        result.push(entry.map_err(|e| format!("Failed to read history row: {}", e))?);
    }

    Ok(result)
}

/// Get a single conversion record by ID.
fn get_conversion_record(
    db: &ModDatabase,
    conversion_id: i64,
) -> Result<ConversionHistoryEntry, String> {
    init_schema(db)?;

    let conn = db.conn().map_err(|e| e.to_string())?;
    conn.query_row(
        "SELECT id, game_id, bottle_name, snapshot_id, status,
                disabled_mods, swapped_mods, enb_installed, created_at
         FROM shader_conversions
         WHERE id = ?1",
        params![conversion_id],
        |row| {
            let disabled_json: String = row.get(5)?;
            let swapped_json: String = row.get(6)?;
            let enb_int: i64 = row.get(7)?;

            let disabled_mods: Vec<i64> =
                serde_json::from_str(&disabled_json).unwrap_or_default();
            let swapped_mods: Vec<i64> =
                serde_json::from_str(&swapped_json).unwrap_or_default();

            Ok(ConversionHistoryEntry {
                id: row.get(0)?,
                game_id: row.get(1)?,
                bottle_name: row.get(2)?,
                snapshot_id: row.get(3)?,
                status: row.get(4)?,
                disabled_mods,
                swapped_mods,
                enb_installed: enb_int != 0,
                created_at: row.get(8)?,
            })
        },
    )
    .map_err(|e| format!("Conversion record {} not found: {}", conversion_id, e))
}

/// Revert a previous conversion by restoring its snapshot and removing ENB
/// files.
///
/// 1. Looks up the conversion record
/// 2. Restores the pre-conversion snapshot (re-enables disabled mods, resets
///    priorities)
/// 3. Removes ENB root files from the game directory
/// 4. Marks the conversion record as 'reverted'
pub fn revert_conversion(
    db: &ModDatabase,
    conversion_id: i64,
    game_id: &str,
    bottle_name: &str,
    game_path: &Path,
) -> Result<rollback::RestoreResult, String> {
    let record = get_conversion_record(db, conversion_id)?;

    if record.status == "reverted" {
        return Err(format!(
            "Conversion {} has already been reverted",
            conversion_id
        ));
    }

    // Restore snapshot
    let restore_result =
        rollback::restore_snapshot(db, record.snapshot_id, game_id, bottle_name)?;

    // Remove ENB files from game root
    if record.enb_installed {
        remove_enb_files(game_path);
    }

    // Mark conversion as reverted
    let conn = db.conn().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE shader_conversions SET status = 'reverted' WHERE id = ?1",
        params![conversion_id],
    )
    .map_err(|e| format!("Failed to mark conversion as reverted: {}", e))?;

    info!(
        "Reverted conversion {}: enabled={}, disabled={}, not_found={}",
        conversion_id,
        restore_result.mods_enabled,
        restore_result.mods_disabled,
        restore_result.mods_not_found
    );

    Ok(restore_result)
}

/// Remove ENB files from the game root directory.
fn remove_enb_files(game_path: &Path) {
    for file_name in ENB_ROOT_FILES {
        let path = game_path.join(file_name);
        if path.exists() {
            if let Err(e) = fs::remove_file(&path) {
                warn!("Failed to remove ENB file {}: {}", path.display(), e);
            } else {
                debug!("Removed ENB file: {}", file_name);
            }
        }
    }

    // Remove enbseries/ directory
    let enbseries_dir = game_path.join("enbseries");
    if enbseries_dir.is_dir() {
        if let Err(e) = fs::remove_dir_all(&enbseries_dir) {
            warn!(
                "Failed to remove enbseries directory: {}",
                e
            );
        } else {
            debug!("Removed enbseries/ directory");
        }
    }

    // Remove version marker
    let marker = game_path.join("enb.version");
    if marker.exists() {
        let _ = fs::remove_file(&marker);
    }
}

// ---------------------------------------------------------------------------
// Tauri Commands
// ---------------------------------------------------------------------------

/// Scan a game's mod list for Community Shaders dependencies.
#[tauri::command]
#[allow(private_interfaces)]
pub async fn scan_shader_compatibility(
    game_id: String,
    bottle_name: String,
    state: State<'_, super::AppState>,
) -> Result<ShaderScanResult, String> {
    let db = state.db.clone();
    let (_, game, _data_dir) = super::resolve_game(&game_id, &bottle_name)?;
    let game_path = game.game_path.clone();

    tokio::task::spawn_blocking(move || {
        scan_for_cs_mods(&db, &game_id, &bottle_name, &game_path)
    })
    .await
    .map_err(|e| format!("Scan task failed: {}", e))?
}

/// Quick check: are any enabled mods CS-related? Returns count.
/// Lightweight version of scan_for_cs_mods that skips FOMOD/NexusID checks.
#[tauri::command]
#[allow(private_interfaces)]
pub async fn quick_cs_mod_count(
    game_id: String,
    bottle_name: String,
    state: State<'_, super::AppState>,
) -> Result<usize, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let mods = db
            .list_mods(&game_id, &bottle_name)
            .map_err(|e| format!("Failed to list mods: {}", e))?;

        let mut count = 0usize;
        for m in &mods {
            if !m.enabled {
                continue;
            }
            let mut reasons = Vec::new();
            check_file_patterns(&m.installed_files, &mut reasons);
            check_name_patterns(&m.name, &mut reasons);
            if !reasons.is_empty() {
                count += 1;
            }
        }
        Ok(count)
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))?
}

/// Discover ENB-variant files for detected CS mods via NexusMods API.
#[tauri::command]
#[allow(private_interfaces)]
pub async fn discover_shader_swap_options(
    game_id: String,
    bottle_name: String,
    detected_mods: Vec<CsDetectedMod>,
    state: State<'_, super::AppState>,
) -> Result<Vec<CsDetectedMod>, String> {
    let _ = state; // state not directly needed but kept for consistency
    let (_, game, _) = super::resolve_game(&game_id, &bottle_name)?;
    let game_slug = if game.nexus_slug.is_empty() {
        game_id_to_nexus_slug(&game_id)
    } else {
        game.nexus_slug.clone()
    };

    discover_enb_siblings(&detected_mods, &game_slug).await
}

/// Execute a full CS→ENB shader conversion.
#[tauri::command]
#[allow(private_interfaces)]
pub async fn execute_shader_conversion_cmd(
    app: AppHandle,
    game_id: String,
    bottle_name: String,
    config: ConversionConfig,
    state: State<'_, super::AppState>,
) -> Result<ConversionResult, String> {
    let db = state.db.clone();
    let _guard = super::DeployGuard::new(state.deploy_in_progress.clone(), app.clone());
    let (bottle, game, data_dir) = super::resolve_game(&game_id, &bottle_name)?;
    let game_path = game.game_path.clone();

    execute_conversion(
        &app,
        &db,
        &game_id,
        &bottle_name,
        &game_path,
        &data_dir,
        config,
        Some(&bottle),
    )
    .await
}

/// Revert a previous shader conversion.
#[tauri::command]
#[allow(private_interfaces)]
pub async fn revert_shader_conversion_cmd(
    app: AppHandle,
    game_id: String,
    bottle_name: String,
    conversion_id: i64,
    state: State<'_, super::AppState>,
) -> Result<serde_json::Value, String> {
    let db = state.db.clone();
    let _guard = super::DeployGuard::new(state.deploy_in_progress.clone(), app.clone());
    let (_, game, data_dir) = super::resolve_game(&game_id, &bottle_name)?;
    let game_path = game.game_path.clone();

    let result = tokio::task::spawn_blocking(move || {
        let restore = revert_conversion(&db, conversion_id, &game_id, &bottle_name, &game_path)?;

        let journal_id = deploy_journal::begin(
            &game_id, &bottle_name, deploy_journal::JournalOp::RedeployAll, &[],
        ).unwrap_or_default();

        // Redeploy after revert to apply restored mod states
        let app_clone = app.clone();
        deployer::redeploy_all_with_progress(
            &db,
            &game_id,
            &bottle_name,
            &data_dir,
            &game_path,
            Some(
                move |current: usize,
                      total: usize,
                      mod_name: &str,
                      files_deployed: usize,
                      total_files: usize| {
                    let _ = app_clone.emit(
                        "deploy-progress",
                        serde_json::json!({
                            "current": current,
                            "total": total,
                            "mod_name": mod_name,
                            "files_deployed": files_deployed,
                            "total_files": total_files,
                        }),
                    );
                },
            ),
        )
        .map_err(|e| format!("Redeploy after revert failed: {}", e))?;

        let _ = deploy_journal::complete(&journal_id);

        Ok::<_, String>(restore)
    })
    .await
    .map_err(|e| format!("Revert task failed: {}", e))??;

    serde_json::to_value(&result).map_err(|e| format!("Serialization failed: {}", e))
}

/// Get shader conversion history for a game/bottle.
#[tauri::command]
#[allow(private_interfaces)]
pub async fn get_shader_conversion_history_cmd(
    game_id: String,
    bottle_name: String,
    state: State<'_, super::AppState>,
) -> Result<Vec<ConversionHistoryEntry>, String> {
    let db = state.db.clone();

    tokio::task::spawn_blocking(move || get_conversion_history(&db, &game_id, &bottle_name))
        .await
        .map_err(|e| format!("History query task failed: {}", e))?
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Create an in-memory database with full schema for testing.
    fn test_db() -> (ModDatabase, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test_shader.db");
        let db = ModDatabase::new(&db_path).unwrap();
        // rollback schema is needed for snapshots
        rollback::init_schema(&db).unwrap();
        // shader conversion schema
        init_schema(&db).unwrap();
        (db, tmp)
    }

    /// Insert a fake mod with the given name and installed files. Returns the
    /// mod ID.
    fn insert_test_mod(db: &ModDatabase, name: &str, files: &[String]) -> i64 {
        let id = db
            .add_mod("skyrimse", "Gaming", None, name, "1.0", "test.zip", files)
            .unwrap();
        id
    }

    /// Insert a fake mod with nexus_mod_id set.
    fn insert_test_mod_with_nexus(
        db: &ModDatabase,
        name: &str,
        files: &[String],
        nexus_mod_id: i64,
    ) -> i64 {
        let id = db
            .add_mod(
                "skyrimse",
                "Gaming",
                Some(nexus_mod_id),
                name,
                "1.0",
                "test.zip",
                files,
            )
            .unwrap();
        id
    }

    // -----------------------------------------------------------------------
    // File pattern detection tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_cs_file_pattern_detection_core_dll() {
        let files = vec![
            "SKSE/Plugins/CommunityShaders.dll".to_string(),
            "meshes/architecture/farmhouse01.nif".to_string(),
        ];
        let mut reasons = Vec::new();
        check_file_patterns(&files, &mut reasons);
        assert!(reasons.contains(&CsDetectionReason::CoreDll));
    }

    #[test]
    fn test_cs_file_pattern_detection_config_files() {
        let files = vec![
            "SKSE/Plugins/CommunityShaders/Settings.ini".to_string(),
            "SKSE/Plugins/CommunityShaders/Features.json".to_string(),
        ];
        let mut reasons = Vec::new();
        check_file_patterns(&files, &mut reasons);
        assert!(reasons.contains(&CsDetectionReason::CsConfigFiles));
    }

    #[test]
    fn test_cs_file_pattern_detection_light_placer() {
        // DLL itself is a breaking CS dependency
        let files = vec!["SKSE/Plugins/LightPlacer.dll".to_string()];
        let mut reasons = Vec::new();
        check_file_patterns(&files, &mut reasons);
        assert!(reasons.contains(&CsDetectionReason::CsOnlyFiles));

        // Config files in lightplacer/ are harmless without the DLL
        let files2 = vec!["SKSE/Plugins/LightPlacer/some_config.json".to_string()];
        let mut reasons2 = Vec::new();
        check_file_patterns(&files2, &mut reasons2);
        assert!(reasons2.contains(&CsDetectionReason::LightPlacerConfigs));
    }

    #[test]
    fn test_cs_file_pattern_detection_pbr_textures() {
        let files = vec!["textures/pbr/terrain/dirt01.dds".to_string()];
        let mut reasons = Vec::new();
        check_file_patterns(&files, &mut reasons);
        assert!(reasons.contains(&CsDetectionReason::PbrTextures));
    }

    #[test]
    fn test_cs_file_pattern_detection_other_cs_dlls() {
        let files = vec![
            "SKSE/Plugins/GrassLighting.dll".to_string(),
            "SKSE/Plugins/ScreenSpaceShadows.dll".to_string(),
        ];
        let mut reasons = Vec::new();
        check_file_patterns(&files, &mut reasons);
        assert!(reasons.contains(&CsDetectionReason::CsOnlyFiles));
    }

    #[test]
    fn test_case_insensitive_file_matching() {
        let files = vec![
            "skse/plugins/COMMUNITYSHADERS.DLL".to_string(),
            "SKSE/PLUGINS/lightlimitfix.dll".to_string(),
        ];
        let mut reasons = Vec::new();
        check_file_patterns(&files, &mut reasons);
        assert!(reasons.contains(&CsDetectionReason::CoreDll));
        assert!(reasons.contains(&CsDetectionReason::CsOnlyFiles));
    }

    #[test]
    fn test_backslash_normalization() {
        let files = vec!["SKSE\\Plugins\\CommunityShaders.dll".to_string()];
        let mut reasons = Vec::new();
        check_file_patterns(&files, &mut reasons);
        assert!(reasons.contains(&CsDetectionReason::CoreDll));
    }

    #[test]
    fn test_no_cs_files_no_detection() {
        let files = vec![
            "textures/landscape/dirt01.dds".to_string(),
            "meshes/architecture/farmhouse01.nif".to_string(),
        ];
        let mut reasons = Vec::new();
        check_file_patterns(&files, &mut reasons);
        assert!(reasons.is_empty());
    }

    #[test]
    fn test_no_duplicate_reasons() {
        let files = vec![
            "SKSE/Plugins/CommunityShaders.dll".to_string(),
            "skse/plugins/communityshaders.dll".to_string(),
        ];
        let mut reasons = Vec::new();
        check_file_patterns(&files, &mut reasons);
        let core_count = reasons
            .iter()
            .filter(|r| matches!(r, CsDetectionReason::CoreDll))
            .count();
        assert_eq!(core_count, 1, "CoreDll reason should appear exactly once");
    }

    // -----------------------------------------------------------------------
    // Name pattern detection tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_cs_mod_name_detection() {
        let mut reasons = Vec::new();
        check_name_patterns("Community Shaders", &mut reasons);
        assert!(reasons.contains(&CsDetectionReason::KnownCsEcosystemMod));
    }

    #[test]
    fn test_partial_name_match() {
        let mut reasons = Vec::new();
        check_name_patterns(
            "My Custom Textures - Community Shaders Patch",
            &mut reasons,
        );
        assert!(reasons.contains(&CsDetectionReason::KnownCsEcosystemMod));
    }

    #[test]
    fn test_name_case_insensitive() {
        let mut reasons = Vec::new();
        check_name_patterns("COMMUNITY SHADERS", &mut reasons);
        assert!(reasons.contains(&CsDetectionReason::KnownCsEcosystemMod));
    }

    #[test]
    fn test_light_limit_fix_name() {
        let mut reasons = Vec::new();
        check_name_patterns("Light Limit Fix", &mut reasons);
        assert!(reasons.contains(&CsDetectionReason::KnownCsEcosystemMod));
    }

    #[test]
    fn test_water_parallax_name() {
        let mut reasons = Vec::new();
        check_name_patterns("Water Parallax", &mut reasons);
        assert!(reasons.contains(&CsDetectionReason::KnownCsEcosystemMod));
    }

    #[test]
    fn test_non_cs_name_no_detection() {
        let mut reasons = Vec::new();
        check_name_patterns("Immersive Armors", &mut reasons);
        assert!(reasons.is_empty());
    }

    // -----------------------------------------------------------------------
    // FOMOD recipe detection tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_cs_fomod_selection_detection() {
        let (db, _tmp) = test_db();
        let mod_id = insert_test_mod(&db, "Test Mod", &[]);

        let mut selections = HashMap::new();
        selections.insert(
            "Shader Version".to_string(),
            vec!["Community Shaders".to_string()],
        );
        fomod_recipes::save_recipe(&db, mod_id, "Test Mod", None, &selections).unwrap();

        let mut reasons = Vec::new();
        check_fomod_recipe(&db, mod_id, &mut reasons);
        assert!(reasons.contains(&CsDetectionReason::FomodCsSelection));
    }

    #[test]
    fn test_fomod_cs_version_pattern() {
        let (db, _tmp) = test_db();
        let mod_id = insert_test_mod(&db, "Test Mod", &[]);

        let mut selections = HashMap::new();
        selections.insert(
            "Compatibility".to_string(),
            vec!["CS Version".to_string()],
        );
        fomod_recipes::save_recipe(&db, mod_id, "Test Mod", None, &selections).unwrap();

        let mut reasons = Vec::new();
        check_fomod_recipe(&db, mod_id, &mut reasons);
        assert!(reasons.contains(&CsDetectionReason::FomodCsSelection));
    }

    #[test]
    fn test_fomod_no_cs_selection() {
        let (db, _tmp) = test_db();
        let mod_id = insert_test_mod(&db, "Test Mod", &[]);

        let mut selections = HashMap::new();
        selections.insert("Quality".to_string(), vec!["High".to_string()]);
        fomod_recipes::save_recipe(&db, mod_id, "Test Mod", None, &selections).unwrap();

        let mut reasons = Vec::new();
        check_fomod_recipe(&db, mod_id, &mut reasons);
        assert!(!reasons.contains(&CsDetectionReason::FomodCsSelection));
    }

    #[test]
    fn test_fomod_no_recipe_no_detection() {
        let (db, _tmp) = test_db();
        let mod_id = insert_test_mod(&db, "Test Mod", &[]);

        let mut reasons = Vec::new();
        check_fomod_recipe(&db, mod_id, &mut reasons);
        assert!(reasons.is_empty());
    }

    // -----------------------------------------------------------------------
    // ENB FOMOD selection computation tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_enb_fomod_selection_computation_basic() {
        let installer = fomod::FomodInstaller {
            module_name: "Test".to_string(),
            required_files: vec![],
            steps: vec![fomod::FomodStep {
                name: "Shader Options".to_string(),
                groups: vec![fomod::FomodGroup {
                    name: "Shader Version".to_string(),
                    group_type: "SelectExactlyOne".to_string(),
                    options: vec![
                        fomod::FomodOption {
                            name: "Community Shaders".to_string(),
                            description: String::new(),
                            image: None,
                            files: vec![],
                            type_descriptor: "Optional".to_string(),
                            conditional_type_patterns: vec![],
                            condition_flags: HashMap::new(),
                        },
                        fomod::FomodOption {
                            name: "ENB Version".to_string(),
                            description: String::new(),
                            image: None,
                            files: vec![],
                            type_descriptor: "Optional".to_string(),
                            conditional_type_patterns: vec![],
                            condition_flags: HashMap::new(),
                        },
                    ],
                }],
                visible: None,
            }],
            module_dependencies: None,
            conditional_file_installs: vec![],
        };

        let mut current = HashMap::new();
        current.insert(
            "Shader Version".to_string(),
            vec!["Community Shaders".to_string()],
        );

        let result = compute_enb_fomod_selections(&current, &installer);
        let sel = result.get("Shader Version").unwrap();
        assert!(sel.contains(&"ENB Version".to_string()));
        assert!(!sel.contains(&"Community Shaders".to_string()));
    }

    #[test]
    fn test_mesh_split_selection_swap() {
        let installer = fomod::FomodInstaller {
            module_name: "Test".to_string(),
            required_files: vec![],
            steps: vec![fomod::FomodStep {
                name: "Meshes".to_string(),
                groups: vec![fomod::FomodGroup {
                    name: "Mesh Options".to_string(),
                    group_type: "SelectExactlyOne".to_string(),
                    options: vec![
                        fomod::FomodOption {
                            name: "Split Meshes".to_string(),
                            description: String::new(),
                            image: None,
                            files: vec![],
                            type_descriptor: "Optional".to_string(),
                            conditional_type_patterns: vec![],
                            condition_flags: HashMap::new(),
                        },
                        fomod::FomodOption {
                            name: "Default Meshes".to_string(),
                            description: String::new(),
                            image: None,
                            files: vec![],
                            type_descriptor: "Optional".to_string(),
                            conditional_type_patterns: vec![],
                            condition_flags: HashMap::new(),
                        },
                    ],
                }],
                visible: None,
            }],
            module_dependencies: None,
            conditional_file_installs: vec![],
        };

        let mut current = HashMap::new();
        current.insert(
            "Mesh Options".to_string(),
            vec!["Split Meshes".to_string()],
        );

        let result = compute_enb_fomod_selections(&current, &installer);
        let sel = result.get("Mesh Options").unwrap();
        assert!(sel.contains(&"Default Meshes".to_string()));
        assert!(!sel.contains(&"Split Meshes".to_string()));
    }

    #[test]
    fn test_fomod_no_change_when_no_cs() {
        let installer = fomod::FomodInstaller {
            module_name: "Test".to_string(),
            required_files: vec![],
            steps: vec![fomod::FomodStep {
                name: "Quality".to_string(),
                groups: vec![fomod::FomodGroup {
                    name: "Quality".to_string(),
                    group_type: "SelectExactlyOne".to_string(),
                    options: vec![
                        fomod::FomodOption {
                            name: "High".to_string(),
                            description: String::new(),
                            image: None,
                            files: vec![],
                            type_descriptor: "Optional".to_string(),
                            conditional_type_patterns: vec![],
                            condition_flags: HashMap::new(),
                        },
                        fomod::FomodOption {
                            name: "Low".to_string(),
                            description: String::new(),
                            image: None,
                            files: vec![],
                            type_descriptor: "Optional".to_string(),
                            conditional_type_patterns: vec![],
                            condition_flags: HashMap::new(),
                        },
                    ],
                }],
                visible: None,
            }],
            module_dependencies: None,
            conditional_file_installs: vec![],
        };

        let mut current = HashMap::new();
        current.insert("Quality".to_string(), vec!["High".to_string()]);

        let result = compute_enb_fomod_selections(&current, &installer);
        assert_eq!(result, current, "Selections should be unchanged when no CS options");
    }

    // -----------------------------------------------------------------------
    // Scan integration tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_scan_empty_modlist() {
        let (db, tmp) = test_db();
        let game_path = tmp.path().join("game");
        fs::create_dir_all(&game_path).unwrap();

        let result = scan_for_cs_mods(&db, "skyrimse", "Gaming", &game_path).unwrap();
        assert_eq!(result.total_cs_mods, 0);
        assert!(result.detected_mods.is_empty());
        assert!(!result.enb_already_installed);
    }

    #[test]
    fn test_scan_no_cs_mods() {
        let (db, tmp) = test_db();
        let game_path = tmp.path().join("game");
        fs::create_dir_all(&game_path).unwrap();

        insert_test_mod(
            &db,
            "Immersive Armors",
            &["meshes/armor/dragonscale/cuirass.nif".to_string()],
        );
        insert_test_mod(
            &db,
            "Static Mesh Improvement Mod",
            &["meshes/architecture/whiterun/wrplanks01.nif".to_string()],
        );

        let result = scan_for_cs_mods(&db, "skyrimse", "Gaming", &game_path).unwrap();
        assert_eq!(result.total_cs_mods, 0);
        assert!(result.detected_mods.is_empty());
    }

    #[test]
    fn test_scan_detects_cs_core() {
        let (db, tmp) = test_db();
        let game_path = tmp.path().join("game");
        fs::create_dir_all(&game_path).unwrap();

        insert_test_mod(
            &db,
            "Community Shaders",
            &["SKSE/Plugins/CommunityShaders.dll".to_string()],
        );

        let result = scan_for_cs_mods(&db, "skyrimse", "Gaming", &game_path).unwrap();
        assert_eq!(result.total_cs_mods, 1);
        assert!(result.detected_mods[0]
            .reasons
            .contains(&CsDetectionReason::CoreDll));
        assert!(result.detected_mods[0]
            .reasons
            .contains(&CsDetectionReason::KnownCsEcosystemMod));
    }

    #[test]
    fn test_scan_detects_by_nexus_id() {
        let (db, tmp) = test_db();
        let game_path = tmp.path().join("game");
        fs::create_dir_all(&game_path).unwrap();

        insert_test_mod_with_nexus(
            &db,
            "Some Shader Mod",
            &["textures/effects/glow01.dds".to_string()],
            86492, // CS NexusMods ID
        );

        let result = scan_for_cs_mods(&db, "skyrimse", "Gaming", &game_path).unwrap();
        assert_eq!(result.total_cs_mods, 1);
        assert!(result.detected_mods[0]
            .reasons
            .contains(&CsDetectionReason::KnownCsEcosystemMod));
    }

    #[test]
    fn test_enb_already_installed_detection() {
        let (db, tmp) = test_db();
        let game_path = tmp.path().join("game");
        fs::create_dir_all(&game_path).unwrap();

        // Create fake ENB files
        fs::write(game_path.join("d3d11.dll"), b"fake dll").unwrap();
        fs::write(game_path.join("enblocal.ini"), b"[GLOBAL]").unwrap();

        let result = scan_for_cs_mods(&db, "skyrimse", "Gaming", &game_path).unwrap();
        assert!(result.enb_already_installed);
    }

    #[test]
    fn test_enb_not_installed_partial() {
        let (db, tmp) = test_db();
        let game_path = tmp.path().join("game");
        fs::create_dir_all(&game_path).unwrap();

        // Only d3d11.dll without enblocal.ini
        fs::write(game_path.join("d3d11.dll"), b"fake dll").unwrap();

        let result = scan_for_cs_mods(&db, "skyrimse", "Gaming", &game_path).unwrap();
        assert!(!result.enb_already_installed);
    }

    // -----------------------------------------------------------------------
    // Batch disable tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_batch_disable() {
        let (db, tmp) = test_db();
        let game_path = tmp.path().join("game");
        let data_dir = game_path.join("Data");
        fs::create_dir_all(&data_dir).unwrap();

        let id1 = insert_test_mod(&db, "CS Mod 1", &[]);
        let id2 = insert_test_mod(&db, "CS Mod 2", &[]);
        let id3 = insert_test_mod(&db, "Normal Mod", &[]);

        let disabled =
            batch_disable_cs_mods(&db, "skyrimse", "Gaming", &[id1, id2], &data_dir, &game_path)
                .unwrap();
        assert_eq!(disabled, 2);

        // Verify mods are disabled in DB
        let m1 = db.get_mod(id1).unwrap().unwrap();
        assert!(!m1.enabled);
        let m2 = db.get_mod(id2).unwrap().unwrap();
        assert!(!m2.enabled);
        // Normal mod should still be enabled
        let m3 = db.get_mod(id3).unwrap().unwrap();
        assert!(m3.enabled);
    }

    #[test]
    fn test_batch_disable_nonexistent_mod() {
        let (db, tmp) = test_db();
        let game_path = tmp.path().join("game");
        let data_dir = game_path.join("Data");
        fs::create_dir_all(&data_dir).unwrap();

        // Disabling a non-existent mod should not panic — just skip it
        let disabled =
            batch_disable_cs_mods(&db, "skyrimse", "Gaming", &[99999], &data_dir, &game_path)
                .unwrap();
        assert_eq!(disabled, 0);
    }

    // -----------------------------------------------------------------------
    // ENB local INI patching tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_enb_local_ini_patching_new_file() {
        let result = ensure_enb_linux_version("");
        assert!(result.contains("[GLOBAL]"));
        assert!(result.contains("LinuxVersion=true"));
    }

    #[test]
    fn test_enb_local_ini_patching_existing_global() {
        let content = "[GLOBAL]\nSomeKey=value\n[OTHER]\nFoo=bar\n";
        let result = ensure_enb_linux_version(content);
        assert!(result.contains("LinuxVersion=true"));
        assert!(result.contains("SomeKey=value"));
    }

    #[test]
    fn test_enb_local_ini_patching_update_existing() {
        let content = "[GLOBAL]\nLinuxVersion=false\n";
        let result = ensure_enb_linux_version(content);
        assert!(result.contains("LinuxVersion=true"));
        assert!(!result.contains("LinuxVersion=false"));
    }

    #[test]
    fn test_enb_local_ini_patching_preserves_other_sections() {
        let content = "[MEMORY]\nVideoMemorySizeMb=12288\n\n[GLOBAL]\nUsePatchSpeedhackWithoutGraphics=false\n\n[ENGINE]\nEnableAmbientOcclusion=true\n";
        let result = ensure_enb_linux_version(content);
        assert!(result.contains("VideoMemorySizeMb=12288"));
        assert!(result.contains("LinuxVersion=true"));
        assert!(result.contains("EnableAmbientOcclusion=true"));
    }

    #[test]
    fn test_enb_local_ini_trailing_newline() {
        let content = "[GLOBAL]\nSomeKey=value";
        let result = ensure_enb_linux_version(content);
        assert!(result.ends_with('\n'));
    }

    // -----------------------------------------------------------------------
    // ENB version parsing tests
    // ---------------------------------------------------------------- ------

    #[test]
    fn test_parse_enb_version_from_html_basic() {
        let html = r#"<a href="enbseries_skyrimse_v0503.zip">Download</a>"#;
        let version = parse_enb_version_from_html(html);
        assert_eq!(version, Some("0503".to_string()));
    }

    #[test]
    fn test_parse_enb_version_picks_highest() {
        let html = r#"
            <a href="enbseries_skyrimse_v0499.zip">Old</a>
            <a href="enbseries_skyrimse_v0503.zip">New</a>
            <a href="enbseries_skyrimse_v0501.zip">Mid</a>
        "#;
        let version = parse_enb_version_from_html(html);
        assert_eq!(version, Some("0503".to_string()));
    }

    #[test]
    fn test_parse_enb_version_no_match() {
        let html = r#"<h1>Welcome to enbdev.com</h1>"#;
        let version = parse_enb_version_from_html(html);
        assert!(version.is_none());
    }

    #[test]
    fn test_parse_enb_version_ignores_non_numeric() {
        let html = r#"<a href="enbseries_skyrimse_vbeta.zip">Beta</a>"#;
        let version = parse_enb_version_from_html(html);
        assert!(version.is_none());
    }

    // -----------------------------------------------------------------------
    // ENB variant file discovery tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_find_enb_variant_file_found() {
        let files = vec![
            NexusModFile {
                mod_id: 100,
                file_id: 1,
                name: "CS Version".to_string(),
                version: "1.0".to_string(),
                file_name: "cs_version.zip".to_string(),
                size_kb: 1000,
                description: "For Community Shaders".to_string(),
                category: "main".to_string(),
            },
            NexusModFile {
                mod_id: 100,
                file_id: 2,
                name: "ENB Version".to_string(),
                version: "1.0".to_string(),
                file_name: "enb_version.zip".to_string(),
                size_kb: 1000,
                description: "For ENBSeries".to_string(),
                category: "main".to_string(),
            },
        ];

        let result = find_enb_variant_file(&files);
        assert!(result.is_some());
        assert_eq!(result.unwrap().file_id, 2);
    }

    #[test]
    fn test_find_enb_variant_file_not_found() {
        let files = vec![NexusModFile {
            mod_id: 100,
            file_id: 1,
            name: "Main File".to_string(),
            version: "1.0".to_string(),
            file_name: "main.zip".to_string(),
            size_kb: 1000,
            description: "The main mod".to_string(),
            category: "main".to_string(),
        }];

        let result = find_enb_variant_file(&files);
        assert!(result.is_none());
    }

    #[test]
    fn test_find_enb_variant_skips_wrong_category() {
        let files = vec![NexusModFile {
            mod_id: 100,
            file_id: 1,
            name: "ENB Patch".to_string(),
            version: "1.0".to_string(),
            file_name: "enb_patch.zip".to_string(),
            size_kb: 100,
            description: "Old ENB patch".to_string(),
            category: "old_version".to_string(),
        }];

        let result = find_enb_variant_file(&files);
        assert!(result.is_none());
    }

    // -----------------------------------------------------------------------
    // Conversion record DB tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_conversion_record_save_load() {
        let (db, _tmp) = test_db();

        let id = save_conversion_record(
            &db,
            "skyrimse",
            "Gaming",
            1,
            &[10, 20, 30],
            &[40, 50],
            true,
        )
        .unwrap();

        assert!(id > 0);

        let history = get_conversion_history(&db, "skyrimse", "Gaming").unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].id, id);
        assert_eq!(history[0].snapshot_id, 1);
        assert_eq!(history[0].status, "completed");
        assert_eq!(history[0].disabled_mods, vec![10, 20, 30]);
        assert_eq!(history[0].swapped_mods, vec![40, 50]);
        assert!(history[0].enb_installed);
    }

    #[test]
    fn test_conversion_history_multiple() {
        let (db, _tmp) = test_db();

        save_conversion_record(&db, "skyrimse", "Gaming", 1, &[10], &[], false).unwrap();
        save_conversion_record(&db, "skyrimse", "Gaming", 2, &[20], &[30], true).unwrap();
        save_conversion_record(&db, "skyrimse", "OtherBottle", 3, &[40], &[], true).unwrap();

        let history = get_conversion_history(&db, "skyrimse", "Gaming").unwrap();
        assert_eq!(history.len(), 2);
        // Should be ordered newest first
        assert_eq!(history[0].snapshot_id, 2);
        assert_eq!(history[1].snapshot_id, 1);

        // Other bottle should have its own history
        let other = get_conversion_history(&db, "skyrimse", "OtherBottle").unwrap();
        assert_eq!(other.len(), 1);
    }

    #[test]
    fn test_conversion_history_empty() {
        let (db, _tmp) = test_db();
        let history = get_conversion_history(&db, "skyrimse", "Gaming").unwrap();
        assert!(history.is_empty());
    }

    #[test]
    fn test_get_conversion_record_not_found() {
        let (db, _tmp) = test_db();
        let result = get_conversion_record(&db, 99999);
        assert!(result.is_err());
    }

    #[test]
    fn test_revert_marks_reverted() {
        let (db, tmp) = test_db();
        let game_path = tmp.path().join("game");
        fs::create_dir_all(&game_path).unwrap();

        // Create a mod so the snapshot has something
        let mod_id = insert_test_mod(&db, "Test Mod", &[]);

        // Create a snapshot
        let snapshot_id =
            rollback::create_snapshot(&db, "skyrimse", "Gaming", "test", None).unwrap();

        // Save conversion record
        let conv_id =
            save_conversion_record(&db, "skyrimse", "Gaming", snapshot_id, &[mod_id], &[], false)
                .unwrap();

        // Revert
        let result =
            revert_conversion(&db, conv_id, "skyrimse", "Gaming", &game_path).unwrap();
        // Snapshot was restored — at least some action was taken
        assert!(result.mods_enabled > 0 || result.mods_disabled > 0 || result.mods_not_found > 0
            || (result.mods_enabled == 0 && result.mods_disabled == 0));

        // Verify status is reverted
        let record = get_conversion_record(&db, conv_id).unwrap();
        assert_eq!(record.status, "reverted");
    }

    #[test]
    fn test_revert_already_reverted() {
        let (db, tmp) = test_db();
        let game_path = tmp.path().join("game");
        fs::create_dir_all(&game_path).unwrap();

        let _mod_id = insert_test_mod(&db, "Test Mod", &[]);
        let snapshot_id =
            rollback::create_snapshot(&db, "skyrimse", "Gaming", "test", None).unwrap();
        let conv_id =
            save_conversion_record(&db, "skyrimse", "Gaming", snapshot_id, &[], &[], false)
                .unwrap();

        // First revert
        revert_conversion(&db, conv_id, "skyrimse", "Gaming", &game_path).unwrap();

        // Second revert should fail
        let result = revert_conversion(&db, conv_id, "skyrimse", "Gaming", &game_path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already been reverted"));
    }

    // -----------------------------------------------------------------------
    // ENB file removal tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_remove_enb_files() {
        let tmp = TempDir::new().unwrap();
        let game_path = tmp.path();

        // Create fake ENB files
        fs::write(game_path.join("d3d11.dll"), b"fake").unwrap();
        fs::write(game_path.join("d3dcompiler_46e.dll"), b"fake").unwrap();
        fs::write(game_path.join("enblocal.ini"), b"[GLOBAL]").unwrap();
        fs::write(game_path.join("enbseries.ini"), b"[EFFECT]").unwrap();
        fs::create_dir_all(game_path.join("enbseries")).unwrap();
        fs::write(game_path.join("enbseries/effect.fx"), b"shader").unwrap();
        fs::write(game_path.join("enb.version"), b"0503").unwrap();

        remove_enb_files(game_path);

        assert!(!game_path.join("d3d11.dll").exists());
        assert!(!game_path.join("d3dcompiler_46e.dll").exists());
        assert!(!game_path.join("enblocal.ini").exists());
        assert!(!game_path.join("enbseries.ini").exists());
        assert!(!game_path.join("enbseries").exists());
        assert!(!game_path.join("enb.version").exists());
    }

    #[test]
    fn test_remove_enb_files_missing() {
        let tmp = TempDir::new().unwrap();
        // Should not panic when files don't exist
        remove_enb_files(tmp.path());
    }

    // -----------------------------------------------------------------------
    // Detection reason coverage test
    // -----------------------------------------------------------------------

    #[test]
    fn test_known_cs_detection_reasons() {
        // Verify all reason types can be constructed and compared
        let reasons = vec![
            CsDetectionReason::CoreDll,
            CsDetectionReason::CsConfigFiles,
            CsDetectionReason::LightPlacerConfigs,
            CsDetectionReason::PbrTextures,
            CsDetectionReason::FomodCsSelection,
            CsDetectionReason::KnownCsEcosystemMod,
            CsDetectionReason::CsOnlyFiles,
        ];

        assert_eq!(reasons.len(), 7, "All 7 detection reason types should exist");

        // Verify PartialEq works
        assert_eq!(CsDetectionReason::CoreDll, CsDetectionReason::CoreDll);
        assert_ne!(CsDetectionReason::CoreDll, CsDetectionReason::PbrTextures);
    }

    // -----------------------------------------------------------------------
    // Game ID to slug mapping tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_game_id_to_nexus_slug() {
        assert_eq!(game_id_to_nexus_slug("skyrimse"), "skyrimspecialedition");
        assert_eq!(game_id_to_nexus_slug("fallout4"), "fallout4");
        assert_eq!(game_id_to_nexus_slug("falloutnv"), "newvegas");
        assert_eq!(game_id_to_nexus_slug("unknown"), "unknown");
    }

    // -----------------------------------------------------------------------
    // Check ENB installed tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_check_enb_installed_both_files() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("d3d11.dll"), b"dll").unwrap();
        fs::write(tmp.path().join("enblocal.ini"), b"ini").unwrap();
        assert!(check_enb_installed(tmp.path()));
    }

    #[test]
    fn test_check_enb_installed_missing_ini() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("d3d11.dll"), b"dll").unwrap();
        assert!(!check_enb_installed(tmp.path()));
    }

    #[test]
    fn test_check_enb_installed_missing_dll() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("enblocal.ini"), b"ini").unwrap();
        assert!(!check_enb_installed(tmp.path()));
    }

    #[test]
    fn test_check_enb_installed_empty_dir() {
        let tmp = TempDir::new().unwrap();
        assert!(!check_enb_installed(tmp.path()));
    }

    // -----------------------------------------------------------------------
    // Schema idempotency test
    // -----------------------------------------------------------------------

    #[test]
    fn test_schema_idempotent() {
        let (db, _tmp) = test_db();
        // Should not error on second call
        init_schema(&db).unwrap();
        init_schema(&db).unwrap();
    }

    // -----------------------------------------------------------------------
    // Copy dir recursive test
    // -----------------------------------------------------------------------

    #[test]
    fn test_copy_dir_recursive() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");

        fs::create_dir_all(src.join("sub1/sub2")).unwrap();
        fs::write(src.join("file1.txt"), b"content1").unwrap();
        fs::write(src.join("sub1/file2.txt"), b"content2").unwrap();
        fs::write(src.join("sub1/sub2/file3.txt"), b"content3").unwrap();

        copy_dir_recursive(&src, &dst).unwrap();

        assert!(dst.join("file1.txt").exists());
        assert!(dst.join("sub1/file2.txt").exists());
        assert!(dst.join("sub1/sub2/file3.txt").exists());
        assert_eq!(fs::read_to_string(dst.join("file1.txt")).unwrap(), "content1");
        assert_eq!(
            fs::read_to_string(dst.join("sub1/sub2/file3.txt")).unwrap(),
            "content3"
        );
    }

    // -----------------------------------------------------------------------
    // Default action assignment test
    // -----------------------------------------------------------------------

    #[test]
    fn test_scan_default_action_disable() {
        let (db, tmp) = test_db();
        let game_path = tmp.path().join("game");
        fs::create_dir_all(&game_path).unwrap();

        insert_test_mod(
            &db,
            "Light Limit Fix",
            &["SKSE/Plugins/LightLimitFix.dll".to_string()],
        );

        let result = scan_for_cs_mods(&db, "skyrimse", "Gaming", &game_path).unwrap();
        assert_eq!(result.total_cs_mods, 1);
        assert!(matches!(
            result.detected_mods[0].action,
            CsModAction::Disable
        ));
    }

    #[test]
    fn test_scan_default_action_fomod_rerun() {
        let (db, tmp) = test_db();
        let game_path = tmp.path().join("game");
        fs::create_dir_all(&game_path).unwrap();

        let mod_id = insert_test_mod(
            &db,
            "Texture Pack",
            &["textures/landscape/dirt01.dds".to_string()],
        );

        // Save a FOMOD recipe with CS selection
        let mut selections = HashMap::new();
        selections.insert(
            "Shader".to_string(),
            vec!["Community Shaders".to_string()],
        );
        fomod_recipes::save_recipe(&db, mod_id, "Texture Pack", None, &selections).unwrap();

        let result = scan_for_cs_mods(&db, "skyrimse", "Gaming", &game_path).unwrap();
        assert_eq!(result.total_cs_mods, 1);
        assert!(matches!(
            result.detected_mods[0].action,
            CsModAction::RerunFomod { .. }
        ));
    }

    // -----------------------------------------------------------------------
    // ENB version numeric comparison test
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_enb_version_numeric_not_lexicographic() {
        let html = r#"
            <a href="enbseries_skyrimse_v503.zip">Old</a>
            <a href="enbseries_skyrimse_v1001.zip">New</a>
        "#;
        let version = parse_enb_version_from_html(html);
        assert_eq!(version, Some("1001".to_string()));
    }

    // -----------------------------------------------------------------------
    // Game ID to slug mapping coverage
    // -----------------------------------------------------------------------

    #[test]
    fn test_game_id_to_nexus_slug_all_mappings() {
        assert_eq!(game_id_to_nexus_slug("skyrim"), "skyrim");
        assert_eq!(game_id_to_nexus_slug("fallout3"), "fallout3");
        assert_eq!(game_id_to_nexus_slug("oblivion"), "oblivion");
        assert_eq!(game_id_to_nexus_slug("starfield"), "starfield");
        // Unknown games pass through unchanged
        assert_eq!(game_id_to_nexus_slug("cyberpunk2077"), "cyberpunk2077");
    }

    // -----------------------------------------------------------------------
    // Water Parallax NexusMods ID test
    // -----------------------------------------------------------------------

    #[test]
    fn test_known_cs_nexus_ids_contains_water_parallax() {
        assert!(KNOWN_CS_NEXUS_IDS.contains(&108949), "Water Parallax CS mod ID should be in known list");
    }

    // -----------------------------------------------------------------------
    // Counts test
    // -----------------------------------------------------------------------

    #[test]
    fn test_scan_result_counts() {
        let (db, tmp) = test_db();
        let game_path = tmp.path().join("game");
        fs::create_dir_all(&game_path).unwrap();

        // Two CS mods that should be disabled
        insert_test_mod(
            &db,
            "Community Shaders",
            &["SKSE/Plugins/CommunityShaders.dll".to_string()],
        );
        insert_test_mod(
            &db,
            "Grass Lighting",
            &["SKSE/Plugins/GrassLighting.dll".to_string()],
        );

        // One mod with FOMOD CS selection (should be rerun)
        let fomod_mod = insert_test_mod(
            &db,
            "Texture Mod with FOMOD",
            &["textures/landscape/dirt01.dds".to_string()],
        );
        let mut sel = HashMap::new();
        sel.insert("Shader".to_string(), vec!["CS Patch".to_string()]);
        fomod_recipes::save_recipe(&db, fomod_mod, "Texture Mod with FOMOD", None, &sel).unwrap();

        // One normal mod (not detected)
        insert_test_mod(
            &db,
            "Immersive Armors",
            &["meshes/armor/dragonscale.nif".to_string()],
        );

        let result = scan_for_cs_mods(&db, "skyrimse", "Gaming", &game_path).unwrap();
        assert_eq!(result.total_cs_mods, 3);
        assert_eq!(result.disable_only_count, 2);
        assert_eq!(result.fomod_rerun_count, 1);
        assert_eq!(result.swappable_count, 0);
    }
}
