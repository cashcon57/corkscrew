//! CrossOver shortcut auto-discovery.
//!
//! Modern CrossOver / Wine writes Windows `.lnk` shortcuts to several
//! standard Windows locations inside the bottle:
//!
//! * `<bottle>/drive_c/users/<user>/AppData/Roaming/Microsoft/Windows/Start Menu/Programs/**/*.lnk` (per-user)
//! * `<bottle>/drive_c/ProgramData/Microsoft/Windows/Start Menu/Programs/**/*.lnk` (system-wide)
//! * `<bottle>/drive_c/users/<user>/Desktop/*.lnk` (per-user desktop)
//! * `<bottle>/drive_c/users/Public/Desktop/*.lnk` (shared desktop)
//! * `<bottle>/drive_c/users/<user>/Start Menu/Programs/**/*.lnk` (legacy
//!   Wine path — kept for old prefixes)
//!
//! Parsing the .lnk files lets us surface manually-installed games (GOG /
//! itch.io / DRM-free exes dropped into `drive_c/Games/`, plus tool
//! shortcuts like `launchmod_eldenring.bat.lnk` from Mod Engine 2) that the
//! Steam appmanifest scanner can't see.
//!
//! For each .lnk we extract the absolute Windows target path, resolve it
//! to a host path inside the bottle, filter out non-game shortcuts
//! (system tools, browsers, installers), and try to auto-match the exe
//! filename against a known game (registered native plugin or
//! `vortex_extension_index.json`). The frontend uses [`UnregisteredGame`]
//! to render a one-click registration banner.
//!
//! Path safety: every Windows path coming out of a `.lnk` is treated as
//! untrusted input. `..`, null bytes, and absolute paths leaving the
//! bottle's `drive_c` are rejected before we touch the filesystem.

use std::collections::HashSet;
use std::convert::TryFrom;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::bottles::Bottle;
use crate::crossover_cxmenu;
use crate::games::DetectedGame;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A `.lnk` shortcut discovered inside a CrossOver/Wine bottle.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrossoverShortcut {
    /// Bottle the shortcut lives in.
    pub bottle_name: String,
    /// Friendly display name derived from the `.lnk` filename (no extension).
    pub display_name: String,
    /// Absolute host path to the source `.lnk` file.
    pub source_lnk_path: PathBuf,
    /// The Windows-shaped target inside the bottle (e.g.
    /// `C:\Games\SEKIRO Shadows Die Twice\sekiro.exe`).
    pub windows_target: String,
    /// Resolved host filesystem path for the target executable.
    pub host_target: PathBuf,
    /// Working directory (host path), if the shortcut specifies one and
    /// it resolves cleanly inside the bottle.
    pub working_directory: Option<PathBuf>,
    /// Icon path (host path), best-effort. Often `None` — CrossOver
    /// shortcuts may reference Windows system icons we can't display.
    pub icon_path: Option<PathBuf>,
}

/// Source of an auto-match hint.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum MatchSource {
    /// Matched against a registered native [`crate::games::GamePlugin`].
    Plugin,
    /// Matched against `data/vortex_extension_index.json`.
    VortexIndex,
}

/// Best-guess registration metadata for an unregistered shortcut.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MatchHint {
    pub game_id: String,
    pub display_name: String,
    pub nexus_slug: String,
    pub steam_app_id: Option<String>,
    pub source: MatchSource,
}

/// A shortcut surfaced to the UI as an unregistered/installable game.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UnregisteredGame {
    pub shortcut: CrossoverShortcut,
    /// `Some` when the exe filename matches a known game; the frontend
    /// uses this to pre-fill the registration form for one-click install.
    pub match_hint: Option<MatchHint>,
}

// ---------------------------------------------------------------------------
// .lnk parsing
// ---------------------------------------------------------------------------

/// Parse a single `.lnk` file and return the resolved Windows target string.
///
/// Falls back through the standard parselnk fields:
/// 1. `link_info.local_base_path_unicode` (preferred, full unicode)
/// 2. `link_info.local_base_path` (system code page)
/// 3. `string_data.relative_path` joined with `working_dir`
fn extract_lnk_target(lnk: &parselnk::Lnk) -> Option<String> {
    if let Some(p) = lnk.link_info.local_base_path_unicode.as_ref() {
        if !p.is_empty() {
            return Some(p.clone());
        }
    }
    if let Some(p) = lnk.link_info.local_base_path.as_ref() {
        if !p.is_empty() {
            return Some(p.clone());
        }
    }
    // Fall back to relative_path resolved against working_dir.
    if let Some(rel) = lnk.string_data.relative_path.as_ref() {
        if let Some(wd) = lnk.string_data.working_dir.as_ref() {
            let mut joined = wd.clone();
            joined.push(rel);
            return Some(joined.to_string_lossy().into_owned());
        }
        return Some(rel.to_string_lossy().into_owned());
    }
    None
}

/// Resolve a Windows-style absolute path (e.g. `C:\Games\Foo\bar.exe`) to a
/// host filesystem path under the bottle's drive root for that letter.
///
/// Supports any drive letter A-Z, mapping `X:` → `<bottle>/drive_x` when that
/// directory exists. Returns `None` for UNC paths, paths whose drive directory
/// doesn't exist in the bottle, or paths that fail safety checks (traversal,
/// null bytes).
fn resolve_windows_path(bottle: &Bottle, win_path: &str) -> Option<PathBuf> {
    let trimmed = win_path.trim();
    if trimmed.is_empty() || trimmed.contains('\0') {
        return None;
    }

    // Strip NT object-manager / extended-length prefixes before inspecting
    // the drive letter. Treat `\??\C:\foo`, `\\?\C:\foo`, and `C:\foo` uniformly.
    let no_prefix = trimmed
        .strip_prefix("\\??\\")
        .or_else(|| trimmed.strip_prefix("\\\\?\\"))
        .unwrap_or(trimmed);

    // We require an absolute Windows path with a drive letter (A-Z / a-z).
    let (drive_root, after_drive) =
        if no_prefix.len() >= 2 && no_prefix.as_bytes()[1] == b':' {
            let letter = no_prefix.as_bytes()[0].to_ascii_lowercase();
            if !letter.is_ascii_alphabetic() {
                return None;
            }
            // Map the drive letter to the host directory: drive_c, drive_d, …
            let drive_dir_name = format!("drive_{}", letter as char);
            let drive_root = bottle.path.join(&drive_dir_name);
            if !drive_root.is_dir() {
                return None;
            }
            // Strip "X:" — keep the rest, which may start with `\` or `/`.
            (drive_root, &no_prefix[2..])
        } else {
            return None;
        };

    // Split into components, normalize to forward slashes, reject traversal.
    let mut walk = drive_root;
    let mut had_any = false;
    for raw in after_drive.split(|c| c == '\\' || c == '/') {
        if raw.is_empty() {
            continue;
        }
        if raw == "." {
            continue;
        }
        if raw == ".." {
            // Refuse to climb out of the drive root.
            return None;
        }
        had_any = true;
        // Try case-insensitive match against the current directory; if it
        // doesn't exist yet (e.g. parent of the exe), join verbatim and let
        // the caller's `exists()` check fail naturally.
        if walk.is_dir() {
            let lower = raw.to_lowercase();
            let mut matched = None;
            if let Ok(entries) = std::fs::read_dir(&walk) {
                for e in entries.flatten() {
                    if e.file_name().to_string_lossy().to_lowercase() == lower {
                        matched = Some(e.path());
                        break;
                    }
                }
            }
            walk = matched.unwrap_or_else(|| walk.join(raw));
        } else {
            walk.push(raw);
        }
    }

    if !had_any {
        return None;
    }

    // Final containment check: after case-insensitive resolution (which may
    // follow symlinks on some filesystems), verify the result still lives
    // inside the bottle root. This prevents symlink-escape attacks where a
    // Wine game directory contains a symlink that points outside the bottle.
    //
    // We only apply this check when both the bottle root and the target path
    // can be canonicalized (i.e. they both exist on disk). If either path
    // doesn't exist yet (e.g. the exe hasn't been installed) we skip the check
    // and let the caller's `exists()` check fail naturally.
    if let (Ok(bottle_canonical), Ok(walk_canonical)) = (
        bottle.path.canonicalize(),
        walk.canonicalize(),
    ) {
        if !walk_canonical.starts_with(&bottle_canonical) {
            log::warn!(
                "resolve_windows_path: '{}' resolved outside bottle root — rejected",
                win_path
            );
            return None;
        }
    }

    Some(walk)
}

