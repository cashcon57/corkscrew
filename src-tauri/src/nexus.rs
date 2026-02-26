use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, USER_AGENT};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

const NEXUS_API_BASE: &str = "https://api.nexusmods.com/v1";

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum NexusError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Invalid NXM link: {0}")]
    InvalidNxmLink(String),

    #[error("URL parse error: {0}")]
    UrlParse(#[from] url::ParseError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("API error ({status}): {message}")]
    Api { status: u16, message: String },

    #[error("Missing download links in API response")]
    NoDownloadLinks,

    #[error("Rate limited by NexusMods API (retry after {retry_after}s)")]
    RateLimited { retry_after: u64 },

    #[error("Authentication error: {0}")]
    Auth(String),
}

pub type Result<T> = std::result::Result<T, NexusError>;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Metadata for a single file available on a Nexus Mods page.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NexusModFile {
    pub mod_id: i64,
    pub file_id: i64,
    pub name: String,
    pub version: String,
    pub file_name: String,
    pub size_kb: i64,
    pub description: String,
    pub category: String,
}

/// Parse raw JSON file entries from the NexusMods API into typed structs.
///
/// The `default_mod_id` is used when individual file entries don't include
/// a `mod_id` field (the v1 /files.json endpoint omits it since the mod_id
/// is already in the URL).
pub fn parse_mod_files(files: &[serde_json::Value], default_mod_id: i64) -> Vec<NexusModFile> {
    fn category_name(id: i64) -> &'static str {
        match id {
            1 => "main",
            2 => "update",
            3 => "optional",
            4 => "old_version",
            5 => "miscellaneous",
            6 => "deleted",
            7 => "archived",
            _ => "unknown",
        }
    }

    files
        .iter()
        .filter_map(|f| {
            Some(NexusModFile {
                mod_id: f.get("mod_id").and_then(|v| v.as_i64()).unwrap_or(default_mod_id),
                file_id: f.get("file_id")?.as_i64()?,
                name: f.get("name")?.as_str().unwrap_or("").to_string(),
                version: f.get("version")?.as_str().unwrap_or("").to_string(),
                file_name: f.get("file_name")?.as_str().unwrap_or("").to_string(),
                size_kb: f.get("size_kb").and_then(|v| v.as_i64()).unwrap_or(0),
                description: f
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                category: category_name(f.get("category_id").and_then(|v| v.as_i64()).unwrap_or(0))
                    .to_string(),
            })
        })
        .collect()
}

/// A parsed `nxm://` link handed to the application by the browser / OS.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NXMLink {
    pub game_slug: String,
    pub mod_id: i64,
    pub file_id: i64,
    pub key: Option<String>,
    pub expires: Option<String>,
}

impl NXMLink {
    /// Parse an `nxm://` URL into its components.
    ///
    /// Expected format:
    /// `nxm://<game>/mods/<mod_id>/files/<file_id>?key=xxx&expires=xxx`
    pub fn parse(url: &str) -> Result<Self> {
        // url::Url does not recognise `nxm` as a scheme with authority,
        // so we swap it for `http` to leverage its parser.
        let normalised = if let Some(stripped) = url.strip_prefix("nxm://") {
            format!("http://{}", stripped)
        } else {
            return Err(NexusError::InvalidNxmLink(format!(
                "URL does not start with nxm://: {url}"
            )));
        };

        let parsed = Url::parse(&normalised)?;

        // Host is the game slug.
        let game_slug = parsed
            .host_str()
            .ok_or_else(|| NexusError::InvalidNxmLink("missing game slug".into()))?
            .to_string();

        // Path segments: ["mods", "<mod_id>", "files", "<file_id>"]
        let segments: Vec<&str> = parsed
            .path_segments()
            .ok_or_else(|| NexusError::InvalidNxmLink("missing path segments".into()))?
            .collect();

        if segments.len() < 4 || segments[0] != "mods" || segments[2] != "files" {
            return Err(NexusError::InvalidNxmLink(format!(
                "unexpected path structure: {url}"
            )));
        }

        let mod_id: i64 = segments[1]
            .parse()
            .map_err(|_| NexusError::InvalidNxmLink(format!("invalid mod_id: {}", segments[1])))?;

        let file_id: i64 = segments[3]
            .parse()
            .map_err(|_| NexusError::InvalidNxmLink(format!("invalid file_id: {}", segments[3])))?;

        // Optional query parameters.
        let mut key: Option<String> = None;
        let mut expires: Option<String> = None;

        for (k, v) in parsed.query_pairs() {
            match k.as_ref() {
                "key" => key = Some(v.into_owned()),
                "expires" => expires = Some(v.into_owned()),
                _ => {}
            }
        }

        Ok(Self {
            game_slug,
            mod_id,
            file_id,
            key,
            expires,
        })
    }
}

/// Query input for checking mod updates.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModUpdateQuery {
    pub local_mod_id: i64,
    pub nexus_mod_id: i64,
    pub nexus_file_id: Option<i64>,
    pub mod_name: String,
    pub current_version: String,
}

/// Result of an update check for a single mod.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModUpdateInfo {
    pub mod_id: i64,
    pub nexus_mod_id: i64,
    pub mod_name: String,
    pub current_version: String,
    pub latest_version: String,
    pub latest_file_name: String,
    pub latest_file_id: i64,
}

