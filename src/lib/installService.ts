/**
 * Persistent install tracking service.
 *
 * Subscribes to "install-progress" Tauri events and updates the global
 * collectionInstallStatus store.  Lives at the layout level so progress
 * tracking survives page navigation.
 */
import { listen } from "@tauri-apps/api/event";
import { collectionInstallStatus } from "$lib/stores";
import type { CollectionInstallStatus, ModProgressDetail, LogEntry } from "$lib/stores";
import type { InstallProgressEvent } from "$lib/types";

let unlisten: (() => void) | null = null;
let timer: ReturnType<typeof setInterval> | null = null;

// Speed tracking — rolling window
let speedSamples: { time: number; bytes: number }[] = [];
const SPEED_WINDOW_MS = 5000;
let cumulativeDownloaded = 0;

// Staging (extraction) speed tracking
let stagingSpeedSamples: { time: number; bytes: number }[] = [];
let stagingSizeAccumulator = 0;

// Install (deploy) speed tracking
let installSpeedSamples: { time: number; bytes: number }[] = [];
let installSizeAccumulator = 0;

// Event throttling — batch rapid events to avoid overwhelming the UI
let pendingEvent: InstallProgressEvent | null = null;
let throttleTimer: ReturnType<typeof setTimeout> | null = null;
const THROTTLE_MS = 50; // max ~20 UI updates per second for non-critical events

// Events that should always update immediately (phase transitions, completions)
const IMMEDIATE_EVENTS = new Set([
  "downloadPhaseStarted", "allDownloadsCompleted",
  "stagingPhaseStarted", "installPhaseStarted",
  "modStarted", "modCompleted", "modFailed", "collectionCompleted",
  "userActionRequired", "downloadModStarted", "downloadModCompleted", "downloadModFailed",
  "stagingModStarted", "stagingModCompleted", "stagingModFailed", "initializing",
]);

function calculateSpeed(currentBytes: number): number {
  const now = Date.now();
  cumulativeDownloaded = currentBytes;
  speedSamples.push({ time: now, bytes: currentBytes });
  speedSamples = speedSamples.filter((s) => now - s.time <= SPEED_WINDOW_MS);
  if (speedSamples.length < 2) return 0;
  const oldest = speedSamples[0];
  const elapsed = (now - oldest.time) / 1000;
  if (elapsed <= 0) return 0;
  return (currentBytes - oldest.bytes) / elapsed;
}

function calculateStagingSpeed(currentBytes: number): number {
  const now = Date.now();
  stagingSizeAccumulator = currentBytes;
  stagingSpeedSamples.push({ time: now, bytes: currentBytes });
  stagingSpeedSamples = stagingSpeedSamples.filter((s) => now - s.time <= SPEED_WINDOW_MS);
  if (stagingSpeedSamples.length < 2) return 0;
  const oldest = stagingSpeedSamples[0];
  const elapsed = (now - oldest.time) / 1000;
  if (elapsed <= 0) return 0;
  return (currentBytes - oldest.bytes) / elapsed;
}

function calculateInstallSpeed(currentBytes: number): number {
  const now = Date.now();
  installSizeAccumulator = currentBytes;
  installSpeedSamples.push({ time: now, bytes: currentBytes });
  installSpeedSamples = installSpeedSamples.filter((s) => now - s.time <= SPEED_WINDOW_MS);
  if (installSpeedSamples.length < 2) return 0;
  const oldest = installSpeedSamples[0];
  const elapsed = (now - oldest.time) / 1000;
  if (elapsed <= 0) return 0;
  return (currentBytes - oldest.bytes) / elapsed;
}

function formatEta(remainingBytes: number, speed: number): string {
  if (speed <= 0) return "";
  const secs = remainingBytes / speed;
  if (secs < 60) return "< 1 min";
  if (secs < 3600) return `~${Math.ceil(secs / 60)} min`;
  const hrs = Math.floor(secs / 3600);
  const mins = Math.ceil((secs % 3600) / 60);
  return `~${hrs}h ${mins}m`;
}

