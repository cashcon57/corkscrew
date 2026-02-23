/**
 * Persistent install tracking service.
 *
 * Subscribes to "install-progress" Tauri events and updates the global
 * collectionInstallStatus store.  Lives at the layout level so progress
 * tracking survives page navigation.
 */
import { listen } from "@tauri-apps/api/event";
import { collectionInstallStatus } from "$lib/stores";
import type { CollectionInstallStatus, ModProgressDetail } from "$lib/stores";
import type { InstallProgressEvent } from "$lib/types";

let unlisten: (() => void) | null = null;
let timer: ReturnType<typeof setInterval> | null = null;

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
    },
    modDetails,
    startTime: now,
    elapsed: "0s",
    result: null,
    userActions: [],
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

  // Subscribe to progress events
  unlisten = await listen<InstallProgressEvent>("install-progress", (event) => {
    handleProgressEvent(event.payload);
  });
}

function handleProgressEvent(e: InstallProgressEvent) {
  collectionInstallStatus.update((s) => {
    if (!s) return s;
    const next = { ...s };

    switch (e.kind) {
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
          next.modDetails[e.mod_index] = { ...next.modDetails[e.mod_index], status: "downloading" };
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

      case "downloadProgress":
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
        next.downloadProgress = {
          ...next.downloadProgress,
          active: next.downloadProgress.active.map((d) =>
            d.modIndex === e.mod_index
              ? { ...d, downloaded: e.downloaded, total: e.total }
              : d,
          ),
        };
        break;

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
          next.modDetails[e.mod_index] = { ...next.modDetails[e.mod_index], status: "downloaded" };
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
        break;

      case "modStarted":
        next.installProgress = {
          ...next.installProgress,
          current: e.mod_index + 1,
          currentMod: e.mod_name,
          step: "preparing",
        };
        // Legacy compat
        next.currentMod = e.mod_name;
        next.current = e.mod_index + 1;
        next.total = e.total_mods;
        next.step = "preparing";

        if (next.modDetails[e.mod_index]) {
          next.modDetails = [...next.modDetails];
          next.modDetails[e.mod_index] = { ...next.modDetails[e.mod_index], status: "extracting" };
        }
        break;

      case "stepChanged":
        next.installProgress = {
          ...next.installProgress,
          step: e.step,
        };
        next.step = e.step;
        if (next.modDetails[e.mod_index]) {
          const stepStatus = e.step === "deploying" ? "deploying" as const : "extracting" as const;
          next.modDetails = [...next.modDetails];
          next.modDetails[e.mod_index] = { ...next.modDetails[e.mod_index], status: stepStatus };
        }
        break;

      case "modCompleted":
        if (next.modDetails[e.mod_index]) {
          next.modDetails = [...next.modDetails];
          next.modDetails[e.mod_index] = { ...next.modDetails[e.mod_index], status: "done" };
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
        break;
    }

    return next;
  });
}

/** Resume tracking a previously interrupted collection install. */
export async function resumeInstallTracking(
  collectionName: string,
  totalMods: number,
  completedMods: number,
  modStatuses: Record<string, string>,
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
      name: `Mod ${i + 1}`,
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
    },
    modDetails,
    startTime: now,
    elapsed: "0s",
    result: null,
    userActions: [],
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
    handleProgressEvent(event.payload);
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
}

/** Mark install as finished and deactivate after a delay. */
export function dismissInstall() {
  stopInstallTracking();
  collectionInstallStatus.set(null);
}
