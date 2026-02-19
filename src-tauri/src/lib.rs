pub mod bottles;
pub mod config;
pub mod database;
pub mod deployer;
pub mod executables;
pub mod fomod;
pub mod games;
pub mod installer;
pub mod integrity;
pub mod loot;
pub mod migrations;
pub mod nexus;
pub mod plugins;
pub mod profiles;
pub mod launcher;
pub mod staging;
pub mod skse;
pub mod downgrader;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tauri::State;

use bottles::Bottle;
use config::AppConfig;
use database::{DeploymentEntry, FileConflict, InstalledMod, ModDatabase};
use executables::CustomExecutable;
use fomod::FomodInstaller;
use games::DetectedGame;
use integrity::IntegrityReport;
use plugins::skyrim_plugins::PluginEntry;
use profiles::Profile;
use launcher::LaunchResult;
use nexus::ModUpdateInfo;
use skse::SkseStatus;
use downgrader::DowngradeStatus;
use loot::{PluginWarning, SortResult};

struct AppState {
    db: Mutex<ModDatabase>,
}

// --- Tauri Commands ---

#[tauri::command]
fn get_bottles() -> Result<Vec<Bottle>, String> {
    Ok(bottles::detect_bottles())
}

#[tauri::command]
fn get_games(bottle_name: Option<String>) -> Result<Vec<DetectedGame>, String> {
    match bottle_name {
        Some(name) => {
            let bottle = bottles::find_bottle_by_name(&name)
                .ok_or_else(|| format!("Bottle '{}' not found", name))?;
            Ok(games::detect_games(&bottle))
        }
        None => Ok(games::detect_all_games()),
    }
}

#[tauri::command]
fn get_all_games() -> Result<Vec<DetectedGame>, String> {
    Ok(games::detect_all_games())
}

#[tauri::command]
fn get_installed_mods(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<Vec<InstalledMod>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.list_mods(&game_id, &bottle_name).map_err(|e| e.to_string())
}

#[tauri::command]
fn install_mod_cmd(
    archive_path: String,
    game_id: String,
    bottle_name: String,
    mod_name: Option<String>,
    mod_version: Option<String>,
    state: State<AppState>,
) -> Result<InstalledMod, String> {
    let archive = PathBuf::from(&archive_path);
    if !archive.exists() {
        return Err(format!("Archive not found: {}", archive_path));
    }

    // Find the game
    let bottle = bottles::find_bottle_by_name(&bottle_name)
        .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))?;
    let detected_games = games::detect_games(&bottle);
    let game = detected_games
        .iter()
        .find(|g| g.game_id == game_id)
        .ok_or_else(|| format!("Game '{}' not found in bottle '{}'", game_id, bottle_name))?;

    let data_dir = PathBuf::from(&game.data_dir);
    let name = mod_name.unwrap_or_else(|| {
        archive
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Unknown Mod".to_string())
    });
    let version = mod_version.unwrap_or_default();

    let db = state.db.lock().map_err(|e| e.to_string())?;

    // 1. Reserve a DB record (empty files initially) and assign priority
    let next_priority = db.get_next_priority(&game_id, &bottle_name).map_err(|e| e.to_string())?;
    let mod_id = db
        .add_mod(&game_id, &bottle_name, None, &name, &version, &archive_path, &[])
        .map_err(|e| e.to_string())?;
    db.set_mod_priority(mod_id, next_priority).map_err(|e| e.to_string())?;

    // 2. Stage the mod (extract to staging folder, compute hashes)
    let staging_result = match staging::stage_mod(&archive, &game_id, &bottle_name, mod_id, &name) {
        Ok(r) => r,
        Err(e) => {
            let _ = db.remove_mod(mod_id);
            return Err(format!("Staging failed: {}", e));
        }
    };

    // 3. Update DB with staging info
    db.set_staging_path(mod_id, &staging_result.staging_path.to_string_lossy())
        .map_err(|e| e.to_string())?;
    db.update_installed_files(mod_id, &staging_result.files)
        .map_err(|e| e.to_string())?;
    db.store_file_hashes(mod_id, &staging_result.hashes)
        .map_err(|e| e.to_string())?;

    // 4. Deploy from staging to game dir via hardlink/copy
    if let Err(e) = deployer::deploy_mod(
        &db, &game_id, &bottle_name, mod_id,
        &staging_result.staging_path, &data_dir, &staging_result.files,
    ) {
        let _ = staging::remove_staging(&staging_result.staging_path);
        let _ = db.remove_mod(mod_id);
        return Err(format!("Deploy failed: {}", e));
    }

    // 5. Sync Skyrim plugins if applicable
    if game_id == "skyrimse" {
        let _ = sync_skyrim_plugins_for_game(game, &bottle);
    }

    db.get_mod(mod_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Failed to retrieve installed mod".to_string())
}

