pub mod archive_preview;
pub mod background_hash;
pub mod baselines;
pub mod bg3_lsx;
pub mod bg3_pak;
pub mod bottle_config;
pub mod bottles;
pub mod cleaner;
pub mod collection_installer;
pub mod collections;
pub mod config;
pub mod conflict_resolver;
pub mod crashlog;
pub mod crossover_shortcuts;
pub mod cursor_clamp;
pub mod database;
pub mod deck;
pub mod deploy_journal;
pub mod deployer;
pub mod depot_downloader;
pub mod disk_budget;
pub mod display_fix;
pub mod downgrader;
pub mod download_queue;
pub mod dxvk;
pub mod esp_analyzer;
pub mod executables;
pub mod fomod;
pub mod fomod_recipes;
pub mod fromsoft_saves;
pub mod game_lock;
pub mod game_registry;
pub mod games;
pub mod graphics_backend;
pub mod google_oauth;
pub mod gpu_encoder;
pub mod ini_manager;
pub mod installer;
pub mod instruction_parser;
pub mod instruction_types;
pub mod instruction_validator;
pub mod integrity;
pub mod launch_fixes;
pub mod launcher;
pub mod llm_chat;
pub mod llm_parser;
pub mod loot;
pub mod loot_rules;
pub mod migrations;
pub mod mod_dependencies;
pub mod mod_recommendations;
pub mod mod_tools;
pub mod mod_types;
pub mod modengine2_config;
pub mod modlist_io;
pub mod nexus;
pub mod nexus_games_index;
pub mod nexus_sso;
pub mod bg3se;
pub mod native_scanner;
pub mod paralives_bepinex;
pub mod nxm_handler;
pub mod oauth;
pub mod platform;
pub mod plist;
pub mod plugins;
pub mod preflight;
pub mod prefix_setup;
pub mod profile_sharing;
pub mod profiles;
pub mod progress;
pub mod proton;
pub mod regulation_conflicts;
pub mod rollback;
pub mod runtime;
pub mod session_tracker;
pub mod skse;
pub mod smapi;
pub mod staging;
pub mod steam_integration;
pub mod umu;
pub mod vortex_fetcher;
pub mod vortex_index;
pub mod vortex_plugin;
pub mod vortex_registry;
pub mod vortex_runtime;
pub mod vortex_types;
pub mod wabbajack;
pub mod wabbajack_directives;
pub mod wabbajack_downloader;
pub mod wabbajack_installer;
pub mod wabbajack_types;
pub mod wine_compat;
pub mod wine_dll_overrides;
pub mod tool_automation;
pub mod app_updates;
pub mod self_update;
pub mod shader_conversion;
pub mod texture_optimizer;
pub mod thunderstore;
pub mod verified_lists;
pub mod wine_diagnostic;
pub mod commands;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

use lru::LruCache;
use tauri::{AppHandle, Emitter, Manager};

use bottles::Bottle;
use database::ModDatabase;
use fomod::FomodInstaller;
use games::DetectedGame;
use plugins::skyrim_plugins::PluginEntry;

pub struct AppState {
    db: Arc<ModDatabase>,
    download_queue: Arc<download_queue::DownloadQueue>,
    wj_cancel_tokens:
        std::sync::Mutex<std::collections::HashMap<i64, Arc<std::sync::atomic::AtomicBool>>>,
    /// LRU cache for parsed FOMOD installers, keyed by archive SHA-256 hash.
    fomod_cache: Arc<RwLock<LruCache<String, FomodInstaller>>>,
    /// Chat session for local LLM interaction.
    chat_session: llm_chat::SharedChatSession,
    /// Session-level flag: once we verify the LOOT masterlist is fresh for the
    /// current game, we skip further freshness checks until the game changes
    /// or the user force-refreshes.
    loot_masterlist_checked: Arc<AtomicBool>,
    /// Tracks running game processes per (game_id, bottle_name).
    game_locks: Arc<game_lock::GameLockManager>,
    /// True while a deployment operation (redeploy, switch collection) is in progress.
    deploy_in_progress: Arc<AtomicBool>,
}

/// RAII guard that sets an `AtomicBool` flag to `true` on creation and back to
/// `false` on drop, ensuring the flag is always cleared even if the enclosing
/// scope exits via panic or early return.
pub struct DeployGuard(Arc<AtomicBool>, AppHandle);

impl DeployGuard {
    pub(crate) fn new(flag: Arc<AtomicBool>, app_handle: AppHandle) -> Self {
        flag.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = app_handle.emit("deploy-status-changed", true);
        Self(flag, app_handle)
    }
}

impl Drop for DeployGuard {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Relaxed);
        let _ = self.1.emit("deploy-status-changed", false);
    }
}

/// Resolve a bottle by name, returning a useful error if not found.
pub(crate) fn resolve_bottle(bottle_name: &str) -> Result<Bottle, String> {
    bottles::find_bottle_by_name(bottle_name)
        .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))
}

/// Resolve a bottle + game pair, returning both plus the data directory.
///
/// Wine games only — callers that support both runtimes should use
/// [`resolve_game_any_runtime`] instead, which handles native games without
/// requiring a bottle.
pub(crate) fn resolve_game(
    game_id: &str,
    bottle_name: &str,
) -> Result<(Bottle, DetectedGame, PathBuf), String> {
    let bottle = resolve_bottle(bottle_name)?;
    let detected_games = games::detect_games(&bottle);
    let game = detected_games
        .into_iter()
        .find(|g| g.game_id == game_id)
        .ok_or_else(|| format!("Game '{}' not found in bottle '{}'", game_id, bottle_name))?;
    let data_dir = PathBuf::from(&game.data_dir);
    Ok((bottle, game, data_dir))
}

/// Resolve a game for either Wine or Native runtime.
///
/// When `bottle_name` is non-empty the call delegates to [`resolve_game`] and
/// returns `(Some(bottle), game, data_dir)`. When `bottle_name` is empty (the
/// native sentinel) the full game list is scanned for a native game whose
/// `game_id` matches; bottle is returned as `None`.
///
/// Returns `Err` if the game cannot be found in either path.
pub(crate) fn resolve_game_any_runtime(
    game_id: &str,
    bottle_name: &str,
) -> Result<(Option<Bottle>, DetectedGame, PathBuf), String> {
    if !bottle_name.is_empty() {
        // Wine path — preserve existing behaviour exactly.
        let (bottle, game, data_dir) = resolve_game(game_id, bottle_name)?;
        return Ok((Some(bottle), game, data_dir));
    }

    // Native path — scan all detected games (bottles + native) for a match.
    // We do not call detect_all_games_with_custom here because that takes a
    // DB reference; detect_all_games() covers every registered native plugin
    // and is sufficient for install routing.
    let all_games = games::detect_all_games();
    let game = all_games
        .into_iter()
        .find(|g| g.game_id == game_id && g.runtime.is_native())
        .ok_or_else(|| {
            format!(
                "Native game '{}' not found. Make sure the game is detected in Native Mode.",
                game_id
            )
        })?;
    let data_dir = PathBuf::from(&game.data_dir);
    Ok((None, game, data_dir))
}

