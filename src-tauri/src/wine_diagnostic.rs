//! Wine environment diagnostic and auto-fixer.
//!
//! Checks Wine/Proton bottle configuration for common issues that affect
//! game modding: DXVK version, DLL overrides, vcredist, symlink structure.

use std::collections::HashMap;
use std::fs;

use serde::Serialize;

use crate::bottles::Bottle;

/// Result of a diagnostic check.
#[derive(Clone, Debug, Serialize)]
pub struct DiagnosticResult {
    pub checks: Vec<DiagnosticCheck>,
    pub passed: usize,
    pub warnings: usize,
    pub errors: usize,
}

/// A single diagnostic check.
#[derive(Clone, Debug, Serialize)]
pub struct DiagnosticCheck {
    pub name: String,
    pub category: String,
    pub status: CheckStatus,
    pub message: String,
    pub fix_available: bool,
    pub fix_description: Option<String>,
}

/// Status of a diagnostic check.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Pass,
    Warning,
    Error,
    Skipped,
}

/// Run all diagnostics on a wine bottle.
pub fn run_diagnostics(bottle: &Bottle, game_id: &str) -> DiagnosticResult {
    let checks = vec![
        check_drive_c(bottle),
        check_appdata(bottle),
        check_retina_mode(bottle),
        check_dxvk(bottle),
        check_dll_overrides(bottle, game_id),
        check_vcredist(bottle),
        check_dotnet(bottle),
        check_windows_version(bottle),
        check_user_dirs(bottle),
    ];

    let passed = checks
        .iter()
        .filter(|c| c.status == CheckStatus::Pass)
        .count();
    let warnings = checks
        .iter()
        .filter(|c| c.status == CheckStatus::Warning)
        .count();
    let errors = checks
        .iter()
        .filter(|c| c.status == CheckStatus::Error)
        .count();

    DiagnosticResult {
        checks,
        passed,
        warnings,
        errors,
    }
}

/// Check that drive_c exists and is accessible.
fn check_drive_c(bottle: &Bottle) -> DiagnosticCheck {
    let drive_c = bottle.drive_c();
    if drive_c.exists() && drive_c.is_dir() {
        DiagnosticCheck {
            name: "Drive C".into(),
            category: "Structure".into(),
            status: CheckStatus::Pass,
            message: format!("drive_c exists at {}", drive_c.display()),
            fix_available: false,
            fix_description: None,
        }
    } else {
        DiagnosticCheck {
            name: "Drive C".into(),
            category: "Structure".into(),
            status: CheckStatus::Error,
            message: "drive_c directory is missing or inaccessible".into(),
            fix_available: false,
            fix_description: None,
        }
    }
}

/// Check that AppData\Local exists.
fn check_appdata(bottle: &Bottle) -> DiagnosticCheck {
    let appdata = bottle.appdata_local();
    if appdata.exists() {
        DiagnosticCheck {
            name: "AppData Local".into(),
            category: "Structure".into(),
            status: CheckStatus::Pass,
            message: "AppData\\Local directory exists".into(),
            fix_available: false,
            fix_description: None,
        }
    } else {
        DiagnosticCheck {
            name: "AppData Local".into(),
            category: "Structure".into(),
            status: CheckStatus::Warning,
            message: "AppData\\Local directory not found — game settings may not save".into(),
            fix_available: true,
            fix_description: Some("Create the AppData\\Local directory structure".into()),
        }
    }
}

