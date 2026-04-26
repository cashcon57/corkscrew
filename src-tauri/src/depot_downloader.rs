//! DepotDownloader integration for automated Steam game version rollback.
//!
//! Downloads and manages the [DepotDownloader](https://github.com/SteamRE/DepotDownloader)
//! binary (GPL-2.0 by SteamRE) to enable downloading specific game versions
//! from Steam's content delivery network.
//!
//! ## Security
//! - Steam credentials are NEVER stored by Corkscrew — they are piped
//!   directly to DepotDownloader's stdin and discarded immediately.
//! - DepotDownloader manages its own session tokens in its config directory.
//! - All communication happens directly between DepotDownloader and Steam's
//!   servers — Corkscrew never sees or proxies the auth tokens.
//!
//! ## Credits
//! DepotDownloader is developed by [SteamRE](https://github.com/SteamRE/DepotDownloader)
//! and licensed under GPL-2.0. Corkscrew downloads it on demand and runs it
//! as a subprocess — it is not bundled or redistributed.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use log::{debug, info};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::config;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum DDError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("DepotDownloader not installed")]
    NotInstalled,
    #[error("Steam authentication required")]
    AuthRequired,
    #[error("Steam Guard code required")]
    SteamGuardRequired,
    #[error("Steam Guard mobile confirmation required")]
    SteamGuardMobile,
    #[error("Authentication failed: {0}")]
    AuthFailed(String),
    #[error("Download failed: {0}")]
    DownloadFailed(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, DDError>;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A depot manifest entry from DepotDownloader's manifest listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepotManifest {
    pub manifest_id: String,
    pub date: String,
    pub depot_id: String,
}

/// Status of a downgrade operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DowngradeProgress {
    pub phase: String,
    pub detail: String,
    pub percent: Option<f64>,
}

/// Steam auth state for DD sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthState {
    /// No auth needed — DD has a valid saved session
    Ready,
    /// Need username + password
    NeedCredentials,
    /// Need Steam Guard / 2FA code
    NeedSteamGuard,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const DD_GITHUB_REPO: &str = "SteamRE/DepotDownloader";
const DD_DIR_NAME: &str = "depot_downloader";
const DD_BINARY_NAME: &str = "DepotDownloader";

/// Get the platform-specific asset name for the current architecture.
fn dd_asset_name() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        #[cfg(target_arch = "aarch64")]
        return "DepotDownloader-macos-arm64.zip";
        #[cfg(target_arch = "x86_64")]
        return "DepotDownloader-macos-x64.zip";
    }
    #[cfg(target_os = "linux")]
    {
        #[cfg(target_arch = "aarch64")]
        return "DepotDownloader-linux-arm64.zip";
        #[cfg(target_arch = "x86_64")]
        return "DepotDownloader-linux-x64.zip";
    }
    #[cfg(target_os = "windows")]
    {
        return "DepotDownloader-windows-x64.zip";
    }
}

// ---------------------------------------------------------------------------
// Binary management
// ---------------------------------------------------------------------------

/// Get the directory where DD is installed.
fn dd_install_dir() -> PathBuf {
    config::cache_dir().join(DD_DIR_NAME)
}

/// Get the path to the DD binary.
fn dd_binary_path() -> PathBuf {
    dd_install_dir().join(DD_BINARY_NAME)
}

/// Check if DD is installed and executable.
pub fn is_installed() -> bool {
    let path = dd_binary_path();
    path.exists() && path.is_file()
}

