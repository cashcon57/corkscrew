//! Bottle configuration reader/writer.
//!
//! Reads and modifies CrossOver-style `cxbottle.conf` INI files and
//! generic Wine bottle settings. Changes are written back in the native
//! format so they transfer seamlessly to CrossOver or other managers.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::bottles::Bottle;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum BottleConfigError {
    #[error("bottle config file not found: {0}")]
    NotFound(PathBuf),

    #[error("failed to read bottle config: {0}")]
    Io(#[from] std::io::Error),

    #[error("unsupported bottle source: {0}")]
    UnsupportedSource(String),
}

pub type Result<T> = std::result::Result<T, BottleConfigError>;

// ---------------------------------------------------------------------------
// Windows version templates
// ---------------------------------------------------------------------------

/// Known Windows version templates and their display names.
pub const WINDOWS_VERSIONS: &[(&str, &str)] = &[
    ("win10_64", "Windows 10 (64-bit)"),
    ("win81_64", "Windows 8.1 (64-bit)"),
    ("win8_64", "Windows 8 (64-bit)"),
    ("win7_64", "Windows 7 (64-bit)"),
    ("win7_32", "Windows 7 (32-bit)"),
    ("winxp", "Windows XP"),
    ("win2k", "Windows 2000"),
];

// ---------------------------------------------------------------------------
// Bottle settings struct
// ---------------------------------------------------------------------------

/// Represents the editable settings for a bottle.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BottleSettings {
    /// Bottle display name.
    pub name: String,
    /// Bottle source manager (CrossOver, Whisky, etc.).
    pub source: String,
    /// Absolute path to bottle.
    pub path: String,
    /// Architecture: "win32" or "win64".
    pub arch: String,
    /// Windows version template (e.g., "win10_64").
    pub windows_version: String,
    /// CrossOver version that created/updated the bottle.
    pub crossover_version: String,
    /// Whether msync is enabled (performance).
    pub msync_enabled: bool,
    /// Whether MetalFX upscaling is enabled.
    pub metalfx_enabled: bool,
    /// Whether DXMT NVIDIA extensions are enabled.
    pub dxmt_nvext_enabled: bool,
    /// All environment variables set on the bottle.
    pub env_vars: HashMap<String, String>,
    /// Whether this bottle has a cxbottle.conf (CrossOver-managed).
    pub has_native_config: bool,
}

/// A single setting definition for the frontend to render.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BottleSettingDef {
    pub key: String,
    pub label: String,
    pub description: String,
    pub setting_type: SettingType,
    pub recommended: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SettingType {
    Toggle { current: bool },
    Select { current: String, options: Vec<SelectOption> },
    ReadOnly { value: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelectOption {
    pub value: String,
    pub label: String,
}

// ---------------------------------------------------------------------------
// INI parsing (cxbottle.conf format)
// ---------------------------------------------------------------------------

/// Parsed representation of a cxbottle.conf file.
#[derive(Debug, Default)]
struct CxBottleConf {
    /// [Bottle] section key-value pairs.
    bottle: HashMap<String, String>,
    /// [EnvironmentVariables] section key-value pairs.
    env_vars: HashMap<String, String>,
    /// Raw lines for sections we don't parse (preserved for round-trip).
    raw_sections: Vec<(String, Vec<String>)>,
}

fn parse_cxbottle_conf(path: &Path) -> Result<CxBottleConf> {
    let content = fs::read_to_string(path)?;
    let mut conf = CxBottleConf::default();

    let mut current_section = String::new();
    let mut current_lines: Vec<String> = Vec::new();
    let mut known_section = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Section header
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            // Save previous unknown section
            if !known_section && !current_section.is_empty() {
                conf.raw_sections.push((current_section.clone(), current_lines.clone()));
            }
            current_section = trimmed[1..trimmed.len() - 1].to_string();
            current_lines = Vec::new();
            known_section = matches!(current_section.as_str(), "Bottle" | "EnvironmentVariables");
            continue;
        }

        // Key = Value
        if let Some(eq_pos) = trimmed.find('=') {
            let key = trimmed[..eq_pos].trim().to_string();
            let value = trimmed[eq_pos + 1..].trim().to_string();

            match current_section.as_str() {
                "Bottle" => { conf.bottle.insert(key, value); }
                "EnvironmentVariables" => { conf.env_vars.insert(key, value); }
                _ => { current_lines.push(line.to_string()); }
            }
        } else if !known_section {
            current_lines.push(line.to_string());
        }
    }

    // Save last unknown section
    if !known_section && !current_section.is_empty() {
        conf.raw_sections.push((current_section, current_lines));
    }

    Ok(conf)
}

