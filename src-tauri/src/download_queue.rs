//! Download queue with retry support.
//!
//! Tracks download attempts across collection installs and manual downloads,
//! allowing users to view status and retry failed downloads.

use std::collections::VecDeque;
use std::sync::Mutex;

use serde::Serialize;

/// Event name emitted when the download queue changes.
pub const DOWNLOAD_QUEUE_EVENT: &str = "download-queue-update";

/// Status of a queued download.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DownloadStatus {
    Pending,
    Downloading,
    Completed,
    Failed,
    Cancelled,
}

/// A single download item in the queue.
#[derive(Clone, Debug, Serialize)]
pub struct QueueItem {
    pub id: u64,
    pub mod_name: String,
    pub file_name: String,
    pub status: DownloadStatus,
    pub error: Option<String>,
    pub attempt: u32,
    pub max_attempts: u32,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    /// Nexus mod ID (if from Nexus)
    pub nexus_mod_id: Option<i64>,
    /// Nexus file ID (if from Nexus)
    pub nexus_file_id: Option<i64>,
    /// Direct download URL (if direct source)
    pub url: Option<String>,
    /// Game slug for Nexus API
    pub game_slug: Option<String>,
}

/// Thread-safe download queue.
pub struct DownloadQueue {
    inner: Mutex<QueueInner>,
}

struct QueueInner {
    items: VecDeque<QueueItem>,
    next_id: u64,
}

impl Default for DownloadQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl DownloadQueue {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(QueueInner {
                items: VecDeque::new(),
                next_id: 1,
            }),
        }
    }

    /// Add a new item to the queue and return its ID.
    pub fn enqueue(
        &self,
        mod_name: &str,
        file_name: &str,
        nexus_mod_id: Option<i64>,
        nexus_file_id: Option<i64>,
        url: Option<&str>,
        game_slug: Option<&str>,
    ) -> u64 {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let id = inner.next_id;
        inner.next_id += 1;

        inner.items.push_back(QueueItem {
            id,
            mod_name: mod_name.to_string(),
            file_name: file_name.to_string(),
            status: DownloadStatus::Pending,
            error: None,
            attempt: 0,
            max_attempts: 3,
            downloaded_bytes: 0,
            total_bytes: 0,
            nexus_mod_id,
            nexus_file_id,
            url: url.map(|s| s.to_string()),
            game_slug: game_slug.map(|s| s.to_string()),
        });

        id
    }

    /// Mark a download as started.
    pub fn set_downloading(&self, id: u64) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(item) = inner.items.iter_mut().find(|i| i.id == id) {
            item.status = DownloadStatus::Downloading;
            item.attempt += 1;
        }
    }

    /// Update download progress.
    pub fn set_progress(&self, id: u64, downloaded: u64, total: u64) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(item) = inner.items.iter_mut().find(|i| i.id == id) {
            item.downloaded_bytes = downloaded;
            item.total_bytes = total;
        }
    }

    /// Mark a download as completed.
    pub fn set_completed(&self, id: u64) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(item) = inner.items.iter_mut().find(|i| i.id == id) {
            item.status = DownloadStatus::Completed;
            item.error = None;
        }
    }

    /// Mark a download as failed.
    pub fn set_failed(&self, id: u64, error: &str) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(item) = inner.items.iter_mut().find(|i| i.id == id) {
            item.status = DownloadStatus::Failed;
            item.error = Some(error.to_string());
        }
    }

    /// Mark a download as cancelled.
    pub fn set_cancelled(&self, id: u64) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(item) = inner.items.iter_mut().find(|i| i.id == id) {
            item.status = DownloadStatus::Cancelled;
        }
    }

    /// Reset a failed item back to pending for retry.
    pub fn mark_for_retry(&self, id: u64) -> bool {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(item) = inner.items.iter_mut().find(|i| i.id == id) {
            if item.status == DownloadStatus::Failed && item.attempt < item.max_attempts {
                item.status = DownloadStatus::Pending;
                item.error = None;
                item.downloaded_bytes = 0;
                return true;
            }
        }
        false
    }

    /// Get all items in the queue.
    pub fn get_all(&self) -> Vec<QueueItem> {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.items.iter().cloned().collect()
    }

    /// Get a specific item by ID.
    pub fn get_item(&self, id: u64) -> Option<QueueItem> {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.items.iter().find(|i| i.id == id).cloned()
    }

    /// Remove completed and cancelled items from the queue.
    pub fn clear_finished(&self) -> usize {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let before = inner.items.len();
        inner.items.retain(|item| {
            !matches!(
                item.status,
                DownloadStatus::Completed | DownloadStatus::Cancelled
            )
        });
        before - inner.items.len()
    }

    /// Get counts by status.
    pub fn status_counts(&self) -> QueueCounts {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut counts = QueueCounts::default();
        for item in &inner.items {
            match item.status {
                DownloadStatus::Pending => counts.pending += 1,
                DownloadStatus::Downloading => counts.downloading += 1,
                DownloadStatus::Completed => counts.completed += 1,
                DownloadStatus::Failed => counts.failed += 1,
                DownloadStatus::Cancelled => counts.cancelled += 1,
            }
        }
        counts
    }
}

