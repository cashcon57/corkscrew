//! Baldur's Gate 3 Script Extender (BG3SE) detection.
//!
//! Norbyte's BG3SE is the C++ runtime that enables many advanced BG3
//! mods (gameplay scripting, UI hooks, etc.). On Windows it ships as
//! a DLL injection (`DWrite.dll`). On macOS the build lags Win and
//! has historically been distributed as `bg3se.dylib` /
//! `libbg3se.dylib` dropped into the .app bundle's `Contents/MacOS/`.
//!
//! This module is read-only — install / upgrade is out of scope for
//! Phase 4. Frontend uses `get_bg3se_status` to surface a "BG3SE not
//! installed" or "outdated" warning.
//!
//! See also: `plugins/baldurs_gate_3_native.rs` for the BG3 native
//! game plugin. BG3SE detection is intentionally separated here so it
//! can be called without a full game-plugin context.

use std::fs;
use std::path::Path;

use serde::Serialize;

/// Detection result for the BG3 Script Extender in a given .app bundle.
///
/// `mac_supported` distinguishes between a proper macOS `.dylib` (true)
/// and a mis-dropped Windows `DWrite.dll` (false) that cannot load on mac.
#[derive(Clone, Debug, Serialize)]
pub struct Bg3seStatus {
    /// True only when a mac-compatible BG3SE loader was found.
    pub installed: bool,
    /// Absolute path to the detected loader dylib, if present.
    pub loader_path: Option<String>,
    /// Version string read from the installer-written version file, if present.
    pub version: Option<String>,
    /// True when a macOS-native `.dylib` exists; false when only
    /// `DWrite.dll` was found (Windows variant, won't run on mac)
    /// or when nothing at all was found.
    pub mac_supported: bool,
}

/// Scan `app_bundle` for a BG3 Script Extender installation and return
/// the detected status.
///
/// # Detection logic
///
/// 1. Reads `<bundle>/Contents/MacOS/` for any file whose lowercased name
///    contains `"bg3se"` and ends with `".dylib"` — covers `bg3se.dylib`,
///    `libbg3se.dylib`, `libbg3se.0.dylib`, etc.
/// 2. Also records whether `DWrite.dll` is present (Windows variant).
/// 3. If a `.dylib` match is found: `installed = true`, `mac_supported = true`.
/// 4. If only `DWrite.dll` is present: `installed = false`, `mac_supported = false`
///    — the caller should surface a "you've installed the Windows version" warning.
/// 5. If nothing is found: `installed = false`, `mac_supported = true`
///    (mac is potentially supportable; BG3SE just isn't installed yet).
///
/// Optionally reads a version string from well-known installer-written files.
pub fn detect(app_bundle: &Path) -> Bg3seStatus {
    let macos = app_bundle.join("Contents/MacOS");
    let mut found_dylib: Option<std::path::PathBuf> = None;
    let mut found_dwrite = false;

    if let Ok(read) = fs::read_dir(&macos) {
        for entry in read.flatten() {
            let name = entry.file_name();
            let lower = name.to_string_lossy().to_lowercase();
            if lower.contains("bg3se") && lower.ends_with(".dylib") {
                found_dylib = Some(entry.path());
                break;
            }
            if lower == "dwrite.dll" {
                found_dwrite = true;
            }
        }
    }

    let version = read_version(&macos);

    if let Some(path) = found_dylib {
        Bg3seStatus {
            installed: true,
            loader_path: Some(path.display().to_string()),
            version,
            mac_supported: true,
        }
    } else if found_dwrite {
        Bg3seStatus {
            installed: false, // Windows-only DLL; won't load on macOS
            loader_path: None,
            version: None,
            mac_supported: false,
        }
    } else {
        Bg3seStatus {
            installed: false,
            loader_path: None,
            version: None,
            mac_supported: true, // mac potentially supportable; BG3SE just not present
        }
    }
}

