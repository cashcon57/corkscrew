//! Wine bottle detection for CrossOver, Whisky, Moonshine, Mythic, Heroic,
//! and native Wine/Proton managers on macOS and Linux.

#[cfg(target_os = "linux")]
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fomod::{find_case_insensitive, resolve_path_case_insensitive};

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
        find_case_insensitive(&self.drive_c(), "users").unwrap_or_else(|| self.drive_c().join("users"))
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
                    if let Some(local) = resolve_path_case_insensitive(&user_dir, "AppData/Local") {
                        return local;
                    }

                    // Legacy path used by some bottles
                    if let Some(legacy) =
                        resolve_path_case_insensitive(&user_dir, "Local Settings/Application Data")
                    {
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
                    if let Some(docs) = resolve_path_case_insensitive(&user_dir, "Documents") {
                        return docs;
                    }

                    // Legacy "My Documents" path
                    if let Some(my_docs) =
                        resolve_path_case_insensitive(&user_dir, "My Documents")
                    {
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

/// Compare two paths for identity: equal verbatim, or equal once both
/// successfully canonicalize (resolving symlinks and `.`/`..` segments).
#[cfg(target_os = "linux")]
fn same_dir(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
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
    platform_search_locations_with_xdg(home, crate::proton::xdg_data_home_env().as_deref())
}

/// Build the list of directories to scan on Linux, honoring a custom
/// `$XDG_DATA_HOME` in addition to the standard/Flatpak/Snap locations.
#[cfg(target_os = "linux")]
fn platform_search_locations_with_xdg(
    home: &Path,
    xdg_data_home: Option<&OsStr>,
) -> Vec<SearchLocation> {
    // Normalize for Fedora Atomic / Bazzite (/var/home -> /home)
    let home = normalize_container_path(home);
    let home = home.as_path();
    let xdg_steam =
        crate::proton::xdg_steam_root(xdg_data_home).map(|p| normalize_container_path(&p));
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
        // Bottles (native install)
        SearchLocation {
            source: "Bottles",
            path: home
                .join(".local")
                .join("share")
                .join("bottles")
                .join("bottles"),
        },
        // Bottles (Flatpak)
        SearchLocation {
            source: "Bottles",
            path: home
                .join(".var")
                .join("app")
                .join("com.usebottles.bottles")
                .join("data")
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

    // Custom $XDG_DATA_HOME Steam install, unless it's the same directory as
    // one of the hard-coded locations above (e.g. XDG_DATA_HOME=~/.local/share
    // collapsing onto the default).
    if let Some(ref xdg_root) = xdg_steam {
        let compatdata = xdg_root.join("steamapps").join("compatdata");
        let already_present = locations.iter().any(|l| same_dir(&l.path, &compatdata));
        if !already_present {
            locations.push(SearchLocation {
                source: "Proton",
                path: compatdata,
            });
        }
    }

    // Snap Steam (Ubuntu's `steam` snap keeps its whole Steam root under the
    // snap's per-user `common` dir), unless it aliases a location above.
    //
    // Deliberately NOT added to `preferred_bottle_roots`: unlike
    // `$XDG_DATA_HOME`, nothing on the host says which Steam is active. The
    // snap remaps HOME, so its `~/.steam/steam` pointer lives under
    // `~/snap/steam/common/` and the host one keeps targeting native Steam;
    // `/snap/steam` only proves it's installed, and both can coexist. Same-
    // name native-vs-Snap prefixes therefore use the plain mtime policy.
    let snap_steam = home.join("snap/steam/common/.local/share/Steam");
    let snap_compatdata = snap_steam.join("steamapps").join("compatdata");
    if !locations
        .iter()
        .any(|l| same_dir(&l.path, &snap_compatdata))
    {
        locations.push(SearchLocation {
            source: "Proton",
            path: snap_compatdata,
        });
    }

    // Also scan secondary Steam library folders (e.g. SD card on Steam Deck)
    let mut steam_dirs = vec![
        home.join(".local/share/Steam"),
        home.join(".steam/steam"),
        snap_steam,
    ];
    steam_dirs.extend(xdg_steam);
    for steam_dir in &steam_dirs {
        for vdf_name in &["steamapps/libraryfolders.vdf", "config/libraryfolders.vdf"] {
            let vdf_path = steam_dir.join(vdf_name);
            if let Ok(content) = fs::read_to_string(&vdf_path) {
                for line in content.lines() {
                    if let Some(root) = steam_library_root_from_vdf_line(line) {
                        let lib_path = root.join("steamapps").join("compatdata");
                        // Avoid duplicates (e.g. the custom XDG root listed as
                        // entry "0" inside its own libraryfolders.vdf).
                        if !locations.iter().any(|l| same_dir(&l.path, &lib_path))
                            && lib_path.is_dir()
                        {
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

    locations
}

/// Compatdata root(s) that should win name-collision deduplication
/// (`deduplicate_bottles_by_name_preferring`) regardless of mtime: the
/// directory that the active `$XDG_DATA_HOME` Steam install's compatdata
/// resolves to among the locations actually scanned, plus the compatdata
/// dirs of any secondary Steam libraries listed in *that same XDG root's*
/// own `steamapps/libraryfolders.vdf` / `config/libraryfolders.vdf` (reusing
/// `steam_library_root_from_vdf_line`). A library listed only by a default
/// root's vdf (`~/.local/share/Steam`, `~/.steam/steam`) is never added here
/// — only the custom root's own library list counts.
///
/// Each candidate is mapped onto the matching scanned `SearchLocation` path
/// (Proton source, `same_dir`) so later `starts_with` checks line up with
/// the paths bottles were actually collected from; candidates with no
/// matching scanned location, or whose resolved path isn't a directory, are
/// dropped. A candidate whose compatdata is the same directory as a default
/// root's compatdata is also dropped unless it *is* the XDG primary itself —
/// otherwise a vdf alias could let the stale default copy be preferred
/// after all. The result is deduplicated by directory identity.
///
/// Empty when `$XDG_DATA_HOME` is unset or invalid.
#[cfg(target_os = "linux")]
fn preferred_bottle_roots(home: &Path, xdg_data_home: Option<&OsStr>) -> Vec<PathBuf> {
    let home = normalize_container_path(home);
    let Some(xdg_root) =
        crate::proton::xdg_steam_root(xdg_data_home).map(|p| normalize_container_path(&p))
    else {
        return Vec::new();
    };
    let compatdata = xdg_root.join("steamapps").join("compatdata");

    let locations = platform_search_locations_with_xdg(&home, xdg_data_home);
    let resolve = |candidate: &Path| {
        locations
            .iter()
            .find(|l| l.source == "Proton" && same_dir(&l.path, candidate))
            .map(|l| l.path.clone())
    };

    let primary = resolve(&compatdata).unwrap_or(compatdata);
    let mut roots = vec![primary.clone()];

    // Default roots' compatdata dirs — a secondary library that merely
    // aliases one of these (e.g. also happens to be listed by a default
    // root's own vdf) must not smuggle the stale default copy into the
    // preferred set.
    let default_compatdata: Vec<PathBuf> = [
        home.join(".local/share/Steam"),
        home.join(".steam/steam"),
    ]
    .iter()
    .map(|root| root.join("steamapps/compatdata"))
    .collect();

    for vdf_name in &["steamapps/libraryfolders.vdf", "config/libraryfolders.vdf"] {
        let Ok(content) = fs::read_to_string(xdg_root.join(vdf_name)) else {
            continue;
        };
        for line in content.lines() {
            let Some(library_root) = steam_library_root_from_vdf_line(line) else {
                continue;
            };
            let candidate = library_root.join("steamapps").join("compatdata");
            let Some(resolved) = resolve(&candidate) else {
                continue;
            };
            if !resolved.is_dir() {
                continue;
            }
            let aliases_default = default_compatdata.iter().any(|d| same_dir(d, &resolved));
            if aliases_default && !same_dir(&resolved, &primary) {
                continue;
            }
            if !roots.iter().any(|r| same_dir(r, &resolved)) {
                roots.push(resolved);
            }
        }
    }

    roots
}

/// Extract a Steam library root from one `libraryfolders.vdf` line.
///
/// Accepts both on-disk forms:
/// - modern: `"path"		"/mnt/games/SteamLibrary"` (inside a numbered block)
/// - legacy: `"1"		"/mnt/games/SteamLibrary"` (numeric index maps straight to the path)
///
/// The value comes from an external file, so only absolute paths without
/// `..` components or NUL bytes are accepted. This also filters out numeric
/// app-size entries such as `"228980"		"0"` in modern `"apps"` blocks.
#[cfg(target_os = "linux")]
fn steam_library_root_from_vdf_line(line: &str) -> Option<PathBuf> {
    let rest = line.trim().strip_prefix('"')?;
    let (key, rest) = rest.split_once('"')?;
    let rest = rest.trim_start().strip_prefix('"')?;
    let (value, _) = rest.split_once('"')?;

    let is_index = !key.is_empty() && key.bytes().all(|b| b.is_ascii_digit());
    if key != "path" && !is_index {
        return None;
    }
    if value.contains('\0') {
        return None;
    }
    let root = PathBuf::from(value.replace('\\', "/"));
    if !root.is_absolute()
        || root
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return None;
    }
    Some(root)
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
    deduplicate_bottles_by_name_preferring(bottles, &[])
}

/// Deduplicate a list of bottles by name (case-insensitive), with a
/// deterministic override for `preferred_roots`.
///
/// A bottle is "preferred" when its path starts with one of `preferred_roots`
/// (used on Linux to mark the active `$XDG_DATA_HOME` Steam compatdata root).
/// When both same-name duplicates are Proton prefixes and exactly one is
/// preferred, the preferred one always wins, regardless of mtime — this keeps
/// a stale default-root Proton prefix from shadowing the active custom-XDG
/// one. Collisions involving any non-Proton bottle (Heroic, Bottles, Wine,
/// CrossOver, ...) are never overridden. Otherwise, the existing behavior
/// applies: the instance with the most
/// recent `bottle.toml` (or `drive_c`) mtime wins, and the first occurrence
/// wins on a tie.
fn deduplicate_bottles_by_name_preferring(
    bottles: Vec<Bottle>,
    preferred_roots: &[PathBuf],
) -> Vec<Bottle> {
    use std::collections::HashMap;

    let is_preferred =
        |bottle: &Bottle| preferred_roots.iter().any(|root| bottle.path.starts_with(root));

    // Preserve insertion order so that the output is deterministic.  We build
    // a map from lowercase name -> index into `result`, and update the entry
    // whenever a replacement should win.
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
                let existing = &result[existing_idx];
                // The preferred-root override only arbitrates Proton-vs-Proton
                // collisions; any pairing involving a non-Proton bottle keeps
                // the plain mtime/first-wins policy.
                let both_proton = bottle.source == "Proton" && existing.source == "Proton";
                let replace = match (both_proton, is_preferred(&bottle), is_preferred(existing)) {
                    (true, true, false) => true,
                    (true, false, true) => false,
                    // Otherwise fall back to mtime, keep the one with the
                    // newer bottle.toml (or drive_c) mtime.
                    _ => bottle_mtime(&bottle) > bottle_mtime(existing),
                };
                if replace {
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
    // user has. On Linux, same-name Proton prefixes under the active custom
    // `$XDG_DATA_HOME` compatdata root always win over default-root
    // duplicates, regardless of mtime — see `preferred_bottle_roots`.
    #[cfg(target_os = "linux")]
    let preferred_roots = preferred_bottle_roots(&home, crate::proton::xdg_data_home_env().as_deref());
    #[cfg(not(target_os = "linux"))]
    let preferred_roots: Vec<PathBuf> = Vec::new();
    bottles = deduplicate_bottles_by_name_preferring(bottles, &preferred_roots);

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
    /// Asserts that `actual` names the same filesystem entry as `expected`.
    /// Case-insensitive filesystems (the macOS default) reach one directory
    /// through several spellings, so existing prefixes are compared by
    /// identity and the not-yet-existing tail must match exactly.
    fn assert_same_path(actual: &Path, expected: &Path) {
        fn split(path: &Path) -> (PathBuf, Vec<std::ffi::OsString>) {
            let mut base = path.to_path_buf();
            let mut rest = Vec::new();
            while !base.exists() {
                match base.file_name() {
                    Some(name) => rest.push(name.to_owned()),
                    None => break,
                }
                base.pop();
            }
            rest.reverse();
            (base, rest)
        }
        #[cfg(unix)]
        fn same_entry(a: &Path, b: &Path) -> bool {
            use std::os::unix::fs::MetadataExt;
            match (fs::metadata(a), fs::metadata(b)) {
                (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
                _ => a == b,
            }
        }
        #[cfg(not(unix))]
        fn same_entry(a: &Path, b: &Path) -> bool {
            a.canonicalize().ok() == b.canonicalize().ok()
        }
        let (actual_base, actual_rest) = split(actual);
        let (expected_base, expected_rest) = split(expected);
        assert!(
            actual_rest == expected_rest && same_entry(&actual_base, &expected_base),
            "{} is not the same path as {}",
            actual.display(),
            expected.display()
        );
    }

    fn create_fake_bottle(parent: &Path, name: &str) -> PathBuf {
        let bottle = parent.join(name);
        fs::create_dir_all(bottle.join("drive_c")).expect("create drive_c");
        bottle
    }

    #[cfg(target_os = "linux")]
    fn collect_fixture_bottles(home: &Path) -> Vec<Bottle> {
        collect_fixture_bottles_xdg(home, None)
    }

    #[cfg(target_os = "linux")]
    fn collect_fixture_bottles_xdg(home: &Path, xdg: Option<&OsStr>) -> Vec<Bottle> {
        let mut bottles = Vec::new();
        for location in platform_search_locations_with_xdg(home, xdg) {
            collect_bottles_from(&location, &mut bottles);
        }
        let preferred_roots = preferred_bottle_roots(home, xdg);
        deduplicate_bottles_by_name_preferring(bottles, &preferred_roots)
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn flatpak_bottles_collects_prefix() {
        let home = tempfile::tempdir().unwrap();
        let parent = home
            .path()
            .join(".var/app/com.usebottles.bottles/data/bottles/bottles");
        let prefix = create_fake_bottle(&parent, "FlatpakGame");

        let bottles = collect_fixture_bottles(home.path());

        assert_eq!(bottles.len(), 1);
        assert_eq!(bottles[0].name, "FlatpakGame");
        assert_eq!(bottles[0].path, prefix);
        assert_eq!(bottles[0].source, "Bottles");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn flatpak_bottles_coexists_with_native_prefix() {
        let home = tempfile::tempdir().unwrap();
        let flatpak = create_fake_bottle(
            &home
                .path()
                .join(".var/app/com.usebottles.bottles/data/bottles/bottles"),
            "FlatpakGame",
        );
        let native = create_fake_bottle(
            &home.path().join(".local/share/bottles/bottles"),
            "NativeGame",
        );

        let mut bottles = collect_fixture_bottles(home.path());
        bottles.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(bottles.len(), 2);
        assert_eq!(bottles[0].name, "FlatpakGame");
        assert_eq!(bottles[0].path, flatpak);
        assert_eq!(bottles[0].source, "Bottles");
        assert_eq!(bottles[1].name, "NativeGame");
        assert_eq!(bottles[1].path, native);
        assert_eq!(bottles[1].source, "Bottles");
    }

    /// Create `compatdata/{app_id}/pfx/drive_c` plus an appmanifest under
    /// `library/steamapps` and return the expected prefix path.
    #[cfg(target_os = "linux")]
    fn create_fake_proton_prefix(library: &Path, app_id: &str, name: &str) -> PathBuf {
        let steamapps = library.join("steamapps");
        let pfx = steamapps.join("compatdata").join(app_id).join("pfx");
        fs::create_dir_all(pfx.join("drive_c")).expect("create pfx/drive_c");
        fs::write(
            steamapps.join(format!("appmanifest_{app_id}.acf")),
            format!(
                "\"AppState\"\n{{\n\t\"appid\"\t\t\"{app_id}\"\n\t\"name\"\t\t\"{name}\"\n}}\n"
            ),
        )
        .expect("write appmanifest");
        pfx
    }

    #[cfg(target_os = "linux")]
    fn write_libraryfolders(home: &Path, content: &str) {
        let steamapps = home.join(".local/share/Steam/steamapps");
        fs::create_dir_all(&steamapps).expect("create steamapps");
        fs::write(steamapps.join("libraryfolders.vdf"), content).expect("write vdf");
    }

    #[cfg(target_os = "linux")]
    fn proton_bottles(home: &Path) -> Vec<Bottle> {
        collect_fixture_bottles(home)
            .into_iter()
            .filter(|b| b.source == "Proton")
            .collect()
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn secondary_steam_library_modern_vdf_discovers_proton_prefix() {
        let home = tempfile::tempdir().unwrap();
        let library = tempfile::tempdir().unwrap();
        let pfx = create_fake_proton_prefix(library.path(), "489830", "Skyrim Special Edition");
        write_libraryfolders(
            home.path(),
            &format!(
                "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{home}/.local/share/Steam\"\n\t\t\"apps\"\n\t\t{{\n\t\t\t\"228980\"\t\t\"0\"\n\t\t}}\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{lib}\"\n\t\t\"label\"\t\t\"\"\n\t\t\"apps\"\n\t\t{{\n\t\t\t\"489830\"\t\t\"12345\"\n\t\t}}\n\t}}\n}}\n",
                home = home.path().display(),
                lib = library.path().display(),
            ),
        );

        let bottles = proton_bottles(home.path());

        assert_eq!(bottles.len(), 1, "bottles: {bottles:?}");
        assert_eq!(bottles[0].name, "Skyrim Special Edition");
        assert_eq!(bottles[0].path, pfx);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn secondary_steam_library_legacy_numeric_vdf_discovers_proton_prefix() {
        let home = tempfile::tempdir().unwrap();
        let library = tempfile::tempdir().unwrap();
        let pfx = create_fake_proton_prefix(library.path(), "377160", "Fallout 4");
        write_libraryfolders(
            home.path(),
            &format!(
                "\"LibraryFolders\"\n{{\n\t\"TimeNextStatsReport\"\t\t\"1700000000\"\n\t\"ContentStatsID\"\t\t\"-123456789\"\n\t\"1\"\t\t\"{lib}\"\n}}\n",
                lib = library.path().display(),
            ),
        );

        let bottles = proton_bottles(home.path());

        assert_eq!(bottles.len(), 1, "bottles: {bottles:?}");
        assert_eq!(bottles[0].name, "Fallout 4");
        assert_eq!(bottles[0].path, pfx);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn secondary_steam_library_listed_twice_is_scanned_once() {
        let home = tempfile::tempdir().unwrap();
        let library = tempfile::tempdir().unwrap();
        create_fake_proton_prefix(library.path(), "489830", "Skyrim Special Edition");
        let lib = library.path().display();
        // Both the steamapps/ and config/ copies list the same library, in
        // different formats.
        write_libraryfolders(
            home.path(),
            &format!("\"LibraryFolders\"\n{{\n\t\"1\"\t\t\"{lib}\"\n}}\n"),
        );
        let config = home.path().join(".local/share/Steam/config");
        fs::create_dir_all(&config).unwrap();
        fs::write(
            config.join("libraryfolders.vdf"),
            format!("\"libraryfolders\"\n{{\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{lib}\"\n\t}}\n}}\n"),
        )
        .unwrap();

        let locations: Vec<_> = platform_search_locations(home.path())
            .into_iter()
            .filter(|l| l.path.starts_with(library.path()))
            .collect();
        assert_eq!(locations.len(), 1);
        assert_eq!(proton_bottles(home.path()).len(), 1);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn custom_xdg_data_home_discovers_proton_prefixes() {
        let home = tempfile::tempdir().unwrap();
        let xdg = tempfile::tempdir().unwrap();
        let xdg_steam = xdg.path().join("Steam");
        let primary_pfx = create_fake_proton_prefix(&xdg_steam, "489830", "Skyrim Special Edition");

        let secondary_library = tempfile::tempdir().unwrap();
        let secondary_pfx =
            create_fake_proton_prefix(secondary_library.path(), "377160", "Fallout 4");

        fs::create_dir_all(xdg_steam.join("steamapps")).unwrap();
        fs::write(
            xdg_steam.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{primary}\"\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{secondary}\"\n\t}}\n}}\n",
                primary = xdg_steam.display(),
                secondary = secondary_library.path().display(),
            ),
        )
        .unwrap();

        // Without XDG_DATA_HOME, the custom root is never considered.
        assert!(proton_bottles(home.path()).is_empty());

        let xdg_os = xdg_steam_env(&xdg);
        let mut bottles = proton_bottles_xdg(home.path(), Some(&xdg_os));
        bottles.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(bottles.len(), 2, "bottles: {bottles:?}");
        assert_eq!(bottles[0].name, "Fallout 4");
        assert_eq!(bottles[0].path, secondary_pfx);
        assert_eq!(bottles[1].name, "Skyrim Special Edition");
        assert_eq!(bottles[1].path, primary_pfx);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn snap_steam_primary_and_secondary_libraries_discover_proton_prefixes() {
        let home = tempfile::tempdir().unwrap();
        let snap_steam = home.path().join("snap/steam/common/.local/share/Steam");
        let primary_pfx =
            create_fake_proton_prefix(&snap_steam, "489830", "Skyrim Special Edition");

        let secondary_library = tempfile::tempdir().unwrap();
        let secondary_pfx =
            create_fake_proton_prefix(secondary_library.path(), "377160", "Fallout 4");

        // Snap Steam's own vdf lists itself as entry "0" plus a secondary
        // library, and also carries an unsafe relative entry that must be
        // ignored.
        fs::write(
            snap_steam.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{primary}\"\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{secondary}\"\n\t}}\n\t\"2\"\n\t{{\n\t\t\"path\"\t\t\"../escape\"\n\t}}\n}}\n",
                primary = snap_steam.display(),
                secondary = secondary_library.path().display(),
            ),
        )
        .unwrap();

        let mut bottles = proton_bottles(home.path());
        bottles.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(bottles.len(), 2, "bottles: {bottles:?}");
        assert_eq!(bottles[0].name, "Fallout 4");
        assert_eq!(bottles[0].path, secondary_pfx);
        assert_eq!(bottles[1].name, "Skyrim Special Edition");
        assert_eq!(bottles[1].path, primary_pfx);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn snap_steam_libraries_are_deduplicated_against_other_roots() {
        let home = tempfile::tempdir().unwrap();
        let snap_steam = home.path().join("snap/steam/common/.local/share/Steam");
        create_fake_proton_prefix(&snap_steam, "489830", "Skyrim Special Edition");
        let snap_compatdata = snap_steam.join("steamapps/compatdata");

        let shared_library = tempfile::tempdir().unwrap();
        create_fake_proton_prefix(shared_library.path(), "377160", "Fallout 4");
        let shared = shared_library.path().display();

        // The secondary library is declared by both the native and the Snap
        // Steam roots (in both vdf formats); the Snap vdf also lists the Snap
        // root itself, which is already a primary location.
        write_libraryfolders(
            home.path(),
            &format!("\"LibraryFolders\"\n{{\n\t\"1\"\t\t\"{shared}\"\n}}\n"),
        );
        fs::write(
            snap_steam.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{snap}\"\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{shared}\"\n\t}}\n}}\n",
                snap = snap_steam.display(),
            ),
        )
        .unwrap();

        let locations = platform_search_locations(home.path());
        let snap_hits = locations
            .iter()
            .filter(|l| same_dir(&l.path, &snap_compatdata))
            .count();
        let shared_hits = locations
            .iter()
            .filter(|l| l.path.starts_with(shared_library.path()))
            .count();
        assert_eq!(snap_hits, 1);
        assert_eq!(shared_hits, 1);
        assert_eq!(proton_bottles(home.path()).len(), 2);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn snap_steam_root_aliasing_default_root_is_not_duplicated() {
        // Some setups symlink the Snap Steam root onto the native one; the
        // Snap compatdata must then collapse onto the existing location.
        let home = tempfile::tempdir().unwrap();
        let native = home.path().join(".local/share/Steam");
        create_fake_proton_prefix(&native, "489830", "Skyrim Special Edition");
        let snap_parent = home.path().join("snap/steam/common/.local/share");
        fs::create_dir_all(&snap_parent).unwrap();
        std::os::unix::fs::symlink(&native, snap_parent.join("Steam")).unwrap();

        let native_compatdata = native.join("steamapps/compatdata");
        let hits = platform_search_locations(home.path())
            .into_iter()
            .filter(|l| same_dir(&l.path, &native_compatdata))
            .count();
        assert_eq!(hits, 1);
        assert_eq!(proton_bottles(home.path()).len(), 1);
    }

    #[cfg(target_os = "linux")]
    fn set_drive_c_mtime(pfx: &Path, secs: i64) {
        filetime::set_file_mtime(
            pfx.join("drive_c"),
            filetime::FileTime::from_unix_time(secs, 0),
        )
        .unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn native_vs_snap_same_name_prefix_uses_mtime_not_snap_priority() {
        // No host-side signal identifies the active Steam between native and
        // Snap, so neither root is preferred: the newer prefix wins either way.
        for snap_is_newer in [true, false] {
            let home = tempfile::tempdir().unwrap();
            let native_pfx = create_fake_proton_prefix(
                &home.path().join(".local/share/Steam"),
                "489830",
                "Skyrim Special Edition",
            );
            let snap_pfx = create_fake_proton_prefix(
                &home.path().join("snap/steam/common/.local/share/Steam"),
                "489830",
                "Skyrim Special Edition",
            );
            let (old, new) = if snap_is_newer {
                (&native_pfx, &snap_pfx)
            } else {
                (&snap_pfx, &native_pfx)
            };
            set_drive_c_mtime(old, 1_000_000);
            set_drive_c_mtime(new, 2_000_000);

            let bottles = proton_bottles(home.path());
            assert_eq!(bottles.len(), 1, "snap_is_newer={snap_is_newer}");
            assert_same_path(&bottles[0].path, new);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn custom_xdg_prefix_beats_newer_snap_same_name_prefix() {
        let home = tempfile::tempdir().unwrap();
        let xdg = tempfile::tempdir().unwrap();
        let xdg_pfx = create_fake_proton_prefix(
            &xdg.path().join("Steam"),
            "489830",
            "Skyrim Special Edition",
        );
        let snap_pfx = create_fake_proton_prefix(
            &home.path().join("snap/steam/common/.local/share/Steam"),
            "489830",
            "Skyrim Special Edition",
        );
        set_drive_c_mtime(&xdg_pfx, 1_000_000);
        set_drive_c_mtime(&snap_pfx, 2_000_000);

        let xdg_os = xdg_steam_env(&xdg);
        let bottles = proton_bottles_xdg(home.path(), Some(&xdg_os));
        assert_eq!(bottles.len(), 1);
        assert_same_path(&bottles[0].path, &xdg_pfx);
    }

    #[cfg(target_os = "linux")]
    fn xdg_steam_env(xdg: &tempfile::TempDir) -> std::ffi::OsString {
        xdg.path().as_os_str().to_owned()
    }

    #[cfg(target_os = "linux")]
    fn proton_bottles_xdg(home: &Path, xdg: Option<&OsStr>) -> Vec<Bottle> {
        collect_fixture_bottles_xdg(home, xdg)
            .into_iter()
            .filter(|b| b.source == "Proton")
            .collect()
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn custom_xdg_data_home_equal_to_default_is_not_duplicated() {
        let home = tempfile::tempdir().unwrap();
        let xdg = home.path().join(".local/share");
        let steam = xdg.join("Steam");
        let pfx = create_fake_proton_prefix(&steam, "489830", "Skyrim Special Edition");

        let xdg_os = xdg.as_os_str();
        let bottles = proton_bottles_xdg(home.path(), Some(xdg_os));
        assert_eq!(bottles.len(), 1, "bottles: {bottles:?}");
        assert_eq!(bottles[0].path, pfx);

        let compatdata = steam.join("steamapps/compatdata");
        let matching = platform_search_locations_with_xdg(home.path(), Some(xdg_os))
            .into_iter()
            .filter(|l| l.path == compatdata)
            .count();
        assert_eq!(matching, 1);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn relative_or_empty_xdg_data_home_is_ignored_in_bottle_search() {
        let home = tempfile::tempdir().unwrap();
        create_fake_bottle(&home.path().join(".local/share/bottles/bottles"), "Game");

        let default_paths: Vec<PathBuf> = platform_search_locations_with_xdg(home.path(), None)
            .into_iter()
            .map(|l| l.path)
            .collect();

        for invalid in ["relative/data", ""] {
            let xdg = OsStr::new(invalid);
            let paths: Vec<PathBuf> = platform_search_locations_with_xdg(home.path(), Some(xdg))
                .into_iter()
                .map(|l| l.path)
                .collect();
            assert_eq!(paths, default_paths, "value: {invalid:?}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn steam_library_root_from_vdf_line_accepts_modern_and_legacy() {
        assert_eq!(
            steam_library_root_from_vdf_line("\t\t\"path\"\t\t\"/mnt/games/SteamLibrary\""),
            Some(PathBuf::from("/mnt/games/SteamLibrary"))
        );
        assert_eq!(
            steam_library_root_from_vdf_line("\t\"1\"\t\t\"/run/media/deck/SD Card\""),
            Some(PathBuf::from("/run/media/deck/SD Card"))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn steam_library_root_from_vdf_line_rejects_non_library_and_unsafe_lines() {
        for line in [
            r#""label"		"""#,
            r#""contentid"		"1234567890""#,
            r#""ContentStatsID"		"-123456789""#,
            r#""TimeNextStatsReport"		"1700000000""#,
            r#""228980"		"0""#,
            r#""1"		"relative/SteamLibrary""#,
            r#""path"		"relative/SteamLibrary""#,
            r#""path"		"""#,
            r#""path"		"/mnt/../etc""#,
            r#""1"		"/mnt/games/../../etc""#,
            r#""1a"		"/mnt/games/SteamLibrary""#,
            r#""-1"		"/mnt/games/SteamLibrary""#,
            r#""path"		"/unterminated"#,
            r#""path" /unquoted"#,
            r#"path		"/mnt/games/SteamLibrary""#,
            r#""libraryfolders""#,
            r#""0""#,
            "{",
            "}",
            "",
            "\"path\"\t\"/mnt/ga\0mes\"",
        ] {
            assert_eq!(
                steam_library_root_from_vdf_line(line),
                None,
                "line: {line:?}"
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn flatpak_bottles_skips_missing_and_invalid_prefixes() {
        let home = tempfile::tempdir().unwrap();
        assert!(collect_fixture_bottles(home.path()).is_empty());

        let parent = home
            .path()
            .join(".var/app/com.usebottles.bottles/data/bottles/bottles");
        fs::create_dir_all(parent.join("MissingDriveC")).unwrap();
        fs::write(parent.join("NotADirectory"), "not a prefix").unwrap();
        assert!(collect_fixture_bottles(home.path()).is_empty());

        let valid = create_fake_bottle(&parent, "ValidGame");
        let bottles = collect_fixture_bottles(home.path());
        assert_eq!(bottles.len(), 1);
        assert_eq!(bottles[0].name, "ValidGame");
        assert_eq!(bottles[0].path, valid);
        assert_eq!(bottles[0].source, "Bottles");
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
    fn user_data_paths_match_mixed_case_modern_and_legacy_layouts() {
        for (local, documents) in [
            ("aPpDaTa/lOcAl", "dOcUmEnTs"),
            ("lOcAl SeTtInGs/aPpLiCaTiOn DaTa", "mY dOcUmEnTs"),
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let bottle = Bottle {
                name: "Test".into(),
                path: create_fake_bottle(tmp.path(), "Bottle"),
                source: "Test".into(),
            };
            let users = bottle.drive_c().join("UsErS");
            let user = users.join("Player");
            fs::create_dir_all(user.join(local)).unwrap();
            fs::create_dir_all(user.join(documents)).unwrap();

            assert_same_path(&bottle.users_dir(), &users);
            assert_same_path(&bottle.appdata_local(), &user.join(local));
            assert_same_path(&bottle.documents_dir(), &user.join(documents));
        }
    }

    #[test]
    fn user_data_paths_prefer_modern_over_legacy() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = Bottle {
            name: "Test".into(),
            path: create_fake_bottle(tmp.path(), "Bottle"),
            source: "Test".into(),
        };
        let user = bottle.drive_c().join("users/Player");
        for relative in [
            "aPpDaTa/lOcAl",
            "Local Settings/Application Data",
            "dOcUmEnTs",
            "My Documents",
        ] {
            fs::create_dir_all(user.join(relative)).unwrap();
        }
        assert_same_path(&bottle.appdata_local(), &user.join("aPpDaTa/lOcAl"));
        assert_same_path(&bottle.documents_dir(), &user.join("dOcUmEnTs"));
    }

    // Linux permits distinct entries whose names differ only in case.
    #[cfg(target_os = "linux")]
    #[test]
    fn user_data_paths_prefer_exact_case() {
        for (local, documents, alternate_local, alternate_documents) in [
            ("AppData/Local", "Documents", "appdata/local", "documents"),
            (
                "Local Settings/Application Data",
                "My Documents",
                "local settings/application data",
                "my documents",
            ),
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let bottle = Bottle {
                name: "Test".into(),
                path: create_fake_bottle(tmp.path(), "Bottle"),
                source: "Test".into(),
            };
            let users = bottle.drive_c().join("users");
            let user = users.join("Player");
            fs::create_dir_all(bottle.drive_c().join("USERS/Other/AppData/Local")).unwrap();
            fs::create_dir_all(bottle.drive_c().join("USERS/Other/Documents")).unwrap();
            for relative in [alternate_local, alternate_documents, local, documents] {
                fs::create_dir_all(user.join(relative)).unwrap();
            }
            // Also exercise exact preference at the final AppData component.
            if local == "AppData/Local" {
                fs::create_dir_all(user.join("AppData/local")).unwrap();
            } else {
                fs::create_dir_all(user.join("Local Settings/application data")).unwrap();
            }
            assert_eq!(bottle.users_dir(), users);
            assert_eq!(bottle.appdata_local(), user.join(local));
            assert_eq!(bottle.documents_dir(), user.join(documents));
        }
    }

    #[test]
    fn user_data_paths_preserve_cross_over_fallbacks() {
        for users_name in ["users", "UsErS"] {
            let tmp = tempfile::tempdir().unwrap();
            let bottle = Bottle {
                name: "Test".into(),
                path: create_fake_bottle(tmp.path(), "Bottle"),
                source: "Test".into(),
            };
            let default_users = bottle.drive_c().join("users");
            assert_same_path(&bottle.users_dir(), &default_users);
            assert_same_path(&bottle.appdata_local(), &default_users.join("crossover/AppData/Local"));
            assert_same_path(&bottle.documents_dir(), &default_users.join("crossover/Documents"));

            let users = bottle.drive_c().join(users_name);
            fs::create_dir_all(&users).unwrap();
            for populated in [false, true] {
                if populated {
                    fs::create_dir_all(users.join("Player/AppData")).unwrap();
                    fs::create_dir_all(users.join("Player/Local Settings")).unwrap();
                    fs::write(users.join("not-a-user"), "").unwrap();
                }
                assert_same_path(&bottle.users_dir(), &users);
                assert_same_path(&bottle.appdata_local(), &users.join("crossover/AppData/Local"));
                assert_same_path(&bottle.documents_dir(), &users.join("crossover/Documents"));
            }
        }
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

    /// Non-preferred duplicates still fall back to mtime/first-wins.
    #[test]
    fn deduplicate_bottles_by_name_preferring_falls_back_to_mtime_when_not_preferred() {
        let tmp = tempfile::tempdir().unwrap();

        let path_old = create_fake_bottle(tmp.path(), "MySkyrim_old");
        let old_toml = path_old.join("bottle.toml");
        fs::write(&old_toml, "[bottle]\nname = \"MySkyrim\"\n").unwrap();
        filetime::set_file_mtime(&old_toml, filetime::FileTime::from_unix_time(1_000_000, 0))
            .unwrap();

        let path_new = create_fake_bottle(tmp.path(), "MySkyrim_new");
        let new_toml = path_new.join("bottle.toml");
        fs::write(&new_toml, "[bottle]\nname = \"MySkyrim\"\n").unwrap();
        filetime::set_file_mtime(&new_toml, filetime::FileTime::from_unix_time(2_000_000, 0))
            .unwrap();

        let bottles = vec![
            Bottle { name: "MySkyrim".into(), path: path_old.clone(), source: "CrossOver".into() },
            Bottle { name: "MySkyrim".into(), path: path_new.clone(), source: "CrossOver".into() },
        ];

        // Neither path is under the (unrelated) preferred root, so this must
        // behave exactly like `deduplicate_bottles_by_name` (mtime wins).
        let unrelated_root = tmp.path().join("somewhere-else");
        let deduped = deduplicate_bottles_by_name_preferring(bottles, &[unrelated_root]);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].path, path_new, "newer bottle.toml should still win");
    }

    /// A preferred-root bottle wins over a same-name duplicate even when the
    /// non-preferred duplicate has a strictly newer mtime.
    #[test]
    fn deduplicate_bottles_by_name_preferring_ignores_mtime_for_preferred_root() {
        let tmp = tempfile::tempdir().unwrap();

        let preferred_root = tmp.path().join("preferred");
        let path_preferred = create_fake_bottle(&preferred_root, "MySkyrim_preferred");
        filetime::set_file_mtime(
            path_preferred.join("drive_c"),
            filetime::FileTime::from_unix_time(1_000_000, 0),
        )
        .unwrap();

        let path_stale = create_fake_bottle(tmp.path(), "MySkyrim_stale");
        filetime::set_file_mtime(
            path_stale.join("drive_c"),
            filetime::FileTime::from_unix_time(2_000_000, 0),
        )
        .unwrap();

        let bottles = vec![
            Bottle { name: "MySkyrim".into(), path: path_stale.clone(), source: "Proton".into() },
            Bottle { name: "MySkyrim".into(), path: path_preferred.clone(), source: "Proton".into() },
        ];

        let deduped = deduplicate_bottles_by_name_preferring(bottles, &[preferred_root]);
        assert_eq!(deduped.len(), 1);
        assert_eq!(
            deduped[0].path, path_preferred,
            "preferred root should win regardless of mtime"
        );
    }

    /// Regression: the preferred-root override is scoped to Proton-vs-Proton
    /// collisions. A custom-XDG Proton prefix must NOT automatically shadow a
    /// newer same-name non-Proton bottle — mtime still decides, in either
    /// insertion order.
    #[test]
    fn deduplicate_bottles_by_name_preferring_does_not_override_non_proton() {
        let tmp = tempfile::tempdir().unwrap();

        let preferred_root = tmp.path().join("preferred");
        let path_proton = create_fake_bottle(&preferred_root, "MySkyrim_proton");
        filetime::set_file_mtime(
            path_proton.join("drive_c"),
            filetime::FileTime::from_unix_time(1_000_000, 0),
        )
        .unwrap();

        let path_heroic = create_fake_bottle(tmp.path(), "MySkyrim_heroic");
        filetime::set_file_mtime(
            path_heroic.join("drive_c"),
            filetime::FileTime::from_unix_time(2_000_000, 0),
        )
        .unwrap();

        let proton =
            Bottle { name: "MySkyrim".into(), path: path_proton.clone(), source: "Proton".into() };
        let heroic =
            Bottle { name: "MySkyrim".into(), path: path_heroic.clone(), source: "Heroic".into() };

        for bottles in [vec![proton.clone(), heroic.clone()], vec![heroic, proton]] {
            let deduped =
                deduplicate_bottles_by_name_preferring(bottles, &[preferred_root.clone()]);
            assert_eq!(deduped.len(), 1);
            assert_eq!(
                deduped[0].path, path_heroic,
                "newer non-Proton bottle should win over preferred-root Proton prefix"
            );
        }
    }

    /// Regression: a same-name Proton prefix under the active custom
    /// `$XDG_DATA_HOME` compatdata root must win over a stale duplicate under
    /// the default `~/.local/share/Steam` root, even when the default
    /// prefix's mtime is newer.
    #[cfg(target_os = "linux")]
    #[test]
    fn xdg_proton_prefix_wins_over_newer_stale_default_prefix() {
        let home = tempfile::tempdir().unwrap();
        let xdg = tempfile::tempdir().unwrap();

        // Default library: ~/.local/share/Steam/steamapps/compatdata/489830
        let default_library = home.path().join(".local/share/Steam");
        let default_pfx =
            create_fake_proton_prefix(&default_library, "489830", "Skyrim Special Edition");

        // Custom XDG library: $XDG_DATA_HOME/Steam/steamapps/compatdata/489830
        let xdg_steam = xdg.path().join("Steam");
        let xdg_pfx =
            create_fake_proton_prefix(&xdg_steam, "489830", "Skyrim Special Edition");

        // Give the default (stale) prefix a strictly newer mtime than the
        // active XDG one — a plain mtime-based dedupe would pick it, which
        // is the bug this test guards against.
        filetime::set_file_mtime(
            xdg_pfx.join("drive_c"),
            filetime::FileTime::from_unix_time(1_000_000, 0),
        )
        .unwrap();
        filetime::set_file_mtime(
            default_pfx.join("drive_c"),
            filetime::FileTime::from_unix_time(2_000_000, 0),
        )
        .unwrap();

        let xdg_os = xdg.path().as_os_str();
        let bottles = proton_bottles_xdg(home.path(), Some(xdg_os));

        assert_eq!(bottles.len(), 1, "bottles: {bottles:?}");
        assert_eq!(bottles[0].name, "Skyrim Special Edition");
        assert_eq!(bottles[0].path, xdg_pfx);
    }

    /// Regression: a same-name Proton prefix in a *secondary* Steam library
    /// must also win over a stale default-root duplicate, as long as that
    /// secondary library is listed in the active custom `$XDG_DATA_HOME`
    /// root's own libraryfolders.vdf — not just the XDG root's primary
    /// compatdata.
    #[cfg(target_os = "linux")]
    #[test]
    fn xdg_secondary_library_proton_prefix_wins_over_newer_stale_default_prefix() {
        let home = tempfile::tempdir().unwrap();
        let xdg = tempfile::tempdir().unwrap();
        let secondary_library = tempfile::tempdir().unwrap();

        // Stale default-root copy, given a newer mtime (the bug this guards
        // against: a plain mtime-based dedupe would pick this one).
        let default_library = home.path().join(".local/share/Steam");
        let default_pfx =
            create_fake_proton_prefix(&default_library, "489830", "Skyrim Special Edition");

        // Active copy lives in a secondary library that only the custom XDG
        // root's own libraryfolders.vdf lists.
        let xdg_pfx =
            create_fake_proton_prefix(secondary_library.path(), "489830", "Skyrim Special Edition");
        let xdg_steam = xdg.path().join("Steam");
        fs::create_dir_all(xdg_steam.join("steamapps")).unwrap();
        fs::write(
            xdg_steam.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{secondary}\"\n\t}}\n}}\n",
                secondary = secondary_library.path().display(),
            ),
        )
        .unwrap();

        filetime::set_file_mtime(
            xdg_pfx.join("drive_c"),
            filetime::FileTime::from_unix_time(1_000_000, 0),
        )
        .unwrap();
        filetime::set_file_mtime(
            default_pfx.join("drive_c"),
            filetime::FileTime::from_unix_time(2_000_000, 0),
        )
        .unwrap();

        let xdg_os = xdg_steam_env(&xdg);
        let bottles = proton_bottles_xdg(home.path(), Some(&xdg_os));

        assert_eq!(bottles.len(), 1, "bottles: {bottles:?}");
        assert_eq!(bottles[0].name, "Skyrim Special Edition");
        assert_eq!(
            bottles[0].path, xdg_pfx,
            "secondary-library copy listed by the XDG root's own vdf should win"
        );
    }

    /// Regression: a secondary library listed only by a *default* root's own
    /// libraryfolders.vdf must NOT be treated as preferred, even while an
    /// unrelated custom `$XDG_DATA_HOME` is active — mtime still decides,
    /// exactly as it did before secondary libraries were considered at all.
    #[cfg(target_os = "linux")]
    #[test]
    fn default_root_secondary_library_proton_prefix_is_not_preferred() {
        let home = tempfile::tempdir().unwrap();
        let xdg = tempfile::tempdir().unwrap();
        let secondary_library = tempfile::tempdir().unwrap();

        // Secondary library is listed only by the DEFAULT root's own vdf.
        write_libraryfolders(
            home.path(),
            &format!(
                "\"libraryfolders\"\n{{\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{secondary}\"\n\t}}\n}}\n",
                secondary = secondary_library.path().display(),
            ),
        );

        let default_library = home.path().join(".local/share/Steam");
        let default_pfx =
            create_fake_proton_prefix(&default_library, "489830", "Skyrim Special Edition");
        let secondary_pfx =
            create_fake_proton_prefix(secondary_library.path(), "489830", "Skyrim Special Edition");

        // Secondary (default-vdf) copy is older; default primary copy is
        // newer, and should win since neither is preferred here.
        filetime::set_file_mtime(
            secondary_pfx.join("drive_c"),
            filetime::FileTime::from_unix_time(1_000_000, 0),
        )
        .unwrap();
        filetime::set_file_mtime(
            default_pfx.join("drive_c"),
            filetime::FileTime::from_unix_time(2_000_000, 0),
        )
        .unwrap();

        let xdg_os = xdg_steam_env(&xdg);
        let bottles = proton_bottles_xdg(home.path(), Some(&xdg_os));

        assert_eq!(bottles.len(), 1, "bottles: {bottles:?}");
        assert_eq!(
            bottles[0].path, default_pfx,
            "a secondary library listed only by the default root's vdf must not be preferred; mtime decides"
        );
    }
}
