//! Proton version detection and management for Linux.
//!
//! Scans three tiers of Proton installations:
//! 1. Steam's `steamapps/common/Proton*` (official Valve releases)
//! 2. User `~/.steam/root/compatibilitytools.d/` (GE-Proton, custom builds)
//! 3. Flatpak Steam paths
//!
//! Parses version strings from directory names covering all major naming
//! conventions: GE-Proton10-27, Proton 10.0, Proton-10.0, EM-10.0-33,
//! CachyOS-Proton-10-27, etc.

use log::{debug, info};
use once_cell::sync::Lazy;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static PROTON_CACHE: Lazy<Mutex<Option<Vec<ProtonVersion>>>> = Lazy::new(|| Mutex::new(None));

/// A detected Proton installation.
#[derive(Debug, Clone, Serialize)]
pub struct ProtonVersion {
    /// Display name (e.g., "GE-Proton10-27", "Proton 10.0")
    pub name: String,
    /// Path to the Proton installation directory
    pub path: PathBuf,
    /// Path to the wine binary within this Proton
    pub wine_bin: PathBuf,
    /// Parsed major version number (e.g., 10 for "Proton 10.0")
    pub major: u32,
    /// Parsed minor version number (e.g., 0 for "Proton 10.0", 27 for "GE-Proton10-27")
    pub minor: u32,
    /// The variant/flavor (Official, GE, CachyOS, EM, etc.)
    pub variant: ProtonVariant,
    /// Whether this meets the minimum recommended version
    pub is_recommended: bool,
}

/// Proton variant/flavor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ProtonVariant {
    /// Official Valve Proton
    Official,
    /// GloriousEggroll custom build
    GE,
    /// CachyOS custom build
    CachyOS,
    /// EM (experimental/community)
    EM,
    /// Unknown/other custom build
    Custom(String),
}

impl std::fmt::Display for ProtonVariant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Official => write!(f, "Official"),
            Self::GE => write!(f, "GE"),
            Self::CachyOS => write!(f, "CachyOS"),
            Self::EM => write!(f, "EM"),
            Self::Custom(s) => write!(f, "{}", s),
        }
    }
}

/// Minimum Proton major version for general game compatibility.
const MIN_PROTON_MAJOR: u32 = 9;
/// Recommended Proton major version.
const RECOMMENDED_PROTON_MAJOR: u32 = 10;

/// Detect all installed Proton versions (cached after first scan).
pub fn detect_proton_versions() -> Vec<ProtonVersion> {
    let mut cache = PROTON_CACHE.lock().unwrap();
    if let Some(cached) = cache.as_ref() {
        return cached.clone();
    }
    let versions = detect_proton_versions_uncached();
    *cache = Some(versions.clone());
    versions
}

/// Force a refresh of the Proton version cache.
pub fn invalidate_proton_cache() {
    if let Ok(mut cache) = PROTON_CACHE.lock() {
        *cache = None;
    }
}

/// Scan the filesystem for all installed Proton versions (uncached).
fn detect_proton_versions_uncached() -> Vec<ProtonVersion> {
    let mut versions = Vec::new();

    // Tier 1: Steam's steamapps/common
    for common_dir in find_steam_common_dirs() {
        scan_proton_dir(&common_dir, &mut versions);
    }

    // Tier 2: User compatibilitytools.d
    for compat_dir in find_compat_tools_dirs() {
        scan_proton_dir(&compat_dir, &mut versions);
    }

    // Deduplicate by path
    versions.sort_by(|a, b| a.path.cmp(&b.path));
    versions.dedup_by(|a, b| a.path == b.path);

    // Sort by version (newest first), then by variant preference
    versions.sort_by(|a, b| {
        b.major
            .cmp(&a.major)
            .then(b.minor.cmp(&a.minor))
            .then(variant_priority(&a.variant).cmp(&variant_priority(&b.variant)))
    });

    info!("Detected {} Proton versions", versions.len());
    for v in &versions {
        debug!("  {} ({}) at {}", v.name, v.variant, v.path.display());
    }

    versions
}