/// Look for a version file written by the BG3SE installer alongside the
/// dylib. Returns the first non-empty line from the first candidate that
/// exists.
///
/// Known locations (subject to change as Norbyte's installer evolves):
/// - `Contents/MacOS/ScriptExtender/version.txt`
/// - `Contents/MacOS/bg3se/version.txt`
/// - `Contents/MacOS/BG3SE_VERSION`
fn read_version(macos: &Path) -> Option<String> {
    let candidates = [
        macos.join("ScriptExtender/version.txt"),
        macos.join("bg3se/version.txt"),
        macos.join("BG3SE_VERSION"),
    ];
    for p in &candidates {
        if let Ok(s) = fs::read_to_string(p) {
            let trimmed = s.lines().next().map(|l| l.trim().to_string()).filter(|s| !s.is_empty());
            if trimmed.is_some() {
                return trimmed;
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Install / uninstall stubs (RESEARCH BLOCKER)
// ---------------------------------------------------------------------------
//
// Norbyte's macOS BG3SE build has historically been distributed as a `.dylib`
// dropped into the `.app` bundle's `Contents/MacOS/` directory. The exact
// install layout for the current release — which subdirectory, which file
// names, whether a sibling `ScriptExtender/` directory holds the runtime
// config — has not been independently verified against an upstream macOS
// release at the time of writing.
//
// Per the project's honesty-first rule, we surface this as a research
// blocker rather than ship an install path that could corrupt the user's
// bundle. The Tauri commands [`crate::commands::native_cmds::install_bg3se`]
// / `uninstall_bg3se` wrap these stubs and return the same error to the UI.
//
// When the install layout is verified (see
// `docs/superpowers/plans/2026-04-28-native-macos-game-support-smapi-install-spec.md`
// for the analogous SMAPI spec — BG3SE needs an equivalent doc before the
// stubs become real implementations), replace the bodies below with the
// real Mach-O validation + dylib placement logic and remove the BLOCKER
// guard.

/// Error returned by [`install`] / [`uninstall`] while BG3SE install support
/// is a research blocker. The message is intentionally explicit so the UI
/// can surface it verbatim without surprising the user.
#[cfg(target_os = "macos")]
pub const BG3SE_INSTALL_BLOCKER: &str =
    "BG3SE install path verification pending — manual install required";

/// Install BG3SE into a Baldur's Gate 3 `.app` bundle.
///
/// **Currently a stub.** Returns [`BG3SE_INSTALL_BLOCKER`] because the
/// upstream install layout has not been verified for the current macOS
/// release. The Mach-O arm64 fetch + validation pipeline is intentionally
/// not run so the failure happens before any HTTP traffic.
///
/// macOS-only. On non-macOS this function does not exist (Linux builds skip
/// it via `#[cfg]`).
#[cfg(target_os = "macos")]
pub fn install(
    _app_bundle: &std::path::Path,
    _db: &std::sync::Arc<crate::database::ModDatabase>,
) -> Result<(), String> {
    // TODO: implement real install once the upstream macOS layout is verified.
    //
    // 1. Fetch latest BG3SE release from `Norbyte/bg3se` (macOS asset).
    // 2. Validate the dylib is a Mach-O arm64 binary (reject FAT-only / x86).
    // 3. Snapshot the bundle via `rollback::create_native_snapshot`.
    // 4. Drop dylib into the verified install location (TBD — DO NOT GUESS).
    // 5. Clear the `com.apple.quarantine` xattr on the bundle.
    Err(BG3SE_INSTALL_BLOCKER.to_string())
}

/// Uninstall BG3SE from a Baldur's Gate 3 `.app` bundle.
///
/// **Currently a stub.** Returns [`BG3SE_INSTALL_BLOCKER`] for symmetry
/// with [`install`] — until install is verified, we cannot guarantee that
/// uninstall removes the right files. Safer to return blocker than to
/// silently no-op or, worse, delete unrelated files.
///
/// macOS-only.
#[cfg(target_os = "macos")]
pub fn uninstall(
    _app_bundle: &std::path::Path,
    _db: &std::sync::Arc<crate::database::ModDatabase>,
) -> Result<(), String> {
    Err(BG3SE_INSTALL_BLOCKER.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Create `<tmpdir>/Baldurs Gate 3.app/Contents/MacOS/` and return the
    /// `MacOS` path. The bundle root is `<tmpdir>/Baldurs Gate 3.app`.
    fn make_macos(dir: &Path) -> std::path::PathBuf {
        let macos = dir.join("Baldurs Gate 3.app/Contents/MacOS");
        fs::create_dir_all(&macos).unwrap();
        macos
    }

    #[test]
    fn detect_returns_not_installed_for_vanilla_bundle() {
        let dir = tempfile::tempdir().unwrap();
        make_macos(dir.path());
        let bundle = dir.path().join("Baldurs Gate 3.app");
        let status = detect(&bundle);
        assert!(!status.installed);
        assert!(status.loader_path.is_none());
        assert!(status.version.is_none());
        // mac is supportable; BG3SE just isn't present
        assert!(status.mac_supported);
    }

    #[test]
    fn detect_returns_installed_for_dylib_present() {
        let dir = tempfile::tempdir().unwrap();
        let macos = make_macos(dir.path());
        fs::write(macos.join("bg3se.dylib"), b"fake dylib content").unwrap();
        let bundle = dir.path().join("Baldurs Gate 3.app");
        let status = detect(&bundle);
        assert!(status.installed);
        assert!(status.mac_supported);
        assert!(status.loader_path.is_some());
        let loader = status.loader_path.unwrap();
        assert!(loader.ends_with("bg3se.dylib"));
    }

    #[test]
    fn detect_handles_libbg3se_naming_variant() {
        let dir = tempfile::tempdir().unwrap();
        let macos = make_macos(dir.path());
        fs::write(macos.join("libbg3se.dylib"), b"fake dylib content").unwrap();
        let bundle = dir.path().join("Baldurs Gate 3.app");
        let status = detect(&bundle);
        assert!(status.installed);
        assert!(status.mac_supported);
        assert!(status.loader_path.is_some());
    }

    #[test]
    fn detect_returns_unsupported_for_dwrite_dll_only() {
        let dir = tempfile::tempdir().unwrap();
        let macos = make_macos(dir.path());
        // DWrite.dll is the Windows DLL-hijacking variant; won't load on mac
        fs::write(macos.join("DWrite.dll"), b"win-only pe binary").unwrap();
        let bundle = dir.path().join("Baldurs Gate 3.app");
        let status = detect(&bundle);
        assert!(!status.installed);
        assert!(!status.mac_supported);
        assert!(status.loader_path.is_none());
    }

    #[test]
    fn detect_reads_version_file_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let macos = make_macos(dir.path());
        fs::write(macos.join("bg3se.dylib"), b"fake dylib").unwrap();
        fs::create_dir_all(macos.join("ScriptExtender")).unwrap();
        fs::write(macos.join("ScriptExtender/version.txt"), "v0.5.0\n").unwrap();
        let bundle = dir.path().join("Baldurs Gate 3.app");
        let status = detect(&bundle);
        assert!(status.installed);
        assert_eq!(status.version.as_deref(), Some("v0.5.0"));
    }
}
