//! Steam integration for Linux/SteamOS.
//!
//! Provides auto-detection of Steam installations, registration of Corkscrew as a
//! non-Steam game (so it appears in Steam library / Game Mode), and .desktop entry
//! creation for standard Linux desktop environments.

use anyhow::{Context, Result};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// Information about a detected Steam installation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SteamInfo {
    /// Root Steam directory (e.g. ~/.steam/steam)
    pub steam_root: PathBuf,
    /// Per-user config directories under userdata/
    pub userdata_dirs: Vec<PathBuf>,
    /// Whether this appears to be a Steam Deck / SteamOS device
    pub is_steam_deck: bool,
}

/// Current status of Steam integration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SteamStatus {
    pub installed: bool,
    pub registered: bool,
    pub is_deck: bool,
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Detect whether the application is running inside a Flatpak sandbox.
pub fn is_flatpak() -> bool {
    std::env::var("FLATPAK_ID").is_ok() || std::path::Path::new("/.flatpak-info").exists()
}

/// Detect a Steam installation on this system.
#[cfg(target_os = "linux")]
pub fn detect_steam_installation() -> Option<SteamInfo> {
    let raw_home = dirs::home_dir()?;
    // Normalize for Fedora Atomic / Bazzite (/var/home -> /home)
    let home = crate::bottles::normalize_container_path(&raw_home);

    // Log container environment once during detection
    if let Some(ref env) = crate::bottles::detect_container_environment() {
        log::info!("Steam detection running in container environment: {:?}", env);
    }

    let mut candidates = vec![
        home.join(".steam/steam"),
        home.join(".local/share/Steam"),
        home.join(".var/app/com.valvesoftware.Steam/.steam/steam"), // Flatpak
    ];
    // Also check /var/home variant if the normalized home differs from the raw home
    if home != raw_home {
        candidates.push(raw_home.join(".steam/steam"));
        candidates.push(raw_home.join(".local/share/Steam"));
        candidates.push(raw_home.join(".var/app/com.valvesoftware.Steam/.steam/steam"));
    }

    for candidate in &candidates {
        if candidate.join("steam.sh").exists() || candidate.join("ubuntu12_32").exists() {
            let userdata = candidate.join("userdata");
            let mut userdata_dirs = Vec::new();

            if userdata.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&userdata) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_dir() && path.join("config").is_dir() {
                            userdata_dirs.push(path);
                        }
                    }
                }
            }

            if is_flatpak() {
                log::warn!(
                    "Running under Flatpak \u{2014} Steam paths may require portal permissions"
                );
            }

            return Some(SteamInfo {
                steam_root: candidate.clone(),
                userdata_dirs,
                is_steam_deck: is_steam_deck(),
            });
        }
    }

    None
}

#[cfg(not(target_os = "linux"))]
pub fn detect_steam_installation() -> Option<SteamInfo> {
    None
}

/// Check if running on Steam Deck / SteamOS.
#[cfg(target_os = "linux")]
pub fn is_steam_deck() -> bool {
    if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
        let lower = content.to_lowercase();
        if lower.contains("steamos") || lower.contains("steamdeck") {
            return true;
        }
    }
    // Fallback: check for Deck-specific hardware identifiers
    if Path::new("/sys/devices/virtual/dmi/id/board_vendor").exists() {
        if let Ok(vendor) = std::fs::read_to_string("/sys/devices/virtual/dmi/id/board_vendor") {
            if vendor.trim().eq_ignore_ascii_case("Valve") {
                return true;
            }
        }
    }
    false
}

#[cfg(not(target_os = "linux"))]
pub fn is_steam_deck() -> bool {
    false
}

/// Return platform warnings relevant to Steam Deck / SteamOS.
///
/// Checks for read-only filesystems and provides guidance on where to store
/// downloads and staging directories.
pub fn steam_deck_warnings() -> Vec<String> {
    let mut warnings = Vec::new();
    if !is_steam_deck() {
        return warnings;
    }
    // Check if home directory is writable
    let home = dirs::home_dir().unwrap_or_default();
    if !home.exists()
        || std::fs::metadata(&home)
            .map(|m| m.permissions().readonly())
            .unwrap_or(true)
    {
        warnings.push(
            "Home directory may be read-only on SteamOS. Check filesystem permissions.".into(),
        );
    }
    // General guidance for Steam Deck users
    warnings.push(
        "Steam Deck detected: ensure download and staging directories are under /home/deck/".into(),
    );
    warnings
}

// ---------------------------------------------------------------------------
// Steam shortcut registration (binary VDF)
// ---------------------------------------------------------------------------

