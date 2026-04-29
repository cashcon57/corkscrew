/**
 * Native Mode theme toggle. Sets body[data-theme] to "native" when on,
 * reverts to the prior theme (or "dark" default) when off.
 *
 * Hydration: read native_mode flag from backend on app start (Task 5.5
 * wires this in the layout). When the user toggles native mode at
 * runtime, call applyNativeTheme(true|false) — DOM updates immediately
 * and the Liquid Glass variant ratchets accordingly.
 *
 * applyNativeTheme is async because it calls the backend to set the
 * window effect variant. Failures are non-fatal (console.warn) — the
 * theme switch always completes even if the effect call fails (e.g. on
 * Linux or an older macOS that hasn't crashed yet).
 */

import { applyNativeWindowEffect } from "../api";

const PRIOR_THEME_KEY = "corkscrew:prior-theme";

export async function applyNativeTheme(enabled: boolean): Promise<void> {
  if (typeof document === "undefined") return;
  const body = document.body;

  if (enabled) {
    // Capture the existing theme so we can restore it on toggle-off.
    const current = body.dataset.theme ?? "dark";
    if (current !== "native") {
      try {
        localStorage.setItem(PRIOR_THEME_KEY, current);
      } catch {
        // localStorage may be unavailable; ignore.
      }
    }
    body.dataset.theme = "native";
    // Ratchet glass to Inspector — the deepest variant per spike.
    try {
      await applyNativeWindowEffect("high");
    } catch (err) {
      console.warn("apply_native_window_effect failed (non-fatal):", err);
    }
  } else {
    let prior = "dark";
    try {
      prior = localStorage.getItem(PRIOR_THEME_KEY) ?? "dark";
    } catch {
      // ignore
    }
    body.dataset.theme = prior;
    // Restore default Regular glass variant.
    try {
      await applyNativeWindowEffect("default");
    } catch (err) {
      console.warn("apply_native_window_effect failed (non-fatal):", err);
    }
  }
}

export function isNativeThemeApplied(): boolean {
  if (typeof document === "undefined") return false;
  return document.body.dataset.theme === "native";
}
