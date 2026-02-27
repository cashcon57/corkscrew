use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use once_cell::sync::Lazy;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use std::collections::HashSet;
use tokio::sync::oneshot;
use tokio::sync::{Notify, Semaphore};

/// Global cancellation flag for collection installs.
static INSTALL_CANCELLED: AtomicBool = AtomicBool::new(false);

/// FOMOD selection channel type alias.
type FomodSelections = HashMap<String, Vec<String>>;

/// Registry of pending FOMOD selections from the frontend.
static FOMOD_PENDING: Lazy<std::sync::Mutex<HashMap<String, oneshot::Sender<FomodSelections>>>> =
    Lazy::new(|| std::sync::Mutex::new(HashMap::new()));

/// Request cancellation of the currently running collection install.
pub fn cancel_install() {
    INSTALL_CANCELLED.store(true, Ordering::SeqCst);
    // Drain any pending FOMOD wizard channels so blocked tasks unblock immediately
    drain_fomod_pending();
}

/// Check if install has been cancelled.
fn is_cancelled() -> bool {
    INSTALL_CANCELLED.load(Ordering::SeqCst)
}

/// Create a FOMOD request channel and return a correlation ID + receiver.
fn create_fomod_request() -> (String, oneshot::Receiver<FomodSelections>) {
    let id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = oneshot::channel();
    FOMOD_PENDING.lock().unwrap().insert(id.clone(), tx);
    (id, rx)
}

/// Submit FOMOD choices from the frontend for a pending request.
pub fn submit_fomod_choices(
    correlation_id: &str,
    selections: FomodSelections,
) -> Result<(), String> {
    let tx = FOMOD_PENDING
        .lock()
        .unwrap()
        .remove(correlation_id)
        .ok_or_else(|| format!("No pending FOMOD request: {}", correlation_id))?;
    tx.send(selections)
        .map_err(|_| "FOMOD receiver dropped".to_string())
}

/// Drain all pending FOMOD channels (used on cancellation).
fn drain_fomod_pending() {
    let mut pending = FOMOD_PENDING.lock().unwrap();
    for (_, tx) in pending.drain() {
        drop(tx);
    }
}

use crate::bottles;
use crate::collections::{self, CollectionManifest, CollectionModEntry};
use crate::config;
use crate::database::{self, ModDatabase};
use crate::conflict_resolver;
use crate::deployer;
use crate::disk_budget;
use crate::download_queue::{DownloadQueue, DOWNLOAD_QUEUE_EVENT};
use crate::fomod;
use crate::games;
use crate::nexus::NexusClient;
use crate::oauth;
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
#[derive(Debug)]
struct DownloadResult {
    archive_path: PathBuf,
    cached: bool,
}

/// Guard that cleans up tracked temp directories on drop.
/// Paths that are successfully consumed should be removed via `take()`.
struct TempDirGuard(Vec<PathBuf>);

impl TempDirGuard {
    fn new() -> Self {
        Self(Vec::new())
    }

    fn track(&mut self, path: PathBuf) {
        self.0.push(path);
    }

    /// Remove a path from the guard (it was consumed successfully).
    fn untrack(&mut self, path: &Path) {
        self.0.retain(|p| p != path);
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        for dir in &self.0 {
            if dir.exists() {
                log::info!("Cleaning up orphaned temp dir: {:?}", dir);
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }
}

/// Quick-check that an archive file has valid headers before extraction.
fn validate_archive(path: &Path) -> Result<(), String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "zip" | "fomod" => {
            let file = std::fs::File::open(path)
                .map_err(|e| format!("Cannot open archive: {}", e))?;
            let _ = zip::ZipArchive::new(file)
                .map_err(|e| format!("Invalid ZIP archive: {}", e))?;
            Ok(())
        }
        "7z" => {
            // Just verify the file is readable and non-empty.
            let meta = std::fs::metadata(path)
                .map_err(|e| format!("Cannot read archive: {}", e))?;
            if meta.len() == 0 {
                return Err("Archive is empty (0 bytes)".into());
            }
            Ok(())
        }
        _ => Ok(()),
    }
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

/// Maximum retry attempts for transient collection download failures.
const COLLECTION_DOWNLOAD_RETRIES: u32 = 5;
/// Base delay in milliseconds for exponential backoff.
const COLLECTION_RETRY_BASE_MS: u64 = 3000;

/// Download a mod archive without extracting or deploying it.
///
/// Checks the dedup cache first; if found and the archive still exists on disk,
/// returns the cached path.  Otherwise performs the actual download and registers
/// the result in the dedup registry.  Retries transient network failures up to
/// 3 times with exponential backoff.
#[allow(clippy::too_many_arguments)]
async fn download_mod_archive(
    app: &AppHandle,
    db: &Arc<ModDatabase>,
    queue: &Arc<DownloadQueue>,
    mod_entry: &CollectionModEntry,
    mod_index: usize,
    game_slug: &str,
    download_dir: &Path,
    auth_method: &oauth::AuthMethod,
    manifest_name: &str,
    game_id: &str,
    bottle_name: &str,
) -> Result<DownloadResult, InstallError> {
    let source_type = mod_entry.source.source_type.as_str();

    let mut last_err = None;
    for attempt in 0..COLLECTION_DOWNLOAD_RETRIES {
        if attempt > 0 {
            let delay = COLLECTION_RETRY_BASE_MS * (1 << (attempt - 1));
            log::warn!(
                "Retry {}/{} for '{}' after {}ms",
                attempt,
                COLLECTION_DOWNLOAD_RETRIES,
                mod_entry.name,
                delay,
            );
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
        }

        let result = match source_type {
            "nexus" => {
                download_nexus_archive(
                    app,
                    db,
                    queue,
                    mod_entry,
                    mod_index,
                    game_slug,
                    download_dir,
                    auth_method,
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
                    auth_method,
                    manifest_name,
                    game_id,
                    bottle_name,
                )
                .await
            }
            _ => {
                return Err(InstallError::Failed(format!(
                    "Cannot pre-download source type '{}' for '{}'",
                    source_type, mod_entry.name
                )));
            }
        };

        match result {
            Ok(dl) => return Ok(dl),
            Err(ref e) if is_transient_collection_error(e) && attempt + 1 < COLLECTION_DOWNLOAD_RETRIES => {
                log::warn!("Transient download error for '{}': {:?}", mod_entry.name, e);
                last_err = Some(result.unwrap_err());
                continue;
            }
            Err(e) => return Err(e),
        }
    }

    Err(last_err.unwrap_or_else(|| InstallError::Failed("max retries exhausted".into())))
}

/// Returns true for transient errors worth retrying (network/IO failures).
/// Only permanent errors (404, 403, missing IDs, unsupported source types) are
/// NOT retried.  Everything else is assumed transient — this is deliberately
/// permissive because it's better to retry a few extra times than to stall a
/// 500-mod collection install on a single CDN hiccup.
fn is_transient_collection_error(e: &InstallError) -> bool {
    match e {
        InstallError::Failed(msg) => {
            // Never retry permanent HTTP errors or missing-resource errors
            let permanent = msg.contains("(404)")
                || msg.contains("(403)")
                || msg.contains("No Mod Found")
                || msg.contains("No Nexus mod ID")
                || msg.contains("No Nexus file ID")
                || msg.contains("No download URL")
                || msg.contains("Cannot pre-download source type")
                || msg.contains("No NexusMods auth configured")
                || msg.contains("premium");
            !permanent
        }
        _ => false,
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
    auth_method: &oauth::AuthMethod,
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
            // Verify file size matches the registered size to catch truncated/corrupt files.
            let size_ok = if existing.file_size > 0 {
                std::fs::metadata(&path)
                    .map(|m| m.len() as i64 == existing.file_size)
                    .unwrap_or(false)
            } else {
                true // No size recorded, trust the file exists
            };
            if size_ok {
                log::info!(
                    "Reusing cached download for '{}' (nexus {}:{})",
                    mod_entry.name,
                    nexus_mod_id,
                    nexus_file_id
                );
                let _ = db.add_download_collection_ref(
                    existing.id,
                    manifest_name,
                    game_id,
                    bottle_name,
                );
                return Ok(DownloadResult {
                    archive_path: path,
                    cached: true,
                });
            } else {
                log::warn!(
                    "Cached download for '{}' has wrong file size, will re-download",
                    mod_entry.name
                );
            }
        } else {
            log::info!(
                "Cached file missing from disk for '{}', will re-download",
                mod_entry.name
            );
        }
    }

