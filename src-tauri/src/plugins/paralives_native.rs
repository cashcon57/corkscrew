//! Paralives native macOS game plugin.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::bottles::Bottle;
use crate::database::ModDatabase;
use crate::deployer::{DeployResult, DeployerError};
use crate::games::{DetectedGame, GamePlugin};
use crate::runtime::{GameRuntime, NativeContext};

const GAME_ID: &str = "paralives_native";
const DISPLAY_NAME: &str = "Paralives (Native)";
const NEXUS_SLUG: &str = "paralives";
const BUNDLE_IDS: &[&str] = &["com.Paralives.Paralives", "com.paralives.paralives"];
const EXECUTABLES: &[&str] = &["Paralives"];

pub struct ParalivesNativePlugin;

pub fn paralives_data_mods_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| {
        home.join("Library")
            .join("Application Support")
            .join("com.Paralives.Paralives")
            .join("Mods")
    })
}

fn detect_from_candidates(
    candidates: Vec<crate::native_scanner::NativeAppCandidate>,
) -> Vec<DetectedGame> {
    let Some(data_dir) = paralives_data_mods_dir() else {
        return Vec::new();
    };
    candidates
        .into_iter()
        .filter(|c| {
            BUNDLE_IDS
                .iter()
                .any(|id| c.info.bundle_identifier.eq_ignore_ascii_case(id))
                || c.info.bundle_executable.eq_ignore_ascii_case("Paralives")
        })
        .map(|c| {
            let game_root = c.bundle_path.join("Contents").join("MacOS");
            let exe = game_root.join(&c.info.bundle_executable);
            DetectedGame {
                game_id: GAME_ID.to_string(),
                display_name: DISPLAY_NAME.to_string(),
                nexus_slug: NEXUS_SLUG.to_string(),
                game_path: game_root.clone(),
                exe_path: Some(exe),
                data_dir: data_dir.clone(),
                runtime: GameRuntime::Native(NativeContext {
                    app_bundle_path: c.bundle_path,
                    game_data_root: game_root,
                    architecture: c.architecture,
                    sandboxed: c.sandboxed,
                    source: c.source,
                }),
                steam_app_id: None,
                is_custom: false,
            }
        })
        .collect()
}

fn copy_one(src: &Path, dst: &Path) -> Result<bool, DeployerError> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut fallback = false;
    let _ = std::fs::remove_file(dst);
    if std::fs::hard_link(src, dst).is_err() {
        std::fs::copy(src, dst)?;
        fallback = true;
    }
    Ok(fallback)
}

fn deploy_staged_file(
    game_root: &Path,
    data_mods_dir: &Path,
    mod_name: &str,
    staging_path: &Path,
    rel: &str,
    bepinex_status: &crate::paralives_bepinex::ParalivesBepInExStatus,
) -> Result<(usize, bool), DeployerError> {
    let src = staging_path.join(rel);
    if !src.is_file() {
        return Ok((0, false));
    }

    if crate::paralives_bepinex::is_bepinex_plugin_file(rel) {
        if !bepinex_status.installed || !bepinex_status.mac_supported {
            return Err(DeployerError::Other(
                "BepInEx required — see Settings → Native → Install BepInEx".into(),
            ));
        }
        let target_rel = crate::paralives_bepinex::plugin_target_relative(mod_name, rel)
            .map_err(|e| DeployerError::Other(e.to_string()))?;
        let dst = game_root.join(target_rel);
        let fallback = copy_one(&src, &dst)?;
        return Ok((1, fallback));
    }

    let target_rel = crate::paralives_bepinex::data_mod_target_relative(rel)
        .map_err(|e| DeployerError::Other(e.to_string()))?;
    let dst = data_mods_dir.join(target_rel);
    let fallback = copy_one(&src, &dst)?;
    Ok((1, fallback))
}

impl GamePlugin for ParalivesNativePlugin {
    fn game_id(&self) -> &str {
        GAME_ID
    }

    fn display_name(&self) -> &str {
        DISPLAY_NAME
    }

    fn nexus_slug(&self) -> &str {
        NEXUS_SLUG
    }

    fn executables(&self) -> &[&str] {
        EXECUTABLES
    }

    fn detect_native(&self) -> Vec<DetectedGame> {
        detect_from_candidates(crate::native_scanner::scan_all_native())
    }

    fn get_data_dir(&self, _game_path: &Path) -> PathBuf {
        paralives_data_mods_dir().unwrap_or_else(|| PathBuf::from("Mods"))
    }

    fn get_plugins_file(&self, _game_path: &Path, _bottle: &Bottle) -> Option<PathBuf> {
        None
    }

