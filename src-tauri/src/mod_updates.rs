//! Nexus mod updates: update detection, install planning (replace vs. add)
//! and in-place replacement of an installed mod.
//!
//! The pure parts (parsing, version comparison, update-chain following,
//! install planning, routing Premium vs. free users) are unit-tested here
//! without network access. The replacement itself works against a real
//! database, staging directory and game data directory.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::database::{InstalledMod, ModDatabase};
use crate::deployer;
use crate::games::{self, DetectedGame};
use crate::installer;
use crate::nexus::ModUpdateInfo;
use crate::staging::{self, StagingResult};

// ---------------------------------------------------------------------------
// Nexus file categories (v1 API `category_id`)
// ---------------------------------------------------------------------------

pub const CATEGORY_MAIN: i64 = 1;
pub const CATEGORY_UPDATE: i64 = 2;
pub const CATEGORY_OPTIONAL: i64 = 3;
pub const CATEGORY_OLD_VERSION: i64 = 4;
pub const CATEGORY_MISC: i64 = 5;
pub const CATEGORY_DELETED: i64 = 6;
pub const CATEGORY_ARCHIVED: i64 = 7;

/// One file from a mod page's `files.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub file_id: i64,
    pub name: String,
    pub version: String,
    pub file_name: String,
    pub category_id: i64,
    pub uploaded_timestamp: i64,
}

/// One `file_updates` link: the author marked `new_file_id` as the
/// replacement for `old_file_id`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileUpdateLink {
    pub old_file_id: i64,
    pub new_file_id: i64,
    pub uploaded_timestamp: i64,
}

fn json_str(v: &serde_json::Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string()
}

pub fn parse_file_entries(files: &[serde_json::Value]) -> Vec<FileEntry> {
    files
        .iter()
        .filter_map(|f| {
            Some(FileEntry {
                file_id: f.get("file_id")?.as_i64()?,
                name: json_str(f, "name"),
                version: json_str(f, "version"),
                file_name: json_str(f, "file_name"),
                category_id: f.get("category_id").and_then(|v| v.as_i64()).unwrap_or(0),
                uploaded_timestamp: f
                    .get("uploaded_timestamp")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0),
            })
        })
        .collect()
}

