<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { selectedGame, showError, collectionUninstallStatus, collectionList, activeCollection, installedMods } from "$lib/stores";
  import type { CollectionUninstallStatus } from "$lib/stores";
  import type { UninstallProgressEvent, DetectedGame } from "$lib/types";
  import {
    deleteCollection,
    getCollectionDownloadSize,
    hasGameSnapshot,
    cleanGameDirectory,
    listInstalledCollections,
    getInstalledMods,
  } from "$lib/api";

  interface Props {
    collectionName: string;
    game: DetectedGame;
    ondelete: () => Promise<void>;
    oncancel: () => void;
  }

  let { collectionName, game, ondelete, oncancel }: Props = $props();

  let deleteDownloads = $state(false);
  let deleteRemoveAllMods = $state(false);
  let deleteCleanGameDir = $state(false);
  let deleteHasSnapshot = $state(false);
  let deleteDownloadSize = $state<number | null>(null);
  let deleteDownloadSizeLoading = $state(true);
  let deletingCollection = $state(false);

  // Load download size and snapshot status on mount
  $effect(() => {
    // Re-run when collectionName changes
    const name = collectionName;
    const g = game;
    deleteDownloads = false;
    deleteRemoveAllMods = false;
    deleteCleanGameDir = false;
    deleteHasSnapshot = false;
    deleteDownloadSize = null;
    deleteDownloadSizeLoading = true;

    Promise.all([
      getCollectionDownloadSize(g.game_id, g.bottle_name, name).catch(() => null),
      hasGameSnapshot(g.game_id, g.bottle_name).catch(() => false),
    ]).then(([size, snap]) => {
      deleteDownloadSize = size;
      deleteHasSnapshot = snap;
    }).catch((err) => {
      console.warn('Failed to fetch collection delete info:', err);
      deleteDownloadSize = null;
    }).finally(() => {
      deleteDownloadSizeLoading = false;
    });
  });

  function formatDiskSize(bytes: number): string {
    if (bytes >= 1_073_741_824) return (bytes / 1_073_741_824).toFixed(1) + " GB";
    if (bytes >= 1_048_576) return (bytes / 1_048_576).toFixed(1) + " MB";
    if (bytes >= 1024) return (bytes / 1024).toFixed(1) + " KB";
    return bytes + " B";
  }

  async function handleDelete() {
    const shouldCleanGameDir = deleteCleanGameDir;
    deletingCollection = true;

    // Initialize uninstall status
    collectionUninstallStatus.set({
      active: true,
      collectionName,
      totalMods: 0,
      currentMod: 0,
      currentModName: "",
      currentStep: "",
      completed: 0,
      failed: 0,
      phase: "removing",
      errors: [],
      result: null,
    });

    // Listen for progress events
    const unlistenUninstall = await listen<UninstallProgressEvent>("uninstall-progress", (event) => {
      const e = event.payload;
      collectionUninstallStatus.update((s) => {
        if (!s) return s;
        const next = { ...s };

        switch (e.kind) {
          case "uninstallStarted":
            next.totalMods = e.total_mods;
            break;
          case "modUninstalling":
            next.currentMod = e.mod_index + 1;
            next.currentModName = e.mod_name;
            next.currentStep = e.step;
            break;
          case "modUninstalled":
            next.completed = next.completed + 1;
            break;
          case "modUninstallFailed":
            next.failed = next.failed + 1;
            next.errors = [...next.errors, `${e.mod_name}: ${e.error}`];
            break;
          case "redeployStarted":
            next.phase = "redeploying";
            next.currentModName = "";
            next.currentStep = "Redeploying remaining mods...";
            break;
          case "redeployCompleted":
            break;
          case "uninstallCompleted":
            next.phase = "complete";
            next.result = { modsRemoved: e.mods_removed, downloadsRemoved: e.downloads_removed };
            if (e.errors.length > 0) {
              next.errors = e.errors;
            }
            break;
        }
        return next;
      });
    });

    try {
      await deleteCollection(game.game_id, game.bottle_name, collectionName, deleteDownloads, deleteRemoveAllMods);

      // After successful uninstall, optionally clean non-stock files (preserving SKSE)
      if (shouldCleanGameDir) {
        collectionUninstallStatus.update((s) => {
          if (!s) return s;
          return { ...s, currentStep: "Cleaning non-stock files from game directory...", phase: "redeploying" };
        });
        try {
          const cleanResult = await cleanGameDirectory(game.game_id, game.bottle_name, {
            remove_loose_files: true,
            remove_archives: true,
            remove_enb: false,
            remove_saves: false,
            remove_skse: false,
            orphans_only: false,
            dry_run: false,
            exclude_patterns: [],
          });
          collectionUninstallStatus.update((s) => {
            if (!s) return s;
            return { ...s, currentStep: `Cleaned ${cleanResult.removed_files.length} non-stock files` };
          });
        } catch (cleanErr: unknown) {
          collectionUninstallStatus.update((s) => {
            if (!s) return s;
            return { ...s, errors: [...s.errors, `Game dir cleanup: ${cleanErr}`] };
          });
        }
      }
    } catch (e: unknown) {
      showError(`Failed to delete: ${e}`);
      collectionUninstallStatus.set(null);
    } finally {
      unlistenUninstall();
      deletingCollection = false;

      // Refresh global state so Mods page and top bar reflect the uninstall
      listInstalledCollections(game.game_id, game.bottle_name)
        .then(cols => {
          collectionList.set(cols);
          // Clear active collection if the deleted one was active
          const currentActive = $activeCollection;
          if (currentActive?.name === collectionName) {
            activeCollection.set(null);
          }
        })
        .catch((err) => console.error('Failed to refresh collections after uninstall:', err));

      // Refresh the installed mods store so Mods page updates immediately
      getInstalledMods(game.game_id, game.bottle_name)
        .then(mods => installedMods.set(mods))
        .catch((err) => console.error('Failed to refresh mods after uninstall:', err));

      await ondelete();
    }
  }
