import { writable, derived } from "svelte/store";
import type { Bottle, DetectedGame, InstalledMod, AppConfig, SkseStatus, Profile, CollectionSummary, FomodInstaller, GameLock, WjArchiveStatus, KnownUninstalledGame } from "./types";
import { wineCtx } from "./types";

// App state
export const bottles = writable<Bottle[]>([]);
export const games = writable<DetectedGame[]>([]);
export const installedMods = writable<InstalledMod[]>([]);
export const config = writable<AppConfig>({ nexus_api_key: null, download_dir: null, staging_dir: null, has_completed_setup: false, controller_mode: false });

// UI state
export const selectedBottle = writable<string | null>(null);
export const selectedGame = writable<DetectedGame | null>(null);

// "Show uninstalled games in the game-selector dropdown" toggle. Persisted
// via the config_value table (hydrated once on app startup) so the choice
// survives restarts. The companion list of uninstalled games is lazy-fetched
// when the toggle is first turned on.
export const showUninstalledGames = writable<boolean>(false);
export const uninstalledGames = writable<KnownUninstalledGame[]>([]);
export const currentPage = writable<string>("mods");
export const isLoading = writable<boolean>(false);
export const errorMessage = writable<string | null>(null);
export const successMessage = writable<string | null>(null);

// Derived stores
export const activeGames = derived(
  [games, selectedBottle],
  ([$games, $selectedBottle]) => {
    if (!$selectedBottle) return $games;
    return $games.filter((g) => wineCtx(g)?.bottle_name === $selectedBottle);
  }
);

export const activeMods = derived(
  [installedMods, selectedGame],
  ([$installedMods, $selectedGame]) => {
    if (!$selectedGame) return $installedMods;
    return $installedMods.filter(
      (m) =>
        m.game_id === $selectedGame.game_id &&
        m.bottle_name === (wineCtx($selectedGame)?.bottle_name ?? '')
    );
  }
);

// Profile state (global — sidebar selector + profile page)
export const activeProfile = writable<Profile | null>(null);
export const profileList = writable<Profile[]>([]);

// Collection/modlist state (global — top bar selector + collection pages)
export const activeCollection = writable<CollectionSummary | null>(null);
export const collectionList = writable<CollectionSummary[]>([]);

// Collection install progress (global — visible from any page)
export interface DownloadItem {
  modName: string;
  modIndex: number;
  downloaded: number;
  total: number;
}

export interface ModProgressDetail {
  name: string;
  index: number;
  status: "pending" | "queued" | "downloading" | "downloaded" | "cached" | "extracting" | "staged" | "installing" | "deploying" | "done" | "failed" | "skipped" | "user_action" | "fomod_pending";
  error?: string;
  downloadBytes?: number;
  downloadTotal?: number;
  stepDetail?: string;
  extractionSpeed?: number;
  installSpeed?: number;
  extractFilesDone?: number;
  extractFilesTotal?: number;
  extractBytesDone?: number;
  extractBytesTotal?: number;
  extractSpeedLive?: number;
  extractLastBytes?: number;
  extractLastTime?: number;
  deployFilesDone?: number;
  deployFilesTotal?: number;
  deployBytesDone?: number;
  deployBytesTotal?: number;
  deploySpeedLive?: number;
  deployLastBytes?: number;
  deployLastTime?: number;
  fomodData?: {
    correlationId: string;
    installer: FomodInstaller;
  };
}

export interface UserActionItem {
  modName: string;
  action: string;
  url?: string;
  instructions?: string;
}

export interface PendingFomod {
  modIndex: number;
  modName: string;
  correlationId: string;
  installer: FomodInstaller;
}

export interface CollectionInstallStatus {
  active: boolean;
  collectionName: string;
  collectionDescription?: string;
  phase: "downloading" | "staging" | "installing" | "complete" | "failed" | "";
  // Download phase
  downloadProgress: {
    total: number;
    completed: number;
    failed: number;
    cached: number;
    maxConcurrent: number;
    active: DownloadItem[];
  };
  // Install phase (legacy compat + install phase)
  installProgress: {
    current: number;
    total: number;
    currentMod: string;
    step: string;
    stepDetail: string;
  };
  // Per-mod details
  modDetails: ModProgressDetail[];
  // Timing
  startTime: number;
  elapsed: string;
  // Result
  result: { installed: number; skipped: number; failed: number } | null;
  // User actions
  userActions: UserActionItem[];
  // Pending FOMOD wizards
  pendingFomods: PendingFomod[];
  // Overall progress
  overallProgress: number;
  downloadSpeed: number;
  downloadEta: string;
  stagingSpeed: number;   // bytes/sec during extraction phase
  installSpeed: number;   // bytes/sec during install/deploy phase
  // Verbose log entries
  logEntries: LogEntry[];
  // Legacy compat fields
  currentMod: string;
  step: string;
  current: number;
  total: number;
}

export interface LogEntry {
  timestamp: number;
  message: string;
  level: "info" | "warn" | "error";
}
export const collectionInstallStatus = writable<CollectionInstallStatus | null>(null);

// Wabbajack install progress (global — visible from any page)
export interface WjActiveDownload {
  name: string;
  index: number;
  bytes: number;
  totalBytes: number;
}

