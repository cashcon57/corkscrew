//! FOMOD choice replay (recipes).
//!
//! Saves and restores FOMOD installer selections so they can be replayed
//! during mod updates or shared with other users.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::database::ModDatabase;

/// A saved FOMOD recipe.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FomodRecipe {
    pub id: i64,
    pub mod_id: i64,
    pub mod_name: String,
    pub installer_hash: Option<String>,
    pub selections: HashMap<String, Vec<String>>,
    pub created_at: String,
}

/// Save a FOMOD recipe for a mod.
pub fn save_recipe(
    db: &ModDatabase,
    mod_id: i64,
    mod_name: &str,
    installer_hash: Option<&str>,
    selections: &HashMap<String, Vec<String>>,
) -> Result<i64, String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().to_rfc3339();
    let json = serde_json::to_string(selections).map_err(|e| e.to_string())?;

    conn.execute(
        "INSERT OR REPLACE INTO fomod_recipes
            (mod_id, mod_name, installer_hash, selections_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![mod_id, mod_name, installer_hash, json, now],
    )
    .map_err(|e| e.to_string())?;

    Ok(conn.last_insert_rowid())
}

/// Get a FOMOD recipe for a specific mod.
pub fn get_recipe(db: &ModDatabase, mod_id: i64) -> Result<Option<FomodRecipe>, String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, mod_id, mod_name, installer_hash, selections_json, created_at
             FROM fomod_recipes WHERE mod_id = ?1",
        )
        .map_err(|e| e.to_string())?;

    let result = stmt
        .query_row([mod_id], |row| {
            let json: String = row.get(4)?;
            let selections: HashMap<String, Vec<String>> =
                serde_json::from_str(&json).unwrap_or_default();

            Ok(FomodRecipe {
                id: row.get(0)?,
                mod_id: row.get(1)?,
                mod_name: row.get(2)?,
                installer_hash: row.get(3)?,
                selections,
                created_at: row.get(5)?,
            })
        })
        .ok();

    Ok(result)
}

