/**
 * Background hashing tracking service.
 *
 * Subscribes to "background-hashing-progress" Tauri events and updates the
 * global hashingProgress store.  Lives at the layout level so progress
 * tracking survives page navigation.
 */
import { listen } from "@tauri-apps/api/event";
import { hashingProgress } from "$lib/stores";
import { startBackgroundHashing } from "$lib/api";
import type { HashingProgress } from "$lib/types";

let unlisten: (() => void) | null = null;

/** Auto-dismiss timer — clears the banner a few seconds after hashing completes. */
let dismissTimer: ReturnType<typeof setTimeout> | null = null;

/**
 * Start listening for background hashing events.
 * Call once at layout mount.
 */
export async function initHashingListener(): Promise<void> {
  if (unlisten) return; // already listening

  unlisten = await listen<HashingProgress>("background-hashing-progress", (event) => {
    const p = event.payload;
    hashingProgress.set(p);

    if (p.done) {
      // Auto-dismiss the banner after 15s so the user sees the final state
      if (dismissTimer) clearTimeout(dismissTimer);
      dismissTimer = setTimeout(() => {
        hashingProgress.set(null);
        dismissTimer = null;
      }, 15_000);
    }
  });
}

/**
 * Kick off background hashing for a game/bottle.
 * Typically called after a collection install completes.
 */
export async function triggerBackgroundHashing(
  gameId: string,
  bottleName: string,
  gamePid?: number,
): Promise<void> {
  // Clear any previous done state
  hashingProgress.set(null);
  if (dismissTimer) {
    clearTimeout(dismissTimer);
    dismissTimer = null;
  }

  await startBackgroundHashing(gameId, bottleName, gamePid);
}

/**
 * Dismiss the hashing banner immediately.
 */
export function dismissHashingBanner(): void {
  hashingProgress.set(null);
  if (dismissTimer) {
    clearTimeout(dismissTimer);
    dismissTimer = null;
  }
}

/**
 * Clean up the event listener. Call on layout destroy.
 */
export function destroyHashingListener(): void {
  if (unlisten) {
    unlisten();
    unlisten = null;
  }
  if (dismissTimer) {
    clearTimeout(dismissTimer);
    dismissTimer = null;
  }
}
