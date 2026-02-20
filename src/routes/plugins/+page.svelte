<script lang="ts">
  import { onMount } from "svelte";
  import {
    getPluginOrder,
    sortPluginsLoot,
    updateLootMasterlist,
    togglePlugin,
    movePlugin,
  } from "$lib/api";
  import { selectedGame, games, showError } from "$lib/stores";
  import type { PluginEntry, DetectedGame, PluginWarning } from "$lib/types";

  let plugins = $state<PluginEntry[]>([]);
  let warnings = $state<PluginWarning[]>([]);
  let loading = $state(false);
  let sorting = $state(false);
  let updatingMasterlist = $state(false);
  let togglingPlugin = $state<string | null>(null);
  let sortMessage = $state<string | null>(null);

  let gameList = $state<DetectedGame[]>([]);
  games.subscribe((g) => (gameList = g));

  $effect(() => {
    if ($selectedGame) {
      loadPlugins($selectedGame);
    }
  });

  async function loadPlugins(game: DetectedGame) {
    loading = true;
    warnings = [];
    sortMessage = null;
    try {
      plugins = await getPluginOrder(game.game_id, game.bottle_name);
    } catch (e: any) {
      showError(`Failed to load plugins: ${e}`);
      plugins = [];
    } finally {
      loading = false;
    }
  }

  async function handleSort() {
    if (!$selectedGame || sorting) return;
    sorting = true;
    sortMessage = null;
    try {
      const result = await sortPluginsLoot(
        $selectedGame.game_id,
        $selectedGame.bottle_name
      );
      warnings = result.warnings;

      if (result.plugins_moved > 0) {
        // Reload plugins from disk to get the sorted order
        plugins = await getPluginOrder(
          $selectedGame.game_id,
          $selectedGame.bottle_name
        );
        sortMessage = `Sorted — ${result.plugins_moved} plugin${result.plugins_moved !== 1 ? "s" : ""} moved`;
      } else {
        sortMessage = "Load order is already optimal";
      }
    } catch (e: any) {
      showError(`LOOT sort failed: ${e}`);
    } finally {
      sorting = false;
    }
  }

  async function handleUpdateMasterlist() {
    if (!$selectedGame || updatingMasterlist) return;
    updatingMasterlist = true;
    try {
      await updateLootMasterlist($selectedGame.game_id);
      sortMessage = "Masterlist updated";
    } catch (e: any) {
      showError(`Masterlist update failed: ${e}`);
    } finally {
      updatingMasterlist = false;
    }
  }

  async function handleToggle(plugin: PluginEntry) {
    if (!$selectedGame || togglingPlugin) return;
    togglingPlugin = plugin.filename;
    try {
      plugins = await togglePlugin(
        $selectedGame.game_id,
        $selectedGame.bottle_name,
        plugin.filename,
        !plugin.enabled
      );
    } catch (e: any) {
      showError(`Failed to toggle plugin: ${e}`);
    } finally {
      togglingPlugin = null;
    }
  }

  async function handleMove(pluginName: string, direction: "up" | "down") {
    if (!$selectedGame) return;
    const currentIndex = plugins.findIndex(
      (p) => p.filename === pluginName
    );
    if (currentIndex < 0) return;

    const newIndex =
      direction === "up"
        ? Math.max(0, currentIndex - 1)
        : Math.min(plugins.length - 1, currentIndex + 1);

    if (newIndex === currentIndex) return;

    try {
      plugins = await movePlugin(
        $selectedGame.game_id,
        $selectedGame.bottle_name,
        pluginName,
        newIndex
      );
    } catch (e: any) {
      showError(`Failed to move plugin: ${e}`);
    }
  }

  function getPluginType(entry: PluginEntry): string {
    const name = entry.filename.toLowerCase();
    if (name.endsWith(".esm")) return "ESM";
    if (name.endsWith(".esl")) return "ESL";
    return "ESP";
  }

  function getWarningsForPlugin(pluginName: string): PluginWarning[] {
    return warnings.filter(
      (w) => w.plugin_name.toLowerCase() === pluginName.toLowerCase()
    );
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

  <!-- Toolbar -->
  {#if $selectedGame && plugins.length > 0}
    <div class="toolbar">
      <div class="toolbar-left">
        <button
          class="btn btn-accent"
          onclick={handleSort}
          disabled={sorting}
        >
          {#if sorting}
            <span class="btn-spinner"></span>
            Sorting...
          {:else}
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M3 6h18M6 12h12M9 18h6" />
            </svg>
            Sort with LOOT
          {/if}
        </button>
        <button
          class="btn btn-secondary"
          onclick={handleUpdateMasterlist}
          disabled={updatingMasterlist}
        >
          {#if updatingMasterlist}
            <span class="btn-spinner"></span>
            Updating...
          {:else}
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M21 2v6h-6M3 12a9 9 0 0 1 15-6.7L21 8M3 22v-6h6M21 12a9 9 0 0 1-15 6.7L3 16" />
            </svg>
            Update Masterlist
          {/if}
        </button>
      </div>
      {#if sortMessage}
        <span class="sort-message">{sortMessage}</span>
      {/if}
    </div>
  {/if}

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
        <span class="col-toggle"></span>
        <span class="col-plugin">Plugin</span>
        <span class="col-type">Type</span>
        <span class="col-actions">Order</span>
      </div>

      <div class="list-body">
        {#each plugins as plugin, i}
          {@const pluginWarnings = getWarningsForPlugin(plugin.filename)}
          <div
            class="list-row"
            class:row-disabled={!plugin.enabled}
            class:row-has-warning={pluginWarnings.some(w => w.level === "warn" || w.level === "error")}
          >
            <span class="col-index">
              <span class="index-num">{i}</span>
            </span>
            <span class="col-toggle">
              <button
                class="toggle-btn"
                class:toggle-on={plugin.enabled}
                onclick={() => handleToggle(plugin)}
                disabled={togglingPlugin === plugin.filename}
                title={plugin.enabled ? "Disable plugin" : "Enable plugin"}
              >
                <span class="toggle-thumb"></span>
              </button>
            </span>
            <span class="col-plugin">
              <span class="plugin-filename">{plugin.filename}</span>
              {#if pluginWarnings.length > 0}
                <span class="plugin-warnings">
                  {#each pluginWarnings as w}
                    <span
                      class="warning-badge"
                      class:warning-info={w.level === "info"}
                      class:warning-warn={w.level === "warn"}
                      class:warning-error={w.level === "error"}
                      title={w.message}
                    >
                      {#if w.level === "error"}!{:else if w.level === "warn"}⚠{:else}i{/if}
                    </span>
                  {/each}
                </span>
              {/if}
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
            <span class="col-actions">
              <button
                class="move-btn"
                onclick={() => handleMove(plugin.filename, "up")}
                disabled={i === 0}
                title="Move up"
              >
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                  <polyline points="18 15 12 9 6 15" />
                </svg>
              </button>
              <button
                class="move-btn"
                onclick={() => handleMove(plugin.filename, "down")}
                disabled={i === plugins.length - 1}
                title="Move down"
              >
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                  <polyline points="6 9 12 15 18 9" />
                </svg>
              </button>
            </span>
          </div>
          {#if pluginWarnings.length > 0}
            <div class="warning-row">
              {#each pluginWarnings as w}
                <div
                  class="warning-message"
                  class:warning-info={w.level === "info"}
                  class:warning-warn={w.level === "warn"}
                  class:warning-error={w.level === "error"}
                >
                  {w.message}
                </div>
              {/each}
            </div>
          {/if}
        {/each}
      </div>
    </div>
  {/if}
</div>

<style>
  /* ---- Page layout ---- */

  .plugins-page {
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

  /* ---- Toolbar ---- */

  .toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .toolbar-left {
    display: flex;
    gap: var(--space-2);
  }

  .btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 14px;
    font-size: 13px;
    font-weight: 500;
    border: none;
    border-radius: var(--radius-md);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
    white-space: nowrap;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-accent {
    background: var(--system-accent);
    color: white;
  }

  .btn-accent:hover:not(:disabled) {
    filter: brightness(1.1);
  }

  .btn-secondary {
    background: var(--surface-hover);
    color: var(--text-secondary);
  }

  .btn-secondary:hover:not(:disabled) {
    background: var(--surface-active);
    color: var(--text-primary);
  }

  .btn-spinner {
    width: 12px;
    height: 12px;
    border: 2px solid rgba(255, 255, 255, 0.3);
    border-top-color: white;
    border-radius: 50%;
    animation: spin 0.75s linear infinite;
  }

  .btn-secondary .btn-spinner {
    border-color: rgba(128, 128, 128, 0.3);
    border-top-color: var(--text-secondary);
  }

  .sort-message {
    font-size: 12px;
    font-weight: 500;
    color: var(--green);
    animation: fade-in 0.2s ease;
  }

  @keyframes fade-in {
    from { opacity: 0; transform: translateY(-4px); }
    to { opacity: 1; transform: translateY(0); }
  }

  /* ---- List container ---- */

  .list-container {
    background: var(--surface);
    border-radius: var(--radius-lg);
    overflow: hidden;
    box-shadow: var(--glass-edge-shadow);
  }

  /* ---- List header ---- */

  .list-header {
    display: grid;
    grid-template-columns: 40px 44px 1fr 56px 60px;
    padding: var(--space-2) var(--space-4);
    background: var(--bg-secondary);
    border-bottom: 1px solid var(--separator);
    font-size: 11px;
    font-weight: 500;
    color: var(--text-secondary);
    align-items: center;
  }

  /* ---- List body ---- */

  .list-body {
    max-height: calc(100vh - 300px);
    overflow-y: auto;
  }

  /* ---- List row ---- */

  .list-row {
    display: grid;
    grid-template-columns: 40px 44px 1fr 56px 60px;
    padding: var(--space-2) var(--space-4);
    align-items: center;
    transition: background var(--duration-fast) var(--ease);
  }

  .list-row:nth-child(even) {
    background: rgba(255, 255, 255, 0.025);
  }

  :global([data-theme="light"]) .list-row:nth-child(even) {
    background: rgba(0, 0, 0, 0.025);
  }

  .list-row:hover {
    background: var(--surface-hover);
  }

  .list-row.row-disabled {
    opacity: 0.5;
  }

  .list-row.row-disabled:hover {
    opacity: 0.6;
  }

  .list-row.row-has-warning {
    border-left: 2px solid var(--yellow);
  }

  /* ---- Toggle ---- */

  .toggle-btn {
    position: relative;
    width: 32px;
    height: 18px;
    background: var(--separator-opaque);
    border: none;
    border-radius: 9px;
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease);
    padding: 0;
  }

  .toggle-btn.toggle-on {
    background: var(--green);
  }

  .toggle-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .toggle-thumb {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 14px;
    height: 14px;
    background: white;
    border-radius: 50%;
    transition: transform var(--duration-fast) var(--ease);
    box-shadow: 0 1px 2px rgba(0,0,0,0.2);
  }

  .toggle-on .toggle-thumb {
    transform: translateX(14px);
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
  }

  .col-toggle {
    display: flex;
    align-items: center;
  }

  .col-plugin {
    min-width: 0;
    overflow: hidden;
    display: flex;
    align-items: center;
    gap: 6px;
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
  }

  .plugin-warnings {
    display: flex;
    gap: 3px;
    flex-shrink: 0;
  }

  .warning-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    font-size: 9px;
    font-weight: 700;
    border-radius: 50%;
    cursor: help;
  }

  .warning-badge.warning-info {
    background: rgba(59, 130, 246, 0.2);
    color: var(--accent);
  }

  .warning-badge.warning-warn {
    background: rgba(234, 179, 8, 0.2);
    color: var(--yellow);
  }

  .warning-badge.warning-error {
    background: rgba(239, 68, 68, 0.2);
    color: var(--red);
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

  .col-actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 2px;
  }

  .move-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    background: none;
    border: none;
    border-radius: var(--radius-sm);
    color: var(--text-tertiary);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
    padding: 0;
  }

  .move-btn:hover:not(:disabled) {
    background: var(--surface-active);
    color: var(--text-primary);
  }

  .move-btn:disabled {
    opacity: 0.2;
    cursor: not-allowed;
  }

  /* ---- Warning row ---- */

  .warning-row {
    padding: 0 var(--space-4) var(--space-2);
    padding-left: calc(40px + 44px + var(--space-4));
  }

  .warning-message {
    font-size: 11px;
    line-height: 1.4;
    padding: 4px 8px;
    border-radius: var(--radius-sm);
    margin-bottom: 2px;
  }

  .warning-message.warning-info {
    background: rgba(59, 130, 246, 0.08);
    color: var(--accent);
  }

  .warning-message.warning-warn {
    background: rgba(234, 179, 8, 0.08);
    color: var(--yellow);
  }

  .warning-message.warning-error {
    background: rgba(239, 68, 68, 0.08);
    color: var(--red);
  }

  /* ---- Empty state ---- */

  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-12) var(--space-8);
    background: var(--surface);
    border-radius: var(--radius-lg);
    box-shadow: var(--glass-edge-shadow);
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
    border-top-color: var(--system-accent);
    border-radius: 50%;
    animation: spin 0.75s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