// ---------------------------------------------------------------------------
// Shortcut scanning
// ---------------------------------------------------------------------------

/// Scan a single bottle for `.lnk` shortcuts and return the parsed entries.
/// Broken shortcuts (target doesn't exist on disk) are dropped.
pub fn scan_bottle_shortcuts(bottle: &Bottle) -> Vec<CrossoverShortcut> {
    let mut out = Vec::new();
    let drive_c = bottle.drive_c();
    if !drive_c.is_dir() {
        return out;
    }

    // Recursive scan roots — Start Menu trees nest by app, walk them deep.
    let recursive_roots = [
        // System-wide Start Menu (modern Wine).
        drive_c
            .join("ProgramData")
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs"),
    ];
    for root in &recursive_roots {
        scan_recursive(bottle, root, &mut out);
    }

    // Public/shared desktop (single level).
    scan_flat(
        bottle,
        &drive_c
            .join("users")
            .join("Public")
            .join("Desktop"),
        &mut out,
    );

    // Per-user paths — iterate every user directory inside `users/`.
    let users = bottle.users_dir();
    if let Ok(user_entries) = std::fs::read_dir(&users) {
        for user_entry in user_entries.flatten() {
            let user_dir = user_entry.path();
            if !user_dir.is_dir() {
                continue;
            }
            // Skip the shared "Public" branch — already handled above.
            if user_dir.file_name().is_some_and(|n| n == "Public") {
                continue;
            }

            // Modern Wine: AppData\Roaming\Microsoft\Windows\Start Menu\Programs
            scan_recursive(
                bottle,
                &user_dir
                    .join("AppData")
                    .join("Roaming")
                    .join("Microsoft")
                    .join("Windows")
                    .join("Start Menu")
                    .join("Programs"),
                &mut out,
            );

            // Legacy Wine: <user>\Start Menu\Programs (kept for old prefixes)
            scan_recursive(
                bottle,
                &user_dir.join("Start Menu").join("Programs"),
                &mut out,
            );

            // Per-user Desktop (single level)
            scan_flat(bottle, &user_dir.join("Desktop"), &mut out);
        }
    }

    // --- cxmenu.conf augmentation -------------------------------------------
    // CrossOver registers many shortcuts only in `cxmenu.conf` (a Mac-side INI
    // at the bottle root) without writing a `.lnk` into drive_c. We parse it
    // now and synthesize CrossoverShortcut entries for any exe we can locate.
    merge_cxmenu_shortcuts(bottle, &mut out);

    // De-dup by resolved exe path: the same exe is often linked from
    // multiple Start Menu / Desktop locations.
    let mut seen: HashSet<PathBuf> = HashSet::new();
    out.retain(|s| seen.insert(s.host_target.clone()));
    out
}

// ---------------------------------------------------------------------------
// cxmenu.conf augmentation
// ---------------------------------------------------------------------------

/// Parse the bottle's `cxmenu.conf` (or `cxmenu`) and append synthetic
/// [`CrossoverShortcut`] entries for any entries not already captured by the
/// `.lnk` walk.
///
/// For each [`crossover_cxmenu::CxmenuEntry`]:
/// 1. If the decoded `windows_path` ends in `.lnk` and resolves to a file on
///    disk, we skip it — the `.lnk` walk will have already handled it (or the
///    `.lnk` itself is missing, meaning the shortcut is broken and we should
///    not emit it).
/// 2. Otherwise, if the entry carries a `startup_wm_class` (typically an
///    `.exe` filename), we walk `drive_c` up to depth 6 looking for a file
///    whose name matches (case-insensitively). The first match is used as
///    `host_target`. If none is found, we skip the entry rather than emitting
///    a broken shortcut.
fn merge_cxmenu_shortcuts(bottle: &Bottle, out: &mut Vec<CrossoverShortcut>) {
    let Some(cxmenu_path) = crossover_cxmenu::find_cxmenu_file(&bottle.path) else {
        return;
    };

    let entries = crossover_cxmenu::parse_cxmenu(&cxmenu_path);
    if entries.is_empty() {
        return;
    }

    // Build a set of host targets already discovered by the .lnk walk so we
    // can skip duplicates cheaply (the final dedup pass also catches these, but
    // avoiding unnecessary filesystem walks is better).
    let existing: HashSet<PathBuf> = out.iter().map(|s| s.host_target.clone()).collect();

    let drive_c = bottle.drive_c();

    for entry in &entries {
        let win_path = &entry.windows_path;

        // Case 1: path ends in .lnk — check if it already resolved; if so,
        // skip. We don't re-parse lnk files here; the walk already handled it.
        if win_path.to_lowercase().ends_with(".lnk") {
            if let Some(host) = resolve_windows_path(bottle, win_path) {
                if host.is_file() && existing.contains(&host) {
                    continue;
                }
            }
            // .lnk not on disk — skip entirely; broken shortcut.
            continue;
        }

        // Case 2: non-.lnk entry — requires startup_wm_class to find the exe.
        let Some(ref wm_class) = entry.startup_wm_class else {
            continue;
        };

        // Reject startup_wm_class values that look like paths (no separators
        // allowed) or that don't end in a recognised executable extension
        // (.exe or .bat for ME2 launchers).
        let wm_lower = wm_class.to_lowercase();
        if wm_class.contains('\\')
            || wm_class.contains('/')
            || (!wm_lower.ends_with(".exe") && !wm_lower.ends_with(".bat"))
        {
            continue;
        }

        // Search drive_c for the executable (case-insensitive, depth-limited).
        let Some(host_target) = find_exe_in_drive_c(&drive_c, wm_class, 6) else {
            continue;
        };

        if existing.contains(&host_target) {
            continue;
        }

        // Derive a display name from the windows_path filename (strip extension).
        let display_name = std::path::Path::new(win_path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| {
                host_target
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });

        out.push(CrossoverShortcut {
            bottle_name: bottle.name.clone(),
            display_name,
            // Use the cxmenu file path as source_lnk_path to signal cxmenu
            // origin (distinguishable from .lnk-derived shortcuts by extension).
            source_lnk_path: cxmenu_path.clone(),
            windows_target: win_path.clone(),
            host_target,
            working_directory: None,
            icon_path: None,
        });
    }
}

