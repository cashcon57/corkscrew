<script lang="ts">
  import { showError, showSuccess } from "$lib/stores";
  import type { CleanReport, CleanOptions, DetectedGame } from "$lib/types";
  import { cleanGameDirectory } from "$lib/api";

  interface Props {
    game: DetectedGame;
    report: CleanReport;
    onclean: () => void;
    onskip: () => void;
    oncancel: () => void;
  }

  let { game, report, onclean, onskip, oncancel }: Props = $props();

  let cleanRunning = $state(false);
  let cleanOptions = $state<CleanOptions>({
    remove_loose_files: true,
    remove_archives: true,
    remove_enb: false,
    remove_saves: false,
    remove_skse: false,
    orphans_only: false,
    dry_run: false,
    exclude_patterns: [],
  });
  let cleanExcludeInput = $state("");

  function formatSize(bytes: number): string {
    if (bytes >= 1073741824) return `${(bytes / 1073741824).toFixed(1)} GB`;
    if (bytes >= 1048576) return `${(bytes / 1048576).toFixed(0)} MB`;
    if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KB`;
    return `${bytes} B`;
  }

  async function handleCleanAndInstall() {
    cleanRunning = true;
    try {
      const patterns = cleanExcludeInput
        .split("\n")
        .map((p) => p.trim())
        .filter((p) => p.length > 0);
      const options: CleanOptions = {
        ...cleanOptions,
        exclude_patterns: patterns,
        dry_run: false,
      };

      const result = await cleanGameDirectory(
        game.game_id,
        game.bottle_name,
        options
      );

      showSuccess(`Cleaned ${result.removed_files.length} files (${formatSize(result.bytes_freed)} freed)`);
    } catch (e) {
      showError(`Cleanup failed: ${e}`);
    } finally {
      cleanRunning = false;
    }

    onclean();
  }
</script>

<div class="modal-overlay" onclick={oncancel} role="presentation">
  <div class="cleanup-modal" onclick={(e) => e.stopPropagation()} role="dialog" aria-label="Pre-install cleanup">
    <div class="cleanup-header">
      <h3 class="cleanup-title">Pre-Install Cleanup</h3>
      <button class="cleanup-close" onclick={oncancel}>&times;</button>
    </div>

    <div class="cleanup-body">
      <div class="cleanup-summary">
        <p class="cleanup-info">
          Found <strong>{report.non_stock_files.length}</strong> non-stock files
          ({formatSize(report.total_size)}) in the game directory.
          Cleaning these before installing a collection ensures a fresh start.
        </p>

        <div class="cleanup-stats">
          <div class="cleanup-stat">
            <span class="cleanup-stat-value">{report.orphaned_count}</span>
            <span class="cleanup-stat-label">Orphaned</span>
          </div>
          <div class="cleanup-stat">
            <span class="cleanup-stat-value">{report.managed_count}</span>
            <span class="cleanup-stat-label">Managed</span>
          </div>
          {#if report.enb_files.length > 0}
            <div class="cleanup-stat">
              <span class="cleanup-stat-value">{report.enb_files.length}</span>
              <span class="cleanup-stat-label">ENB Files</span>
            </div>
          {/if}
          {#if report.save_files.length > 0}
            <div class="cleanup-stat">
              <span class="cleanup-stat-value">{report.save_files.length}</span>
              <span class="cleanup-stat-label">Saves (safe)</span>
            </div>
          {/if}
        </div>
      </div>

      <div class="cleanup-options">
        <h4>Clean Options</h4>
        <label class="cleanup-checkbox">
          <input type="checkbox" bind:checked={cleanOptions.remove_loose_files} />
          Remove loose mod files (plugins, meshes, textures, scripts)
        </label>
        <label class="cleanup-checkbox">
          <input type="checkbox" bind:checked={cleanOptions.remove_archives} />
          Remove non-stock BSA/BA2 archives
        </label>
        <label class="cleanup-checkbox">
          <input type="checkbox" bind:checked={cleanOptions.remove_enb} />
          Remove ENB files ({report.enb_files.length} found)
        </label>
        <label class="cleanup-checkbox">
          <input type="checkbox" bind:checked={cleanOptions.orphans_only} />
          Only remove orphaned files (skip Corkscrew-managed files)
        </label>
      </div>

      <details class="cleanup-advanced">
        <summary>Exclude Patterns</summary>
        <p class="cleanup-hint">One glob pattern per line (e.g., <code>SKSE/Plugins/*</code>)</p>
        <textarea
          class="cleanup-exclude-input"
          bind:value={cleanExcludeInput}
          placeholder="SKSE/Plugins/*&#10;SkyUI_SE.bsa"
          rows="3"
        ></textarea>
      </details>

      {#if report.save_files.length > 0}
        <div class="cleanup-save-notice">
          Save files ({report.save_files.length}) are automatically excluded from cleanup.
        </div>
      {/if}
    </div>

    <div class="cleanup-footer">
      <button class="btn btn-ghost" onclick={oncancel} disabled={cleanRunning}>Cancel</button>
      <button class="btn btn-secondary" onclick={onskip} disabled={cleanRunning}>Skip & Install</button>
      <button class="btn btn-primary" onclick={handleCleanAndInstall} disabled={cleanRunning}>
        {#if cleanRunning}
          <div class="spinner-sm"></div>
          Cleaning...
        {:else}
          Clean & Install
        {/if}
      </button>
    </div>
  </div>
</div>

<style>
  .modal-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
  }

  .cleanup-modal {
    background: var(--bg-secondary);
    border: 1px solid var(--border-primary);
    border-radius: 12px;
    width: 560px;
    max-width: 90vw;
    max-height: 80vh;
    display: flex;
    flex-direction: column;
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.5);
  }

  .cleanup-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 20px;
    border-bottom: 1px solid var(--border-primary);
  }

  .cleanup-title {
    font-size: 16px;
    font-weight: 600;
    margin: 0;
    color: var(--text-primary);
  }

  .cleanup-close {
    background: none;
    border: none;
    color: var(--text-secondary);
    font-size: 20px;
    cursor: pointer;
    padding: 4px 8px;
    border-radius: 4px;
    line-height: 1;
  }

  .cleanup-close:hover {
    background: var(--bg-tertiary);
    color: var(--text-primary);
  }

  .cleanup-body {
    padding: 20px;
    overflow-y: auto;
    flex: 1;
  }

  .cleanup-summary {
    margin-bottom: 16px;
  }

  .cleanup-info {
    font-size: 13px;
    color: var(--text-secondary);
    margin: 0 0 12px 0;
    line-height: 1.5;
  }

  .cleanup-info :global(strong) {
    color: var(--text-primary);
  }

  .cleanup-stats {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
  }

  .cleanup-stat {
    display: flex;
    flex-direction: column;
    align-items: center;
    background: var(--bg-tertiary);
    padding: 8px 16px;
    border-radius: 8px;
    min-width: 80px;
  }

  .cleanup-stat-value {
    font-size: 18px;
    font-weight: 700;
    color: var(--text-primary);
    font-family: var(--font-mono);
  }

  .cleanup-stat-label {
    font-size: 11px;
    color: var(--text-tertiary);
    margin-top: 2px;
  }

  .cleanup-options {
    margin-bottom: 16px;
  }

  .cleanup-options h4 {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    margin: 0 0 8px 0;
  }

  .cleanup-checkbox {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    color: var(--text-secondary);
    padding: 4px 0;
    cursor: pointer;
  }

  .cleanup-checkbox input[type="checkbox"] {
    accent-color: var(--system-accent);
    width: 16px;
    height: 16px;
  }

  .cleanup-advanced {
    margin-bottom: 12px;
  }

  .cleanup-advanced summary {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-secondary);
    cursor: pointer;
    padding: 4px 0;
  }

  .cleanup-advanced summary:hover {
    color: var(--text-primary);
  }

  .cleanup-hint {
    font-size: 11px;
    color: var(--text-tertiary);
    margin: 8px 0 4px 0;
  }

  .cleanup-hint code {
    background: var(--bg-tertiary);
    padding: 1px 4px;
    border-radius: 3px;
    font-size: 11px;
  }

  .cleanup-exclude-input {
    width: 100%;
    background: var(--bg-primary);
    border: 1px solid var(--border-primary);
    border-radius: 6px;
    color: var(--text-primary);
    font-family: var(--font-mono);
    font-size: 12px;
    padding: 8px;
    resize: vertical;
  }

  .cleanup-exclude-input:focus {
    outline: none;
    border-color: var(--system-accent);
  }

  .cleanup-save-notice {
    font-size: 12px;
    color: #22c55e;
    background: rgba(34, 197, 94, 0.1);
    border: 1px solid rgba(34, 197, 94, 0.2);
    border-radius: 6px;
    padding: 8px 12px;
  }

  .cleanup-footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 16px 20px;
    border-top: 1px solid var(--border-primary);
  }
</style>
