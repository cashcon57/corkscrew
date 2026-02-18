<script lang="ts">
  import { onMount } from "svelte";
  import { getPluginOrder } from "$lib/api";
  import { selectedGame, games, showError } from "$lib/stores";
  import type { PluginEntry, DetectedGame } from "$lib/types";

  let plugins = $state<PluginEntry[]>([]);
  let loading = $state(false);

  let gameList = $state<DetectedGame[]>([]);
  games.subscribe((g) => (gameList = g));

  $effect(() => {
    if ($selectedGame) {
      loadPlugins($selectedGame);
    }
  });

  async function loadPlugins(game: DetectedGame) {
    loading = true;
    try {
      plugins = await getPluginOrder(game.game_id, game.bottle_name);
    } catch (e: any) {
      showError(`Failed to load plugins: ${e}`);
      plugins = [];
    } finally {
      loading = false;
    }
  }

  function getPluginType(entry: PluginEntry): string {
    const name = entry.filename.toLowerCase();
    if (name.endsWith(".esm")) return "ESM";
    if (name.endsWith(".esl")) return "ESL";
    return "ESP";
  }

  const enabledCount = $derived(plugins.filter((p) => p.enabled).length);
</script>

<div class="plugins-page">
  <!-- Page header -->
  <div class="page-header">
    <div class="header-title">
      <h2>Load Order</h2>
      {#if $selectedGame}
        <span class="header-context">
          {$selectedGame.display_name}
          <span class="header-separator">/</span>
          {$selectedGame.bottle_name}
        </span>
      {/if}
    </div>
    {#if plugins.length > 0}
      <div class="header-meta">
        <span class="meta-chip">
          <span class="meta-value">{enabledCount}</span>
          <span class="meta-label">active</span>
        </span>
        <span class="meta-divider"></span>
        <span class="meta-chip">
          <span class="meta-value">{plugins.length}</span>
          <span class="meta-label">total</span>
        </span>
      </div>
    {/if}
  </div>

  <!-- No game selected -->
  {#if !$selectedGame}
    <div class="empty-state">
      <div class="empty-icon">
        <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
          <polyline points="14 2 14 8 20 8" />
          <line x1="16" y1="13" x2="8" y2="13" />
          <line x1="16" y1="17" x2="8" y2="17" />
          <polyline points="10 9 9 9 8 9" />
        </svg>
      </div>
      <p class="empty-title">No game selected</p>
      <p class="empty-description">
        Select a game from the Dashboard or Mods page to view its plugin load order.
      </p>
    </div>

  <!-- Loading -->
  {:else if loading}
    <div class="loading-state">
      <div class="spinner"></div>
      <p class="loading-text">Loading plugins...</p>
    </div>

  <!-- No plugins found -->
  {:else if plugins.length === 0}
    <div class="empty-state">
      <div class="empty-icon">
        <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="10" />
          <line x1="8" y1="12" x2="16" y2="12" />
        </svg>
      </div>
      <p class="empty-title">No plugins found</p>
      <p class="empty-description">
        Install mods containing .esp, .esm, or .esl files to manage their load order here.
      </p>
    </div>

  <!-- Plugin list -->
  {:else}
    <div class="list-container">
      <div class="list-header">
        <span class="col-index">#</span>
        <span class="col-plugin">Plugin</span>
        <span class="col-type">Type</span>
        <span class="col-status">Status</span>
      </div>

      <div class="list-body">
        {#each plugins as plugin, i}
          <div
            class="list-row"
            class:row-disabled={!plugin.enabled}
          >
            <span class="col-index">
              <span class="index-num">{String(i).padStart(2, "0")}</span>
            </span>
            <span class="col-plugin">
              <span class="plugin-filename">{plugin.filename}</span>
            </span>
            <span class="col-type">
              {#if getPluginType(plugin) === "ESM"}
                <span class="type-label type-esm">ESM</span>
              {:else if getPluginType(plugin) === "ESL"}
                <span class="type-label type-esl">ESL</span>
              {:else}
                <span class="type-label type-esp">ESP</span>
              {/if}
            </span>
            <span class="col-status">
              {#if plugin.enabled}
                <span class="status-active">Active</span>
              {:else}
                <span class="status-disabled">Disabled</span>
              {/if}
            </span>
          </div>
        {/each}
      </div>
    </div>

    <div class="list-footer">
      <span class="footer-text">{plugins.length} plugins loaded</span>
    </div>
  {/if}
</div>

<style>
  /* ---- Page layout ---- */

  .plugins-page {
    max-width: 760px;
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
  }

  /* ---- Header ---- */

  .page-header {
    display: flex;
    justify-content: space-between;
    align-items: flex-end;
    gap: var(--space-4);
  }

  .header-title h2 {
    font-size: 22px;
    font-weight: 700;
    letter-spacing: -0.02em;
    color: var(--text-primary);
  }

  .header-context {
    display: block;
    font-size: 13px;
    color: var(--text-secondary);
    margin-top: var(--space-1);
  }

  .header-separator {
    color: var(--text-quaternary);
    margin: 0 var(--space-1);
  }

  .header-meta {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-shrink: 0;
  }

  .meta-chip {
    display: flex;
    align-items: baseline;
    gap: 5px;
  }

  .meta-value {
    font-size: 16px;
    font-weight: 700;
    font-family: var(--font-mono);
    color: var(--text-primary);
    letter-spacing: -0.02em;
  }

  .meta-label {
    font-size: 11px;
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-tertiary);
  }

  .meta-divider {
    width: 1px;
    height: 16px;
    background: var(--separator-opaque);
  }

  /* ---- List container ---- */

  .list-container {
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-lg);
    overflow: hidden;
  }

  /* ---- List header ---- */

  .list-header {
    display: grid;
    grid-template-columns: 48px 1fr 64px 80px;
    padding: var(--space-2) var(--space-4);
    background: var(--bg-secondary);
    border-bottom: 1px solid var(--separator);
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-tertiary);
    align-items: center;
  }

  /* ---- List body ---- */

  .list-body {
    max-height: calc(100vh - 240px);
    overflow-y: auto;
  }

  /* ---- List row ---- */

  .list-row {
    display: grid;
    grid-template-columns: 48px 1fr 64px 80px;
    padding: var(--space-2) var(--space-4);
    border-bottom: 1px solid var(--separator);
    align-items: center;
    transition: background var(--duration-fast) var(--ease);
  }

  .list-row:last-child {
    border-bottom: none;
  }

  .list-row:hover {
    background: var(--surface-hover);
  }

  .list-row.row-disabled {
    opacity: 0.45;
  }

  .list-row.row-disabled:hover {
    opacity: 0.55;
  }

  /* ---- Columns ---- */

  .col-index {
    display: flex;
    align-items: center;
  }

  .index-num {
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 500;
    color: var(--text-quaternary);
    letter-spacing: 0;
    min-width: 20px;
  }

  .col-plugin {
    min-width: 0;
    overflow: hidden;
  }

  .plugin-filename {
    font-family: var(--font-mono);
    font-size: 12.5px;
    font-weight: 500;
    color: var(--text-primary);
    letter-spacing: -0.01em;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    display: block;
  }

  .col-type {
    display: flex;
    align-items: center;
  }

  .type-label {
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }

  .type-esm {
    color: var(--yellow);
  }

  .type-esp {
    color: var(--accent);
  }

  .type-esl {
    color: var(--green);
  }

  .col-status {
    display: flex;
    align-items: center;
    justify-content: flex-end;
  }

  .status-active {
    font-size: 12px;
    font-weight: 500;
    color: var(--green);
  }

  .status-disabled {
    font-size: 12px;
    font-weight: 400;
    color: var(--text-tertiary);
  }

  /* ---- Footer ---- */

  .list-footer {
    display: flex;
    justify-content: center;
  }

  .footer-text {
    font-size: 12px;
    font-weight: 400;
    color: var(--text-tertiary);
    letter-spacing: 0.01em;
  }

  /* ---- Empty state ---- */

  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-12) var(--space-8);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-lg);
    text-align: center;
  }

  .empty-icon {
    color: var(--text-quaternary);
    margin-bottom: var(--space-1);
  }

  .empty-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .empty-description {
    font-size: 13px;
    color: var(--text-tertiary);
    max-width: 340px;
    line-height: 1.5;
  }

  /* ---- Loading state ---- */

  .loading-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-4);
    padding: var(--space-12);
  }

  .loading-text {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-tertiary);
  }

  .spinner {
    width: 28px;
    height: 28px;
    border: 2.5px solid var(--separator-opaque);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.75s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
