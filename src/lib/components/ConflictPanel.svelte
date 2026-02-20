<script lang="ts">
  import { getConflicts, setModPriority, redeployAllMods } from "$lib/api";
  import { selectedGame, showError, showSuccess } from "$lib/stores";
  import type { FileConflict, ConflictModInfo } from "$lib/types";

  // ---- Props ----
  interface Props {
    visible?: boolean;
    onclose?: () => void;
  }

  let { visible = true, onclose }: Props = $props();

  // ---- State ----
  let conflicts = $state<FileConflict[]>([]);
  let loading = $state(false);
  let expandedPaths = $state<Set<string>>(new Set());
  let draggingModId = $state<number | null>(null);
  let dragOverModId = $state<number | null>(null);
  let dragOverPath = $state<string | null>(null);
  let redeploying = $state(false);

  const game = $derived($selectedGame);

  const totalConflicts = $derived(conflicts.length);
  const involvedMods = $derived.by(() => {
    const modIds = new Set<number>();
    for (const c of conflicts) {
      for (const m of c.mods) {
        modIds.add(m.mod_id);
      }
    }
    return modIds.size;
  });

  $effect(() => {
    if (visible && game) {
      loadConflicts();
    }
  });

  async function loadConflicts() {
    if (!game) return;
    loading = true;
    try {
      conflicts = await getConflicts(game.game_id, game.bottle_name);
    } catch (e: unknown) {
      showError(`Failed to load conflicts: ${e}`);
    } finally {
      loading = false;
    }
  }

  function togglePath(path: string) {
    const next = new Set(expandedPaths);
    if (next.has(path)) {
      next.delete(path);
    } else {
      next.add(path);
    }
    expandedPaths = next;
  }

  function isExpanded(path: string): boolean {
    return expandedPaths.has(path);
  }

  // Drag-and-drop reordering within a conflict group
  function handleDragStart(e: DragEvent, modId: number, path: string) {
    draggingModId = modId;
    dragOverPath = path;
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = "move";
    }
  }

  function handleDragOver(e: DragEvent, modId: number) {
    e.preventDefault();
    dragOverModId = modId;
  }

  function handleDragLeave() {
    dragOverModId = null;
  }

  async function handleDrop(e: DragEvent, targetModId: number, conflict: FileConflict) {
    e.preventDefault();
    if (draggingModId === null || draggingModId === targetModId) {
      draggingModId = null;
      dragOverModId = null;
      dragOverPath = null;
      return;
    }

    // Find the target mod's priority and assign it to the dragged mod
    const targetMod = conflict.mods.find(m => m.mod_id === targetModId);
    if (!targetMod) return;

    try {
      await setModPriority(draggingModId, targetMod.priority);
      showSuccess("Priority updated");
      await loadConflicts();
    } catch (e: unknown) {
      showError(`Failed to update priority: ${e}`);
    } finally {
      draggingModId = null;
      dragOverModId = null;
      dragOverPath = null;
    }
  }

  function handleDragEnd() {
    draggingModId = null;
    dragOverModId = null;
    dragOverPath = null;
  }

  async function handleRedeploy() {
    if (!game || redeploying) return;
    redeploying = true;
    try {
      const result = await redeployAllMods(game.game_id, game.bottle_name);
      showSuccess(`Redeployed ${result.deployed_count} files`);
      await loadConflicts();
    } catch (e: unknown) {
      showError(`Redeploy failed: ${e}`);
    } finally {
      redeploying = false;
    }
  }

  function truncatePath(path: string, maxLen: number = 60): string {
    if (path.length <= maxLen) return path;
    const parts = path.split("/");
    if (parts.length <= 2) return "..." + path.slice(-maxLen);
    return parts[0] + "/.../" + parts.slice(-2).join("/");
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && onclose) {
      onclose();
    }
  }
</script>

