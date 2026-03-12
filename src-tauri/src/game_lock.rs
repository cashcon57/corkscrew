//! Game Lock Manager — tracks running game processes per (game_id, bottle_name).
//!
//! When a game is launched, we register its PID here.  The frontend polls
//! `get_game_lock_status` to show the MO2-style "game is running" banner and
//! block mod changes.  If the process dies, the lock auto-clears on the next
//! poll.  Users can also force-unlock via the UI.

use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;

/// Key for the running-games map.
type GameKey = (String, String); // (game_id, bottle_name)

/// Information about a running game process.
#[derive(Clone, Debug, Serialize)]
pub struct GameLock {
    pub game_id: String,
    pub bottle_name: String,
    pub pid: u32,
    pub started_at: String, // RFC 3339
}

/// Thread-safe registry of running game processes.
pub struct GameLockManager {
    locks: Mutex<HashMap<GameKey, GameLock>>,
}

impl GameLockManager {
    pub fn new() -> Self {
        Self {
            locks: Mutex::new(HashMap::new()),
        }
    }

    /// Register a newly launched game.  Overwrites any previous lock for the
    /// same (game_id, bottle_name) — there can only be one instance per game.
    pub fn register(&self, game_id: &str, bottle_name: &str, pid: u32) {
        let lock = GameLock {
            game_id: game_id.to_string(),
            bottle_name: bottle_name.to_string(),
            pid,
            started_at: chrono::Utc::now().to_rfc3339(),
        };
        let key = (game_id.to_string(), bottle_name.to_string());
        self.locks.lock().unwrap().insert(key, lock);
        log::info!(
            "game_lock: registered pid={} for {}/{}",
            pid,
            game_id,
            bottle_name
        );
    }

    /// Check if the game is still running.  If the process has exited, the
    /// lock is automatically cleared and `None` is returned.
    pub fn get(&self, game_id: &str, bottle_name: &str) -> Option<GameLock> {
        let key = (game_id.to_string(), bottle_name.to_string());
        let mut map = self.locks.lock().unwrap();

        if let Some(lock) = map.get(&key) {
            if is_process_alive(lock.pid) {
                return Some(lock.clone());
            }
            // Process has exited — auto-clear
            log::info!(
                "game_lock: pid={} for {}/{} exited — auto-clearing",
                lock.pid,
                game_id,
                bottle_name
            );
            map.remove(&key);
        }
        None
    }

    /// Check if ANY game is running (for global lock queries).
    pub fn any_running(&self) -> Option<GameLock> {
        let mut map = self.locks.lock().unwrap();
        // Prune dead processes first
        map.retain(|_, lock| is_process_alive(lock.pid));
        map.values().next().cloned()
    }

    /// Force-unlock a game (user override).
    pub fn force_unlock(&self, game_id: &str, bottle_name: &str) -> bool {
        let key = (game_id.to_string(), bottle_name.to_string());
        let removed = self.locks.lock().unwrap().remove(&key).is_some();
        if removed {
            log::warn!(
                "game_lock: force-unlocked {}/{} by user",
                game_id,
                bottle_name
            );
        }
        removed
    }

    /// Get all active locks (pruning dead processes).
    pub fn all_locks(&self) -> Vec<GameLock> {
        let mut map = self.locks.lock().unwrap();
        map.retain(|_, lock| is_process_alive(lock.pid));
        map.values().cloned().collect()
    }
}

/// Check if a process with the given PID is still alive.
/// Uses kill(pid, 0) on Unix — sends no signal, just checks existence.
#[cfg(unix)]
fn is_process_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[cfg(not(unix))]
fn is_process_alive(pid: u32) -> bool {
    // On non-Unix, assume alive (conservative).  Wine processes run under
    // macOS/Linux so this path should never be hit in practice.
    true
}
