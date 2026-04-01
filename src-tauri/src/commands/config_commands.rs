//! Configuration and data management: disk budget, staging, INI editing, FOMOD,
//! instruction parsing, modlist import/export, and Vortex extension management.

use crate::config;
use crate::deployer;
use crate::disk_budget;
use crate::fomod;
use crate::fomod::{FomodInstaller};
use crate::fomod_recipes;
use crate::ini_manager;
use crate::instruction_parser;
use crate::instruction_types;
use crate::instruction_validator;
use crate::llm_parser;
use crate::modlist_io;
use crate::modlist_io::{ImportPlan, ModlistDiff};
use crate::staging;
use crate::vortex_fetcher;
use crate::vortex_registry;
use crate::vortex_types;
use crate::{AppState, resolve_bottle, resolve_game};
use std::path::{Path, PathBuf};
use tauri::State;

// --- Disk Budget Commands ---

#[tauri::command]
pub async fn get_disk_budget(
    game_id: String,
    bottle_name: String,
) -> Result<disk_budget::DiskBudget, String> {
    tokio::task::spawn_blocking(move || {
        let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
        Ok(disk_budget::compute_budget(
            &game_id,
            &bottle_name,
            &data_dir,
        ))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn estimate_install_impact_cmd(
    archive_size: u64,
    game_id: String,
    bottle_name: String,
) -> Result<disk_budget::InstallImpact, String> {
    tokio::task::spawn_blocking(move || {
        let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
        Ok(disk_budget::estimate_install_impact(
            archive_size,
            &data_dir,
        ))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn get_available_disk_space_cmd(path: String) -> Result<u64, String> {
    tokio::task::spawn_blocking(move || {
        Ok(disk_budget::available_space(std::path::Path::new(&path)))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}


// --- Staging Info Commands ---

#[tauri::command]
pub async fn get_staging_info(
    game_id: String,
    bottle_name: String,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || {
        let staging_root = staging::staging_root();
        let staging_dir = staging::staging_base_dir(&game_id, &bottle_name);

        let (hardlinks_supported, data_dir_str) = match resolve_game(&game_id, &bottle_name) {
            Ok((_, _, data_dir)) => (
                deployer::same_filesystem(&staging_dir, &data_dir),
                data_dir.to_string_lossy().to_string(),
            ),
            Err(_) => (false, String::new()),
        };

        let config = config::get_config().map_err(|e| e.to_string())?;
        let is_custom = config.staging_dir.is_some();

        Ok(serde_json::json!({
            "staging_root": staging_root.to_string_lossy(),
            "staging_dir": staging_dir.to_string_lossy(),
            "data_dir": data_dir_str,
            "hardlinks_supported": hardlinks_supported,
            "is_custom_path": is_custom,
        }))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn set_staging_directory(path: Option<String>) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        match path {
            Some(ref p) if !p.is_empty() => {
                // Validate path exists or can be created
                let path_buf = std::path::PathBuf::from(p);
                if !path_buf.exists() {
                    std::fs::create_dir_all(&path_buf)
                        .map_err(|e| format!("Cannot create staging directory '{}': {}", p, e))?;
                }
                config::set_config_value("staging_dir", p).map_err(|e| e.to_string())
            }
            _ => {
                // Clear override — revert to default
                let mut cfg = config::get_config().map_err(|e| e.to_string())?;
                cfg.staging_dir = None;
                config::save_config(&cfg).map_err(|e| e.to_string())
            }
        }
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}


// --- INI Manager Commands ---

#[tauri::command]
pub async fn get_ini_settings(
    game_id: String,
    bottle_name: String,
) -> Result<Vec<ini_manager::IniFile>, String> {
    tokio::task::spawn_blocking(move || {
        let bottle = resolve_bottle(&bottle_name)?;
        Ok(ini_manager::read_all_ini(&bottle, &game_id))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn set_ini_setting(
    file_path: String,
    section: String,
    key: String,
    value: String,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        ini_manager::set_setting(Path::new(&file_path), &section, &key, &value)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn get_ini_presets(game_id: String) -> Result<Vec<ini_manager::IniPreset>, String> {
    tokio::task::spawn_blocking(move || Ok(ini_manager::builtin_presets(&game_id)))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn apply_ini_preset(
    game_id: String,
    bottle_name: String,
    preset_name: String,
) -> Result<usize, String> {
    tokio::task::spawn_blocking(move || {
        let bottle = resolve_bottle(&bottle_name)?;
        let presets = ini_manager::builtin_presets(&game_id);
        let preset = presets
            .iter()
            .find(|p| p.name == preset_name)
            .ok_or_else(|| format!("Preset '{}' not found", preset_name))?;
        ini_manager::apply_preset(&bottle, &game_id, preset).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Read a text file from a mod's staging directory.
/// `staging_path` is the mod's staging root, `relative_path` is the file within it.
#[tauri::command]
pub async fn read_mod_file(staging_path: String, relative_path: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let full = Path::new(&staging_path).join(&relative_path);
        if !full.exists() {
            return Err(format!("File not found: {}", full.display()));
        }
        // Prevent directory traversal
        let canon = full.canonicalize().map_err(|e| e.to_string())?;
        let base = Path::new(&staging_path)
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if !canon.starts_with(&base) {
            return Err("Path traversal denied".into());
        }
        std::fs::read_to_string(&canon).map_err(|e| format!("Failed to read file: {}", e))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Write a text file in a mod's staging directory.
#[tauri::command]
pub async fn write_mod_file(
    staging_path: String,
    relative_path: String,
    content: String,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let full = Path::new(&staging_path).join(&relative_path);
        // Prevent directory traversal
        let base = Path::new(&staging_path)
            .canonicalize()
            .map_err(|e| e.to_string())?;
        // For writes, parent must exist and resolved path must be under base
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let canon = full.canonicalize().unwrap_or_else(|_| full.clone());
        if !canon.starts_with(&base) {
            return Err("Path traversal denied".into());
        }
        std::fs::write(&full, content).map_err(|e| format!("Failed to write file: {}", e))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}


// --- FOMOD ---

#[tauri::command]
pub async fn detect_fomod(
    staging_path: String,
    archive_hash: Option<String>,
    state: State<'_, AppState>,
) -> Result<Option<FomodInstaller>, String> {
    let fomod_cache = state.fomod_cache.clone();
    tokio::task::spawn_blocking(move || {
        let path = PathBuf::from(&staging_path);
        // Use archive SHA-256 hash as cache key if provided, otherwise fall back
        // to the staging path itself (still deterministic per-archive).
        let cache_key = archive_hash.unwrap_or_else(|| staging_path.clone());
        let mut installer = fomod::parse_fomod_cached(&fomod_cache, &cache_key, &path)
            .map_err(|e| e.to_string())?;
        // Resolve relative image paths to absolute so the frontend can serve them
        // via the Tauri asset: protocol.
        if let Some(ref mut inst) = installer {
            fomod::resolve_image_paths(inst, &path);
        }
        Ok(installer)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn get_fomod_defaults(
    installer: FomodInstaller,
) -> Result<std::collections::HashMap<String, Vec<String>>, String> {
    tokio::task::spawn_blocking(move || Ok(fomod::get_default_selections(&installer, None, None)))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn get_fomod_files(
    installer: FomodInstaller,
    selections: std::collections::HashMap<String, Vec<String>>,
) -> Result<Vec<fomod::FomodFile>, String> {
    tokio::task::spawn_blocking(move || {
        Ok(fomod::get_files_for_selections(
            &installer,
            &selections,
            None,
            None,
        ))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}


// --- FOMOD Recipe Commands ---

#[tauri::command]
pub async fn save_fomod_recipe(
    mod_id: i64,
    mod_name: String,
    installer_hash: Option<String>,
    selections: std::collections::HashMap<String, Vec<String>>,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        fomod_recipes::save_recipe(
            &db,
            mod_id,
            &mod_name,
            installer_hash.as_deref(),
            &selections,
        )
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn get_fomod_recipe(
    mod_id: i64,
    state: State<'_, AppState>,
) -> Result<Option<fomod_recipes::FomodRecipe>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || fomod_recipes::get_recipe(&db, mod_id))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn list_fomod_recipes(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<fomod_recipes::FomodRecipe>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || fomod_recipes::list_recipes(&db, &game_id, &bottle_name))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn delete_fomod_recipe(mod_id: i64, state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || fomod_recipes::delete_recipe(&db, mod_id))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn has_compatible_fomod_recipe(
    mod_id: i64,
    current_hash: Option<String>,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        fomod_recipes::has_compatible_recipe(&db, mod_id, current_hash.as_deref())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}


// --- Instruction Parsing (Collection Author Instructions) ---

/// Parse collection instructions using the deterministic (Tier 1) parser.
#[tauri::command]
pub async fn parse_instructions_cmd(
    instructions: String,
    mod_names: Vec<String>,
) -> Result<instruction_types::ParsedInstructions, String> {
    tokio::task::spawn_blocking(move || {
        Ok(instruction_parser::parse_instructions(
            &instructions,
            &mod_names,
        ))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Parse instructions using a local Ollama model (Tier 2a).
#[tauri::command]
pub async fn parse_instructions_llm_cmd(
    instructions: String,
    mod_names: Vec<String>,
    model: String,
    platform: String,
    game_version: String,
) -> Result<Vec<instruction_types::ConditionalAction>, String> {
    llm_parser::parse_with_ollama(&model, &instructions, &mod_names, &platform, &game_version).await
}

/// Parse instructions using a cloud LLM (Tier 2a — cloud).
#[tauri::command]
pub async fn parse_instructions_cloud_cmd(
    instructions: String,
    mod_names: Vec<String>,
    provider: String,
    api_key: String,
    platform: String,
    game_version: String,
) -> Result<Vec<instruction_types::ConditionalAction>, String> {
    match provider.as_str() {
        "groq" => {
            llm_parser::parse_with_groq(
                &api_key,
                &instructions,
                &mod_names,
                &platform,
                &game_version,
            )
            .await
        }
        "cerebras" => {
            llm_parser::parse_with_cerebras(
                &api_key,
                &instructions,
                &mod_names,
                &platform,
                &game_version,
            )
            .await
        }
        "gemini" => {
            llm_parser::parse_with_gemini(
                &api_key,
                &instructions,
                &mod_names,
                &platform,
                &game_version,
            )
            .await
        }
        _ => Err(format!("Unknown cloud provider: {provider}")),
    }
}

/// Validate parsed actions against the actual installed mod list.
#[tauri::command]
pub async fn validate_instruction_actions_cmd(
    actions: Vec<instruction_types::ConditionalAction>,
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<instruction_types::ValidatedAction>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        Ok(instruction_validator::validate_actions(
            &actions,
            &db,
            &game_id,
            &bottle_name,
        ))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Check Ollama status (installed, running, available models).
#[tauri::command]
pub async fn check_ollama_status_cmd() -> Result<instruction_types::OllamaStatus, String> {
    Ok(llm_parser::check_ollama_status().await)
}

/// Get the list of recommended local models.
#[tauri::command]
pub fn get_recommended_models() -> Vec<instruction_types::OllamaModel> {
    instruction_types::recommended_models()
}

/// Get available cloud LLM providers.
#[tauri::command]
pub fn get_cloud_providers() -> Vec<llm_parser::CloudProvider> {
    llm_parser::available_cloud_providers()
}

/// Download (pull) a model via Ollama.
#[tauri::command]
pub async fn pull_ollama_model_cmd(model_name: String) -> Result<(), String> {
    llm_parser::pull_ollama_model(&model_name).await
}

/// Delete a model from Ollama (removes from disk).
#[tauri::command]
pub async fn delete_ollama_model_cmd(model_name: String) -> Result<(), String> {
    llm_parser::delete_ollama_model(&model_name).await
}

/// Unload a model from Ollama's memory (keeps on disk).
#[tauri::command]
pub async fn unload_ollama_model_cmd(model_name: String) -> Result<(), String> {
    llm_parser::unload_ollama_model(&model_name).await
}


// --- Modlist Export/Import ---

#[tauri::command]
pub async fn export_modlist_cmd(
    game_id: String,
    bottle_name: String,
    output_path: String,
    notes: Option<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        // Get current plugin order if applicable
        let plugin_entries = crate::get_current_plugins(&game_id, &bottle_name);

        let modlist = modlist_io::export_modlist(
            &db,
            &game_id,
            &bottle_name,
            &plugin_entries,
            notes.as_deref(),
        )
        .map_err(|e| e.to_string())?;

        let path = PathBuf::from(&output_path);
        modlist_io::write_modlist_file(&modlist, &path).map_err(|e| e.to_string())?;

        Ok(output_path)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn import_modlist_plan(
    file_path: String,
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<ImportPlan, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let modlist =
            modlist_io::read_modlist_file(Path::new(&file_path)).map_err(|e| e.to_string())?;
        modlist_io::validate_modlist(&modlist, &game_id).map_err(|e| e.to_string())?;

        modlist_io::plan_import(&db, &modlist, &game_id, &bottle_name).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn diff_modlists_cmd(
    file_path: String,
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<ModlistDiff, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let imported =
            modlist_io::read_modlist_file(Path::new(&file_path)).map_err(|e| e.to_string())?;

        let plugin_entries = crate::get_current_plugins(&game_id, &bottle_name);

        let current =
            modlist_io::export_modlist(&db, &game_id, &bottle_name, &plugin_entries, None)
                .map_err(|e| e.to_string())?;

        Ok(modlist_io::diff_modlists(&current, &imported))
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn execute_modlist_import(
    file_path: String,
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<modlist_io::ImportResult, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let imported =
            modlist_io::read_modlist_file(Path::new(&file_path)).map_err(|e| e.to_string())?;
        modlist_io::execute_import(&db, &imported, &game_id, &bottle_name)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}


// --- Vortex Extension Commands ---

#[tauri::command]
pub async fn vortex_fetch_extension(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<vortex_types::VortexGameRegistration, String> {
    let db = state.db.clone();
    vortex_registry::fetch_and_register(&db, &game_id).await
}

#[tauri::command]
pub async fn vortex_refresh_extension(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<vortex_types::VortexGameRegistration, String> {
    let db = state.db.clone();
    vortex_registry::refresh_extension(&db, &game_id).await
}

#[tauri::command]
pub async fn vortex_list_cached_extensions(
    state: State<'_, AppState>,
) -> Result<Vec<vortex_types::ExtensionSummary>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || Ok(vortex_registry::list_cached(&db)))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn vortex_list_available_extensions() -> Result<Vec<String>, String> {
    vortex_fetcher::list_available_extensions().await
}

#[tauri::command]
pub async fn vortex_delete_cached_extension(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        vortex_registry::delete_cached(&db, &game_id);
        Ok(())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn vortex_get_extension_detail(
    game_id: String,
    state: State<'_, AppState>,
) -> Result<Option<vortex_types::VortexGameRegistration>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || Ok(vortex_registry::load_cached(&db, &game_id)))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}


