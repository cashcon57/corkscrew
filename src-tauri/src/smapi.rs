//! SMAPI (Stardew Modding API) detection and install.
//!
//! SMAPI is the canonical mod loader for Stardew Valley. Its installer
//! mutates the game's .app bundle by renaming the vanilla
//! `Contents/MacOS/StardewValley` launcher to `StardewValley-original`
//! and dropping a new launcher that loads SMAPI's runtime. We detect
//! presence by checking for those markers.
//!
//! ## Install procedure
//!
//! `install(app_bundle, installer_archive)` replicates the file-op steps
//! from SMAPI's `install on macOS.command` + `InteractiveInstaller.cs`
//! in pure Rust, without shelling out to the .NET installer binary.
//!
//! See the spike spec at `docs/superpowers/plans/2026-04-28-native-macos-game-support-smapi-install-spec.md`
//! for the full procedure and source references.
//!
//! ## Pre-operation snapshots
//!
//! Both `install` and `uninstall` accept a `&Arc<ModDatabase>` and call
//! `rollback::create_native_snapshot` (best-effort) before mutating the bundle.
//! A snapshot failure is logged via `log::warn` and does NOT abort the operation.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SmapiError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),

    #[error("{0}")]
    Other(String),
}

// Impl From<zip::result::ZipError> so we can use ? in zip operations.
impl From<zip::result::ZipError> for SmapiError {
    fn from(e: zip::result::ZipError) -> Self {
        SmapiError::Other(e.to_string())
    }
}

/// Marker file: SMAPI renames the vanilla launcher to this when installed.
const VANILLA_LAUNCHER_RENAMED: &str = "StardewValley-original";

/// Marker file: SMAPI's main executable, present only when SMAPI is installed.
const SMAPI_EXECUTABLE: &str = "StardewModdingAPI";

/// Stardew Valley's macOS bundle identifier. SMAPI mutates the app bundle, so
/// Corkscrew refuses to run it against look-alike or user-selected non-Stardew
/// `.app` bundles.
const STARDEW_BUNDLE_IDENTIFIER: &str = "com.chucklefish.stardewvalley";

/// Vanilla macOS launcher name inside `Contents/MacOS`.
const STARDEW_LAUNCHER: &str = "StardewValley";

/// Detect whether SMAPI is installed in a Stardew Valley `.app` bundle.
///
/// Returns true iff BOTH the renamed vanilla launcher (`StardewValley-original`)
/// AND the SMAPI executable (`StardewModdingAPI`) exist in the bundle's
/// `Contents/MacOS` directory. Either marker alone indicates a partial /
/// broken install — return false so the caller can prompt re-install.
pub fn is_installed(app_bundle: &Path) -> bool {
    let macos = app_bundle.join("Contents").join("MacOS");
    macos.join(VANILLA_LAUNCHER_RENAMED).exists() && macos.join(SMAPI_EXECUTABLE).exists()
}

/// Read the installed SMAPI version, if available.
///
/// Looks at the deps.json file SMAPI's installer drops alongside the
/// SMAPI executable. Returns None if the file is missing, malformed,
/// or doesn't contain a recognizable version string.
pub fn installed_version(app_bundle: &Path) -> Option<String> {
    let macos = app_bundle.join("Contents").join("MacOS");
    let candidates = [
        macos.join("StardewModdingAPI.deps.json"),
        macos.join("smapi-internal/Stardew Valley.deps.json"),
    ];
    for path in &candidates {
        if let Ok(contents) = fs::read_to_string(path) {
            if let Some(version) = extract_version_from_deps_json(&contents) {
                return Some(version);
            }
        }
    }
    None
}

