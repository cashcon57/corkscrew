use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::Semaphore;

use crate::bottles;
use crate::collections::{self, CollectionManifest, CollectionModEntry};
use crate::config;
use crate::database::{self, ModDatabase};
use crate::deployer;
use crate::download_queue::{DownloadQueue, DOWNLOAD_QUEUE_EVENT};
use crate::fomod;
use crate::games;
use crate::nexus::NexusClient;
use crate::plugins;
use crate::profiles;
use crate::progress::{InstallProgress, INSTALL_PROGRESS_EVENT};
use crate::staging;

#[derive(Clone, Debug, Serialize)]
pub struct CollectionInstallResult {
    pub installed: usize,
    pub already_installed: usize,
    pub skipped: usize,
    pub failed: usize,
    pub details: Vec<ModInstallDetail>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ModInstallDetail {
    pub name: String,
    pub status: String, // "installed", "already_installed", "user_action", "failed"
    pub error: Option<String>,
    pub url: Option<String>,
    pub instructions: Option<String>,
}

/// Result of a download-only operation (no extract/deploy).
struct DownloadResult {
    archive_path: PathBuf,
    cached: bool,
}

/// Map internal game IDs to NexusMods domain slugs.
fn game_id_to_nexus_slug(game_id: &str) -> &str {
    match game_id {
        "skyrimse" => "skyrimspecialedition",
        "falloutnv" => "newvegas",
        "enderalse" => "enderalspecialedition",
        // These map 1:1
        "skyrim" | "fallout4" | "fallout3" | "oblivion" | "morrowind" | "starfield" | "enderal" => {
            game_id
        }
        other => other,
    }
}

/// Download a mod archive without extracting or deploying it.
///
/// Checks the dedup cache first; if found and the archive still exists on disk,
/// returns the cached path.  Otherwise performs the actual download and registers
/// the result in the dedup registry.
#[allow(clippy::too_many_arguments)]
async fn download_mod_archive(
    app: &AppHandle,
    db: &Arc<ModDatabase>,
    queue: &Arc<DownloadQueue>,
    mod_entry: &CollectionModEntry,
    mod_index: usize,
    game_slug: &str,
    download_dir: &Path,
    api_key: &Option<String>,
    manifest_name: &str,
    game_id: &str,
    bottle_name: &str,
) -> Result<DownloadResult, InstallError> {
    let source_type = mod_entry.source.source_type.as_str();

    match source_type {
        "nexus" => {
            download_nexus_archive(
                app,
                db,
                queue,
                mod_entry,
                mod_index,
                game_slug,
                download_dir,
                api_key,
                manifest_name,
                game_id,
                bottle_name,
            )
            .await
        }
        "direct" => {
            download_direct_archive(
                app,
                db,
                queue,
                mod_entry,
                mod_index,
                download_dir,
                api_key,
                manifest_name,
                game_id,
                bottle_name,
            )
            .await
        }
        _ => Err(InstallError::Failed(format!(
            "Cannot pre-download source type '{}' for '{}'",
            source_type, mod_entry.name
        ))),
    }
}

/// Download-only path for a Nexus mod.  Returns the archive path without
/// extracting or deploying anything.
#[allow(clippy::too_many_arguments)]
async fn download_nexus_archive(
    app: &AppHandle,
    db: &Arc<ModDatabase>,
    queue: &Arc<DownloadQueue>,
    mod_entry: &CollectionModEntry,
    mod_index: usize,
    game_slug: &str,
    download_dir: &Path,
    api_key: &Option<String>,
    manifest_name: &str,
    game_id: &str,
    bottle_name: &str,
) -> Result<DownloadResult, InstallError> {
    let nexus_mod_id = mod_entry
        .source
        .mod_id
        .ok_or_else(|| InstallError::Failed(format!("No Nexus mod ID for '{}'", mod_entry.name)))?;
    let nexus_file_id = mod_entry.source.file_id.ok_or_else(|| {
        InstallError::Failed(format!("No Nexus file ID for '{}'", mod_entry.name))
    })?;

    // Check dedup cache
    if let Ok(Some(existing)) = db.find_download_by_nexus_ids(nexus_mod_id, nexus_file_id) {
        let path = PathBuf::from(&existing.archive_path);
        if path.exists() {
            log::info!(
                "Reusing cached download for '{}' (nexus {}:{})",
                mod_entry.name,
                nexus_mod_id,
                nexus_file_id
            );
            let _ =
                db.add_download_collection_ref(existing.id, manifest_name, game_id, bottle_name);
            return Ok(DownloadResult {
                archive_path: path,
                cached: true,
            });
        }
    }

    let key = api_key
        .as_ref()
        .ok_or_else(|| InstallError::Failed("No API key configured".to_string()))?;
    let client = NexusClient::new(key.clone());

    // Get download links
    let links = client
        .get_download_links(game_slug, nexus_mod_id, nexus_file_id, None, None)
        .await
        .map_err(|e| InstallError::Failed(format!("Download links failed: {}", e)))?;

    let download_url = &links
        .first()
        .ok_or_else(|| {
            InstallError::Failed("No download links returned by NexusMods API".to_string())
        })?
        .uri;

    // Enqueue in download queue for tracking
    let queue_id = queue.enqueue(
        &mod_entry.name,
        download_url.rsplit('/').next().unwrap_or(&mod_entry.name),
        Some(nexus_mod_id),
        Some(nexus_file_id),
        None,
        Some(game_slug),
    );
    queue.set_downloading(queue_id);
    let _ = app.emit(DOWNLOAD_QUEUE_EVENT, queue.get_all());

    // Download with progress
    let last_emit = std::sync::Mutex::new(std::time::Instant::now());
    let app_clone = app.clone();
    let queue_clone = Arc::clone(queue);
    let archive_path = client
        .download_file(
            download_url,
            download_dir,
            Some(move |downloaded, total| {
                let mut last = last_emit.lock().unwrap_or_else(|e| e.into_inner());
                if last.elapsed().as_millis() >= 100 || downloaded == total {
                    queue_clone.set_progress(queue_id, downloaded, total);
                    let _ = app_clone.emit(
                        INSTALL_PROGRESS_EVENT,
                        InstallProgress::DownloadProgress {
                            mod_index,
                            downloaded,
                            total,
                        },
                    );
                    *last = std::time::Instant::now();
                }
            }),
        )
        .await
        .map_err(|e| {
            queue.set_failed(queue_id, &e.to_string());
            let _ = app.emit(DOWNLOAD_QUEUE_EVENT, queue.get_all());
            InstallError::Failed(format!("Download failed: {}", e))
        })?;

    queue.set_completed(queue_id);
    let _ = app.emit(DOWNLOAD_QUEUE_EVENT, queue.get_all());

    // Register in dedup registry
    let archive_name = archive_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let sha = staging::compute_sha256(&archive_path).ok();
    let file_size = std::fs::metadata(&archive_path)
        .map(|m| m.len() as i64)
        .unwrap_or(0);
    if let Ok(dl_id) = db.register_download(
        &archive_path.to_string_lossy(),
        &archive_name,
        Some(nexus_mod_id),
        Some(nexus_file_id),
        sha.as_deref(),
        file_size,
    ) {
        let _ = db.add_download_collection_ref(dl_id, manifest_name, game_id, bottle_name);
    }

    Ok(DownloadResult {
        archive_path,
        cached: false,
    })
}

/// Download-only path for a direct-URL mod.
#[allow(clippy::too_many_arguments)]
async fn download_direct_archive(
    app: &AppHandle,
    db: &Arc<ModDatabase>,
    queue: &Arc<DownloadQueue>,
    mod_entry: &CollectionModEntry,
    mod_index: usize,
    download_dir: &Path,
    api_key: &Option<String>,
    manifest_name: &str,
    game_id: &str,
    bottle_name: &str,
) -> Result<DownloadResult, InstallError> {
    let url =
        mod_entry.source.url.as_ref().ok_or_else(|| {
            InstallError::Failed(format!("No download URL for '{}'", mod_entry.name))
        })?;

    // Check dedup by filename
    let url_filename = url
        .rsplit('/')
        .next()
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("");
    if !url_filename.is_empty() {
        if let Ok(Some(existing)) = db.find_download_by_name(url_filename) {
            let path = PathBuf::from(&existing.archive_path);
            if path.exists() {
                log::info!(
                    "Reusing cached download for '{}' ({})",
                    mod_entry.name,
                    url_filename
                );
                return Ok(DownloadResult {
                    archive_path: path,
                    cached: true,
                });
            }
        }
    }

    // Enqueue in download queue for tracking
    let queue_id = queue.enqueue(
        &mod_entry.name,
        url_filename,
        mod_entry.source.mod_id,
        mod_entry.source.file_id,
        Some(url),
        None,
    );
    queue.set_downloading(queue_id);
    let _ = app.emit(DOWNLOAD_QUEUE_EVENT, queue.get_all());

    // Download
    let dummy_key = api_key.as_deref().unwrap_or("").to_string();
    let client = NexusClient::new(dummy_key);

    let app_clone = app.clone();
    let last_emit = std::sync::Mutex::new(std::time::Instant::now());
    let queue_clone = Arc::clone(queue);
    let archive_path = client
        .download_file(
            url,
            download_dir,
            Some(move |downloaded, total| {
                let mut last = last_emit.lock().unwrap_or_else(|e| e.into_inner());
                if last.elapsed().as_millis() >= 100 || downloaded == total {
                    queue_clone.set_progress(queue_id, downloaded, total);
                    let _ = app_clone.emit(
                        INSTALL_PROGRESS_EVENT,
                        InstallProgress::DownloadProgress {
                            mod_index,
                            downloaded,
                            total,
                        },
                    );
                    *last = std::time::Instant::now();
                }
            }),
        )
        .await
        .map_err(|e| {
            queue.set_failed(queue_id, &e.to_string());
            let _ = app.emit(DOWNLOAD_QUEUE_EVENT, queue.get_all());
            InstallError::Failed(format!("Download failed: {}", e))
        })?;

    queue.set_completed(queue_id);
    let _ = app.emit(DOWNLOAD_QUEUE_EVENT, queue.get_all());

    // Register in dedup registry
    let archive_name = archive_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let sha = staging::compute_sha256(&archive_path).ok();
    let file_size = std::fs::metadata(&archive_path)
        .map(|m| m.len() as i64)
        .unwrap_or(0);
    if let Ok(dl_id) = db.register_download(
        &archive_path.to_string_lossy(),
        &archive_name,
        mod_entry.source.mod_id,
        mod_entry.source.file_id,
        sha.as_deref(),
        file_size,
    ) {
        let _ = db.add_download_collection_ref(dl_id, manifest_name, game_id, bottle_name);
    }

    Ok(DownloadResult {
        archive_path,
        cached: false,
    })
}

/// Install a NexusMods collection into the given game/bottle.
///
/// This orchestrates the full pipeline in two phases:
/// Phase 1: Concurrent downloads (nexus + direct mods only, guarded by semaphore)
/// Phase 2: Sequential install (extract, stage, deploy each mod in order)
/// Resume a previously interrupted collection install from its checkpoint.
pub async fn resume_collection_install(
    app: &AppHandle,
    db: &Arc<ModDatabase>,
    queue: &Arc<DownloadQueue>,
    checkpoint_id: i64,
) -> Result<CollectionInstallResult, String> {
    let checkpoint = db
        .get_active_checkpoint_by_id(checkpoint_id)
        .map_err(|e| format!("Failed to load checkpoint: {}", e))?
        .ok_or_else(|| "Checkpoint not found or already completed".to_string())?;

    let manifest: CollectionManifest = serde_json::from_str(&checkpoint.manifest_json)
        .map_err(|e| format!("Failed to parse saved manifest: {}", e))?;

    let string_statuses: HashMap<String, String> =
        serde_json::from_str(&checkpoint.mod_statuses).unwrap_or_default();

    // Convert string keys to usize
    let completed: HashMap<usize, String> = string_statuses
        .into_iter()
        .filter_map(|(k, v)| k.parse::<usize>().ok().map(|idx| (idx, v)))
        .collect();

    install_collection(
        app,
        db,
        queue,
        &manifest,
        &checkpoint.game_id,
        &checkpoint.bottle_name,
        Some((checkpoint_id, completed)),
    )
    .await
}

pub async fn install_collection(
    app: &AppHandle,
    db: &Arc<ModDatabase>,
    queue: &Arc<DownloadQueue>,
    manifest: &CollectionManifest,
    game_id: &str,
    bottle_name: &str,
    resume_checkpoint: Option<(i64, HashMap<usize, String>)>,
) -> Result<CollectionInstallResult, String> {
    // Resolve game and paths
    let bottle = bottles::find_bottle_by_name(bottle_name)
        .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))?;
    let detected_games = games::detect_games(&bottle);
    let game = detected_games
        .iter()
        .find(|g| g.game_id == game_id)
        .ok_or_else(|| format!("Game '{}' not found in bottle '{}'", game_id, bottle_name))?;

    let data_dir = PathBuf::from(&game.data_dir);

    // Get download directory
    let download_dir = config::get_config()
        .ok()
        .and_then(|c| c.download_dir.map(PathBuf::from))
        .unwrap_or_else(config::downloads_dir);

    if !download_dir.exists() {
        std::fs::create_dir_all(&download_dir)
            .map_err(|e| format!("Failed to create download dir: {}", e))?;
    }

    // Check premium status once upfront
    let api_key = config::get_config().ok().and_then(|c| c.nexus_api_key);

    let is_premium = if let Some(ref key) = api_key {
        let client = NexusClient::new(key.clone());
        client.is_premium().await
    } else {
        false
    };

    // Resolve install order (topological sort respecting mod rules)
    let install_order = collections::resolve_install_order(manifest);
    let total_mods = install_order.len();

    let mut installed = 0usize;
    let mut already_installed = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let mut details: Vec<ModInstallDetail> = Vec::new();

    // Determine game slug for Nexus API
    let game_slug = game_id_to_nexus_slug(game_id);

    // Validate collection is for the correct game
    if !manifest.game_domain.is_empty() && manifest.game_domain != game_slug {
        return Err(format!(
            "Collection is for '{}' but target game is '{}' ({})",
            manifest.game_domain, game_slug, game_id
        ));
    }

    // Load existing mods for already-installed detection
    let existing_mods = db.list_mods(game_id, bottle_name).unwrap_or_default();

    // ---------------------------------------------------------------
    // Checkpoint: create or resume
    // ---------------------------------------------------------------
    let (checkpoint_id, completed_statuses) = match resume_checkpoint {
        Some((id, statuses)) => {
            log::info!(
                "Resuming collection install from checkpoint {} ({}/{} completed)",
                id,
                statuses.values().filter(|s| matches!(s.as_str(), "installed" | "already_installed" | "skipped" | "user_action")).count(),
                total_mods,
            );
            (id, statuses)
        }
        None => {
            let manifest_json = serde_json::to_string(manifest)
                .map_err(|e| format!("Failed to serialize manifest: {}", e))?;
            let id = db
                .create_collection_checkpoint(
                    &manifest.name,
                    game_id,
                    bottle_name,
                    &manifest_json,
                    total_mods,
                )
                .map_err(|e| format!("Failed to create checkpoint: {}", e))?;
            log::info!("Created collection install checkpoint {}", id);
            (id, HashMap::new())
        }
    };

    // ---------------------------------------------------------------
    // Phase 1: Concurrent Downloads
    // ---------------------------------------------------------------
    // Only nexus/direct mods that are NOT already installed and (for
    // nexus) require premium are eligible for concurrent download.
    // browse/manual/bundled types are skipped here and handled in
    // Phase 2 as before.

    // Determine concurrency limit from config or platform heuristics
    let max_concurrent = config::get_config()
        .ok()
        .and_then(|c| c.extra.get("download_threads").and_then(|v| v.as_u64()))
        .map(|v| v as usize)
        .unwrap_or_else(|| {
            let cores = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4);
            let is_apple_silicon =
                cfg!(target_arch = "aarch64") && cfg!(target_os = "macos");
            let is_steam_os = std::path::Path::new("/etc/steamos-release").exists();
            if is_steam_os {
                cores.min(4)
            } else if is_apple_silicon {
                (cores / 2).clamp(4, 8)
            } else {
                (cores / 2).clamp(3, 6)
            }
        });

    // Build list of (order_position, manifest_index) pairs eligible for pre-download
    let mut downloadable: Vec<(usize, usize)> = Vec::new();
    for (i, &mod_idx) in install_order.iter().enumerate() {
        let entry = &manifest.mods[mod_idx];
        let stype = entry.source.source_type.as_str();

        // Skip types that cannot be pre-downloaded
        if !matches!(stype, "nexus" | "direct") {
            continue;
        }

        // Skip nexus mods when user is not premium (they need manual browser download)
        if stype == "nexus" && !is_premium {
            continue;
        }

        // Skip already-installed mods
        let is_already = if let Some(nexus_id) = entry.source.mod_id {
            existing_mods
                .iter()
                .any(|m| m.nexus_mod_id == Some(nexus_id))
        } else {
            existing_mods
                .iter()
                .any(|m| m.name.eq_ignore_ascii_case(&entry.name))
        };
        if is_already {
            continue;
        }

        downloadable.push((i, mod_idx));
    }

    let total_downloads = downloadable.len();

    let _ = app.emit(
        INSTALL_PROGRESS_EVENT,
        InstallProgress::DownloadPhaseStarted {
            total_downloads,
            max_concurrent,
        },
    );

    let semaphore = Arc::new(Semaphore::new(max_concurrent));
    let mut handles = Vec::with_capacity(total_downloads);

    for &(order_pos, mod_idx) in &downloadable {
        let entry = manifest.mods[mod_idx].clone();
        let mod_name = entry.name.clone();

        let _ = app.emit(
            INSTALL_PROGRESS_EVENT,
            InstallProgress::DownloadQueued {
                mod_index: order_pos,
                mod_name: mod_name.clone(),
            },
        );

        // Clone what the spawned task needs
        let app_h = app.clone();
        let db_c = Arc::clone(db);
        let queue_c = Arc::clone(queue);
        let sem_c = Arc::clone(&semaphore);
        let game_slug_c = game_slug.to_string();
        let download_dir_c = download_dir.clone();
        let api_key_c = api_key.clone();
        let manifest_name_c = manifest.name.clone();
        let game_id_c = game_id.to_string();
        let bottle_name_c = bottle_name.to_string();

        let handle = tokio::spawn(async move {
            let _permit = sem_c.acquire().await.expect("semaphore closed");

            let _ = app_h.emit(
                INSTALL_PROGRESS_EVENT,
                InstallProgress::DownloadModStarted {
                    mod_index: order_pos,
                    mod_name: mod_name.clone(),
                },
            );

            let result = download_mod_archive(
                &app_h,
                &db_c,
                &queue_c,
                &entry,
                order_pos,
                &game_slug_c,
                &download_dir_c,
                &api_key_c,
                &manifest_name_c,
                &game_id_c,
                &bottle_name_c,
            )
            .await;

            match &result {
                Ok(dl) => {
                    let _ = app_h.emit(
                        INSTALL_PROGRESS_EVENT,
                        InstallProgress::DownloadModCompleted {
                            mod_index: order_pos,
                            mod_name: mod_name.clone(),
                            cached: dl.cached,
                        },
                    );
                }
                Err(InstallError::Failed(err)) => {
                    let _ = app_h.emit(
                        INSTALL_PROGRESS_EVENT,
                        InstallProgress::DownloadModFailed {
                            mod_index: order_pos,
                            mod_name: mod_name.clone(),
                            error: err.clone(),
                        },
                    );
                }
                Err(InstallError::UserAction { action, .. }) => {
                    let _ = app_h.emit(
                        INSTALL_PROGRESS_EVENT,
                        InstallProgress::DownloadModFailed {
                            mod_index: order_pos,
                            mod_name: mod_name.clone(),
                            error: action.clone(),
                        },
                    );
                }
            }

            (order_pos, result)
        });

        handles.push(handle);
    }

    // Wait for all download tasks
    let download_results = futures::future::join_all(handles).await;

    // Collect successful downloads into a map: order_position -> archive path
    let mut pre_downloaded: HashMap<usize, PathBuf> = HashMap::new();
    let mut dl_downloaded = 0usize;
    let mut dl_cached = 0usize;
    let mut dl_failed = 0usize;
    let dl_skipped = total_mods - total_downloads; // browse/manual/bundled/non-premium/already-installed

    for join_result in download_results {
        match join_result {
            Ok((order_pos, Ok(dl))) => {
                if dl.cached {
                    dl_cached += 1;
                } else {
                    dl_downloaded += 1;
                }
                pre_downloaded.insert(order_pos, dl.archive_path);
            }
            Ok((_order_pos, Err(_))) => {
                dl_failed += 1;
            }
            Err(join_err) => {
                log::error!("Download task panicked: {}", join_err);
                dl_failed += 1;
            }
        }
    }

    let _ = app.emit(
        INSTALL_PROGRESS_EVENT,
        InstallProgress::AllDownloadsCompleted {
            downloaded: dl_downloaded,
            cached: dl_cached,
            failed: dl_failed,
            skipped: dl_skipped,
        },
    );

    // ---------------------------------------------------------------
    // Phase 1.5: Concurrent Extraction
    // ---------------------------------------------------------------
    // Extract all pre-downloaded archives concurrently using dedicated
    // blocking threads. This is the biggest single speedup: archive
    // extraction is CPU+IO-bound and perfectly parallelizable.
    let max_extract = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(2, max_concurrent);

    // Collect archives that need extraction
    let archives_to_extract: Vec<(usize, PathBuf, String)> = install_order
        .iter()
        .enumerate()
        .filter_map(|(i, &mod_idx)| {
            if completed_statuses.contains_key(&i) {
                return None;
            }
            let entry = &manifest.mods[mod_idx];
            let is_already = if let Some(nexus_id) = entry.source.mod_id {
                existing_mods
                    .iter()
                    .any(|m| m.nexus_mod_id == Some(nexus_id))
            } else {
                existing_mods
                    .iter()
                    .any(|m| m.name.eq_ignore_ascii_case(&entry.name))
            };
            if is_already {
                return None;
            }
            pre_downloaded
                .get(&i)
                .map(|p| (i, p.clone(), entry.name.clone()))
        })
        .collect();

    let mut pre_extracted: HashMap<usize, PathBuf> = HashMap::new();

    if !archives_to_extract.is_empty() {
        let _ = app.emit(
            INSTALL_PROGRESS_EVENT,
            InstallProgress::StagingPhaseStarted {
                total_mods: archives_to_extract.len(),
                max_concurrent: max_extract,
            },
        );

        let extract_sem = Arc::new(Semaphore::new(max_extract));
        let mut extract_handles = Vec::with_capacity(archives_to_extract.len());

        for (install_idx, archive_path, mod_name) in &archives_to_extract {
            let sem = extract_sem.clone();
            let archive = archive_path.clone();
            let idx = *install_idx;
            let app_c = app.clone();
            let name = mod_name.clone();

            let handle = tokio::spawn(async move {
                let _permit = sem.acquire().await.expect("semaphore closed");

                let _ = app_c.emit(
                    INSTALL_PROGRESS_EVENT,
                    InstallProgress::StagingModStarted {
                        mod_index: idx,
                        mod_name: name.clone(),
                    },
                );

                let temp_dir = std::env::temp_dir()
                    .join(format!("corkscrew_extract_{}", idx));

                let result = tokio::task::spawn_blocking(move || {
                    if temp_dir.exists() {
                        let _ = std::fs::remove_dir_all(&temp_dir);
                    }
                    let _ = std::fs::create_dir_all(&temp_dir);
                    match crate::installer::extract_archive(&archive, &temp_dir) {
                        Ok(_) => Ok(temp_dir),
                        Err(e) => {
                            let _ = std::fs::remove_dir_all(&temp_dir);
                            Err(e.to_string())
                        }
                    }
                })
                .await;

                match result {
                    Ok(Ok(dir)) => {
                        let _ = app_c.emit(
                            INSTALL_PROGRESS_EVENT,
                            InstallProgress::StagingModCompleted {
                                mod_index: idx,
                                mod_name: name,
                            },
                        );
                        Some((idx, dir))
                    }
                    Ok(Err(e)) => {
                        log::warn!("Pre-extraction failed for mod {}: {}", idx, e);
                        None
                    }
                    Err(e) => {
                        log::warn!("Extraction task panicked for mod {}: {}", idx, e);
                        None
                    }
                }
            });

            extract_handles.push(handle);
        }

        // Collect extraction results
        let extract_results = futures::future::join_all(extract_handles).await;
        for join_result in extract_results {
            if let Ok(Some((idx, dir))) = join_result {
                pre_extracted.insert(idx, dir);
            }
        }

        log::info!(
            "Pre-extracted {}/{} archives concurrently",
            pre_extracted.len(),
            archives_to_extract.len()
        );
    }

    // ---------------------------------------------------------------
    // Phase 2: Sequential Install
    // ---------------------------------------------------------------
    let _ = app.emit(
        INSTALL_PROGRESS_EVENT,
        InstallProgress::InstallPhaseStarted { total_mods },
    );

    for (i, &mod_idx) in install_order.iter().enumerate() {
        let mod_entry = &manifest.mods[mod_idx];
        let mod_name = &mod_entry.name;

        // Check if this mod was already completed in a previous run (resume)
        if let Some(prev_status) = completed_statuses.get(&i) {
            match prev_status.as_str() {
                "installed" | "already_installed" | "skipped" | "user_action" => {
                    let _ = app.emit(
                        INSTALL_PROGRESS_EVENT,
                        InstallProgress::ModCompleted {
                            mod_index: i,
                            mod_name: mod_name.clone(),
                            mod_id: 0,
                        },
                    );
                    match prev_status.as_str() {
                        "installed" | "already_installed" => already_installed += 1,
                        "skipped" | "user_action" => skipped += 1,
                        _ => {}
                    }
                    details.push(ModInstallDetail {
                        name: mod_name.clone(),
                        status: prev_status.clone(),
                        error: None,
                        url: None,
                        instructions: None,
                    });
                    continue;
                }
                _ => {
                    // "failed" or "pending" — retry this mod
                }
            }
        }

        // Emit: mod started
        let _ = app.emit(
            INSTALL_PROGRESS_EVENT,
            InstallProgress::ModStarted {
                mod_index: i,
                total_mods,
                mod_name: mod_name.clone(),
            },
        );

        // Check if this mod is already installed (by Nexus mod ID or name match)
        let is_already = if let Some(nexus_id) = mod_entry.source.mod_id {
            existing_mods
                .iter()
                .any(|m| m.nexus_mod_id == Some(nexus_id))
        } else {
            existing_mods
                .iter()
                .any(|m| m.name.eq_ignore_ascii_case(mod_name))
        };

        if is_already {
            let _ = app.emit(
                INSTALL_PROGRESS_EVENT,
                InstallProgress::ModCompleted {
                    mod_index: i,
                    mod_name: mod_name.clone(),
                    mod_id: 0,
                },
            );
            already_installed += 1;
            let _ = db.update_checkpoint_mod_status(checkpoint_id, i, "already_installed");
            details.push(ModInstallDetail {
                name: mod_name.clone(),
                status: "already_installed".to_string(),
                error: None,
                url: None,
                instructions: None,
            });
            continue;
        }

        // Look up pre-downloaded archive and pre-extracted dir for this position
        let pre_dl = pre_downloaded.get(&i).map(|p| p.as_path());
        let pre_ext = pre_extracted.remove(&i);

        let result = install_single_mod(
            app,
            db,
            queue,
            mod_entry,
            i,
            game_id,
            bottle_name,
            game_slug,
            &data_dir,
            &download_dir,
            &api_key,
            is_premium,
            &manifest.name,
            pre_dl,
            pre_ext,
        )
        .await;

        match result {
            Ok(mod_id) => {
                // Tag this mod with the collection name
                let _ = db.set_collection_name(mod_id, &manifest.name);

                let _ = app.emit(
                    INSTALL_PROGRESS_EVENT,
                    InstallProgress::ModCompleted {
                        mod_index: i,
                        mod_name: mod_name.clone(),
                        mod_id,
                    },
                );
                installed += 1;
                let _ = db.update_checkpoint_mod_status(checkpoint_id, i, "installed");
                details.push(ModInstallDetail {
                    name: mod_name.clone(),
                    status: "installed".to_string(),
                    error: None,
                    url: None,
                    instructions: None,
                });
            }
            Err(InstallError::UserAction {
                action,
                url,
                instructions,
            }) => {
                let _ = app.emit(
                    INSTALL_PROGRESS_EVENT,
                    InstallProgress::UserActionRequired {
                        mod_index: i,
                        mod_name: mod_name.clone(),
                        action: action.clone(),
                        url: url.clone(),
                        instructions: instructions.clone(),
                    },
                );
                skipped += 1;
                let _ = db.update_checkpoint_mod_status(checkpoint_id, i, "user_action");
                details.push(ModInstallDetail {
                    name: mod_name.clone(),
                    status: "user_action".to_string(),
                    error: Some(action),
                    url,
                    instructions,
                });
            }
            Err(InstallError::Failed(error)) => {
                let _ = app.emit(
                    INSTALL_PROGRESS_EVENT,
                    InstallProgress::ModFailed {
                        mod_index: i,
                        mod_name: mod_name.clone(),
                        error: error.clone(),
                    },
                );
                failed += 1;
                let _ = db.update_checkpoint_mod_status(checkpoint_id, i, "failed");
                details.push(ModInstallDetail {
                    name: mod_name.clone(),
                    status: "failed".to_string(),
                    error: Some(error),
                    url: None,
                    instructions: None,
                });
            }
        }
    }

    // Clean up any remaining pre-extracted temp dirs (for skipped/failed mods)
    for (_idx, dir) in pre_extracted {
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Apply plugin load order from manifest (works for any game with plugin support)
    if !manifest.plugins.is_empty() {
        let has_plugin_support = games::with_plugin(game_id, |plugin| {
            plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
        })
        .flatten()
        .is_some();

        if has_plugin_support {
            apply_collection_plugin_order(&manifest.plugins, game, &bottle);
        }
    }

    // Sync plugins for games that support them
    {
        let game_path = Path::new(&game.game_path);
        let plugins_file = games::with_plugin(game_id, |plugin| {
            plugin.get_plugins_file(game_path, &bottle)
        })
        .flatten();

        if let Some(pf) = plugins_file {
            let loadorder_file = pf
                .parent()
                .map(|p| p.join("loadorder.txt"))
                .unwrap_or_else(|| pf.with_file_name("loadorder.txt"));
            let _ = plugins::skyrim_plugins::sync_plugins(
                Path::new(&game.data_dir),
                &pf,
                &loadorder_file,
            );
        }
    }

    // Save collection metadata for My Collections view + diff system
    let manifest_json = serde_json::to_string(&manifest).ok();
    let _ = db.save_collection_metadata(&database::CollectionMetadata {
        id: 0,
        collection_name: manifest.name.clone(),
        game_id: game_id.to_string(),
        bottle_name: bottle_name.to_string(),
        slug: manifest.slug.clone(),
        author: Some(manifest.author.clone()),
        description: Some(manifest.description.clone()),
        game_domain: Some(manifest.game_domain.clone()),
        image_url: manifest.image_url.clone(),
        installed_revision: manifest.revision,
        total_mods: Some(manifest.mods.len()),
        installed_at: chrono::Utc::now().to_rfc3339(),
        manifest_json,
    });

    // Auto-create a profile snapshot for the installed collection
    let profile_name = format!("{} (auto)", manifest.name);
    if let Ok(profile_id) = profiles::create_profile(db, game_id, bottle_name, &profile_name) {
        // Try to find plugins file for snapshot (any game with plugin support)
        let plugins_file = {
            let snapshot_bottle = bottles::detect_bottles()
                .into_iter()
                .find(|b| b.name == bottle_name);
            snapshot_bottle.and_then(|b| {
                games::with_plugin(game_id, |plugin| {
                    plugin.get_plugins_file(Path::new(&game.game_path), &b)
                })
                .flatten()
            })
        };
        let _ = profiles::snapshot_current_state(
            db,
            profile_id,
            game_id,
            bottle_name,
            plugins_file.as_deref(),
        );
        log::info!("Auto-created profile '{}' for collection", profile_name);
    }

    // Mark checkpoint as completed
    let _ = db.complete_checkpoint(checkpoint_id);

    // Emit collection completed summary
    let _ = app.emit(
        INSTALL_PROGRESS_EVENT,
        InstallProgress::CollectionCompleted {
            installed,
            skipped,
            failed,
        },
    );

    Ok(CollectionInstallResult {
        installed,
        already_installed,
        skipped,
        failed,
        details,
    })
}