#[tauri::command]
fn uninstall_mod(
    mod_id: i64,
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<Vec<String>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;

    let installed_mod = db
        .get_mod(mod_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Mod with ID {} not found", mod_id))?;

    // Find the game to get the data dir
    let bottle = bottles::find_bottle_by_name(&bottle_name)
        .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))?;
    let detected_games = games::detect_games(&bottle);
    let game = detected_games
        .iter()
        .find(|g| g.game_id == game_id)
        .ok_or_else(|| format!("Game '{}' not found in bottle '{}'", game_id, bottle_name))?;

    let data_dir = PathBuf::from(&game.data_dir);

    // Remove deployed files from game directory
    let removed = if installed_mod.staging_path.is_some() {
        // Staged mod: undeploy via deployment manifest
        deployer::undeploy_mod(&db, &game_id, &bottle_name, mod_id, &data_dir)
            .map_err(|e| e.to_string())?
    } else {
        // Legacy mod: remove files directly
        installer::uninstall_mod_files(&data_dir, &installed_mod.installed_files)
            .map_err(|e| e.to_string())?
    };

    // Remove staging directory if it exists
    if let Some(ref staging_path) = installed_mod.staging_path {
        let _ = staging::remove_staging(Path::new(staging_path));
    }

    // Remove from database (cascades to deployment_manifest, file_hashes)
    db.remove_mod(mod_id).map_err(|e| e.to_string())?;

    // Sync Skyrim plugins if applicable
    if game_id == "skyrimse" {
        let _ = sync_skyrim_plugins_for_game(game, &bottle);
    }

    Ok(removed)
}

#[tauri::command]
fn toggle_mod(
    mod_id: i64,
    game_id: String,
    bottle_name: String,
    enabled: bool,
    state: State<AppState>,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;

    let installed_mod = db
        .get_mod(mod_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Mod with ID {} not found", mod_id))?;

    // Update DB flag
    db.set_enabled(mod_id, enabled).map_err(|e| e.to_string())?;

    // For staged mods, actually deploy/undeploy files
    if let Some(ref staging_path_str) = installed_mod.staging_path {
        let bottle = bottles::find_bottle_by_name(&bottle_name)
            .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))?;
        let detected_games = games::detect_games(&bottle);
        let game = detected_games
            .iter()
            .find(|g| g.game_id == game_id)
            .ok_or_else(|| format!("Game '{}' not found in bottle '{}'", game_id, bottle_name))?;

        let data_dir = PathBuf::from(&game.data_dir);
        let staging_path = PathBuf::from(staging_path_str);

        if enabled {
            // Re-deploy from staging
            let files = staging::list_staging_files(&staging_path)
                .map_err(|e| e.to_string())?;
            deployer::deploy_mod(
                &db, &game_id, &bottle_name, mod_id,
                &staging_path, &data_dir, &files,
            ).map_err(|e| e.to_string())?;
        } else {
            // Undeploy (remove from game dir, keep staging intact)
            deployer::undeploy_mod(&db, &game_id, &bottle_name, mod_id, &data_dir)
                .map_err(|e| e.to_string())?;
        }

        // Sync Skyrim plugins if applicable
        if game_id == "skyrimse" {
            let _ = sync_skyrim_plugins_for_game(game, &bottle);
        }
    }
    // Legacy mods (no staging_path): only the DB flag changes

    Ok(())
}

#[tauri::command]
fn get_plugin_order(game_id: String, bottle_name: String) -> Result<Vec<PluginEntry>, String> {
    if game_id != "skyrimse" {
        return Ok(vec![]);
    }

    let bottle = bottles::find_bottle_by_name(&bottle_name)
        .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))?;

    let detected_games = games::detect_games(&bottle);
    let game = detected_games
        .iter()
        .find(|g| g.game_id == game_id)
        .ok_or_else(|| format!("Game '{}' not found in bottle '{}'", game_id, bottle_name))?;

    // Get plugins file path via the plugin
    let plugins_file = games::with_plugin(&game_id, |plugin| {
        plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
    })
    .flatten()
    .ok_or_else(|| "Could not determine plugins file location".to_string())?;

    if !plugins_file.exists() {
        return Ok(vec![]);
    }

    plugins::skyrim_plugins::read_plugins_txt(&plugins_file).map_err(|e| e.to_string())
}

