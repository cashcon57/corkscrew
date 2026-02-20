//! OAuth 2.0 + PKCE authentication for Nexus Mods.
//!
//! Implements the Authorization Code flow with PKCE for desktop applications,
//! along with token storage, refresh, JWT parsing, and auth header generation.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::config;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const NEXUS_AUTH_URL: &str = "https://users.nexusmods.com/oauth/authorize";
const NEXUS_TOKEN_URL: &str = "https://users.nexusmods.com/oauth/token";
const OAUTH_SCOPES: &str = "openid public";
const CALLBACK_PATH: &str = "/callback";

/// How long (in seconds) to wait for the browser callback before timing out.
const CALLBACK_TIMEOUT_SECS: u64 = 300;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum OAuthError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Token exchange failed: {0}")]
    TokenExchange(String),

    #[error("Authorization cancelled or timed out")]
    Cancelled,

    #[error("Invalid token: {0}")]
    InvalidToken(String),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Configuration for the OAuth client.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OAuthConfig {
    pub client_id: String,
}

/// A pair of access and refresh tokens with expiration information.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix timestamp (seconds) at which the access token expires.
    pub expires_at: i64,
}

/// User information extracted from the JWT access token.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NexusUserInfo {
    pub name: String,
    pub email: Option<String>,
    pub avatar: Option<String>,
    pub is_premium: bool,
    pub membership_roles: Vec<String>,
}

/// Represents the authentication method in use.
#[derive(Clone, Debug)]
pub enum AuthMethod {
    OAuth(TokenPair),
    ApiKey(String),
    None,
}

// ---------------------------------------------------------------------------
// Internal: base64url encoding (no padding)
// ---------------------------------------------------------------------------

/// RFC 4648 base64url alphabet.
const BASE64URL_CHARS: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Encode bytes to base64url without padding.
fn base64url_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity((input.len() * 4 + 2) / 3);
    let mut i = 0;
    while i + 2 < input.len() {
        let b0 = input[i] as u32;
        let b1 = input[i + 1] as u32;
        let b2 = input[i + 2] as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(BASE64URL_CHARS[((triple >> 18) & 0x3F) as usize] as char);
        out.push(BASE64URL_CHARS[((triple >> 12) & 0x3F) as usize] as char);
        out.push(BASE64URL_CHARS[((triple >> 6) & 0x3F) as usize] as char);
        out.push(BASE64URL_CHARS[(triple & 0x3F) as usize] as char);
        i += 3;
    }
    let remaining = input.len() - i;
    if remaining == 2 {
        let b0 = input[i] as u32;
        let b1 = input[i + 1] as u32;
        let triple = (b0 << 16) | (b1 << 8);
        out.push(BASE64URL_CHARS[((triple >> 18) & 0x3F) as usize] as char);
        out.push(BASE64URL_CHARS[((triple >> 12) & 0x3F) as usize] as char);
        out.push(BASE64URL_CHARS[((triple >> 6) & 0x3F) as usize] as char);
    } else if remaining == 1 {
        let b0 = input[i] as u32;
        let triple = b0 << 16;
        out.push(BASE64URL_CHARS[((triple >> 18) & 0x3F) as usize] as char);
        out.push(BASE64URL_CHARS[((triple >> 12) & 0x3F) as usize] as char);
    }
    out
}

