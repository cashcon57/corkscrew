//! Validation layer for parsed instruction actions.
//!
//! Every action (whether from deterministic parser or LLM) passes through
//! validation before execution. This is the critical safety net against
//! LLM hallucinations.

use crate::database::ModDatabase;
use crate::instruction_types::*;

/// Validate a list of parsed actions against the actual game state.
pub fn validate_actions(
    actions: &[ConditionalAction],
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
) -> Vec<ValidatedAction> {
    let installed_mods = db
        .list_mods_summary(game_id, bottle_name)
        .unwrap_or_default();
    let mod_names: Vec<String> = installed_mods.iter().map(|m| m.name.clone()).collect();

    actions
        .iter()
        .map(|action| validate_single(action, &mod_names, &installed_mods))
        .collect()
}

fn validate_single(
    action: &ConditionalAction,
    _mod_names: &[String],
    installed_mods: &[crate::database::ModSummary],
) -> ValidatedAction {
    match &action.action {
        InstructionAction::EnableMod { mod_name }
        | InstructionAction::DisableMod { mod_name } => {
            // Check that the mod exists in the installed mod list
            let matched = find_mod_by_name(mod_name, installed_mods);
            match matched {
                Some(m) => ValidatedAction {
                    action: action.clone(),
                    status: if action.confidence >= 0.8 {
                        ValidationStatus::Valid
                    } else {
                        ValidationStatus::NeedsConfirmation
                    },
                    resolved_mod_id: Some(m.id),
                    reason: None,
                },
                None => ValidatedAction {
                    action: action.clone(),
                    status: ValidationStatus::Rejected,
                    resolved_mod_id: None,
                    reason: Some(format!(
                        "Mod '{}' not found in installed mods",
                        mod_name
                    )),
                },
            }
        }

        InstructionAction::EnableAllOptional | InstructionAction::DisableAllOptional => {
            // Always valid — applies to all optional mods
            ValidatedAction {
                action: action.clone(),
                status: ValidationStatus::Valid,
                resolved_mod_id: None,
                reason: None,
            }
        }

        InstructionAction::SetFomodChoice { mod_name, .. } => {
            let matched = find_mod_by_name(mod_name, installed_mods);
            match matched {
                Some(m) => ValidatedAction {
                    action: action.clone(),
                    status: ValidationStatus::NeedsConfirmation,
                    resolved_mod_id: Some(m.id),
                    reason: Some("FOMOD choice will be applied on next reinstall".into()),
                },
                None => ValidatedAction {
                    action: action.clone(),
                    status: ValidationStatus::Rejected,
                    resolved_mod_id: None,
                    reason: Some(format!("Mod '{}' not found", mod_name)),
                },
            }
        }

        InstructionAction::SetIniSetting { file, section, key, value } => {
            // Validate the INI file name is a known game INI
            let valid_files = [
                "skyrim.ini",
                "skyrimprefs.ini",
                "skyrimcustom.ini",
                "fallout4.ini",
                "fallout4prefs.ini",
                "fallout4custom.ini",
            ];
            let file_lower = file.to_lowercase();
            if valid_files.contains(&file_lower.as_str()) {
                ValidatedAction {
                    action: action.clone(),
                    status: ValidationStatus::NeedsConfirmation,
                    resolved_mod_id: None,
                    reason: Some(format!("Set [{section}] {key}={value} in {file}")),
                }
            } else {
                ValidatedAction {
                    action: action.clone(),
                    status: ValidationStatus::Rejected,
                    resolved_mod_id: None,
                    reason: Some(format!("Unknown INI file: {file}")),
                }
            }
        }

        InstructionAction::SetLoadOrder { plugin, .. } => {
            // Validate plugin filename looks like an ESP/ESM/ESL
            let lower = plugin.to_lowercase();
            if lower.ends_with(".esp") || lower.ends_with(".esm") || lower.ends_with(".esl") {
                ValidatedAction {
                    action: action.clone(),
                    status: ValidationStatus::NeedsConfirmation,
                    resolved_mod_id: None,
                    reason: None,
                }
            } else {
                ValidatedAction {
                    action: action.clone(),
                    status: ValidationStatus::Rejected,
                    resolved_mod_id: None,
                    reason: Some(format!("'{}' is not a valid plugin filename", plugin)),
                }
            }
        }

        InstructionAction::ManualStep { .. } => {
            // Manual steps are always valid — they just show info
            ValidatedAction {
                action: action.clone(),
                status: ValidationStatus::Valid,
                resolved_mod_id: None,
                reason: None,
            }
        }
    }
}

/// Find a mod by name (case-insensitive, with substring fallback).
fn find_mod_by_name<'a>(
    name: &str,
    mods: &'a [crate::database::ModSummary],
) -> Option<&'a crate::database::ModSummary> {
    let lower = name.to_lowercase();

    // Exact match (case-insensitive)
    if let Some(m) = mods.iter().find(|m| m.name.to_lowercase() == lower) {
        return Some(m);
    }

    // Substring match (query contained in mod name)
    let matches: Vec<&crate::database::ModSummary> = mods
        .iter()
        .filter(|m| m.name.to_lowercase().contains(&lower))
        .collect();
    if matches.len() == 1 {
        return Some(matches[0]);
    }

    None
}

/// Check whether a condition is satisfied in the current environment.
pub fn evaluate_condition(
    condition: &InstructionCondition,
    game_version: &str,
    platform: &str,
    _installed_dlcs: &[String],
    installed_mod_names: &[String],
) -> bool {
    match condition {
        InstructionCondition::Always => true,

        InstructionCondition::GameVersion { version } => match version {
            GameVersionMatch::Ae => {
                game_version.starts_with("1.6") || game_version.to_lowercase().contains("ae")
            }
            GameVersionMatch::Se => {
                game_version.starts_with("1.5") || game_version.to_lowercase().contains("se")
            }
            GameVersionMatch::Pattern(pat) => game_version.contains(pat),
        },

        InstructionCondition::Platform { platform: p } => {
            let is_wine = platform == "wine" || platform == "crossover";
            let is_proton = platform == "proton";
            match p {
                PlatformMatch::Wine => is_wine,
                PlatformMatch::Proton => is_proton,
                PlatformMatch::WineOrProton => is_wine || is_proton,
                PlatformMatch::Native => !is_wine && !is_proton,
            }
        }

        InstructionCondition::DlcPresent { dlc_name } => {
            _installed_dlcs.iter().any(|d| d.to_lowercase() == dlc_name.to_lowercase())
        }

        InstructionCondition::ModInstalled { mod_name } => {
            let lower = mod_name.to_lowercase();
            installed_mod_names.iter().any(|m| m.to_lowercase() == lower)
        }

        InstructionCondition::ModNotInstalled { mod_name } => {
            let lower = mod_name.to_lowercase();
            !installed_mod_names.iter().any(|m| m.to_lowercase() == lower)
        }

        InstructionCondition::All { conditions } => conditions
            .iter()
            .all(|c| evaluate_condition(c, game_version, platform, _installed_dlcs, installed_mod_names)),

        InstructionCondition::Any { conditions } => conditions
            .iter()
            .any(|c| evaluate_condition(c, game_version, platform, _installed_dlcs, installed_mod_names)),
    }
}
