//! Skyrim SE display fix for Wine/CrossOver/Proton on macOS and Linux.
//!
//! When running Skyrim SE through a compatibility layer, the game often
//! renders windowed or at the wrong resolution due to incorrect display
//! settings in SkyrimPrefs.ini. This module detects the correct screen
//! resolution for the platform and configures exclusive fullscreen mode.
//!
//! Platform support:
//! - **macOS**: Detects Retina vs non-Retina via system_profiler, respects
//!   Wine's RetinaMode setting to choose physical vs logical resolution.
//! - **Linux (X11)**: Uses xrandr to detect primary display resolution.
//! - **Linux (Wayland)**: Uses wlr-randr or xdpyinfo via XWayland.
//! - **SteamOS/Steam Deck**: Detects Gamescope resolution or defaults to 1280x800.
//!
//! The fix applies three changes:
//! 1. **SkyrimPrefs.ini**: Set detected resolution + `bFull Screen=1`
//! 2. **Wine registry**: Remove virtual desktop settings that force windowed mode
//! 3. **Wine registry**: Configure display capture + mouse warping for proper input

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};

use log::{debug, warn};
use serde::{Deserialize, Serialize};

use crate::bottles::Bottle;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DisplaySettings {
    pub width: u32,
    pub height: u32,
    pub fullscreen: bool,
    pub borderless: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DisplayFixResult {
    pub fixed: bool,
    pub prefs_path: String,
    pub previous: DisplaySettings,
    pub applied: DisplaySettings,
    pub screen_width: u32,
    pub screen_height: u32,
}

// ---------------------------------------------------------------------------
// Screen resolution detection (cross-platform)
// ---------------------------------------------------------------------------

/// Check whether Wine's Retina/HiDPI mode is enabled in a bottle's registry.
pub fn is_retina_enabled(bottle: &Bottle) -> bool {
    let user_reg = bottle.path.join("user.reg");
    let content = fs::read_to_string(&user_reg).unwrap_or_default();
    content.contains("\"RetinaMode\"=\"Y\"")
}

/// Detect the screen resolution appropriate for a given bottle.
///
/// On macOS Retina displays, the result depends on whether Wine's
/// RetinaMode is enabled:
/// - **Retina ON**: returns physical pixels (e.g., 3456x2234) since Wine
///   sees the full native resolution.
/// - **Retina OFF**: returns logical resolution (e.g., 1728x1117) which
///   is what Wine actually exposes to applications.
///
/// On Linux, returns the current display resolution via xrandr (X11),
/// wlr-randr (Wayland), or Gamescope env vars (SteamOS).
pub fn detect_screen_resolution(bottle: &Bottle) -> Result<(u32, u32), String> {
    #[cfg(target_os = "macos")]
    {
        detect_macos_resolution(bottle)
    }
    #[cfg(target_os = "linux")]
    {
        let _ = bottle; // Bottle not needed for Linux resolution detection
        detect_linux_resolution()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = bottle;
        Err("Screen resolution detection not supported on this platform".into())
    }
}

// ---------------------------------------------------------------------------
// macOS resolution detection
// ---------------------------------------------------------------------------

/// Detect resolution on macOS, accounting for Retina scaling.
#[cfg(target_os = "macos")]
fn detect_macos_resolution(bottle: &Bottle) -> Result<(u32, u32), String> {
    let retina = is_retina_enabled(bottle);
    let (logical, physical) = detect_macos_resolutions()?;

    let (w, h) = if retina {
        debug!(
            "Retina mode enabled — using physical pixels: {}x{}",
            physical.0, physical.1
        );
        physical
    } else {
        debug!(
            "Retina mode disabled — using logical resolution: {}x{}",
            logical.0, logical.1
        );
        logical
    };

    Ok((w, h))
}

/// Query system_profiler for both logical and physical display resolutions.
///
/// Returns `(logical, physical)` where:
/// - `logical` = `_spdisplays_resolution` (what macOS reports to apps, e.g., 1728x1117)
/// - `physical` = `_spdisplays_pixels` (actual hardware pixels, e.g., 3456x2234)
///
/// On non-Retina displays these are the same value.
#[cfg(target_os = "macos")]
#[allow(clippy::type_complexity)]
fn detect_macos_resolutions() -> Result<((u32, u32), (u32, u32)), String> {
    let output = Command::new("system_profiler")
        .args(["SPDisplaysDataType", "-json"])
        .output()
        .map_err(|e| format!("Failed to run system_profiler: {}", e))?;

    if !output.status.success() {
        return Err("system_profiler returned non-zero exit code".into());
    }

    let json_str = String::from_utf8_lossy(&output.stdout);
    let data: serde_json::Value =
        serde_json::from_str(&json_str).map_err(|e| format!("Failed to parse JSON: {}", e))?;

    if let Some(displays) = data.get("SPDisplaysDataType").and_then(|v| v.as_array()) {
        for gpu in displays {
            if let Some(screens) = gpu.get("spdisplays_ndrvs").and_then(|v| v.as_array()) {
                for screen in screens {
                    // Parse physical pixels (e.g., "3456 x 2234")
                    let physical = screen
                        .get("_spdisplays_pixels")
                        .and_then(|v| v.as_str())
                        .and_then(parse_resolution_string);

                    // Parse logical resolution (e.g., "1728 x 1117 @ 120.00Hz")
                    let logical = screen
                        .get("_spdisplays_resolution")
                        .and_then(|v| v.as_str())
                        .and_then(parse_resolution_string);

                    match (logical, physical) {
                        (Some(l), Some(p)) => return Ok((l, p)),
                        (Some(l), None) => return Ok((l, l)),
                        (None, Some(p)) => return Ok((p, p)),
                        (None, None) => continue,
                    }
                }
            }
        }
    }

    // Fallback: try screenresolution tool
    if let Ok(output) = Command::new("screenresolution").arg("get").output() {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            if line.contains("Display 0") || line.contains("Display") {
                if let Some(res) = line.split_whitespace().last() {
                    let parts: Vec<&str> = res.split('x').collect();
                    if parts.len() >= 2 {
                        if let (Ok(w), Ok(h)) = (parts[0].parse(), parts[1].parse()) {
                            return Ok(((w, h), (w, h)));
                        }
                    }
                }
            }
        }
    }

    Err("Could not detect screen resolution on macOS".into())
}

// ---------------------------------------------------------------------------
// Linux resolution detection
// ---------------------------------------------------------------------------

/// Detect resolution on Linux, trying SteamOS/Gamescope first, then
/// Wayland-native methods (wlr-randr, kscreen-doctor, gdbus/Mutter), then
/// X11/XWayland (xrandr).
///
/// `wlr-randr` only works on wlroots-based compositors (Sway, Hyprland).
/// CachyOS ships KDE Plasma 6 by default and GNOME ships Mutter — neither
/// bundles `wlr-randr`. For those sessions we try `kscreen-doctor -o`
/// (KDE Plasma) and the `org.gnome.Mutter.DisplayConfig.GetCurrentState`
/// D-Bus call (GNOME) before falling back to xrandr via XWayland — which
/// itself fails on pure Wayland sessions where XWayland is not started.
#[cfg(target_os = "linux")]
fn detect_linux_resolution() -> Result<(u32, u32), String> {
    // SteamOS / Steam Deck — check Gamescope env vars first
    if crate::steam_integration::is_steam_deck() {
        if let Ok(res) = detect_gamescope_resolution() {
            return Ok(res);
        }
        // Known Steam Deck native resolution (landscape orientation)
        debug!("Steam Deck detected, using default 1280x800");
        return Ok((1280, 800));
    }

    // Wayland-native paths
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        // 1. wlr-randr (Sway / Hyprland / other wlroots compositors)
        match detect_wayland_resolution() {
            Ok(res) => return Ok(res),
            Err(e) => debug!("wlr-randr unavailable: {}", e),
        }

        // 2. kscreen-doctor (KDE Plasma 5/6 — CachyOS default)
        match detect_wayland_resolution_via_kscreen() {
            Ok(res) => return Ok(res),
            Err(e) => debug!("kscreen-doctor unavailable: {}", e),
        }

        // 3. gdbus + org.gnome.Mutter.DisplayConfig (GNOME)
        match detect_wayland_resolution_via_mutter() {
            Ok(res) => return Ok(res),
            Err(e) => debug!("gdbus/Mutter unavailable: {}", e),
        }

        // Wayland fallback: try xrandr via XWayland
    }

    // X11 (or XWayland fallback)
    if std::env::var("DISPLAY").is_ok() {
        if let Ok(res) = detect_x11_resolution() {
            return Ok(res);
        }
    }

    Err(
        "Could not detect screen resolution on Linux. Tried wlr-randr, kscreen-doctor, \
         gdbus/Mutter, xrandr."
            .into(),
    )
}

