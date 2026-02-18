<script lang="ts">
  import { onMount } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import {
    getInstalledMods,
    installMod,
    uninstallMod,
    toggleMod,
  } from "$lib/api";
  import {
    selectedGame,
    installedMods,
    games,
    currentPage,
    showError,
    showSuccess,
  } from "$lib/stores";
  import type { InstalledMod, DetectedGame } from "$lib/types";

  let installing = $state(false);
  let confirmUninstall = $state<number | null>(null);
  let togglingMod = $state<number | null>(null);

  // Game picker state
  let pickedGame = $state<DetectedGame | null>(null);
  let gameList = $state<DetectedGame[]>([]);
  let hoveredGame = $state<string | null>(null);

  games.subscribe((g) => (gameList = g));

  $effect(() => {
    const game = pickedGame ?? $selectedGame;
    if (game) {
      loadMods(game);
    }
  });

  async function loadMods(game: DetectedGame) {
    try {
      const mods = await getInstalledMods(game.game_id, game.bottle_name);
      installedMods.set(mods);
    } catch (e: any) {
      showError(`Failed to load mods: ${e}`);
    }
  }

  async function handleInstall() {
    const game = pickedGame ?? $selectedGame;
    if (!game) return;

    const filePath = await open({
      multiple: false,
      filters: [
        {
          name: "Mod Archives",
          extensions: ["zip", "7z", "rar"],
        },
      ],
    });

    if (!filePath) return;

    installing = true;
    try {
      const mod = await installMod(
        filePath as string,
        game.game_id,
        game.bottle_name
      );
      showSuccess(`Installed "${(mod as InstalledMod).name}" successfully`);
      await loadMods(game);
    } catch (e: any) {
      showError(`Install failed: ${e}`);
    } finally {
      installing = false;
    }
  }

  async function handleUninstall(modId: number) {
    const game = pickedGame ?? $selectedGame;
    if (!game) return;

    try {
      const removed = await uninstallMod(modId, game.game_id, game.bottle_name);
      showSuccess(`Uninstalled — ${(removed as string[]).length} files removed`);
      confirmUninstall = null;
      await loadMods(game);
    } catch (e: any) {
      showError(`Uninstall failed: ${e}`);
    }
  }

  async function handleToggle(mod: InstalledMod) {
    togglingMod = mod.id;
    try {
      await toggleMod(mod.id, !mod.enabled);
      const game = pickedGame ?? $selectedGame;
      if (game) await loadMods(game);
    } catch (e: any) {
      showError(`Failed to toggle mod: ${e}`);
    } finally {
      togglingMod = null;
    }
  }

  function selectGameForMods(game: DetectedGame) {
    pickedGame = game;
    selectedGame.set(game);
  }

  function formatDate(iso: string): string {
    return new Date(iso).toLocaleDateString(undefined, {
      month: "short",
      day: "numeric",
      year: "numeric",
    });
  }

  const activeGame = $derived(pickedGame ?? $selectedGame);
  const modCount = $derived($installedMods.length);
  const enabledCount = $derived($installedMods.filter((m) => m.enabled).length);
</script>

