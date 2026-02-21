<script lang="ts">
  import {
    getDeploymentHealth,
    getConflicts,
    checkDependencyIssues,
    getDiskBudget,
    runPreflightCheck,
  } from "$lib/api";
  import type {
    DeploymentHealth,
    FileConflict,
    DependencyIssue,
    DiskBudget,
    PreflightResult,
  } from "$lib/types";
  import { selectedGame } from "$lib/stores";

  // ---- State ----

  let loading = $state(true);
  let health = $state<DeploymentHealth | null>(null);
  let conflicts = $state<FileConflict[]>([]);
  let dependencies = $state<DependencyIssue[]>([]);
  let diskBudget = $state<DiskBudget | null>(null);
  let preflight = $state<PreflightResult | null>(null);

  const game = $derived($selectedGame);

  // ---- Load all health data ----

  async function loadHealth() {
    if (!game) return;
    loading = true;
    try {
      const [h, c, d, db, pf] = await Promise.allSettled([
        getDeploymentHealth(game.game_id, game.bottle_name),
        getConflicts(game.game_id, game.bottle_name),
        checkDependencyIssues(game.game_id, game.bottle_name),
        getDiskBudget(game.game_id, game.bottle_name),
        runPreflightCheck(game.game_id, game.bottle_name),
      ]);
      health = h.status === "fulfilled" ? h.value : null;
      conflicts = c.status === "fulfilled" ? c.value : [];
      dependencies = d.status === "fulfilled" ? d.value : [];
      diskBudget = db.status === "fulfilled" ? db.value : null;
      preflight = pf.status === "fulfilled" ? pf.value : null;
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    if (game) {
      loadHealth();
    }
  });

  // ---- Derived metrics ----

  const overallScore = $derived.by(() => {
    if (!health && !preflight) return null;
    let score = 100;
    if (conflicts.length > 0) score -= Math.min(conflicts.length * 5, 30);
    if (dependencies.length > 0) score -= Math.min(dependencies.length * 10, 30);
    if (preflight) score -= preflight.failed * 15 + preflight.warnings * 5;
    if (health && !health.is_deployed) score -= 10;
    return Math.max(0, score);
  });

  const scoreColor = $derived(
    overallScore === null ? "var(--text-tertiary)" :
    overallScore >= 80 ? "var(--green)" :
    overallScore >= 50 ? "var(--yellow)" :
    "var(--red)"
  );

  const scoreLabel = $derived(
    overallScore === null ? "Unknown" :
    overallScore >= 80 ? "Healthy" :
    overallScore >= 50 ? "Needs Attention" :
    "Issues Found"
  );

  function formatBytes(bytes: number): string {
    if (bytes === 0) return "0 B";
    const k = 1024;
    const sizes = ["B", "KB", "MB", "GB"];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return `${(bytes / Math.pow(k, i)).toFixed(1)} ${sizes[i]}`;
  }
</script>

