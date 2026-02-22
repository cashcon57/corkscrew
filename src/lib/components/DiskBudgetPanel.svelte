<script lang="ts">
  import { getDiskBudget } from "$lib/api";
  import type { DiskBudget } from "$lib/types";

  interface Props {
    gameId: string;
    bottleName: string;
  }

  let { gameId, bottleName }: Props = $props();
  let budget = $state<DiskBudget | null>(null);
  let loading = $state(false);
  let expanded = $state(false);

  function formatBytes(bytes: number): string {
    if (bytes === 0) return "0 B";
    const units = ["B", "KB", "MB", "GB"];
    const i = Math.floor(Math.log(bytes) / Math.log(1024));
    return `${(bytes / Math.pow(1024, i)).toFixed(i > 0 ? 1 : 0)} ${units[i]}`;
  }

  const usageRatio = $derived.by(() => {
    if (!budget || budget.available_bytes === 0) return 0;
    const total = budget.staging_bytes + budget.available_bytes;
    return Math.min((budget.staging_bytes / total) * 100, 100);
  });

  const barStatus = $derived.by(() => {
    if (usageRatio > 90) return "critical";
    if (usageRatio > 70) return "warning";
    return "ok";
  });

  // Stacked bar segments
  const stackedBar = $derived.by(() => {
    if (!budget) return { staging: 0, deployed: 0, free: 100 };
    const total = budget.staging_bytes + budget.available_bytes;
    if (total === 0) return { staging: 0, deployed: 0, free: 100 };
    const staging = (budget.staging_bytes / total) * 100;
    // Only show deployed segment for copies (hardlinks have zero extra cost)
    const deployed = budget.uses_hardlinks ? 0 : (budget.deployment_bytes / total) * 100;
    const free = Math.max(0, 100 - staging - deployed);
    return { staging, deployed, free };
  });

  const methodLabel = $derived(
    budget?.uses_hardlinks ? "Hardlinks" : "Copies"
  );

  async function load() {
    loading = true;
    try {
      budget = await getDiskBudget(gameId, bottleName);
    } catch {
      budget = null;
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    if (gameId && bottleName) load();
  });
</script>

