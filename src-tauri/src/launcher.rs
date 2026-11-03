//! Game launching through Wine, CrossOver, Whisky, and other compatibility
//! layers on macOS and Linux.
//!
//! Provides the ability to spawn a game executable within a Wine bottle,
//! automatically selecting the correct Wine binary and environment variables
//! based on the bottle's `source` field.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::bottles::Bottle;

/// Whether the given game_id refers to one of the FromSoft titles where we
/// run a pre-launch save backup. Centralized here so launcher.rs and the
/// frontend agree on the canonical set.
fn is_fromsoft_game(game_id: &str) -> bool {
    matches!(
        game_id,
        "sekiro" | "eldenring" | "darksouls3" | "darksouls_remastered" | "armoredcore6"
    )
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum LauncherError {
    #[error("Executable not found: {0}")]
    ExecutableNotFound(PathBuf),

    #[error("Bottle does not exist: {0}")]
    BottleNotFound(String),

    #[error("Wine is required but was not found.\n\n\
        Looked for: {tried}\n\n\
        To install Wine:\n\
        \u{2022} macOS: Install CrossOver or run 'brew install --cask wine-stable'\n\
        \u{2022} Linux: Install via your package manager (e.g. 'sudo apt install wine' or 'sudo dnf install wine')\n\
        \u{2022} Steam Deck: Wine is bundled with Proton \u{2014} ensure Steam is installed")]
    WineNotFound {
        bottle_source: String,
        tried: String,
    },

    #[error("Failed to launch process: {0}")]
    ProcessSpawn(#[from] std::io::Error),

    #[error("Unsupported bottle source: {0}")]
    UnsupportedSource(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, LauncherError>;

// ---------------------------------------------------------------------------
// LaunchResult
// ---------------------------------------------------------------------------

/// Result of a game launch attempt, suitable for returning to the Tauri frontend.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaunchResult {
    /// The executable that was launched (display path).
    pub executable: String,
    /// Name of the bottle used for the launch.
    pub bottle_name: String,
    /// PID of the spawned process (for monitoring).
    pub pid: Option<u32>,
    /// Whether the process was successfully spawned.
    pub success: bool,
    /// Optional warning message (e.g., missing permissions for cursor clamp).
    pub warning: Option<String>,
}

// ---------------------------------------------------------------------------
// Wine binary resolution
// ---------------------------------------------------------------------------

/// Well-known path to the CrossOver Wine binary on macOS.
#[cfg(target_os = "macos")]
const CROSSOVER_WINE: &str =
    "/Applications/CrossOver.app/Contents/SharedSupport/CrossOver/bin/wine";

/// Path from a CrossOver.app bundle root to its Wine binary.
#[cfg(target_os = "macos")]
const CROSSOVER_WINE_SUFFIX: &str = "Contents/SharedSupport/CrossOver/bin/wine";

/// Base container path for Whisky on macOS.
#[cfg(target_os = "macos")]
const WHISKY_CONTAINER: &str = "Library/Containers/com.isaacmarovitz.Whisky";

/// Attempt to locate the Wine binary for a CrossOver bottle.
#[cfg(target_os = "macos")]
fn find_crossover_wine() -> Option<PathBuf> {
    // System-wide and per-user Applications folders.
    let mut bundles = vec![PathBuf::from("/Applications/CrossOver.app")];
    if let Some(home) = dirs::home_dir() {
        bundles.push(home.join("Applications").join("CrossOver.app"));
    }
    for bundle in &bundles {
        let wine = bundle.join(CROSSOVER_WINE_SUFFIX);
        if wine.exists() {
            return Some(wine);
        }
    }

    // Fall back to a Spotlight lookup by bundle identifier, which finds
    // CrossOver wherever the user moved it.
    let output = std::process::Command::new("mdfind")
        .arg("kMDItemCFBundleIdentifier == 'com.codeweavers.CrossOver'")
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let wine = Path::new(line.trim()).join(CROSSOVER_WINE_SUFFIX);
        if wine.exists() {
            return Some(wine);
        }
    }

    None
}

/// Attempt to locate a Wine binary shipped inside the Whisky container.
#[cfg(target_os = "macos")]
fn find_whisky_wine() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let container = home.join(WHISKY_CONTAINER);

    if !container.exists() {
        return None;
    }

    // Whisky bundles Wine under its container; search for the binary.
    // Common locations within the container:
    let candidates = [
        container
            .join("Bottles")
            .join("Wine")
            .join("bin")
            .join("wine"),
        container.join("Wine").join("bin").join("wine"),
    ];

    for candidate in &candidates {
        if candidate.exists() {
            return Some(candidate.clone());
        }
    }

    // Fall back to searching for any `wine` binary under the container.
    find_wine_binary_under(&container)
}

/// Recursively search under `dir` for a file named `wine` (the binary).
/// Stops at the first match to avoid an exhaustive walk.
#[allow(dead_code)]
fn find_wine_binary_under(dir: &Path) -> Option<PathBuf> {
    let walker = walkdir::WalkDir::new(dir)
        .max_depth(6)
        .into_iter()
        .filter_map(|e| e.ok());

    for entry in walker {
        if entry.file_type().is_file() {
            if let Some(name) = entry.file_name().to_str() {
                if name == "wine" {
                    return Some(entry.into_path());
                }
            }
        }
    }

    None
}

/// NotProton's CrossOver runner, for running Windows programs in a NotProton
/// prefix without Steam (mod tools such as xEdit). Mirrors the runner choice
/// in NotProton's `run` script.
#[derive(Debug, PartialEq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
struct NotProtonRunner {
    cx_root: PathBuf,
    wine: PathBuf,
    wineserver: PathBuf,
    /// `lib/wine/<arch>-unix`, the Unix-side DLL directory for `WINEDLLPATH`.
    wine_unix: PathBuf,
}

/// Find the runner of an installed NotProton tool:
/// `<steam_root>/compatibilitytools.d/notproton-*/{build,flavor}` names
/// `<support_dir>/runners/crossover-<build>/CrossOver`. The newest tool name
/// with an existing runner wins.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn find_notproton_runner(steam_root: &Path, support_dir: &Path) -> Option<NotProtonRunner> {
    let mut tools: Vec<PathBuf> = std::fs::read_dir(steam_root.join("compatibilitytools.d"))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("notproton-"))
        })
        .collect();
    tools.sort();
    tools.reverse();

    for tool in tools {
        let Ok(build) = std::fs::read_to_string(tool.join("build")) else {
            continue;
        };
        let build = build.trim();
        if build.is_empty()
            || !build.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        {
            continue;
        }
        let flavor = std::fs::read_to_string(tool.join("flavor")).unwrap_or_default();
        let cx_root = support_dir
            .join("runners")
            .join(format!("crossover-{build}"))
            .join("CrossOver");

        let arm_unix = cx_root.join("lib/wine/aarch64-unix");
        let arm = NotProtonRunner {
            wine: arm_unix.join("wine.app/Contents/MacOS/wine"),
            wineserver: cx_root.join("bin/wineserver-arm64"),
            wine_unix: arm_unix,
            cx_root: cx_root.clone(),
        };
        if flavor.trim() != "rosetta" && arm.wine.is_file() && arm.wineserver.is_file() {
            return Some(arm);
        }

        let x86_unix = cx_root.join("lib/wine/x86_64-unix");
        let wineserver = [cx_root.join("bin/wineserver"), cx_root.join("bin/wineserver-x86")]
            .into_iter()
            .find(|p| p.is_file());
        let wine = x86_unix.join("wine");
        if let (true, Some(wineserver)) = (wine.is_file(), wineserver) {
            return Some(NotProtonRunner {
                cx_root,
                wine,
                wineserver,
                wine_unix: x86_unix,
            });
        }
    }
    None
}