fn write_cxbottle_conf(path: &Path, conf: &CxBottleConf) -> Result<()> {
    let mut output = String::new();

    // Write [Bottle] section
    output.push_str("[Bottle]\n");
    // Write in a stable order: known keys first, then alphabetical
    let known_keys = ["WineArch", "BottleID", "Version", "Timestamp", "Encoding",
                      "Template", "MenuRoot", "MenuStrip", "MenuMode", "AssocMode"];
    for key in &known_keys {
        if let Some(val) = conf.bottle.get(*key) {
            output.push_str(&format!("{} = {}\n", key, val));
        }
    }
    // Remaining keys alphabetically
    let mut remaining: Vec<_> = conf.bottle.iter()
        .filter(|(k, _)| !known_keys.contains(&k.as_str()))
        .collect();
    remaining.sort_by_key(|(k, _)| (*k).clone());
    for (key, val) in remaining {
        output.push_str(&format!("{} = {}\n", key, val));
    }

    // Write [EnvironmentVariables] section
    if !conf.env_vars.is_empty() {
        output.push_str("\n[EnvironmentVariables]\n");
        let mut vars: Vec<_> = conf.env_vars.iter().collect();
        vars.sort_by_key(|(k, _)| (*k).clone());
        for (key, val) in vars {
            output.push_str(&format!("{} = {}\n", key, val));
        }
    }

    // Write preserved sections
    for (section, lines) in &conf.raw_sections {
        output.push_str(&format!("\n[{}]\n", section));
        for line in lines {
            output.push_str(line);
            output.push('\n');
        }
    }

    // Atomic write
    let tmp_path = path.with_extension("conf.tmp");
    fs::write(&tmp_path, &output)?;
    fs::rename(&tmp_path, path)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Path to the cxbottle.conf file for a given bottle.
fn conf_path(bottle: &Bottle) -> PathBuf {
    bottle.path.join("cxbottle.conf")
}

/// Read the current settings for a bottle.
pub fn get_bottle_settings(bottle: &Bottle) -> Result<BottleSettings> {
    let conf_file = conf_path(bottle);
    let has_native = conf_file.exists();

    if has_native {
        let conf = parse_cxbottle_conf(&conf_file)?;

        Ok(BottleSettings {
            name: bottle.name.clone(),
            source: bottle.source.clone(),
            path: bottle.path.to_string_lossy().into_owned(),
            arch: conf.bottle.get("WineArch").cloned().unwrap_or_else(|| "win64".to_string()),
            windows_version: conf.bottle.get("Template").cloned().unwrap_or_else(|| "win10_64".to_string()),
            crossover_version: conf.bottle.get("Version").cloned().unwrap_or_default(),
            msync_enabled: conf.env_vars.get("WINEMSYNC").map(|v| v == "1").unwrap_or(false),
            metalfx_enabled: conf.env_vars.get("D3DM_ENABLE_METALFX").map(|v| v == "1").unwrap_or(false),
            dxmt_nvext_enabled: conf.env_vars.get("DXMT_ENABLE_NVEXT").map(|v| v == "1").unwrap_or(false),
            env_vars: conf.env_vars,
            has_native_config: true,
        })
    } else {
        // Generic Wine bottle without cxbottle.conf — read-only basics
        Ok(BottleSettings {
            name: bottle.name.clone(),
            source: bottle.source.clone(),
            path: bottle.path.to_string_lossy().into_owned(),
            arch: if bottle.path.join("drive_c").join("windows").join("syswow64").exists() {
                "win64".to_string()
            } else {
                "win32".to_string()
            },
            windows_version: "unknown".to_string(),
            crossover_version: String::new(),
            msync_enabled: false,
            metalfx_enabled: false,
            dxmt_nvext_enabled: false,
            env_vars: HashMap::new(),
            has_native_config: false,
        })
    }
}

/// Update a single setting on a bottle's cxbottle.conf.
pub fn set_bottle_setting(bottle: &Bottle, key: &str, value: &str) -> Result<()> {
    let conf_file = conf_path(bottle);
    if !conf_file.exists() {
        return Err(BottleConfigError::NotFound(conf_file));
    }

    let mut conf = parse_cxbottle_conf(&conf_file)?;

    match key {
        "windows_version" => {
            conf.bottle.insert("Template".to_string(), value.to_string());
        }
        "msync_enabled" => {
            if value == "true" || value == "1" {
                conf.env_vars.insert("WINEMSYNC".to_string(), "1".to_string());
            } else {
                conf.env_vars.remove("WINEMSYNC");
            }
        }
        "metalfx_enabled" => {
            if value == "true" || value == "1" {
                conf.env_vars.insert("D3DM_ENABLE_METALFX".to_string(), "1".to_string());
            } else {
                conf.env_vars.remove("D3DM_ENABLE_METALFX");
            }
        }
        "dxmt_nvext_enabled" => {
            if value == "true" || value == "1" {
                conf.env_vars.insert("DXMT_ENABLE_NVEXT".to_string(), "1".to_string());
            } else {
                conf.env_vars.remove("DXMT_ENABLE_NVEXT");
            }
        }
        // Generic env var: "env.VARIABLE_NAME"
        key if key.starts_with("env.") => {
            let var_name = &key[4..];
            if value.is_empty() {
                conf.env_vars.remove(var_name);
            } else {
                conf.env_vars.insert(var_name.to_string(), value.to_string());
            }
        }
        _ => {
            return Err(BottleConfigError::UnsupportedSource(
                format!("unknown setting key: {}", key),
            ));
        }
    }

    // Update timestamp
    let now = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    conf.bottle.insert("Timestamp".to_string(), now);

    write_cxbottle_conf(&conf_file, &conf)
}

/// Get the list of setting definitions for the frontend to render.
pub fn get_setting_definitions(settings: &BottleSettings) -> Vec<BottleSettingDef> {
    let mut defs = Vec::new();

    // Read-only info
    defs.push(BottleSettingDef {
        key: "arch".to_string(),
        label: "Architecture".to_string(),
        description: "Windows architecture for this bottle".to_string(),
        setting_type: SettingType::ReadOnly {
            value: if settings.arch == "win64" { "64-bit".to_string() } else { "32-bit".to_string() },
        },
        recommended: None,
    });

    if !settings.crossover_version.is_empty() {
        defs.push(BottleSettingDef {
            key: "crossover_version".to_string(),
            label: "CrossOver Version".to_string(),
            description: "Version of CrossOver that manages this bottle".to_string(),
            setting_type: SettingType::ReadOnly {
                value: settings.crossover_version.clone(),
            },
            recommended: None,
        });
    }

    // Editable settings (only for CrossOver bottles with cxbottle.conf)
    if settings.has_native_config {
        // Windows version
        defs.push(BottleSettingDef {
            key: "windows_version".to_string(),
            label: "Windows Version".to_string(),
            description: "Which version of Windows to emulate. Most games work best with Windows 10.".to_string(),
            setting_type: SettingType::Select {
                current: settings.windows_version.clone(),
                options: WINDOWS_VERSIONS.iter().map(|(val, label)| SelectOption {
                    value: val.to_string(),
                    label: label.to_string(),
                }).collect(),
            },
            recommended: Some("win10_64".to_string()),
        });

        // MSync
        defs.push(BottleSettingDef {
            key: "msync_enabled".to_string(),
            label: "MSync".to_string(),
            description: "Faster synchronization primitive for multi-threaded games. Improves performance in most titles.".to_string(),
            setting_type: SettingType::Toggle { current: settings.msync_enabled },
            recommended: Some("true".to_string()),
        });

        // MetalFX
        defs.push(BottleSettingDef {
            key: "metalfx_enabled".to_string(),
            label: "MetalFX Upscaling".to_string(),
            description: "Apple's GPU-accelerated upscaling for better frame rates. Recommended for most games on macOS.".to_string(),
            setting_type: SettingType::Toggle { current: settings.metalfx_enabled },
            recommended: Some("true".to_string()),
        });

        // DXMT NVIDIA Extensions
        defs.push(BottleSettingDef {
            key: "dxmt_nvext_enabled".to_string(),
            label: "DXMT NVIDIA Extensions".to_string(),
            description: "Extended D3D-to-Metal translation features. Improves compatibility with some DirectX effects.".to_string(),
            setting_type: SettingType::Toggle { current: settings.dxmt_nvext_enabled },
            recommended: Some("true".to_string()),
        });
    }

    defs
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn sample_conf() -> &'static str {
        "[Bottle]\n\
         WineArch = win64\n\
         BottleID = TEST-1234\n\
         Version = 26.0.0.39794\n\
         Template = win10_64\n\
         \n\
         [EnvironmentVariables]\n\
         WINEMSYNC = 1\n\
         D3DM_ENABLE_METALFX = 1\n\
         DXMT_ENABLE_NVEXT = 1\n\
         \n\
         [ManagedUpdatePolicy]\n\
         system.reg = Registry,ReplaceFiles\n"
    }

    #[test]
    fn parse_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cxbottle.conf");
        fs::write(&path, sample_conf()).unwrap();

        let conf = parse_cxbottle_conf(&path).unwrap();
        assert_eq!(conf.bottle.get("WineArch").unwrap(), "win64");
        assert_eq!(conf.bottle.get("Template").unwrap(), "win10_64");
        assert_eq!(conf.env_vars.get("WINEMSYNC").unwrap(), "1");
        assert_eq!(conf.raw_sections.len(), 1);
        assert_eq!(conf.raw_sections[0].0, "ManagedUpdatePolicy");
    }

    #[test]
    fn write_preserves_unknown_sections() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cxbottle.conf");
        fs::write(&path, sample_conf()).unwrap();

        let conf = parse_cxbottle_conf(&path).unwrap();
        write_cxbottle_conf(&path, &conf).unwrap();

        let reread = parse_cxbottle_conf(&path).unwrap();
        assert_eq!(reread.raw_sections.len(), 1);
        assert_eq!(reread.raw_sections[0].0, "ManagedUpdatePolicy");
    }

    #[test]
    fn settings_from_conf() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_dir = tmp.path().join("TestBottle");
        fs::create_dir_all(bottle_dir.join("drive_c")).unwrap();
        fs::write(bottle_dir.join("cxbottle.conf"), sample_conf()).unwrap();

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_dir,
            source: "CrossOver".into(),
        };

        let settings = get_bottle_settings(&bottle).unwrap();
        assert_eq!(settings.arch, "win64");
        assert_eq!(settings.windows_version, "win10_64");
        assert!(settings.msync_enabled);
        assert!(settings.metalfx_enabled);
        assert!(settings.has_native_config);
    }

    #[test]
    fn set_toggle_setting() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_dir = tmp.path().join("TestBottle");
        fs::create_dir_all(bottle_dir.join("drive_c")).unwrap();
        fs::write(bottle_dir.join("cxbottle.conf"), sample_conf()).unwrap();

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_dir,
            source: "CrossOver".into(),
        };

        // Disable msync
        set_bottle_setting(&bottle, "msync_enabled", "false").unwrap();
        let settings = get_bottle_settings(&bottle).unwrap();
        assert!(!settings.msync_enabled);

        // Re-enable
        set_bottle_setting(&bottle, "msync_enabled", "true").unwrap();
        let settings = get_bottle_settings(&bottle).unwrap();
        assert!(settings.msync_enabled);
    }

    #[test]
    fn set_windows_version() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_dir = tmp.path().join("TestBottle");
        fs::create_dir_all(bottle_dir.join("drive_c")).unwrap();
        fs::write(bottle_dir.join("cxbottle.conf"), sample_conf()).unwrap();

        let bottle = Bottle {
            name: "TestBottle".into(),
            path: bottle_dir,
            source: "CrossOver".into(),
        };

        set_bottle_setting(&bottle, "windows_version", "win7_64").unwrap();
        let settings = get_bottle_settings(&bottle).unwrap();
        assert_eq!(settings.windows_version, "win7_64");
    }
}