/// Verify the .NET 8+ runtime is available on Linux.
///
/// The Linux DepotDownloader asset is framework-dependent and silently fails
/// with a non-zero exit code if `dotnet` is missing or the installed runtime
/// is too old. macOS bundles ship with the runtime baked in, so we skip the
/// probe there. Windows is not a build target.
#[cfg(target_os = "linux")]
async fn check_dotnet_runtime() -> Result<()> {
    let output = Command::new("dotnet")
        .arg("--list-runtimes")
        .output()
        .await;

    let install_hint = "DepotDownloader requires .NET 8+. Install: \
        `sudo pacman -S dotnet-runtime` (Arch/CachyOS), \
        `sudo apt install dotnet-runtime-8.0` (Debian/Ubuntu), \
        `sudo dnf install dotnet-runtime-8.0` (Fedora). \
        On Steam Deck, install via Distrobox.";

    let output = match output {
        Ok(o) => o,
        Err(_) => return Err(DDError::Other(install_hint.into())),
    };

    if !output.status.success() {
        return Err(DDError::Other(install_hint.into()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Lines look like: "Microsoft.NETCore.App 8.0.10 [/usr/share/dotnet/...]"
    let mut has_supported = false;
    for line in stdout.lines() {
        if !line.starts_with("Microsoft.NETCore.App ") {
            continue;
        }
        if let Some(version) = line.split_whitespace().nth(1) {
            if let Some(major) = version.split('.').next() {
                if let Ok(n) = major.parse::<u32>() {
                    if n >= 8 {
                        has_supported = true;
                        break;
                    }
                }
            }
        }
    }

    if !has_supported {
        return Err(DDError::Other(install_hint.into()));
    }

    Ok(())
}

#[cfg(not(target_os = "linux"))]
async fn check_dotnet_runtime() -> Result<()> {
    Ok(())
}

/// Get the installed DD version (from .version file).
pub fn installed_version() -> Option<String> {
    let version_file = dd_install_dir().join(".version");
    std::fs::read_to_string(version_file).ok().map(|s| s.trim().to_string())
}

/// Download and install DepotDownloader from GitHub releases.
pub async fn install() -> Result<String> {
    let client = reqwest::Client::builder()
        .user_agent(format!("Corkscrew/{}", env!("CARGO_PKG_VERSION")))
        .build()?;

    // Get latest release info
    let url = format!("https://api.github.com/repos/{}/releases/latest", DD_GITHUB_REPO);
    let release: serde_json::Value = client.get(&url).send().await?.json().await?;

    let tag = release["tag_name"]
        .as_str()
        .ok_or_else(|| DDError::Other("No tag_name in release".into()))?
        .to_string();

    let asset_name = dd_asset_name();
    let asset = release["assets"]
        .as_array()
        .ok_or_else(|| DDError::Other("No assets in release".into()))?
        .iter()
        .find(|a| a["name"].as_str() == Some(asset_name))
        .ok_or_else(|| DDError::Other(format!("Asset {} not found in release", asset_name)))?;

    let download_url = asset["browser_download_url"]
        .as_str()
        .ok_or_else(|| DDError::Other("No download URL for asset".into()))?;

    let size = asset["size"].as_u64().unwrap_or(0);
    info!(
        "Downloading DepotDownloader {} ({} bytes) from {}",
        tag, size, download_url
    );

    // Download
    let bytes = client.get(download_url).send().await?.bytes().await?;

    // Extract to install dir
    let install_dir = dd_install_dir();
    if install_dir.exists() {
        std::fs::remove_dir_all(&install_dir)?;
    }
    std::fs::create_dir_all(&install_dir)?;

    // Extract ZIP
    let cursor = std::io::Cursor::new(&bytes);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| DDError::Other(format!("Failed to open ZIP: {}", e)))?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| DDError::Other(format!("ZIP entry error: {}", e)))?;

        let name = file.name().to_string();
        if name.ends_with('/') {
            continue; // skip directories
        }

        // Prevent ZIP path traversal (Zip Slip)
        if name.contains("..") || name.starts_with('/') || name.starts_with('\\') {
            return Err(DDError::Other(format!(
                "ZIP contains unsafe path: {}",
                name
            )));
        }

        let dest = install_dir.join(&name);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut out = std::fs::File::create(&dest)?;
        std::io::copy(&mut file, &mut out)?;
    }

    // Make binary executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let binary = dd_binary_path();
        if binary.exists() {
            let mut perms = std::fs::metadata(&binary)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&binary, perms)?;
        }
    }

    // Write version marker
    std::fs::write(install_dir.join(".version"), &tag)?;

    info!("DepotDownloader {} installed to {}", tag, install_dir.display());
    Ok(tag)
}

