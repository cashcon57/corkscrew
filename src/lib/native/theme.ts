/**
 * Native Mode theme toggle. Sets body[data-theme] to "native" when on,
 * reverts to the prior theme (or "dark" default) when off.
 *
 * Hydration: read native_mode flag from backend on app start (Task 5.5
 * wires this in the layout). When the user toggles native mode at
 * runtime, call applyNativeTheme(true|false) — DOM updates immediately.
 */

const PRIOR_THEME_KEY = "corkscrew:prior-theme";

export function applyNativeTheme(enabled: boolean): void {
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
  } else {
    let prior = "dark";
    try {
      prior = localStorage.getItem(PRIOR_THEME_KEY) ?? "dark";
    } catch {
      // ignore
    }
    body.dataset.theme = prior;
  }
}

export function isNativeThemeApplied(): boolean {
  if (typeof document === "undefined") return false;
  return document.body.dataset.theme === "native";
}
