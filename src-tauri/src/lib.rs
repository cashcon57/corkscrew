pub mod background_hash;
pub mod baselines;
pub mod bottle_config;
pub mod bottles;
pub mod cleaner;
pub mod collection_installer;
pub mod collections;
pub mod config;
pub mod conflict_resolver;
pub mod crashlog;
pub mod cursor_clamp;
pub mod database;
pub mod deploy_journal;
pub mod deployer;
pub mod disk_budget;
pub mod display_fix;
pub mod downgrader;
pub mod download_queue;
pub mod executables;
pub mod fomod;
pub mod fomod_recipes;
pub mod game_lock;
pub mod game_registry;
pub mod games;
pub mod google_oauth;
pub mod ini_manager;
pub mod installer;
pub mod instruction_parser;
pub mod instruction_types;
pub mod instruction_validator;
pub mod integrity;
pub mod launcher;
pub mod llm_chat;
pub mod llm_parser;
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
pub mod platform;
pub mod plugins;
pub mod preflight;
pub mod profiles;
pub mod progress;
pub mod rollback;
pub mod session_tracker;
pub mod skse;
pub mod staging;
pub mod steam_integration;
pub mod vortex_fetcher;
pub mod vortex_plugin;
pub mod vortex_registry;
pub mod vortex_runtime;
pub mod vortex_types;
pub mod wabbajack;
pub mod wabbajack_directives;
pub mod wabbajack_downloader;
pub mod wabbajack_installer;
pub mod wabbajack_types;
pub mod wine_compat;
pub mod app_updates;
pub mod self_update;
pub mod wine_diagnostic;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

use lru::LruCache;
use tauri::{AppHandle, Emitter, Manager, State};

use bottles::Bottle;
use collections::{
    CollectionDiff, CollectionInfo, CollectionManifest, CollectionRevision, CollectionSearchResult,
    RevisionModsResult,
};
use config::AppConfig;
use crashlog::{CrashLogEntry, CrashReport, NewCrashInfo};
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
    wj_cancel_tokens:
        std::sync::Mutex<std::collections::HashMap<i64, Arc<std::sync::atomic::AtomicBool>>>,
    /// LRU cache for parsed FOMOD installers, keyed by archive SHA-256 hash.
    fomod_cache: Arc<RwLock<LruCache<String, FomodInstaller>>>,
    /// Chat session for local LLM interaction.
    chat_session: llm_chat::SharedChatSession,
    /// Session-level flag: once we verify the LOOT masterlist is fresh for the
    /// current game, we skip further freshness checks until the game changes
    /// or the user force-refreshes.
    loot_masterlist_checked: Arc<AtomicBool>,
    /// Tracks running game processes per (game_id, bottle_name).
    game_locks: Arc<game_lock::GameLockManager>,
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

/// Create an auto-snapshot before a destructive operation.
/// Silent on failure — logs a warning but never blocks the operation.
fn auto_snapshot_before_destructive(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    label: &str,
) {
    match rollback::create_snapshot(
        db,
        game_id,
        bottle_name,
        label,
        Some("Auto-snapshot before destructive operation"),
    ) {
        Ok(id) => log::info!("Auto-snapshot {} created: {}", id, label),
        Err(e) => log::warn!("Failed to create auto-snapshot '{}': {}", label, e),
    }
}

/// Check game lock for a specific game/bottle and return an error if locked.
fn check_game_lock(
    locks: &game_lock::GameLockManager,
    game_id: &str,
    bottle_name: &str,
) -> Result<(), String> {
    if let Some(lock) = locks.get(game_id, bottle_name) {
        return Err(format!(
            "GAME_LOCKED: Cannot modify mods while {} is running (pid {}). \
             Close the game first or use 'Unlock anyway' to override.",
            game_id, lock.pid
        ));
    }
    Ok(())
}

/// Create a NexusClient from the current auth method (OAuth or API key),
/// auto-refreshing expired OAuth tokens as needed.
async fn nexus_client() -> Result<nexus::NexusClient, String> {
    let method = oauth::get_auth_method_refreshed().await;
    nexus::NexusClient::from_auth_method(&method).map_err(|e| e.to_string())
}

/// Get the current API key string for functions that need a raw key
/// (e.g. GraphQL helpers). Prefers OAuth Bearer token, falls back to API key.
async fn nexus_api_key_or_token() -> Result<(String, bool), String> {
    let method = oauth::get_auth_method_refreshed().await;
    match method {
        oauth::AuthMethod::OAuth(tokens) => Ok((tokens.access_token, true)),
        oauth::AuthMethod::ApiKey(key) => Ok((key, false)),
        oauth::AuthMethod::None => {
            Err("No NexusMods authentication configured. Sign in via Settings.".to_string())
        }
    }
}

// --- Tauri Commands ---

