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

/// Detect a Steam installation on this system.
#[cfg(target_os = "linux")]
pub fn detect_steam_installation() -> Option<SteamInfo> {
    let home = dirs::home_dir()?;

    let candidates = [
        home.join(".steam/steam"),
        home.join(".local/share/Steam"),
        home.join(".var/app/com.valvesoftware.Steam/.steam/steam"), // Flatpak
    ];

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
                let value = u32::from_le_bytes([
                    data[pos],
                    data[pos + 1],
                    data[pos + 2],
                    data[pos + 3],
                ]);
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
            let data = std::fs::read(&shortcuts_path)
                .context("Failed to read shortcuts.vdf")?;
            parse_shortcuts_vdf(&data).unwrap_or_default()
        } else {
            Vec::new()
        };

        // Check if already registered — update if so
        let existing_idx = entries.iter().position(|e| entry_is_corkscrew(e));
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

        std::fs::write(&shortcuts_path, &vdf_data)
            .context("Failed to write shortcuts.vdf")?;
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

        let data = std::fs::read(&shortcuts_path)
            .context("Failed to read shortcuts.vdf")?;

        let mut entries = parse_shortcuts_vdf(&data).unwrap_or_default();

        let before_len = entries.len();
        entries.retain(|e| !entry_is_corkscrew(e));

        if entries.len() < before_len {
            // Re-index entries
            for (i, entry) in entries.iter_mut().enumerate() {
                entry.index = i.to_string();
            }

            let vdf_data = write_shortcuts_vdf(&entries);
            std::fs::write(&shortcuts_path, &vdf_data)
                .context("Failed to write shortcuts.vdf")?;

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
                if entries.iter().any(|e| entry_is_corkscrew(e)) {
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
        let icon_path = home
            .join(".local/share/icons/hicolor/128x128/apps/corkscrew.png");
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
        .map(|info| is_registered_in_steam(info))
        .unwrap_or(false);

    SteamStatus {
        installed: steam_info.is_some(),
        registered,
        is_deck: is_steam_deck(),
    }
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
        let entries = vec![
            build_corkscrew_entry("0", "/opt/Corkscrew.AppImage", "/opt/icon.png"),
        ];

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
}
