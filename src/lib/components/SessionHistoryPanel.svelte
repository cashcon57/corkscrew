<script lang="ts">
  import { getSessionHistory, getStabilitySummary } from "$lib/api";
  import type { GameSession, StabilitySummary } from "$lib/types";

  interface Props {
    gameId: string;
    bottleName: string;
  }

  let { gameId, bottleName }: Props = $props();
  let sessions = $state<GameSession[]>([]);
  let stability = $state<StabilitySummary | null>(null);
  let loading = $state(false);

  function formatDuration(secs: number | null): string {
    if (!secs) return "\u2014";
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    if (h > 0) return `${h}h ${m}m`;
    return `${m}m`;
  }

  function formatDate(iso: string): string {
    return new Date(iso).toLocaleDateString(undefined, {
      month: "short", day: "numeric", hour: "2-digit", minute: "2-digit"
    });
  }

  async function load() {
    loading = true;
    try {
      sessions = await getSessionHistory(gameId, bottleName, 20);
      stability = await getStabilitySummary(gameId, bottleName);
    } catch {} finally { loading = false; }
  }

  $effect(() => { if (gameId && bottleName) load(); });
</script>

<div class="session-panel">
  <!-- Stability Summary -->
  <div class="section-card">
    <div class="card-header">
      <h4 class="card-title">
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
        </svg>
        Stability Summary
      </h4>
    </div>

    {#if loading}
      <div class="card-loading">
        <span class="spinner-sm"></span>
        <span class="loading-label">Loading session data...</span>
      </div>
    {:else if stability}
      <div class="summary-grid">
        <div class="summary-stat">
          <span class="stat-value">{stability.total_sessions}</span>
          <span class="stat-label">Sessions</span>
        </div>
        <div class="summary-stat">
          <span class="stat-value stat-green">{stability.clean_exits}</span>
          <span class="stat-label">Clean Exits</span>
        </div>
        <div class="summary-stat">
          <span class="stat-value stat-red">{stability.crashes}</span>
          <span class="stat-label">Crashes</span>
        </div>
        <div class="summary-stat">
          <span class="stat-value">{formatDuration(stability.avg_duration_secs)}</span>
          <span class="stat-label">Avg Duration</span>
        </div>
        <div class="summary-stat summary-stat-wide">
          <span class="stat-value stat-secondary">
            {stability.last_stable_session ? formatDate(stability.last_stable_session) : "\u2014"}
          </span>
          <span class="stat-label">Last Stable</span>
        </div>
      </div>

      <!-- Mod changes warning -->
      {#if stability.mods_since_last_stable.length > 0}
        <div class="mod-warning" role="alert">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
            <line x1="12" y1="9" x2="12" y2="13" />
            <line x1="12" y1="17" x2="12.01" y2="17" />
          </svg>
          <span>
            {stability.mods_since_last_stable.length} mod change{stability.mods_since_last_stable.length !== 1 ? "s" : ""} since last stable session
          </span>
        </div>
      {/if}
    {:else}
      <div class="card-empty-inline">
        <span class="empty-text">No stability data available.</span>
      </div>
    {/if}
  </div>

  <!-- Session List -->
  <div class="section-card">
    <div class="card-header">
      <h4 class="card-title">
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="10" />
          <polyline points="12 6 12 12 16 14" />
        </svg>
        Session History
      </h4>
      {#if sessions.length > 0}
        <span class="card-count">{sessions.length}</span>
      {/if}
    </div>

    {#if loading}
      <div class="card-loading">
        <span class="spinner-sm"></span>
        <span class="loading-label">Loading sessions...</span>
      </div>
    {:else if sessions.length === 0}
      <div class="card-empty">
        <div class="empty-icon">
          <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="10" />
            <polyline points="12 6 12 12 16 14" />
          </svg>
        </div>
        <p class="empty-title">No sessions recorded yet</p>
        <p class="empty-description">Launch the game to start tracking.</p>
      </div>
    {:else}
      <div class="session-list" role="list">
        {#each sessions as session (session.id)}
          <div class="session-row" role="listitem">
            <div class="session-date">{formatDate(session.started_at)}</div>
            <div class="session-duration">{formatDuration(session.duration_secs)}</div>
            <div class="session-status">
              {#if session.clean_exit === true}
                <span class="status-badge status-clean">Clean</span>
              {:else if session.clean_exit === false}
                <span class="status-badge status-crash">Crash</span>
              {:else}
                <span class="status-badge status-unknown">Unknown</span>
              {/if}
            </div>
            {#if session.notes}
              <div class="session-notes" title={session.notes}>{session.notes}</div>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  /* ---- Panel Container ---- */

  .session-panel {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  /* ---- Section Card ---- */

  .section-card {
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-lg);
    overflow: hidden;
    box-shadow: var(--glass-edge-shadow);
  }

  .card-header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    background: rgba(255, 255, 255, 0.03);
    border-bottom: 1px solid var(--separator);
  }

  .card-title {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .card-title svg {
    color: var(--system-accent);
    flex-shrink: 0;
  }

  .card-count {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    border-radius: 100px;
    font-size: 10px;
    font-weight: 700;
    color: var(--text-tertiary);
    background: var(--bg-grouped-secondary);
    font-variant-numeric: tabular-nums;
    margin-left: auto;
  }

  /* ---- Loading ---- */

  .card-loading {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-4);
  }

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

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .loading-label {
    font-size: 12px;
    color: var(--text-tertiary);
  }

  /* ---- Empty States ---- */

  .card-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-8);
    text-align: center;
  }

  .card-empty-inline {
    padding: var(--space-4);
    text-align: center;
  }

  .empty-icon {
    color: var(--text-quaternary);
    margin-bottom: var(--space-1);
  }

  .empty-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .empty-description {
    font-size: 12px;
    color: var(--text-tertiary);
  }

  .empty-text {
    font-size: 12px;
    color: var(--text-tertiary);
  }

  /* ---- Stability Summary Grid ---- */

  .summary-grid {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 1px;
    background: var(--separator);
  }

  .summary-stat {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding: var(--space-3) var(--space-2);
    background: var(--surface);
  }

  .summary-stat-wide {
    grid-column: span 4;
  }

  .stat-value {
    font-size: 18px;
    font-weight: 700;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
    letter-spacing: -0.02em;
  }

  .stat-green {
    color: var(--green);
  }

  .stat-red {
    color: var(--red);
  }

  .stat-secondary {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .stat-label {
    font-size: 10px;
    font-weight: 600;
    color: var(--text-tertiary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  /* ---- Mod Warning ---- */

  .mod-warning {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    background: var(--yellow-subtle, rgba(255, 214, 10, 0.1));
    color: var(--yellow);
    font-size: 12px;
    font-weight: 500;
    line-height: 1.4;
    border-top: 1px solid var(--separator);
  }

  .mod-warning svg {
    flex-shrink: 0;
  }

  /* ---- Session List ---- */

  .session-list {
    overflow-y: auto;
    max-height: 400px;
  }

  .session-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-4);
    border-bottom: 1px solid var(--separator);
    transition: background var(--duration-fast) var(--ease);
  }

  .session-row:last-child {
    border-bottom: none;
  }

  .session-row:hover {
    background: var(--surface-hover);
  }

  .session-date {
    font-size: 12px;
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    min-width: 120px;
  }

  .session-duration {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-primary);
    font-family: var(--font-mono);
    letter-spacing: 0;
    min-width: 50px;
    text-align: right;
  }

  .session-notes {
    flex: 1;
    font-size: 11px;
    color: var(--text-tertiary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  /* ---- Status Badges ---- */

  .status-badge {
    display: inline-flex;
    align-items: center;
    padding: 1px 6px;
    border-radius: 4px;
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    flex-shrink: 0;
  }

  .status-clean {
    background: var(--green-subtle, rgba(52, 199, 89, 0.12));
    color: var(--green);
  }

  .status-crash {
    background: var(--red-subtle, rgba(255, 69, 58, 0.12));
    color: var(--red);
  }

  .status-unknown {
    background: var(--surface-hover);
    color: var(--text-tertiary);
  }
</style>