/// Format a `tokio::task::JoinError` with panic details when available.
/// Replaces the generic "Task failed: JoinError" with the actual panic message.
pub(crate) fn format_join_error(e: tokio::task::JoinError) -> String {
    if e.is_panic() {
        let panic = e.into_panic();
        if let Some(s) = panic.downcast_ref::<&str>() {
            format!("Task panicked: {s}")
        } else if let Some(s) = panic.downcast_ref::<String>() {
            format!("Task panicked: {s}")
        } else {
            "Task panicked (unknown payload)".to_string()
        }
    } else {
        format!("Task cancelled: {e}")
    }
}

/// Create an auto-snapshot before a destructive operation.
/// Silent on failure — logs a warning but never blocks the operation.
pub(crate) fn auto_snapshot_before_destructive(
    db: &ModDatabase,
    game_id: &str,
    bottle_name: &str,
    label: &str,
) {
    match rollback::create_snapshot(
        db,
        game_id,
        bottle_name,
        label,
        Some("Auto-snapshot before destructive operation"),
    ) {
        Ok(id) => log::info!("Auto-snapshot {} created: {}", id, label),
        Err(e) => log::warn!("Failed to create auto-snapshot '{}': {}", label, e),
    }
}

/// Check game lock for a specific game/bottle and return an error if locked.
pub(crate) fn check_game_lock(
    locks: &game_lock::GameLockManager,
    game_id: &str,
    bottle_name: &str,
) -> Result<(), String> {
    if let Some(lock) = locks.get(game_id, bottle_name) {
        return Err(format!(
            "GAME_LOCKED: Cannot modify mods while {} is running (pid {}). \
             Close the game first or use 'Unlock anyway' to override.",
            game_id, lock.pid
        ));
    }
    Ok(())
}

/// Create a NexusClient from the current auth method (OAuth or API key),
/// auto-refreshing expired OAuth tokens as needed.
pub(crate) async fn nexus_client() -> Result<nexus::NexusClient, String> {
    let method = oauth::get_auth_method_refreshed().await;
    nexus::NexusClient::from_auth_method(&method).map_err(|e| e.to_string())
}

/// Get the current API key string for functions that need a raw key
/// (e.g. GraphQL helpers). Prefers OAuth Bearer token, falls back to API key.
pub(crate) async fn nexus_api_key_or_token() -> Result<(String, bool), String> {
    let method = oauth::get_auth_method_refreshed().await;
    match method {
        oauth::AuthMethod::OAuth(tokens) => Ok((tokens.access_token, true)),
        oauth::AuthMethod::ApiKey(key) => Ok((key, false)),
        oauth::AuthMethod::None => {
            Err("No NexusMods authentication configured. Sign in via Settings.".to_string())
        }
    }
}


// --- Helpers ---