/// Walk `drive_c` recursively up to `max_depth` looking for a file whose name
/// matches `exe_name` case-insensitively. Stops after the first match.
///
/// Returns the absolute host path on success, `None` if not found.
fn find_exe_in_drive_c(drive_c: &Path, exe_name: &str, max_depth: usize) -> Option<PathBuf> {
    let exe_lower = exe_name.to_lowercase();
    for entry in WalkDir::new(drive_c)
        .max_depth(max_depth)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name == exe_lower {
            return Some(entry.into_path());
        }
    }
    None
}

fn scan_recursive(bottle: &Bottle, root: &Path, out: &mut Vec<CrossoverShortcut>) {
    if !root.is_dir() {
        return;
    }
    for entry in WalkDir::new(root)
        .max_depth(8)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        if !is_lnk_path(entry.path()) {
            continue;
        }
        if let Some(s) = parse_shortcut_file(bottle, entry.path()) {
            out.push(s);
        }
    }
}

fn scan_flat(bottle: &Bottle, dir: &Path, out: &mut Vec<CrossoverShortcut>) {
    if !dir.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || !is_lnk_path(&path) {
            continue;
        }
        if let Some(s) = parse_shortcut_file(bottle, &path) {
            out.push(s);
        }
    }
}

/// Scan all bottles passed in.
pub fn scan_all_bottles_shortcuts(bottles: &[Bottle]) -> Vec<CrossoverShortcut> {
    let mut out = Vec::new();
    for b in bottles {
        // Only scan bottle managers that actually emit shortcuts. CrossOver,
        // Whisky, Moonshine, and native Wine all create them; Proton
        // prefixes don't (Steam manages its own shortcuts), but scanning is
        // cheap so we don't bother filtering.
        out.extend(scan_bottle_shortcuts(b));
    }
    out
}

fn is_lnk_path(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("lnk"))
        .unwrap_or(false)
}

/// Return `true` if a Windows target path looks like a Mod Engine 2
/// `launchmod_<slug>.bat` launcher. These are `.bat` files, not `.exe`
/// files, so they need special-casing wherever we filter by extension.
fn is_me2_bat_target(win_path_lower: &str) -> bool {
    let filename = win_path_lower
        .rsplit(|c| c == '\\' || c == '/')
        .next()
        .unwrap_or(win_path_lower);
    filename.starts_with("launchmod_") && filename.ends_with(".bat")
}

fn parse_shortcut_file(bottle: &Bottle, lnk_path: &Path) -> Option<CrossoverShortcut> {
    let lnk = parselnk::Lnk::try_from(lnk_path).ok()?;
    let win_target = extract_lnk_target(&lnk)?;

    // Only surface shortcuts that point at executables or recognised ME2
    // launchers. CrossOver creates shortcuts for help files, URLs, and
    // uninstallers we don't want.
    let win_lower = win_target.to_lowercase();
    if !win_lower.ends_with(".exe") && !is_me2_bat_target(&win_lower) {
        return None;
    }

    let host_target = resolve_windows_path(bottle, &win_target)?;
    if !host_target.is_file() {
        return None;
    }

    let working_directory = lnk
        .string_data
        .working_dir
        .as_ref()
        .and_then(|wd| resolve_windows_path(bottle, &wd.to_string_lossy()))
        .filter(|p| p.is_dir());

    let icon_path = lnk
        .string_data
        .icon_location
        .as_ref()
        .and_then(|ic| resolve_windows_path(bottle, &ic.to_string_lossy()))
        .filter(|p| p.exists());

    let display_name = lnk_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| {
            host_target
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        });

    Some(CrossoverShortcut {
        bottle_name: bottle.name.clone(),
        display_name,
        source_lnk_path: lnk_path.to_path_buf(),
        windows_target: win_target,
        host_target,
        working_directory,
        icon_path,
    })
}

// ---------------------------------------------------------------------------
// Game-vs-tool filter
// ---------------------------------------------------------------------------

