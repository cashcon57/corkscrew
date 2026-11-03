//! GameHub (GameSir's Wine-based launcher for macOS, bundle `com.gamemac.www`).
//!
//! GameHub keeps all of its state under
//! `~/Library/Application Support/com.gamemac.www/`. Its Wine prefixes are
//! split in two (checked against GameHub 0.8.613):
//!
//! - `wine-engine/containers/base_containers/<id>`: a shared, pristine prefix
//!   per Wine version. GameHub layers it underneath each game's container at
//!   runtime and never writes game data into it.
//! - `wine-engine/containers/virtual_containers/<id>`: one per game, the
//!   writable layer. It holds `drive_c/users/steamuser` (AppData, Documents,
//!   saves) and a per-container Wine Steam install whose
//!   `steamapps/common/<installdir>` entries are symlinks to the host install
//!   (by default `~/GameHub/steamapps/common/<installdir>`). Only the bound
//!   game (plus shared redistributables) is linked there.
//!
//! So a virtual container is a normal single-game bottle: the in-prefix Steam
//! layout resolves the game, and mods and plugin lists land where the game
//! reads them. Base containers are never listed as bottles.
//!
//! The virtual container on disk is only the top layer. GameHub assembles the
//! full prefix when it launches a game, so Corkscrew must never run Wine on it
//! directly; games are started through GameHub's `gamehub://` deep link.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::bottles::Bottle;

/// `Bottle::source` value for GameHub virtual containers.
pub const SOURCE: &str = "GameHub";

/// GameHub's data root relative to the user's home directory.
const DATA_ROOT: &[&str] = &["Library", "Application Support", "com.gamemac.www"];

/// Virtual containers relative to the data root.
const VIRTUAL_CONTAINERS: &[&str] = &["wine-engine", "containers", "virtual_containers"];

/// Game ↔ container bindings relative to the data root.
const CONTAINER_STORE: &[&str] = &["gamehub", "game_container_store.json"];

fn join_all(base: &Path, parts: &[&str]) -> PathBuf {
    parts.iter().fold(base.to_path_buf(), |p, part| p.join(part))
}

/// Directory holding GameHub's per-game virtual containers.
pub fn virtual_containers_dir(home: &Path) -> PathBuf {
    join_all(&join_all(home, DATA_ROOT), VIRTUAL_CONTAINERS)
}

/// A container ID is a positive decimal integer (GameHub rejects anything
/// else, see its "virtual_container_id must be a positive canonical integer").
fn is_container_id(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 20
        && name.bytes().all(|b| b.is_ascii_digit())
        && !name.starts_with('0')
}

/// Collect GameHub virtual containers from `dir` as bottles named
/// `GameHub <id>`.
///
/// The name uses the container ID rather than the bound game's title so it
/// never changes: Corkscrew keys mods and deployments by bottle name, and a
/// title read from GameHub's (concurrently written) JSON could fail to parse
/// and silently rename the bottle. Symlinked container directories are
/// skipped so a planted link can't point a "bottle" elsewhere.
pub fn collect_bottles(dir: &Path, bottles: &mut Vec<Bottle>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    let mut found: Vec<(u64, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_string();
            if !is_container_id(&name) {
                return None;
            }
            // file_type() does not follow symlinks.
            if !entry.file_type().ok()?.is_dir() {
                return None;
            }
            Some((name.parse().ok()?, entry.path()))
        })
        .collect();
    found.sort();

    for (id, path) in found {
        let bottle = Bottle {
            name: format!("{} {}", SOURCE, id),
            path,
            source: SOURCE.to_string(),
        };
        if bottle.exists() {
            bottles.push(bottle);
        }
    }
}

#[derive(Deserialize)]
struct ContainerStore {
    #[serde(default)]
    bindings: Vec<Binding>,
}

