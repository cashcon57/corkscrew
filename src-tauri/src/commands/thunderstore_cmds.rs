//! Thunderstore catalog commands.

use crate::thunderstore::{self, Community, Package};

#[tauri::command]
pub async fn thunderstore_list_communities() -> Result<Vec<Community>, String> {
    thunderstore::list_communities().await
}

#[tauri::command]
pub async fn thunderstore_list_packages(community: String) -> Result<Vec<Package>, String> {
    thunderstore::list_packages(&community).await
}