/// Generate a Steam-compatible app ID from the executable path and app name.
/// Steam uses CRC32 of ("exe" + "appname") | 0x80000000 for non-Steam games.
fn generate_app_id(exe: &str, app_name: &str) -> u32 {
    let input = format!("{}{}", exe, app_name);
    let mut hasher = DefaultHasher::new();
    input.hash(&mut hasher);
    let hash = hasher.finish() as u32;
    hash | 0x80000000
}

/// Binary VDF type markers
const VDF_TYPE_SECTION: u8 = 0x00;
const VDF_TYPE_STRING: u8 = 0x01;
const VDF_TYPE_UINT32: u8 = 0x02;
const VDF_TYPE_END: u8 = 0x08;

/// A single non-Steam shortcut entry parsed from shortcuts.vdf.
#[derive(Debug, Clone)]
struct ShortcutEntry {
    /// The index/key of this entry (e.g. "0", "1", ...)
    index: String,
    /// Raw binary data for this entry (everything between section markers)
    fields: Vec<VdfField>,
}

#[derive(Debug, Clone)]
enum VdfField {
    String { key: String, value: String },
    Uint32 { key: String, value: u32 },
    Section { key: String, fields: Vec<VdfField> },
}

/// Read a null-terminated string from a byte slice, returning (string, bytes_consumed).
fn read_cstring(data: &[u8]) -> Option<(String, usize)> {
    let null_pos = data.iter().position(|&b| b == 0)?;
    let s = String::from_utf8_lossy(&data[..null_pos]).into_owned();
    Some((s, null_pos + 1))
}

/// Parse VDF fields from binary data until end marker.
fn parse_vdf_fields(data: &[u8]) -> Option<(Vec<VdfField>, usize)> {
    let mut fields = Vec::new();
    let mut pos = 0;

    loop {
        if pos >= data.len() {
            break;
        }

        let type_byte = data[pos];
        pos += 1;

        if type_byte == VDF_TYPE_END {
            break;
        }

        // Read key name
        let (key, key_len) = read_cstring(&data[pos..])?;
        pos += key_len;

        match type_byte {
            VDF_TYPE_STRING => {
                let (value, val_len) = read_cstring(&data[pos..])?;
                pos += val_len;
                fields.push(VdfField::String { key, value });
            }
            VDF_TYPE_UINT32 => {
                if pos + 4 > data.len() {
                    return None;
                }
                let value =
                    u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
                pos += 4;
                fields.push(VdfField::Uint32 { key, value });
            }
            VDF_TYPE_SECTION => {
                let (sub_fields, sub_len) = parse_vdf_fields(&data[pos..])?;
                pos += sub_len;
                fields.push(VdfField::Section {
                    key,
                    fields: sub_fields,
                });
            }
            _ => {
                // Unknown type — skip
                log::warn!("Unknown VDF type byte: 0x{:02x} at offset", type_byte);
                return None;
            }
        }
    }

    Some((fields, pos))
}

/// Parse shortcuts.vdf into a list of shortcut entries.
fn parse_shortcuts_vdf(data: &[u8]) -> Option<Vec<ShortcutEntry>> {
    if data.is_empty() {
        return Some(Vec::new());
    }

    // File starts with: \x00 "shortcuts" \x00 <entries...> \x08 \x08
    if data[0] != VDF_TYPE_SECTION {
        return None;
    }

    let (root_key, key_len) = read_cstring(&data[1..])?;
    if root_key != "shortcuts" {
        return None;
    }

    let mut pos = 1 + key_len;
    let mut entries = Vec::new();

    loop {
        if pos >= data.len() {
            break;
        }

        let type_byte = data[pos];
        pos += 1;

        if type_byte == VDF_TYPE_END {
            break;
        }

        if type_byte != VDF_TYPE_SECTION {
            break;
        }

        let (index, idx_len) = read_cstring(&data[pos..])?;
        pos += idx_len;

        let (fields, fields_len) = parse_vdf_fields(&data[pos..])?;
        pos += fields_len;

        entries.push(ShortcutEntry { index, fields });
    }

    Some(entries)
}

/// Write a null-terminated string to a buffer.
fn write_cstring(buf: &mut Vec<u8>, s: &str) {
    buf.extend_from_slice(s.as_bytes());
    buf.push(0);
}

