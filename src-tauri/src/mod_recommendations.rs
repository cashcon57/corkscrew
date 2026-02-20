//! Contextual mod recommendations based on collection co-occurrence.
//!
//! Analyzes which mods appear together across installed collections to
//! surface compatibility patches and commonly-used companion mods.

use std::collections::HashMap;

use serde::Serialize;

use crate::database::ModDatabase;

/// A recommended mod based on collection co-occurrence.
#[derive(Clone, Debug, Serialize)]
pub struct ModRecommendation {
    pub nexus_mod_id: i64,
    pub name: String,
    pub reason: String,
    pub co_occurrence_count: usize,
    pub is_installed: bool,
}

/// Summary of recommendations for a specific mod.
#[derive(Clone, Debug, Serialize)]
pub struct RecommendationResult {
    pub mod_id: i64,
    pub mod_name: String,
    pub recommendations: Vec<ModRecommendation>,
}

/// Analyze co-occurrence of mods within the same collections to generate recommendations.
pub fn get_recommendations(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    target_mod_id: i64,
) -> Result<RecommendationResult, String> {
    let mods = db
        .list_mods(game_id, bottle_name)
        .map_err(|e| e.to_string())?;

    let target = mods
        .iter()
        .find(|m| m.id == target_mod_id)
        .ok_or_else(|| format!("Mod {} not found", target_mod_id))?;

    let target_collection = target.collection_name.clone();
    let target_nexus_id = target.nexus_mod_id;

    let mut recommendations = Vec::new();

    // Strategy 1: Mods from the same collection
    if let Some(ref coll) = target_collection {
        let collection_mods: Vec<_> = mods
            .iter()
            .filter(|m| m.collection_name.as_ref() == Some(coll) && m.id != target_mod_id)
            .collect();

        for cm in &collection_mods {
            if let Some(nid) = cm.nexus_mod_id {
                // Don't recommend mods that are already installed outside this collection
                let already_installed = mods.iter().any(|m| {
                    m.nexus_mod_id == Some(nid)
                        && m.id != cm.id
                        && m.collection_name != target_collection
                });

                if !already_installed {
                    recommendations.push(ModRecommendation {
                        nexus_mod_id: nid,
                        name: cm.name.clone(),
                        reason: format!("Also in collection '{}'", coll),
                        co_occurrence_count: 1,
                        is_installed: true,
                    });
                }
            }
        }
    }

    // Strategy 2: Mods with the same tags (if available)
    if !target.user_tags.is_empty() {
        let target_tags: std::collections::HashSet<&str> =
            target.user_tags.iter().map(|s| s.as_str()).collect();

        for m in &mods {
            if m.id == target_mod_id {
                continue;
            }

            let m_tags: std::collections::HashSet<&str> =
                m.user_tags.iter().map(|s| s.as_str()).collect();
            let common: Vec<&&str> = target_tags.intersection(&m_tags).collect();

            if !common.is_empty() {
                let already = recommendations
                    .iter()
                    .any(|r| m.nexus_mod_id.map(|n| r.nexus_mod_id == n).unwrap_or(false));

                if !already {
                    if let Some(nid) = m.nexus_mod_id {
                        recommendations.push(ModRecommendation {
                            nexus_mod_id: nid,
                            name: m.name.clone(),
                            reason: format!(
                                "Shares tags: {}",
                                common.iter().map(|t| **t).collect::<Vec<_>>().join(", ")
                            ),
                            co_occurrence_count: common.len(),
                            is_installed: true,
                        });
                    }
                }
            }
        }
    }

    // Strategy 3: Cross-collection co-occurrence
    // Build a map of nexus_mod_id → collection names
    if let Some(target_nid) = target_nexus_id {
        let mut mod_collections: HashMap<i64, Vec<String>> = HashMap::new();
        for m in &mods {
            if let (Some(nid), Some(ref coll)) = (m.nexus_mod_id, &m.collection_name) {
                mod_collections.entry(nid).or_default().push(coll.clone());
            }
        }

        let target_colls: Vec<String> = mod_collections
            .get(&target_nid)
            .cloned()
            .unwrap_or_default();

        if !target_colls.is_empty() {
            let mut co_occurrences: HashMap<i64, usize> = HashMap::new();

            for m in &mods {
                if let (Some(nid), Some(ref coll)) = (m.nexus_mod_id, &m.collection_name) {
                    if nid != target_nid && target_colls.contains(coll) {
                        *co_occurrences.entry(nid).or_insert(0) += 1;
                    }
                }
            }

            for (nid, count) in &co_occurrences {
                let already = recommendations.iter().any(|r| r.nexus_mod_id == *nid);
                if !already && *count > 0 {
                    let name = mods
                        .iter()
                        .find(|m| m.nexus_mod_id == Some(*nid))
                        .map(|m| m.name.clone())
                        .unwrap_or_else(|| format!("Nexus #{}", nid));

                    recommendations.push(ModRecommendation {
                        nexus_mod_id: *nid,
                        name,
                        reason: format!("Appears in {} shared collection(s)", count),
                        co_occurrence_count: *count,
                        is_installed: mods.iter().any(|m| m.nexus_mod_id == Some(*nid)),
                    });
                }
            }
        }
    }

    // Sort by co-occurrence count descending
    recommendations.sort_by(|a, b| b.co_occurrence_count.cmp(&a.co_occurrence_count));

    // Limit to top 10
    recommendations.truncate(10);

    Ok(RecommendationResult {
        mod_id: target_mod_id,
        mod_name: target.name.clone(),
        recommendations,
    })
}

