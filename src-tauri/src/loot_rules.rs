//! Custom LOOT load order rules.
//!
//! Manages user-defined rules that override LOOT's automatic sorting. Rules
//! allow specifying that a plugin should load before or after another, or
//! assigning plugins to named groups. Rules persist in the SQLite database
//! and are applied on top of a LOOT-sorted order via topological sort.

use std::collections::{HashMap, HashSet, VecDeque};

use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::database::ModDatabase;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A single load order rule for a plugin.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginRule {
    pub id: i64,
    pub game_id: String,
    pub bottle_name: String,
    pub plugin_name: String,
    pub rule_type: PluginRuleType,
    pub reference_plugin: String,
    pub created_at: String,
}

/// The kind of load order constraint.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum PluginRuleType {
    /// `plugin_name` must load after `reference_plugin`.
    LoadAfter,
    /// `plugin_name` must load before `reference_plugin`.
    LoadBefore,
    /// `plugin_name` belongs to the named group (`reference_plugin` = group name).
    Group,
}

impl PluginRuleType {
    fn as_str(&self) -> &'static str {
        match self {
            PluginRuleType::LoadAfter => "LoadAfter",
            PluginRuleType::LoadBefore => "LoadBefore",
            PluginRuleType::Group => "Group",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "LoadAfter" => Some(PluginRuleType::LoadAfter),
            "LoadBefore" => Some(PluginRuleType::LoadBefore),
            "Group" => Some(PluginRuleType::Group),
            _ => None,
        }
    }
}

/// A named group of plugins with a sort index that determines inter-group
/// ordering.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginGroup {
    pub name: String,
    pub plugins: Vec<String>,
    pub sort_index: i32,
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

/// Create the `plugin_rules` table if it does not exist.
pub fn init_schema(db: &ModDatabase) -> Result<(), rusqlite::Error> {
    let conn = db.conn().map_err(|_| {
        rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
            Some("Failed to lock database".to_string()),
        )
    })?;
    init_schema_with_conn(&conn)
}

fn init_schema_with_conn(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS plugin_rules (
            id               INTEGER PRIMARY KEY AUTOINCREMENT,
            game_id          TEXT NOT NULL,
            bottle_name      TEXT NOT NULL,
            plugin_name      TEXT NOT NULL,
            rule_type        TEXT NOT NULL,
            reference_plugin TEXT NOT NULL,
            created_at       TEXT NOT NULL,
            UNIQUE(game_id, bottle_name, plugin_name, rule_type, reference_plugin)
        );

        CREATE INDEX IF NOT EXISTS idx_plugin_rules_game_bottle
            ON plugin_rules (game_id, bottle_name);",
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// CRUD operations
// ---------------------------------------------------------------------------

/// Add a new load order rule. Returns the new rule's row ID.
pub fn add_rule(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    plugin_name: &str,
    rule_type: PluginRuleType,
    reference_plugin: &str,
) -> Result<i64, String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    let created_at = Utc::now().to_rfc3339();

    conn.execute(
        "INSERT INTO plugin_rules
            (game_id, bottle_name, plugin_name, rule_type, reference_plugin, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            game_id,
            bottle_name,
            plugin_name,
            rule_type.as_str(),
            reference_plugin,
            created_at,
        ],
    )
    .map_err(|e| format!("Failed to add rule: {}", e))?;

    Ok(conn.last_insert_rowid())
}

/// Remove a rule by its row ID.
pub fn remove_rule(db: &ModDatabase, rule_id: i64) -> Result<(), String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    let rows = conn
        .execute("DELETE FROM plugin_rules WHERE id = ?1", params![rule_id])
        .map_err(|e| format!("Failed to remove rule: {}", e))?;

    if rows == 0 {
        return Err(format!("Rule with ID {} not found", rule_id));
    }
    Ok(())
}