{#if visible}
  <div class="conflict-panel" role="region" aria-label="File Conflicts" onkeydown={handleKeydown}>
    <!-- Header -->
    <div class="panel-header">
      <div class="panel-title-row">
        <h3 class="panel-title">File Conflicts</h3>
        <span class="conflict-count-badge">{totalConflicts}</span>
        {#if onclose}
          <button class="panel-close" onclick={onclose} aria-label="Close panel" type="button">
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
              <line x1="2" y1="2" x2="10" y2="10" />
              <line x1="10" y1="2" x2="2" y2="10" />
            </svg>
          </button>
        {/if}
      </div>
      {#if totalConflicts > 0}
        <p class="panel-summary">
          {totalConflicts} file{totalConflicts !== 1 ? "s" : ""} have conflicts across {involvedMods} mod{involvedMods !== 1 ? "s" : ""}
        </p>
      {/if}
    </div>

    <!-- Toolbar -->
    {#if totalConflicts > 0}
      <div class="panel-toolbar">
        <button
          class="btn btn-secondary btn-sm"
          onclick={handleRedeploy}
          disabled={redeploying}
          type="button"
        >
          {#if redeploying}
            <span class="spinner-sm"></span>
            Redeploying...
          {:else}
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M21 2v6h-6M3 12a9 9 0 0 1 15-6.7L21 8M3 22v-6h6M21 12a9 9 0 0 1-15 6.7L3 16" />
            </svg>
            Redeploy
          {/if}
        </button>
        <button
          class="btn btn-ghost btn-sm"
          onclick={loadConflicts}
          disabled={loading}
          type="button"
        >
          Refresh
        </button>
      </div>
    {/if}

    <!-- Content -->
    {#if loading}
      <div class="panel-loading">
        <div class="spinner"></div>
        <p class="loading-text">Loading conflicts...</p>
      </div>
    {:else if totalConflicts === 0}
      <div class="panel-empty">
        <div class="empty-icon">
          <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="10" />
            <path d="M9 12l2 2 4-4" />
          </svg>
        </div>
        <p class="empty-title">No file conflicts</p>
        <p class="empty-description">All mod files have unique deployment paths.</p>
      </div>
    {:else}
      <div class="conflict-list">
        {#each conflicts as conflict (conflict.relative_path)}
          <div class="conflict-group">
            <button
              class="conflict-row"
              onclick={() => togglePath(conflict.relative_path)}
              aria-expanded={isExpanded(conflict.relative_path)}
              type="button"
            >
              <svg
                class="row-chevron"
                class:row-chevron-open={isExpanded(conflict.relative_path)}
                width="10"
                height="10"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2.5"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <polyline points="6 9 12 15 18 9" />
              </svg>
              <span class="conflict-path" title={conflict.relative_path}>
                {truncatePath(conflict.relative_path)}
              </span>
              <span class="conflict-mod-count">
                {conflict.mods.length} mod{conflict.mods.length !== 1 ? "s" : ""}
              </span>
            </button>

            {#if isExpanded(conflict.relative_path)}
              <div class="conflict-detail">
                {#each conflict.mods.sort((a, b) => a.priority - b.priority) as mod (mod.mod_id)}
                  <div
                    class="mod-priority-row"
                    class:mod-winner={mod.mod_id === conflict.winner_mod_id}
                    class:mod-drag-over={dragOverModId === mod.mod_id && dragOverPath === conflict.relative_path}
                    draggable="true"
                    ondragstart={(e) => handleDragStart(e, mod.mod_id, conflict.relative_path)}
                    ondragover={(e) => handleDragOver(e, mod.mod_id)}
                    ondragleave={handleDragLeave}
                    ondrop={(e) => handleDrop(e, mod.mod_id, conflict)}
                    ondragend={handleDragEnd}
                    role="listitem"
                  >
                    <span class="drag-handle" aria-label="Drag to reorder">
                      <svg width="8" height="12" viewBox="0 0 8 12" fill="currentColor">
                        <circle cx="2" cy="2" r="1" />
                        <circle cx="6" cy="2" r="1" />
                        <circle cx="2" cy="6" r="1" />
                        <circle cx="6" cy="6" r="1" />
                        <circle cx="2" cy="10" r="1" />
                        <circle cx="6" cy="10" r="1" />
                      </svg>
                    </span>
                    <span class="mod-priority-num">{mod.priority}</span>
                    <span class="mod-priority-name">{mod.mod_name}</span>
                    {#if mod.mod_id === conflict.winner_mod_id}
                      <span class="winner-badge" title="This mod's file wins">
                        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                          <path d="M20 6L9 17l-5-5" />
                        </svg>
                      </span>
                    {/if}
                  </div>
                {/each}
              </div>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  </div>
{/if}

<style>
  /* ---- Panel ---- */

  .conflict-panel {
    display: flex;
    flex-direction: column;
    background: var(--bg-grouped-secondary);
    border-radius: var(--radius-lg);
    overflow: hidden;
    max-height: calc(100vh - 200px);
  }

  /* ---- Header ---- */

  .panel-header {
    padding: var(--space-4) var(--space-5);
    border-bottom: 1px solid var(--separator);
    flex-shrink: 0;
  }

  .panel-title-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .panel-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .conflict-count-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 20px;
    height: 20px;
    padding: 0 6px;
    border-radius: 100px;
    font-size: 11px;
    font-weight: 700;
    color: var(--red);
    background: var(--red-subtle);
    font-variant-numeric: tabular-nums;
  }

  .panel-close {
    margin-left: auto;
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    color: var(--text-tertiary);
    transition: all var(--duration-fast) var(--ease);
  }

  .panel-close:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .panel-summary {
    font-size: 12px;
    color: var(--text-tertiary);
    margin-top: var(--space-1);
  }

  /* ---- Toolbar ---- */

  .panel-toolbar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-5);
    border-bottom: 1px solid var(--separator);
    flex-shrink: 0;
  }

  .btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 12px;
    font-weight: 500;
    border: none;
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
    white-space: nowrap;
  }

  .btn-sm {
    padding: var(--space-1) var(--space-3);
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-secondary {
    background: var(--surface-hover);
    color: var(--text-secondary);
  }

  .btn-secondary:hover:not(:disabled) {
    background: var(--surface-active);
    color: var(--text-primary);
  }

  .btn-ghost {
    background: transparent;
    color: var(--text-tertiary);
  }

  .btn-ghost:hover:not(:disabled) {
    background: var(--surface-hover);
    color: var(--text-secondary);
  }

  /* ---- Loading / Empty ---- */

  .panel-loading {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-8);
  }

  .spinner {
    width: 24px;
    height: 24px;
    border: 2px solid var(--separator-opaque);
    border-top-color: var(--system-accent);
    border-radius: 50%;
    animation: spin 0.75s linear infinite;
  }

  .spinner-sm {
    display: inline-block;
    width: 12px;
    height: 12px;
    border: 2px solid var(--separator-opaque);
    border-top-color: var(--system-accent);
    border-radius: 50%;
    animation: spin 0.75s linear infinite;
    flex-shrink: 0;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .loading-text {
    font-size: 12px;
    color: var(--text-tertiary);
  }

  .panel-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-8);
    text-align: center;
  }

  .empty-icon {
    color: var(--text-quaternary);
    margin-bottom: var(--space-1);
  }

  .empty-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .empty-description {
    font-size: 12px;
    color: var(--text-tertiary);
  }

  /* ---- Conflict List ---- */

  .conflict-list {
    overflow-y: auto;
    flex: 1;
  }

  .conflict-group {
    border-bottom: 1px solid var(--separator);
  }

  .conflict-group:last-child {
    border-bottom: none;
  }

  .conflict-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: var(--space-2) var(--space-4);
    text-align: left;
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease);
  }

  .conflict-row:hover {
    background: var(--surface-hover);
  }

  .row-chevron {
    transition: transform var(--duration-fast) var(--ease);
    color: var(--text-tertiary);
    flex-shrink: 0;
  }

  .row-chevron-open {
    transform: rotate(0deg);
  }

  .row-chevron:not(.row-chevron-open) {
    transform: rotate(-90deg);
  }

  .conflict-path {
    flex: 1;
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-primary);
    letter-spacing: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  .conflict-mod-count {
    flex-shrink: 0;
    font-size: 11px;
    font-weight: 500;
    color: var(--text-tertiary);
    background: var(--surface);
    padding: 1px 6px;
    border-radius: 100px;
  }

  /* ---- Conflict Detail (expanded) ---- */

  .conflict-detail {
    padding: 0 var(--space-4) var(--space-2);
    padding-left: calc(var(--space-4) + 10px + var(--space-2));
  }

  .mod-priority-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
    transition: background var(--duration-fast) var(--ease);
    cursor: grab;
  }

  .mod-priority-row:active {
    cursor: grabbing;
  }

  .mod-priority-row:hover {
    background: var(--surface-hover);
  }

  .mod-priority-row.mod-drag-over {
    background: var(--system-accent-subtle);
    border: 1px dashed var(--system-accent);
  }

  .mod-priority-row.mod-winner {
    background: var(--green-subtle);
  }

  .drag-handle {
    color: var(--text-quaternary);
    cursor: grab;
    flex-shrink: 0;
    display: flex;
    align-items: center;
  }

  .drag-handle:active {
    cursor: grabbing;
  }

  .mod-priority-num {
    font-family: var(--font-mono);
    font-size: 10px;
    font-weight: 600;
    color: var(--text-tertiary);
    min-width: 18px;
    text-align: center;
    flex-shrink: 0;
    letter-spacing: 0;
  }

  .mod-priority-name {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-primary);
    flex: 1;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .winner-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--green);
    flex-shrink: 0;
  }
</style>