/// Get all mods that appear in multiple collections (popular mods).
pub fn get_popular_mods(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
) -> Result<Vec<(String, i64, usize)>, String> {
    let mods = db
        .list_mods(game_id, bottle_name)
        .map_err(|e| e.to_string())?;

    let mut collection_count: HashMap<i64, (String, usize)> = HashMap::new();
    let mut seen: HashMap<i64, std::collections::HashSet<String>> = HashMap::new();

    for m in &mods {
        if let (Some(nid), Some(ref coll)) = (m.nexus_mod_id, &m.collection_name) {
            let entry = seen.entry(nid).or_default();
            if entry.insert(coll.clone()) {
                let counter = collection_count
                    .entry(nid)
                    .or_insert_with(|| (m.name.clone(), 0));
                counter.1 += 1;
            }
        }
    }

    let mut popular: Vec<(String, i64, usize)> = collection_count
        .into_iter()
        .filter(|(_, (_, count))| *count > 1)
        .map(|(nid, (name, count))| (name, nid, count))
        .collect();

    popular.sort_by(|a, b| b.2.cmp(&a.2));
    Ok(popular)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> ModDatabase {
        ModDatabase::new(std::path::Path::new(":memory:")).unwrap()
    }

    fn add_mod_with_collection(
        db: &ModDatabase,
        name: &str,
        nexus_id: Option<i64>,
        collection: Option<&str>,
    ) -> i64 {
        let id = db
            .add_mod("skyrimse", "Gaming", nexus_id, name, "1.0", "test.zip", &[])
            .unwrap();
        if let Some(coll) = collection {
            db.set_collection_name(id, coll).unwrap();
        }
        id
    }

    #[test]
    fn recommendations_for_standalone_mod() {
        let db = test_db();
        let mod_a = add_mod_with_collection(&db, "Standalone", Some(100), None);
        let result = get_recommendations(&db, "skyrimse", "Gaming", mod_a).unwrap();
        assert!(result.recommendations.is_empty());
    }

    #[test]
    fn recommendations_mod_name_matches() {
        let db = test_db();
        let mod_a = add_mod_with_collection(&db, "My Mod", Some(100), None);
        let result = get_recommendations(&db, "skyrimse", "Gaming", mod_a).unwrap();
        assert_eq!(result.mod_name, "My Mod");
    }

    #[test]
    fn recommendations_mod_id_matches() {
        let db = test_db();
        let mod_a = add_mod_with_collection(&db, "My Mod", Some(100), None);
        let result = get_recommendations(&db, "skyrimse", "Gaming", mod_a).unwrap();
        assert_eq!(result.mod_id, mod_a);
    }

    #[test]
    fn recommendations_nonexistent_mod() {
        let db = test_db();
        let result = get_recommendations(&db, "skyrimse", "Gaming", 99999);
        assert!(result.is_err());
    }

    #[test]
    fn recommendations_from_same_collection() {
        let db = test_db();
        let mod_a = add_mod_with_collection(&db, "Mod A", Some(100), Some("Collection X"));
        let _mod_b = add_mod_with_collection(&db, "Mod B", Some(200), Some("Collection X"));
        let _mod_c = add_mod_with_collection(&db, "Mod C", Some(300), Some("Collection X"));

        let result = get_recommendations(&db, "skyrimse", "Gaming", mod_a).unwrap();
        assert_eq!(result.recommendations.len(), 2);
    }

    #[test]
    fn recommendations_different_collection_not_included() {
        let db = test_db();
        let mod_a = add_mod_with_collection(&db, "Mod A", Some(100), Some("Collection X"));
        let _mod_b = add_mod_with_collection(&db, "Mod B", Some(200), Some("Collection Y"));

        let result = get_recommendations(&db, "skyrimse", "Gaming", mod_a).unwrap();
        assert!(result.recommendations.is_empty());
    }

    #[test]
    fn recommendations_limited_to_10() {
        let db = test_db();
        let mod_a = add_mod_with_collection(&db, "Main", Some(1), Some("BigColl"));
        for i in 2..=15 {
            add_mod_with_collection(&db, &format!("Mod {}", i), Some(i as i64), Some("BigColl"));
        }

        let result = get_recommendations(&db, "skyrimse", "Gaming", mod_a).unwrap();
        assert!(result.recommendations.len() <= 10);
    }

    #[test]
    fn recommendations_sorted_by_cooccurrence() {
        let db = test_db();
        let mod_a = add_mod_with_collection(&db, "Mod A", Some(100), Some("Coll1"));
        add_mod_with_collection(&db, "Mod B", Some(200), Some("Coll1"));

        let result = get_recommendations(&db, "skyrimse", "Gaming", mod_a).unwrap();
        for window in result.recommendations.windows(2) {
            assert!(window[0].co_occurrence_count >= window[1].co_occurrence_count);
        }
    }

    #[test]
    fn recommendations_include_reason() {
        let db = test_db();
        let mod_a = add_mod_with_collection(&db, "Mod A", Some(100), Some("My Collection"));
        add_mod_with_collection(&db, "Mod B", Some(200), Some("My Collection"));

        let result = get_recommendations(&db, "skyrimse", "Gaming", mod_a).unwrap();
        assert!(!result.recommendations.is_empty());
        assert!(result.recommendations[0].reason.contains("My Collection"));
    }

    #[test]
    fn popular_mods_empty_db() {
        let db = test_db();
        let popular = get_popular_mods(&db, "skyrimse", "Gaming").unwrap();
        assert!(popular.is_empty());
    }

    #[test]
    fn popular_mods_single_collection() {
        let db = test_db();
        add_mod_with_collection(&db, "Mod A", Some(100), Some("Coll1"));
        add_mod_with_collection(&db, "Mod B", Some(200), Some("Coll1"));

        let popular = get_popular_mods(&db, "skyrimse", "Gaming").unwrap();
        // No mod appears in multiple collections
        assert!(popular.is_empty());
    }

    #[test]
    fn popular_mods_cross_collection() {
        let db = test_db();
        add_mod_with_collection(&db, "SkyUI", Some(100), Some("Coll1"));
        add_mod_with_collection(&db, "SkyUI", Some(100), Some("Coll2"));
        add_mod_with_collection(&db, "Other", Some(200), Some("Coll1"));

        let popular = get_popular_mods(&db, "skyrimse", "Gaming").unwrap();
        assert_eq!(popular.len(), 1);
        assert_eq!(popular[0].1, 100); // nexus_mod_id
        assert_eq!(popular[0].2, 2); // appears in 2 collections
    }

    #[test]
    fn popular_mods_sorted_by_count() {
        let db = test_db();
        add_mod_with_collection(&db, "ModA", Some(100), Some("C1"));
        add_mod_with_collection(&db, "ModA", Some(100), Some("C2"));
        add_mod_with_collection(&db, "ModB", Some(200), Some("C1"));
        add_mod_with_collection(&db, "ModB", Some(200), Some("C2"));
        add_mod_with_collection(&db, "ModB", Some(200), Some("C3"));

        let popular = get_popular_mods(&db, "skyrimse", "Gaming").unwrap();
        assert!(popular.len() >= 2);
        assert!(popular[0].2 >= popular[1].2);
    }
}
