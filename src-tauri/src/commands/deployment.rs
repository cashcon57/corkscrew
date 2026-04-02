//! Deployment commands: deploy, redeploy, health checks, and background hashing.

use crate::background_hash;
use crate::config;
use crate::conflict_resolver;
use crate::database::{DeploymentEntry, FileConflict};
use crate::deploy_journal;
use crate::deployer;
use crate::skse;
use crate::staging;
use crate::{AppState, DeployGuard, auto_snapshot_before_destructive, check_game_lock, resolve_bottle, resolve_game};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tauri::Emitter;
use tauri::{AppHandle, State};

// --- Deployment Management ---

#[tauri::command]
pub async fn get_conflicts(
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
pub struct AnalyzeConflictsResponse {
    suggestions: Vec<conflict_resolver::ConflictSuggestion>,
    identical_stats: conflict_resolver::IdenticalContentStats,
}

#[tauri::command]
pub async fn analyze_conflicts_cmd(
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
        let loot_order = crate::get_current_plugins(&game_id, &bottle_name);
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
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn resolve_all_conflicts_cmd(
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

        let loot_order = crate::get_current_plugins(&game_id, &bottle_name);
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
                let _ = crate::sync_plugins_for_game(&game, &bottle);
            }
        }

        Ok(result)
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn record_conflict_winner(
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
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn get_deployment_manifest_cmd(
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
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn set_mod_priority(
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
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn reorder_mods(
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
            let _ = crate::sync_plugins_for_game(&game, &bottle);
        }

        Ok(())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn redeploy_all_mods(
    app: AppHandle,
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    check_game_lock(&state.game_locks, &game_id, &bottle_name)?;
    let db = state.db.clone();
    let app = app.clone();
    let _guard = DeployGuard::new(state.deploy_in_progress.clone(), app.clone());
    tokio::task::spawn_blocking(move || {
        let redeploy_start = Instant::now();
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
            let _ = crate::sync_plugins_for_game(&game, &bottle);
            let use_wine_ef = !config::get_config()
                .map(|c| c.use_original_engine_fixes)
                .unwrap_or(false);
            if use_wine_ef {
                let ef = skse::fix_engine_fixes_for_wine(&data_dir, &db, &game_id, &bottle_name);
                if ef > 0 {
                    log::info!(
                        "Redeploy: patched {} EngineFixes TOML(s) for Wine compatibility",
                        ef
                    );
                }
            } else {
                log::info!("Redeploy: skipping Wine EngineFixes (user chose original)");
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
            if use_wine_ef {
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
        }

        let elapsed = redeploy_start.elapsed();
        log::info!(
            "Redeploy complete: {} deployed, {} skipped, {:.1}s",
            result.deployed_count, result.skipped_count,
            elapsed.as_secs_f64()
        );

        Ok(serde_json::json!({
            "deployed_count": result.deployed_count,
            "skipped_count": result.skipped_count,
            "fallback_used": result.fallback_used,
            "elapsed_ms": elapsed.as_millis() as u64,
        }))
    })
    .await
    .map_err(crate::format_join_error)?
}

/// Check whether a deployment operation is currently in progress.
#[tauri::command]
pub fn is_deploy_in_progress(state: State<'_, AppState>) -> bool {
    state.deploy_in_progress.load(std::sync::atomic::Ordering::Relaxed)
}

/// Incremental deployment: compute diff and apply only changes.
/// Falls back to full redeploy if >80% of files would change.
#[tauri::command]
pub async fn deploy_incremental_cmd(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<deployer::IncrementalDeployResult, String> {
    check_game_lock(&state.game_locks, &game_id, &bottle_name)?;
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let deploy_start = Instant::now();
        let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;

        let result =
            deployer::deploy_incremental(&db, &game_id, &bottle_name, &data_dir, &game.game_path)
                .map_err(|e| e.to_string())?;

        log::info!(
            "Incremental deploy: {} added, {} removed, {} updated, {} unchanged, {:.1}s",
            result.files_added, result.files_removed, result.files_updated, result.files_unchanged,
            deploy_start.elapsed().as_secs_f64()
        );

        if game_id == "skyrimse" {
            let _ = crate::sync_plugins_for_game(&game, &bottle);
            let use_wine_ef = !config::get_config()
                .map(|c| c.use_original_engine_fixes)
                .unwrap_or(false);
            if use_wine_ef {
                let ef = skse::fix_engine_fixes_for_wine(&data_dir, &db, &game_id, &bottle_name);
                if ef > 0 {
                    log::info!(
                        "Incremental deploy: patched {} EngineFixes TOML(s) for Wine compatibility",
                        ef
                    );
                }
            } else {
                log::info!("Incremental deploy: skipping Wine EngineFixes (user chose original)");
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
            if use_wine_ef {
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
        }

        log::info!("Incremental deploy total (with post-deploy fixes): {:.1}s", deploy_start.elapsed().as_secs_f64());

        Ok(result)
    })
    .await
    .map_err(crate::format_join_error)?
}

/// Check deployment health: verify mods have staging dirs and deployed files.
/// Verification depth is controlled by the `verification_level` config setting:
/// - Fast: file existence only
/// - Balanced: existence + spot-check 10% of files by SHA-256
/// - Paranoid: existence + full SHA-256 verification of every file
#[tauri::command]
pub async fn check_deployment_health(
    app: AppHandle,
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let db = state.db.clone();
    let app = app.clone();
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

        let _ = app.emit("health-check-progress", serde_json::json!({
            "step": "staging",
            "message": format!("Checking staging for {} mods...", mods.len()),
            "current": 0,
            "total": mods.len(),
        }));

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

        let _ = app.emit("health-check-progress", serde_json::json!({
            "step": "deployment",
            "message": format!("Verifying {} deployed files...", manifest.len()),
            "current": 0,
            "total": manifest.len(),
        }));

        // Check deployment manifest vs data dir (existence check — all modes)
        let mut deployed_ok = 0usize;
        let mut deployed_missing = 0usize;
        for (idx, entry) in manifest.iter().enumerate() {
            let file_path = data_dir.join(&entry.relative_path);
            if file_path.exists() {
                deployed_ok += 1;
            } else {
                deployed_missing += 1;
            }
            if idx % 5000 == 0 {
                let _ = app.emit("health-check-progress", serde_json::json!({
                    "step": "deployment",
                    "message": format!("Checking file {}/{}...", idx + 1, manifest.len()),
                    "current": idx + 1,
                    "total": manifest.len(),
                }));
            }
        }

        let level_str = match verification_level {
            config::VerificationLevel::Fast => "Fast",
            config::VerificationLevel::Balanced => "Balanced",
            config::VerificationLevel::Paranoid => "Paranoid",
        };

        let _ = app.emit("health-check-progress", serde_json::json!({
            "step": "verification",
            "message": format!("Running {} hash verification...", level_str),
            "current": 0,
            "total": 0,
        }));

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

        let _ = app.emit("health-check-progress", serde_json::json!({
            "step": "complete",
            "message": "Health check complete",
            "current": 1,
            "total": 1,
        }));

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
    .map_err(crate::format_join_error)?
}

/// Get the current verification level from config.
#[tauri::command]
pub async fn get_verification_level() -> Result<String, String> {
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
    .map_err(crate::format_join_error)?
}

/// Set the verification level in config.
#[tauri::command]
pub async fn set_verification_level(level: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        config::set_config_value("verification_level", &level).map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}

/// Legacy: Toggle whether to use the original SSE Engine Fixes instead of the Wine fork.
/// Kept for backward compatibility — new code should use `set_use_wine_engine_fixes`.
#[tauri::command]
pub async fn set_use_original_engine_fixes(enabled: bool) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let mut cfg = config::get_config().map_err(|e| e.to_string())?;
        cfg.use_original_engine_fixes = enabled;
        config::save_config(&cfg).map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
    .map_err(crate::format_join_error)?
}

/// Toggle whether to deploy SSE Engine Fixes for Wine (opt-in).
#[tauri::command]
pub async fn set_use_wine_engine_fixes(enabled: bool) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let mut cfg = config::get_config().map_err(|e| e.to_string())?;
        cfg.use_wine_engine_fixes = enabled;
        config::save_config(&cfg).map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn purge_deployment_cmd(
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
            let _ = crate::sync_plugins_for_game(&game, &bottle);
        }

        Ok(removed)
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn verify_mod_integrity(
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
    .map_err(crate::format_join_error)?
}


// --- Deployment Health ---

#[tauri::command]
pub async fn get_deployment_health(
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
pub async fn get_deployment_stats(
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
pub async fn start_background_hashing(
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
pub fn cancel_background_hashing() {
    background_hash::cancel();
}

