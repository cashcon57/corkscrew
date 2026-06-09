//! Pre-launch fix system: data-driven Wine tweaks applied before game launch.
//!
//! Inspired by umu-protonfixes, this module provides a per-game + per-modlist
//! fix database that auto-applies env vars, DLL overrides, and Wine registry
//! patches before launching a modded game.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// The kind of fix to apply before game launch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FixType {
    /// Set an environment variable.
    EnvVar,
    /// Override a Wine DLL (native, builtin, native+builtin, etc.).
    DllOverride,
    /// Set a Wine registry value.
    RegistryPatch,
}

/// Where the fix came from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FixSource {
    /// Shipped with Corkscrew binary.
    Builtin,
    /// User-defined override.
    User,
}

/// A single pre-launch fix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchFix {
    /// Corkscrew game_id (e.g., "skyrimse").
    pub game_id: String,
    /// Optional modlist name — `None` means "all modlists for this game".
    pub modlist_name: Option<String>,
    /// Type of fix.
    pub fix_type: FixType,
    /// Key: env var name, DLL name, or registry key path.
    pub key: String,
    /// Value: env var value, DLL override mode, or registry value.
    pub value: String,
    /// Human-readable explanation of why this fix is needed.
    pub reason: String,
    /// Where this fix came from.
    pub source: FixSource,
    /// Whether this fix is currently enabled.
    pub enabled: bool,
}

/// Result of applying fixes before launch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppliedFixes {
    /// Environment variables that were set.
    pub env_vars: Vec<(String, String)>,
    /// DLL overrides that were applied.
    pub dll_overrides: Vec<(String, String)>,
    /// Registry patches that were applied.
    pub registry_patches: Vec<(String, String)>,
    /// Total number of fixes applied.
    pub total_applied: usize,
}

// ---------------------------------------------------------------------------
// Builtin fix database
// ---------------------------------------------------------------------------

/// Builtin fixes that ship with Corkscrew. These are compiled into the binary
/// and applied automatically based on game_id and modlist_name matching.
///
/// Modeled after umu-protonfixes: per-game fixes that address known Wine
/// compatibility issues with modded Bethesda games.
pub fn builtin_fixes() -> Vec<LaunchFix> {
    vec![
        // --- Skyrim SE: General Wine fixes ---
        LaunchFix {
            game_id: "skyrimse".into(),
            modlist_name: None,
            fix_type: FixType::EnvVar,
            key: "WINE_LARGE_ADDRESS_AWARE".into(),
            value: "1".into(),
            reason: "Prevents 32-bit address space exhaustion crashes with DXVK. \
                     Required for heavily modded setups (1000+ mods).".into(),
            source: FixSource::Builtin,
            enabled: true,
        },
        LaunchFix {
            game_id: "skyrimse".into(),
            modlist_name: None,
            fix_type: FixType::EnvVar,
            key: "DXVK_ASYNC".into(),
            value: "1".into(),
            reason: "Enables async shader compilation to eliminate shader stutter. \
                     Renders with fallback shader while compiling, avoiding micro-freezes.".into(),
            source: FixSource::Builtin,
            enabled: true,
        },

        // --- Fallout 4: General Wine fixes ---
        LaunchFix {
            game_id: "fallout4".into(),
            modlist_name: None,
            fix_type: FixType::EnvVar,
            key: "WINE_LARGE_ADDRESS_AWARE".into(),
            value: "1".into(),
            reason: "Prevents address space exhaustion crashes with DXVK on Fallout 4.".into(),
            source: FixSource::Builtin,
            enabled: true,
        },
        LaunchFix {
            game_id: "fallout4".into(),
            modlist_name: None,
            fix_type: FixType::EnvVar,
            key: "DXVK_ASYNC".into(),
            value: "1".into(),
            reason: "Async shader compilation for stutter-free Fallout 4.".into(),
            source: FixSource::Builtin,
            enabled: true,
        },

        // --- Skyrim SE + ENB: d3d11 native override ---
        LaunchFix {
            game_id: "skyrimse".into(),
            modlist_name: None,
            fix_type: FixType::DllOverride,
            key: "d3d11".into(),
            value: "native".into(),
            reason: "Required for ENBSeries to hook DirectX 11 correctly under Wine. \
                     Without this, ENB's d3d11.dll is ignored in favor of Wine's builtin.".into(),
            source: FixSource::Builtin,
            enabled: true,
        },
        LaunchFix {
            game_id: "skyrimse".into(),
            modlist_name: None,
            fix_type: FixType::DllOverride,
            key: "d3dcompiler_47".into(),
            value: "native".into(),
            reason: "Many SKSE plugins and ENB require the native d3dcompiler_47.dll \
                     for shader compilation. Wine's builtin version lacks features.".into(),
            source: FixSource::Builtin,
            enabled: true,
        },

        // --- Paralives: BepInEx script mods under Wine/Proton ---
        LaunchFix {
            game_id: "paralives".into(),
            modlist_name: None,
            fix_type: FixType::DllOverride,
            key: "winhttp".into(),
            value: "n,b".into(),
            reason: "BepInEx 5 uses UnityDoorstop's winhttp.dll proxy. Wine/Proton must prefer the game-local native winhttp.dll for script mods to load.".into(),
            source: FixSource::Builtin,
            enabled: true,
        },

        // --- Oblivion fixes ---
        LaunchFix {
            game_id: "oblivion".into(),
            modlist_name: None,
            fix_type: FixType::EnvVar,
            key: "WINE_LARGE_ADDRESS_AWARE".into(),
            value: "1".into(),
            reason: "Prevents memory exhaustion on heavily modded Oblivion.".into(),
            source: FixSource::Builtin,
            enabled: true,
        },
    ]
}