/// Get the recommended Proton version (newest, preferring GE > Official > others).
pub fn get_recommended_proton() -> Option<ProtonVersion> {
    let versions = detect_proton_versions();
    versions
        .into_iter()
        .find(|v| v.major >= RECOMMENDED_PROTON_MAJOR)
}

/// Check if a Proton version meets minimum requirements.
pub fn meets_minimum_version(version: &ProtonVersion) -> bool {
    version.major >= MIN_PROTON_MAJOR
}

/// Find a Proton wine binary for a specific bottle path.
///
/// Enhanced replacement for `find_proton_wine()` in launcher.rs.
/// Tries to find the best Proton for the given bottle, falling back to
/// the globally recommended version.
pub fn find_proton_for_bottle(bottle_path: &Path) -> Option<ProtonVersion> {
    let versions = detect_proton_versions();
    if versions.is_empty() {
        return None;
    }

    // Try to find a Proton co-located with this bottle's steamapps
    if let Some(steamapps) = find_steamapps_ancestor(bottle_path) {
        let common = steamapps.join("common");
        if let Some(local) = versions.iter().find(|v| v.path.starts_with(&common)) {
            return Some(local.clone());
        }
    }

    // Fall back to best available
    versions.into_iter().next()
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn variant_priority(v: &ProtonVariant) -> u32 {
    match v {
        ProtonVariant::GE => 0,       // Preferred for modding (more patches)
        ProtonVariant::Official => 1,
        ProtonVariant::CachyOS => 2,
        ProtonVariant::EM => 3,
        ProtonVariant::Custom(_) => 4,
    }
}

fn find_steam_common_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Some(home) = dirs::home_dir() {
        // Standard Steam install
        let standard = home.join(".local/share/Steam/steamapps/common");
        if standard.is_dir() {
            dirs.push(standard);
        }

        // Symlinked .steam path
        let steam_link = home.join(".steam/root/steamapps/common");
        if steam_link.is_dir() && !dirs.iter().any(|d| same_dir(d, &steam_link)) {
            dirs.push(steam_link);
        }

        // Flatpak Steam
        let flatpak =
            home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam/steamapps/common");
        if flatpak.is_dir() {
            dirs.push(flatpak);
        }

        // Snap Steam
        let snap = home.join("snap/steam/common/.local/share/Steam/steamapps/common");
        if snap.is_dir() {
            dirs.push(snap);
        }
    }

    // Also check Steam library folders (secondary drives)
    for libraryfolders_path in find_library_folders_paths() {
        if let Ok(content) = std::fs::read_to_string(&libraryfolders_path) {
            for line in content.lines() {
                let line = line.trim();
                if line.starts_with('"') && line.contains("\"path\"") {
                    // VDF format: "path"		"/mnt/games/SteamLibrary"
                    if let Some(path_str) = extract_vdf_value(line) {
                        let common = PathBuf::from(path_str).join("steamapps/common");
                        if common.is_dir() && !dirs.contains(&common) {
                            dirs.push(common);
                        }
                    }
                }
            }
        }
    }

    dirs
}

fn find_compat_tools_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Some(home) = dirs::home_dir() {
        // Standard location
        let standard = home.join(".steam/root/compatibilitytools.d");
        if standard.is_dir() {
            dirs.push(standard);
        }

        // Alternative location
        let alt = home.join(".local/share/Steam/compatibilitytools.d");
        if alt.is_dir() && !dirs.iter().any(|d| same_dir(d, &alt)) {
            dirs.push(alt);
        }

        // Flatpak Steam
        let flatpak = home
            .join(".var/app/com.valvesoftware.Steam/.local/share/Steam/compatibilitytools.d");
        if flatpak.is_dir() {
            dirs.push(flatpak);
        }
    }

    dirs
}

fn find_library_folders_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = dirs::home_dir() {
        let candidates = [
            home.join(".local/share/Steam/steamapps/libraryfolders.vdf"),
            home.join(".steam/root/steamapps/libraryfolders.vdf"),
            home.join(
                ".var/app/com.valvesoftware.Steam/.local/share/Steam/steamapps/libraryfolders.vdf",
            ),
        ];
        for c in &candidates {
            if c.exists() {
                paths.push(c.clone());
                break; // Usually only one is authoritative
            }
        }
    }
    paths
}