/// Install SMAPI into a Stardew Valley `.app` bundle.
///
/// # Arguments
///
/// * `app_bundle` — path to the `.app` directory (e.g. `Stardew Valley.app`).
/// * `installer_archive` — path to the SMAPI installer zip
///   (`SMAPI-X.Y.Z-installer.zip`).  The zip must contain a nested
///   `install.dat` (a second zip with the actual bundle payload) somewhere
///   in its directory tree — typically at `macOS/install.dat`.
/// * `db` — mod database; used to create a pre-install snapshot (best-effort,
///   a snapshot failure does not abort the install).
///
/// # Procedure
///
/// Implements SMAPI's macOS install steps in order:
///
/// 1. Extract outer installer zip to a temp directory.
/// 2. Locate `install.dat` (nested zip) in the extracted tree.
/// 3. Extract `install.dat` to a second temp dir — that is the bundle payload.
/// 4. Recursive-copy payload into `<app_bundle>/Contents/MacOS/`,
///    **excluding** the `mcs` and `Mods` top-level directories.
/// 5. Rename `Contents/MacOS/StardewValley` → `StardewValley-original`
///    (idempotent: only if `StardewValley-original` does not yet exist).
/// 6. Move `unix-launcher.sh` → `Contents/MacOS/StardewValley`.
/// 7. `chmod 755` on `StardewValley` and `StardewModdingAPI`.
/// 8. Copy `Stardew Valley.deps.json` → `StardewModdingAPI.deps.json`
///    (if the source exists).
/// 9. Create `Contents/MacOS/Mods/` if missing.
///
/// Quarantine xattr clearing is handled by the caller via [`clear_quarantine`].
pub fn install(
    app_bundle: &Path,
    installer_archive: &Path,
    db: &Arc<crate::database::ModDatabase>,
) -> Result<(), SmapiError> {
    // 0. Validate inputs.
    if !app_bundle.is_dir() {
        return Err(SmapiError::Other(format!(
            "app_bundle is not a directory: {}",
            app_bundle.display()
        )));
    }
    if crate::native_scanner::is_sandboxed(app_bundle) {
        return Err(SmapiError::Other(format!(
            "SMAPI cannot install in sandboxed bundle: {}",
            app_bundle.display()
        )));
    }
    if !installer_archive.is_file() {
        return Err(SmapiError::Other(format!(
            "installer archive not found: {}",
            installer_archive.display()
        )));
    }
    validate_stardew_bundle(app_bundle)?;

    // Pre-install snapshot (best-effort — failure must not abort install).
    if let Err(e) = crate::rollback::create_native_snapshot(
        db,
        "stardew_valley_native",
        "smapi-install",
        &format!("SMAPI install in {}", app_bundle.display()),
    ) {
        log::warn!("snapshot before SMAPI install failed: {}", e);
    }

    // 1. Extract the outer installer zip.
    let outer_temp = tempfile::tempdir().map_err(SmapiError::Io)?;
    extract_zip_into(installer_archive, outer_temp.path())?;

    // 2. Locate install.dat in the extracted tree.
    let install_dat = locate_install_dat(outer_temp.path()).ok_or_else(|| {
        SmapiError::Other("install.dat not found in installer archive".to_string())
    })?;

    // 3. Extract install.dat (the inner zip) to a payload temp dir.
    let payload_temp = tempfile::tempdir().map_err(SmapiError::Io)?;
    extract_zip_into(&install_dat, payload_temp.path())?;

    let macos = app_bundle.join("Contents/MacOS");
    fs::create_dir_all(&macos)?;

    // 4. Rename StardewValley -> StardewValley-original BEFORE payload copy.
    // The SMAPI payload includes replacement launch assets. Preserving vanilla
    // after payload copy can accidentally save a SMAPI file instead of the
    // original game launcher.
    let launcher = macos.join(STARDEW_LAUNCHER);
    let launcher_original = macos.join(VANILLA_LAUNCHER_RENAMED);
    if !launcher_original.exists() && launcher.exists() {
        fs::rename(&launcher, &launcher_original)?;
    } else if !launcher_original.exists() {
        return Err(SmapiError::Other(format!(
            "missing Stardew launcher before SMAPI install: {}",
            launcher.display()
        )));
    }

    // 5. Recursive copy payload into <bundle>/Contents/MacOS/, excluding mcs and Mods.
    copy_dir_excluding(payload_temp.path(), &macos, &["mcs", "Mods"])?;

    // 6. Move unix-launcher.sh -> StardewValley.
    //    If StardewValley-original already existed on entry (second install),
    //    the current StardewValley is the SMAPI launcher from the first pass;
    //    we just overwrite it.
    let unix_launcher = macos.join("unix-launcher.sh");
    if unix_launcher.exists() {
        // If the SMAPI launcher script is still present from a previous install,
        // remove it before moving (rename is atomic only within the same device;
        // on some filesystems it errors if dst exists and is a different type —
        // plain fs::rename handles same-type overwrites on APFS/HFS+).
        if launcher.exists() {
            fs::remove_file(&launcher)?;
        }
        fs::rename(&unix_launcher, &launcher)?;
    }

    // 7. chmod 755 on the executable files.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for name in ["StardewValley", "StardewModdingAPI"] {
            let p = macos.join(name);
            if p.exists() {
                let mut perms = fs::metadata(&p)?.permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&p, perms)?;
            }
        }
    }

    // 8. Copy "Stardew Valley.deps.json" -> "StardewModdingAPI.deps.json".
    let src_deps = macos.join("Stardew Valley.deps.json");
    let dst_deps = macos.join("StardewModdingAPI.deps.json");
    if src_deps.exists() {
        fs::copy(&src_deps, &dst_deps)?;
    }

    // 9. Create Mods/ if missing.
    let mods_dir = macos.join("Mods");
    if !mods_dir.exists() {
        fs::create_dir_all(&mods_dir)?;
    }

    Ok(())
}

