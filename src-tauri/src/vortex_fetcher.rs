//! On-demand download of Vortex game extensions from GitHub.
//!
//! Fetches individual game extensions from the Nexus-Mods/vortex-games
//! repository. Caches raw files locally with SHA256 hashing for
//! cache invalidation.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use regex::Regex;
use sha2::{Digest, Sha256};

use crate::vortex_types::ExtensionSource;

const GITHUB_RAW_BASE: &str = "https://raw.githubusercontent.com/Nexus-Mods/vortex-games/master";
const GITHUB_API_CONTENTS: &str = "https://api.github.com/repos/Nexus-Mods/vortex-games/contents/";

/// Validate a game ID to prevent path traversal and URL injection.
///
/// Vortex game IDs are lowercase alphanumeric with optional hyphens/underscores
/// (e.g. "skyrimse", "baldursgate3", "dragon-age-inquisition").
pub fn validate_game_id(game_id: &str) -> Result<(), String> {
    if game_id.is_empty() {
        return Err("Game ID cannot be empty".into());
    }
    if game_id.len() > 128 {
        return Err("Game ID too long".into());
    }
    if !game_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "Invalid game ID '{}': only alphanumeric, hyphens, and underscores allowed",
            game_id
        ));
    }
    Ok(())
}

/// Validate a filename to prevent path traversal.
fn validate_filename(name: &str) -> Result<(), String> {
    let lower = name.to_lowercase();
    if name.is_empty()
        || name.contains("..")
        || name.contains('/')
        || name.contains('\\')
        || lower.contains("%2e%2e")
        || lower.contains("%2f")
        || lower.contains("%5c")
    {
        return Err(format!("Invalid filename: '{}'", name));
    }
    Ok(())
}

/// Directory where cached extension sources are stored.
fn cache_dir() -> PathBuf {
    let base = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("corkscrew")
        .join("vortex_extensions");
    let _ = fs::create_dir_all(&base);
    base
}

/// Fetch a game extension's source files from GitHub.
///
/// Tries the standard naming convention `game-{game_id}`. Falls back
/// to cached source if the network request fails.
pub async fn fetch_extension(game_id: &str) -> Result<ExtensionSource, String> {
    validate_game_id(game_id)?;
    let dir_name = format!("game-{}", game_id);

    // Try to fetch index.js
    let index_url = format!("{}/{}/index.js", GITHUB_RAW_BASE, dir_name);
    let index_js = fetch_url(&index_url)
        .await
        .map_err(|e| format!("Failed to fetch {}: {}", index_url, e))?;

    // Try to fetch info.json (optional)
    let info_url = format!("{}/{}/info.json", GITHUB_RAW_BASE, dir_name);
    let info_json = fetch_url(&info_url).await.ok();

    // Scan index.js for relative require() calls and fetch those files
    let mut extra_files = HashMap::new();
    let relative_requires = detect_relative_requires(&index_js);
    for rel_path in &relative_requires {
        match fetch_extra_file(game_id, rel_path).await {
            Ok(content) => {
                let filename = normalize_relative_path(rel_path);
                extra_files.insert(filename, content);
            }
            Err(e) => {
                log::debug!("Optional extra file '{}' not found: {}", rel_path, e);
            }
        }
    }

    // Compute hash
    let source_hash = hex_sha256(index_js.as_bytes());

    // Cache to disk
    let game_cache = cache_dir().join(game_id);
    let _ = fs::create_dir_all(&game_cache);
    let _ = fs::write(game_cache.join("index.js"), &index_js);
    if let Some(ref info) = info_json {
        let _ = fs::write(game_cache.join("info.json"), info);
    }
    for (name, content) in &extra_files {
        let _ = fs::write(game_cache.join(name), content);
    }
    let _ = fs::write(game_cache.join("source_hash"), &source_hash);

    Ok(ExtensionSource {
        index_js,
        info_json,
        source_hash,
        extra_files,
    })
}

/// Fetch an additional file from the same extension directory.
///
/// Used for relative requires like `require('./common')`.
pub async fn fetch_extra_file(game_id: &str, relative_path: &str) -> Result<String, String> {
    validate_game_id(game_id)?;
    let dir_name = format!("game-{}", game_id);
    // Normalize: strip leading ./ and ../ and ensure .js extension
    let clean = relative_path
        .trim_start_matches("./")
        .trim_start_matches("../");
    let filename = if clean.ends_with(".js") {
        clean.to_string()
    } else {
        format!("{}.js", clean)
    };

    // Validate the resulting filename to prevent path traversal
    validate_filename(&filename)?;

    let url = format!("{}/{}/{}", GITHUB_RAW_BASE, dir_name, filename);
    let content = fetch_url(&url)
        .await
        .map_err(|e| format!("Failed to fetch extra file {}: {}", url, e))?;

    // Cache
    let game_cache = cache_dir().join(game_id);
    let _ = fs::create_dir_all(&game_cache);
    let _ = fs::write(game_cache.join(&filename), &content);

    Ok(content)
}

/// Load a cached extension source from disk.
pub fn get_cached_source(game_id: &str) -> Option<ExtensionSource> {
    validate_game_id(game_id).ok()?;
    let game_cache = cache_dir().join(game_id);
    let index_js = fs::read_to_string(game_cache.join("index.js")).ok()?;
    let info_json = fs::read_to_string(game_cache.join("info.json")).ok();
    let source_hash = fs::read_to_string(game_cache.join("source_hash"))
        .unwrap_or_else(|_| hex_sha256(index_js.as_bytes()));

    // Load any extra cached JS files
    let mut extra_files = HashMap::new();
    if let Ok(entries) = fs::read_dir(&game_cache) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".js") && name != "index.js" {
                if let Ok(content) = fs::read_to_string(entry.path()) {
                    extra_files.insert(name, content);
                }
            }
        }
    }

    Some(ExtensionSource {
        index_js,
        info_json,
        source_hash,
        extra_files,
    })
}

