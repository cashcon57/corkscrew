<script lang="ts">
  type SortKey = "priority" | "name" | "date" | "version" | "files";
  type SortDir = "asc" | "desc";

  interface Props {
    sortBy: SortKey;
    sortDir: SortDir;
    columnWidths?: Record<string, number>;
    onSort: (key: SortKey) => void;
    onColumnResize?: (key: string, width: number) => void;
  }

  let { sortBy, sortDir, onSort }: Props = $props();
</script>

<!-- Phase 2 will implement sortable headers + resize handles -->
<div class="table-header">
  <span class="col-grip"></span>
  <span class="col-toggle"></span>
  <span class="col-name sortable" onclick={() => onSort("name")}>
    Name
    {#if sortBy === "name"}
      <span class="sort-indicator">{sortDir === "asc" ? "\u25B2" : "\u25BC"}</span>
    {/if}
  </span>
  <span class="col-version sortable" onclick={() => onSort("version")}>
    Version
    {#if sortBy === "version"}
      <span class="sort-indicator">{sortDir === "asc" ? "\u25B2" : "\u25BC"}</span>
    {/if}
  </span>
  <span class="col-files sortable" onclick={() => onSort("files")}>
    Files
    {#if sortBy === "files"}
      <span class="sort-indicator">{sortDir === "asc" ? "\u25B2" : "\u25BC"}</span>
    {/if}
  </span>
  <span class="col-date sortable" onclick={() => onSort("date")}>
    Installed
    {#if sortBy === "date"}
      <span class="sort-indicator">{sortDir === "asc" ? "\u25B2" : "\u25BC"}</span>
    {/if}
  </span>
  <span class="col-actions">Actions</span>
</div>

<style>
  .sortable {
    cursor: pointer;
    user-select: none;
  }

  .sortable:hover {
    color: var(--text-primary);
  }

  .sort-indicator {
    font-size: 9px;
    margin-left: 2px;
    color: var(--accent);
  }
</style>
