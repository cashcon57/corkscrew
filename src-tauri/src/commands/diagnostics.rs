//! Diagnostic commands: Wine compatibility, preflight, crash logs, DXVK, and session tracking.

use crate::crashlog;
use crate::integrity;
use crate::crashlog::{CrashLogEntry, CrashReport, NewCrashInfo};
use crate::integrity::{IntegrityReport};
use crate::mod_dependencies;
use crate::mod_recommendations;
use crate::preflight;
use crate::session_tracker;
use crate::wine_diagnostic;
use crate::{AppState, resolve_bottle, resolve_game};
use std::path::{Path, PathBuf};
use tauri::State;

// --- Wine Diagnostic Commands ---

#[tauri::command]
pub async fn run_wine_diagnostics(
    game_id: String,
    bottle_name: String,
) -> Result<wine_diagnostic::DiagnosticResult, String> {
    tokio::task::spawn_blocking(move || {
        let bottle = resolve_bottle(&bottle_name)?;
        Ok(wine_diagnostic::run_diagnostics(&bottle, &game_id))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn fix_wine_appdata(bottle_name: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let bottle = resolve_bottle(&bottle_name)?;
        wine_diagnostic::fix_appdata(&bottle).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn fix_wine_dll_override(
    bottle_name: String,
    dll_name: String,
    override_type: String,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let bottle = resolve_bottle(&bottle_name)?;
        wine_diagnostic::fix_dll_override(&bottle, &dll_name, &override_type)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn fix_wine_retina_mode(bottle_name: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let bottle = resolve_bottle(&bottle_name)?;
        wine_diagnostic::fix_retina_mode(&bottle).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn check_prefix_health_linux(
    bottle_name: String,
    game_id: String,
) -> Result<wine_diagnostic::LinuxPrefixHealth, String> {
    tokio::task::spawn_blocking(move || {
        let bottle = resolve_bottle(&bottle_name)?;
        Ok(wine_diagnostic::check_prefix_health_linux(&bottle.path, &game_id))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}


// --- Pre-flight Commands ---

#[tauri::command]
pub async fn run_preflight_check(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<preflight::PreflightResult, String> {
    let (bottle, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        Ok(preflight::run_preflight(
            &db,
            &bottle,
            &game_id,
            &bottle_name,
            &data_dir,
        ))
    })
    .await
    .map_err(|e| format!("Preflight task failed: {e}"))?
}


// --- Mod Dependency Commands ---

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn add_mod_dependency(
    game_id: String,
    bottle_name: String,
    mod_id: i64,
    depends_on_id: Option<i64>,
    nexus_dep_id: Option<i64>,
    dep_name: String,
    relationship: String,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        mod_dependencies::add_dependency(
            &db,
            &game_id,
            &bottle_name,
            mod_id,
            depends_on_id,
            nexus_dep_id,
            &dep_name,
            &relationship,
        )
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn remove_mod_dependency(dep_id: i64, state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || mod_dependencies::remove_dependency(&db, dep_id))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn get_mod_dependencies(
    mod_id: i64,
    state: State<'_, AppState>,
) -> Result<Vec<mod_dependencies::ModDependency>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || mod_dependencies::get_dependencies(&db, mod_id))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn get_mod_dependents(
    mod_id: i64,
    state: State<'_, AppState>,
) -> Result<Vec<mod_dependencies::ModDependency>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || mod_dependencies::get_dependents(&db, mod_id))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn check_dependency_issues(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<mod_dependencies::DependencyIssue>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        mod_dependencies::check_dependency_issues(&db, &game_id, &bottle_name)
    })
    .await
    .map_err(|e| format!("Dependency check task failed: {e}"))?
}


// --- Mod Recommendation Commands ---

#[tauri::command]
pub async fn get_mod_recommendations(
    game_id: String,
    bottle_name: String,
    target_mod_id: i64,
    state: State<'_, AppState>,
) -> Result<mod_recommendations::RecommendationResult, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        mod_recommendations::get_recommendations(&db, &game_id, &bottle_name, target_mod_id)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn get_popular_mods(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<(String, i64, usize)>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        mod_recommendations::get_popular_mods(&db, &game_id, &bottle_name)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}


// --- Crash Logs ---

#[tauri::command]
pub async fn find_crash_logs_cmd(
    game_id: String,
    bottle_name: String,
) -> Result<Vec<CrashLogEntry>, String> {
    tokio::task::spawn_blocking(move || {
        let (bottle, game, _) = resolve_game(&game_id, &bottle_name)?;

        let game_path = PathBuf::from(&game.game_path);
        Ok(crashlog::find_crash_logs(
            &game_path,
            &bottle.path,
            &game_id,
        ))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn analyze_crash_log_cmd(log_path: String) -> Result<CrashReport, String> {
    tokio::task::spawn_blocking(move || {
        crashlog::analyze_crash_log(Path::new(&log_path)).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Check for new (unseen) crash logs since the last check.
#[tauri::command]
pub async fn chat_check_new_crashes(
    game_id: String,
    bottle_name: String,
) -> Result<NewCrashInfo, String> {
    tokio::task::spawn_blocking(move || {
        let bottle = resolve_bottle(&bottle_name)?;
        Ok(crashlog::check_new_crashes(&PathBuf::from(&bottle.path), &game_id))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}


// --- Integrity ---

#[tauri::command]
pub async fn create_game_snapshot(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<usize, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
        integrity::create_game_snapshot(&db, &game_id, &bottle_name, &data_dir)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn check_game_integrity(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<IntegrityReport, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
        integrity::check_game_integrity(&db, &game_id, &bottle_name, &data_dir)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn has_game_snapshot(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        integrity::has_snapshot(&db, &game_id, &bottle_name).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}


// --- Session Tracker Commands ---

#[tauri::command]
pub async fn start_game_session(
    game_id: String,
    bottle_name: String,
    profile_name: Option<String>,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        session_tracker::start_session(&db, &game_id, &bottle_name, profile_name.as_deref())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn end_game_session(
    session_id: i64,
    clean_exit: bool,
    crash_log_path: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        session_tracker::end_session(&db, session_id, clean_exit, crash_log_path.as_deref())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn record_session_mod_change(
    session_id: i64,
    mod_id: Option<i64>,
    mod_name: String,
    change_type: String,
    detail: Option<String>,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        session_tracker::record_mod_change(
            &db,
            session_id,
            mod_id,
            &mod_name,
            &change_type,
            detail.as_deref(),
        )
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn get_session_history(
    game_id: String,
    bottle_name: String,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<session_tracker::GameSession>, String> {
    let db = state.db.clone();
    let lim = limit.unwrap_or(20);
    tokio::task::spawn_blocking(move || {
        session_tracker::get_session_history(&db, &game_id, &bottle_name, lim)
    })
    .await
    .map_err(|e| format!("Session history task failed: {e}"))?
}

#[tauri::command]
pub async fn get_stability_summary(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<session_tracker::StabilitySummary, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        session_tracker::get_stability_summary(&db, &game_id, &bottle_name)
    })
    .await
    .map_err(|e| format!("Stability summary task failed: {e}"))?
}


// --- DXVK Configuration Commands ---

#[tauri::command]
pub async fn get_dxvk_config(game_id: String) -> Result<String, String> {
    Ok(crate::dxvk::generate_config(
        &game_id,
        &crate::dxvk::DxvkPreset::Default,
        &std::collections::HashMap::new(),
    ))
}

#[tauri::command]
pub async fn deploy_dxvk_config(
    game_dir: String,
    game_id: String,
    preset: String,
) -> Result<String, String> {
    let preset = match preset.as_str() {
        "performance" => crate::dxvk::DxvkPreset::Performance,
        "compatibility" => crate::dxvk::DxvkPreset::Compatibility,
        _ => crate::dxvk::DxvkPreset::Default,
    };
    crate::dxvk::deploy_config(
        std::path::Path::new(&game_dir),
        &game_id,
        &preset,
        &std::collections::HashMap::new(),
    )
    .map(|p| p.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn detect_dxvk_version(prefix_path: String) -> Result<Option<String>, String> {
    Ok(crate::dxvk::detect_dxvk_version(std::path::Path::new(
        &prefix_path,
    )))
}