/// List all rules for a given game/bottle combination.
pub fn list_rules(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
) -> Result<Vec<PluginRule>, String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, game_id, bottle_name, plugin_name, rule_type,
                    reference_plugin, created_at
             FROM plugin_rules
             WHERE game_id = ?1 AND bottle_name = ?2
             ORDER BY id ASC",
        )
        .map_err(|e| format!("Failed to prepare statement: {}", e))?;

    let rows = stmt
        .query_map(params![game_id, bottle_name], |row| {
            let rule_type_str: String = row.get(4)?;
            let rule_type =
                PluginRuleType::from_str(&rule_type_str).unwrap_or(PluginRuleType::LoadAfter);
            Ok(PluginRule {
                id: row.get(0)?,
                game_id: row.get(1)?,
                bottle_name: row.get(2)?,
                plugin_name: row.get(3)?,
                rule_type,
                reference_plugin: row.get(5)?,
                created_at: row.get(6)?,
            })
        })
        .map_err(|e| format!("Failed to query rules: {}", e))?;

    let mut rules = Vec::new();
    for row in rows {
        rules.push(row.map_err(|e| format!("Failed to read row: {}", e))?);
    }
    Ok(rules)
}

/// Clear all rules for a given game/bottle combination.
pub fn clear_rules(db: &ModDatabase, game_id: &str, bottle_name: &str) -> Result<(), String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM plugin_rules WHERE game_id = ?1 AND bottle_name = ?2",
        params![game_id, bottle_name],
    )
    .map_err(|e| format!("Failed to clear rules: {}", e))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validate a set of rules for cycles.
