//! Wine prefix dependency auto-setup.
//!
//! Automates installation of runtime dependencies (vcrun, .NET, d3d libs) into
//! Wine prefixes using winetricks or protontricks. This is the #1 barrier to
//! modding on Linux — users must manually install these components.

use log::{info, warn};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::{AppHandle, Emitter};

// ---------------------------------------------------------------------------
// Dependency definitions per game
// ---------------------------------------------------------------------------

/// A Wine prefix dependency that can be installed via winetricks.
#[derive(Debug, Clone, Serialize)]
pub struct WineDependency {
    /// Winetricks verb (e.g., "vcrun2022", "d3dcompiler_47")
    pub verb: String,
    /// Human-readable name
    pub name: String,
    /// Whether this is required (vs recommended)
    pub required: bool,
    /// Whether this is already installed (detected)
    pub installed: bool,
}

/// Get the required dependencies for a given game.
pub fn get_game_dependencies(game_id: &str) -> Vec<WineDependency> {
    match game_id {
        "skyrimse" | "skyrimspecialedition" => vec![
            dep("vcrun2022", "Visual C++ 2022 Runtime", true),
            dep("d3dcompiler_47", "D3D Compiler 47", true),
            dep("d3dx9", "DirectX 9 Extensions", false),
        ],
        "fallout4" => vec![
            dep("vcrun2022", "Visual C++ 2022 Runtime", true),
            dep("d3dcompiler_47", "D3D Compiler 47", true),
            dep("d3dx11_43", "DirectX 11 Extensions", false),
        ],
        "oblivion" => vec![
            dep("vcrun2019", "Visual C++ 2019 Runtime", true),
            dep("d3dx9", "DirectX 9 Extensions", true),
        ],
        "skyrim" => vec![
            dep("vcrun2019", "Visual C++ 2019 Runtime", true),
            dep("d3dx9", "DirectX 9 Extensions", true),
        ],
        "fallout3" | "falloutnv" => vec![
            dep("vcrun2019", "Visual C++ 2019 Runtime", true),
            dep("d3dx9", "DirectX 9 Extensions", true),
        ],
        "starfield" => vec![
            dep("vcrun2022", "Visual C++ 2022 Runtime", true),
            dep("d3dcompiler_47", "D3D Compiler 47", true),
        ],
        _ => vec![
            // Sensible defaults for unknown games
            dep("vcrun2022", "Visual C++ 2022 Runtime", false),
            dep("d3dcompiler_47", "D3D Compiler 47", false),
        ],
    }
}

fn dep(verb: &str, name: &str, required: bool) -> WineDependency {
    WineDependency {
        verb: verb.to_string(),
        name: name.to_string(),
        required,
        installed: false,
    }
}

// ---------------------------------------------------------------------------
// Dependency detection
// ---------------------------------------------------------------------------

/// Check which dependencies are already installed in a Wine prefix.
pub fn check_installed_deps(
    prefix_path: &Path,
    deps: &mut [WineDependency],
) {
    // Check winetricks.log for installed verbs
    let log_path = prefix_path.join("winetricks.log");
    let installed_verbs: Vec<String> = if log_path.exists() {
        std::fs::read_to_string(&log_path)
            .unwrap_or_default()
            .lines()
            .map(|l| l.trim().to_string())
            .collect()
    } else {
        Vec::new()
    };

    // Also check for DLL presence as a secondary signal.
    // Wine paths are case-insensitive — try common case variants.
    let system32_candidates = [
        prefix_path.join("drive_c/windows/system32"),
        prefix_path.join("drive_c/Windows/System32"),
        prefix_path.join("drive_c/windows/System32"),
    ];
    let system32 = system32_candidates
        .iter()
        .find(|p| p.is_dir())
        .cloned()
        .unwrap_or_else(|| prefix_path.join("drive_c/windows/system32"));

    for dep in deps.iter_mut() {
        // Check winetricks log
        if installed_verbs.iter().any(|v| v == &dep.verb) {
            dep.installed = true;
            continue;
        }

        // Check DLL presence as fallback detection (case-insensitive via walkdir)
        dep.installed = match dep.verb.as_str() {
            "vcrun2022" | "vcrun2019" => {
                dll_exists_ci(&system32, "msvcp140.dll")
                    || dll_exists_ci(&system32, "vcruntime140.dll")
            }
            "d3dcompiler_47" => dll_exists_ci(&system32, "d3dcompiler_47.dll"),
            "d3dx9" => dll_exists_ci(&system32, "d3dx9_43.dll"),
            "d3dx11_43" => dll_exists_ci(&system32, "d3dx11_43.dll"),
            _ => false,
        };
    }
}

