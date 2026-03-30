/**
 * Persistent Wabbajack install tracking service.
 *
 * Subscribes to "wj-install-progress" Tauri events and updates the global
 * wjInstallStatus store.  Lives at the service level so progress tracking
 * survives page navigation (same pattern as installService.ts for collections).
 */
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { wjInstallStatus, wjInstallGeneration } from "$lib/stores";
import type { WjInstallStatus, WjActiveDownload, WjLogEntry } from "$lib/stores";
import type { WjInstallProgressEvent, WjArchiveStatus } from "$lib/types";
import { get } from "svelte/store";
import { SpeedTracker } from "$lib/speedTracker";

let unlisten: UnlistenFn | null = null;
let elapsedTimer: ReturnType<typeof setInterval> | null = null;
let speedTracker = new SpeedTracker();

// ---- Concurrent download tracking ----
// Map of archive name → { index, bytes, totalBytes }
const activeDownloadMap = new Map<string, WjActiveDownload>();
let completedDownloadCount = 0;
let totalDownloadBytes = 0;   // Running sum of all known totalBytes
let downloadedBytes = 0;       // Running sum of completed + active bytes

// ---- ETA smoothing ----
let smoothedEtaSecs = 0;
const ETA_ALPHA = 0.15;  // EMA smoothing factor — lower = more stable
let lastEtaUpdateTime = 0;
const ETA_UPDATE_INTERVAL_MS = 2000; // Only update ETA display every 2s

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

function addLog(s: WjInstallStatus, message: string, level: "info" | "warn" | "error" = "info"): void {
  s.logEntries.push({ timestamp: Date.now(), message, level });
  // Cap at 500 entries to prevent memory growth
  if (s.logEntries.length > 500) {
    s.logEntries = s.logEntries.slice(-400);
  }
}

/** Recalculate cumulative download bytes from the active map + completed archives. */
function recalcCumulativeBytes(): number {
  let activeBytes = 0;
  for (const dl of activeDownloadMap.values()) {
    activeBytes += dl.bytes;
  }
  return downloadedBytes + activeBytes;
}

/** Recalculate total estimated bytes from known archive sizes. */
function recalcTotalEstimate(total: number): number {
  // Sum known sizes from active + completed
  let knownTotal = totalDownloadBytes;
  let knownCount = completedDownloadCount + activeDownloadMap.size;
  if (knownCount > 0 && knownCount < total) {
    // Project average to remaining
    const avg = knownTotal / knownCount;
    return Math.round(knownTotal + avg * (total - knownCount));
  }
  return knownTotal > 0 ? knownTotal : 1;
}

/** Compute smoothed ETA — only updates the displayed value every 2s to avoid jitter. */
function computeSmoothedEta(remainingBytes: number, speed: number): string {
  if (speed <= 0) return "";
  const rawSecs = remainingBytes / speed;
  const now = Date.now();

  // Apply EMA smoothing
  if (smoothedEtaSecs <= 0) {
    smoothedEtaSecs = rawSecs;
  } else {
    smoothedEtaSecs = ETA_ALPHA * rawSecs + (1 - ETA_ALPHA) * smoothedEtaSecs;
  }

  // Only update display string every 2s
  if (now - lastEtaUpdateTime < ETA_UPDATE_INTERVAL_MS) {
    return ""; // Signal: don't update
  }
  lastEtaUpdateTime = now;

  const secs = smoothedEtaSecs;
  if (secs < 60) return "< 1 min";
  if (secs < 3600) return `~${Math.ceil(secs / 60)} min`;
  const hrs = Math.floor(secs / 3600);
  const mins = Math.ceil((secs % 3600) / 60);
  return `~${hrs}h ${mins}m`;
}

