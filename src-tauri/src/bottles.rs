//! Wine bottle detection for CrossOver, Whisky, Moonshine, Mythic, Heroic,
//! and native Wine/Proton managers on macOS and Linux.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Enriched game info from Heroic Launcher.
#[derive(Debug, Clone, Serialize)]
pub struct HeroicGameInfo {
    pub app_name: String,
    pub title: String,
    /// "gog" or "epic"
    pub platform: String,
    pub install_path: Option<String>,
    pub wine_prefix: Option<PathBuf>,
    pub version: Option<String>,
}

/// Represents a Wine bottle (prefix) managed by a compatibility layer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bottle {
    /// Display name of the bottle (usually the directory name).
    pub name: String,
    /// Absolute path to the bottle root directory.
    pub path: PathBuf,
    /// Which manager created this bottle (e.g. "CrossOver", "Whisky", "Proton").
    pub source: String,
}

impl Bottle {
    /// Path to the virtual C: drive inside this bottle.
    pub fn drive_c(&self) -> PathBuf {
        self.path.join("drive_c")
    }

    /// Path to `C:\Program Files`.
    pub fn program_files(&self) -> PathBuf {
        self.drive_c().join("Program Files")
    }

    /// Path to `C:\Program Files (x86)`.
    pub fn program_files_x86(&self) -> PathBuf {
        self.drive_c().join("Program Files (x86)")
    }

    /// Path to the `users` directory inside drive_c.
    pub fn users_dir(&self) -> PathBuf {
        self.drive_c().join("users")
    }

    /// Best-effort path to a user's `AppData\Local` directory.
    ///
    /// Iterates over user directories looking for the standard AppData layout.
    /// Falls back to legacy `Local Settings\Application Data`, then to the
    /// CrossOver default user path.
    pub fn appdata_local(&self) -> PathBuf {
        let users = self.users_dir();
        if users.exists() {
            if let Ok(entries) = fs::read_dir(&users) {
                for entry in entries.flatten() {
                    let user_dir = entry.path();
                    if !user_dir.is_dir() {
                        continue;
                    }

                    // Standard AppData path
                    let local = user_dir.join("AppData").join("Local");
                    if local.exists() {
                        return local;
                    }

                    // Legacy path used by some bottles
                    let legacy = user_dir.join("Local Settings").join("Application Data");
                    if legacy.exists() {
                        return legacy;
                    }
                }
            }
        }

        // Default fallback (CrossOver convention)
        users.join("crossover").join("AppData").join("Local")
    }

    /// Best-effort path to a user's Documents directory.
    ///
    /// Iterates over user directories looking for `Documents` or `My Documents`.
    /// Falls back to the CrossOver default user path.
    pub fn documents_dir(&self) -> PathBuf {
        let users = self.users_dir();
        if users.exists() {
            if let Ok(entries) = fs::read_dir(&users) {
                for entry in entries.flatten() {
                    let user_dir = entry.path();
                    if !user_dir.is_dir() {
                        continue;
                    }

                    // Standard Documents path
                    let docs = user_dir.join("Documents");
                    if docs.exists() {
                        return docs;
                    }

                    // Legacy "My Documents" path
                    let my_docs = user_dir.join("My Documents");
                    if my_docs.exists() {
                        return my_docs;
                    }
                }
            }
        }

        // Default fallback (CrossOver convention)
        users.join("crossover").join("Documents")
    }

    /// Returns `true` if the bottle's `drive_c` directory exists on disk.
    pub fn exists(&self) -> bool {
        self.drive_c().exists()
    }

    /// Walk into the bottle's `drive_c` following the given path components,
    /// matching each component **case-insensitively**.
    ///
    /// This is essential for Wine compatibility because Windows paths are
    /// case-insensitive but the underlying macOS/Linux filesystem may not be.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Finds "C:\users\steamuser\AppData\Local" regardless of casing on disk.
    /// let local = bottle.find_path(&["users", "steamuser", "AppData", "Local"]);
    /// ```
    pub fn find_path(&self, parts: &[&str]) -> Option<PathBuf> {
        let mut current = self.drive_c();

        for part in parts {
            if !current.exists() {
                return None;
            }

            // Try an exact match first (fast path).
            let candidate = current.join(part);
            if candidate.exists() {
                current = candidate;
                continue;
            }

            // Case-insensitive fallback: scan directory entries.
            let part_lower = part.to_lowercase();
            let mut found = false;

            if let Ok(entries) = fs::read_dir(&current) {
                for entry in entries.flatten() {
                    if entry.file_name().to_string_lossy().to_lowercase() == part_lower {
                        current = entry.path();
                        found = true;
                        break;
                    }
                }
            }

            if !found {
                return None;
            }
        }

        Some(current)
    }
}

// ---------------------------------------------------------------------------
// Container / immutable-distro path normalization
// ---------------------------------------------------------------------------

/// Container environment type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[allow(dead_code)]
pub enum ContainerEnvironment {
    Flatpak,
    Snap,
    Docker,
}

