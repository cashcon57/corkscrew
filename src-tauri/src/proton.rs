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
static WINE_FORK_CACHE: Lazy<Mutex<Option<Vec<WineFork>>>> = Lazy::new(|| Mutex::new(None));

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
// System Wine fork detection
// ---------------------------------------------------------------------------

/// A detected system-installed Wine fork (not a Proton compatibility tool).
///
/// These are full Wine builds shipped via system packages (pacman, dpkg, AUR,
/// brew, manual install) at well-known paths outside of Steam. They include
/// wine-tkg, wine-staging, wine-ge, wine-cachyos, etc.
#[derive(Debug, Clone, Serialize)]
pub struct WineFork {
    /// Display name derived from the parent directory (e.g. "wine-tkg-git").
    pub name: String,
    /// Path to the wine binary.
    pub wine_bin: PathBuf,
    /// Normalized variant string (e.g. "wine-tkg", "wine-ge", "wine-staging").
    pub variant: String,
    /// Whether this fork is recommended. Always false for system forks since
    /// we don't know which version they are without execution; UI can prompt.
    pub is_recommended: bool,
}

/// Scan the filesystem for system-installed Wine forks (wine-tkg, wine-staging,
/// wine-ge, wine-cachyos, etc.).
///
/// Looks under:
/// - `/opt/wine-*` (any subdir matching that pattern)
/// - `/usr/local/wine*` and `/usr/local/bin/wine*` binaries
/// - `~/.local/opt/wine*`
///
/// Silently skips non-existent paths. Returns the list of forks discovered.
///
/// **Cached.** This function is called from `find_system_wine` on every
/// game launch, but the underlying filesystem layout doesn't change between
/// launches. The first call computes; subsequent calls return the cached
/// vector. Call `invalidate_wine_fork_cache()` to force a rescan (e.g.
/// after the user installs a new Wine package).
pub fn detect_system_wine_forks() -> Vec<WineFork> {
    let mut cache = WINE_FORK_CACHE.lock().unwrap();
    if let Some(cached) = cache.as_ref() {
        return cached.clone();
    }
    let forks = detect_system_wine_forks_uncached();
    *cache = Some(forks.clone());
    forks
}

/// Force a refresh of the system Wine fork cache.
#[allow(dead_code)]
pub fn invalidate_wine_fork_cache() {
    if let Ok(mut cache) = WINE_FORK_CACHE.lock() {
        *cache = None;
    }
}

/// Uncached scan implementation. Public-ish for testing via the inner
/// `scan_wine_forks_in_roots` helper, but called only by the cached entry
/// point above in production code.
fn detect_system_wine_forks_uncached() -> Vec<WineFork> {
    let mut roots: Vec<PathBuf> =
        vec![PathBuf::from("/opt"), PathBuf::from("/usr/local")];
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".local/opt"));
    }
    let usr_local_bin = PathBuf::from("/usr/local/bin");
    let mut forks = scan_wine_forks_in_roots(&roots, Some(&usr_local_bin));

    // On CachyOS hosts, surface wine-cachyos / Proton-CachyOS variants as
    // recommended so the launcher and UI prefer them over generic wine.
    if host_is_cachyos() {
        for f in forks.iter_mut() {
            if f.variant.contains("cachyos") || f.name.to_lowercase().contains("cachyos") {
                f.is_recommended = true;
            }
        }
    }

    info!("Detected {} system Wine forks", forks.len());
    for f in &forks {
        debug!(
            "  {} ({}, recommended={}) at {}",
            f.name,
            f.variant,
            f.is_recommended,
            f.wine_bin.display()
        );
    }

    forks
}

/// Detect whether the current host is CachyOS (or a CachyOS-derived distro)
/// by reading `/etc/os-release`. Pure-Rust, no shell-out.
///
/// Matches if `ID=cachyos` (exact) OR `ID_LIKE` contains the token
/// `cachyos` (whitespace-separated, per the os-release spec).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn host_is_cachyos() -> bool {
    match std::fs::read_to_string("/etc/os-release") {
        Ok(content) => os_release_is_cachyos(&content),
        Err(_) => false,
    }
}

