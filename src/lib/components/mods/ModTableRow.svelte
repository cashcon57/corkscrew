<script lang="ts">
  import type { InstalledMod, ModUpdateInfo } from "$lib/types";

  interface Props {
    mod: InstalledMod;
    index: number;
    isSelected: boolean;
    isDisabled: boolean;
    isDragging: boolean;
    isDragOver: boolean;
    isDragAbove: boolean;
    isDragBelow: boolean;
    hasConflict: boolean;
    conflictTooltip: string;
    update: ModUpdateInfo | undefined;
    confirmingUninstall: boolean;
    togglingMod: boolean;
    overflowOpen: boolean;
    searchQuery: string;
    onClick: () => void;
    onToggle: () => void;
    onUninstall: () => void;
    onConfirmUninstall: () => void;
    onCancelUninstall: () => void;
    onOverflowToggle: () => void;
    onDragStart: (e: DragEvent) => void;
    onDragOver: (e: DragEvent) => void;
    onDragEnd: () => void;
    onDrop: (e: DragEvent) => void;
    onContextMenu?: (e: MouseEvent) => void;
  }

  let {
    mod, index, isSelected, isDisabled, isDragging,
    isDragOver, isDragAbove, isDragBelow, hasConflict,
    conflictTooltip, update, confirmingUninstall, togglingMod,
    overflowOpen, searchQuery, onClick, onToggle, onUninstall,
    onConfirmUninstall, onCancelUninstall, onOverflowToggle,
    onDragStart, onDragOver, onDragEnd, onDrop, onContextMenu,
  }: Props = $props();
</script>

<!-- Phase 2-3 will implement full row with hover actions, context menu, search highlighting, category chips -->
<div
  class="table-row"
  class:row-disabled={isDisabled}
  class:row-selected={isSelected}
  class:row-dragging={isDragging}
  class:row-drag-over={isDragOver}
  class:row-drag-above={isDragAbove}
  class:row-drag-below={isDragBelow}
  draggable="true"
  onclick={onClick}
  ondragstart={onDragStart}
  ondragover={onDragOver}
  ondragend={onDragEnd}
  ondrop={onDrop}
  oncontextmenu={onContextMenu}
>
  <span class="col-grip">
    <span class="drag-handle" title="Drag to reorder">
      <svg width="12" height="12" viewBox="0 0 12 12" fill="currentColor">
        <circle cx="4" cy="2.5" r="1" /><circle cx="8" cy="2.5" r="1" />
        <circle cx="4" cy="6" r="1" /><circle cx="8" cy="6" r="1" />
        <circle cx="4" cy="9.5" r="1" /><circle cx="8" cy="9.5" r="1" />
      </svg>
    </span>
  </span>

  <span class="col-toggle">
    <button
      class="toggle-switch"
      class:toggle-on={mod.enabled}
      class:toggle-busy={togglingMod}
      onclick={(e) => { e.stopPropagation(); onToggle(); }}
      title={mod.enabled ? "Disable mod" : "Enable mod"}
    >
      <span class="toggle-track"><span class="toggle-thumb"></span></span>
    </button>
  </span>

  <span class="col-name">
    <span class="mod-name">{mod.name}</span>
    {#if hasConflict}
      <span class="conflict-icon" title={conflictTooltip}>
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
          <line x1="12" y1="9" x2="12" y2="13" /><line x1="12" y1="17" x2="12.01" y2="17" />
        </svg>
      </span>
    {/if}
    {#if mod.nexus_mod_id}
      <span class="nexus-badge">Nexus</span>
    {/if}
    {#if mod.collection_name}
      <span class="collection-badge" title={mod.collection_name}>{mod.collection_name}</span>
    {/if}
    {#if mod.user_notes}
      <span class="notes-icon" title={mod.user_notes}>
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
          <polyline points="14 2 14 8 20 8" />
          <line x1="16" y1="13" x2="8" y2="13" /><line x1="16" y1="17" x2="8" y2="17" />
        </svg>
      </span>
    {/if}
  </span>

  <span class="col-version">
    <span class="version-text">{mod.version || "\u2014"}</span>
    {#if update}
      <span class="update-badge" title={`Update available: v${update.latest_version}`}>Update</span>
    {/if}
  </span>

  <span class="col-files">{mod.installed_files.length}</span>

  <span class="col-date">{new Date(mod.installed_at).toLocaleDateString()}</span>

  <span class="col-actions">
    {#if confirmingUninstall}
      <div class="confirm-actions">
        <button class="btn btn-danger btn-sm" onclick={(e) => { e.stopPropagation(); onUninstall(); }}>Yes</button>
        <button class="btn btn-ghost btn-sm" onclick={(e) => { e.stopPropagation(); onCancelUninstall(); }}>No</button>
      </div>
    {:else}
      <div class="mod-action-group">
        <button
          class="mod-uninstall-btn"
          onclick={(e) => { e.stopPropagation(); onConfirmUninstall(); }}
          title="Uninstall mod"
        >
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="3 6 5 6 21 6" />
            <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
          </svg>
        </button>
        <div class="mod-overflow-wrap">
          <button
            class="mod-overflow-btn"
            onclick={(e) => { e.stopPropagation(); onOverflowToggle(); }}
            title="More actions"
          >
            <svg width="13" height="13" viewBox="0 0 24 24" fill="currentColor">
              <circle cx="12" cy="5" r="2" /><circle cx="12" cy="12" r="2" /><circle cx="12" cy="19" r="2" />
            </svg>
          </button>
        </div>
      </div>
    {/if}
  </span>
</div>