    let client = NexusClient::from_auth_method(auth_method)
        .map_err(|e| InstallError::Failed(format!("No NexusMods auth configured: {}", e)))?;

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

    // Register in dedup registry — skip SHA-256 hashing during collection
    // installs to avoid blocking the download+extract pipeline.  The dedup
    // registry already uses filename + file_size + nexus IDs for matching.
    let archive_name = archive_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let file_size = std::fs::metadata(&archive_path)
        .map(|m| m.len() as i64)
        .unwrap_or(0);
    if let Ok(dl_id) = db.register_download(
        &archive_path.to_string_lossy(),
        &archive_name,
        Some(nexus_mod_id),
        Some(nexus_file_id),
        None,
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
    auth_method: &oauth::AuthMethod,
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
                // Verify file size matches the registered size.
                let size_ok = if existing.file_size > 0 {
                    std::fs::metadata(&path)
                        .map(|m| m.len() as i64 == existing.file_size)
                        .unwrap_or(false)
                } else {
                    true
                };
                if size_ok {
                    log::info!(
                        "Reusing cached download for '{}' ({})",
                        mod_entry.name,
                        url_filename
                    );
                    return Ok(DownloadResult {
                        archive_path: path,
                        cached: true,
                    });
                } else {
                    log::warn!(
                        "Cached download for '{}' has wrong file size, will re-download",
                        mod_entry.name
                    );
                }
            } else {
                log::info!(
                    "Cached file missing from disk for '{}', will re-download",
                    mod_entry.name
                );
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

    // Download — for direct URLs we just need a client with proper headers
    let client = NexusClient::from_auth_method(auth_method)
        .unwrap_or_else(|_| NexusClient::new(String::new()));

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

    // Register in dedup registry — skip SHA-256 to avoid blocking pipeline
    let archive_name = archive_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let file_size = std::fs::metadata(&archive_path)
        .map(|m| m.len() as i64)
        .unwrap_or(0);
    if let Ok(dl_id) = db.register_download(
        &archive_path.to_string_lossy(),
        &archive_name,
        mod_entry.source.mod_id,
        mod_entry.source.file_id,
        None,
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
    // Reset cancellation flag at the start of each install
    INSTALL_CANCELLED.store(false, Ordering::SeqCst);

    // Emit initialization progress so the UI shows something immediately
    let _ = app.emit(
        INSTALL_PROGRESS_EVENT,
        InstallProgress::Initializing {
            message: "Resolving game and bottle paths...".to_string(),
        },
    );

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

    // Check premium status once upfront — supports both OAuth and API key auth
    let _ = app.emit(
        INSTALL_PROGRESS_EVENT,
        InstallProgress::Initializing {
            message: "Checking NexusMods account status...".to_string(),
        },
    );
    let auth_method = oauth::get_auth_method_refreshed().await;

    let is_premium = match &auth_method {
        oauth::AuthMethod::ApiKey(key) => {
            let client = NexusClient::new(key.clone());
            client.is_premium().await
        }
        oauth::AuthMethod::OAuth(tokens) => {
            oauth::parse_user_info(&tokens.access_token)
                .map(|u| u.is_premium)
                .unwrap_or(false)
        }
        oauth::AuthMethod::None => false,
    };

    // Download collection bundle to get real FOMOD choices, patches, and mod rules
    let manifest = if let (Some(slug), Some(revision)) = (&manifest.slug, manifest.revision) {
        let _ = app.emit(
            INSTALL_PROGRESS_EVENT,
            InstallProgress::Initializing {
                message: "Downloading collection manifest...".to_string(),
            },
        );

        let token = match &auth_method {
            oauth::AuthMethod::ApiKey(key) => Some(key.clone()),
            oauth::AuthMethod::OAuth(tokens) => Some(tokens.access_token.clone()),
            oauth::AuthMethod::None => None,
        };

        match collections::fetch_collection_bundle(token.as_deref(), slug, revision).await {
            Ok(bundle) => {
                log::info!(
                    "Downloaded collection bundle for {}/rev{}: {} mods with choices",
                    slug,
                    revision,
                    bundle.mods.iter().filter(|m| m.choices.is_some()).count()
                );
                merge_bundle_into_manifest(manifest, &bundle)
            }
            Err(e) => {
                log::warn!(
                    "Failed to download collection bundle: {} — FOMOD choices will require manual selection",
                    e
                );
                manifest.clone()
            }
        }
    } else {
        manifest.clone()
    };
    let manifest = &manifest;

    // Resolve install order (topological sort respecting mod rules)
    let _ = app.emit(
        INSTALL_PROGRESS_EVENT,
        InstallProgress::Initializing {
            message: "Resolving install order...".to_string(),
        },
    );
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
    let _ = app.emit(
        INSTALL_PROGRESS_EVENT,
        InstallProgress::Initializing {
            message: format!(
                "Loading existing mods for {} ({} in collection)...",
                game_id, total_mods
            ),
        },
    );
    let existing_mods = db.list_mods(game_id, bottle_name).unwrap_or_default();

    // ---------------------------------------------------------------
    // Checkpoint: create or resume
    // ---------------------------------------------------------------
    let (checkpoint_id, completed_statuses) = match resume_checkpoint {
        Some((id, statuses)) => {
            log::info!(
                "Resuming collection install from checkpoint {} ({}/{} completed)",
                id,
                statuses
                    .values()
                    .filter(|s| matches!(
                        s.as_str(),
                        "installed" | "already_installed" | "skipped" | "user_action"
                    ))
                    .count(),
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

    let _ = app.emit(
        INSTALL_PROGRESS_EVENT,
        InstallProgress::Initializing {
            message: format!(
                "Preparing download phase ({} mods, {} already installed)...",
                total_mods,
                existing_mods.len()
            ),
        },
    );

    // ---------------------------------------------------------------
    // Phase 1: Concurrent Downloads
    // ---------------------------------------------------------------
    // Only nexus/direct mods that are NOT already installed and (for
    // nexus) require premium are eligible for concurrent download.
    // browse/manual/bundled types are skipped here and handled in
    // Phase 2 as before.

    // Determine concurrency limit from config or platform heuristics.
    // Network downloads are IO-bound, so we can safely run more than CPU core count.
    let max_concurrent = config::get_config()
        .ok()
        .and_then(|c| c.extra.get("download_threads").and_then(|v| v.as_u64()))
        .map(|v| v as usize)
        .unwrap_or_else(|| {
            let cores = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4);
            let is_apple_silicon = cfg!(target_arch = "aarch64") && cfg!(target_os = "macos");
            let is_steam_os = std::path::Path::new("/etc/steamos-release").exists();
            if is_steam_os {
                cores.clamp(4, 8)
            } else if is_apple_silicon {
                cores.clamp(6, 16)
            } else {
                cores.clamp(4, 12)
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

        // Skip already-installed mods (check by nexus mod ID + file ID, or name).
        // When the manifest specifies a file_id, require it to match too —
        // the same NM mod can have multiple files (e.g. INI config + DLL).
        let is_already = existing_mods.iter().any(|m| {
            if let Some(nexus_id) = entry.source.mod_id {
                if m.nexus_mod_id == Some(nexus_id) {
                    if entry.source.file_id.is_none() || m.nexus_file_id == entry.source.file_id {
                        return true;
                    }
                }
            }
            if let Some(file_id) = entry.source.file_id {
                if m.nexus_file_id == Some(file_id) {
                    return true;
                }
            }
            m.name.eq_ignore_ascii_case(&entry.name)
        });
        if is_already {
            continue;
        }

        downloadable.push((i, mod_idx));
    }

    // ---------------------------------------------------------------
    // Disk space pre-check before downloading
    // ---------------------------------------------------------------
    // Sum expected download sizes from the archive specs. file_size is
    // optional per-entry, so we only check when we have *some* data.
    let estimated_download_bytes: u64 = downloadable
        .iter()
        .filter_map(|&(_, mod_idx)| manifest.mods[mod_idx].source.file_size)
        .sum();

    if estimated_download_bytes > 0 {
        // Archives need to be downloaded AND extracted (~3x expansion).
        // Use the same heuristic as disk_budget::estimate_install_impact.
        let estimated_total = estimated_download_bytes * 4; // archive + ~3x extracted

        if let Err(space_err) = disk_budget::check_space_guard(&download_dir, estimated_total) {
            let available = disk_budget::available_space(&download_dir);
            return Err(format!(
                "Not enough disk space: need ~{} (downloads + extraction), have {}. {}",
                disk_budget::format_bytes(estimated_total),
                disk_budget::format_bytes(available),
                space_err,
            ));
        }
    }

    let total_downloads = downloadable.len();

    let _ = app.emit(
        INSTALL_PROGRESS_EVENT,
        InstallProgress::DownloadPhaseStarted {
            total_downloads,
            max_concurrent,
        },
    );

    // ---------------------------------------------------------------
    // Set up extraction shared state BEFORE downloads so each download
    // task can immediately start extracting upon completion.
    // ---------------------------------------------------------------
    let max_extract = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(4, 16);

    // Emit staging phase started early — extraction overlaps with downloads
    let total_staging_bytes_est: u64 = downloadable
        .iter()
        .filter_map(|&(_, mod_idx)| manifest.mods.get(mod_idx).and_then(|m| m.source.file_size))
        .sum();
    let _ = app.emit(
        INSTALL_PROGRESS_EVENT,
        InstallProgress::StagingPhaseStarted {
            total_mods: downloadable.len(),
            max_concurrent: max_extract,
            total_bytes: total_staging_bytes_est,
        },
    );

    // Pre-compute which mods need extraction (not already installed).
    let current_mods_snapshot = db.list_mods(game_id, bottle_name).unwrap_or_default();
    let needs_extraction_set: Arc<HashSet<usize>> = Arc::new(
        downloadable
            .iter()
            .filter(|&&(_, mod_idx)| {
                let entry = &manifest.mods[mod_idx];
                !current_mods_snapshot.iter().any(|m| {
                    if let Some(nexus_id) = entry.source.mod_id {
                        if m.nexus_mod_id == Some(nexus_id) {
                            if entry.source.file_id.is_none() || m.nexus_file_id == entry.source.file_id {
                                return true;
                            }
                        }
                    }
                    if let Some(file_id) = entry.source.file_id {
                        if m.nexus_file_id == Some(file_id) { return true; }
                    }
                    m.name.eq_ignore_ascii_case(&entry.name)
                })
            })
            .map(|&(order_pos, _)| order_pos)
            .collect(),
    );

    let extracted_map: Arc<std::sync::Mutex<HashMap<usize, PathBuf>>> =
        Arc::new(std::sync::Mutex::new(HashMap::new()));
    let extraction_done: Arc<std::sync::Mutex<HashSet<usize>>> =
        Arc::new(std::sync::Mutex::new(HashSet::new()));
    let extraction_notify = Arc::new(Notify::new());
    let extract_sem = Arc::new(Semaphore::new(max_extract));

    // Manifest data needed by extraction tasks
    let manifest_mods = Arc::new(manifest.mods.clone());

    let download_sem = Arc::new(Semaphore::new(max_concurrent));
    let mut handles = Vec::with_capacity(total_downloads);

    for &(order_pos, mod_idx) in &downloadable {
        // Check cancellation before spawning each download task
        if is_cancelled() {
            log::info!("Collection install cancelled during download phase");
            break;
        }

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
        let dl_sem_c = Arc::clone(&download_sem);
        let ext_sem_c = Arc::clone(&extract_sem);
        let game_slug_c = game_slug.to_string();
        let download_dir_c = download_dir.clone();
        let auth_method_c = auth_method.clone();
        let manifest_name_c = manifest.name.clone();
        let game_id_c = game_id.to_string();
        let bottle_name_c = bottle_name.to_string();
        let needs_ext = Arc::clone(&needs_extraction_set);
        let map_c = Arc::clone(&extracted_map);
        let done_c = Arc::clone(&extraction_done);
        let notify_c = Arc::clone(&extraction_notify);
        let manifest_mods_c = Arc::clone(&manifest_mods);

        let handle = tokio::spawn(async move {
            // ---- Download Phase ----
            let _dl_permit = dl_sem_c.acquire().await.expect("download semaphore closed");

            if is_cancelled() {
                // Mark extraction done so install loop doesn't hang
                done_c.lock().unwrap_or_else(|e| e.into_inner()).insert(order_pos);
                notify_c.notify_waiters();
                return (order_pos, Err(InstallError::Failed("Cancelled".to_string())));
            }

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
                &auth_method_c,
                &manifest_name_c,
                &game_id_c,
                &bottle_name_c,
            )
            .await;

            // Release download permit — we're done with network I/O
            drop(_dl_permit);

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
                    // Download failed — mark extraction as done (no archive to extract)
                    done_c.lock().unwrap_or_else(|e| e.into_inner()).insert(order_pos);
                    notify_c.notify_waiters();
                    return (order_pos, result);
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
                    done_c.lock().unwrap_or_else(|e| e.into_inner()).insert(order_pos);
                    notify_c.notify_waiters();
                    return (order_pos, result);
                }
            }

            // ---- Inline Extraction Phase ----
            // Start extracting immediately after download completes.
            if let Ok(ref dl) = result {
                if needs_ext.contains(&order_pos) && !is_cancelled() {
                    let archive = dl.archive_path.clone();
                    let arc_size = manifest_mods_c
                        .get(mod_idx)
                        .and_then(|m| m.source.file_size)
                        .unwrap_or(0);

                    let _ = app_h.emit(
                        INSTALL_PROGRESS_EVENT,
                        InstallProgress::StagingModStarted {
                            mod_index: order_pos,
                            mod_name: mod_name.clone(),
                            archive_size: arc_size,
                        },
                    );

                    let _ext_permit = ext_sem_c.acquire().await.expect("extract semaphore closed");

                    if !is_cancelled() {
                        let extract_start = std::time::Instant::now();
                        let estimated_total = arc_size.saturating_mul(3);
                        let temp_dir = std::env::temp_dir()
                            .join(format!("corkscrew_extract_{}", order_pos));

                        // Spawn dir-size poller for progress tracking
                        let poller_stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
                        let poller_stop_c = poller_stop.clone();
                        let poll_dir = temp_dir.clone();
                        let app_poll = app_h.clone();
                        let poller_handle = tokio::spawn(async move {
                            let mut prev_bytes = 0u64;
                            loop {
                                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                                if poller_stop_c.load(std::sync::atomic::Ordering::Relaxed) {
                                    break;
                                }
                                let bytes = crate::disk_budget::dir_size(&poll_dir);
                                if bytes != prev_bytes {
                                    let _ = app_poll.emit(
                                        INSTALL_PROGRESS_EVENT,
                                        InstallProgress::StagingProgress {
                                            mod_index: order_pos,
                                            files_done: 0,
                                            files_total: 0,
                                            bytes_done: bytes,
                                            bytes_total: estimated_total,
                                        },
                                    );
                                    prev_bytes = bytes;
                                }
                            }
                        });

                        // Extract with 30-minute timeout
                        let ext_result = tokio::time::timeout(
                            std::time::Duration::from_secs(1800),
                            tokio::task::spawn_blocking({
                                let archive = archive.clone();
                                let temp_dir = temp_dir.clone();
                                move || {
                                    if is_cancelled() {
                                        return Err("Cancelled".to_string());
                                    }
                                    if let Err(e) = validate_archive(&archive) {
                                        return Err(format!("Archive validation failed: {}", e));
                                    }
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
                                }
                            }),
                        )
                        .await;

                        // Stop poller
                        poller_stop.store(true, std::sync::atomic::Ordering::SeqCst);
                        poller_handle.abort();

                        match ext_result {
                            Ok(Ok(Ok(dir))) => {
                                let duration_ms = extract_start.elapsed().as_millis() as u64;
                                let extracted_size: u64 = walkdir::WalkDir::new(&dir)
                                    .into_iter()
                                    .filter_map(|e| e.ok())
                                    .filter(|e| e.file_type().is_file())
                                    .map(|e| e.metadata().map(|m| m.len()).unwrap_or(0))
                                    .sum();
                                let _ = app_h.emit(
                                    INSTALL_PROGRESS_EVENT,
                                    InstallProgress::StagingModCompleted {
                                        mod_index: order_pos,
                                        mod_name: mod_name.clone(),
                                        extracted_size,
                                        duration_ms,
                                    },
                                );
                                map_c.lock().unwrap_or_else(|e| e.into_inner()).insert(order_pos, dir);
                            }
                            Ok(Ok(Err(e))) => {
                                log::warn!("Extraction failed for mod {}: {}", order_pos, e);
                                let _ = app_h.emit(
                                    INSTALL_PROGRESS_EVENT,
                                    InstallProgress::StagingModFailed {
                                        mod_index: order_pos,
                                        mod_name: mod_name.clone(),
                                        error: e,
                                    },
                                );
                            }
                            Ok(Err(e)) => {
                                log::warn!("Extraction task panicked for mod {}: {}", order_pos, e);
                                let _ = app_h.emit(
                                    INSTALL_PROGRESS_EVENT,
                                    InstallProgress::StagingModFailed {
                                        mod_index: order_pos,
                                        mod_name: mod_name.clone(),
                                        error: format!("Task panicked: {}", e),
                                    },
                                );
                            }
                            Err(_) => {
                                log::warn!("Extraction timed out for mod {} after 30 minutes", order_pos);
                                let _ = app_h.emit(
                                    INSTALL_PROGRESS_EVENT,
                                    InstallProgress::StagingModFailed {
                                        mod_index: order_pos,
                                        mod_name: mod_name.clone(),
                                        error: "Extraction timed out after 30 minutes".to_string(),
                                    },
                                );
                            }
                        }
                    }
                }
            }

            // Always mark extraction as done and wake install loop
            done_c.lock().unwrap_or_else(|e| e.into_inner()).insert(order_pos);
            notify_c.notify_waiters();

            (order_pos, result)
        });

        handles.push(handle);
    }

    // Wait for all download+extract tasks
    let download_results = futures::future::join_all(handles).await;

    // Collect successful downloads into a map: order_position -> archive path
    let mut pre_downloaded: HashMap<usize, PathBuf> = HashMap::new();
    let mut dl_downloaded = 0usize;
    let mut dl_cached = 0usize;
    let mut dl_failed_count = 0usize;
    let dl_skipped = total_mods - total_downloads; // browse/manual/bundled/non-premium/already-installed

    // Track which downloads failed so we can retry them as a batch
    let mut failed_entries: Vec<(usize, usize)> = Vec::new(); // (order_pos, mod_idx)

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
            Ok((order_pos, Err(e))) => {
                // Find the mod_idx for this order_pos
                if let Some(&(_, mod_idx)) = downloadable.iter().find(|&&(op, _)| op == order_pos) {
                    // Only retry transient errors
                    if is_transient_collection_error(&e) {
                        failed_entries.push((order_pos, mod_idx));
                    } else {
                        dl_failed_count += 1;
                    }
                } else {
                    dl_failed_count += 1;
                }
            }
            Err(join_err) => {
                log::error!("Download task panicked: {}", join_err);
                dl_failed_count += 1;
            }
        }
    }

    // ---------------------------------------------------------------
    // Retry pass: re-attempt failed transient downloads sequentially
    // ---------------------------------------------------------------
    // If any downloads failed with transient errors despite per-mod retries,
    // wait a bit and try them again.  CDN/API hiccups often resolve within
    // 15-30 seconds, so a second wave of attempts frequently succeeds.
    if !failed_entries.is_empty() && !is_cancelled() {
        let retry_count = failed_entries.len();
        log::info!(
            "Retrying {} failed downloads after 10s cooldown",
            retry_count
        );
        let _ = app.emit(
            INSTALL_PROGRESS_EVENT,
            InstallProgress::DownloadRetryStarted {
                count: retry_count,
            },
        );

        tokio::time::sleep(std::time::Duration::from_secs(10)).await;

        for (order_pos, mod_idx) in failed_entries {
            if is_cancelled() {
                dl_failed_count += 1;
                continue;
            }

            let entry = &manifest.mods[mod_idx];
            log::info!("Retry-pass: re-attempting download for '{}'", entry.name);

            let result = download_mod_archive(
                app,
                db,
                queue,
                entry,
                order_pos,
                game_slug,
                &download_dir,
                &auth_method,
                &manifest.name,
                game_id,
                bottle_name,
            )
            .await;

            match result {
                Ok(dl) => {
                    log::info!("Retry-pass succeeded for '{}'", entry.name);
                    if dl.cached {
                        dl_cached += 1;
                    } else {
                        dl_downloaded += 1;
                    }
                    pre_downloaded.insert(order_pos, dl.archive_path.clone());
                    let _ = app.emit(
                        INSTALL_PROGRESS_EVENT,
                        InstallProgress::DownloadModCompleted {
                            mod_index: order_pos,
                            mod_name: entry.name.clone(),
                            cached: dl.cached,
                        },
                    );

                    // Inline extraction for retried downloads
                    if needs_extraction_set.contains(&order_pos) && !is_cancelled() {
                        let archive = dl.archive_path.clone();
                        let arc_size = entry.source.file_size.unwrap_or(0);
                        let mod_name_ext = entry.name.clone();

                        let _ = app.emit(
                            INSTALL_PROGRESS_EVENT,
                            InstallProgress::StagingModStarted {
                                mod_index: order_pos,
                                mod_name: mod_name_ext.clone(),
                                archive_size: arc_size,
                            },
                        );

                        let estimated_total = arc_size.saturating_mul(3);
                        let temp_dir = std::env::temp_dir()
                            .join(format!("corkscrew_extract_{}", order_pos));

                        let poller_stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
                        let poller_stop_c = poller_stop.clone();
                        let poll_dir = temp_dir.clone();
                        let app_poll = app.clone();
                        let poller_handle = tokio::spawn(async move {
                            let mut prev_bytes = 0u64;
                            loop {
                                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                                if poller_stop_c.load(std::sync::atomic::Ordering::Relaxed) {
                                    break;
                                }
                                let bytes = crate::disk_budget::dir_size(&poll_dir);
                                if bytes != prev_bytes {
                                    let _ = app_poll.emit(
                                        INSTALL_PROGRESS_EVENT,
                                        InstallProgress::StagingProgress {
                                            mod_index: order_pos,
                                            files_done: 0,
                                            files_total: 0,
                                            bytes_done: bytes,
                                            bytes_total: estimated_total,
                                        },
                                    );
                                    prev_bytes = bytes;
                                }
                            }
                        });

                        let extract_start = std::time::Instant::now();
                        let ext_result = tokio::time::timeout(
                            std::time::Duration::from_secs(1800),
                            tokio::task::spawn_blocking({
                                let archive = archive.clone();
                                let temp_dir = temp_dir.clone();
                                move || {
                                    if is_cancelled() {
                                        return Err("Cancelled".to_string());
                                    }
                                    if let Err(e) = validate_archive(&archive) {
                                        return Err(format!("Archive validation failed: {}", e));
                                    }
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
                                }
                            }),
                        )
                        .await;

                        poller_stop.store(true, std::sync::atomic::Ordering::SeqCst);
                        poller_handle.abort();

                        match ext_result {
                            Ok(Ok(Ok(dir))) => {
                                let duration_ms = extract_start.elapsed().as_millis() as u64;
                                let extracted_size: u64 = walkdir::WalkDir::new(&dir)
                                    .into_iter()
                                    .filter_map(|e| e.ok())
                                    .filter(|e| e.file_type().is_file())
                                    .map(|e| e.metadata().map(|m| m.len()).unwrap_or(0))
                                    .sum();
                                let _ = app.emit(
                                    INSTALL_PROGRESS_EVENT,
                                    InstallProgress::StagingModCompleted {
                                        mod_index: order_pos,
                                        mod_name: mod_name_ext,
                                        extracted_size,
                                        duration_ms,
                                    },
                                );
                                extracted_map.lock().unwrap_or_else(|e| e.into_inner()).insert(order_pos, dir);
                            }
                            Ok(Ok(Err(e))) => {
                                log::warn!("Retry extraction failed for mod {}: {}", order_pos, e);
                                let _ = app.emit(
                                    INSTALL_PROGRESS_EVENT,
                                    InstallProgress::StagingModFailed {
                                        mod_index: order_pos,
                                        mod_name: mod_name_ext,
                                        error: e,
                                    },
                                );
                            }
                            Ok(Err(e)) => {
                                log::warn!("Retry extraction panicked for mod {}: {}", order_pos, e);
                                let _ = app.emit(
                                    INSTALL_PROGRESS_EVENT,
                                    InstallProgress::StagingModFailed {
                                        mod_index: order_pos,
                                        mod_name: mod_name_ext,
                                        error: format!("Task panicked: {}", e),
                                    },
                                );
                            }
                            Err(_) => {
                                log::warn!("Retry extraction timed out for mod {}", order_pos);
                                let _ = app.emit(
                                    INSTALL_PROGRESS_EVENT,
                                    InstallProgress::StagingModFailed {
                                        mod_index: order_pos,
                                        mod_name: mod_name_ext,
                                        error: "Extraction timed out after 30 minutes".to_string(),
                                    },
                                );
                            }
                        }

                        extraction_done.lock().unwrap_or_else(|e| e.into_inner()).insert(order_pos);
                        extraction_notify.notify_waiters();
                    }
                }
                Err(e) => {
                    log::warn!("Retry-pass failed for '{}': {:?}", entry.name, e);
                    dl_failed_count += 1;
                    let _ = app.emit(
                        INSTALL_PROGRESS_EVENT,
                        InstallProgress::DownloadModFailed {
                            mod_index: order_pos,
                            mod_name: entry.name.clone(),
                            error: format!("{:?}", e),
                        },
                    );
                    // Mark extraction done for failed retries so install loop doesn't hang
                    extraction_done.lock().unwrap_or_else(|e| e.into_inner()).insert(order_pos);
                    extraction_notify.notify_waiters();
                }
            }
        }
    }

    let _ = app.emit(
        INSTALL_PROGRESS_EVENT,
        InstallProgress::AllDownloadsCompleted {
            downloaded: dl_downloaded,
            cached: dl_cached,
            failed: dl_failed_count,
            skipped: dl_skipped,
        },
    );

    // Check for cancellation before install phase
    if is_cancelled() {
        log::info!("Collection install cancelled before install phase");
        let _ = app.emit(
            INSTALL_PROGRESS_EVENT,
            InstallProgress::CollectionCompleted {
                installed: 0,
                skipped: total_mods,
                failed: 0,
            },
        );
        return Ok(CollectionInstallResult {
            installed: 0,
            already_installed: 0,
            skipped: total_mods,
            failed: 0,
            details,
        });
    }

    // ---------------------------------------------------------------
    // Phase 2: Sequential Install (overlaps with ongoing extractions)
    // ---------------------------------------------------------------
    // Install starts immediately — the loop waits only for each
    // individual mod's extraction, not all of them.
    let mut temp_guard = TempDirGuard::new();
    let _ = app.emit(
        INSTALL_PROGRESS_EVENT,
        InstallProgress::InstallPhaseStarted { total_mods },
    );

    // Deferred mods whose extraction wasn't done yet during the first pass.
    // After pass 1 completes, these are retried (extractions will be done by then).
    let mut deferred: Vec<(usize, usize)> = Vec::new(); // (order_pos, mod_idx)

    for (i, &mod_idx) in install_order.iter().enumerate() {
        // Check for cancellation at the top of each iteration
        if is_cancelled() {
            log::info!("Collection install cancelled by user at mod {}/{}", i, total_mods);
            let _ = app.emit(
                INSTALL_PROGRESS_EVENT,
                InstallProgress::CollectionCompleted {
                    installed,
                    skipped: skipped + (total_mods - i - installed - already_installed - failed),
                    failed,
                },
            );
            return Ok(CollectionInstallResult {
                installed,
                already_installed,
                skipped: skipped + (total_mods - i - installed - already_installed - failed),
                failed,
                details,
            });
        }

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
                            deployed_size: 0,
                            duration_ms: 0,
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

        // Skip mods still extracting — defer to pass 2 so we don't stall
        // the entire install pipeline waiting for a few slow archives.
        if needs_extraction_set.contains(&i) {
            let is_done = extraction_done.lock().unwrap_or_else(|e| e.into_inner()).contains(&i);
            if !is_done {
                log::info!("Deferring install of '{}' — extraction not done yet", mod_name);
                deferred.push((i, mod_idx));
                continue;
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

        // Check if this mod is already installed — query DB live to catch mods
        // installed earlier in this same run (existing_mods is a stale snapshot).
        // When file_id is specified, require it to match too — same NM mod can
        // have multiple files (e.g. INI config + DLL).
        let current_mods = db.list_mods(game_id, bottle_name).unwrap_or_default();
        let is_already = current_mods.iter().any(|m| {
            if let Some(nexus_id) = mod_entry.source.mod_id {
                if m.nexus_mod_id == Some(nexus_id) {
                    if mod_entry.source.file_id.is_none() || m.nexus_file_id == mod_entry.source.file_id {
                        return true;
                    }
                }
            }
            if let Some(file_id) = mod_entry.source.file_id {
                if m.nexus_file_id == Some(file_id) {
                    return true;
                }
            }
            m.name.eq_ignore_ascii_case(mod_name)
        });

        if is_already {
            // Tag the existing mod with this collection name so delete-collection
            // can find and remove it later (handles mods from previous cancelled installs
            // that may not have been tagged).
            if let Some(existing) = current_mods.iter().find(|m| {
                if let Some(nexus_id) = mod_entry.source.mod_id {
                    if m.nexus_mod_id == Some(nexus_id) {
                        if mod_entry.source.file_id.is_none() || m.nexus_file_id == mod_entry.source.file_id {
                            return true;
                        }
                    }
                }
                if let Some(file_id) = mod_entry.source.file_id {
                    if m.nexus_file_id == Some(file_id) { return true; }
                }
                m.name.eq_ignore_ascii_case(mod_name)
            }) {
                let _ = db.set_collection_name(existing.id, &manifest.name);
            }

            let _ = app.emit(
                INSTALL_PROGRESS_EVENT,
                InstallProgress::ModCompleted {
                    mod_index: i,
                    mod_name: mod_name.clone(),
                    mod_id: 0,
                    deployed_size: 0,
                    duration_ms: 0,
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

        // Look up pre-downloaded archive for this position
        let pre_dl = pre_downloaded.get(&i).map(|p| p.as_path());

        // Wait for this mod's extraction if it's in the extraction pipeline.
        // This blocks only until THIS mod is ready, not all of them.
        let pre_ext = if needs_extraction_set.contains(&i) {
            loop {
                // Check if extraction is done (success or failure)
                if extraction_done.lock().unwrap_or_else(|e| e.into_inner()).contains(&i) {
                    // Take the extracted dir if extraction succeeded
                    let dir = extracted_map.lock().unwrap_or_else(|e| e.into_inner()).remove(&i);
                    if let Some(ref d) = dir {
                        temp_guard.track(d.clone());
                    }
                    break dir;
                }
                // Not done yet — wait for notification, then check again
                extraction_notify.notified().await;
            }
        } else {
            None
        };
        if let Some(ref dir) = pre_ext {
            temp_guard.untrack(dir);
        }

        let install_start = std::time::Instant::now();
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
            &auth_method,
            is_premium,
            &manifest.name,
            pre_dl,
            pre_ext,
        )
        .await;

        match result {
            Ok((mod_id, deployed_size)) => {
                let install_duration_ms = install_start.elapsed().as_millis() as u64;
                // Tag this mod with the collection name
                let _ = db.set_collection_name(mod_id, &manifest.name);

                let _ = app.emit(
                    INSTALL_PROGRESS_EVENT,
                    InstallProgress::ModCompleted {
                        mod_index: i,
                        mod_name: mod_name.clone(),
                        mod_id,
                        deployed_size,
                        duration_ms: install_duration_ms,
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

    // ---------------------------------------------------------------
    // Pass 2: install deferred mods (extraction should be done now)
    // ---------------------------------------------------------------
    if !deferred.is_empty() && !is_cancelled() {
        log::info!(
            "Pass 2: installing {} deferred mods whose extractions were slow",
            deferred.len()
        );
        for (i, mod_idx) in deferred {
            if is_cancelled() {
                break;
            }

            let mod_entry = &manifest.mods[mod_idx];
            let mod_name = &mod_entry.name;

            // Wait for extraction to complete (it should be done by now, but be safe)
            let pre_ext = if needs_extraction_set.contains(&i) {
                loop {
                    if extraction_done.lock().unwrap_or_else(|e| e.into_inner()).contains(&i) {
                        let dir = extracted_map.lock().unwrap_or_else(|e| e.into_inner()).remove(&i);
                        if let Some(ref d) = dir {
                            temp_guard.track(d.clone());
                        }
                        break dir;
                    }
                    extraction_notify.notified().await;
                }
            } else {
                None
            };
            if let Some(ref dir) = pre_ext {
                temp_guard.untrack(dir);
            }

            let _ = app.emit(
                INSTALL_PROGRESS_EVENT,
                InstallProgress::ModStarted {
                    mod_index: i,
                    total_mods,
                    mod_name: mod_name.clone(),
                },
            );

            // Check already-installed (live DB query).
            // Require file_id match when specified — same mod can have multiple files.
            let current_mods = db.list_mods(game_id, bottle_name).unwrap_or_default();
            let is_already = current_mods.iter().any(|m| {
                if let Some(nexus_id) = mod_entry.source.mod_id {
                    if m.nexus_mod_id == Some(nexus_id) {
                        if mod_entry.source.file_id.is_none() || m.nexus_file_id == mod_entry.source.file_id {
                            return true;
                        }
                    }
                }
                if let Some(file_id) = mod_entry.source.file_id {
                    if m.nexus_file_id == Some(file_id) {
                        return true;
                    }
                }
                m.name.eq_ignore_ascii_case(mod_name)
            });

            if is_already {
                if let Some(existing) = current_mods.iter().find(|m| {
                    m.name.eq_ignore_ascii_case(mod_name)
                        || mod_entry.source.mod_id.map_or(false, |id| m.nexus_mod_id == Some(id))
                }) {
                    let _ = db.set_collection_name(existing.id, &manifest.name);
                }
                let _ = app.emit(
                    INSTALL_PROGRESS_EVENT,
                    InstallProgress::ModCompleted {
                        mod_index: i,
                        mod_name: mod_name.clone(),
                        mod_id: 0,
                        deployed_size: 0,
                        duration_ms: 0,
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

            let pre_dl = pre_downloaded.get(&i).map(|p| p.as_path());
            let install_start = std::time::Instant::now();
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
                &auth_method,
                is_premium,
                &manifest.name,
                pre_dl,
                pre_ext,
            )
            .await;

            match result {
                Ok((mod_id, deployed_size)) => {
                    let install_duration_ms = install_start.elapsed().as_millis() as u64;
                    let _ = db.set_collection_name(mod_id, &manifest.name);
                    let _ = app.emit(
                        INSTALL_PROGRESS_EVENT,
                        InstallProgress::ModCompleted {
                            mod_index: i,
                            mod_name: mod_name.clone(),
                            mod_id,
                            deployed_size,
                            duration_ms: install_duration_ms,
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
                Err(InstallError::UserAction { action, url, instructions }) => {
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
    }

    // Clean up any remaining pre-extracted temp dirs (for skipped/failed mods).
    // The TempDirGuard also handles this on drop, but explicit cleanup is clearer.
    {
        let remaining = extracted_map.lock().unwrap_or_else(|e| e.into_inner());
        for dir in remaining.values() {
            temp_guard.untrack(dir);
            let _ = std::fs::remove_dir_all(dir);
        }
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
            let implicit = plugins::skyrim_plugins::implicit_plugins_for_game(game_id);
            let _ = plugins::skyrim_plugins::sync_plugins(
                Path::new(&game.data_dir),
                &pf,
                &loadorder_file,
                implicit,
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

    // Post-install deployment verification
    let manifest_count = db
        .get_deployment_manifest(game_id, bottle_name)
        .map(|m| m.len())
        .unwrap_or(0);
    if installed > 0 && manifest_count == 0 {
        log::error!(
            "DEPLOYMENT BUG: {} mods installed but deployment manifest is empty! \
             Mods are staged but not linked to the game directory. \
             data_dir={}, staging_root={}",
            installed,
            data_dir.display(),
            staging::staging_base_dir(game_id, bottle_name).display(),
        );
        // Try an emergency redeploy to fix the situation
        log::info!("Attempting emergency redeploy for {}/{}", game_id, bottle_name);
        match deployer::redeploy_all(db, game_id, bottle_name, &data_dir) {
            Ok(result) => {
                log::info!(
                    "Emergency redeploy succeeded: {} files deployed",
                    result.deployed_count
                );
            }
            Err(e) => {
                log::error!("Emergency redeploy failed: {}", e);
            }
        }
    } else {
        log::info!(
            "Post-install check: {} mods installed, {} deployment manifest entries",
            installed,
            manifest_count
        );
    }

    // Auto-resolve file conflicts using heuristics (collection priority order,
    // identical-content detection, patch naming, etc.) so the user doesn't see
    // hundreds of unresolved conflicts after a one-click collection install.
    match db.find_all_conflicts(game_id, bottle_name) {
        Ok(conflicts) if !conflicts.is_empty() => {
            let mods = db.list_mods(game_id, bottle_name).unwrap_or_default();
            let mod_ids: Vec<i64> = conflicts
                .iter()
                .flat_map(|c| c.mods.iter().map(|m| m.mod_id))
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            let file_hashes = db.get_file_hashes_bulk(&mod_ids).unwrap_or_default();
            let (suggestions, _) =
                conflict_resolver::analyze_conflicts(&conflicts, &mods, None, &file_hashes);
            match conflict_resolver::apply_suggestions(db, game_id, bottle_name, &suggestions) {
                Ok(result) => {
                    // Record resolved conflicts so they don't show in the UI
                    for suggestion in &suggestions {
                        let winner = match suggestion.status {
                            conflict_resolver::ConflictStatus::AuthorResolved
                            | conflict_resolver::ConflictStatus::IdenticalContent => {
                                suggestion.current_winner_id
                            }
                            conflict_resolver::ConflictStatus::Suggested => {
                                suggestion.suggested_winner_id
                            }
                            conflict_resolver::ConflictStatus::Manual => continue,
                        };
                        for m in &suggestion.mods {
                            if m.mod_id != winner {
                                let _ = db.add_conflict_rule(
                                    game_id,
                                    bottle_name,
                                    winner,
                                    m.mod_id,
                                );
                            }
                        }
                    }
                    log::info!(
                        "Auto-resolved conflicts: {} total, {} author, {} suggested, {} identical, {} manual, {} priorities changed",
                        result.total_conflicts,
                        result.author_resolved,
                        result.auto_suggested,
                        result.identical_content,
                        result.manual_needed,
                        result.priorities_changed,
                    );
                    // Redeploy if priorities changed
                    if result.priorities_changed > 0 {
                        let _ = deployer::redeploy_all(db, game_id, bottle_name, &data_dir);
                    }
                }
                Err(e) => {
                    log::warn!("Auto-conflict resolution failed: {}", e);
                }
            }
        }
        _ => {}
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

#[derive(Debug)]
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
    auth_method: &oauth::AuthMethod,
    is_premium: bool,
    manifest_name: &str,
    pre_downloaded: Option<&Path>,
    pre_extracted: Option<PathBuf>,
) -> Result<(i64, u64), InstallError> {
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
                auth_method,
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
                auth_method,
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
    auth_method: &oauth::AuthMethod,
    is_premium: bool,
    manifest_name: &str,
    pre_downloaded: Option<&Path>,
    pre_extracted: Option<PathBuf>,
) -> Result<(i64, u64), InstallError> {
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

        let (mod_id, deployed_size) = stage_and_deploy(
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
        return Ok((mod_id, deployed_size));
    }

    // Fallback: no pre-downloaded archive — download now (shouldn't normally
    // happen in the two-phase flow, but handles edge cases).

    // Check download registry for existing archive (dedup)
    if let Ok(Some(existing)) = db.find_download_by_nexus_ids(nexus_mod_id, nexus_file_id) {
        let path = std::path::Path::new(&existing.archive_path);
        if path.exists() {
            // Verify file size matches the registered size.
            let size_ok = if existing.file_size > 0 {
                std::fs::metadata(path)
                    .map(|m| m.len() as i64 == existing.file_size)
                    .unwrap_or(false)
            } else {
                true
            };
            if size_ok {
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
                        detail: Some(format!(
                            "Reusing cached download for '{}'",
                            mod_entry.name
                        )),
                    },
                );

                let (mod_id, deployed_size) = stage_and_deploy(
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
                let _ = db.add_download_collection_ref(
                    existing.id,
                    manifest_name,
                    game_id,
                    bottle_name,
                );

                return Ok((mod_id, deployed_size));
            } else {
                log::warn!(
                    "Cached download for '{}' has wrong file size, will re-download",
                    mod_entry.name
                );
            }
        } else {
            log::info!(
                "Cached file missing from disk for '{}', will re-download",
                mod_entry.name
            );
        }
    }

    let client = NexusClient::from_auth_method(auth_method)
        .map_err(|e| InstallError::Failed(format!("No NexusMods auth configured: {}", e)))?;

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

    // Register download in dedup registry — skip SHA-256 to avoid blocking pipeline
    let archive_name = archive_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let file_size = std::fs::metadata(&archive_path)
        .map(|m| m.len() as i64)
        .unwrap_or(0);
    if let Ok(dl_id) = db.register_download(
        &archive_path.to_string_lossy(),
        &archive_name,
        Some(nexus_mod_id),
        Some(nexus_file_id),
        None,
        file_size,
    ) {
        let _ = db.add_download_collection_ref(dl_id, manifest_name, game_id, bottle_name);
    }

    // Stage and deploy the downloaded archive
    let (mod_id, deployed_size) = stage_and_deploy(
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

    Ok((mod_id, deployed_size))
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
    auth_method: &oauth::AuthMethod,
    manifest_name: &str,
    pre_downloaded: Option<&Path>,
    pre_extracted: Option<PathBuf>,
) -> Result<(i64, u64), InstallError> {
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

        let (mod_id, deployed_size) = stage_and_deploy(
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
        return Ok((mod_id, deployed_size));
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
                // Verify file size matches the registered size.
                let size_ok = if existing.file_size > 0 {
                    std::fs::metadata(path)
                        .map(|m| m.len() as i64 == existing.file_size)
                        .unwrap_or(false)
                } else {
                    true
                };
                if size_ok {
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
                            detail: Some(format!(
                                "Reusing cached download for '{}'",
                                mod_entry.name
                            )),
                        },
                    );

                    let (mod_id, deployed_size) = stage_and_deploy(
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

                    return Ok((mod_id, deployed_size));
                } else {
                    log::warn!(
                        "Cached download for '{}' has wrong file size, will re-download",
                        mod_entry.name
                    );
                }
            } else {
                log::info!(
                    "Cached file missing from disk for '{}', will re-download",
                    mod_entry.name
                );
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
    let client = NexusClient::from_auth_method(auth_method)
        .unwrap_or_else(|_| NexusClient::new(String::new()));

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

    // Register download in dedup registry — skip SHA-256 to avoid blocking pipeline
    let archive_name = archive_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let file_size = std::fs::metadata(&archive_path)
        .map(|m| m.len() as i64)
        .unwrap_or(0);
    if let Ok(dl_id) = db.register_download(
        &archive_path.to_string_lossy(),
        &archive_name,
        mod_entry.source.mod_id,
        mod_entry.source.file_id,
        None,
        file_size,
    ) {
        let _ = db.add_download_collection_ref(dl_id, manifest_name, game_id, bottle_name);
    }

    let (mod_id, deployed_size) = stage_and_deploy(
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

    Ok((mod_id, deployed_size))
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
) -> Result<(i64, u64), InstallError> {
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
                // Fast path: skip SHA-256 hashing for collection installs
                let result =
                    staging::stage_mod_from_extracted_opts(&extracted_dir, &gid, &bn, mod_id, &mn, true);
                // Clean up pre-extracted temp dir
                let _ = std::fs::remove_dir_all(&extracted_dir);
                result
            } else {
                // Fallback: direct extraction + staging (also skip hash for collections)
                staging::stage_mod_extract_direct(&archive, &gid, &bn, mod_id, &mn, true)
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
                match parse_fomod_choices(choices) {
                    Some(parsed) => {
                        log::info!(
                            "FOMOD auto-applying {} group selections for '{}'",
                            parsed.len(),
                            mod_name
                        );
                        parsed
                    }
                    None => {
                        log::warn!(
                            "FOMOD choices present but unparseable for '{}': {}",
                            mod_name,
                            choices,
                        );
                        fomod::get_default_selections(&fomod_installer)
                    }
                }
            } else {
                // No manifest choices — check saved recipe, then use defaults.
                // Collection curators omit choices when defaults are correct;
                // prompting the user would stall the entire pipeline.
                if let Ok(Some(recipe)) = crate::fomod_recipes::get_recipe(db, mod_id) {
                    log::info!(
                        "FOMOD auto-applying saved recipe ({} selections) for '{}'",
                        recipe.selections.len(),
                        mod_name
                    );
                    recipe.selections
                } else {
                    log::info!(
                        "FOMOD using defaults for '{}' — no choices in manifest, no saved recipe",
                        mod_name
                    );
                    fomod::get_default_selections(&fomod_installer)
                }
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
            let patch_bytes =
                match base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64_patch)
                {
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
        let app_deploy = app.clone();

        let deploy_result = tokio::task::spawn_blocking(move || {
            let last_emit = std::sync::Mutex::new(
                std::time::Instant::now()
                    .checked_sub(std::time::Duration::from_secs(1))
                    .unwrap_or_else(std::time::Instant::now),
            );
            let progress_cb = move |files_done: u64, files_total: u64| {
                let mut last = last_emit.lock().unwrap_or_else(|e| e.into_inner());
                if last.elapsed().as_millis() >= 200 || files_done == files_total {
                    let _ = app_deploy.emit(
                        INSTALL_PROGRESS_EVENT,
                        InstallProgress::DeployProgress {
                            mod_index,
                            files_done,
                            files_total,
                            bytes_done: 0,
                            bytes_total: 0,
                        },
                    );
                    *last = std::time::Instant::now();
                }
            };
            deployer::deploy_mod_atomic_with_progress(
                &db_c, &gid, &bn, mod_id, &sp, &dd, &files, &progress_cb,
            )
        })
        .await
        .map_err(|e| InstallError::Failed(format!("Deploy join error: {}", e)))?;

        if let Err(e) = deploy_result {
            let err_msg = e.to_string();
            // If deploy failed because 0 files could be found, keep staging + DB
            // entry so a "redeploy" can fix it later. Only remove staging for
            // truly fatal errors (I/O, DB, etc.).
            if err_msg.contains("0 of") && err_msg.contains("files deployed") {
                log::warn!(
                    "Deploy returned 0 files for mod {} — keeping staging for later redeploy: {}",
                    mod_id,
                    err_msg
                );
                // Still return Ok so the mod appears as "installed" — the user
                // can trigger a redeploy to actually link the files.
            } else {
                let _ = staging::remove_staging(&staging_result.staging_path);
                let _ = db.remove_mod(mod_id);
                return Err(InstallError::Failed(format!("Deploy failed: {}", e)));
            }
        }
    }

    // If the collection manifest marks this mod as disabled, disable it after deploy
    if mod_entry.install_disabled {
        let _ = db.set_enabled(mod_id, false);
    }

    // Calculate deployed size from staging files
    let deployed_size: u64 = files_to_deploy
        .iter()
        .map(|f| {
            let path = staging_result.staging_path.join(f);
            path.metadata().map(|m| m.len()).unwrap_or(0)
        })
        .sum();

    Ok((mod_id, deployed_size))
}

/// Parse FOMOD choices from a collection manifest's JSON value into the
/// HashMap<GroupName, Vec<OptionName>> format expected by `get_files_for_selections`.
///
/// NexusMods collection.json uses the format:
/// ```json
/// { "type": "fomod", "options": [
///   { "name": "StepName", "groups": [
///     { "name": "GroupName", "choices": [
///       { "name": "OptionName", "idx": 2 }
///     ]}
///   ]}
/// ]}
/// ```
fn parse_fomod_choices(
    choices: &serde_json::Value,
) -> Option<HashMap<String, Vec<String>>> {
    let obj = choices.as_object()?;

    // Check for NexusMods nested format: { "type": "fomod", "options": [...] }
    if obj.get("type").and_then(|v| v.as_str()) == Some("fomod") {
        let options = match obj.get("options").and_then(|v| v.as_array()) {
            Some(arr) => arr,
            None => return None,
        };
        let mut result = HashMap::new();

        for step in options {
            // Skip steps without groups (e.g. informational/welcome pages)
            let groups = match step.get("groups").and_then(|v| v.as_array()) {
                Some(arr) => arr,
                None => continue,
            };
            for group in groups {
                let group_name = match group.get("name").and_then(|v| v.as_str()) {
                    Some(n) => n,
                    None => continue,
                };
                let group_choices = match group.get("choices").and_then(|v| v.as_array()) {
                    Some(arr) => arr,
                    None => continue,
                };
                let selected: Vec<String> = group_choices
                    .iter()
                    .filter_map(|c| c.get("name").and_then(|v| v.as_str()).map(String::from))
                    .collect();
                if !selected.is_empty() {
                    result.insert(group_name.to_string(), selected);
                }
            }
        }

        if result.is_empty() {
            return None;
        }
        return Some(result);
    }

    // Fallback: flat format { "GroupName": ["Option1", "Option2"] }
    let mut result = HashMap::new();
    for (key, value) in obj {
        if let Some(arr) = value.as_array() {
            let selections: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
            result.insert(key.clone(), selections);
        }
    }
    if result.is_empty() { None } else { Some(result) }
}

/// Merge FOMOD choices, patches, rules, and plugins from the downloaded
/// collection bundle into the frontend-built manifest.
fn merge_bundle_into_manifest(
    frontend: &collections::CollectionManifest,
    bundle: &collections::CollectionManifest,
) -> collections::CollectionManifest {
    let mut merged = frontend.clone();

    // Build lookup by (mod_id, file_id) from bundle manifest
    let mut bundle_lookup: HashMap<(i64, i64), &CollectionModEntry> = HashMap::new();
    for entry in &bundle.mods {
        if let (Some(mod_id), Some(file_id)) = (entry.source.mod_id, entry.source.file_id) {
            bundle_lookup.insert((mod_id, file_id), entry);
        }
    }

    // Also build a name-based fallback lookup
    let mut bundle_name_lookup: HashMap<String, &CollectionModEntry> = HashMap::new();
    for entry in &bundle.mods {
        bundle_name_lookup.insert(entry.name.to_lowercase(), entry);
    }

    let mut merged_count = 0usize;
    let mut unmatched = Vec::new();
    for mod_entry in &mut merged.mods {
        // Try (mod_id, file_id) match first, then name fallback
        let bundle_entry = if let (Some(mod_id), Some(file_id)) =
            (mod_entry.source.mod_id, mod_entry.source.file_id)
        {
            bundle_lookup.get(&(mod_id, file_id)).copied()
        } else {
            None
        }
        .or_else(|| bundle_name_lookup.get(&mod_entry.name.to_lowercase()).copied());

        if bundle_entry.is_none() {
            unmatched.push(mod_entry.name.clone());
        }

        if let Some(be) = bundle_entry {
            if mod_entry.choices.is_none() {
                mod_entry.choices = be.choices.clone();
            }
            if mod_entry.patches.is_none() {
                mod_entry.patches = be.patches.clone();
            }
            if mod_entry.phase.is_none() {
                mod_entry.phase = be.phase;
            }
            if mod_entry.file_overrides.is_empty() {
                mod_entry.file_overrides = be.file_overrides.clone();
            }
            if mod_entry.choices.is_some() || mod_entry.patches.is_some() {
                merged_count += 1;
            }
        }
    }

    // Copy mod rules and plugins from bundle if frontend didn't provide them
    if merged.mod_rules.is_empty() {
        merged.mod_rules = bundle.mod_rules.clone();
    }
    if merged.plugins.is_empty() {
        merged.plugins = bundle.plugins.clone();
    }

    let bundle_choices_count = bundle.mods.iter().filter(|m| m.choices.is_some()).count();
    let merged_choices_count = merged.mods.iter().filter(|m| m.choices.is_some()).count();
    log::info!(
        "Bundle merge: {} frontend mods, {} bundle mods ({} with choices), {} matched ({} now have choices), {} rules, {} plugins",
        merged.mods.len(),
        bundle.mods.len(),
        bundle_choices_count,
        merged_count,
        merged_choices_count,
        merged.mod_rules.len(),
        merged.plugins.len()
    );
    if !unmatched.is_empty() {
        log::warn!(
            "Bundle merge: {} mods unmatched (no bundle entry found): {:?}",
            unmatched.len(),
            &unmatched[..unmatched.len().min(10)]
        );
    }

    merged
}

/// Check if a path component is safe (no traversal or absolute paths).
fn is_safe_relative_path(path: &str) -> bool {
    !path.contains("..")
        && !path.starts_with('/')
        && !path.starts_with('\\')
        && !path.contains(":/")
        && !path.contains(":\\")
}

/// Apply FOMOD selections to staging by physically rearranging files in the
/// staging directory to match FOMOD destination layout, then returning the
/// list of relative paths to deploy.
///
/// This is necessary because `deploy_mod` uses `staging_path.join(rel_path)`
/// to find source files — if FOMOD remaps `source → destination`, we must
/// move the files so that the destination paths actually exist in staging.
fn apply_fomod_to_staging(
    staging_path: &Path,
    fomod_files: &[fomod::FomodFile],
) -> Option<Vec<String>> {
    // Phase 1: Collect (source_abs, dest_rel) pairs
    let mut moves: Vec<(std::path::PathBuf, String)> = Vec::new();

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
            if src.is_dir() {
                for entry in walkdir::WalkDir::new(&src)
                    .into_iter()
                    .filter_map(|e| e.ok())
                {
                    if entry.file_type().is_file() {
                        if let Ok(rel) = entry.path().strip_prefix(&src) {
                            let dest_rel = if f.destination.is_empty() {
                                rel.to_string_lossy().to_string()
                            } else {
                                format!("{}/{}", f.destination, rel.to_string_lossy())
                            };
                            moves.push((entry.path().to_path_buf(), dest_rel));
                        }
                    }
                }
            }
        } else if src.exists() {
            let dest_rel = if f.destination.is_empty() {
                f.source.clone()
            } else {
                f.destination.clone()
            };
            moves.push((src, dest_rel));
        }
    }

    if moves.is_empty() {
        return None;
    }

    // Phase 2: Copy files to a temp layout dir inside staging, then swap
    let layout_dir = staging_path.join(".fomod_layout");
    if layout_dir.exists() {
        let _ = std::fs::remove_dir_all(&layout_dir);
    }

    let mut files = Vec::with_capacity(moves.len());

    for (src_abs, dest_rel) in &moves {
        let dest_abs = layout_dir.join(dest_rel);
        if let Some(parent) = dest_abs.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                log::warn!("FOMOD layout: failed to create dir {}: {}", parent.display(), e);
                continue;
            }
        }
        // Copy (not move) because sources may overlap or be referenced multiple times
        if let Err(e) = std::fs::copy(src_abs, &dest_abs) {
            log::warn!(
                "FOMOD layout: failed to copy {} → {}: {}",
                src_abs.display(),
                dest_abs.display(),
                e
            );
            continue;
        }
        files.push(dest_rel.replace('\\', "/"));
    }

    if files.is_empty() {
        let _ = std::fs::remove_dir_all(&layout_dir);
        return None;
    }

    // Phase 3: Remove old staging contents (except the layout dir) and move
    // layout contents to staging root
    if let Ok(entries) = std::fs::read_dir(staging_path) {
        for entry in entries.flatten() {
            if entry.path() != layout_dir {
                if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                    let _ = std::fs::remove_dir_all(entry.path());
                } else {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }

    // Move layout contents to staging root
    fn move_contents(from: &std::path::Path, to: &std::path::Path) {
        if let Ok(entries) = std::fs::read_dir(from) {
            for entry in entries.flatten() {
                let dest = to.join(entry.file_name());
                if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                    let _ = std::fs::create_dir_all(&dest);
                    move_contents(&entry.path(), &dest);
                    let _ = std::fs::remove_dir_all(entry.path());
                } else {
                    let _ = std::fs::rename(entry.path(), &dest);
                }
            }
        }
    }
    move_contents(&layout_dir, staging_path);
    let _ = std::fs::remove_dir_all(&layout_dir);

    log::info!(
        "FOMOD: rearranged staging to {} files at {}",
        files.len(),
        staging_path.display()
    );

    Some(files)
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
        // Empty choices → None (no selections configured)
        assert!(parse_fomod_choices(&json).is_none());
    }

    #[test]
    fn test_parse_fomod_choices_non_object() {
        let json = serde_json::json!("not an object");
        assert!(parse_fomod_choices(&json).is_none());
    }

    #[test]
    fn test_parse_fomod_choices_nexus_format() {
        let json = serde_json::json!({
            "type": "fomod",
            "options": [
                {
                    "name": "Texture Resolution",
                    "groups": [
                        {
                            "name": "Resolution",
                            "choices": [
                                { "name": "4K", "idx": 0 },
                                { "name": "Parallax", "idx": 2 }
                            ]
                        }
                    ]
                },
                {
                    "name": "Patches",
                    "groups": [
                        {
                            "name": "Compatibility",
                            "choices": [
                                { "name": "USSEP Patch", "idx": 1 }
                            ]
                        }
                    ]
                }
            ]
        });
        let result = parse_fomod_choices(&json).unwrap();
        assert_eq!(result["Resolution"], vec!["4K", "Parallax"]);
        assert_eq!(result["Compatibility"], vec!["USSEP Patch"]);
    }
}
