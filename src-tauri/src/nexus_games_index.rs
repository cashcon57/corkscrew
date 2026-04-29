//! NexusMods full games index — cached locally.
//!
//! Hits `GET /v1/games.json` to fetch every game NexusMods knows about
//! (~3000 entries, ~few hundred KB) and caches the result on disk. Used by
//! the "show uninstalled games" feature in the game-selector dropdown so
//! users can see which titles Corkscrew supports before installing them.
//!
//! The bundled `vortex_extension_index.json` only covers the ~85 games with
//! Vortex extensions; this module fills the gap so the dropdown reflects
//! Nexus's full catalog.
//!
//! Cache TTL: 7 days. The list changes rarely (a few new games per month)
//! so a long TTL keeps users offline-friendly without going stale fast.
//!
//! Auth: required. Uses the current session's auth method (OAuth or API
//! key). When unauthenticated, the fetcher returns `Err` and callers should
//! fall back to the bundled index.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

const CACHE_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const CACHE_FILENAME: &str = "nexus_games_cache.json";

/// One entry from `GET /v1/games.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NexusGame {
    pub id: u64,
    pub name: String,
    pub domain_name: String,
    #[serde(default)]
    pub genre: String,
    #[serde(default)]
    pub mods: u64,
    #[serde(default)]
    pub downloads: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CacheFile {
    fetched_at_unix: u64,
    games: Vec<NexusGame>,
}

fn cache_path() -> PathBuf {
    crate::config::cache_dir().join(CACHE_FILENAME)
}

/// Read the cache from disk if it exists and is younger than [`CACHE_TTL`].
pub fn load_cached() -> Option<Vec<NexusGame>> {
    let path = cache_path();
    let bytes = fs::read(&path).ok()?;
    let cache: CacheFile = serde_json::from_slice(&bytes).ok()?;
    let fetched_at = SystemTime::UNIX_EPOCH + Duration::from_secs(cache.fetched_at_unix);
    if SystemTime::now()
        .duration_since(fetched_at)
        .map(|age| age < CACHE_TTL)
        .unwrap_or(false)
    {
        Some(cache.games)
    } else {
        None
    }
}

/// Read whatever is in the cache regardless of age. Used as a fallback when
/// a refresh fails so we still surface *something* rather than nothing.
pub fn load_stale() -> Option<Vec<NexusGame>> {
    let path = cache_path();
    let bytes = fs::read(&path).ok()?;
    let cache: CacheFile = serde_json::from_slice(&bytes).ok()?;
    Some(cache.games)
}

/// Write the games list to the on-disk cache atomically.
fn write_cache(games: &[NexusGame]) -> std::io::Result<()> {
    let path = cache_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let cache = CacheFile {
        fetched_at_unix: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        games: games.to_vec(),
    };
    let json = serde_json::to_vec_pretty(&cache).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

/// Fetch the full NexusMods games list, using cached data when fresh.
///
/// Returns `Err` if the cache is stale/missing AND the network fetch fails.
/// Callers should fall back to `vortex_index` when this errors.
pub async fn get_games() -> Result<Vec<NexusGame>, String> {
    if let Some(cached) = load_cached() {
        return Ok(cached);
    }

    // Cache stale — refresh from the API. Auth is required; without it the
    // endpoint 401s. We bubble that up so callers know to fall back.
    let auth_method = crate::oauth::get_auth_method_refreshed().await;
    let client = crate::nexus::NexusClient::from_auth_method(&auth_method)
        .map_err(|e| format!("NexusMods sign-in required: {e}"))?;

    let games = client
        .fetch_games_list()
        .await
        .map_err(|e| format!("Failed to fetch NM games list: {e}"))?;

    if let Err(e) = write_cache(&games) {
        log::warn!("Failed to write nexus games cache: {e}");
    }

    Ok(games)
}
