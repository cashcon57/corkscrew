use crate::database;
use crate::collections;
use crate::nexus;
use crate::cleaner;
use crate::collection_installer;
use crate::collections::{CollectionDiff, CollectionInfo, CollectionManifest, CollectionRevision, CollectionSearchResult, RevisionModsResult};
use crate::database::{CollectionSummary};
use crate::deploy_journal;
use crate::deployer;
use crate::loot_rules;
use crate::nexus::{NexusCategory, NexusSearchResult};
use crate::rollback;
use crate::{AppState, DeployGuard, auto_snapshot_before_destructive, check_game_lock, nexus_api_key_or_token, nexus_client, resolve_game};
use std::path::{Path, PathBuf};
use tauri::Emitter;
use tauri::{AppHandle, State};

// --- Collection Management ---

#[tauri::command]
pub async fn list_installed_collections_cmd(
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
pub async fn set_mod_collection_name_cmd(
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
pub async fn switch_collection_cmd(
    app: AppHandle,
    game_id: String,
    bottle_name: String,
    collection_name: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    check_game_lock(&state.game_locks, &game_id, &bottle_name)?;
    let db = state.db.clone();
    let app = app.clone();
    let _guard = DeployGuard::new(state.deploy_in_progress.clone(), app.clone());
    tokio::task::spawn_blocking(move || {
        let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;

        let journal_id = deploy_journal::begin(
            &game_id, &bottle_name, deploy_journal::JournalOp::RedeployAll, &[],
        ).unwrap_or_default();

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

        // 4. Redeploy with progress events
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

        // 5. Sync plugins if Skyrim SE
        if game_id == "skyrimse" {
            let _ = crate::sync_plugins_for_game(&game, &bottle);
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
pub async fn collection_download_size_cmd(
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
pub async fn delete_collection_cmd(
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
            let _ = crate::sync_plugins_for_game(&game, &bottle);
        }

        // Clean up game-specific framework files deployed outside data_dir.
        // For Hogwarts Legacy: remove UE4SS from Phoenix/Binaries/Win64/ if no
        // remaining mods need it (Lua/Logic mods).
        if game_id == "hogwartslegacy" {
            let remaining_mods = db.list_mods(&game_id, &bottle_name).unwrap_or_default();
            let has_lua_or_logic = remaining_mods.iter().any(|m| {
                m.installed_files.iter().any(|f| {
                    let fl = f.to_lowercase();
                    fl.contains("scripts/main.lua")
                        || fl.contains("logicmods/")
                        || fl.ends_with(".logicmod")
                        || fl.ends_with(".ue4sslogicmod")
                })
            });
            if !has_lua_or_logic {
                let win64 = game.game_path.join("Phoenix").join("Binaries").join("Win64");
                let ue4ss_files = ["dwmapi.dll", "UE4SS.dll", "UE4SS-settings.ini", "Changelog.md", "README.md", ".version"];
                let mut removed = 0;
                for fname in &ue4ss_files {
                    let f = win64.join(fname);
                    if f.exists() {
                        if let Ok(()) = std::fs::remove_file(&f) {
                            removed += 1;
                        }
                    }
                }
                // Remove Mods/ directory (UE4SS Lua mods)
                let mods_dir = win64.join("Mods");
                if mods_dir.exists() {
                    let _ = std::fs::remove_dir_all(&mods_dir);
                    removed += 1;
                }
                // Remove Tools/ue4ss/
                let tools_dir = game.game_path.join("Phoenix").join("Binaries").join("Tools").join("ue4ss");
                if tools_dir.exists() {
                    let _ = std::fs::remove_dir_all(&tools_dir);
                    removed += 1;
                }
                if removed > 0 {
                    log::info!(
                        "HL cleanup: removed {} UE4SS files/dirs (no remaining Lua/Logic mods)",
                        removed
                    );
                }
            }
            // Remove Tools/ directories that may contain UE4SS DLLs.
            // Paks/Tools/ is toxic (game scans Paks/ and loads DLLs).
            // Binaries/Tools/ is safe but should be cleaned on full uninstall.
            if let Some(paks_dir) = data_dir.parent() {
                let toxic_tools = paks_dir.join("Tools");
                if toxic_tools.exists() {
                    let _ = std::fs::remove_dir_all(&toxic_tools);
                    log::info!("HL cleanup: removed toxic Tools/ directory under Paks/");
                }
            }
            let binaries_tools = game.game_path.join("Phoenix").join("Binaries").join("Tools");
            if binaries_tools.exists() {
                let _ = std::fs::remove_dir_all(&binaries_tools);
                log::info!("HL cleanup: removed Binaries/Tools/");
            }

            // Remove merged PAK database if no remaining PAK mods
            let merged_pak = data_dir.join("zMergedMods_P.pak");
            if merged_pak.exists() {
                let remaining_paks = std::fs::read_dir(&data_dir)
                    .map(|entries| {
                        entries
                            .filter_map(|e| e.ok())
                            .filter(|e| {
                                let name = e.file_name().to_string_lossy().to_lowercase();
                                name.ends_with(".pak") && name != "zmergedmods_p.pak"
                            })
                            .count()
                    })
                    .unwrap_or(0);
                if remaining_paks == 0 {
                    let _ = std::fs::remove_file(&merged_pak);
                    log::info!("HL cleanup: removed zMergedMods_P.pak (no remaining PAK mods)");
                }
            }
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
pub async fn uninstall_wabbajack_modlist(
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
            let _ = crate::sync_plugins_for_game(&game, &bottle);
        }

        // Clean up HL-specific framework files (UE4SS) if no remaining mods need them
        if game_id == "hogwartslegacy" {
            let remaining_mods = db.list_mods(&game_id, &bottle_name).unwrap_or_default();
            let has_lua_or_logic = remaining_mods.iter().any(|m| {
                m.installed_files.iter().any(|f| {
                    let fl = f.to_lowercase();
                    fl.contains("scripts/main.lua")
                        || fl.contains("logicmods/")
                        || fl.ends_with(".logicmod")
                        || fl.ends_with(".ue4sslogicmod")
                })
            });
            if !has_lua_or_logic {
                let win64 = game.game_path.join("Phoenix").join("Binaries").join("Win64");
                let ue4ss_files = ["dwmapi.dll", "UE4SS.dll", "UE4SS-settings.ini", "Changelog.md", "README.md", ".version"];
                let mut removed = 0;
                for fname in &ue4ss_files {
                    let f = win64.join(fname);
                    if f.exists() {
                        if let Ok(()) = std::fs::remove_file(&f) {
                            removed += 1;
                        }
                    }
                }
                let mods_dir = win64.join("Mods");
                if mods_dir.exists() {
                    let _ = std::fs::remove_dir_all(&mods_dir);
                    removed += 1;
                }
                let tools_dir = game.game_path.join("Phoenix").join("Binaries").join("Tools").join("ue4ss");
                if tools_dir.exists() {
                    let _ = std::fs::remove_dir_all(&tools_dir);
                    removed += 1;
                }
                if removed > 0 {
                    log::info!(
                        "HL cleanup: removed {} UE4SS files/dirs (no remaining Lua/Logic mods)",
                        removed
                    );
                }
            }
            // Remove Tools/ directories that may contain UE4SS DLLs.
            // Paks/Tools/ is toxic (game scans Paks/ and loads DLLs).
            // Binaries/Tools/ is safe but should be cleaned on full uninstall.
            if let Some(paks_dir) = data_dir.parent() {
                let toxic_tools = paks_dir.join("Tools");
                if toxic_tools.exists() {
                    let _ = std::fs::remove_dir_all(&toxic_tools);
                    log::info!("HL cleanup: removed toxic Tools/ directory under Paks/");
                }
            }
            let binaries_tools = game.game_path.join("Phoenix").join("Binaries").join("Tools");
            if binaries_tools.exists() {
                let _ = std::fs::remove_dir_all(&binaries_tools);
                log::info!("HL cleanup: removed Binaries/Tools/");
            }

            // Remove merged PAK database if no remaining PAK mods
            let merged_pak = data_dir.join("zMergedMods_P.pak");
            if merged_pak.exists() {
                let remaining_paks = std::fs::read_dir(&data_dir)
                    .map(|entries| {
                        entries
                            .filter_map(|e| e.ok())
                            .filter(|e| {
                                let name = e.file_name().to_string_lossy().to_lowercase();
                                name.ends_with(".pak") && name != "zmergedmods_p.pak"
                            })
                            .count()
                    })
                    .unwrap_or(0);
                if remaining_paks == 0 {
                    let _ = std::fs::remove_file(&merged_pak);
                    log::info!("HL cleanup: removed zMergedMods_P.pak (no remaining PAK mods)");
                }
            }
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
pub async fn restore_mod_snapshot(
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
            let _ = crate::sync_plugins_for_game(&game, &bottle);
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
pub async fn return_to_vanilla(
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
            let _ = crate::sync_plugins_for_game(&game, &bottle);
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
pub async fn get_collection_diff_cmd(
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


// --- Collections ---

#[tauri::command]
pub async fn fetch_url_text(url: String) -> Result<String, String> {
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
pub async fn browse_nexus_mods_cmd(
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
pub async fn get_nexus_mod_detail(
    game_slug: String,
    mod_id: i64,
) -> Result<nexus::NexusModInfo, String> {
    let client = nexus_client().await?;
    client
        .get_mod_info(&game_slug, mod_id)
        .await
        .map_err(|e| e.to_string())
}


// --- Collection Install Resume ---

#[tauri::command]
pub async fn get_incomplete_collection_installs(
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
pub async fn get_all_interrupted_installs(
    state: State<'_, AppState>,
) -> Result<Vec<database::CollectionInstallCheckpoint>, String> {
    state
        .db
        .get_all_active_checkpoints()
        .map_err(|e| format!("Failed to query interrupted installs: {}", e))
}

#[tauri::command]
pub async fn get_checkpoint_mod_names(
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
pub async fn resume_collection_install_cmd(
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
pub async fn abandon_collection_install(
    checkpoint_id: i64,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .db
        .abandon_checkpoint(checkpoint_id)
        .map_err(|e| format!("Failed to abandon checkpoint: {}", e))
}

#[tauri::command]
pub async fn get_pending_wabbajack_installs(
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

/// Permanently dismiss a pending Wabbajack install by marking it as cancelled.
#[tauri::command]
pub async fn dismiss_wabbajack_install(
    install_id: i64,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .db
        .update_wj_install_status(install_id, "cancelled", None)
        .map_err(|e| format!("Failed to dismiss WJ install: {}", e))
}


// --- Endorsements ---

#[tauri::command]
pub async fn endorse_mod(
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
pub async fn abstain_mod(game_slug: String, mod_id: i64) -> Result<nexus::EndorseResponse, String> {
    let client = nexus_client().await?;
    client
        .abstain_mod(&game_slug, mod_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_user_endorsements() -> Result<Vec<nexus::UserEndorsement>, String> {
    let client = nexus_client().await?;
    client
        .get_user_endorsements()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn search_nexus_mods_cmd(
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
pub async fn get_game_categories_cmd(game_slug: String) -> Result<Vec<NexusCategory>, String> {
    let client = nexus_client().await?;
    client
        .get_game_categories(&game_slug)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn browse_collections_cmd(
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
pub async fn get_collection_cmd(slug: String, game_domain: String) -> Result<CollectionInfo, String> {
    let token = nexus_api_key_or_token().await.ok().map(|(t, _)| t);

    collections::get_collection(token.as_deref(), &slug, &game_domain)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_collection_revisions(slug: String) -> Result<Vec<CollectionRevision>, String> {
    let token = nexus_api_key_or_token().await.ok().map(|(t, _)| t);

    collections::get_revisions(token.as_deref(), &slug)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_collection_mods(slug: String, revision: u32) -> Result<RevisionModsResult, String> {
    let token = nexus_api_key_or_token().await.ok().map(|(t, _)| t);

    collections::get_revision_mods(token.as_deref(), &slug, revision)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn parse_collection_bundle_cmd(bundle_path: String) -> Result<CollectionManifest, String> {
    tokio::task::spawn_blocking(move || {
        collections::parse_collection_bundle(Path::new(&bundle_path)).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn install_collection_cmd(
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
pub async fn cancel_collection_install_cmd() -> Result<(), String> {
    collection_installer::cancel_install();
    Ok(())
}

#[tauri::command]
pub async fn submit_fomod_choices(
    correlation_id: String,
    selections: std::collections::HashMap<String, Vec<String>>,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        collection_installer::submit_fomod_choices(&correlation_id, selections)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}


// --- Download Cache Check ---

#[tauri::command]
pub async fn check_cached_files(
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


/// Re-sync plugin load order, enabling all deployed plugins.
///
/// Call this after a collection install (or any time Plugins.txt looks wrong)
/// to ensure every plugin file in the Data directory is marked as enabled.
#[tauri::command]
pub async fn sync_plugins_cmd(
    game_id: String,
    bottle_name: String,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || {
        let (bottle, game, _data_dir) = crate::resolve_game(&game_id, &bottle_name)?;
        crate::sync_plugins_for_game(&game, &bottle)?;
        Ok(serde_json::json!({ "ok": true }))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}
