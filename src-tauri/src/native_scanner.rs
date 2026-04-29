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
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::plist::{read_info_plist, InfoPlist};
use crate::runtime::{Architecture, NativeSource};

/// A `.app` bundle discovered during a scan of the application install
/// locations. Fields that require deeper per-bundle inspection
/// (`architecture`, `sandboxed`) are filled in by later tasks; for now
/// they carry safe zero-value defaults.
#[derive(Clone, Debug, serde::Serialize)]
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

/// Detect the CPU architecture of a Mach-O binary.
///
/// Reads the first ~36 bytes of the file. Recognizes single-arch 64-bit
/// (`FEEDFACF`), fat universal (`CAFEBABE`), and CPU type discriminators
/// for arm64 (`0x0100000C`) and x86_64 (`0x01000007`).
///
/// Returns [`Architecture::Unknown`] for unreadable files, unrecognized
/// magic, or CPU types we don't classify.
fn detect_architecture(executable_path: &Path) -> Architecture {
    const MH_MAGIC_64: u32 = 0xFEED_FACF;
    const MH_CIGAM_64: u32 = 0xCFFA_EDFE; // byte-swapped (LE → BE encoded)
    const FAT_MAGIC: u32 = 0xCAFE_BABE;
    const FAT_CIGAM: u32 = 0xBEBA_FECA;
    const CPU_TYPE_ARM64: u32 = 0x0100_000C;
    const CPU_TYPE_X86_64: u32 = 0x0100_0007;

    let mut file = match fs::File::open(executable_path) {
        Ok(f) => f,
        Err(_) => return Architecture::Unknown,
    };
    // 4 magic + 4 nfat_arch + 2 × 20-byte fat_arch entries = 48 bytes covers
    // both single-arch and typical Universal-2 fat binaries inline.
    let mut buf = [0u8; 48];
    let n = match file.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return Architecture::Unknown,
    };
    if n < 8 {
        return Architecture::Unknown;
    }

    let magic_le = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    let magic_be = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);

    // Single-arch 64-bit (most common case for native modern macOS apps).
    if magic_le == MH_MAGIC_64 || magic_le == MH_CIGAM_64 {
        let cputype = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        return match cputype {
            CPU_TYPE_ARM64 => Architecture::AppleSilicon,
            CPU_TYPE_X86_64 => Architecture::IntelOnly,
            _ => Architecture::Unknown,
        };
    }

    // Fat / universal binary (BE header).
    if magic_be == FAT_MAGIC || magic_be == FAT_CIGAM {
        let nfat_arch = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
        if nfat_arch == 0 || nfat_arch > 16 {
            return Architecture::Unknown; // bogus entry count
        }
        let mut has_arm64 = false;
        let mut has_x86_64 = false;

        // Each fat_arch entry is 20 bytes; first 4 bytes are cputype (BE).
        // With a 48-byte buffer: 8 bytes header + 2 × 20-byte entries = 48,
        // so we can parse up to 2 slices inline without re-reading.
        let max_in_buf = ((n.saturating_sub(8)) / 20).min(nfat_arch as usize);
        for i in 0..max_in_buf {
            let offset = 8 + i * 20;
            if offset + 4 > n {
                break;
            }
            let cputype = u32::from_be_bytes([
                buf[offset],
                buf[offset + 1],
                buf[offset + 2],
                buf[offset + 3],
            ]);
            match cputype {
                CPU_TYPE_ARM64 => has_arm64 = true,
                CPU_TYPE_X86_64 => has_x86_64 = true,
                _ => {}
            }
        }

        // For nfat_arch > what we buffered, read the rest of the table.
        let remaining_slices = nfat_arch as usize - max_in_buf;
        if remaining_slices > 0 {
            let mut more = vec![0u8; remaining_slices * 20];
            if file.read_exact(&mut more).is_ok() {
                for i in 0..remaining_slices {
                    let off = i * 20;
                    if off + 4 > more.len() {
                        break;
                    }
                    let cputype = u32::from_be_bytes([
                        more[off],
                        more[off + 1],
                        more[off + 2],
                        more[off + 3],
                    ]);
                    match cputype {
                        CPU_TYPE_ARM64 => has_arm64 = true,
                        CPU_TYPE_X86_64 => has_x86_64 = true,
                        _ => {}
                    }
                }
            }
        }

        return match (has_arm64, has_x86_64) {
            (true, true) => Architecture::Universal,
            (true, false) => Architecture::AppleSilicon,
            (false, true) => Architecture::IntelOnly,
            (false, false) => Architecture::Unknown,
        };
    }

    Architecture::Unknown
}

