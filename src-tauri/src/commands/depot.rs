//! Tauri commands for DepotDownloader integration.

use crate::depot_downloader;
use serde::Serialize;
use tauri::State;
use crate::AppState;

#[derive(Serialize)]
pub struct DDStatus {
    pub installed: bool,
    pub version: Option<String>,
    pub auth_state: String,
}

/// Check if DepotDownloader is installed and auth status.
#[tauri::command]
pub async fn dd_status() -> Result<DDStatus, String> {
    Ok(DDStatus {
        installed: depot_downloader::is_installed(),
        version: depot_downloader::installed_version(),
        auth_state: match depot_downloader::check_auth_state() {
            depot_downloader::AuthState::Ready => "ready".into(),
            depot_downloader::AuthState::NeedCredentials => "need_credentials".into(),
            depot_downloader::AuthState::NeedSteamGuard => "need_steam_guard".into(),
        },
    })
}

/// Install DepotDownloader from GitHub.
#[tauri::command]
pub async fn dd_install() -> Result<String, String> {
    depot_downloader::install().await.map_err(|e| e.to_string())
}

/// Authenticate with Steam via DepotDownloader.
#[tauri::command]
pub async fn dd_authenticate(
    username: String,
    password: String,
    steam_guard_code: Option<String>,
) -> Result<(), String> {
    depot_downloader::authenticate(
        &username,
        &password,
        steam_guard_code.as_deref(),
    )
    .await
    .map_err(|e| match e {
        depot_downloader::DDError::SteamGuardRequired => "STEAM_GUARD_REQUIRED".into(),
        depot_downloader::DDError::AuthFailed(msg) => format!("AUTH_FAILED: {}", msg),
        other => other.to_string(),
    })
}

/// List available manifests for a depot.
#[tauri::command]
pub async fn dd_list_manifests(
    app_id: u32,
    depot_id: u32,
) -> Result<Vec<depot_downloader::DepotManifest>, String> {
    depot_downloader::list_manifests(app_id, depot_id, None, None)
        .await
        .map_err(|e| e.to_string())
}

/// Download a specific depot version.
#[tauri::command]
pub async fn dd_download_depot(
    app_id: u32,
    depot_id: u32,
    manifest_id: String,
    game_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let download_dir = crate::config::cache_dir()
        .join("depot_downloads")
        .join(format!("{}_{}", app_id, depot_id));

    std::fs::create_dir_all(&download_dir).map_err(|e| e.to_string())?;

    depot_downloader::download_depot(
        app_id,
        depot_id,
        &manifest_id,
        &download_dir,
        None,
        None,
        None,
    )
    .await
    .map_err(|e| e.to_string())?;

    Ok(download_dir.to_string_lossy().to_string())
}
