//! SKSE (Skyrim Script Extender) detection, download, and installation.
//!
//! Provides utilities for:
//! - Detecting whether SKSE is installed in a Skyrim SE game directory
//! - Downloading the latest SKSE build from the official site
//! - Installing SKSE files (loader, DLLs, Data/Scripts) into a game directory
//! - Persisting per-game SKSE preference in the Corkscrew config

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config;
use crate::installer;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum SkseError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Could not find SKSE download link on the official page")]
    DownloadLinkNotFound,

    #[error("Archive extraction failed: {0}")]
    Extraction(String),

    #[error("SKSE root directory not found in extracted archive")]
    SkseRootNotFound,

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, SkseError>;

// ---------------------------------------------------------------------------
// SkseStatus
// ---------------------------------------------------------------------------

/// Status of SKSE in a game installation, suitable for the Tauri frontend.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkseStatus {
    /// Whether SKSE appears to be installed (loader exe found).
    pub installed: bool,
    /// Path to the SKSE loader executable, if found.
    pub loader_path: Option<String>,
    /// Detected SKSE version (e.g. "2.2.6"), if a versioned DLL was found.
    pub version: Option<String>,
    /// Whether the user has opted to use SKSE for launching.
    pub use_skse: bool,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// The SKSE loader executable name (Skyrim SE 64-bit).
const SKSE_LOADER: &str = "skse64_loader.exe";

/// Official SKSE download page.
const SKSE_URL: &str = "https://skse.silverlock.org/";

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Detect whether SKSE is installed in the given game directory.
///
/// Scans for `skse64_loader.exe` (case-insensitive) and attempts to extract
/// the version number from any `skse64_X_Y_Z.dll` file present.
pub fn detect_skse(game_path: &Path) -> SkseStatus {
    let loader = find_file_case_insensitive(game_path, SKSE_LOADER);
    let version = detect_skse_version(game_path);

    let installed = loader.is_some();
    let loader_path = loader.map(|p| p.to_string_lossy().into_owned());

    SkseStatus {
        installed,
        loader_path,
        version,
        use_skse: false, // Caller should overlay the preference from config.
    }
}