/// Ensure DD is installed and up-to-date. Installs or updates as needed.
/// Returns the installed version string.
pub async fn ensure_up_to_date() -> Result<String> {
    let client = reqwest::Client::builder()
        .user_agent(format!("Corkscrew/{}", env!("CARGO_PKG_VERSION")))
        .build()?;

    // Get latest release tag
    let url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        DD_GITHUB_REPO
    );
    let release: serde_json::Value = client.get(&url).send().await?.json().await?;
    let latest_tag = release["tag_name"]
        .as_str()
        .ok_or_else(|| DDError::Other("No tag_name in release".into()))?
        .to_string();

    // Compare with installed version
    if is_installed() {
        if let Some(ref current) = installed_version() {
            if current == &latest_tag {
                info!("DepotDownloader {} is up-to-date", current);
                return Ok(latest_tag);
            }
            info!(
                "DepotDownloader update available: {} → {}",
                current, latest_tag
            );
        }
    }

    // Install (or update) to latest
    install().await
}

// ---------------------------------------------------------------------------
// Auth check
// ---------------------------------------------------------------------------

/// Check if DD has a saved session (no auth needed).
pub fn check_auth_state() -> AuthState {
    // Can't reliably detect DD's credential storage location (varies by OS,
    // .NET version, and DD version — may use keychain, DPAPI, or files).
    // Default to NeedCredentials; callers should use check_auth_state_live()
    // for an accurate check when possible.
    if !is_installed() {
        return AuthState::NeedCredentials;
    }
    AuthState::NeedCredentials
}

/// Actually run DD to check if a saved session is valid.
/// This is slower (spawns a process) but reliable across all platforms.
/// DD stores credentials in a platform-specific way (keychain, DPAPI, etc.)
/// that we can't inspect directly, so we run a harmless operation to verify.
pub async fn check_auth_state_live(username: &str) -> AuthState {
    if !is_installed() {
        return AuthState::NeedCredentials;
    }
    // If .NET is missing the binary will exit with a runtime-loader error
    // that we'd misinterpret as "needs credentials". Bail early instead.
    if check_dotnet_runtime().await.is_err() {
        return AuthState::NeedCredentials;
    }

    let binary = dd_binary_path();
    let mut cmd = Command::new(&binary);
    cmd.arg("-app").arg("10")
        .arg("-depot").arg("11")
        .arg("-manifest-only")
        .arg("-username").arg(username)
        .arg("-remember-password");
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        cmd.output(),
    ).await;

    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.contains("Logged in")
                || (stdout.contains("Logging '") && stdout.contains("Done!"))
                || stdout.contains("licenses for account")
                || stdout.contains("manifest")
            {
                AuthState::Ready
            } else {
                AuthState::NeedCredentials
            }
        }
        _ => AuthState::NeedCredentials,
    }
}

// ---------------------------------------------------------------------------
// Core operations
// ---------------------------------------------------------------------------

