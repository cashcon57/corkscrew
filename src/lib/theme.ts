import { writable, derived } from "svelte/store";
import { getConfig, setConfigValue } from "./api";

export type ThemePreference = "system" | "light" | "dark";
export type ResolvedTheme = "light" | "dark";

/** User's chosen theme preference: "system", "light", or "dark". */
export const themePreference = writable<ThemePreference>("system");

/** The OS-level color scheme detected via matchMedia. */
export const systemTheme = writable<ResolvedTheme>("dark");

/** Whether macOS vibrancy effects are available. */
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
