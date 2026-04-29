//! Minimal Info.plist reader for macOS .app bundles.
//!
//! Extracts the bundle identifier, executable name, version, and
//! application category. Handles both XML and binary plist formats
//! transparently — the `plist` crate auto-detects.
//!
//! Used by `native_scanner` to identify discovered .app bundles. We do
//! not need the full Info.plist surface; only the four fields above
//! drive game detection (bundle identifier matches per-game plugin
//! filters; executable name resolves the binary path; category type
//! lets us prefer game-category bundles when surfacing discovery
//! results to the user).

use std::path::Path;

use serde::Deserialize;
use thiserror::Error;

#[derive(Clone, Debug, Deserialize)]
pub struct InfoPlist {
    #[serde(rename = "CFBundleIdentifier")]
    pub bundle_identifier: String,
    #[serde(rename = "CFBundleExecutable")]
    pub bundle_executable: String,
    #[serde(rename = "CFBundleShortVersionString", default)]
    pub short_version: Option<String>,
    #[serde(rename = "LSApplicationCategoryType", default)]
    pub category: Option<String>,
}

#[derive(Debug, Error)]
pub enum PlistError {
    #[error("plist file not found: {0}")]
    NotFound(String),

    #[error("malformed plist: {0}")]
    Malformed(String),
}

/// Read an Info.plist file and extract the four supported keys.
///
/// Returns `PlistError::NotFound` if `path` does not exist, or
/// `PlistError::Malformed` if the file cannot be parsed as a plist
/// (XML or binary) or is missing one of the required keys
/// (`CFBundleIdentifier`, `CFBundleExecutable`).
pub fn read_info_plist(path: &Path) -> Result<InfoPlist, PlistError> {
    if !path.exists() {
        return Err(PlistError::NotFound(path.display().to_string()));
    }
    plist::from_file(path).map_err(|e| PlistError::Malformed(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_path() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/Info.xml.plist")
    }

    #[test]
    fn reads_xml_plist_fixture() {
        let info = read_info_plist(&fixture_path()).expect("fixture should parse");
        assert_eq!(info.bundle_identifier, "com.example.testapp");
        assert_eq!(info.bundle_executable, "TestApp");
        assert_eq!(info.short_version.as_deref(), Some("1.0.0"));
        assert_eq!(info.category.as_deref(), Some("public.app-category.games"));
    }

    #[test]
    fn missing_file_returns_not_found() {
        let path = std::path::Path::new("/nonexistent/Info.plist");
        match read_info_plist(path) {
            Err(PlistError::NotFound(_)) => {}
            other => panic!("expected NotFound, got {:?}", other),
        }
    }

    #[test]
    fn malformed_plist_returns_malformed() {
        // Write a non-plist file to a tempfile.
        let tmp = tempfile::NamedTempFile::new().expect("temp file");
        std::fs::write(tmp.path(), "this is not a plist").expect("write");
        match read_info_plist(tmp.path()) {
            Err(PlistError::Malformed(_)) => {}
            other => panic!("expected Malformed, got {:?}", other),
        }
    }

    #[test]
    fn missing_required_key_returns_malformed() {
        // A plist that's valid XML but missing CFBundleIdentifier.
        let tmp = tempfile::NamedTempFile::new().expect("temp file");
        std::fs::write(
            tmp.path(),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>NoIdentifier</string>
</dict>
</plist>
"#,
        )
        .expect("write");
        match read_info_plist(tmp.path()) {
            Err(PlistError::Malformed(_)) => {}
            other => panic!("expected Malformed (missing required key), got {:?}", other),
        }
    }
}
