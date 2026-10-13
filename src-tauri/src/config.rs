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
// ExperimentalConfig
// ---------------------------------------------------------------------------

/// Opt-in experimental features that may graduate to permanent settings later.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ExperimentalConfig {
    /// When true, surface Native Mode UI for games that support macOS-native
    /// modding (e.g. Stardew Valley, Baldur's Gate 3). Off by default — opt-in
    /// only, as native-mode modding is not yet the primary supported workflow.
    #[serde(default)]
    pub native_mode: bool,

    /// When true, the Native Mode toggle button is visible in the topbar and
    /// the first-run banner can appear. Off by default — native macOS modding
    /// is in active development and does not yet function for end users.
    #[serde(default)]
    pub native_mode_visible: bool,
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

    /// Legacy field — ignored. Kept for backwards-compatible deserialization
    /// of existing config files.
    #[serde(default)]
    pub use_original_engine_fixes: bool,

    /// If true, deploy the Wine fork of SSE Engine Fixes before Skyrim SE
    /// launches. This is opt-in because the Wine fork is in active development
    /// and may introduce issues. May fix some Wine-specific crashes in large
    /// modlists.
    ///
    /// Read this through [`engine_fixes_wine_enabled`] — never gate on the
    /// legacy `use_original_engine_fixes` field, which previously caused
    /// redeploy and launch to disagree about whether the Wine fork is active.
    #[serde(default)]
    pub use_wine_engine_fixes: bool,

    /// If true, surface The Sims 4 adult-content sources (e.g. LoversLab)
    /// in tool listings. Off by default — explicit opt-in required since
    /// these sources host NSFW mods.
    #[serde(default)]
    pub enable_adult_content_for_sims4: bool,

    /// Experimental / opt-in features (native mode, etc.).
    /// Deserializes gracefully from old configs that lack this block.
    #[serde(default)]
    pub experimental: ExperimentalConfig,

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
    // E2E tests can override with CORKSCREW_DATA_DIR to use fixture data.
    if let Ok(test_dir) = std::env::var("CORKSCREW_DATA_DIR") {
        return PathBuf::from(&test_dir).join("config.json");
    }
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
    if let Ok(test_dir) = std::env::var("CORKSCREW_DATA_DIR") {
        return PathBuf::from(&test_dir).join("mods.db");
    }
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
    if let Ok(test_dir) = std::env::var("CORKSCREW_DATA_DIR") {
        return PathBuf::from(test_dir);
    }
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
        // Set restrictive umask before creating directory to avoid TOCTOU race
        #[cfg(unix)]
        let _old_umask = unsafe { libc::umask(0o077) };
        let dir_result = fs::create_dir_all(parent);
        #[cfg(unix)]
        unsafe {
            libc::umask(_old_umask);
        }
        dir_result?;
    }

    let json = serde_json::to_string_pretty(config)?;
    let data = format!("{json}\n");

    // Atomic write: write to temp file then rename to avoid corruption
    // if the process is interrupted mid-write.
    let tmp_path = path.with_extension("json.tmp");

    // Write with restrictive permissions atomically (no TOCTOU window)
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp_path)?;
        file.write_all(data.as_bytes())?;
    }
    #[cfg(not(unix))]
    {
        fs::write(&tmp_path, &data)?;
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

/// Single source of truth for the "deploy SSEEngineFixesForWine" decision.
/// Opt-in, default off. Every gate (launch, CLI, redeploy, collection
/// install) must use this so the deployed state can't depend on which code
/// path ran last.
pub fn engine_fixes_wine_enabled() -> bool {
    get_config().map(|c| c.use_wine_engine_fixes).unwrap_or(false)
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
        "use_original_engine_fixes" => {
            // Legacy — keep for compat but no-op
            config.use_original_engine_fixes = value == "true";
        }
        "use_wine_engine_fixes" => {
            config.use_wine_engine_fixes = value == "true";
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
        "use_original_engine_fixes" => Some(config.use_original_engine_fixes.to_string()),
        "use_wine_engine_fixes" => Some(config.use_wine_engine_fixes.to_string()),
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
            use_original_engine_fixes: false,
            use_wine_engine_fixes: false,
            enable_adult_content_for_sims4: false,
            experimental: ExperimentalConfig::default(),
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

    // Task 1.6: ExperimentalConfig / native_mode tests

    #[test]
    fn native_mode_default_is_false() {
        // Fresh AppConfig (Default) must have experimental.native_mode = false.
        let cfg = AppConfig::default();
        assert!(
            !cfg.experimental.native_mode,
            "experimental.native_mode should default to false"
        );
    }

    #[test]
    fn native_mode_round_trip() {
        // Set native_mode = true, serialize, deserialize, assert it survived.
        let mut cfg = AppConfig::default();
        cfg.experimental.native_mode = true;
        let json = serde_json::to_string(&cfg).unwrap();
        let restored: AppConfig = serde_json::from_str(&json).unwrap();
        assert!(
            restored.experimental.native_mode,
            "native_mode should survive a JSON round-trip as true"
        );
    }

    #[test]
    fn loading_old_config_without_experimental_block_defaults_native_mode_false() {
        // Simulate a pre-Task-1.6 config file that has no "experimental" key.
        // The #[serde(default)] annotation should fill it in with false.
        let old_json = r#"{}"#;
        let cfg: AppConfig = serde_json::from_str(old_json).unwrap();
        assert!(
            !cfg.experimental.native_mode,
            "old config without experimental block should default native_mode to false"
        );
    }

    // native_mode_visible tests

    #[test]
    fn native_mode_visible_default_is_false() {
        let cfg = AppConfig::default();
        assert!(
            !cfg.experimental.native_mode_visible,
            "experimental.native_mode_visible should default to false"
        );
    }

    #[test]
    fn native_mode_visible_round_trips() {
        let mut cfg = AppConfig::default();
        cfg.experimental.native_mode_visible = true;
        let json = serde_json::to_string(&cfg).unwrap();
        let restored: AppConfig = serde_json::from_str(&json).unwrap();
        assert!(
            restored.experimental.native_mode_visible,
            "native_mode_visible should survive a JSON round-trip as true"
        );
        // Also verify the false case is preserved independently
        let mut cfg2 = AppConfig::default();
        cfg2.experimental.native_mode_visible = false;
        let json2 = serde_json::to_string(&cfg2).unwrap();
        let restored2: AppConfig = serde_json::from_str(&json2).unwrap();
        assert!(
            !restored2.experimental.native_mode_visible,
            "native_mode_visible should survive a JSON round-trip as false"
        );
    }
}