fn extract_vdf_value(line: &str) -> Option<&str> {
    // Parse VDF line: "key"		"value"
    let mut parts = line.splitn(2, "\"path\"");
    parts.next()?;
    let rest = parts.next()?.trim();
    if rest.starts_with('"') {
        // Skip opening quote
        let inner = &rest[1..];
        inner.find('"').map(|end| &inner[..end])
    } else {
        None
    }
}

fn scan_proton_dir(dir: &Path, versions: &mut Vec<ProtonVersion>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();

        // Check if this looks like a Proton installation
        if !looks_like_proton(&name) {
            continue;
        }

        // Find wine binary
        let wine_bin = find_wine_in_proton(&path);
        let wine_bin = match wine_bin {
            Some(w) => w,
            None => {
                debug!("Skipping {} — no wine binary found", name);
                continue;
            }
        };

        // Parse version from directory name
        let (major, minor, variant) = parse_proton_version(&name);

        let is_recommended = major >= RECOMMENDED_PROTON_MAJOR;

        versions.push(ProtonVersion {
            name: name.clone(),
            path,
            wine_bin,
            major,
            minor,
            variant,
            is_recommended,
        });
    }
}

fn looks_like_proton(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("proton") || lower.starts_with("ge-") || lower.starts_with("em-")
}

fn find_wine_in_proton(proton_dir: &Path) -> Option<PathBuf> {
    // Check both directory structures:
    // Newer Proton (9+): files/bin/wine
    // Older Proton: dist/bin/wine
    for sub in &[
        "files/bin/wine",
        "dist/bin/wine",
        "files/bin/wine64",
        "dist/bin/wine64",
    ] {
        let wine = proton_dir.join(sub);
        if wine.exists() {
            return Some(wine);
        }
    }
    None
}

/// Parse version numbers from a Proton directory name.
///
/// Handles:
/// - "Proton 10.0" -> (10, 0, Official)
/// - "Proton-10.0" -> (10, 0, Official)
/// - "Proton 9.0-4" -> (9, 0, Official)
/// - "GE-Proton10-27" -> (10, 27, GE)
/// - "GE-Proton9-22" -> (9, 22, GE)
/// - "CachyOS-Proton-10-27" -> (10, 27, CachyOS)
/// - "EM-10.0-33" -> (10, 0, EM)
/// - "Proton Experimental" -> (99, 0, Official) -- always newest
fn parse_proton_version(name: &str) -> (u32, u32, ProtonVariant) {
    // Handle "Proton Experimental" — treat as newest
    if name.to_lowercase().contains("experimental") {
        return (99, 0, ProtonVariant::Official);
    }

    // Determine variant from prefix
    let variant = if name.starts_with("GE-") || name.contains("-GE-") {
        ProtonVariant::GE
    } else if name.to_lowercase().starts_with("cachyos") {
        ProtonVariant::CachyOS
    } else if name.starts_with("EM-") {
        ProtonVariant::EM
    } else if name.to_lowercase().starts_with("proton") {
        ProtonVariant::Official
    } else {
        ProtonVariant::Custom(name.split('-').next().unwrap_or("unknown").to_string())
    };

    // Extract numbers from the name
    let numbers: Vec<u32> = name
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect();

    let major = numbers.first().copied().unwrap_or(0);
    let minor = numbers.get(1).copied().unwrap_or(0);

    (major, minor, variant)
}

