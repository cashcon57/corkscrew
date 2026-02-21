<script lang="ts">
  import { currentPage, selectedGame, installedMods } from "$lib/stores";
  import { get } from "svelte/store";
  import type { InstalledMod } from "$lib/types";

  let { onClose }: { onClose: () => void } = $props();

  let query = $state("");
  let selectedIndex = $state(0);
  let inputEl = $state<HTMLInputElement | null>(null);

  // Focus input when shown
  $effect(() => {
    inputEl?.focus();
  });

  // Search result categories
  interface SearchResult {
    type: "page" | "mod" | "action";
    label: string;
    description: string;
    action: () => void;
  }

  const pages: SearchResult[] = [
    { type: "page", label: "Dashboard", description: "Bottles and game detection", action: () => { currentPage.set("dashboard"); onClose(); } },
    { type: "page", label: "Mods", description: "Install and manage mods", action: () => { currentPage.set("mods"); onClose(); } },
    { type: "page", label: "Load Order", description: "Plugin load order and LOOT", action: () => { currentPage.set("plugins"); onClose(); } },
    { type: "page", label: "Discover", description: "Browse collections and modlists", action: () => { currentPage.set("discover"); onClose(); } },
    { type: "page", label: "Profiles", description: "Manage mod profiles", action: () => { currentPage.set("profiles"); onClose(); } },
    { type: "page", label: "Crash Logs", description: "Analyze crash logs", action: () => { currentPage.set("logs"); onClose(); } },
    { type: "page", label: "Settings", description: "App configuration", action: () => { currentPage.set("settings"); onClose(); } },
  ];

  const actions: SearchResult[] = [
    { type: "action", label: "Deploy Mods", description: "Deploy all enabled mods", action: () => { currentPage.set("mods"); onClose(); } },
    { type: "action", label: "Sort with LOOT", description: "Auto-sort plugin load order", action: () => { currentPage.set("plugins"); onClose(); } },
    { type: "action", label: "Check for Updates", description: "Check NexusMods for mod updates", action: () => { currentPage.set("mods"); onClose(); } },
  ];

  const results = $derived.by((): SearchResult[] => {
    const q = query.toLowerCase().trim();
    if (!q) return [...pages, ...actions];

    const mods = get(installedMods);
    const modResults: SearchResult[] = mods
      .filter(m => m.name.toLowerCase().includes(q))
      .slice(0, 8)
      .map(m => ({
        type: "mod" as const,
        label: m.name,
        description: `Priority ${m.install_priority} · ${m.enabled ? "Enabled" : "Disabled"}`,
        action: () => { currentPage.set("mods"); onClose(); },
      }));

    const matchingPages = pages.filter(p =>
      p.label.toLowerCase().includes(q) || p.description.toLowerCase().includes(q)
    );
    const matchingActions = actions.filter(a =>
      a.label.toLowerCase().includes(q) || a.description.toLowerCase().includes(q)
    );

    return [...matchingPages, ...matchingActions, ...modResults];
  });

  // Reset selection when results change
  $effect(() => {
    if (results) selectedIndex = 0;
  });

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      selectedIndex = Math.min(selectedIndex + 1, results.length - 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      selectedIndex = Math.max(selectedIndex - 1, 0);
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (results[selectedIndex]) results[selectedIndex].action();
    } else if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    }
  }

  const typeLabels: Record<string, string> = {
    page: "Page",
    mod: "Mod",
    action: "Action",
  };
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div class="spotlight-overlay" onclick={onClose} onkeydown={handleKeydown} role="dialog">
  <div class="spotlight-card" onclick={(e) => e.stopPropagation()}>
    <div class="spotlight-input-row">
      <svg class="spotlight-search-icon" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="11" cy="11" r="8" />
        <line x1="21" y1="21" x2="16.65" y2="16.65" />
      </svg>
      <input
        bind:this={inputEl}
        type="text"
        class="spotlight-input"
        placeholder="Search pages, mods, and actions..."
        bind:value={query}
        onkeydown={handleKeydown}
      />
      <kbd class="spotlight-kbd">esc</kbd>
    </div>

    {#if results.length > 0}
      <div class="spotlight-results">
        {#each results as result, i}
          <button
            class="spotlight-result"
            class:selected={selectedIndex === i}
            onclick={() => result.action()}
            onmouseenter={() => selectedIndex = i}
          >
            <span class="result-type-badge" class:type-page={result.type === "page"} class:type-mod={result.type === "mod"} class:type-action={result.type === "action"}>
              {typeLabels[result.type]}
            </span>
            <div class="result-text">
              <span class="result-label">{result.label}</span>
              <span class="result-desc">{result.description}</span>
            </div>
          </button>
        {/each}
      </div>
    {:else}
      <div class="spotlight-empty">No results for "{query}"</div>
    {/if}
  </div>
</div>

<style>
  .spotlight-overlay {
    position: fixed;
    inset: 0;
    z-index: 9999;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding-top: 15vh;
    background: rgba(0, 0, 0, 0.5);
    backdrop-filter: blur(4px);
    -webkit-backdrop-filter: blur(4px);
  }

  .spotlight-card {
    width: 520px;
    max-height: 440px;
    background: var(--bg-base);
    border: 1px solid var(--separator);
    border-radius: var(--radius-lg);
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5);
    overflow: hidden;
    animation: spotlight-in 0.15s var(--ease);
  }

  @keyframes spotlight-in {
    from { opacity: 0; transform: scale(0.98) translateY(-8px); }
    to { opacity: 1; transform: scale(1) translateY(0); }
  }

  .spotlight-input-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--separator);
  }

  .spotlight-search-icon {
    color: var(--text-tertiary);
    flex-shrink: 0;
  }

  .spotlight-input {
    flex: 1;
    background: none;
    border: none;
    outline: none;
    font-size: 15px;
    color: var(--text-primary);
    font-weight: 400;
  }

  .spotlight-input::placeholder {
    color: var(--text-quaternary);
  }

  .spotlight-kbd {
    font-size: 10px;
    font-weight: 600;
    color: var(--text-quaternary);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: 4px;
    padding: 1px 6px;
    font-family: var(--font-mono);
  }

  .spotlight-results {
    max-height: 360px;
    overflow-y: auto;
    padding: var(--space-1) 0;
  }

  .spotlight-result {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    text-align: left;
    padding: var(--space-2) var(--space-4);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease);
  }

  .spotlight-result.selected {
    background: var(--surface-hover);
  }

  .result-type-badge {
    flex-shrink: 0;
    font-size: 9px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    padding: 2px 6px;
    border-radius: var(--radius-sm);
    min-width: 44px;
    text-align: center;
  }

  .type-page {
    background: var(--accent-subtle);
    color: var(--accent);
  }

  .type-mod {
    background: var(--green-subtle);
    color: var(--green);
  }

  .type-action {
    background: var(--yellow-subtle);
    color: var(--yellow);
  }

  .result-text {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }

  .result-label {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .result-desc {
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .spotlight-empty {
    padding: var(--space-6) var(--space-4);
    text-align: center;
    font-size: 13px;
    color: var(--text-tertiary);
  }
</style>
