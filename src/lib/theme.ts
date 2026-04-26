import { writable, derived } from "svelte/store";
import { getConfig, setConfigValue } from "./api";

/* ============================================================================
 * Vibrancy CSS Architecture (decision log — v0.13.x)
 *
 * Roughly 27 Svelte components use `backdrop-filter` for the macOS vibrancy /
 * "glass" look. The cross-platform gating pattern is:
 *
 *   .my-card {
 *     background: color-mix(in srgb, var(--bg-secondary) 75%, transparent);
 *     backdrop-filter: var(--glass-blur-heavy);
 *     -webkit-backdrop-filter: var(--glass-blur-heavy);
 *   }
 *   :global(html:not(.vibrancy-active)) .my-card {
 *     background: var(--bg-secondary);          // opaque fallback
 *     backdrop-filter: none;
 *     -webkit-backdrop-filter: none;
 *   }
 *
 * The `vibrancy-active` class is set on <html> in `initTheme()` below ONLY on
 * macOS, where the Tauri window has a real NSVisualEffectView behind the web
 * content. On Linux/Windows we fall through to the opaque branch because
 * there is no compositor-backed blur to refract.
 *
 * Why we DON'T strip the vibrancy rules from non-macOS builds:
 *
 *   1. Bundle impact is trivial. The total weight of all backdrop-filter
 *      declarations across the app is ~5 KB pre-gzip — well under noise on
 *      a Tauri webview that already loads several hundred KB of Svelte +
 *      app code. Not worth a refactor.
 *
 *   2. The negation guard MAKES the gating work. Removing
 *      `:global(html:not(.vibrancy-active))` rules would either (a) leak
 *      the translucent macOS look onto Linux (broken — there's no blur
 *      behind the window so you'd just see semi-transparent garbage) or
 *      (b) require inverting every rule into a positive
 *      `html.vibrancy-active` form across all 27 files. Both directions
 *      already exist in `+layout.svelte` for cases where vibrancy needs
 *      to opt INTO transparency (e.g. `.app-shell`, `.content-column`),
 *      so a full inversion would mean auditing each rule for the correct
 *      polarity. High mechanical-edit risk for negligible byte savings.
 *
 *   3. Maintenance cost. The current pattern is consistent and grep-able.
 *      A custom-property-based approach (e.g. `--vibrancy-fallback-bg`)
 *      can't express both the background swap AND the `backdrop-filter:
 *      none` toggle in a single variable; you'd still need per-component
 *      overrides, just spread across more abstractions.
 *
 * If the Linux bundle ever needs aggressive trimming, prefer build-time
 * stripping via PostCSS rather than re-architecting the runtime CSS.
 * ========================================================================= */

export type ThemePreference = "system" | "light" | "dark";
export type ResolvedTheme = "light" | "dark";

/** User's chosen theme preference: "system", "light", or "dark". */
export const themePreference = writable<ThemePreference>("system");

/** The OS-level color scheme detected via matchMedia. */
export const systemTheme = writable<ResolvedTheme>("dark");

/**
 * Whether macOS vibrancy effects are available. Mirrors the presence of the
 * `vibrancy-active` class on <html>. See the architecture comment at the top
 * of this file for how component CSS gates on this.
 */
export const vibrancyAvailable = writable<boolean>(false);

/**
 * The actual theme in use. When themePreference is "system",
 * this resolves to whatever the OS reports.
 */
export const resolvedTheme = derived<
  [typeof themePreference, typeof systemTheme],
  ResolvedTheme
>([themePreference, systemTheme], ([$pref, $sys]) => {
  if ($pref === "system") return $sys;
  return $pref;
});

/**
 * Initialize the theme system.
 *
 * 1. Detects macOS via navigator and enables vibrancy class.
 * 2. Reads the OS color scheme with matchMedia and listens for changes.
 * 3. Loads any saved preference from the app config.
 * 4. Subscribes to resolvedTheme to keep the `data-theme` attribute in sync.
 */
export async function initTheme(): Promise<void> {
  // 1. Detect macOS for vibrancy support
  const isMacOS =
    typeof navigator !== "undefined" &&
    /Macintosh|Mac OS X/i.test(navigator.userAgent);

  if (isMacOS) {
    vibrancyAvailable.set(true);
    document.documentElement.classList.add("vibrancy-active");
  }

  // 2. Detect system color scheme via matchMedia
  if (typeof window !== "undefined" && window.matchMedia) {
    const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");

    // Set initial value
    systemTheme.set(darkQuery.matches ? "dark" : "light");

    // Listen for OS-level theme changes
    darkQuery.addEventListener("change", (e: MediaQueryListEvent) => {
      systemTheme.set(e.matches ? "dark" : "light");
    });
  }

  // 3. Load saved preference from config
  try {
    const cfg = await getConfig();
    const saved = (cfg as Record<string, unknown>)["theme"];
    if (saved === "light" || saved === "dark" || saved === "system") {
      themePreference.set(saved);
    }
  } catch {
    // Config unavailable — keep default "system"
  }

  // 4. Apply resolved theme to <html> data-theme attribute
  resolvedTheme.subscribe((theme: ResolvedTheme) => {
    document.documentElement.setAttribute("data-theme", theme);
  });
}

/**
 * Update the theme preference in both the local store and persisted config.
 */
export async function setThemePreference(
  pref: ThemePreference,
): Promise<void> {
  themePreference.set(pref);
  try {
    await setConfigValue("theme", pref);
  } catch {
    // Silently fail — the store is still updated for this session
  }
}