/// List all available manifests for a depot.
/// Returns a list of (manifest_id, date) pairs, newest first.
pub async fn list_manifests(
    app_id: u32,
    depot_id: u32,
    username: Option<&str>,
    password: Option<&str>,
) -> Result<Vec<DepotManifest>> {
    if !is_installed() {
        return Err(DDError::NotInstalled);
    }
    check_dotnet_runtime().await?;

    let binary = dd_binary_path();
    let mut cmd = Command::new(&binary);
    cmd.arg("-app").arg(app_id.to_string())
        .arg("-depot").arg(depot_id.to_string())
        .arg("-manifest-only");

    // Only pass username — password is never passed as a CLI arg (visible in ps).
    // After initial authenticate() with -remember-password, DD reuses the saved session.
    if let Some(user) = username {
        cmd.arg("-username").arg(user);
    }
    // password parameter kept for API compatibility but intentionally unused
    let _ = password;

    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped());

    debug!("Running DepotDownloader for app={}, depot={}", app_id, depot_id);

    let output = cmd.output().await?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        if stderr.contains("InvalidPassword") || stderr.contains("InvalidLoginAuthCode") {
            return Err(DDError::AuthFailed(stderr.to_string()));
        }
        if stderr.contains("SteamGuard") || stdout.contains("Steam Guard") {
            return Err(DDError::SteamGuardRequired);
        }
        return Err(DDError::Other(format!(
            "DD exited with {}: {}",
            output.status,
            stderr
        )));
    }

    // Parse manifest list from stdout
    // DD outputs lines like:
    //   Manifest ID: 1234567890 | Date: 2024-01-15 12:00:00
    let mut manifests = Vec::new();
    for line in stdout.lines() {
        if let Some(manifest_id) = extract_manifest_line(line, depot_id) {
            manifests.push(manifest_id);
        }
    }

    info!(
        "Listed {} manifests for app {} depot {}",
        manifests.len(),
        app_id,
        depot_id
    );
    Ok(manifests)
}

/// Download a specific depot manifest.
/// Streams DD's stdout line by line and calls `progress_callback` with parsed progress.
pub async fn download_depot(
    app_id: u32,
    depot_id: u32,
    manifest_id: &str,
    output_dir: &Path,
    username: Option<&str>,
    password: Option<&str>,
    progress_callback: Option<&(dyn Fn(DowngradeProgress) + Send + Sync)>,
) -> Result<PathBuf> {
    if !is_installed() {
        return Err(DDError::NotInstalled);
    }
    check_dotnet_runtime().await?;

    let binary = dd_binary_path();
    let mut cmd = Command::new(&binary);
    cmd.arg("-app").arg(app_id.to_string())
        .arg("-depot").arg(depot_id.to_string())
        .arg("-manifest").arg(manifest_id)
        .arg("-dir").arg(output_dir.to_string_lossy().to_string())
        .arg("-validate"); // Resume interrupted downloads by skipping valid chunks

    if let Some(user) = username {
        cmd.arg("-username").arg(user);
    }
    let _ = password;

    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped());

    info!(
        "Downloading depot: app={} depot={} manifest={} → {}",
        app_id, depot_id, manifest_id, output_dir.display()
    );

    if let Some(cb) = progress_callback {
        cb(DowngradeProgress {
            phase: "downloading".into(),
            detail: "Connecting to Steam...".into(),
            percent: Some(0.0),
        });
    }

    let mut child = cmd.spawn()?;
    let stdout = child.stdout.take();

    // Stream stdout line by line for real-time progress
    if let Some(stdout) = stdout {
        use tokio::io::{AsyncBufReadExt, BufReader};
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();

        while let Ok(Some(line)) = lines.next_line().await {
            // Parse progress from DD output lines
            if let Some(cb) = progress_callback {
                if let Some(pct) = parse_dd_progress(&line) {
                    cb(DowngradeProgress {
                        phase: "downloading".into(),
                        detail: line.trim().to_string(),
                        percent: Some(pct),
                    });
                } else if line.contains("Downloading") || line.contains("Validating") || line.contains("Total") {
                    cb(DowngradeProgress {
                        phase: "downloading".into(),
                        detail: line.trim().to_string(),
                        percent: None,
                    });
                }
            }
        }
    }

    let status = child.wait().await?;

    if !status.success() {
        return Err(DDError::DownloadFailed(format!(
            "DepotDownloader exited with {}",
            status
        )));
    }

    if let Some(cb) = progress_callback {
        cb(DowngradeProgress {
            phase: "complete".into(),
            detail: "Depot download complete".into(),
            percent: Some(100.0),
        });
    }

    Ok(output_dir.to_path_buf())
}

