//! Paralives BepInEx support for native macOS mode.
//!
//! Layer 1 is read-only detection. Layer 2 routes staged BepInEx plugin DLLs
//! into an already-installed loader. Layer 3 installs/uninstalls the loader with
//! explicit caller consent; that mutates the app bundle signature via
//! `codesign --remove-signature` on macOS and must stay behind prominent UI
//! warnings.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParalivesBepInExStatus {
    pub installed: bool,
    pub version: Option<String>,
    pub mac_supported: bool,
    pub loader_path: Option<PathBuf>,
    pub reason: Option<String>,
}

#[derive(Debug, Error)]
pub enum BepInExError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("{0}")]
    Other(String),
}

const CORE_DLL: &str = "BepInEx/core/BepInEx.Core.dll";
const VERSION_TXT: &str = "BepInEx/version.txt";
const DOORSTOP_CONFIG: &str = "doorstop_config.ini";
const RUN_SCRIPT: &str = "run_bepinex.sh";

/// Read-only detection of an existing BepInEx install under the Paralives game
/// install directory (usually `Paralives.app/Contents/MacOS`).
pub fn detect(game_install_dir: &Path) -> ParalivesBepInExStatus {
    let bepinex_dir = game_install_dir.join("BepInEx");
    let core = game_install_dir.join(CORE_DLL);
    let doorstop = game_install_dir.join(DOORSTOP_CONFIG);
    let run_script = game_install_dir.join(RUN_SCRIPT);
    let installed =
        bepinex_dir.is_dir() && (core.is_file() || doorstop.is_file() || run_script.is_file());

    let loader_path = find_loader_path(game_install_dir);
    let version = read_version(game_install_dir);
    let mac_supported = loader_path
        .as_ref()
        .map(|p| looks_like_arm64_or_universal_loader(p))
        .unwrap_or(false);
    let reason = if !installed {
        Some("BepInEx markers not found".to_string())
    } else if !mac_supported {
        Some(
            "BepInEx is installed, but no ARM64/universal macOS doorstop loader was found"
                .to_string(),
        )
    } else {
        None
    };

    ParalivesBepInExStatus {
        installed,
        version,
        mac_supported,
        loader_path,
        reason,
    }
}

pub fn is_bepinex_plugin_file(rel: &str) -> bool {
    let norm = rel.replace('\\', "/");
    let lower = norm.to_ascii_lowercase();
    lower.ends_with(".dll")
        && (lower.starts_with("bepinex/plugins/")
            || lower.starts_with("plugins/")
            || !lower.contains('/'))
}

pub fn plugin_target_relative(mod_name: &str, source_rel: &str) -> Result<PathBuf, BepInExError> {
    let safe_mod = safe_component(mod_name)?;
    let norm = source_rel.replace('\\', "/");
    let stripped = norm
        .strip_prefix("BepInEx/plugins/")
        .or_else(|| norm.strip_prefix("bepinex/plugins/"))
        .or_else(|| norm.strip_prefix("plugins/"))
        .unwrap_or(&norm);
    if !crate::staging::is_safe_relative_path(stripped) || stripped.starts_with('/') {
        return Err(BepInExError::Other(format!(
            "unsafe BepInEx plugin path: {source_rel}"
        )));
    }
    Ok(PathBuf::from("BepInEx")
        .join("plugins")
        .join(safe_mod)
        .join(stripped))
}

pub fn data_mod_target_relative(source_rel: &str) -> Result<PathBuf, BepInExError> {
    let norm = source_rel.replace('\\', "/");
    if !crate::staging::is_safe_relative_path(&norm) || norm.starts_with('/') {
        return Err(BepInExError::Other(format!(
            "unsafe Paralives data mod path: {source_rel}"
        )));
    }
    Ok(PathBuf::from(norm))
}

/// Install BepInEx from a caller-provided macOS ARM64/universal release zip.
/// The caller must present the consent warning before invoking this function.
pub fn install_from_archive(
    game_install_dir: &Path,
    app_bundle_path: &Path,
    archive_path: &Path,
    db: &Arc<crate::database::ModDatabase>,
) -> Result<ParalivesBepInExStatus, BepInExError> {
    validate_paralives_paths(game_install_dir, app_bundle_path)?;
    if !archive_path.is_file() {
        return Err(BepInExError::Other(format!(
            "BepInEx archive not found: {}",
            archive_path.display()
        )));
    }
    snapshot(db, app_bundle_path, "paralives-bepinex-install");
    extract_zip_into(archive_path, game_install_dir)?;
    remove_signature_for_doorstop(app_bundle_path)?;
    Ok(detect(game_install_dir))
}

/// Fetch and install the latest BepInEx 6 IL2CPP macOS ARM64/universal release.
pub fn install_latest(
    game_install_dir: &Path,
    app_bundle_path: &Path,
    db: &Arc<crate::database::ModDatabase>,
) -> Result<ParalivesBepInExStatus, BepInExError> {
    let asset = latest_macos_arm64_asset_url()?;
    let tmp = tempfile::tempdir()?;
    let archive = tmp.path().join("bepinex.zip");
    let bytes = reqwest::blocking::get(&asset)?
        .error_for_status()?
        .bytes()?;
    fs::write(&archive, bytes.as_ref())?;
    install_from_archive(game_install_dir, app_bundle_path, &archive, db)
}

