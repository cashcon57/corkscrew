//! Thunderstore API client.
//!
//! Thunderstore is the canonical mod catalog for most Unity + BepInEx games
//! (Risk of Rain 2, Lethal Company, Content Warning, Silksong, etc.). Each
//! supported game is a "community"; each community has its own package catalog.
//!
//! This module is the read-only catalog half — discover communities, list
//! packages, resolve versions + dependencies, surface download URLs. The
//! install pipeline is wired in a separate pass (Discover → Thunderstore UI
//! + deploy integration are session 2 work).
//!
//! Caching: per-community package listings are cached on disk for 1h and
//! in-memory for the process lifetime. The full community list is cached
//! for 24h. Downloaded package zips are cached indefinitely (named by
//! `full_name`, content-addressable by version). All cache files live under
//! the platform's data-local dir.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const API_BASE: &str = "https://thunderstore.io";
const USER_AGENT: &str = "Corkscrew";

const COMMUNITIES_CACHE_TTL: u64 = 24 * 60 * 60;
const PACKAGES_CACHE_TTL: u64 = 60 * 60;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// A Thunderstore community = one game's mod scene. Identifier is the URL
/// slug (e.g. `"hollow-knight-silksong"`, `"lethal-company"`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Community {
    pub identifier: String,
    pub name: String,
    #[serde(default)]
    pub discord_url: Option<String>,
    #[serde(default)]
    pub wiki_url: Option<String>,
    #[serde(default)]
    pub require_package_listing_approval: bool,
}

/// A package in a community. Has one or more versions.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Package {
    pub name: String,
    /// `"author-name"` — used as dep key prefix.
    pub full_name: String,
    pub owner: String,
    pub package_url: String,
    pub date_created: String,
    pub date_updated: String,
    pub rating_score: i64,
    #[serde(default)]
    pub is_pinned: bool,
    #[serde(default)]
    pub is_deprecated: bool,
    #[serde(default)]
    pub has_nsfw_content: bool,
    #[serde(default)]
    pub categories: Vec<String>,
    pub versions: Vec<PackageVersion>,
}

/// A specific version of a package. This is what you download + install.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackageVersion {
    pub name: String,
    /// `"author-name-version"` — canonical identifier, used in dep chains.
    pub full_name: String,
    pub description: String,
    #[serde(default)]
    pub icon: Option<String>,
    pub version_number: String,
    /// List of `"author-name-version"` dep identifiers.
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub download_url: String,
    pub downloads: i64,
    pub date_created: String,
    #[serde(default)]
    pub website_url: Option<String>,
    #[serde(default)]
    pub is_active: bool,
    pub file_size: u64,
}

// ---------------------------------------------------------------------------
// Cache state
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct CommunitiesCache {
    fetched_at: u64,
    communities: Vec<Community>,
}

#[derive(Serialize, Deserialize)]
struct PackagesCache {
    fetched_at: u64,
    community: String,
    packages: Vec<Package>,
}

struct MemCache {
    communities: Option<(u64, Vec<Community>)>,
    packages: HashMap<String, (u64, Vec<Package>)>,
}

static CACHE: RwLock<Option<MemCache>> = RwLock::new(None);

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn cache_dir() -> Option<PathBuf> {
    let base = dirs::data_local_dir()?;
    let dir = base.join("corkscrew").join("thunderstore");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir)
}

fn packages_cache_dir(community: &str) -> Option<PathBuf> {
    let dir = cache_dir()?
        .join("packages")
        .join(sanitize_segment(community));
    let _ = std::fs::create_dir_all(&dir);
    Some(dir)
}

/// Sanitize a path segment: strip traversal attempts, keep alphanumerics,
/// dashes, underscores, dots. Rejects `.`/`..`/empty results (which the OS
/// would treat as path components, not filenames) by falling back to a
/// content-derived hash.
fn sanitize_segment(s: &str) -> String {
    let mapped: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if mapped.is_empty() || mapped == "." || mapped == ".." {
        return hashed_fallback(s);
    }
    mapped
}

/// 16-hex-char SHA256 prefix of the input, used as a safe filename when the
/// sanitized form would collide with a path component (`.`, `..`, empty).
fn hashed_fallback(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(16 + 4);
    hex.push_str("pkg_");
    for b in digest.iter().take(8) {
        use std::fmt::Write;
        let _ = write!(hex, "{:02x}", b);
    }
    hex
}