/// A single CDN download link returned by the Nexus Mods API.
#[derive(Clone, Debug, Deserialize)]
pub struct DownloadLink {
    #[serde(rename = "URI")]
    pub uri: String,
    pub name: String,
    pub short_name: String,
}

/// Mod info returned from NexusMods browse/search endpoints.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NexusModInfo {
    pub mod_id: i64,
    pub name: String,
    pub summary: String,
    pub description: Option<String>,
    pub author: String,
    pub category_id: i64,
    pub version: String,
    pub endorsement_count: i64,
    pub unique_downloads: i64,
    pub picture_url: Option<String>,
    pub updated_at: Option<String>,
    pub created_at: Option<String>,
    pub available: bool,
    pub adult_content: bool,
}

fn parse_nexus_mod(v: &serde_json::Value) -> Option<NexusModInfo> {
    Some(NexusModInfo {
        mod_id: v.get("mod_id").and_then(|x| x.as_i64())?,
        name: v
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        summary: v
            .get("summary")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        description: v
            .get("description")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string()),
        author: v
            .get("author")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        category_id: v.get("category_id").and_then(|x| x.as_i64()).unwrap_or(0),
        version: v
            .get("version")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        endorsement_count: v
            .get("endorsement_count")
            .and_then(|x| x.as_i64())
            .unwrap_or(0),
        unique_downloads: v
            .get("mod_unique_downloads")
            .and_then(|x| x.as_i64())
            .or_else(|| v.get("unique_downloads").and_then(|x| x.as_i64()))
            .unwrap_or(0),
        picture_url: v
            .get("picture_url")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string()),
        updated_at: v
            .get("updated_timestamp")
            .and_then(|x| x.as_i64())
            .map(|ts| {
                chrono::DateTime::from_timestamp(ts, 0)
                    .map(|dt| dt.format("%Y-%m-%d").to_string())
                    .unwrap_or_default()
            })
            .or_else(|| {
                v.get("updated_time")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string())
            }),
        created_at: v
            .get("created_timestamp")
            .and_then(|x| x.as_i64())
            .map(|ts| {
                chrono::DateTime::from_timestamp(ts, 0)
                    .map(|dt| dt.format("%Y-%m-%d").to_string())
                    .unwrap_or_default()
            })
            .or_else(|| {
                v.get("created_time")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string())
            }),
        available: v.get("available").and_then(|x| x.as_bool()).unwrap_or(true),
        adult_content: v
            .get("contains_adult_content")
            .and_then(|x| x.as_bool())
            .unwrap_or(false),
    })
}

// ---------------------------------------------------------------------------
// Rate limit tracking
// ---------------------------------------------------------------------------

/// Tracks NexusMods API rate limit state from response headers.
struct RateLimitState {
    hourly_remaining: AtomicI64,
    daily_remaining: AtomicI64,
    last_request: Mutex<Option<Instant>>,
}

impl RateLimitState {
    fn new() -> Self {
        Self {
            hourly_remaining: AtomicI64::new(-1), // -1 = unknown
            daily_remaining: AtomicI64::new(-1),
            last_request: Mutex::new(None),
        }
    }

    /// Record the time of a request and enforce minimum spacing (1 second).
    async fn throttle(&self) {
        let wait = {
            let last = self.last_request.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(prev) = *last {
                let elapsed = prev.elapsed();
                let min_interval = std::time::Duration::from_secs(1);
                if elapsed < min_interval {
                    Some(min_interval - elapsed)
                } else {
                    None
                }
            } else {
                None
            }
        };

        if let Some(duration) = wait {
            tokio::time::sleep(duration).await;
        }

        // Update last request time after any wait
        let mut last_guard = self.last_request.lock().unwrap_or_else(|e| e.into_inner());
        *last_guard = Some(Instant::now());
    }

    /// Parse rate limit headers from a response and update internal state.
    fn update_from_response(&self, response: &reqwest::Response) {
        if let Some(val) = response.headers().get("x-rl-hourly-remaining") {
            if let Ok(s) = val.to_str() {
                if let Ok(n) = s.parse::<i64>() {
                    self.hourly_remaining.store(n, Ordering::Relaxed);
                }
            }
        }
        if let Some(val) = response.headers().get("x-rl-daily-remaining") {
            if let Ok(s) = val.to_str() {
                if let Ok(n) = s.parse::<i64>() {
                    self.daily_remaining.store(n, Ordering::Relaxed);
                }
            }
        }
    }

    /// Get current rate limit info for logging/diagnostics.
    fn _hourly_remaining(&self) -> i64 {
        self.hourly_remaining.load(Ordering::Relaxed)
    }

    fn _daily_remaining(&self) -> i64 {
        self.daily_remaining.load(Ordering::Relaxed)
    }
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// Async client for the Nexus Mods v1 REST API.
pub struct NexusClient {
    client: reqwest::Client,
    rate_limit: RateLimitState,
    /// When created from OAuth, premium status is known from the JWT claims
    /// and we don't need to hit the v1 REST API (which doesn't support Bearer
    /// tokens reliably for /users/validate.json).
    oauth_premium: Option<bool>,
}

impl NexusClient {
    /// Create a new client using the supplied personal API key.
    ///
    /// Sets all NexusMods-required headers:
    /// - `apikey` - personal API key for authentication
    /// - `User-Agent` - identifies the application
    /// - `Application-Name` - required by NexusMods API TOS
    /// - `Application-Version` - required by NexusMods API TOS
    /// - `Protocol-Version` - NexusMods API protocol version
    pub fn new(api_key: String) -> Self {
        let mut headers = HeaderMap::new();
        // HeaderValue::from_str accepts visible ASCII (0x20..=0x7E) + tab.
        // If the key somehow contains invalid chars, fall back to from_bytes
        // which is more permissive.
        let header_val = HeaderValue::from_str(&api_key)
            .or_else(|_| HeaderValue::from_bytes(api_key.as_bytes()))
            .expect("API key contains non-ASCII bytes");
        headers.insert("apikey", header_val);
        Self::build_with_headers(headers)
    }