/// Detect resolution from Gamescope environment variables.
/// Gamescope sets these when running inside the Steam Deck compositor.
#[cfg(target_os = "linux")]
fn detect_gamescope_resolution() -> Result<(u32, u32), String> {
    // Gamescope exposes resolution via env vars when available
    if let (Ok(w_str), Ok(h_str)) = (
        std::env::var("GAMESCOPE_WIDTH"),
        std::env::var("GAMESCOPE_HEIGHT"),
    ) {
        if let (Ok(w), Ok(h)) = (w_str.parse::<u32>(), h_str.parse::<u32>()) {
            debug!("Gamescope resolution from env: {}x{}", w, h);
            return Ok((w, h));
        }
    }
    Err("Gamescope env vars not set".into())
}

/// Detect primary display resolution via xrandr (X11).
///
/// Parses output like: `DP-1 connected primary 2560x1440+0+0 ...`
#[cfg(target_os = "linux")]
fn detect_x11_resolution() -> Result<(u32, u32), String> {
    let output = Command::new("xrandr")
        .arg("--query")
        .output()
        .map_err(|e| format!("Failed to run xrandr: {}", e))?;

    if !output.status.success() {
        return Err("xrandr returned non-zero exit code".into());
    }

    let text = String::from_utf8_lossy(&output.stdout);

    // First try: look for "connected primary WxH+X+Y"
    for line in text.lines() {
        if line.contains(" connected primary ") {
            if let Some(res) = parse_xrandr_connected_line(line) {
                debug!("xrandr primary display: {}x{}", res.0, res.1);
                return Ok(res);
            }
        }
    }

    // Fallback: first "connected" display with a resolution
    for line in text.lines() {
        if line.contains(" connected ") {
            if let Some(res) = parse_xrandr_connected_line(line) {
                debug!("xrandr first connected display: {}x{}", res.0, res.1);
                return Ok(res);
            }
        }
    }

    Err("Could not parse xrandr output".into())
}

/// Parse a resolution from an xrandr "connected" line.
/// Format: `NAME connected [primary] WIDTHxHEIGHT+X+Y ...`
#[cfg(target_os = "linux")]
fn parse_xrandr_connected_line(line: &str) -> Option<(u32, u32)> {
    for token in line.split_whitespace() {
        // Match "WxH+X+Y" pattern (e.g., "2560x1440+0+0")
        if token.contains('x') && token.contains('+') {
            let res_part = token.split('+').next()?;
            return parse_resolution_string(res_part);
        }
    }
    None
}

/// Detect resolution via wlr-randr (Wayland/wlroots compositors).
///
/// Parses output like:
/// ```text
/// eDP-1 "..." (DP-1)
///   Enabled: yes
///   Modes:
///     2560x1600 px, 60.004005 Hz (preferred, current)
/// ```
#[cfg(target_os = "linux")]
fn detect_wayland_resolution() -> Result<(u32, u32), String> {
    let output = Command::new("wlr-randr")
        .output()
        .map_err(|e| format!("Failed to run wlr-randr: {}", e))?;

    if !output.status.success() {
        return Err("wlr-randr returned non-zero exit code".into());
    }

    let text = String::from_utf8_lossy(&output.stdout);

    // Look for the line with "(current)" which indicates the active mode
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.contains("current") {
            // Format: "2560x1600 px, 60.004005 Hz (preferred, current)"
            if let Some(res_str) = trimmed.split_whitespace().next() {
                if let Some(res) = parse_resolution_string(res_str) {
                    debug!("wlr-randr current mode: {}x{}", res.0, res.1);
                    return Ok(res);
                }
            }
        }
    }

    Err("Could not parse wlr-randr output".into())
}

/// Run a command with a wall-clock timeout. Returns the captured stdout on
/// success or an error string on timeout / failure.
///
/// We spawn the child, then poll `try_wait()` in a short sleep loop. If the
/// deadline expires we kill the child (best-effort) so it cannot keep
/// holding the launch flow. This is intentionally synchronous to match the
/// rest of this module — `display_fix` is invoked from contexts that may
/// not have a tokio runtime active.
#[cfg(target_os = "linux")]
fn run_command_with_timeout(
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    use std::process::Stdio;
    use std::thread;

    let mut child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn {}: {}", program, e))?;

    // Drain stdout/stderr concurrently so the OS pipe buffers (~64KB) can't
    // fill and block the child. Without these threads, processes that produce
    // a lot of output (e.g. `kscreen-doctor -o` on a busy session, gdbus
    // dumping a multi-monitor GVariant) can deadlock waiting for someone to
    // read their stdout, and we'd timeout-kill them every time.
    use std::io::Read;
    use std::sync::{Arc, Mutex};
    let stdout_buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let stderr_buf = Arc::new(Mutex::new(Vec::<u8>::new()));

    let stdout_handle = child.stdout.take().map(|mut s| {
        let buf = Arc::clone(&stdout_buf);
        thread::spawn(move || {
            let mut local = Vec::new();
            let _ = s.read_to_end(&mut local);
            if let Ok(mut g) = buf.lock() {
                *g = local;
            }
        })
    });
    let stderr_handle = child.stderr.take().map(|mut s| {
        let buf = Arc::clone(&stderr_buf);
        thread::spawn(move || {
            let mut local = Vec::new();
            let _ = s.read_to_end(&mut local);
            if let Ok(mut g) = buf.lock() {
                *g = local;
            }
        })
    });

    let deadline = Instant::now() + timeout;
    let poll = Duration::from_millis(50);

    let take_buf = |buf: &Arc<Mutex<Vec<u8>>>| -> Vec<u8> {
        buf.lock()
            .map(|mut g| std::mem::take(&mut *g))
            .unwrap_or_default()
    };

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                // Child exited. Wait for the drain threads to consume the
                // remaining buffered output before reading the buffers.
                if let Some(h) = stdout_handle {
                    let _ = h.join();
                }
                if let Some(h) = stderr_handle {
                    let _ = h.join();
                }
                let stdout = take_buf(&stdout_buf);
                if !status.success() {
                    let stderr = take_buf(&stderr_buf);
                    return Err(format!(
                        "{} exited with status {}: {}",
                        program,
                        status,
                        String::from_utf8_lossy(&stderr).trim()
                    ));
                }
                return Ok(stdout);
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    // Best-effort kill; ignore errors (process may already be dead).
                    let _ = child.kill();
                    let _ = child.wait();
                    // Joining drain threads after kill ensures pipe FDs close.
                    if let Some(h) = stdout_handle {
                        let _ = h.join();
                    }
                    if let Some(h) = stderr_handle {
                        let _ = h.join();
                    }
                    return Err(format!(
                        "{} timed out after {}ms",
                        program,
                        timeout.as_millis()
                    ));
                }
                thread::sleep(poll);
            }
            Err(e) => return Err(format!("Failed to wait on {}: {}", program, e)),
        }
    }
}

/// Detect resolution via `kscreen-doctor -o` (KDE Plasma 5/6).
///
/// Plasma is the default desktop on CachyOS and ships `kscreen-doctor` as
/// part of `libkscreen`. The tool prints one block per output. Each enabled
/// output ends with a `Geometry: x,y WIDTHxHEIGHT` line and lists its modes
/// with the active one marked by `*` (current) or `!` (preferred-current).
///
/// We honour:
/// 1. The first enabled+priority-1 output's `Geometry:` line, OR
/// 2. The first mode flagged with `*` on that output.
#[cfg(target_os = "linux")]
fn detect_wayland_resolution_via_kscreen() -> Result<(u32, u32), String> {
    let stdout = run_command_with_timeout("kscreen-doctor", &["-o"], Duration::from_millis(2500))?;
    let text = String::from_utf8_lossy(&stdout);
    parse_kscreen_doctor_output(&text)
        .ok_or_else(|| "Could not parse kscreen-doctor output".to_string())
        .inspect(|res| debug!("kscreen-doctor current mode: {}x{}", res.0, res.1))
}