<div class="mods-page">
  {#if !activeGame}
    <!-- Game Picker -->
    <div class="picker-container">
      <div class="picker-icon">
        <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <rect x="2" y="6" width="20" height="12" rx="2" />
          <circle cx="8" cy="12" r="2" />
          <circle cx="16" cy="12" r="2" />
          <line x1="12" y1="8" x2="12" y2="16" />
        </svg>
      </div>
      <h2 class="picker-title">Select a Game</h2>
      <p class="picker-subtitle">Choose a game to view and manage its installed mods.</p>

      {#if gameList.length === 0}
        <div class="picker-empty">
          <p>No games detected yet.</p>
          <p class="picker-empty-hint">Scan for bottles and games from the Dashboard first.</p>
          <button class="btn btn-secondary" onclick={() => currentPage.set("dashboard")}>
            Open Dashboard
          </button>
        </div>
      {:else}
        <div class="game-cards">
          {#each gameList as game (game.game_id + game.bottle_name)}
            <button
              class="game-card"
              class:game-card-hovered={hoveredGame === game.game_id + game.bottle_name}
              onmouseenter={() => (hoveredGame = game.game_id + game.bottle_name)}
              onmouseleave={() => (hoveredGame = null)}
              onclick={() => selectGameForMods(game)}
            >
              <div class="game-card-content">
                <span class="game-card-name">{game.display_name}</span>
                <span class="game-card-bottle">{game.bottle_name}</span>
              </div>
              <div class="game-card-chevron">
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
                  <path d="M6 3.5L10.5 8L6 12.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>
                </svg>
              </div>
            </button>
          {/each}
        </div>
      {/if}
    </div>
  {:else}
    <!-- Page Header -->
    <div class="page-header">
      <div class="header-info">
        <div class="header-title-row">
          <h2>Mods</h2>
          {#if modCount > 0}
            <span class="mod-count-badge">{enabledCount}/{modCount} active</span>
          {/if}
        </div>
        <div class="header-meta">
          <span class="meta-game">{activeGame.display_name}</span>
          <span class="meta-separator">/</span>
          <span class="meta-bottle">{activeGame.bottle_name}</span>
        </div>
      </div>
      <div class="header-actions">
        <button class="btn btn-ghost" onclick={() => { pickedGame = null; selectedGame.set(null); }}>
          Change Game
        </button>
        <button class="btn btn-primary" onclick={handleInstall} disabled={installing}>
          {#if installing}
            <span class="spinner"></span>
            Installing...
          {:else}
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
              <line x1="7" y1="2" x2="7" y2="12" />
              <line x1="2" y1="7" x2="12" y2="7" />
            </svg>
            Install Mod
          {/if}
        </button>
      </div>
    </div>

    <!-- Content Area -->
    {#if $installedMods.length === 0}
      <div class="empty-state">
        <div class="empty-icon">
          <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z" />
            <polyline points="3.27 6.96 12 12.01 20.73 6.96" />
            <line x1="12" y1="22.08" x2="12" y2="12" />
          </svg>
        </div>
        <h3 class="empty-title">No mods installed</h3>
        <p class="empty-description">
          Install mods from .zip, .7z, or .rar archives, or use NXM links via Settings.
        </p>
        <button class="btn btn-primary" onclick={handleInstall}>
          <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
            <line x1="7" y1="2" x2="7" y2="12" />
            <line x1="2" y1="7" x2="12" y2="7" />
          </svg>
          Install Your First Mod
        </button>
      </div>
    {:else}
      <div class="mod-table-container">
        <div class="mod-table">
          <!-- Sticky Header -->
          <div class="table-header">
            <span class="col-toggle"></span>
            <span class="col-name">Name</span>
            <span class="col-version">Version</span>
            <span class="col-files">Files</span>
            <span class="col-date">Installed</span>
            <span class="col-actions"></span>
          </div>

          <!-- Mod Rows -->
          <div class="table-body">
            {#each $installedMods as mod, i (mod.id)}
              <div
                class="table-row"
                class:row-disabled={!mod.enabled}
                class:row-even={i % 2 === 0}
              >
                <!-- Toggle Switch -->
                <span class="col-toggle">
                  <button
                    class="toggle-switch"
                    class:toggle-on={mod.enabled}
                    class:toggle-busy={togglingMod === mod.id}
                    onclick={() => handleToggle(mod)}
                    title={mod.enabled ? "Disable mod" : "Enable mod"}
                  >
                    <span class="toggle-track">
                      <span class="toggle-thumb"></span>
                    </span>
                  </button>
                </span>

                <!-- Name -->
                <span class="col-name">
                  <span class="mod-name">{mod.name}</span>
                  {#if mod.nexus_mod_id}
                    <span class="nexus-badge">Nexus</span>
                  {/if}
                </span>

                <!-- Version -->
                <span class="col-version">
                  {mod.version || "\u2014"}
                </span>

                <!-- File Count -->
                <span class="col-files">
                  {mod.installed_files.length}
                </span>

                <!-- Date -->
                <span class="col-date">
                  {formatDate(mod.installed_at)}
                </span>

                <!-- Actions -->
                <span class="col-actions">
                  {#if confirmUninstall === mod.id}
                    <div class="confirm-actions">
                      <button
                        class="btn btn-danger btn-sm"
                        onclick={() => handleUninstall(mod.id)}
                      >
                        Confirm
                      </button>
                      <button
                        class="btn btn-ghost btn-sm"
                        onclick={() => (confirmUninstall = null)}
                      >
                        Cancel
                      </button>
                    </div>
                  {:else}
                    <button
                      class="btn btn-ghost-danger btn-sm"
                      onclick={() => (confirmUninstall = mod.id)}
                    >
                      Uninstall
                    </button>
                  {/if}
                </span>
              </div>
            {/each}
          </div>
        </div>
      </div>
    {/if}
  {/if}
</div>

<style>
  /* ============================
     Page Layout
     ============================ */
  .mods-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    max-width: 1000px;
    padding: var(--space-6);
    gap: var(--space-6);
  }

  /* ============================
     Game Picker
     ============================ */
  .picker-container {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    flex: 1;
    padding: var(--space-12) var(--space-6);
    text-align: center;
  }

  .picker-icon {
    color: var(--text-tertiary);
    margin-bottom: var(--space-5);
  }

  .picker-title {
    font-size: 22px;
    font-weight: 700;
    letter-spacing: -0.02em;
    margin-bottom: var(--space-2);
  }

  .picker-subtitle {
    color: var(--text-secondary);
    font-size: 14px;
    line-height: 1.5;
    margin-bottom: var(--space-8);
    max-width: 340px;
  }

  .picker-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
  }

  .picker-empty p {
    color: var(--text-secondary);
    font-size: 14px;
  }

  .picker-empty-hint {
    color: var(--text-tertiary) !important;
    font-size: 13px !important;
    margin-bottom: var(--space-2);
  }

  .game-cards {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    width: 100%;
    max-width: 460px;
  }

  .game-card {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    padding: var(--space-4) var(--space-5);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    text-align: left;
    transition:
      background var(--duration) var(--ease),
      border-color var(--duration) var(--ease),
      box-shadow var(--duration) var(--ease);
  }

  .game-card:hover {
    background: var(--surface-hover);
    border-color: var(--accent-muted);
    box-shadow: var(--shadow-sm);
  }

  .game-card:active {
    background: var(--surface-active);
  }

  .game-card-content {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .game-card-name {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .game-card-bottle {
    font-size: 12px;
    color: var(--text-tertiary);
    font-family: var(--font-mono);
    letter-spacing: 0;
  }

  .game-card-chevron {
    color: var(--text-quaternary);
    transition: color var(--duration-fast) var(--ease), transform var(--duration-fast) var(--ease);
  }

  .game-card:hover .game-card-chevron {
    color: var(--accent);
    transform: translateX(2px);
  }

  /* ============================
     Page Header
     ============================ */
  .page-header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: var(--space-4);
    flex-shrink: 0;
  }

  .header-info {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .header-title-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .header-title-row h2 {
    font-size: 22px;
    font-weight: 700;
    letter-spacing: -0.02em;
  }

  .mod-count-badge {
    display: inline-flex;
    align-items: center;
    padding: 2px 10px;
    background: var(--surface);
    border-radius: 100px;
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
    letter-spacing: 0;
  }

  .header-meta {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: 13px;
  }

  .meta-game {
    color: var(--text-secondary);
    font-weight: 500;
  }

  .meta-separator {
    color: var(--text-quaternary);
  }

  .meta-bottle {
    color: var(--text-tertiary);
    font-family: var(--font-mono);
    font-size: 12px;
    letter-spacing: 0;
  }

  .header-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-shrink: 0;
  }

  /* ============================
     Buttons
     ============================ */
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    border-radius: var(--radius-sm);
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
    transition:
      background var(--duration-fast) var(--ease),
      color var(--duration-fast) var(--ease),
      box-shadow var(--duration-fast) var(--ease),
      opacity var(--duration-fast) var(--ease);
  }

  .btn-primary {
    background: var(--accent);
    color: #fff;
    padding: var(--space-2) var(--space-5);
    border-radius: var(--radius);
  }

  .btn-primary:hover:not(:disabled) {
    background: var(--accent-hover);
    box-shadow: 0 1px 6px rgba(232, 128, 42, 0.3);
  }

  .btn-primary:active:not(:disabled) {
    filter: brightness(0.92);
  }

  .btn-primary:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-secondary {
    background: var(--surface);
    color: var(--text-primary);
    border: 1px solid var(--separator);
  }

  .btn-secondary:hover {
    background: var(--surface-hover);
    border-color: var(--separator-opaque);
  }

  .btn-danger {
    background: var(--red-subtle);
    color: var(--red);
  }

  .btn-danger:hover {
    background: rgba(255, 69, 58, 0.25);
  }

  .btn-ghost {
    background: transparent;
    color: var(--text-secondary);
  }

  .btn-ghost:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .btn-ghost-danger {
    background: transparent;
    color: var(--text-tertiary);
  }

  .btn-ghost-danger:hover {
    background: var(--red-subtle);
    color: var(--red);
  }

  .btn-sm {
    padding: var(--space-1) var(--space-3);
    font-size: 12px;
    font-weight: 500;
    border-radius: var(--radius-sm);
  }

  /* ============================
     Spinner
     ============================ */
  .spinner {
    display: inline-block;
    width: 14px;
    height: 14px;
    border: 2px solid rgba(255, 255, 255, 0.3);
    border-top-color: #fff;
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  /* ============================
     Empty State
     ============================ */
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    flex: 1;
    padding: var(--space-12) var(--space-6);
    background: var(--surface);
    border: 1px dashed var(--separator-opaque);
    border-radius: var(--radius-lg);
    text-align: center;
    gap: var(--space-3);
  }

  .empty-icon {
    color: var(--text-quaternary);
    margin-bottom: var(--space-2);
  }

  .empty-title {
    font-size: 17px;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .empty-description {
    font-size: 13px;
    color: var(--text-tertiary);
    max-width: 320px;
    line-height: 1.5;
    margin-bottom: var(--space-2);
  }

  /* ============================
     Mod Table
     ============================ */
  .mod-table-container {
    flex: 1;
    overflow: hidden;
    border-radius: var(--radius-lg);
    border: 1px solid var(--separator);
    background: var(--bg-primary);
  }

  .mod-table {
    display: flex;
    flex-direction: column;
    height: 100%;
  }

  .table-header {
    display: grid;
    grid-template-columns: 52px 1fr 80px 64px 110px 120px;
    padding: var(--space-3) var(--space-4);
    background: var(--bg-secondary);
    border-bottom: 1px solid var(--separator);
    position: sticky;
    top: 0;
    z-index: 2;
  }

  .table-header span {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-tertiary);
  }

  .table-body {
    flex: 1;
    overflow-y: auto;
  }

  .table-row {
    display: grid;
    grid-template-columns: 52px 1fr 80px 64px 110px 120px;
    padding: var(--space-3) var(--space-4);
    align-items: center;
    font-size: 13px;
    border-bottom: 1px solid var(--separator);
    transition: background var(--duration-fast) var(--ease);
  }

  .table-row:last-child {
    border-bottom: none;
  }

  .table-row.row-even {
    background: rgba(255, 255, 255, 0.015);
  }

  .table-row:hover {
    background: var(--surface-hover);
  }

  .table-row.row-disabled {
    opacity: 0.45;
  }

  .table-row.row-disabled:hover {
    opacity: 0.6;
  }

  /* ============================
     Toggle Switch (Pill)
     ============================ */
  .toggle-switch {
    display: inline-flex;
    align-items: center;
    padding: 0;
    background: transparent;
    cursor: pointer;
  }

  .toggle-track {
    position: relative;
    width: 32px;
    height: 18px;
    border-radius: 9px;
    background: var(--bg-tertiary);
    transition:
      background var(--duration) var(--ease),
      box-shadow var(--duration) var(--ease);
  }

  .toggle-on .toggle-track {
    background: var(--green);
    box-shadow: 0 0 8px rgba(48, 209, 88, 0.25);
  }

  .toggle-thumb {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
    transition: transform var(--duration) var(--ease);
  }

  .toggle-on .toggle-thumb {
    transform: translateX(14px);
  }

  .toggle-busy .toggle-track {
    opacity: 0.6;
  }

  /* ============================
     Mod Name & Badge
     ============================ */
  .col-name {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .mod-name {
    font-weight: 500;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .nexus-badge {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    padding: 1px 6px;
    border-radius: 4px;
    background: var(--accent-subtle);
    color: var(--accent);
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }

  /* ============================
     Table Columns
     ============================ */
  .col-toggle {
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .col-version,
  .col-files,
  .col-date {
    color: var(--text-secondary);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }

  .col-files {
    font-family: var(--font-mono);
    font-size: 12px;
    letter-spacing: 0;
  }

  .col-actions {
    display: flex;
    justify-content: flex-end;
    align-items: center;
  }

  .confirm-actions {
    display: flex;
    gap: var(--space-1);
    align-items: center;
  }
</style>
