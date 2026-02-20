import { writable, derived } from "svelte/store";
import type { Bottle, DetectedGame, InstalledMod, AppConfig, SkseStatus } from "./types";

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

// Collection install progress (global — visible from any page)
export interface CollectionInstallStatus {
  active: boolean;
  collectionName: string;
  currentMod: string;
  step: string;
  current: number;
  total: number;
}
export const collectionInstallStatus = writable<CollectionInstallStatus | null>(null);

// SKSE state
export const skseStatus = writable<SkseStatus | null>(null);

// App version (loaded at startup from Tauri config)
export const appVersion = writable<string>("0.0.0");

// Auto-update state (shared between layout and settings)
export const updateReady = writable(false);
export const updateVersion = writable("");
export const updateChecking = writable(false);
export const updateError = writable<string | null>(null);
// Set by layout to allow settings page to trigger a manual check
export let triggerUpdateCheck: (() => Promise<void>) | null = null;
export function setUpdateCheckFn(fn: () => Promise<void>) {
  triggerUpdateCheck = fn;
}

// Notification log (persistent — backed by SQLite)
export const notificationCount = writable<number>(0);
export const showNotificationLog = writable<boolean>(false);

// Notification helpers
export function showError(msg: string) {
  errorMessage.set(msg);
  setTimeout(() => errorMessage.set(null), 5000);
}

export function showSuccess(msg: string) {
  successMessage.set(msg);
  setTimeout(() => successMessage.set(null), 3000);
}
