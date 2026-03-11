//! Wine/CrossOver/Proton mod compatibility registry.
//!
//! Static registry of mods known to crash or malfunction under Wine.
//! Used by the LLM chat system to warn users proactively and via the
//! `check_wine_compatibility` tool.

use serde::{Deserialize, Serialize};

use crate::database::ModSummary;

/// Build wine compat input from a mod list. Extracts mod name, DLL signals,
/// and nexus ID for each enabled mod. Shared helper to avoid copy-pasting
/// the same mapping logic across tool handlers.
pub fn build_compat_input(mods: &[ModSummary]) -> Vec<(String, Vec<String>, Option<i64>)> {
    mods.iter()
        .filter(|m| m.enabled)
        .map(|m| {
            let mut dlls = Vec::new();
            if m.archive_name.to_lowercase().ends_with(".dll") {
                dlls.push(m.archive_name.clone());
            }
            dlls.push(m.name.clone());
            (m.name.clone(), dlls, m.nexus_mod_id)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Hard crash — game won't start or crashes immediately.
    Crash,
    /// Mod is non-functional on Wine (no crash, but does nothing useful).
    Broken,
    /// Partially works — some features missing or degraded.
    Degraded,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Crash => write!(f, "CRASH"),
            Self::Broken => write!(f, "BROKEN"),
            Self::Degraded => write!(f, "DEGRADED"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WineCompatWarning {
    pub mod_pattern: String,
    pub severity: Severity,
    pub reason: String,
    pub workaround: Option<String>,
    pub alternative_mod_name: Option<String>,
    pub alternative_nexus_id: Option<i64>,
}

struct RegistryEntry {
    /// Case-insensitive substring match against mod name.
    mod_name_patterns: &'static [&'static str],
    /// Case-insensitive substring match against DLL filenames.
    dll_patterns: &'static [&'static str],
    /// If set, exact match on Nexus mod ID.
    nexus_mod_id: Option<i64>,
    severity: Severity,
    reason: &'static str,
    workaround: Option<&'static str>,
    alternative_mod_name: Option<&'static str>,
    alternative_nexus_id: Option<i64>,
}

// ---------------------------------------------------------------------------
// Static registry
// ---------------------------------------------------------------------------

static REGISTRY: &[RegistryEntry] = &[
    RegistryEntry {
        mod_name_patterns: &["sureofstealing"],
        dll_patterns: &["sureofstealing"],
        nexus_mod_id: None,
        severity: Severity::Crash,
        reason: "Null function pointer dereference on Wine — crashes at main menu.",
        workaround: None,
        alternative_mod_name: None,
        alternative_nexus_id: None,
    },
    RegistryEntry {
        mod_name_patterns: &[".net script framework", "netscriptframework"],
        dll_patterns: &["netscriptframework"],
        nexus_mod_id: Some(21294),
        severity: Severity::Crash,
        reason: ".NET runtime is incompatible with Wine. Crashes on load.",
        workaround: None,
        alternative_mod_name: None,
        alternative_nexus_id: None,
    },
    RegistryEntry {
        mod_name_patterns: &["sse engine fixes"],
        dll_patterns: &["enginefixes"],
        nexus_mod_id: Some(17230),
        severity: Severity::Crash,
        reason: "d3dx9_42.dll preloader crashes Wine. Use the Wine-compatible fork instead.",
        workaround: None,
        alternative_mod_name: Some("SSE Engine Fixes for Wine (bundled with Corkscrew)"),
        alternative_nexus_id: None,
    },
    RegistryEntry {
        mod_name_patterns: &["enb", "enbseries"],
        dll_patterns: &["enbseries", "d3d11_enb"],
        nexus_mod_id: None,
        severity: Severity::Degraded,
        reason: "ENB's d3d11 proxy DLL may not work on Wine/DXVK.",
        workaround: Some("Use Wine's built-in DXVK for graphics enhancement instead."),
        alternative_mod_name: None,
        alternative_nexus_id: None,
    },
    RegistryEntry {
        mod_name_patterns: &["reshade"],
        dll_patterns: &["reshade", "dxgi_reshade"],
        nexus_mod_id: None,
        severity: Severity::Broken,
        reason: "ReShade injection is incompatible with Wine's D3D translation layer.",
        workaround: None,
        alternative_mod_name: None,
        alternative_nexus_id: None,
    },
    RegistryEntry {
        mod_name_patterns: &["crash logger"],
        dll_patterns: &["crashloggersse"],
        nexus_mod_id: Some(59596),
        severity: Severity::Broken,
        reason: "Windows-specific crash handling (SEH) does not work on Wine.",
        workaround: None,
        alternative_mod_name: Some("Corkscrew's built-in crash detection"),
        alternative_nexus_id: None,
    },
    RegistryEntry {
        mod_name_patterns: &["sse display tweaks", "ssedisplaytweaks"],
        dll_patterns: &["ssedisplaytweaks"],
        nexus_mod_id: Some(34705),
        severity: Severity::Degraded,
        reason: "Some display features (framerate control, borderless) are non-functional on Wine.",
        workaround: Some("Disable framerate/vsync options; let Wine/DXVK handle display."),
        alternative_mod_name: None,
        alternative_nexus_id: None,
    },
];

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Check a single mod against the Wine compatibility registry.
///
/// Matches by mod name (case-insensitive substring), DLL filenames, or Nexus mod ID.
pub fn check_wine_compatibility(
    mod_name: &str,
    dll_names: &[&str],
    nexus_id: Option<i64>,
) -> Vec<WineCompatWarning> {
    let mod_lower = mod_name.to_lowercase();
    // Precompute lowercase DLL names (strip path, keep filename only).
    let dll_lower: Vec<String> = dll_names
        .iter()
        .map(|d| {
            let fname = d.rsplit(['/', '\\']).next().unwrap_or(d);
            fname.to_lowercase()
        })
        .collect();

    let mut warnings = Vec::new();

    for entry in REGISTRY {
        let mut matched = false;

        // Match by Nexus mod ID (exact).
        if let (Some(entry_id), Some(mod_id)) = (entry.nexus_mod_id, nexus_id) {
            if entry_id == mod_id {
                matched = true;
            }
        }

        // Match by mod name (case-insensitive substring).
        if !matched {
            for pattern in entry.mod_name_patterns {
                if mod_lower.contains(pattern) {
                    // Avoid false positives: "sse engine fixes" should NOT match
                    // the Wine-compatible fork.
                    if *pattern == "sse engine fixes" && mod_lower.contains("wine") {
                        continue;
                    }
                    matched = true;
                    break;
                }
            }
        }

        // Match by DLL filename (case-insensitive substring).
        if !matched {
            for dll_pat in entry.dll_patterns {
                for dll in &dll_lower {
                    if dll.contains(dll_pat) {
                        // Skip Wine-compatible engine fixes DLL.
                        if *dll_pat == "enginefixes"
                            && (dll.contains("wine") || dll.starts_with("0_"))
                        {
                            continue;
                        }
                        matched = true;
                        break;
                    }
                }
                if matched {
                    break;
                }
            }
        }

        if matched {
            warnings.push(WineCompatWarning {
                mod_pattern: entry.mod_name_patterns.first().unwrap_or(&"").to_string(),
                severity: entry.severity.clone(),
                reason: entry.reason.to_string(),
                workaround: entry.workaround.map(|s| s.to_string()),
                alternative_mod_name: entry.alternative_mod_name.map(|s| s.to_string()),
                alternative_nexus_id: entry.alternative_nexus_id,
            });
        }
    }

    warnings
}

/// Batch-check multiple mods. Returns `(mod_name, warning)` pairs.
///
/// Each tuple in `mods` is `(mod_name, dll_filenames, nexus_id)`.
pub fn check_all_mods_wine_compat(
    mods: &[(String, Vec<String>, Option<i64>)],
) -> Vec<(String, WineCompatWarning)> {
    let mut results = Vec::new();
    for (name, dlls, nexus_id) in mods {
        let dll_refs: Vec<&str> = dlls.iter().map(|s| s.as_str()).collect();
        let warnings = check_wine_compatibility(name, &dll_refs, *nexus_id);
        for w in warnings {
            results.push((name.clone(), w));
        }
    }
    results
}

/// Format Wine compat warnings into a human-readable report grouped by severity.
pub fn format_warnings_report(warnings: &[(String, WineCompatWarning)]) -> String {
    if warnings.is_empty() {
        return "No Wine compatibility issues detected.".to_string();
    }

    let mut crash = Vec::new();
    let mut broken = Vec::new();
    let mut degraded = Vec::new();

    for (mod_name, w) in warnings {
        let mut line = format!("- {}: {}", mod_name, w.reason);
        if let Some(ref alt) = w.alternative_mod_name {
            line.push_str(&format!(" Alternative: {alt}."));
        }
        if let Some(ref wa) = w.workaround {
            line.push_str(&format!(" Workaround: {wa}"));
        }
        match w.severity {
            Severity::Crash => crash.push(line),
            Severity::Broken => broken.push(line),
            Severity::Degraded => degraded.push(line),
        }
    }

    let mut report = String::new();
    if !crash.is_empty() {
        report.push_str(&format!(
            "CRASH (will crash the game):\n{}\n",
            crash.join("\n")
        ));
    }
    if !broken.is_empty() {
        report.push_str(&format!(
            "BROKEN (non-functional on Wine):\n{}\n",
            broken.join("\n")
        ));
    }
    if !degraded.is_empty() {
        report.push_str(&format!(
            "DEGRADED (partially works):\n{}\n",
            degraded.join("\n")
        ));
    }
    report.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_sureofstealing_by_dll() {
        let warnings = check_wine_compatibility("Some Mod", &["SureOfStealing.dll"], None);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].severity, Severity::Crash);
    }

    #[test]
    fn detects_engine_fixes_by_name() {
        let warnings = check_wine_compatibility("SSE Engine Fixes", &[], Some(17230));
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].severity, Severity::Crash);
    }

    #[test]
    fn skips_wine_fork() {
        let warnings = check_wine_compatibility(
            "SSE Engine Fixes for Wine",
            &["0_SSEEngineFixesForWine.dll"],
            None,
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn detects_multiple_issues() {
        let mods = vec![
            ("ReShade".to_string(), vec!["reshade.dll".to_string()], None),
            (
                "Crash Logger SSE".to_string(),
                vec!["CrashLoggerSSE.dll".to_string()],
                Some(59596),
            ),
        ];
        let results = check_all_mods_wine_compat(&mods);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn format_report_groups_by_severity() {
        let warnings = vec![
            (
                "BadMod".to_string(),
                WineCompatWarning {
                    mod_pattern: "badmod".into(),
                    severity: Severity::Crash,
                    reason: "Crashes Wine.".into(),
                    workaround: None,
                    alternative_mod_name: None,
                    alternative_nexus_id: None,
                },
            ),
            (
                "OkMod".to_string(),
                WineCompatWarning {
                    mod_pattern: "okmod".into(),
                    severity: Severity::Degraded,
                    reason: "Partially works.".into(),
                    workaround: Some("Disable feature X.".into()),
                    alternative_mod_name: None,
                    alternative_nexus_id: None,
                },
            ),
        ];
        let report = format_warnings_report(&warnings);
        assert!(report.contains("CRASH"));
        assert!(report.contains("DEGRADED"));
        assert!(report.contains("BadMod"));
        assert!(report.contains("OkMod"));
    }
}