enum InstallError {
    UserAction {
        action: String,
        url: Option<String>,
        instructions: Option<String>,
    },
    Failed(String),
}

/// Install a single mod from a collection entry.
///
/// If `pre_downloaded` is `Some`, the archive at that path is used directly
/// (skipping the download step).  Otherwise the mod is downloaded as before.
#[allow(clippy::too_many_arguments)]
async fn install_single_mod(
    app: &AppHandle,
    db: &Arc<ModDatabase>,
    queue: &Arc<DownloadQueue>,
    mod_entry: &CollectionModEntry,
    mod_index: usize,
    game_id: &str,
    bottle_name: &str,
    game_slug: &str,
    data_dir: &Path,
    download_dir: &Path,
    api_key: &Option<String>,
    is_premium: bool,
    manifest_name: &str,
    pre_downloaded: Option<&Path>,
    pre_extracted: Option<PathBuf>,
) -> Result<i64, InstallError> {
    let source_type = mod_entry.source.source_type.as_str();

    match source_type {
        "nexus" => {
            install_nexus_mod(
                app,
                db,
                queue,
                mod_entry,
                mod_index,
                game_id,
                bottle_name,
                game_slug,
                data_dir,
                download_dir,
                api_key,
                is_premium,
                manifest_name,
                pre_downloaded,
                pre_extracted,
            )
            .await
        }
        "direct" => {
            install_direct_mod(
                app,
                db,
                queue,
                mod_entry,
                mod_index,
                game_id,
                bottle_name,
                data_dir,
                download_dir,
                api_key,
                manifest_name,
                pre_downloaded,
                pre_extracted,
            )
            .await
        }
        "browse" | "manual" => Err(InstallError::UserAction {
            action: format!("Manual download required for '{}'", mod_entry.name),
            url: mod_entry.source.url.clone(),
            instructions: mod_entry
                .instructions
                .clone()
                .or_else(|| mod_entry.source.instructions.clone()),
        }),
        "bundle" | "bundled" => Err(InstallError::UserAction {
            action: format!(
                "'{}' is bundled in the collection archive. Download the full collection .7z from NexusMods to include bundled mods.",
                mod_entry.name
            ),
            url: None,
            instructions: Some(
                "This mod is embedded in the collection archive and cannot be downloaded separately."
                    .to_string(),
            ),
        }),
        _ => Err(InstallError::Failed(format!(
            "Unsupported source type '{}' for mod '{}'",
            source_type, mod_entry.name
        ))),
    }
}