/// Detect the SKSE version by finding a `skse64_X_Y_Z.dll` file.
///
/// Specifically looks for DLLs matching the pattern `skse64_\d+_\d+_\d+.dll`
/// but **excludes** the Steam loader variant (`skse64_steam_loader.dll`).
#[allow(dead_code)]
fn detect_skse_version(game_path: &Path) -> Option<String> {
    let entries = fs::read_dir(game_path).ok()?;

    for entry in entries.flatten() {
        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_lowercase();

        // Skip the steam loader — we want the main SKSE DLL.
        if name.contains("steam_loader") {
            continue;
        }

        // Match pattern: skse64_X_Y_Z.dll
        if name.starts_with("skse64_") && name.ends_with(".dll") {
            let stem = name
                .trim_start_matches("skse64_")
                .trim_end_matches(".dll");

            // Parse "X_Y_Z" into "X.Y.Z"
            let parts: Vec<&str> = stem.split('_').collect();
            if parts.len() == 3 && parts.iter().all(|p| p.parse::<u32>().is_ok()) {
                let version = parts.join(".");
                debug!("Detected SKSE version: {} from file: {}", version, name);
                return Some(version);
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Download & Install
// ---------------------------------------------------------------------------

/// Download and install SKSE from the official website.
///
/// 1. Fetches the SKSE download page and parses the `.7z` link.
/// 2. Downloads the archive into `download_dir`.
/// 3. Extracts the archive.
/// 4. Copies SKSE files into the game directory.
///
/// Returns an updated [`SkseStatus`] reflecting the new installation.
pub async fn download_and_install_skse(
    game_path: &Path,
    download_dir: &Path,
) -> Result<SkseStatus> {
    info!("Starting SKSE download and installation");

    // 1. Fetch the SKSE page and find the download link.
    let html = reqwest::get(SKSE_URL)
        .await?
        .text()
        .await?;

    let relative_url =
        parse_skse_download_url(&html).ok_or(SkseError::DownloadLinkNotFound)?;

    // Build the full URL (relative links on skse.silverlock.org).
    let download_url = if relative_url.starts_with("http") {
        relative_url.clone()
    } else {
        format!("{}{}", SKSE_URL.trim_end_matches('/'), relative_url)
    };

    info!("SKSE download URL: {}", download_url);

    // 2. Download the archive.
    fs::create_dir_all(download_dir)?;

    let file_name = download_url
        .rsplit('/')
        .next()
        .unwrap_or("skse64.7z")
        .to_string();
    let archive_path = download_dir.join(&file_name);

    info!("Downloading SKSE to: {}", archive_path.display());

    let response = reqwest::get(&download_url).await?;
    let bytes = response.bytes().await?;
    fs::write(&archive_path, &bytes)?;

    info!(
        "Downloaded {} bytes to {}",
        bytes.len(),
        archive_path.display()
    );

    // 3. Extract the archive.
    let extract_dir = download_dir.join("skse_extract");
    if extract_dir.exists() {
        fs::remove_dir_all(&extract_dir)?;
    }
    fs::create_dir_all(&extract_dir)?;

    installer::extract_archive(&archive_path, &extract_dir).map_err(|e| {
        SkseError::Extraction(format!("Failed to extract SKSE archive: {}", e))
    })?;

    // 4. Install SKSE files into the game directory.
    install_skse_files(&extract_dir, game_path)?;

    // Clean up extraction directory.
    if let Err(e) = fs::remove_dir_all(&extract_dir) {
        warn!("Failed to clean up SKSE extraction dir: {}", e);
    }

    // 5. Return updated status.
    let status = detect_skse(game_path);
    info!(
        "SKSE installation complete: installed={}, version={:?}",
        status.installed, status.version
    );

    Ok(status)
}

/// Install SKSE files from an extracted archive directory into the game.
///
/// Expects the extracted archive to contain a single top-level directory
/// (e.g. `skse64_2_02_06/`) which in turn contains the SKSE files.
///
/// Copies:
/// - `.exe` and `.dll` files from the SKSE root -> game root
/// - `Data/` subfolder contents -> game's `Data/` directory
pub fn install_skse_files(extracted_dir: &Path, game_path: &Path) -> Result<()> {
    // Find the SKSE root directory (the single top-level folder in the archive).
    let skse_root = find_skse_root(extracted_dir)?;

    info!(
        "Installing SKSE from {} to {}",
        skse_root.display(),
        game_path.display()
    );

    // Copy .exe and .dll files from SKSE root to game root.
    let entries = fs::read_dir(&skse_root).map_err(SkseError::Io)?;
    let mut files_copied = 0u32;

    for entry in entries.flatten() {
        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name.ends_with(".exe") || name.ends_with(".dll") {
            let dest = game_path.join(entry.file_name());
            fs::copy(entry.path(), &dest)?;
            debug!("Copied {} -> {}", entry.path().display(), dest.display());
            files_copied += 1;
        }
    }

    info!("Copied {} exe/dll files to game root", files_copied);

    // Copy Data/ subfolder if present.
    let skse_data = find_subdirectory_case_insensitive(&skse_root, "data");
    if let Some(data_src) = skse_data {
        let data_dst = game_path.join("Data");
        fs::create_dir_all(&data_dst)?;
        copy_dir_recursive(&data_src, &data_dst)?;
        info!(
            "Copied SKSE Data directory: {} -> {}",
            data_src.display(),
            data_dst.display()
        );
    } else {
        debug!("No Data/ subfolder found in SKSE archive");
    }

    Ok(())
}

/// Locate the SKSE root directory inside an extracted archive.
///
/// Most SKSE archives contain a single top-level directory like
/// `skse64_2_02_06/`. This function finds it.
fn find_skse_root(extracted_dir: &Path) -> Result<PathBuf> {
    let entries: Vec<_> = fs::read_dir(extracted_dir)
        .map_err(SkseError::Io)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
        .filter(|e| {
            !e.file_name()
                .to_string_lossy()
                .starts_with('.')
        })
        .collect();

    // If there's exactly one top-level directory, that's our SKSE root.
    if entries.len() == 1 {
        return Ok(entries[0].path());
    }

    // If there are multiple directories, look for one starting with "skse".
    for entry in &entries {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name.starts_with("skse") {
            return Ok(entry.path());
        }
    }

    // Check if the extracted_dir itself contains SKSE files directly.
    if find_file_case_insensitive(extracted_dir, SKSE_LOADER).is_some() {
        return Ok(extracted_dir.to_path_buf());
    }

    Err(SkseError::SkseRootNotFound)
}

// ---------------------------------------------------------------------------
// Config preferences
// ---------------------------------------------------------------------------

/// Read the user's SKSE preference for a specific game+bottle combination.
///
/// Stores the preference under the config key `skse_enabled_{game_id}_{bottle_name}`.
pub fn get_skse_preference(game_id: &str, bottle_name: &str) -> bool {
    let key = format!("skse_enabled_{}_{}", game_id, bottle_name);
    config::get_config_value(&key)
        .ok()
        .flatten()
        .map(|v| v == "true")
        .unwrap_or(false)
}

/// Store the user's SKSE preference for a specific game+bottle combination.
pub fn set_skse_preference(game_id: &str, bottle_name: &str, enabled: bool) -> Result<()> {
    let key = format!("skse_enabled_{}_{}", game_id, bottle_name);
    let value = if enabled { "true" } else { "false" };
    config::set_config_value(&key, value).map_err(|e| SkseError::Config(e.to_string()))
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Find a file by name (case-insensitive) in a directory (non-recursive).
#[allow(dead_code)]
pub fn find_file_case_insensitive(dir: &Path, target: &str) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }

    let target_lower = target.to_lowercase();

    // Fast path: try exact match.
    let exact = dir.join(target);
    if exact.exists() {
        return Some(exact);
    }

    // Case-insensitive scan.
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name == target_lower {
                return Some(entry.path());
            }
        }
    }

    None
}

/// Find a subdirectory by name (case-insensitive) in a directory.
#[allow(dead_code)]
fn find_subdirectory_case_insensitive(dir: &Path, target: &str) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }

    let target_lower = target.to_lowercase();

    // Fast path: try exact match.
    let exact = dir.join(target);
    if exact.is_dir() {
        return Some(exact);
    }

    // Case-insensitive scan.
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name == target_lower {
                return Some(entry.path());
            }
        }
    }

    None
}