/// Case-insensitive DLL existence check (Wine may use any casing).
fn dll_exists_ci(dir: &std::path::Path, dll_name: &str) -> bool {
    let lower = dll_name.to_lowercase();
    // Fast path: check exact name first
    if dir.join(dll_name).exists() {
        return true;
    }
    // Slow path: case-insensitive scan (only if dir exists)
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().to_lowercase() == lower {
                return true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Dependency installation
// ---------------------------------------------------------------------------

/// Result of a dependency installation attempt.
#[derive(Debug, Clone, Serialize)]
pub struct InstallResult {
    pub verb: String,
    pub success: bool,
    pub message: String,
}

/// Install dependencies into a Wine prefix.
///
/// Detects whether to use protontricks (Proton prefix) or winetricks (standalone Wine).
pub fn install_dependencies(
    app: &AppHandle,
    prefix_path: &Path,
    wine_bin: Option<&Path>,
    deps: &[WineDependency],
    steam_app_id: Option<u32>,
) -> Vec<InstallResult> {
    let mut results = Vec::new();

    let deps_to_install: Vec<&WineDependency> = deps.iter()
        .filter(|d| !d.installed)
        .collect();

    if deps_to_install.is_empty() {
        info!("All dependencies already installed");
        return results;
    }

    info!(
        "Installing {} dependencies into prefix {}",
        deps_to_install.len(),
        prefix_path.display()
    );

    // Determine if this is a Proton prefix (has Steam app ID)
    let use_protontricks = steam_app_id.is_some() && which_command("protontricks").is_some();

    for (i, dep) in deps_to_install.iter().enumerate() {
        let _ = app.emit("prefix://setup-progress", serde_json::json!({
            "current": i + 1,
            "total": deps_to_install.len(),
            "verb": &dep.verb,
            "name": &dep.name,
        }));

        let result = if use_protontricks {
            install_via_protontricks(steam_app_id.unwrap(), &dep.verb)
        } else {
            install_via_winetricks(prefix_path, wine_bin, &dep.verb)
        };

        let install_result = match result {
            Ok(()) => {
                info!("Installed {} successfully", dep.verb);
                InstallResult {
                    verb: dep.verb.clone(),
                    success: true,
                    message: "Installed successfully".to_string(),
                }
            }
            Err(e) => {
                warn!("Failed to install {}: {}", dep.verb, e);
                InstallResult {
                    verb: dep.verb.clone(),
                    success: false,
                    message: e,
                }
            }
        };

        results.push(install_result);
    }

    results
}

fn install_via_protontricks(app_id: u32, verb: &str) -> Result<(), String> {
    info!("Running protontricks {} {}", app_id, verb);

    let output = Command::new("protontricks")
        .arg("--no-bwrap")  // Avoid bubblewrap issues on some distros
        .arg(app_id.to_string())
        .arg(verb)
        .env("STEAM_RUNTIME", "0")
        .output()
        .map_err(|e| format!("Failed to run protontricks: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("protontricks failed: {}", stderr.trim()))
    }
}

fn install_via_winetricks(
    prefix_path: &Path,
    wine_bin: Option<&Path>,
    verb: &str,
) -> Result<(), String> {
    let winetricks = which_command("winetricks")
        .ok_or_else(|| "winetricks not found — install it via your package manager".to_string())?;

    info!("Running winetricks {} in prefix {}", verb, prefix_path.display());

    let mut cmd = Command::new(&winetricks);
    cmd.arg("--unattended")
        .arg(verb)
        .env("WINEPREFIX", prefix_path);

    if let Some(wine) = wine_bin {
        if let Some(wine_dir) = wine.parent() {
            cmd.env("WINE", wine);
            // Add wine bin dir to PATH
            if let Ok(path) = std::env::var("PATH") {
                cmd.env("PATH", format!("{}:{}", wine_dir.display(), path));
            }
        }
    }

    let output = cmd.output()
        .map_err(|e| format!("Failed to run winetricks: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("winetricks failed: {}", stderr.trim()))
    }
}

/// Install a .NET runtime into a Wine prefix by running the installer EXE.
///
/// For Synthesis and other tools that need .NET 9+ SDK or Desktop Runtime.
pub fn install_dotnet_runtime(
    prefix_path: &Path,
    wine_bin: &Path,
    installer_path: &Path,
) -> Result<(), String> {
    if !installer_path.exists() {
        return Err(format!("Installer not found: {}", installer_path.display()));
    }

    info!(
        "Installing .NET runtime from {} into {}",
        installer_path.display(),
        prefix_path.display()
    );

    let output = Command::new(wine_bin)
        .arg(installer_path)
        .arg("/install")
        .arg("/quiet")
        .arg("/norestart")
        .env("WINEPREFIX", prefix_path)
        .output()
        .map_err(|e| format!("Failed to run .NET installer via Wine: {}", e))?;

    if output.status.success() {
        info!(".NET runtime installed successfully");
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(".NET installer failed: {}", stderr.trim()))
    }
}

/// Find a command on PATH.
fn which_command(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?
        .to_string_lossy()
        .split(':')
        .map(|dir| PathBuf::from(dir).join(name))
        .find(|p| p.exists())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_skyrim_deps() {
        let deps = get_game_dependencies("skyrimse");
        assert!(deps.iter().any(|d| d.verb == "vcrun2022"));
        assert!(deps.iter().any(|d| d.verb == "d3dcompiler_47"));
    }

    #[test]
    fn test_get_fallout4_deps() {
        let deps = get_game_dependencies("fallout4");
        assert!(deps.iter().any(|d| d.verb == "vcrun2022"));
    }

    #[test]
    fn test_get_unknown_game_deps() {
        let deps = get_game_dependencies("unknown_game_xyz");
        assert!(!deps.is_empty()); // Should have sensible defaults
    }

    #[test]
    fn test_check_installed_empty_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let mut deps = get_game_dependencies("skyrimse");
        check_installed_deps(dir.path(), &mut deps);
        assert!(deps.iter().all(|d| !d.installed));
    }

    #[test]
    fn test_check_installed_with_log() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("winetricks.log"), "vcrun2022\n").unwrap();
        let mut deps = get_game_dependencies("skyrimse");
        check_installed_deps(dir.path(), &mut deps);
        assert!(deps.iter().find(|d| d.verb == "vcrun2022").unwrap().installed);
    }
}