/// Install a mod from Nexus Mods (premium only for API downloads).
///
/// When `pre_downloaded` is provided the download step is skipped entirely and
/// the archive at that path is used for staging + deployment.
#[allow(clippy::too_many_arguments)]
async fn install_nexus_mod(
    app: &AppHandle,
    db: &Arc<ModDatabase>,
    queue: &Arc<DownloadQueue>,
    mod_entry: &CollectionModEntry,
    mod_index: usize,
    game_id: &str,
    bottle_name: &str,
    game_slug: &str,
    data_dir: &Path,
    download_dir: &Path,
    api_key: &Option<String>,
    is_premium: bool,
    manifest_name: &str,
    pre_downloaded: Option<&Path>,
    pre_extracted: Option<PathBuf>,
) -> Result<i64, InstallError> {
    let nexus_mod_id = mod_entry
        .source
        .mod_id
        .ok_or_else(|| InstallError::Failed(format!("No Nexus mod ID for '{}'", mod_entry.name)))?;

    let nexus_file_id = mod_entry.source.file_id.ok_or_else(|| {
        InstallError::Failed(format!("No Nexus file ID for '{}'", mod_entry.name))
    })?;

    if !is_premium {
        // Free users: emit user action with NexusMods URL
        let url = format!(
            "https://www.nexusmods.com/{}/mods/{}?tab=files&file_id={}",
            game_slug, nexus_mod_id, nexus_file_id
        );
        return Err(InstallError::UserAction {
            action: format!(
                "Free users must download '{}' from NexusMods website",
                mod_entry.name
            ),
            url: Some(url),
            instructions: Some("Click 'Slow Download' on the NexusMods page, then install the downloaded archive via the Mods tab.".to_string()),
        });
    }

    // If we already have a pre-downloaded archive from Phase 1, skip straight
    // to staging + deployment.
    if let Some(archive) = pre_downloaded {
        let _ = app.emit(
            INSTALL_PROGRESS_EVENT,
            InstallProgress::StepChanged {
                mod_index,
                step: "pre-downloaded".to_string(),
                detail: Some(format!(
                    "Using pre-downloaded archive for '{}'",
                    mod_entry.name
                )),
            },
        );

        let mod_id = stage_and_deploy(
            app,
            db,
            archive,
            mod_entry,
            mod_index,
            game_id,
            bottle_name,
            data_dir,
            pre_extracted,
        )
        .await?;

        let _ = db.set_nexus_ids(mod_id, nexus_mod_id, Some(nexus_file_id));
        let _ = db.set_source_url(
            mod_id,
            &format!(
                "https://www.nexusmods.com/{}/mods/{}",
                game_slug, nexus_mod_id
            ),
        );
        return Ok(mod_id);
    }

    // Fallback: no pre-downloaded archive — download now (shouldn't normally
    // happen in the two-phase flow, but handles edge cases).

    // Check download registry for existing archive (dedup)
    if let Ok(Some(existing)) = db.find_download_by_nexus_ids(nexus_mod_id, nexus_file_id) {
        let path = std::path::Path::new(&existing.archive_path);
        if path.exists() {
            log::info!(
                "Reusing cached download for '{}' (nexus {}:{})",
                mod_entry.name,
                nexus_mod_id,
                nexus_file_id
            );
            let _ = app.emit(
                INSTALL_PROGRESS_EVENT,
                InstallProgress::StepChanged {
                    mod_index,
                    step: "cached".to_string(),
                    detail: Some(format!("Reusing cached download for '{}'", mod_entry.name)),
                },
            );

            let mod_id = stage_and_deploy(
                app,
                db,
                path,
                mod_entry,
                mod_index,
                game_id,
                bottle_name,
                data_dir,
                None,
            )
            .await?;

            let _ = db.set_nexus_ids(mod_id, nexus_mod_id, Some(nexus_file_id));
            let _ = db.set_source_url(
                mod_id,
                &format!(
                    "https://www.nexusmods.com/{}/mods/{}",
                    game_slug, nexus_mod_id
                ),
            );

            // Add collection ref for the reused download
            let _ =
                db.add_download_collection_ref(existing.id, manifest_name, game_id, bottle_name);

            return Ok(mod_id);
        }
    }

    let key = api_key
        .as_ref()
        .ok_or_else(|| InstallError::Failed("No API key configured".to_string()))?;

    let client = NexusClient::new(key.clone());

    // Step: get download links
    let _ = app.emit(
        INSTALL_PROGRESS_EVENT,
        InstallProgress::StepChanged {
            mod_index,
            step: "downloading".to_string(),
            detail: Some(format!(
                "Getting download links for '{}'...",
                mod_entry.name
            )),
        },
    );

    let links = client
        .get_download_links(game_slug, nexus_mod_id, nexus_file_id, None, None)
        .await
        .map_err(|e| InstallError::Failed(format!("Download links failed: {}", e)))?;

    let download_url = &links
        .first()
        .ok_or_else(|| {
            InstallError::Failed("No download links returned by NexusMods API".to_string())
        })?
        .uri;

    // Enqueue in download queue for tracking
    let queue_id = queue.enqueue(
        &mod_entry.name,
        download_url.rsplit('/').next().unwrap_or(&mod_entry.name),
        Some(nexus_mod_id),
        Some(nexus_file_id),
        None,
        Some(game_slug),
    );
    queue.set_downloading(queue_id);
    let _ = app.emit(DOWNLOAD_QUEUE_EVENT, queue.get_all());

    // Step: download with progress
    let last_emit = std::sync::Mutex::new(std::time::Instant::now());
    let app_clone = app.clone();
    let queue_clone = Arc::clone(queue);
    let archive_path = client
        .download_file(
            download_url,
            download_dir,
            Some(move |downloaded, total| {
                let mut last = last_emit.lock().unwrap_or_else(|e| e.into_inner());
                if last.elapsed().as_millis() >= 100 || downloaded == total {
                    queue_clone.set_progress(queue_id, downloaded, total);
                    let _ = app_clone.emit(
                        INSTALL_PROGRESS_EVENT,
                        InstallProgress::DownloadProgress {
                            mod_index,
                            downloaded,
                            total,
                        },
                    );
                    *last = std::time::Instant::now();
                }
            }),
        )
        .await
        .map_err(|e| {
            queue.set_failed(queue_id, &e.to_string());
            let _ = app.emit(DOWNLOAD_QUEUE_EVENT, queue.get_all());
            InstallError::Failed(format!("Download failed: {}", e))
        })?;

    queue.set_completed(queue_id);
    let _ = app.emit(DOWNLOAD_QUEUE_EVENT, queue.get_all());

    // Register download in dedup registry
    let archive_name = archive_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let sha = staging::compute_sha256(&archive_path).ok();
    let file_size = std::fs::metadata(&archive_path)
        .map(|m| m.len() as i64)
        .unwrap_or(0);
    if let Ok(dl_id) = db.register_download(
        &archive_path.to_string_lossy(),
        &archive_name,
        Some(nexus_mod_id),
        Some(nexus_file_id),
        sha.as_deref(),
        file_size,
    ) {
        let _ = db.add_download_collection_ref(dl_id, manifest_name, game_id, bottle_name);
    }

    // Stage and deploy the downloaded archive
    let mod_id = stage_and_deploy(
        app,
        db,
        &archive_path,
        mod_entry,
        mod_index,
        game_id,
        bottle_name,
        data_dir,
        None,
    )
    .await?;

    // Set Nexus IDs for tracking
    let _ = db.set_nexus_ids(mod_id, nexus_mod_id, Some(nexus_file_id));
    let _ = db.set_source_url(
        mod_id,
        &format!(
            "https://www.nexusmods.com/{}/mods/{}",
            game_slug, nexus_mod_id
        ),
    );

    Ok(mod_id)
}