/// Serialize VDF fields to binary.
fn write_vdf_fields(buf: &mut Vec<u8>, fields: &[VdfField]) {
    for field in fields {
        match field {
            VdfField::String { key, value } => {
                buf.push(VDF_TYPE_STRING);
                write_cstring(buf, key);
                write_cstring(buf, value);
            }
            VdfField::Uint32 { key, value } => {
                buf.push(VDF_TYPE_UINT32);
                write_cstring(buf, key);
                buf.extend_from_slice(&value.to_le_bytes());
            }
            VdfField::Section { key, fields } => {
                buf.push(VDF_TYPE_SECTION);
                write_cstring(buf, key);
                write_vdf_fields(buf, fields);
                buf.push(VDF_TYPE_END);
            }
        }
    }
}

/// Serialize shortcut entries back to binary VDF format.
fn write_shortcuts_vdf(entries: &[ShortcutEntry]) -> Vec<u8> {
    let mut buf = Vec::new();

    // Root section: \x00 "shortcuts" \x00
    buf.push(VDF_TYPE_SECTION);
    write_cstring(&mut buf, "shortcuts");

    for entry in entries {
        buf.push(VDF_TYPE_SECTION);
        write_cstring(&mut buf, &entry.index);
        write_vdf_fields(&mut buf, &entry.fields);
        buf.push(VDF_TYPE_END);
    }

    buf.push(VDF_TYPE_END); // End shortcuts section
    buf.push(VDF_TYPE_END); // End file (sometimes double-terminated)

    buf
}