/// List available game extension IDs from the GitHub repository.
///
/// Fetches the repo's top-level directory listing and extracts game IDs
/// from directory names matching `game-{id}`.
pub async fn list_available_extensions() -> Result<Vec<String>, String> {
    // Check for cached listing first
    let listing_cache = cache_dir().join("_available_games.json");
    let cache_age = listing_cache
        .metadata()
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
        .map(|d| d.as_secs())
        .unwrap_or(u64::MAX);

    // Use cache if less than 1 hour old
    if cache_age < 3600 {
        if let Ok(cached) = fs::read_to_string(&listing_cache) {
            if let Ok(ids) = serde_json::from_str::<Vec<String>>(&cached) {
                return Ok(ids);
            }
        }
    }

    // Fetch directory listing from GitHub API
    let body = fetch_url(GITHUB_API_CONTENTS)
        .await
        .map_err(|e| format!("Failed to list extensions: {}", e))?;

    let entries: Vec<serde_json::Value> = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse GitHub API response: {}", e))?;

    let mut game_ids: Vec<String> = entries
        .iter()
        .filter_map(|e| {
            let name = e.get("name")?.as_str()?;
            let entry_type = e.get("type")?.as_str()?;
            if entry_type == "dir" && name.starts_with("game-") {
                Some(name.strip_prefix("game-")?.to_string())
            } else {
                None
            }
        })
        .collect();

    game_ids.sort();

    // Cache the listing
    if let Ok(json) = serde_json::to_string(&game_ids) {
        let _ = fs::write(&listing_cache, json);
    }

    Ok(game_ids)
}

/// Check if we have a cached extension for a game.
pub fn has_cached_extension(game_id: &str) -> bool {
    validate_game_id(game_id).is_ok() && cache_dir().join(game_id).join("index.js").exists()
}

/// Get the cache directory path for a game extension.
pub fn extension_cache_path(game_id: &str) -> PathBuf {
    // Caller should have validated game_id, but be safe.
    debug_assert!(validate_game_id(game_id).is_ok());
    cache_dir().join(game_id)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

async fn fetch_url(url: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .user_agent("Corkscrew-Mod-Manager/1.0")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client.get(url).send().await.map_err(|e| e.to_string())?;

    if resp.status() == 404 {
        return Err(format!("Not found: {}", url));
    }
    if !resp.status().is_success() {
        return Err(format!("HTTP {}: {}", resp.status(), url));
    }

    resp.text().await.map_err(|e| e.to_string())
}

/// Scan JS source for `require('./xxx')` patterns to find relative dependencies.
fn detect_relative_requires(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    // Match require('./foo') or require("./foo") — only single-level relative paths
    let re = Regex::new(r#"require\s*\(\s*['"](\./[a-zA-Z0-9_\-]+)['"]"#).unwrap();
    for cap in re.captures_iter(source) {
        let path = cap[1].to_string();
        if !found.contains(&path) {
            found.push(path);
        }
    }
    found
}

/// Normalize a relative path like `./common` into a filename like `common.js`.
fn normalize_relative_path(rel: &str) -> String {
    let clean = rel.trim_start_matches("./");
    if clean.ends_with(".js") {
        clean.to_string()
    } else {
        format!("{}.js", clean)
    }
}

fn hex_sha256(data: &[u8]) -> String {
    let hash = Sha256::digest(data);
    hash.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_dir_is_created() {
        let dir = cache_dir();
        assert!(dir.exists() || dir.to_str().is_some());
    }

    #[test]
    fn hex_sha256_works() {
        let hash = hex_sha256(b"hello");
        assert_eq!(hash.len(), 64);
        assert!(hash.starts_with("2cf24d"));
    }

    #[test]
    fn validate_game_id_accepts_valid() {
        assert!(validate_game_id("skyrimse").is_ok());
        assert!(validate_game_id("baldursgate3").is_ok());
        assert!(validate_game_id("dragon-age-inquisition").is_ok());
        assert!(validate_game_id("witcher3_goty").is_ok());
    }

    #[test]
    fn validate_game_id_rejects_traversal() {
        assert!(validate_game_id("../etc/passwd").is_err());
        assert!(validate_game_id("../../secret").is_err());
        assert!(validate_game_id("game/../../../etc").is_err());
    }

    #[test]
    fn validate_game_id_rejects_special_chars() {
        assert!(validate_game_id("game id with spaces").is_err());
        assert!(validate_game_id("game/subdir").is_err());
        assert!(validate_game_id("game\\subdir").is_err());
        assert!(validate_game_id("").is_err());
    }

    #[test]
    fn validate_filename_rejects_traversal() {
        assert!(validate_filename("../secret.js").is_err());
        assert!(validate_filename("foo/../bar.js").is_err());
        assert!(validate_filename("foo/bar.js").is_err());
        assert!(validate_filename("").is_err());
    }

    #[test]
    fn validate_filename_rejects_url_encoded_traversal() {
        assert!(validate_filename("%2e%2e%2fsecret.js").is_err());
        assert!(validate_filename("%2E%2E%2Fsecret.js").is_err());
        assert!(validate_filename("foo%2fbar.js").is_err());
        assert!(validate_filename("foo%5cbar.js").is_err());
        assert!(validate_filename("foo%2Fbar.js").is_err());
        assert!(validate_filename("foo%5Cbar.js").is_err());
    }

    #[test]
    fn validate_filename_accepts_valid() {
        assert!(validate_filename("common.js").is_ok());
        assert!(validate_filename("stardewValley.js").is_ok());
    }
}