    /// Create a new client using an OAuth Bearer access token.
    pub fn with_bearer(access_token: &str) -> Self {
        let mut headers = HeaderMap::new();
        let bearer = format!("Bearer {}", access_token);
        if let Ok(val) = HeaderValue::from_str(&bearer) {
            headers.insert(AUTHORIZATION, val);
        }
        Self::build_with_headers(headers)
    }

    /// Create a NexusClient from the current authentication method.
    ///
    /// Returns `Err` if no auth is configured.
    pub fn from_auth_method(method: &crate::oauth::AuthMethod) -> Result<Self> {
        match method {
            crate::oauth::AuthMethod::ApiKey(key) => Ok(Self::new(key.clone())),
            crate::oauth::AuthMethod::OAuth(tokens) => {
                let mut client = Self::with_bearer(&tokens.access_token);
                // Extract premium status from JWT claims so we don't have to
                // hit /users/validate.json (which doesn't support Bearer tokens).
                client.oauth_premium = Some(
                    crate::oauth::parse_user_info(&tokens.access_token)
                        .map(|u| u.is_premium)
                        .unwrap_or(false),
                );
                Ok(client)
            }
            crate::oauth::AuthMethod::None => Err(NexusError::Auth(
                "No NexusMods authentication configured. Sign in via Settings.".to_string(),
            )),
        }
    }

    fn build_with_headers(mut headers: HeaderMap) -> Self {
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        let ua = format!("Corkscrew/{}", env!("CARGO_PKG_VERSION"));
        headers.insert(USER_AGENT, HeaderValue::from_str(&ua).unwrap());

        // NexusMods API compliance headers
        headers.insert("Application-Name", HeaderValue::from_static("Corkscrew"));
        let app_version = HeaderValue::from_str(env!("CARGO_PKG_VERSION"))
            .unwrap_or_else(|_| HeaderValue::from_static("0.0.0"));
        headers.insert("Application-Version", app_version);
        headers.insert("Protocol-Version", HeaderValue::from_static("0.15.5"));

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("failed to build reqwest client");

        Self {
            client,
            rate_limit: RateLimitState::new(),
            oauth_premium: None,
        }
    }

    // -- helpers -----------------------------------------------------------

    /// Send a GET request, returning a `serde_json::Value` on success or a
    /// `NexusError::Api` on a non-2xx status.
    ///
    /// Includes client-side rate limit throttling (minimum 1s between requests),
    /// response header tracking, and automatic HTTP 429 retry with backoff.
    async fn get_json(&self, url: &str) -> Result<serde_json::Value> {
        // Enforce minimum spacing between requests
        self.rate_limit.throttle().await;

        let response = self.client.get(url).send().await?;
        self.rate_limit.update_from_response(&response);
        let status = response.status();

        // Handle HTTP 429 (Too Many Requests) with retry
        if status.as_u16() == 429 {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(5);

            log::warn!(
                "NexusMods API rate limited (429). Retrying after {}s. URL: {}",
                retry_after,
                url
            );

            tokio::time::sleep(std::time::Duration::from_secs(retry_after)).await;

            // Retry once
            self.rate_limit.throttle().await;
            let retry_response = self.client.get(url).send().await?;
            self.rate_limit.update_from_response(&retry_response);
            let retry_status = retry_response.status();

            if retry_status.as_u16() == 429 {
                return Err(NexusError::RateLimited { retry_after });
            }

            if !retry_status.is_success() {
                let message = retry_response
                    .text()
                    .await
                    .unwrap_or_else(|_| "no response body".into());
                return Err(NexusError::Api {
                    status: retry_status.as_u16(),
                    message,
                });
            }

            return Ok(retry_response.json().await?);
        }

        if !status.is_success() {
            let message = response
                .text()
                .await
                .unwrap_or_else(|_| "no response body".into());
            return Err(NexusError::Api {
                status: status.as_u16(),
                message,
            });
        }

        Ok(response.json().await?)
    }

    // -- public API --------------------------------------------------------

    /// Validate the API key and return the user information object.
    pub async fn validate_key(&self) -> Result<serde_json::Value> {
        let url = format!("{NEXUS_API_BASE}/users/validate.json");
        self.get_json(&url).await
    }

    /// Fetch metadata for a single mod.
    pub async fn get_mod(&self, game_slug: &str, mod_id: i64) -> Result<serde_json::Value> {
        let url = format!("{NEXUS_API_BASE}/games/{game_slug}/mods/{mod_id}.json");
        self.get_json(&url).await
    }