/// Check for graphics translation layer (DXVK on Linux, D3DMetal on macOS).
fn check_dxvk(bottle: &Bottle) -> DiagnosticCheck {
    // macOS uses D3DMetal/GPTK via CrossOver — DXVK is Linux-only (Vulkan)
    if cfg!(target_os = "macos") {
        return DiagnosticCheck {
            name: "Graphics Translation".into(),
            category: "Graphics".into(),
            status: CheckStatus::Pass,
            message: "macOS uses D3DMetal for DirectX translation (DXVK not needed)".into(),
            fix_available: false,
            fix_description: None,
        };
    }

    let system32 = bottle.drive_c().join("windows").join("system32");

    let dxvk_dlls = ["d3d11.dll", "d3d10core.dll", "dxgi.dll"];
    let found: Vec<&str> = dxvk_dlls
        .iter()
        .filter(|dll| system32.join(dll).exists())
        .copied()
        .collect();

    if found.len() == dxvk_dlls.len() {
        // Multi-signal detection: check for DXVK signature in binary
        let has_dxvk_string = fs::read(system32.join("dxgi.dll"))
            .ok()
            .map(|bytes| {
                let check_len = bytes.len().min(4096);
                bytes[..check_len]
                    .windows(4)
                    .any(|w| w == b"dxvk" || w == b"DXVK")
            })
            .unwrap_or(false);

        // Also check DLL overrides for native (indicates DXVK is configured)
        let has_native_override = fs::read_to_string(bottle.path.join("user.reg"))
            .ok()
            .map(|content| {
                let overrides = parse_dll_overrides(&content);
                overrides.get("dxgi").is_some_and(|v| v.contains("native"))
            })
            .unwrap_or(false);

        if has_dxvk_string || has_native_override {
            DiagnosticCheck {
                name: "DXVK".into(),
                category: "Graphics".into(),
                status: CheckStatus::Pass,
                message: "DXVK detected and configured".into(),
                fix_available: false,
                fix_description: None,
            }
        } else {
            DiagnosticCheck {
                name: "DXVK".into(),
                category: "Graphics".into(),
                status: CheckStatus::Warning,
                message: "DirectX DLLs present but DXVK could not be confirmed — Wine's built-in D3D translation may be active".into(),
                fix_available: false,
                fix_description: Some("Install DXVK through your Wine manager for better performance".into()),
            }
        }
    } else if found.is_empty() {
        DiagnosticCheck {
            name: "DXVK".into(),
            category: "Graphics".into(),
            status: CheckStatus::Warning,
            message: "No DXVK DLLs found — game may use Wine's built-in D3D translation".into(),
            fix_available: false,
            fix_description: Some("Install DXVK for better graphics performance".into()),
        }
    } else {
        DiagnosticCheck {
            name: "DXVK".into(),
            category: "Graphics".into(),
            status: CheckStatus::Warning,
            message: format!(
                "Partial DXVK installation ({}/{})",
                found.len(),
                dxvk_dlls.len()
            ),
            fix_available: false,
            fix_description: Some("Reinstall DXVK to ensure all DLLs are present".into()),
        }
    }
}

/// Check for required DLL overrides (especially for SKSE).
fn check_dll_overrides(bottle: &Bottle, game_id: &str) -> DiagnosticCheck {
    if game_id != "skyrimse" && game_id != "skyrim" {
        return DiagnosticCheck {
            name: "DLL Overrides".into(),
            category: "Configuration".into(),
            status: CheckStatus::Skipped,
            message: "DLL override check only applies to Skyrim".into(),
            fix_available: false,
            fix_description: None,
        };
    }

    // Parse overrides from BOTH user.reg and system.reg
    let mut all_overrides = HashMap::new();
    for reg_file in &["user.reg", "system.reg"] {
        let reg_path = bottle.path.join(reg_file);
        if let Ok(content) = fs::read_to_string(&reg_path) {
            for (k, v) in parse_dll_overrides(&content) {
                all_overrides.entry(k).or_insert(v);
            }
        }
    }

    let system32 = bottle.drive_c().join("windows").join("system32");
    let needed = vec!["xaudio2_7"];
    let mut missing = Vec::new();

    for dll in &needed {
        let has_override = all_overrides.contains_key(*dll);
        let has_builtin = system32.join(format!("{}.dll", dll)).exists();
        // Pass if either an explicit override exists or the DLL is present as a built-in
        if !has_override && !has_builtin {
            missing.push(*dll);
        }
    }

    if missing.is_empty() {
        DiagnosticCheck {
            name: "DLL Overrides".into(),
            category: "Configuration".into(),
            status: CheckStatus::Pass,
            message: "Required DLL overrides are configured or built-in".into(),
            fix_available: false,
            fix_description: None,
        }
    } else {
        DiagnosticCheck {
            name: "DLL Overrides".into(),
            category: "Configuration".into(),
            status: CheckStatus::Warning,
            message: format!("Missing DLL overrides: {}", missing.join(", ")),
            fix_available: true,
            fix_description: Some(
                "Click Fix to add the required DLL overrides to your Wine bottle".into(),
            ),
        }
    }
}

