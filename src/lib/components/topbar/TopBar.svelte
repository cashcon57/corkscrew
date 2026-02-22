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

  <span class="topbar-separator" data-tauri-drag-region>&rsaquo;</span>

  <TopBarModlistSelector
    {onNavigate}
    isOpen={openDropdown === "modlist"}
    onToggle={() => handleOpenDropdown("modlist")}
    onClose={closeAll}
  />

  <span class="topbar-separator" data-tauri-drag-region>&rsaquo;</span>

  <TopBarProfileSelector
    isOpen={openDropdown === "profile"}
    onToggle={() => handleOpenDropdown("profile")}
    onClose={closeAll}
  />
</div>

<style>
  .top-bar {
    display: flex;
    align-items: center;
    gap: 2px;
    height: 44px;
    padding: 0 16px;
    flex-shrink: 0;
    border-bottom: 1px solid var(--separator);
    background: transparent;
    -webkit-app-region: drag;
    position: relative;
    z-index: 10;
  }

  .topbar-separator {
    color: var(--text-quaternary);
    font-size: 18px;
    line-height: 1;
    user-select: none;
    padding: 0 2px;
    -webkit-app-region: drag;
  }
</style>
