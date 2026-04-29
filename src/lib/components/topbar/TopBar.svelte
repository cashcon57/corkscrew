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
    scrolled?: boolean;
  }

  let {
    detectedGames,
    onPickGame,
    onLaunchGame,
    onNavigate,
    launching,
    scrolled = false,
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
<div class="top-bar" class:scrolled data-tauri-drag-region>
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
    height: 64px;
    flex-shrink: 0;
    -webkit-app-region: drag;
    position: sticky;
    top: 0;
    z-index: 10;
    /* Extend edge-to-edge within .content padding */
    margin: 0 calc(-1 * var(--space-4));
    padding: 0 calc(var(--space-4) + 4px);
    background: transparent;
    border-bottom: 1px solid transparent;
    transition: border-color var(--duration-fast) var(--ease);
  }

  .top-bar.scrolled {
    border-bottom-color: var(--separator);
  }

  @media (max-width: 800px) {
    .top-bar {
      margin: 0 calc(-1 * var(--space-3));
      padding: 0 var(--space-3);
    }
  }

  /* --- Liquid Glass pill --- */

  .topbar-pill {
    position: relative;
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 4px 6px;
    -webkit-app-region: no-drag;
    border-radius: 100px;
    /* Glass material: minimal tint so content shows through with distortion */
    background: var(--surface-glass);
    /* Light blur + brightness/contrast shift = refractive look, not smeared */
    backdrop-filter: blur(4px) saturate(1.8) brightness(1.1) contrast(1.05);
    -webkit-backdrop-filter: blur(4px) saturate(1.8) brightness(1.1) contrast(1.05);
    /* Crisp edge — the border IS the refraction boundary */
    border: 0.5px solid rgba(255, 255, 255, 0.22);
    box-shadow:
      /* Top specular — strongest catch light */
      inset 0 1px 0 0 rgba(255, 255, 255, 0.25),
      /* Bottom counter-highlight */
      inset 0 -1px 0 0 rgba(255, 255, 255, 0.08),
      /* Outer refraction ring */
      0 0 0 0.5px rgba(255, 255, 255, 0.08),
      /* Subtle drop shadow for depth */
      0 1px 4px rgba(0, 0, 0, 0.10);
    transition: box-shadow var(--duration-fast) var(--ease),
                background var(--duration-fast) var(--ease);
  }

  :global([data-theme="light"]) .topbar-pill {
    background: rgba(255, 255, 255, 0.25);
    border-color: rgba(255, 255, 255, 0.50);
    backdrop-filter: blur(6px) saturate(1.4) brightness(1.05);
    -webkit-backdrop-filter: blur(6px) saturate(1.4) brightness(1.05);
    box-shadow:
      inset 0 1px 0 0 rgba(255, 255, 255, 0.50),
      inset 0 -1px 0 0 rgba(255, 255, 255, 0.20),
      0 0 0 0.5px rgba(0, 0, 0, 0.04),
      0 1px 4px rgba(0, 0, 0, 0.05);
  }

  /* Linux / non-macOS: opaque pill without backdrop-filter */
  :global(html:not(.vibrancy-active)) .topbar-pill {
    background: var(--bg-grouped);
    backdrop-filter: none;
    -webkit-backdrop-filter: none;
    border: 1px solid var(--separator);
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.15);
  }

  .top-bar.scrolled .topbar-pill::after {
    content: '';
    position: absolute;
    top: 0; left: 0; right: 0; bottom: 0;
    border-radius: inherit;
    background: linear-gradient(90deg, transparent 30%, rgba(255,255,255,0.04) 50%, transparent 70%);
    background-size: 200% 100%;
    animation: glass-shimmer 3s var(--ease) infinite;
    pointer-events: none;
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