/// Resolved Wine command with binary, extra args (before exe), and environment.
struct WineCommand {
    /// Path to the Wine binary.
    binary: PathBuf,
    /// Arguments to insert before the exe path (e.g. CrossOver's `--bottle <name>`).
    prefix_args: Vec<String>,
    /// Environment variables to set.
    env_vars: Vec<(String, String)>,
}

/// Resolve the Wine binary path and command structure for a given bottle source.
fn resolve_wine_binary(bottle: &Bottle) -> Result<WineCommand> {
    let source = bottle.source.as_str();
    let mut env_vars: Vec<(String, String)> = Vec::new();
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut prefix_args: Vec<String> = Vec::new();

    let wine_bin = match source {
        #[cfg(target_os = "macos")]
        "CrossOver" => {
            let wine = find_crossover_wine().ok_or_else(|| LauncherError::WineNotFound {
                bottle_source: source.to_string(),
                tried: format!(
                    "{}, ~/Applications/CrossOver.app, and Spotlight (com.codeweavers.CrossOver)",
                    CROSSOVER_WINE
                ),
            })?;
            // Validate bottle name with a whitelist of safe characters.
            // CrossOver bottle names are directory names (e.g. "Skyrim: Anniversary Edition").
            if bottle.name.is_empty()
                || bottle.name.len() > 255
                || bottle.name.starts_with('-')
                || !bottle
                    .name
                    .chars()
                    .all(|c| c.is_alphanumeric() || " :-_().,'!+#@".contains(c))
            {
                return Err(LauncherError::Other(format!(
                    "Invalid bottle name: {}",
                    bottle.name
                )));
            }
            prefix_args.push("--bottle".to_string());
            prefix_args.push(bottle.name.clone());
            wine
        }

        // NotProton prefix run without Steam (mod tools). Games themselves go
        // through `launch_via_steam`.
        #[cfg(target_os = "macos")]
        "NotProton" => {
            let home = dirs::home_dir().unwrap_or_default();
            let support = home.join("Library/Application Support/notproton");
            let runner = find_notproton_runner(&crate::proton::macos_steam_root(&home), &support)
                .ok_or_else(|| LauncherError::WineNotFound {
                    bottle_source: source.to_string(),
                    tried: format!(
                        "NotProton runner under {}/runners (from Steam/compatibilitytools.d/notproton-*)",
                        support.display()
                    ),
                })?;
            let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin:/usr/sbin:/sbin".into());
            env_vars.extend([
                ("WINEPREFIX".to_string(), bottle.path.to_string_lossy().into_owned()),
                ("CX_ROOT".to_string(), runner.cx_root.to_string_lossy().into_owned()),
                ("WINELOADER".to_string(), runner.wine.to_string_lossy().into_owned()),
                ("WINESERVER".to_string(), runner.wineserver.to_string_lossy().into_owned()),
                (
                    "WINEDLLPATH".to_string(),
                    format!(
                        "{}:{}",
                        runner.cx_root.join("lib/wine/x86_64-windows").display(),
                        runner.wine_unix.display()
                    ),
                ),
                ("PATH".to_string(), format!("{}:{path}", runner.cx_root.join("bin").display())),
            ]);
            runner.wine
        }

        #[cfg(target_os = "macos")]
        "Whisky" | "Moonshine" => {
            // Whisky/Moonshine use WINEPREFIX like standard Wine
            env_vars.push((
                "WINEPREFIX".to_string(),
                bottle.path.to_string_lossy().into_owned(),
            ));
            find_whisky_wine().ok_or_else(|| LauncherError::WineNotFound {
                bottle_source: source.to_string(),
                tried: format!("~/{}", WHISKY_CONTAINER),
            })?
        }

        // Native Wine, Lutris, Bottles, Heroic, Mythic — all use
        // the system `wine` binary (or a Wine binary on PATH) with WINEPREFIX.
        "Wine" | "Lutris" | "Bottles" | "Heroic" | "Mythic" => {
            env_vars.push((
                "WINEPREFIX".to_string(),
                bottle.path.to_string_lossy().into_owned(),
            ));
            find_system_wine().ok_or_else(|| LauncherError::WineNotFound {
                bottle_source: source.to_string(),
                tried: "wine (system PATH)".to_string(),
            })?
        }

        // GameHub bottles are only the top layer of a prefix GameHub
        // assembles at launch; running Wine on them would corrupt it.
        crate::gamehub::SOURCE => {
            return Err(LauncherError::UnsupportedSource(format!(
                "{} (games must be started through GameHub)",
                source
            )));
        }

        // Proton behaves similarly to native Wine for our purposes.
        "Proton" => {
            env_vars.push((
                "WINEPREFIX".to_string(),
                bottle.path.to_string_lossy().into_owned(),
            ));
            let proton_wine = find_proton_wine(bottle);
            match proton_wine {
                Some(p) => p,
                None => find_system_wine().ok_or_else(|| LauncherError::WineNotFound {
                    bottle_source: source.to_string(),
                    tried: "Proton wine, wine (system PATH)".to_string(),
                })?,
            }
        }

        #[cfg(not(target_os = "macos"))]
        "CrossOver" | "Whisky" | "Moonshine" => {
            env_vars.push((
                "WINEPREFIX".to_string(),
                bottle.path.to_string_lossy().into_owned(),
            ));
            find_system_wine().ok_or_else(|| LauncherError::WineNotFound {
                bottle_source: source.to_string(),
                tried: "wine (system PATH)".to_string(),
            })?
        }

        _ => {
            warn!(
                "Unknown bottle source '{}', falling back to system wine",
                source
            );
            env_vars.push((
                "WINEPREFIX".to_string(),
                bottle.path.to_string_lossy().into_owned(),
            ));
            find_system_wine().ok_or_else(|| LauncherError::WineNotFound {
                bottle_source: source.to_string(),
                tried: "wine (system PATH)".to_string(),
            })?
        }
    };

    Ok(WineCommand {
        binary: wine_bin,
        prefix_args,
        env_vars,
    })
}