/// Decode a base64url-encoded string (with or without padding) to bytes.
fn base64url_decode(input: &str) -> Result<Vec<u8>, OAuthError> {
    // Also accept standard base64 by replacing + with - and / with _
    let input = input.replace('+', "-").replace('/', "_");
    // Strip padding
    let input = input.trim_end_matches('=');

    let mut out = Vec::with_capacity(input.len() * 3 / 4);

    let decode_char = |c: u8| -> Result<u8, OAuthError> {
        match c {
            b'A'..=b'Z' => Ok(c - b'A'),
            b'a'..=b'z' => Ok(c - b'a' + 26),
            b'0'..=b'9' => Ok(c - b'0' + 52),
            b'-' => Ok(62),
            b'_' => Ok(63),
            _ => Err(OAuthError::InvalidToken(format!(
                "invalid base64url character: {}",
                c as char
            ))),
        }
    };

    let bytes = input.as_bytes();
    let mut i = 0;

    while i + 3 < bytes.len() {
        let a = decode_char(bytes[i])? as u32;
        let b = decode_char(bytes[i + 1])? as u32;
        let c = decode_char(bytes[i + 2])? as u32;
        let d = decode_char(bytes[i + 3])? as u32;
        let triple = (a << 18) | (b << 12) | (c << 6) | d;
        out.push((triple >> 16) as u8);
        out.push((triple >> 8) as u8);
        out.push(triple as u8);
        i += 4;
    }

    let remaining = bytes.len() - i;
    if remaining == 3 {
        let a = decode_char(bytes[i])? as u32;
        let b = decode_char(bytes[i + 1])? as u32;
        let c = decode_char(bytes[i + 2])? as u32;
        let triple = (a << 18) | (b << 12) | (c << 6);
        out.push((triple >> 16) as u8);
        out.push((triple >> 8) as u8);
    } else if remaining == 2 {
        let a = decode_char(bytes[i])? as u32;
        let b = decode_char(bytes[i + 1])? as u32;
        let triple = (a << 18) | (b << 12);
        out.push((triple >> 16) as u8);
    }

    Ok(out)
}

// ---------------------------------------------------------------------------
// Internal: cryptographic random bytes
// ---------------------------------------------------------------------------

/// Generate `n` cryptographically random bytes using platform APIs.
fn random_bytes(n: usize) -> Result<Vec<u8>, OAuthError> {
    let mut buf = vec![0u8; n];

    #[cfg(unix)]
    {
        use std::io::Read;
        let mut f = fs::File::open("/dev/urandom")?;
        f.read_exact(&mut buf)?;
    }

    #[cfg(not(unix))]
    {
        // Fallback: use std::collections::hash_map::RandomState for entropy.
        // This is less ideal but works on all platforms.
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        use std::time::SystemTime;

        for chunk in buf.chunks_mut(8) {
            let mut hasher = DefaultHasher::new();
            SystemTime::now().hash(&mut hasher);
            std::thread::current().id().hash(&mut hasher);
            let val = hasher.finish().to_le_bytes();
            for (dst, src) in chunk.iter_mut().zip(val.iter()) {
                *dst = *src;
            }
        }
    }

    Ok(buf)
}

// ---------------------------------------------------------------------------
// PKCE
// ---------------------------------------------------------------------------

/// PKCE code verifier and challenge pair.
struct PkceChallenge {
    verifier: String,
    challenge: String,
}

/// Generate a PKCE code verifier (43-128 characters, unreserved URI chars)
/// and its S256 challenge.
fn generate_pkce() -> Result<PkceChallenge, OAuthError> {
    // Generate 32 random bytes -> base64url gives 43 chars (within 43-128).
    let bytes = random_bytes(32)?;
    let verifier = base64url_encode(&bytes);

    // S256: SHA-256 hash of the ASCII verifier, then base64url-encode.
    let hash = Sha256::digest(verifier.as_bytes());
    let challenge = base64url_encode(&hash);

    Ok(PkceChallenge {
        verifier,
        challenge,
    })
}

// ---------------------------------------------------------------------------
// Internal: generate random state parameter
// ---------------------------------------------------------------------------

fn generate_state() -> Result<String, OAuthError> {
    let bytes = random_bytes(16)?;
    Ok(base64url_encode(&bytes))
}

// ---------------------------------------------------------------------------
// Authorization URL builder
// ---------------------------------------------------------------------------

/// Build the authorization URL for the Nexus Mods OAuth flow.
fn build_auth_url(
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    code_challenge: &str,
) -> String {
    format!(
        "{}?client_id={}&response_type=code&scope={}&redirect_uri={}&state={}&code_challenge_method=S256&code_challenge={}",
        NEXUS_AUTH_URL,
        urlencoding::encode(client_id),
        urlencoding::encode(OAUTH_SCOPES),
        urlencoding::encode(redirect_uri),
        urlencoding::encode(state),
        urlencoding::encode(code_challenge),
    )
}

