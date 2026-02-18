<script lang="ts">
  import { onMount } from "svelte";
  import { getBottles, getAllGames } from "$lib/api";
  import {
    bottles,
    games,
    selectedGame,
    currentPage,
    isLoading,
    showError,
  } from "$lib/stores";
  import type { Bottle, DetectedGame } from "$lib/types";

  let loadingState = $state<"idle" | "loading" | "done">("idle");

  onMount(async () => {
    loadingState = "loading";
    try {
      const [b, g] = await Promise.all([getBottles(), getAllGames()]);
      bottles.set(b);
      games.set(g);
      loadingState = "done";
    } catch (e: any) {
      loadingState = "done";
      showError(`Failed to scan: ${e}`);
    }
  });

  function selectGame(game: DetectedGame) {
    selectedGame.set(game);
    currentPage.set("mods");
  }

  const sourceColors: Record<string, { color: string; bg: string }> = {
    CrossOver: { color: "#ff453a", bg: "rgba(255, 69, 58, 0.12)" },
    Whisky: { color: "#ffd60a", bg: "rgba(255, 214, 10, 0.12)" },
    Moonshine: { color: "#bf5af2", bg: "rgba(191, 90, 242, 0.12)" },
    Heroic: { color: "#0a84ff", bg: "rgba(10, 132, 255, 0.12)" },
    Mythic: { color: "#30d158", bg: "rgba(48, 209, 88, 0.12)" },
    Wine: { color: "#ff6961", bg: "rgba(255, 105, 97, 0.12)" },
    Lutris: { color: "#ff9f0a", bg: "rgba(255, 159, 10, 0.12)" },
    Proton: { color: "#30d158", bg: "rgba(48, 209, 88, 0.12)" },
    Bottles: { color: "#bf5af2", bg: "rgba(191, 90, 242, 0.12)" },
  };

  function getSourceStyle(source: string): { color: string; bg: string } {
    return sourceColors[source] || { color: "#8e8e93", bg: "rgba(142, 142, 147, 0.12)" };
  }

  function truncatePath(path: string, maxLen: number = 60): string {
    if (path.length <= maxLen) return path;
    const parts = path.split("/");
    if (parts.length <= 3) return path;
    return parts[0] + "/" + parts[1] + "/.../" + parts.slice(-2).join("/");
  }
</script>