/// Parse a percentage from DD output lines.
/// DD outputs lines like:
///   "  45.2%  depot 489833 (123 / 456)"
///   "Downloading depot ... 67.8%"
///   "Pre-allocating depot ... 12.3%"
fn parse_dd_progress(line: &str) -> Option<f64> {
    // Look for a pattern like "XX.X%" or "XX%"
    for word in line.split_whitespace() {
        let trimmed = word.trim_end_matches('%');
        if word.ends_with('%') {
            if let Ok(pct) = trimmed.parse::<f64>() {
                if (0.0..=100.0).contains(&pct) {
                    return Some(pct);
                }
            }
        }
    }
    None
}

/// Authenticate with Steam via DepotDownloader.
/// Pipes credentials via stdin (never as command-line args) so they don't
/// appear in process listings (`ps`). Uses `-remember-password` per DD FAQ
/// so subsequent operations reuse the saved session.
/// Clear saved Steam credentials. Removes the saved username from config
/// and deletes DD's internal session data by removing and reinstalling DD.
pub fn logout() -> Result<()> {
    // Clear saved username from Corkscrew config
    let _ = config::set_config_value("steam_username", "");
    info!("Cleared saved Steam username from config");

    // DD stores session data internally (platform-specific: keychain, DPAPI, etc.)
    // The most reliable way to clear it is to delete the DD binary directory
    // and let it be re-downloaded fresh on next use.
    let dd_dir = dd_install_dir();
    if dd_dir.exists() {
        std::fs::remove_dir_all(&dd_dir).map_err(|e| {
            DDError::Other(format!("Failed to remove DD directory: {}", e))
        })?;
        info!("Removed DepotDownloader directory to clear session data");
    }

    Ok(())
}

pub async fn authenticate(
    username: &str,
    password: &str,
    steam_guard_code: Option<&str>,
) -> Result<()> {
    if !is_installed() {
        return Err(DDError::NotInstalled);
    }
    check_dotnet_runtime().await?;

    let binary = dd_binary_path();
    let mut cmd = Command::new(&binary);
    // Use a harmless operation (list manifests for Steam itself) to trigger auth.
    // Omit -password so credentials don't appear in process args — pipe via stdin.
    cmd.arg("-app").arg("10") // Steam app ID (always accessible)
        .arg("-depot").arg("11")
        .arg("-manifest-only")
        .arg("-username").arg(username)
        .arg("-remember-password");

    // Always pipe stdin — DD prompts for password when -password is omitted
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn()?;

    // Pipe password via stdin. DD prompts immediately when -username is given
    // without -password. Then pipe Steam Guard code if needed.
    if let Some(mut stdin) = child.stdin.take() {
        // Write password (DD prompts for it right away)
        stdin.write_all(format!("{}\n", password).as_bytes()).await?;
        stdin.flush().await?;

        // If we have a Steam Guard code, write it after a delay for DD to process
        // the password and prompt for the code
        if let Some(code) = steam_guard_code {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            stdin.write_all(format!("{}\n", code).as_bytes()).await?;
            stdin.flush().await?;
        }
        drop(stdin); // Close stdin so DD can proceed
    }

    // Longer timeout when waiting for mobile confirmation (user needs to unlock phone)
    let timeout_secs = if steam_guard_code.is_none() { 60 } else { 30 };
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| DDError::Other("DepotDownloader authentication timed out".into()))?
    .map_err(DDError::Io)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    info!("DD auth exit_code={}, stdout={}, stderr={}", output.status, &stdout[..stdout.len().min(500)], &stderr[..stderr.len().min(500)]);

    if stderr.contains("InvalidPassword") || stderr.contains("InvalidLoginAuthCode") {
        return Err(DDError::AuthFailed(
            "Invalid username, password, or Steam Guard code".into(),
        ));
    }

    // Check for success BEFORE checking for Steam Guard text — DD may mention
    // Steam Guard in output even after a successful mobile confirmation.
    // DD exits non-zero if the app isn't owned, but auth still succeeded.
    // Match on actual login indicators from DD's output.
    if output.status.success()
        || stdout.contains("Logged in")
        || stdout.contains("Logging '") && stdout.contains("Done!")
        || stdout.contains("licenses for account")
        || stdout.contains("manifest")
    {
        info!("Steam authentication successful via DepotDownloader");
        // Save username so we can do live auth checks later
        let _ = config::set_config_value("steam_username", username);
        return Ok(());
    }

    // Distinguish mobile confirmation (push notification) vs code-based Steam Guard.
    // DD outputs "confirm" / "mobile" / "phone" for push notifications,
    // vs "Enter" / "code" for email/authenticator code prompts.
    if stderr.contains("SteamGuard") || stdout.contains("Steam Guard") {
        if steam_guard_code.is_none() {
            let combined = format!("{} {}", stdout, stderr).to_lowercase();
            if combined.contains("confirm") || combined.contains("mobile") || combined.contains("phone") {
                return Err(DDError::SteamGuardMobile);
            }
            return Err(DDError::SteamGuardRequired);
        }
    }

    Err(DDError::AuthFailed(format!(
        "Unexpected output: stdout={}, stderr={}",
        &stdout[..stdout.len().min(200)],
        &stderr[..stderr.len().min(200)]
    )))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse a manifest line from DD output.
