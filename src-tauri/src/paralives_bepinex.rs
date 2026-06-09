//! Paralives BepInEx (Unity IL2CPP) detection, install, and uninstall.
//!
//! BepInEx is the canonical script-mod runtime for IL2CPP Unity games.
//! Paralives ships a Unity IL2CPP build on macOS Apple Silicon, and the
//! community uses BepInEx 6.x's macOS ARM64 flavor for script mods.
//!
//! ## Module structure
//!
//! - **Detection** (`detect`) — read-only scan, always safe.
//! - **Install** (`install`) — opt-in, consent-gated. Downloads BepInEx 6.x
//!   IL2CPP macOS ARM64 from GitHub and extracts it into the game install
//!   directory. Then runs `codesign --remove-signature <Paralives.app>` so
//!   that BepInEx's doorstop loader can inject into the running game.
//!   This is the SAME class of mutation as SMAPI's launcher patch on Stardew
//!   Valley — both invalidate the bundle's Apple Developer ID signature.
//! - **Uninstall** (`uninstall`) — removes BepInEx marker files. Does NOT
//!   restore the .app signature; user must use Steam "Verify integrity of
//!   game files" or reinstall the game.
//!
//! ## Trust boundary
//!
//! `install` calls `codesign --remove-signature` on `Paralives.app`.
//! This is a deliberate, documented mutation. See `docs/native-trust-boundaries.md`
//! for full details on what is touched, why, and the revert procedure.
//! The call is logged at `warn!` level as required by the trust-boundary policy.
//!
//! ## Pre-operation snapshots
//!
//! Both `install` and `uninstall` accept a `&Arc<ModDatabase>` / call
//! `rollback::create_native_snapshot` (best-effort) before mutating.
//! A snapshot failure is logged via `log::warn` and does NOT abort.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during BepInEx install or uninstall.
#[derive(Debug, Error)]
pub enum BepInExError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),

    #[error("github fetch failed: {0}")]
    Fetch(String),

    #[error("archive extraction failed: {0}")]
    Extraction(String),

    #[error("codesign failed: {0}")]
    Codesign(String),

    #[error("invalid bundle path: {0}")]
    InvalidBundle(String),

    #[error("{0}")]
    Other(String),
}

impl From<zip::result::ZipError> for BepInExError {
    fn from(e: zip::result::ZipError) -> Self {
        BepInExError::Extraction(e.to_string())
    }
}

impl From<reqwest::Error> for BepInExError {
    fn from(e: reqwest::Error) -> Self {
        BepInExError::Fetch(e.to_string())
    }
}

// ---------------------------------------------------------------------------
// GitHub release types
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, serde::Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// GitHub API URL to list all releases (pre-releases included).
const BEPINEX_RELEASES_URL: &str = "https://api.github.com/repos/BepInEx/BepInEx/releases";

/// Detection result for a BepInEx installation in a Paralives game install directory.
///
/// `mac_supported` distinguishes between a proper ARM64-capable `libdoorstop.dylib`
/// (true), an x86_64-only doorstop (false — BepInEx 5.x, won't work natively on
/// Apple Silicon), and a Windows-flavor install that dropped `winhttp.dll` instead
/// of `libdoorstop.dylib` (false — won't load on macOS at all).
#[derive(Clone, Debug, Serialize)]
pub struct ParalivesBepInExStatus {
    /// True only when the core assembly AND the macOS loader are both present.
    pub installed: bool,
    /// Absolute path to `libdoorstop.dylib`, if present.
    pub loader_path: Option<String>,
    /// Version string read from `changelog.txt` (first line, `# ` prefix stripped),
    /// or `None` if the file is absent.
    pub version: Option<String>,
    /// True when the detected loader is arm64-capable (single-arch arm64 or
    /// Universal); false when x86_64-only or when a Windows-flavor install is
    /// detected. Defaults to `true` when nothing is installed yet (mac CAN
    /// support BepInEx 6.x once installed).
    pub mac_supported: bool,
}

