<script lang="ts">
  import type { InstalledMod } from "$lib/types";

  interface Props {
    mods: InstalledMod[];
    categoryColors: Record<string, string>;
    categoryIcons: Record<string, string>;
    conflictModIds: Set<number>;
    togglingMod: number | null;
    selectedModId: number | undefined;
    onselect: (mod: InstalledMod) => void;
    ontoggle: (mod: InstalledMod) => void;
  }

  let {
    mods, categoryColors, categoryIcons, conflictModIds, togglingMod,
    selectedModId, onselect, ontoggle,
  }: Props = $props();

  // Preference: show disabled mods within their categories vs. in a separate section
  let disabledInCategory = $state(
    (() => { try { return localStorage.getItem("corkscrew:disabledInCategory") === "true"; } catch { return false; } })()
  );

  $effect(() => { try { localStorage.setItem("corkscrew:disabledInCategory", String(disabledInCategory)); } catch {} });

  // Separate enabled and disabled mods
  let enabledMods = $derived(mods.filter(m => m.enabled));
  let disabledMods = $derived(mods.filter(m => !m.enabled));

  // Group mods by auto_category
  let categorized = $derived((() => {
    const source = disabledInCategory ? mods : enabledMods;
    const groups = new Map<string, InstalledMod[]>();
    for (const mod of source) {
      const cat = mod.auto_category || "Miscellaneous";
      if (!groups.has(cat)) groups.set(cat, []);
      groups.get(cat)!.push(mod);
    }
    // Sort categories alphabetically, but "Miscellaneous" last
    return [...groups.entries()].sort(([a], [b]) => {
      if (a === "Miscellaneous") return 1;
      if (b === "Miscellaneous") return -1;
      return a.localeCompare(b);
    });
  })());

  // Expand/collapse state (all collapsed by default)
  let expandedKey = "corkscrew:cat-expanded";
  let expanded = $state<Set<string>>(new Set());

  function toggleCategory(cat: string) {
    const next = new Set(expanded);
    if (next.has(cat)) next.delete(cat);
    else next.add(cat);
    expanded = next;
    try { localStorage.setItem(expandedKey, JSON.stringify([...next])); } catch {}
  }

  // Restore expand state from localStorage
  $effect(() => {
    try {
      const saved = localStorage.getItem(expandedKey);
      if (saved) expanded = new Set(JSON.parse(saved));
    } catch {}
  });
</script>