fn find_steamapps_ancestor(path: &Path) -> Option<PathBuf> {
    let mut ancestor = path.parent();
    while let Some(dir) = ancestor {
        if dir
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase() == "steamapps")
            .unwrap_or(false)
        {
            return Some(dir.to_path_buf());
        }
        ancestor = dir.parent();
    }
    None
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_official_proton() {
        let (major, minor, variant) = parse_proton_version("Proton 10.0");
        assert_eq!(major, 10);
        assert_eq!(minor, 0);
        assert_eq!(variant, ProtonVariant::Official);
    }

    #[test]
    fn test_parse_ge_proton() {
        let (major, minor, variant) = parse_proton_version("GE-Proton10-27");
        assert_eq!(major, 10);
        assert_eq!(minor, 27);
        assert_eq!(variant, ProtonVariant::GE);
    }

    #[test]
    fn test_parse_ge_proton_9() {
        let (major, minor, variant) = parse_proton_version("GE-Proton9-22");
        assert_eq!(major, 9);
        assert_eq!(minor, 22);
        assert_eq!(variant, ProtonVariant::GE);
    }

    #[test]
    fn test_parse_cachyos_proton() {
        let (major, minor, variant) = parse_proton_version("CachyOS-Proton-10-27");
        assert_eq!(major, 10);
        assert_eq!(minor, 27);
        assert_eq!(variant, ProtonVariant::CachyOS);
    }

    #[test]
    fn test_parse_em_proton() {
        let (major, minor, variant) = parse_proton_version("EM-10.0-33");
        assert_eq!(major, 10);
        assert_eq!(minor, 0);
        assert_eq!(variant, ProtonVariant::EM);
    }

    #[test]
    fn test_parse_experimental() {
        let (major, _, variant) = parse_proton_version("Proton Experimental");
        assert_eq!(major, 99);
        assert_eq!(variant, ProtonVariant::Official);
    }

    #[test]
    fn test_parse_proton_with_dash() {
        let (major, minor, variant) = parse_proton_version("Proton-10.0");
        assert_eq!(major, 10);
        assert_eq!(minor, 0);
        assert_eq!(variant, ProtonVariant::Official);
    }

    #[test]
    fn test_parse_proton_9_0_4() {
        let (major, minor, variant) = parse_proton_version("Proton 9.0-4");
        assert_eq!(major, 9);
        assert_eq!(minor, 0);
        assert_eq!(variant, ProtonVariant::Official);
    }

    #[test]
    fn test_looks_like_proton() {
        assert!(looks_like_proton("Proton 10.0"));
        assert!(looks_like_proton("GE-Proton10-27"));
        assert!(looks_like_proton("EM-10.0-33"));
        assert!(!looks_like_proton("SteamLinuxRuntime"));
        assert!(!looks_like_proton("Skyrim Special Edition"));
    }

    #[test]
    fn test_meets_minimum() {
        let v = ProtonVersion {
            name: "Proton 10.0".into(),
            path: PathBuf::from("/tmp/proton"),
            wine_bin: PathBuf::from("/tmp/proton/files/bin/wine"),
            major: 10,
            minor: 0,
            variant: ProtonVariant::Official,
            is_recommended: true,
        };
        assert!(meets_minimum_version(&v));

        let old = ProtonVersion {
            major: 7,
            ..v.clone()
        };
        assert!(!meets_minimum_version(&old));
    }

    #[test]
    fn test_variant_display() {
        assert_eq!(format!("{}", ProtonVariant::Official), "Official");
        assert_eq!(format!("{}", ProtonVariant::GE), "GE");
        assert_eq!(format!("{}", ProtonVariant::CachyOS), "CachyOS");
        assert_eq!(format!("{}", ProtonVariant::EM), "EM");
        assert_eq!(
            format!("{}", ProtonVariant::Custom("TKG".to_string())),
            "TKG"
        );
    }

    #[test]
    fn test_variant_priority_ordering() {
        assert!(variant_priority(&ProtonVariant::GE) < variant_priority(&ProtonVariant::Official));
        assert!(
            variant_priority(&ProtonVariant::Official) < variant_priority(&ProtonVariant::CachyOS)
        );
        assert!(variant_priority(&ProtonVariant::CachyOS) < variant_priority(&ProtonVariant::EM));
        assert!(
            variant_priority(&ProtonVariant::EM)
                < variant_priority(&ProtonVariant::Custom("x".into()))
        );
    }

    #[test]
    fn test_extract_vdf_value() {
        assert_eq!(
            extract_vdf_value(r#""path"		"/mnt/games/SteamLibrary""#),
            Some("/mnt/games/SteamLibrary")
        );
        assert_eq!(extract_vdf_value(r#""1"		"something""#), None);
    }
}
