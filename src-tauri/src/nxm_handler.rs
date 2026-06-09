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
/// Creates a .desktop file and registers it via xdg-mime. Returns a clear,
/// actionable error if the required `xdg-utils` package isn't installed.
pub fn register_nxm_handler() -> Result<(), String> {
    #[cfg(not(target_os = "linux"))]
    {
        return Err("NXM handler registration is only supported on Linux".to_string());
    }

    #[cfg(target_os = "linux")]
    {
        // Pre-flight: confirm xdg-mime is on PATH. Some minimal distros
        // (Alpine, container images, busybox-only setups) don't ship it by
        // default and the bare `Command::new` failure mode is opaque.
        if !binary_on_path("xdg-mime") {
            return Err(install_hint_for("xdg-mime", "xdg-utils"));
        }

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

        // Update desktop database — optional. The helper checks whether
        // the binary is on PATH and logs a hint if not, but never fails
        // the registration. If the binary is missing the system will pick
        // up the .desktop file at next session anyway.
        if let Some(app_dir) = desktop_path.parent() {
            try_refresh_desktop_database(app_dir);
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

            // Refresh the desktop database. Same gated helper as register —
            // skips the spawn (and the cryptic ENOENT) if the binary isn't
            // installed, which mirrors what the register path was already
            // doing.
            if let Some(app_dir) = desktop_path.parent() {
                try_refresh_desktop_database(app_dir);
            }

            info!("Unregistered Corkscrew NXM protocol handler");
        }

        Ok(())
    }
}

/// Refresh the freedesktop applications database for `dir`. No-op (with a
/// warn-level log) when `update-desktop-database` is not on `$PATH`.
///
/// Used by both `register_nxm_handler` and `unregister_nxm_handler` so we
/// don't get the ugly `Failed to run update-desktop-database: ENOENT` on
/// minimal distros that don't ship desktop-file-utils.
#[cfg(target_os = "linux")]
fn try_refresh_desktop_database(dir: &std::path::Path) {
    if !binary_on_path("update-desktop-database") {
        log::warn!(
            "update-desktop-database not on PATH ({}). Skipping cache refresh; \
             the .desktop change may take a session restart to pick up.",
            install_hint_for("update-desktop-database", "desktop-file-utils")
        );
        return;
    }
    let _ = Command::new("update-desktop-database").arg(dir).output();
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
    InstallNexus { game_domain: String, mod_id: i64 },
    #[serde(rename = "launch")]
    Launch {
        game_id: String,
        bottle_name: String,
    },
    #[serde(rename = "import_profile")]
    ImportProfile { code: String },
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
            Ok(CorkscrewAction::InstallNexus {
                game_domain,
                mod_id,
            })
        }
        "launch" => {
            // corkscrew://launch/{game_id}/{bottle}
            if parts.len() < 3 {
                return Err("Expected corkscrew://launch/{game_id}/{bottle}".to_string());
            }
            let game_id = parts[1].to_string();
            // Bottle name may contain URL-encoded spaces
            let bottle_name = urlencoding_decode(parts[2]);
            Ok(CorkscrewAction::Launch {
                game_id,
                bottle_name,
            })
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

/// Outcome of [`route_nxm_game`] — describes how an NXM URL's `game_domain`
/// was matched against currently-known games.
///
/// Mirrors Vortex's `InstallManager.ts` ~L1418-1450 fallback: when the
/// declared game isn't installed, route to the user's active selection
/// rather than failing outright. The user sees a clear warning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NxmRoute {
    /// The NXM `game_domain` matched a registered game directly. Use the
    /// returned `game_id`.
    Recognized { game_id: String },
    /// The NXM `game_domain` did NOT match any registered game. Fall back
    /// to `active_game_id`. UI should show `warning` to the user.
    Fallback {
        active_game_id: String,
        warning: String,
    },
    /// No active game profile is set, and the declared `game_domain` is
    /// unknown. Cannot proceed. UI should show `error` to the user.
    NoActiveGame { error: String },
}

