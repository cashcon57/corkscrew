<script lang="ts">
  import { goto } from "$app/navigation";
  import { untrack } from "svelte";
  import { collectionInstallStatus } from "$lib/stores";
  import type { PendingFomod } from "$lib/stores";
  import { dismissInstall } from "$lib/installService";
  import { cancelCollectionInstall, submitFomodChoices } from "$lib/api";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { marked } from "marked";
  import DOMPurify from "dompurify";
  import FomodWizard from "$lib/components/FomodWizard.svelte";

  let modLogExpanded = $state(false);
  let modLogAutoExpanded = $state(false);
  let userActionsExpanded = $state(true);
  let verboseLogExpanded = $state(false);
  let verboseLogEl: HTMLDivElement | undefined = $state();

  // FOMOD wizard state
  let activeFomod = $state<PendingFomod | null>(null);

  let status = $derived($collectionInstallStatus);
  let isActive = $derived(status?.active ?? false);
  let phase = $derived(status?.phase ?? "");
  let dl = $derived(status?.downloadProgress ?? { total: 0, completed: 0, failed: 0, cached: 0, maxConcurrent: 0, active: [] });
  let inst = $derived(status?.installProgress ?? { current: 0, total: 0, currentMod: "", step: "", stepDetail: "" });
  let mods = $derived(status?.modDetails ?? []);
  let actions = $derived(status?.userActions ?? []);
  let result = $derived(status?.result ?? null);
  let overallProgress = $derived(status?.overallProgress ?? 0);
  let downloadSpeed = $derived(status?.downloadSpeed ?? 0);
  let downloadEta = $derived(status?.downloadEta ?? "");
  let stagingSpeed = $derived(status?.stagingSpeed ?? 0);
  let installSpeed = $derived(status?.installSpeed ?? 0);

  let logEntries = $derived(status?.logEntries ?? []);
  let pendingFomods = $derived(status?.pendingFomods ?? []);
  let collectionDescription = $derived(status?.collectionDescription ?? "");
  let descriptionExpanded = $state(false);
  let renderedDescription = $derived(
    collectionDescription ? DOMPurify.sanitize(marked.parse(collectionDescription) as string) : "",
  );

  let dlPercent = $derived(dl.total > 0 ? Math.round((dl.completed / dl.total) * 100) : 0);
  let instPercent = $derived(inst.total > 0 ? Math.round((inst.current / inst.total) * 100) : 0);

  // Failed mods — for error summary display
  let failedMods = $derived(mods.filter((m) => m.status === "failed"));

  // Queued downloads — mods waiting to start downloading
  let queuedCount = $derived(mods.filter((m) => m.status === "queued").length);

  // Stall detection — if downloading phase but 0 speed for extended period
  let stallTimestamp = $state<number>(0);
  let isStalled = $state(false);

  $effect(() => {
    // Depend on reactive inputs only; read own state via untrack to avoid loop
    const _phase = phase;
    const _dlActive = dl.active.length;
    const _speed = downloadSpeed;
    if (_phase !== "downloading" || _dlActive === 0) {
      stallTimestamp = 0;
      isStalled = false;
      return;
    }
    if (_speed === 0) {
      const ts = untrack(() => stallTimestamp);
      if (ts === 0) {
        stallTimestamp = Date.now();
      } else if (Date.now() - ts > 30_000) {
        isStalled = true;
      }
    } else {
      stallTimestamp = 0;
      isStalled = false;
    }
  });

  // Staging progress — count only mods that have completed extraction (past extracting phase)
  let extractingMods = $derived(mods.filter((m) => m.status === "extracting"));
  let stagingCount = $derived(extractingMods.length);
  let stagingDone = $derived(
    mods.filter((m) => ["staged", "deploying", "done", "failed", "skipped", "user_action"].includes(m.status)).length,
  );
  let stagingTotal = $derived(mods.length);
  let stagingPercent = $derived(stagingTotal > 0 ? Math.round((stagingDone / stagingTotal) * 100) : 0);

  // Activity feed — currently active mods surfaced to the top
  const ACTIVE_STATUSES = new Set(["downloading", "extracting", "installing", "deploying"]);
  let activeWorkMods = $derived(mods.filter((m) => ACTIVE_STATUSES.has(m.status)));

  // Mod log summary stats
  let modsDone = $derived(mods.filter((m) => m.status === "done").length);
  let modsFailed = $derived(mods.filter((m) => m.status === "failed").length);
  let modsSkipped = $derived(mods.filter((m) => m.status === "skipped" || m.status === "user_action").length);
  let modsPending = $derived(mods.filter((m) => ["pending", "queued"].includes(m.status)).length);

  // Recently completed — track mods that just finished for brief display in activity
  let recentlyCompleted = $state<{ name: string; index: number; status: string; timestamp: number }[]>([]);
  let prevModStatuses = $state<Map<number, string>>(new Map());
  let recentTick = $state(0);

  $effect(() => {
    // Only depend on `mods`; read own state via untrack to avoid loop
    const currentMods = mods;
    const now = Date.now();
    const newRecent: typeof recentlyCompleted = [];

    const prevMap = untrack(() => prevModStatuses);
    for (const mod of currentMods) {
      const prev = prevMap.get(mod.index);
      if (prev && ACTIVE_STATUSES.has(prev) && !ACTIVE_STATUSES.has(mod.status)) {
        newRecent.push({ name: mod.name, index: mod.index, status: mod.status, timestamp: now });
      }
    }

    const oldRecent = untrack(() => recentlyCompleted);
    if (newRecent.length > 0) {
      recentlyCompleted = [...oldRecent.filter((r) => now - r.timestamp < 3000), ...newRecent].slice(-5);
    } else {
      recentlyCompleted = oldRecent.filter((r) => now - r.timestamp < 3000);
    }

    // Update previous statuses
    const nextMap = new Map<number, string>();
    for (const mod of currentMods) {
      nextMap.set(mod.index, mod.status);
    }
    prevModStatuses = nextMap;
  });

  // Auto-expand mod log when first failure occurs
  $effect(() => {
    if (modsFailed > 0 && !modLogAutoExpanded) {
      modLogExpanded = true;
      modLogAutoExpanded = true;
    }
  });

  // Tick timer to drive opacity fading on recently-completed items
  $effect(() => {
    if (recentlyCompleted.length === 0) return;
    const interval = setInterval(() => { recentTick++; }, 200);
    return () => clearInterval(interval);
  });

  // Activity pulse — modulate animation speed by event throughput
  let eventTimestamps = $state<number[]>([]);
  let pulseSpeed = $state(0); // 0 = stopped, higher = faster (events/min)

  // Track event throughput via a plain (non-reactive) array to avoid $effect loops
  let _eventTsArray: number[] = [];

  $effect(() => {
    // Only depend on logEntries.length; timestamps are non-reactive
    const count = logEntries.length;
    if (count > 0) {
      const now = Date.now();
      _eventTsArray = [..._eventTsArray.filter((t) => now - t < 60_000), now];
      eventTimestamps = _eventTsArray;
      pulseSpeed = _eventTsArray.length;
    }
  });

  // Fade pulse to 0 if no events for 10s
  $effect(() => {
    const _phase = phase;
    if (_phase === "complete" || _phase === "failed" || _phase === "") {
      pulseSpeed = 0;
      return;
    }
    const interval = setInterval(() => {
      const now = Date.now();
      _eventTsArray = _eventTsArray.filter((t) => now - t < 10_000);
      if (_eventTsArray.length === 0) pulseSpeed = 0;
    }, 2000);
    return () => clearInterval(interval);
  });

  // Derived animation duration: fast = 0.6s, slow = 3s, stopped = 0
  let pulseDuration = $derived(
    pulseSpeed === 0 ? 0 : Math.max(0.6, 3 - (pulseSpeed / 30) * 2.4),
  );

  // Auto-scroll verbose log to bottom
  $effect(() => {
    if (verboseLogExpanded && verboseLogEl && logEntries.length > 0) {
      requestAnimationFrame(() => {
        if (verboseLogEl) verboseLogEl.scrollTop = verboseLogEl.scrollHeight;
      });
    }
  });

  // Mod log: show 10 items when collapsed, all when expanded
  let visibleMods = $derived(modLogExpanded ? mods : mods.slice(0, 10));

  // Phase timeline
  const phases = [
    { id: "downloading", label: "Download" },
    { id: "staging", label: "Extract" },
    { id: "installing", label: "Install" },
    { id: "complete", label: "Done" },
  ] as const;
  const phaseOrder = ["downloading", "staging", "installing", "complete", "failed"];

  function formatBytes(bytes: number): string {
    if (bytes >= 1_073_741_824) return (bytes / 1_073_741_824).toFixed(1) + " GB";
    if (bytes >= 1_048_576) return (bytes / 1_048_576).toFixed(1) + " MB";
    if (bytes >= 1024) return (bytes / 1024).toFixed(1) + " KB";
    return bytes + " B";
  }

  function dlItemPercent(item: { downloaded: number; total: number }): number {
    if (item.total <= 0) return 0;
    return Math.min(100, Math.round((item.downloaded / item.total) * 100));
  }

  function safeOpenUrl(url: string | null | undefined) {
    if (!url) return;
    try {
      const parsed = new URL(url);
      if (parsed.protocol === "http:" || parsed.protocol === "https:") {
        openUrl(url);
      }
    } catch { /* ignore */ }
  }

  function formatLogTime(ts: number): string {
    const d = new Date(ts);
    return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  }

  let showCancelConfirm = $state(false);

  function handleCancel() {
    dismissInstall();
    goto("/collections").catch(() => {
      window.location.href = "/collections";
    });
  }

  async function handleCancelInstall() {
    showCancelConfirm = false;
    // Signal the backend to stop the install loop
    try {
      await cancelCollectionInstall();
    } catch { /* best effort */ }
    // Update the UI to reflect cancellation
    collectionInstallStatus.update(s => s ? { ...s, phase: "failed" as const } : s);
  }

  function humanizeStep(step: string, detail: string): string {
    if (detail) return detail;
    switch (step) {
      case "preparing": return "Preparing...";
      case "extracting": return "Extracting archive...";
      case "deploying": return "Deploying files...";
      case "registering": return "Registering in database...";
      case "resuming": return "Resuming...";
      default: return step;
    }
  }