<div class="category-view">
  <div class="category-options">
    <label class="category-option-label">
      <input type="checkbox" bind:checked={disabledInCategory} />
      Show disabled in-category
    </label>
  </div>
  {#each categorized as [category, categoryMods]}
    {@const catColor = categoryColors[category] ?? '#6b7280'}
    <div class="category-group">
      <button class="category-header" onclick={() => toggleCategory(category)}>
        <svg
          class="category-chevron"
          class:expanded={expanded.has(category)}
          width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
        >
          <path d="M4 2l4 4-4 4" />
        </svg>
        {#if categoryIcons[category]}
          <!-- eslint-disable-next-line svelte/no-at-html-tags -->
          <svg class="category-header-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke={catColor} stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html categoryIcons[category]}</svg>
        {/if}
        <span
          class="category-chip"
          style="background: color-mix(in srgb, {catColor} 15%, transparent); color: {catColor};"
        >
          {category}
        </span>
        <span class="category-count">{categoryMods.length}</span>
      </button>
      {#if expanded.has(category)}
        <div class="category-mods">
          {#each categoryMods as mod (mod.id)}
            <button
              class="category-mod-row"
              class:category-mod-disabled={!mod.enabled}
              class:category-mod-selected={selectedModId === mod.id}
              onclick={() => onselect(mod)}
            >
              <span class="category-mod-toggle" onclick={(e) => { e.stopPropagation(); ontoggle(mod); }}>
                <span
                  class="toggle-switch-mini"
                  class:toggle-on={mod.enabled}
                  class:toggle-busy={togglingMod === mod.id}
                >
                  <span class="toggle-track-mini"><span class="toggle-thumb-mini"></span></span>
                </span>
              </span>
              <span class="category-mod-name">{mod.name}</span>
              {#if conflictModIds.has(mod.id)}
                <span class="category-conflict-icon" title="Has file conflicts">
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
                    <line x1="12" y1="9" x2="12" y2="13" /><line x1="12" y1="17" x2="12.01" y2="17" />
                  </svg>
                </span>
              {/if}
              <span class="category-mod-version">{mod.version || "\u2014"}</span>
            </button>
          {/each}
        </div>
      {/if}
    </div>
  {/each}

  <!-- Disabled section (only when not in-category) -->
  {#if !disabledInCategory && disabledMods.length > 0}
    <div class="category-group">
      <button class="category-header" onclick={() => toggleCategory("__disabled__")}>
        <svg
          class="category-chevron"
          class:expanded={expanded.has("__disabled__")}
          width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
        >
          <path d="M4 2l4 4-4 4" />
        </svg>
        <span class="category-chip category-chip-disabled">
          Disabled
        </span>
        <span class="category-count">{disabledMods.length}</span>
      </button>
      {#if expanded.has("__disabled__")}
        <div class="category-mods">
          {#each disabledMods as mod (mod.id)}
            <button
              class="category-mod-row category-mod-disabled"
              class:category-mod-selected={selectedModId === mod.id}
              onclick={() => onselect(mod)}
            >
              <span class="category-mod-toggle" onclick={(e) => { e.stopPropagation(); ontoggle(mod); }}>
                <span
                  class="toggle-switch-mini"
                  class:toggle-on={mod.enabled}
                  class:toggle-busy={togglingMod === mod.id}
                >
                  <span class="toggle-track-mini"><span class="toggle-thumb-mini"></span></span>
                </span>
              </span>
              <span class="category-mod-name">{mod.name}</span>
              <span class="category-mod-version">{mod.version || "\u2014"}</span>
            </button>
          {/each}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .category-view {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .category-header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: var(--space-2) var(--space-3);
    cursor: pointer;
    border-radius: var(--radius-sm);
    transition: background var(--duration-fast) var(--ease);
    text-align: left;
  }

  .category-header:hover {
    background: var(--surface-hover);
  }

  .category-chevron {
    transition: transform var(--duration-fast) var(--ease);
    flex-shrink: 0;
    color: var(--text-tertiary);
  }

  .category-chevron.expanded {
    transform: rotate(90deg);
  }

  .category-header-icon {
    flex-shrink: 0;
  }

  .category-chip {
    font-size: 12px;
    font-weight: 600;
    padding: 1px 8px;
    border-radius: 4px;
  }

  .category-count {
    font-size: 12px;
    color: var(--text-tertiary);
    font-weight: 500;
  }

  .category-mods {
    padding-left: 20px;
  }

  .category-mod-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: 3px var(--space-3);
    font-size: 13px;
    color: var(--text-secondary);
    cursor: pointer;
    border-radius: var(--radius-sm);
    text-align: left;
    transition: background var(--duration-fast) var(--ease);
  }

  .category-mod-row:hover {
    background: var(--surface-hover);
  }

  .category-mod-selected {
    background: var(--accent-subtle);
  }

  .category-mod-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .category-mod-version {
    font-size: 12px;
    color: var(--text-tertiary);
    flex-shrink: 0;
  }

  .category-mod-disabled {
    opacity: 0.45;
  }

  .category-mod-toggle {
    flex-shrink: 0;
    display: flex;
    align-items: center;
  }

  .category-conflict-icon {
    flex-shrink: 0;
    color: var(--warning, #f59e0b);
    display: flex;
    align-items: center;
  }

  /* Mini toggle switch for category rows */
  .toggle-switch-mini {
    display: inline-flex;
    align-items: center;
    cursor: pointer;
  }

  .toggle-track-mini {
    width: 24px;
    height: 14px;
    border-radius: 7px;
    background: var(--text-tertiary);
    opacity: 0.3;
    position: relative;
    transition: all var(--duration-fast) var(--ease);
  }

  .toggle-on .toggle-track-mini {
    background: var(--accent, #6366f1);
    opacity: 1;
  }

  .toggle-busy .toggle-track-mini {
    opacity: 0.5;
  }

  .toggle-thumb-mini {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: white;
    transition: transform var(--duration-fast) var(--ease);
  }

  .toggle-on .toggle-thumb-mini {
    transform: translateX(10px);
  }

  .category-chip-disabled {
    background: color-mix(in srgb, var(--text-tertiary) 12%, transparent) !important;
    color: var(--text-tertiary) !important;
  }

  .category-options {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-3);
    margin-bottom: var(--space-1);
  }

  .category-option-label {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--text-tertiary);
    cursor: pointer;
    user-select: none;
  }

  .category-option-label input {
    accent-color: var(--system-accent);
  }
</style>
