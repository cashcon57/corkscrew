<script lang="ts">
  import TopBarGameSelector from "./TopBarGameSelector.svelte";
  import TopBarModlistSelector from "./TopBarModlistSelector.svelte";
  import TopBarProfileSelector from "./TopBarProfileSelector.svelte";
  import type { DetectedGame } from "$lib/types";

  interface Props {
    detectedGames: DetectedGame[];
    onPickGame: (game: DetectedGame) => void;
    onLaunchGame: () => void;
    onNavigate: (page: string) => void;
    launching: boolean;
  }

  let {
    detectedGames,
    onPickGame,
    onLaunchGame,
    onNavigate,
    launching,
  }: Props = $props();

  let openDropdown = $state<"game" | "modlist" | "profile" | null>(null);

  function handleOpenDropdown(which: "game" | "modlist" | "profile") {
    openDropdown = openDropdown === which ? null : which;
  }

  function closeAll() {
    openDropdown = null;
  }
</script>

<svelte:window
  onclick={(e) => {
    const target = e.target as HTMLElement;
    if (!target.closest(".topbar-selector-wrap")) {
      closeAll();
    }
  }}
  onkeydown={(e) => {
    if (e.key === "Escape") closeAll();
  }}
/>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="top-bar" data-tauri-drag-region>
  <div class="topbar-pill">
    <TopBarGameSelector
      {detectedGames}
      {onPickGame}
      {onLaunchGame}
      {launching}
      {onNavigate}
      isOpen={openDropdown === "game"}
      onToggle={() => handleOpenDropdown("game")}
      onClose={closeAll}
    />

    <span class="topbar-separator">&rsaquo;</span>

    <TopBarModlistSelector
      {onNavigate}
      isOpen={openDropdown === "modlist"}
      onToggle={() => handleOpenDropdown("modlist")}
      onClose={closeAll}
    />

    <span class="topbar-separator">&rsaquo;</span>

    <TopBarProfileSelector
      isOpen={openDropdown === "profile"}
      onToggle={() => handleOpenDropdown("profile")}
      onClose={closeAll}
    />
  </div>
</div>

<style>
  .top-bar {
    display: flex;
    align-items: center;
    gap: 2px;
    height: 52px;
    flex-shrink: 0;
    -webkit-app-region: drag;
    position: sticky;
    top: 0;
    z-index: 10;
    /* Extend edge-to-edge within .content padding */
    margin: 0 calc(-1 * var(--space-6));
    padding: 0 calc(var(--space-6) + 4px);
    /* Transparent — glass lives on the pill only */
    background: transparent;
  }

  @media (max-width: 800px) {
    .top-bar {
      margin: 0 calc(-1 * var(--space-3));
      padding: 0 calc(var(--space-3) + 4px);
    }
  }

  .topbar-pill {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 2px 0;
    -webkit-app-region: no-drag;
  }

  .topbar-separator {
    color: var(--text-quaternary);
    font-size: 16px;
    line-height: 1;
    user-select: none;
    padding: 0 2px;
    opacity: 0.5;
  }
</style>
