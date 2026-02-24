//! Skyrim SE display scaling fix for CrossOver/macOS.
//!
//! When running Skyrim SE in CrossOver on macOS, the game often renders
//! zoomed-in or pinned to the top-left corner due to incorrect display
//! settings in SkyrimPrefs.ini. This module detects and fixes those
//! settings by matching the Mac's native display resolution and
//! configuring exclusive fullscreen mode.
//!
//! Exclusive fullscreen creates a macOS Space, allowing the user to
//! 3-finger-swipe between the game and their desktop.
//!
//! The fix applies two changes:
//! 1. **SkyrimPrefs.ini**: Set native resolution + `bFull Screen=1`
//! 2. **Wine registry**: Remove virtual desktop settings that force windowed mode

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use log::warn;
use serde::{Deserialize, Serialize};

use crate::bottles::Bottle;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DisplaySettings {
    pub width: u32,
    pub height: u32,
    pub fullscreen: bool,
    pub borderless: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DisplayFixResult {
    pub fixed: bool,
    pub prefs_path: String,
    pub previous: DisplaySettings,
    pub applied: DisplaySettings,
    pub screen_width: u32,
    pub screen_height: u32,
}

// ---------------------------------------------------------------------------
// macOS screen resolution detection
// ---------------------------------------------------------------------------

/// Detect the main display resolution on macOS using system_profiler.
pub fn detect_screen_resolution() -> Result<(u32, u32), String> {
    let output = Command::new("system_profiler")
        .args(["SPDisplaysDataType", "-json"])
        .output()
        .map_err(|e| format!("Failed to run system_profiler: {}", e))?;

    if !output.status.success() {
        return Err("system_profiler returned non-zero exit code".into());
    }

    let json_str = String::from_utf8_lossy(&output.stdout);
    let data: serde_json::Value =
        serde_json::from_str(&json_str).map_err(|e| format!("Failed to parse JSON: {}", e))?;

    // Navigate: SPDisplaysDataType[*].spdisplays_ndrvs[*]._spdisplays_resolution
    if let Some(displays) = data.get("SPDisplaysDataType").and_then(|v| v.as_array()) {
        for gpu in displays {
            if let Some(screens) = gpu.get("spdisplays_ndrvs").and_then(|v| v.as_array()) {
                for screen in screens {
                    // Check if this is the main display
                    let is_main = screen.get("_spdisplays_displayID").is_some()
                        || screen.get("spdisplays_main").and_then(|v| v.as_str())
                            == Some("spdisplays_yes");

                    // Try _spdisplays_pixels first (e.g. "2560 x 1440")
                    if let Some(pixels) = screen.get("_spdisplays_pixels").and_then(|v| v.as_str())
                    {
                        if let Some((w, h)) = parse_resolution_string(pixels) {
                            return Ok((w, h));
                        }
                    }

                    // Try _spdisplays_resolution (e.g. "2560 x 1440 @ 60.00Hz")
                    if let Some(res) = screen
                        .get("_spdisplays_resolution")
                        .and_then(|v| v.as_str())
                    {
                        if let Some((w, h)) = parse_resolution_string(res) {
                            return Ok((w, h));
                        }
                    }

                    // If this was the main display and we couldn't parse, still continue
                    if is_main {
                        continue;
                    }
                }
            }
        }
    }

    // Fallback: try screenresolution tool
    if let Ok(output) = Command::new("screenresolution").arg("get").output() {
        let text = String::from_utf8_lossy(&output.stdout);
        // Output format: "Display 0: 2560x1440x32@0"
        for line in text.lines() {
            if line.contains("Display 0") || line.contains("Display") {
                if let Some(res) = line.split_whitespace().last() {
                    let parts: Vec<&str> = res.split('x').collect();
                    if parts.len() >= 2 {
                        if let (Ok(w), Ok(h)) = (parts[0].parse(), parts[1].parse()) {
                            return Ok((w, h));
                        }
                    }
                }
            }
        }
    }

    Err("Could not detect screen resolution".into())
}

/// Parse a resolution string like "2560 x 1440" or "2560 x 1440 @ 60Hz".
fn parse_resolution_string(s: &str) -> Option<(u32, u32)> {
    // Remove anything after "@" (refresh rate)
    let res_part = s.split('@').next()?;
    let parts: Vec<&str> = res_part.split('x').collect();
    if parts.len() >= 2 {
        let w = parts[0].trim().parse().ok()?;
        let h = parts[1].trim().parse().ok()?;
        return Some((w, h));
    }
    // Try "x" with spaces: "2560 x 1440"
    let parts: Vec<&str> = res_part.split(" x ").collect();
    if parts.len() >= 2 {
        let w = parts[0].trim().parse().ok()?;
        let h = parts[1].trim().parse().ok()?;
        return Some((w, h));
    }
    None
}

// ---------------------------------------------------------------------------
// SkyrimPrefs.ini location
// ---------------------------------------------------------------------------

/// Find SkyrimPrefs.ini in a Wine bottle for Skyrim SE.
///
/// The file is at: `<bottle>/drive_c/users/<user>/Documents/My Games/Skyrim Special Edition/SkyrimPrefs.ini`
pub fn find_skyrim_prefs(bottle: &Bottle) -> Option<PathBuf> {
    let users_dir = bottle.users_dir();
    if !users_dir.exists() {
        return None;
    }

    if let Ok(entries) = fs::read_dir(&users_dir) {
        for entry in entries.flatten() {
            let user_dir = entry.path();
            if !user_dir.is_dir() {
                continue;
            }

            // Try standard Documents path (case-insensitive)
            let candidates = [
                user_dir
                    .join("Documents")
                    .join("My Games")
                    .join("Skyrim Special Edition")
                    .join("SkyrimPrefs.ini"),
                user_dir
                    .join("My Documents")
                    .join("My Games")
                    .join("Skyrim Special Edition")
                    .join("SkyrimPrefs.ini"),
            ];

            for candidate in &candidates {
                if candidate.exists() {
                    return Some(candidate.clone());
                }
            }

            // Case-insensitive search using bottle.find_path doesn't work here
            // because we need to search under a specific user dir. Do manual case-insensitive.
            if let Some(prefs) = find_prefs_case_insensitive(&user_dir) {
                return Some(prefs);
            }
        }
    }

    None
}

/// Case-insensitive search for SkyrimPrefs.ini under a user directory.
fn find_prefs_case_insensitive(user_dir: &Path) -> Option<PathBuf> {
    let docs =
        find_dir_ci(user_dir, "documents").or_else(|| find_dir_ci(user_dir, "my documents"))?;
    let my_games = find_dir_ci(&docs, "my games")?;
    let skyrim_dir = find_dir_ci(&my_games, "skyrim special edition")?;
    find_file_ci(&skyrim_dir, "skyrimprefs.ini")
}

fn find_dir_ci(parent: &Path, name: &str) -> Option<PathBuf> {
    let name_lower = name.to_lowercase();
    if let Ok(entries) = fs::read_dir(parent) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() && entry.file_name().to_string_lossy().to_lowercase() == name_lower {
                return Some(p);
            }
        }
    }
    None
}

