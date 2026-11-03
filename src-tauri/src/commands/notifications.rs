//! Notifications, error tracking, download queue, update checking, and browser webview management.

use crate::config;
use crate::database::{self, InstalledMod};
use crate::download_queue;
use crate::mod_updates;
use crate::nexus;
use crate::nexus::ModUpdateInfo;
use crate::{AppState, check_game_lock, nexus_client};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{mpsc, Semaphore};

// --- Download Queue ---

#[tauri::command]
pub fn get_download_queue(state: State<AppState>) -> Vec<download_queue::QueueItem> {
    state.download_queue.get_all()
}

#[tauri::command]
pub fn get_download_queue_counts(state: State<AppState>) -> download_queue::QueueCounts {
    state.download_queue.status_counts()
}

#[tauri::command]
pub async fn retry_download(id: u64, state: State<'_, AppState>) -> Result<bool, String> {
    let queue = state.download_queue.clone();
    tokio::task::spawn_blocking(move || Ok(queue.mark_for_retry(id)))
        .await
        .map_err(crate::format_join_error)?
}

#[tauri::command]
pub fn cancel_download(id: u64, state: State<AppState>) -> Result<(), String> {
    state.download_queue.set_cancelled(id);
    Ok(())
}

#[tauri::command]
pub fn clear_finished_downloads(state: State<AppState>) -> usize {
    state.download_queue.clear_finished()
}


// --- Notification Log ---

