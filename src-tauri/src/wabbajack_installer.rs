// ---------------------------------------------------------------------------
// Wabbajack Install Orchestrator
//
// Coordinates the full Wabbajack modlist installation pipeline:
//   Pending → PreFlight → Downloading → Extracting → Processing → Deploying → Completed
//
// Uses WjDownloader for archive downloads (Phase 4), DirectiveProcessor for
// file production/patching (Phase 5), and deployer for game directory
// deployment with atomic rollback (Phase 6).
// ---------------------------------------------------------------------------

use crate::database::ModDatabase;
use crate::nexus;
use crate::oauth;
use crate::wabbajack_directives::DirectiveProcessor;
use crate::wabbajack_downloader::WjDownloader;
use crate::wabbajack_types::*;

use serde::Serialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Semaphore;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum WjInstallError {
    #[error("Pre-flight check failed: {0}")]
    PreFlight(String),
    #[error("Download error: {0}")]
    Download(String),
    #[error("Extraction error: {0}")]
    Extraction(String),
    #[error("Directive error: {0}")]
    Directive(String),
    #[error("Deployment error: {0}")]
    Deployment(String),
    #[error("Installation cancelled")]
    Cancelled,
    #[error("Database error: {0}")]
    Database(String),
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("{0}")]
    Other(String),
}

// ---------------------------------------------------------------------------
// Progress event types (emitted to frontend via Tauri events)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct WjInstallResult {
    pub install_id: i64,
    pub status: String,
    pub total_archives: usize,
    pub total_directives: usize,
    pub files_deployed: usize,
    pub elapsed_secs: f64,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WjPreflightReport {
    pub can_proceed: bool,
    pub issues: Vec<WjPreflightIssue>,
    pub total_download_size: u64,
    pub total_archives: usize,
    pub total_directives: usize,
    pub cached_archives: usize,
    pub disk_space_available: u64,
    pub disk_space_required: u64,
    pub nexus_archives: usize,
    pub is_nexus_premium: bool,
    pub manual_downloads: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type")]
pub enum WjInstallProgressEvent {
    PreFlightStarted,
    PreFlightCompleted {
        report: WjPreflightReport,
    },
    DownloadPhaseStarted {
        total: usize,
    },
    DownloadStarted {
        name: String,
        index: usize,
        total: usize,
    },
    DownloadProgress {
        name: String,
        bytes: u64,
        total_bytes: u64,
    },
    DownloadCompleted {
        name: String,
    },
    DownloadFailed {
        name: String,
        error: String,
    },
    DownloadSkipped {
        name: String,
        reason: String,
    },
    ExtractionStarted {
        total: usize,
    },
    ExtractionProgress {
        name: String,
        index: usize,
        total: usize,
    },
    DirectivePhaseStarted {
        total: usize,
    },
    DirectiveProgress {
        current: usize,
        total: usize,
        directive_type: String,
    },
    DeployStarted {
        total: usize,
    },
    DeployProgress {
        current: usize,
        total: usize,
    },
    InstallCompleted {
        result: WjInstallResult,
    },
    InstallFailed {
        error: String,
    },
    InstallCancelled,
    UserActionRequired {
        archive_name: String,
        url: String,
        prompt: String,
    },
}

// ---------------------------------------------------------------------------
// Wabbajack file parser (typed)
// ---------------------------------------------------------------------------

/// Parse a .wabbajack ZIP file and deserialize the modlist JSON into a
/// strongly-typed `WjTypedModlist`. Tries entry names "modlist" then
/// "modlist.json".
fn parse_wabbajack_file_typed(path: &Path) -> Result<WjTypedModlist, String> {
    let file =
        std::fs::File::open(path).map_err(|e| format!("Cannot open .wabbajack file: {}", e))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("Not a valid ZIP/.wabbajack file: {}", e))?;

    let modlist_json = {
        let try_entry =
            |archive: &mut zip::ZipArchive<std::fs::File>, name: &str| -> Result<String, String> {
                let mut entry = archive.by_name(name).map_err(|e| e.to_string())?;
                let mut buf = String::new();
                entry.read_to_string(&mut buf).map_err(|e| e.to_string())?;
                Ok(buf)
            };
        try_entry(&mut archive, "modlist")
            .or_else(|_| try_entry(&mut archive, "modlist.json"))
            .map_err(|_| {
                "No 'modlist' or 'modlist.json' entry found in .wabbajack file".to_string()
            })?
    };

    serde_json::from_str::<WjTypedModlist>(&modlist_json)
        .map_err(|e| format!("Failed to deserialize modlist: {}", e))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Emit a progress event to the frontend.
fn emit_progress(app: &AppHandle, event: &WjInstallProgressEvent) {
    if let Err(e) = app.emit("wj-install-progress", event) {
        log::warn!("Failed to emit wj-install-progress event: {}", e);
    }
}

/// Check if the cancellation token has been set.
fn is_cancelled(cancel_token: &AtomicBool) -> bool {
    cancel_token.load(Ordering::Relaxed)
}

/// Get available disk space for the filesystem containing `path`.
///
/// Uses `libc::statvfs` on macOS/Linux to query the underlying filesystem.
/// Returns 0 if the path doesn't exist or the statvfs call fails.
fn get_available_disk_space(path: &Path) -> u64 {
    use std::ffi::CString;

    // Ensure the path exists — walk up to the nearest existing ancestor
    let check_path = if path.exists() {
        path.to_path_buf()
    } else {
        let mut ancestor = path.to_path_buf();
        while !ancestor.exists() {
            if !ancestor.pop() {
                return 0;
            }
        }
        ancestor
    };

    let c_path = match CString::new(check_path.to_string_lossy().as_bytes()) {
        Ok(p) => p,
        Err(_) => return 0,
    };

    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut stat) == 0 {
            // Casts needed for cross-platform: field types differ between macOS and Linux
            #[allow(clippy::unnecessary_cast)]
            {
                stat.f_bavail as u64 * stat.f_frsize as u64
            }
        } else {
            0
        }
    }
}

