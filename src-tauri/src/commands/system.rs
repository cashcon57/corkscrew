use crate::bottles;
use crate::fomod;
use crate::modlist_io;
use crate::nexus;
use crate::plugins;
use crate::rollback;
use crate::cleaner;
use crate::config;
use crate::database;
use crate::deck;
use crate::deploy_journal;
use crate::deployer;
use crate::disk_budget;
use crate::download_queue;
use crate::fomod::{FomodInstaller};
use crate::fomod_recipes;
use crate::game_lock;
use crate::games;
use crate::ini_manager;
use crate::instruction_parser;
use crate::instruction_types;
use crate::instruction_validator;
use crate::llm_chat;
use crate::llm_parser;
use crate::modlist_io::{ImportPlan, ModlistDiff};
use crate::nexus::{ModUpdateInfo};
use crate::proton;
use crate::rollback::{ModSnapshot, ModVersion};
use crate::staging;
use crate::steam_integration;
use crate::vortex_fetcher;
use crate::vortex_registry;
use crate::vortex_types;
use crate::{AppState, auto_snapshot_before_destructive, nexus_client, resolve_bottle, resolve_game};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::Manager;
use tauri::{AppHandle, State};

// --- System Info ---

/// Get total system memory in bytes (unified memory on Apple Silicon).
#[tauri::command]
pub fn get_system_memory() -> Result<u64, String> {
    #[cfg(target_os = "macos")]
    {
        unsafe {
            let mut size: u64 = 0;
            let mut len = std::mem::size_of::<u64>();
            let mib = [libc::CTL_HW, libc::HW_MEMSIZE];
            let ret = libc::sysctl(
                mib.as_ptr() as *mut _,
                2,
                &mut size as *mut u64 as *mut _,
                &mut len,
                std::ptr::null_mut(),
                0,
            );
            if ret == 0 {
                Ok(size)
            } else {
                Err("Failed to query system memory".into())
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        use std::fs;
        let meminfo = fs::read_to_string("/proc/meminfo")
            .map_err(|e| format!("Failed to read /proc/meminfo: {e}"))?;
        for line in meminfo.lines() {
            if line.starts_with("MemTotal:") {
                let kb: u64 = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|s| s.parse().ok())
                    .ok_or("Failed to parse MemTotal")?;
                return Ok(kb * 1024);
            }
        }
        Err("MemTotal not found in /proc/meminfo".into())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err("System memory detection not supported on this platform".into())
    }
}

/// Install Ollama. On Linux, runs the official install script.
/// On macOS, opens the download page (Ollama ships as a .app bundle).
#[tauri::command]
pub async fn install_ollama() -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        // Download the install script to a temp file first, then execute it.
        // This avoids piping arbitrary remote content directly into a shell.
        let tmp_dir = std::env::temp_dir();
        let script_path = tmp_dir.join("ollama-install.sh");

        let download = tokio::process::Command::new("curl")
            .args(["-fsSL", "--max-time", "30", "-o"])
            .arg(&script_path)
            .arg("https://ollama.com/install.sh")
            .output()
            .await
            .map_err(|e| format!("Failed to download install script: {e}"))?;

        if !download.status.success() {
            let stderr = String::from_utf8_lossy(&download.stderr);
            return Err(format!("Failed to download Ollama installer: {stderr}"));
        }

        let output = tokio::process::Command::new("sh")
            .arg(&script_path)
            .output()
            .await
            .map_err(|e| format!("Failed to run install script: {e}"))?;

        // Clean up the script regardless of outcome
        let _ = tokio::fs::remove_file(&script_path).await;

        if output.status.success() {
            Ok("Ollama installed successfully. It should now be running.".into())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("Install failed: {stderr}"))
        }
    }
    #[cfg(target_os = "macos")]
    {
        // macOS: Ollama is a native .app — open the download page
        let _ = std::process::Command::new("open")
            .arg("https://ollama.com/download/mac")
            .spawn();
        Ok("Opening Ollama download page. Install the app, then return here.".into())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err("Automatic Ollama install not supported on this platform".into())
    }
}

