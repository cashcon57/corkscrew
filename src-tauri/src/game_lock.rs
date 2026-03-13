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
            if is_game_running(lock.pid, game_id) {
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
        map.retain(|_, lock| is_game_running(lock.pid, &lock.game_id));
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
        map.retain(|_, lock| is_game_running(lock.pid, &lock.game_id));
        map.values().cloned().collect()
    }
}

/// Known game executable names per game_id.
/// When the SKSE loader (registered PID) becomes a zombie, we fall back to
/// checking if any of these executables are still running system-wide.
fn game_exe_names(game_id: &str) -> &'static [&'static str] {
    match game_id {
        "skyrimse" => &["SkyrimSE.exe", "skse64_loader.exe"],
        "skyrim" => &["TESV.exe", "skse_loader.exe"],
        "fallout4" => &["Fallout4.exe", "f4se_loader.exe"],
        "falloutnv" => &["FalloutNV.exe", "nvse_loader.exe"],
        "fallout3" => &["Fallout3.exe", "fose_loader.exe"],
        "oblivion" => &["Oblivion.exe", "obse_loader.exe"],
        "starfield" => &["Starfield.exe", "sfse_loader.exe"],
        _ => &[], // Unknown game — fall back to PID-only check
    }
}

/// Check if a game is still running.
///
/// Wine/CrossOver launches have a tricky process model:
///   wine --bottle Steam skse64_loader.exe
///     → skse64_loader.exe spawns SkyrimSE.exe
///     → skse64_loader.exe exits (becomes zombie)
///     → SkyrimSE.exe runs as a sibling under Wine, NOT as a child
///
/// So we can't rely on process tree traversal. Instead:
/// 1. If the registered PID is alive and not a zombie → game running
/// 2. If the registered PID is a zombie or dead → check if any known
///    game exe is still running via pgrep
#[cfg(unix)]
fn is_game_running(pid: u32, game_id: &str) -> bool {
    let pid_exists = unsafe { libc::kill(pid as i32, 0) == 0 };

    if pid_exists && !is_zombie(pid) {
        // PID is alive and healthy — game is running
        return true;
    }

    // PID is dead or zombie — check if the actual game exe is still running.
    // The SKSE loader exits quickly after spawning the game, so the registered
    // PID becoming a zombie is normal. The real game runs as a separate process.
    let exe_names = game_exe_names(game_id);
    if exe_names.is_empty() {
        // Unknown game — fall back to PID-only check
        return pid_exists;
    }

    for exe_name in exe_names {
        if is_exe_running(exe_name) {
            return true;
        }
    }

    // Neither the PID nor any known game exe is running
    false
}

/// Check if a process is a zombie (exited but not yet reaped by parent).
#[cfg(target_os = "macos")]
fn is_zombie(pid: u32) -> bool {
    match std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "state="])
        .output()
    {
        Ok(output) if output.status.success() => {
            let state = String::from_utf8_lossy(&output.stdout);
            // macOS state can be "Z", "ZN", "Z+" etc
            state.trim().starts_with('Z')
        }
        _ => false,
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn is_zombie(pid: u32) -> bool {
    if let Ok(stat) = std::fs::read_to_string(format!("/proc/{}/stat", pid)) {
        if let Some(rest) = stat.rsplit(')').next() {
            let state = rest.trim().chars().next().unwrap_or(' ');
            return state == 'Z';
        }
    }
    false
}

/// Check if any process matching the given executable name is running.
/// Uses pgrep with case-insensitive matching.
#[cfg(unix)]
fn is_exe_running(exe_name: &str) -> bool {
    match std::process::Command::new("pgrep")
        .args(["-if", exe_name])
        .output()
    {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

#[cfg(not(unix))]
fn is_game_running(pid: u32, _game_id: &str) -> bool {
    // On non-Unix, assume alive (conservative).
    true
}
