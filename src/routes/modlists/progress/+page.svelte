<script lang="ts">
  import { goto } from "$app/navigation";
  import { wjInstallStatus } from "$lib/stores";
  import { cancelWabbajackInstall, cleanupWabbajackInstall } from "$lib/api";
  import { clearWjInstallStatus, stopWjInstallTracking } from "$lib/wjInstallService";
  import { SpeedTracker } from "$lib/speedTracker";
  import { formatBytes } from "$lib/format";
  import { openUrl } from "@tauri-apps/plugin-opener";

  let status = $derived($wjInstallStatus);
  let phase = $derived(status?.phase ?? "");
  let overallProgress = $derived(status?.overallProgress ?? 0);
  let dl = $derived(status?.downloadProgress ?? { current: 0, total: 0, completed: 0, bytesDownloaded: 0, totalBytes: 0, currentFile: "", speed: 0, eta: "", maxConcurrent: 0, activeDownloads: [] });
  let ext = $derived(status?.extractionProgress ?? { current: 0, total: 0, currentArchive: "", totalBytes: 0, bytesCompleted: 0 });
  let dir = $derived(status?.directiveProgress ?? { current: 0, total: 0, bytesProcessed: 0, totalBytes: 0, currentFile: "", directiveType: "files" });
  let dep = $derived(status?.deployProgress ?? { current: 0, total: 0, bytesDeployed: 0, totalBytes: 0 });
  let archives = $derived(status?.archives ?? []);
  let result = $derived(status?.result ?? null);
  let logEntries = $derived(status?.logEntries ?? []);

  let verboseLogExpanded = $state(false);
  let verboseLogEl: HTMLDivElement | null = $state(null);

  // Auto-scroll log when expanded
  $effect(() => {
    if (verboseLogExpanded && verboseLogEl && logEntries.length > 0) {
      verboseLogEl.scrollTop = verboseLogEl.scrollHeight;
    }
  });

  function dlItemPercent(item: { bytes: number; totalBytes: number }): number {
    if (item.totalBytes <= 0) return 0;
    return Math.min(100, (item.bytes / item.totalBytes) * 100);
  }

  function formatLogTime(ts: number): string {
    const d = new Date(ts);
    return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  }

  const phaseLabels: Record<string, string> = {
    preflight: "Running Pre-Flight Checks",
    downloading: "Downloading Archives",
    extracting: "Extracting Archives",
    directives: "Processing Directives",
    deploying: "Deploying Files",
    complete: "Installation Complete",
    failed: "Installation Failed",
    cancelled: "Installation Cancelled",
  };

  const phases = [
    { id: "preflight", label: "Preflight" },
    { id: "downloading", label: "Download" },
    { id: "extracting", label: "Extract" },
    { id: "directives", label: "Process" },
    { id: "deploying", label: "Deploy" },
    { id: "complete", label: "Done" },
  ] as const;
  const phaseOrder = ["preflight", "downloading", "extracting", "directives", "deploying", "complete", "failed", "cancelled"];

  let showCancelConfirm = $state(false);
  let cancelInProgress = $state(false);

  async function handleCancelInstall() {
    if (!status?.installId) return;
    cancelInProgress = true;
    try {
      await cancelWabbajackInstall(status.installId);
    } catch (e) {
      console.error("Failed to cancel WJ install:", e);
    } finally {
      cancelInProgress = false;
      showCancelConfirm = false;
    }
  }

  function handleDone() {
    if (status?.installId) {
      cleanupWabbajackInstall(status.installId).catch((err) => console.error("WJ cleanup error:", err));
    }
    clearWjInstallStatus();
    // Don't navigate here — let each calling button decide where to go
  }

  function handleBackToGallery() {
    // Navigate away but keep the install running in background
    goto("/").catch(() => { window.location.href = "/"; });
  }
</script>

