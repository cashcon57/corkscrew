import { getCurrent, onOpenUrl } from "@tauri-apps/plugin-deep-link";

/**
 * Deep-link (nxm:// / corkscrew://) delivery.
 *
 * URLs reach the webview two ways:
 *  - `deep-link://new-url` events (`onOpenUrl`) — a link clicked while the
 *    app is running (Linux: forwarded from the second process by the
 *    single-instance plugin; macOS: Apple event).
 *  - `getCurrent()` — the URL the app was *launched* with. On a Linux cold
 *    start the deep-link plugin emits its event during plugin init, before
 *    the webview exists, so without this the link is simply lost.
 *
 * The backend keeps the "current" URL for the whole process lifetime, so
 * `getCurrent()` would replay the same link after a webview reload, and it
 * can race with the event for the same click. Event URLs are always handled
 * (every event is a real click, and users retry by clicking again);
 * `getCurrent()` URLs are skipped if already handled.
 */

const HANDLED_STORAGE_KEY = "corkscrew:handled-deep-links";
const MAX_REMEMBERED = 50;
/** How long an event matching a just-handled launch URL counts as its echo. */
const ECHO_WINDOW_MS = 5000;

function loadHandled(): string[] {
  try {
    const raw = sessionStorage.getItem(HANDLED_STORAGE_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed) ? parsed.filter((u): u is string => typeof u === "string") : [];
  } catch (err) {
    console.warn("Failed to read handled deep links from sessionStorage:", err);
    return [];
  }
}

function saveHandled(urls: string[]): void {
  try {
    sessionStorage.setItem(HANDLED_STORAGE_KEY, JSON.stringify(urls.slice(-MAX_REMEMBERED)));
  } catch (err) {
    console.warn("Failed to persist handled deep links to sessionStorage:", err);
  }
}

/**
 * Route every deep link — launch URL and runtime clicks — through `handler`.
 * Registers the event listener before reading the launch URL so a link that
 * arrives in between isn't missed. Resolves to an unlisten function.
 */
export async function listenForDeepLinks(
  handler: (url: string) => void | Promise<void>,
): Promise<() => void> {
  const handled = loadHandled();
  // URLs taken from getCurrent() → when they were handled. If the event for
  // the same click is still in flight it arrives within ECHO_WINDOW_MS and is
  // swallowed once; a later deliberate re-click of the same link still works.
  const pendingEventEcho = new Map<string, number>();

  const dispatch = (url: string) => {
    if (!handled.includes(url)) {
      handled.push(url);
      saveHandled(handled);
    }
    Promise.resolve()
      .then(() => handler(url))
      .catch((err) => console.error("Deep link handler failed:", err));
  };

  const unlisten = await onOpenUrl((urls) => {
    for (const url of urls) {
      const echoAt = pendingEventEcho.get(url);
      if (echoAt !== undefined) {
        pendingEventEcho.delete(url);
        if (Date.now() - echoAt < ECHO_WINDOW_MS) continue;
      }
      dispatch(url);
    }
  });

  try {
    const launchUrls = (await getCurrent()) ?? [];
    for (const url of launchUrls) {
      if (handled.includes(url)) continue;
      pendingEventEcho.set(url, Date.now());
      dispatch(url);
    }
  } catch (err) {
    console.error("Failed to read launch deep link:", err);
  }

  return unlisten;
}
