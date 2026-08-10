//! Verified-lists registry: Wine/CrossOver compatibility data for
//! NexusMods collections and Wabbajack modlists.
//!
//! Sources, in order of precedence:
//! 1. Remote manifest fetched from GitHub (24h TTL disk cache)
//! 2. Disk cache from a previous fetch (if remote fails)
//! 3. Bundled manifest (`../verified_lists.json`, embedded at build time)
//!
//! Keys:
//! - Collections are keyed by `game_domain` (e.g. `skyrimspecialedition`) → `slug`.
//! - Wabbajack modlists are keyed by exact `modlist_name` (Wabbajack's own name).
//!
//! Statuses:
//! - `Verified`   — installs and plays cleanly under Wine/CrossOver/Proton.
//! - `Partial`    — installs but has known issues (see `notes`).
//! - `Broken`     — confirmed broken under Wine.
//! - `Untested`   — no entry (default for anything not in the manifest).
//!
//! Disk-cache write strategy:
//! Writes go to a tmp file with a `json.tmp.<pid>.<unix_secs>` suffix and are
//! atomically renamed to the final cache filename. The PID + timestamp suffix
//! ensures concurrent `refresh_from_remote()` calls (e.g. background refresh
//! racing a manual one) write to *distinct* tmp files, so neither writer can
//! truncate the other mid-write. The final rename is still atomic per writer;
//! whichever rename lands last wins, and both produce well-formed manifests.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const BUNDLED_MANIFEST: &str = include_str!("../verified_lists.json");
const REMOTE_URL: &str =
    "https://raw.githubusercontent.com/cashcon57/Corkscrew/main/src-tauri/verified_lists.json";