/// Write a DLL override entry into the bottle's user.reg file.
pub fn fix_dll_override(
    bottle: &Bottle,
    dll_name: &str,
    override_type: &str,
) -> std::io::Result<()> {
    let user_reg = bottle.path.join("user.reg");
    let mut content = fs::read_to_string(&user_reg).unwrap_or_default();

    let section = "[Software\\\\Wine\\\\DllOverrides]";
    let entry = format!("\"{}\"=\"{}\"", dll_name, override_type);

    if let Some(pos) = content.find(section) {
        // Find the end of the section header line
        let after_section = pos + section.len();
        if let Some(newline) = content[after_section..].find('\n') {
            let insert_pos = after_section + newline + 1;
            content.insert_str(insert_pos, &format!("{}\n", entry));
        }
    } else {
        // Create the section at the end
        content.push_str(&format!("\n{}\n{}\n", section, entry));
    }

    // Atomic write: temp + rename
    let tmp = user_reg.with_extension("reg.tmp");
    fs::write(&tmp, &content)?;
    fs::rename(&tmp, &user_reg)?;
    Ok(())
}

/// Check for Retina/HiDPI display mode (macOS only).
fn check_retina_mode(bottle: &Bottle) -> DiagnosticCheck {
    if !cfg!(target_os = "macos") {
        return DiagnosticCheck {
            name: "Retina Display".into(),
            category: "Display".into(),
            status: CheckStatus::Skipped,
            message: "Retina check only applies to macOS".into(),
            fix_available: false,
            fix_description: None,
        };
    }

    let user_reg = bottle.path.join("user.reg");
    let content = fs::read_to_string(&user_reg).unwrap_or_default();

    let has_retina = content.contains("\"RetinaMode\"=\"Y\"");

    if has_retina {
        DiagnosticCheck {
            name: "Retina Display".into(),
            category: "Display".into(),
            status: CheckStatus::Pass,
            message: "Retina mode enabled — game renders at native resolution".into(),
            fix_available: false,
            fix_description: None,
        }
    } else {
        DiagnosticCheck {
            name: "Retina Display".into(),
            category: "Display".into(),
            status: CheckStatus::Warning,
            message:
                "Retina mode not enabled — game may appear zoomed in or blurry on HiDPI displays"
                    .into(),
            fix_available: true,
            fix_description: Some(
                "Click Fix to enable Retina mode for native resolution rendering".into(),
            ),
        }
    }
}

/// Enable Retina/HiDPI mode in the bottle's Wine registry.
pub fn fix_retina_mode(bottle: &Bottle) -> std::io::Result<()> {
    let user_reg = bottle.path.join("user.reg");
    let mut content = fs::read_to_string(&user_reg).unwrap_or_default();

    let section = "[Software\\\\Wine\\\\Mac Driver]";
    let entry = "\"RetinaMode\"=\"Y\"";

    if content.contains(section) {
        // Section exists — check if RetinaMode line is there
        if content.contains("\"RetinaMode\"=") {
            // Replace existing value
            content = content.replace("\"RetinaMode\"=\"N\"", entry);
        } else {
            // Add entry after section header
            if let Some(pos) = content.find(section) {
                let after_section = pos + section.len();
                if let Some(newline) = content[after_section..].find('\n') {
                    let insert_pos = after_section + newline + 1;
                    content.insert_str(insert_pos, &format!("{}\n", entry));
                }
            }
        }
    } else {
        // Create the section at the end
        content.push_str(&format!("\n{}\n{}\n", section, entry));
    }

    // Atomic write: temp + rename
    let tmp = user_reg.with_extension("reg.tmp");
    fs::write(&tmp, &content)?;
    fs::rename(&tmp, &user_reg)?;
    Ok(())
}