/// Start a game in a GameHub bottle through GameHub's `gamehub://` deep
/// link, which runs it exactly like GameHub's Play button.
///
/// GameHub only starts the games it manages, so this refuses anything that
/// isn't the game's own executable (script extender loaders, modding tools)
/// instead of silently starting the vanilla game.
fn launch_via_gamehub(bottle: &Bottle, exe: &Path, game_id: Option<&str>) -> Result<LaunchResult> {
    let exe_name = exe.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let is_game_exe = game_id
        .and_then(|gid| {
            crate::games::with_plugin(gid, |p| {
                p.executables().iter().any(|e| e.eq_ignore_ascii_case(exe_name))
            })
        })
        .unwrap_or(false);
    if !is_game_exe {
        return Err(LauncherError::Other(format!(
            "GameHub only starts the game itself, so Corkscrew can't run {} in {}. \
             Script extenders (like SKSE) and modding tools aren't supported for GameHub games yet. \
             Your deployed mods are in place; start the game from GameHub.",
            exe_name, bottle.name
        )));
    }

    let app_id = crate::gamehub::bound_steam_app_id(bottle).or_else(|| {
        game_id
            .and_then(|gid| {
                crate::games::with_plugin(gid, |p| p.steam_launch_id().map(str::to_string))
            })
            .flatten()
    });
    let url = app_id
        .as_deref()
        .and_then(crate::gamehub::steam_launch_url)
        .ok_or_else(|| {
            LauncherError::Other(format!(
                "Couldn't find the Steam app GameHub has bound to {}. Start the game from GameHub.",
                bottle.name
            ))
        })?;

    info!("Launching via GameHub: {} (bottle {})", url, bottle.name);
    open_url(&url)?;

    Ok(LaunchResult {
        executable: exe.display().to_string(),
        bottle_name: bottle.name.clone(),
        // GameHub owns the game process; there is no child to track.
        pid: None,
        success: true,
        warning: None,
    })
}