/// Install a mod from a direct download URL.
///
/// When `pre_downloaded` is provided the download step is skipped entirely.
#[allow(clippy::too_many_arguments)]
async fn install_direct_mod(
    app: &AppHandle,
    db: &Arc<ModDatabase>,
    queue: &Arc<DownloadQueue>,
    mod_entry: &CollectionModEntry,
    mod_index: usize,
    game_id: &str,
    bottle_name: &str,
    data_dir: &Path,
    download_dir: &Path,
    api_key: &Option<String>,
    manifest_name: &str,
    pre_downloaded: Option<&Path>,
    pre_extracted: Option<PathBuf>,
) -> Result<i64, InstallError> {
    // If we already have a pre-downloaded archive from Phase 1, skip straight
    // to staging + deployment.
    if let Some(archive) = pre_downloaded {
        let _ = app.emit(
            INSTALL_PROGRESS_EVENT,
            InstallProgress::StepChanged {
                mod_index,
                step: "pre-downloaded".to_string(),
                detail: Some(format!(
                    "Using pre-downloaded archive for '{}'",
                    mod_entry.name
                )),
            },
        );

        let mod_id = stage_and_deploy(
            app,
            db,
            archive,
            mod_entry,
            mod_index,
            game_id,
            bottle_name,
            data_dir,
            pre_extracted,
        )
        .await?;

        if let Some(ref source_url) = mod_entry.source.url {
            let _ = db.set_source_url(mod_id, source_url);
        }
        return Ok(mod_id);
    }

    // Fallback: no pre-downloaded archive — download now.
    let url =
        mod_entry.source.url.as_ref().ok_or_else(|| {
            InstallError::Failed(format!("No download URL for '{}'", mod_entry.name))
        })?;

    // Check download registry for existing archive (dedup by filename)
    let url_filename = url
        .rsplit('/')
        .next()
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("");
    if !url_filename.is_empty() {
        if let Ok(Some(existing)) = db.find_download_by_name(url_filename) {
            let path = std::path::Path::new(&existing.archive_path);
            if path.exists() {
                log::info!(
                    "Reusing cached download for '{}' ({})",
                    mod_entry.name,
                    url_filename
                );
                let _ = app.emit(
                    INSTALL_PROGRESS_EVENT,
                    InstallProgress::StepChanged {
                        mod_index,
                        step: "cached".to_string(),
                        detail: Some(format!("Reusing cached download for '{}'", mod_entry.name)),
                    },
                );

                let mod_id = stage_and_deploy(
                    app,
                    db,
                    path,
                    mod_entry,
                    mod_index,
                    game_id,
                    bottle_name,
                    data_dir,
                    None,
                )
                .await?;

                if let Some(ref source_url) = mod_entry.source.url {
                    let _ = db.set_source_url(mod_id, source_url);
                }

                return Ok(mod_id);
            }
        }
    }

    let _ = app.emit(
        INSTALL_PROGRESS_EVENT,
        InstallProgress::StepChanged {
            mod_index,
            step: "downloading".to_string(),
            detail: Some(format!("Downloading '{}'...", mod_entry.name)),
        },
    );

    // Enqueue in download queue for tracking
    let queue_id = queue.enqueue(
        &mod_entry.name,
        url_filename,
        mod_entry.source.mod_id,
        mod_entry.source.file_id,
        Some(url),
        None,
    );
    queue.set_downloading(queue_id);
    let _ = app.emit(DOWNLOAD_QUEUE_EVENT, queue.get_all());

    // Use NexusClient for HTTP downloads (it has a proper user-agent)
    let dummy_key = api_key.as_deref().unwrap_or("").to_string();
    let client = NexusClient::new(dummy_key);

    let app_clone = app.clone();
    let last_emit = std::sync::Mutex::new(std::time::Instant::now());
    let queue_clone = Arc::clone(queue);
    let archive_path = client
        .download_file(
            url,
            download_dir,
            Some(move |downloaded, total| {
                let mut last = last_emit.lock().unwrap_or_else(|e| e.into_inner());
                if last.elapsed().as_millis() >= 100 || downloaded == total {
                    queue_clone.set_progress(queue_id, downloaded, total);
                    let _ = app_clone.emit(
                        INSTALL_PROGRESS_EVENT,
                        InstallProgress::DownloadProgress {
                            mod_index,
                            downloaded,
                            total,
                        },
                    );
                    *last = std::time::Instant::now();
                }
            }),
        )
        .await
        .map_err(|e| {
            queue.set_failed(queue_id, &e.to_string());
            let _ = app.emit(DOWNLOAD_QUEUE_EVENT, queue.get_all());
            InstallError::Failed(format!("Download failed: {}", e))
        })?;

    queue.set_completed(queue_id);
    let _ = app.emit(DOWNLOAD_QUEUE_EVENT, queue.get_all());

    // Register download in dedup registry
    let archive_name = archive_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let sha = staging::compute_sha256(&archive_path).ok();
    let file_size = std::fs::metadata(&archive_path)
        .map(|m| m.len() as i64)
        .unwrap_or(0);
    if let Ok(dl_id) = db.register_download(
        &archive_path.to_string_lossy(),
        &archive_name,
        mod_entry.source.mod_id,
        mod_entry.source.file_id,
        sha.as_deref(),
        file_size,
    ) {
        let _ = db.add_download_collection_ref(dl_id, manifest_name, game_id, bottle_name);
    }

    let mod_id = stage_and_deploy(
        app,
        db,
        &archive_path,
        mod_entry,
        mod_index,
        game_id,
        bottle_name,
        data_dir,
        None,
    )
    .await?;

    if let Some(ref source_url) = mod_entry.source.url {
        let _ = db.set_source_url(mod_id, source_url);
    }

    Ok(mod_id)
}

