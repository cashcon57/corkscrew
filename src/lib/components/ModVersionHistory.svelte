<script lang="ts">
  import { selectedGame, showError, showSuccess } from "$lib/stores";
  import { listModVersions, rollbackModVersion } from "$lib/api";
  import type { InstalledMod, ModVersion } from "$lib/types";

  // ---- Props ----

  interface Props {
    mod: InstalledMod;
    onrollback?: (version: ModVersion) => void;
  }

  let { mod, onrollback }: Props = $props();

  // ---- State ----

  let versions = $state<ModVersion[]>([]);
  let loading = $state(false);
  let rollingBack = $state(false);
  let confirmRollback = $state<ModVersion | null>(null);
  let showConfirmDialog = $state(false);

  const game = $derived($selectedGame);

  $effect(() => {
    if (mod && game) {
      loadVersionHistory();
    }
  });

  async function loadVersionHistory() {
    if (!game) return;
    loading = true;
    try {
      versions = await listModVersions(mod.id);
    } catch (e: unknown) {
      showError(`Failed to load version history: ${e}`);
    } finally {
      loading = false;
    }
  }

  function requestRollback(version: ModVersion) {
    confirmRollback = version;
    showConfirmDialog = true;
  }

  function cancelRollback() {
    confirmRollback = null;
    showConfirmDialog = false;
  }

  async function executeRollback() {
    if (!confirmRollback || !game) return;
    rollingBack = true;
    try {
      await rollbackModVersion(mod.id, confirmRollback.id);

      showSuccess(`Rolled back "${mod.name}" to version ${confirmRollback.version}`);

      if (onrollback) {
        onrollback(confirmRollback);
      }

      // Reload versions
      await loadVersionHistory();
    } catch (e: unknown) {
      showError(`Rollback failed: ${e}`);
    } finally {
      rollingBack = false;
      cancelRollback();
    }
  }

  function formatDate(iso: string): string {
    return new Date(iso).toLocaleDateString(undefined, {
      month: "short",
      day: "numeric",
      year: "numeric",
    });
  }

  function formatDateTime(iso: string): string {
    return new Date(iso).toLocaleString(undefined, {
      month: "short",
      day: "numeric",
      year: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  }

  function handleDialogKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      cancelRollback();
    }
  }
</script>