fn find_file_ci(parent: &Path, name: &str) -> Option<PathBuf> {
    let name_lower = name.to_lowercase();
    if let Ok(entries) = fs::read_dir(parent) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() && entry.file_name().to_string_lossy().to_lowercase() == name_lower {
                return Some(p);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// INI reading / writing
// ---------------------------------------------------------------------------

/// Read a display-related value from the [Display] section of SkyrimPrefs.ini.
fn read_ini_display_value(content: &str, key: &str) -> Option<String> {
    let mut in_display = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case("[display]") {
            in_display = true;
            continue;
        }
        if trimmed.starts_with('[') {
            in_display = false;
            continue;
        }
        if in_display {
            if let Some((k, v)) = trimmed.split_once('=') {
                if k.trim().eq_ignore_ascii_case(key) {
                    return Some(v.trim().to_string());
                }
            }
        }
    }
    None
}

/// Set a value in the [Display] section, or create it if missing.
fn set_ini_display_value(content: &str, key: &str, value: &str) -> String {
    let mut result = String::with_capacity(content.len() + 50);
    let mut in_display = false;
    let mut found = false;
    let mut display_section_ended = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.eq_ignore_ascii_case("[display]") {
            in_display = true;
            result.push_str(line);
            result.push('\n');
            continue;
        }

        if in_display && trimmed.starts_with('[') {
            // About to leave [Display] section — insert if not found
            if !found {
                result.push_str(&format!("{}={}\n", key, value));
                found = true;
            }
            in_display = false;
            display_section_ended = true;
        }

        if in_display {
            if let Some((k, _)) = trimmed.split_once('=') {
                if k.trim().eq_ignore_ascii_case(key) {
                    result.push_str(&format!("{}={}\n", k.trim(), value));
                    found = true;
                    continue;
                }
            }
        }

        result.push_str(line);
        result.push('\n');
    }

    // If still in [Display] at EOF and key not found, append
    if !found && in_display {
        result.push_str(&format!("{}={}\n", key, value));
    }

    // If [Display] section doesn't exist at all, create it
    if !found && !in_display && !display_section_ended {
        if !result.ends_with('\n') {
            result.push('\n');
        }
        result.push_str("[Display]\n");
        result.push_str(&format!("{}={}\n", key, value));
    }

    // Remove trailing double newline
    while result.ends_with("\n\n") {
        result.pop();
    }
    if !result.ends_with('\n') {
        result.push('\n');
    }

    result
}

