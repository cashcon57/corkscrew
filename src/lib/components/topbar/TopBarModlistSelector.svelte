<script lang="ts">
  import { selectedGame, activeCollection, collectionList, showError, showSuccess } from "$lib/stores";
  import { switchCollection, deleteCollection, listInstalledCollections } from "$lib/api";
  import { get } from "svelte/store";
  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";

  interface Props {
    onNavigate: (page: string) => void;
    isOpen: boolean;
    onToggle: () => void;
    onClose: () => void;
  }

  let {
    onNavigate,
    isOpen,
    onToggle,
    onClose,
  }: Props = $props();

  let switching = $state<string | null>(null);
  let confirmDeleteName = $state<string | null>(null);
  let deleting = $state(false);

  async function handleSwitch(name: string) {
    const game = get(selectedGame);
    if (!game || switching) return;
    switching = name;
    try {
      const result = await switchCollection(game.game_id, game.bottle_name, name);
      const col = get(collectionList).find(c => c.name === name) ?? null;
      activeCollection.set(col);
      showSuccess(`Switched to "${name}" — ${result.deployed_count} files deployed`);
      onClose();
    } catch (e: unknown) {
      showError(`Failed to switch modlist: ${e}`);
    } finally {
      switching = null;
    }
  }

  async function handleConfirmDelete() {
    const name = confirmDeleteName;
    if (!name) return;
    const game = get(selectedGame);
    if (!game) return;
    deleting = true;
    try {
      await deleteCollection(game.game_id, game.bottle_name, name, false);
      // Reload collection list
      const collections = await listInstalledCollections(game.game_id, game.bottle_name);
      collectionList.set(collections);
      // Clear active if deleted
      if (get(activeCollection)?.name === name) {
        activeCollection.set(null);
      }
      showSuccess(`Deleted modlist "${name}"`);
    } catch (e: unknown) {
      showError(`Failed to delete modlist: ${e}`);
    } finally {
      deleting = false;
      confirmDeleteName = null;
    }
  }
</script>

