<script lang="ts">
  import { goto } from "$app/navigation";
  import { collectionInstallStatus } from "$lib/stores";
  import { dismissInstall } from "$lib/installService";
  import { openUrl } from "@tauri-apps/plugin-opener";

  let modLogExpanded = $state(false);
  let userActionsExpanded = $state(true);

  let status = $derived($collectionInstallStatus);
  let isActive = $derived(status?.active ?? false);
  let phase = $derived(status?.phase ?? "");
  let dl = $derived(status?.downloadProgress ?? { total: 0, completed: 0, failed: 0, cached: 0, maxConcurrent: 0, active: [] });
  let inst = $derived(status?.installProgress ?? { current: 0, total: 0, currentMod: "", step: "" });
  let mods = $derived(status?.modDetails ?? []);
  let actions = $derived(status?.userActions ?? []);
  let result = $derived(status?.result ?? null);

  let dlPercent = $derived(dl.total > 0 ? Math.round((dl.completed / dl.total) * 100) : 0);
  let instPercent = $derived(inst.total > 0 ? Math.round((inst.current / inst.total) * 100) : 0);

  // Mod log: show 10 items when collapsed, all when expanded
  let visibleMods = $derived(modLogExpanded ? mods : mods.slice(0, 10));

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

  function handleCancel() {
    dismissInstall();
    goto("/collections");
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
      <button class="btn btn-primary" onclick={() => goto('/collections')}>Back to Collections</button>
    </div>
  </div>
{:else}
  <div class="progress-page">
    <!-- Header -->
    <header class="page-header">
      <div class="header-left">
        <button class="btn btn-ghost" onclick={() => goto('/collections')}>
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
      </div>
    </header>

    {#if phase === "complete"}
      <!-- Completion Panel -->
      <div class="completion-panel">
        <div class="completion-icon">
          <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="#22c55e" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14" />
            <polyline points="22 4 12 14.01 9 11.01" />
          </svg>
        </div>
        <h2 class="completion-title">Collection installed successfully</h2>
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
        <p class="completion-elapsed">Total time: {status.elapsed}</p>
        <button class="btn btn-primary" onclick={() => { dismissInstall(); goto('/collections'); }}>
          Back to Collections
        </button>
      </div>
    {:else}
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
        </div>
        <div class="progress-track">
          <div class="progress-fill" style="width: {dlPercent}%"></div>
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

      <!-- Install Phase -->
      {#if phase === "installing"}
        <section class="phase-section">
          <div class="phase-header">
            <h3 class="phase-title">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z" />
              </svg>
              INSTALL
            </h3>
            <span class="phase-count">{inst.current} / {inst.total}</span>
          </div>
          <div class="progress-track">
            <div class="progress-fill" style="width: {instPercent}%"></div>
          </div>
          {#if inst.currentMod}
            <div class="install-detail">
              <span class="current-mod" title={inst.currentMod}>{inst.currentMod}</span>
              <span class="current-step">{inst.step}</span>
            </div>
          {/if}
        </section>
      {/if}
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
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--system-accent)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" /><polyline points="7 10 12 15 17 10" /><line x1="12" y1="15" x2="12" y2="3" /></svg>
              {:else if mod.status === "downloaded" || mod.status === "cached"}
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#22c55e" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg>
              {:else if mod.status === "extracting"}
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" /></svg>
              {:else if mod.status === "deploying"}
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--system-accent)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" /><polyline points="17 8 12 3 7 8" /><line x1="12" y1="3" x2="12" y2="15" /></svg>
              {:else if mod.status === "done"}
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#22c55e" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg>
              {:else if mod.status === "failed"}
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#ef4444" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
              {:else if mod.status === "skipped"}
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" opacity="0.5"><path d="M5 12h14" /></svg>
              {:else if mod.status === "user_action"}
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#f59e0b" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" /><line x1="12" y1="9" x2="12" y2="13" /><line x1="12" y1="17" x2="12.01" y2="17" /></svg>
              {/if}
            </span>
            <span class="mod-name" title={mod.name}>{mod.name}</span>
            <span class="mod-status-label">{mod.status.replace("_", " ")}</span>
            {#if mod.status === "failed" && mod.error}
              <span class="mod-error" title={mod.error}>{mod.error}</span>
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

    <!-- Cancel Button (during active install) -->
    {#if phase === "downloading" || phase === "installing"}
      <div class="footer-actions">
        <button class="btn btn-ghost-danger" onclick={handleCancel}>
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

  /* ---- Progress Bars ---- */

  .progress-track {
    width: 100%;
    height: 8px;
    background: var(--bg-tertiary);
    border-radius: 4px;
    overflow: hidden;
  }

  .progress-track-sm {
    height: 4px;
  }

  .progress-fill {
    height: 100%;
    background: var(--system-accent);
    border-radius: 4px;
    transition: width 300ms var(--ease);
    min-width: 0;
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
    text-transform: capitalize;
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
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    margin-bottom: var(--space-4);
  }

  .completion-icon {
    margin-bottom: var(--space-1);
  }

  .completion-title {
    font-size: 20px;
    font-weight: 700;
    color: var(--text-primary);
    letter-spacing: -0.02em;
    margin: 0;
  }

  .completion-stats {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
    justify-content: center;
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

  .completion-elapsed {
    font-size: 13px;
    color: var(--text-tertiary);
    font-family: var(--font-mono);
    margin: 0;
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
    transition: transform var(--duration-fast) var(--ease);
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

  /* ---- Mod Log ---- */

  .mod-log-section .collapsible-header {
    margin-bottom: 0;
  }

  .mod-log-list {
    margin-top: var(--space-3);
    max-height: 360px;
    overflow-y: auto;
  }

  .mod-log-list.expanded {
    max-height: 500px;
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
    transition: all var(--duration-fast) var(--ease);
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
</style>