    /// List all available files for a mod.
    pub async fn get_mod_files(
        &self,
        game_slug: &str,
        mod_id: i64,
    ) -> Result<Vec<serde_json::Value>> {
        let url = format!("{NEXUS_API_BASE}/games/{game_slug}/mods/{mod_id}/files.json");
        let json = self.get_json(&url).await?;

        // The API wraps the file list inside a `files` key.
        let files = json
            .get("files")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(files)
    }

    /// Check if the authenticated user has premium/supporter status.
    ///
    /// When the client was created from OAuth (via `from_auth_method`), the
    /// premium status is read directly from the JWT claims — no API call needed.
    /// For API-key clients, falls back to the v1 `/users/validate.json` endpoint.
    pub async fn is_premium(&self) -> bool {
        // Fast path: OAuth premium status was cached at construction time.
        if let Some(premium) = self.oauth_premium {
            return premium;
        }

        // API-key path: hit the v1 REST endpoint.
        match self.validate_key().await {
            Ok(info) => {
                let premium = info
                    .get("is_premium")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let supporter = info
                    .get("is_supporter")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                premium || supporter
            }
            Err(_) => false,
        }
    }

    /// Retrieve CDN download links for a specific file.
    ///
    /// **NexusMods policy:** Free users CANNOT automate downloads. They must
    /// click "Slow Download" on the website, which provides an NXM link with
    /// `key` and `expires` parameters. If key/expires are missing, this
    /// function verifies the user has premium status before proceeding.
    pub async fn get_download_links(
        &self,
        game_slug: &str,
        mod_id: i64,
        file_id: i64,
        key: Option<&str>,
        expires: Option<&str>,
    ) -> Result<Vec<DownloadLink>> {
        let mut url = format!(
            "{NEXUS_API_BASE}/games/{game_slug}/mods/{mod_id}/files/{file_id}/download_link.json"
        );

        // NexusMods compliance: free users MUST provide key/expires from
        // clicking "Slow Download" on the website. Only premium users may
        // request download links without these parameters.
        if (key.is_none() || expires.is_none()) && !self.is_premium().await {
            return Err(NexusError::Api {
                status: 403,
                message: "Free users must download from the NexusMods website. \
                              Please click the download button on the mod page."
                    .to_string(),
            });
        }

        // Attach query parameters when present (URL-encoded for safety).
        if let (Some(k), Some(e)) = (key, expires) {
            let encoded_key: String = k
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '~' {
                        c.to_string()
                    } else {
                        format!("%{:02X}", c as u8)
                    }
                })
                .collect();
            let encoded_expires: String = e
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '~' {
                        c.to_string()
                    } else {
                        format!("%{:02X}", c as u8)
                    }
                })
                .collect();
            url.push_str(&format!("?key={encoded_key}&expires={encoded_expires}"));
        }

        // Enforce minimum spacing between requests
        self.rate_limit.throttle().await;

        let response = self.client.get(&url).send().await?;
        self.rate_limit.update_from_response(&response);
        let status = response.status();

        // Handle HTTP 429 with retry
        if status.as_u16() == 429 {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(5);

            log::warn!(
                "NexusMods API rate limited (429) on download links. Retrying after {}s.",
                retry_after
            );

            tokio::time::sleep(std::time::Duration::from_secs(retry_after)).await;

            self.rate_limit.throttle().await;
            let retry_response = self.client.get(&url).send().await?;
            self.rate_limit.update_from_response(&retry_response);
            let retry_status = retry_response.status();

            if retry_status.as_u16() == 429 {
                return Err(NexusError::RateLimited { retry_after });
            }

            if !retry_status.is_success() {
                let message = retry_response
                    .text()
                    .await
                    .unwrap_or_else(|_| "no response body".into());
                return Err(NexusError::Api {
                    status: retry_status.as_u16(),
                    message,
                });
            }

            let links: Vec<DownloadLink> = retry_response.json().await?;
            if links.is_empty() {
                return Err(NexusError::NoDownloadLinks);
            }
            return Ok(links);
        }

        if !status.is_success() {
            let message = response
                .text()
                .await
                .unwrap_or_else(|_| "no response body".into());
            return Err(NexusError::Api {
                status: status.as_u16(),
                message,
            });
        }

        let links: Vec<DownloadLink> = response.json().await?;
        if links.is_empty() {
            return Err(NexusError::NoDownloadLinks);
        }

        Ok(links)
    }

    /// Download a file from a direct CDN URL to `dest`, optionally reporting
    /// progress via `progress_callback(downloaded_bytes, total_bytes)`.
    ///
    /// Returns the full path to the downloaded file.
    pub async fn download_file<F>(
        &self,
        download_url: &str,
        dest: &Path,
        progress_callback: Option<F>,
    ) -> Result<PathBuf>
    where
        F: Fn(u64, u64),
    {
        let response = self.client.get(download_url).send().await?;
        let status = response.status();

        if !status.is_success() {
            let message = response
                .text()
                .await
                .unwrap_or_else(|_| "no response body".into());
            return Err(NexusError::Api {
                status: status.as_u16(),
                message,
            });
        }

        let total = response.content_length().unwrap_or(0);

        // Derive the file name from the URL path or fall back to a default.
        let file_name = Url::parse(download_url)
            .ok()
            .and_then(|u| {
                u.path_segments()
                    .and_then(|mut seg| seg.next_back().map(|s| s.to_string()))
            })
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "download".to_string());

        let file_path = dest.join(&file_name);

        // Ensure the destination directory exists.
        tokio::fs::create_dir_all(dest).await?;

        let mut file = tokio::fs::File::create(&file_path).await?;
        let mut downloaded: u64 = 0;
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await?;
            downloaded += chunk.len() as u64;

            if let Some(ref cb) = progress_callback {
                cb(downloaded, total);
            }
        }

        tokio::io::AsyncWriteExt::flush(&mut file).await?;

        Ok(file_path)
    }

    /// Check for updates on a list of mods.
    ///
    /// For each mod with a `nexus_mod_id`, fetches the latest files from the
    /// Nexus API and compares with the locally stored `nexus_file_id`. Returns
    /// info about mods that have newer versions available.
    pub async fn check_updates(
        &self,
        game_slug: &str,
        mods: &[ModUpdateQuery],
    ) -> Result<Vec<ModUpdateInfo>> {
        let mut updates = Vec::new();

        for m in mods {
            let files = match self.get_mod_files(game_slug, m.nexus_mod_id).await {
                Ok(f) => f,
                Err(_) => continue, // Skip mods that fail (rate limited, removed, etc.)
            };

            // Find the latest "main" file or the file with the highest file_id
            let latest = files
                .iter()
                .filter_map(|f| {
                    let file_id = f.get("file_id").and_then(|v| v.as_i64())?;
                    let version = f
                        .get("version")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = f
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let category = f.get("category_id").and_then(|v| v.as_i64());
                    Some((file_id, version, name, category))
                })
                // Prefer main files (category 1), then updates (category 4)
                .max_by_key(|(id, _, _, cat)| {
                    let cat_weight = match cat {
                        Some(1) => 1000000, // main files
                        Some(4) => 500000,  // update files
                        _ => 0,
                    };
                    cat_weight + *id
                });

            if let Some((latest_file_id, latest_version, latest_name, _)) = latest {
                // Compare against locally installed file_id
                let has_update = match m.nexus_file_id {
                    Some(local_id) => latest_file_id > local_id,
                    None => false, // No file ID tracked, can't compare
                };

                if has_update {
                    updates.push(ModUpdateInfo {
                        mod_id: m.local_mod_id,
                        nexus_mod_id: m.nexus_mod_id,
                        mod_name: m.mod_name.clone(),
                        current_version: m.current_version.clone(),
                        latest_version,
                        latest_file_name: latest_name,
                        latest_file_id,
                    });
                }
            }
        }

        Ok(updates)
    }

    /// Browse mods on NexusMods by category (trending, latest, updated, all).
    ///
    /// When `category` is `"all"`, fetches trending + latest + updated
    /// concurrently and deduplicates by mod_id, returning the combined set.
    pub async fn browse_mods(&self, game_slug: &str, category: &str) -> Result<Vec<NexusModInfo>> {
        if category == "all" {
            // Fetch all three endpoints concurrently for maximum coverage
            let (trending, latest, updated) = tokio::join!(
                self.browse_mods_single(game_slug, "trending"),
                self.browse_mods_single(game_slug, "latest_added"),
                self.browse_mods_single(game_slug, "updated"),
            );

            let mut seen = std::collections::HashSet::new();
            let mut combined = Vec::new();
            for list in [trending, latest, updated] {
                for m in list.unwrap_or_default() {
                    if seen.insert(m.mod_id) {
                        combined.push(m);
                    }
                }
            }
            Ok(combined)
        } else {
            self.browse_mods_single(
                game_slug,
                match category {
                    "latest" => "latest_added",
                    "updated" => "updated",
                    _ => "trending",
                },
            )
            .await
        }
    }

    /// Fetch a single browse endpoint.
    async fn browse_mods_single(
        &self,
        game_slug: &str,
        endpoint: &str,
    ) -> Result<Vec<NexusModInfo>> {
        let url = format!("{NEXUS_API_BASE}/games/{game_slug}/mods/{endpoint}.json");
        let json = self.get_json(&url).await?;

        let mods = json
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|v| parse_nexus_mod(&v))
            .collect();

        Ok(mods)
    }

    /// Get detailed information for a single mod, returned as NexusModInfo.
    pub async fn get_mod_info(&self, game_slug: &str, mod_id: i64) -> Result<NexusModInfo> {
        let json = self.get_mod(game_slug, mod_id).await?;
        parse_nexus_mod(&json).ok_or_else(|| NexusError::Api {
            status: 0,
            message: "Failed to parse mod info".into(),
        })
    }

    /// Convenience wrapper: resolve an NXM link, fetch CDN URLs, then
    /// download the first available mirror.
    pub async fn download_from_nxm<F>(
        &self,
        nxm: &NXMLink,
        download_dir: &Path,
        progress_callback: Option<F>,
    ) -> Result<PathBuf>
    where
        F: Fn(u64, u64),
    {
        let links = self
            .get_download_links(
                &nxm.game_slug,
                nxm.mod_id,
                nxm.file_id,
                nxm.key.as_deref(),
                nxm.expires.as_deref(),
            )
            .await?;

        let link = links.first().ok_or(NexusError::NoDownloadLinks)?;

        self.download_file(&link.uri, download_dir, progress_callback)
            .await
    }

    // -- Endorsements -------------------------------------------------------

    /// POST JSON to a NexusMods API endpoint.
    async fn post_json(
        &self,
        url: &str,
        body: Option<serde_json::Value>,
    ) -> Result<serde_json::Value> {
        self.rate_limit.throttle().await;

        let mut req = self.client.post(url);
        if let Some(b) = body {
            req = req.json(&b);
        }

        let response = req.send().await?;
        self.rate_limit.update_from_response(&response);
        let status = response.status();

        if status.as_u16() == 429 {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(5);
            return Err(NexusError::RateLimited { retry_after });
        }

        if !status.is_success() {
            let message = response
                .text()
                .await
                .unwrap_or_else(|_| "no response body".into());
            return Err(NexusError::Api {
                status: status.as_u16(),
                message,
            });
        }

        Ok(response.json().await?)
    }

    /// Endorse a mod on NexusMods.
    ///
    /// Returns the new endorsement status (e.g., "Endorsed").
    pub async fn endorse_mod(
        &self,
        game_slug: &str,
        mod_id: i64,
        version: Option<&str>,
    ) -> Result<EndorseResponse> {
        let url = format!("{NEXUS_API_BASE}/games/{game_slug}/mods/{mod_id}/endorse.json");
        let body = version.map(|v| serde_json::json!({ "Version": v }));
        let json = self.post_json(&url, body).await?;
        Ok(EndorseResponse {
            status: json
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown")
                .to_string(),
            message: json
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })
    }

    /// Abstain from endorsing a mod on NexusMods.
    pub async fn abstain_mod(&self, game_slug: &str, mod_id: i64) -> Result<EndorseResponse> {
        let url = format!("{NEXUS_API_BASE}/games/{game_slug}/mods/{mod_id}/abstain.json");
        let json = self.post_json(&url, None).await?;
        Ok(EndorseResponse {
            status: json
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown")
                .to_string(),
            message: json
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })
    }

    /// Get the user's endorsement list to determine which mods they've endorsed.
    pub async fn get_user_endorsements(&self) -> Result<Vec<UserEndorsement>> {
        let url = format!("{NEXUS_API_BASE}/user/endorsements.json");
        let json = self.get_json(&url).await?;

        let endorsements = json
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| {
                        Some(UserEndorsement {
                            mod_id: v.get("mod_id").and_then(|x| x.as_i64())?,
                            domain_name: v.get("domain_name").and_then(|x| x.as_str())?.to_string(),
                            status: v
                                .get("status")
                                .and_then(|x| x.as_str())
                                .unwrap_or("Undecided")
                                .to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(endorsements)
    }
}

// ---------------------------------------------------------------------------
// Endorsement types
// ---------------------------------------------------------------------------

/// Response from an endorse/abstain API call.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EndorseResponse {
    pub status: String,
    pub message: String,
}

/// A single endorsement entry from the user's endorsement list.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserEndorsement {
    pub mod_id: i64,
    pub domain_name: String,
    /// "Endorsed", "Abstained", or "Undecided"
    pub status: String,
}

