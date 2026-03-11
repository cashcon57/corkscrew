//! Tier 1: Deterministic regex/pattern-based instruction parser.
//!
//! Parses common collection author instruction formats into structured
//! `ConditionalAction`s with 100% confidence. Falls through to unparsed
//! for anything it cannot confidently match.

use regex::Regex;
use std::sync::LazyLock;

use crate::instruction_types::*;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse collection instructions into structured actions.
///
/// Returns parsed actions (confidence 1.0) and any lines that could not
/// be deterministically parsed (for LLM or manual fallback).
pub fn parse_instructions(raw_text: &str, available_mods: &[String]) -> ParsedInstructions {
    let lines = preprocess(raw_text);
    let mut actions = Vec::new();
    let mut unparsed = Vec::new();

    for line in &lines {
        if line.trim().is_empty() {
            continue;
        }
        if let Some(action) = try_parse_line(line, available_mods) {
            actions.push(action);
        } else {
            unparsed.push(line.clone());
        }
    }

    let fully_parsed = unparsed.is_empty() && !actions.is_empty();
    ParsedInstructions {
        actions,
        unparsed_lines: unparsed,
        source: ParseSource::Deterministic,
        fully_parsed,
    }
}

// ---------------------------------------------------------------------------
// Preprocessing
// ---------------------------------------------------------------------------

/// Strip markdown formatting, split into logical lines, skip headings/dividers.
fn preprocess(raw: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();

        // Skip empty lines, markdown headings, horizontal rules, images
        if trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed.starts_with("---")
            || trimmed.starts_with("***")
            || trimmed.starts_with("![")
        {
            continue;
        }

        // Strip markdown bold/italic/links
        let cleaned = strip_markdown(trimmed);
        let cleaned = cleaned.trim();
        if !cleaned.is_empty() {
            lines.push(cleaned.to_string());
        }
    }
    lines
}