#[derive(Deserialize)]
struct Binding {
    #[serde(default)]
    platform: Option<String>,
    #[serde(default)]
    platform_app_id: Option<String>,
    /// Written as a string (`"1"`), accepted as a number too.
    #[serde(default)]
    virtual_container_id: Option<serde_json::Value>,
}

impl Binding {
    fn container_id(&self) -> Option<String> {
        match self.virtual_container_id.as_ref()? {
            serde_json::Value::String(s) => Some(s.trim().to_string()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    }
}

/// The container ID and GameHub data root for a GameHub bottle, derived from
/// the bottle path `<root>/wine-engine/containers/virtual_containers/<id>`.
fn container_location(bottle: &Bottle) -> Option<(String, PathBuf)> {
    if bottle.source != SOURCE {
        return None;
    }
    let id = bottle.path.file_name()?.to_str()?;
    if !is_container_id(id) {
        return None;
    }
    let mut dir = bottle.path.parent()?;
    for expected in VIRTUAL_CONTAINERS.iter().rev() {
        if dir.file_name()?.to_str()? != *expected {
            return None;
        }
        dir = dir.parent()?;
    }
    Some((id.to_string(), dir.to_path_buf()))
}

/// The Steam app ID GameHub has bound to this bottle's virtual container,
/// from `gamehub/game_container_store.json`. `None` for non-GameHub bottles,
/// non-Steam bindings, or when the store can't be read.
pub fn bound_steam_app_id(bottle: &Bottle) -> Option<String> {
    let (id, root) = container_location(bottle)?;
    let store_path = join_all(&root, CONTAINER_STORE);
    let content = fs::read_to_string(&store_path).ok()?;
    let store: ContainerStore = match serde_json::from_str(&content) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("Failed to parse {}: {}", store_path.display(), e);
            return None;
        }
    };
    store
        .bindings
        .iter()
        .filter(|b| b.container_id().as_deref() == Some(id.as_str()))
        .filter(|b| {
            b.platform
                .as_deref()
                .is_some_and(|p| p.eq_ignore_ascii_case("steam"))
        })
        .find_map(|b| {
            let app_id = b.platform_app_id.as_deref()?.trim();
            is_steam_app_id(app_id).then(|| app_id.to_string())
        })
}

fn is_steam_app_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 10 && id.bytes().all(|b| b.is_ascii_digit())
}