// ---------------------------------------------------------------------------
// Game categories (v1 REST API)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NexusCategory {
    pub category_id: i64,
    pub name: String,
    pub parent_category: Option<i64>,
}

impl NexusClient {
    /// Fetch the full category tree for a game from v1 REST API.
    pub async fn get_game_categories(&self, game_slug: &str) -> Result<Vec<NexusCategory>> {
        let url = format!("{NEXUS_API_BASE}/games/{game_slug}/categories.json");
        let json = self.get_json(&url).await?;
        let categories = json
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|v| {
                Some(NexusCategory {
                    category_id: v.get("category_id").and_then(|x| x.as_i64())?,
                    name: v
                        .get("name")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                    parent_category: v.get("parent_category").and_then(|x| {
                        let id = x.as_i64().unwrap_or(0);
                        if id == 0 {
                            None
                        } else {
                            Some(id)
                        }
                    }),
                })
            })
            .collect();
        Ok(categories)
    }
}

// ---------------------------------------------------------------------------
// NexusMods v2 GraphQL mod search
// ---------------------------------------------------------------------------

const GRAPHQL_ENDPOINT: &str = "https://api.nexusmods.com/v2/graphql";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NexusSearchResult {
    pub mods: Vec<NexusModInfo>,
    pub total_count: u64,
    pub offset: u32,
    pub has_more: bool,
}

