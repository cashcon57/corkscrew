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

// ---------------------------------------------------------------------------
// Encoding detection (BOM-aware)
// ---------------------------------------------------------------------------

/// Encoding of an INI file, detected via byte-order-mark.
///
/// Bethesda's launcher occasionally writes Skyrim INI files as UTF-16 LE
/// with a BOM. Plain UTF-8 (with or without BOM) is the common case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IniEncoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
}

/// Line-ending style detected in the source file. Preserved on write so
/// third-party tools that hash the INI don't see spurious modifications.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineEnding {
    Lf,
    Crlf,
}

impl LineEnding {
    fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::Crlf => "\r\n",
        }
    }
}

/// Read an INI file, detecting any BOM and decoding to a UTF-8 String.
/// Returns the decoded text, the source encoding, and the dominant line
/// ending so they can be preserved on round-trip writes.
fn read_ini_decoded(path: &Path) -> Result<(String, IniEncoding, LineEnding)> {
    let bytes = fs::read(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            IniError::NotFound(path.to_string_lossy().to_string())
        } else {
            IniError::Io(e)
        }
    })?;

    let (text, encoding) = decode_ini_bytes(&bytes).map_err(IniError::Other)?;
    let line_ending = detect_line_ending(&text);
    Ok((text, encoding, line_ending))
}

/// Decode INI bytes based on a leading BOM (if any). Falls back to UTF-8
/// (lossy) so even a malformed file still parses to something.
fn decode_ini_bytes(bytes: &[u8]) -> std::result::Result<(String, IniEncoding), String> {
    // UTF-8 BOM: EF BB BF
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        let s = String::from_utf8_lossy(&bytes[3..]).into_owned();
        return Ok((s, IniEncoding::Utf8Bom));
    }
    // UTF-16 LE BOM: FF FE
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let payload = &bytes[2..];
        let mut units: Vec<u16> = Vec::with_capacity(payload.len() / 2);
        for chunk in payload.chunks_exact(2) {
            units.push(u16::from_le_bytes([chunk[0], chunk[1]]));
        }
        let s =
            String::from_utf16(&units).map_err(|e| format!("UTF-16 LE decode failed: {}", e))?;
        return Ok((s, IniEncoding::Utf16Le));
    }
    // UTF-16 BE BOM: FE FF
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let payload = &bytes[2..];
        let mut units: Vec<u16> = Vec::with_capacity(payload.len() / 2);
        for chunk in payload.chunks_exact(2) {
            units.push(u16::from_be_bytes([chunk[0], chunk[1]]));
        }
        let s =
            String::from_utf16(&units).map_err(|e| format!("UTF-16 BE decode failed: {}", e))?;
        return Ok((s, IniEncoding::Utf16Be));
    }

    // No BOM — assume UTF-8 (lossy for safety; Bethesda INIs are ASCII in
    // practice and we don't want a stray invalid byte to fail the whole read).
    let s = String::from_utf8_lossy(bytes).into_owned();
    Ok((s, IniEncoding::Utf8))
}

/// Decide whether the source uses CRLF or bare LF. Defaults to LF when
/// neither is present.
fn detect_line_ending(text: &str) -> LineEnding {
    if text.contains("\r\n") {
        LineEnding::Crlf
    } else {
        LineEnding::Lf
    }
}

/// Encode a UTF-8 string back to the original byte representation,
/// re-prepending any BOM that was stripped on read.
fn encode_ini_bytes(text: &str, encoding: IniEncoding) -> Vec<u8> {
    match encoding {
        IniEncoding::Utf8 => text.as_bytes().to_vec(),
        IniEncoding::Utf8Bom => {
            let mut out = Vec::with_capacity(text.len() + 3);
            out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
            out.extend_from_slice(text.as_bytes());
            out
        }
        IniEncoding::Utf16Le => {
            let mut out = Vec::with_capacity(2 + text.len() * 2);
            out.extend_from_slice(&[0xFF, 0xFE]);
            for unit in text.encode_utf16() {
                out.extend_from_slice(&unit.to_le_bytes());
            }
            out
        }
        IniEncoding::Utf16Be => {
            let mut out = Vec::with_capacity(2 + text.len() * 2);
            out.extend_from_slice(&[0xFE, 0xFF]);
            for unit in text.encode_utf16() {
                out.extend_from_slice(&unit.to_be_bytes());
            }
            out
        }
    }
}