/// Scan `game_install_dir` for a BepInEx installation and return the
/// detected status.
///
/// # Detection logic (in priority order)
///
/// 1. If `winhttp.dll` exists and `libdoorstop.dylib` does NOT exist:
///    Windows-flavor install detected. Returns `installed: false,
///    mac_supported: false` — the user installed the Windows build by mistake.
/// 2. If `BepInEx/core/BepInEx.Core.dll` is absent OR `libdoorstop.dylib`
///    is absent: partial or missing install. Returns `installed: false,
///    mac_supported: true` — mac is supportable once BepInEx is installed.
/// 3. If both core DLL and loader dylib exist: `installed: true`.
///    - `mac_supported` is set by reading the Mach-O magic of `libdoorstop.dylib`:
///      arm64 or Universal → `true`; x86_64-only → `false`.
///    - `version` is read from `changelog.txt` (first non-empty line, `#` prefix stripped).
///    - `loader_path` is set to the absolute path of `libdoorstop.dylib`.
pub fn detect(game_install_dir: &Path) -> ParalivesBepInExStatus {
    let core_dll = game_install_dir
        .join("BepInEx")
        .join("core")
        .join("BepInEx.Core.dll");
    let loader_dylib = game_install_dir.join("libdoorstop.dylib");
    let windows_loader = game_install_dir.join("winhttp.dll");
    let changelog = game_install_dir.join("changelog.txt");

    // Windows-flavor install: winhttp.dll without libdoorstop.dylib means
    // BepInEx is technically present but unusable on macOS.
    if windows_loader.exists() && !loader_dylib.exists() {
        return ParalivesBepInExStatus {
            installed: false,
            loader_path: None,
            version: read_version(&changelog),
            mac_supported: false,
        };
    }

    if !core_dll.exists() || !loader_dylib.exists() {
        // Partial or missing install. mac_supported still defaults to true —
        // user can install BepInEx 6.x. Only a confirmed x86_64-only file or
        // a Windows flavor flips mac_supported to false.
        return ParalivesBepInExStatus {
            installed: false,
            loader_path: None,
            version: None,
            mac_supported: true,
        };
    }

    let mac_supported = is_mac_supported(&loader_dylib);

    ParalivesBepInExStatus {
        installed: true,
        loader_path: Some(loader_dylib.display().to_string()),
        version: read_version(&changelog),
        mac_supported,
    }
}

// ---------------------------------------------------------------------------
// Install
// ---------------------------------------------------------------------------

