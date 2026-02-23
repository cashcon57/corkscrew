use serde::Serialize;

pub const INSTALL_PROGRESS_EVENT: &str = "install-progress";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum InstallProgress {
    ModStarted {
        mod_index: usize,
        total_mods: usize,
        mod_name: String,
    },
    StepChanged {
        mod_index: usize,
        step: String,
        detail: Option<String>,
    },
    DownloadProgress {
        mod_index: usize,
        downloaded: u64,
        total: u64,
    },
    ModCompleted {
        mod_index: usize,
        mod_name: String,
        mod_id: i64,
    },
    ModFailed {
        mod_index: usize,
        mod_name: String,
        error: String,
    },
    CollectionCompleted {
        installed: usize,
        skipped: usize,
        failed: usize,
    },
    UserActionRequired {
        mod_index: usize,
        mod_name: String,
        action: String,
        url: Option<String>,
        instructions: Option<String>,
    },
    DownloadPhaseStarted {
        total_downloads: usize,
        max_concurrent: usize,
    },
    DownloadQueued {
        mod_index: usize,
        mod_name: String,
    },
    DownloadModStarted {
        mod_index: usize,
        mod_name: String,
    },
    DownloadModCompleted {
        mod_index: usize,
        mod_name: String,
        cached: bool,
    },
    DownloadModFailed {
        mod_index: usize,
        mod_name: String,
        error: String,
    },
    AllDownloadsCompleted {
        downloaded: usize,
        cached: usize,
        failed: usize,
        skipped: usize,
    },
    InstallPhaseStarted {
        total_mods: usize,
    },
    StagingPhaseStarted {
        total_mods: usize,
        max_concurrent: usize,
    },
    StagingModStarted {
        mod_index: usize,
        mod_name: String,
    },
    StagingModCompleted {
        mod_index: usize,
        mod_name: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_mod_started() {
        let event = InstallProgress::ModStarted {
            mod_index: 0,
            total_mods: 5,
            mod_name: "Test Mod".to_string(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"kind\":\"modStarted\""));
        assert!(json.contains("\"mod_name\":\"Test Mod\""));
        assert!(json.contains("\"total_mods\":5"));
    }

    #[test]
    fn test_serialize_step_changed() {
        let event = InstallProgress::StepChanged {
            mod_index: 1,
            step: "extracting".to_string(),
            detail: Some("archive.7z".to_string()),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"kind\":\"stepChanged\""));
        assert!(json.contains("\"step\":\"extracting\""));
    }

    #[test]
    fn test_serialize_collection_completed() {
        let event = InstallProgress::CollectionCompleted {
            installed: 10,
            skipped: 2,
            failed: 1,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"kind\":\"collectionCompleted\""));
        assert!(json.contains("\"installed\":10"));
    }
}