fn extract_manifest_line(line: &str, depot_id: u32) -> Option<DepotManifest> {
    // DD manifest-only output format varies by version.
    // Common formats:
    //   "Manifest 1234567890123456789 (...)"
    //   "1234567890123456789 | 2024-01-15 12:00:00"
    let trimmed = line.trim();

    // Try "Manifest {id}" format
    if let Some(rest) = trimmed.strip_prefix("Manifest ") {
        let id = rest.split_whitespace().next()?;
        if id.chars().all(|c| c.is_ascii_digit()) && id.len() > 10 {
            return Some(DepotManifest {
                manifest_id: id.to_string(),
                date: String::new(),
                depot_id: depot_id.to_string(),
            });
        }
    }

    // Try "{id} | {date}" format
    if trimmed.contains('|') {
        let parts: Vec<&str> = trimmed.split('|').collect();
        if parts.len() >= 2 {
            let id = parts[0].trim();
            let date = parts[1].trim();
            if id.chars().all(|c| c.is_ascii_digit()) && id.len() > 10 {
                return Some(DepotManifest {
                    manifest_id: id.to_string(),
                    date: date.to_string(),
                    depot_id: depot_id.to_string(),
                });
            }
        }
    }

    // Try bare manifest ID on a line
    if trimmed.chars().all(|c| c.is_ascii_digit()) && trimmed.len() > 15 {
        return Some(DepotManifest {
            manifest_id: trimmed.to_string(),
            date: String::new(),
            depot_id: depot_id.to_string(),
        });
    }

    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_manifest_line_with_pipe() {
        let line = "5198899101792588169 | 2024-06-15 10:30:00";
        let result = extract_manifest_line(line, 990081);
        assert!(result.is_some());
        let m = result.unwrap();
        assert_eq!(m.manifest_id, "5198899101792588169");
        assert_eq!(m.date, "2024-06-15 10:30:00");
    }

    #[test]
    fn parse_manifest_line_prefix() {
        let line = "Manifest 5198899101792588169 (2024-06-15)";
        let result = extract_manifest_line(line, 990081);
        assert!(result.is_some());
        assert_eq!(result.unwrap().manifest_id, "5198899101792588169");
    }

    #[test]
    fn parse_manifest_line_bare() {
        let line = "5198899101792588169";
        let result = extract_manifest_line(line, 990081);
        assert!(result.is_some());
    }

    #[test]
    fn parse_manifest_line_junk() {
        assert!(extract_manifest_line("Loading manifest list...", 990081).is_none());
        assert!(extract_manifest_line("", 990081).is_none());
        assert!(extract_manifest_line("12345", 990081).is_none()); // too short
    }

    #[test]
    fn asset_name_is_valid() {
        let name = dd_asset_name();
        assert!(name.starts_with("DepotDownloader-"));
        assert!(name.ends_with(".zip"));
    }
}
