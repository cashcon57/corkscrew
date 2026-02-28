<script lang="ts">
  import type { InstalledMod } from "$lib/types";

  interface Props {
    mods: InstalledMod[];
  }

  let { mods }: Props = $props();

  // Group mods by auto_category
  let categorized = $derived((() => {
    const groups = new Map<string, InstalledMod[]>();
    for (const mod of mods) {
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

  const categoryColors: Record<string, string> = {
    "Gameplay":          "#6366f1",  // Indigo
    "Texture":           "#22c55e",  // Green
    "Model & Mesh":      "#f59e0b",  // Amber
    "Framework":         "#ef4444",  // Red
    "Audio":             "#8b5cf6",  // Violet
    "UI":                "#06b6d4",  // Cyan
    "Script":            "#f97316",  // Orange
    "Lighting & Weather":"#ec4899",  // Pink
    "Animation":         "#3b82f6",  // Blue
    "Misc":              "#6b7280",  // Gray
    "Miscellaneous":     "#6b7280",  // Gray (legacy)
  };
</script>

<!-- Phase 4 will implement full category tree with mod rows inside -->
<div class="category-view">
  {#each categorized as [category, categoryMods]}
    <div class="category-group">
      <button class="category-header" onclick={() => toggleCategory(category)}>
        <svg
          class="category-chevron"
          class:expanded={expanded.has(category)}
          width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
        >
          <path d="M4 2l4 4-4 4" />
        </svg>
        <span
          class="category-chip"
          style="background: color-mix(in srgb, {categoryColors[category] ?? '#6b7280'} 15%, transparent); color: {categoryColors[category] ?? '#6b7280'};"
        >
          {category}
        </span>
        <span class="category-count">{categoryMods.length}</span>
      </button>
      {#if expanded.has(category)}
        <div class="category-mods">
          {#each categoryMods as mod (mod.id)}
            <div class="category-mod-row">
              <span class="category-mod-name">{mod.name}</span>
              <span class="category-mod-version">{mod.version || "\u2014"}</span>
            </div>
          {/each}
        </div>
      {/if}
    </div>
  {/each}
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
    padding-left: 28px;
  }

  .category-mod-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-1) var(--space-3);
    font-size: 13px;
    color: var(--text-secondary);
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
</style>
