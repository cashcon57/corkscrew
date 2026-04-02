<script lang="ts">
  import {
    batchToggleMods,
    uninstallMod,
    onBulkOperationProgress,
  } from "$lib/api";
  import { showError, showSuccess, gameLock, gameLockOverridden } from "$lib/stores";
  import type { DetectedGame } from "$lib/types";

  let {
    game,
    selectedModIds,
    onComplete,
    onClearSelection,
  }: {
    game: DetectedGame;
    selectedModIds: Set<number>;
    onComplete: () => Promise<void>;
    onClearSelection: () => void;
  } = $props();

  let bulkOperating = $state<"enabling" | "disabling" | "uninstalling" | null>(null);
  let bulkProgress = $state<{ phase: string; current: number; total: number; message: string } | null>(null);
  let bulkProgressUnlisten: (() => void) | null = null;

  async function startBulkListener() {
    bulkProgress = null;
    bulkProgressUnlisten = await onBulkOperationProgress((p) => {
      bulkProgress = p;
    });
  }

  function stopBulkListener() {
    if (bulkProgressUnlisten) { bulkProgressUnlisten(); bulkProgressUnlisten = null; }
    bulkProgress = null;
  }

  /** Clean up listener on component destroy */
  export function cleanup() {
    stopBulkListener();
  }

  export async function batchUninstallFromParent() {
    await batchUninstall();
  }

  async function batchEnable() {
    if ($gameLock && !$gameLockOverridden) {
      showError('Cannot modify mods while the game is running. Close the game or click "Unlock Anyway".');
      return;
    }
    bulkOperating = "enabling";
    await startBulkListener();
    try {
      const ids = Array.from(selectedModIds);
      const result = await batchToggleMods(ids, game.game_id, game.bottle_name, true);
      onClearSelection();
      await onComplete();
      const count = parseInt(result, 10);
      if (count > 0) showSuccess(`Enabled ${count} mod${count === 1 ? "" : "s"}`);
    } catch (e) {
      await onComplete();
      showError(`${e}`);
    } finally {
      stopBulkListener();
      bulkOperating = null;
    }
  }

  async function batchDisable() {
    if ($gameLock && !$gameLockOverridden) {
      showError('Cannot modify mods while the game is running. Close the game or click "Unlock Anyway".');
      return;
    }
    bulkOperating = "disabling";
    await startBulkListener();
    try {
      const ids = Array.from(selectedModIds);
      const result = await batchToggleMods(ids, game.game_id, game.bottle_name, false);
      onClearSelection();
      await onComplete();
      const count = parseInt(result, 10);
      if (count > 0) showSuccess(`Disabled ${count} mod${count === 1 ? "" : "s"}`);
    } catch (e) {
      await onComplete();
      showError(`${e}`);
    } finally {
      stopBulkListener();
      bulkOperating = null;
    }
  }

  async function batchUninstall() {
    if ($gameLock && !$gameLockOverridden) {
      showError('Cannot uninstall mods while the game is running. Close the game or click "Unlock Anyway".');
      return;
    }
    bulkOperating = "uninstalling";
    try {
      for (const id of selectedModIds) {
        await uninstallMod(id, game.game_id, game.bottle_name);
      }
      onClearSelection();
      await onComplete();
    } finally {
      bulkOperating = null;
    }
  }
</script>

{#if selectedModIds.size > 0}
  <div class="bulk-action-bar">
    {#if bulkOperating && bulkProgress}
      <div class="bulk-progress">
        <div class="bulk-progress-header">
          <span class="bulk-progress-label">{bulkProgress.message}</span>
          {#if bulkProgress.phase === "toggle"}
            <span class="bulk-progress-count">{bulkProgress.current}/{bulkProgress.total}</span>
          {/if}
        </div>
        <div class="bulk-progress-bar">
          <div
            class="bulk-progress-fill"
            class:indeterminate={bulkProgress.phase === "redeploy" || bulkProgress.phase === "plugins"}
            style="width: {bulkProgress.phase === 'toggle' ? (bulkProgress.current / bulkProgress.total) * 100 : 100}%"
          ></div>
        </div>
      </div>
    {:else}
      <span class="bulk-count">{selectedModIds.size} selected</span>
      <button class="btn btn-sm btn-secondary" disabled={bulkOperating !== null} onclick={batchEnable}>
        {bulkOperating === "enabling" ? "Enabling..." : "Enable All"}
      </button>
      <button class="btn btn-sm btn-secondary" disabled={bulkOperating !== null} onclick={batchDisable}>
        {bulkOperating === "disabling" ? "Disabling..." : "Disable All"}
      </button>
      <button class="btn btn-sm btn-ghost-danger" disabled={bulkOperating !== null} onclick={batchUninstall}>
        {bulkOperating === "uninstalling" ? "Uninstalling..." : "Uninstall"}
      </button>
      <button class="btn btn-sm btn-ghost" disabled={bulkOperating !== null} onclick={onClearSelection}>Clear</button>
    {/if}
  </div>
{/if}
