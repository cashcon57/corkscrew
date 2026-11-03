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

/// URL-handler-only desktop entry written by [`register_nxm_handler`]
/// (Settings → "Register NXM handler").
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const DESKTOP_FILE_NAME: &str = "corkscrew-nxm.desktop";

/// Full launcher desktop entry written by
/// `steam_integration::create_desktop_entry` (the "Add to Steam" / SteamOS
/// auto-setup path). It also declares `x-scheme-handler/nxm` and is set as
/// the xdg-mime default, so it counts as "Corkscrew is the NXM handler" too.
pub(crate) const MAIN_DESKTOP_FILE_NAME: &str = "corkscrew.desktop";

/// Desktop entries Corkscrew itself writes into `~/.local/share/applications`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const OWN_DESKTOP_FILES: [&str; 2] = [DESKTOP_FILE_NAME, MAIN_DESKTOP_FILE_NAME];

/// Build the .desktop file content for NXM and corkscrew:// handler
/// registration, launching `exe_path` (the AppImage path when running as
/// one — `corkscrew` is not on `$PATH` for AppImage installs, the usual
/// SteamOS setup).
///
/// The URL must be the *only* argument: `tauri-plugin-deep-link` on Linux
/// ignores argv unless it is exactly `[bin, url]`, so any extra flag (the
/// old `--url %u`) silently drops the link before `onOpenUrl` sees it.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn desktop_file_content(exe_path: &str) -> Result<String, String> {
    Ok(format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Corkscrew URL Handler\n\
         Comment=Handle NexusMods and Corkscrew protocol links\n\
         Exec={} %u\n\
         Terminal=false\n\
         MimeType=x-scheme-handler/nxm;x-scheme-handler/corkscrew;\n\
         NoDisplay=true\n\
         Categories=Game;\n",
        quote_exec_arg(exe_path)?
    ))
}

