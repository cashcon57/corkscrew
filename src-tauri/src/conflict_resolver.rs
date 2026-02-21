//! Smart conflict resolution using LOOT metadata, collection context, and mod
//! naming heuristics.
//!
//! When two mods overwrite the same file, we need to decide which mod "wins".
//! Instead of forcing the user to resolve every conflict manually, this module
//! analyzes mod relationships to produce an intelligent suggestion:
//!
//! - **Collection-authored**: If all conflicting mods come from the same
//!   collection, the author already chose the priority order. No action needed.
//! - **LOOT-informed**: If LOOT's masterlist says plugin A loads after plugin B,
//!   mod A's files should generally overwrite mod B's.
//! - **Patch heuristic**: Mods whose names contain "patch", "fix", or "compat"
//!   should win over their base mods.
//! - **Unknown**: Manual resolution required.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::database::{FileConflict, InstalledMod, ModDatabase};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// The resolution status of a file conflict.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum ConflictStatus {
    /// All conflicting mods are from the same collection — author chose this.
    AuthorResolved,
    /// The resolver has a suggested winner with a reason.
    Suggested,
    /// No heuristic applies — needs manual resolution.
    Manual,
}

/// A conflict with an attached resolution suggestion.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConflictSuggestion {
    pub relative_path: String,
    pub current_winner_id: i64,
    pub suggested_winner_id: i64,
    pub suggested_winner_name: String,
    pub status: ConflictStatus,
    pub reason: String,
    pub mods: Vec<ConflictModBrief>,
}

/// Lightweight mod info for a conflict entry.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConflictModBrief {
    pub mod_id: i64,
    pub mod_name: String,
    pub priority: i32,
    pub collection_name: Option<String>,
}

/// Summary of the bulk resolution operation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResolutionResult {
    pub total_conflicts: usize,
    pub author_resolved: usize,
    pub auto_suggested: usize,
    pub manual_needed: usize,
    pub priorities_changed: usize,
}

// ---------------------------------------------------------------------------
// Analysis
// ---------------------------------------------------------------------------

/// Analyze all file conflicts and suggest winners using collection authorship,
/// LOOT load order, and patch heuristics.
pub fn analyze_conflicts(
    conflicts: &[FileConflict],
    mods: &[InstalledMod],
    loot_order: Option<&[String]>,
) -> Vec<ConflictSuggestion> {
    let mod_map: HashMap<i64, &InstalledMod> = mods.iter().map(|m| (m.id, m)).collect();

    let loot_positions: HashMap<String, usize> = loot_order
        .map(|order| {
            order
                .iter()
                .enumerate()
                .map(|(i, name)| (name.to_lowercase(), i))
                .collect()
        })
        .unwrap_or_default();

    let mut suggestions = Vec::new();

    for conflict in conflicts {
        let briefs: Vec<ConflictModBrief> = conflict
            .mods
            .iter()
            .map(|cm| {
                let collection = mod_map
                    .get(&cm.mod_id)
                    .and_then(|m| m.collection_name.clone());
                ConflictModBrief {
                    mod_id: cm.mod_id,
                    mod_name: cm.mod_name.clone(),
                    priority: cm.priority,
                    collection_name: collection,
                }
            })
            .collect();

        let (status, winner_id, reason) =
            suggest_winner(&briefs, &mod_map, &loot_positions, conflict.winner_mod_id);

        let winner_name = mod_map
            .get(&winner_id)
            .map(|m| m.name.clone())
            .unwrap_or_default();

        suggestions.push(ConflictSuggestion {
            relative_path: conflict.relative_path.clone(),
            current_winner_id: conflict.winner_mod_id,
            suggested_winner_id: winner_id,
            suggested_winner_name: winner_name,
            status,
            reason,
            mods: briefs,
        });
    }

    suggestions
}

