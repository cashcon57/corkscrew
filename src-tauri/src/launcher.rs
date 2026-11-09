//! Game launching through Wine, CrossOver, Whisky, and other compatibility
//! layers on macOS and Linux.
//!
//! Provides the ability to spawn a game executable within a Wine bottle,
//! automatically selecting the correct Wine binary and environment variables
//! based on the bottle's `source` field.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

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

    #[error("The game exited immediately ({}).{}\n\nFull log: {}",
        code.map_or_else(|| "killed by a signal".to_string(), |c| format!("exit code {c}")),
        if tail.is_empty() { String::new() } else { format!(" Last output:\n{tail}") },
        log_path.display())]
    EarlyExit {
        code: Option<i32>,
        log_path: PathBuf,
        tail: String,
    },

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

/// Base container path for Whisky on macOS.
#[cfg(target_os = "macos")]
const WHISKY_CONTAINER: &str = "Library/Containers/com.isaacmarovitz.Whisky";

/// Locate the Wine binary of the CrossOver install that owns `bottle` (see
/// [`crate::crossover_install`]): a per-bottle override, the build whose
/// wineserver is running for the prefix, or the build that created it.
#[cfg(target_os = "macos")]
fn find_crossover_wine(bottle: &Bottle) -> Result<PathBuf> {
    use crate::crossover_install::{select_for_bottle, SelectError};
    match select_for_bottle(&bottle.name, &bottle.path) {
        Ok(sel) => Ok(sel.install.wine()),
        Err(SelectError::NoInstalls) => Err(LauncherError::WineNotFound {
            bottle_source: bottle.source.clone(),
            tried: "CrossOver*.app in /Applications and ~/Applications, and Spotlight \
                    (com.codeweavers.CrossOver*)"
                .to_string(),
        }),
        Err(e) => Err(LauncherError::Other(e.to_string())),
    }
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
            let wine = find_crossover_wine(bottle)?;
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
/// Run a modding tool for a GameHub game with GameHub's bundled Wine in a
/// prefix Corkscrew owns. GameHub assembles a game's container only when it
/// starts the game, so Wine must never run on the container itself. Tools
/// that only need the game's files (ModImporter, BodySlide, …) work here;
/// tools for games with a launcher run inside the container instead, via
/// the launch trampoline (see `commands::tools::launch_mod_tool`).
fn launch_gamehub_tool(bottle: &Bottle, exe: &Path) -> Result<LaunchResult> {
    let home = dirs::home_dir().unwrap_or_default();
    let wine = crate::gamehub::bundled_wine(&home).ok_or_else(|| LauncherError::WineNotFound {
        bottle_source: bottle.source.clone(),
        tried: "GameHub's bundled Wine (wine-engine/containers/wine_installations)".to_string(),
    })?;
    let prefix = crate::config::data_dir().join("tool_prefixes").join("gamehub");
    if !prefix.join("drive_c").is_dir() {
        info!("Creating Corkscrew's GameHub tools prefix at {}", prefix.display());
        std::fs::create_dir_all(&prefix)?;
        let status = Command::new(&wine)
            .arg("wineboot")
            .arg("-i")
            .env("WINEPREFIX", &prefix)
            .env("WINEDEBUG", "-all")
            .status()?;
        if !status.success() {
            return Err(LauncherError::Other(format!(
                "Couldn't create the tools prefix with GameHub's Wine ({status})"
            )));
        }
    }
    info!("Running {} for {} with GameHub's Wine in {}", exe.display(), bottle.name, prefix.display());
    let mut child = Command::new(&wine)
        .arg(exe)
        .current_dir(exe.parent().unwrap_or(Path::new("/")))
        .env("WINEPREFIX", &prefix)
        .env("WINEDEBUG", "-all")
        .spawn()?;
    let pid = child.id();
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(LaunchResult {
        executable: exe.display().to_string(),
        bottle_name: bottle.name.clone(),
        pid: Some(pid),
        success: true,
        warning: Some(
            "This tool runs with GameHub's Wine in Corkscrew's own tools prefix: it sees the game's files, \
             not GameHub's per-game settings."
                .to_string(),
        ),
    })
}

/// Whether GameHub should start `exe` as the game (GameHub starts only the
/// games it manages); anything else is a tool.
fn is_gamehub_game_exe(exe: &Path, game_id: Option<&str>) -> bool {
    let exe_name = exe.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    game_id
        .and_then(|gid| {
            crate::games::with_plugin(gid, |p| {
                p.executables().iter().any(|e| e.eq_ignore_ascii_case(exe_name))
            })
        })
        .unwrap_or(false)
}

fn launch_via_gamehub(bottle: &Bottle, exe: &Path, game_id: Option<&str>) -> Result<LaunchResult> {
    if !is_gamehub_game_exe(exe, game_id) {
        return launch_gamehub_tool(bottle, exe);
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

/// Launch `exe_path` in `bottle` under the bottle's Wine.
///
/// Blocks for up to [`EARLY_EXIT_WINDOW`] watching for an immediate failure:
/// a non-zero exit in that window is returned as
/// [`LauncherError::EarlyExit`] (with the tail of the per-launch log in
/// [`launch_log_dir`]) instead of a success the UI would show as "running".
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

    run_wine(
        bottle,
        &wine_cmd,
        &resolved_exe,
        &work_dir,
        &launch_log_dir(),
        EARLY_EXIT_WINDOW,
    )
}

// ---------------------------------------------------------------------------
// Spawning and early-exit detection
// ---------------------------------------------------------------------------

/// How long a freshly started game is watched for an immediate failure
/// (e.g. CrossOver's `wine` refusing a bottle whose wineserver belongs to
/// another CrossOver build exits within a few seconds).
const EARLY_EXIT_WINDOW: Duration = Duration::from_secs(8);

/// Per-launch Wine output logs kept on disk.
const MAX_LAUNCH_LOGS: usize = 20;

/// Lines of Wine output quoted in an early-exit error.
const EARLY_EXIT_TAIL_LINES: usize = 15;

/// Directory for per-launch Wine output logs.
pub fn launch_log_dir() -> PathBuf {
    crate::config::data_dir().join("logs").join("launch")
}

/// `launch-YYYYmmdd-HHMMSS-<label>.log` in `dir`, pruning old launch logs.
fn new_launch_log_path(dir: &Path, label: &str) -> PathBuf {
    if let Ok(entries) = fs::read_dir(dir) {
        let mut logs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("launch-") && n.ends_with(".log"))
            })
            .collect();
        if logs.len() >= MAX_LAUNCH_LOGS {
            logs.sort(); // timestamped names sort chronologically
            for old in &logs[..=logs.len() - MAX_LAUNCH_LOGS] {
                let _ = fs::remove_file(old);
            }
        }
    }
    let label: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .take(48)
        .collect();
    dir.join(format!(
        "launch-{}-{}.log",
        chrono::Local::now().format("%Y%m%d-%H%M%S%.3f"),
        label
    ))
}

