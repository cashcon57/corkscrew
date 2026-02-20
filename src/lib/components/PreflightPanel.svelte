<script lang="ts">
  import { runPreflightCheck } from "$lib/api";
  import type { PreflightResult } from "$lib/types";

  interface Props {
    gameId: string;
    bottleName: string;
    onComplete?: (canProceed: boolean) => void;
  }

  let { gameId, bottleName, onComplete }: Props = $props();
  let result = $state<PreflightResult | null>(null);
  let loading = $state(false);
  let expandedChecks = $state<Set<string>>(new Set());

  const summary = $derived.by(() => {
    if (!result) return null;
    return {
      passed: result.passed,
      warnings: result.warnings,
      failed: result.failed,
      total: result.checks.length,
    };
  });

  function toggleCheck(name: string) {
    const next = new Set(expandedChecks);
    if (next.has(name)) {
      next.delete(name);
    } else {
      next.add(name);
    }
    expandedChecks = next;
  }

  function isExpanded(name: string): boolean {
    return expandedChecks.has(name);
  }

  async function runCheck() {
    loading = true;
    try {
      result = await runPreflightCheck(gameId, bottleName);
      onComplete?.(result.can_proceed);
    } catch {
      result = null;
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    if (gameId && bottleName) runCheck();
  });
</script>

