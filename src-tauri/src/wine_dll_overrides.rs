//! Wine DLL override management for modding tools.
//!
//! xEdit, BodySlide, Nemesis, Pandora, and other modding tools require specific
//! DLL overrides to work under Wine. This module manages those overrides by
//! writing to the Wine registry (`user.reg`).

use log::info;
use std::collections::HashMap;
use std::path::Path;

/// DLL override mode in Wine.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum DllOverride {
    /// Use native (Windows) DLL only
    Native,
    /// Use builtin (Wine) DLL only
    Builtin,
    /// Try native first, fall back to builtin
    NativeBuiltin,
    /// Try builtin first, fall back to native
    BuiltinNative,
    /// Disable the DLL
    Disabled,
}

impl DllOverride {
    fn to_wine_value(&self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Builtin => "builtin",
            Self::NativeBuiltin => "native,builtin",
            Self::BuiltinNative => "builtin,native",
            Self::Disabled => "",
        }
    }
}

/// Get the required DLL overrides for a modding tool.
///
/// Returns a map of DLL name -> override mode.
pub fn get_tool_overrides(tool_name: &str) -> HashMap<String, DllOverride> {
    let lower = tool_name.to_lowercase();
    let mut overrides = HashMap::new();

    if lower.contains("xedit")
        || lower.contains("sseedit")
        || lower.contains("fo4edit")
        || lower.contains("tes5edit")
        || lower.contains("fnvedit")
        || lower.contains("fo3edit")
    {
        // xEdit variants: need native file browser for open/save dialogs
        overrides.insert("comdlg32".to_string(), DllOverride::Native);
        overrides.insert("ole32".to_string(), DllOverride::Native);
        overrides.insert("oleaut32".to_string(), DllOverride::Native);
        overrides.insert("shell32".to_string(), DllOverride::Native);
    }

    if lower.contains("bodyslide") || lower.contains("outfitstudio") {
        // BodySlide/Outfit Studio: D3D11 native for rendering
        overrides.insert("d3d11".to_string(), DllOverride::Native);
        overrides.insert("dxgi".to_string(), DllOverride::Native);
    }

    if lower.contains("nemesis") {
        // Nemesis Unlimited Behavior Engine
        overrides.insert("mscoree".to_string(), DllOverride::Native);
        overrides.insert("mscorwks".to_string(), DllOverride::Native);
    }

    if lower.contains("pandora") {
        // Pandora Behavior Engine Plus
        overrides.insert("mscoree".to_string(), DllOverride::Native);
    }

    if lower.contains("synthesis") || lower.contains("mutagen") {
        // Synthesis patcher (.NET)
        overrides.insert("mscoree".to_string(), DllOverride::Native);
    }

    if lower.contains("loot") {
        // LOOT: needs native shell for browser launch
        overrides.insert("shell32".to_string(), DllOverride::Native);
    }

    if lower.contains("cathedral") && lower.contains("assets") {
        // Cathedral Assets Optimizer
        overrides.insert("d3d11".to_string(), DllOverride::Native);
        overrides.insert("dxgi".to_string(), DllOverride::Native);
    }

    overrides
}

