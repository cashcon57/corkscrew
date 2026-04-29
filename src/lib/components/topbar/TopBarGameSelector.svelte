<script lang="ts">
  import { selectedGame, showUninstalledGames, uninstalledGames } from "$lib/stores";
  import GameIcon from "$lib/components/GameIcon.svelte";
  import GameSupportBadge from "$lib/components/GameSupportBadge.svelte";
  import type { DetectedGame, KnownUninstalledGame } from "$lib/types";
  import { setConfigValue, listKnownUninstalledGames } from "$lib/api";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { wineCtx } from "$lib/types";

  interface Props {
    detectedGames: DetectedGame[];
    onPickGame: (game: DetectedGame) => void;
    onLaunchGame: () => void;
    onNavigate: (page: string) => void;
    launching: boolean;
    isOpen: boolean;
    onToggle: () => void;
    onClose: () => void;
  }

  let {
    detectedGames,
    onPickGame,
    onLaunchGame,
    onNavigate,
    launching,
    isOpen,
    onToggle,
    onClose,
  }: Props = $props();

  function selectGame(game: DetectedGame) {
    onPickGame(game);
    onClose();
  }

  function browseUninstalled(g: KnownUninstalledGame) {
    openUrl(`https://www.nexusmods.com/${g.nexus_slug}/mods`).catch((err) =>
      console.error('browseUninstalled: openUrl failed:', err)
    );
    onClose();
  }

  async function toggleShowUninstalled() {
    const next = !$showUninstalledGames;
    showUninstalledGames.set(next);
    try {
      await setConfigValue("show_uninstalled_games", next ? "true" : "false");
    } catch (err) {
      console.error('toggleShowUninstalled: persist failed:', err);
    }
    if (next && $uninstalledGames.length === 0) {
      try {
        const games = await listKnownUninstalledGames();
        uninstalledGames.set(games);
      } catch (err) {
        console.error('toggleShowUninstalled: fetch failed:', err);
      }
    }
  }
</script>

