<script lang="ts">
  /**
   * Shared search + filter bar used across Collections, Browse Nexus, and Wabbajack pages.
   *
   * Layout: [Game Selector | Search Input]  [Controls Strip: sort, NSFW, filters, extras]
   */

  import type { Snippet } from "svelte";

  interface Props {
    /** Search input placeholder text */
    searchPlaceholder?: string;
    /** Current search value (bindable) */
    searchValue?: string;
    /** Called when search input changes */
    onsearch?: (value: string) => void;
    /** Slot for the game selector (rendered as search prefix) */
    gameSelector?: Snippet;
    /** Slot for controls inside the toolbar strip */
    controls?: Snippet;
  }

  let {
    searchPlaceholder = "Search...",
    searchValue = $bindable(""),
    onsearch,
    gameSelector,
    controls,
  }: Props = $props();

  function handleInput(e: Event) {
    const value = (e.target as HTMLInputElement).value;
    searchValue = value;
    onsearch?.(value);
  }
</script>

<div class="sfb-bar">
  <div class="sfb-search-combo">
    {#if gameSelector}
      <div class="sfb-game-prefix">
        {@render gameSelector()}
      </div>
    {/if}
    <div class="sfb-search-wrapper">
      <svg class="sfb-search-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="11" cy="11" r="8" />
        <line x1="21" y1="21" x2="16.65" y2="16.65" />
      </svg>
      <input
        type="text"
        class="sfb-search-input"
        placeholder={searchPlaceholder}
        value={searchValue}
        oninput={handleInput}
      />
    </div>
  </div>

  {#if controls}
    <div class="sfb-controls-strip">
      {@render controls()}
    </div>
  {/if}
</div>

<style>
  .sfb-bar {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-bottom: var(--space-4);
    flex-wrap: wrap;
  }

  /* Search combo: optional game prefix + search input */
  .sfb-search-combo {
    flex: 1;
    min-width: 260px;
    display: flex;
    align-items: stretch;
    background: color-mix(in srgb, var(--surface) 60%, transparent);
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
    border: 1px solid color-mix(in srgb, var(--separator) 50%, transparent);
    border-radius: var(--radius);
    overflow: hidden;
    transition: border-color var(--duration-fast) var(--ease);
  }
  .sfb-search-combo:focus-within {
    border-color: var(--system-accent);
  }

  .sfb-game-prefix {
    display: flex;
    align-items: center;
    background: color-mix(in srgb, var(--surface-subtle) 70%, transparent);
    border-right: 1px solid color-mix(in srgb, var(--separator) 50%, transparent);
    flex-shrink: 1;
    max-width: 200px;
    overflow: hidden;
  }

  /* Style selects rendered inside the game prefix slot */
  .sfb-game-prefix :global(select) {
    padding: var(--space-2) var(--space-3);
    background: transparent;
    border: none;
    color: var(--text-secondary);
    font-size: 12px;
    font-weight: 500;
    outline: none;
    cursor: pointer;
    font-family: inherit;
    width: 100%;
  }

  .sfb-search-wrapper {
    flex: 1;
    min-width: 120px;
    position: relative;
  }

  .sfb-search-icon {
    position: absolute;
    left: var(--space-3);
    top: 50%;
    transform: translateY(-50%);
    color: var(--text-tertiary);
    pointer-events: none;
  }

  .sfb-search-input {
    width: 100%;
    padding: var(--space-2) var(--space-3) var(--space-2) 36px;
    background: transparent;
    border: none;
    color: var(--text-primary);
    font-size: 13px;
    outline: none;
    font-family: inherit;
  }

  .sfb-search-input::placeholder {
    color: var(--text-tertiary);
  }

  /* Controls toolbar strip */
  .sfb-controls-strip {
    display: flex;
    align-items: center;
    gap: 6px;
    background: color-mix(in srgb, var(--surface-subtle) 60%, transparent);
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
    border: 1px solid color-mix(in srgb, var(--separator) 30%, transparent);
    border-radius: 8px;
    padding: 4px 10px;
    flex-shrink: 0;
  }

  /* Shared styles for items inside the controls strip */
  .sfb-controls-strip :global(.strip-sep) {
    width: 1px;
    height: 16px;
    background: color-mix(in srgb, var(--separator) 50%, transparent);
    flex-shrink: 0;
  }

  .sfb-controls-strip :global(select) {
    padding: 3px 6px;
    background: transparent;
    border: none;
    color: var(--text-secondary);
    font-size: 12px;
    outline: none;
    cursor: pointer;
    font-family: inherit;
  }
  .sfb-controls-strip :global(select:hover) {
    color: var(--text-primary);
  }

  .sfb-controls-strip :global(button) {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 3px 6px;
    background: transparent;
    border: none;
    color: var(--text-secondary);
    font-size: 12px;
    cursor: pointer;
    font-family: inherit;
    transition: color var(--duration-fast) var(--ease);
    white-space: nowrap;
  }
  .sfb-controls-strip :global(button:hover) {
    color: var(--text-primary);
  }

  .sfb-controls-strip :global(.filter-badge) {
    font-size: 10px;
    font-weight: 700;
    background: var(--accent);
    color: white;
    padding: 0 5px;
    border-radius: 100px;
    min-width: 16px;
    text-align: center;
  }

  .sfb-controls-strip :global(.nsfw-indicator) {
    font-size: 10px;
  }
</style>