{#if !status}
  <!-- No active install -->
  <div class="progress-page">
    <div class="empty-state">
      <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="12" cy="12" r="10" />
        <line x1="12" y1="8" x2="12" y2="12" />
        <line x1="12" y1="16" x2="12.01" y2="16" />
      </svg>
      <p class="empty-title">No active Wabbajack installation</p>
      <button class="btn btn-primary" onclick={handleBackToGallery}>Back to Gallery</button>
    </div>
  </div>
{:else}
  <div class="progress-page">
    <!-- Hero Banner -->
    {#if status.imageUrl}
      <div class="hero-banner" style="background-image: url('{status.imageUrl}')">
        <div class="hero-overlay"></div>
        <div class="hero-content">
          <button class="btn btn-ghost hero-back" onclick={handleBackToGallery}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <polyline points="15 18 9 12 15 6" />
            </svg>
          </button>
          <div class="hero-text">
            <h1 class="hero-title">
              {#if phase === "complete"}
                Installation Complete
              {:else if phase === "failed"}
                Installation Failed
              {:else if phase === "cancelled"}
                Installation Cancelled
              {:else}
                {status.modlistName}
              {/if}
            </h1>
            {#if status.author}
              <span class="hero-author">by {status.author}</span>
            {/if}
          </div>
        </div>
      </div>
    {/if}

    <!-- Header (fallback when no image) -->
    <header class="page-header" class:page-header-hidden={!!status.imageUrl}>
      <div class="header-left">
        {#if !status.imageUrl}
          <button class="btn btn-ghost" onclick={handleBackToGallery}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <polyline points="15 18 9 12 15 6" />
            </svg>
            Back
          </button>
        {/if}
        <h1 class="page-title" class:page-title-hidden={!!status.imageUrl}>
          {#if phase === "complete"}
            Installation Complete
          {:else if phase === "failed"}
            Installation Failed
          {:else if phase === "cancelled"}
            Installation Cancelled
          {:else}
            Installing '{status.modlistName}'
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
        {#if phase !== "complete" && phase !== "failed" && phase !== "cancelled"}
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

    <!-- Cancel Confirmation -->
    {#if showCancelConfirm}
      <div class="cancel-confirm-panel">
        <p>Are you sure you want to cancel this installation?</p>
        <div class="cancel-actions">
          <button class="btn btn-danger btn-sm" disabled={cancelInProgress} onclick={handleCancelInstall}>
            {cancelInProgress ? "Cancelling..." : "Yes, Cancel"}
          </button>
          <button class="btn btn-ghost btn-sm" onclick={() => showCancelConfirm = false}>
            No, Continue
          </button>
        </div>
      </div>
    {/if}

    {#if phase === "complete"}
      <!-- Completion Panel -->
      <div class="completion-panel">
        <div class="completion-icon">
          <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="#22c55e" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14" />
            <polyline points="22 4 12 14.01 9 11.01" />
          </svg>
        </div>
        <h2 class="completion-title">Modlist installed successfully</h2>
        <div class="completion-stats">
          <div class="stat-chip stat-success">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg>
            {result?.filesDeployed?.toLocaleString() ?? 0} files deployed
          </div>
          {#if result?.warnings && result.warnings.length > 0}
            <div class="stat-chip stat-warn">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" /><line x1="12" y1="9" x2="12" y2="13" /><line x1="12" y1="17" x2="12.01" y2="17" /></svg>
              {result.warnings.length} warnings
            </div>
          {/if}
        </div>
        {#if result?.warnings && result.warnings.length > 0}
          <div class="warning-list">
            {#each result.warnings as warning}
              <div class="warning-item">{warning}</div>
            {/each}
          </div>
        {/if}
        <p class="completion-elapsed">Total time: {status.elapsed}{#if result?.elapsed} ({result.elapsed.toFixed(0)}s){/if}</p>
        <div class="completion-actions">
          <button class="btn btn-primary" onclick={() => { handleDone(); goto('/mods').catch((err) => { console.error('goto /mods:', err); window.location.href = '/mods'; }); }}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="3" width="20" height="14" rx="2" ry="2" /><line x1="8" y1="21" x2="16" y2="21" /><line x1="12" y1="17" x2="12" y2="21" /></svg>
            View Mods
          </button>
          <button class="btn btn-secondary" onclick={() => { handleDone(); goto('/plugins').catch((err) => { console.error('goto /plugins:', err); window.location.href = '/plugins'; }); }}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="8" y1="6" x2="21" y2="6" /><line x1="8" y1="12" x2="21" y2="12" /><line x1="8" y1="18" x2="21" y2="18" /><line x1="3" y1="6" x2="3.01" y2="6" /><line x1="3" y1="12" x2="3.01" y2="12" /><line x1="3" y1="18" x2="3.01" y2="18" /></svg>
            Load Order
          </button>
          <button class="btn btn-ghost" onclick={() => { handleDone(); goto('/modlists').catch(() => { window.location.href = '/modlists'; }); }}>
            Back to Gallery
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
        {#if status.error}
          <div class="error-summary">
            <p class="error-text">{status.error}</p>
          </div>
        {/if}
        <p class="completion-elapsed">Time elapsed: {status.elapsed}</p>
        <div class="completion-actions">
          <button class="btn btn-primary" onclick={handleDone}>
            Back to Gallery
          </button>
        </div>
      </div>

    {:else if phase === "cancelled"}
      <!-- Cancelled Panel -->
      <div class="completion-panel">
        <div class="completion-icon">
          <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="var(--text-tertiary)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="10" />
            <line x1="15" y1="9" x2="9" y2="15" />
            <line x1="9" y1="9" x2="15" y2="15" />
          </svg>
        </div>
        <h2 class="completion-title">Installation Cancelled</h2>
        <p class="completion-elapsed">Time elapsed: {status.elapsed}</p>
        <div class="completion-actions">
          <button class="btn btn-primary" onclick={handleDone}>
            Back to Gallery
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
            <span class="activity-orb" class:activity-orb-idle={!status.active}>
              <span class="activity-orb-inner"></span>
            </span>
            <span class="overall-label">OVERALL PROGRESS</span>
          </div>
          <span class="overall-percent">{overallProgress}%</span>
        </div>
        <div class="progress-track progress-track-lg">
          <div class="progress-fill progress-fill-overall" class:progress-active={status.active} class:complete={overallProgress >= 100} style="width: {overallProgress}%"></div>
        </div>
      </section>

      <!-- Preflight Phase -->
      {#if phase === "preflight"}
        <section class="phase-section">
          <div class="phase-header">
            <h3 class="phase-title">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M9 11l3 3L22 4" />
                <path d="M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11" />
              </svg>
              PREFLIGHT CHECKS
            </h3>
          </div>
          <div class="preflight-content">
            <div class="status-spinner status-spinner-inline"></div>
            <span class="preflight-note">{status.preflightNote || "Running checks..."}</span>
          </div>
        </section>
      {/if}

      <!-- Download Phase -->
      {#if phase === "downloading" || (phaseOrder.indexOf(phase) > phaseOrder.indexOf("downloading") && dl.total > 0)}
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
            <span class="phase-count">{dl.completed} / {dl.total}{#if phase === "downloading" && dl.activeDownloads.length > 0} <span class="phase-count-detail">({dl.activeDownloads.length} active)</span>{/if}</span>
            {#if dl.maxConcurrent > 0 && phase === "downloading"}
              <span class="concurrency-badge">{dl.maxConcurrent} threads</span>
            {/if}
            {#if phase === "downloading" && status.speedLabel}
              <span class="speed-badge">{status.speedLabel}</span>
            {/if}
            {#if phase === "downloading" && status.etaLabel}
              <span class="eta-badge">{status.etaLabel}</span>
            {/if}
          </div>
          {#if dl.total > 0}
            <div class="progress-track">
              <div class="progress-fill" style="width: {dl.totalBytes > 0 ? Math.min(100, (dl.bytesDownloaded / dl.totalBytes) * 100).toFixed(1) : (dl.total > 0 ? (dl.completed / dl.total) * 100 : 0).toFixed(1)}%"></div>
            </div>
            <div class="phase-detail-row">
              {#if dl.totalBytes > 0}
                <span class="detail-bytes">{formatBytes(dl.bytesDownloaded)} / {formatBytes(dl.totalBytes)}</span>
              {/if}
            </div>
          {/if}

          <!-- Active Downloads -->
          {#if dl.activeDownloads.length > 0 && phase === "downloading"}
            <div class="active-downloads">
              <div class="active-dl-header">
                <span class="active-dl-title">Active Downloads</span>
                <span class="active-dl-count">{dl.activeDownloads.length} active</span>
              </div>
              {#each dl.activeDownloads as item (item.index)}
                <div class="download-item">
                  <div class="dl-info">
                    <span class="dl-icon icon-bounce">
                      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="var(--system-accent)" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                        <polyline points="7 10 12 15 17 10" />
                        <line x1="12" y1="15" x2="12" y2="3" />
                      </svg>
                    </span>
                    <span class="dl-name" title={item.name}>{item.name}</span>
                    <span class="dl-bytes">
                      {formatBytes(item.bytes)}{#if item.totalBytes > 0} / {formatBytes(item.totalBytes)}{/if}
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
      {/if}

      <!-- Extraction Phase -->
      {#if phase === "extracting" || (phaseOrder.indexOf(phase) > phaseOrder.indexOf("extracting") && ext.total > 0)}
        <section class="phase-section">
          <div class="phase-header">
            <h3 class="phase-title">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M21 10V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16v-2" />
                <polyline points="7.5 4.27 12 6.81 16.5 4.27" />
                <polyline points="7.5 19.77 7.5 14.27 3 11.73" />
              </svg>
              EXTRACTION
            </h3>
            <span class="phase-count">{ext.current} / {ext.total}</span>
            {#if phase === "extracting" && status.speedLabel}
              <span class="speed-badge">{status.speedLabel}</span>
            {/if}
          </div>
          {#if ext.total > 0}
            <div class="progress-track">
              <div class="progress-fill" style="width: {(ext.current / ext.total * 100).toFixed(1)}%"></div>
            </div>
            <div class="phase-detail-row">
              {#if ext.totalBytes > 0}
                <span class="detail-bytes">{formatBytes(ext.bytesCompleted)} / {formatBytes(ext.totalBytes)}</span>
              {/if}
              {#if ext.currentArchive && phase === "extracting"}
                <span class="detail-file" title={ext.currentArchive}>{ext.currentArchive}</span>
              {/if}
            </div>
          {/if}

          <!-- Per-archive status list -->
          {#if archives.length > 0 && phase === "extracting"}
            <div class="archive-list">
              {#each archives as archive (archive.index)}
                <div class="archive-item" class:archive-done={archive.status === "extracted" || archive.status === "downloaded"} class:archive-active={archive.status === "extracting" || archive.status === "downloading"} class:archive-failed={archive.status === "failed"}>
                  <span class="archive-status-icon">
                    {#if archive.status === "extracting" || archive.status === "downloading"}
                      <span class="spinner-xs"></span>
                    {:else if archive.status === "extracted" || archive.status === "downloaded"}
                      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="var(--system-green, #34C759)" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"></polyline></svg>
                    {:else if archive.status === "failed"}
                      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="var(--system-red, #FF3B30)" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"></line><line x1="6" y1="6" x2="18" y2="18"></line></svg>
                    {:else}
                      <span class="archive-pending-dot"></span>
                    {/if}
                  </span>
                  <span class="archive-name" title={archive.name}>{archive.name}</span>
                  {#if archive.size > 0}
                    <span class="archive-size">{formatBytes(archive.size)}</span>
                  {/if}
                  {#if archive.error}
                    <span class="archive-error" title={archive.error}>{archive.error}</span>
                  {/if}
                </div>
              {/each}
            </div>
          {/if}
        </section>
      {/if}

      <!-- Directive Phase -->
      {#if phase === "directives" || (phaseOrder.indexOf(phase) > phaseOrder.indexOf("directives") && dir.total > 0)}
        <section class="phase-section">
          <div class="phase-header">
            <h3 class="phase-title">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="16 18 22 12 16 6" />
                <polyline points="8 6 2 12 8 18" />
              </svg>
              DIRECTIVES
            </h3>
            <span class="phase-count">{dir.current.toLocaleString()} / {dir.total.toLocaleString()}</span>
            {#if dir.directiveType && phase === "directives"}
              <span class="directive-type-badge">{dir.directiveType}</span>
            {/if}
            {#if phase === "directives" && status.speedLabel}
              <span class="speed-badge">{status.speedLabel}</span>
            {/if}
          </div>
          {#if dir.total > 0}
            <div class="progress-track">
              <div class="progress-fill" style="width: {(dir.current / dir.total * 100).toFixed(1)}%"></div>
            </div>
            <div class="phase-detail-row">
              {#if dir.totalBytes > 0}
                <span class="detail-bytes">{formatBytes(dir.bytesProcessed)} / {formatBytes(dir.totalBytes)}</span>
              {/if}
              {#if dir.currentFile && phase === "directives"}
                <span class="detail-file" title={dir.currentFile}>{dir.currentFile}</span>
              {/if}
            </div>
          {/if}
        </section>
      {/if}

      <!-- Deploy Phase -->
      {#if phase === "deploying" || (phaseOrder.indexOf(phase) > phaseOrder.indexOf("deploying") && dep.total > 0)}
        <section class="phase-section">
          <div class="phase-header">
            <h3 class="phase-title">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
              </svg>
              DEPLOYMENT
            </h3>
            <span class="phase-count">{dep.current.toLocaleString()} / {dep.total.toLocaleString()}</span>
            {#if phase === "deploying" && status.speedLabel}
              <span class="speed-badge">{status.speedLabel}</span>
            {/if}
          </div>
          {#if dep.total > 0}
            <div class="progress-track">
              <div class="progress-fill" style="width: {(dep.current / dep.total * 100).toFixed(1)}%"></div>
            </div>
            <div class="phase-detail-row">
              {#if dep.totalBytes > 0}
                <span class="detail-bytes">{formatBytes(dep.bytesDeployed)} / {formatBytes(dep.totalBytes)}</span>
              {/if}
            </div>
          {/if}
        </section>
      {/if}
    {/if}

    <!-- Verbose Log -->
    {#if logEntries.length > 0}
      <section class="phase-section verbose-log-section">
        <button class="collapsible-header" onclick={() => verboseLogExpanded = !verboseLogExpanded}>
          <h3 class="phase-title">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
              <polyline points="14 2 14 8 20 8" />
              <line x1="16" y1="13" x2="8" y2="13" />
              <line x1="16" y1="17" x2="8" y2="17" />
            </svg>
            LOG
          </h3>
          <span class="phase-count">{logEntries.length} entries</span>
          <svg class="collapse-chevron" class:collapse-chevron-open={verboseLogExpanded} width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="6 9 12 15 18 9" />
          </svg>
        </button>
        {#if verboseLogExpanded}
          <div class="verbose-log-body" bind:this={verboseLogEl}>
            {#each logEntries as entry, i (i)}
              <div class="log-entry" class:log-warn={entry.level === "warn"} class:log-error={entry.level === "error"}>
                <span class="log-time">{formatLogTime(entry.timestamp)}</span>
                <span class="log-msg">{entry.message}</span>
              </div>
            {/each}
            {#if logEntries.length === 0}
              <div class="log-empty">No events yet</div>
            {/if}
          </div>
        {/if}
      </section>
    {/if}

    <!-- Featured Mods (shown during active install to fill empty space) -->
    {#if status.active && status.featuredMods.length > 0}
      <section class="featured-section">
        <h3 class="section-label">FEATURED MODS IN THIS LIST</h3>
        <div class="featured-grid">
          {#each status.featuredMods as mod}
            <div class="featured-mod">
              <span class="featured-name">{mod.name}</span>
              <span class="featured-size">{mod.description}</span>
            </div>
          {/each}
        </div>
      </section>
    {/if}

    <!-- Readme (scrollable, shown during install) -->
    {#if status.readmeHtml}
      <section class="readme-section">
        <h3 class="section-label">README</h3>
        <div class="readme-content">
          {@html status.readmeHtml}
        </div>
      </section>
    {:else if status.description}
      <section class="readme-section">
        <h3 class="section-label">ABOUT THIS MODLIST</h3>
        <div class="readme-content">
          <p>{status.description}</p>
        </div>
      </section>
    {/if}
  </div>
{/if}

<style>
  /* Hero banner */
  .hero-banner {
    position: relative;
    width: 100%;
    height: 180px;
    background-size: cover;
    background-position: center;
    border-radius: 12px;
    margin-bottom: var(--space-4);
    overflow: hidden;
  }
  .hero-overlay {
    position: absolute;
    inset: 0;
    background: linear-gradient(to top, rgba(0,0,0,0.85) 0%, rgba(0,0,0,0.3) 60%, rgba(0,0,0,0.1) 100%);
  }
  .hero-content {
    position: absolute;
    bottom: 0;
    left: 0;
    right: 0;
    padding: var(--space-4);
    display: flex;
    align-items: flex-end;
    gap: var(--space-3);
  }
  .hero-back {
    background: rgba(255,255,255,0.1);
    backdrop-filter: blur(8px);
    border-radius: 8px;
    padding: 6px;
    color: white;
    flex-shrink: 0;
  }
  .hero-back:hover { background: rgba(255,255,255,0.2); }
  .hero-text { flex: 1; }
  .hero-title {
    font-size: 22px;
    font-weight: 700;
    color: white;
    margin: 0;
    text-shadow: 0 1px 4px rgba(0,0,0,0.5);
  }
  .hero-author {
    font-size: 13px;
    color: rgba(255,255,255,0.7);
  }
  .page-header-hidden { display: none; }
  .page-title-hidden { display: none; }

  .progress-page {
    max-width: 800px;
    margin: 0 auto;
    padding: var(--space-6) var(--space-4);
  }

  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-4);
    padding: var(--space-10) var(--space-6);
    text-align: center;
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
    gap: var(--space-3);
  }

  .header-left {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .page-title {
    font-size: 18px;
    font-weight: 700;
    color: var(--text-primary);
    margin: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .header-right {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-shrink: 0;
  }

  .elapsed-badge {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    font-size: 12px;
    font-weight: 600;
    font-family: var(--font-mono);
    color: var(--text-secondary);
    background: var(--surface);
    border: 1px solid var(--separator);
    padding: 4px 10px;
    border-radius: 100px;
    font-variant-numeric: tabular-nums;
  }

  .cancel-install-btn {
    color: #ef4444 !important;
  }

  .cancel-install-btn:hover {
    background: rgba(239, 68, 68, 0.1) !important;
  }

  /* ---- Cancel Confirm ---- */

  .cancel-confirm-panel {
    background: var(--surface);
    border: 1px solid #ef4444;
    border-radius: var(--radius);
    padding: var(--space-4);
    margin-bottom: var(--space-4);
    text-align: center;
  }

  .cancel-confirm-panel p {
    margin: 0 0 var(--space-3);
    color: var(--text-primary);
    font-size: 14px;
  }

  .cancel-actions {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
  }

  /* ---- Phase Timeline ---- */

  .phase-timeline {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0;
    margin-bottom: var(--space-6);
    padding: var(--space-3) 0;
  }

  .timeline-step {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-1);
    min-width: 56px;
  }

  .timeline-dot {
    width: 24px;
    height: 24px;
    border-radius: 50%;
    background: var(--bg-tertiary);
    border: 2px solid var(--separator);
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all 300ms ease;
  }

  .timeline-step.done .timeline-dot {
    background: var(--system-accent);
    border-color: var(--system-accent);
    color: white;
  }

  .timeline-step.active .timeline-dot {
    border-color: var(--system-accent);
    background: color-mix(in srgb, var(--system-accent) 15%, transparent);
    animation: glass-scale-pop var(--duration-fast) var(--ease-spring);
  }

  .timeline-label {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-tertiary);
  }

  .timeline-step.active .timeline-label {
    color: var(--system-accent);
  }

  .timeline-step.done .timeline-label {
    color: var(--text-secondary);
  }

  .timeline-connector {
    flex: 1;
    height: 2px;
    background: var(--separator);
    margin: 0 -2px;
    margin-bottom: 20px;
    min-width: 16px;
    max-width: 48px;
    transition: background var(--duration-slow) var(--ease);
  }

  .timeline-connector.done {
    background: var(--system-accent);
  }

  .timeline-pulse {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--system-accent);
    animation: pulse-dot 1.5s ease-in-out infinite;
  }

  @keyframes pulse-dot {
    0%, 100% { opacity: 1; transform: scale(1); }
    50% { opacity: 0.5; transform: scale(0.7); }
  }

  /* ---- Overall Progress ---- */

  .overall-progress-section {
    background: var(--surface-glass);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    padding: var(--space-4);
    margin-bottom: var(--space-4);
    backdrop-filter: var(--glass-blur-light);
  }

  .overall-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--space-3);
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

  .overall-percent {
    font-size: 18px;
    font-weight: 700;
    font-family: var(--font-mono);
    color: var(--text-primary);
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
    animation: orb-glow 2s ease-in-out infinite;
  }

  .activity-orb:not(.activity-orb-idle)::before {
    content: "";
    position: absolute;
    inset: -2px;
    border-radius: 50%;
    border: 2px solid color-mix(in srgb, var(--system-accent) 40%, transparent);
    animation: orb-ping 2s ease-out infinite;
  }

  .activity-orb-idle .activity-orb-inner {
    background: var(--text-tertiary);
    box-shadow: none;
    opacity: 0.4;
    animation: none;
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

  /* ---- Progress Bars ---- */

  .progress-track {
    width: 100%;
    height: 8px;
    background: var(--bg-tertiary);
    border-radius: 4px;
    overflow: hidden;
    position: relative;
  }

  .progress-track-lg {
    height: 12px;
    border-radius: 6px;
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

  .progress-fill-overall {
    background: linear-gradient(90deg, var(--system-accent), color-mix(in srgb, var(--system-accent) 70%, #22c55e));
  }

  .progress-fill.complete {
    animation: glass-scale-pop var(--duration) var(--ease-spring);
    box-shadow: 0 0 12px rgba(48, 209, 88, 0.4);
  }

  .progress-active {
    animation: progress-pulse 2s ease-in-out infinite;
  }

  @keyframes progress-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.7; }
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

  .directive-type-badge {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-secondary);
    background: color-mix(in srgb, var(--text-secondary) 10%, transparent);
    padding: 2px 8px;
    border-radius: 100px;
    text-transform: capitalize;
  }

  .phase-detail-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    margin-top: var(--space-2);
    font-size: 12px;
  }

  .detail-bytes {
    font-family: var(--font-mono);
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
  }

  .detail-file {
    max-width: 350px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-tertiary);
    font-size: 12px;
  }

  /* ---- Preflight ---- */

  .preflight-content {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) 0;
  }

  .preflight-note {
    font-size: 13px;
    color: var(--text-secondary);
  }

  .status-spinner-inline {
    width: 16px;
    height: 16px;
    border: 2px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
    flex-shrink: 0;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  /* ---- Archive List ---- */

  .archive-list {
    max-height: 260px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-top: var(--space-3);
    padding: var(--space-2);
    background: var(--surface-secondary, rgba(255, 255, 255, 0.03));
    border-radius: 6px;
    border-top: 1px solid var(--separator);
  }

  .archive-item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 3px 6px;
    border-radius: 4px;
    font-size: 12px;
    transition: opacity 0.2s;
  }

  .archive-done {
    opacity: 0.5;
  }

  .archive-active {
    background: color-mix(in srgb, var(--system-accent, #007AFF) 8%, transparent);
  }

  .archive-failed {
    background: color-mix(in srgb, var(--system-red, #FF3B30) 8%, transparent);
  }

  .archive-status-icon {
    width: 16px;
    height: 16px;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .archive-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-primary);
  }

  .archive-size {
    font-size: 11px;
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }

  .archive-error {
    font-size: 11px;
    color: var(--system-red, #FF3B30);
    max-width: 200px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .archive-pending-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--text-tertiary);
    opacity: 0.4;
  }

  .spinner-xs {
    width: 12px;
    height: 12px;
    border: 2px solid var(--surface-hover, rgba(255, 255, 255, 0.1));
    border-top-color: var(--system-accent, #007AFF);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  /* ---- Concurrency Badge ---- */

  .phase-count-detail {
    font-size: 11px;
    font-weight: 500;
    color: var(--text-tertiary);
  }

  .concurrency-badge {
    font-size: 10px;
    font-weight: 600;
    color: var(--text-tertiary);
    background: var(--surface-hover);
    padding: 2px 8px;
    border-radius: 100px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  /* ---- Active Downloads ---- */

  .active-downloads {
    margin-top: var(--space-3);
    padding: var(--space-3);
    background: var(--surface-secondary, rgba(255, 255, 255, 0.03));
    border-radius: 8px;
    border-top: 1px solid var(--separator);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .active-dl-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--space-1);
  }

  .active-dl-title {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-tertiary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .active-dl-count {
    font-size: 11px;
    font-weight: 600;
    color: var(--system-accent);
  }

  .download-item {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 6px 8px;
    border-radius: 6px;
    background: color-mix(in srgb, var(--system-accent) 5%, transparent);
    border: 1px solid color-mix(in srgb, var(--system-accent) 12%, transparent);
    animation: dl-item-fade-in 200ms ease-out;
  }

  @keyframes dl-item-fade-in {
    from { opacity: 0; transform: translateY(-4px); }
    to { opacity: 1; transform: translateY(0); }
  }

  .dl-info {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: 12px;
  }

  .dl-icon {
    flex-shrink: 0;
    display: flex;
    align-items: center;
  }

  .icon-bounce {
    animation: bounce 1s ease-in-out infinite;
  }

  @keyframes bounce {
    0%, 100% { transform: translateY(0); }
    50% { transform: translateY(2px); }
  }

  .dl-name {
    flex: 1;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .dl-bytes {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
    white-space: nowrap;
  }

  .progress-track-sm {
    height: 4px;
    border-radius: 2px;
  }

  /* ---- Verbose Log ---- */

  .verbose-log-section {
    border-color: color-mix(in srgb, var(--separator) 60%, transparent);
  }

  .collapsible-header {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    background: none;
    border: none;
    padding: 0;
    cursor: pointer;
    color: inherit;
    font: inherit;
  }

  .collapsible-header:hover {
    opacity: 0.8;
  }

  .collapse-chevron {
    margin-left: auto;
    transition: transform 200ms ease;
    color: var(--text-tertiary);
  }

  .collapse-chevron-open {
    transform: rotate(180deg);
  }

  .verbose-log-body {
    max-height: 300px;
    overflow-y: auto;
    margin-top: var(--space-3);
    padding: var(--space-2);
    background: var(--bg-tertiary);
    border-radius: 6px;
    font-size: 11px;
    font-family: var(--font-mono);
  }

  .log-entry {
    display: flex;
    gap: var(--space-2);
    padding: 2px 4px;
    border-radius: 2px;
  }

  .log-entry:hover {
    background: rgba(255, 255, 255, 0.03);
  }

  .log-time {
    color: var(--text-tertiary);
    flex-shrink: 0;
    font-variant-numeric: tabular-nums;
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

  .log-empty {
    color: var(--text-tertiary);
    padding: var(--space-2);
    text-align: center;
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

  .failed-panel {
    border-color: rgba(239, 68, 68, 0.3);
  }

  .completion-icon {
    margin-bottom: var(--space-1);
    animation: glass-scale-pop 600ms var(--ease-spring);
  }

  .completion-icon :global(svg) {
    filter: drop-shadow(0 0 12px rgba(34, 197, 94, 0.4));
  }

  .failed-panel .completion-icon :global(svg) {
    filter: drop-shadow(0 0 12px rgba(239, 68, 68, 0.4));
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

  .stat-warn {
    color: #f59e0b;
    background: rgba(245, 158, 11, 0.12);
  }

  .completion-elapsed {
    font-size: 13px;
    color: var(--text-tertiary);
    margin: 0;
  }

  .completion-actions {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
    justify-content: center;
    animation: glass-fade-in 400ms var(--ease-out) 500ms both;
  }

  /* ---- Error Summary ---- */

  .error-summary {
    background: rgba(239, 68, 68, 0.08);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: var(--radius);
    padding: var(--space-4);
    max-width: 500px;
    width: 100%;
  }

  .error-text {
    font-size: 13px;
    color: #ef4444;
    margin: 0;
    word-break: break-word;
  }

  /* ---- Warning List ---- */

  .warning-list {
    background: rgba(245, 158, 11, 0.08);
    border: 1px solid rgba(245, 158, 11, 0.3);
    border-radius: var(--radius);
    padding: var(--space-3);
    max-width: 500px;
    width: 100%;
    max-height: 150px;
    overflow-y: auto;
  }

  .warning-item {
    font-size: 12px;
    color: #f59e0b;
    padding: 2px 0;
  }

  /* ---- Buttons (re-use global) ---- */

  .btn {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    padding: 8px 16px;
    border-radius: var(--radius);
    border: 1px solid var(--separator);
    background: var(--surface);
    color: var(--text-primary);
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
    white-space: nowrap;
  }

  .btn:hover {
    background: var(--surface-hover);
  }

  .btn-primary {
    background: var(--system-accent);
    border-color: var(--system-accent);
    color: white;
  }

  .btn-primary:hover {
    filter: brightness(1.1);
  }

  .btn-secondary {
    background: var(--surface-hover);
  }

  .btn-ghost {
    background: transparent;
    border-color: transparent;
    color: var(--text-secondary);
  }

  .btn-ghost:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .btn-danger {
    background: #ef4444;
    border-color: #ef4444;
    color: white;
  }

  .btn-danger:hover {
    filter: brightness(1.1);
  }

  .btn-sm {
    padding: 4px 12px;
    font-size: 12px;
  }

  /* Featured mods grid */
  .featured-section {
    margin-top: var(--space-4);
  }
  .section-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-tertiary);
    letter-spacing: 0.05em;
    margin-bottom: var(--space-2);
  }
  .featured-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: var(--space-2);
  }
  .featured-mod {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    padding: var(--space-2) var(--space-3);
    background: var(--bg-tertiary);
    border-radius: 6px;
    font-size: 12px;
  }
  .featured-name {
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    flex: 1;
    margin-right: var(--space-2);
  }
  .featured-size {
    color: var(--text-tertiary);
    font-size: 11px;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }

  /* Readme section */
  .readme-section {
    margin-top: var(--space-4);
  }
  .readme-content {
    max-height: 400px;
    overflow-y: auto;
    padding: var(--space-3);
    background: var(--bg-tertiary);
    border-radius: 8px;
    font-size: 13px;
    line-height: 1.6;
    color: var(--text-secondary);
  }
  .readme-content :global(h1),
  .readme-content :global(h2),
  .readme-content :global(h3) {
    color: var(--text-primary);
    margin-top: var(--space-3);
    margin-bottom: var(--space-1);
  }
  .readme-content :global(h1) { font-size: 18px; }
  .readme-content :global(h2) { font-size: 15px; }
  .readme-content :global(h3) { font-size: 13px; }
  .readme-content :global(a) { color: var(--accent); }
  .readme-content :global(p) { margin-bottom: var(--space-2); }
  .readme-content :global(ul),
  .readme-content :global(ol) {
    padding-left: var(--space-4);
    margin-bottom: var(--space-2);
  }
  .readme-content :global(code) {
    background: rgba(255,255,255,0.06);
    padding: 1px 4px;
    border-radius: 3px;
    font-size: 12px;
  }
  .readme-content :global(img) {
    max-width: 100%;
    border-radius: 6px;
  }
</style>
