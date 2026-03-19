/**
 * Persistent Wabbajack install tracking service.
 *
 * Subscribes to "wj-install-progress" Tauri events and updates the global
 * wjInstallStatus store.  Lives at the service level so progress tracking
 * survives page navigation (same pattern as installService.ts for collections).
 */
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { wjInstallStatus, wjInstallGeneration } from "$lib/stores";
import type { WjInstallStatus } from "$lib/stores";
import type { WjInstallProgressEvent, WjArchiveStatus } from "$lib/types";
import { get } from "svelte/store";
import { SpeedTracker } from "$lib/speedTracker";

let unlisten: UnlistenFn | null = null;
let elapsedTimer: ReturnType<typeof setInterval> | null = null;
let speedTracker = new SpeedTracker();

// Cumulative download tracking across all archives
let completedArchiveBytes = 0;   // Sum of total_bytes from finished archives
let currentArchiveBytes = 0;     // bytes_downloaded for the archive in progress
let currentArchiveTotalBytes = 0;// total_bytes for the archive in progress
let totalEstimatedBytes = 0;     // Best estimate of total bytes across all archives
let currentArchiveName = "";     // Name of the archive currently downloading

// Monotonic progress floor — progress can never go below this
let progressFloor = 0;

const PHASE_WEIGHTS: Record<string, [number, number]> = {
  preflight: [0, 2],
  downloading: [2, 32],
  extracting: [32, 52],
  directives: [52, 82],
  deploying: [82, 100],
  complete: [100, 100],
};

function computeOverall(phase: string, current: number, total: number): number {
  const [start, end] = PHASE_WEIGHTS[phase] || [0, 0];
  if (total <= 0) return start;
  const phaseProgress = Math.min(current / total, 1);
  return Math.round(start + (end - start) * phaseProgress);
}

function formatElapsed(startMs: number): string {
  const ms = Date.now() - startMs;
  const secs = Math.floor(ms / 1000);
  const mins = Math.floor(secs / 60);
  const hrs = Math.floor(mins / 60);
  if (hrs > 0) return `${hrs}h ${mins % 60}m`;
  if (mins > 0) return `${mins}m ${secs % 60}s`;
  return `${secs}s`;
}

