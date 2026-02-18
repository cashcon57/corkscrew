//! Configuration management for Corkscrew.
//!
//! Ported from `legacy-python/config.py`. Stores application configuration as
//! JSON in the platform-appropriate config directory:
//!   - macOS:  ~/Library/Application Support/corkscrew/config.json
//!   - Linux:  ~/.config/corkscrew/config.json

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read or write config file: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to parse or serialize config JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("could not determine platform config directory")]
    NoConfigDir,

    #[error("could not determine platform data directory")]
    NoDataDir,
}

pub type Result<T> = std::result::Result<T, ConfigError>;

// ---------------------------------------------------------------------------
// AppConfig
// ---------------------------------------------------------------------------

/// Top-level application configuration persisted as JSON.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AppConfig {
    /// Nexus Mods API key (optional until the user configures it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nexus_api_key: Option<String>,

    /// Override for the download directory (optional; falls back to the
    /// platform data directory when `None`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_dir: Option<String>,

    /// Catch-all for any additional settings that may be added in the future.
    /// Flattened so extra keys sit at the top level of the JSON object.
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Returns the path to the configuration file.
///
/// - macOS:  `~/Library/Application Support/corkscrew/config.json`
/// - Linux:  `~/.config/corkscrew/config.json`
pub fn config_path() -> PathBuf {
    // dirs::config_dir() returns None only on truly exotic platforms; for a
    // desktop app we treat that as a hard failure elsewhere, but here we
    // provide a best-effort fallback so the function stays infallible.
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("corkscrew")
        .join("config.json")
}

/// Returns the path to the SQLite mod database.
///
/// Stored under the platform-local data directory:
/// - macOS:  `~/Library/Application Support/corkscrew/mods.db`
/// - Linux:  `~/.local/share/corkscrew/mods.db`
pub fn db_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("corkscrew")
        .join("mods.db")
}

/// Returns the path to the downloads directory.
///
/// - macOS:  `~/Library/Application Support/corkscrew/downloads`
/// - Linux:  `~/.local/share/corkscrew/downloads`
pub fn downloads_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("corkscrew")
        .join("downloads")
}

// ---------------------------------------------------------------------------
// Config I/O
// ---------------------------------------------------------------------------

/// Loads the application configuration from disk.
///
/// If the config file does not exist yet, a default (empty) `AppConfig` is
/// returned so callers never have to deal with a missing-file error.
pub fn get_config() -> Result<AppConfig> {
    let path = config_path();

    if !path.exists() {
        return Ok(AppConfig::default());
    }

    let contents = fs::read_to_string(&path)?;
    let config: AppConfig = serde_json::from_str(&contents)?;
    Ok(config)
}

/// Persists the given configuration to disk, creating parent directories as
/// needed.
pub fn save_config(config: &AppConfig) -> Result<()> {
    let path = config_path();

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let json = serde_json::to_string_pretty(config)?;
    // Match the Python version's trailing newline.
    fs::write(&path, format!("{json}\n"))?;
    Ok(())
}

/// Sets a single configuration value by key name and saves to disk.
///
/// Known keys (`nexus_api_key`, `download_dir`) are written to their typed
/// fields; any other key is stored in the extensible `extra` map.
pub fn set_config_value(key: &str, value: &str) -> Result<()> {
    let mut config = get_config()?;

    match key {
        "nexus_api_key" => {
            config.nexus_api_key = Some(value.to_owned());
        }
        "download_dir" => {
            config.download_dir = Some(value.to_owned());
        }
        _ => {
            config.extra.insert(
                key.to_owned(),
                serde_json::Value::String(value.to_owned()),
            );
        }
    }

    save_config(&config)
}

/// Retrieves a single configuration value by key name.
///
/// Returns `Ok(None)` when the key is not present or the config file does not
/// exist. Known keys are read from their typed fields; unknown keys are looked
/// up in the `extra` map.
pub fn get_config_value(key: &str) -> Result<Option<String>> {
    let config = get_config()?;

    let value = match key {
        "nexus_api_key" => config.nexus_api_key,
        "download_dir" => config.download_dir,
        _ => config.extra.get(key).and_then(|v| match v {
            serde_json::Value::String(s) => Some(s.clone()),
            other => Some(other.to_string()),
        }),
    };

    Ok(value)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// We test the pure logic (ser/de, path construction) rather than full I/O
    /// against the real filesystem so tests never touch real user files.

    #[test]
    fn default_config_round_trips_through_json() {
        let config = AppConfig::default();
        let json = serde_json::to_string_pretty(&config).unwrap();
        let restored: AppConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.nexus_api_key, None);
        assert_eq!(restored.download_dir, None);
        assert!(restored.extra.is_empty());
    }

    #[test]
    fn config_with_extra_keys_round_trips() {
        let mut config = AppConfig {
            nexus_api_key: Some("abc123".into()),
            download_dir: Some("/tmp/mods".into()),
            extra: HashMap::new(),
        };
        config
            .extra
            .insert("theme".into(), serde_json::Value::String("dark".into()));

        let json = serde_json::to_string(&config).unwrap();
        let restored: AppConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.nexus_api_key.as_deref(), Some("abc123"));
        assert_eq!(restored.download_dir.as_deref(), Some("/tmp/mods"));
        assert_eq!(
            restored.extra.get("theme"),
            Some(&serde_json::Value::String("dark".into()))
        );
    }

    #[test]
    fn config_path_ends_with_expected_segments() {
        let p = config_path();
        assert!(p.ends_with("corkscrew/config.json"));
    }

    #[test]
    fn db_path_ends_with_expected_segments() {
        let p = db_path();
        assert!(p.ends_with("corkscrew/mods.db"));
    }

    #[test]
    fn downloads_dir_ends_with_expected_segments() {
        let p = downloads_dir();
        assert!(p.ends_with("corkscrew/downloads"));
    }
}
