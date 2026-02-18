use std::path::{Path, PathBuf};

use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
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
        let normalised = if url.starts_with("nxm://") {
            format!("http://{}", &url[6..])
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

        let mod_id: i64 = segments[1].parse().map_err(|_| {
            NexusError::InvalidNxmLink(format!("invalid mod_id: {}", segments[1]))
        })?;

        let file_id: i64 = segments[3].parse().map_err(|_| {
            NexusError::InvalidNxmLink(format!("invalid file_id: {}", segments[3]))
        })?;

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

/// A single CDN download link returned by the Nexus Mods API.
#[derive(Clone, Debug, Deserialize)]
pub struct DownloadLink {
    #[serde(rename = "URI")]
    pub uri: String,
    pub name: String,
    pub short_name: String,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// Async client for the Nexus Mods v1 REST API.
pub struct NexusClient {
    client: reqwest::Client,
}

impl NexusClient {
    /// Create a new client using the supplied personal API key.
    pub fn new(api_key: String) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(
            "apikey",
            HeaderValue::from_str(&api_key).expect("invalid API key characters"),
        );
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static("Corkscrew/0.1.0"),
        );

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .expect("failed to build reqwest client");

        Self { client }
    }

    // -- helpers -----------------------------------------------------------

    /// Send a GET request, returning a `serde_json::Value` on success or a
    /// `NexusError::Api` on a non-2xx status.
    async fn get_json(&self, url: &str) -> Result<serde_json::Value> {
        let response = self.client.get(url).send().await?;
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

        Ok(response.json().await?)
    }

    // -- public API --------------------------------------------------------

    /// Validate the API key and return the user information object.
    pub async fn validate_key(&self) -> Result<serde_json::Value> {
        let url = format!("{NEXUS_API_BASE}/users/validate.json");
        self.get_json(&url).await
    }

    /// Fetch metadata for a single mod.
    pub async fn get_mod(
        &self,
        game_slug: &str,
        mod_id: i64,
    ) -> Result<serde_json::Value> {
        let url = format!("{NEXUS_API_BASE}/games/{game_slug}/mods/{mod_id}.json");
        self.get_json(&url).await
    }

    /// List all available files for a mod.
    pub async fn get_mod_files(
        &self,
        game_slug: &str,
        mod_id: i64,
    ) -> Result<Vec<serde_json::Value>> {
        let url =
            format!("{NEXUS_API_BASE}/games/{game_slug}/mods/{mod_id}/files.json");
        let json = self.get_json(&url).await?;

        // The API wraps the file list inside a `files` key.
        let files = json
            .get("files")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(files)
    }

    /// Retrieve CDN download links for a specific file.
    ///
    /// For premium users the `key` and `expires` parameters may be `None`.
    /// Free-tier users must supply the values received through the NXM link.
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

        // Attach query parameters when present.
        if let (Some(k), Some(e)) = (key, expires) {
            url.push_str(&format!("?key={k}&expires={e}"));
        }

        let response = self.client.get(&url).send().await?;
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
                    .and_then(|seg| seg.last().map(|s| s.to_string()))
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