function processEvent(s: WjInstallStatus, p: WjInstallProgressEvent): WjInstallStatus {
  switch (p.type) {
    case "PreFlightStarted":
      s.phase = "preflight";
      s.overallProgress = computeOverall("preflight", 0, 1);
      s.preflightNote = "Running preflight checks...";
      addLog(s, "Preflight checks started");
      break;
    case "PreFlightCompleted":
      s.overallProgress = computeOverall("preflight", 1, 1);
      if (!p.report.can_proceed) {
        const issues = p.report.issues.map((i) => i.message).join("; ");
        s.preflightNote = `Preflight issues: ${issues}`;
        addLog(s, `Preflight failed: ${issues}`, "error");
      } else {
        s.preflightNote = `Preflight OK — ${p.report.total_archives} archives, ${p.report.cached_archives} cached`;
        addLog(s, `Preflight OK: ${p.report.total_archives} archives (${p.report.cached_archives} cached)`);
      }
      break;

    case "DownloadPhaseStarted":
      s.phase = "downloading";
      s.downloadProgress.total = p.total;
      s.downloadProgress.current = 0;
      s.downloadProgress.completed = 0;
      s.downloadProgress.bytesDownloaded = 0;
      s.downloadProgress.totalBytes = 0;
      s.downloadProgress.maxConcurrent = p.max_concurrent ?? 4;
      s.downloadProgress.activeDownloads = [];
      activeDownloadMap.clear();
      completedDownloadCount = 0;
      totalDownloadBytes = 0;
      downloadedBytes = 0;
      smoothedEtaSecs = 0;
      lastEtaUpdateTime = 0;
      speedTracker.reset();
      addLog(s, `Download phase started: ${p.total} archives, ${p.max_concurrent ?? 4} concurrent threads`);
      break;

    case "DownloadStarted":
      // Track this as a new active download
      activeDownloadMap.set(p.name, { name: p.name, index: p.index, bytes: 0, totalBytes: 0 });
      s.downloadProgress.current = Math.max(s.downloadProgress.current, p.index + 1);
      s.downloadProgress.currentFile = p.name;
      s.downloadProgress.activeDownloads = Array.from(activeDownloadMap.values()).map(d => ({ ...d }));
      addLog(s, `Downloading: ${p.name}`);
      break;

    case "DownloadProgress": {
      const active = activeDownloadMap.get(p.name);
      if (active) {
        active.bytes = p.bytes;
        if (p.total_bytes > 0) {
          // First time we learn the total for this archive
          if (active.totalBytes === 0 && p.total_bytes > 0) {
            totalDownloadBytes += p.total_bytes;
          }
          active.totalBytes = p.total_bytes;
        }
      }

      // Update cumulative bytes and totals
      const cumulative = recalcCumulativeBytes();
      const estTotal = recalcTotalEstimate(s.downloadProgress.total);
      s.downloadProgress.bytesDownloaded = cumulative;
      s.downloadProgress.totalBytes = estTotal;
      s.downloadProgress.activeDownloads = Array.from(activeDownloadMap.values()).map(d => ({ ...d }));

      // Speed + ETA
      s.speed = speedTracker.update(cumulative);
      s.speedLabel = SpeedTracker.formatSpeed(s.speed);
      const remaining = Math.max(0, estTotal - cumulative);
      const etaStr = computeSmoothedEta(remaining, s.speed);
      if (etaStr !== "") {
        s.etaLabel = etaStr;
      }
      s.downloadProgress.speed = s.speed;
      s.downloadProgress.eta = s.etaLabel;

      // Overall progress from bytes
      s.overallProgress = computeOverall("downloading", cumulative, estTotal);
      break;
    }

    case "DownloadCompleted": {
      const completed = activeDownloadMap.get(p.name);
      if (completed) {
        // Move bytes from active to completed pool
        downloadedBytes += completed.totalBytes > 0 ? completed.totalBytes : completed.bytes;
        activeDownloadMap.delete(p.name);
        completedDownloadCount++;
      }
      s.downloadProgress.completed = completedDownloadCount;
      s.downloadProgress.activeDownloads = Array.from(activeDownloadMap.values()).map(d => ({ ...d }));
      addLog(s, `Downloaded: ${p.name}`);
      break;
    }

    case "DownloadFailed": {
      const failed = activeDownloadMap.get(p.name);
      if (failed) {
        // Count the total bytes toward estimate even on failure, so ETA doesn't regress
        if (failed.totalBytes > 0) {
          downloadedBytes += failed.totalBytes;
        }
        activeDownloadMap.delete(p.name);
        completedDownloadCount++;
      }
      s.downloadProgress.completed = completedDownloadCount;
      s.downloadProgress.activeDownloads = Array.from(activeDownloadMap.values()).map(d => ({ ...d }));
      addLog(s, `Download failed: ${p.name} — ${p.error}`, "error");
      break;
    }

    case "DownloadSkipped":
      completedDownloadCount++;
      s.downloadProgress.completed = completedDownloadCount;
      addLog(s, `Skipped: ${p.name} (${p.reason})`);
      break;

    case "DownloadPhaseCompleted":
      addLog(s, `Download phase complete: ${p.succeeded} succeeded, ${p.failed} failed, ${p.skipped} cached`);
      if (p.failed > 0) {
        addLog(s, `${p.failed} downloads failed — affected files will be missing from install`, "warn");
        for (const failure of (p.failures || []).slice(0, 10)) {
          addLog(s, `  ${failure}`, "error");
        }
      }
      break;

    case "ExtractionStarted":
      s.phase = "extracting";
      s.extractionProgress.total = p.total;
      s.extractionProgress.current = 0;
      s.extractionProgress.totalBytes = p.total_bytes ?? 0;
      s.extractionProgress.bytesCompleted = 0;
      speedTracker.reset();
      smoothedEtaSecs = 0;
      lastEtaUpdateTime = 0;
      addLog(s, `Extraction phase started: ${p.total} archives`);
      break;
    case "ExtractionArchiveStarted":
      s.extractionProgress.currentArchive = p.name;
      s.archives = [
        ...s.archives.filter((a) => a.index !== p.index),
        { name: p.name, index: p.index, size: p.size, status: "extracting" as const },
      ].sort((a, b) => a.index - b.index);
      addLog(s, `Extracting: ${p.name}`);
      break;
    case "ExtractionArchiveCompleted":
      s.extractionProgress.current++;
      s.archives = s.archives.map((a) =>
        a.index === p.index ? { ...a, status: "extracted" as const } : a,
      );
      s.overallProgress = computeOverall("extracting", s.extractionProgress.current, s.extractionProgress.total);
      addLog(s, `Extracted: ${p.name}`);
      break;
    case "ExtractionArchiveFailed":
      s.archives = s.archives.map((a) =>
        a.name === p.name ? { ...a, status: "failed" as const, error: p.error } : a,
      );
      s.extractionProgress.current++;
      addLog(s, `Extraction failed: ${p.name} — ${p.error}`, "error");
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
      smoothedEtaSecs = 0;
      lastEtaUpdateTime = 0;
      addLog(s, `Directive phase started: ${p.total.toLocaleString()} directives`);
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
      smoothedEtaSecs = 0;
      lastEtaUpdateTime = 0;
      addLog(s, `Deploy phase started: ${p.total.toLocaleString()} files`);
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
      addLog(s, `Installation complete: ${p.result.files_deployed.toLocaleString()} files deployed`);
      // Signal mod list refresh needed (CLAUDE.md invariant: loadMods + refreshHealth after state change)
      wjInstallGeneration.update((n) => n + 1);
      break;
    case "InstallFailed":
      s.phase = "failed";
      s.active = false;
      s.error = p.error || "Unknown error";
      addLog(s, `Installation failed: ${p.error}`, "error");
      break;
    case "InstallCancelled":
      s.phase = "cancelled";
      s.active = false;
      addLog(s, "Installation cancelled", "warn");
      break;

    case "UserActionRequired":
      // Handled elsewhere (e.g., open browser for manual download)
      addLog(s, `Manual download required: ${p.archive_name}`, "warn");
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
    downloadProgress: { current: 0, total: 0, completed: 0, bytesDownloaded: 0, totalBytes: 0, currentFile: "", speed: 0, eta: "", maxConcurrent: 0, activeDownloads: [] },
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
    logEntries: [],
    readmeHtml: meta?.readmeHtml || "",
    description: meta?.description || "",
    author: meta?.author || "",
    imageUrl: meta?.imageUrl || "",
    featuredMods: meta?.featuredMods || [],
  };

  wjInstallStatus.set(initial);
  speedTracker = new SpeedTracker();
  progressFloor = 0;
  activeDownloadMap.clear();
  completedDownloadCount = 0;
  totalDownloadBytes = 0;
  downloadedBytes = 0;
  smoothedEtaSecs = 0;
  lastEtaUpdateTime = 0;

  // Elapsed timer
  elapsedTimer = setInterval(() => {
    wjInstallStatus.update((s) => {
      if (!s) return s;
      return { ...s, elapsed: formatElapsed(s.startTime) };
    });
  }, 1000);

  // Register event listeners BEFORE backend command fires.
  // Events are deduplicated (only latest DownloadProgress per archive kept)
  // and flushed at ~4 Hz to prevent UI thread starvation from high-frequency
  // backend events (16 concurrent downloads × hundreds of MB/s = thousands/sec).
  let eventQueue: any[] = [];
  let progressLatest = new Map<string, any>(); // archive_name → latest DownloadProgress
  let flushTimer: ReturnType<typeof setTimeout> | null = null;
  const FLUSH_INTERVAL_MS = 250; // 4 Hz — fast enough for smooth UI, slow enough to not block

  function flushEventQueue() {
    flushTimer = null;
    // Merge deduped progress events back into queue
    const deduped = [...eventQueue, ...progressLatest.values()];
    eventQueue = [];
    progressLatest.clear();
    if (deduped.length === 0) return;

    wjInstallStatus.update((s) => {
      if (!s) return s;
      let updated = s;
      for (const payload of deduped) {
        updated = processEvent(updated, payload);
      }
      return { ...updated };
    });
  }

  function enqueueEvent(payload: any) {
    // Deduplicate DownloadProgress: only keep latest per archive
    if (payload.type === "DownloadProgress" && payload.name) {
      progressLatest.set(payload.name, payload);
    } else {
      eventQueue.push(payload);
    }
    if (flushTimer === null) {
      flushTimer = setTimeout(flushEventQueue, FLUSH_INTERVAL_MS);
    }
  }

  unlisten = await listen<WjInstallProgressEvent>("wj-install-progress", (e) => enqueueEvent(e.payload));

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
    if (p.max_concurrent !== undefined) mapped.max_concurrent = p.max_concurrent;
    enqueueEvent(mapped);
  });

  // Store both unlisten fns — also cancel any pending flush timer
  const origUnlisten = unlisten;
  unlisten = () => {
    origUnlisten();
    unlistenDl();
    if (flushTimer !== null) { clearTimeout(flushTimer); flushTimer = null; }
  };
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
