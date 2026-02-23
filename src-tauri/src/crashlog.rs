//! Skyrim SE crash log parser and analyzer.
//!
//! Parses crash logs produced by CrashLoggerSSE (typically found in the SKSE
//! logs directory inside a Wine bottle) and performs pattern-based diagnosis
//! to identify likely crash causes and suggest fixes.
//!
//! # Crash Log Format
//!
//! CrashLoggerSSE produces structured text logs with the following sections:
//!
//! 1. **Header** — logger version and timestamp
//! 2. **Exception** — exception type and faulting address
//! 3. **Probable Call Stack** — stack frames with module + offset
//! 4. **Registers** — CPU register values with optional RTTI type info
//! 5. **Possible Relevant Objects** — game objects, FormIDs, source plugins
//! 6. **Modules** — all loaded DLLs
//! 7. **SKSE Plugins** — third-party SKSE plugin DLLs with versions
//! 8. **Game Plugins** — ESM/ESP/ESL load order

use std::fs;
use std::path::{Path, PathBuf};

use log::{debug, warn};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use walkdir::WalkDir;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum CrashLogError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Not a valid crash log")]
    InvalidFormat,
}

pub type Result<T> = std::result::Result<T, CrashLogError>;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Full analysis of a single crash log.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrashReport {
    /// Path to the original log file.
    pub log_file: String,
    /// Timestamp from the log header.
    pub timestamp: String,
    /// Exception type (e.g. "EXCEPTION_ACCESS_VIOLATION").
    pub exception_type: String,
    /// Full crash address string.
    pub crash_address: String,
    /// Module that caused the crash (e.g. "SkyrimSE.exe" or "hdtSMP64.dll").
    pub module_name: String,
    /// Offset within the faulting module (e.g. "D6DDDA").
    pub module_offset: String,
    /// Diagnoses with confidence levels and suggested actions.
    pub diagnosis: Vec<CrashDiagnosis>,
    /// Overall severity of the crash.
    pub severity: CrashSeverity,
    /// ESP/ESM plugin names found in relevant objects.
    pub involved_plugins: Vec<String>,
    /// SKSE DLL names found in the call stack.
    pub involved_skse_plugins: Vec<String>,
    /// System information parsed from the log, if available.
    pub system_info: Option<SystemInfo>,
    /// First 10 call stack frames as human-readable strings.
    pub call_stack_summary: Vec<String>,
}

/// A single diagnosis produced by pattern matching.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrashDiagnosis {
    /// Short title (e.g. "Memory Exhaustion (D6DDDA)").
    pub title: String,
    /// Human-readable explanation of the crash cause.
    pub description: String,
    /// How confident we are in this diagnosis.
    pub confidence: Confidence,
    /// Actions the user can take to resolve the issue.
    pub suggested_actions: Vec<SuggestedAction>,
}

/// A suggested remediation action.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SuggestedAction {
    /// Category of the action.
    pub action_type: ActionType,
    /// Human-readable description of what to do.
    pub description: String,
    /// Optional target (mod name, plugin name, or file path).
    pub target: Option<String>,
}

/// Categories of suggested fix actions.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ActionType {
    UpdateMod,
    VerifyIntegrity,
    SortLoadOrder,
    DisableMod,
    ReinstallMod,
    CheckVRAM,
    UpdateDrivers,
    CheckINI,
    ManualFix,
}

/// Overall crash severity.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CrashSeverity {
    Critical,
    High,
    Medium,
    Low,
    Unknown,
}

/// Confidence level for a diagnosis.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Confidence {
    High,
    Medium,
    Low,
}

/// System information extracted from the crash log header area.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SystemInfo {
    pub os: Option<String>,
    pub cpu: Option<String>,
    pub gpu: Option<String>,
    pub ram_used_mb: Option<u64>,
    pub ram_total_mb: Option<u64>,
    pub vram_used_mb: Option<u64>,
    pub vram_total_mb: Option<u64>,
}

/// Summary entry for listing crash logs without full analysis.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrashLogEntry {
    /// Log filename.
    pub filename: String,
    /// Timestamp extracted from the filename or log header.
    pub timestamp: String,
    /// One-line summary diagnosis.
    pub summary: String,
    /// Severity level.
    pub severity: CrashSeverity,
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

/// An object reference found in the "Possible Relevant Objects" section.
#[derive(Clone, Debug)]
#[allow(dead_code)]
struct RelevantObject {
    /// The type name (e.g. "NiNode", "BSLightingShaderProperty").
    type_name: String,
    /// Optional FormID (hex string).
    form_id: Option<String>,
    /// Source plugin file if mentioned.
    source_plugin: Option<String>,
    /// The full raw line for content pattern matching.
    raw: String,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Return the script extender log directory path for a given game and bottle.
///
/// Skyrim SE: `[bottle]/drive_c/users/crossover/Documents/My Games/Skyrim Special Edition/SKSE/`
/// Fallout 4: `[bottle]/drive_c/users/crossover/Documents/My Games/Fallout4/F4SE/`
pub fn script_extender_log_dir(bottle_path: &Path, game_id: &str) -> PathBuf {
    let (game_dir, se_dir) = match game_id {
        "fallout4" => ("Fallout4", "F4SE"),
        _ => ("Skyrim Special Edition", "SKSE"),
    };
    bottle_path
        .join("drive_c")
        .join("users")
        .join("crossover")
        .join("Documents")
        .join("My Games")
        .join(game_dir)
        .join(se_dir)
}

/// Legacy alias for backwards compatibility.
pub fn skse_log_dir(bottle_path: &Path) -> PathBuf {
    script_extender_log_dir(bottle_path, "skyrimse")
}

/// Find all crash logs in the SKSE directory for a game.
///
/// Returns a list of [`CrashLogEntry`] summaries sorted by timestamp
/// (most recent first). Each entry includes a quick one-line summary
/// derived from the exception line without performing full analysis.
pub fn find_crash_logs(game_path: &Path, bottle_path: &Path, game_id: &str) -> Vec<CrashLogEntry> {
    let _ = game_path;
    let log_dir = script_extender_log_dir(bottle_path, game_id);

    if !log_dir.is_dir() {
        debug!(
            "Script extender log directory does not exist: {}",
            log_dir.display()
        );
        return Vec::new();
    }

    let mut entries: Vec<CrashLogEntry> = Vec::new();

    for entry in WalkDir::new(&log_dir).max_depth(1).into_iter().flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let filename = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) if name.starts_with("crash-") && name.ends_with(".log") => name.to_string(),
            _ => continue,
        };

        // Extract timestamp from filename: crash-YYYY-MM-DD-HH-MM-SS.log
        let timestamp = filename
            .trim_start_matches("crash-")
            .trim_end_matches(".log")
            .to_string();

        // Quick summary from the first few lines
        let (summary, severity) = match fs::read_to_string(path) {
            Ok(content) => quick_summary(&content),
            Err(e) => {
                warn!("Could not read crash log {}: {}", path.display(), e);
                ("Unreadable crash log".to_string(), CrashSeverity::Unknown)
            }
        };

        entries.push(CrashLogEntry {
            filename,
            timestamp,
            summary,
            severity,
        });
    }

    // Sort by timestamp descending (most recent first).
    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    entries
}