/// Quote one argument for a .desktop `Exec=` key per the Desktop Entry
/// spec: wrap in double quotes, backslash-escape `"` `` ` `` `$` `\`, then
/// apply the string-value escaping (each `\` doubled) and escape `%` as `%%`
/// so field codes can't be injected. Control characters can't be
/// represented and are rejected.
pub(crate) fn quote_exec_arg(arg: &str) -> Result<String, String> {
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    for c in arg.chars() {
        match c {
            '"' | '`' | '$' => {
                out.push_str("\\\\");
                out.push(c);
            }
            '\\' => out.push_str("\\\\\\\\"),
            '%' => out.push_str("%%"),
            c if c.is_control() => {
                return Err(format!(
                    "Executable path contains a control character and can't be \
                     written to a .desktop file: {:?}",
                    arg
                ));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    Ok(out)
}

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
        let exe_path = crate::steam_integration::get_exe_path()
            .map_err(|e| format!("Failed to resolve Corkscrew executable: {}", e))?;
        let content = desktop_file_content(&exe_path)?;

        // Write .desktop file
        std::fs::create_dir_all(desktop_path.parent().unwrap())
            .map_err(|e| format!("Failed to create directory: {}", e))?;
        std::fs::write(&desktop_path, content)
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
///
/// True only when the xdg-mime default for `x-scheme-handler/nxm` is one of
/// Corkscrew's desktop entries *and* that entry's `Exec=` still launches an
/// existing program. A handler whose target was moved or renamed (e.g. an
/// AppImage replaced by a differently named download) silently does nothing
/// on click, so it is reported as not registered and the UI offers to fix it.
pub fn is_nxm_handler_registered() -> bool {
    #[cfg(not(target_os = "linux"))]
    {
        return false;
    }

    #[cfg(target_os = "linux")]
    {
        current_handler_status()
            .map(|(_, status)| status.launches_corkscrew())
            .unwrap_or(false)
    }
}

/// Startup self-heal (Linux): if the registered `nxm://` handler is one of
/// the desktop entries Corkscrew wrote itself, but its `Exec=` no longer
/// points at the running executable (AppImage moved/renamed/updated under a
/// new name), rewrite it to launch the current executable.
///
/// Best-effort: never fails, only logs. A handler that belongs to another
/// app, or whose desktop file was deliberately removed, is left alone.
#[cfg(target_os = "linux")]
pub fn self_heal_handler_registration() {
    let Some((handler, status)) = current_handler_status() else {
        return;
    };
    let Some(file_name) = self_heal_target(&handler, &status) else {
        return;
    };
    let exe_path = match crate::steam_integration::get_exe_path() {
        Ok(p) => p,
        Err(e) => {
            log::warn!("NXM handler self-heal: cannot resolve executable: {}", e);
            return;
        }
    };
    log::info!(
        "NXM handler self-heal: {} is {:?}; rewriting Exec to {}",
        file_name,
        status,
        exe_path
    );

    let result = if file_name == MAIN_DESKTOP_FILE_NAME {
        crate::steam_integration::create_desktop_entry(&exe_path)
            .map(|_| ())
            .map_err(|e| e.to_string())
    } else {
        rewrite_nxm_desktop_file(&exe_path)
    };
    if let Err(e) = result {
        log::warn!("NXM handler self-heal failed for {}: {}", file_name, e);
    }
}

/// Rewrite `corkscrew-nxm.desktop` in place. The xdg-mime association is by
/// file name and already points at it, so only the file contents change.
#[cfg(target_os = "linux")]
fn rewrite_nxm_desktop_file(exe_path: &str) -> Result<(), String> {
    let desktop_path = get_desktop_file_path()?;
    let content = desktop_file_content(exe_path)?;
    std::fs::write(&desktop_path, content)
        .map_err(|e| format!("Failed to write {}: {}", desktop_path.display(), e))?;
    if let Some(app_dir) = desktop_path.parent() {
        try_refresh_desktop_database(app_dir);
    }
    Ok(())
}

/// How the current `x-scheme-handler/nxm` default relates to Corkscrew.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) enum HandlerStatus {
    /// The default handler is some other application (or none).
    NotCorkscrew,
    /// The default names a Corkscrew desktop entry, but the file doesn't
    /// exist (e.g. removed by "Unregister", which doesn't reset xdg-mime).
    Missing,
    /// The desktop file exists but its `Exec=` is missing/unparseable or
    /// launches a program that no longer exists. Clicks do nothing.
    Stale,
    /// `Exec=` launches an existing program, but not the running executable.
    Outdated,
    /// `Exec=` launches the running executable.
    Current,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
impl HandlerStatus {
    /// Clicking an nxm:// link will start *some* Corkscrew binary.
    pub(crate) fn launches_corkscrew(&self) -> bool {
        matches!(self, HandlerStatus::Outdated | HandlerStatus::Current)
    }
}

/// Is `desktop_id` (as printed by `xdg-mime query default`) one of
/// Corkscrew's desktop entries? Case-insensitive so a packaged
/// `Corkscrew.desktop` (deb/rpm bundle) also counts.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn is_corkscrew_desktop_id(desktop_id: &str) -> bool {
    OWN_DESKTOP_FILES
        .iter()
        .any(|own| own.eq_ignore_ascii_case(desktop_id.trim()))
}

/// Classify the registered NXM handler. Pure: filesystem checks are injected
/// so this can be unit-tested on any platform.
///
/// - `desktop_content`: contents of the handler's desktop file, `None` if
///   it couldn't be found/read.
/// - `program_exists(program)`: whether the `Exec=` program can be launched.
/// - `same_program(a, b)`: whether two paths name the same executable.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn classify_handler(
    default_handler: &str,
    desktop_content: Option<&str>,
    current_exe: &str,
    program_exists: impl Fn(&str) -> bool,
    same_program: impl Fn(&str, &str) -> bool,
) -> HandlerStatus {
    if !is_corkscrew_desktop_id(default_handler) {
        return HandlerStatus::NotCorkscrew;
    }
    let Some(content) = desktop_content else {
        return HandlerStatus::Missing;
    };
    let args = desktop_entry_exec(content).and_then(parse_exec_args);
    let Some(program) = args.as_deref().and_then(exec_program) else {
        return HandlerStatus::Stale;
    };
    if !program_exists(program) {
        return HandlerStatus::Stale;
    }
    if same_program(program, current_exe) {
        HandlerStatus::Current
    } else {
        HandlerStatus::Outdated
    }
}

/// Which of Corkscrew's own desktop files (if any) the startup self-heal
/// should rewrite. Only exact names we write into the user's applications
/// directory qualify, and only when the entry exists but doesn't launch the
/// running executable.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn self_heal_target(
    default_handler: &str,
    status: &HandlerStatus,
) -> Option<&'static str> {
    if !matches!(status, HandlerStatus::Stale | HandlerStatus::Outdated) {
        return None;
    }
    OWN_DESKTOP_FILES
        .iter()
        .copied()
        .find(|own| *own == default_handler.trim())
}

