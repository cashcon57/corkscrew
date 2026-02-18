//! Wine bottle detection for CrossOver, Whisky, Moonshine, Mythic, Heroic,
//! and native Wine/Proton managers on macOS and Linux.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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
        // CrossOver
        SearchLocation {
            source: "CrossOver",
            path: home
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
    vec![
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
        // Steam / Proton
        SearchLocation {
            source: "Proton",
            path: home
                .join(".local")
                .join("share")
                .join("Steam")
                .join("steamapps")
                .join("compatdata"),
        },
    ]
}

// ---------------------------------------------------------------------------
// Detection helpers
// ---------------------------------------------------------------------------

/// Native Wine on Linux stores its prefix directly at `~/.wine` rather than
/// as a subdirectory. This constant tracks the source label that uses that
/// layout so we can handle it specially.
#[cfg(target_os = "linux")]
const DIRECT_PREFIX_SOURCE: &str = "Wine";

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

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Scan all known locations for Wine bottles and return every valid bottle
/// found. A bottle is considered valid if its `drive_c` directory exists.
pub fn detect_bottles() -> Vec<Bottle> {
    let Some(home) = dirs::home_dir() else {
        log::warn!("Could not determine home directory; no bottles detected.");
        return Vec::new();
    };

    let locations = platform_search_locations(&home);
    let mut bottles = Vec::new();

    for location in &locations {
        collect_bottles_from(location, &mut bottles);
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
}