/// Common staging and deployment pipeline for an already-downloaded archive.
///
/// Runs extraction and deployment on blocking threads via `spawn_blocking`
/// so the tokio runtime stays free for other Tauri commands (app remains
/// usable during installs). When `pre_extracted` is provided, the archive
/// extraction step is skipped entirely.
#[allow(clippy::too_many_arguments)]
async fn stage_and_deploy(
    app: &AppHandle,
    db: &Arc<ModDatabase>,
    archive_path: &Path,
    mod_entry: &CollectionModEntry,
    mod_index: usize,
    game_id: &str,
    bottle_name: &str,
    data_dir: &Path,
    pre_extracted: Option<PathBuf>,
) -> Result<i64, InstallError> {
    let mod_name = &mod_entry.name;

    // Step: extract
    let _ = app.emit(
        INSTALL_PROGRESS_EVENT,
        InstallProgress::StepChanged {
            mod_index,
            step: if pre_extracted.is_some() {
                "staging".to_string()
            } else {
                "extracting".to_string()
            },
            detail: Some(format!(
                "{} '{}'...",
                if pre_extracted.is_some() {
                    "Staging"
                } else {
                    "Extracting"
                },
                mod_name
            )),
        },
    );

    // Reserve DB record (brief lock, fine on tokio thread)
    let next_priority = db
        .get_next_priority(game_id, bottle_name)
        .map_err(|e| InstallError::Failed(e.to_string()))?;
    let mod_id = db
        .add_mod(
            game_id,
            bottle_name,
            None,
            mod_name,
            &mod_entry.version,
            &archive_path.to_string_lossy(),
            &[],
        )
        .map_err(|e| InstallError::Failed(e.to_string()))?;
    db.set_mod_priority(mod_id, next_priority)
        .map_err(|e| InstallError::Failed(e.to_string()))?;

    // Stage: heavy I/O — run on a dedicated blocking thread so the tokio
    // runtime remains free for other commands (app stays usable).
    let staging_result = {
        let archive = archive_path.to_path_buf();
        let gid = game_id.to_string();
        let bn = bottle_name.to_string();
        let mn = mod_name.clone();

        tokio::task::spawn_blocking(move || {
            if let Some(extracted_dir) = pre_extracted {
                let result =
                    staging::stage_mod_from_extracted(&extracted_dir, &gid, &bn, mod_id, &mn);
                // Clean up pre-extracted temp dir
                let _ = std::fs::remove_dir_all(&extracted_dir);
                result
            } else {
                staging::stage_mod(&archive, &gid, &bn, mod_id, &mn)
            }
        })
        .await
        .map_err(|e| {
            let _ = db.remove_mod(mod_id);
            InstallError::Failed(format!("Staging join error: {}", e))
        })?
        .map_err(|e| {
            let _ = db.remove_mod(mod_id);
            InstallError::Failed(format!("Staging failed: {}", e))
        })?
    };

    // Handle FOMOD if present and manifest provides choices
    let files_to_deploy =
        if let Ok(Some(fomod_installer)) = fomod::parse_fomod(&staging_result.staging_path) {
            let selections = if let Some(ref choices) = mod_entry.choices {
                parse_fomod_choices(choices)
                    .unwrap_or_else(|| fomod::get_default_selections(&fomod_installer))
            } else {
                fomod::get_default_selections(&fomod_installer)
            };

            let fomod_files = fomod::get_files_for_selections(&fomod_installer, &selections);

            apply_fomod_to_staging(&staging_result.staging_path, &fomod_files)
                .unwrap_or(staging_result.files.clone())
        } else {
            staging_result.files.clone()
        };

    // Apply collection patches (BSDiff) if any
    if let Some(ref patches) = mod_entry.patches {
        let mut patch_failures = 0u32;
        for (rel_path, b64_patch) in patches {
            let file_path = staging_result.staging_path.join(rel_path);
            if !file_path.exists() {
                log::warn!("Collection patch target not found: {}", file_path.display());
                patch_failures += 1;
                continue;
            }
            let patch_bytes = match base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                b64_patch,
            ) {
                Ok(b) => b,
                Err(e) => {
                    log::warn!("Failed to decode patch for {}: {}", rel_path, e);
                    patch_failures += 1;
                    continue;
                }
            };
            let source_data = match std::fs::read(&file_path) {
                Ok(d) => d,
                Err(e) => {
                    log::warn!("Failed to read patch target {}: {}", rel_path, e);
                    patch_failures += 1;
                    continue;
                }
            };
            match qbsdiff::Bspatch::new(&patch_bytes) {
                Ok(patcher) => {
                    let target_size = patcher.hint_target_size() as usize;
                    let mut target_data = Vec::with_capacity(target_size);
                    if let Err(e) = patcher.apply(&source_data, &mut target_data) {
                        log::warn!("Failed to apply patch for {}: {}", rel_path, e);
                        patch_failures += 1;
                        continue;
                    }
                    if let Err(e) = std::fs::write(&file_path, &target_data) {
                        log::warn!("Failed to write patched file {}: {}", rel_path, e);
                        patch_failures += 1;
                    }
                }
                Err(e) => {
                    log::warn!("Invalid BSDiff patch for {}: {}", rel_path, e);
                    patch_failures += 1;
                }
            }
        }
        if patch_failures > 0 {
            log::warn!(
                "{} of {} patches failed for mod '{}'",
                patch_failures,
                patches.len(),
                mod_name
            );
            let _ = app.emit(
                INSTALL_PROGRESS_EVENT,
                InstallProgress::StepChanged {
                    mod_index,
                    step: "warning".to_string(),
                    detail: Some(format!(
                        "{} patch(es) failed to apply for '{}'",
                        patch_failures, mod_name
                    )),
                },
            );
        }
    }

    // Apply file overrides — when non-empty, only deploy listed files
    let files_to_deploy = if !mod_entry.file_overrides.is_empty() {
        files_to_deploy
            .into_iter()
            .filter(|f| {
                let staging_prefix = staging_result.staging_path.to_string_lossy();
                let rel = f
                    .strip_prefix(staging_prefix.as_ref())
                    .unwrap_or(f)
                    .trim_start_matches('/')
                    .trim_start_matches('\\')
                    .replace('\\', "/");
                mod_entry.file_overrides.iter().any(|ov| {
                    let norm_ov = ov.replace('\\', "/");
                    rel == norm_ov || rel.ends_with(&format!("/{}", norm_ov))
                })
            })
            .collect()
    } else {
        files_to_deploy
    };

    // Update DB with staging info (brief locks)
    let _ = app.emit(
        INSTALL_PROGRESS_EVENT,
        InstallProgress::StepChanged {
            mod_index,
            step: "registering".to_string(),
            detail: Some(format!("Recording {} files...", files_to_deploy.len())),
        },
    );

    db.set_staging_path(mod_id, &staging_result.staging_path.to_string_lossy())
        .map_err(|e| InstallError::Failed(e.to_string()))?;
    db.update_installed_files(mod_id, &files_to_deploy)
        .map_err(|e| InstallError::Failed(e.to_string()))?;
    db.store_file_hashes(mod_id, &staging_result.hashes)
        .map_err(|e| InstallError::Failed(e.to_string()))?;

    // Deploy: heavy I/O — run on a blocking thread
    let _ = app.emit(
        INSTALL_PROGRESS_EVENT,
        InstallProgress::StepChanged {
            mod_index,
            step: "deploying".to_string(),
            detail: Some(format!("Deploying '{}' to game...", mod_name)),
        },
    );

    {
        let db_c = Arc::clone(db);
        let gid = game_id.to_string();
        let bn = bottle_name.to_string();
        let sp = staging_result.staging_path.clone();
        let dd = data_dir.to_path_buf();
        let files = files_to_deploy.clone();

        let deploy_result = tokio::task::spawn_blocking(move || {
            deployer::deploy_mod(&db_c, &gid, &bn, mod_id, &sp, &dd, &files)
        })
        .await
        .map_err(|e| InstallError::Failed(format!("Deploy join error: {}", e)))?;

        if let Err(e) = deploy_result {
            let _ = staging::remove_staging(&staging_result.staging_path);
            let _ = db.remove_mod(mod_id);
            return Err(InstallError::Failed(format!("Deploy failed: {}", e)));
        }
    }

    Ok(mod_id)
}