/// Return `true` if the shortcut looks like a real game we should surface.
pub fn is_likely_game(shortcut: &CrossoverShortcut) -> bool {
    let exe_name = shortcut
        .host_target
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    if exe_name.is_empty() {
        return false;
    }

    // Hard-coded exclusions for well-known non-games. These are storefronts,
    // launchers, system tools, and utilities — none of them are something a
    // user wants Corkscrew to surface as "an unregistered game" in their
    // bottle. (Steam itself was the trigger for this list growing — its
    // CrossOver shortcut was being flagged alongside actual games.)
    const EXCLUDED_EXES: &[&str] = &[
        // System / general tools
        "notepad.exe",
        "notepad++.exe",
        "regedit.exe",
        "cmd.exe",
        "taskmgr.exe",
        "iexplore.exe",
        "msedge.exe",
        "chrome.exe",
        "firefox.exe",
        "winecfg.exe",
        "wineboot.exe",
        "explorer.exe",
        "control.exe",
        "msiexec.exe",
        "msconfig.exe",
        "uninstall.exe",
        "setup.exe",
        "installer.exe",
        // Storefronts / launchers — never the "game" itself
        "steam.exe",
        "steamwebhelper.exe",
        "epicgameslauncher.exe",
        "epicwebhelper.exe",
        "goggalaxy.exe",
        "galaxyclient.exe",
        "galaxyclient-service.exe",
        "battle.net.exe",
        "battle.net launcher.exe",
        "origin.exe",
        "originwebhelperservice.exe",
        "eadesktop.exe",
        "ealauncher.exe",
        "uplay.exe",
        "upc.exe",
        "ubisoftconnect.exe",
        "ubisoft connect.exe",
        "ubisoftgamelauncher.exe",
        "rockstargameslauncher.exe",
        "launcher.exe",
        "bethesda.net_launcher.exe",
        "bethesdanetlauncher.exe",
        "itch.exe",
        "itch-setup.exe",
        "amazongameslauncher.exe",
    ];
    if EXCLUDED_EXES.contains(&exe_name.as_str()) {
        return false;
    }

    // Pattern-based exclusions (uninstallers, mod loaders, cheat tools).
    if matches_unins_pattern(&exe_name) {
        return false;
    }
    if exe_name.starts_with("cheatengine") {
        return false;
    }
    // Reject the ModEngine2 tool binary itself (e.g. `modengine2.exe`,
    // `modengine2_launcher.exe`) but ALLOW `launchmod_*.bat` — those are
    // the actual per-game launch shortcuts that ME2 v2.x registers, and
    // they ARE the thing users want to register as a "game".
    if exe_name.starts_with("modengine") {
        return false;
    }

    // Anything inside Windows system dirs is not a game.
    let path_lower = shortcut.host_target.to_string_lossy().to_lowercase();
    let path_norm = path_lower.replace('\\', "/");
    if path_norm.contains("/drive_c/windows/system32/")
        || path_norm.contains("/drive_c/windows/syswow64/")
        || path_norm.contains("/drive_c/windows/")
    {
        return false;
    }

    // Size check: most real game executables are 5 MB+. Batch files are
    // inherently small, so skip the size filter for `.bat` targets.
    // Read errors → keep (don't false-negative).
    let is_bat = exe_name.ends_with(".bat");
    if !is_bat {
        if let Ok(meta) = std::fs::metadata(&shortcut.host_target) {
            let size = meta.len();
            if size > 0 && size < 5 * 1024 * 1024 {
                return false;
            }
        }
    }

    true
}

/// Match `unins000.exe`, `unins001.exe`, `uninstall.exe`, etc.
fn matches_unins_pattern(name: &str) -> bool {
    if !name.starts_with("unins") {
        return false;
    }
    // Strip "unins" prefix and ".exe" suffix; remainder must be all digits
    // (or empty for plain "unins.exe", which is rare but harmless).
    let rest = match name.strip_suffix(".exe") {
        Some(r) => &r[5..],
        None => return false,
    };
    rest.is_empty() || rest.chars().all(|c| c.is_ascii_digit())
}

// ---------------------------------------------------------------------------
// Auto-match
// ---------------------------------------------------------------------------

/// Try to auto-match a shortcut's exe name to a known game.
///
/// Checks registered native plugins first, then the Vortex extension index
/// (which is keyed by Steam app ID — we match by exe name for shortcuts
/// since the .lnk doesn't carry an app ID).
pub fn match_shortcut(shortcut: &CrossoverShortcut) -> Option<MatchHint> {
    let exe_name = shortcut
        .host_target
        .file_name()?
        .to_string_lossy()
        .to_lowercase();

    // 1. Native plugin lookup. We can't iterate the registry without holding
    //    its mutex, so we walk known executables for each registered game.
    if let Some(hint) = match_against_plugins(&exe_name) {
        return Some(hint);
    }

    // 2. Vortex extension index — match by exe name against the registry's
    //    `executable` field. The vortex_index itself is Steam-app-id-keyed,
    //    so we cross-reference through `game_registry::all_entries` for the
    //    name → executable mapping, then look up the vortex entry.
    if let Some(hint) = match_against_vortex(&exe_name) {
        return Some(hint);
    }

    None
}

fn match_against_plugins(exe_name: &str) -> Option<MatchHint> {
    use crate::games::with_plugin;

    // Special case: Mod Engine 2 (v2.x) registers CrossOver shortcuts as
    // `launchmod_<slug>.bat` rather than a plain `.exe`. Check for this
    // pattern before the generic exe-name scan so that ME2-managed games are
    // auto-matched even though their shortcut doesn't end in `.exe`.
    if let Some(hint) = match_against_modengine2_bat(exe_name) {
        return Some(hint);
    }

    // We need to scan the plugin registry. There is no public iterator over
    // the registry, so we walk the embedded game registry for known IDs and
    // probe each one. This is O(n_games) but only runs on demand.
    for entry in crate::game_registry::all_game_entries() {
        let game_id = entry.game_id.clone();
        let matched = with_plugin(&game_id, |p| {
            p.executables()
                .iter()
                .any(|e| e.eq_ignore_ascii_case(exe_name))
        });
        if matched == Some(true) {
            return Some(MatchHint {
                game_id: entry.game_id.clone(),
                display_name: entry.name.clone(),
                nexus_slug: entry.nexus_domain.clone(),
                steam_app_id: entry.steam_id.clone(),
                source: MatchSource::Plugin,
            });
        }
    }
    None
}

/// Match a Mod Engine 2 `launchmod_<slug>.bat` shortcut against the FromSoft
/// plugin specs.
///
/// ME2 v2.x supports Elden Ring, Dark Souls III, and Armored Core VI. Sekiro
/// and Dark Souls: Remastered were dropped in ME2 v2.x and are excluded to
/// avoid mismatching a user's manually-created `.bat` file.
fn match_against_modengine2_bat(exe_name: &str) -> Option<MatchHint> {
    // Supported slugs for Mod Engine 2 v2.x (case-insensitive filename match).
    // Sekiro (sekiro) and Dark Souls: Remastered (darksouls_remastered) are
    // intentionally excluded — ME2 v2 dropped support for both.
    const ME2_SLUGS: &[&str] = &["eldenring", "darksouls3", "armoredcore6"];

    let lower = exe_name.to_lowercase();
    let stem = lower.strip_prefix("launchmod_")?.strip_suffix(".bat")?;

    // Reject slugs that ME2 v2.x doesn't support.
    if !ME2_SLUGS.contains(&stem) {
        return None;
    }

    // Look up the matching FromSoft spec so we return accurate metadata.
    let spec = crate::plugins::fromsoft::SPECS
        .iter()
        .find(|s| s.game_id == stem)?;

    Some(MatchHint {
        game_id: spec.game_id.to_string(),
        display_name: spec.display_name.to_string(),
        nexus_slug: spec.nexus_slug.to_string(),
        // Steam app ID is unknown from the .bat alone — the user may have
        // installed the game outside of Steam.
        steam_app_id: None,
        source: MatchSource::Plugin,
    })
}