<div class="preflight-panel">
  <!-- Header -->
  <div class="preflight-header">
    <div class="preflight-title-row">
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
      </svg>
      <h4 class="preflight-title">Pre-Deployment Checks</h4>
      {#if summary}
        <span class="preflight-count-badge" class:badge-green={summary.failed === 0} class:badge-red={summary.failed > 0}>
          {summary.total}
        </span>
      {/if}
    </div>
    {#if summary}
      <p class="preflight-summary">
        <span class="summary-passed">{summary.passed} passed</span>
        {#if summary.warnings > 0}
          <span class="summary-sep">,</span>
          <span class="summary-warnings">{summary.warnings} warning{summary.warnings !== 1 ? "s" : ""}</span>
        {/if}
        {#if summary.failed > 0}
          <span class="summary-sep">,</span>
          <span class="summary-failed">{summary.failed} failed</span>
        {/if}
      </p>
    {/if}
  </div>

  <!-- Content -->
  {#if loading}
    <div class="preflight-loading">
      <div class="preflight-spinner"></div>
      <span>Running pre-deployment checks...</span>
    </div>
  {:else if result}
    <!-- Check List -->
    <div class="check-list">
      {#each result.checks as check (check.name)}
        <div class="check-group">
          <button
            class="check-row"
            onclick={() => { if (check.detail) toggleCheck(check.name); }}
            class:check-expandable={!!check.detail}
            aria-expanded={check.detail ? isExpanded(check.name) : undefined}
            type="button"
          >
            <!-- Status Icon -->
            <div class="check-icon" class:icon-pass={check.status === "pass"} class:icon-warning={check.status === "warning"} class:icon-fail={check.status === "fail"}>
              {#if check.status === "pass"}
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M20 6L9 17l-5-5" />
                </svg>
              {:else if check.status === "warning"}
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
                  <line x1="12" y1="9" x2="12" y2="13" />
                  <line x1="12" y1="17" x2="12.01" y2="17" />
                </svg>
              {:else}
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <circle cx="12" cy="12" r="10" />
                  <line x1="15" y1="9" x2="9" y2="15" />
                  <line x1="9" y1="9" x2="15" y2="15" />
                </svg>
              {/if}
            </div>

            <!-- Check Info -->
            <div class="check-info">
              <span class="check-name">{check.name}</span>
              <span class="check-message">{check.message}</span>
            </div>

            <!-- Chevron (only if detail exists) -->
            {#if check.detail}
              <svg
                class="check-chevron"
                class:check-chevron-open={isExpanded(check.name)}
                width="10"
                height="10"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2.5"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <polyline points="6 9 12 15 18 9" />
              </svg>
            {/if}
          </button>

          <!-- Expanded Detail -->
          {#if check.detail && isExpanded(check.name)}
            <div class="check-detail">
              <p class="check-detail-text">{check.detail}</p>
            </div>
          {/if}
        </div>
      {/each}
    </div>

    <!-- Proceed Banner -->
    <div class="preflight-banner" class:banner-ready={result.can_proceed} class:banner-blocked={!result.can_proceed}>
      {#if result.can_proceed}
        <div class="banner-icon banner-icon-ready">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M20 6L9 17l-5-5" />
          </svg>
        </div>
        <span class="banner-text">Ready to deploy</span>
      {:else}
        <div class="banner-icon banner-icon-blocked">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="10" />
            <line x1="4.93" y1="4.93" x2="19.07" y2="19.07" />
          </svg>
        </div>
        <span class="banner-text">Issues need attention</span>
      {/if}
      <button class="btn-rerun" onclick={runCheck} disabled={loading} type="button">
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M21 2v6h-6M3 12a9 9 0 0 1 15-6.7L21 8M3 22v-6h6M21 12a9 9 0 0 1-15 6.7L3 16" />
        </svg>
        Re-run
      </button>
    </div>
  {:else}
    <!-- Error state -->
    <div class="preflight-empty">
      <div class="empty-icon">
        <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="10" />
          <line x1="15" y1="9" x2="9" y2="15" />
          <line x1="9" y1="9" x2="15" y2="15" />
        </svg>
      </div>
      <p class="empty-title">Pre-flight check failed</p>
      <p class="empty-description">Could not run deployment checks.</p>
      <button class="btn-rerun" onclick={runCheck} disabled={loading} type="button">
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M21 2v6h-6M3 12a9 9 0 0 1 15-6.7L21 8M3 22v-6h6M21 12a9 9 0 0 1-15 6.7L3 16" />
        </svg>
        Retry
      </button>
    </div>
  {/if}
</div>

<style>
  /* ---- Panel Container ---- */

  .preflight-panel {
    background: var(--bg-grouped-secondary);
    border-radius: var(--radius-lg);
    overflow: hidden;
    box-shadow: var(--glass-edge-shadow);
  }

  /* ---- Header ---- */

  .preflight-header {
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--separator);
    background: rgba(255, 255, 255, 0.03);
  }

  .preflight-title-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .preflight-title-row > svg {
    color: var(--system-accent);
    flex-shrink: 0;
  }

  .preflight-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .preflight-count-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 20px;
    height: 20px;
    padding: 0 6px;
    border-radius: 100px;
    font-size: 11px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    margin-left: auto;
  }

  .badge-green {
    color: var(--green);
    background: color-mix(in srgb, var(--green) 15%, transparent);
  }

  .badge-red {
    color: var(--red);
    background: color-mix(in srgb, var(--red) 15%, transparent);
  }

  .preflight-summary {
    font-size: 12px;
    color: var(--text-tertiary);
    margin-top: var(--space-1);
  }

  .summary-passed {
    color: var(--green);
    font-weight: 500;
  }

  .summary-sep {
    color: var(--text-quaternary);
    margin: 0 2px;
  }

  .summary-warnings {
    color: var(--yellow);
    font-weight: 500;
  }

  .summary-failed {
    color: var(--red);
    font-weight: 500;
  }

  /* ---- Loading ---- */

  .preflight-loading {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-5) var(--space-4);
    font-size: 13px;
    color: var(--text-tertiary);
  }

  .preflight-spinner {
    width: 16px;
    height: 16px;
    border: 2px solid var(--separator);
    border-top-color: var(--system-accent);
    border-radius: 50%;
    animation: preflight-spin 0.9s cubic-bezier(0.4, 0, 0.2, 1) infinite;
    flex-shrink: 0;
  }

  @keyframes preflight-spin {
    to {
      transform: rotate(360deg);
    }
  }

  /* ---- Check List ---- */

  .check-list {
    overflow-y: auto;
    max-height: 400px;
  }

  .check-group {
    border-bottom: 1px solid var(--separator);
  }

  .check-group:last-child {
    border-bottom: none;
  }

  .check-row {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    width: 100%;
    padding: var(--space-3) var(--space-4);
    text-align: left;
    background: none;
    border: none;
    transition: background var(--duration-fast) var(--ease);
  }

  .check-row.check-expandable {
    cursor: pointer;
  }

  .check-row.check-expandable:hover {
    background: var(--surface-hover);
  }

  .check-row:not(.check-expandable) {
    cursor: default;
  }

  /* ---- Status Icons ---- */

  .check-icon {
    flex-shrink: 0;
    width: 22px;
    height: 22px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 50%;
    margin-top: 1px;
  }

  .icon-pass {
    color: var(--green);
    background: color-mix(in srgb, var(--green) 12%, transparent);
  }

  .icon-warning {
    color: var(--yellow);
    background: color-mix(in srgb, var(--yellow) 12%, transparent);
  }

  .icon-fail {
    color: var(--red);
    background: color-mix(in srgb, var(--red) 12%, transparent);
  }

  /* ---- Check Info ---- */

  .check-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .check-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    line-height: 1.3;
  }

  .check-message {
    font-size: 12px;
    color: var(--text-secondary);
    line-height: 1.4;
  }

  /* ---- Chevron ---- */

  .check-chevron {
    flex-shrink: 0;
    color: var(--text-tertiary);
    transition: transform var(--duration-fast) var(--ease);
    margin-top: 5px;
    transform: rotate(-90deg);
  }

  .check-chevron-open {
    transform: rotate(0deg);
  }

  /* ---- Check Detail (expanded) ---- */

  .check-detail {
    padding: 0 var(--space-4) var(--space-3);
    padding-left: calc(var(--space-4) + 22px + var(--space-3));
  }

  .check-detail-text {
    font-size: 12px;
    line-height: 1.5;
    color: var(--text-tertiary);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-sm);
    padding: var(--space-2) var(--space-3);
    white-space: pre-wrap;
    word-break: break-word;
  }

  /* ---- Proceed Banner ---- */

  .preflight-banner {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-top: 1px solid var(--separator);
  }

  .banner-ready {
    background: color-mix(in srgb, var(--green) 6%, transparent);
  }

  .banner-blocked {
    background: color-mix(in srgb, var(--red) 6%, transparent);
  }

  .banner-icon {
    flex-shrink: 0;
    width: 24px;
    height: 24px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 50%;
  }

  .banner-icon-ready {
    color: var(--green);
    background: color-mix(in srgb, var(--green) 15%, transparent);
  }

  .banner-icon-blocked {
    color: var(--red);
    background: color-mix(in srgb, var(--red) 15%, transparent);
  }

  .banner-text {
    flex: 1;
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .btn-rerun {
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
    flex-shrink: 0;
  }

  .btn-rerun:hover:not(:disabled) {
    background: var(--surface-hover);
    color: var(--text-secondary);
  }

  .btn-rerun:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* ---- Empty / Error State ---- */

  .preflight-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-8);
    text-align: center;
  }

  .preflight-empty .empty-icon {
    color: var(--text-quaternary);
    margin-bottom: var(--space-1);
  }

  .preflight-empty .empty-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .preflight-empty .empty-description {
    font-size: 12px;
    color: var(--text-tertiary);
    margin-bottom: var(--space-2);
  }
</style>