#[tauri::command]
async fn download_from_nexus(
    nxm_url: String,
    game_id: String,
    bottle_name: String,
    auto_install: bool,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let cfg = config::get_config().map_err(|e| e.to_string())?;
    let api_key = cfg
        .nexus_api_key
        .ok_or_else(|| "No Nexus API key configured. Set it in Settings.".to_string())?;

    let nxm = nexus::NXMLink::parse(&nxm_url).map_err(|e| e.to_string())?;

    let client = nexus::NexusClient::new(api_key);

    // Get mod info
    let mod_info = client
        .get_mod(&nxm.game_slug, nxm.mod_id)
        .await
        .map_err(|e| e.to_string())?;
    let mod_name = mod_info
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown Mod")
        .to_string();
    let mod_version = mod_info
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Download
    let download_dir = cfg
        .download_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| config::downloads_dir());

    let archive_path = client
        .download_from_nxm(&nxm, &download_dir, None::<Box<dyn Fn(u64, u64) + Send>>)
        .await
        .map_err(|e| e.to_string())?;

    if auto_install {
        let bottle = bottles::find_bottle_by_name(&bottle_name)
            .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))?;
        let detected_games = games::detect_games(&bottle);
        let game = detected_games
            .iter()
            .find(|g| g.game_id == game_id)
            .ok_or_else(|| {
                format!("Game '{}' not found in bottle '{}'", game_id, bottle_name)
            })?;

        let data_dir = PathBuf::from(&game.data_dir);
        let db = state.db.lock().map_err(|e| e.to_string())?;

        // 1. Add mod to DB with Nexus ID
        let next_priority = db.get_next_priority(&game_id, &bottle_name).map_err(|e| e.to_string())?;
        let mod_id = db
            .add_mod(
                &game_id,
                &bottle_name,
                Some(nxm.mod_id),
                &mod_name,
                &mod_version,
                &archive_path.to_string_lossy(),
                &[],
            )
            .map_err(|e| e.to_string())?;
        db.set_mod_priority(mod_id, next_priority).map_err(|e| e.to_string())?;

        // 2. Stage
        let staging_result = match staging::stage_mod(
            &archive_path, &game_id, &bottle_name, mod_id, &mod_name,
        ) {
            Ok(r) => r,
            Err(e) => {
                let _ = db.remove_mod(mod_id);
                return Err(format!("Staging failed: {}", e));
            }
        };

        // 3. Update DB
        db.set_staging_path(mod_id, &staging_result.staging_path.to_string_lossy())
            .map_err(|e| e.to_string())?;
        db.update_installed_files(mod_id, &staging_result.files)
            .map_err(|e| e.to_string())?;
        db.store_file_hashes(mod_id, &staging_result.hashes)
            .map_err(|e| e.to_string())?;

        // 4. Deploy
        if let Err(e) = deployer::deploy_mod(
            &db, &game_id, &bottle_name, mod_id,
            &staging_result.staging_path, &data_dir, &staging_result.files,
        ) {
            let _ = staging::remove_staging(&staging_result.staging_path);
            let _ = db.remove_mod(mod_id);
            return Err(format!("Deploy failed: {}", e));
        }

        if game_id == "skyrimse" {
            let _ = sync_skyrim_plugins_for_game(game, &bottle);
        }

        let installed = db
            .get_mod(mod_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "Failed to retrieve installed mod".to_string())?;

        Ok(serde_json::to_value(installed).map_err(|e| e.to_string())?)
    } else {
        Ok(serde_json::json!({
            "downloaded": archive_path.to_string_lossy(),
            "mod_name": mod_name,
            "mod_version": mod_version,
        }))
    }
}

#[tauri::command]
fn get_config() -> Result<AppConfig, String> {
    config::get_config().map_err(|e| e.to_string())
}

#[tauri::command]
fn set_config_value(key: String, value: String) -> Result<(), String> {
    config::set_config_value(&key, &value).map_err(|e| e.to_string())
}

#[tauri::command]
fn launch_game_cmd(
    game_id: String,
    bottle_name: String,
    use_skse: bool,
    state: State<AppState>,
) -> Result<LaunchResult, String> {
    let bottle = bottles::find_bottle_by_name(&bottle_name)
        .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))?;
    let detected_games = games::detect_games(&bottle);
    let game = detected_games
        .iter()
        .find(|g| g.game_id == game_id)
        .ok_or_else(|| format!("Game '{}' not found in bottle '{}'", game_id, bottle_name))?;

    let game_path = PathBuf::from(&game.game_path);

    // Check for a custom default executable first
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let custom_exe = executables::get_default_executable(&db, &game_id, &bottle_name)
        .unwrap_or(None);
    drop(db); // Release lock before launching

    if let Some(custom) = custom_exe {
        let exe_path = PathBuf::from(&custom.exe_path);
        let work_dir = custom.working_dir.as_deref().map(Path::new);

        log::info!(
            "launch_game_cmd: using custom exe '{}' at {}",
            custom.name, exe_path.display()
        );

        return launcher::launch_game(
            &bottle,
            &exe_path,
            work_dir.or(Some(&game_path)),
        )
        .map_err(|e| format!("Launch failed ({}): {}", bottle.source, e));
    }

    // Determine which built-in executable to launch
    let exe_name = if use_skse && game_id == "skyrimse" {
        "skse64_loader.exe".to_string()
    } else {
        games::with_plugin(&game_id, |plugin| {
            plugin.executables().first().map(|s| s.to_string()).unwrap_or_default()
        })
        .unwrap_or_default()
    };

    if exe_name.is_empty() {
        return Err(format!(
            "No executable configured for game '{}'. Cannot launch.",
            game_id
        ));
    }

    let exe_path = launcher::find_executable(&game_path, &exe_name)
        .ok_or_else(|| {
            if use_skse {
                format!(
                    "SKSE loader '{}' not found in {}. Is SKSE installed?",
                    exe_name, game_path.display()
                )
            } else {
                format!(
                    "Game executable '{}' not found in {}",
                    exe_name, game_path.display()
                )
            }
        })?;

    log::info!(
        "launch_game_cmd: source={} bottle={} exe={} use_skse={}",
        bottle.source, bottle.name, exe_path.display(), use_skse
    );

    launcher::launch_game(&bottle, &exe_path, Some(&game_path))
        .map_err(|e| format!("Launch failed ({}): {}", bottle.source, e))
}

