//! Paralives BepInEx (Unity IL2CPP) detection — read-only.
//!
//! BepInEx is the canonical script-mod runtime for IL2CPP Unity games.
//! Paralives ships a Unity IL2CPP build on macOS Apple Silicon, and the
//! community uses BepInEx 6.x's macOS ARM64 flavor for script mods.
//!
//! This module is DETECTION ONLY. Install and uninstall (which break
//! and restore the .app's Apple Developer ID signature) live in
//! Layer 3 — they are explicit, snapshot-protected, consent-gated
//! operations the user opts into. This module never mutates.

use std::fs;
use std::path::Path;

use serde::Serialize;

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
}
