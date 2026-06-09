//! NexusMods authentication: SSO, OAuth, Google OAuth, mod files, and NXM protocol handling.

use crate::oauth;
use crate::config;
use crate::deployer;
use crate::google_oauth;
use crate::nexus;
use crate::nexus_sso;
use crate::nxm_handler;
use crate::oauth::{NexusUserInfo, TokenPair};
use crate::staging;
use crate::{AppState, nexus_client, resolve_game_any_runtime};
use std::path::PathBuf;
use tauri::Emitter;
use tauri::{AppHandle, State};

// --- Nexus SSO ---

#[tauri::command]
pub async fn start_nexus_sso() -> Result<String, String> {
    // Run the blocking SSO WebSocket flow on a background thread
    tokio::task::spawn_blocking(nexus_sso::run_sso_flow)
        .await
        .map_err(|e| format!("SSO task failed: {}", e))?
        .map_err(|e| e.to_string())
}


// --- OAuth ---

/// Start OAuth login flow using the hardcoded Corkscrew client ID.
/// Opens the user's default browser to NexusMods for authorization.
#[tauri::command]
pub async fn start_oauth_login() -> Result<TokenPair, String> {
    oauth::start_oauth_flow(oauth::CLIENT_ID)
        .await
        .map_err(|e| e.to_string())
}

