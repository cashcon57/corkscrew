//! NXM protocol handler registration for Linux.
//!
//! Registers Corkscrew as the handler for `nxm://` URLs so that clicking
//! "Download with Mod Manager" on NexusMods opens Corkscrew.

#[cfg(target_os = "linux")]
use log::info;
#[cfg(target_os = "linux")]
use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::process::Command;

/// The .desktop file content for NXM and corkscrew:// handler registration.
#[allow(dead_code)]
const DESKTOP_FILE_CONTENT: &str = r#"[Desktop Entry]
Type=Application
Name=Corkscrew URL Handler
Comment=Handle NexusMods and Corkscrew protocol links
Exec=corkscrew --url %u
Terminal=false
MimeType=x-scheme-handler/nxm;x-scheme-handler/corkscrew;
NoDisplay=true
Categories=Game;
"#;

#[allow(dead_code)]
const DESKTOP_FILE_NAME: &str = "corkscrew-nxm.desktop";

/// Register Corkscrew as the NXM protocol handler on Linux.
///
/// Creates a .desktop file and registers it via xdg-mime.
pub fn register_nxm_handler() -> Result<(), String> {
    #[cfg(not(target_os = "linux"))]
    {
        return Err("NXM handler registration is only supported on Linux".to_string());
    }

    #[cfg(target_os = "linux")]
    {
        let desktop_path = get_desktop_file_path()?;

        // Write .desktop file
        std::fs::create_dir_all(desktop_path.parent().unwrap())
            .map_err(|e| format!("Failed to create directory: {}", e))?;
        std::fs::write(&desktop_path, DESKTOP_FILE_CONTENT)
            .map_err(|e| format!("Failed to write .desktop file: {}", e))?;

        info!(
            "Wrote NXM handler .desktop file to {}",
            desktop_path.display()
        );

        // Register via xdg-mime for both nxm:// and corkscrew://
        for scheme in &["x-scheme-handler/nxm", "x-scheme-handler/corkscrew"] {
            let output = Command::new("xdg-mime")
                .arg("default")
                .arg(DESKTOP_FILE_NAME)
                .arg(scheme)
                .output()
                .map_err(|e| format!("Failed to run xdg-mime: {}", e))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(format!("xdg-mime failed for {}: {}", scheme, stderr));
            }
        }

        // Update desktop database
        if let Some(app_dir) = desktop_path.parent() {
            let _ = Command::new("update-desktop-database")
                .arg(app_dir)
                .output();
        }

        info!("Registered Corkscrew as NXM protocol handler");
        Ok(())
    }
}

/// Unregister Corkscrew as the NXM protocol handler.
pub fn unregister_nxm_handler() -> Result<(), String> {
    #[cfg(not(target_os = "linux"))]
    {
        return Err("NXM handler registration is only supported on Linux".to_string());
    }

    #[cfg(target_os = "linux")]
    {
        let desktop_path = get_desktop_file_path()?;

        if desktop_path.exists() {
            std::fs::remove_file(&desktop_path)
                .map_err(|e| format!("Failed to remove .desktop file: {}", e))?;

            // Update desktop database
            if let Some(app_dir) = desktop_path.parent() {
                let _ = Command::new("update-desktop-database")
                    .arg(app_dir)
                    .output();
            }

            info!("Unregistered Corkscrew NXM protocol handler");
        }

        Ok(())
    }
}

/// Check if Corkscrew is registered as the NXM handler.
pub fn is_nxm_handler_registered() -> bool {
    #[cfg(not(target_os = "linux"))]
    {
        return false;
    }

    #[cfg(target_os = "linux")]
    {
        let output = Command::new("xdg-mime")
            .arg("query")
            .arg("default")
            .arg("x-scheme-handler/nxm")
            .output();

        match output {
            Ok(o) => {
                let handler = String::from_utf8_lossy(&o.stdout).trim().to_string();
                handler == DESKTOP_FILE_NAME
            }
            Err(_) => false,
        }
    }
}

/// Parse an NXM URL into its components.
///
/// Format: `nxm://skyrimspecialedition/mods/12345/files/67890`
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct NxmUrl {
    pub game_domain: String,
    pub mod_id: i64,
    pub file_id: i64,
    pub key: Option<String>,
    pub expires: Option<i64>,
}