/// GameHub deep link that launches a Steam game the same way its Play button
/// does. GameHub's own desktop shortcuts use this scheme.
pub fn steam_launch_url(app_id: &str) -> Option<String> {
    is_steam_app_id(app_id).then(|| format!("gamehub://launch/steam/{}", app_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        containers: PathBuf,
    }

    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let root = join_all(tmp.path(), DATA_ROOT);
        let containers = join_all(&root, VIRTUAL_CONTAINERS);
        fs::create_dir_all(&containers).unwrap();
        Fixture {
            _tmp: tmp,
            root,
            containers,
        }
    }

    fn make_container(fx: &Fixture, id: &str) -> PathBuf {
        let path = fx.containers.join(id);
        fs::create_dir_all(path.join("drive_c").join("users").join("steamuser")).unwrap();
        path
    }

    fn write_store(fx: &Fixture, json: &str) {
        let path = join_all(&fx.root, CONTAINER_STORE);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, json).unwrap();
    }

    fn bottle(path: PathBuf) -> Bottle {
        Bottle {
            name: "GameHub 1".into(),
            path,
            source: SOURCE.into(),
        }
    }

    #[test]
    fn virtual_containers_dir_is_under_app_support() {
        let home = Path::new("/Users/someone");
        assert_eq!(
            virtual_containers_dir(home),
            PathBuf::from(
                "/Users/someone/Library/Application Support/com.gamemac.www/wine-engine/containers/virtual_containers"
            )
        );
    }

    #[test]
    fn collects_numeric_containers_with_drive_c_in_id_order() {
        let fx = fixture();
        make_container(&fx, "10");
        make_container(&fx, "2");
        // No drive_c: not a usable prefix yet.
        fs::create_dir_all(fx.containers.join("3")).unwrap();
        // Not a container ID.
        make_container(&fx, "tmp");
        make_container(&fx, "007");

        let mut bottles = Vec::new();
        collect_bottles(&fx.containers, &mut bottles);
        let names: Vec<&str> = bottles.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["GameHub 2", "GameHub 10"]);
        assert!(bottles.iter().all(|b| b.source == SOURCE));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_container_is_skipped() {
        let fx = fixture();
        let elsewhere = fx._tmp.path().join("elsewhere");
        fs::create_dir_all(elsewhere.join("drive_c")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, fx.containers.join("5")).unwrap();

        let mut bottles = Vec::new();
        collect_bottles(&fx.containers, &mut bottles);
        assert!(bottles.is_empty());
    }

    #[test]
    fn missing_dir_collects_nothing() {
        let mut bottles = Vec::new();
        collect_bottles(Path::new("/nonexistent/gamehub"), &mut bottles);
        assert!(bottles.is_empty());
    }

    // Shape of GameHub 0.8.613's store (trimmed).
    const STORE: &str = r#"{
      "schema_version": 3,
      "bindings": [
        {"id": 1, "game_name": "The Elder Scrolls V: Skyrim Special Edition",
         "platform": "steam", "platform_app_id": "489830",
         "game_path": "/Users/someone/GameHub",
         "virtual_container_id": "1", "base_container_id": "1"},
        {"id": 2, "platform": "epic", "platform_app_id": "Fortnite",
         "virtual_container_id": "2"},
        {"id": 3, "platform": "steam", "platform_app_id": "1145350",
         "virtual_container_id": 3}
      ]
    }"#;

    #[test]
    fn bound_steam_app_id_reads_binding_for_container() {
        let fx = fixture();
        write_store(&fx, STORE);
        let one = make_container(&fx, "1");
        let two = make_container(&fx, "2");
        let three = make_container(&fx, "3");
        let four = make_container(&fx, "4");

        assert_eq!(bound_steam_app_id(&bottle(one)).as_deref(), Some("489830"));
        // Epic binding: no Steam app ID.
        assert_eq!(bound_steam_app_id(&bottle(two)), None);
        // Numeric container ID in the JSON.
        assert_eq!(bound_steam_app_id(&bottle(three)).as_deref(), Some("1145350"));
        // Unbound container.
        assert_eq!(bound_steam_app_id(&bottle(four)), None);
    }

    #[test]
    fn bound_steam_app_id_rejects_other_sources_and_shapes() {
        let fx = fixture();
        write_store(&fx, STORE);
        let one = make_container(&fx, "1");

        let crossover = Bottle {
            source: "CrossOver".into(),
            ..bottle(one.clone())
        };
        assert_eq!(bound_steam_app_id(&crossover), None);

        // Right ID, wrong parent directories.
        let stray = fx._tmp.path().join("other").join("1");
        fs::create_dir_all(stray.join("drive_c")).unwrap();
        assert_eq!(bound_steam_app_id(&bottle(stray)), None);
    }

    #[test]
    fn bound_steam_app_id_rejects_unsafe_app_ids_and_bad_json() {
        let fx = fixture();
        let one = make_container(&fx, "1");
        write_store(
            &fx,
            r#"{"bindings":[{"platform":"steam","platform_app_id":"48;rm -rf","virtual_container_id":"1"}]}"#,
        );
        assert_eq!(bound_steam_app_id(&bottle(one.clone())), None);

        write_store(&fx, "{not json");
        assert_eq!(bound_steam_app_id(&bottle(one)), None);
    }

    #[test]
    fn steam_launch_url_only_for_numeric_ids() {
        assert_eq!(
            steam_launch_url("489830").as_deref(),
            Some("gamehub://launch/steam/489830")
        );
        assert_eq!(steam_launch_url(""), None);
        assert_eq!(steam_launch_url("489830/../x"), None);
    }
}
