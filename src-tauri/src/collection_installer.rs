use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

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

/// Install a NexusMods collection into the given game/bottle.
///
/// This orchestrates the full pipeline: resolve order, download (premium only),
/// extract, stage, deploy, and sync plugins.
pub async fn install_collection(
    app: &AppHandle,
    db: &Arc<ModDatabase>,
    queue: &Arc<DownloadQueue>,
    manifest: &CollectionManifest,
    game_id: &str,
    bottle_name: &str,
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

    // Load existing mods for already-installed detection
    let existing_mods = db.list_mods(game_id, bottle_name).unwrap_or_default();

    for (i, &mod_idx) in install_order.iter().enumerate() {
        let mod_entry = &manifest.mods[mod_idx];
        let mod_name = &mod_entry.name;

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
            details.push(ModInstallDetail {
                name: mod_name.clone(),
                status: "already_installed".to_string(),
                error: None,
                url: None,
                instructions: None,
            });
            continue;
        }

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

    // Apply plugin load order from manifest
    if !manifest.plugins.is_empty() && game_id == "skyrimse" {
        apply_collection_plugin_order(&manifest.plugins, game, &bottle);
    }

    // Sync plugins if Skyrim SE
    if game_id == "skyrimse" {
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
        // Try to find plugins file for snapshot (Skyrim SE only)
        let plugins_file = if game_id == "skyrimse" {
            let bottle = bottles::detect_bottles()
                .into_iter()
                .find(|b| b.name == bottle_name);
            bottle.and_then(|b| {
                games::with_plugin(game_id, |plugin| {
                    plugin.get_plugins_file(Path::new(&game.game_path), &b)
                })
                .flatten()
            })
        } else {
            None
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
            )?;

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
    )?;

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
) -> Result<i64, InstallError> {
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
                )?;

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
    )?;

    if let Some(ref source_url) = mod_entry.source.url {
        let _ = db.set_source_url(mod_id, source_url);
    }

    Ok(mod_id)
}

/// Common staging and deployment pipeline for an already-downloaded archive.
#[allow(clippy::too_many_arguments)]
fn stage_and_deploy(
    app: &AppHandle,
    db: &Arc<ModDatabase>,
    archive_path: &Path,
    mod_entry: &CollectionModEntry,
    mod_index: usize,
    game_id: &str,
    bottle_name: &str,
    data_dir: &Path,
) -> Result<i64, InstallError> {
    let mod_name = &mod_entry.name;

    // Step: extract
    let _ = app.emit(
        INSTALL_PROGRESS_EVENT,
        InstallProgress::StepChanged {
            mod_index,
            step: "extracting".to_string(),
            detail: Some(format!("Extracting '{}'...", mod_name)),
        },
    );

    // Reserve DB record
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

    // Stage
    let staging_result = staging::stage_mod(archive_path, game_id, bottle_name, mod_id, mod_name)
        .map_err(|e| {
        let _ = db.remove_mod(mod_id);
        InstallError::Failed(format!("Staging failed: {}", e))
    })?;

    // Handle FOMOD if present and manifest provides choices
    let files_to_deploy =
        if let Ok(Some(fomod_installer)) = fomod::parse_fomod(&staging_result.staging_path) {
            // If the manifest has FOMOD choices, use them; otherwise use defaults
            let selections = if let Some(ref choices) = mod_entry.choices {
                // Convert manifest choices (JSON value) to HashMap<String, Vec<String>>
                parse_fomod_choices(choices)
                    .unwrap_or_else(|| fomod::get_default_selections(&fomod_installer))
            } else {
                fomod::get_default_selections(&fomod_installer)
            };

            let fomod_files = fomod::get_files_for_selections(&fomod_installer, &selections);

            // Copy FOMOD-selected files into a clean staging area

            apply_fomod_to_staging(&staging_result.staging_path, &fomod_files)
                .unwrap_or(staging_result.files.clone())
        } else {
            staging_result.files.clone()
        };

    // Update DB with staging info
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

    // Deploy
    let _ = app.emit(
        INSTALL_PROGRESS_EVENT,
        InstallProgress::StepChanged {
            mod_index,
            step: "deploying".to_string(),
            detail: Some(format!("Deploying '{}' to game...", mod_name)),
        },
    );

    if let Err(e) = deployer::deploy_mod(
        db,
        game_id,
        bottle_name,
        mod_id,
        &staging_result.staging_path,
        data_dir,
        &files_to_deploy,
    ) {
        let _ = staging::remove_staging(&staging_result.staging_path);
        let _ = db.remove_mod(mod_id);
        return Err(InstallError::Failed(format!("Deploy failed: {}", e)));
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
