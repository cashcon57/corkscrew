//! BepInEx bootstrap support for Unity games under Wine/Proton.
//!
//! BepInEx 5 uses UnityDoorstop's `winhttp.dll` proxy. Under Wine/Proton the
//! proxy is ignored unless `winhttp` is forced native/builtin via
//! `WINEDLLOVERRIDES=winhttp=n,b` (or winecfg). Corkscrew applies that launch
//! fix automatically for games that opt into BepInEx support and can also lay
//! down the BepInEx 5 Mono x64 bootstrap at the game root.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

const GITHUB_LATEST_RELEASE: &str = "https://api.github.com/repos/BepInEx/BepInEx/releases/latest";
const USER_AGENT: &str = "Corkscrew-BepInEx-Installer";
const MAX_BEPINEX_ZIP_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BepInExStatus {
    pub installed: bool,
    pub winhttp_dll: PathBuf,
    pub core_dll: PathBuf,
    pub plugins_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BepInExInstallResult {
    AlreadyInstalled,
    Installed { asset_name: String },
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    size: Option<u64>,
}

pub fn status(game_path: &Path) -> BepInExStatus {
    let winhttp_dll = game_path.join("winhttp.dll");
    let core_dll = game_path.join("BepInEx").join("core").join("BepInEx.dll");
    let plugins_dir = game_path.join("BepInEx").join("plugins");
    BepInExStatus {
        installed: winhttp_dll.is_file() && core_dll.is_file(),
        winhttp_dll,
        core_dll,
        plugins_dir,
    }
}

pub fn ensure_bepinex_mono_x64(game_path: &Path) -> Result<BepInExInstallResult> {
    let current = status(game_path);
    if current.installed {
        fs::create_dir_all(&current.plugins_dir).with_context(|| {
            format!(
                "Failed to create BepInEx plugins dir at {}",
                current.plugins_dir.display()
            )
        })?;
        return Ok(BepInExInstallResult::AlreadyInstalled);
    }

    if !game_path.is_dir() {
        anyhow::bail!("Game path does not exist: {}", game_path.display());
    }

    let asset = find_latest_mono_x64_asset()?;
    let bytes = download_asset(&asset)?;
    extract_bepinex_zip(game_path, &bytes)?;

    let installed = status(game_path);
    if !installed.installed {
        anyhow::bail!(
            "BepInEx extraction completed but expected files are missing: {}, {}",
            installed.winhttp_dll.display(),
            installed.core_dll.display()
        );
    }
    fs::create_dir_all(&installed.plugins_dir).with_context(|| {
        format!(
            "Failed to create BepInEx plugins dir at {}",
            installed.plugins_dir.display()
        )
    })?;

    Ok(BepInExInstallResult::Installed {
        asset_name: asset.name,
    })
}

fn find_latest_mono_x64_asset() -> Result<GithubAsset> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .context("Failed to build BepInEx HTTP client")?;

    let release: GithubRelease = client
        .get(GITHUB_LATEST_RELEASE)
        .send()
        .context("Failed to fetch latest BepInEx release")?
        .error_for_status()
        .context("BepInEx latest release request failed")?
        .json()
        .context("Failed to parse latest BepInEx release JSON")?;

    release
        .assets
        .into_iter()
        .find(|asset| is_mono_x64_zip(&asset.name))
        .ok_or_else(|| anyhow!("No BepInEx 5 Mono x64 zip asset found in latest release"))
}

fn is_mono_x64_zip(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".zip")
        && lower.contains("bepinex")
        && lower.contains("x64")
        && !lower.contains("x86")
        && !lower.contains("unix")
        && !lower.contains("il2cpp")
}

fn download_asset(asset: &GithubAsset) -> Result<Vec<u8>> {
    if let Some(size) = asset.size {
        if size > MAX_BEPINEX_ZIP_BYTES {
            anyhow::bail!("BepInEx asset too large: {} bytes", size);
        }
    }

    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .context("Failed to build BepInEx download client")?;

    let mut resp = client
        .get(&asset.browser_download_url)
        .send()
        .with_context(|| format!("Failed to download {}", asset.name))?
        .error_for_status()
        .with_context(|| format!("BepInEx asset download failed: {}", asset.name))?;

    let mut bytes = Vec::new();
    resp.read_to_end(&mut bytes)
        .with_context(|| format!("Failed to read BepInEx asset: {}", asset.name))?;
    if bytes.len() as u64 > MAX_BEPINEX_ZIP_BYTES {
        anyhow::bail!("BepInEx asset exceeded size limit while downloading");
    }
    Ok(bytes)
}

fn extract_bepinex_zip(game_path: &Path, bytes: &[u8]) -> Result<()> {
    let reader = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(reader).context("Failed to open BepInEx zip")?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .context("Failed to read BepInEx zip entry")?;
        if file.is_dir() {
            continue;
        }
        let enclosed = file
            .enclosed_name()
            .ok_or_else(|| anyhow!("BepInEx zip contains unsafe path: {}", file.name()))?
            .to_path_buf();
        let rel = normalize_bepinex_archive_path(&enclosed);
        let dest = game_path.join(&rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create BepInEx directory {}", parent.display())
            })?;
        }
        let mut out = fs::File::create(&dest)
            .with_context(|| format!("Failed to create BepInEx file {}", dest.display()))?;
        std::io::copy(&mut file, &mut out)
            .with_context(|| format!("Failed to extract BepInEx file {}", dest.display()))?;
    }

    Ok(())
}

fn normalize_bepinex_archive_path(path: &Path) -> PathBuf {
    // Thunderstore BepInExPack wraps payload in BepInExPack/. Official BepInEx
    // releases do not. Support both so manual bootstrap archives route cleanly.
    let mut comps = path.components();
    if let Some(first) = comps.next() {
        if first
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case("BepInExPack")
        {
            return comps.as_path().to_path_buf();
        }
    }
    path.to_path_buf()
}

pub fn is_bepinex_mod_type(type_id: &str) -> bool {
    matches!(
        type_id,
        "BepInEx" | "Generic_BepInExPack_Bootstrap" | "Paralives_BepInEx"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_x64_asset_filter_rejects_wrong_runtimes() {
        assert!(is_mono_x64_zip("BepInEx_win_x64_5.4.23.4.zip"));
        assert!(!is_mono_x64_zip("BepInEx_win_x86_5.4.23.4.zip"));
        assert!(!is_mono_x64_zip("BepInEx_unix_5.4.23.4.zip"));
        assert!(!is_mono_x64_zip("BepInEx_UnityIL2CPP_x64.zip"));
    }

    #[test]
    fn normalizes_thunderstore_pack_wrapper() {
        assert_eq!(
            normalize_bepinex_archive_path(Path::new("BepInExPack/BepInEx/core/BepInEx.dll")),
            PathBuf::from("BepInEx/core/BepInEx.dll")
        );
        assert_eq!(
            normalize_bepinex_archive_path(Path::new("BepInEx/core/BepInEx.dll")),
            PathBuf::from("BepInEx/core/BepInEx.dll")
        );
    }

    #[test]
    fn status_detects_required_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        assert!(!status(root).installed);
        fs::create_dir_all(root.join("BepInEx/core")).unwrap();
        fs::write(root.join("winhttp.dll"), b"proxy").unwrap();
        fs::write(root.join("BepInEx/core/BepInEx.dll"), b"core").unwrap();
        assert!(status(root).installed);
    }
}
