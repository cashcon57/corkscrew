//! Discovery of native macOS games.
//!
//! Walks the standard application install locations (`/Applications`,
//! `~/Applications`) and returns a `Vec<NativeAppCandidate>`. Per-game
//! native plugins (Stardew, BG3, etc.) consume this and filter for their
//! bundle identifier.
//!
//! Additional locations (Steam mac libraries, GOG mac, Mac App Store) will
//! be added by later tasks in the native-mode work stream.
//!
//! Module is read-only — it does NOT mutate any files. The native deploy
//! logic that DOES mutate bundles lives in per-game plugins (Task 3.7+).

use std::fs;
use std::path::{Path, PathBuf};

use crate::plist::{read_info_plist, InfoPlist};
use crate::runtime::{Architecture, NativeSource};

/// A `.app` bundle discovered during a scan of the application install
/// locations. Fields that require deeper per-bundle inspection
/// (`architecture`, `sandboxed`) are filled in by later tasks; for now
/// they carry safe zero-value defaults.
#[derive(Clone, Debug)]
pub struct NativeAppCandidate {
    /// Absolute path to the `.app` bundle directory.
    pub bundle_path: PathBuf,
    /// Parsed `Contents/Info.plist` for this bundle.
    pub info: InfoPlist,
    /// CPU architecture of the primary game binary.
    /// Set to [`Architecture::Unknown`] until Task 2.5 fills this in.
    pub architecture: Architecture,
    /// How this candidate was discovered.
    pub source: NativeSource,
    /// Whether the app runs inside the App Sandbox.
    /// `false` until Task 2.4 fills this in via entitlement inspection.
    pub sandboxed: bool,
}

/// Detect whether a bundle is Mac App Store sandboxed.
///
/// Two indicators:
/// 1. Presence of `Contents/_MASReceipt/receipt` — the App Store
///    cryptographic receipt. Definitive.
/// 2. Path under `/System/Applications` — Apple's system apps
///    (Mail.app, Safari.app, etc.) are also sandboxed.
///
/// Sandboxed apps cannot be modded — Corkscrew must refuse to deploy
/// against them and surface a clear error in the UI (Task 6.3).
fn is_sandboxed(bundle: &Path) -> bool {
    if bundle.starts_with("/System/Applications") {
        return true;
    }
    bundle.join("Contents/_MASReceipt/receipt").exists()
}

/// Walk `/Applications` and `~/Applications` for `.app` bundles.
///
/// Returns one [`NativeAppCandidate`] per discovered bundle that has a
/// readable `Contents/Info.plist`. Bundles whose plist is missing or
/// malformed are silently skipped — many entries in `/Applications` are
/// not standard game bundles.
///
/// Symlinks are skipped unconditionally; `~/Applications` sometimes
/// contains symlinks into versioned dirs, a problem deferred to the
/// aggregator task (Task 2.7).
pub fn scan_applications_dirs() -> Vec<NativeAppCandidate> {
    let mut dirs: Vec<PathBuf> = vec![PathBuf::from("/Applications")];
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join("Applications"));
    }
    let mut results = Vec::new();
    for d in dirs {
        results.extend(scan_dir(&d));
    }
    results
}

/// Walk `dir` (non-recursively) for `.app` bundles.
///
/// Skips symlinks and any bundle whose `Contents/Info.plist` is missing
/// or malformed. A missing `dir` itself returns an empty `Vec` without
/// panicking — callers should not assume the directory exists.
pub(crate) fn scan_dir(dir: &Path) -> Vec<NativeAppCandidate> {
    let mut results = Vec::new();
    let read = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return results, // missing or unreadable dir → empty result
    };
    for entry in read.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let p = entry.path();
        if p.extension().is_none_or(|e| e != "app") {
            continue;
        }
        let info_path = p.join("Contents").join("Info.plist");
        let Ok(info) = read_info_plist(&info_path) else {
            continue;
        };
        results.push(NativeAppCandidate {
            bundle_path: p.clone(),
            info,
            architecture: Architecture::Unknown,
            source: NativeSource::SystemApplications,
            sandboxed: is_sandboxed(&p),
        });
    }
    results
}

// ---------------------------------------------------------------------
// Steam mac integration
// ---------------------------------------------------------------------

/// Walk Steam's macOS install for native games.
///
/// Reads `~/Library/Application Support/Steam/steamapps/libraryfolders.vdf`
/// to discover all library roots, then scans each library's
/// `steamapps/appmanifest_*.acf` files to find installed games. For each
/// game, resolves the install directory under
/// `<library>/steamapps/common/<installdir>` and descends up to 2 levels
/// looking for `.app` bundles. Each found bundle becomes a
/// [`NativeAppCandidate`] with `source = NativeSource::Steam`.
///
/// Symlinks are skipped unconditionally (consistent with `scan_dir`).
/// Missing or unreadable files are silently skipped — callers should not
/// assume Steam is installed.
pub fn scan_steam_mac() -> Vec<NativeAppCandidate> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    scan_steam_mac_at(
        &home
            .join("Library")
            .join("Application Support")
            .join("Steam"),
    )
}