/// Build a new ShortcutEntry for Corkscrew.
fn build_corkscrew_entry(index: &str, exe_path: &str, icon_path: &str) -> ShortcutEntry {
    let app_id = generate_app_id(exe_path, "Corkscrew");
    let start_dir = Path::new(exe_path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();

    ShortcutEntry {
        index: index.to_string(),
        fields: vec![
            VdfField::Uint32 {
                key: "appid".to_string(),
                value: app_id,
            },
            VdfField::String {
                key: "AppName".to_string(),
                value: "Corkscrew".to_string(),
            },
            VdfField::String {
                key: "Exe".to_string(),
                value: format!("\"{}\"", exe_path),
            },
            VdfField::String {
                key: "StartDir".to_string(),
                value: format!("\"{}\"", start_dir),
            },
            VdfField::String {
                key: "icon".to_string(),
                value: icon_path.to_string(),
            },
            VdfField::String {
                key: "ShortcutPath".to_string(),
                value: String::new(),
            },
            VdfField::String {
                key: "LaunchOptions".to_string(),
                value: String::new(),
            },
            VdfField::Uint32 {
                key: "IsHidden".to_string(),
                value: 0,
            },
            VdfField::Uint32 {
                key: "AllowDesktopConfig".to_string(),
                value: 1,
            },
            VdfField::Uint32 {
                key: "AllowOverlay".to_string(),
                value: 1,
            },
            VdfField::Uint32 {
                key: "OpenVR".to_string(),
                value: 0,
            },
            VdfField::Uint32 {
                key: "Devkit".to_string(),
                value: 0,
            },
            VdfField::String {
                key: "DevkitGameID".to_string(),
                value: String::new(),
            },
            VdfField::Uint32 {
                key: "DevkitOverrideAppID".to_string(),
                value: 0,
            },
            VdfField::Uint32 {
                key: "LastPlayTime".to_string(),
                value: 0,
            },
            VdfField::String {
                key: "FlatpakAppID".to_string(),
                value: String::new(),
            },
            VdfField::Section {
                key: "tags".to_string(),
                fields: vec![VdfField::String {
                    key: "0".to_string(),
                    value: "Mod Manager".to_string(),
                }],
            },
        ],
    }
}

/// Check if the AppName field of an entry matches "Corkscrew".
fn entry_is_corkscrew(entry: &ShortcutEntry) -> bool {
    entry.fields.iter().any(|f| match f {
        VdfField::String { key, value } => key == "AppName" && value == "Corkscrew",
        _ => false,
    })
}

/// Add Corkscrew to Steam's non-Steam game shortcuts.
pub fn add_to_steam(steam_info: &SteamInfo, exe_path: &str, icon_path: &str) -> Result<()> {
    if steam_info.userdata_dirs.is_empty() {
        anyhow::bail!("No Steam user profiles found");
    }

    for user_dir in &steam_info.userdata_dirs {
        let shortcuts_path = user_dir.join("config").join("shortcuts.vdf");

        // Back up existing file
        if shortcuts_path.exists() {
            let backup_path = shortcuts_path.with_extension("vdf.bak");
            std::fs::copy(&shortcuts_path, &backup_path)
                .context("Failed to back up shortcuts.vdf")?;
        }

        // Parse existing entries
        let mut entries = if shortcuts_path.exists() {
            let data = std::fs::read(&shortcuts_path).context("Failed to read shortcuts.vdf")?;
            parse_shortcuts_vdf(&data).unwrap_or_default()
        } else {
            Vec::new()
        };

        // Check if already registered — update if so
        let existing_idx = entries.iter().position(entry_is_corkscrew);
        if let Some(idx) = existing_idx {
            let index = entries[idx].index.clone();
            entries[idx] = build_corkscrew_entry(&index, exe_path, icon_path);
            log::info!("Updated existing Corkscrew entry in shortcuts.vdf");
        } else {
            let next_index = entries.len().to_string();
            entries.push(build_corkscrew_entry(&next_index, exe_path, icon_path));
            log::info!("Added Corkscrew to shortcuts.vdf as entry {}", next_index);
        }

        // Write back
        let vdf_data = write_shortcuts_vdf(&entries);

        // Ensure parent directory exists
        if let Some(parent) = shortcuts_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(&shortcuts_path, &vdf_data).context("Failed to write shortcuts.vdf")?;
    }

    Ok(())
}

/// Remove Corkscrew from Steam's non-Steam game shortcuts.
pub fn remove_from_steam(steam_info: &SteamInfo) -> Result<()> {
    for user_dir in &steam_info.userdata_dirs {
        let shortcuts_path = user_dir.join("config").join("shortcuts.vdf");

        if !shortcuts_path.exists() {
            continue;
        }

        let data = std::fs::read(&shortcuts_path).context("Failed to read shortcuts.vdf")?;

        let mut entries = parse_shortcuts_vdf(&data).unwrap_or_default();

        let before_len = entries.len();
        entries.retain(|e| !entry_is_corkscrew(e));

        if entries.len() < before_len {
            // Re-index entries
            for (i, entry) in entries.iter_mut().enumerate() {
                entry.index = i.to_string();
            }

            let vdf_data = write_shortcuts_vdf(&entries);
            std::fs::write(&shortcuts_path, &vdf_data).context("Failed to write shortcuts.vdf")?;

            log::info!("Removed Corkscrew from shortcuts.vdf");
        }
    }

    Ok(())
}

/// Check if Corkscrew is registered in any Steam user's shortcuts.
pub fn is_registered_in_steam(steam_info: &SteamInfo) -> bool {
    for user_dir in &steam_info.userdata_dirs {
        let shortcuts_path = user_dir.join("config").join("shortcuts.vdf");

        if !shortcuts_path.exists() {
            continue;
        }

        if let Ok(data) = std::fs::read(&shortcuts_path) {
            if let Some(entries) = parse_shortcuts_vdf(&data) {
                if entries.iter().any(entry_is_corkscrew) {
                    return true;
                }
            }
        }
    }

    false
}

// ---------------------------------------------------------------------------
// Desktop entry
// ---------------------------------------------------------------------------

/// Create a .desktop entry for Corkscrew on Linux.
pub fn create_desktop_entry(exe_path: &str) -> Result<PathBuf> {
    let home = dirs::home_dir().context("Cannot determine home directory")?;

    // Install icon
    let icon_dir = home.join(".local/share/icons/hicolor/128x128/apps");
    std::fs::create_dir_all(&icon_dir)?;
    let icon_dest = icon_dir.join("corkscrew.png");

    // Try to extract icon from the AppImage's embedded resources or use bundled icon
    // For now, look for bundled icon next to the executable
    let exe_dir = Path::new(exe_path).parent().unwrap_or(Path::new("/"));
    let bundled_icon = exe_dir.join("icons/128x128.png");
    if bundled_icon.exists() {
        std::fs::copy(&bundled_icon, &icon_dest)?;
    } else {
        // Try common Tauri icon location
        let alt_icon = exe_dir.join("icons/128x128@2x.png");
        if alt_icon.exists() {
            std::fs::copy(&alt_icon, &icon_dest)?;
        }
    }

    // Write .desktop file
    let desktop_dir = home.join(".local/share/applications");
    std::fs::create_dir_all(&desktop_dir)?;
    let desktop_path = desktop_dir.join("corkscrew.desktop");

    let icon_value = if icon_dest.exists() {
        icon_dest.to_string_lossy().into_owned()
    } else {
        "corkscrew".to_string()
    };

    let content = format!(
        "[Desktop Entry]\n\
         Name=Corkscrew\n\
         Comment=Mod manager for CrossOver/Wine games on macOS and Linux\n\
         Exec={exe_path} %u\n\
         Icon={icon_value}\n\
         Type=Application\n\
         Categories=Game;Utility;\n\
         MimeType=x-scheme-handler/nxm;\n\
         Terminal=false\n\
         StartupWMClass=Corkscrew\n"
    );

    std::fs::write(&desktop_path, content)?;

    // Try to update desktop database (non-fatal if missing)
    let _ = std::process::Command::new("update-desktop-database")
        .arg(desktop_dir.to_string_lossy().as_ref())
        .status();

    // Register as NXM handler
    let _ = std::process::Command::new("xdg-mime")
        .args(["default", "corkscrew.desktop", "x-scheme-handler/nxm"])
        .status();

    log::info!("Created desktop entry at {:?}", desktop_path);

    Ok(desktop_path)
}

/// Get the path to the current executable (resolves AppImage path if applicable).
pub fn get_exe_path() -> Result<String> {
    // If running as an AppImage, APPIMAGE env var has the real path
    if let Ok(appimage) = std::env::var("APPIMAGE") {
        return Ok(appimage);
    }
    // Otherwise use the current executable
    let exe = std::env::current_exe().context("Cannot determine executable path")?;
    Ok(exe.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------------------
// High-level integration
// ---------------------------------------------------------------------------

/// Perform full Steam integration: create desktop entry + add to Steam library.
pub fn setup_steam_integration() -> Result<SteamStatus> {
    let exe_path = get_exe_path()?;
    let is_deck = is_steam_deck();

    // Create desktop entry
    let _ = create_desktop_entry(&exe_path)
        .map_err(|e| log::warn!("Desktop entry creation failed: {}", e));

    // Detect Steam and add shortcut
    let steam_info = detect_steam_installation();
    let mut registered = false;

    if let Some(ref info) = steam_info {
        // Determine icon path
        let home = dirs::home_dir().unwrap_or_default();
        let icon_path = home.join(".local/share/icons/hicolor/128x128/apps/corkscrew.png");
        let icon_str = if icon_path.exists() {
            icon_path.to_string_lossy().into_owned()
        } else {
            String::new()
        };

        match add_to_steam(info, &exe_path, &icon_str) {
            Ok(()) => {
                registered = true;
                log::info!("Successfully registered Corkscrew in Steam");
            }
            Err(e) => log::warn!("Failed to add to Steam: {}", e),
        }
    }

    Ok(SteamStatus {
        installed: steam_info.is_some(),
        registered,
        is_deck,
    })
}

/// Get current Steam integration status without modifying anything.
pub fn get_steam_status() -> SteamStatus {
    let steam_info = detect_steam_installation();
    let registered = steam_info
        .as_ref()
        .map(is_registered_in_steam)
        .unwrap_or(false);

    SteamStatus {
        installed: steam_info.is_some(),
        registered,
        is_deck: is_steam_deck(),
    }
}

// ---------------------------------------------------------------------------
// Steam launch options patching (SKSE / F4SE / script extenders)
// ---------------------------------------------------------------------------

/// Script extender info for a game.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScriptExtenderInfo {
    pub game_id: String,
    pub display_name: String,
    pub exe_name: String,
    pub steam_app_id: u32,
}

/// Known script extenders and their Steam app IDs.
pub fn known_script_extenders() -> Vec<ScriptExtenderInfo> {
    vec![
        ScriptExtenderInfo {
            game_id: "skyrimse".into(),
            display_name: "SKSE64".into(),
            exe_name: "skse64_loader.exe".into(),
            steam_app_id: 489830,
        },
        ScriptExtenderInfo {
            game_id: "fallout4".into(),
            display_name: "F4SE".into(),
            exe_name: "f4se_loader.exe".into(),
            steam_app_id: 377160,
        },
        ScriptExtenderInfo {
            game_id: "oblivion".into(),
            display_name: "OBSE".into(),
            exe_name: "obse_loader.exe".into(),
            steam_app_id: 22330,
        },
    ]
}

/// Status of Steam launch options for a game.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LaunchOptionsStatus {
    pub game_id: String,
    pub steam_app_id: u32,
    pub extender_name: String,
    /// true if launch options are currently set to use the script extender
    pub patched: bool,
    /// The current launch options string (if any)
    pub current_options: Option<String>,
}

/// Parse a text-format VDF file into nested sections.
/// Steam's localconfig.vdf uses indented key-value pairs with quoted strings.
///
/// We use a lightweight approach: read the file line-by-line and track the
/// current path to find/modify `apps/{appid}/LaunchOptions`.
fn parse_text_vdf_launch_options(content: &str, app_id: u32) -> Option<String> {
    let app_id_str = app_id.to_string();
    let mut in_apps = false;
    let mut in_target_app = false;
    let mut depth = 0u32;
    let mut app_depth = 0u32;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed == "{" {
            depth += 1;
            if in_apps && !in_target_app && depth == app_depth + 1 {
                // We just entered the target app's block — but only if
                // the previous line was the app ID key
            }
            continue;
        }
        if trimmed == "}" {
            if in_target_app && depth == app_depth + 1 {
                // Leaving target app block without finding LaunchOptions
                return None;
            }
            depth -= 1;
            if in_apps && depth < app_depth {
                in_apps = false;
            }
            continue;
        }

        // Parse quoted key-value pairs: "key" "value" or just "key"
        let parts: Vec<&str> = trimmed
            .split('"')
            .filter(|s| !s.trim().is_empty())
            .collect();

        if parts.is_empty() {
            continue;
        }

        let key = parts[0];

        // Track section entry
        if key.eq_ignore_ascii_case("apps") || key == "Apps" {
            in_apps = true;
            app_depth = depth;
        } else if in_apps && key == app_id_str && !in_target_app {
            in_target_app = true;
            app_depth = depth;
        } else if in_target_app && key.eq_ignore_ascii_case("LaunchOptions") {
            if parts.len() >= 2 {
                return Some(parts[1].to_string());
            }
            return Some(String::new());
        }
    }

    None
}

