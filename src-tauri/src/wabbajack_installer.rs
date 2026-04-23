// ---------------------------------------------------------------------------
// Wabbajack Install Orchestrator
//
// Coordinates the full Wabbajack modlist installation pipeline:
//   Pending → PreFlight → Downloading → Extracting → Processing → Deploying → Completed
//
// Uses WjDownloader for archive downloads (Phase 4), DirectiveProcessor for
// file production/patching (Phase 5), and deployer for game directory
// deployment with atomic rollback (Phase 6).
// ---------------------------------------------------------------------------

use crate::database::ModDatabase;
use crate::nexus;
use crate::oauth;
use crate::wabbajack_directives::DirectiveProcessor;
use crate::wabbajack_downloader::{verify_xxhash64, WjDownloader};
use crate::wabbajack_types::*;

use serde::Serialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Semaphore;

// ---------------------------------------------------------------------------
// Failure categorization
//
// WJ downloads fail for many reasons — categorizing them lets users tell
// "this list is broken" apart from "my network is flaky" apart from "I need
// premium for this". Pattern-matches against the error strings collected in
// `download_failures`.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum FailureKind {
    NotFound,         // 404 / removed from Nexus
    Unauthorized,     // 401/403 / premium required / auth missing
    RateLimited,      // 429 / rate limit
    Network,          // timeout / DNS / connection reset
    HashMismatch,     // xxHash64 mismatch after download
    DiskFull,         // no space left
    ServerError,      // 5xx
    Unknown,
}

impl FailureKind {
    fn label(self) -> &'static str {
        match self {
            Self::NotFound => "Removed (404)",
            Self::Unauthorized => "Auth / premium required",
            Self::RateLimited => "Rate limited",
            Self::Network => "Network / timeout",
            Self::HashMismatch => "Hash mismatch",
            Self::DiskFull => "Disk full",
            Self::ServerError => "Server error",
            Self::Unknown => "Other",
        }
    }
}

fn classify_failure(msg: &str) -> FailureKind {
    let m = msg.to_ascii_lowercase();
    if m.contains("404") || m.contains("not found") || m.contains("no such file") {
        FailureKind::NotFound
    } else if m.contains("401") || m.contains("403") || m.contains("unauthorized")
        || m.contains("forbidden") || m.contains("premium")
    {
        FailureKind::Unauthorized
    } else if m.contains("429") || m.contains("rate limit") {
        FailureKind::RateLimited
    } else if m.contains("hash mismatch") || m.contains("xxhash") {
        FailureKind::HashMismatch
    } else if m.contains("no space") || m.contains("disk full") || m.contains("enospc") {
        FailureKind::DiskFull
    } else if m.contains("500") || m.contains("502") || m.contains("503")
        || m.contains("504") || m.contains("bad gateway") || m.contains("service unavailable")
    {
        FailureKind::ServerError
    } else if m.contains("timeout") || m.contains("timed out") || m.contains("dns")
        || m.contains("connection") || m.contains("reset") || m.contains("network")
    {
        FailureKind::Network
    } else {
        FailureKind::Unknown
    }
}

fn summarize_failures(failures: &[String]) -> String {
    if failures.is_empty() {
        return "no error details available".to_string();
    }
    let mut counts: HashMap<FailureKind, usize> = HashMap::new();
    for f in failures {
        *counts.entry(classify_failure(f)).or_insert(0) += 1;
    }
    let mut entries: Vec<(FailureKind, usize)> = counts.into_iter().collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1));
    let breakdown: Vec<String> = entries
        .iter()
        .map(|(k, n)| format!("{} × {}", n, k.label()))
        .collect();
    let first = failures.first().map(|s| s.as_str()).unwrap_or("unknown");
    format!("{}. First: {}", breakdown.join(", "), first)
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum WjInstallError {
    #[error("Pre-flight check failed: {0}")]
    PreFlight(String),
    #[error("Download error: {0}")]
    Download(String),
    #[error("Extraction error: {0}")]
    Extraction(String),
    #[error("Directive error: {0}")]
    Directive(String),
    #[error("Deployment error: {0}")]
    Deployment(String),
    #[error("Installation cancelled")]
    Cancelled,
    #[error("Database error: {0}")]
    Database(String),
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("{0}")]
    Other(String),
}

// ---------------------------------------------------------------------------
// Progress event types (emitted to frontend via Tauri events)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct WjInstallResult {
    pub install_id: i64,
    pub status: String,
    pub total_archives: usize,
    pub total_directives: usize,
    pub files_deployed: usize,
    pub elapsed_secs: f64,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WjPreflightReport {
    pub can_proceed: bool,
    pub issues: Vec<WjPreflightIssue>,
    pub total_download_size: u64,
    pub total_archives: usize,
    pub total_directives: usize,
    pub cached_archives: usize,
    pub disk_space_available: u64,
    pub disk_space_required: u64,
    pub nexus_archives: usize,
    pub is_nexus_premium: bool,
    pub manual_downloads: usize,
    /// Game version required by the modlist (from GameFileSource archives).
    pub required_game_version: Option<String>,
    /// Currently installed game version (detected from executable).
    pub detected_game_version: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type")]
pub enum WjInstallProgressEvent {
    PreFlightStarted,
    PreFlightCompleted {
        report: WjPreflightReport,
    },
    DownloadPhaseStarted {
        total: usize,
        max_concurrent: usize,
    },
    DownloadStarted {
        name: String,
        index: usize,
        total: usize,
    },
    DownloadProgress {
        name: String,
        bytes: u64,
        total_bytes: u64,
    },
    DownloadCompleted {
        name: String,
    },
    DownloadFailed {
        name: String,
        error: String,
    },
    DownloadSkipped {
        name: String,
        reason: String,
    },
    DownloadPhaseCompleted {
        total: usize,
        succeeded: usize,
        failed: usize,
        skipped: usize,
        failures: Vec<String>,
    },
    ExtractionStarted {
        total: usize,
        total_bytes: u64,
    },
    ExtractionProgress {
        name: String,
        index: usize,
        total: usize,
        bytes_completed: u64,
        total_bytes: u64,
    },
    ExtractionArchiveStarted {
        name: String,
        index: usize,
        total: usize,
        size: u64,
    },
    ExtractionArchiveCompleted {
        name: String,
        index: usize,
    },
    ExtractionArchiveFailed {
        name: String,
        error: String,
    },
    DirectivePhaseStarted {
        total: usize,
        total_bytes: u64,
    },
    DirectiveProgress {
        current: usize,
        total: usize,
        directive_type: String,
        bytes_processed: u64,
        total_bytes: u64,
        current_file: String,
    },
    DeployStarted {
        total: usize,
        total_bytes: u64,
        modlist_name: String,
    },
    DeployProgress {
        current: usize,
        total: usize,
        bytes_deployed: u64,
        total_bytes: u64,
        modlist_name: String,
    },
    InstallCompleted {
        result: WjInstallResult,
    },
    InstallFailed {
        error: String,
    },
    InstallCancelled,
    UserActionRequired {
        archive_name: String,
        url: String,
        prompt: String,
    },
}

// ---------------------------------------------------------------------------
// Wabbajack file parser (typed)
// ---------------------------------------------------------------------------

/// Parse a .wabbajack ZIP file and deserialize the modlist JSON into a
/// strongly-typed `WjTypedModlist`. Tries entry names "modlist" then
/// "modlist.json".
/// Public wrapper for CLI testing of typed WJ parse.
pub fn parse_wabbajack_file_typed_cli(path: &Path) -> Result<WjTypedModlist, String> {
    parse_wabbajack_file_typed(path)
}

fn parse_wabbajack_file_typed(path: &Path) -> Result<WjTypedModlist, String> {
    let file =
        std::fs::File::open(path).map_err(|e| format!("Cannot open .wabbajack file: {}", e))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("Not a valid ZIP/.wabbajack file: {}", e))?;

    let modlist_json = {
        let try_entry =
            |archive: &mut zip::ZipArchive<std::fs::File>, name: &str| -> Result<String, String> {
                let mut entry = archive.by_name(name).map_err(|e| e.to_string())?;
                let mut buf = String::new();
                entry.read_to_string(&mut buf).map_err(|e| e.to_string())?;
                Ok(buf)
            };
        try_entry(&mut archive, "modlist")
            .or_else(|_| try_entry(&mut archive, "modlist.json"))
            .map_err(|_| {
                "No 'modlist' or 'modlist.json' entry found in .wabbajack file".to_string()
            })?
    };

    serde_json::from_str::<WjTypedModlist>(&modlist_json)
        .map_err(|e| format!("Failed to deserialize modlist: {}", e))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Emit a progress event to the frontend.
fn emit_progress(app: &AppHandle, event: &WjInstallProgressEvent) {
    if let Err(e) = app.emit("wj-install-progress", event) {
        log::warn!("Failed to emit wj-install-progress event: {}", e);
    }
}

/// Check if the cancellation token has been set.
fn is_cancelled(cancel_token: &AtomicBool) -> bool {
    cancel_token.load(Ordering::Relaxed)
}

/// Get available disk space for the filesystem containing `path`.
///
/// Uses `libc::statvfs` on macOS/Linux to query the underlying filesystem.
/// Returns 0 if the path doesn't exist or the statvfs call fails.
fn get_available_disk_space(path: &Path) -> u64 {
    use std::ffi::CString;

    // Ensure the path exists — walk up to the nearest existing ancestor
    let check_path = if path.exists() {
        path.to_path_buf()
    } else {
        let mut ancestor = path.to_path_buf();
        while !ancestor.exists() {
            if !ancestor.pop() {
                return 0;
            }
        }
        ancestor
    };

    let c_path = match CString::new(check_path.to_string_lossy().as_bytes()) {
        Ok(p) => p,
        Err(_) => return 0,
    };

    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut stat) == 0 {
            // Casts needed for cross-platform: field types differ between macOS and Linux
            #[allow(clippy::unnecessary_cast)]
            {
                stat.f_bavail as u64 * stat.f_frsize as u64
            }
        } else {
            0
        }
    }
}