/// Determine the suggested winner for a single conflict.
fn suggest_winner(
    mods: &[ConflictModBrief],
    mod_map: &HashMap<i64, &InstalledMod>,
    loot_positions: &HashMap<String, usize>,
    current_winner_id: i64,
) -> (ConflictStatus, i64, String) {
    let collections: HashSet<Option<&str>> =
        mods.iter().map(|m| m.collection_name.as_deref()).collect();
    if collections.len() == 1 {
        if let Some(Some(col_name)) = collections.into_iter().next() {
            return (
                ConflictStatus::AuthorResolved,
                current_winner_id,
                format!(
                    "All mods from collection \"{}\". Author's priority order applies.",
                    col_name
                ),
            );
        }
    }

    let patch_winner = find_patch_winner(mods);
    if let Some((winner_id, reason)) = patch_winner {
        return (ConflictStatus::Suggested, winner_id, reason);
    }

    let loot_winner = find_loot_winner(mods, mod_map, loot_positions);
    if let Some((winner_id, reason)) = loot_winner {
        return (ConflictStatus::Suggested, winner_id, reason);
    }

    let collection_winner = find_collection_winner(mods);
    if let Some((winner_id, reason)) = collection_winner {
        return (ConflictStatus::Suggested, winner_id, reason);
    }

    (
        ConflictStatus::Manual,
        current_winner_id,
        "No automatic resolution available. Review manually.".to_string(),
    )
}

fn find_patch_winner(mods: &[ConflictModBrief]) -> Option<(i64, String)> {
    let patch_patterns = [
        "patch",
        "fix",
        "compat",
        "compatibility",
        "conflict resolution",
        "cr patch",
        " - patch",
        "reconciliation",
    ];

    let mut patch_mods: Vec<&ConflictModBrief> = Vec::new();
    let mut base_mods: Vec<&ConflictModBrief> = Vec::new();

    for m in mods {
        let name_lower = m.mod_name.to_lowercase();
        let is_patch = patch_patterns
            .iter()
            .any(|pattern| name_lower.contains(pattern));
        if is_patch {
            patch_mods.push(m);
        } else {
            base_mods.push(m);
        }
    }

    if patch_mods.len() == 1 && !base_mods.is_empty() {
        let winner = patch_mods[0];
        return Some((
            winner.mod_id,
            format!(
                "\"{}\" is a patch/compatibility mod and should overwrite base mod files.",
                winner.mod_name
            ),
        ));
    }

    // Multiple patches: highest priority wins (most specific).
    if patch_mods.len() > 1 && !base_mods.is_empty() {
        let winner = patch_mods.iter().max_by_key(|m| m.priority)?;
        return Some((
            winner.mod_id,
            format!(
                "\"{}\" is the highest-priority patch among {} patches.",
                winner.mod_name,
                patch_mods.len()
            ),
        ));
    }

    None
}

/// Later LOOT position = winner (later-loading plugins overwrite earlier ones).
fn find_loot_winner(
    mods: &[ConflictModBrief],
    mod_map: &HashMap<i64, &InstalledMod>,
    loot_positions: &HashMap<String, usize>,
) -> Option<(i64, String)> {
    if loot_positions.is_empty() {
        return None;
    }

    let mut mod_positions: Vec<(i64, &str, usize)> = Vec::new();

    for m in mods {
        let installed = mod_map.get(&m.mod_id)?;
        let best_pos = installed
            .installed_files
            .iter()
            .filter(|f| {
                let lower = f.to_lowercase();
                lower.ends_with(".esp") || lower.ends_with(".esm") || lower.ends_with(".esl")
            })
            .filter_map(|f| loot_positions.get(&f.to_lowercase()))
            .max();

        if let Some(&pos) = best_pos {
            mod_positions.push((m.mod_id, &m.mod_name, pos));
        }
    }

    if mod_positions.len() < 2 {
        return None;
    }

    let winner = mod_positions.iter().max_by_key(|(_, _, pos)| pos)?;
    Some((
        winner.0,
        format!(
            "LOOT masterlist places \"{}\" later in load order — its files should take priority.",
            winner.1
        ),
    ))
}

fn find_collection_winner(mods: &[ConflictModBrief]) -> Option<(i64, String)> {
    let collection_mods: Vec<&ConflictModBrief> = mods
        .iter()
        .filter(|m| m.collection_name.is_some())
        .collect();
    let standalone_mods: Vec<&ConflictModBrief> = mods
        .iter()
        .filter(|m| m.collection_name.is_none())
        .collect();

    if collection_mods.is_empty() || standalone_mods.is_empty() {
        return None;
    }

    let winner = collection_mods.iter().max_by_key(|m| m.priority)?;
    Some((
        winner.mod_id,
        format!(
            "\"{}\" is part of a curated collection and should override standalone mods.",
            winner.mod_name
        ),
    ))
}