<div class="disk-budget-panel">
  <!-- Compact Toggle -->
  <button class="budget-toggle" onclick={() => expanded = !expanded}>
    <div class="budget-summary">
      {#if loading}
        <span class="budget-stat budget-loading-text">Loading disk info...</span>
      {:else if budget}
        <span class="budget-chip" class:chip-hardlink={budget.uses_hardlinks} class:chip-copy={!budget.uses_hardlinks}>
          {methodLabel}
        </span>
        <span class="budget-stat">
          Staging: <strong>{formatBytes(budget.staging_bytes)}</strong>
        </span>
        <span class="budget-divider"></span>
        <span class="budget-stat">
          Available: <strong>{formatBytes(budget.available_bytes)}</strong>
        </span>
      {:else}
        <span class="budget-stat budget-error-text">Disk info unavailable</span>
      {/if}
    </div>
    <svg
      class="budget-chevron"
      class:open={expanded}
      width="12"
      height="12"
      viewBox="0 0 12 12"
      fill="none"
      stroke="currentColor"
      stroke-width="1.5"
      stroke-linecap="round"
      stroke-linejoin="round"
    >
      <path d="M3 4.5L6 7.5L9 4.5" />
    </svg>
  </button>

  <!-- Expanded Details -->
  {#if expanded}
    <div class="budget-details">
      {#if loading}
        <div class="budget-loading">
          <div class="budget-spinner"></div>
          <span>Calculating disk usage...</span>
        </div>
      {:else if budget}
        <!-- Stacked Usage Bar -->
        <div class="usage-bar-container">
          <div class="usage-bar-labels">
            <span class="usage-bar-label">Disk breakdown</span>
            <span class="usage-bar-value">{usageRatio.toFixed(1)}% used</span>
          </div>
          <div class="usage-bar-track">
            {#if stackedBar.staging > 0}
              <div
                class="usage-bar-fill fill-staging"
                style="width: {stackedBar.staging}%"
                title="Staging: {budget ? formatBytes(budget.staging_bytes) : ''}"
              ></div>
            {/if}
            {#if stackedBar.deployed > 0}
              <div
                class="usage-bar-fill fill-deployed"
                style="width: {stackedBar.deployed}%"
                title="Deployed: {budget ? formatBytes(budget.deployment_bytes) : ''}"
              ></div>
            {/if}
          </div>
          <div class="usage-bar-legend">
            <span class="legend-item"><span class="legend-dot dot-staging"></span>Staging</span>
            {#if !budget?.uses_hardlinks}
              <span class="legend-item"><span class="legend-dot dot-deployed"></span>Deployed</span>
            {/if}
            <span class="legend-item"><span class="legend-dot dot-free"></span>Free</span>
          </div>
        </div>

        <!-- Stats Grid -->
        <div class="budget-grid">
          <div class="budget-item">
            <span class="budget-label">Staging Size</span>
            <span class="budget-value">{formatBytes(budget.staging_bytes)}</span>
          </div>
          <div class="budget-item">
            <span class="budget-label">Staged Mods</span>
            <span class="budget-value">{budget.staging_count}</span>
          </div>
          <div class="budget-item">
            <span class="budget-label">Deployment Cost</span>
            <span class="budget-value">
              {#if budget.uses_hardlinks}
                <span class="value-highlight-green">0 B (hardlinks)</span>
              {:else}
                {formatBytes(budget.deployment_bytes)}
              {/if}
            </span>
          </div>
          <div class="budget-item">
            <span class="budget-label">Deploy Method</span>
            <span class="budget-value">
              <span class:value-highlight-green={budget.uses_hardlinks}>
                {methodLabel}
              </span>
            </span>
          </div>
          <div class="budget-item">
            <span class="budget-label">Staging Volume Free</span>
            <span class="budget-value">{formatBytes(budget.available_bytes)}</span>
          </div>
          <div class="budget-item">
            <span class="budget-label">Game Volume Free</span>
            <span class="budget-value">{formatBytes(budget.game_available_bytes)}</span>
          </div>
          <div class="budget-item span-2">
            <span class="budget-label">Total Disk Impact</span>
            <span class="budget-value budget-value-lg">{formatBytes(budget.total_impact_bytes)}</span>
          </div>
        </div>

        <!-- Refresh -->
        <div class="budget-actions">
          <button class="btn-budget-refresh" onclick={load} disabled={loading} type="button">
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M21 2v6h-6M3 12a9 9 0 0 1 15-6.7L21 8M3 22v-6h6M21 12a9 9 0 0 1-15 6.7L3 16" />
            </svg>
            Refresh
          </button>
        </div>
      {:else}
        <div class="budget-empty">
          <p class="budget-empty-text">Could not retrieve disk usage information.</p>
          <button class="btn-budget-refresh" onclick={load} type="button">
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M21 2v6h-6M3 12a9 9 0 0 1 15-6.7L21 8M3 22v-6h6M21 12a9 9 0 0 1-15 6.7L3 16" />
            </svg>
            Retry
          </button>
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  /* ---- Panel Container ---- */

  .disk-budget-panel {
    display: flex;
    flex-direction: column;
  }

  /* ---- Toggle (compact summary line) ---- */

  .budget-toggle {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    padding: var(--space-2) var(--space-4);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease);
  }

  .budget-toggle:hover {
    background: var(--surface-hover);
  }

  .budget-summary {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    font-size: 12px;
  }

  .budget-chip {
    display: inline-flex;
    align-items: center;
    padding: 1px 8px;
    border-radius: 100px;
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }

  .chip-hardlink {
    background: color-mix(in srgb, var(--green) 15%, transparent);
    color: var(--green);
  }

  .chip-copy {
    background: color-mix(in srgb, var(--yellow) 15%, transparent);
    color: var(--yellow);
  }

  .budget-stat {
    color: var(--text-secondary);
    font-weight: 500;
  }

  .budget-stat strong {
    color: var(--text-primary);
    font-weight: 600;
  }

  .budget-divider {
    width: 1px;
    height: 12px;
    background: var(--separator);
    flex-shrink: 0;
  }

  .budget-loading-text {
    color: var(--text-tertiary);
    font-style: italic;
  }

  .budget-error-text {
    color: var(--text-tertiary);
  }

  .budget-chevron {
    color: var(--text-tertiary);
    transition: transform var(--duration-fast) var(--ease);
    flex-shrink: 0;
  }

  .budget-chevron.open {
    transform: rotate(180deg);
  }

  /* ---- Expanded Details ---- */

  .budget-details {
    padding: var(--space-3) var(--space-4);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-top: none;
    border-radius: 0 0 var(--radius) var(--radius);
    margin-top: -1px;
  }

  /* ---- Usage Bar ---- */

  .usage-bar-container {
    margin-bottom: var(--space-4);
  }

  .usage-bar-labels {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    margin-bottom: var(--space-1);
  }

  .usage-bar-label {
    font-size: 11px;
    font-weight: 500;
    color: var(--text-tertiary);
  }

  .usage-bar-value {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
  }

  .usage-bar-track {
    display: flex;
    width: 100%;
    height: 6px;
    border-radius: 3px;
    background: var(--separator-opaque);
    overflow: hidden;
  }

  .usage-bar-fill {
    height: 100%;
    transition: width 0.4s var(--ease);
    min-width: 2px;
  }

  .usage-bar-fill:first-child {
    border-radius: 3px 0 0 3px;
  }

  .usage-bar-fill:last-child {
    border-radius: 0 3px 3px 0;
  }

  .usage-bar-fill:only-child {
    border-radius: 3px;
  }

  .fill-staging {
    background: var(--system-accent);
  }

  .fill-deployed {
    background: var(--green);
  }

  .fill-ok {
    background: var(--green);
  }

  .fill-warning {
    background: var(--yellow);
  }

  .fill-critical {
    background: var(--red);
  }

  .usage-bar-legend {
    display: flex;
    gap: var(--space-3);
    margin-top: var(--space-2);
  }

  .legend-item {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 10px;
    color: var(--text-tertiary);
  }

  .legend-dot {
    width: 8px;
    height: 8px;
    border-radius: 2px;
  }

  .dot-staging { background: var(--system-accent); }
  .dot-deployed { background: var(--green); }
  .dot-free { background: var(--separator-opaque); }

  /* ---- Stats Grid ---- */

  .budget-grid {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: var(--space-4);
  }

  .budget-item {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .budget-item.span-2 {
    grid-column: span 2;
  }

  .budget-label {
    font-size: 11px;
    color: var(--text-tertiary);
    font-weight: 500;
  }

  .budget-value {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .budget-value-lg {
    font-size: 16px;
  }

  .value-highlight-green {
    color: var(--green);
  }

  /* ---- Actions ---- */

  .budget-actions {
    display: flex;
    justify-content: flex-end;
    margin-top: var(--space-4);
    padding-top: var(--space-3);
    border-top: 1px solid var(--separator);
  }

  .btn-budget-refresh {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: var(--space-1) var(--space-3);
    font-size: 12px;
    font-weight: 500;
    background: transparent;
    color: var(--text-tertiary);
    border: none;
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
    white-space: nowrap;
  }

  .btn-budget-refresh:hover:not(:disabled) {
    background: var(--surface-hover);
    color: var(--text-secondary);
  }

  .btn-budget-refresh:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* ---- Loading / Empty ---- */

  .budget-loading {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-4) 0;
    font-size: 13px;
    color: var(--text-tertiary);
  }

  .budget-spinner {
    width: 16px;
    height: 16px;
    border: 2px solid var(--separator);
    border-top-color: var(--system-accent);
    border-radius: 50%;
    animation: budget-spin 0.9s cubic-bezier(0.4, 0, 0.2, 1) infinite;
    flex-shrink: 0;
  }

  @keyframes budget-spin {
    to {
      transform: rotate(360deg);
    }
  }

  .budget-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-4) 0;
    text-align: center;
  }

  .budget-empty-text {
    font-size: 12px;
    color: var(--text-tertiary);
  }
</style>