/// Build default headers for standalone GraphQL requests (outside NexusClient).
///
/// If `is_bearer` is true, the token is sent as `Authorization: Bearer {token}`.
/// Otherwise it is sent as the legacy `apikey` header.
pub fn nexus_graphql_headers_ext(token: &str, is_bearer: bool) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if is_bearer {
        let bearer = format!("Bearer {}", token);
        if let Ok(val) = HeaderValue::from_str(&bearer) {
            headers.insert(AUTHORIZATION, val);
        }
    } else if let Ok(val) = HeaderValue::from_str(token) {
        headers.insert("apikey", val);
    }
    headers.insert("Content-Type", HeaderValue::from_static("application/json"));
    headers.insert("Application-Name", HeaderValue::from_static("Corkscrew"));
    let app_version = HeaderValue::from_str(env!("CARGO_PKG_VERSION"))
        .unwrap_or_else(|_| HeaderValue::from_static("0.0.0"));
    headers.insert("Application-Version", app_version);
    headers.insert("Protocol-Version", HeaderValue::from_static("0.15.5"));
    headers
}

/// Build default headers for standalone GraphQL requests — legacy API key version.
fn nexus_graphql_headers(api_key: &str) -> HeaderMap {
    nexus_graphql_headers_ext(api_key, false)
}

