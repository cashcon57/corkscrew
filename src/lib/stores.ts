import { writable, derived } from "svelte/store";
import type { Bottle, DetectedGame, InstalledMod, AppConfig } from "./types";

// App state
export const bottles = writable<Bottle[]>([]);
export const games = writable<DetectedGame[]>([]);
export const installedMods = writable<InstalledMod[]>([]);
export const config = writable<AppConfig>({ nexus_api_key: null, download_dir: null });

// UI state
export const selectedBottle = writable<string | null>(null);
export const selectedGame = writable<DetectedGame | null>(null);
export const currentPage = writable<string>("dashboard");
export const isLoading = writable<boolean>(false);
export const errorMessage = writable<string | null>(null);
export const successMessage = writable<string | null>(null);

// Derived stores
export const activeGames = derived(
  [games, selectedBottle],
  ([$games, $selectedBottle]) => {
    if (!$selectedBottle) return $games;
    return $games.filter((g) => g.bottle_name === $selectedBottle);
  }
);

export const activeMods = derived(
  [installedMods, selectedGame],
  ([$installedMods, $selectedGame]) => {
    if (!$selectedGame) return $installedMods;
    return $installedMods.filter(
      (m) =>
        m.game_id === $selectedGame.game_id &&
        m.bottle_name === $selectedGame.bottle_name
    );
  }
);

// Notification helpers
export function showError(msg: string) {
  errorMessage.set(msg);
  setTimeout(() => errorMessage.set(null), 5000);
}

export function showSuccess(msg: string) {
  successMessage.set(msg);
  setTimeout(() => successMessage.set(null), 3000);
}