#[tauri::command]
fn check_skse(game_id: String, bottle_name: String) -> Result<SkseStatus, String> {
    if game_id != "skyrimse" {
        return Ok(SkseStatus {
            installed: false,
            loader_path: None,
            version: None,
            use_skse: false,
        });
    }

    let bottle = bottles::find_bottle_by_name(&bottle_name)
        .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))?;
    let detected_games = games::detect_games(&bottle);
    let game = detected_games
        .iter()
        .find(|g| g.game_id == game_id)
        .ok_or_else(|| format!("Game '{}' not found in bottle '{}'", game_id, bottle_name))?;

    let game_path = PathBuf::from(&game.game_path);
    let mut status = skse::detect_skse(&game_path);
    status.use_skse = skse::get_skse_preference(&game_id, &bottle_name);

    Ok(status)
}

#[tauri::command]
async fn install_skse_cmd(
    game_id: String,
    bottle_name: String,
) -> Result<SkseStatus, String> {
    if game_id != "skyrimse" {
        return Err("SKSE is only available for Skyrim Special Edition".to_string());
    }

    let bottle = bottles::find_bottle_by_name(&bottle_name)
        .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))?;
    let detected_games = games::detect_games(&bottle);
    let game = detected_games
        .iter()
        .find(|g| g.game_id == game_id)
        .ok_or_else(|| format!("Game '{}' not found in bottle '{}'", game_id, bottle_name))?;

    let game_path = PathBuf::from(&game.game_path);
    let download_dir = config::get_config()
        .ok()
        .and_then(|c| c.download_dir.map(PathBuf::from))
        .unwrap_or_else(config::downloads_dir);

    let mut status = skse::download_and_install_skse(&game_path, &download_dir)
        .await
        .map_err(|e| e.to_string())?;

    // Auto-enable SKSE after successful installation
    if status.installed {
        let _ = skse::set_skse_preference(&game_id, &bottle_name, true);
        status.use_skse = true;
    }

    Ok(status)
}

#[tauri::command]
fn set_skse_preference_cmd(
    game_id: String,
    bottle_name: String,
    enabled: bool,
) -> Result<(), String> {
    skse::set_skse_preference(&game_id, &bottle_name, enabled)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn check_skyrim_version(game_id: String, bottle_name: String) -> Result<DowngradeStatus, String> {
    if game_id != "skyrimse" {
        return Err("Version check is only available for Skyrim SE".to_string());
    }

    let bottle = bottles::find_bottle_by_name(&bottle_name)
        .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))?;
    let detected_games = games::detect_games(&bottle);
    let game = detected_games
        .iter()
        .find(|g| g.game_id == game_id)
        .ok_or_else(|| format!("Game '{}' not found in bottle '{}'", game_id, bottle_name))?;

    let game_path = PathBuf::from(&game.game_path);
    downgrader::detect_skyrim_version(&game_path)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn downgrade_skyrim(
    game_id: String,
    bottle_name: String,
    _mode: String,
) -> Result<DowngradeStatus, String> {
    if game_id != "skyrimse" {
        return Err("Downgrade is only available for Skyrim SE".to_string());
    }

    let bottle = bottles::find_bottle_by_name(&bottle_name)
        .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))?;
    let detected_games = games::detect_games(&bottle);
    let game = detected_games
        .iter()
        .find(|g| g.game_id == game_id)
        .ok_or_else(|| format!("Game '{}' not found in bottle '{}'", game_id, bottle_name))?;

    let game_path = PathBuf::from(&game.game_path);
    let download_dir = config::get_config()
        .ok()
        .and_then(|c| c.download_dir.map(PathBuf::from))
        .unwrap_or_else(config::downloads_dir);

    // Create a downgrade copy of the game files
    let downgrade_dir = download_dir.parent().unwrap_or(&download_dir).join("downgraded_games");
    let downgrade_path = downgrader::create_downgrade_copy(&game_path, &downgrade_dir)
        .map_err(|e| e.to_string())?;

    // Store downgrade path in config
    let config_key = format!("downgrade:{}:{}", game_id, bottle_name);
    let _ = config::set_config_value(&config_key, &downgrade_path.to_string_lossy());

    // Return status (actual USSEDP patching is a future enhancement)
    downgrader::detect_skyrim_version(&downgrade_path)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn set_vibrancy(window: tauri::Window, material: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState};
        let mat = match material.as_str() {
            "sidebar" => NSVisualEffectMaterial::Sidebar,
            "underWindowBackground" => NSVisualEffectMaterial::UnderWindowBackground,
            "contentBackground" => NSVisualEffectMaterial::ContentBackground,
            "hudWindow" => NSVisualEffectMaterial::HudWindow,
            _ => NSVisualEffectMaterial::UnderWindowBackground,
        };
        apply_vibrancy(&window, mat, Some(NSVisualEffectState::FollowsWindowActiveState), None)
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, material);
    }
    Ok(())
}

// --- Custom Executables ---

#[tauri::command]
fn add_custom_exe(
    game_id: String,
    bottle_name: String,
    name: String,
    exe_path: String,
    working_dir: Option<String>,
    args: Option<String>,
    state: State<AppState>,
) -> Result<i64, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    executables::add_executable(
        &db,
        &game_id,
        &bottle_name,
        &name,
        &exe_path,
        working_dir.as_deref(),
        args.as_deref(),
    )
}

#[tauri::command]
fn remove_custom_exe(exe_id: i64, state: State<AppState>) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    executables::remove_executable(&db, exe_id)
}