pub(crate) fn get_current_plugins(game_id: &str, bottle_name: &str) -> Vec<PluginEntry> {
    if game_id != "skyrimse" {
        return Vec::new();
    }

    let (bottle, game, _) = match resolve_game(game_id, bottle_name) {
        Ok(result) => result,
        Err(_) => return Vec::new(),
    };

    let plugins_file = games::with_plugin(game_id, |plugin| {
        plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
    })
    .flatten();

    match plugins_file {
        Some(pf) if pf.exists() => {
            plugins::skyrim_plugins::read_plugins_txt(&pf).unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

pub(crate) fn sync_plugins_for_game(game: &DetectedGame, bottle: &Bottle) -> Result<(), String> {
    // Only sync for games that support Bethesda-style plugin load order
    if !plugins::skyrim_plugins::supports_plugin_order(&game.game_id) {
        return Ok(());
    }

    let game_path = Path::new(&game.game_path);
    let data_dir = Path::new(&game.data_dir);

    let plugins_file = games::with_plugin(&game.game_id, |plugin| {
        plugin.get_plugins_file(game_path, bottle)
    })
    .flatten();

    if let Some(pf) = plugins_file {
        let loadorder_file = pf
            .parent()
            .map(|p| p.join("loadorder.txt"))
            .unwrap_or_else(|| pf.with_file_name("loadorder.txt"));
        let implicit = plugins::skyrim_plugins::implicit_plugins_for_game(&game.game_id);
        plugins::skyrim_plugins::sync_plugins(data_dir, &pf, &loadorder_file, implicit)
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

// --- App Entry Point ---

fn kill_mlx_server() {
    let _ = std::process::Command::new("pkill")
        .args(["-f", "mlx_lm.server"])
        .output();
    log::info!("Killed MLX LM server if running");
}

/// Run startup health checks after self-healing.
/// Logs results and emits an event if issues are found.
fn run_startup_health_check(
    db: &Arc<database::ModDatabase>,
    app_handle: &tauri::AppHandle,
) {
    let start = std::time::Instant::now();
    let mut issues: Vec<String> = Vec::new();

    // 1. DB integrity check
    match db.execute_pragma_integrity_check() {
        Ok(ref result) if result == "ok" => {
            log::info!("Startup health: DB integrity OK");
        }
        Ok(result) => {
            log::error!("Startup health: DB integrity issue: {}", result);
            issues.push(format!("Database integrity: {}", result));
        }
        Err(e) => {
            log::error!("Startup health: DB integrity check failed: {}", e);
            issues.push(format!("Database integrity check error: {}", e));
        }
    }

    // 2. Staging dir existence
    let cfg = config::get_config().unwrap_or_default();
    let staging_base = cfg
        .staging_dir
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| config::data_dir().join("staging"));
    if !staging_base.exists() {
        log::warn!(
            "Startup health: staging dir missing, creating: {}",
            staging_base.display()
        );
        let _ = std::fs::create_dir_all(&staging_base);
        issues.push("Staging directory was missing (recreated)".to_string());
    }

    // 3. Stale deploy journal entries
    let stale = deploy_journal::get_incomplete().len();
    if stale > 0 {
        log::warn!(
            "Startup health: {} incomplete deploy journal entries",
            stale
        );
        issues.push(format!(
            "{} incomplete deployment journal entries found",
            stale
        ));
    }

    let elapsed = start.elapsed();
    log::info!(
        "Startup health check completed in {}ms ({} issues)",
        elapsed.as_millis(),
        issues.len()
    );

    // Record issues as error events for diagnostics
    for issue in &issues {
        let _ = db.record_error_event("startup_health", "issue", issue);
    }

    if !issues.is_empty() {
        let _ = app_handle.emit(
            "startup-health-issues",
            serde_json::json!({ "issues": issues }),
        );
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Fix WebKitGTK EGL/DMABuf crash on SteamOS/Gamescope and some Wayland compositors.
    //
    // WebKitGTK's DMABuf renderer fails with "Could not create default EGL display:
    // EGL_BAD_PARAMETER" under Gamescope (Steam Deck) and some Wayland sessions.
    // WEBKIT_DISABLE_DMABUF_RENDERER=1 falls back to shared-memory rendering, which
    // works everywhere. Performance impact is negligible for a UI app.
    //
    // This is the same workaround used by Heroic Games Launcher, Lutris, and most
    // Electron/WebKitGTK apps shipping on Steam Deck.
    //
    // We apply it on ALL Linux to avoid the crash, since:
    // - The SHM fallback is fast enough for a mod manager UI
    // - Users who want DMABuf can override: WEBKIT_DISABLE_DMABUF_RENDERER=0 corkscrew
    #[cfg(target_os = "linux")]
    {
        if std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER").is_err() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }

    // Kill any leftover MLX server from a previous crash/dev restart
    kill_mlx_server();
    // Register game plugins (dedicated plugins first, then registry)
    plugins::skyrim_se::register();
    plugins::fallout4::register();
    plugins::hogwarts_legacy::register();
    plugins::hades2::register();
    plugins::crimson_desert::register();
    plugins::crimson_desert_native::register();
    plugins::sims4::register();
    plugins::gtav::register();
    plugins::genshin::register();
    plugins::paralives_native::register();
    plugins::stardew_valley_native::register();
    plugins::baldurs_gate_3_native::register();
    plugins::thunderstore_games::register_all();
    plugins::fromsoft::register_all();
    game_registry::register_all();

    // Initialize database
    let db_path = config::db_path();
    let db = Arc::new(ModDatabase::new(&db_path).expect("Failed to initialize mod database"));

    // Register any previously-cached Vortex extensions as game plugins
    vortex_registry::register_all_cached(&db);

    // Initialize additional schemas
    executables::init_schema(&db).expect("Failed to initialize executables schema");
    profiles::init_schema(&db).expect("Failed to initialize profiles schema");
    integrity::init_schema(&db).expect("Failed to initialize integrity schema");
    loot_rules::init_schema(&db).expect("Failed to initialize loot rules schema");
    rollback::init_schema(&db).expect("Failed to initialize rollback schema");

    // Set up logging: write to both stderr and a log file for GUI debugging.
    let log_path = config::data_dir().join("corkscrew.log");

    // Rotate logs: keep 3 files at 5 MB each (corkscrew.log → .1.log → .2.log)
    if let Ok(meta) = std::fs::metadata(&log_path) {
        if meta.len() > 5 * 1024 * 1024 {
            let dir = log_path.parent().unwrap();
            let log2 = dir.join("corkscrew.2.log");
            let log1 = dir.join("corkscrew.1.log");
            let _ = std::fs::remove_file(&log2);
            let _ = std::fs::rename(&log1, &log2);
            let _ = std::fs::rename(&log_path, &log1);
        }
    }

    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path);

    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    builder
        .filter_module("tao", log::LevelFilter::Warn)
        .filter_module("wry", log::LevelFilter::Warn);

    if let Ok(file) = log_file {
        let file = std::sync::Mutex::new(file);
        builder
            .format(move |buf, record| {
                use std::io::Write;
                let ts = buf.timestamp_seconds();
                let line = format!(
                    "{} [{}] {}: {}\n",
                    ts,
                    record.level(),
                    record.target(),
                    record.args()
                );
                // Write to stderr (normal env_logger behavior)
                let _ = write!(buf, "{}", line);
                // Also write to log file
                if let Ok(mut f) = file.lock() {
                    let _ = std::io::Write::write_all(&mut *f, line.as_bytes());
                    let _ = std::io::Write::flush(&mut *f);
                }
                Ok(())
            })
            .init();
        log::info!("Logging to file: {}", log_path.display());
    } else {
        builder.init();
    }

    // Initialize Sentry if user has opted in to telemetry
    const SENTRY_DSN: &str = "https://de71b88287dbb157e219aff7e1ba2d9c@o4511134300045312.ingest.us.sentry.io/4511134367940608";
    let _sentry_guard = {
        let consent = config::get_config()
            .ok()
            .and_then(|c| c.extra.get("telemetry_consent").and_then(|v| v.as_str().map(String::from)));
        if consent.as_deref() == Some("granted") {
            log::info!("Telemetry enabled — initializing Sentry");
            Some(sentry::init((
                SENTRY_DSN,
                sentry::ClientOptions {
                    release: Some(std::borrow::Cow::Owned(
                        env!("CARGO_PKG_VERSION").to_string(),
                    )),
                    sample_rate: 1.0,
                    attach_stacktrace: true,
                    send_default_pii: false,
                    ..Default::default()
                },
            )))
        } else {
            None
        }
    };

    // Global panic hook — log panics to file, record in error_events, and forward to Sentry
    {
        let panic_db = Arc::clone(&db);
        // Take the current hook (which includes Sentry's if it was initialized)
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let location = info
                .location()
                .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
                .unwrap_or_else(|| "unknown".to_string());
            let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = info.payload().downcast_ref::<String>() {
                s.clone()
            } else {
                "Unknown panic".to_string()
            };
            log::error!("PANIC at {}: {}", location, payload);
            let _ = panic_db.record_error_event("rust_panic", &location, &payload);
            // Forward to previous hook (Sentry's panic handler if active)
            prev_hook(info);
        }));
    }

    // --- CLI mode ---
    // Subcommands dispatched here. Each exits after completion.
    {
        let args: Vec<String> = std::env::args().collect();

        // --launch <game_id> <bottle_name> [--skse]
        if let Some(pos) = args.iter().position(|a| a == "--launch") {
            let game_id = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            let bottle_name = args.get(pos + 2).map(|s| s.as_str()).unwrap_or("");
            let use_skse = args.iter().any(|a| a == "--skse");
            if game_id.is_empty() || bottle_name.is_empty() {
                eprintln!("Usage: corkscrew --launch <game_id> <bottle_name> [--skse]");
                eprintln!("  Example: corkscrew --launch skyrimse Steam --skse");
                std::process::exit(1);
            }
            commands::cli::cli_launch(game_id, bottle_name, use_skse, &db);
            return;
        }

        // --list-mods <game_id> <bottle_name>
        if let Some(pos) = args.iter().position(|a| a == "--list-mods") {
            let game_id = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            let bottle_name = args.get(pos + 2).map(|s| s.as_str()).unwrap_or("");
            if game_id.is_empty() || bottle_name.is_empty() {
                eprintln!("Usage: corkscrew --list-mods <game_id> <bottle_name>");
                std::process::exit(1);
            }
            commands::cli::cli_list_mods(game_id, bottle_name, &db);
            return;
        }

        // --search-mods <query> <game_id> <bottle_name>
        if let Some(pos) = args.iter().position(|a| a == "--search-mods") {
            let query = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            let game_id = args.get(pos + 2).map(|s| s.as_str()).unwrap_or("");
            let bottle_name = args.get(pos + 3).map(|s| s.as_str()).unwrap_or("");
            if query.is_empty() || game_id.is_empty() || bottle_name.is_empty() {
                eprintln!("Usage: corkscrew --search-mods <query> <game_id> <bottle_name>");
                std::process::exit(1);
            }
            commands::cli::cli_search_mods(query, game_id, bottle_name, &db);
            return;
        }

        // --find-file <pattern> <game_id> <bottle_name>
        if let Some(pos) = args.iter().position(|a| a == "--find-file") {
            let pattern = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            let game_id = args.get(pos + 2).map(|s| s.as_str()).unwrap_or("");
            let bottle_name = args.get(pos + 3).map(|s| s.as_str()).unwrap_or("");
            if pattern.is_empty() || game_id.is_empty() || bottle_name.is_empty() {
                eprintln!("Usage: corkscrew --find-file <pattern> <game_id> <bottle_name>");
                std::process::exit(1);
            }
            commands::cli::cli_find_file(pattern, game_id, bottle_name, &db);
            return;
        }

        // --check-plugins <game_id> <bottle_name> [--inactive-only] [--deployed-inactive]
        if let Some(pos) = args.iter().position(|a| a == "--check-plugins") {
            let game_id = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            let bottle_name = args.get(pos + 2).map(|s| s.as_str()).unwrap_or("");
            if game_id.is_empty() || bottle_name.is_empty() {
                eprintln!("Usage: corkscrew --check-plugins <game_id> <bottle_name> [--inactive-only] [--deployed-inactive]");
                std::process::exit(1);
            }
            let inactive_only = args.iter().any(|a| a == "--inactive-only");
            let deployed_inactive = args.iter().any(|a| a == "--deployed-inactive");
            commands::cli::cli_check_plugins(game_id, bottle_name, inactive_only, deployed_inactive, &db);
            return;
        }

        // --sync-plugins <game_id> <bottle_name>
        if let Some(pos) = args.iter().position(|a| a == "--sync-plugins") {
            let game_id = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            let bottle_name = args.get(pos + 2).map(|s| s.as_str()).unwrap_or("");
            if game_id.is_empty() || bottle_name.is_empty() {
                eprintln!("Usage: corkscrew --sync-plugins <game_id> <bottle_name>");
                std::process::exit(1);
            }
            commands::cli::cli_sync_plugins(game_id, bottle_name);
            return;
        }

        // --mod-files <mod_id_or_name> <game_id> <bottle_name>
        if let Some(pos) = args.iter().position(|a| a == "--mod-files") {
            let search = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            let game_id = args.get(pos + 2).map(|s| s.as_str()).unwrap_or("");
            let bottle_name = args.get(pos + 3).map(|s| s.as_str()).unwrap_or("");
            if search.is_empty() || game_id.is_empty() || bottle_name.is_empty() {
                eprintln!("Usage: corkscrew --mod-files <mod_id_or_name> <game_id> <bottle_name>");
                std::process::exit(1);
            }
            commands::cli::cli_mod_files(search, game_id, bottle_name, &db);
            return;
        }

        // --list-bottles  (JSON array of detected bottles)
        if args.iter().any(|a| a == "--list-bottles") {
            commands::cli::cli_list_bottles();
            return;
        }

        // --scan-bottle <bottle_name>  (diagnostic: dump everything Corkscrew
        // can detect in a bottle — Steam appmanifest games, CrossOver
        // shortcuts, FromSoft Mod Engine 2 install, plugin-detected games.
        // Pure read-only; safe to run anywhere. Designed for paste-into-Discord
        // troubleshooting.)
        if let Some(pos) = args.iter().position(|a| a == "--scan-bottle") {
            let bottle_name = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            if bottle_name.is_empty() {
                eprintln!("Usage: corkscrew --scan-bottle <bottle_name>");
                std::process::exit(1);
            }
            commands::cli::cli_scan_bottle(bottle_name);
            return;
        }

        // --list-games  (JSON array of detected games across all bottles)
        if args.iter().any(|a| a == "--list-games") {
            commands::cli::cli_list_games(&db);
            return;
        }

        // --db-stats <game_id> <bottle_name>  (JSON object with database statistics)
        if let Some(pos) = args.iter().position(|a| a == "--db-stats") {
            let game_id = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            let bottle_name = args.get(pos + 2).map(|s| s.as_str()).unwrap_or("");
            if game_id.is_empty() || bottle_name.is_empty() {
                eprintln!("Usage: corkscrew --db-stats <game_id> <bottle_name>");
                std::process::exit(1);
            }
            commands::cli::cli_db_stats(game_id, bottle_name, &db);
            return;
        }

        // --db-integrity  (run SQLite integrity check + schema validation)
        if args.iter().any(|a| a == "--db-integrity") {
            commands::cli::cli_db_integrity(&db);
            return;
        }

        // --vortex-list  (JSON array of cached vortex extensions)
        if args.iter().any(|a| a == "--vortex-list") {
            commands::cli::cli_vortex_list(&db);
            return;
        }

        // --deployment-health <game_id> <bottle_name>  (check deployment integrity)
        if let Some(pos) = args.iter().position(|a| a == "--deployment-health") {
            let game_id = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            let bottle_name = args.get(pos + 2).map(|s| s.as_str()).unwrap_or("");
            if game_id.is_empty() || bottle_name.is_empty() {
                eprintln!("Usage: corkscrew --deployment-health <game_id> <bottle_name>");
                std::process::exit(1);
            }
            commands::cli::cli_deployment_health(game_id, bottle_name, &db);
            return;
        }

        // --list-profiles <game_id> <bottle_name>  (JSON array of profiles)
        if let Some(pos) = args.iter().position(|a| a == "--list-profiles") {
            let game_id = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            let bottle_name = args.get(pos + 2).map(|s| s.as_str()).unwrap_or("");
            if game_id.is_empty() || bottle_name.is_empty() {
                eprintln!("Usage: corkscrew --list-profiles <game_id> <bottle_name>");
                std::process::exit(1);
            }
            commands::cli::cli_list_profiles(game_id, bottle_name, &db);
            return;
        }

        // --add-game <game_id> <name> <bottle> <path> [--exe <name>] [--mod-dir <dir>] [--nexus <slug>] [--steam-id <id>]
        if let Some(pos) = args.iter().position(|a| a == "--add-game") {
            let game_id = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            let name = args.get(pos + 2).map(|s| s.as_str()).unwrap_or("");
            let bottle = args.get(pos + 3).map(|s| s.as_str()).unwrap_or("");
            let path = args.get(pos + 4).map(|s| s.as_str()).unwrap_or("");
            if game_id.is_empty() || name.is_empty() || bottle.is_empty() || path.is_empty() {
                eprintln!("Usage: corkscrew --add-game <game_id> <name> <bottle> <path>");
                eprintln!("  Options: --exe <name> --mod-dir <dir> --nexus <slug> --steam-id <id>");
                eprintln!("  Example: corkscrew --add-game re-requiem \"Resident Evil Requiem\" Steam \"Program Files (x86)/Steam/steamapps/common/RESIDENT EVIL requiem BIOHAZARD requiem\" --exe re9.exe");
                std::process::exit(1);
            }
            let exe = args
                .iter()
                .position(|a| a == "--exe")
                .and_then(|i| args.get(i + 1))
                .map(|s| s.as_str());
            let mod_dir = args
                .iter()
                .position(|a| a == "--mod-dir")
                .and_then(|i| args.get(i + 1))
                .map(|s| s.as_str());
            let nexus = args
                .iter()
                .position(|a| a == "--nexus")
                .and_then(|i| args.get(i + 1))
                .map(|s| s.as_str());
            let steam_id = args
                .iter()
                .position(|a| a == "--steam-id")
                .and_then(|i| args.get(i + 1))
                .map(|s| s.as_str());
            commands::cli::cli_add_game(
                game_id, name, bottle, path, exe, mod_dir, nexus, steam_id, &db,
            );
            return;
        }

        // --remove-game <game_id>
        if let Some(pos) = args.iter().position(|a| a == "--remove-game") {
            let game_id = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            if game_id.is_empty() {
                eprintln!("Usage: corkscrew --remove-game <game_id>");
                std::process::exit(1);
            }
            commands::cli::cli_remove_game(game_id, &db);
            return;
        }

        // --vortex-test <game_id>  (fetch + execute a Vortex extension, report results as JSON)
        if let Some(pos) = args.iter().position(|a| a == "--vortex-test") {
            let game_id = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            if game_id.is_empty() {
                eprintln!("Usage: corkscrew --vortex-test <game_id>");
                eprintln!("  Example: corkscrew --vortex-test skyrimse");
                std::process::exit(1);
            }
            commands::cli::cli_vortex_test(game_id);
            return;
        }

        // --test-shader-scan <game_id> <bottle_name>  (scan for CS mods, print results as JSON)
        if let Some(pos) = args.iter().position(|a| a == "--test-shader-scan") {
            let game_id = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            let bottle_name = args.get(pos + 2).map(|s| s.as_str()).unwrap_or("");
            if game_id.is_empty() || bottle_name.is_empty() {
                eprintln!("Usage: corkscrew --test-shader-scan <game_id> <bottle_name>");
                eprintln!("  Scans installed mods for Community Shaders dependencies.");
                eprintln!("  Example: corkscrew --test-shader-scan skyrimse Steam");
                std::process::exit(1);
            }
            commands::cli::cli_test_shader_scan(game_id, bottle_name, &db);
            return;
        }

        // --parse-wj <path>  (parse a .wabbajack file and print summary JSON)
        if let Some(pos) = args.iter().position(|a| a == "--parse-wj") {
            let path = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("");
            if path.is_empty() {
                eprintln!("Usage: corkscrew --parse-wj <path-to-wabbajack-file>");
                std::process::exit(1);
            }
            let path = std::path::Path::new(path);
            if !path.exists() {
                eprintln!("File not found: {}", path.display());
                std::process::exit(1);
            }
            eprintln!("Parsing {}...", path.display());
            // First try the summary parse (lenient)
            match wabbajack::parse_wabbajack_file(path) {
                Ok(parsed) => {
                    eprintln!("Summary parse OK: {} archives, {} directives", parsed.archive_count, parsed.directive_count);
                }
                Err(e) => {
                    eprintln!("Summary parse error: {}", e);
                }
            }

            // Now try the full typed parse (strict — this is what the installer uses)
            match wabbajack_installer::parse_wabbajack_file_typed_cli(path) {
                Ok(typed) => {
                    println!("{}", serde_json::json!({
                        "name": typed.name,
                        "archives": typed.archives.len(),
                        "directives": typed.directives.len(),
                        "status": "OK"
                    }));
                }
                Err(e) => {
                    eprintln!("Typed parse error: {}", e);
                    // Diagnose: extract raw JSON and find the problematic field
                    if let Ok(file) = std::fs::File::open(path) {
                        if let Ok(mut archive) = zip::ZipArchive::new(file) {
                            if let Ok(entry) = archive.by_name("modlist") {
                                let raw: Result<serde_json::Value, _> = serde_json::from_reader(entry);
                                if let Ok(val) = raw {
                                    // Check archives for problematic states
                                    if let Some(archives) = val.get("Archives").and_then(|v| v.as_array()) {
                                        eprintln!("\nArchives: {} total", archives.len());
                                        // Find archives with string fields where i64 expected
                                        for (i, a) in archives.iter().enumerate() {
                                            if let Some(state) = a.get("State") {
                                                let t = state.get("$type").and_then(|v| v.as_str()).unwrap_or("");
                                                // Check for string IDs that should be i64
                                                for key in ["ModID", "FileID", "modID", "fileID", "IPS4Mod", "IPS4File"] {
                                                    if let Some(val) = state.get(key) {
                                                        if val.is_string() {
                                                            eprintln!("  FOUND STRING-AS-NUMBER: archive[{}] $type={} {key}={}", i, t, val);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    // Check directives for problematic fields
                                    if let Some(directives) = val.get("Directives").and_then(|v| v.as_array()) {
                                        eprintln!("Directives: {} total", directives.len());
                                        for (i, d) in directives.iter().enumerate() {
                                            if let Some(ahp) = d.get("ArchiveHashPath") {
                                                if ahp.is_string() {
                                                    eprintln!("  FOUND STRING ArchiveHashPath: directive[{}] = {}", i, ahp);
                                                    if i < 3 { eprintln!("    full: {}", serde_json::to_string(d).unwrap_or_default()); }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    std::process::exit(1);
                }
            }
            return;
        }

        // --version  (print version and exit)
        if args.iter().any(|a| a == "--version") {
            println!("{}", env!("CARGO_PKG_VERSION"));
            return;
        }

        // --help  (print usage and exit)
        if args.iter().any(|a| a == "--help" || a == "-h") {
            println!("Corkscrew — Mod Manager for Wine/CrossOver/Proton");
            println!("Version: {}", env!("CARGO_PKG_VERSION"));
            println!();
            println!("USAGE:");
            println!("  corkscrew                                    Launch GUI");
            println!("  corkscrew --launch <game> <bottle> [--skse]  Launch game headless");
            println!("  corkscrew --list-mods <game> <bottle>        List installed mods");
            println!("  corkscrew --search-mods <q> <game> <bottle>  Search mods by name");
            println!("  corkscrew --find-file <pat> <game> <bottle>  Find file across mods");
            println!("  corkscrew --check-plugins <game> <bottle>    Analyze plugin state");
            println!("  corkscrew --sync-plugins <game> <bottle>     Sync plugin state");
            println!("  corkscrew --mod-files <id> <game> <bottle>   Show mod's files");
            println!("  corkscrew --list-bottles                     List detected bottles (JSON)");
            println!("  corkscrew --list-games                       List detected games (JSON)");
            println!("  corkscrew --scan-bottle <bottle>             Diagnostic: dump everything detected in a bottle");
            println!("  corkscrew --db-stats <game> <bottle>         Database statistics (JSON)");
            println!("  corkscrew --db-integrity                     SQLite integrity check");
            println!("  corkscrew --vortex-list                      List cached Vortex extensions (JSON)");
            println!("  corkscrew --deployment-health <game> <bottle> Check deployment integrity");
            println!("  corkscrew --list-profiles <game> <bottle>    List profiles (JSON)");
            println!("  corkscrew --add-game <id> <name> <bottle> <path>  Add custom game");
            println!("  corkscrew --remove-game <id>                 Remove custom game");
            println!("  corkscrew --test-shader-scan <game> <bottle>  Scan for CS mods (JSON)");
            println!("  corkscrew --version                          Print version");
            return;
        }
    }

    // Recover Dock if a previous session crashed while cursor fix was active
    cursor_clamp::recover_dock_if_needed();

    // Clean up orphaned Wabbajack installs from previous crash/forced quit
    commands::cli::cleanup_orphaned_wj_installs(&db);

    // Clean up stale corkscrew_extract_* temp dirs from collection installs
    commands::cli::cleanup_orphaned_temp_dirs();

    // Replay any incomplete deployment journal entries (self-healing)
    {
        let healed = deploy_journal::replay_incomplete(&db);
        if !healed.is_empty() {
            log::info!(
                "Self-healed {} deployment(s) from interrupted operations: {:?}",
                healed.len(),
                healed
            );
        }
    }

    // Clone db for setup closure access
    let db_for_setup = Arc::clone(&db);

    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_liquid_glass::init());

    // WebDriver E2E testing — debug builds only (port 4445)
    #[cfg(debug_assertions)]
    {
        builder = builder.plugin(tauri_plugin_webdriver::init());
    }

    let mut builder = builder
        .setup(move |app| {
            // Register updater plugin in setup per Tauri docs (advanced pattern)
            app.handle()
                .plugin(tauri_plugin_updater::Builder::new().build())?;
            app.manage(app_updates::PendingUpdate(std::sync::Mutex::new(None)));

            // Run startup health check in background
            let health_db = db_for_setup.clone();
            let health_handle = app.handle().clone();
            std::thread::spawn(move || {
                run_startup_health_check(&health_db, &health_handle);
            });

            // Periodic error reporter — every 30 min, report high-frequency errors to Sentry
            let report_db = db_for_setup;
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(30 * 60));
                    let consent = config::get_config()
                        .ok()
                        .and_then(|c| {
                            c.extra
                                .get("telemetry_consent")
                                .and_then(|v| v.as_str().map(String::from))
                        });
                    if consent.as_deref() != Some("granted") {
                        continue;
                    }
                    if let Ok(errors) = report_db.get_error_summary(10) {
                        for err in &errors {
                            if err.count > 5 {
                                sentry::capture_message(
                                    &format!(
                                        "[{}] {}: {} (count: {})",
                                        err.module, err.error_type, err.message, err.count
                                    ),
                                    sentry::Level::Warning,
                                );
                            }
                        }
                    }
                }
            });

            Ok(())
        });

    #[cfg(debug_assertions)]
    {
        builder = builder.plugin(tauri_plugin_mcp::init_with_config(
            tauri_plugin_mcp::PluginConfig::new("Corkscrew".to_string())
                .start_socket_server(true)
                .socket_path("/tmp/corkscrew-mcp.sock".into()),
        ));
    }

    builder
        .manage({
            let queue = download_queue::DownloadQueue::new();
            // Restore persisted queue items from database
            match db.load_queue_items() {
                Ok(items) => {
                    if !items.is_empty() {
                        log::info!(
                            "Restored {} download queue items from database",
                            items.len()
                        );
                        queue.load_from(items);
                    }
                }
                Err(e) => log::warn!("Failed to load download queue from database: {}", e),
            }
            AppState {
                db: Arc::clone(&db),
                download_queue: Arc::new(queue),
                wj_cancel_tokens: std::sync::Mutex::new(std::collections::HashMap::new()),
                fomod_cache: Arc::new(fomod::new_fomod_cache()),
                loot_masterlist_checked: Arc::new(AtomicBool::new(false)),
                chat_session: llm_chat::create_shared_session(),
                game_locks: Arc::new(game_lock::GameLockManager::new()),
                deploy_in_progress: Arc::new(AtomicBool::new(false)),
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::mods::get_bottles,
            commands::mods::get_games,
            commands::mods::get_all_games,
            commands::mods::get_game_version,
            commands::mods::lookup_version_manifest,
            commands::mods::get_depot_history_cmd,
            commands::mods::sync_lua_mods,
            commands::mods::list_supported_games,
            commands::mods::get_game_support_tier,
            commands::mods::get_bottle_settings,
            commands::mods::get_bottle_setting_defs,
            commands::mods::set_bottle_setting,
            commands::mods::get_installed_mods,
            commands::mods::get_installed_mods_summary,
            commands::mods::get_mod_detail,
            commands::mods::install_mod_cmd,
            commands::mods::uninstall_mod,
            commands::mods::toggle_mod,
            commands::mods::batch_toggle_mods,
            commands::mods::get_plugin_order,
            commands::mods::download_from_nexus,
            commands::mods::is_nexus_premium,
            commands::mods::get_config,
            commands::mods::set_config_value,
            commands::mods::get_game_logo,
            commands::mods::launch_game_cmd,
            commands::mods::check_skse,
            commands::mods::get_skse_download_url,
            commands::mods::get_skse_builds,
            commands::mods::install_skse_auto_cmd,
            commands::mods::install_skse_from_archive_cmd,
            commands::mods::uninstall_skse_cmd,
            commands::mods::set_skse_preference_cmd,
            commands::mods::check_skyrim_version,
            commands::mods::check_skse_compatibility_cmd,
            commands::mods::scan_skse_plugins_cmd,
            commands::mods::fix_skse_plugins_cmd,
            commands::mods::list_disabled_wine_plugins_cmd,
            commands::mods::reenable_wine_plugin_cmd,
            commands::mods::fix_skyrim_display,
            commands::mods::downgrade_skyrim,
            commands::mods::get_depot_download_command,
            commands::mods::start_depot_download,
            commands::mods::check_depot_ready,
            commands::mods::apply_downgrade_cmd,
            commands::mods::list_game_versions,
            commands::mods::swap_game_version,
            commands::mods::set_vibrancy,
            // Download Archive Management
            commands::mods::list_download_archives,
            commands::mods::delete_download_archive,
            commands::mods::get_downloads_stats,
            commands::mods::clear_all_download_archives,
            commands::mods::find_orphaned_downloads,
            commands::mods::delete_orphaned_downloads,
            // Notes & Tags
            commands::mods::set_mod_notes,
            commands::mods::set_mod_source,
            commands::mods::set_mod_tags,
            commands::mods::get_all_tags,
            // Auto-category
            commands::mods::backfill_categories,
            // Custom Executables
            commands::tools::add_custom_exe,
            commands::tools::remove_custom_exe,
            commands::tools::list_custom_exes,
            commands::tools::set_default_exe,
            // Deployment Management
            commands::deployment::get_conflicts,
            commands::deployment::analyze_conflicts_cmd,
            commands::deployment::resolve_all_conflicts_cmd,
            commands::deployment::record_conflict_winner,
            commands::deployment::get_deployment_manifest_cmd,
            commands::deployment::set_mod_priority,
            commands::deployment::reorder_mods,
            commands::deployment::redeploy_all_mods,
            commands::deployment::deploy_incremental_cmd,
            commands::deployment::is_deploy_in_progress,
            // Deployment Health
            commands::deployment::check_deployment_health,
            commands::deployment::get_verification_level,
            commands::deployment::set_verification_level,
            commands::deployment::set_use_original_engine_fixes,
            commands::deployment::set_use_wine_engine_fixes,
            commands::deployment::purge_deployment_cmd,
            commands::deployment::verify_mod_integrity,
            commands::deployment::get_deployment_health,
            commands::deployment::get_deployment_stats,
            // Background Hashing
            commands::deployment::start_background_hashing,
            commands::deployment::cancel_background_hashing,
            commands::deployment::get_merged_file_tree,
            // Collection Management
            commands::collections::list_installed_collections_cmd,
            commands::collections::set_mod_collection_name_cmd,
            commands::collections::switch_collection_cmd,
            commands::collections::delete_collection_cmd,
            commands::collections::uninstall_wabbajack_modlist,
            commands::collections::return_to_vanilla,
            commands::collections::collection_download_size_cmd,
            commands::collections::get_collection_diff_cmd,
            commands::collections::sync_plugins_cmd,
            // Collections (GraphQL)
            commands::collections::fetch_url_text,
            commands::collections::browse_nexus_mods_cmd,
            commands::collections::get_nexus_mod_detail,
            commands::collections::search_nexus_mods_cmd,
            commands::collections::get_game_categories_cmd,
            commands::collections::browse_collections_cmd,
            commands::collections::get_collection_cmd,
            commands::collections::get_collection_revisions,
            commands::collections::get_collection_mods,
            commands::collections::parse_collection_bundle_cmd,
            commands::collections::install_collection_cmd,
            commands::collections::cancel_collection_install_cmd,
            commands::collections::submit_fomod_choices,
            // Collection Install Resume
            commands::collections::get_incomplete_collection_installs,
            commands::collections::get_all_interrupted_installs,
            commands::collections::get_checkpoint_mod_names,
            commands::collections::resume_collection_install_cmd,
            commands::collections::abandon_collection_install,
            commands::collections::get_pending_wabbajack_installs,
            commands::collections::dismiss_wabbajack_install,
            // Endorsements
            commands::collections::endorse_mod,
            commands::collections::abstain_mod,
            commands::collections::get_user_endorsements,
            // Download Cache Check
            commands::collections::check_cached_files,
            // LOOT & Plugin Management
            commands::plugins::sort_plugins_loot,
            commands::plugins::update_loot_masterlist,
            commands::plugins::force_refresh_loot_masterlist,
            commands::plugins::get_masterlist_status,
            commands::plugins::reorder_plugins_cmd,
            commands::plugins::toggle_plugin_cmd,
            commands::plugins::move_plugin_cmd,
            commands::plugins::get_plugin_messages,
            // File-Based Load Order (generic)
            commands::load_order::get_load_order_kind_cmd,
            commands::load_order::get_file_based_load_order,
            commands::load_order::set_file_based_load_order,
            // Plugin Load Order Rules
            commands::plugins::add_plugin_rule,
            commands::plugins::remove_plugin_rule,
            commands::plugins::list_plugin_rules,
            commands::plugins::clear_plugin_rules,
            // Profiles
            commands::profiles::list_profiles_cmd,
            commands::profiles::create_profile_cmd,
            commands::profiles::delete_profile_cmd,
            commands::profiles::deactivate_profile_cmd,
            commands::profiles::rename_profile_cmd,
            commands::profiles::save_profile_snapshot,
            commands::profiles::activate_profile,
            commands::profiles::get_profile_save_info,
            commands::profiles::backup_profile_saves,
            commands::profiles::restore_profile_saves,
            // Update Checking
            commands::notifications::check_mod_updates,
            // Mod Tools
            commands::tools::detect_mod_tools_cmd,
            commands::tools::install_mod_tool,
            commands::tools::uninstall_mod_tool,
            commands::tools::launch_mod_tool,
            commands::tools::reinstall_mod_tool,
            commands::tools::check_mod_tool_update,
            commands::tools::apply_tool_ini_edits_cmd,
            commands::tools::detect_collection_tools,
            commands::tools::detect_wabbajack_tools,
            // Wine Prefix Dependencies
            commands::tools::get_prefix_dependencies,
            commands::tools::install_prefix_dependencies,
            // FromSoft: Mod Engine 2 modengine2.toml editor
            commands::modengine2_cmds::get_modengine2_config,
            commands::modengine2_cmds::save_modengine2_config,
            commands::modengine2_cmds::add_mod_to_modengine2,
            commands::modengine2_cmds::remove_mod_from_modengine2,
            // FromSoft: regulation.bin conflict detection
            commands::modengine2_cmds::get_regulation_conflicts,
            // FromSoft: pre-launch save backup
            commands::modengine2_cmds::list_fromsoft_saves,
            commands::modengine2_cmds::get_fromsoft_saves_dir,
            commands::modengine2_cmds::backup_fromsoft_saves,
            // Platform Detection
            commands::platform::get_platform_detail,
            commands::platform::get_optimal_download_threads,
            // FOMOD
            commands::config_commands::detect_fomod,
            commands::config_commands::get_fomod_defaults,
            commands::config_commands::get_fomod_files,
            // DLC Detection
            commands::platform::check_dlc_status,
            // Integrity
            commands::diagnostics::create_game_snapshot,
            commands::diagnostics::check_game_integrity,
            commands::diagnostics::has_game_snapshot,
            // Game Directory Cleaner
            commands::game_state::list_known_uninstalled_games_cmd,
            commands::game_state::scan_game_directory,
            commands::game_state::clean_game_directory,
            // Native Game Manual Add
            commands::game_state::add_native_game_manually,
            // Wabbajack Modlists
            commands::wabbajack::get_wabbajack_modlists,
            commands::wabbajack::parse_wabbajack_file,
            commands::wabbajack::check_wabbajack_cache,
            commands::wabbajack::download_wabbajack_file,
            // Wabbajack Install Pipeline
            wabbajack_installer::install_wabbajack_modlist_cmd,
            wabbajack_installer::cancel_wabbajack_install,
            wabbajack_installer::resume_wabbajack_install,
            wabbajack_installer::cleanup_wabbajack_install,
            wabbajack_installer::get_wabbajack_install_status,
            wabbajack_installer::wabbajack_preflight_cmd,
            // Nexus SSO
            commands::nexus::start_nexus_sso,
            // OAuth
            commands::nexus::start_oauth_login,
            commands::nexus::start_nexus_oauth,
            commands::nexus::refresh_nexus_tokens,
            commands::nexus::save_oauth_tokens,
            commands::nexus::load_oauth_tokens,
            commands::nexus::clear_oauth_tokens,
            commands::nexus::get_nexus_user_info,
            commands::nexus::get_auth_method_cmd,
            commands::nexus::get_nexus_account_status,
            // Google OAuth (Gemini)
            commands::nexus::google_sign_in,
            commands::nexus::google_sign_out,
            commands::nexus::google_auth_status,
            // Crash Logs
            commands::diagnostics::find_crash_logs_cmd,
            commands::diagnostics::analyze_crash_log_cmd,
            commands::diagnostics::chat_check_new_crashes,
            // Collections & Nexus Browse
            commands::collections::restore_mod_snapshot,
            // Game Version Pinning
            commands::game_state::get_pinned_game_version,
            commands::game_state::pin_game_version,
            // Rollback & Snapshots
            commands::game_state::save_mod_version_cmd,
            commands::game_state::list_mod_versions_cmd,
            commands::game_state::rollback_mod_version,
            commands::game_state::cleanup_mod_versions,
            commands::game_state::create_mod_snapshot,
            commands::game_state::list_mod_snapshots,
            commands::game_state::delete_mod_snapshot,
            // Modlist Import/Export
            commands::config_commands::export_modlist_cmd,
            commands::config_commands::import_modlist_plan,
            commands::config_commands::diff_modlists_cmd,
            commands::config_commands::execute_modlist_import,
            // Disk Budget
            commands::config_commands::get_disk_budget,
            commands::config_commands::estimate_install_impact_cmd,
            commands::config_commands::get_available_disk_space_cmd,
            // Staging Info
            commands::config_commands::get_staging_info,
            commands::config_commands::set_staging_directory,
            // INI Manager
            commands::config_commands::get_ini_settings,
            commands::config_commands::set_ini_setting,
            commands::config_commands::get_ini_presets,
            commands::config_commands::apply_ini_preset,
            commands::config_commands::read_mod_file,
            commands::config_commands::write_mod_file,
            // Wine Diagnostics
            commands::diagnostics::run_wine_diagnostics,
            commands::diagnostics::fix_wine_appdata,
            commands::diagnostics::fix_wine_dll_override,
            commands::diagnostics::fix_wine_retina_mode,
            commands::diagnostics::check_prefix_health_linux,
            // DXVK Configuration
            commands::diagnostics::get_dxvk_config,
            commands::diagnostics::deploy_dxvk_config,
            commands::diagnostics::detect_dxvk_version,
            // Pre-flight
            commands::diagnostics::run_preflight_check,
            // Mod Dependencies
            commands::diagnostics::add_mod_dependency,
            commands::diagnostics::remove_mod_dependency,
            commands::diagnostics::get_mod_dependencies,
            commands::diagnostics::get_mod_dependents,
            commands::diagnostics::check_dependency_issues,
            // Mod Recommendations
            commands::diagnostics::get_mod_recommendations,
            commands::diagnostics::get_popular_mods,
            // Session Tracker
            commands::diagnostics::start_game_session,
            commands::diagnostics::end_game_session,
            commands::diagnostics::record_session_mod_change,
            commands::diagnostics::get_session_history,
            commands::diagnostics::get_stability_summary,
            // Game Lock
            commands::game_state::get_game_lock_status,
            commands::game_state::get_all_game_locks,
            commands::game_state::force_unlock_game,
            // Deploy Journal
            commands::game_state::get_deploy_journal_status,
            commands::game_state::heal_deployment,
            // FOMOD Recipes
            commands::config_commands::save_fomod_recipe,
            commands::config_commands::get_fomod_recipe,
            commands::config_commands::list_fomod_recipes,
            commands::config_commands::delete_fomod_recipe,
            commands::config_commands::has_compatible_fomod_recipe,
            // Embedded Browser Webview
            commands::notifications::create_browser_webview,
            commands::notifications::resize_browser_webview,
            commands::notifications::close_browser_webview,
            commands::notifications::navigate_browser_webview,
            // Nexus Mod Files & Direct Download
            commands::nexus::get_nexus_mod_files,
            commands::nexus::download_and_install_nexus_mod,
            // Download Cache
            // (check_cached_files is in collections)
            // Proton Detection
            commands::platform::list_proton_versions,
            commands::platform::get_recommended_proton,
            // Steam Integration
            commands::platform::detect_steam,
            commands::platform::check_steam_status,
            commands::platform::add_to_steam,
            commands::platform::remove_from_steam,
            commands::platform::is_steam_deck,
            commands::platform::steam_deck_warnings,
            commands::platform::get_launch_options_status,
            commands::platform::patch_steam_launch_options,
            commands::platform::unpatch_steam_launch_options,
            // Steam Deck Profile
            commands::platform::get_deck_profile,
            commands::platform::get_deck_defaults,
            // NXM Handler
            commands::nexus::register_nxm_handler,
            commands::nexus::unregister_nxm_handler,
            commands::nexus::is_nxm_handler_registered,
            // Instruction parsing (LLM)
            commands::config_commands::parse_instructions_cmd,
            commands::config_commands::parse_instructions_llm_cmd,
            commands::config_commands::parse_instructions_cloud_cmd,
            commands::config_commands::validate_instruction_actions_cmd,
            commands::config_commands::check_ollama_status_cmd,
            commands::config_commands::get_recommended_models,
            commands::config_commands::get_cloud_providers,
            commands::config_commands::pull_ollama_model_cmd,
            commands::config_commands::delete_ollama_model_cmd,
            commands::config_commands::unload_ollama_model_cmd,
            commands::platform::get_system_memory,
            commands::platform::install_ollama,
            commands::platform::start_ollama,
            commands::platform::check_mlx_status,
            commands::platform::install_mlx,
            commands::platform::get_recommended_model,
            // LLM Chat
            commands::chat::chat_get_state,
            commands::chat::chat_get_starters,
            commands::chat::chat_load_model,
            commands::chat::chat_unload_model,
            commands::chat::get_cached_mlx_models,
            commands::chat::delete_model,
            commands::chat::chat_send_message,
            commands::chat::chat_clear_history,
            commands::chat::chat_get_history,
            commands::chat::chat_validate_cloud_key,
            // Vortex Extensions
            commands::config_commands::vortex_fetch_extension,
            commands::config_commands::vortex_refresh_extension,
            commands::config_commands::vortex_list_cached_extensions,
            commands::config_commands::vortex_list_available_extensions,
            commands::config_commands::vortex_delete_cached_extension,
            commands::config_commands::vortex_get_extension_detail,
            commands::config_commands::get_vortex_extension_suggestions,
            // Native Mode (experimental)
            commands::config_commands::get_native_mode,
            commands::config_commands::set_native_mode,
            commands::config_commands::get_native_mode_visible,
            commands::config_commands::set_native_mode_visible,
            commands::native_cmds::rescan_native_games,
            commands::native_cmds::add_manual_native_game,
            commands::native_cmds::register_manual_native_game,
            commands::native_cmds::get_bg3se_status,
            commands::native_cmds::get_paralives_bepinex_status,
            commands::native_cmds::install_paralives_bepinex,
            commands::native_cmds::uninstall_paralives_bepinex,
            commands::native_cmds::apply_native_window_effect,
            commands::stardew_cmds::get_stardew_mod_status,
            // Download Queue
            commands::notifications::get_download_queue,
            commands::notifications::get_download_queue_counts,
            commands::notifications::retry_download,
            commands::notifications::cancel_download,
            commands::notifications::clear_finished_downloads,
            // Notification Log
            commands::notifications::get_notification_log,
            commands::notifications::clear_notification_log,
            commands::notifications::log_notification,
            commands::notifications::get_notification_count,
            // Error Event Diagnostics
            commands::notifications::record_error_event_cmd,
            commands::notifications::get_error_summary,
            // DepotDownloader (game version rollback)
            commands::depot::dd_status,
            commands::depot::dd_install,
            commands::depot::dd_ensure_updated,
            commands::depot::dd_authenticate,
            commands::depot::dd_logout,
            commands::depot::dd_check_partial_download,
            commands::depot::dd_delete_partial_download,
            commands::depot::dd_list_manifests,
            commands::depot::dd_download_depot,
            commands::depot::dd_get_depot_versions,
            commands::depot::dd_apply_depot,
            // Self-Update (macOS fallback)
            self_update::get_installed_app_version,
            self_update::manual_self_update,
            // App Updates (advanced Rust-side updater per Tauri docs)
            app_updates::fetch_update,
            app_updates::install_update,
            // Shader Conversion
            shader_conversion::scan_shader_compatibility,
            shader_conversion::quick_cs_mod_count,
            shader_conversion::discover_shader_swap_options,
            shader_conversion::execute_shader_conversion_cmd,
            shader_conversion::revert_shader_conversion_cmd,
            shader_conversion::get_shader_conversion_history_cmd,
            // Verified Lists (Wine compatibility registry)
            commands::verified_lists_cmds::get_collection_verification,
            commands::verified_lists_cmds::get_wabbajack_verification,
            commands::verified_lists_cmds::get_verification_manifest,
            commands::verified_lists_cmds::refresh_verification_manifest,
            commands::verified_lists_cmds::get_verification_cache_age_secs,
            // Thunderstore catalog + install
            commands::thunderstore_cmds::thunderstore_list_communities,
            commands::thunderstore_cmds::thunderstore_list_packages,
            commands::thunderstore_cmds::thunderstore_install_package,
            commands::thunderstore_cmds::thunderstore_install_with_dependencies,
            // CrossOver shortcut auto-discovery (unregistered games)
            commands::crossover_cmds::list_unregistered_crossover_games,
            commands::crossover_cmds::register_unregistered_game,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| match event {
            tauri::RunEvent::Exit | tauri::RunEvent::ExitRequested { .. } => {
                kill_mlx_server();
            }
            _ => {}
        });
}