/// Check Nexus premium status using the current auth method.
async fn check_nexus_premium() -> bool {
    let method = oauth::get_auth_method();
    match method {
        oauth::AuthMethod::ApiKey(key) => {
            let client = nexus::NexusClient::new(key);
            client.is_premium().await
        }
        oauth::AuthMethod::OAuth(tokens) => match oauth::parse_user_info(&tokens.access_token) {
            Ok(user) => user.is_premium,
            Err(_) => false,
        },
        oauth::AuthMethod::None => false,
    }
}

// ---------------------------------------------------------------------------
// Pre-flight check
// ---------------------------------------------------------------------------

/// Run pre-flight checks before starting a Wabbajack installation.
///
/// Parses the .wabbajack file, checks disk space, validates Nexus premium
/// status, counts cached archives, and identifies any manual-download
/// archives. Returns a report with all findings and whether the install
/// can proceed.
pub async fn preflight_check(
    app: &AppHandle,
    db: &Arc<ModDatabase>,
    wabbajack_path: &Path,
    game_id: &str,
    bottle_name: &str,
    install_dir: &Path,
    download_dir: &Path,
) -> Result<WjPreflightReport, WjInstallError> {
    emit_progress(app, &WjInstallProgressEvent::PreFlightStarted);

    // 1. Parse the .wabbajack file (typed)
    let modlist = parse_wabbajack_file_typed(wabbajack_path).map_err(WjInstallError::Parse)?;

    let total_archives = modlist.archives.len();
    let total_directives = modlist.directives.len();

    // 2. Calculate total download size
    let total_download_size: u64 = modlist.archives.iter().map(|a| a.size).sum();

    // 3. Check disk space for both download_dir and install_dir
    let download_space = get_available_disk_space(download_dir);
    let install_space = get_available_disk_space(install_dir);

    // Estimate: download size + installed files (roughly 2x download for extraction headroom)
    let disk_space_required = total_download_size.saturating_mul(2);
    let disk_space_available = download_space.min(install_space);

    // 4. Count Nexus archives and check premium status
    let nexus_archives = modlist
        .archives
        .iter()
        .filter(|a| matches!(a.state, WjArchiveState::Nexus { .. }))
        .count();

    let is_nexus_premium = if nexus_archives > 0 {
        check_nexus_premium().await
    } else {
        false
    };

    // 5. Count manual-download archives
    let manual_downloads = modlist
        .archives
        .iter()
        .filter(|a| matches!(a.state, WjArchiveState::Manual { .. }))
        .count();

    // 6. Count cached archives (already downloaded and available)
    let mut cached_archives = 0usize;
    for archive in &modlist.archives {
        let hash_str = &archive.hash.0;
        if !hash_str.is_empty() {
            if let Ok(Some(_path)) = db.find_download_by_xxhash(hash_str) {
                cached_archives += 1;
            }
        }
    }

    // 7. Detect game version and check against modlist requirements
    let required_game_version = modlist.required_game_version();
    let detected_game_version = detect_installed_game_version(game_id, bottle_name);

    // 8. Collect issues
    let mut issues = Vec::new();

    if disk_space_available < disk_space_required {
        issues.push(WjPreflightIssue {
            severity: "error".to_string(),
            message: format!(
                "Insufficient disk space: need {} GB, have {} GB",
                disk_space_required / (1024 * 1024 * 1024),
                disk_space_available / (1024 * 1024 * 1024)
            ),
        });
    }

    if nexus_archives > 0 && !is_nexus_premium {
        issues.push(WjPreflightIssue {
            severity: "warning".to_string(),
            message: format!(
                "{} archives require Nexus Mods. Without Premium, you must manually \
                 download each one via the Nexus website (Slow Download).",
                nexus_archives
            ),
        });
    }

    if manual_downloads > 0 {
        issues.push(WjPreflightIssue {
            severity: "warning".to_string(),
            message: format!(
                "{} archives require manual download from external sites.",
                manual_downloads
            ),
        });
    }

    // Game version mismatch check
    if let (Some(ref required), Some(ref detected)) =
        (&required_game_version, &detected_game_version)
    {
        let required_major = parse_version_major(required);
        let detected_major = parse_version_major(detected);

        if let (Some(req), Some(det)) = (required_major, detected_major) {
            // Major version mismatch (e.g. 1.5 vs 1.6) is a blocking error
            if req != det {
                issues.push(WjPreflightIssue {
                    severity: "error".to_string(),
                    message: format!(
                        "Game version mismatch: modlist requires {} but your game is {}. \
                         You may need to downgrade or update your game.",
                        required, detected
                    ),
                });
            } else if !versions_match_normalized(required, detected) {
                // Same major, different patch — warn but allow
                issues.push(WjPreflightIssue {
                    severity: "warning".to_string(),
                    message: format!(
                        "Game version differs slightly: modlist was built for {} but \
                         your game is {}. This may still work.",
                        required, detected
                    ),
                });
            }
        }
    } else if required_game_version.is_some() && detected_game_version.is_none() {
        issues.push(WjPreflightIssue {
            severity: "warning".to_string(),
            message: format!(
                "Modlist requires game version {} but we couldn't detect your installed version.",
                required_game_version.as_deref().unwrap_or("unknown")
            ),
        });
    }

    // Can proceed if there are no "error" severity issues
    let can_proceed = !issues.iter().any(|i| i.severity == "error");

    let report = WjPreflightReport {
        can_proceed,
        issues,
        total_download_size,
        total_archives,
        total_directives,
        cached_archives,
        disk_space_available,
        disk_space_required,
        nexus_archives,
        is_nexus_premium,
        manual_downloads,
        required_game_version,
        detected_game_version,
    };

    emit_progress(
        app,
        &WjInstallProgressEvent::PreFlightCompleted {
            report: report.clone(),
        },
    );

    Ok(report)
}

/// Detect the installed game version by resolving the game path from bottle + game_id.
/// Returns a version string like "1.6.1170" or "1.5.97", or None if detection fails.
fn detect_installed_game_version(game_id: &str, bottle_name: &str) -> Option<String> {
    // Currently only Skyrim SE has detailed version detection
    if !game_id.eq_ignore_ascii_case("skyrimse") {
        return None;
    }

    let bottles = crate::bottles::detect_bottles();
    let bottle = bottles.iter().find(|b| b.name == bottle_name)?;

    let game_path = crate::games::with_plugin(game_id, |plugin| {
        plugin.detect(bottle).map(|g| g.game_path)
    })
    .flatten()?;

    match crate::downgrader::detect_skyrim_version(&game_path) {
        Ok(status) => {
            log::info!(
                "Detected Skyrim version: {} (downgraded: {})",
                status.current_version,
                status.is_downgraded
            );
            Some(status.current_version)
        }
        Err(e) => {
            log::warn!("Failed to detect Skyrim version: {}", e);
            None
        }
    }
}

/// Parse the major version prefix from a version string.
/// e.g. "1.6.1170.0" → "1.6", "1.5.97" → "1.5"
fn parse_version_major(version: &str) -> Option<String> {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() >= 2 {
        Some(format!("{}.{}", parts[0], parts[1]))
    } else {
        None
    }
}