/// Remove common markdown formatting inline.
fn strip_markdown(text: &str) -> String {
    static RE_BOLD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\*\*(.+?)\*\*").unwrap());
    static RE_ITALIC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\*(.+?)\*").unwrap());
    static RE_LINK: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\([^)]+\)").unwrap());
    static RE_CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`([^`]+)`").unwrap());
    static RE_BULLET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[-*•]\s+").unwrap());

    let s = RE_LINK.replace_all(text, "$1");
    let s = RE_BOLD.replace_all(&s, "$1");
    let s = RE_ITALIC.replace_all(&s, "$1");
    let s = RE_CODE.replace_all(&s, "$1");
    let s = RE_BULLET.replace_all(&s, "");
    s.to_string()
}

// ---------------------------------------------------------------------------
// Pattern matchers
// ---------------------------------------------------------------------------

fn try_parse_line(line: &str, available_mods: &[String]) -> Option<ConditionalAction> {
    // Try each matcher in order; first match wins.
    None.or_else(|| match_enable_all_optional(line))
        .or_else(|| match_disable_all_optional(line))
        .or_else(|| match_enable_if(line, available_mods))
        .or_else(|| match_disable_if(line, available_mods))
        .or_else(|| match_enable_disable_simple(line, available_mods))
        .or_else(|| match_fomod_choice(line, available_mods))
        .or_else(|| match_ini_setting(line))
        .or_else(|| match_load_order(line))
        .or_else(|| match_requires_mod(line))
        .or_else(|| match_manual_download(line))
}

// --- Enable/disable all optional ---

fn match_enable_all_optional(line: &str) -> Option<ConditionalAction> {
    static RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)enable\s+all\s+optional\b").unwrap());

    if !RE.is_match(line) {
        return None;
    }

    let condition = extract_condition(line);
    Some(ConditionalAction {
        action: InstructionAction::EnableAllOptional,
        condition,
        source_text: line.to_string(),
        confidence: 1.0,
    })
}

fn match_disable_all_optional(line: &str) -> Option<ConditionalAction> {
    static RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)disable\s+all\s+optional\b").unwrap());

    if !RE.is_match(line) {
        return None;
    }

    let condition = extract_condition(line);
    Some(ConditionalAction {
        action: InstructionAction::DisableAllOptional,
        condition,
        source_text: line.to_string(),
        confidence: 1.0,
    })
}

// --- Enable/disable with condition ---

fn match_enable_if(line: &str, available_mods: &[String]) -> Option<ConditionalAction> {
    // Patterns: "enable X if you have AE", "X should be enabled for AE users",
    //           "enable X on Wine/Proton"
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)enable\s+['"]?(.+?)['"]?\s+(?:if|when|for|on)\b"#).unwrap()
    });

    let caps = RE.captures(line)?;
    let mod_name_raw = caps.get(1)?.as_str().trim();
    let mod_name = fuzzy_match_mod(mod_name_raw, available_mods)?;
    let condition = extract_condition(line);

    // Only match if we found a meaningful condition
    if condition == InstructionCondition::Always {
        return None;
    }

    Some(ConditionalAction {
        action: InstructionAction::EnableMod { mod_name },
        condition,
        source_text: line.to_string(),
        confidence: 1.0,
    })
}

fn match_disable_if(line: &str, available_mods: &[String]) -> Option<ConditionalAction> {
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)disable\s+['"]?(.+?)['"]?\s+(?:if|when|for|on)\b"#).unwrap()
    });

    let caps = RE.captures(line)?;
    let mod_name_raw = caps.get(1)?.as_str().trim();
    let mod_name = fuzzy_match_mod(mod_name_raw, available_mods)?;
    let condition = extract_condition(line);

    if condition == InstructionCondition::Always {
        return None;
    }

    Some(ConditionalAction {
        action: InstructionAction::DisableMod { mod_name },
        condition,
        source_text: line.to_string(),
        confidence: 1.0,
    })
}

// --- Simple enable/disable (no condition) ---

fn match_enable_disable_simple(line: &str, available_mods: &[String]) -> Option<ConditionalAction> {
    static RE_ENABLE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)^(?:please\s+)?enable\s+['"]?(.+?)['"]?\s*$"#).unwrap());
    static RE_DISABLE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)^(?:please\s+)?disable\s+['"]?(.+?)['"]?\s*$"#).unwrap()
    });

    if let Some(caps) = RE_ENABLE.captures(line) {
        let name = caps.get(1)?.as_str().trim();
        let mod_name = fuzzy_match_mod(name, available_mods)?;
        return Some(ConditionalAction {
            action: InstructionAction::EnableMod { mod_name },
            condition: InstructionCondition::Always,
            source_text: line.to_string(),
            confidence: 1.0,
        });
    }

    if let Some(caps) = RE_DISABLE.captures(line) {
        let name = caps.get(1)?.as_str().trim();
        let mod_name = fuzzy_match_mod(name, available_mods)?;
        return Some(ConditionalAction {
            action: InstructionAction::DisableMod { mod_name },
            condition: InstructionCondition::Always,
            source_text: line.to_string(),
            confidence: 1.0,
        });
    }

    None
}

// --- FOMOD choices ---

fn match_fomod_choice(line: &str, available_mods: &[String]) -> Option<ConditionalAction> {
    // Patterns: "Select 'Lite' preset in FOMOD for X", "Choose performance option for X"
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)(?:select|choose|pick)\s+['"]?(.+?)['"]?\s+(?:preset|option|in\s+(?:the\s+)?fomod)\s+(?:for|in|during)\s+['"]?(.+?)['"]?\s*$"#).unwrap()
    });

    let caps = RE.captures(line)?;
    let option = caps.get(1)?.as_str().trim().to_string();
    let mod_name_raw = caps.get(2)?.as_str().trim();
    let mod_name = fuzzy_match_mod(mod_name_raw, available_mods)?;
    let condition = extract_condition(line);

    Some(ConditionalAction {
        action: InstructionAction::SetFomodChoice {
            mod_name,
            step: None,
            group: None,
            option,
        },
        condition,
        source_text: line.to_string(),
        confidence: 1.0,
    })
}

// --- INI settings ---

fn match_ini_setting(line: &str) -> Option<ConditionalAction> {
    // Patterns: "Set bFoo=1 in Skyrim.ini [General]", "Add bBar=0 to SkyrimPrefs.ini under [Display]"
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)(?:set|add|change)\s+(\w+)\s*=\s*(\S+)\s+(?:in|to)\s+(\w+\.ini)\s*(?:\[(\w+)\]|under\s+\[(\w+)\])").unwrap()
    });

    let caps = RE.captures(line)?;
    let key = caps.get(1)?.as_str().to_string();
    let value = caps.get(2)?.as_str().to_string();
    let file = caps.get(3)?.as_str().to_string();
    let section = caps.get(4).or(caps.get(5))?.as_str().to_string();
    let condition = extract_condition(line);

    Some(ConditionalAction {
        action: InstructionAction::SetIniSetting {
            file,
            section,
            key,
            value,
        },
        condition,
        source_text: line.to_string(),
        confidence: 1.0,
    })
}

// --- Load order ---

fn match_load_order(line: &str) -> Option<ConditionalAction> {
    // "Load X after Y", "Place X before Y", "X at the bottom of load order"
    static RE_AFTER: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)(?:load|place|put)\s+(\S+\.es[pml])\s+after\s+(\S+\.es[pml])").unwrap()
    });
    static RE_BEFORE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)(?:load|place|put)\s+(\S+\.es[pml])\s+before\s+(\S+\.es[pml])").unwrap()
    });
    static RE_BOTTOM: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?i)(\S+\.es[pml])\s+(?:at\s+)?(?:the\s+)?bottom\s+(?:of\s+)?(?:the\s+)?load\s*order",
        )
        .unwrap()
    });

    if let Some(caps) = RE_AFTER.captures(line) {
        let plugin = caps.get(1)?.as_str().to_string();
        let reference = caps.get(2)?.as_str().to_string();
        return Some(ConditionalAction {
            action: InstructionAction::SetLoadOrder {
                plugin,
                position: LoadOrderPosition::After { reference },
            },
            condition: extract_condition(line),
            source_text: line.to_string(),
            confidence: 1.0,
        });
    }

    if let Some(caps) = RE_BEFORE.captures(line) {
        let plugin = caps.get(1)?.as_str().to_string();
        let reference = caps.get(2)?.as_str().to_string();
        return Some(ConditionalAction {
            action: InstructionAction::SetLoadOrder {
                plugin,
                position: LoadOrderPosition::Before { reference },
            },
            condition: extract_condition(line),
            source_text: line.to_string(),
            confidence: 1.0,
        });
    }

    if let Some(caps) = RE_BOTTOM.captures(line) {
        let plugin = caps.get(1)?.as_str().to_string();
        return Some(ConditionalAction {
            action: InstructionAction::SetLoadOrder {
                plugin,
                position: LoadOrderPosition::Bottom,
            },
            condition: extract_condition(line),
            source_text: line.to_string(),
            confidence: 1.0,
        });
    }

    None
}

// --- Requires mod (informational) ---

fn match_requires_mod(line: &str) -> Option<ConditionalAction> {
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)requires?\s+['"]?(.+?)['"]?\s*(?:to\s+(?:be\s+)?install|is\s+required|must\s+be)"#).unwrap()
    });

    let caps = RE.captures(line)?;
    let mod_name = caps.get(1)?.as_str().trim().to_string();

    Some(ConditionalAction {
        action: InstructionAction::ManualStep {
            description: format!("Ensure '{}' is installed", mod_name),
            url: None,
        },
        condition: InstructionCondition::Always,
        source_text: line.to_string(),
        confidence: 1.0,
    })
}

// --- Manual download / external link ---

fn match_manual_download(line: &str) -> Option<ConditionalAction> {
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)(?:download|get|grab)\s+(?:from|at)\s+(https?://\S+)").unwrap()
    });

    let caps = RE.captures(line)?;
    let url = caps.get(1)?.as_str().to_string();

    Some(ConditionalAction {
        action: InstructionAction::ManualStep {
            description: line.to_string(),
            url: Some(url),
        },
        condition: InstructionCondition::Always,
        source_text: line.to_string(),
        confidence: 1.0,
    })
}

// ---------------------------------------------------------------------------
// Condition extraction
// ---------------------------------------------------------------------------

/// Extract a condition from the line text (looks for AE/SE, Wine/Proton, DLC keywords).
fn extract_condition(line: &str) -> InstructionCondition {
    let lower = line.to_lowercase();
    let mut conditions: Vec<InstructionCondition> = Vec::new();

    // Game version conditions
    if lower.contains("anniversary")
        || lower.contains(" ae ")
        || lower.contains(" ae,")
        || lower.contains(" ae.")
        || lower.contains("(ae)")
        || lower.contains("1.6.")
        || lower.ends_with(" ae")
    {
        conditions.push(InstructionCondition::GameVersion {
            version: GameVersionMatch::Ae,
        });
    } else if (lower.contains("special edition") && !lower.contains("anniversary"))
        || lower.contains(" se ")
        || lower.contains(" se,")
        || lower.contains("(se)")
        || lower.contains("1.5.")
        || lower.ends_with(" se")
    {
        conditions.push(InstructionCondition::GameVersion {
            version: GameVersionMatch::Se,
        });
    }

    // Platform conditions
    if lower.contains("wine") || lower.contains("crossover") {
        if lower.contains("proton") {
            conditions.push(InstructionCondition::Platform {
                platform: PlatformMatch::WineOrProton,
            });
        } else {
            conditions.push(InstructionCondition::Platform {
                platform: PlatformMatch::Wine,
            });
        }
    } else if lower.contains("proton") {
        conditions.push(InstructionCondition::Platform {
            platform: PlatformMatch::Proton,
        });
    } else if lower.contains("native") || lower.contains("windows only") {
        conditions.push(InstructionCondition::Platform {
            platform: PlatformMatch::Native,
        });
    }

    // DLC conditions
    for (keyword, dlc_name) in &[
        ("dawnguard", "Dawnguard"),
        ("hearthfire", "Hearthfire"),
        ("dragonborn", "Dragonborn"),
    ] {
        if lower.contains(keyword) {
            conditions.push(InstructionCondition::DlcPresent {
                dlc_name: dlc_name.to_string(),
            });
        }
    }

    match conditions.len() {
        0 => InstructionCondition::Always,
        1 => conditions.into_iter().next().unwrap(),
        _ => InstructionCondition::All { conditions },
    }
}

// ---------------------------------------------------------------------------
// Fuzzy mod name matching
// ---------------------------------------------------------------------------

/// Try to match a name from instructions against the available mod list.
/// Returns the actual mod name if a close enough match is found.
fn fuzzy_match_mod(query: &str, available_mods: &[String]) -> Option<String> {
    if available_mods.is_empty() {
        // If no mod list provided, accept the raw name.
        return Some(query.to_string());
    }

    let query_lower = query.to_lowercase();

    // Exact match (case-insensitive)
    if let Some(exact) = available_mods
        .iter()
        .find(|m| m.to_lowercase() == query_lower)
    {
        return Some(exact.clone());
    }

    // Substring match: query is contained in a mod name
    let substring_matches: Vec<&String> = available_mods
        .iter()
        .filter(|m| m.to_lowercase().contains(&query_lower))
        .collect();
    if substring_matches.len() == 1 {
        return Some(substring_matches[0].clone());
    }

    // Reverse substring: mod name is contained in query
    let reverse_matches: Vec<&String> = available_mods
        .iter()
        .filter(|m| query_lower.contains(&m.to_lowercase()))
        .collect();
    if reverse_matches.len() == 1 {
        return Some(reverse_matches[0].clone());
    }

    // Levenshtein distance for short names (likely typos)
    if query.len() <= 30 {
        let mut best_score = 0.0f64;
        let mut best_match: Option<&String> = None;

        for mod_name in available_mods {
            let score = normalized_similarity(&query_lower, &mod_name.to_lowercase());
            if score > best_score {
                best_score = score;
                best_match = Some(mod_name);
            }
        }

        // Require >70% similarity for fuzzy matches
        if best_score > 0.70 {
            return best_match.cloned();
        }
    }

    None
}

/// Normalized string similarity (0.0–1.0) using Levenshtein distance.
fn normalized_similarity(a: &str, b: &str) -> f64 {
    let max_len = a.len().max(b.len());
    if max_len == 0 {
        return 1.0;
    }
    let dist = levenshtein(a, b);
    1.0 - (dist as f64 / max_len as f64)
}

/// Basic Levenshtein distance implementation.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (m, n) = (a.len(), b.len());

    let mut prev = (0..=n).collect::<Vec<_>>();
    let mut curr = vec![0; n + 1];

    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[n]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn mods() -> Vec<String> {
        vec![
            "Immersive Armors".into(),
            "SkyUI".into(),
            "USSEP".into(),
            "Community Shaders".into(),
            "Sky Sync - Community Shaders".into(),
        ]
    }

    #[test]
    fn test_enable_all_optional_ae() {
        let result = parse_instructions("Enable all optional mods if you have AE", &mods());
        assert_eq!(result.actions.len(), 1);
        assert_eq!(
            result.actions[0].action,
            InstructionAction::EnableAllOptional
        );
        assert_eq!(
            result.actions[0].condition,
            InstructionCondition::GameVersion {
                version: GameVersionMatch::Ae
            }
        );
    }

    #[test]
    fn test_disable_mod_wine() {
        let result = parse_instructions("Disable Community Shaders if on Wine or Proton", &mods());
        assert_eq!(result.actions.len(), 1);
        match &result.actions[0].action {
            InstructionAction::DisableMod { mod_name } => {
                assert_eq!(mod_name, "Community Shaders");
            }
            _ => panic!("Expected DisableMod"),
        }
    }

    #[test]
    fn test_ini_setting() {
        let result = parse_instructions("Set bFXAAEnabled=0 in SkyrimPrefs.ini [Display]", &[]);
        assert_eq!(result.actions.len(), 1);
        match &result.actions[0].action {
            InstructionAction::SetIniSetting {
                file,
                section,
                key,
                value,
            } => {
                assert_eq!(file, "SkyrimPrefs.ini");
                assert_eq!(section, "Display");
                assert_eq!(key, "bFXAAEnabled");
                assert_eq!(value, "0");
            }
            _ => panic!("Expected SetIniSetting"),
        }
    }

    #[test]
    fn test_fuzzy_match() {
        assert_eq!(
            fuzzy_match_mod("community shaders", &mods()),
            Some("Community Shaders".into())
        );
        assert_eq!(
            fuzzy_match_mod("Immersive Armor", &mods()),
            Some("Immersive Armors".into())
        );
    }

    #[test]
    fn test_unparsed_lines() {
        let result = parse_instructions(
            "This is a random comment that makes no sense as an instruction",
            &mods(),
        );
        assert!(result.actions.is_empty());
        assert!(!result.unparsed_lines.is_empty());
        assert!(!result.fully_parsed);
    }
}