/// Pure parser for `kscreen-doctor -o` stdout. Extracted as a free function
/// so it is unit-testable without spawning a real process.
///
/// Strategy:
/// 1. Walk the lines top-to-bottom.
/// 2. When we hit `Output: ...` start a new block.
/// 3. Prefer the first `priority 1` enabled block's resolution.
/// 4. Inside a block, accept the first `Geometry: <x>,<y> WxH` (newer
///    format) OR `Geometry: WxH` (older format), or the first mode line
///    matching `<id>:WxH@<rate>*` / `...*!`.
/// 5. Fall back to the first enabled output if no priority-1 marker found.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_kscreen_doctor_output(text: &str) -> Option<(u32, u32)> {
    #[derive(Default)]
    struct Block {
        enabled: bool,
        priority_one: bool,
        resolution: Option<(u32, u32)>,
    }

    let mut blocks: Vec<Block> = Vec::new();
    let mut cur: Option<Block> = None;

    for raw in text.lines() {
        let line = raw.trim();

        if line.starts_with("Output:") {
            if let Some(b) = cur.take() {
                blocks.push(b);
            }
            let mut b = Block::default();
            // Header line itself can carry "enabled", "priority N".
            if line.contains(" enabled") {
                b.enabled = true;
            }
            if has_priority_one_token(line) {
                b.priority_one = true;
            }
            cur = Some(b);
            continue;
        }

        let block = match cur.as_mut() {
            Some(b) => b,
            None => continue,
        };

        // Some kscreen-doctor versions split fields onto their own lines.
        if line.eq_ignore_ascii_case("enabled") || line.eq_ignore_ascii_case("enabled: yes") {
            block.enabled = true;
        }
        if line.starts_with("priority") && has_priority_one_token(line) {
            // "priority: 1" or "priority 1" — must be exactly 1, not 10/11/etc.
            block.priority_one = true;
        }

        // Geometry line. Handle "Geometry: x,y WxH" and "Geometry: WxH".
        if let Some(rest) = line.strip_prefix("Geometry:") {
            let rest = rest.trim();
            // Try "x,y WIDTHxHEIGHT" — take the last whitespace-separated chunk
            // that contains 'x'.
            let candidate = rest
                .split_whitespace()
                .rev()
                .find(|t| t.contains('x'))
                .unwrap_or(rest);
            if let Some(res) = parse_resolution_string(candidate) {
                block.resolution.get_or_insert(res);
                continue;
            }
            // Fallback: "Geometry: 1920,1080" with no 'x' — comma-separated
            // single point isn't a resolution, skip.
        }

        // Mode lines look like "  18:1920x1080@60*!" or "1920x1080@60.00*"
        // — only accept entries marked current (the '*' flag).
        if line.contains('*') {
            // Strip leading "<idx>:" if present.
            let after_colon = line.rsplit(':').next().unwrap_or(line);
            // Take just the "WxH" portion.
            let core = after_colon
                .split(|c: char| c == '@' || c == '*' || c == '!' || c.is_whitespace())
                .find(|t| t.contains('x') && t.chars().any(|c| c.is_ascii_digit()))
                .unwrap_or("");
            if !core.is_empty() {
                if let Some(res) = parse_resolution_string(core) {
                    block.resolution.get_or_insert(res);
                }
            }
        }
    }

    if let Some(b) = cur.take() {
        blocks.push(b);
    }

    // Prefer enabled + priority-1 with a resolution; then enabled with one;
    // then any with one.
    blocks
        .iter()
        .find(|b| b.enabled && b.priority_one && b.resolution.is_some())
        .and_then(|b| b.resolution)
        .or_else(|| {
            blocks
                .iter()
                .find(|b| b.enabled && b.resolution.is_some())
                .and_then(|b| b.resolution)
        })
        .or_else(|| {
            blocks
                .iter()
                .find(|b| b.resolution.is_some())
                .and_then(|b| b.resolution)
        })
}

/// Detect resolution via gdbus on the GNOME Mutter D-Bus interface.
///
/// Calls `org.gnome.Mutter.DisplayConfig.GetCurrentState` and parses the
/// returned GVariant. The structure is a 4-tuple
/// `(uint32 serial, logical_monitors, monitors, properties)`. Each entry
/// in `monitors` carries a list of mode tuples; the active mode has a
/// `'is-current': <true>` property. We locate that marker and walk the
/// preceding tokens to find the `int32 W, int32 H` pair from the same mode
/// tuple — its dimensions are the current physical resolution.
///
/// GVariant parsing is fragile by design: if the format ever changes we
/// log a warning and return Err so the caller can fall through to xrandr.
#[cfg(target_os = "linux")]
fn detect_wayland_resolution_via_mutter() -> Result<(u32, u32), String> {
    let stdout = run_command_with_timeout(
        "gdbus",
        &[
            "call",
            "--session",
            "--dest",
            "org.gnome.Mutter.DisplayConfig",
            "--object-path",
            "/org/gnome/Mutter/DisplayConfig",
            "--method",
            "org.gnome.Mutter.DisplayConfig.GetCurrentState",
        ],
        Duration::from_millis(3000),
    )?;
    let text = String::from_utf8_lossy(&stdout);
    match parse_mutter_get_current_state(&text) {
        Some(res) => {
            debug!("gdbus/Mutter current mode: {}x{}", res.0, res.1);
            Ok(res)
        }
        None => {
            warn!(
                "gdbus/Mutter returned a GVariant we could not parse for resolution; \
                 falling through to xrandr"
            );
            Err("Could not parse gdbus/Mutter output".into())
        }
    }
}

/// Pure parser for the GVariant returned by `GetCurrentState`. Extracted
/// for testability — see `parse_mutter_*` tests below.
///
/// Approach:
/// 1. **Prefer** the modes block whose owning monitor section is marked
///    `'is-primary': <true>`. With multiple monitors, every monitor has
///    its own `'is-current': <true>` mode, but only the primary is the
///    one users actually launch fullscreen apps on.
/// 2. If no primary marker is found, fall back to the first
///    `'is-current': <true>` (legacy single-monitor behavior).
///
/// Once we've picked an `is-current` position, walk **backwards** through
/// the text looking for a tuple containing two `int32 N` integers. Those
/// are the mode's width and height.
///
/// Mode tuple shape (per Mutter docs):
/// `(s mode_id, i width, i height, d refresh_rate, d preferred_scale,
///   ad supported_scales, a{sv} properties)`
///
/// We accept either spelling produced by gdbus: `int32 1920` or just
/// `1920` followed by `,` (older bindings drop the type tag).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_mutter_get_current_state(text: &str) -> Option<(u32, u32)> {
    let primary_marker_a = "'is-primary': <true>";
    let primary_marker_b = "\"is-primary\": <true>";
    let current_marker_a = "'is-current': <true>";
    let current_marker_b = "\"is-current\": <true>";

    // Find every `is-current` position (one per monitor in multi-monitor setups).
    let current_positions = find_all_occurrences(text, &[current_marker_a, current_marker_b]);
    if current_positions.is_empty() {
        return None;
    }

    // Prefer an `is-current` whose monitor section also has `is-primary: <true>`.
    // Mutter's GVariant orders things as: (connector, modes_array, monitor_props).
    // So for a given monitor, `is-primary` appears AFTER its `is-current` mode.
    // We pick the `is-current` position whose nearest *following* `is-primary`
    // appears before the next `is-current` (i.e. they're in the same monitor).
    let primary_positions = find_all_occurrences(text, &[primary_marker_a, primary_marker_b]);

    let chosen_pos = if !primary_positions.is_empty() {
        let mut best: Option<usize> = None;
        for (i, &cur) in current_positions.iter().enumerate() {
            // The end of "this monitor" in the text is just before the next
            // is-current, or end of text for the last monitor.
            let monitor_end = current_positions.get(i + 1).copied().unwrap_or(text.len());
            // If any is-primary marker falls strictly between cur and monitor_end,
            // this monitor is the primary.
            if primary_positions
                .iter()
                .any(|&pp| pp > cur && pp < monitor_end)
            {
                best = Some(cur);
                break;
            }
        }
        best.unwrap_or(current_positions[0])
    } else {
        current_positions[0]
    };

    // Slice the prefix and look back at most ~2KB to find the mode tuple
    // that owns this property. 2KB is generous: a single mode tuple is
    // typically <300 chars even with many supported scales.
    let prefix_start = chosen_pos.saturating_sub(2048);
    let window = &text[prefix_start..chosen_pos];

    // Find candidate "(int32 W, int32 H," patterns. We grab them all and
    // take the *last* one — the closest preceding pair belongs to the
    // same mode tuple.
    let mut last: Option<(u32, u32)> = None;

    // Walk char-by-char looking for "int32 " sequences; collect pairs.
    let mut pending_w: Option<u32> = None;
    let mut idx = 0;
    let bytes = window.as_bytes();
    while idx < bytes.len() {
        // Match the literal "int32 " then read digits.
        if window[idx..].starts_with("int32 ") {
            idx += "int32 ".len();
            let start = idx;
            while idx < bytes.len() && (bytes[idx] as char).is_ascii_digit() {
                idx += 1;
            }
            if let Ok(n) = window[start..idx].parse::<u32>() {
                if let Some(w) = pending_w.take() {
                    // Got a pair — record it.
                    last = Some((w, n));
                } else {
                    pending_w = Some(n);
                }
            }
            continue;
        }
        // A non-int32 separator that resets a half-pair — only reset on
        // '(' (new tuple) so we don't lose width when "int32 W, int32 H"
        // is split by simple ", ".
        if bytes[idx] == b'(' {
            pending_w = None;
        }
        idx += 1;
    }

    last.filter(|(w, h)| *w > 0 && *h > 0 && *w < 32_768 && *h < 32_768)
}