/// Uninstall SMAPI from a Stardew Valley `.app` bundle. Reverses the
/// mutations made by `install`: deletes SMAPI files, restores the
/// vanilla launcher from `StardewValley-original`, preserves `Mods/`.
///
/// * `db` — mod database; used to create a pre-uninstall snapshot (best-effort,
///   a snapshot failure does not abort the uninstall).
///
/// Idempotent: calling on a vanilla bundle is a no-op (returns Ok).
pub fn uninstall(
    app_bundle: &Path,
    db: &Arc<crate::database::ModDatabase>,
) -> Result<(), SmapiError> {
    if !app_bundle.is_dir() {
        return Err(SmapiError::Other(format!(
            "not a directory: {}",
            app_bundle.display()
        )));
    }
    if crate::native_scanner::is_sandboxed(app_bundle) {
        return Err(SmapiError::Other(format!(
            "SMAPI cannot uninstall from sandboxed bundle: {}",
            app_bundle.display()
        )));
    }
    validate_stardew_bundle(app_bundle)?;

    let macos = app_bundle.join("Contents/MacOS");
    if !macos.is_dir() {
        return Err(SmapiError::Other(format!(
            "missing Contents/MacOS in bundle: {}",
            app_bundle.display()
        )));
    }

    // No-op if SMAPI isn't installed.
    if !is_installed(app_bundle) {
        return Ok(());
    }

    // Pre-uninstall snapshot (best-effort — failure must not abort uninstall).
    if let Err(e) = crate::rollback::create_native_snapshot(
        db,
        "stardew_valley_native",
        "smapi-uninstall",
        &format!("SMAPI uninstall from {}", app_bundle.display()),
    ) {
        log::warn!("snapshot before SMAPI uninstall failed: {}", e);
    }

    let smapi_launcher = macos.join(STARDEW_LAUNCHER);
    let vanilla_renamed = macos.join(VANILLA_LAUNCHER_RENAMED);

    if !vanilla_renamed.is_file() {
        return Err(SmapiError::Other(format!(
            "cannot uninstall SMAPI safely: missing vanilla launcher backup at {}",
            vanilla_renamed.display()
        )));
    }

    // 1-2. Restore vanilla launcher before deleting other SMAPI files. Keep the
    // current SMAPI launcher as a temporary rollback source until restore
    // succeeds, so an IO failure cannot leave the bundle without a launcher.
    let smapi_launcher_backup = macos.join("StardewValley-smapi-removing");
    if smapi_launcher_backup.exists() {
        fs::remove_file(&smapi_launcher_backup)?;
    }
    if smapi_launcher.exists() {
        fs::rename(&smapi_launcher, &smapi_launcher_backup)?;
    }
    if let Err(e) = fs::rename(&vanilla_renamed, &smapi_launcher) {
        if smapi_launcher_backup.exists() && !smapi_launcher.exists() {
            let _ = fs::rename(&smapi_launcher_backup, &smapi_launcher);
        }
        return Err(SmapiError::Io(e));
    }
    if smapi_launcher_backup.exists() {
        fs::remove_file(&smapi_launcher_backup)?;
    }

    // 3-5. Delete SMAPI files (best-effort — missing files are fine).
    for name in [
        "StardewModdingAPI",
        "StardewModdingAPI.dll",
        "StardewModdingAPI.deps.json",
    ] {
        let p = macos.join(name);
        if p.exists() {
            fs::remove_file(&p)?;
        }
    }

    // 6. Delete smapi-internal/ recursively.
    let smapi_internal = macos.join("smapi-internal");
    if smapi_internal.exists() {
        fs::remove_dir_all(&smapi_internal)?;
    }

    // 7. Preserve Mods/ (intentional — per official SMAPI uninstall behavior).

    Ok(())
}

/// Clear macOS Gatekeeper's `com.apple.quarantine` extended attribute
/// from a Stardew Valley `.app` bundle.
///
/// Steam game updates re-quarantine the bundle. SMAPI's launcher script
/// is then blocked by Gatekeeper until the user approves it manually.
/// To avoid that friction, we clear the quarantine attribute before each
/// launch when SMAPI is detected.
///
/// On non-macOS platforms this is a no-op.
///
/// Tolerates the case where no quarantine attribute is present (xattr
/// returns a non-zero exit silently — that's expected and harmless).
pub fn clear_quarantine(app_bundle: &Path) -> Result<(), SmapiError> {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        // -d delete the named attribute, -r recursive (whole bundle).
        // We don't propagate non-zero exits — the most common cause is
        // "no such xattr" which is fine.
        let output = Command::new("xattr")
            .arg("-dr")
            .arg("com.apple.quarantine")
            .arg(app_bundle)
            .output()
            .map_err(|e| SmapiError::Other(format!("xattr command failed: {}", e)))?;

        // Log non-zero exits but don't fail. The attribute may not exist.
        if !output.status.success() {
            log::debug!(
                "xattr -dr returned non-zero (likely no quarantine to clear): {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app_bundle;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Extract every entry from `archive` into `dest`, creating directories as
/// needed. Entries with unsafe paths (path traversal etc.) are silently
/// skipped — `ZipEntry::enclosed_name()` returns `None` for those.
fn extract_zip_into(archive: &Path, dest: &Path) -> Result<(), SmapiError> {
    let file = fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let entry_path = match entry.enclosed_name() {
            Some(p) => dest.join(p),
            None => continue, // unsafe path — skip
        };
        if entry.is_dir() {
            fs::create_dir_all(&entry_path)?;
        } else {
            if let Some(parent) = entry_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut out = fs::File::create(&entry_path)?;
            io::copy(&mut entry, &mut out)?;
        }
    }
    Ok(())
}

/// Walk `root` recursively looking for a file named `install.dat`. Returns
/// the path to the first one found, or `None` if the tree contains no such
/// file.
fn locate_install_dat(root: &Path) -> Option<PathBuf> {
    fn recurse(dir: &Path, found: &mut Option<PathBuf>) {
        if found.is_some() {
            return;
        }
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                if found.is_some() {
                    return;
                }
                let path = entry.path();
                if path.is_dir() {
                    recurse(&path, found);
                } else if path.file_name().and_then(|s| s.to_str()) == Some("install.dat") {
                    *found = Some(path);
                }
            }
        }
    }
    let mut found = None;
    recurse(root, &mut found);
    found
}

