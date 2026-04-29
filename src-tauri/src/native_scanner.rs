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
            bundle_path: p,
            info,
            architecture: Architecture::Unknown,
            source: NativeSource::SystemApplications,
            sandboxed: false,
        });
    }
    results
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
}