/// Detect if running inside a container environment.
#[allow(dead_code)]
pub fn detect_container_environment() -> Option<ContainerEnvironment> {
    // Flatpak
    if std::path::Path::new("/.flatpak-info").exists()
        || std::env::var("FLATPAK_ID").is_ok()
    {
        return Some(ContainerEnvironment::Flatpak);
    }

    // Snap
    if std::env::var("SNAP").is_ok() {
        return Some(ContainerEnvironment::Snap);
    }

    // Podman/Docker (less likely for desktop apps but possible)
    if std::path::Path::new("/.dockerenv").exists() {
        return Some(ContainerEnvironment::Docker);
    }

    None
}

/// Normalize paths for container environments (Flatpak, Fedora Atomic/Bazzite).
///
/// - `/var/home/user/` -> `/home/user/` (Fedora Atomic/Bazzite use /var/home)
/// - Detects Flatpak environment and adjusts paths accordingly
#[allow(dead_code)]
pub fn normalize_container_path(path: &Path) -> PathBuf {
    let path_str = path.to_string_lossy();

    // Fedora Atomic / Bazzite: /var/home/ -> /home/
    if path_str.starts_with("/var/home/") {
        let normalized = PathBuf::from(path_str.replacen("/var/home/", "/home/", 1));
        if normalized.exists() || !path.exists() {
            return normalized;
        }
    }

    path.to_path_buf()
}

// ---------------------------------------------------------------------------
// Platform-specific search path definitions
// ---------------------------------------------------------------------------

/// A named search location: (source label, path to parent directory of bottles).
struct SearchLocation {
    source: &'static str,
    path: PathBuf,
}

/// Build the list of directories to scan on macOS.
#[cfg(target_os = "macos")]
fn platform_search_locations(home: &Path) -> Vec<SearchLocation> {
    vec![
        // CrossOver (unsandboxed — direct install from CodeWeavers website)
        SearchLocation {
            source: "CrossOver",
            path: home
                .join("Library")
                .join("Application Support")
                .join("CrossOver")
                .join("Bottles"),
        },
        // CrossOver (sandboxed — Mac App Store or Setapp install)
        SearchLocation {
            source: "CrossOver",
            path: home
                .join("Library")
                .join("Containers")
                .join("com.codeweavers.CrossOver")
                .join("Data")
                .join("Library")
                .join("Application Support")
                .join("CrossOver")
                .join("Bottles"),
        },
        // Whisky
        SearchLocation {
            source: "Whisky",
            path: home
                .join("Library")
                .join("Containers")
                .join("com.isaacmarovitz.Whisky")
                .join("Bottles"),
        },
        // Moonshine
        SearchLocation {
            source: "Moonshine",
            path: home
                .join("Library")
                .join("Containers")
                .join("com.ybmeng.moonshine")
                .join("Bottles"),
        },
        // Heroic Games Launcher
        SearchLocation {
            source: "Heroic",
            path: home
                .join("Library")
                .join("Application Support")
                .join("heroic")
                .join("Prefixes"),
        },
        // Mythic
        SearchLocation {
            source: "Mythic",
            path: home
                .join("Library")
                .join("Containers")
                .join("io.getmythic.Mythic")
                .join("Bottles"),
        },
    ]
}