#[tauri::command]
pub async fn get_notification_log(
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<database::NotificationEntry>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        db.get_notifications(limit.unwrap_or(50))
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn clear_notification_log(state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || db.clear_notifications().map_err(|e| e.to_string()))
        .await
        .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn log_notification(
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
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn get_notification_count(state: State<'_, AppState>) -> Result<usize, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || db.notification_count().map_err(|e| e.to_string()))
        .await
        .map_err(crate::format_join_error)?
}


// --- Error Event Diagnostics ---

#[tauri::command]
pub async fn record_error_event_cmd(
    module: String,
    error_type: String,
    message: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        db.record_error_event(&module, &error_type, &message)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn get_error_summary(
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> Result<Vec<database::ErrorEvent>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        db.get_error_summary(limit.unwrap_or(20))
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}


// --- Update Checking ---

/// Nexus game domain for a game, falling back to a static mapping when the
/// game can't be resolved (e.g. bottle temporarily unavailable).
fn nexus_slug_for(game_id: &str, bottle_name: &str) -> String {
    crate::resolve_game(game_id, bottle_name)
        .ok()
        .map(|(_, g, _)| g.nexus_slug)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| mod_updates::fallback_game_slug(game_id).to_string())
}

fn archive_basename(archive_name: &str) -> &str {
    archive_name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(archive_name)
}

/// Check Nexus for updates to every installed mod that came from Nexus.
///
/// Results are stored (so they survive a restart) and logged. Each mod page
/// is fetched once even when several of its files are installed. Mods
/// installed before Corkscrew recorded file ids get their file id filled in
/// when their archive name matches a file on the page.
#[tauri::command]
pub async fn check_mod_updates(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<ModUpdateInfo>, String> {
    let client = nexus_client().await?;

    let db = state.db.clone();
    let (gid, bn) = (game_id.clone(), bottle_name.clone());
    let (mods, game_slug) = tokio::task::spawn_blocking(move || {
        let mods = db.list_mods(&gid, &bn).map_err(|e| e.to_string())?;
        Ok::<_, String>((mods, nexus_slug_for(&gid, &bn)))
    })
    .await
    .map_err(crate::format_join_error)??;

    let mut by_page: BTreeMap<i64, Vec<&InstalledMod>> = BTreeMap::new();
    for m in &mods {
        if let Some(nid) = m.nexus_mod_id {
            by_page.entry(nid).or_default().push(m);
        }
    }
    if by_page.is_empty() {
        log::info!("Nexus update check ({game_id}/{bottle_name}): no Nexus mods installed");
        return Ok(vec![]);
    }

    let mut rows: Vec<database::UpdateCheckRow> = Vec::new();
    let mut backfill: Vec<(i64, i64, i64)> = Vec::new();
    let mut updates: Vec<ModUpdateInfo> = Vec::new();
    let mut failed_pages = 0usize;
    let mut failed_mods = 0usize;
    let mut untracked = 0usize;
    let mut first_error: Option<String> = None;

    for (nexus_id, group) in &by_page {
        let (raw_files, raw_links) = match client.get_mod_files_with_updates(&game_slug, *nexus_id).await {
            Ok(r) => r,
            Err(e) => {
                failed_pages += 1;
                failed_mods += group.len();
                for m in group {
                    log::warn!(
                        "Update check failed for '{}' (Nexus mod {}): {}",
                        m.name,
                        nexus_id,
                        e
                    );
                }
                first_error.get_or_insert_with(|| e.to_string());
                continue;
            }
        };
        let files = mod_updates::parse_file_entries(&raw_files);
        let links = mod_updates::parse_file_updates(&raw_links);

        for m in group {
            let file_id = m.nexus_file_id.or_else(|| {
                let archive = archive_basename(&m.archive_name);
                let found = files
                    .iter()
                    .find(|f| !f.file_name.is_empty() && f.file_name.eq_ignore_ascii_case(archive))
                    .map(|f| f.file_id);
                if let Some(fid) = found {
                    backfill.push((m.id, *nexus_id, fid));
                }
                found
            });
            let Some(file_id) = file_id else {
                untracked += 1;
                log::debug!(
                    "Skipping update check for '{}' (Nexus mod {}): installed file unknown",
                    m.name,
                    nexus_id
                );
                continue;
            };
            let latest = mod_updates::find_update(file_id, &m.version, &files, &links);
            if let Some(f) = latest {
                log::info!(
                    "Update available: '{}' v{} (file {}) -> v{} (file {} '{}')",
                    m.name,
                    m.version,
                    file_id,
                    f.version,
                    f.file_id,
                    f.name
                );
                updates.push(ModUpdateInfo {
                    mod_id: m.id,
                    nexus_mod_id: *nexus_id,
                    mod_name: m.name.clone(),
                    current_version: m.version.clone(),
                    latest_version: f.version.clone(),
                    latest_file_name: f.name.clone(),
                    latest_file_id: f.file_id,
                });
            }
            rows.push(database::UpdateCheckRow {
                mod_id: m.id,
                nexus_mod_id: *nexus_id,
                installed_file_id: Some(file_id),
                latest: latest.map(|f| (f.file_id, f.version.clone(), f.name.clone())),
            });
        }
    }

    let checked = rows.len();
    let db = state.db.clone();
    let (gid, bn) = (game_id.clone(), bottle_name.clone());
    tokio::task::spawn_blocking(move || {
        for (mod_id, nexus_id, file_id) in backfill {
            if let Err(e) = db.set_nexus_ids(mod_id, nexus_id, Some(file_id)) {
                log::warn!("Failed to record Nexus file id for mod {mod_id}: {e}");
            }
        }
        db.save_update_check_results(&gid, &bn, &rows)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)??;

    log::info!(
        "Nexus update check ({game_id}/{bottle_name}): {checked} mods checked, {} updates, \
         {failed_mods} failed ({failed_pages} pages), {untracked} skipped (installed file unknown)",
        updates.len()
    );

    if checked == 0 && failed_pages > 0 {
        return Err(first_error.unwrap_or_else(|| "Update check failed".to_string()));
    }
    Ok(updates)
}

/// Updates found by the last check that haven't been applied yet.
#[tauri::command]
pub async fn get_mod_update_results(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<ModUpdateInfo>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        db.list_pending_updates(&game_id, &bottle_name)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[derive(Clone, Debug, Serialize)]
pub struct RoutedUpdate {
    pub mod_id: i64,
    pub mod_name: String,
    pub route: mod_updates::UpdateRoute,
}

/// How each pending update should be fetched: Premium accounts download
/// through the API, free accounts get the file's page on the Nexus website.
#[tauri::command]
pub async fn route_mod_updates(
    game_id: String,
    bottle_name: String,
    mod_ids: Vec<i64>,
    state: State<'_, AppState>,
) -> Result<Vec<RoutedUpdate>, String> {
    let is_premium = crate::commands::mods::is_nexus_premium().await.unwrap_or_else(|e| {
        log::warn!("Could not determine Nexus Premium status, treating as free: {e}");
        false
    });
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let slug = nexus_slug_for(&game_id, &bottle_name);
        let pending = db
            .list_pending_updates(&game_id, &bottle_name)
            .map_err(|e| e.to_string())?;
        Ok(pending
            .into_iter()
            .filter(|u| mod_ids.contains(&u.mod_id))
            .map(|u| RoutedUpdate {
                mod_id: u.mod_id,
                route: mod_updates::route_update(is_premium, &slug, u.nexus_mod_id, u.latest_file_id),
                mod_name: u.mod_name,
            })
            .collect())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[derive(Clone, Debug, Serialize)]
pub struct UpdateOutcome {
    pub mod_id: i64,
    pub mod_name: String,
    pub ok: bool,
    pub new_version: Option<String>,
    pub error: Option<String>,
    /// Extra information, e.g. that a collection mod now diverges from the
    /// file its collection pins.
    pub note: Option<String>,
}

static UPDATES_CANCELLED: AtomicBool = AtomicBool::new(false);

/// Stop a running "Update All" after the mods already in progress.
#[tauri::command]
pub fn cancel_mod_updates() {
    UPDATES_CANCELLED.store(true, Ordering::SeqCst);
}

const UPDATE_PROGRESS_EVENT: &str = "mod-update-progress";
const MAX_CONCURRENT_UPDATE_DOWNLOADS: usize = 3;

fn is_transient(e: &nexus::NexusError) -> bool {
    match e {
        nexus::NexusError::Http(_) | nexus::NexusError::Io(_) => true,
        nexus::NexusError::Api { status, .. } => *status >= 500,
        _ => false,
    }
}

/// Download one update through the API (Premium only; `get_download_links`
/// enforces that again) with backoff on transient failures.
async fn download_update(
    client: &nexus::NexusClient,
    app: &AppHandle,
    game_slug: &str,
    update: &ModUpdateInfo,
    download_dir: &Path,
) -> Result<PathBuf, String> {
    const BACKOFF_SECS: [u64; 2] = [2, 6];
    let mut attempt = 0usize;
    loop {
        if UPDATES_CANCELLED.load(Ordering::SeqCst) {
            return Err("Cancelled".to_string());
        }
        let result = async {
            let links = client
                .get_download_links(game_slug, update.nexus_mod_id, update.latest_file_id, None, None)
                .await?;
            let link = links.first().ok_or(nexus::NexusError::NoDownloadLinks)?;
            let app = app.clone();
            let mod_id = update.mod_id;
            client
                .download_file(
                    &link.uri,
                    download_dir,
                    Some(move |downloaded: u64, total: u64| {
                        let _ = app.emit(
                            UPDATE_PROGRESS_EVENT,
                            serde_json::json!({
                                "mod_id": mod_id,
                                "stage": "downloading",
                                "downloaded": downloaded,
                                "total": total,
                            }),
                        );
                    }),
                )
                .await
        }
        .await;
        match result {
            Ok(path) => return Ok(path),
            Err(e) if is_transient(&e) && attempt < BACKOFF_SECS.len() => {
                log::warn!(
                    "Download of the update for '{}' failed (attempt {}), retrying: {e}",
                    update.mod_name,
                    attempt + 1
                );
                tokio::time::sleep(std::time::Duration::from_secs(BACKOFF_SECS[attempt])).await;
                attempt += 1;
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// Update mods in place from Nexus (Premium accounts only).
///
/// Downloads run concurrently (bounded); each finished download is installed
/// right away while the others continue. Every update replaces its mod's
/// files in place, keeping load order position, enabled state and collection
/// membership. Free accounts are refused here: they must use the website.
#[tauri::command]
pub async fn update_mods_from_nexus(
    app: AppHandle,
    game_id: String,
    bottle_name: String,
    mod_ids: Vec<i64>,
    state: State<'_, AppState>,
) -> Result<Vec<UpdateOutcome>, String> {
    check_game_lock(&state.game_locks, &game_id, &bottle_name)?;
    let client = nexus_client().await?;
    // NexusMods compliance: never automate downloads for free accounts.
    if !client.is_premium().await {
        return Err(
            "Automatic updates need a Nexus Mods Premium account. Download the updates from the Nexus website instead."
                .to_string(),
        );
    }
    let _guard = crate::DeployGuard::try_acquire(state.deploy_in_progress.clone(), app.clone())?;
    UPDATES_CANCELLED.store(false, Ordering::SeqCst);

    let db = state.db.clone();
    let (gid, bn) = (game_id.clone(), bottle_name.clone());
    let (pending, existing, game_slug) = tokio::task::spawn_blocking(move || {
        let pending = db.list_pending_updates(&gid, &bn).map_err(|e| e.to_string())?;
        let existing: Vec<mod_updates::ModIdentity> = db
            .list_mods(&gid, &bn)
            .map_err(|e| e.to_string())?
            .iter()
            .map(mod_updates::ModIdentity::from)
            .collect();
        Ok::<_, String>((pending, existing, nexus_slug_for(&gid, &bn)))
    })
    .await
    .map_err(crate::format_join_error)??;

    let todo: Vec<ModUpdateInfo> = pending
        .into_iter()
        .filter(|u| mod_ids.contains(&u.mod_id))
        .collect();
    let total = todo.len();
    if total == 0 {
        return Ok(vec![]);
    }
    log::info!("Updating {total} mods from Nexus ({game_id}/{bottle_name})");

    let cfg = config::get_config().map_err(|e| e.to_string())?;
    let download_dir = cfg
        .download_dir
        .clone()
        .map(PathBuf::from)
        .unwrap_or_else(config::downloads_dir);
    let delete_archives = cfg
        .extra
        .get("auto_delete_archives")
        .and_then(|v| v.as_str())
        == Some("true");

    // Downloads: semaphore-gated tasks that report back as they finish.
    let client = Arc::new(client);
    let sem = Arc::new(Semaphore::new(MAX_CONCURRENT_UPDATE_DOWNLOADS));
    let (tx, mut rx) = mpsc::unbounded_channel::<(ModUpdateInfo, Result<PathBuf, String>)>();
    for update in todo {
        let (client, sem, tx, app) = (client.clone(), sem.clone(), tx.clone(), app.clone());
        let (slug, dir) = (game_slug.clone(), download_dir.clone());
        tokio::spawn(async move {
            let result = match sem.acquire_owned().await {
                Ok(_permit) => download_update(&client, &app, &slug, &update, &dir).await,
                Err(e) => Err(e.to_string()),
            };
            let _ = tx.send((update, result));
        });
    }
    drop(tx);

    // Install loop: one at a time (deploys must not overlap), as soon as
    // each download is ready.
    let mut outcomes = Vec::with_capacity(total);
    while let Some((update, downloaded)) = rx.recv().await {
        let note = existing
            .iter()
            .find(|m| m.id == update.mod_id)
            .and_then(|m| {
                m.collection_name
                    .as_deref()
                    .map(|c| mod_updates::collection_divergence_note(&m.name, c))
            });
        let result = match downloaded {
            Err(e) => Err(e),
            Ok(_) if UPDATES_CANCELLED.load(Ordering::SeqCst) => Err("Cancelled".to_string()),
            Ok(archive) => {
                let _ = app.emit(
                    UPDATE_PROGRESS_EVENT,
                    serde_json::json!({ "mod_id": update.mod_id, "stage": "installing" }),
                );
                let db = state.db.clone();
                let (gid, bn) = (game_id.clone(), bottle_name.clone());
                let u = update.clone();
                tokio::task::spawn_blocking(move || {
                    let (_, game, data_dir) = crate::resolve_game(&gid, &bn)?;
                    let spec = mod_updates::ReplaceSpec {
                        name: u.mod_name.clone(),
                        version: u.latest_version.clone(),
                        archive_name: archive.to_string_lossy().into_owned(),
                        nexus_ids: Some((u.nexus_mod_id, Some(u.latest_file_id))),
                    };
                    let installed = mod_updates::replace_mod_in_place(
                        &db, &gid, &bn, &game, &data_dir, u.mod_id, &archive, &spec,
                    )?;
                    if delete_archives {
                        if let Err(e) = std::fs::remove_file(&archive) {
                            log::warn!("Failed to auto-delete {}: {e}", archive.display());
                        }
                    }
                    Ok::<_, String>(installed)
                })
                .await
                .map_err(crate::format_join_error)
                .and_then(|r| r)
            }
        };
        let outcome = match result {
            Ok(installed) => {
                log::info!(
                    "Updated '{}' {} -> {}",
                    update.mod_name,
                    update.current_version,
                    installed.version
                );
                UpdateOutcome {
                    mod_id: update.mod_id,
                    mod_name: update.mod_name.clone(),
                    ok: true,
                    new_version: Some(installed.version),
                    error: None,
                    note,
                }
            }
            Err(e) => {
                log::warn!("Updating '{}' failed: {e}", update.mod_name);
                UpdateOutcome {
                    mod_id: update.mod_id,
                    mod_name: update.mod_name.clone(),
                    ok: false,
                    new_version: None,
                    error: Some(e),
                    note: None,
                }
            }
        };
        let _ = app.emit(
            UPDATE_PROGRESS_EVENT,
            serde_json::json!({
                "mod_id": outcome.mod_id,
                "stage": if outcome.ok { "done" } else { "failed" },
                "completed": outcomes.len() + 1,
                "total": total,
                "error": outcome.error,
            }),
        );
        outcomes.push(outcome);
    }

    // Plugin order once for the whole batch.
    let (gid, bn) = (game_id.clone(), bottle_name.clone());
    let synced = tokio::task::spawn_blocking(move || {
        let (bottle, game, _) = crate::resolve_game(&gid, &bn)?;
        crate::sync_plugins_for_game(&game, &bottle)
    })
    .await
    .map_err(crate::format_join_error)?;
    if let Err(e) = synced {
        log::warn!("Plugin sync after updates failed: {e}");
    }

    let ok = outcomes.iter().filter(|o| o.ok).count();
    log::info!(
        "Nexus updates ({game_id}/{bottle_name}): {ok} of {total} updated, {} failed",
        total - ok
    );
    Ok(outcomes)
}


// --- Browser WebView Management ---

#[tauri::command]
pub async fn create_browser_webview(
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
pub async fn resize_browser_webview(
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
pub async fn close_browser_webview(app: AppHandle) -> Result<(), String> {
    if let Some(webview) = app.get_webview("browser-panel") {
        webview.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn navigate_browser_webview(app: AppHandle, url: String) -> Result<(), String> {
    let webview = app
        .get_webview("browser-panel")
        .ok_or("Browser panel not found")?;
    let parsed_url: tauri::Url = url.parse().map_err(|e: url::ParseError| e.to_string())?;
    webview.navigate(parsed_url).map_err(|e| e.to_string())?;
    Ok(())
}