<div class="topbar-selector-wrap">
  <button
    class="topbar-selector"
    onclick={(e) => { e.stopPropagation(); onToggle(); }}
    title={$activeCollection?.name ?? "No Modlist"}
  >
    <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" opacity="0.5">
      <path d="M2 4h12M2 8h12M2 12h8" />
    </svg>
    <span class="topbar-selector-label" class:placeholder={!$activeCollection}>
      {$activeCollection?.name ?? "No Modlist"}
    </span>
    <svg class="topbar-chevron" class:open={isOpen} width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
      <path d="M3 4l2 2 2-2" />
    </svg>
  </button>

  {#if isOpen}
    <div class="topbar-dropdown" onclick={(e) => e.stopPropagation()}>
      {#if $collectionList.length > 0}
        {#each $collectionList as col}
          <div class="dropdown-item-row">
            <button
              class="dropdown-item"
              class:active={$activeCollection?.name === col.name}
              disabled={switching === col.name}
              onclick={() => handleSwitch(col.name)}
            >
              <div class="dropdown-item-text">
                <span class="dropdown-item-name">
                  {#if switching === col.name}
                    <span class="spinner spinner-xs"></span>
                  {/if}
                  {col.name}
                </span>
                <span class="dropdown-item-sub">{col.mod_count} mods</span>
              </div>
              {#if $activeCollection?.name === col.name}
                <svg class="active-check" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                  <polyline points="20 6 9 17 4 12" />
                </svg>
              {/if}
            </button>
            <button
              class="dropdown-delete-btn"
              title="Delete modlist"
              onclick={(e) => { e.stopPropagation(); confirmDeleteName = col.name; }}
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="3 6 5 6 21 6" /><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
              </svg>
            </button>
          </div>
        {/each}
      {:else}
        <div class="dropdown-empty">No modlists installed</div>
      {/if}
      <div class="dropdown-footer">
        <button class="dropdown-action" onclick={() => { onNavigate("discover"); onClose(); }}>
          <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <line x1="8" y1="3" x2="8" y2="13" /><line x1="3" y1="8" x2="13" y2="8" />
          </svg>
          Add Modlist
        </button>
      </div>
    </div>
  {/if}
</div>

<ConfirmDialog
  open={confirmDeleteName !== null}
  title="Delete Modlist"
  message={`Are you sure you want to delete "${confirmDeleteName}"? This will uninstall all mods in this modlist.`}
  confirmLabel={deleting ? "Deleting..." : "Delete"}
  confirmDanger={true}
  onConfirm={handleConfirmDelete}
  onCancel={() => confirmDeleteName = null}
/>

<style>
  .topbar-selector-wrap {
    position: relative;
    display: flex;
    align-items: center;
    -webkit-app-region: no-drag;
  }

  .topbar-selector {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    border-radius: 100px;
    font-size: 13px;
    color: var(--text-primary);
    background: none;
    border: none;
    cursor: pointer;
    -webkit-app-region: no-drag;
    transition: background 0.15s ease;
    max-width: 220px;
  }

  .topbar-selector:hover {
    background: rgba(255, 255, 255, 0.06);
  }

  .topbar-selector-label {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-weight: 500;
  }

  .topbar-selector-label.placeholder {
    color: var(--text-tertiary);
    font-weight: 400;
  }

  .topbar-chevron {
    flex-shrink: 0;
    opacity: 0.4;
    transition: transform 0.15s ease;
  }

  .topbar-chevron.open {
    transform: rotate(180deg);
  }

  .topbar-dropdown {
    position: absolute;
    top: calc(100% + 6px);
    left: 0;
    min-width: 240px;
    max-width: 320px;
    background: var(--bg-elevated);
    backdrop-filter: var(--glass-blur);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: var(--radius-lg);
    padding: 4px;
    z-index: 100;
    box-shadow:
      var(--glass-refraction),
      var(--glass-edge-shadow),
      0 8px 32px rgba(0, 0, 0, 0.3),
      0 1px 4px rgba(0, 0, 0, 0.15);
    animation: dropdownIn 0.2s var(--ease-out);
    max-height: 320px;
    overflow-y: auto;
  }

  .dropdown-item-row {
    display: flex;
    align-items: center;
  }

  .dropdown-item {
    display: flex;
    align-items: center;
    gap: 10px;
    flex: 1;
    padding: 7px 10px;
    border-radius: calc(var(--radius) - 2px);
    background: none;
    border: none;
    color: var(--text-primary);
    cursor: pointer;
    font-size: 13px;
    text-align: left;
    transition: background 0.1s ease;
    min-width: 0;
  }

  .dropdown-item:hover {
    background: var(--surface-hover);
  }

  .dropdown-item.active {
    background: var(--accent-subtle);
    color: var(--accent);
  }

  .dropdown-item:disabled {
    opacity: 0.6;
    cursor: default;
  }

  .dropdown-item-text {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
  }

  .dropdown-item-name {
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .dropdown-item-sub {
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .active-check {
    flex-shrink: 0;
    color: var(--accent);
  }

  .dropdown-delete-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    border-radius: var(--radius);
    background: none;
    border: none;
    color: var(--text-quaternary);
    cursor: pointer;
    opacity: 0;
    transition: opacity 0.1s ease, background 0.1s ease, color 0.1s ease;
    flex-shrink: 0;
  }

  .dropdown-item-row:hover .dropdown-delete-btn {
    opacity: 1;
  }

  .dropdown-delete-btn:hover {
    background: rgba(255, 59, 48, 0.15);
    color: var(--red);
  }

  .dropdown-empty {
    padding: 12px 10px;
    font-size: 12px;
    color: var(--text-tertiary);
    text-align: center;
  }

  .dropdown-footer {
    border-top: 1px solid var(--separator);
    margin-top: 4px;
    padding-top: 4px;
  }

  .dropdown-action {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 7px 10px;
    border-radius: calc(var(--radius) - 2px);
    background: none;
    border: none;
    color: var(--text-secondary);
    cursor: pointer;
    font-size: 12px;
    text-align: left;
    transition: background 0.1s ease, color 0.1s ease;
  }

  .dropdown-action:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  @keyframes dropdownIn {
    from { opacity: 0; transform: translateY(-4px); }
    to { opacity: 1; transform: translateY(0); }
  }
</style>