pub fn parse_file_updates(updates: &[serde_json::Value]) -> Vec<FileUpdateLink> {
    updates
        .iter()
        .filter_map(|u| {
            Some(FileUpdateLink {
                old_file_id: u.get("old_file_id")?.as_i64()?,
                new_file_id: u.get("new_file_id")?.as_i64()?,
                uploaded_timestamp: u
                    .get("uploaded_timestamp")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Version comparison
// ---------------------------------------------------------------------------

/// Normalize a version string for equality checks: trims, lowercases, drops
/// a leading `v`, and drops trailing `.0` components so `1.2`, `v1.2.0` and
/// `1.2.0.0` compare equal.
pub fn normalize_version(v: &str) -> String {
    let t = v.trim().to_lowercase();
    let t = t.strip_prefix('v').unwrap_or(&t).trim();
    let mut parts: Vec<&str> = t.split('.').collect();
    while parts.len() > 1 && parts.last().is_some_and(|p| p.chars().all(|c| c == '0')) {
        parts.pop();
    }
    parts.join(".")
}

/// `true` only when both versions are known and equal after normalization.
/// An empty version is "unknown" and never equal to anything.
pub fn versions_equal(a: &str, b: &str) -> bool {
    let (a, b) = (normalize_version(a), normalize_version(b));
    !a.is_empty() && !b.is_empty() && a == b
}

// ---------------------------------------------------------------------------
// Update detection
// ---------------------------------------------------------------------------

/// File ids reachable from `start` through the `file_updates` chain, in
/// order. When an author linked one file to several replacements, the most
/// recently uploaded link wins. Cycles are cut.
pub fn follow_update_chain(start: i64, links: &[FileUpdateLink]) -> Vec<i64> {
    let mut seen = HashSet::from([start]);
    let mut chain = Vec::new();
    let mut current = start;
    loop {
        let next = links
            .iter()
            .filter(|l| l.old_file_id == current)
            .max_by_key(|l| (l.uploaded_timestamp, l.new_file_id));
        match next {
            Some(l) if seen.insert(l.new_file_id) => {
                chain.push(l.new_file_id);
                current = l.new_file_id;
            }
            _ => break,
        }
    }
    chain
}

/// Decide whether the installed file has a newer replacement.
///
/// 1. Follow the author's `file_updates` chain from the installed file and
///    take the newest file in it that is still listed. That is an update
///    unless it carries the same version (a re-upload).
/// 2. Without a chain entry, only flag an update when the installed file was
///    moved to "old version" or "archived" and a main file with the same name
///    (or the page's only main file) has a different version.
///
/// A newer sibling main file is never an update by itself: pages often ship
/// several main files (SE/AE builds, optional parts) side by side.
pub fn find_update<'a>(
    installed_file_id: i64,
    installed_version: &str,
    files: &'a [FileEntry],
    links: &[FileUpdateLink],
) -> Option<&'a FileEntry> {
    let installed = files.iter().find(|f| f.file_id == installed_file_id);
    let installed_ver = installed
        .map(|f| f.version.as_str())
        .filter(|v| !v.trim().is_empty())
        .unwrap_or(installed_version);

    let chain = follow_update_chain(installed_file_id, links);
    if !chain.is_empty() {
        let newest = chain.iter().rev().find_map(|id| {
            files
                .iter()
                .find(|f| f.file_id == *id && f.category_id != CATEGORY_DELETED)
        })?;
        if newest.file_id == installed_file_id || versions_equal(installed_ver, &newest.version) {
            return None;
        }
        return Some(newest);
    }

    let installed = installed?;
    if installed.category_id != CATEGORY_OLD_VERSION && installed.category_id != CATEGORY_ARCHIVED {
        return None;
    }
    let mains: Vec<&FileEntry> = files
        .iter()
        .filter(|f| f.category_id == CATEGORY_MAIN && f.file_id != installed_file_id)
        .collect();
    let candidate = mains
        .iter()
        .find(|f| !f.name.is_empty() && f.name.eq_ignore_ascii_case(&installed.name))
        .or_else(|| if mains.len() == 1 { mains.first() } else { None })?;
    if versions_equal(installed_ver, &candidate.version) {
        return None;
    }
    Some(candidate)
}

/// Name for a mod row installed from a specific Nexus file.
///
/// Main files keep the page title. Other files from the same page (optional
/// patches, add-ons) get their own name so several of them don't all show
/// as the page title.
pub fn display_name_for_file(mod_name: &str, file: Option<&FileEntry>) -> String {
    let Some(file) = file else {
        return mod_name.to_string();
    };
    let file_name = file.name.trim();
    if file_name.is_empty()
        || file.category_id == CATEGORY_MAIN
        || file_name.eq_ignore_ascii_case(mod_name.trim())
    {
        return mod_name.to_string();
    }
    if mod_name.trim().is_empty()
        || file_name
            .to_lowercase()
            .contains(&mod_name.trim().to_lowercase())
    {
        return file_name.to_string();
    }
    format!("{} - {}", mod_name.trim(), file_name)
}

// ---------------------------------------------------------------------------
// Install planning
// ---------------------------------------------------------------------------

/// The identifying fields of an installed mod that install planning needs.
#[derive(Clone, Debug)]
pub struct ModIdentity {
    pub id: i64,
    pub name: String,
    pub version: String,
    pub nexus_mod_id: Option<i64>,
    pub nexus_file_id: Option<i64>,
    pub collection_name: Option<String>,
}

impl From<&InstalledMod> for ModIdentity {
    fn from(m: &InstalledMod) -> Self {
        Self {
            id: m.id,
            name: m.name.clone(),
            version: m.version.clone(),
            nexus_mod_id: m.nexus_mod_id,
            nexus_file_id: m.nexus_file_id,
            collection_name: m.collection_name.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplaceReason {
    /// The caller asked to update this specific mod.
    Requested,
    /// The same Nexus file is already installed: reinstall it in place.
    SameFile,
    /// The file is the update found by the last update check.
    PendingUpdate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallPlan {
    /// Add a new mod row. `siblings` are installed mods from the same Nexus
    /// page (different files).
    New { siblings: Vec<i64> },
    /// Replace an installed mod's contents, keeping its row.
    Replace { mod_id: i64, reason: ReplaceReason },
}

/// Decide whether an incoming archive replaces an installed mod or is added
/// as a new one.
pub fn plan_install(
    existing: &[ModIdentity],
    pending_updates: &[ModUpdateInfo],
    nexus_mod_id: Option<i64>,
    nexus_file_id: Option<i64>,
    replace_mod_id: Option<i64>,
) -> Result<InstallPlan, String> {
    if let Some(target) = replace_mod_id {
        let row = existing
            .iter()
            .find(|m| m.id == target)
            .ok_or_else(|| format!("Mod {target} is not installed for this game"))?;
        if let (Some(incoming), Some(current)) = (nexus_mod_id, row.nexus_mod_id) {
            if incoming != current {
                return Err(format!(
                    "Refusing to update '{}' (Nexus mod {current}) with a file from Nexus mod {incoming}",
                    row.name
                ));
            }
        }
        return Ok(InstallPlan::Replace {
            mod_id: target,
            reason: ReplaceReason::Requested,
        });
    }

    let Some(nexus_mod_id) = nexus_mod_id else {
        return Ok(InstallPlan::New { siblings: vec![] });
    };

    if let Some(file_id) = nexus_file_id {
        if let Some(same) = existing
            .iter()
            .find(|m| m.nexus_mod_id == Some(nexus_mod_id) && m.nexus_file_id == Some(file_id))
        {
            return Ok(InstallPlan::Replace {
                mod_id: same.id,
                reason: ReplaceReason::SameFile,
            });
        }
        if let Some(update) = pending_updates.iter().find(|u| {
            u.nexus_mod_id == nexus_mod_id
                && u.latest_file_id == file_id
                && existing.iter().any(|m| m.id == u.mod_id)
        }) {
            return Ok(InstallPlan::Replace {
                mod_id: update.mod_id,
                reason: ReplaceReason::PendingUpdate,
            });
        }
    }

    let siblings = existing
        .iter()
        .filter(|m| m.nexus_mod_id == Some(nexus_mod_id))
        .map(|m| m.id)
        .collect();
    Ok(InstallPlan::New { siblings })
}

/// What an install will do, for showing the user before they confirm.
#[derive(Clone, Debug, Default, Serialize)]
pub struct InstallPreview {
    pub replaces_mod_id: Option<i64>,
    pub reason: Option<ReplaceReason>,
    pub note: Option<String>,
}

/// Warning shown when an update moves a collection mod off the file the
/// collection pinned.
pub fn collection_divergence_note(mod_name: &str, collection: &str) -> String {
    format!(
        "'{mod_name}' was installed by the collection '{collection}'. \
         Updating it replaces the file the collection pins, so your setup will \
         differ from the collection."
    )
}

pub fn describe_plan(plan: &InstallPlan, existing: &[ModIdentity]) -> InstallPreview {
    match plan {
        InstallPlan::New { siblings } if siblings.is_empty() => InstallPreview::default(),
        InstallPlan::New { siblings } => {
            let names: Vec<String> = existing
                .iter()
                .filter(|m| siblings.contains(&m.id))
                .map(|m| format!("'{}'", m.name))
                .collect();
            InstallPreview {
                note: Some(format!(
                    "Another file from this Nexus mod is already installed ({}). \
                     This one will be added as a separate mod.",
                    names.join(", ")
                )),
                ..Default::default()
            }
        }
        InstallPlan::Replace { mod_id, reason } => {
            let row = existing.iter().find(|m| m.id == *mod_id);
            let name = row.map(|m| m.name.as_str()).unwrap_or("this mod");
            let mut note = match reason {
                ReplaceReason::SameFile => format!(
                    "'{name}' is already installed from this file. Installing reinstalls it in place."
                ),
                ReplaceReason::PendingUpdate | ReplaceReason::Requested => format!(
                    "Updates '{name}'{} in place, keeping its load order position and enabled state.",
                    row.filter(|m| !m.version.is_empty())
                        .map(|m| format!(" (currently v{})", m.version))
                        .unwrap_or_default()
                ),
            };
            if *reason != ReplaceReason::SameFile {
                if let Some(collection) = row.and_then(|m| m.collection_name.as_deref()) {
                    note.push(' ');
                    note.push_str(&collection_divergence_note(name, collection));
                }
            }
            InstallPreview {
                replaces_mod_id: Some(*mod_id),
                reason: Some(*reason),
                note: Some(note),
            }
        }
    }
}

/// Plan an install against the current mods and stored update results.
pub fn plan_install_for(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    nexus_mod_id: Option<i64>,
    nexus_file_id: Option<i64>,
    replace_mod_id: Option<i64>,
) -> Result<(InstallPlan, Vec<ModIdentity>), String> {
    let existing: Vec<ModIdentity> = db
        .list_mods(game_id, bottle_name)
        .map_err(|e| e.to_string())?
        .iter()
        .map(ModIdentity::from)
        .collect();
    let pending = db
        .list_pending_updates(game_id, bottle_name)
        .map_err(|e| e.to_string())?;
    let plan = plan_install(&existing, &pending, nexus_mod_id, nexus_file_id, replace_mod_id)?;
    Ok((plan, existing))
}

// ---------------------------------------------------------------------------
// Premium vs. free routing
// ---------------------------------------------------------------------------

/// How a single update gets downloaded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UpdateRoute {
    /// Premium: Corkscrew downloads through the API and replaces the mod.
    ApiDownload,
    /// Free account: open the file on the Nexus website. Nexus policy forbids
    /// automating downloads for free users.
    Browser { url: String },
}

/// Nexus page for one file; `file_id` opens that file's download dialog.
pub fn nexus_file_page_url(game_slug: &str, nexus_mod_id: i64, file_id: i64) -> String {
    format!("https://www.nexusmods.com/{game_slug}/mods/{nexus_mod_id}?tab=files&file_id={file_id}")
}

pub fn route_update(is_premium: bool, game_slug: &str, nexus_mod_id: i64, file_id: i64) -> UpdateRoute {
    if is_premium {
        UpdateRoute::ApiDownload
    } else {
        UpdateRoute::Browser {
            url: nexus_file_page_url(game_slug, nexus_mod_id, file_id),
        }
    }
}

/// Nexus game domain for a game id when the detected game isn't available.
pub fn fallback_game_slug(game_id: &str) -> &str {
    match game_id {
        "skyrimse" => "skyrimspecialedition",
        other => other,
    }
}

// ---------------------------------------------------------------------------
// In-place replacement
// ---------------------------------------------------------------------------

/// New metadata for a replaced mod.
#[derive(Clone, Debug)]
pub struct ReplaceSpec {
    pub name: String,
    pub version: String,
    pub archive_name: String,
    /// `(nexus_mod_id, nexus_file_id)` to record, or `None` to keep the
    /// row's current ids.
    pub nexus_ids: Option<(i64, Option<i64>)>,
}

pub struct ReplaceOutcome {
    pub installed: InstalledMod,
    pub removed_files: Vec<String>,
}

/// Replace an installed mod with already-staged new contents.
///
/// Order matters so a failure part-way leaves a usable state:
/// 1. snapshot the mod configuration,
/// 2. undeploy the old files (the row is disabled meanwhile so the undeploy
///    doesn't restore its own files as the "next winner"),
/// 3. delete the old staging folder and move the new one into place,
/// 4. update the row (same id: priority, enabled state, collection, notes,
///    tags and profile membership stay),
/// 5. deploy the new files if the mod was enabled.
///
/// `incoming` must already be staged; on an early failure it is removed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn replace_staged_mod(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    game_path: &Path,
    data_dir: &Path,
    old_id: i64,
    spec: &ReplaceSpec,
    incoming: StagingResult,
    final_dir: &Path,
    resolve_target: &dyn Fn(&[String]) -> (PathBuf, String),
) -> Result<ReplaceOutcome, String> {
    let discard_incoming = |incoming: &StagingResult| {
        if let Err(e) = staging::remove_staging(&incoming.staging_path) {
            log::warn!(
                "Failed to remove unused staging {}: {e}",
                incoming.staging_path.display()
            );
        }
    };

    let old = match db.get_mod(old_id) {
        Ok(Some(m)) if m.game_id == game_id && m.bottle_name == bottle_name => m,
        Ok(_) => {
            discard_incoming(&incoming);
            return Err(format!("Mod {old_id} is not installed for this game"));
        }
        Err(e) => {
            discard_incoming(&incoming);
            return Err(e.to_string());
        }
    };

    crate::auto_snapshot_before_destructive(
        db,
        game_id,
        bottle_name,
        &format!("Before updating '{}'", old.name),
    );

    let old_target = db
        .get_deploy_target_for_mod(old_id)
        .unwrap_or_else(|_| "data".to_string());

    // Undeploy the old files.
    let removed_files = if old.staging_path.is_some() {
        if let Err(e) = db.set_enabled(old_id, false) {
            discard_incoming(&incoming);
            return Err(e.to_string());
        }
        match deployer::undeploy_mod(db, game_id, bottle_name, old_id, data_dir, game_path) {
            Ok(removed) => removed,
            Err(e) => {
                if let Err(e2) = db.set_enabled(old_id, old.enabled) {
                    log::warn!("Failed to restore enabled state of mod {old_id}: {e2}");
                }
                discard_incoming(&incoming);
                return Err(format!("Failed to remove the old files of '{}': {e}", old.name));
            }
        }
    } else {
        installer::uninstall_mod_files(data_dir, &old.installed_files).map_err(|e| {
            discard_incoming(&incoming);
            format!("Failed to remove the old files of '{}': {e}", old.name)
        })?
    };

    // Drop the old staging folder and move the new one to its final name.
    if let Some(old_staging) = old.staging_path.as_deref().map(PathBuf::from) {
        if old_staging != incoming.staging_path {
            if let Err(e) = staging::remove_staging(&old_staging) {
                log::warn!("Failed to remove old staging {}: {e}", old_staging.display());
            }
        }
    }
    let mut staging_path = incoming.staging_path.clone();
    if staging_path != final_dir {
        if final_dir.exists() {
            if let Err(e) = staging::remove_staging(final_dir) {
                log::warn!("Failed to clear {}: {e}", final_dir.display());
            }
        }
        match std::fs::rename(&staging_path, final_dir) {
            Ok(()) => staging_path = final_dir.to_path_buf(),
            Err(e) => log::warn!(
                "Keeping staging at {} (rename to {} failed: {e})",
                staging_path.display(),
                final_dir.display()
            ),
        }
    }

    let auto_category = ModDatabase::classify_mod_category(&incoming.files);
    db.replace_mod_contents(
        old_id,
        &spec.name,
        &spec.version,
        &spec.archive_name,
        spec.nexus_ids,
        &staging_path.to_string_lossy(),
        &incoming.files,
        &incoming.hashes,
        auto_category.as_deref(),
    )
    .map_err(|e| format!("Failed to record the new files of '{}': {e}", spec.name))?;
    db.set_enabled(old_id, old.enabled).map_err(|e| e.to_string())?;

    let (deploy_dir, target) = if old_target == "root" {
        (game_path.to_path_buf(), old_target)
    } else {
        resolve_target(&incoming.files)
    };
    if let Err(e) = db.set_deploy_target_for_mod(old_id, &target) {
        log::warn!("Failed to store deploy target for mod {old_id}: {e}");
    }
    if old.enabled {
        deployer::deploy_mod(
            db,
            game_id,
            bottle_name,
            old_id,
            &staging_path,
            &deploy_dir,
            &incoming.files,
            &target,
        )
        .map_err(|e| {
            format!(
                "'{}' was updated, but deploying its new files failed: {e}. Use Deploy to retry.",
                spec.name
            )
        })?;
    }

    let installed = db
        .get_mod(old_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Mod {old_id} disappeared during update"))?;
    log::info!(
        "Replaced mod {} '{}' v{} -> '{}' v{} ({} files removed, {} files staged)",
        old_id,
        old.name,
        old.version,
        installed.name,
        installed.version,
        removed_files.len(),
        incoming.files.len()
    );
    Ok(ReplaceOutcome {
        installed,
        removed_files,
    })
}

/// Stage `archive` and replace mod `old_id` with it in place. The caller
/// holds the `DeployGuard` and syncs plugins afterwards.
#[allow(clippy::too_many_arguments)]
pub(crate) fn replace_mod_in_place(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    game: &DetectedGame,
    data_dir: &Path,
    old_id: i64,
    archive: &Path,
    spec: &ReplaceSpec,
) -> Result<InstalledMod, String> {
    // Stage next to the old copy first so a bad archive leaves the installed
    // mod untouched.
    let incoming = staging::stage_mod(
        archive,
        game_id,
        bottle_name,
        old_id,
        &format!("{}.incoming", spec.name),
    )
    .map_err(|e| format!("Staging failed: {e}"))?;
    let final_dir = staging::mod_staging_dir(game_id, bottle_name, old_id, &spec.name);

    let resolve = |files: &[String]| {
        let (dir, _) = crate::commands::mods::resolve_effective_deploy_dir(
            game_id, game, data_dir, &spec.name, files,
        );
        let target = if dir != data_dir { "custom" } else { "data" };
        (dir, target.to_string())
    };

    let outcome = replace_staged_mod(
        db,
        game_id,
        bottle_name,
        &game.game_path,
        data_dir,
        old_id,
        spec,
        incoming,
        &final_dir,
        &resolve,
    )?;

    games::with_plugin(game_id, |plugin| {
        plugin.on_mod_undeployed(&game.game_path, None, &outcome.removed_files);
        if outcome.installed.enabled {
            let mod_type = plugin.detect_mod_type_from_files(&outcome.installed.installed_files);
            plugin.on_mod_deployed(
                &game.game_path,
                mod_type.as_deref(),
                &outcome.installed.installed_files,
            );
        }
    });

    Ok(outcome.installed)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::UpdateCheckRow;
    use serde_json::json;
    use tempfile::TempDir;

    fn file(id: i64, name: &str, version: &str, cat: i64, ts: i64) -> FileEntry {
        FileEntry {
            file_id: id,
            name: name.into(),
            version: version.into(),
            file_name: format!("{name}-{id}.7z"),
            category_id: cat,
            uploaded_timestamp: ts,
        }
    }

    fn link(old: i64, new: i64, ts: i64) -> FileUpdateLink {
        FileUpdateLink {
            old_file_id: old,
            new_file_id: new,
            uploaded_timestamp: ts,
        }
    }

    fn ident(id: i64, name: &str, nexus: Option<i64>, file: Option<i64>) -> ModIdentity {
        ModIdentity {
            id,
            name: name.into(),
            version: "1.0".into(),
            nexus_mod_id: nexus,
            nexus_file_id: file,
            collection_name: None,
        }
    }

    fn pending(mod_id: i64, nexus: i64, latest: i64) -> ModUpdateInfo {
        ModUpdateInfo {
            mod_id,
            nexus_mod_id: nexus,
            mod_name: "x".into(),
            current_version: "1.0".into(),
            latest_version: "2.0".into(),
            latest_file_name: "x".into(),
            latest_file_id: latest,
        }
    }

    // -- parsing --

    #[test]
    fn parses_files_and_file_updates() {
        let files = parse_file_entries(&[
            json!({"file_id": 10, "name": "Main", "version": "1.0", "file_name": "a.7z",
                   "category_id": 1, "uploaded_timestamp": 100}),
            json!({"name": "no id"}),
        ]);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].category_id, CATEGORY_MAIN);
        let links = parse_file_updates(&[
            json!({"old_file_id": 10, "new_file_id": 11, "uploaded_timestamp": 200}),
            json!({"old_file_id": 10}),
        ]);
        assert_eq!(links, vec![link(10, 11, 200)]);
    }

    // -- versions --

    #[test]
    fn version_equality_is_normalized() {
        assert!(versions_equal("1.2", "v1.2.0"));
        assert!(versions_equal(" 3.18 ", "3.18.0.0"));
        assert!(!versions_equal("3.18", "3.22"));
        assert!(!versions_equal("", ""));
        assert!(!versions_equal("1.0", ""));
        assert!(versions_equal("0", "0.0"));
    }

    // -- update detection --

    #[test]
    fn same_version_sibling_main_file_is_not_an_update() {
        // Installed main file 100; the author uploaded another main file
        // (e.g. the AE build) with a higher id and the same version.
        let files = vec![
            file(100, "Mod SE", "1.5", CATEGORY_MAIN, 1),
            file(200, "Mod AE", "1.5", CATEGORY_MAIN, 2),
        ];
        assert!(find_update(100, "1.5", &files, &[]).is_none());
    }

    #[test]
    fn newer_sibling_main_file_without_chain_is_not_an_update() {
        let files = vec![
            file(100, "Part A", "1.0", CATEGORY_MAIN, 1),
            file(200, "Part B", "2.0", CATEGORY_MAIN, 2),
        ];
        assert!(find_update(100, "1.0", &files, &[]).is_none());
    }

    #[test]
    fn file_updates_chain_is_flagged() {
        let files = vec![
            file(100, "Mod", "3.18", CATEGORY_OLD_VERSION, 1),
            file(150, "Mod", "3.20", CATEGORY_OLD_VERSION, 2),
            file(200, "Mod", "3.22", CATEGORY_MAIN, 3),
        ];
        let links = vec![link(100, 150, 2), link(150, 200, 3)];
        let up = find_update(100, "3.18", &files, &links).expect("update");
        assert_eq!(up.file_id, 200);
        assert_eq!(up.version, "3.22");
    }

    #[test]
    fn chain_to_same_version_reupload_is_not_flagged() {
        let files = vec![
            file(100, "Mod", "1.0", CATEGORY_OLD_VERSION, 1),
            file(101, "Mod", "1.0", CATEGORY_MAIN, 2),
        ];
        assert!(find_update(100, "1.0", &files, &[link(100, 101, 2)]).is_none());
    }

    #[test]
    fn chain_skips_files_no_longer_listed() {
        let files = vec![
            file(100, "Mod", "1.0", CATEGORY_OLD_VERSION, 1),
            file(150, "Mod", "1.1", CATEGORY_MAIN, 2),
        ];
        // 150 -> 200 exists in the chain but 200 was removed from the page.
        let links = vec![link(100, 150, 2), link(150, 200, 3)];
        assert_eq!(find_update(100, "1.0", &files, &links).unwrap().file_id, 150);
    }

    #[test]
    fn chain_cycles_terminate() {
        let links = vec![link(1, 2, 1), link(2, 1, 2)];
        assert_eq!(follow_update_chain(1, &links), vec![2]);
    }

    #[test]
    fn chain_prefers_most_recent_link() {
        let links = vec![link(1, 2, 5), link(1, 3, 9)];
        assert_eq!(follow_update_chain(1, &links), vec![3]);
    }

    #[test]
    fn old_version_without_chain_uses_matching_main_file() {
        let files = vec![
            file(100, "Mod", "1.5.4", CATEGORY_OLD_VERSION, 1),
            file(200, "Mod", "1.6.0", CATEGORY_MAIN, 2),
            file(300, "Other Part", "9.0", CATEGORY_MAIN, 3),
        ];
        assert_eq!(find_update(100, "1.5.4", &files, &[]).unwrap().file_id, 200);
    }

    #[test]
    fn old_version_with_same_version_main_is_not_flagged() {
        let files = vec![
            file(100, "Mod", "2.0", CATEGORY_ARCHIVED, 1),
            file(200, "Mod", "v2.0.0", CATEGORY_MAIN, 2),
        ];
        assert!(find_update(100, "2.0", &files, &[]).is_none());
    }

    #[test]
    fn installed_main_file_without_chain_is_up_to_date() {
        let files = vec![file(100, "Mod", "1.0", CATEGORY_MAIN, 1)];
        assert!(find_update(100, "1.0", &files, &[]).is_none());
    }

    // -- names --

    #[test]
    fn sibling_files_get_distinct_names() {
        let main = file(368378, "Scrambled Bugs - Anniversary Edition", "1", CATEGORY_MAIN, 1);
        let opt1 = file(368379, "Script Effect Archetype Crash Fix", "1", CATEGORY_OPTIONAL, 1);
        let opt2 = file(368380, "Vendor Respawn Fix", "1", CATEGORY_OPTIONAL, 1);
        let opt3 = file(1, "Scrambled Bugs Patch", "1", CATEGORY_OPTIONAL, 1);
        assert_eq!(display_name_for_file("Scrambled Bugs", Some(&main)), "Scrambled Bugs");
        assert_eq!(
            display_name_for_file("Scrambled Bugs", Some(&opt1)),
            "Scrambled Bugs - Script Effect Archetype Crash Fix"
        );
        assert_eq!(
            display_name_for_file("Scrambled Bugs", Some(&opt2)),
            "Scrambled Bugs - Vendor Respawn Fix"
        );
        assert_eq!(display_name_for_file("Scrambled Bugs", Some(&opt3)), "Scrambled Bugs Patch");
        assert_eq!(display_name_for_file("Scrambled Bugs", None), "Scrambled Bugs");
    }

    // -- install planning --

    #[test]
    fn same_nexus_file_replaces_existing_row() {
        let existing = vec![ident(1, "A", Some(500), Some(10)), ident(2, "B", None, None)];
        let plan = plan_install(&existing, &[], Some(500), Some(10), None).unwrap();
        assert_eq!(
            plan,
            InstallPlan::Replace {
                mod_id: 1,
                reason: ReplaceReason::SameFile
            }
        );
    }

    #[test]
    fn pending_update_file_replaces_its_mod() {
        let existing = vec![ident(1, "A", Some(500), Some(10))];
        let plan = plan_install(&existing, &[pending(1, 500, 11)], Some(500), Some(11), None).unwrap();
        assert_eq!(
            plan,
            InstallPlan::Replace {
                mod_id: 1,
                reason: ReplaceReason::PendingUpdate
            }
        );
    }

    #[test]
    fn other_file_from_same_page_is_new_with_siblings() {
        let existing = vec![ident(1, "A", Some(500), Some(10))];
        let plan = plan_install(&existing, &[], Some(500), Some(12), None).unwrap();
        assert_eq!(plan, InstallPlan::New { siblings: vec![1] });
        let preview = describe_plan(&plan, &existing);
        assert!(preview.note.unwrap().contains("'A'"));
        assert!(preview.replaces_mod_id.is_none());
    }

    #[test]
    fn manual_archive_is_new() {
        let existing = vec![ident(1, "A", Some(500), Some(10))];
        assert_eq!(
            plan_install(&existing, &[], None, None, None).unwrap(),
            InstallPlan::New { siblings: vec![] }
        );
    }

    #[test]
    fn explicit_replace_is_validated() {
        let existing = vec![ident(1, "A", Some(500), Some(10))];
        assert!(plan_install(&existing, &[], Some(500), Some(11), Some(99)).is_err());
        assert!(plan_install(&existing, &[], Some(777), Some(11), Some(1)).is_err());
        assert_eq!(
            plan_install(&existing, &[], Some(500), Some(11), Some(1)).unwrap(),
            InstallPlan::Replace {
                mod_id: 1,
                reason: ReplaceReason::Requested
            }
        );
    }

    #[test]
    fn updating_a_collection_mod_warns_about_divergence() {
        let mut row = ident(1, "SkyUI", Some(500), Some(10));
        row.collection_name = Some("Essential Mods".into());
        let plan = InstallPlan::Replace {
            mod_id: 1,
            reason: ReplaceReason::PendingUpdate,
        };
        let note = describe_plan(&plan, &[row.clone()]).note.unwrap();
        assert!(note.contains("Essential Mods"), "{note}");
        // A plain reinstall of the pinned file doesn't diverge.
        let same = InstallPlan::Replace {
            mod_id: 1,
            reason: ReplaceReason::SameFile,
        };
        assert!(!describe_plan(&same, &[row]).note.unwrap().contains("Essential Mods"));
    }

    // -- routing --

    #[test]
    fn premium_updates_never_open_the_browser() {
        assert_eq!(route_update(true, "skyrimspecialedition", 1, 2), UpdateRoute::ApiDownload);
    }

    #[test]
    fn free_updates_never_use_api_downloads() {
        match route_update(false, "skyrimspecialedition", 43532, 368380) {
            UpdateRoute::Browser { url } => assert_eq!(
                url,
                "https://www.nexusmods.com/skyrimspecialedition/mods/43532?tab=files&file_id=368380"
            ),
            other => panic!("free user routed to {other:?}"),
        }
    }

    // -- persistence + replacement against a real database --

    fn setup() -> (ModDatabase, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db = ModDatabase::new(&tmp.path().join("mods.db")).unwrap();
        crate::rollback::init_schema(&db).unwrap();
        crate::profiles::init_schema(&db).unwrap();
        (db, tmp)
    }

    fn write(dir: &Path, rel: &str, body: &[u8]) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    #[test]
    fn update_results_survive_reopening_the_db() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("mods.db");
        let mod_id;
        {
            let db = ModDatabase::new(&path).unwrap();
            mod_id = db
                .add_mod("skyrimse", "Steam", Some(120352), "Mod", "3.18", "a.7z", &[])
                .unwrap();
            db.set_nexus_ids(mod_id, 120352, Some(1)).unwrap();
            let up_to_date = db
                .add_mod("skyrimse", "Steam", Some(2482), "Other", "1.0", "b.7z", &[])
                .unwrap();
            db.save_update_check_results(
                "skyrimse",
                "Steam",
                &[
                    UpdateCheckRow {
                        mod_id,
                        nexus_mod_id: 120352,
                        installed_file_id: Some(1),
                        latest: Some((2, "3.22".into(), "Mod".into())),
                    },
                    UpdateCheckRow {
                        mod_id: up_to_date,
                        nexus_mod_id: 2482,
                        installed_file_id: None,
                        latest: None,
                    },
                ],
            )
            .unwrap();
        }
        let db = ModDatabase::new(&path).unwrap();
        let pending = db.list_pending_updates("skyrimse", "Steam").unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].mod_id, mod_id);
        assert_eq!(pending[0].latest_file_id, 2);
        assert_eq!(pending[0].latest_version, "3.22");
        assert!(db.list_pending_updates("skyrimse", "Other").unwrap().is_empty());

        // Once the mod is on the new file, the stored result no longer counts.
        db.set_nexus_ids(mod_id, 120352, Some(2)).unwrap();
        assert!(db.list_pending_updates("skyrimse", "Steam").unwrap().is_empty());

        // And rows go away with their mod.
        db.remove_mod(mod_id).unwrap();
        let conn = db.conn().unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM mod_update_checks WHERE mod_id = ?1", [mod_id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn reinstalling_same_file_replaces_in_place() {
        let (db, tmp) = setup();
        let data_dir = tmp.path().join("Data");
        let game_path = tmp.path().join("game");
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::create_dir_all(&game_path).unwrap();

        // An unrelated mod so priorities aren't trivially 0.
        let other = db
            .add_mod("testgame", "B", None, "Other", "1", "o.7z", &[])
            .unwrap();
        db.set_mod_priority(other, 0).unwrap();

        // Installed collection mod: Nexus 500 / file 10, priority 7.
        let old_staging = tmp.path().join("staging/2_Mod");
        write(&old_staging, "textures/old.dds", b"old");
        let old_files = vec!["textures/old.dds".to_string()];
        let id = db
            .add_mod("testgame", "B", Some(500), "Mod", "1.0", "mod-1.0.7z", &old_files)
            .unwrap();
        db.set_nexus_ids(id, 500, Some(10)).unwrap();
        db.set_mod_priority(id, 7).unwrap();
        db.set_collection_name(id, "Essential Mods").unwrap();
        db.set_staging_path(id, &old_staging.to_string_lossy()).unwrap();
        deployer::deploy_mod(&db, "testgame", "B", id, &old_staging, &data_dir, &old_files, "data")
            .unwrap();
        assert!(data_dir.join("textures/old.dds").exists());

        // The same (mod, file) arrives again: the plan must be a replace.
        let (plan, _) = plan_install_for(&db, "testgame", "B", Some(500), Some(10), None).unwrap();
        let InstallPlan::Replace { mod_id, reason } = plan else {
            panic!("expected replace, got {plan:?}");
        };
        assert_eq!((mod_id, reason), (id, ReplaceReason::SameFile));

        let incoming_dir = tmp.path().join("staging/2_Mod.incoming");
        write(&incoming_dir, "textures/new.dds", b"new");
        let incoming = StagingResult {
            staging_path: incoming_dir.clone(),
            files: vec!["textures/new.dds".to_string()],
            hashes: vec![("textures/new.dds".into(), "abc".into(), 3)],
        };
        let final_dir = tmp.path().join("staging/2_Mod");
        let data = data_dir.clone();
        let resolve = move |_: &[String]| (data.clone(), "data".to_string());
        let spec = ReplaceSpec {
            name: "Mod".into(),
            version: "1.0".into(),
            archive_name: "mod-1.0.7z".into(),
            nexus_ids: Some((500, Some(10))),
        };
        let out = replace_staged_mod(
            &db, "testgame", "B", &game_path, &data_dir, id, &spec, incoming, &final_dir, &resolve,
        )
        .unwrap();

        let mods = db.list_mods("testgame", "B").unwrap();
        assert_eq!(mods.len(), 2, "no duplicate row");
        let m = &out.installed;
        assert_eq!(m.id, id);
        assert_eq!(m.install_priority, 7);
        assert_eq!(m.collection_name.as_deref(), Some("Essential Mods"));
        assert!(m.enabled);
        assert_eq!(m.nexus_file_id, Some(10));
        assert_eq!(m.installed_files, vec!["textures/new.dds".to_string()]);
        assert_eq!(m.staging_path.as_deref(), Some(final_dir.to_string_lossy().as_ref()));
        assert!(!data_dir.join("textures/old.dds").exists(), "old file undeployed");
        assert!(data_dir.join("textures/new.dds").exists(), "new file deployed");
        assert!(!incoming_dir.exists(), "incoming staging moved into place");
        assert_eq!(db.get_file_hashes(id).unwrap().len(), 1);
    }

    #[test]
    fn replacing_a_disabled_mod_keeps_it_disabled_and_undeployed() {
        let (db, tmp) = setup();
        let data_dir = tmp.path().join("Data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let old_staging = tmp.path().join("staging/1_Mod");
        write(&old_staging, "a.txt", b"a");
        let id = db
            .add_mod("testgame", "B", Some(1), "Mod", "1.0", "a.7z", &["a.txt".to_string()])
            .unwrap();
        db.set_staging_path(id, &old_staging.to_string_lossy()).unwrap();
        db.set_enabled(id, false).unwrap();

        let incoming_dir = tmp.path().join("staging/1_Mod v2.incoming");
        write(&incoming_dir, "b.txt", b"b");
        let incoming = StagingResult {
            staging_path: incoming_dir,
            files: vec!["b.txt".to_string()],
            hashes: vec![],
        };
        let final_dir = tmp.path().join("staging/1_Mod_v2");
        let data = data_dir.clone();
        let resolve = move |_: &[String]| (data.clone(), "data".to_string());
        let spec = ReplaceSpec {
            name: "Mod v2".into(),
            version: "2.0".into(),
            archive_name: "b.7z".into(),
            nexus_ids: Some((1, Some(2))),
        };
        let out = replace_staged_mod(
            &db, "testgame", "B", tmp.path(), &data_dir, id, &spec, incoming, &final_dir, &resolve,
        )
        .unwrap();
        assert!(!out.installed.enabled);
        assert_eq!(out.installed.version, "2.0");
        assert_eq!(out.installed.nexus_file_id, Some(2));
        assert!(!data_dir.join("b.txt").exists());
        assert!(!old_staging.exists(), "old staging removed");
        assert!(final_dir.join("b.txt").exists());
    }
}