</script>

{#if !status || !isActive}
  <!-- No active install -->
  <div class="progress-page">
    <div class="empty-state">
      <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="12" cy="12" r="10" />
        <line x1="12" y1="8" x2="12" y2="12" />
        <line x1="12" y1="16" x2="12.01" y2="16" />
      </svg>
      <p class="empty-title">No active installation</p>
      <button class="btn btn-primary" onclick={() => goto('/collections').catch(() => { window.location.href = '/collections'; })}>Back to Collections</button>
    </div>
  </div>
{:else}
  <div class="progress-page">
    <!-- Header -->
    <header class="page-header">
      <div class="header-left">
        <button class="btn btn-ghost" onclick={() => goto('/collections').catch(() => { window.location.href = '/collections'; })}>
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="15 18 9 12 15 6" />
          </svg>
          Back
        </button>
        <h1 class="page-title">
          {#if phase === "complete"}
            Installation Complete
          {:else if phase === "failed"}
            Installation Failed
          {:else}
            Installing '{status.collectionName}'
          {/if}
        </h1>
      </div>
      <div class="header-right">
        <span class="elapsed-badge">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="10" />
            <polyline points="12 6 12 12 16 14" />
          </svg>
          {status.elapsed}
        </span>
        {#if phase !== "complete" && phase !== "failed"}
          <button class="btn btn-ghost cancel-install-btn" onclick={() => showCancelConfirm = true}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="12" cy="12" r="10" />
              <line x1="15" y1="9" x2="9" y2="15" />
              <line x1="9" y1="9" x2="15" y2="15" />
            </svg>
            Cancel
          </button>
        {/if}
      </div>
    </header>

    <!-- Collection Description -->
    {#if renderedDescription}
      <section class="phase-section description-section">
        <button class="collapsible-header" onclick={() => descriptionExpanded = !descriptionExpanded}>
          <h3 class="phase-title">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
              <polyline points="14 2 14 8 20 8" />
              <line x1="16" y1="13" x2="8" y2="13" />
              <line x1="16" y1="17" x2="8" y2="17" />
              <polyline points="10 9 9 9 8 9" />
            </svg>
            ABOUT THIS COLLECTION
          </h3>
          <svg class="chevron" class:expanded={descriptionExpanded} width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="6 9 12 15 18 9" />
          </svg>
        </button>
        {#if descriptionExpanded}
          <div class="description-content rendered-markdown">
            {@html renderedDescription}
          </div>
        {/if}
      </section>
    {/if}

    {#if phase === "complete"}
      <!-- Completion Panel -->
      <div class="completion-panel">
        <div class="completion-icon">
          {#if (result?.failed ?? 0) > 0}
            <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="#f59e0b" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
              <line x1="12" y1="9" x2="12" y2="13" />
              <line x1="12" y1="17" x2="12.01" y2="17" />
            </svg>
          {:else}
            <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="#22c55e" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14" />
              <polyline points="22 4 12 14.01 9 11.01" />
            </svg>
          {/if}
        </div>
        <h2 class="completion-title">
          {#if (result?.failed ?? 0) > 0}
            Installed with errors
          {:else}
            Collection installed successfully
          {/if}
        </h2>
        <div class="completion-stats">
          <div class="stat-chip stat-success">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg>
            {result?.installed ?? 0} installed
          </div>
          <div class="stat-chip stat-skip">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12h14" /></svg>
            {result?.skipped ?? 0} skipped
          </div>
          {#if (result?.failed ?? 0) > 0}
            <div class="stat-chip stat-fail">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
              {result?.failed ?? 0} failed
            </div>
          {/if}
        </div>
        {#if (result?.failed ?? 0) > 0}
          <div class="error-summary">
            <h4 class="error-summary-title">Failed Mods</h4>
            {#each failedMods.slice(0, 5) as mod}
              <div class="error-summary-item">
                <span class="error-mod-name">{mod.name}</span>
                {#if mod.error}
                  <span class="error-mod-reason">{mod.error}</span>
                {/if}
              </div>
            {/each}
            {#if failedMods.length > 5}
              <span class="error-more">+{failedMods.length - 5} more — check Mod Log below</span>
            {/if}
          </div>
        {/if}
        <p class="completion-elapsed">Total time: {status.elapsed}</p>
        <div class="completion-actions">
          <button class="btn btn-primary" onclick={() => { dismissInstall(); goto('/mods').catch(() => { window.location.href = '/mods'; }); }}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="3" width="20" height="14" rx="2" ry="2" /><line x1="8" y1="21" x2="16" y2="21" /><line x1="12" y1="17" x2="12" y2="21" /></svg>
            View Mods
          </button>
          <button class="btn btn-secondary" onclick={() => { dismissInstall(); goto('/plugins').catch(() => { window.location.href = '/plugins'; }); }}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="8" y1="6" x2="21" y2="6" /><line x1="8" y1="12" x2="21" y2="12" /><line x1="8" y1="18" x2="21" y2="18" /><line x1="3" y1="6" x2="3.01" y2="6" /><line x1="3" y1="12" x2="3.01" y2="12" /><line x1="3" y1="18" x2="3.01" y2="18" /></svg>
            Load Order
          </button>
          <button class="btn btn-ghost" onclick={() => { dismissInstall(); goto('/collections').catch(() => { window.location.href = '/collections'; }); }}>
            Back to Collections
          </button>
        </div>
      </div>
    {:else if phase === "failed"}
      <!-- Failed Panel -->
      <div class="completion-panel failed-panel">
        <div class="completion-icon">
          <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="#ef4444" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="10" />
            <line x1="15" y1="9" x2="9" y2="15" />
            <line x1="9" y1="9" x2="15" y2="15" />
          </svg>
        </div>
        <h2 class="completion-title">Installation Failed</h2>
        <div class="completion-stats">
          {#if modsDone > 0}
            <div class="stat-chip stat-success">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg>
              {modsDone} completed
            </div>
          {/if}
          {#if modsFailed > 0}
            <div class="stat-chip stat-fail">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
              {modsFailed} failed
            </div>
          {/if}
          {#if modsPending > 0}
            <div class="stat-chip chip-pending">
              {modsPending} not attempted
            </div>
          {/if}
        </div>
        {#if failedMods.length > 0}
          <div class="error-summary">
            <h4 class="error-summary-title">Errors</h4>
            {#each failedMods.slice(0, 8) as mod}
              <div class="error-summary-item">
                <span class="error-mod-name">{mod.name}</span>
                {#if mod.error}
                  <span class="error-mod-reason">{mod.error}</span>
                {/if}
              </div>
            {/each}
            {#if failedMods.length > 8}
              <span class="error-more">+{failedMods.length - 8} more errors</span>
            {/if}
          </div>
        {/if}
        <p class="completion-elapsed">Time elapsed: {status.elapsed}</p>
        <div class="completion-actions">
          <button class="btn btn-primary" onclick={() => { dismissInstall(); goto('/collections').catch(() => { window.location.href = '/collections'; }); }}>
            Back to Collections
          </button>
          <button class="btn btn-ghost" onclick={handleCancel}>
            Dismiss
          </button>
        </div>
      </div>
    {:else}
      <!-- Phase Timeline -->
      <div class="phase-timeline">
        {#each phases as step, i}
          {@const currentIdx = phaseOrder.indexOf(phase)}
          {@const stepIdx = phaseOrder.indexOf(step.id)}
          {@const isActivePhase = step.id === phase}
          {@const isDone = stepIdx < currentIdx}
          <div class="timeline-step" class:active={isActivePhase} class:done={isDone} class:future={stepIdx > currentIdx}>
            <div class="timeline-dot">
              {#if isDone}
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg>
              {:else if isActivePhase}
                <div class="timeline-pulse"></div>
              {/if}
            </div>
            <span class="timeline-label">{step.label}</span>
          </div>
          {#if i < phases.length - 1}
            <div class="timeline-connector" class:done={stepIdx < currentIdx}></div>
          {/if}
        {/each}
      </div>

      <!-- Overall Progress -->
      <section class="overall-progress-section">
        <div class="overall-header">
          <div class="overall-left">
            {#if pulseDuration > 0}
              <span class="activity-orb" style="--orb-duration: {pulseDuration}s">
                <span class="activity-orb-inner"></span>
              </span>
            {:else}
              <span class="activity-orb activity-orb-idle">
                <span class="activity-orb-inner"></span>
              </span>
            {/if}
            <span class="overall-label">OVERALL PROGRESS</span>
          </div>
          <span class="overall-percent">{overallProgress}%</span>
        </div>
        <div class="progress-track progress-track-lg">
          <div class="progress-fill progress-fill-overall" class:progress-active={pulseDuration > 0} style="width: {overallProgress}%"></div>
        </div>
      </section>

      <!-- Download Phase -->
      <section class="phase-section">
        <div class="phase-header">
          <h3 class="phase-title">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
              <polyline points="7 10 12 15 17 10" />
              <line x1="12" y1="15" x2="12" y2="3" />
            </svg>
            DOWNLOADS
          </h3>
          <span class="phase-count">{dl.completed} / {dl.total}</span>
          {#if dl.cached > 0}
            <span class="cache-badge">{dl.cached} cached</span>
          {/if}
          {#if dl.failed > 0}
            <span class="fail-badge">{dl.failed} failed</span>
          {/if}
          {#if queuedCount > 0 && phase === "downloading"}
            <span class="queued-badge">{queuedCount} queued</span>
          {/if}
          {#if downloadSpeed > 0 && phase === "downloading"}
            <span class="speed-badge">{formatBytes(downloadSpeed)}/s</span>
          {/if}
          {#if downloadEta && phase === "downloading"}
            <span class="eta-badge">{downloadEta}</span>
          {/if}
          {#if isStalled}
            <span class="stall-badge">Stalled</span>
          {/if}
        </div>
        <div class="progress-track">
          <div class="progress-fill" class:progress-active={phase === "downloading" && dl.active.length > 0} style="width: {dlPercent}%"></div>
        </div>

        {#if dl.active.length > 0}
          <div class="active-downloads">
            <div class="sub-header">
              <span class="sub-title">Active Downloads</span>
              <span class="concurrency-badge">{dl.maxConcurrent} concurrent threads</span>
            </div>
            {#each dl.active as item (item.modIndex)}
              <div class="download-item">
                <div class="dl-info">
                  <span class="dl-name" title={item.modName}>{item.modName}</span>
                  <span class="dl-bytes">
                    {formatBytes(item.downloaded)} / {item.total > 0 ? formatBytes(item.total) : "..."}
                  </span>
                </div>
                <div class="progress-track progress-track-sm">
                  <div class="progress-fill" style="width: {dlItemPercent(item)}%"></div>
                </div>
              </div>
            {/each}
          </div>
        {/if}
      </section>

      <!-- Staging Phase -->
      {#if phase === "staging" || phase === "installing"}
        <section class="phase-section">
          <div class="phase-header">
            <h3 class="phase-title">
              <svg class:icon-spin={stagingDone < stagingTotal} width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <circle cx="12" cy="12" r="3" />
                <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
              </svg>
              EXTRACTION
            </h3>
            <span class="phase-count">{stagingDone} / {stagingTotal}</span>
            {#if stagingCount > 0}
              <span class="cache-badge">{stagingCount} extracting</span>
            {/if}
            {#if stagingSpeed > 0 && stagingDone < stagingTotal}
              <span class="speed-badge">{formatBytes(stagingSpeed)}/s</span>
            {/if}
          </div>
          <div class="progress-track">
            <div class="progress-fill" class:progress-active={stagingDone < stagingTotal} style="width: {stagingPercent}%"></div>
          </div>
          {#if extractingMods.length > 0}
            <div class="extracting-list">
              {#each extractingMods as mod (mod.index)}
                <div class="extracting-item-wrap">
                  <div class="extracting-item">
                    <svg class="icon-spin" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="var(--system-accent)" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                      <circle cx="12" cy="12" r="3" />
                      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
                    </svg>
                    <span class="extracting-name" title={mod.name}>{mod.name}</span>
                    {#if mod.stepDetail}
                      <span class="extracting-detail">{mod.stepDetail}</span>
                    {/if}
                  </div>
                  <div class="extracting-bar">
                    <div class="extracting-bar-fill"></div>
                  </div>
                </div>
              {/each}
            </div>
          {/if}
        </section>
      {/if}

      <!-- Install Phase -->
      {#if phase === "staging" || phase === "installing"}
        <section class="phase-section">
          <div class="phase-header">
            <h3 class="phase-title">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z" />
              </svg>
              INSTALL
            </h3>
            <span class="phase-count">{modsDone} / {mods.length}</span>
            {#if installSpeed > 0}
              <span class="speed-badge">{formatBytes(installSpeed)}/s</span>
            {/if}
          </div>
          <div class="progress-track">
            <div class="progress-fill" class:progress-active={phase === "installing"} style="width: {mods.length > 0 ? Math.round((modsDone / mods.length) * 100) : 0}%"></div>
          </div>
          {#if inst.currentMod}
            <div class="install-detail">
              <span class="current-mod" title={inst.currentMod}>{inst.currentMod}</span>
              <span class="current-step">{humanizeStep(inst.step, inst.stepDetail)}</span>
            </div>
          {/if}
        </section>
      {/if}
    {/if}

    <!-- FOMOD Attention Banner -->
    {#if pendingFomods.length > 0}
      <section class="phase-section fomod-attention-section">
        <div class="fomod-attention-banner">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="#a78bfa" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="3" />
            <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
          </svg>
          <span class="fomod-attention-text">
            {pendingFomods.length} mod{pendingFomods.length > 1 ? "s" : ""} need{pendingFomods.length === 1 ? "s" : ""} FOMOD configuration
          </span>
          <div class="fomod-attention-actions">
            {#each pendingFomods as fomod (fomod.correlationId)}
              <button class="btn btn-secondary btn-sm" onclick={() => { activeFomod = fomod; }}>
                {fomod.modName}
              </button>
            {/each}
          </div>
        </div>
      </section>
    {/if}

    <!-- User Actions -->
    {#if actions.length > 0}
      <section class="phase-section actions-section">
        <button class="collapsible-header" onclick={() => userActionsExpanded = !userActionsExpanded}>
          <h3 class="phase-title">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="#f59e0b" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
              <line x1="12" y1="9" x2="12" y2="13" />
              <line x1="12" y1="17" x2="12.01" y2="17" />
            </svg>
            USER ACTIONS REQUIRED
          </h3>
          <span class="action-count">{actions.length}</span>
          <svg class="chevron" class:expanded={userActionsExpanded} width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="6 9 12 15 18 9" />
          </svg>
        </button>
        {#if userActionsExpanded}
          <div class="actions-list">
            {#each actions as action}
              <div class="action-item">
                <div class="action-info">
                  <span class="action-mod">{action.modName}</span>
                  <span class="action-desc">{action.action}</span>
                  {#if action.instructions}
                    <span class="action-instructions">{action.instructions}</span>
                  {/if}
                </div>
                {#if action.url}
                  <button class="btn btn-secondary btn-sm" onclick={() => safeOpenUrl(action.url)}>
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
                      <polyline points="15 3 21 3 21 9" />
                      <line x1="10" y1="14" x2="21" y2="3" />
                    </svg>
                    Open
                  </button>
                {/if}
              </div>
            {/each}
          </div>
        {/if}
      </section>
    {/if}

    <!-- Activity Feed — currently active items + recently completed -->
    {#if (activeWorkMods.length > 0 || recentlyCompleted.length > 0) && phase !== "complete"}
      <section class="phase-section activity-section">
        <div class="activity-header">
          <h3 class="phase-title">
            <svg class="icon-spin" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="var(--system-accent)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <line x1="12" y1="2" x2="12" y2="6" />
              <line x1="12" y1="18" x2="12" y2="22" />
              <line x1="4.93" y1="4.93" x2="7.76" y2="7.76" />
              <line x1="16.24" y1="16.24" x2="19.07" y2="19.07" />
              <line x1="2" y1="12" x2="6" y2="12" />
              <line x1="18" y1="12" x2="22" y2="12" />
              <line x1="4.93" y1="19.07" x2="7.76" y2="16.24" />
              <line x1="16.24" y1="7.76" x2="19.07" y2="4.93" />
            </svg>
            ACTIVITY
          </h3>
          {#if activeWorkMods.length > 0}
            <span class="activity-count">{activeWorkMods.length} active</span>
          {/if}
        </div>
        <div class="activity-list">
          <!-- Currently active mods -->
          {#each activeWorkMods as mod (mod.index)}
            <div class="activity-item">
              <span class="activity-icon">
                {#if mod.status === "downloading"}
                  <svg class="icon-bounce" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--system-accent)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" /><polyline points="7 10 12 15 17 10" /><line x1="12" y1="15" x2="12" y2="3" /></svg>
                {:else if mod.status === "extracting" || mod.status === "installing"}
                  <svg class="icon-spin" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--system-accent)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" /></svg>
                {:else if mod.status === "deploying"}
                  <svg class="icon-bounce" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--system-accent)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" /><polyline points="17 8 12 3 7 8" /><line x1="12" y1="3" x2="12" y2="15" /></svg>
                {/if}
              </span>
              <span class="activity-name" title={mod.name}>{mod.name}</span>
              <span class="activity-status">
                {#if mod.status === "downloading" && mod.downloadBytes && mod.downloadTotal}
                  {formatBytes(mod.downloadBytes)} / {formatBytes(mod.downloadTotal)}
                  {#if downloadSpeed > 0}
                    <span class="speed-inline">{formatBytes(downloadSpeed)}/s</span>
                  {/if}
                {:else if mod.status === "extracting"}
                  extracting...
                  {#if stagingSpeed > 0}
                    <span class="speed-inline">{formatBytes(stagingSpeed)}/s</span>
                  {/if}
                {:else if mod.status === "deploying"}
                  deploying files...
                {:else if mod.status === "installing"}
                  installing...
                {:else if mod.stepDetail}
                  {mod.stepDetail}
                {:else}
                  {mod.status}
                {/if}
              </span>
              {#if mod.status === "downloading" && mod.downloadTotal && mod.downloadTotal > 0}
                <div class="activity-bar">
                  <div class="activity-bar-fill" style="width: {Math.min(100, Math.round(((mod.downloadBytes ?? 0) / mod.downloadTotal) * 100))}%"></div>
                </div>
              {:else if mod.status === "extracting" || mod.status === "installing" || mod.status === "deploying"}
                <div class="activity-bar">
                  <div class="activity-bar-fill activity-bar-indeterminate"></div>
                </div>
              {/if}
            </div>
          {/each}
          <!-- Recently completed mods (fade out after 3s) -->
          {#each recentlyCompleted as recent (recent.index)}
            {@const _tick = recentTick}
            {@const age = Date.now() - recent.timestamp}
            {@const opacity = Math.max(0, 1 - age / 3000)}
            <div class="activity-item activity-item-recent" style="opacity: {opacity}">
              <span class="activity-icon">
                {#if recent.status === "done" || recent.status === "staged"}
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#22c55e" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg>
                {:else if recent.status === "failed"}
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#ef4444" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
                {:else if recent.status === "cached" || recent.status === "downloaded"}
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#22c55e" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg>
                {:else}
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--text-tertiary)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12h14" /></svg>
                {/if}
              </span>
              <span class="activity-name activity-name-faded" title={recent.name}>{recent.name}</span>
              <span class="activity-status activity-status-done">
                {recent.status === "done" ? "completed" : recent.status === "failed" ? "failed" : recent.status}
              </span>
            </div>
          {/each}
        </div>
      </section>
    {/if}

    <!-- Mod Log -->
    <section class="phase-section mod-log-section">
      <button class="collapsible-header" onclick={() => modLogExpanded = !modLogExpanded}>
        <h3 class="phase-title">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="8" y1="6" x2="21" y2="6" />
            <line x1="8" y1="12" x2="21" y2="12" />
            <line x1="8" y1="18" x2="21" y2="18" />
            <line x1="3" y1="6" x2="3.01" y2="6" />
            <line x1="3" y1="12" x2="3.01" y2="12" />
            <line x1="3" y1="18" x2="3.01" y2="18" />
          </svg>
          MOD LOG
        </h3>
        <span class="phase-count">{mods.length} mods</span>
        <div class="log-summary-chips">
          {#if modsDone > 0}
            <span class="summary-chip chip-done">{modsDone} done</span>
          {/if}
          {#if modsFailed > 0}
            <span class="summary-chip chip-failed">{modsFailed} failed</span>
          {/if}
          {#if modsSkipped > 0}
            <span class="summary-chip chip-skipped">{modsSkipped} skipped</span>
          {/if}
          {#if modsPending > 0}
            <span class="summary-chip chip-pending">{modsPending} pending</span>
          {/if}
        </div>
        <svg class="chevron" class:expanded={modLogExpanded} width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="6 9 12 15 18 9" />
        </svg>
      </button>
      <div class="mod-log-list" class:expanded={modLogExpanded}>
        {#each visibleMods as mod (mod.index)}
          <div class="mod-entry" class:mod-failed={mod.status === "failed"} class:mod-action={mod.status === "user_action"}>
            <span class="mod-status-icon">
              {#if mod.status === "pending"}
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" opacity="0.3"><circle cx="12" cy="12" r="10" /></svg>
              {:else if mod.status === "queued"}
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" opacity="0.5"><circle cx="12" cy="12" r="10" /><polyline points="12 6 12 12 16 14" /></svg>
              {:else if mod.status === "downloading"}
                <svg class="icon-bounce" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--system-accent)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" /><polyline points="7 10 12 15 17 10" /><line x1="12" y1="15" x2="12" y2="3" /></svg>
              {:else if mod.status === "downloaded" || mod.status === "cached" || mod.status === "staged"}
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#22c55e" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg>
              {:else if mod.status === "extracting" || mod.status === "installing"}
                <svg class="icon-spin" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" /></svg>
              {:else if mod.status === "deploying"}
                <svg class="icon-bounce" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--system-accent)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" /><polyline points="17 8 12 3 7 8" /><line x1="12" y1="3" x2="12" y2="15" /></svg>
              {:else if mod.status === "done"}
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#22c55e" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg>
              {:else if mod.status === "failed"}
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#ef4444" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
              {:else if mod.status === "skipped"}
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" opacity="0.5"><path d="M5 12h14" /></svg>
              {:else if mod.status === "user_action"}
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#f59e0b" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" /><line x1="12" y1="9" x2="12" y2="13" /><line x1="12" y1="17" x2="12.01" y2="17" /></svg>
              {:else if mod.status === "fomod_pending"}
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#a78bfa" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" /></svg>
              {/if}
            </span>
            <span class="mod-name" title={mod.name}>{mod.name}</span>
            <span class="mod-status-label">{mod.status.replace("_", " ")}</span>
            {#if mod.status === "failed" && mod.error}
              <span class="mod-error" title={mod.error}>{mod.error}</span>
            {/if}
            {#if mod.status === "fomod_pending" && mod.fomodData}
              <button class="btn btn-secondary btn-sm fomod-configure-btn" onclick={() => {
                if (mod.fomodData) {
                  activeFomod = {
                    modIndex: mod.index,
                    modName: mod.name,
                    correlationId: mod.fomodData.correlationId,
                    installer: mod.fomodData.installer,
                  };
                }
              }}>
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" /></svg>
                Configure
              </button>
              <button class="btn btn-ghost btn-sm" onclick={async () => {
                if (mod.fomodData) {
                  try {
                    await submitFomodChoices(mod.fomodData.correlationId, {});
                    collectionInstallStatus.update(s => s ? {
                      ...s,
                      pendingFomods: (s.pendingFomods ?? []).filter(f => f.correlationId !== mod.fomodData?.correlationId),
                    } : s);
                  } catch { /* best effort */ }
                }
              }}>
                Use Defaults
              </button>
            {/if}
          </div>
        {/each}
        {#if !modLogExpanded && mods.length > 10}
          <button class="show-all-btn" onclick={() => modLogExpanded = true}>
            Show all {mods.length} mods
          </button>
        {/if}
      </div>
    </section>

    <!-- Verbose Log -->
    <section class="phase-section verbose-log-section">
      <button class="collapsible-header" onclick={() => verboseLogExpanded = !verboseLogExpanded}>
        <h3 class="phase-title">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="4 17 10 11 4 5" />
            <line x1="12" y1="19" x2="20" y2="19" />
          </svg>
          VERBOSE LOG
        </h3>
        <span class="phase-count">{logEntries.length} entries</span>
        <svg class="chevron" class:expanded={verboseLogExpanded} width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="6 9 12 15 18 9" />
        </svg>
      </button>
      {#if verboseLogExpanded}
        <div class="verbose-log-list" bind:this={verboseLogEl}>
          {#each logEntries as entry, i (i)}
            <div class="log-entry" class:log-warn={entry.level === "warn"} class:log-error={entry.level === "error"}>
              <span class="log-time">{formatLogTime(entry.timestamp)}</span>
              <span class="log-msg">{entry.message}</span>
            </div>
          {/each}
          {#if logEntries.length === 0}
            <div class="log-entry log-empty">
              <span class="log-msg">Waiting for events...</span>
            </div>
          {/if}
        </div>
      {/if}
    </section>

    <!-- Cancel Button (during active install) -->
    {#if phase === "downloading" || phase === "installing" || phase === "staging"}
      <div class="footer-actions">
        <button class="btn btn-ghost-danger" onclick={() => showCancelConfirm = true}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="10" />
            <line x1="15" y1="9" x2="9" y2="15" />
            <line x1="9" y1="9" x2="15" y2="15" />
          </svg>
          Cancel Installation
        </button>
      </div>
    {/if}
  </div>
{/if}

<!-- FOMOD Wizard -->
{#if activeFomod}
  <FomodWizard
    installer={activeFomod.installer}
    onComplete={async (selections) => {
      if (!activeFomod) return;
      const corrId = activeFomod.correlationId;
      try {
        await submitFomodChoices(corrId, selections);
        collectionInstallStatus.update(s => s ? {
          ...s,
          pendingFomods: (s.pendingFomods ?? []).filter(f => f.correlationId !== corrId),
        } : s);
      } catch { /* best effort */ }
      activeFomod = null;
    }}
    onCancel={() => { activeFomod = null; }}
  />
{/if}

<!-- Cancel Confirmation Modal -->
{#if showCancelConfirm}
  <div class="modal-overlay" onclick={() => showCancelConfirm = false} role="dialog" aria-modal="true">
    <div class="modal-panel cancel-modal" onclick={(e) => e.stopPropagation()}>
      <div class="cancel-modal-icon">
        <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="#f59e0b" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
          <line x1="12" y1="9" x2="12" y2="13" />
          <line x1="12" y1="17" x2="12.01" y2="17" />
        </svg>
      </div>
      <h3 class="cancel-modal-title">Cancel Installation?</h3>
      <p class="cancel-modal-desc">
        Mods already installed will be kept. Downloaded archives are preserved and can be reused.
        The install can be resumed later from the checkpoint.
      </p>
      <div class="cancel-modal-actions">
        <button class="btn btn-primary" onclick={() => showCancelConfirm = false}>
          Continue Installing
        </button>
        <button class="btn btn-ghost-danger" onclick={handleCancelInstall}>
          Cancel Install
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  /* ---- Page Layout ---- */

  .progress-page {
    padding: var(--space-2) 0 var(--space-12) 0;
    max-width: 100%;
  }

  /* ---- Empty State ---- */

  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-4);
    min-height: 400px;
    color: var(--text-tertiary);
  }

  .empty-title {
    font-size: 16px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  /* ---- Header ---- */

  .page-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--space-6);
    padding-bottom: var(--space-4);
    border-bottom: 1px solid var(--separator);
  }

  .header-left {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-width: 0;
  }

  .page-title {
    font-size: 20px;
    font-weight: 700;
    color: var(--text-primary);
    letter-spacing: -0.02em;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .header-right {
    flex-shrink: 0;
  }

  .elapsed-badge {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    padding: var(--space-1) var(--space-3);
    font-size: 13px;
    font-weight: 500;
    font-family: var(--font-mono);
    color: var(--text-secondary);
  }

  /* ---- Phase Timeline ---- */

  .phase-timeline {
    display: flex;
    align-items: flex-start;
    justify-content: center;
    margin-bottom: var(--space-5);
    padding: var(--space-3) var(--space-4);
  }

  .timeline-step {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-1);
    min-width: 64px;
  }

  .timeline-dot {
    width: 24px;
    height: 24px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 2px solid var(--separator);
    background: var(--surface);
    transition: all 0.3s ease;
  }

  .timeline-step.active .timeline-dot {
    border-color: var(--system-accent);
    background: var(--system-accent);
    color: white;
  }

  .timeline-step.done .timeline-dot {
    border-color: #22c55e;
    background: #22c55e;
    color: white;
  }

  .timeline-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-tertiary);
  }

  .timeline-step.active .timeline-label {
    color: var(--system-accent);
  }

  .timeline-step.done .timeline-label {
    color: #22c55e;
  }

  .timeline-connector {
    flex: 1;
    height: 2px;
    background: var(--separator);
    margin: 0 var(--space-1);
    margin-top: 12px;
    transition: background 0.3s ease;
  }

  .timeline-connector.done {
    background: #22c55e;
  }

  .timeline-pulse {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: white;
    animation: pulse 1.5s ease-in-out infinite;
  }

  @keyframes pulse {
    0%, 100% { opacity: 1; transform: scale(1); }
    50% { opacity: 0.5; transform: scale(0.7); }
  }

  /* ---- Overall Progress ---- */

  .overall-progress-section {
    margin-bottom: var(--space-4);
  }

  .overall-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--space-2);
  }

  .overall-left {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .overall-label {
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-secondary);
  }

  /* ---- Activity Orb ---- */

  .activity-orb {
    position: relative;
    width: 12px;
    height: 12px;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .activity-orb-inner {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--system-accent);
    box-shadow: 0 0 6px color-mix(in srgb, var(--system-accent) 60%, transparent);
  }

  .activity-orb:not(.activity-orb-idle)::before {
    content: "";
    position: absolute;
    inset: -2px;
    border-radius: 50%;
    border: 2px solid color-mix(in srgb, var(--system-accent) 40%, transparent);
    animation: orb-ping var(--orb-duration, 2s) ease-out infinite;
  }

  .activity-orb:not(.activity-orb-idle) {
    --orb-duration: inherit;
  }

  .activity-orb:not(.activity-orb-idle) .activity-orb-inner {
    animation: orb-glow var(--orb-duration, 2s) ease-in-out infinite;
  }

  .activity-orb-idle .activity-orb-inner {
    background: var(--text-tertiary);
    box-shadow: none;
    opacity: 0.4;
  }

  @keyframes orb-ping {
    0% { transform: scale(1); opacity: 0.6; }
    70% { transform: scale(1.8); opacity: 0; }
    100% { transform: scale(1.8); opacity: 0; }
  }

  @keyframes orb-glow {
    0%, 100% { box-shadow: 0 0 4px color-mix(in srgb, var(--system-accent) 40%, transparent); }
    50% { box-shadow: 0 0 10px color-mix(in srgb, var(--system-accent) 80%, transparent); }
  }

  .overall-percent {
    font-size: 18px;
    font-weight: 700;
    font-family: var(--font-mono);
    color: var(--text-primary);
  }

  .progress-track-lg {
    height: 12px;
    border-radius: 6px;
  }

  .progress-fill-overall {
    background: linear-gradient(90deg, var(--system-accent), color-mix(in srgb, var(--system-accent) 70%, #22c55e));
  }

  /* ---- Phase Sections ---- */

  .phase-section {
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    padding: var(--space-4);
    margin-bottom: var(--space-4);
  }

  .phase-header {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-bottom: var(--space-3);
    flex-wrap: wrap;
  }

  .phase-title {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-secondary);
    margin: 0;
  }

  .phase-count {
    font-size: 13px;
    font-weight: 600;
    font-family: var(--font-mono);
    color: var(--text-primary);
  }

  .cache-badge {
    font-size: 11px;
    font-weight: 600;
    color: #22c55e;
    background: rgba(34, 197, 94, 0.12);
    padding: 2px 8px;
    border-radius: 100px;
  }

  .fail-badge {
    font-size: 11px;
    font-weight: 600;
    color: #ef4444;
    background: rgba(239, 68, 68, 0.12);
    padding: 2px 8px;
    border-radius: 100px;
  }

  .speed-badge {
    font-size: 11px;
    font-weight: 600;
    color: var(--system-accent);
    background: color-mix(in srgb, var(--system-accent) 12%, transparent);
    padding: 2px 8px;
    border-radius: 100px;
    font-family: var(--font-mono);
  }

  .eta-badge {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-secondary);
    background: var(--surface-hover);
    padding: 2px 8px;
    border-radius: 100px;
  }

  /* ---- Progress Bars ---- */

  .progress-track {
    width: 100%;
    height: 8px;
    background: var(--bg-tertiary);
    border-radius: 4px;
    overflow: hidden;
    position: relative;
  }

  .progress-track-sm {
    height: 4px;
  }

  .progress-fill {
    height: 100%;
    background: var(--system-accent);
    border-radius: 4px;
    transition: width 300ms ease;
    min-width: 0;
    position: relative;
    overflow: hidden;
  }

  .progress-fill::after {
    content: "";
    position: absolute;
    inset: 0;
    background: linear-gradient(
      90deg,
      transparent 0%,
      rgba(255, 255, 255, 0.25) 45%,
      rgba(255, 255, 255, 0.35) 50%,
      rgba(255, 255, 255, 0.25) 55%,
      transparent 100%
    );
    animation: glass-progress-shimmer 2s var(--ease) infinite;
  }

  .progress-active {
    animation: progress-pulse 2s ease-in-out infinite;
  }

  @keyframes progress-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.7; }
  }

  /* ---- Active Downloads ---- */

  .active-downloads {
    margin-top: var(--space-3);
    padding-top: var(--space-3);
    border-top: 1px solid var(--separator);
  }

  .sub-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--space-2);
  }

  .sub-title {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .concurrency-badge {
    font-size: 11px;
    color: var(--text-tertiary);
    font-family: var(--font-mono);
  }

  .download-item {
    padding: var(--space-2) 0;
  }

  .download-item + .download-item {
    border-top: 1px solid var(--separator);
  }

  .dl-info {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-2);
    margin-bottom: var(--space-1);
  }

  .dl-name {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
    flex: 1;
  }

  .dl-bytes {
    font-size: 12px;
    font-family: var(--font-mono);
    color: var(--text-tertiary);
    white-space: nowrap;
    flex-shrink: 0;
  }

  /* ---- Extracting List ---- */

  .extracting-list {
    margin-top: var(--space-3);
    padding-top: var(--space-3);
    border-top: 1px solid var(--separator);
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .extracting-item-wrap {
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding: 3px var(--space-2);
    border-radius: var(--radius-sm);
  }

  .extracting-item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: 12px;
  }

  .extracting-bar {
    height: 2px;
    background: var(--bg-tertiary);
    border-radius: 2px;
    overflow: hidden;
    margin-left: 20px;
  }

  .extracting-bar-fill {
    height: 100%;
    width: 40%;
    background: var(--system-accent);
    border-radius: 2px;
    animation: indeterminate 1.5s ease-in-out infinite;
  }

  .extracting-name {
    flex: 1;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--text-primary);
    font-weight: 500;
  }

  .extracting-detail {
    flex-shrink: 0;
    font-size: 11px;
    color: var(--text-tertiary);
    font-style: italic;
    font-family: var(--font-mono);
  }

  /* ---- Install Detail ---- */

  .install-detail {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-top: var(--space-3);
    padding-top: var(--space-2);
  }

  .current-mod {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
    flex: 1;
  }

  .current-step {
    font-size: 12px;
    color: var(--text-tertiary);
    flex-shrink: 0;
    font-style: italic;
  }

  /* ---- Completion Panel ---- */

  .completion-panel {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: var(--space-4);
    padding: var(--space-10) var(--space-6);
    background: var(--surface-glass);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    margin-bottom: var(--space-4);
    backdrop-filter: var(--glass-blur-light);
    box-shadow: var(--glass-refraction), var(--glass-edge-shadow);
    animation: glass-slide-up 500ms var(--ease-out);
  }

  .completion-icon {
    margin-bottom: var(--space-1);
    animation: glass-scale-pop 600ms var(--ease-spring);
  }

  .completion-icon :global(svg) {
    filter: drop-shadow(0 0 12px rgba(34, 197, 94, 0.4));
  }

  .completion-title {
    font-size: 20px;
    font-weight: 700;
    color: var(--text-primary);
    letter-spacing: -0.02em;
    margin: 0;
    animation: glass-fade-in 400ms var(--ease-out) 200ms both;
  }

  .completion-stats {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
    justify-content: center;
    animation: glass-fade-in 400ms var(--ease-out) 350ms both;
  }

  .stat-chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-3);
    border-radius: 100px;
    font-size: 13px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .stat-success {
    color: #22c55e;
    background: rgba(34, 197, 94, 0.12);
  }

  .stat-skip {
    color: var(--text-tertiary);
    background: var(--surface-hover);
  }

  .stat-fail {
    color: #ef4444;
    background: rgba(239, 68, 68, 0.12);
  }

  .completion-actions {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
    justify-content: center;
  }

  .completion-elapsed {
    font-size: 13px;
    color: var(--text-tertiary);
    font-family: var(--font-mono);
    margin: 0;
  }

  .failed-panel {
    border-color: rgba(239, 68, 68, 0.2);
    background: color-mix(in srgb, #ef4444 3%, var(--surface));
  }

  .error-summary {
    width: 100%;
    max-width: 480px;
    background: var(--bg-tertiary);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    padding: var(--space-3);
    text-align: left;
  }

  .error-summary-title {
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: #ef4444;
    margin: 0 0 var(--space-2) 0;
  }

  .error-summary-item {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--space-1) 0;
    border-bottom: 1px solid var(--separator);
  }

  .error-summary-item:last-of-type {
    border-bottom: none;
  }

  .error-mod-name {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .error-mod-reason {
    font-size: 11px;
    color: #ef4444;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .error-more {
    display: block;
    font-size: 11px;
    color: var(--text-tertiary);
    margin-top: var(--space-2);
    font-style: italic;
  }

  .queued-badge {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-secondary);
    background: var(--surface-hover);
    padding: 2px 8px;
    border-radius: 100px;
    font-family: var(--font-mono);
  }

  .stall-badge {
    font-size: 11px;
    font-weight: 700;
    color: #f59e0b;
    background: rgba(245, 158, 11, 0.12);
    padding: 2px 8px;
    border-radius: 100px;
    animation: stall-blink 1.5s ease-in-out infinite;
  }

  @keyframes stall-blink {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
  }

  /* ---- Collapsible Sections ---- */

  .collapsible-header {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    padding: 0;
    margin-bottom: 0;
    cursor: pointer;
    background: none;
    border: none;
    color: inherit;
  }

  .collapsible-header:hover .phase-title {
    color: var(--text-primary);
  }

  .chevron {
    margin-left: auto;
    color: var(--text-tertiary);
    transition: transform 0.15s ease;
  }

  .chevron.expanded {
    transform: rotate(180deg);
  }

  .action-count {
    font-size: 11px;
    font-weight: 700;
    color: #f59e0b;
    background: rgba(245, 158, 11, 0.12);
    padding: 2px 8px;
    border-radius: 100px;
    min-width: 20px;
    text-align: center;
  }

  /* ---- User Actions ---- */

  .actions-section .phase-title {
    color: #f59e0b;
  }

  .actions-list {
    margin-top: var(--space-3);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .action-item {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-3);
    background: rgba(245, 158, 11, 0.06);
    border: 1px solid rgba(245, 158, 11, 0.15);
    border-radius: var(--radius-sm);
  }

  .action-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .action-mod {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .action-desc {
    font-size: 12px;
    color: var(--text-secondary);
  }

  .action-instructions {
    font-size: 11px;
    color: var(--text-tertiary);
    font-style: italic;
  }

  /* ---- Mod Log Summary Chips ---- */

  .log-summary-chips {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  .summary-chip {
    display: inline-flex;
    align-items: center;
    font-size: 10px;
    font-weight: 700;
    padding: 1px 7px;
    border-radius: 100px;
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.02em;
  }

  .chip-done {
    color: #22c55e;
    background: rgba(34, 197, 94, 0.12);
  }

  .chip-failed {
    color: #ef4444;
    background: rgba(239, 68, 68, 0.12);
  }

  .chip-skipped {
    color: #f59e0b;
    background: rgba(245, 158, 11, 0.12);
  }

  .chip-pending {
    color: var(--text-tertiary);
    background: var(--surface-hover);
  }

  /* ---- Mod Log ---- */

  .mod-log-section .collapsible-header {
    margin-bottom: 0;
  }

  .mod-log-list {
    margin-top: var(--space-3);
    max-height: 340px;
    overflow-y: auto;
  }

  .mod-log-list.expanded {
    max-height: 70vh;
  }

  .mod-entry {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 5px var(--space-2);
    font-size: 12px;
    border-radius: var(--radius-sm);
  }

  .mod-entry:hover {
    background: var(--surface-hover);
  }

  .mod-entry.mod-failed {
    background: rgba(239, 68, 68, 0.06);
  }

  .mod-entry.mod-action {
    background: rgba(245, 158, 11, 0.06);
  }

  .mod-status-icon {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
  }

  .mod-name {
    flex: 1;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--text-primary);
    font-weight: 500;
  }

  .mod-status-label {
    flex-shrink: 0;
    font-size: 11px;
    color: var(--text-tertiary);
    text-transform: capitalize;
    font-family: var(--font-mono);
  }

  .mod-error {
    flex-shrink: 0;
    max-width: 200px;
    font-size: 11px;
    color: #ef4444;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .show-all-btn {
    display: block;
    width: 100%;
    padding: var(--space-2);
    margin-top: var(--space-1);
    font-size: 12px;
    font-weight: 500;
    color: var(--system-accent);
    text-align: center;
    background: none;
    border: none;
    cursor: pointer;
    border-radius: var(--radius-sm);
  }

  .show-all-btn:hover {
    background: var(--surface-hover);
  }

  /* ---- Activity Feed ---- */

  .activity-section {
    border-color: color-mix(in srgb, var(--system-accent) 30%, var(--separator));
    background: color-mix(in srgb, var(--system-accent) 4%, var(--surface));
  }

  .activity-header {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-bottom: var(--space-3);
  }

  .activity-count {
    font-size: 11px;
    font-weight: 600;
    color: var(--system-accent);
    background: color-mix(in srgb, var(--system-accent) 12%, transparent);
    padding: 2px 8px;
    border-radius: 100px;
    font-family: var(--font-mono);
  }

  .activity-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .activity-item {
    display: grid;
    grid-template-columns: 20px 1fr auto;
    grid-template-rows: auto auto;
    gap: 0 var(--space-2);
    align-items: center;
    padding: var(--space-2) var(--space-2);
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--system-accent) 6%, var(--surface));
    border: 1px solid color-mix(in srgb, var(--system-accent) 15%, transparent);
  }

  .activity-icon {
    grid-row: 1 / -1;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .activity-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  .activity-status {
    font-size: 11px;
    color: var(--text-tertiary);
    font-family: var(--font-mono);
    white-space: nowrap;
  }

  .activity-bar {
    grid-column: 2 / -1;
    height: 3px;
    background: var(--bg-tertiary);
    border-radius: 2px;
    overflow: hidden;
    margin-top: 2px;
  }

  .activity-bar-fill {
    height: 100%;
    background: var(--system-accent);
    border-radius: 2px;
    transition: width 300ms ease;
  }

  .activity-bar-indeterminate {
    width: 40%;
    animation: indeterminate 1.5s ease-in-out infinite;
  }

  .activity-item-recent {
    background: transparent;
    border-color: transparent;
    transition: opacity 0.5s ease-out;
  }

  .activity-name-faded {
    color: var(--text-tertiary);
  }

  .activity-status-done {
    color: #22c55e;
    font-weight: 600;
  }

  /* ---- Animated Icons ---- */

  .icon-spin {
    animation: spin 2s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .icon-bounce {
    animation: bounce 1s ease-in-out infinite;
  }

  @keyframes bounce {
    0%, 100% { transform: translateY(0); }
    50% { transform: translateY(2px); }
  }

  @keyframes indeterminate {
    0% { transform: translateX(-100%); }
    100% { transform: translateX(350%); }
  }

  @keyframes pulse-glow {
    0%, 100% { border-color: color-mix(in srgb, var(--system-accent) 15%, transparent); }
    50% { border-color: color-mix(in srgb, var(--system-accent) 35%, transparent); }
  }

  .activity-item:not(.activity-item-recent) {
    animation: pulse-glow 2s ease-in-out infinite;
  }

  .speed-inline {
    color: var(--system-accent);
    font-weight: 600;
    margin-left: 4px;
  }

  /* ---- Footer ---- */

  .footer-actions {
    display: flex;
    justify-content: center;
    padding-top: var(--space-4);
  }

  /* ---- Buttons ---- */

  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    border-radius: var(--radius);
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s ease;
    white-space: nowrap;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-sm {
    padding: var(--space-1) var(--space-3);
    font-size: 12px;
    min-height: 28px;
  }

  .btn-primary {
    background: var(--system-accent);
    color: var(--system-accent-on);
    padding: var(--space-2) var(--space-5);
    border-radius: var(--radius);
  }

  .btn-primary:hover:not(:disabled) {
    background: var(--system-accent-hover);
    box-shadow: 0 1px 6px rgba(0, 122, 255, 0.25);
  }

  .btn-secondary {
    background: var(--surface-hover);
    color: var(--text-primary);
    border: 1px solid var(--separator);
    padding: var(--space-1) var(--space-3);
  }

  .btn-secondary:hover {
    background: var(--surface-active);
  }

  .btn-ghost {
    background: transparent;
    color: var(--text-secondary);
    padding: var(--space-2) var(--space-3);
    font-size: 13px;
    font-weight: 500;
  }

  .btn-ghost:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .btn-ghost-danger {
    background: transparent;
    color: var(--red);
    padding: var(--space-2) var(--space-4);
    font-size: 13px;
    font-weight: 500;
  }

  .btn-ghost-danger:hover {
    background: rgba(255, 59, 48, 0.08);
  }

  /* ---- Scrollbar ---- */

  .mod-log-list::-webkit-scrollbar {
    width: 6px;
  }

  .mod-log-list::-webkit-scrollbar-track {
    background: transparent;
  }

  .mod-log-list::-webkit-scrollbar-thumb {
    background: var(--scrollbar-thumb);
    border-radius: 3px;
  }

  .mod-log-list::-webkit-scrollbar-thumb:hover {
    background: var(--scrollbar-thumb-hover);
  }

  /* ---- Verbose Log ---- */

  .verbose-log-section {
    background: var(--bg-tertiary);
    border-color: var(--separator);
  }

  .verbose-log-list {
    margin-top: var(--space-3);
    max-height: 300px;
    overflow-y: auto;
    font-family: var(--font-mono);
    font-size: 11px;
    line-height: 1.6;
    background: color-mix(in srgb, #000 30%, var(--bg-tertiary));
    border-radius: var(--radius-sm);
    padding: var(--space-2);
  }

  .log-entry {
    display: flex;
    gap: var(--space-2);
    padding: 1px 0;
  }

  .log-time {
    flex-shrink: 0;
    color: var(--text-tertiary);
    opacity: 0.6;
  }

  .log-msg {
    color: var(--text-secondary);
    word-break: break-word;
  }

  .log-warn .log-msg {
    color: #f59e0b;
  }

  .log-error .log-msg {
    color: #ef4444;
  }

  .log-empty .log-msg {
    color: var(--text-tertiary);
    font-style: italic;
  }

  .verbose-log-list::-webkit-scrollbar {
    width: 6px;
  }

  .verbose-log-list::-webkit-scrollbar-track {
    background: transparent;
  }

  .verbose-log-list::-webkit-scrollbar-thumb {
    background: var(--scrollbar-thumb);
    border-radius: 3px;
  }

  .verbose-log-list::-webkit-scrollbar-thumb:hover {
    background: var(--scrollbar-thumb-hover);
  }

  /* ---- Collection Description ---- */

  .description-section .collapsible-header {
    margin-bottom: 0;
  }

  .description-content {
    margin-top: var(--space-3);
    font-size: 13px;
    line-height: 1.6;
    color: var(--text-secondary);
    max-height: 300px;
    overflow-y: auto;
    padding-right: var(--space-2);
  }

  .description-content :global(h1),
  .description-content :global(h2),
  .description-content :global(h3) {
    font-size: 14px;
    font-weight: 700;
    color: var(--text-primary);
    margin: var(--space-3) 0 var(--space-1) 0;
  }

  .description-content :global(p) {
    margin: var(--space-2) 0;
  }

  .description-content :global(a) {
    color: var(--system-accent);
    text-decoration: none;
  }

  .description-content :global(a:hover) {
    text-decoration: underline;
  }

  .description-content :global(ul),
  .description-content :global(ol) {
    padding-left: var(--space-4);
    margin: var(--space-2) 0;
  }

  .description-content :global(li) {
    margin: var(--space-1) 0;
  }

  .description-content :global(code) {
    font-family: var(--font-mono);
    font-size: 12px;
    background: var(--bg-tertiary);
    padding: 1px 4px;
    border-radius: 3px;
  }

  .description-content :global(img) {
    max-width: 100%;
    border-radius: var(--radius-sm);
  }

  .description-content::-webkit-scrollbar {
    width: 6px;
  }

  .description-content::-webkit-scrollbar-track {
    background: transparent;
  }

  .description-content::-webkit-scrollbar-thumb {
    background: var(--scrollbar-thumb);
    border-radius: 3px;
  }

  /* ---- Cancel Install Button (header) ---- */

  .cancel-install-btn {
    color: var(--red);
    font-size: 12px;
    padding: var(--space-1) var(--space-3);
    gap: var(--space-1);
  }

  .cancel-install-btn:hover {
    background: rgba(255, 59, 48, 0.08);
  }

  /* ---- Cancel Confirmation Modal ---- */

  .modal-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
    backdrop-filter: var(--glass-blur-light);
  }

  .cancel-modal {
    background: var(--bg-secondary);
    border: 1px solid var(--separator);
    border-radius: var(--radius-lg);
    padding: var(--space-6);
    max-width: 420px;
    width: 90%;
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
  }

  .cancel-modal-icon {
    margin-bottom: var(--space-1);
  }

  .cancel-modal-title {
    font-size: 16px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .cancel-modal-desc {
    font-size: 13px;
    color: var(--text-secondary);
    line-height: 1.5;
    margin: 0;
  }

  .cancel-modal-actions {
    display: flex;
    gap: var(--space-3);
    margin-top: var(--space-2);
    width: 100%;
    justify-content: center;
  }

  /* ---- FOMOD Attention Banner ---- */

  .fomod-attention-section {
    margin-bottom: var(--space-3);
  }

  .fomod-attention-banner {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    background: color-mix(in srgb, #a78bfa 10%, var(--surface));
    border: 1px solid color-mix(in srgb, #a78bfa 30%, var(--separator));
    border-radius: var(--radius);
    flex-wrap: wrap;
  }

  .fomod-attention-text {
    font-size: 13px;
    font-weight: 600;
    color: #a78bfa;
    flex: 1;
  }

  .fomod-attention-actions {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  /* ---- FOMOD Configure Button in Mod Log ---- */

  .fomod-configure-btn {
    flex-shrink: 0;
  }

  .mod-entry .btn-sm {
    padding: 2px var(--space-2);
    font-size: 11px;
    min-height: 24px;
  }
</style>