/// Compare version strings with normalization.
/// Handles: "1.6.1170" == "1.6.1170.0", trailing ".0" removal,
/// and fuzzy "1.6.x" detected versions matching any "1.6.NNNN" required.
fn versions_match_normalized(required: &str, detected: &str) -> bool {
    // Exact match
    if required == detected {
        return true;
    }
    // Extract numeric parts only (strip labels like "(Anniversary Edition, ~35.4 MB)")
    let req_nums = extract_version_nums(required);
    let det_nums = extract_version_nums(detected);
    // If detected is "1.6.x" (unknown AE build), accept any required 1.6.x version
    if det_nums.len() >= 2 && det_nums[0] == req_nums.get(0).copied().unwrap_or(0)
        && det_nums[1] == req_nums.get(1).copied().unwrap_or(0)
        && detected.contains("1.6.x")
    {
        return true;
    }
    // Compare numeric parts, ignoring trailing zeros
    // "1.6.1170" == "1.6.1170.0"
    let max_len = req_nums.len().max(det_nums.len());
    for i in 0..max_len {
        let r = req_nums.get(i).copied().unwrap_or(0);
        let d = det_nums.get(i).copied().unwrap_or(0);
        if r != d {
            return false;
        }
    }
    true
}

/// Extract numeric version parts from a version string.
/// "1.6.1170.0" → [1, 6, 1170, 0]
/// "1.6.x (Anniversary Edition, ~35.4 MB)" → [1, 6]
fn extract_version_nums(s: &str) -> Vec<u32> {
    s.split('.')
        .map_while(|part| part.trim().parse::<u32>().ok())
        .collect()
}

// ---------------------------------------------------------------------------
// Phase metrics helpers
// ---------------------------------------------------------------------------

/// Get the current process RSS (Resident Set Size) in bytes.
fn get_rss_bytes() -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
            for line in status.lines() {
                if line.starts_with("VmRSS:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if let Some(kb_str) = parts.get(1) {
                        if let Ok(kb) = kb_str.parse::<u64>() {
                            return kb * 1024;
                        }
                    }
                }
            }
        }
        return 0;
    }

    #[cfg(target_os = "macos")]
    {
        use std::mem;

        // Layout must match macOS `mach_task_basic_info` (stable since macOS 10.x).
        // If Apple changes this struct in a future OS, RSS reporting will return 0
        // (safe fallback). Consider the `mach2` crate if this breaks.
        #[repr(C)]
        struct MachTaskBasicInfo {
            virtual_size: u64,
            resident_size: u64,
            resident_size_max: u64,
            user_time: [u32; 2],
            system_time: [u32; 2],
            policy: i32,
            suspend_count: i32,
        }

        const MACH_TASK_BASIC_INFO: u32 = 20;

        extern "C" {
            fn mach_task_self() -> u32;
            fn task_info(
                target_task: u32,
                flavor: u32,
                task_info_out: *mut MachTaskBasicInfo,
                task_info_count: *mut u32,
            ) -> i32;
        }

        let mut info: MachTaskBasicInfo = unsafe { mem::zeroed() };
        let mut count =
            (mem::size_of::<MachTaskBasicInfo>() / mem::size_of::<u32>()) as u32;

        let result =
            unsafe { task_info(mach_task_self(), MACH_TASK_BASIC_INFO, &mut info, &mut count) };

        if result == 0 {
            return info.resident_size;
        }
        return 0;
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        0
    }
}

/// Emit phase metrics (elapsed time + RSS memory) for observability.
fn log_phase_metrics(app: &AppHandle, phase: &str, start_time: Instant) {
    let elapsed_ms = start_time.elapsed().as_millis() as u64;
    let rss_bytes = get_rss_bytes();

    log::info!(
        "WJ phase '{}' completed in {}ms (RSS: {:.1}MB)",
        phase,
        elapsed_ms,
        rss_bytes as f64 / 1_048_576.0,
    );

    let _ = app.emit(
        "wj://phase-metrics",
        serde_json::json!({
            "phase": phase,
            "elapsed_ms": elapsed_ms,
            "rss_bytes": rss_bytes,
        }),
    );

    // On Linux (glibc), call malloc_trim to return freed pages to OS.
    // Critical on Steam Deck with 16GB shared RAM.
    #[cfg(target_os = "linux")]
    unsafe {
        libc::malloc_trim(0);
        log::debug!("malloc_trim(0) called after phase '{}'", phase);
    }
}

// ---------------------------------------------------------------------------
// Main install orchestrator
// ---------------------------------------------------------------------------