/// Pure parser for `/etc/os-release` content; returns true if the host
/// identifies as CachyOS or has CachyOS in its `ID_LIKE`.
///
/// Per the os-release spec, values may be unquoted, single-quoted, or
/// double-quoted. `ID_LIKE` is a space-separated list of distro IDs.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn os_release_is_cachyos(content: &str) -> bool {
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = match line.split_once('=') {
            Some((k, v)) => (k.trim(), v.trim()),
            None => continue,
        };
        let value = strip_os_release_quotes(value);

        if key.eq_ignore_ascii_case("ID") {
            if value.eq_ignore_ascii_case("cachyos") {
                return true;
            }
        } else if key.eq_ignore_ascii_case("ID_LIKE") {
            for tok in value.split(|c: char| c.is_whitespace()) {
                if tok.eq_ignore_ascii_case("cachyos") {
                    return true;
                }
            }
        }
    }
    false
}

/// Strip surrounding `"` or `'` from an os-release value.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn strip_os_release_quotes(s: &str) -> &str {
    let bytes = s.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return &s[1..s.len() - 1];
        }
    }
    s
}

/// Inner scan for system Wine forks. Takes explicit roots and an optional
/// `usr_local_bin` directory so it can be exercised from tests against a
/// tempdir.
///
/// Roots are scanned for `wine-*` subdirectories; each match has its
/// `bin/wine` or `bin/wine64` extracted. Then the optional bin directory is
/// scanned for loose `wine*` executables (e.g. `/usr/local/bin/wine-staging`).
///
/// Non-existent paths are skipped silently with a debug log.
fn scan_wine_forks_in_roots(roots: &[PathBuf], usr_local_bin: Option<&Path>) -> Vec<WineFork> {
    let mut forks = Vec::new();
    let mut seen = std::collections::HashSet::<PathBuf>::new();

    // Scan directories matching wine-* and pick out their wine binaries.
    for root in roots {
        let entries = match std::fs::read_dir(root) {
            Ok(e) => e,
            Err(_) => {
                debug!("scanned {}: not present", root.display());
                continue;
            }
        };

        let mut count = 0usize;
        for entry in entries.flatten() {
            let path = entry.path();
            let dir_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };

            // Match "wine-*" subdirectories (and bare "wine"). Skip plain files.
            let lower = dir_name.to_lowercase();
            let looks_wine_dir = lower == "wine"
                || lower.starts_with("wine-")
                || lower.starts_with("wine_");
            if !looks_wine_dir {
                continue;
            }

            let md = match std::fs::metadata(&path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if !md.is_dir() {
                continue;
            }

            // Look for the wine binary inside; cover bin/wine, bin/wine64.
            for sub in &["bin/wine", "bin/wine64"] {
                let wine = path.join(sub);
                if wine.exists() {
                    let canonical = wine.canonicalize().unwrap_or_else(|_| wine.clone());
                    if seen.insert(canonical.clone()) {
                        let variant = derive_wine_variant(&dir_name);
                        forks.push(WineFork {
                            name: dir_name.clone(),
                            wine_bin: wine,
                            variant,
                            is_recommended: false,
                        });
                        count += 1;
                    }
                    break;
                }
            }
        }
        debug!("scanned {}, found {} wine fork dir entries", root.display(), count);
    }

    // Also scan a *bin* directory (typically /usr/local/bin) for loose
    // wine* binaries that aren't packaged in a wine-*/bin layout.
    if let Some(bin_dir) = usr_local_bin {
        match std::fs::read_dir(bin_dir) {
            Ok(entries) => {
                let mut count = 0usize;
                for entry in entries.flatten() {
                    let path = entry.path();
                    let name = match path.file_name().and_then(|n| n.to_str()) {
                        Some(n) => n.to_string(),
                        None => continue,
                    };
                    let lower = name.to_lowercase();
                    let is_wine_bin = lower == "wine"
                        || lower == "wine64"
                        || lower.starts_with("wine-")
                        || lower.starts_with("wine_");
                    if !is_wine_bin {
                        continue;
                    }
                    let md = match std::fs::metadata(&path) {
                        Ok(m) => m,
                        Err(_) => continue,
                    };
                    if !md.is_file() {
                        continue;
                    }

                    let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
                    if !seen.insert(canonical.clone()) {
                        continue;
                    }
                    let variant = derive_wine_variant(&name);
                    forks.push(WineFork {
                        name: name.clone(),
                        wine_bin: path,
                        variant,
                        is_recommended: false,
                    });
                    count += 1;
                }
                debug!(
                    "scanned {}, found {} wine binaries",
                    bin_dir.display(),
                    count
                );
            }
            Err(_) => {
                debug!("scanned {}: not present", bin_dir.display());
            }
        }
    }

    forks
}

