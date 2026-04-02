//! Mod tool management: detection, installation, and custom executables.

use crate::collections;
use crate::executables;
use crate::executables::{CustomExecutable};
use crate::launcher::{LaunchResult};
use crate::mod_tools;
use crate::wabbajack;
use crate::{AppState, resolve_game};
use std::path::Path;
use tauri::{AppHandle, State};

// --- Mod Tools ---

#[tauri::command]
pub async fn detect_mod_tools_cmd(
    game_id: String,
    bottle_name: String,
    _state: State<'_, AppState>,
) -> Result<Vec<mod_tools::ModTool>, String> {
    let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
    tokio::task::spawn_blocking(move || mod_tools::detect_tools_for_game(&data_dir, &game_id))
        .await
        .map_err(|e| format!("Tool detection task failed: {e}"))
}

#[tauri::command]
pub async fn install_mod_tool(
    app: AppHandle,
    tool_id: String,
    game_id: String,
    bottle_name: String,
) -> Result<String, String> {
    let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
    mod_tools::install_tool(&tool_id, &data_dir, &app)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn uninstall_mod_tool(
    tool_id: String,
    game_id: String,
    bottle_name: String,
    detected_path: Option<String>,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
        mod_tools::uninstall_tool(&tool_id, &data_dir, detected_path.as_deref())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn launch_mod_tool(
    tool_id: String,
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<LaunchResult, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let (bottle, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
        let tools = mod_tools::detect_tools_for_game(&data_dir, &game_id);
        let tool = tools
            .iter()
            .find(|t| t.id == tool_id)
            .ok_or_else(|| format!("Tool '{}' not found", tool_id))?;
        let exe_path = tool
            .detected_path
            .as_ref()
            .ok_or_else(|| format!("Tool '{}' is not installed", tool_id))?;

        // Apply DLL overrides for this tool
        let overrides = crate::wine_dll_overrides::detect_and_get_overrides(Path::new(exe_path));
        if !overrides.is_empty() {
            if let Err(e) = crate::wine_dll_overrides::apply_overrides(&bottle.path, &overrides) {
                log::warn!("Failed to apply DLL overrides for {}: {}", exe_path, e);
            }
        }

        mod_tools::launch_tool_with_logging(Path::new(exe_path), &bottle, &tool_id, &tool.name, &db)
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn get_prefix_dependencies(game_id: String, prefix_path: String) -> Result<Vec<crate::prefix_setup::WineDependency>, String> {
    let mut deps = crate::prefix_setup::get_game_dependencies(&game_id);
    crate::prefix_setup::check_installed_deps(std::path::Path::new(&prefix_path), &mut deps);
    Ok(deps)
}

#[tauri::command]
pub async fn install_prefix_dependencies(
    app: AppHandle,
    game_id: String,
    prefix_path: String,
    wine_bin: Option<String>,
    steam_app_id: Option<u32>,
) -> Result<Vec<crate::prefix_setup::InstallResult>, String> {
    let mut deps = crate::prefix_setup::get_game_dependencies(&game_id);
    crate::prefix_setup::check_installed_deps(std::path::Path::new(&prefix_path), &mut deps);
    Ok(crate::prefix_setup::install_dependencies(
        &app,
        std::path::Path::new(&prefix_path),
        wine_bin.as_ref().map(|p| std::path::Path::new(p.as_str())),
        &deps,
        steam_app_id,
    ))
}

#[tauri::command]
pub async fn reinstall_mod_tool(
    app: AppHandle,
    tool_id: String,
    game_id: String,
    bottle_name: String,
) -> Result<String, String> {
    let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
    mod_tools::reinstall_tool(&tool_id, &data_dir, &app)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn check_mod_tool_update(
    tool_id: String,
    game_id: String,
    bottle_name: String,
) -> Result<mod_tools::ToolUpdateInfo, String> {
    let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
    mod_tools::check_tool_update(&tool_id, &data_dir)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn apply_tool_ini_edits_cmd(
    tool_id: String,
    game_id: String,
    bottle_name: String,
) -> Result<usize, String> {
    tokio::task::spawn_blocking(move || {
        let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
        mod_tools::apply_tool_ini_edits(&tool_id, &data_dir).map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}


// --- Tool Requirement Detection ---

#[tauri::command]
pub async fn detect_collection_tools(
    manifest_json: String,
    game_id: String,
    bottle_name: String,
) -> Result<Vec<mod_tools::RequiredTool>, String> {
    tokio::task::spawn_blocking(move || {
        let manifest: collections::CollectionManifest = serde_json::from_str(&manifest_json)
            .map_err(|e| format!("Invalid manifest JSON: {}", e))?;
        let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
        Ok(mod_tools::detect_required_tools_collection(
            &manifest, &data_dir, &game_id,
        ))
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn detect_wabbajack_tools(
    wj_path: String,
    game_id: String,
    bottle_name: String,
) -> Result<Vec<mod_tools::RequiredTool>, String> {
    tokio::task::spawn_blocking(move || {
        let parsed = wabbajack::parse_wabbajack_file(std::path::Path::new(&wj_path))
            .map_err(|e| format!("Failed to parse .wabbajack: {}", e))?;
        let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
        Ok(mod_tools::detect_required_tools_wabbajack(
            &parsed, &data_dir,
        ))
    })
    .await
    .map_err(crate::format_join_error)?
}


// --- Custom Executables ---

#[tauri::command]
pub async fn add_custom_exe(
    game_id: String,
    bottle_name: String,
    name: String,
    exe_path: String,
    working_dir: Option<String>,
    args: Option<String>,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        executables::add_executable(
            &db,
            &game_id,
            &bottle_name,
            &name,
            &exe_path,
            working_dir.as_deref(),
            args.as_deref(),
        )
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn remove_custom_exe(exe_id: i64, state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || executables::remove_executable(&db, exe_id))
        .await
        .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn list_custom_exes(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<CustomExecutable>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || executables::list_executables(&db, &game_id, &bottle_name))
        .await
        .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn set_default_exe(
    game_id: String,
    bottle_name: String,
    exe_id: Option<i64>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || match exe_id {
        Some(id) => executables::set_default_executable(&db, &game_id, &bottle_name, id),
        None => executables::clear_default_executable(&db, &game_id, &bottle_name),
    })
    .await
    .map_err(crate::format_join_error)?
}

