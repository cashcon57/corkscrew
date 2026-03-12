//! Google OAuth 2.0 + PKCE for Gemini Code Assist.
//!
//! Implements the Authorization Code flow with PKCE for desktop applications,
//! using Google's Code Assist endpoint (cloudcode-pa.googleapis.com).
//! Each user gets a managed GCP project auto-provisioned by Google.

use std::collections::HashMap;
use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::oauth;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";
const GOOGLE_USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v3/userinfo";

/// Scopes needed for Gemini API + user info.
/// cloud-platform is required per Google's OAuth docs for generativelanguage.googleapis.com.
const SCOPES: &str = "openid https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile https://www.googleapis.com/auth/generative-language.peruserquota";

/// OAuth callback path.
const CALLBACK_PATH: &str = "/callback";

/// Refresh tokens this many seconds before they actually expire.
const REFRESH_MARGIN_SECS: i64 = 60;

const GOOGLE_CLIENT_ID: &str =
    "440303664335-9pgcd0u055bcjl7g03nmsaoj623s8m11.apps.googleusercontent.com";
const GOOGLE_CLIENT_SECRET: &str = "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";

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum GoogleOAuthError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Token exchange failed: {0}")]
    TokenExchange(String),

    #[error("Authorization cancelled or timed out")]
    Cancelled,

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Not signed in")]
    NotSignedIn,
}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GoogleTokens {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix timestamp (seconds) at which the access token expires.
    pub expires_at: i64,
    /// User email from the ID token or userinfo.
    #[serde(default)]
    pub email: Option<String>,
    /// User display name.
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GoogleAuthStatus {
    pub signed_in: bool,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

// ---------------------------------------------------------------------------
// Token file path
// ---------------------------------------------------------------------------

fn tokens_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("corkscrew")
        .join("google_tokens.json")
}

// ---------------------------------------------------------------------------
// Token persistence
// ---------------------------------------------------------------------------

pub fn save_google_tokens(tokens: &GoogleTokens) -> Result<(), GoogleOAuthError> {
    let path = tokens_path();

    if let Some(parent) = path.parent() {
        #[cfg(unix)]
        let _old_umask = unsafe { libc::umask(0o077) };
        let dir_result = fs::create_dir_all(parent);
        #[cfg(unix)]
        unsafe {
            libc::umask(_old_umask);
        }
        dir_result?;
    }

    let json = serde_json::to_string_pretty(tokens)?;

    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)?;
        file.write_all(format!("{json}\n").as_bytes())?;
    }
    #[cfg(not(unix))]
    {
        fs::write(&path, format!("{json}\n"))?;
    }

    Ok(())
}

pub fn load_google_tokens() -> Result<Option<GoogleTokens>, GoogleOAuthError> {
    let path = tokens_path();
    if !path.exists() {
        return Ok(None);
    }
    let contents = fs::read_to_string(&path)?;
    let tokens: GoogleTokens = serde_json::from_str(&contents)?;
    Ok(Some(tokens))
}

pub fn clear_google_tokens() -> Result<(), GoogleOAuthError> {
    let path = tokens_path();
    if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// OAuth flow
// ---------------------------------------------------------------------------

/// Start the Google OAuth 2.0 + PKCE flow.
///
/// 1. Generate PKCE verifier/challenge and random state.
/// 2. Bind a local HTTP server on a random port.
/// 3. Open the user's browser to the Google consent page.
/// 4. Wait for the redirect callback with the authorization code.
/// 5. Exchange the code for access + refresh tokens.
/// 6. Fetch user info.
/// 7. Return the tokens.
pub async fn start_google_oauth_flow() -> Result<GoogleTokens, GoogleOAuthError> {
    let pkce = oauth::generate_pkce().map_err(|e| GoogleOAuthError::TokenExchange(e.to_string()))?;
    let state =
        oauth::generate_state().map_err(|e| GoogleOAuthError::TokenExchange(e.to_string()))?;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://127.0.0.1:{}{}", port, CALLBACK_PATH);

    // Build Google auth URL
    let auth_url = format!(
        "{}?client_id={}&response_type=code&scope={}&redirect_uri={}&state={}&code_challenge_method=S256&code_challenge={}&access_type=offline&prompt=consent",
        GOOGLE_AUTH_URL,
        oauth::urlencoding::encode(GOOGLE_CLIENT_ID),
        oauth::urlencoding::encode(SCOPES),
        oauth::urlencoding::encode(&redirect_uri),
        oauth::urlencoding::encode(&state),
        oauth::urlencoding::encode(&pkce.challenge),
    );

    oauth::open_browser(&auth_url).map_err(|e| GoogleOAuthError::Io(std::io::Error::other(e.to_string())))?;

    // Wait for callback
    let callback = tokio::task::spawn_blocking({
        let expected_state = state.clone();
        move || {
            oauth::wait_for_callback(&listener, &expected_state)
                .map_err(|e| GoogleOAuthError::TokenExchange(e.to_string()))
        }
    })
    .await
    .map_err(|_| GoogleOAuthError::Cancelled)??;

    // Exchange code for tokens
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let mut params = HashMap::new();
    params.insert("grant_type", "authorization_code");
    params.insert("code", &callback.code);
    params.insert("client_id", GOOGLE_CLIENT_ID);
    params.insert("client_secret", GOOGLE_CLIENT_SECRET);
    params.insert("redirect_uri", &redirect_uri);
    params.insert("code_verifier", &pkce.verifier);

    let response = client.post(GOOGLE_TOKEN_URL).form(&params).send().await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(GoogleOAuthError::TokenExchange(format!(
            "token endpoint returned {}: {}",
            status, body
        )));
    }

    let token_response: TokenResponse = response.json().await?;

    let now = chrono::Utc::now().timestamp();
    let expires_at = now + token_response.expires_in.unwrap_or(3600);

    // Fetch user info
    let (email, name) = fetch_user_info(&client, &token_response.access_token).await;

    let tokens = GoogleTokens {
        access_token: token_response.access_token,
        refresh_token: token_response.refresh_token.unwrap_or_default(),
        expires_at,
        email,
        name,
    };

    save_google_tokens(&tokens)?;
    Ok(tokens)
}