// ---------------------------------------------------------------------------
// Corkscrew protocol handler
// ---------------------------------------------------------------------------

/// Parsed `corkscrew://` URL action.
///
/// Supported schemes:
/// - `corkscrew://install/nexus/{game}/{mod_id}` — open install dialog
/// - `corkscrew://launch/{game_id}/{bottle}` — launch a game
/// - `corkscrew://profile/{code}` — import a shared profile code
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "action")]
pub enum CorkscrewAction {
    #[serde(rename = "install_nexus")]
    InstallNexus {
        game_domain: String,
        mod_id: i64,
    },
    #[serde(rename = "launch")]
    Launch {
        game_id: String,
        bottle_name: String,
    },
    #[serde(rename = "import_profile")]
    ImportProfile {
        code: String,
    },
}

/// Parse a `corkscrew://` URL into an action.
#[allow(dead_code)]
pub fn parse_corkscrew_url(url: &str) -> Result<CorkscrewAction, String> {
    let path = url
        .strip_prefix("corkscrew://")
        .ok_or("Not a corkscrew:// URL")?;
    let path = path.trim_end_matches('/');
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    if parts.is_empty() {
        return Err("Empty corkscrew:// URL".to_string());
    }

    match parts[0] {
        "install" => {
            // corkscrew://install/nexus/{game}/{mod_id}
            if parts.len() < 4 || parts[1] != "nexus" {
                return Err("Expected corkscrew://install/nexus/{game}/{mod_id}".to_string());
            }
            let game_domain = parts[2].to_string();
            let mod_id: i64 = parts[3]
                .parse()
                .map_err(|_| format!("Invalid mod ID: {}", parts[3]))?;
            Ok(CorkscrewAction::InstallNexus { game_domain, mod_id })
        }
        "launch" => {
            // corkscrew://launch/{game_id}/{bottle}
            if parts.len() < 3 {
                return Err("Expected corkscrew://launch/{game_id}/{bottle}".to_string());
            }
            let game_id = parts[1].to_string();
            // Bottle name may contain URL-encoded spaces
            let bottle_name = urlencoding_decode(parts[2]);
            Ok(CorkscrewAction::Launch { game_id, bottle_name })
        }
        "profile" => {
            // corkscrew://profile/{code}
            if parts.len() < 2 {
                return Err("Expected corkscrew://profile/{code}".to_string());
            }
            let code = parts[1..].join("/");
            Ok(CorkscrewAction::ImportProfile { code })
        }
        other => Err(format!("Unknown corkscrew:// action: {}", other)),
    }
}

/// Simple percent-decoding for URL path segments.
fn urlencoding_decode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let hi = chars.next().and_then(|c| (c as char).to_digit(16));
            let lo = chars.next().and_then(|c| (c as char).to_digit(16));
            if let (Some(h), Some(l)) = (hi, lo) {
                result.push((h * 16 + l) as u8 as char);
            } else {
                result.push('%');
            }
        } else if b == b'+' {
            result.push(' ');
        } else {
            result.push(b as char);
        }
    }
    result
}