#[tauri::command]
fn list_custom_exes(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<Vec<CustomExecutable>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    executables::list_executables(&db, &game_id, &bottle_name)
}

#[tauri::command]
fn set_default_exe(
    game_id: String,
    bottle_name: String,
    exe_id: Option<i64>,
    state: State<AppState>,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    match exe_id {
        Some(id) => executables::set_default_executable(&db, &game_id, &bottle_name, id),
        None => executables::clear_default_executable(&db, &game_id, &bottle_name),
    }
}

// --- Deployment Management ---

#[tauri::command]
fn get_conflicts(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<Vec<FileConflict>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.find_all_conflicts(&game_id, &bottle_name).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_deployment_manifest_cmd(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<Vec<DeploymentEntry>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_deployment_manifest(&game_id, &bottle_name).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_mod_priority(
    mod_id: i64,
    priority: i32,
    state: State<AppState>,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.set_mod_priority(mod_id, priority).map_err(|e| e.to_string())
}

#[tauri::command]
fn reorder_mods(
    game_id: String,
    bottle_name: String,
    ordered_mod_ids: Vec<i64>,
    state: State<AppState>,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.reorder_priorities(&game_id, &bottle_name, &ordered_mod_ids)
        .map_err(|e| e.to_string())?;

    // Find the game for data_dir
    let bottle = bottles::find_bottle_by_name(&bottle_name)
        .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))?;
    let detected_games = games::detect_games(&bottle);
    let game = detected_games
        .iter()
        .find(|g| g.game_id == game_id)
        .ok_or_else(|| format!("Game '{}' not found in bottle '{}'", game_id, bottle_name))?;

    let data_dir = PathBuf::from(&game.data_dir);

    // Redeploy to reflect new priority order
    deployer::redeploy_all(&db, &game_id, &bottle_name, &data_dir)
        .map_err(|e| e.to_string())?;

    // Sync plugins after redeploy
    if game_id == "skyrimse" {
        let _ = sync_skyrim_plugins_for_game(game, &bottle);
    }

    Ok(())
}

#[tauri::command]
fn redeploy_all_mods(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<serde_json::Value, String> {
    let bottle = bottles::find_bottle_by_name(&bottle_name)
        .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))?;
    let detected_games = games::detect_games(&bottle);
    let game = detected_games
        .iter()
        .find(|g| g.game_id == game_id)
        .ok_or_else(|| format!("Game '{}' not found in bottle '{}'", game_id, bottle_name))?;

    let data_dir = PathBuf::from(&game.data_dir);
    let db = state.db.lock().map_err(|e| e.to_string())?;

    let result = deployer::redeploy_all(&db, &game_id, &bottle_name, &data_dir)
        .map_err(|e| e.to_string())?;

    if game_id == "skyrimse" {
        let _ = sync_skyrim_plugins_for_game(game, &bottle);
    }

    Ok(serde_json::json!({
        "deployed_count": result.deployed_count,
        "skipped_count": result.skipped_count,
        "fallback_used": result.fallback_used,
    }))
}

#[tauri::command]
fn purge_deployment_cmd(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<Vec<String>, String> {
    let bottle = bottles::find_bottle_by_name(&bottle_name)
        .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))?;
    let detected_games = games::detect_games(&bottle);
    let game = detected_games
        .iter()
        .find(|g| g.game_id == game_id)
        .ok_or_else(|| format!("Game '{}' not found in bottle '{}'", game_id, bottle_name))?;

    let data_dir = PathBuf::from(&game.data_dir);
    let db = state.db.lock().map_err(|e| e.to_string())?;

    let removed = deployer::purge_deployment(&db, &game_id, &bottle_name, &data_dir)
        .map_err(|e| e.to_string())?;

    if game_id == "skyrimse" {
        let _ = sync_skyrim_plugins_for_game(game, &bottle);
    }

    Ok(removed)
}

#[tauri::command]
fn verify_mod_integrity(
    mod_id: i64,
    state: State<AppState>,
) -> Result<Vec<String>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;

    let installed_mod = db
        .get_mod(mod_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Mod with ID {} not found", mod_id))?;

    let staging_path = installed_mod
        .staging_path
        .as_ref()
        .ok_or_else(|| "Legacy mod — no staging data for integrity check".to_string())?;

    let hashes = db.get_file_hashes(mod_id).map_err(|e| e.to_string())?;
    staging::verify_staging_integrity(Path::new(staging_path), &hashes)
        .map_err(|e| e.to_string())
}

// --- LOOT & Plugin Management ---