/// Check Nexus premium status using the current auth method.
async fn check_nexus_premium() -> bool {
    let method = oauth::get_auth_method();
    match method {
        oauth::AuthMethod::ApiKey(key) => {
            let client = nexus::NexusClient::new(key);
            client.is_premium().await
        }
        oauth::AuthMethod::OAuth(tokens) => match oauth::parse_user_info(&tokens.access_token) {
            Ok(user) => user.is_premium,
            Err(_) => false,
        },
        oauth::AuthMethod::None => false,
    }
}

// ---------------------------------------------------------------------------
// Pre-flight check
// ---------------------------------------------------------------------------

/// Run pre-flight checks before starting a Wabbajack installation.
///
/// Parses the .wabbajack file, checks disk space, validates Nexus premium
/// status, counts cached archives, and identifies any manual-download
/// archives. Returns a report with all findings and whether the install
/// can proceed.
pub async fn preflight_check(
    app: &AppHandle,
    db: &Arc<ModDatabase>,
    wabbajack_path: &Path,
    _game_id: &str,
    _bottle_name: &str,
    install_dir: &Path,
    download_dir: &Path,
) -> Result<WjPreflightReport, WjInstallError> {
    emit_progress(app, &WjInstallProgressEvent::PreFlightStarted);

    // 1. Parse the .wabbajack file (typed)
    let modlist = parse_wabbajack_file_typed(wabbajack_path).map_err(WjInstallError::Parse)?;

    let total_archives = modlist.archives.len();
    let total_directives = modlist.directives.len();

    // 2. Calculate total download size
    let total_download_size: u64 = modlist.archives.iter().map(|a| a.size).sum();

    // 3. Check disk space for both download_dir and install_dir
    let download_space = get_available_disk_space(download_dir);
    let install_space = get_available_disk_space(install_dir);

    // Estimate: download size + installed files (roughly 2x download for extraction headroom)
    let disk_space_required = total_download_size.saturating_mul(2);
    let disk_space_available = download_space.min(install_space);

    // 4. Count Nexus archives and check premium status
    let nexus_archives = modlist
        .archives
        .iter()
        .filter(|a| matches!(a.state, WjArchiveState::Nexus { .. }))
        .count();

    let is_nexus_premium = if nexus_archives > 0 {
        check_nexus_premium().await
    } else {
        false
    };

    // 5. Count manual-download archives
    let manual_downloads = modlist
        .archives
        .iter()
        .filter(|a| matches!(a.state, WjArchiveState::Manual { .. }))
        .count();

    // 6. Count cached archives (already downloaded and available)
    let mut cached_archives = 0usize;
    for archive in &modlist.archives {
        let hash_str = &archive.hash.0;
        if !hash_str.is_empty() {
            if let Ok(Some(_path)) = db.find_download_by_xxhash(hash_str) {
                cached_archives += 1;
            }
        }
    }

    // 7. Collect issues
    let mut issues = Vec::new();

    if disk_space_available < disk_space_required {
        issues.push(WjPreflightIssue {
            severity: "error".to_string(),
            message: format!(
                "Insufficient disk space: need {} GB, have {} GB",
                disk_space_required / (1024 * 1024 * 1024),
                disk_space_available / (1024 * 1024 * 1024)
            ),
        });
    }

    if nexus_archives > 0 && !is_nexus_premium {
        issues.push(WjPreflightIssue {
            severity: "warning".to_string(),
            message: format!(
                "{} archives require Nexus Mods. Without Premium, you must manually \
                 download each one via the Nexus website (Slow Download).",
                nexus_archives
            ),
        });
    }

    if manual_downloads > 0 {
        issues.push(WjPreflightIssue {
            severity: "warning".to_string(),
            message: format!(
                "{} archives require manual download from external sites.",
                manual_downloads
            ),
        });
    }

    // Can proceed if there are no "error" severity issues
    let can_proceed = !issues.iter().any(|i| i.severity == "error");

    let report = WjPreflightReport {
        can_proceed,
        issues,
        total_download_size,
        total_archives,
        total_directives,
        cached_archives,
        disk_space_available,
        disk_space_required,
        nexus_archives,
        is_nexus_premium,
        manual_downloads,
    };

    emit_progress(
        app,
        &WjInstallProgressEvent::PreFlightCompleted {
            report: report.clone(),
        },
    );

    Ok(report)
}