/// Atomic write: write to `<path>.ini.tmp`, then rename onto the target.
/// Wine bottles share INIs with Steam Proton and INI corruption from a
/// half-written file ruins the user's settings.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("ini.tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

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
        "skyrim" => (
            "Skyrim",
            &["Skyrim.ini", "SkyrimPrefs.ini", "SkyrimCustom.ini"],
        ),
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
///
/// Detects UTF-8 / UTF-8 BOM / UTF-16 LE / UTF-16 BE encodings via leading
/// BOM. `fs::read_to_string` would either fail or return mojibake on the
/// UTF-16 INI files Skyrim's launcher occasionally produces.
pub fn parse_ini(path: &Path) -> Result<IniFile> {
    if !path.exists() {
        return Err(IniError::NotFound(path.to_string_lossy().to_string()));
    }

    let (content, _enc, _le) = read_ini_decoded(path)?;
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
///
/// Preserves the original encoding (UTF-8 / UTF-8 BOM / UTF-16 LE/BE) and
/// line-ending style (LF vs CRLF) so third-party tools comparing hashes
/// don't flag the file as gratuitously modified. Writes are atomic via
/// `<path>.ini.tmp` + rename; an interrupted call cannot leave the user's
/// shared Wine/Proton INI truncated.
pub fn set_setting(path: &Path, section: &str, key: &str, value: &str) -> Result<()> {
    let (content, encoding, line_ending) = read_ini_decoded(path)?;

    // Note whether the source ends with a trailing newline so we can preserve
    // it. `str::lines` strips the final newline, so we'd otherwise lose it.
    let ends_with_newline = content.ends_with('\n');

    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();

    let mut in_section = false;
    let mut found = false;
    let section_header = format!("[{}]", section);
    let le = line_ending.as_str();

    for line in &mut lines {
        let trimmed = line.trim();

        if trimmed.eq_ignore_ascii_case(&section_header) {
            in_section = true;
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if in_section && !found {
                // Insert before the next section, preserving the source's
                // line-ending style.
                *line = format!("{}={}{}{}", key, value, le, line);
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

    let mut output = lines.join(le);
    if ends_with_newline {
        output.push_str(le);
    }

    let bytes = encode_ini_bytes(&output, encoding);
    write_atomic(path, &bytes)?;
    Ok(())
}

/// Parse an INI string into sections and key-value pairs (in-memory, no file).
///
/// Used to parse INI tweak content extracted from collection bundles.
pub fn parse_ini_string(content: &str) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut sections: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut current_section = String::new();

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            current_section = trimmed[1..trimmed.len() - 1].to_string();
            sections.entry(current_section.clone()).or_default();
            continue;
        }

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

    sections
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
                IniSetting {
                    file_name: "SkyrimPrefs.ini".into(),
                    section: "Display".into(),
                    key: "iSize W".into(),
                    value: "1280".into(),
                },
                IniSetting {
                    file_name: "SkyrimPrefs.ini".into(),
                    section: "Display".into(),
                    key: "iSize H".into(),
                    value: "800".into(),
                },
                IniSetting {
                    file_name: "SkyrimPrefs.ini".into(),
                    section: "Display".into(),
                    key: "bFull Screen".into(),
                    value: "1".into(),
                },
                IniSetting {
                    file_name: "SkyrimPrefs.ini".into(),
                    section: "Display".into(),
                    key: "bBorderless".into(),
                    value: "1".into(),
                },
                IniSetting {
                    file_name: "SkyrimPrefs.ini".into(),
                    section: "Display".into(),
                    key: "iShadowMapResolution".into(),
                    value: "1024".into(),
                },
                IniSetting {
                    file_name: "Skyrim.ini".into(),
                    section: "General".into(),
                    key: "bAlwaysActive".into(),
                    value: "1".into(),
                },
            ],
        },
        IniPreset {
            name: "High Quality".to_string(),
            description: "Maximum visual quality for powerful systems".to_string(),
            settings: vec![
                IniSetting {
                    file_name: "SkyrimPrefs.ini".into(),
                    section: "Display".into(),
                    key: "iShadowMapResolution".into(),
                    value: "4096".into(),
                },
                IniSetting {
                    file_name: "SkyrimPrefs.ini".into(),
                    section: "Display".into(),
                    key: "fShadowDistance".into(),
                    value: "8000.0000".into(),
                },
                IniSetting {
                    file_name: "SkyrimPrefs.ini".into(),
                    section: "Display".into(),
                    key: "iMaxAnisotropy".into(),
                    value: "16".into(),
                },
                IniSetting {
                    file_name: "SkyrimPrefs.ini".into(),
                    section: "Display".into(),
                    key: "bTreesReceiveShadows".into(),
                    value: "1".into(),
                },
                IniSetting {
                    file_name: "SkyrimPrefs.ini".into(),
                    section: "Display".into(),
                    key: "bDrawLandShadows".into(),
                    value: "1".into(),
                },
            ],
        },
        IniPreset {
            name: "Performance".to_string(),
            description: "Reduced quality for better frame rates".to_string(),
            settings: vec![
                IniSetting {
                    file_name: "SkyrimPrefs.ini".into(),
                    section: "Display".into(),
                    key: "iShadowMapResolution".into(),
                    value: "512".into(),
                },
                IniSetting {
                    file_name: "SkyrimPrefs.ini".into(),
                    section: "Display".into(),
                    key: "fShadowDistance".into(),
                    value: "2000.0000".into(),
                },
                IniSetting {
                    file_name: "SkyrimPrefs.ini".into(),
                    section: "Display".into(),
                    key: "bTreesReceiveShadows".into(),
                    value: "0".into(),
                },
                IniSetting {
                    file_name: "SkyrimPrefs.ini".into(),
                    section: "Display".into(),
                    key: "bDrawLandShadows".into(),
                    value: "0".into(),
                },
                IniSetting {
                    file_name: "SkyrimPrefs.ini".into(),
                    section: "Display".into(),
                    key: "iMaxAnisotropy".into(),
                    value: "4".into(),
                },
                IniSetting {
                    file_name: "Skyrim.ini".into(),
                    section: "General".into(),
                    key: "bAlwaysActive".into(),
                    value: "1".into(),
                },
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
                IniSetting {
                    file_name: "Fallout4Prefs.ini".into(),
                    section: "Display".into(),
                    key: "iSize W".into(),
                    value: "1280".into(),
                },
                IniSetting {
                    file_name: "Fallout4Prefs.ini".into(),
                    section: "Display".into(),
                    key: "iSize H".into(),
                    value: "800".into(),
                },
                IniSetting {
                    file_name: "Fallout4Prefs.ini".into(),
                    section: "Display".into(),
                    key: "bFull Screen".into(),
                    value: "1".into(),
                },
                IniSetting {
                    file_name: "Fallout4Prefs.ini".into(),
                    section: "Display".into(),
                    key: "bBorderless".into(),
                    value: "1".into(),
                },
                IniSetting {
                    file_name: "Fallout4Prefs.ini".into(),
                    section: "Display".into(),
                    key: "iShadowMapResolution".into(),
                    value: "1024".into(),
                },
                IniSetting {
                    file_name: "Fallout4.ini".into(),
                    section: "General".into(),
                    key: "bAlwaysActive".into(),
                    value: "1".into(),
                },
            ],
        },
        IniPreset {
            name: "High Quality".to_string(),
            description: "Maximum visual quality for powerful systems".to_string(),
            settings: vec![
                IniSetting {
                    file_name: "Fallout4Prefs.ini".into(),
                    section: "Display".into(),
                    key: "iShadowMapResolution".into(),
                    value: "4096".into(),
                },
                IniSetting {
                    file_name: "Fallout4Prefs.ini".into(),
                    section: "Display".into(),
                    key: "fDirShadowDistance".into(),
                    value: "20000.0000".into(),
                },
                IniSetting {
                    file_name: "Fallout4Prefs.ini".into(),
                    section: "Display".into(),
                    key: "iMaxAnisotropy".into(),
                    value: "16".into(),
                },
                IniSetting {
                    file_name: "Fallout4Prefs.ini".into(),
                    section: "Display".into(),
                    key: "bTreesReceiveShadows".into(),
                    value: "1".into(),
                },
                IniSetting {
                    file_name: "Fallout4Prefs.ini".into(),
                    section: "Display".into(),
                    key: "bDrawLandShadows".into(),
                    value: "1".into(),
                },
            ],
        },
        IniPreset {
            name: "Performance".to_string(),
            description: "Reduced quality for better frame rates".to_string(),
            settings: vec![
                IniSetting {
                    file_name: "Fallout4Prefs.ini".into(),
                    section: "Display".into(),
                    key: "iShadowMapResolution".into(),
                    value: "512".into(),
                },
                IniSetting {
                    file_name: "Fallout4Prefs.ini".into(),
                    section: "Display".into(),
                    key: "fDirShadowDistance".into(),
                    value: "3000.0000".into(),
                },
                IniSetting {
                    file_name: "Fallout4Prefs.ini".into(),
                    section: "Display".into(),
                    key: "bTreesReceiveShadows".into(),
                    value: "0".into(),
                },
                IniSetting {
                    file_name: "Fallout4Prefs.ini".into(),
                    section: "Display".into(),
                    key: "bDrawLandShadows".into(),
                    value: "0".into(),
                },
                IniSetting {
                    file_name: "Fallout4Prefs.ini".into(),
                    section: "Display".into(),
                    key: "iMaxAnisotropy".into(),
                    value: "4".into(),
                },
                IniSetting {
                    file_name: "Fallout4.ini".into(),
                    section: "General".into(),
                    key: "bAlwaysActive".into(),
                    value: "1".into(),
                },
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

    // ── parse_ini_string tests ───────────────────────────────────────────

    #[test]
    fn parse_ini_string_basic() {
        let content = "[Display]\niSize W=1920\niSize H=1080\n[General]\nbAlwaysActive=1\n";
        let sections = parse_ini_string(content);
        assert_eq!(sections.len(), 2);
        assert_eq!(
            sections.get("Display").unwrap().get("iSize W").unwrap(),
            "1920"
        );
        assert_eq!(
            sections
                .get("General")
                .unwrap()
                .get("bAlwaysActive")
                .unwrap(),
            "1"
        );
    }

    #[test]
    fn parse_ini_string_empty() {
        let sections = parse_ini_string("");
        assert!(sections.is_empty());
    }

    #[test]
    fn parse_ini_string_comments_only() {
        let content = "; This is a comment\n# Another comment\n";
        let sections = parse_ini_string(content);
        assert!(sections.is_empty());
    }

    #[test]
    fn parse_ini_string_with_spaces() {
        let content = "[Section]\n  key = value with spaces  \n";
        let sections = parse_ini_string(content);
        assert_eq!(
            sections.get("Section").unwrap().get("key").unwrap(),
            "value with spaces"
        );
    }

    #[test]
    fn parse_ini_string_no_section_header() {
        // Keys before any section header should be ignored
        let content = "orphan_key=orphan_value\n[Section]\nkey=value\n";
        let sections = parse_ini_string(content);
        assert_eq!(sections.len(), 1);
        assert!(sections.get("Section").unwrap().contains_key("key"));
    }

    // ── BOM / encoding round-trip tests ──────────────────────────────────

    #[test]
    fn parse_ini_handles_utf8_bom() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("bom.ini");
        // EF BB BF + "[Display]\nkey=val\n"
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"[Display]\nkey=val\n");
        fs::write(&path, &bytes).unwrap();

        let ini = parse_ini(&path).unwrap();
        assert_eq!(
            ini.sections.get("Display").unwrap().get("key").unwrap(),
            "val"
        );
    }

    #[test]
    fn parse_ini_handles_utf16_le_bom() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("u16.ini");
        let text = "[Display]\nkey=val\n";
        let mut bytes = vec![0xFF, 0xFE];
        for u in text.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        fs::write(&path, &bytes).unwrap();

        let ini = parse_ini(&path).unwrap();
        assert_eq!(
            ini.sections.get("Display").unwrap().get("key").unwrap(),
            "val"
        );
    }

    #[test]
    fn parse_ini_handles_utf16_be_bom() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("u16be.ini");
        let text = "[Display]\nkey=val\n";
        let mut bytes = vec![0xFE, 0xFF];
        for u in text.encode_utf16() {
            bytes.extend_from_slice(&u.to_be_bytes());
        }
        fs::write(&path, &bytes).unwrap();

        let ini = parse_ini(&path).unwrap();
        assert_eq!(
            ini.sections.get("Display").unwrap().get("key").unwrap(),
            "val"
        );
    }

    #[test]
    fn set_setting_preserves_utf16_le_encoding() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("u16.ini");
        let text = "[Display]\r\nkey=val\r\n";
        let mut bytes = vec![0xFF, 0xFE];
        for u in text.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        fs::write(&path, &bytes).unwrap();

        set_setting(&path, "Display", "key", "newval").unwrap();

        let after = fs::read(&path).unwrap();
        assert!(
            after.starts_with(&[0xFF, 0xFE]),
            "UTF-16 LE BOM should be preserved on write"
        );
        let parsed = parse_ini(&path).unwrap();
        assert_eq!(
            parsed.sections.get("Display").unwrap().get("key").unwrap(),
            "newval"
        );
    }

    #[test]
    fn set_setting_preserves_crlf_line_endings() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("crlf.ini");
        fs::write(&path, "[Display]\r\nkey=val\r\nother=keep\r\n").unwrap();

        set_setting(&path, "Display", "key", "newval").unwrap();

        let after = fs::read_to_string(&path).unwrap();
        assert!(
            after.contains("\r\n"),
            "CRLF should be preserved (got {:?})",
            after
        );
        assert!(
            !after.contains("\n\n") && !after.replace("\r\n", "").contains('\n'),
            "no bare LFs should be introduced (got {:?})",
            after
        );
        assert!(after.contains("key=newval"));
    }

    #[test]
    fn set_setting_preserves_lf_line_endings() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("lf.ini");
        fs::write(&path, "[Display]\nkey=val\n").unwrap();

        set_setting(&path, "Display", "key", "newval").unwrap();

        let after = fs::read_to_string(&path).unwrap();
        assert!(!after.contains("\r\n"), "no CRLFs should be introduced");
        assert!(after.contains("key=newval"));
    }

    #[test]
    fn set_setting_atomic_no_tmp_left_behind_on_success() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("atomic.ini");
        fs::write(&path, "[A]\nkey=val\n").unwrap();

        set_setting(&path, "A", "key", "v2").unwrap();

        // .ini.tmp sibling should not remain after successful rename.
        let tmp_sibling = path.with_extension("ini.tmp");
        assert!(
            !tmp_sibling.exists(),
            ".ini.tmp should be renamed away on success"
        );
    }

    #[test]
    fn detect_line_ending_lf_vs_crlf() {
        assert_eq!(detect_line_ending("a\nb\n"), LineEnding::Lf);
        assert_eq!(detect_line_ending("a\r\nb\r\n"), LineEnding::Crlf);
        assert_eq!(detect_line_ending(""), LineEnding::Lf);
    }

    #[test]
    fn decode_ini_bytes_no_bom_is_utf8() {
        let (s, enc) = decode_ini_bytes(b"[A]\nk=v\n").unwrap();
        assert_eq!(enc, IniEncoding::Utf8);
        assert_eq!(s, "[A]\nk=v\n");
    }
}