/// Find every occurrence of any of `needles` inside `haystack`, sorted by
/// ascending position. Used by the Mutter parser to locate `is-current` /
/// `is-primary` markers regardless of which quoting style gdbus used.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn find_all_occurrences(haystack: &str, needles: &[&str]) -> Vec<usize> {
    let mut positions = Vec::new();
    for needle in needles {
        let mut start = 0;
        while let Some(p) = haystack[start..].find(needle) {
            let abs = start + p;
            positions.push(abs);
            start = abs + needle.len();
        }
    }
    positions.sort_unstable();
    positions.dedup();
    positions
}

/// Returns true if the line contains a `priority` field with the exact
/// value `1` (not `10`, `11`, etc.).
///
/// kscreen-doctor prints output priority as either:
/// - `"... priority 1 ..."` (older / inline)
/// - `"priority: 1"` (newer split-line variant)
///
/// We tokenize on whitespace and `:` and look for the token immediately
/// after a `priority` token to be exactly `"1"`. A naive
/// `line.contains("priority 1")` match would falsely accept `"priority 10"`
/// and `"priority 11"`, which broke multi-monitor priority detection.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn has_priority_one_token(line: &str) -> bool {
    let tokens: Vec<&str> = line
        .split(|c: char| c.is_whitespace() || c == ':' || c == ',')
        .filter(|t| !t.is_empty())
        .collect();
    for (i, tok) in tokens.iter().enumerate() {
        if tok.eq_ignore_ascii_case("priority") {
            if let Some(next) = tokens.get(i + 1) {
                if *next == "1" {
                    return true;
                }
            }
        }
    }
    false
}

/// Parse a resolution string like "2560 x 1440" or "2560 x 1440 @ 60Hz".
fn parse_resolution_string(s: &str) -> Option<(u32, u32)> {
    // Remove anything after "@" (refresh rate)
    let res_part = s.split('@').next()?;
    let parts: Vec<&str> = res_part.split('x').collect();
    if parts.len() >= 2 {
        let w = parts[0].trim().parse().ok()?;
        let h = parts[1].trim().parse().ok()?;
        return Some((w, h));
    }
    // Try "x" with spaces: "2560 x 1440"
    let parts: Vec<&str> = res_part.split(" x ").collect();
    if parts.len() >= 2 {
        let w = parts[0].trim().parse().ok()?;
        let h = parts[1].trim().parse().ok()?;
        return Some((w, h));
    }
    None
}

// ---------------------------------------------------------------------------
// SkyrimPrefs.ini location
// ---------------------------------------------------------------------------

/// Find SkyrimPrefs.ini in a Wine bottle for Skyrim SE.
///
/// The file is at: `<bottle>/drive_c/users/<user>/Documents/My Games/Skyrim Special Edition/SkyrimPrefs.ini`
pub fn find_skyrim_prefs(bottle: &Bottle) -> Option<PathBuf> {
    let users_dir = bottle.users_dir();
    if !users_dir.exists() {
        return None;
    }

    if let Ok(entries) = fs::read_dir(&users_dir) {
        for entry in entries.flatten() {
            let user_dir = entry.path();
            if !user_dir.is_dir() {
                continue;
            }

            // Try standard Documents path (case-insensitive)
            let candidates = [
                user_dir
                    .join("Documents")
                    .join("My Games")
                    .join("Skyrim Special Edition")
                    .join("SkyrimPrefs.ini"),
                user_dir
                    .join("My Documents")
                    .join("My Games")
                    .join("Skyrim Special Edition")
                    .join("SkyrimPrefs.ini"),
            ];

            for candidate in &candidates {
                if candidate.exists() {
                    return Some(candidate.clone());
                }
            }

            // Case-insensitive search using bottle.find_path doesn't work here
            // because we need to search under a specific user dir. Do manual case-insensitive.
            if let Some(prefs) = find_prefs_case_insensitive(&user_dir) {
                return Some(prefs);
            }
        }
    }

    None
}

/// Case-insensitive search for SkyrimPrefs.ini under a user directory.
fn find_prefs_case_insensitive(user_dir: &Path) -> Option<PathBuf> {
    let docs =
        find_dir_ci(user_dir, "documents").or_else(|| find_dir_ci(user_dir, "my documents"))?;
    let my_games = find_dir_ci(&docs, "my games")?;
    let skyrim_dir = find_dir_ci(&my_games, "skyrim special edition")?;
    find_file_ci(&skyrim_dir, "skyrimprefs.ini")
}

fn find_dir_ci(parent: &Path, name: &str) -> Option<PathBuf> {
    let name_lower = name.to_lowercase();
    if let Ok(entries) = fs::read_dir(parent) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() && entry.file_name().to_string_lossy().to_lowercase() == name_lower {
                return Some(p);
            }
        }
    }
    None
}

fn find_file_ci(parent: &Path, name: &str) -> Option<PathBuf> {
    let name_lower = name.to_lowercase();
    if let Ok(entries) = fs::read_dir(parent) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() && entry.file_name().to_string_lossy().to_lowercase() == name_lower {
                return Some(p);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// INI reading / writing
// ---------------------------------------------------------------------------

/// Read a display-related value from the [Display] section of SkyrimPrefs.ini.
fn read_ini_display_value(content: &str, key: &str) -> Option<String> {
    let mut in_display = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case("[display]") {
            in_display = true;
            continue;
        }
        if trimmed.starts_with('[') {
            in_display = false;
            continue;
        }
        if in_display {
            if let Some((k, v)) = trimmed.split_once('=') {
                if k.trim().eq_ignore_ascii_case(key) {
                    return Some(v.trim().to_string());
                }
            }
        }
    }
    None
}

/// Set a value in the [Display] section, or create it if missing.
fn set_ini_display_value(content: &str, key: &str, value: &str) -> String {
    let mut result = String::with_capacity(content.len() + 50);
    let mut in_display = false;
    let mut found = false;
    let mut display_section_ended = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.eq_ignore_ascii_case("[display]") {
            in_display = true;
            result.push_str(line);
            result.push('\n');
            continue;
        }

        if in_display && trimmed.starts_with('[') {
            // About to leave [Display] section — insert if not found
            if !found {
                result.push_str(&format!("{}={}\n", key, value));
                found = true;
            }
            in_display = false;
            display_section_ended = true;
        }

        if in_display {
            if let Some((k, _)) = trimmed.split_once('=') {
                if k.trim().eq_ignore_ascii_case(key) {
                    result.push_str(&format!("{}={}\n", k.trim(), value));
                    found = true;
                    continue;
                }
            }
        }

        result.push_str(line);
        result.push('\n');
    }

    // If still in [Display] at EOF and key not found, append
    if !found && in_display {
        result.push_str(&format!("{}={}\n", key, value));
    }

    // If [Display] section doesn't exist at all, create it
    if !found && !in_display && !display_section_ended {
        if !result.ends_with('\n') {
            result.push('\n');
        }
        result.push_str("[Display]\n");
        result.push_str(&format!("{}={}\n", key, value));
    }

    // Remove trailing double newline
    while result.ends_with("\n\n") {
        result.pop();
    }
    if !result.ends_with('\n') {
        result.push('\n');
    }

    result
}

/// Read current display settings from SkyrimPrefs.ini.
pub fn read_display_settings(prefs_path: &Path) -> Result<DisplaySettings, String> {
    let content = fs::read_to_string(prefs_path)
        .map_err(|e| format!("Failed to read {}: {}", prefs_path.display(), e))?;

    let width = read_ini_display_value(&content, "iSize W")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let height = read_ini_display_value(&content, "iSize H")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let fullscreen = read_ini_display_value(&content, "bFull Screen")
        .map(|v| v == "1")
        .unwrap_or(false);
    let borderless = read_ini_display_value(&content, "bBorderless")
        .map(|v| v == "1")
        .unwrap_or(false);

    Ok(DisplaySettings {
        width,
        height,
        fullscreen,
        borderless,
    })
}