#[cfg(target_os = "macos")]
fn open_url(url: &str) -> Result<()> {
    let status = Command::new("/usr/bin/open").arg(url).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(LauncherError::Other(format!(
            "Couldn't open {} (is GameHub installed?)",
            url
        )))
    }
}

#[cfg(not(target_os = "macos"))]
fn open_url(url: &str) -> Result<()> {
    Err(LauncherError::Other(format!(
        "GameHub launches are only supported on macOS ({})",
        url
    )))
}

/// Locate `wine` on the system PATH, falling back to common install
/// locations and finally to the system Wine fork enumerator (which finds
/// wine-tkg / wine-staging / wine-ge / wine-cachyos under `/opt/wine-*`,
/// `~/.local/opt/wine*`, and similar custom paths).
fn find_system_wine() -> Option<PathBuf> {
    // Try `which wine` to find it on PATH.
    let output = Command::new("which").arg("wine").output().ok();
    if let Some(out) = output {
        if out.status.success() {
            let path_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path_str.is_empty() {
                let path = PathBuf::from(&path_str);
                if path.exists() {
                    return Some(path);
                }
            }
        }
    }

    // Common fallback locations on Linux.
    let mut fallbacks: Vec<PathBuf> = vec![
        PathBuf::from("/usr/bin/wine"),
        PathBuf::from("/usr/local/bin/wine"),
        PathBuf::from("/opt/wine-stable/bin/wine"),
        PathBuf::from("/opt/wine-staging/bin/wine"),
    ];

    // User-compiled Wine (e.g. from source or wine-builds).
    if let Some(home) = dirs::home_dir() {
        fallbacks.push(home.join(".local/bin/wine"));
    }

    for fb in &fallbacks {
        if fb.exists() {
            return Some(fb.clone());
        }
    }

    // Final fallback: scan well-known system Wine fork locations
    // (`/opt/wine-*`, `/usr/local/bin/wine*`, `~/.local/opt/wine*`).
    // Picks the first discovered fork; CachyOS users often have wine-tkg or
    // wine-cachyos here without it being on $PATH.
    let forks = crate::proton::detect_system_wine_forks();
    if let Some(fork) = forks.into_iter().next() {
        debug!(
            "find_system_wine: falling back to system Wine fork {} at {}",
            fork.variant,
            fork.wine_bin.display()
        );
        return Some(fork.wine_bin);
    }

    None
}

