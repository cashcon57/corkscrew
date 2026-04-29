<script lang="ts">
  /**
   * Generic re-orderable list for games whose load order is a file-based
   * ordered list of mod IDs (UE4 `~mods`, Unity, RimWorld `ModsConfig.xml`,
   * etc.). Mirrors the drag/drop pattern used by the Bethesda plugins page
   * but doesn't make any assumptions about ESP/ESL/ESM.
   */
  import {
    getFileBasedLoadOrder,
    setFileBasedLoadOrder,
  } from "$lib/api";
  import { selectedGame, showError } from "$lib/stores";
  import type { DetectedGame, LoadOrderEntry } from "$lib/types";
  import { wineCtx } from "$lib/types";

  let entries = $state<LoadOrderEntry[]>([]);
  let loading = $state(false);
  let saving = $state(false);
  let savedMessage = $state<string | null>(null);

  // Drag-and-drop state mirrors the plugins page: track the source index and
  // the current drag-over target so we can render the insertion indicator.
  let dragIndex = $state<number | null>(null);
  let dragOverIndex = $state<number | null>(null);

  let search = $state("");
  let filtered = $derived.by(() => {
    const term = search.trim().toLowerCase();
    if (!term) return entries;
    return entries.filter(
      (e) =>
        e.id.toLowerCase().includes(term) ||
        e.display_name.toLowerCase().includes(term),
    );
  });

  const enabledCount = $derived(entries.filter((e) => e.enabled).length);

  $effect(() => {
    if ($selectedGame) {
      loadEntries($selectedGame);
    }
  });

  async function loadEntries(game: DetectedGame) {
    loading = true;
    try {
      entries = await getFileBasedLoadOrder(game.game_id, (wineCtx(game)?.bottle_name ?? ""));
    } catch (e: unknown) {
      showError(`Failed to load order: ${e}`);
      entries = [];
    } finally {
      loading = false;
    }
  }

  async function persist() {
    if (!$selectedGame || saving) return;
    saving = true;
    savedMessage = null;
    try {
      await setFileBasedLoadOrder(
        $selectedGame.game_id,
        (wineCtx($selectedGame)?.bottle_name ?? ""),
        entries,
      );
      savedMessage = "Saved";
      // Auto-fade the message
      setTimeout(() => {
        savedMessage = null;
      }, 1500);
    } catch (e: unknown) {
      showError(`Failed to save load order: ${e}`);
      // Reload from disk on failure to recover the on-disk state
      if ($selectedGame) await loadEntries($selectedGame);
    } finally {
      saving = false;
    }
  }

  function handleDragStart(e: DragEvent, index: number) {
    dragIndex = index;
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = "move";
      e.dataTransfer.setData("text/plain", String(index));
    }
  }

  function handleDragOver(e: DragEvent, index: number) {
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
    dragOverIndex = index;
  }

  function handleDragEnd() {
    dragIndex = null;
    dragOverIndex = null;
  }

  async function handleDrop(e: DragEvent, toIndex: number) {
    e.preventDefault();
    if (dragIndex === null || dragIndex === toIndex) {
      handleDragEnd();
      return;
    }
    const moved = entries.splice(dragIndex, 1)[0];
    entries.splice(toIndex, 0, moved);
    entries = [...entries];
    handleDragEnd();
    await persist();
  }

  async function handleToggle(entry: LoadOrderEntry) {
    if (saving) return;
    const idx = entries.findIndex((e) => e.id === entry.id);
    if (idx < 0) return;
    entries[idx] = { ...entry, enabled: !entry.enabled };
    entries = [...entries];
    await persist();
  }

  async function handleMove(id: string, direction: "up" | "down") {
    const currentIndex = entries.findIndex((e) => e.id === id);
    if (currentIndex < 0) return;
    const newIndex =
      direction === "up"
        ? Math.max(0, currentIndex - 1)
        : Math.min(entries.length - 1, currentIndex + 1);
    if (newIndex === currentIndex) return;
    const moved = entries.splice(currentIndex, 1)[0];
    entries.splice(newIndex, 0, moved);
    entries = [...entries];
    await persist();
  }
