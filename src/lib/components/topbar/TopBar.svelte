<script lang="ts">
  import { goto } from "$app/navigation";
  import TopBarGameSelector from "./TopBarGameSelector.svelte";
  import TopBarModlistSelector from "./TopBarModlistSelector.svelte";
  import TopBarProfileSelector from "./TopBarProfileSelector.svelte";
  import type { DetectedGame } from "$lib/types";
  import { nativeMode, nativeModeVisible } from "$lib/stores";
  import { setNativeMode } from "$lib/api";
  import { applyNativeTheme } from "$lib/native/theme";

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

  let nativeToggling = $state(false);

  async function toggleNativeMode() {
    if (nativeToggling) return; // debounce rapid clicks
    nativeToggling = true;
    const newValue = !$nativeMode;
    try {
      // Persist the new value to the backend first so the config is
      // consistent if the app restarts mid-navigation.
      await setNativeMode(newValue);
      nativeMode.set(newValue);
      // Navigate BEFORE applying the theme so that Wine pages never briefly
      // render with native theme tokens (the "in-between" state the user saw).
      // applyNativeTheme is called after goto() returns to guarantee the
      // destination route is already mounted.
      if (newValue) {
        await goto("/native").catch((err) => console.error("navigation to /native failed:", err));
      } else {
        await goto("/").catch((err) => console.error("navigation to / failed:", err));
      }
      await applyNativeTheme(newValue);
    } catch (err) {
      console.error("toggleNativeMode failed:", err);
    } finally {
      nativeToggling = false;
    }
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

    {#if $nativeModeVisible}
      <span class="topbar-separator topbar-separator-spacer"></span>

      <!-- Two-position slider: both labels always visible, thumb slides under active side -->
      <div
        class="topbar-mode-slider"
        role="radiogroup"
        aria-label="Mod runtime mode"
      >
        <button
          class="slider-option"
          class:active={!$nativeMode}
          role="radio"
          aria-checked={!$nativeMode}
          onclick={() => { if ($nativeMode) toggleNativeMode(); }}
          disabled={nativeToggling}
          title="Wine / CrossOver"
        >
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
            <path d="M8 22h8M12 22v-7M5 2h14l-1 6a6 6 0 1 1-12 0L5 2z"/>
          </svg>
          Wine
        </button>
        <button
          class="slider-option"
          class:active={$nativeMode}
          role="radio"
          aria-checked={$nativeMode}
          onclick={() => { if (!$nativeMode) toggleNativeMode(); }}
          disabled={nativeToggling}
          title="Native macOS"
        >
          <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" aria-hidden="true">
            <path d="M8 1.5v3M8 11.5v3M1.5 8h3M11.5 8h3M3.5 3.5l2 2M10.5 10.5l2 2M3.5 12.5l2-2M10.5 5.5l2-2"/>
          </svg>
          Native
        </button>
        <div class="slider-thumb" class:right={$nativeMode}></div>
      </div>
    {/if}
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

  /* Push the mode slider to the far right of the pill */
  .topbar-separator-spacer {
    flex: 1;
    min-width: 8px;
    padding: 0;
    opacity: 0;
    pointer-events: none;
  }

  /* Two-position slider */
  .topbar-mode-slider {
    position: relative;
    display: inline-flex;
    background: var(--bg-grouped, rgba(255, 255, 255, 0.06));
    border: 1px solid var(--separator, rgba(255, 255, 255, 0.1));
    border-radius: 100px;
    padding: 2px;
    gap: 0;
    isolation: isolate;
    -webkit-app-region: no-drag;
  }

  .slider-option {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 5px 10px;
    background: transparent;
    border: none;
    color: var(--text-secondary);
    cursor: pointer;
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.01em;
    white-space: nowrap;
    border-radius: 100px;
    position: relative;
    z-index: 1;
    transition: color 200ms var(--ease, ease);
    -webkit-app-region: no-drag;
  }

  .slider-option.active {
    color: var(--text-primary);
  }

  .slider-option:not(.active):hover {
    color: var(--text-primary);
  }

  .slider-option:disabled {
    cursor: wait;
    opacity: 0.6;
  }

  /* Sliding highlight thumb — translates to the right half when native is active */
  .slider-thumb {
    position: absolute;
    top: 2px;
    bottom: 2px;
    left: 2px;
    width: calc(50% - 2px);
    background: var(--accent-subtle, rgba(91, 115, 255, 0.16));
    border-radius: 100px;
    transition: transform 220ms cubic-bezier(0.4, 0, 0.2, 1);
    z-index: 0;
    pointer-events: none;
  }

  .slider-thumb.right {
    transform: translateX(100%);
  }

  /* Native-side accent when native mode is active */
  :global([data-theme="native"]) .slider-thumb.right {
    background: linear-gradient(
      135deg,
      rgba(78, 203, 255, 0.25),
      rgba(168, 85, 247, 0.25)
    );
  }
</style>