/// Copy every entry from `src` into `dst`, skipping any entry whose name
/// (as an OS string) matches one of the names in `exclude_top_level`.
/// Sub-directories not excluded at the top level are copied fully
/// (i.e. exclusion only applies to the immediate children of `src`).
fn copy_dir_excluding(
    src: &Path,
    dst: &Path,
    exclude_top_level: &[&str],
) -> Result<(), SmapiError> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if exclude_top_level.contains(&name_str.as_ref()) {
            continue;
        }
        let src_path = entry.path();
        let dst_path = dst.join(&name);
        if entry.file_type()?.is_dir() {
            fs::create_dir_all(&dst_path)?;
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

/// Recursively copy a directory tree from `src` into `dst`.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), SmapiError> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

/// Validate that a user-selected `.app` bundle is Stardew Valley and that its
/// declared executable matches the launcher Corkscrew/SMAPI will mutate.
fn validate_stardew_bundle(app_bundle: &Path) -> Result<(), SmapiError> {
    let info_path = app_bundle.join("Contents/Info.plist");
    let info = crate::plist::read_info_plist(&info_path).map_err(|e| {
        SmapiError::Other(format!(
            "cannot validate Stardew Valley app bundle at {}: {}",
            app_bundle.display(),
            e
        ))
    })?;
    if info.bundle_identifier != STARDEW_BUNDLE_IDENTIFIER {
        return Err(SmapiError::Other(format!(
            "refusing to install SMAPI into non-Stardew bundle {} (bundle id: {})",
            app_bundle.display(),
            info.bundle_identifier
        )));
    }
    if info.bundle_executable != STARDEW_LAUNCHER {
        return Err(SmapiError::Other(format!(
            "refusing to install SMAPI into Stardew bundle with unexpected executable '{}' (expected '{}')",
            info.bundle_executable, STARDEW_LAUNCHER
        )));
    }
    Ok(())
}

/// Extract a SMAPI version from a deps.json blob. The format is .NET's
/// dependency JSON; SMAPI's assembly version typically appears under
/// targets.<framework>.StardewModdingAPI/<version>. We do a forgiving
/// scan: parse JSON, walk to find a key that looks like
/// "StardewModdingAPI/<version>" and pull the version part.
fn extract_version_from_deps_json(contents: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(contents).ok()?;
    // Check libraries section first (more reliable — it's a flat map of
    // "AssemblyName/version" → metadata, present in all .NET deps.json files).
    if let Some(libraries) = value.get("libraries").and_then(|v| v.as_object()) {
        for key in libraries.keys() {
            if let Some(rest) = key.strip_prefix("StardewModdingAPI/") {
                return Some(rest.to_string());
            }
        }
    }
    // Fall back to scanning targets.* for any "StardewModdingAPI/X.Y.Z" key.
    if let Some(targets) = value.get("targets").and_then(|v| v.as_object()) {
        for target_obj in targets.values() {
            if let Some(target) = target_obj.as_object() {
                for key in target.keys() {
                    if let Some(rest) = key.strip_prefix("StardewModdingAPI/") {
                        return Some(rest.to_string());
                    }
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    // -----------------------------------------------------------------------
    // Helpers shared by detection + install tests
    // -----------------------------------------------------------------------

    /// Create an in-memory `ModDatabase` in the given temp directory.
    ///
    /// Also initialises the rollback schema (`mod_snapshots` / `snapshot_entries`)
    /// which lives outside the main migration chain and must be created explicitly.
    fn make_db(dir: &Path) -> Arc<crate::database::ModDatabase> {
        let db_path = dir.join("test.db");
        let db = Arc::new(crate::database::ModDatabase::new(&db_path).unwrap());
        crate::rollback::init_schema(&db).unwrap();
        db
    }

    fn make_macos_dir(dir: &Path) -> std::path::PathBuf {
        let macos = dir.join("Stardew Valley.app/Contents/MacOS");
        fs::create_dir_all(&macos).expect("mkdir");
        macos
    }

    /// Build a minimal vanilla bundle with a `Contents/MacOS/StardewValley`
    /// launcher script. Returns the `.app` path.
    fn vanilla_bundle(dir: &Path) -> PathBuf {
        let bundle = dir.join("Stardew Valley.app");
        let contents = bundle.join("Contents");
        let macos = contents.join("MacOS");
        fs::create_dir_all(&macos).unwrap();
        fs::write(
            contents.join("Info.plist"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>com.chucklefish.stardewvalley</string>
    <key>CFBundleExecutable</key>
    <string>StardewValley</string>
</dict>
</plist>
"#,
        )
        .unwrap();
        fs::write(
            macos.join("StardewValley"),
            b"#!/bin/bash\n# vanilla launcher\nexec ./StardewValley.bin\n",
        )
        .unwrap();
        bundle
    }

    /// Build a synthetic `install.dat` in memory — this is the inner zip
    /// (the payload extracted from the SMAPI installer).
    fn build_install_dat() -> std::io::Result<Vec<u8>> {
        let mut buf = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut buf);
            let mut zw = zip::ZipWriter::new(cursor);
            let opts = zip::write::FileOptions::<()>::default();

            zw.start_file("unix-launcher.sh", opts)?;
            zw.write_all(b"#!/bin/bash\nexec ./StardewModdingAPI\n")?;

            zw.start_file("StardewModdingAPI", opts)?;
            zw.write_all(b"fake smapi binary")?;

            zw.start_file("StardewModdingAPI.dll", opts)?;
            zw.write_all(b"fake smapi dll")?;

            zw.start_file("StardewModdingAPI.runtimeconfig.json", opts)?;
            zw.write_all(br#"{"runtimeOptions":{}}"#)?;

            zw.start_file("StardewModdingAPI.xml", opts)?;
            zw.write_all(b"<doc />")?;

            zw.start_file("steam_appid.txt", opts)?;
            zw.write_all(b"413150\n")?;

            zw.start_file("Stardew Valley.deps.json", opts)?;
            zw.write_all(br#"{"libraries":{"StardewModdingAPI/4.1.10":{}}}"#)?;

            // mcs and Mods — must NOT be copied per spike step 4.
            zw.start_file("mcs/should-not-copy.txt", opts)?;
            zw.write_all(b"mcs cleanup")?;

            zw.start_file("Mods/should-not-copy/manifest.json", opts)?;
            zw.write_all(b"{}")?;

            // smapi-internal — SHOULD be copied.
            zw.start_file("smapi-internal/config.json", opts)?;
            zw.write_all(b"{}")?;

            zw.finish()?;
        }
        Ok(buf)
    }

    /// Build a synthetic SMAPI installer zip that wraps the `install.dat`
    /// at `macOS/install.dat` — matching real SMAPI release layout.
    fn build_synthetic_installer_archive(out_path: &Path) -> std::io::Result<()> {
        let outer_file = fs::File::create(out_path)?;
        let mut outer_zip = zip::ZipWriter::new(outer_file);
        let opts = zip::write::FileOptions::<()>::default();

        let inner_buf = build_install_dat()?;

        outer_zip.start_file("macOS/install.dat", opts)?;
        outer_zip.write_all(&inner_buf)?;

        // A top-level file that the installer ships alongside install.dat;
        // Corkscrew ignores it, but it should not trip up our extractor.
        outer_zip.start_file("README.txt", opts)?;
        outer_zip.write_all(b"SMAPI installer\n")?;

        outer_zip.finish()?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Detection tests (unchanged from Task 3.3)
    // -----------------------------------------------------------------------

    #[test]
    fn is_installed_returns_false_for_vanilla_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let macos = make_macos_dir(dir.path());
        // Vanilla: only StardewValley present, no SMAPI markers.
        fs::write(macos.join("StardewValley"), b"#!/bin/bash\n").unwrap();
        let bundle = dir.path().join("Stardew Valley.app");
        assert!(!is_installed(&bundle));
    }

    #[test]
    fn is_installed_returns_true_when_both_markers_exist() {
        let dir = tempfile::tempdir().unwrap();
        let macos = make_macos_dir(dir.path());
        fs::write(macos.join("StardewValley-original"), b"#!/bin/bash\n").unwrap();
        fs::write(macos.join("StardewModdingAPI"), b"smapi binary").unwrap();
        let bundle = dir.path().join("Stardew Valley.app");
        assert!(is_installed(&bundle));
    }

    #[test]
    fn is_installed_returns_false_when_only_renamed_launcher_exists() {
        let dir = tempfile::tempdir().unwrap();
        let macos = make_macos_dir(dir.path());
        fs::write(macos.join("StardewValley-original"), b"#!/bin/bash\n").unwrap();
        // No StardewModdingAPI — partial install.
        let bundle = dir.path().join("Stardew Valley.app");
        assert!(!is_installed(&bundle));
    }

    #[test]
    fn is_installed_returns_false_when_only_smapi_executable_exists() {
        let dir = tempfile::tempdir().unwrap();
        let macos = make_macos_dir(dir.path());
        fs::write(macos.join("StardewModdingAPI"), b"smapi binary").unwrap();
        // No renamed launcher — partial install.
        let bundle = dir.path().join("Stardew Valley.app");
        assert!(!is_installed(&bundle));
    }

    #[test]
    fn installed_version_returns_none_for_uninstalled() {
        let dir = tempfile::tempdir().unwrap();
        make_macos_dir(dir.path());
        let bundle = dir.path().join("Stardew Valley.app");
        assert_eq!(installed_version(&bundle), None);
    }

    #[test]
    fn installed_version_extracts_version_from_deps_json() {
        let dir = tempfile::tempdir().unwrap();
        let macos = make_macos_dir(dir.path());
        let deps = serde_json::json!({
            "libraries": {
                "StardewModdingAPI/4.1.10": {
                    "type": "project",
                    "serviceable": false,
                    "sha512": ""
                }
            }
        });
        fs::write(macos.join("StardewModdingAPI.deps.json"), deps.to_string()).unwrap();
        let bundle = dir.path().join("Stardew Valley.app");
        assert_eq!(installed_version(&bundle), Some("4.1.10".to_string()));
    }

    #[test]
    fn installed_version_returns_none_for_malformed_deps_json() {
        let dir = tempfile::tempdir().unwrap();
        let macos = make_macos_dir(dir.path());
        fs::write(
            macos.join("StardewModdingAPI.deps.json"),
            b"not valid json {",
        )
        .unwrap();
        let bundle = dir.path().join("Stardew Valley.app");
        assert_eq!(installed_version(&bundle), None);
    }

    // -----------------------------------------------------------------------
    // install() tests
    // -----------------------------------------------------------------------

    /// Happy path: install into a vanilla bundle; verify is_installed returns true.
    #[test]
    fn install_creates_smapi_markers_in_vanilla_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = vanilla_bundle(dir.path());
        let db = make_db(dir.path());

        let archive_path = dir.path().join("smapi-installer.zip");
        build_synthetic_installer_archive(&archive_path).unwrap();

        install(&bundle, &archive_path, &db).expect("install should succeed");

        assert!(
            is_installed(&bundle),
            "is_installed should return true after install"
        );

        // Verify StardewModdingAPI exists with content.
        let smapi_bin = bundle.join("Contents/MacOS/StardewModdingAPI");
        assert!(smapi_bin.exists(), "StardewModdingAPI binary should exist");
    }

    /// Calling install twice succeeds and is_installed stays true.
    #[test]
    fn install_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = vanilla_bundle(dir.path());
        let db = make_db(dir.path());

        let archive_path = dir.path().join("smapi-installer.zip");
        build_synthetic_installer_archive(&archive_path).unwrap();

        install(&bundle, &archive_path, &db).expect("first install should succeed");
        install(&bundle, &archive_path, &db).expect("second install should succeed");

        assert!(
            is_installed(&bundle),
            "is_installed should remain true after second install"
        );
    }

    /// The first install must preserve the original vanilla launcher content
    /// in StardewValley-original, and the second install must not overwrite it.
    #[test]
    fn install_preserves_stardewvalley_original_content() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = vanilla_bundle(dir.path());
        let db = make_db(dir.path());

        // Record vanilla launcher content before any install.
        let vanilla_content = fs::read(bundle.join("Contents/MacOS/StardewValley")).unwrap();

        let archive_path = dir.path().join("smapi-installer.zip");
        build_synthetic_installer_archive(&archive_path).unwrap();

        // First install.
        install(&bundle, &archive_path, &db).expect("first install");

        let original_after_first =
            fs::read(bundle.join("Contents/MacOS/StardewValley-original")).unwrap();
        assert_eq!(
            vanilla_content, original_after_first,
            "StardewValley-original should equal vanilla content after first install"
        );

        // Second install must NOT overwrite StardewValley-original.
        install(&bundle, &archive_path, &db).expect("second install");

        let original_after_second =
            fs::read(bundle.join("Contents/MacOS/StardewValley-original")).unwrap();
        assert_eq!(
            vanilla_content, original_after_second,
            "StardewValley-original must not change on second install"
        );
    }

    /// mcs/ and Mods/ in the installer payload are excluded from the copy.
    #[test]
    fn install_skips_mcs_and_mods_during_copy() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = vanilla_bundle(dir.path());
        let db = make_db(dir.path());

        let archive_path = dir.path().join("smapi-installer.zip");
        build_synthetic_installer_archive(&archive_path).unwrap();

        install(&bundle, &archive_path, &db).expect("install should succeed");

        let macos = bundle.join("Contents/MacOS");

        // mcs/ must not exist.
        assert!(
            !macos.join("mcs").exists(),
            "mcs/ directory must not be copied into the bundle"
        );

        // The Mods/ path created by install() is the one we create (step 9),
        // but it must NOT contain anything from the installer payload's Mods/.
        // The synthetic payload puts "Mods/should-not-copy/manifest.json" —
        // that specific file must not be present.
        assert!(
            !macos.join("Mods/should-not-copy/manifest.json").exists(),
            "Mods/ payload content must not be copied into the bundle"
        );
    }

    /// Non-excluded top-level payload sidecars are copied into Contents/MacOS.
    /// This pins the current trust-boundary surface documented in
    /// docs/native-trust-boundaries.md.
    #[test]
    fn install_copies_non_excluded_payload_sidecars() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = vanilla_bundle(dir.path());
        let db = make_db(dir.path());

        let archive_path = dir.path().join("smapi-installer.zip");
        build_synthetic_installer_archive(&archive_path).unwrap();

        install(&bundle, &archive_path, &db).expect("install should succeed");

        let macos = bundle.join("Contents/MacOS");
        for f in [
            "Stardew Valley.deps.json",
            "StardewModdingAPI.runtimeconfig.json",
            "StardewModdingAPI.xml",
            "steam_appid.txt",
            "smapi-internal/config.json",
        ] {
            assert!(
                macos.join(f).exists(),
                "{f} should be copied from install.dat"
            );
        }

        assert!(
            !macos.join("unix-launcher.sh").exists(),
            "unix-launcher.sh should be moved to StardewValley, not left as a sidecar"
        );
    }

    /// install() creates Contents/MacOS/Mods/ if it does not exist.
    #[test]
    fn install_creates_mods_directory_if_missing() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = vanilla_bundle(dir.path());
        let db = make_db(dir.path());

        // Confirm Mods/ is not present before install.
        let mods_dir = bundle.join("Contents/MacOS/Mods");
        assert!(
            !mods_dir.exists(),
            "precondition: Mods/ must not exist before install"
        );

        let archive_path = dir.path().join("smapi-installer.zip");
        build_synthetic_installer_archive(&archive_path).unwrap();

        install(&bundle, &archive_path, &db).expect("install should succeed");

        assert!(
            mods_dir.is_dir(),
            "Contents/MacOS/Mods/ must be created by install"
        );
    }

    /// Passing a nonexistent bundle path returns an error.
    #[test]
    fn install_returns_error_for_nonexistent_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let db = make_db(dir.path());

        let archive_path = dir.path().join("smapi-installer.zip");
        build_synthetic_installer_archive(&archive_path).unwrap();

        let result = install(Path::new("/nonexistent/Foo.app"), &archive_path, &db);
        assert!(
            result.is_err(),
            "install should return Err for a nonexistent bundle"
        );
    }

    // -----------------------------------------------------------------------
    // uninstall() tests
    // -----------------------------------------------------------------------

    /// Uninstalling a vanilla bundle (SMAPI not installed) is a no-op: Ok,
    /// and the launcher file is still present and unchanged.
    #[test]
    fn uninstall_returns_ok_when_smapi_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = vanilla_bundle(dir.path());
        let db = make_db(dir.path());
        assert!(uninstall(&bundle, &db).is_ok());
        // Bundle still vanilla.
        assert!(bundle.join("Contents/MacOS/StardewValley").exists());
    }

    /// install → uninstall restores the vanilla launcher byte-for-byte.
    /// This is the round-trip property: post-uninstall launcher must equal
    /// the content that existed before any install was run.
    #[test]
    fn uninstall_restores_vanilla_launcher() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = vanilla_bundle(dir.path());
        let db = make_db(dir.path());
        let vanilla_content = fs::read(bundle.join("Contents/MacOS/StardewValley")).unwrap();

        let archive = dir.path().join("smapi.zip");
        build_synthetic_installer_archive(&archive).unwrap();
        install(&bundle, &archive, &db).unwrap();
        assert!(is_installed(&bundle));

        uninstall(&bundle, &db).unwrap();
        assert!(!is_installed(&bundle));

        let post = fs::read(bundle.join("Contents/MacOS/StardewValley")).unwrap();
        assert_eq!(post, vanilla_content, "launcher byte-equal to pre-install");
    }

    /// After uninstall, SMAPI executable, dll, deps.json, and smapi-internal/
    /// are all gone, and StardewValley-original is also gone (renamed back).
    #[test]
    fn uninstall_removes_smapi_files() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = vanilla_bundle(dir.path());
        let db = make_db(dir.path());
        let archive = dir.path().join("smapi.zip");
        build_synthetic_installer_archive(&archive).unwrap();
        install(&bundle, &archive, &db).unwrap();

        uninstall(&bundle, &db).unwrap();

        let macos = bundle.join("Contents/MacOS");
        for f in [
            "StardewModdingAPI",
            "StardewModdingAPI.dll",
            "StardewModdingAPI.deps.json",
        ] {
            assert!(!macos.join(f).exists(), "{f} should have been deleted");
        }
        assert!(
            !macos.join("smapi-internal").exists(),
            "smapi-internal/ should have been deleted"
        );
        assert!(
            !macos.join("StardewValley-original").exists(),
            "StardewValley-original should have been renamed back"
        );
    }

    /// Contents/MacOS/Mods/ and everything inside it survive uninstall.
    #[test]
    fn uninstall_preserves_mods_directory() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = vanilla_bundle(dir.path());
        let db = make_db(dir.path());
        let archive = dir.path().join("smapi.zip");
        build_synthetic_installer_archive(&archive).unwrap();
        install(&bundle, &archive, &db).unwrap();

        // Add a user mod.
        let mods = bundle.join("Contents/MacOS/Mods");
        let user_mod = mods.join("MyMod/manifest.json");
        fs::create_dir_all(user_mod.parent().unwrap()).unwrap();
        fs::write(&user_mod, b"{\"Name\":\"MyMod\"}").unwrap();

        uninstall(&bundle, &db).unwrap();

        assert!(mods.exists(), "Mods/ should persist after uninstall");
        assert!(user_mod.exists(), "user mod content should persist");
        assert_eq!(
            fs::read(&user_mod).unwrap(),
            b"{\"Name\":\"MyMod\"}",
            "user mod content must be byte-equal after uninstall"
        );
    }

    /// Calling uninstall twice succeeds; the second call is a no-op.
    #[test]
    fn uninstall_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = vanilla_bundle(dir.path());
        let db = make_db(dir.path());
        let archive = dir.path().join("smapi.zip");
        build_synthetic_installer_archive(&archive).unwrap();
        install(&bundle, &archive, &db).unwrap();
        uninstall(&bundle, &db).unwrap();
        // Second call is a no-op.
        assert!(uninstall(&bundle, &db).is_ok());
        assert!(!is_installed(&bundle));
    }

    /// Passing a nonexistent bundle path to uninstall returns an error.
    #[test]
    fn uninstall_returns_error_for_nonexistent_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let db = make_db(dir.path());
        let result = uninstall(Path::new("/nonexistent/Foo.app"), &db);
        assert!(
            result.is_err(),
            "uninstall should return Err for a nonexistent bundle"
        );
    }

    // -----------------------------------------------------------------------
    // clear_quarantine() tests
    // -----------------------------------------------------------------------

    /// clear_quarantine succeeds on a bundle without a quarantine attribute.
    /// xattr -dr returns non-zero (no such xattr), but we tolerate that silently.
    #[test]
    fn clear_quarantine_succeeds_on_bundle_without_quarantine() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = vanilla_bundle(dir.path());
        // No quarantine attribute set; xattr -dr should silently no-op.
        assert!(clear_quarantine(&bundle).is_ok());
    }

    /// On non-macOS platforms, clear_quarantine is a no-op that always returns Ok.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn clear_quarantine_returns_ok_on_non_macos_platforms() {
        let dir = tempfile::tempdir().unwrap();
        assert!(clear_quarantine(dir.path()).is_ok());
    }

    /// On real macOS, clear_quarantine does not panic when invoking xattr.
    #[cfg(target_os = "macos")]
    #[test]
    fn clear_quarantine_does_not_panic_on_real_macos_invocation() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = vanilla_bundle(dir.path());
        // xattr should exist on macOS; calling clear_quarantine should not panic.
        // It will return Ok even though there's no quarantine attribute present.
        let result = clear_quarantine(&bundle);
        assert!(
            result.is_ok(),
            "clear_quarantine should not panic on real macOS"
        );
    }

    // -----------------------------------------------------------------------
    // Snapshot integration tests (Task 6.1)
    // -----------------------------------------------------------------------

    /// install() creates a snapshot row in the DB before mutating the bundle.
    #[test]
    fn smapi_install_creates_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = vanilla_bundle(dir.path());
        let db = make_db(dir.path());

        let archive_path = dir.path().join("smapi-installer.zip");
        build_synthetic_installer_archive(&archive_path).unwrap();

        install(&bundle, &archive_path, &db).expect("install should succeed");

        let snapshots = crate::rollback::list_snapshots(&db, "stardew_valley_native", "").unwrap();
        assert!(
            !snapshots.is_empty(),
            "install should have created at least one snapshot"
        );
        let snap = &snapshots[0];
        assert_eq!(snap.name, "smapi-install");
        assert!(
            snap.description
                .as_deref()
                .unwrap_or("")
                .contains("SMAPI install"),
            "description should mention 'SMAPI install', got: {:?}",
            snap.description
        );
    }

    /// uninstall() creates a snapshot row in the DB before mutating the bundle.
    #[test]
    fn smapi_uninstall_creates_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = vanilla_bundle(dir.path());
        let db = make_db(dir.path());

        // First install SMAPI so there's something to uninstall.
        let archive_path = dir.path().join("smapi-installer.zip");
        build_synthetic_installer_archive(&archive_path).unwrap();
        install(&bundle, &archive_path, &db).expect("install before uninstall test");

        // Now uninstall.
        uninstall(&bundle, &db).expect("uninstall should succeed");

        // There should be at least one "smapi-uninstall" snapshot (install
        // creates "smapi-install" first, then uninstall creates "smapi-uninstall").
        let snapshots = crate::rollback::list_snapshots(&db, "stardew_valley_native", "").unwrap();
        let uninstall_snap = snapshots.iter().find(|s| s.name == "smapi-uninstall");
        assert!(
            uninstall_snap.is_some(),
            "uninstall should have created a 'smapi-uninstall' snapshot; got: {:?}",
            snapshots.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
    }

    // -----------------------------------------------------------------------
    // Sandbox refusal tests (Task 6.2)
    // -----------------------------------------------------------------------

    /// Build a synthetic bundle that looks like a Mac App Store app by writing
    /// the `Contents/_MASReceipt/receipt` marker file.
    fn make_mas_bundle(dir: &Path, name: &str) -> PathBuf {
        let bundle = dir.join(format!("{}.app", name));
        let macos = bundle.join("Contents/MacOS");
        let receipt_dir = bundle.join("Contents/_MASReceipt");
        fs::create_dir_all(&macos).unwrap();
        fs::create_dir_all(&receipt_dir).unwrap();
        fs::write(
            macos.join("StardewValley"),
            b"#!/bin/bash\n# vanilla launcher\n",
        )
        .unwrap();
        fs::write(receipt_dir.join("receipt"), b"fake mas receipt").unwrap();
        bundle
    }

    /// `install` must return an error immediately for a sandboxed (MAS) bundle.
    /// No SMAPI files should be written.
    #[test]
    fn smapi_install_refuses_sandboxed_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = make_mas_bundle(dir.path(), "Stardew Valley");
        let db = make_db(dir.path());

        let archive_path = dir.path().join("smapi-installer.zip");
        build_synthetic_installer_archive(&archive_path).unwrap();

        let result = install(&bundle, &archive_path, &db);

        assert!(result.is_err(), "install must refuse sandboxed bundle");
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("sandboxed"),
            "error must mention 'sandboxed': {msg}"
        );
        // SMAPI must NOT have been installed.
        assert!(
            !is_installed(&bundle),
            "SMAPI must not be installed in sandboxed bundle"
        );
    }

    /// `uninstall` must return an error immediately for a sandboxed (MAS) bundle.
    #[test]
    fn smapi_uninstall_refuses_sandboxed_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = make_mas_bundle(dir.path(), "Stardew Valley Sandboxed");
        let db = make_db(dir.path());

        // Simulate a partially-installed SMAPI by writing the marker files
        // directly (we bypass install() since it would also be refused).
        let macos = bundle.join("Contents/MacOS");
        fs::write(macos.join("StardewValley-original"), b"original").unwrap();
        fs::write(macos.join("StardewModdingAPI"), b"smapi binary").unwrap();

        let result = uninstall(&bundle, &db);

        assert!(result.is_err(), "uninstall must refuse sandboxed bundle");
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("sandboxed"),
            "error must mention 'sandboxed': {msg}"
        );
    }
}