</script>

<div class="modal-overlay" onclick={() => { if (!deletingCollection) oncancel(); }} role="dialog" aria-modal="true" aria-label="Confirm deletion">
  <div class="modal-dialog" onclick={(e) => e.stopPropagation()} role="document">
    <div class="modal-icon">
      <svg width="36" height="36" viewBox="0 0 24 24" fill="none" stroke="#ef4444" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
        <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
        <line x1="12" y1="9" x2="12" y2="13" />
        <line x1="12" y1="17" x2="12.01" y2="17" />
      </svg>
    </div>
    <h3 class="modal-title">Delete '{collectionName}'?</h3>
    <p class="modal-desc">This will uninstall all mods in this collection, remove staging files, and clean up the database. This cannot be undone.</p>

    <div class="modal-option">
      <label class="modal-checkbox-label">
        <input type="checkbox" bind:checked={deleteDownloads} />
        <span class="modal-checkbox-text">
          Also delete downloaded archives
          {#if deleteDownloadSizeLoading}
            <span class="modal-size-loading">calculating...</span>
          {:else if deleteDownloadSize != null && deleteDownloadSize > 0}
            <span class="modal-size-badge">saves {formatDiskSize(deleteDownloadSize)}</span>
          {:else if deleteDownloadSize === 0}
            <span class="modal-size-note">no unique downloads</span>
          {/if}
        </span>
      </label>
      {#if deleteDownloads}
        <p class="modal-option-hint modal-option-hint-warn">Archives unique to this collection will be permanently deleted.</p>
      {:else}
        <p class="modal-option-hint">Downloaded archives are kept so you can reinstall later without re-downloading.</p>
      {/if}
    </div>

    <div class="modal-option">
      <label class="modal-checkbox-label">
        <input type="checkbox" bind:checked={deleteRemoveAllMods} />
        <span class="modal-checkbox-text">
          Remove ALL mods, not just this collection
          <span class="modal-size-note">fastest</span>
        </span>
      </label>
      {#if deleteRemoveAllMods}
        <p class="modal-option-hint modal-option-hint-warn">Removes every installed mod for this game, including any manually installed mods outside the collection.</p>
      {:else}
        <p class="modal-option-hint">Only removes mods that belong to this collection.</p>
      {/if}
    </div>

    {#if deleteHasSnapshot}
      <div class="modal-option">
        <label class="modal-checkbox-label">
          <input type="checkbox" bind:checked={deleteCleanGameDir} />
          <span class="modal-checkbox-text">
            Clean non-stock files from game directory
            <span class="modal-size-note">preserves SKSE</span>
          </span>
        </label>
        {#if deleteCleanGameDir}
          <p class="modal-option-hint modal-option-hint-warn">Removes leftover loose files (meshes, textures, scripts, plugins) that aren't part of the original game. SKSE files are preserved.</p>
        {:else}
          <p class="modal-option-hint">Leave the game directory as-is after uninstalling the collection.</p>
        {/if}
      </div>
    {/if}

    <div class="modal-actions">
      <button
        class="btn btn-danger"
        onclick={handleDelete}
        disabled={deletingCollection}
      >
        {#if deletingCollection}
          <svg class="icon-spin" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
            <line x1="12" y1="2" x2="12" y2="6" />
            <line x1="12" y1="18" x2="12" y2="22" />
            <line x1="4.93" y1="4.93" x2="7.76" y2="7.76" />
            <line x1="16.24" y1="16.24" x2="19.07" y2="19.07" />
            <line x1="2" y1="12" x2="6" y2="12" />
            <line x1="18" y1="12" x2="22" y2="12" />
            <line x1="4.93" y1="19.07" x2="7.76" y2="16.24" />
            <line x1="16.24" y1="7.76" x2="19.07" y2="4.93" />
          </svg>
          Deleting...
        {:else}
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="3 6 5 6 21 6" />
            <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
          </svg>
          Delete Collection
        {/if}
      </button>
      <button
        class="btn btn-ghost"
        onclick={oncancel}
        disabled={deletingCollection}
      >
        Cancel
      </button>
    </div>
  </div>
</div>

<style>
  .modal-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
  }

  .modal-dialog {
    background: color-mix(in srgb, var(--bg-grouped) 75%, transparent);
    backdrop-filter: var(--glass-blur-heavy);
    -webkit-backdrop-filter: var(--glass-blur-heavy);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: var(--radius-lg, 12px);
    width: min(440px, 90vw);
    padding: var(--space-6);
    box-shadow: var(--glass-refraction),
                var(--glass-edge-shadow),
                0 8px 32px rgba(0, 0, 0, 0.4);
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: var(--space-4);
  }

  .modal-icon {
    flex-shrink: 0;
  }

  .modal-title {
    font-size: 18px;
    font-weight: 700;
    color: var(--text-primary);
    margin: 0;
    letter-spacing: -0.02em;
  }

  .modal-desc {
    font-size: 13px;
    color: var(--text-secondary);
    line-height: 1.5;
    margin: 0;
    max-width: 360px;
  }

  .modal-option {
    width: 100%;
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    padding: var(--space-3) var(--space-4);
    text-align: left;
  }

  .modal-checkbox-label {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    cursor: pointer;
    font-size: 13px;
    font-weight: 500;
    color: var(--text-primary);
  }

  .modal-checkbox-label input {
    accent-color: var(--system-accent);
    width: 16px;
    height: 16px;
    flex-shrink: 0;
  }

  .modal-checkbox-text {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .modal-size-badge {
    font-size: 11px;
    font-weight: 600;
    color: #22c55e;
    background: rgba(34, 197, 94, 0.12);
    padding: 1px 8px;
    border-radius: 100px;
    font-family: var(--font-mono);
  }

  .modal-size-loading {
    font-size: 11px;
    color: var(--text-tertiary);
    font-style: italic;
  }

  .modal-size-note {
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .modal-option-hint {
    font-size: 11px;
    color: var(--text-tertiary);
    margin: var(--space-2) 0 0 24px;
    line-height: 1.4;
  }

  .modal-option-hint-warn {
    color: #f59e0b;
  }

  .modal-actions {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    justify-content: center;
  }

  .modal-actions :global(.btn-danger) {
    padding: var(--space-2) var(--space-5);
  }

  .icon-spin { animation: spin 1.5s linear infinite; }

  @keyframes spin {
    from { transform: rotate(0deg); }
    to { transform: rotate(360deg); }
  }
</style>
