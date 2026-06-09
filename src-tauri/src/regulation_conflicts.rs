//! Regulation.bin conflict detection for FromSoft games.
//!
//! `regulation.bin` is the master gameplay-rules archive used by Sekiro,
//! Elden Ring, Dark Souls 3, and Armored Core 6. Two enabled mods both
//! shipping their own `regulation.bin` will silently overwrite each
//! other at deploy time, leading to subtle in-game breakage. We detect
//! this and surface a banner.
//!
//! We're detecting only — not merging. A merge implementation would
//! require parsing every param table per game (huge RE work). Detection
//! alone is high-value: users either disable one mod or open Smithbox /
//! Yapped to merge manually.
//!
//! The check inspects `installed_files` JSON on each enabled mod and
//! reports a conflict when ≥2 mods include a path ending in
//! `regulation.bin`.

use serde::{Deserialize, Serialize};

use crate::database::ModDatabase;

/// One conflict over `regulation.bin` for a given (game_id, bottle).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegulationConflict {
    pub game_id: String,
    pub mod_ids_modifying_regulation: Vec<i64>,
    pub mod_names: Vec<String>,
}

/// Inspect installed mods for the given (game_id, bottle_name) and report
/// any cases where ≥2 enabled mods modify `regulation.bin`.
///
/// Returns an empty Vec when there are 0 or 1 conflicting mods. When ≥2
/// mods conflict, returns a single `RegulationConflict` listing all of
/// them — we treat the whole conflict set as one item rather than N(N-1)/2
/// pairs, because the resolution is "pick one".
pub fn detect_regulation_conflicts(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
) -> Result<Vec<RegulationConflict>, String> {
    let mods = db
        .list_mods(game_id, bottle_name)
        .map_err(|e| format!("Failed to list mods: {}", e))?;

    let mut ids: Vec<i64> = Vec::new();
    let mut names: Vec<String> = Vec::new();

    for m in mods {
        if !m.enabled {
            continue;
        }
        if mod_modifies_regulation(&m.installed_files) {
            ids.push(m.id);
            names.push(m.name);
        }
    }

    if ids.len() >= 2 {
        Ok(vec![RegulationConflict {
            game_id: game_id.to_string(),
            mod_ids_modifying_regulation: ids,
            mod_names: names,
        }])
    } else {
        Ok(Vec::new())
    }
}

/// Predicate: does this `installed_files` list include a path ending in
/// `regulation.bin` (case-insensitive, slash-normalized)?
pub fn mod_modifies_regulation(installed_files: &[String]) -> bool {
    installed_files.iter().any(|f| is_regulation_path(f))
}

fn is_regulation_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/").to_lowercase();
    normalized == "regulation.bin" || normalized.ends_with("/regulation.bin")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::ModDatabase;
    use std::sync::Arc;

    fn temp_db() -> (Arc<ModDatabase>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Arc::new(ModDatabase::new(&db_path).expect("open db"));
        (db, dir)
    }

    fn add_mod(
        db: &ModDatabase,
        game_id: &str,
        bottle: &str,
        name: &str,
        files: &[&str],
        enabled: bool,
    ) -> i64 {
        let files_owned: Vec<String> = files.iter().map(|s| s.to_string()).collect();
        let id = db
            .add_mod(
                game_id,
                bottle,
                None,
                name,
                "1.0",
                &format!("{}.zip", name),
                &files_owned,
            )
            .expect("add mod");
        if !enabled {
            db.set_enabled(id, false).expect("toggle");
        }
        id
    }

    #[test]
    fn predicate_matches_regulation_only() {
        assert!(is_regulation_path("regulation.bin"));
        assert!(is_regulation_path("Regulation.BIN"));
        assert!(is_regulation_path("subdir/regulation.bin"));
        assert!(is_regulation_path("subdir\\regulation.bin"));
        assert!(!is_regulation_path("regulationoptional.bin"));
        assert!(!is_regulation_path("regulation.bin.bak"));
        assert!(!is_regulation_path("noregulation.bin"));
        assert!(!is_regulation_path("foo/bar.bin"));
    }

    #[test]
    fn no_conflicts_when_no_mods() {
        let (db, _t) = temp_db();
        let conflicts = detect_regulation_conflicts(&db, "eldenring", "default").expect("ok");
        assert!(conflicts.is_empty());
    }

    #[test]
    fn no_conflicts_for_single_modifier() {
        let (db, _t) = temp_db();
        add_mod(
            &db,
            "eldenring",
            "default",
            "OnlyOne",
            &["mod/A/regulation.bin"],
            true,
        );
        let conflicts = detect_regulation_conflicts(&db, "eldenring", "default").expect("ok");
        assert!(conflicts.is_empty());
    }

    #[test]
    fn conflict_when_two_enabled_mods_modify_regulation() {
        let (db, _t) = temp_db();
        add_mod(
            &db,
            "eldenring",
            "default",
            "GTS",
            &["mod/GTS/regulation.bin"],
            true,
        );
        add_mod(
            &db,
            "eldenring",
            "default",
            "Convergence",
            &["mod/Convergence/regulation.bin"],
            true,
        );
        // One enabled non-regulation mod for noise.
        add_mod(
            &db,
            "eldenring",
            "default",
            "Reshade",
            &["mod/Reshade/d3d11.dll"],
            true,
        );

        let conflicts = detect_regulation_conflicts(&db, "eldenring", "default").expect("ok");
        assert_eq!(conflicts.len(), 1);
        let c = &conflicts[0];
        assert_eq!(c.game_id, "eldenring");
        assert_eq!(c.mod_ids_modifying_regulation.len(), 2);
        assert!(c.mod_names.contains(&"GTS".to_string()));
        assert!(c.mod_names.contains(&"Convergence".to_string()));
        assert!(!c.mod_names.contains(&"Reshade".to_string()));
    }

    #[test]
    fn disabled_mod_excluded_from_conflict() {
        let (db, _t) = temp_db();
        add_mod(
            &db,
            "sekiro",
            "default",
            "Active",
            &["mod/A/regulation.bin"],
            true,
        );
        // Disabled mod with regulation.bin should NOT count.
        add_mod(
            &db,
            "sekiro",
            "default",
            "Disabled",
            &["mod/B/regulation.bin"],
            false,
        );

        let conflicts = detect_regulation_conflicts(&db, "sekiro", "default").expect("ok");
        assert!(conflicts.is_empty(), "disabled mod must not contribute");
    }

    #[test]
    fn three_way_conflict_groups_into_single_item() {
        let (db, _t) = temp_db();
        for name in ["A", "B", "C"] {
            add_mod(
                &db,
                "darksouls3",
                "default",
                name,
                &[&format!("mod/{}/regulation.bin", name)],
                true,
            );
        }
        let conflicts = detect_regulation_conflicts(&db, "darksouls3", "default").expect("ok");
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].mod_ids_modifying_regulation.len(), 3);
    }
}
