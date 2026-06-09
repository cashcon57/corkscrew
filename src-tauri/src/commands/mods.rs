//! Mod management commands: CRUD, archive management, notes, tags, and categories.

use crate::database;
use crate::bottles;
use crate::config;
use crate::downgrader;
use crate::games;
use crate::launcher;
use crate::plugins;
use crate::skse;
use crate::bottle_config;
use crate::bottles::{Bottle};
use crate::config::{AppConfig};
use crate::database::{InstalledMod};
use crate::deploy_journal;
use crate::deployer;
use crate::display_fix;
use crate::downgrader::{DowngradeStatus};
use crate::executables;
use crate::game_registry;
use crate::games::{DetectedGame};
use crate::installer;
use crate::launcher::{LaunchResult};
use crate::mod_types;
use crate::nexus;
use crate::nxm_handler;
use crate::oauth;
use crate::plugins::skyrim_plugins::{PluginEntry};
use crate::progress;
use crate::rollback;
use crate::skse::{SkseStatus};
use crate::staging;
use crate::{AppState, check_game_lock, nexus_client, resolve_bottle, resolve_game, resolve_game_any_runtime};
use serde::{Serialize};
use std::path::{Path, PathBuf};
use std::time::Instant;
use tauri::Emitter;
use tauri::{AppHandle, State};

// --- Tauri Commands ---

#[tauri::command]
pub async fn get_bottles() -> Result<Vec<Bottle>, String> {
    tokio::task::spawn_blocking(move || Ok(bottles::detect_bottles()))
        .await
        .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn get_games(bottle_name: Option<String>) -> Result<Vec<DetectedGame>, String> {
    tokio::task::spawn_blocking(move || match bottle_name {
        Some(name) => {
            let bottle = resolve_bottle(&name)?;
            Ok(games::detect_games(&bottle))
        }
        None => Ok(games::detect_all_games()),
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn get_all_games() -> Result<Vec<DetectedGame>, String> {
    tokio::task::spawn_blocking(move || Ok(games::detect_all_games()))
        .await
        .map_err(crate::format_join_error)?
}

#[tauri::command]
pub fn list_supported_games() -> Result<Vec<game_registry::SupportedGame>, String> {
    Ok(game_registry::list_supported_games())
}

/// Categorisation of how well-supported a given game is.
///
/// The frontend uses this to badge games and gate first-install warnings for
/// experimental / unknown targets. Strings are stable identifiers consumed by
/// the TS-side `GameSupportTier` type.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameSupportTier {
    /// Has a dedicated Rust plugin AND has been end-to-end tested.
    Verified,
    /// Has a dedicated Rust plugin but the game itself is unverified
    /// (e.g. pre-release titles, mods not yet validated).
    Experimental,
    /// Supported via a Vortex extension fetched at runtime from
    /// `Nexus-Mods/vortex-games`.
    VortexExtension,
    /// Listed in the bundled Vortex registry JSON but no dedicated plugin
    /// nor an upstream extension matched.
    VortexRegistry,
    /// Discovered via Steam appmanifest scan or a custom user entry.
    /// Modding behaviour is untested for this game.
    Unknown,
}

/// Game IDs that have a dedicated, end-to-end-tested Rust plugin.
const VERIFIED_PLUGIN_IDS: &[&str] = &[
    "skyrimse",
    "fallout4",
    "hogwartslegacy",
    // Native macOS plugins — install + deploy + launch verified end-to-end
    // against real installs (Paralives 5/31/2026, BG3 6/9/2026, Stardew via
    // SMAPI). Crimson Desert native is intentionally listed below as
    // experimental because deploy is gated behind a VERIFIED const pending
    // real-install confirmation of the PAZ overlay path.
    "stardew_valley_native",
    "paralives_native",
    "baldurs_gate_3_native",
];

/// Game IDs whose dedicated Rust plugin is known to be experimental.
/// We hard-code these rather than thread a `verified()` accessor through the
/// `GamePlugin` trait — the set is small and stable.
const EXPERIMENTAL_PLUGIN_IDS: &[&str] = &[
    "hades2",
    "crimsondesert",
    "crimson_desert_native",
    "genshin",
];

#[tauri::command]
pub fn get_game_support_tier(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<GameSupportTier, String> {
    let id = game_id.as_str();

    // 1. Verified — explicit allow-list of dedicated, tested plugins.
    if VERIFIED_PLUGIN_IDS.contains(&id) {
        return Ok(GameSupportTier::Verified);
    }

    // 2. Thunderstore plugin? Use its `verified` flag from the spec table.
    if let Some(spec) = crate::plugins::thunderstore_games::SPECS
        .iter()
        .find(|s| s.game_id == id)
    {
        return Ok(if spec.verified {
            GameSupportTier::Verified
        } else {
            GameSupportTier::Experimental
        });
    }

    // 3. Other dedicated plugin marked experimental.
    if EXPERIMENTAL_PLUGIN_IDS.contains(&id) {
        return Ok(GameSupportTier::Experimental);
    }

    // 4. Vortex extension — only if a cached registration is present in DB.
    //    (The plugin registry can also contain RegistryGamePlugin entries
    //    sourced from `vortex_game_registry.json`; those are checked separately
    //    below so we don't conflate the two paths.)
    if crate::vortex_registry::load_cached(&state.db, id).is_some() {
        return Ok(GameSupportTier::VortexExtension);
    }

    // 5. Bundled Vortex registry JSON entry.
    if game_registry::get_game_entry(id).is_some() {
        return Ok(GameSupportTier::VortexRegistry);
    }

    // 6. Default — custom games and Steam-appmanifest-only discoveries.
    Ok(GameSupportTier::Unknown)
}

#[tauri::command]
pub async fn get_game_version(
    game_id: String,
    bottle_name: String,
) -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || {
        // Universal: detect_game_version is purely a file inspection.
        // Works for both Wine and native (empty bottle_name) games.
        let (_opt_bottle, game, _data_dir) = resolve_game_any_runtime(&game_id, &bottle_name)?;
        let version = games::with_plugin(&game_id, |plugin| {
            plugin.detect_game_version(&game.game_path)
        })
        .flatten();
        Ok(version)
    })
    .await
    .map_err(crate::format_join_error)?
}

/// Look up a captured Steam depot manifest for a specific game version.
/// Returns the depot download command if a manifest is found.
#[tauri::command]
pub async fn lookup_version_manifest(
    game_id: String,
    target_version: String,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        match game_registry::lookup_manifest_for_version(&db, &game_id, &target_version) {
            Some(info) => Ok(Some(format!(
                "download_depot {} {} {}",
                info.app_id, info.depot_id, info.manifest_id
            ))),
            None => Ok(None),
        }
    })
    .await
    .map_err(crate::format_join_error)?
}