// ---------------------------------------------------------------------------
// Minimal URL-encoding (only for build_auth_url helper)
// ---------------------------------------------------------------------------

mod urlencoding {
    /// Percent-encode a string for use in URL query parameters.
    pub fn encode(input: &str) -> String {
        let mut out = String::with_capacity(input.len() * 3);
        for byte in input.bytes() {
            match byte {
                b'A'..=b'Z'
                | b'a'..=b'z'
                | b'0'..=b'9'
                | b'-'
                | b'_'
                | b'.'
                | b'~' => out.push(byte as char),
                _ => {
                    out.push('%');
                    out.push_str(&format!("{:02X}", byte));
                }
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Open browser
// ---------------------------------------------------------------------------

/// Open a URL in the user's default browser.
fn open_browser(url: &str) -> Result<(), OAuthError> {
    #[cfg(target_os = "macos")]
    let cmd = "open";

    #[cfg(target_os = "linux")]
    let cmd = "xdg-open";

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let cmd = "open"; // fallback

    std::process::Command::new(cmd)
        .arg(url)
        .spawn()
        .map_err(|e| OAuthError::Io(e))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Local callback server
// ---------------------------------------------------------------------------

/// Result from the callback server: the authorization code received.
struct CallbackResult {
    code: String,
    #[allow(dead_code)]
    state: String,
}

/// Parse query parameters from a URL path string (e.g. "/callback?code=xxx&state=yyy").
fn parse_query_params(path: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();

    if let Some(query) = path.split('?').nth(1) {
        for pair in query.split('&') {
            let mut parts = pair.splitn(2, '=');
            if let (Some(key), Some(value)) = (parts.next(), parts.next()) {
                // Minimal percent-decoding for the values we care about.
                let decoded = value
                    .replace("%20", " ")
                    .replace("%2F", "/")
                    .replace("%3D", "=")
                    .replace("%2B", "+")
                    .replace('+', " ");
                params.insert(key.to_string(), decoded);
            }
        }
    }

    params
}

/// Spin up a temporary HTTP server on a random port, wait for the OAuth
/// callback, then return the authorization code.
fn wait_for_callback(
    listener: &TcpListener,
    expected_state: &str,
) -> Result<CallbackResult, OAuthError> {
    listener.set_nonblocking(false)?;

    // Set a timeout so we don't block forever.
    let timeout = std::time::Duration::from_secs(CALLBACK_TIMEOUT_SECS);
    listener
        .set_nonblocking(false)
        .ok();

    // Use a blocking accept with a manual timeout via SO_RCVTIMEO equivalent.
    // TcpListener doesn't have set_timeout directly, so we use
    // incoming() with a thread-based timeout approach.
    let start = std::time::Instant::now();

    loop {
        // Check timeout
        if start.elapsed() > timeout {
            return Err(OAuthError::Cancelled);
        }

        // Try to accept with a short non-blocking poll
        listener.set_nonblocking(true)?;
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false)?;
                // Read the HTTP request
                let mut reader = BufReader::new(&stream);
                let mut request_line = String::new();
                reader.read_line(&mut request_line)?;

                // Parse the request line: "GET /callback?code=...&state=... HTTP/1.1"
                let path = request_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("")
                    .to_string();

                // Drain remaining headers (read until empty line)
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line)?;
                    if line.trim().is_empty() {
                        break;
                    }
                }

                // Only handle requests to /callback
                if !path.starts_with(CALLBACK_PATH) {
                    // Send 404 and continue listening
                    let response = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
                    let mut writer = stream.try_clone()?;
                    let _ = writer.write_all(response.as_bytes());
                    continue;
                }

                let params = parse_query_params(&path);

                // Check for errors from the OAuth provider
                if let Some(error) = params.get("error") {
                    let description = params
                        .get("error_description")
                        .cloned()
                        .unwrap_or_else(|| error.clone());

                    // Send error page
                    let body = format!(
                        "<html><body><h1>Authorization Failed</h1><p>{}</p>\
                         <p>You can close this tab.</p></body></html>",
                        description
                    );
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let mut writer = stream.try_clone()?;
                    let _ = writer.write_all(response.as_bytes());
                    let _ = writer.flush();

                    return Err(OAuthError::TokenExchange(description));
                }

                let code = params.get("code").cloned().ok_or_else(|| {
                    OAuthError::TokenExchange("no authorization code in callback".to_string())
                })?;

                let state = params.get("state").cloned().unwrap_or_default();

                // Verify state parameter matches
                if state != expected_state {
                    let body = "<html><body><h1>Authorization Failed</h1>\
                                <p>State mismatch - possible CSRF attack.</p>\
                                <p>You can close this tab.</p></body></html>";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let mut writer = stream.try_clone()?;
                    let _ = writer.write_all(response.as_bytes());
                    let _ = writer.flush();

                    return Err(OAuthError::TokenExchange(
                        "state parameter mismatch".to_string(),
                    ));
                }

                // Send success page
                let body = "<html><body style=\"font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; \
                            display: flex; justify-content: center; align-items: center; height: 100vh; margin: 0; \
                            background: #1a1a2e; color: #e0e0e0;\">\
                            <div style=\"text-align: center;\">\
                            <h1 style=\"color: #da8e35;\">Authorization Successful!</h1>\
                            <p>You can close this tab and return to Corkscrew.</p>\
                            </div></body></html>";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let mut writer = stream.try_clone()?;
                let _ = writer.write_all(response.as_bytes());
                let _ = writer.flush();

                return Ok(CallbackResult { code, state });
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // No connection yet; sleep briefly and retry.
                std::thread::sleep(std::time::Duration::from_millis(100));
                continue;
            }
            Err(e) => return Err(OAuthError::Io(e)),
        }
    }
}