export interface WjLogEntry {
  timestamp: number;
  message: string;
  level: "info" | "warn" | "error";
}

export interface WjInstallStatus {
  active: boolean;
  modlistName: string;
  phase: "preflight" | "downloading" | "extracting" | "directives" | "deploying" | "complete" | "failed" | "cancelled" | "";
  // Download phase
  downloadProgress: {
    current: number;
    total: number;
    completed: number;
    bytesDownloaded: number;
    totalBytes: number;
    currentFile: string;
    speed: number; // bytes/sec
    eta: string;
    maxConcurrent: number;
    activeDownloads: WjActiveDownload[];
  };
  // Extraction phase
  extractionProgress: {
    current: number;
    total: number;
    currentArchive: string;
    totalBytes: number;
    bytesCompleted: number;
  };
  // Directive phase
  directiveProgress: {
    current: number;
    total: number;
    bytesProcessed: number;
    totalBytes: number;
    currentFile: string;
    directiveType: string;
  };
  // Deploy phase
  deployProgress: {
    current: number;
    total: number;
    bytesDeployed: number;
    totalBytes: number;
  };
  // Per-archive status (for extraction view)
  archives: WjArchiveStatus[];
  // Timing
  startTime: number;
  elapsed: string;
  overallProgress: number; // 0-100
  speed: number; // bytes/sec — current phase speed
  speedLabel: string;
  etaLabel: string;
  // Result
  result: {
    filesDeployed: number;
    warnings: string[];
    elapsed: number;
  } | null;
  error: string | null;
  // Install ID for cancellation
  installId: number | null;
  // Preflight report
  preflightNote: string;
  // Activity log
  logEntries: WjLogEntry[];
  // Modlist metadata (for display during install)
  readmeHtml: string;
  description: string;
  author: string;
  imageUrl: string;
  featuredMods: Array<{ name: string; description: string }>;
}

export const wjInstallStatus = writable<WjInstallStatus | null>(null);

/** Incremented when a WJ install completes — mods page should refresh on change. */
export const wjInstallGeneration = writable<number>(0);

// Collection uninstall progress (global — visible from any page)
export interface CollectionUninstallStatus {
  active: boolean;
  collectionName: string;
  totalMods: number;
  currentMod: number;
  currentModName: string;
  currentStep: string;
  completed: number;
  failed: number;
  phase: "removing" | "redeploying" | "complete";
  errors: string[];
  result: { modsRemoved: number; downloadsRemoved: number } | null;
}
export const collectionUninstallStatus = writable<CollectionUninstallStatus | null>(null);

// Background hashing progress (global — visible from any page)
export const hashingProgress = writable<import("$lib/types").HashingProgress | null>(null);

// SKSE state
export const skseStatus = writable<SkseStatus | null>(null);

// App version (loaded at startup from Tauri config)
export const appVersion = writable<string>("0.0.0");

// Auto-update state (shared between layout and settings)
export const updateReady = writable(false);
export const updateVersion = writable("");
export const updateNotes = writable<string | null>(null);
export const updateChecking = writable(false);
export const updateError = writable<string | null>(null);
// Set by layout to allow settings page to trigger a manual check
export const triggerUpdateCheck = writable<(() => Promise<void>) | null>(null);
export function setUpdateCheckFn(fn: () => Promise<void>) {
  triggerUpdateCheck.set(fn);
}

// Sidebar collapse state (persisted to localStorage)
function createPersistedBool(key: string, fallback: boolean) {
  const stored = typeof localStorage !== "undefined" ? localStorage.getItem(key) : null;
  const initial = stored !== null ? stored === "true" : fallback;
  const store = writable(initial);
  store.subscribe((v) => {
    if (typeof localStorage !== "undefined") localStorage.setItem(key, String(v));
  });
  return store;
}
export const sidebarCollapsed = createPersistedBool("corkscrew:sidebar-collapsed", false);
export const controllerMode = createPersistedBool("corkscrew:controller-mode", false);

// Pending NXM install (download complete, awaiting user confirmation)
export interface PendingNxmInstall {
  archivePath: string;
  modName: string;
  modVersion: string;
  gameId: string;
  bottleName: string;
  nexusModId?: number;
  nxmUrl: string;
}
export const pendingNxmInstall = writable<PendingNxmInstall | null>(null);

// Counter that increments when an NXM install completes — mods page watches this to reload
export const nxmInstallComplete = writable<number>(0);

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

// Mod state version — incremented on profile/collection switches to trigger mods page refresh
export const modStateVersion = writable(0);

// Game Lock — tracks whether a game is currently running (MO2-style lock)
export const gameLock = writable<GameLock | null>(null);
// Whether the user has force-unlocked (overridden the lock for this session)
export const gameLockOverridden = writable<boolean>(false);

// Experimental: Native Mode (macOS-native modding for supported games).
// Hydrated from the backend config in Task 5.5 (Mode-scoped routing).
export const nativeMode = writable<boolean>(false);

// Controls visibility of the Native Mode topbar toggle and first-run banner.
// Off by default — native macOS modding is in active development and does not
// yet function for end users. The user opts in via Settings → About.
export const nativeModeVisible = writable<boolean>(false);
