<script lang="ts">
  import { selectedGame } from "$lib/stores";
  import GameIcon from "$lib/components/GameIcon.svelte";
  import type { DetectedGame } from "$lib/types";

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
</script>

<div class="topbar-selector-wrap">
  <button
    class="topbar-selector"
    onclick={(e) => { e.stopPropagation(); onToggle(); }}
    title={$selectedGame?.display_name ?? "Select a game"}
  >
    {#if $selectedGame}
      <GameIcon gameId={$selectedGame.game_id} size={18} />
      <span class="topbar-selector-label">{$selectedGame.display_name}</span>
    {:else}
      <svg width="18" height="18" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" opacity="0.4">
        <rect x="2" y="4" width="12" height="8" rx="2" />
        <circle cx="6" cy="8" r="1.5" />
        <circle cx="10" cy="8" r="1.5" />
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
        {#each detectedGames as game}
          <button
            class="dropdown-item"
            class:active={$selectedGame?.game_id === game.game_id && $selectedGame?.bottle_name === game.bottle_name}
            onclick={() => selectGame(game)}
          >
            <GameIcon gameId={game.game_id} size={16} />
            <div class="dropdown-item-text">
              <span class="dropdown-item-name">{game.display_name}</span>
              <span class="dropdown-item-sub">{game.bottle_name}</span>
            </div>
          </button>
        {/each}
      {:else}
        <div class="dropdown-empty">No games detected</div>
      {/if}
      <div class="dropdown-footer">
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
    padding: 5px 10px;
    border-radius: var(--radius);
    font-size: 13px;
    color: var(--text-primary);
    background: none;
    border: none;
    cursor: pointer;
    -webkit-app-region: no-drag;
    transition: background 0.15s ease;
    max-width: 220px;
  }

  .topbar-selector:hover {
    background: var(--surface-hover);
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
    width: 26px;
    height: 26px;
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
    background: var(--bg-elevated);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    padding: 4px;
    z-index: 100;
    box-shadow:
      0 4px 24px rgba(0, 0, 0, 0.3),
      0 1px 4px rgba(0, 0, 0, 0.15),
      inset 0 1px 0 0 var(--surface);
    animation: dropdownIn 0.15s ease-out;
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

  .dropdown-footer {
    border-top: 1px solid var(--separator);
    margin-top: 4px;
    padding-top: 4px;
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

  @keyframes dropdownIn {
    from { opacity: 0; transform: translateY(-4px); }
    to { opacity: 1; transform: translateY(0); }
  }
</style>