function processEvent(s: WjInstallStatus, p: WjInstallProgressEvent): WjInstallStatus {
  switch (p.type) {
    case "PreFlightStarted":
      s.phase = "preflight";
      s.overallProgress = computeOverall("preflight", 0, 1);
      s.preflightNote = "Running preflight checks...";
      break;
    case "PreFlightCompleted":
      s.overallProgress = computeOverall("preflight", 1, 1);
      if (!p.report.can_proceed) {
        const issues = p.report.issues.map((i) => i.message).join("; ");
        s.preflightNote = `Preflight issues: ${issues}`;
      } else {
        s.preflightNote = `Preflight OK — ${p.report.total_archives} archives, ${p.report.cached_archives} cached`;
      }
      break;

    case "DownloadPhaseStarted":
      s.phase = "downloading";
      s.downloadProgress.total = p.total;
      s.downloadProgress.current = 0;
      s.downloadProgress.bytesDownloaded = 0;
      completedArchiveBytes = 0;
      currentArchiveBytes = 0;
      currentArchiveTotalBytes = 0;
      totalEstimatedBytes = 0;
      currentArchiveName = "";
      speedTracker.reset();
      break;
    case "DownloadStarted":
      // A new archive is starting — finalize the previous one
      if (currentArchiveName && currentArchiveTotalBytes > 0) {
        completedArchiveBytes += currentArchiveTotalBytes;
      }
      currentArchiveName = p.name;
      currentArchiveBytes = 0;
      currentArchiveTotalBytes = 0;
      s.downloadProgress.current = p.index + 1;
      s.downloadProgress.currentFile = p.name;
      break;
    case "DownloadProgress": {
      currentArchiveBytes = p.bytes;
      if (p.total_bytes > 0) currentArchiveTotalBytes = p.total_bytes;
      s.downloadProgress.currentFile = p.name;
      // Cumulative bytes for display
      const cumulativeBytes = completedArchiveBytes + currentArchiveBytes;
      s.downloadProgress.bytesDownloaded = cumulativeBytes;
      // Estimate total: assume average archive size * total count
      if (s.downloadProgress.current > 0 && currentArchiveTotalBytes > 0) {
        const avgArchiveSize = (completedArchiveBytes + currentArchiveTotalBytes) / s.downloadProgress.current;
        totalEstimatedBytes = Math.max(totalEstimatedBytes, Math.round(avgArchiveSize * s.downloadProgress.total));
      }
      s.downloadProgress.totalBytes = totalEstimatedBytes > 0 ? totalEstimatedBytes : currentArchiveTotalBytes;
      s.speed = speedTracker.update(cumulativeBytes);
      s.speedLabel = SpeedTracker.formatSpeed(s.speed);
      const remaining = Math.max(0, (totalEstimatedBytes || currentArchiveTotalBytes) - cumulativeBytes);
      s.etaLabel = SpeedTracker.formatEta(remaining, s.speed);
      s.downloadProgress.speed = s.speed;
      s.downloadProgress.eta = s.etaLabel;
      // Compute overall from cumulative bytes
      s.overallProgress = computeOverall("downloading", cumulativeBytes, totalEstimatedBytes > 0 ? totalEstimatedBytes : 1);
      break;
    }
    case "DownloadCompleted":
      // Finalize this archive's bytes
      if (currentArchiveTotalBytes > 0) {
        completedArchiveBytes += currentArchiveTotalBytes;
        currentArchiveBytes = 0;
        currentArchiveTotalBytes = 0;
        currentArchiveName = "";
      }
      break;
    case "DownloadFailed":
      // Finalize bytes so next archive doesn't regress
      if (currentArchiveTotalBytes > 0) {
        completedArchiveBytes += currentArchiveTotalBytes;
        currentArchiveBytes = 0;
        currentArchiveTotalBytes = 0;
        currentArchiveName = "";
      }
      break;
    case "DownloadSkipped":
      break;

    case "ExtractionStarted":
      s.phase = "extracting";
      s.extractionProgress.total = p.total;
      s.extractionProgress.current = 0;
      s.extractionProgress.totalBytes = p.total_bytes ?? 0;
      s.extractionProgress.bytesCompleted = 0;
      speedTracker.reset();
      break;
    case "ExtractionArchiveStarted":
      s.extractionProgress.currentArchive = p.name;
      s.archives = [
        ...s.archives.filter((a) => a.index !== p.index),
        { name: p.name, index: p.index, size: p.size, status: "extracting" as const },
      ].sort((a, b) => a.index - b.index);
      break;
    case "ExtractionArchiveCompleted":
      s.extractionProgress.current++;
      s.archives = s.archives.map((a) =>
        a.index === p.index ? { ...a, status: "extracted" as const } : a,
      );
      s.overallProgress = computeOverall("extracting", s.extractionProgress.current, s.extractionProgress.total);
      break;
    case "ExtractionArchiveFailed":
      s.archives = s.archives.map((a) =>
        a.name === p.name ? { ...a, status: "failed" as const, error: p.error } : a,
      );
      s.extractionProgress.current++;
      break;
    case "ExtractionProgress":
      // Only advance current — never regress (events may arrive out of order)
      s.extractionProgress.current = Math.max(s.extractionProgress.current, p.index + 1);
      s.extractionProgress.currentArchive = p.name;
      if (p.total_bytes > 0) {
        s.extractionProgress.bytesCompleted = Math.max(s.extractionProgress.bytesCompleted, p.bytes_completed ?? 0);
        s.extractionProgress.totalBytes = p.total_bytes;
        s.speed = speedTracker.update(s.extractionProgress.bytesCompleted);
        s.speedLabel = SpeedTracker.formatSpeed(s.speed);
        s.overallProgress = computeOverall("extracting", s.extractionProgress.current, s.extractionProgress.total);
      }
      break;

    case "DirectivePhaseStarted":
      s.phase = "directives";
      s.directiveProgress.total = p.total;
      s.directiveProgress.current = 0;
      s.directiveProgress.totalBytes = p.total_bytes ?? 0;
      s.directiveProgress.bytesProcessed = 0;
      speedTracker.reset();
      break;
    case "DirectiveProgress":
      s.directiveProgress.current = p.current;
      s.directiveProgress.bytesProcessed = p.bytes_processed;
      s.directiveProgress.totalBytes = p.total_bytes;
      s.directiveProgress.currentFile = p.current_file ?? "";
      s.directiveProgress.directiveType = p.directive_type ?? "";
      if (p.bytes_processed > 0) {
        s.speed = speedTracker.update(p.bytes_processed);
        s.speedLabel = SpeedTracker.formatSpeed(s.speed);
      }
      s.overallProgress = computeOverall("directives", p.current, p.total);
      break;

    case "DeployStarted":
      s.phase = "deploying";
      s.deployProgress.total = p.total;
      s.deployProgress.current = 0;
      s.deployProgress.totalBytes = p.total_bytes ?? 0;
      s.deployProgress.bytesDeployed = 0;
      speedTracker.reset();
      break;
    case "DeployProgress":
      s.deployProgress.current = p.current;
      if (p.bytes_deployed > 0) {
        s.deployProgress.bytesDeployed = p.bytes_deployed;
        s.speed = speedTracker.update(p.bytes_deployed);
        s.speedLabel = SpeedTracker.formatSpeed(s.speed);
      }
      s.overallProgress = computeOverall("deploying", p.current, p.total);
      break;

    case "InstallCompleted":
      s.phase = "complete";
      s.active = false;
      s.overallProgress = 100;
      s.result = {
        filesDeployed: p.result.files_deployed,
        warnings: p.result.warnings || [],
        elapsed: p.result.elapsed_secs,
      };
      // Signal mod list refresh needed (CLAUDE.md invariant: loadMods + refreshHealth after state change)
      wjInstallGeneration.update((n) => n + 1);
      break;
    case "InstallFailed":
      s.phase = "failed";
      s.active = false;
      s.error = p.error || "Unknown error";
      break;
    case "InstallCancelled":
      s.phase = "cancelled";
      s.active = false;
      break;

    case "UserActionRequired":
      // Handled elsewhere (e.g., open browser for manual download)
      break;
  }

  // Monotonic enforcement: progress can never decrease (prevents visual jitter
  // from out-of-order events, phase transitions, or per-file resets).
  if (s.overallProgress < progressFloor) {
    s.overallProgress = progressFloor;
  } else {
    progressFloor = s.overallProgress;
  }

  return s;
}