/// Parse a single crash log file and produce a full analysis.
pub fn analyze_crash_log(log_path: &Path) -> Result<CrashReport> {
    let content = fs::read_to_string(log_path)?;

    // Validate that this looks like a CrashLoggerSSE log.
    if !content.contains("Crash Logger") && !content.contains("Unhandled exception") {
        return Err(CrashLogError::InvalidFormat);
    }

    let (exception_type, crash_address) = parse_header(&content);
    let (module_name, module_offset) = parse_crash_module(&content);
    let call_stack = parse_call_stack(&content);
    let relevant_objects = parse_relevant_objects(&content);
    let _skse_plugins = parse_skse_plugins(&content);
    let _game_plugins = parse_game_plugins(&content);
    let system_info = parse_system_info(&content);
    let timestamp = parse_timestamp(&content);

    // Build call stack summary (first 10 frames).
    let call_stack_summary: Vec<String> = call_stack
        .iter()
        .take(10)
        .map(|(module, offset)| format!("{}+{}", module, offset))
        .collect();

    // Find SKSE DLLs in the call stack.
    let involved_skse_plugins: Vec<String> = call_stack
        .iter()
        .filter_map(|(module, _)| {
            let lower = module.to_lowercase();
            if lower.ends_with(".dll") && lower != "skyrimse.exe" && !is_system_dll(&lower) {
                Some(module.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .into_iter()
        .fold(Vec::new(), |mut acc, item| {
            if !acc
                .iter()
                .any(|x: &String| x.to_lowercase() == item.to_lowercase())
            {
                acc.push(item);
            }
            acc
        });

    // Collect plugins mentioned in relevant objects.
    let involved_plugins: Vec<String> = relevant_objects
        .iter()
        .filter_map(|obj| obj.source_plugin.clone())
        .collect::<Vec<_>>()
        .into_iter()
        .fold(Vec::new(), |mut acc, item| {
            if !acc
                .iter()
                .any(|x: &String| x.to_lowercase() == item.to_lowercase())
            {
                acc.push(item);
            }
            acc
        });

    // Run pattern matching.
    let diagnosis = match_crash_patterns(
        &module_name,
        &module_offset,
        &call_stack,
        &relevant_objects,
        &content,
    );

    // Determine overall severity from highest-confidence diagnosis.
    let severity = determine_severity(&diagnosis);

    let log_file = log_path.to_string_lossy().to_string();

    Ok(CrashReport {
        log_file,
        timestamp,
        exception_type,
        crash_address,
        module_name,
        module_offset,
        diagnosis,
        severity,
        involved_plugins,
        involved_skse_plugins,
        system_info,
        call_stack_summary,
    })
}

// ---------------------------------------------------------------------------
// Internal parsing functions
// ---------------------------------------------------------------------------

/// Extract the exception type and crash address from the log header.
fn parse_header(content: &str) -> (String, String) {
    // Look for: Unhandled exception "EXCEPTION_ACCESS_VIOLATION" at 0x7FF6D1B0DDDA SkyrimSE.exe+D6DDDA
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Unhandled exception") {
            // Extract exception type between quotes.
            let exception_type =
                extract_between(trimmed, "\"", "\"").unwrap_or_else(|| "UNKNOWN".to_string());

            // Extract address after " at ".
            let crash_address = if let Some(at_idx) = trimmed.find(" at ") {
                trimmed[at_idx + 4..].trim().to_string()
            } else {
                String::new()
            };

            return (exception_type, crash_address);
        }
    }

    ("UNKNOWN".to_string(), String::new())
}

/// Extract the faulting module name and offset from the exception line.
fn parse_crash_module(content: &str) -> (String, String) {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Unhandled exception") {
            // Look for the module+offset pattern at the end of the line.
            // e.g. "SkyrimSE.exe+D6DDDA" or "hdtSMP64.dll+1234"
            for token in trimmed.split_whitespace().rev() {
                if token.contains('+') {
                    let parts: Vec<&str> = token.splitn(2, '+').collect();
                    if parts.len() == 2 {
                        return (parts[0].to_string(), parts[1].to_string());
                    }
                }
            }
        }
    }

    ("Unknown".to_string(), String::new())
}

/// Parse the timestamp from the log header.
fn parse_timestamp(content: &str) -> String {
    for line in content.lines().take(10) {
        let trimmed = line.trim();
        // CrashLoggerSSE typically has a line like:
        // "Skyrim SE v1.6.640.0" or a date/time line.
        // The crash- filename has the timestamp, but also look for date patterns.
        if trimmed.contains("202") && (trimmed.contains('-') || trimmed.contains('/')) {
            // Heuristic: if the line looks like a date.
            if trimmed.len() < 60 && !trimmed.starts_with("Unhandled") {
                return trimmed.to_string();
            }
        }
    }
    String::new()
}

/// Parse the "PROBABLE CALL STACK" section.
///
/// Returns a list of (module_name, offset) tuples.
fn parse_call_stack(content: &str) -> Vec<(String, String)> {
    let mut frames: Vec<(String, String)> = Vec::new();
    let mut in_call_stack = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Detect start of call stack section.
        if trimmed.contains("PROBABLE CALL STACK")
            || trimmed.contains("CALL STACK")
            || trimmed.contains("Call Stack")
        {
            in_call_stack = true;
            continue;
        }

        // An empty line or a new section header ends the call stack.
        if in_call_stack {
            if trimmed.is_empty()
                || (trimmed
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_uppercase())
                    && !trimmed.starts_with('[')
                    && trimmed.contains(':')
                    && !trimmed.contains('+'))
            {
                // Might be a new section header. Check more carefully.
                if trimmed.is_empty() {
                    in_call_stack = false;
                    continue;
                }
            }

            // Call stack frames look like:
            // [0]  0x7FF6D1B0DDDA  SkyrimSE.exe+D6DDDA
            // [1]  0x7FF6D1B0DDDA  hdtSMP64.dll+1234
            if trimmed.starts_with('[') {
                // Find the module+offset token.
                for token in trimmed.split_whitespace() {
                    if token.contains('+') && !token.starts_with("0x") {
                        let parts: Vec<&str> = token.splitn(2, '+').collect();
                        if parts.len() == 2 {
                            frames.push((parts[0].to_string(), parts[1].to_string()));
                            break;
                        }
                    }
                }
            }
        }
    }

    frames
}

/// Parse the "POSSIBLE RELEVANT OBJECTS" section.
fn parse_relevant_objects(content: &str) -> Vec<RelevantObject> {
    let mut objects: Vec<RelevantObject> = Vec::new();
    let mut in_section = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.contains("POSSIBLE RELEVANT OBJECTS") || trimmed.contains("Relevant Objects") {
            in_section = true;
            continue;
        }

        if in_section {
            // End of section on empty line or new section header.
            if trimmed.is_empty() {
                in_section = false;
                continue;
            }

            // New section headers are typically all-caps or contain colons.
            if !trimmed.starts_with('[')
                && !trimmed.starts_with('(')
                && !trimmed.chars().next().is_some_and(|c| c.is_ascii_digit())
                && trimmed.to_uppercase() == trimmed
                && trimmed.len() > 3
            {
                in_section = false;
                continue;
            }

            let type_name = extract_type_name(trimmed);
            let form_id = extract_form_id(trimmed);
            let source_plugin = extract_source_plugin(trimmed);

            objects.push(RelevantObject {
                type_name,
                form_id,
                source_plugin,
                raw: trimmed.to_string(),
            });
        }
    }

    objects
}

/// Parse the "SKSE PLUGINS" section to get loaded SKSE plugin names.
fn parse_skse_plugins(content: &str) -> Vec<String> {
    let mut plugins: Vec<String> = Vec::new();
    let mut in_section = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.contains("SKSE PLUGINS") || trimmed.contains("SKSE Plugins") {
            in_section = true;
            continue;
        }

        if in_section {
            if trimmed.is_empty() {
                in_section = false;
                continue;
            }

            // Lines look like: "  hdtSMP64 v1.0" or "  RaceMenu v0.4.19.4"
            // Or sometimes: "hdtSMP64.dll v1.0"
            let name = trimmed
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_end_matches(".dll")
                .to_string();

            if !name.is_empty() {
                plugins.push(name);
            }
        }
    }

    plugins
}

/// Parse the "GAME PLUGINS" section to get the ESP/ESM/ESL load order.
fn parse_game_plugins(content: &str) -> Vec<String> {
    let mut plugins: Vec<String> = Vec::new();
    let mut in_section = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.contains("GAME PLUGINS")
            || trimmed.contains("Game Plugins")
            || trimmed.contains("PLUGINS:")
        {
            in_section = true;
            continue;
        }

        if in_section {
            if trimmed.is_empty() {
                in_section = false;
                continue;
            }

            // Lines look like:
            // [00] Skyrim.esm
            // [01] Update.esm
            // [FE:000] ccBGSSSE001-Fish.esl
            // Or sometimes without brackets:
            // Skyrim.esm
            let plugin_name = if trimmed.starts_with('[') {
                // Strip the load order prefix.
                if let Some(bracket_end) = trimmed.find(']') {
                    trimmed[bracket_end + 1..].trim().to_string()
                } else {
                    trimmed.to_string()
                }
            } else {
                trimmed.to_string()
            };

            let plugin_name = plugin_name.trim().to_string();
            if !plugin_name.is_empty()
                && (plugin_name.ends_with(".esm")
                    || plugin_name.ends_with(".esp")
                    || plugin_name.ends_with(".esl"))
            {
                plugins.push(plugin_name);
            }
        }
    }

    plugins
}