/// Resolve a bundle's main executable from `Info.plist`'s
/// `CFBundleExecutable`, then call [`detect_architecture`] on it.
///
/// Returns [`Architecture::Unknown`] if the executable path does not exist
/// or cannot be read.
fn detect_bundle_architecture(bundle: &Path, info: &InfoPlist) -> Architecture {
    let exe = bundle
        .join("Contents")
        .join("MacOS")
        .join(&info.bundle_executable);
    if !exe.exists() {
        return Architecture::Unknown;
    }
    detect_architecture(&exe)
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
/// against them and surface a clear error in the UI (Task 6.2).
pub fn is_sandboxed(bundle: &Path) -> bool {
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

/// Returns `true` if the parsed `Info.plist` category indicates a game.
///
/// Games use `LSApplicationCategoryType` values that start with
/// `"public.app-category.games"` (e.g. `"public.app-category.games"`,
/// `"public.app-category.action-games"`, etc.). Bundles with no category
/// or a non-game category (utilities, developer tools, productivity, etc.)
/// return `false` and are skipped by [`scan_dir`].
fn is_game_category(info: &InfoPlist) -> bool {
    info.category
        .as_deref()
        .map(|cat| cat.starts_with("public.app-category.games"))
        .unwrap_or(false)
}

/// Walk `dir` (non-recursively) for `.app` bundles.
///
/// Only includes bundles whose `Info.plist` `LSApplicationCategoryType`
/// starts with `"public.app-category.games"`. Bundles with no category
/// (VS Code, Claude, etc.) or non-game categories (utilities, developer
/// tools, productivity) are silently skipped — this filter keeps the
/// discovery list limited to actual games.
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
        // Only include bundles with a game-category LSApplicationCategoryType.
        // VS Code, Chrome, Slack, Claude.app, etc. all lack the games category
        // and would otherwise pollute the discovery list.
        if !is_game_category(&info) {
            continue;
        }
        let architecture = detect_bundle_architecture(&p, &info);
        results.push(NativeAppCandidate {
            bundle_path: p.clone(),
            architecture,
            sandboxed: is_sandboxed(&p),
            info,
            source: NativeSource::SystemApplications,
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
                let architecture = detect_bundle_architecture(&bundle, &info);
                results.push(NativeAppCandidate {
                    bundle_path: bundle.clone(),
                    architecture,
                    sandboxed: is_sandboxed(&bundle),
                    info,
                    source: NativeSource::Steam,
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

// ---------------------------------------------------------------------
// GOG mac integration
// ---------------------------------------------------------------------

/// Walk `~/Games` for GOG-style `.app` installs.
///
/// GOG installs on macOS typically follow the convention
/// `~/Games/<Game Name>/<Game Name>.app`. This function discovers all
/// `.app` bundles up to depth 2 inside `~/Games` and returns one
/// [`NativeAppCandidate`] per bundle with `source = NativeSource::Gog`.
///
/// Symlinks are skipped (consistent with `scan_dir`). A missing
/// `~/Games` directory returns an empty `Vec` without panicking.
pub fn scan_gog_mac() -> Vec<NativeAppCandidate> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    scan_gog_mac_at(&home.join("Games"))
}

/// Testable inner: takes the GOG games directory explicitly.
///
/// The default GOG games directory on macOS is `~/Games`. Tests pass a
/// temporary directory here so that the scanner can be exercised without
/// a real GOG installation.
pub(crate) fn scan_gog_mac_at(games_dir: &Path) -> Vec<NativeAppCandidate> {
    let mut results = Vec::new();
    for bundle in find_app_bundles(games_dir, 2) {
        let info_path = bundle.join("Contents").join("Info.plist");
        let Ok(info) = read_info_plist(&info_path) else {
            continue;
        };
        let architecture = detect_bundle_architecture(&bundle, &info);
        let sandboxed = is_sandboxed(&bundle);
        results.push(NativeAppCandidate {
            bundle_path: bundle,
            info,
            architecture,
            source: NativeSource::Gog,
            sandboxed,
        });
    }
    results
}

// ---------------------------------------------------------------------
// Aggregate scanner
// ---------------------------------------------------------------------

/// Aggregate scan: `/Applications` + Steam mac + GOG mac.
///
/// Deduplicates by canonicalized `bundle_path` so that an app discovered via
/// multiple sources appears only once. Priority order: Steam first (most
/// specific), then GOG, then `/Applications` (least specific). The
/// first-discovered source wins when there is a conflict.
///
/// Callers should note that canonicalization requires the paths to exist on
/// disk; if the path cannot be canonicalized the raw path is used for dedup
/// instead (so the guard against duplicates is still best-effort for
/// non-existent paths, which shouldn't occur in production).
pub fn scan_all_native() -> Vec<NativeAppCandidate> {
    let mut seen = std::collections::HashSet::new();
    let mut results = Vec::new();
    // Order matters for dedup — Steam first because it is most specific.
    for batch in [scan_steam_mac(), scan_gog_mac(), scan_applications_dirs()] {
        for cand in batch {
            let key = cand
                .bundle_path
                .canonicalize()
                .unwrap_or_else(|_| cand.bundle_path.clone());
            if seen.insert(key) {
                results.push(cand);
            }
        }
    }
    results
}

// ---------------------------------------------------------------------
// Manual native game add
// ---------------------------------------------------------------------

/// Validate a user-supplied `.app` path and produce a `NativeAppCandidate`.
///
/// Used by the `add_native_game_manually` Tauri command as the testable
/// validation layer. Returns user-readable error strings on failure.
///
/// Checks performed (in order):
/// 1. Path ends with `.app` extension.
/// 2. Path exists and is a directory.
/// 3. `Contents/Info.plist` is present and parseable.
pub fn validate_manual_native_app(app_path: &Path) -> Result<NativeAppCandidate, String> {
    if app_path.extension().map(|e| e != "app").unwrap_or(true) {
        return Err(format!("not a .app bundle: {}", app_path.display()));
    }
    if !app_path.is_dir() {
        return Err(format!(
            "path does not exist or is not a directory: {}",
            app_path.display()
        ));
    }
    let info_path = app_path.join("Contents").join("Info.plist");
    let info = read_info_plist(&info_path)
        .map_err(|e| format!("could not read Info.plist: {}", e))?;
    let architecture = detect_bundle_architecture(app_path, &info);
    let sandboxed = is_sandboxed(app_path);
    if sandboxed {
        return Err(format!(
            "cannot mod a sandboxed app: {}",
            app_path.display()
        ));
    }
    Ok(NativeAppCandidate {
        bundle_path: app_path.to_path_buf(),
        info,
        architecture,
        source: NativeSource::Manual,
        sandboxed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Write a valid XML Info.plist with the given bundle identifier,
    /// executable name, and optional `LSApplicationCategoryType` at `path`.
    fn write_valid_info_plist(path: &Path, identifier: &str, exe: &str) {
        write_info_plist_with_category(path, identifier, exe, Some("public.app-category.games"));
    }

    /// Write a valid XML Info.plist with full control over the category key.
    ///
    /// Pass `Some("public.app-category.games")` for a game bundle, or `None`
    /// to omit the key entirely (simulating a non-game app like VS Code).
    fn write_info_plist_with_category(
        path: &Path,
        identifier: &str,
        exe: &str,
        category: Option<&str>,
    ) {
        let category_xml = match category {
            Some(cat) => format!(
                "    <key>LSApplicationCategoryType</key>\n    <string>{cat}</string>\n"
            ),
            None => String::new(),
        };
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>{identifier}</string>
    <key>CFBundleExecutable</key>
    <string>{exe}</string>
{category_xml}</dict>
</plist>
"#
        );
        fs::write(path, xml).expect("write plist");
    }

    /// Create a minimal `.app` bundle under `dir` with a valid Info.plist
    /// that includes `LSApplicationCategoryType = "public.app-category.games"`.
    fn make_app(dir: &Path, name: &str, identifier: &str) -> PathBuf {
        let bundle = dir.join(format!("{}.app", name));
        let contents = bundle.join("Contents");
        fs::create_dir_all(&contents).expect("mkdir Contents");
        write_valid_info_plist(&contents.join("Info.plist"), identifier, name);
        bundle
    }

    /// Create a `.app` bundle under `dir` whose Info.plist has NO
    /// `LSApplicationCategoryType` key (simulates a non-game app like VS Code).
    fn make_non_game_app(dir: &Path, name: &str, identifier: &str) -> PathBuf {
        let bundle = dir.join(format!("{}.app", name));
        let contents = bundle.join("Contents");
        fs::create_dir_all(&contents).expect("mkdir Contents");
        write_info_plist_with_category(&contents.join("Info.plist"), identifier, name, None);
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

    /// scan_dir must filter out bundles that lack LSApplicationCategoryType
    /// (e.g. VS Code, Claude.app, CrossOver, developer tools). Only the
    /// bundle with a games category should survive.
    #[test]
    fn scan_dir_filters_non_game_apps() {
        let dir = tempfile::tempdir().expect("tempdir");
        // A proper game — has the games category.
        make_app(dir.path(), "RimWorld", "ludeon.rimworld");
        // A non-game app — no LSApplicationCategoryType key at all.
        make_non_game_app(dir.path(), "VSCode", "com.microsoft.vscode");
        // Another non-game app — utility category, not games.
        let bundle = dir.path().join("Terminal.app");
        let contents = bundle.join("Contents");
        fs::create_dir_all(&contents).expect("mkdir");
        write_info_plist_with_category(
            &contents.join("Info.plist"),
            "com.apple.terminal",
            "Terminal",
            Some("public.app-category.utilities"),
        );

        let candidates = scan_dir(dir.path());
        assert_eq!(candidates.len(), 1, "only the game-category bundle should survive");
        assert_eq!(candidates[0].info.bundle_identifier, "ludeon.rimworld");
    }

    /// Positive case: a bundle with an exact game category is returned.
    #[test]
    fn scan_dir_includes_game_category_bundles() {
        let dir = tempfile::tempdir().expect("tempdir");
        make_app(dir.path(), "Balatro", "com.localthunk.balatro");

        let candidates = scan_dir(dir.path());
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].info.bundle_identifier, "com.localthunk.balatro");
        assert_eq!(candidates[0].source, NativeSource::SystemApplications);
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

    // -----------------------------------------------------------------
    // Mach-O architecture detection tests
    // -----------------------------------------------------------------

    fn write_macho_header(path: &Path, bytes: &[u8]) {
        fs::write(path, bytes).expect("write header");
    }

    #[test]
    fn arch_detects_arm64_single() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("test_arm64");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0xFEED_FACFu32.to_le_bytes()); // MH_MAGIC_64
        bytes.extend_from_slice(&0x0100_000Cu32.to_le_bytes()); // cputype = arm64
        bytes.extend(std::iter::repeat(0u8).take(28));
        write_macho_header(&exe, &bytes);
        assert_eq!(detect_architecture(&exe), Architecture::AppleSilicon);
    }

    #[test]
    fn arch_detects_x86_64_single() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("test_x86");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0xFEED_FACFu32.to_le_bytes()); // MH_MAGIC_64
        bytes.extend_from_slice(&0x0100_0007u32.to_le_bytes()); // cputype = x86_64
        bytes.extend(std::iter::repeat(0u8).take(28));
        write_macho_header(&exe, &bytes);
        assert_eq!(detect_architecture(&exe), Architecture::IntelOnly);
    }

    #[test]
    fn arch_detects_universal_arm64_x86_64() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("test_fat");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0xCAFE_BABEu32.to_be_bytes()); // FAT_MAGIC (BE)
        bytes.extend_from_slice(&2u32.to_be_bytes()); // nfat_arch = 2
        // First fat_arch entry (20 bytes): cputype = arm64 (BE)
        bytes.extend_from_slice(&0x0100_000Cu32.to_be_bytes());
        bytes.extend(std::iter::repeat(0u8).take(16));
        // Second fat_arch entry (20 bytes): cputype = x86_64 (BE)
        bytes.extend_from_slice(&0x0100_0007u32.to_be_bytes());
        bytes.extend(std::iter::repeat(0u8).take(16));
        write_macho_header(&exe, &bytes);
        assert_eq!(detect_architecture(&exe), Architecture::Universal);
    }

    #[test]
    fn arch_detects_unknown_for_random_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("not_macho");
        write_macho_header(&exe, b"hello world this is not a macho header\x00\x00");
        assert_eq!(detect_architecture(&exe), Architecture::Unknown);
    }

    #[test]
    fn arch_returns_unknown_for_missing_file() {
        assert_eq!(
            detect_architecture(Path::new("/nonexistent/binary")),
            Architecture::Unknown
        );
    }

    // -----------------------------------------------------------------
    // GOG mac integration tests
    // -----------------------------------------------------------------

    #[test]
    fn scan_gog_mac_finds_apps_in_games_dir() {
        let dir = tempfile::tempdir().unwrap();
        // GOG convention: ~/Games/<Game Name>/<Game Name>.app
        let game1_contents = dir.path().join("Awesome Game/Awesome Game.app/Contents");
        let game2_contents = dir.path().join("Other Game/Other Game.app/Contents");
        fs::create_dir_all(&game1_contents).unwrap();
        fs::create_dir_all(&game2_contents).unwrap();
        write_valid_info_plist(&game1_contents.join("Info.plist"), "com.gog.awesome", "Awesome");
        write_valid_info_plist(&game2_contents.join("Info.plist"), "com.gog.other", "Other");

        let candidates = scan_gog_mac_at(dir.path());
        assert_eq!(candidates.len(), 2, "expected 2 GOG candidates");
        for c in &candidates {
            assert_eq!(c.source, NativeSource::Gog);
        }
    }

    #[test]
    fn scan_gog_mac_returns_empty_for_missing_dir() {
        let candidates = scan_gog_mac_at(Path::new("/nonexistent/very/unlikely"));
        assert!(candidates.is_empty());
    }

    // -----------------------------------------------------------------
    // Manual native game add tests
    // -----------------------------------------------------------------

    #[test]
    fn validate_manual_native_app_rejects_non_app_path() {
        let dir = tempfile::tempdir().unwrap();
        let txt = dir.path().join("readme.txt");
        fs::write(&txt, "hello").unwrap();
        assert!(validate_manual_native_app(&txt).is_err());
    }

    #[test]
    fn validate_manual_native_app_rejects_missing_path() {
        let result = validate_manual_native_app(Path::new("/nonexistent.app"));
        assert!(result.is_err());
    }

    #[test]
    fn validate_manual_native_app_returns_candidate_for_valid_app() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = make_app(dir.path(), "Manual", "com.user.manual");
        let result = validate_manual_native_app(&bundle).expect("should succeed");
        assert_eq!(result.source, NativeSource::Manual);
        assert_eq!(result.info.bundle_identifier, "com.user.manual");
    }

    // -----------------------------------------------------------------
    // Aggregate scanner dedup test
    // -----------------------------------------------------------------

    /// scan_all_native deduplicates when the same bundle_path appears in
    /// multiple source batches. We synthesize this by scanning the same
    /// directory twice via scan_steam_mac_at and scan_gog_mac_at and then
    /// verifying scan_all_native (which calls all three scanners) doesn't
    /// double-count when the underlying scanners return the same canonical
    /// path.
    ///
    /// Implementation note: scan_all_native calls the public top-level
    /// functions (scan_steam_mac, scan_gog_mac, scan_applications_dirs).
    /// Those functions rely on the real home directory and Steam/GOG paths,
    /// so we can't easily inject test roots. Instead, we test the dedup
    /// logic directly by constructing two `NativeAppCandidate` lists that
    /// share a bundle_path and verifying that our dedup key logic works.
    /// The helper below mimics what scan_all_native does internally.
    fn dedup_candidates(batches: Vec<Vec<NativeAppCandidate>>) -> Vec<NativeAppCandidate> {
        let mut seen = std::collections::HashSet::new();
        let mut results = Vec::new();
        for batch in batches {
            for cand in batch {
                let key = cand
                    .bundle_path
                    .canonicalize()
                    .unwrap_or_else(|_| cand.bundle_path.clone());
                if seen.insert(key) {
                    results.push(cand);
                }
            }
        }
        results
    }

    #[test]
    fn scan_all_native_dedupes_overlapping_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = make_app(dir.path(), "SharedGame", "com.example.shared");

        // Simulate the same app being found by both Steam and GOG scanners.
        let steam_cand = NativeAppCandidate {
            bundle_path: bundle.clone(),
            info: crate::plist::read_info_plist(&bundle.join("Contents/Info.plist")).unwrap(),
            architecture: Architecture::Unknown,
            source: NativeSource::Steam,
            sandboxed: false,
        };
        let gog_cand = NativeAppCandidate {
            bundle_path: bundle.clone(),
            info: crate::plist::read_info_plist(&bundle.join("Contents/Info.plist")).unwrap(),
            architecture: Architecture::Unknown,
            source: NativeSource::Gog,
            sandboxed: false,
        };

        let results = dedup_candidates(vec![vec![steam_cand], vec![gog_cand]]);

        assert_eq!(results.len(), 1, "duplicate bundle_path must be deduped");
        // Steam source wins (it was first in priority order).
        assert_eq!(results[0].source, NativeSource::Steam);
        assert_eq!(results[0].info.bundle_identifier, "com.example.shared");
    }

    // -----------------------------------------------------------------
    // validate_manual_native_app sandbox refusal tests (Task 6.2)
    // -----------------------------------------------------------------

    /// `validate_manual_native_app` must return an error for a bundle that
    /// carries the `_MASReceipt/receipt` marker (Mac App Store sandboxed app).
    /// The error message must mention "sandboxed".
    #[test]
    fn validate_manual_native_app_refuses_sandboxed() {
        let dir = tempfile::tempdir().unwrap();

        // Create a valid .app bundle with a MAS receipt.
        let bundle = make_app(dir.path(), "SandboxedGame", "com.example.sandboxed");
        let receipt_dir = bundle.join("Contents/_MASReceipt");
        fs::create_dir_all(&receipt_dir).unwrap();
        fs::write(receipt_dir.join("receipt"), b"fake receipt").unwrap();

        let result = validate_manual_native_app(&bundle);

        assert!(
            result.is_err(),
            "validate_manual_native_app must refuse sandboxed bundle"
        );
        let msg = result.unwrap_err();
        assert!(
            msg.contains("sandboxed"),
            "error must mention 'sandboxed': {msg}"
        );
    }
}