// ---------------------------------------------------------------------------
// Fix resolution
// ---------------------------------------------------------------------------

/// Resolve all applicable fixes for a game/modlist combination.
///
/// Merges builtin fixes with user overrides. User fixes take precedence
/// over builtin fixes with the same (game_id, fix_type, key) combination.
pub fn resolve_fixes(
    game_id: &str,
    modlist_name: Option<&str>,
    user_fixes: &[LaunchFix],
) -> Vec<LaunchFix> {
    let mut merged: HashMap<(String, String), LaunchFix> = HashMap::new();

    // First, collect applicable builtin fixes
    for fix in builtin_fixes() {
        if fix.game_id != game_id {
            continue;
        }
        // Match if fix has no modlist_name (applies to all) or matches the specific modlist
        if fix.modlist_name.is_some() && fix.modlist_name.as_deref() != modlist_name {
            continue;
        }
        let key = (format!("{:?}", fix.fix_type), fix.key.clone());
        merged.insert(key, fix);
    }

    // Then, layer user fixes on top (override builtins)
    for fix in user_fixes {
        if fix.game_id != game_id {
            continue;
        }
        if fix.modlist_name.is_some() && fix.modlist_name.as_deref() != modlist_name {
            continue;
        }
        let key = (format!("{:?}", fix.fix_type), fix.key.clone());
        merged.insert(key, fix.clone());
    }

    // Return only enabled fixes
    merged.into_values().filter(|f| f.enabled).collect()
}

/// Apply resolved fixes, returning env vars and DLL override strings
/// suitable for injection into the launch command.
pub fn apply_fixes(fixes: &[LaunchFix]) -> AppliedFixes {
    let mut env_vars = Vec::new();
    let mut dll_overrides = Vec::new();
    let mut registry_patches = Vec::new();

    for fix in fixes {
        match fix.fix_type {
            FixType::EnvVar => {
                env_vars.push((fix.key.clone(), fix.value.clone()));
            }
            FixType::DllOverride => {
                dll_overrides.push((fix.key.clone(), fix.value.clone()));
            }
            FixType::RegistryPatch => {
                registry_patches.push((fix.key.clone(), fix.value.clone()));
            }
        }
    }

    let total = env_vars.len() + dll_overrides.len() + registry_patches.len();

    AppliedFixes {
        env_vars,
        dll_overrides,
        registry_patches,
        total_applied: total,
    }
}