/// Read current display settings from SkyrimPrefs.ini.
pub fn read_display_settings(prefs_path: &Path) -> Result<DisplaySettings, String> {
    let content = fs::read_to_string(prefs_path)
        .map_err(|e| format!("Failed to read {}: {}", prefs_path.display(), e))?;

    let width = read_ini_display_value(&content, "iSize W")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let height = read_ini_display_value(&content, "iSize H")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let fullscreen = read_ini_display_value(&content, "bFull Screen")
        .map(|v| v == "1")
        .unwrap_or(false);
    let borderless = read_ini_display_value(&content, "bBorderless")
        .map(|v| v == "1")
        .unwrap_or(false);

    Ok(DisplaySettings {
        width,
        height,
        fullscreen,
        borderless,
    })
}

/// Apply display fix: set resolution to Mac's native resolution in exclusive
/// fullscreen mode. This maps to a macOS native fullscreen Space that the user
/// can 3-finger-swipe away from. Borderless windowed stays on the current
/// desktop and doesn't get its own Space.
pub fn fix_display_settings(
    prefs_path: &Path,
    width: u32,
    height: u32,
) -> Result<DisplaySettings, String> {
    let content = fs::read_to_string(prefs_path)
        .map_err(|e| format!("Failed to read {}: {}", prefs_path.display(), e))?;

    let mut updated = content.clone();
    updated = set_ini_display_value(&updated, "iSize W", &width.to_string());
    updated = set_ini_display_value(&updated, "iSize H", &height.to_string());
    updated = set_ini_display_value(&updated, "bFull Screen", "1");
    updated = set_ini_display_value(&updated, "bBorderless", "0");

    // Write via temp file + rename for atomicity
    let temp_path = prefs_path.with_extension("ini.tmp");
    fs::write(&temp_path, &updated).map_err(|e| format!("Failed to write temp file: {}", e))?;
    fs::rename(&temp_path, prefs_path).map_err(|e| format!("Failed to rename temp file: {}", e))?;

    Ok(DisplaySettings {
        width,
        height,
        fullscreen: true,
        borderless: false,
    })
}