/// Parse the SKSE download page HTML and extract the `.7z` download URL
/// for the 64-bit build (skse64).
///
/// Looks for `href="..."` attributes containing both "skse64" and ".7z".
#[allow(dead_code)]
fn parse_skse_download_url(html: &str) -> Option<String> {
    // Simple HTML parsing: find href attributes that point to .7z files
    // containing "skse64" in the filename.
    //
    // We avoid pulling in a full HTML parser for this one use case.
    // The SKSE page has a straightforward structure with direct links.

    for segment in html.split("href=\"") {
        if let Some(end) = segment.find('"') {
            let href = &segment[..end];

            let href_lower = href.to_lowercase();
            if href_lower.contains("skse64") && href_lower.ends_with(".7z") {
                // Skip any "src" (source code) archives.
                if href_lower.contains("_src") {
                    continue;
                }

                debug!("Found SKSE download link: {}", href);
                return Some(href.to_string());
            }
        }
    }

    // Also try looking for links with the pattern in single-quoted hrefs.
    for segment in html.split("href='") {
        if let Some(end) = segment.find('\'') {
            let href = &segment[..end];

            let href_lower = href.to_lowercase();
            if href_lower.contains("skse64") && href_lower.ends_with(".7z") {
                if href_lower.contains("_src") {
                    continue;
                }

                debug!("Found SKSE download link (single-quoted): {}", href);
                return Some(href.to_string());
            }
        }
    }

    None
}