/// Set or clear launch options for a Steam game in localconfig.vdf.
fn set_text_vdf_launch_options(
    content: &str,
    app_id: u32,
    new_options: Option<&str>,
) -> Result<String> {
    let app_id_str = app_id.to_string();
    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    let mut in_apps = false;
    let mut in_target_app = false;
    let mut depth = 0u32;
    let mut app_depth = 0u32;
    let mut found_launch_options = false;
    let mut target_app_close_line: Option<usize> = None;
    let mut prev_was_app_id = false;

    for i in 0..lines.len() {
        let trimmed = lines[i].trim().to_string();

        if trimmed == "{" {
            depth += 1;
            if prev_was_app_id {
                in_target_app = true;
            }
            prev_was_app_id = false;
            continue;
        }
        if trimmed == "}" {
            if in_target_app && depth == app_depth + 1 {
                target_app_close_line = Some(i);
                in_target_app = false;
                break;
            }
            if depth > 0 {
                depth -= 1;
            }
            if in_apps && depth < app_depth {
                in_apps = false;
            }
            prev_was_app_id = false;
            continue;
        }

        let parts: Vec<&str> = trimmed
            .split('"')
            .filter(|s| !s.trim().is_empty())
            .collect();

        if parts.is_empty() {
            prev_was_app_id = false;
            continue;
        }

        let key = parts[0];

        if key.eq_ignore_ascii_case("apps") || key == "Apps" {
            in_apps = true;
            app_depth = depth;
            prev_was_app_id = false;
        } else if in_apps && key == app_id_str && !in_target_app {
            app_depth = depth;
            prev_was_app_id = true;
        } else if in_target_app && key.eq_ignore_ascii_case("LaunchOptions") {
            if let Some(ref opts) = new_options {
                // Replace existing line
                let indent = lines[i].len() - lines[i].trim_start().len();
                let indent_str: String = std::iter::repeat(' ').take(indent).collect();
                lines[i] = format!("{}\"LaunchOptions\"\t\t\"{}\"", indent_str, opts);
            } else {
                // Remove the line
                lines.remove(i);
            }
            found_launch_options = true;
            break;
        } else {
            prev_was_app_id = false;
        }
    }

    // If we didn't find LaunchOptions but need to set it, insert before the closing brace
    if !found_launch_options {
        if let Some(ref opts) = new_options {
            if let Some(close_line) = target_app_close_line {
                // Insert before the closing brace of the app section
                let indent = lines[close_line].len() - lines[close_line].trim_start().len();
                let indent_str: String = std::iter::repeat(' ').take(indent + 1).collect();
                lines.insert(
                    close_line,
                    format!("{}\"LaunchOptions\"\t\t\"{}\"", indent_str, opts),
                );
            } else {
                anyhow::bail!(
                    "Could not find app {} section in localconfig.vdf to insert launch options",
                    app_id
                );
            }
        }
        // If new_options is None and we didn't find it, nothing to do — already cleared
    }

    Ok(lines.join("\n"))
}