/// Run the full Wabbajack modlist installation pipeline.
///
/// Pipeline steps:
/// 1. Parse the .wabbajack file
/// 2. Run pre-flight checks
/// 3. Create a DB record for this install
/// 4. Download phase (multi-source: Nexus, HTTP, MediaFire, Mega, Google Drive, WJ CDN)
/// 5. Extraction phase (extract each archive to temp dirs)
/// 6. Directive processing phase (BSDiff, CreateBSA/BA2, DDS transform, MergedPatch, inline)
/// 7. Deploy phase (hardlink-first with copy fallback)
/// 8. Update DB record to completed
/// 9. Return result
///
/// Checks the `cancel_token` at the start of each major loop iteration.
/// If cancelled, updates DB to cancelled, emits InstallCancelled, and
/// returns `WjInstallError::Cancelled`.
#[allow(clippy::too_many_arguments)]
pub async fn install_wabbajack_modlist(
    app: &AppHandle,
    db: &Arc<ModDatabase>,
    wabbajack_path: &Path,
    game_id: &str,
    bottle_name: &str,
    install_dir: &Path,
    download_dir: &Path,
    cancel_token: Arc<AtomicBool>,
) -> Result<WjInstallResult, WjInstallError> {
    let start_time = Instant::now();
    let mut warnings: Vec<String> = Vec::new();

    // Compute parallelism from CPU core count.
    // Downloads are I/O-bound (network) so they get more headroom.
    // Extraction and directives are mixed I/O + CPU, capped lower.
    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let download_concurrency = cpu_cores.clamp(2, 16);
    let extract_concurrency = cpu_cores.clamp(2, 8);

    log::info!(
        "Parallelism: {} cores detected → {} download, {} extraction threads",
        cpu_cores, download_concurrency, extract_concurrency
    );

    // -----------------------------------------------------------------------
    // Step 1: Parse the .wabbajack file
    // -----------------------------------------------------------------------
    log::info!("Parsing Wabbajack modlist: {:?}", wabbajack_path);

    let modlist = parse_wabbajack_file_typed(wabbajack_path).map_err(WjInstallError::Parse)?;

    let total_archives = modlist.archives.len();
    let total_directives = modlist.directives.len();

    log::info!(
        "Modlist '{}' v{}: {} archives, {} directives",
        modlist.name,
        modlist.version,
        total_archives,
        total_directives
    );

    // -----------------------------------------------------------------------
    // Step 2: Run pre-flight check
    // -----------------------------------------------------------------------
    let phase_start = Instant::now();
    let preflight_report = preflight_check(
        app,
        db,
        wabbajack_path,
        game_id,
        bottle_name,
        install_dir,
        download_dir,
    )
    .await?;

    if !preflight_report.can_proceed {
        let error_msgs: Vec<String> = preflight_report
            .issues
            .iter()
            .filter(|i| i.severity == "error")
            .map(|i| i.message.clone())
            .collect();
        return Err(WjInstallError::PreFlight(error_msgs.join("; ")));
    }

    // Collect warnings from pre-flight
    for issue in &preflight_report.issues {
        if issue.severity == "warning" {
            warnings.push(issue.message.clone());
        }
    }

    log_phase_metrics(app, "preflight", phase_start);

    // Check cancellation
    if is_cancelled(&cancel_token) {
        return Err(WjInstallError::Cancelled);
    }

    // -----------------------------------------------------------------------
    // Step 3: Create DB record
    // -----------------------------------------------------------------------
    let install_id = db
        .create_wj_install(
            &modlist.name,
            &modlist.version,
            modlist.game_type,
            &install_dir.to_string_lossy(),
            total_archives,
            total_directives,
        )
        .map_err(|e| WjInstallError::Database(e.to_string()))?;

    log::info!("Created wabbajack_installs record: id={}", install_id);

    // Helper: update DB status on failure/cancel
    let _mark_failed = |db: &ModDatabase, id: i64, err: &str| {
        let _ = db.update_wj_install_status(id, "failed", Some(err));
    };
    let mark_cancelled = |db: &ModDatabase, id: i64| {
        let _ = db.update_wj_install_status(id, "cancelled", None);
    };

    // -----------------------------------------------------------------------
    // Steps 4+5: Combined download + extraction phase
    //
    // Each archive flows through download → verify → extract in a single
    // task. Dual semaphores ensure download slots are released before
    // extraction begins, so later archives can start downloading while
    // earlier ones are still extracting. This overlaps I/O-bound downloads
    // with CPU-bound extraction for significantly better throughput.
    // -----------------------------------------------------------------------
    let phase_start = Instant::now();
    db.update_wj_install_status(install_id, "downloading", None)
        .map_err(|e| WjInstallError::Database(e.to_string()))?;

    emit_progress(
        app,
        &WjInstallProgressEvent::DownloadPhaseStarted {
            total: total_archives,
            max_concurrent: download_concurrency,
        },
    );

    // Get Nexus API key and premium status for the downloader
    let (nexus_api_key, nexus_oauth_token, is_premium) = match oauth::get_auth_method() {
        oauth::AuthMethod::ApiKey(key) => {
            let client = nexus::NexusClient::new(key.clone());
            let premium = client.is_premium().await;
            (Some(key), None, premium)
        }
        oauth::AuthMethod::OAuth(tokens) => {
            let premium = oauth::parse_user_info(&tokens.access_token)
                .map(|u| u.is_premium)
                .unwrap_or(false);
            (None, Some(tokens.access_token.clone()), premium)
        }
        oauth::AuthMethod::None => (None, None, false),
    };

    let downloader =
        WjDownloader::new(nexus_api_key, nexus_oauth_token, is_premium, download_dir.to_path_buf());

    // Dual semaphores: download permits released before extraction starts
    let download_sem = Arc::new(Semaphore::new(download_concurrency));
    let extract_sem = Arc::new(Semaphore::new(extract_concurrency));

    // Shared state collected by spawned tasks
    let extracted_dirs: Arc<std::sync::Mutex<HashMap<String, PathBuf>>> =
        Arc::new(std::sync::Mutex::new(HashMap::new()));
    let archive_download_paths: Arc<std::sync::Mutex<HashMap<String, PathBuf>>> =
        Arc::new(std::sync::Mutex::new(HashMap::new()));
    let download_failures: Arc<std::sync::Mutex<Vec<String>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let download_skipped_count = Arc::new(AtomicU64::new(0));

    // Build hash→size map for extraction byte-level progress
    let archive_size_map: HashMap<String, u64> = modlist
        .archives
        .iter()
        .map(|a| (a.hash.0.clone(), a.size))
        .collect();
    let total_extract_bytes: u64 = archive_size_map.values().sum();
    let extract_bytes_completed = Arc::new(AtomicU64::new(0));

    // Extraction temp: prefer install_dir, fall back to download_dir if install_dir
    // is not writable (e.g. read-only volumes on macOS, sandboxed paths).
    let extraction_temp_base = {
        let preferred = install_dir.join(".wj_extraction_temp");
        match std::fs::create_dir_all(&preferred) {
            Ok(()) => {
                // Verify writable by attempting a temp file
                let probe = preferred.join(".write_probe");
                match std::fs::write(&probe, b"ok") {
                    Ok(()) => {
                        let _ = std::fs::remove_file(&probe);
                        preferred
                    }
                    Err(e) => {
                        log::warn!(
                            "Extraction temp at install_dir is not writable ({}), falling back to download_dir",
                            e
                        );
                        let fallback = download_dir.join(".wj_extraction_temp");
                        std::fs::create_dir_all(&fallback).ok();
                        fallback
                    }
                }
            }
            Err(e) => {
                log::warn!(
                    "Cannot create extraction temp at install_dir ({}), falling back to download_dir",
                    e
                );
                let fallback = download_dir.join(".wj_extraction_temp");
                std::fs::create_dir_all(&fallback).ok();
                fallback
            }
        }
    };
    let checkpoint_dir = download_dir.join(".wj_checkpoint");
    std::fs::create_dir_all(&checkpoint_dir).ok();

    let collection_name = format!("wj:{}", modlist.name);

    // Inline helper: sanitize a string for use as a filename
    fn sanitize_for_filename(s: &str) -> String {
        s.chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' })
            .collect()
    }

    // -----------------------------------------------------------------
    // Pre-filter: check checkpoints and DB cache sequentially (fast
    // filesystem metadata lookups). Only archives needing work are
    // pushed into `to_process`.
    // -----------------------------------------------------------------
    let mut to_process: Vec<(usize, WjTypedArchive)> = Vec::new();

    for (index, archive) in modlist.archives.iter().enumerate() {
        if is_cancelled(&cancel_token) {
            break;
        }

        let hash_str = archive.hash.0.clone();
        let archive_name = archive.name.clone();
        let checkpoint_file =
            checkpoint_dir.join(format!("{}.done", sanitize_for_filename(&hash_str)));
        let extract_dest = extraction_temp_base.join(&hash_str);

        // Check resume checkpoint: archive downloaded + extracted previously
        if checkpoint_file.exists() {
            let dest = download_dir.join(sanitize_for_filename(&archive_name));
            if dest.exists() && extract_dest.exists() {
                // Both download and extraction present — skip entirely
                if verify_xxhash64(&dest, &archive.hash).is_ok() {
                    emit_progress(
                        app,
                        &WjInstallProgressEvent::DownloadSkipped {
                            name: archive_name.clone(),
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
                    archive_download_paths.lock().unwrap().insert(hash_str.clone(), dest);
                    extracted_dirs.lock().unwrap().insert(hash_str, extract_dest);
                    download_skipped_count.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                log::warn!(
                    "Checkpoint archive failed hash check, re-processing: {}",
                    archive_name
                );
            }
            // Checkpoint exists but files missing/invalid — remove and re-process
            let _ = std::fs::remove_file(&checkpoint_file);
        }

        // Check shared cache
        if let Ok(Some(cached_path)) = db.find_download_by_xxhash(&hash_str) {
            let cached = PathBuf::from(&cached_path);
            if cached.exists() && verify_xxhash64(&cached, &archive.hash).is_ok() {
                // Have download cached but still need extraction — check if extracted dir exists
                if extract_dest.exists() {
                    emit_progress(
                        app,
                        &WjInstallProgressEvent::DownloadSkipped {
                            name: archive_name.clone(),
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
                    let now_secs = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let _ = std::fs::write(&checkpoint_file, format!("{now_secs}"));
                    archive_download_paths.lock().unwrap().insert(hash_str.clone(), cached);
                    extracted_dirs.lock().unwrap().insert(hash_str, extract_dest);
                    download_skipped_count.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                // Cached download exists but needs extraction — still push to process
                // but the download step will be skipped inside the task
            }
        }

        to_process.push((index, archive.clone()));
    }

    // -----------------------------------------------------------------
    // Spawn combined download+extract tasks for each archive
    // -----------------------------------------------------------------
    let process_count = to_process.len();
    let mut handles = Vec::with_capacity(process_count);

    for (index, archive) in to_process {
        if is_cancelled(&cancel_token) {
            break;
        }

        let mut task_downloader = downloader.clone();
        task_downloader.set_cancel_token(cancel_token.clone());

        let dl_sem = Arc::clone(&download_sem);
        let ext_sem = Arc::clone(&extract_sem);
        let cancel = Arc::clone(&cancel_token);
        let app_c = app.clone();
        let db_c = db.clone();
        let extracted_dirs_c = Arc::clone(&extracted_dirs);
        let download_paths_c = Arc::clone(&archive_download_paths);
        let failures_c = Arc::clone(&download_failures);
        let bytes_completed = Arc::clone(&extract_bytes_completed);
        let extraction_temp = extraction_temp_base.clone();
        let checkpoint_dir_c = checkpoint_dir.clone();
        let archive_size = archive_size_map.get(&archive.hash.0).copied().unwrap_or(0);
        let total_bytes = total_extract_bytes;
        let total = total_archives;
        let collection_name_c = collection_name.clone();
        let game_id_c = game_id.to_string();
        let bottle_name_c = bottle_name.to_string();

        let handle = tokio::spawn(async move {
            let hash_str = archive.hash.0.clone();
            let archive_name = archive.name.clone();

            // Check cancellation
            if cancel.load(Ordering::Relaxed) {
                return;
            }

            // ---- Download phase (acquire download permit) ----
            let dl_permit = dl_sem.acquire().await.expect("download semaphore closed");

            if cancel.load(Ordering::Relaxed) {
                drop(dl_permit);
                return;
            }

            // Check if download already exists (cached but needs extraction)
            let checkpoint_file = checkpoint_dir_c
                .join(format!("{}.done", sanitize_for_filename(&hash_str)));
            let cached_download = db_c
                .find_download_by_xxhash(&hash_str)
                .ok()
                .flatten()
                .map(PathBuf::from)
                .filter(|p| p.exists())
                .filter(|p| verify_xxhash64(p, &archive.hash).is_ok());

            let download_path = if let Some(cached) = cached_download {
                // Already have a valid download — skip downloading
                let _ = app_c.emit(
                    "wabbajack-install-progress",
                    crate::wabbajack_downloader::WjProgressEvent::DownloadSkipped {
                        archive_name: archive_name.clone(),
                        reason: "already cached".into(),
                    },
                );
                emit_progress(
                    &app_c,
                    &WjInstallProgressEvent::DownloadSkipped {
                        name: archive_name.clone(),
                        reason: "already cached".into(),
                    },
                );
                drop(dl_permit); // Release download slot immediately
                Some(cached)
            } else {
                // Emit download started
                emit_progress(
                    &app_c,
                    &WjInstallProgressEvent::DownloadStarted {
                        name: archive_name.clone(),
                        index,
                        total,
                    },
                );

                let dl_result = task_downloader
                    .download_archive(&app_c, &archive, install_id, &db_c)
                    .await;

                // Release download permit BEFORE extraction
                drop(dl_permit);

                match dl_result {
                    Ok(path) => {
                        // Verify hash
                        match verify_xxhash64(&path, &archive.hash) {
                            Ok(()) => {
                                emit_progress(
                                    &app_c,
                                    &WjInstallProgressEvent::DownloadCompleted {
                                        name: archive_name.clone(),
                                    },
                                );
                                let _ = db_c.upsert_wj_archive_status(
                                    install_id,
                                    &hash_str,
                                    &archive_name,
                                    archive.state.source_type_name(),
                                    "verified",
                                    Some(&path.to_string_lossy()),
                                    None,
                                );
                                Some(path)
                            }
                            Err(e) => {
                                let err_msg =
                                    format!("{}: hash mismatch — {}", archive_name, e);
                                log::error!("{}", err_msg);
                                let _ = tokio::fs::remove_file(&path).await;
                                emit_progress(
                                    &app_c,
                                    &WjInstallProgressEvent::DownloadFailed {
                                        name: archive_name.clone(),
                                        error: e.to_string(),
                                    },
                                );
                                failures_c.lock().unwrap().push(err_msg);
                                None
                            }
                        }
                    }
                    Err(e) => {
                        let err_msg = format!("{}: {}", archive_name, e);
                        log::error!("Download failed: {}", err_msg);
                        emit_progress(
                            &app_c,
                            &WjInstallProgressEvent::DownloadFailed {
                                name: archive_name.clone(),
                                error: e.to_string(),
                            },
                        );
                        failures_c.lock().unwrap().push(err_msg);
                        None
                    }
                }
            };

            // If download failed, skip extraction
            let download_path = match download_path {
                Some(p) => p,
                None => return,
            };

            // Record download path
            download_paths_c
                .lock()
                .unwrap()
                .insert(hash_str.clone(), download_path.clone());

            // Register in download_registry for orphan tracking
            {
                let filename = download_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let file_size =
                    std::fs::metadata(&download_path).map(|m| m.len() as i64).unwrap_or(0);
                let (nexus_mod_id, nexus_file_id) = match &archive.state {
                    WjArchiveState::Nexus { mod_id, file_id, .. } => {
                        (Some(*mod_id), Some(*file_id))
                    }
                    _ => (None, None),
                };
                if let Ok(download_id) = db_c.register_download(
                    &download_path.to_string_lossy(),
                    &filename,
                    nexus_mod_id,
                    nexus_file_id,
                    None,
                    file_size,
                ) {
                    let _ = db_c.add_download_collection_ref(
                        download_id,
                        &collection_name_c,
                        &game_id_c,
                        &bottle_name_c,
                    );
                }
            }

            // Check cancellation before extraction
            if cancel.load(Ordering::Relaxed) {
                return;
            }

            // ---- Extraction phase (acquire extraction permit) ----
            let _ext_permit = ext_sem.acquire().await.expect("extract semaphore closed");

            if cancel.load(Ordering::Relaxed) {
                return;
            }

            let extract_dest = extraction_temp.join(&hash_str);

            emit_progress(
                &app_c,
                &WjInstallProgressEvent::ExtractionArchiveStarted {
                    name: archive_name.clone(),
                    index,
                    total,
                    size: archive_size,
                },
            );

            // Determine if this is a single-file source (game file, bare .dll/.exe/.ccc etc.)
            // that should be placed directly into the extraction dir rather than extracted.
            let is_single_file_source = matches!(
                &archive.state,
                crate::wabbajack_types::WjArchiveState::GameFileSource { .. }
            ) || {
                // Check if the downloaded file is not a recognized archive format.
                // Common single-file extensions from game directories:
                let name_lower = download_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase();
                let single_exts = [
                    ".dll", ".exe", ".esm", ".esp", ".esl", ".bsa", ".ba2",
                    ".bik", ".bk2", ".ccc", ".ini", ".cfg", ".txt", ".pdf",
                    ".bat", ".py", ".json", ".xml", ".css", ".toml", ".hkx",
                    ".nif", ".dds", ".pex", ".psc", ".wav", ".xwm", ".fuz",
                    ".swf", ".tlx", ".clx",
                ];
                single_exts.iter().any(|ext| name_lower.ends_with(ext))
                    && !name_lower.ends_with(".7z")
                    && !name_lower.ends_with(".zip")
                    && !name_lower.ends_with(".rar")
            };

            // Extraction with retry: on failure, delete archive, re-download, re-extract
            let max_extract_retries = 2u32;
            let mut extract_ok = false;

            if is_single_file_source {
                // Single-file source: place the file directly into the extraction
                // directory under its original filename. Directives reference files
                // inside archives by path — for single files, the "archive" contains
                // exactly one file: the file itself.
                if let Err(e) = tokio::fs::create_dir_all(&extract_dest).await {
                    let err_msg = format!(
                        "Failed to create extraction dir for single-file '{}': {}",
                        archive_name, e
                    );
                    log::error!("{}", err_msg);
                    failures_c.lock().unwrap().push(err_msg.clone());
                    emit_progress(
                        &app_c,
                        &WjInstallProgressEvent::ExtractionArchiveFailed {
                            name: archive_name.clone(),
                            error: err_msg,
                        },
                    );
                    return;
                }

                // Determine the filename to use inside the extraction dir.
                // For GameFileSource, use the game_file path so directives can find it.
                // For other single files, use the archive name.
                let inner_name = if let crate::wabbajack_types::WjArchiveState::GameFileSource {
                    game_file, ..
                } = &archive.state
                {
                    game_file.replace('\\', "/")
                } else {
                    archive_name.clone()
                };

                let dest_file = extract_dest.join(&inner_name);
                if let Some(parent) = dest_file.parent() {
                    let _ = tokio::fs::create_dir_all(parent).await;
                }

                match tokio::fs::copy(&download_path, &dest_file).await {
                    Ok(_) => {
                        log::info!(
                            "Placed single-file source '{}' as '{}'",
                            archive_name,
                            inner_name
                        );
                        extract_ok = true;
                    }
                    Err(e) => {
                        let err_msg = format!(
                            "Failed to copy single-file source '{}': {}",
                            archive_name, e
                        );
                        log::error!("{}", err_msg);
                        failures_c.lock().unwrap().push(err_msg.clone());
                        emit_progress(
                            &app_c,
                            &WjInstallProgressEvent::ExtractionArchiveFailed {
                                name: archive_name.clone(),
                                error: err_msg,
                            },
                        );
                        return;
                    }
                }
            } else {
            // Archive extraction with retry
            for attempt in 0..=max_extract_retries {
                let dest_c = extract_dest.clone();
                let archive_for_blocking = download_path.clone();
                let result = tokio::task::spawn_blocking(move || {
                    crate::installer::extract_archive(&archive_for_blocking, &dest_c)
                })
                .await;

                match result {
                    Ok(Ok(_files)) => {
                        extract_ok = true;
                        break;
                    }
                    Ok(Err(e)) => {
                        log::error!(
                            "Extraction attempt {}/{} failed for '{}': {}",
                            attempt + 1,
                            max_extract_retries + 1,
                            archive_name,
                            e
                        );
                        if attempt < max_extract_retries {
                            // Clean up failed extraction dir and archive, then retry
                            let _ = tokio::fs::remove_dir_all(&extract_dest).await;
                            let _ = tokio::fs::remove_file(&download_path).await;
                            // Re-download
                            log::info!(
                                "Re-downloading '{}' after extraction failure (attempt {})",
                                archive_name,
                                attempt + 2
                            );
                            // Note: we don't hold the download semaphore here, but
                            // re-downloads after extraction failure are rare and brief
                            match task_downloader
                                .download_archive(&app_c, &archive, install_id, &db_c)
                                .await
                            {
                                Ok(_new_path) => continue, // retry extraction
                                Err(dl_err) => {
                                    let err_msg = format!(
                                        "Re-download failed for '{}': {}",
                                        archive_name, dl_err
                                    );
                                    log::error!("{}", err_msg);
                                    failures_c.lock().unwrap().push(err_msg.clone());
                                    emit_progress(
                                        &app_c,
                                        &WjInstallProgressEvent::ExtractionArchiveFailed {
                                            name: archive_name.clone(),
                                            error: err_msg,
                                        },
                                    );
                                    return;
                                }
                            }
                        } else {
                            let err_msg =
                                format!("Failed to extract '{}': {}", archive_name, e);
                            failures_c.lock().unwrap().push(err_msg.clone());
                            emit_progress(
                                &app_c,
                                &WjInstallProgressEvent::ExtractionArchiveFailed {
                                    name: archive_name.clone(),
                                    error: err_msg,
                                },
                            );
                            return;
                        }
                    }
                    Err(e) => {
                        let err_msg = format!(
                            "Extraction task panicked for '{}': {}",
                            archive_name, e
                        );
                        log::error!("{}", err_msg);
                        failures_c.lock().unwrap().push(err_msg.clone());
                        emit_progress(
                            &app_c,
                            &WjInstallProgressEvent::ExtractionArchiveFailed {
                                name: archive_name.clone(),
                                error: err_msg,
                            },
                        );
                        return;
                    }
                }
            }
            } // end else (archive extraction)

            if extract_ok {
                let completed =
                    bytes_completed.fetch_add(archive_size, Ordering::Relaxed) + archive_size;
                emit_progress(
                    &app_c,
                    &WjInstallProgressEvent::ExtractionProgress {
                        name: archive_name.clone(),
                        index,
                        total,
                        bytes_completed: completed,
                        total_bytes,
                    },
                );
                emit_progress(
                    &app_c,
                    &WjInstallProgressEvent::ExtractionArchiveCompleted {
                        name: archive_name.clone(),
                        index,
                    },
                );
                log::info!(
                    "Downloaded + extracted archive {}/{}: {}",
                    index + 1,
                    total,
                    archive_name
                );

                // Write checkpoint (download + extraction both succeeded)
                let now_secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let _ = tokio::fs::write(&checkpoint_file, format!("{now_secs}")).await;

                extracted_dirs_c
                    .lock()
                    .unwrap()
                    .insert(hash_str, extract_dest);
            }
        });

        handles.push(handle);
    }

    // Wait for all tasks to complete
    futures::future::join_all(handles).await;

    // Collect results from shared state
    let archive_download_paths = Arc::try_unwrap(archive_download_paths)
        .map(|m| m.into_inner().unwrap())
        .unwrap_or_else(|arc| arc.lock().unwrap().clone());
    let extracted_dirs = Arc::try_unwrap(extracted_dirs)
        .map(|m| m.into_inner().unwrap())
        .unwrap_or_else(|arc| arc.lock().unwrap().clone());
    let failure_list = Arc::try_unwrap(download_failures)
        .map(|m| m.into_inner().unwrap())
        .unwrap_or_else(|arc| arc.lock().unwrap().clone());

    let download_succeeded = archive_download_paths.len();
    let download_failed = failure_list.len();
    let download_skipped = download_skipped_count.load(Ordering::Relaxed) as usize;

    // Emit download+extraction phase completion summary
    emit_progress(
        app,
        &WjInstallProgressEvent::DownloadPhaseCompleted {
            total: total_archives,
            succeeded: download_succeeded,
            failed: download_failed,
            skipped: download_skipped,
            failures: if failure_list.len() > 20 {
                let mut truncated = failure_list[..20].to_vec();
                truncated.push(format!(
                    "... and {} more failures",
                    failure_list.len() - 20
                ));
                truncated
            } else {
                failure_list.clone()
            },
        },
    );

    // Emit extraction summary events for frontend progress tracking
    let extraction_count = extracted_dirs.len();
    let total_extracted_bytes: u64 = extracted_dirs
        .keys()
        .filter_map(|hash| archive_size_map.get(hash))
        .sum();
    emit_progress(
        app,
        &WjInstallProgressEvent::ExtractionStarted {
            total: extraction_count,
            total_bytes: total_extracted_bytes,
        },
    );
    emit_progress(
        app,
        &WjInstallProgressEvent::ExtractionProgress {
            name: String::new(),
            index: extraction_count,
            total: extraction_count,
            bytes_completed: total_extracted_bytes,
            total_bytes: total_extracted_bytes,
        },
    );

    log::info!(
        "Download+extract phase complete: {} downloaded, {} extracted, {} failed, {} skipped (of {} total)",
        download_succeeded, extraction_count, download_failed, download_skipped, total_archives
    );

    // If ALL downloads failed, fail the install early
    if download_succeeded == 0 && total_archives > 0 {
        let reason = format!(
            "All {} archives failed to download. Breakdown: {}",
            total_archives,
            summarize_failures(&failure_list),
        );
        db.update_wj_install_status(install_id, "failed", Some(&reason))
            .map_err(|e| WjInstallError::Database(e.to_string()))?;
        emit_progress(
            app,
            &WjInstallProgressEvent::InstallFailed {
                error: reason.clone(),
            },
        );
        return Err(WjInstallError::Download(reason));
    }

    // If a significant portion failed, add a prominent warning with categorized breakdown
    if download_failed > 0 {
        let pct = (download_failed as f64 / total_archives as f64 * 100.0) as u32;
        warnings.push(format!(
            "{} of {} archives failed ({}%) — files from these archives will be missing. Breakdown: {}",
            download_failed,
            total_archives,
            pct,
            summarize_failures(&failure_list),
        ));
        for failure in &failure_list {
            warnings.push(format!("  Failure: {}", failure));
        }
    }

    db.update_wj_install_archive_progress(install_id, archive_download_paths.len() as i64)
        .map_err(|e| WjInstallError::Database(e.to_string()))?;

    log_phase_metrics(app, "download+extraction", phase_start);

    // Check cancellation
    if is_cancelled(&cancel_token) {
        mark_cancelled(db, install_id);
        emit_progress(app, &WjInstallProgressEvent::InstallCancelled);
        return Err(WjInstallError::Cancelled);
    }

    // -----------------------------------------------------------------------
    // Step 6: Directive processing phase
    // -----------------------------------------------------------------------
    let phase_start = Instant::now();
    db.update_wj_install_status(install_id, "processing", None)
        .map_err(|e| WjInstallError::Database(e.to_string()))?;

    let total_directive_bytes: u64 = modlist
        .directives
        .iter()
        .map(|d| d.size().max(0) as u64)
        .sum();

    emit_progress(
        app,
        &WjInstallProgressEvent::DirectivePhaseStarted {
            total: total_directives,
            total_bytes: total_directive_bytes,
        },
    );

    // Determine game data directory for directive path substitution
    let game_dir = {
        let bottles = crate::bottles::detect_bottles();
        let bottle = bottles.iter().find(|b| b.name == bottle_name);
        bottle
            .and_then(|b| {
                crate::games::with_plugin(game_id, |plugin| plugin.detect(b).map(|g| g.data_dir))
                    .flatten()
            })
            .unwrap_or_else(|| install_dir.join("Data"))
    };

    let install_id_str = format!("{}::{}", modlist.name, modlist.version);
    let processor = DirectiveProcessor::new(
        wabbajack_path.to_path_buf(),
        extracted_dirs.clone(),
        install_dir.to_path_buf(),
        game_dir.clone(),
    )
    .with_resume_tracking(db.clone(), install_id_str);

    let app_clone = app.clone();
    let directive_count = modlist.directives.len();
    // Use a moderate interval but with a time-based fallback so the UI
    // always gets updates at least every 2 seconds.
    let progress_interval = match directive_count {
        0..=100 => 1,
        101..=500 => 5,
        501..=2000 => 15,
        _ => 30,
    };
    let last_emit_time = std::sync::Mutex::new(std::time::Instant::now());
    let directive_result = processor
        .process_all(
            &modlist.directives,
            &|current, total, phase, bytes_processed, total_bytes, current_file| {
                let should_emit = current == 1
                    || current == total
                    || current % progress_interval == 0
                    || {
                        // Time-based fallback: emit at least every 2 seconds
                        let mut last = last_emit_time.lock().unwrap();
                        if last.elapsed() >= std::time::Duration::from_secs(2) {
                            *last = std::time::Instant::now();
                            true
                        } else {
                            false
                        }
                    };
                if should_emit {
                    emit_progress(
                        &app_clone,
                        &WjInstallProgressEvent::DirectiveProgress {
                            current,
                            total,
                            directive_type: phase.to_string(),
                            bytes_processed,
                            total_bytes,
                            current_file: current_file.to_string(),
                        },
                    );
                }
            },
        )
        .map_err(|e| {
            WjInstallError::Directive(format!(
                "Directive processing failed for '{}' ({} directives): {}",
                modlist.name, directive_count, e
            ))
        })?;

    let processed_count = directive_result.total_processed;

    // Merge directive warnings and errors
    for w in &directive_result.warnings {
        warnings.push(format!("Directive: {}", w));
    }
    for e in &directive_result.errors {
        warnings.push(format!("Directive error: {}", e));
    }

    db.update_wj_install_directive_progress(install_id, processed_count as i64)
        .map_err(|e| WjInstallError::Database(e.to_string()))?;

    log::info!(
        "Directive processing complete: {} processed, {} skipped, {} errors",
        directive_result.total_processed,
        directive_result.total_skipped,
        directive_result.errors.len(),
    );

    log_phase_metrics(app, "directives", phase_start);

    // Check cancellation
    if is_cancelled(&cancel_token) {
        mark_cancelled(db, install_id);
        emit_progress(app, &WjInstallProgressEvent::InstallCancelled);
        return Err(WjInstallError::Cancelled);
    }

    // -----------------------------------------------------------------------
    // Step 7: Deploy phase
    // -----------------------------------------------------------------------
    let phase_start = Instant::now();
    db.update_wj_install_status(install_id, "deploying", None)
        .map_err(|e| WjInstallError::Database(e.to_string()))?;

    // Collect all files produced by directive processing for deployment
    let mut deploy_files: Vec<String> = Vec::new();
    for entry in walkdir::WalkDir::new(install_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        if let Ok(rel) = entry.path().strip_prefix(install_dir) {
            let rel_str = rel.to_string_lossy().to_string();
            // Skip temp extraction dirs and hidden files
            if !rel_str.starts_with(".wj_") && !rel_str.starts_with('.') {
                deploy_files.push(rel_str);
            }
        }
    }

    let total_deploy_bytes: u64 = deploy_files
        .iter()
        .filter_map(|f| std::fs::metadata(install_dir.join(f)).map(|m| m.len()).ok())
        .sum();

    emit_progress(
        app,
        &WjInstallProgressEvent::DeployStarted {
            total: deploy_files.len(),
            total_bytes: total_deploy_bytes,
            modlist_name: modlist.name.clone(),
        },
    );

    let files_deployed = if deploy_files.is_empty() {
        0usize
    } else {
        // Remove any existing mod records for this modlist (from prior interrupted installs)
        // to prevent duplicates showing up in the mod list.
        if let Ok(existing_ids) = db.find_mods_by_name(game_id, bottle_name, &modlist.name) {
            for old_id in existing_ids {
                log::info!("Removing stale mod record id={} for '{}' (prior interrupted install)", old_id, modlist.name);
                let _ = db.remove_mod(old_id);
            }
        }

        // Create a mod record for the modlist output
        let mod_id = db
            .add_mod(
                game_id,
                bottle_name,
                None, // no nexus_mod_id
                &modlist.name,
                &modlist.version,
                &format!("{}.wabbajack", modlist.name),
                &deploy_files,
            )
            .map_err(|e| WjInstallError::Database(e.to_string()))?;

        // Tag this mod as belonging to the WJ modlist collection
        let wj_collection = format!("wj:{}", modlist.name);
        let _ = db.set_collection_name(mod_id, &wj_collection);

        match crate::deployer::deploy_mod_atomic(
            db,
            game_id,
            bottle_name,
            mod_id,
            install_dir,
            &game_dir,
            &deploy_files,
            &game_dir,
        ) {
            Ok(deploy_result) => {
                emit_progress(
                    app,
                    &WjInstallProgressEvent::DeployProgress {
                        current: deploy_result.deployed_count,
                        total: deploy_files.len(),
                        bytes_deployed: total_deploy_bytes,
                        total_bytes: total_deploy_bytes,
                        modlist_name: modlist.name.clone(),
                    },
                );

                log::info!(
                    "Deployed {} files ({} skipped, fallback: {})",
                    deploy_result.deployed_count,
                    deploy_result.skipped_count,
                    deploy_result.fallback_used,
                );

                deploy_result.deployed_count
            }
            Err(e) => {
                // Clean up orphaned mod record (deploy_mod_atomic already rolled back files)
                let _ = db.remove_mod(mod_id);
                let _ = db.update_wj_install_status(
                    install_id,
                    "failed",
                    Some(&format!("Deployment failed (rolled back): {}", e)),
                );
                return Err(WjInstallError::Deployment(format!(
                    "deploy_mod_atomic for modlist '{}' ({} files): {}",
                    modlist.name,
                    deploy_files.len(),
                    e
                )));
            }
        }
    };

    log_phase_metrics(app, "deployment", phase_start);

    // -----------------------------------------------------------------------
    // Step 7b: Import MO2 profiles (if present in the modlist)
    // -----------------------------------------------------------------------
    let imported_profiles = crate::profiles::import_mo2_profiles(
        db,
        install_dir,
        game_id,
        bottle_name,
        &modlist.name,
    );
    if !imported_profiles.is_empty() {
        log::info!(
            "Imported {} MO2 profile(s): {}",
            imported_profiles.len(),
            imported_profiles.join(", ")
        );
        for name in &imported_profiles {
            warnings.push(format!("Imported MO2 profile: {}", name));
        }
    }

    // -----------------------------------------------------------------------
    // Step 8: Mark completed in DB
    // -----------------------------------------------------------------------
    let elapsed = start_time.elapsed().as_secs_f64();

    db.update_wj_install_status(install_id, "completed", None)
        .map_err(|e| WjInstallError::Database(e.to_string()))?;

    // -----------------------------------------------------------------------
    // Step 9: Build and return result
    // -----------------------------------------------------------------------
    let result = WjInstallResult {
        install_id,
        status: "completed".to_string(),
        total_archives,
        total_directives,
        files_deployed,
        elapsed_secs: elapsed,
        warnings,
    };

    emit_progress(
        app,
        &WjInstallProgressEvent::InstallCompleted {
            result: result.clone(),
        },
    );

    log::info!(
        "Wabbajack install completed: id={}, archives={}, directives={}, elapsed={:.1}s",
        install_id,
        total_archives,
        total_directives,
        elapsed
    );

    // Clean up extraction temp directory
    if extraction_temp_base.exists() {
        if let Err(e) = std::fs::remove_dir_all(&extraction_temp_base) {
            log::warn!(
                "Failed to clean up extraction temp dir {:?}: {}",
                extraction_temp_base,
                e
            );
        }
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Tauri command: Run pre-flight check for a Wabbajack modlist.
///
/// Returns a WjPreflightReport with disk space, archive counts, and any
/// issues that would prevent installation.
#[tauri::command]
pub(crate) async fn wabbajack_preflight_cmd(
    app: AppHandle,
    state: tauri::State<'_, crate::AppState>,
    wabbajack_path: String,
    game_id: String,
    bottle_name: String,
    install_dir: String,
    download_dir: String,
) -> Result<WjPreflightReport, String> {
    let db = state.db.clone();
    preflight_check(
        &app,
        &db,
        Path::new(&wabbajack_path),
        &game_id,
        &bottle_name,
        Path::new(&install_dir),
        Path::new(&download_dir),
    )
    .await
    .map_err(|e| e.to_string())
}

/// Tauri command: Start a Wabbajack modlist installation.
///
/// Spawns the installation in a background tokio task and returns the
/// install_id immediately. Progress is emitted via `wj-install-progress`
/// events on the AppHandle.
#[tauri::command]
pub(crate) async fn install_wabbajack_modlist_cmd(
    app: AppHandle,
    state: tauri::State<'_, crate::AppState>,
    wabbajack_path: String,
    game_id: String,
    bottle_name: String,
    install_dir: String,
    download_dir: String,
) -> Result<i64, String> {
    let db = state.db.clone();
    let wj_path = PathBuf::from(&wabbajack_path);
    let inst_dir = PathBuf::from(&install_dir);
    let dl_dir = PathBuf::from(&download_dir);

    // Create a cancel token for this install
    let cancel_token = Arc::new(AtomicBool::new(false));

    // Create DB record first to get the install_id
    // We do a quick parse to get modlist metadata
    let modlist = parse_wabbajack_file_typed(&wj_path)
        .map_err(|e| format!("Failed to parse modlist: {}", e))?;

    let install_id = db
        .create_wj_install(
            &modlist.name,
            &modlist.version,
            modlist.game_type,
            &install_dir,
            modlist.archives.len(),
            modlist.directives.len(),
        )
        .map_err(|e| format!("Failed to create install record: {}", e))?;

    // Store the cancel token so it can be retrieved by cancel_wabbajack_install
    state
        .wj_cancel_tokens
        .lock()
        .unwrap()
        .insert(install_id, cancel_token.clone());

    // Spawn the installation task
    let app_clone = app.clone();
    let cancel_clone = cancel_token;
    tokio::spawn(async move {
        let result = install_wabbajack_modlist(
            &app_clone,
            &db,
            &wj_path,
            &game_id,
            &bottle_name,
            &inst_dir,
            &dl_dir,
            cancel_clone,
        )
        .await;

        match result {
            Ok(res) => {
                log::info!(
                    "Wabbajack install {} completed successfully",
                    res.install_id
                );
            }
            Err(WjInstallError::Cancelled) => {
                log::info!("Wabbajack install {} was cancelled", install_id);
                emit_progress(&app_clone, &WjInstallProgressEvent::InstallCancelled);
            }
            Err(e) => {
                log::error!("Wabbajack install {} failed: {}", install_id, e);
                emit_progress(
                    &app_clone,
                    &WjInstallProgressEvent::InstallFailed {
                        error: e.to_string(),
                    },
                );
            }
        }

        // Clean up the cancel token now that the install is finished
        let st = app_clone.state::<crate::AppState>();
        st.wj_cancel_tokens.lock().unwrap().remove(&install_id);
        log::info!(
            "Cleaned up cancel token for wabbajack install {}",
            install_id
        );
    });

    Ok(install_id)
}

/// Tauri command: Cancel a running Wabbajack installation.
///
/// Sets the cancel token for the given install_id, which will be picked up
/// by the running install task at the next cancellation check point.
#[tauri::command]
pub(crate) async fn cancel_wabbajack_install(
    state: tauri::State<'_, crate::AppState>,
    install_id: i64,
) -> Result<(), String> {
    let tokens = state.wj_cancel_tokens.lock().unwrap();
    if let Some(token) = tokens.get(&install_id) {
        token.store(true, Ordering::Relaxed);
        log::info!(
            "Cancellation requested for wabbajack install {}",
            install_id
        );
        Ok(())
    } else {
        Err(format!("No active install found with id {}", install_id))
    }
}

/// Tauri command: Resume a paused/failed Wabbajack installation.
///
/// Resets the cancel token and updates DB status back to "downloading" so
/// the install can be re-triggered by the frontend.
#[tauri::command]
pub(crate) async fn resume_wabbajack_install(
    state: tauri::State<'_, crate::AppState>,
    install_id: i64,
) -> Result<(), String> {
    // Clear the cancellation flag if present
    {
        let tokens = state.wj_cancel_tokens.lock().unwrap();
        if let Some(token) = tokens.get(&install_id) {
            token.store(false, Ordering::Relaxed);
        }
    }

    // Update DB status back to "pending" so it can be restarted
    state
        .db
        .update_wj_install_status(install_id, "pending", None)
        .map_err(|e| format!("Database error: {}", e))?;

    log::info!("Resume requested for wabbajack install {}", install_id);
    Ok(())
}

/// Tauri command: Clean up the cancel token for a finished Wabbajack installation.
///
/// The backend auto-cleans tokens after install completion, but this command
/// provides an explicit cleanup path if the frontend detects a stale token.
#[tauri::command]
pub(crate) async fn cleanup_wabbajack_install(
    state: tauri::State<'_, crate::AppState>,
    install_id: i64,
) -> Result<(), String> {
    let removed = state
        .wj_cancel_tokens
        .lock()
        .unwrap()
        .remove(&install_id)
        .is_some();
    if removed {
        log::info!(
            "Explicitly cleaned up cancel token for install {}",
            install_id
        );
    }
    Ok(())
}

/// Tauri command: Get the current progress/status of a Wabbajack installation.
///
/// Returns a JSON object with status, progress counters, and any error message.
#[tauri::command]
pub(crate) async fn get_wabbajack_install_status(
    state: tauri::State<'_, crate::AppState>,
    install_id: i64,
) -> Result<serde_json::Value, String> {
    let db = &state.db;

    let status_row = db
        .get_wj_install_status(install_id)
        .map_err(|e| format!("Database error: {}", e))?
        .ok_or_else(|| format!("Install {} not found", install_id))?;

    let (
        status,
        total_archives,
        completed_archives,
        total_directives,
        completed_directives,
        error_message,
    ) = status_row;

    Ok(serde_json::json!({
        "install_id": install_id,
        "status": status,
        "total_archives": total_archives,
        "completed_archives": completed_archives,
        "total_directives": total_directives,
        "completed_directives": completed_directives,
        "error_message": error_message,
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_available_disk_space() {
        // Should return non-zero for root
        let space = get_available_disk_space(Path::new("/"));
        assert!(space > 0, "Expected non-zero disk space for /");
    }

    #[test]
    fn test_get_available_disk_space_nonexistent() {
        // Should walk up to nearest existing ancestor
        let space = get_available_disk_space(Path::new("/tmp/nonexistent/deeply/nested/path"));
        assert!(
            space > 0,
            "Expected non-zero disk space for existing ancestor"
        );
    }

    #[test]
    fn test_is_cancelled() {
        let token = AtomicBool::new(false);
        assert!(!is_cancelled(&token));

        token.store(true, Ordering::Relaxed);
        assert!(is_cancelled(&token));
    }

    #[test]
    fn test_wj_install_result_serializes() {
        let result = WjInstallResult {
            install_id: 42,
            status: "completed".to_string(),
            total_archives: 100,
            total_directives: 5000,
            files_deployed: 4500,
            elapsed_secs: 123.45,
            warnings: vec!["test warning".to_string()],
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"install_id\":42"));
        assert!(json.contains("\"status\":\"completed\""));
    }

    #[test]
    fn test_preflight_report_serializes() {
        let report = WjPreflightReport {
            can_proceed: true,
            issues: vec![WjPreflightIssue {
                severity: "warning".to_string(),
                message: "Test warning".to_string(),
            }],
            total_download_size: 1024 * 1024 * 1024,
            total_archives: 50,
            total_directives: 2000,
            cached_archives: 10,
            disk_space_available: 100 * 1024 * 1024 * 1024,
            disk_space_required: 2 * 1024 * 1024 * 1024,
            nexus_archives: 30,
            is_nexus_premium: false,
            manual_downloads: 5,
            required_game_version: Some("1.6.1170.0".to_string()),
            detected_game_version: Some("1.6.1170".to_string()),
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"can_proceed\":true"));
        assert!(json.contains("\"required_game_version\""));
        assert!(json.contains("\"total_archives\":50"));
    }

    #[test]
    fn test_progress_event_serializes_tagged() {
        let event = WjInstallProgressEvent::DownloadStarted {
            name: "test.zip".to_string(),
            index: 0,
            total: 10,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"DownloadStarted\""));
        assert!(json.contains("\"name\":\"test.zip\""));

        let event2 = WjInstallProgressEvent::InstallCancelled;
        let json2 = serde_json::to_string(&event2).unwrap();
        assert!(json2.contains("\"type\":\"InstallCancelled\""));
    }

    #[test]
    fn classify_failure_buckets() {
        assert_eq!(classify_failure("HTTP 404 Not Found"), FailureKind::NotFound);
        assert_eq!(classify_failure("HTTP 403 Forbidden"), FailureKind::Unauthorized);
        assert_eq!(classify_failure("premium required for this file"), FailureKind::Unauthorized);
        assert_eq!(classify_failure("HTTP 429 Too Many Requests"), FailureKind::RateLimited);
        assert_eq!(classify_failure("xxhash mismatch after download"), FailureKind::HashMismatch);
        assert_eq!(classify_failure("ENOSPC: no space left on device"), FailureKind::DiskFull);
        assert_eq!(classify_failure("HTTP 502 Bad Gateway"), FailureKind::ServerError);
        assert_eq!(classify_failure("connection timed out"), FailureKind::Network);
        assert_eq!(classify_failure("something entirely weird"), FailureKind::Unknown);
    }

    #[test]
    fn summarize_failures_breakdown() {
        let out = summarize_failures(&[
            "HTTP 404 Not Found".to_string(),
            "HTTP 404 Not Found".to_string(),
            "connection timed out".to_string(),
        ]);
        assert!(out.contains("2 × Removed (404)"));
        assert!(out.contains("1 × Network / timeout"));
        assert!(out.contains("First: HTTP 404"));
    }

    #[test]
    fn summarize_failures_empty() {
        let out = summarize_failures(&[]);
        assert!(out.contains("no error details"));
    }
}
