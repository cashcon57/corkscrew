//! Vortex extension registry — orchestrates fetch, execute, cache, and register.
//!
//! This module ties together the fetcher (GitHub download), runtime (QuickJS
//! execution), and plugin (GamePlugin impl) layers. It provides:
//!
//! - SQLite-backed cache of extracted extension data (avoids re-executing JS)
//! - On-demand fetching: extensions are downloaded per-game when first needed
//! - Dynamic plugin registration: creates `VortexGamePlugin` instances from
//!   cached or freshly-extracted data and registers them with the game registry

use std::sync::Arc;

use rusqlite::params;

use crate::database::ModDatabase;
use crate::games::register_plugin;
use crate::vortex_fetcher;
use crate::vortex_plugin::VortexGamePlugin;
use crate::vortex_runtime;
use crate::vortex_types::*;

// ---------------------------------------------------------------------------
// SQLite schema (called from migrations)
// ---------------------------------------------------------------------------

/// Create the vortex_extensions table. Called from the migration system.
pub fn create_table(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS vortex_extensions (
            game_id        TEXT PRIMARY KEY,
            name           TEXT NOT NULL,
            executable     TEXT NOT NULL DEFAULT '',
            mod_path       TEXT NOT NULL DEFAULT '.',
            merge_mods     INTEGER NOT NULL DEFAULT 1,
            required_files TEXT NOT NULL DEFAULT '[]',
            store_ids      TEXT NOT NULL DEFAULT '{}',
            details        TEXT NOT NULL DEFAULT '{}',
            environment    TEXT NOT NULL DEFAULT '{}',
            tools          TEXT NOT NULL DEFAULT '[]',
            mod_types      TEXT NOT NULL DEFAULT '[]',
            installers     TEXT NOT NULL DEFAULT '[]',
            is_stub        INTEGER NOT NULL DEFAULT 0,
            steam_dir_name TEXT,
            source_hash    TEXT NOT NULL DEFAULT '',
            fetched_at     TEXT NOT NULL DEFAULT (datetime('now')),
            raw_json       TEXT NOT NULL DEFAULT '{}'
        );",
    )
}

// ---------------------------------------------------------------------------
// Cache read/write
// ---------------------------------------------------------------------------

/// Load a cached registration from the database.
pub fn load_cached(db: &ModDatabase, game_id: &str) -> Option<VortexGameRegistration> {
    let conn = db.conn().ok()?;
    let mut stmt = conn
        .prepare("SELECT raw_json FROM vortex_extensions WHERE game_id = ?1")
        .ok()?;
    let json: String = stmt.query_row(params![game_id], |row| row.get(0)).ok()?;
    serde_json::from_str(&json).ok()
}

/// Save a registration to the database cache.
pub fn save_cached(db: &ModDatabase, reg: &VortexGameRegistration, source_hash: &str) {
    let raw_json = serde_json::to_string(reg).unwrap_or_default();
    let required_files = serde_json::to_string(&reg.required_files).unwrap_or_default();
    let store_ids = serde_json::to_string(&reg.store_ids).unwrap_or_default();
    let details = serde_json::to_string(&reg.details).unwrap_or_default();
    let environment = serde_json::to_string(&reg.environment).unwrap_or_default();
    let tools = serde_json::to_string(&reg.supported_tools).unwrap_or_default();
    let mod_types = serde_json::to_string(&reg.mod_types).unwrap_or_default();
    let installers = serde_json::to_string(&reg.installers).unwrap_or_default();

    let Ok(conn) = db.conn() else { return };
    let _ = conn.execute(
        "INSERT OR REPLACE INTO vortex_extensions
            (game_id, name, executable, mod_path, merge_mods, required_files,
             store_ids, details, environment, tools, mod_types, installers,
             is_stub, steam_dir_name, source_hash, fetched_at, raw_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, datetime('now'), ?16)",
        params![
            reg.id,
            reg.name,
            reg.executable,
            reg.query_mod_path,
            reg.merge_mods as i32,
            required_files,
            store_ids,
            details,
            environment,
            tools,
            mod_types,
            installers,
            reg.is_stub as i32,
            reg.steam_dir_name,
            source_hash,
            raw_json,
        ],
    );
}

/// Check if we have a cached extension and its source hash matches.
pub fn is_cache_fresh(db: &ModDatabase, game_id: &str, source_hash: &str) -> bool {
    let Ok(conn) = db.conn() else { return false };
    let mut stmt = match conn
        .prepare("SELECT source_hash FROM vortex_extensions WHERE game_id = ?1")
    {
        Ok(s) => s,
        Err(_) => return false,
    };
    stmt.query_row(params![game_id], |row| row.get::<_, String>(0))
        .map(|cached| cached == source_hash)
        .unwrap_or(false)
}

