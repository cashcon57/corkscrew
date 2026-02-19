pub mod bottles;
pub mod config;
pub mod database;
pub mod fomod;
pub mod games;
pub mod installer;
pub mod nexus;
pub mod plugins;
pub mod launcher;
pub mod skse;
pub mod downgrader;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tauri::State;

use bottles::Bottle;
use config::AppConfig;
use database::{InstalledMod, ModDatabase};
use games::DetectedGame;
use plugins::skyrim_plugins::PluginEntry;
use launcher::LaunchResult;
use skse::SkseStatus;
use downgrader::DowngradeStatus;

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

    // Install files
    let installed_files = installer::install_mod(&archive, &data_dir, &name, &version, None)
        .map_err(|e| e.to_string())?;

    // Record in database
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let mod_id = db
        .add_mod(&game_id, &bottle_name, None, &name, &version, &archive_path, &installed_files)
        .map_err(|e| e.to_string())?;

    // Sync Skyrim plugins if applicable
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

    // Remove files from disk
    let removed = installer::uninstall_mod_files(&data_dir, &installed_mod.installed_files)
        .map_err(|e| e.to_string())?;

    // Remove from database
    db.remove_mod(mod_id).map_err(|e| e.to_string())?;

    // Sync Skyrim plugins if applicable
    if game_id == "skyrimse" {
        let _ = sync_skyrim_plugins_for_game(game, &bottle);
    }

    Ok(removed)
}

#[tauri::command]
fn toggle_mod(mod_id: i64, enabled: bool, state: State<AppState>) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.set_enabled(mod_id, enabled).map_err(|e| e.to_string())
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
        let installed_files =
            installer::install_mod(&archive_path, &data_dir, &mod_name, &mod_version, Some(nxm.mod_id))
                .map_err(|e| e.to_string())?;

        let db = state.db.lock().map_err(|e| e.to_string())?;
        let mod_id = db
            .add_mod(
                &game_id,
                &bottle_name,
                Some(nxm.mod_id),
                &mod_name,
                &mod_version,
                &archive_path.to_string_lossy(),
                &installed_files,
            )
            .map_err(|e| e.to_string())?;

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
) -> Result<LaunchResult, String> {
    let bottle = bottles::find_bottle_by_name(&bottle_name)
        .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))?;
    let detected_games = games::detect_games(&bottle);
    let game = detected_games
        .iter()
        .find(|g| g.game_id == game_id)
        .ok_or_else(|| format!("Game '{}' not found in bottle '{}'", game_id, bottle_name))?;

    let game_path = PathBuf::from(&game.game_path);

    // Determine which executable to launch
    let exe_name = if use_skse && game_id == "skyrimse" {
        "skse64_loader.exe".to_string()
    } else {
        games::with_plugin(&game_id, |plugin| {
            plugin.executables().first().map(|s| s.to_string()).unwrap_or_default()
        })
        .unwrap_or_default()
    };

    let exe_path = launcher::find_executable(&game_path, &exe_name)
        .ok_or_else(|| format!("Executable '{}' not found in {}", exe_name, game_path.display()))?;

    launcher::launch_game(&bottle, &exe_path, Some(&game_path))
        .map_err(|e| e.to_string())
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

    // Create stock game copy first
    let stock_dir = download_dir.parent().unwrap_or(&download_dir).join("stock_games");
    let stock_game_path = downgrader::create_stock_game(&game_path, &stock_dir)
        .map_err(|e| e.to_string())?;

    // Store stock game path in config
    let config_key = format!("stock_game:{}:{}", game_id, bottle_name);
    let _ = config::set_config_value(&config_key, &stock_game_path.to_string_lossy());

    // Return status (actual USSEDP patching is a future enhancement)
    downgrader::detect_skyrim_version(&stock_game_path)
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
