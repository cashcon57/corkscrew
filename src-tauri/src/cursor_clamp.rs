//! macOS cursor fix for fullscreen Wine/CrossOver games.
//!
//! Problem: When running games through Wine/CrossOver in fullscreen, the macOS
//! Dock auto-show trigger zone at the bottom of the screen causes the system
//! cursor to become visible — even though the game hides it. Hot Corners also
//! trigger the system cursor at screen edges.
//!
//! Solution (layered):
//! 1. **Dock suppression**: Temporarily set `autohide-delay` to a huge value
//!    so the Dock trigger zone never activates. Restored on game exit.
//! 2. **Hot Corner suppression**: Temporarily disable all Hot Corners.
//! 3. **CGDisplayHideCursor**: Force-hide the system cursor at the CG level.
//! 4. **CGEventTap Y-clamp**: Belt-and-suspenders — clamp cursor position
//!    away from the bottom edge as a secondary defense.
//!
//! Game exit detection:
//! - Wine process PIDs are unreliable (launcher dies instantly, actual game
//!   runs under wineserver/winewrapper which may be stale or shared).
//! - Instead, we check for the actual game process by name (`pgrep -if`).
//!   When `SkyrimSE.exe` (or whatever game_exe was passed) disappears from
//!   the process list, the game has truly exited → deactivate.
//! - Process watcher: polls every 3s after a 15s grace period.
//! - Window focus handler: instant deactivation when Corkscrew gains focus
//!   and the game process is gone.
//! - Tauri RunEvent::Exit handler calls deactivate() on app close.
//! - Recovery: On next app start, detects sentinel autohide-delay=86400.