fn ensure_mem() {
    let read_hit = CACHE
        .read()
        .ok()
        .and_then(|g| g.as_ref().map(|_| ()))
        .is_some();
    if !read_hit {
        if let Ok(mut w) = CACHE.write() {
            if w.is_none() {
                *w = Some(MemCache {
                    communities: None,
                    packages: HashMap::new(),
                });
            }
        }
    }
}

fn read_disk_communities() -> Option<CommunitiesCache> {
    let path = cache_dir()?.join("communities.json");
    let bytes = std::fs::read(&path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_disk_communities(c: &CommunitiesCache) {
    let Some(path) = cache_dir().map(|d| d.join("communities.json")) else {
        return;
    };
    if let Ok(bytes) = serde_json::to_vec_pretty(c) {
        // Unique tmp suffix prevents concurrent refreshes from clobbering
        // each other's `*.tmp` file before rename.
        let tmp = path.with_extension(format!("json.tmp.{}", unique_tmp_suffix()));
        if std::fs::write(&tmp, &bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

fn read_disk_packages(community: &str) -> Option<PackagesCache> {
    let path = cache_dir()?.join(format!("packages-{}.json", community));
    let bytes = std::fs::read(&path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_disk_packages(p: &PackagesCache) {
    let Some(path) = cache_dir().map(|d| d.join(format!("packages-{}.json", p.community))) else {
        return;
    };
    if let Ok(bytes) = serde_json::to_vec(p) {
        // Unique tmp suffix prevents concurrent refreshes (two
        // `list_packages(community)` calls racing) from clobbering each
        // other's `*.tmp` file before rename.
        let tmp = path.with_extension(format!("json.tmp.{}", unique_tmp_suffix()));
        if std::fs::write(&tmp, &bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

/// Process-unique tmp suffix: PID + nanos-since-epoch. Used to keep
/// concurrent atomic-write callers from overwriting each other's tmp file
/// before rename.
fn unique_tmp_suffix() -> String {
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{}", pid, nanos)
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub async fn list_communities() -> Result<Vec<Community>, String> {
    ensure_mem();
    if let Ok(g) = CACHE.read() {
        if let Some(cache) = g.as_ref() {
            if let Some((t, v)) = &cache.communities {
                if unix_now().saturating_sub(*t) < COMMUNITIES_CACHE_TTL {
                    return Ok(v.clone());
                }
            }
        }
    }
    if let Some(c) = read_disk_communities() {
        if unix_now().saturating_sub(c.fetched_at) < COMMUNITIES_CACHE_TTL {
            if let Ok(mut w) = CACHE.write() {
                if let Some(m) = w.as_mut() {
                    m.communities = Some((c.fetched_at, c.communities.clone()));
                }
            }
            return Ok(c.communities);
        }
    }

    let client = http_client()?;
    let mut url = format!("{}/api/experimental/community/", API_BASE);
    let mut all: Vec<Community> = Vec::new();
    // Paginated: {"next": url, "results": [...]}
    loop {
        #[derive(Deserialize)]
        struct Page {
            next: Option<String>,
            results: Vec<Community>,
        }
        let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {} fetching communities", resp.status()));
        }
        let page: Page = resp.json().await.map_err(|e| e.to_string())?;
        all.extend(page.results);
        match page.next {
            Some(n) if !n.is_empty() && n != url => url = n,
            _ => break,
        }
    }

    let fetched_at = unix_now();
    let cache = CommunitiesCache {
        fetched_at,
        communities: all.clone(),
    };
    write_disk_communities(&cache);
    if let Ok(mut w) = CACHE.write() {
        if let Some(m) = w.as_mut() {
            m.communities = Some((fetched_at, all.clone()));
        }
    }
    Ok(all)
}

pub async fn list_packages(community: &str) -> Result<Vec<Package>, String> {
    ensure_mem();
    if let Ok(g) = CACHE.read() {
        if let Some(cache) = g.as_ref() {
            if let Some((t, v)) = cache.packages.get(community) {
                if unix_now().saturating_sub(*t) < PACKAGES_CACHE_TTL {
                    return Ok(v.clone());
                }
            }
        }
    }
    if let Some(c) = read_disk_packages(community) {
        if unix_now().saturating_sub(c.fetched_at) < PACKAGES_CACHE_TTL {
            if let Ok(mut w) = CACHE.write() {
                if let Some(m) = w.as_mut() {
                    m.packages
                        .insert(community.to_string(), (c.fetched_at, c.packages.clone()));
                }
            }
            return Ok(c.packages);
        }
    }

    let client = http_client()?;
    let url = format!("{}/c/{}/api/v1/package/", API_BASE, community);
    let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!(
            "HTTP {} fetching packages for {}",
            resp.status(),
            community
        ));
    }
    let packages: Vec<Package> = resp.json().await.map_err(|e| e.to_string())?;

    let fetched_at = unix_now();
    write_disk_packages(&PackagesCache {
        fetched_at,
        community: community.to_string(),
        packages: packages.clone(),
    });
    if let Ok(mut w) = CACHE.write() {
        if let Some(m) = w.as_mut() {
            m.packages
                .insert(community.to_string(), (fetched_at, packages.clone()));
        }
    }
    Ok(packages)
}

/// Find a package by `author-name` within a community.
pub fn find_package<'a>(packages: &'a [Package], author_name: &str) -> Option<&'a Package> {
    packages.iter().find(|p| p.full_name == author_name)
}

/// Find a specific `author-name-version` across a community's package list.
///
/// Dep strings are `"author-Name-VERSION"` where VERSION may itself contain
/// dashes (SemVer pre-release tags like `1.0.0-rc.1`). Naive `rsplit_once('-')`
/// breaks those. Instead we walk every `package.full_name` (the `author-name`
/// half) and try to match it as a prefix of `full_name`, then check whether
/// the suffix-after-`-` corresponds to a known `version_number` on that
/// package. This handles pre-release versions cleanly without parsing.
pub fn find_version<'a>(packages: &'a [Package], full_name: &str) -> Option<&'a PackageVersion> {
    let (pkg, version) = match_pkg_and_version(packages, full_name)?;
    pkg.versions
        .iter()
        .find(|v| v.version_number == version && v.full_name == full_name)
}

/// Match a `"author-name-version"` string against the package list, returning
/// the matching package and the parsed version string. Used by both
/// `find_version` and the dep walker. Tries each `package.full_name` as a
/// prefix and validates the trailing suffix against that package's known
/// `version_number`s.
fn match_pkg_and_version<'a>(
    packages: &'a [Package],
    full_name: &str,
) -> Option<(&'a Package, String)> {
    for pkg in packages {
        let prefix = &pkg.full_name;
        // Need at least one extra `-` and a version after it.
        if full_name.len() <= prefix.len() + 1 {
            continue;
        }
        if !full_name.starts_with(prefix.as_str()) {
            continue;
        }
        let after = &full_name[prefix.len()..];
        let Some(version) = after.strip_prefix('-') else {
            continue;
        };
        if pkg.versions.iter().any(|v| v.version_number == version) {
            return Some((pkg, version.to_string()));
        }
    }
    None
}

/// Resolve the full dependency closure for a version within a community.
/// Dependencies are `"author-name-version"`; we match by checking every
/// `package.full_name` as a prefix and validating the trailing version against
/// that package's known versions. This handles pre-release tags like
/// `1.0.0-rc.1` that contain extra dashes.
pub fn resolve_dependencies<'a>(
    packages: &'a [Package],
    root: &PackageVersion,
) -> Vec<&'a PackageVersion> {
    let mut resolved: Vec<&PackageVersion> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut queue: Vec<&PackageVersion> = vec![root];

    while let Some(v) = queue.pop() {
        for dep in &v.dependencies {
            let Some((pkg, version)) = match_pkg_and_version(packages, dep) else {
                continue;
            };
            if !seen.insert(pkg.full_name.clone()) {
                continue;
            }
            let picked = pkg
                .versions
                .iter()
                .find(|pv| pv.version_number == version)
                .or_else(|| pkg.versions.first());
            if let Some(pv) = picked {
                resolved.push(pv);
                queue.push(pv);
            }
        }
    }

    resolved
}

// ---------------------------------------------------------------------------
// Package download + install
// ---------------------------------------------------------------------------

/// Report from installing one Thunderstore package version.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstallReport {
    pub full_name: String,
    pub mod_subdir: String,
    pub installed_files: Vec<String>,
}