/// List all cached extension summaries.
pub fn list_cached(db: &ModDatabase) -> Vec<ExtensionSummary> {
    let Ok(conn) = db.conn() else { return Vec::new() };
    let mut stmt = match conn.prepare(
        "SELECT game_id, name, is_stub, fetched_at, tools, mod_types FROM vortex_extensions ORDER BY name",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let rows = stmt
        .query_map([], |row| {
            let tools_json: String = row.get(4)?;
            let mod_types_json: String = row.get(5)?;
            let tools: Vec<serde_json::Value> =
                serde_json::from_str(&tools_json).unwrap_or_default();
            let mod_types: Vec<serde_json::Value> =
                serde_json::from_str(&mod_types_json).unwrap_or_default();
            Ok(ExtensionSummary {
                game_id: row.get(0)?,
                name: row.get(1)?,
                version: None,
                is_stub: row.get::<_, i32>(2)? != 0,
                fetched_at: row.get(3)?,
                tool_count: tools.len(),
                mod_type_count: mod_types.len(),
            })
        })
        .ok();

    match rows {
        Some(iter) => iter.filter_map(|r| r.ok()).collect(),
        None => Vec::new(),
    }
}

/// Delete a cached extension.
pub fn delete_cached(db: &ModDatabase, game_id: &str) {
    let Ok(conn) = db.conn() else { return };
    let _ = conn.execute(
        "DELETE FROM vortex_extensions WHERE game_id = ?1",
        params![game_id],
    );
}

// ---------------------------------------------------------------------------
// Orchestration: fetch → execute → cache → register
// ---------------------------------------------------------------------------

/// Fetch a Vortex extension from GitHub, execute it, cache the result,
/// and register the resulting GamePlugin.
///
/// If the extension is already cached with the same source hash, skips
/// re-execution and uses the cached data.
pub async fn fetch_and_register(
    db: &Arc<ModDatabase>,
    game_id: &str,
) -> Result<VortexGameRegistration, String> {
    // Validate game_id before using it in paths or URLs
    crate::vortex_fetcher::validate_game_id(game_id)?;
    log::info!("Fetching Vortex extension for game: {}", game_id);

    // 1. Fetch source from GitHub (async network I/O)
    let source = vortex_fetcher::fetch_extension(game_id).await?;

    // 2-6. DB checks, JS execution, and caching are all blocking —
    // run them off the async executor to avoid stalling other tasks.
    let db_clone = Arc::clone(db);
    let gid = game_id.to_string();
    tokio::task::spawn_blocking(move || {
        // 2. Check if cache is fresh
        if is_cache_fresh(&db_clone, &gid, &source.source_hash) {
            if let Some(cached) = load_cached(&db_clone, &gid) {
                log::info!("Using cached extension for {} (hash match)", gid);
                register_vortex_plugin(cached.clone());
                return Ok(cached);
            }
        }

        // 3. Execute in QuickJS sandbox (CPU-bound)
        log::info!("Executing extension JS for {}", gid);
        let mut captured = vortex_runtime::execute_extension(&source)?;

        // 4. Merge mod types and installers into the game registration
        let reg = if let Some(mut game) = captured.game.take() {
            game.mod_types = captured.mod_types;
            game.installers = captured.installers;
            game
        } else {
            return Err(format!(
                "Extension for '{}' did not call registerGame()",
                gid
            ));
        };

        // 5. Cache to SQLite
        save_cached(&db_clone, &reg, &source.source_hash);

        // 6. Register as a GamePlugin
        register_vortex_plugin(reg.clone());

        log::info!(
            "Registered Vortex game: {} ({}) — {} tools, {} mod types",
            reg.name,
            reg.id,
            reg.supported_tools.len(),
            reg.mod_types.len(),
        );

        Ok(reg)
    })
    .await
    .map_err(|e| format!("Task panicked: {e}"))?
}

/// Register a game from cached data only (no network).
pub fn register_from_cache(db: &ModDatabase, game_id: &str) -> Option<VortexGameRegistration> {
    let reg = load_cached(db, game_id)?;
    register_vortex_plugin(reg.clone());
    Some(reg)
}

/// Register all cached extensions as GamePlugins.
///
/// Called at startup to restore previously fetched game support.
pub fn register_all_cached(db: &ModDatabase) {
    let summaries = list_cached(db);
    let mut count = 0;
    for summary in &summaries {
        if summary.is_stub {
            continue;
        }
        if let Some(reg) = load_cached(db, &summary.game_id) {
            register_vortex_plugin(reg);
            count += 1;
        }
    }
    if count > 0 {
        log::info!("Registered {} Vortex game plugins from cache", count);
    }
}

/// Refresh an existing extension (re-fetch + re-execute).
pub async fn refresh_extension(
    db: &Arc<ModDatabase>,
    game_id: &str,
) -> Result<VortexGameRegistration, String> {
    delete_cached(db, game_id);
    fetch_and_register(db, game_id).await
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn register_vortex_plugin(reg: VortexGameRegistration) {
    let plugin = VortexGamePlugin::new(reg);
    register_plugin(Box::new(plugin));
}
