//! Mod dependency graph tracking.
//!
//! Allows users to declare relationships between mods (requires, conflicts,
//! patches) and warns when disabling or removing a mod would break dependents.

use serde::Serialize;

use crate::database::ModDatabase;

/// A dependency relationship between two mods.
#[derive(Clone, Debug, Serialize)]
pub struct ModDependency {
    pub id: i64,
    pub game_id: String,
    pub bottle_name: String,
    pub mod_id: i64,
    pub depends_on_id: Option<i64>,
    pub nexus_dep_id: Option<i64>,
    pub dep_name: String,
    pub relationship: String,
    pub created_at: String,
}

/// Dependency validation issue.
#[derive(Clone, Debug, Serialize)]
pub struct DependencyIssue {
    pub mod_id: i64,
    pub mod_name: String,
    pub issue_type: DependencyIssueType,
    pub message: String,
    pub related_mod_name: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DependencyIssueType {
    MissingRequirement,
    ActiveConflict,
    OrphanedPatch,
}

/// Add a dependency relationship.
#[allow(clippy::too_many_arguments)]
pub fn add_dependency(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    mod_id: i64,
    depends_on_id: Option<i64>,
    nexus_dep_id: Option<i64>,
    dep_name: &str,
    relationship: &str,
) -> Result<i64, String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().to_rfc3339();

