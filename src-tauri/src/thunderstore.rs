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
//! for 24h. Cache files live under the platform's data-local dir (same
//! convention as verified_lists).

use std::collections::HashMap;
use std::path::PathBuf;
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

fn ensure_mem() {
    let read_hit = CACHE.read().ok().and_then(|g| g.as_ref().map(|_| ())).is_some();
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
        let tmp = path.with_extension("json.tmp");
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
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, &bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
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

/// Resolve the full dependency closure for a version within a community.
/// Dependencies are `"author-name-version"`; we match by `"author-name"` prefix
/// and prefer the explicitly requested version if present, otherwise the
/// latest version of that package.
pub fn resolve_dependencies<'a>(
    packages: &'a [Package],
    root: &PackageVersion,
) -> Vec<&'a PackageVersion> {
    let mut resolved: Vec<&PackageVersion> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut queue: Vec<&PackageVersion> = vec![root];

    while let Some(v) = queue.pop() {
        for dep in &v.dependencies {
            // Dep format: "author-name-1.2.3". Split off the version suffix.
            let (pkg_id, version) = match dep.rsplit_once('-') {
                Some((head, ver)) => (head.to_string(), ver.to_string()),
                None => continue,
            };
            if !seen.insert(pkg_id.clone()) {
                continue;
            }
            let Some(pkg) = packages.iter().find(|p| p.full_name == pkg_id) else {
                continue;
            };
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
}