#[cfg(not(target_os = "macos"))]
pub fn activate(_screen_height: u32, _game_pid: u32, _game_exe: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn deactivate() {}

#[cfg(not(target_os = "macos"))]
pub fn is_active() -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub fn has_permission() -> bool {
    true
}

#[cfg(not(target_os = "macos"))]
pub fn request_permission() -> bool {
    true
}

#[cfg(not(target_os = "macos"))]
pub fn recover_dock_if_needed() {}

#[cfg(not(target_os = "macos"))]
pub fn check_and_maybe_deactivate() {}

#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::c_void;
    use std::ptr;
    use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};
    use std::sync::Mutex;
    use std::thread::{self, JoinHandle};

    use log::{debug, info, warn};

    /// Pixels from the bottom screen edge to block (secondary defense).
    const BOTTOM_MARGIN: f64 = 20.0;

    /// How often (seconds) to check if the game process is still running.
    const POLL_INTERVAL: f64 = 3.0;

    /// Grace period after activation before checking (seconds).
    /// Wine needs time to spawn the actual game executable.
    const POLL_GRACE: f64 = 15.0;

    // -----------------------------------------------------------------------
    // Static state
    // -----------------------------------------------------------------------

    static ACTIVE: AtomicBool = AtomicBool::new(false);
    static SCREEN_HEIGHT: AtomicU32 = AtomicU32::new(0);
    /// The game executable name to check with pgrep (e.g. "SkyrimSE").
    static GAME_EXE: Mutex<String> = Mutex::new(String::new());
    static CURSOR_HIDDEN: AtomicBool = AtomicBool::new(false);
    static CLAMP_COUNT: AtomicU32 = AtomicU32::new(0);
    static EVENT_COUNT: AtomicU32 = AtomicU32::new(0);
    static EVENT_TAP_REF: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
    static RUN_LOOP_REF: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
    static TAP_THREAD: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);
    static WATCHER_THREAD: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);
    static DOCK_SUPPRESSED: AtomicBool = AtomicBool::new(false);
    static DOCK_ORIGINAL_DELAY: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);
    /// Whether we suppressed Hot Corners and need to restore them.
    static CORNERS_SUPPRESSED: AtomicBool = AtomicBool::new(false);

    // -----------------------------------------------------------------------
    // CoreGraphics / CoreFoundation FFI
    // -----------------------------------------------------------------------

    #[repr(C)]
    #[derive(Debug, Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    const K_CG_SESSION_EVENT_TAP: u32 = 1;
    const K_CG_HEAD_INSERT_EVENT_TAP: u32 = 0;
    const K_CG_EVENT_TAP_OPTION_DEFAULT: u32 = 0;

    const K_CG_EVENT_MOUSE_MOVED: u32 = 5;
    const K_CG_EVENT_LEFT_MOUSE_DRAGGED: u32 = 6;
    const K_CG_EVENT_RIGHT_MOUSE_DRAGGED: u32 = 7;
    const K_CG_EVENT_OTHER_MOUSE_DRAGGED: u32 = 27;
    const K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFFFFFE;

    type CGEventMask = u64;
    type CGEventTapCallBack = unsafe extern "C" fn(
        proxy: *const c_void,
        event_type: u32,
        event: *mut c_void,
        user_info: *mut c_void,
    ) -> *mut c_void;

    type CFIndex = isize;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventTapCreate(
            tap: u32,
            place: u32,
            options: u32,
            events_of_interest: CGEventMask,
            callback: CGEventTapCallBack,
            user_info: *mut c_void,
        ) -> *mut c_void;

        fn CGEventTapEnable(tap: *mut c_void, enable: bool);
        fn CGEventGetLocation(event: *mut c_void) -> CGPoint;
        fn CGEventSetLocation(event: *mut c_void, point: CGPoint);
        fn CGMainDisplayID() -> u32;
        fn CGDisplayBounds(display: u32) -> CGRect;
        fn CGDisplayHideCursor(display: u32) -> u32;
        fn CGDisplayShowCursor(display: u32) -> u32;
    }

    #[repr(C)]
    #[derive(Debug, Clone, Copy)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }

    #[repr(C)]
    #[derive(Debug, Clone, Copy)]
    struct CGSize {
        width: f64,
        height: f64,
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFMachPortCreateRunLoopSource(
            allocator: *const c_void,
            port: *mut c_void,
            order: CFIndex,
        ) -> *mut c_void;

        fn CFRunLoopAddSource(rl: *mut c_void, source: *mut c_void, mode: *const c_void);
        fn CFRunLoopGetCurrent() -> *mut c_void;
        fn CFRunLoopRun();
        fn CFRunLoopStop(rl: *mut c_void);
        fn CFRelease(cf: *const c_void);

        fn CFDictionaryCreate(
            allocator: *const c_void,
            keys: *const *const c_void,
            values: *const *const c_void,
            num_values: CFIndex,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> *mut c_void;

        static kCFRunLoopCommonModes: *const c_void;
        static kCFBooleanTrue: *const c_void;
        static kCFTypeDictionaryKeyCallBacks: c_void;
        static kCFTypeDictionaryValueCallBacks: c_void;
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
        static kAXTrustedCheckOptionPrompt: *const c_void;
    }

    // -----------------------------------------------------------------------
    // Dock suppression
    // -----------------------------------------------------------------------

    static DOCK_WAS_AUTOHIDE: AtomicBool = AtomicBool::new(false);

    fn is_dock_autohide() -> bool {
        std::process::Command::new("defaults")
            .args(["read", "com.apple.dock", "autohide"])
            .output()
            .map(|o| {
                o.status.success()
                    && String::from_utf8_lossy(&o.stdout).trim() == "1"
            })
            .unwrap_or(false)
    }

    fn read_dock_autohide_delay() -> f64 {
        std::process::Command::new("defaults")
            .args(["read", "com.apple.dock", "autohide-delay"])
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    String::from_utf8_lossy(&o.stdout).trim().parse::<f64>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0.0)
    }

    fn suppress_dock() {
        let was_autohide = is_dock_autohide();
        let original_delay = read_dock_autohide_delay();

        DOCK_WAS_AUTOHIDE.store(was_autohide, Ordering::SeqCst);
        DOCK_ORIGINAL_DELAY.store(original_delay.to_bits(), Ordering::SeqCst);

        info!(
            "cursor_clamp: suppressing Dock (was_autohide={}, original_delay={:.1}s)",
            was_autohide, original_delay
        );

        if !was_autohide {
            let _ = std::process::Command::new("defaults")
                .args(["write", "com.apple.dock", "autohide", "-bool", "true"])
                .status();
        }

        let ok = std::process::Command::new("defaults")
            .args(["write", "com.apple.dock", "autohide-delay", "-float", "86400"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if ok {
            DOCK_SUPPRESSED.store(true, Ordering::SeqCst);
            let _ = std::process::Command::new("killall")
                .arg("Dock")
                .status();
            info!("cursor_clamp: Dock suppressed (autohide=true, delay=86400)");
        } else {
            warn!("cursor_clamp: failed to suppress Dock");
        }
    }

    fn restore_dock() {
        if !DOCK_SUPPRESSED.swap(false, Ordering::SeqCst) {
            return;
        }

        let was_autohide = DOCK_WAS_AUTOHIDE.load(Ordering::SeqCst);
        let original_delay = f64::from_bits(DOCK_ORIGINAL_DELAY.load(Ordering::SeqCst));

        info!(
            "cursor_clamp: restoring Dock (autohide → {}, delay → {:.1}s)",
            was_autohide, original_delay
        );

        if !was_autohide {
            let _ = std::process::Command::new("defaults")
                .args(["write", "com.apple.dock", "autohide", "-bool", "false"])
                .status();
        }

        if original_delay == 0.0 {
            let _ = std::process::Command::new("defaults")
                .args(["delete", "com.apple.dock", "autohide-delay"])
                .status();
        } else {
            let _ = std::process::Command::new("defaults")
                .args([
                    "write", "com.apple.dock", "autohide-delay",
                    "-float", &format!("{}", original_delay),
                ])
                .status();
        }

        let _ = std::process::Command::new("killall")
            .arg("Dock")
            .status();
        info!("cursor_clamp: Dock restored");
    }

    // -----------------------------------------------------------------------
    // Hot Corner suppression
    //
    // macOS Hot Corners trigger system cursor visibility at screen edges.
    // We save the current values and set them all to 0 (disabled).
    // The four corners are: tl (top-left), tr, bl, br.
    // -----------------------------------------------------------------------

    /// Original Hot Corner values, stored as "tl,tr,bl,br" or empty if none.
    /// Also persisted to CORNER_BACKUP_FILE for crash recovery.
    static CORNER_ORIGINALS: Mutex<String> = Mutex::new(String::new());

    /// File path for persisting hot corner originals across crashes.
    fn corner_backup_path() -> std::path::PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
            .join(".corkscrew-hot-corners-backup")
    }

    const CORNER_KEYS: [&str; 4] = [
        "wvous-tl-corner",
        "wvous-tr-corner",
        "wvous-bl-corner",
        "wvous-br-corner",
    ];

    fn read_corner(key: &str) -> i32 {
        std::process::Command::new("defaults")
            .args(["read", "com.apple.dock", key])
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    String::from_utf8_lossy(&o.stdout).trim().parse::<i32>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0)
    }

    fn suppress_hot_corners() {
        let values: Vec<i32> = CORNER_KEYS.iter().map(|k| read_corner(k)).collect();
        let any_set = values.iter().any(|&v| v != 0);

        if !any_set {
            info!("cursor_clamp: no Hot Corners to suppress");
            return;
        }

        // Save originals as "v1,v2,v3,v4"
        let originals = values
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(",");

        if let Ok(mut s) = CORNER_ORIGINALS.lock() {
            *s = originals.clone();
        }

        info!("cursor_clamp: suppressing Hot Corners (originals: {})", originals);

        for key in &CORNER_KEYS {
            let _ = std::process::Command::new("defaults")
                .args(["write", "com.apple.dock", key, "-int", "0"])
                .status();
        }

        CORNERS_SUPPRESSED.store(true, Ordering::SeqCst);

        // Persist originals to disk for crash recovery
        let _ = std::fs::write(corner_backup_path(), &originals);

        // Changes take effect when suppress_dock() does `killall Dock` next.
    }

    fn restore_hot_corners() {
        if !CORNERS_SUPPRESSED.swap(false, Ordering::SeqCst) {
            return;
        }

        let originals = if let Ok(s) = CORNER_ORIGINALS.lock() {
            s.clone()
        } else {
            return;
        };

        let values: Vec<i32> = originals
            .split(',')
            .filter_map(|s| s.parse().ok())
            .collect();

        if values.len() != 4 {
            warn!("cursor_clamp: invalid corner originals: {}", originals);
            return;
        }

        info!("cursor_clamp: restoring Hot Corners ({})", originals);

        for (key, val) in CORNER_KEYS.iter().zip(values.iter()) {
            if *val == 0 {
                let _ = std::process::Command::new("defaults")
                    .args(["delete", "com.apple.dock", key])
                    .status();
            } else {
                let _ = std::process::Command::new("defaults")
                    .args(["write", "com.apple.dock", key, "-int", &val.to_string()])
                    .status();
            }
        }
        // Remove crash recovery file
        let _ = std::fs::remove_file(corner_backup_path());

        // Dock restart (done by restore_dock) applies corner changes too
    }

    // -----------------------------------------------------------------------
    // Game process detection
    //
    // Wine process PIDs are unreliable: the launcher PID dies instantly,
    // and the actual game runs under wineserver/winewrapper which may be
    // stale or shared across sessions. Instead we check for the actual
    // game executable by name using `pgrep -if <name>`.
    // -----------------------------------------------------------------------

    /// Check if the game process is still running by searching for the
    /// executable name in the process list (case-insensitive).
    fn is_game_running() -> bool {
        let exe = match GAME_EXE.lock() {
            Ok(s) => s.clone(),
            Err(e) => e.into_inner().clone(),
        };

        if exe.is_empty() {
            return false;
        }

        std::process::Command::new("pgrep")
            .args(["-if", &exe])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    // -----------------------------------------------------------------------
    // Game exit watcher
    //
    // Polls `pgrep -if <game_exe>` every POLL_INTERVAL seconds after a
    // grace period. When the game process disappears for two consecutive
    // checks, deactivate.
    // -----------------------------------------------------------------------

    fn spawn_game_watcher() {
        let handle = thread::Builder::new()
            .name("cursor-game-watch".into())
            .spawn(|| {
                let start = std::time::Instant::now();
                let exe = match GAME_EXE.lock() {
                    Ok(s) => s.clone(),
                    Err(e) => e.into_inner().clone(),
                };

                info!(
                    "game_watcher: monitoring '{}' (grace={}s, interval={}s)",
                    exe, POLL_GRACE, POLL_INTERVAL
                );

                let mut gone_count = 0u32;

                loop {
                    thread::sleep(std::time::Duration::from_secs_f64(POLL_INTERVAL));

                    if !ACTIVE.load(Ordering::SeqCst) {
                        info!("game_watcher: ACTIVE=false, exiting");
                        return;
                    }

                    // Grace period — let Wine spawn the game executable
                    if start.elapsed().as_secs_f64() < POLL_GRACE {
                        debug!(
                            "game_watcher: grace period ({:.0}s / {}s)",
                            start.elapsed().as_secs_f64(), POLL_GRACE
                        );
                        continue;
                    }

                    if is_game_running() {
                        gone_count = 0;
                        continue;
                    }

                    gone_count += 1;

                    if gone_count == 1 {
                        // First miss — could be transient. Check once more.
                        info!(
                            "game_watcher: '{}' not found (check 1/2, {:.0}s elapsed)",
                            exe, start.elapsed().as_secs_f64()
                        );
                        continue;
                    }

                    // Two consecutive misses — game has exited.
                    info!(
                        "game_watcher: '{}' gone for 2 checks after {:.0}s — deactivating",
                        exe, start.elapsed().as_secs_f64()
                    );
                    deactivate();
                    return;
                }
            })
            .ok();

        if let Some(h) = handle {
            if let Ok(mut t) = WATCHER_THREAD.lock() {
                *t = Some(h);
            }
        }
    }

    /// Called on app startup to recover from a previous crash that left the
    /// Dock/Hot Corners suppressed. Checks for our sentinel values.
    pub fn recover_dock_if_needed() {
        let delay = read_dock_autohide_delay();
        let backup = corner_backup_path();
        let corners_backup = std::fs::read_to_string(&backup).ok();

        if delay != 86400.0 && corners_backup.is_none() {
            return;
        }

        warn!("cursor_clamp: detected leftover suppression from previous crash");

        // Restore Hot Corners from backup file (if present)
        if let Some(originals) = corners_backup {
            let values: Vec<i32> = originals
                .trim()
                .split(',')
                .filter_map(|s| s.parse().ok())
                .collect();

            if values.len() == 4 {
                info!("cursor_clamp: recovering Hot Corners from backup ({})", originals.trim());
                for (key, val) in CORNER_KEYS.iter().zip(values.iter()) {
                    if *val == 0 {
                        let _ = std::process::Command::new("defaults")
                            .args(["delete", "com.apple.dock", key])
                            .status();
                    } else {
                        let _ = std::process::Command::new("defaults")
                            .args(["write", "com.apple.dock", key, "-int", &val.to_string()])
                            .status();
                    }
                }
            }
            let _ = std::fs::remove_file(&backup);
        }

        // Restore Dock
        if delay == 86400.0 {
            let _ = std::process::Command::new("defaults")
                .args(["write", "com.apple.dock", "autohide", "-bool", "false"])
                .status();
            let _ = std::process::Command::new("defaults")
                .args(["delete", "com.apple.dock", "autohide-delay"])
                .status();
        }

        let _ = std::process::Command::new("killall")
            .arg("Dock")
            .status();
        info!("cursor_clamp: recovered from previous crash");
    }

    /// Called when Corkscrew's window gains focus. If the cursor fix is active
    /// and the game process is gone, deactivate immediately.
    pub fn check_and_maybe_deactivate() {
        if !ACTIVE.load(Ordering::SeqCst) {
            return;
        }

        if !is_game_running() {
            info!("check_and_maybe_deactivate: game not running — deactivating on focus");
            deactivate();
        }
    }

    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    pub fn has_permission() -> bool {
        let result = unsafe { AXIsProcessTrusted() };
        info!("cursor_clamp::has_permission: AXIsProcessTrusted() = {}", result);
        result
    }

    pub fn request_permission() -> bool {
        unsafe {
            let keys = [kAXTrustedCheckOptionPrompt];
            let values = [kCFBooleanTrue];
            let options = CFDictionaryCreate(
                ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                &kCFTypeDictionaryKeyCallBacks as *const _ as *const c_void,
                &kCFTypeDictionaryValueCallBacks as *const _ as *const c_void,
            );
            let result = AXIsProcessTrustedWithOptions(options as *const c_void);
            if !options.is_null() {
                CFRelease(options as *const c_void);
            }
            info!(
                "request_permission: AXIsProcessTrustedWithOptions(prompt=true) = {}",
                result
            );
            result
        }
    }

    pub fn is_active() -> bool {
        ACTIVE.load(Ordering::SeqCst)
    }

    fn detect_display_height_points() -> u32 {
        let bounds = unsafe { CGDisplayBounds(CGMainDisplayID()) };
        let h = bounds.size.height as u32;
        info!("cursor_clamp: display height = {} points (logical)", h);
        h
    }

    pub fn activate(_screen_height: u32, _game_pid: u32, game_exe: &str) -> Result<(), String> {
        if ACTIVE.load(Ordering::SeqCst) {
            debug!("Cursor clamp already active");
            return Ok(());
        }

        // Store the game executable name for process detection
        if let Ok(mut s) = GAME_EXE.lock() {
            *s = game_exe.to_string();
        }

        // Layer 1: Suppress Hot Corners + Dock (no permissions needed)
        // Hot corners MUST be written before suppress_dock(), because
        // suppress_dock() does `killall Dock` which applies all changes.
        suppress_hot_corners();
        suppress_dock();

        let screen_height = detect_display_height_points();
        SCREEN_HEIGHT.store(screen_height, Ordering::SeqCst);
        ACTIVE.store(true, Ordering::SeqCst);

        spawn_game_watcher();

        // Layer 2+3: CGDisplayHideCursor + event tap (needs Accessibility)
        if has_permission() {
            let handle = thread::Builder::new()
                .name("cursor-clamp".into())
                .spawn(run_event_tap)
                .map_err(|e| format!("Failed to spawn cursor clamp thread: {}", e))?;

            if let Ok(mut t) = TAP_THREAD.lock() {
                *t = Some(handle);
            }

            thread::sleep(std::time::Duration::from_millis(100));
            info!("cursor_clamp: fully activated (dock + corners + event tap + cursor hide)");
        } else {
            info!("cursor_clamp: activated with dock + corners suppression only (no Accessibility)");
        }

        Ok(())
    }

    pub fn deactivate() {
        if ACTIVE
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        info!(
            "cursor_clamp: deactivating (events={}, clamped={})",
            EVENT_COUNT.load(Ordering::Relaxed),
            CLAMP_COUNT.load(Ordering::Relaxed),
        );

        if CURSOR_HIDDEN.swap(false, Ordering::SeqCst) {
            unsafe {
                CGDisplayShowCursor(CGMainDisplayID());
            }
            info!("cursor_clamp: cursor restored (CGDisplayShowCursor)");
        }

        let rl = RUN_LOOP_REF.load(Ordering::SeqCst);
        if !rl.is_null() {
            unsafe { CFRunLoopStop(rl) };
        }

        if let Ok(mut t) = TAP_THREAD.lock() {
            if let Some(handle) = t.take() {
                let _ = handle.join();
            }
        }

        if let Ok(mut t) = WATCHER_THREAD.lock() {
            let _ = t.take();
        }

        RUN_LOOP_REF.store(ptr::null_mut(), Ordering::SeqCst);
        EVENT_TAP_REF.store(ptr::null_mut(), Ordering::SeqCst);

        if let Ok(mut s) = GAME_EXE.lock() {
            s.clear();
        }

        // Restore Hot Corners first, then Dock (Dock restart applies both)
        restore_hot_corners();
        restore_dock();

        info!("cursor_clamp: fully deactivated");
    }

    // -----------------------------------------------------------------------
    // Event tap callback (mouse events only)
    // -----------------------------------------------------------------------

    unsafe extern "C" fn event_tap_callback(
        _proxy: *const c_void,
        event_type: u32,
        event: *mut c_void,
        _user_info: *mut c_void,
    ) -> *mut c_void {
        if event_type == K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT {
            let tap = EVENT_TAP_REF.load(Ordering::Relaxed);
            if !tap.is_null() {
                CGEventTapEnable(tap, true);
                info!("Re-enabled cursor clamp event tap after timeout");
            }
            return event;
        }

        if !CURSOR_HIDDEN.load(Ordering::Relaxed) {
            CGDisplayHideCursor(CGMainDisplayID());
            CURSOR_HIDDEN.store(true, Ordering::Relaxed);
            info!("cursor_clamp: CGDisplayHideCursor called");
        }

        let screen_h = SCREEN_HEIGHT.load(Ordering::Relaxed) as f64;
        if screen_h <= 0.0 {
            return event;
        }

        let max_y = screen_h - BOTTOM_MARGIN;
        let location = CGEventGetLocation(event);

        let count = EVENT_COUNT.fetch_add(1, Ordering::Relaxed);
        if count % 500 == 0 && count > 0 {
            let clamped = CLAMP_COUNT.load(Ordering::Relaxed);
            debug!(
                "cursor_clamp: {} events, {} clamped, y={:.1}, max_y={:.1}",
                count, clamped, location.y, max_y
            );
        }

        if location.y > max_y {
            CLAMP_COUNT.fetch_add(1, Ordering::Relaxed);
            if CLAMP_COUNT.load(Ordering::Relaxed) <= 5 {
                info!(
                    "cursor_clamp: CLAMPED y={:.1} → {:.1} (screen_h={:.0})",
                    location.y, max_y, screen_h
                );
            }
            CGEventSetLocation(
                event,
                CGPoint {
                    x: location.x,
                    y: max_y,
                },
            );
        }

        event
    }

    // -----------------------------------------------------------------------
    // Event tap thread
    // -----------------------------------------------------------------------

    fn run_event_tap() {
        unsafe {
            let event_mask: CGEventMask = (1 << K_CG_EVENT_MOUSE_MOVED)
                | (1 << K_CG_EVENT_LEFT_MOUSE_DRAGGED)
                | (1 << K_CG_EVENT_RIGHT_MOUSE_DRAGGED)
                | (1 << K_CG_EVENT_OTHER_MOUSE_DRAGGED);

            let tap = CGEventTapCreate(
                K_CG_SESSION_EVENT_TAP,
                K_CG_HEAD_INSERT_EVENT_TAP,
                K_CG_EVENT_TAP_OPTION_DEFAULT,
                event_mask,
                event_tap_callback,
                ptr::null_mut(),
            );

            if tap.is_null() {
                warn!("Failed to create CGEventTap — Accessibility permission may not be granted");
                return;
            }

            EVENT_TAP_REF.store(tap, Ordering::SeqCst);

            let source = CFMachPortCreateRunLoopSource(ptr::null(), tap, 0);
            if source.is_null() {
                warn!("Failed to create run loop source for cursor clamp event tap");
                CFRelease(tap as *const c_void);
                EVENT_TAP_REF.store(ptr::null_mut(), Ordering::SeqCst);
                return;
            }

            let run_loop = CFRunLoopGetCurrent();
            RUN_LOOP_REF.store(run_loop, Ordering::SeqCst);

            CFRunLoopAddSource(run_loop, source, kCFRunLoopCommonModes);
            CGEventTapEnable(tap, true);

            let screen_h = SCREEN_HEIGHT.load(Ordering::Relaxed);
            let exe = match GAME_EXE.lock() {
                Ok(s) => s.clone(),
                Err(e) => e.into_inner().clone(),
            };
            info!(
                "Event tap active (screen={}pts, max_y={}, game='{}')",
                screen_h,
                screen_h as f64 - BOTTOM_MARGIN,
                exe,
            );

            CFRunLoopRun();

            CGEventTapEnable(tap, false);
            CFRelease(source as *const c_void);
            CFRelease(tap as *const c_void);
            EVENT_TAP_REF.store(ptr::null_mut(), Ordering::SeqCst);
            RUN_LOOP_REF.store(ptr::null_mut(), Ordering::SeqCst);
            EVENT_COUNT.store(0, Ordering::Relaxed);
            CLAMP_COUNT.store(0, Ordering::Relaxed);
            info!("Event tap thread exited");
        }
    }
}