/// List all saved recipes for a game/bottle.
pub fn list_recipes(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
) -> Result<Vec<FomodRecipe>, String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT fr.id, fr.mod_id, fr.mod_name, fr.installer_hash, fr.selections_json, fr.created_at
             FROM fomod_recipes fr
             JOIN installed_mods im ON im.id = fr.mod_id
             WHERE im.game_id = ?1 AND im.bottle_name = ?2
             ORDER BY fr.created_at DESC",
        )
        .map_err(|e| e.to_string())?;

    let recipes = stmt
        .query_map(rusqlite::params![game_id, bottle_name], |row| {
            let json: String = row.get(4)?;
            let selections: HashMap<String, Vec<String>> =
                serde_json::from_str(&json).unwrap_or_default();

            Ok(FomodRecipe {
                id: row.get(0)?,
                mod_id: row.get(1)?,
                mod_name: row.get(2)?,
                installer_hash: row.get(3)?,
                selections,
                created_at: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    Ok(recipes)
}

/// Delete a recipe.
pub fn delete_recipe(db: &ModDatabase, mod_id: i64) -> Result<(), String> {
    let conn = db.conn().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM fomod_recipes WHERE mod_id = ?1", [mod_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Check if a recipe exists and its installer hash matches.
pub fn has_compatible_recipe(
    db: &ModDatabase,
    mod_id: i64,
    current_hash: Option<&str>,
) -> Result<bool, String> {
    let recipe = get_recipe(db, mod_id)?;
    match recipe {
        None => Ok(false),
        Some(r) => {
            // If no hash stored or no hash to compare, it's compatible (best effort)
            match (r.installer_hash.as_deref(), current_hash) {
                (Some(stored), Some(current)) => Ok(stored == current),
                _ => Ok(true),
            }
        }
    }
}

/// Export a recipe to a portable JSON string.
pub fn export_recipe(recipe: &FomodRecipe) -> Result<String, String> {
    serde_json::to_string_pretty(recipe).map_err(|e| e.to_string())
}

/// Import a recipe from a JSON string.
pub fn import_recipe(json: &str) -> Result<FomodRecipe, String> {
    serde_json::from_str(json).map_err(|e| e.to_string())
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

    fn sample_selections() -> HashMap<String, Vec<String>> {
        let mut s = HashMap::new();
        s.insert("Body Type".into(), vec!["CBBE".into()]);
        s.insert("Textures".into(), vec!["4K".into(), "Normal Maps".into()]);
        s
    }

    #[test]
    fn save_and_get_recipe() {
        let db = test_db();
        let mod_id = add_test_mod(&db, "Test Mod");
        let selections = sample_selections();

        save_recipe(&db, mod_id, "Test Mod", Some("abc123"), &selections).unwrap();

        let recipe = get_recipe(&db, mod_id).unwrap().unwrap();
        assert_eq!(recipe.mod_id, mod_id);
        assert_eq!(recipe.mod_name, "Test Mod");
        assert_eq!(recipe.installer_hash, Some("abc123".to_string()));
        assert_eq!(recipe.selections, selections);
    }

    #[test]
    fn save_recipe_returns_id() {
        let db = test_db();
        let mod_id = add_test_mod(&db, "Mod");
        let id = save_recipe(&db, mod_id, "Mod", None, &HashMap::new()).unwrap();
        assert!(id > 0);
    }

    #[test]
    fn save_recipe_overwrites_existing() {
        let db = test_db();
        let mod_id = add_test_mod(&db, "Test Mod");
        let sel1 = sample_selections();
        let mut sel2 = HashMap::new();
        sel2.insert("Option".into(), vec!["B".into()]);

        save_recipe(&db, mod_id, "Test Mod", None, &sel1).unwrap();
        save_recipe(&db, mod_id, "Test Mod", None, &sel2).unwrap();

        let recipe = get_recipe(&db, mod_id).unwrap().unwrap();
        assert_eq!(recipe.selections, sel2);
    }

    #[test]
    fn save_recipe_no_hash() {
        let db = test_db();
        let mod_id = add_test_mod(&db, "Mod");
        save_recipe(&db, mod_id, "Mod", None, &HashMap::new()).unwrap();

        let recipe = get_recipe(&db, mod_id).unwrap().unwrap();
        assert!(recipe.installer_hash.is_none());
    }

    #[test]
    fn get_recipe_nonexistent() {
        let db = test_db();
        let recipe = get_recipe(&db, 99999).unwrap();
        assert!(recipe.is_none());
    }

    #[test]
    fn get_recipe_has_timestamp() {
        let db = test_db();
        let mod_id = add_test_mod(&db, "Mod");
        save_recipe(&db, mod_id, "Mod", None, &HashMap::new()).unwrap();

        let recipe = get_recipe(&db, mod_id).unwrap().unwrap();
        assert!(!recipe.created_at.is_empty());
    }

    #[test]
    fn get_recipe_preserves_selections() {
        let db = test_db();
        let mod_id = add_test_mod(&db, "Mod");
        let mut sels = HashMap::new();
        sels.insert("Step1".into(), vec!["A".into(), "B".into(), "C".into()]);

        save_recipe(&db, mod_id, "Mod", None, &sels).unwrap();
        let recipe = get_recipe(&db, mod_id).unwrap().unwrap();
        assert_eq!(recipe.selections.get("Step1").unwrap().len(), 3);
    }

    #[test]
    fn delete_recipe_works() {
        let db = test_db();
        let mod_id = add_test_mod(&db, "Test Mod");
        save_recipe(&db, mod_id, "Test Mod", None, &sample_selections()).unwrap();

        delete_recipe(&db, mod_id).unwrap();
        let recipe = get_recipe(&db, mod_id).unwrap();
        assert!(recipe.is_none());
    }

    #[test]
    fn delete_recipe_nonexistent() {
        let db = test_db();
        // Should not error
        delete_recipe(&db, 99999).unwrap();
    }

    #[test]
    fn delete_recipe_preserves_others() {
        let db = test_db();
        let mod_a = add_test_mod(&db, "A");
        let mod_b = add_test_mod(&db, "B");
        save_recipe(&db, mod_a, "A", None, &HashMap::new()).unwrap();
        save_recipe(&db, mod_b, "B", None, &HashMap::new()).unwrap();

        delete_recipe(&db, mod_a).unwrap();
        assert!(get_recipe(&db, mod_b).unwrap().is_some());
    }

    #[test]
    fn delete_recipe_double_delete() {
        let db = test_db();
        let mod_id = add_test_mod(&db, "Mod");
        save_recipe(&db, mod_id, "Mod", None, &HashMap::new()).unwrap();
        delete_recipe(&db, mod_id).unwrap();
        delete_recipe(&db, mod_id).unwrap(); // Should not error
    }

    #[test]
    fn has_compatible_recipe_no_recipe() {
        let db = test_db();
        assert!(!has_compatible_recipe(&db, 99999, None).unwrap());
    }

    #[test]
    fn has_compatible_recipe_matching_hash() {
        let db = test_db();
        let mod_id = add_test_mod(&db, "Mod");
        save_recipe(&db, mod_id, "Mod", Some("hash123"), &HashMap::new()).unwrap();

        assert!(has_compatible_recipe(&db, mod_id, Some("hash123")).unwrap());
    }

    #[test]
    fn has_compatible_recipe_different_hash() {
        let db = test_db();
        let mod_id = add_test_mod(&db, "Mod");
        save_recipe(&db, mod_id, "Mod", Some("hash123"), &HashMap::new()).unwrap();

        assert!(!has_compatible_recipe(&db, mod_id, Some("different")).unwrap());
    }

    #[test]
    fn has_compatible_recipe_no_hash_stored() {
        let db = test_db();
        let mod_id = add_test_mod(&db, "Mod");
        save_recipe(&db, mod_id, "Mod", None, &HashMap::new()).unwrap();

        assert!(has_compatible_recipe(&db, mod_id, Some("any")).unwrap());
    }

    #[test]
    fn has_compatible_recipe_no_hash_provided() {
        let db = test_db();
        let mod_id = add_test_mod(&db, "Mod");
        save_recipe(&db, mod_id, "Mod", Some("hash"), &HashMap::new()).unwrap();

        assert!(has_compatible_recipe(&db, mod_id, None).unwrap());
    }

    #[test]
    fn export_and_import_recipe() {
        let recipe = FomodRecipe {
            id: 1,
            mod_id: 42,
            mod_name: "Test Mod".into(),
            installer_hash: Some("abc".into()),
            selections: sample_selections(),
            created_at: "2024-01-01T00:00:00Z".into(),
        };

        let json = export_recipe(&recipe).unwrap();
        let imported = import_recipe(&json).unwrap();

        assert_eq!(imported.mod_name, "Test Mod");
        assert_eq!(imported.selections, recipe.selections);
    }

    #[test]
    fn export_recipe_is_json() {
        let recipe = FomodRecipe {
            id: 1,
            mod_id: 1,
            mod_name: "Mod".into(),
            installer_hash: None,
            selections: HashMap::new(),
            created_at: "now".into(),
        };

        let json = export_recipe(&recipe).unwrap();
        assert!(json.starts_with('{'));
    }

    #[test]
    fn import_recipe_invalid_json() {
        let result = import_recipe("not json");
        assert!(result.is_err());
    }

    #[test]
    fn import_recipe_empty_json() {
        let result = import_recipe("{}");
        assert!(result.is_err()); // Missing required fields
    }

    #[test]
    fn list_recipes_empty() {
        let db = test_db();
        let recipes = list_recipes(&db, "skyrimse", "Gaming").unwrap();
        assert!(recipes.is_empty());
    }

    #[test]
    fn list_recipes_returns_saved() {
        let db = test_db();
        let mod_id = add_test_mod(&db, "Mod A");
        save_recipe(&db, mod_id, "Mod A", None, &sample_selections()).unwrap();

        let recipes = list_recipes(&db, "skyrimse", "Gaming").unwrap();
        assert_eq!(recipes.len(), 1);
        assert_eq!(recipes[0].mod_name, "Mod A");
    }

    #[test]
    fn list_recipes_filters_by_game() {
        let db = test_db();
        let mod_id = add_test_mod(&db, "Mod");
        save_recipe(&db, mod_id, "Mod", None, &HashMap::new()).unwrap();

        let recipes = list_recipes(&db, "fallout4", "Gaming").unwrap();
        assert!(recipes.is_empty());
    }

    #[test]
    fn list_recipes_multiple() {
        let db = test_db();
        let id1 = add_test_mod(&db, "A");
        let id2 = add_test_mod(&db, "B");
        save_recipe(&db, id1, "A", None, &HashMap::new()).unwrap();
        save_recipe(&db, id2, "B", None, &HashMap::new()).unwrap();

        let recipes = list_recipes(&db, "skyrimse", "Gaming").unwrap();
        assert_eq!(recipes.len(), 2);
    }

    #[test]
    fn export_recipe_contains_selections() {
        let db = test_db();
        let mod_id = add_test_mod(&db, "Selections Mod");
        let selections = sample_selections();
        save_recipe(&db, mod_id, "Selections Mod", Some("hash_sel"), &selections).unwrap();

        let recipe = get_recipe(&db, mod_id).unwrap().unwrap();
        let json = export_recipe(&recipe).unwrap();

        // Verify the exported JSON contains the selections keys and values
        assert!(json.contains("Body Type"));
        assert!(json.contains("CBBE"));
        assert!(json.contains("Textures"));
        assert!(json.contains("4K"));
        assert!(json.contains("Normal Maps"));
    }

    #[test]
    fn export_recipe_roundtrip_preserves_data() {
        let recipe = FomodRecipe {
            id: 10,
            mod_id: 42,
            mod_name: "Immersive Armors".into(),
            installer_hash: Some("deadbeef".into()),
            selections: sample_selections(),
            created_at: "2025-06-15T12:00:00Z".into(),
        };

        let json = export_recipe(&recipe).unwrap();
        let imported = import_recipe(&json).unwrap();

        assert_eq!(imported.mod_name, recipe.mod_name);
        assert_eq!(imported.mod_id, recipe.mod_id);
        assert_eq!(imported.id, recipe.id);
        assert_eq!(imported.installer_hash, recipe.installer_hash);
        assert_eq!(imported.selections, recipe.selections);
        assert_eq!(imported.created_at, recipe.created_at);
    }

    #[test]
    fn import_recipe_missing_fields() {
        // JSON object with some fields but missing required ones (mod_id, mod_name, etc.)
        let incomplete_json = r#"{"id": 1, "mod_id": 5}"#;
        let result = import_recipe(incomplete_json);
        assert!(result.is_err());
    }

    #[test]
    fn import_recipe_valid_saves_to_db() {
        let db = test_db();
        let mod_id = add_test_mod(&db, "Imported Mod");
        let selections = sample_selections();

        // Create a recipe, export it, import it, then save to DB
        let original = FomodRecipe {
            id: 1,
            mod_id,
            mod_name: "Imported Mod".into(),
            installer_hash: Some("importhash".into()),
            selections: selections.clone(),
            created_at: "2025-06-15T12:00:00Z".into(),
        };

        let json = export_recipe(&original).unwrap();
        let imported = import_recipe(&json).unwrap();

        // Save the imported recipe to the database
        save_recipe(
            &db,
            imported.mod_id,
            &imported.mod_name,
            imported.installer_hash.as_deref(),
            &imported.selections,
        )
        .unwrap();

        // Retrieve from DB and verify
        let retrieved = get_recipe(&db, mod_id).unwrap().unwrap();
        assert_eq!(retrieved.mod_name, "Imported Mod");
        assert_eq!(retrieved.installer_hash, Some("importhash".to_string()));
        assert_eq!(retrieved.selections, selections);
    }
}
