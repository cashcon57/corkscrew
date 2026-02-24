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

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum LauncherError {
    #[error("Executable not found: {0}")]
    ExecutableNotFound(PathBuf),

    #[error("Bottle does not exist: {0}")]
    BottleNotFound(String),

    #[error("Wine binary not found for source '{bottle_source}': tried {tried}")]
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
}

// ---------------------------------------------------------------------------
// Wine binary resolution
// ---------------------------------------------------------------------------

/// Well-known path to the CrossOver Wine binary on macOS.
#[cfg(target_os = "macos")]
const CROSSOVER_WINE: &str =
    "/Applications/CrossOver.app/Contents/SharedSupport/CrossOver/bin/wine";

/// Base container path for Whisky on macOS.
#[cfg(target_os = "macos")]
const WHISKY_CONTAINER: &str = "Library/Containers/com.isaacmarovitz.Whisky";

/// Attempt to locate the Wine binary for a CrossOver bottle.
#[cfg(target_os = "macos")]
fn find_crossover_wine() -> Option<PathBuf> {
    let path = PathBuf::from(CROSSOVER_WINE);
    if path.exists() {
        Some(path)
    } else {
        // Also check a user-local install via Homebrew Cask / alternate location.
        let alt = PathBuf::from("/opt/homebrew/Caskroom/crossover")
            .join("bin")
            .join("wine");
        if alt.exists() {
            Some(alt)
        } else {
            None
        }
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
                tried: CROSSOVER_WINE.to_string(),
            })?;
            // CrossOver uses --bottle <name> to target a specific bottle.
            // The bottle name is the directory name under CrossOver/Bottles/.
            // Validate bottle name to prevent argument injection
            if bottle.name.starts_with('-')
                || !bottle
                    .name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.' || c == ' ')
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

        // Proton behaves similarly to native Wine for our purposes.
        "Proton" => {
            env_vars.push((
                "WINEPREFIX".to_string(),
                bottle.path.to_string_lossy().into_owned(),
            ));
            let proton_wine = find_proton_wine(bottle);
            proton_wine
                .unwrap_or_else(|| find_system_wine().unwrap_or_else(|| PathBuf::from("wine")))
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

/// Locate `wine` on the system PATH.
fn find_system_wine() -> Option<PathBuf> {
    // Try `which wine` to find it on PATH.
    let output = Command::new("which").arg("wine").output().ok()?;
    if output.status.success() {
        let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path_str.is_empty() {
            let path = PathBuf::from(&path_str);
            if path.exists() {
                return Some(path);
            }
        }
    }

    // Common fallback locations on Linux.
    let fallbacks = [
        "/usr/bin/wine",
        "/usr/local/bin/wine",
        "/opt/wine-stable/bin/wine",
        "/opt/wine-staging/bin/wine",
    ];

    for fb in &fallbacks {
        let p = PathBuf::from(fb);
        if p.exists() {
            return Some(p);
        }
    }

    None
}

/// Attempt to find a Wine binary bundled with Proton for this bottle.
///
/// Steam Proton stores its Wine build alongside the compatibility tool.
/// The typical layout is:
///   `~/.local/share/Steam/steamapps/common/Proton X.Y/dist/bin/wine`
/// or
///   `~/.local/share/Steam/steamapps/common/Proton X.Y/files/bin/wine`
fn find_proton_wine(bottle: &Bottle) -> Option<PathBuf> {
    // Proton bottles live under steamapps/compatdata/<appid>/pfx.
    // Walk up from the bottle path to find the steamapps root, then look
    // for Proton installations under steamapps/common/.
    let bottle_path = &bottle.path;

    // Navigate up to find `steamapps`
    let mut ancestor = bottle_path.parent();
    while let Some(dir) = ancestor {
        if dir
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase() == "steamapps")
            .unwrap_or(false)
        {
            // Found the steamapps directory; look for Proton under common/
            let common = dir.join("common");
            if common.is_dir() {
                if let Ok(entries) = fs::read_dir(&common) {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_lowercase();
                        if name.starts_with("proton") {
                            let proton_dir = entry.path();
                            // Check both dist/bin/wine and files/bin/wine
                            for sub in &["dist/bin/wine", "files/bin/wine"] {
                                let wine = proton_dir.join(sub);
                                if wine.exists() {
                                    return Some(wine);
                                }
                            }
                        }
                    }
                }
            }
            break;
        }
        ancestor = dir.parent();
    }

    None
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
pub fn launch_game(
    bottle: &Bottle,
    exe_path: &Path,
    working_dir: Option<&Path>,
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

    // Determine working directory.
    let work_dir = working_dir
        .map(|p| p.to_path_buf())
        .or_else(|| resolved_exe.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| bottle.drive_c());

    // Resolve the Wine binary and environment.
    let wine_cmd = resolve_wine_binary(bottle)?;

    info!(
        "Launching game: wine={} prefix_args={:?} exe={} prefix={} workdir={}",
        wine_cmd.binary.display(),
        wine_cmd.prefix_args,
        resolved_exe.display(),
        bottle.path.display(),
        work_dir.display(),
    );

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

        let result = launch_game(&bottle, Path::new("/tmp/some.exe"), None);
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

        let result = launch_game(&bottle, Path::new("/tmp/corkscrew_no_such_game.exe"), None);
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