/// Get launch options status for all known script extenders.
pub fn get_launch_options_status(steam_info: &SteamInfo) -> Vec<LaunchOptionsStatus> {
    let extenders = known_script_extenders();
    let mut results = Vec::new();

    // Read localconfig.vdf from the first available userdata dir
    let config_content = steam_info
        .userdata_dirs
        .iter()
        .find_map(|dir| {
            let path = dir.join("config").join("localconfig.vdf");
            std::fs::read_to_string(&path).ok()
        });

    for ext in &extenders {
        let current = config_content
            .as_ref()
            .and_then(|c| parse_text_vdf_launch_options(c, ext.steam_app_id));

        let patched = current
            .as_ref()
            .map(|opts| opts.contains(&ext.exe_name))
            .unwrap_or(false);

        results.push(LaunchOptionsStatus {
            game_id: ext.game_id.clone(),
            steam_app_id: ext.steam_app_id,
            extender_name: ext.display_name.clone(),
            patched,
            current_options: current,
        });
    }

    results
}

/// Patch Steam launch options so the game launches through the script extender.
///
/// For Proton/Wine on Linux, this uses the standard `%command%` substitution:
/// e.g., `skse64_loader.exe %command%` makes Steam launch SKSE instead of the
/// default game executable.
pub fn patch_launch_options(
    steam_info: &SteamInfo,
    game_id: &str,
) -> Result<()> {
    let extender = known_script_extenders()
        .into_iter()
        .find(|e| e.game_id == game_id)
        .ok_or_else(|| anyhow::anyhow!("No known script extender for game '{}'", game_id))?;

    for user_dir in &steam_info.userdata_dirs {
        let config_path = user_dir.join("config").join("localconfig.vdf");
        if !config_path.exists() {
            continue;
        }

        // Backup
        let backup = config_path.with_extension("vdf.bak");
        std::fs::copy(&config_path, &backup)
            .context("Failed to back up localconfig.vdf")?;

        let content = std::fs::read_to_string(&config_path)
            .context("Failed to read localconfig.vdf")?;

        // Set launch options to use the script extender
        let launch_opt = format!("{} %command%", extender.exe_name);
        let updated = set_text_vdf_launch_options(&content, extender.steam_app_id, Some(&launch_opt))?;

        std::fs::write(&config_path, &updated)
            .context("Failed to write localconfig.vdf")?;

        log::info!(
            "Patched Steam launch options for {} (app {}): {}",
            extender.display_name,
            extender.steam_app_id,
            launch_opt
        );
    }

    Ok(())
}

