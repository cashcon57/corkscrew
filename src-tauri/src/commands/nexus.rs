//! NexusMods authentication: SSO, OAuth, Google OAuth, mod files, and NXM protocol handling.

use crate::oauth;
use crate::config;
use crate::google_oauth;
use crate::nexus;
use crate::nexus_sso;
use crate::nxm_handler;
use crate::oauth::{NexusUserInfo, TokenPair};
use crate::{AppState, check_game_lock, nexus_client};
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
pub async fn start_oauth_login(app: tauri::AppHandle) -> Result<TokenPair, String> {
    oauth::start_oauth_flow(oauth::CLIENT_ID, |url| emit_oauth_url(&app, url))
        .await
        .map_err(|e| e.to_string())
}

/// Sends the sign-in URL to the UI so the user can copy it into another
/// browser or profile if the default browser opened the wrong one.
fn emit_oauth_url(app: &tauri::AppHandle, url: &str) {
    use tauri::Emitter;
    if let Err(e) = app.emit("nexus-oauth-url", url) {
        log::warn!("Failed to emit nexus-oauth-url: {e}");
    }
}

/// Stop a sign-in that is waiting for the browser.
#[tauri::command]
pub fn cancel_oauth_login() {
    oauth::cancel_oauth_flow();
}

/// Legacy command that accepts an explicit client_id (kept for compatibility).
#[tauri::command]
pub async fn start_nexus_oauth(
    app: tauri::AppHandle,
    client_id: String,
) -> Result<TokenPair, String> {
    oauth::start_oauth_flow(&client_id, |url| emit_oauth_url(&app, url))
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
    check_game_lock(&state.game_locks, &game_id, &bottle_name)?;

    // Name/version from the chosen file, not only the mod page.
    let (mod_name, mod_version) =
        crate::commands::mods::nexus_file_identity(&client, &game_slug, mod_id, file_id).await?;

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
        .clone()
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

    // Stage & Deploy (replaces the installed copy in place when this file,
    // or the pending update of a mod, is already installed)
    let _ = app.emit(
        "install-progress",
        serde_json::json!({
            "kind": "stepChanged",
            "mod_index": 0,
            "step": "installing",
            "detail": format!("Installing {}...", mod_name),
        }),
    );

    let _guard = crate::DeployGuard::try_acquire(state.deploy_in_progress.clone(), app.clone())?;
    let db = state.db.clone();
    let req = crate::commands::mods::NexusInstallRequest {
        game_id,
        bottle_name,
        archive_path,
        name: mod_name,
        version: mod_version,
        nexus_mod_id: mod_id,
        nexus_file_id: Some(file_id),
        source_url: format!("https://www.nexusmods.com/{}/mods/{}", game_slug, mod_id),
        replace_mod_id: None,
        delete_archive_after: dl_cfg
            .extra
            .get("auto_delete_archives")
            .and_then(|v: &serde_json::Value| v.as_str())
            == Some("true"),
    };
    let installed = tokio::task::spawn_blocking(move || {
        crate::commands::mods::install_nexus_archive_blocking(&db, &req)
    })
    .await
    .map_err(crate::format_join_error)??;

    let _ = app.emit(
        "install-progress",
        serde_json::json!({
            "kind": "modCompleted",
            "mod_index": 0,
            "mod_name": &installed.name,
            "mod_id": installed.id,
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