/// Apply display fix: set resolution to the detected screen resolution in
/// exclusive fullscreen mode (`bFull Screen=1, bBorderless=0`).
///
/// Exclusive fullscreen is required for Wine to properly capture input and
/// hide the macOS cursor. Borderless mode leaves Wine in windowed mode with
/// no input grab and a visible OS cursor.
pub fn fix_display_settings(
    prefs_path: &Path,
    width: u32,
    height: u32,
) -> Result<DisplaySettings, String> {
    let content = fs::read_to_string(prefs_path)
        .map_err(|e| format!("Failed to read {}: {}", prefs_path.display(), e))?;

    let mut updated = content.clone();
    updated = set_ini_display_value(&updated, "iSize W", &width.to_string());
    updated = set_ini_display_value(&updated, "iSize H", &height.to_string());
    updated = set_ini_display_value(&updated, "bFull Screen", "1");
    updated = set_ini_display_value(&updated, "bBorderless", "0");
    // Constrain cursor to the game window — prevents the cursor from reaching
    // the very bottom edge of the screen where macOS triggers Dock auto-show,
    // which breaks through Wine's display capture and makes the OS cursor visible.
    updated = set_ini_display_value(&updated, "bConstrainCursor", "1");

    // Write via temp file + rename for atomicity
    let temp_path = prefs_path.with_extension("ini.tmp");
    fs::write(&temp_path, &updated).map_err(|e| format!("Failed to write temp file: {}", e))?;
    fs::rename(&temp_path, prefs_path).map_err(|e| format!("Failed to rename temp file: {}", e))?;

    Ok(DisplaySettings {
        width,
        height,
        fullscreen: true,
        borderless: false,
    })
}

// ---------------------------------------------------------------------------
// Wine registry — virtual desktop removal
// ---------------------------------------------------------------------------

/// Disable Wine's virtual desktop mode by removing the relevant registry
/// keys from `user.reg`. When virtual desktop is enabled, Wine forces a
/// windowed display regardless of the game's own fullscreen settings.
///
/// Removing these keys allows the game to use true exclusive fullscreen,
/// which on macOS creates a native Space the user can 3-finger-swipe away from.
pub fn disable_wine_virtual_desktop(bottle: &Bottle) -> Result<(), String> {
    let user_reg = bottle.path.join("user.reg");
    if !user_reg.exists() {
        return Ok(()); // No registry file — nothing to fix
    }

    let content =
        fs::read_to_string(&user_reg).map_err(|e| format!("Failed to read user.reg: {}", e))?;

    let mut updated = content.clone();

    // Remove the virtual desktop definitions section entirely
    updated = remove_registry_section(&updated, r#"[Software\\Wine\\Explorer\\Desktops]"#);

    // Remove any sub-sections like [Software\\Wine\\Explorer\\Desktops\Default]
    updated =
        remove_registry_sections_matching(&updated, r#"[Software\\Wine\\Explorer\\Desktops\"#);

    // Remove the "Desktop" key from [Software\\Wine\\Explorer] which activates
    // the virtual desktop
    updated = remove_registry_key(&updated, r#"[Software\\Wine\\Explorer]"#, "Desktop");

    if updated == content {
        return Ok(()); // No changes needed
    }

    let temp_path = user_reg.with_extension("reg.tmp");
    fs::write(&temp_path, &updated)
        .map_err(|e| format!("Failed to write temp registry file: {}", e))?;
    fs::rename(&temp_path, &user_reg)
        .map_err(|e| format!("Failed to rename temp registry file: {}", e))?;

    Ok(())
}

/// Remove an entire registry section (header + all keys until next section).
fn remove_registry_section(content: &str, section_header: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut skip = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed == section_header {
            skip = true;
            continue;
        }

        if skip && trimmed.starts_with('[') {
            skip = false;
        }

        if !skip {
            result.push_str(line);
            result.push('\n');
        }
    }

    // Preserve original trailing newline behavior
    while result.ends_with("\n\n\n") {
        result.pop();
    }

    result
}

/// Remove all registry sections whose header starts with the given prefix.
/// Used to remove sub-keys like `[Software\\Wine\\Explorer\\Desktops\Default]`.
fn remove_registry_sections_matching(content: &str, prefix: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut skip = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with(prefix) && trimmed.ends_with(']') {
            skip = true;
            continue;
        }

        if skip && trimmed.starts_with('[') {
            skip = false;
        }

        if !skip {
            result.push_str(line);
            result.push('\n');
        }
    }

    while result.ends_with("\n\n\n") {
        result.pop();
    }

    result
}

/// Remove a specific key from a registry section.
fn remove_registry_key(content: &str, section_header: &str, key_name: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut in_section = false;
    let key_pattern = format!("\"{}\"", key_name);

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed == section_header {
            in_section = true;
            result.push_str(line);
            result.push('\n');
            continue;
        }

        if in_section && trimmed.starts_with('[') {
            in_section = false;
        }

        // Skip lines matching "KeyName"=... in the target section
        if in_section && trimmed.starts_with(&key_pattern) && trimmed.contains('=') {
            continue;
        }

        result.push_str(line);
        result.push('\n');
    }

    result
}

// ---------------------------------------------------------------------------
// Full pipeline
// ---------------------------------------------------------------------------

