// ---------------------------------------------------------------------------
// Wabbajack archive download engine
// ---------------------------------------------------------------------------
//
// Phase 1 of the Wabbajack install pipeline. Downloads archives referenced by
// a parsed Wabbajack modlist from multiple sources (NexusMods, HTTP, Google
// Drive, MEGA, MediaFire, ModDB, Wabbajack CDN, game files, manual).

use crate::database::ModDatabase;
use crate::wabbajack_types::*;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::Semaphore;

const NEXUS_API_BASE: &str = "https://api.nexusmods.com/v1";

/// Maximum retry attempts for transient download failures.
const MAX_DOWNLOAD_RETRIES: u32 = 8;
/// Base delay in milliseconds for exponential backoff between retries.
const RETRY_BASE_DELAY_MS: u64 = 2000;
/// Timeout in seconds before a download is considered stalled (no progress).
const STALL_TIMEOUT_SECS: u64 = 180;
/// Maximum backoff delay between retries (seconds).
const MAX_BACKOFF_SECS: u64 = 60;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum WjDownloadError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Hash mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },
    #[error("User action required: {0}")]
    UserActionRequired(String),
    #[error("Download source unsupported: {0}")]
    Unsupported(String),
    #[error("{0}")]
    Other(String),
    #[error("Download cancelled")]
    Cancelled,
    #[error("Download stalled after {secs}s with no progress")]
    Stalled { secs: u64 },
}

// ---------------------------------------------------------------------------
// Progress events
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize)]
#[serde(tag = "type")]
pub enum WjProgressEvent {
    DownloadStarted {
        archive_name: String,
        index: usize,
        total: usize,
    },
    DownloadProgress {
        archive_name: String,
        bytes_downloaded: u64,
        total_bytes: u64,
    },
    DownloadCompleted {
        archive_name: String,
    },
    DownloadFailed {
        archive_name: String,
        error: String,
    },
    DownloadSkipped {
        archive_name: String,
        reason: String,
    },
    UserActionRequired {
        archive_name: String,
        url: String,
        prompt: String,
    },
}

/// Summary of the download phase result — includes both successes and failure details.
pub struct WjDownloadPhaseResult {
    /// Map of archive hash → downloaded file path (successes only).
    pub paths: HashMap<String, PathBuf>,
    /// Number of archives that failed to download.
    pub failed_count: usize,
    /// Number of archives skipped (cached/checkpointed).
    pub skipped_count: usize,
    /// Human-readable failure messages (archive name + error).
    pub failures: Vec<String>,
}

// ---------------------------------------------------------------------------
// WjDownloader
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct WjDownloader {
    http_client: reqwest::Client,
    nexus_api_key: Option<String>,
    /// OAuth Bearer token for NexusMods API (alternative to API key).
    nexus_oauth_token: Option<String>,
    is_premium: bool,
    download_dir: PathBuf,
    cancel_token: Option<Arc<AtomicBool>>,
}

impl WjDownloader {
    /// Create a new downloader. If `nexus_api_key` is supplied the downloader
    /// can make NexusMods API calls; `is_premium` gates automated downloads.
    pub fn new(
        nexus_api_key: Option<String>,
        nexus_oauth_token: Option<String>,
        is_premium: bool,
        download_dir: PathBuf,
    ) -> Self {
        let ua = format!("Corkscrew/{}", env!("CARGO_PKG_VERSION"));
        let mut default_headers = HeaderMap::new();
        default_headers.insert(USER_AGENT, HeaderValue::from_str(&ua).unwrap());

        let http_client = reqwest::Client::builder()
            .default_headers(default_headers)
            .redirect(reqwest::redirect::Policy::limited(10))
            .timeout(std::time::Duration::from_secs(600))
            .connect_timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("failed to build reqwest client");

        Self {
            http_client,
            nexus_api_key,
            nexus_oauth_token,
            is_premium,
            download_dir,
            cancel_token: None,
        }
    }

    /// Set a cancel token so that in-flight streaming downloads can be
    /// interrupted mid-transfer instead of only between archives.
    pub fn set_cancel_token(&mut self, token: Arc<AtomicBool>) {
        self.cancel_token = Some(token);
    }

    // -----------------------------------------------------------------------
    // Source-specific download handlers
    // -----------------------------------------------------------------------

    /// Download from NexusMods (premium only).
    async fn download_nexus(
        &self,
        app: &AppHandle,
        archive_name: &str,
        game: &str,
        mod_id: i64,
        file_id: i64,
    ) -> Result<PathBuf, WjDownloadError> {
        // CRITICAL: free users must NOT get automated downloads.
        if !self.is_premium {
            let domain = wj_game_to_nexus_domain(game);
            let url = format!(
                "https://www.nexusmods.com/{domain}/mods/{mod_id}?tab=files&file_id={file_id}"
            );
            return Err(WjDownloadError::UserActionRequired(format!(
                "NexusMods free account: please download manually from {url}"
            )));
        }

        let domain = wj_game_to_nexus_domain(game);
        let url = format!(
            "{NEXUS_API_BASE}/games/{domain}/mods/{mod_id}/files/{file_id}/download_link.json"
        );

        // Use API key if available, otherwise OAuth Bearer token
        let resp = if let Some(api_key) = self.nexus_api_key.as_deref() {
            self.http_client
                .get(&url)
                .header("apikey", api_key)
                .send()
                .await?
        } else if let Some(token) = self.nexus_oauth_token.as_deref() {
            self.http_client
                .get(&url)
                .header("Authorization", format!("Bearer {}", token))
                .send()
                .await?
        } else {
            return Err(WjDownloadError::Other(
                "NexusMods authentication required for downloads. Sign in via Settings.".into(),
            ));
        };

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(WjDownloadError::Other(format!(
                "Nexus API {status}: {body}"
            )));
        }

        let links: Vec<serde_json::Value> = resp.json().await?;
        let download_url = links
            .first()
            .and_then(|l| l.get("URI").or(l.get("uri")))
            .and_then(|u| u.as_str())
            .ok_or_else(|| WjDownloadError::Other("No download URI in Nexus response".into()))?;

