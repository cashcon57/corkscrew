//! Tauri commands for Stardew Valley (native) mod status analysis.
//!
//! [`get_stardew_mod_status`] surfaces unmet SMAPI dependencies and UniqueID
//! conflicts so the frontend can display warnings on the Stardew mods page.

use std::path::Path;

use tauri::State;

use crate::plugins::stardew_valley_native::{
    analyze_mod_status, parse_manifest, InstalledModInfo, StardewModStatus,
};
use crate::AppState;

/// Stardew (native) game id — kept inline so a refactor of the plugin's
/// `game_id()` won't silently break this command.
const STARDEW_GAME_ID: &str = "stardew_valley_native";

/// Return per-mod dependency and conflict status for all installed
/// Stardew Valley (native) mods.
///
/// Pipeline:
///
/// 1. Read every mod registered under `stardew_valley_native` in the DB.
/// 2. For each enabled mod that has a `staging_path`, open
///    `<staging_path>/manifest.json` (or, if missing, recurse into the
///    staging tree to find the first `manifest.json` — SMAPI mod archives
///    often nest the manifest one directory deep).
/// 3. Build an [`InstalledModInfo`] from the parsed manifest.
/// 4. Hand the list to [`analyze_mod_status`] and return the per-mod result.
///
/// Mods without a parseable manifest are skipped silently — the warning
/// surface is "missing required dependency" and "duplicate UniqueID", not
/// "missing manifest" (the latter is a separate install-integrity concern
/// covered elsewhere).
#[tauri::command]
pub async fn get_stardew_mod_status(
    state: State<'_, AppState>,
) -> Result<Vec<StardewModStatus>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let mods = db
            .list_mods(STARDEW_GAME_ID, "")
            .map_err(|e| format!("list_mods failed: {e}"))?;

        let infos: Vec<InstalledModInfo> = mods
            .iter()
            .filter(|m| m.enabled)
            .filter_map(|m| {
                let staging = m.staging_path.as_deref()?;
                let manifest_path = find_manifest_in_staging(Path::new(staging))?;
                let manifest = parse_manifest(&manifest_path).ok()?;
                Some(InstalledModInfo {
                    unique_id: manifest.unique_id,
                    name: manifest.name,
                    version: manifest.version,
                    dependencies: manifest.dependencies,
                    minimum_api_version: manifest.minimum_api_version,
                })
            })
            .collect();

        Ok(analyze_mod_status(&infos))
    })
    .await
    .map_err(|e| format!("Stardew mod status task failed: {e}"))?
}

/// Locate the `manifest.json` for a SMAPI mod staged at `staging`.
///
/// SMAPI mod archives are inconsistent about depth: some ship the manifest at
/// the archive root, others nest it inside a single directory. We first check
/// the root and then fall back to a one-level deep search. Returns the first
/// match or `None`.
fn find_manifest_in_staging(staging: &Path) -> Option<std::path::PathBuf> {
    let direct = staging.join("manifest.json");
    if direct.is_file() {
        return Some(direct);
    }
    let read = std::fs::read_dir(staging).ok()?;
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let nested = path.join("manifest.json");
            if nested.is_file() {
                return Some(nested);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Build an in-memory DB with a Stardew mod that depends on a UniqueID
    /// that is NOT present in the installed list, and assert the resulting
    /// status reports the missing dependency.
    #[test]
    fn get_stardew_mod_status_returns_warnings_for_unknown_deps() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = crate::database::ModDatabase::new(&db_path).unwrap();

        // Stage a mod whose manifest declares a required dep on a missing
        // UniqueID. This mod must be present in the DB and enabled so the
        // command reads it.
        let staging = dir.path().join("staging/ModWithBadDep");
        fs::create_dir_all(&staging).unwrap();
        fs::write(
            staging.join("manifest.json"),
            br#"{
                "Name": "MyMod",
                "Author": "tester",
                "Version": "1.0.0",
                "UniqueID": "tester.mymod",
                "Dependencies": [
                    { "UniqueID": "missing.dependency", "IsRequired": true }
                ]
            }"#,
        )
        .unwrap();

        let mod_id = db
            .add_mod(
                STARDEW_GAME_ID,
                "",
                None,
                "MyMod",
                "1.0.0",
                "MyMod.zip",
                &["manifest.json".to_string()],
            )
            .unwrap();
        db.set_staging_path(mod_id, &staging.display().to_string())
            .unwrap();
        // add_mod inserts as enabled by default — confirm.

        // Run the same pipeline the command runs.
        let mods = db.list_mods(STARDEW_GAME_ID, "").unwrap();
        let infos: Vec<InstalledModInfo> = mods
            .iter()
            .filter(|m| m.enabled)
            .filter_map(|m| {
                let staging = m.staging_path.as_deref()?;
                let manifest_path = find_manifest_in_staging(Path::new(staging))?;
                let manifest = parse_manifest(&manifest_path).ok()?;
                Some(InstalledModInfo {
                    unique_id: manifest.unique_id,
                    name: manifest.name,
                    version: manifest.version,
                    dependencies: manifest.dependencies,
                    minimum_api_version: manifest.minimum_api_version,
                })
            })
            .collect();

        let statuses = analyze_mod_status(&infos);

        assert_eq!(statuses.len(), 1, "exactly one mod in the DB");
        let s = &statuses[0];
        assert_eq!(s.unique_id, "tester.mymod");
        assert!(
            s.missing_required_deps
                .iter()
                .any(|d| d == "missing.dependency"),
            "expected missing.dependency in missing_required_deps, got {:?}",
            s.missing_required_deps
        );
    }
}