/// Value of the `Exec` key in the `[Desktop Entry]` group, if present.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn desktop_entry_exec(content: &str) -> Option<&str> {
    let mut in_main_group = false;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_main_group = line == "[Desktop Entry]";
            continue;
        }
        if !in_main_group {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            if key.trim() == "Exec" {
                return Some(value.trim_start());
            }
        }
    }
    None
}

/// Split a desktop-entry `Exec=` value into arguments, undoing the escaping
/// [`quote_exec_arg`] applies: first the string-value escapes (`\s` `\n`
/// `\t` `\r` `\\`), then Exec quoting (`"..."` with `\"` `` \` `` `\$` `\\`
/// inside), then `%%` → `%`. Field codes such as `%u` are kept verbatim.
/// Returns `None` for an unterminated quote.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn parse_exec_args(value: &str) -> Option<Vec<String>> {
    let mut unescaped = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            unescaped.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => unescaped.push(' '),
            Some('n') => unescaped.push('\n'),
            Some('t') => unescaped.push('\t'),
            Some('r') => unescaped.push('\r'),
            Some('\\') => unescaped.push('\\'),
            Some(other) => {
                unescaped.push('\\');
                unescaped.push(other);
            }
            None => unescaped.push('\\'),
        }
    }

    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_arg = false;
    let mut in_quotes = false;
    let mut chars = unescaped.chars();
    while let Some(c) = chars.next() {
        if in_quotes {
            match c {
                '"' => in_quotes = false,
                '\\' => match chars.next() {
                    Some(e @ ('"' | '`' | '$' | '\\')) => current.push(e),
                    Some(other) => {
                        current.push('\\');
                        current.push(other);
                    }
                    None => return None,
                },
                c => current.push(c),
            }
        } else {
            match c {
                ' ' | '\t' => {
                    if in_arg {
                        args.push(std::mem::take(&mut current));
                        in_arg = false;
                    }
                }
                '"' => {
                    in_quotes = true;
                    in_arg = true;
                }
                c => {
                    current.push(c);
                    in_arg = true;
                }
            }
        }
    }
    if in_quotes {
        return None;
    }
    if in_arg {
        args.push(current);
    }
    Some(args.into_iter().map(|a| a.replace("%%", "%")).collect())
}

/// The program an `Exec=` line launches, looking through a leading
/// `env NAME=VALUE ...` wrapper (which `create_desktop_entry` uses on NVIDIA
/// systems).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn exec_program(args: &[String]) -> Option<&str> {
    let mut iter = args.iter().map(String::as_str);
    let first = iter.next()?;
    if first != "env" && !first.ends_with("/env") {
        return Some(first);
    }
    iter.find(|arg| !is_env_assignment(arg))
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn is_env_assignment(arg: &str) -> bool {
    match arg.split_once('=') {
        Some((name, _)) => !name.is_empty() && !name.contains('/'),
        None => false,
    }
}

/// Query xdg-mime for the nxm:// default and classify it against the running
/// executable. `None` when xdg-mime isn't available or reports nothing.
#[cfg(target_os = "linux")]
fn current_handler_status() -> Option<(String, HandlerStatus)> {
    let output = Command::new("xdg-mime")
        .args(["query", "default", "x-scheme-handler/nxm"])
        .output()
        .ok()?;
    let handler = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if handler.is_empty() {
        return None;
    }
    if !is_corkscrew_desktop_id(&handler) {
        return Some((handler, HandlerStatus::NotCorkscrew));
    }
    let content = find_desktop_file(&handler).and_then(|p| std::fs::read_to_string(p).ok());
    let current_exe = crate::steam_integration::get_exe_path().unwrap_or_default();
    let status = classify_handler(
        &handler,
        content.as_deref(),
        &current_exe,
        program_exists,
        same_program,
    );
    Some((handler, status))
}

