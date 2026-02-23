pub mod bottle_config;
pub mod bottles;
pub mod collection_installer;
pub mod collections;
pub mod config;
pub mod conflict_resolver;
pub mod crashlog;
pub mod database;
pub mod deployer;
pub mod disk_budget;
pub mod display_fix;
pub mod downgrader;
pub mod download_queue;
pub mod executables;
pub mod fomod;
pub mod fomod_recipes;
pub mod game_registry;
pub mod games;
pub mod ini_manager;
pub mod installer;
pub mod integrity;
pub mod launcher;
pub mod loot;
pub mod loot_rules;
pub mod migrations;
pub mod mod_dependencies;
pub mod mod_recommendations;
pub mod mod_tools;
pub mod modlist_io;
pub mod nexus;
pub mod nexus_sso;
pub mod oauth;
pub mod plugins;
pub mod preflight;
pub mod profiles;
pub mod progress;
pub mod rollback;
pub mod session_tracker;
pub mod skse;
pub mod staging;
pub mod wabbajack;
pub mod wabbajack_directives;
pub mod wabbajack_downloader;
pub mod wabbajack_installer;
pub mod wabbajack_types;
pub mod wine_diagnostic;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, State};

use bottles::Bottle;
use collections::{
    CollectionDiff, CollectionInfo, CollectionManifest, CollectionMod, CollectionRevision,
    CollectionSearchResult,
};
use config::AppConfig;
use crashlog::{CrashLogEntry, CrashReport};
use database::{CollectionSummary, DeploymentEntry, FileConflict, InstalledMod, ModDatabase};
use downgrader::DowngradeStatus;
use executables::CustomExecutable;
use fomod::FomodInstaller;
use games::DetectedGame;
use integrity::IntegrityReport;
use launcher::LaunchResult;
use loot::{PluginWarning, SortResult};
use loot_rules::PluginRule;
use modlist_io::{ImportPlan, ModlistDiff};
use nexus::{ModUpdateInfo, NexusCategory, NexusSearchResult};
use oauth::{NexusUserInfo, TokenPair};
use plugins::skyrim_plugins::PluginEntry;
use profiles::Profile;
use rollback::{ModSnapshot, ModVersion};
use skse::SkseStatus;
use wabbajack::{ModlistSummary, ParsedModlist};

struct AppState {
    db: Arc<ModDatabase>,
    download_queue: Arc<download_queue::DownloadQueue>,
    wj_cancel_tokens: std::sync::Mutex<std::collections::HashMap<i64, Arc<std::sync::atomic::AtomicBool>>>,
}

/// Resolve a bottle by name, returning a useful error if not found.
fn resolve_bottle(bottle_name: &str) -> Result<Bottle, String> {
    bottles::find_bottle_by_name(bottle_name)
        .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))
}

/// Resolve a bottle + game pair, returning both plus the data directory.
fn resolve_game(
    game_id: &str,
    bottle_name: &str,
) -> Result<(Bottle, DetectedGame, PathBuf), String> {
    let bottle = resolve_bottle(bottle_name)?;
    let detected_games = games::detect_games(&bottle);
    let game = detected_games
        .into_iter()
        .find(|g| g.game_id == game_id)
        .ok_or_else(|| format!("Game '{}' not found in bottle '{}'", game_id, bottle_name))?;
    let data_dir = PathBuf::from(&game.data_dir);
    Ok((bottle, game, data_dir))
}

// --- Tauri Commands ---

#[tauri::command]
fn get_bottles() -> Result<Vec<Bottle>, String> {
    Ok(bottles::detect_bottles())
}

#[tauri::command]
fn get_games(bottle_name: Option<String>) -> Result<Vec<DetectedGame>, String> {
    match bottle_name {
        Some(name) => {
            let bottle = resolve_bottle(&name)?;
            Ok(games::detect_games(&bottle))
        }
        None => Ok(games::detect_all_games()),
    }
}

#[tauri::command]
fn get_all_games() -> Result<Vec<DetectedGame>, String> {
    Ok(games::detect_all_games())
}

#[tauri::command]
fn list_supported_games() -> Result<Vec<game_registry::SupportedGame>, String> {
    Ok(game_registry::list_supported_games())
}