/// Testable inner: takes the Steam root explicitly.
///
/// The default Steam root on macOS is
/// `~/Library/Application Support/Steam`. Tests pass a temporary
/// directory here so that the scanner can be exercised without a real
/// Steam installation.
pub(crate) fn scan_steam_mac_at(steam_root: &Path) -> Vec<NativeAppCandidate> {
    // Always include steam_root itself as the first library (it is its own
    // primary library). Additional libraries are discovered via libraryfolders.vdf.
    // Deduplicate by canonical path so that libraryfolders.vdf entries pointing
    // at the same directory as steam_root don't cause double-scanning.
    let extra_libraries =
        parse_libraryfolders(&steam_root.join("steamapps").join("libraryfolders.vdf"));
    let mut seen = std::collections::HashSet::new();
    let mut all_libs: Vec<PathBuf> = Vec::new();
    for lib in std::iter::once(steam_root.to_path_buf()).chain(extra_libraries) {
        // Use the canonicalized path for dedup; fall back to the raw path if
        // canonicalization fails (e.g. library path doesn't exist yet).
        let key = lib.canonicalize().unwrap_or_else(|_| lib.clone());
        if seen.insert(key) {
            all_libs.push(lib);
        }
    }

    let mut results = Vec::new();
    for lib in all_libs {
        let steamapps = lib.join("steamapps");
        if !steamapps.is_dir() {
            continue;
        }
        let read = match fs::read_dir(&steamapps) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for entry in read.flatten() {
            let p = entry.path();
            let Some(file_name) = p.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if !file_name.starts_with("appmanifest_") || !file_name.ends_with(".acf") {
                continue;
            }
            let Some(manifest) = parse_appmanifest(&p) else {
                continue;
            };
            let install_root = steamapps.join("common").join(&manifest.installdir);
            for bundle in find_app_bundles(&install_root, 2) {
                let info_path = bundle.join("Contents").join("Info.plist");
                let Ok(info) = read_info_plist(&info_path) else {
                    continue;
                };
                results.push(NativeAppCandidate {
                    bundle_path: bundle.clone(),
                    info,
                    architecture: Architecture::Unknown,
                    source: NativeSource::Steam,
                    sandboxed: is_sandboxed(&bundle),
                });
            }
        }
    }
    results
}

// ---------------------------------------------------------------------------
// VDF / ACF parsers — minimal line-based; Steam's key/value files on macOS
// use ASCII with no quote escaping in the path or name fields we care about.
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct SteamAppManifest {
    #[allow(dead_code)]
    appid: String,
    #[allow(dead_code)]
    name: String,
    installdir: String,
}

/// Extract the list of additional Steam library paths from `libraryfolders.vdf`.
///
/// The VDF format used here is the legacy key-value text format (not JSON).
/// We only need lines of the form `"path"    "/some/path"` — a simple line
/// scan is sufficient and avoids pulling in a full VDF parser.
fn parse_libraryfolders(path: &Path) -> Vec<PathBuf> {
    let Ok(contents) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in contents.lines() {
        let line = line.trim();
        // Match lines like: "path"    "/Users/foo/Library/Application Support/Steam"
        if let Some(rest) = line.strip_prefix(r#""path""#) {
            let trimmed = rest.trim();
            if let Some(value) = trimmed.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                if !value.is_empty() {
                    out.push(PathBuf::from(value));
                }
            }
        }
    }
    out
}

/// Parse a single `appmanifest_<appid>.acf` file.
///
/// Returns `None` if the file is unreadable or any of the three required
/// fields (`appid`, `name`, `installdir`) are missing.
fn parse_appmanifest(path: &Path) -> Option<SteamAppManifest> {
    let contents = fs::read_to_string(path).ok()?;
    let mut appid = None;
    let mut name = None;
    let mut installdir = None;
    for line in contents.lines() {
        let line = line.trim();
        if let Some(v) = parse_kv(line, "appid") {
            appid = Some(v);
        } else if let Some(v) = parse_kv(line, "name") {
            name = Some(v);
        } else if let Some(v) = parse_kv(line, "installdir") {
            installdir = Some(v);
        }
    }
    Some(SteamAppManifest {
        appid: appid?,
        name: name?,
        installdir: installdir?,
    })
}