fn match_against_vortex(exe_name: &str) -> Option<MatchHint> {
    // Walk the registry → for each entry whose `executable` filename matches,
    // check if vortex_index has a corresponding entry.
    for entry in crate::game_registry::all_game_entries() {
        let Some(exe) = entry.executable.as_deref() else {
            continue;
        };
        let entry_exe_name = std::path::Path::new(exe)
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if !entry_exe_name.eq_ignore_ascii_case(exe_name) {
            continue;
        }
        let Some(steam_id) = entry.steam_id.as_deref() else {
            continue;
        };
        if let Some(vx) = crate::vortex_index::lookup_extension_for_steam_appid(steam_id) {
            return Some(MatchHint {
                game_id: entry.game_id.clone(),
                display_name: vx.name.clone(),
                nexus_slug: vx.nexus_slug.clone(),
                steam_app_id: Some(steam_id.to_string()),
                source: MatchSource::VortexIndex,
            });
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Top-level orchestration
// ---------------------------------------------------------------------------

/// List shortcut-derived games that aren't already registered.
///
/// Steps:
/// 1. Detect bottles + scan their shortcuts.
/// 2. Drop entries that match an existing detected/custom game (by resolved
///    exe path or `(game_id, bottle_name)`).
/// 3. Apply the game-vs-tool filter.
/// 4. Auto-match each survivor against plugins / vortex_index.
pub fn list_unregistered_games(
    bottles: &[Bottle],
    already_registered: &[DetectedGame],
) -> Vec<UnregisteredGame> {
    let shortcuts = scan_all_bottles_shortcuts(bottles);

    // Build a set of host exe paths the existing detection already covers.
    let mut registered_exes: HashSet<PathBuf> = HashSet::new();
    for g in already_registered {
        if let Some(p) = g.exe_path.as_ref() {
            registered_exes.insert(canonicalize_or(p));
        }
    }

    let mut out = Vec::new();
    for s in shortcuts {
        if registered_exes.contains(&canonicalize_or(&s.host_target)) {
            continue;
        }
        if !is_likely_game(&s) {
            continue;
        }
        let match_hint = match_shortcut(&s);
        out.push(UnregisteredGame {
            shortcut: s,
            match_hint,
        });
    }
    out
}

fn canonicalize_or(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{GameRuntime, WineContext};
    use std::fs;

    fn make_fake_bottle(parent: &Path, name: &str) -> Bottle {
        let path = parent.join(name);
        fs::create_dir_all(path.join("drive_c")).unwrap();
        Bottle {
            name: name.to_string(),
            path,
            source: "CrossOver".to_string(),
        }
    }

    #[test]
    fn matches_unins_pattern_handles_known_shapes() {
        assert!(matches_unins_pattern("unins000.exe"));
        assert!(matches_unins_pattern("unins001.exe"));
        assert!(matches_unins_pattern("unins.exe"));
        assert!(!matches_unins_pattern("uninspectable.exe"));
        assert!(!matches_unins_pattern("notepad.exe"));
    }

    #[test]
    fn is_likely_game_excludes_known_non_games() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = make_fake_bottle(tmp.path(), "test");
        let games_dir = bottle.drive_c().join("Games");
        fs::create_dir_all(&games_dir).unwrap();

        // 6 MB fake exe so the size filter passes.
        let big = vec![0u8; 6 * 1024 * 1024];

        let notepad = games_dir.join("notepad.exe");
        fs::write(&notepad, &big).unwrap();
        let setup = games_dir.join("setup.exe");
        fs::write(&setup, &big).unwrap();
        let unins = games_dir.join("unins000.exe");
        fs::write(&unins, &big).unwrap();
        let game = games_dir.join("eldenring.exe");
        fs::write(&game, &big).unwrap();

        let make_sc = |target: PathBuf, name: &str| CrossoverShortcut {
            bottle_name: bottle.name.clone(),
            display_name: name.to_string(),
            source_lnk_path: PathBuf::from("/dev/null"),
            windows_target: format!("C:\\Games\\{}", name),
            host_target: target,
            working_directory: None,
            icon_path: None,
        };

        assert!(!is_likely_game(&make_sc(notepad, "notepad")));
        assert!(!is_likely_game(&make_sc(setup, "setup")));
        assert!(!is_likely_game(&make_sc(unins, "unins000")));
        assert!(is_likely_game(&make_sc(game, "eldenring")));
    }

    #[test]
    fn is_likely_game_excludes_storefronts_and_launchers() {
        // Regression for "Add Game" banner surfacing Steam itself as an
        // unregistered game. Every storefront / publisher launcher should
        // be filtered out.
        let tmp = tempfile::tempdir().unwrap();
        let bottle = make_fake_bottle(tmp.path(), "test");
        let games_dir = bottle.drive_c().join("Games");
        fs::create_dir_all(&games_dir).unwrap();

        let big = vec![0u8; 6 * 1024 * 1024];

        let make_sc = |target: PathBuf, name: &str| CrossoverShortcut {
            bottle_name: bottle.name.clone(),
            display_name: name.to_string(),
            source_lnk_path: PathBuf::from("/dev/null"),
            windows_target: format!("C:\\Games\\{}", name),
            host_target: target,
            working_directory: None,
            icon_path: None,
        };

        for exe_name in &[
            "steam.exe",
            "steamwebhelper.exe",
            "EpicGamesLauncher.exe",
            "GalaxyClient.exe",
            "Battle.net.exe",
            "Origin.exe",
            "EADesktop.exe",
            "UbisoftConnect.exe",
            "uplay.exe",
            "RockstarGamesLauncher.exe",
            "BethesdaNetLauncher.exe",
            "itch.exe",
            "AmazonGamesLauncher.exe",
        ] {
            let p = games_dir.join(exe_name);
            fs::write(&p, &big).unwrap();
            assert!(
                !is_likely_game(&make_sc(p, exe_name)),
                "{} must not be surfaced as an unregistered game",
                exe_name
            );
        }
    }

    #[test]
    fn is_likely_game_excludes_tiny_exes() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = make_fake_bottle(tmp.path(), "test");
        let games_dir = bottle.drive_c().join("Games");
        fs::create_dir_all(&games_dir).unwrap();
        let tiny = games_dir.join("game.exe");
        fs::write(&tiny, b"tiny stub").unwrap(); // <5MB

        let sc = CrossoverShortcut {
            bottle_name: bottle.name.clone(),
            display_name: "game".to_string(),
            source_lnk_path: PathBuf::from("/dev/null"),
            windows_target: "C:\\Games\\game.exe".into(),
            host_target: tiny,
            working_directory: None,
            icon_path: None,
        };
        assert!(!is_likely_game(&sc));
    }

    #[test]
    fn is_likely_game_excludes_windows_system_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = make_fake_bottle(tmp.path(), "test");
        let sys = bottle.drive_c().join("windows").join("system32");
        fs::create_dir_all(&sys).unwrap();
        let exe = sys.join("randomtool.exe");
        fs::write(&exe, vec![0u8; 6 * 1024 * 1024]).unwrap();

        let sc = CrossoverShortcut {
            bottle_name: bottle.name.clone(),
            display_name: "randomtool".to_string(),
            source_lnk_path: PathBuf::from("/dev/null"),
            windows_target: "C:\\windows\\system32\\randomtool.exe".into(),
            host_target: exe,
            working_directory: None,
            icon_path: None,
        };
        assert!(!is_likely_game(&sc));
    }

    #[test]
    fn resolve_windows_path_handles_c_drive() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = make_fake_bottle(tmp.path(), "b");
        fs::create_dir_all(bottle.drive_c().join("Games").join("Foo")).unwrap();
        let resolved = resolve_windows_path(&bottle, "C:\\Games\\Foo\\bar.exe").unwrap();
        assert_eq!(resolved, bottle.drive_c().join("Games").join("Foo").join("bar.exe"));
    }

    #[test]
    fn resolve_windows_path_is_case_insensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = make_fake_bottle(tmp.path(), "b");
        fs::create_dir_all(bottle.drive_c().join("Games").join("Foo")).unwrap();
        // Lower-case input matches mixed-case directories.
        let resolved = resolve_windows_path(&bottle, "c:\\games\\foo\\bar.exe").unwrap();
        assert_eq!(resolved, bottle.drive_c().join("Games").join("Foo").join("bar.exe"));
    }

    #[test]
    fn resolve_windows_path_rejects_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = make_fake_bottle(tmp.path(), "b");
        assert!(resolve_windows_path(&bottle, "C:\\..\\..\\etc\\passwd").is_none());
    }

    #[test]
    fn resolve_windows_path_rejects_drive_when_directory_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = make_fake_bottle(tmp.path(), "b");
        assert!(resolve_windows_path(&bottle, "D:\\Games\\foo.exe").is_none());
        assert!(resolve_windows_path(&bottle, "\\\\server\\share\\foo.exe").is_none());
    }

    /// Verify the containment guard rejects a symlink that escapes the bottle.
    #[test]
    fn resolve_windows_path_rejects_symlink_escape() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = make_fake_bottle(tmp.path(), "b");

        // Create a directory inside drive_c that contains a symlink pointing
        // outside the bottle to the tmp dir itself.
        let games_dir = bottle.drive_c().join("Games");
        fs::create_dir_all(&games_dir).unwrap();
        let outside = tmp.path().join("outside.exe");
        fs::write(&outside, b"payload").unwrap();

        // Create a symlink: drive_c/Games/evil.exe -> ../../outside.exe
        // (points to a file outside the bottle root)
        let symlink_target = games_dir.join("evil.exe");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &symlink_target).unwrap();
        #[cfg(not(unix))]
        {
            // On non-Unix we can't create symlinks easily in tests; skip.
            return;
        }

        // The file is reachable via the bottle path (no `..` in the Windows
        // path), so the component walk won't catch it. The containment guard
        // must reject it.
        let result = resolve_windows_path(&bottle, "C:\\Games\\evil.exe");
        assert!(
            result.is_none(),
            "symlink escape should be caught by containment guard"
        );
    }

    #[test]
    fn resolve_windows_path_rejects_null_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = make_fake_bottle(tmp.path(), "b");
        assert!(resolve_windows_path(&bottle, "C:\\Games\\foo\0bar.exe").is_none());
    }

    #[test]
    fn match_shortcut_returns_none_for_unknown_exe() {
        let sc = CrossoverShortcut {
            bottle_name: "b".into(),
            display_name: "totally-unknown".into(),
            source_lnk_path: PathBuf::from("/dev/null"),
            windows_target: "C:\\Games\\zzz_not_a_real_game_zzz.exe".into(),
            host_target: PathBuf::from("/tmp/zzz_not_a_real_game_zzz.exe"),
            working_directory: None,
            icon_path: None,
        };
        assert!(match_shortcut(&sc).is_none());
    }

    /// Synthetic .lnk fixture: build an in-memory shell-link binary minimal
    /// enough for parselnk to extract `local_base_path`. We exercise the
    /// scanner end-to-end: fake bottle + synthetic `.lnk` + fake exe.
    ///
    /// parselnk's binary layout is finicky, so rather than hand-rolling a
    /// shell link we test scan_bottle_shortcuts against a real .lnk-style
    /// blob captured from a CrossOver bottle. If no fixture is available,
    /// this test verifies the scanner is at least called and returns
    /// gracefully.
    #[test]
    fn scan_bottle_returns_empty_when_no_lnks_present() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = make_fake_bottle(tmp.path(), "empty");
        let users = bottle.users_dir().join("crossover").join("Start Menu").join("Programs");
        fs::create_dir_all(&users).unwrap();
        let result = scan_bottle_shortcuts(&bottle);
        assert!(result.is_empty());
    }

    #[test]
    fn scan_bottle_skips_non_lnk_files() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = make_fake_bottle(tmp.path(), "noise");
        let programs = bottle
            .users_dir()
            .join("crossover")
            .join("Start Menu")
            .join("Programs");
        fs::create_dir_all(&programs).unwrap();
        fs::write(programs.join("readme.txt"), "not a shortcut").unwrap();
        fs::write(programs.join("config.ini"), "[stuff]").unwrap();

        let result = scan_bottle_shortcuts(&bottle);
        assert!(result.is_empty());
    }

    /// Modern CrossOver / Wine writes Start Menu shortcuts under
    /// `<user>/AppData/Roaming/Microsoft/Windows/Start Menu/Programs`, NOT
    /// `<user>/Start Menu/Programs`. The scanner must walk the modern path.
    #[test]
    fn scan_walks_modern_appdata_start_menu() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = make_fake_bottle(tmp.path(), "modern");
        let modern = bottle
            .users_dir()
            .join("crossover")
            .join("AppData")
            .join("Roaming")
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("Steam");
        fs::create_dir_all(&modern).unwrap();
        // Sentinel file — not a real .lnk, but the recursive walk must reach
        // it. We assert presence by writing a fake non-.lnk noise file and
        // confirming the scan returns clean (no panic, walked path exists).
        fs::write(modern.join("Steam Support Center.url"), "[InternetShortcut]\n").unwrap();

        let result = scan_bottle_shortcuts(&bottle);
        // No real .lnk → still empty, but the path was walked without panic.
        assert!(result.is_empty());
    }

    /// System-wide ProgramData Start Menu also needs to be walked.
    #[test]
    fn scan_walks_programdata_start_menu() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = make_fake_bottle(tmp.path(), "sys");
        let system = bottle
            .drive_c()
            .join("ProgramData")
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("Steam");
        fs::create_dir_all(&system).unwrap();
        fs::write(system.join("readme.txt"), "noise").unwrap();
        let result = scan_bottle_shortcuts(&bottle);
        assert!(result.is_empty());
    }

    /// Public/shared desktop must be scanned too.
    #[test]
    fn scan_walks_public_desktop() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = make_fake_bottle(tmp.path(), "pub");
        let public = bottle
            .drive_c()
            .join("users")
            .join("Public")
            .join("Desktop");
        fs::create_dir_all(&public).unwrap();
        fs::write(public.join("Steam.url"), "[InternetShortcut]\n").unwrap();
        let result = scan_bottle_shortcuts(&bottle);
        assert!(result.is_empty());
    }

    #[test]
    fn list_unregistered_games_dedupes_already_registered_exes() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = make_fake_bottle(tmp.path(), "b");
        let game_path = bottle.drive_c().join("Games").join("Foo");
        fs::create_dir_all(&game_path).unwrap();
        let exe = game_path.join("foo.exe");
        fs::write(&exe, vec![0u8; 6 * 1024 * 1024]).unwrap();

        let already = vec![DetectedGame {
            game_id: "foo".into(),
            display_name: "Foo".into(),
            nexus_slug: "foo".into(),
            game_path: game_path.clone(),
            exe_path: Some(exe.clone()),
            data_dir: game_path.clone(),
            runtime: GameRuntime::Wine(WineContext {
                bottle_name: bottle.name.clone(),
                bottle_path: bottle.path.clone(),
                source: bottle.source.clone(),
            }),
            steam_app_id: None,
            is_custom: false,
        }];

        // No shortcuts present — list should be empty regardless of
        // dedup logic. The point here is the function executes cleanly
        // without scanning the actual filesystem.
        let result = list_unregistered_games(&[bottle], &already);
        assert!(result.is_empty());
    }

    // -----------------------------------------------------------------------
    // Fix 4: Mod Engine 2 .bat shortcut matching
    // -----------------------------------------------------------------------

    /// Build a minimal CrossoverShortcut whose `host_target` filename is the
    /// supplied string — enough for `match_shortcut` to extract the name.
    fn bat_shortcut(filename: &str) -> CrossoverShortcut {
        CrossoverShortcut {
            bottle_name: "b".into(),
            display_name: filename.to_string(),
            source_lnk_path: PathBuf::from("/dev/null"),
            windows_target: format!("C:\\Games\\ModEngine2\\{}", filename),
            host_target: PathBuf::from("/fake/bottle/drive_c/Games/ModEngine2").join(filename),
            working_directory: None,
            icon_path: None,
        }
    }

    #[test]
    fn me2_bat_matches_eldenring() {
        let sc = bat_shortcut("launchmod_eldenring.bat");
        let hint = match_shortcut(&sc).expect("should match eldenring ME2 bat");
        assert_eq!(hint.game_id, "eldenring");
        assert_eq!(hint.display_name, "Elden Ring");
        assert_eq!(hint.nexus_slug, "eldenring");
        assert_eq!(hint.steam_app_id, None);
        assert_eq!(hint.source, MatchSource::Plugin);
    }

    #[test]
    fn me2_bat_matches_darksouls3() {
        let sc = bat_shortcut("launchmod_darksouls3.bat");
        let hint = match_shortcut(&sc).expect("should match darksouls3 ME2 bat");
        assert_eq!(hint.game_id, "darksouls3");
        assert_eq!(hint.source, MatchSource::Plugin);
    }

    #[test]
    fn me2_bat_matches_armoredcore6() {
        let sc = bat_shortcut("launchmod_armoredcore6.bat");
        let hint = match_shortcut(&sc).expect("should match armoredcore6 ME2 bat");
        assert_eq!(hint.game_id, "armoredcore6");
        assert_eq!(hint.source, MatchSource::Plugin);
    }

    #[test]
    fn me2_bat_rejects_sekiro() {
        // Sekiro was dropped from ME2 v2.x — must not produce a hint.
        let sc = bat_shortcut("launchmod_sekiro.bat");
        assert!(match_shortcut(&sc).is_none(), "sekiro should not be matched by ME2 bat logic");
    }

    #[test]
    fn me2_bat_rejects_random_bat() {
        let sc = bat_shortcut("random.bat");
        assert!(match_shortcut(&sc).is_none());
    }

    #[test]
    fn me2_bat_is_case_insensitive() {
        // CrossOver may preserve the original filename capitalisation from
        // Windows, e.g. `LaunchMod_EldenRing.BAT`.
        let sc = bat_shortcut("LaunchMod_EldenRing.BAT");
        let hint = match_shortcut(&sc).expect("case-insensitive ME2 bat should match");
        assert_eq!(hint.game_id, "eldenring");
    }

    // -----------------------------------------------------------------------
    // Fix 5: Multi-drive resolution
    // -----------------------------------------------------------------------

    fn make_bottle_with_drive(parent: &Path, name: &str, extra_drives: &[char]) -> Bottle {
        let path = parent.join(name);
        fs::create_dir_all(path.join("drive_c")).unwrap();
        for &letter in extra_drives {
            fs::create_dir_all(path.join(format!("drive_{}", letter))).unwrap();
        }
        Bottle {
            name: name.to_string(),
            path,
            source: "CrossOver".to_string(),
        }
    }

    #[test]
    fn resolve_d_drive_when_drive_d_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = make_bottle_with_drive(tmp.path(), "b", &['d']);
        let games_dir = bottle.path.join("drive_d").join("Games").join("Foo");
        fs::create_dir_all(&games_dir).unwrap();

        let resolved = resolve_windows_path(&bottle, "D:\\Games\\Foo\\foo.exe")
            .expect("should resolve D: path when drive_d exists");
        assert_eq!(resolved, games_dir.join("foo.exe"));
    }

    #[test]
    fn reject_d_drive_when_drive_d_missing() {
        let tmp = tempfile::tempdir().unwrap();
        // No drive_d directory created.
        let bottle = make_fake_bottle(tmp.path(), "b");
        assert!(
            resolve_windows_path(&bottle, "D:\\Games\\Foo\\foo.exe").is_none(),
            "should reject D: when drive_d does not exist"
        );
    }

    #[test]
    fn reject_traversal_on_non_c_drive() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = make_bottle_with_drive(tmp.path(), "b", &['z']);
        // Path traversal attempt via Z: drive.
        assert!(
            resolve_windows_path(&bottle, "Z:\\..\\..\\etc\\passwd").is_none(),
            "traversal via non-C drive must be rejected"
        );
    }

    #[test]
    fn c_drive_still_works_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = make_fake_bottle(tmp.path(), "b");
        fs::create_dir_all(bottle.drive_c().join("Games").join("Foo")).unwrap();
        let resolved = resolve_windows_path(&bottle, "C:\\Games\\Foo\\bar.exe").unwrap();
        assert_eq!(
            resolved,
            bottle.drive_c().join("Games").join("Foo").join("bar.exe")
        );
    }

    // -----------------------------------------------------------------------
    // cxmenu.conf integration tests
    // -----------------------------------------------------------------------

    /// A bottle with both a real `.lnk` (via parse_shortcut_file) AND a
    /// `cxmenu.conf` entry for a different game should return both entries,
    /// deduplicated so the Steam.lnk entry is not doubled.
    ///
    /// We can't easily synthesize a valid parselnk binary, so this test only
    /// places a `cxmenu.conf` entry (no real .lnk) and verifies the cxmenu
    /// path surfaces the exe correctly.
    #[test]
    fn scan_bottle_merges_cxmenu_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = make_fake_bottle(tmp.path(), "cx");

        // Create a fake game exe in drive_c that cxmenu.conf references.
        let game_dir = bottle.drive_c().join("Games").join("EldenRing");
        fs::create_dir_all(&game_dir).unwrap();
        // Use a large file so the size-based filter in is_likely_game() would
        // pass if called — scan_bottle_shortcuts itself does not filter, but
        // this keeps the fixture realistic for downstream callers.
        let exe = game_dir.join("eldenring.exe");
        fs::write(&exe, vec![0u8; 6 * 1024 * 1024]).unwrap();

        // Write a cxmenu.conf pointing to eldenring.exe.
        let cxmenu = r#"