/// Run an introspection query against the NexusMods v2 GraphQL API.
/// Returns the raw JSON schema string for development/debugging.
pub async fn graphql_introspect(api_key: &str) -> Result<String> {
    let headers = nexus_graphql_headers(api_key);
    let client = reqwest::Client::builder()
        .user_agent(format!("Corkscrew/{}", env!("CARGO_PKG_VERSION")))
        .default_headers(headers)
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(NexusError::Http)?;

    let body = serde_json::json!({
        "query": "{ __schema { queryType { name } types { name kind fields { name type { name kind ofType { name kind } } args { name type { name kind } } } } } }"
    });

    let response = client
        .post(GRAPHQL_ENDPOINT)
        .json(&body)
        .send()
        .await
        .map_err(NexusError::Http)?;

    let text = response.text().await.map_err(NexusError::Http)?;
    Ok(text)
}

/// Search mods via NexusMods v2 GraphQL API.
/// Falls back gracefully if the query doesn't exist.
#[allow(clippy::too_many_arguments)]
/// Search mods via GraphQL with explicit auth type.
#[allow(clippy::too_many_arguments)]
pub async fn graphql_search_mods_ext(
    token: &str,
    is_bearer: bool,
    game_domain: &str,
    search_text: Option<&str>,
    sort_by: Option<&str>,
    sort_dir: Option<&str>,
    count: u32,
    offset: u32,
    include_adult: bool,
    category_id: Option<i64>,
    author: Option<&str>,
    updated_since: Option<&str>,
    min_downloads: Option<i64>,
    min_endorsements: Option<i64>,
) -> Result<NexusSearchResult> {
    let headers = nexus_graphql_headers_ext(token, is_bearer);
    graphql_search_mods_with_headers(headers, game_domain, search_text, sort_by, sort_dir, count, offset, include_adult, category_id, author, updated_since, min_downloads, min_endorsements).await
}

#[allow(clippy::too_many_arguments)]
pub async fn graphql_search_mods(
    api_key: &str,
    game_domain: &str,
    search_text: Option<&str>,
    sort_by: Option<&str>,
    sort_dir: Option<&str>,
    count: u32,
    offset: u32,
    include_adult: bool,
    category_id: Option<i64>,
    author: Option<&str>,
    updated_since: Option<&str>,
    min_downloads: Option<i64>,
    min_endorsements: Option<i64>,
) -> Result<NexusSearchResult> {
    let headers = nexus_graphql_headers(api_key);
    graphql_search_mods_with_headers(headers, game_domain, search_text, sort_by, sort_dir, count, offset, include_adult, category_id, author, updated_since, min_downloads, min_endorsements).await
}