/// Build a `WINEDLLOVERRIDES` environment variable string from DLL overrides.
///
/// Format: `dll1=mode1;dll2=mode2`
/// If there's an existing WINEDLLOVERRIDES value, the new overrides are appended.
pub fn build_dll_override_env(overrides: &[(String, String)], existing: Option<&str>) -> String {
    let new_parts: Vec<String> = overrides
        .iter()
        .map(|(dll, mode)| format!("{}={}", dll, mode))
        .collect();

    if let Some(existing) = existing {
        if existing.is_empty() {
            new_parts.join(";")
        } else {
            format!("{};{}", existing, new_parts.join(";"))
        }
    } else {
        new_parts.join(";")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_fixes_cover_skyrimse() {
        let fixes = builtin_fixes();
        let skyrim_fixes: Vec<_> = fixes.iter().filter(|f| f.game_id == "skyrimse").collect();
        assert!(
            skyrim_fixes.len() >= 3,
            "Expected at least 3 Skyrim SE fixes"
        );

        // Check WINE_LARGE_ADDRESS_AWARE exists
        assert!(skyrim_fixes
            .iter()
            .any(|f| f.key == "WINE_LARGE_ADDRESS_AWARE"));
        assert!(skyrim_fixes.iter().any(|f| f.key == "DXVK_ASYNC"));
        assert!(skyrim_fixes.iter().any(|f| f.key == "d3d11"));
    }

    #[test]
    fn resolve_merges_builtin_and_user() {
        let user = vec![LaunchFix {
            game_id: "skyrimse".into(),
            modlist_name: None,
            fix_type: FixType::EnvVar,
            key: "DXVK_ASYNC".into(),
            value: "0".into(), // User wants to disable async
            reason: "User preference".into(),
            source: FixSource::User,
            enabled: true,
        }];

        let resolved = resolve_fixes("skyrimse", None, &user);
        let async_fix = resolved.iter().find(|f| f.key == "DXVK_ASYNC").unwrap();
        assert_eq!(async_fix.value, "0", "User override should win");
        assert_eq!(async_fix.source, FixSource::User);
    }

    #[test]
    fn resolve_filters_by_game() {
        let resolved = resolve_fixes("hogwartslegacy", None, &[]);
        assert!(resolved.is_empty(), "No builtin fixes for Hogwarts Legacy");
    }

    #[test]
    fn resolve_disabled_fixes_excluded() {
        let user = vec![LaunchFix {
            game_id: "skyrimse".into(),
            modlist_name: None,
            fix_type: FixType::EnvVar,
            key: "WINE_LARGE_ADDRESS_AWARE".into(),
            value: "1".into(),
            reason: "Disabled by user".into(),
            source: FixSource::User,
            enabled: false,
        }];

        let resolved = resolve_fixes("skyrimse", None, &user);
        assert!(
            !resolved.iter().any(|f| f.key == "WINE_LARGE_ADDRESS_AWARE"),
            "Disabled user fix should exclude the key entirely"
        );
    }

    #[test]
    fn apply_separates_fix_types() {
        let fixes = vec![
            LaunchFix {
                game_id: "skyrimse".into(),
                modlist_name: None,
                fix_type: FixType::EnvVar,
                key: "FOO".into(),
                value: "bar".into(),
                reason: "test".into(),
                source: FixSource::Builtin,
                enabled: true,
            },
            LaunchFix {
                game_id: "skyrimse".into(),
                modlist_name: None,
                fix_type: FixType::DllOverride,
                key: "d3d11".into(),
                value: "native".into(),
                reason: "test".into(),
                source: FixSource::Builtin,
                enabled: true,
            },
        ];

        let applied = apply_fixes(&fixes);
        assert_eq!(applied.env_vars.len(), 1);
        assert_eq!(applied.dll_overrides.len(), 1);
        assert_eq!(applied.total_applied, 2);
    }

    #[test]
    fn build_dll_override_env_merges() {
        let overrides = vec![
            ("d3d11".into(), "native".into()),
            ("dxgi".into(), "native".into()),
        ];
        let result = build_dll_override_env(&overrides, Some("mscoree=native"));
        assert_eq!(result, "mscoree=native;d3d11=native;dxgi=native");
    }

    #[test]
    fn build_dll_override_env_empty() {
        let overrides = vec![("d3d11".into(), "native".into())];
        let result = build_dll_override_env(&overrides, None);
        assert_eq!(result, "d3d11=native");
    }
}