</script>

<div class="lo-panel">
  <div class="page-header">
    <div class="header-title">
      <h2>Load Order</h2>
    </div>
    {#if entries.length > 0}
      <div class="header-meta">
        <span class="meta-chip">
          <span class="meta-value">{enabledCount}</span>
          <span class="meta-label">active</span>
        </span>
        <span class="meta-divider"></span>
        <span class="meta-chip">
          <span class="meta-value">{entries.length}</span>
          <span class="meta-label">total</span>
        </span>
      </div>
    {/if}
  </div>

  {#if loading}
    <div class="loading-state">
      <div class="spinner"></div>
      <p class="loading-text">Loading load order...</p>
    </div>
  {:else if entries.length === 0}
    <div class="empty-state">
      <div class="empty-icon">
        <svg
          width="40"
          height="40"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="1.5"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <line x1="8" y1="6" x2="21" y2="6" />
          <line x1="8" y1="12" x2="21" y2="12" />
          <line x1="8" y1="18" x2="21" y2="18" />
          <circle cx="4" cy="6" r="1" />
          <circle cx="4" cy="12" r="1" />
          <circle cx="4" cy="18" r="1" />
        </svg>
      </div>
      <p class="empty-title">No mods in the load order</p>
      <p class="empty-description">
        Once you install mods that participate in this game's load order they'll
        appear here. Drag rows to reorder, toggle to enable/disable.
      </p>
    </div>
  {:else}
    <div class="toolbar">
      <div class="toolbar-right">
        <div class="search-box">
          <svg
            width="13"
            height="13"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <circle cx="11" cy="11" r="8" />
            <line x1="21" y1="21" x2="16.65" y2="16.65" />
          </svg>
          <input
            type="text"
            placeholder="Filter mods..."
            bind:value={search}
            class="search-input"
          />
          {#if search}
            <button class="search-clear" onclick={() => (search = "")}>
              <svg
                width="10"
                height="10"
                viewBox="0 0 12 12"
                fill="none"
                stroke="currentColor"
                stroke-width="1.5"
                stroke-linecap="round"
              >
                <line x1="3" y1="3" x2="9" y2="9" />
                <line x1="9" y1="3" x2="3" y2="9" />
              </svg>
            </button>
          {/if}
        </div>
        {#if search}
          <span class="filter-count">{filtered.length} of {entries.length}</span>
        {/if}
        {#if savedMessage}
          <span class="saved-message">{savedMessage}</span>
        {/if}
      </div>
    </div>

    <div class="list-container">
      <div class="list-header">
        <span class="col-drag"></span>
        <span class="col-index">#</span>
        <span class="col-toggle"></span>
        <span class="col-mod">Mod</span>
        <span class="col-id">ID</span>
        <span class="col-actions">Order</span>
      </div>

      <div class="list-body">
        {#each filtered as entry, i}
          {@const realIndex = entries.indexOf(entry)}
          <div
            class="list-row"
            class:row-disabled={!entry.enabled}
            class:row-dragging={dragIndex === realIndex}
            class:row-drag-over={dragOverIndex === realIndex &&
              dragIndex !== realIndex}
            draggable={!search}
            ondragstart={(e) =>
              search ? e.preventDefault() : handleDragStart(e, realIndex)}
            ondragover={(e) =>
              search ? null : handleDragOver(e, realIndex)}
            ondragend={handleDragEnd}
            ondrop={(e) => (search ? null : handleDrop(e, realIndex))}
            role="listitem"
            style="animation: glass-fade-in var(--duration-slow) var(--ease) both; animation-delay: {Math.min(
              i,
              15,
            ) * 30}ms"
          >
            <span class="col-drag">
              <svg
                class="drag-handle"
                width="12"
                height="12"
                viewBox="0 0 12 12"
                fill="currentColor"
                aria-label="Drag to reorder {entry.display_name}"
              >
                <circle cx="4" cy="2.5" r="1" />
                <circle cx="8" cy="2.5" r="1" />
                <circle cx="4" cy="6" r="1" />
                <circle cx="8" cy="6" r="1" />
                <circle cx="4" cy="9.5" r="1" />
                <circle cx="8" cy="9.5" r="1" />
              </svg>
            </span>
            <span class="col-index">
              <span class="index-num">{realIndex}</span>
            </span>
            <span class="col-toggle">
              <button
                class="toggle-btn"
                class:toggle-on={entry.enabled}
                onclick={() => handleToggle(entry)}
                disabled={saving}
                title={entry.enabled ? "Disable mod" : "Enable mod"}
                aria-label={entry.enabled
                  ? `Disable ${entry.display_name}`
                  : `Enable ${entry.display_name}`}
                aria-pressed={entry.enabled}
                role="switch"
              >
                <span class="toggle-thumb"></span>
              </button>
            </span>
            <span class="col-mod">
              <span class="mod-name">{entry.display_name}</span>
            </span>
            <span class="col-id">
              <span class="mod-id">{entry.id}</span>
            </span>
            <span class="col-actions">
              <button
                class="move-btn"
                onclick={() => handleMove(entry.id, "up")}
                disabled={i === 0 || saving}
                title="Move up"
              >
                <svg
                  width="12"
                  height="12"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2.5"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                >
                  <polyline points="18 15 12 9 6 15" />
                </svg>
              </button>
              <button
                class="move-btn"
                onclick={() => handleMove(entry.id, "down")}
                disabled={i === filtered.length - 1 || saving}
                title="Move down"
              >
                <svg
                  width="12"
                  height="12"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2.5"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                >
                  <polyline points="6 9 12 15 18 9" />
                </svg>
              </button>
            </span>
          </div>
        {/each}
      </div>
    </div>
  {/if}
</div>

<style>
  .lo-panel {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
  }

  /* ---- Header ---- */

  .page-header {
    display: flex;
    justify-content: space-between;
    align-items: flex-end;
    gap: var(--space-4);
  }

  .header-title h2 {
    font-size: 22px;
    font-weight: 700;
    letter-spacing: -0.02em;
    color: var(--text-primary);
  }

  .header-meta {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-shrink: 0;
  }

  .meta-chip {
    display: flex;
    align-items: baseline;
    gap: 5px;
  }

  .meta-value {
    font-size: 16px;
    font-weight: 700;
    font-family: var(--font-mono);
    color: var(--text-primary);
    letter-spacing: -0.02em;
  }

  .meta-label {
    font-size: 11px;
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-tertiary);
  }

  .meta-divider {
    width: 1px;
    height: 16px;
    background: var(--separator-opaque);
  }

  /* ---- Toolbar ---- */

  .toolbar {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-3);
    flex-wrap: wrap;
  }

  .toolbar-right {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .search-box {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    padding: 4px 10px;
    min-width: 160px;
  }

  .search-box:focus-within {
    border-color: var(--system-accent);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--system-accent) 20%, transparent);
  }

  .search-input {
    border: none;
    background: transparent;
    outline: none;
    color: var(--text-primary);
    font-size: 12px;
    width: 100%;
    min-width: 0;
  }

  .search-input::placeholder {
    color: var(--text-tertiary);
  }

  .search-clear {
    display: flex;
    align-items: center;
    justify-content: center;
    background: none;
    border: none;
    padding: 2px;
    cursor: pointer;
    color: var(--text-tertiary);
    border-radius: 50%;
  }

  .search-clear:hover {
    color: var(--text-primary);
    background: var(--surface-hover);
  }

  .filter-count {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-tertiary);
    font-family: var(--font-mono);
    white-space: nowrap;
  }

  .saved-message {
    font-size: 12px;
    font-weight: 500;
    color: var(--green);
    animation: fade-in 0.2s ease;
  }

  @keyframes fade-in {
    from {
      opacity: 0;
      transform: translateY(-4px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  /* ---- List container ---- */

  .list-container {
    background: var(--surface);
    border-radius: var(--radius-lg);
    overflow: hidden;
    box-shadow: var(--glass-refraction), var(--glass-edge-shadow);
  }

  .list-header {
    display: grid;
    grid-template-columns: 24px 40px 44px 1fr 1fr 60px;
    padding: var(--space-2) var(--space-4);
    background: var(--bg-secondary);
    border-bottom: 1px solid var(--separator);
    font-size: 11px;
    font-weight: 500;
    color: var(--text-secondary);
    align-items: center;
  }

  .list-body {
    max-height: calc(100vh - 300px);
    overflow-y: auto;
  }

  .list-row {
    display: grid;
    grid-template-columns: 24px 40px 44px 1fr 1fr 60px;
    padding: var(--space-2) var(--space-4);
    align-items: center;
    transition:
      background var(--duration-fast) var(--ease),
      transform var(--duration-fast) var(--ease),
      box-shadow var(--duration-fast) var(--ease);
    cursor: grab;
  }

  .list-row:active {
    cursor: grabbing;
  }

  .list-row.row-dragging {
    opacity: 0.4;
  }

  .list-row.row-drag-over {
    border-top: 2px solid var(--accent);
  }

  .list-row:nth-child(even) {
    background: var(--surface-subtle);
  }

  .list-row:hover {
    background: var(--surface-hover);
    transform: translateY(-1px);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.15);
  }

  .list-row.row-disabled {
    opacity: 0.5;
  }

  .list-row.row-disabled:hover {
    opacity: 0.6;
  }

  /* ---- Toggle ---- */

  .toggle-btn {
    position: relative;
    width: 32px;
    height: 18px;
    background: var(--separator-opaque);
    border: none;
    border-radius: 9px;
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease);
    padding: 0;
  }

  .toggle-btn.toggle-on {
    background: var(--green);
  }

  .toggle-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .toggle-thumb {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 14px;
    height: 14px;
    background: white;
    border-radius: 50%;
    transition:
      transform var(--duration-fast)
        var(--ease-spring, cubic-bezier(0.34, 1.56, 0.64, 1)),
      width var(--duration-fast) var(--ease);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.2);
  }

  .toggle-on .toggle-thumb {
    transform: translateX(14px);
  }

  /* ---- Columns ---- */

  .col-drag {
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .drag-handle {
    color: var(--text-quaternary);
    cursor: grab;
  }

  .col-index {
    display: flex;
    align-items: center;
  }

  .index-num {
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 500;
    color: var(--text-quaternary);
  }

  .col-toggle {
    display: flex;
    align-items: center;
  }

  .col-mod,
  .col-id {
    min-width: 0;
    overflow: hidden;
  }

  .mod-name {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    display: block;
  }

  .mod-id {
    font-family: var(--font-mono);
    font-size: 11.5px;
    color: var(--text-tertiary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    display: block;
  }

  .col-actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 2px;
  }

  .move-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    background: none;
    border: none;
    border-radius: var(--radius-sm);
    color: var(--text-tertiary);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
    padding: 0;
  }

  .move-btn:hover:not(:disabled) {
    background: var(--surface-active);
    color: var(--text-primary);
  }

  .move-btn:disabled {
    opacity: 0.2;
    cursor: not-allowed;
  }

  /* ---- Empty / loading ---- */

  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-12) var(--space-8);
    background: var(--surface);
    border-radius: var(--radius-lg);
    box-shadow: var(--glass-refraction), var(--glass-edge-shadow);
    text-align: center;
  }

  .empty-icon {
    color: var(--text-quaternary);
    margin-bottom: var(--space-1);
  }

  .empty-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .empty-description {
    font-size: 13px;
    color: var(--text-tertiary);
    max-width: 340px;
    line-height: 1.5;
  }

  .loading-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-4);
    padding: var(--space-12);
  }

  .loading-text {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-tertiary);
  }

  .spinner {
    width: 28px;
    height: 28px;
    border: 2.5px solid var(--separator-opaque);
    border-top-color: var(--system-accent);
    border-radius: 50%;
    animation: spin 0.75s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