/// Parse system information from the log header area.
fn parse_system_info(content: &str) -> Option<SystemInfo> {
    let mut os = None;
    let mut cpu = None;
    let mut gpu = None;
    let mut ram_used_mb = None;
    let mut ram_total_mb = None;
    let mut vram_used_mb = None;
    let mut vram_total_mb = None;

    // System info is typically in the first ~30 lines.
    for line in content.lines().take(50) {
        let trimmed = line.trim();
        let lower = trimmed.to_lowercase();

        if lower.starts_with("os:") || lower.starts_with("os version:") {
            os = Some(after_colon(trimmed).to_string());
        } else if lower.starts_with("cpu:") || lower.starts_with("processor:") {
            cpu = Some(after_colon(trimmed).to_string());
        } else if lower.starts_with("gpu:")
            || lower.starts_with("video:")
            || lower.starts_with("adapter:")
        {
            gpu = Some(after_colon(trimmed).to_string());
        } else if lower.contains("physical memory") || lower.contains("ram") {
            // Parse lines like: "Physical Memory: 8192 MB / 16384 MB"
            if let Some((used, total)) = parse_memory_line(trimmed) {
                ram_used_mb = Some(used);
                ram_total_mb = Some(total);
            }
        } else if lower.contains("vram") || lower.contains("video memory") {
            if let Some((used, total)) = parse_memory_line(trimmed) {
                vram_used_mb = Some(used);
                vram_total_mb = Some(total);
            }
        }
    }

    if os.is_some() || cpu.is_some() || gpu.is_some() || ram_total_mb.is_some() {
        Some(SystemInfo {
            os,
            cpu,
            gpu,
            ram_used_mb,
            ram_total_mb,
            vram_used_mb,
            vram_total_mb,
        })
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Crash pattern matching
// ---------------------------------------------------------------------------

/// Match the crash data against known patterns and produce diagnoses.
fn match_crash_patterns(
    module: &str,
    offset: &str,
    call_stack: &[(String, String)],
    objects: &[RelevantObject],
    raw_content: &str,
) -> Vec<CrashDiagnosis> {
    let mut diagnoses: Vec<CrashDiagnosis> = Vec::new();

    // Collect all raw text for content-based matching.
    let content_lower = raw_content.to_lowercase();
    let all_raw_objects: String = objects
        .iter()
        .map(|o| o.raw.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let all_raw_lower = all_raw_objects.to_lowercase();

    // All call stack modules lowered for easy matching.
    let stack_modules: Vec<String> = call_stack.iter().map(|(m, _)| m.to_lowercase()).collect();

    // -----------------------------------------------------------------------
    // Address-based patterns (SkyrimSE.exe + specific offset)
    // -----------------------------------------------------------------------

    if module == "SkyrimSE.exe" || module.to_lowercase() == "skyrimse.exe" {
        // D6DDDA — RAM/VRAM exhaustion or corrupt mesh
        if offset == "D6DDDA" || offset == "d6ddda" {
            diagnoses.push(CrashDiagnosis {
                title: "Memory Exhaustion (D6DDDA)".to_string(),
                description: "This crash occurs when the game runs out of addressable memory \
                    or VRAM. It can also be caused by a corrupt mesh file that triggers \
                    excessive memory allocation."
                    .to_string(),
                confidence: Confidence::High,
                suggested_actions: vec![
                    SuggestedAction {
                        action_type: ActionType::CheckVRAM,
                        description: "Check VRAM usage — lower texture resolution or remove \
                            high-resolution texture packs"
                            .to_string(),
                        target: None,
                    },
                    SuggestedAction {
                        action_type: ActionType::VerifyIntegrity,
                        description: "Verify game file integrity to rule out corrupt meshes"
                            .to_string(),
                        target: None,
                    },
                    SuggestedAction {
                        action_type: ActionType::CheckINI,
                        description: "Set iTextureUpgrade=0 in SkyrimPrefs.ini to reduce \
                            texture memory usage"
                            .to_string(),
                        target: Some("SkyrimPrefs.ini".to_string()),
                    },
                ],
            });
        }

        // 12FDD00 — Shadow Scene Node crash
        if offset == "12FDD00" || offset == "12fdd00" {
            diagnoses.push(CrashDiagnosis {
                title: "Shadow Scene Node Crash (12FDD00)".to_string(),
                description: "The Shadow Scene Node has become corrupted. This is typically \
                    caused by lighting or shadow mods that modify the scene graph incorrectly, \
                    or by an NPC/object with a broken light reference."
                    .to_string(),
                confidence: Confidence::High,
                suggested_actions: vec![
                    SuggestedAction {
                        action_type: ActionType::DisableMod,
                        description: "Disable lighting overhaul mods temporarily to isolate \
                            the cause"
                            .to_string(),
                        target: None,
                    },
                    SuggestedAction {
                        action_type: ActionType::SortLoadOrder,
                        description: "Sort your load order with LOOT — lighting mod conflicts \
                            are often load-order sensitive"
                            .to_string(),
                        target: None,
                    },
                ],
            });
        }

        // A0D789 — Animation limit exceeded
        if offset == "A0D789" || offset == "a0d789" {
            diagnoses.push(CrashDiagnosis {
                title: "Animation Limit Exceeded (A0D789)".to_string(),
                description: "The game has exceeded its animation limit. This happens when \
                    too many animation mods are installed without a proper animation framework \
                    like Nemesis or FNIS to merge them."
                    .to_string(),
                confidence: Confidence::High,
                suggested_actions: vec![
                    SuggestedAction {
                        action_type: ActionType::ManualFix,
                        description: "Run Nemesis or FNIS to regenerate the animation cache"
                            .to_string(),
                        target: None,
                    },
                    SuggestedAction {
                        action_type: ActionType::DisableMod,
                        description: "Remove some animation mods if the limit is still exceeded"
                            .to_string(),
                        target: None,
                    },
                ],
            });
        }

        // 0CB748E — Broken NIF mesh files
        if offset == "0CB748E" || offset == "CB748E" || offset == "cb748e" || offset == "0cb748e" {
            diagnoses.push(CrashDiagnosis {
                title: "Broken NIF Mesh (CB748E)".to_string(),
                description: "A NIF mesh file is malformed or incompatible. The game crashes \
                    when trying to load or render the mesh. Check any recently installed \
                    mesh-replacement mods."
                    .to_string(),
                confidence: Confidence::High,
                suggested_actions: vec![
                    SuggestedAction {
                        action_type: ActionType::ReinstallMod,
                        description: "Reinstall or update mesh-replacement mods — the NIF \
                            file may be corrupt or built for the wrong Skyrim version"
                            .to_string(),
                        target: None,
                    },
                    SuggestedAction {
                        action_type: ActionType::VerifyIntegrity,
                        description: "Verify game integrity to restore any vanilla meshes \
                            that may have been overwritten"
                            .to_string(),
                        target: None,
                    },
                ],
            });
        }
    }

    // -----------------------------------------------------------------------
    // DLL-based patterns (specific DLLs in the call stack)
    // -----------------------------------------------------------------------

    // hdtSMP64.dll — HDT-SMP physics crash
    if stack_modules.iter().any(|m| m.contains("hdtsmp64")) {
        diagnoses.push(CrashDiagnosis {
            title: "HDT-SMP Physics Crash".to_string(),
            description: "The HDT-SMP physics engine crashed. This is often caused by \
                incompatible physics configurations, too many physics-enabled objects in a \
                scene, or an outdated HDT-SMP version."
                .to_string(),
            confidence: Confidence::High,
            suggested_actions: vec![
                SuggestedAction {
                    action_type: ActionType::UpdateMod,
                    description: "Update HDT-SMP to the latest version".to_string(),
                    target: Some("hdtSMP64".to_string()),
                },
                SuggestedAction {
                    action_type: ActionType::CheckINI,
                    description: "Reduce physics object limits in hdtSMP64 configs".to_string(),
                    target: Some("hdtSMP64.ini".to_string()),
                },
            ],
        });
    }

    // skee64.dll — RaceMenu / skin overlay issues
    if stack_modules.iter().any(|m| m.contains("skee64")) {
        diagnoses.push(CrashDiagnosis {
            title: "RaceMenu / Skin Overlay Crash".to_string(),
            description: "The skee64.dll (RaceMenu overlay system) crashed. This is \
                commonly caused by too many overlays, corrupt overlay textures, or \
                incompatible RaceMenu version."
                .to_string(),
            confidence: Confidence::High,
            suggested_actions: vec![
                SuggestedAction {
                    action_type: ActionType::UpdateMod,
                    description: "Update RaceMenu to the latest version".to_string(),
                    target: Some("RaceMenu".to_string()),
                },
                SuggestedAction {
                    action_type: ActionType::ManualFix,
                    description: "Reduce the number of skin overlays in skee64 overrides"
                        .to_string(),
                    target: None,
                },
            ],
        });
    }

    // cbp.dll — CBP Physics crash
    if stack_modules.iter().any(|m| m.contains("cbp")) {
        diagnoses.push(CrashDiagnosis {
            title: "CBP Physics Crash".to_string(),
            description: "CBP Physics has crashed. This can be caused by conflicts with \
                HDT-SMP (both should not be active simultaneously) or by a corrupt \
                CBP configuration."
                .to_string(),
            confidence: Confidence::Medium,
            suggested_actions: vec![
                SuggestedAction {
                    action_type: ActionType::DisableMod,
                    description: "Ensure CBP and HDT-SMP are not both active — use only one \
                        physics framework"
                        .to_string(),
                    target: Some("cbp.dll".to_string()),
                },
                SuggestedAction {
                    action_type: ActionType::UpdateMod,
                    description: "Update CBP Physics to the latest version".to_string(),
                    target: Some("CBP Physics".to_string()),
                },
            ],
        });
    }

    // JContainers64.dll — JContainers crash
    if stack_modules.iter().any(|m| m.contains("jcontainers64")) {
        diagnoses.push(CrashDiagnosis {
            title: "JContainers Crash".to_string(),
            description: "JContainers64.dll crashed. This is often caused by a corrupt \
                JContainers save data file or a version mismatch with the running SKSE."
                .to_string(),
            confidence: Confidence::Medium,
            suggested_actions: vec![
                SuggestedAction {
                    action_type: ActionType::UpdateMod,
                    description: "Update JContainers to the latest version matching your \
                        SKSE version"
                        .to_string(),
                    target: Some("JContainers".to_string()),
                },
                SuggestedAction {
                    action_type: ActionType::ManualFix,
                    description: "Delete the JContainers co-save files (.jc) for the \
                        affected save game"
                        .to_string(),
                    target: None,
                },
            ],
        });
    }

    // PDPerfPlugin.dll — DLAA incompatibility
    if stack_modules.iter().any(|m| m.contains("pdperfplugin")) {
        diagnoses.push(CrashDiagnosis {
            title: "PD Perf Plugin / DLAA Incompatibility".to_string(),
            description: "PDPerfPlugin.dll (Display Tweaks or performance plugin) has \
                crashed. This is commonly caused by DLAA or upscaling incompatibilities, \
                especially under Wine/CrossOver where GPU features differ."
                .to_string(),
            confidence: Confidence::Medium,
            suggested_actions: vec![
                SuggestedAction {
                    action_type: ActionType::DisableMod,
                    description: "Disable DLAA/upscaling features in the performance plugin"
                        .to_string(),
                    target: Some("PDPerfPlugin".to_string()),
                },
                SuggestedAction {
                    action_type: ActionType::UpdateMod,
                    description: "Update the performance plugin to the latest version".to_string(),
                    target: Some("PDPerfPlugin".to_string()),
                },
            ],
        });
    }

    // nvwgf2umx.dll — NVIDIA driver crash
    if stack_modules.iter().any(|m| m.contains("nvwgf2umx")) {
        diagnoses.push(CrashDiagnosis {
            title: "NVIDIA Graphics Driver Crash".to_string(),
            description: "The crash originated in the NVIDIA graphics driver (nvwgf2umx.dll). \
                This can indicate a driver bug, overheated GPU, or VRAM exhaustion."
                .to_string(),
            confidence: Confidence::High,
            suggested_actions: vec![
                SuggestedAction {
                    action_type: ActionType::UpdateDrivers,
                    description: "Update NVIDIA graphics drivers to the latest version".to_string(),
                    target: None,
                },
                SuggestedAction {
                    action_type: ActionType::CheckVRAM,
                    description: "Reduce graphics settings and texture quality to lower VRAM usage"
                        .to_string(),
                    target: None,
                },
            ],
        });
    }

    // tbbmalloc.dll — Memory allocator crash
    if stack_modules.iter().any(|m| m.contains("tbbmalloc")) {
        diagnoses.push(CrashDiagnosis {
            title: "Memory Allocator Crash (tbbmalloc)".to_string(),
            description: "The TBB memory allocator has crashed. This usually means the game \
                ran out of memory or there is a heap corruption bug in a mod."
                .to_string(),
            confidence: Confidence::Medium,
            suggested_actions: vec![
                SuggestedAction {
                    action_type: ActionType::CheckVRAM,
                    description: "Reduce memory usage — lower texture quality and remove \
                        heavy mods"
                        .to_string(),
                    target: None,
                },
                SuggestedAction {
                    action_type: ActionType::ManualFix,
                    description: "If using a custom allocator mod, try reverting to the \
                        default allocator"
                        .to_string(),
                    target: None,
                },
            ],
        });
    }

    // usvfs_x64.dll — MO2 VFS blocked by antivirus
    if stack_modules.iter().any(|m| m.contains("usvfs_x64")) {
        diagnoses.push(CrashDiagnosis {
            title: "MO2 Virtual File System Crash".to_string(),
            description: "The Mod Organizer 2 virtual file system (usvfs_x64.dll) has \
                crashed. This is commonly caused by antivirus software blocking the VFS \
                hooks, or an incompatible USVFS version."
                .to_string(),
            confidence: Confidence::High,
            suggested_actions: vec![
                SuggestedAction {
                    action_type: ActionType::ManualFix,
                    description: "Add MO2 and the game directory to your antivirus exclusions"
                        .to_string(),
                    target: None,
                },
                SuggestedAction {
                    action_type: ActionType::UpdateMod,
                    description: "Update Mod Organizer 2 to the latest version".to_string(),
                    target: Some("Mod Organizer 2".to_string()),
                },
            ],
        });
    }

    // -----------------------------------------------------------------------
    // Content-based patterns (registers, relevant objects, raw log text)
    // -----------------------------------------------------------------------

    // .STRINGS in registers — Non-ASCII in skyrim.ini
    if content_lower.contains(".strings") {
        // Check specifically in the registers section
        let in_registers = is_in_section(raw_content, "REGISTERS", ".STRINGS")
            || is_in_section(raw_content, "Registers", ".STRINGS");
        if in_registers || content_lower.contains("bslocalizedstring") {
            diagnoses.push(CrashDiagnosis {
                title: "String Encoding Issue".to_string(),
                description: "A .STRINGS reference was found in the crash registers. This \
                    typically indicates non-ASCII characters in Skyrim.ini, SkyrimPrefs.ini, \
                    or a plugin's string data, causing a localization lookup crash."
                    .to_string(),
                confidence: Confidence::Medium,
                suggested_actions: vec![SuggestedAction {
                    action_type: ActionType::CheckINI,
                    description: "Check Skyrim.ini and SkyrimPrefs.ini for non-ASCII \
                            characters (accented letters, special symbols) and remove them"
                        .to_string(),
                    target: Some("Skyrim.ini".to_string()),
                }],
            });
        }
    }

    // CompressedArchiveStream — Corrupt BSA archive
    if content_lower.contains("compressedarchivestream") || content_lower.contains("bsarchive") {
        diagnoses.push(CrashDiagnosis {
            title: "Corrupt BSA Archive".to_string(),
            description: "A CompressedArchiveStream error was detected, indicating a corrupt \
                or incomplete BSA/BA2 archive. This can happen when a mod was partially \
                downloaded or the archive was built incorrectly."
                .to_string(),
            confidence: Confidence::Medium,
            suggested_actions: vec![
                SuggestedAction {
                    action_type: ActionType::ReinstallMod,
                    description: "Re-download and reinstall mods that include BSA archives"
                        .to_string(),
                    target: None,
                },
                SuggestedAction {
                    action_type: ActionType::VerifyIntegrity,
                    description: "Verify game integrity to restore any corrupt vanilla BSA files"
                        .to_string(),
                    target: None,
                },
            ],
        });
    }

    // NiNode + bone names — Skeleton/XP32 issues
    if (all_raw_lower.contains("ninode") || content_lower.contains("ninode"))
        && (content_lower.contains("skeleton")
            || content_lower.contains("npc root")
            || content_lower.contains("npc l hand")
            || content_lower.contains("npc r hand")
            || content_lower.contains("weapon"))
    {
        diagnoses.push(CrashDiagnosis {
            title: "Skeleton / XP32 Issue".to_string(),
            description: "The crash involves NiNode objects with skeleton bone references. \
                This usually means a skeleton mismatch — mods expecting XP32 Maximum \
                Skeleton (XPMSE) but finding the vanilla skeleton, or a conflicting \
                skeleton replacer."
                .to_string(),
            confidence: Confidence::Medium,
            suggested_actions: vec![
                SuggestedAction {
                    action_type: ActionType::ReinstallMod,
                    description: "Reinstall XP32 Maximum Skeleton Extended (XPMSE) and \
                        ensure it overwrites other skeleton mods"
                        .to_string(),
                    target: Some("XPMSE".to_string()),
                },
                SuggestedAction {
                    action_type: ActionType::ManualFix,
                    description: "Run FNIS or Nemesis after reinstalling the skeleton".to_string(),
                    target: None,
                },
            ],
        });
    }

    // ShadowSceneNode — Lighting mod conflicts
    if content_lower.contains("shadowscenenode") {
        // Only add if we haven't already matched the address-based Shadow Scene Node pattern.
        let already_matched = diagnoses
            .iter()
            .any(|d| d.title.contains("Shadow Scene Node"));
        if !already_matched {
            diagnoses.push(CrashDiagnosis {
                title: "Shadow Scene Node Conflict".to_string(),
                description: "ShadowSceneNode references appear in the crash data. Lighting \
                    or shadow mods may be conflicting, or an object in the game world has \
                    a broken light/shadow attachment."
                    .to_string(),
                confidence: Confidence::Medium,
                suggested_actions: vec![
                    SuggestedAction {
                        action_type: ActionType::DisableMod,
                        description: "Temporarily disable ENB, lighting overhauls, or shadow \
                            mods to isolate the conflict"
                            .to_string(),
                        target: None,
                    },
                    SuggestedAction {
                        action_type: ActionType::SortLoadOrder,
                        description: "Sort your load order with LOOT".to_string(),
                        target: None,
                    },
                ],
            });
        }
    }

    // BSLightingShaderProperty — Shader crash
    if content_lower.contains("bslightingshaderproperty") {
        diagnoses.push(CrashDiagnosis {
            title: "Lighting Shader Crash".to_string(),
            description: "BSLightingShaderProperty was involved in the crash. A mesh file \
                has a broken or incompatible shader assignment. This is often caused by \
                meshes ported from Skyrim LE without proper conversion."
                .to_string(),
            confidence: Confidence::Medium,
            suggested_actions: vec![
                SuggestedAction {
                    action_type: ActionType::ManualFix,
                    description: "Run NIF files through Cathedral Assets Optimizer (CAO) to \
                        fix shader assignments"
                        .to_string(),
                    target: None,
                },
                SuggestedAction {
                    action_type: ActionType::ReinstallMod,
                    description: "Ensure mesh-replacement mods are the SSE version, not LE"
                        .to_string(),
                    target: None,
                },
            ],
        });
    }

    // bad_alloc / no_alloc — Out of memory
    if content_lower.contains("bad_alloc") || content_lower.contains("no_alloc") {
        diagnoses.push(CrashDiagnosis {
            title: "Out of Memory".to_string(),
            description: "The game threw a memory allocation failure (bad_alloc). The system \
                has run out of available RAM or the process has hit its address space limit."
                .to_string(),
            confidence: Confidence::High,
            suggested_actions: vec![
                SuggestedAction {
                    action_type: ActionType::CheckVRAM,
                    description: "Close other applications and reduce texture/mesh quality"
                        .to_string(),
                    target: None,
                },
                SuggestedAction {
                    action_type: ActionType::DisableMod,
                    description: "Remove heavy texture packs or reduce the number of active mods"
                        .to_string(),
                    target: None,
                },
            ],
        });
    }

    // .dds path — Corrupt texture (identify file)
    if let Some(dds_path) = find_file_reference(&content_lower, ".dds") {
        diagnoses.push(CrashDiagnosis {
            title: "Corrupt Texture File".to_string(),
            description: format!(
                "A .dds texture file was referenced near the crash point: \"{}\". This \
                 texture may be corrupt, incompatible (wrong DDS format), or too large.",
                dds_path
            ),
            confidence: Confidence::Low,
            suggested_actions: vec![SuggestedAction {
                action_type: ActionType::ReinstallMod,
                description: format!(
                    "Reinstall the mod that provides \"{}\" or convert it with \
                         Cathedral Assets Optimizer",
                    dds_path
                ),
                target: Some(dds_path.to_string()),
            }],
        });
    }

    // .nif path — Corrupt mesh (identify file)
    if let Some(nif_path) = find_file_reference(&content_lower, ".nif") {
        diagnoses.push(CrashDiagnosis {
            title: "Corrupt Mesh File".to_string(),
            description: format!(
                "A .nif mesh file was referenced near the crash point: \"{}\". This \
                 mesh may be malformed or built for the wrong Skyrim version (LE vs SE).",
                nif_path
            ),
            confidence: Confidence::Low,
            suggested_actions: vec![SuggestedAction {
                action_type: ActionType::ReinstallMod,
                description: format!(
                    "Reinstall the mod that provides \"{}\" or run it through \
                         NIF Optimizer / Cathedral Assets Optimizer",
                    nif_path
                ),
                target: Some(nif_path.to_string()),
            }],
        });
    }

    // If no patterns matched, produce a generic diagnosis.
    if diagnoses.is_empty() {
        diagnoses.push(CrashDiagnosis {
            title: "Unrecognized Crash".to_string(),
            description: format!(
                "The crash at {}+{} did not match any known patterns. Check the call stack \
                 and relevant objects for clues.",
                module, offset
            ),
            confidence: Confidence::Low,
            suggested_actions: vec![
                SuggestedAction {
                    action_type: ActionType::VerifyIntegrity,
                    description: "Verify game file integrity as a first step".to_string(),
                    target: None,
                },
                SuggestedAction {
                    action_type: ActionType::SortLoadOrder,
                    description: "Sort your load order with LOOT to resolve potential conflicts"
                        .to_string(),
                    target: None,
                },
            ],
        });
    }

    diagnoses
}

// ---------------------------------------------------------------------------
// Helper utilities
// ---------------------------------------------------------------------------

/// Determine overall severity from the list of diagnoses.
fn determine_severity(diagnoses: &[CrashDiagnosis]) -> CrashSeverity {
    if diagnoses.is_empty() {
        return CrashSeverity::Unknown;
    }

    // If any high-confidence diagnosis exists, severity is at least High.
    let has_high = diagnoses.iter().any(|d| d.confidence == Confidence::High);
    let has_medium = diagnoses.iter().any(|d| d.confidence == Confidence::Medium);

    // Check for truly critical patterns.
    let is_critical = diagnoses.iter().any(|d| {
        d.title.contains("Out of Memory")
            || d.title.contains("Memory Exhaustion")
            || d.title.contains("NVIDIA")
    });

    if is_critical {
        CrashSeverity::Critical
    } else if has_high {
        CrashSeverity::High
    } else if has_medium {
        CrashSeverity::Medium
    } else {
        CrashSeverity::Low
    }
}

/// Quick one-line summary for listing without full analysis.
fn quick_summary(content: &str) -> (String, CrashSeverity) {
    let (exception_type, crash_address) = parse_header(content);
    let (module, offset) = parse_crash_module(content);

    if module == "Unknown" && exception_type == "UNKNOWN" {
        return (
            "Unable to parse crash log".to_string(),
            CrashSeverity::Unknown,
        );
    }

    let summary = if module.to_lowercase() != "skyrimse.exe" && module != "Unknown" {
        format!("{} in {}", exception_type, module)
    } else if !offset.is_empty() {
        // Check for well-known offsets.
        let hint = match offset.to_uppercase().as_str() {
            "D6DDDA" => " (Memory exhaustion)",
            "12FDD00" => " (Shadow Scene Node)",
            "A0D789" => " (Animation limit)",
            "CB748E" | "0CB748E" => " (Broken NIF mesh)",
            _ => "",
        };
        format!("{} at {}+{}{}", exception_type, module, offset, hint)
    } else {
        format!("{} at {}", exception_type, crash_address)
    };

    let severity = if offset.to_uppercase() == "D6DDDA" {
        CrashSeverity::Critical
    } else if module.to_lowercase() != "skyrimse.exe" && module != "Unknown" {
        CrashSeverity::High
    } else {
        CrashSeverity::Medium
    };

    (summary, severity)
}

/// Check whether a DLL name is a common Windows system DLL.
fn is_system_dll(name: &str) -> bool {
    let system_dlls = [
        "ntdll.dll",
        "kernel32.dll",
        "kernelbase.dll",
        "user32.dll",
        "gdi32.dll",
        "advapi32.dll",
        "msvcrt.dll",
        "ucrtbase.dll",
        "vcruntime140.dll",
        "msvcp140.dll",
        "d3d11.dll",
        "dxgi.dll",
        "xinput1_3.dll",
        "xinput9_1_0.dll",
        "win32u.dll",
        "combase.dll",
        "rpcrt4.dll",
        "sechost.dll",
        "bcrypt.dll",
        "ws2_32.dll",
    ];
    system_dlls.contains(&name)
}

/// Extract text between two delimiter strings.
fn extract_between(s: &str, start: &str, end: &str) -> Option<String> {
    let start_idx = s.find(start)?;
    let after_start = start_idx + start.len();
    let end_idx = s[after_start..].find(end)?;
    Some(s[after_start..after_start + end_idx].to_string())
}

/// Get the text after the first colon in a line, trimmed.
fn after_colon(s: &str) -> &str {
    if let Some(idx) = s.find(':') {
        s[idx + 1..].trim()
    } else {
        s.trim()
    }
}

/// Parse a memory line like "8192 MB / 16384 MB" or "8192MB/16384MB".
fn parse_memory_line(s: &str) -> Option<(u64, u64)> {
    // Look for the colon-separated value portion.
    let value_part = after_colon(s);

    // Split by "/" to get used/total.
    let parts: Vec<&str> = value_part.split('/').collect();
    if parts.len() != 2 {
        return None;
    }

    let used = extract_number(parts[0])?;
    let total = extract_number(parts[1])?;
    Some((used, total))
}

/// Extract the first numeric value from a string.
fn extract_number(s: &str) -> Option<u64> {
    let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    digits.parse::<u64>().ok()
}

/// Extract a type name from a relevant object line.
///
/// Lines may look like:
/// `(1)  NiNode "NPC Root [Root]"  (FormID: 00014)`
/// `[2]  BSLightingShaderProperty  mymod.esp`
fn extract_type_name(line: &str) -> String {
    // Skip the leading index (number in brackets or parens).
    let stripped = line.trim_start_matches(|c: char| {
        c == '[' || c == '(' || c.is_ascii_digit() || c == ']' || c == ')' || c == ' '
    });

    // The type name is the first whitespace-delimited token.
    stripped.split_whitespace().next().unwrap_or("").to_string()
}

/// Extract a FormID from a relevant object line, if present.
fn extract_form_id(line: &str) -> Option<String> {
    // Look for "FormID: XXXXX" or "(FormID: XXXXX)"
    let lower = line.to_lowercase();
    if let Some(idx) = lower.find("formid") {
        // Skip past the "formid" keyword itself (6 chars), then skip any
        // non-hex-digit characters (colon, spaces, parens) to reach the value.
        let after_keyword = &line[idx + 6..];
        let value_start = after_keyword.find(|c: char| c.is_ascii_hexdigit());
        if let Some(start) = value_start {
            let value: String = after_keyword[start..]
                .chars()
                .take_while(|c| c.is_ascii_hexdigit())
                .collect();
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

/// Extract a source plugin filename from a relevant object line, if present.
fn extract_source_plugin(line: &str) -> Option<String> {
    // Look for anything ending in .esm, .esp, or .esl.
    for token in line.split_whitespace() {
        let cleaned =
            token.trim_matches(|c: char| c == '"' || c == '(' || c == ')' || c == '[' || c == ']');
        let lower = cleaned.to_lowercase();
        if lower.ends_with(".esm") || lower.ends_with(".esp") || lower.ends_with(".esl") {
            return Some(cleaned.to_string());
        }
    }
    None
}

/// Check if a pattern appears within a specific section of the log.
fn is_in_section(content: &str, section_name: &str, pattern: &str) -> bool {
    let mut in_section = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.contains(section_name) {
            in_section = true;
            continue;
        }

        if in_section {
            if trimmed.is_empty() {
                in_section = false;
                continue;
            }
            if trimmed.contains(pattern) {
                return true;
            }
        }
    }

    false
}

/// Find a file path reference with a given extension in the content.
///
/// Returns the first path-like string ending with the given extension.
fn find_file_reference(content: &str, extension: &str) -> Option<String> {
    for line in content.lines() {
        let lower = line.to_lowercase();
        if !lower.contains(extension) {
            continue;
        }

        // Try to extract a file path containing the extension.
        for token in line.split_whitespace() {
            let cleaned = token.trim_matches(|c: char| {
                c == '"' || c == '\'' || c == '(' || c == ')' || c == '[' || c == ']'
            });
            if cleaned.to_lowercase().ends_with(extension) && cleaned.contains('/')
                || cleaned.contains('\\')
            {
                return Some(cleaned.to_string());
            }
        }

        // Fallback: look for "path/file.ext" patterns in the line.
        if let Some(idx) = lower.find(extension) {
            // Walk backwards from the extension to find the start of the path.
            let before = &line[..idx + extension.len()];
            let path_start = before.rfind(['"', '\'', ' ', '(', '[']);
            let start = match path_start {
                Some(i) => i + 1,
                None => 0,
            };
            let candidate = before[start..].trim();
            if candidate.len() > extension.len() + 1 {
                return Some(candidate.to_string());
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

    // -----------------------------------------------------------------------
    // Realistic test crash log content
    // -----------------------------------------------------------------------

    const SAMPLE_CRASH_LOG: &str = r#"Crash Logger SSE v1.8.0
Skyrim SE v1.6.640.0
2024-03-15 14:23:45

OS: Windows 10 v10.0.19041
CPU: AMD Ryzen 7 5800X
GPU: NVIDIA GeForce RTX 3080
Physical Memory: 12288 MB / 32768 MB
Video Memory: 6144 MB / 10240 MB

Unhandled exception "EXCEPTION_ACCESS_VIOLATION" at 0x7FF6D1B0DDDA SkyrimSE.exe+D6DDDA

PROBABLE CALL STACK:
[0]  0x7FF6D1B0DDDA  SkyrimSE.exe+D6DDDA
[1]  0x7FF6D0E01234  SkyrimSE.exe+A01234
[2]  0x7FF6D0F05678  SkyrimSE.exe+B05678

REGISTERS:
RAX 0x0000000000000000  (void*)
RBX 0x00000254A8B0C120  (BSResource::CompressedArchiveStream*)
RCX 0x0000000000000000  (void*)

POSSIBLE RELEVANT OBJECTS:
[1]  NiNode "NPC Root [Root]"  (FormID: 00000014)  Skyrim.esm
[2]  BSFadeNode  (FormID: 000A2B3C)  MyMod.esp

MODULES:
SkyrimSE.exe
ntdll.dll
kernel32.dll
hdtSMP64.dll
skee64.dll

SKSE PLUGINS:
hdtSMP64 v2.2.1
RaceMenu v0.4.19.4
JContainers64 v4.2.6

GAME PLUGINS:
[00] Skyrim.esm
[01] Update.esm
[02] Dawnguard.esm
[03] HearthFires.esm
[04] Dragonborn.esm
[FE:000] MyMod.esp
[FE:001] AnotherMod.esp
"#;

    const SAMPLE_HDT_CRASH: &str = r#"Crash Logger SSE v1.8.0
Skyrim SE v1.6.640.0
2024-03-14 10:15:30

Unhandled exception "EXCEPTION_ACCESS_VIOLATION" at 0x7FFA12345678 hdtSMP64.dll+ABCDE

PROBABLE CALL STACK:
[0]  0x7FFA12345678  hdtSMP64.dll+ABCDE
[1]  0x7FFA12340000  hdtSMP64.dll+10000
[2]  0x7FF6D0E01234  SkyrimSE.exe+A01234

REGISTERS:
RAX 0x0000000000000000  (void*)

POSSIBLE RELEVANT OBJECTS:
[1]  NiNode "NPC Root [Root]"  skeleton.nif  Skyrim.esm

SKSE PLUGINS:
hdtSMP64 v2.2.1

GAME PLUGINS:
[00] Skyrim.esm
[01] Update.esm
"#;

    const SAMPLE_SHADOW_CRASH: &str = r#"Crash Logger SSE v1.8.0
2024-02-28 09:00:00

Unhandled exception "EXCEPTION_ACCESS_VIOLATION" at 0x7FF6D2F0DD00 SkyrimSE.exe+12FDD00

PROBABLE CALL STACK:
[0]  0x7FF6D2F0DD00  SkyrimSE.exe+12FDD00
[1]  0x7FF6D0E05555  SkyrimSE.exe+A05555

REGISTERS:
RAX 0x00000254A8B0C120  (ShadowSceneNode*)

POSSIBLE RELEVANT OBJECTS:
[1]  ShadowSceneNode  Skyrim.esm

SKSE PLUGINS:

GAME PLUGINS:
[00] Skyrim.esm
[01] ELFX.esp
"#;

    const SAMPLE_BAD_ALLOC: &str = r#"Crash Logger SSE v1.8.0

Unhandled exception "EXCEPTION_ACCESS_VIOLATION" at 0x7FF6DEADBEEF SkyrimSE.exe+AABBCC

PROBABLE CALL STACK:
[0]  0x7FF6DEADBEEF  SkyrimSE.exe+AABBCC
[1]  0x7FF6D0001111  tbbmalloc.dll+1111

REGISTERS:
RAX bad_alloc

POSSIBLE RELEVANT OBJECTS:

SKSE PLUGINS:

GAME PLUGINS:
[00] Skyrim.esm
"#;

    const SAMPLE_ANIMATION_CRASH: &str = r#"Crash Logger SSE v1.8.0

Unhandled exception "EXCEPTION_ACCESS_VIOLATION" at 0x7FF6D1A0D789 SkyrimSE.exe+A0D789

PROBABLE CALL STACK:
[0]  0x7FF6D1A0D789  SkyrimSE.exe+A0D789
[1]  0x7FF6D0E09999  SkyrimSE.exe+A09999

REGISTERS:
RAX 0x0000000000000000  (void*)

POSSIBLE RELEVANT OBJECTS:
[1]  NiNode "NPC Root [Root]"  (FormID: 00000014)  Skyrim.esm

SKSE PLUGINS:

GAME PLUGINS:
[00] Skyrim.esm
"#;

    const SAMPLE_NIF_CRASH: &str = r#"Crash Logger SSE v1.8.0

Unhandled exception "EXCEPTION_ACCESS_VIOLATION" at 0x7FF6D1CB748E SkyrimSE.exe+CB748E

PROBABLE CALL STACK:
[0]  0x7FF6D1CB748E  SkyrimSE.exe+CB748E
[1]  0x7FF6D0E01111  SkyrimSE.exe+A01111

REGISTERS:
RAX 0x0000000000000000  (void*)
RBX 0x00000254A8000000  (BSLightingShaderProperty*)

POSSIBLE RELEVANT OBJECTS:
[1]  BSLightingShaderProperty  meshes/actors/character/myarmor.nif  MyArmor.esp

SKSE PLUGINS:

GAME PLUGINS:
[00] Skyrim.esm
[01] MyArmor.esp
"#;

    const SAMPLE_STRINGS_CRASH: &str = r#"Crash Logger SSE v1.8.0

Unhandled exception "EXCEPTION_ACCESS_VIOLATION" at 0x7FF6D1FFFFFF SkyrimSE.exe+FFFFFF

PROBABLE CALL STACK:
[0]  0x7FF6D1FFFFFF  SkyrimSE.exe+FFFFFF

REGISTERS:
RAX 0x0000025400001234  (BSLocalizedString*)
RBX 0x0000000000000000  Skyrim_English.STRINGS

POSSIBLE RELEVANT OBJECTS:

SKSE PLUGINS:

GAME PLUGINS:
[00] Skyrim.esm
"#;

    const MALFORMED_LOG: &str = r#"This is not a crash log at all.
Just some random text.
Nothing to see here.
"#;

    const EMPTY_LOG: &str = "";

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_header_standard() {
        let (exception_type, crash_address) = parse_header(SAMPLE_CRASH_LOG);
        assert_eq!(exception_type, "EXCEPTION_ACCESS_VIOLATION");
        assert!(crash_address.contains("SkyrimSE.exe+D6DDDA"));
    }

    #[test]
    fn test_parse_header_hdt() {
        let (exception_type, crash_address) = parse_header(SAMPLE_HDT_CRASH);
        assert_eq!(exception_type, "EXCEPTION_ACCESS_VIOLATION");
        assert!(crash_address.contains("hdtSMP64.dll+ABCDE"));
    }

    #[test]
    fn test_parse_call_stack() {
        let frames = parse_call_stack(SAMPLE_CRASH_LOG);
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].0, "SkyrimSE.exe");
        assert_eq!(frames[0].1, "D6DDDA");
        assert_eq!(frames[1].0, "SkyrimSE.exe");
        assert_eq!(frames[1].1, "A01234");
    }

    #[test]
    fn test_parse_call_stack_hdt() {
        let frames = parse_call_stack(SAMPLE_HDT_CRASH);
        assert!(frames.len() >= 2);
        assert_eq!(frames[0].0, "hdtSMP64.dll");
        assert_eq!(frames[0].1, "ABCDE");
    }

    #[test]
    fn test_pattern_match_memory_exhaustion() {
        let diagnosis = match_crash_patterns(
            "SkyrimSE.exe",
            "D6DDDA",
            &[("SkyrimSE.exe".to_string(), "D6DDDA".to_string())],
            &[],
            SAMPLE_CRASH_LOG,
        );
        assert!(!diagnosis.is_empty());
        assert!(diagnosis
            .iter()
            .any(|d| d.title.contains("Memory Exhaustion")));
        let mem_diag = diagnosis
            .iter()
            .find(|d| d.title.contains("Memory Exhaustion"))
            .unwrap();
        assert_eq!(mem_diag.confidence, Confidence::High);
    }

    #[test]
    fn test_pattern_match_hdt_smp() {
        let call_stack = vec![
            ("hdtSMP64.dll".to_string(), "ABCDE".to_string()),
            ("SkyrimSE.exe".to_string(), "A01234".to_string()),
        ];
        let diagnosis =
            match_crash_patterns("hdtSMP64.dll", "ABCDE", &call_stack, &[], SAMPLE_HDT_CRASH);
        assert!(diagnosis.iter().any(|d| d.title.contains("HDT-SMP")));
    }

    #[test]
    fn test_pattern_match_shadow_scene_node() {
        let diagnosis = match_crash_patterns(
            "SkyrimSE.exe",
            "12FDD00",
            &[("SkyrimSE.exe".to_string(), "12FDD00".to_string())],
            &[],
            SAMPLE_SHADOW_CRASH,
        );
        assert!(diagnosis
            .iter()
            .any(|d| d.title.contains("Shadow Scene Node")));
    }

    #[test]
    fn test_pattern_match_bad_alloc() {
        let call_stack = vec![
            ("SkyrimSE.exe".to_string(), "AABBCC".to_string()),
            ("tbbmalloc.dll".to_string(), "1111".to_string()),
        ];
        let diagnosis =
            match_crash_patterns("SkyrimSE.exe", "AABBCC", &call_stack, &[], SAMPLE_BAD_ALLOC);
        assert!(diagnosis.iter().any(|d| d.title.contains("Out of Memory")));
        assert!(diagnosis
            .iter()
            .any(|d| d.title.contains("Memory Allocator")));
    }

    #[test]
    fn test_pattern_match_animation_limit() {
        let diagnosis = match_crash_patterns(
            "SkyrimSE.exe",
            "A0D789",
            &[("SkyrimSE.exe".to_string(), "A0D789".to_string())],
            &[],
            SAMPLE_ANIMATION_CRASH,
        );
        assert!(diagnosis
            .iter()
            .any(|d| d.title.contains("Animation Limit")));
    }

    #[test]
    fn test_pattern_match_broken_nif_and_shader() {
        let objects = vec![RelevantObject {
            type_name: "BSLightingShaderProperty".to_string(),
            form_id: None,
            source_plugin: Some("MyArmor.esp".to_string()),
            raw: "BSLightingShaderProperty  meshes/actors/character/myarmor.nif  MyArmor.esp"
                .to_string(),
        }];
        let diagnosis = match_crash_patterns(
            "SkyrimSE.exe",
            "CB748E",
            &[("SkyrimSE.exe".to_string(), "CB748E".to_string())],
            &objects,
            SAMPLE_NIF_CRASH,
        );
        assert!(diagnosis.iter().any(|d| d.title.contains("Broken NIF")));
        assert!(diagnosis
            .iter()
            .any(|d| d.title.contains("Lighting Shader")));
    }

    #[test]
    fn test_pattern_match_strings_encoding() {
        let diagnosis = match_crash_patterns(
            "SkyrimSE.exe",
            "FFFFFF",
            &[("SkyrimSE.exe".to_string(), "FFFFFF".to_string())],
            &[],
            SAMPLE_STRINGS_CRASH,
        );
        // The .STRINGS content is in the registers section, should be caught.
        assert!(
            diagnosis
                .iter()
                .any(|d| d.title.contains("String Encoding")),
            "Expected a String Encoding diagnosis, got: {:?}",
            diagnosis.iter().map(|d| &d.title).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_parse_game_plugins() {
        let plugins = parse_game_plugins(SAMPLE_CRASH_LOG);
        assert!(plugins.contains(&"Skyrim.esm".to_string()));
        assert!(plugins.contains(&"Update.esm".to_string()));
        assert!(plugins.contains(&"Dawnguard.esm".to_string()));
        assert!(plugins.contains(&"MyMod.esp".to_string()));
        assert!(plugins.contains(&"AnotherMod.esp".to_string()));
    }

    #[test]
    fn test_parse_skse_plugins() {
        let plugins = parse_skse_plugins(SAMPLE_CRASH_LOG);
        assert!(plugins.contains(&"hdtSMP64".to_string()));
        assert!(plugins.contains(&"RaceMenu".to_string()));
        assert!(plugins.contains(&"JContainers64".to_string()));
    }

    #[test]
    fn test_parse_system_info() {
        let info = parse_system_info(SAMPLE_CRASH_LOG);
        assert!(info.is_some());
        let info = info.unwrap();
        assert_eq!(info.os.as_deref(), Some("Windows 10 v10.0.19041"));
        assert_eq!(info.cpu.as_deref(), Some("AMD Ryzen 7 5800X"));
        assert_eq!(info.gpu.as_deref(), Some("NVIDIA GeForce RTX 3080"));
        assert_eq!(info.ram_used_mb, Some(12288));
        assert_eq!(info.ram_total_mb, Some(32768));
        assert_eq!(info.vram_used_mb, Some(6144));
        assert_eq!(info.vram_total_mb, Some(10240));
    }

    #[test]
    fn test_parse_relevant_objects() {
        let objects = parse_relevant_objects(SAMPLE_CRASH_LOG);
        assert!(objects.len() >= 2);
        assert!(objects.iter().any(|o| o.type_name == "NiNode"));
        assert!(objects
            .iter()
            .any(|o| o.source_plugin.as_deref() == Some("Skyrim.esm")));
        assert!(objects
            .iter()
            .any(|o| o.source_plugin.as_deref() == Some("MyMod.esp")));
    }

    #[test]
    fn test_malformed_log_returns_invalid_format() {
        let tmp = tempfile::tempdir().unwrap();
        let log_path = tmp.path().join("crash-2024-01-01-00-00-00.log");
        fs::write(&log_path, MALFORMED_LOG).unwrap();

        let result = analyze_crash_log(&log_path);
        assert!(result.is_err());
        match result.unwrap_err() {
            CrashLogError::InvalidFormat => {}
            other => panic!("Expected InvalidFormat, got: {:?}", other),
        }
    }

    #[test]
    fn test_empty_log_returns_invalid_format() {
        let tmp = tempfile::tempdir().unwrap();
        let log_path = tmp.path().join("crash-2024-01-01-00-00-00.log");
        fs::write(&log_path, EMPTY_LOG).unwrap();

        let result = analyze_crash_log(&log_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_full_analysis_produces_report() {
        let tmp = tempfile::tempdir().unwrap();
        let log_path = tmp.path().join("crash-2024-03-15-14-23-45.log");
        fs::write(&log_path, SAMPLE_CRASH_LOG).unwrap();

        let report = analyze_crash_log(&log_path).unwrap();

        assert_eq!(report.exception_type, "EXCEPTION_ACCESS_VIOLATION");
        assert_eq!(report.module_name, "SkyrimSE.exe");
        assert_eq!(report.module_offset, "D6DDDA");
        assert!(!report.diagnosis.is_empty());
        assert!(report
            .diagnosis
            .iter()
            .any(|d| d.title.contains("Memory Exhaustion")));
        assert_eq!(report.severity, CrashSeverity::Critical);
        assert!(report.involved_plugins.contains(&"Skyrim.esm".to_string()));
        assert!(report.involved_plugins.contains(&"MyMod.esp".to_string()));
        assert!(!report.call_stack_summary.is_empty());
        assert!(report.system_info.is_some());
    }

    #[test]
    fn test_full_analysis_hdt_crash() {
        let tmp = tempfile::tempdir().unwrap();
        let log_path = tmp.path().join("crash-2024-03-14-10-15-30.log");
        fs::write(&log_path, SAMPLE_HDT_CRASH).unwrap();

        let report = analyze_crash_log(&log_path).unwrap();

        assert_eq!(report.module_name, "hdtSMP64.dll");
        assert!(report.diagnosis.iter().any(|d| d.title.contains("HDT-SMP")));
        assert!(report
            .involved_skse_plugins
            .iter()
            .any(|p| p.contains("hdtSMP64")));
    }

    #[test]
    fn test_find_crash_logs_in_directory() {
        let tmp = tempfile::tempdir().unwrap();

        // Create a fake bottle structure with SKSE log dir.
        let skse_dir = tmp
            .path()
            .join("drive_c")
            .join("users")
            .join("crossover")
            .join("Documents")
            .join("My Games")
            .join("Skyrim Special Edition")
            .join("SKSE");
        fs::create_dir_all(&skse_dir).unwrap();

        // Write two crash logs and one non-crash file.
        fs::write(
            skse_dir.join("crash-2024-03-15-14-23-45.log"),
            SAMPLE_CRASH_LOG,
        )
        .unwrap();
        fs::write(
            skse_dir.join("crash-2024-03-14-10-15-30.log"),
            SAMPLE_HDT_CRASH,
        )
        .unwrap();
        fs::write(skse_dir.join("skse64.log"), "Not a crash log").unwrap();

        let game_path = tmp.path().join("drive_c").join("Games").join("Skyrim");
        let entries = find_crash_logs(&game_path, tmp.path(), "skyrimse");

        assert_eq!(entries.len(), 2);
        // Most recent first.
        assert_eq!(entries[0].timestamp, "2024-03-15-14-23-45");
        assert_eq!(entries[1].timestamp, "2024-03-14-10-15-30");
        assert!(!entries[0].summary.is_empty());
    }

    #[test]
    fn test_find_crash_logs_empty_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let game_path = tmp.path().join("games").join("skyrim");
        let entries = find_crash_logs(&game_path, tmp.path(), "skyrimse");
        assert!(entries.is_empty());
    }

    #[test]
    fn test_skse_log_dir_path() {
        let bottle = PathBuf::from("/home/user/.wine/my_bottle");
        let dir = skse_log_dir(&bottle);
        assert_eq!(
            dir,
            PathBuf::from(
                "/home/user/.wine/my_bottle/drive_c/users/crossover/Documents/My Games/Skyrim Special Edition/SKSE"
            )
        );
    }

    #[test]
    fn test_extract_between() {
        assert_eq!(
            extract_between(r#"foo "bar" baz"#, "\"", "\""),
            Some("bar".to_string())
        );
        assert_eq!(extract_between("no quotes here", "\"", "\""), None);
    }

    #[test]
    fn test_extract_form_id() {
        assert_eq!(
            extract_form_id("[1]  NiNode (FormID: 00000014)  Skyrim.esm"),
            Some("00000014".to_string())
        );
        assert_eq!(extract_form_id("[1]  NiNode  Skyrim.esm"), None);
    }

    #[test]
    fn test_extract_source_plugin() {
        assert_eq!(
            extract_source_plugin("[1]  NiNode (FormID: 00000014)  Skyrim.esm"),
            Some("Skyrim.esm".to_string())
        );
        assert_eq!(
            extract_source_plugin("[2]  BSFadeNode  MyMod.esp"),
            Some("MyMod.esp".to_string())
        );
        assert_eq!(extract_source_plugin("[1]  NiNode"), None);
    }

    #[test]
    fn test_severity_determination() {
        // Critical: memory exhaustion
        let critical = vec![CrashDiagnosis {
            title: "Memory Exhaustion (D6DDDA)".to_string(),
            description: String::new(),
            confidence: Confidence::High,
            suggested_actions: vec![],
        }];
        assert_eq!(determine_severity(&critical), CrashSeverity::Critical);

        // High: high confidence non-critical
        let high = vec![CrashDiagnosis {
            title: "HDT-SMP Physics Crash".to_string(),
            description: String::new(),
            confidence: Confidence::High,
            suggested_actions: vec![],
        }];
        assert_eq!(determine_severity(&high), CrashSeverity::High);

        // Medium
        let medium = vec![CrashDiagnosis {
            title: "Something Medium".to_string(),
            description: String::new(),
            confidence: Confidence::Medium,
            suggested_actions: vec![],
        }];
        assert_eq!(determine_severity(&medium), CrashSeverity::Medium);

        // Unknown when empty
        assert_eq!(determine_severity(&[]), CrashSeverity::Unknown);
    }

    #[test]
    fn test_is_system_dll() {
        assert!(is_system_dll("ntdll.dll"));
        assert!(is_system_dll("kernel32.dll"));
        assert!(!is_system_dll("hdtSMP64.dll"));
        assert!(!is_system_dll("skee64.dll"));
    }

    #[test]
    fn test_parse_memory_line() {
        assert_eq!(
            parse_memory_line("Physical Memory: 12288 MB / 32768 MB"),
            Some((12288, 32768))
        );
        assert_eq!(
            parse_memory_line("Video Memory: 6144 MB / 10240 MB"),
            Some((6144, 10240))
        );
        assert_eq!(parse_memory_line("No memory info here"), None);
    }

    #[test]
    fn test_quick_summary_memory_exhaustion() {
        let (summary, severity) = quick_summary(SAMPLE_CRASH_LOG);
        assert!(summary.contains("D6DDDA"));
        assert!(summary.contains("Memory exhaustion"));
        assert_eq!(severity, CrashSeverity::Critical);
    }

    #[test]
    fn test_quick_summary_hdt() {
        let (summary, severity) = quick_summary(SAMPLE_HDT_CRASH);
        assert!(summary.contains("hdtSMP64.dll"));
        assert_eq!(severity, CrashSeverity::High);
    }
}