<div class="health-dashboard">
  <div class="health-header">
    <h3 class="health-title">Mod Health</h3>
    <button class="btn-refresh" onclick={loadHealth} disabled={loading}>
      {#if loading}
        <span class="spinner-xs"></span>
      {:else}
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="23 4 23 10 17 10" />
          <path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10" />
        </svg>
      {/if}
    </button>
  </div>

  {#if loading && !health}
    <div class="health-loading">
      <span class="spinner-sm"></span>
      <span>Checking mod health...</span>
    </div>
  {:else}
    <!-- Overall Score -->
    {#if overallScore !== null}
      <div class="score-card">
        <div class="score-ring" style="--score-color: {scoreColor}">
          <span class="score-value">{overallScore}</span>
        </div>
        <div class="score-info">
          <span class="score-label" style="color: {scoreColor}">{scoreLabel}</span>
          <span class="score-detail">{game?.display_name ?? "Game"}</span>
        </div>
      </div>
    {/if}

    <!-- Status Cards Grid -->
    <div class="status-grid">
      <!-- Deployment -->
      <div class="status-card">
        <div class="status-icon" class:status-ok={health?.is_deployed} class:status-warn={health && !health.is_deployed}>
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="20 6 9 17 4 12" />
          </svg>
        </div>
        <div class="status-text">
          <span class="status-label">Deployment</span>
          {#if health}
            <span class="status-value">{health.is_deployed ? `${health.total_deployed} files` : "Not deployed"}</span>
            <span class="status-meta">{health.deploy_method}</span>
          {:else}
            <span class="status-value status-dim">No data</span>
          {/if}
        </div>
      </div>

      <!-- Conflicts -->
      <div class="status-card">
        <div class="status-icon" class:status-ok={conflicts.length === 0} class:status-warn={conflicts.length > 0}>
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
            <line x1="12" y1="9" x2="12" y2="13" />
            <line x1="12" y1="17" x2="12.01" y2="17" />
          </svg>
        </div>
        <div class="status-text">
          <span class="status-label">Conflicts</span>
          <span class="status-value">{conflicts.length === 0 ? "None" : `${conflicts.length} file${conflicts.length !== 1 ? "s" : ""}`}</span>
        </div>
      </div>

      <!-- Dependencies -->
      <div class="status-card">
        <div class="status-icon" class:status-ok={dependencies.length === 0} class:status-error={dependencies.length > 0}>
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="10" />
            <line x1="12" y1="8" x2="12" y2="12" />
            <line x1="12" y1="16" x2="12.01" y2="16" />
          </svg>
        </div>
        <div class="status-text">
          <span class="status-label">Dependencies</span>
          <span class="status-value">{dependencies.length === 0 ? "OK" : `${dependencies.length} issue${dependencies.length !== 1 ? "s" : ""}`}</span>
        </div>
      </div>

      <!-- Disk Usage -->
      <div class="status-card">
        <div class="status-icon" class:status-ok={diskBudget !== null}>
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="2" y="2" width="20" height="8" rx="2" ry="2" />
            <rect x="2" y="14" width="20" height="8" rx="2" ry="2" />
            <line x1="6" y1="6" x2="6.01" y2="6" />
            <line x1="6" y1="18" x2="6.01" y2="18" />
          </svg>
        </div>
        <div class="status-text">
          <span class="status-label">Disk Usage</span>
          {#if diskBudget}
            <span class="status-value">{formatBytes(diskBudget.staging_bytes)}</span>
            <span class="status-meta">{formatBytes(diskBudget.available_bytes)} free</span>
          {:else}
            <span class="status-value status-dim">No data</span>
          {/if}
        </div>
      </div>
    </div>

    <!-- Pre-flight Checks Summary -->
    {#if preflight && (preflight.failed > 0 || preflight.warnings > 0)}
      <div class="preflight-summary">
        <span class="preflight-title">Pre-flight Checks</span>
        <div class="preflight-badges">
          {#if preflight.passed > 0}
            <span class="pf-badge pf-pass">{preflight.passed} passed</span>
          {/if}
          {#if preflight.warnings > 0}
            <span class="pf-badge pf-warn">{preflight.warnings} warning{preflight.warnings !== 1 ? "s" : ""}</span>
          {/if}
          {#if preflight.failed > 0}
            <span class="pf-badge pf-fail">{preflight.failed} failed</span>
          {/if}
        </div>
      </div>
    {/if}

    <!-- Dependency Issues List -->
    {#if dependencies.length > 0}
      <div class="issues-list">
        <span class="issues-title">Dependency Issues</span>
        {#each dependencies.slice(0, 5) as issue}
          <div class="issue-row">
            <span class="issue-type" class:issue-missing={issue.issue_type === "missing_requirement"} class:issue-conflict={issue.issue_type === "active_conflict"} class:issue-orphan={issue.issue_type === "orphaned_patch"}>
              {issue.issue_type === "missing_requirement" ? "Missing" : issue.issue_type === "active_conflict" ? "Conflict" : "Orphan"}
            </span>
            <span class="issue-text">{issue.message}</span>
          </div>
        {/each}
        {#if dependencies.length > 5}
          <span class="issues-more">+{dependencies.length - 5} more</span>
        {/if}
      </div>
    {/if}
  {/if}
</div>

<style>
  .health-dashboard {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .health-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .health-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .btn-refresh {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border-radius: var(--radius-sm);
    color: var(--text-tertiary);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
  }

  .btn-refresh:hover:not(:disabled) {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .btn-refresh:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .health-loading {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-4);
    font-size: 13px;
    color: var(--text-tertiary);
    justify-content: center;
  }

  /* Score Card */
  .score-card {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3);
    background: var(--surface);
    border-radius: var(--radius);
  }

  .score-ring {
    width: 44px;
    height: 44px;
    border-radius: 50%;
    border: 3px solid var(--score-color);
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .score-value {
    font-size: 15px;
    font-weight: 700;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .score-info {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .score-label {
    font-size: 14px;
    font-weight: 600;
  }

  .score-detail {
    font-size: 11px;
    color: var(--text-tertiary);
  }

  /* Status Grid */
  .status-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-2);
  }

  .status-card {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: var(--surface);
    border-radius: var(--radius-sm);
  }

  .status-icon {
    width: 28px;
    height: 28px;
    border-radius: var(--radius-sm);
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    background: var(--surface-hover);
    color: var(--text-tertiary);
  }

  .status-ok {
    background: var(--green-subtle);
    color: var(--green);
  }

  .status-warn {
    background: var(--yellow-subtle);
    color: var(--yellow);
  }

  .status-error {
    background: var(--red-subtle);
    color: var(--red);
  }

  .status-text {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }

  .status-label {
    font-size: 11px;
    font-weight: 500;
    color: var(--text-tertiary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .status-value {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-primary);
  }

  .status-dim {
    color: var(--text-quaternary);
  }

  .status-meta {
    font-size: 11px;
    color: var(--text-quaternary);
  }

  /* Preflight Summary */
  .preflight-summary {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-2) var(--space-3);
    background: var(--surface);
    border-radius: var(--radius-sm);
  }

  .preflight-title {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
  }

  .preflight-badges {
    display: flex;
    gap: var(--space-1);
  }

  .pf-badge {
    padding: 1px 8px;
    border-radius: 100px;
    font-size: 10px;
    font-weight: 600;
  }

  .pf-pass {
    background: var(--green-subtle);
    color: var(--green);
  }

  .pf-warn {
    background: var(--yellow-subtle);
    color: var(--yellow);
  }

  .pf-fail {
    background: var(--red-subtle);
    color: var(--red);
  }

  /* Issues List */
  .issues-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-2) var(--space-3);
    background: var(--surface);
    border-radius: var(--radius-sm);
  }

  .issues-title {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-secondary);
    margin-bottom: var(--space-1);
  }

  .issue-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: 12px;
  }

  .issue-type {
    flex-shrink: 0;
    padding: 1px 6px;
    border-radius: var(--radius-sm);
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
  }

  .issue-missing {
    background: var(--red-subtle);
    color: var(--red);
  }

  .issue-conflict {
    background: var(--yellow-subtle);
    color: var(--yellow);
  }

  .issue-orphan {
    background: var(--system-accent-subtle);
    color: var(--system-accent);
  }

  .issue-text {
    color: var(--text-secondary);
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .issues-more {
    font-size: 11px;
    color: var(--text-quaternary);
    padding-top: var(--space-1);
  }

  /* Spinners */
  .spinner-xs {
    display: inline-block;
    width: 14px;
    height: 14px;
    border: 2px solid var(--separator-opaque);
    border-top-color: var(--system-accent);
    border-radius: 50%;
    animation: spin 0.75s linear infinite;
  }

  .spinner-sm {
    display: inline-block;
    width: 14px;
    height: 14px;
    border: 2px solid var(--separator-opaque);
    border-top-color: var(--system-accent);
    border-radius: 50%;
    animation: spin 0.75s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }
</style>