/// Locate a desktop file by ID in the XDG application directories, user
/// directory first (matching how xdg-mime resolves it).
#[cfg(target_os = "linux")]
fn find_desktop_file(desktop_id: &str) -> Option<PathBuf> {
    // A desktop ID is a bare file name; refuse anything path-like.
    if desktop_id.contains('/') || desktop_id.contains('\0') || desktop_id.starts_with('.') {
        return None;
    }
    let mut data_dirs: Vec<PathBuf> = Vec::new();
    match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(dir) => data_dirs.push(PathBuf::from(dir)),
        None => {
            if let Some(home) = dirs::home_dir() {
                data_dirs.push(home.join(".local/share"));
            }
        }
    }
    let system_dirs = std::env::var_os("XDG_DATA_DIRS")
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    data_dirs.extend(std::env::split_paths(&system_dirs));

    data_dirs
        .into_iter()
        .map(|dir| dir.join("applications").join(desktop_id))
        .find(|p| p.is_file())
}

/// Whether an `Exec=` program can be launched: an absolute/relative path
/// must be an executable file; a bare name must be on `$PATH`.
#[cfg(target_os = "linux")]
fn program_exists(program: &str) -> bool {
    if program.contains('/') {
        is_executable_file(std::path::Path::new(program))
    } else {
        binary_on_path(program)
    }
}