#[allow(dead_code)]
pub fn parse_nxm_url(url: &str) -> Result<NxmUrl, String> {
    let url = url.strip_prefix("nxm://").ok_or("Not an NXM URL")?;
    // Strip trailing slash if present
    let url = url.trim_end_matches('/');
    let parts: Vec<&str> = url.split('/').filter(|s| !s.is_empty()).collect();

    // nxm://game_domain/mods/mod_id/files/file_id?key=...&expires=...
    if parts.len() < 5 {
        return Err("Invalid NXM URL format: not enough segments".to_string());
    }

    let game_domain = parts[0].to_string();

    // Sanitize game_domain — must be alphanumeric/hyphens only (no path separators or null bytes)
    if game_domain.contains('\0')
        || game_domain.contains('/')
        || game_domain.contains('\\')
        || game_domain.contains("..")
    {
        return Err(format!("Invalid game domain: {}", game_domain));
    }

    if parts[1] != "mods" {
        return Err(format!("Expected 'mods' segment, got '{}'", parts[1]));
    }

    let mod_id: i64 = parts[2]
        .parse()
        .map_err(|_| format!("Invalid mod ID: {}", parts[2]))?;

    if parts[3] != "files" {
        return Err(format!("Expected 'files' segment, got '{}'", parts[3]));
    }

    // File ID may have query string attached
    let file_part = parts[4];
    let (file_id_str, query) = if let Some(idx) = file_part.find('?') {
        (&file_part[..idx], Some(&file_part[idx + 1..]))
    } else {
        (file_part, None)
    };

    let file_id: i64 = file_id_str
        .parse()
        .map_err(|_| format!("Invalid file ID: {}", file_id_str))?;

    let mut key = None;
    let mut expires = None;

    if let Some(query_str) = query {
        for param in query_str.split('&') {
            let mut kv = param.splitn(2, '=');
            if let (Some(k), Some(v)) = (kv.next(), kv.next()) {
                match k {
                    "key" => key = Some(v.to_string()),
                    "expires" => expires = v.parse().ok(),
                    _ => {}
                }
            }
        }
    }

    Ok(NxmUrl {
        game_domain,
        mod_id,
        file_id,
        key,
        expires,
    })
}

#[cfg(target_os = "linux")]
fn get_desktop_file_path() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("Could not find home directory")?;
    Ok(home
        .join(".local/share/applications")
        .join(DESKTOP_FILE_NAME))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_nxm_url_basic() {
        let url = "nxm://skyrimspecialedition/mods/12345/files/67890";
        let parsed = parse_nxm_url(url).unwrap();
        assert_eq!(parsed.game_domain, "skyrimspecialedition");
        assert_eq!(parsed.mod_id, 12345);
        assert_eq!(parsed.file_id, 67890);
        assert!(parsed.key.is_none());
    }

    #[test]
    fn test_parse_nxm_url_with_key() {
        let url =
            "nxm://skyrimspecialedition/mods/12345/files/67890?key=abc123&expires=1234567890";
        let parsed = parse_nxm_url(url).unwrap();
        assert_eq!(parsed.game_domain, "skyrimspecialedition");
        assert_eq!(parsed.mod_id, 12345);
        assert_eq!(parsed.file_id, 67890);
        assert_eq!(parsed.key, Some("abc123".to_string()));
        assert_eq!(parsed.expires, Some(1234567890));
    }

    #[test]
    fn test_parse_nxm_url_invalid() {
        assert!(parse_nxm_url("https://nexusmods.com").is_err());
        assert!(parse_nxm_url("nxm://game").is_err());
    }

    #[test]
    fn test_parse_corkscrew_url_install() {
        let action = parse_corkscrew_url("corkscrew://install/nexus/skyrimspecialedition/12345").unwrap();
        match action {
            CorkscrewAction::InstallNexus { game_domain, mod_id } => {
                assert_eq!(game_domain, "skyrimspecialedition");
                assert_eq!(mod_id, 12345);
            }
            _ => panic!("Expected InstallNexus action"),
        }
    }

    #[test]
    fn test_parse_corkscrew_url_launch() {
        let action = parse_corkscrew_url("corkscrew://launch/skyrimse/My%20Bottle").unwrap();
        match action {
            CorkscrewAction::Launch { game_id, bottle_name } => {
                assert_eq!(game_id, "skyrimse");
                assert_eq!(bottle_name, "My Bottle");
            }
            _ => panic!("Expected Launch action"),
        }
    }

    #[test]
    fn test_parse_corkscrew_url_profile() {
        let action = parse_corkscrew_url("corkscrew://profile/CRKS-7x9Km-4pQw").unwrap();
        match action {
            CorkscrewAction::ImportProfile { code } => {
                assert_eq!(code, "CRKS-7x9Km-4pQw");
            }
            _ => panic!("Expected ImportProfile action"),
        }
    }

    #[test]
    fn test_parse_corkscrew_url_invalid() {
        assert!(parse_corkscrew_url("https://example.com").is_err());
        assert!(parse_corkscrew_url("corkscrew://").is_err());
        assert!(parse_corkscrew_url("corkscrew://unknown/action").is_err());
    }
}
