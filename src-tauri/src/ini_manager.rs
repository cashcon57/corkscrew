//! INI tweak manager for Skyrim configuration files within Wine bottles.
//!
//! Parses Skyrim.ini, SkyrimPrefs.ini, and SkyrimCustom.ini, presenting
//! settings in a structured format. Supports presets and per-setting edits.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::bottles::Bottle;

#[derive(Debug, Error)]
pub enum IniError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("INI file not found: {0}")]
    NotFound(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, IniError>;

/// A single INI setting.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IniSetting {
    pub file_name: String,
    pub section: String,
    pub key: String,
    pub value: String,
}

/// All settings from a Skyrim INI file, grouped by section.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IniFile {
    pub file_name: String,
    pub path: String,
    pub sections: BTreeMap<String, BTreeMap<String, String>>,
}

/// Preset for INI settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IniPreset {
    pub name: String,
    pub description: String,
    pub settings: Vec<IniSetting>,
}

/// Find game INI files within a wine bottle.
pub fn find_ini_files(bottle: &Bottle, game_id: &str) -> Vec<PathBuf> {
    let appdata_local = bottle.appdata_local();

    let (dir_name, ini_names): (&str, &[&str]) = match game_id {
        "skyrimse" => (
            "Skyrim Special Edition",
            &["Skyrim.ini", "SkyrimPrefs.ini", "SkyrimCustom.ini"],
        ),
        "skyrim" => ("Skyrim", &["Skyrim.ini", "SkyrimPrefs.ini", "SkyrimCustom.ini"]),
        "fallout4" => ("Fallout4", &["Fallout4.ini", "Fallout4Prefs.ini"]),
        _ => return Vec::new(),
    };

    let game_dir = appdata_local.join(dir_name);
    let mut found = Vec::new();

    for name in ini_names {
        let path = game_dir.join(name);
        if path.exists() {
            found.push(path);
        }
    }

    found
}

/// Parse an INI file into sections and key-value pairs.
pub fn parse_ini(path: &Path) -> Result<IniFile> {
    if !path.exists() {
        return Err(IniError::NotFound(path.to_string_lossy().to_string()));
    }

    let content = fs::read_to_string(path)?;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let mut sections: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut current_section = String::new();

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
            continue;
        }

        // Section header
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            current_section = trimmed[1..trimmed.len() - 1].to_string();
            sections.entry(current_section.clone()).or_default();
            continue;
        }

        // Key=Value pair
        if let Some(eq_pos) = trimmed.find('=') {
            let key = trimmed[..eq_pos].trim().to_string();
            let value = trimmed[eq_pos + 1..].trim().to_string();
            if !current_section.is_empty() {
                sections
                    .entry(current_section.clone())
                    .or_default()
                    .insert(key, value);
            }
        }
    }

    Ok(IniFile {
        file_name,
        path: path.to_string_lossy().to_string(),
        sections,
    })
}

/// Read all Skyrim INI files from a bottle.
pub fn read_all_ini(bottle: &Bottle, game_id: &str) -> Vec<IniFile> {
    find_ini_files(bottle, game_id)
        .iter()
        .filter_map(|p| parse_ini(p).ok())
        .collect()
}

/// Get a specific setting from an INI file.
pub fn get_setting(ini: &IniFile, section: &str, key: &str) -> Option<String> {
    ini.sections.get(section).and_then(|s| s.get(key)).cloned()
}

