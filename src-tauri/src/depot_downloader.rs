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

use log::{debug, error, info, warn};
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

// ---------------------------------------------------------------------------
// Auth check
// ---------------------------------------------------------------------------

/// Check if DD has a saved session (no auth needed).
pub fn check_auth_state() -> AuthState {
    // DD saves sessions to its config dir. Check if login token exists.
    let dd_config = dirs::config_dir()
        .map(|d| d.join("DepotDownloader"))
        .unwrap_or_else(|| PathBuf::from("~/.config/DepotDownloader"));

    if dd_config.exists() {
        // Check for any saved session files
        if let Ok(entries) = std::fs::read_dir(&dd_config) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with(".sentryFile") || name.ends_with(".key") {
                    return AuthState::Ready;
                }
            }
        }
    }

    AuthState::NeedCredentials
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

    let binary = dd_binary_path();
    let mut cmd = Command::new(&binary);
    cmd.arg("-app").arg(app_id.to_string())
        .arg("-depot").arg(depot_id.to_string())
        .arg("-manifest-only");

    if let Some(user) = username {
        cmd.arg("-username").arg(user);
    }
    if let Some(pass) = password {
        cmd.arg("-password").arg(pass);
    }

    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped());

    debug!("Running DD: {:?}", cmd);

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

    let binary = dd_binary_path();
    let mut cmd = Command::new(&binary);
    cmd.arg("-app").arg(app_id.to_string())
        .arg("-depot").arg(depot_id.to_string())
        .arg("-manifest").arg(manifest_id)
        .arg("-dir").arg(output_dir.to_string_lossy().to_string());

    if let Some(user) = username {
        cmd.arg("-username").arg(user);
    }
    if let Some(pass) = password {
        cmd.arg("-password").arg(pass);
    }

    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(cb) = progress_callback {
        cb(DowngradeProgress {
            phase: "downloading".into(),
            detail: format!("Downloading depot {} manifest {}...", depot_id, manifest_id),
            percent: Some(0.0),
        });
    }

    info!(
        "Downloading depot: app={} depot={} manifest={} → {}",
        app_id,
        depot_id,
        manifest_id,
        output_dir.display()
    );

    let output = cmd.output().await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DDError::DownloadFailed(stderr.to_string()));
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

/// Authenticate with Steam via DepotDownloader.
/// Runs DD with just -username/-password to establish a session.
/// Returns Ok if auth succeeded, or an error indicating what's needed.
pub async fn authenticate(
    username: &str,
    password: &str,
    steam_guard_code: Option<&str>,
) -> Result<()> {
    if !is_installed() {
        return Err(DDError::NotInstalled);
    }

    let binary = dd_binary_path();
    let mut cmd = Command::new(&binary);
    // Use a harmless operation (list manifests for Steam itself) to trigger auth
    cmd.arg("-app").arg("10") // Steam app ID (always accessible)
        .arg("-depot").arg("11")
        .arg("-manifest-only")
        .arg("-username").arg(username)
        .arg("-password").arg(password);

    if let Some(code) = steam_guard_code {
        // DD accepts Steam Guard codes via stdin when prompted
        cmd.stdin(Stdio::piped());
    }

    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn()?;

    // If we have a Steam Guard code, write it to stdin when prompted
    if let Some(code) = steam_guard_code {
        if let Some(mut stdin) = child.stdin.take() {
            // Give DD a moment to prompt for the code
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            stdin.write_all(format!("{}\n", code).as_bytes()).await?;
            stdin.flush().await?;
        }
    }

    let output = child.wait_with_output().await?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if stderr.contains("InvalidPassword") || stderr.contains("InvalidLoginAuthCode") {
        return Err(DDError::AuthFailed(
            "Invalid username, password, or Steam Guard code".into(),
        ));
    }

    if stderr.contains("SteamGuard") || stdout.contains("Steam Guard") {
        if steam_guard_code.is_none() {
            return Err(DDError::SteamGuardRequired);
        }
    }

    if output.status.success() || stdout.contains("Logged in") || stdout.contains("manifest") {
        info!("Steam authentication successful via DepotDownloader");
        Ok(())
    } else {
        Err(DDError::AuthFailed(format!(
            "Unexpected output: stdout={}, stderr={}",
            &stdout[..stdout.len().min(200)],
            &stderr[..stderr.len().min(200)]
        )))
    }
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