#[tauri::command]
fn get_bottle_settings(bottle_name: String) -> Result<bottle_config::BottleSettings, String> {
    let bottle = resolve_bottle(&bottle_name)?;
    bottle_config::get_bottle_settings(&bottle).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_bottle_setting_defs(
    bottle_name: String,
) -> Result<Vec<bottle_config::BottleSettingDef>, String> {
    let bottle = resolve_bottle(&bottle_name)?;
    let settings = bottle_config::get_bottle_settings(&bottle).map_err(|e| e.to_string())?;
    Ok(bottle_config::get_setting_definitions(&settings))
}

#[tauri::command]
fn set_bottle_setting(bottle_name: String, key: String, value: String) -> Result<(), String> {
    let bottle = resolve_bottle(&bottle_name)?;
    bottle_config::set_bottle_setting(&bottle, &key, &value).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_installed_mods(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<Vec<InstalledMod>, String> {
    let db = &state.db;
    db.list_mods(&game_id, &bottle_name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn install_mod_cmd(
    app: AppHandle,
    archive_path: String,
    game_id: String,
    bottle_name: String,
    mod_name: Option<String>,
    mod_version: Option<String>,
    source_type: Option<String>,
    source_url: Option<String>,
    state: State<AppState>,
) -> Result<InstalledMod, String> {
    use progress::{InstallProgress, INSTALL_PROGRESS_EVENT};

    let archive = PathBuf::from(&archive_path);
    if !archive.exists() {
        return Err(format!("Archive not found: {}", archive_path));
    }

    let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;
    let name = mod_name.unwrap_or_else(|| {
        archive
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Unknown Mod".to_string())
    });
    let version = mod_version.unwrap_or_default();

    // Emit: mod started
    let _ = app.emit(
        INSTALL_PROGRESS_EVENT,
        InstallProgress::ModStarted {
            mod_index: 0,
            total_mods: 1,
            mod_name: name.clone(),
        },
    );

    // Step 1: Reserve DB record
    let _ = app.emit(
        INSTALL_PROGRESS_EVENT,
        InstallProgress::StepChanged {
            mod_index: 0,
            step: "preparing".to_string(),
            detail: Some("Reserving database entry...".to_string()),
        },
    );

    let db = &state.db;

    let next_priority = db
        .get_next_priority(&game_id, &bottle_name)
        .map_err(|e| e.to_string())?;
    let mod_id = db
        .add_mod(
            &game_id,
            &bottle_name,
            None,
            &name,
            &version,
            &archive_path,
            &[],
        )
        .map_err(|e| {
            let _ = app.emit(
                INSTALL_PROGRESS_EVENT,
                InstallProgress::ModFailed {
                    mod_index: 0,
                    mod_name: name.clone(),
                    error: e.to_string(),
                },
            );
            e.to_string()
        })?;
    db.set_mod_priority(mod_id, next_priority)
        .map_err(|e| e.to_string())?;

    // Step 2: Extract and stage
    let _ = app.emit(
        INSTALL_PROGRESS_EVENT,
        InstallProgress::StepChanged {
            mod_index: 0,
            step: "extracting".to_string(),
            detail: Some(format!(
                "Extracting {}...",
                archive.file_name().unwrap_or_default().to_string_lossy()
            )),
        },
    );

    let staging_result = match staging::stage_mod(&archive, &game_id, &bottle_name, mod_id, &name) {
        Ok(r) => r,
        Err(e) => {
            let _ = db.remove_mod(mod_id);
            let _ = app.emit(
                INSTALL_PROGRESS_EVENT,
                InstallProgress::ModFailed {
                    mod_index: 0,
                    mod_name: name.clone(),
                    error: format!("Staging failed: {}", e),
                },
            );
            return Err(format!("Staging failed: {}", e));
        }
    };

    // Step 3: Update DB with staging info
    let _ = app.emit(
        INSTALL_PROGRESS_EVENT,
        InstallProgress::StepChanged {
            mod_index: 0,
            step: "registering".to_string(),
            detail: Some(format!("Recording {} files...", staging_result.files.len())),
        },
    );

    db.set_staging_path(mod_id, &staging_result.staging_path.to_string_lossy())
        .map_err(|e| e.to_string())?;
    db.update_installed_files(mod_id, &staging_result.files)
        .map_err(|e| e.to_string())?;
    db.store_file_hashes(mod_id, &staging_result.hashes)
        .map_err(|e| e.to_string())?;

    // Step 4: Deploy from staging to game dir
    let _ = app.emit(
        INSTALL_PROGRESS_EVENT,
        InstallProgress::StepChanged {
            mod_index: 0,
            step: "deploying".to_string(),
            detail: Some("Creating hardlinks to game directory...".to_string()),
        },
    );

    if let Err(e) = deployer::deploy_mod(
        db,
        &game_id,
        &bottle_name,
        mod_id,
        &staging_result.staging_path,
        &data_dir,
        &staging_result.files,
    ) {
        let _ = staging::remove_staging(&staging_result.staging_path);
        let _ = db.remove_mod(mod_id);
        let _ = app.emit(
            INSTALL_PROGRESS_EVENT,
            InstallProgress::ModFailed {
                mod_index: 0,
                mod_name: name.clone(),
                error: format!("Deploy failed: {}", e),
            },
        );
        return Err(format!("Deploy failed: {}", e));
    }

    // Step 5: Sync plugins
    if game_id == "skyrimse" {
        let _ = app.emit(
            INSTALL_PROGRESS_EVENT,
            InstallProgress::StepChanged {
                mod_index: 0,
                step: "syncing-plugins".to_string(),
                detail: Some("Syncing plugin load order...".to_string()),
            },
        );
        let _ = sync_skyrim_plugins_for_game(&game, &bottle);
    }

    // Set source type if provided
    if let Some(ref st) = source_type {
        let _ = db.set_mod_source(mod_id, st, source_url.as_deref());
    }

    // Emit: mod completed
    let _ = app.emit(
        INSTALL_PROGRESS_EVENT,
        InstallProgress::ModCompleted {
            mod_index: 0,
            mod_name: name,
            mod_id,
        },
    );

    db.get_mod(mod_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Failed to retrieve installed mod".to_string())
}

#[tauri::command]
fn uninstall_mod(
    mod_id: i64,
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<Vec<String>, String> {
    let db = &state.db;

    let installed_mod = db
        .get_mod(mod_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Mod with ID {} not found", mod_id))?;

    let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;

    // Remove deployed files from game directory
    let removed = if installed_mod.staging_path.is_some() {
        // Staged mod: undeploy via deployment manifest
        deployer::undeploy_mod(db, &game_id, &bottle_name, mod_id, &data_dir)
            .map_err(|e| e.to_string())?
    } else {
        // Legacy mod: remove files directly
        installer::uninstall_mod_files(&data_dir, &installed_mod.installed_files)
            .map_err(|e| e.to_string())?
    };

    // Clean orphaned rollback staging directories before DB removal
    let _ = rollback::cleanup_mod_version_staging(db, mod_id);

    // Remove staging directory if it exists
    if let Some(ref staging_path) = installed_mod.staging_path {
        let _ = staging::remove_staging(Path::new(staging_path));
    }

    // Remove from database (cascades to deployment_manifest, file_hashes; cleans profile_mods)
    db.remove_mod(mod_id).map_err(|e| e.to_string())?;

    // Sync Skyrim plugins if applicable
    if game_id == "skyrimse" {
        let _ = sync_skyrim_plugins_for_game(&game, &bottle);
    }

    Ok(removed)
}

#[tauri::command]
fn toggle_mod(
    mod_id: i64,
    game_id: String,
    bottle_name: String,
    enabled: bool,
    state: State<AppState>,
) -> Result<(), String> {
    let db = &state.db;

    let installed_mod = db
        .get_mod(mod_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Mod with ID {} not found", mod_id))?;

    // Update DB flag
    db.set_enabled(mod_id, enabled).map_err(|e| e.to_string())?;

    // For staged mods, actually deploy/undeploy files
    if let Some(ref staging_path_str) = installed_mod.staging_path {
        let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;
        let staging_path = PathBuf::from(staging_path_str);

        if enabled {
            // Re-deploy from staging
            let files = staging::list_staging_files(&staging_path).map_err(|e| e.to_string())?;
            deployer::deploy_mod(
                db,
                &game_id,
                &bottle_name,
                mod_id,
                &staging_path,
                &data_dir,
                &files,
            )
            .map_err(|e| e.to_string())?;
        } else {
            // Undeploy (remove from game dir, keep staging intact)
            deployer::undeploy_mod(db, &game_id, &bottle_name, mod_id, &data_dir)
                .map_err(|e| e.to_string())?;
        }

        // Sync Skyrim plugins if applicable
        if game_id == "skyrimse" {
            let _ = sync_skyrim_plugins_for_game(&game, &bottle);
        }
    }
    // Legacy mods (no staging_path): only the DB flag changes

    Ok(())
}

#[tauri::command]
fn get_plugin_order(game_id: String, bottle_name: String) -> Result<Vec<PluginEntry>, String> {
    if game_id != "skyrimse" {
        return Ok(vec![]);
    }

    let (bottle, game, _) = resolve_game(&game_id, &bottle_name)?;

    // Get plugins file path via the plugin
    let plugins_file = games::with_plugin(&game_id, |plugin| {
        plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
    })
    .flatten()
    .ok_or_else(|| "Could not determine plugins file location".to_string())?;

    if !plugins_file.exists() {
        return Ok(vec![]);
    }

    plugins::skyrim_plugins::read_plugins_txt(&plugins_file).map_err(|e| e.to_string())
}

#[tauri::command]
async fn download_from_nexus(
    nxm_url: String,
    game_id: String,
    bottle_name: String,
    auto_install: bool,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let cfg = config::get_config().map_err(|e| e.to_string())?;
    let api_key = cfg
        .nexus_api_key
        .ok_or_else(|| "No Nexus API key configured. Set it in Settings.".to_string())?;

    let nxm = nexus::NXMLink::parse(&nxm_url).map_err(|e| e.to_string())?;

    let client = nexus::NexusClient::new(api_key);

    // Get mod info
    let mod_info = client
        .get_mod(&nxm.game_slug, nxm.mod_id)
        .await
        .map_err(|e| e.to_string())?;
    let mod_name = mod_info
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown Mod")
        .to_string();
    let mod_version = mod_info
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Download
    let download_dir = cfg
        .download_dir
        .map(PathBuf::from)
        .unwrap_or_else(config::downloads_dir);

    let archive_path = client
        .download_from_nxm(&nxm, &download_dir, None::<Box<dyn Fn(u64, u64) + Send>>)
        .await
        .map_err(|e| e.to_string())?;

    if auto_install {
        let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;
        let db = &state.db;

        // 1. Add mod to DB with Nexus ID
        let next_priority = db
            .get_next_priority(&game_id, &bottle_name)
            .map_err(|e| e.to_string())?;
        let mod_id = db
            .add_mod(
                &game_id,
                &bottle_name,
                Some(nxm.mod_id),
                &mod_name,
                &mod_version,
                &archive_path.to_string_lossy(),
                &[],
            )
            .map_err(|e| e.to_string())?;
        db.set_mod_priority(mod_id, next_priority)
            .map_err(|e| e.to_string())?;

        // 2. Stage
        let staging_result =
            match staging::stage_mod(&archive_path, &game_id, &bottle_name, mod_id, &mod_name) {
                Ok(r) => r,
                Err(e) => {
                    let _ = db.remove_mod(mod_id);
                    return Err(format!("Staging failed: {}", e));
                }
            };

        // 3. Update DB
        db.set_staging_path(mod_id, &staging_result.staging_path.to_string_lossy())
            .map_err(|e| e.to_string())?;
        db.update_installed_files(mod_id, &staging_result.files)
            .map_err(|e| e.to_string())?;
        db.store_file_hashes(mod_id, &staging_result.hashes)
            .map_err(|e| e.to_string())?;

        // 4. Deploy
        if let Err(e) = deployer::deploy_mod(
            db,
            &game_id,
            &bottle_name,
            mod_id,
            &staging_result.staging_path,
            &data_dir,
            &staging_result.files,
        ) {
            let _ = staging::remove_staging(&staging_result.staging_path);
            let _ = db.remove_mod(mod_id);
            return Err(format!("Deploy failed: {}", e));
        }

        if game_id == "skyrimse" {
            let _ = sync_skyrim_plugins_for_game(&game, &bottle);
        }

        // Auto-delete archive if setting is enabled
        if cfg
            .extra
            .get("auto_delete_archives")
            .and_then(|v| v.as_str())
            == Some("true")
        {
            let _ = std::fs::remove_file(&archive_path);
        }

        let installed = db
            .get_mod(mod_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "Failed to retrieve installed mod".to_string())?;

        Ok(serde_json::to_value(installed).map_err(|e| e.to_string())?)
    } else {
        Ok(serde_json::json!({
            "downloaded": archive_path.to_string_lossy(),
            "mod_name": mod_name,
            "mod_version": mod_version,
        }))
    }
}

/// Check if the current user has Nexus Mods premium/supporter status.
/// Used by the frontend to determine download workflows.
#[tauri::command]
async fn is_nexus_premium() -> Result<bool, String> {
    let method = oauth::get_auth_method();
    match method {
        oauth::AuthMethod::ApiKey(key) => {
            let client = nexus::NexusClient::new(key);
            Ok(client.is_premium().await)
        }
        oauth::AuthMethod::OAuth(tokens) => {
            let user = oauth::parse_user_info(&tokens.access_token).map_err(|e| e.to_string())?;
            Ok(user.is_premium)
        }
        oauth::AuthMethod::None => Ok(false),
    }
}

#[tauri::command]
fn get_config() -> Result<AppConfig, String> {
    config::get_config().map_err(|e| e.to_string())
}

#[tauri::command]
fn set_config_value(key: String, value: String) -> Result<(), String> {
    config::set_config_value(&key, &value).map_err(|e| e.to_string())
}

// --- Download Archive Management ---

#[tauri::command]
fn list_download_archives() -> Result<Vec<serde_json::Value>, String> {
    let cfg = config::get_config().map_err(|e| e.to_string())?;
    let dir = cfg
        .download_dir
        .map(PathBuf::from)
        .unwrap_or_else(config::downloads_dir);

    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut archives = Vec::new();
    let entries = std::fs::read_dir(&dir).map_err(|e| e.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();

        // Only include archive files
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if !["zip", "7z", "rar", "gz", "tar"].contains(&ext.as_str()) {
            continue;
        }

        let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        archives.push(serde_json::json!({
            "filename": path.file_name().unwrap_or_default().to_string_lossy(),
            "path": path.to_string_lossy(),
            "size_bytes": metadata.len(),
            "modified_at": modified,
        }));
    }

    // Sort newest first
    archives.sort_by(|a, b| {
        let a_time = a["modified_at"].as_u64().unwrap_or(0);
        let b_time = b["modified_at"].as_u64().unwrap_or(0);
        b_time.cmp(&a_time)
    });

    Ok(archives)
}

#[tauri::command]
fn delete_download_archive(path: String) -> Result<(), String> {
    let archive_path = PathBuf::from(&path);
    if !archive_path.exists() {
        return Err("File not found".to_string());
    }
    // Safety: canonicalize to resolve symlinks before checking containment
    let canonical_archive = archive_path
        .canonicalize()
        .map_err(|e| format!("Cannot resolve path: {e}"))?;
    let cfg = config::get_config().map_err(|e| e.to_string())?;
    let downloads = cfg
        .download_dir
        .map(PathBuf::from)
        .unwrap_or_else(config::downloads_dir);
    let canonical_downloads = downloads
        .canonicalize()
        .map_err(|e| format!("Invalid downloads directory: {e}"))?;
    if !canonical_archive.starts_with(&canonical_downloads) {
        return Err("Cannot delete files outside the downloads directory".to_string());
    }
    // Only delete regular files, not directories or symlinks
    if !canonical_archive.is_file() {
        return Err("Path is not a regular file".to_string());
    }
    std::fs::remove_file(&canonical_archive).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_downloads_stats() -> Result<serde_json::Value, String> {
    let cfg = config::get_config().map_err(|e| e.to_string())?;
    let dir = cfg
        .download_dir
        .map(PathBuf::from)
        .unwrap_or_else(config::downloads_dir);

    if !dir.exists() {
        return Ok(serde_json::json!({
            "total_size_bytes": 0,
            "archive_count": 0,
            "directory": dir.to_string_lossy(),
        }));
    }

    let mut total_size: u64 = 0;
    let mut count: u64 = 0;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if ["zip", "7z", "rar", "gz", "tar"].contains(&ext.as_str()) {
                if let Ok(meta) = std::fs::metadata(&path) {
                    total_size += meta.len();
                    count += 1;
                }
            }
        }
    }

    Ok(serde_json::json!({
        "total_size_bytes": total_size,
        "archive_count": count,
        "directory": dir.to_string_lossy(),
    }))
}

#[tauri::command]
fn clear_all_download_archives() -> Result<u64, String> {
    let cfg = config::get_config().map_err(|e| e.to_string())?;
    let dir = cfg
        .download_dir
        .map(PathBuf::from)
        .unwrap_or_else(config::downloads_dir);

    if !dir.exists() {
        return Ok(0);
    }

    let mut deleted = 0u64;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if ["zip", "7z", "rar", "gz", "tar"].contains(&ext.as_str())
                && std::fs::remove_file(&path).is_ok()
            {
                deleted += 1;
            }
        }
    }

    Ok(deleted)
}

/// Fetch a transparent game logo PNG from Steam CDN and cache it locally.
/// Returns a base64-encoded data URL, or null if unavailable.
/// The PNG is cached on disk so subsequent calls are instant.
#[tauri::command]
async fn get_game_logo(game_id: String) -> Result<Option<String>, String> {
    use std::collections::HashMap;

    // Steam App IDs for known games
    let steam_ids: HashMap<&str, u32> = HashMap::from([
        ("skyrimse", 489830),
        ("skyrim", 72850),
        ("fallout4", 377160),
        ("falloutnv", 22380),
        ("fallout3", 22300),
        ("oblivion", 22330),
        ("morrowind", 22320),
        ("starfield", 1716740),
        ("enderal", 933480),
        ("cyberpunk2077", 1091500),
        ("baldursgate3", 1086940),
        ("witcher3", 292030),
    ]);

    let app_id = match steam_ids.get(game_id.as_str()) {
        Some(id) => *id,
        None => return Ok(None),
    };

    let logo_dir = config::cache_dir().join("game-logos");
    let cached_path = logo_dir.join(format!("{game_id}.png"));

    // Return cached version if it exists (instant — no network)
    if cached_path.exists() {
        let bytes = std::fs::read(&cached_path).map_err(|e| e.to_string())?;
        let b64 = base64_encode(&bytes);
        return Ok(Some(format!("data:image/png;base64,{b64}")));
    }

    // Fetch from Steam CDN
    let url = format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{app_id}/logo.png");

    let client = reqwest::Client::builder()
        .user_agent(format!("Corkscrew/{}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client.get(&url).send().await.map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        return Ok(None);
    }

    let bytes = response.bytes().await.map_err(|e| e.to_string())?;

    // Verify it's actually a PNG (starts with PNG magic bytes)
    if bytes.len() < 8 || &bytes[..4] != b"\x89PNG" {
        return Ok(None);
    }

    // Cache to disk for next time
    std::fs::create_dir_all(&logo_dir).map_err(|e| e.to_string())?;
    std::fs::write(&cached_path, &bytes).map_err(|e| e.to_string())?;

    let b64 = base64_encode(&bytes);
    Ok(Some(format!("data:image/png;base64,{b64}")))
}

/// Simple base64 encoder (avoids adding a dependency).
fn base64_encode(input: &[u8]) -> String {
    const CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 2 < input.len() {
        let b0 = input[i] as u32;
        let b1 = input[i + 1] as u32;
        let b2 = input[i + 2] as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        out.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        out.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        out.push(CHARS[(triple & 0x3F) as usize] as char);
        i += 3;
    }
    match input.len() - i {
        2 => {
            let b0 = input[i] as u32;
            let b1 = input[i + 1] as u32;
            let triple = (b0 << 16) | (b1 << 8);
            out.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
            out.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
            out.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
            out.push('=');
        }
        1 => {
            let b0 = input[i] as u32;
            let triple = b0 << 16;
            out.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
            out.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
            out.push('=');
            out.push('=');
        }
        _ => {}
    }
    out
}

#[tauri::command]
fn launch_game_cmd(
    game_id: String,
    bottle_name: String,
    use_skse: bool,
    state: State<AppState>,
) -> Result<LaunchResult, String> {
    let (bottle, game, _) = resolve_game(&game_id, &bottle_name)?;
    let game_path = PathBuf::from(&game.game_path);

    // Check for a custom default executable first
    let custom_exe =
        executables::get_default_executable(&state.db, &game_id, &bottle_name).unwrap_or(None);

    if let Some(custom) = custom_exe {
        let exe_path = PathBuf::from(&custom.exe_path);
        let work_dir = custom.working_dir.as_deref().map(Path::new);

        log::info!(
            "launch_game_cmd: using custom exe '{}' at {}",
            custom.name,
            exe_path.display()
        );

        return launcher::launch_game(&bottle, &exe_path, work_dir.or(Some(&game_path)))
            .map_err(|e| format!("Launch failed ({}): {}", bottle.source, e));
    }

    // Determine which built-in executable to launch
    let exe_name = if use_skse && game_id == "skyrimse" {
        "skse64_loader.exe".to_string()
    } else {
        games::with_plugin(&game_id, |plugin| {
            plugin
                .executables()
                .first()
                .map(|s| s.to_string())
                .unwrap_or_default()
        })
        .unwrap_or_default()
    };

    if exe_name.is_empty() {
        return Err(format!(
            "No executable configured for game '{}'. Cannot launch.",
            game_id
        ));
    }

    let exe_path = launcher::find_executable(&game_path, &exe_name).ok_or_else(|| {
        if use_skse {
            format!(
                "SKSE loader '{}' not found in {}. Is SKSE installed?",
                exe_name,
                game_path.display()
            )
        } else {
            format!(
                "Game executable '{}' not found in {}",
                exe_name,
                game_path.display()
            )
        }
    })?;

    log::info!(
        "launch_game_cmd: source={} bottle={} exe={} use_skse={}",
        bottle.source,
        bottle.name,
        exe_path.display(),
        use_skse
    );

    // Auto-apply display fix for Skyrim SE before launching to ensure fullscreen
    if game_id == "skyrimse" {
        match display_fix::auto_fix_display(&bottle) {
            Ok(result) => {
                if result.fixed {
                    log::info!(
                        "Auto-applied display fix: {}x{} fullscreen (was {}x{} fs={} borderless={})",
                        result.applied.width, result.applied.height,
                        result.previous.width, result.previous.height,
                        result.previous.fullscreen, result.previous.borderless
                    );
                } else {
                    log::debug!("Display settings already correct, no fix needed");
                }
            }
            Err(e) => {
                log::warn!("Could not auto-fix display settings: {}", e);
            }
        }
    }

    launcher::launch_game(&bottle, &exe_path, Some(&game_path))
        .map_err(|e| format!("Launch failed ({}): {}", bottle.source, e))
}

#[tauri::command]
fn check_skse(game_id: String, bottle_name: String) -> Result<SkseStatus, String> {
    if game_id != "skyrimse" {
        return Ok(SkseStatus {
            installed: false,
            loader_path: None,
            version: None,
            use_skse: false,
        });
    }

    let (_, game, _) = resolve_game(&game_id, &bottle_name)?;
    let game_path = PathBuf::from(&game.game_path);
    let mut status = skse::detect_skse(&game_path);
    status.use_skse = skse::get_skse_preference(&game_id, &bottle_name);

    Ok(status)
}

#[tauri::command]
fn get_skse_download_url() -> String {
    skse::skse_download_url().to_string()
}

#[tauri::command]
fn install_skse_from_archive_cmd(
    game_id: String,
    bottle_name: String,
    archive_path: String,
) -> Result<SkseStatus, String> {
    if game_id != "skyrimse" {
        return Err("SKSE is only available for Skyrim Special Edition".to_string());
    }

    let (_, game, _) = resolve_game(&game_id, &bottle_name)?;
    let game_path = PathBuf::from(&game.game_path);
    let archive = PathBuf::from(&archive_path);

    let mut status =
        skse::install_skse_from_archive(&game_path, &archive).map_err(|e| e.to_string())?;

    // Auto-enable SKSE after successful installation
    if status.installed {
        let _ = skse::set_skse_preference(&game_id, &bottle_name, true);
        status.use_skse = true;
    }

    Ok(status)
}

#[tauri::command]
fn set_skse_preference_cmd(
    game_id: String,
    bottle_name: String,
    enabled: bool,
) -> Result<(), String> {
    skse::set_skse_preference(&game_id, &bottle_name, enabled).map_err(|e| e.to_string())
}

#[tauri::command]
fn check_skyrim_version(game_id: String, bottle_name: String) -> Result<DowngradeStatus, String> {
    if game_id != "skyrimse" {
        return Err("Version check is only available for Skyrim SE".to_string());
    }

    let (_, game, _) = resolve_game(&game_id, &bottle_name)?;
    downgrader::detect_skyrim_version(Path::new(&game.game_path)).map_err(|e| e.to_string())
}

#[tauri::command]
fn check_skse_compatibility_cmd(
    game_id: String,
    bottle_name: String,
) -> Result<skse::SkseCompatibility, String> {
    if game_id != "skyrimse" {
        return Err("SKSE compatibility check is only for Skyrim SE".into());
    }

    let (_, game, _) = resolve_game(&game_id, &bottle_name)?;
    let game_path = PathBuf::from(&game.game_path);
    let skse_status = skse::detect_skse(&game_path);
    let downgrade_status =
        downgrader::detect_skyrim_version(&game_path).map_err(|e| e.to_string())?;

    Ok(skse::check_skse_compatibility(
        &skse_status,
        &downgrade_status,
    ))
}

#[tauri::command]
fn get_skse_builds(
    game_id: String,
    bottle_name: String,
) -> Result<skse::SkseAvailableBuilds, String> {
    if game_id != "skyrimse" {
        return Err("SKSE is only available for Skyrim Special Edition".into());
    }

    let (_, game, _) = resolve_game(&game_id, &bottle_name)?;
    let game_path = PathBuf::from(&game.game_path);
    let downgrade_status =
        downgrader::detect_skyrim_version(&game_path).map_err(|e| e.to_string())?;

    Ok(skse::get_available_skse_builds(
        &downgrade_status.current_version,
    ))
}

#[tauri::command]
async fn install_skse_auto_cmd(game_id: String, bottle_name: String) -> Result<SkseStatus, String> {
    if game_id != "skyrimse" {
        return Err("SKSE is only available for Skyrim Special Edition".into());
    }

    let (_, game, _) = resolve_game(&game_id, &bottle_name)?;
    let game_path = PathBuf::from(&game.game_path);
    let downgrade_status =
        downgrader::detect_skyrim_version(&game_path).map_err(|e| e.to_string())?;

    let mut status = skse::install_skse_auto(&game_path, &downgrade_status.current_version)
        .await
        .map_err(|e| e.to_string())?;

    if status.installed {
        let _ = skse::set_skse_preference(&game_id, &bottle_name, true);
        status.use_skse = true;
    }

    Ok(status)
}

#[tauri::command]
fn fix_skyrim_display(bottle_name: String) -> Result<display_fix::DisplayFixResult, String> {
    let bottle = resolve_bottle(&bottle_name)?;
    display_fix::auto_fix_display(&bottle)
}

#[tauri::command]
async fn downgrade_skyrim(
    game_id: String,
    bottle_name: String,
    _mode: String,
) -> Result<DowngradeStatus, String> {
    if game_id != "skyrimse" {
        return Err("Downgrade is only available for Skyrim SE".to_string());
    }

    let (_, game, _) = resolve_game(&game_id, &bottle_name)?;
    let game_path = PathBuf::from(&game.game_path);
    let download_dir = config::get_config()
        .ok()
        .and_then(|c| c.download_dir.map(PathBuf::from))
        .unwrap_or_else(config::downloads_dir);

    // Create a downgrade copy of the game files
    let downgrade_dir = download_dir
        .parent()
        .unwrap_or(&download_dir)
        .join("downgraded_games");
    let downgrade_path =
        downgrader::create_downgrade_copy(&game_path, &downgrade_dir).map_err(|e| e.to_string())?;

    // Store downgrade path in config
    let config_key = format!("downgrade:{}:{}", game_id, bottle_name);
    let _ = config::set_config_value(&config_key, &downgrade_path.to_string_lossy());

    // Return status (actual USSEDP patching is a future enhancement)
    downgrader::detect_skyrim_version(&downgrade_path).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_vibrancy(window: tauri::Window, material: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState};
        let mat = match material.as_str() {
            "sidebar" => NSVisualEffectMaterial::Sidebar,
            "underWindowBackground" => NSVisualEffectMaterial::UnderWindowBackground,
            "contentBackground" => NSVisualEffectMaterial::ContentBackground,
            "hudWindow" => NSVisualEffectMaterial::HudWindow,
            _ => NSVisualEffectMaterial::UnderWindowBackground,
        };
        apply_vibrancy(
            &window,
            mat,
            Some(NSVisualEffectState::FollowsWindowActiveState),
            None,
        )
        .map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, material);
    }
    Ok(())
}

// --- Custom Executables ---

#[tauri::command]
fn add_custom_exe(
    game_id: String,
    bottle_name: String,
    name: String,
    exe_path: String,
    working_dir: Option<String>,
    args: Option<String>,
    state: State<AppState>,
) -> Result<i64, String> {
    let db = &state.db;
    executables::add_executable(
        db,
        &game_id,
        &bottle_name,
        &name,
        &exe_path,
        working_dir.as_deref(),
        args.as_deref(),
    )
}

#[tauri::command]
fn remove_custom_exe(exe_id: i64, state: State<AppState>) -> Result<(), String> {
    let db = &state.db;
    executables::remove_executable(db, exe_id)
}

#[tauri::command]
fn list_custom_exes(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<Vec<CustomExecutable>, String> {
    let db = &state.db;
    executables::list_executables(db, &game_id, &bottle_name)
}

#[tauri::command]
fn set_default_exe(
    game_id: String,
    bottle_name: String,
    exe_id: Option<i64>,
    state: State<AppState>,
) -> Result<(), String> {
    let db = &state.db;
    match exe_id {
        Some(id) => executables::set_default_executable(db, &game_id, &bottle_name, id),
        None => executables::clear_default_executable(db, &game_id, &bottle_name),
    }
}

// --- Deployment Management ---

#[tauri::command]
fn get_conflicts(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<Vec<FileConflict>, String> {
    let db = &state.db;
    db.find_all_conflicts(&game_id, &bottle_name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn analyze_conflicts_cmd(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<Vec<conflict_resolver::ConflictSuggestion>, String> {
    let db = &state.db;
    let conflicts = db
        .find_all_conflicts(&game_id, &bottle_name)
        .map_err(|e| e.to_string())?;
    let mods = db
        .list_mods(&game_id, &bottle_name)
        .map_err(|e| e.to_string())?;

    // Try to get LOOT sort order for smarter suggestions.
    let loot_order = get_current_plugins(&game_id, &bottle_name);
    let loot_names: Vec<String> = loot_order.iter().map(|p| p.filename.clone()).collect();
    let loot_ref = if loot_names.is_empty() {
        None
    } else {
        Some(loot_names.as_slice())
    };

    Ok(conflict_resolver::analyze_conflicts(
        &conflicts, &mods, loot_ref,
    ))
}

#[tauri::command]
fn resolve_all_conflicts_cmd(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<conflict_resolver::ResolutionResult, String> {
    let db = &state.db;
    let conflicts = db
        .find_all_conflicts(&game_id, &bottle_name)
        .map_err(|e| e.to_string())?;
    let mods = db
        .list_mods(&game_id, &bottle_name)
        .map_err(|e| e.to_string())?;

    let loot_order = get_current_plugins(&game_id, &bottle_name);
    let loot_names: Vec<String> = loot_order.iter().map(|p| p.filename.clone()).collect();
    let loot_ref = if loot_names.is_empty() {
        None
    } else {
        Some(loot_names.as_slice())
    };

    let suggestions = conflict_resolver::analyze_conflicts(&conflicts, &mods, loot_ref);
    let result = conflict_resolver::apply_suggestions(db, &game_id, &bottle_name, &suggestions)?;

    // Record conflict rules for resolved conflicts so they disappear from the list.
    for suggestion in &suggestions {
        match suggestion.status {
            conflict_resolver::ConflictStatus::AuthorResolved => {
                let winner = suggestion.current_winner_id;
                for m in &suggestion.mods {
                    if m.mod_id != winner {
                        let _ = db.add_conflict_rule(&game_id, &bottle_name, winner, m.mod_id);
                    }
                }
            }
            conflict_resolver::ConflictStatus::Suggested => {
                let winner = suggestion.suggested_winner_id;
                for m in &suggestion.mods {
                    if m.mod_id != winner {
                        let _ = db.add_conflict_rule(&game_id, &bottle_name, winner, m.mod_id);
                    }
                }
            }
            conflict_resolver::ConflictStatus::Manual => {}
        }
    }

    // Redeploy to apply new priorities if any changed.
    if result.priorities_changed > 0 {
        let (_bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;
        deployer::redeploy_all(db, &game_id, &bottle_name, &data_dir).map_err(|e| e.to_string())?;
        if game_id == "skyrimse" {
            let bottle = resolve_bottle(&bottle_name)?;
            let _ = sync_skyrim_plugins_for_game(&game, &bottle);
        }
    }

    Ok(result)
}

#[tauri::command]
fn record_conflict_winner(
    game_id: String,
    bottle_name: String,
    winner_mod_id: i64,
    loser_mod_ids: Vec<i64>,
    state: State<AppState>,
) -> Result<(), String> {
    let db = &state.db;
    for loser_id in loser_mod_ids {
        db.add_conflict_rule(&game_id, &bottle_name, winner_mod_id, loser_id)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn get_deployment_manifest_cmd(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<Vec<DeploymentEntry>, String> {
    let db = &state.db;
    db.get_deployment_manifest(&game_id, &bottle_name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn set_mod_priority(mod_id: i64, priority: i32, state: State<AppState>) -> Result<(), String> {
    let db = &state.db;
    db.set_mod_priority(mod_id, priority)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn reorder_mods(
    game_id: String,
    bottle_name: String,
    ordered_mod_ids: Vec<i64>,
    state: State<AppState>,
) -> Result<(), String> {
    let db = &state.db;
    db.reorder_priorities(&game_id, &bottle_name, &ordered_mod_ids)
        .map_err(|e| e.to_string())?;

    let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;

    // Redeploy to reflect new priority order
    deployer::redeploy_all(db, &game_id, &bottle_name, &data_dir).map_err(|e| e.to_string())?;

    // Sync plugins after redeploy
    if game_id == "skyrimse" {
        let _ = sync_skyrim_plugins_for_game(&game, &bottle);
    }

    Ok(())
}

#[tauri::command]
fn redeploy_all_mods(
    app: AppHandle,
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<serde_json::Value, String> {
    let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;
    let db = &state.db;

    let app_clone = app.clone();
    let result = deployer::redeploy_all_with_progress(
        db,
        &game_id,
        &bottle_name,
        &data_dir,
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
    .map_err(|e| e.to_string())?;

    if game_id == "skyrimse" {
        let _ = sync_skyrim_plugins_for_game(&game, &bottle);
    }

    Ok(serde_json::json!({
        "deployed_count": result.deployed_count,
        "skipped_count": result.skipped_count,
        "fallback_used": result.fallback_used,
    }))
}

#[tauri::command]
fn purge_deployment_cmd(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<Vec<String>, String> {
    let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;
    let db = &state.db;

    let removed = deployer::purge_deployment(db, &game_id, &bottle_name, &data_dir)
        .map_err(|e| e.to_string())?;

    if game_id == "skyrimse" {
        let _ = sync_skyrim_plugins_for_game(&game, &bottle);
    }

    Ok(removed)
}

#[tauri::command]
fn verify_mod_integrity(mod_id: i64, state: State<AppState>) -> Result<Vec<String>, String> {
    let db = &state.db;

    let installed_mod = db
        .get_mod(mod_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Mod with ID {} not found", mod_id))?;

    let staging_path = installed_mod
        .staging_path
        .as_ref()
        .ok_or_else(|| "Legacy mod — no staging data for integrity check".to_string())?;

    let hashes = db.get_file_hashes(mod_id).map_err(|e| e.to_string())?;
    staging::verify_staging_integrity(Path::new(staging_path), &hashes).map_err(|e| e.to_string())
}

// --- Deployment Health ---

#[tauri::command]
fn get_deployment_health(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<serde_json::Value, String> {
    let db = &state.db;

    let manifest = db
        .get_deployment_manifest(&game_id, &bottle_name)
        .map_err(|e| e.to_string())?;
    let mods = db
        .list_mods(&game_id, &bottle_name)
        .map_err(|e| e.to_string())?;
    let conflicts = db
        .find_all_conflicts(&game_id, &bottle_name)
        .map_err(|e| e.to_string())?;

    let total_mods = mods.len();
    let total_enabled = mods.iter().filter(|m| m.enabled).count();
    let is_deployed = !manifest.is_empty();
    let total_deployed = manifest.len();
    let conflict_count = conflicts.len();

    let deploy_method = if is_deployed {
        match resolve_game(&game_id, &bottle_name) {
            Ok((_, _, data_dir)) => {
                // Test actual staging→data_dir filesystem match (not same-dir)
                let staging_root = staging::staging_base_dir(&game_id, &bottle_name);
                if deployer::same_filesystem(&staging_root, &data_dir) {
                    "hardlink"
                } else {
                    "copy"
                }
            }
            Err(_) => "unknown",
        }
    } else {
        "none"
    };

    Ok(serde_json::json!({
        "total_deployed": total_deployed,
        "total_enabled": total_enabled,
        "total_mods": total_mods,
        "conflict_count": conflict_count,
        "deploy_method": deploy_method,
        "is_deployed": is_deployed,
    }))
}

// --- Collection Management ---

#[tauri::command]
fn list_installed_collections_cmd(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<Vec<CollectionSummary>, String> {
    let db = &state.db;
    let collections = db
        .list_installed_collections(&game_id, &bottle_name)
        .map_err(|e| e.to_string())?;
    let metadata_list = db
        .list_collection_metadata(&game_id, &bottle_name)
        .unwrap_or_default();
    Ok(collections
        .into_iter()
        .map(|(name, mod_count, enabled_count)| {
            let meta = metadata_list.iter().find(|m| m.collection_name == name);
            CollectionSummary {
                name,
                mod_count,
                enabled_count,
                slug: meta.and_then(|m| m.slug.clone()),
                author: meta.and_then(|m| m.author.clone()),
                image_url: meta.and_then(|m| m.image_url.clone()),
                game_domain: meta.and_then(|m| m.game_domain.clone()),
                installed_revision: meta.and_then(|m| m.installed_revision),
            }
        })
        .collect())
}

#[tauri::command]
fn set_mod_collection_name_cmd(
    mod_id: i64,
    collection_name: String,
    state: State<AppState>,
) -> Result<(), String> {
    state.db.set_collection_name(mod_id, &collection_name).map_err(|e| e.to_string())
}

#[tauri::command]
fn switch_collection_cmd(
    game_id: String,
    bottle_name: String,
    collection_name: String,
    state: State<AppState>,
) -> Result<serde_json::Value, String> {
    let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;
    let db = &state.db;

    // 1. Purge current deployment
    deployer::purge_deployment(db, &game_id, &bottle_name, &data_dir).map_err(|e| e.to_string())?;

    // 2. Disable all mods for this game/bottle
    {
        let conn = db.conn().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE installed_mods SET enabled = 0 WHERE game_id = ?1 AND bottle_name = ?2",
            rusqlite::params![game_id, bottle_name],
        )
        .map_err(|e| e.to_string())?;
    }

    // 3. Enable mods belonging to the target collection
    {
        let conn = db.conn().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE installed_mods SET enabled = 1
             WHERE game_id = ?1 AND bottle_name = ?2 AND collection_name = ?3",
            rusqlite::params![game_id, bottle_name, collection_name],
        )
        .map_err(|e| e.to_string())?;
    }

    // 4. Redeploy
    let result =
        deployer::redeploy_all(db, &game_id, &bottle_name, &data_dir).map_err(|e| e.to_string())?;

    // 5. Sync plugins if Skyrim SE
    if game_id == "skyrimse" {
        let _ = sync_skyrim_plugins_for_game(&game, &bottle);
    }

    Ok(serde_json::json!({
        "deployed_count": result.deployed_count,
        "active_collection": collection_name,
    }))
}

#[tauri::command]
fn delete_collection_cmd(
    game_id: String,
    bottle_name: String,
    collection_name: String,
    delete_unique_downloads: bool,
    state: State<AppState>,
) -> Result<serde_json::Value, String> {
    let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;
    let db = &state.db;

    // Get mods in this collection
    let collection_mods = db
        .list_mods_by_collection(&game_id, &bottle_name, &collection_name)
        .map_err(|e| e.to_string())?;

    let mut mods_removed = 0usize;
    let mut downloads_removed = 0usize;
    let mut errors: Vec<String> = Vec::new();

    // Collect plugin filenames for rule cleanup
    let mut plugin_names: Vec<String> = Vec::new();

    for m in &collection_mods {
        // Gather plugin filenames before removal
        for file in &m.installed_files {
            let lower = file.to_lowercase();
            if lower.ends_with(".esp") || lower.ends_with(".esm") || lower.ends_with(".esl") {
                if let Some(fname) = Path::new(file).file_name().and_then(|f| f.to_str()) {
                    plugin_names.push(fname.to_string());
                }
            }
        }

        // Undeploy
        if let Err(e) = deployer::undeploy_mod(db, &game_id, &bottle_name, m.id, &data_dir) {
            errors.push(format!("Failed to undeploy '{}': {}", m.name, e));
        }

        // Clean orphaned rollback staging directories
        if let Err(e) = rollback::cleanup_mod_version_staging(db, m.id) {
            errors.push(format!("Failed to clean rollback staging for '{}': {}", m.name, e));
        }

        // Remove staging
        if let Some(sp) = &m.staging_path {
            if let Err(e) = std::fs::remove_dir_all(sp) {
                if Path::new(sp).exists() {
                    errors.push(format!("Failed to remove staging for '{}': {}", m.name, e));
                }
            }
        }

        // Find download record — try nexus IDs first, fall back to archive name
        let download = if let (Some(nmod_id), Some(nfile_id)) = (m.nexus_mod_id, m.nexus_file_id) {
            db.find_download_by_nexus_ids(nmod_id, nfile_id).ok().flatten()
        } else {
            None
        }
        .or_else(|| db.find_download_by_name(&m.archive_name).ok().flatten());

        if let Some(dl) = download {
            // Check uniqueness before removing the ref
            let is_unique = db
                .is_download_unique_to_collection(dl.id, &collection_name)
                .unwrap_or(false);

            // Optionally delete the actual archive file if unique to this collection
            if delete_unique_downloads && is_unique {
                if let Err(e) = std::fs::remove_file(&dl.archive_path) {
                    if Path::new(&dl.archive_path).exists() {
                        errors.push(format!("Failed to delete download for '{}': {}", m.name, e));
                    }
                } else {
                    downloads_removed += 1;
                }
            }

            // Always clean up the collection ref
            if let Err(e) = db.remove_download_collection_ref(
                dl.id,
                &collection_name,
                &game_id,
                &bottle_name,
            ) {
                errors.push(format!("Failed to remove download ref for '{}': {}", m.name, e));
            }
        }

        // Remove from DB (cascades deployment_manifest, file_hashes; also cleans profile_mods)
        if let Err(e) = db.remove_mod(m.id) {
            errors.push(format!("Failed to remove mod '{}' from DB: {}", m.name, e));
        } else {
            mods_removed += 1;
        }
    }

    // Clean orphaned download_registry rows
    if let Err(e) = db.cleanup_orphaned_downloads() {
        errors.push(format!("Failed to clean orphaned downloads: {}", e));
    }

    // Clean plugin rules for removed mods' plugins
    if !plugin_names.is_empty() {
        if let Err(e) =
            loot_rules::remove_rules_for_plugins(db, &game_id, &bottle_name, &plugin_names)
        {
            errors.push(format!("Failed to clean plugin rules: {}", e));
        }
    }

    // Clean up collection metadata
    if let Err(e) = db.remove_collection_metadata(&game_id, &bottle_name, &collection_name) {
        errors.push(format!("Failed to remove collection metadata: {}", e));
    }

    // Redeploy remaining mods to restore any files that were shadowed
    if let Err(e) = deployer::redeploy_all(db, &game_id, &bottle_name, &data_dir) {
        errors.push(format!("Failed to redeploy remaining mods: {}", e));
    }

    if game_id == "skyrimse" {
        let _ = sync_skyrim_plugins_for_game(&game, &bottle);
    }

    Ok(serde_json::json!({
        "mods_removed": mods_removed,
        "downloads_removed": downloads_removed,
        "errors": errors,
    }))
}

#[tauri::command]
async fn get_collection_diff_cmd(
    game_id: String,
    bottle_name: String,
    collection_name: String,
    state: State<'_, AppState>,
) -> Result<CollectionDiff, String> {
    let db = &state.db;

    // Load stored manifest from metadata
    let meta = db
        .get_collection_metadata(&game_id, &bottle_name, &collection_name)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("No metadata found for collection '{}'", collection_name))?;

    let manifest_json = meta
        .manifest_json
        .ok_or("No stored manifest for this collection")?;
    let manifest: CollectionManifest =
        serde_json::from_str(&manifest_json).map_err(|e| e.to_string())?;

    let slug = meta
        .slug
        .ok_or("Collection slug not stored — cannot fetch latest revision")?;

    let game_domain = meta
        .game_domain
        .unwrap_or_else(|| "skyrimspecialedition".to_string());

    // Load config to get API key
    let cfg = config::get_config().map_err(|e| e.to_string())?;
    let api_key = cfg.nexus_api_key.as_deref();

    // Get collection info to find latest revision number
    let info = collections::get_collection(api_key, &slug, &game_domain)
        .await
        .map_err(|e| e.to_string())?;

    let latest_revision = info.latest_revision;

    // Fetch mods from the latest revision
    let latest_mods = collections::get_revision_mods(api_key, &slug, latest_revision)
        .await
        .map_err(|e| e.to_string())?;

    // Compute diff
    Ok(collections::compute_diff(
        &collection_name,
        meta.installed_revision,
        latest_revision,
        &manifest.mods,
        &latest_mods,
    ))
}

// --- Notes & Tags ---

#[tauri::command]
fn set_mod_notes(mod_id: i64, notes: Option<String>, state: State<AppState>) -> Result<(), String> {
    let db = &state.db;
    db.set_user_notes(mod_id, notes.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn set_mod_source(
    mod_id: i64,
    source_type: String,
    source_url: Option<String>,
    state: State<AppState>,
) -> Result<(), String> {
    let db = &state.db;
    db.set_mod_source(mod_id, &source_type, source_url.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn set_mod_tags(mod_id: i64, tags: Vec<String>, state: State<AppState>) -> Result<(), String> {
    let db = &state.db;
    db.set_user_tags(mod_id, &tags).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_all_tags(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<Vec<String>, String> {
    let db = &state.db;
    db.get_all_user_tags(&game_id, &bottle_name)
        .map_err(|e| e.to_string())
}

// --- Auto-category ---

#[tauri::command]
fn backfill_categories(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<usize, String> {
    let db = &state.db;
    db.backfill_categories(&game_id, &bottle_name)
        .map_err(|e| e.to_string())
}

// --- Notification Log ---

#[tauri::command]
fn get_notification_log(
    limit: Option<usize>,
    state: State<AppState>,
) -> Result<Vec<database::NotificationEntry>, String> {
    let db = &state.db;
    db.get_notifications(limit.unwrap_or(50))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn clear_notification_log(state: State<AppState>) -> Result<(), String> {
    let db = &state.db;
    db.clear_notifications().map_err(|e| e.to_string())
}

#[tauri::command]
fn log_notification(
    level: String,
    message: String,
    detail: Option<String>,
    state: State<AppState>,
) -> Result<(), String> {
    let db = &state.db;
    db.log_notification(&level, &message, detail.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_notification_count(state: State<AppState>) -> Result<usize, String> {
    let db = &state.db;
    db.notification_count().map_err(|e| e.to_string())
}

// --- Download Queue ---

#[tauri::command]
fn get_download_queue(state: State<AppState>) -> Vec<download_queue::QueueItem> {
    state.download_queue.get_all()
}

#[tauri::command]
fn get_download_queue_counts(state: State<AppState>) -> download_queue::QueueCounts {
    state.download_queue.status_counts()
}

#[tauri::command]
fn retry_download(id: u64, state: State<AppState>) -> Result<bool, String> {
    Ok(state.download_queue.mark_for_retry(id))
}

#[tauri::command]
fn cancel_download(id: u64, state: State<AppState>) -> Result<(), String> {
    state.download_queue.set_cancelled(id);
    Ok(())
}

#[tauri::command]
fn clear_finished_downloads(state: State<AppState>) -> usize {
    state.download_queue.clear_finished()
}

// --- LOOT & Plugin Management ---

#[tauri::command]
async fn sort_plugins_loot(game_id: String, bottle_name: String) -> Result<SortResult, String> {
    let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;
    let game_path = PathBuf::from(&game.game_path);
    let local_path = loot::local_game_path(&bottle, &game_id)
        .ok_or_else(|| format!("Cannot determine local path for game '{}'", game_id))?;

    // Sort using LOOT
    let sort_result = loot::sort_plugins(&game_id, &game_path, &data_dir, &local_path)
        .map_err(|e| e.to_string())?;

    // Apply the sorted order to disk
    if sort_result.plugins_moved > 0 {
        let plugins_file = games::with_plugin(&game_id, |plugin| {
            plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
        })
        .flatten()
        .ok_or_else(|| "Could not determine plugins file location".to_string())?;

        let loadorder_file = plugins_file
            .parent()
            .map(|p| p.join("loadorder.txt"))
            .unwrap_or_else(|| plugins_file.with_file_name("loadorder.txt"));

        // Build PluginEntry list from sorted order, preserving enabled state
        let existing = if plugins_file.exists() {
            plugins::skyrim_plugins::read_plugins_txt(&plugins_file).unwrap_or_default()
        } else {
            Vec::new()
        };

        let enabled_map: std::collections::HashMap<String, bool> = existing
            .iter()
            .map(|e| (e.filename.to_lowercase(), e.enabled))
            .collect();

        let ordered_entries: Vec<PluginEntry> = sort_result
            .sorted_order
            .iter()
            .map(|name| PluginEntry {
                filename: name.clone(),
                enabled: enabled_map
                    .get(&name.to_lowercase())
                    .copied()
                    .unwrap_or(false),
            })
            .collect();

        plugins::skyrim_plugins::apply_load_order(&plugins_file, &loadorder_file, &ordered_entries)
            .map_err(|e| e.to_string())?;
    }

    Ok(sort_result)
}

#[tauri::command]
async fn update_loot_masterlist(game_id: String) -> Result<String, String> {
    loot::update_masterlist(&game_id)
        .await
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn reorder_plugins_cmd(
    game_id: String,
    bottle_name: String,
    ordered_plugins: Vec<String>,
) -> Result<Vec<PluginEntry>, String> {
    let (bottle, game, _) = resolve_game(&game_id, &bottle_name)?;

    let plugins_file = games::with_plugin(&game_id, |plugin| {
        plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
    })
    .flatten()
    .ok_or_else(|| "Could not determine plugins file location".to_string())?;

    let loadorder_file = plugins_file
        .parent()
        .map(|p| p.join("loadorder.txt"))
        .unwrap_or_else(|| plugins_file.with_file_name("loadorder.txt"));

    plugins::skyrim_plugins::reorder_plugins(&plugins_file, &loadorder_file, &ordered_plugins)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn toggle_plugin_cmd(
    game_id: String,
    bottle_name: String,
    plugin_name: String,
    enabled: bool,
) -> Result<Vec<PluginEntry>, String> {
    let (bottle, game, _) = resolve_game(&game_id, &bottle_name)?;

    let plugins_file = games::with_plugin(&game_id, |plugin| {
        plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
    })
    .flatten()
    .ok_or_else(|| "Could not determine plugins file location".to_string())?;

    let loadorder_file = plugins_file
        .parent()
        .map(|p| p.join("loadorder.txt"))
        .unwrap_or_else(|| plugins_file.with_file_name("loadorder.txt"));

    plugins::skyrim_plugins::toggle_plugin(&plugins_file, &loadorder_file, &plugin_name, enabled)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn move_plugin_cmd(
    game_id: String,
    bottle_name: String,
    plugin_name: String,
    new_index: usize,
) -> Result<Vec<PluginEntry>, String> {
    let (bottle, game, _) = resolve_game(&game_id, &bottle_name)?;

    let plugins_file = games::with_plugin(&game_id, |plugin| {
        plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
    })
    .flatten()
    .ok_or_else(|| "Could not determine plugins file location".to_string())?;

    let loadorder_file = plugins_file
        .parent()
        .map(|p| p.join("loadorder.txt"))
        .unwrap_or_else(|| plugins_file.with_file_name("loadorder.txt"));

    plugins::skyrim_plugins::move_plugin(&plugins_file, &loadorder_file, &plugin_name, new_index)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_plugin_messages(
    game_id: String,
    bottle_name: String,
    plugin_name: String,
) -> Result<Vec<PluginWarning>, String> {
    let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;

    let game_path = PathBuf::from(&game.game_path);
    let local_path = loot::local_game_path(&bottle, &game_id)
        .ok_or_else(|| format!("Cannot determine local path for game '{}'", game_id))?;

    loot::get_plugin_messages(&game_id, &game_path, &data_dir, &local_path, &plugin_name)
        .map_err(|e| e.to_string())
}

// --- Profiles ---

#[tauri::command]
fn list_profiles_cmd(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<Vec<Profile>, String> {
    let db = &state.db;
    profiles::list_profiles(db, &game_id, &bottle_name).map_err(|e| e.to_string())
}

#[tauri::command]
fn create_profile_cmd(
    game_id: String,
    bottle_name: String,
    name: String,
    state: State<AppState>,
) -> Result<i64, String> {
    let db = &state.db;
    profiles::create_profile(db, &game_id, &bottle_name, &name).map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_profile_cmd(profile_id: i64, state: State<AppState>) -> Result<(), String> {
    let db = &state.db;
    profiles::delete_profile(db, profile_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn rename_profile_cmd(
    profile_id: i64,
    new_name: String,
    state: State<AppState>,
) -> Result<(), String> {
    let db = &state.db;
    profiles::rename_profile(db, profile_id, &new_name).map_err(|e| e.to_string())
}

#[tauri::command]
fn save_profile_snapshot(
    profile_id: i64,
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<(), String> {
    let db = &state.db;

    // Determine plugins file path
    let plugins_file = if game_id == "skyrimse" {
        let (bottle, game, _) = resolve_game(&game_id, &bottle_name)?;

        games::with_plugin(&game_id, |plugin| {
            plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
        })
        .flatten()
    } else {
        None
    };

    profiles::snapshot_current_state(
        db,
        profile_id,
        &game_id,
        &bottle_name,
        plugins_file.as_deref(),
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn activate_profile(
    profile_id: i64,
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<(), String> {
    let db = &state.db;

    // Look up the game
    let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;

    // 1. Save current state to the currently active profile (if any)
    if let Ok(Some(current_active)) = profiles::get_active_profile(db, &game_id, &bottle_name) {
        let plugins_file = if game_id == "skyrimse" {
            games::with_plugin(&game_id, |plugin| {
                plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
            })
            .flatten()
        } else {
            None
        };

        let _ = profiles::snapshot_current_state(
            db,
            current_active.id,
            &game_id,
            &bottle_name,
            plugins_file.as_deref(),
        );
    }

    // 2. Purge current deployment
    let _ = deployer::purge_deployment(db, &game_id, &bottle_name, &data_dir);

    // 3. Load target profile state
    let mod_states = profiles::get_mod_states(db, profile_id).map_err(|e| e.to_string())?;

    // 4. Apply mod enabled states and priorities
    for ms in &mod_states {
        let _ = db.set_enabled(ms.mod_id, ms.enabled);
        let _ = db.set_mod_priority(ms.mod_id, ms.priority);
    }

    // 5. Redeploy enabled mods
    let _ = deployer::redeploy_all(db, &game_id, &bottle_name, &data_dir);

    // 6. Apply plugin states
    let plugin_states = profiles::get_plugin_states(db, profile_id).map_err(|e| e.to_string())?;

    if !plugin_states.is_empty() && game_id == "skyrimse" {
        let plugins_file = games::with_plugin(&game_id, |plugin| {
            plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
        })
        .flatten();

        if let Some(pf) = plugins_file {
            let loadorder_file = pf
                .parent()
                .map(|p| p.join("loadorder.txt"))
                .unwrap_or_else(|| pf.with_file_name("loadorder.txt"));

            let entries: Vec<PluginEntry> = plugin_states
                .iter()
                .map(|ps| PluginEntry {
                    filename: ps.plugin_filename.clone(),
                    enabled: ps.enabled,
                })
                .collect();

            let _ = plugins::skyrim_plugins::apply_load_order(&pf, &loadorder_file, &entries);
        }
    }

    // 7. Mark profile as active
    profiles::set_active_profile(db, &game_id, &bottle_name, profile_id)
        .map_err(|e| e.to_string())?;

    Ok(())
}

// --- Update Checking ---

#[tauri::command]
async fn check_mod_updates(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<ModUpdateInfo>, String> {
    let cfg = config::get_config().map_err(|e| e.to_string())?;
    let api_key = cfg
        .nexus_api_key
        .ok_or_else(|| "No Nexus API key configured".to_string())?;

    let mods = {
        let db = &state.db;
        db.list_mods(&game_id, &bottle_name)
            .map_err(|e| e.to_string())?
    };

    // Build query list from mods that have a nexus_mod_id
    let queries: Vec<nexus::ModUpdateQuery> = mods
        .iter()
        .filter_map(|m| {
            m.nexus_mod_id.map(|nid| nexus::ModUpdateQuery {
                local_mod_id: m.id,
                nexus_mod_id: nid,
                nexus_file_id: m.nexus_file_id,
                mod_name: m.name.clone(),
                current_version: m.version.clone(),
            })
        })
        .collect();

    if queries.is_empty() {
        return Ok(vec![]);
    }

    // Determine game slug from game_id
    let game_slug = match game_id.as_str() {
        "skyrimse" => "skyrimspecialedition",
        other => other,
    };

    let client = nexus::NexusClient::new(api_key);
    client
        .check_updates(game_slug, &queries)
        .await
        .map_err(|e| e.to_string())
}

// --- Mod Tools ---

#[tauri::command]
fn detect_mod_tools_cmd(
    game_id: String,
    bottle_name: String,
    _state: State<AppState>,
) -> Result<Vec<mod_tools::ModTool>, String> {
    let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
    Ok(mod_tools::detect_tools(&data_dir))
}

#[tauri::command]
async fn install_mod_tool(
    tool_id: String,
    game_id: String,
    bottle_name: String,
) -> Result<String, String> {
    let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
    mod_tools::install_tool(&tool_id, &data_dir)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn uninstall_mod_tool(
    tool_id: String,
    game_id: String,
    bottle_name: String,
    detected_path: Option<String>,
) -> Result<(), String> {
    let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
    mod_tools::uninstall_tool(&tool_id, &data_dir, detected_path.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn launch_mod_tool(
    tool_id: String,
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<LaunchResult, String> {
    let (bottle, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
    let tools = mod_tools::detect_tools(&data_dir);
    let tool = tools
        .iter()
        .find(|t| t.id == tool_id)
        .ok_or_else(|| format!("Tool '{}' not found", tool_id))?;
    let exe_path = tool
        .detected_path
        .as_ref()
        .ok_or_else(|| format!("Tool '{}' is not installed", tool_id))?;
    mod_tools::launch_tool_with_logging(
        Path::new(exe_path),
        &bottle,
        &tool_id,
        &tool.name,
        &state.db,
    )
}

#[tauri::command]
async fn reinstall_mod_tool(
    tool_id: String,
    game_id: String,
    bottle_name: String,
) -> Result<String, String> {
    let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
    mod_tools::reinstall_tool(&tool_id, &data_dir)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn apply_tool_ini_edits_cmd(
    tool_id: String,
    game_id: String,
    bottle_name: String,
) -> Result<usize, String> {
    let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
    mod_tools::apply_tool_ini_edits(&tool_id, &data_dir).map_err(|e| e.to_string())
}

// --- Tool Requirement Detection ---

#[tauri::command]
fn detect_collection_tools(
    manifest_json: String,
    game_id: String,
    bottle_name: String,
) -> Result<Vec<mod_tools::RequiredTool>, String> {
    let manifest: collections::CollectionManifest = serde_json::from_str(&manifest_json)
        .map_err(|e| format!("Invalid manifest JSON: {}", e))?;
    let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
    Ok(mod_tools::detect_required_tools_collection(
        &manifest, &data_dir,
    ))
}

#[tauri::command]
fn detect_wabbajack_tools(
    wj_path: String,
    game_id: String,
    bottle_name: String,
) -> Result<Vec<mod_tools::RequiredTool>, String> {
    let parsed = wabbajack::parse_wabbajack_file(std::path::Path::new(&wj_path))
        .map_err(|e| format!("Failed to parse .wabbajack: {}", e))?;
    let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
    Ok(mod_tools::detect_required_tools_wabbajack(
        &parsed, &data_dir,
    ))
}

// --- Platform Detection ---

#[derive(Clone, Debug, serde::Serialize)]
struct PlatformInfo {
    os: String,
    is_steam_os: bool,
}

#[tauri::command]
fn get_platform_detail() -> PlatformInfo {
    let os = std::env::consts::OS.to_string();
    let is_steam_os = if cfg!(target_os = "linux") {
        std::path::Path::new("/etc/steamos-release").exists() || std::env::var("SteamOS").is_ok()
    } else {
        false
    };
    PlatformInfo { os, is_steam_os }
}

// --- FOMOD ---

#[tauri::command]
fn detect_fomod(staging_path: String) -> Result<Option<FomodInstaller>, String> {
    let path = PathBuf::from(&staging_path);
    fomod::parse_fomod(&path).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_fomod_defaults(
    installer: FomodInstaller,
) -> Result<std::collections::HashMap<String, Vec<String>>, String> {
    Ok(fomod::get_default_selections(&installer))
}

#[tauri::command]
fn get_fomod_files(
    installer: FomodInstaller,
    selections: std::collections::HashMap<String, Vec<String>>,
) -> Result<Vec<fomod::FomodFile>, String> {
    Ok(fomod::get_files_for_selections(&installer, &selections))
}

// --- Integrity ---

#[tauri::command]
fn create_game_snapshot(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<usize, String> {
    let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
    let db = &state.db;

    integrity::create_game_snapshot(db, &game_id, &bottle_name, &data_dir)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn check_game_integrity(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<IntegrityReport, String> {
    let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
    let db = &state.db;

    integrity::check_game_integrity(db, &game_id, &bottle_name, &data_dir)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn has_game_snapshot(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<bool, String> {
    let db = &state.db;
    integrity::has_snapshot(db, &game_id, &bottle_name).map_err(|e| e.to_string())
}

// --- Wabbajack Modlists ---

#[tauri::command]
async fn get_wabbajack_modlists() -> Result<Vec<ModlistSummary>, String> {
    wabbajack::fetch_modlist_gallery().await
}

#[tauri::command]
fn parse_wabbajack_file(file_path: String) -> Result<ParsedModlist, String> {
    wabbajack::parse_wabbajack_file(std::path::Path::new(&file_path))
}

#[tauri::command]
async fn download_wabbajack_file(url: String, filename: String) -> Result<String, String> {
    let download_dir = config::get_config()
        .ok()
        .and_then(|c| c.download_dir)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            dirs::data_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("corkscrew")
                .join("downloads")
        });

    std::fs::create_dir_all(&download_dir)
        .map_err(|e| format!("Failed to create download directory: {e}"))?;

    let dest = download_dir.join(&filename);

    let client = reqwest::Client::builder()
        .user_agent(format!("Corkscrew/{}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Download failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!(
            "Download failed: HTTP {} — the file may have been removed from the Wabbajack CDN.",
            resp.status()
        ));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Failed to read download: {e}"))?;

    std::fs::write(&dest, &bytes)
        .map_err(|e| format!("Failed to save file: {e}"))?;

    Ok(dest.to_string_lossy().to_string())
}

// --- Helpers ---

fn get_current_plugins(game_id: &str, bottle_name: &str) -> Vec<PluginEntry> {
    if game_id != "skyrimse" {
        return Vec::new();
    }

    let (bottle, game, _) = match resolve_game(game_id, bottle_name) {
        Ok(result) => result,
        Err(_) => return Vec::new(),
    };

    let plugins_file = games::with_plugin(game_id, |plugin| {
        plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
    })
    .flatten();

    match plugins_file {
        Some(pf) if pf.exists() => {
            plugins::skyrim_plugins::read_plugins_txt(&pf).unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

fn sync_skyrim_plugins_for_game(game: &DetectedGame, bottle: &Bottle) -> Result<(), String> {
    let game_path = Path::new(&game.game_path);
    let data_dir = Path::new(&game.data_dir);

    let plugins_file = games::with_plugin(&game.game_id, |plugin| {
        plugin.get_plugins_file(game_path, bottle)
    })
    .flatten();

    if let Some(pf) = plugins_file {
        // Derive loadorder.txt path from plugins.txt path
        let loadorder_file = pf
            .parent()
            .map(|p| p.join("loadorder.txt"))
            .unwrap_or_else(|| pf.with_file_name("loadorder.txt"));
        plugins::skyrim_plugins::sync_plugins(data_dir, &pf, &loadorder_file)
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

// --- Nexus SSO ---

#[tauri::command]
async fn start_nexus_sso() -> Result<String, String> {
    // Run the blocking SSO WebSocket flow on a background thread
    tokio::task::spawn_blocking(nexus_sso::run_sso_flow)
        .await
        .map_err(|e| format!("SSO task failed: {}", e))?
        .map_err(|e| e.to_string())
}

// --- OAuth (legacy) ---

#[tauri::command]
async fn start_nexus_oauth(client_id: String) -> Result<TokenPair, String> {
    oauth::start_oauth_flow(&client_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn refresh_nexus_tokens(
    client_id: String,
    refresh_token: String,
) -> Result<TokenPair, String> {
    oauth::refresh_tokens(&client_id, &refresh_token)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn save_oauth_tokens(tokens: TokenPair) -> Result<(), String> {
    oauth::save_tokens(&tokens).map_err(|e| e.to_string())
}

#[tauri::command]
fn load_oauth_tokens() -> Result<Option<TokenPair>, String> {
    oauth::load_tokens().map_err(|e| e.to_string())
}

#[tauri::command]
fn clear_oauth_tokens() -> Result<(), String> {
    oauth::clear_tokens().map_err(|e| e.to_string())
}

#[tauri::command]
fn get_nexus_user_info(access_token: String) -> Result<NexusUserInfo, String> {
    oauth::parse_user_info(&access_token).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_auth_method_cmd() -> Result<serde_json::Value, String> {
    let method = oauth::get_auth_method();
    match method {
        oauth::AuthMethod::OAuth(ref tokens) => Ok(serde_json::json!({
            "type": "oauth",
            "expires_at": tokens.expires_at,
        })),
        oauth::AuthMethod::ApiKey(ref key) => Ok(serde_json::json!({
            "type": "api_key",
            "key_prefix": &key[..key.len().min(8)],
        })),
        oauth::AuthMethod::None => Ok(serde_json::json!({
            "type": "none",
        })),
    }
}

#[tauri::command]
async fn get_nexus_account_status() -> Result<serde_json::Value, String> {
    let method = oauth::get_auth_method();
    match method {
        oauth::AuthMethod::OAuth(ref tokens) => {
            let user = oauth::parse_user_info(&tokens.access_token).map_err(|e| e.to_string())?;
            Ok(serde_json::json!({
                "connected": true,
                "auth_type": "oauth",
                "name": user.name,
                "email": user.email,
                "avatar": user.avatar,
                "is_premium": user.is_premium,
                "membership_roles": user.membership_roles,
            }))
        }
        oauth::AuthMethod::ApiKey(ref key) => {
            let client = nexus::NexusClient::new(key.clone());
            let info = client.validate_key().await.map_err(|e| e.to_string())?;
            let name = info
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let is_premium = info
                .get("is_premium")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let is_supporter = info
                .get("is_supporter")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let avatar = info
                .get("profile_url")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let email = info
                .get("email")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            Ok(serde_json::json!({
                "connected": true,
                "auth_type": "api_key",
                "name": name,
                "email": email,
                "avatar": avatar,
                "is_premium": is_premium || is_supporter,
                "membership_roles": [],
            }))
        }
        oauth::AuthMethod::None => Ok(serde_json::json!({
            "connected": false,
        })),
    }
}

// --- Crash Logs ---

#[tauri::command]
fn find_crash_logs_cmd(game_id: String, bottle_name: String) -> Result<Vec<CrashLogEntry>, String> {
    let (bottle, game, _) = resolve_game(&game_id, &bottle_name)?;

    let game_path = PathBuf::from(&game.game_path);
    Ok(crashlog::find_crash_logs(&game_path, &bottle.path))
}

#[tauri::command]
fn analyze_crash_log_cmd(log_path: String) -> Result<CrashReport, String> {
    crashlog::analyze_crash_log(Path::new(&log_path)).map_err(|e| e.to_string())
}

// --- Collections ---

#[tauri::command]
async fn fetch_url_text(url: String) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .user_agent(format!("Corkscrew/{}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;

    // Convert GitHub URLs to raw content URLs so we get raw markdown
    // instead of the full GitHub HTML page with navigation chrome.
    let resolved_url = if url.contains("github.com") && url.contains("/blob/") {
        // Blob URL: github.com/user/repo/blob/main/FILE → raw.githubusercontent.com/user/repo/main/FILE
        url.replace("github.com", "raw.githubusercontent.com")
            .replace("/blob/", "/")
    } else if url.contains("github.com") && !url.contains("/raw/") && !url.contains("raw.githubusercontent.com") {
        // Plain repo URL: github.com/user/repo → try raw README.md
        let trimmed = url.trim_end_matches('/');
        let raw_base = trimmed.replace("github.com", "raw.githubusercontent.com");
        // Try main branch first, fall back to master
        let main_url = format!("{}/main/README.md", raw_base);
        let resp = client.get(&main_url)
            .header("Accept", "text/plain, text/markdown, */*")
            .send().await;
        if let Ok(r) = resp {
            if r.status().is_success() {
                return r.text().await.map_err(|e| format!("Failed to read response: {e}"));
            }
        }
        format!("{}/master/README.md", raw_base)
    } else {
        url.clone()
    };

    let resp = client
        .get(&resolved_url)
        .header("Accept", "text/plain, text/markdown, */*")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch URL: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}: {}", resp.status(), resolved_url));
    }

    resp.text()
        .await
        .map_err(|e| format!("Failed to read response: {e}"))
}

#[tauri::command]
async fn browse_nexus_mods_cmd(
    game_slug: String,
    category: String,
) -> Result<Vec<nexus::NexusModInfo>, String> {
    let api_key = config::get_config()
        .ok()
        .and_then(|c| c.nexus_api_key)
        .ok_or_else(|| "No NexusMods API key configured".to_string())?;
    let client = nexus::NexusClient::new(api_key);
    client
        .browse_mods(&game_slug, &category)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn search_nexus_mods_cmd(
    game_slug: String,
    search_text: Option<String>,
    sort_by: Option<String>,
    sort_dir: Option<String>,
    count: u32,
    offset: u32,
    include_adult: bool,
    category_id: Option<i64>,
    author: Option<String>,
    updated_since: Option<String>,
    min_downloads: Option<i64>,
    min_endorsements: Option<i64>,
) -> Result<NexusSearchResult, String> {
    let api_key = config::get_config()
        .ok()
        .and_then(|c| c.nexus_api_key)
        .ok_or_else(|| "No NexusMods API key configured".to_string())?;
    nexus::graphql_search_mods(
        &api_key,
        &game_slug,
        search_text.as_deref(),
        sort_by.as_deref(),
        sort_dir.as_deref(),
        count,
        offset,
        include_adult,
        category_id,
        author.as_deref(),
        updated_since.as_deref(),
        min_downloads,
        min_endorsements,
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_game_categories_cmd(
    game_slug: String,
) -> Result<Vec<NexusCategory>, String> {
    let api_key = config::get_config()
        .ok()
        .and_then(|c| c.nexus_api_key)
        .ok_or_else(|| "No NexusMods API key configured".to_string())?;
    let client = nexus::NexusClient::new(api_key);
    client
        .get_game_categories(&game_slug)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn browse_collections_cmd(
    game_domain: String,
    count: u32,
    offset: u32,
    sort_field: Option<String>,
    sort_direction: Option<String>,
    search_text: Option<String>,
    author: Option<String>,
    min_downloads: Option<i64>,
    min_endorsements: Option<i64>,
) -> Result<CollectionSearchResult, String> {
    let api_key = config::get_config().ok().and_then(|c| c.nexus_api_key);

    let sf = sort_field.as_deref().unwrap_or("endorsements");
    let sd = sort_direction.as_deref().unwrap_or("desc");
    let st = search_text.as_deref().filter(|s| !s.is_empty());

    collections::browse_collections(
        api_key.as_deref(), &game_domain, count, offset, sf, sd, st,
        author.as_deref(),
        min_downloads,
        min_endorsements,
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_collection_cmd(slug: String, game_domain: String) -> Result<CollectionInfo, String> {
    let api_key = config::get_config().ok().and_then(|c| c.nexus_api_key);

    collections::get_collection(api_key.as_deref(), &slug, &game_domain)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_collection_revisions(slug: String) -> Result<Vec<CollectionRevision>, String> {
    let api_key = config::get_config().ok().and_then(|c| c.nexus_api_key);

    collections::get_revisions(api_key.as_deref(), &slug)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_collection_mods(slug: String, revision: u32) -> Result<Vec<CollectionMod>, String> {
    let api_key = config::get_config().ok().and_then(|c| c.nexus_api_key);

    collections::get_revision_mods(api_key.as_deref(), &slug, revision)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn parse_collection_bundle_cmd(bundle_path: String) -> Result<CollectionManifest, String> {
    collections::parse_collection_bundle(Path::new(&bundle_path)).map_err(|e| e.to_string())
}

#[tauri::command]
async fn install_collection_cmd(
    app: AppHandle,
    manifest: CollectionManifest,
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let result = collection_installer::install_collection(
        &app,
        &state.db,
        &state.download_queue,
        &manifest,
        &game_id,
        &bottle_name,
    )
    .await?;

    Ok(serde_json::json!({
        "installed": result.installed,
        "already_installed": result.already_installed,
        "skipped": result.skipped,
        "failed": result.failed,
        "details": result.details,
    }))
}

// --- Plugin Load Order Rules ---

#[tauri::command]
fn add_plugin_rule(
    game_id: String,
    bottle_name: String,
    plugin_name: String,
    rule_type: loot_rules::PluginRuleType,
    reference_plugin: String,
    state: State<AppState>,
) -> Result<i64, String> {
    let db = &state.db;
    loot_rules::add_rule(
        db,
        &game_id,
        &bottle_name,
        &plugin_name,
        rule_type,
        &reference_plugin,
    )
}

#[tauri::command]
fn remove_plugin_rule(rule_id: i64, state: State<AppState>) -> Result<(), String> {
    let db = &state.db;
    loot_rules::remove_rule(db, rule_id)
}

#[tauri::command]
fn list_plugin_rules(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<Vec<PluginRule>, String> {
    let db = &state.db;
    loot_rules::list_rules(db, &game_id, &bottle_name)
}

#[tauri::command]
fn clear_plugin_rules(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<(), String> {
    let db = &state.db;
    loot_rules::clear_rules(db, &game_id, &bottle_name)
}

// --- Mod Rollback & Snapshots ---

#[tauri::command]
fn save_mod_version_cmd(
    mod_id: i64,
    version: String,
    staging_path: String,
    archive_name: String,
    state: State<AppState>,
) -> Result<i64, String> {
    let db = &state.db;
    rollback::save_mod_version(db, mod_id, &version, &staging_path, &archive_name)
}

#[tauri::command]
fn list_mod_versions_cmd(mod_id: i64, state: State<AppState>) -> Result<Vec<ModVersion>, String> {
    let db = &state.db;
    rollback::list_mod_versions(db, mod_id)
}

#[tauri::command]
fn rollback_mod_version(
    mod_id: i64,
    version_id: i64,
    state: State<AppState>,
) -> Result<ModVersion, String> {
    let db = &state.db;
    rollback::rollback_to_version(db, mod_id, version_id)
}

#[tauri::command]
fn cleanup_mod_versions(
    mod_id: i64,
    keep_count: usize,
    state: State<AppState>,
) -> Result<usize, String> {
    let db = &state.db;
    rollback::cleanup_old_versions(db, mod_id, keep_count)
}

#[tauri::command]
fn create_mod_snapshot(
    game_id: String,
    bottle_name: String,
    name: String,
    description: Option<String>,
    state: State<AppState>,
) -> Result<i64, String> {
    let db = &state.db;
    rollback::create_snapshot(db, &game_id, &bottle_name, &name, description.as_deref())
}

#[tauri::command]
fn list_mod_snapshots(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<Vec<ModSnapshot>, String> {
    let db = &state.db;
    rollback::list_snapshots(db, &game_id, &bottle_name)
}

#[tauri::command]
fn delete_mod_snapshot(snapshot_id: i64, state: State<AppState>) -> Result<(), String> {
    let db = &state.db;
    rollback::delete_snapshot(db, snapshot_id)
}

// --- Modlist Export/Import ---

#[tauri::command]
fn export_modlist_cmd(
    game_id: String,
    bottle_name: String,
    output_path: String,
    notes: Option<String>,
    state: State<AppState>,
) -> Result<String, String> {
    let db = &state.db;

    // Get current plugin order if applicable
    let plugin_entries = get_current_plugins(&game_id, &bottle_name);

    let modlist = modlist_io::export_modlist(
        db,
        &game_id,
        &bottle_name,
        &plugin_entries,
        notes.as_deref(),
    )
    .map_err(|e| e.to_string())?;

    let path = PathBuf::from(&output_path);
    modlist_io::write_modlist_file(&modlist, &path).map_err(|e| e.to_string())?;

    Ok(output_path)
}

#[tauri::command]
fn import_modlist_plan(
    file_path: String,
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<ImportPlan, String> {
    let modlist =
        modlist_io::read_modlist_file(Path::new(&file_path)).map_err(|e| e.to_string())?;
    modlist_io::validate_modlist(&modlist, &game_id).map_err(|e| e.to_string())?;

    let db = &state.db;
    modlist_io::plan_import(db, &modlist, &game_id, &bottle_name).map_err(|e| e.to_string())
}

#[tauri::command]
fn diff_modlists_cmd(
    file_path: String,
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<ModlistDiff, String> {
    let imported =
        modlist_io::read_modlist_file(Path::new(&file_path)).map_err(|e| e.to_string())?;

    let db = &state.db;
    let plugin_entries = get_current_plugins(&game_id, &bottle_name);

    let current = modlist_io::export_modlist(db, &game_id, &bottle_name, &plugin_entries, None)
        .map_err(|e| e.to_string())?;

    Ok(modlist_io::diff_modlists(&current, &imported))
}

#[tauri::command]
fn execute_modlist_import(
    file_path: String,
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<modlist_io::ImportResult, String> {
    let imported =
        modlist_io::read_modlist_file(Path::new(&file_path)).map_err(|e| e.to_string())?;
    let db = &state.db;
    modlist_io::execute_import(db, &imported, &game_id, &bottle_name)
        .map_err(|e| e.to_string())
}

// --- Disk Budget Commands ---

#[tauri::command]
fn get_disk_budget(
    game_id: String,
    bottle_name: String,
) -> Result<disk_budget::DiskBudget, String> {
    let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
    Ok(disk_budget::compute_budget(
        &game_id,
        &bottle_name,
        &data_dir,
    ))
}

#[tauri::command]
fn estimate_install_impact_cmd(
    archive_size: u64,
    game_id: String,
    bottle_name: String,
) -> Result<disk_budget::InstallImpact, String> {
    let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
    Ok(disk_budget::estimate_install_impact(
        archive_size,
        &data_dir,
    ))
}

#[tauri::command]
fn get_available_disk_space_cmd(path: String) -> Result<u64, String> {
    Ok(disk_budget::available_space(std::path::Path::new(&path)))
}

// --- Staging Info Commands ---

#[tauri::command]
fn get_staging_info(
    game_id: String,
    bottle_name: String,
) -> Result<serde_json::Value, String> {
    let staging_root = staging::staging_root();
    let staging_dir = staging::staging_base_dir(&game_id, &bottle_name);

    let (hardlinks_supported, data_dir_str) = match resolve_game(&game_id, &bottle_name) {
        Ok((_, _, data_dir)) => (
            deployer::same_filesystem(&staging_dir, &data_dir),
            data_dir.to_string_lossy().to_string(),
        ),
        Err(_) => (false, String::new()),
    };

    let config = config::get_config().map_err(|e| e.to_string())?;
    let is_custom = config.staging_dir.is_some();

    Ok(serde_json::json!({
        "staging_root": staging_root.to_string_lossy(),
        "staging_dir": staging_dir.to_string_lossy(),
        "data_dir": data_dir_str,
        "hardlinks_supported": hardlinks_supported,
        "is_custom_path": is_custom,
    }))
}

#[tauri::command]
fn set_staging_directory(path: Option<String>) -> Result<(), String> {
    match path {
        Some(ref p) if !p.is_empty() => {
            // Validate path exists or can be created
            let path_buf = std::path::PathBuf::from(p);
            if !path_buf.exists() {
                std::fs::create_dir_all(&path_buf).map_err(|e| {
                    format!("Cannot create staging directory '{}': {}", p, e)
                })?;
            }
            config::set_config_value("staging_dir", p).map_err(|e| e.to_string())
        }
        _ => {
            // Clear override — revert to default
            let mut cfg = config::get_config().map_err(|e| e.to_string())?;
            cfg.staging_dir = None;
            config::save_config(&cfg).map_err(|e| e.to_string())
        }
    }
}

// --- INI Manager Commands ---

#[tauri::command]
fn get_ini_settings(
    game_id: String,
    bottle_name: String,
) -> Result<Vec<ini_manager::IniFile>, String> {
    let bottle = resolve_bottle(&bottle_name)?;
    Ok(ini_manager::read_all_ini(&bottle, &game_id))
}

#[tauri::command]
fn set_ini_setting(
    file_path: String,
    section: String,
    key: String,
    value: String,
) -> Result<(), String> {
    ini_manager::set_setting(Path::new(&file_path), &section, &key, &value)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_ini_presets(game_id: String) -> Vec<ini_manager::IniPreset> {
    ini_manager::builtin_presets(&game_id)
}

#[tauri::command]
fn apply_ini_preset(
    game_id: String,
    bottle_name: String,
    preset_name: String,
) -> Result<usize, String> {
    let bottle = resolve_bottle(&bottle_name)?;
    let presets = ini_manager::builtin_presets(&game_id);
    let preset = presets
        .iter()
        .find(|p| p.name == preset_name)
        .ok_or_else(|| format!("Preset '{}' not found", preset_name))?;
    ini_manager::apply_preset(&bottle, &game_id, preset).map_err(|e| e.to_string())
}

// --- Wine Diagnostic Commands ---

#[tauri::command]
fn run_wine_diagnostics(
    game_id: String,
    bottle_name: String,
) -> Result<wine_diagnostic::DiagnosticResult, String> {
    let bottle = resolve_bottle(&bottle_name)?;
    Ok(wine_diagnostic::run_diagnostics(&bottle, &game_id))
}

#[tauri::command]
fn fix_wine_appdata(bottle_name: String) -> Result<(), String> {
    let bottle = resolve_bottle(&bottle_name)?;
    wine_diagnostic::fix_appdata(&bottle).map_err(|e| e.to_string())
}

#[tauri::command]
fn fix_wine_dll_override(
    bottle_name: String,
    dll_name: String,
    override_type: String,
) -> Result<(), String> {
    let bottle = resolve_bottle(&bottle_name)?;
    wine_diagnostic::fix_dll_override(&bottle, &dll_name, &override_type).map_err(|e| e.to_string())
}

#[tauri::command]
fn fix_wine_retina_mode(bottle_name: String) -> Result<(), String> {
    let bottle = resolve_bottle(&bottle_name)?;
    wine_diagnostic::fix_retina_mode(&bottle).map_err(|e| e.to_string())
}

// --- Pre-flight Commands ---

#[tauri::command]
fn run_preflight_check(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<preflight::PreflightResult, String> {
    let (bottle, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
    let db = &state.db;
    Ok(preflight::run_preflight(
        db,
        &bottle,
        &game_id,
        &bottle_name,
        &data_dir,
    ))
}

// --- Mod Dependency Commands ---

#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn add_mod_dependency(
    game_id: String,
    bottle_name: String,
    mod_id: i64,
    depends_on_id: Option<i64>,
    nexus_dep_id: Option<i64>,
    dep_name: String,
    relationship: String,
    state: State<AppState>,
) -> Result<i64, String> {
    mod_dependencies::add_dependency(
        &state.db,
        &game_id,
        &bottle_name,
        mod_id,
        depends_on_id,
        nexus_dep_id,
        &dep_name,
        &relationship,
    )
}

#[tauri::command]
fn remove_mod_dependency(dep_id: i64, state: State<AppState>) -> Result<(), String> {
    mod_dependencies::remove_dependency(&state.db, dep_id)
}

#[tauri::command]
fn get_mod_dependencies(
    mod_id: i64,
    state: State<AppState>,
) -> Result<Vec<mod_dependencies::ModDependency>, String> {
    mod_dependencies::get_dependencies(&state.db, mod_id)
}

#[tauri::command]
fn check_dependency_issues(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<Vec<mod_dependencies::DependencyIssue>, String> {
    mod_dependencies::check_dependency_issues(&state.db, &game_id, &bottle_name)
}

// --- Mod Recommendation Commands ---

#[tauri::command]
fn get_mod_recommendations(
    game_id: String,
    bottle_name: String,
    target_mod_id: i64,
    state: State<AppState>,
) -> Result<mod_recommendations::RecommendationResult, String> {
    mod_recommendations::get_recommendations(&state.db, &game_id, &bottle_name, target_mod_id)
}

#[tauri::command]
fn get_popular_mods(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<Vec<(String, i64, usize)>, String> {
    mod_recommendations::get_popular_mods(&state.db, &game_id, &bottle_name)
}

// --- Session Tracker Commands ---

#[tauri::command]
fn start_game_session(
    game_id: String,
    bottle_name: String,
    profile_name: Option<String>,
    state: State<AppState>,
) -> Result<i64, String> {
    session_tracker::start_session(&state.db, &game_id, &bottle_name, profile_name.as_deref())
}

#[tauri::command]
fn end_game_session(
    session_id: i64,
    clean_exit: bool,
    crash_log_path: Option<String>,
    state: State<AppState>,
) -> Result<(), String> {
    session_tracker::end_session(&state.db, session_id, clean_exit, crash_log_path.as_deref())
}

#[tauri::command]
fn record_session_mod_change(
    session_id: i64,
    mod_id: Option<i64>,
    mod_name: String,
    change_type: String,
    detail: Option<String>,
    state: State<AppState>,
) -> Result<i64, String> {
    session_tracker::record_mod_change(
        &state.db,
        session_id,
        mod_id,
        &mod_name,
        &change_type,
        detail.as_deref(),
    )
}

#[tauri::command]
fn get_session_history(
    game_id: String,
    bottle_name: String,
    limit: Option<usize>,
    state: State<AppState>,
) -> Result<Vec<session_tracker::GameSession>, String> {
    session_tracker::get_session_history(&state.db, &game_id, &bottle_name, limit.unwrap_or(20))
}

#[tauri::command]
fn get_stability_summary(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<session_tracker::StabilitySummary, String> {
    session_tracker::get_stability_summary(&state.db, &game_id, &bottle_name)
}

// --- FOMOD Recipe Commands ---

#[tauri::command]
fn save_fomod_recipe(
    mod_id: i64,
    mod_name: String,
    installer_hash: Option<String>,
    selections: std::collections::HashMap<String, Vec<String>>,
    state: State<AppState>,
) -> Result<i64, String> {
    fomod_recipes::save_recipe(
        &state.db,
        mod_id,
        &mod_name,
        installer_hash.as_deref(),
        &selections,
    )
}

#[tauri::command]
fn get_fomod_recipe(
    mod_id: i64,
    state: State<AppState>,
) -> Result<Option<fomod_recipes::FomodRecipe>, String> {
    fomod_recipes::get_recipe(&state.db, mod_id)
}

#[tauri::command]
fn list_fomod_recipes(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<Vec<fomod_recipes::FomodRecipe>, String> {
    fomod_recipes::list_recipes(&state.db, &game_id, &bottle_name)
}

#[tauri::command]
fn delete_fomod_recipe(mod_id: i64, state: State<AppState>) -> Result<(), String> {
    fomod_recipes::delete_recipe(&state.db, mod_id)
}

#[tauri::command]
fn has_compatible_fomod_recipe(
    mod_id: i64,
    current_hash: Option<String>,
    state: State<AppState>,
) -> Result<bool, String> {
    fomod_recipes::has_compatible_recipe(&state.db, mod_id, current_hash.as_deref())
}

// --- Browser WebView Management ---

#[tauri::command]
async fn create_browser_webview(
    app: AppHandle,
    url: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    // Close existing browser panel if any
    if let Some(existing) = app.get_webview("browser-panel") {
        let _ = existing.close();
    }

    let parsed_url: tauri::Url = url.parse().map_err(|e: url::ParseError| e.to_string())?;
    let window = app
        .get_window("main")
        .ok_or("Main window not found")?;

    let builder = tauri::webview::WebviewBuilder::new(
        "browser-panel",
        tauri::WebviewUrl::External(parsed_url),
    );

    window
        .add_child(
            builder,
            tauri::LogicalPosition::new(x, y),
            tauri::LogicalSize::new(width, height),
        )
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
async fn resize_browser_webview(
    app: AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let webview = app
        .get_webview("browser-panel")
        .ok_or("Browser panel not found")?;
    webview
        .set_position(tauri::LogicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    webview
        .set_size(tauri::LogicalSize::new(width, height))
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn close_browser_webview(app: AppHandle) -> Result<(), String> {
    if let Some(webview) = app.get_webview("browser-panel") {
        webview.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn navigate_browser_webview(app: AppHandle, url: String) -> Result<(), String> {
    let webview = app
        .get_webview("browser-panel")
        .ok_or("Browser panel not found")?;
    let parsed_url: tauri::Url = url.parse().map_err(|e: url::ParseError| e.to_string())?;
    webview.navigate(parsed_url).map_err(|e| e.to_string())?;
    Ok(())
}

// --- Nexus Mod Files & Direct Download ---

#[tauri::command]
async fn get_nexus_mod_files(
    game_slug: String,
    mod_id: i64,
) -> Result<Vec<nexus::NexusModFile>, String> {
    let cfg = config::get_config().map_err(|e| e.to_string())?;
    let api_key = cfg
        .nexus_api_key
        .ok_or("No Nexus API key configured")?;

    let client = nexus::NexusClient::new(api_key);
    let raw_files = client
        .get_mod_files(&game_slug, mod_id)
        .await
        .map_err(|e| e.to_string())?;

    Ok(nexus::parse_mod_files(&raw_files))
}

#[tauri::command]
async fn download_and_install_nexus_mod(
    app: AppHandle,
    game_slug: String,
    mod_id: i64,
    file_id: i64,
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let cfg = config::get_config().map_err(|e| e.to_string())?;
    let api_key = cfg
        .nexus_api_key
        .ok_or("No Nexus API key configured")?;

    let client = nexus::NexusClient::new(api_key);

    // Enforce premium (backend safety check)
    if !client.is_premium().await {
        return Err("Premium membership required for direct downloads".to_string());
    }

    // Get mod info for name/version
    let mod_info = client
        .get_mod(&game_slug, mod_id)
        .await
        .map_err(|e| e.to_string())?;
    let mod_name = mod_info
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown Mod")
        .to_string();
    let mod_version = mod_info
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Emit progress: starting
    let _ = app.emit(
        "install-progress",
        serde_json::json!({
            "kind": "modStarted",
            "mod_index": 0,
            "total_mods": 1,
            "mod_name": &mod_name,
        }),
    );

    // Get download links (premium: no key/expires needed)
    let links = client
        .get_download_links(&game_slug, mod_id, file_id, None, None)
        .await
        .map_err(|e| e.to_string())?;
    let link = links.first().ok_or("No download links available")?;

    // Download
    let download_dir = cfg
        .download_dir
        .map(PathBuf::from)
        .unwrap_or_else(config::downloads_dir);

    let _ = app.emit(
        "install-progress",
        serde_json::json!({
            "kind": "stepChanged",
            "mod_index": 0,
            "step": "downloading",
            "detail": format!("Downloading {}...", mod_name),
        }),
    );

    let app_clone = app.clone();
    let dl_mod_name = mod_name.clone();
    let archive_path = client
        .download_file(
            &link.uri,
            &download_dir,
            Some(move |downloaded: u64, total: u64| {
                let _ = app_clone.emit(
                    "download-progress",
                    serde_json::json!({
                        "downloaded": downloaded,
                        "total": total,
                        "mod_name": &dl_mod_name,
                    }),
                );
            }),
        )
        .await
        .map_err(|e| e.to_string())?;

    // Stage & Deploy (reuse existing install pattern)
    let _ = app.emit(
        "install-progress",
        serde_json::json!({
            "kind": "stepChanged",
            "mod_index": 0,
            "step": "installing",
            "detail": format!("Installing {}...", mod_name),
        }),
    );

    let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;
    let db = &state.db;

    let next_priority = db
        .get_next_priority(&game_id, &bottle_name)
        .map_err(|e| e.to_string())?;
    let db_mod_id = db
        .add_mod(
            &game_id,
            &bottle_name,
            Some(mod_id),
            &mod_name,
            &mod_version,
            &archive_path.to_string_lossy(),
            &[],
        )
        .map_err(|e| e.to_string())?;
    db.set_mod_priority(db_mod_id, next_priority)
        .map_err(|e| e.to_string())?;

    // Stage
    let staging_result = staging::stage_mod(
        &archive_path,
        &game_id,
        &bottle_name,
        db_mod_id,
        &mod_name,
    )
    .map_err(|e| {
        let _ = db.remove_mod(db_mod_id);
        format!("Staging failed: {e}")
    })?;

    // Update DB
    db.set_staging_path(db_mod_id, &staging_result.staging_path.to_string_lossy())
        .map_err(|e| e.to_string())?;
    db.update_installed_files(db_mod_id, &staging_result.files)
        .map_err(|e| e.to_string())?;
    db.store_file_hashes(db_mod_id, &staging_result.hashes)
        .map_err(|e| e.to_string())?;

    // Deploy
    deployer::deploy_mod(
        db,
        &game_id,
        &bottle_name,
        db_mod_id,
        &staging_result.staging_path,
        &data_dir,
        &staging_result.files,
    )
    .map_err(|e| {
        let _ = staging::remove_staging(&staging_result.staging_path);
        let _ = db.remove_mod(db_mod_id);
        format!("Deploy failed: {e}")
    })?;

    // Set source
    let _ = db.set_mod_source(
        db_mod_id,
        "nexus",
        Some(&format!(
            "https://www.nexusmods.com/{}/mods/{}",
            game_slug, mod_id
        )),
    );

    // Sync plugins if Skyrim
    if game_id == "skyrimse" {
        let _ = sync_skyrim_plugins_for_game(&game, &bottle);
    }

    // Auto-delete archive if setting enabled
    if cfg
        .extra
        .get("auto_delete_archives")
        .and_then(|v| v.as_str())
        == Some("true")
    {
        let _ = std::fs::remove_file(&archive_path);
    }

    let installed = db
        .get_mod(db_mod_id)
        .map_err(|e| e.to_string())?
        .ok_or("Failed to retrieve installed mod")?;

    let _ = app.emit(
        "install-progress",
        serde_json::json!({
            "kind": "modCompleted",
            "mod_index": 0,
            "mod_name": &installed.name,
            "mod_id": db_mod_id,
        }),
    );

    serde_json::to_value(installed).map_err(|e| e.to_string())
}

// --- App Entry Point ---

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Register game plugins (dedicated plugins first, then registry)
    plugins::skyrim_se::register();
    plugins::fallout4::register();
    game_registry::register_all();

    // Initialize database
    let db_path = config::db_path();
    let db = ModDatabase::new(&db_path).expect("Failed to initialize mod database");

    // Initialize additional schemas
    executables::init_schema(&db).expect("Failed to initialize executables schema");
    profiles::init_schema(&db).expect("Failed to initialize profiles schema");
    integrity::init_schema(&db).expect("Failed to initialize integrity schema");
    loot_rules::init_schema(&db).expect("Failed to initialize loot rules schema");
    rollback::init_schema(&db).expect("Failed to initialize rollback schema");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage({
            let queue = download_queue::DownloadQueue::new();
            // Restore persisted queue items from database
            match db.load_queue_items() {
                Ok(items) => {
                    if !items.is_empty() {
                        log::info!("Restored {} download queue items from database", items.len());
                        queue.load_from(items);
                    }
                }
                Err(e) => log::warn!("Failed to load download queue from database: {}", e),
            }
            AppState {
                db: Arc::new(db),
                download_queue: Arc::new(queue),
                wj_cancel_tokens: std::sync::Mutex::new(std::collections::HashMap::new()),
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_bottles,
            get_games,
            get_all_games,
            list_supported_games,
            get_bottle_settings,
            get_bottle_setting_defs,
            set_bottle_setting,
            get_installed_mods,
            install_mod_cmd,
            uninstall_mod,
            toggle_mod,
            get_plugin_order,
            download_from_nexus,
            is_nexus_premium,
            get_config,
            set_config_value,
            get_game_logo,
            launch_game_cmd,
            check_skse,
            get_skse_download_url,
            get_skse_builds,
            install_skse_auto_cmd,
            install_skse_from_archive_cmd,
            set_skse_preference_cmd,
            check_skyrim_version,
            check_skse_compatibility_cmd,
            fix_skyrim_display,
            downgrade_skyrim,
            set_vibrancy,
            add_custom_exe,
            remove_custom_exe,
            list_custom_exes,
            set_default_exe,
            get_conflicts,
            analyze_conflicts_cmd,
            resolve_all_conflicts_cmd,
            record_conflict_winner,
            get_deployment_manifest_cmd,
            set_mod_priority,
            reorder_mods,
            redeploy_all_mods,
            purge_deployment_cmd,
            verify_mod_integrity,
            sort_plugins_loot,
            update_loot_masterlist,
            reorder_plugins_cmd,
            toggle_plugin_cmd,
            move_plugin_cmd,
            get_plugin_messages,
            list_profiles_cmd,
            create_profile_cmd,
            delete_profile_cmd,
            rename_profile_cmd,
            save_profile_snapshot,
            activate_profile,
            check_mod_updates,
            detect_mod_tools_cmd,
            install_mod_tool,
            uninstall_mod_tool,
            launch_mod_tool,
            reinstall_mod_tool,
            apply_tool_ini_edits_cmd,
            detect_collection_tools,
            detect_wabbajack_tools,
            get_platform_detail,
            detect_fomod,
            get_fomod_defaults,
            get_fomod_files,
            create_game_snapshot,
            check_game_integrity,
            has_game_snapshot,
            get_wabbajack_modlists,
            parse_wabbajack_file,
            download_wabbajack_file,
            // Wabbajack Install Pipeline
            wabbajack_installer::install_wabbajack_modlist_cmd,
            wabbajack_installer::cancel_wabbajack_install,
            wabbajack_installer::resume_wabbajack_install,
            wabbajack_installer::get_wabbajack_install_status,
            wabbajack_installer::wabbajack_preflight_cmd,
            // Nexus SSO
            start_nexus_sso,
            // OAuth (legacy)
            start_nexus_oauth,
            refresh_nexus_tokens,
            save_oauth_tokens,
            load_oauth_tokens,
            clear_oauth_tokens,
            get_nexus_user_info,
            get_auth_method_cmd,
            get_nexus_account_status,
            // Crash Logs
            find_crash_logs_cmd,
            analyze_crash_log_cmd,
            // Utility
            fetch_url_text,
            // Collections & Nexus Browse
            browse_nexus_mods_cmd,
            search_nexus_mods_cmd,
            get_game_categories_cmd,
            browse_collections_cmd,
            get_collection_cmd,
            get_collection_revisions,
            get_collection_mods,
            parse_collection_bundle_cmd,
            install_collection_cmd,
            // Plugin Rules
            add_plugin_rule,
            remove_plugin_rule,
            list_plugin_rules,
            clear_plugin_rules,
            // Rollback & Snapshots
            save_mod_version_cmd,
            list_mod_versions_cmd,
            rollback_mod_version,
            cleanup_mod_versions,
            create_mod_snapshot,
            list_mod_snapshots,
            delete_mod_snapshot,
            // Modlist Import/Export
            export_modlist_cmd,
            import_modlist_plan,
            diff_modlists_cmd,
            execute_modlist_import,
            // Download Archive Management
            list_download_archives,
            delete_download_archive,
            get_downloads_stats,
            clear_all_download_archives,
            // Collection Management
            list_installed_collections_cmd,
            set_mod_collection_name_cmd,
            switch_collection_cmd,
            delete_collection_cmd,
            get_collection_diff_cmd,
            get_deployment_health,
            // Notes & Tags
            set_mod_notes,
            set_mod_source,
            set_mod_tags,
            get_all_tags,
            // Auto-category
            backfill_categories,
            // Notification Log
            get_notification_log,
            clear_notification_log,
            log_notification,
            get_notification_count,
            // Download Queue
            get_download_queue,
            get_download_queue_counts,
            retry_download,
            cancel_download,
            clear_finished_downloads,
            // Disk Budget
            get_disk_budget,
            estimate_install_impact_cmd,
            get_available_disk_space_cmd,
            // Staging Info
            get_staging_info,
            set_staging_directory,
            // INI Manager
            get_ini_settings,
            set_ini_setting,
            get_ini_presets,
            apply_ini_preset,
            // Wine Diagnostics
            run_wine_diagnostics,
            fix_wine_appdata,
            fix_wine_dll_override,
            fix_wine_retina_mode,
            // Pre-flight
            run_preflight_check,
            // Mod Dependencies
            add_mod_dependency,
            remove_mod_dependency,
            get_mod_dependencies,
            check_dependency_issues,
            // Mod Recommendations
            get_mod_recommendations,
            get_popular_mods,
            // Session Tracker
            start_game_session,
            end_game_session,
            record_session_mod_change,
            get_session_history,
            get_stability_summary,
            // FOMOD Recipes
            save_fomod_recipe,
            get_fomod_recipe,
            list_fomod_recipes,
            delete_fomod_recipe,
            has_compatible_fomod_recipe,
            // Embedded Browser Webview
            create_browser_webview,
            resize_browser_webview,
            close_browser_webview,
            navigate_browser_webview,
            // Nexus Mod Files & Direct Download
            get_nexus_mod_files,
            download_and_install_nexus_mod,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
