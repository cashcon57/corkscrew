//! macOS cursor fix for fullscreen Wine/CrossOver games.
//!
//! Problem: When running games through Wine/CrossOver in fullscreen, the macOS
//! Dock auto-show trigger zone at the bottom of the screen causes the system
//! cursor to become visible — even though the game hides it. The Dock's trigger
//! zone evaluation happens at the WindowServer level, BEFORE event taps see the
//! event, so CGEventTap position clamping alone cannot prevent it.
//!
//! Solution (layered):
//! 1. **Dock suppression**: Temporarily set `autohide-delay` to a huge value
//!    so the Dock trigger zone never activates. Restored on game exit.
//! 2. **CGDisplayHideCursor**: Force-hide the system cursor at the CG level.
//! 3. **CGEventTap Y-clamp**: Belt-and-suspenders — clamp cursor position
//!    away from the bottom edge as a secondary defense.
//!
//! Game exit detection:
//! - Primary: PID watcher thread polls the launched Wine process. When the
//!   PID dies, the game has exited → deactivate.
//! - Secondary: Window focus handler — when Corkscrew regains focus and the
//!   game PID is dead, deactivate immediately.
//! - Tertiary: Tauri RunEvent::Exit handler calls deactivate() on app close.
//! - Recovery: On next app start, detects sentinel autohide-delay=86400.

#[cfg(not(target_os = "macos"))]
pub fn activate(_screen_height: u32, _game_pid: u32) -> Result<(), String> {
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

    /// How often (seconds) to check if the game process is still alive.
    const PID_CHECK_INTERVAL: f64 = 3.0;

    /// Grace period after activation before checking PID (seconds).
    /// Wine launchers need a moment to start the game process.
    const PID_CHECK_GRACE: f64 = 10.0;

    // -----------------------------------------------------------------------
    // Static state
    // -----------------------------------------------------------------------

    static ACTIVE: AtomicBool = AtomicBool::new(false);
    static SCREEN_HEIGHT: AtomicU32 = AtomicU32::new(0);
    static GAME_PID: AtomicU32 = AtomicU32::new(0);
    static CURSOR_HIDDEN: AtomicBool = AtomicBool::new(false);
    static CLAMP_COUNT: AtomicU32 = AtomicU32::new(0);
    static EVENT_COUNT: AtomicU32 = AtomicU32::new(0);
    static EVENT_TAP_REF: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
    static RUN_LOOP_REF: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
    static TAP_THREAD: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);
    static PID_THREAD: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);
    static DOCK_SUPPRESSED: AtomicBool = AtomicBool::new(false);
    static DOCK_ORIGINAL_DELAY: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);

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
    // PID watcher
    //
    // Simple approach: the launched Wine PID is alive while the game runs.
    // When it dies, the game has exited. No wineserver/preloader checks needed
    // — CrossOver's wine binary stays alive for the game's lifetime.
    // -----------------------------------------------------------------------

    fn spawn_pid_watcher(game_pid: u32) {
        let handle = thread::Builder::new()
            .name("cursor-pid-watch".into())
            .spawn(move || {
                let start = std::time::Instant::now();

                info!(
                    "pid_watcher: started for PID {} (grace={}s, interval={}s)",
                    game_pid, PID_CHECK_GRACE, PID_CHECK_INTERVAL
                );

                loop {
                    thread::sleep(std::time::Duration::from_secs_f64(PID_CHECK_INTERVAL));

                    if !ACTIVE.load(Ordering::SeqCst) {
                        info!("pid_watcher: ACTIVE=false, exiting");
                        return;
                    }

                    // Grace period for Wine to start the game
                    if start.elapsed().as_secs_f64() < PID_CHECK_GRACE {
                        debug!(
                            "pid_watcher: grace period ({:.0}s / {}s)",
                            start.elapsed().as_secs_f64(), PID_CHECK_GRACE
                        );
                        continue;
                    }

                    let pid_alive =
                        unsafe { libc::kill(game_pid as i32, 0) } == 0;

                    if pid_alive {
                        // Game still running
                        continue;
                    }

                    // PID is dead → game has exited
                    info!(
                        "pid_watcher: PID {} exited after {:.0}s — deactivating",
                        game_pid, start.elapsed().as_secs_f64()
                    );
                    deactivate();
                    return;
                }
            })
            .ok();

        if let Some(h) = handle {
            if let Ok(mut t) = PID_THREAD.lock() {
                *t = Some(h);
            }
        }
    }

    /// Called on app startup to recover from a previous crash that left the
    /// Dock suppressed. Checks if autohide-delay is 86400 (our sentinel).
    pub fn recover_dock_if_needed() {
        let delay = read_dock_autohide_delay();
        if delay == 86400.0 {
            warn!(
                "cursor_clamp: detected leftover Dock suppression (delay=86400) — restoring"
            );
            let _ = std::process::Command::new("defaults")
                .args(["write", "com.apple.dock", "autohide", "-bool", "false"])
                .status();
            let _ = std::process::Command::new("defaults")
                .args(["delete", "com.apple.dock", "autohide-delay"])
                .status();
            let _ = std::process::Command::new("killall")
                .arg("Dock")
                .status();
            info!("cursor_clamp: recovered Dock from previous crash");
        }
    }

    /// Called when Corkscrew's window gains focus. If the cursor fix is active
    /// and the game PID is dead, deactivate immediately. This provides instant
    /// cleanup when the game exits and macOS switches focus to Corkscrew.
    pub fn check_and_maybe_deactivate() {
        if !ACTIVE.load(Ordering::SeqCst) {
            return;
        }
        let pid = GAME_PID.load(Ordering::SeqCst);
        if pid == 0 {
            return;
        }

        let pid_alive = unsafe { libc::kill(pid as i32, 0) } == 0;
        if !pid_alive {
            info!(
                "check_and_maybe_deactivate: PID {} dead — deactivating on window focus",
                pid
            );
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

    pub fn activate(_screen_height: u32, game_pid: u32) -> Result<(), String> {
        if ACTIVE.load(Ordering::SeqCst) {
            debug!("Cursor clamp already active, updating PID to {}", game_pid);
            GAME_PID.store(game_pid, Ordering::SeqCst);
            return Ok(());
        }

        suppress_dock();

        let screen_height = detect_display_height_points();
        SCREEN_HEIGHT.store(screen_height, Ordering::SeqCst);
        GAME_PID.store(game_pid, Ordering::SeqCst);
        ACTIVE.store(true, Ordering::SeqCst);

        spawn_pid_watcher(game_pid);

        if has_permission() {
            let handle = thread::Builder::new()
                .name("cursor-clamp".into())
                .spawn(run_event_tap)
                .map_err(|e| format!("Failed to spawn cursor clamp thread: {}", e))?;

            if let Ok(mut t) = TAP_THREAD.lock() {
                *t = Some(handle);
            }

            thread::sleep(std::time::Duration::from_millis(100));
            info!("cursor_clamp: fully activated (dock suppression + event tap + cursor hide)");
        } else {
            info!("cursor_clamp: activated with dock suppression only (no Accessibility permission)");
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

        if let Ok(mut t) = PID_THREAD.lock() {
            let _ = t.take();
        }

        RUN_LOOP_REF.store(ptr::null_mut(), Ordering::SeqCst);
        EVENT_TAP_REF.store(ptr::null_mut(), Ordering::SeqCst);

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
            let pid = GAME_PID.load(Ordering::Relaxed);
            info!(
                "Event tap active (screen={}pts, max_y={}, pid={})",
                screen_h,
                screen_h as f64 - BOTTOM_MARGIN,
                pid,
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