/// Recursively copy the contents of `src` into `dst`.
///
/// Creates `dst` and any necessary parent directories. Existing files
/// in `dst` will be overwritten.
#[allow(dead_code)]
pub fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    if !src.is_dir() {
        return Err(SkseError::Io(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Source directory does not exist: {}", src.display()),
        )));
    }

    fs::create_dir_all(dst)?;

    for entry in fs::read_dir(src)?.flatten() {
        let entry_path = entry.path();
        let dest_path = dst.join(entry.file_name());

        if entry_path.is_dir() {
            copy_dir_recursive(&entry_path, &dest_path)?;
        } else {
            fs::copy(&entry_path, &dest_path)?;
            debug!(
                "Copied: {} -> {}",
                entry_path.display(),
                dest_path.display()
            );
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn detect_skse_not_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let status = detect_skse(tmp.path());
        assert!(!status.installed);
        assert!(status.loader_path.is_none());
        assert!(status.version.is_none());
    }

    #[test]
    fn detect_skse_installed_with_version() {
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path();

        // Create fake SKSE files.
        fs::write(game_dir.join("skse64_loader.exe"), b"fake loader").unwrap();
        fs::write(game_dir.join("skse64_2_2_6.dll"), b"fake dll").unwrap();
        fs::write(
            game_dir.join("skse64_steam_loader.dll"),
            b"fake steam loader",
        )
        .unwrap();

        let status = detect_skse(game_dir);
        assert!(status.installed);
        assert!(status.loader_path.is_some());
        assert_eq!(status.version, Some("2.2.6".to_string()));
    }

    #[test]
    fn detect_skse_installed_without_version_dll() {
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path();

        // Only the loader, no versioned DLL.
        fs::write(game_dir.join("skse64_loader.exe"), b"fake loader").unwrap();

        let status = detect_skse(game_dir);
        assert!(status.installed);
        assert!(status.version.is_none());
    }

    #[test]
    fn detect_skse_case_insensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path();

        // Use different casing.
        fs::write(game_dir.join("SKSE64_Loader.exe"), b"fake").unwrap();
        fs::write(game_dir.join("SKSE64_2_2_6.dll"), b"fake").unwrap();

        let status = detect_skse(game_dir);
        assert!(status.installed);
        assert_eq!(status.version, Some("2.2.6".to_string()));
    }

    #[test]
    fn parse_skse_download_url_finds_link() {
        let html = r#"
            <html><body>
            <a href="/beta/skse64_2_02_06.7z">SKSE64 2.2.6</a>
            <a href="/beta/skse64_2_02_06_src.7z">Source</a>
            </body></html>
        "#;

        let url = parse_skse_download_url(html);
        assert!(url.is_some());
        let url = url.unwrap();
        assert!(url.contains("skse64_2_02_06.7z"));
        // Should NOT match the source archive.
        assert!(!url.contains("_src"));
    }

    #[test]
    fn parse_skse_download_url_returns_none_for_no_match() {
        let html = r#"<html><body><p>No links here</p></body></html>"#;
        assert!(parse_skse_download_url(html).is_none());
    }

    #[test]
    fn parse_skse_download_url_skips_source() {
        let html = r#"<a href="/beta/skse64_2_02_06_src.7z">Source only</a>"#;
        // Only source links, no binary.
        assert!(parse_skse_download_url(html).is_none());
    }

    #[test]
    fn find_file_case_insensitive_works() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("SomeFile.TXT"), b"hello").unwrap();

        let result = find_file_case_insensitive(tmp.path(), "somefile.txt");
        assert!(result.is_some());
    }

    #[test]
    fn find_file_case_insensitive_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let result = find_file_case_insensitive(tmp.path(), "nonexistent.txt");
        assert!(result.is_none());
    }

    #[test]
    fn copy_dir_recursive_works() {
        let tmp = tempfile::tempdir().unwrap();

        // Create source tree.
        let src = tmp.path().join("src");
        let src_sub = src.join("subdir");
        fs::create_dir_all(&src_sub).unwrap();
        fs::write(src.join("file1.txt"), b"hello").unwrap();
        fs::write(src_sub.join("file2.txt"), b"world").unwrap();

        // Copy to destination.
        let dst = tmp.path().join("dst");
        copy_dir_recursive(&src, &dst).unwrap();

        // Verify.
        assert!(dst.join("file1.txt").exists());
        assert!(dst.join("subdir").join("file2.txt").exists());
        assert_eq!(
            fs::read_to_string(dst.join("file1.txt")).unwrap(),
            "hello"
        );
        assert_eq!(
            fs::read_to_string(dst.join("subdir").join("file2.txt")).unwrap(),
            "world"
        );
    }

    #[test]
    fn find_skse_root_single_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let skse_dir = tmp.path().join("skse64_2_02_06");
        fs::create_dir_all(&skse_dir).unwrap();
        fs::write(skse_dir.join("skse64_loader.exe"), b"fake").unwrap();

        let root = find_skse_root(tmp.path()).unwrap();
        assert_eq!(root, skse_dir);
    }

    #[test]
    fn find_skse_root_multiple_dirs_picks_skse() {
        let tmp = tempfile::tempdir().unwrap();
        let skse_dir = tmp.path().join("skse64_2_02_06");
        let other_dir = tmp.path().join("readme_files");
        fs::create_dir_all(&skse_dir).unwrap();
        fs::create_dir_all(&other_dir).unwrap();

        let root = find_skse_root(tmp.path()).unwrap();
        assert_eq!(root, skse_dir);
    }

    #[test]
    fn install_skse_files_copies_correctly() {
        let tmp = tempfile::tempdir().unwrap();

        // Create a fake extracted SKSE archive.
        let extracted = tmp.path().join("extracted");
        let skse_root = extracted.join("skse64_2_02_06");
        let skse_data = skse_root.join("Data").join("Scripts");
        fs::create_dir_all(&skse_data).unwrap();
        fs::write(skse_root.join("skse64_loader.exe"), b"loader").unwrap();
        fs::write(skse_root.join("skse64_2_2_6.dll"), b"main dll").unwrap();
        fs::write(
            skse_root.join("skse64_steam_loader.dll"),
            b"steam loader",
        )
        .unwrap();
        fs::write(skse_data.join("SKSE.pex"), b"script").unwrap();

        // Create a fake game directory.
        let game_dir = tmp.path().join("game");
        fs::create_dir_all(&game_dir).unwrap();

        // Install.
        install_skse_files(&extracted, &game_dir).unwrap();

        // Verify exe/dll files were copied to game root.
        assert!(game_dir.join("skse64_loader.exe").exists());
        assert!(game_dir.join("skse64_2_2_6.dll").exists());
        assert!(game_dir.join("skse64_steam_loader.dll").exists());

        // Verify Data/Scripts was copied.
        assert!(game_dir.join("Data").join("Scripts").join("SKSE.pex").exists());
    }

    #[test]
    fn skse_status_serializes() {
        let status = SkseStatus {
            installed: true,
            loader_path: Some("/game/skse64_loader.exe".to_string()),
            version: Some("2.2.6".to_string()),
            use_skse: true,
        };

        let json = serde_json::to_string(&status).unwrap();
        let deserialized: SkseStatus = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.installed, true);
        assert_eq!(
            deserialized.loader_path,
            Some("/game/skse64_loader.exe".to_string())
        );
        assert_eq!(deserialized.version, Some("2.2.6".to_string()));
        assert_eq!(deserialized.use_skse, true);
    }
}