#[tauri::command]
async fn sort_plugins_loot(
    game_id: String,
    bottle_name: String,
) -> Result<SortResult, String> {
    let bottle = bottles::find_bottle_by_name(&bottle_name)
        .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))?;
    let detected_games = games::detect_games(&bottle);
    let game = detected_games
        .iter()
        .find(|g| g.game_id == game_id)
        .ok_or_else(|| format!("Game '{}' not found in bottle '{}'", game_id, bottle_name))?;

    let game_path = PathBuf::from(&game.game_path);
    let data_dir = PathBuf::from(&game.data_dir);
    let local_path = loot::local_game_path(&bottle, &game_id)
        .ok_or_else(|| format!("Cannot determine local path for game '{}'", game_id))?;

    // Sort using LOOT
    let sort_result = loot::sort_plugins(&game_id, &game_path, &data_dir, &local_path)
        .map_err(|e| e.to_string())?;

    // Apply the sorted order to disk
    if sort_result.plugins_moved > 0 {
        let plugins_file = games::with_plugin(&game_id, |plugin| {
            plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
        })
        .flatten()
        .ok_or_else(|| "Could not determine plugins file location".to_string())?;

        let loadorder_file = plugins_file
            .parent()
            .map(|p| p.join("loadorder.txt"))
            .unwrap_or_else(|| plugins_file.with_file_name("loadorder.txt"));

        // Build PluginEntry list from sorted order, preserving enabled state
        let existing = if plugins_file.exists() {
            plugins::skyrim_plugins::read_plugins_txt(&plugins_file)
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let enabled_map: std::collections::HashMap<String, bool> = existing
            .iter()
            .map(|e| (e.filename.to_lowercase(), e.enabled))
            .collect();

        let ordered_entries: Vec<PluginEntry> = sort_result
            .sorted_order
            .iter()
            .map(|name| PluginEntry {
                filename: name.clone(),
                enabled: enabled_map
                    .get(&name.to_lowercase())
                    .copied()
                    .unwrap_or(false),
            })
            .collect();

        plugins::skyrim_plugins::apply_load_order(
            &plugins_file,
            &loadorder_file,
            &ordered_entries,
        )
        .map_err(|e| e.to_string())?;
    }

    Ok(sort_result)
}

#[tauri::command]
async fn update_loot_masterlist(game_id: String) -> Result<String, String> {
    loot::update_masterlist(&game_id)
        .await
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn reorder_plugins_cmd(
    game_id: String,
    bottle_name: String,
    ordered_plugins: Vec<String>,
) -> Result<Vec<PluginEntry>, String> {
    let bottle = bottles::find_bottle_by_name(&bottle_name)
        .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))?;
    let detected_games = games::detect_games(&bottle);
    let game = detected_games
        .iter()
        .find(|g| g.game_id == game_id)
        .ok_or_else(|| format!("Game '{}' not found in bottle '{}'", game_id, bottle_name))?;

    let plugins_file = games::with_plugin(&game_id, |plugin| {
        plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
    })
    .flatten()
    .ok_or_else(|| "Could not determine plugins file location".to_string())?;

    let loadorder_file = plugins_file
        .parent()
        .map(|p| p.join("loadorder.txt"))
        .unwrap_or_else(|| plugins_file.with_file_name("loadorder.txt"));

    plugins::skyrim_plugins::reorder_plugins(&plugins_file, &loadorder_file, &ordered_plugins)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn toggle_plugin_cmd(
    game_id: String,
    bottle_name: String,
    plugin_name: String,
    enabled: bool,
) -> Result<Vec<PluginEntry>, String> {
    let bottle = bottles::find_bottle_by_name(&bottle_name)
        .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))?;
    let detected_games = games::detect_games(&bottle);
    let game = detected_games
        .iter()
        .find(|g| g.game_id == game_id)
        .ok_or_else(|| format!("Game '{}' not found in bottle '{}'", game_id, bottle_name))?;

    let plugins_file = games::with_plugin(&game_id, |plugin| {
        plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
    })
    .flatten()
    .ok_or_else(|| "Could not determine plugins file location".to_string())?;

    let loadorder_file = plugins_file
        .parent()
        .map(|p| p.join("loadorder.txt"))
        .unwrap_or_else(|| plugins_file.with_file_name("loadorder.txt"));

    plugins::skyrim_plugins::toggle_plugin(
        &plugins_file,
        &loadorder_file,
        &plugin_name,
        enabled,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn move_plugin_cmd(
    game_id: String,
    bottle_name: String,
    plugin_name: String,
    new_index: usize,
) -> Result<Vec<PluginEntry>, String> {
    let bottle = bottles::find_bottle_by_name(&bottle_name)
        .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))?;
    let detected_games = games::detect_games(&bottle);
    let game = detected_games
        .iter()
        .find(|g| g.game_id == game_id)
        .ok_or_else(|| format!("Game '{}' not found in bottle '{}'", game_id, bottle_name))?;

    let plugins_file = games::with_plugin(&game_id, |plugin| {
        plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
    })
    .flatten()
    .ok_or_else(|| "Could not determine plugins file location".to_string())?;

    let loadorder_file = plugins_file
        .parent()
        .map(|p| p.join("loadorder.txt"))
        .unwrap_or_else(|| plugins_file.with_file_name("loadorder.txt"));

    plugins::skyrim_plugins::move_plugin(
        &plugins_file,
        &loadorder_file,
        &plugin_name,
        new_index,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_plugin_messages(
    game_id: String,
    bottle_name: String,
    plugin_name: String,
) -> Result<Vec<PluginWarning>, String> {
    let bottle = bottles::find_bottle_by_name(&bottle_name)
        .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))?;
    let detected_games = games::detect_games(&bottle);
    let game = detected_games
        .iter()
        .find(|g| g.game_id == game_id)
        .ok_or_else(|| format!("Game '{}' not found in bottle '{}'", game_id, bottle_name))?;

    let game_path = PathBuf::from(&game.game_path);
    let data_dir = PathBuf::from(&game.data_dir);
    let local_path = loot::local_game_path(&bottle, &game_id)
        .ok_or_else(|| format!("Cannot determine local path for game '{}'", game_id))?;

    loot::get_plugin_messages(&game_id, &game_path, &data_dir, &local_path, &plugin_name)
        .map_err(|e| e.to_string())
}

