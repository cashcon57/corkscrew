//! Wine/CrossOver verification manifest commands.

use crate::verified_lists::{self, Manifest, VerifiedEntry};

#[tauri::command]
pub fn get_collection_verification(game_domain: String, slug: String) -> VerifiedEntry {
    verified_lists::maybe_refresh_in_background();
    verified_lists::collection_status(&game_domain, &slug)
}

#[tauri::command]
pub fn get_wabbajack_verification(modlist_name: String) -> VerifiedEntry {
    verified_lists::maybe_refresh_in_background();
    verified_lists::wabbajack_status(&modlist_name)
}

#[tauri::command]
pub fn get_verification_manifest() -> Manifest {
    verified_lists::maybe_refresh_in_background();
    verified_lists::full_manifest()
}

#[tauri::command]
pub async fn refresh_verification_manifest() -> Result<(), String> {
    verified_lists::refresh_from_remote().await
}

#[tauri::command]
pub fn get_verification_cache_age_secs() -> Option<u64> {
    verified_lists::cache_age_secs()
}