/// Pure helper: given an NXM URL `game_domain`, the list of known
/// `(game_id, nexus_slug)` pairs from the registry, and the currently
/// active `game_id` (if any), decide where to route the install.
///
/// - If `game_domain` matches one of `known_games[i].nexus_slug` (case
///   insensitive) — return `Recognized(game_id)`.
/// - Else, if `active_game_id` is `Some` — return `Fallback` with a
///   warning string callers can surface to the UI.
/// - Else — `NoActiveGame`.
///
/// Pure, no I/O. Callers wire the inputs from the database / app state.
pub fn route_nxm_game(
    game_domain: &str,
    known_games: &[(String, String)],
    active_game_id: Option<&str>,
) -> NxmRoute {
    let domain_lower = game_domain.to_lowercase();

    // 1. Match by nexus_slug.
    for (gid, slug) in known_games {
        if slug.eq_ignore_ascii_case(&domain_lower) {
            return NxmRoute::Recognized {
                game_id: gid.clone(),
            };
        }
    }
    // 2. Match by game_id (some custom games use the slug as the id).
    for (gid, _slug) in known_games {
        if gid.eq_ignore_ascii_case(&domain_lower) {
            return NxmRoute::Recognized {
                game_id: gid.clone(),
            };
        }
    }

    // 3. Fall back to active game.
    match active_game_id {
        Some(active) if !active.is_empty() => NxmRoute::Fallback {
            active_game_id: active.to_string(),
            warning: format!(
                "Routing this NXM mod to '{}' — no installed game matched \
                 the link's domain '{}'. Confirm this is the right game \
                 before installing.",
                active, game_domain
            ),
        },
        _ => NxmRoute::NoActiveGame {
            error: format!(
                "Cannot install: NXM link declares game '{}', which is not \
                 installed, and no active game is selected. Open Corkscrew, \
                 select a game, then re-click the download link.",
                game_domain
            ),
        },
    }
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

/// Check whether a binary is reachable via `$PATH`. Pure-Rust, no `which`
/// crate dependency.
///
/// Honors the executable bit on Unix — `is_file()` alone matches data
/// files and shell-completion stubs that happen to share a name (e.g.
/// `/usr/share/bash-completion/completions/xdg-mime` on some distros sits
/// alongside `/usr/bin/xdg-mime`, and the bash-completion file is plain
/// text). Without the mode check, we'd report a false positive and then
/// later fail trying to `Command::spawn()` it.
#[cfg(target_os = "linux")]
fn binary_on_path(name: &str) -> bool {
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path_var).any(|dir| is_executable_file(&dir.join(name)))
}

/// Return true iff `path` exists, is a regular file, and has at least one
/// of the user/group/other executable bits set. Same semantics as
/// `access(path, X_OK)` against the inode mode (no setuid/setgid/ACL
/// awareness, but neither does `access()` from a non-root process for
/// our purposes).
#[cfg(target_os = "linux")]
fn is_executable_file(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let md = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return false,
    };
    md.is_file() && md.permissions().mode() & 0o111 != 0
}