{#if $currentPage === "dashboard"}
  <div class="dashboard">
    <!-- Page Header -->
    <header class="page-header">
      <div class="header-text">
        <h2 class="page-title">Dashboard</h2>
        <p class="page-subtitle">Wine bottles and detected games</p>
      </div>
      <div class="header-stats">
        {#if loadingState === "done"}
          <div class="stat-pill">
            <span class="stat-value">{$bottles.length}</span>
            <span class="stat-label">{$bottles.length === 1 ? "Bottle" : "Bottles"}</span>
          </div>
          <div class="stat-pill">
            <span class="stat-value">{$games.length}</span>
            <span class="stat-label">{$games.length === 1 ? "Game" : "Games"}</span>
          </div>
        {/if}
      </div>
    </header>

    {#if loadingState === "loading"}
      <!-- Loading State -->
      <div class="loading-container">
        <div class="loading-card">
          <div class="spinner">
            <div class="spinner-ring"></div>
          </div>
          <div class="loading-text">
            <p class="loading-title">Scanning environment</p>
            <p class="loading-detail">Looking for Wine bottles and installed games...</p>
          </div>
        </div>
      </div>
    {:else}
      <!-- Bottles Section -->
      <section class="section">
        <div class="section-header">
          <h3 class="section-title">Bottles</h3>
          <span class="section-count">{$bottles.length}</span>
        </div>

        {#if $bottles.length === 0}
          <div class="empty-state">
            <div class="empty-icon">
              <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                <path d="M8 2h8l1.5 6H6.5L8 2z" />
                <path d="M6.5 8v12a2 2 0 0 0 2 2h7a2 2 0 0 0 2-2V8" />
                <path d="M10 12v4" />
                <path d="M14 12v4" />
              </svg>
            </div>
            <p class="empty-title">No bottles found</p>
            <p class="empty-detail">
              Corkscrew looks for bottles from CrossOver, Whisky, Moonshine, Heroic, Mythic, Lutris, Proton, and native Wine.
            </p>
          </div>
        {:else}
          <div class="card-grid">
            {#each $bottles as bottle, i}
              <div
                class="card bottle-card"
                style="animation-delay: {i * 40}ms"
              >
                <div class="card-top-row">
                  <span
                    class="source-badge"
                    style="color: {getSourceStyle(bottle.source).color}; background: {getSourceStyle(bottle.source).bg};"
                  >
                    {bottle.source}
                  </span>
                </div>
                <h4 class="card-name">{bottle.name}</h4>
                <p class="card-path" title={bottle.path}>
                  {truncatePath(bottle.path)}
                </p>
              </div>
            {/each}
          </div>
        {/if}
      </section>

      <!-- Games Section -->
      <section class="section">
        <div class="section-header">
          <h3 class="section-title">Games</h3>
          <span class="section-count">{$games.length}</span>
        </div>

        {#if $games.length === 0}
          <div class="empty-state">
            <div class="empty-icon">
              <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                <rect x="2" y="6" width="20" height="12" rx="3" />
                <circle cx="8.5" cy="12" r="1.5" />
                <circle cx="15.5" cy="12" r="1.5" />
                <path d="M6 10v4" />
                <path d="M4.5 12h3" />
              </svg>
            </div>
            <p class="empty-title">No games detected</p>
            <p class="empty-detail">
              Install a supported game in one of your Wine bottles to get started with mod management.
            </p>
          </div>
        {:else}
          <div class="card-grid card-grid-games">
            {#each $games as game, i}
              <button
                class="card game-card"
                style="animation-delay: {($bottles.length + i) * 40}ms"
                onclick={() => selectGame(game)}
              >
                <div class="card-top-row">
                  <span class="game-tag">{game.game_id}</span>
                  <svg class="card-chevron" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <polyline points="9 18 15 12 9 6" />
                  </svg>
                </div>
                <h4 class="card-name">{game.display_name}</h4>
                <div class="game-meta">
                  <span class="meta-item">
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M8 2h8l1.5 6H6.5L8 2z" />
                      <path d="M6.5 8v12a2 2 0 0 0 2 2h7a2 2 0 0 0 2-2V8" />
                    </svg>
                    {game.bottle_name}
                  </span>
                </div>
                <p class="card-path" title={game.game_path}>
                  {truncatePath(game.game_path)}
                </p>
                <span class="card-action-label">Manage Mods</span>
              </button>
            {/each}
          </div>
        {/if}
      </section>
    {/if}
  </div>
{:else if $currentPage === "mods"}
  {#await import("./mods/+page.svelte") then mod}
    <mod.default />
  {/await}
{:else if $currentPage === "plugins"}
  {#await import("./plugins/+page.svelte") then mod}
    <mod.default />
  {/await}
{:else if $currentPage === "settings"}
  {#await import("./settings/+page.svelte") then mod}
    <mod.default />
  {/await}
{:else if $currentPage === "about"}
  {#await import("./about/+page.svelte") then mod}
    <mod.default />
  {/await}
{/if}

<style>
  /* ============================================
     Dashboard Layout
     ============================================ */

  .dashboard {
    max-width: 1040px;
    padding: var(--space-2) 0 var(--space-12) 0;
  }

  /* ============================================
     Page Header
     ============================================ */

  .page-header {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    margin-bottom: var(--space-10);
    padding-bottom: var(--space-6);
    border-bottom: 1px solid var(--separator);
  }

  .page-title {
    font-size: 28px;
    font-weight: 700;
    color: var(--text-primary);
    letter-spacing: -0.025em;
    line-height: 1.15;
  }

  .page-subtitle {
    font-size: 14px;
    color: var(--text-secondary);
    margin-top: var(--space-1);
    font-weight: 400;
  }

  .header-stats {
    display: flex;
    gap: var(--space-3);
  }

  .stat-pill {
    display: flex;
    align-items: baseline;
    gap: var(--space-1);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    padding: var(--space-2) var(--space-4);
  }

  .stat-value {
    font-size: 18px;
    font-weight: 700;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .stat-label {
    font-size: 12px;
    color: var(--text-tertiary);
    font-weight: 500;
  }

  /* ============================================
     Loading State
     ============================================ */

  .loading-container {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 280px;
  }

  .loading-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-6);
    padding: var(--space-12) var(--space-10);
  }

  .spinner {
    width: 36px;
    height: 36px;
    position: relative;
  }

  .spinner-ring {
    width: 100%;
    height: 100%;
    border: 2.5px solid var(--separator);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.9s cubic-bezier(0.4, 0, 0.2, 1) infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .loading-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
    text-align: center;
  }

  .loading-detail {
    font-size: 13px;
    color: var(--text-tertiary);
    text-align: center;
    margin-top: var(--space-1);
  }

  /* ============================================
     Section Layout
     ============================================ */

  .section {
    margin-bottom: var(--space-10);
  }

  .section-header {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-bottom: var(--space-5);
  }

  .section-title {
    font-size: 17px;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .section-count {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-tertiary);
    background: var(--surface);
    border: 1px solid var(--separator);
    padding: 1px var(--space-2);
    border-radius: var(--radius-sm);
    font-variant-numeric: tabular-nums;
    min-width: 22px;
    text-align: center;
  }

  /* ============================================
     Card Grid
     ============================================ */

  .card-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: var(--space-3);
  }

  .card-grid-games {
    grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
  }

  /* ============================================
     Card Base
     ============================================ */

  .card {
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-lg);
    padding: var(--space-5);
    text-align: left;
    transition:
      background var(--duration) var(--ease),
      border-color var(--duration) var(--ease),
      box-shadow var(--duration) var(--ease),
      transform var(--duration-fast) var(--ease);
    animation: cardFadeIn var(--duration-slow) var(--ease) both;
    position: relative;
  }

  @keyframes cardFadeIn {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  /* ============================================
     Bottle Cards
     ============================================ */

  .bottle-card:hover {
    background: var(--surface-hover);
    border-color: rgba(255, 255, 255, 0.12);
  }

  /* ============================================
     Game Cards (interactive)
     ============================================ */

  .game-card {
    width: 100%;
    cursor: pointer;
  }

  .game-card:hover {
    background: var(--surface-hover);
    border-color: var(--accent);
    box-shadow: 0 0 0 1px rgba(232, 128, 42, 0.08);
  }

  .game-card:hover .card-action-label {
    opacity: 1;
    color: var(--accent);
  }

  .game-card:hover .card-chevron {
    opacity: 1;
    transform: translateX(2px);
    color: var(--accent);
  }

  .game-card:active {
    transform: scale(0.985);
    background: var(--surface-active);
  }

  /* ============================================
     Card Inner Elements
     ============================================ */

  .card-top-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--space-3);
  }

  .source-badge {
    display: inline-flex;
    align-items: center;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.02em;
    padding: 2px var(--space-2);
    border-radius: var(--radius-sm);
    line-height: 1.5;
  }

  .game-tag {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    color: var(--accent);
    background: var(--accent-subtle);
    padding: 2px var(--space-2);
    border-radius: var(--radius-sm);
    line-height: 1.5;
  }

  .card-chevron {
    opacity: 0;
    color: var(--text-quaternary);
    transition:
      opacity var(--duration) var(--ease),
      transform var(--duration) var(--ease),
      color var(--duration) var(--ease);
    flex-shrink: 0;
  }

  .card-name {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
    margin-bottom: var(--space-2);
    line-height: 1.35;
    letter-spacing: -0.01em;
  }

  .game-meta {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-bottom: var(--space-2);
  }

  .meta-item {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    font-size: 12px;
    color: var(--text-secondary);
    font-weight: 450;
  }

  .meta-item svg {
    color: var(--text-tertiary);
    flex-shrink: 0;
  }

  .card-path {
    font-size: 11px;
    color: var(--text-tertiary);
    font-family: var(--font-mono);
    line-height: 1.45;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    letter-spacing: 0;
  }

  .card-action-label {
    display: inline-block;
    margin-top: var(--space-3);
    font-size: 12px;
    font-weight: 600;
    color: var(--text-quaternary);
    opacity: 0.6;
    transition:
      opacity var(--duration) var(--ease),
      color var(--duration) var(--ease);
    letter-spacing: 0.01em;
  }

  /* ============================================
     Empty State
     ============================================ */

  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: var(--space-12) var(--space-8);
    border: 1px dashed rgba(255, 255, 255, 0.1);
    border-radius: var(--radius-lg);
    text-align: center;
    background: rgba(255, 255, 255, 0.015);
  }

  .empty-icon {
    color: var(--text-quaternary);
    margin-bottom: var(--space-4);
    opacity: 0.7;
  }

  .empty-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-secondary);
    margin-bottom: var(--space-2);
  }

  .empty-detail {
    font-size: 13px;
    color: var(--text-tertiary);
    max-width: 360px;
    line-height: 1.55;
  }
</style>
