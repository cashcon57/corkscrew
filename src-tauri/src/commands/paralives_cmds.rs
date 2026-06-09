use std::path::PathBuf;
use std::sync::Arc;

use tauri::State;

use crate::database::ModDatabase;
use crate::paralives_bepinex::ParalivesBepInExStatus;

#[tauri::command]
pub fn get_paralives_bepinex_status(
    game_install_dir: String,
) -> Result<ParalivesBepInExStatus, String> {
    Ok(crate::paralives_bepinex::detect(&PathBuf::from(
        game_install_dir,
    )))
}

#[tauri::command]
pub fn install_paralives_bepinex_latest(
    game_install_dir: String,
    app_bundle_path: String,
    db: State<'_, Arc<ModDatabase>>,
) -> Result<ParalivesBepInExStatus, String> {
    crate::paralives_bepinex::install_latest(
        &PathBuf::from(game_install_dir),
        &PathBuf::from(app_bundle_path),
        &db,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn install_paralives_bepinex_from_archive(
    game_install_dir: String,
    app_bundle_path: String,
    archive_path: String,
    db: State<'_, Arc<ModDatabase>>,
) -> Result<ParalivesBepInExStatus, String> {
    crate::paralives_bepinex::install_from_archive(
        &PathBuf::from(game_install_dir),
        &PathBuf::from(app_bundle_path),
        &PathBuf::from(archive_path),
        &db,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn uninstall_paralives_bepinex(
    game_install_dir: String,
    app_bundle_path: String,
    db: State<'_, Arc<ModDatabase>>,
) -> Result<(), String> {
    crate::paralives_bepinex::uninstall(
        &PathBuf::from(game_install_dir),
        &PathBuf::from(app_bundle_path),
        &db,
    )
    .map_err(|e| e.to_string())
}
