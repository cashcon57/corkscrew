//! Configuration management for Corkscrew.
//!
//! Ported from `legacy-python/config.py`. Stores application configuration as
//! JSON in the platform-appropriate config directory:
//!   - macOS:  ~/Library/Application Support/corkscrew/config.json
//!   - Linux:  ~/.config/corkscrew/config.json

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Global lock for config file read-modify-write operations.
/// Prevents concurrent access from corrupting the JSON file.
static CONFIG_LOCK: Mutex<()> = Mutex::new(());

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
// VerificationLevel
// ---------------------------------------------------------------------------

/// Controls how thoroughly deployment health checks verify file integrity.
///
/// - **Fast**: File existence only (fastest, good for rapid mod development).
/// - **Balanced**: Existence + spot-check 10% of files by SHA-256 hash (default).
/// - **Paranoid**: Full SHA-256 verification of every deployed file (slowest).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub enum VerificationLevel {
    Paranoid,
    #[default]
    #[serde(alias = "balanced")]
    Balanced,
    Fast,
}

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

    /// Override for the staging directory (optional; falls back to the
    /// platform data directory when `None`). Setting this to a directory on
    /// the same filesystem as the game's Wine bottle enables hardlink
    /// deployment (zero disk overhead).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staging_dir: Option<String>,

    /// Whether the first-run setup wizard has been completed.
    #[serde(default)]
    pub has_completed_setup: bool,

    /// Whether controller/gamepad mode is enabled (larger UI targets for Steam Deck).
    #[serde(default)]
    pub controller_mode: bool,

    /// Verification level for deployment health checks.
    #[serde(default)]
    pub verification_level: VerificationLevel,

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

/// Returns the base data directory for Corkscrew application data.
///
/// - macOS:  `~/Library/Application Support/corkscrew`
/// - Linux:  `~/.local/share/corkscrew`
pub fn data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("corkscrew")
}

/// Returns the path to the downloads directory.
///
/// - macOS:  `~/Library/Application Support/corkscrew/downloads`
/// - Linux:  `~/.local/share/corkscrew/downloads`
pub fn downloads_dir() -> PathBuf {
    data_dir().join("downloads")
}

/// Returns the path to the cache directory.
///
/// - macOS:  `~/Library/Application Support/corkscrew/cache`
/// - Linux:  `~/.local/share/corkscrew/cache`
pub fn cache_dir() -> PathBuf {
    data_dir().join("cache")
}

// ---------------------------------------------------------------------------
// Config I/O
// ---------------------------------------------------------------------------

// Internal (unlocked) implementations — used by the public API to avoid
// deadlocks when `set_config_value` calls both read and write internally.

fn get_config_inner() -> Result<AppConfig> {
    let path = config_path();

    if !path.exists() {
        return Ok(AppConfig::default());
    }

    let contents = fs::read_to_string(&path)?;

    // Handle empty/whitespace-only files (e.g. from interrupted writes)
    let trimmed = contents.trim_start_matches('\u{feff}').trim();
    if trimmed.is_empty() {
        return Ok(AppConfig::default());
    }

    let config: AppConfig = serde_json::from_str(trimmed)?;
    Ok(config)
}

fn save_config_inner(config: &AppConfig) -> Result<()> {
    let path = config_path();

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let json = serde_json::to_string_pretty(config)?;
    let data = format!("{json}\n");

    // Atomic write: write to temp file then rename to avoid corruption
    // if the process is interrupted mid-write.
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, &data)?;

    // Set restrictive permissions (owner-only) since config may contain API keys
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600));
    }

    fs::rename(&tmp_path, &path)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Public (locked) API
// ---------------------------------------------------------------------------

/// Loads the application configuration from disk.
///
/// If the config file does not exist yet, a default (empty) `AppConfig` is
/// returned so callers never have to deal with a missing-file error.
pub fn get_config() -> Result<AppConfig> {
    let _lock = CONFIG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    get_config_inner()
}

/// Persists the given configuration to disk, creating parent directories as
/// needed.
pub fn save_config(config: &AppConfig) -> Result<()> {
    let _lock = CONFIG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    save_config_inner(config)
}

/// Sets a single configuration value by key name and saves to disk.
///
/// Known keys (`nexus_api_key`, `download_dir`) are written to their typed
/// fields; any other key is stored in the extensible `extra` map.
///
/// This acquires the config lock for the entire read-modify-write cycle to
/// prevent concurrent calls from corrupting the JSON file.
pub fn set_config_value(key: &str, value: &str) -> Result<()> {
    let _lock = CONFIG_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let mut config = get_config_inner()?;

    match key {
        "nexus_api_key" => {
            config.nexus_api_key = Some(value.to_owned());
        }
        "download_dir" => {
            config.download_dir = Some(value.to_owned());
        }
        "staging_dir" => {
            config.staging_dir = Some(value.to_owned());
        }
        "has_completed_setup" => {
            config.has_completed_setup = value == "true";
        }
        "controller_mode" => {
            config.controller_mode = value == "true";
        }
        "verification_level" => {
            config.verification_level = match value {
                "Fast" => VerificationLevel::Fast,
                "Paranoid" => VerificationLevel::Paranoid,
                _ => VerificationLevel::Balanced,
            };
        }
        _ => {
            config
                .extra
                .insert(key.to_owned(), serde_json::Value::String(value.to_owned()));
        }
    }

    save_config_inner(&config)
}

/// Retrieves a single configuration value by key name.
///
/// Returns `Ok(None)` when the key is not present or the config file does not
/// exist. Known keys are read from their typed fields; unknown keys are looked
/// up in the `extra` map.
pub fn get_config_value(key: &str) -> Result<Option<String>> {
    let _lock = CONFIG_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let config = get_config_inner()?;

    let value = match key {
        "nexus_api_key" => config.nexus_api_key,
        "download_dir" => config.download_dir,
        "staging_dir" => config.staging_dir,
        "has_completed_setup" => Some(config.has_completed_setup.to_string()),
        "controller_mode" => Some(config.controller_mode.to_string()),
        "verification_level" => Some(match config.verification_level {
            VerificationLevel::Fast => "Fast".to_string(),
            VerificationLevel::Balanced => "Balanced".to_string(),
            VerificationLevel::Paranoid => "Paranoid".to_string(),
        }),
        _ => config.extra.get(key).map(|v| match v {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
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
            staging_dir: None,
            has_completed_setup: false,
            controller_mode: false,
            verification_level: VerificationLevel::default(),
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

    // Workstream 5: Verification level serde tests

    #[test]
    fn verification_level_defaults_to_balanced() {
        let level = VerificationLevel::default();
        assert_eq!(level, VerificationLevel::Balanced);
    }

    #[test]
    fn verification_level_round_trips_through_json() {
        for level in [
            VerificationLevel::Fast,
            VerificationLevel::Balanced,
            VerificationLevel::Paranoid,
        ] {
            let json = serde_json::to_string(&level).unwrap();
            let restored: VerificationLevel = serde_json::from_str(&json).unwrap();
            assert_eq!(restored, level);
        }
    }

    #[test]
    fn config_without_verification_level_defaults_to_balanced() {
        // Simulate old config JSON that doesn't have verification_level
        let json = r#"{"nexus_api_key": null, "download_dir": null}"#;
        let config: AppConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.verification_level, VerificationLevel::Balanced);
    }

    #[test]
    fn config_with_paranoid_level_round_trips() {
        let mut config = AppConfig::default();
        config.verification_level = VerificationLevel::Paranoid;
        let json = serde_json::to_string(&config).unwrap();
        let restored: AppConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.verification_level, VerificationLevel::Paranoid);
    }
}