/// Download a package version's zip and cache it on disk. Returns the local
/// path to the zip. Repeat calls for the same version are no-ops.
pub async fn download_version(
    community: &str,
    version: &PackageVersion,
) -> Result<PathBuf, String> {
    let dir = packages_cache_dir(community).ok_or("cache dir unavailable")?;
    let filename = format!("{}.zip", sanitize_segment(&version.full_name));
    let path = dir.join(&filename);
    if path.exists() {
        if let Ok(meta) = std::fs::metadata(&path) {
            // File_size on Thunderstore can be 0 for pre-v1 packages; treat
            // any non-empty cache file as a hit.
            if meta.len() > 0 {
                return Ok(path);
            }
        }
    }

    let client = http_client()?;
    let resp = client
        .get(&version.download_url)
        .send()
        .await
        .map_err(|e| format!("download: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "HTTP {} downloading {}",
            resp.status(),
            version.full_name
        ));
    }
    let bytes = resp.bytes().await.map_err(|e| format!("read body: {e}"))?;

    let tmp = path.with_extension("zip.tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Extract + deploy a downloaded package zip into `<data_dir>/<full_name>/`.
/// Each Thunderstore mod gets its own subdirectory under `data_dir` (which
/// for BepInEx games is `BepInEx/plugins`). Returns the `InstallReport`.
pub fn install_version(
    zip_path: &Path,
    data_dir: &Path,
    full_name: &str,
) -> Result<InstallReport, String> {
    let sub = sanitize_segment(full_name);
    if sub.is_empty() {
        return Err("empty full_name".into());
    }
    let mod_dir = data_dir.join(&sub);

    // `crate::installer::install_mod` extracts + copies into the given
    // data_dir, flattening the archive root. We wrap it with the per-mod
    // subdir so each Thunderstore package stays self-contained.
    let installed_files = crate::installer::install_mod(
        zip_path, &mod_dir, full_name,
        "",   // version string not needed — embedded in full_name
        None, // no Nexus ID
    )
    .map_err(|e| format!("install_mod: {e}"))?;

    Ok(InstallReport {
        full_name: full_name.to_string(),
        mod_subdir: sub,
        installed_files,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_version(full_name: &str, deps: &[&str]) -> PackageVersion {
        PackageVersion {
            name: full_name.to_string(),
            full_name: full_name.to_string(),
            description: "".into(),
            icon: None,
            version_number: full_name
                .rsplit_once('-')
                .map(|(_, v)| v.to_string())
                .unwrap_or_default(),
            dependencies: deps.iter().map(|s| s.to_string()).collect(),
            download_url: "".into(),
            downloads: 0,
            date_created: "".into(),
            website_url: None,
            is_active: true,
            file_size: 0,
        }
    }

    fn fake_package(author_name: &str, version: &str, deps: &[&str]) -> Package {
        Package {
            name: "".into(),
            full_name: author_name.to_string(),
            owner: "".into(),
            package_url: "".into(),
            date_created: "".into(),
            date_updated: "".into(),
            rating_score: 0,
            is_pinned: false,
            is_deprecated: false,
            has_nsfw_content: false,
            categories: vec![],
            versions: vec![fake_version(&format!("{author_name}-{version}"), deps)],
        }
    }

    #[test]
    fn find_package_matches_by_full_name() {
        let pkgs = vec![
            fake_package("BepInEx-BepInExPack", "5.4.21", &[]),
            fake_package("author-Mod", "1.0.0", &[]),
        ];
        assert!(find_package(&pkgs, "BepInEx-BepInExPack").is_some());
        assert!(find_package(&pkgs, "other-Thing").is_none());
    }

    #[test]
    fn resolve_dependencies_pulls_transitive() {
        let pkgs = vec![
            fake_package("BepInEx-BepInExPack", "5.4.21", &[]),
            fake_package("author-ModA", "1.0.0", &["BepInEx-BepInExPack-5.4.21"]),
            fake_package(
                "author-ModB",
                "2.0.0",
                &["author-ModA-1.0.0", "BepInEx-BepInExPack-5.4.21"],
            ),
        ];
        let root = &pkgs[2].versions[0];
        let closure = resolve_dependencies(&pkgs, root);
        let names: Vec<&str> = closure.iter().map(|v| v.full_name.as_str()).collect();
        assert!(names.contains(&"author-ModA-1.0.0"));
        assert!(names.contains(&"BepInEx-BepInExPack-5.4.21"));
        // Each dep resolved once even though BepInEx is referenced twice.
        assert_eq!(
            names
                .iter()
                .filter(|n| n.starts_with("BepInEx-BepInExPack"))
                .count(),
            1
        );
    }

    #[test]
    fn resolve_dependencies_missing_dep_skipped() {
        let pkgs = vec![fake_package(
            "author-ModA",
            "1.0.0",
            &["nonexistent-Package-9.9.9"],
        )];
        let root = &pkgs[0].versions[0];
        let closure = resolve_dependencies(&pkgs, root);
        assert!(closure.is_empty());
    }

    #[test]
    fn find_version_walks_package_list() {
        let pkgs = vec![
            fake_package("author-ModA", "1.0.0", &[]),
            fake_package("author-ModA", "2.0.0", &[]),
        ];
        // Our fake_package only stores one version. Patch the first to hold both.
        let mut pkgs = pkgs;
        pkgs[0]
            .versions
            .push(fake_version("author-ModA-2.0.0", &[]));

        let v = find_version(&pkgs, "author-ModA-2.0.0");
        assert!(v.is_some());
        assert_eq!(v.unwrap().version_number, "2.0.0");

        let missing = find_version(&pkgs, "author-ModA-9.9.9");
        assert!(missing.is_none());

        let malformed = find_version(&pkgs, "no-dashes");
        assert!(malformed.is_none());
    }

    #[test]
    fn find_version_handles_semver_prerelease() {
        // Pre-release like `1.0.0-rc.1` contains an extra `-`. The naive
        // `rsplit_once('-')` would split on `rc.1`'s leading dash and break.
        let mut pkg = Package {
            name: "".into(),
            full_name: "author-Mod".into(),
            owner: "".into(),
            package_url: "".into(),
            date_created: "".into(),
            date_updated: "".into(),
            rating_score: 0,
            is_pinned: false,
            is_deprecated: false,
            has_nsfw_content: false,
            categories: vec![],
            versions: vec![],
        };
        let mut v = fake_version("author-Mod-1.0.0-rc.1", &[]);
        v.version_number = "1.0.0-rc.1".to_string();
        pkg.versions.push(v);
        let mut v2 = fake_version("author-Mod-1.0.0", &[]);
        v2.version_number = "1.0.0".to_string();
        pkg.versions.push(v2);
        let pkgs = vec![pkg];

        let pre = find_version(&pkgs, "author-Mod-1.0.0-rc.1");
        assert!(pre.is_some(), "pre-release version should resolve");
        assert_eq!(pre.unwrap().version_number, "1.0.0-rc.1");

        let stable = find_version(&pkgs, "author-Mod-1.0.0");
        assert!(stable.is_some());
        assert_eq!(stable.unwrap().version_number, "1.0.0");
    }

    #[test]
    fn resolve_dependencies_handles_semver_prerelease() {
        // Build a pkg whose dep string is `BepInEx-BepInExPack-5.4.21-pre.7`.
        let mut bepinex = Package {
            name: "".into(),
            full_name: "BepInEx-BepInExPack".into(),
            owner: "".into(),
            package_url: "".into(),
            date_created: "".into(),
            date_updated: "".into(),
            rating_score: 0,
            is_pinned: false,
            is_deprecated: false,
            has_nsfw_content: false,
            categories: vec![],
            versions: vec![],
        };
        let mut v = fake_version("BepInEx-BepInExPack-5.4.21-pre.7", &[]);
        v.version_number = "5.4.21-pre.7".to_string();
        bepinex.versions.push(v);

        let consumer = fake_package(
            "author-ModA",
            "1.0.0",
            &["BepInEx-BepInExPack-5.4.21-pre.7"],
        );

        let pkgs = vec![bepinex, consumer];
        let root = &pkgs[1].versions[0];
        let closure = resolve_dependencies(&pkgs, root);
        let names: Vec<&str> = closure.iter().map(|v| v.full_name.as_str()).collect();
        assert!(names.contains(&"BepInEx-BepInExPack-5.4.21-pre.7"));
    }

    #[test]
    fn sanitize_segment_strips_traversal() {
        assert_eq!(sanitize_segment("author-Mod-1.0.0"), "author-Mod-1.0.0");
        assert_eq!(sanitize_segment("../etc/passwd"), ".._etc_passwd");
        assert_eq!(sanitize_segment("foo/bar\\baz"), "foo_bar_baz");
        assert_eq!(sanitize_segment("null\0byte"), "null_byte");
    }

    #[test]
    fn sanitize_segment_rejects_dot_dotdot_empty() {
        // Empty input → hashed fallback, not ""
        let empty = sanitize_segment("");
        assert!(empty.starts_with("pkg_"));
        assert_eq!(empty.len(), 4 + 16);

        // Bare "." would become "." after mapping (dots are preserved) — must
        // be rejected because the OS would treat it as the current directory.
        let dot = sanitize_segment(".");
        assert!(dot.starts_with("pkg_"));
        assert_ne!(dot, ".");

        // ".." likewise must not survive — path traversal.
        let dd = sanitize_segment("..");
        assert!(dd.starts_with("pkg_"));
        assert_ne!(dd, "..");

        // All-slashes input maps to underscores → produces a non-empty,
        // non-traversal segment, no fallback needed.
        assert_eq!(sanitize_segment("/"), "_");
    }
}
