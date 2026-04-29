# tauri-plugin-liquid-glass API Spec (Task 5.0 Research)

For: Task 5.4 — Runtime intensity ratchet / vibrancy toggle
Date: 2026-04-29
Crate version in use: `tauri-plugin-liquid-glass = "0.1"` (latest: 0.1.6)

---

## 1. Plugin Scope

`tauri-plugin-liquid-glass` adds macOS Liquid Glass effects (Apple's `NSGlassEffectView` private API) to
Tauri v2 windows. On macOS 26 (Tahoe) it uses `NSGlassEffectView`; on macOS 10.10–25 it falls back to
`NSVisualEffectView`. On Windows and Linux it compiles to safe no-ops — no `#[cfg(target_os = "macos")]`
guard required at the call site.

**Platforms:** macOS 10.10+ (Liquid Glass only on macOS 26+), Windows (no-op), Linux (no-op).

**macOS version requirements:**
- macOS 26 (Tahoe): Full Liquid Glass via `NSGlassEffectView` (all 24 `GlassMaterialVariant` values)
- macOS 10.10–25: Automatic fallback to `NSVisualEffectView` (standard vibrancy)

---

## 2. Initialization API

**Rust side** — one line added to the builder:

```rust
tauri::Builder::default()
    .plugin(tauri_plugin_liquid_glass::init())
```

No parameters to `init()`. Effect configuration happens entirely through JS commands at runtime (see
Section 3), not at init time.

**Required `tauri.conf.json` config:**

```json
{
  "tauri": {
    "macOSPrivateApi": true,
    "windows": [{ "transparent": true }]
  }
}
```

**Required capability entry** (in the relevant capability file):

```json
{ "identifier": "liquid-glass:default" }
```

This is **per-window** via the JS API — the Rust-side init is global (app-level), but effects are applied
to the calling window via the JS commands.

---

## 3. JS-Side API

The plugin exposes a JS package: `tauri-plugin-liquid-glass-api`. It provides **two callable commands**:

### `isGlassSupported(): Promise<boolean>`

Returns `true` when running on macOS 26+ with `NSGlassEffectView` available. Use this to gate the
Liquid Glass code path vs the fallback `window-vibrancy` path.

### `setLiquidGlassEffect(config?: LiquidGlassConfig): Promise<void>`

Applies, updates, or removes the glass effect on the **current window** at runtime. Config is optional
and all fields default gracefully.

```typescript
interface LiquidGlassConfig {
  enabled?: boolean;           // false = remove effect from window (default: true)
  cornerRadius?: number;       // pixels, default 0
  tintColor?: string;          // "#RRGGBB" or "#RRGGBBAA", default none
  variant?: GlassMaterialVariant; // see Section 4, default: Regular (0)
}
```

**This is a runtime toggle.** Calling `setLiquidGlassEffect({ enabled: false })` removes the effect
from a live window. Calling it again with `enabled: true` re-applies it. No window restart required.

---

## 4. Material / Intensity Options

`GlassMaterialVariant` is a numeric enum with 24 values (macOS 26+ only; silently ignored on older macOS
where `NSVisualEffectView` fallback handles the effect):

| Variant | Value | Notes |
|---|---|---|
| Regular | 0 | Default |
| Clear | 1 | |
| Dock | 2 | |
| AppIcons | 3 | |
| Widgets | 4 | |
| Text | 5 | |
| Avplayer | 6 | |
| Facetime | 7 | |
| ControlCenter | 8 | |
| NotificationCenter | 9 | |
| Monogram | 10 | |
| Bubbles | 11 | |
| Identity | 12 | |
| FocusBorder | 13 | |
| FocusPlatter | 14 | |
| Keyboard | 15 | |
| Sidebar | 16 | Best match for a sidebar/panel |
| AbuttedSidebar | 17 | |
| Inspector | 18 | Lighter — good for inspector panels |
| Control | 19 | |
| Loupe | 20 | |
| Slider | 21 | |
| Camera | 22 | |
| CartouchePopover | 23 | |

Additional knobs: `cornerRadius` (px) and `tintColor` (hex with optional alpha). No custom blur-radius
parameter — that is controlled internally by the variant.

---

## 5. Tauri 2 Compatibility

Confirmed. The crate's `Cargo.toml` declares `tauri = { version = "2.0", default-features = false }`.
Package version 0.1.6 requires Rust 1.77+ and targets `cdylib` + `rlib`. The existing
`Cargo.toml` in Corkscrew-native-mode uses `tauri = { version = "2", features = ["macos-private-api",
"unstable"] }`, which satisfies this. `macOSPrivateApi` (already enabled in the feature list) is
exactly the flag the plugin requires.

---

## 6. Runtime Toggle Capability

**The plugin IS runtime-capable.** `setLiquidGlassEffect()` is a Tauri command handler that operates on
the calling window at the time of invocation — it is not init-only. The `enabled` field on
`LiquidGlassConfig` acts as an explicit on/off switch. You can call it from any Svelte event handler
without restarting the window:

```typescript
// Turn on
await setLiquidGlassEffect({ variant: GlassMaterialVariant.Sidebar });

// Turn off
await setLiquidGlassEffect({ enabled: false });

// Intensity ratchet: swap variant based on a 0-100 intensity value
await setLiquidGlassEffect({
  variant: intensity > 66
    ? GlassMaterialVariant.Regular
    : intensity > 33
      ? GlassMaterialVariant.Sidebar
      : GlassMaterialVariant.Inspector,
});
```

---

## 7. Fallback Strategy if Init-Only

Not needed — the plugin is runtime-capable. However, `window-vibrancy = "0.7"` is also present in
`Cargo.toml` and provides a useful fallback for the macOS 10.10–25 case where the Liquid Glass
NSGlassEffectView is unavailable and you need explicit control over the `NSVisualEffectView` material.

**`window-vibrancy` `apply_vibrancy` signature:**

```rust
pub fn apply_vibrancy<W: raw_window_handle::HasRawWindowHandle>(
    window: &W,
    effect: NSVisualEffectMaterial,
    state: Option<NSVisualEffectState>,
    radius: Option<f64>,
) -> Result<(), Error>
```

**`NSVisualEffectMaterial` variants** (macOS 10.10+ unless noted):

| Variant | macOS |
|---|---|
| Titlebar, Selection | 10.10+ |
| Menu, Popover, Sidebar | 10.11+ |
| HeaderView, Sheet, WindowBackground, HudWindow, FullScreenUI, Tooltip, ContentBackground, UnderWindowBackground, UnderPageBackground | 10.14+ |
| AppearanceBased, Light, Dark, MediumLight, UltraDark | deprecated 10.14 |

`apply_vibrancy` CAN be called on a live window — it updates the effect in place. No restart required.

**When to use `window-vibrancy` directly:** Use it via a custom Tauri command as the fallback path on
macOS 10.10–25 if you need a specific `NSVisualEffectMaterial` rather than the plugin's automatic
fallback selection.

---

## 8. Recommendation for Task 5.4

**Recommendation A** — use the plugin's runtime JS commands directly.

`setLiquidGlassEffect()` is callable from Svelte at runtime with no window restart. The plugin already
handles the macOS 26 vs older macOS fork internally. Use `isGlassSupported()` to branch:

- macOS 26+: call `setLiquidGlassEffect()` with an appropriate `GlassMaterialVariant`
- macOS 10.10–25: the plugin auto-falls back to `NSVisualEffectView`; optionally supplement with a
  direct `window-vibrancy` Tauri command if you need a specific material

This avoids duplicating the macOS version detection logic in Corkscrew and keeps the glass toggle as a
pure JS call with no custom Rust command required for the happy path.

---

## 9. Concrete Code Sketch for Task 5.4

### Svelte side (stores.ts / NativeMode component)

```typescript
import {
  isGlassSupported,
  setLiquidGlassEffect,
  GlassMaterialVariant,
} from "tauri-plugin-liquid-glass-api";

// Called when native mode is toggled or intensity slider changes
export async function applyGlassIntensity(intensity: number): Promise<void> {
  // intensity: 0 = off, 1-33 = subtle, 34-66 = moderate, 67-100 = full
  if (intensity === 0) {
    await setLiquidGlassEffect({ enabled: false });
    return;
  }

  const glassAvailable = await isGlassSupported(); // true only on macOS 26+

  if (glassAvailable) {
    const variant =
      intensity > 66
        ? GlassMaterialVariant.Regular   // full Liquid Glass
        : intensity > 33
          ? GlassMaterialVariant.Sidebar // mid — sidebar-weight glass
          : GlassMaterialVariant.Inspector; // subtle — lighter glass

    await setLiquidGlassEffect({ variant, cornerRadius: 0 });
  } else {
    // macOS 10.10-25: plugin falls back to NSVisualEffectView automatically.
    // Call apply_vibrancy via custom command if you need a specific material.
    await setLiquidGlassEffect({ enabled: true }); // uses plugin's auto-fallback
    // Or: await invoke("set_native_vibrancy", { intensity });
  }
}
```

### Rust side — optional custom command for pre-Tahoe material control

Only needed if the automatic `NSVisualEffectView` fallback in the plugin is insufficient and you want
to select a specific material on macOS 10.10–25:

```rust
#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn set_native_vibrancy(
    window: tauri::Window,
    intensity: u8,
) -> Result<(), String> {
    use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState};
    let material = if intensity > 66 {
        NSVisualEffectMaterial::Sidebar
    } else if intensity > 33 {
        NSVisualEffectMaterial::HudWindow
    } else {
        NSVisualEffectMaterial::UnderWindowBackground
    };
    apply_vibrancy(&window, material, Some(NSVisualEffectState::Active), None)
        .map_err(|e| e.to_string())
}

// No-op stub for Linux/Windows so register_commands() compiles cross-platform
#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub async fn set_native_vibrancy(_window: tauri::Window, _intensity: u8) -> Result<(), String> {
    Ok(())
}
```

### Registration (lib.rs)

```rust
tauri::Builder::default()
    .plugin(tauri_plugin_liquid_glass::init())
    .invoke_handler(tauri::generate_handler![
        // ... existing handlers ...
        set_native_vibrancy,
    ])
```

**Summary for Task 5.4:** The `setLiquidGlassEffect()` JS command is the primary runtime ratchet on
macOS 26+. The custom `set_native_vibrancy` Tauri command is an optional supplement for explicit
material control on macOS 10.10–25. No "restart to apply" UX is needed.