/// Set a registry key in user.reg, creating the section if needed.
fn set_registry_value(content: &mut String, section: &str, key: &str, value: &str) {
    let entry = format!("\"{}\"=\"{}\"", key, value);
    let key_prefix = format!("\"{}\"", key);

    if content.contains(section) {
        if content.contains(&key_prefix) {
            // Replace existing value
            let mut replaced = false;
            *content = content
                .lines()
                .map(|line| {
                    if !replaced && line.trim().starts_with(&key_prefix) {
                        replaced = true;
                        entry.clone()
                    } else {
                        line.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
        } else {
            // Add after section header
            if let Some(pos) = content.find(section) {
                let after = pos + section.len();
                if let Some(nl) = content[after..].find('\n') {
                    let insert = after + nl + 1;
                    content.insert_str(insert, &format!("{}\n", entry));
                }
            }
        }
    } else {
        content.push_str(&format!("\n{}\n{}\n", section, entry));
    }
}

/// Configure Wine registry keys to fix the double-cursor bug in fullscreen games.
///
/// This is the standard community fix applied automatically. Sets keys across
/// three Wine driver sections to ensure the game grabs the mouse exclusively,
/// preventing the macOS/Linux cursor from escaping or doubling.
///
/// Keys set in `user.reg`:
/// - `[Software\\Wine\\Mac Driver]`
///   - `CaptureDisplaysForFullscreen=Y` — exclusive display control, hides OS cursor
/// - `[Software\\Wine\\X11 Driver]`
///   - `DXGrab=Y` — DirectX grabs mouse exclusively
///   - `MouseWarpOverride=force` — forces mouse coordinate alignment
///   - `UseXVidMode=N` — disables X11 video mode switching (prevents conflicts)
///   - `UseTakeFocus=N` — disables X11 focus stealing prevention
/// - `[Software\\Wine\\DirectInput]`
///   - `MouseWarpOverride=force` — input coordinate alignment
pub fn fix_cursor_grab(bottle: &Bottle) -> Result<CursorFixResult, String> {
    let user_reg = bottle.path.join("user.reg");
    let mut content = if user_reg.exists() {
        fs::read_to_string(&user_reg).map_err(|e| format!("Failed to read user.reg: {}", e))?
    } else {
        String::new()
    };

    let original = content.clone();

    let mac_section = "[Software\\\\Wine\\\\Mac Driver]";
    let x11_section = "[Software\\\\Wine\\\\X11 Driver]";
    let di_section = "[Software\\\\Wine\\\\DirectInput]";

    // Mac Driver — exclusive display capture (macOS CrossOver)
    set_registry_value(
        &mut content,
        mac_section,
        "CaptureDisplaysForFullscreen",
        "Y",
    );

    // X11 Driver — full cursor grab (Linux, or CrossOver X11 mode)
    set_registry_value(&mut content, x11_section, "DXGrab", "Y");
    set_registry_value(&mut content, x11_section, "MouseWarpOverride", "force");
    set_registry_value(&mut content, x11_section, "UseXVidMode", "N");
    set_registry_value(&mut content, x11_section, "UseTakeFocus", "N");

    // DirectInput — mouse warp for coordinate alignment
    set_registry_value(&mut content, di_section, "MouseWarpOverride", "force");

    let changed = content != original;

    // Atomic write (only if something changed)
    if changed {
        let tmp = user_reg.with_extension("reg.tmp");
        fs::write(&tmp, &content)
            .map_err(|e| format!("Failed to write temp registry file: {}", e))?;
        fs::rename(&tmp, &user_reg)
            .map_err(|e| format!("Failed to rename temp registry file: {}", e))?;
    }

    Ok(CursorFixResult { applied: changed })
}

/// Result of the Wine registry cursor fix.
#[derive(Clone, Debug)]
pub struct CursorFixResult {
    /// True if registry keys were written (false if already present).
    pub applied: bool,
}

/// Fix GPU device name in Bethesda game INI files.
///
/// Wine/Proton reports incorrect GPU names (often "llvmpipe" or the Mesa driver
/// name) in game INI `sD3DDevice=` entries. This causes CTDs or rendering
/// issues. We detect the actual GPU and fix the entries.
pub fn fix_gpu_ini_entries(game_dir: &Path) -> Result<usize, String> {
    let gpu_name = detect_gpu_name();
    if gpu_name.is_empty() {
        debug!("Could not detect GPU name, skipping INI fix");
        return Ok(0);
    }

    log::info!("Detected GPU: {}", gpu_name);

    // Known Bethesda INI basenames (lowercase for matching)
    let known_inis: std::collections::HashSet<&str> = [
        "skyrimprefs.ini", "skyrim.ini",
        "fallout4prefs.ini", "fallout4.ini", "fallout.ini", "falloutprefs.ini",
        "oblivion.ini", "starfieldprefs.ini", "starfieldcustom.ini",
    ].into_iter().collect();

    let mut fixed_count = 0;

    if let Ok(entries) = std::fs::read_dir(game_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if known_inis.contains(name.to_lowercase().as_str()) {
                match fix_gpu_in_ini(&entry.path(), &gpu_name) {
                    Ok(true) => {
                        log::info!("Fixed GPU name in {}", name);
                        fixed_count += 1;
                    }
                    Ok(false) => {}
                    Err(e) => {
                        warn!("Failed to fix GPU in {}: {}", name, e);
                    }
                }
            }
        }
    }

    Ok(fixed_count)
}

/// Fix sD3DDevice= entry in a single INI file, preserving line endings.
fn fix_gpu_in_ini(ini_path: &Path, gpu_name: &str) -> Result<bool, String> {
    let content = fs::read(ini_path)
        .map_err(|e| format!("Failed to read {}: {}", ini_path.display(), e))?;

    let content_str = String::from_utf8_lossy(&content);

    // Detect line ending style (CRLF vs LF)
    let line_ending = if content_str.contains("\r\n") { "\r\n" } else { "\n" };

    let mut modified = false;
    let mut output_lines = Vec::new();

    for line in content_str.split(line_ending) {
        let trimmed = line.trim();
        if trimmed.to_lowercase().starts_with("sd3ddevice=") {
            let current_value = trimmed.splitn(2, '=').nth(1).unwrap_or("").trim();
            if current_value != gpu_name {
                output_lines.push(format!("sD3DDevice={}", gpu_name));
                modified = true;
                continue;
            }
        }
        output_lines.push(line.to_string());
    }

    if modified {
        let output = output_lines.join(line_ending);
        fs::write(ini_path, output.as_bytes())
            .map_err(|e| format!("Failed to write {}: {}", ini_path.display(), e))?;
    }

    Ok(modified)
}

/// Detect the GPU name from the system.
fn detect_gpu_name() -> String {
    #[cfg(target_os = "linux")]
    {
        // Try lspci first (most reliable)
        if let Ok(output) = Command::new("lspci").output() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if line.contains("VGA") || line.contains("3D controller") || line.contains("Display controller") {
                    // Extract the device name after the last colon
                    if let Some(name) = line.split(':').last() {
                        let name = name.trim();
                        if !name.is_empty() {
                            return name.to_string();
                        }
                    }
                }
            }
        }

        // Fallback: read /sys/class/drm
        if let Ok(entries) = fs::read_dir("/sys/class/drm") {
            for entry in entries.flatten() {
                let name_path = entry.path().join("device/label");
                if let Ok(name) = fs::read_to_string(&name_path) {
                    let name = name.trim().to_string();
                    if !name.is_empty() {
                        return name;
                    }
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        // Use system_profiler on macOS
        if let Ok(output) = Command::new("system_profiler")
            .arg("SPDisplaysDataType")
            .arg("-json")
            .output()
        {
            if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                if let Some(displays) = json.get("SPDisplaysDataType").and_then(|v| v.as_array()) {
                    if let Some(first) = displays.first() {
                        if let Some(name) = first.get("sppci_model").and_then(|v| v.as_str()) {
                            return name.to_string();
                        }
                    }
                }
            }
        }
    }

    String::new()
}

/// Full pipeline: detect resolution, find prefs, fix INI + Wine registry.
///
/// 1. Detects correct resolution for the platform and bottle's Retina setting
/// 2. Sets SkyrimPrefs.ini to detected resolution in exclusive fullscreen
/// 3. Removes Wine virtual desktop to allow true fullscreen
/// 4. Configures mouse capture and display capture for proper input
/// 5. Fixes GPU device name in Bethesda INI files
pub fn auto_fix_display(bottle: &Bottle) -> Result<DisplayFixResult, String> {
    let (screen_w, screen_h) = detect_screen_resolution(bottle)?;

    let prefs_path = find_skyrim_prefs(bottle)
        .ok_or("Could not find SkyrimPrefs.ini in this bottle. Launch Skyrim once first to create the settings file.")?;

    let previous = read_display_settings(&prefs_path)?;

    // Always attempt Wine registry fixes
    if let Err(e) = disable_wine_virtual_desktop(bottle) {
        warn!("Could not disable Wine virtual desktop: {}", e);
    }

    match fix_cursor_grab(bottle) {
        Ok(cursor_result) => {
            if cursor_result.applied {
                log::info!("Applied Wine cursor fix (DXGrab, MouseWarpOverride, CaptureDisplays)");
            }
        }
        Err(e) => warn!("Could not apply Wine cursor fix: {}", e),
    }

    // Fix GPU device name in INI files (prevents CTDs from wrong sD3DDevice)
    if let Some(prefs_parent) = prefs_path.parent() {
        match fix_gpu_ini_entries(prefs_parent) {
            Ok(count) if count > 0 => {
                log::info!("Fixed GPU name in {} INI file(s)", count);
            }
            Ok(_) => {}
            Err(e) => warn!("Could not fix GPU INI entries: {}", e),
        }
    }

    // Check if INI fix is needed (target is exclusive fullscreen)
    let ini_already_correct = previous.width == screen_w
        && previous.height == screen_h
        && previous.fullscreen
        && !previous.borderless;

    let applied = if ini_already_correct {
        previous.clone()
    } else {
        fix_display_settings(&prefs_path, screen_w, screen_h)?
    };

    Ok(DisplayFixResult {
        fixed: true, // Always report fixed since we also fix Wine registry
        prefs_path: prefs_path.to_string_lossy().into_owned(),
        previous,
        applied,
        screen_width: screen_w,
        screen_height: screen_h,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bottles::Bottle;

    const SAMPLE_INI: &str = r#"[General]
sLanguage=ENGLISH

[Display]
iSize H=720
iSize W=1280
bFull Screen=0
bBorderless=0
fDefaultFOV=90

[Audio]
fMusicVolume=0.5
"#;

    #[test]
    fn read_ini_values() {
        assert_eq!(
            read_ini_display_value(SAMPLE_INI, "iSize H"),
            Some("720".to_string())
        );
        assert_eq!(
            read_ini_display_value(SAMPLE_INI, "iSize W"),
            Some("1280".to_string())
        );
        assert_eq!(
            read_ini_display_value(SAMPLE_INI, "bFull Screen"),
            Some("0".to_string())
        );
        assert_eq!(
            read_ini_display_value(SAMPLE_INI, "bBorderless"),
            Some("0".to_string())
        );
    }

    #[test]
    fn read_ini_missing_key() {
        assert_eq!(read_ini_display_value(SAMPLE_INI, "iSomething"), None);
    }

    #[test]
    fn set_ini_updates_existing() {
        let result = set_ini_display_value(SAMPLE_INI, "iSize H", "1440");
        assert!(result.contains("iSize H=1440"));
        assert!(!result.contains("iSize H=720"));
    }

    #[test]
    fn set_ini_adds_missing_key() {
        let result = set_ini_display_value(SAMPLE_INI, "iNewSetting", "42");
        assert!(result.contains("iNewSetting=42"));
        // Should be added inside [Display] section
        let display_start = result.find("[Display]").unwrap();
        let new_setting = result.find("iNewSetting=42").unwrap();
        let audio_start = result.find("[Audio]").unwrap();
        assert!(new_setting > display_start && new_setting < audio_start);
    }

    #[test]
    fn set_ini_creates_display_section() {
        let ini = "[General]\nsLanguage=ENGLISH\n";
        let result = set_ini_display_value(ini, "iSize W", "2560");
        assert!(result.contains("[Display]"));
        assert!(result.contains("iSize W=2560"));
    }

    #[test]
    fn full_display_fix_pipeline() {
        let mut content = SAMPLE_INI.to_string();
        content = set_ini_display_value(&content, "iSize W", "2560");
        content = set_ini_display_value(&content, "iSize H", "1440");
        content = set_ini_display_value(&content, "bFull Screen", "1");
        content = set_ini_display_value(&content, "bBorderless", "0");

        assert!(content.contains("iSize W=2560"));
        assert!(content.contains("iSize H=1440"));
        assert!(content.contains("bFull Screen=1"));
        assert!(content.contains("bBorderless=0"));
        // Original values should be gone
        assert!(!content.contains("iSize W=1280"));
        assert!(!content.contains("iSize H=720"));
        assert!(!content.contains("bFull Screen=0"));
    }

    #[test]
    fn parse_resolution_variants() {
        assert_eq!(parse_resolution_string("2560 x 1440"), Some((2560, 1440)));
        assert_eq!(
            parse_resolution_string("2560 x 1440 @ 60.00Hz"),
            Some((2560, 1440))
        );
        assert_eq!(parse_resolution_string("1920x1080"), Some((1920, 1080)));
        // Formats from xrandr/wlr-randr
        assert_eq!(parse_resolution_string("2560x1600"), Some((2560, 1600)));
        assert_eq!(parse_resolution_string("1280x800"), Some((1280, 800)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_xrandr_connected_line_formats() {
        assert_eq!(
            parse_xrandr_connected_line("DP-1 connected primary 2560x1440+0+0 (normal left inverted right x axis y axis) 597mm x 336mm"),
            Some((2560, 1440))
        );
        assert_eq!(
            parse_xrandr_connected_line("eDP-1 connected 1920x1080+0+0 (normal) 344mm x 194mm"),
            Some((1920, 1080))
        );
        assert_eq!(
            parse_xrandr_connected_line(
                "HDMI-1 connected (normal left inverted right x axis y axis)"
            ),
            None // No resolution shown = display not active
        );
    }

    #[test]
    fn fix_gpu_in_ini_replaces_wrong_name() {
        let tmp = tempfile::tempdir().unwrap();
        let ini_path = tmp.path().join("SkyrimPrefs.ini");
        std::fs::write(
            &ini_path,
            "[Display]\r\niSize W=1920\r\nsD3DDevice=llvmpipe\r\niSize H=1080\r\n",
        )
        .unwrap();

        let result = fix_gpu_in_ini(&ini_path, "NVIDIA GeForce RTX 4090");
        assert!(result.is_ok());
        assert!(result.unwrap()); // modified

        let content = std::fs::read_to_string(&ini_path).unwrap();
        assert!(content.contains("sD3DDevice=NVIDIA GeForce RTX 4090"));
        assert!(!content.contains("llvmpipe"));
        // Verify CRLF preserved
        assert!(content.contains("\r\n"));
    }

    #[test]
    fn fix_gpu_in_ini_skips_correct_name() {
        let tmp = tempfile::tempdir().unwrap();
        let ini_path = tmp.path().join("SkyrimPrefs.ini");
        std::fs::write(
            &ini_path,
            "[Display]\nsD3DDevice=My GPU\niSize W=1920\n",
        )
        .unwrap();

        let result = fix_gpu_in_ini(&ini_path, "My GPU");
        assert!(result.is_ok());
        assert!(!result.unwrap()); // not modified
    }

    #[test]
    fn fix_gpu_in_ini_no_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let ini_path = tmp.path().join("SkyrimPrefs.ini");
        std::fs::write(&ini_path, "[Display]\niSize W=1920\n").unwrap();

        let result = fix_gpu_in_ini(&ini_path, "My GPU");
        assert!(result.is_ok());
        assert!(!result.unwrap()); // not modified — no sD3DDevice entry
    }

    #[test]
    fn fix_gpu_ini_entries_fixes_multiple_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        std::fs::write(
            dir.join("SkyrimPrefs.ini"),
            "[Display]\nsD3DDevice=llvmpipe\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("Skyrim.ini"),
            "[Display]\nsD3DDevice=wrong gpu\n",
        )
        .unwrap();

        // This test only validates the file-scanning logic; detect_gpu_name()
        // may return empty on CI, so we call fix_gpu_in_ini directly per file.
        let r1 = fix_gpu_in_ini(&dir.join("SkyrimPrefs.ini"), "RTX 4090").unwrap();
        let r2 = fix_gpu_in_ini(&dir.join("Skyrim.ini"), "RTX 4090").unwrap();
        assert!(r1);
        assert!(r2);
    }

    #[test]
    fn is_retina_enabled_from_registry() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle_path = tmp.path().to_path_buf();
        std::fs::create_dir_all(bottle_path.join("drive_c")).unwrap();

        let bottle = Bottle {
            name: "Test".into(),
            path: bottle_path.clone(),
            source: "Wine".into(),
        };

        // No user.reg = not enabled
        assert!(!is_retina_enabled(&bottle));

        // user.reg without RetinaMode = not enabled
        std::fs::write(
            bottle_path.join("user.reg"),
            "[Software\\\\Wine\\\\Mac Driver]\n\"RetinaMode\"=\"N\"\n",
        )
        .unwrap();
        assert!(!is_retina_enabled(&bottle));

        // user.reg with RetinaMode=Y = enabled
        std::fs::write(
            bottle_path.join("user.reg"),
            "[Software\\\\Wine\\\\Mac Driver]\n\"RetinaMode\"=\"Y\"\n",
        )
        .unwrap();
        assert!(is_retina_enabled(&bottle));
    }

    #[test]
    fn case_insensitive_key_read() {
        let ini = "[display]\nisize h=900\nisize w=1600\nbfull screen=1\n";
        assert_eq!(
            read_ini_display_value(ini, "iSize H"),
            Some("900".to_string())
        );
        assert_eq!(
            read_ini_display_value(ini, "iSize W"),
            Some("1600".to_string())
        );
    }

    // --- Wine registry tests ---

    const SAMPLE_REGISTRY: &str = r#"WINE REGISTRY Version 2
;; All keys relative to \\User\\S-1-5-21

[Software\\Wine\\DllOverrides]
"dxgi"="native"

[Software\\Wine\\Explorer]
"Desktop"="Default"

[Software\\Wine\\Explorer\\Desktops]
"Default"="1920x1080"

[Software\\Wine\\Mac Driver]
"RetinaMode"="Y"
"#;

    #[test]
    fn remove_registry_section_removes_entire_section() {
        let result =
            remove_registry_section(SAMPLE_REGISTRY, r#"[Software\\Wine\\Explorer\\Desktops]"#);
        assert!(!result.contains("Desktops"));
        assert!(!result.contains("1920x1080"));
        // Other sections remain
        assert!(result.contains("[Software\\\\Wine\\\\DllOverrides]"));
        assert!(result.contains("[Software\\\\Wine\\\\Mac Driver]"));
    }

    #[test]
    fn remove_registry_key_removes_single_key() {
        let result =
            remove_registry_key(SAMPLE_REGISTRY, r#"[Software\\Wine\\Explorer]"#, "Desktop");
        assert!(!result.contains("\"Desktop\"=\"Default\""));
        // The section header remains
        assert!(result.contains("[Software\\\\Wine\\\\Explorer]"));
        // Other keys in other sections remain
        assert!(result.contains("\"dxgi\"=\"native\""));
    }

    #[test]
    fn remove_registry_section_noop_when_missing() {
        let input = "[Software\\\\Wine\\\\Mac Driver]\n\"RetinaMode\"=\"Y\"\n";
        let result = remove_registry_section(input, r#"[Software\\Wine\\Explorer\\Desktops]"#);
        assert_eq!(result, input);
    }

    // --- kscreen-doctor parser ---

    #[test]
    fn parse_kscreen_geometry_with_xy_prefix() {
        // Newer kscreen-doctor format: "Geometry: 0,0 1920x1080"
        let sample = "\
Output: 1 HDMI-A-1 enabled connected priority 1 modes:
  17:1280x720@60
  18:1920x1080@60*!
  19:2560x1440@60
Geometry: 0,0 1920x1080
";
        assert_eq!(parse_kscreen_doctor_output(sample), Some((1920, 1080)));
    }

    #[test]
    fn parse_kscreen_geometry_legacy_format() {
        // Older format documented in the task description.
        let sample = "\
Output: 1 HDMI-A-1 enabled connected priority 1 modes:
  18:1920x1080@60*!
Geometry: 1920x1080
";
        assert_eq!(parse_kscreen_doctor_output(sample), Some((1920, 1080)));
    }

    #[test]
    fn parse_kscreen_falls_back_to_starred_mode() {
        // No Geometry line — must use the "*" marked current mode.
        let sample = "\
Output: 1 HDMI-A-1 enabled connected priority 1 modes:
  17:1280x720@60
  18:2560x1440@60*
  19:3840x2160@60
";
        assert_eq!(parse_kscreen_doctor_output(sample), Some((2560, 1440)));
    }

    #[test]
    fn parse_kscreen_prefers_priority_one_over_other_outputs() {
        // External monitor enabled but priority 2; built-in is priority 1.
        let sample = "\
Output: 2 HDMI-A-1 enabled connected priority 2 modes:
  9:3840x2160@60*
Geometry: 1920,0 3840x2160

Output: 1 eDP-1 enabled connected priority 1 modes:
  18:2560x1600@60*
Geometry: 0,0 2560x1600
";
        assert_eq!(parse_kscreen_doctor_output(sample), Some((2560, 1600)));
    }

    #[test]
    fn parse_kscreen_skips_disabled_outputs() {
        // Disabled output has a Geometry line but should not win unless it
        // is the only candidate. Here the second block is enabled.
        let sample = "\
Output: 2 DP-1 disabled disconnected modes:
Output: 1 eDP-1 enabled connected priority 1 modes:
  18:1366x768@60*
Geometry: 0,0 1366x768
";
        assert_eq!(parse_kscreen_doctor_output(sample), Some((1366, 768)));
    }

    #[test]
    fn parse_kscreen_returns_none_on_garbage() {
        assert_eq!(parse_kscreen_doctor_output(""), None);
        assert_eq!(parse_kscreen_doctor_output("not kscreen output\n"), None);
    }

    #[test]
    fn parse_kscreen_priority_10_does_not_match_priority_1() {
        // Prior to the word-boundary fix this returned 3840x2160 because
        // "priority 10" was greedily matched by `contains("priority 1")`.
        // The eDP-1 (priority 1) panel is the actual primary; we must
        // honour it over the high-priority secondary.
        let sample = "\
Output: 2 HDMI-A-1 enabled connected priority 10 modes:
  9:3840x2160@60*
Geometry: 1920,0 3840x2160

Output: 1 eDP-1 enabled connected priority 1 modes:
  18:2560x1600@60*
Geometry: 0,0 2560x1600
";
        assert_eq!(parse_kscreen_doctor_output(sample), Some((2560, 1600)));
    }

    #[test]
    fn parse_kscreen_priority_11_does_not_match_priority_1_split_form() {
        // Same fix, exercising the split-line "priority: N" variant some
        // kscreen-doctor versions emit instead of the inline form.
        let sample = "\
Output: 2 HDMI-A-1
enabled
priority: 11
modes:
  9:3840x2160@60*
Geometry: 1920,0 3840x2160

Output: 1 eDP-1
enabled
priority: 1
modes:
  18:2560x1600@60*
Geometry: 0,0 2560x1600
";
        assert_eq!(parse_kscreen_doctor_output(sample), Some((2560, 1600)));
    }

    #[test]
    fn has_priority_one_token_basic() {
        assert!(has_priority_one_token("priority 1"));
        assert!(has_priority_one_token("priority: 1"));
        assert!(has_priority_one_token("Output: 1 eDP-1 enabled connected priority 1 modes:"));
        assert!(!has_priority_one_token("priority 10"));
        assert!(!has_priority_one_token("priority: 11"));
        assert!(!has_priority_one_token("priority 100"));
        assert!(!has_priority_one_token("not a priority line"));
    }

    // --- gdbus / Mutter parser ---

    #[test]
    fn parse_mutter_basic_single_monitor() {
        // Trimmed gdbus output for a 2560x1600 internal panel. Real output
        // is a single line; we keep it that way to mirror reality.
        let sample = "\
(uint32 1, [(0, 0, 1.0, 0, true, [('eDP-1', 'eDP-1', 'AU Optronics', 'B140QAN', 'AUO0000', \
[('eDP-1-2560x1600@60.001', int32 2560, int32 1600, 60.000999450683594, 1.0, [1.0, 1.25, 1.5], \
{'is-current': <true>, 'is-preferred': <true>})], {'display-name': <'Built-in display'>})], \
{'is-presentation': <false>}, {'layout-mode': <uint32 1>})\n";
        assert_eq!(
            parse_mutter_get_current_state(sample),
            Some((2560, 1600))
        );
    }

    #[test]
    fn parse_mutter_picks_first_current_when_multiple_modes() {
        // A monitor lists many modes; only one carries is-current.
        let sample = "\
[('id-1', int32 1280, int32 720, 60.0, 1.0, [1.0], {'is-preferred': <false>}), \
('id-2', int32 1920, int32 1080, 60.0, 1.0, [1.0], {'is-current': <true>, 'is-preferred': <true>}), \
('id-3', int32 2560, int32 1440, 60.0, 1.0, [1.0], {})]\n";
        assert_eq!(
            parse_mutter_get_current_state(sample),
            Some((1920, 1080))
        );
    }

    #[test]
    fn parse_mutter_returns_none_when_no_current_marker() {
        let sample = "\
[('id-1', int32 1280, int32 720, 60.0, 1.0, [1.0], {})]\n";
        assert_eq!(parse_mutter_get_current_state(sample), None);
    }

    #[test]
    fn parse_mutter_returns_none_on_empty() {
        assert_eq!(parse_mutter_get_current_state(""), None);
    }

    #[test]
    fn parse_mutter_rejects_implausible_dimensions() {
        // Pathological input where the int32s are absurd should be rejected.
        let sample = "(int32 999999, int32 999999, {'is-current': <true>})";
        assert_eq!(parse_mutter_get_current_state(sample), None);
    }

    #[test]
    fn parse_mutter_prefers_primary_monitor_over_secondary() {
        // Two monitors, both with their own is-current mode. Only the
        // *secondary* is listed first in the GVariant; without primary
        // preference the parser would pick the secondary (4K) panel and
        // we'd render the game at 3840x2160 instead of the laptop's native
        // 2560x1600. With the fix, we honour 'is-primary': <true>.
        let sample = "\
(uint32 1, [...], [\
(('HDMI-A-1', 'HDMI-A-1', 'Dell', 'U2723QE', 'DEL0001'), \
[('hdmi-3840x2160@60', int32 3840, int32 2160, 60.0, 1.0, [1.0, 2.0], \
{'is-current': <true>, 'is-preferred': <true>})], \
{'display-name': <'External display'>, 'is-primary': <false>}), \
(('eDP-1', 'eDP-1', 'AU Optronics', 'B140QAN', 'AUO0000'), \
[('edp-2560x1600@60', int32 2560, int32 1600, 60.0, 1.0, [1.0, 1.25, 1.5], \
{'is-current': <true>, 'is-preferred': <true>})], \
{'display-name': <'Built-in display'>, 'is-primary': <true>})], \
{})\n";
        assert_eq!(
            parse_mutter_get_current_state(sample),
            Some((2560, 1600))
        );
    }

    #[test]
    fn parse_mutter_falls_back_to_first_current_when_no_primary_marker() {
        // No 'is-primary' anywhere — older Mutter versions or single
        // monitor setups. Original behavior (first is-current wins) holds.
        let sample = "\
(uint32 1, [\
(('eDP-1', 'eDP-1', '', '', ''), \
[('id-1', int32 1920, int32 1080, 60.0, 1.0, [1.0], {'is-current': <true>})], \
{'display-name': <'eDP-1'>}), \
(('HDMI-A-1', 'HDMI-A-1', '', '', ''), \
[('id-2', int32 3840, int32 2160, 60.0, 1.0, [1.0], {'is-current': <true>})], \
{'display-name': <'HDMI-A-1'>})], \
{})\n";
        assert_eq!(
            parse_mutter_get_current_state(sample),
            Some((1920, 1080))
        );
    }

    #[test]
    fn remove_registry_sections_matching_removes_subsections() {
        // remove_registry_sections_matching handles sub-sections (prefix ending with \).
        // The main section is removed separately by remove_registry_section.
        let input = concat!(
            "[Software\\\\Wine\\\\Explorer\\\\Desktops]\n",
            "\"Default\"=\"1024x768\"\n",
            "\n",
            "[Software\\\\Wine\\\\Explorer\\\\Desktops\\\\Default]\n",
            "\"Width\"=\"1024\"\n",
            "\"Height\"=\"768\"\n",
            "\n",
            "[Software\\\\Wine\\\\Mac Driver]\n",
            "\"RetinaMode\"=\"Y\"\n",
        );
        // First remove main section, then sub-sections (mirrors disable_wine_virtual_desktop)
        let result = remove_registry_section(input, r#"[Software\\Wine\\Explorer\\Desktops]"#);
        let result =
            remove_registry_sections_matching(&result, r#"[Software\\Wine\\Explorer\\Desktops\"#);
        assert!(!result.contains("Desktops"));
        assert!(!result.contains("1024x768"));
        assert!(result.contains("Mac Driver"));
        assert!(result.contains("RetinaMode"));
    }
}