/// Parse FOMOD choices from a collection manifest's JSON value into the
/// HashMap format expected by `get_files_for_selections`.
fn parse_fomod_choices(
    choices: &serde_json::Value,
) -> Option<std::collections::HashMap<String, Vec<String>>> {
    let obj = choices.as_object()?;
    let mut result = std::collections::HashMap::new();
    for (key, value) in obj {
        if let Some(arr) = value.as_array() {
            let selections: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
            result.insert(key.clone(), selections);
        }
    }
    Some(result)
}

/// Check if a path component is safe (no traversal or absolute paths).
fn is_safe_relative_path(path: &str) -> bool {
    !path.contains("..")
        && !path.starts_with('/')
        && !path.starts_with('\\')
        && !path.contains(":/")
        && !path.contains(":\\")
}

/// Apply FOMOD selections to staging by returning the list of files to deploy.
fn apply_fomod_to_staging(
    staging_path: &Path,
    fomod_files: &[fomod::FomodFile],
) -> Option<Vec<String>> {
    let mut files = Vec::new();
    for f in fomod_files {
        // Validate source and destination paths to prevent path traversal
        if !is_safe_relative_path(&f.source) {
            log::warn!("Skipping FOMOD file with unsafe source path: {}", f.source);
            continue;
        }
        if !f.destination.is_empty() && !is_safe_relative_path(&f.destination) {
            log::warn!(
                "Skipping FOMOD file with unsafe destination path: {}",
                f.destination
            );
            continue;
        }

        let src = staging_path.join(&f.source);
        if f.is_folder {
            // Recursively walk the folder and add all files
            if src.is_dir() {
                for entry in walkdir::WalkDir::new(&src)
                    .into_iter()
                    .filter_map(|e| e.ok())
                {
                    if entry.file_type().is_file() {
                        if let Ok(rel) = entry.path().strip_prefix(&src) {
                            let dest = if f.destination.is_empty() {
                                rel.to_string_lossy().to_string()
                            } else {
                                format!("{}/{}", f.destination, rel.to_string_lossy())
                            };
                            files.push(dest);
                        }
                    }
                }
            }
        } else if src.exists() {
            let dest = if f.destination.is_empty() {
                f.source.clone()
            } else {
                f.destination.clone()
            };
            files.push(dest);
        }
    }

    if files.is_empty() {
        None
    } else {
        Some(files)
    }
}

