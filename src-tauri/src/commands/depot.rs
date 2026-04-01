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
    if app_id == 0 {
        return Err("app_id cannot be 0".into());
    }
    if depot_id == 0 {
        return Err("depot_id cannot be 0".into());
    }
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
    _game_id: String,
    _state: State<'_, AppState>,
) -> Result<String, String> {
    if app_id == 0 || depot_id == 0 {
        return Err("app_id and depot_id must be non-zero".into());
    }
    if manifest_id.is_empty() || manifest_id.len() > 30 || !manifest_id.chars().all(|c| c.is_ascii_digit()) {
        return Err("manifest_id must be a non-empty numeric string".into());
    }
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

/// Apply a downloaded depot to a game installation.
/// Copies all files from the depot download directory over the game files.
#[tauri::command]
pub async fn dd_apply_depot(
    game_id: String,
    bottle_name: String,
    depot_dir: String,
    state: State<'_, AppState>,
) -> Result<u64, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let (_bottle, game, _data_dir) = crate::resolve_game(&game_id, &bottle_name)?;
        let depot_path = std::path::Path::new(&depot_dir);
        crate::downgrader::apply_depot_to_game(&game.game_path, depot_path, &game_id)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}