#[tauri::command]
async fn get_bottles() -> Result<Vec<Bottle>, String> {
    tokio::task::spawn_blocking(move || Ok(bottles::detect_bottles()))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn get_games(bottle_name: Option<String>) -> Result<Vec<DetectedGame>, String> {
    tokio::task::spawn_blocking(move || match bottle_name {
        Some(name) => {
            let bottle = resolve_bottle(&name)?;
            Ok(games::detect_games(&bottle))
        }
        None => Ok(games::detect_all_games()),
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn get_all_games() -> Result<Vec<DetectedGame>, String> {
    tokio::task::spawn_blocking(move || Ok(games::detect_all_games()))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
fn list_supported_games() -> Result<Vec<game_registry::SupportedGame>, String> {
    Ok(game_registry::list_supported_games())
}

#[tauri::command]
async fn get_bottle_settings(bottle_name: String) -> Result<bottle_config::BottleSettings, String> {
    tokio::task::spawn_blocking(move || {
        let bottle = resolve_bottle(&bottle_name)?;
        bottle_config::get_bottle_settings(&bottle).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn get_bottle_setting_defs(
    bottle_name: String,
) -> Result<Vec<bottle_config::BottleSettingDef>, String> {
    tokio::task::spawn_blocking(move || {
        let bottle = resolve_bottle(&bottle_name)?;
        let settings = bottle_config::get_bottle_settings(&bottle).map_err(|e| e.to_string())?;
        Ok(bottle_config::get_setting_definitions(&settings))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn set_bottle_setting(bottle_name: String, key: String, value: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let bottle = resolve_bottle(&bottle_name)?;
        bottle_config::set_bottle_setting(&bottle, &key, &value).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn get_installed_mods(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<InstalledMod>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        db.list_mods(&game_id, &bottle_name)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn get_installed_mods_summary(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<database::ModSummary>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        db.list_mods_summary(&game_id, &bottle_name)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Summary task failed: {e}"))?
}

/// Fetch a single mod's full details (including installed_files) for the detail panel.
#[tauri::command]
async fn get_mod_detail(mod_id: i64, state: State<'_, AppState>) -> Result<InstalledMod, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        db.get_mod(mod_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Mod {} not found", mod_id))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn install_mod_cmd(
    app: AppHandle,
    archive_path: String,
    game_id: String,
    bottle_name: String,
    mod_name: Option<String>,
    mod_version: Option<String>,
    source_type: Option<String>,
    source_url: Option<String>,
    nexus_mod_id: Option<i64>,
    state: State<'_, AppState>,
) -> Result<InstalledMod, String> {
    check_game_lock(&state.game_locks, &game_id, &bottle_name)?;
    let db = state.db.clone();
    let app = app.clone();
    tokio::task::spawn_blocking(move || {
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

        let next_priority = db
            .get_next_priority(&game_id, &bottle_name)
            .map_err(|e| e.to_string())?;
        let mod_id = db
            .add_mod(
                &game_id,
                &bottle_name,
                nexus_mod_id,
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

        let staging_result =
            match staging::stage_mod(&archive, &game_id, &bottle_name, mod_id, &name) {
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
            &db,
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
            let _ = sync_plugins_for_game(&game, &bottle);
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
                deployed_size: 0,
                duration_ms: 0,
            },
        );

        db.get_mod(mod_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "Failed to retrieve installed mod".to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn uninstall_mod(
    mod_id: i64,
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    check_game_lock(&state.game_locks, &game_id, &bottle_name)?;
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let installed_mod = db
            .get_mod(mod_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Mod with ID {} not found", mod_id))?;

        let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;

        // Disable the mod first so restore_next_winner won't re-deploy its files
        // during undeploy (it checks m.enabled when finding candidates).
        let _ = db.set_enabled(mod_id, false);

        // Remove deployed files from game directory
        let removed = if installed_mod.staging_path.is_some() {
            // Staged mod: undeploy via deployment manifest
            deployer::undeploy_mod(
                &db,
                &game_id,
                &bottle_name,
                mod_id,
                &data_dir,
                &game.game_path,
            )
            .map_err(|e| e.to_string())?
        } else {
            // Legacy mod: remove files directly
            installer::uninstall_mod_files(&data_dir, &installed_mod.installed_files)
                .map_err(|e| e.to_string())?
        };

        // Clean orphaned rollback staging directories before DB removal
        let _ = rollback::cleanup_mod_version_staging(&db, mod_id);

        // Remove staging directory if it exists
        if let Some(ref staging_path) = installed_mod.staging_path {
            let _ = staging::remove_staging(Path::new(staging_path));
        }

        // Remove from database (cascades to deployment_manifest, file_hashes; cleans profile_mods)
        db.remove_mod(mod_id).map_err(|e| e.to_string())?;

        // Sync Skyrim plugins if applicable
        if game_id == "skyrimse" {
            let _ = sync_plugins_for_game(&game, &bottle);
        }

        Ok(removed)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn toggle_mod(
    mod_id: i64,
    game_id: String,
    bottle_name: String,
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    check_game_lock(&state.game_locks, &game_id, &bottle_name)?;
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
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

            let op = if enabled {
                deploy_journal::JournalOp::Deploy
            } else {
                deploy_journal::JournalOp::Undeploy
            };
            let journal_id = deploy_journal::begin(&game_id, &bottle_name, op, &[mod_id])
                .unwrap_or_default();

            if enabled {
                // Re-deploy from staging
                let files =
                    staging::list_staging_files(&staging_path).map_err(|e| e.to_string())?;
                deployer::deploy_mod(
                    &db,
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
                deployer::undeploy_mod(
                    &db,
                    &game_id,
                    &bottle_name,
                    mod_id,
                    &data_dir,
                    &game.game_path,
                )
                .map_err(|e| e.to_string())?;
            }

            let _ = deploy_journal::complete(&journal_id);

            // Sync Skyrim plugins if applicable
            if game_id == "skyrimse" {
                let _ = sync_plugins_for_game(&game, &bottle);
            }
        }
        // Legacy mods (no staging_path): only the DB flag changes

        Ok(())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Batch enable/disable mods. For bulk operations (>=5 mods), updates all DB
/// flags first then does a single `redeploy_all` pass instead of per-mod
/// deploy/undeploy — dramatically faster for large batches.
#[tauri::command]
async fn batch_toggle_mods(
    mod_ids: Vec<i64>,
    game_id: String,
    bottle_name: String,
    enabled: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    check_game_lock(&state.game_locks, &game_id, &bottle_name)?;
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;

        let total = mod_ids.len();
        let action = if enabled { "Enabling" } else { "Disabling" };

        const BATCH_REDEPLOY_THRESHOLD: usize = 5;

        if mod_ids.len() >= BATCH_REDEPLOY_THRESHOLD {
            // Fast path: batch DB updates then single redeploy_all
            let mut toggled: Vec<(i64, String)> = Vec::new();
            let mut errors: Vec<String> = Vec::new();

            for (i, mod_id) in mod_ids.iter().enumerate() {
                let installed_mod = match db.get_mod(*mod_id) {
                    Ok(Some(m)) => m,
                    Ok(None) => continue,
                    Err(e) => {
                        errors.push(format!("mod {}: {}", mod_id, e));
                        continue;
                    }
                };
                let _ = app.emit(
                    "bulk-operation-progress",
                    serde_json::json!({
                        "phase": "toggle",
                        "current": i + 1,
                        "total": total,
                        "message": format!("{} {}", action, &installed_mod.name),
                    }),
                );
                if installed_mod.enabled == enabled {
                    continue;
                }
                if let Err(e) = db.set_enabled(*mod_id, enabled) {
                    errors.push(format!("{}: {}", installed_mod.name, e));
                    continue;
                }
                toggled.push((*mod_id, installed_mod.name.clone()));
            }

            if !toggled.is_empty() {
                let _ = app.emit(
                    "bulk-operation-progress",
                    serde_json::json!({
                        "phase": "redeploy",
                        "current": 0,
                        "total": 1,
                        "message": format!("Redeploying {} mods...", toggled.len()),
                    }),
                );
                // Single redeploy pass for all enabled mods
                if let Err(e) =
                    deployer::redeploy_all(&db, &game_id, &bottle_name, &data_dir, &game.game_path)
                {
                    // Revert all DB flags on failure
                    for (mod_id, _) in &toggled {
                        let _ = db.set_enabled(*mod_id, !enabled);
                    }
                    return Err(format!(
                        "Redeploy failed after toggling {} mods: {}",
                        toggled.len(),
                        e
                    ));
                }
            }

            if game_id == "skyrimse" {
                let _ = app.emit(
                    "bulk-operation-progress",
                    serde_json::json!({
                        "phase": "plugins",
                        "current": 0,
                        "total": 1,
                        "message": "Syncing plugins.txt...",
                    }),
                );
                let _ = sync_plugins_for_game(&game, &bottle);
            }

            let count = toggled.len();
            let _ = app.emit("bulk-operation-progress", serde_json::json!({
            "phase": "done",
            "current": count,
            "total": count,
            "message": format!("{} {} mod{}", action, count, if count == 1 { "" } else { "s" }),
        }));
            if errors.is_empty() {
                Ok(format!("{}", count))
            } else {
                Err(format!(
                    "{} succeeded, {} failed: {}",
                    count,
                    errors.len(),
                    errors.join("; ")
                ))
            }
        } else {
            // Small batch: per-mod deploy/undeploy (preserves granular error reporting)
            let mut count = 0u32;
            let mut errors: Vec<String> = Vec::new();

            for mod_id in &mod_ids {
                let installed_mod = match db.get_mod(*mod_id) {
                    Ok(Some(m)) => m,
                    Ok(None) => continue,
                    Err(e) => {
                        errors.push(format!("mod {}: {}", mod_id, e));
                        continue;
                    }
                };
                if installed_mod.enabled == enabled {
                    continue;
                }

                if let Err(e) = db.set_enabled(*mod_id, enabled) {
                    errors.push(format!("{}: {}", installed_mod.name, e));
                    continue;
                }

                if let Some(ref staging_path_str) = installed_mod.staging_path {
                    let staging_path = PathBuf::from(staging_path_str);
                    let result: Result<(), String> = if enabled {
                        staging::list_staging_files(&staging_path)
                            .map_err(|e| e.to_string())
                            .and_then(|files| {
                                deployer::deploy_mod(
                                    &db,
                                    &game_id,
                                    &bottle_name,
                                    *mod_id,
                                    &staging_path,
                                    &data_dir,
                                    &files,
                                )
                                .map(|_| ())
                                .map_err(|e| e.to_string())
                            })
                    } else {
                        deployer::undeploy_mod(
                            &db,
                            &game_id,
                            &bottle_name,
                            *mod_id,
                            &data_dir,
                            &game.game_path,
                        )
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                    };
                    if let Err(e) = result {
                        let _ = db.set_enabled(*mod_id, !enabled);
                        errors.push(format!("{}: {}", installed_mod.name, e));
                        continue;
                    }
                }
                count += 1;
            }

            if game_id == "skyrimse" {
                let _ = sync_plugins_for_game(&game, &bottle);
            }

            if errors.is_empty() {
                Ok(format!("{}", count))
            } else {
                Err(format!(
                    "{} succeeded, {} failed: {}",
                    count,
                    errors.len(),
                    errors.join("; ")
                ))
            }
        }
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn get_plugin_order(
    game_id: String,
    bottle_name: String,
) -> Result<Vec<PluginEntry>, String> {
    tokio::task::spawn_blocking(move || {
        if !plugins::skyrim_plugins::supports_plugin_order(&game_id) {
            return Ok(vec![]);
        }

        let (bottle, game, _) = resolve_game(&game_id, &bottle_name)?;

        // Auto-sync plugins.txt with deployed files before reading.
        // This ensures all deployed plugins are marked enabled and stale
        // entries for removed plugins are cleaned up.
        sync_plugins_for_game(&game, &bottle)?;

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
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn download_from_nexus(
    nxm_url: String,
    game_id: String,
    bottle_name: String,
    auto_install: bool,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let client = nexus_client().await?;

    let nxm = nexus::NXMLink::parse(&nxm_url).map_err(|e| e.to_string())?;

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
    let cfg = config::get_config().map_err(|e| e.to_string())?;
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
            let _ = sync_plugins_for_game(&game, &bottle);
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
    let method = oauth::get_auth_method_refreshed().await;
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
async fn get_config() -> Result<AppConfig, String> {
    tokio::task::spawn_blocking(move || config::get_config().map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn set_config_value(key: String, value: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        config::set_config_value(&key, &value).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// --- Download Archive Management ---

#[tauri::command]
async fn list_download_archives() -> Result<Vec<serde_json::Value>, String> {
    tokio::task::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn delete_download_archive(path: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn get_downloads_stats() -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn clear_all_download_archives() -> Result<u64, String> {
    tokio::task::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
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
async fn launch_game_cmd(
    game_id: String,
    bottle_name: String,
    use_skse: bool,
    state: State<'_, AppState>,
) -> Result<LaunchResult, String> {
    let db = state.db.clone();
    let game_locks = state.game_locks.clone();
    tokio::task::spawn_blocking(move || {
    let (bottle, game, _) = resolve_game(&game_id, &bottle_name)?;
    let game_path = PathBuf::from(&game.game_path);
    let data_dir_check = PathBuf::from(&game.data_dir);

    // Pre-launch self-healing: check if deployment is consistent, auto-fix if not
    {
        let manifest = db.get_deployment_manifest(&game_id, &bottle_name).unwrap_or_default();
        if !manifest.is_empty() {
            let missing: usize = manifest.iter()
                .filter(|e| !data_dir_check.join(&e.relative_path).exists())
                .count();
            if missing > 0 {
                log::warn!(
                    "Pre-launch: {} deployed files missing — triggering self-heal redeploy",
                    missing
                );
                match deployer::redeploy_all(
                    &db, &game_id, &bottle_name, &data_dir_check, &game.game_path,
                ) {
                    Ok(_) => log::info!("Pre-launch: self-heal redeploy succeeded"),
                    Err(e) => log::error!("Pre-launch: self-heal redeploy failed: {}", e),
                }
            }
        }
    }

    // When SKSE is requested, it takes priority over custom executables.
    // Otherwise, check for a custom default executable first.
    if !use_skse {
        let custom_exe =
            executables::get_default_executable(&db, &game_id, &bottle_name).unwrap_or(None);

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

    // Detect game version once for both SKSE compat check and version guard
    let detected_version = if game_id == "skyrimse" {
        downgrader::detect_skyrim_version(&game_path).ok()
    } else {
        None
    };

    // Pre-launch SKSE compatibility check — warn on version mismatch
    let mut skse_warning: Option<String> = None;
    if use_skse && game_id == "skyrimse" {
        let skse_status = skse::detect_skse(&game_path);
        if let Some(ref downgrade_status) = detected_version {
            let compat = skse::check_skse_compatibility(&skse_status, downgrade_status);
            if !compat.compatible {
                log::warn!(
                    "SKSE compatibility issue: {} (SKSE={:?}, Game={})",
                    compat.message,
                    compat.skse_version,
                    compat.game_version
                );
                skse_warning = Some(compat.message);
            } else {
                log::info!(
                    "SKSE compatibility OK: SKSE={:?}, Game={}",
                    compat.skse_version,
                    compat.game_version
                );
            }
        }
    }

    // Pre-launch version guard — check active collection's target version
    if let Some(ref downgrade_status) = detected_version {
        let collections = db
            .list_installed_collections(&game_id, &bottle_name)
            .unwrap_or_default();
        let metadata_list = db
            .list_collection_metadata(&game_id, &bottle_name)
            .unwrap_or_default();

        let active_versions: Option<Vec<String>> = collections
            .iter()
            .find(|(_, _, enabled)| *enabled > 0)
            .and_then(|(name, _, _)| metadata_list.iter().find(|m| m.collection_name == *name))
            .and_then(|m| m.manifest_json.as_ref())
            .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
            .and_then(|v| v.get("gameVersions").cloned())
            .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok());

        if let Some(target_versions) = active_versions {
            if !target_versions.is_empty() {
                let current = &downgrade_status.current_version;
                let is_se = current.starts_with("1.5.");
                let targets_se = target_versions.iter().any(|v| v.starts_with("1.5."));
                let targets_ae = target_versions.iter().any(|v| v.starts_with("1.6."));

                let mismatch =
                    (is_se && !targets_se && targets_ae) || (!is_se && targets_se && !targets_ae);

                if mismatch {
                    let target_label = if targets_se {
                        "SE (1.5.x)"
                    } else {
                        "AE (1.6.x)"
                    };
                    let current_label = if is_se { "SE" } else { "AE" };
                    let warning_msg = format!(
                        "Version mismatch: Active collection targets Skyrim {} but your game is {}. \
                         You may experience crashes or incompatible mods. \
                         Use Settings → Game Version to switch.",
                        target_label, current_label
                    );

                    skse_warning = Some(match skse_warning {
                        Some(existing) => format!("{} | {}", existing, warning_msg),
                        None => warning_msg,
                    });
                }
            }
        }
    }

    // Check if user has disabled automatic game launch fixes
    let fixes_disabled = config::get_config_value("disable_game_fixes")
        .unwrap_or(None)
        .map(|v| v == "true")
        .unwrap_or(false);

    // Auto-apply display fix for Skyrim SE before launching to ensure fullscreen
    if game_id == "skyrimse" && !fixes_disabled {
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

    // Pre-launch plugin sync — ensure plugins.txt reflects all deployed
    // plugins as enabled.  This catches any staleness from the game itself
    // rewriting the file on a previous exit/crash.
    let _ = sync_plugins_for_game(&game, &bottle);

    // Pre-launch SKSE plugin DLL version fix — swap incompatible plugins
    // for compatible alternatives from other installed mods' staging dirs.
    let mut wine_plugin_warning: Option<String> = None;
    if game_id == "skyrimse" {
        let data_dir = PathBuf::from(&game.data_dir);
        let skse_fixes = skse::fix_skse_plugin_conflicts(
            &db,
            &game_id,
            &bottle_name,
            &data_dir,
            &game_path,
        );
        if skse_fixes > 0 {
            log::info!(
                "Pre-launch: swapped {} incompatible SKSE plugin DLL(s)",
                skse_fixes
            );
        }

        // EngineFixes Wine compatibility: disable all patches (they crash under Wine)
        let ef_fixes =
            skse::fix_engine_fixes_for_wine(&data_dir, &db, &game_id, &bottle_name);
        if ef_fixes > 0 {
            log::info!(
                "Pre-launch: patched {} EngineFixes TOML(s) for Wine compatibility",
                ef_fixes
            );
        }

        // Disable Wine-incompatible SKSE plugins (CrashLogger, etc.)
        let wine_disabled =
            skse::disable_wine_incompatible_plugins(&data_dir, &db, &game_id, &bottle_name);
        if !wine_disabled.is_empty() {
            let names: Vec<&str> = wine_disabled.iter().map(|(n, _)| n.as_str()).collect();
            log::info!(
                "Pre-launch: disabled Wine-incompatible plugin(s): {}",
                names.join(", ")
            );
            let msg = format!(
                "Disabled Wine-incompatible plugin(s): {}. See Settings > Game > Wine-Incompatible Plugins to manage.",
                names.join(", ")
            );
            wine_plugin_warning = Some(msg);
        }

        // Auto-deploy SSE Engine Fixes for Wine (Wine-safe replacement)
        match skse::install_engine_fixes_wine_blocking(&data_dir) {
            Ok(true) => log::info!("Pre-launch: auto-deployed SSE Engine Fixes for Wine"),
            Ok(false) => log::debug!("Pre-launch: SSE Engine Fixes for Wine already deployed"),
            Err(e) => log::warn!(
                "Pre-launch: could not auto-deploy SSE Engine Fixes for Wine: {}",
                e
            ),
        }
    }

    let mut result = launcher::launch_game(&bottle, &exe_path, Some(&game_path))
        .map_err(|e| format!("Launch failed ({}): {}", bottle.source, e))?;

    // Cursor fix is now handled by Wine registry keys (set in auto_fix_display
    // above via fix_cursor_grab). No runtime Dock/Hot Corner/event tap needed.

    // Attach Wine-incompatible plugin warning
    if let Some(w) = wine_plugin_warning {
        result.warning = Some(match result.warning {
            Some(existing) => format!("{}\n{}", existing, w),
            None => w,
        });
    }

    // Attach any SKSE compatibility warning to the launch result
    if let Some(warning) = skse_warning {
        result.warning = Some(warning);
    }

    // Register game lock so frontend can show "game is running" banner
    if result.success {
        if let Some(pid) = result.pid {
            game_locks.register(&game_id, &bottle_name, pid);
        }
    }

    Ok(result)
    }).await.map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn check_skse(game_id: String, bottle_name: String) -> Result<SkseStatus, String> {
    tokio::task::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
fn get_skse_download_url() -> String {
    skse::skse_download_url().to_string()
}

#[tauri::command]
async fn install_skse_from_archive_cmd(
    game_id: String,
    bottle_name: String,
    archive_path: String,
) -> Result<SkseStatus, String> {
    tokio::task::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn uninstall_skse_cmd(game_id: String, bottle_name: String) -> Result<SkseStatus, String> {
    tokio::task::spawn_blocking(move || {
        if game_id != "skyrimse" {
            return Err("SKSE is only available for Skyrim Special Edition".to_string());
        }

        let (_, game, _) = resolve_game(&game_id, &bottle_name)?;
        let game_path = PathBuf::from(&game.game_path);

        let mut status = skse::uninstall_skse(&game_path).map_err(|e| e.to_string())?;

        // Disable SKSE preference after uninstall
        if !status.installed {
            let _ = skse::set_skse_preference(&game_id, &bottle_name, false);
            status.use_skse = false;
        }

        Ok(status)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn set_skse_preference_cmd(
    game_id: String,
    bottle_name: String,
    enabled: bool,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        skse::set_skse_preference(&game_id, &bottle_name, enabled).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn check_skyrim_version(
    game_id: String,
    bottle_name: String,
) -> Result<DowngradeStatus, String> {
    tokio::task::spawn_blocking(move || {
        if game_id != "skyrimse" {
            return Err("Version check is only available for Skyrim SE".to_string());
        }

        let (_, game, _) = resolve_game(&game_id, &bottle_name)?;
        downgrader::detect_skyrim_version(Path::new(&game.game_path)).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn check_skse_compatibility_cmd(
    game_id: String,
    bottle_name: String,
) -> Result<skse::SkseCompatibility, String> {
    tokio::task::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn get_skse_builds(
    game_id: String,
    bottle_name: String,
) -> Result<skse::SkseAvailableBuilds, String> {
    tokio::task::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
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
async fn scan_skse_plugins_cmd(
    game_id: String,
    bottle_name: String,
) -> Result<skse::SksePluginScanResult, String> {
    tokio::task::spawn_blocking(move || {
        if game_id != "skyrimse" {
            return Err("SKSE plugin scan is only available for Skyrim SE".into());
        }

        let (_, game, data_dir) = resolve_game(&game_id, &bottle_name)?;
        let game_path = PathBuf::from(&game.game_path);
        let version = downgrader::detect_skyrim_version(&game_path)
            .map(|s| s.current_version)
            .unwrap_or_else(|_| "unknown".to_string());

        Ok(skse::scan_skse_plugins(&data_dir, &version))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn fix_skse_plugins_cmd(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<usize, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let (_, game, data_dir) = resolve_game(&game_id, &bottle_name)?;
        let game_path = PathBuf::from(&game.game_path);
        Ok(skse::fix_skse_plugin_conflicts(
            &db,
            &game_id,
            &bottle_name,
            &data_dir,
            &game_path,
        ))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// List SKSE plugins that have been auto-disabled for Wine compatibility.
#[tauri::command]
async fn list_disabled_wine_plugins_cmd(
    game_id: String,
    bottle_name: String,
) -> Result<Vec<(String, String)>, String> {
    tokio::task::spawn_blocking(move || {
        if game_id != "skyrimse" {
            return Ok(vec![]);
        }
        let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
        Ok(skse::list_disabled_wine_plugins(&data_dir))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Re-enable a Wine-incompatible plugin that was auto-disabled (user override).
#[tauri::command]
async fn reenable_wine_plugin_cmd(
    game_id: String,
    bottle_name: String,
    dll_name: String,
) -> Result<bool, String> {
    tokio::task::spawn_blocking(move || {
        if game_id != "skyrimse" {
            return Ok(false);
        }
        let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
        skse::reenable_wine_plugin(&data_dir, &dll_name)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn fix_skyrim_display(bottle_name: String) -> Result<display_fix::DisplayFixResult, String> {
    tokio::task::spawn_blocking(move || {
        let bottle = resolve_bottle(&bottle_name)?;
        display_fix::auto_fix_display(&bottle)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
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
async fn get_depot_download_command(
    game_id: String,
    bottle_name: String,
) -> Result<downgrader::DepotDownloadInfo, String> {
    tokio::task::spawn_blocking(move || {
        let (bottle, _, _) = resolve_game(&game_id, &bottle_name)?;
        downgrader::get_depot_download_info(&game_id, &bottle.path).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn start_depot_download(game_id: String) -> Result<bool, String> {
    tokio::task::spawn_blocking(move || {
        if game_id != "skyrimse" {
            return Err("Depot download only supported for Skyrim SE".into());
        }
        downgrader::send_depot_command_to_steam()
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn check_depot_ready(game_id: String, bottle_name: String) -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || {
        let (bottle, _, _) = resolve_game(&game_id, &bottle_name)?;
        let steam_dir = downgrader::find_steam_dir(&bottle.path)
            .ok_or_else(|| "Steam directory not found in bottle".to_string())?;

        Ok(downgrader::check_depot_downloaded(
            &steam_dir,
            downgrader::SKYRIM_APP_ID,
            downgrader::SKYRIM_DEPOT_ID,
        )
        .map(|p| p.to_string_lossy().into_owned()))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn apply_downgrade_cmd(
    game_id: String,
    bottle_name: String,
) -> Result<DowngradeStatus, String> {
    tokio::task::spawn_blocking(move || {
        let (bottle, game, _) = resolve_game(&game_id, &bottle_name)?;
        let game_path = PathBuf::from(&game.game_path);
        let steam_dir = downgrader::find_steam_dir(&bottle.path)
            .ok_or_else(|| "Steam directory not found in bottle".to_string())?;

        let depot_exe = downgrader::check_depot_downloaded(
            &steam_dir,
            downgrader::SKYRIM_APP_ID,
            downgrader::SKYRIM_DEPOT_ID,
        )
        .ok_or_else(|| {
            "Depot files not downloaded yet. Run download_depot in Steam console first.".to_string()
        })?;

        downgrader::apply_depot_downgrade(&game_path, &depot_exe, &game_id)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn list_game_versions(game_id: String) -> Result<Vec<downgrader::CachedVersion>, String> {
    tokio::task::spawn_blocking(move || Ok(downgrader::list_cached_versions(&game_id)))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn swap_game_version(
    game_id: String,
    bottle_name: String,
    target_version: String,
) -> Result<DowngradeStatus, String> {
    let (_, game, _) = resolve_game(&game_id, &bottle_name)?;
    let game_path = PathBuf::from(&game.game_path);

    // Cache current version before swapping
    if let Err(e) = downgrader::cache_current_version(&game_path, &game_id) {
        log::warn!("Failed to cache current version before swap: {}", e);
    }

    let status = downgrader::swap_to_version(&game_path, &game_id, &target_version)
        .map_err(|e| e.to_string())?;

    // Auto-reinstall SKSE for the new version if SKSE preference is enabled
    if game_id == "skyrimse" && skse::get_skse_preference(&game_id, &bottle_name) {
        match skse::install_skse_auto(&game_path, &target_version).await {
            Ok(skse_status) => {
                log::info!(
                    "Auto-reinstalled SKSE for version {}: {:?}",
                    target_version,
                    skse_status.version
                );
            }
            Err(e) => {
                log::warn!("SKSE auto-reinstall failed after version swap: {}", e);
            }
        }
    }

    Ok(status)
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
async fn add_custom_exe(
    game_id: String,
    bottle_name: String,
    name: String,
    exe_path: String,
    working_dir: Option<String>,
    args: Option<String>,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        executables::add_executable(
            &db,
            &game_id,
            &bottle_name,
            &name,
            &exe_path,
            working_dir.as_deref(),
            args.as_deref(),
        )
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn remove_custom_exe(exe_id: i64, state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || executables::remove_executable(&db, exe_id))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn list_custom_exes(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<CustomExecutable>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || executables::list_executables(&db, &game_id, &bottle_name))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn set_default_exe(
    game_id: String,
    bottle_name: String,
    exe_id: Option<i64>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || match exe_id {
        Some(id) => executables::set_default_executable(&db, &game_id, &bottle_name, id),
        None => executables::clear_default_executable(&db, &game_id, &bottle_name),
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// --- Deployment Management ---

#[tauri::command]
async fn get_conflicts(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<FileConflict>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        db.find_all_conflicts(&game_id, &bottle_name)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Conflict detection task failed: {e}"))?
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct AnalyzeConflictsResponse {
    suggestions: Vec<conflict_resolver::ConflictSuggestion>,
    identical_stats: conflict_resolver::IdenticalContentStats,
}

#[tauri::command]
async fn analyze_conflicts_cmd(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<AnalyzeConflictsResponse, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
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

        // Batch-fetch file hashes for checksum-based conflict auto-resolution.
        let mod_ids: Vec<i64> = conflicts
            .iter()
            .flat_map(|c| c.mods.iter().map(|m| m.mod_id))
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        let file_hashes = db.get_file_hashes_bulk(&mod_ids).unwrap_or_default();

        let (suggestions, identical_stats) =
            conflict_resolver::analyze_conflicts(&conflicts, &mods, loot_ref, &file_hashes);
        Ok(AnalyzeConflictsResponse {
            suggestions,
            identical_stats,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn resolve_all_conflicts_cmd(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<conflict_resolver::ResolutionResult, String> {
    check_game_lock(&state.game_locks, &game_id, &bottle_name)?;
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
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

        // Batch-fetch file hashes for checksum-based conflict auto-resolution.
        let mod_ids: Vec<i64> = conflicts
            .iter()
            .flat_map(|c| c.mods.iter().map(|m| m.mod_id))
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        let file_hashes = db.get_file_hashes_bulk(&mod_ids).unwrap_or_default();

        let (suggestions, _identical_stats) =
            conflict_resolver::analyze_conflicts(&conflicts, &mods, loot_ref, &file_hashes);
        let result =
            conflict_resolver::apply_suggestions(&db, &game_id, &bottle_name, &suggestions)?;

        // Record conflict rules for resolved conflicts so they disappear from the list.
        for suggestion in &suggestions {
            match suggestion.status {
                conflict_resolver::ConflictStatus::AuthorResolved
                | conflict_resolver::ConflictStatus::IdenticalContent => {
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
            deployer::redeploy_all(&db, &game_id, &bottle_name, &data_dir, &game.game_path)
                .map_err(|e| e.to_string())?;
            if game_id == "skyrimse" {
                let bottle = resolve_bottle(&bottle_name)?;
                let _ = sync_plugins_for_game(&game, &bottle);
            }
        }

        Ok(result)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn record_conflict_winner(
    game_id: String,
    bottle_name: String,
    winner_mod_id: i64,
    loser_mod_ids: Vec<i64>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        for loser_id in loser_mod_ids {
            db.add_conflict_rule(&game_id, &bottle_name, winner_mod_id, loser_id)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn get_deployment_manifest_cmd(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<DeploymentEntry>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        db.get_deployment_manifest(&game_id, &bottle_name)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn set_mod_priority(
    mod_id: i64,
    priority: i32,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        db.set_mod_priority(mod_id, priority)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn reorder_mods(
    game_id: String,
    bottle_name: String,
    ordered_mod_ids: Vec<i64>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    check_game_lock(&state.game_locks, &game_id, &bottle_name)?;
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        db.reorder_priorities(&game_id, &bottle_name, &ordered_mod_ids)
            .map_err(|e| e.to_string())?;

        let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;

        // Redeploy to reflect new priority order
        deployer::redeploy_all(&db, &game_id, &bottle_name, &data_dir, &game.game_path)
            .map_err(|e| e.to_string())?;

        // Sync plugins after redeploy
        if game_id == "skyrimse" {
            let _ = sync_plugins_for_game(&game, &bottle);
        }

        Ok(())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn redeploy_all_mods(
    app: AppHandle,
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    check_game_lock(&state.game_locks, &game_id, &bottle_name)?;
    let db = state.db.clone();
    let app = app.clone();
    tokio::task::spawn_blocking(move || {
        let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;

        let journal_id = deploy_journal::begin(
            &game_id, &bottle_name, deploy_journal::JournalOp::RedeployAll, &[],
        ).unwrap_or_default();

        let app_clone = app.clone();
        let result = deployer::redeploy_all_with_progress(
            &db,
            &game_id,
            &bottle_name,
            &data_dir,
            &game.game_path,
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

        let _ = deploy_journal::complete(&journal_id);

        if game_id == "skyrimse" {
            let _ = sync_plugins_for_game(&game, &bottle);
            let ef = skse::fix_engine_fixes_for_wine(&data_dir, &db, &game_id, &bottle_name);
            if ef > 0 {
                log::info!(
                    "Redeploy: patched {} EngineFixes TOML(s) for Wine compatibility",
                    ef
                );
            }
            // Disable Wine-incompatible SKSE plugins
            let wine_disabled =
                skse::disable_wine_incompatible_plugins(&data_dir, &db, &game_id, &bottle_name);
            for (name, reason) in &wine_disabled {
                log::info!(
                    "Redeploy: disabled Wine-incompatible plugin {} — {}",
                    name,
                    reason
                );
            }
            // Auto-deploy SSE Engine Fixes for Wine on redeploy
            match skse::install_engine_fixes_wine_blocking(&data_dir) {
                Ok(true) => log::info!("Redeploy: auto-deployed SSE Engine Fixes for Wine"),
                Ok(false) => {}
                Err(e) => log::warn!(
                    "Redeploy: could not auto-deploy SSE Engine Fixes for Wine: {}",
                    e
                ),
            }
        }

        Ok(serde_json::json!({
            "deployed_count": result.deployed_count,
            "skipped_count": result.skipped_count,
            "fallback_used": result.fallback_used,
        }))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Incremental deployment: compute diff and apply only changes.
/// Falls back to full redeploy if >80% of files would change.
#[tauri::command]
async fn deploy_incremental_cmd(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<deployer::IncrementalDeployResult, String> {
    check_game_lock(&state.game_locks, &game_id, &bottle_name)?;
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;

        let result =
            deployer::deploy_incremental(&db, &game_id, &bottle_name, &data_dir, &game.game_path)
                .map_err(|e| e.to_string())?;

        if game_id == "skyrimse" {
            let _ = sync_plugins_for_game(&game, &bottle);
            let ef = skse::fix_engine_fixes_for_wine(&data_dir, &db, &game_id, &bottle_name);
            if ef > 0 {
                log::info!(
                    "Incremental deploy: patched {} EngineFixes TOML(s) for Wine compatibility",
                    ef
                );
            }
            // Disable Wine-incompatible SKSE plugins
            let wine_disabled =
                skse::disable_wine_incompatible_plugins(&data_dir, &db, &game_id, &bottle_name);
            for (name, reason) in &wine_disabled {
                log::info!(
                    "Incremental deploy: disabled Wine-incompatible plugin {} — {}",
                    name,
                    reason
                );
            }
            match skse::install_engine_fixes_wine_blocking(&data_dir) {
                Ok(true) => {
                    log::info!("Incremental deploy: auto-deployed SSE Engine Fixes for Wine")
                }
                Ok(false) => {}
                Err(e) => log::warn!(
                    "Incremental deploy: could not auto-deploy SSE Engine Fixes for Wine: {}",
                    e
                ),
            }
        }

        Ok(result)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Check deployment health: verify mods have staging dirs and deployed files.
/// Verification depth is controlled by the `verification_level` config setting:
/// - Fast: file existence only
/// - Balanced: existence + spot-check 10% of files by SHA-256
/// - Paranoid: existence + full SHA-256 verification of every file
#[tauri::command]
async fn check_deployment_health(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let (_bottle, _game, data_dir) = resolve_game(&game_id, &bottle_name)?;

        // Read verification level from config
        let verification_level = config::get_config()
            .map(|c| c.verification_level)
            .unwrap_or_default();

        let mods = db
            .list_mods(&game_id, &bottle_name)
            .map_err(|e| e.to_string())?;
        let manifest = db
            .get_deployment_manifest(&game_id, &bottle_name)
            .map_err(|e| e.to_string())?;

        let mut enabled_count = 0usize;
        let mut staging_ok = 0usize;
        let mut staging_missing = 0usize;
        let mut staging_empty = 0usize;
        let mut no_staging_path = 0usize;
        let mut missing_mods: Vec<serde_json::Value> = Vec::new();

        for m in &mods {
            if !m.enabled {
                continue;
            }
            enabled_count += 1;
            match &m.staging_path {
                Some(sp) => {
                    let p = std::path::Path::new(sp);
                    if !p.exists() {
                        staging_missing += 1;
                        if missing_mods.len() < 20 {
                            missing_mods.push(serde_json::json!({
                                "id": m.id,
                                "name": m.name,
                                "issue": "staging_missing",
                            }));
                        }
                    } else {
                        let files = staging::list_staging_files(p).unwrap_or_default();
                        if files.is_empty() {
                            staging_empty += 1;
                            if missing_mods.len() < 20 {
                                missing_mods.push(serde_json::json!({
                                    "id": m.id,
                                    "name": m.name,
                                    "issue": "staging_empty",
                                }));
                            }
                        } else {
                            staging_ok += 1;
                        }
                    }
                }
                None => {
                    no_staging_path += 1;
                    if missing_mods.len() < 20 {
                        missing_mods.push(serde_json::json!({
                            "id": m.id,
                            "name": m.name,
                            "issue": "no_staging_path",
                        }));
                    }
                }
            }
        }

        // Check deployment manifest vs data dir (existence check — all modes)
        let mut deployed_ok = 0usize;
        let mut deployed_missing = 0usize;
        for entry in &manifest {
            let file_path = data_dir.join(&entry.relative_path);
            if file_path.exists() {
                deployed_ok += 1;
            } else {
                deployed_missing += 1;
            }
        }

        // Hash verification (Balanced/Paranoid modes only)
        let verification = deployer::verify_deployment(
            &verification_level,
            &db,
            &game_id,
            &bottle_name,
            &data_dir,
        )
        .map_err(|e| e.to_string())?;

        let healthy = staging_missing == 0
            && staging_empty == 0
            && no_staging_path == 0
            && deployed_missing == 0
            && verification.hash_mismatches == 0
            && !manifest.is_empty();

        let level_str = match verification_level {
            config::VerificationLevel::Fast => "Fast",
            config::VerificationLevel::Balanced => "Balanced",
            config::VerificationLevel::Paranoid => "Paranoid",
        };

        Ok(serde_json::json!({
            "healthy": healthy,
            "total_mods": mods.len(),
            "enabled_mods": enabled_count,
            "staging_ok": staging_ok,
            "staging_missing": staging_missing,
            "staging_empty": staging_empty,
            "no_staging_path": no_staging_path,
            "manifest_entries": manifest.len(),
            "deployed_files_ok": deployed_ok,
            "deployed_files_missing": deployed_missing,
            "problem_mods": missing_mods,
            "needs_reinstall": staging_missing > 0 || staging_empty > 0,
            "needs_redeploy": staging_ok > 0 && manifest.is_empty(),
            "verification_level": level_str,
            "hash_checked": verification.hash_checked,
            "hash_mismatches": verification.hash_mismatches,
            "hash_skipped_no_record": verification.hash_skipped_no_record,
            "mismatched_files": verification.mismatched_files,
        }))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Get the current verification level from config.
#[tauri::command]
async fn get_verification_level() -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let cfg = config::get_config().map_err(|e| e.to_string())?;
        let level = match cfg.verification_level {
            config::VerificationLevel::Fast => "Fast",
            config::VerificationLevel::Balanced => "Balanced",
            config::VerificationLevel::Paranoid => "Paranoid",
        };
        Ok(level.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Set the verification level in config.
#[tauri::command]
async fn set_verification_level(level: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        config::set_config_value("verification_level", &level).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn purge_deployment_cmd(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    check_game_lock(&state.game_locks, &game_id, &bottle_name)?;
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;

        auto_snapshot_before_destructive(&db, &game_id, &bottle_name, "Before purge deployment");

        let journal_id = deploy_journal::begin(
            &game_id, &bottle_name, deploy_journal::JournalOp::Purge, &[],
        ).unwrap_or_default();

        let removed =
            deployer::purge_deployment(&db, &game_id, &bottle_name, &data_dir, &game.game_path)
                .map_err(|e| e.to_string())?;

        let _ = deploy_journal::complete(&journal_id);

        if game_id == "skyrimse" {
            let _ = sync_plugins_for_game(&game, &bottle);
        }

        Ok(removed)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn verify_mod_integrity(
    mod_id: i64,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let installed_mod = db
            .get_mod(mod_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Mod with ID {} not found", mod_id))?;

        let staging_path = installed_mod
            .staging_path
            .as_ref()
            .ok_or_else(|| "Legacy mod — no staging data for integrity check".to_string())?;

        let hashes = db.get_file_hashes(mod_id).map_err(|e| e.to_string())?;
        staging::verify_staging_integrity(Path::new(staging_path), &hashes)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// --- Deployment Health ---

#[tauri::command]
async fn get_deployment_health(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| format!("Health task failed: {e}"))?
}

/// Lightweight deployment stats without the expensive `find_all_conflicts()` call.
/// The frontend already loads conflicts separately via `get_conflicts`, so this
/// avoids computing them twice.  Used after mod toggle and for sidebar stats.
#[tauri::command]
async fn get_deployment_stats(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let (total_mods, total_enabled) = db
            .get_mod_counts(&game_id, &bottle_name)
            .map_err(|e| e.to_string())?;
        let total_deployed = db
            .get_deployment_count(&game_id, &bottle_name)
            .map_err(|e| e.to_string())?;
        let is_deployed = total_deployed > 0;

        let deploy_method = if is_deployed {
            match resolve_game(&game_id, &bottle_name) {
                Ok((_, _, data_dir)) => {
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
            "deploy_method": deploy_method,
            "is_deployed": is_deployed,
        }))
    })
    .await
    .map_err(|e| format!("Stats task failed: {e}"))?
}

// --- Background Hashing ---

#[tauri::command]
async fn start_background_hashing(
    app: AppHandle,
    game_id: String,
    bottle_name: String,
    game_pid: Option<u32>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = Arc::clone(&state.db);
    let gid = game_id.clone();
    let bn = bottle_name.clone();

    tauri::async_runtime::spawn_blocking(move || {
        background_hash::run_background_hashing(&db, &gid, &bn, game_pid, |progress| {
            let _ = app.emit("background-hashing-progress", &progress);
        });
    });

    Ok(())
}

#[tauri::command]
fn cancel_background_hashing() {
    background_hash::cancel();
}

// --- Collection Management ---

#[tauri::command]
async fn list_installed_collections_cmd(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<CollectionSummary>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
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
                // Extract game_versions from stored manifest JSON if available
                let game_versions = meta
                    .and_then(|m| m.manifest_json.as_ref())
                    .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
                    .and_then(|v| v.get("gameVersions").cloned())
                    .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
                    .unwrap_or_default();

                CollectionSummary {
                    name,
                    mod_count,
                    enabled_count,
                    slug: meta.and_then(|m| m.slug.clone()),
                    author: meta.and_then(|m| m.author.clone()),
                    image_url: meta.and_then(|m| m.image_url.clone()),
                    game_domain: meta.and_then(|m| m.game_domain.clone()),
                    installed_revision: meta.and_then(|m| m.installed_revision),
                    original_mod_count: meta.and_then(|m| m.total_mods),
                    game_versions,
                }
            })
            .collect())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn set_mod_collection_name_cmd(
    mod_id: i64,
    collection_name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        db.set_collection_name(mod_id, &collection_name)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn switch_collection_cmd(
    game_id: String,
    bottle_name: String,
    collection_name: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    check_game_lock(&state.game_locks, &game_id, &bottle_name)?;
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;

        // 1. Purge current deployment
        deployer::purge_deployment(&db, &game_id, &bottle_name, &data_dir, &game.game_path)
            .map_err(|e| e.to_string())?;

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
            deployer::redeploy_all(&db, &game_id, &bottle_name, &data_dir, &game.game_path)
                .map_err(|e| e.to_string())?;

        // 5. Sync plugins if Skyrim SE
        if game_id == "skyrimse" {
            let _ = sync_plugins_for_game(&game, &bottle);
        }

        Ok(serde_json::json!({
            "deployed_count": result.deployed_count,
            "active_collection": collection_name,
        }))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn collection_download_size_cmd(
    game_id: String,
    bottle_name: String,
    collection_name: String,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        db.collection_unique_download_size(&game_id, &bottle_name, &collection_name)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn delete_collection_cmd(
    app: AppHandle,
    game_id: String,
    bottle_name: String,
    collection_name: String,
    delete_unique_downloads: bool,
    remove_all_mods: bool,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    check_game_lock(&state.game_locks, &game_id, &bottle_name)?;
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;

        auto_snapshot_before_destructive(
            &db,
            &game_id,
            &bottle_name,
            &format!("Before deleting collection: {}", collection_name),
        );

        // If "remove ALL mods" is selected, get every mod — not just the collection's.
        // This skips the per-mod restore_next_winner overhead entirely since nothing
        // remains to restore.
        let collection_mods = if remove_all_mods {
            db.list_mods(&game_id, &bottle_name)
                .map_err(|e| e.to_string())?
        } else {
            db.list_mods_by_collection(&game_id, &bottle_name, &collection_name)
                .map_err(|e| e.to_string())?
        };

        let total_mods = collection_mods.len();
        let mut mods_removed = 0usize;
        let mut downloads_removed = 0usize;
        let mut errors: Vec<String> = Vec::new();

        // Emit: uninstall started
        let _ = app.emit(
            "uninstall-progress",
            serde_json::json!({
                "kind": "uninstallStarted",
                "collection_name": &collection_name,
                "total_mods": total_mods,
            }),
        );

        // Collect plugin filenames for rule cleanup + mod IDs for bulk ops
        let mut plugin_names: Vec<String> = Vec::new();
        let mod_ids: Vec<i64> = collection_mods.iter().map(|m| m.id).collect();

        for m in &collection_mods {
            for file in &m.installed_files {
                let lower = file.to_lowercase();
                if lower.ends_with(".esp") || lower.ends_with(".esm") || lower.ends_with(".esl") {
                    if let Some(fname) = Path::new(file).file_name().and_then(|f| f.to_str()) {
                        plugin_names.push(fname.to_string());
                    }
                }
            }
        }

        // Phase 1: Bulk-remove all deployed files for collection mods.
        // This avoids the per-file `restore_next_winner` overhead — we do one
        // redeploy of remaining mods at the end instead.
        let _ = app.emit(
            "uninstall-progress",
            serde_json::json!({
                "kind": "modUninstalling",
                "mod_index": 0,
                "mod_name": "all collection mods",
                "step": "undeploying",
            }),
        );
        let deployed_paths = db
            .bulk_remove_deployment_entries(&mod_ids)
            .unwrap_or_default();
        let removed_count = std::sync::atomic::AtomicUsize::new(0);
        let path_total = deployed_paths.len();
        let game_path = game.game_path.clone();
        use rayon::prelude::*;
        deployed_paths
            .par_iter()
            .for_each(|(rel_path, deploy_target)| {
                let base = if deploy_target == "root" {
                    &game_path
                } else {
                    &data_dir
                };
                let file_path = base.join(rel_path);
                if file_path.exists() {
                    // Make writable before deleting
                    if let Ok(metadata) = std::fs::metadata(&file_path) {
                        let perms = metadata.permissions();
                        if perms.readonly() {
                            let mut writable = perms;
                            #[allow(clippy::permissions_set_readonly_false)]
                            writable.set_readonly(false);
                            let _ = std::fs::set_permissions(&file_path, writable);
                        }
                    }
                    let _ = std::fs::remove_file(&file_path);
                }
                let done = removed_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                if done.is_multiple_of(5000) || done == path_total {
                    let _ = app.emit(
                        "uninstall-progress",
                        serde_json::json!({
                            "kind": "modUninstalling",
                            "mod_index": 0,
                            "mod_name": format!("Removing files ({}/{})", done, path_total),
                            "step": "undeploying",
                        }),
                    );
                }
            });
        log::info!(
            "Bulk-removed {} deployed files for {} collection mods",
            path_total,
            total_mods
        );

        // Prune empty directories left behind after file removal.
        // Collect unique parent directories, sort deepest-first, and remove if empty.
        {
            let mut parent_dirs: std::collections::BTreeSet<PathBuf> =
                std::collections::BTreeSet::new();
            for (rel_path, deploy_target) in &deployed_paths {
                let base = if deploy_target == "root" {
                    &game_path
                } else {
                    &data_dir
                };
                let mut current = base.join(rel_path);
                while let Some(parent) = current.parent() {
                    if parent == data_dir || parent == game_path {
                        break;
                    }
                    parent_dirs.insert(parent.to_path_buf());
                    current = parent.to_path_buf();
                }
            }
            // Sort deepest-first so child dirs are removed before parents
            let mut sorted: Vec<_> = parent_dirs.into_iter().collect();
            sorted.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
            for dir in sorted {
                if dir.exists() {
                    let is_empty = std::fs::read_dir(&dir)
                        .map(|mut rd| rd.next().is_none())
                        .unwrap_or(false);
                    if is_empty {
                        let _ = std::fs::remove_dir(&dir);
                    }
                }
            }
        }

        // Phase 2: Clean staging + rollback dirs in parallel
        let _ = app.emit(
            "uninstall-progress",
            serde_json::json!({
                "kind": "modUninstalling",
                "mod_index": 0,
                "mod_name": "Cleaning staging directories",
                "step": "cleaning_staging",
            }),
        );
        collection_mods.par_iter().for_each(|m| {
            let _ = rollback::cleanup_mod_version_staging(&db, m.id);
            if let Some(sp) = &m.staging_path {
                let _ = std::fs::remove_dir_all(sp);
            }
        });

        // Phase 3: Handle download records.
        // Look up downloads via the collection_refs table directly (reliable),
        // rather than going through mod records (which may lack nexus IDs or
        // have full-path archive names that don't match the download registry).
        if delete_unique_downloads {
            let unique_downloads = db
                .get_unique_downloads_for_collection(&game_id, &bottle_name, &collection_name)
                .unwrap_or_default();
            for (dl_id, archive_path) in &unique_downloads {
                if std::fs::remove_file(archive_path).is_ok() {
                    downloads_removed += 1;
                    let _ = db.delete_download_record(*dl_id);
                } else {
                    log::warn!(
                        "Failed to delete archive (may already be removed): {}",
                        archive_path
                    );
                }
            }
        }
        // Clean up all collection refs for this collection
        let _ = db.remove_all_collection_download_refs(&collection_name, &game_id, &bottle_name);

        // Phase 4: Bulk-remove all mods from DB
        let _ = app.emit(
            "uninstall-progress",
            serde_json::json!({
                "kind": "modUninstalling",
                "mod_index": 0,
                "mod_name": "Cleaning database",
                "step": "cleaning_staging",
            }),
        );
        match db.bulk_remove_mods(&mod_ids) {
            Ok(count) => {
                mods_removed = count;
            }
            Err(e) => {
                errors.push(format!("Bulk DB removal failed: {}", e));
                // Fall back to per-mod removal
                for m in &collection_mods {
                    if let Err(e2) = db.remove_mod(m.id) {
                        errors.push(format!("Failed to remove '{}': {}", m.name, e2));
                    } else {
                        mods_removed += 1;
                    }
                }
            }
        }

        // Phase 5: Redeploy remaining mods (restores files that collection
        // mods were overwriting). Only needed if non-collection mods exist.
        let remaining_mods = db.list_mods(&game_id, &bottle_name).unwrap_or_default();
        if remaining_mods.iter().any(|m| m.enabled) {
            log::info!(
                "Redeploying {} remaining mods after collection removal",
                remaining_mods.len()
            );
            let _ = deployer::redeploy_all(&db, &game_id, &bottle_name, &data_dir, &game.game_path);
        }

        // Note: We intentionally do NOT call cleanup_orphaned_downloads() here.
        // Download registry entries should persist as a cache record even after
        // mods are uninstalled, so the cache % feature works correctly.
        // Entries are only deleted when the actual archive file is also removed.

        // Clean plugin rules for removed mods' plugins
        if !plugin_names.is_empty() {
            if let Err(e) =
                loot_rules::remove_rules_for_plugins(&db, &game_id, &bottle_name, &plugin_names)
            {
                errors.push(format!("Failed to clean plugin rules: {}", e));
            }
        }

        // Clean up collection metadata
        if let Err(e) = db.remove_collection_metadata(&game_id, &bottle_name, &collection_name) {
            errors.push(format!("Failed to remove collection metadata: {}", e));
        }

        // Clean up install checkpoint so "Resume Install" prompt doesn't appear
        if let Err(e) = db.delete_collection_checkpoints(&collection_name, &game_id, &bottle_name) {
            errors.push(format!("Failed to remove install checkpoint: {}", e));
        }

        // Clean up orphaned files left behind by partial installs.
        // remove_skse must be true here — collection mods deploy into SKSE/Plugins/
        // and those files become orphans once the collection is deleted.
        let clean_opts = cleaner::CleanOptions {
            remove_loose_files: true,
            remove_archives: true,
            remove_enb: false,
            remove_saves: false,
            remove_skse: true,
            orphans_only: true,
            dry_run: false,
            exclude_patterns: Vec::new(),
        };
        match cleaner::clean_game_directory(&db, &game_id, &bottle_name, &data_dir, &clean_opts) {
            Ok(result) => {
                if !result.removed_files.is_empty() {
                    log::info!(
                        "Cleaned {} orphaned files after deleting collection '{}'",
                        result.removed_files.len(),
                        collection_name,
                    );
                }
            }
            Err(e) => {
                errors.push(format!("Orphan cleanup failed: {}", e));
            }
        }

        // Emit: redeploy phase
        let _ = app.emit(
            "uninstall-progress",
            serde_json::json!({ "kind": "redeployStarted" }),
        );

        // Redeploy remaining mods to restore any files that were shadowed
        if let Err(e) =
            deployer::redeploy_all(&db, &game_id, &bottle_name, &data_dir, &game.game_path)
        {
            errors.push(format!("Failed to redeploy remaining mods: {}", e));
        }

        let _ = app.emit(
            "uninstall-progress",
            serde_json::json!({ "kind": "redeployCompleted" }),
        );

        if game_id == "skyrimse" {
            let _ = sync_plugins_for_game(&game, &bottle);
        }

        // Emit: uninstall completed
        let _ = app.emit(
            "uninstall-progress",
            serde_json::json!({
                "kind": "uninstallCompleted",
                "mods_removed": mods_removed,
                "downloads_removed": downloads_removed,
                "errors": &errors,
            }),
        );

        Ok(serde_json::json!({
            "mods_removed": mods_removed,
            "downloads_removed": downloads_removed,
            "errors": errors,
        }))
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))?
}

#[tauri::command]
async fn uninstall_wabbajack_modlist(
    app: AppHandle,
    game_id: String,
    bottle_name: String,
    modlist_name: String,
    delete_downloads: bool,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    // WJ installs use collection_name = "wj:{modlist_name}"
    let collection_name = format!("wj:{}", modlist_name);
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;

        auto_snapshot_before_destructive(
            &db,
            &game_id,
            &bottle_name,
            &format!("Before uninstalling WJ modlist: {}", modlist_name),
        );

        // Get mods in this WJ modlist collection
        let collection_mods = db
            .list_mods_by_collection(&game_id, &bottle_name, &collection_name)
            .map_err(|e| e.to_string())?;

        let total_mods = collection_mods.len();
        let mut mods_removed = 0usize;
        let mut downloads_removed = 0usize;
        let mut errors: Vec<String> = Vec::new();

        let _ = app.emit(
            "uninstall-progress",
            serde_json::json!({
                "kind": "uninstallStarted",
                "collection_name": &collection_name,
                "total_mods": total_mods,
            }),
        );

        let mut plugin_names: Vec<String> = Vec::new();

        for (idx, m) in collection_mods.iter().enumerate() {
            let _ = app.emit(
                "uninstall-progress",
                serde_json::json!({
                    "kind": "modUninstalling",
                    "mod_index": idx,
                    "mod_name": &m.name,
                    "step": "undeploying",
                }),
            );

            // Gather plugin filenames
            for file in &m.installed_files {
                let lower = file.to_lowercase();
                if lower.ends_with(".esp") || lower.ends_with(".esm") || lower.ends_with(".esl") {
                    if let Some(fname) = Path::new(file).file_name().and_then(|f| f.to_str()) {
                        plugin_names.push(fname.to_string());
                    }
                }
            }

            // Undeploy
            if let Err(e) = deployer::undeploy_mod(
                &db,
                &game_id,
                &bottle_name,
                m.id,
                &data_dir,
                &game.game_path,
            ) {
                errors.push(format!("Failed to undeploy '{}': {}", m.name, e));
            }

            // Clean rollback staging
            if let Err(e) = rollback::cleanup_mod_version_staging(&db, m.id) {
                errors.push(format!(
                    "Failed to clean rollback staging for '{}': {}",
                    m.name, e
                ));
            }

            // Remove staging
            if let Some(sp) = &m.staging_path {
                if let Err(e) = std::fs::remove_dir_all(sp) {
                    if Path::new(sp).exists() {
                        errors.push(format!("Failed to remove staging for '{}': {}", m.name, e));
                    }
                }
            }

            // Handle download cleanup
            let download =
                if let (Some(nmod_id), Some(nfile_id)) = (m.nexus_mod_id, m.nexus_file_id) {
                    db.find_download_by_nexus_ids(nmod_id, nfile_id)
                        .ok()
                        .flatten()
                } else {
                    None
                }
                .or_else(|| db.find_download_by_name(&m.archive_name).ok().flatten());

            if let Some(dl) = download {
                let is_unique = db
                    .is_download_unique_to_collection(dl.id, &collection_name)
                    .unwrap_or(false);

                if delete_downloads && is_unique {
                    if let Err(e) = std::fs::remove_file(&dl.archive_path) {
                        if Path::new(&dl.archive_path).exists() {
                            errors
                                .push(format!("Failed to delete download for '{}': {}", m.name, e));
                        }
                    } else {
                        downloads_removed += 1;
                        let _ = db.delete_download_record(dl.id);
                    }
                }

                if let Err(e) = db.remove_download_collection_ref(
                    dl.id,
                    &collection_name,
                    &game_id,
                    &bottle_name,
                ) {
                    errors.push(format!(
                        "Failed to remove download ref for '{}': {}",
                        m.name, e
                    ));
                }
            }

            // Remove from DB
            if let Err(e) = db.remove_mod(m.id) {
                errors.push(format!("Failed to remove mod '{}' from DB: {}", m.name, e));
            } else {
                mods_removed += 1;
                let _ = app.emit(
                    "uninstall-progress",
                    serde_json::json!({
                        "kind": "modUninstalled",
                        "mod_index": idx,
                        "mod_name": &m.name,
                    }),
                );
            }
        }

        // Clean plugin rules
        if !plugin_names.is_empty() {
            if let Err(e) =
                loot_rules::remove_rules_for_plugins(&db, &game_id, &bottle_name, &plugin_names)
            {
                errors.push(format!("Failed to clean plugin rules: {}", e));
            }
        }

        // Clean up collection metadata
        if let Err(e) = db.remove_collection_metadata(&game_id, &bottle_name, &collection_name) {
            errors.push(format!("Failed to remove collection metadata: {}", e));
        }

        // Redeploy remaining mods
        let _ = app.emit(
            "uninstall-progress",
            serde_json::json!({ "kind": "redeployStarted" }),
        );
        if let Err(e) =
            deployer::redeploy_all(&db, &game_id, &bottle_name, &data_dir, &game.game_path)
        {
            errors.push(format!("Failed to redeploy remaining mods: {}", e));
        }
        let _ = app.emit(
            "uninstall-progress",
            serde_json::json!({ "kind": "redeployCompleted" }),
        );

        if game_id == "skyrimse" {
            let _ = sync_plugins_for_game(&game, &bottle);
        }

        let _ = app.emit(
            "uninstall-progress",
            serde_json::json!({
                "kind": "uninstallCompleted",
                "mods_removed": mods_removed,
                "downloads_removed": downloads_removed,
                "errors": &errors,
            }),
        );

        Ok(serde_json::json!({
            "mods_removed": mods_removed,
            "downloads_removed": downloads_removed,
            "errors": errors,
        }))
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))?
}

#[tauri::command]
async fn restore_mod_snapshot(
    app: AppHandle,
    snapshot_id: i64,
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    check_game_lock(&state.game_locks, &game_id, &bottle_name)?;
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;

        let result = rollback::restore_snapshot(&db, snapshot_id, &game_id, &bottle_name)?;

        // Redeploy to apply the restored state
        let _ = app.emit(
            "deploy-progress",
            serde_json::json!({ "kind": "redeployStarted" }),
        );
        deployer::redeploy_all(&db, &game_id, &bottle_name, &data_dir, &game.game_path)
            .map_err(|e| format!("Failed to redeploy after snapshot restore: {}", e))?;
        let _ = app.emit(
            "deploy-progress",
            serde_json::json!({ "kind": "redeployCompleted" }),
        );

        if game_id == "skyrimse" {
            let _ = sync_plugins_for_game(&game, &bottle);
        }

        Ok(serde_json::json!({
            "mods_enabled": result.mods_enabled,
            "mods_disabled": result.mods_disabled,
            "mods_not_found": result.mods_not_found,
        }))
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))?
}

#[tauri::command]
async fn return_to_vanilla(
    game_id: String,
    bottle_name: String,
    clean_orphans: bool,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    check_game_lock(&state.game_locks, &game_id, &bottle_name)?;
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;

        // 1. Auto-snapshot
        auto_snapshot_before_destructive(&db, &game_id, &bottle_name, "Before return to vanilla");

        // 2. Purge deployment
        let removed =
            deployer::purge_deployment(&db, &game_id, &bottle_name, &data_dir, &game.game_path)
                .map_err(|e| e.to_string())?;
        let files_removed = removed.len();

        // 3. Disable all mods
        let mods_disabled = {
            let conn = db.conn().map_err(|e| e.to_string())?;
            conn.execute(
                "UPDATE installed_mods SET enabled = 0 WHERE game_id = ?1 AND bottle_name = ?2",
                rusqlite::params![game_id, bottle_name],
            )
            .map_err(|e| e.to_string())?
        };

        // 4. Optionally clean orphans
        let orphans_cleaned = if clean_orphans {
            let opts = cleaner::CleanOptions {
                remove_loose_files: true,
                remove_archives: true,
                remove_enb: false,
                remove_saves: false,
                remove_skse: false,
                orphans_only: true,
                dry_run: false,
                exclude_patterns: Vec::new(),
            };
            match cleaner::clean_game_directory(&db, &game_id, &bottle_name, &data_dir, &opts) {
                Ok(result) => result.removed_files.len(),
                Err(e) => {
                    log::warn!("Orphan cleanup failed: {}", e);
                    0
                }
            }
        } else {
            0
        };

        if game_id == "skyrimse" {
            let _ = sync_plugins_for_game(&game, &bottle);
        }

        Ok(serde_json::json!({
            "mods_disabled": mods_disabled,
            "files_removed": files_removed,
            "orphans_cleaned": orphans_cleaned,
        }))
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))?
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

    // Resolve auth token for collection API calls
    let token = nexus_api_key_or_token().await.ok().map(|(t, _)| t);

    // Get collection info to find latest revision number
    let info = collections::get_collection(token.as_deref(), &slug, &game_domain)
        .await
        .map_err(|e| e.to_string())?;

    let latest_revision = info.latest_revision;

    // Fetch mods from the latest revision
    let latest_result = collections::get_revision_mods(token.as_deref(), &slug, latest_revision)
        .await
        .map_err(|e| e.to_string())?;

    // Compute diff
    Ok(collections::compute_diff(
        &collection_name,
        meta.installed_revision,
        latest_revision,
        &manifest.mods,
        &latest_result.mods,
    ))
}

// --- Notes & Tags ---

#[tauri::command]
async fn set_mod_notes(
    mod_id: i64,
    notes: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        db.set_user_notes(mod_id, notes.as_deref())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn set_mod_source(
    mod_id: i64,
    source_type: String,
    source_url: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        db.set_mod_source(mod_id, &source_type, source_url.as_deref())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn set_mod_tags(
    mod_id: i64,
    tags: Vec<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || db.set_user_tags(mod_id, &tags).map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn get_all_tags(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        db.get_all_user_tags(&game_id, &bottle_name)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// --- Auto-category ---

#[tauri::command]
async fn backfill_categories(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<usize, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        // Also backfill source_type for legacy mods with nexus_mod_id but source_type="manual"
        let _ = db.backfill_source_types(&game_id, &bottle_name);
        db.backfill_categories(&game_id, &bottle_name)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// --- Notification Log ---

#[tauri::command]
async fn get_notification_log(
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<database::NotificationEntry>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        db.get_notifications(limit.unwrap_or(50))
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn clear_notification_log(state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || db.clear_notifications().map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn log_notification(
    level: String,
    message: String,
    detail: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        db.log_notification(&level, &message, detail.as_deref())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn get_notification_count(state: State<'_, AppState>) -> Result<usize, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || db.notification_count().map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
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
async fn retry_download(id: u64, state: State<'_, AppState>) -> Result<bool, String> {
    let queue = state.download_queue.clone();
    tokio::task::spawn_blocking(move || Ok(queue.mark_for_retry(id)))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
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
async fn update_loot_masterlist(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    loot::update_masterlist(&game_id, Some(&state.loot_masterlist_checked))
        .await
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn force_refresh_loot_masterlist(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    loot::force_refresh_masterlist(&game_id, Some(&state.loot_masterlist_checked))
        .await
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn reorder_plugins_cmd(
    game_id: String,
    bottle_name: String,
    ordered_plugins: Vec<String>,
) -> Result<Vec<PluginEntry>, String> {
    tokio::task::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn toggle_plugin_cmd(
    game_id: String,
    bottle_name: String,
    plugin_name: String,
    enabled: bool,
) -> Result<Vec<PluginEntry>, String> {
    tokio::task::spawn_blocking(move || {
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

        plugins::skyrim_plugins::toggle_plugin(
            &plugins_file,
            &loadorder_file,
            &plugin_name,
            enabled,
        )
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn move_plugin_cmd(
    game_id: String,
    bottle_name: String,
    plugin_name: String,
    new_index: usize,
) -> Result<Vec<PluginEntry>, String> {
    tokio::task::spawn_blocking(move || {
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

        plugins::skyrim_plugins::move_plugin(
            &plugins_file,
            &loadorder_file,
            &plugin_name,
            new_index,
        )
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn get_plugin_messages(
    game_id: String,
    bottle_name: String,
    plugin_name: String,
) -> Result<Vec<PluginWarning>, String> {
    tokio::task::spawn_blocking(move || {
        let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;

        let game_path = PathBuf::from(&game.game_path);
        let local_path = loot::local_game_path(&bottle, &game_id)
            .ok_or_else(|| format!("Cannot determine local path for game '{}'", game_id))?;

        loot::get_plugin_messages(&game_id, &game_path, &data_dir, &local_path, &plugin_name)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// --- Profiles ---

#[tauri::command]
async fn list_profiles_cmd(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<Profile>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        profiles::list_profiles(&db, &game_id, &bottle_name).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn create_profile_cmd(
    game_id: String,
    bottle_name: String,
    name: String,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        profiles::create_profile(&db, &game_id, &bottle_name, &name).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn delete_profile_cmd(profile_id: i64, state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        profiles::delete_profile(&db, profile_id).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn deactivate_profile_cmd(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        profiles::deactivate_profile(&db, &game_id, &bottle_name).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn rename_profile_cmd(
    profile_id: i64,
    new_name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        profiles::rename_profile(&db, profile_id, &new_name).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn save_profile_snapshot(
    profile_id: i64,
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        // Determine plugins file path (for Bethesda games with plugin load order)
        let plugins_file = if plugins::skyrim_plugins::supports_plugin_order(&game_id) {
            let (bottle, game, _) = resolve_game(&game_id, &bottle_name)?;

            games::with_plugin(&game_id, |plugin| {
                plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
            })
            .flatten()
        } else {
            None
        };

        profiles::snapshot_current_state(
            &db,
            profile_id,
            &game_id,
            &bottle_name,
            plugins_file.as_deref(),
        )
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn activate_profile(
    profile_id: i64,
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    check_game_lock(&state.game_locks, &game_id, &bottle_name)?;
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        // Look up the game
        let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;

        // Check if per-profile saves is enabled
        let saves_enabled = config::get_config_value("profile_saves_enabled")
            .ok()
            .flatten()
            .map(|v| v == "true")
            .unwrap_or(false);

        // Resolve saves directory for the game
        let saves_dir = if saves_enabled {
            games::with_plugin(&game_id, |plugin| {
                plugin.get_saves_dir(Path::new(&game.game_path), &bottle)
            })
            .flatten()
        } else {
            None
        };

        // 1. Save current state to the currently active profile (if any)
        if let Ok(Some(current_active)) = profiles::get_active_profile(&db, &game_id, &bottle_name)
        {
            let plugins_file = if plugins::skyrim_plugins::supports_plugin_order(&game_id) {
                games::with_plugin(&game_id, |plugin| {
                    plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
                })
                .flatten()
            } else {
                None
            };

            let _ = profiles::snapshot_current_state(
                &db,
                current_active.id,
                &game_id,
                &bottle_name,
                plugins_file.as_deref(),
            );

            // Backup current saves for the outgoing profile
            if let Some(ref sd) = saves_dir {
                let _ = profiles::backup_saves(current_active.id, &game_id, &bottle_name, sd);
            }
        }

        // 2. Purge current deployment
        let _ = deployer::purge_deployment(&db, &game_id, &bottle_name, &data_dir, &game.game_path);

        // 3. Load target profile state
        let mod_states = profiles::get_mod_states(&db, profile_id).map_err(|e| e.to_string())?;

        // 4. Apply mod enabled states and priorities
        for ms in &mod_states {
            let _ = db.set_enabled(ms.mod_id, ms.enabled);
            let _ = db.set_mod_priority(ms.mod_id, ms.priority);
        }

        // 5. Redeploy enabled mods
        let _ = deployer::redeploy_all(&db, &game_id, &bottle_name, &data_dir, &game.game_path);

        // 6. Restore saves for the incoming profile
        if let Some(ref sd) = saves_dir {
            let _ = profiles::restore_saves(profile_id, &game_id, &bottle_name, sd);
        }

        // 7. Apply plugin states
        let plugin_states =
            profiles::get_plugin_states(&db, profile_id).map_err(|e| e.to_string())?;

        if !plugin_states.is_empty() && plugins::skyrim_plugins::supports_plugin_order(&game_id) {
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

        // 8. Sync plugins to ensure plugins.txt matches on-disk state
        let _ = sync_plugins_for_game(&game, &bottle);

        // 9. Mark profile as active
        profiles::set_active_profile(&db, &game_id, &bottle_name, profile_id)
            .map_err(|e| e.to_string())?;

        Ok(())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn get_profile_save_info(
    profile_id: i64,
    game_id: String,
    bottle_name: String,
) -> Result<profiles::ProfileSaveInfo, String> {
    tokio::task::spawn_blocking(move || {
        Ok(profiles::get_profile_save_info(
            profile_id,
            &game_id,
            &bottle_name,
        ))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn backup_profile_saves(
    profile_id: i64,
    game_id: String,
    bottle_name: String,
) -> Result<usize, String> {
    tokio::task::spawn_blocking(move || {
        let (bottle, game, _) = resolve_game(&game_id, &bottle_name)?;
        let saves_dir = games::with_plugin(&game_id, |plugin| {
            plugin.get_saves_dir(Path::new(&game.game_path), &bottle)
        })
        .flatten()
        .ok_or("Game does not have a known saves directory")?;

        profiles::backup_saves(profile_id, &game_id, &bottle_name, &saves_dir)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn restore_profile_saves(
    profile_id: i64,
    game_id: String,
    bottle_name: String,
) -> Result<usize, String> {
    tokio::task::spawn_blocking(move || {
        let (bottle, game, _) = resolve_game(&game_id, &bottle_name)?;
        let saves_dir = games::with_plugin(&game_id, |plugin| {
            plugin.get_saves_dir(Path::new(&game.game_path), &bottle)
        })
        .flatten()
        .ok_or("Game does not have a known saves directory")?;

        profiles::restore_saves(profile_id, &game_id, &bottle_name, &saves_dir)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// --- Update Checking ---

#[tauri::command]
async fn check_mod_updates(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<ModUpdateInfo>, String> {
    let client = nexus_client().await?;

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

    client
        .check_updates(game_slug, &queries)
        .await
        .map_err(|e| e.to_string())
}

// --- Mod Tools ---

#[tauri::command]
async fn detect_mod_tools_cmd(
    game_id: String,
    bottle_name: String,
    _state: State<'_, AppState>,
) -> Result<Vec<mod_tools::ModTool>, String> {
    let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
    tokio::task::spawn_blocking(move || mod_tools::detect_tools_for_game(&data_dir, &game_id))
        .await
        .map_err(|e| format!("Tool detection task failed: {e}"))
}

#[tauri::command]
async fn install_mod_tool(
    app: AppHandle,
    tool_id: String,
    game_id: String,
    bottle_name: String,
) -> Result<String, String> {
    let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
    mod_tools::install_tool(&tool_id, &data_dir, &app)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn uninstall_mod_tool(
    tool_id: String,
    game_id: String,
    bottle_name: String,
    detected_path: Option<String>,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
        mod_tools::uninstall_tool(&tool_id, &data_dir, detected_path.as_deref())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn launch_mod_tool(
    tool_id: String,
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<LaunchResult, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let (bottle, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
        let tools = mod_tools::detect_tools_for_game(&data_dir, &game_id);
        let tool = tools
            .iter()
            .find(|t| t.id == tool_id)
            .ok_or_else(|| format!("Tool '{}' not found", tool_id))?;
        let exe_path = tool
            .detected_path
            .as_ref()
            .ok_or_else(|| format!("Tool '{}' is not installed", tool_id))?;
        mod_tools::launch_tool_with_logging(Path::new(exe_path), &bottle, &tool_id, &tool.name, &db)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn reinstall_mod_tool(
    app: AppHandle,
    tool_id: String,
    game_id: String,
    bottle_name: String,
) -> Result<String, String> {
    let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
    mod_tools::reinstall_tool(&tool_id, &data_dir, &app)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn check_mod_tool_update(
    tool_id: String,
    game_id: String,
    bottle_name: String,
) -> Result<mod_tools::ToolUpdateInfo, String> {
    let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
    mod_tools::check_tool_update(&tool_id, &data_dir)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn apply_tool_ini_edits_cmd(
    tool_id: String,
    game_id: String,
    bottle_name: String,
) -> Result<usize, String> {
    tokio::task::spawn_blocking(move || {
        let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
        mod_tools::apply_tool_ini_edits(&tool_id, &data_dir).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// --- Tool Requirement Detection ---

#[tauri::command]
async fn detect_collection_tools(
    manifest_json: String,
    game_id: String,
    bottle_name: String,
) -> Result<Vec<mod_tools::RequiredTool>, String> {
    tokio::task::spawn_blocking(move || {
        let manifest: collections::CollectionManifest = serde_json::from_str(&manifest_json)
            .map_err(|e| format!("Invalid manifest JSON: {}", e))?;
        let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
        Ok(mod_tools::detect_required_tools_collection(
            &manifest, &data_dir,
        ))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn detect_wabbajack_tools(
    wj_path: String,
    game_id: String,
    bottle_name: String,
) -> Result<Vec<mod_tools::RequiredTool>, String> {
    tokio::task::spawn_blocking(move || {
        let parsed = wabbajack::parse_wabbajack_file(std::path::Path::new(&wj_path))
            .map_err(|e| format!("Failed to parse .wabbajack: {}", e))?;
        let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
        Ok(mod_tools::detect_required_tools_wabbajack(
            &parsed, &data_dir,
        ))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// --- Platform Detection ---

#[derive(Clone, Debug, serde::Serialize)]
struct PlatformInfo {
    os: String,
    is_steam_os: bool,
    cpu_cores: usize,
    cpu_brand: String,
    memory_gb: u64,
    arch: String,
}

#[cfg(target_os = "macos")]
fn get_sysctl_string(name: &str) -> String {
    use std::ffi::CString;
    let cname = match CString::new(name) {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    let mut size: libc::size_t = 0;
    unsafe {
        if libc::sysctlbyname(
            cname.as_ptr(),
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        ) != 0
        {
            return String::new();
        }
        let mut buf = vec![0u8; size];
        if libc::sysctlbyname(
            cname.as_ptr(),
            buf.as_mut_ptr() as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        ) != 0
        {
            return String::new();
        }
        // Remove trailing null
        if let Some(pos) = buf.iter().position(|&b| b == 0) {
            buf.truncate(pos);
        }
        String::from_utf8_lossy(&buf).to_string()
    }
}

#[cfg(target_os = "macos")]
fn get_sysctl_u64(name: &str) -> u64 {
    use std::ffi::CString;
    let cname = match CString::new(name) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    let mut val: u64 = 0;
    let mut size = std::mem::size_of::<u64>() as libc::size_t;
    unsafe {
        if libc::sysctlbyname(
            cname.as_ptr(),
            &mut val as *mut u64 as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        ) != 0
        {
            return 0;
        }
    }
    val
}

#[tauri::command]
fn get_platform_detail() -> PlatformInfo {
    let os = std::env::consts::OS.to_string();
    let is_steam_os = if cfg!(target_os = "linux") {
        std::path::Path::new("/etc/steamos-release").exists() || std::env::var("SteamOS").is_ok()
    } else {
        false
    };

    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    let arch = std::env::consts::ARCH.to_string();

    #[cfg(target_os = "macos")]
    let (cpu_brand, memory_gb) = {
        let brand = get_sysctl_string("machdep.cpu.brand_string");
        let mem_bytes = get_sysctl_u64("hw.memsize");
        (brand, mem_bytes / (1024 * 1024 * 1024))
    };

    #[cfg(target_os = "linux")]
    let (cpu_brand, memory_gb) = {
        let brand = std::fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("model name"))
                    .and_then(|l| l.split(':').nth(1))
                    .map(|s| s.trim().to_string())
            })
            .unwrap_or_default();
        let mem = std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("MemTotal"))
                    .and_then(|l| l.split_whitespace().nth(1))
                    .and_then(|v| v.parse::<u64>().ok())
            })
            .unwrap_or(0)
            / (1024 * 1024); // kB → GB
        (brand, mem)
    };

    PlatformInfo {
        os,
        is_steam_os,
        cpu_cores,
        cpu_brand,
        memory_gb,
        arch,
    }
}

#[tauri::command]
fn get_optimal_download_threads() -> usize {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    let is_apple_silicon = cfg!(target_arch = "aarch64") && cfg!(target_os = "macos");
    let is_steam_os = std::path::Path::new("/etc/steamos-release").exists();

    if is_steam_os {
        cores.min(4)
    } else if is_apple_silicon {
        (cores / 2).clamp(4, 8)
    } else {
        (cores / 2).clamp(3, 6)
    }
}

// --- FOMOD ---

#[tauri::command]
async fn detect_fomod(
    staging_path: String,
    archive_hash: Option<String>,
    state: State<'_, AppState>,
) -> Result<Option<FomodInstaller>, String> {
    let fomod_cache = state.fomod_cache.clone();
    tokio::task::spawn_blocking(move || {
        let path = PathBuf::from(&staging_path);
        // Use archive SHA-256 hash as cache key if provided, otherwise fall back
        // to the staging path itself (still deterministic per-archive).
        let cache_key = archive_hash.unwrap_or_else(|| staging_path.clone());
        let mut installer = fomod::parse_fomod_cached(&fomod_cache, &cache_key, &path)
            .map_err(|e| e.to_string())?;
        // Resolve relative image paths to absolute so the frontend can serve them
        // via the Tauri asset: protocol.
        if let Some(ref mut inst) = installer {
            fomod::resolve_image_paths(inst, &path);
        }
        Ok(installer)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn get_fomod_defaults(
    installer: FomodInstaller,
) -> Result<std::collections::HashMap<String, Vec<String>>, String> {
    tokio::task::spawn_blocking(move || Ok(fomod::get_default_selections(&installer, None, None)))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn get_fomod_files(
    installer: FomodInstaller,
    selections: std::collections::HashMap<String, Vec<String>>,
) -> Result<Vec<fomod::FomodFile>, String> {
    tokio::task::spawn_blocking(move || {
        Ok(fomod::get_files_for_selections(
            &installer,
            &selections,
            None,
            None,
        ))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// --- DLC Detection ---

/// Known Skyrim SE framework mods and their NexusMods IDs, used by
/// `get_mod_requirements` to identify dependencies from mod descriptions.
const KNOWN_FRAMEWORKS: &[(&str, i64)] = &[
    ("SKSE64", 30379),
    ("SkyUI", 12604),
    ("Address Library for SKSE Plugins", 32444),
    ("powerofthree's Tweaks", 51073),
    ("PapyrusUtil SE", 13048),
    ("JContainers SE", 16495),
    ("ConsoleUtilSSE", 24858),
    ("FileAccess Interface for Skyrim SE", 13956),
    ("MCM Helper", 53000),
    ("Keyword Item Distributor", 55728),
    ("Spell Perk Item Distributor", 36869),
    ("Base Object Swapper", 60805),
    ("Sound Record Distributor", 77815),
    ("USSEP", 266),
    ("RaceMenu", 19080),
    ("Nemesis", 60033),
    ("FNIS", 3038),
    ("DAR - Dynamic Animation Replacer", 33746),
    ("OAR - Open Animation Replacer", 92109),
    ("CBBE", 198),
    ("XP32 Maximum Skeleton", 1988),
];

const SKYRIM_SE_DLC_FILES: &[(&str, &str)] = &[
    ("Dawnguard.esm", "Dawnguard"),
    ("HearthFires.esm", "Hearthfire"),
    ("Dragonborn.esm", "Dragonborn"),
];

/// Expected DLC files for Fallout 4.
const FALLOUT4_DLC_FILES: &[(&str, &str)] = &[
    ("DLCRobot.esm", "Automatron"),
    ("DLCworkshop01.esm", "Wasteland Workshop"),
    ("DLCworkshop02.esm", "Contraptions Workshop"),
    ("DLCworkshop03.esm", "Vault-Tec Workshop"),
    ("DLCCoast.esm", "Far Harbor"),
    ("DLCNukaWorld.esm", "Nuka-World"),
];

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DlcStatus {
    /// Whether all expected DLC files are present.
    all_present: bool,
    /// Per-DLC detection results.
    dlcs: Vec<DlcInfo>,
    /// Whether the game has been initialized (base game ESM exists).
    game_initialized: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DlcInfo {
    /// DLC name (e.g., "Dawnguard", "Dragonborn").
    name: String,
    /// Whether all files for this DLC are present.
    present: bool,
    /// Files that are missing.
    missing_files: Vec<String>,
}

#[tauri::command]
async fn check_dlc_status(game_id: String, bottle_name: String) -> Result<DlcStatus, String> {
    tokio::task::spawn_blocking(move || {
        let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;

        let dlc_files: &[(&str, &str)] = match game_id.as_str() {
            "skyrimse" => SKYRIM_SE_DLC_FILES,
            "fallout4" => FALLOUT4_DLC_FILES,
            _ => {
                return Ok(DlcStatus {
                    all_present: true,
                    dlcs: vec![],
                    game_initialized: true,
                })
            }
        };

        // Check if base game is initialized
        let base_esm = match game_id.as_str() {
            "skyrimse" => "Skyrim.esm",
            "fallout4" => "Fallout4.esm",
            _ => "",
        };
        let game_initialized = if base_esm.is_empty() {
            true
        } else {
            data_dir.join(base_esm).exists()
        };

        // Group by DLC name and check each file
        let mut dlc_map: std::collections::BTreeMap<String, Vec<(String, bool)>> =
            std::collections::BTreeMap::new();
        for (filename, dlc_name) in dlc_files {
            let present = data_dir.join(filename).exists();
            dlc_map
                .entry(dlc_name.to_string())
                .or_default()
                .push((filename.to_string(), present));
        }

        let mut dlcs = Vec::new();
        let mut all_present = true;
        for (name, files) in &dlc_map {
            let missing: Vec<String> = files
                .iter()
                .filter(|(_, p)| !p)
                .map(|(f, _)| f.clone())
                .collect();
            let present = missing.is_empty();
            if !present {
                all_present = false;
            }
            dlcs.push(DlcInfo {
                name: name.clone(),
                present,
                missing_files: missing,
            });
        }

        Ok(DlcStatus {
            all_present,
            dlcs,
            game_initialized,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// --- Integrity ---

#[tauri::command]
async fn create_game_snapshot(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<usize, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
        integrity::create_game_snapshot(&db, &game_id, &bottle_name, &data_dir)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn check_game_integrity(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<IntegrityReport, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
        integrity::check_game_integrity(&db, &game_id, &bottle_name, &data_dir)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn has_game_snapshot(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        integrity::has_snapshot(&db, &game_id, &bottle_name).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// --- Game Directory Cleaner ---

#[tauri::command]
async fn scan_game_directory(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<cleaner::CleanReport, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
        cleaner::scan_game_directory(&db, &game_id, &bottle_name, &data_dir)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn clean_game_directory(
    game_id: String,
    bottle_name: String,
    options: cleaner::CleanOptions,
    state: State<'_, AppState>,
) -> Result<cleaner::CleanResult, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;

        if !options.dry_run {
            auto_snapshot_before_destructive(
                &db,
                &game_id,
                &bottle_name,
                "Before clean game directory",
            );
        }

        let result =
            cleaner::clean_game_directory(&db, &game_id, &bottle_name, &data_dir, &options)
                .map_err(|e| e.to_string())?;

        // After a full clean (not orphans-only), reset plugins.txt to vanilla state
        // so the load order doesn't show stale entries for removed plugins
        if !options.dry_run && !options.orphans_only && !result.removed_files.is_empty() {
            if let Some(plugins_file) = games::with_plugin(&game_id, |plugin| {
                plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
            })
            .flatten()
            {
                // Build vanilla plugin list from stock ESMs still on disk
                let vanilla_entries: Vec<plugins::skyrim_plugins::PluginEntry> =
                    plugins::skyrim_plugins::get_implicit_plugins(&game_id)
                        .iter()
                        .filter(|name| data_dir.join(name).exists())
                        .map(|name| plugins::skyrim_plugins::PluginEntry {
                            filename: name.to_string(),
                            enabled: true,
                        })
                        .collect();
                let _ = plugins::skyrim_plugins::write_plugins_txt(&plugins_file, &vanilla_entries);
                log::info!(
                    "Reset plugins.txt to {} vanilla entries after clean",
                    vanilla_entries.len()
                );

                // Also reset loadorder.txt if it exists alongside plugins.txt
                if let Some(parent) = plugins_file.parent() {
                    let loadorder_file = parent.join("loadorder.txt");
                    if loadorder_file.exists() {
                        let _ = std::fs::remove_file(&loadorder_file);
                        log::info!("Removed stale loadorder.txt after clean");
                    }
                }
            }
        }

        Ok(result)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// --- Wabbajack Modlists ---

#[tauri::command]
async fn get_wabbajack_modlists() -> Result<Vec<ModlistSummary>, String> {
    wabbajack::fetch_modlist_gallery().await
}

#[tauri::command]
async fn parse_wabbajack_file(file_path: String) -> Result<ParsedModlist, String> {
    tokio::task::spawn_blocking(move || {
        wabbajack::parse_wabbajack_file(std::path::Path::new(&file_path))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
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

    std::fs::write(&dest, &bytes).map_err(|e| format!("Failed to save file: {e}"))?;

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

fn sync_plugins_for_game(game: &DetectedGame, bottle: &Bottle) -> Result<(), String> {
    // Only sync for games that support Bethesda-style plugin load order
    if !plugins::skyrim_plugins::supports_plugin_order(&game.game_id) {
        return Ok(());
    }

    let game_path = Path::new(&game.game_path);
    let data_dir = Path::new(&game.data_dir);

    let plugins_file = games::with_plugin(&game.game_id, |plugin| {
        plugin.get_plugins_file(game_path, bottle)
    })
    .flatten();

    if let Some(pf) = plugins_file {
        let loadorder_file = pf
            .parent()
            .map(|p| p.join("loadorder.txt"))
            .unwrap_or_else(|| pf.with_file_name("loadorder.txt"));
        let implicit = plugins::skyrim_plugins::implicit_plugins_for_game(&game.game_id);
        plugins::skyrim_plugins::sync_plugins(data_dir, &pf, &loadorder_file, implicit)
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Re-sync plugin load order, enabling all deployed plugins.
///
/// Call this after a collection install (or any time Plugins.txt looks wrong)
/// to ensure every plugin file in the Data directory is marked as enabled.
#[tauri::command]
async fn sync_plugins_cmd(
    game_id: String,
    bottle_name: String,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || {
        let (bottle, game, _data_dir) = resolve_game(&game_id, &bottle_name)?;
        sync_plugins_for_game(&game, &bottle)?;
        Ok(serde_json::json!({ "ok": true }))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
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

// --- OAuth ---

/// Start OAuth login flow using the hardcoded Corkscrew client ID.
/// Opens the user's default browser to NexusMods for authorization.
#[tauri::command]
async fn start_oauth_login() -> Result<TokenPair, String> {
    oauth::start_oauth_flow(oauth::CLIENT_ID)
        .await
        .map_err(|e| e.to_string())
}

/// Legacy command that accepts an explicit client_id (kept for compatibility).
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
async fn save_oauth_tokens(tokens: TokenPair) -> Result<(), String> {
    tokio::task::spawn_blocking(move || oauth::save_tokens(&tokens).map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn load_oauth_tokens() -> Result<Option<TokenPair>, String> {
    tokio::task::spawn_blocking(move || oauth::load_tokens().map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn clear_oauth_tokens() -> Result<(), String> {
    tokio::task::spawn_blocking(move || oauth::clear_tokens().map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn get_nexus_user_info(access_token: String) -> Result<NexusUserInfo, String> {
    tokio::task::spawn_blocking(move || {
        oauth::parse_user_info(&access_token).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn get_auth_method_cmd() -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn get_nexus_account_status() -> Result<serde_json::Value, String> {
    let method = oauth::get_auth_method_refreshed().await;
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

// --- Google OAuth (Gemini) ---

#[tauri::command]
async fn google_sign_in() -> Result<google_oauth::GoogleAuthStatus, String> {
    google_oauth::start_google_oauth_flow()
        .await
        .map(|tokens| google_oauth::GoogleAuthStatus {
            signed_in: true,
            email: tokens.email,
            name: tokens.name,
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn google_sign_out() -> Result<(), String> {
    google_oauth::sign_out().await.map_err(|e| e.to_string())
}

#[tauri::command]
fn google_auth_status() -> google_oauth::GoogleAuthStatus {
    google_oauth::get_google_auth_status()
}

// --- Crash Logs ---

#[tauri::command]
async fn find_crash_logs_cmd(
    game_id: String,
    bottle_name: String,
) -> Result<Vec<CrashLogEntry>, String> {
    tokio::task::spawn_blocking(move || {
        let (bottle, game, _) = resolve_game(&game_id, &bottle_name)?;

        let game_path = PathBuf::from(&game.game_path);
        Ok(crashlog::find_crash_logs(
            &game_path,
            &bottle.path,
            &game_id,
        ))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn analyze_crash_log_cmd(log_path: String) -> Result<CrashReport, String> {
    tokio::task::spawn_blocking(move || {
        crashlog::analyze_crash_log(Path::new(&log_path)).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Check for new (unseen) crash logs since the last check.
#[tauri::command]
async fn chat_check_new_crashes(
    game_id: String,
    bottle_name: String,
) -> Result<NewCrashInfo, String> {
    tokio::task::spawn_blocking(move || {
        let bottle = resolve_bottle(&bottle_name)?;
        Ok(crashlog::check_new_crashes(&PathBuf::from(&bottle.path), &game_id))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
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
    } else if url.contains("github.com")
        && !url.contains("/raw/")
        && !url.contains("raw.githubusercontent.com")
    {
        // Plain repo URL: github.com/user/repo → try raw README.md
        let trimmed = url.trim_end_matches('/');
        let raw_base = trimmed.replace("github.com", "raw.githubusercontent.com");
        // Try main branch first, fall back to master
        let main_url = format!("{}/main/README.md", raw_base);
        let resp = client
            .get(&main_url)
            .header("Accept", "text/plain, text/markdown, */*")
            .send()
            .await;
        if let Ok(r) = resp {
            if r.status().is_success() {
                return r
                    .text()
                    .await
                    .map_err(|e| format!("Failed to read response: {e}"));
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
    let client = nexus_client().await?;
    client
        .browse_mods(&game_slug, &category)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_nexus_mod_detail(
    game_slug: String,
    mod_id: i64,
) -> Result<nexus::NexusModInfo, String> {
    let client = nexus_client().await?;
    client
        .get_mod_info(&game_slug, mod_id)
        .await
        .map_err(|e| e.to_string())
}

// --- Endorsements ---

#[tauri::command]
async fn endorse_mod(
    game_slug: String,
    mod_id: i64,
    version: Option<String>,
) -> Result<nexus::EndorseResponse, String> {
    let client = nexus_client().await?;
    client
        .endorse_mod(&game_slug, mod_id, version.as_deref())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn abstain_mod(game_slug: String, mod_id: i64) -> Result<nexus::EndorseResponse, String> {
    let client = nexus_client().await?;
    client
        .abstain_mod(&game_slug, mod_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_user_endorsements() -> Result<Vec<nexus::UserEndorsement>, String> {
    let client = nexus_client().await?;
    client
        .get_user_endorsements()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
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
    let (token, is_bearer) = nexus_api_key_or_token().await?;
    nexus::graphql_search_mods_ext(
        &token,
        is_bearer,
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
async fn get_game_categories_cmd(game_slug: String) -> Result<Vec<NexusCategory>, String> {
    let client = nexus_client().await?;
    client
        .get_game_categories(&game_slug)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
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
    adult_content: Option<bool>,
) -> Result<CollectionSearchResult, String> {
    let token = nexus_api_key_or_token().await.ok().map(|(t, _)| t);

    let sf = sort_field.as_deref().unwrap_or("endorsements");
    let sd = sort_direction.as_deref().unwrap_or("desc");
    let st = search_text.as_deref().filter(|s| !s.is_empty());

    collections::browse_collections(
        token.as_deref(),
        &game_domain,
        count,
        offset,
        sf,
        sd,
        st,
        author.as_deref(),
        min_downloads,
        min_endorsements,
        adult_content,
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_collection_cmd(slug: String, game_domain: String) -> Result<CollectionInfo, String> {
    let token = nexus_api_key_or_token().await.ok().map(|(t, _)| t);

    collections::get_collection(token.as_deref(), &slug, &game_domain)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_collection_revisions(slug: String) -> Result<Vec<CollectionRevision>, String> {
    let token = nexus_api_key_or_token().await.ok().map(|(t, _)| t);

    collections::get_revisions(token.as_deref(), &slug)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_collection_mods(slug: String, revision: u32) -> Result<RevisionModsResult, String> {
    let token = nexus_api_key_or_token().await.ok().map(|(t, _)| t);

    collections::get_revision_mods(token.as_deref(), &slug, revision)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn parse_collection_bundle_cmd(bundle_path: String) -> Result<CollectionManifest, String> {
    tokio::task::spawn_blocking(move || {
        collections::parse_collection_bundle(Path::new(&bundle_path)).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
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
        None, // fresh install, no resume checkpoint
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

#[tauri::command]
async fn cancel_collection_install_cmd() -> Result<(), String> {
    collection_installer::cancel_install();
    Ok(())
}

#[tauri::command]
async fn submit_fomod_choices(
    correlation_id: String,
    selections: std::collections::HashMap<String, Vec<String>>,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        collection_installer::submit_fomod_choices(&correlation_id, selections)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// --- Collection Install Resume ---

#[tauri::command]
async fn get_incomplete_collection_installs(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<database::CollectionInstallCheckpoint>, String> {
    let checkpoint = state
        .db
        .get_active_checkpoint(&game_id, &bottle_name)
        .map_err(|e| format!("Failed to query checkpoints: {}", e))?;
    Ok(checkpoint.into_iter().collect())
}

#[tauri::command]
async fn get_all_interrupted_installs(
    state: State<'_, AppState>,
) -> Result<Vec<database::CollectionInstallCheckpoint>, String> {
    state
        .db
        .get_all_active_checkpoints()
        .map_err(|e| format!("Failed to query interrupted installs: {}", e))
}

#[tauri::command]
async fn get_checkpoint_mod_names(
    checkpoint_id: i64,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let checkpoint = state
        .db
        .get_active_checkpoint_by_id(checkpoint_id)
        .map_err(|e| format!("Failed to query checkpoint: {}", e))?
        .ok_or_else(|| "Checkpoint not found".to_string())?;

    let manifest: serde_json::Value = serde_json::from_str(&checkpoint.manifest_json)
        .map_err(|e| format!("Failed to parse manifest: {}", e))?;

    let names: Vec<String> = manifest
        .get("mods")
        .and_then(|m| m.as_array())
        .map(|mods| {
            mods.iter()
                .filter_map(|m| m.get("name").and_then(|n| n.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();

    Ok(names)
}

#[tauri::command]
async fn resume_collection_install_cmd(
    app: AppHandle,
    checkpoint_id: i64,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let result = collection_installer::resume_collection_install(
        &app,
        &state.db,
        &state.download_queue,
        checkpoint_id,
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

#[tauri::command]
async fn abandon_collection_install(
    checkpoint_id: i64,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .db
        .abandon_checkpoint(checkpoint_id)
        .map_err(|e| format!("Failed to abandon checkpoint: {}", e))
}

#[tauri::command]
async fn get_pending_wabbajack_installs(
    state: State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    let rows = state
        .db
        .list_pending_wj_installs()
        .map_err(|e| format!("Failed to query pending installs: {}", e))?;

    Ok(rows
        .into_iter()
        .map(
            |(id, name, version, status, total_a, completed_a, total_d, completed_d, error)| {
                serde_json::json!({
                    "install_id": id,
                    "modlist_name": name,
                    "modlist_version": version,
                    "status": status,
                    "total_archives": total_a,
                    "completed_archives": completed_a,
                    "total_directives": total_d,
                    "completed_directives": completed_d,
                    "error_message": error,
                })
            },
        )
        .collect())
}

// --- Game Version Pinning ---

#[tauri::command]
async fn get_pinned_game_version(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    state
        .db
        .get_pinned_game_version(&game_id, &bottle_name)
        .map_err(|e| format!("Failed to get pinned version: {}", e))
}

#[tauri::command]
async fn pin_game_version(
    game_id: String,
    bottle_name: String,
    version: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .db
        .set_pinned_game_version(&game_id, &bottle_name, &version)
        .map_err(|e| format!("Failed to pin version: {}", e))
}

// --- Plugin Load Order Rules ---

#[tauri::command]
async fn add_plugin_rule(
    game_id: String,
    bottle_name: String,
    plugin_name: String,
    rule_type: loot_rules::PluginRuleType,
    reference_plugin: String,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        loot_rules::add_rule(
            &db,
            &game_id,
            &bottle_name,
            &plugin_name,
            rule_type,
            &reference_plugin,
        )
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn remove_plugin_rule(rule_id: i64, state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || loot_rules::remove_rule(&db, rule_id))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn list_plugin_rules(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<PluginRule>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || loot_rules::list_rules(&db, &game_id, &bottle_name))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn clear_plugin_rules(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || loot_rules::clear_rules(&db, &game_id, &bottle_name))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

// --- Mod Rollback & Snapshots ---

#[tauri::command]
async fn save_mod_version_cmd(
    mod_id: i64,
    version: String,
    staging_path: String,
    archive_name: String,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        rollback::save_mod_version(&db, mod_id, &version, &staging_path, &archive_name)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn list_mod_versions_cmd(
    mod_id: i64,
    state: State<'_, AppState>,
) -> Result<Vec<ModVersion>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || rollback::list_mod_versions(&db, mod_id))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn rollback_mod_version(
    mod_id: i64,
    version_id: i64,
    state: State<'_, AppState>,
) -> Result<ModVersion, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || rollback::rollback_to_version(&db, mod_id, version_id))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn cleanup_mod_versions(
    mod_id: i64,
    keep_count: usize,
    state: State<'_, AppState>,
) -> Result<usize, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || rollback::cleanup_old_versions(&db, mod_id, keep_count))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn create_mod_snapshot(
    game_id: String,
    bottle_name: String,
    name: String,
    description: Option<String>,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        rollback::create_snapshot(&db, &game_id, &bottle_name, &name, description.as_deref())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn list_mod_snapshots(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<ModSnapshot>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || rollback::list_snapshots(&db, &game_id, &bottle_name))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn delete_mod_snapshot(snapshot_id: i64, state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || rollback::delete_snapshot(&db, snapshot_id))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

// --- Modlist Export/Import ---

#[tauri::command]
async fn export_modlist_cmd(
    game_id: String,
    bottle_name: String,
    output_path: String,
    notes: Option<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        // Get current plugin order if applicable
        let plugin_entries = get_current_plugins(&game_id, &bottle_name);

        let modlist = modlist_io::export_modlist(
            &db,
            &game_id,
            &bottle_name,
            &plugin_entries,
            notes.as_deref(),
        )
        .map_err(|e| e.to_string())?;

        let path = PathBuf::from(&output_path);
        modlist_io::write_modlist_file(&modlist, &path).map_err(|e| e.to_string())?;

        Ok(output_path)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn import_modlist_plan(
    file_path: String,
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<ImportPlan, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let modlist =
            modlist_io::read_modlist_file(Path::new(&file_path)).map_err(|e| e.to_string())?;
        modlist_io::validate_modlist(&modlist, &game_id).map_err(|e| e.to_string())?;

        modlist_io::plan_import(&db, &modlist, &game_id, &bottle_name).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn diff_modlists_cmd(
    file_path: String,
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<ModlistDiff, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let imported =
            modlist_io::read_modlist_file(Path::new(&file_path)).map_err(|e| e.to_string())?;

        let plugin_entries = get_current_plugins(&game_id, &bottle_name);

        let current =
            modlist_io::export_modlist(&db, &game_id, &bottle_name, &plugin_entries, None)
                .map_err(|e| e.to_string())?;

        Ok(modlist_io::diff_modlists(&current, &imported))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn execute_modlist_import(
    file_path: String,
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<modlist_io::ImportResult, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let imported =
            modlist_io::read_modlist_file(Path::new(&file_path)).map_err(|e| e.to_string())?;
        modlist_io::execute_import(&db, &imported, &game_id, &bottle_name)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// --- Disk Budget Commands ---

#[tauri::command]
async fn get_disk_budget(
    game_id: String,
    bottle_name: String,
) -> Result<disk_budget::DiskBudget, String> {
    tokio::task::spawn_blocking(move || {
        let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
        Ok(disk_budget::compute_budget(
            &game_id,
            &bottle_name,
            &data_dir,
        ))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn estimate_install_impact_cmd(
    archive_size: u64,
    game_id: String,
    bottle_name: String,
) -> Result<disk_budget::InstallImpact, String> {
    tokio::task::spawn_blocking(move || {
        let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
        Ok(disk_budget::estimate_install_impact(
            archive_size,
            &data_dir,
        ))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn get_available_disk_space_cmd(path: String) -> Result<u64, String> {
    tokio::task::spawn_blocking(move || {
        Ok(disk_budget::available_space(std::path::Path::new(&path)))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// --- Staging Info Commands ---

#[tauri::command]
async fn get_staging_info(
    game_id: String,
    bottle_name: String,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn set_staging_directory(path: Option<String>) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        match path {
            Some(ref p) if !p.is_empty() => {
                // Validate path exists or can be created
                let path_buf = std::path::PathBuf::from(p);
                if !path_buf.exists() {
                    std::fs::create_dir_all(&path_buf)
                        .map_err(|e| format!("Cannot create staging directory '{}': {}", p, e))?;
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
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// --- INI Manager Commands ---

#[tauri::command]
async fn get_ini_settings(
    game_id: String,
    bottle_name: String,
) -> Result<Vec<ini_manager::IniFile>, String> {
    tokio::task::spawn_blocking(move || {
        let bottle = resolve_bottle(&bottle_name)?;
        Ok(ini_manager::read_all_ini(&bottle, &game_id))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn set_ini_setting(
    file_path: String,
    section: String,
    key: String,
    value: String,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        ini_manager::set_setting(Path::new(&file_path), &section, &key, &value)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn get_ini_presets(game_id: String) -> Result<Vec<ini_manager::IniPreset>, String> {
    tokio::task::spawn_blocking(move || Ok(ini_manager::builtin_presets(&game_id)))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn apply_ini_preset(
    game_id: String,
    bottle_name: String,
    preset_name: String,
) -> Result<usize, String> {
    tokio::task::spawn_blocking(move || {
        let bottle = resolve_bottle(&bottle_name)?;
        let presets = ini_manager::builtin_presets(&game_id);
        let preset = presets
            .iter()
            .find(|p| p.name == preset_name)
            .ok_or_else(|| format!("Preset '{}' not found", preset_name))?;
        ini_manager::apply_preset(&bottle, &game_id, preset).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Read a text file from a mod's staging directory.
/// `staging_path` is the mod's staging root, `relative_path` is the file within it.
#[tauri::command]
async fn read_mod_file(staging_path: String, relative_path: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let full = Path::new(&staging_path).join(&relative_path);
        if !full.exists() {
            return Err(format!("File not found: {}", full.display()));
        }
        // Prevent directory traversal
        let canon = full.canonicalize().map_err(|e| e.to_string())?;
        let base = Path::new(&staging_path)
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if !canon.starts_with(&base) {
            return Err("Path traversal denied".into());
        }
        std::fs::read_to_string(&canon).map_err(|e| format!("Failed to read file: {}", e))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Write a text file in a mod's staging directory.
#[tauri::command]
async fn write_mod_file(
    staging_path: String,
    relative_path: String,
    content: String,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let full = Path::new(&staging_path).join(&relative_path);
        // Prevent directory traversal
        let base = Path::new(&staging_path)
            .canonicalize()
            .map_err(|e| e.to_string())?;
        // For writes, parent must exist and resolved path must be under base
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let canon = full.canonicalize().unwrap_or_else(|_| full.clone());
        if !canon.starts_with(&base) {
            return Err("Path traversal denied".into());
        }
        std::fs::write(&full, content).map_err(|e| format!("Failed to write file: {}", e))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// --- Instruction Parsing (Collection Author Instructions) ---

/// Parse collection instructions using the deterministic (Tier 1) parser.
#[tauri::command]
async fn parse_instructions_cmd(
    instructions: String,
    mod_names: Vec<String>,
) -> Result<instruction_types::ParsedInstructions, String> {
    tokio::task::spawn_blocking(move || {
        Ok(instruction_parser::parse_instructions(
            &instructions,
            &mod_names,
        ))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Parse instructions using a local Ollama model (Tier 2a).
#[tauri::command]
async fn parse_instructions_llm_cmd(
    instructions: String,
    mod_names: Vec<String>,
    model: String,
    platform: String,
    game_version: String,
) -> Result<Vec<instruction_types::ConditionalAction>, String> {
    llm_parser::parse_with_ollama(&model, &instructions, &mod_names, &platform, &game_version).await
}

/// Parse instructions using a cloud LLM (Tier 2a — cloud).
#[tauri::command]
async fn parse_instructions_cloud_cmd(
    instructions: String,
    mod_names: Vec<String>,
    provider: String,
    api_key: String,
    platform: String,
    game_version: String,
) -> Result<Vec<instruction_types::ConditionalAction>, String> {
    match provider.as_str() {
        "groq" => {
            llm_parser::parse_with_groq(
                &api_key,
                &instructions,
                &mod_names,
                &platform,
                &game_version,
            )
            .await
        }
        "cerebras" => {
            llm_parser::parse_with_cerebras(
                &api_key,
                &instructions,
                &mod_names,
                &platform,
                &game_version,
            )
            .await
        }
        "gemini" => {
            llm_parser::parse_with_gemini(
                &api_key,
                &instructions,
                &mod_names,
                &platform,
                &game_version,
            )
            .await
        }
        _ => Err(format!("Unknown cloud provider: {provider}")),
    }
}

/// Validate parsed actions against the actual installed mod list.
#[tauri::command]
async fn validate_instruction_actions_cmd(
    actions: Vec<instruction_types::ConditionalAction>,
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<instruction_types::ValidatedAction>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        Ok(instruction_validator::validate_actions(
            &actions,
            &db,
            &game_id,
            &bottle_name,
        ))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Check Ollama status (installed, running, available models).
#[tauri::command]
async fn check_ollama_status_cmd() -> Result<instruction_types::OllamaStatus, String> {
    Ok(llm_parser::check_ollama_status().await)
}

/// Get the list of recommended local models.
#[tauri::command]
fn get_recommended_models() -> Vec<instruction_types::OllamaModel> {
    instruction_types::recommended_models()
}

/// Get available cloud LLM providers.
#[tauri::command]
fn get_cloud_providers() -> Vec<llm_parser::CloudProvider> {
    llm_parser::available_cloud_providers()
}

/// Download (pull) a model via Ollama.
#[tauri::command]
async fn pull_ollama_model_cmd(model_name: String) -> Result<(), String> {
    llm_parser::pull_ollama_model(&model_name).await
}

/// Delete a model from Ollama (removes from disk).
#[tauri::command]
async fn delete_ollama_model_cmd(model_name: String) -> Result<(), String> {
    llm_parser::delete_ollama_model(&model_name).await
}

/// Unload a model from Ollama's memory (keeps on disk).
#[tauri::command]
async fn unload_ollama_model_cmd(model_name: String) -> Result<(), String> {
    llm_parser::unload_ollama_model(&model_name).await
}

// --- System Info ---

/// Get total system memory in bytes (unified memory on Apple Silicon).
#[tauri::command]
fn get_system_memory() -> Result<u64, String> {
    #[cfg(target_os = "macos")]
    {
        unsafe {
            let mut size: u64 = 0;
            let mut len = std::mem::size_of::<u64>();
            let mib = [libc::CTL_HW, libc::HW_MEMSIZE];
            let ret = libc::sysctl(
                mib.as_ptr() as *mut _,
                2,
                &mut size as *mut u64 as *mut _,
                &mut len,
                std::ptr::null_mut(),
                0,
            );
            if ret == 0 {
                Ok(size)
            } else {
                Err("Failed to query system memory".into())
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        use std::fs;
        let meminfo = fs::read_to_string("/proc/meminfo")
            .map_err(|e| format!("Failed to read /proc/meminfo: {e}"))?;
        for line in meminfo.lines() {
            if line.starts_with("MemTotal:") {
                let kb: u64 = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|s| s.parse().ok())
                    .ok_or("Failed to parse MemTotal")?;
                return Ok(kb * 1024);
            }
        }
        Err("MemTotal not found in /proc/meminfo".into())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err("System memory detection not supported on this platform".into())
    }
}

/// Install Ollama. On Linux, runs the official install script.
/// On macOS, opens the download page (Ollama ships as a .app bundle).
#[tauri::command]
async fn install_ollama() -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg("curl -fsSL https://ollama.com/install.sh | sh")
            .output()
            .await
            .map_err(|e| format!("Failed to run install script: {e}"))?;

        if output.status.success() {
            Ok("Ollama installed successfully. It should now be running.".into())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("Install failed: {stderr}"))
        }
    }
    #[cfg(target_os = "macos")]
    {
        // macOS: Ollama is a native .app — open the download page
        let _ = std::process::Command::new("open")
            .arg("https://ollama.com/download/mac")
            .spawn();
        Ok("Opening Ollama download page. Install the app, then return here.".into())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err("Automatic Ollama install not supported on this platform".into())
    }
}

/// Start Ollama headlessly (serve mode) if not already running.
#[tauri::command]
async fn start_ollama() -> Result<String, String> {
    // Check if already running
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .map_err(|e| e.to_string())?;

    if let Ok(resp) = client.get("http://localhost:11434/api/tags").send().await {
        if resp.status().is_success() {
            return Ok("Ollama is already running.".into());
        }
    }

    // Try to start ollama serve in background
    #[cfg(target_os = "macos")]
    {
        // Try CLI first (if installed via homebrew or ollama cli is in PATH)
        let result = tokio::process::Command::new("ollama")
            .arg("serve")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();

        if result.is_err() {
            // Fallback: try launching the .app which also starts the server
            let _ = std::process::Command::new("open")
                .arg("-a")
                .arg("Ollama")
                .arg("--background")
                .spawn();
        }
    }

    #[cfg(target_os = "linux")]
    {
        let _ = tokio::process::Command::new("ollama")
            .arg("serve")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }

    // Wait briefly for it to start
    for _ in 0..10 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        if let Ok(resp) = client.get("http://localhost:11434/api/tags").send().await {
            if resp.status().is_success() {
                return Ok("Ollama started successfully.".into());
            }
        }
    }

    Err("Could not start Ollama. Please install and launch it manually.".into())
}

/// Check if MLX LM (Apple's MLX inference library) is installed.
#[tauri::command]
async fn check_mlx_status() -> Result<bool, String> {
    Ok(llm_chat::check_mlx_status().await)
}

/// Install MLX LM into a dedicated venv (~/.corkscrew/mlx-venv/).
#[tauri::command]
async fn install_mlx() -> Result<String, String> {
    llm_chat::install_mlx().await
}

/// Get the recommended model name based on system memory.
#[tauri::command]
fn get_recommended_model() -> Result<String, String> {
    let mem = get_system_memory()?;
    Ok(instruction_types::recommended_model_for_memory(mem))
}

// --- LLM Chat Commands ---

/// Get the current chat session state.
#[tauri::command]
async fn chat_get_state(state: State<'_, AppState>) -> Result<llm_chat::ChatState, String> {
    let session = state.chat_session.lock().await;
    let ollama_status = llm_parser::check_ollama_status().await;
    let cloud_provider = match &session.backend {
        llm_chat::LlmBackend::Cloud { ref provider, .. } => Some(provider.clone()),
        llm_chat::LlmBackend::GeminiOAuth => Some("gemini_oauth".to_string()),
        _ => None,
    };
    let google_auth = Some(google_oauth::get_google_auth_status());
    Ok(llm_chat::ChatState {
        model: session.model.clone(),
        backend: session.backend.clone(),
        loaded: session.model.is_some(),
        messages: session.messages.clone(),
        available_models: ollama_status
            .available_models
            .into_iter()
            .map(|m| instruction_types::OllamaModel {
                name: m.name,
                size_bytes: m.size_bytes,
                size_display: m.size_display,
                description: m.description,
                expected_accuracy: m.expected_accuracy,
                supports_tool_use: m.supports_tool_use,
                min_memory_bytes: 0,
            })
            .collect(),
        cloud_provider,
        google_auth,
    })
}

/// Resolve game display name from game_id.
fn game_display_name(game_id: &str) -> &str {
    match game_id {
        "skyrimse" => "Skyrim Special Edition",
        "skyrim" => "Skyrim",
        "fallout4" => "Fallout 4",
        "fallout3" => "Fallout 3",
        "falloutnv" => "Fallout: New Vegas",
        "oblivion" => "The Elder Scrolls IV: Oblivion",
        "morrowind" => "Morrowind",
        "starfield" => "Starfield",
        other => other,
    }
}

/// Load a model for chat and initialize the session.
#[tauri::command]
async fn chat_load_model(
    model_name: String,
    game_id: String,
    bottle_name: String,
    current_page: Option<String>,
    backend: Option<String>,
    cloud_provider: Option<String>,
    cloud_api_key: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let backend_enum = match backend.as_deref() {
        Some("cloud") => {
            let provider = cloud_provider.ok_or("cloud_provider is required for cloud backend")?;
            let api_key = cloud_api_key.ok_or("cloud_api_key is required for cloud backend")?;
            llm_chat::LlmBackend::Cloud { provider, api_key }
        }
        Some("gemini_oauth") => llm_chat::LlmBackend::GeminiOAuth,
        Some("mlx") => llm_chat::LlmBackend::Mlx,
        _ => llm_chat::LlmBackend::Ollama,
    };

    // Resolve model name for the backend
    let resolved_model = match &backend_enum {
        llm_chat::LlmBackend::Mlx => llm_chat::mlx_model_name(&model_name),
        llm_chat::LlmBackend::Ollama => model_name.clone(),
        llm_chat::LlmBackend::Cloud { ref provider, .. } => llm_chat::cloud_model_display(provider),
        llm_chat::LlmBackend::GeminiOAuth => llm_chat::cloud_model_display("gemini_oauth"),
    };

    // Start the MLX server if needed
    if backend_enum == llm_chat::LlmBackend::Mlx {
        llm_chat::start_mlx_server(&resolved_model).await?;
    }

    // Compute context config based on system memory
    let mem_bytes = get_system_memory().unwrap_or(16_000_000_000);
    let num_ctx = instruction_types::context_size_for_memory(mem_bytes);

    // Load the model (no-op for cloud)
    llm_chat::load_model(&backend_enum, &resolved_model, num_ctx).await?;

    let db = state.db.clone();
    let gid = game_id.clone();
    let bn = bottle_name.clone();
    let (mod_count, wine_warnings_text) = tokio::task::spawn_blocking(move || {
        let mods = db.list_mods_summary(&gid, &bn).unwrap_or_default();
        let mod_count = mods.len();

        // Run Wine compat check on all installed mods
        let compat_input = wine_compat::build_compat_input(&mods);
        let warnings = wine_compat::check_all_mods_wine_compat(&compat_input);
        let warnings_text = if warnings.is_empty() {
            None
        } else {
            Some(wine_compat::format_warnings_report(&warnings))
        };
        (mod_count, warnings_text)
    })
    .await
    .unwrap_or((0, None));

    let game_name = game_display_name(&game_id);
    let page = current_page.as_deref().unwrap_or("Mods");

    let mut session = state.chat_session.lock().await;
    session.model = Some(resolved_model.clone());
    session.backend = backend_enum;
    session.messages.clear();
    session.touch();

    // Add system message
    let system = llm_chat::build_chat_system_prompt(
        game_name,
        mod_count,
        "Wine/CrossOver",
        page,
        None,
        wine_warnings_text.as_deref(),
        &session.backend,
    );
    session.messages.push(llm_chat::ChatMessage {
        role: "system".into(),
        content: system,
        tool_calls: None,
        mentioned_mods: None,
        timestamp: None,
    });

    // Restore recent chat history from DB
    {
        let db = state.db.clone();
        let gid = game_id.clone();
        let bn = bottle_name.clone();
        let history = tokio::task::spawn_blocking(move || {
            db.load_chat_history(&gid, &bn, 50).unwrap_or_default()
        })
        .await
        .unwrap_or_default();
        if !history.is_empty() {
            log::info!("[CHAT] Restored {} messages from history", history.len());
            session.messages.extend(history);
        }
    }

    Ok(())
}

/// Unload the chat model and clear the session.
#[tauri::command]
async fn chat_unload_model(state: State<'_, AppState>) -> Result<(), String> {
    let mut session = state.chat_session.lock().await;
    if let Some(ref model) = session.model {
        let _ = llm_chat::unload_model(&session.backend, model).await;
    }
    session.model = None;
    session.messages.clear();
    Ok(())
}

/// Check which MLX models are already cached locally in ~/.cache/huggingface/hub/.
#[tauri::command]
async fn get_cached_mlx_models() -> Vec<String> {
    let mut cached = Vec::new();
    if let Some(home) = dirs::home_dir() {
        let hub_dir = home.join(".cache/huggingface/hub");
        if let Ok(entries) = std::fs::read_dir(&hub_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("models--") && entry.path().is_dir() {
                    // Convert "models--org--name" back to "org/name"
                    let model_id = name
                        .strip_prefix("models--")
                        .unwrap_or(&name)
                        .replace("--", "/");
                    cached.push(model_id);
                }
            }
        }
    }
    cached
}

/// Delete a model from disk (Ollama: API delete, MLX: remove HuggingFace cache).
#[tauri::command]
async fn delete_model(model_name: String, backend: Option<String>) -> Result<String, String> {
    match backend.as_deref() {
        Some("mlx") => {
            // MLX models cached in ~/.cache/huggingface/hub/models--<org>--<name>
            let sanitized = model_name.replace("/", "--");
            if let Some(home) = dirs::home_dir() {
                let cache_dir = home
                    .join(".cache/huggingface/hub")
                    .join(format!("models--{sanitized}"));
                if cache_dir.exists() {
                    tokio::fs::remove_dir_all(&cache_dir)
                        .await
                        .map_err(|e| format!("Failed to delete: {e}"))?;
                    Ok(format!("Deleted {model_name} from MLX cache."))
                } else {
                    Err(format!("Cache directory not found for {model_name}"))
                }
            } else {
                Err("Cannot determine home directory.".into())
            }
        }
        _ => {
            // Ollama: DELETE /api/delete
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|e| e.to_string())?;
            let resp = client
                .delete("http://localhost:11434/api/delete")
                .json(&serde_json::json!({ "name": model_name }))
                .send()
                .await
                .map_err(|e| format!("Failed to delete model: {e}"))?;
            if resp.status().is_success() {
                Ok(format!("Deleted {model_name} from Ollama."))
            } else {
                let text = resp.text().await.unwrap_or_default();
                Err(format!("Ollama delete failed: {text}"))
            }
        }
    }
}

/// Send a user message and get the assistant response.
/// Handles tool calls automatically (one round).
#[tauri::command]
async fn chat_send_message(
    message: String,
    game_id: String,
    bottle_name: String,
    current_page: Option<String>,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<llm_chat::ChatResponse, String> {
    let (model, backend, tier, messages) = {
        let mut session = state.chat_session.lock().await;
        session.touch();
        let model = session.model.clone().ok_or("No model loaded")?;
        let backend = session.backend.clone();
        let tier = instruction_types::ModelCapabilityTier::from_model_name(&model);

        // Update system prompt with current page context if it changed
        if let Some(ref page) = current_page {
            let backend_ref = session.backend.clone();
            if let Some(system_msg) = session.messages.first_mut() {
                if system_msg.role == "system"
                    && !system_msg.content.contains(&format!("Page: {page}"))
                {
                    // Rebuild system prompt with updated page + wine compat
                    let db = state.db.clone();
                    let gid = game_id.clone();
                    let bn = bottle_name.clone();
                    let mods_list = db.list_mods_summary(&gid, &bn).unwrap_or_default();
                    let mod_count = mods_list.len();
                    let compat_input = wine_compat::build_compat_input(&mods_list);
                    let warnings = wine_compat::check_all_mods_wine_compat(&compat_input);
                    let warnings_text = if warnings.is_empty() {
                        None
                    } else {
                        Some(wine_compat::format_warnings_report(&warnings))
                    };
                    system_msg.content = llm_chat::build_chat_system_prompt(
                        game_display_name(&game_id),
                        mod_count,
                        "Wine/CrossOver",
                        page,
                        None,
                        warnings_text.as_deref(),
                        &backend_ref,
                    );
                }
            }
        }

        // Add user message
        let user_msg = llm_chat::ChatMessage {
            role: "user".into(),
            content: message,
            tool_calls: None,
            mentioned_mods: None,
            timestamp: None,
        };
        session.messages.push(user_msg.clone());

        // Persist user message to DB
        let db = state.db.clone();
        let gid = game_id.clone();
        let bn = bottle_name.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if let Err(e) = db.save_chat_message(&gid, &bn, &user_msg) {
                log::warn!("[CHAT] Failed to save user message: {e}");
            }
        });

        (model, backend, tier, session.messages.clone())
    };

    let tools = llm_chat::get_chat_tools(tier);

    // Compute context config based on system memory
    let mem_bytes = get_system_memory().unwrap_or(16_000_000_000);
    let num_ctx = instruction_types::context_size_for_memory(mem_bytes);
    let max_tokens = instruction_types::max_response_tokens(num_ctx);

    // Get LLM response with streaming
    log::info!("[CHAT] Sending to {:?} model={}", backend, model);
    let handle = app_handle.clone();
    let response = llm_chat::chat_send_streaming(
        &backend,
        &model,
        &messages,
        &tools,
        num_ctx,
        max_tokens,
        move |token, phase| {
            let _ = handle.emit(
                "chat-stream-token",
                serde_json::json!({
                    "text": token,
                    "phase": phase,
                }),
            );
        },
    )
    .await?;

    log::info!(
        "[CHAT] Response: content_len={} tool_calls={:?}",
        response.content.len(),
        response.tool_calls.as_ref().map(|tc| tc.len())
    );

    let mut all_tool_results = Vec::new();
    let mut current_response = response;
    let max_tool_rounds = 5;

    for round in 0..max_tool_rounds {
        // Check if this response has tool calls
        let has_tool_calls = current_response
            .tool_calls
            .as_ref()
            .map(|tc| !tc.is_empty())
            .unwrap_or(false);
        if !has_tool_calls {
            break;
        }

        let mut round_results = Vec::new();
        for tc in current_response.tool_calls.as_ref().unwrap() {
            log::info!(
                "[CHAT] [round {}] Executing tool: {} args={}",
                round,
                tc.function.name,
                tc.function.arguments
            );
            let display = tool_display_name(&tc.function.name, &tc.function.arguments);
            let _ = app_handle.emit(
                "chat-tool-status",
                serde_json::json!({
                    "tool_name": tc.function.name,
                    "status": "running",
                    "display_text": display,
                }),
            );

            // UI control tools are handled here (need app_handle for event emission)
            let result = if tc.function.name == "navigate_ui" {
                let page = tc
                    .function
                    .arguments
                    .get("page")
                    .and_then(|p| p.as_str())
                    .unwrap_or("mods");
                let _ = app_handle.emit("chat-navigate", serde_json::json!({ "page": page }));
                llm_chat::ToolResult {
                    tool_name: "navigate_ui".into(),
                    result: format!("Navigated to {} page.", page),
                    success: true,
                    display_name: format!("Navigating to {}", page),
                    structured_data: None,
                }
            } else if tc.function.name == "open_nexus_mod" {
                let mod_id = tc
                    .function
                    .arguments
                    .get("mod_id")
                    .and_then(|i| i.as_i64())
                    .unwrap_or(0);
                let mod_name = tc
                    .function
                    .arguments
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("mod");
                let _ = app_handle.emit(
                    "chat-open-nexus-mod",
                    serde_json::json!({
                        "mod_id": mod_id,
                        "name": mod_name,
                    }),
                );
                llm_chat::ToolResult {
                    tool_name: "open_nexus_mod".into(),
                    result: format!("Opened {} (ID: {}) in Corkscrew's Discover tab with images and install button.", mod_name, mod_id),
                    success: true,
                    display_name: format!("Opening {} in Discover", mod_name),
                    structured_data: None,
                }
            } else {
                execute_tool(
                    &tc.function.name,
                    &tc.function.arguments,
                    &game_id,
                    &bottle_name,
                    &state,
                )
                .await
            };
            log::info!(
                "[CHAT] [round {}] Tool result: success={} len={}",
                round,
                result.success,
                result.result.len()
            );

            let _ = app_handle.emit(
                "chat-tool-status",
                serde_json::json!({
                    "tool_name": tc.function.name,
                    "status": "complete",
                    "display_text": display,
                }),
            );

            round_results.push(result);
        }

        // Store assistant response + tool results in session
        {
            let mut session = state.chat_session.lock().await;
            session.messages.push(current_response.clone());
            for tr in &round_results {
                session.messages.push(llm_chat::ChatMessage {
                    role: "tool".into(),
                    content: tr.result.clone(),
                    tool_calls: None,
                    mentioned_mods: None,
                    timestamp: None,
                });
            }
        }
        all_tool_results.extend(round_results);

        // Get follow-up response WITH tools so model can chain calls
        log::info!("[CHAT] Making follow-up call (round {})", round);
        let messages = {
            let session = state.chat_session.lock().await;
            log::info!(
                "[CHAT] Session has {} messages for follow-up",
                session.messages.len()
            );
            session.messages.clone()
        };
        let handle2 = app_handle.clone();
        let followup = llm_chat::chat_send_streaming(
            &backend,
            &model,
            &messages,
            &tools,
            num_ctx,
            max_tokens,
            move |token, phase| {
                let _ = handle2.emit(
                    "chat-stream-token",
                    serde_json::json!({
                        "text": token,
                        "phase": phase,
                    }),
                );
            },
        )
        .await?;
        log::info!(
            "[CHAT] Follow-up response (round {}): content_len={} tool_calls={:?}",
            round,
            followup.content.len(),
            followup.tool_calls.as_ref().map(|tc| tc.len())
        );
        current_response = followup;
    }

    // If we exhausted tool rounds and still have no content, force a text-only response
    if !all_tool_results.is_empty() && current_response.content.trim().is_empty() {
        log::info!(
            "[CHAT] Forcing text-only follow-up (no content after {} tool rounds)",
            max_tool_rounds
        );
        // Store the last tool-call response + its results
        {
            let mut session = state.chat_session.lock().await;
            session.messages.push(current_response.clone());
            // If there are pending tool calls, add a synthetic tool result
            if let Some(ref tcs) = current_response.tool_calls {
                for _tc in tcs {
                    session.messages.push(llm_chat::ChatMessage {
                        role: "tool".into(),
                        content: "Tool call limit reached. Please summarize what you found so far."
                            .into(),
                        tool_calls: None,
                        mentioned_mods: None,
                        timestamp: None,
                    });
                }
            }
        }
        let messages = {
            let session = state.chat_session.lock().await;
            session.messages.clone()
        };
        let handle3 = app_handle.clone();
        let forced = llm_chat::chat_send_streaming(
            &backend,
            &model,
            &messages,
            &[],
            num_ctx,
            max_tokens,
            move |token, phase| {
                let _ = handle3.emit(
                    "chat-stream-token",
                    serde_json::json!({
                        "text": token,
                        "phase": phase,
                    }),
                );
            },
        )
        .await?;
        log::info!(
            "[CHAT] Forced text response: content_len={}",
            forced.content.len()
        );
        current_response = forced;
    }

    let tool_results = all_tool_results;

    // Scan for mentioned mods and attach to the final response
    let mut final_msg = current_response;
    let mentioned = scan_mentioned_mods(
        &final_msg.content,
        &tool_results,
        &game_id,
        &bottle_name,
        &state,
    )
    .await;
    if !mentioned.is_empty() {
        final_msg.mentioned_mods = Some(mentioned);
    }

    // Store final response (with mentioned_mods) in session
    {
        let mut session = state.chat_session.lock().await;
        session.messages.push(final_msg.clone());
    }

    // Persist assistant message to DB (only if it has content)
    if !final_msg.content.trim().is_empty() {
        let db = state.db.clone();
        let gid = game_id.clone();
        let bn = bottle_name.clone();
        let msg_to_save = final_msg.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if let Err(e) = db.save_chat_message(&gid, &bn, &msg_to_save) {
                log::warn!("[CHAT] Failed to save assistant message: {e}");
            }
        });
    }

    Ok(llm_chat::ChatResponse {
        message: final_msg,
        tool_results,
        needs_confirmation: false,
        pending_tool_calls: None,
    })
}

/// Clear chat history but keep model loaded.
#[tauri::command]
async fn chat_clear_history(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut session = state.chat_session.lock().await;
    // Keep system message, clear the rest
    if let Some(system) = session.messages.first().cloned() {
        session.messages.clear();
        if system.role == "system" {
            session.messages.push(system);
        }
    }

    // Also clear persisted history
    let db = state.db.clone();
    let _ = tokio::task::spawn_blocking(move || {
        if let Err(e) = db.clear_chat_history(&game_id, &bottle_name) {
            log::warn!("[CHAT] Failed to clear DB history: {e}");
        }
    });

    Ok(())
}

/// Get persisted chat history for display before model is loaded.
#[tauri::command]
async fn chat_get_history(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<llm_chat::ChatMessage>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        db.load_chat_history(&game_id, &bottle_name, 50)
            .map_err(|e| format!("Failed to load chat history: {e}"))
    })
    .await
    .map_err(|e| format!("Task join error: {e}"))?
}

/// Validate a cloud API key by making a minimal request.
#[tauri::command]
async fn chat_validate_cloud_key(
    provider: String,
    api_key: String,
) -> Result<String, String> {
    llm_chat::validate_cloud_key(&provider, &api_key).await
}

/// Get contextual conversation starters based on game state.
#[tauri::command]
async fn chat_get_starters(
    game_id: String,
    bottle_name: String,
    current_page: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<llm_chat::ChatStarter>, String> {
    let db = state.db.clone();
    let gid = game_id.clone();
    let bn = bottle_name.clone();

    let (mod_count, _enabled_count, disabled_count, has_conflicts) =
        tokio::task::spawn_blocking(move || {
            let mods = db.list_mods_summary(&gid, &bn).unwrap_or_default();
            let enabled = mods.iter().filter(|m| m.enabled).count();
            let disabled = mods.len() - enabled;
            let conflicts = db
                .find_all_conflicts(&gid, &bn)
                .map(|c| !c.is_empty())
                .unwrap_or(false);
            (mods.len(), enabled, disabled, conflicts)
        })
        .await
        .unwrap_or((0, 0, 0, false));

    // Check for new crash logs (fast filesystem stat).
    let crash_gid = game_id.clone();
    let crash_bn = bottle_name.clone();
    let crash_info = tokio::task::spawn_blocking(move || {
        match resolve_bottle(&crash_bn) {
            Ok(bottle) => {
                crashlog::check_new_crashes(&PathBuf::from(&bottle.path), &crash_gid)
            }
            Err(_) => crashlog::NewCrashInfo {
                count: 0,
                entries: vec![],
            },
        }
    })
    .await
    .unwrap_or(crashlog::NewCrashInfo {
        count: 0,
        entries: vec![],
    });

    let mut starters = Vec::new();

    // Prepend crash starter if new crashes detected.
    if crash_info.count > 0 {
        let label = if crash_info.count == 1 {
            "\u{1F534} New crash detected".to_string()
        } else {
            format!("\u{1F534} {} new crashes detected", crash_info.count)
        };
        starters.push(llm_chat::ChatStarter {
            label,
            prompt:
                "I just crashed. Can you analyze my latest crash log and tell me what went wrong?"
                    .into(),
        });
    }

    // Quick health check (wine compat only — fast, no expensive dep/conflict queries)
    let health_db = state.db.clone();
    let health_gid = game_id.clone();
    let health_bn = bottle_name.clone();
    let wine_warning_count = tokio::task::spawn_blocking(move || {
        let mods = health_db
            .list_mods_summary(&health_gid, &health_bn)
            .unwrap_or_default();
        let compat_input = wine_compat::build_compat_input(&mods);
        let warnings = wine_compat::check_all_mods_wine_compat(&compat_input);
        warnings
            .iter()
            .filter(|(_, w)| matches!(w.severity, wine_compat::Severity::Crash | wine_compat::Severity::Broken))
            .count()
    })
    .await
    .unwrap_or(0);

    if wine_warning_count > 0 {
        starters.push(llm_chat::ChatStarter {
            label: format!(
                "\u{26A0}\u{FE0F} {} Wine-incompatible mod{}",
                wine_warning_count,
                if wine_warning_count == 1 { "" } else { "s" }
            ),
            prompt: "Check my mod health score and tell me what issues to fix.".into(),
        });
    }

    let page = current_page.as_deref().unwrap_or("Mods");

    if has_conflicts {
        starters.push(llm_chat::ChatStarter {
            label: "Explain my mod conflicts".into(),
            prompt: "Check my mod conflicts and explain what's happening. Are any of them serious?"
                .into(),
        });
    }

    if disabled_count > 5 {
        starters.push(llm_chat::ChatStarter {
            label: format!("{} mods are disabled", disabled_count),
            prompt: "I have a lot of disabled mods. Can you review them and tell me which ones I might want to enable?".into(),
        });
    }

    match page {
        "Load Order" => {
            starters.push(llm_chat::ChatStarter {
                label: "Check my load order".into(),
                prompt: "Is my load order correct? Are there any issues I should fix?".into(),
            });
        }
        "Crash Logs" => {
            starters.push(llm_chat::ChatStarter {
                label: "Analyze my latest crash".into(),
                prompt: "Check my crash logs and analyze the most recent crash.".into(),
            });
        }
        "Discover" => {
            starters.push(llm_chat::ChatStarter {
                label: "Recommend mods for me".into(),
                prompt: "Based on my installed mods, what would you recommend I add?".into(),
            });
        }
        _ => {}
    }

    if mod_count > 0 {
        starters.push(llm_chat::ChatStarter {
            label: format!("Overview of my {} mods", mod_count),
            prompt: "Give me an overview of my mod setup. How many mods do I have, any issues?"
                .into(),
        });
    }

    if starters.len() < 3 {
        starters.push(llm_chat::ChatStarter {
            label: "Find me a mod".into(),
            prompt: "Help me find a good mod. What kind of mods are you looking for?".into(),
        });
    }

    starters.truncate(4);
    Ok(starters)
}

/// Helper: fuzzy-find a mod by name from the summary list.
fn find_mod_by_name<'a>(
    mods: &'a [database::ModSummary],
    name: &str,
) -> Option<&'a database::ModSummary> {
    let lower = name.to_lowercase();
    mods.iter()
        .find(|m| m.name.to_lowercase() == lower)
        .or_else(|| {
            let matches: Vec<_> = mods
                .iter()
                .filter(|m| m.name.to_lowercase().contains(&lower))
                .collect();
            if matches.len() == 1 {
                Some(matches[0])
            } else {
                None
            }
        })
}

/// Web search via DuckDuckGo lite HTML (no API key needed).
async fn web_search_ddg(query: &str) -> String {
    let client = match reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => return format!("Search failed: {e}"),
    };

    // Use Brave Search (DuckDuckGo blocks programmatic access with CAPTCHAs)
    let resp = match client
        .get("https://search.brave.com/search")
        .query(&[("q", query), ("source", "web")])
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return format!("Search failed: {e}"),
    };
    let html = match resp.text().await {
        Ok(t) => t,
        Err(e) => return format!("Search failed: {e}"),
    };

    // Brave Search: results have <a> tags with snippet-title spans
    // Structure: <div class="snippet"> containing <a href="URL"> with nested <span class="snippet-title">Title</span>
    // and <p class="snippet-description">Description</p>
    let mut results = Vec::new();

    fn strip_tags(s: &str) -> String {
        let mut clean = String::new();
        let mut in_tag = false;
        for ch in s.chars() {
            match ch {
                '<' => in_tag = true,
                '>' => in_tag = false,
                _ if !in_tag => clean.push(ch),
                _ => {}
            }
        }
        clean.trim().to_string()
    }

    // Split by snippet blocks
    let parts: Vec<&str> = html.split("class=\"snippet ").collect();
    for part in parts.iter().skip(1) {
        if results.len() >= 6 {
            break;
        }

        // Extract URL from first href="https://..."
        let url = part
            .split("href=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .unwrap_or("");
        if url.is_empty() || !url.starts_with("http") {
            continue;
        }

        // Extract title from snippet-title span
        let title = if let Some(pos) = part.find("snippet-title") {
            let after = &part[pos..];
            after
                .split('>')
                .nth(1)
                .and_then(|s| s.split("</span>").next())
                .map(|s| strip_tags(s))
                .unwrap_or_default()
        } else {
            String::new()
        };

        // Extract description from snippet-description
        let desc = if let Some(pos) = part.find("snippet-description") {
            let after = &part[pos..];
            after
                .split('>')
                .nth(1)
                .and_then(|s| s.split("</p>").next().or_else(|| s.split("</div>").next()))
                .map(|s| strip_tags(s))
                .unwrap_or_default()
        } else {
            String::new()
        };

        if !title.is_empty() {
            let snippet = if desc.len() > 200 {
                format!("{}...", &desc[..200])
            } else {
                desc
            };
            if snippet.is_empty() {
                results.push(format!("• {} ({})", title, url));
            } else {
                results.push(format!("• {} ({})\n  {}", title, url, snippet));
            }
        }
    }

    if results.is_empty() {
        format!("No web results found for \"{}\".", query)
    } else {
        format!("Web results for \"{}\":\n{}", query, results.join("\n"))
    }
}

/// Human-friendly display name for a tool while it's running.
fn tool_display_name(tool_name: &str, args: &serde_json::Value) -> String {
    match tool_name {
        "search_nexus" => {
            let q = args.get("query").and_then(|q| q.as_str()).unwrap_or("mods");
            format!("Searching NexusMods for \"{}\"...", q)
        }
        "web_search" => {
            let q = args.get("query").and_then(|q| q.as_str()).unwrap_or("...");
            format!("Searching the web for \"{}\"...", q)
        }
        "list_mods" => "Listing installed mods...".into(),
        "get_load_order" => "Checking load order...".into(),
        "get_conflicts" => "Checking mod conflicts...".into(),
        "get_mod_info" => {
            let n = args
                .get("mod_name")
                .and_then(|n| n.as_str())
                .unwrap_or("mod");
            format!("Getting info for \"{}\"...", n)
        }
        "get_nexus_mod_detail" => "Fetching mod details from NexusMods...".into(),
        "get_nexus_mod_files" => "Fetching available files...".into(),
        "download_and_install_mod" => "Downloading and installing mod...".into(),
        "sort_load_order" => "Sorting load order...".into(),
        "get_crash_logs" => "Checking crash logs...".into(),
        "analyze_crash_log" => "Analyzing crash log...".into(),
        "check_wine_compatibility" => "Checking Wine mod compatibility...".into(),
        "enable_mod" | "disable_mod" => {
            let n = args
                .get("mod_name")
                .and_then(|n| n.as_str())
                .unwrap_or("mod");
            format!(
                "{} \"{}\"...",
                if tool_name == "enable_mod" {
                    "Enabling"
                } else {
                    "Disabling"
                },
                n
            )
        }
        "check_mod_updates" => "Checking for mod updates...".into(),
        "run_preflight_check" => "Running preflight check...".into(),
        "redeploy_mods" => "Redeploying mods...".into(),
        "navigate_ui" => {
            let page = args.get("page").and_then(|p| p.as_str()).unwrap_or("page");
            format!("Navigating to {}...", page)
        }
        "open_nexus_mod" => {
            let name = args.get("name").and_then(|n| n.as_str()).unwrap_or("mod");
            format!("Opening {} in Discover...", name)
        }
        "find_needed_patches" => "Analyzing mod list for needed patches...".into(),
        "run_full_diagnostic" => "Running full diagnostic...".into(),
        "get_mod_requirements" => "Checking mod requirements...".into(),
        "batch_mod_operation" => {
            let action = args.get("action").and_then(|a| a.as_str()).unwrap_or("toggle");
            let filter = args.get("filter_value").and_then(|f| f.as_str()).unwrap_or("mods");
            format!("{}ing {} mods...", if action == "enable" { "Enabl" } else { "Disabl" }, filter)
        }
        "get_mod_health" => "Calculating mod health score...".into(),
        other => format!("Running {}...", other),
    }
}

/// Tool result display name for collapsible headers.
fn tool_result_display_name(tool_name: &str, result: &str) -> String {
    match tool_name {
        "list_mods" => {
            let count = result.split('\n').next().unwrap_or("").to_string();
            format!("Listed mods ({})", count.split(' ').next().unwrap_or("?"))
        }
        "search_nexus" => {
            let count = result.split('\n').next().unwrap_or("").to_string();
            format!(
                "NexusMods search ({})",
                count.split(' ').next().unwrap_or("?")
            )
        }
        "web_search" => "Web search results".into(),
        "get_load_order" => "Load order".into(),
        "get_conflicts" => "File conflicts".into(),
        "get_mod_info" => "Mod details".into(),
        "get_nexus_mod_detail" => "NexusMods details".into(),
        "get_nexus_mod_files" => "Available files".into(),
        "get_crash_logs" => "Crash logs".into(),
        "analyze_crash_log" => "Crash analysis".into(),
        "enable_mod" | "disable_mod" => result.lines().next().unwrap_or("Done").to_string(),
        "check_wine_compatibility" => "Wine compatibility check".into(),
        "find_needed_patches" => "Patch analysis".into(),
        "run_full_diagnostic" => "Diagnostic report".into(),
        "get_mod_requirements" => "Dependency check".into(),
        "batch_mod_operation" => "Batch mod operation".into(),
        "get_mod_health" => "Health score".into(),
        other => format!("{} result", other),
    }
}

/// Scan assistant response and tool results for mod references, cross-reference with installed mods.
async fn scan_mentioned_mods(
    content: &str,
    tool_results: &[llm_chat::ToolResult],
    game_id: &str,
    bottle_name: &str,
    state: &State<'_, AppState>,
) -> Vec<llm_chat::MentionedMod> {
    let db = state.db.clone();
    let gid = game_id.to_string();
    let bn = bottle_name.to_string();

    let mods = match tokio::task::spawn_blocking(move || {
        db.list_mods_summary(&gid, &bn).unwrap_or_default()
    })
    .await
    {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };

    let mut mentioned = Vec::new();
    let content_lower = content.to_lowercase();

    // Check installed mods against response text
    for m in &mods {
        let name_lower = m.name.to_lowercase();
        if name_lower.len() > 3 && content_lower.contains(&name_lower) {
            mentioned.push(llm_chat::MentionedMod {
                name: m.name.clone(),
                local_id: Some(m.id),
                nexus_mod_id: m.nexus_mod_id,
                enabled: Some(m.enabled),
                installed: true,
                picture_url: None,
            });
        }
    }

    // Check tool results for Nexus mod IDs (from structured_data)
    for tr in tool_results {
        if let Some(ref data) = tr.structured_data {
            if let Some(mods_arr) = data.as_array() {
                for nexus_mod in mods_arr {
                    let name = nexus_mod.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let mod_id = nexus_mod.get("mod_id").and_then(|i| i.as_i64());
                    let pic = nexus_mod
                        .get("picture_url")
                        .and_then(|p| p.as_str())
                        .map(String::from);
                    if !name.is_empty()
                        && !mentioned.iter().any(|m| m.name.eq_ignore_ascii_case(name))
                    {
                        let is_installed = mods.iter().any(|m| m.name.eq_ignore_ascii_case(name));
                        let local = mods.iter().find(|m| m.name.eq_ignore_ascii_case(name));
                        mentioned.push(llm_chat::MentionedMod {
                            name: name.to_string(),
                            local_id: local.map(|m| m.id),
                            nexus_mod_id: mod_id,
                            enabled: local.map(|m| m.enabled),
                            installed: is_installed,
                            picture_url: pic,
                        });
                    }
                }
            }
        }
    }

    // Limit to avoid overwhelming the UI
    mentioned.truncate(10);
    mentioned
}

/// Execute a tool call from the LLM.
async fn execute_tool(
    name: &str,
    args: &serde_json::Value,
    game_id: &str,
    bottle_name: &str,
    state: &State<'_, AppState>,
) -> llm_chat::ToolResult {
    let db = state.db.clone();
    let gid = game_id.to_string();
    let bn = bottle_name.to_string();

    let mut structured_data: Option<serde_json::Value> = None;

    let (result, success) = match name {
        // ── Basic: mod list & toggle ─────────────────────────────────
        "list_mods" => {
            let filter = args
                .get("filter")
                .and_then(|f| f.as_str())
                .unwrap_or("")
                .to_lowercase();
            let r = tokio::task::spawn_blocking(move || {
                let mods = db.list_mods_summary(&gid, &bn).unwrap_or_default();
                let filtered: Vec<_> = if filter.is_empty() {
                    mods.iter().collect()
                } else {
                    mods.iter()
                        .filter(|m| m.name.to_lowercase().contains(&filter))
                        .collect()
                };
                let lines: Vec<String> = filtered
                    .iter()
                    .map(|m| {
                        format!(
                            "{} [{}]",
                            m.name,
                            if m.enabled { "enabled" } else { "disabled" }
                        )
                    })
                    .collect();
                format!("{} mods found:\n{}", filtered.len(), lines.join("\n"))
            })
            .await
            .unwrap_or_else(|e| format!("Error: {e}"));
            (r, true)
        }

        "enable_mod" | "disable_mod" => {
            let mod_name = args
                .get("mod_name")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            let enable = name == "enable_mod";
            let r = tokio::task::spawn_blocking(move || {
                let mods = db.list_mods_summary(&gid, &bn).unwrap_or_default();
                match find_mod_by_name(&mods, &mod_name) {
                    Some(m) => match db.set_enabled(m.id, enable) {
                        Ok(_) => format!(
                            "{} \"{}\"",
                            if enable { "Enabled" } else { "Disabled" },
                            m.name
                        ),
                        Err(e) => format!("Failed: {e}"),
                    },
                    None => format!("Mod \"{}\" not found", mod_name),
                }
            })
            .await
            .unwrap_or_else(|e| format!("Error: {e}"));
            (r, true)
        }

        "get_mod_info" => {
            let mod_name = args
                .get("mod_name")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            let r = tokio::task::spawn_blocking(move || {
                let mods = db.list_mods_summary(&gid, &bn).unwrap_or_default();
                match find_mod_by_name(&mods, &mod_name) {
                    Some(m) => format!(
                        "Name: {}\nEnabled: {}\nVersion: {}\nFiles: {}\nCategory: {}\nCollection: {}\nOptional: {}\nNexus ID: {}",
                        m.name, m.enabled, m.version, m.file_count,
                        m.auto_category.as_deref().unwrap_or("uncategorized"),
                        m.collection_name.as_deref().unwrap_or("none"),
                        m.collection_optional,
                        m.nexus_mod_id.map_or("none".to_string(), |id| id.to_string()),
                    ),
                    None => format!("Mod \"{}\" not found", mod_name),
                }
            }).await.unwrap_or_else(|e| format!("Error: {e}"));
            (r, true)
        }

        "get_deployment_status" => {
            let r = tokio::task::spawn_blocking(move || {
                let mods = db.list_mods_summary(&gid, &bn).unwrap_or_default();
                let enabled = mods.iter().filter(|m| m.enabled).count();
                let collections: std::collections::HashSet<_> = mods
                    .iter()
                    .filter_map(|m| m.collection_name.as_deref())
                    .collect();
                format!(
                    "{} enabled / {} total mods, {} collections",
                    enabled,
                    mods.len(),
                    collections.len()
                )
            })
            .await
            .unwrap_or_else(|e| format!("Error: {e}"));
            (r, true)
        }

        "check_wine_compatibility" => {
            let r = tokio::task::spawn_blocking(move || {
                let mods = db.list_mods_summary(&gid, &bn).unwrap_or_default();
                let compat_input = wine_compat::build_compat_input(&mods);
                let warnings = wine_compat::check_all_mods_wine_compat(&compat_input);
                if warnings.is_empty() {
                    "No Wine compatibility issues detected. All enabled mods appear compatible.".to_string()
                } else {
                    wine_compat::format_warnings_report(&warnings)
                }
            })
            .await
            .unwrap_or_else(|e| format!("Error: {e}"));
            (r, true)
        }

        // ── Standard: load order, conflicts, Nexus search ────────────
        "get_load_order" => {
            let r = match get_plugin_order(gid.clone(), bn.clone()).await {
                Ok(plugins) => {
                    let lines: Vec<String> = plugins
                        .iter()
                        .enumerate()
                        .map(|(i, p)| {
                            format!(
                                "{:3}. {} [{}]",
                                i,
                                p.filename,
                                if p.enabled { "active" } else { "inactive" }
                            )
                        })
                        .collect();
                    format!("{} plugins:\n{}", plugins.len(), lines.join("\n"))
                }
                Err(e) => format!("Error: {e}"),
            };
            (r, true)
        }

        "get_conflicts" => {
            let db2 = state.db.clone();
            let db3 = state.db.clone();
            let gid2 = gid.clone();
            let gid3 = gid.clone();
            let bn2 = bn.clone();
            let bn3 = bn.clone();
            let r = tokio::task::spawn_blocking(move || {
                let conflicts = match db2
                    .find_all_conflicts(&gid2, &bn2)
                    .map_err(|e| e.to_string())
                {
                    Ok(c) => c,
                    Err(e) => return format!("Error: {e}"),
                };

                if conflicts.is_empty() {
                    return "No file conflicts detected.".into();
                }

                // Build mod_name -> auto_category lookup
                let mod_categories: std::collections::HashMap<String, String> = db3
                    .list_mods_summary(&gid3, &bn3)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|m| {
                        let cat = m.auto_category.unwrap_or_else(|| "uncategorized".into());
                        (m.name, cat)
                    })
                    .collect();

                // Use cleaner's categorize_file for consistent file type detection

                // Group conflicts by mod pair (sorted pair of mod names)
                struct PairInfo {
                    mod_a: String,
                    mod_b: String,
                    type_counts: std::collections::HashMap<String, usize>,
                    winner: String,
                    same_collection: bool,
                }

                let mut pairs: std::collections::HashMap<(String, String), PairInfo> =
                    std::collections::HashMap::new();

                for c in &conflicts {
                    let file_cat = cleaner::categorize_file(&c.relative_path);
                    let winner_name = c
                        .mods
                        .iter()
                        .find(|m| m.mod_id == c.winner_mod_id)
                        .map(|m| m.mod_name.clone())
                        .unwrap_or_default();

                    // For each pair of conflicting mods in this file
                    for i in 0..c.mods.len() {
                        for j in (i + 1)..c.mods.len() {
                            let (a, b) = if c.mods[i].mod_name <= c.mods[j].mod_name {
                                (c.mods[i].mod_name.clone(), c.mods[j].mod_name.clone())
                            } else {
                                (c.mods[j].mod_name.clone(), c.mods[i].mod_name.clone())
                            };
                            let key = (a.clone(), b.clone());
                            let entry = pairs.entry(key).or_insert_with(|| PairInfo {
                                mod_a: a,
                                mod_b: b,
                                type_counts: std::collections::HashMap::new(),
                                winner: winner_name.clone(),
                                same_collection: c.same_collection,
                            });
                            *entry.type_counts.entry(file_cat.clone()).or_insert(0) += 1;
                        }
                    }
                }

                // Sort pairs by total conflict count descending
                let mut pair_list: Vec<PairInfo> = pairs.into_values().collect();
                pair_list.sort_by(|a, b| {
                    let total_a: usize = a.type_counts.values().sum();
                    let total_b: usize = b.type_counts.values().sum();
                    total_b.cmp(&total_a)
                });

                let total_conflicts = conflicts.len();
                let total_pairs = pair_list.len();
                let mut output = format!(
                    "{} conflicts found between {} mod pair{}:\n",
                    total_conflicts,
                    total_pairs,
                    if total_pairs == 1 { "" } else { "s" }
                );

                for (i, pair) in pair_list.iter().enumerate() {
                    if i >= 15 {
                        output.push_str(&format!(
                            "\n...and {} more mod pairs",
                            total_pairs - 15
                        ));
                        break;
                    }
                    let total: usize = pair.type_counts.values().sum();
                    let cat_a = mod_categories
                        .get(&pair.mod_a)
                        .map(|s| s.as_str())
                        .unwrap_or("uncategorized");
                    let cat_b = mod_categories
                        .get(&pair.mod_b)
                        .map(|s| s.as_str())
                        .unwrap_or("uncategorized");

                    // Format type breakdown: "3 Mesh, 2 Texture"
                    let mut types: Vec<(&String, &usize)> = pair.type_counts.iter().collect();
                    types.sort_by(|a, b| b.1.cmp(a.1));
                    let type_str: Vec<String> = types
                        .iter()
                        .map(|(t, n)| format!("{} {}", n, t))
                        .collect();

                    output.push_str(&format!(
                        "\n{} vs {} ({} conflict{}):\n  - {} ({} vs {})\n  - Winner: {}{}\n",
                        pair.mod_a,
                        pair.mod_b,
                        total,
                        if total == 1 { "" } else { "s" },
                        type_str.join(", "),
                        cat_a,
                        cat_b,
                        pair.winner,
                        if pair.same_collection {
                            " [same collection - expected overlap]"
                        } else {
                            ""
                        },
                    ));
                }

                output.push_str(
                    "\nHigher-priority mod wins file conflicts. Use sort_load_order for plugin ordering.",
                );
                output
            })
            .await
            .unwrap_or_else(|e| format!("Error: {e}"));
            (r, true)
        }

        "web_search" => {
            let query = args
                .get("query")
                .and_then(|q| q.as_str())
                .unwrap_or("")
                .to_string();
            let r = web_search_ddg(&query).await;
            (r, true)
        }

        "search_nexus" => {
            let query = args
                .get("query")
                .and_then(|q| q.as_str())
                .unwrap_or("")
                .to_string();
            // Map LLM-friendly sort names to NexusMods GraphQL field names
            let sort = args.get("sort_by").and_then(|s| s.as_str()).map(|s| {
                match s {
                    "total_downloads" | "downloads" => "downloads",
                    "latest_updated" | "updated" => "updatedAt",
                    "endorsements" | "endorsement_count" => "endorsements",
                    other => other,
                }
                .to_string()
            });
            let include_adult = args
                .get("include_adult")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let game_slug = nexus_game_slug(&gid);
            let r = match search_nexus_mods_cmd(
                game_slug,
                Some(query),
                sort,
                None,
                10,
                0,
                include_adult,
                None,
                None,
                None,
                None,
                None,
            )
            .await
            {
                Ok(result) => {
                    // Build structured data for rich mod cards
                    let cards: Vec<serde_json::Value> = result
                        .mods
                        .iter()
                        .map(|m| {
                            serde_json::json!({
                                "mod_id": m.mod_id,
                                "name": m.name,
                                "summary": m.summary,
                                "author": m.author,
                                "picture_url": m.picture_url,
                                "unique_downloads": m.unique_downloads,
                                "endorsements": m.endorsement_count,
                            })
                        })
                        .collect();
                    if !cards.is_empty() {
                        structured_data = Some(serde_json::json!(cards));
                    }
                    let lines: Vec<String> = result
                        .mods
                        .iter()
                        .map(|m| {
                            format!(
                                "[{}] {} — {} downloads — {}",
                                m.mod_id,
                                m.name,
                                m.unique_downloads,
                                m.summary.chars().take(80).collect::<String>()
                            )
                        })
                        .collect();
                    if lines.is_empty() {
                        "No mods found matching that search.".into()
                    } else {
                        format!("{} results:\n{}", lines.len(), lines.join("\n"))
                    }
                }
                Err(e) => format!("Search error: {e}"),
            };
            (r, true)
        }

        "get_nexus_mod_detail" => {
            let mod_id = args.get("mod_id").and_then(|v| v.as_i64()).unwrap_or(0);
            let game_slug = nexus_game_slug(&gid);
            let r = match get_nexus_mod_detail(game_slug, mod_id).await {
                Ok(info) => format!(
                    "Name: {}\nAuthor: {}\nVersion: {}\nDownloads: {}\nEndorsements: {}\nSummary: {}\nDescription: {}",
                    info.name,
                    info.author,
                    info.version,
                    info.unique_downloads,
                    info.endorsement_count,
                    info.summary,
                    info.description.as_deref().unwrap_or("").chars().take(500).collect::<String>(),
                ),
                Err(e) => format!("Error: {e}"),
            };
            (r, true)
        }

        "get_nexus_mod_files" => {
            let mod_id = args.get("mod_id").and_then(|v| v.as_i64()).unwrap_or(0);
            let game_slug = nexus_game_slug(&gid);
            let r = match get_nexus_mod_files(game_slug, mod_id).await {
                Ok(files) => {
                    let lines: Vec<String> = files
                        .iter()
                        .map(|f| format!("[{}] {} ({} KB)", f.file_id, f.name, f.size_kb))
                        .collect();
                    format!("{} files:\n{}", lines.len(), lines.join("\n"))
                }
                Err(e) => format!("Error: {e}"),
            };
            (r, true)
        }

        "check_mod_updates" => {
            let db2 = state.db.clone();
            let gid2 = gid.clone();
            let bn2 = bn.clone();
            let r = match tokio::task::spawn_blocking(move || {
                let mods = db2
                    .list_mods_summary(&gid2, &bn2)
                    .map_err(|e| e.to_string())?;
                let nexus_mods: Vec<_> = mods
                    .iter()
                    .filter_map(|m| {
                        m.nexus_mod_id
                            .map(|nid| (m.name.clone(), m.version.clone(), nid))
                    })
                    .collect();
                Ok::<_, String>(nexus_mods)
            })
            .await
            {
                Ok(Ok(nexus_mods)) => {
                    if nexus_mods.is_empty() {
                        "No mods with Nexus IDs to check.".into()
                    } else {
                        format!("{} mods have Nexus IDs. Use get_nexus_mod_detail to check individual mod versions.", nexus_mods.len())
                    }
                }
                Ok(Err(e)) => format!("Error: {e}"),
                Err(e) => format!("Error: {e}"),
            };
            (r, true)
        }

        "get_mod_recommendations" => {
            let mod_name = args
                .get("mod_name")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            let r = tokio::task::spawn_blocking({
                let db2 = db.clone();
                let gid2 = gid.clone();
                let bn2 = bn.clone();
                move || {
                    let mods = db2.list_mods_summary(&gid2, &bn2).unwrap_or_default();
                    match find_mod_by_name(&mods, &mod_name) {
                        Some(m) => match get_mod_recommendations_sync(&db2, &gid2, &bn2, m.id) {
                            Ok(recs) => {
                                if recs.is_empty() {
                                    format!("No recommendations found for \"{}\"", m.name)
                                } else {
                                    let lines: Vec<String> = recs
                                        .iter()
                                        .take(10)
                                        .map(|r| format!("{} (score: {})", r.0, r.1))
                                        .collect();
                                    format!(
                                        "Recommended with \"{}\":\n{}",
                                        m.name,
                                        lines.join("\n")
                                    )
                                }
                            }
                            Err(e) => format!("Error: {e}"),
                        },
                        None => format!("Mod \"{}\" not found", mod_name),
                    }
                }
            })
            .await
            .unwrap_or_else(|e| format!("Error: {e}"));
            (r, true)
        }

        "get_popular_companion_mods" => {
            let r =
                tokio::task::spawn_blocking(move || {
                    match mod_recommendations::get_popular_mods(&db, &gid, &bn) {
                        Ok(popular) => {
                            let lines: Vec<String> = popular
                                .iter()
                                .take(15)
                                .map(|(name, _nexus_id, count)| {
                                    format!("{} (installed by {} users)", name, count)
                                })
                                .collect();
                            if lines.is_empty() {
                                "No popularity data available.".into()
                            } else {
                                format!("Popular mods:\n{}", lines.join("\n"))
                            }
                        }
                        Err(e) => format!("Error: {e}"),
                    }
                })
                .await
                .unwrap_or_else(|e| format!("Error: {e}"));
            (r, true)
        }

        // ── Advanced: install, sort, crash analysis, profiles ─────────
        "download_and_install_mod" => {
            let mod_id = args.get("mod_id").and_then(|v| v.as_i64()).unwrap_or(0);
            let file_id = args.get("file_id").and_then(|v| v.as_i64()).unwrap_or(0);
            (format!("To install mod {} (file {}), please use the Downloads tab in the UI. I can help you find the right mod and file ID using search_nexus and get_nexus_mod_files.", mod_id, file_id), true)
        }

        "sort_load_order" => {
            let r = match sort_plugins_loot(gid.clone(), bn.clone()).await {
                Ok(result) => {
                    format!(
                        "Load order sorted. {} plugins reordered, {} warnings.",
                        result.plugins_moved,
                        result.warnings.len()
                    )
                }
                Err(e) => format!("Sort failed: {e}"),
            };
            (r, true)
        }

        "get_crash_logs" => {
            let r = match find_crash_logs_cmd(gid.clone(), bn.clone()).await {
                Ok(logs) => {
                    if logs.is_empty() {
                        "No crash logs found.".into()
                    } else {
                        let lines: Vec<String> = logs
                            .iter()
                            .take(5)
                            .map(|l| format!("{}: {} — {}", l.timestamp, l.filename, l.summary))
                            .collect();
                        format!("{} crash logs found:\n{}", logs.len(), lines.join("\n"))
                    }
                }
                Err(e) => format!("Error: {e}"),
            };
            (r, true)
        }

        "analyze_crash_log" => {
            let log_path = args
                .get("log_path")
                .and_then(|p| p.as_str())
                .unwrap_or("")
                .to_string();
            let r = match analyze_crash_log_cmd(log_path).await {
                Ok(report) => {
                    let diagnoses: Vec<String> = report
                        .diagnosis
                        .iter()
                        .map(|d| format!("  - {}: {}", d.title, d.description))
                        .collect();
                    format!(
                        "Crash Analysis:\nModule: {}\nPlugins involved: {}\nDiagnosis:\n{}",
                        report.module_name,
                        if report.involved_plugins.is_empty() {
                            "none".into()
                        } else {
                            report.involved_plugins.join(", ")
                        },
                        if diagnoses.is_empty() {
                            "  No specific diagnosis.".into()
                        } else {
                            diagnoses.join("\n")
                        },
                    )
                }
                Err(e) => format!("Analysis failed: {e}"),
            };
            (r, true)
        }

        "list_profiles" => {
            let db2 = state.db.clone();
            let gid2 = gid.clone();
            let bn2 = bn.clone();
            let r = tokio::task::spawn_blocking(move || {
                match profiles::list_profiles(&db2, &gid2, &bn2) {
                    Ok(profs) => {
                        let lines: Vec<String> = profs
                            .iter()
                            .map(|p| {
                                format!("{}{}", p.name, if p.is_active { " (active)" } else { "" })
                            })
                            .collect();
                        if lines.is_empty() {
                            "No profiles created.".into()
                        } else {
                            format!("{} profiles:\n{}", lines.len(), lines.join("\n"))
                        }
                    }
                    Err(e) => format!("Error: {e}"),
                }
            })
            .await
            .unwrap_or_else(|e| format!("Error: {e}"));
            (r, true)
        }

        "activate_profile" => {
            let profile_name = args
                .get("profile_name")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            let db2 = state.db.clone();
            let gid2 = gid.clone();
            let bn2 = bn.clone();
            let r = tokio::task::spawn_blocking(move || {
                let profs =
                    profiles::list_profiles(&db2, &gid2, &bn2).map_err(|e| e.to_string())?;
                let lower = profile_name.to_lowercase();
                match profs.iter().find(|p| p.name.to_lowercase() == lower) {
                    Some(p) => {
                        profiles::set_active_profile(&db2, &gid2, &bn2, p.id)
                            .map_err(|e| e.to_string())?;
                        Ok(format!("Activated profile \"{}\"", p.name))
                    }
                    None => Ok(format!("Profile \"{}\" not found", profile_name)),
                }
            })
            .await
            .unwrap_or_else(|e| Ok(format!("Error: {e}")))
            .unwrap_or_else(|e: String| format!("Error: {e}"));
            (r, true)
        }

        "run_preflight_check" => {
            let db2 = state.db.clone();
            let gid2 = gid.clone();
            let bn2 = bn.clone();
            let r = tokio::task::spawn_blocking(move || {
                let (bottle, _, data_dir) = resolve_game(&gid2, &bn2)?;
                let result = preflight::run_preflight(&db2, &bottle, &gid2, &bn2, &data_dir);
                let mut lines = Vec::new();
                lines.push(format!(
                    "{} passed, {} failed, {} warnings",
                    result.passed, result.failed, result.warnings
                ));
                if result.can_proceed {
                    lines.push("Can proceed with launch.".into());
                } else {
                    lines.push("Issues must be resolved before launch.".into());
                }
                for check in &result.checks {
                    lines.push(format!(
                        "  [{:?}] {}: {}",
                        check.status, check.name, check.message
                    ));
                }
                Ok::<String, String>(lines.join("\n"))
            })
            .await
            .unwrap_or_else(|e| Ok(format!("Error: {e}")))
            .unwrap_or_else(|e| format!("Error: {e}"));
            (r, true)
        }

        "check_dependency_issues" => {
            let db2 = state.db.clone();
            let gid2 = gid.clone();
            let bn2 = bn.clone();
            let r =
                tokio::task::spawn_blocking(
                    move || match mod_dependencies::check_dependency_issues(&db2, &gid2, &bn2) {
                        Ok(issues) => {
                            if issues.is_empty() {
                                "No dependency issues found.".into()
                            } else {
                                let lines: Vec<String> = issues
                                    .iter()
                                    .take(15)
                                    .map(|i| format!("{}: {}", i.mod_name, i.message))
                                    .collect();
                                format!("{} issues:\n{}", issues.len(), lines.join("\n"))
                            }
                        }
                        Err(e) => format!("Error: {e}"),
                    },
                )
                .await
                .unwrap_or_else(|e| format!("Error: {e}"));
            (r, true)
        }

        "redeploy_mods" => {
            let db2 = state.db.clone();
            let gid2 = gid.clone();
            let bn2 = bn.clone();
            let r = tokio::task::spawn_blocking(move || {
                let (_bottle, game, data_dir) = resolve_game(&gid2, &bn2)?;
                let game_path = game.game_path.clone();
                match deployer::redeploy_all(&db2, &gid2, &bn2, &data_dir, &game_path) {
                    Ok(result) => Ok(format!(
                        "Redeployment complete: {} files deployed, {} skipped",
                        result.deployed_count, result.skipped_count
                    )),
                    Err(e) => Err(format!("Redeploy failed: {e}")),
                }
            })
            .await
            .unwrap_or_else(|e| Ok(format!("Error: {e}")))
            .unwrap_or_else(|e| e);
            (r, true)
        }

        "get_mod_requirements" => {
            let mod_id = args.get("mod_id").and_then(|v| v.as_i64()).unwrap_or(0);
            let game_slug = nexus_game_slug(&gid);

            let mod_info = get_nexus_mod_detail(game_slug, mod_id).await;
            let r = match mod_info {
                Ok(info) => {
                    let full_text = format!(
                        "{} {}",
                        info.summary,
                        info.description.as_deref().unwrap_or("")
                    );
                    let text_lower = full_text.to_lowercase();

                    // Strip HTML tags for cleaner matching
                    let tag_re = regex::Regex::new(r"<[^>]+>")
                        .unwrap_or_else(|_| regex::Regex::new("$^").unwrap());
                    let text_clean = tag_re.replace_all(&text_lower, " ").to_string();

                    let has_req_context = text_clean.contains("require")
                        || text_clean.contains("requirement")
                        || text_clean.contains("dependenc")
                        || text_clean.contains("you need")
                        || text_clean.contains("prerequisite")
                        || text_clean.contains("needed")
                        || text_clean.contains("must have")
                        || text_clean.contains("install first");

                    // Get installed mod names for cross-referencing
                    let db2 = state.db.clone();
                    let gid2 = gid.clone();
                    let bn2 = bn.clone();
                    let installed_mods = tokio::task::spawn_blocking(move || {
                        db2.list_mods_summary(&gid2, &bn2).unwrap_or_default()
                    })
                    .await
                    .unwrap_or_default();

                    let installed_names_lower: Vec<String> = installed_mods
                        .iter()
                        .map(|m| m.name.to_lowercase())
                        .collect();

                    let mut detected: Vec<(&str, i64, bool)> = Vec::new();

                    for &(fw_name, fw_nexus_id) in KNOWN_FRAMEWORKS {
                        let fw_lower = fw_name.to_lowercase();

                        let mentioned = text_clean.contains(&fw_lower) || {
                            let short = fw_lower
                                .split(|c: char| c == '-' || c == ' ')
                                .next()
                                .unwrap_or(&fw_lower);
                            short.len() > 3 && text_clean.contains(short)
                        };

                        if mentioned {
                            let is_installed = installed_names_lower.iter().any(|installed| {
                                installed.contains(&fw_lower)
                                    || fw_lower.contains(installed.as_str())
                                    || {
                                        let short = fw_lower
                                            .split(|c: char| c == '-' || c == ' ')
                                            .next()
                                            .unwrap_or(&fw_lower);
                                        short.len() > 3 && installed.contains(short)
                                    }
                            });

                            if !detected.iter().any(|(_, nid, _)| *nid == fw_nexus_id) {
                                detected.push((fw_name, fw_nexus_id, is_installed));
                            }
                        }
                    }

                    let mut out =
                        format!("Requirements for {} (Nexus ID {}):\n", info.name, mod_id);

                    if detected.is_empty() {
                        if has_req_context {
                            out.push_str(
                                "The mod mentions requirements but none matched known frameworks.\nCheck the mod page manually for specific requirements.\n",
                            );
                        } else {
                            out.push_str("No known framework dependencies detected.\n");
                        }
                    } else {
                        let mut missing_count = 0;
                        for (fw_name, fw_nexus_id, is_installed) in &detected {
                            if *is_installed {
                                out.push_str(&format!("[installed] {}\n", fw_name));
                            } else {
                                missing_count += 1;
                                out.push_str(&format!(
                                    "[MISSING]   {} — NOT installed (Nexus ID: {})\n",
                                    fw_name, fw_nexus_id
                                ));
                            }
                        }
                        if missing_count > 0 {
                            out.push_str(
                                "\nUse open_nexus_mod to show missing mods for installation.",
                            );
                        } else {
                            out.push_str("\nAll detected dependencies are installed.");
                        }
                    }

                    out
                }
                Err(e) => format!("Error fetching mod details: {e}"),
            };
            (r, true)
        }

        "find_needed_patches" => {
            let r = tokio::task::spawn_blocking(move || {
                let mods = db.list_mods_summary(&gid, &bn).unwrap_or_default();
                let enabled: Vec<_> = mods.iter().filter(|m| m.enabled).collect();
                let total = enabled.len();

                // Group by auto_category
                let mut groups: std::collections::HashMap<String, Vec<String>> =
                    std::collections::HashMap::new();
                for m in &enabled {
                    let cat = m
                        .auto_category
                        .as_deref()
                        .unwrap_or("uncategorized")
                        .to_string();
                    groups.entry(cat).or_default().push(m.name.clone());
                }

                // Only keep categories with 2+ mods (potential conflicts)
                let mut conflict_groups: Vec<(String, Vec<String>)> = groups
                    .into_iter()
                    .filter(|(cat, members)| {
                        members.len() >= 2 && cat != "uncategorized"
                    })
                    .collect();
                conflict_groups.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

                let mut out = format!(
                    "Analyzing {} enabled mods for patch needs...\n\n",
                    total
                );

                if conflict_groups.is_empty() {
                    out.push_str(
                        "No multi-mod category groups found. Mods may still need patches — use your knowledge of Skyrim modding to identify common pairs that need compatibility patches, then search_nexus for them.",
                    );
                } else {
                    out.push_str("Potential conflict groups:\n");
                    let mut suggestions = Vec::new();
                    for (cat, members) in &conflict_groups {
                        out.push_str(&format!(
                            "- {} ({} mods): {}\n",
                            cat,
                            members.len(),
                            members.join(", ")
                        ));
                        // Generate search suggestions for pairs within the group
                        if members.len() >= 2 {
                            for i in 0..members.len().min(3) {
                                for j in (i + 1)..members.len().min(4) {
                                    suggestions.push(format!(
                                        "\"{}\" \"{}\" patch",
                                        members[i], members[j]
                                    ));
                                }
                            }
                        }
                    }
                    if !suggestions.is_empty() {
                        out.push_str("\nSuggested searches:\n");
                        for s in suggestions.iter().take(8) {
                            out.push_str(&format!("- {}\n", s));
                        }
                    }
                    out.push_str("\nUse search_nexus to find patches for these combinations, then open_nexus_mod to show them to the user.");
                }

                out
            })
            .await
            .unwrap_or_else(|e| format!("Error: {e}"));
            (r, true)
        }

        "batch_mod_operation" => {
            let action = args.get("action").and_then(|a| a.as_str()).unwrap_or("disable").to_string();
            let filter_type = args.get("filter_type").and_then(|f| f.as_str()).unwrap_or("").to_string();
            let filter_value = args.get("filter_value").and_then(|f| f.as_str()).unwrap_or("").to_string();
            let enable = action == "enable";
            let r = tokio::task::spawn_blocking(move || {
                let mods = db.list_mods_summary(&gid, &bn).unwrap_or_default();
                let filter_lower = filter_value.to_lowercase();

                // Apply filter
                let filtered: Vec<&database::ModSummary> = mods.iter().filter(|m| {
                    let type_match = match filter_type.as_str() {
                        "category" => m.auto_category.as_deref().unwrap_or("").to_lowercase().contains(&filter_lower),
                        "collection" => m.collection_name.as_deref().unwrap_or("").to_lowercase() == filter_lower,
                        "name_contains" => m.name.to_lowercase().contains(&filter_lower),
                        "optional" => m.collection_optional,
                        "all_disabled" => !m.enabled,
                        "all_enabled" => m.enabled,
                        _ => false,
                    };
                    // Only include mods that would actually change state
                    type_match && (m.enabled != enable)
                }).collect();

                if filtered.is_empty() {
                    return "No mods match that filter (or they are already in the desired state).".to_string();
                }

                // Execute batch operation
                let mut changed = Vec::new();
                let mut errors = Vec::new();
                for m in &filtered {
                    match db.set_enabled(m.id, enable) {
                        Ok(_) => changed.push(m.name.clone()),
                        Err(e) => errors.push(format!("{}: {}", m.name, e)),
                    }
                }

                let action_past = if enable { "Enabled" } else { "Disabled" };
                let filter_desc = match filter_type.as_str() {
                    "category" => format!("{} mods", filter_value),
                    "collection" => format!("mods from \"{}\"", filter_value),
                    "name_contains" => format!("mods matching \"{}\"", filter_value),
                    "optional" => "optional mods".to_string(),
                    "all_disabled" => "all previously disabled mods".to_string(),
                    "all_enabled" => "all previously enabled mods".to_string(),
                    _ => "mods".to_string(),
                };

                let mut out = format!("{} {} {}:\n", action_past, changed.len(), filter_desc);
                let show_count = changed.len().min(10);
                for name in &changed[..show_count] {
                    out.push_str(&format!("- {}\n", name));
                }
                if changed.len() > show_count {
                    out.push_str(&format!("- ... (and {} more)\n", changed.len() - show_count));
                }
                if !errors.is_empty() {
                    out.push_str(&format!("\n{} errors:\n", errors.len()));
                    for e in &errors {
                        out.push_str(&format!("- {}\n", e));
                    }
                }
                let reverse_action = if enable { "disable" } else { "enable" };
                out.push_str(&format!("\nTo undo, ask me to \"{}\" these mods.", reverse_action));
                // Trigger redeploy notification
                out.push_str("\n\nNote: Run redeploy_mods to apply changes to the game directory.");
                out
            })
            .await
            .unwrap_or_else(|e| format!("Error: {e}"));
            (r, true)
        }


        "run_full_diagnostic" => {
            // Run all diagnostic checks in parallel
            let db_pf = state.db.clone();
            let gid_pf = gid.clone();
            let bn_pf = bn.clone();
            let preflight_fut = tokio::task::spawn_blocking(move || {
                let (bottle, _, data_dir) = resolve_game(&gid_pf, &bn_pf)?;
                let result =
                    preflight::run_preflight(&db_pf, &bottle, &gid_pf, &bn_pf, &data_dir);
                let status = if result.failed > 0 {
                    "FAIL"
                } else if result.warnings > 0 {
                    "WARN"
                } else {
                    "PASS"
                };
                let mut lines = vec![
                    format!("PREFLIGHT: [{}]", status),
                    format!(
                        "- {} passed, {} failed, {} warnings",
                        result.passed, result.failed, result.warnings
                    ),
                ];
                for check in &result.checks {
                    lines.push(format!(
                        "- [{:?}] {}: {}",
                        check.status, check.name, check.message
                    ));
                }
                Ok::<String, String>(lines.join("\n"))
            });

            // Wine compat + mod summary share a single list_mods_summary call
            let db_wc = state.db.clone();
            let gid_wc = gid.clone();
            let bn_wc = bn.clone();
            let wine_and_summary_fut = tokio::task::spawn_blocking(move || {
                let mods = db_wc
                    .list_mods_summary(&gid_wc, &bn_wc)
                    .unwrap_or_default();
                let enabled = mods.iter().filter(|m| m.enabled).count();
                let mod_summary = format!("MOD SUMMARY: {} enabled / {} total", enabled, mods.len());

                let compat_input = wine_compat::build_compat_input(&mods);
                let warnings = wine_compat::check_all_mods_wine_compat(&compat_input);
                let wine_result = if warnings.is_empty() {
                    "WINE COMPATIBILITY: [OK]\n- No issues detected".to_string()
                } else {
                    format!(
                        "WINE COMPATIBILITY: [WARNINGS]\n{}",
                        wine_compat::format_warnings_report(&warnings)
                    )
                };
                (wine_result, mod_summary)
            });

            let db_dep = state.db.clone();
            let gid_dep = gid.clone();
            let bn_dep = bn.clone();
            let dep_fut = tokio::task::spawn_blocking(move || {
                match mod_dependencies::check_dependency_issues(&db_dep, &gid_dep, &bn_dep) {
                    Ok(issues) => {
                        if issues.is_empty() {
                            "DEPENDENCIES: [OK]\n- No issues found".into()
                        } else {
                            let lines: Vec<String> = issues
                                .iter()
                                .take(15)
                                .map(|i| format!("- {}: {}", i.mod_name, i.message))
                                .collect();
                            format!(
                                "DEPENDENCIES: [ISSUES] ({} found)\n{}",
                                issues.len(),
                                lines.join("\n")
                            )
                        }
                    }
                    Err(e) => format!("DEPENDENCIES: [ERROR]\n- {e}"),
                }
            });

            let db_cf = state.db.clone();
            let gid_cf = gid.clone();
            let bn_cf = bn.clone();
            let conflict_fut = tokio::task::spawn_blocking(move || {
                match db_cf
                    .find_all_conflicts(&gid_cf, &bn_cf)
                    .map_err(|e| e.to_string())
                {
                    Ok(conflicts) => {
                        if conflicts.is_empty() {
                            "CONFLICTS: 0 total".into()
                        } else {
                            // Group by mod pair for consistency with get_conflicts output
                            let mut pair_counts: std::collections::HashMap<(String, String), usize> =
                                std::collections::HashMap::new();
                            for c in &conflicts {
                                for i in 0..c.mods.len() {
                                    for j in (i + 1)..c.mods.len() {
                                        let (a, b) = if c.mods[i].mod_name <= c.mods[j].mod_name {
                                            (c.mods[i].mod_name.clone(), c.mods[j].mod_name.clone())
                                        } else {
                                            (c.mods[j].mod_name.clone(), c.mods[i].mod_name.clone())
                                        };
                                        *pair_counts.entry((a, b)).or_insert(0) += 1;
                                    }
                                }
                            }
                            let mut pairs: Vec<_> = pair_counts.into_iter().collect();
                            pairs.sort_by(|a, b| b.1.cmp(&a.1));
                            let lines: Vec<String> = pairs
                                .iter()
                                .take(10)
                                .map(|((a, b), count)| format!("- {} vs {} ({} files)", a, b, count))
                                .collect();
                            format!(
                                "CONFLICTS: {} total, {} mod pairs\n{}",
                                conflicts.len(),
                                pairs.len(),
                                lines.join("\n")
                            )
                        }
                    }
                    Err(e) => format!("CONFLICTS: [ERROR]\n- {e}"),
                }
            });

            // Await all in parallel
            let (preflight_r, wine_summary_r, dep_r, conflict_r) =
                tokio::join!(preflight_fut, wine_and_summary_fut, dep_fut, conflict_fut);

            let preflight_result = preflight_r
                .unwrap_or_else(|e| Ok(format!("PREFLIGHT: [ERROR]\n- {e}")))
                .unwrap_or_else(|e| format!("PREFLIGHT: [ERROR]\n- {e}"));
            let (wine_result, mod_summary) = wine_summary_r
                .unwrap_or_else(|_| ("WINE COMPATIBILITY: [ERROR]".into(), "MOD SUMMARY: [ERROR]".into()));
            let dep_result = dep_r
                .unwrap_or_else(|e| format!("DEPENDENCIES: [ERROR]\n- {e}"));
            let conflict_result = conflict_r
                .unwrap_or_else(|e| format!("CONFLICTS: [ERROR]\n- {e}"));

            // Determine highest severity area for focus recommendation
            let focus = if preflight_result.contains("[FAIL]") {
                "preflight failures (must be resolved before launching)"
            } else if wine_result.contains("[WARNINGS]") {
                "Wine compatibility warnings (may cause crashes)"
            } else if dep_result.contains("[ISSUES]") {
                "dependency issues (missing or circular dependencies)"
            } else if !conflict_result.starts_with("CONFLICTS: 0") {
                "file conflicts (may cause unexpected behavior)"
            } else {
                "no major issues detected \u{2014} check crash logs if problem persists"
            };

            let r = format!(
                "=== FULL DIAGNOSTIC REPORT ===\n\n{}\n\n{}\n\n{}\n\n{}\n\n{}\n\nBased on this report, focus investigation on {}.",
                preflight_result, wine_result, dep_result, conflict_result, mod_summary, focus
            );
            (r, true)
        }

        "get_mod_health" => {
            let db2 = state.db.clone();
            let gid2 = gid.clone();
            let bn2 = bn.clone();
            let r = tokio::task::spawn_blocking(move || {
                let mut score: i32 = 100;
                let mut issues: Vec<serde_json::Value> = Vec::new();

                let mods = db2.list_mods_summary(&gid2, &bn2).unwrap_or_default();
                let enabled_count = mods.iter().filter(|m| m.enabled).count();

                // 1. Check Wine compatibility
                let compat_input = wine_compat::build_compat_input(&mods);
                let warnings = wine_compat::check_all_mods_wine_compat(&compat_input);
                for (mod_name, warning) in &warnings {
                    let (penalty, severity_str) = match warning.severity {
                        wine_compat::Severity::Crash => (30, "error"),
                        wine_compat::Severity::Broken => (15, "warning"),
                        wine_compat::Severity::Degraded => (5, "info"),
                    };
                    score -= penalty;
                    issues.push(serde_json::json!({
                        "severity": severity_str,
                        "message": format!("{} \u{2014} {}", mod_name, warning.reason),
                        "points": -penalty,
                    }));
                }

                // 2. Check dependencies (missing masters)
                match mod_dependencies::check_dependency_issues(&db2, &gid2, &bn2) {
                    Ok(dep_issues) => {
                        let missing: Vec<_> = dep_issues
                            .iter()
                            .filter(|i| {
                                i.issue_type
                                    == mod_dependencies::DependencyIssueType::MissingRequirement
                            })
                            .collect();
                        for issue in &missing {
                            score -= 20;
                            issues.push(serde_json::json!({
                                "severity": "error",
                                "message": format!("{}: {}", issue.mod_name, issue.message),
                                "points": -20,
                            }));
                        }
                    }
                    Err(_) => {} // Skip if dependency check fails
                }

                // 3. Check conflicts (capped at -30)
                match db2.find_all_conflicts(&gid2, &bn2) {
                    Ok(conflicts) => {
                        if !conflicts.is_empty() {
                            let penalty = (conflicts.len() as i32 * 2).min(30);
                            score -= penalty;
                            issues.push(serde_json::json!({
                                "severity": "info",
                                "message": format!("{} file conflicts between mods", conflicts.len()),
                                "points": -penalty,
                            }));
                        }
                    }
                    Err(_) => {}
                }

                // 4. Check mod count sanity
                if enabled_count == 0 {
                    score -= 50;
                    issues.push(serde_json::json!({
                        "severity": "warning",
                        "message": "No mods are enabled",
                        "points": -50,
                    }));
                }

                // Clamp score
                let score = score.clamp(0, 100);
                let color = if score >= 80 {
                    "green"
                } else if score >= 50 {
                    "yellow"
                } else {
                    "red"
                };

                let color_emoji = match color {
                    "green" => "\u{1F7E2}",
                    "yellow" => "\u{1F7E1}",
                    _ => "\u{1F534}",
                };

                // Build text summary
                let mut text = format!("Mod Health Score: {}/100 {}\n", score, color_emoji);
                if !issues.is_empty() {
                    text.push_str("\nIssues:\n");
                    for issue in &issues {
                        let icon = match issue["severity"].as_str().unwrap_or("info") {
                            "error" => "\u{274C}",
                            "warning" => "\u{26A0}\u{FE0F}",
                            _ => "\u{2139}\u{FE0F}",
                        };
                        text.push_str(&format!(
                            "{} {} ({})\n",
                            icon,
                            issue["message"].as_str().unwrap_or(""),
                            issue["points"]
                        ));
                    }
                }
                let overall = match color {
                    "green" => "Your mod setup looks healthy!",
                    "yellow" => "Your mod setup has some issues that could be improved.",
                    _ => "Your mod setup has significant issues that should be addressed.",
                };
                text.push_str(&format!("\nOverall: {}", overall));

                (score, color.to_string(), issues, text)
            })
            .await
            .unwrap_or_else(|e| (0, "red".to_string(), vec![], format!("Error: {e}")));

            let (health_score, health_color, health_issues, text) = r;
            structured_data = Some(serde_json::json!({
                "type": "health_score",
                "score": health_score,
                "color": health_color,
                "issues": health_issues,
            }));
            (text, true)
        }

        _ => (format!("Unknown tool: {name}"), false),
    };

    let display_name = tool_result_display_name(name, &result);
    llm_chat::ToolResult {
        tool_name: name.into(),
        result,
        success,
        display_name,
        structured_data,
    }
}

/// Map game_id to NexusMods game slug.
fn nexus_game_slug(game_id: &str) -> String {
    match game_id {
        "skyrimse" | "skyrimspecialedition" => "skyrimspecialedition".into(),
        "skyrim" => "skyrim".into(),
        "fallout4" => "fallout4".into(),
        "starfield" => "starfield".into(),
        "oblivion" => "oblivion".into(),
        "morrowind" => "morrowind".into(),
        other => other.to_string(),
    }
}

/// Sync wrapper for mod recommendations (used from spawn_blocking).
fn get_mod_recommendations_sync(
    db: &std::sync::Arc<database::ModDatabase>,
    game_id: &str,
    bottle_name: &str,
    _mod_id: i64,
) -> Result<Vec<(String, i64, usize)>, String> {
    mod_recommendations::get_popular_mods(db, game_id, bottle_name)
}

// --- Wine Diagnostic Commands ---

#[tauri::command]
async fn run_wine_diagnostics(
    game_id: String,
    bottle_name: String,
) -> Result<wine_diagnostic::DiagnosticResult, String> {
    tokio::task::spawn_blocking(move || {
        let bottle = resolve_bottle(&bottle_name)?;
        Ok(wine_diagnostic::run_diagnostics(&bottle, &game_id))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn fix_wine_appdata(bottle_name: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let bottle = resolve_bottle(&bottle_name)?;
        wine_diagnostic::fix_appdata(&bottle).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn fix_wine_dll_override(
    bottle_name: String,
    dll_name: String,
    override_type: String,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let bottle = resolve_bottle(&bottle_name)?;
        wine_diagnostic::fix_dll_override(&bottle, &dll_name, &override_type)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn fix_wine_retina_mode(bottle_name: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let bottle = resolve_bottle(&bottle_name)?;
        wine_diagnostic::fix_retina_mode(&bottle).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// --- Pre-flight Commands ---

#[tauri::command]
async fn run_preflight_check(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<preflight::PreflightResult, String> {
    let (bottle, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        Ok(preflight::run_preflight(
            &db,
            &bottle,
            &game_id,
            &bottle_name,
            &data_dir,
        ))
    })
    .await
    .map_err(|e| format!("Preflight task failed: {e}"))?
}

// --- Mod Dependency Commands ---

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn add_mod_dependency(
    game_id: String,
    bottle_name: String,
    mod_id: i64,
    depends_on_id: Option<i64>,
    nexus_dep_id: Option<i64>,
    dep_name: String,
    relationship: String,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        mod_dependencies::add_dependency(
            &db,
            &game_id,
            &bottle_name,
            mod_id,
            depends_on_id,
            nexus_dep_id,
            &dep_name,
            &relationship,
        )
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn remove_mod_dependency(dep_id: i64, state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || mod_dependencies::remove_dependency(&db, dep_id))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn get_mod_dependencies(
    mod_id: i64,
    state: State<'_, AppState>,
) -> Result<Vec<mod_dependencies::ModDependency>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || mod_dependencies::get_dependencies(&db, mod_id))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn get_mod_dependents(
    mod_id: i64,
    state: State<'_, AppState>,
) -> Result<Vec<mod_dependencies::ModDependency>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || mod_dependencies::get_dependents(&db, mod_id))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn check_dependency_issues(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<mod_dependencies::DependencyIssue>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        mod_dependencies::check_dependency_issues(&db, &game_id, &bottle_name)
    })
    .await
    .map_err(|e| format!("Dependency check task failed: {e}"))?
}

// --- Mod Recommendation Commands ---

#[tauri::command]
async fn get_mod_recommendations(
    game_id: String,
    bottle_name: String,
    target_mod_id: i64,
    state: State<'_, AppState>,
) -> Result<mod_recommendations::RecommendationResult, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        mod_recommendations::get_recommendations(&db, &game_id, &bottle_name, target_mod_id)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn get_popular_mods(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<(String, i64, usize)>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        mod_recommendations::get_popular_mods(&db, &game_id, &bottle_name)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// --- Session Tracker Commands ---

#[tauri::command]
async fn start_game_session(
    game_id: String,
    bottle_name: String,
    profile_name: Option<String>,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        session_tracker::start_session(&db, &game_id, &bottle_name, profile_name.as_deref())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn end_game_session(
    session_id: i64,
    clean_exit: bool,
    crash_log_path: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        session_tracker::end_session(&db, session_id, clean_exit, crash_log_path.as_deref())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn record_session_mod_change(
    session_id: i64,
    mod_id: Option<i64>,
    mod_name: String,
    change_type: String,
    detail: Option<String>,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        session_tracker::record_mod_change(
            &db,
            session_id,
            mod_id,
            &mod_name,
            &change_type,
            detail.as_deref(),
        )
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn get_session_history(
    game_id: String,
    bottle_name: String,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<session_tracker::GameSession>, String> {
    let db = state.db.clone();
    let lim = limit.unwrap_or(20);
    tokio::task::spawn_blocking(move || {
        session_tracker::get_session_history(&db, &game_id, &bottle_name, lim)
    })
    .await
    .map_err(|e| format!("Session history task failed: {e}"))?
}

#[tauri::command]
async fn get_stability_summary(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<session_tracker::StabilitySummary, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        session_tracker::get_stability_summary(&db, &game_id, &bottle_name)
    })
    .await
    .map_err(|e| format!("Stability summary task failed: {e}"))?
}

// --- Game Lock Commands ---

#[tauri::command]
async fn get_game_lock_status(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Option<game_lock::GameLock>, String> {
    Ok(state.game_locks.get(&game_id, &bottle_name))
}

#[tauri::command]
async fn get_all_game_locks(
    state: State<'_, AppState>,
) -> Result<Vec<game_lock::GameLock>, String> {
    Ok(state.game_locks.all_locks())
}

#[tauri::command]
async fn force_unlock_game(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    Ok(state.game_locks.force_unlock(&game_id, &bottle_name))
}

// --- Deploy Journal Commands ---

#[tauri::command]
async fn get_deploy_journal_status() -> Result<Vec<deploy_journal::JournalEntry>, String> {
    Ok(deploy_journal::get_incomplete())
}

#[tauri::command]
async fn heal_deployment(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let bottle = bottles::find_bottle_by_name(&bottle_name)
            .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))?;
        let game = games::detect_games(&bottle)
            .into_iter()
            .find(|g| g.game_id == game_id)
            .ok_or_else(|| format!("Game '{}' not found in bottle '{}'", game_id, bottle_name))?;
        let data_dir = PathBuf::from(&game.data_dir);

        deployer::redeploy_all(&db, &game_id, &bottle_name, &data_dir, &game.game_path)
            .map_err(|e| format!("Heal redeploy failed: {e}"))?;

        log::info!("heal_deployment: redeployed {}/{}", game_id, bottle_name);
        Ok(())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// --- FOMOD Recipe Commands ---

#[tauri::command]
async fn save_fomod_recipe(
    mod_id: i64,
    mod_name: String,
    installer_hash: Option<String>,
    selections: std::collections::HashMap<String, Vec<String>>,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        fomod_recipes::save_recipe(
            &db,
            mod_id,
            &mod_name,
            installer_hash.as_deref(),
            &selections,
        )
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn get_fomod_recipe(
    mod_id: i64,
    state: State<'_, AppState>,
) -> Result<Option<fomod_recipes::FomodRecipe>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || fomod_recipes::get_recipe(&db, mod_id))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn list_fomod_recipes(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<fomod_recipes::FomodRecipe>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || fomod_recipes::list_recipes(&db, &game_id, &bottle_name))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn delete_fomod_recipe(mod_id: i64, state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || fomod_recipes::delete_recipe(&db, mod_id))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn has_compatible_fomod_recipe(
    mod_id: i64,
    current_hash: Option<String>,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        fomod_recipes::has_compatible_recipe(&db, mod_id, current_hash.as_deref())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
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
    let window = app.get_window("main").ok_or("Main window not found")?;

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
    let client = nexus_client().await?;
    let raw_files = client
        .get_mod_files(&game_slug, mod_id)
        .await
        .map_err(|e| e.to_string())?;

    Ok(nexus::parse_mod_files(&raw_files, mod_id))
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
    let client = nexus_client().await?;

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
    let dl_cfg = config::get_config().map_err(|e| e.to_string())?;
    let download_dir = dl_cfg
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
    let staging_result =
        staging::stage_mod(&archive_path, &game_id, &bottle_name, db_mod_id, &mod_name).map_err(
            |e| {
                let _ = db.remove_mod(db_mod_id);
                format!("Staging failed: {e}")
            },
        )?;

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
        let _ = sync_plugins_for_game(&game, &bottle);
    }

    // Auto-delete archive if setting enabled
    if dl_cfg
        .extra
        .get("auto_delete_archives")
        .and_then(|v: &serde_json::Value| v.as_str())
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

// --- Download Cache Check ---

#[tauri::command]
async fn check_cached_files(
    mod_file_pairs: Vec<(i64, i64)>,
    state: State<'_, AppState>,
) -> Result<Vec<(i64, i64)>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        db.batch_check_cached_files(&mod_file_pairs)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

// --- Steam Integration ---

#[tauri::command]
async fn detect_steam() -> Option<steam_integration::SteamInfo> {
    tokio::task::spawn_blocking(move || steam_integration::detect_steam_installation())
        .await
        .ok()
        .flatten()
}

#[tauri::command]
async fn check_steam_status() -> Result<steam_integration::SteamStatus, String> {
    tokio::task::spawn_blocking(move || steam_integration::get_steam_status())
        .await
        .map_err(|e| format!("Task failed: {e}"))
}

#[tauri::command]
async fn add_to_steam() -> Result<steam_integration::SteamStatus, String> {
    tokio::task::spawn_blocking(move || {
        steam_integration::setup_steam_integration().map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn remove_from_steam() -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let info = steam_integration::detect_steam_installation()
            .ok_or_else(|| "Steam not found".to_string())?;
        steam_integration::remove_from_steam(&info).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
fn is_steam_deck() -> bool {
    steam_integration::is_steam_deck()
}

#[tauri::command]
async fn steam_deck_warnings() -> Result<Vec<String>, String> {
    tokio::task::spawn_blocking(move || steam_integration::steam_deck_warnings())
        .await
        .map_err(|e| format!("Task failed: {e}"))
}

// --- Startup Cleanup ---

/// Mark orphaned Wabbajack installs (left in active state from a crash) as failed
/// and clean up their extraction temp directories.
fn cleanup_orphaned_wj_installs(db: &database::ModDatabase) {
    match db.get_stale_wj_installs() {
        Ok(stale) => {
            for (id, install_dir, status) in &stale {
                log::warn!(
                    "Found orphaned WJ install {} (status={}) — marking as failed",
                    id,
                    status
                );
                let _ = db.update_wj_install_status(
                    *id,
                    "failed",
                    Some("Interrupted by application exit"),
                );
                // Clean up extraction temp dir if it still exists
                let temp_dir = std::path::Path::new(install_dir).join(".wj_extraction_temp");
                if temp_dir.exists() {
                    log::info!("Removing orphaned extraction temp dir: {:?}", temp_dir);
                    let _ = std::fs::remove_dir_all(&temp_dir);
                }
            }
            if !stale.is_empty() {
                log::info!("Cleaned up {} orphaned WJ install(s)", stale.len());
            }
        }
        Err(e) => log::warn!("Failed to query stale WJ installs: {}", e),
    }
}

/// Remove any leftover `corkscrew_extract_*` directories from the system temp dir.
/// These are created by the collection installer and should be cleaned up on
/// completion, but may be orphaned if the app crashes during extraction.
fn cleanup_orphaned_temp_dirs() {
    let temp = std::env::temp_dir();
    match std::fs::read_dir(&temp) {
        Ok(entries) => {
            let mut cleaned = 0u32;
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if name.starts_with("corkscrew_extract_") && entry.path().is_dir() {
                        log::info!("Removing orphaned temp dir: {:?}", entry.path());
                        let _ = std::fs::remove_dir_all(entry.path());
                        cleaned += 1;
                    }
                }
            }
            if cleaned > 0 {
                log::info!("Cleaned up {} orphaned temp dir(s)", cleaned);
            }
        }
        Err(e) => log::warn!("Failed to scan temp dir for orphans: {}", e),
    }
}

// --- CLI diagnostic tools ---

/// List all installed mods for a game+bottle.
fn cli_list_mods(game_id: &str, bottle_name: &str, db: &Arc<ModDatabase>) {
    let mods = match db.list_mods(game_id, bottle_name) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("[corkscrew] ERROR: {}", e);
            std::process::exit(1);
        }
    };
    println!(
        "[corkscrew] {} mods installed for {}:{}",
        mods.len(),
        game_id,
        bottle_name
    );
    println!(
        "{:<8} {:<50} {:<10} {:<10} Staging",
        "ID", "Name", "Enabled", "Files"
    );
    println!("{}", "-".repeat(120));
    for m in &mods {
        let staging = m.staging_path.as_deref().unwrap_or("(inline)");
        println!(
            "{:<8} {:<50} {:<10} {:<10} {}",
            m.id,
            if m.name.len() > 48 {
                format!("{}…", &m.name[..47])
            } else {
                m.name.clone()
            },
            if m.enabled { "yes" } else { "NO" },
            m.installed_files.len(),
            staging,
        );
    }
}

/// Search installed mods by name (case-insensitive substring).
fn cli_search_mods(query: &str, game_id: &str, bottle_name: &str, db: &Arc<ModDatabase>) {
    let mods = match db.list_mods(game_id, bottle_name) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("[corkscrew] ERROR: {}", e);
            std::process::exit(1);
        }
    };
    let q = query.to_lowercase();
    let matches: Vec<_> = mods
        .iter()
        .filter(|m| m.name.to_lowercase().contains(&q))
        .collect();
    println!(
        "[corkscrew] {} match(es) for '{}' in {}:{}",
        matches.len(),
        query,
        game_id,
        bottle_name
    );
    for m in matches {
        let staging = m.staging_path.as_deref().unwrap_or("(inline)");
        println!(
            "  ID={} name='{}' enabled={} files={} nexus_id={:?}",
            m.id,
            m.name,
            m.enabled,
            m.installed_files.len(),
            m.nexus_mod_id
        );
        println!("    staging: {}", staging);
        if !m.installed_files.is_empty() {
            let plugins: Vec<_> = m
                .installed_files
                .iter()
                .filter(|f| {
                    let fl = f.to_lowercase();
                    fl.ends_with(".esp") || fl.ends_with(".esm") || fl.ends_with(".esl")
                })
                .collect();
            if !plugins.is_empty() {
                println!(
                    "    plugin files: {}",
                    plugins
                        .iter()
                        .map(|p| p.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
    }
}

/// Find files matching a pattern across all staging dirs for a game+bottle.
fn cli_find_file(pattern: &str, game_id: &str, bottle_name: &str, db: &Arc<ModDatabase>) {
    let mods = match db.list_mods(game_id, bottle_name) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("[corkscrew] ERROR: {}", e);
            std::process::exit(1);
        }
    };

    // Also load current plugins state so we can flag deployed-but-inactive plugins
    let plugin_active: std::collections::HashMap<String, bool> = {
        // Try to get game resolution for plugins.txt
        match resolve_game(game_id, bottle_name) {
            Ok((bottle, game, _)) => {
                let game_path = PathBuf::from(&game.game_path);
                let pf = games::with_plugin(game_id, |p| p.get_plugins_file(&game_path, &bottle))
                    .flatten();
                if let Some(pf) = pf {
                    plugins::skyrim_plugins::read_plugins_txt(&pf)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|e| (e.filename.to_lowercase(), e.enabled))
                        .collect()
                } else {
                    Default::default()
                }
            }
            Err(_) => Default::default(),
        }
    };

    let pat = pattern.to_lowercase();
    let mut found = 0usize;

    for m in &mods {
        // Check registered installed_files list
        let file_matches: Vec<_> = m
            .installed_files
            .iter()
            .filter(|f| f.to_lowercase().contains(&pat))
            .collect();

        if !file_matches.is_empty() {
            println!("  [mod {}] id={} enabled={}", m.name, m.id, m.enabled);
            for f in &file_matches {
                let fl = f.to_lowercase();
                let is_plugin =
                    fl.ends_with(".esp") || fl.ends_with(".esm") || fl.ends_with(".esl");
                let basename = f.rsplit(['/', '\\']).next().unwrap_or(f.as_str());
                let active_note = if is_plugin {
                    match plugin_active.get(&basename.to_lowercase()) {
                        Some(true) => " [plugin: ACTIVE ✓]",
                        Some(false) => " [plugin: INACTIVE ✗]",
                        None => " [plugin: not in plugins.txt]",
                    }
                } else {
                    ""
                };
                println!("    {}{}", f, active_note);
                found += 1;
            }
        }

        // Also walk staging dir if available (catches files not in DB list)
        if let Some(ref sp) = m.staging_path {
            let staging = PathBuf::from(sp);
            if staging.is_dir() {
                for entry in walkdir::WalkDir::new(&staging).into_iter().flatten() {
                    let name = entry.file_name().to_string_lossy().to_lowercase();
                    if name.contains(&pat) {
                        let rel = entry.path().strip_prefix(&staging).unwrap_or(entry.path());
                        // Only show if NOT already in installed_files
                        if !m
                            .installed_files
                            .iter()
                            .any(|f| f.to_lowercase().contains(&pat))
                        {
                            if found == 0 {
                                println!("  [mod {}] id={} (staged, not deployed)", m.name, m.id);
                            }
                            println!("    staging/{}", rel.display());
                            found += 1;
                        }
                    }
                }
            }
        }
    }

    println!("[corkscrew] find-file '{}': {} result(s)", pattern, found);
}

/// Show plugin load order state: active/inactive/on-disk/stale.
fn cli_check_plugins(
    game_id: &str,
    bottle_name: &str,
    inactive_only: bool,
    deployed_inactive_only: bool,
    db: &Arc<ModDatabase>,
) {
    let (bottle, game, data_dir) = match resolve_game(game_id, bottle_name) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[corkscrew] ERROR: {}", e);
            std::process::exit(1);
        }
    };

    let game_path = PathBuf::from(&game.game_path);
    let pf =
        match games::with_plugin(game_id, |p| p.get_plugins_file(&game_path, &bottle)).flatten() {
            Some(p) => p,
            None => {
                eprintln!("[corkscrew] No plugins.txt path for game '{}'", game_id);
                std::process::exit(1);
            }
        };

    println!("[corkscrew] plugins.txt: {}", pf.display());
    if let Ok(meta) = std::fs::metadata(&pf) {
        if let Ok(modified) = meta.modified() {
            let secs = modified
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            println!(
                "[corkscrew] plugins.txt last modified: {} (unix {})",
                {
                    let dt = chrono::DateTime::<chrono::Local>::from(modified);
                    dt.format("%Y-%m-%d %H:%M:%S").to_string()
                },
                secs
            );
        }
    }

    // Read plugins.txt
    let entries = match plugins::skyrim_plugins::read_plugins_txt(&pf) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[corkscrew] ERROR reading plugins.txt: {}", e);
            std::process::exit(1);
        }
    };

    // Discover on-disk plugins
    let on_disk = plugins::skyrim_plugins::discover_plugins(&data_dir).unwrap_or_default();
    let on_disk_lower: std::collections::HashSet<String> =
        on_disk.iter().map(|s| s.to_lowercase()).collect();

    // Build active set from plugins.txt
    let in_txt_active: std::collections::HashMap<String, bool> = entries
        .iter()
        .map(|e| (e.filename.to_lowercase(), e.enabled))
        .collect();

    // Build "which mod owns this plugin" map from DB
    let mods = db.list_mods(game_id, bottle_name).unwrap_or_default();
    let mut plugin_owner: std::collections::HashMap<String, String> = Default::default();
    for m in &mods {
        for f in &m.installed_files {
            let fl = f.to_lowercase();
            if fl.ends_with(".esp") || fl.ends_with(".esm") || fl.ends_with(".esl") {
                let basename = f.rsplit(['/', '\\']).next().unwrap_or(f.as_str());
                plugin_owner.insert(basename.to_lowercase(), m.name.clone());
            }
        }
    }

    let active_count = entries.iter().filter(|e| e.enabled).count();
    let inactive_count = entries.iter().filter(|e| !e.enabled).count();
    println!(
        "[corkscrew] plugins.txt: {} active, {} inactive, {} total entries",
        active_count,
        inactive_count,
        entries.len()
    );
    println!("[corkscrew] on disk: {} plugin files", on_disk.len());

    // Find deployed-but-inactive: on disk AND in plugins.txt but NOT active
    let mut deployed_inactive: Vec<&str> = Vec::new();
    for p in &on_disk {
        let key = p.to_lowercase();
        if let Some(&active) = in_txt_active.get(&key) {
            if !active {
                deployed_inactive.push(p.as_str());
            }
        }
    }

    // Find on-disk but not in plugins.txt at all
    let not_in_txt: Vec<&str> = on_disk
        .iter()
        .filter(|p| !in_txt_active.contains_key(&p.to_lowercase()))
        .map(|p| p.as_str())
        .collect();

    // Find in plugins.txt but not on disk (stale)
    let stale: Vec<_> = entries
        .iter()
        .filter(|e| !on_disk_lower.contains(&e.filename.to_lowercase()))
        .collect();

    if deployed_inactive_only {
        println!(
            "\n[DEPLOYED BUT INACTIVE in plugins.txt] ({} plugins):",
            deployed_inactive.len()
        );
        for p in &deployed_inactive {
            let owner = plugin_owner
                .get(&p.to_lowercase())
                .map(|s| s.as_str())
                .unwrap_or("unknown mod");
            println!("  {} ({})", p, owner);
        }
        return;
    }

    println!(
        "\n[DEPLOYED BUT INACTIVE in plugins.txt] ({} plugins):",
        deployed_inactive.len()
    );
    for p in &deployed_inactive {
        let owner = plugin_owner
            .get(&p.to_lowercase())
            .map(|s| s.as_str())
            .unwrap_or("unknown mod");
        println!("  {} ({})", p, owner);
    }

    println!(
        "\n[ON DISK BUT NOT IN plugins.txt] ({} plugins):",
        not_in_txt.len()
    );
    for p in not_in_txt.iter().take(20) {
        println!("  {}", p);
    }
    if not_in_txt.len() > 20 {
        println!("  ... and {} more", not_in_txt.len() - 20);
    }

    println!(
        "\n[STALE: in plugins.txt but NOT on disk] ({} plugins):",
        stale.len()
    );
    for e in stale.iter().take(20) {
        println!(
            "  {} ({})",
            e.filename,
            if e.enabled { "active" } else { "inactive" }
        );
    }
    if stale.len() > 20 {
        println!("  ... and {} more", stale.len() - 20);
    }

    if !inactive_only {
        println!(
            "\n[ALL INACTIVE in plugins.txt] ({} plugins, showing first 50):",
            inactive_count
        );
        let mut shown = 0;
        for e in entries.iter().filter(|e| !e.enabled) {
            let on_disk_flag = if on_disk_lower.contains(&e.filename.to_lowercase()) {
                " [on-disk]"
            } else {
                " [missing]"
            };
            let owner = plugin_owner
                .get(&e.filename.to_lowercase())
                .map(|s| format!(" ({})", s))
                .unwrap_or_default();
            println!("  {}{}{}", e.filename, on_disk_flag, owner);
            shown += 1;
            if shown >= 50 {
                println!("  ... and {} more inactive", inactive_count - shown);
                break;
            }
        }
    }
}

/// Manually run plugin sync for a game+bottle.
fn cli_sync_plugins(game_id: &str, bottle_name: &str) {
    let (bottle, game, _) = match resolve_game(game_id, bottle_name) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[corkscrew] ERROR: {}", e);
            std::process::exit(1);
        }
    };
    match sync_plugins_for_game(&game, &bottle) {
        Ok(()) => println!(
            "[corkscrew] Plugin sync complete for {}:{}",
            game_id, bottle_name
        ),
        Err(e) => {
            eprintln!("[corkscrew] ERROR: sync failed: {}", e);
            std::process::exit(1);
        }
    }
}

/// Show all files registered for a specific mod (by ID or name substring).
fn cli_mod_files(search: &str, game_id: &str, bottle_name: &str, db: &Arc<ModDatabase>) {
    let mods = match db.list_mods(game_id, bottle_name) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("[corkscrew] ERROR: {}", e);
            std::process::exit(1);
        }
    };
    let q = search.to_lowercase();
    let matched: Vec<_> = if let Ok(id) = search.parse::<i64>() {
        mods.iter().filter(|m| m.id == id).collect()
    } else {
        mods.iter()
            .filter(|m| m.name.to_lowercase().contains(&q))
            .collect()
    };
    if matched.is_empty() {
        println!("[corkscrew] No mods found matching '{}'", search);
        return;
    }
    for m in matched {
        println!(
            "[mod {}] id={} enabled={} nexus_id={:?}",
            m.name, m.id, m.enabled, m.nexus_mod_id
        );
        if let Some(sp) = &m.staging_path {
            println!("  staging: {}", sp);
        }
        println!("  registered files ({}):", m.installed_files.len());
        for f in &m.installed_files {
            println!("    {}", f);
        }
        // Also list staging dir if present and different
        if let Some(sp) = &m.staging_path {
            let staging = PathBuf::from(sp);
            if staging.is_dir() {
                let staged: Vec<_> = walkdir::WalkDir::new(&staging)
                    .into_iter()
                    .flatten()
                    .filter(|e| e.file_type().is_file())
                    .map(|e| {
                        e.path()
                            .strip_prefix(&staging)
                            .unwrap_or(e.path())
                            .to_path_buf()
                    })
                    .collect();
                println!("  staged files ({}):", staged.len());
                for f in &staged {
                    println!("    {}", f.display());
                }
            }
        }
    }
}

// --- CLI e2e test support commands ---

/// List detected bottles as JSON.
fn cli_list_bottles() {
    let bottles = crate::bottles::detect_bottles();
    let json: Vec<serde_json::Value> = bottles
        .iter()
        .map(|b| {
            serde_json::json!({
                "name": b.name,
                "path": b.path.display().to_string(),
                "engine": &b.source,
                "exists": b.exists(),
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&json).unwrap_or_default()
    );
}

/// List detected games as JSON (includes auto-detected Steam games and custom games).
fn cli_list_games(db: &Arc<ModDatabase>) {
    let bottles = crate::bottles::detect_bottles();
    let mut all_games: Vec<serde_json::Value> = Vec::new();
    for bottle in &bottles {
        let games = crate::games::detect_games(bottle);
        for game in &games {
            all_games.push(serde_json::json!({
                "id": game.game_id,
                "name": game.display_name,
                "bottle": bottle.name,
                "path": game.game_path.display().to_string(),
                "executable": game.exe_path.as_ref().map(|p| p.display().to_string()),
            }));
        }
    }
    // Include custom games from DB
    let custom = crate::game_registry::load_custom_games(db);
    for game in &custom {
        if !all_games.iter().any(|g| g["id"] == game.game_id) {
            all_games.push(serde_json::json!({
                "id": game.game_id,
                "name": game.display_name,
                "bottle": game.bottle_name,
                "path": game.game_path.display().to_string(),
                "executable": game.exe_path.as_ref().map(|p| p.display().to_string()),
                "custom": true,
            }));
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&all_games).unwrap_or_default()
    );
}

/// Add a custom game to the database.
fn cli_add_game(
    game_id: &str,
    name: &str,
    bottle_name: &str,
    game_path: &str,
    exe_name: Option<&str>,
    mod_dir: Option<&str>,
    nexus_slug: Option<&str>,
    steam_app_id: Option<&str>,
    db: &Arc<ModDatabase>,
) {
    use crate::game_registry::{save_custom_game, CustomGame};

    // Resolve the bottle to get its path
    let bottle = match crate::bottles::find_bottle_by_name(bottle_name) {
        Some(b) => b,
        None => {
            eprintln!("Error: Bottle '{}' not found", bottle_name);
            std::process::exit(1);
        }
    };

    // Resolve game path (could be absolute or relative to bottle's drive_c)
    let full_game_path = if std::path::Path::new(game_path).is_absolute() {
        std::path::PathBuf::from(game_path)
    } else {
        bottle.path.join("drive_c").join(game_path)
    };

    if !full_game_path.is_dir() {
        eprintln!(
            "Error: Game path '{}' does not exist",
            full_game_path.display()
        );
        std::process::exit(1);
    }

    // Find executable
    let exe_path = if let Some(exe) = exe_name {
        let p = full_game_path.join(exe);
        if !p.exists() {
            eprintln!("Warning: Executable '{}' not found at expected path", exe);
        }
        Some(p.display().to_string())
    } else {
        crate::game_registry::find_main_executable_public(&full_game_path)
            .map(|p| p.display().to_string())
    };

    let data_dir = if let Some(md) = mod_dir {
        full_game_path.join(md).display().to_string()
    } else {
        full_game_path.display().to_string()
    };

    let custom = CustomGame {
        game_id: game_id.to_string(),
        display_name: name.to_string(),
        nexus_slug: nexus_slug.unwrap_or(game_id).to_string(),
        game_path: full_game_path.display().to_string(),
        exe_path,
        data_dir,
        bottle_name: bottle.name.clone(),
        bottle_path: bottle.path.display().to_string(),
        steam_app_id: steam_app_id.map(|s| s.to_string()),
    };

    match save_custom_game(db, &custom) {
        Ok(()) => {
            let output = serde_json::json!({
                "ok": true,
                "game_id": custom.game_id,
                "display_name": custom.display_name,
                "game_path": custom.game_path,
                "exe_path": custom.exe_path,
                "data_dir": custom.data_dir,
                "bottle": custom.bottle_name,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&output).unwrap_or_default()
            );
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

/// Remove a custom game from the database.
fn cli_remove_game(game_id: &str, db: &Arc<ModDatabase>) {
    match crate::game_registry::remove_custom_game(db, game_id) {
        Ok(()) => println!("{{\"ok\":true,\"removed\":\"{}\"}}", game_id),
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

/// Database statistics as JSON.
fn cli_db_stats(game_id: &str, bottle_name: &str, db: &Arc<ModDatabase>) {
    let (total_mods, enabled_mods) = db.get_mod_counts(game_id, bottle_name).unwrap_or((0, 0));
    let disabled_mods = total_mods.saturating_sub(enabled_mods);
    let deployment_count = db.get_deployment_count(game_id, bottle_name).unwrap_or(0);

    let stats = serde_json::json!({
        "game_id": game_id,
        "bottle_name": bottle_name,
        "total_mods": total_mods,
        "enabled_mods": enabled_mods,
        "disabled_mods": disabled_mods,
        "deployed_files": deployment_count,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&stats).unwrap_or_default()
    );
}

/// SQLite integrity check.
fn cli_db_integrity(db: &Arc<ModDatabase>) {
    let conn = match db.conn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{{\"ok\":false,\"error\":\"{}\"}}", e);
            std::process::exit(1);
        }
    };

    // PRAGMA integrity_check
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap_or_else(|e| format!("error: {}", e));

    // Count tables
    let table_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    // List tables
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap();
    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    // Check schema version
    let schema_version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap_or(0);

    let ok = integrity == "ok";
    let result = serde_json::json!({
        "ok": ok,
        "integrity_check": integrity,
        "table_count": table_count,
        "tables": tables,
        "schema_version": schema_version,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&result).unwrap_or_default()
    );
    if !ok {
        std::process::exit(1);
    }
}

/// List cached Vortex extensions as JSON.
fn cli_vortex_list(db: &Arc<ModDatabase>) {
    let summaries = crate::vortex_registry::list_cached(db);
    let json: Vec<serde_json::Value> = summaries
        .iter()
        .map(|s| {
            serde_json::json!({
                "game_id": s.game_id,
                "name": s.name,
                "is_stub": s.is_stub,
                "fetched_at": s.fetched_at,
                "tool_count": s.tool_count,
                "mod_type_count": s.mod_type_count,
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&json).unwrap_or_default()
    );
}

/// Check deployment health — verify staged files exist on disk.
fn cli_deployment_health(game_id: &str, bottle_name: &str, db: &Arc<ModDatabase>) {
    let mods = match db.list_mods(game_id, bottle_name) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{{\"ok\":false,\"error\":\"{}\"}}", e);
            std::process::exit(1);
        }
    };

    let mut total_mods = 0;
    let mut mods_with_staging = 0;
    let mut staging_exists = 0;
    let mut staging_missing = 0;
    let mut missing_dirs: Vec<String> = Vec::new();

    for m in &mods {
        total_mods += 1;
        if let Some(ref path) = m.staging_path {
            mods_with_staging += 1;
            if std::path::Path::new(path).exists() {
                staging_exists += 1;
            } else {
                staging_missing += 1;
                if missing_dirs.len() < 20 {
                    missing_dirs.push(format!("{}:{}", m.id, m.name));
                }
            }
        }
    }

    let ok = staging_missing == 0;
    let result = serde_json::json!({
        "ok": ok,
        "total_mods": total_mods,
        "mods_with_staging": mods_with_staging,
        "staging_exists": staging_exists,
        "staging_missing": staging_missing,
        "missing_examples": missing_dirs,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&result).unwrap_or_default()
    );
    if !ok {
        std::process::exit(1);
    }
}

/// List profiles as JSON.
fn cli_list_profiles(game_id: &str, bottle_name: &str, db: &Arc<ModDatabase>) {
    let conn = match db.conn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{{\"error\":\"{}\"}}", e);
            std::process::exit(1);
        }
    };
    let mut stmt = match conn.prepare(
        "SELECT id, name, is_active FROM profiles WHERE game_id = ?1 AND bottle_name = ?2 ORDER BY name",
    ) {
        Ok(s) => s,
        Err(_) => {
            // profiles table may not exist
            println!("[]");
            return;
        }
    };
    let profiles: Vec<serde_json::Value> = stmt
        .query_map(rusqlite::params![game_id, bottle_name], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, i64>(0)?,
                "name": row.get::<_, String>(1)?,
                "is_active": row.get::<_, i32>(2)? != 0,
            }))
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&profiles).unwrap_or_default()
    );
}

/// Fetch + execute a Vortex extension and report results as JSON.
fn cli_vortex_test(game_id: &str) {
    if let Err(e) = vortex_fetcher::validate_game_id(game_id) {
        eprintln!("{{\"ok\":false,\"error\":\"{}\"}}", e);
        std::process::exit(1);
    }

    // Use a tokio runtime for the async fetch
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let source = match rt.block_on(vortex_fetcher::fetch_extension(game_id)) {
        Ok(s) => s,
        Err(e) => {
            let result = serde_json::json!({
                "ok": false,
                "phase": "fetch",
                "error": e,
            });
            println!("{}", serde_json::to_string_pretty(&result).unwrap());
            std::process::exit(1);
        }
    };

    println!(
        "[vortex-test] Fetched {}: {} bytes index.js, hash={}",
        game_id,
        source.index_js.len(),
        source.source_hash
    );

    // Execute in QuickJS
    let captured = match vortex_runtime::execute_extension(&source) {
        Ok(c) => c,
        Err(e) => {
            let result = serde_json::json!({
                "ok": false,
                "phase": "execute",
                "error": e,
                "source_bytes": source.index_js.len(),
            });
            println!("{}", serde_json::to_string_pretty(&result).unwrap());
            std::process::exit(1);
        }
    };

    // Report what was captured
    let game = captured.game.as_ref();
    let result = serde_json::json!({
        "ok": game.is_some(),
        "game": game.map(|g| serde_json::json!({
            "id": g.id,
            "name": g.name,
            "executable": g.executable,
            "query_mod_path": g.query_mod_path,
            "merge_mods": g.merge_mods,
            "required_files": g.required_files,
            "is_stub": g.is_stub,
            "steam_app_id": g.store_ids.steam_app_id,
            "gog_app_id": g.store_ids.gog_app_id,
            "epic_app_id": g.store_ids.epic_app_id,
            "xbox_id": g.store_ids.xbox_id,
            "steam_dir_name": g.steam_dir_name,
            "tool_count": g.supported_tools.len(),
            "tools": g.supported_tools.iter().map(|t| serde_json::json!({
                "id": t.id,
                "name": t.name,
                "executable": t.executable,
            })).collect::<Vec<_>>(),
        })),
        "mod_types": captured.mod_types.iter().map(|mt| serde_json::json!({
            "id": mt.id,
            "priority": mt.priority,
            "target_path": mt.target_path,
        })).collect::<Vec<serde_json::Value>>(),
        "installers": captured.installers.iter().map(|i| serde_json::json!({
            "id": i.id,
            "priority": i.priority,
        })).collect::<Vec<serde_json::Value>>(),
    });

    println!("{}", serde_json::to_string_pretty(&result).unwrap());
}

// --- CLI headless launch ---

/// Run the full pre-launch pipeline and spawn the game without opening the UI.
///
/// Usage:  corkscrew --launch <game_id> <bottle_name> [--skse]
/// Example: corkscrew --launch skyrimse Steam --skse
fn cli_launch(game_id: &str, bottle_name: &str, use_skse: bool, db: &Arc<ModDatabase>) {
    println!(
        "[corkscrew] --launch mode: game={} bottle={} skse={}",
        game_id, bottle_name, use_skse
    );

    let (bottle, game, _) = match resolve_game(game_id, bottle_name) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[corkscrew] ERROR: {}", e);
            std::process::exit(1);
        }
    };
    let game_path = PathBuf::from(&game.game_path);

    let exe_name = if use_skse && game_id == "skyrimse" {
        "skse64_loader.exe".to_string()
    } else {
        games::with_plugin(game_id, |plugin| {
            plugin
                .executables()
                .first()
                .map(|s| s.to_string())
                .unwrap_or_default()
        })
        .unwrap_or_default()
    };

    if exe_name.is_empty() {
        eprintln!(
            "[corkscrew] ERROR: No executable configured for game '{}'",
            game_id
        );
        std::process::exit(1);
    }

    let exe_path = match launcher::find_executable(&game_path, &exe_name) {
        Some(p) => p,
        None => {
            eprintln!(
                "[corkscrew] ERROR: {} not found in {}",
                exe_name,
                game_path.display()
            );
            std::process::exit(1);
        }
    };

    let fixes_disabled = config::get_config_value("disable_game_fixes")
        .unwrap_or(None)
        .map(|v| v == "true")
        .unwrap_or(false);

    if game_id == "skyrimse" && !fixes_disabled {
        match display_fix::auto_fix_display(&bottle) {
            Ok(r) => {
                if r.fixed {
                    println!(
                        "[corkscrew] Display fix applied: {}x{} fullscreen",
                        r.applied.width, r.applied.height
                    );
                }
            }
            Err(e) => eprintln!("[corkscrew] Warning: display fix failed: {}", e),
        }
    }

    let _ = sync_plugins_for_game(&game, &bottle);

    if game_id == "skyrimse" {
        let data_dir = PathBuf::from(&game.data_dir);

        let fixes =
            skse::fix_skse_plugin_conflicts(db, game_id, bottle_name, &data_dir, &game_path);
        if fixes > 0 {
            println!("[corkscrew] Fixed {} SKSE plugin DLL(s)", fixes);
        }

        let ef = skse::fix_engine_fixes_for_wine(&data_dir, db, game_id, bottle_name);
        if ef > 0 {
            println!("[corkscrew] Patched {} EngineFixes TOML(s) for Wine", ef);
        }

        let wine_disabled =
            skse::disable_wine_incompatible_plugins(&data_dir, db, game_id, bottle_name);
        for (name, _reason) in &wine_disabled {
            println!("[corkscrew] Disabled Wine-incompatible plugin: {}", name);
        }

        match skse::install_engine_fixes_wine_blocking(&data_dir) {
            Ok(true) => println!("[corkscrew] Deployed SSE Engine Fixes for Wine"),
            Ok(false) => println!("[corkscrew] SSE Engine Fixes for Wine already up to date"),
            Err(e) => eprintln!("[corkscrew] Warning: Engine Fixes deploy failed: {}", e),
        }
    }

    println!("[corkscrew] Launching {} ...", exe_path.display());
    match launcher::launch_game(&bottle, &exe_path, Some(&game_path)) {
        Ok(r) => {
            println!("[corkscrew] Launched OK (pid={:?})", r.pid);
            if let Some(w) = r.warning {
                eprintln!("[corkscrew] Warning: {}", w);
            }
        }
        Err(e) => {
            eprintln!("[corkscrew] ERROR: Launch failed: {}", e);
            std::process::exit(1);
        }
    }
}

// --- Vortex Extension Commands ---

#[tauri::command]
async fn vortex_fetch_extension(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<vortex_types::VortexGameRegistration, String> {
    let db = state.db.clone();
    vortex_registry::fetch_and_register(&db, &game_id).await
}

#[tauri::command]
async fn vortex_refresh_extension(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<vortex_types::VortexGameRegistration, String> {
    let db = state.db.clone();
    vortex_registry::refresh_extension(&db, &game_id).await
}

#[tauri::command]
async fn vortex_list_cached_extensions(
    state: State<'_, AppState>,
) -> Result<Vec<vortex_types::ExtensionSummary>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || Ok(vortex_registry::list_cached(&db)))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn vortex_list_available_extensions() -> Result<Vec<String>, String> {
    vortex_fetcher::list_available_extensions().await
}

#[tauri::command]
async fn vortex_delete_cached_extension(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        vortex_registry::delete_cached(&db, &game_id);
        Ok(())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
async fn vortex_get_extension_detail(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<Option<vortex_types::VortexGameRegistration>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || Ok(vortex_registry::load_cached(&db, &game_id)))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

// --- App Entry Point ---

fn kill_mlx_server() {
    let _ = std::process::Command::new("pkill")
        .args(["-f", "mlx_lm.server"])
        .output();
    log::info!("Killed MLX LM server if running");
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Kill any leftover MLX server from a previous crash/dev restart
    kill_mlx_server();
    // Register game plugins (dedicated plugins first, then registry)
    plugins::skyrim_se::register();
    plugins::fallout4::register();
    game_registry::register_all();

    // Initialize database
    let db_path = config::db_path();
    let db = Arc::new(ModDatabase::new(&db_path).expect("Failed to initialize mod database"));

    // Register any previously-cached Vortex extensions as game plugins
    vortex_registry::register_all_cached(&db);

    // Initialize additional schemas
    executables::init_schema(&db).expect("Failed to initialize executables schema");
    profiles::init_schema(&db).expect("Failed to initialize profiles schema");
    integrity::init_schema(&db).expect("Failed to initialize integrity schema");
    loot_rules::init_schema(&db).expect("Failed to initialize loot rules schema");
    rollback::init_schema(&db).expect("Failed to initialize rollback schema");

    // Set up logging: write to both stderr and a log file for GUI debugging.
    let log_path = config::data_dir().join("corkscrew.log");
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path);

    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    builder
        .filter_module("tao", log::LevelFilter::Warn)
        .filter_module("wry", log::LevelFilter::Warn);

    if let Ok(file) = log_file {
        // Truncate log if over 10 MB to avoid unbounded growth
        if let Ok(meta) = std::fs::metadata(&log_path) {
            if meta.len() > 10 * 1024 * 1024 {
                let _ = std::fs::write(&log_path, b"");
            }
        }
        let file = std::sync::Mutex::new(file);
        builder
            .format(move |buf, record| {
                use std::io::Write;
                let ts = buf.timestamp_seconds();
                let line = format!(
                    "{} [{}] {}: {}\n",
                    ts,
                    record.level(),
                    record.target(),
                    record.args()
                );
                // Write to stderr (normal env_logger behavior)
                let _ = write!(buf, "{}", line);
                // Also write to log file
                if let Ok(mut f) = file.lock() {
                    let _ = std::io::Write::write_all(&mut *f, line.as_bytes());
                    let _ = std::io::Write::flush(&mut *f);
                }
                Ok(())
            })
            .init();
        log::info!("Logging to file: {}", log_path.display());
    } else {
        builder.init();
    }

    // --- CLI mode ---
    // Subcommands dispatched here. Each exits after completion.
    {
        let args: Vec<String> = std::env::args().collect();

        // --launch <game_id> <bottle_name> [--skse]
        if let Some(pos) = args.iter().position(|a| a == "--launch") {
            let game_id = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            let bottle_name = args.get(pos + 2).map(|s| s.as_str()).unwrap_or("");
            let use_skse = args.iter().any(|a| a == "--skse");
            if game_id.is_empty() || bottle_name.is_empty() {
                eprintln!("Usage: corkscrew --launch <game_id> <bottle_name> [--skse]");
                eprintln!("  Example: corkscrew --launch skyrimse Steam --skse");
                std::process::exit(1);
            }
            cli_launch(game_id, bottle_name, use_skse, &db);
            return;
        }

        // --list-mods <game_id> <bottle_name>
        if let Some(pos) = args.iter().position(|a| a == "--list-mods") {
            let game_id = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            let bottle_name = args.get(pos + 2).map(|s| s.as_str()).unwrap_or("");
            if game_id.is_empty() || bottle_name.is_empty() {
                eprintln!("Usage: corkscrew --list-mods <game_id> <bottle_name>");
                std::process::exit(1);
            }
            cli_list_mods(game_id, bottle_name, &db);
            return;
        }

        // --search-mods <query> <game_id> <bottle_name>
        if let Some(pos) = args.iter().position(|a| a == "--search-mods") {
            let query = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            let game_id = args.get(pos + 2).map(|s| s.as_str()).unwrap_or("");
            let bottle_name = args.get(pos + 3).map(|s| s.as_str()).unwrap_or("");
            if query.is_empty() || game_id.is_empty() || bottle_name.is_empty() {
                eprintln!("Usage: corkscrew --search-mods <query> <game_id> <bottle_name>");
                std::process::exit(1);
            }
            cli_search_mods(query, game_id, bottle_name, &db);
            return;
        }

        // --find-file <pattern> <game_id> <bottle_name>
        if let Some(pos) = args.iter().position(|a| a == "--find-file") {
            let pattern = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            let game_id = args.get(pos + 2).map(|s| s.as_str()).unwrap_or("");
            let bottle_name = args.get(pos + 3).map(|s| s.as_str()).unwrap_or("");
            if pattern.is_empty() || game_id.is_empty() || bottle_name.is_empty() {
                eprintln!("Usage: corkscrew --find-file <pattern> <game_id> <bottle_name>");
                std::process::exit(1);
            }
            cli_find_file(pattern, game_id, bottle_name, &db);
            return;
        }

        // --check-plugins <game_id> <bottle_name> [--inactive-only] [--deployed-inactive]
        if let Some(pos) = args.iter().position(|a| a == "--check-plugins") {
            let game_id = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            let bottle_name = args.get(pos + 2).map(|s| s.as_str()).unwrap_or("");
            if game_id.is_empty() || bottle_name.is_empty() {
                eprintln!("Usage: corkscrew --check-plugins <game_id> <bottle_name> [--inactive-only] [--deployed-inactive]");
                std::process::exit(1);
            }
            let inactive_only = args.iter().any(|a| a == "--inactive-only");
            let deployed_inactive = args.iter().any(|a| a == "--deployed-inactive");
            cli_check_plugins(game_id, bottle_name, inactive_only, deployed_inactive, &db);
            return;
        }

        // --sync-plugins <game_id> <bottle_name>
        if let Some(pos) = args.iter().position(|a| a == "--sync-plugins") {
            let game_id = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            let bottle_name = args.get(pos + 2).map(|s| s.as_str()).unwrap_or("");
            if game_id.is_empty() || bottle_name.is_empty() {
                eprintln!("Usage: corkscrew --sync-plugins <game_id> <bottle_name>");
                std::process::exit(1);
            }
            cli_sync_plugins(game_id, bottle_name);
            return;
        }

        // --mod-files <mod_id_or_name> <game_id> <bottle_name>
        if let Some(pos) = args.iter().position(|a| a == "--mod-files") {
            let search = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            let game_id = args.get(pos + 2).map(|s| s.as_str()).unwrap_or("");
            let bottle_name = args.get(pos + 3).map(|s| s.as_str()).unwrap_or("");
            if search.is_empty() || game_id.is_empty() || bottle_name.is_empty() {
                eprintln!("Usage: corkscrew --mod-files <mod_id_or_name> <game_id> <bottle_name>");
                std::process::exit(1);
            }
            cli_mod_files(search, game_id, bottle_name, &db);
            return;
        }

        // --list-bottles  (JSON array of detected bottles)
        if args.iter().any(|a| a == "--list-bottles") {
            cli_list_bottles();
            return;
        }

        // --list-games  (JSON array of detected games across all bottles)
        if args.iter().any(|a| a == "--list-games") {
            cli_list_games(&db);
            return;
        }

        // --db-stats <game_id> <bottle_name>  (JSON object with database statistics)
        if let Some(pos) = args.iter().position(|a| a == "--db-stats") {
            let game_id = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            let bottle_name = args.get(pos + 2).map(|s| s.as_str()).unwrap_or("");
            if game_id.is_empty() || bottle_name.is_empty() {
                eprintln!("Usage: corkscrew --db-stats <game_id> <bottle_name>");
                std::process::exit(1);
            }
            cli_db_stats(game_id, bottle_name, &db);
            return;
        }

        // --db-integrity  (run SQLite integrity check + schema validation)
        if args.iter().any(|a| a == "--db-integrity") {
            cli_db_integrity(&db);
            return;
        }

        // --vortex-list  (JSON array of cached vortex extensions)
        if args.iter().any(|a| a == "--vortex-list") {
            cli_vortex_list(&db);
            return;
        }

        // --deployment-health <game_id> <bottle_name>  (check deployment integrity)
        if let Some(pos) = args.iter().position(|a| a == "--deployment-health") {
            let game_id = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            let bottle_name = args.get(pos + 2).map(|s| s.as_str()).unwrap_or("");
            if game_id.is_empty() || bottle_name.is_empty() {
                eprintln!("Usage: corkscrew --deployment-health <game_id> <bottle_name>");
                std::process::exit(1);
            }
            cli_deployment_health(game_id, bottle_name, &db);
            return;
        }

        // --list-profiles <game_id> <bottle_name>  (JSON array of profiles)
        if let Some(pos) = args.iter().position(|a| a == "--list-profiles") {
            let game_id = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            let bottle_name = args.get(pos + 2).map(|s| s.as_str()).unwrap_or("");
            if game_id.is_empty() || bottle_name.is_empty() {
                eprintln!("Usage: corkscrew --list-profiles <game_id> <bottle_name>");
                std::process::exit(1);
            }
            cli_list_profiles(game_id, bottle_name, &db);
            return;
        }

        // --add-game <game_id> <name> <bottle> <path> [--exe <name>] [--mod-dir <dir>] [--nexus <slug>] [--steam-id <id>]
        if let Some(pos) = args.iter().position(|a| a == "--add-game") {
            let game_id = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            let name = args.get(pos + 2).map(|s| s.as_str()).unwrap_or("");
            let bottle = args.get(pos + 3).map(|s| s.as_str()).unwrap_or("");
            let path = args.get(pos + 4).map(|s| s.as_str()).unwrap_or("");
            if game_id.is_empty() || name.is_empty() || bottle.is_empty() || path.is_empty() {
                eprintln!("Usage: corkscrew --add-game <game_id> <name> <bottle> <path>");
                eprintln!("  Options: --exe <name> --mod-dir <dir> --nexus <slug> --steam-id <id>");
                eprintln!("  Example: corkscrew --add-game re-requiem \"Resident Evil Requiem\" Steam \"Program Files (x86)/Steam/steamapps/common/RESIDENT EVIL requiem BIOHAZARD requiem\" --exe re9.exe");
                std::process::exit(1);
            }
            let exe = args
                .iter()
                .position(|a| a == "--exe")
                .and_then(|i| args.get(i + 1))
                .map(|s| s.as_str());
            let mod_dir = args
                .iter()
                .position(|a| a == "--mod-dir")
                .and_then(|i| args.get(i + 1))
                .map(|s| s.as_str());
            let nexus = args
                .iter()
                .position(|a| a == "--nexus")
                .and_then(|i| args.get(i + 1))
                .map(|s| s.as_str());
            let steam_id = args
                .iter()
                .position(|a| a == "--steam-id")
                .and_then(|i| args.get(i + 1))
                .map(|s| s.as_str());
            cli_add_game(
                game_id, name, bottle, path, exe, mod_dir, nexus, steam_id, &db,
            );
            return;
        }

        // --remove-game <game_id>
        if let Some(pos) = args.iter().position(|a| a == "--remove-game") {
            let game_id = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            if game_id.is_empty() {
                eprintln!("Usage: corkscrew --remove-game <game_id>");
                std::process::exit(1);
            }
            cli_remove_game(game_id, &db);
            return;
        }

        // --vortex-test <game_id>  (fetch + execute a Vortex extension, report results as JSON)
        if let Some(pos) = args.iter().position(|a| a == "--vortex-test") {
            let game_id = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            if game_id.is_empty() {
                eprintln!("Usage: corkscrew --vortex-test <game_id>");
                eprintln!("  Example: corkscrew --vortex-test skyrimse");
                std::process::exit(1);
            }
            cli_vortex_test(game_id);
            return;
        }

        // --version  (print version and exit)
        if args.iter().any(|a| a == "--version") {
            println!("{}", env!("CARGO_PKG_VERSION"));
            return;
        }

        // --help  (print usage and exit)
        if args.iter().any(|a| a == "--help" || a == "-h") {
            println!("Corkscrew — Mod Manager for Wine/CrossOver/Proton");
            println!("Version: {}", env!("CARGO_PKG_VERSION"));
            println!();
            println!("USAGE:");
            println!("  corkscrew                                    Launch GUI");
            println!("  corkscrew --launch <game> <bottle> [--skse]  Launch game headless");
            println!("  corkscrew --list-mods <game> <bottle>        List installed mods");
            println!("  corkscrew --search-mods <q> <game> <bottle>  Search mods by name");
            println!("  corkscrew --find-file <pat> <game> <bottle>  Find file across mods");
            println!("  corkscrew --check-plugins <game> <bottle>    Analyze plugin state");
            println!("  corkscrew --sync-plugins <game> <bottle>     Sync plugin state");
            println!("  corkscrew --mod-files <id> <game> <bottle>   Show mod's files");
            println!("  corkscrew --list-bottles                     List detected bottles (JSON)");
            println!("  corkscrew --list-games                       List detected games (JSON)");
            println!("  corkscrew --db-stats <game> <bottle>         Database statistics (JSON)");
            println!("  corkscrew --db-integrity                     SQLite integrity check");
            println!("  corkscrew --vortex-list                      List cached Vortex extensions (JSON)");
            println!("  corkscrew --deployment-health <game> <bottle> Check deployment integrity");
            println!("  corkscrew --list-profiles <game> <bottle>    List profiles (JSON)");
            println!("  corkscrew --add-game <id> <name> <bottle> <path>  Add custom game");
            println!("  corkscrew --remove-game <id>                 Remove custom game");
            println!("  corkscrew --version                          Print version");
            return;
        }
    }

    // Recover Dock if a previous session crashed while cursor fix was active
    cursor_clamp::recover_dock_if_needed();

    // Clean up orphaned Wabbajack installs from previous crash/forced quit
    cleanup_orphaned_wj_installs(&db);

    // Clean up stale corkscrew_extract_* temp dirs from collection installs
    cleanup_orphaned_temp_dirs();

    // Replay any incomplete deployment journal entries (self-healing)
    {
        let healed = deploy_journal::replay_incomplete(&db);
        if !healed.is_empty() {
            log::info!(
                "Self-healed {} deployment(s) from interrupted operations: {:?}",
                healed.len(),
                healed
            );
        }
    }

    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_liquid_glass::init())
        .setup(|app| {
            // Register updater plugin in setup per Tauri docs (advanced pattern)
            app.handle()
                .plugin(tauri_plugin_updater::Builder::new().build())?;
            app.manage(app_updates::PendingUpdate(std::sync::Mutex::new(None)));
            Ok(())
        });

    #[cfg(debug_assertions)]
    {
        builder = builder.plugin(tauri_plugin_mcp::init_with_config(
            tauri_plugin_mcp::PluginConfig::new("Corkscrew".to_string())
                .start_socket_server(true)
                .socket_path("/tmp/corkscrew-mcp.sock".into()),
        ));
    }

    builder
        .manage({
            let queue = download_queue::DownloadQueue::new();
            // Restore persisted queue items from database
            match db.load_queue_items() {
                Ok(items) => {
                    if !items.is_empty() {
                        log::info!(
                            "Restored {} download queue items from database",
                            items.len()
                        );
                        queue.load_from(items);
                    }
                }
                Err(e) => log::warn!("Failed to load download queue from database: {}", e),
            }
            AppState {
                db: Arc::clone(&db),
                download_queue: Arc::new(queue),
                wj_cancel_tokens: std::sync::Mutex::new(std::collections::HashMap::new()),
                fomod_cache: Arc::new(fomod::new_fomod_cache()),
                loot_masterlist_checked: Arc::new(AtomicBool::new(false)),
                chat_session: llm_chat::create_shared_session(),
                game_locks: Arc::new(game_lock::GameLockManager::new()),
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
            get_installed_mods_summary,
            get_mod_detail,
            install_mod_cmd,
            uninstall_mod,
            toggle_mod,
            batch_toggle_mods,
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
            uninstall_skse_cmd,
            set_skse_preference_cmd,
            check_skyrim_version,
            check_skse_compatibility_cmd,
            scan_skse_plugins_cmd,
            fix_skse_plugins_cmd,
            list_disabled_wine_plugins_cmd,
            reenable_wine_plugin_cmd,
            fix_skyrim_display,
            downgrade_skyrim,
            get_depot_download_command,
            start_depot_download,
            check_depot_ready,
            apply_downgrade_cmd,
            list_game_versions,
            swap_game_version,
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
            deploy_incremental_cmd,
            check_deployment_health,
            get_verification_level,
            set_verification_level,
            purge_deployment_cmd,
            verify_mod_integrity,
            sort_plugins_loot,
            update_loot_masterlist,
            force_refresh_loot_masterlist,
            reorder_plugins_cmd,
            toggle_plugin_cmd,
            move_plugin_cmd,
            get_plugin_messages,
            list_profiles_cmd,
            create_profile_cmd,
            delete_profile_cmd,
            deactivate_profile_cmd,
            rename_profile_cmd,
            save_profile_snapshot,
            activate_profile,
            get_profile_save_info,
            backup_profile_saves,
            restore_profile_saves,
            check_mod_updates,
            detect_mod_tools_cmd,
            install_mod_tool,
            uninstall_mod_tool,
            launch_mod_tool,
            reinstall_mod_tool,
            check_mod_tool_update,
            apply_tool_ini_edits_cmd,
            detect_collection_tools,
            detect_wabbajack_tools,
            get_platform_detail,
            get_optimal_download_threads,
            detect_fomod,
            get_fomod_defaults,
            get_fomod_files,
            check_dlc_status,
            create_game_snapshot,
            check_game_integrity,
            has_game_snapshot,
            scan_game_directory,
            clean_game_directory,
            get_wabbajack_modlists,
            parse_wabbajack_file,
            download_wabbajack_file,
            // Wabbajack Install Pipeline
            wabbajack_installer::install_wabbajack_modlist_cmd,
            wabbajack_installer::cancel_wabbajack_install,
            wabbajack_installer::resume_wabbajack_install,
            wabbajack_installer::cleanup_wabbajack_install,
            wabbajack_installer::get_wabbajack_install_status,
            wabbajack_installer::wabbajack_preflight_cmd,
            // Nexus SSO
            start_nexus_sso,
            // OAuth
            start_oauth_login,
            start_nexus_oauth,
            refresh_nexus_tokens,
            save_oauth_tokens,
            load_oauth_tokens,
            clear_oauth_tokens,
            get_nexus_user_info,
            get_auth_method_cmd,
            get_nexus_account_status,
            // Google OAuth (Gemini)
            google_sign_in,
            google_sign_out,
            google_auth_status,
            // Crash Logs
            find_crash_logs_cmd,
            analyze_crash_log_cmd,
            chat_check_new_crashes,
            // Utility
            fetch_url_text,
            // Collections & Nexus Browse
            browse_nexus_mods_cmd,
            get_nexus_mod_detail,
            endorse_mod,
            abstain_mod,
            get_user_endorsements,
            search_nexus_mods_cmd,
            get_game_categories_cmd,
            browse_collections_cmd,
            get_collection_cmd,
            get_collection_revisions,
            get_collection_mods,
            parse_collection_bundle_cmd,
            install_collection_cmd,
            cancel_collection_install_cmd,
            submit_fomod_choices,
            get_incomplete_collection_installs,
            get_all_interrupted_installs,
            get_checkpoint_mod_names,
            resume_collection_install_cmd,
            abandon_collection_install,
            get_pending_wabbajack_installs,
            get_pinned_game_version,
            pin_game_version,
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
            restore_mod_snapshot,
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
            uninstall_wabbajack_modlist,
            return_to_vanilla,
            collection_download_size_cmd,
            get_collection_diff_cmd,
            get_deployment_health,
            get_deployment_stats,
            sync_plugins_cmd,
            // Background Hashing
            start_background_hashing,
            cancel_background_hashing,
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
            read_mod_file,
            write_mod_file,
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
            get_mod_dependents,
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
            // Game Lock
            get_game_lock_status,
            get_all_game_locks,
            force_unlock_game,
            // Deploy Journal
            get_deploy_journal_status,
            heal_deployment,
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
            // Download Cache
            check_cached_files,
            // Steam Integration
            detect_steam,
            check_steam_status,
            add_to_steam,
            remove_from_steam,
            is_steam_deck,
            steam_deck_warnings,
            // Instruction parsing (LLM)
            parse_instructions_cmd,
            parse_instructions_llm_cmd,
            parse_instructions_cloud_cmd,
            validate_instruction_actions_cmd,
            check_ollama_status_cmd,
            get_recommended_models,
            get_cloud_providers,
            pull_ollama_model_cmd,
            delete_ollama_model_cmd,
            unload_ollama_model_cmd,
            get_system_memory,
            install_ollama,
            start_ollama,
            check_mlx_status,
            install_mlx,
            get_recommended_model,
            chat_get_state,
            chat_get_starters,
            chat_load_model,
            chat_unload_model,
            get_cached_mlx_models,
            delete_model,
            chat_send_message,
            chat_clear_history,
            chat_get_history,
            chat_validate_cloud_key,
            vortex_fetch_extension,
            vortex_refresh_extension,
            vortex_list_cached_extensions,
            vortex_list_available_extensions,
            vortex_delete_cached_extension,
            vortex_get_extension_detail,
            // Self-Update (macOS fallback)
            self_update::get_installed_app_version,
            self_update::manual_self_update,
            // App Updates (advanced Rust-side updater per Tauri docs)
            app_updates::fetch_update,
            app_updates::install_update,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| match event {
            tauri::RunEvent::Exit | tauri::RunEvent::ExitRequested { .. } => {
                kill_mlx_server();
            }
            _ => {}
        });
}