// ---------------------------------------------------------------------------
// Token exchange response
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    #[allow(dead_code)]
    token_type: Option<String>,
}

// ---------------------------------------------------------------------------
// Token file path
// ---------------------------------------------------------------------------

/// Returns the path to the stored OAuth tokens file.
fn tokens_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("corkscrew")
        .join("nexus_tokens.json")
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Start the full OAuth 2.0 + PKCE authorization flow.
///
/// 1. Generate PKCE verifier/challenge and a random state.
/// 2. Bind a local HTTP server on a random port.
/// 3. Open the user's browser to the Nexus Mods authorization page.
/// 4. Wait for the redirect callback with the authorization code.
/// 5. Exchange the code for access + refresh tokens.
/// 6. Return the token pair.
pub async fn start_oauth_flow(client_id: &str) -> Result<TokenPair, OAuthError> {
    // 1. PKCE
    let pkce = generate_pkce()?;
    let state = generate_state()?;

    // 2. Local server
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://localhost:{}{}", port, CALLBACK_PATH);

    // 3. Build URL and open browser
    let auth_url = build_auth_url(client_id, &redirect_uri, &state, &pkce.challenge);
    open_browser(&auth_url)?;

    // 4. Wait for callback (this blocks until the user completes auth or timeout)
    let callback = tokio::task::spawn_blocking({
        let expected_state = state.clone();
        move || wait_for_callback(&listener, &expected_state)
    })
    .await
    .map_err(|_| OAuthError::Cancelled)??;

    // 5. Exchange code for tokens
    let client = reqwest::Client::new();
    let mut params = HashMap::new();
    params.insert("grant_type", "authorization_code");
    params.insert("code", &callback.code);
    params.insert("client_id", client_id);
    params.insert("redirect_uri", &redirect_uri);
    params.insert("code_verifier", &pkce.verifier);

    let response = client
        .post(NEXUS_TOKEN_URL)
        .form(&params)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(OAuthError::TokenExchange(format!(
            "token endpoint returned {}: {}",
            status, body
        )));
    }

    let token_response: TokenResponse = response.json().await?;

    let now = chrono::Utc::now().timestamp();
    let expires_at = now + token_response.expires_in.unwrap_or(3600);

    let tokens = TokenPair {
        access_token: token_response.access_token,
        refresh_token: token_response
            .refresh_token
            .unwrap_or_default(),
        expires_at,
    };

    // Auto-save tokens
    save_tokens(&tokens)?;

    Ok(tokens)
}