/// Legacy command that accepts an explicit client_id (kept for compatibility).
#[tauri::command]
pub async fn start_nexus_oauth(client_id: String) -> Result<TokenPair, String> {
    oauth::start_oauth_flow(&client_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn refresh_nexus_tokens(
    client_id: String,
    refresh_token: String,
) -> Result<TokenPair, String> {
    oauth::refresh_tokens(&client_id, &refresh_token)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn save_oauth_tokens(tokens: TokenPair) -> Result<(), String> {
    tokio::task::spawn_blocking(move || oauth::save_tokens(&tokens).map_err(|e| e.to_string()))
        .await
        .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn load_oauth_tokens() -> Result<Option<TokenPair>, String> {
    tokio::task::spawn_blocking(move || oauth::load_tokens().map_err(|e| e.to_string()))
        .await
        .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn clear_oauth_tokens() -> Result<(), String> {
    tokio::task::spawn_blocking(move || oauth::clear_tokens().map_err(|e| e.to_string()))
        .await
        .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn get_nexus_user_info(access_token: String) -> Result<NexusUserInfo, String> {
    tokio::task::spawn_blocking(move || {
        oauth::parse_user_info(&access_token).map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn get_auth_method_cmd() -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || {
        let method = oauth::get_auth_method();
        match method {
            oauth::AuthMethod::OAuth(ref tokens) => Ok(serde_json::json!({
                "type": "oauth",
                "expires_at": tokens.expires_at,
            })),
            oauth::AuthMethod::ApiKey(ref key) => Ok(serde_json::json!({
                "type": "api_key",
                "key_prefix": &key[..key.len().min(8)],
            })),
            oauth::AuthMethod::None => Ok(serde_json::json!({
                "type": "none",
            })),
        }
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn get_nexus_account_status() -> Result<serde_json::Value, String> {
    let method = oauth::get_auth_method_refreshed().await;
    match method {
        oauth::AuthMethod::OAuth(ref tokens) => {
            let user = oauth::parse_user_info(&tokens.access_token).map_err(|e| e.to_string())?;
            Ok(serde_json::json!({
                "connected": true,
                "auth_type": "oauth",
                "name": user.name,
                "email": user.email,
                "avatar": user.avatar,
                "is_premium": user.is_premium,
                "membership_roles": user.membership_roles,
            }))
        }
        oauth::AuthMethod::ApiKey(ref key) => {
            let client = nexus::NexusClient::new(key.clone());
            let info = client.validate_key().await.map_err(|e| e.to_string())?;
            let name = info
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let is_premium = info
                .get("is_premium")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let is_supporter = info
                .get("is_supporter")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let avatar = info
                .get("profile_url")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let email = info
                .get("email")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            Ok(serde_json::json!({
                "connected": true,
                "auth_type": "api_key",
                "name": name,
                "email": email,
                "avatar": avatar,
                "is_premium": is_premium || is_supporter,
                "membership_roles": [],
            }))
        }
        oauth::AuthMethod::None => Ok(serde_json::json!({
            "connected": false,
        })),
    }
}


// --- Google OAuth (Gemini) ---

#[tauri::command]
pub async fn google_sign_in() -> Result<google_oauth::GoogleAuthStatus, String> {
    google_oauth::start_google_oauth_flow()
        .await
        .map(|tokens| google_oauth::GoogleAuthStatus {
            signed_in: true,
            email: tokens.email,
            name: tokens.name,
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn google_sign_out() -> Result<(), String> {
    google_oauth::sign_out().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub fn google_auth_status() -> google_oauth::GoogleAuthStatus {
    google_oauth::get_google_auth_status()
}


// --- Nexus Mod Files & Direct Download ---

#[tauri::command]
pub async fn get_nexus_mod_files(
    game_slug: String,
    mod_id: i64,
) -> Result<Vec<nexus::NexusModFile>, String> {
    let client = nexus_client().await?;
    let raw_files = client
        .get_mod_files(&game_slug, mod_id)
        .await
        .map_err(|e| e.to_string())?;

    Ok(nexus::parse_mod_files(&raw_files, mod_id))
}

#[tauri::command]
pub async fn download_and_install_nexus_mod(
    app: AppHandle,
    game_slug: String,
    mod_id: i64,
    file_id: i64,
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let client = nexus_client().await?;

    // Enforce premium (backend safety check)
    if !client.is_premium().await {
        return Err("Premium membership required for direct downloads".to_string());
    }

    // Get mod info for name/version
    let mod_info = client
        .get_mod(&game_slug, mod_id)
        .await
        .map_err(|e| e.to_string())?;
    let mod_name = mod_info
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown Mod")
        .to_string();
    let mod_version = mod_info
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Emit progress: starting
    let _ = app.emit(
        "install-progress",
        serde_json::json!({
            "kind": "modStarted",
            "mod_index": 0,
            "total_mods": 1,
            "mod_name": &mod_name,
        }),
    );

    // Get download links (premium: no key/expires needed)
    let links = client
        .get_download_links(&game_slug, mod_id, file_id, None, None)
        .await
        .map_err(|e| e.to_string())?;
    let link = links.first().ok_or("No download links available")?;

    // Download
    let dl_cfg = config::get_config().map_err(|e| e.to_string())?;
    let download_dir = dl_cfg
        .download_dir
        .map(PathBuf::from)
        .unwrap_or_else(config::downloads_dir);

    let _ = app.emit(
        "install-progress",
        serde_json::json!({
            "kind": "stepChanged",
            "mod_index": 0,
            "step": "downloading",
            "detail": format!("Downloading {}...", mod_name),
        }),
    );

    let app_clone = app.clone();
    let dl_mod_name = mod_name.clone();
    let archive_path = client
        .download_file(
            &link.uri,
            &download_dir,
            Some(move |downloaded: u64, total: u64| {
                let _ = app_clone.emit(
                    "download-progress",
                    serde_json::json!({
                        "downloaded": downloaded,
                        "total": total,
                        "mod_name": &dl_mod_name,
                    }),
                );
            }),
        )
        .await
        .map_err(|e| e.to_string())?;

    // Stage & Deploy (reuse existing install pattern)
    let _ = app.emit(
        "install-progress",
        serde_json::json!({
            "kind": "stepChanged",
            "mod_index": 0,
            "step": "installing",
            "detail": format!("Installing {}...", mod_name),
        }),
    );

    let (_opt_bottle, game, data_dir) = resolve_game_any_runtime(&game_id, &bottle_name)?;
    let db = &state.db;

    let next_priority = db
        .get_next_priority(&game_id, &bottle_name)
        .map_err(|e| e.to_string())?;
    let db_mod_id = db
        .add_mod(
            &game_id,
            &bottle_name,
            Some(mod_id),
            &mod_name,
            &mod_version,
            &archive_path.to_string_lossy(),
            &[],
        )
        .map_err(|e| e.to_string())?;
    db.set_mod_priority(db_mod_id, next_priority)
        .map_err(|e| e.to_string())?;

    // Stage
    let staging_result =
        staging::stage_mod(&archive_path, &game_id, &bottle_name, db_mod_id, &mod_name).map_err(
            |e| {
                let _ = db.remove_mod(db_mod_id);
                format!("Staging failed: {e}")
            },
        )?;

    // Update DB
    db.set_staging_path(db_mod_id, &staging_result.staging_path.to_string_lossy())
        .map_err(|e| e.to_string())?;
    db.update_installed_files(db_mod_id, &staging_result.files)
        .map_err(|e| e.to_string())?;
    db.store_file_hashes(db_mod_id, &staging_result.hashes)
        .map_err(|e| e.to_string())?;

    // Deploy
    deployer::deploy_mod(
        db,
        &game_id,
        &bottle_name,
        db_mod_id,
        &staging_result.staging_path,
        &data_dir,
        &staging_result.files,
    )
    .map_err(|e| {
        let _ = staging::remove_staging(&staging_result.staging_path);
        let _ = db.remove_mod(db_mod_id);
        format!("Deploy failed: {e}")
    })?;

    // Set source
    let _ = db.set_mod_source(
        db_mod_id,
        "nexus",
        Some(&format!(
            "https://www.nexusmods.com/{}/mods/{}",
            game_slug, mod_id
        )),
    );

    // Sync plugins if Skyrim — Wine-only, requires a bottle.
    if game_id == "skyrimse" {
        if let Some(ref bottle) = _opt_bottle {
            let _ = crate::sync_plugins_for_game(&game, bottle);
        }
    }

    // Auto-delete archive if setting enabled
    if dl_cfg
        .extra
        .get("auto_delete_archives")
        .and_then(|v: &serde_json::Value| v.as_str())
        == Some("true")
    {
        let _ = std::fs::remove_file(&archive_path);
    }

    let installed = db
        .get_mod(db_mod_id)
        .map_err(|e| e.to_string())?
        .ok_or("Failed to retrieve installed mod")?;

    let _ = app.emit(
        "install-progress",
        serde_json::json!({
            "kind": "modCompleted",
            "mod_index": 0,
            "mod_name": &installed.name,
            "mod_id": db_mod_id,
        }),
    );

    serde_json::to_value(installed).map_err(|e| e.to_string())
}


// --- NXM Handler ---

#[tauri::command]
pub async fn register_nxm_handler() -> Result<(), String> {
    nxm_handler::register_nxm_handler()
}

#[tauri::command]
pub async fn unregister_nxm_handler() -> Result<(), String> {
    nxm_handler::unregister_nxm_handler()
}

#[tauri::command]
pub async fn is_nxm_handler_registered() -> Result<bool, String> {
    Ok(nxm_handler::is_nxm_handler_registered())
}