/// Extract the value from a VDF/ACF key-value line of the form:
/// `"<key>"    "<value>"`.
///
/// Returns `None` if the line does not match the expected pattern for
/// `key`, or if the value is not properly quoted.
fn parse_kv(line: &str, key: &str) -> Option<String> {
    let prefix = format!(r#""{key}""#);
    let rest = line.strip_prefix(&prefix)?;
    let trimmed = rest.trim();
    let value = trimmed.strip_prefix('"')?.strip_suffix('"')?;
    Some(value.to_string())
}

/// Recursively find `.app` bundles inside `root`, up to `max_depth` levels
/// deep.
///
/// Symlinks are skipped (consistent with `scan_dir`). Descends into
/// subdirectories but stops recursing once a `.app` is found — there should
/// never be a game inside a `.app` inside this context, and descending into
/// bundle internals would be wasteful.
fn find_app_bundles(root: &Path, max_depth: usize) -> Vec<PathBuf> {
    fn recurse(p: &Path, depth: usize, max_depth: usize, out: &mut Vec<PathBuf>) {
        if depth > max_depth {
            return;
        }
        let Ok(read) = fs::read_dir(p) else {
            return;
        };
        for entry in read.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            // Symlinks skipped unconditionally — consistent with scan_dir.
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "app") {
                out.push(path);
                // Don't descend into a .app bundle's internals.
                continue;
            }
            if file_type.is_dir() {
                recurse(&path, depth + 1, max_depth, out);
            }
        }
    }
    let mut out = Vec::new();
    recurse(root, 0, max_depth, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Write a valid XML Info.plist with the given bundle identifier and
    /// executable name at `path`.
    fn write_valid_info_plist(path: &Path, identifier: &str, exe: &str) {
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>{identifier}</string>
    <key>CFBundleExecutable</key>
    <string>{exe}</string>
</dict>
</plist>
"#
        );
        fs::write(path, xml).expect("write plist");
    }

    /// Create a minimal `.app` bundle under `dir` with a valid Info.plist.
    fn make_app(dir: &Path, name: &str, identifier: &str) -> PathBuf {
        let bundle = dir.join(format!("{}.app", name));
        let contents = bundle.join("Contents");
        fs::create_dir_all(&contents).expect("mkdir Contents");
        write_valid_info_plist(&contents.join("Info.plist"), identifier, name);
        bundle
    }

    #[test]
    fn scan_dir_finds_app_bundles() {
        let dir = tempfile::tempdir().expect("tempdir");
        make_app(dir.path(), "Alpha", "com.example.alpha");
        make_app(dir.path(), "Beta", "com.example.beta");
        // A plain directory without `.app` extension should be ignored.
        fs::create_dir_all(dir.path().join("not-an-app")).unwrap();

        let candidates = scan_dir(dir.path());
        assert_eq!(candidates.len(), 2);

        let ids: Vec<&str> = candidates
            .iter()
            .map(|c| c.info.bundle_identifier.as_str())
            .collect();
        assert!(ids.contains(&"com.example.alpha"));
        assert!(ids.contains(&"com.example.beta"));

        // Defaults — populated by later tasks.
        for c in &candidates {
            assert_eq!(c.architecture, Architecture::Unknown);
            assert_eq!(c.source, NativeSource::SystemApplications);
            assert!(!c.sandboxed);
        }
    }

    #[test]
    fn scan_dir_skips_apps_without_info_plist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bundle = dir.path().join("NoPlist.app");
        // Create the bundle dir structure but omit the Info.plist file.
        fs::create_dir_all(bundle.join("Contents")).unwrap();

        assert!(scan_dir(dir.path()).is_empty());
    }

    #[test]
    fn scan_dir_skips_apps_with_malformed_info_plist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bundle = dir.path().join("Bad.app");
        let contents = bundle.join("Contents");
        fs::create_dir_all(&contents).unwrap();
        // Write content that is definitely not a plist.
        fs::write(contents.join("Info.plist"), "definitely not a plist").unwrap();

        assert!(scan_dir(dir.path()).is_empty());
    }

    #[test]
    fn scan_dir_returns_empty_for_missing_dir() {
        let result = scan_dir(Path::new("/nonexistent/very/unlikely/to/exist"));
        assert!(result.is_empty());
    }

    // -----------------------------------------------------------------
    // Steam mac integration tests
    // -----------------------------------------------------------------

    /// Happy path: scan_steam_mac_at finds a game whose install dir
    /// contains a properly-structured .app bundle.
    #[test]
    fn scan_steam_mac_finds_native_app_in_library() {
        let dir = tempfile::tempdir().expect("tempdir");
        let steam_root = dir.path();
        let steamapps = steam_root.join("steamapps");
        fs::create_dir_all(&steamapps).unwrap();

        // libraryfolders.vdf — references steam_root itself (the scanner
        // always prepends steam_root, so this exercises duplicate-path
        // tolerance, but the path entry being steam_root is fine too).
        fs::write(
            steamapps.join("libraryfolders.vdf"),
            format!(
                r#""libraryfolders"
{{
    "0"
    {{
        "path"      "{}"
    }}
}}
"#,
                steam_root.display()
            ),
        )
        .unwrap();

        // appmanifest for Stardew Valley (appid 413150).
        fs::write(
            steamapps.join("appmanifest_413150.acf"),
            r#""AppState"
{
    "appid"        "413150"
    "name"         "Stardew Valley"
    "installdir"   "Stardew Valley"
}
"#,
        )
        .unwrap();

        // Create the .app bundle with a valid Info.plist.
        let install = steamapps.join("common").join("Stardew Valley");
        let bundle = install.join("Stardew Valley.app").join("Contents");
        fs::create_dir_all(&bundle).unwrap();
        write_valid_info_plist(
            &bundle.join("Info.plist"),
            "com.chucklefish.stardewvalley",
            "StardewValley",
        );

        let candidates = scan_steam_mac_at(steam_root);
        assert_eq!(candidates.len(), 1, "expected exactly one candidate");
        assert_eq!(candidates[0].source, NativeSource::Steam);
        assert_eq!(
            candidates[0].info.bundle_identifier,
            "com.chucklefish.stardewvalley"
        );
        assert_eq!(candidates[0].architecture, Architecture::Unknown);
        assert!(!candidates[0].sandboxed);
    }

    /// A library with an appmanifest but no .app bundle inside the install
    /// dir should yield no candidates.
    #[test]
    fn scan_steam_mac_skips_library_with_no_apps() {
        let dir = tempfile::tempdir().expect("tempdir");
        let steam_root = dir.path();
        let steamapps = steam_root.join("steamapps");

        // Create the install dir without any .app inside it.
        fs::create_dir_all(steamapps.join("common").join("SomeGame")).unwrap();

        fs::write(
            steamapps.join("libraryfolders.vdf"),
            r#""libraryfolders" { "0" { "path" "" } }"#,
        )
        .unwrap();

        fs::write(
            steamapps.join("appmanifest_123.acf"),
            r#""AppState" { "appid" "123" "name" "X" "installdir" "SomeGame" }"#,
        )
        .unwrap();

        let candidates = scan_steam_mac_at(steam_root);
        assert!(
            candidates.is_empty(),
            "no .app bundles → should return empty Vec"
        );
    }

    /// When libraryfolders.vdf is absent (Steam not installed) the scanner
    /// returns an empty Vec without panicking.
    #[test]
    fn scan_steam_mac_handles_missing_libraryfolders_gracefully() {
        let dir = tempfile::tempdir().expect("tempdir");
        // The temp dir exists but has no steamapps/ subdirectory at all.
        let candidates = scan_steam_mac_at(dir.path());
        assert!(
            candidates.is_empty(),
            "missing libraryfolders.vdf → should return empty Vec"
        );
    }

    // -----------------------------------------------------------------
    // Sandbox detection tests
    // -----------------------------------------------------------------

    #[test]
    fn is_sandboxed_returns_true_for_app_with_mas_receipt() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bundle = dir.path().join("MASApp.app");
        fs::create_dir_all(bundle.join("Contents/_MASReceipt")).unwrap();
        fs::write(bundle.join("Contents/_MASReceipt/receipt"), b"fake receipt").unwrap();
        assert!(is_sandboxed(&bundle));
    }

    #[test]
    fn is_sandboxed_returns_false_for_app_without_mas_receipt() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bundle = dir.path().join("Plain.app");
        fs::create_dir_all(bundle.join("Contents")).unwrap();
        assert!(!is_sandboxed(&bundle));
    }

    #[test]
    fn is_sandboxed_returns_true_for_system_applications_path() {
        // Path-based; we don't need the file to exist for this branch.
        let p = Path::new("/System/Applications/Mail.app");
        assert!(is_sandboxed(p));
    }

    #[test]
    fn scan_dir_marks_mas_apps_sandboxed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let plain = make_app(dir.path(), "Plain", "com.example.plain");
        let mas = make_app(dir.path(), "MASApp", "com.example.mas");
        fs::create_dir_all(mas.join("Contents/_MASReceipt")).unwrap();
        fs::write(mas.join("Contents/_MASReceipt/receipt"), b"r").unwrap();

        let candidates = scan_dir(dir.path());
        assert_eq!(candidates.len(), 2);
        let plain_c = candidates.iter().find(|c| c.bundle_path == plain).unwrap();
        let mas_c = candidates.iter().find(|c| c.bundle_path == mas).unwrap();
        assert!(!plain_c.sandboxed);
        assert!(mas_c.sandboxed);
    }
}