/// Refresh an expired access token using the refresh token.
pub async fn refresh_tokens(
    client_id: &str,
    refresh_token: &str,
) -> Result<TokenPair, OAuthError> {
    let client = reqwest::Client::new();
    let mut params = HashMap::new();
    params.insert("grant_type", "refresh_token");
    params.insert("refresh_token", refresh_token);
    params.insert("client_id", client_id);

    let response = client
        .post(NEXUS_TOKEN_URL)
        .form(&params)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(OAuthError::TokenExchange(format!(
            "refresh failed with {}: {}",
            status, body
        )));
    }

    let token_response: TokenResponse = response.json().await?;

    let now = chrono::Utc::now().timestamp();
    let expires_at = now + token_response.expires_in.unwrap_or(3600);

    let tokens = TokenPair {
        access_token: token_response.access_token,
        refresh_token: token_response
            .refresh_token
            .unwrap_or_else(|| refresh_token.to_string()),
        expires_at,
    };

    // Auto-save refreshed tokens
    save_tokens(&tokens)?;

    Ok(tokens)
}

/// Save tokens to the Corkscrew config directory.
pub fn save_tokens(tokens: &TokenPair) -> Result<(), OAuthError> {
    let path = tokens_path();

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let json = serde_json::to_string_pretty(tokens)?;
    fs::write(&path, format!("{json}\n"))?;

    // Set restrictive permissions on Unix (owner read/write only).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        fs::set_permissions(&path, perms)?;
    }

    Ok(())
}

/// Load saved tokens from disk, returning `None` if no token file exists.
pub fn load_tokens() -> Result<Option<TokenPair>, OAuthError> {
    let path = tokens_path();

    if !path.exists() {
        return Ok(None);
    }

    let contents = fs::read_to_string(&path)?;
    let tokens: TokenPair = serde_json::from_str(&contents)?;
    Ok(Some(tokens))
}

/// Remove saved tokens from disk.
pub fn clear_tokens() -> Result<(), OAuthError> {
    let path = tokens_path();

    if path.exists() {
        fs::remove_file(&path)?;
    }

    Ok(())
}

/// Parse user information from a JWT access token.
///
/// This performs a simple base64 decode of the JWT payload (middle segment)
/// without cryptographic signature verification, since we trust the token
/// came directly from the Nexus Mods token endpoint over HTTPS.
pub fn parse_user_info(access_token: &str) -> Result<NexusUserInfo, OAuthError> {
    let parts: Vec<&str> = access_token.split('.').collect();
    if parts.len() != 3 {
        return Err(OAuthError::InvalidToken(
            "JWT must have 3 dot-separated parts".to_string(),
        ));
    }

    let payload_bytes = base64url_decode(parts[1])?;
    let payload_str = String::from_utf8(payload_bytes).map_err(|e| {
        OAuthError::InvalidToken(format!("JWT payload is not valid UTF-8: {}", e))
    })?;

    let claims: serde_json::Value = serde_json::from_str(&payload_str)?;

    let name = claims
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    let email = claims
        .get("email")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let avatar = claims
        .get("avatar")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let membership_roles: Vec<String> = claims
        .get("membership_roles")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    // Determine premium status from membership_roles or premium_expiry.
    let is_premium = membership_roles
        .iter()
        .any(|r: &String| r.eq_ignore_ascii_case("premium") || r.eq_ignore_ascii_case("supporter"))
        || claims
            .get("premium_expiry")
            .and_then(|v| v.as_i64())
            .map(|exp| exp > chrono::Utc::now().timestamp())
            .unwrap_or(false);

    Ok(NexusUserInfo {
        name,
        email,
        avatar,
        is_premium,
        membership_roles,
    })
}

/// Determine the current authentication method.
///
/// Checks (in order):
/// 1. Saved OAuth tokens on disk.
/// 2. Legacy API key in the app configuration.
/// 3. Falls back to `AuthMethod::None`.
pub fn get_auth_method() -> AuthMethod {
    // Check for saved OAuth tokens first.
    if let Ok(Some(tokens)) = load_tokens() {
        return AuthMethod::OAuth(tokens);
    }

    // Fall back to legacy API key from config.
    if let Ok(Some(api_key)) = config::get_config_value("nexus_api_key") {
        if !api_key.is_empty() {
            return AuthMethod::ApiKey(api_key);
        }
    }

    AuthMethod::None
}