        self.stream_download(app, archive_name, download_url, &HeaderMap::new())
            .await
    }

    /// Download from a direct HTTP/HTTPS URL with optional headers.
    async fn download_http(
        &self,
        app: &AppHandle,
        archive_name: &str,
        url: &str,
        extra_headers: &[String],
    ) -> Result<PathBuf, WjDownloadError> {
        let mut headers = HeaderMap::new();
        for h in extra_headers {
            if let Some((k, v)) = h.split_once(':') {
                if let (Ok(name), Ok(val)) = (
                    reqwest::header::HeaderName::from_bytes(k.trim().as_bytes()),
                    HeaderValue::from_str(v.trim()),
                ) {
                    headers.insert(name, val);
                }
            }
        }
        self.stream_download(app, archive_name, url, &headers).await
    }

    /// Download from Google Drive.
    ///
    /// Google Drive serves large files (>100 MB) behind an HTML virus-scan
    /// confirmation page instead of the binary payload. We detect this by
    /// checking the `Content-Type` of the initial response. If it is HTML we
    /// parse the page with `scraper` to extract the real download URL (the
    /// confirmation form action or a direct link), then follow that URL.
    async fn download_google_drive(
        &self,
        app: &AppHandle,
        archive_name: &str,
        id: &str,
    ) -> Result<PathBuf, WjDownloadError> {
        let initial_url = format!(
            "https://drive.usercontent.google.com/download?id={id}&export=download&confirm=t"
        );

        // First request -- may return the file directly or an HTML warning.
        let resp = self.http_client.get(&initial_url).send().await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(WjDownloadError::Other(format!(
                "Google Drive HTTP {status}: {body}"
            )));
        }

        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();

        if !content_type.contains("text/html") {
            // Got the actual file directly -- stream it to disk.
            return self
                .stream_download_from_response(app, archive_name, resp)
                .await;
        }

        // HTML confirmation page -- parse out the real download URL.
        let html_body = resp.text().await?;

        // Scope the `scraper::Html` (non-Send) so it is dropped before any
        // `.await` point.
        let confirmed_url = {
            let document = scraper::Html::parse_document(&html_body);

            // Strategy 1: Look for a form with id="download-form" and use its
            // action URL.
            let form_url = scraper::Selector::parse("form#download-form")
                .ok()
                .and_then(|sel| {
                    document
                        .select(&sel)
                        .next()
                        .and_then(|form| form.value().attr("action").map(|s| s.to_owned()))
                });

            if let Some(url) = form_url {
                url
            } else {
                // Strategy 2: Look for any anchor whose href contains
                // "download" and "confirm".
                let link_url = scraper::Selector::parse("a[href]").ok().and_then(|sel| {
                    document.select(&sel).find_map(|el| {
                        let href = el.value().attr("href")?;
                        if href.contains("download") && href.contains("confirm") {
                            Some(href.to_owned())
                        } else {
                            None
                        }
                    })
                });

                if let Some(url) = link_url {
                    // Relative URLs need the host prepended.
                    if url.starts_with('/') {
                        format!("https://drive.usercontent.google.com{url}")
                    } else {
                        url
                    }
                } else {
                    // Strategy 3: Fall back to appending confirm=t with a
                    // cache-busting parameter to the original URL (handles
                    // older page layouts).
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis();
                    format!(
                        "https://drive.usercontent.google.com/download?id={id}&export=download&confirm=t&_cb={ts}"
                    )
                }
            }
        };

        self.stream_download(app, archive_name, &confirmed_url, &HeaderMap::new())
            .await
    }

    /// Download from MEGA using the `mega` crate.
    ///
    /// Streams directly to disk via `futures::io::AllowStdIo` to avoid loading
    /// the entire file into memory (which would OOM on large archives).
    async fn download_mega(
        &self,
        app: &AppHandle,
        archive_name: &str,
        url: &str,
    ) -> Result<PathBuf, WjDownloadError> {
        let http = reqwest::Client::new();
        let mega_client = mega::Client::builder()
            .build(http)
            .map_err(|e| WjDownloadError::Other(format!("Failed to create MEGA client: {e}")))?;

        let nodes = mega_client
            .fetch_public_nodes(url)
            .await
            .map_err(|e| WjDownloadError::Other(format!("MEGA fetch nodes failed: {e}")))?;

        // Find the first file node.
        let file_node = nodes
            .iter()
            .find(|n| n.kind().is_file())
            .ok_or_else(|| WjDownloadError::Other("No file node found in MEGA link".into()))?;

        let dest = self.download_dir.join(sanitize_filename(archive_name));
        let partial = dest.with_extension("partial");

        // Stream directly to disk instead of buffering in memory. The mega
        // crate uses `futures::io::AsyncWrite`, so we wrap a std BufWriter
        // with `AllowStdIo` to bridge sync I/O into the async trait.
        let file = std::fs::File::create(&partial)
            .map_err(|e| WjDownloadError::Other(format!("Failed to create MEGA temp file: {e}")))?;
        let buf_writer = std::io::BufWriter::new(file);
        let async_writer = futures::io::AllowStdIo::new(buf_writer);

        let _ = app.emit(
            "wabbajack-install-progress",
            WjProgressEvent::DownloadProgress {
                archive_name: archive_name.to_string(),
                bytes_downloaded: 0,
                total_bytes: file_node.size(),
            },
        );

        if let Err(e) = mega_client.download_node(file_node, async_writer).await {
            // Clean up partial file on download failure.
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(WjDownloadError::Other(format!("MEGA download failed: {e}")));
        }

        // Rename .partial -> final name, cleaning up on failure.
        if let Err(e) = tokio::fs::rename(&partial, &dest).await {
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(WjDownloadError::Io(e));
        }
        Ok(dest)
    }

    /// Download from MediaFire (scrape the actual download link from the page).
    async fn download_mediafire(
        &self,
        app: &AppHandle,
        archive_name: &str,
        url: &str,
    ) -> Result<PathBuf, WjDownloadError> {
        let page_html = self.http_client.get(url).send().await?.text().await?;

        // Extract the real download URL from the MediaFire page.
        // Scope the HTML parsing so the non-Send `scraper::Html` is dropped
        // before we hit the next `.await`.
        let real_url = {
            let document = scraper::Html::parse_document(&page_html);
            let selector = scraper::Selector::parse("a#downloadButton, a.input.popsok")
                .map_err(|_| WjDownloadError::Other("Failed to parse CSS selector".into()))?;

            document
                .select(&selector)
                .next()
                .and_then(|el| el.value().attr("href").map(|s| s.to_owned()))
                .ok_or_else(|| {
                    WjDownloadError::Other("Could not find download link on MediaFire page".into())
                })?
        };

        self.stream_download(app, archive_name, &real_url, &HeaderMap::new())
            .await
    }

    /// Download from Wabbajack CDN (direct HTTP), using concurrent parts when
    /// the server supports Range requests and the file is large enough.
    async fn download_wabbajack_cdn(
        &self,
        app: &AppHandle,
        archive_name: &str,
        url: &str,
    ) -> Result<PathBuf, WjDownloadError> {
        let url = remap_cdn_url(url);

        // Probe the server with a HEAD request to check size + Range support.
        let head_resp = self.http_client.head(&url).send().await?;
        let supports_range = head_resp
            .headers()
            .get("accept-ranges")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("bytes"));
        let total_size = head_resp
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);

        const MIN_CONCURRENT_SIZE: u64 = 2 * 1024 * 1024; // 2MB threshold

        if supports_range && total_size >= MIN_CONCURRENT_SIZE {
            let dest = self.download_dir.join(sanitize_filename(archive_name));
            let cancel = self
                .cancel_token
                .clone()
                .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));

            download_cdn_concurrent(&self.http_client, app, archive_name, &url, &dest, total_size, &cancel).await?;
            Ok(dest)
        } else {
            // Fall back to normal streaming download.
            self.stream_download(app, archive_name, &url, &HeaderMap::new())
                .await
        }
    }

    /// Download from ModDB (direct HTTP with redirect following).
    async fn download_moddb(
        &self,
        app: &AppHandle,
        archive_name: &str,
        url: &str,
    ) -> Result<PathBuf, WjDownloadError> {
        self.stream_download(app, archive_name, url, &HeaderMap::new())
            .await
    }

    /// Copy a file from the game directory.
    async fn download_game_file_source(
        &self,
        _app: &AppHandle,
        archive_name: &str,
        game: &str,
        game_file: &str,
    ) -> Result<PathBuf, WjDownloadError> {
        // Try to locate the game directory by scanning all detected games.
        let domain = wj_game_to_nexus_domain(game);
        let all_games = crate::games::detect_all_games();
        let detected = all_games
            .iter()
            .find(|g| g.nexus_slug == domain || g.game_id.eq_ignore_ascii_case(game))
            .ok_or_else(|| {
                WjDownloadError::Other(format!("Cannot detect install path for game '{game}'"))
            })?;

        let source = detected.game_path.join(game_file.replace('\\', "/"));
        if !source.exists() {
            return Err(WjDownloadError::Other(format!(
                "Game file not found: {}",
                source.display()
            )));
        }

        let dest = self.download_dir.join(sanitize_filename(archive_name));
        tokio::fs::copy(&source, &dest).await?;
        Ok(dest)
    }

    /// Manual download -- user must fetch the file themselves.
    async fn download_manual(
        &self,
        _app: &AppHandle,
        _archive_name: &str,
        url: &str,
        prompt: &str,
    ) -> Result<PathBuf, WjDownloadError> {
        let msg = if prompt.is_empty() {
            format!("Please download this file manually: {url}")
        } else {
            format!("{prompt}\n{url}")
        };
        Err(WjDownloadError::UserActionRequired(msg))
    }

    // -----------------------------------------------------------------------
    // Dispatcher
    // -----------------------------------------------------------------------

    /// Download a single archive, dispatching to the correct handler based on
    /// the archive's state variant.
    pub async fn download_archive(
        &self,
        app: &AppHandle,
        archive: &WjTypedArchive,
        install_id: i64,
        db: &Arc<ModDatabase>,
    ) -> Result<PathBuf, WjDownloadError> {
        let name = &archive.name;

        // Retry loop with exponential backoff for transient failures.
        let mut last_err = None;
        let result = 'retry: {
            for attempt in 0..MAX_DOWNLOAD_RETRIES {
                if attempt > 0 {
                    // Check cancel before retrying.
                    if self
                        .cancel_token
                        .as_ref()
                        .is_some_and(|t| t.load(Ordering::Relaxed))
                    {
                        break 'retry Err(last_err.unwrap_or(WjDownloadError::Cancelled));
                    }
                    let delay = std::cmp::min(
                        RETRY_BASE_DELAY_MS * (1u64 << (attempt - 1)),
                        MAX_BACKOFF_SECS * 1000,
                    );
                    log::warn!(
                        "Retry {}/{} for '{}' after {}ms",
                        attempt,
                        MAX_DOWNLOAD_RETRIES,
                        name,
                        delay,
                    );
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                }

                let try_result = match &archive.state {
                    WjArchiveState::Nexus {
                        game,
                        mod_id,
                        file_id,
                    } => {
                        self.download_nexus(app, name, game, *mod_id, *file_id)
                            .await
                    }
                    WjArchiveState::Http { url, headers } => {
                        self.download_http(app, name, url, headers).await
                    }
                    WjArchiveState::GoogleDrive { id } => {
                        self.download_google_drive(app, name, id).await
                    }
                    WjArchiveState::Mega { url } => self.download_mega(app, name, url).await,
                    WjArchiveState::MediaFire { url } => {
                        self.download_mediafire(app, name, url).await
                    }
                    WjArchiveState::WabbajackCDN { url } => {
                        self.download_wabbajack_cdn(app, name, url).await
                    }
                    WjArchiveState::ModDB { url } => self.download_moddb(app, name, url).await,
                    WjArchiveState::GameFileSource {
                        game, game_file, ..
                    } => {
                        self.download_game_file_source(app, name, game, game_file)
                            .await
                    }
                    WjArchiveState::Manual { url, prompt } => {
                        self.download_manual(app, name, url, prompt).await
                    }
                    WjArchiveState::LoversLab { url, .. } => {
                        Err(WjDownloadError::UserActionRequired(format!(
                            "LoversLab downloads require manual action: {url}"
                        )))
                    }
                    WjArchiveState::VectorPlexus { url, .. } => {
                        Err(WjDownloadError::UserActionRequired(format!(
                            "VectorPlexus downloads require manual action: {url}"
                        )))
                    }
                    WjArchiveState::TESAlliance { url, .. } => {
                        Err(WjDownloadError::UserActionRequired(format!(
                            "TESAlliance downloads require manual action: {url}"
                        )))
                    }
                    WjArchiveState::Bethesda { .. } => Err(WjDownloadError::Unsupported(
                        "Bethesda.net downloads are not supported".into(),
                    )),
                };

                match try_result {
                    Ok(path) => break 'retry Ok(path),
                    Err(ref e) if is_transient_error(e) && attempt + 1 < MAX_DOWNLOAD_RETRIES => {
                        log::warn!("Transient download error for '{}': {}", name, e);
                        last_err = Some(try_result.unwrap_err());
                        continue;
                    }
                    Err(_) => break 'retry try_result,
                }
            }
            Err(last_err.unwrap_or(WjDownloadError::Other("max retries exhausted".into())))
        };

        // Update archive status in DB.
        match &result {
            Ok(path) => {
                let _ = db.upsert_wj_archive_status(
                    install_id,
                    &archive.hash.0,
                    name,
                    archive.state.source_type_name(),
                    "downloaded",
                    Some(&path.to_string_lossy()),
                    None,
                );
            }
            Err(e) => {
                let status = match e {
                    WjDownloadError::UserActionRequired(_) => "user_action",
                    WjDownloadError::Unsupported(_) => "unsupported",
                    WjDownloadError::Cancelled => "cancelled",
                    _ => "failed",
                };
                let _ = db.upsert_wj_archive_status(
                    install_id,
                    &archive.hash.0,
                    name,
                    archive.state.source_type_name(),
                    status,
                    None,
                    Some(&e.to_string()),
                );
            }
        }

        result
    }

    // -----------------------------------------------------------------------
    // Parallel orchestrator
    // -----------------------------------------------------------------------

    /// Download all archives in the list, respecting concurrency limits and a
    /// shared cancel token. Returns a map of archive_hash -> file_path for
    /// successfully downloaded archives.
    ///
    /// Resume support: after each successful download + verification, a
    /// checkpoint file is written to `.wj_checkpoint/{hash}.done` inside the
    /// download directory. On subsequent runs the checkpoint is detected and
    /// the archive is skipped. The checkpoint directory is removed on
    /// successful completion of all archives.
    pub async fn download_all_archives(
        &self,
        app: &AppHandle,
        db: &Arc<ModDatabase>,
        install_id: i64,
        archives: &[WjTypedArchive],
        concurrency: usize,
        cancel_token: Arc<AtomicBool>,
    ) -> Result<WjDownloadPhaseResult, WjDownloadError> {
        let total = archives.len();
        let results: Arc<tokio::sync::Mutex<HashMap<String, PathBuf>>> =
            Arc::new(tokio::sync::Mutex::new(HashMap::new()));

        // Resume checkpoint directory.
        let checkpoint_dir = self.download_dir.join(".wj_checkpoint");
        tokio::fs::create_dir_all(&checkpoint_dir).await?;

        let all_succeeded = Arc::new(AtomicBool::new(true));
        let failures: Arc<tokio::sync::Mutex<Vec<String>>> =
            Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let mut skipped_count: usize = 0;

        // -----------------------------------------------------------------
        // Pre-filter pass: check checkpoints and cache sequentially (fast,
        // mostly filesystem metadata lookups). Only archives that actually
        // need downloading are pushed into `to_download`.
        // -----------------------------------------------------------------
        let mut to_download: Vec<(usize, WjTypedArchive)> = Vec::new();

        for (index, archive) in archives.iter().enumerate() {
            if cancel_token.load(Ordering::Relaxed) {
                all_succeeded.store(false, Ordering::Relaxed);
                break;
            }

            let hash_str = archive.hash.0.clone();
            let archive_name = archive.name.clone();

            // ----- Resume checkpoint check -----
            let checkpoint_file =
                checkpoint_dir.join(format!("{}.done", sanitize_filename(&hash_str)));
            if checkpoint_file.exists() {
                let dest = self.download_dir.join(sanitize_filename(&archive_name));
                if dest.exists() {
                    if verify_xxhash64(&dest, &archive.hash).is_ok() {
                        let _ = app.emit(
                            "wabbajack-install-progress",
                            WjProgressEvent::DownloadSkipped {
                                archive_name: archive_name.clone(),
                                reason: "resume checkpoint".into(),
                            },
                        );
                        let _ = db.upsert_wj_archive_status(
                            install_id,
                            &hash_str,
                            &archive_name,
                            archive.state.source_type_name(),
                            "verified",
                            Some(&dest.to_string_lossy()),
                            None,
                        );
                        results.lock().await.insert(hash_str, dest);
                        skipped_count += 1;
                        continue;
                    }
                    log::warn!(
                        "Checkpoint archive failed hash check, re-downloading: {}",
                        archive_name
                    );
                    let _ = tokio::fs::remove_file(&dest).await;
                }
                let _ = tokio::fs::remove_file(&checkpoint_file).await;
            }

            // Check shared cache -- re-verify hash before trusting it.
            if let Ok(Some(cached_path)) = db.find_download_by_xxhash(&hash_str) {
                let cached = PathBuf::from(&cached_path);
                if cached.exists() {
                    if verify_xxhash64(&cached, &archive.hash).is_ok() {
                        let _ = app.emit(
                            "wabbajack-install-progress",
                            WjProgressEvent::DownloadSkipped {
                                archive_name: archive_name.clone(),
                                reason: "already cached".into(),
                            },
                        );
                        let _ = db.upsert_wj_archive_status(
                            install_id,
                            &hash_str,
                            &archive_name,
                            archive.state.source_type_name(),
                            "verified",
                            Some(&cached_path),
                            None,
                        );
                        let _ = write_checkpoint(&checkpoint_file).await;
                        results.lock().await.insert(hash_str, cached);
                        skipped_count += 1;
                        continue;
                    } else {
                        log::warn!(
                            "Cached archive failed hash check, re-downloading: {}",
                            archive_name
                        );
                    }
                }
            }

            to_download.push((index, archive.clone()));
        }

        // -----------------------------------------------------------------
        // Concurrent download phase: run actual HTTP downloads in parallel
        // using buffer_unordered for true concurrency.
        // -----------------------------------------------------------------
        let sem = Arc::new(Semaphore::new(concurrency));

        let download_futures = to_download.into_iter().map(|(index, archive)| {
            let mut downloader = self.clone();
            downloader.cancel_token = Some(cancel_token.clone());
            let app = app.clone();
            let db = db.clone();
            let cancel = cancel_token.clone();
            let sem = sem.clone();
            let results = results.clone();
            let failures = failures.clone();
            let checkpoint_dir = checkpoint_dir.clone();
            let all_succeeded = all_succeeded.clone();

            async move {
                let _permit = sem.acquire_owned().await.expect("semaphore closed");

                if cancel.load(Ordering::Relaxed) {
                    all_succeeded.store(false, Ordering::Relaxed);
                    return;
                }

                let hash_str = archive.hash.0.clone();
                let archive_name = archive.name.clone();
                let checkpoint_file =
                    checkpoint_dir.join(format!("{}.done", sanitize_filename(&hash_str)));

                let _ = app.emit(
                    "wabbajack-install-progress",
                    WjProgressEvent::DownloadStarted {
                        archive_name: archive_name.clone(),
                        index,
                        total,
                    },
                );

                match downloader
                    .download_archive(&app, &archive, install_id, &db)
                    .await
                {
                    Ok(path) => {
                        match verify_xxhash64(&path, &archive.hash) {
                            Ok(()) => {
                                let _ = app.emit(
                                    "wabbajack-install-progress",
                                    WjProgressEvent::DownloadCompleted {
                                        archive_name: archive_name.clone(),
                                    },
                                );
                                let _ = db.upsert_wj_archive_status(
                                    install_id,
                                    &hash_str,
                                    &archive_name,
                                    archive.state.source_type_name(),
                                    "verified",
                                    Some(&path.to_string_lossy()),
                                    None,
                                );
                                let _ = write_checkpoint(&checkpoint_file).await;
                                results.lock().await.insert(hash_str, path);
                            }
                            Err(e) => {
                                all_succeeded.store(false, Ordering::Relaxed);
                                let err_msg = format!("{}: hash mismatch — {}", archive_name, e);
                                failures.lock().await.push(err_msg);
                                let _ = tokio::fs::remove_file(&path).await;
                                let _ = app.emit(
                                    "wabbajack-install-progress",
                                    WjProgressEvent::DownloadFailed {
                                        archive_name: archive_name.clone(),
                                        error: e.to_string(),
                                    },
                                );
                                let _ = db.upsert_wj_archive_status(
                                    install_id,
                                    &hash_str,
                                    &archive_name,
                                    archive.state.source_type_name(),
                                    "failed",
                                    None,
                                    Some(&e.to_string()),
                                );
                            }
                        }
                    }
                    Err(WjDownloadError::Cancelled) => {
                        all_succeeded.store(false, Ordering::Relaxed);
                        failures.lock().await.push(format!("{}: cancelled", archive_name));
                        let _ = app.emit(
                            "wabbajack-install-progress",
                            WjProgressEvent::DownloadFailed {
                                archive_name: archive_name.clone(),
                                error: "Download cancelled".into(),
                            },
                        );
                        // Signal cancellation so other tasks stop too.
                        cancel.store(true, Ordering::Relaxed);
                    }
                    Err(WjDownloadError::UserActionRequired(msg)) => {
                        all_succeeded.store(false, Ordering::Relaxed);
                        failures.lock().await.push(format!("{}: {}", archive_name, msg));
                        let url_part = msg
                            .split_whitespace()
                            .find(|w| w.starts_with("http"))
                            .unwrap_or("")
                            .to_string();
                        let _ = app.emit(
                            "wabbajack-install-progress",
                            WjProgressEvent::UserActionRequired {
                                archive_name: archive_name.clone(),
                                url: url_part,
                                prompt: msg.clone(),
                            },
                        );
                    }
                    Err(e) => {
                        all_succeeded.store(false, Ordering::Relaxed);
                        failures.lock().await.push(format!("{}: {}", archive_name, e));
                        let _ = app.emit(
                            "wabbajack-install-progress",
                            WjProgressEvent::DownloadFailed {
                                archive_name: archive_name.clone(),
                                error: e.to_string(),
                            },
                        );
                    }
                }
            }
        });

        futures::stream::iter(download_futures)
            .buffer_unordered(concurrency)
            .collect::<Vec<()>>()
            .await;

        // -----------------------------------------------------------------
        // Secondary retry pass: any archives that failed during the
        // concurrent wave get one more sequential attempt after a cooldown.
        // This mirrors the collections downloader approach and handles
        // transient CDN issues that resolve after a short wait.
        // -----------------------------------------------------------------
        {
            let current_results = results.lock().await;
            let failed_archives: Vec<(usize, WjTypedArchive)> = archives
                .iter()
                .enumerate()
                .filter(|(_, a)| !current_results.contains_key(&a.hash.0))
                .filter(|(_, a)| {
                    // Only retry transient-type failures, not unsupported/user-action
                    !matches!(
                        a.state,
                        WjArchiveState::Bethesda { .. }
                            | WjArchiveState::LoversLab { .. }
                            | WjArchiveState::VectorPlexus { .. }
                            | WjArchiveState::TESAlliance { .. }
                    )
                })
                .map(|(i, a)| (i, a.clone()))
                .collect();
            drop(current_results);

            if !failed_archives.is_empty()
                && !cancel_token.load(Ordering::Relaxed)
            {
                log::info!(
                    "Secondary retry pass: {} archives to retry after 10s cooldown",
                    failed_archives.len()
                );
                tokio::time::sleep(Duration::from_secs(10)).await;

                for (index, archive) in &failed_archives {
                    if cancel_token.load(Ordering::Relaxed) {
                        break;
                    }

                    let archive_name = archive.name.clone();
                    let hash_str = archive.hash.0.clone();

                    log::info!("Retrying download: {} (archive {}/{})", archive_name, index + 1, total);
                    let _ = app.emit(
                        "wabbajack-install-progress",
                        WjProgressEvent::DownloadStarted {
                            archive_name: archive_name.clone(),
                            index: *index,
                            total,
                        },
                    );

                    let mut downloader = self.clone();
                    downloader.cancel_token = Some(cancel_token.clone());

                    match downloader.download_archive(app, &archive, install_id, db).await {
                        Ok(path) => {
                            match verify_xxhash64(&path, &archive.hash) {
                                Ok(()) => {
                                    let _ = app.emit(
                                        "wabbajack-install-progress",
                                        WjProgressEvent::DownloadCompleted {
                                            archive_name: archive_name.clone(),
                                        },
                                    );
                                    let checkpoint_file = checkpoint_dir.join(format!(
                                        "{}.done",
                                        sanitize_filename(&hash_str),
                                    ));
                                    let _ = write_checkpoint(&checkpoint_file).await;
                                    results.lock().await.insert(hash_str, path);
                                    // Remove from failures list
                                    let mut fl = failures.lock().await;
                                    fl.retain(|f| !f.starts_with(&archive_name));
                                }
                                Err(e) => {
                                    let _ = tokio::fs::remove_file(&path).await;
                                    let _ = app.emit(
                                        "wabbajack-install-progress",
                                        WjProgressEvent::DownloadFailed {
                                            archive_name: archive_name.clone(),
                                            error: format!("Retry hash mismatch: {}", e),
                                        },
                                    );
                                }
                            }
                        }
                        Err(WjDownloadError::Cancelled) => break,
                        Err(e) => {
                            let _ = app.emit(
                                "wabbajack-install-progress",
                                WjProgressEvent::DownloadFailed {
                                    archive_name: archive_name.clone(),
                                    error: format!("Retry failed: {}", e),
                                },
                            );
                        }
                    }
                }
            }
        }

        // Clean up checkpoint directory on successful full completion.
        if all_succeeded.load(Ordering::Relaxed) {
            let _ = tokio::fs::remove_dir_all(&checkpoint_dir).await;
        }

        let map = Arc::try_unwrap(results)
            .map(|mutex| mutex.into_inner())
            .unwrap_or_else(|arc| {
                // Should not happen, but handle gracefully.
                let guard = arc.blocking_lock();
                guard.clone()
            });

        let failure_list = Arc::try_unwrap(failures)
            .map(|mutex| mutex.into_inner())
            .unwrap_or_else(|arc| {
                let guard = arc.blocking_lock();
                guard.clone()
            });
        let failed_count = failure_list.len();

        Ok(WjDownloadPhaseResult {
            paths: map,
            failed_count,
            skipped_count,
            failures: failure_list,
        })
    }

    // -----------------------------------------------------------------------
    // Streaming download helper
    // -----------------------------------------------------------------------

    /// Stream an HTTP response body to a file with progress events.
    ///
    /// Supports:
    /// - **HTTP Range resume**: if a `.partial` file exists from a previous
    ///   failed attempt, sends a `Range` header to resume from where it left
    ///   off. Falls back to a fresh download if the server doesn't support it.
    /// - **Stall detection**: uses `tokio::select!` to race the byte stream
    ///   against a stall timer. If no bytes arrive for `STALL_TIMEOUT_SECS`,
    ///   the download is aborted so the retry loop can kick in.
    async fn stream_download(
        &self,
        app: &AppHandle,
        archive_name: &str,
        url: &str,
        extra_headers: &HeaderMap,
    ) -> Result<PathBuf, WjDownloadError> {
        let dest = self.download_dir.join(sanitize_filename(archive_name));
        let partial = dest.with_extension("partial");

        // --- HTTP Range resume: check for a partial file from a prior attempt ---
        let partial_size = if partial.exists() {
            tokio::fs::metadata(&partial)
                .await
                .map(|m| m.len())
                .unwrap_or(0)
        } else {
            0
        };

        let mut request = self.http_client.get(url).headers(extra_headers.clone());
        if partial_size > 0 {
            log::info!(
                "Resuming download of '{}' from byte {}",
                archive_name,
                partial_size
            );
            request = request.header("Range", format!("bytes={}-", partial_size));
        }

        let resp = request.send().await?;
        let status = resp.status();

        // Determine whether we're resuming or starting fresh.
        let (mut file, offset) = if status == reqwest::StatusCode::PARTIAL_CONTENT
            && partial_size > 0
        {
            // Server supports Range -- append to the existing partial file.
            log::info!(
                "Server returned 206 Partial Content, resuming from byte {}",
                partial_size
            );
            let file = tokio::fs::OpenOptions::new()
                .append(true)
                .open(&partial)
                .await?;
            (file, partial_size)
        } else if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
            // Range is invalid (e.g. file changed on server) -- start over.
            log::warn!(
                "Server returned 416 Range Not Satisfiable for '{}', restarting download",
                archive_name
            );
            let _ = tokio::fs::remove_file(&partial).await;
            let file = tokio::fs::File::create(&partial).await?;
            (file, 0)
        } else if status.is_success() {
            // Either a fresh 200 OK (no Range support) or first download.
            if partial_size > 0 {
                log::info!(
                    "Server does not support Range resume for '{}' (got {}), restarting",
                    archive_name,
                    status.as_u16()
                );
            }
            let _ = tokio::fs::remove_file(&partial).await;
            let file = tokio::fs::File::create(&partial).await?;
            (file, 0)
        } else {
            let code = status.as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(WjDownloadError::Other(format!("HTTP {code}: {body}")));
        };

        // Content-Length from the response reflects remaining bytes (for 206)
        // or total bytes (for 200).
        let remaining_bytes = resp.content_length().unwrap_or(0);
        let total_bytes = offset + remaining_bytes;
        let mut stream = resp.bytes_stream();
        let mut downloaded: u64 = offset;
        // Throttle progress events to avoid flooding.
        let mut last_progress_emit: u64 = 0;

        // --- Stall detection via tokio::select! + sleep reset ---
        let last_progress_time = Arc::new(AtomicU64::new(now_epoch()));

        use tokio::io::AsyncWriteExt;

        let stream_result: Result<(), WjDownloadError> = async {
            let stall_sleep = tokio::time::sleep(Duration::from_secs(STALL_TIMEOUT_SECS));
            tokio::pin!(stall_sleep);

            loop {
                tokio::select! {
                    chunk_opt = stream.next() => {
                        match chunk_opt {
                            Some(Ok(chunk)) => {
                                // Check cancel token.
                                if let Some(ref token) = self.cancel_token {
                                    if token.load(Ordering::Relaxed) {
                                        return Err(WjDownloadError::Cancelled);
                                    }
                                }

                                file.write_all(&chunk).await?;
                                downloaded += chunk.len() as u64;

                                // Reset stall deadline.
                                last_progress_time.store(now_epoch(), Ordering::Relaxed);
                                stall_sleep.as_mut().reset(
                                    tokio::time::Instant::now()
                                        + Duration::from_secs(STALL_TIMEOUT_SECS),
                                );

                                // Emit progress at most every 256 KB.
                                if downloaded - last_progress_emit > 256 * 1024
                                    || downloaded == total_bytes
                                {
                                    let _ = app.emit(
                                        "wabbajack-install-progress",
                                        WjProgressEvent::DownloadProgress {
                                            archive_name: archive_name.to_string(),
                                            bytes_downloaded: downloaded,
                                            total_bytes,
                                        },
                                    );
                                    last_progress_emit = downloaded;
                                }
                            }
                            Some(Err(e)) => return Err(WjDownloadError::Http(e)),
                            None => return Ok(()), // stream finished
                        }
                    }
                    _ = &mut stall_sleep => {
                        let elapsed = now_epoch().saturating_sub(
                            last_progress_time.load(Ordering::Relaxed),
                        );
                        log::warn!(
                            "Download stalled for {}s, aborting for retry",
                            elapsed
                        );
                        return Err(WjDownloadError::Stalled { secs: elapsed });
                    }
                }
            }
        }
        .await;

        match stream_result {
            Ok(()) => {
                file.flush().await?;
                drop(file);
                // Rename .partial -> final name.
                tokio::fs::rename(&partial, &dest).await?;
                Ok(dest)
            }
            Err(WjDownloadError::Cancelled) => {
                drop(file);
                // On cancel, remove partial (user explicitly cancelled).
                let _ = tokio::fs::remove_file(&partial).await;
                Err(WjDownloadError::Cancelled)
            }
            Err(WjDownloadError::Stalled { secs }) => {
                // Keep partial file so Range resume can pick up where we left off.
                drop(file);
                log::warn!(
                    "Download of '{}' stalled after {}s at {} bytes, partial file kept for resume",
                    archive_name,
                    secs,
                    downloaded
                );
                Err(WjDownloadError::Stalled { secs })
            }
            Err(e) => {
                // Keep partial file for resume on transient errors.
                drop(file);
                Err(e)
            }
        }
    }

    /// Stream an already-obtained HTTP response body to a file with progress
    /// events. Used when we already hold a `reqwest::Response` (e.g. the
    /// Google Drive fast path where the first response is the actual file).
    ///
    /// Includes stall detection but no Range resume since we already have the
    /// response in hand.
    async fn stream_download_from_response(
        &self,
        app: &AppHandle,
        archive_name: &str,
        resp: reqwest::Response,
    ) -> Result<PathBuf, WjDownloadError> {
        let dest = self.download_dir.join(sanitize_filename(archive_name));
        let partial = dest.with_extension("partial");

        let total_bytes = resp.content_length().unwrap_or(0);
        let mut stream = resp.bytes_stream();
        let mut file = tokio::fs::File::create(&partial).await?;
        let mut downloaded: u64 = 0;
        let mut last_progress_emit: u64 = 0;

        // --- Stall detection via tokio::select! + sleep reset ---
        let last_progress_time = Arc::new(AtomicU64::new(now_epoch()));

        use tokio::io::AsyncWriteExt;

        let stream_result: Result<(), WjDownloadError> = async {
            let stall_sleep = tokio::time::sleep(Duration::from_secs(STALL_TIMEOUT_SECS));
            tokio::pin!(stall_sleep);

            loop {
                tokio::select! {
                    chunk_opt = stream.next() => {
                        match chunk_opt {
                            Some(Ok(chunk)) => {
                                if let Some(ref token) = self.cancel_token {
                                    if token.load(Ordering::Relaxed) {
                                        return Err(WjDownloadError::Cancelled);
                                    }
                                }

                                file.write_all(&chunk).await?;
                                downloaded += chunk.len() as u64;

                                last_progress_time.store(now_epoch(), Ordering::Relaxed);
                                stall_sleep.as_mut().reset(
                                    tokio::time::Instant::now()
                                        + Duration::from_secs(STALL_TIMEOUT_SECS),
                                );

                                if downloaded - last_progress_emit > 256 * 1024
                                    || downloaded == total_bytes
                                {
                                    let _ = app.emit(
                                        "wabbajack-install-progress",
                                        WjProgressEvent::DownloadProgress {
                                            archive_name: archive_name.to_string(),
                                            bytes_downloaded: downloaded,
                                            total_bytes,
                                        },
                                    );
                                    last_progress_emit = downloaded;
                                }
                            }
                            Some(Err(e)) => return Err(WjDownloadError::Http(e)),
                            None => return Ok(()),
                        }
                    }
                    _ = &mut stall_sleep => {
                        let elapsed = now_epoch().saturating_sub(
                            last_progress_time.load(Ordering::Relaxed),
                        );
                        log::warn!(
                            "Download stalled for {}s, aborting for retry",
                            elapsed
                        );
                        return Err(WjDownloadError::Stalled { secs: elapsed });
                    }
                }
            }
        }
        .await;

        match stream_result {
            Ok(()) => {
                file.flush().await?;
                drop(file);
                tokio::fs::rename(&partial, &dest).await?;
                Ok(dest)
            }
            Err(WjDownloadError::Cancelled) => {
                drop(file);
                let _ = tokio::fs::remove_file(&partial).await;
                Err(WjDownloadError::Cancelled)
            }
            Err(WjDownloadError::Stalled { secs }) => {
                drop(file);
                log::warn!(
                    "Download of '{}' stalled after {}s at {} bytes",
                    archive_name,
                    secs,
                    downloaded
                );
                Err(WjDownloadError::Stalled { secs })
            }
            Err(e) => {
                drop(file);
                Err(e)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Retry helpers
// ---------------------------------------------------------------------------

/// Returns true for transient errors that are worth retrying (network, IO).
/// Permanent HTTP status codes (401, 403, 404, 410) are NOT retried.
fn is_transient_error(e: &WjDownloadError) -> bool {
    match e {
        WjDownloadError::Http(req_err) => {
            // Check if the reqwest error wraps a known-permanent status code.
            if let Some(status) = req_err.status() {
                let code = status.as_u16();
                // Permanent client errors — retrying won't help.
                if matches!(code, 401 | 403 | 404 | 410 | 451) {
                    return false;
                }
                // Transient server errors + rate limiting.
                if matches!(code, 408 | 429 | 500 | 502 | 503 | 504) {
                    return true;
                }
                // Other 4xx are usually permanent.
                if (400..500).contains(&code) {
                    return false;
                }
            }
            // Network-level errors (timeout, connect, etc.) are transient.
            req_err.is_timeout() || req_err.is_connect() || req_err.is_request()
        }
        WjDownloadError::Io(_) => true,
        WjDownloadError::Other(msg) => {
            // Classify HTTP status codes embedded in "HTTP NNN:" error strings
            // (from stream_download's manual status check).
            if msg.starts_with("HTTP ") {
                let code_str = msg
                    .trim_start_matches("HTTP ")
                    .split(':')
                    .next()
                    .unwrap_or("");
                if let Ok(code) = code_str.parse::<u16>() {
                    return !matches!(code, 401 | 403 | 404 | 410 | 451)
                        && (matches!(code, 408 | 429 | 500 | 502 | 503 | 504) || code >= 500);
                }
            }
            // MEGA / other generic errors — treat as transient if network-related.
            msg.contains("timed out")
                || msg.contains("timeout")
                || msg.contains("connection")
                || msg.contains("network")
        }
        WjDownloadError::Stalled { .. } => true,
        _ => false,
    }
}

/// Returns the current epoch time in seconds.
fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ---------------------------------------------------------------------------
// Hash verification
// ---------------------------------------------------------------------------

/// Verify that a downloaded file matches the expected xxHash64 (base64).
/// Returns Ok(()) on match, Err(WjDownloadError::HashMismatch) otherwise.
pub fn verify_xxhash64(path: &Path, expected: &WjHash) -> Result<(), WjDownloadError> {
    if expected.is_empty() {
        // No hash to verify -- treat as pass.
        return Ok(());
    }

    let actual = xxhash64_file(path).map_err(WjDownloadError::Io)?;

    if actual != *expected {
        return Err(WjDownloadError::HashMismatch {
            expected: expected.0.clone(),
            actual: actual.0,
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Resume checkpoint helper
// ---------------------------------------------------------------------------

/// Write a checkpoint file indicating an archive was successfully downloaded
/// and verified. The file contains a timestamp for debugging purposes.
async fn write_checkpoint(path: &Path) -> Result<(), std::io::Error> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    tokio::fs::write(path, format!("{now}")).await
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Map Wabbajack's `Game` field (e.g. "SkyrimSpecialEdition") to a NexusMods
/// API game domain slug (e.g. "skyrimspecialedition").
fn wj_game_to_nexus_domain(game: &str) -> String {
    // Wabbajack's C# Game enum uses PascalCase. NexusMods API requires the
    // lowercase slug. We handle the known mappings and fall back to lowercasing.
    match game {
        "Skyrim" | "skyrim" => "skyrim".into(),
        "SkyrimSpecialEdition" | "skyrimspecialedition" => "skyrimspecialedition".into(),
        "SkyrimVR" | "skyrimvr" => "skyrimvr".into(),
        "Fallout4" | "fallout4" => "fallout4".into(),
        "Fallout4VR" | "fallout4vr" => "fallout4vr".into(),
        "FalloutNewVegas" | "falloutnewvegas" => "falloutnewvegas".into(),
        "Fallout3" | "fallout3" => "fallout3".into(),
        "Oblivion" | "oblivion" => "oblivion".into(),
        "Morrowind" | "morrowind" => "morrowind".into(),
        "Enderal" | "enderal" => "enderal".into(),
        "EnderalSpecialEdition" | "enderalspecialedition" => "enderalspecialedition".into(),
        "Cyberpunk2077" | "cyberpunk2077" => "cyberpunk2077".into(),
        "StardewValley" | "stardewvalley" => "stardewvalley".into(),
        "Witcher3" | "witcher3" | "TheWitcher3" => "witcher3".into(),
        "Starfield" | "starfield" => "starfield".into(),
        "BaldursGate3" | "baldursgate3" => "baldursgate3".into(),
        "DragonAgeInquisition" | "dragonageinquisition" => "dragonageinquisition".into(),
        "MechWarrior5" | "mechwarrior5mercenaries" => "mechwarrior5mercenaries".into(),
        "NoMansSky" | "nomanssky" => "nomanssky".into(),
        "KingdomComeDeliverance" | "kingdomcomedeliverance" => "kingdomcomedeliverance".into(),
        // Utility/tool domains on NexusMods (not actual games)
        "ModdingTools" | "moddingtools" | "site" => "site".into(),
        "DarkSouls" | "darksouls" => "darksouls".into(),
        "DarkSouls3" | "darksouls3" => "darksouls3".into(),
        "MountAndBlade2Bannerlord" | "mountandblade2bannerlord" => {
            "mountandblade2bannerlord".into()
        }
        "EldenRing" | "eldenring" => "eldenring".into(),
        "DragonsDogma2" | "dragonsdogma2" => "dragonsdogma2".into(),
        other => other.to_lowercase(),
    }
}

/// Sanitize a filename by replacing unsafe characters.
fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "download".to_string()
    } else {
        cleaned
    }
}

// ---------------------------------------------------------------------------
// CDN concurrent download
// ---------------------------------------------------------------------------

/// Download a file from WJ CDN using concurrent parts.
async fn download_cdn_concurrent(
    client: &reqwest::Client,
    app: &AppHandle,
    archive_name: &str,
    url: &str,
    dest_path: &Path,
    total_size: u64,
    cancel_token: &Arc<AtomicBool>,
) -> Result<(), WjDownloadError> {
    const MAX_CONCURRENT_PARTS: usize = 16;
    const MIN_PART_SIZE: u64 = 1024 * 1024; // 1MB minimum per part

    // Pre-allocate the file
    let file = std::fs::File::create(dest_path).map_err(WjDownloadError::Io)?;
    file.set_len(total_size).map_err(WjDownloadError::Io)?;
    drop(file);

    // Calculate part ranges
    let num_parts = if total_size / MIN_PART_SIZE > MAX_CONCURRENT_PARTS as u64 {
        MAX_CONCURRENT_PARTS
    } else {
        (total_size / MIN_PART_SIZE).max(1) as usize
    };
    let part_size = total_size / num_parts as u64;

    let mut ranges: Vec<(u64, u64)> = Vec::new();
    for i in 0..num_parts {
        let start = i as u64 * part_size;
        let end = if i == num_parts - 1 {
            total_size - 1
        } else {
            (i as u64 + 1) * part_size - 1
        };
        ranges.push((start, end));
    }

    log::info!(
        "CDN concurrent download: {} parts, {:.1}MB each",
        num_parts,
        part_size as f64 / 1_048_576.0
    );

    // Shared counter for aggregate progress across all parts
    let bytes_downloaded = Arc::new(AtomicU64::new(0));
    let last_emit = Arc::new(AtomicU64::new(0));

    let dest = dest_path.to_path_buf();
    let results: Vec<Result<(), WjDownloadError>> = futures::stream::iter(ranges)
        .map(|(start, end)| {
            let client = client.clone();
            let url = url.to_string();
            let dest = dest.clone();
            let cancel = cancel_token.clone();
            let bytes_downloaded = bytes_downloaded.clone();
            let last_emit = last_emit.clone();
            let app = app.clone();
            let name = archive_name.to_string();

            async move {
                if cancel.load(Ordering::Relaxed) {
                    return Err(WjDownloadError::Cancelled);
                }

                let response = client
                    .get(&url)
                    .header("Range", format!("bytes={}-{}", start, end))
                    .send()
                    .await?;

                // Stream response bytes to buffer, emitting progress as chunks arrive.
                let mut stream = response.bytes_stream();
                let mut buf = Vec::with_capacity((end - start + 1) as usize);
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk?;
                    buf.extend_from_slice(&chunk);

                    // Update aggregate byte counter and emit throttled progress
                    let total_dl = bytes_downloaded.fetch_add(chunk.len() as u64, Ordering::Relaxed) + chunk.len() as u64;
                    let prev = last_emit.load(Ordering::Relaxed);
                    if total_dl - prev > 256 * 1024 {
                        last_emit.store(total_dl, Ordering::Relaxed);
                        let _ = app.emit(
                            "wabbajack-install-progress",
                            WjProgressEvent::DownloadProgress {
                                archive_name: name.clone(),
                                bytes_downloaded: total_dl,
                                total_bytes: total_size,
                            },
                        );
                    }
                }

                // Write buffer to file at offset (blocking I/O in spawn_blocking)
                let offset = start;
                tokio::task::spawn_blocking(move || {
                    use std::io::{Seek, SeekFrom, Write};
                    let mut file = std::fs::OpenOptions::new()
                        .write(true)
                        .open(&dest)
                        .map_err(WjDownloadError::Io)?;
                    file.seek(SeekFrom::Start(offset))
                        .map_err(WjDownloadError::Io)?;
                    file.write_all(&buf).map_err(WjDownloadError::Io)?;
                    Ok::<(), WjDownloadError>(())
                })
                .await
                .map_err(|e| WjDownloadError::Other(format!("Task join error: {}", e)))?;

                Ok(())
            }
        })
        .buffer_unordered(MAX_CONCURRENT_PARTS)
        .collect()
        .await;

    // Check for errors
    for result in results {
        result?;
    }

    // Emit final progress to ensure 100%
    let _ = app.emit(
        "wabbajack-install-progress",
        WjProgressEvent::DownloadProgress {
            archive_name: archive_name.to_string(),
            bytes_downloaded: total_size,
            total_bytes: total_size,
        },
    );

    Ok(())
}

/// Remap CDN URLs to preferred domains.
fn remap_cdn_url(url: &str) -> String {
    // B-CDN → official WJ domain fallback
    url.replace("authored-files.wabbajack.b-cdn.net", "build.wabbajack.org")
        .replace("wabbajack.b-cdn.net", "build.wabbajack.org")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_wj_game_to_nexus_domain() {
        assert_eq!(
            wj_game_to_nexus_domain("SkyrimSpecialEdition"),
            "skyrimspecialedition"
        );
        assert_eq!(wj_game_to_nexus_domain("Fallout4"), "fallout4");
        assert_eq!(wj_game_to_nexus_domain("Oblivion"), "oblivion");
        // Unknown game falls back to lowercase.
        assert_eq!(wj_game_to_nexus_domain("SomeNewGame"), "somenewgame");
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("SkyUI_5_2_SE.zip"), "SkyUI_5_2_SE.zip");
        assert_eq!(
            sanitize_filename("mod with spaces.7z"),
            "mod_with_spaces.7z"
        );
        assert_eq!(
            sanitize_filename("../../../etc/passwd"),
            ".._.._.._etc_passwd"
        );
        assert_eq!(sanitize_filename(""), "download");
    }

    #[test]
    fn test_verify_xxhash64_matches() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.bin");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(b"hello world").unwrap();
        }

        // Compute the expected hash.
        let expected = xxhash64_bytes(b"hello world");
        assert!(verify_xxhash64(&path, &expected).is_ok());
    }

    #[test]
    fn test_verify_xxhash64_mismatch() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.bin");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(b"hello world").unwrap();
        }

        let wrong = WjHash::from_u64(0xDEADBEEF);
        let err = verify_xxhash64(&path, &wrong).unwrap_err();
        match err {
            WjDownloadError::HashMismatch { .. } => {} // expected
            other => panic!("Expected HashMismatch, got: {other}"),
        }
    }

    #[test]
    fn test_verify_xxhash64_empty_hash_passes() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.bin");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(b"anything").unwrap();
        }

        let empty = WjHash::default();
        assert!(verify_xxhash64(&path, &empty).is_ok());
    }

    #[test]
    fn test_progress_event_serialization() {
        let event = WjProgressEvent::DownloadStarted {
            archive_name: "test.zip".into(),
            index: 0,
            total: 10,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"DownloadStarted\""));
        assert!(json.contains("\"archive_name\":\"test.zip\""));
    }

    #[test]
    fn test_downloader_free_user_nexus_blocked() {
        // Verify that constructing a downloader with is_premium=false
        // and no API key means the struct is in the right state.
        let tmp = TempDir::new().unwrap();
        let dl = WjDownloader::new(None, None, false, tmp.path().to_path_buf());
        assert!(!dl.is_premium);
        assert!(dl.nexus_api_key.is_none());
    }

    #[tokio::test]
    async fn test_write_checkpoint_creates_file() {
        let tmp = TempDir::new().unwrap();
        let checkpoint_path = tmp.path().join("abc123.done");
        write_checkpoint(&checkpoint_path).await.unwrap();
        assert!(checkpoint_path.exists());

        // File should contain a timestamp (numeric string).
        let contents = tokio::fs::read_to_string(&checkpoint_path).await.unwrap();
        assert!(
            contents.parse::<u64>().is_ok(),
            "Expected numeric timestamp, got: {contents}"
        );
    }

    #[tokio::test]
    async fn test_checkpoint_dir_cleanup() {
        let tmp = TempDir::new().unwrap();
        let checkpoint_dir = tmp.path().join(".wj_checkpoint");
        tokio::fs::create_dir_all(&checkpoint_dir).await.unwrap();

        // Write a few checkpoint files.
        write_checkpoint(&checkpoint_dir.join("hash1.done"))
            .await
            .unwrap();
        write_checkpoint(&checkpoint_dir.join("hash2.done"))
            .await
            .unwrap();
        assert!(checkpoint_dir.exists());

        // Simulate cleanup on success.
        tokio::fs::remove_dir_all(&checkpoint_dir).await.unwrap();
        assert!(!checkpoint_dir.exists());
    }

    #[test]
    fn test_sanitize_filename_for_checkpoint_hash() {
        // Base64 hashes may contain +, /, = characters -- sanitize should
        // replace them so checkpoint filenames are safe.
        let hash = "abc+def/ghi=jk==";
        let sanitized = sanitize_filename(hash);
        assert!(!sanitized.contains('+'));
        assert!(!sanitized.contains('/'));
        assert!(!sanitized.contains('='));
        assert_eq!(sanitized, "abc_def_ghi_jk__");
    }

    #[test]
    fn test_remap_cdn_url_b_cdn() {
        assert_eq!(
            remap_cdn_url("https://authored-files.wabbajack.b-cdn.net/some/file.7z"),
            "https://build.wabbajack.org/some/file.7z"
        );
        assert_eq!(
            remap_cdn_url("https://wabbajack.b-cdn.net/files/archive.zip"),
            "https://build.wabbajack.org/files/archive.zip"
        );
    }

    #[test]
    fn test_remap_cdn_url_passthrough() {
        let url = "https://build.wabbajack.org/already/correct.7z";
        assert_eq!(remap_cdn_url(url), url);

        let url2 = "https://example.com/unrelated.zip";
        assert_eq!(remap_cdn_url(url2), url2);
    }
}