/// Set a specific value in an INI file on disk.
pub fn set_setting(path: &Path, section: &str, key: &str, value: &str) -> Result<()> {
    if !path.exists() {
        return Err(IniError::NotFound(path.to_string_lossy().to_string()));
    }

    let content = fs::read_to_string(path)?;
    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();

    let mut in_section = false;
    let mut found = false;
    let section_header = format!("[{}]", section);

    for line in &mut lines {
        let trimmed = line.trim();

        if trimmed.eq_ignore_ascii_case(&section_header) {
            in_section = true;
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if in_section && !found {
                // Insert before the next section
                *line = format!("{}={}\n{}", key, value, line);
                found = true;
            }
            in_section = false;
            continue;
        }

        if in_section && !found {
            if let Some(eq_pos) = trimmed.find('=') {
                let k = trimmed[..eq_pos].trim();
                if k.eq_ignore_ascii_case(key) {
                    *line = format!("{}={}", key, value);
                    found = true;
                }
            }
        }
    }

    // If setting wasn't found, append it
    if !found {
        if !in_section {
            // Section doesn't exist, create it
            lines.push(String::new());
            lines.push(section_header);
        }
        lines.push(format!("{}={}", key, value));
    }

    let output = lines.join("\n");
    fs::write(path, output)?;
    Ok(())
}

/// Get built-in presets for a game.
pub fn builtin_presets(game_id: &str) -> Vec<IniPreset> {
    match game_id {
        "skyrimse" | "skyrim" => skyrim_presets(),
        "fallout4" => fallout4_presets(),
        _ => Vec::new(),
    }
}

fn skyrim_presets() -> Vec<IniPreset> {
    vec![
        IniPreset {
            name: "Steam Deck Optimized".to_string(),
            description: "Optimized settings for Steam Deck (720p, medium quality)".to_string(),
            settings: vec![
                IniSetting { file_name: "SkyrimPrefs.ini".into(), section: "Display".into(), key: "iSize W".into(), value: "1280".into() },
                IniSetting { file_name: "SkyrimPrefs.ini".into(), section: "Display".into(), key: "iSize H".into(), value: "800".into() },
                IniSetting { file_name: "SkyrimPrefs.ini".into(), section: "Display".into(), key: "bFull Screen".into(), value: "1".into() },
                IniSetting { file_name: "SkyrimPrefs.ini".into(), section: "Display".into(), key: "bBorderless".into(), value: "1".into() },
                IniSetting { file_name: "SkyrimPrefs.ini".into(), section: "Display".into(), key: "iShadowMapResolution".into(), value: "1024".into() },
                IniSetting { file_name: "Skyrim.ini".into(), section: "General".into(), key: "bAlwaysActive".into(), value: "1".into() },
            ],
        },
        IniPreset {
            name: "High Quality".to_string(),
            description: "Maximum visual quality for powerful systems".to_string(),
            settings: vec![
                IniSetting { file_name: "SkyrimPrefs.ini".into(), section: "Display".into(), key: "iShadowMapResolution".into(), value: "4096".into() },
                IniSetting { file_name: "SkyrimPrefs.ini".into(), section: "Display".into(), key: "fShadowDistance".into(), value: "8000.0000".into() },
                IniSetting { file_name: "SkyrimPrefs.ini".into(), section: "Display".into(), key: "iMaxAnisotropy".into(), value: "16".into() },
                IniSetting { file_name: "SkyrimPrefs.ini".into(), section: "Display".into(), key: "bTreesReceiveShadows".into(), value: "1".into() },
                IniSetting { file_name: "SkyrimPrefs.ini".into(), section: "Display".into(), key: "bDrawLandShadows".into(), value: "1".into() },
            ],
        },
        IniPreset {
            name: "Performance".to_string(),
            description: "Reduced quality for better frame rates".to_string(),
            settings: vec![
                IniSetting { file_name: "SkyrimPrefs.ini".into(), section: "Display".into(), key: "iShadowMapResolution".into(), value: "512".into() },
                IniSetting { file_name: "SkyrimPrefs.ini".into(), section: "Display".into(), key: "fShadowDistance".into(), value: "2000.0000".into() },
                IniSetting { file_name: "SkyrimPrefs.ini".into(), section: "Display".into(), key: "bTreesReceiveShadows".into(), value: "0".into() },
                IniSetting { file_name: "SkyrimPrefs.ini".into(), section: "Display".into(), key: "bDrawLandShadows".into(), value: "0".into() },
                IniSetting { file_name: "SkyrimPrefs.ini".into(), section: "Display".into(), key: "iMaxAnisotropy".into(), value: "4".into() },
                IniSetting { file_name: "Skyrim.ini".into(), section: "General".into(), key: "bAlwaysActive".into(), value: "1".into() },
            ],
        },
    ]
}