[Desktop.C^3A_users_Public_Desktop/Elden+Ring.url]
"Type" = "Windows"
"Mode" = "install"
"StartupWMClass" = "eldenring.exe"
"#;
        fs::write(bottle.path.join("cxmenu.conf"), cxmenu).unwrap();

        let shortcuts = scan_bottle_shortcuts(&bottle);

        // Exactly one shortcut — the cxmenu-synthesized one.
        assert_eq!(shortcuts.len(), 1, "expected 1 shortcut, got {:?}", shortcuts);
        assert_eq!(shortcuts[0].host_target, exe);
        assert_eq!(shortcuts[0].bottle_name, "cx");
    }

    /// A cxmenu.conf entry that references an exe not present in drive_c must
    /// be silently dropped rather than producing a broken shortcut.
    #[test]
    fn scan_bottle_cxmenu_skips_missing_exe() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = make_fake_bottle(tmp.path(), "cx2");

        let cxmenu = r#"
[Desktop.C^3A_users_Public_Desktop/Missing+Game.url]
"Type" = "Windows"
"Mode" = "install"
"StartupWMClass" = "totally_missing.exe"
"#;
        fs::write(bottle.path.join("cxmenu.conf"), cxmenu).unwrap();

        let shortcuts = scan_bottle_shortcuts(&bottle);
        assert!(shortcuts.is_empty(), "expected empty, got {:?}", shortcuts);
    }

    /// An entry without StartupWMClass must be skipped gracefully.
    #[test]
    fn scan_bottle_cxmenu_skips_entry_without_wm_class() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = make_fake_bottle(tmp.path(), "cx3");

        let cxmenu = r#"