/// Remove script extender from Steam launch options (restore to default).
pub fn unpatch_launch_options(
    steam_info: &SteamInfo,
    game_id: &str,
) -> Result<()> {
    let extender = known_script_extenders()
        .into_iter()
        .find(|e| e.game_id == game_id)
        .ok_or_else(|| anyhow::anyhow!("No known script extender for game '{}'", game_id))?;

    for user_dir in &steam_info.userdata_dirs {
        let config_path = user_dir.join("config").join("localconfig.vdf");
        if !config_path.exists() {
            continue;
        }

        let content = std::fs::read_to_string(&config_path)
            .context("Failed to read localconfig.vdf")?;

        // Check if the current options contain our script extender
        let current = parse_text_vdf_launch_options(&content, extender.steam_app_id);
        if let Some(ref opts) = current {
            if !opts.contains(&extender.exe_name) {
                continue; // Not patched by us
            }
        } else {
            continue; // No launch options set
        }

        // Backup
        let backup = config_path.with_extension("vdf.bak");
        std::fs::copy(&config_path, &backup)
            .context("Failed to back up localconfig.vdf")?;

        // Remove our launch options (set to None to clear the key)
        let updated = set_text_vdf_launch_options(&content, extender.steam_app_id, None)?;

        std::fs::write(&config_path, &updated)
            .context("Failed to write localconfig.vdf")?;

        log::info!(
            "Unpatched Steam launch options for {} (app {})",
            extender.display_name,
            extender.steam_app_id
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_corkscrew_entry_creation() {
        let entry = build_corkscrew_entry("0", "/opt/Corkscrew.AppImage", "/opt/icon.png");
        assert_eq!(entry.index, "0");
        assert!(entry_is_corkscrew(&entry));
    }

    #[test]
    fn test_vdf_roundtrip() {
        let entries = vec![build_corkscrew_entry(
            "0",
            "/opt/Corkscrew.AppImage",
            "/opt/icon.png",
        )];

        let data = write_shortcuts_vdf(&entries);
        let parsed = parse_shortcuts_vdf(&data).expect("Failed to parse VDF");

        assert_eq!(parsed.len(), 1);
        assert!(entry_is_corkscrew(&parsed[0]));
    }

    #[test]
    fn test_vdf_roundtrip_multiple() {
        let entries = vec![
            build_corkscrew_entry("0", "/opt/Corkscrew.AppImage", "/opt/icon.png"),
            build_corkscrew_entry("1", "/opt/Other.AppImage", "/opt/other.png"),
        ];

        let data = write_shortcuts_vdf(&entries);
        let parsed = parse_shortcuts_vdf(&data).expect("Failed to parse VDF");

        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn test_empty_vdf() {
        let entries: Vec<ShortcutEntry> = Vec::new();
        let data = write_shortcuts_vdf(&entries);
        let parsed = parse_shortcuts_vdf(&data).expect("Failed to parse empty VDF");
        assert!(parsed.is_empty());
    }

    #[test]
    fn test_generate_app_id() {
        let id = generate_app_id("/opt/Corkscrew.AppImage", "Corkscrew");
        // Should have the high bit set
        assert!(id & 0x80000000 != 0);
    }

    #[test]
    fn test_parse_text_vdf_launch_options() {
        let vdf = r#"
"UserLocalConfigStore"
{
    "Software"
    {
        "Valve"
        {
            "Steam"
            {
                "apps"
                {
                    "489830"
                    {
                        "LastPlayed"		"1711234567"
                        "LaunchOptions"		"skse64_loader.exe %command%"
                    }
                    "377160"
                    {
                        "LastPlayed"		"1711234568"
                    }
                }
            }
        }
    }
}
"#;
        // Should find SKSE launch options for Skyrim SE
        let result = parse_text_vdf_launch_options(vdf, 489830);
        assert_eq!(result, Some("skse64_loader.exe %command%".to_string()));

        // FO4 has no launch options
        let result2 = parse_text_vdf_launch_options(vdf, 377160);
        assert_eq!(result2, None);

        // Unknown app
        let result3 = parse_text_vdf_launch_options(vdf, 999999);
        assert_eq!(result3, None);
    }

    #[test]
    fn test_set_text_vdf_launch_options_replace() {
        let vdf = r#""UserLocalConfigStore"
{
    "Software"
    {
        "Valve"
        {
            "Steam"
            {
                "apps"
                {
                    "489830"
                    {
                        "LastPlayed"		"1711234567"
                        "LaunchOptions"		"old_option %command%"
                    }
                }
            }
        }
    }
}"#;
        let updated =
            set_text_vdf_launch_options(vdf, 489830, Some("skse64_loader.exe %command%"))
                .expect("Failed to set launch options");
        let opts = parse_text_vdf_launch_options(&updated, 489830);
        assert_eq!(opts, Some("skse64_loader.exe %command%".to_string()));
    }

    #[test]
    fn test_set_text_vdf_launch_options_insert() {
        let vdf = r#""UserLocalConfigStore"
{
    "Software"
    {
        "Valve"
        {
            "Steam"
            {
                "apps"
                {
                    "489830"
                    {
                        "LastPlayed"		"1711234567"
                    }
                }
            }
        }
    }
}"#;
        let updated =
            set_text_vdf_launch_options(vdf, 489830, Some("skse64_loader.exe %command%"))
                .expect("Failed to insert launch options");
        let opts = parse_text_vdf_launch_options(&updated, 489830);
        assert_eq!(opts, Some("skse64_loader.exe %command%".to_string()));
    }

    #[test]
    fn test_set_text_vdf_launch_options_remove() {
        let vdf = r#""UserLocalConfigStore"
{
    "Software"
    {
        "Valve"
        {
            "Steam"
            {
                "apps"
                {
                    "489830"
                    {
                        "LastPlayed"		"1711234567"
                        "LaunchOptions"		"skse64_loader.exe %command%"
                    }
                }
            }
        }
    }
}"#;
        let updated = set_text_vdf_launch_options(vdf, 489830, None)
            .expect("Failed to remove launch options");
        let opts = parse_text_vdf_launch_options(&updated, 489830);
        assert_eq!(opts, None);
    }

    #[test]
    fn test_known_script_extenders() {
        let extenders = known_script_extenders();
        assert!(extenders.len() >= 3);
        assert!(extenders.iter().any(|e| e.game_id == "skyrimse"));
        assert!(extenders.iter().any(|e| e.game_id == "fallout4"));
        assert!(extenders.iter().any(|e| e.game_id == "oblivion"));
    }
}