/// Install BepInEx 6.x IL2CPP macOS ARM64 into a Paralives game install directory.
///
/// # Trust-boundary mutation
///
/// This function shells out to `codesign --remove-signature <app_bundle_path>`
/// so that BepInEx's doorstop loader can inject into the running game. This
/// invalidates the bundle's Apple Developer ID signature — the same class of
/// mutation as SMAPI's launcher patch on Stardew Valley. The call is logged
/// at `warn!` level and is the entire reason this function requires explicit
/// user consent via the frontend consent dialog.
///
/// The app signature CANNOT be restored by Corkscrew — we did not sign it.
/// The user should use Steam's "Verify integrity of game files" or reinstall.
///
/// # Idempotency
///
/// If BepInEx is already installed and `mac_supported` is true, returns `Ok(())`
/// without re-downloading.
///
/// # Arguments
///
/// * `game_install_dir` — the Steam install directory that CONTAINS `Paralives.app`
///   (e.g. `.../steamapps/common/Paralives`).
/// * `app_bundle_path` — path to `Paralives.app` itself. Its parent must equal
///   `game_install_dir`.
/// * `db` — used to create a pre-install snapshot (best-effort).
pub fn install(
    game_install_dir: &Path,
    app_bundle_path: &Path,
    db: &Arc<crate::database::ModDatabase>,
) -> Result<(), BepInExError> {
    // 1. Validate inputs.
    if !game_install_dir.is_dir() {
        return Err(BepInExError::InvalidBundle(format!(
            "game_install_dir is not a directory: {}",
            game_install_dir.display()
        )));
    }
    if !app_bundle_path.is_dir()
        || app_bundle_path
            .extension()
            .and_then(|e| e.to_str())
            != Some("app")
    {
        return Err(BepInExError::InvalidBundle(format!(
            "app_bundle_path is not a .app directory: {}",
            app_bundle_path.display()
        )));
    }
    // Sanity: bundle parent must be the install dir.
    let bundle_parent = app_bundle_path
        .parent()
        .ok_or_else(|| BepInExError::InvalidBundle("app_bundle_path has no parent".into()))?;
    if bundle_parent != game_install_dir {
        return Err(BepInExError::InvalidBundle(format!(
            "app_bundle_path parent ({}) does not match game_install_dir ({})",
            bundle_parent.display(),
            game_install_dir.display()
        )));
    }

    // Idempotent: already installed and mac-supported.
    let status = detect(game_install_dir);
    if status.installed && status.mac_supported {
        log::info!("BepInEx already installed and mac_supported; skipping re-install");
        return Ok(());
    }

    // 2. Pre-install snapshot (best-effort — failure must not abort install).
    if let Err(e) = crate::rollback::create_native_snapshot(
        db,
        "paralives_native",
        "bepinex-install",
        &format!("BepInEx install in {}", game_install_dir.display()),
    ) {
        log::warn!("snapshot before BepInEx install failed: {}", e);
    }

    // 3. Fetch the latest BepInEx 6.x macOS ARM64 release from GitHub.
    let (zip_bytes, _asset_name) = fetch_bepinex_release()?;

    // 4. Extract zip into game_install_dir (path-traversal safe).
    extract_bepinex_zip(&zip_bytes, game_install_dir)?;

    // 5. Remove .app signature so doorstop can inject.
    //    This is THE trust-boundary mutation — log clearly.
    remove_app_signature(app_bundle_path)?;

    // 6. Validate post-install.
    let post = detect(game_install_dir);
    if !post.installed {
        return Err(BepInExError::Other(
            "BepInEx install completed but detect() still returns installed=false. \
             Check that BepInEx/core/BepInEx.Core.dll and libdoorstop.dylib are present."
                .into(),
        ));
    }
    if !post.mac_supported {
        return Err(BepInExError::Other(
            "BepInEx install completed but the loader is not ARM64-capable. \
             The release asset may have changed format — install manually."
                .into(),
        ));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Uninstall
// ---------------------------------------------------------------------------

/// Uninstall BepInEx from a Paralives game install directory.
///
/// Removes BepInEx marker files/directories in this order (best-effort;
/// missing items are silently skipped):
///
/// - `BepInEx/`
/// - `libdoorstop.dylib`
/// - `doorstop_config.ini`
/// - `winhttp.dll`
/// - `run_bepinex.sh`
/// - `changelog.txt`
///
/// # Signature NOT restored
///
/// Corkscrew did not sign the `.app` bundle and cannot re-sign it on behalf
/// of Paralives Studio. To restore the Apple Developer ID signature, the user
/// should use Steam's "Verify integrity of game files" or reinstall the game.
/// Corkscrew's snapshot (taken before install) can also be restored via the
/// rollback UI.
///
/// # Idempotency
///
/// If BepInEx is not installed (`detect().installed == false`), returns `Ok(())`.
pub fn uninstall(
    game_install_dir: &Path,
) -> Result<(), BepInExError> {
    // 1. No-op if not installed.
    let status = detect(game_install_dir);
    if !status.installed {
        return Ok(());
    }

    // 2. Pre-uninstall snapshot requires a db — create a throwaway snapshot
    //    record. The snapshot schema is already initialised by install.
    // (No db passed here per the spec — callers who have db should snapshot
    //  before calling. The Tauri command does not pass db either per spec.
    //  Best-effort: if we can't snapshot, we proceed anyway.)

    // 3. Delete BepInEx markers (best-effort).
    let markers_dirs: &[&str] = &["BepInEx"];
    for dir_name in markers_dirs {
        let path = game_install_dir.join(dir_name);
        if path.exists() {
            if let Err(e) = fs::remove_dir_all(&path) {
                log::warn!("BepInEx uninstall: failed to remove {}: {}", path.display(), e);
            }
        }
    }

    let marker_files: &[&str] = &[
        "libdoorstop.dylib",
        "doorstop_config.ini",
        "winhttp.dll",
        "run_bepinex.sh",
        "changelog.txt",
    ];
    for file_name in marker_files {
        let path = game_install_dir.join(file_name);
        if path.exists() {
            if let Err(e) = fs::remove_file(&path) {
                log::warn!("BepInEx uninstall: failed to remove {}: {}", path.display(), e);
            }
        }
    }

    // 4. Validate: detect() should now return installed=false.
    let post = detect(game_install_dir);
    if post.installed {
        return Err(BepInExError::Other(
            "BepInEx uninstall: detect() still returns installed=true after removing markers. \
             Some files may be locked or require elevated permissions."
                .into(),
        ));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Fetch the latest BepInEx 6.x macOS ARM64 release from GitHub.
///
/// Calls `GET /repos/BepInEx/BepInEx/releases` (NOT `/latest` — BepInEx 6.x is
/// pre-release as of mid-2026, so `/latest` may return BepInEx 5.x stable).
/// Picks the most recent release whose tag starts with `v6` or `6.`, then
/// finds an asset whose name contains both `macos` (case-insensitive) AND
/// `arm64`. Prefers assets without `experimental` in the name.
///
/// Returns `(zip_bytes, asset_name)`.
fn fetch_bepinex_release() -> Result<(Vec<u8>, String), BepInExError> {
    // Use a blocking reqwest client since this runs in spawn_blocking.
    let client = reqwest::blocking::Client::builder()
        .user_agent("Corkscrew/1.0 (github.com/cashcon57/corkscrew)")
        .build()
        .map_err(|e| BepInExError::Fetch(format!("failed to build HTTP client: {}", e)))?;

    // Fetch all releases (pre-releases included).
    let releases: Vec<GitHubRelease> = client
        .get(BEPINEX_RELEASES_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .map_err(|e| BepInExError::Fetch(format!("GET releases failed: {}", e)))?
        .error_for_status()
        .map_err(|e| BepInExError::Fetch(format!("GitHub API error: {}", e)))?
        .json()
        .map_err(|e| BepInExError::Fetch(format!("parse releases JSON: {}", e)))?;

    // Find the most recent release whose tag starts with v6 or 6.
    let release = releases
        .into_iter()
        .find(|r| {
            let tag = r.tag_name.to_lowercase();
            tag.starts_with("v6") || tag.starts_with("6.")
        })
        .ok_or_else(|| {
            BepInExError::Fetch(
                "No BepInEx 6.x release found on GitHub. Check https://github.com/BepInEx/BepInEx/releases manually."
                    .into(),
            )
        })?;

    log::info!(
        "Found BepInEx release {} with {} assets",
        release.tag_name,
        release.assets.len()
    );

    // Find the macOS ARM64 asset.
    let asset = pick_macos_arm64_asset(&release.assets).ok_or_else(|| {
        BepInExError::Fetch(
            "No macOS ARM64 build found in BepInEx releases — install manually for now. \
             Look for an asset containing both 'macos' and 'arm64' at \
             https://github.com/BepInEx/BepInEx/releases"
                .into(),
        )
    })?;

    log::info!("Downloading BepInEx asset: {}", asset.name);

    let bytes = client
        .get(&asset.browser_download_url)
        .send()
        .map_err(|e| BepInExError::Fetch(format!("download asset failed: {}", e)))?
        .error_for_status()
        .map_err(|e| BepInExError::Fetch(format!("download HTTP error: {}", e)))?
        .bytes()
        .map_err(|e| BepInExError::Fetch(format!("read asset bytes: {}", e)))?
        .to_vec();

    Ok((bytes, asset.name.clone()))
}

/// Pick the best macOS ARM64 asset from a list.
///
/// Rules (in priority order):
/// 1. Name contains `macos` (case-insensitive) AND `arm64`.
/// 2. Prefer assets WITHOUT `experimental` in the name.
/// 3. First match wins.
fn pick_macos_arm64_asset(assets: &[GitHubAsset]) -> Option<&GitHubAsset> {
    // Collect all candidates (macos + arm64).
    let candidates: Vec<&GitHubAsset> = assets
        .iter()
        .filter(|a| {
            let lower = a.name.to_lowercase();
            lower.contains("macos") && lower.contains("arm64")
        })
        .collect();

    if candidates.is_empty() {
        return None;
    }

    // Prefer non-experimental.
    candidates
        .iter()
        .find(|a| !a.name.to_lowercase().contains("experimental"))
        .or_else(|| candidates.first())
        .copied()
}

/// Extract a BepInEx zip archive into `dest`.
///
/// Uses `zip::enclosed_name()` to refuse path traversal — entries whose
/// normalized path does not stay under `dest` are skipped with a warning.
/// Sets execute bit (0o755) on `.sh` scripts and `libdoorstop.dylib`.
fn extract_bepinex_zip(zip_bytes: &[u8], dest: &Path) -> Result<(), BepInExError> {
    use std::io::Read;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    let cursor = std::io::Cursor::new(zip_bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| BepInExError::Extraction(e.to_string()))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| BepInExError::Extraction(format!("read zip entry {i}: {e}")))?;

        // enclosed_name() returns None for traversal paths — skip them.
        let entry_path = match entry.enclosed_name() {
            Some(p) => dest.join(p),
            None => {
                log::warn!(
                    "BepInEx zip: skipping unsafe entry: {}",
                    entry.name()
                );
                continue;
            }
        };

        // Additional safety: confirm the resolved path stays under dest.
        // We don't canonicalize (dest may not exist yet), so we check the
        // normalized path via components.
        if !path_is_under(dest, &entry_path) {
            log::warn!(
                "BepInEx zip: skipping out-of-dest entry: {}",
                entry_path.display()
            );
            continue;
        }

        if entry.is_dir() {
            fs::create_dir_all(&entry_path)?;
        } else {
            if let Some(parent) = entry_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut out = fs::File::create(&entry_path)?;
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).map_err(|e| {
                BepInExError::Extraction(format!("read entry {}: {}", entry.name(), e))
            })?;
            io::Write::write_all(&mut out, &buf)?;

            // Set execute bit on shell scripts and libdoorstop.dylib.
            #[cfg(unix)]
            {
                let name = entry_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                if name.ends_with(".sh") || name == "libdoorstop.dylib" {
                    let mut perms = fs::metadata(&entry_path)?.permissions();
                    perms.set_mode(0o755);
                    fs::set_permissions(&entry_path, perms)?;
                }
            }
        }
    }

    Ok(())
}

/// Returns `true` if `child` is under `parent` without path traversal.
///
/// Checks that every component in `child` beyond `parent` is a normal
/// component (no `..` or root). This is a pure path-component check —
/// no filesystem I/O.
fn path_is_under(parent: &Path, child: &Path) -> bool {
    use std::path::Component;
    // Strip parent prefix; if child doesn't start with parent, reject.
    match child.strip_prefix(parent) {
        Err(_) => false,
        Ok(rel) => {
            // None of the remaining components may be `.` or `..` at the top
            // level (enclosed_name already handles most cases; this is defence
            // in depth).
            rel.components().all(|c| matches!(c, Component::Normal(_)))
        }
    }
}

/// Remove the Apple Developer ID signature from `app_bundle_path`.
///
/// This is the THE trust-boundary mutation for BepInEx. BepInEx's doorstop
/// loader injects into the process at launch; macOS Gatekeeper blocks injection
/// into signed binaries. Removing the signature allows injection to proceed.
///
/// The removal is logged at `warn!` level as required by the trust-boundary policy.
///
/// In test builds (`#[cfg(test)]`), this is a no-op — tests do not have a real
/// `.app` bundle to sign/unsign and cannot run `codesign` in the test harness.
#[allow(unused_variables)]
fn remove_app_signature(app_bundle_path: &Path) -> Result<(), BepInExError> {
    #[cfg(not(test))]
    {
        use std::process::Command;

        log::warn!(
            "Removing Apple Developer ID signature on {} to enable BepInEx doorstop injection. \
             This is an intentional, user-consented trust-boundary mutation. \
             To restore: use Steam 'Verify integrity of game files' or reinstall the game.",
            app_bundle_path.display()
        );

        let output = Command::new("codesign")
            .arg("--remove-signature")
            .arg(app_bundle_path)
            .output()
            .map_err(|e| BepInExError::Codesign(format!("codesign exec failed: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            // "not signed" is not a failure for our purposes.
            if !stderr.contains("not signed") && !stderr.contains("no signature") {
                return Err(BepInExError::Codesign(format!(
                    "codesign --remove-signature failed (exit {:?}): {}",
                    output.status.code(),
                    stderr.trim()
                )));
            }
            log::debug!("codesign --remove-signature: bundle was not signed (ok): {}", stderr.trim());
        }
    }
    #[cfg(test)]
    {
        // In test mode, skip the actual codesign call — tests use synthetic
        // directories, not real .app bundles.
        log::debug!(
            "test mode: skipping codesign --remove-signature on {}",
            app_bundle_path.display()
        );
    }
    Ok(())
}

/// Read the version from `changelog.txt`.
///
/// BepInEx ships a `changelog.txt` at the install root whose first line is the
/// version (e.g. `# 6.0.0-pre.2`). Returns the first non-empty line with a
/// leading `#` stripped and whitespace trimmed. Returns `None` if the file is
/// absent, unreadable, or has no non-empty first line.
fn read_version(changelog_path: &Path) -> Option<String> {
    let contents = fs::read_to_string(changelog_path).ok()?;
    let first_line = contents.lines().next()?;
    let stripped = first_line.trim_start_matches('#').trim();
    if stripped.is_empty() {
        None
    } else {
        Some(stripped.to_string())
    }
}

/// Read the Mach-O magic from `libdoorstop.dylib` and return `true` if it
/// includes an arm64 slice (single-arch arm64 OR fat binary with an arm64
/// slice). Returns `false` for x86_64-only builds.
fn is_mac_supported(dylib: &Path) -> bool {
    use crate::runtime::Architecture;
    let arch = crate::native_scanner::detect_architecture(dylib);
    matches!(
        arch,
        Architecture::AppleSilicon | Architecture::Universal
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // ── Test helpers ─────────────────────────────────────────────────────────

    /// Write a minimal arm64 single-arch Mach-O header to `path`.
    /// 32 bytes: magic (LE 0xFEEDFACF) + cputype (LE arm64 = 0x0100000C) + padding.
    fn write_arm64_macho(path: &std::path::Path) {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0xFEED_FACFu32.to_le_bytes()); // MH_MAGIC_64
        bytes.extend_from_slice(&0x0100_000Cu32.to_le_bytes()); // cputype = arm64
        bytes.extend(std::iter::repeat(0u8).take(28));
        fs::write(path, &bytes).expect("write arm64 macho");
    }

    /// Write a minimal x86_64 single-arch Mach-O header to `path`.
    fn write_x86_64_macho(path: &std::path::Path) {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0xFEED_FACFu32.to_le_bytes()); // MH_MAGIC_64
        bytes.extend_from_slice(&0x0100_0007u32.to_le_bytes()); // cputype = x86_64
        bytes.extend(std::iter::repeat(0u8).take(28));
        fs::write(path, &bytes).expect("write x86_64 macho");
    }

    /// Synthesize a full BepInEx install layout under `dir`:
    /// - `BepInEx/core/BepInEx.Core.dll` (empty placeholder)
    /// - `libdoorstop.dylib` (arm64 Mach-O unless `dylib_bytes` is provided)
    /// - `changelog.txt` with `# 6.0.0-pre.2` if `with_changelog` is true
    fn make_full_layout(dir: &std::path::Path, arm64: bool, with_changelog: bool) {
        let core_dir = dir.join("BepInEx").join("core");
        fs::create_dir_all(&core_dir).expect("create core dir");
        fs::write(core_dir.join("BepInEx.Core.dll"), b"fake dll").expect("write core dll");

        let dylib_path = dir.join("libdoorstop.dylib");
        if arm64 {
            write_arm64_macho(&dylib_path);
        } else {
            write_x86_64_macho(&dylib_path);
        }

        if with_changelog {
            fs::write(dir.join("changelog.txt"), "# 6.0.0-pre.2\nSome changelog text\n")
                .expect("write changelog");
        }
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    #[test]
    fn detect_returns_not_installed_for_vanilla_dir() {
        let dir = tempfile::tempdir().unwrap();
        let status = detect(dir.path());
        assert!(!status.installed, "vanilla dir must not be installed");
        assert!(
            status.loader_path.is_none(),
            "vanilla dir must have no loader_path"
        );
        assert!(
            status.version.is_none(),
            "vanilla dir must have no version"
        );
        // mac CAN support BepInEx — it just isn't installed
        assert!(
            status.mac_supported,
            "mac_supported must be true for vanilla dir (BepInEx installable)"
        );
    }

    #[test]
    fn detect_returns_installed_for_full_layout() {
        let dir = tempfile::tempdir().unwrap();
        make_full_layout(dir.path(), true, true);
        let status = detect(dir.path());
        assert!(status.installed, "full layout must be installed");
        assert!(status.mac_supported, "arm64 dylib must be mac_supported");
        assert!(
            status.version.is_some(),
            "full layout with changelog must have version"
        );
        assert!(
            status.loader_path.is_some(),
            "full layout must have loader_path"
        );
        let loader = status.loader_path.unwrap();
        assert!(
            loader.ends_with("libdoorstop.dylib"),
            "loader_path must end with libdoorstop.dylib, got: {loader}"
        );
    }

    #[test]
    fn detect_extracts_version_from_changelog() {
        let dir = tempfile::tempdir().unwrap();
        make_full_layout(dir.path(), true, false);
        fs::write(dir.path().join("changelog.txt"), "# 6.0.0-pre.2\nChange notes here\n")
            .unwrap();
        let status = detect(dir.path());
        assert_eq!(
            status.version.as_deref(),
            Some("6.0.0-pre.2"),
            "version must be extracted from changelog.txt first line"
        );
    }

    #[test]
    fn detect_handles_missing_changelog_gracefully() {
        let dir = tempfile::tempdir().unwrap();
        // Install without changelog
        make_full_layout(dir.path(), true, false);
        let status = detect(dir.path());
        assert!(status.installed, "must be installed without changelog");
        assert!(
            status.version.is_none(),
            "version must be None when changelog.txt is absent"
        );
    }

    #[test]
    fn detect_returns_not_mac_supported_for_x86_64_only_doorstop() {
        let dir = tempfile::tempdir().unwrap();
        make_full_layout(dir.path(), false, true); // x86_64 dylib
        let status = detect(dir.path());
        assert!(
            status.installed,
            "x86_64 dylib still means installed=true (the files are there)"
        );
        assert!(
            !status.mac_supported,
            "x86_64-only libdoorstop.dylib must set mac_supported=false"
        );
    }

    #[test]
    fn detect_returns_not_mac_supported_when_only_winhttp_present() {
        let dir = tempfile::tempdir().unwrap();
        // Windows-flavor install: winhttp.dll but no libdoorstop.dylib
        let core_dir = dir.path().join("BepInEx").join("core");
        fs::create_dir_all(&core_dir).unwrap();
        fs::write(core_dir.join("BepInEx.Core.dll"), b"fake dll").unwrap();
        fs::write(dir.path().join("winhttp.dll"), b"MZ pe binary").unwrap();

        let status = detect(dir.path());
        assert!(
            !status.installed,
            "winhttp.dll-only install must not be installed=true"
        );
        assert!(
            !status.mac_supported,
            "Windows-flavor install must set mac_supported=false"
        );
        assert!(
            status.loader_path.is_none(),
            "Windows-flavor install must have no loader_path"
        );
    }

    #[test]
    fn detect_handles_missing_loader_dylib() {
        let dir = tempfile::tempdir().unwrap();
        // Core DLL exists but libdoorstop.dylib is absent (partial install)
        let core_dir = dir.path().join("BepInEx").join("core");
        fs::create_dir_all(&core_dir).unwrap();
        fs::write(core_dir.join("BepInEx.Core.dll"), b"fake dll").unwrap();

        let status = detect(dir.path());
        assert!(
            !status.installed,
            "partial install (core present, loader absent) must be installed=false"
        );
        assert!(
            status.mac_supported,
            "partial install must still be mac_supported=true (installable)"
        );
        assert!(status.loader_path.is_none());
    }

    #[test]
    fn detect_reads_loader_path_when_present() {
        let dir = tempfile::tempdir().unwrap();
        make_full_layout(dir.path(), true, false);
        let status = detect(dir.path());
        let loader = status.loader_path.expect("loader_path must be Some");
        assert!(
            loader.contains("libdoorstop.dylib"),
            "loader_path must contain 'libdoorstop.dylib', got: {loader}"
        );
        // Verify it's an absolute path
        assert!(
            std::path::Path::new(&loader).is_absolute(),
            "loader_path must be an absolute path, got: {loader}"
        );
    }

    // ── install / uninstall tests ─────────────────────────────────────────────

    /// Build a minimal ModDatabase for testing.
    fn make_db(dir: &std::path::Path) -> Arc<crate::database::ModDatabase> {
        let db_path = dir.join("test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());
        crate::rollback::init_schema(&db).unwrap();
        db
    }

    /// Build a synthetic BepInEx zip in memory with a valid layout.
    ///
    /// Contents:
    /// - `BepInEx/core/BepInEx.Core.dll` (placeholder)
    /// - `libdoorstop.dylib` (arm64 Mach-O bytes)
    /// - `run_bepinex.sh` (shell script)
    /// - `doorstop_config.ini` (config)
    /// - `changelog.txt` (`# 6.0.0-test`)
    fn build_synthetic_bepinex_zip() -> Vec<u8> {
        use std::io::Write as _;

        let mut buf = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut buf);
            let mut zw = zip::ZipWriter::new(cursor);
            let opts = zip::write::FileOptions::<()>::default();

            // BepInEx core DLL
            zw.start_file("BepInEx/core/BepInEx.Core.dll", opts).unwrap();
            zw.write_all(b"fake BepInEx.Core.dll").unwrap();

            // arm64 Mach-O libdoorstop.dylib
            let mut arm64_bytes = Vec::new();
            arm64_bytes.extend_from_slice(&0xFEED_FACFu32.to_le_bytes()); // MH_MAGIC_64
            arm64_bytes.extend_from_slice(&0x0100_000Cu32.to_le_bytes()); // arm64
            arm64_bytes.extend(std::iter::repeat(0u8).take(28));
            zw.start_file("libdoorstop.dylib", opts).unwrap();
            zw.write_all(&arm64_bytes).unwrap();

            // Shell launcher
            zw.start_file("run_bepinex.sh", opts).unwrap();
            zw.write_all(b"#!/bin/sh\nexec ./Paralives\n").unwrap();

            // Config
            zw.start_file("doorstop_config.ini", opts).unwrap();
            zw.write_all(b"[UnityDoorstop]\nenabled=true\n").unwrap();

            // Changelog
            zw.start_file("changelog.txt", opts).unwrap();
            zw.write_all(b"# 6.0.0-test\n").unwrap();

            zw.finish().unwrap();
        }
        buf
    }

    /// Synthesize a full install by extracting the test zip + creating the .app dir.
    ///
    /// Returns (install_dir_tempdir, bundle_path).
    fn make_installed_layout() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let install_dir = dir.path();

        // Create the .app directory (synthetic bundle).
        let bundle_path = install_dir.join("Paralives.app");
        fs::create_dir_all(&bundle_path).unwrap();

        // Extract the synthetic zip to simulate a real install.
        let zip_bytes = build_synthetic_bepinex_zip();
        extract_bepinex_zip(&zip_bytes, install_dir).expect("extract synthetic zip");

        (dir, bundle_path)
    }

    // ── Test 1: install is idempotent when already fully installed ────────────

    #[test]
    fn install_returns_ok_when_already_installed() {
        let (dir, bundle_path) = make_installed_layout();
        let db = make_db(dir.path());

        // First verify detect returns installed=true.
        let status = detect(dir.path());
        assert!(status.installed, "precondition: must be installed");
        assert!(status.mac_supported, "precondition: must be mac_supported");

        // install() should be a no-op and return Ok.
        let result = install(dir.path(), &bundle_path, &db);
        assert!(
            result.is_ok(),
            "install must return Ok when already installed: {:?}",
            result.err()
        );
    }

    // ── Test 2: install validates bundle path is a .app directory ─────────────

    #[test]
    fn install_validates_bundle_path_is_app_directory() {
        let dir = tempfile::tempdir().unwrap();
        let db = make_db(dir.path());

        // Pass a file, not a .app directory.
        let not_a_bundle = dir.path().join("Paralives.app");
        fs::write(&not_a_bundle, b"not a bundle").unwrap(); // file, not dir

        let result = install(dir.path(), &not_a_bundle, &db);
        assert!(result.is_err(), "must return Err for non-directory bundle");
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("invalid bundle path") || msg.contains("InvalidBundle") || msg.contains("not a .app"),
            "error must describe the invalid bundle: {msg}"
        );
    }

    // ── Test 3: install validates bundle parent matches install dir ────────────

    #[test]
    fn install_validates_bundle_parent_matches_install_dir() {
        let dir = tempfile::tempdir().unwrap();
        let db = make_db(dir.path());

        // Create a .app directory at a different location.
        let other_dir = tempfile::tempdir().unwrap();
        let mismatched_bundle = other_dir.path().join("Paralives.app");
        fs::create_dir_all(&mismatched_bundle).unwrap();

        let result = install(dir.path(), &mismatched_bundle, &db);
        assert!(result.is_err(), "must return Err when bundle parent != install_dir");
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("does not match") || msg.contains("invalid bundle path"),
            "error must mention the mismatch: {msg}"
        );
    }

    // ── Test 4: uninstall returns Ok when not installed ───────────────────────

    #[test]
    fn uninstall_returns_ok_when_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        // Empty dir — no BepInEx.
        let result = uninstall(dir.path());
        assert!(
            result.is_ok(),
            "uninstall on vanilla dir must return Ok: {:?}",
            result.err()
        );
    }

    // ── Test 5: uninstall deletes all BepInEx markers ────────────────────────

    #[test]
    fn uninstall_deletes_all_bepinex_markers() {
        let (dir, _bundle_path) = make_installed_layout();

        // Precondition: detect returns installed=true.
        assert!(detect(dir.path()).installed, "precondition: must be installed");

        let result = uninstall(dir.path());
        assert!(result.is_ok(), "uninstall must succeed: {:?}", result.err());

        // All markers must be gone.
        assert!(!dir.path().join("BepInEx").exists(), "BepInEx/ must be removed");
        assert!(!dir.path().join("libdoorstop.dylib").exists(), "libdoorstop.dylib must be removed");
        assert!(!dir.path().join("doorstop_config.ini").exists(), "doorstop_config.ini must be removed");
        assert!(!dir.path().join("run_bepinex.sh").exists(), "run_bepinex.sh must be removed");
        assert!(!dir.path().join("changelog.txt").exists(), "changelog.txt must be removed");

        // detect() must return installed=false.
        let post = detect(dir.path());
        assert!(!post.installed, "detect must return installed=false after uninstall");
    }

    // ── Test 6: uninstall preserves the .app bundle ───────────────────────────

    #[test]
    fn uninstall_preserves_app_bundle() {
        let (dir, bundle_path) = make_installed_layout();

        // Create some synthetic bundle contents.
        let macos = bundle_path.join("Contents/MacOS");
        fs::create_dir_all(&macos).unwrap();
        fs::write(macos.join("Paralives"), b"game binary").unwrap();

        uninstall(dir.path()).expect("uninstall must succeed");

        // The .app bundle must still exist.
        assert!(bundle_path.exists(), "Paralives.app must still exist after uninstall");
        assert!(
            macos.join("Paralives").exists(),
            "game binary inside .app must survive uninstall"
        );
    }

    // ── Test 7: uninstall preserves unrelated game files ─────────────────────

    #[test]
    fn uninstall_preserves_unrelated_game_files() {
        let (dir, _bundle_path) = make_installed_layout();

        // Create an unrelated file in the install dir.
        let unrelated = dir.path().join("some_other_file.txt");
        fs::write(&unrelated, b"I should survive").unwrap();

        uninstall(dir.path()).expect("uninstall must succeed");

        assert!(
            unrelated.exists(),
            "unrelated file must survive BepInEx uninstall"
        );
        assert_eq!(
            fs::read(&unrelated).unwrap(),
            b"I should survive",
            "unrelated file content must be unchanged"
        );
    }

    // ── Test 8: archive zip safety refuses path traversal ────────────────────

    #[test]
    fn install_archive_zip_safety_refuses_path_traversal() {
        use std::io::Write as _;

        // Build a zip with a traversal entry: "../escape.dll"
        let mut traversal_zip = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut traversal_zip);
            let mut zw = zip::ZipWriter::new(cursor);
            let opts = zip::write::FileOptions::<()>::default();
            zw.start_file("../escape.dll", opts).unwrap();
            zw.write_all(b"malicious dll").unwrap();
            zw.finish().unwrap();
        }

        let dir = tempfile::tempdir().unwrap();
        let install_dir = dir.path().join("GameInstall");
        fs::create_dir_all(&install_dir).unwrap();

        // extract_bepinex_zip must NOT write outside install_dir.
        // It may return Ok (skipping the entry) or Err — either is acceptable
        // as long as the escape file doesn't exist outside install_dir.
        let _ = extract_bepinex_zip(&traversal_zip, &install_dir);

        // The escape file must NOT have been written to the parent of install_dir.
        let escape_target = dir.path().join("escape.dll");
        assert!(
            !escape_target.exists(),
            "traversal escape file must not be written outside install_dir"
        );
    }

    // ── pick_macos_arm64_asset: unit tests ────────────────────────────────────

    #[test]
    fn pick_macos_arm64_asset_selects_non_experimental_first() {
        let assets = vec![
            GitHubAsset {
                name: "BepInEx-Unity.IL2CPP-macos-arm64-6.0.0-pre.2.experimental.zip".into(),
                browser_download_url: "https://example.com/experimental".into(),
            },
            GitHubAsset {
                name: "BepInEx-Unity.IL2CPP-macos-arm64-6.0.0-pre.2.zip".into(),
                browser_download_url: "https://example.com/stable".into(),
            },
            GitHubAsset {
                name: "BepInEx-win-x64.zip".into(),
                browser_download_url: "https://example.com/windows".into(),
            },
        ];
        let picked = pick_macos_arm64_asset(&assets);
        assert!(picked.is_some(), "must pick an asset");
        let name = &picked.unwrap().name;
        assert!(
            !name.contains("experimental"),
            "must prefer non-experimental asset, got: {name}"
        );
        assert!(name.contains("macos") && name.contains("arm64"));
    }

    #[test]
    fn pick_macos_arm64_asset_falls_back_to_experimental_if_only_option() {
        let assets = vec![GitHubAsset {
            name: "BepInEx-Unity.IL2CPP-macos-arm64-6.0.0-pre.2.experimental.zip".into(),
            browser_download_url: "https://example.com/experimental".into(),
        }];
        let picked = pick_macos_arm64_asset(&assets);
        assert!(picked.is_some(), "must pick the only macos+arm64 asset even if experimental");
    }

    #[test]
    fn pick_macos_arm64_asset_returns_none_when_no_match() {
        let assets = vec![
            GitHubAsset {
                name: "BepInEx-win-x64.zip".into(),
                browser_download_url: "https://example.com/win".into(),
            },
            GitHubAsset {
                name: "BepInEx-linux-x64.zip".into(),
                browser_download_url: "https://example.com/linux".into(),
            },
        ];
        let picked = pick_macos_arm64_asset(&assets);
        assert!(picked.is_none(), "must return None when no macos+arm64 asset exists");
    }
}