/// Build the appropriate HTTP headers for the given authentication method.
///
/// - `AuthMethod::OAuth` -> `Authorization: Bearer {access_token}`
/// - `AuthMethod::ApiKey` -> `apikey: {key}`
/// - `AuthMethod::None` -> empty headers
pub fn auth_headers(method: &AuthMethod) -> HeaderMap {
    let mut headers = HeaderMap::new();

    match method {
        AuthMethod::OAuth(tokens) => {
            if let Ok(val) = HeaderValue::from_str(&format!("Bearer {}", tokens.access_token)) {
                headers.insert(AUTHORIZATION, val);
            }
        }
        AuthMethod::ApiKey(key) => {
            if let Ok(val) = HeaderValue::from_str(key) {
                headers.insert("apikey", val);
            }
        }
        AuthMethod::None => {}
    }

    headers
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pkce_generation() {
        let pkce = generate_pkce().expect("PKCE generation should succeed");

        // Verifier must be 43-128 characters of unreserved URI characters.
        assert!(
            pkce.verifier.len() >= 43 && pkce.verifier.len() <= 128,
            "verifier length {} should be 43-128",
            pkce.verifier.len()
        );

        // Verifier should only contain base64url characters.
        assert!(
            pkce.verifier
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "verifier contains invalid characters: {}",
            pkce.verifier
        );

        // Challenge should be the base64url-encoded SHA-256 of the verifier.
        let expected_hash = Sha256::digest(pkce.verifier.as_bytes());
        let expected_challenge = base64url_encode(&expected_hash);
        assert_eq!(pkce.challenge, expected_challenge);

        // Challenge should be 43 characters (256 bits / 6 bits per char = ~43).
        assert_eq!(
            pkce.challenge.len(),
            43,
            "S256 challenge should be 43 base64url characters"
        );
    }

    #[test]
    fn test_build_auth_url() {
        let url = build_auth_url(
            "my-client-id",
            "http://localhost:12345/callback",
            "random-state",
            "challenge-value",
        );

        assert!(url.starts_with(NEXUS_AUTH_URL));
        assert!(url.contains("client_id=my-client-id"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("scope=openid%20public"));
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A12345%2Fcallback"));
        assert!(url.contains("state=random-state"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("code_challenge=challenge-value"));
    }

    #[test]
    fn test_parse_jwt_user_info() {
        // Build a fake JWT with known claims.
        let header = base64url_encode(b"{\"alg\":\"RS256\",\"typ\":\"JWT\"}");
        let payload_json = serde_json::json!({
            "name": "TestUser",
            "email": "test@example.com",
            "avatar": "https://avatars.nexusmods.com/test.png",
            "membership_roles": ["premium", "member"],
            "premium_expiry": 9999999999i64,
            "sub": "12345"
        });
        let payload = base64url_encode(payload_json.to_string().as_bytes());
        let signature = base64url_encode(b"fake-signature");
        let fake_jwt = format!("{}.{}.{}", header, payload, signature);

        let info = parse_user_info(&fake_jwt).expect("should parse JWT");

        assert_eq!(info.name, "TestUser");
        assert_eq!(info.email.as_deref(), Some("test@example.com"));
        assert_eq!(
            info.avatar.as_deref(),
            Some("https://avatars.nexusmods.com/test.png")
        );
        assert!(info.is_premium);
        assert_eq!(info.membership_roles, vec!["premium", "member"]);
    }

    #[test]
    fn test_parse_jwt_minimal() {
        // JWT with minimal claims (no email, avatar, etc.).
        let header = base64url_encode(b"{\"alg\":\"RS256\"}");
        let payload_json = serde_json::json!({
            "name": "BasicUser",
            "sub": "67890"
        });
        let payload = base64url_encode(payload_json.to_string().as_bytes());
        let signature = base64url_encode(b"sig");
        let fake_jwt = format!("{}.{}.{}", header, payload, signature);

        let info = parse_user_info(&fake_jwt).expect("should parse JWT");

        assert_eq!(info.name, "BasicUser");
        assert!(info.email.is_none());
        assert!(info.avatar.is_none());
        assert!(!info.is_premium);
        assert!(info.membership_roles.is_empty());
    }

    #[test]
    fn test_parse_jwt_invalid() {
        // Not a JWT at all.
        let result = parse_user_info("not-a-jwt");
        assert!(result.is_err());

        // Too few parts.
        let result = parse_user_info("part1.part2");
        assert!(result.is_err());
    }

    #[test]
    fn test_token_save_load_roundtrip() {
        // Use a temp directory to avoid touching real config.
        let dir = tempfile::tempdir().expect("create tempdir");
        let path = dir.path().join("nexus_tokens.json");

        let tokens = TokenPair {
            access_token: "access-abc-123".to_string(),
            refresh_token: "refresh-xyz-789".to_string(),
            expires_at: 1700000000,
        };

        // Write directly to the temp path.
        let json = serde_json::to_string_pretty(&tokens).expect("serialize");
        fs::write(&path, format!("{json}\n")).expect("write");

        // Read back.
        let contents = fs::read_to_string(&path).expect("read");
        let loaded: TokenPair = serde_json::from_str(&contents).expect("deserialize");

        assert_eq!(loaded.access_token, "access-abc-123");
        assert_eq!(loaded.refresh_token, "refresh-xyz-789");
        assert_eq!(loaded.expires_at, 1700000000);
    }

    #[test]
    fn test_auth_headers_oauth() {
        let tokens = TokenPair {
            access_token: "my-access-token".to_string(),
            refresh_token: "my-refresh-token".to_string(),
            expires_at: 9999999999,
        };
        let method = AuthMethod::OAuth(tokens);
        let headers = auth_headers(&method);

        assert_eq!(
            headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
            "Bearer my-access-token"
        );
        assert!(headers.get("apikey").is_none());
    }

    #[test]
    fn test_auth_headers_api_key() {
        let method = AuthMethod::ApiKey("legacy-api-key-123".to_string());
        let headers = auth_headers(&method);

        assert_eq!(
            headers.get("apikey").unwrap().to_str().unwrap(),
            "legacy-api-key-123"
        );
        assert!(headers.get(AUTHORIZATION).is_none());
    }

    #[test]
    fn test_auth_headers_none() {
        let method = AuthMethod::None;
        let headers = auth_headers(&method);

        assert!(headers.is_empty());
    }

    #[test]
    fn test_base64url_roundtrip() {
        let test_cases: Vec<&[u8]> = vec![
            b"",
            b"f",
            b"fo",
            b"foo",
            b"foob",
            b"fooba",
            b"foobar",
            b"\x00\x01\x02\xff\xfe\xfd",
        ];

        for input in test_cases {
            let encoded = base64url_encode(input);
            let decoded = base64url_decode(&encoded).expect("decode should succeed");
            assert_eq!(
                decoded, input,
                "roundtrip failed for input {:?}",
                input
            );
        }
    }

    #[test]
    fn test_parse_query_params() {
        let params = parse_query_params("/callback?code=abc123&state=xyz789&extra=hello%20world");
        assert_eq!(params.get("code").unwrap(), "abc123");
        assert_eq!(params.get("state").unwrap(), "xyz789");
        assert_eq!(params.get("extra").unwrap(), "hello world");
    }

    #[test]
    fn test_parse_query_params_empty() {
        let params = parse_query_params("/callback");
        assert!(params.is_empty());
    }

    #[test]
    fn test_generate_state() {
        let state1 = generate_state().expect("generate state");
        let state2 = generate_state().expect("generate state");

        // States should be non-empty.
        assert!(!state1.is_empty());
        assert!(!state2.is_empty());

        // Two generated states should (almost certainly) be different.
        assert_ne!(state1, state2);
    }

    #[test]
    fn test_tokens_path() {
        let path = tokens_path();
        assert!(path.ends_with("corkscrew/nexus_tokens.json"));
    }
}
