//! Skyrim SE display fix for Wine/CrossOver/Proton on macOS and Linux.
//!
//! When running Skyrim SE through a compatibility layer, the game often
//! renders windowed or at the wrong resolution due to incorrect display
//! settings in SkyrimPrefs.ini. This module detects the correct screen
//! resolution for the platform and configures exclusive fullscreen mode.
//!
//! Platform support:
//! - **macOS**: Detects Retina vs non-Retina via system_profiler, respects
//!   Wine's RetinaMode setting to choose physical vs logical resolution.
//! - **Linux (X11)**: Uses xrandr to detect primary display resolution.
//! - **Linux (Wayland)**: Uses wlr-randr or xdpyinfo via XWayland.
//! - **SteamOS/Steam Deck**: Detects Gamescope resolution or defaults to 1280x800.
//!
//! The fix applies three changes:
//! 1. **SkyrimPrefs.ini**: Set detected resolution + `bFull Screen=1`
//! 2. **Wine registry**: Remove virtual desktop settings that force windowed mode
//! 3. **Wine registry**: Configure display capture + mouse warping for proper input

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use log::{debug, warn};
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
// Screen resolution detection (cross-platform)
// ---------------------------------------------------------------------------

/// Check whether Wine's Retina/HiDPI mode is enabled in a bottle's registry.
pub fn is_retina_enabled(bottle: &Bottle) -> bool {
    let user_reg = bottle.path.join("user.reg");
    let content = fs::read_to_string(&user_reg).unwrap_or_default();
    content.contains("\"RetinaMode\"=\"Y\"")
}

/// Detect the screen resolution appropriate for a given bottle.
///
/// On macOS Retina displays, the result depends on whether Wine's
/// RetinaMode is enabled:
/// - **Retina ON**: returns physical pixels (e.g., 3456x2234) since Wine
///   sees the full native resolution.
/// - **Retina OFF**: returns logical resolution (e.g., 1728x1117) which
///   is what Wine actually exposes to applications.
///
/// On Linux, returns the current display resolution via xrandr (X11),
/// wlr-randr (Wayland), or Gamescope env vars (SteamOS).
pub fn detect_screen_resolution(bottle: &Bottle) -> Result<(u32, u32), String> {
    #[cfg(target_os = "macos")]
    {
        detect_macos_resolution(bottle)
    }
    #[cfg(target_os = "linux")]
    {
        let _ = bottle; // Bottle not needed for Linux resolution detection
        detect_linux_resolution()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = bottle;
        Err("Screen resolution detection not supported on this platform".into())
    }
}

// ---------------------------------------------------------------------------
// macOS resolution detection
// ---------------------------------------------------------------------------

/// Detect resolution on macOS, accounting for Retina scaling.
#[cfg(target_os = "macos")]
fn detect_macos_resolution(bottle: &Bottle) -> Result<(u32, u32), String> {
    let retina = is_retina_enabled(bottle);
    let (logical, physical) = detect_macos_resolutions()?;

    let (w, h) = if retina {
        debug!(
            "Retina mode enabled — using physical pixels: {}x{}",
            physical.0, physical.1
        );
        physical
    } else {
        debug!(
            "Retina mode disabled — using logical resolution: {}x{}",
            logical.0, logical.1
        );
        logical
    };

    Ok((w, h))
}