<div class="topbar-selector-wrap">
  <button
    class="topbar-selector"
    onclick={(e) => { e.stopPropagation(); onToggle(); }}
    title={$selectedGame?.display_name ?? "Select a game"}
  >
    {#if $selectedGame}
      <GameIcon gameId={$selectedGame.game_id} steamAppId={$selectedGame.steam_app_id} size={20} />
      <span class="topbar-selector-label">{$selectedGame.display_name}</span>
      <GameSupportBadge gameId={$selectedGame.game_id} compact hideWhenVerified />
    {:else}
      <svg width="20" height="20" viewBox="0 0 24 28" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" opacity="0.4">
        <rect x="1.5" y="1.5" width="21" height="20" rx="2" />
        <rect x="3.5" y="3" width="17" height="13" rx="1" />
        <line x1="1.5" y1="21.5" x2="22.5" y2="21.5" />
        <line x1="5" y1="21.5" x2="5" y2="26.5" />
        <line x1="8" y1="21.5" x2="8" y2="26.5" />
        <line x1="11" y1="21.5" x2="11" y2="26.5" />
        <line x1="14" y1="21.5" x2="14" y2="26.5" />
        <line x1="17" y1="21.5" x2="17" y2="26.5" />
        <line x1="20" y1="21.5" x2="20" y2="26.5" />
      </svg>
      <span class="topbar-selector-label placeholder">Select Game</span>
    {/if}
    <svg class="topbar-chevron" class:open={isOpen} width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
      <path d="M3 4l2 2 2-2" />
    </svg>
  </button>

  {#if $selectedGame}
    <button
      class="topbar-launch-btn"
      onclick={(e) => { e.stopPropagation(); onLaunchGame(); }}
      disabled={launching}
      title="Launch {$selectedGame.display_name}"
    >
      {#if launching}
        <span class="spinner spinner-sm"></span>
      {:else}
        <svg width="12" height="12" viewBox="0 0 16 16" fill="currentColor">
          <path d="M4 2.5v11l9-5.5z" />
        </svg>
      {/if}
    </button>
  {/if}

  {#if isOpen}
    <div class="topbar-dropdown" onclick={(e) => e.stopPropagation()}>
      {#if detectedGames.length > 0}
        <div class="dropdown-section-label">Installed</div>
        {#each detectedGames as game}
          <button
            class="dropdown-item"
            class:active={$selectedGame?.game_id === game.game_id && ($selectedGame ? wineCtx($selectedGame)?.bottle_name : null) === wineCtx(game)?.bottle_name}
            onclick={() => selectGame(game)}
          >
            <GameIcon gameId={game.game_id} steamAppId={game.steam_app_id} size={16} />
            <div class="dropdown-item-text">
              <span class="dropdown-item-name">{game.display_name}</span>
              <span class="dropdown-item-sub">{(wineCtx(game)?.bottle_name ?? "")}</span>
            </div>
            <GameSupportBadge gameId={game.game_id} compact hideWhenVerified />
          </button>
        {/each}
      {:else}
        <div class="dropdown-empty">No games detected</div>
      {/if}

      {#if $showUninstalledGames && $uninstalledGames.length > 0}
        <div class="dropdown-section-label">Not Installed</div>
        {#each $uninstalledGames as game}
          <button
            class="dropdown-item dropdown-item-uninstalled"
            onclick={() => browseUninstalled(game)}
            title="Browse {game.name} mods on Nexus"
          >
            <GameIcon gameId={game.game_id} size={16} />
            <div class="dropdown-item-text">
              <span class="dropdown-item-name">{game.name}</span>
              <span class="dropdown-item-sub">Browse on Nexus</span>
            </div>
            <svg width="11" height="11" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" opacity="0.6">
              <path d="M3 13l10-10" /><path d="M6 3h7v7" />
            </svg>
          </button>
        {/each}
      {/if}

      <div class="dropdown-footer">
        <label class="dropdown-toggle">
          <input
            type="checkbox"
            checked={$showUninstalledGames}
            onchange={toggleShowUninstalled}
          />
          <span>Show uninstalled games</span>
        </label>
        <button class="dropdown-action" onclick={() => { onNavigate("dashboard"); onClose(); }}>
          <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <line x1="8" y1="3" x2="8" y2="13" /><line x1="3" y1="8" x2="13" y2="8" />
          </svg>
          Add Game
        </button>
      </div>
    </div>
  {/if}
</div>

<style>
  .topbar-selector-wrap {
    position: relative;
    display: flex;
    align-items: center;
    gap: 2px;
    -webkit-app-region: no-drag;
  }

  .topbar-selector {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    border-radius: 100px;
    font-size: 13.5px;
    color: var(--text-primary);
    background: none;
    border: none;
    cursor: pointer;
    -webkit-app-region: no-drag;
    transition: background 0.15s ease;
    max-width: 240px;
  }

  .topbar-selector:hover {
    background: var(--surface);
  }

  .topbar-selector-label {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-weight: 500;
  }

  .topbar-selector-label.placeholder {
    color: var(--text-tertiary);
    font-weight: 400;
  }

  .topbar-chevron {
    flex-shrink: 0;
    opacity: 0.4;
    transition: transform 0.15s ease;
  }

  .topbar-chevron.open {
    transform: rotate(180deg);
  }

  .topbar-launch-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 30px;
    height: 30px;
    border-radius: var(--radius);
    background: none;
    border: none;
    color: var(--text-secondary);
    cursor: pointer;
    -webkit-app-region: no-drag;
    transition: background 0.15s ease, color 0.15s ease;
  }

  .topbar-launch-btn:hover {
    background: var(--accent-subtle);
    color: var(--accent);
  }

  .topbar-launch-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .topbar-dropdown {
    position: absolute;
    top: calc(100% + 6px);
    left: 0;
    min-width: 240px;
    max-width: 320px;
    background: color-mix(in srgb, var(--bg-elevated) 92%, transparent);
    backdrop-filter: var(--glass-blur-heavy);
    -webkit-backdrop-filter: var(--glass-blur-heavy);
    border: 1px solid var(--separator);
    border-radius: var(--radius-lg);
    padding: 4px;
    z-index: 100;
    box-shadow:
      var(--glass-refraction),
      var(--glass-edge-shadow),
      0 8px 32px rgba(0, 0, 0, 0.3),
      0 1px 4px rgba(0, 0, 0, 0.15);
    animation: glass-dropdown-in 0.2s var(--ease-spring);
    max-height: 320px;
    overflow-y: auto;
  }

  .dropdown-item {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 7px 10px;
    border-radius: calc(var(--radius) - 2px);
    background: none;
    border: none;
    color: var(--text-primary);
    cursor: pointer;
    font-size: 13px;
    text-align: left;
    transition: background 0.1s ease;
  }

  .dropdown-item:hover {
    background: var(--surface-hover);
  }

  .dropdown-item.active {
    background: var(--accent-subtle);
    color: var(--accent);
  }

  .dropdown-item-text {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
  }

  .dropdown-item-name {
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .dropdown-item-sub {
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .dropdown-empty {
    padding: 12px 10px;
    font-size: 12px;
    color: var(--text-tertiary);
    text-align: center;
  }

  .dropdown-section-label {
    padding: 6px 10px 4px;
    font-size: 10.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-tertiary);
  }

  .dropdown-item-uninstalled {
    opacity: 0.75;
  }

  .dropdown-item-uninstalled:hover {
    opacity: 1;
  }

  .dropdown-footer {
    border-top: 1px solid var(--separator);
    margin-top: 4px;
    padding-top: 4px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .dropdown-toggle {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 10px;
    border-radius: calc(var(--radius) - 2px);
    color: var(--text-secondary);
    cursor: pointer;
    font-size: 12px;
    transition: background 0.1s ease, color 0.1s ease;
  }

  .dropdown-toggle:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .dropdown-toggle input[type="checkbox"] {
    accent-color: var(--accent);
    cursor: pointer;
  }

  .dropdown-action {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 7px 10px;
    border-radius: calc(var(--radius) - 2px);
    background: none;
    border: none;
    color: var(--text-secondary);
    cursor: pointer;
    font-size: 12px;
    text-align: left;
    transition: background 0.1s ease, color 0.1s ease;
  }

  .dropdown-action:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  :global(html:not(.vibrancy-active)) .topbar-dropdown {
    background: var(--bg-elevated);
    backdrop-filter: none;
    -webkit-backdrop-filter: none;
    border-color: var(--separator);
  }

</style>