function computeOverallProgress(s: CollectionInstallStatus): number {
  const DL_WEIGHT = 0.40;
  const STAGING_WEIGHT = 0.20;
  const INSTALL_WEIGHT = 0.40;

  const dlTotal = s.downloadProgress.total || 1;
  const dlProgress = s.downloadProgress.completed / dlTotal;

  // Count mods that completed extraction (past extracting phase)
  const stagingDone = s.modDetails.filter(
    (m) => ["staged", "deploying", "done", "failed", "skipped", "user_action"].includes(m.status),
  ).length;
  const stagingTotal = s.modDetails.length || 1;
  const stagingProgress = stagingDone / stagingTotal;

  const instTotal = s.installProgress.total || 1;
  const instDone = s.modDetails.filter(
    (m) => m.status === "done" || m.status === "failed" || m.status === "skipped" || m.status === "user_action",
  ).length;
  const instProgress = instDone / instTotal;

  if (s.phase === "complete") return 100;
  if (s.phase === "downloading") return Math.round(dlProgress * DL_WEIGHT * 100);
  if (s.phase === "staging") return Math.round((DL_WEIGHT + stagingProgress * STAGING_WEIGHT) * 100);
  if (s.phase === "installing") return Math.round((DL_WEIGHT + STAGING_WEIGHT + instProgress * INSTALL_WEIGHT) * 100);
  return 0;
}

function formatElapsed(startTime: number): string {
  const secs = Math.floor((Date.now() - startTime) / 1000);
  if (secs < 60) return `${secs}s`;
  return `${Math.floor(secs / 60)}m ${secs % 60}s`;
}

/** Start tracking a collection install. Call from the collections page when install begins. */
export async function startInstallTracking(
  collectionName: string,
  totalMods: number,
  modNames: string[],
  description?: string,
) {
  // Clean up any previous tracking
  stopInstallTracking();

  const now = Date.now();

  const modDetails: ModProgressDetail[] = modNames.map((name, i) => ({
    name,
    index: i,
    status: "pending" as const,
  }));

  const initial: CollectionInstallStatus = {
    active: true,
    collectionName,
    phase: "downloading",
    downloadProgress: {
      total: 0,
      completed: 0,
      failed: 0,
      cached: 0,
      maxConcurrent: 0,
      active: [],
    },
    installProgress: {
      current: 0,
      total: totalMods,
      currentMod: "",
      step: "preparing",
      stepDetail: "",
    },
    modDetails,
    startTime: now,
    elapsed: "0s",
    result: null,
    userActions: [],
    overallProgress: 0,
    downloadSpeed: 0,
    downloadEta: "",
    stagingSpeed: 0,
    installSpeed: 0,
    logEntries: [{ timestamp: Date.now(), message: `Starting installation of '${collectionName}' (${totalMods} mods)`, level: "info" as const }],
    collectionDescription: description,
    // Legacy compat
    currentMod: "",
    step: "preparing",
    current: 0,
    total: totalMods,
  };

  collectionInstallStatus.set(initial);

  // Start elapsed timer
  timer = setInterval(() => {
    collectionInstallStatus.update((s) => {
      if (!s) return s;
      return { ...s, elapsed: formatElapsed(s.startTime) };
    });
  }, 1000);

  // Subscribe to progress events with throttling for non-critical updates
  unlisten = await listen<InstallProgressEvent>("install-progress", (event) => {
    const e = event.payload;
    if (IMMEDIATE_EVENTS.has(e.kind)) {
      // Flush any pending throttled event first, then handle immediately
      if (pendingEvent) {
        handleProgressEvent(pendingEvent);
        pendingEvent = null;
        if (throttleTimer) { clearTimeout(throttleTimer); throttleTimer = null; }
      }
      handleProgressEvent(e);
    } else {
      // Throttle high-frequency events (downloadProgress, stepChanged, modStarted, etc.)
      pendingEvent = e;
      if (!throttleTimer) {
        throttleTimer = setTimeout(() => {
          if (pendingEvent) {
            handleProgressEvent(pendingEvent);
            pendingEvent = null;
          }
          throttleTimer = null;
        }, THROTTLE_MS);
      }
    }
  });
}