<div class="version-history">
  <!-- Header -->
  <div class="vh-header">
    <h4 class="vh-title">Version History</h4>
    <span class="vh-mod-name">{mod.name}</span>
  </div>

  <!-- Content -->
  {#if loading}
    <div class="vh-loading">
      <span class="spinner-sm"></span>
      <span class="loading-label">Loading version history...</span>
    </div>
  {:else if versions.length === 0}
    <div class="vh-empty">
      <p class="empty-text">No version history available for this mod.</p>
    </div>
  {:else}
    <div class="vh-list" role="list">
      {#each versions as version (version.id)}
        <div
          class="vh-entry"
          class:vh-entry-current={version.is_current}
          role="listitem"
        >
          <div class="vh-entry-marker">
            <div class="marker-dot" class:marker-current={version.is_current}></div>
            {#if versions.indexOf(version) < versions.length - 1}
              <div class="marker-line"></div>
            {/if}
          </div>
          <div class="vh-entry-content">
            <div class="vh-entry-header">
              <span class="vh-version">{version.version}</span>
              {#if version.is_current}
                <span class="current-badge">Current</span>
              {/if}
              <span class="vh-date">{formatDate(version.created_at)}</span>
            </div>
            {#if version.archive_name}
              <span class="vh-archive">{version.archive_name}</span>
            {/if}
            {#if !version.is_current}
              <button
                class="btn btn-rollback"
                onclick={() => requestRollback(version)}
                disabled={rollingBack}
                type="button"
              >
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
                  <path d="M3 3v5h5" />
                </svg>
                Rollback
              </button>
            {/if}
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<!-- Confirmation Dialog -->
{#if showConfirmDialog && confirmRollback}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div
    class="dialog-backdrop"
    onclick={cancelRollback}
    onkeydown={handleDialogKeydown}
    role="presentation"
  >
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_interactive_supports_focus -->
    <div class="dialog" onclick={(e) => e.stopPropagation()} role="dialog" aria-label="Confirm rollback">
      <div class="dialog-icon">
        <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
          <path d="M3 3v5h5" />
        </svg>
      </div>
      <h3 class="dialog-title">Roll back mod version?</h3>
      <p class="dialog-description">
        Roll back <strong>{mod.name}</strong> to version <strong>{confirmRollback.version}</strong>?
        This will redeploy the older version of the mod files.
      </p>
      <div class="dialog-meta">
        <div class="meta-row">
          <span class="meta-label">Version</span>
          <span class="meta-value">{confirmRollback.version}</span>
        </div>
        <div class="meta-row">
          <span class="meta-label">Installed</span>
          <span class="meta-value">{formatDateTime(confirmRollback.created_at)}</span>
        </div>
        {#if confirmRollback.archive_name}
          <div class="meta-row">
            <span class="meta-label">Archive</span>
            <span class="meta-value meta-mono">{confirmRollback.archive_name}</span>
          </div>
        {/if}
      </div>
      <div class="dialog-actions">
        <button
          class="btn btn-danger"
          onclick={executeRollback}
          disabled={rollingBack}
          type="button"
        >
          {#if rollingBack}
            <span class="spinner-sm spinner-white"></span>
            Rolling back...
          {:else}
            Roll Back
          {/if}
        </button>
        <button
          class="btn btn-ghost"
          onclick={cancelRollback}
          disabled={rollingBack}
          type="button"
        >
          Cancel
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  /* ---- Container ---- */

  .version-history {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  /* ---- Header ---- */

  .vh-header {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .vh-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .vh-mod-name {
    font-size: 12px;
    color: var(--text-tertiary);
  }

  /* ---- Loading / Empty ---- */

  .vh-loading {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3);
  }

  .loading-label {
    font-size: 12px;
    color: var(--text-tertiary);
  }

  .vh-empty {
    padding: var(--space-4);
    text-align: center;
  }

  .empty-text {
    font-size: 12px;
    color: var(--text-tertiary);
    line-height: 1.5;
  }

  /* ---- Version List (Timeline) ---- */

  .vh-list {
    display: flex;
    flex-direction: column;
  }

  .vh-entry {
    display: flex;
    gap: var(--space-3);
  }

  /* ---- Timeline Marker ---- */

  .vh-entry-marker {
    display: flex;
    flex-direction: column;
    align-items: center;
    width: 16px;
    flex-shrink: 0;
    padding-top: 5px;
  }

  .marker-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--text-quaternary);
    flex-shrink: 0;
  }

  .marker-current {
    background: var(--green);
    box-shadow: 0 0 6px rgba(48, 209, 88, 0.3);
  }

  .marker-line {
    width: 1px;
    flex: 1;
    background: var(--separator);
    margin-top: 4px;
  }

  /* ---- Entry Content ---- */

  .vh-entry-content {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding-bottom: var(--space-3);
    flex: 1;
    min-width: 0;
  }

  .vh-entry-header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .vh-version {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    font-family: var(--font-mono);
    letter-spacing: 0;
  }

  .current-badge {
    display: inline-flex;
    align-items: center;
    padding: 1px 6px;
    border-radius: 100px;
    font-size: 10px;
    font-weight: 600;
    color: var(--green);
    background: var(--green-subtle);
  }

  .vh-date {
    font-size: 11px;
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
  }

  .vh-archive {
    font-size: 11px;
    color: var(--text-quaternary);
    font-family: var(--font-mono);
    letter-spacing: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* ---- Buttons ---- */

  .btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 12px;
    font-weight: 500;
    border: none;
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
    white-space: nowrap;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-rollback {
    padding: var(--space-1) var(--space-2);
    background: var(--surface-hover);
    color: var(--text-secondary);
    margin-top: var(--space-1);
    align-self: flex-start;
  }

  .btn-rollback:hover:not(:disabled) {
    background: var(--system-accent-subtle);
    color: var(--system-accent);
  }

  .btn-danger {
    padding: var(--space-2) var(--space-4);
    background: var(--red);
    color: #fff;
    font-weight: 600;
  }

  .btn-danger:hover:not(:disabled) {
    filter: brightness(1.1);
  }

  .btn-ghost {
    padding: var(--space-2) var(--space-4);
    background: transparent;
    color: var(--text-secondary);
  }

  .btn-ghost:hover:not(:disabled) {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  /* ---- Spinners ---- */

  .spinner-sm {
    display: inline-block;
    width: 12px;
    height: 12px;
    border: 2px solid var(--separator-opaque);
    border-top-color: var(--system-accent);
    border-radius: 50%;
    animation: spin 0.75s linear infinite;
    flex-shrink: 0;
  }

  .spinner-white {
    border-color: rgba(255, 255, 255, 0.3);
    border-top-color: #fff;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  /* ---- Confirmation Dialog ---- */

  .dialog-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 2000;
    animation: fadeIn 0.2s var(--ease);
  }

  .dialog {
    background: var(--bg-elevated);
    border: 1px solid var(--separator-opaque);
    border-radius: var(--radius-xl);
    box-shadow: var(--shadow-lg);
    width: 380px;
    max-width: calc(100vw - var(--space-8));
    padding: var(--space-6);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    animation: dialogIn 0.25s var(--ease-out);
    text-align: center;
  }

  .dialog-icon {
    color: var(--system-accent);
    margin-bottom: var(--space-1);
  }

  .dialog-title {
    font-size: 17px;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .dialog-description {
    font-size: 13px;
    color: var(--text-secondary);
    line-height: 1.5;
  }

  .dialog-description strong {
    color: var(--text-primary);
    font-weight: 600;
  }

  .dialog-meta {
    width: 100%;
    background: var(--surface);
    border-radius: var(--radius);
    overflow: hidden;
    margin: var(--space-1) 0;
  }

  .meta-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-2) var(--space-3);
  }

  .meta-row + .meta-row {
    border-top: 1px solid var(--separator);
  }

  .meta-label {
    font-size: 12px;
    color: var(--text-secondary);
  }

  .meta-value {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-primary);
  }

  .meta-mono {
    font-family: var(--font-mono);
    letter-spacing: 0;
    font-size: 11px;
    max-width: 200px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .dialog-actions {
    display: flex;
    gap: var(--space-2);
    margin-top: var(--space-2);
    width: 100%;
  }

  .dialog-actions .btn {
    flex: 1;
    justify-content: center;
  }

  @keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  @keyframes dialogIn {
    from { transform: scale(0.95); opacity: 0; }
    to { transform: scale(1); opacity: 1; }
  }
</style>