/// Apply DLL overrides to a Wine prefix registry.
///
/// Writes to `user.reg` in the Wine prefix, under the
/// `[Software\\Wine\\DllOverrides]` section.
pub fn apply_overrides(
    prefix_path: &Path,
    overrides: &HashMap<String, DllOverride>,
) -> Result<(), String> {
    if overrides.is_empty() {
        return Ok(());
    }

    let user_reg = prefix_path.join("user.reg");
    if !user_reg.exists() {
        return Err(format!("Wine user.reg not found at {}", user_reg.display()));
    }

    let content = std::fs::read_to_string(&user_reg)
        .map_err(|e| format!("Failed to read user.reg: {}", e))?;

    let section_header = "[Software\\\\Wine\\\\DllOverrides]";
    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();

    // Find or create the DllOverrides section
    let section_idx = lines.iter().position(|l| l.starts_with(section_header));

    if let Some(idx) = section_idx {
        // Section exists — find its extent (until next section or EOF)
        let section_end = lines[idx + 1..]
            .iter()
            .position(|l| l.starts_with('['))
            .map(|p| p + idx + 1)
            .unwrap_or(lines.len());

        // Collect new entries to insert (avoids stale index from mid-loop inserts)
        let mut new_entries: Vec<String> = Vec::new();

        for (dll, mode) in overrides {
            let key = format!("\"*{}\"", dll);
            let value = format!("\"{}\"", mode.to_wine_value());
            let entry = format!("{}={}", key, value);

            // Check if this DLL already has an override in the section
            let existing = lines[idx + 1..section_end]
                .iter()
                .position(|l| l.starts_with(&key));

            if let Some(existing_idx) = existing {
                lines[idx + 1 + existing_idx] = entry;
                info!("Updated DLL override: {} = {}", dll, mode.to_wine_value());
            } else {
                new_entries.push(entry);
                info!("Added DLL override: {} = {}", dll, mode.to_wine_value());
            }
        }

        // Batch-insert all new entries at section end (avoids stale index bug)
        for (i, entry) in new_entries.into_iter().enumerate() {
            lines.insert(section_end + i, entry);
        }
    } else {
        // Section doesn't exist — create it
        lines.push(String::new());
        lines.push(section_header.to_string());
        // Sort keys for deterministic output
        let mut sorted: Vec<_> = overrides.iter().collect();
        sorted.sort_by_key(|(k, _)| (*k).clone());
        for (dll, mode) in sorted {
            let entry = format!("\"*{}\"=\"{}\"", dll, mode.to_wine_value());
            lines.push(entry);
            info!("Added DLL override: {} = {}", dll, mode.to_wine_value());
        }
    }

    // Atomic write: write to temp file then rename (prevents corruption on crash)
    let tmp_path = user_reg.with_extension("reg.tmp");
    let output = lines.join("\n");
    std::fs::write(&tmp_path, &output)
        .map_err(|e| format!("Failed to write temp user.reg: {}", e))?;
    std::fs::rename(&tmp_path, &user_reg)
        .map_err(|e| format!("Failed to rename temp user.reg: {}", e))?;

    Ok(())
}

/// Remove DLL overrides from a Wine prefix registry.
pub fn remove_overrides(prefix_path: &Path, dll_names: &[&str]) -> Result<(), String> {
    let user_reg = prefix_path.join("user.reg");
    if !user_reg.exists() {
        return Ok(()); // Nothing to remove
    }

    let content = std::fs::read_to_string(&user_reg)
        .map_err(|e| format!("Failed to read user.reg: {}", e))?;

    let section_header = "[Software\\\\Wine\\\\DllOverrides]";
    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();

    // Only remove overrides within the DllOverrides section (not the entire file)
    if let Some(section_start) = lines.iter().position(|l| l.starts_with(section_header)) {
        let section_end = lines[section_start + 1..]
            .iter()
            .position(|l| l.starts_with('['))
            .map(|p| p + section_start + 1)
            .unwrap_or(lines.len());

        let keys: Vec<String> = dll_names
            .iter()
            .map(|dll| format!("\"*{}\"", dll))
            .collect();
        // Remove matching lines within section bounds only (iterate in reverse to keep indices stable)
        for i in (section_start + 1..section_end).rev() {
            if keys.iter().any(|k| lines[i].starts_with(k)) {
                lines.remove(i);
            }
        }
    }

    let output = lines.join("\n");
    std::fs::write(&user_reg, output).map_err(|e| format!("Failed to write user.reg: {}", e))?;

    Ok(())
}

/// Auto-detect tool type from executable name and return appropriate overrides.
pub fn detect_and_get_overrides(exe_path: &Path) -> HashMap<String, DllOverride> {
    let name = exe_path
        .file_stem()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    get_tool_overrides(&name)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xedit_overrides() {
        let overrides = get_tool_overrides("SSEEdit.exe");
        assert!(overrides.contains_key("comdlg32"));
        assert_eq!(overrides["comdlg32"], DllOverride::Native);
    }

    #[test]
    fn test_bodyslide_overrides() {
        let overrides = get_tool_overrides("BodySlide.exe");
        assert!(overrides.contains_key("d3d11"));
        assert_eq!(overrides["d3d11"], DllOverride::Native);
    }

    #[test]
    fn test_nemesis_overrides() {
        let overrides = get_tool_overrides("Nemesis Unlimited Behavior Engine.exe");
        assert!(overrides.contains_key("mscoree"));
    }

    #[test]
    fn test_unknown_tool_no_overrides() {
        let overrides = get_tool_overrides("random_tool.exe");
        assert!(overrides.is_empty());
    }

    #[test]
    fn test_dll_override_values() {
        assert_eq!(DllOverride::Native.to_wine_value(), "native");
        assert_eq!(DllOverride::Builtin.to_wine_value(), "builtin");
        assert_eq!(DllOverride::NativeBuiltin.to_wine_value(), "native,builtin");
    }

    #[test]
    fn test_detect_from_path() {
        let overrides = detect_and_get_overrides(Path::new("/some/path/FO4Edit.exe"));
        assert!(overrides.contains_key("comdlg32"));
    }
}