function logMessageForEvent(e: InstallProgressEvent): { message: string; level: LogEntry["level"] } | null {
  switch (e.kind) {
    case "initializing": return { message: e.message, level: "info" };
    case "downloadPhaseStarted": return { message: `Download phase started (${e.total_downloads} files, ${e.max_concurrent} threads)`, level: "info" };
    case "downloadModStarted": return { message: `Downloading: ${e.mod_name}`, level: "info" };
    case "downloadModCompleted": return { message: `${e.cached ? "Cached" : "Downloaded"}: ${e.mod_name}`, level: "info" };
    case "downloadModFailed": return { message: `Download failed: ${e.mod_name} — ${e.error}`, level: "error" };
    case "allDownloadsCompleted": return { message: `Downloads complete (${e.downloaded} downloaded, ${e.cached} cached, ${e.failed} failed, ${e.skipped} skipped)`, level: "info" };
    case "stagingPhaseStarted": return { message: `Extraction phase started (${e.total_mods} archives, ${e.max_concurrent} threads)`, level: "info" };
    case "stagingModStarted": return { message: `Extracting: ${e.mod_name}`, level: "info" };
    case "stagingModCompleted": return { message: `Extracted: ${e.mod_name}`, level: "info" };
    case "stagingModFailed": return { message: `Extraction failed: ${e.mod_name} — ${e.error}`, level: "error" };
    case "installPhaseStarted": return { message: `Install phase started (${e.total_mods} mods)`, level: "info" };
    case "modStarted": return { message: `Installing ${e.mod_index + 1}/${e.total_mods}: ${e.mod_name}`, level: "info" };
    case "modCompleted": return { message: `Installed: ${e.mod_name}`, level: "info" };
    case "modFailed": return { message: `Install failed: ${e.mod_name} — ${e.error}`, level: "error" };
    case "userActionRequired": return { message: `Action required: ${e.mod_name} — ${e.action}`, level: "warn" };
    case "collectionCompleted": return { message: `Collection complete (${e.installed} installed, ${e.skipped} skipped, ${e.failed} failed)`, level: e.failed > 0 ? "warn" : "info" };
    case "stepChanged": return e.detail ? { message: `  ${e.detail}`, level: "info" } : null;
    default: return null;
  }
}