/// Last `n` non-empty lines of a log file.
fn log_tail(path: &Path, n: usize) -> String {
    let content = fs::read(path).unwrap_or_default();
    // Only the end matters; cap what we decode.
    let start = content.len().saturating_sub(64 * 1024);
    let text = String::from_utf8_lossy(&content[start..]);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// Build the Wine command for `exe` and start it (see [`spawn_and_watch`]).
fn run_wine(
    bottle: &Bottle,
    wine_cmd: &WineCommand,
    exe: &Path,
    work_dir: &Path,
    log_dir: &Path,
    window: Duration,
) -> Result<LaunchResult> {
    let mut cmd = Command::new(&wine_cmd.binary);
    // Add prefix args (e.g. CrossOver --bottle <name>) before the exe
    cmd.args(&wine_cmd.prefix_args);
    cmd.arg(exe);
    cmd.current_dir(work_dir);
    for (key, value) in &wine_cmd.env_vars {
        cmd.env(key, value);
        debug!("  env: {}={}", key, value);
    }

    let label = exe
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "game".to_string());
    let pid = spawn_and_watch(cmd, log_dir, &label, window).map_err(|e| match e {
        LauncherError::EarlyExit { code, log_path, tail } => {
            let mut tail = tail;
            let lower = tail.to_ascii_lowercase();
            if bottle.source == "CrossOver"
                && (lower.contains("version mismatch") || lower.contains("wineserver"))
            {
                tail.push_str(
                    "\n\nThis bottle's wineserver may belong to a different CrossOver install. \
                     Quit CrossOver (or choose the matching install in Settings \u{2192} \
                     Game Maintenance \u{2192} CrossOver install) and try again.",
                );
            }
            LauncherError::EarlyExit { code, log_path, tail }
        }
        other => other,
    })?;
    info!("Game process running (pid={}): {}", pid, exe.display());
    Ok(LaunchResult {
        executable: exe.to_string_lossy().into_owned(),
        bottle_name: bottle.name.clone(),
        pid: Some(pid),
        success: true,
        warning: None,
    })
}