/// Remove Corkscrew-deployed BepInEx files. This cannot restore Paralives
/// Studio's original signature; Steam Verify/reinstall is the real signature
/// revert path.
pub fn uninstall(
    game_install_dir: &Path,
    app_bundle_path: &Path,
    db: &Arc<crate::database::ModDatabase>,
) -> Result<(), BepInExError> {
    validate_paralives_paths(game_install_dir, app_bundle_path)?;
    snapshot(db, app_bundle_path, "paralives-bepinex-uninstall");
    for rel in ["BepInEx", DOORSTOP_CONFIG, RUN_SCRIPT, ".doorstop_version"] {
        let path = game_install_dir.join(rel);
        if path.is_dir() {
            fs::remove_dir_all(path)?;
        } else if path.exists() {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn find_loader_path(game_install_dir: &Path) -> Option<PathBuf> {
    let candidates = [
        "libdoorstop.dylib",
        "doorstop_libs/libdoorstop.dylib",
        "BepInEx/core/libdoorstop.dylib",
        "BepInEx/doorstop_libs/libdoorstop.dylib",
    ];
    candidates
        .iter()
        .map(|rel| game_install_dir.join(rel))
        .find(|p| p.is_file())
}

fn looks_like_arm64_or_universal_loader(path: &Path) -> bool {
    let name = path.to_string_lossy().to_ascii_lowercase();
    if name.contains("x64") || name.contains("x86_64") || name.contains("intel") {
        return false;
    }
    if name.contains("arm64") || name.contains("universal") || name.ends_with(".dylib") {
        return true;
    }
    false
}

fn read_version(game_install_dir: &Path) -> Option<String> {
    let version_txt = game_install_dir.join(VERSION_TXT);
    if let Ok(s) = fs::read_to_string(version_txt) {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    let core = game_install_dir.join(CORE_DLL);
    let bytes = fs::read(core).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    find_semver(&text)
}

fn find_semver(text: &str) -> Option<String> {
    for token in
        text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '+'))
    {
        let mut parts = token.split('.');
        let (Some(a), Some(b), Some(c)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        if a.chars().all(|c| c.is_ascii_digit())
            && b.chars().all(|c| c.is_ascii_digit())
            && c.chars()
                .next()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
        {
            return Some(token.to_string());
        }
    }
    None
}

fn safe_component(s: &str) -> Result<String, BepInExError> {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('.').trim_matches('_').to_string();
    if cleaned.is_empty()
        || !crate::staging::is_safe_relative_path(&cleaned)
        || cleaned.contains('/')
        || cleaned.contains('\\')
    {
        return Err(BepInExError::Other(format!("unsafe mod name: {s}")));
    }
    Ok(cleaned)
}

fn validate_paralives_paths(
    game_install_dir: &Path,
    app_bundle_path: &Path,
) -> Result<(), BepInExError> {
    if !game_install_dir.is_dir() {
        return Err(BepInExError::Other(format!(
            "Paralives install dir not found: {}",
            game_install_dir.display()
        )));
    }
    if !app_bundle_path.is_dir()
        || app_bundle_path
            .extension()
            .map(|e| e != "app")
            .unwrap_or(true)
    {
        return Err(BepInExError::Other(format!(
            "Paralives app bundle not found: {}",
            app_bundle_path.display()
        )));
    }
    if crate::native_scanner::is_sandboxed(app_bundle_path) {
        return Err(BepInExError::Other(format!(
            "BepInEx cannot be installed into sandboxed app bundle: {}",
            app_bundle_path.display()
        )));
    }
    Ok(())
}

fn snapshot(db: &Arc<crate::database::ModDatabase>, app_bundle_path: &Path, name: &str) {
    if let Err(e) = crate::rollback::create_native_snapshot(
        db,
        "paralives_native",
        name,
        &format!(
            "Paralives BepInEx operation for {}",
            app_bundle_path.display()
        ),
    ) {
        log::warn!("snapshot before Paralives BepInEx operation failed: {e}");
    }
}

fn extract_zip_into(archive: &Path, dest: &Path) -> Result<(), BepInExError> {
    let file = fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let Some(enclosed) = entry.enclosed_name() else {
            continue;
        };
        let out = dest.join(enclosed);
        if entry.is_dir() {
            fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut f = fs::File::create(&out)?;
            io::copy(&mut entry, &mut f)?;
        }
    }
    Ok(())
}

fn latest_macos_arm64_asset_url() -> Result<String, BepInExError> {
    #[derive(Deserialize)]
    struct Release {
        assets: Vec<Asset>,
    }
    #[derive(Deserialize)]
    struct Asset {
        name: String,
        browser_download_url: String,
    }

    let release: Release = reqwest::blocking::Client::new()
        .get("https://api.github.com/repos/BepInEx/BepInEx/releases/latest")
        .header(reqwest::header::USER_AGENT, "Corkscrew")
        .send()?
        .error_for_status()?
        .json()?;
    release
        .assets
        .into_iter()
        .filter(|a| {
            let n = a.name.to_ascii_lowercase();
            n.ends_with(".zip")
                && n.contains("bepinex")
                && n.contains("il2cpp")
                && (n.contains("macos") || n.contains("osx"))
                && (n.contains("arm64") || n.contains("universal"))
        })
        .map(|a| a.browser_download_url)
        .next()
        .ok_or_else(|| {
            BepInExError::Other(
                "no BepInEx 6 IL2CPP macOS ARM64/universal release asset found".into(),
            )
        })
}

fn remove_signature_for_doorstop(app_bundle_path: &Path) -> Result<(), BepInExError> {
    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("codesign")
            .arg("--remove-signature")
            .arg(app_bundle_path)
            .status()?;
        if !status.success() {
            return Err(BepInExError::Other(format!(
                "codesign --remove-signature failed for {}",
                app_bundle_path.display()
            )));
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app_bundle_path;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn detect_reports_missing_loader() {
        let tmp = tempfile::tempdir().unwrap();
        let status = detect(tmp.path());
        assert!(!status.installed);
        assert!(!status.mac_supported);
    }

    #[test]
    fn detect_reads_version_and_loader() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("BepInEx/core")).unwrap();
        fs::write(tmp.path().join(CORE_DLL), b"BepInEx.Core 6.0.0").unwrap();
        fs::write(tmp.path().join(VERSION_TXT), b"6.0.0-be.733").unwrap();
        fs::write(tmp.path().join(DOORSTOP_CONFIG), b"[UnityDoorstop]").unwrap();
        fs::write(tmp.path().join("libdoorstop.dylib"), b"fake dylib").unwrap();
        let status = detect(tmp.path());
        assert!(status.installed);
        assert!(status.mac_supported);
        assert_eq!(status.version.as_deref(), Some("6.0.0-be.733"));
        assert!(status.loader_path.unwrap().ends_with("libdoorstop.dylib"));
    }

    #[test]
    fn detect_flags_intel_named_loader_as_unsupported() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("BepInEx/core")).unwrap();
        fs::write(tmp.path().join(CORE_DLL), b"BepInEx.Core 5.4.23").unwrap();
        fs::write(tmp.path().join(DOORSTOP_CONFIG), b"[UnityDoorstop]").unwrap();
        fs::create_dir_all(tmp.path().join("doorstop_libs")).unwrap();
        fs::write(
            tmp.path().join("doorstop_libs/libdoorstop_x64.dylib"),
            b"x64",
        )
        .unwrap();
        let status = detect(tmp.path());
        assert!(status.installed);
        assert!(!status.mac_supported);
    }

    #[test]
    fn classifies_plugin_shapes() {
        assert!(is_bepinex_plugin_file("BepInEx/plugins/Foo.dll"));
        assert!(is_bepinex_plugin_file("plugins/Foo.dll"));
        assert!(is_bepinex_plugin_file("Foo.dll"));
        assert!(!is_bepinex_plugin_file("README.md"));
        assert!(!is_bepinex_plugin_file("BepInEx/config/Foo.cfg"));
    }

    #[test]
    fn plugin_target_strips_common_prefixes() {
        let got = plugin_target_relative("Cool Mod", "BepInEx/plugins/Nested/Foo.dll").unwrap();
        assert_eq!(
            got,
            PathBuf::from("BepInEx/plugins/Cool_Mod/Nested/Foo.dll")
        );
    }

    #[test]
    fn plugin_target_rejects_traversal() {
        let err = plugin_target_relative("Cool", "BepInEx/plugins/../Foo.dll").unwrap_err();
        assert!(err.to_string().contains("unsafe"));
    }

    #[test]
    fn data_target_rejects_traversal() {
        assert!(data_mod_target_relative("../escape.asset").is_err());
    }

    #[test]
    fn install_from_archive_extracts_markers() {
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().join("Paralives.app/Contents/MacOS");
        let app = tmp.path().join("Paralives.app");
        fs::create_dir_all(&game).unwrap();
        let db = Arc::new(crate::database::ModDatabase::new(&tmp.path().join("test.db")).unwrap());
        crate::rollback::init_schema(&db).unwrap();

        let zip_path = tmp.path().join("bepinex.zip");
        let file = fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::FileOptions::<()>::default();
        zip.start_file("BepInEx/core/BepInEx.Core.dll", opts)
            .unwrap();
        zip.write_all(b"BepInEx.Core 6.0.0").unwrap();
        zip.start_file("BepInEx/version.txt", opts).unwrap();
        zip.write_all(b"6.0.0").unwrap();
        zip.start_file("doorstop_config.ini", opts).unwrap();
        zip.write_all(b"[UnityDoorstop]").unwrap();
        zip.start_file("libdoorstop.dylib", opts).unwrap();
        zip.write_all(b"fake").unwrap();
        zip.finish().unwrap();

        let status = install_from_archive(&game, &app, &zip_path, &db).unwrap();
        assert!(status.installed);
        assert!(game.join(CORE_DLL).exists());
    }
}