/// Parse DLL overrides from user.reg content.
pub fn parse_dll_overrides(user_reg: &str) -> HashMap<String, String> {
    let mut overrides = HashMap::new();
    let mut in_override_section = false;

    for line in user_reg.lines() {
        let trimmed = line.trim();

        if trimmed.contains("[Software\\\\Wine\\\\DllOverrides]") {
            in_override_section = true;
            continue;
        }

        if in_override_section {
            if trimmed.starts_with('[') {
                break; // Next section
            }

            // Parse "dllname"="override_type"
            if trimmed.starts_with('"') {
                let parts: Vec<&str> = trimmed.splitn(2, '=').collect();
                if parts.len() == 2 {
                    let key = parts[0].trim_matches('"');
                    let val = parts[1].trim_matches('"');
                    overrides.insert(key.to_string(), val.to_string());
                }
            }
        }
    }

    overrides
}

/// Check for Visual C++ redistributable files.
fn check_vcredist(bottle: &Bottle) -> DiagnosticCheck {
    let system32 = bottle.drive_c().join("windows").join("system32");

    // Key MSVC runtime DLLs
    let vcredist_dlls = ["msvcp140.dll", "vcruntime140.dll", "vcruntime140_1.dll"];

    let found: Vec<&str> = vcredist_dlls
        .iter()
        .filter(|dll| system32.join(dll).exists())
        .copied()
        .collect();

    if found.len() >= 2 {
        DiagnosticCheck {
            name: "Visual C++ Runtime".into(),
            category: "Runtime".into(),
            status: CheckStatus::Pass,
            message: format!(
                "{}/{} VC++ runtime DLLs present",
                found.len(),
                vcredist_dlls.len()
            ),
            fix_available: false,
            fix_description: None,
        }
    } else {
        DiagnosticCheck {
            name: "Visual C++ Runtime".into(),
            category: "Runtime".into(),
            status: CheckStatus::Warning,
            message: format!(
                "Missing VC++ runtime DLLs ({}/{}). Some mods may not work.",
                found.len(),
                vcredist_dlls.len()
            ),
            fix_available: true,
            fix_description: Some(
                "Install Visual C++ Redistributable through your Wine manager".into(),
            ),
        }
    }
}

/// Check for .NET framework presence.
fn check_dotnet(bottle: &Bottle) -> DiagnosticCheck {
    let dotnet_dir = bottle
        .drive_c()
        .join("windows")
        .join("Microsoft.NET")
        .join("Framework");

    if dotnet_dir.exists() {
        let versions: Vec<String> = fs::read_dir(&dotnet_dir)
            .ok()
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|e| e.path().is_dir())
                    .filter_map(|e| {
                        e.file_name()
                            .to_str()
                            .filter(|n| n.starts_with('v'))
                            .map(|n| n.to_string())
                    })
                    .collect()
            })
            .unwrap_or_default();

        if versions.is_empty() {
            DiagnosticCheck {
                name: ".NET Framework".into(),
                category: "Runtime".into(),
                status: CheckStatus::Warning,
                message: ".NET directory exists but no versions found".into(),
                fix_available: false,
                fix_description: None,
            }
        } else {
            DiagnosticCheck {
                name: ".NET Framework".into(),
                category: "Runtime".into(),
                status: CheckStatus::Pass,
                message: format!(".NET versions: {}", versions.join(", ")),
                fix_available: false,
                fix_description: None,
            }
        }
    } else {
        DiagnosticCheck {
            name: ".NET Framework".into(),
            category: "Runtime".into(),
            status: CheckStatus::Skipped,
            message: ".NET Framework directory not found (may not be needed)".into(),
            fix_available: false,
            fix_description: None,
        }
    }
}

