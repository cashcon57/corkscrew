<script lang="ts">
  type FilterStatus = "all" | "enabled" | "disabled" | "conflicts" | "has-updates";
  type ViewMode = "flat" | "collection" | "category";

  interface Props {
    searchQuery: string;
    filterStatus: FilterStatus;
    filterCollection: string | null;
    collectionNames: string[];
    modCount: number;
    filteredCount: number;
    viewMode: ViewMode;
    onSearchChange: (query: string) => void;
    onFilterStatusChange: (status: FilterStatus) => void;
    onFilterCollectionChange: (collection: string | null) => void;
    onViewModeChange: (mode: ViewMode) => void;
  }

  let {
    searchQuery, filterStatus, filterCollection, collectionNames,
    modCount, filteredCount, viewMode,
    onSearchChange, onFilterStatusChange, onFilterCollectionChange,
    onViewModeChange,
  }: Props = $props();

  let hasActiveFilters = $derived(searchQuery !== "" || filterStatus !== "all" || filterCollection !== null);
</script>

<div class="filter-bar">
  <div class="search-box">
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <circle cx="11" cy="11" r="8" /><line x1="21" y1="21" x2="16.65" y2="16.65" />
    </svg>
    <input
      type="text"
      placeholder="Search mods..."
      value={searchQuery}
      oninput={(e) => onSearchChange(e.currentTarget.value)}
      class="search-input"
    />
    {#if searchQuery}
      <button class="search-clear" onclick={() => onSearchChange("")}>
        <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
          <line x1="3" y1="3" x2="9" y2="9" /><line x1="9" y1="3" x2="3" y2="9" />
        </svg>
      </button>
    {/if}
  </div>

  <select class="filter-select" value={filterStatus} onchange={(e) => onFilterStatusChange(e.currentTarget.value as FilterStatus)}>
    <option value="all">All Status</option>
    <option value="enabled">Enabled</option>
    <option value="disabled">Disabled</option>
    <option value="conflicts">Has Conflicts</option>
    <option value="has-updates">Has Updates</option>
  </select>

  {#if collectionNames.length > 0}
    <select class="filter-select" value={filterCollection ?? ""} onchange={(e) => onFilterCollectionChange(e.currentTarget.value || null)}>
      <option value="">All Sources</option>
      <option value="__standalone__">Standalone</option>
      {#each collectionNames as name}
        <option value={name}>{name}</option>
      {/each}
    </select>
  {/if}

  <!-- View mode toggle (Phase 4) -->
  <div class="view-mode-toggle">
    <button
      class="view-mode-btn"
      class:active={viewMode === "flat"}
      onclick={() => onViewModeChange("flat")}
      title="List view"
    >
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <line x1="8" y1="6" x2="21" y2="6" /><line x1="8" y1="12" x2="21" y2="12" /><line x1="8" y1="18" x2="21" y2="18" />
        <line x1="3" y1="6" x2="3.01" y2="6" /><line x1="3" y1="12" x2="3.01" y2="12" /><line x1="3" y1="18" x2="3.01" y2="18" />
      </svg>
    </button>
    <button
      class="view-mode-btn"
      class:active={viewMode === "collection"}
      onclick={() => onViewModeChange("collection")}
      title="Group by collection"
    >
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
      </svg>
    </button>
    <button
      class="view-mode-btn"
      class:active={viewMode === "category"}
      onclick={() => onViewModeChange("category")}
      title="Group by category"
    >
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <rect x="3" y="3" width="7" height="7" /><rect x="14" y="3" width="7" height="7" />
        <rect x="14" y="14" width="7" height="7" /><rect x="3" y="14" width="7" height="7" />
      </svg>
    </button>
  </div>

  {#if hasActiveFilters}
    <span class="filter-count">{filteredCount} of {modCount}</span>
  {/if}
</div>

<style>
  .view-mode-toggle {
    display: flex;
    gap: 0;
    border: 1px solid var(--separator);
    border-radius: var(--radius-sm);
    overflow: hidden;
  }

  .view-mode-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 30px;
    height: 28px;
    color: var(--text-tertiary);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
    border-right: 1px solid var(--separator);
  }

  .view-mode-btn:last-child {
    border-right: none;
  }

  .view-mode-btn:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .view-mode-btn.active {
    background: var(--accent-subtle);
    color: var(--accent);
  }
</style>