/// Refresh an expired Google access token.
pub async fn refresh_google_tokens(refresh_token: &str) -> Result<GoogleTokens, GoogleOAuthError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let mut params = HashMap::new();
    params.insert("grant_type", "refresh_token");
    params.insert("refresh_token", refresh_token);
    params.insert("client_id", GOOGLE_CLIENT_ID);
    params.insert("client_secret", GOOGLE_CLIENT_SECRET);

    let response = client.post(GOOGLE_TOKEN_URL).form(&params).send().await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(GoogleOAuthError::TokenExchange(format!(
            "refresh failed with {}: {}",
            status, body
        )));
    }

    let token_response: TokenResponse = response.json().await?;

    let now = chrono::Utc::now().timestamp();
    let expires_at = now + token_response.expires_in.unwrap_or(3600);

    // Fetch user info with fresh token
    let (email, name) = fetch_user_info(&client, &token_response.access_token).await;

    let tokens = GoogleTokens {
        access_token: token_response.access_token,
        refresh_token: token_response
            .refresh_token
            .unwrap_or_else(|| refresh_token.to_string()),
        expires_at,
        email,
        name,
    };

    save_google_tokens(&tokens)?;
    Ok(tokens)
}

/// Load tokens, auto-refresh if expired, return a fresh access_token.
pub async fn ensure_valid_token() -> Result<String, GoogleOAuthError> {
    let tokens = load_google_tokens()?.ok_or(GoogleOAuthError::NotSignedIn)?;

    let now = chrono::Utc::now().timestamp();
    if tokens.expires_at > now + REFRESH_MARGIN_SECS {
        return Ok(tokens.access_token);
    }

    if tokens.refresh_token.is_empty() {
        clear_google_tokens()?;
        return Err(GoogleOAuthError::NotSignedIn);
    }

    match refresh_google_tokens(&tokens.refresh_token).await {
        Ok(new_tokens) => Ok(new_tokens.access_token),
        Err(e) => {
            log::warn!("[google_oauth] refresh failed, clearing tokens: {e}");
            clear_google_tokens()?;
            Err(GoogleOAuthError::NotSignedIn)
        }
    }
}

/// Get the current Google auth status.
pub fn get_google_auth_status() -> GoogleAuthStatus {
    match load_google_tokens() {
        Ok(Some(tokens)) => GoogleAuthStatus {
            signed_in: true,
            email: tokens.email,
            name: tokens.name,
        },
        _ => GoogleAuthStatus {
            signed_in: false,
            email: None,
            name: None,
        },
    }
}

/// Sign out: revoke token + clear stored tokens.
pub async fn sign_out() -> Result<(), GoogleOAuthError> {
    if let Ok(Some(tokens)) = load_google_tokens() {
        // Best-effort revoke
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()?;
        let _ = client
            .post(GOOGLE_REVOKE_URL)
            .form(&[("token", &tokens.access_token)])
            .send()
            .await;
    }
    clear_google_tokens()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    #[allow(dead_code)]
    token_type: Option<String>,
}

/// Fetch user info from Google's userinfo endpoint.
async fn fetch_user_info(
    client: &reqwest::Client,
    access_token: &str,
) -> (Option<String>, Option<String>) {
    match client
        .get(GOOGLE_USERINFO_URL)
        .header("Authorization", format!("Bearer {access_token}"))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(info) = resp.json::<serde_json::Value>().await {
                let email = info.get("email").and_then(|e| e.as_str()).map(String::from);
                let name = info.get("name").and_then(|n| n.as_str()).map(String::from);
                return (email, name);
            }
            (None, None)
        }
        _ => (None, None),
    }
}