    conn.execute(
        "INSERT INTO mod_dependencies
            (game_id, bottle_name, mod_id, depends_on_id, nexus_dep_id, dep_name, relationship, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![game_id, bottle_name, mod_id, depends_on_id, nexus_dep_id, dep_name, relationship, now],
    ).map_err(|e| e.to_string())?;

    Ok(conn.last_insert_rowid())
}

/// Remove a dependency relationship.
pub fn remove_dependency(db: &ModDatabase, dep_id: i64) -> Result<(), String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM mod_dependencies WHERE id = ?1", [dep_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Get all dependencies for a specific mod.
pub fn get_dependencies(db: &ModDatabase, mod_id: i64) -> Result<Vec<ModDependency>, String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, game_id, bottle_name, mod_id, depends_on_id, nexus_dep_id, dep_name, relationship, created_at
             FROM mod_dependencies WHERE mod_id = ?1",
        )
        .map_err(|e| e.to_string())?;

    let deps = stmt
        .query_map([mod_id], |row| {
            Ok(ModDependency {
                id: row.get(0)?,
                game_id: row.get(1)?,
                bottle_name: row.get(2)?,
                mod_id: row.get(3)?,
                depends_on_id: row.get(4)?,
                nexus_dep_id: row.get(5)?,
                dep_name: row.get(6)?,
                relationship: row.get(7)?,
                created_at: row.get(8)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    Ok(deps)
}

/// Get all dependencies in a game/bottle.
pub fn get_all_dependencies(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
) -> Result<Vec<ModDependency>, String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, game_id, bottle_name, mod_id, depends_on_id, nexus_dep_id, dep_name, relationship, created_at
             FROM mod_dependencies WHERE game_id = ?1 AND bottle_name = ?2",
        )
        .map_err(|e| e.to_string())?;

    let deps = stmt
        .query_map(rusqlite::params![game_id, bottle_name], |row| {
            Ok(ModDependency {
                id: row.get(0)?,
                game_id: row.get(1)?,
                bottle_name: row.get(2)?,
                mod_id: row.get(3)?,
                depends_on_id: row.get(4)?,
                nexus_dep_id: row.get(5)?,
                dep_name: row.get(6)?,
                relationship: row.get(7)?,
                created_at: row.get(8)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    Ok(deps)
}

/// Get mods that depend on a specific mod (reverse lookup).
pub fn get_dependents(db: &ModDatabase, mod_id: i64) -> Result<Vec<ModDependency>, String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, game_id, bottle_name, mod_id, depends_on_id, nexus_dep_id, dep_name, relationship, created_at
             FROM mod_dependencies WHERE depends_on_id = ?1",
        )
        .map_err(|e| e.to_string())?;

    let deps = stmt
        .query_map([mod_id], |row| {
            Ok(ModDependency {
                id: row.get(0)?,
                game_id: row.get(1)?,
                bottle_name: row.get(2)?,
                mod_id: row.get(3)?,
                depends_on_id: row.get(4)?,
                nexus_dep_id: row.get(5)?,
                dep_name: row.get(6)?,
                relationship: row.get(7)?,
                created_at: row.get(8)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    Ok(deps)
}

/// Check all dependency relationships and return issues.
pub fn check_dependency_issues(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
) -> Result<Vec<DependencyIssue>, String> {
    let deps = get_all_dependencies(db, game_id, bottle_name)?;
    let mods = db
        .list_mods(game_id, bottle_name)
        .map_err(|e| e.to_string())?;

    let mut issues = Vec::new();

    for dep in &deps {
        let source_mod = mods.iter().find(|m| m.id == dep.mod_id);
        let source_name = source_mod
            .map(|m| m.name.clone())
            .unwrap_or_else(|| format!("Mod #{}", dep.mod_id));
        let source_enabled = source_mod.map(|m| m.enabled).unwrap_or(false);

        match dep.relationship.as_str() {
            "requires" => {
                if source_enabled {
                    // Check if the required mod is installed and enabled
                    if let Some(target_id) = dep.depends_on_id {
                        let target = mods.iter().find(|m| m.id == target_id);
                        match target {
                            None => {
                                issues.push(DependencyIssue {
                                    mod_id: dep.mod_id,
                                    mod_name: source_name,
                                    issue_type: DependencyIssueType::MissingRequirement,
                                    message: format!(
                                        "Required mod '{}' is not installed",
                                        dep.dep_name
                                    ),
                                    related_mod_name: dep.dep_name.clone(),
                                });
                            }
                            Some(t) if !t.enabled => {
                                issues.push(DependencyIssue {
                                    mod_id: dep.mod_id,
                                    mod_name: source_name,
                                    issue_type: DependencyIssueType::MissingRequirement,
                                    message: format!("Required mod '{}' is disabled", dep.dep_name),
                                    related_mod_name: dep.dep_name.clone(),
                                });
                            }
                            _ => {}
                        }
                    } else {
                        // No installed mod ID — check by nexus ID
                        if let Some(nexus_id) = dep.nexus_dep_id {
                            let found = mods
                                .iter()
                                .any(|m| m.nexus_mod_id == Some(nexus_id) && m.enabled);
                            if !found {
                                issues.push(DependencyIssue {
                                    mod_id: dep.mod_id,
                                    mod_name: source_name,
                                    issue_type: DependencyIssueType::MissingRequirement,
                                    message: format!(
                                        "Required mod '{}' (Nexus #{}) is not installed or disabled",
                                        dep.dep_name, nexus_id
                                    ),
                                    related_mod_name: dep.dep_name.clone(),
                                });
                            }
                        }
                    }
                }
            }
            "conflicts" => {
                if source_enabled {
                    if let Some(target_id) = dep.depends_on_id {
                        if let Some(target) = mods.iter().find(|m| m.id == target_id) {
                            if target.enabled {
                                issues.push(DependencyIssue {
                                    mod_id: dep.mod_id,
                                    mod_name: source_name,
                                    issue_type: DependencyIssueType::ActiveConflict,
                                    message: format!(
                                        "Conflicts with '{}' — both are enabled",
                                        dep.dep_name
                                    ),
                                    related_mod_name: dep.dep_name.clone(),
                                });
                            }
                        }
                    }
                }
            }
            "patches" => {
                // A patch mod should only be enabled if its target is enabled
                if source_enabled {
                    if let Some(target_id) = dep.depends_on_id {
                        let target = mods.iter().find(|m| m.id == target_id);
                        if target.map(|t| !t.enabled).unwrap_or(true) {
                            issues.push(DependencyIssue {
                                mod_id: dep.mod_id,
                                mod_name: source_name,
                                issue_type: DependencyIssueType::OrphanedPatch,
                                message: format!(
                                    "Patch for '{}' but target mod is disabled or missing",
                                    dep.dep_name
                                ),
                                related_mod_name: dep.dep_name.clone(),
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Ok(issues)
}

/// Clear all dependencies for a specific mod.
pub fn clear_mod_dependencies(db: &ModDatabase, mod_id: i64) -> Result<usize, String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    let count = conn
        .execute("DELETE FROM mod_dependencies WHERE mod_id = ?1", [mod_id])
        .map_err(|e| e.to_string())?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> ModDatabase {
        ModDatabase::new(std::path::Path::new(":memory:")).unwrap()
    }

    fn add_test_mod(db: &ModDatabase, name: &str) -> i64 {
        db.add_mod("skyrimse", "Gaming", None, name, "1.0", "test.zip", &[])
            .unwrap()
    }

    #[test]
    fn add_and_get_dependency() {
        let db = test_db();
        let mod_a = add_test_mod(&db, "Mod A");
        let mod_b = add_test_mod(&db, "Mod B");

        let dep_id = add_dependency(
            &db,
            "skyrimse",
            "Gaming",
            mod_a,
            Some(mod_b),
            None,
            "Mod B",
            "requires",
        )
        .unwrap();
        assert!(dep_id > 0);

        let deps = get_dependencies(&db, mod_a).unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].dep_name, "Mod B");
        assert_eq!(deps[0].relationship, "requires");
    }

    #[test]
    fn add_dependency_returns_unique_ids() {
        let db = test_db();
        let mod_a = add_test_mod(&db, "Mod A");
        let mod_b = add_test_mod(&db, "Mod B");
        let mod_c = add_test_mod(&db, "Mod C");

        let id1 = add_dependency(
            &db,
            "skyrimse",
            "Gaming",
            mod_a,
            Some(mod_b),
            None,
            "Mod B",
            "requires",
        )
        .unwrap();
        let id2 = add_dependency(
            &db,
            "skyrimse",
            "Gaming",
            mod_a,
            Some(mod_c),
            None,
            "Mod C",
            "requires",
        )
        .unwrap();
        assert_ne!(id1, id2);
    }

    #[test]
    fn add_dependency_with_nexus_id() {
        let db = test_db();
        let mod_a = add_test_mod(&db, "Mod A");

        let dep_id = add_dependency(
            &db,
            "skyrimse",
            "Gaming",
            mod_a,
            None,
            Some(12345),
            "SkyUI",
            "requires",
        )
        .unwrap();
        let deps = get_dependencies(&db, mod_a).unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].nexus_dep_id, Some(12345));
        assert!(dep_id > 0);
    }

    #[test]
    fn add_dependency_conflict_type() {
        let db = test_db();
        let mod_a = add_test_mod(&db, "Mod A");
        let mod_b = add_test_mod(&db, "Mod B");

        add_dependency(
            &db,
            "skyrimse",
            "Gaming",
            mod_a,
            Some(mod_b),
            None,
            "Mod B",
            "conflicts",
        )
        .unwrap();
        let deps = get_dependencies(&db, mod_a).unwrap();
        assert_eq!(deps[0].relationship, "conflicts");
    }

    #[test]
    fn remove_dependency_works() {
        let db = test_db();
        let mod_a = add_test_mod(&db, "Mod A");
        let mod_b = add_test_mod(&db, "Mod B");

        let dep_id = add_dependency(
            &db,
            "skyrimse",
            "Gaming",
            mod_a,
            Some(mod_b),
            None,
            "Mod B",
            "requires",
        )
        .unwrap();
        remove_dependency(&db, dep_id).unwrap();

        let deps = get_dependencies(&db, mod_a).unwrap();
        assert!(deps.is_empty());
    }

    #[test]
    fn remove_nonexistent_dependency() {
        let db = test_db();
        // Should not error
        remove_dependency(&db, 99999).unwrap();
    }

    #[test]
    fn remove_dependency_preserves_others() {
        let db = test_db();
        let mod_a = add_test_mod(&db, "Mod A");
        let mod_b = add_test_mod(&db, "Mod B");
        let mod_c = add_test_mod(&db, "Mod C");

        let id1 = add_dependency(
            &db,
            "skyrimse",
            "Gaming",
            mod_a,
            Some(mod_b),
            None,
            "Mod B",
            "requires",
        )
        .unwrap();
        add_dependency(
            &db,
            "skyrimse",
            "Gaming",
            mod_a,
            Some(mod_c),
            None,
            "Mod C",
            "requires",
        )
        .unwrap();

        remove_dependency(&db, id1).unwrap();
        let deps = get_dependencies(&db, mod_a).unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].dep_name, "Mod C");
    }

    #[test]
    fn remove_dependency_double_remove() {
        let db = test_db();
        let mod_a = add_test_mod(&db, "Mod A");
        let mod_b = add_test_mod(&db, "Mod B");

        let dep_id = add_dependency(
            &db,
            "skyrimse",
            "Gaming",
            mod_a,
            Some(mod_b),
            None,
            "Mod B",
            "requires",
        )
        .unwrap();
        remove_dependency(&db, dep_id).unwrap();
        remove_dependency(&db, dep_id).unwrap(); // Second remove should not error
    }

    #[test]
    fn get_dependents_returns_reverse_deps() {
        let db = test_db();
        let mod_a = add_test_mod(&db, "Framework");
        let mod_b = add_test_mod(&db, "Plugin 1");
        let mod_c = add_test_mod(&db, "Plugin 2");

        add_dependency(
            &db,
            "skyrimse",
            "Gaming",
            mod_b,
            Some(mod_a),
            None,
            "Framework",
            "requires",
        )
        .unwrap();
        add_dependency(
            &db,
            "skyrimse",
            "Gaming",
            mod_c,
            Some(mod_a),
            None,
            "Framework",
            "requires",
        )
        .unwrap();

        let dependents = get_dependents(&db, mod_a).unwrap();
        assert_eq!(dependents.len(), 2);
    }

    #[test]
    fn get_dependents_empty() {
        let db = test_db();
        let mod_a = add_test_mod(&db, "Standalone");
        let dependents = get_dependents(&db, mod_a).unwrap();
        assert!(dependents.is_empty());
    }

    #[test]
    fn get_dependents_correct_ids() {
        let db = test_db();
        let mod_a = add_test_mod(&db, "Framework");
        let mod_b = add_test_mod(&db, "Plugin");

        add_dependency(
            &db,
            "skyrimse",
            "Gaming",
            mod_b,
            Some(mod_a),
            None,
            "Framework",
            "requires",
        )
        .unwrap();

        let dependents = get_dependents(&db, mod_a).unwrap();
        assert_eq!(dependents[0].mod_id, mod_b);
    }

    #[test]
    fn get_dependents_nonexistent_mod() {
        let db = test_db();
        let dependents = get_dependents(&db, 99999).unwrap();
        assert!(dependents.is_empty());
    }

    #[test]
    fn get_all_dependencies_filters_by_game() {
        let db = test_db();
        let mod_a = add_test_mod(&db, "Mod A");
        let mod_b = add_test_mod(&db, "Mod B");

        add_dependency(
            &db,
            "skyrimse",
            "Gaming",
            mod_a,
            Some(mod_b),
            None,
            "Mod B",
            "requires",
        )
        .unwrap();

        let deps = get_all_dependencies(&db, "skyrimse", "Gaming").unwrap();
        assert_eq!(deps.len(), 1);

        let empty = get_all_dependencies(&db, "fallout4", "Gaming").unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn get_all_dependencies_empty_game() {
        let db = test_db();
        let deps = get_all_dependencies(&db, "nonexistent", "bottle").unwrap();
        assert!(deps.is_empty());
    }

    #[test]
    fn get_all_dependencies_multiple() {
        let db = test_db();
        let mod_a = add_test_mod(&db, "A");
        let mod_b = add_test_mod(&db, "B");
        let mod_c = add_test_mod(&db, "C");

        add_dependency(
            &db,
            "skyrimse",
            "Gaming",
            mod_a,
            Some(mod_b),
            None,
            "B",
            "requires",
        )
        .unwrap();
        add_dependency(
            &db,
            "skyrimse",
            "Gaming",
            mod_a,
            Some(mod_c),
            None,
            "C",
            "requires",
        )
        .unwrap();
        add_dependency(
            &db,
            "skyrimse",
            "Gaming",
            mod_b,
            Some(mod_c),
            None,
            "C",
            "patches",
        )
        .unwrap();

        let deps = get_all_dependencies(&db, "skyrimse", "Gaming").unwrap();
        assert_eq!(deps.len(), 3);
    }

    #[test]
    fn get_all_dependencies_filters_by_bottle() {
        let db = test_db();
        let mod_a = add_test_mod(&db, "Mod A");
        let mod_b = add_test_mod(&db, "Mod B");

        add_dependency(
            &db,
            "skyrimse",
            "Gaming",
            mod_a,
            Some(mod_b),
            None,
            "Mod B",
            "requires",
        )
        .unwrap();

        let deps = get_all_dependencies(&db, "skyrimse", "OtherBottle").unwrap();
        assert!(deps.is_empty());
    }

    #[test]
    fn check_issues_missing_requirement() {
        let db = test_db();
        let mod_a = add_test_mod(&db, "Mod A");

        // Mod A requires a non-existent mod
        add_dependency(
            &db,
            "skyrimse",
            "Gaming",
            mod_a,
            None,
            Some(99999),
            "Missing Mod",
            "requires",
        )
        .unwrap();

        let issues = check_dependency_issues(&db, "skyrimse", "Gaming").unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(
            issues[0].issue_type,
            DependencyIssueType::MissingRequirement
        );
    }

    #[test]
    fn check_issues_missing_requirement_message() {
        let db = test_db();
        let mod_a = add_test_mod(&db, "Mod A");

        add_dependency(
            &db,
            "skyrimse",
            "Gaming",
            mod_a,
            None,
            Some(99999),
            "Missing Mod",
            "requires",
        )
        .unwrap();

        let issues = check_dependency_issues(&db, "skyrimse", "Gaming").unwrap();
        assert!(issues[0].message.contains("Missing Mod"));
    }

    #[test]
    fn check_issues_no_issues_when_satisfied() {
        let db = test_db();
        let mod_a = add_test_mod(&db, "Mod A");
        let mod_b = add_test_mod(&db, "Mod B");

        add_dependency(
            &db,
            "skyrimse",
            "Gaming",
            mod_a,
            Some(mod_b),
            None,
            "Mod B",
            "requires",
        )
        .unwrap();

        let issues = check_dependency_issues(&db, "skyrimse", "Gaming").unwrap();
        assert!(issues.is_empty());
    }

    #[test]
    fn check_issues_empty() {
        let db = test_db();
        let issues = check_dependency_issues(&db, "skyrimse", "Gaming").unwrap();
        assert!(issues.is_empty());
    }

    #[test]
    fn clear_mod_dependencies_works() {
        let db = test_db();
        let mod_a = add_test_mod(&db, "Mod A");
        let mod_b = add_test_mod(&db, "Mod B");
        let mod_c = add_test_mod(&db, "Mod C");

        add_dependency(
            &db,
            "skyrimse",
            "Gaming",
            mod_a,
            Some(mod_b),
            None,
            "B",
            "requires",
        )
        .unwrap();
        add_dependency(
            &db,
            "skyrimse",
            "Gaming",
            mod_a,
            Some(mod_c),
            None,
            "C",
            "requires",
        )
        .unwrap();

        let cleared = clear_mod_dependencies(&db, mod_a).unwrap();
        assert_eq!(cleared, 2);

        let deps = get_dependencies(&db, mod_a).unwrap();
        assert!(deps.is_empty());
    }

    #[test]
    fn clear_mod_dependencies_empty() {
        let db = test_db();
        let mod_a = add_test_mod(&db, "Mod A");
        let cleared = clear_mod_dependencies(&db, mod_a).unwrap();
        assert_eq!(cleared, 0);
    }

    #[test]
    fn clear_mod_dependencies_preserves_others() {
        let db = test_db();
        let mod_a = add_test_mod(&db, "A");
        let mod_b = add_test_mod(&db, "B");
        let mod_c = add_test_mod(&db, "C");

        add_dependency(
            &db,
            "skyrimse",
            "Gaming",
            mod_a,
            Some(mod_c),
            None,
            "C",
            "requires",
        )
        .unwrap();
        add_dependency(
            &db,
            "skyrimse",
            "Gaming",
            mod_b,
            Some(mod_c),
            None,
            "C",
            "requires",
        )
        .unwrap();

        clear_mod_dependencies(&db, mod_a).unwrap();

        let deps_b = get_dependencies(&db, mod_b).unwrap();
        assert_eq!(deps_b.len(), 1);
    }

    #[test]
    fn clear_nonexistent_mod() {
        let db = test_db();
        let cleared = clear_mod_dependencies(&db, 99999).unwrap();
        assert_eq!(cleared, 0);
    }

    #[test]
    fn add_dependency_with_version_range() {
        let db = test_db();
        let mod_a = add_test_mod(&db, "Mod A");
        let mod_b = add_test_mod(&db, "SkyUI");

        // Store a version range as part of the dependency name
        let dep_id = add_dependency(
            &db,
            "skyrimse",
            "Gaming",
            mod_a,
            Some(mod_b),
            Some(51245),
            "SkyUI >= 5.2",
            "requires",
        )
        .unwrap();
        assert!(dep_id > 0);

        let deps = get_dependencies(&db, mod_a).unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].dep_name, "SkyUI >= 5.2");
        assert_eq!(deps[0].depends_on_id, Some(mod_b));
        assert_eq!(deps[0].nexus_dep_id, Some(51245));
        assert_eq!(deps[0].relationship, "requires");
        assert_eq!(deps[0].game_id, "skyrimse");
        assert_eq!(deps[0].bottle_name, "Gaming");
    }
}