/// Apply the plugin load order from a collection manifest.
fn apply_collection_plugin_order(
    collection_plugins: &[collections::CollectionPlugin],
    game: &games::DetectedGame,
    bottle: &bottles::Bottle,
) {
    let game_path = Path::new(&game.game_path);

    let plugins_file = games::with_plugin(&game.game_id, |plugin| {
        plugin.get_plugins_file(game_path, bottle)
    })
    .flatten();

    if let Some(pf) = plugins_file {
        let loadorder_file = pf
            .parent()
            .map(|p| p.join("loadorder.txt"))
            .unwrap_or_else(|| pf.with_file_name("loadorder.txt"));

        let entries: Vec<plugins::skyrim_plugins::PluginEntry> = collection_plugins
            .iter()
            .map(|p| plugins::skyrim_plugins::PluginEntry {
                filename: p.name.clone(),
                enabled: p.enabled,
            })
            .collect();

        let _ = plugins::skyrim_plugins::apply_load_order(&pf, &loadorder_file, &entries);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_fomod_choices_valid() {
        let json = serde_json::json!({
            "Textures": ["4K", "Parallax"],
            "Patches": ["USSEP Patch"]
        });
        let result = parse_fomod_choices(&json).unwrap();
        assert_eq!(result["Textures"], vec!["4K", "Parallax"]);
        assert_eq!(result["Patches"], vec!["USSEP Patch"]);
    }

    #[test]
    fn test_parse_fomod_choices_empty() {
        let json = serde_json::json!({});
        let result = parse_fomod_choices(&json).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_fomod_choices_non_object() {
        let json = serde_json::json!("not an object");
        assert!(parse_fomod_choices(&json).is_none());
    }
}