// ---------------------------------------------------------------------------
// Wine registry — virtual desktop removal
// ---------------------------------------------------------------------------

/// Disable Wine's virtual desktop mode by removing the relevant registry
/// keys from `user.reg`. When virtual desktop is enabled, Wine forces a
/// windowed display regardless of the game's own fullscreen settings.
///
/// Removing these keys allows the game to use true exclusive fullscreen,
/// which on macOS creates a native Space the user can 3-finger-swipe away from.
pub fn disable_wine_virtual_desktop(bottle: &Bottle) -> Result<(), String> {
    let user_reg = bottle.path.join("user.reg");
    if !user_reg.exists() {
        return Ok(()); // No registry file — nothing to fix
    }

    let content = fs::read_to_string(&user_reg)
        .map_err(|e| format!("Failed to read user.reg: {}", e))?;

    let mut updated = content.clone();

    // Remove the virtual desktop definitions section entirely
    updated = remove_registry_section(&updated, r#"[Software\\Wine\\Explorer\\Desktops]"#);

    // Remove any sub-sections like [Software\\Wine\\Explorer\\Desktops\Default]
    updated = remove_registry_sections_matching(&updated, r#"[Software\\Wine\\Explorer\\Desktops\"#);

    // Remove the "Desktop" key from [Software\\Wine\\Explorer] which activates
    // the virtual desktop
    updated = remove_registry_key(&updated, r#"[Software\\Wine\\Explorer]"#, "Desktop");

    if updated == content {
        return Ok(()); // No changes needed
    }

    let temp_path = user_reg.with_extension("reg.tmp");
    fs::write(&temp_path, &updated)
        .map_err(|e| format!("Failed to write temp registry file: {}", e))?;
    fs::rename(&temp_path, &user_reg)
        .map_err(|e| format!("Failed to rename temp registry file: {}", e))?;

    Ok(())
}

/// Remove an entire registry section (header + all keys until next section).
fn remove_registry_section(content: &str, section_header: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut skip = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed == section_header {
            skip = true;
            continue;
        }

        if skip && trimmed.starts_with('[') {
            skip = false;
        }

        if !skip {
            result.push_str(line);
            result.push('\n');
        }
    }

    // Preserve original trailing newline behavior
    while result.ends_with("\n\n\n") {
        result.pop();
    }

    result
}

/// Remove all registry sections whose header starts with the given prefix.
/// Used to remove sub-keys like `[Software\\Wine\\Explorer\\Desktops\Default]`.
fn remove_registry_sections_matching(content: &str, prefix: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut skip = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with(prefix) && trimmed.ends_with(']') {
            skip = true;
            continue;
        }

        if skip && trimmed.starts_with('[') {
            skip = false;
        }

        if !skip {
            result.push_str(line);
            result.push('\n');
        }
    }

    while result.ends_with("\n\n\n") {
        result.pop();
    }

    result
}

/// Remove a specific key from a registry section.
fn remove_registry_key(content: &str, section_header: &str, key_name: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut in_section = false;
    let key_pattern = format!("\"{}\"", key_name);

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed == section_header {
            in_section = true;
            result.push_str(line);
            result.push('\n');
            continue;
        }

        if in_section && trimmed.starts_with('[') {
            in_section = false;
        }

        // Skip lines matching "KeyName"=... in the target section
        if in_section && trimmed.starts_with(&key_pattern) && trimmed.contains('=') {
            continue;
        }

        result.push_str(line);
        result.push('\n');
    }

    result
}

// ---------------------------------------------------------------------------
// Full pipeline
// ---------------------------------------------------------------------------

