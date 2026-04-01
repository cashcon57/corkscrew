//! Notifications, error tracking, download queue, update checking, and browser webview management.

use crate::database;
use crate::download_queue;
use crate::nexus;
use crate::nexus::ModUpdateInfo;
use crate::{AppState, nexus_client};
use tauri::{AppHandle, Manager, State};

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
        .map_err(|e| format!("Task failed: {e}"))?
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
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn clear_notification_log(state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || db.clear_notifications().map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
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
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn get_notification_count(state: State<'_, AppState>) -> Result<usize, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || db.notification_count().map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
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
    .map_err(|e| format!("Task failed: {e}"))?
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
    .map_err(|e| format!("Task failed: {e}"))?
}


// --- Update Checking ---

#[tauri::command]
pub async fn check_mod_updates(
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