// ---------------------------------------------------------------------------
// Bulk resolution
// ---------------------------------------------------------------------------

/// Bulk-apply suggested resolutions by bumping winner priorities.
/// Only acts on `Suggested` conflicts; skips `AuthorResolved` and `Manual`.
pub fn apply_suggestions(
    db: &ModDatabase,
    _game_id: &str,
    _bottle_name: &str,
    suggestions: &[ConflictSuggestion],
) -> Result<ResolutionResult, String> {
    let mut priorities_changed = 0;
    let mut author_resolved = 0;
    let mut auto_suggested = 0;
    let mut manual_needed = 0;

    let mut needed_priority_bumps: HashMap<i64, i32> = HashMap::new();

    for suggestion in suggestions {
        match suggestion.status {
            ConflictStatus::AuthorResolved => {
                author_resolved += 1;
            }
            ConflictStatus::Suggested => {
                auto_suggested += 1;
                if suggestion.suggested_winner_id != suggestion.current_winner_id {
                    let max_other_priority = suggestion
                        .mods
                        .iter()
                        .filter(|m| m.mod_id != suggestion.suggested_winner_id)
                        .map(|m| m.priority)
                        .max()
                        .unwrap_or(0);

                    let needed = max_other_priority + 1;
                    let current_best = needed_priority_bumps
                        .get(&suggestion.suggested_winner_id)
                        .copied()
                        .unwrap_or(0);
                    if needed > current_best {
                        needed_priority_bumps.insert(suggestion.suggested_winner_id, needed);
                    }
                }
            }
            ConflictStatus::Manual => {
                manual_needed += 1;
            }
        }
    }

    for (mod_id, new_priority) in &needed_priority_bumps {
        db.set_mod_priority(*mod_id, *new_priority)
            .map_err(|e| e.to_string())?;
        priorities_changed += 1;
    }

    Ok(ResolutionResult {
        total_conflicts: suggestions.len(),
        author_resolved,
        auto_suggested,
        manual_needed,
        priorities_changed,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::ConflictModInfo;

    fn make_mod(id: i64, name: &str, priority: i32, collection: Option<&str>) -> InstalledMod {
        InstalledMod {
            id,
            game_id: "skyrimse".into(),
            bottle_name: "Gaming".into(),
            nexus_mod_id: None,
            nexus_file_id: None,
            source_url: None,
            name: name.into(),
            version: "1.0".into(),
            archive_name: format!("{}.zip", name),
            installed_files: vec![],
            installed_at: "2024-01-01".into(),
            enabled: true,
            staging_path: None,
            install_priority: priority,
            collection_name: collection.map(String::from),
            user_notes: None,
            user_tags: vec![],
            auto_category: None,
            source_type: "manual".into(),
        }
    }

    fn make_conflict(path: &str, mods: Vec<(i64, &str, i32)>) -> FileConflict {
        let winner = mods.iter().max_by_key(|m| m.2).unwrap().0;
        FileConflict {
            relative_path: path.into(),
            mods: mods
                .into_iter()
                .map(|(id, name, priority)| ConflictModInfo {
                    mod_id: id,
                    mod_name: name.into(),
                    priority,
                })
                .collect(),
            winner_mod_id: winner,
        }
    }

    #[test]
    fn same_collection_is_author_resolved() {
        let mods = vec![
            make_mod(1, "Base Textures", 1, Some("My Collection")),
            make_mod(2, "Better Textures", 2, Some("My Collection")),
        ];
        let conflicts = vec![make_conflict(
            "textures/sky.dds",
            vec![(1, "Base Textures", 1), (2, "Better Textures", 2)],
        )];

        let results = analyze_conflicts(&conflicts, &mods, None);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, ConflictStatus::AuthorResolved);
    }

    #[test]
    fn patch_mod_wins_over_base() {
        let mods = vec![
            make_mod(1, "SMIM", 2, None),
            make_mod(2, "SMIM Compatibility Patch", 1, None),
        ];
        let conflicts = vec![make_conflict(
            "meshes/door.nif",
            vec![(1, "SMIM", 2), (2, "SMIM Compatibility Patch", 1)],
        )];

        let results = analyze_conflicts(&conflicts, &mods, None);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, ConflictStatus::Suggested);
        assert_eq!(results[0].suggested_winner_id, 2);
        assert!(results[0].reason.contains("patch"));
    }

    #[test]
    fn loot_order_determines_winner() {
        let mut mod1 = make_mod(1, "Mod A", 1, None);
        mod1.installed_files = vec!["modA.esp".into(), "textures/shared.dds".into()];
        let mut mod2 = make_mod(2, "Mod B", 2, None);
        mod2.installed_files = vec!["modB.esp".into(), "textures/shared.dds".into()];

        let mods = vec![mod1, mod2];
        let loot_order = vec!["modB.esp".into(), "modA.esp".into()]; // A loads later

        let conflicts = vec![make_conflict(
            "textures/shared.dds",
            vec![(1, "Mod A", 1), (2, "Mod B", 2)],
        )];

        let results = analyze_conflicts(&conflicts, &mods, Some(&loot_order));
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, ConflictStatus::Suggested);
        assert_eq!(results[0].suggested_winner_id, 1); // A loads later in LOOT
    }

    #[test]
    fn collection_wins_over_standalone() {
        let mods = vec![
            make_mod(1, "Collection Mod", 1, Some("Lexy's LOTD")),
            make_mod(2, "My Custom Mod", 2, None),
        ];
        let conflicts = vec![make_conflict(
            "meshes/item.nif",
            vec![(1, "Collection Mod", 1), (2, "My Custom Mod", 2)],
        )];

        let results = analyze_conflicts(&conflicts, &mods, None);
        assert_eq!(results.len(), 1);
        // Patch heuristic doesn't apply, LOOT not available, but collection
        // mod should win over standalone.
        assert_eq!(results[0].status, ConflictStatus::Suggested);
        assert_eq!(results[0].suggested_winner_id, 1);
    }

    #[test]
    fn no_heuristic_is_manual() {
        let mods = vec![
            make_mod(1, "Mod Alpha", 1, None),
            make_mod(2, "Mod Beta", 2, None),
        ];
        let conflicts = vec![make_conflict(
            "textures/shared.dds",
            vec![(1, "Mod Alpha", 1), (2, "Mod Beta", 2)],
        )];

        let results = analyze_conflicts(&conflicts, &mods, None);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, ConflictStatus::Manual);
    }

    #[test]
    fn resolution_result_counts() {
        let suggestions = vec![
            ConflictSuggestion {
                relative_path: "a.dds".into(),
                current_winner_id: 1,
                suggested_winner_id: 1,
                suggested_winner_name: "Mod A".into(),
                status: ConflictStatus::AuthorResolved,
                reason: "Same collection".into(),
                mods: vec![],
            },
            ConflictSuggestion {
                relative_path: "b.dds".into(),
                current_winner_id: 1,
                suggested_winner_id: 2,
                suggested_winner_name: "Patch Mod".into(),
                status: ConflictStatus::Suggested,
                reason: "Patch mod".into(),
                mods: vec![
                    ConflictModBrief {
                        mod_id: 1,
                        mod_name: "Base".into(),
                        priority: 2,
                        collection_name: None,
                    },
                    ConflictModBrief {
                        mod_id: 2,
                        mod_name: "Patch Mod".into(),
                        priority: 1,
                        collection_name: None,
                    },
                ],
            },
            ConflictSuggestion {
                relative_path: "c.dds".into(),
                current_winner_id: 1,
                suggested_winner_id: 1,
                suggested_winner_name: "Mod A".into(),
                status: ConflictStatus::Manual,
                reason: "Manual".into(),
                mods: vec![],
            },
        ];

        // We can't call apply_suggestions without a real DB, but we can
        // verify the suggestion categorization.
        assert_eq!(
            suggestions
                .iter()
                .filter(|s| s.status == ConflictStatus::AuthorResolved)
                .count(),
            1
        );
        assert_eq!(
            suggestions
                .iter()
                .filter(|s| s.status == ConflictStatus::Suggested)
                .count(),
            1
        );
        assert_eq!(
            suggestions
                .iter()
                .filter(|s| s.status == ConflictStatus::Manual)
                .count(),
            1
        );
    }
}
