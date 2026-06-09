<script lang="ts">
  import { onMount } from "svelte";
  import { selectedGame } from "$lib/stores";
  import {
    getLoadOrderKind,
    getFileBasedLoadOrder,
    setFileBasedLoadOrder,
  } from "$lib/api";
  import GameIcon from "$lib/components/GameIcon.svelte";
  import { wineCtx } from "$lib/types";
  import type {
    DetectedGame,
    LoadOrderEntry,
    LoadOrderKindResponse,
  } from "$lib/types";

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  let kind = $state<LoadOrderKindResponse["kind"] | null>(null);
  let entries = $state<LoadOrderEntry[]>([]);
  let original = $state<LoadOrderEntry[]>([]);
  let loading = $state(false);
  let saving = $state(false);
  let error = $state<string | null>(null);
  let savedFlash = $state(false);

  // Drag state
  let draggingIndex = $state<number | null>(null);
  let dragOverIndex = $state<number | null>(null);

  // ---------------------------------------------------------------------------
  // Derived
  // ---------------------------------------------------------------------------

  /** Native games always use an empty bottle name sentinel. */
  function nativeBottleName(g: DetectedGame): string {
    return wineCtx(g)?.bottle_name ?? "";
  }

  function entriesEqual(a: LoadOrderEntry[], b: LoadOrderEntry[]): boolean {
    if (a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) {
      if (a[i].id !== b[i].id || a[i].enabled !== b[i].enabled) return false;
    }
    return true;
  }

  let isDirty = $derived(!entriesEqual(entries, original));
  let enabledCount = $derived(entries.filter((e) => e.enabled).length);

  // ---------------------------------------------------------------------------
  // Data loading
  // ---------------------------------------------------------------------------

  async function loadOrder() {
    const g = $selectedGame;
    if (!g) {
      kind = null;
      entries = [];
      original = [];
      return;
    }
    loading = true;
    error = null;
    try {
      const k = await getLoadOrderKind(g.game_id, nativeBottleName(g));
      kind = k.kind;
      if (k.kind === "file_based") {
        const list = await getFileBasedLoadOrder(g.game_id, nativeBottleName(g));
        entries = list;
        original = list.map((e) => ({ ...e }));
      } else {
        entries = [];
        original = [];
      }
    } catch (e) {
      console.error("loadOrder failed:", e);
      error = String(e);
      kind = null;
      entries = [];
      original = [];
    } finally {
      loading = false;
    }
  }

  onMount(loadOrder);

  $effect(() => {
    const _ = $selectedGame?.game_id;
    loadOrder();
  });

  // ---------------------------------------------------------------------------
  // Actions
  // ---------------------------------------------------------------------------

  async function handleSave() {
    const g = $selectedGame;
    if (!g) return;
    saving = true;
    error = null;
    try {
      await setFileBasedLoadOrder(g.game_id, nativeBottleName(g), entries);
      original = entries.map((e) => ({ ...e }));
      savedFlash = true;
      setTimeout(() => (savedFlash = false), 1500);
    } catch (e) {
      console.error("setFileBasedLoadOrder failed:", e);
      error = String(e);
    } finally {
      saving = false;
    }
  }

  async function handleReset() {
    await loadOrder();
  }

  function toggleEnabled(index: number) {
    const next = entries.map((e, i) =>
      i === index ? { ...e, enabled: !e.enabled } : e
    );
    entries = next;
  }

  // ---------------------------------------------------------------------------
  // Drag reorder (HTML5 DnD)
  // ---------------------------------------------------------------------------

  function onDragStart(e: DragEvent, index: number) {
    draggingIndex = index;
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = "move";
      // Required for Firefox to start a drag.
      e.dataTransfer.setData("text/plain", String(index));
    }
  }

  function onDragOver(e: DragEvent, index: number) {
    if (draggingIndex === null) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
    if (dragOverIndex !== index) dragOverIndex = index;
  }

  function onDragLeave(_e: DragEvent, index: number) {
    if (dragOverIndex === index) dragOverIndex = null;
  }

  function onDrop(e: DragEvent, index: number) {
    e.preventDefault();
    const from = draggingIndex;
    draggingIndex = null;
    dragOverIndex = null;
    if (from === null || from === index) return;
    const next = entries.slice();
    const [moved] = next.splice(from, 1);
    next.splice(index, 0, moved);
    entries = next;
  }

  function onDragEnd() {
    draggingIndex = null;
    dragOverIndex = null;
  }

  function moveUp(index: number) {
    if (index <= 0) return;
    const next = entries.slice();
    [next[index - 1], next[index]] = [next[index], next[index - 1]];
    entries = next;
  }

  function moveDown(index: number) {
    if (index >= entries.length - 1) return;
    const next = entries.slice();
    [next[index], next[index + 1]] = [next[index + 1], next[index]];
    entries = next;
  }