/// Query system_profiler for both logical and physical display resolutions.
///
/// Returns `(logical, physical)` where:
/// - `logical` = `_spdisplays_resolution` (what macOS reports to apps, e.g., 1728x1117)
/// - `physical` = `_spdisplays_pixels` (actual hardware pixels, e.g., 3456x2234)
///
/// On non-Retina displays these are the same value.
#[cfg(target_os = "macos")]
fn detect_macos_resolutions() -> Result<((u32, u32), (u32, u32)), String> {
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

    if let Some(displays) = data.get("SPDisplaysDataType").and_then(|v| v.as_array()) {
        for gpu in displays {
            if let Some(screens) = gpu.get("spdisplays_ndrvs").and_then(|v| v.as_array()) {
                for screen in screens {
                    // Parse physical pixels (e.g., "3456 x 2234")
                    let physical = screen
                        .get("_spdisplays_pixels")
                        .and_then(|v| v.as_str())
                        .and_then(parse_resolution_string);

                    // Parse logical resolution (e.g., "1728 x 1117 @ 120.00Hz")
                    let logical = screen
                        .get("_spdisplays_resolution")
                        .and_then(|v| v.as_str())
                        .and_then(parse_resolution_string);

                    match (logical, physical) {
                        (Some(l), Some(p)) => return Ok((l, p)),
                        (Some(l), None) => return Ok((l, l)),
                        (None, Some(p)) => return Ok((p, p)),
                        (None, None) => continue,
                    }
                }
            }
        }
    }

    // Fallback: try screenresolution tool
    if let Ok(output) = Command::new("screenresolution").arg("get").output() {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            if line.contains("Display 0") || line.contains("Display") {
                if let Some(res) = line.split_whitespace().last() {
                    let parts: Vec<&str> = res.split('x').collect();
                    if parts.len() >= 2 {
                        if let (Ok(w), Ok(h)) = (parts[0].parse(), parts[1].parse()) {
                            return Ok(((w, h), (w, h)));
                        }
                    }
                }
            }
        }
    }

    Err("Could not detect screen resolution on macOS".into())
}

// ---------------------------------------------------------------------------
// Linux resolution detection
// ---------------------------------------------------------------------------

/// Detect resolution on Linux, trying SteamOS/Gamescope first, then
/// Wayland (wlr-randr), then X11 (xrandr).
#[cfg(target_os = "linux")]
fn detect_linux_resolution() -> Result<(u32, u32), String> {
    // SteamOS / Steam Deck — check Gamescope env vars first
    if crate::steam_integration::is_steam_deck() {
        if let Ok(res) = detect_gamescope_resolution() {
            return Ok(res);
        }
        // Known Steam Deck native resolution (landscape orientation)
        debug!("Steam Deck detected, using default 1280x800");
        return Ok((1280, 800));
    }

    // Wayland
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        if let Ok(res) = detect_wayland_resolution() {
            return Ok(res);
        }
        // Wayland fallback: try xrandr via XWayland
    }

    // X11 (or XWayland fallback)
    if std::env::var("DISPLAY").is_ok() {
        if let Ok(res) = detect_x11_resolution() {
            return Ok(res);
        }
    }

    Err("Could not detect screen resolution on Linux. Tried wlr-randr, xrandr.".into())
}

/// Detect resolution from Gamescope environment variables.
/// Gamescope sets these when running inside the Steam Deck compositor.
#[cfg(target_os = "linux")]
fn detect_gamescope_resolution() -> Result<(u32, u32), String> {
    // Gamescope exposes resolution via env vars when available
    if let (Ok(w_str), Ok(h_str)) = (
        std::env::var("GAMESCOPE_WIDTH"),
        std::env::var("GAMESCOPE_HEIGHT"),
    ) {
        if let (Ok(w), Ok(h)) = (w_str.parse::<u32>(), h_str.parse::<u32>()) {
            debug!("Gamescope resolution from env: {}x{}", w, h);
            return Ok((w, h));
        }
    }
    Err("Gamescope env vars not set".into())
}

/// Detect primary display resolution via xrandr (X11).
///
/// Parses output like: `DP-1 connected primary 2560x1440+0+0 ...`
#[cfg(target_os = "linux")]
fn detect_x11_resolution() -> Result<(u32, u32), String> {
    let output = Command::new("xrandr")
        .arg("--query")
        .output()
        .map_err(|e| format!("Failed to run xrandr: {}", e))?;

    if !output.status.success() {
        return Err("xrandr returned non-zero exit code".into());
    }

    let text = String::from_utf8_lossy(&output.stdout);

    // First try: look for "connected primary WxH+X+Y"
    for line in text.lines() {
        if line.contains(" connected primary ") {
            if let Some(res) = parse_xrandr_connected_line(line) {
                debug!("xrandr primary display: {}x{}", res.0, res.1);
                return Ok(res);
            }
        }
    }

    // Fallback: first "connected" display with a resolution
    for line in text.lines() {
        if line.contains(" connected ") {
            if let Some(res) = parse_xrandr_connected_line(line) {
                debug!("xrandr first connected display: {}x{}", res.0, res.1);
                return Ok(res);
            }
        }
    }

    Err("Could not parse xrandr output".into())
}