/// Full pipeline: detect resolution, find prefs, fix INI + Wine registry.
///
/// 1. Sets SkyrimPrefs.ini to native resolution in exclusive fullscreen
/// 2. Removes Wine virtual desktop to allow true fullscreen (macOS Space)
pub fn auto_fix_display(bottle: &Bottle) -> Result<DisplayFixResult, String> {
    let (screen_w, screen_h) = detect_screen_resolution()?;

    let prefs_path = find_skyrim_prefs(bottle)
        .ok_or("Could not find SkyrimPrefs.ini in this bottle. Launch Skyrim once first to create the settings file.")?;

    let previous = read_display_settings(&prefs_path)?;

    // Always attempt Wine registry fix (virtual desktop may be the only issue)
    if let Err(e) = disable_wine_virtual_desktop(bottle) {
        warn!("Could not disable Wine virtual desktop: {}", e);
    }

    // Check if INI fix is needed
    let ini_already_correct = previous.width == screen_w
        && previous.height == screen_h
        && previous.fullscreen
        && !previous.borderless;

    let applied = if ini_already_correct {
        previous.clone()
    } else {
        fix_display_settings(&prefs_path, screen_w, screen_h)?
    };

    Ok(DisplayFixResult {
        fixed: true, // Always report fixed since we also fix Wine registry
        prefs_path: prefs_path.to_string_lossy().into_owned(),
        previous,
        applied,
        screen_width: screen_w,
        screen_height: screen_h,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_INI: &str = r#"[General]
sLanguage=ENGLISH

[Display]
iSize H=720
iSize W=1280
bFull Screen=0
bBorderless=0
fDefaultFOV=90

[Audio]
fMusicVolume=0.5
"#;

    #[test]
    fn read_ini_values() {
        assert_eq!(
            read_ini_display_value(SAMPLE_INI, "iSize H"),
            Some("720".to_string())
        );
        assert_eq!(
            read_ini_display_value(SAMPLE_INI, "iSize W"),
            Some("1280".to_string())
        );
        assert_eq!(
            read_ini_display_value(SAMPLE_INI, "bFull Screen"),
            Some("0".to_string())
        );
        assert_eq!(
            read_ini_display_value(SAMPLE_INI, "bBorderless"),
            Some("0".to_string())
        );
    }

    #[test]
    fn read_ini_missing_key() {
        assert_eq!(read_ini_display_value(SAMPLE_INI, "iSomething"), None);
    }

    #[test]
    fn set_ini_updates_existing() {
        let result = set_ini_display_value(SAMPLE_INI, "iSize H", "1440");
        assert!(result.contains("iSize H=1440"));
        assert!(!result.contains("iSize H=720"));
    }

    #[test]
    fn set_ini_adds_missing_key() {
        let result = set_ini_display_value(SAMPLE_INI, "iNewSetting", "42");
        assert!(result.contains("iNewSetting=42"));
        // Should be added inside [Display] section
        let display_start = result.find("[Display]").unwrap();
        let new_setting = result.find("iNewSetting=42").unwrap();
        let audio_start = result.find("[Audio]").unwrap();
        assert!(new_setting > display_start && new_setting < audio_start);
    }

    #[test]
    fn set_ini_creates_display_section() {
        let ini = "[General]\nsLanguage=ENGLISH\n";
        let result = set_ini_display_value(ini, "iSize W", "2560");
        assert!(result.contains("[Display]"));
        assert!(result.contains("iSize W=2560"));
    }

    #[test]
    fn full_display_fix_pipeline() {
        let mut content = SAMPLE_INI.to_string();
        content = set_ini_display_value(&content, "iSize W", "2560");
        content = set_ini_display_value(&content, "iSize H", "1440");
        content = set_ini_display_value(&content, "bFull Screen", "1");
        content = set_ini_display_value(&content, "bBorderless", "0");

        assert!(content.contains("iSize W=2560"));
        assert!(content.contains("iSize H=1440"));
        assert!(content.contains("bFull Screen=1"));
        assert!(content.contains("bBorderless=0"));
        // Original values should be gone
        assert!(!content.contains("iSize W=1280"));
        assert!(!content.contains("iSize H=720"));
        assert!(!content.contains("bFull Screen=0"));
    }

    #[test]
    fn parse_resolution_variants() {
        assert_eq!(parse_resolution_string("2560 x 1440"), Some((2560, 1440)));
        assert_eq!(
            parse_resolution_string("2560 x 1440 @ 60.00Hz"),
            Some((2560, 1440))
        );
        assert_eq!(parse_resolution_string("1920x1080"), Some((1920, 1080)));
    }

    #[test]
    fn case_insensitive_key_read() {
        let ini = "[display]\nisize h=900\nisize w=1600\nbfull screen=1\n";
        assert_eq!(
            read_ini_display_value(ini, "iSize H"),
            Some("900".to_string())
        );
        assert_eq!(
            read_ini_display_value(ini, "iSize W"),
            Some("1600".to_string())
        );
    }

    // --- Wine registry tests ---

    const SAMPLE_REGISTRY: &str = r#"WINE REGISTRY Version 2
;; All keys relative to \\User\\S-1-5-21

[Software\\Wine\\DllOverrides]
"dxgi"="native"

[Software\\Wine\\Explorer]
"Desktop"="Default"

[Software\\Wine\\Explorer\\Desktops]
"Default"="1920x1080"

[Software\\Wine\\Mac Driver]
"RetinaMode"="Y"
"#;

    #[test]
    fn remove_registry_section_removes_entire_section() {
        let result = remove_registry_section(
            SAMPLE_REGISTRY,
            r#"[Software\\Wine\\Explorer\\Desktops]"#,
        );
        assert!(!result.contains("Desktops"));
        assert!(!result.contains("1920x1080"));
        // Other sections remain
        assert!(result.contains("[Software\\\\Wine\\\\DllOverrides]"));
        assert!(result.contains("[Software\\\\Wine\\\\Mac Driver]"));
    }

    #[test]
    fn remove_registry_key_removes_single_key() {
        let result = remove_registry_key(
            SAMPLE_REGISTRY,
            r#"[Software\\Wine\\Explorer]"#,
            "Desktop",
        );
        assert!(!result.contains("\"Desktop\"=\"Default\""));
        // The section header remains
        assert!(result.contains("[Software\\\\Wine\\\\Explorer]"));
        // Other keys in other sections remain
        assert!(result.contains("\"dxgi\"=\"native\""));
    }

    #[test]
    fn remove_registry_section_noop_when_missing() {
        let input = "[Software\\\\Wine\\\\Mac Driver]\n\"RetinaMode\"=\"Y\"\n";
        let result = remove_registry_section(input, r#"[Software\\Wine\\Explorer\\Desktops]"#);
        assert_eq!(result, input);
    }

    #[test]
    fn remove_registry_sections_matching_removes_subsections() {
        // remove_registry_sections_matching handles sub-sections (prefix ending with \).
        // The main section is removed separately by remove_registry_section.
        let input = concat!(
            "[Software\\\\Wine\\\\Explorer\\\\Desktops]\n",
            "\"Default\"=\"1024x768\"\n",
            "\n",
            "[Software\\\\Wine\\\\Explorer\\\\Desktops\\\\Default]\n",
            "\"Width\"=\"1024\"\n",
            "\"Height\"=\"768\"\n",
            "\n",
            "[Software\\\\Wine\\\\Mac Driver]\n",
            "\"RetinaMode\"=\"Y\"\n",
        );
        // First remove main section, then sub-sections (mirrors disable_wine_virtual_desktop)
        let result = remove_registry_section(
            input,
            r#"[Software\\Wine\\Explorer\\Desktops]"#,
        );
        let result = remove_registry_sections_matching(
            &result,
            r#"[Software\\Wine\\Explorer\\Desktops\"#,
        );
        assert!(!result.contains("Desktops"));
        assert!(!result.contains("1024x768"));
        assert!(result.contains("Mac Driver"));
        assert!(result.contains("RetinaMode"));
    }
}