</script>

<div class="lo-page">
  {#if !$selectedGame}
    <div class="empty-state">
      <div class="empty-icon">
        <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <rect x="2" y="3" width="20" height="14" rx="2" ry="2" />
          <line x1="8" y1="21" x2="16" y2="21" />
          <line x1="12" y1="17" x2="12" y2="21" />
        </svg>
      </div>
      <h3 class="empty-title">No game selected</h3>
      <p class="empty-description">Pick a native game from the dropdown in the top bar.</p>
    </div>
  {:else}
    <!-- Banner -->
    <div class="game-banner">
      <div class="game-banner-icon">
        <GameIcon gameId={$selectedGame.game_id} steamAppId={$selectedGame.steam_app_id} size={36} />
      </div>
      <div class="game-banner-info">
        <h2 class="game-banner-title">{$selectedGame.display_name}</h2>
        <div class="game-banner-meta">
          <span class="meta-native-badge">Native</span>
          {#if kind === "file_based" && entries.length > 0}
            <span class="meta-separator">&middot;</span>
            <span class="meta-mods">{enabledCount}/{entries.length} in load order</span>
          {/if}
        </div>
      </div>
      <div class="game-banner-actions">
        {#if kind === "file_based"}
          <button
            class="btn btn-ghost"
            onclick={handleReset}
            disabled={loading || saving}
            title="Re-read load order from disk"
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <polyline points="23 4 23 10 17 10" />
              <path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10" />
            </svg>
            Reset
          </button>
          <button
            class="btn btn-primary"
            onclick={handleSave}
            disabled={!isDirty || saving || loading}
            title={isDirty ? "Save load order" : "No unsaved changes"}
          >
            {#if saving}
              <span class="spinner"></span>
              Saving…
            {:else if savedFlash}
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="20 6 9 17 4 12" />
              </svg>
              Saved
            {:else}
              Save
            {/if}
          </button>
        {/if}
      </div>
    </div>

    {#if loading}
      <div class="empty-state">
        <div class="empty-icon"><span class="spinner"></span></div>
        <h3 class="empty-title">Loading load order…</h3>
      </div>
    {:else if error}
      <div class="empty-state">
        <div class="empty-icon">
          <svg width="36" height="36" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" style="color: var(--red)">
            <circle cx="12" cy="12" r="10" />
            <line x1="15" y1="9" x2="9" y2="15" />
            <line x1="9" y1="9" x2="15" y2="15" />
          </svg>
        </div>
        <h3 class="empty-title">Failed to load load order</h3>
        <p class="empty-description">{error}</p>
        <button class="btn btn-secondary btn-sm" onclick={loadOrder}>Retry</button>
      </div>
    {:else if kind === "none" || kind === "plugins"}
      <!-- "plugins" kind means Bethesda-style, which the native UI doesn't
           render — the native mode targets non-Bethesda games. Either way,
           there's nothing for the user to do on this page. -->
      <div class="empty-state">
        <div class="empty-icon">
          <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="10" />
            <line x1="12" y1="8" x2="12" y2="12" />
            <line x1="12" y1="16" x2="12.01" y2="16" />
          </svg>
        </div>
        <h3 class="empty-title">Load order not applicable</h3>
        <p class="empty-description">
          {$selectedGame.display_name} doesn't expose a user-editable load order. Mods are applied in install order or by mod authority, not by a sortable list.
        </p>
      </div>
    {:else if entries.length === 0}
      <div class="empty-state">
        <div class="empty-icon">
          <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z" />
            <polyline points="3.27 6.96 12 12.01 20.73 6.96" />
            <line x1="12" y1="22.08" x2="12" y2="12" />
          </svg>
        </div>
        <h3 class="empty-title">Load order is empty</h3>
        <p class="empty-description">
          Install a mod for {$selectedGame.display_name} and it will appear here.
        </p>
      </div>
    {:else}
      <!-- Drag-reorderable list -->
      <div class="hint-bar">
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="10" />
          <line x1="12" y1="16" x2="12" y2="12" />
          <line x1="12" y1="8" x2="12.01" y2="8" />
        </svg>
        Drag rows to reorder. Save to write to disk; Reset re-reads from disk.
      </div>

      <div class="lo-list-container">
        <div class="lo-list">
          <div class="lo-header">
            <span class="col-handle"></span>
            <span class="col-num">#</span>
            <span class="col-toggle">On</span>
            <span class="col-name">Mod</span>
            <span class="col-id">ID</span>
            <span class="col-move">Move</span>
          </div>
          <div class="lo-body">
            {#each entries as entry, i (entry.id)}
              <div
                class="lo-row"
                class:row-dragging={draggingIndex === i}
                class:row-dragover={dragOverIndex === i && draggingIndex !== i}
                class:row-disabled={!entry.enabled}
                draggable="true"
                ondragstart={(e) => onDragStart(e, i)}
                ondragover={(e) => onDragOver(e, i)}
                ondragleave={(e) => onDragLeave(e, i)}
                ondrop={(e) => onDrop(e, i)}
                ondragend={onDragEnd}
                role="listitem"
              >
                <span class="col-handle" title="Drag to reorder">
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <circle cx="9" cy="6" r="1" />
                    <circle cx="9" cy="12" r="1" />
                    <circle cx="9" cy="18" r="1" />
                    <circle cx="15" cy="6" r="1" />
                    <circle cx="15" cy="12" r="1" />
                    <circle cx="15" cy="18" r="1" />
                  </svg>
                </span>
                <span class="col-num">{i + 1}</span>
                <span class="col-toggle">
                  <button
                    class="toggle-switch"
                    class:toggle-on={entry.enabled}
                    onclick={() => toggleEnabled(i)}
                    title={entry.enabled ? "Disable" : "Enable"}
                    aria-label="{entry.enabled ? 'Disable' : 'Enable'} {entry.display_name}"
                    aria-pressed={entry.enabled}
                    role="switch"
                  >
                    <span class="toggle-track">
                      <span class="toggle-thumb"></span>
                    </span>
                  </button>
                </span>
                <span class="col-name">
                  <span class="mod-name">{entry.display_name}</span>
                </span>
                <span class="col-id">
                  <code>{entry.id}</code>
                </span>
                <span class="col-move">
                  <button
                    class="move-btn"
                    onclick={() => moveUp(i)}
                    disabled={i === 0}
                    title="Move up"
                    aria-label="Move {entry.display_name} up"
                  >
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                      <polyline points="18 15 12 9 6 15" />
                    </svg>
                  </button>
                  <button
                    class="move-btn"
                    onclick={() => moveDown(i)}
                    disabled={i === entries.length - 1}
                    title="Move down"
                    aria-label="Move {entry.display_name} down"
                  >
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                      <polyline points="6 9 12 15 18 9" />
                    </svg>
                  </button>
                </span>
              </div>
            {/each}
          </div>
        </div>
      </div>

      <div class="status-footer">
        <span>{entries.length} entr{entries.length === 1 ? "y" : "ies"}</span>
        <span class="meta-separator">&middot;</span>
        <span>{enabledCount} enabled</span>
        {#if isDirty}
          <span class="meta-separator">&middot;</span>
          <span class="footer-dirty">Unsaved changes</span>
        {/if}
      </div>
    {/if}
  {/if}
</div>

<style>
  .lo-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: var(--space-4) var(--space-5);
    gap: var(--space-3);
    overflow: hidden;
    position: relative;
  }

  @media (max-width: 800px) {
    .lo-page {
      padding: var(--space-3) var(--space-3);
      gap: var(--space-2);
    }
  }

  /* Banner — mirrors .game-banner in /native/mods */
  .game-banner {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    box-shadow: var(--glass-refraction), var(--glass-edge-shadow);
    flex-shrink: 0;
  }

  .game-banner-icon {
    flex-shrink: 0;
    color: var(--text-primary);
    display: flex;
    align-items: center;
    justify-content: center;
    width: 40px;
    height: 40px;
  }

  .game-banner-info {
    flex: 1;
    min-width: 0;
  }

  .game-banner-title {
    font-size: 16px;
    font-weight: 700;
    letter-spacing: -0.02em;
    color: var(--text-primary);
    line-height: 1.2;
  }

  .game-banner-meta {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: 2px;
    font-size: 13px;
  }

  .meta-native-badge {
    background: var(--green-subtle, rgba(48, 209, 88, 0.12));
    color: var(--green, #30d158);
    border: 1px solid rgba(48, 209, 88, 0.25);
    padding: 1px 7px;
    border-radius: 100px;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .meta-separator {
    color: var(--text-quaternary);
  }

  .meta-mods {
    color: var(--text-secondary);
    font-weight: 500;
  }

  .game-banner-actions {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-2);
    flex-shrink: 0;
  }

  /* Buttons */
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    border-radius: var(--radius-sm);
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
    transition:
      background var(--duration-fast) var(--ease),
      color var(--duration-fast) var(--ease),
      box-shadow var(--duration-fast) var(--ease),
      opacity var(--duration-fast) var(--ease);
    border: 1px solid transparent;
    cursor: pointer;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-primary {
    background: var(--system-accent, #007aff);
    color: white;
  }

  .btn-primary:hover:not(:disabled) {
    filter: brightness(1.08);
  }

  .btn-secondary {
    background: var(--surface);
    color: var(--text-primary);
    border-color: var(--separator);
  }

  .btn-secondary:hover:not(:disabled) {
    background: var(--surface-hover);
    border-color: var(--separator-opaque);
  }

  .btn-ghost {
    background: transparent;
    color: var(--text-secondary);
  }

  .btn-ghost:hover:not(:disabled) {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .btn-sm {
    padding: var(--space-1) var(--space-3);
    font-size: 12px;
    font-weight: 500;
    border-radius: var(--radius-sm);
  }

  .spinner {
    display: inline-block;
    width: 12px;
    height: 12px;
    border: 2px solid currentColor;
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  /* Hint bar */
  .hint-bar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: var(--surface-subtle, var(--surface));
    border: 1px solid var(--separator);
    border-radius: var(--radius-sm);
    font-size: 12px;
    color: var(--text-tertiary);
    flex-shrink: 0;
  }

  /* Empty state — translucent like /native/mods */
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    flex: 1;
    padding: var(--space-12) var(--space-6);
    background: var(--surface-glass);
    border: 1px solid var(--separator);
    border-radius: var(--radius-lg);
    box-shadow: var(--glass-refraction), var(--glass-edge-shadow);
    backdrop-filter: var(--glass-blur-light);
    text-align: center;
    gap: var(--space-3);
  }

  .empty-icon {
    color: var(--text-quaternary);
    margin-bottom: var(--space-2);
  }

  .empty-title {
    font-size: 17px;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .empty-description {
    font-size: 13px;
    color: var(--text-tertiary);
    max-width: 380px;
    line-height: 1.5;
  }

  /* Load order list */
  .lo-list-container {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border-radius: var(--radius-lg);
    background: var(--bg-primary);
    box-shadow: var(--glass-refraction), var(--glass-edge-shadow);
    min-height: 200px;
  }

  .lo-list {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    --lo-cols: 28px 36px 48px minmax(0, 2fr) minmax(0, 1.4fr) 72px;
  }

  @media (max-width: 900px) {
    .lo-list {
      --lo-cols: 28px 32px 44px minmax(0, 1fr) 0px 64px !important;
    }
    .col-id {
      display: none;
    }
  }

  .lo-header {
    display: grid;
    grid-template-columns: var(--lo-cols);
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: var(--bg-secondary);
    border-bottom: 1px solid var(--separator);
    flex-shrink: 0;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .lo-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }

  .lo-row {
    display: grid;
    grid-template-columns: var(--lo-cols);
    gap: var(--space-2);
    padding: 0 var(--space-3);
    align-items: center;
    font-size: 13px;
    height: 40px;
    box-sizing: border-box;
    border-bottom: 1px solid var(--separator);
    cursor: grab;
    transition:
      background var(--duration-fast) var(--ease),
      opacity var(--duration-fast) var(--ease),
      transform var(--duration-fast) var(--ease),
      box-shadow var(--duration-fast) var(--ease);
  }

  .lo-row:nth-child(even) {
    background: var(--surface-subtle);
  }

  .lo-row:hover {
    background: var(--surface-hover);
  }

  .lo-row.row-dragging {
    opacity: 0.5;
    cursor: grabbing;
  }

  .lo-row.row-dragover {
    box-shadow: inset 0 2px 0 var(--system-accent);
    background: color-mix(in srgb, var(--system-accent) 10%, transparent);
  }

  .lo-row.row-disabled {
    opacity: 0.5;
  }

  .lo-row.row-disabled:hover {
    opacity: 0.7;
  }

  .col-handle {
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-quaternary);
  }

  .col-num {
    font-variant-numeric: tabular-nums;
    color: var(--text-tertiary);
    font-size: 12px;
    font-family: var(--font-mono);
  }

  .col-toggle {
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .col-name {
    min-width: 0;
    display: flex;
    align-items: center;
  }

  .mod-name {
    font-weight: 500;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
    flex: 1;
  }

  .col-id {
    min-width: 0;
    overflow: hidden;
  }

  .col-id code {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-tertiary);
    background: var(--surface);
    padding: 1px 5px;
    border-radius: 4px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    display: inline-block;
    max-width: 100%;
  }

  .col-move {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 2px;
  }

  .move-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-tertiary);
    border: 1px solid transparent;
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
  }

  .move-btn:hover:not(:disabled) {
    background: var(--surface-hover);
    color: var(--text-primary);
    border-color: var(--separator);
  }

  .move-btn:disabled {
    opacity: 0.3;
    cursor: not-allowed;
  }

  /* Toggle switch — matches /native/mods */
  .toggle-switch {
    display: inline-flex;
    align-items: center;
    padding: 0;
    background: transparent;
    cursor: pointer;
    border: none;
  }

  .toggle-track {
    position: relative;
    width: 32px;
    height: 18px;
    border-radius: 9px;
    background: var(--bg-tertiary);
    transition:
      background var(--duration) var(--ease),
      box-shadow var(--duration) var(--ease);
  }

  .toggle-on .toggle-track {
    background: var(--green);
    box-shadow: 0 0 8px rgba(48, 209, 88, 0.25);
  }

  .toggle-thumb {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
    transition: transform var(--duration-fast) var(--ease-spring, cubic-bezier(0.34, 1.56, 0.64, 1));
  }

  .toggle-on .toggle-thumb {
    transform: translateX(14px);
  }

  /* Footer */
  .status-footer {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: 12px;
    color: var(--text-tertiary);
    padding: var(--space-1) var(--space-2);
    flex-shrink: 0;
    font-variant-numeric: tabular-nums;
  }

  .footer-dirty {
    color: var(--system-accent);
    font-weight: 600;
  }
</style>