// --- Profiles ---

#[tauri::command]
fn list_profiles_cmd(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<Vec<Profile>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    profiles::list_profiles(&db, &game_id, &bottle_name).map_err(|e| e.to_string())
}

#[tauri::command]
fn create_profile_cmd(
    game_id: String,
    bottle_name: String,
    name: String,
    state: State<AppState>,
) -> Result<i64, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    profiles::create_profile(&db, &game_id, &bottle_name, &name).map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_profile_cmd(
    profile_id: i64,
    state: State<AppState>,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    profiles::delete_profile(&db, profile_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn rename_profile_cmd(
    profile_id: i64,
    new_name: String,
    state: State<AppState>,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    profiles::rename_profile(&db, profile_id, &new_name).map_err(|e| e.to_string())
}

#[tauri::command]
fn save_profile_snapshot(
    profile_id: i64,
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;

    // Determine plugins file path
    let plugins_file = if game_id == "skyrimse" {
        let bottle = bottles::find_bottle_by_name(&bottle_name)
            .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))?;
        let detected_games = games::detect_games(&bottle);
        let game = detected_games
            .iter()
            .find(|g| g.game_id == game_id)
            .ok_or_else(|| format!("Game '{}' not found", game_id))?;

        games::with_plugin(&game_id, |plugin| {
            plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
        })
        .flatten()
    } else {
        None
    };

    profiles::snapshot_current_state(
        &db,
        profile_id,
        &game_id,
        &bottle_name,
        plugins_file.as_deref(),
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn activate_profile(
    profile_id: i64,
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;

    // Look up the game
    let bottle = bottles::find_bottle_by_name(&bottle_name)
        .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))?;
    let detected_games = games::detect_games(&bottle);
    let game = detected_games
        .iter()
        .find(|g| g.game_id == game_id)
        .ok_or_else(|| format!("Game '{}' not found in bottle '{}'", game_id, bottle_name))?;

    let data_dir = PathBuf::from(&game.data_dir);

    // 1. Save current state to the currently active profile (if any)
    if let Ok(Some(current_active)) = profiles::get_active_profile(&db, &game_id, &bottle_name) {
        let plugins_file = if game_id == "skyrimse" {
            games::with_plugin(&game_id, |plugin| {
                plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
            })
            .flatten()
        } else {
            None
        };

        let _ = profiles::snapshot_current_state(
            &db,
            current_active.id,
            &game_id,
            &bottle_name,
            plugins_file.as_deref(),
        );
    }

    // 2. Purge current deployment
    let _ = deployer::purge_deployment(&db, &game_id, &bottle_name, &data_dir);

    // 3. Load target profile state
    let mod_states = profiles::get_mod_states(&db, profile_id)
        .map_err(|e| e.to_string())?;

    // 4. Apply mod enabled states and priorities
    for ms in &mod_states {
        let _ = db.set_enabled(ms.mod_id, ms.enabled);
        let _ = db.set_mod_priority(ms.mod_id, ms.priority);
    }

    // 5. Redeploy enabled mods
    let _ = deployer::redeploy_all(&db, &game_id, &bottle_name, &data_dir);

    // 6. Apply plugin states
    let plugin_states = profiles::get_plugin_states(&db, profile_id)
        .map_err(|e| e.to_string())?;

    if !plugin_states.is_empty() && game_id == "skyrimse" {
        let plugins_file = games::with_plugin(&game_id, |plugin| {
            plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
        })
        .flatten();

        if let Some(pf) = plugins_file {
            let loadorder_file = pf
                .parent()
                .map(|p| p.join("loadorder.txt"))
                .unwrap_or_else(|| pf.with_file_name("loadorder.txt"));

            let entries: Vec<PluginEntry> = plugin_states
                .iter()
                .map(|ps| PluginEntry {
                    filename: ps.plugin_filename.clone(),
                    enabled: ps.enabled,
                })
                .collect();

            let _ = plugins::skyrim_plugins::apply_load_order(&pf, &loadorder_file, &entries);
        }
    }

    // 7. Mark profile as active
    profiles::set_active_profile(&db, &game_id, &bottle_name, profile_id)
        .map_err(|e| e.to_string())?;

    Ok(())
}

// --- Update Checking ---

#[tauri::command]
async fn check_mod_updates(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<ModUpdateInfo>, String> {
    let cfg = config::get_config().map_err(|e| e.to_string())?;
    let api_key = cfg
        .nexus_api_key
        .ok_or_else(|| "No Nexus API key configured".to_string())?;

    let mods = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.list_mods(&game_id, &bottle_name).map_err(|e| e.to_string())?
    };

    // Build query list from mods that have a nexus_mod_id
    let queries: Vec<nexus::ModUpdateQuery> = mods
        .iter()
        .filter_map(|m| {
            m.nexus_mod_id.map(|nid| nexus::ModUpdateQuery {
                local_mod_id: m.id,
                nexus_mod_id: nid,
                nexus_file_id: m.nexus_file_id,
                mod_name: m.name.clone(),
                current_version: m.version.clone(),
            })
        })
        .collect();

    if queries.is_empty() {
        return Ok(vec![]);
    }

    // Determine game slug from game_id
    let game_slug = match game_id.as_str() {
        "skyrimse" => "skyrimspecialedition",
        other => other,
    };

    let client = nexus::NexusClient::new(api_key);
    client
        .check_updates(game_slug, &queries)
        .await
        .map_err(|e| e.to_string())
}