/// Start Ollama headlessly (serve mode) if not already running.
#[tauri::command]
pub async fn start_ollama() -> Result<String, String> {
    // Check if already running
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .map_err(|e| e.to_string())?;

    if let Ok(resp) = client.get("http://localhost:11434/api/tags").send().await {
        if resp.status().is_success() {
            return Ok("Ollama is already running.".into());
        }
    }

    // Try to start ollama serve in background
    #[cfg(target_os = "macos")]
    {
        // Try CLI first (if installed via homebrew or ollama cli is in PATH)
        let result = tokio::process::Command::new("ollama")
            .arg("serve")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();

        if result.is_err() {
            // Fallback: try launching the .app which also starts the server
            let _ = std::process::Command::new("open")
                .arg("-a")
                .arg("Ollama")
                .arg("--background")
                .spawn();
        }
    }

    #[cfg(target_os = "linux")]
    {
        let _ = tokio::process::Command::new("ollama")
            .arg("serve")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }

    // Wait briefly for it to start
    for _ in 0..10 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        if let Ok(resp) = client.get("http://localhost:11434/api/tags").send().await {
            if resp.status().is_success() {
                return Ok("Ollama started successfully.".into());
            }
        }
    }

    Err("Could not start Ollama. Please install and launch it manually.".into())
}

/// Check if MLX LM (Apple's MLX inference library) is installed.
#[tauri::command]
pub async fn check_mlx_status() -> Result<bool, String> {
    Ok(llm_chat::check_mlx_status().await)
}

/// Install MLX LM into a dedicated venv (~/.corkscrew/mlx-venv/).
#[tauri::command]
pub async fn install_mlx() -> Result<String, String> {
    llm_chat::install_mlx().await
}

/// Get the recommended model name based on system memory.
#[tauri::command]
pub fn get_recommended_model() -> Result<String, String> {
    let mem = get_system_memory()?;
    Ok(instruction_types::recommended_model_for_memory(mem))
}


// --- Platform Detection ---

#[derive(Clone, Debug, serde::Serialize)]
pub struct PlatformInfo {
    os: String,
    is_steam_os: bool,
    cpu_cores: usize,
    cpu_brand: String,
    memory_gb: u64,
    arch: String,
}

#[cfg(target_os = "macos")]
pub fn get_sysctl_string(name: &str) -> String {
    use std::ffi::CString;
    let cname = match CString::new(name) {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    let mut size: libc::size_t = 0;
    unsafe {
        if libc::sysctlbyname(
            cname.as_ptr(),
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        ) != 0
        {
            return String::new();
        }
        let mut buf = vec![0u8; size];
        if libc::sysctlbyname(
            cname.as_ptr(),
            buf.as_mut_ptr() as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        ) != 0
        {
            return String::new();
        }
        // Remove trailing null
        if let Some(pos) = buf.iter().position(|&b| b == 0) {
            buf.truncate(pos);
        }
        String::from_utf8_lossy(&buf).to_string()
    }
}

#[cfg(target_os = "macos")]
pub fn get_sysctl_u64(name: &str) -> u64 {
    use std::ffi::CString;
    let cname = match CString::new(name) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    let mut val: u64 = 0;
    let mut size = std::mem::size_of::<u64>() as libc::size_t;
    unsafe {
        if libc::sysctlbyname(
            cname.as_ptr(),
            &mut val as *mut u64 as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        ) != 0
        {
            return 0;
        }
    }
    val
}

#[tauri::command]
pub fn get_platform_detail() -> PlatformInfo {
    let os = std::env::consts::OS.to_string();
    let is_steam_os = if cfg!(target_os = "linux") {
        std::path::Path::new("/etc/steamos-release").exists() || std::env::var("SteamOS").is_ok()
    } else {
        false
    };

    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    let arch = std::env::consts::ARCH.to_string();

    #[cfg(target_os = "macos")]
    let (cpu_brand, memory_gb) = {
        let brand = get_sysctl_string("machdep.cpu.brand_string");
        let mem_bytes = get_sysctl_u64("hw.memsize");
        (brand, mem_bytes / (1024 * 1024 * 1024))
    };

    #[cfg(target_os = "linux")]
    let (cpu_brand, memory_gb) = {
        let brand = std::fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("model name"))
                    .and_then(|l| l.split(':').nth(1))
                    .map(|s| s.trim().to_string())
            })
            .unwrap_or_default();
        let mem = std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("MemTotal"))
                    .and_then(|l| l.split_whitespace().nth(1))
                    .and_then(|v| v.parse::<u64>().ok())
            })
            .unwrap_or(0)
            / (1024 * 1024); // kB → GB
        (brand, mem)
    };

    PlatformInfo {
        os,
        is_steam_os,
        cpu_cores,
        cpu_brand,
        memory_gb,
        arch,
    }
}