/// Parse a resolution from an xrandr "connected" line.
/// Format: `NAME connected [primary] WIDTHxHEIGHT+X+Y ...`
#[cfg(target_os = "linux")]
fn parse_xrandr_connected_line(line: &str) -> Option<(u32, u32)> {
    for token in line.split_whitespace() {
        // Match "WxH+X+Y" pattern (e.g., "2560x1440+0+0")
        if token.contains('x') && token.contains('+') {
            let res_part = token.split('+').next()?;
            return parse_resolution_string(res_part);
        }
    }
    None
}

/// Detect resolution via wlr-randr (Wayland/wlroots compositors).
///
/// Parses output like:
/// ```text
/// eDP-1 "..." (DP-1)
///   Enabled: yes
///   Modes:
///     2560x1600 px, 60.004005 Hz (preferred, current)
/// ```
#[cfg(target_os = "linux")]
fn detect_wayland_resolution() -> Result<(u32, u32), String> {
    let output = Command::new("wlr-randr")
        .output()
        .map_err(|e| format!("Failed to run wlr-randr: {}", e))?;

    if !output.status.success() {
        return Err("wlr-randr returned non-zero exit code".into());
    }

    let text = String::from_utf8_lossy(&output.stdout);

    // Look for the line with "(current)" which indicates the active mode
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.contains("current") {
            // Format: "2560x1600 px, 60.004005 Hz (preferred, current)"
            if let Some(res_str) = trimmed.split_whitespace().next() {
                if let Some(res) = parse_resolution_string(res_str) {
                    debug!("wlr-randr current mode: {}x{}", res.0, res.1);
                    return Ok(res);
                }
            }
        }
    }

    Err("Could not parse wlr-randr output".into())
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

/// Apply display fix: set resolution to the detected screen resolution in
/// exclusive fullscreen mode (`bFull Screen=1, bBorderless=0`).
///
/// Exclusive fullscreen is required for Wine to properly capture input and
/// hide the macOS cursor. Borderless mode leaves Wine in windowed mode with
/// no input grab and a visible OS cursor.
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
    // Constrain cursor to the game window — prevents the cursor from reaching
    // the very bottom edge of the screen where macOS triggers Dock auto-show,
    // which breaks through Wine's display capture and makes the OS cursor visible.
    updated = set_ini_display_value(&updated, "bConstrainCursor", "1");

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