/// Attempt to find a Wine binary bundled with Proton for this bottle.
///
/// Steam Proton stores its Wine build alongside the compatibility tool.
///
/// Delegates to `crate::proton::find_proton_for_bottle()` which scans all
/// known Proton locations (steamapps/common, compatibilitytools.d, Flatpak,
/// Snap, secondary library folders) and returns the best match.
fn find_proton_wine(bottle: &Bottle) -> Option<PathBuf> {
    crate::proton::find_proton_for_bottle(&bottle.path).map(|v| v.wine_bin)
}

// ---------------------------------------------------------------------------
// Flatpak sandbox escape
// ---------------------------------------------------------------------------

/// Wrap a command for Flatpak if needed.
/// If running inside Flatpak, prepends `flatpak-spawn --host` to break out of sandbox.
#[allow(dead_code)]
pub fn maybe_flatpak_spawn(cmd: &str, args: &[&str]) -> (String, Vec<String>) {
    if crate::bottles::detect_container_environment()
        == Some(crate::bottles::ContainerEnvironment::Flatpak)
    {
        let mut all_args = vec!["--host".to_string(), cmd.to_string()];
        all_args.extend(args.iter().map(|a| a.to_string()));
        ("flatpak-spawn".to_string(), all_args)
    } else {
        (cmd.to_string(), args.iter().map(|a| a.to_string()).collect())
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Launch a game executable within a Wine bottle.
///
/// Spawns the Wine process with the appropriate binary and environment
/// variables for the bottle's source manager. The child process is detached
/// so the UI does not block while the game is running.
///
/// # Arguments
///
/// * `bottle` - The Wine bottle to launch within.
/// * `exe_path` - Absolute path to the Windows `.exe` to run.
/// * `working_dir` - Optional working directory (defaults to the exe's parent).
///
/// # Returns
///
/// A [`LaunchResult`] indicating whether the process was spawned successfully.
/// Start a Steam Play game through the host Steam client
/// (`steam://rungameid/<appid>`). NotProton games must start this way: Steam
/// runs NotProton's launch script, which supplies the Steam client bridge the
/// game's DRM needs. Running the exe under Wine directly would skip it.
pub fn launch_via_steam(bottle: &Bottle) -> Result<LaunchResult> {
    let app_id = bottle.steam_compat_app_id().ok_or_else(|| {
        LauncherError::Other(format!(
            "'{}' is not a Steam Play prefix (expected .../steamapps/compatdata/<appid>/pfx)",
            bottle.name
        ))
    })?;
    let url = format!("steam://rungameid/{app_id}");

    info!("Launching {} through Steam: {}", bottle.name, url);
    #[cfg(target_os = "macos")]
    Command::new("open").arg(&url).spawn()?;
    // xdg-open uses the desktop's steam:// handler (native or Flatpak
    // Steam); the `steam` binary forwards the URL to a running client.
    #[cfg(not(target_os = "macos"))]
    if let Err(e) = Command::new("xdg-open").arg(&url).spawn() {
        warn!("xdg-open failed ({e}); trying the steam binary");
        Command::new("steam").arg(&url).spawn()?;
    }

    Ok(LaunchResult {
        executable: url,
        bottle_name: bottle.name.clone(),
        // Steam owns the game process; there is no child PID to track.
        pid: None,
        success: true,
        warning: None,
    })
}

pub fn launch_game(
    bottle: &Bottle,
    exe_path: &Path,
    working_dir: Option<&Path>,
    game_id: Option<&str>,
    modlist_name: Option<&str>,
) -> Result<LaunchResult> {
    // Validate the bottle exists.
    if !bottle.exists() {
        return Err(LauncherError::BottleNotFound(bottle.name.clone()));
    }

    // Validate the executable exists (case-insensitive search as fallback).
    let resolved_exe = if exe_path.exists() {
        exe_path.to_path_buf()
    } else if let Some(parent) = exe_path.parent() {
        if let Some(fname) = exe_path.file_name().and_then(|n| n.to_str()) {
            find_executable(parent, fname)
                .ok_or_else(|| LauncherError::ExecutableNotFound(exe_path.to_path_buf()))?
        } else {
            return Err(LauncherError::ExecutableNotFound(exe_path.to_path_buf()));
        }
    } else {
        return Err(LauncherError::ExecutableNotFound(exe_path.to_path_buf()));
    };

    // GameHub assembles its layered prefix only when it starts a game itself,
    // so its bottles are launched through GameHub, never with Wine directly.
    if bottle.source == crate::gamehub::SOURCE {
        return launch_via_gamehub(bottle, &resolved_exe, game_id);
    }

    // Determine working directory.
    let work_dir = working_dir
        .map(|p| p.to_path_buf())
        .or_else(|| resolved_exe.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| bottle.drive_c());

    // Resolve the Wine binary and environment.
    let mut wine_cmd = resolve_wine_binary(bottle)?;

    // Apply pre-launch fixes if game_id is known.
    // This injects env vars and DLL overrides from the builtin + user fix database.
    if let Some(gid) = game_id {
        let fixes = crate::launch_fixes::resolve_fixes(gid, modlist_name, &[]);
        let applied = crate::launch_fixes::apply_fixes(&fixes);

        if applied.total_applied > 0 {
            info!("Applying {} pre-launch fixes for '{}'", applied.total_applied, gid);
        }

        // Inject env vars
        for (k, v) in &applied.env_vars {
            wine_cmd.env_vars.push((k.clone(), v.clone()));
        }

        // Merge DLL overrides into WINEDLLOVERRIDES env var
        if !applied.dll_overrides.is_empty() {
            let existing = wine_cmd.env_vars.iter()
                .find(|(k, _)| k == "WINEDLLOVERRIDES")
                .map(|(_, v)| v.as_str());
            let merged = crate::launch_fixes::build_dll_override_env(
                &applied.dll_overrides,
                existing,
            );
            // Remove existing WINEDLLOVERRIDES if present, then add merged
            wine_cmd.env_vars.retain(|(k, _)| k != "WINEDLLOVERRIDES");
            wine_cmd.env_vars.push(("WINEDLLOVERRIDES".into(), merged));
        }
    }

    info!(
        "Launching game: wine={} prefix_args={:?} exe={} prefix={} workdir={}",
        wine_cmd.binary.display(),
        wine_cmd.prefix_args,
        resolved_exe.display(),
        bottle.path.display(),
        work_dir.display(),
    );

    // Pre-launch FromSoft save backup. Best-effort: never block the launch
    // on backup failure. FromSoft saves get corrupted under Wine when the
    // game is interrupted ungracefully — this guards against losing tens
    // of hours of progress.
    if let Some(gid) = game_id {
        if is_fromsoft_game(gid) {
            match crate::fromsoft_saves::backup_saves_before_launch(
                bottle,
                gid,
                crate::fromsoft_saves::DEFAULT_MAX_BACKUPS,
            ) {
                Ok(0) => {} // No saves yet — nothing to back up.
                Ok(n) => info!("FromSoft save backup: {} file(s) saved before launch", n),
                Err(e) => warn!("FromSoft save backup failed (non-fatal): {}", e),
            }
        }
    }

    // Build and spawn the command.
    let mut cmd = Command::new(&wine_cmd.binary);
    // Add prefix args (e.g. CrossOver --bottle <name>) before the exe
    for arg in &wine_cmd.prefix_args {
        cmd.arg(arg);
    }
    cmd.arg(&resolved_exe);
    cmd.current_dir(&work_dir);

    for (key, value) in &wine_cmd.env_vars {
        cmd.env(key, value);
        debug!("  env: {}={}", key, value);
    }

    // Detach the child so the Tauri app does not block.
    // On Unix, we can use spawn() which does not wait.
    match cmd.spawn() {
        Ok(child) => {
            let pid = child.id();
            info!(
                "Game process spawned (pid={}): {}",
                pid,
                resolved_exe.display()
            );
            Ok(LaunchResult {
                executable: resolved_exe.to_string_lossy().into_owned(),
                bottle_name: bottle.name.clone(),
                pid: Some(pid),
                success: true,
                warning: None,
            })
        }
        Err(e) => {
            warn!("Failed to spawn game process: {}", e);
            Err(LauncherError::ProcessSpawn(e))
        }
    }
}

/// Search for an executable by name (case-insensitive) in a directory.
///
/// Returns the full path to the first matching file, or `None` if no match
/// is found. Only scans the immediate directory (non-recursive).
pub fn find_executable(game_path: &Path, exe_name: &str) -> Option<PathBuf> {
    if !game_path.is_dir() {
        return None;
    }

    let target = exe_name.to_lowercase();

    // Try exact match first (fast path).
    let exact = game_path.join(exe_name);
    if exact.exists() {
        return Some(exact);
    }

    // Case-insensitive scan.
    let entries = fs::read_dir(game_path).ok()?;
    for entry in entries.flatten() {
        if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name == target {
                return Some(entry.path());
            }
        }
    }

    None
}

/// Search recursively for an executable by name (case-insensitive) in a
/// directory tree. Useful for finding deeply nested game executables.
#[allow(dead_code)]
pub fn find_executable_recursive(game_path: &Path, exe_name: &str) -> Option<PathBuf> {
    if !game_path.is_dir() {
        return None;
    }

    let target = exe_name.to_lowercase();

    for entry in walkdir::WalkDir::new(game_path)
        .max_depth(5)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name == target {
                return Some(entry.into_path());
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_notproton(tmp: &Path, build: &str, flavor: &str, arm: bool) -> (PathBuf, PathBuf) {
        let steam_root = tmp.join("Steam");
        let support = tmp.join("notproton");
        let tool = steam_root.join("compatibilitytools.d/notproton-26.3");
        fs::create_dir_all(&tool).unwrap();
        fs::write(tool.join("build"), format!("{build}\n")).unwrap();
        fs::write(tool.join("flavor"), format!("{flavor}\n")).unwrap();
        let cx = support.join(format!("runners/crossover-{build}/CrossOver"));
        for f in ["lib/wine/x86_64-unix/wine", "bin/wineserver"] {
            fs::create_dir_all(cx.join(f).parent().unwrap()).unwrap();
            fs::write(cx.join(f), "").unwrap();
        }
        if arm {
            for f in ["lib/wine/aarch64-unix/wine.app/Contents/MacOS/wine", "bin/wineserver-arm64"] {
                fs::create_dir_all(cx.join(f).parent().unwrap()).unwrap();
                fs::write(cx.join(f), "").unwrap();
            }
        }
        (steam_root, support)
    }

    #[test]
    fn notproton_runner_uses_x86_64_wine_for_rosetta() {
        let tmp = tempfile::tempdir().unwrap();
        let (steam_root, support) = make_notproton(tmp.path(), "26.3.0.39832", "rosetta", true);
        let cx = support.join("runners/crossover-26.3.0.39832/CrossOver");
        assert_eq!(
            find_notproton_runner(&steam_root, &support),
            Some(NotProtonRunner {
                wine: cx.join("lib/wine/x86_64-unix/wine"),
                wineserver: cx.join("bin/wineserver"),
                wine_unix: cx.join("lib/wine/x86_64-unix"),
                cx_root: cx,
            })
        );
    }

    #[test]
    fn notproton_runner_uses_arm64_wine_for_fex_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        let (steam_root, support) = make_notproton(tmp.path(), "26.3.0.39832", "fex", true);
        let runner = find_notproton_runner(&steam_root, &support).unwrap();
        assert!(runner.wine.ends_with("aarch64-unix/wine.app/Contents/MacOS/wine"));
        assert!(runner.wineserver.ends_with("bin/wineserver-arm64"));

        // FEX tool without the arm64 binaries falls back to x86_64, like `run`.
        let tmp = tempfile::tempdir().unwrap();
        let (steam_root, support) = make_notproton(tmp.path(), "26.3.0.39832", "fex", false);
        let runner = find_notproton_runner(&steam_root, &support).unwrap();
        assert!(runner.wine.ends_with("x86_64-unix/wine"));
    }

    #[test]
    fn notproton_runner_rejects_unsafe_build_and_missing_runner() {
        let tmp = tempfile::tempdir().unwrap();
        let (steam_root, support) = make_notproton(tmp.path(), "../../etc", "rosetta", false);
        assert_eq!(find_notproton_runner(&steam_root, &support), None);

        let tmp = tempfile::tempdir().unwrap();
        let (steam_root, _) = make_notproton(tmp.path(), "26.3.0.39832", "rosetta", false);
        assert_eq!(find_notproton_runner(&steam_root, &tmp.path().join("elsewhere")), None);
    }

    #[test]
    fn launch_via_steam_rejects_non_steam_prefix() {
        let bottle = Bottle {
            name: "Skyrim".into(),
            path: PathBuf::from("/tmp/Bottles/Skyrim"),
            source: "CrossOver".into(),
        };
        assert!(matches!(launch_via_steam(&bottle), Err(LauncherError::Other(_))));
    }

    #[test]
    fn gamehub_bottle_never_resolves_a_wine_binary() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = create_fake_bottle(tmp.path(), "1", crate::gamehub::SOURCE);
        assert!(matches!(
            resolve_wine_binary(&bottle),
            Err(LauncherError::UnsupportedSource(_))
        ));
    }

    #[test]
    fn gamehub_refuses_non_game_executables() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = create_fake_bottle(tmp.path(), "1", crate::gamehub::SOURCE);
        let tool = tmp.path().join("skse64_loader.exe");
        fs::write(&tool, b"MZ").unwrap();

        // A script extender loader and an unknown tool (no game id) are both
        // refused before anything is spawned.
        for game_id in [Some("skyrimse"), None] {
            let err = launch_game(&bottle, &tool, None, game_id, None).unwrap_err();
            assert!(
                matches!(&err, LauncherError::Other(msg) if msg.contains("GameHub only starts the game itself")),
                "unexpected error: {err}"
            );
        }
    }

    /// Create a minimal fake bottle with drive_c.
    fn create_fake_bottle(parent: &Path, name: &str, source: &str) -> Bottle {
        let path = parent.join(name);
        fs::create_dir_all(path.join("drive_c")).expect("create drive_c");
        Bottle {
            name: name.to_string(),
            path,
            source: source.to_string(),
        }
    }

    #[test]
    fn find_executable_exact_match() {
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path().join("game");
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join("SkyrimSE.exe"), b"fake exe").unwrap();

        let result = find_executable(&game_dir, "SkyrimSE.exe");
        assert!(result.is_some());
        assert!(result.unwrap().ends_with("SkyrimSE.exe"));
    }

    #[test]
    fn find_executable_case_insensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path().join("game");
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join("SkyrimSE.exe"), b"fake exe").unwrap();

        // Search with different casing.
        let result = find_executable(&game_dir, "skyrimse.exe");
        assert!(result.is_some());
    }

    #[test]
    fn find_executable_returns_none_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path().join("game");
        fs::create_dir_all(&game_dir).unwrap();

        let result = find_executable(&game_dir, "nonexistent.exe");
        assert!(result.is_none());
    }

    #[test]
    fn find_executable_returns_none_for_nonexistent_dir() {
        let result = find_executable(Path::new("/tmp/corkscrew_no_such_dir"), "test.exe");
        assert!(result.is_none());
    }

    #[test]
    fn launch_result_serializes() {
        let result = LaunchResult {
            executable: "C:\\Games\\SkyrimSE.exe".to_string(),
            bottle_name: "Gaming".to_string(),
            pid: Some(12345),
            success: true,
            warning: None,
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("SkyrimSE.exe"));
        assert!(json.contains("Gaming"));
        assert!(json.contains("true"));

        let deserialized: LaunchResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.executable, result.executable);
        assert_eq!(deserialized.bottle_name, result.bottle_name);
        assert_eq!(deserialized.success, result.success);
    }

    #[test]
    fn launch_fails_with_nonexistent_bottle() {
        let bottle = Bottle {
            name: "Ghost".to_string(),
            path: PathBuf::from("/tmp/corkscrew_no_such_bottle"),
            source: "Wine".to_string(),
        };

        let result = launch_game(&bottle, Path::new("/tmp/some.exe"), None, None, None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, LauncherError::BottleNotFound(_)),
            "Expected BottleNotFound, got: {:?}",
            err
        );
    }

    #[test]
    fn launch_fails_with_missing_exe() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = create_fake_bottle(tmp.path(), "TestBottle", "Wine");

        let result = launch_game(&bottle, Path::new("/tmp/corkscrew_no_such_game.exe"), None, None, None);
        assert!(result.is_err());
    }

    #[test]
    fn find_executable_recursive_finds_nested() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("sub").join("dir");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("game.exe"), b"fake").unwrap();

        let result = find_executable_recursive(tmp.path(), "game.exe");
        assert!(result.is_some());
        assert!(result.unwrap().ends_with("game.exe"));
    }
}