function handleProgressEvent(e: InstallProgressEvent) {
  collectionInstallStatus.update((s) => {
    if (!s) return s;
    const next = { ...s };

    // Append log entry
    const logMsg = logMessageForEvent(e);
    if (logMsg) {
      next.logEntries = [...next.logEntries, { timestamp: Date.now(), ...logMsg }];
    }

    switch (e.kind) {
      // ---- Initialization ----
      case "initializing":
        // Only adds to log, no state change needed
        break;

      // ---- Download Phase ----
      case "downloadPhaseStarted":
        next.phase = "downloading";
        next.downloadProgress = {
          ...next.downloadProgress,
          total: e.total_downloads,
          maxConcurrent: e.max_concurrent,
        };
        break;

      case "downloadQueued":
        if (next.modDetails[e.mod_index]) {
          next.modDetails = [...next.modDetails];
          next.modDetails[e.mod_index] = { ...next.modDetails[e.mod_index], status: "queued" };
        }
        break;

      case "downloadModStarted":
        if (next.modDetails[e.mod_index]) {
          next.modDetails = [...next.modDetails];
          next.modDetails[e.mod_index] = { ...next.modDetails[e.mod_index], name: e.mod_name, status: "downloading" };
        }
        // Add to active downloads
        next.downloadProgress = {
          ...next.downloadProgress,
          active: [
            ...next.downloadProgress.active.filter((d) => d.modIndex !== e.mod_index),
            { modName: e.mod_name, modIndex: e.mod_index, downloaded: 0, total: 0 },
          ],
        };
        break;

      case "downloadProgress": {
        // Update per-mod download bytes
        if (next.modDetails[e.mod_index]) {
          next.modDetails = [...next.modDetails];
          next.modDetails[e.mod_index] = {
            ...next.modDetails[e.mod_index],
            downloadBytes: e.downloaded,
            downloadTotal: e.total,
          };
        }
        // Update active download list
        const updatedActive = next.downloadProgress.active.map((d) =>
          d.modIndex === e.mod_index
            ? { ...d, downloaded: e.downloaded, total: e.total }
            : d,
        );
        next.downloadProgress = { ...next.downloadProgress, active: updatedActive };
        // Speed + ETA
        const totalActiveBytes = updatedActive.reduce((sum, d) => sum + d.downloaded, 0);
        const speed = calculateSpeed(totalActiveBytes);
        const totalRemaining = updatedActive.reduce((sum, d) => sum + Math.max(0, d.total - d.downloaded), 0);
        next.downloadSpeed = speed;
        next.downloadEta = formatEta(totalRemaining, speed);
        break;
      }

      case "downloadModCompleted":
        if (next.modDetails[e.mod_index]) {
          next.modDetails = [...next.modDetails];
          next.modDetails[e.mod_index] = {
            ...next.modDetails[e.mod_index],
            status: e.cached ? "cached" : "downloaded",
          };
        }
        next.downloadProgress = {
          ...next.downloadProgress,
          completed: next.downloadProgress.completed + 1,
          cached: next.downloadProgress.cached + (e.cached ? 1 : 0),
          active: next.downloadProgress.active.filter((d) => d.modIndex !== e.mod_index),
        };
        // Update legacy compat
        next.current = next.downloadProgress.completed;
        break;

      case "downloadModFailed":
        if (next.modDetails[e.mod_index]) {
          next.modDetails = [...next.modDetails];
          next.modDetails[e.mod_index] = {
            ...next.modDetails[e.mod_index],
            status: "failed",
            error: e.error,
          };
        }
        next.downloadProgress = {
          ...next.downloadProgress,
          failed: next.downloadProgress.failed + 1,
          active: next.downloadProgress.active.filter((d) => d.modIndex !== e.mod_index),
        };
        break;

      case "allDownloadsCompleted":
        next.downloadProgress = {
          ...next.downloadProgress,
          completed: e.downloaded + e.cached,
          cached: e.cached,
          failed: e.failed,
          active: [],
        };
        break;

      // ---- Staging Phase (concurrent extraction) ----
      case "stagingPhaseStarted":
        next.phase = "staging";
        next.step = "extracting";
        stagingSpeedSamples = [];
        stagingSizeAccumulator = 0;
        break;

      case "stagingModStarted":
        if (next.modDetails[e.mod_index]) {
          next.modDetails = [...next.modDetails];
          next.modDetails[e.mod_index] = { ...next.modDetails[e.mod_index], status: "extracting" };
        }
        break;

      case "stagingModCompleted":
        if (next.modDetails[e.mod_index]) {
          next.modDetails = [...next.modDetails];
          next.modDetails[e.mod_index] = { ...next.modDetails[e.mod_index], status: "staged" };
        }
        // Track extraction throughput
        if (e.extracted_size) {
          stagingSizeAccumulator += e.extracted_size;
          next.stagingSpeed = calculateStagingSpeed(stagingSizeAccumulator);
        }
        break;

      case "stagingModFailed":
        if (next.modDetails[e.mod_index]) {
          next.modDetails = [...next.modDetails];
          next.modDetails[e.mod_index] = { ...next.modDetails[e.mod_index], status: "failed", error: e.error };
        }
        break;

      // ---- Install Phase ----
      case "installPhaseStarted":
        next.phase = "installing";
        next.installProgress = {
          ...next.installProgress,
          total: e.total_mods,
          current: 0,
        };
        installSpeedSamples = [];
        installSizeAccumulator = 0;
        break;

      case "modStarted":
        next.installProgress = {
          ...next.installProgress,
          current: e.mod_index + 1,
          currentMod: e.mod_name,
          step: "preparing",
          stepDetail: "",
        };
        // Legacy compat
        next.currentMod = e.mod_name;
        next.current = e.mod_index + 1;
        next.total = e.total_mods;
        next.step = "preparing";

        if (next.modDetails[e.mod_index]) {
          next.modDetails = [...next.modDetails];
          next.modDetails[e.mod_index] = { ...next.modDetails[e.mod_index], name: e.mod_name, status: "installing" };
        }
        break;

      case "stepChanged":
        next.installProgress = {
          ...next.installProgress,
          step: e.step,
          stepDetail: e.detail ?? "",
        };
        next.step = e.step;
        if (next.modDetails[e.mod_index]) {
          const stepStatus = e.step === "deploying" ? "deploying" as const
            : e.step === "extracting" ? "extracting" as const
            : "installing" as const;
          next.modDetails = [...next.modDetails];
          next.modDetails[e.mod_index] = { ...next.modDetails[e.mod_index], status: stepStatus, stepDetail: e.detail ?? undefined };
        }
        break;

      case "modCompleted":
        if (next.modDetails[e.mod_index]) {
          next.modDetails = [...next.modDetails];
          next.modDetails[e.mod_index] = { ...next.modDetails[e.mod_index], status: "done" };
        }
        // Also advance install counter (in case modStarted was missed)
        {
          const doneCount = next.modDetails.filter(m => m.status === "done" || m.status === "failed" || m.status === "skipped" || m.status === "user_action").length;
          if (doneCount > next.installProgress.current) {
            next.installProgress = { ...next.installProgress, current: doneCount };
            next.current = doneCount;
          }
        }
        break;

      case "modFailed":
        if (next.modDetails[e.mod_index]) {
          next.modDetails = [...next.modDetails];
          next.modDetails[e.mod_index] = {
            ...next.modDetails[e.mod_index],
            status: "failed",
            error: e.error,
          };
        }
        break;

      case "userActionRequired":
        next.userActions = [
          ...next.userActions,
          {
            modName: e.mod_name,
            action: e.action,
            url: e.url ?? undefined,
            instructions: e.instructions ?? undefined,
          },
        ];
        if (next.modDetails[e.mod_index]) {
          next.modDetails = [...next.modDetails];
          next.modDetails[e.mod_index] = { ...next.modDetails[e.mod_index], status: "user_action" };
        }
        break;

      case "collectionCompleted":
        next.phase = "complete";
        next.result = {
          installed: e.installed,
          skipped: e.skipped,
          failed: e.failed,
        };
        next.overallProgress = 100;
        break;
    }

    next.overallProgress = computeOverallProgress(next);
    return next;
  });
}