fn fallout4_presets() -> Vec<IniPreset> {
    vec![
        IniPreset {
            name: "Steam Deck Optimized".to_string(),
            description: "Optimized settings for Steam Deck (720p, medium quality)".to_string(),
            settings: vec![
                IniSetting { file_name: "Fallout4Prefs.ini".into(), section: "Display".into(), key: "iSize W".into(), value: "1280".into() },
                IniSetting { file_name: "Fallout4Prefs.ini".into(), section: "Display".into(), key: "iSize H".into(), value: "800".into() },
                IniSetting { file_name: "Fallout4Prefs.ini".into(), section: "Display".into(), key: "bFull Screen".into(), value: "1".into() },
                IniSetting { file_name: "Fallout4Prefs.ini".into(), section: "Display".into(), key: "bBorderless".into(), value: "1".into() },
                IniSetting { file_name: "Fallout4Prefs.ini".into(), section: "Display".into(), key: "iShadowMapResolution".into(), value: "1024".into() },
                IniSetting { file_name: "Fallout4.ini".into(), section: "General".into(), key: "bAlwaysActive".into(), value: "1".into() },
            ],
        },
        IniPreset {
            name: "High Quality".to_string(),
            description: "Maximum visual quality for powerful systems".to_string(),
            settings: vec![
                IniSetting { file_name: "Fallout4Prefs.ini".into(), section: "Display".into(), key: "iShadowMapResolution".into(), value: "4096".into() },
                IniSetting { file_name: "Fallout4Prefs.ini".into(), section: "Display".into(), key: "fDirShadowDistance".into(), value: "20000.0000".into() },
                IniSetting { file_name: "Fallout4Prefs.ini".into(), section: "Display".into(), key: "iMaxAnisotropy".into(), value: "16".into() },
                IniSetting { file_name: "Fallout4Prefs.ini".into(), section: "Display".into(), key: "bTreesReceiveShadows".into(), value: "1".into() },
                IniSetting { file_name: "Fallout4Prefs.ini".into(), section: "Display".into(), key: "bDrawLandShadows".into(), value: "1".into() },
            ],
        },
        IniPreset {
            name: "Performance".to_string(),
            description: "Reduced quality for better frame rates".to_string(),
            settings: vec![
                IniSetting { file_name: "Fallout4Prefs.ini".into(), section: "Display".into(), key: "iShadowMapResolution".into(), value: "512".into() },
                IniSetting { file_name: "Fallout4Prefs.ini".into(), section: "Display".into(), key: "fDirShadowDistance".into(), value: "3000.0000".into() },
                IniSetting { file_name: "Fallout4Prefs.ini".into(), section: "Display".into(), key: "bTreesReceiveShadows".into(), value: "0".into() },
                IniSetting { file_name: "Fallout4Prefs.ini".into(), section: "Display".into(), key: "bDrawLandShadows".into(), value: "0".into() },
                IniSetting { file_name: "Fallout4Prefs.ini".into(), section: "Display".into(), key: "iMaxAnisotropy".into(), value: "4".into() },
                IniSetting { file_name: "Fallout4.ini".into(), section: "General".into(), key: "bAlwaysActive".into(), value: "1".into() },
            ],
        },
    ]
}

/// Apply a preset to the INI files in a bottle.
pub fn apply_preset(bottle: &Bottle, game_id: &str, preset: &IniPreset) -> Result<usize> {
    let ini_files = find_ini_files(bottle, game_id);
    let mut applied = 0;

    for setting in &preset.settings {
        // Find the matching INI file
        let target = ini_files.iter().find(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().eq_ignore_ascii_case(&setting.file_name))
                .unwrap_or(false)
        });

        if let Some(path) = target {
            if set_setting(path, &setting.section, &setting.key, &setting.value).is_ok() {
                applied += 1;
            }
        }
    }

    Ok(applied)
}

