//! Deployment Journal — write-ahead log for deployment operations.
//!
//! Before any deployment change (deploy, undeploy, redeploy, purge), we write
//! intent to a journal file.  After the operation completes, we mark it done.
//! On startup, any incomplete entries trigger a self-healing redeploy to bring
//! the deployment back to a consistent state.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// A single journal entry representing a deployment operation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JournalEntry {
    pub id: String,
    pub game_id: String,
    pub bottle_name: String,
    pub operation: JournalOp,
    /// Mod IDs affected (empty for redeploy_all / purge).
    pub mod_ids: Vec<i64>,
    pub timestamp: String, // RFC 3339
    pub status: JournalStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum JournalOp {
    Deploy,
    Undeploy,
    RedeployAll,
    Purge,
    BatchToggle,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum JournalStatus {
    Pending,
    Complete,
}

impl std::fmt::Display for JournalOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JournalOp::Deploy => write!(f, "deploy"),
            JournalOp::Undeploy => write!(f, "undeploy"),
            JournalOp::RedeployAll => write!(f, "redeploy_all"),
            JournalOp::Purge => write!(f, "purge"),
            JournalOp::BatchToggle => write!(f, "batch_toggle"),
        }
    }
}

/// Path to the journal file.
fn journal_path() -> PathBuf {
    crate::config::data_dir().join("deploy_journal.json")
}

/// Read all journal entries from disk.
fn read_journal() -> Vec<JournalEntry> {
    let path = journal_path();
    match fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Write journal entries to disk (atomic via temp file + rename).
fn write_journal(entries: &[JournalEntry]) -> Result<(), String> {
    let path = journal_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create journal dir: {e}"))?;
    }
    let tmp = path.with_extension("tmp");
    let data = serde_json::to_string_pretty(entries).map_err(|e| format!("serialize: {e}"))?;
    fs::write(&tmp, data).map_err(|e| format!("write journal tmp: {e}"))?;
    fs::rename(&tmp, &path).map_err(|e| format!("rename journal: {e}"))?;
    Ok(())
}

/// Begin a deployment operation — writes a pending entry to the journal.
/// Returns the entry ID for later completion.
pub fn begin(
    game_id: &str,
    bottle_name: &str,
    operation: JournalOp,
    mod_ids: &[i64],
) -> Result<String, String> {
    let id = format!("{}-{}", chrono::Utc::now().timestamp_millis(), uuid_short());
    let entry = JournalEntry {
        id: id.clone(),
        game_id: game_id.to_string(),
        bottle_name: bottle_name.to_string(),
        operation,
        mod_ids: mod_ids.to_vec(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        status: JournalStatus::Pending,
    };

    log::info!(
        "deploy_journal: begin {} for {}/{} (id={})",
        entry.operation,
        game_id,
        bottle_name,
        id
    );

    let mut entries = read_journal();
    entries.push(entry);
    write_journal(&entries)?;
    Ok(id)
}

/// Mark a journal entry as complete.
pub fn complete(id: &str) -> Result<(), String> {
    let mut entries = read_journal();
    if let Some(entry) = entries.iter_mut().find(|e| e.id == id) {
        entry.status = JournalStatus::Complete;
        write_journal(&entries)?;
        log::info!("deploy_journal: complete id={}", id);
    }
    // Prune completed entries older than 24 hours
    prune_old_entries(&mut entries);
    Ok(())
}

/// Check for incomplete journal entries (called on startup).
/// Returns entries that need replay.
pub fn get_incomplete() -> Vec<JournalEntry> {
    read_journal()
        .into_iter()
        .filter(|e| e.status == JournalStatus::Pending)
        .collect()
}

/// Replay incomplete journal entries by triggering a redeploy for each
/// affected (game_id, bottle_name) pair.  Returns the set of game/bottle
/// pairs that were healed.
pub fn replay_incomplete(
    db: &std::sync::Arc<crate::database::ModDatabase>,
) -> Vec<(String, String)> {
    let incomplete = get_incomplete();
    if incomplete.is_empty() {
        return Vec::new();
    }

    log::warn!(
        "deploy_journal: found {} incomplete entries — triggering self-heal",
        incomplete.len()
    );

    // Deduplicate by (game_id, bottle_name) — one redeploy per game is enough
    let mut pairs: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    for entry in &incomplete {
        pairs.insert((entry.game_id.clone(), entry.bottle_name.clone()));
    }

    let mut healed = Vec::new();
    for (game_id, bottle_name) in &pairs {
        match heal_deployment(db, game_id, bottle_name) {
            Ok(()) => {
                log::info!(
                    "deploy_journal: healed deployment for {}/{}",
                    game_id,
                    bottle_name
                );
                healed.push((game_id.clone(), bottle_name.clone()));
            }
            Err(e) => {
                log::error!(
                    "deploy_journal: failed to heal {}/{}: {}",
                    game_id,
                    bottle_name,
                    e
                );
            }
        }
    }

    // Mark all incomplete entries as complete after healing
    let mut entries = read_journal();
    for entry in entries.iter_mut() {
        if entry.status == JournalStatus::Pending {
            entry.status = JournalStatus::Complete;
        }
    }
    let _ = write_journal(&entries);

    healed
}

/// Perform a full redeploy to heal a potentially inconsistent deployment.
fn heal_deployment(
    db: &std::sync::Arc<crate::database::ModDatabase>,
    game_id: &str,
    bottle_name: &str,
) -> Result<(), String> {
    let bottle = crate::bottles::find_bottle_by_name(bottle_name)
        .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))?;
    let game = crate::games::detect_games(&bottle)
        .into_iter()
        .find(|g| g.game_id == game_id)
        .ok_or_else(|| format!("Game '{}' not found in bottle '{}'", game_id, bottle_name))?;
    let data_dir = std::path::PathBuf::from(&game.data_dir);

    crate::deployer::redeploy_all(db, game_id, bottle_name, &data_dir, &game.game_path)
        .map_err(|e| format!("redeploy failed: {e}"))?;

    Ok(())
}

/// Remove completed entries older than 24 hours.
fn prune_old_entries(entries: &mut Vec<JournalEntry>) {
    let cutoff = chrono::Utc::now() - chrono::TimeDelta::hours(24);
    let before = entries.len();
    entries.retain(|e| {
        if e.status == JournalStatus::Complete {
            if let Ok(ts) = chrono::DateTime::parse_from_rfc3339(&e.timestamp) {
                return ts > cutoff;
            }
        }
        true
    });
    if entries.len() < before {
        let _ = write_journal(entries);
    }
}

/// Generate a short pseudo-random suffix for journal IDs.
fn uuid_short() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    std::time::SystemTime::now().hash(&mut hasher);
    std::thread::current().id().hash(&mut hasher);
    format!("{:08x}", hasher.finish() as u32)
}