/// Build the list of directories to scan on Linux.
#[cfg(target_os = "linux")]
fn platform_search_locations(home: &Path) -> Vec<SearchLocation> {
    // Normalize for Fedora Atomic / Bazzite (/var/home -> /home)
    let home = normalize_container_path(home);
    let home = home.as_path();
    let mut locations = vec![
        // Native Wine default prefix
        SearchLocation {
            source: "Wine",
            path: home.join(".wine"),
        },
        // Heroic Games Launcher (native install)
        SearchLocation {
            source: "Heroic",
            path: home.join("Games").join("Heroic").join("Prefixes"),
        },
        // Heroic Games Launcher (Flatpak)
        SearchLocation {
            source: "Heroic",
            path: home
                .join(".var")
                .join("app")
                .join("com.heroicgameslauncher.hgl")
                .join("data")
                .join("heroic")
                .join("Prefixes"),
        },
        // Lutris
        SearchLocation {
            source: "Lutris",
            path: home
                .join(".local")
                .join("share")
                .join("lutris")
                .join("runners")
                .join("wine")
                .join("prefixes"),
        },
        // Bottles (Flatpak-first app)
        SearchLocation {
            source: "Bottles",
            path: home
                .join(".local")
                .join("share")
                .join("bottles")
                .join("bottles"),
        },
        // Steam / Proton (primary library)
        SearchLocation {
            source: "Proton",
            path: home
                .join(".local")
                .join("share")
                .join("Steam")
                .join("steamapps")
                .join("compatdata"),
        },
        // Steam via symlink (common on SteamOS)
        SearchLocation {
            source: "Proton",
            path: home
                .join(".steam")
                .join("steam")
                .join("steamapps")
                .join("compatdata"),
        },
        // Flatpak Steam
        SearchLocation {
            source: "Proton",
            path: home
                .join(".var")
                .join("app")
                .join("com.valvesoftware.Steam")
                .join(".local")
                .join("share")
                .join("Steam")
                .join("steamapps")
                .join("compatdata"),
        },
    ];

    // Also scan secondary Steam library folders (e.g. SD card on Steam Deck)
    let steam_dirs = [
        home.join(".local/share/Steam"),
        home.join(".steam/steam"),
    ];
    for steam_dir in &steam_dirs {
        for vdf_name in &["steamapps/libraryfolders.vdf", "config/libraryfolders.vdf"] {
            let vdf_path = steam_dir.join(vdf_name);
            if let Ok(content) = fs::read_to_string(&vdf_path) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if let Some(rest) = trimmed.strip_prefix("\"path\"") {
                        let rest = rest.trim().trim_matches('"');
                        if !rest.is_empty() {
                            let lib_path =
                                PathBuf::from(rest.replace('\\', "/"))
                                    .join("steamapps")
                                    .join("compatdata");
                            // Avoid duplicates
                            if !locations.iter().any(|l| l.path == lib_path) && lib_path.is_dir() {
                                log::info!(
                                    "Found additional Steam library: {}",
                                    lib_path.display()
                                );
                                locations.push(SearchLocation {
                                    source: "Proton",
                                    path: lib_path,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    locations
}

// ---------------------------------------------------------------------------
// Detection helpers
// ---------------------------------------------------------------------------

/// Native Wine on Linux stores its prefix directly at `~/.wine` rather than
/// as a subdirectory. This constant tracks the source label that uses that
/// layout so we can handle it specially.
#[cfg(target_os = "linux")]
const DIRECT_PREFIX_SOURCE: &str = "Wine";

/// Deduplicate a list of bottles by name (case-insensitive).
///
/// When the same bottle name is present in multiple scan roots (e.g. both the
/// unsandboxed and sandboxed CrossOver paths), retain only the instance whose
/// `bottle.toml` has the most recent modification time.  If no `bottle.toml`
/// is found in either path, the first occurrence wins.
fn deduplicate_bottles_by_name(bottles: Vec<Bottle>) -> Vec<Bottle> {
    use std::collections::HashMap;

    // Preserve insertion order so that the output is deterministic.  We build
    // a map from lowercase name -> index into `result`, and update the entry
    // whenever we find a newer mtime.
    let mut result: Vec<Bottle> = Vec::with_capacity(bottles.len());
    let mut name_to_idx: HashMap<String, usize> = HashMap::new();

    for bottle in bottles {
        let key = bottle.name.to_lowercase();
        match name_to_idx.get(&key).copied() {
            None => {
                // First time we see this name.
                name_to_idx.insert(key, result.len());
                result.push(bottle);
            }
            Some(existing_idx) => {
                // We've already seen a bottle with this name — keep the one
                // with the newer bottle.toml (or drive_c) mtime.
                if bottle_mtime(&bottle) > bottle_mtime(&result[existing_idx]) {
                    log::debug!(
                        "CrossOver bottle '{}': preferring path {} over {}",
                        bottle.name,
                        bottle.path.display(),
                        result[existing_idx].path.display()
                    );
                    result[existing_idx] = bottle;
                }
            }
        }
    }

    result
}

/// Return the mtime of a bottle as seconds since the Unix epoch.
///
/// We check `bottle.toml` first (CrossOver writes this on last use), then fall
/// back to `drive_c` itself.  Returns `0` if neither is readable.
fn bottle_mtime(bottle: &Bottle) -> u64 {
    // Try bottle.toml first — CrossOver updates it on every launch.
    let toml_path = bottle.path.join("bottle.toml");
    if let Ok(meta) = fs::metadata(&toml_path) {
        if let Ok(modified) = meta.modified() {
            return modified
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
        }
    }
    // Fall back to drive_c mtime.
    if let Ok(meta) = fs::metadata(bottle.drive_c()) {
        if let Ok(modified) = meta.modified() {
            return modified
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
        }
    }
    0
}

/// Scan a single search location and collect any valid bottles it contains.
fn collect_bottles_from(location: &SearchLocation, bottles: &mut Vec<Bottle>) {
    // On Linux the native Wine prefix (~/.wine) is itself a bottle, not a
    // directory *containing* bottles.
    #[cfg(target_os = "linux")]
    if location.source == DIRECT_PREFIX_SOURCE {
        if location.path.is_dir() {
            let bottle = Bottle {
                name: location
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| location.source.to_string()),
                path: location.path.clone(),
                source: location.source.to_string(),
            };
            if bottle.exists() {
                bottles.push(bottle);
            }
        }
        return;
    }

    // Proton/Steam prefixes have a special structure:
    //   compatdata/{appid}/pfx/drive_c    (the actual Wine prefix)
    // Other bottle managers use:
    //   bottles/{name}/drive_c
    #[cfg(target_os = "linux")]
    if location.source == "Proton" {
        collect_proton_bottles(&location.path, bottles);
        return;
    }

    if !location.path.is_dir() {
        return;
    }

    let Ok(entries) = fs::read_dir(&location.path) else {
        return;
    };

    // Collect and sort entries by name for deterministic ordering.
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();

    for dir in dirs {
        let bottle = Bottle {
            name: dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: dir,
            source: location.source.to_string(),
        };
        if bottle.exists() {
            bottles.push(bottle);
        }
    }
}

/// Collect Proton bottles from a Steam `compatdata` directory.
///
/// Proton prefixes live at `compatdata/{appid}/pfx/` — the `pfx/` subdirectory
/// is the actual Wine prefix containing `drive_c/`.  We cross-reference the app
/// ID against `appmanifest_{appid}.acf` in the parent `steamapps/` to get a
/// human-readable game name for the bottle.
#[cfg(target_os = "linux")]
fn collect_proton_bottles(compatdata_dir: &Path, bottles: &mut Vec<Bottle>) {
    if !compatdata_dir.is_dir() {
        return;
    }

    // The steamapps directory is one level up from compatdata
    let steamapps_dir = compatdata_dir.parent();

    let Ok(entries) = fs::read_dir(compatdata_dir) else {
        return;
    };

    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();

    for dir in dirs {
        let app_id_str = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        // Skip non-numeric directories (not app IDs)
        if !app_id_str.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }

        // The actual Wine prefix is at {appid}/pfx/
        let pfx_dir = dir.join("pfx");
        if !pfx_dir.join("drive_c").exists() {
            continue;
        }

        // Try to resolve game name from appmanifest
        let name = steamapps_dir
            .and_then(|sa| {
                let manifest = sa.join(format!("appmanifest_{}.acf", app_id_str));
                parse_appmanifest_name(&manifest)
            })
            .unwrap_or_else(|| format!("Proton {}", app_id_str));

        bottles.push(Bottle {
            name,
            path: pfx_dir,
            source: "Proton".to_string(),
        });
    }
}

/// Extract the game name from a Steam appmanifest .acf file.
#[cfg(target_os = "linux")]
fn parse_appmanifest_name(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        // Match: "name"		"Game Title"
        if trimmed.starts_with("\"name\"") {
            let rest = trimmed.strip_prefix("\"name\"")?;
            let rest = rest.trim();
            if rest.starts_with('"') && rest.ends_with('"') && rest.len() >= 2 {
                return Some(rest[1..rest.len() - 1].to_string());
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Heroic Launcher metadata parsing
// ---------------------------------------------------------------------------

/// Parse Heroic's GOG installed games.
fn parse_heroic_gog_games(heroic_config_dir: &Path) -> Vec<HeroicGameInfo> {
    let gog_path = heroic_config_dir.join("gog_store").join("installed.json");
    let mut games = Vec::new();

    let content = match fs::read_to_string(&gog_path) {
        Ok(c) => c,
        Err(_) => return games,
    };

    let parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("Failed to parse Heroic GOG installed.json: {}", e);
            return games;
        }
    };

    if let Some(installed) = parsed.get("installed").and_then(|v| v.as_array()) {
        for game in installed {
            let app_name = game
                .get("appName")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let title = game
                .get("install_path")
                .and_then(|v| v.as_str())
                .and_then(|p| Path::new(p).file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| app_name.clone());
            let install_path = game
                .get("install_path")
                .and_then(|v| v.as_str())
                .map(String::from);
            let version = game
                .get("version")
                .and_then(|v| v.as_str())
                .map(String::from);

            let wine_prefix = get_heroic_wine_prefix(heroic_config_dir, &app_name);

            games.push(HeroicGameInfo {
                app_name,
                title,
                platform: "gog".to_string(),
                install_path,
                wine_prefix,
                version,
            });
        }
    }

    games
}

/// Parse Heroic's Epic installed games (via Legendary).
fn parse_heroic_epic_games(heroic_config_dir: &Path) -> Vec<HeroicGameInfo> {
    let epic_path = heroic_config_dir
        .join("store_cache")
        .join("legendary_library.json");
    let mut games = Vec::new();

    let content = match fs::read_to_string(&epic_path) {
        Ok(c) => c,
        Err(_) => return games,
    };

    let parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("Failed to parse Heroic Epic library: {}", e);
            return games;
        }
    };

    if let Some(library) = parsed.get("library").and_then(|v| v.as_array()) {
        for game in library {
            let app_name = game
                .get("app_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let title = game
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or(&app_name)
                .to_string();
            let install_path = game
                .get("install_path")
                .and_then(|v| v.as_str())
                .map(String::from);
            let version = game
                .get("version")
                .and_then(|v| v.as_str())
                .map(String::from);

            let is_installed = game
                .get("is_installed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !is_installed {
                continue;
            }

            let wine_prefix = get_heroic_wine_prefix(heroic_config_dir, &app_name);

            games.push(HeroicGameInfo {
                app_name,
                title,
                platform: "epic".to_string(),
                install_path,
                wine_prefix,
                version,
            });
        }
    }

    games
}

/// Get Wine prefix path for a Heroic game from its GamesConfig.
fn get_heroic_wine_prefix(heroic_config_dir: &Path, app_name: &str) -> Option<PathBuf> {
    let config_path = heroic_config_dir
        .join("GamesConfig")
        .join(format!("{}.json", app_name));
    let content = fs::read_to_string(&config_path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;

    // The config can have the prefix nested under the app_name key or at top level
    parsed
        .get(app_name)
        .or_else(|| parsed.get("winePrefix"))
        .and_then(|v| {
            v.get("winePrefix")
                .and_then(|p| p.as_str())
                .or_else(|| v.as_str())
        })
        .map(PathBuf::from)
        .and_then(|p| {
            // Validate path from external JSON — reject traversal, null bytes, drive letters
            let s = p.to_string_lossy();
            if s.contains('\0') || s.contains("..") {
                log::warn!("Rejecting unsafe Heroic wine prefix path: {:?}", p);
                return None;
            }
            // Must be an absolute path to be a valid wine prefix
            if !p.is_absolute() {
                log::warn!("Rejecting non-absolute Heroic wine prefix path: {:?}", p);
                return None;
            }
            Some(p)
        })
}

/// Find all Heroic config directories (standard + Flatpak + macOS).
fn find_heroic_config_dirs() -> Vec<PathBuf> {
    let mut config_dirs = Vec::new();
    if let Some(home) = dirs::home_dir() {
        // Linux: standard install
        let standard = home.join(".config").join("heroic");
        if standard.is_dir() {
            config_dirs.push(standard);
        }
        // Linux: Flatpak install
        let flatpak = home
            .join(".var")
            .join("app")
            .join("com.heroicgameslauncher.hgl")
            .join("config")
            .join("heroic");
        if flatpak.is_dir() {
            config_dirs.push(flatpak);
        }
        // macOS
        let macos = home
            .join("Library")
            .join("Application Support")
            .join("heroic");
        if macos.is_dir() {
            config_dirs.push(macos);
        }
    }
    config_dirs
}

/// Detect all Heroic games with enriched metadata.
pub fn detect_heroic_games() -> Vec<HeroicGameInfo> {
    let mut all_games = Vec::new();

    for config_dir in find_heroic_config_dirs() {
        let mut gog = parse_heroic_gog_games(&config_dir);
        let mut epic = parse_heroic_epic_games(&config_dir);
        let mut sideload = parse_heroic_sideloaded_games(&config_dir);
        all_games.append(&mut gog);
        all_games.append(&mut epic);
        all_games.append(&mut sideload);
    }

    // Deduplicate by app_name
    all_games.sort_by(|a, b| a.app_name.cmp(&b.app_name));
    all_games.dedup_by(|a, b| a.app_name == b.app_name);

    log::info!(
        "Detected {} Heroic games ({} GOG, {} Epic, {} sideload)",
        all_games.len(),
        all_games.iter().filter(|g| g.platform == "gog").count(),
        all_games.iter().filter(|g| g.platform == "epic").count(),
        all_games.iter().filter(|g| g.platform == "sideload").count(),
    );

    all_games
}

/// Parse Heroic's sideloaded (custom Windows exe) library. These games
/// don't come from GOG or Epic — users add them manually through Heroic's
/// "Add Game" dialog. Heroic stores them under
/// `sideload_apps/library.json`. Wine prefix lives in GamesConfig as
/// usual.
fn parse_heroic_sideloaded_games(heroic_config_dir: &Path) -> Vec<HeroicGameInfo> {
    let library_path = heroic_config_dir
        .join("sideload_apps")
        .join("library.json");
    let mut games = Vec::new();

    let content = match fs::read_to_string(&library_path) {
        Ok(c) => c,
        Err(_) => return games, // sideload_apps/ may not exist if user has none
    };

    let parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("Failed to parse Heroic sideload library: {}", e);
            return games;
        }
    };

    // Schema mirrors legendary_library.json: { "library": [ {...}, ... ] }.
    // Older Heroic versions used a top-level array; handle both.
    let library_array = parsed
        .get("library")
        .and_then(|v| v.as_array())
        .or_else(|| parsed.as_array());

    let Some(library) = library_array else {
        return games;
    };

    for game in library {
        let app_name = game
            .get("app_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if app_name.is_empty() {
            continue;
        }
        let title = game
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or(&app_name)
            .to_string();
        let install_path = game
            .get("install")
            .and_then(|v| v.get("install_path"))
            .or_else(|| game.get("install_path"))
            .and_then(|v| v.as_str())
            .map(String::from);
        let version = game
            .get("install")
            .and_then(|v| v.get("version"))
            .or_else(|| game.get("version"))
            .and_then(|v| v.as_str())
            .map(String::from);

        let is_installed = game
            .get("is_installed")
            .or_else(|| game.get("install").and_then(|v| v.get("is_installed")))
            .and_then(|v| v.as_bool())
            .unwrap_or(true); // sideloaded entries imply install
        if !is_installed {
            continue;
        }

        let wine_prefix = get_heroic_wine_prefix(heroic_config_dir, &app_name);

        games.push(HeroicGameInfo {
            app_name,
            title,
            platform: "sideload".to_string(),
            install_path,
            wine_prefix,
            version,
        });
    }

    games
}

/// Build a lookup from wine prefix path to HeroicGameInfo for enriching bottles.
fn build_heroic_prefix_map(
    games: &[HeroicGameInfo],
) -> std::collections::HashMap<PathBuf, &HeroicGameInfo> {
    let mut map = std::collections::HashMap::new();
    for game in games {
        if let Some(ref prefix) = game.wine_prefix {
            map.insert(prefix.clone(), game);
        }
    }
    map
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Scan all known locations for Wine bottles and return every valid bottle
/// found. A bottle is considered valid if its `drive_c` directory exists.
///
/// Heroic bottles are enriched with game metadata (title, platform) from
/// Heroic's config files when available.
pub fn detect_bottles() -> Vec<Bottle> {
    let Some(home) = dirs::home_dir() else {
        log::warn!("Could not determine home directory; no bottles detected.");
        return Vec::new();
    };

    // Log container environment on Linux
    #[cfg(target_os = "linux")]
    if let Some(ref env) = detect_container_environment() {
        log::info!("Container environment detected: {:?}", env);
    }

    let locations = platform_search_locations(&home);
    let mut bottles = Vec::new();

    for location in &locations {
        collect_bottles_from(location, &mut bottles);
    }

    // Deduplicate: if the same bottle name appears more than once (e.g. both
    // sandboxed and unsandboxed CrossOver paths), keep the one whose
    // bottle.toml has the most recent mtime.  This ensures we always point at
    // the active install regardless of which CrossOver distribution variant the
    // user has.
    bottles = deduplicate_bottles_by_name(bottles);

    // Enrich Heroic bottles with game metadata
    let heroic_games = detect_heroic_games();
    if !heroic_games.is_empty() {
        let prefix_map = build_heroic_prefix_map(&heroic_games);
        for bottle in &mut bottles {
            if bottle.source != "Heroic" {
                continue;
            }
            // Match by wine prefix path (the bottle path itself)
            if let Some(game) = prefix_map.get(&bottle.path) {
                bottle.name = game.title.clone();
                bottle.source = format!(
                    "Heroic ({})",
                    if game.platform == "gog" { "GOG" } else { "Epic" }
                );
            }
        }
    }

    bottles
}

/// Find a specific bottle by name (case-insensitive).
pub fn find_bottle_by_name(name: &str) -> Option<Bottle> {
    let name_lower = name.to_lowercase();
    detect_bottles()
        .into_iter()
        .find(|b| b.name.to_lowercase() == name_lower)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Helper: create a minimal fake bottle on disk and return its path.
    fn create_fake_bottle(parent: &Path, name: &str) -> PathBuf {
        let bottle = parent.join(name);
        fs::create_dir_all(bottle.join("drive_c")).expect("create drive_c");
        bottle
    }

    #[test]
    fn bottle_exists_when_drive_c_present() {
        let tmp = tempfile::tempdir().unwrap();
        let path = create_fake_bottle(tmp.path(), "TestBottle");

        let bottle = Bottle {
            name: "TestBottle".into(),
            path,
            source: "Test".into(),
        };

        assert!(bottle.exists());
    }

    #[test]
    fn bottle_does_not_exist_without_drive_c() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("EmptyBottle");
        fs::create_dir_all(&path).unwrap();

        let bottle = Bottle {
            name: "EmptyBottle".into(),
            path,
            source: "Test".into(),
        };

        assert!(!bottle.exists());
    }

    #[test]
    fn find_path_exact_match() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = create_fake_bottle(tmp.path(), "Bottle");
        fs::create_dir_all(bottle_path.join("drive_c").join("Games").join("Skyrim")).unwrap();

        let bottle = Bottle {
            name: "Bottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };

        let result = bottle.find_path(&["Games", "Skyrim"]);
        assert!(result.is_some());
        assert!(result.unwrap().ends_with("Skyrim"));
    }

    #[test]
    fn find_path_case_insensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = create_fake_bottle(tmp.path(), "Bottle");
        fs::create_dir_all(
            bottle_path
                .join("drive_c")
                .join("Program Files")
                .join("MyGame"),
        )
        .unwrap();

        let bottle = Bottle {
            name: "Bottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };

        // Search with different casing
        let result = bottle.find_path(&["program files", "mygame"]);
        assert!(result.is_some());
    }

    #[test]
    fn find_path_returns_none_for_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = create_fake_bottle(tmp.path(), "Bottle");

        let bottle = Bottle {
            name: "Bottle".into(),
            path: bottle_path,
            source: "Test".into(),
        };

        assert!(bottle.find_path(&["NonExistent", "Path"]).is_none());
    }

    #[test]
    fn parse_heroic_gog_games_from_json() {
        let tmp = tempfile::tempdir().unwrap();
        let gog_dir = tmp.path().join("gog_store");
        fs::create_dir_all(&gog_dir).unwrap();

        let json = r#"{
            "installed": [
                {
                    "appName": "1234567890",
                    "install_path": "/home/user/Games/Heroic/Cyberpunk 2077",
                    "version": "1.63"
                }
            ]
        }"#;
        fs::write(gog_dir.join("installed.json"), json).unwrap();

        let games = parse_heroic_gog_games(tmp.path());
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].app_name, "1234567890");
        assert_eq!(games[0].title, "Cyberpunk 2077");
        assert_eq!(games[0].platform, "gog");
        assert_eq!(games[0].version.as_deref(), Some("1.63"));
    }

    #[test]
    fn parse_heroic_sideloaded_games_basic() {
        let tmp = tempfile::tempdir().unwrap();
        let sideload_dir = tmp.path().join("sideload_apps");
        fs::create_dir_all(&sideload_dir).unwrap();
        let games_config = tmp.path().join("GamesConfig");
        fs::create_dir_all(&games_config).unwrap();

        let json = r#"{
            "library": [
                {
                    "app_name": "custom-skyrim",
                    "title": "Custom Skyrim Install",
                    "install": {
                        "install_path": "/home/user/Games/Skyrim",
                        "is_installed": true,
                        "version": "1.6.640"
                    }
                },
                {
                    "app_name": "uninstalled-thing",
                    "title": "Removed",
                    "install": { "is_installed": false }
                }
            ]
        }"#;
        fs::write(sideload_dir.join("library.json"), json).unwrap();

        // Wine prefix entry for the first sideloaded game
        fs::write(
            games_config.join("custom-skyrim.json"),
            r#"{"custom-skyrim":{"winePrefix":"/home/user/Games/Heroic/Prefixes/Skyrim"}}"#,
        )
        .unwrap();

        let games = parse_heroic_sideloaded_games(tmp.path());
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].app_name, "custom-skyrim");
        assert_eq!(games[0].title, "Custom Skyrim Install");
        assert_eq!(games[0].platform, "sideload");
        assert_eq!(games[0].version.as_deref(), Some("1.6.640"));
        assert_eq!(
            games[0].wine_prefix.as_deref(),
            Some(Path::new("/home/user/Games/Heroic/Prefixes/Skyrim"))
        );
    }

    #[test]
    fn parse_heroic_sideloaded_games_legacy_top_level_array() {
        let tmp = tempfile::tempdir().unwrap();
        let sideload_dir = tmp.path().join("sideload_apps");
        fs::create_dir_all(&sideload_dir).unwrap();

        let json = r#"[
            { "app_name": "old-format-game", "title": "Old Format" }
        ]"#;
        fs::write(sideload_dir.join("library.json"), json).unwrap();

        let games = parse_heroic_sideloaded_games(tmp.path());
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].app_name, "old-format-game");
        assert_eq!(games[0].platform, "sideload");
    }

    #[test]
    fn parse_heroic_sideloaded_games_no_file() {
        let tmp = tempfile::tempdir().unwrap();
        let games = parse_heroic_sideloaded_games(tmp.path());
        assert!(games.is_empty());
    }

    #[test]
    fn parse_heroic_epic_games_filters_uninstalled() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().join("store_cache");
        fs::create_dir_all(&cache_dir).unwrap();

        let json = r#"{
            "library": [
                {
                    "app_name": "Fortnite",
                    "title": "Fortnite",
                    "is_installed": true
                },
                {
                    "app_name": "UninstalledGame",
                    "title": "Some Game",
                    "is_installed": false
                }
            ]
        }"#;
        fs::write(cache_dir.join("legendary_library.json"), json).unwrap();

        let games = parse_heroic_epic_games(tmp.path());
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].app_name, "Fortnite");
        assert_eq!(games[0].title, "Fortnite");
        assert_eq!(games[0].platform, "epic");
    }

    #[test]
    fn get_heroic_wine_prefix_from_games_config() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().join("GamesConfig");
        fs::create_dir_all(&config_dir).unwrap();

        let json = r#"{
            "MyGame123": {
                "winePrefix": "/home/user/.wine/prefixes/mygame"
            }
        }"#;
        fs::write(config_dir.join("MyGame123.json"), json).unwrap();

        let prefix = get_heroic_wine_prefix(tmp.path(), "MyGame123");
        assert_eq!(
            prefix,
            Some(PathBuf::from("/home/user/.wine/prefixes/mygame"))
        );
    }

    #[test]
    fn get_heroic_wine_prefix_returns_none_for_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let prefix = get_heroic_wine_prefix(tmp.path(), "NonExistent");
        assert!(prefix.is_none());
    }

    #[test]
    fn parse_heroic_gog_returns_empty_for_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let games = parse_heroic_gog_games(tmp.path());
        assert!(games.is_empty());
    }

    #[test]
    fn parse_heroic_epic_returns_empty_for_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let games = parse_heroic_epic_games(tmp.path());
        assert!(games.is_empty());
    }

    #[test]
    fn standard_paths_are_correct() {
        let bottle = Bottle {
            name: "Test".into(),
            path: PathBuf::from("/fake/bottle"),
            source: "Test".into(),
        };

        assert_eq!(bottle.drive_c(), PathBuf::from("/fake/bottle/drive_c"));
        assert_eq!(
            bottle.program_files(),
            PathBuf::from("/fake/bottle/drive_c/Program Files")
        );
        assert_eq!(
            bottle.program_files_x86(),
            PathBuf::from("/fake/bottle/drive_c/Program Files (x86)")
        );
        assert_eq!(
            bottle.users_dir(),
            PathBuf::from("/fake/bottle/drive_c/users")
        );
    }

    // -----------------------------------------------------------------------
    // Fix 2: sandboxed CrossOver path — deduplication tests
    // -----------------------------------------------------------------------

    /// Both unsandboxed and sandboxed paths produce bottles when they exist.
    #[test]
    fn deduplicate_bottles_unique_names_preserved() {
        let tmp = tempfile::tempdir().unwrap();

        let path_a = create_fake_bottle(tmp.path(), "Alpha");
        let path_b = create_fake_bottle(tmp.path(), "Beta");

        let bottles = vec![
            Bottle { name: "Alpha".into(), path: path_a.clone(), source: "CrossOver".into() },
            Bottle { name: "Beta".into(), path: path_b.clone(), source: "CrossOver".into() },
        ];

        let deduped = deduplicate_bottles_by_name(bottles);
        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].name, "Alpha");
        assert_eq!(deduped[1].name, "Beta");
    }

    /// When the same name appears twice with no bottle.toml, the first path wins.
    #[test]
    fn deduplicate_bottles_duplicate_no_toml_first_wins() {
        let tmp = tempfile::tempdir().unwrap();

        let path_a = create_fake_bottle(tmp.path(), "MySkyrim_a");
        let path_b = create_fake_bottle(tmp.path(), "MySkyrim_b");

        let bottles = vec![
            Bottle { name: "MySkyrim".into(), path: path_a.clone(), source: "CrossOver".into() },
            Bottle { name: "MySkyrim".into(), path: path_b.clone(), source: "CrossOver".into() },
        ];

        let deduped = deduplicate_bottles_by_name(bottles);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].path, path_a);
    }

    /// When the same name appears twice and the second has a newer bottle.toml, the
    /// second (sandboxed) path wins.
    #[test]
    fn deduplicate_bottles_newer_toml_wins() {
        let tmp = tempfile::tempdir().unwrap();

        // First bottle: older bottle.toml
        let path_old = create_fake_bottle(tmp.path(), "MySkyrim_old");
        let old_toml = path_old.join("bottle.toml");
        fs::write(&old_toml, "[bottle]\nname = \"MySkyrim\"\n").unwrap();

        // Small sleep is not reliable in unit tests — instead we manually set mtimes
        // using filetime so the test is deterministic regardless of filesystem
        // resolution.
        let old_time = filetime::FileTime::from_unix_time(1_000_000, 0);
        filetime::set_file_mtime(&old_toml, old_time).unwrap();

        // Second bottle: newer bottle.toml
        let path_new = create_fake_bottle(tmp.path(), "MySkyrim_new");
        let new_toml = path_new.join("bottle.toml");
        fs::write(&new_toml, "[bottle]\nname = \"MySkyrim\"\n").unwrap();
        let new_time = filetime::FileTime::from_unix_time(2_000_000, 0);
        filetime::set_file_mtime(&new_toml, new_time).unwrap();

        let bottles = vec![
            Bottle { name: "MySkyrim".into(), path: path_old.clone(), source: "CrossOver".into() },
            Bottle { name: "MySkyrim".into(), path: path_new.clone(), source: "CrossOver".into() },
        ];

        let deduped = deduplicate_bottles_by_name(bottles);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].path, path_new, "newer bottle.toml should win");
    }

    /// Case-insensitive dedup: "Skyrim" and "skyrim" treated as the same bottle.
    #[test]
    fn deduplicate_bottles_case_insensitive() {
        let tmp = tempfile::tempdir().unwrap();

        let path_a = create_fake_bottle(tmp.path(), "SkyrimA");
        let path_b = create_fake_bottle(tmp.path(), "SkyrimB");

        let bottles = vec![
            Bottle { name: "Skyrim".into(), path: path_a.clone(), source: "CrossOver".into() },
            Bottle { name: "SKYRIM".into(), path: path_b.clone(), source: "CrossOver".into() },
        ];

        let deduped = deduplicate_bottles_by_name(bottles);
        assert_eq!(deduped.len(), 1, "case-insensitive duplicate should be collapsed to one entry");
    }
}