// ---------------------------------------------------------------------------
// Main install orchestrator
// ---------------------------------------------------------------------------

/// Run the full Wabbajack modlist installation pipeline.
///
/// Pipeline steps:
/// 1. Parse the .wabbajack file
/// 2. Run pre-flight checks
/// 3. Create a DB record for this install
/// 4. Download phase (multi-source: Nexus, HTTP, MediaFire, Mega, Google Drive, WJ CDN)
/// 5. Extraction phase (extract each archive to temp dirs)
/// 6. Directive processing phase (BSDiff, CreateBSA/BA2, DDS transform, MergedPatch, inline)
/// 7. Deploy phase (hardlink-first with copy fallback)
/// 8. Update DB record to completed
/// 9. Return result
///
/// Checks the `cancel_token` at the start of each major loop iteration.
/// If cancelled, updates DB to cancelled, emits InstallCancelled, and
/// returns `WjInstallError::Cancelled`.
#[allow(clippy::too_many_arguments)]
pub async fn install_wabbajack_modlist(
    app: &AppHandle,
    db: &Arc<ModDatabase>,
    wabbajack_path: &Path,
    game_id: &str,
    bottle_name: &str,
    install_dir: &Path,
    download_dir: &Path,
    cancel_token: Arc<AtomicBool>,
) -> Result<WjInstallResult, WjInstallError> {
    let start_time = Instant::now();
    let mut warnings: Vec<String> = Vec::new();

    // -----------------------------------------------------------------------
    // Step 1: Parse the .wabbajack file
    // -----------------------------------------------------------------------
    log::info!("Parsing Wabbajack modlist: {:?}", wabbajack_path);

    let modlist = parse_wabbajack_file_typed(wabbajack_path).map_err(WjInstallError::Parse)?;

    let total_archives = modlist.archives.len();
    let total_directives = modlist.directives.len();

    log::info!(
        "Modlist '{}' v{}: {} archives, {} directives",
        modlist.name,
        modlist.version,
        total_archives,
        total_directives
    );

    // -----------------------------------------------------------------------
    // Step 2: Run pre-flight check
    // -----------------------------------------------------------------------
    let preflight_report = preflight_check(
        app,
        db,
        wabbajack_path,
        game_id,
        bottle_name,
        install_dir,
        download_dir,
    )
    .await?;

    if !preflight_report.can_proceed {
        let error_msgs: Vec<String> = preflight_report
            .issues
            .iter()
            .filter(|i| i.severity == "error")
            .map(|i| i.message.clone())
            .collect();
        return Err(WjInstallError::PreFlight(error_msgs.join("; ")));
    }

    // Collect warnings from pre-flight
    for issue in &preflight_report.issues {
        if issue.severity == "warning" {
            warnings.push(issue.message.clone());
        }
    }

    // Check cancellation
    if is_cancelled(&cancel_token) {
        return Err(WjInstallError::Cancelled);
    }

    // -----------------------------------------------------------------------
    // Step 3: Create DB record
    // -----------------------------------------------------------------------
    let install_id = db
        .create_wj_install(
            &modlist.name,
            &modlist.version,
            modlist.game_type,
            &install_dir.to_string_lossy(),
            total_archives,
            total_directives,
        )
        .map_err(|e| WjInstallError::Database(e.to_string()))?;

    log::info!("Created wabbajack_installs record: id={}", install_id);

    // Helper: update DB status on failure/cancel
    let _mark_failed = |db: &ModDatabase, id: i64, err: &str| {
        let _ = db.update_wj_install_status(id, "failed", Some(err));
    };
    let mark_cancelled = |db: &ModDatabase, id: i64| {
        let _ = db.update_wj_install_status(id, "cancelled", None);
    };

    // -----------------------------------------------------------------------
    // Step 4: Download phase
    // -----------------------------------------------------------------------
    db.update_wj_install_status(install_id, "downloading", None)
        .map_err(|e| WjInstallError::Database(e.to_string()))?;

    emit_progress(
        app,
        &WjInstallProgressEvent::DownloadPhaseStarted {
            total: total_archives,
        },
    );

    // Get Nexus API key and premium status for the downloader
    let (nexus_api_key, is_premium) = match oauth::get_auth_method() {
        oauth::AuthMethod::ApiKey(key) => {
            let client = nexus::NexusClient::new(key.clone());
            let premium = client.is_premium().await;
            (Some(key), premium)
        }
        oauth::AuthMethod::OAuth(tokens) => {
            let premium = oauth::parse_user_info(&tokens.access_token)
                .map(|u| u.is_premium)
                .unwrap_or(false);
            // OAuth doesn't provide an API key for NexusMods REST API;
            // NexusMods downloads will require manual action.
            (None, premium)
        }
        oauth::AuthMethod::None => (None, false),
    };

    let mut downloader = WjDownloader::new(nexus_api_key, is_premium, download_dir.to_path_buf());
    let archive_download_paths = downloader
        .download_all_archives(
            app,
            db,
            install_id,
            &modlist.archives,
            8,
            cancel_token.clone(),
        )
        .await
        .map_err(|e| {
            WjInstallError::Download(format!(
                "Download phase failed for '{}' ({} archives): {}",
                modlist.name, total_archives, e
            ))
        })?;

    db.update_wj_install_archive_progress(install_id, archive_download_paths.len() as i64)
        .map_err(|e| WjInstallError::Database(e.to_string()))?;

    // Check cancellation
    if is_cancelled(&cancel_token) {
        mark_cancelled(db, install_id);
        emit_progress(app, &WjInstallProgressEvent::InstallCancelled);
        return Err(WjInstallError::Cancelled);
    }

    // -----------------------------------------------------------------------
    // Step 5: Extraction phase
    // -----------------------------------------------------------------------
    db.update_wj_install_status(install_id, "extracting", None)
        .map_err(|e| WjInstallError::Database(e.to_string()))?;

    // Collect into owned Vec for 'static spawned tasks
    let archives_to_extract: Vec<(String, PathBuf)> = archive_download_paths
        .iter()
        .map(|(h, p)| (h.clone(), p.clone()))
        .collect();
    let extraction_count = archives_to_extract.len();

    emit_progress(
        app,
        &WjInstallProgressEvent::ExtractionStarted {
            total: extraction_count,
        },
    );

    // Map archive hash → extracted directory path
    let mut extracted_dirs: HashMap<String, PathBuf> = HashMap::new();
    let extraction_temp_base = install_dir.join(".wj_extraction_temp");

    // Parallel extraction: up to 6 concurrent extractions via semaphore
    let extract_semaphore = Arc::new(Semaphore::new(6));
    let mut extract_handles = Vec::with_capacity(extraction_count);

    for (idx, (hash, archive_path)) in archives_to_extract.iter().enumerate() {
        // Check cancellation before spawning each extraction task
        if is_cancelled(&cancel_token) {
            break;
        }

        let sem = Arc::clone(&extract_semaphore);
        let cancel = Arc::clone(&cancel_token);
        let hash_owned = hash.clone();
        let archive_owned = archive_path.clone();
        let extract_dest = extraction_temp_base.join(&hash_owned);
        let app_c = app.clone();
        let total = extraction_count;

        let handle = tokio::spawn(async move {
            let _permit = sem.acquire().await.expect("semaphore closed");

            // Check cancellation after acquiring permit
            if cancel.load(Ordering::Relaxed) {
                return (idx, hash_owned, Err("Cancelled".to_string()));
            }

            let archive_name = archive_owned
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| format!("archive_{}", idx));

            emit_progress(
                &app_c,
                &WjInstallProgressEvent::ExtractionProgress {
                    name: archive_name.clone(),
                    index: idx,
                    total,
                },
            );

            let dest = extract_dest.clone();
            let archive_for_blocking = archive_owned.clone();
            let result = tokio::task::spawn_blocking(move || {
                crate::installer::extract_archive(&archive_for_blocking, &dest)
            })
            .await;

            match result {
                Ok(Ok(_files)) => {
                    log::info!(
                        "Extracted archive {}/{}: {}",
                        idx + 1,
                        total,
                        archive_name
                    );
                    (idx, hash_owned, Ok(extract_dest))
                }
                Ok(Err(e)) => {
                    let err_msg = format!("Failed to extract '{}': {}", archive_name, e);
                    log::error!("{}", err_msg);
                    (idx, hash_owned, Err(err_msg))
                }
                Err(e) => {
                    let err_msg = format!("Extraction task panicked for '{}': {}", archive_name, e);
                    log::error!("{}", err_msg);
                    (idx, hash_owned, Err(err_msg))
                }
            }
        });

        extract_handles.push(handle);
    }

    // Collect extraction results
    let extract_results = futures::future::join_all(extract_handles).await;
    for join_result in extract_results {
        match join_result {
            Ok((_idx, hash, Ok(extract_dir))) => {
                extracted_dirs.insert(hash, extract_dir);
            }
            Ok((_idx, _hash, Err(err_msg))) => {
                warnings.push(err_msg);
            }
            Err(e) => {
                warnings.push(format!("Extraction join error: {}", e));
            }
        }
    }

    // Check cancellation
    if is_cancelled(&cancel_token) {
        mark_cancelled(db, install_id);
        emit_progress(app, &WjInstallProgressEvent::InstallCancelled);
        return Err(WjInstallError::Cancelled);
    }

    // -----------------------------------------------------------------------
    // Step 6: Directive processing phase
    // -----------------------------------------------------------------------
    db.update_wj_install_status(install_id, "processing", None)
        .map_err(|e| WjInstallError::Database(e.to_string()))?;

    emit_progress(
        app,
        &WjInstallProgressEvent::DirectivePhaseStarted {
            total: total_directives,
        },
    );

    // Determine game data directory for directive path substitution
    let game_dir = {
        let bottles = crate::bottles::detect_bottles();
        let bottle = bottles.iter().find(|b| b.name == bottle_name);
        bottle
            .and_then(|b| {
                crate::games::with_plugin(game_id, |plugin| plugin.detect(b).map(|g| g.data_dir))
                    .flatten()
            })
            .unwrap_or_else(|| install_dir.join("Data"))
    };

    let processor = DirectiveProcessor::new(
        wabbajack_path.to_path_buf(),
        extracted_dirs.clone(),
        install_dir.to_path_buf(),
        game_dir.clone(),
    );

    let app_clone = app.clone();
    let directive_count = modlist.directives.len();
    let progress_interval = match directive_count {
        0..=100 => 1,
        101..=500 => 10,
        501..=2000 => 25,
        _ => 50,
    };
    let directive_result = processor
        .process_all(&modlist.directives, &|current, total, phase| {
            if current % progress_interval == 0 || current == total {
                emit_progress(
                    &app_clone,
                    &WjInstallProgressEvent::DirectiveProgress {
                        current,
                        total,
                        directive_type: phase.to_string(),
                    },
                );
            }
        })
        .map_err(|e| {
            WjInstallError::Directive(format!(
                "Directive processing failed for '{}' ({} directives): {}",
                modlist.name, directive_count, e
            ))
        })?;

    let processed_count = directive_result.total_processed;

    // Merge directive warnings and errors
    for w in &directive_result.warnings {
        warnings.push(format!("Directive: {}", w));
    }
    for e in &directive_result.errors {
        warnings.push(format!("Directive error: {}", e));
    }

    db.update_wj_install_directive_progress(install_id, processed_count as i64)
        .map_err(|e| WjInstallError::Database(e.to_string()))?;

    log::info!(
        "Directive processing complete: {} processed, {} skipped, {} errors",
        directive_result.total_processed,
        directive_result.total_skipped,
        directive_result.errors.len(),
    );

    // Check cancellation
    if is_cancelled(&cancel_token) {
        mark_cancelled(db, install_id);
        emit_progress(app, &WjInstallProgressEvent::InstallCancelled);
        return Err(WjInstallError::Cancelled);
    }

    // -----------------------------------------------------------------------
    // Step 7: Deploy phase
    // -----------------------------------------------------------------------
    db.update_wj_install_status(install_id, "deploying", None)
        .map_err(|e| WjInstallError::Database(e.to_string()))?;

    // Collect all files produced by directive processing for deployment
    let mut deploy_files: Vec<String> = Vec::new();
    for entry in walkdir::WalkDir::new(install_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        if let Ok(rel) = entry.path().strip_prefix(install_dir) {
            let rel_str = rel.to_string_lossy().to_string();
            // Skip temp extraction dirs and hidden files
            if !rel_str.starts_with(".wj_") && !rel_str.starts_with('.') {
                deploy_files.push(rel_str);
            }
        }
    }

    emit_progress(
        app,
        &WjInstallProgressEvent::DeployStarted {
            total: deploy_files.len(),
        },
    );

    let files_deployed = if deploy_files.is_empty() {
        0usize
    } else {
        // Create a mod record for the modlist output
        let mod_id = db
            .add_mod(
                game_id,
                bottle_name,
                None, // no nexus_mod_id
                &modlist.name,
                &modlist.version,
                &format!("{}.wabbajack", modlist.name),
                &deploy_files,
            )
            .map_err(|e| WjInstallError::Database(e.to_string()))?;

        match crate::deployer::deploy_mod_atomic(
            db,
            game_id,
            bottle_name,
            mod_id,
            install_dir,
            &game_dir,
            &deploy_files,
        ) {
            Ok(deploy_result) => {
                emit_progress(
                    app,
                    &WjInstallProgressEvent::DeployProgress {
                        current: deploy_result.deployed_count,
                        total: deploy_files.len(),
                    },
                );

                log::info!(
                    "Deployed {} files ({} skipped, fallback: {})",
                    deploy_result.deployed_count,
                    deploy_result.skipped_count,
                    deploy_result.fallback_used,
                );

                deploy_result.deployed_count
            }
            Err(e) => {
                // Clean up orphaned mod record (deploy_mod_atomic already rolled back files)
                let _ = db.remove_mod(mod_id);
                let _ = db.update_wj_install_status(
                    install_id,
                    "failed",
                    Some(&format!("Deployment failed (rolled back): {}", e)),
                );
                return Err(WjInstallError::Deployment(format!(
                    "deploy_mod_atomic for modlist '{}' ({} files): {}",
                    modlist.name,
                    deploy_files.len(),
                    e
                )));
            }
        }
    };

    // -----------------------------------------------------------------------
    // Step 8: Mark completed in DB
    // -----------------------------------------------------------------------
    let elapsed = start_time.elapsed().as_secs_f64();

    db.update_wj_install_status(install_id, "completed", None)
        .map_err(|e| WjInstallError::Database(e.to_string()))?;

    // -----------------------------------------------------------------------
    // Step 9: Build and return result
    // -----------------------------------------------------------------------
    let result = WjInstallResult {
        install_id,
        status: "completed".to_string(),
        total_archives,
        total_directives,
        files_deployed,
        elapsed_secs: elapsed,
        warnings,
    };

    emit_progress(
        app,
        &WjInstallProgressEvent::InstallCompleted {
            result: result.clone(),
        },
    );

    log::info!(
        "Wabbajack install completed: id={}, archives={}, directives={}, elapsed={:.1}s",
        install_id,
        total_archives,
        total_directives,
        elapsed
    );

    // Clean up extraction temp directory
    if extraction_temp_base.exists() {
        if let Err(e) = std::fs::remove_dir_all(&extraction_temp_base) {
            log::warn!(
                "Failed to clean up extraction temp dir {:?}: {}",
                extraction_temp_base,
                e
            );
        }
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Tauri command: Run pre-flight check for a Wabbajack modlist.
///
/// Returns a WjPreflightReport with disk space, archive counts, and any
/// issues that would prevent installation.
#[tauri::command]
pub(crate) async fn wabbajack_preflight_cmd(
    app: AppHandle,
    state: tauri::State<'_, crate::AppState>,
    wabbajack_path: String,
    game_id: String,
    bottle_name: String,
    install_dir: String,
    download_dir: String,
) -> Result<WjPreflightReport, String> {
    let db = state.db.clone();
    preflight_check(
        &app,
        &db,
        Path::new(&wabbajack_path),
        &game_id,
        &bottle_name,
        Path::new(&install_dir),
        Path::new(&download_dir),
    )
    .await
    .map_err(|e| e.to_string())
}

/// Tauri command: Start a Wabbajack modlist installation.
///
/// Spawns the installation in a background tokio task and returns the
/// install_id immediately. Progress is emitted via `wj-install-progress`
/// events on the AppHandle.
#[tauri::command]
pub(crate) async fn install_wabbajack_modlist_cmd(
    app: AppHandle,
    state: tauri::State<'_, crate::AppState>,
    wabbajack_path: String,
    game_id: String,
    bottle_name: String,
    install_dir: String,
    download_dir: String,
) -> Result<i64, String> {
    let db = state.db.clone();
    let wj_path = PathBuf::from(&wabbajack_path);
    let inst_dir = PathBuf::from(&install_dir);
    let dl_dir = PathBuf::from(&download_dir);

    // Create a cancel token for this install
    let cancel_token = Arc::new(AtomicBool::new(false));

    // Create DB record first to get the install_id
    // We do a quick parse to get modlist metadata
    let modlist = parse_wabbajack_file_typed(&wj_path)
        .map_err(|e| format!("Failed to parse modlist: {}", e))?;

    let install_id = db
        .create_wj_install(
            &modlist.name,
            &modlist.version,
            modlist.game_type,
            &install_dir,
            modlist.archives.len(),
            modlist.directives.len(),
        )
        .map_err(|e| format!("Failed to create install record: {}", e))?;

    // Store the cancel token so it can be retrieved by cancel_wabbajack_install
    state
        .wj_cancel_tokens
        .lock()
        .unwrap()
        .insert(install_id, cancel_token.clone());

    // Spawn the installation task
    let app_clone = app.clone();
    let cancel_clone = cancel_token;
    tokio::spawn(async move {
        let result = install_wabbajack_modlist(
            &app_clone,
            &db,
            &wj_path,
            &game_id,
            &bottle_name,
            &inst_dir,
            &dl_dir,
            cancel_clone,
        )
        .await;

        match result {
            Ok(res) => {
                log::info!(
                    "Wabbajack install {} completed successfully",
                    res.install_id
                );
            }
            Err(WjInstallError::Cancelled) => {
                log::info!("Wabbajack install {} was cancelled", install_id);
                emit_progress(&app_clone, &WjInstallProgressEvent::InstallCancelled);
            }
            Err(e) => {
                log::error!("Wabbajack install {} failed: {}", install_id, e);
                emit_progress(
                    &app_clone,
                    &WjInstallProgressEvent::InstallFailed {
                        error: e.to_string(),
                    },
                );
            }
        }

        // Clean up the cancel token now that the install is finished
        let st = app_clone.state::<crate::AppState>();
        st.wj_cancel_tokens.lock().unwrap().remove(&install_id);
        log::info!("Cleaned up cancel token for wabbajack install {}", install_id);
    });

    Ok(install_id)
}

/// Tauri command: Cancel a running Wabbajack installation.
///
/// Sets the cancel token for the given install_id, which will be picked up
/// by the running install task at the next cancellation check point.
#[tauri::command]
pub(crate) async fn cancel_wabbajack_install(
    state: tauri::State<'_, crate::AppState>,
    install_id: i64,
) -> Result<(), String> {
    let tokens = state.wj_cancel_tokens.lock().unwrap();
    if let Some(token) = tokens.get(&install_id) {
        token.store(true, Ordering::Relaxed);
        log::info!(
            "Cancellation requested for wabbajack install {}",
            install_id
        );
        Ok(())
    } else {
        Err(format!("No active install found with id {}", install_id))
    }
}

/// Tauri command: Resume a paused/failed Wabbajack installation.
///
/// Resets the cancel token and updates DB status back to "downloading" so
/// the install can be re-triggered by the frontend.
#[tauri::command]
pub(crate) async fn resume_wabbajack_install(
    state: tauri::State<'_, crate::AppState>,
    install_id: i64,
) -> Result<(), String> {
    // Clear the cancellation flag if present
    {
        let tokens = state.wj_cancel_tokens.lock().unwrap();
        if let Some(token) = tokens.get(&install_id) {
            token.store(false, Ordering::Relaxed);
        }
    }

    // Update DB status back to "pending" so it can be restarted
    state
        .db
        .update_wj_install_status(install_id, "pending", None)
        .map_err(|e| format!("Database error: {}", e))?;

    log::info!("Resume requested for wabbajack install {}", install_id);
    Ok(())
}

/// Tauri command: Clean up the cancel token for a finished Wabbajack installation.
///
/// The backend auto-cleans tokens after install completion, but this command
/// provides an explicit cleanup path if the frontend detects a stale token.
#[tauri::command]
pub(crate) async fn cleanup_wabbajack_install(
    state: tauri::State<'_, crate::AppState>,
    install_id: i64,
) -> Result<(), String> {
    let removed = state
        .wj_cancel_tokens
        .lock()
        .unwrap()
        .remove(&install_id)
        .is_some();
    if removed {
        log::info!("Explicitly cleaned up cancel token for install {}", install_id);
    }
    Ok(())
}

/// Tauri command: Get the current progress/status of a Wabbajack installation.
///
/// Returns a JSON object with status, progress counters, and any error message.
#[tauri::command]
pub(crate) async fn get_wabbajack_install_status(
    state: tauri::State<'_, crate::AppState>,
    install_id: i64,
) -> Result<serde_json::Value, String> {
    let db = &state.db;

    let status_row = db
        .get_wj_install_status(install_id)
        .map_err(|e| format!("Database error: {}", e))?
        .ok_or_else(|| format!("Install {} not found", install_id))?;

    let (
        status,
        total_archives,
        completed_archives,
        total_directives,
        completed_directives,
        error_message,
    ) = status_row;

    Ok(serde_json::json!({
        "install_id": install_id,
        "status": status,
        "total_archives": total_archives,
        "completed_archives": completed_archives,
        "total_directives": total_directives,
        "completed_directives": completed_directives,
        "error_message": error_message,
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_available_disk_space() {
        // Should return non-zero for root
        let space = get_available_disk_space(Path::new("/"));
        assert!(space > 0, "Expected non-zero disk space for /");
    }

    #[test]
    fn test_get_available_disk_space_nonexistent() {
        // Should walk up to nearest existing ancestor
        let space = get_available_disk_space(Path::new("/tmp/nonexistent/deeply/nested/path"));
        assert!(
            space > 0,
            "Expected non-zero disk space for existing ancestor"
        );
    }

    #[test]
    fn test_is_cancelled() {
        let token = AtomicBool::new(false);
        assert!(!is_cancelled(&token));

        token.store(true, Ordering::Relaxed);
        assert!(is_cancelled(&token));
    }

    #[test]
    fn test_wj_install_result_serializes() {
        let result = WjInstallResult {
            install_id: 42,
            status: "completed".to_string(),
            total_archives: 100,
            total_directives: 5000,
            files_deployed: 4500,
            elapsed_secs: 123.45,
            warnings: vec!["test warning".to_string()],
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"install_id\":42"));
        assert!(json.contains("\"status\":\"completed\""));
    }

    #[test]
    fn test_preflight_report_serializes() {
        let report = WjPreflightReport {
            can_proceed: true,
            issues: vec![WjPreflightIssue {
                severity: "warning".to_string(),
                message: "Test warning".to_string(),
            }],
            total_download_size: 1024 * 1024 * 1024,
            total_archives: 50,
            total_directives: 2000,
            cached_archives: 10,
            disk_space_available: 100 * 1024 * 1024 * 1024,
            disk_space_required: 2 * 1024 * 1024 * 1024,
            nexus_archives: 30,
            is_nexus_premium: false,
            manual_downloads: 5,
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"can_proceed\":true"));
        assert!(json.contains("\"total_archives\":50"));
    }

    #[test]
    fn test_progress_event_serializes_tagged() {
        let event = WjInstallProgressEvent::DownloadStarted {
            name: "test.zip".to_string(),
            index: 0,
            total: 10,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"DownloadStarted\""));
        assert!(json.contains("\"name\":\"test.zip\""));

        let event2 = WjInstallProgressEvent::InstallCancelled;
        let json2 = serde_json::to_string(&event2).unwrap();
        assert!(json2.contains("\"type\":\"InstallCancelled\""));
    }
}