#[tauri::command]
pub fn get_optimal_download_threads() -> usize {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    let is_apple_silicon = cfg!(target_arch = "aarch64") && cfg!(target_os = "macos");
    let is_steam_os = std::path::Path::new("/etc/steamos-release").exists();

    if is_steam_os {
        cores.min(4)
    } else if is_apple_silicon {
        (cores / 2).clamp(4, 8)
    } else {
        (cores / 2).clamp(3, 6)
    }
}


// --- Steam Integration ---

#[tauri::command]
pub async fn detect_steam() -> Option<steam_integration::SteamInfo> {
    tokio::task::spawn_blocking(move || steam_integration::detect_steam_installation())
        .await
        .ok()
        .flatten()
}

#[tauri::command]
pub async fn check_steam_status() -> Result<steam_integration::SteamStatus, String> {
    tokio::task::spawn_blocking(move || steam_integration::get_steam_status())
        .await
        .map_err(|e| format!("Task failed: {e}"))
}

#[tauri::command]
pub async fn add_to_steam() -> Result<steam_integration::SteamStatus, String> {
    tokio::task::spawn_blocking(move || {
        steam_integration::setup_steam_integration().map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn remove_from_steam() -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let info = steam_integration::detect_steam_installation()
            .ok_or_else(|| "Steam not found".to_string())?;
        steam_integration::remove_from_steam(&info).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub fn is_steam_deck() -> bool {
    steam_integration::is_steam_deck()
}

#[tauri::command]
pub async fn steam_deck_warnings() -> Result<Vec<String>, String> {
    tokio::task::spawn_blocking(move || steam_integration::steam_deck_warnings())
        .await
        .map_err(|e| format!("Task failed: {e}"))
}


// --- Proton Detection ---

#[tauri::command]
pub async fn list_proton_versions() -> Result<Vec<proton::ProtonVersion>, String> {
    tokio::task::spawn_blocking(move || Ok(proton::detect_proton_versions()))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn get_recommended_proton() -> Result<Option<proton::ProtonVersion>, String> {
    tokio::task::spawn_blocking(move || Ok(proton::get_recommended_proton()))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}


// --- Steam Deck Profile ---

#[tauri::command]
pub async fn get_deck_profile() -> Result<deck::DeckProfile, String> {
    Ok(deck::detect_deck_profile())
}

#[tauri::command]
pub async fn get_deck_defaults() -> Result<deck::DeckDefaults, String> {
    Ok(deck::get_defaults())
}


// --- DLC Detection ---

/// Known Skyrim SE framework mods and their NexusMods IDs, used by
/// `get_mod_requirements` to identify dependencies from mod descriptions.
pub const KNOWN_FRAMEWORKS: &[(&str, i64)] = &[
    ("SKSE64", 30379),
    ("SkyUI", 12604),
    ("Address Library for SKSE Plugins", 32444),
    ("powerofthree's Tweaks", 51073),
    ("PapyrusUtil SE", 13048),
    ("JContainers SE", 16495),
    ("ConsoleUtilSSE", 24858),
    ("FileAccess Interface for Skyrim SE", 13956),
    ("MCM Helper", 53000),
    ("Keyword Item Distributor", 55728),
    ("Spell Perk Item Distributor", 36869),
    ("Base Object Swapper", 60805),
    ("Sound Record Distributor", 77815),
    ("USSEP", 266),
    ("RaceMenu", 19080),
    ("Nemesis", 60033),
    ("FNIS", 3038),
    ("DAR - Dynamic Animation Replacer", 33746),
    ("OAR - Open Animation Replacer", 92109),
    ("CBBE", 198),
    ("XP32 Maximum Skeleton", 1988),
];

const SKYRIM_SE_DLC_FILES: &[(&str, &str)] = &[
    ("Dawnguard.esm", "Dawnguard"),
    ("HearthFires.esm", "Hearthfire"),
    ("Dragonborn.esm", "Dragonborn"),
];

/// Expected DLC files for Fallout 4.
const FALLOUT4_DLC_FILES: &[(&str, &str)] = &[
    ("DLCRobot.esm", "Automatron"),
    ("DLCworkshop01.esm", "Wasteland Workshop"),
    ("DLCworkshop02.esm", "Contraptions Workshop"),
    ("DLCworkshop03.esm", "Vault-Tec Workshop"),
    ("DLCCoast.esm", "Far Harbor"),
    ("DLCNukaWorld.esm", "Nuka-World"),
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DlcStatus {
    /// Whether all expected DLC files are present.
    all_present: bool,
    /// Per-DLC detection results.
    dlcs: Vec<DlcInfo>,
    /// Whether the game has been initialized (base game ESM exists).
    game_initialized: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DlcInfo {
    /// DLC name (e.g., "Dawnguard", "Dragonborn").
    name: String,
    /// Whether all files for this DLC are present.
    present: bool,
    /// Files that are missing.
    missing_files: Vec<String>,
}

#[tauri::command]
pub async fn check_dlc_status(game_id: String, bottle_name: String) -> Result<DlcStatus, String> {
    tokio::task::spawn_blocking(move || {
        let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;

        let dlc_files: &[(&str, &str)] = match game_id.as_str() {
            "skyrimse" => SKYRIM_SE_DLC_FILES,
            "fallout4" => FALLOUT4_DLC_FILES,
            _ => {
                return Ok(DlcStatus {
                    all_present: true,
                    dlcs: vec![],
                    game_initialized: true,
                })
            }
        };

        // Check if base game is initialized
        let base_esm = match game_id.as_str() {
            "skyrimse" => "Skyrim.esm",
            "fallout4" => "Fallout4.esm",
            _ => "",
        };
        let game_initialized = if base_esm.is_empty() {
            true
        } else {
            data_dir.join(base_esm).exists()
        };

        // Group by DLC name and check each file
        let mut dlc_map: std::collections::BTreeMap<String, Vec<(String, bool)>> =
            std::collections::BTreeMap::new();
        for (filename, dlc_name) in dlc_files {
            let present = data_dir.join(filename).exists();
            dlc_map
                .entry(dlc_name.to_string())
                .or_default()
                .push((filename.to_string(), present));
        }

        let mut dlcs = Vec::new();
        let mut all_present = true;
        for (name, files) in &dlc_map {
            let missing: Vec<String> = files
                .iter()
                .filter(|(_, p)| !p)
                .map(|(f, _)| f.clone())
                .collect();
            let present = missing.is_empty();
            if !present {
                all_present = false;
            }
            dlcs.push(DlcInfo {
                name: name.clone(),
                present,
                missing_files: missing,
            });
        }

        Ok(DlcStatus {
            all_present,
            dlcs,
            game_initialized,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}


// --- Game Lock Commands ---

#[tauri::command]
pub async fn get_game_lock_status(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Option<game_lock::GameLock>, String> {
    Ok(state.game_locks.get(&game_id, &bottle_name))
}

#[tauri::command]
pub async fn get_all_game_locks(
    state: State<'_, AppState>,
) -> Result<Vec<game_lock::GameLock>, String> {
    Ok(state.game_locks.all_locks())
}

#[tauri::command]
pub async fn force_unlock_game(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    Ok(state.game_locks.force_unlock(&game_id, &bottle_name))
}


// --- Deploy Journal Commands ---

#[tauri::command]
pub async fn get_deploy_journal_status() -> Result<Vec<deploy_journal::JournalEntry>, String> {
    Ok(deploy_journal::get_incomplete())
}

#[tauri::command]
pub async fn heal_deployment(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let bottle = bottles::find_bottle_by_name(&bottle_name)
            .ok_or_else(|| format!("Bottle '{}' not found", bottle_name))?;
        let game = games::detect_games(&bottle)
            .into_iter()
            .find(|g| g.game_id == game_id)
            .ok_or_else(|| format!("Game '{}' not found in bottle '{}'", game_id, bottle_name))?;
        let data_dir = PathBuf::from(&game.data_dir);

        deployer::redeploy_all(&db, &game_id, &bottle_name, &data_dir, &game.game_path)
            .map_err(|e| format!("Heal redeploy failed: {e}"))?;

        log::info!("heal_deployment: redeployed {}/{}", game_id, bottle_name);
        Ok(())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}


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


// --- Game Version Pinning ---

#[tauri::command]
pub async fn get_pinned_game_version(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    state
        .db
        .get_pinned_game_version(&game_id, &bottle_name)
        .map_err(|e| format!("Failed to get pinned version: {}", e))
}

#[tauri::command]
pub async fn pin_game_version(
    game_id: String,
    bottle_name: String,
    version: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .db
        .set_pinned_game_version(&game_id, &bottle_name, &version)
        .map_err(|e| format!("Failed to pin version: {}", e))
}


// --- Mod Rollback & Snapshots ---

#[tauri::command]
pub async fn save_mod_version_cmd(
    mod_id: i64,
    version: String,
    staging_path: String,
    archive_name: String,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        rollback::save_mod_version(&db, mod_id, &version, &staging_path, &archive_name)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn list_mod_versions_cmd(
    mod_id: i64,
    state: State<'_, AppState>,
) -> Result<Vec<ModVersion>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || rollback::list_mod_versions(&db, mod_id))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn rollback_mod_version(
    mod_id: i64,
    version_id: i64,
    state: State<'_, AppState>,
) -> Result<ModVersion, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || rollback::rollback_to_version(&db, mod_id, version_id))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn cleanup_mod_versions(
    mod_id: i64,
    keep_count: usize,
    state: State<'_, AppState>,
) -> Result<usize, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || rollback::cleanup_old_versions(&db, mod_id, keep_count))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn create_mod_snapshot(
    game_id: String,
    bottle_name: String,
    name: String,
    description: Option<String>,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        rollback::create_snapshot(&db, &game_id, &bottle_name, &name, description.as_deref())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn list_mod_snapshots(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<ModSnapshot>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || rollback::list_snapshots(&db, &game_id, &bottle_name))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn delete_mod_snapshot(snapshot_id: i64, state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || rollback::delete_snapshot(&db, snapshot_id))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
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


// --- Download Queue ---

#[tauri::command]
pub fn get_download_queue(state: State<AppState>) -> Vec<download_queue::QueueItem> {
    state.download_queue.get_all()
}

#[tauri::command]
pub fn get_download_queue_counts(state: State<AppState>) -> download_queue::QueueCounts {
    state.download_queue.status_counts()
}

#[tauri::command]
pub async fn retry_download(id: u64, state: State<'_, AppState>) -> Result<bool, String> {
    let queue = state.download_queue.clone();
    tokio::task::spawn_blocking(move || Ok(queue.mark_for_retry(id)))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub fn cancel_download(id: u64, state: State<AppState>) -> Result<(), String> {
    state.download_queue.set_cancelled(id);
    Ok(())
}

#[tauri::command]
pub fn clear_finished_downloads(state: State<AppState>) -> usize {
    state.download_queue.clear_finished()
}


// --- Notification Log ---

#[tauri::command]
pub async fn get_notification_log(
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<database::NotificationEntry>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        db.get_notifications(limit.unwrap_or(50))
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn clear_notification_log(state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || db.clear_notifications().map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn log_notification(
    level: String,
    message: String,
    detail: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        db.log_notification(&level, &message, detail.as_deref())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn get_notification_count(state: State<'_, AppState>) -> Result<usize, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || db.notification_count().map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("Task failed: {e}"))?
}


// --- Error Event Diagnostics ---

#[tauri::command]
pub async fn record_error_event_cmd(
    module: String,
    error_type: String,
    message: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        db.record_error_event(&module, &error_type, &message)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn get_error_summary(
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> Result<Vec<database::ErrorEvent>, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        db.get_error_summary(limit.unwrap_or(20))
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}


// --- Update Checking ---

#[tauri::command]
pub async fn check_mod_updates(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<ModUpdateInfo>, String> {
    let client = nexus_client().await?;

    let mods = {
        let db = &state.db;
        db.list_mods(&game_id, &bottle_name)
            .map_err(|e| e.to_string())?
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

    client
        .check_updates(game_slug, &queries)
        .await
        .map_err(|e| e.to_string())
}


// --- Browser WebView Management ---

#[tauri::command]
pub async fn create_browser_webview(
    app: AppHandle,
    url: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    // Close existing browser panel if any
    if let Some(existing) = app.get_webview("browser-panel") {
        let _ = existing.close();
    }

    let parsed_url: tauri::Url = url.parse().map_err(|e: url::ParseError| e.to_string())?;
    let window = app.get_window("main").ok_or("Main window not found")?;

    let builder = tauri::webview::WebviewBuilder::new(
        "browser-panel",
        tauri::WebviewUrl::External(parsed_url),
    );

    window
        .add_child(
            builder,
            tauri::LogicalPosition::new(x, y),
            tauri::LogicalSize::new(width, height),
        )
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub async fn resize_browser_webview(
    app: AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let webview = app
        .get_webview("browser-panel")
        .ok_or("Browser panel not found")?;
    webview
        .set_position(tauri::LogicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    webview
        .set_size(tauri::LogicalSize::new(width, height))
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn close_browser_webview(app: AppHandle) -> Result<(), String> {
    if let Some(webview) = app.get_webview("browser-panel") {
        webview.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn navigate_browser_webview(app: AppHandle, url: String) -> Result<(), String> {
    let webview = app
        .get_webview("browser-panel")
        .ok_or("Browser panel not found")?;
    let parsed_url: tauri::Url = url.parse().map_err(|e: url::ParseError| e.to_string())?;
    webview.navigate(parsed_url).map_err(|e| e.to_string())?;
    Ok(())
}


// --- Game Directory Cleaner ---

#[tauri::command]
pub async fn scan_game_directory(
    game_id: String,
    bottle_name: String,
    state: State<'_, AppState>,
) -> Result<cleaner::CleanReport, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let (_, _, data_dir) = resolve_game(&game_id, &bottle_name)?;
        cleaner::scan_game_directory(&db, &game_id, &bottle_name, &data_dir)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[tauri::command]
pub async fn clean_game_directory(
    game_id: String,
    bottle_name: String,
    options: cleaner::CleanOptions,
    state: State<'_, AppState>,
) -> Result<cleaner::CleanResult, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let (bottle, game, data_dir) = resolve_game(&game_id, &bottle_name)?;

        if !options.dry_run {
            auto_snapshot_before_destructive(
                &db,
                &game_id,
                &bottle_name,
                "Before clean game directory",
            );
        }

        let result =
            cleaner::clean_game_directory(&db, &game_id, &bottle_name, &data_dir, &options)
                .map_err(|e| e.to_string())?;

        // After a full clean (not orphans-only), reset plugins.txt to vanilla state
        // so the load order doesn't show stale entries for removed plugins
        if !options.dry_run && !options.orphans_only && !result.removed_files.is_empty() {
            if let Some(plugins_file) = games::with_plugin(&game_id, |plugin| {
                plugin.get_plugins_file(Path::new(&game.game_path), &bottle)
            })
            .flatten()
            {
                // Build vanilla plugin list from stock ESMs still on disk
                let vanilla_entries: Vec<plugins::skyrim_plugins::PluginEntry> =
                    plugins::skyrim_plugins::get_implicit_plugins(&game_id)
                        .iter()
                        .filter(|name| data_dir.join(name).exists())
                        .map(|name| plugins::skyrim_plugins::PluginEntry {
                            filename: name.to_string(),
                            enabled: true,
                        })
                        .collect();
                let _ = plugins::skyrim_plugins::write_plugins_txt(&plugins_file, &vanilla_entries);
                log::info!(
                    "Reset plugins.txt to {} vanilla entries after clean",
                    vanilla_entries.len()
                );

                // Also reset loadorder.txt if it exists alongside plugins.txt
                if let Some(parent) = plugins_file.parent() {
                    let loadorder_file = parent.join("loadorder.txt");
                    if loadorder_file.exists() {
                        let _ = std::fs::remove_file(&loadorder_file);
                        log::info!("Removed stale loadorder.txt after clean");
                    }
                }
            }
        }

        Ok(result)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

