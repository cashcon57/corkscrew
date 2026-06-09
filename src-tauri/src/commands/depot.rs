//! Tauri commands for DepotDownloader integration.

use crate::depot_downloader;
use crate::AppState;
use serde::Serialize;
use tauri::{Emitter, State};

#[derive(Serialize)]
pub struct DDStatus {
    pub installed: bool,
    pub version: Option<String>,
    pub auth_state: String,
}

/// Check if DepotDownloader is installed and auth status.
/// If a username is provided (or was previously saved), does a live auth check.
#[tauri::command]
pub async fn dd_status(username: Option<String>) -> Result<DDStatus, String> {
    let user = username.or_else(|| {
        crate::config::get_config_value("steam_username")
            .ok()
            .flatten()
    });

    let auth_state = if let Some(ref u) = user {
        depot_downloader::check_auth_state_live(u).await
    } else {
        depot_downloader::check_auth_state()
    };

    Ok(DDStatus {
        installed: depot_downloader::is_installed(),
        version: depot_downloader::installed_version(),
        auth_state: match auth_state {
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

/// Ensure DD is installed and up-to-date. Installs or updates as needed.
#[tauri::command]
pub async fn dd_ensure_updated() -> Result<String, String> {
    depot_downloader::ensure_up_to_date()
        .await
        .map_err(|e| e.to_string())
}

/// Authenticate with Steam via DepotDownloader.
#[tauri::command]
pub async fn dd_authenticate(
    username: String,
    password: String,
    steam_guard_code: Option<String>,
) -> Result<(), String> {
    depot_downloader::authenticate(&username, &password, steam_guard_code.as_deref())
        .await
        .map_err(|e| match e {
            depot_downloader::DDError::SteamGuardRequired => "STEAM_GUARD_REQUIRED".into(),
            depot_downloader::DDError::SteamGuardMobile => "STEAM_GUARD_MOBILE".into(),
            depot_downloader::DDError::AuthFailed(msg) => format!("AUTH_FAILED: {}", msg),
            other => other.to_string(),
        })
}

/// Clear saved Steam credentials for DepotDownloader.
#[tauri::command]
pub async fn dd_logout() -> Result<(), String> {
    depot_downloader::logout().map_err(|e| e.to_string())
}

/// Check for a partial (interrupted) depot download.
/// Returns the directory path and file count if a partial download exists.
#[tauri::command]
pub async fn dd_check_partial_download(
    app_id: u32,
    depot_id: u32,
) -> Result<Option<PartialDownloadInfo>, String> {
    let download_dir = crate::config::cache_dir()
        .join("depot_downloads")
        .join(format!("{}_{}", app_id, depot_id));

    if !download_dir.exists() {
        return Ok(None);
    }

    // Count files and total size
    let mut file_count: u64 = 0;
    let mut total_bytes: u64 = 0;
    fn walk(dir: &std::path::Path, count: &mut u64, bytes: &mut u64) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    if meta.is_file() {
                        *count += 1;
                        *bytes += meta.len();
                    } else if meta.is_dir() {
                        walk(&entry.path(), count, bytes);
                    }
                }
            }
        }
    }
    walk(&download_dir, &mut file_count, &mut total_bytes);

    if file_count == 0 {
        // Empty dir — clean it up
        let _ = std::fs::remove_dir_all(&download_dir);
        return Ok(None);
    }

    Ok(Some(PartialDownloadInfo {
        path: download_dir.to_string_lossy().to_string(),
        file_count,
        total_bytes,
    }))
}

/// Delete a partial depot download.
#[tauri::command]
pub async fn dd_delete_partial_download(app_id: u32, depot_id: u32) -> Result<(), String> {
    let download_dir = crate::config::cache_dir()
        .join("depot_downloads")
        .join(format!("{}_{}", app_id, depot_id));

    if download_dir.exists() {
        std::fs::remove_dir_all(&download_dir).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[derive(Serialize)]
pub struct PartialDownloadInfo {
    pub path: String,
    pub file_count: u64,
    pub total_bytes: u64,
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

/// Download a specific depot version. Emits `dd-download-progress` events.
#[tauri::command]
pub async fn dd_download_depot(
    app_id: u32,
    depot_id: u32,
    manifest_id: String,
    _game_id: String,
    _state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    if app_id == 0 || depot_id == 0 {
        return Err("app_id and depot_id must be non-zero".into());
    }
    if manifest_id.is_empty()
        || manifest_id.len() > 30
        || !manifest_id.chars().all(|c| c.is_ascii_digit())
    {
        return Err("manifest_id must be a non-empty numeric string".into());
    }
    let download_dir = crate::config::cache_dir()
        .join("depot_downloads")
        .join(format!("{}_{}", app_id, depot_id));

    std::fs::create_dir_all(&download_dir).map_err(|e| e.to_string())?;

    let progress_cb = {
        let handle = app_handle.clone();
        move |progress: depot_downloader::DowngradeProgress| {
            let _ = handle.emit("dd-download-progress", &progress);
        }
    };

    depot_downloader::download_depot(
        app_id,
        depot_id,
        &manifest_id,
        &download_dir,
        None,
        None,
        Some(&progress_cb),
    )
    .await
    .map_err(|e| e.to_string())?;

    Ok(download_dir.to_string_lossy().to_string())
}

/// Get depot history for a game (all captured versions with app/depot/manifest IDs).
#[tauri::command]
pub async fn dd_get_depot_versions(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<DepotVersion>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let conn = db.conn().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT game_version, app_id, depot_id, manifest_id, build_id
             FROM steam_depot_history
             WHERE game_id = ?1 AND game_version IS NOT NULL
             GROUP BY game_version
             ORDER BY captured_at DESC",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map(rusqlite::params![&game_id], |row| {
                Ok(DepotVersion {
                    game_version: row.get(0)?,
                    app_id: row.get::<_, String>(1)?.parse().unwrap_or(0),
                    depot_id: row.get::<_, String>(2)?.parse().unwrap_or(0),
                    manifest_id: row.get(3)?,
                    build_id: row.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?;

        Ok(rows.filter_map(|r| r.ok()).collect())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[derive(Serialize)]
pub struct DepotVersion {
    pub game_version: String,
    pub app_id: u32,
    pub depot_id: u32,
    pub manifest_id: String,
    pub build_id: String,
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
    let _db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let (_bottle, game, _data_dir) = crate::resolve_game(&game_id, &bottle_name)?;
        let depot_path = std::path::Path::new(&depot_dir);
        crate::downgrader::apply_depot_to_game(&game.game_path, depot_path, &game_id)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}