/**
 * Begin tracking a new WJ install. Call BEFORE invoking the backend command.
 * Registers the event listener first to avoid race conditions.
 */
export interface WjInstallMeta {
  readmeHtml?: string;
  description?: string;
  author?: string;
  imageUrl?: string;
  featuredMods?: Array<{ name: string; description: string }>;
}

export async function startWjInstallTracking(modlistName: string, meta?: WjInstallMeta): Promise<void> {
  // Clean up any previous tracking
  stopWjInstallTracking();

  const initial: WjInstallStatus = {
    active: true,
    modlistName,
    phase: "preflight",
    downloadProgress: { current: 0, total: 0, bytesDownloaded: 0, totalBytes: 0, currentFile: "", speed: 0, eta: "" },
    extractionProgress: { current: 0, total: 0, currentArchive: "", totalBytes: 0, bytesCompleted: 0 },
    directiveProgress: { current: 0, total: 0, bytesProcessed: 0, totalBytes: 0, currentFile: "", directiveType: "files" },
    deployProgress: { current: 0, total: 0, bytesDeployed: 0, totalBytes: 0 },
    archives: [],
    startTime: Date.now(),
    elapsed: "0s",
    overallProgress: 0,
    speed: 0,
    speedLabel: "",
    etaLabel: "",
    result: null,
    error: null,
    installId: null,
    preflightNote: "",
    readmeHtml: meta?.readmeHtml || "",
    description: meta?.description || "",
    author: meta?.author || "",
    imageUrl: meta?.imageUrl || "",
    featuredMods: meta?.featuredMods || [],
  };

  wjInstallStatus.set(initial);
  speedTracker = new SpeedTracker();
  progressFloor = 0;
  completedArchiveBytes = 0;
  currentArchiveBytes = 0;
  currentArchiveTotalBytes = 0;
  totalEstimatedBytes = 0;
  currentArchiveName = "";

  // Elapsed timer
  elapsedTimer = setInterval(() => {
    wjInstallStatus.update((s) => {
      if (!s) return s;
      return { ...s, elapsed: formatElapsed(s.startTime) };
    });
  }, 1000);

  // Register event listeners BEFORE backend command fires.
  // The installer emits on "wj-install-progress" and the downloader emits on
  // "wabbajack-install-progress" with slightly different field names.
  const updateStore = (payload: any) => {
    wjInstallStatus.update((s) => {
      if (!s) return s;
      const updated = processEvent(s, payload);
      return { ...updated };
    });
  };

  unlisten = await listen<WjInstallProgressEvent>("wj-install-progress", (e) => updateStore(e.payload));

  // Also listen to downloader-specific events (different channel + field names)
  const unlistenDl = await listen<any>("wabbajack-install-progress", (e) => {
    const p = e.payload;
    // Map downloader field names to installer field names
    const mapped: any = { type: p.type };
    if (p.archive_name !== undefined) mapped.name = p.archive_name;
    if (p.bytes_downloaded !== undefined) mapped.bytes = p.bytes_downloaded;
    if (p.total_bytes !== undefined) mapped.total_bytes = p.total_bytes;
    if (p.index !== undefined) mapped.index = p.index;
    if (p.total !== undefined) mapped.total = p.total;
    if (p.error !== undefined) mapped.error = p.error;
    if (p.reason !== undefined) mapped.reason = p.reason;
    updateStore(mapped);
  });

  // Store both unlisten fns
  const origUnlisten = unlisten;
  unlisten = () => { origUnlisten(); unlistenDl(); };
}

/** Set the install ID (returned by the backend command) for cancellation support. */
export function setWjInstallId(id: number): void {
  wjInstallStatus.update((s) => {
    if (!s) return s;
    s.installId = id;
    return s;
  });
}

/** Stop event listening and timer, but keep status visible. */
export function stopWjInstallTracking(): void {
  if (unlisten) {
    unlisten();
    unlisten = null;
  }
  if (elapsedTimer) {
    clearInterval(elapsedTimer);
    elapsedTimer = null;
  }
}

/** Fully clear the WJ install status (dismiss the UI). */
export function clearWjInstallStatus(): void {
  stopWjInstallTracking();
  wjInstallStatus.set(null);
}