/** Resume tracking a previously interrupted collection install. */
export async function resumeInstallTracking(
  collectionName: string,
  totalMods: number,
  completedMods: number,
  modStatuses: Record<string, string>,
  modNames?: string[],
) {
  stopInstallTracking();

  const now = Date.now();

  const modDetails: ModProgressDetail[] = [];
  for (let i = 0; i < totalMods; i++) {
    const status = modStatuses[String(i)];
    let mappedStatus: ModProgressDetail["status"] = "pending";
    if (status === "installed" || status === "already_installed") mappedStatus = "done";
    else if (status === "failed") mappedStatus = "failed";
    else if (status === "user_action") mappedStatus = "user_action";
    else if (status === "skipped") mappedStatus = "skipped";

    modDetails.push({
      name: modNames?.[i] ?? `Mod ${i + 1}`,
      index: i,
      status: mappedStatus,
    });
  }

  const initial: CollectionInstallStatus = {
    active: true,
    collectionName,
    phase: "downloading",
    downloadProgress: {
      total: 0,
      completed: 0,
      failed: 0,
      cached: 0,
      maxConcurrent: 0,
      active: [],
    },
    installProgress: {
      current: completedMods,
      total: totalMods,
      currentMod: "",
      step: "resuming",
      stepDetail: "",
    },
    modDetails,
    startTime: now,
    elapsed: "0s",
    result: null,
    userActions: [],
    overallProgress: totalMods > 0 ? Math.round((completedMods / totalMods) * 100) : 0,
    downloadSpeed: 0,
    downloadEta: "",
    stagingSpeed: 0,
    installSpeed: 0,
    logEntries: [{ timestamp: Date.now(), message: `Resuming installation of '${collectionName}' (${completedMods}/${totalMods} completed)`, level: "info" as const }],
    currentMod: "",
    step: "resuming",
    current: completedMods,
    total: totalMods,
  };

  collectionInstallStatus.set(initial);

  timer = setInterval(() => {
    collectionInstallStatus.update((s) => {
      if (!s) return s;
      return { ...s, elapsed: formatElapsed(s.startTime) };
    });
  }, 1000);

  unlisten = await listen<InstallProgressEvent>("install-progress", (event) => {
    const e = event.payload;
    if (IMMEDIATE_EVENTS.has(e.kind)) {
      if (pendingEvent) {
        handleProgressEvent(pendingEvent);
        pendingEvent = null;
        if (throttleTimer) { clearTimeout(throttleTimer); throttleTimer = null; }
      }
      handleProgressEvent(e);
    } else {
      pendingEvent = e;
      if (!throttleTimer) {
        throttleTimer = setTimeout(() => {
          if (pendingEvent) {
            handleProgressEvent(pendingEvent);
            pendingEvent = null;
          }
          throttleTimer = null;
        }, THROTTLE_MS);
      }
    }
  });
}

/** Stop tracking and clean up resources. */
export function stopInstallTracking() {
  if (unlisten) {
    unlisten();
    unlisten = null;
  }
  if (timer) {
    clearInterval(timer);
    timer = null;
  }
  speedSamples = [];
  cumulativeDownloaded = 0;
  stagingSpeedSamples = [];
  stagingSizeAccumulator = 0;
  installSpeedSamples = [];
  installSizeAccumulator = 0;
  pendingEvent = null;
  if (throttleTimer) { clearTimeout(throttleTimer); throttleTimer = null; }
}

/** Mark install as finished and deactivate after a delay. */
export function dismissInstall() {
  stopInstallTracking();
  collectionInstallStatus.set(null);
}