///
/// A cycle exists when, e.g., rule A says "load after B" and rule B says
/// "load after A". Group rules are ignored for cycle detection.
///
/// Returns `Ok(())` if no cycles are found, or `Err` with a description of
/// the cycle.
pub fn validate_rules(rules: &[PluginRule]) -> Result<(), String> {
    // Build a directed graph: edge from X -> Y means "X must load before Y".
    let mut graph: HashMap<String, Vec<String>> = HashMap::new();
    let mut nodes: HashSet<String> = HashSet::new();

    for rule in rules {
        if rule.rule_type == PluginRuleType::Group {
            continue;
        }

        let plugin = rule.plugin_name.to_lowercase();
        let reference = rule.reference_plugin.to_lowercase();
        nodes.insert(plugin.clone());
        nodes.insert(reference.clone());

        match rule.rule_type {
            PluginRuleType::LoadAfter => {
                // plugin loads after reference => reference -> plugin
                graph.entry(reference).or_default().push(plugin);
            }
            PluginRuleType::LoadBefore => {
                // plugin loads before reference => plugin -> reference
                graph.entry(plugin).or_default().push(reference);
            }
            PluginRuleType::Group => unreachable!(),
        }
    }

    // Kahn's algorithm for cycle detection via topological sort.
    let mut in_degree: HashMap<String, usize> = HashMap::new();
    for node in &nodes {
        in_degree.entry(node.clone()).or_insert(0);
    }
    for successors in graph.values() {
        for s in successors {
            *in_degree.entry(s.clone()).or_insert(0) += 1;
        }
    }

    let mut queue: VecDeque<String> = VecDeque::new();
    for (node, &deg) in &in_degree {
        if deg == 0 {
            queue.push_back(node.clone());
        }
    }

    let mut visited_count = 0usize;
    while let Some(node) = queue.pop_front() {
        visited_count += 1;
        if let Some(successors) = graph.get(&node) {
            for s in successors {
                if let Some(deg) = in_degree.get_mut(s) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push_back(s.clone());
                    }
                }
            }
        }
    }

    if visited_count != nodes.len() {
        // Find the nodes that are part of the cycle (those with in_degree > 0).
        let cycle_nodes: Vec<String> = in_degree
            .iter()
            .filter(|(_, &deg)| deg > 0)
            .map(|(n, _)| n.clone())
            .collect();
        Err(format!(
            "Cycle detected among plugins: {}",
            cycle_nodes.join(", ")
        ))
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Apply rules
// ---------------------------------------------------------------------------

/// Apply custom rules on top of a LOOT-sorted plugin order.
///
/// Starts with the given `sorted_order`, then adjusts positions to satisfy
/// LoadAfter/LoadBefore constraints. If a cycle is detected, logs a warning
/// and returns the original order unchanged.
pub fn apply_rules(sorted_order: &[String], rules: &[PluginRule]) -> Vec<String> {
    // Filter to ordering rules only (LoadAfter / LoadBefore).
    let ordering_rules: Vec<&PluginRule> = rules
        .iter()
        .filter(|r| r.rule_type != PluginRuleType::Group)
        .collect();

    if ordering_rules.is_empty() {
        return sorted_order.to_vec();
    }

    // Validate for cycles first.
    if let Err(msg) = validate_rules(rules) {
        log::warn!(
            "Cannot apply custom rules: {}. Returning original order.",
            msg
        );
        return sorted_order.to_vec();
    }

    // Build a position index for the current order (case-insensitive).
    let plugins: Vec<String> = sorted_order.to_vec();
    let plugin_set: HashSet<String> = plugins.iter().map(|p| p.to_lowercase()).collect();

    // Build the constraint graph from rules that reference plugins actually
    // present in the load order.
    // Edge: predecessor -> successor (predecessor must load before successor).
    let mut graph: HashMap<String, Vec<String>> = HashMap::new();
    let mut in_degree: HashMap<String, usize> = HashMap::new();

    // Initialize all plugins as nodes.
    for p in &plugins {
        let key = p.to_lowercase();
        graph.entry(key.clone()).or_default();
        in_degree.entry(key).or_insert(0);
    }

    for rule in &ordering_rules {
        let plugin = rule.plugin_name.to_lowercase();
        let reference = rule.reference_plugin.to_lowercase();

        // Skip rules for plugins not in the current order.
        if !plugin_set.contains(&plugin) || !plugin_set.contains(&reference) {
            continue;
        }

        match rule.rule_type {
            PluginRuleType::LoadAfter => {
                // plugin loads after reference => reference -> plugin
                graph
                    .entry(reference.clone())
                    .or_default()
                    .push(plugin.clone());
                *in_degree.entry(plugin).or_insert(0) += 1;
            }
            PluginRuleType::LoadBefore => {
                // plugin loads before reference => plugin -> reference
                graph
                    .entry(plugin.clone())
                    .or_default()
                    .push(reference.clone());
                *in_degree.entry(reference).or_insert(0) += 1;
            }
            PluginRuleType::Group => {}
        }
    }

    // Topological sort using Kahn's algorithm, but use the original LOOT
    // order as a tie-breaker (stable sort). This preserves LOOT ordering
    // among plugins with no rule constraints between them.
    let position_map: HashMap<String, usize> = plugins
        .iter()
        .enumerate()
        .map(|(i, p)| (p.to_lowercase(), i))
        .collect();

    // Use a BinaryHeap-based priority queue ordered by original position.
    // Since BinaryHeap is a max-heap we use Reverse for min ordering.
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;

    let mut heap: BinaryHeap<Reverse<(usize, String)>> = BinaryHeap::new();
    for (node, &deg) in &in_degree {
        if deg == 0 {
            let pos = position_map.get(node).copied().unwrap_or(usize::MAX);
            heap.push(Reverse((pos, node.clone())));
        }
    }

    let mut topo_order: Vec<String> = Vec::with_capacity(plugins.len());
    while let Some(Reverse((_, node))) = heap.pop() {
        topo_order.push(node.clone());
        if let Some(successors) = graph.get(&node) {
            for s in successors {
                if let Some(deg) = in_degree.get_mut(s) {
                    *deg -= 1;
                    if *deg == 0 {
                        let pos = position_map.get(s).copied().unwrap_or(usize::MAX);
                        heap.push(Reverse((pos, s.clone())));
                    }
                }
            }
        }
    }

    // Map back to original-case names.
    let lower_to_original: HashMap<String, &String> =
        plugins.iter().map(|p| (p.to_lowercase(), p)).collect();

    topo_order
        .iter()
        .filter_map(|lc| lower_to_original.get(lc).map(|p| (*p).clone()))
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_db() -> (ModDatabase, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test_rules.db");
        let db = ModDatabase::new(&db_path).unwrap();
        init_schema(&db).unwrap();
        (db, tmp)
    }

    #[test]
    fn test_schema_initialization() {
        let (db, _tmp) = test_db();
        // Schema should be initialized; verify by listing rules (empty).
        let rules = list_rules(&db, "skyrimse", "Gaming").unwrap();
        assert!(rules.is_empty());

        // Calling init_schema again should be idempotent.
        init_schema(&db).unwrap();
    }

    #[test]
    fn test_add_list_remove_roundtrip() {
        let (db, _tmp) = test_db();

        let id1 = add_rule(
            &db,
            "skyrimse",
            "Gaming",
            "MyMod.esp",
            PluginRuleType::LoadAfter,
            "SkyUI_SE.esp",
        )
        .unwrap();

        let id2 = add_rule(
            &db,
            "skyrimse",
            "Gaming",
            "Patch.esp",
            PluginRuleType::LoadBefore,
            "Skyrim.esm",
        )
        .unwrap();

        assert!(id1 > 0);
        assert!(id2 > 0);
        assert_ne!(id1, id2);

        let rules = list_rules(&db, "skyrimse", "Gaming").unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].plugin_name, "MyMod.esp");
        assert_eq!(rules[0].rule_type, PluginRuleType::LoadAfter);
        assert_eq!(rules[0].reference_plugin, "SkyUI_SE.esp");
        assert_eq!(rules[1].plugin_name, "Patch.esp");
        assert_eq!(rules[1].rule_type, PluginRuleType::LoadBefore);

        // Remove the first rule.
        remove_rule(&db, id1).unwrap();
        let rules = list_rules(&db, "skyrimse", "Gaming").unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].plugin_name, "Patch.esp");

        // Removing a non-existent rule should error.
        assert!(remove_rule(&db, 99999).is_err());
    }

    #[test]
    fn test_apply_rules_load_after() {
        // Initial LOOT order: A, B, C, D
        // Rule: C loads after A (already satisfied), B loads after D
        let order = vec![
            "A.esp".to_string(),
            "B.esp".to_string(),
            "C.esp".to_string(),
            "D.esp".to_string(),
        ];

        let rules = vec![PluginRule {
            id: 1,
            game_id: "skyrimse".to_string(),
            bottle_name: "Gaming".to_string(),
            plugin_name: "B.esp".to_string(),
            rule_type: PluginRuleType::LoadAfter,
            reference_plugin: "D.esp".to_string(),
            created_at: "2024-01-01".to_string(),
        }];

        let result = apply_rules(&order, &rules);
        // B should come after D.
        let pos_b = result.iter().position(|p| p == "B.esp").unwrap();
        let pos_d = result.iter().position(|p| p == "D.esp").unwrap();
        assert!(
            pos_b > pos_d,
            "B.esp (pos {}) should be after D.esp (pos {}): {:?}",
            pos_b,
            pos_d,
            result
        );
        // All original plugins should still be present.
        assert_eq!(result.len(), 4);
    }

    #[test]
    fn test_apply_rules_load_before() {
        // Initial LOOT order: A, B, C, D
        // Rule: D loads before B
        let order = vec![
            "A.esp".to_string(),
            "B.esp".to_string(),
            "C.esp".to_string(),
            "D.esp".to_string(),
        ];

        let rules = vec![PluginRule {
            id: 1,
            game_id: "skyrimse".to_string(),
            bottle_name: "Gaming".to_string(),
            plugin_name: "D.esp".to_string(),
            rule_type: PluginRuleType::LoadBefore,
            reference_plugin: "B.esp".to_string(),
            created_at: "2024-01-01".to_string(),
        }];

        let result = apply_rules(&order, &rules);
        let pos_d = result.iter().position(|p| p == "D.esp").unwrap();
        let pos_b = result.iter().position(|p| p == "B.esp").unwrap();
        assert!(
            pos_d < pos_b,
            "D.esp (pos {}) should be before B.esp (pos {}): {:?}",
            pos_d,
            pos_b,
            result
        );
        assert_eq!(result.len(), 4);
    }

    #[test]
    fn test_cycle_detection() {
        let rules = vec![
            PluginRule {
                id: 1,
                game_id: "skyrimse".to_string(),
                bottle_name: "Gaming".to_string(),
                plugin_name: "A.esp".to_string(),
                rule_type: PluginRuleType::LoadAfter,
                reference_plugin: "B.esp".to_string(),
                created_at: "2024-01-01".to_string(),
            },
            PluginRule {
                id: 2,
                game_id: "skyrimse".to_string(),
                bottle_name: "Gaming".to_string(),
                plugin_name: "B.esp".to_string(),
                rule_type: PluginRuleType::LoadAfter,
                reference_plugin: "A.esp".to_string(),
                created_at: "2024-01-01".to_string(),
            },
        ];

        let result = validate_rules(&rules);
        assert!(result.is_err());
        let err_msg = result.unwrap_err();
        assert!(err_msg.contains("Cycle detected"));
    }

    #[test]
    fn test_cycle_returns_original_order() {
        // When a cycle is detected, apply_rules should return the original
        // order unchanged.
        let order = vec![
            "A.esp".to_string(),
            "B.esp".to_string(),
            "C.esp".to_string(),
        ];

        let rules = vec![
            PluginRule {
                id: 1,
                game_id: "skyrimse".to_string(),
                bottle_name: "Gaming".to_string(),
                plugin_name: "A.esp".to_string(),
                rule_type: PluginRuleType::LoadAfter,
                reference_plugin: "B.esp".to_string(),
                created_at: "2024-01-01".to_string(),
            },
            PluginRule {
                id: 2,
                game_id: "skyrimse".to_string(),
                bottle_name: "Gaming".to_string(),
                plugin_name: "B.esp".to_string(),
                rule_type: PluginRuleType::LoadAfter,
                reference_plugin: "A.esp".to_string(),
                created_at: "2024-01-01".to_string(),
            },
        ];

        let result = apply_rules(&order, &rules);
        assert_eq!(result, order, "Cyclic rules should return original order");
    }

    #[test]
    fn test_clear_rules() {
        let (db, _tmp) = test_db();

        add_rule(
            &db,
            "skyrimse",
            "Gaming",
            "A.esp",
            PluginRuleType::LoadAfter,
            "B.esp",
        )
        .unwrap();
        add_rule(
            &db,
            "skyrimse",
            "Gaming",
            "C.esp",
            PluginRuleType::LoadBefore,
            "D.esp",
        )
        .unwrap();
        add_rule(
            &db,
            "fallout4",
            "Gaming",
            "X.esp",
            PluginRuleType::LoadAfter,
            "Y.esp",
        )
        .unwrap();

        clear_rules(&db, "skyrimse", "Gaming").unwrap();

        let skyrim_rules = list_rules(&db, "skyrimse", "Gaming").unwrap();
        assert!(skyrim_rules.is_empty());

        // Fallout rules should be untouched.
        let fallout_rules = list_rules(&db, "fallout4", "Gaming").unwrap();
        assert_eq!(fallout_rules.len(), 1);
    }

    #[test]
    fn test_no_cycle_validates_ok() {
        let rules = vec![
            PluginRule {
                id: 1,
                game_id: "skyrimse".to_string(),
                bottle_name: "Gaming".to_string(),
                plugin_name: "B.esp".to_string(),
                rule_type: PluginRuleType::LoadAfter,
                reference_plugin: "A.esp".to_string(),
                created_at: "2024-01-01".to_string(),
            },
            PluginRule {
                id: 2,
                game_id: "skyrimse".to_string(),
                bottle_name: "Gaming".to_string(),
                plugin_name: "C.esp".to_string(),
                rule_type: PluginRuleType::LoadAfter,
                reference_plugin: "B.esp".to_string(),
                created_at: "2024-01-01".to_string(),
            },
        ];

        assert!(validate_rules(&rules).is_ok());
    }

    #[test]
    fn test_apply_rules_preserves_order_without_constraints() {
        // With no rules, the original order should be returned verbatim.
        let order = vec![
            "A.esp".to_string(),
            "B.esp".to_string(),
            "C.esp".to_string(),
        ];

        let result = apply_rules(&order, &[]);
        assert_eq!(result, order);
    }

    #[test]
    fn test_group_rules_ignored_in_validation() {
        // Group rules should not cause cycle detection issues.
        let rules = vec![PluginRule {
            id: 1,
            game_id: "skyrimse".to_string(),
            bottle_name: "Gaming".to_string(),
            plugin_name: "A.esp".to_string(),
            rule_type: PluginRuleType::Group,
            reference_plugin: "MyGroup".to_string(),
            created_at: "2024-01-01".to_string(),
        }];

        assert!(validate_rules(&rules).is_ok());
    }
}