const CACHE_FILENAME: &str = "verified_lists_cache.json";
const CACHE_TTL_SECS: u64 = 24 * 60 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifiedStatus {
    Verified,
    Partial,
    Broken,
    Untested,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerifiedEntry {
    pub status: VerifiedStatus,
    #[serde(default)]
    pub version_tested: Option<String>,
    #[serde(default)]
    pub last_verified: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub reporter: Option<String>,
}

impl VerifiedEntry {
    pub fn untested() -> Self {
        Self {
            status: VerifiedStatus::Untested,
            version_tested: None,
            last_verified: None,
            notes: None,
            reporter: None,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub generated_at: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub collections: HashMap<String, HashMap<String, VerifiedEntry>>,
    #[serde(default)]
    pub wabbajack: HashMap<String, VerifiedEntry>,
}

impl Manifest {
    fn from_bundled() -> Self {
        serde_json::from_str(BUNDLED_MANIFEST).unwrap_or_default()
    }
}

struct CacheState {
    manifest: Manifest,
    fetched_at: Option<u64>,
}

static CACHE: RwLock<Option<CacheState>> = RwLock::new(None);

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn cache_path() -> Option<PathBuf> {
    let base = dirs::data_local_dir()?;
    let dir = base.join("corkscrew");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join(CACHE_FILENAME))
}

#[derive(Serialize, Deserialize)]
struct DiskCache {
    fetched_at: u64,
    manifest: Manifest,
}

fn load_disk_cache() -> Option<(Manifest, u64)> {
    let path = cache_path()?;
    let bytes = std::fs::read(&path).ok()?;
    let decoded: DiskCache = serde_json::from_slice(&bytes).ok()?;
    Some((decoded.manifest, decoded.fetched_at))
}

fn write_disk_cache(manifest: &Manifest, fetched_at: u64) {
    let Some(path) = cache_path() else {
        return;
    };
    let payload = DiskCache {
        fetched_at,
        manifest: manifest.clone(),
    };
    if let Ok(bytes) = serde_json::to_vec_pretty(&payload) {
        // Suffix tmp filename with PID + unix-seconds so two concurrent
        // refreshes write to disjoint tmp paths and cannot truncate each other.
        // The rename-to-final is atomic per writer; last-rename-wins is fine
        // because both inputs are well-formed manifests.
        let tmp = path.with_extension(format!(
            "json.tmp.{}.{}",
            std::process::id(),
            unix_now()
        ));
        if std::fs::write(&tmp, &bytes).is_ok() {
            if std::fs::rename(&tmp, &path).is_err() {
                // Best-effort cleanup if rename failed (e.g. cross-device).
                let _ = std::fs::remove_file(&tmp);
            }
        } else {
            // Best-effort cleanup of a partial tmp on write failure.
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

fn ensure_initialized() {
    if CACHE.read().ok().and_then(|g| g.as_ref().map(|_| ())).is_some() {
        return;
    }
    let (manifest, fetched_at) = match load_disk_cache() {
        Some((m, t)) => (m, Some(t)),
        None => (Manifest::from_bundled(), None),
    };
    if let Ok(mut w) = CACHE.write() {
        if w.is_none() {
            *w = Some(CacheState { manifest, fetched_at });
        }
    }
}

fn current_manifest() -> Manifest {
    ensure_initialized();
    CACHE
        .read()
        .ok()
        .and_then(|g| g.as_ref().map(|c| c.manifest.clone()))
        .unwrap_or_default()
}

fn current_fetched_at() -> Option<u64> {
    CACHE
        .read()
        .ok()
        .and_then(|g| g.as_ref().and_then(|c| c.fetched_at))
}

fn store(manifest: Manifest, fetched_at: u64) {
    if let Ok(mut w) = CACHE.write() {
        *w = Some(CacheState {
            manifest: manifest.clone(),
            fetched_at: Some(fetched_at),
        });
    }
    write_disk_cache(&manifest, fetched_at);
}

pub async fn refresh_from_remote() -> Result<(), String> {
    let client = reqwest::Client::builder()
        .user_agent("Corkscrew")
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .get(REMOTE_URL)
        .send()
        .await
        .map_err(|e| format!("fetch: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let text = resp.text().await.map_err(|e| e.to_string())?;
    let manifest: Manifest =
        serde_json::from_str(&text).map_err(|e| format!("parse: {e}"))?;
    store(manifest, unix_now());
    Ok(())
}

pub fn maybe_refresh_in_background() {
    ensure_initialized();
    let fetched_at = current_fetched_at();
    let should_refresh = match fetched_at {
        None => true,
        Some(t) => unix_now().saturating_sub(t) > CACHE_TTL_SECS,
    };
    if !should_refresh {
        return;
    }
    // This spawns onto Tauri's Tokio runtime and therefore must be invoked
    // from a context where that runtime is active. Callers (the verified-lists
    // Tauri commands) are `async fn`, so they execute on the runtime and a
    // reactor is present; calling this from a synchronous command would panic
    // with "there is no reactor running".
    tauri::async_runtime::spawn(async {
        if let Err(e) = refresh_from_remote().await {
            log::debug!("verified_lists refresh: {e}");
        }
    });
}

pub fn collection_status(game_domain: &str, slug: &str) -> VerifiedEntry {
    let m = current_manifest();
    m.collections
        .get(&game_domain.to_ascii_lowercase())
        .and_then(|inner| inner.get(slug))
        .cloned()
        .unwrap_or_else(VerifiedEntry::untested)
}

pub fn wabbajack_status(modlist_name: &str) -> VerifiedEntry {
    let m = current_manifest();
    m.wabbajack
        .get(modlist_name)
        .cloned()
        .unwrap_or_else(VerifiedEntry::untested)
}

pub fn full_manifest() -> Manifest {
    ensure_initialized();
    current_manifest()
}

pub fn cache_age_secs() -> Option<u64> {
    current_fetched_at().map(|t| unix_now().saturating_sub(t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_manifest_parses() {
        let m = Manifest::from_bundled();
        assert_eq!(m.schema_version, 1);
    }

    #[test]
    fn unknown_collection_is_untested() {
        let e = collection_status("skyrimspecialedition", "does-not-exist-xyz");
        assert_eq!(e.status, VerifiedStatus::Untested);
    }

    #[test]
    fn unknown_wabbajack_is_untested() {
        let e = wabbajack_status("no-such-modlist");
        assert_eq!(e.status, VerifiedStatus::Untested);
    }

    #[test]
    fn manifest_roundtrip() {
        let mut m = Manifest {
            schema_version: 1,
            ..Default::default()
        };
        let mut col = HashMap::new();
        col.insert(
            "test-slug".to_string(),
            VerifiedEntry {
                status: VerifiedStatus::Verified,
                version_tested: Some("1.0".to_string()),
                last_verified: Some("2026-04-20".to_string()),
                notes: Some("works".to_string()),
                reporter: None,
            },
        );
        m.collections.insert("skyrimspecialedition".to_string(), col);
        let json = serde_json::to_string(&m).unwrap();
        let back: Manifest = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.collections["skyrimspecialedition"]["test-slug"].status,
            VerifiedStatus::Verified
        );
    }
}