/// Set a registry key in user.reg, creating the section if needed.
fn set_registry_value(content: &mut String, section: &str, key: &str, value: &str) {
    let entry = format!("\"{}\"=\"{}\"", key, value);
    let key_prefix = format!("\"{}\"", key);

    if content.contains(section) {
        if content.contains(&key_prefix) {
            // Replace existing value
            let mut replaced = false;
            *content = content
                .lines()
                .map(|line| {
                    if !replaced && line.trim().starts_with(&key_prefix) {
                        replaced = true;
                        entry.clone()
                    } else {
                        line.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
        } else {
            // Add after section header
            if let Some(pos) = content.find(section) {
                let after = pos + section.len();
                if let Some(nl) = content[after..].find('\n') {
                    let insert = after + nl + 1;
                    content.insert_str(insert, &format!("{}\n", entry));
                }
            }
        }
    } else {
        content.push_str(&format!("\n{}\n{}\n", section, entry));
    }
}

/// Configure Wine registry keys for proper mouse/input capture in fullscreen.
///
/// Sets these keys in `user.reg`:
/// - `[Software\\Wine\\Mac Driver]`
///   - `CaptureDisplaysForFullscreen=Y` — exclusive display control, hides OS cursor
/// - `[Software\\Wine\\DirectInput]`
///   - `MouseWarpOverride=force` — continuously warps the mouse pointer so game
///     and OS coordinates stay aligned, even across loading screens and menu transitions
pub fn fix_mouse_capture(bottle: &Bottle) -> Result<(), String> {
    let user_reg = bottle.path.join("user.reg");
    let mut content = if user_reg.exists() {
        fs::read_to_string(&user_reg)
            .map_err(|e| format!("Failed to read user.reg: {}", e))?
    } else {
        String::new()
    };

    let mac_section = "[Software\\\\Wine\\\\Mac Driver]";
    let di_section = "[Software\\\\Wine\\\\DirectInput]";

    set_registry_value(&mut content, mac_section, "CaptureDisplaysForFullscreen", "Y");
    set_registry_value(&mut content, di_section, "MouseWarpOverride", "force");

    // Atomic write
    let tmp = user_reg.with_extension("reg.tmp");
    fs::write(&tmp, &content)
        .map_err(|e| format!("Failed to write temp registry file: {}", e))?;
    fs::rename(&tmp, &user_reg)
        .map_err(|e| format!("Failed to rename temp registry file: {}", e))?;

    Ok(())
}

/// Full pipeline: detect resolution, find prefs, fix INI + Wine registry.
///
/// 1. Detects correct resolution for the platform and bottle's Retina setting
/// 2. Sets SkyrimPrefs.ini to detected resolution in exclusive fullscreen
/// 3. Removes Wine virtual desktop to allow true fullscreen
/// 4. Configures mouse capture and display capture for proper input
pub fn auto_fix_display(bottle: &Bottle) -> Result<DisplayFixResult, String> {
    let (screen_w, screen_h) = detect_screen_resolution(bottle)?;

    let prefs_path = find_skyrim_prefs(bottle)
        .ok_or("Could not find SkyrimPrefs.ini in this bottle. Launch Skyrim once first to create the settings file.")?;

    let previous = read_display_settings(&prefs_path)?;

    // Always attempt Wine registry fixes
    if let Err(e) = disable_wine_virtual_desktop(bottle) {
        warn!("Could not disable Wine virtual desktop: {}", e);
    }

    if let Err(e) = fix_mouse_capture(bottle) {
        warn!("Could not configure mouse capture: {}", e);
    }

    // Check if INI fix is needed (target is exclusive fullscreen)
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
    use crate::bottles::Bottle;
    use std::path::PathBuf;

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
        // Formats from xrandr/wlr-randr
        assert_eq!(parse_resolution_string("2560x1600"), Some((2560, 1600)));
        assert_eq!(parse_resolution_string("1280x800"), Some((1280, 800)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_xrandr_connected_line_formats() {
        assert_eq!(
            parse_xrandr_connected_line("DP-1 connected primary 2560x1440+0+0 (normal left inverted right x axis y axis) 597mm x 336mm"),
            Some((2560, 1440))
        );
        assert_eq!(
            parse_xrandr_connected_line("eDP-1 connected 1920x1080+0+0 (normal) 344mm x 194mm"),
            Some((1920, 1080))
        );
        assert_eq!(
            parse_xrandr_connected_line("HDMI-1 connected (normal left inverted right x axis y axis)"),
            None // No resolution shown = display not active
        );
    }

    #[test]
    fn is_retina_enabled_from_registry() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().to_path_buf();
        std::fs::create_dir_all(bottle_path.join("drive_c")).unwrap();

        let bottle = Bottle {
            name: "Test".into(),
            path: bottle_path.clone(),
            source: "Wine".into(),
        };

        // No user.reg = not enabled
        assert!(!is_retina_enabled(&bottle));

        // user.reg without RetinaMode = not enabled
        std::fs::write(
            bottle_path.join("user.reg"),
            "[Software\\\\Wine\\\\Mac Driver]\n\"RetinaMode\"=\"N\"\n",
        )
        .unwrap();
        assert!(!is_retina_enabled(&bottle));

        // user.reg with RetinaMode=Y = enabled
        std::fs::write(
            bottle_path.join("user.reg"),
            "[Software\\\\Wine\\\\Mac Driver]\n\"RetinaMode\"=\"Y\"\n",
        )
        .unwrap();
        assert!(is_retina_enabled(&bottle));
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