/// Spawn `cmd` with stdout/stderr going to a new per-launch log in
/// `log_dir`, then watch it for `window`. A non-zero exit inside the window
/// is returned as [`LauncherError::EarlyExit`] with the log's last lines; a
/// process still running afterwards is handed to a reaper thread so it never
/// lingers as a zombie. Returns the child's PID.
fn spawn_and_watch(mut cmd: Command, log_dir: &Path, label: &str, window: Duration) -> Result<u32> {
    let log_path = fs::create_dir_all(log_dir)
        .ok()
        .map(|_| new_launch_log_path(log_dir, label));
    let log_file = log_path.as_ref().and_then(|p| match fs::File::create(p) {
        Ok(f) => Some(f),
        Err(e) => {
            warn!("Could not create launch log {}: {}", p.display(), e);
            None
        }
    });
    cmd.stdin(Stdio::null());
    match log_file.as_ref().map(|f| (f.try_clone(), f.try_clone())) {
        Some((Ok(out), Ok(err))) => {
            cmd.stdout(out).stderr(err);
        }
        _ => {
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
        }
    }

    let mut child = cmd.spawn().map_err(|e| {
        warn!("Failed to spawn game process: {}", e);
        LauncherError::ProcessSpawn(e)
    })?;
    let pid = child.id();
    if let Some(p) = &log_path {
        info!("Game process spawned (pid={}), output: {}", pid, p.display());
    }

    let started = Instant::now();
    let exited = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() >= window => break None,
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => {
                warn!("Could not poll game process {}: {}", pid, e);
                break None;
            }
        }
    };

    match exited {
        Some(status) if !status.success() => {
            let tail = log_path
                .as_deref()
                .map(|p| log_tail(p, EARLY_EXIT_TAIL_LINES))
                .unwrap_or_default();
            warn!(
                "Game process {} exited after {:.1}s with {}",
                pid,
                started.elapsed().as_secs_f32(),
                status
            );
            Err(LauncherError::EarlyExit {
                code: status.code(),
                log_path: log_path.unwrap_or_default(),
                tail,
            })
        }
        Some(_) => {
            // Some launchers hand off and exit 0 while the game keeps running.
            info!("Launcher process {} exited cleanly after {:.1}s", pid, started.elapsed().as_secs_f32());
            Ok(pid)
        }
        None => {
            let spawned = std::thread::Builder::new()
                .name(format!("game-reaper-{pid}"))
                .spawn(move || match child.wait() {
                    Ok(status) => info!("Game process {} exited: {}", pid, status),
                    Err(e) => warn!("Waiting on game process {} failed: {}", pid, e),
                });
            if let Err(e) = spawned {
                warn!("Could not start reaper thread for pid {}: {}", pid, e);
            }
            Ok(pid)
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
    fn gamehub_starts_only_game_exes_and_treats_the_rest_as_tools() {
        crate::plugins::skyrim_se::register();
        assert!(is_gamehub_game_exe(Path::new("/g/SkyrimSE.exe"), Some("skyrimse")));
        assert!(is_gamehub_game_exe(Path::new("/g/skyrimse.EXE"), Some("skyrimse")));
        // Loaders and tools are never handed to GameHub as the game: they
        // run via the trampoline or Corkscrew's own tools prefix.
        assert!(!is_gamehub_game_exe(Path::new("/g/skse64_loader.exe"), Some("skyrimse")));
        assert!(!is_gamehub_game_exe(Path::new("/g/SSEEdit.exe"), None));
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

    // --- Early-exit detection ---

    /// A "wine" that exits immediately with `code`. Uses `/bin/sh` because
    /// it's the only path guaranteed everywhere: `true`/`false` live in /bin
    /// on FHS Linux, /usr/bin on macOS, and neither on NixOS.
    fn exiting_wine(code: i32) -> WineCommand {
        wine_cmd("/bin/sh", &["-c", &format!("exit {code}"), "sh"])
    }

    fn wine_cmd(binary: &str, prefix_args: &[&str]) -> WineCommand {
        WineCommand {
            binary: PathBuf::from(binary),
            prefix_args: prefix_args.iter().map(|s| s.to_string()).collect(),
            env_vars: Vec::new(),
        }
    }

    #[cfg(unix)]
    #[test]
    fn launch_with_failing_wine_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = create_fake_bottle(tmp.path(), "B", "CrossOver");
        let exe = tmp.path().join("game.exe");
        fs::write(&exe, b"MZ").unwrap();
        let logs = tmp.path().join("logs");

        let err = run_wine(
            &bottle,
            &exiting_wine(1),
            &exe,
            tmp.path(),
            &logs,
            Duration::from_secs(5),
        )
        .unwrap_err();
        match &err {
            LauncherError::EarlyExit { code, log_path, .. } => {
                assert_eq!(*code, Some(1));
                assert!(log_path.starts_with(&logs), "{}", log_path.display());
            }
            other => panic!("expected EarlyExit, got {other:?}"),
        }
        assert!(err.to_string().contains("exited immediately"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn early_exit_error_quotes_wine_output_and_hints_crossover() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = create_fake_bottle(tmp.path(), "Steam", "CrossOver");
        let exe = tmp.path().join("skse64_loader.exe");
        fs::write(&exe, b"MZ").unwrap();
        // `sh -c <script> sh <exe>`: the exe lands in $1 like it would for wine.
        let script = "echo \"starting $1\"; echo 'wine client error:0: version mismatch 812/811.' >&2; exit 3";
        let err = run_wine(
            &bottle,
            &wine_cmd("/bin/sh", &["-c", script, "sh"]),
            &exe,
            tmp.path(),
            &tmp.path().join("logs"),
            Duration::from_secs(5),
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("exit code 3"), "{msg}");
        assert!(msg.contains("version mismatch 812/811"), "{msg}");
        assert!(msg.contains("starting"), "{msg}");
        assert!(msg.contains("CrossOver install"), "{msg}");
    }

    #[cfg(unix)]
    #[test]
    fn long_running_game_is_reported_running() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = create_fake_bottle(tmp.path(), "B", "Wine");
        let exe = tmp.path().join("game.exe");
        fs::write(&exe, b"MZ").unwrap();
        let started = Instant::now();
        let result = run_wine(
            &bottle,
            &wine_cmd("/bin/sh", &["-c", "sleep 2", "sh"]),
            &exe,
            tmp.path(),
            &tmp.path().join("logs"),
            Duration::from_millis(300),
        )
        .unwrap();
        assert!(result.success);
        assert!(result.pid.is_some());
        assert!(started.elapsed() < Duration::from_secs(2));

        // A clean exit inside the window is also a success.
        let ok = run_wine(
            &bottle,
            &exiting_wine(0),
            &exe,
            tmp.path(),
            &tmp.path().join("logs"),
            Duration::from_secs(5),
        );
        assert!(ok.is_ok(), "{ok:?}");
    }

    #[test]
    fn launch_logs_are_pruned() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..MAX_LAUNCH_LOGS + 5 {
            fs::write(tmp.path().join(format!("launch-20260101-0000{i:02}-x.log")), b"").unwrap();
        }
        fs::write(tmp.path().join("other.txt"), b"").unwrap();
        let next = new_launch_log_path(tmp.path(), "Skyrim SE/../x");
        let name = next.file_name().unwrap().to_str().unwrap().to_string();
        assert!(name.starts_with("launch-") && name.ends_with("-Skyrim_SE____x.log"), "{name}");
        let remaining = fs::read_dir(tmp.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("launch-"))
            .count();
        assert_eq!(remaining, MAX_LAUNCH_LOGS - 1);
        assert!(tmp.path().join("other.txt").exists());
    }

    #[test]
    fn log_tail_returns_last_non_empty_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("l.log");
        fs::write(&p, "a\n\nb\nc\n\n").unwrap();
        assert_eq!(log_tail(&p, 2), "b\nc");
        assert_eq!(log_tail(&tmp.path().join("missing"), 2), "");
    }
}
