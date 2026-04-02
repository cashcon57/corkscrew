/**
 * Reactive filter/sort state composable for the mods table.
 * Uses Svelte 5 runes ($state, $derived) via the .svelte.ts extension.
 */

import type { InstalledMod, FileConflict, ModUpdateInfo } from "$lib/types";

export type FilterStatus = "all" | "enabled" | "disabled" | "conflicts" | "has-updates";
export type FilterSource = "all" | "nexus" | "loverslab" | "moddb" | "curseforge" | "direct" | "manual";
export type SortKey = "priority" | "name" | "date" | "version" | "files";
export type SortDirection = "asc" | "desc";
export type ViewMode = "flat" | "collection" | "category";

const FACET_PREFIXES = ["tag", "source", "enabled", "conflict", "update", "category", "collection", "priority", "files"] as const;
type FacetKey = typeof FACET_PREFIXES[number];

export interface ParsedSearch {
  facets: Map<FacetKey, string>;
  freeText: string;
}

function parseFacets(query: string): ParsedSearch {
  const facets = new Map<FacetKey, string>();
  const freeWords: string[] = [];
  const tokens = query.match(/(?:[^\s"]+|"[^"]*")+/g) ?? [];

  for (const token of tokens) {
    const colonIdx = token.indexOf(":");
    if (colonIdx > 0) {
      const prefix = token.slice(0, colonIdx).toLowerCase();
      if (FACET_PREFIXES.includes(prefix as FacetKey)) {
        facets.set(prefix as FacetKey, token.slice(colonIdx + 1).replace(/^"|"$/g, ""));
        continue;
      }
    }
    freeWords.push(token);
  }

  return { facets, freeText: freeWords.join(" ") };
}

function compareVersions(a: string, b: string): number {
  const pa = a.split(".").map(Number);
  const pb = b.split(".").map(Number);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const diff = (pa[i] || 0) - (pb[i] || 0);
    if (diff !== 0) return diff;
  }
  return 0;
}

export function createModFilters() {
  let searchQuery = $state("");
  let filterStatus = $state<FilterStatus>("all");
  let filterSource = $state<FilterSource>("all");
  let filterCollection = $state<string | null>(null);
  let filterCategory = $state<string | null>(null);
  let sortBy = $state<SortKey>("priority");
  let sortDir = $state<SortDirection>("asc");
  let showCategoryPopover = $state(false);
  let viewMode = $state<ViewMode>(
    (() => {
      try {
        const v = localStorage.getItem("corkscrew:viewMode");
        if (v === "flat" || v === "collection" || v === "category") return v;
      } catch { /* ignore */ }
      return "flat";
    })()
  );

  // Persist view mode preference
  $effect(() => {
    try { localStorage.setItem("corkscrew:viewMode", viewMode); } catch { /* ignore */ }
  });

  /** Toggle sort: click same column toggles direction, different column sets ascending */
  function toggleSort(key: SortKey) {
    if (sortBy === key) {
      sortDir = sortDir === "asc" ? "desc" : "asc";
    } else {
      sortBy = key;
      sortDir = "asc";
    }
  }

  /** Clear all filters and search back to defaults */
  function clearAll() {
    searchQuery = "";
    filterStatus = "all";
    filterSource = "all";
    filterCollection = null;
    filterCategory = null;
  }

  /** Whether any filter is active (for showing "X of Y" count) */
  function hasActiveFilters(): boolean {
    return searchQuery !== "" || filterStatus !== "all" || filterSource !== "all" || filterCollection !== null || filterCategory !== null;
  }

  /** Parsed faceted search derived from searchQuery */
  let parsedSearch = $derived(parseFacets(searchQuery));

  /**
   * Sort mods by the current sort key/direction with secondary sort for stability.
   */
  function sortMods(mods: InstalledMod[]): InstalledMod[] {
    const items = [...mods];
    const dir = sortDir === "asc" ? 1 : -1;
    items.sort((a, b) => {
      let primary: number;
      switch (sortBy) {
        case "name":
          primary = a.name.localeCompare(b.name);
          return dir * primary || (a.install_priority - b.install_priority);
        case "date":
          primary = new Date(a.installed_at).getTime() - new Date(b.installed_at).getTime();
          return dir * primary || a.name.localeCompare(b.name);
        case "version":
          primary = compareVersions(a.version || "0", b.version || "0");
          return dir * primary || a.name.localeCompare(b.name);
        case "files":
          primary = a.file_count - b.file_count;
          return dir * primary || a.name.localeCompare(b.name);
        default:
          primary = a.install_priority - b.install_priority;
          return dir * primary || a.name.localeCompare(b.name);
      }
    });
    return items;
  }

  /**
   * Filter mods based on all active filters + faceted search.
   * Requires external state (conflictModIds, updateMap) passed as params.
   */
  function filterMods(
    sortedMods: InstalledMod[],
    conflictModIds: Set<number>,
    updateMap: Map<number, ModUpdateInfo>,
  ): InstalledMod[] {
    const { facets, freeText } = parsedSearch;

    // Pre-compute facet values outside the loop
    const tagFacet = facets.get("tag")?.toLowerCase() ?? null;
    const sourceFacet = facets.get("source")?.toLowerCase() ?? null;
    const enabledFacet = facets.has("enabled") ? facets.get("enabled")!.toLowerCase() === "true" : null;
    const conflictFacet = facets.get("conflict")?.toLowerCase() === "true" || false;
    const updateFacet = facets.get("update")?.toLowerCase() === "true" || false;
    const categoryFacet = facets.get("category")?.toLowerCase() ?? null;
    const collectionFacet = facets.get("collection")?.toLowerCase() ?? null;

    const priorityFacet = facets.get("priority");
    let priorityOp: ">" | "<" | null = null;
    let priorityN = 0;
    if (priorityFacet) {
      const match = priorityFacet.match(/^([><])(\d+)$/);
      if (match) { priorityOp = match[1] as ">" | "<"; priorityN = parseInt(match[2]); }
    }

    const filesFacet = facets.get("files");
    let filesOp: ">" | "<" | null = null;
    let filesN = 0;
    if (filesFacet) {
      const match = filesFacet.match(/^([><])(\d+)$/);
      if (match) { filesOp = match[1] as ">" | "<"; filesN = parseInt(match[2]); }
    }

    const q = freeText.trim() ? freeText.toLowerCase() : null;

    // Local copies of dropdown filter state for the closure
    const fStatus = filterStatus;
    const fSource = filterSource;
    const fCollection = filterCollection;
    const fCategory = filterCategory;

    return sortedMods.filter(m => {
      // Facet: tag
      if (tagFacet !== null && !m.user_tags.some(tag => tag.toLowerCase().includes(tagFacet))) return false;
      // Facet: source
      if (sourceFacet !== null && (m.source_type || "manual").toLowerCase() !== sourceFacet) return false;
      // Facet: enabled
      if (enabledFacet !== null && m.enabled !== enabledFacet) return false;
      // Facet: conflict
      if (conflictFacet && !conflictModIds.has(m.id)) return false;
      // Facet: update
      if (updateFacet && !updateMap.has(m.id)) return false;
      // Facet: category
      if (categoryFacet !== null && !m.auto_category?.toLowerCase().includes(categoryFacet)) return false;
      // Facet: collection
      if (collectionFacet !== null && !m.collection_name?.toLowerCase().includes(collectionFacet)) return false;
      // Facet: priority
      if (priorityOp === ">" && m.install_priority <= priorityN) return false;
      if (priorityOp === "<" && m.install_priority >= priorityN) return false;
      // Facet: files
      if (filesOp === ">" && m.file_count <= filesN) return false;
      if (filesOp === "<" && m.file_count >= filesN) return false;
      // Free text search across name, tags, notes, collection, category
      if (q !== null &&
        !m.name.toLowerCase().includes(q) &&
        !m.user_tags.some(t => t.toLowerCase().includes(q)) &&
        !(m.user_notes && m.user_notes.toLowerCase().includes(q)) &&
        !(m.collection_name && m.collection_name.toLowerCase().includes(q)) &&
        !(m.auto_category && m.auto_category.toLowerCase().includes(q))
      ) return false;
      // Dropdown: status filter
      if (fStatus === "enabled" && !m.enabled) return false;
      if (fStatus === "disabled" && m.enabled) return false;
      if (fStatus === "conflicts" && !conflictModIds.has(m.id)) return false;
      if (fStatus === "has-updates" && !updateMap.has(m.id)) return false;
      // Dropdown: source filter
      if (fSource !== "all" && (m.source_type || "manual") !== fSource) return false;
      // Dropdown: collection filter
      if (fCollection !== null) {
        if (fCollection === "__standalone__") { if (m.collection_name) return false; }
        else { if (m.collection_name !== fCollection) return false; }
      }
      // Dropdown: category filter
      if (fCategory !== null && (m.auto_category || "Miscellaneous") !== fCategory) return false;

      return true;
    });
  }

  return {
    get searchQuery() { return searchQuery; },
    set searchQuery(v: string) { searchQuery = v; },
    get filterStatus() { return filterStatus; },
    set filterStatus(v: FilterStatus) { filterStatus = v; },
    get filterSource() { return filterSource; },
    set filterSource(v: FilterSource) { filterSource = v; },
    get filterCollection() { return filterCollection; },
    set filterCollection(v: string | null) { filterCollection = v; },
    get filterCategory() { return filterCategory; },
    set filterCategory(v: string | null) { filterCategory = v; },
    get sortBy() { return sortBy; },
    set sortBy(v: SortKey) { sortBy = v; },
    get sortDir() { return sortDir; },
    set sortDir(v: SortDirection) { sortDir = v; },
    get showCategoryPopover() { return showCategoryPopover; },
    set showCategoryPopover(v: boolean) { showCategoryPopover = v; },
    get viewMode() { return viewMode; },
    set viewMode(v: ViewMode) { viewMode = v; },
    get parsedSearch() { return parsedSearch; },
    toggleSort,
    clearAll,
    hasActiveFilters,
    sortMods,
    filterMods,
  };
}
