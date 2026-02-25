//! macOS cursor edge clamping via CGEventTap.
//!
//! When running games through Wine/CrossOver in exclusive fullscreen, pushing
//! the cursor to the very bottom of the screen triggers macOS's Dock auto-show
//! evaluation zone. Even though the Dock never appears (CGDisplayCapture blocks
//! it), the evaluation causes macOS to override Wine's `[NSCursor hide]` and
//! make the system cursor visible.
//!
//! This module installs a CGEventTap that intercepts mouse movement events and
//! clamps the cursor Y position to prevent it from ever reaching the trigger
//! zone. Only the absolute position is modified — relative deltas (used by
//! games for camera movement) are untouched, so in-game controls are unaffected.
//!
//! The tap auto-deactivates when the monitored game process exits. If Corkscrew
//! itself crashes, macOS automatically removes the tap (OS-level cleanup).
//!
//! Requires macOS Accessibility permission (System Settings > Privacy & Security
//! > Accessibility). Falls back gracefully (no-op) if permission is not granted.

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

    /// Pixels from the bottom screen edge to block.
    /// The macOS Dock trigger zone is approximately 4 pixels.
    const BOTTOM_MARGIN: f64 = 4.0;

    /// How often (seconds) to check if the game process is still alive.
    const PID_CHECK_INTERVAL: f64 = 2.0;

    // -----------------------------------------------------------------------
    // Static state
    // -----------------------------------------------------------------------

    static ACTIVE: AtomicBool = AtomicBool::new(false);
    static SCREEN_HEIGHT: AtomicU32 = AtomicU32::new(0);
    static GAME_PID: AtomicU32 = AtomicU32::new(0);
    static EVENT_TAP_REF: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
    static RUN_LOOP_REF: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
    static TAP_THREAD: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

    // -----------------------------------------------------------------------
    // CoreGraphics / CoreFoundation FFI
    // -----------------------------------------------------------------------

    #[repr(C)]
    #[derive(Debug, Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    // CGEventTapLocation
    const K_CG_SESSION_EVENT_TAP: u32 = 1;
    // CGEventTapPlacement
    const K_CG_HEAD_INSERT_EVENT_TAP: u32 = 0;
    // CGEventTapOptions — active (can modify events)
    const K_CG_EVENT_TAP_OPTION_DEFAULT: u32 = 0;

    // CGEventType values
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
    type CFTimeInterval = f64;

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
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFMachPortCreateRunLoopSource(
            allocator: *const c_void,
            port: *mut c_void,
            order: CFIndex,
        ) -> *mut c_void;

        fn CFRunLoopAddSource(
            rl: *mut c_void,
            source: *mut c_void,
            mode: *const c_void,
        );

        fn CFRunLoopGetCurrent() -> *mut c_void;
        fn CFRunLoopRun();
        fn CFRunLoopStop(rl: *mut c_void);
        fn CFRelease(cf: *const c_void);

        fn CFRunLoopAddTimer(
            rl: *mut c_void,
            timer: *mut c_void,
            mode: *const c_void,
        );

        fn CFRunLoopTimerCreate(
            allocator: *const c_void,
            fire_date: f64,
            interval: CFTimeInterval,
            flags: u64,
            order: CFIndex,
            callout: unsafe extern "C" fn(*mut c_void, *mut c_void),
            context: *mut c_void,
        ) -> *mut c_void;

        fn CFAbsoluteTimeGetCurrent() -> f64;

        static kCFRunLoopCommonModes: *const c_void;
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }

    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    /// Check if Corkscrew has Accessibility permission.
    ///
    /// Uses a test `CGEventTapCreate` instead of `AXIsProcessTrusted()` because
    /// the latter caches its result at process startup and doesn't reliably
    /// update when the user toggles the permission in System Settings.
    pub fn has_permission() -> bool {
        // A no-op callback — we never actually process events from this tap.
        unsafe extern "C" fn noop_callback(
            _proxy: *const c_void,
            _type: u32,
            event: *mut c_void,
            _user_info: *mut c_void,
        ) -> *mut c_void {
            event
        }

        let mask: CGEventMask = 1 << K_CG_EVENT_MOUSE_MOVED;
        let tap = unsafe {
            CGEventTapCreate(
                K_CG_SESSION_EVENT_TAP,
                K_CG_HEAD_INSERT_EVENT_TAP,
                K_CG_EVENT_TAP_OPTION_DEFAULT,
                mask,
                noop_callback,
                std::ptr::null_mut(),
            )
        };
        if tap.is_null() {
            return false; // No permission
        }
        // Permission granted — clean up the test tap immediately
        unsafe { CFRelease(tap as *const c_void) };
        true
    }

    /// Request Accessibility permission by opening System Settings.
    /// Returns `true` if already granted.
    pub fn request_permission() -> bool {
        if has_permission() {
            return true;
        }
        let _ = std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .spawn();
        false
    }

    /// Check if cursor clamping is currently active.
    pub fn is_active() -> bool {
        ACTIVE.load(Ordering::SeqCst)
    }

    /// Activate cursor edge clamping for the duration of a game session.
    ///
    /// The tap monitors `game_pid` and auto-deactivates when the process exits.
    /// If Corkscrew crashes, macOS automatically removes the tap.
    pub fn activate(screen_height: u32, game_pid: u32) -> Result<(), String> {
        if ACTIVE.load(Ordering::SeqCst) {
            debug!("Cursor clamp already active, updating PID to {}", game_pid);
            GAME_PID.store(game_pid, Ordering::SeqCst);
            return Ok(());
        }

        if !has_permission() {
            return Err(
                "Accessibility permission required for cursor clamping. \
                 Grant Corkscrew access in System Settings > Privacy & Security > Accessibility."
                    .into(),
            );
        }

        SCREEN_HEIGHT.store(screen_height, Ordering::SeqCst);
        GAME_PID.store(game_pid, Ordering::SeqCst);

        let handle = thread::Builder::new()
            .name("cursor-clamp".into())
            .spawn(run_event_tap)
            .map_err(|e| format!("Failed to spawn cursor clamp thread: {}", e))?;

        if let Ok(mut t) = TAP_THREAD.lock() {
            *t = Some(handle);
        }

        // Wait briefly for the tap to activate
        thread::sleep(std::time::Duration::from_millis(100));

        if ACTIVE.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err("Event tap failed to activate — check Accessibility permission".into())
        }
    }

    /// Manually deactivate cursor edge clamping.
    pub fn deactivate() {
        if !ACTIVE.load(Ordering::SeqCst) {
            return;
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

        RUN_LOOP_REF.store(ptr::null_mut(), Ordering::SeqCst);
        EVENT_TAP_REF.store(ptr::null_mut(), Ordering::SeqCst);
    }

    // -----------------------------------------------------------------------
    // Event tap callback
    // -----------------------------------------------------------------------

    /// Clamp cursor Y to prevent reaching the bottom screen edge.
    ///
    /// Only modifies the absolute position — relative deltas (used by games
    /// for camera/look movement) are untouched.
    unsafe extern "C" fn event_tap_callback(
        _proxy: *const c_void,
        event_type: u32,
        event: *mut c_void,
        _user_info: *mut c_void,
    ) -> *mut c_void {
        // Re-enable tap if macOS disabled it due to timeout
        if event_type == K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT {
            let tap = EVENT_TAP_REF.load(Ordering::Relaxed);
            if !tap.is_null() {
                CGEventTapEnable(tap, true);
                debug!("Re-enabled cursor clamp event tap after timeout");
            }
            return event;
        }

        let screen_h = SCREEN_HEIGHT.load(Ordering::Relaxed) as f64;
        if screen_h <= 0.0 {
            return event;
        }

        let max_y = screen_h - BOTTOM_MARGIN;
        let location = CGEventGetLocation(event);

        if location.y > max_y {
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

    /// Timer callback: check if the game process is still alive.
    /// If dead, stop the run loop to deactivate the tap.
    unsafe extern "C" fn pid_check_callback(
        _timer: *mut c_void,
        _info: *mut c_void,
    ) {
        let pid = GAME_PID.load(Ordering::Relaxed);
        if pid == 0 {
            return;
        }

        // kill(pid, 0) returns 0 if the process exists, -1 otherwise
        let alive = libc::kill(pid as i32, 0) == 0;
        if !alive {
            info!(
                "Game process {} exited — deactivating cursor clamp",
                pid
            );
            let rl = RUN_LOOP_REF.load(Ordering::Relaxed);
            if !rl.is_null() {
                CFRunLoopStop(rl);
            }
        }
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

            // Create a timer to periodically check if the game process is alive
            let fire_date = CFAbsoluteTimeGetCurrent() + PID_CHECK_INTERVAL;
            let timer = CFRunLoopTimerCreate(
                ptr::null(),
                fire_date,
                PID_CHECK_INTERVAL,
                0,
                0,
                pid_check_callback,
                ptr::null_mut(),
            );

            if !timer.is_null() {
                CFRunLoopAddTimer(run_loop, timer, kCFRunLoopCommonModes);
            }

            let screen_h = SCREEN_HEIGHT.load(Ordering::Relaxed);
            let pid = GAME_PID.load(Ordering::Relaxed);
            ACTIVE.store(true, Ordering::SeqCst);
            info!(
                "Cursor clamp activated (screen_height={}, max_y={}, game_pid={})",
                screen_h,
                screen_h as f64 - BOTTOM_MARGIN,
                pid,
            );

            // Block until CFRunLoopStop is called (by PID check or manual deactivate)
            CFRunLoopRun();

            // Cleanup
            CGEventTapEnable(tap, false);
            if !timer.is_null() {
                CFRelease(timer as *const c_void);
            }
            CFRelease(source as *const c_void);
            CFRelease(tap as *const c_void);
            EVENT_TAP_REF.store(ptr::null_mut(), Ordering::SeqCst);
            RUN_LOOP_REF.store(ptr::null_mut(), Ordering::SeqCst);
            ACTIVE.store(false, Ordering::SeqCst);
            info!("Cursor clamp deactivated");
        }
    }
}