/// Count total settings across all INI files.
pub fn count_settings(ini_files: &[IniFile]) -> usize {
    ini_files
        .iter()
        .flat_map(|f| f.sections.values())
        .map(|s| s.len())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_test_ini(dir: &Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn parse_ini_basic() {
        let tmp = TempDir::new().unwrap();
        let path = write_test_ini(
            tmp.path(),
            "test.ini",
            "[Display]\niSize W=1920\niSize H=1080\n[General]\nbAlwaysActive=1\n",
        );
        let ini = parse_ini(&path).unwrap();
        assert_eq!(ini.sections.len(), 2);
        assert_eq!(
            ini.sections.get("Display").unwrap().get("iSize W").unwrap(),
            "1920"
        );
    }

    #[test]
    fn parse_ini_comments_and_empty_lines() {
        let tmp = TempDir::new().unwrap();
        let path = write_test_ini(
            tmp.path(),
            "test.ini",
            "; Comment\n\n[Section]\n# Another comment\nkey=value\n",
        );
        let ini = parse_ini(&path).unwrap();
        assert_eq!(ini.sections.len(), 1);
        assert_eq!(
            ini.sections.get("Section").unwrap().get("key").unwrap(),
            "value"
        );
    }

    #[test]
    fn parse_ini_nonexistent_file() {
        let result = parse_ini(Path::new("/nonexistent/file.ini"));
        assert!(result.is_err());
    }

    #[test]
    fn parse_ini_empty_file() {
        let tmp = TempDir::new().unwrap();
        let path = write_test_ini(tmp.path(), "empty.ini", "");
        let ini = parse_ini(&path).unwrap();
        assert!(ini.sections.is_empty());
    }

    #[test]
    fn get_setting_exists() {
        let tmp = TempDir::new().unwrap();
        let path = write_test_ini(tmp.path(), "test.ini", "[Display]\niSize W=1920\n");
        let ini = parse_ini(&path).unwrap();
        assert_eq!(get_setting(&ini, "Display", "iSize W"), Some("1920".into()));
    }

    #[test]
    fn get_setting_missing_section() {
        let tmp = TempDir::new().unwrap();
        let path = write_test_ini(tmp.path(), "test.ini", "[Display]\nkey=val\n");
        let ini = parse_ini(&path).unwrap();
        assert_eq!(get_setting(&ini, "NoSection", "key"), None);
    }

    #[test]
    fn get_setting_missing_key() {
        let tmp = TempDir::new().unwrap();
        let path = write_test_ini(tmp.path(), "test.ini", "[Display]\nkey=val\n");
        let ini = parse_ini(&path).unwrap();
        assert_eq!(get_setting(&ini, "Display", "nokey"), None);
    }

    #[test]
    fn get_setting_empty_sections() {
        let ini = IniFile {
            file_name: "test.ini".into(),
            path: "/test.ini".into(),
            sections: BTreeMap::new(),
        };
        assert_eq!(get_setting(&ini, "Any", "key"), None);
    }

    #[test]
    fn set_setting_existing_key() {
        let tmp = TempDir::new().unwrap();
        let path = write_test_ini(tmp.path(), "test.ini", "[Display]\niSize W=1920\n");
        set_setting(&path, "Display", "iSize W", "2560").unwrap();
        let ini = parse_ini(&path).unwrap();
        assert_eq!(
            ini.sections.get("Display").unwrap().get("iSize W").unwrap(),
            "2560"
        );
    }

    #[test]
    fn set_setting_new_key_existing_section() {
        let tmp = TempDir::new().unwrap();
        let path = write_test_ini(tmp.path(), "test.ini", "[Display]\niSize W=1920\n");
        set_setting(&path, "Display", "newKey", "42").unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("newKey=42"));
    }

    #[test]
    fn set_setting_new_section() {
        let tmp = TempDir::new().unwrap();
        let path = write_test_ini(tmp.path(), "test.ini", "[Display]\niSize W=1920\n");
        set_setting(&path, "NewSection", "key", "value").unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("[NewSection]"));
        assert!(content.contains("key=value"));
    }

    #[test]
    fn set_setting_nonexistent_file() {
        let result = set_setting(Path::new("/nonexistent/file.ini"), "S", "k", "v");
        assert!(result.is_err());
    }

    #[test]
    fn builtin_presets_skyrimse() {
        let presets = builtin_presets("skyrimse");
        assert_eq!(presets.len(), 3);
        assert!(presets.iter().any(|p| p.name == "Steam Deck Optimized"));
        assert!(presets.iter().any(|p| p.name == "High Quality"));
        assert!(presets.iter().any(|p| p.name == "Performance"));
    }

    #[test]
    fn builtin_presets_unknown_game() {
        let presets = builtin_presets("unknowngame");
        assert!(presets.is_empty());
    }

    #[test]
    fn builtin_presets_have_settings() {
        let presets = builtin_presets("skyrimse");
        for preset in &presets {
            assert!(
                !preset.settings.is_empty(),
                "Preset '{}' has no settings",
                preset.name
            );
        }
    }

    #[test]
    fn builtin_presets_skyrim_classic() {
        let presets = builtin_presets("skyrim");
        assert_eq!(presets.len(), 3);
    }

    #[test]
    fn count_settings_empty() {
        let files: Vec<IniFile> = vec![];
        assert_eq!(count_settings(&files), 0);
    }

    #[test]
    fn count_settings_multiple_files() {
        let mut sections1 = BTreeMap::new();
        sections1.insert("A".into(), {
            let mut m = BTreeMap::new();
            m.insert("k1".into(), "v1".into());
            m.insert("k2".into(), "v2".into());
            m
        });
        let mut sections2 = BTreeMap::new();
        sections2.insert("B".into(), {
            let mut m = BTreeMap::new();
            m.insert("k3".into(), "v3".into());
            m
        });
        let files = vec![
            IniFile {
                file_name: "a.ini".into(),
                path: "/a".into(),
                sections: sections1,
            },
            IniFile {
                file_name: "b.ini".into(),
                path: "/b".into(),
                sections: sections2,
            },
        ];
        assert_eq!(count_settings(&files), 3);
    }

    #[test]
    fn count_settings_file_with_empty_sections() {
        let mut sections = BTreeMap::new();
        sections.insert("Empty".into(), BTreeMap::new());
        sections.insert("HasOne".into(), {
            let mut m = BTreeMap::new();
            m.insert("k".into(), "v".into());
            m
        });
        let files = vec![IniFile {
            file_name: "x.ini".into(),
            path: "/x".into(),
            sections,
        }];
        assert_eq!(count_settings(&files), 1);
    }

    #[test]
    fn parse_ini_preserves_values_with_spaces() {
        let tmp = TempDir::new().unwrap();
        let path = write_test_ini(tmp.path(), "test.ini", "[Section]\nfoo = bar baz \n");
        let ini = parse_ini(&path).unwrap();
        assert_eq!(
            ini.sections.get("Section").unwrap().get("foo").unwrap(),
            "bar baz"
        );
    }

    #[test]
    fn parse_ini_handles_equals_in_value() {
        let tmp = TempDir::new().unwrap();
        let path = write_test_ini(tmp.path(), "test.ini", "[Section]\nkey=value=extra\n");
        let ini = parse_ini(&path).unwrap();
        assert_eq!(
            ini.sections.get("Section").unwrap().get("key").unwrap(),
            "value=extra"
        );
    }

    /// Helper: create a fake bottle with the Skyrim SE AppData directory structure
    /// and return (Bottle, path-to-skyrim-ini-dir).
    fn make_test_bottle(tmp: &TempDir) -> (Bottle, PathBuf) {
        let skyrim_dir = tmp
            .path()
            .join("drive_c")
            .join("users")
            .join("testuser")
            .join("AppData")
            .join("Local")
            .join("Skyrim Special Edition");
        fs::create_dir_all(&skyrim_dir).unwrap();
        let bottle = Bottle {
            name: "test".into(),
            path: tmp.path().to_path_buf(),
            source: "Test".into(),
        };
        (bottle, skyrim_dir)
    }

    // ── find_ini_files tests ──────────────────────────────────────────────

    #[test]
    fn find_ini_files_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let (bottle, _skyrim_dir) = make_test_bottle(&tmp);
        // No .ini files written — directory exists but is empty.
        let found = find_ini_files(&bottle, "skyrimse");
        assert!(found.is_empty());
    }

    #[test]
    fn find_ini_files_with_ini() {
        let tmp = TempDir::new().unwrap();
        let (bottle, skyrim_dir) = make_test_bottle(&tmp);
        write_test_ini(&skyrim_dir, "Skyrim.ini", "[General]\n");
        write_test_ini(&skyrim_dir, "SkyrimPrefs.ini", "[Display]\n");
        let found = find_ini_files(&bottle, "skyrimse");
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn find_ini_files_ignores_non_ini() {
        let tmp = TempDir::new().unwrap();
        let (bottle, skyrim_dir) = make_test_bottle(&tmp);
        // Write a non-INI file and one valid INI file.
        write_test_ini(&skyrim_dir, "readme.txt", "hello");
        write_test_ini(&skyrim_dir, "notes.log", "log stuff");
        write_test_ini(&skyrim_dir, "Skyrim.ini", "[General]\n");
        let found = find_ini_files(&bottle, "skyrimse");
        // Only Skyrim.ini should be found; readme.txt and notes.log are ignored.
        assert_eq!(found.len(), 1);
        assert!(found[0]
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("Skyrim.ini"));
    }

    #[test]
    fn find_ini_files_nonexistent_dir() {
        let tmp = TempDir::new().unwrap();
        // Bottle points at a directory that has no drive_c at all.
        let bottle = Bottle {
            name: "ghost".into(),
            path: tmp.path().join("does_not_exist"),
            source: "Test".into(),
        };
        let found = find_ini_files(&bottle, "skyrimse");
        assert!(found.is_empty());
    }

    // ── read_all_ini tests ────────────────────────────────────────────────

    #[test]
    fn read_all_ini_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let (bottle, _skyrim_dir) = make_test_bottle(&tmp);
        let ini_files = read_all_ini(&bottle, "skyrimse");
        assert!(ini_files.is_empty());
    }

    #[test]
    fn read_all_ini_reads_files() {
        let tmp = TempDir::new().unwrap();
        let (bottle, skyrim_dir) = make_test_bottle(&tmp);
        write_test_ini(&skyrim_dir, "Skyrim.ini", "[General]\nbAlwaysActive=1\n");
        let ini_files = read_all_ini(&bottle, "skyrimse");
        assert_eq!(ini_files.len(), 1);
        assert!(ini_files[0].sections.contains_key("General"));
    }

    #[test]
    fn read_all_ini_multiple_files() {
        let tmp = TempDir::new().unwrap();
        let (bottle, skyrim_dir) = make_test_bottle(&tmp);
        write_test_ini(&skyrim_dir, "Skyrim.ini", "[General]\nkey=val\n");
        write_test_ini(&skyrim_dir, "SkyrimPrefs.ini", "[Display]\niSize W=1920\n");
        write_test_ini(&skyrim_dir, "SkyrimCustom.ini", "[Custom]\nfoo=bar\n");
        let ini_files = read_all_ini(&bottle, "skyrimse");
        assert_eq!(ini_files.len(), 3);
    }

    #[test]
    fn read_all_ini_file_has_correct_name() {
        let tmp = TempDir::new().unwrap();
        let (bottle, skyrim_dir) = make_test_bottle(&tmp);
        write_test_ini(&skyrim_dir, "SkyrimPrefs.ini", "[Display]\niSize W=1920\n");
        let ini_files = read_all_ini(&bottle, "skyrimse");
        assert_eq!(ini_files.len(), 1);
        assert_eq!(ini_files[0].file_name, "SkyrimPrefs.ini");
    }

    // ── apply_preset tests ────────────────────────────────────────────────

    #[test]
    fn apply_preset_creates_file() {
        let tmp = TempDir::new().unwrap();
        let (bottle, skyrim_dir) = make_test_bottle(&tmp);
        // Create the INI files that the preset targets (apply_preset needs
        // them to already exist because find_ini_files looks for them).
        write_test_ini(&skyrim_dir, "SkyrimPrefs.ini", "[Display]\n");
        write_test_ini(&skyrim_dir, "Skyrim.ini", "[General]\n");

        let presets = builtin_presets("skyrimse");
        let preset = &presets[0]; // "Steam Deck Optimized"
        let result = apply_preset(&bottle, "skyrimse", preset);
        assert!(result.is_ok());

        // Verify the INI was written to disk with new settings.
        let content = fs::read_to_string(skyrim_dir.join("SkyrimPrefs.ini")).unwrap();
        assert!(content.contains("iSize W=1280"));
    }

    #[test]
    fn apply_preset_updates_existing() {
        let tmp = TempDir::new().unwrap();
        let (bottle, skyrim_dir) = make_test_bottle(&tmp);
        write_test_ini(
            &skyrim_dir,
            "SkyrimPrefs.ini",
            "[Display]\niSize W=1920\niSize H=1080\n",
        );
        write_test_ini(&skyrim_dir, "Skyrim.ini", "[General]\nbAlwaysActive=0\n");

        let presets = builtin_presets("skyrimse");
        let preset = &presets[0]; // "Steam Deck Optimized"
        apply_preset(&bottle, "skyrimse", preset).unwrap();

        let prefs = parse_ini(&skyrim_dir.join("SkyrimPrefs.ini")).unwrap();
        // The Steam Deck preset sets iSize W to 1280 (was 1920).
        assert_eq!(
            prefs
                .sections
                .get("Display")
                .unwrap()
                .get("iSize W")
                .unwrap(),
            "1280"
        );
    }

    #[test]
    fn apply_preset_returns_count() {
        let tmp = TempDir::new().unwrap();
        let (bottle, skyrim_dir) = make_test_bottle(&tmp);
        write_test_ini(&skyrim_dir, "SkyrimPrefs.ini", "[Display]\n");
        write_test_ini(&skyrim_dir, "Skyrim.ini", "[General]\n");

        let presets = builtin_presets("skyrimse");
        let preset = &presets[0]; // "Steam Deck Optimized" — 6 settings
        let count = apply_preset(&bottle, "skyrimse", preset).unwrap();
        assert_eq!(count, preset.settings.len());
    }

    #[test]
    fn apply_preset_handles_multiple_sections() {
        let tmp = TempDir::new().unwrap();
        let (bottle, skyrim_dir) = make_test_bottle(&tmp);
        // The Steam Deck preset touches both SkyrimPrefs.ini [Display] and
        // Skyrim.ini [General], so create both with those sections.
        write_test_ini(&skyrim_dir, "SkyrimPrefs.ini", "[Display]\niSize W=1920\n");
        write_test_ini(&skyrim_dir, "Skyrim.ini", "[General]\nbAlwaysActive=0\n");

        let presets = builtin_presets("skyrimse");
        let preset = &presets[0]; // "Steam Deck Optimized"
        apply_preset(&bottle, "skyrimse", preset).unwrap();

        // Check SkyrimPrefs.ini [Display] section
        let prefs = parse_ini(&skyrim_dir.join("SkyrimPrefs.ini")).unwrap();
        assert_eq!(
            prefs
                .sections
                .get("Display")
                .unwrap()
                .get("iSize W")
                .unwrap(),
            "1280"
        );
        assert_eq!(
            prefs
                .sections
                .get("Display")
                .unwrap()
                .get("iSize H")
                .unwrap(),
            "800"
        );

        // Check Skyrim.ini [General] section
        let general = parse_ini(&skyrim_dir.join("Skyrim.ini")).unwrap();
        assert_eq!(
            general
                .sections
                .get("General")
                .unwrap()
                .get("bAlwaysActive")
                .unwrap(),
            "1"
        );
    }
}