/// Check Windows version reported by the bottle.
fn check_windows_version(bottle: &Bottle) -> DiagnosticCheck {
    let system_reg = bottle.path.join("system.reg");
    if !system_reg.exists() {
        return DiagnosticCheck {
            name: "Windows Version".into(),
            category: "Configuration".into(),
            status: CheckStatus::Skipped,
            message: "system.reg not found".into(),
            fix_available: false,
            fix_description: None,
        };
    }

    let content = match fs::read_to_string(&system_reg) {
        Ok(c) => c,
        Err(_) => {
            return DiagnosticCheck {
                name: "Windows Version".into(),
                category: "Configuration".into(),
                status: CheckStatus::Warning,
                message: "Cannot read system.reg".into(),
                fix_available: false,
                fix_description: None,
            };
        }
    };

    // Look for CurrentVersion or ProductName
    let version = extract_windows_version(&content);

    DiagnosticCheck {
        name: "Windows Version".into(),
        category: "Configuration".into(),
        status: CheckStatus::Pass,
        message: format!("Reported version: {}", version),
        fix_available: false,
        fix_description: None,
    }
}

/// Extract Windows version from system.reg content.
pub fn extract_windows_version(system_reg: &str) -> String {
    for line in system_reg.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("\"ProductName\"=") {
            return trimmed
                .split('=')
                .nth(1)
                .unwrap_or("Unknown")
                .trim_matches('"')
                .to_string();
        }
        if trimmed.starts_with("\"CurrentVersion\"=") {
            let ver = trimmed.split('=').nth(1).unwrap_or("").trim_matches('"');
            return match ver {
                "10.0" => "Windows 10".to_string(),
                "6.3" => "Windows 8.1".to_string(),
                "6.1" => "Windows 7".to_string(),
                v => format!("Windows (version {})", v),
            };
        }
    }
    "Unknown".to_string()
}

/// Check user directory structure.
fn check_user_dirs(bottle: &Bottle) -> DiagnosticCheck {
    let users = bottle.users_dir();
    if !users.exists() {
        return DiagnosticCheck {
            name: "User Directories".into(),
            category: "Structure".into(),
            status: CheckStatus::Warning,
            message: "Users directory not found".into(),
            fix_available: false,
            fix_description: None,
        };
    }

    let user_count = fs::read_dir(&users)
        .ok()
        .map(|entries| entries.flatten().filter(|e| e.path().is_dir()).count())
        .unwrap_or(0);

    DiagnosticCheck {
        name: "User Directories".into(),
        category: "Structure".into(),
        status: CheckStatus::Pass,
        message: format!("{} user directories found", user_count),
        fix_available: false,
        fix_description: None,
    }
}