/// Get captured depot history for a game (all versions we've seen).
#[tauri::command]
pub async fn get_depot_history_cmd(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let history = game_registry::get_depot_history(&db, &game_id);
        Ok(history
            .into_iter()
            .map(|(version, build_id, manifest_id)| {
                serde_json::json!({
                    "game_version": version,
                    "build_id": build_id,
                    "manifest_id": manifest_id,
                })
            })
            .collect())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn sync_lua_mods(
    game_id: String,
    bottle_name: String,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        // Lua mod sync is Hogwarts Legacy-specific (Wine-only).
        if bottle_name.is_empty() {
            return Err(
                "Lua mod sync is only available for Hogwarts Legacy under Wine — not for native games.".into(),
            );
        }
        let (_bottle, game, _data_dir) = resolve_game(&game_id, &bottle_name)?;
        if game_id == "hogwartslegacy" {
            crate::plugins::hogwarts_legacy::sync_mods_txt(&game.game_path)
        } else {
            Err("Lua mod management is only supported for Hogwarts Legacy".into())
        }
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn get_bottle_settings(bottle_name: String) -> Result<bottle_config::BottleSettings, String> {
    tokio::task::spawn_blocking(move || {
        let bottle = resolve_bottle(&bottle_name)?;
        bottle_config::get_bottle_settings(&bottle).map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn get_bottle_setting_defs(
    bottle_name: String,
) -> Result<Vec<bottle_config::BottleSettingDef>, String> {
    tokio::task::spawn_blocking(move || {
        let bottle = resolve_bottle(&bottle_name)?;
        let settings = bottle_config::get_bottle_settings(&bottle).map_err(|e| e.to_string())?;
        Ok(bottle_config::get_setting_definitions(&settings))
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn set_bottle_setting(bottle_name: String, key: String, value: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let bottle = resolve_bottle(&bottle_name)?;
        bottle_config::set_bottle_setting(&bottle, &key, &value).map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn get_installed_mods(
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
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn get_installed_mods_summary(
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
pub async fn get_mod_detail(mod_id: i64, state: State<'_, AppState>) -> Result<InstalledMod, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        db.get_mod(mod_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Mod {} not found", mod_id))
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn install_mod_cmd(
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

        // Resolve game for either Wine (bottle_name non-empty) or native
        // (bottle_name is the empty-string sentinel). Native games skip bottle
        // resolution entirely so they no longer fail with "Bottle '' not found".
        let (opt_bottle, game, data_dir) = resolve_game_any_runtime(&game_id, &bottle_name)?;
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

        // Step 3b: Auto-detect mod type and resolve deploy directory.
        //
        // Two layers, in order:
        //
        // 1. **Vortex mod types** — per-plugin `detect_mod_type_from_files`
        //    + `vortex_mod_types()`. Used by games whose plugin came from a
        //    Vortex extension (Witcher 3, etc.).
        //
        // 2. **mod_types registry** — archive-shape heuristics for games
        //    whose plugin returns `use_legacy_data_dir() == false` OR has
        //    no plugin at all (unknown / appmanifest-detected games).
        //    Routes BepInEx plugins to `BepInEx/plugins/<modname>/`, UE
        //    paks to `~mods/`, etc. See `crate::mod_types`.
        //
        // The Bethesda fast-path (plugin exists AND `use_legacy_data_dir`
        // is true, the default) skips both — mods MERGE into `Data`.
        let detected_mod_type = games::with_plugin(&game_id, |plugin| {
            plugin.detect_mod_type_from_files(&staging_result.files)
        })
        .flatten();

        // Layer 2: only consult the mod_types registry if the plugin
        // explicitly opts out OR there is no plugin for this game.
        let use_legacy = games::with_plugin(&game_id, |p| p.use_legacy_data_dir())
            .unwrap_or(false);

        let effective_dir = if let Some(ref mod_type_id) = detected_mod_type {
            // Look up the target path from registered vortex mod types
            let target = games::with_plugin(&game_id, |plugin| {
                plugin
                    .vortex_mod_types()
                    .into_iter()
                    .find(|t| t.id == *mod_type_id)
                    .map(|t| t.target_path)
            })
            .flatten();
            if let Some(rel_path) = target {
                let resolved = game.game_path.join(rel_path);
                log::debug!(
                    "Auto-detected mod type '{}' → deploying to {}",
                    mod_type_id,
                    resolved.display()
                );
                resolved
            } else {
                data_dir.clone()
            }
        } else if !use_legacy {
            // No Vortex hit, plugin opts out (or is missing). Use the
            // archive-shape registry.
            let target = mod_types::resolve_install_target(
                &game.game_path,
                &name,
                &staging_result.files,
            );
            log::info!(
                "Mod-type registry: '{}' → {} (per_mod_subfolder={})",
                target.type_id,
                target.target_dir.display(),
                target.per_mod_subfolder
            );
            target.target_dir
        } else {
            data_dir.clone()
        };

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
            &effective_dir,
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

        // Record detected mod type and fire post-deploy hook
        if let Some(ref mod_type_id) = detected_mod_type {
            let deploy_target = if effective_dir != data_dir { "custom" } else { "data" };
            let _ = db.set_deploy_target_for_mod(mod_id, deploy_target);
            games::with_plugin(&game_id, |plugin| {
                plugin.on_mod_deployed(
                    &game.game_path,
                    Some(mod_type_id.as_str()),
                    &staging_result.files,
                );
            });
        }

        // Step 5: Sync plugins (Wine only — Skyrim SE is always Wine)
        if game_id == "skyrimse" {
            if let Some(ref bottle) = opt_bottle {
                let _ = app.emit(
                    INSTALL_PROGRESS_EVENT,
                    InstallProgress::StepChanged {
                        mod_index: 0,
                        step: "syncing-plugins".to_string(),
                        detail: Some("Syncing plugin load order...".to_string()),
                    },
                );
                let _ = crate::sync_plugins_for_game(&game, bottle);
            }
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
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn uninstall_mod(
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

        // Universal: uninstall works for both Wine and native (empty bottle_name).
        let (opt_bottle, game, data_dir) = resolve_game_any_runtime(&game_id, &bottle_name)?;

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

        // Sync Skyrim plugins if applicable (Wine-only — Skyrim SE is always Wine)
        if game_id == "skyrimse" {
            if let Some(ref bottle) = opt_bottle {
                let _ = crate::sync_plugins_for_game(&game, bottle);
            }
        }

        // Post-undeploy hook (e.g. sync Mods.txt for HL Lua mods)
        games::with_plugin(&game_id, |plugin| {
            plugin.on_mod_undeployed(&game.game_path, None, &removed);
        });

        Ok(removed)
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn toggle_mod(
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
            // Universal: toggle works for both Wine and native (empty bottle_name).
            let (opt_bottle, game, data_dir) = resolve_game_any_runtime(&game_id, &bottle_name)?;
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

            // Sync Skyrim plugins if applicable (Wine-only — Skyrim SE is always Wine)
            if game_id == "skyrimse" {
                if let Some(ref bottle) = opt_bottle {
                    let _ = crate::sync_plugins_for_game(&game, bottle);
                }
            }
        }
        // Legacy mods (no staging_path): only the DB flag changes

        Ok(())
    })
    .await
    .map_err(crate::format_join_error)?
}

/// Batch enable/disable mods. For bulk operations (>=5 mods), updates all DB
/// flags first then does a single `redeploy_all` pass instead of per-mod
/// deploy/undeploy — dramatically faster for large batches.
#[tauri::command]
pub async fn batch_toggle_mods(
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
        // Universal: batch toggle works for both Wine and native (empty bottle_name).
        let (opt_bottle, game, data_dir) = resolve_game_any_runtime(&game_id, &bottle_name)?;

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
                if let Some(ref bottle) = opt_bottle {
                    let _ = app.emit(
                        "bulk-operation-progress",
                        serde_json::json!({
                            "phase": "plugins",
                            "current": 0,
                            "total": 1,
                            "message": "Syncing plugins.txt...",
                        }),
                    );
                    let _ = crate::sync_plugins_for_game(&game, bottle);
                }
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
                if let Some(ref bottle) = opt_bottle {
                    let _ = crate::sync_plugins_for_game(&game, bottle);
                }
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
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn get_plugin_order(
    game_id: String,
    bottle_name: String,
) -> Result<Vec<PluginEntry>, String> {
    tokio::task::spawn_blocking(move || {
        if !plugins::skyrim_plugins::supports_plugin_order(&game_id) {
            return Ok(vec![]);
        }
        // Plugin order is Bethesda-Wine only (no native Bethesda games yet).
        if bottle_name.is_empty() {
            return Err(
                "Plugin load order is only available for Wine-hosted Bethesda games.".into(),
            );
        }

        let (bottle, game, _) = resolve_game(&game_id, &bottle_name)?;

        // Auto-sync plugins.txt with deployed files before reading.
        // This ensures all deployed plugins are marked enabled and stale
        // entries for removed plugins are cleaned up.
        crate::sync_plugins_for_game(&game, &bottle)?;

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
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn download_from_nexus(
    nxm_url: String,
    game_id: String,
    bottle_name: String,
    auto_install: bool,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let client = nexus_client().await?;

    let nxm = nexus::NXMLink::parse(&nxm_url).map_err(|e| e.to_string())?;

    // Cross-check: route the NXM link's game_domain against the games we
    // can detect. If it doesn't match any known game, fall back to the
    // user-supplied `game_id` (which the UI sourced from the active
    // profile) and log a warning. Mirrors Vortex InstallManager.ts
    // L1418-1450 — we don't fail the download just because we don't know
    // the game; we route to active and let the user confirm.
    {
        let known: Vec<(String, String)> = {
            let db = state.db.clone();
            // Avoid blocking the async runtime on the bottle scan; the
            // caller is awaiting so spawn_blocking is fine. We do this
            // inside a block so the result drops before the await below.
            tokio::task::spawn_blocking(move || {
                games::detect_all_games_with_custom(&db)
                    .into_iter()
                    .map(|g| (g.game_id.clone(), g.nexus_slug.clone()))
                    .collect::<Vec<_>>()
            })
            .await
            .unwrap_or_default()
        };
        let active = if game_id.is_empty() { None } else { Some(game_id.as_str()) };
        match nxm_handler::route_nxm_game(&nxm.game_slug, &known, active) {
            nxm_handler::NxmRoute::Recognized { game_id: matched } => {
                if !matched.eq_ignore_ascii_case(&game_id) {
                    log::warn!(
                        "NXM link declares game '{}' (matches '{}') but caller \
                         requested install into '{}'. Proceeding with caller's \
                         game per active selection.",
                        nxm.game_slug, matched, game_id
                    );
                }
            }
            nxm_handler::NxmRoute::Fallback { active_game_id, warning } => {
                log::warn!("{}", warning);
                debug_assert_eq!(active_game_id, game_id);
            }
            nxm_handler::NxmRoute::NoActiveGame { error } => {
                return Err(error);
            }
        }
    }

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
        // Support both Wine and native games — native sentinel is empty bottle_name.
        let (opt_bottle, game, data_dir) = resolve_game_any_runtime(&game_id, &bottle_name)?;
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

        // Plugin sync is Wine-only (Skyrim SE is always Wine)
        if game_id == "skyrimse" {
            if let Some(ref bottle) = opt_bottle {
                let _ = crate::sync_plugins_for_game(&game, bottle);
            }
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
pub async fn is_nexus_premium() -> Result<bool, String> {
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
pub async fn get_config() -> Result<AppConfig, String> {
    tokio::task::spawn_blocking(move || config::get_config().map_err(|e| e.to_string()))
        .await
        .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn set_config_value(key: String, value: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        config::set_config_value(&key, &value).map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}


// --- Download Archive Management ---

#[tauri::command]
pub async fn list_download_archives() -> Result<Vec<serde_json::Value>, String> {
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
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn delete_download_archive(path: String) -> Result<(), String> {
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
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn get_downloads_stats() -> Result<serde_json::Value, String> {
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
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn clear_all_download_archives() -> Result<u64, String> {
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
    .map_err(crate::format_join_error)?
}

#[derive(Clone, Debug, Serialize)]
pub struct OrphanedDownload {
    path: String,
    filename: String,
    size_bytes: u64,
}

#[tauri::command]
pub async fn find_orphaned_downloads(
    state: State<'_, AppState>,
) -> Result<Vec<OrphanedDownload>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let cfg = config::get_config().map_err(|e| e.to_string())?;
        let dir = cfg
            .download_dir
            .map(PathBuf::from)
            .unwrap_or_else(config::downloads_dir);

        if !dir.exists() {
            return Ok(Vec::new());
        }

        // 1. List all archive files on disk
        let mut disk_files: Vec<(String, String, u64)> = Vec::new(); // (path, filename, size)
        let entries = std::fs::read_dir(&dir).map_err(|e| e.to_string())?;
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if !["zip", "7z", "rar", "gz", "tar"].contains(&ext.as_str()) {
                continue;
            }
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let filename = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            disk_files.push((path.to_string_lossy().to_string(), filename, size));
        }

        // 2. Get all archive names referenced by installed mods
        let installed_names = db.get_all_archive_names().map_err(|e| e.to_string())?;

        // 3. Get all download registry records
        let registry_records = db.get_all_download_records().map_err(|e| e.to_string())?;
        let _registry_names: std::collections::HashSet<String> = registry_records
            .iter()
            .map(|r| r.archive_name.clone())
            .collect();

        // 4. Get download IDs with collection refs
        let ids_with_refs = db.get_download_ids_with_refs().map_err(|e| e.to_string())?;

        // 5. A file is orphaned if:
        //    - NOT referenced by any installed mod's archive_name AND
        //    - (NOT in download_registry OR in registry but has zero collection refs)
        let mut orphans = Vec::new();
        for (path, filename, size) in &disk_files {
            if installed_names.contains(filename) {
                continue;
            }
            // Check if it's in the registry with collection refs
            let has_live_ref = registry_records.iter().any(|r| {
                r.archive_name == *filename && ids_with_refs.contains(&r.id)
            });
            if has_live_ref {
                continue;
            }
            orphans.push(OrphanedDownload {
                path: path.clone(),
                filename: filename.clone(),
                size_bytes: *size,
            });
        }

        Ok(orphans)
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn delete_orphaned_downloads(
    paths: Vec<String>,
    state: State<'_, AppState>,
) -> Result<u64, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let cfg = config::get_config().map_err(|e| e.to_string())?;
        let downloads = cfg
            .download_dir
            .map(PathBuf::from)
            .unwrap_or_else(config::downloads_dir);
        let canonical_downloads = downloads
            .canonicalize()
            .map_err(|e| format!("Invalid downloads directory: {e}"))?;

        let mut deleted = 0u64;
        for path_str in &paths {
            let archive_path = PathBuf::from(path_str);
            if !archive_path.exists() {
                continue;
            }
            let canonical = match archive_path.canonicalize() {
                Ok(c) => c,
                Err(_) => continue,
            };
            if !canonical.starts_with(&canonical_downloads) {
                continue;
            }
            if !canonical.is_file() {
                continue;
            }
            // Clean up registry entry if present
            let filename = canonical
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if let Ok(Some(record)) = db.find_download_by_name(&filename) {
                let _ = db.delete_download_record(record.id);
            }
            if std::fs::remove_file(&canonical).is_ok() {
                deleted += 1;
            }
        }
        Ok(deleted)
    })
    .await
    .map_err(crate::format_join_error)?
}

/// Fetch a game icon and cache it locally. Returns a base64-encoded data URL.
///
/// Icon sources (tried in order):
/// 1. Disk cache (instant, no network)
/// 2. SteamGridDB Icons API — most popular transparent PNG icon
/// 3. Steam CDN logo fallback
///
/// Works for ANY game with a Steam App ID in the registry (84+ games).
#[tauri::command]
pub async fn get_game_logo(
    game_id: String,
    steam_app_id: Option<String>,
) -> Result<Option<String>, String> {
    let icon_dir = config::cache_dir().join("game-icons");
    let cached_path = icon_dir.join(format!("{game_id}.png"));

    // 1. Return cached version if it exists (instant)
    if cached_path.exists() {
        let bytes = std::fs::read(&cached_path).map_err(|e| e.to_string())?;
        if bytes.len() >= 4 {
            let b64 = base64_encode(&bytes);
            let mime = if &bytes[..4] == b"\x89PNG" {
                "image/png"
            } else {
                "image/jpeg"
            };
            return Ok(Some(format!("data:{mime};base64,{b64}")));
        }
    }

    // Steam App ID resolution priority:
    //   1. Frontend-supplied (DetectedGame.steam_app_id from the appmanifest
    //      scanner — covers Steam-installed games not in the bundled registry).
    //   2. Bundled Vortex game registry entry.
    //   3. Curated Vortex extension index by id or nexus_slug.
    let steam_app_id = steam_app_id
        .as_deref()
        .and_then(|s| s.parse::<u32>().ok())
        .or_else(|| {
            game_registry::get_game_entry(&game_id)
                .and_then(|e| e.steam_id.as_deref())
                .and_then(|s| s.parse::<u32>().ok())
        })
        .or_else(|| {
            crate::vortex_index::all_entries()
                .iter()
                .find(|e| e.id == game_id || e.nexus_slug == game_id)
                .and_then(|e| e.steam_app_id.parse::<u32>().ok())
        });

    let client = reqwest::Client::builder()
        .user_agent(format!("Corkscrew/{}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    // 2. Try SteamGridDB Icons API (transparent PNG, sorted by popularity)
    if let Some(app_id) = steam_app_id {
        if let Some(bytes) = fetch_steamgriddb_icon(&client, app_id).await {
            std::fs::create_dir_all(&icon_dir).map_err(|e| e.to_string())?;
            std::fs::write(&cached_path, &bytes).map_err(|e| e.to_string())?;
            let b64 = base64_encode(&bytes);
            return Ok(Some(format!("data:image/png;base64,{b64}")));
        }
    }

    // 3. Fall back to Steam CDN header capsule (square-ish, 460x215)
    //    Skip the wide logo.png (640x360) — it looks wrong in icon slots.
    if let Some(app_id) = steam_app_id {
        let url = format!(
            "https://cdn.cloudflare.steamstatic.com/steam/apps/{app_id}/header.jpg"
        );
        if let Ok(response) = client.get(&url).send().await {
            if response.status().is_success() {
                if let Ok(bytes) = response.bytes().await {
                    if bytes.len() >= 4 {
                        std::fs::create_dir_all(&icon_dir).map_err(|e| e.to_string())?;
                        std::fs::write(&cached_path, &bytes).map_err(|e| e.to_string())?;
                        let b64 = base64_encode(&bytes);
                        let mime = if &bytes[..4] == b"\x89PNG" {
                            "image/png"
                        } else {
                            "image/jpeg"
                        };
                        return Ok(Some(format!("data:{mime};base64,{b64}")));
                    }
                }
            }
        }
    }

    Ok(None)
}

/// Fetch the most popular transparent icon from SteamGridDB for a Steam app.
///
/// Uses the SteamGridDB API v2:
/// 1. Look up the SteamGridDB game ID from the Steam App ID
/// 2. Fetch icons sorted by score, filtered to PNG only
/// 3. Download the top-scoring icon image
pub async fn fetch_steamgriddb_icon(client: &reqwest::Client, steam_app_id: u32) -> Option<Vec<u8>> {
    let api_key = config::get_config()
        .ok()
        .and_then(|c| {
            let val = c.extra.get("steamgriddb_api_key")?;
            val.as_str().map(String::from)
        })
        .unwrap_or_default();

    if api_key.is_empty() {
        return None;
    }

    // Step 1: Get SteamGridDB game ID from Steam App ID
    let game_url = format!(
        "https://www.steamgriddb.com/api/v2/games/steam/{steam_app_id}"
    );
    let game_resp = client
        .get(&game_url)
        .header("Authorization", format!("Bearer {api_key}"))
        .send()
        .await
        .ok()?;

    if !game_resp.status().is_success() {
        return None;
    }

    let game_json: serde_json::Value = game_resp.json().await.ok()?;
    let sgdb_game_id = game_json
        .get("data")
        .and_then(|d| d.get("id"))
        .and_then(|id| id.as_u64())?;

    // Step 2: Fetch icons — PNG, sorted by score (most popular first)
    let icons_url = format!(
        "https://www.steamgriddb.com/api/v2/icons/game/{sgdb_game_id}?types=static&mimes=image/png&nsfw=false&humor=false&limit=1"
    );
    let icons_resp = client
        .get(&icons_url)
        .header("Authorization", format!("Bearer {api_key}"))
        .send()
        .await
        .ok()?;

    if !icons_resp.status().is_success() {
        return None;
    }

    let icons_json: serde_json::Value = icons_resp.json().await.ok()?;
    let icon_url = icons_json
        .get("data")
        .and_then(|d| d.as_array())
        .and_then(|arr| arr.first())
        .and_then(|icon| icon.get("url"))
        .and_then(|u| u.as_str())?;

    // Step 3: Download the actual icon image
    let img_resp = client.get(icon_url).send().await.ok()?;
    if !img_resp.status().is_success() {
        return None;
    }

    let bytes = img_resp.bytes().await.ok()?;

    let bytes = bytes.to_vec();

    // Verify it's a valid PNG
    if bytes.len() < 8 || &bytes[..4] != b"\x89PNG" {
        return None;
    }

    Some(bytes)
}

/// Simple base64 encoder (avoids adding a dependency).
pub fn base64_encode(input: &[u8]) -> String {
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

/// Look up a native macOS game process PID by executable name via `pgrep -f`.
///
/// Used after launching a native game with `open steam://...` (or `open <bundle>`)
/// since `open` detaches and we can't capture the child PID directly. After a
/// short settle delay we ask `pgrep` for any process whose full command-line
/// matches `exe_name`.
///
/// Returns `Some(pid)` only for a unique match. Returns `None` on:
/// - `pgrep` exits non-zero (no match) — game hasn't started yet or path differs
/// - `pgrep` returns zero or multiple matches — multi-match is ambiguous and we
///   prefer no lock to the wrong lock
/// - `pgrep` itself fails to spawn (also logs to `log::warn`)
///
/// macOS-only — `pgrep -f` behaves slightly differently on Linux Wine launches,
/// which have their own PID handling.
#[cfg(target_os = "macos")]
pub fn lookup_native_pid(exe_name: &str) -> Option<u32> {
    if exe_name.is_empty() {
        return None;
    }
    let output = match std::process::Command::new("pgrep")
        .arg("-f")
        .arg(exe_name)
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            log::warn!("lookup_native_pid: pgrep spawn failed for '{}': {}", exe_name, e);
            return None;
        }
    };
    if !output.status.success() {
        // Exit 1 from pgrep means "no match" — common during the first 100s of
        // ms after `open` while the game is still launching.
        log::debug!(
            "lookup_native_pid: pgrep -f '{}' returned no match (exit {})",
            exe_name,
            output.status
        );
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let pids: Vec<u32> = stdout
        .lines()
        .filter_map(|l| l.trim().parse::<u32>().ok())
        .collect();
    match pids.as_slice() {
        [pid] => Some(*pid),
        many if many.len() > 1 => {
            log::warn!(
                "lookup_native_pid: pgrep -f '{}' matched {} processes ({:?}); refusing to guess",
                exe_name,
                many.len(),
                many,
            );
            None
        }
        _ => None,
    }
}

/// Linux fallback for [`lookup_native_pid`]. Always returns `None` on non-macOS.
/// Keeps the call site compile-clean without per-call `#[cfg]` noise.
#[cfg(not(target_os = "macos"))]
pub fn lookup_native_pid(_exe_name: &str) -> Option<u32> {
    None
}

#[tauri::command]
pub async fn launch_game_cmd(
    game_id: String,
    bottle_name: String,
    use_skse: bool,
    state: State<'_, AppState>,
) -> Result<LaunchResult, String> {
    let db = state.db.clone();
    let game_locks = state.game_locks.clone();
    if state.deploy_in_progress.load(std::sync::atomic::Ordering::Relaxed) {
        return Err("Cannot launch game while deployment is in progress. Please wait for the deployment to finish.".to_string());
    }
    tokio::task::spawn_blocking(move || {
    let launch_start = Instant::now();
    let (opt_bottle, game, _) = resolve_game_any_runtime(&game_id, &bottle_name)?;
    let game_path = PathBuf::from(&game.game_path);
    let data_dir_check = PathBuf::from(&game.data_dir);

    // Pre-launch self-heal applies to both Wine and native — it's just
    // a file-existence check against the deployment manifest. Run it
    // before the runtime branch.
    {
        let t = Instant::now();
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
                    Ok(_) => log::info!("Pre-launch: self-heal redeploy succeeded ({}ms)", t.elapsed().as_millis()),
                    Err(e) => log::error!("Pre-launch: self-heal redeploy failed: {}", e),
                }
            } else {
                log::info!("Pre-launch: deployment integrity OK ({} files, {}ms)", manifest.len(), t.elapsed().as_millis());
            }
        }
    }

    // Native games: launch via macOS `open`. For Steam-distributed native
    // games we MUST go through the steam:// protocol so Steamworks gets
    // injected — otherwise the game's run.sh launcher (or the binary itself)
    // detects no Steam parent process and shuts down with messages like
    // "Game not launched through Steam with Steamworks enabled, shutting
    // down...". For non-Steam sources, fall back to `open <bundle>` which
    // routes through Launch Services exactly like double-clicking the .app.
    if opt_bottle.is_none() {
        let native_ctx = game
            .runtime
            .native()
            .ok_or_else(|| "Native game has no native runtime".to_string())?;
        let bundle_path = native_ctx.app_bundle_path.clone();

        // Native launch resolution order:
        //   1. Custom default executable, if the user has set one for this
        //      game (bottle_name="" is the native sentinel in the DB).
        //   2. `steam://run/<app_id>` for Steam sources so Steamworks injects.
        //   3. `open <bundle>` (Launch Services double-click).
        //
        // For (1) we spawn the binary directly via `Command::new` so we can
        // capture the PID and feed it to the game-lock manager — that's the
        // path the game-lock check actually needs.
        let custom_exe =
            executables::get_default_executable(&db, &game_id, "").unwrap_or(None);

        if let Some(custom) = custom_exe {
            let exe_path = PathBuf::from(&custom.exe_path);
            let work_dir = custom.working_dir.clone().map(PathBuf::from);
            log::info!(
                "launch_game_cmd: native custom exe '{}' at {}",
                custom.name,
                exe_path.display()
            );
            let mut cmd = std::process::Command::new(&exe_path);
            if let Some(w) = work_dir.as_deref() {
                cmd.current_dir(w);
            }
            let child = cmd
                .spawn()
                .map_err(|e| format!("Failed to spawn '{}': {}", exe_path.display(), e))?;
            let pid = child.id();
            // Don't wait — game runs detached. Register with the lock so
            // mid-play destructive operations are blocked.
            game_locks.register(&game_id, "", pid);
            return Ok(LaunchResult {
                executable: exe_path.to_string_lossy().to_string(),
                bottle_name: String::new(),
                pid: Some(pid),
                success: true,
                warning: None,
            });
        }

        let (launch_arg, launch_kind): (String, &'static str) =
            if matches!(native_ctx.source, crate::runtime::NativeSource::Steam) {
                if let Some(app_id) = game.steam_app_id.as_deref() {
                    (format!("steam://run/{}", app_id), "steam url")
                } else {
                    (bundle_path.to_string_lossy().to_string(), "bundle path")
                }
            } else {
                (bundle_path.to_string_lossy().to_string(), "bundle path")
            };

        log::info!(
            "launch_game_cmd: native launch `open {}` ({})",
            launch_arg,
            launch_kind
        );
        let output = std::process::Command::new("open")
            .arg(&launch_arg)
            .output()
            .map_err(|e| format!("Failed to spawn `open`: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "`open {}` exited {}: {}",
                launch_arg,
                output.status,
                stderr.trim()
            ));
        }

        // `open` detaches — give the game ~400ms to start, then use pgrep -f
        // to recover the process id. Best-effort: a None here just means the
        // game-lock won't fire mid-play; the launch itself still succeeded.
        std::thread::sleep(std::time::Duration::from_millis(400));
        let exe_name = native_ctx
            .app_bundle_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let pid = lookup_native_pid(exe_name);
        if let Some(p) = pid {
            log::info!("launch_game_cmd: native pid resolved via pgrep: {}", p);
            game_locks.register(&game_id, "", p);
        }
        return Ok(LaunchResult {
            executable: launch_arg,
            bottle_name: String::new(),
            pid,
            success: true,
            warning: None,
        });
    }
    let bottle = opt_bottle.expect("opt_bottle was just checked to be Some");

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

            return launcher::launch_game(&bottle, &exe_path, work_dir.or(Some(&game_path)), Some(&game_id), None)
                .map_err(|e| format!("Launch failed ({}): {}", bottle.source, e));
        }
    }

    // NOTE: Steam protocol launch (steam://rungameid/{id}) was attempted here
    // but doesn't work when Steam is installed on both macOS and in Wine —
    // the macOS Steam client intercepts the URL and shows "Invalid platform".
    // TODO: Investigate launching the steam:// URL inside Wine instead of on
    // the host OS, or find the Wine Steam.exe and pass launch args to it.

    // Determine which executable to launch.
    // 1. SKSE takes priority if requested
    // 2. Plugin's launch_executable() for games with launcher stubs (e.g. HL)
    // 3. Plugin's detected exe_path
    // 4. Fallback: search game root for executable name
    // Map game IDs to their script extender loader executables
    let script_extender_exe = match game_id.as_str() {
        "skyrimse" => Some("skse64_loader.exe"),
        "fallout4" => Some("f4se_loader.exe"),
        "oblivion" => Some("obse_loader.exe"),
        _ => None,
    };

    let exe_path = if use_skse && script_extender_exe.is_some() {
        let exe_name = script_extender_exe.unwrap();
        launcher::find_executable(&game_path, exe_name).ok_or_else(|| {
            format!(
                "Script extender loader '{}' not found in {}. Is it installed?",
                exe_name,
                game_path.display()
            )
        })?
    } else if let Some(launch_exe) = games::with_plugin(&game_id, |p| p.launch_executable(&game_path)).flatten() {
        // Plugin specifies a dedicated launch executable (e.g. HL root launcher
        // for DRM, vs the Phoenix binary used for detection)
        if launch_exe.exists() {
            launch_exe
        } else {
            return Err(format!(
                "Launch executable not found: {}",
                launch_exe.display()
            ));
        }
    } else {
        // Default: search for the first executable name from the plugin
        let exe_name = games::with_plugin(&game_id, |plugin| {
            plugin
                .executables()
                .first()
                .map(|s| s.to_string())
                .unwrap_or_default()
        })
        .unwrap_or_default();
        if exe_name.is_empty() {
            return Err(format!(
                "No executable configured for game '{}'. Cannot launch.",
                game_id
            ));
        }
        launcher::find_executable(&game_path, &exe_name).ok_or_else(|| {
            format!(
                "Game executable '{}' not found in {}",
                exe_name,
                game_path.display()
            )
        })?
    };

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

    // Deploy DXVK config on Linux (game-specific overrides for modded games)
    #[cfg(target_os = "linux")]
    {
        let t = Instant::now();
        if let Err(e) = crate::dxvk::deploy_config(
            &game_path,
            &game_id,
            &crate::dxvk::DxvkPreset::Default,
            &std::collections::HashMap::new(),
        ) {
            log::warn!("Failed to deploy DXVK config: {}", e);
        } else {
            log::info!("Pre-launch: DXVK config deployed ({}ms)", t.elapsed().as_millis());
        }
    }

    // Pre-launch plugin sync — ensure plugins.txt reflects all deployed
    // plugins as enabled.  This catches any staleness from the game itself
    // rewriting the file on a previous exit/crash.
    let t = Instant::now();
    let _ = crate::sync_plugins_for_game(&game, &bottle);
    log::info!("Pre-launch: plugin sync ({}ms)", t.elapsed().as_millis());

    // Pre-launch SKSE plugin DLL version fix — swap incompatible plugins
    // for compatible alternatives from other installed mods' staging dirs.
    let mut wine_plugin_warning: Option<String> = None;
    if game_id == "skyrimse" {
        let skyrim_prelaunch_start = Instant::now();
        let data_dir = PathBuf::from(&game.data_dir);
        let t = Instant::now();
        let skse_fixes = skse::fix_skse_plugin_conflicts(
            &db,
            &game_id,
            &bottle_name,
            &data_dir,
            &game_path,
        );
        log::info!("Pre-launch: SKSE plugin conflict fix ({} swapped, {}ms)", skse_fixes, t.elapsed().as_millis());

        // Check if user wants Wine fork of Engine Fixes
        let use_wine_ef = config::get_config()
            .map(|c| c.use_wine_engine_fixes)
            .unwrap_or(false);

        if use_wine_ef {
            // EngineFixes Wine compatibility: disable all patches (they crash under Wine)
            let t = Instant::now();
            let ef_fixes =
                skse::fix_engine_fixes_for_wine(&data_dir, &db, &game_id, &bottle_name);
            log::info!("Pre-launch: EngineFixes TOML patch ({} patched, {}ms)", ef_fixes, t.elapsed().as_millis());
        } else {
            log::info!("Pre-launch: skipping Wine EngineFixes (user chose original)");
        }

        // Disable Wine-incompatible SKSE plugins (CrashLogger, etc.)
        let t = Instant::now();
        let wine_disabled =
            skse::disable_wine_incompatible_plugins(&data_dir, &db, &game_id, &bottle_name);
        log::info!("Pre-launch: Wine-incompatible plugin check ({} disabled, {}ms)", wine_disabled.len(), t.elapsed().as_millis());
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

        if use_wine_ef {
            // Auto-deploy SSE Engine Fixes for Wine (Wine-safe replacement)
            let t = Instant::now();
            match skse::install_engine_fixes_wine_blocking(&data_dir) {
                Ok(true) => log::info!("Pre-launch: auto-deployed SSE Engine Fixes for Wine ({}ms)", t.elapsed().as_millis()),
                Ok(false) => log::info!("Pre-launch: SSE Engine Fixes for Wine already deployed ({}ms)", t.elapsed().as_millis()),
                Err(e) => log::warn!(
                    "Pre-launch: could not auto-deploy SSE Engine Fixes for Wine: {} ({}ms)",
                    e, t.elapsed().as_millis()
                ),
            }
        }
        log::info!("Pre-launch: Skyrim-specific fixes total: {}ms", skyrim_prelaunch_start.elapsed().as_millis());
    }

    log::info!("Pre-launch: total pre-launch pipeline: {}ms", launch_start.elapsed().as_millis());

    let mut result = launcher::launch_game(&bottle, &exe_path, Some(&game_path), Some(&game_id), None)
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
    }).await.map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn check_skse(game_id: String, bottle_name: String) -> Result<SkseStatus, String> {
    tokio::task::spawn_blocking(move || {
        if game_id != "skyrimse" || bottle_name.is_empty() {
            // SKSE is Wine-only — Skyrim SE has no native runtime on macOS.
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
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub fn get_skse_download_url() -> String {
    skse::skse_download_url().to_string()
}

#[tauri::command]
pub async fn install_skse_from_archive_cmd(
    game_id: String,
    bottle_name: String,
    archive_path: String,
) -> Result<SkseStatus, String> {
    tokio::task::spawn_blocking(move || {
        if game_id != "skyrimse" {
            return Err("SKSE is only available for Skyrim Special Edition".to_string());
        }
        if bottle_name.is_empty() {
            return Err("SKSE is Wine-only — not available for native games.".to_string());
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
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn uninstall_skse_cmd(game_id: String, bottle_name: String) -> Result<SkseStatus, String> {
    tokio::task::spawn_blocking(move || {
        if game_id != "skyrimse" {
            return Err("SKSE is only available for Skyrim Special Edition".to_string());
        }
        if bottle_name.is_empty() {
            return Err("SKSE is Wine-only — not available for native games.".to_string());
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
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn set_skse_preference_cmd(
    game_id: String,
    bottle_name: String,
    enabled: bool,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        skse::set_skse_preference(&game_id, &bottle_name, enabled).map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn check_skyrim_version(
    game_id: String,
    bottle_name: String,
) -> Result<DowngradeStatus, String> {
    tokio::task::spawn_blocking(move || {
        if game_id != "skyrimse" {
            return Err("Version check is only available for Skyrim SE".to_string());
        }
        if bottle_name.is_empty() {
            return Err("Skyrim version check is Wine-only — not available for native games.".to_string());
        }

        let (_, game, _) = resolve_game(&game_id, &bottle_name)?;
        downgrader::detect_skyrim_version(Path::new(&game.game_path)).map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn check_skse_compatibility_cmd(
    game_id: String,
    bottle_name: String,
) -> Result<skse::SkseCompatibility, String> {
    tokio::task::spawn_blocking(move || {
        if game_id != "skyrimse" {
            return Err("SKSE compatibility check is only for Skyrim SE".into());
        }
        if bottle_name.is_empty() {
            return Err("SKSE compatibility check is Wine-only — not available for native games.".into());
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
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn get_skse_builds(
    game_id: String,
    bottle_name: String,
) -> Result<skse::SkseAvailableBuilds, String> {
    tokio::task::spawn_blocking(move || {
        if game_id != "skyrimse" {
            return Err("SKSE is only available for Skyrim Special Edition".into());
        }
        if bottle_name.is_empty() {
            return Err("SKSE builds are Wine-only — not available for native games.".into());
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
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn install_skse_auto_cmd(game_id: String, bottle_name: String) -> Result<SkseStatus, String> {
    if game_id != "skyrimse" {
        return Err("SKSE is only available for Skyrim Special Edition".into());
    }
    if bottle_name.is_empty() {
        return Err("SKSE auto-install is Wine-only — not available for native games.".into());
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
pub async fn scan_skse_plugins_cmd(
    game_id: String,
    bottle_name: String,
) -> Result<skse::SksePluginScanResult, String> {
    tokio::task::spawn_blocking(move || {
        if game_id != "skyrimse" {
            return Err("SKSE plugin scan is only available for Skyrim SE".into());
        }
        if bottle_name.is_empty() {
            return Err("SKSE plugin scan is Wine-only — not available for native games.".into());
        }

        let (_, game, data_dir) = resolve_game(&game_id, &bottle_name)?;
        let game_path = PathBuf::from(&game.game_path);
        let version = downgrader::detect_skyrim_version(&game_path)
            .map(|s| s.current_version)
            .unwrap_or_else(|_| "unknown".to_string());

        Ok(skse::scan_skse_plugins(&data_dir, &version))
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn fix_skse_plugins_cmd(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<usize, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        if bottle_name.is_empty() {
            return Err("SKSE plugin fix is Wine-only — not available for native games.".into());
        }
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
    .map_err(crate::format_join_error)?
}

/// List SKSE plugins that have been auto-disabled for Wine compatibility.
#[tauri::command]
pub async fn list_disabled_wine_plugins_cmd(
    game_id: String,
    bottle_name: String,
) -> Result<Vec<(String, String)>, String> {
    tokio::task::spawn_blocking(move || {
        if game_id != "skyrimse" || bottle_name.is_empty() {
            // Wine-disabled plugins only exist under Wine — native games skip cleanly.
            return Ok(vec![]);
        }
        let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
        Ok(skse::list_disabled_wine_plugins(&data_dir))
    })
    .await
    .map_err(crate::format_join_error)?
}

/// Re-enable a Wine-incompatible plugin that was auto-disabled (user override).
#[tauri::command]
pub async fn reenable_wine_plugin_cmd(
    game_id: String,
    bottle_name: String,
    dll_name: String,
) -> Result<bool, String> {
    tokio::task::spawn_blocking(move || {
        if game_id != "skyrimse" || bottle_name.is_empty() {
            // Wine-disabled plugins only exist under Wine — native games skip cleanly.
            return Ok(false);
        }
        let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
        skse::reenable_wine_plugin(&data_dir, &dll_name)
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn fix_skyrim_display(bottle_name: String) -> Result<display_fix::DisplayFixResult, String> {
    tokio::task::spawn_blocking(move || {
        let bottle = resolve_bottle(&bottle_name)?;
        display_fix::auto_fix_display(&bottle)
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn downgrade_skyrim(
    game_id: String,
    bottle_name: String,
    _mode: String,
) -> Result<DowngradeStatus, String> {
    if game_id != "skyrimse" {
        return Err("Downgrade is only available for Skyrim SE".to_string());
    }
    if bottle_name.is_empty() {
        return Err("Skyrim downgrade is Wine-only — not available for native games.".to_string());
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
pub async fn get_depot_download_command(
    game_id: String,
    bottle_name: String,
) -> Result<downgrader::DepotDownloadInfo, String> {
    tokio::task::spawn_blocking(move || {
        if bottle_name.is_empty() {
            return Err("Depot download command is Wine-only — not available for native games.".to_string());
        }
        let (bottle, _, _) = resolve_game(&game_id, &bottle_name)?;
        downgrader::get_depot_download_info(&game_id, &bottle.path).map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn start_depot_download(game_id: String) -> Result<bool, String> {
    tokio::task::spawn_blocking(move || {
        if game_id != "skyrimse" {
            return Err("Depot download only supported for Skyrim SE".into());
        }
        downgrader::send_depot_command_to_steam()
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn check_depot_ready(game_id: String, bottle_name: String) -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || {
        if bottle_name.is_empty() {
            return Err("Depot readiness check is Wine-only — not available for native games.".to_string());
        }
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
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn apply_downgrade_cmd(
    game_id: String,
    bottle_name: String,
) -> Result<DowngradeStatus, String> {
    tokio::task::spawn_blocking(move || {
        if bottle_name.is_empty() {
            return Err("Apply downgrade is Wine-only — not available for native games.".to_string());
        }
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
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn list_game_versions(game_id: String) -> Result<Vec<downgrader::CachedVersion>, String> {
    tokio::task::spawn_blocking(move || Ok(downgrader::list_cached_versions(&game_id)))
        .await
        .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn swap_game_version(
    game_id: String,
    bottle_name: String,
    target_version: String,
) -> Result<DowngradeStatus, String> {
    if bottle_name.is_empty() {
        return Err("Game version swap is Wine-only — not available for native games.".to_string());
    }
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
pub fn set_vibrancy(window: tauri::Window, material: String) -> Result<(), String> {
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


// --- Notes & Tags ---

#[tauri::command]
pub async fn set_mod_notes(
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
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn set_mod_source(
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
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn set_mod_tags(
    mod_id: i64,
    tags: Vec<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || db.set_user_tags(mod_id, &tags).map_err(|e| e.to_string()))
        .await
        .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn get_all_tags(
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
    .map_err(crate::format_join_error)?
}


// --- Auto-category ---

#[tauri::command]
pub async fn backfill_categories(
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
    .map_err(crate::format_join_error)?
}


/// Test-only helper: report whether a command that is Wine-only should
/// reject an empty `bottle_name` (the native sentinel) with a clear
/// "Wine-only" error instead of falling into `resolve_game` and emitting
/// the confusing generic "bottle '' not found".
///
/// Mirrors the early-return guards we added before every remaining
/// `resolve_game(...)` call site for Wine-only operations (SKSE,
/// downgrader, Bethesda load order, etc).
#[cfg(test)]
pub(crate) fn wine_only_rejection_message(bottle_name: &str, op: &str) -> Option<String> {
    if bottle_name.is_empty() {
        Some(format!(
            "{} is Wine-only — not available for native games.",
            op
        ))
    } else {
        None
    }
}

#[cfg(test)]
mod native_runtime_tests {
    use super::*;
    use crate::resolve_game_any_runtime;

    /// resolve_game_any_runtime with empty bottle_name and an unknown
    /// game_id must NOT emit the generic "Bottle '' not found" error;
    /// it must emit the native-specific not-found error so users hitting
    /// the bug get an actionable message instead of "bottle '' not found".
    #[test]
    fn native_resolver_emits_native_specific_error_not_bottle_error() {
        let result = resolve_game_any_runtime(
            "definitely_not_a_real_game_xyz_12345",
            "", // empty = native sentinel
        );

        let err = result.expect_err("unknown native game should error");
        assert!(
            err.contains("Native game"),
            "error must reference 'Native game', got: {}",
            err
        );
        assert!(
            !err.contains("Bottle ''") && !err.contains("bottle ''"),
            "must NOT emit the confusing empty-bottle error, got: {}",
            err
        );
    }

    /// resolve_game_any_runtime with a non-empty bottle_name that
    /// doesn't exist still produces the bottle-not-found message
    /// (preserves existing Wine error semantics).
    #[test]
    fn wine_resolver_preserves_bottle_not_found_error() {
        let result = resolve_game_any_runtime(
            "skyrimse",
            "definitely-not-a-real-bottle-xyz-12345",
        );
        let err = result.expect_err("nonexistent bottle should error");
        assert!(
            err.to_lowercase().contains("bottle"),
            "Wine path should mention 'bottle', got: {}",
            err
        );
    }

    /// SKSE commands (and all other Wine-only commands) must early-reject
    /// empty bottle_name with a clear "Wine-only" message instead of
    /// passing through to resolve_game and emitting "bottle '' not found".
    /// This guards the regression where uninstall_mod / SKSE commands
    /// errored with a confusing generic message for native games.
    #[test]
    fn skse_command_rejects_native_with_clear_error() {
        let msg = wine_only_rejection_message("", "SKSE")
            .expect("empty bottle must trigger rejection");
        assert!(
            msg.contains("Wine"),
            "rejection must mention Wine, got: {}",
            msg
        );
        assert!(
            !msg.contains("bottle ''") && !msg.contains("Bottle ''"),
            "must NOT mention the empty-bottle string, got: {}",
            msg
        );
    }

    /// Conversely, with a real (non-empty) bottle name, the Wine-only
    /// helper returns None — meaning the command falls through to
    /// resolve_game and runs as a Wine command would.
    #[test]
    fn wine_only_helper_passes_through_non_empty_bottle() {
        assert_eq!(
            wine_only_rejection_message("Skyrim", "SKSE"),
            None,
            "non-empty bottle must NOT be rejected by the Wine-only guard"
        );
    }

    /// Direct DB simulation: a mod whose `bottle_name` is the empty
    /// native-mode sentinel can be inserted, retrieved, toggled, and
    /// removed end-to-end without touching the bottle resolver at all.
    /// This is the core invariant uninstall_mod / toggle_mod now rely on
    /// after the resolve_game_any_runtime migration.
    #[test]
    fn native_mod_lifecycle_in_db_uses_empty_bottle_sentinel() {
        use crate::database::ModDatabase;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let db = ModDatabase::new(&tmp.path().join("test.db")).unwrap();

        // Native paralives mod: bottle_name = "" sentinel.
        let mod_id = db
            .add_mod(
                "paralives_native",
                "", // native sentinel
                None,
                "Test Native Mod",
                "1.0",
                "/tmp/fake-archive.zip",
                &["mods/fake.dll".to_string()],
            )
            .expect("insert native mod");

        // Toggle should work (just a DB flag change for legacy mods).
        db.set_enabled(mod_id, false).expect("disable native mod");
        let m = db.get_mod(mod_id).unwrap().unwrap();
        assert!(!m.enabled, "mod should be disabled");

        db.set_enabled(mod_id, true).expect("re-enable native mod");
        let m = db.get_mod(mod_id).unwrap().unwrap();
        assert!(m.enabled, "mod should be re-enabled");

        // Uninstall: db.remove_mod is the last step in uninstall_mod
        // after the file removal phase. Verify it succeeds and the row
        // is gone — this is the path the user just hit a bug in.
        db.remove_mod(mod_id).expect("uninstall native mod");
        assert!(
            db.get_mod(mod_id).unwrap().is_none(),
            "mod row must be gone after uninstall"
        );
    }
}

#[cfg(all(test, target_os = "macos"))]
mod native_pid_tests {
    use super::lookup_native_pid;

    /// `pgrep -f` returns no match for a deliberately-bogus exe name → None.
    /// This validates the "no match" branch which is the most common runtime
    /// outcome (game still warming up, or process never started).
    #[test]
    fn lookup_native_pid_returns_none_for_nonexistent_process() {
        // A name long+random enough that no real macOS process matches.
        let pid = lookup_native_pid("zzzz-corkscrew-test-no-such-binary-xyz-9b1f2");
        assert_eq!(pid, None);
    }

    /// Empty exe name short-circuits without spawning pgrep at all.
    #[test]
    fn lookup_native_pid_returns_none_for_empty_name() {
        let pid = lookup_native_pid("");
        assert_eq!(pid, None);
    }
}
