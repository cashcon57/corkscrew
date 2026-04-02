//! Platform detection, system info, Steam/Proton integration, and DLC checks.

use crate::deck;
use crate::instruction_types;
use crate::llm_chat;
use crate::proton;
use crate::steam_integration;
use crate::{resolve_game};
use serde::{Deserialize, Serialize};

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
        .map_err(crate::format_join_error)
}

#[tauri::command]
pub async fn add_to_steam() -> Result<steam_integration::SteamStatus, String> {
    tokio::task::spawn_blocking(move || {
        steam_integration::setup_steam_integration().map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn remove_from_steam() -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let info = steam_integration::detect_steam_installation()
            .ok_or_else(|| "Steam not found".to_string())?;
        steam_integration::remove_from_steam(&info).map_err(|e| e.to_string())
    })
    .await
    .map_err(crate::format_join_error)?
}

#[tauri::command]
pub fn is_steam_deck() -> bool {
    steam_integration::is_steam_deck()
}

#[tauri::command]
pub async fn steam_deck_warnings() -> Result<Vec<String>, String> {
    tokio::task::spawn_blocking(move || steam_integration::steam_deck_warnings())
        .await
        .map_err(crate::format_join_error)
}


// --- Proton Detection ---

#[tauri::command]
pub async fn list_proton_versions() -> Result<Vec<proton::ProtonVersion>, String> {
    tokio::task::spawn_blocking(move || Ok(proton::detect_proton_versions()))
        .await
        .map_err(crate::format_join_error)?
}

#[tauri::command]
pub async fn get_recommended_proton() -> Result<Option<proton::ProtonVersion>, String> {
    tokio::task::spawn_blocking(move || Ok(proton::get_recommended_proton()))
        .await
        .map_err(crate::format_join_error)?
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
    .map_err(crate::format_join_error)?
}