// --- FOMOD ---

#[tauri::command]
fn detect_fomod(staging_path: String) -> Result<Option<FomodInstaller>, String> {
    let path = PathBuf::from(&staging_path);
    fomod::parse_fomod(&path).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_fomod_defaults(installer: FomodInstaller) -> Result<std::collections::HashMap<String, Vec<String>>, String> {
    Ok(fomod::get_default_selections(&installer))
}

#[tauri::command]
fn get_fomod_files(
    installer: FomodInstaller,
    selections: std::collections::HashMap<String, Vec<String>>,
) -> Result<Vec<fomod::FomodFile>, String> {
    Ok(fomod::get_files_for_selections(&installer, &selections))
}

// --- Integrity ---

#[tauri::command]
fn create_game_snapshot(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<usize, String> {
    let bottle = bottles::find_bottle_by_name(&bottle_name)
        .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))?;
    let detected_games = games::detect_games(&bottle);
    let game = detected_games
        .iter()
        .find(|g| g.game_id == game_id)
        .ok_or_else(|| format!("Game '{}' not found in bottle '{}'", game_id, bottle_name))?;

    let data_dir = PathBuf::from(&game.data_dir);
    let db = state.db.lock().map_err(|e| e.to_string())?;

    integrity::create_game_snapshot(&db, &game_id, &bottle_name, &data_dir)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn check_game_integrity(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<IntegrityReport, String> {
    let bottle = bottles::find_bottle_by_name(&bottle_name)
        .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))?;
    let detected_games = games::detect_games(&bottle);
    let game = detected_games
        .iter()
        .find(|g| g.game_id == game_id)
        .ok_or_else(|| format!("Game '{}' not found in bottle '{}'", game_id, bottle_name))?;

    let data_dir = PathBuf::from(&game.data_dir);
    let db = state.db.lock().map_err(|e| e.to_string())?;

    integrity::check_game_integrity(&db, &game_id, &bottle_name, &data_dir)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn has_game_snapshot(
    game_id: String,
    bottle_name: String,
    state: State<AppState>,
) -> Result<bool, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    integrity::has_snapshot(&db, &game_id, &bottle_name).map_err(|e| e.to_string())
}

// --- Helpers ---

fn sync_skyrim_plugins_for_game(game: &DetectedGame, bottle: &Bottle) -> Result<(), String> {
    let game_path = Path::new(&game.game_path);
    let data_dir = Path::new(&game.data_dir);

    let plugins_file = games::with_plugin(&game.game_id, |plugin| {
        plugin.get_plugins_file(game_path, bottle)
    })
    .flatten();

    if let Some(pf) = plugins_file {
        // Derive loadorder.txt path from plugins.txt path
        let loadorder_file = pf
            .parent()
            .map(|p| p.join("loadorder.txt"))
            .unwrap_or_else(|| pf.with_file_name("loadorder.txt"));
        plugins::skyrim_plugins::sync_plugins(data_dir, &pf, &loadorder_file)
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

// --- App Entry Point ---

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Register game plugins
    plugins::skyrim_se::register();

    // Initialize database
    let db_path = config::db_path();
    let db = ModDatabase::new(&db_path).expect("Failed to initialize mod database");

    // Initialize additional schemas
    executables::init_schema(&db).expect("Failed to initialize executables schema");
    profiles::init_schema(&db).expect("Failed to initialize profiles schema");
    integrity::init_schema(&db).expect("Failed to initialize integrity schema");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState { db: Mutex::new(db) })
        .invoke_handler(tauri::generate_handler![
            get_bottles,
            get_games,
            get_all_games,
            get_installed_mods,
            install_mod_cmd,
            uninstall_mod,
            toggle_mod,
            get_plugin_order,
            download_from_nexus,
            get_config,
            set_config_value,
            launch_game_cmd,
            check_skse,
            install_skse_cmd,
            set_skse_preference_cmd,
            check_skyrim_version,
            downgrade_skyrim,
            set_vibrancy,
            add_custom_exe,
            remove_custom_exe,
            list_custom_exes,
            set_default_exe,
            get_conflicts,
            get_deployment_manifest_cmd,
            set_mod_priority,
            reorder_mods,
            redeploy_all_mods,
            purge_deployment_cmd,
            verify_mod_integrity,
            sort_plugins_loot,
            update_loot_masterlist,
            reorder_plugins_cmd,
            toggle_plugin_cmd,
            move_plugin_cmd,
            get_plugin_messages,
            list_profiles_cmd,
            create_profile_cmd,
            delete_profile_cmd,
            rename_profile_cmd,
            save_profile_snapshot,
            activate_profile,
            check_mod_updates,
            detect_fomod,
            get_fomod_defaults,
            get_fomod_files,
            create_game_snapshot,
            check_game_integrity,
            has_game_snapshot,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