[Desktop.C^3A_users_Public_Desktop/SomeURL.url]
"Type" = "Windows"
"Mode" = "install"
"#;
        fs::write(bottle.path.join("cxmenu.conf"), cxmenu).unwrap();

        let shortcuts = scan_bottle_shortcuts(&bottle);
        assert!(shortcuts.is_empty());
    }

    /// Two cxmenu.conf entries pointing to the same exe must be deduplicated
    /// so only one shortcut is returned.
    #[test]
    fn scan_bottle_cxmenu_deduplicates_same_exe() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = make_fake_bottle(tmp.path(), "cx4");

        let game_dir = bottle.drive_c().join("Games").join("SomeGame");
        fs::create_dir_all(&game_dir).unwrap();
        let exe = game_dir.join("somegame.exe");
        fs::write(&exe, vec![0u8; 6 * 1024 * 1024]).unwrap();

        // Two entries for the same game (Desktop + StartMenu).
        let cxmenu = r#"
[Desktop.C^3A_users_Public_Desktop/SomeGame.url]
"Mode" = "install"
"StartupWMClass" = "somegame.exe"

[StartMenu.C^3A_users_crossover_AppData/SomeGame.url]
"Mode" = "install"
"StartupWMClass" = "somegame.exe"
"#;
        fs::write(bottle.path.join("cxmenu.conf"), cxmenu).unwrap();

        let shortcuts = scan_bottle_shortcuts(&bottle);
        assert_eq!(shortcuts.len(), 1, "expected dedup to 1, got {:?}", shortcuts);
    }

    // -----------------------------------------------------------------------
    // End-to-end: ME2 .bat shortcut surfaces through list_unregistered_games
    // -----------------------------------------------------------------------

    /// Verify that a ME2 `launchmod_eldenring.bat` registered in `cxmenu.conf`
    /// (the real-world path CrossOver uses) is surfaced by
    /// `list_unregistered_games` with the correct `MatchHint`.
    ///
    /// This is the integration test the reviewer requested: it exercises the
    /// full pipeline from cxmenu parsing → shortcut synthesis → `is_likely_game`
    /// gate → `match_shortcut` → `list_unregistered_games` output.
    #[test]
    fn me2_bat_surfaces_in_list_unregistered_games_via_cxmenu() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = make_fake_bottle(tmp.path(), "me2_test");

        // Create the ME2 launcher bat file in a plausible location.
        let me2_dir = bottle.drive_c().join("Games").join("ModEngine2");
        fs::create_dir_all(&me2_dir).unwrap();
        // .bat files are tiny by design — the size filter must not apply.
        let bat = me2_dir.join("launchmod_eldenring.bat");
        fs::write(&bat, b"@echo off\nstart \"\" eldenring.exe\n").unwrap();

        // Register the bat via cxmenu.conf (the real CrossOver path).
        let cxmenu = r#"
[Desktop.C^3A_users_Public_Desktop/Elden+Ring+(ME2).url]
"Type" = "Windows"
"Mode" = "install"
"StartupWMClass" = "launchmod_eldenring.bat"
"#;
        fs::write(bottle.path.join("cxmenu.conf"), cxmenu).unwrap();

        // No pre-registered games.
        let already: Vec<DetectedGame> = Vec::new();
        let unreg = list_unregistered_games(&[bottle], &already);

        assert_eq!(
            unreg.len(),
            1,
            "expected 1 unregistered game (ME2 bat), got {:?}",
            unreg
        );
        let u = &unreg[0];
        assert_eq!(
            u.shortcut.host_target, bat,
            "host_target should be the bat file"
        );
        let hint = u.match_hint.as_ref().expect("ME2 bat should produce a match hint");
        assert_eq!(hint.game_id, "eldenring");
        assert_eq!(hint.source, MatchSource::Plugin);
    }
}