/// Summary counts for each download status.
#[derive(Clone, Debug, Default, Serialize)]
pub struct QueueCounts {
    pub pending: usize,
    pub downloading: usize,
    pub completed: usize,
    pub failed: usize,
    pub cancelled: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enqueue_and_lifecycle() {
        let q = DownloadQueue::new();
        let id = q.enqueue("Test Mod", "test.zip", Some(123), Some(456), None, None);
        assert_eq!(id, 1);

        let item = q.get_item(id).unwrap();
        assert_eq!(item.status, DownloadStatus::Pending);
        assert_eq!(item.attempt, 0);

        q.set_downloading(id);
        let item = q.get_item(id).unwrap();
        assert_eq!(item.status, DownloadStatus::Downloading);
        assert_eq!(item.attempt, 1);

        q.set_progress(id, 500, 1000);
        let item = q.get_item(id).unwrap();
        assert_eq!(item.downloaded_bytes, 500);
        assert_eq!(item.total_bytes, 1000);

        q.set_completed(id);
        let item = q.get_item(id).unwrap();
        assert_eq!(item.status, DownloadStatus::Completed);
    }

    #[test]
    fn retry_failed_download() {
        let q = DownloadQueue::new();
        let id = q.enqueue(
            "Test Mod",
            "test.zip",
            None,
            None,
            Some("http://example.com/test.zip"),
            None,
        );

        q.set_downloading(id);
        q.set_failed(id, "Connection timeout");

        let item = q.get_item(id).unwrap();
        assert_eq!(item.status, DownloadStatus::Failed);
        assert_eq!(item.error, Some("Connection timeout".to_string()));

        assert!(q.mark_for_retry(id));
        let item = q.get_item(id).unwrap();
        assert_eq!(item.status, DownloadStatus::Pending);
        assert!(item.error.is_none());
    }

    #[test]
    fn clear_finished_items() {
        let q = DownloadQueue::new();
        let id1 = q.enqueue("Mod 1", "a.zip", None, None, None, None);
        let id2 = q.enqueue("Mod 2", "b.zip", None, None, None, None);
        let _id3 = q.enqueue("Mod 3", "c.zip", None, None, None, None);

        q.set_downloading(id1);
        q.set_completed(id1);
        q.set_downloading(id2);
        q.set_failed(id2, "error");

        let removed = q.clear_finished();
        assert_eq!(removed, 1); // Only completed is cleared

        let items = q.get_all();
        assert_eq!(items.len(), 2); // Failed + Pending remain
    }

    #[test]
    fn status_counts() {
        let q = DownloadQueue::new();
        let id1 = q.enqueue("A", "a.zip", None, None, None, None);
        let id2 = q.enqueue("B", "b.zip", None, None, None, None);
        q.enqueue("C", "c.zip", None, None, None, None);

        q.set_downloading(id1);
        q.set_completed(id1);
        q.set_downloading(id2);
        q.set_failed(id2, "err");

        let counts = q.status_counts();
        assert_eq!(counts.completed, 1);
        assert_eq!(counts.failed, 1);
        assert_eq!(counts.pending, 1);
    }

    #[test]
    fn max_retries_respected() {
        let q = DownloadQueue::new();
        let id = q.enqueue("Mod", "mod.zip", None, None, None, None);

        // Exhaust all 3 attempts
        for _ in 0..3 {
            q.set_downloading(id);
            q.set_failed(id, "error");
            if q.get_item(id).unwrap().attempt < 3 {
                assert!(q.mark_for_retry(id));
            }
        }

        // 4th retry should be denied
        assert!(!q.mark_for_retry(id));
    }
}