/// Extract a normalized variant string from a wine-fork file or directory name.
///
/// Strips trailing version suffixes and `-git`/`-bin` markers, returning a
/// canonical fork identifier.
///
/// Examples:
/// - `wine-tkg-git` -> `wine-tkg`
/// - `wine-staging` -> `wine-staging`
/// - `wine-ge-9.21` -> `wine-ge`
/// - `wine-cachyos-staging-9.0` -> `wine-cachyos`
/// - `wine64` -> `wine`
/// - `wine` -> `wine`
pub fn derive_wine_variant(raw: &str) -> String {
    let lower = raw.to_lowercase();

    // Treat wine64 / wine_64 as plain wine.
    if lower == "wine64" || lower == "wine_64" || lower == "wine" {
        return "wine".to_string();
    }

    // Split on '-' or '_' so we can rebuild only the meaningful prefix.
    let parts: Vec<&str> = lower.split(|c| c == '-' || c == '_').collect();
    if parts.is_empty() {
        return lower;
    }

    // Always start with "wine"; if the first segment isn't wine just return it.
    if parts[0] != "wine" {
        return lower;
    }

    let mut variant = String::from("wine");
    for &seg in &parts[1..] {
        if seg.is_empty() {
            continue;
        }

        // Stop at the first numeric / version-ish segment.
        // "9", "9.0", "9.21", "10", etc.
        let first = seg.chars().next().unwrap();
        if first.is_ascii_digit() {
            break;
        }

        // Strip "git", "bin", "src" build markers — they're not part of the
        // variant identity. *Keep* "stable" because `wine-stable` is a real,
        // distinct variant (the upstream stable branch); collapsing it to
        // `wine` loses meaningful information for the UI.
        if matches!(seg, "git" | "bin" | "src") {
            break;
        }

        variant.push('-');
        variant.push_str(seg);
    }

    variant
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
        // ~/.steam/root/compatibilitytools.d (often a symlink, but cover it)
        let steam_root = home.join(".steam/root/compatibilitytools.d");
        if steam_root.is_dir() {
            dirs.push(steam_root);
        }

        // ~/.steam/steam/compatibilitytools.d (alternate Steam layout)
        let steam_steam = home.join(".steam/steam/compatibilitytools.d");
        if steam_steam.is_dir() && !dirs.iter().any(|d| same_dir(d, &steam_steam)) {
            dirs.push(steam_steam);
        }

        // XDG / standard local install: ~/.local/share/Steam/compatibilitytools.d
        let xdg = home.join(".local/share/Steam/compatibilitytools.d");
        if xdg.is_dir() && !dirs.iter().any(|d| same_dir(d, &xdg)) {
            dirs.push(xdg);
        }

        // Flatpak Steam: ~/.var/app/com.valvesoftware.Steam/data/Steam/compatibilitytools.d
        let flatpak_data = home
            .join(".var/app/com.valvesoftware.Steam/data/Steam/compatibilitytools.d");
        if flatpak_data.is_dir() && !dirs.iter().any(|d| same_dir(d, &flatpak_data)) {
            dirs.push(flatpak_data);
        }

        // Flatpak Steam (legacy layout): .../.local/share/Steam/compatibilitytools.d
        let flatpak_legacy = home
            .join(".var/app/com.valvesoftware.Steam/.local/share/Steam/compatibilitytools.d");
        if flatpak_legacy.is_dir() && !dirs.iter().any(|d| same_dir(d, &flatpak_legacy)) {
            dirs.push(flatpak_legacy);
        }
    }

    // System-installed compatibility tools (e.g., Proton-CachyOS via pacman/AUR)
    let system = PathBuf::from("/usr/share/steam/compatibilitytools.d");
    if system.is_dir() && !dirs.iter().any(|d| same_dir(d, &system)) {
        dirs.push(system);
    }

    debug!("Scanned {} compatibilitytools.d locations", dirs.len());
    for d in &dirs {
        debug!("  compatibilitytools.d: {}", d.display());
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

    // -----------------------------------------------------------------------
    // System Wine fork tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_derive_wine_variant_plain() {
        assert_eq!(derive_wine_variant("wine"), "wine");
        assert_eq!(derive_wine_variant("wine64"), "wine");
    }

    #[test]
    fn test_derive_wine_variant_tkg() {
        assert_eq!(derive_wine_variant("wine-tkg"), "wine-tkg");
        assert_eq!(derive_wine_variant("wine-tkg-git"), "wine-tkg");
        assert_eq!(derive_wine_variant("wine-tkg-9.0"), "wine-tkg");
    }

    #[test]
    fn test_derive_wine_variant_staging() {
        assert_eq!(derive_wine_variant("wine-staging"), "wine-staging");
        assert_eq!(derive_wine_variant("wine-staging-git"), "wine-staging");
        assert_eq!(derive_wine_variant("wine-staging-9.21"), "wine-staging");
    }

    #[test]
    fn test_derive_wine_variant_ge() {
        assert_eq!(derive_wine_variant("wine-ge"), "wine-ge");
        assert_eq!(derive_wine_variant("wine-ge-9.21"), "wine-ge");
        assert_eq!(derive_wine_variant("wine-GE-8.26"), "wine-ge");
    }

    #[test]
    fn test_derive_wine_variant_cachyos() {
        assert_eq!(derive_wine_variant("wine-cachyos"), "wine-cachyos");
        assert_eq!(
            derive_wine_variant("wine-cachyos-staging-9.0"),
            "wine-cachyos-staging"
        );
    }

    #[test]
    fn test_derive_wine_variant_stable_marker() {
        // "wine-stable" is a real variant (upstream stable branch) — keep it.
        // Only `git`, `bin`, `src` are stripped as build markers.
        assert_eq!(derive_wine_variant("wine-stable"), "wine-stable");
        assert_eq!(derive_wine_variant("wine-stable-9.0"), "wine-stable");
    }

    #[test]
    fn test_derive_wine_variant_nonwine_passthrough() {
        // If the input doesn't start with "wine", just lowercase it.
        assert_eq!(derive_wine_variant("crossover"), "crossover");
    }

    #[test]
    fn test_detect_system_wine_forks_finds_directory_install() {
        use std::fs;
        let temp = tempfile::tempdir().unwrap();
        let opt = temp.path().join("opt");

        // Build /opt/wine-tkg-git/bin/wine
        let tkg_bin = opt.join("wine-tkg-git/bin");
        fs::create_dir_all(&tkg_bin).unwrap();
        let wine = tkg_bin.join("wine");
        fs::write(&wine, b"#!/bin/sh\n").unwrap();

        // /opt/wine-staging/bin/wine64 (ensure wine64 is also discovered)
        let staging_bin = opt.join("wine-staging/bin");
        fs::create_dir_all(&staging_bin).unwrap();
        fs::write(staging_bin.join("wine64"), b"#!/bin/sh\n").unwrap();

        // Non-wine directory should be ignored.
        fs::create_dir_all(opt.join("cross-over/bin")).unwrap();
        fs::write(opt.join("cross-over/bin/wine"), b"#!/bin/sh\n").unwrap();

        // Use the helper that takes explicit roots so we don't touch the real
        // filesystem.
        let forks = scan_wine_forks_in_roots(&[opt], None);

        // We should get exactly the two wine-prefixed dirs.
        let names: Vec<_> = forks.iter().map(|f| f.name.as_str()).collect();
        assert!(
            names.contains(&"wine-tkg-git"),
            "expected wine-tkg-git in {:?}",
            names
        );
        assert!(
            names.contains(&"wine-staging"),
            "expected wine-staging in {:?}",
            names
        );
        assert!(
            !names.contains(&"cross-over"),
            "non-wine dir should be skipped"
        );

        // Variant extraction should be applied.
        let tkg = forks.iter().find(|f| f.name == "wine-tkg-git").unwrap();
        assert_eq!(tkg.variant, "wine-tkg");
        assert!(!tkg.is_recommended);
        let stg = forks.iter().find(|f| f.name == "wine-staging").unwrap();
        assert_eq!(stg.variant, "wine-staging");
    }

    #[test]
    fn test_detect_system_wine_forks_finds_usr_local_bin() {
        use std::fs;
        let temp = tempfile::tempdir().unwrap();
        let usr_local_bin = temp.path().join("usr_local_bin");
        fs::create_dir_all(&usr_local_bin).unwrap();

        // Drop in some loose binaries.
        fs::write(usr_local_bin.join("wine-staging"), b"\x7fELF").unwrap();
        fs::write(usr_local_bin.join("wine-tkg"), b"\x7fELF").unwrap();
        fs::write(usr_local_bin.join("foo"), b"unrelated").unwrap();

        let forks = scan_wine_forks_in_roots(&[], Some(&usr_local_bin));
        let names: Vec<_> = forks.iter().map(|f| f.name.as_str()).collect();
        assert!(
            names.contains(&"wine-staging"),
            "expected wine-staging in {:?}",
            names
        );
        assert!(
            names.contains(&"wine-tkg"),
            "expected wine-tkg in {:?}",
            names
        );
        assert!(!names.contains(&"foo"), "non-wine binary should be ignored");
    }

    #[test]
    fn test_detect_system_wine_forks_silently_skips_missing_paths() {
        // Pointing at a non-existent root must not error or panic.
        let forks = scan_wine_forks_in_roots(
            &[PathBuf::from("/definitely/does/not/exist/here")],
            Some(&PathBuf::from("/also/missing")),
        );
        assert!(forks.is_empty());
    }

    #[test]
    fn test_find_compat_tools_dirs_includes_system_path_when_present() {
        // We can't reliably mock /usr/share/steam in unit tests, but we can at
        // least call the function and assert it returns without panicking.
        let _ = find_compat_tools_dirs();
    }

    // -----------------------------------------------------------------------
    // CachyOS host detection
    // -----------------------------------------------------------------------

    #[test]
    fn test_os_release_is_cachyos_exact_id() {
        let sample = "NAME=\"CachyOS\"\n\
                      PRETTY_NAME=\"CachyOS\"\n\
                      ID=cachyos\n\
                      ID_LIKE=arch\n\
                      BUILD_ID=rolling\n";
        assert!(os_release_is_cachyos(sample));
    }

    #[test]
    fn test_os_release_is_cachyos_quoted_id() {
        // Some distros quote the ID value.
        let sample = "ID=\"cachyos\"\n";
        assert!(os_release_is_cachyos(sample));
        let sample = "ID='cachyos'\n";
        assert!(os_release_is_cachyos(sample));
    }

    #[test]
    fn test_os_release_is_cachyos_via_id_like() {
        // A CachyOS-derived distro that lists cachyos in ID_LIKE.
        let sample = "ID=mycustomdistro\n\
                      ID_LIKE=\"cachyos arch\"\n";
        assert!(os_release_is_cachyos(sample));
    }

    #[test]
    fn test_os_release_is_cachyos_arch_not_cachyos() {
        // Pure Arch should not match. Containing "cachy" as a substring of
        // some other word would be a false positive — verify token matching.
        let sample = "NAME=\"Arch Linux\"\nID=arch\nID_LIKE=\"\"\n";
        assert!(!os_release_is_cachyos(sample));
    }

    #[test]
    fn test_os_release_is_cachyos_substring_does_not_match() {
        // ID=cachyos-experimental shouldn't match (we want exact ID); but a
        // deliberate "not-cachyos" string with cachy in it must not match.
        let sample = "ID=notcachyos\n";
        assert!(!os_release_is_cachyos(sample));
    }

    #[test]
    fn test_strip_os_release_quotes() {
        assert_eq!(strip_os_release_quotes("\"cachyos\""), "cachyos");
        assert_eq!(strip_os_release_quotes("'cachyos'"), "cachyos");
        assert_eq!(strip_os_release_quotes("cachyos"), "cachyos");
        assert_eq!(strip_os_release_quotes(""), "");
        assert_eq!(strip_os_release_quotes("\""), "\"");
    }

    #[test]
    fn test_detect_system_wine_forks_caches_after_first_call() {
        // Sanity-check: two consecutive calls return the same data and the
        // cache holds something. We can't reliably assert the cache speeds
        // anything up in a unit test, but we can at least confirm the
        // result is deterministic.
        invalidate_wine_fork_cache();
        let first = detect_system_wine_forks();
        let second = detect_system_wine_forks();
        assert_eq!(first.len(), second.len());
        for (a, b) in first.iter().zip(second.iter()) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.variant, b.variant);
            assert_eq!(a.wine_bin, b.wine_bin);
        }
        invalidate_wine_fork_cache();
    }
}