/// Build a distro-aware install hint for a missing tool.
#[cfg(target_os = "linux")]
fn install_hint_for(binary: &str, package: &str) -> String {
    format!(
        "{} not found on PATH. Install the {} package: \
         `sudo pacman -S {}` (Arch / CachyOS / Manjaro), \
         `sudo apt install {}` (Debian / Ubuntu / Pop!_OS), \
         `sudo dnf install {}` (Fedora / Bazzite). \
         After installing, retry from Settings.",
        binary, package, package, package, package
    )
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
        let url = "nxm://skyrimspecialedition/mods/12345/files/67890?key=abc123&expires=1234567890";
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
        let action =
            parse_corkscrew_url("corkscrew://install/nexus/skyrimspecialedition/12345").unwrap();
        match action {
            CorkscrewAction::InstallNexus {
                game_domain,
                mod_id,
            } => {
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
            CorkscrewAction::Launch {
                game_id,
                bottle_name,
            } => {
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

    // --- route_nxm_game ---------------------------------------------------

    fn known(games: &[(&str, &str)]) -> Vec<(String, String)> {
        games
            .iter()
            .map(|(g, s)| (g.to_string(), s.to_string()))
            .collect()
    }

    #[test]
    fn route_nxm_recognized_by_slug() {
        let games = known(&[
            ("skyrimse", "skyrimspecialedition"),
            ("fallout4", "fallout4"),
        ]);
        let r = route_nxm_game("skyrimspecialedition", &games, Some("fallout4"));
        match r {
            NxmRoute::Recognized { game_id } => assert_eq!(game_id, "skyrimse"),
            other => panic!("expected Recognized, got {other:?}"),
        }
    }

    #[test]
    fn route_nxm_recognized_by_game_id() {
        // Some custom games use the slug as the game_id.
        let games = known(&[("rerequiem", "rerequiem")]);
        let r = route_nxm_game("rerequiem", &games, None);
        match r {
            NxmRoute::Recognized { game_id } => assert_eq!(game_id, "rerequiem"),
            other => panic!("expected Recognized, got {other:?}"),
        }
    }

    #[test]
    fn route_nxm_recognition_is_case_insensitive() {
        let games = known(&[("skyrimse", "SkyrimSpecialEdition")]);
        let r = route_nxm_game("skyrimspecialedition", &games, None);
        assert!(matches!(r, NxmRoute::Recognized { .. }));
    }

    #[test]
    fn route_nxm_fallback_when_unknown_with_active() {
        let games = known(&[("skyrimse", "skyrimspecialedition")]);
        let r = route_nxm_game("oblivionremastered", &games, Some("skyrimse"));
        match r {
            NxmRoute::Fallback {
                active_game_id,
                warning,
            } => {
                assert_eq!(active_game_id, "skyrimse");
                assert!(warning.contains("oblivionremastered"));
                assert!(warning.contains("skyrimse"));
            }
            other => panic!("expected Fallback, got {other:?}"),
        }
    }

    #[test]
    fn route_nxm_no_active_game_returns_error() {
        let games = known(&[("skyrimse", "skyrimspecialedition")]);
        let r = route_nxm_game("oblivionremastered", &games, None);
        match r {
            NxmRoute::NoActiveGame { error } => {
                assert!(error.contains("oblivionremastered"));
            }
            other => panic!("expected NoActiveGame, got {other:?}"),
        }
    }

    #[test]
    fn route_nxm_empty_active_treated_as_none() {
        let games = known(&[("skyrimse", "skyrimspecialedition")]);
        let r = route_nxm_game("oblivionremastered", &games, Some(""));
        assert!(matches!(r, NxmRoute::NoActiveGame { .. }));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_is_executable_file_requires_executable_bit() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();

        // Non-existent path: false.
        assert!(!is_executable_file(&temp.path().join("nonexistent")));

        // Plain text file with mode 0644: false. Mirrors the bash-completion
        // stub case that motivated this check.
        let non_exec = temp.path().join("nonexec");
        fs::write(&non_exec, b"# completion data\n").unwrap();
        let mut perms = fs::metadata(&non_exec).unwrap().permissions();
        perms.set_mode(0o644);
        fs::set_permissions(&non_exec, perms).unwrap();
        assert!(!is_executable_file(&non_exec));

        // Same content with the user-execute bit set: true.
        let exec = temp.path().join("exec");
        fs::write(&exec, b"#!/bin/sh\necho hi\n").unwrap();
        let mut perms = fs::metadata(&exec).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&exec, perms).unwrap();
        assert!(is_executable_file(&exec));

        // Group-only or other-only execute is sufficient (mirrors access()).
        let g_only = temp.path().join("g-only");
        fs::write(&g_only, b"x").unwrap();
        let mut perms = fs::metadata(&g_only).unwrap().permissions();
        perms.set_mode(0o010);
        fs::set_permissions(&g_only, perms).unwrap();
        assert!(is_executable_file(&g_only));

        // Directories must not register as executable files even though
        // they often have the +x bit set.
        assert!(!is_executable_file(temp.path()));
    }
}
