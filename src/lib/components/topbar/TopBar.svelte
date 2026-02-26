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
    height: 44px;
    flex-shrink: 0;
    -webkit-app-region: drag;
    position: sticky;
    top: 0;
    z-index: 10;
    /* Extend glass edge-to-edge within .content padding */
    margin: 0 calc(-1 * var(--space-6));
    padding: 0 calc(var(--space-6) + 4px);
    /* Liquid glass toolbar */
    background: color-mix(in srgb, var(--bg-base) 60%, transparent);
    backdrop-filter: blur(28px) saturate(1.4);
    -webkit-backdrop-filter: blur(28px) saturate(1.4);
    border-bottom: 0.5px solid rgba(255, 255, 255, 0.06);
  }

  /* Fade gradient below toolbar — content dissolves into glass */
  .top-bar::after {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    top: 100%;
    height: 16px;
    background: linear-gradient(
      to bottom,
      color-mix(in srgb, var(--bg-base) 40%, transparent),
      transparent
    );
    pointer-events: none;
    z-index: 9;
  }

  :global([data-theme="light"]) .top-bar {
    background: color-mix(in srgb, var(--bg-base) 70%, transparent);
    border-bottom-color: rgba(0, 0, 0, 0.08);
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
    padding: 3px 6px;
    border-radius: 100px;
    background: var(--surface-glass);
    backdrop-filter: var(--glass-blur-light);
    border: 1px solid rgba(255, 255, 255, 0.08);
    box-shadow: var(--glass-refraction),
                inset 0 1px 0 0 rgba(255, 255, 255, 0.1),
                inset 0 -1px 0 0 rgba(255, 255, 255, 0.04),
                0 1px 3px rgba(0, 0, 0, 0.12);
    transition: box-shadow var(--duration-fast) var(--ease),
                background var(--duration-fast) var(--ease);
    -webkit-app-region: no-drag;
  }

  .topbar-pill:hover {
    background: var(--surface-glass-hover);
    box-shadow: var(--glass-refraction),
                inset 0 1px 0 0 rgba(255, 255, 255, 0.14),
                inset 0 -1px 0 0 rgba(255, 255, 255, 0.05),
                0 2px 8px rgba(0, 0, 0, 0.18);
  }

  .topbar-separator {
    color: var(--text-quaternary);
    font-size: 14px;
    line-height: 1;
    user-select: none;
    padding: 0 1px;
    opacity: 0.5;
  }
</style>