/// Attempt to fix the AppData directory structure.
pub fn fix_appdata(bottle: &Bottle) -> std::io::Result<()> {
    let appdata = bottle.appdata_local();
    fs::create_dir_all(&appdata)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::TempDir;

    fn test_bottle(dir: &Path) -> Bottle {
        Bottle {
            name: "TestBottle".to_string(),
            path: dir.to_path_buf(),
            source: "Test".to_string(),
        }
    }

    fn setup_bottle(tmp: &TempDir) -> Bottle {
        let drive_c = tmp.path().join("drive_c");
        fs::create_dir_all(drive_c.join("windows").join("system32")).unwrap();
        fs::create_dir_all(
            drive_c
                .join("users")
                .join("testuser")
                .join("AppData")
                .join("Local"),
        )
        .unwrap();
        test_bottle(tmp.path())
    }

    #[test]
    fn check_drive_c_exists() {
        let tmp = TempDir::new().unwrap();
        let bottle = setup_bottle(&tmp);
        let check = check_drive_c(&bottle);
        assert_eq!(check.status, CheckStatus::Pass);
    }

    #[test]
    fn check_drive_c_missing() {
        let tmp = TempDir::new().unwrap();
        let bottle = test_bottle(tmp.path());
        let check = check_drive_c(&bottle);
        assert_eq!(check.status, CheckStatus::Error);
    }

    #[test]
    fn check_drive_c_message_contains_path() {
        let tmp = TempDir::new().unwrap();
        let bottle = setup_bottle(&tmp);
        let check = check_drive_c(&bottle);
        assert!(check.message.contains("drive_c"));
    }

    #[test]
    fn check_drive_c_no_fix_available() {
        let tmp = TempDir::new().unwrap();
        let bottle = test_bottle(tmp.path());
        let check = check_drive_c(&bottle);
        assert!(!check.fix_available);
    }

    #[test]
    fn check_appdata_exists() {
        let tmp = TempDir::new().unwrap();
        let bottle = setup_bottle(&tmp);
        let check = check_appdata(&bottle);
        assert_eq!(check.status, CheckStatus::Pass);
    }

    #[test]
    fn check_appdata_missing() {
        let tmp = TempDir::new().unwrap();
        let bottle = test_bottle(tmp.path());
        // Create drive_c but not AppData
        fs::create_dir_all(tmp.path().join("drive_c").join("users")).unwrap();
        let check = check_appdata(&bottle);
        assert_eq!(check.status, CheckStatus::Warning);
    }

    #[test]
    fn check_appdata_missing_has_fix() {
        let tmp = TempDir::new().unwrap();
        let bottle = test_bottle(tmp.path());
        fs::create_dir_all(tmp.path().join("drive_c").join("users")).unwrap();
        let check = check_appdata(&bottle);
        assert!(check.fix_available);
    }

    #[test]
    fn check_appdata_pass_no_fix() {
        let tmp = TempDir::new().unwrap();
        let bottle = setup_bottle(&tmp);
        let check = check_appdata(&bottle);
        assert!(!check.fix_available);
    }

    #[test]
    fn check_vcredist_present() {
        let tmp = TempDir::new().unwrap();
        let bottle = setup_bottle(&tmp);
        let sys32 = tmp.path().join("drive_c").join("windows").join("system32");
        fs::write(sys32.join("msvcp140.dll"), "fake").unwrap();
        fs::write(sys32.join("vcruntime140.dll"), "fake").unwrap();
        let check = check_vcredist(&bottle);
        assert_eq!(check.status, CheckStatus::Pass);
    }

    #[test]
    fn check_vcredist_missing() {
        let tmp = TempDir::new().unwrap();
        let bottle = setup_bottle(&tmp);
        let check = check_vcredist(&bottle);
        assert_eq!(check.status, CheckStatus::Warning);
    }

    #[test]
    fn check_vcredist_partial() {
        let tmp = TempDir::new().unwrap();
        let bottle = setup_bottle(&tmp);
        let sys32 = tmp.path().join("drive_c").join("windows").join("system32");
        fs::write(sys32.join("msvcp140.dll"), "fake").unwrap();
        let check = check_vcredist(&bottle);
        assert_eq!(check.status, CheckStatus::Warning);
    }

    #[test]
    fn check_vcredist_missing_has_fix() {
        let tmp = TempDir::new().unwrap();
        let bottle = setup_bottle(&tmp);
        let check = check_vcredist(&bottle);
        assert!(check.fix_available);
    }

    #[test]
    fn parse_dll_overrides_basic() {
        let content = r#"[Software\\Wine\\DllOverrides]
"xaudio2_7"="native"
"d3d11"="native,builtin"
[Next\\Section]
"other"="value"
"#;
        let overrides = parse_dll_overrides(content);
        assert_eq!(overrides.len(), 2);
        assert_eq!(overrides.get("xaudio2_7"), Some(&"native".to_string()));
        assert_eq!(overrides.get("d3d11"), Some(&"native,builtin".to_string()));
    }

    #[test]
    fn parse_dll_overrides_empty() {
        let overrides = parse_dll_overrides("");
        assert!(overrides.is_empty());
    }

    #[test]
    fn parse_dll_overrides_no_section() {
        let content = "[Other\\Section]\n\"key\"=\"val\"\n";
        let overrides = parse_dll_overrides(content);
        assert!(overrides.is_empty());
    }

    #[test]
    fn parse_dll_overrides_section_only() {
        let content = "[Software\\\\Wine\\\\DllOverrides]\n[Next]\n";
        let overrides = parse_dll_overrides(content);
        assert!(overrides.is_empty());
    }

    #[test]
    fn extract_windows_version_win10() {
        let reg = "\"CurrentVersion\"=\"10.0\"\n";
        assert_eq!(extract_windows_version(reg), "Windows 10");
    }

    #[test]
    fn extract_windows_version_win7() {
        let reg = "\"CurrentVersion\"=\"6.1\"\n";
        assert_eq!(extract_windows_version(reg), "Windows 7");
    }

    #[test]
    fn extract_windows_version_product_name() {
        let reg = "\"ProductName\"=\"Windows 11 Pro\"\n";
        assert_eq!(extract_windows_version(reg), "Windows 11 Pro");
    }

    #[test]
    fn extract_windows_version_unknown() {
        let reg = "something unrelated\n";
        assert_eq!(extract_windows_version(reg), "Unknown");
    }

    #[test]
    fn run_diagnostics_returns_all_checks() {
        let tmp = TempDir::new().unwrap();
        let bottle = setup_bottle(&tmp);
        let result = run_diagnostics(&bottle, "skyrimse");
        assert_eq!(result.checks.len(), 9);
        assert_eq!(
            result.passed
                + result.warnings
                + result.errors
                + result
                    .checks
                    .iter()
                    .filter(|c| c.status == CheckStatus::Skipped)
                    .count(),
            9
        );
    }

    #[test]
    fn run_diagnostics_counts_correct() {
        let tmp = TempDir::new().unwrap();
        let bottle = setup_bottle(&tmp);
        let result = run_diagnostics(&bottle, "skyrimse");
        let manual_passed = result
            .checks
            .iter()
            .filter(|c| c.status == CheckStatus::Pass)
            .count();
        assert_eq!(result.passed, manual_passed);
    }

    #[test]
    fn run_diagnostics_non_skyrim() {
        let tmp = TempDir::new().unwrap();
        let bottle = setup_bottle(&tmp);
        let result = run_diagnostics(&bottle, "fallout4");
        // DLL overrides should be skipped for non-Skyrim
        let dll_check = result
            .checks
            .iter()
            .find(|c| c.name == "DLL Overrides")
            .unwrap();
        assert_eq!(dll_check.status, CheckStatus::Skipped);
    }

    #[test]
    fn run_diagnostics_broken_bottle() {
        let tmp = TempDir::new().unwrap();
        let bottle = test_bottle(tmp.path());
        let result = run_diagnostics(&bottle, "skyrimse");
        assert!(result.errors > 0 || result.warnings > 0);
    }

    #[test]
    fn fix_appdata_creates_dir() {
        let tmp = TempDir::new().unwrap();
        let bottle = test_bottle(tmp.path());
        fs::create_dir_all(tmp.path().join("drive_c").join("users")).unwrap();
        fix_appdata(&bottle).unwrap();
        assert!(bottle.appdata_local().exists());
    }

    #[test]
    fn fix_appdata_idempotent() {
        let tmp = TempDir::new().unwrap();
        let bottle = setup_bottle(&tmp);
        // Should not fail if already exists
        fix_appdata(&bottle).unwrap();
        assert!(bottle.appdata_local().exists());
    }

    #[test]
    fn check_dxvk_no_dlls() {
        let tmp = TempDir::new().unwrap();
        let bottle = setup_bottle(&tmp);
        let check = check_dxvk(&bottle);
        if cfg!(target_os = "macos") {
            // macOS uses D3DMetal — DXVK check always passes
            assert_eq!(check.status, CheckStatus::Pass);
        } else {
            assert_eq!(check.status, CheckStatus::Warning);
        }
    }

    #[test]
    fn check_dxvk_all_dlls_large() {
        let tmp = TempDir::new().unwrap();
        let bottle = setup_bottle(&tmp);
        let sys32 = tmp.path().join("drive_c").join("windows").join("system32");
        // Create fake DXVK DLLs with signature so multi-signal detection works
        let mut big_data = vec![0u8; 600_000];
        big_data[100..104].copy_from_slice(b"dxvk");
        fs::write(sys32.join("d3d11.dll"), &big_data).unwrap();
        fs::write(sys32.join("d3d10core.dll"), &big_data).unwrap();
        fs::write(sys32.join("dxgi.dll"), &big_data).unwrap();
        let check = check_dxvk(&bottle);
        assert_eq!(check.status, CheckStatus::Pass);
    }

    #[test]
    fn check_user_dirs_present() {
        let tmp = TempDir::new().unwrap();
        let bottle = setup_bottle(&tmp);
        let check = check_user_dirs(&bottle);
        assert_eq!(check.status, CheckStatus::Pass);
    }

    #[test]
    fn check_user_dirs_missing() {
        let tmp = TempDir::new().unwrap();
        let bottle = test_bottle(tmp.path());
        let check = check_user_dirs(&bottle);
        assert_eq!(check.status, CheckStatus::Warning);
    }

    #[test]
    fn fix_appdata_returns_ok_on_success() {
        let tmp = TempDir::new().unwrap();
        let bottle = test_bottle(tmp.path());
        // Create enough structure so create_dir_all can succeed
        fs::create_dir_all(tmp.path().join("drive_c").join("users")).unwrap();
        let result = fix_appdata(&bottle);
        assert!(result.is_ok());
    }

    #[test]
    fn fix_appdata_creates_nested_structure() {
        let tmp = TempDir::new().unwrap();
        let bottle = test_bottle(tmp.path());
        // Start with only the bottle root -- no drive_c at all
        fix_appdata(&bottle).unwrap();
        // appdata_local() falls back to drive_c/users/crossover/AppData/Local
        // when no user dirs exist, and fix_appdata creates that full path
        let appdata = bottle.appdata_local();
        assert!(appdata.exists());
        assert!(appdata.is_dir());
    }

    #[test]
    fn check_dxvk_some_dlls() {
        let tmp = TempDir::new().unwrap();
        let bottle = setup_bottle(&tmp);
        let sys32 = tmp.path().join("drive_c").join("windows").join("system32");
        // Only create 1 of the 3 required DXVK DLLs (partial install)
        let big_data = vec![0u8; 600_000];
        fs::write(sys32.join("d3d11.dll"), &big_data).unwrap();
        let check = check_dxvk(&bottle);
        if cfg!(target_os = "macos") {
            assert_eq!(check.status, CheckStatus::Pass);
        } else {
            assert_eq!(check.status, CheckStatus::Warning);
            assert!(check.message.contains("Partial"));
        }
    }

    #[test]
    fn check_dxvk_status_is_pass_when_present() {
        let tmp = TempDir::new().unwrap();
        let bottle = setup_bottle(&tmp);
        let sys32 = tmp.path().join("drive_c").join("windows").join("system32");
        // Create all 3 DXVK DLLs with DXVK signature for multi-signal detection
        let mut big_data = vec![0u8; 700_000];
        big_data[100..104].copy_from_slice(b"dxvk");
        fs::write(sys32.join("d3d11.dll"), &big_data).unwrap();
        fs::write(sys32.join("d3d10core.dll"), &big_data).unwrap();
        fs::write(sys32.join("dxgi.dll"), &big_data).unwrap();
        let check = check_dxvk(&bottle);
        assert_eq!(check.status, CheckStatus::Pass);
        if cfg!(target_os = "macos") {
            assert_eq!(check.name, "Graphics Translation");
        } else {
            assert_eq!(check.name, "DXVK");
        }
        assert!(!check.fix_available);
    }

    #[test]
    fn check_user_dirs_partial() {
        let tmp = TempDir::new().unwrap();
        let bottle = test_bottle(tmp.path());
        // Create users dir with some subdirectories but not the full setup
        let users = tmp.path().join("drive_c").join("users");
        fs::create_dir_all(users.join("alice")).unwrap();
        let check = check_user_dirs(&bottle);
        // Users dir exists so status should be Pass (function only checks existence + counts)
        assert_eq!(check.status, CheckStatus::Pass);
        assert!(check.message.contains("1"));
    }

    #[test]
    fn check_user_dirs_status_pass_when_all_present() {
        let tmp = TempDir::new().unwrap();
        let bottle = test_bottle(tmp.path());
        let users = tmp.path().join("drive_c").join("users");
        fs::create_dir_all(users.join("alice")).unwrap();
        fs::create_dir_all(users.join("bob")).unwrap();
        fs::create_dir_all(users.join("Public")).unwrap();
        let check = check_user_dirs(&bottle);
        assert_eq!(check.status, CheckStatus::Pass);
        assert!(check.message.contains("3"));
        assert!(!check.fix_available);
    }
}