    fn use_legacy_data_dir(&self) -> bool {
        false
    }

    fn deploy_native(
        &self,
        detected: &DetectedGame,
        db: &Arc<ModDatabase>,
    ) -> std::result::Result<DeployResult, DeployerError> {
        let native = detected
            .runtime
            .native()
            .ok_or_else(|| DeployerError::Other("expected native runtime".into()))?;
        if native.sandboxed {
            return Err(DeployerError::Other(format!(
                "native modding refused for sandboxed app: {}",
                native.app_bundle_path.display()
            )));
        }

        std::fs::create_dir_all(&detected.data_dir)?;
        if let Err(e) = crate::rollback::create_native_snapshot(
            db,
            &detected.game_id,
            "paralives-deploy",
            "Paralives native deploy",
        ) {
            log::warn!("snapshot before Paralives deploy failed: {e}");
        }

        let bepinex_status = crate::paralives_bepinex::detect(&native.game_data_root);
        let mods = db
            .list_mods(&detected.game_id, "")
            .map_err(|e| DeployerError::Database(e.to_string()))?;
        let mut deployed_count = 0usize;
        let mut fallback_used = false;
        for m in mods.iter().filter(|m| m.enabled) {
            let Some(staging) = &m.staging_path else {
                continue;
            };
            let staging = PathBuf::from(staging);
            for rel in &m.installed_files {
                let (count, fallback) = deploy_staged_file(
                    &native.game_data_root,
                    &detected.data_dir,
                    &m.name,
                    &staging,
                    rel,
                    &bepinex_status,
                )?;
                deployed_count += count;
                fallback_used |= fallback;
            }
        }

        Ok(DeployResult {
            deployed_count,
            skipped_count: 0,
            fallback_used,
        })
    }
}

pub fn register() {
    crate::games::register_plugin(std::sync::Arc::new(ParalivesNativePlugin));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_scanner::NativeAppCandidate;
    use crate::plist::InfoPlist;
    use crate::runtime::{Architecture, NativeSource};

    fn candidate(id: &str) -> NativeAppCandidate {
        NativeAppCandidate {
            bundle_path: PathBuf::from("/Applications/Paralives.app"),
            info: InfoPlist {
                bundle_identifier: id.into(),
                bundle_executable: "Paralives".into(),
                short_version: Some("1.0".into()),
                category: Some("public.app-category.games".into()),
            },
            architecture: Architecture::AppleSilicon,
            source: NativeSource::Steam,
            sandboxed: false,
        }
    }

    #[test]
    fn detects_paralives_candidate() {
        let games = detect_from_candidates(vec![candidate("com.Paralives.Paralives")]);
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].game_id, GAME_ID);
        assert!(games[0].runtime.is_native());
    }

    #[test]
    fn deploy_routes_bepinex_plugin_to_game_root() {
        let tmp = tempfile::tempdir().unwrap();
        let game_root = tmp.path().join("Game");
        let data = tmp.path().join("DataMods");
        let staging = tmp.path().join("staging");
        std::fs::create_dir_all(staging.join("BepInEx/plugins")).unwrap();
        std::fs::create_dir_all(game_root.join("BepInEx/core")).unwrap();
        std::fs::write(staging.join("BepInEx/plugins/Foo.dll"), b"dll").unwrap();
        std::fs::write(game_root.join("BepInEx/core/BepInEx.Core.dll"), b"6.0.0").unwrap();
        std::fs::write(game_root.join("doorstop_config.ini"), b"[UnityDoorstop]").unwrap();
        std::fs::write(game_root.join("libdoorstop.dylib"), b"dylib").unwrap();
        let status = crate::paralives_bepinex::detect(&game_root);
        deploy_staged_file(
            &game_root,
            &data,
            "Cool Mod",
            &staging,
            "BepInEx/plugins/Foo.dll",
            &status,
        )
        .unwrap();
        assert!(game_root.join("BepInEx/plugins/Cool_Mod/Foo.dll").exists());
        assert!(!data.join("BepInEx/plugins/Foo.dll").exists());
    }

    #[test]
    fn deploy_refuses_plugin_without_bepinex() {
        let tmp = tempfile::tempdir().unwrap();
        let staging = tmp.path().join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("Foo.dll"), b"dll").unwrap();
        let status = crate::paralives_bepinex::detect(tmp.path());
        let err = deploy_staged_file(
            tmp.path(),
            &tmp.path().join("Mods"),
            "Cool",
            &staging,
            "Foo.dll",
            &status,
        )
        .unwrap_err();
        assert!(err.to_string().contains("BepInEx required"));
    }
}