/// Compare two executable paths, resolving symlinks when both exist.
#[cfg(target_os = "linux")]
fn same_program(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => false,
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

/// Percent-decoding for URL path segments. Delegates to the shared decoder
/// in `oauth.rs`, which accumulates raw bytes and converts via UTF-8 —
/// the previous byte-as-char version decoded multi-byte UTF-8 (e.g. `%C3%A9`)
/// as Latin-1 mojibake, breaking non-ASCII bottle names.
fn urlencoding_decode(s: &str) -> String {
    crate::oauth::percent_decode(s)
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
    Fallback { active_game_id: String, warning: String },
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
            return NxmRoute::Recognized { game_id: gid.clone() };
        }
    }
    // 2. Match by game_id (some custom games use the slug as the id).
    for (gid, _slug) in known_games {
        if gid.eq_ignore_ascii_case(&domain_lower) {
            return NxmRoute::Recognized { game_id: gid.clone() };
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

    fn exec_line(content: &str) -> &str {
        content
            .lines()
            .find_map(|l| l.strip_prefix("Exec="))
            .expect("Exec= line")
    }

    #[test]
    fn desktop_exec_passes_url_as_only_argument() {
        // tauri-plugin-deep-link on Linux only accepts argv == [bin, url].
        let content = desktop_file_content("/usr/bin/corkscrew").unwrap();
        assert_eq!(exec_line(&content), "\"/usr/bin/corkscrew\" %u");
        assert!(!content.contains("--url"));
        assert!(content.contains("MimeType=x-scheme-handler/nxm;x-scheme-handler/corkscrew;"));
    }

    #[test]
    fn desktop_exec_uses_appimage_path_with_spaces() {
        let content =
            desktop_file_content("/home/deck/Applications/Corkscrew 0.14.AppImage").unwrap();
        assert_eq!(
            exec_line(&content),
            "\"/home/deck/Applications/Corkscrew 0.14.AppImage\" %u"
        );
    }

    #[test]
    fn desktop_exec_escapes_reserved_characters() {
        assert_eq!(quote_exec_arg("/a/100%/b").unwrap(), "\"/a/100%%/b\"");
        assert_eq!(quote_exec_arg("/a/$x`y\"z").unwrap(), r#""/a/\\$x\\`y\\"z""#);
        assert_eq!(quote_exec_arg(r"/a\b").unwrap(), r#""/a\\\\b""#);
        assert!(quote_exec_arg("/a\nExec=evil").is_err());
    }

    #[test]
    fn exec_args_round_trip_quote_exec_arg() {
        for path in [
            "/usr/bin/corkscrew",
            "/home/deck/Applications/Corkscrew 0.14.AppImage",
            "/a/100%/b",
            "/a/$x`y\"z",
            r"/a\b\\c",
            "/tmp/ünïcode path/corkscrew",
        ] {
            let exec = format!("{} %u", quote_exec_arg(path).unwrap());
            assert_eq!(
                parse_exec_args(&exec).unwrap(),
                vec![path.to_string(), "%u".to_string()],
                "round trip failed for {path:?} via {exec:?}"
            );
        }
    }

    #[test]
    fn exec_args_handle_unquoted_and_env_prefix() {
        let args = parse_exec_args("env WEBKIT_DISABLE_DMABUF_RENDERER=1 /opt/corkscrew %u").unwrap();
        assert_eq!(exec_program(&args), Some("/opt/corkscrew"));

        let quoted = format!(
            "env WEBKIT_DISABLE_DMABUF_RENDERER=1 {} %u",
            quote_exec_arg("/home/u/My Apps/Corkscrew.AppImage").unwrap()
        );
        let args = parse_exec_args(&quoted).unwrap();
        assert_eq!(exec_program(&args), Some("/home/u/My Apps/Corkscrew.AppImage"));

        let args = parse_exec_args("corkscrew --url %u").unwrap();
        assert_eq!(exec_program(&args), Some("corkscrew"));

        assert!(parse_exec_args("\"/unterminated %u").is_none());
        assert_eq!(exec_program(&[]), None);
    }

    #[test]
    fn desktop_entry_exec_reads_main_group_only() {
        let content = "[Desktop Action foo]\nExec=/wrong\n[Desktop Entry]\nName=X\nExec = \"/right\" %u\n";
        assert_eq!(desktop_entry_exec(content), Some("\"/right\" %u"));
        assert_eq!(desktop_entry_exec("[Desktop Entry]\nName=X\n"), None);
    }

    fn classify(handler: &str, content: Option<&str>, existing: &[&str]) -> HandlerStatus {
        classify_handler(
            handler,
            content,
            "/home/deck/Corkscrew.AppImage",
            |p| existing.contains(&p),
            |a, b| a == b,
        )
    }

    #[test]
    fn classify_handler_cases() {
        let current = desktop_file_content("/home/deck/Corkscrew.AppImage").unwrap();
        let moved = desktop_file_content("/home/deck/Downloads/Corkscrew-0.14.1.AppImage").unwrap();
        let existing = ["/home/deck/Corkscrew.AppImage"];

        // Someone else's handler (or none).
        assert_eq!(classify("org.nexusmods.app.desktop", Some(&current), &existing), HandlerStatus::NotCorkscrew);
        assert_eq!(classify("", None, &existing), HandlerStatus::NotCorkscrew);

        // Ours by name, but the file is gone (user unregistered).
        assert_eq!(classify("corkscrew-nxm.desktop", None, &existing), HandlerStatus::Missing);

        // Points at the running executable — via either of our files, and a
        // case-variant packaged name.
        assert_eq!(classify("corkscrew-nxm.desktop", Some(&current), &existing), HandlerStatus::Current);
        assert_eq!(classify("corkscrew.desktop", Some(&current), &existing), HandlerStatus::Current);
        assert_eq!(classify("Corkscrew.desktop", Some(&current), &existing), HandlerStatus::Current);

        // AppImage moved/renamed: Exec target no longer exists.
        assert_eq!(classify("corkscrew-nxm.desktop", Some(&moved), &existing), HandlerStatus::Stale);

        // Exec target exists but isn't the running binary.
        let both = ["/home/deck/Corkscrew.AppImage", "/home/deck/Downloads/Corkscrew-0.14.1.AppImage"];
        assert_eq!(classify("corkscrew.desktop", Some(&moved), &both), HandlerStatus::Outdated);

        // Unparseable / missing Exec.
        assert_eq!(classify("corkscrew.desktop", Some("[Desktop Entry]\nName=X\n"), &existing), HandlerStatus::Stale);

        assert!(HandlerStatus::Current.launches_corkscrew());
        assert!(HandlerStatus::Outdated.launches_corkscrew());
        assert!(!HandlerStatus::Stale.launches_corkscrew());
        assert!(!HandlerStatus::Missing.launches_corkscrew());
        assert!(!HandlerStatus::NotCorkscrew.launches_corkscrew());
    }

    #[test]
    fn self_heal_only_rewrites_own_stale_or_outdated_files() {
        assert_eq!(self_heal_target("corkscrew-nxm.desktop", &HandlerStatus::Stale), Some("corkscrew-nxm.desktop"));
        assert_eq!(self_heal_target("corkscrew.desktop", &HandlerStatus::Outdated), Some("corkscrew.desktop"));
        // Already correct, deliberately removed, or not ours: leave alone.
        assert_eq!(self_heal_target("corkscrew-nxm.desktop", &HandlerStatus::Current), None);
        assert_eq!(self_heal_target("corkscrew-nxm.desktop", &HandlerStatus::Missing), None);
        assert_eq!(self_heal_target("other.desktop", &HandlerStatus::NotCorkscrew), None);
        // A packaged Corkscrew.desktop isn't a file we write — don't touch it.
        assert_eq!(self_heal_target("Corkscrew.desktop", &HandlerStatus::Stale), None);
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