#[allow(clippy::too_many_arguments)]
async fn graphql_search_mods_with_headers(
    headers: HeaderMap,
    game_domain: &str,
    search_text: Option<&str>,
    sort_by: Option<&str>,
    sort_dir: Option<&str>,
    count: u32,
    offset: u32,
    include_adult: bool,
    category_id: Option<i64>,
    author: Option<&str>,
    updated_since: Option<&str>,
    min_downloads: Option<i64>,
    min_endorsements: Option<i64>,
) -> Result<NexusSearchResult> {
    let client = reqwest::Client::builder()
        .user_agent(format!("Corkscrew/{}", env!("CARGO_PKG_VERSION")))
        .default_headers(headers)
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(NexusError::Http)?;

    // Build GraphQL variables using NexusMods v2 filter/sort schema.
    // Filters use array-of-objects format: [{ "value": ..., "op": "EQUALS" }]
    // Sorts use named-field format: [{ "endorsements": { "direction": "DESC" } }]
    let mut filter = serde_json::Map::new();
    filter.insert(
        "gameDomainName".into(),
        serde_json::json!([{ "value": game_domain, "op": "EQUALS" }]),
    );
    if let Some(text) = search_text {
        if !text.is_empty() {
            filter.insert(
                "name".into(),
                serde_json::json!([{ "value": text, "op": "WILDCARD" }]),
            );
        }
    }
    if !include_adult {
        filter.insert(
            "adultContent".into(),
            serde_json::json!([{ "value": false, "op": "EQUALS" }]),
        );
    }
    if let Some(cat_id) = category_id {
        filter.insert(
            "categoryId".into(),
            serde_json::json!([{ "value": cat_id, "op": "EQUALS" }]),
        );
    }
    if let Some(auth) = author {
        if !auth.is_empty() {
            filter.insert(
                "author".into(),
                serde_json::json!([{ "value": format!("*{}*", auth), "op": "WILDCARD" }]),
            );
        }
    }
    if let Some(since) = updated_since {
        if !since.is_empty() {
            filter.insert(
                "updatedAt".into(),
                serde_json::json!([{ "value": since, "op": "GT" }]),
            );
        }
    }
    if let Some(min_dl) = min_downloads {
        filter.insert(
            "downloads".into(),
            serde_json::json!([{ "value": min_dl, "op": "GT" }]),
        );
    }
    if let Some(min_end) = min_endorsements {
        filter.insert(
            "endorsements".into(),
            serde_json::json!([{ "value": min_end, "op": "GT" }]),
        );
    }
    filter.insert("op".into(), serde_json::json!("AND"));

    let sort_field = sort_by.unwrap_or("endorsements");
    let sort_direction = sort_dir.unwrap_or("DESC");

    let query = r#"
        query SearchMods($filter: ModsFilter, $sort: [ModsSort!], $count: Int, $offset: Int) {
            mods(filter: $filter, sort: $sort, count: $count, offset: $offset) {
                nodes {
                    modId
                    name
                    summary
                    author
                    modCategory { id }
                    version
                    endorsements
                    downloads
                    pictureUrl
                    updatedAt
                    createdAt
                    adultContent
                    status
                }
                totalCount
            }
        }
    "#;

    // Build sort object: { "<field_name>": { "direction": "DESC" } }
    let mut sort_obj = serde_json::Map::new();
    sort_obj.insert(
        sort_field.to_string(),
        serde_json::json!({ "direction": sort_direction }),
    );

    let variables = serde_json::json!({
        "filter": filter,
        "sort": [sort_obj],
        "count": count,
        "offset": offset,
    });

    let body = serde_json::json!({
        "query": query,
        "variables": variables,
    });

    let response = client
        .post(GRAPHQL_ENDPOINT)
        .json(&body)
        .send()
        .await
        .map_err(NexusError::Http)?;

    let status = response.status();
    let json: serde_json::Value = response.json().await.map_err(NexusError::Http)?;

    if !status.is_success() {
        return Err(NexusError::Api {
            status: status.as_u16(),
            message: format!("GraphQL error: {}", json),
        });
    }

    // Check for GraphQL errors
    if let Some(errors) = json.get("errors") {
        if let Some(arr) = errors.as_array() {
            if !arr.is_empty() {
                let messages: Vec<String> = arr
                    .iter()
                    .filter_map(|e| e.get("message").and_then(|m| m.as_str()))
                    .map(String::from)
                    .collect();
                return Err(NexusError::Api {
                    status: 0,
                    message: format!("GraphQL errors: {}", messages.join("; ")),
                });
            }
        }
    }

    // Parse response data
    let data = json.get("data").ok_or(NexusError::Api {
        status: 0,
        message: "No 'data' field in GraphQL response".into(),
    })?;

    let mods_data = data.get("mods").ok_or(NexusError::Api {
        status: 0,
        message: "No 'mods' field in GraphQL response — mod search may not be available".into(),
    })?;

    let nodes = mods_data
        .get("nodes")
        .and_then(|n| n.as_array())
        .cloned()
        .unwrap_or_default();

    let total_count = mods_data
        .get("totalCount")
        .and_then(|n| n.as_u64())
        .unwrap_or(0);

    let mods: Vec<NexusModInfo> = nodes
        .into_iter()
        .filter_map(|v| {
            Some(NexusModInfo {
                mod_id: v.get("modId").and_then(|x| x.as_i64())?,
                name: v
                    .get("name")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                summary: v
                    .get("summary")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                description: None, // GraphQL search doesn't return full description
                author: v
                    .get("author")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                category_id: v
                    .get("modCategory")
                    .and_then(|c| c.get("id"))
                    .and_then(|x| x.as_i64())
                    .unwrap_or(0),
                version: v
                    .get("version")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                endorsement_count: v.get("endorsements").and_then(|x| x.as_i64()).unwrap_or(0),
                unique_downloads: v.get("downloads").and_then(|x| x.as_i64()).unwrap_or(0),
                picture_url: v
                    .get("pictureUrl")
                    .and_then(|x| x.as_str())
                    .map(String::from),
                updated_at: v
                    .get("updatedAt")
                    .and_then(|x| x.as_str())
                    .map(String::from),
                created_at: v
                    .get("createdAt")
                    .and_then(|x| x.as_str())
                    .map(String::from),
                available: v
                    .get("status")
                    .and_then(|x| x.as_str())
                    .map(|s| s == "published")
                    .unwrap_or(true),
                adult_content: v
                    .get("adultContent")
                    .and_then(|x| x.as_bool())
                    .unwrap_or(false),
            })
        })
        .collect();

    let has_more = (offset as u64 + mods.len() as u64) < total_count;

    Ok(NexusSearchResult {
        mods,
        total_count,
        offset,
        has_more,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_nxm_link_full() {
        let url = "nxm://skyrimspecialedition/mods/12345/files/67890?key=abc123&expires=1700000000";
        let link = NXMLink::parse(url).unwrap();

        assert_eq!(link.game_slug, "skyrimspecialedition");
        assert_eq!(link.mod_id, 12345);
        assert_eq!(link.file_id, 67890);
        assert_eq!(link.key.as_deref(), Some("abc123"));
        assert_eq!(link.expires.as_deref(), Some("1700000000"));
    }

    #[test]
    fn parse_nxm_link_without_query() {
        let url = "nxm://fallout4/mods/100/files/200";
        let link = NXMLink::parse(url).unwrap();

        assert_eq!(link.game_slug, "fallout4");
        assert_eq!(link.mod_id, 100);
        assert_eq!(link.file_id, 200);
        assert!(link.key.is_none());
        assert!(link.expires.is_none());
    }

    #[test]
    fn parse_nxm_link_bad_scheme() {
        let result = NXMLink::parse("https://nexusmods.com/whatever");
        assert!(result.is_err());
    }

    #[test]
    fn parse_nxm_link_bad_path() {
        let result = NXMLink::parse("nxm://skyrim/wrong/12345/path/67890");
        assert!(result.is_err());
    }
}
