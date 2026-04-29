<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { selectedGame, showError } from "$lib/stores";
  import type { CollectionInstallCheckpoint, DetectedGame } from "$lib/types";
  import { wineCtx } from "$lib/types";
  import {
    getIncompleteCollectionInstalls,
    resumeCollectionInstall,
    abandonCollectionInstall,
    deleteCollection,
  } from "$lib/api";
  import { resumeInstallTracking } from "$lib/installService";

  interface Props {
    game: DetectedGame;
    onresume: () => void;
    ondismiss: () => void;
  }

  let { game, onresume, ondismiss }: Props = $props();

  let interruptedInstall = $state<CollectionInstallCheckpoint | null>(null);
  let resuming = $state(false);
  let showDismissConfirm = $state(false);
  let dismissCleanup = $state(true);
  let dismissing = $state(false);

  // Check for incomplete installs on mount
  $effect(() => {
    const g = game;
    getIncompleteCollectionInstalls(g.game_id, (wineCtx(g)?.bottle_name ?? ""))
      .then((incomplete) => {
        if (incomplete.length > 0) {
          interruptedInstall = incomplete[0];
        }
      })
      .catch((err) => console.warn('Failed to check for interrupted installs:', err));
  });

  async function handleResumeInstall() {
    if (!interruptedInstall) return;
    resuming = true;
    try {
      const modStatuses = JSON.parse(interruptedInstall.mod_statuses) as Record<string, string>;
      await resumeInstallTracking(
        interruptedInstall.collection_name,
        interruptedInstall.total_mods,
        interruptedInstall.completed_mods,
        modStatuses,
      );
      goto("/collections/progress");
      resumeCollectionInstall(interruptedInstall.id).catch((err) => console.error('Failed to resume collection install:', err));
      onresume();
    } catch (e: unknown) {
      showError(`Failed to resume: ${e}`);
      resuming = false;
    }
  }

  function handleDismissInstall() {
    if (!interruptedInstall) return;
    showDismissConfirm = true;
  }

  async function confirmDismissInstall() {
    if (!interruptedInstall) return;
    const checkpoint = interruptedInstall;
    dismissing = true;
    try {
      // Always abandon the checkpoint so it never comes back
      await abandonCollectionInstall(checkpoint.id);

      // Optionally clean up partially installed mods
      if (dismissCleanup) {
        try {
          await deleteCollection(
            game.game_id,
            (wineCtx(game)?.bottle_name ?? ""),
            checkpoint.collection_name,
            true, // delete unique downloads
          );
        } catch (err) {
          console.error("Cleanup of partial install failed (non-fatal):", err);
        }
      }
    } catch (err) {
      console.error("Failed to abandon checkpoint:", err);
    } finally {
      interruptedInstall = null;
      showDismissConfirm = false;
      dismissing = false;
      ondismiss();
    }
  }
</script>

{#if interruptedInstall}
  <div class="resume-banner">
    <div class="resume-info">
      <div class="resume-icon-wrap">
        <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="#f59e0b" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
          <line x1="12" y1="9" x2="12" y2="13" />
          <line x1="12" y1="17" x2="12.01" y2="17" />
        </svg>
      </div>
      <div class="resume-text">
        <span class="resume-title">Interrupted Installation Detected</span>
        <span class="resume-detail">
          "{interruptedInstall.collection_name}" — {interruptedInstall.completed_mods} of {interruptedInstall.total_mods} mods completed
          {#if interruptedInstall.failed_mods > 0}
            <span class="resume-failed">({interruptedInstall.failed_mods} failed)</span>
          {/if}
        </span>
        <div class="resume-progress-mini">
          <div class="resume-progress-fill" style="width: {Math.round((interruptedInstall.completed_mods / interruptedInstall.total_mods) * 100)}%"></div>
        </div>
      </div>
    </div>
    <div class="resume-actions">
      <button class="btn btn-primary" onclick={handleResumeInstall} disabled={resuming}>
        {#if resuming}
          <span class="spinner spinner-sm"></span> Resuming...
        {:else}
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="5 3 19 12 5 21 5 3" /></svg>
          Resume Installation
        {/if}
      </button>
      <button class="btn btn-ghost" onclick={handleDismissInstall} disabled={dismissing}>Dismiss</button>
    </div>
    {#if showDismissConfirm}
      <div class="resume-dismiss-confirm">
        <p class="dismiss-title">Permanently dismiss this installation?</p>
        <label class="dismiss-option">
          <input type="checkbox" bind:checked={dismissCleanup} />
          <span>Remove partially installed mods and downloaded files</span>
        </label>
        <div class="dismiss-actions">
          <button class="btn btn-danger btn-sm" onclick={confirmDismissInstall} disabled={dismissing}>
            {#if dismissing}
              <span class="spinner spinner-sm"></span> Cleaning up...
            {:else}
              Confirm
            {/if}
          </button>
          <button class="btn btn-ghost btn-sm" onclick={() => showDismissConfirm = false} disabled={dismissing}>Cancel</button>
        </div>
      </div>
    {/if}
  </div>
{/if}

<style>
  .resume-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: var(--space-4);
    padding: var(--space-4) var(--space-5);
    background: rgba(255, 159, 10, 0.08);
    border: 2px solid rgba(255, 159, 10, 0.4);
    border-radius: var(--radius-md);
    margin-bottom: var(--space-4);
    animation: resume-attention 2s ease-in-out 2;
  }

  @keyframes resume-attention {
    0%, 100% { border-color: rgba(255, 159, 10, 0.4); }
    50% { border-color: rgba(255, 159, 10, 0.8); background: rgba(255, 159, 10, 0.12); }
  }

  .resume-info { display: flex; align-items: center; gap: var(--space-3); min-width: 0; }
  .resume-icon-wrap { flex-shrink: 0; }
  .resume-text { display: flex; flex-direction: column; gap: 4px; min-width: 0; }
  .resume-title { font-weight: 700; color: var(--text-primary); font-size: 14px; }
  .resume-detail { font-size: 12px; color: var(--text-secondary); }
  .resume-failed { color: #ef4444; font-weight: 600; }
  .resume-progress-mini {
    width: 100%;
    max-width: 200px;
    height: 4px;
    background: rgba(255, 159, 10, 0.15);
    border-radius: 2px;
    overflow: hidden;
  }
  .resume-progress-fill {
    height: 100%;
    background: #f59e0b;
    border-radius: 2px;
    transition: width 300ms ease;
  }
  .resume-actions { display: flex; gap: var(--space-2); flex-shrink: 0; align-items: center; }

  .resume-dismiss-confirm {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    width: 100%;
    padding-top: var(--space-3);
    border-top: 1px solid rgba(255, 159, 10, 0.2);
    margin-top: var(--space-2);
  }
  .dismiss-title { font-size: 13px; font-weight: 600; color: var(--text-primary); }
  .dismiss-option {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: 12px;
    color: var(--text-secondary);
    cursor: pointer;
  }
  .dismiss-option input[type="checkbox"] { accent-color: #f59e0b; }
  .dismiss-actions { display: flex; gap: var(--space-2); margin-top: var(--space-1); }
</style>
