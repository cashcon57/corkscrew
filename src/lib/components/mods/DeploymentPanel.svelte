<script lang="ts">
  import { get } from "svelte/store";
  import { onDestroy } from "svelte";
  import {
    deployIncremental,
    redeployAllMods,
    purgeDeployment,
    getDeploymentStats,
    onDeployProgress,
  } from "$lib/api";
  import type { DeploymentHealth, DeployProgress, DetectedGame } from "$lib/types";
  import {
    showError,
    showSuccess,
    collectionInstallStatus,
    gameLock,
    gameLockOverridden,
  } from "$lib/stores";

  interface Props {
    game: DetectedGame;
    modCount: number;
    backendDeployInProgress: boolean;
    deploying: boolean;
    deployHealth: DeploymentHealth | null;
    onDeployComplete: () => Promise<void>;
  }

  let {
    game,
    modCount,
    backendDeployInProgress,
    deploying = $bindable(false),
    deployHealth = $bindable(null),
    onDeployComplete,
  }: Props = $props();

  let purging = $state(false);
  let deployProgress = $state(0);
  let deployProgressText = $state("");
  let deployUnlisten: (() => void) | null = null;

  onDestroy(() => {
    if (deployUnlisten) { deployUnlisten(); deployUnlisten = null; }
  });

  /** Refresh deployment health stats. Exposed for parent use via bind:this. */
  export async function refreshHealth(conflictCount: number) {
    const t0 = performance.now();
    try {
      const stats = await getDeploymentStats(game.game_id, game.bottle_name);
      deployHealth = { ...stats, conflict_count: conflictCount };
    } catch {
      deployHealth = null;
    } finally {
      console.log(`[perf] refreshHealth: ${(performance.now() - t0).toFixed(0)}ms`);
    }
  }

  /** Deploy mods. Exposed so the parent can trigger via keyboard shortcut (Ctrl+D). */
  export async function handleDeploy() {
    if ($gameLock && !$gameLockOverridden) {
      showError('Cannot deploy while the game is running. Close the game or click "Unlock Anyway".');
      return;
    }
    const installStatus = get(collectionInstallStatus);
    if (installStatus?.active) {
      showError('Cannot modify mods while a collection is being installed');
      return;
    }
    deploying = true;
    deployProgress = 0;
    deployProgressText = "Computing diff...";
    try {
      const incrResult = await deployIncremental(game.game_id, game.bottle_name);
      const totalChanged = incrResult.files_added + incrResult.files_removed + incrResult.files_updated;
      if (incrResult.fallback_used) {
        showSuccess(`Deployed ${incrResult.files_added} files (full redeploy)${incrResult.fallback_used ? " (copy fallback used)" : ""}`);
      } else if (totalChanged === 0) {
        showSuccess("Deployment is already up to date");
      } else {
        const parts: string[] = [];
        if (incrResult.files_added > 0) parts.push(`${incrResult.files_added} added`);
        if (incrResult.files_updated > 0) parts.push(`${incrResult.files_updated} updated`);
        if (incrResult.files_removed > 0) parts.push(`${incrResult.files_removed} removed`);
        parts.push(`${incrResult.files_unchanged} unchanged`);
        showSuccess(`Incremental deploy: ${parts.join(", ")}`);
      }
      if (incrResult.verification_failures.length > 0) {
        showError(`${incrResult.verification_failures.length} file(s) failed to deploy`);
      }
      await onDeployComplete();
    } catch {
      deployProgressText = "Falling back to full deploy...";
      try {
        deployUnlisten = await onDeployProgress((p: DeployProgress) => {
          if (p.total_files > 0) {
            deployProgress = Math.round((p.files_deployed / p.total_files) * 100);
            deployProgressText = `${p.mod_name} (${p.files_deployed}/${p.total_files} files)`;
          } else {
            deployProgress = p.total > 0 ? Math.round((p.current / p.total) * 100) : 0;
            deployProgressText = `${p.current}/${p.total} ${p.mod_name}`;
          }
        });
        const result = await redeployAllMods(game.game_id, game.bottle_name);
        showSuccess(`Deployed ${result.deployed_count} files (full redeploy)${result.fallback_used ? " (copy fallback used)" : ""}`);
        await onDeployComplete();
      } catch (e2: unknown) {
        showError(`Deploy failed: ${e2}`);
      }
    } finally {
      deploying = false;
      deployProgress = 0;
      deployProgressText = "";
      if (deployUnlisten) { deployUnlisten(); deployUnlisten = null; }
    }
  }

  async function handlePurge() {
    if ($gameLock && !$gameLockOverridden) {
      showError('Cannot purge while the game is running. Close the game or click "Unlock Anyway".');
      return;
    }
    purging = true;
    try {
      const removed = await purgeDeployment(game.game_id, game.bottle_name);
      showSuccess(`Purged ${removed.length} deployed files`);
      await onDeployComplete();
    } catch (e: unknown) {
      showError(`Purge failed: ${e}`);
    } finally {
      purging = false;
    }
  }
</script>

<!-- Deploy / Purge Buttons -->
{#if modCount > 0}
  <button
    class="btn btn-secondary btn-deploy"
    class:deploying
    onclick={handleDeploy}
    disabled={deploying || purging}
    title="Deploy all enabled mods to the game directory"
  >
    {#if deploying}
      <div class="deploy-progress-track">
        <div class="deploy-progress-fill" style="width: {deployProgress}%"></div>
      </div>
      <span class="deploy-progress-text">{deployProgressText || "Deploying..."}</span>
    {:else}
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <polyline points="20 6 9 17 4 12" />
      </svg>
      Deploy
    {/if}
  </button>
  <button
    class="btn btn-ghost-danger"
    onclick={handlePurge}
    disabled={deploying || purging}
    title="Remove all deployed files from the game directory"
  >
    {#if purging}
      <span class="spinner spinner-sm"></span>
      Purging...
    {:else}
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <polyline points="3 6 5 6 21 6" />
        <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
      </svg>
      Purge
    {/if}
  </button>
  {#if backendDeployInProgress && !deploying}
    <span class="deploy-in-progress-badge">
      <span class="deploy-pulse-dot"></span>
      Deploying...
    </span>
  {:else if deployHealth}
    <span class="deploy-status" class:status-deployed={deployHealth.is_deployed} class:status-purged={!deployHealth.is_deployed}>
      {deployHealth.is_deployed ? "Deployed" : "Purged"}
    </span>
  {/if}
{/if}

<style>
  .deploy-status {
    display: inline-flex;
    align-items: center;
    padding: 2px 8px;
    border-radius: 100px;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }

  .status-deployed {
    background: color-mix(in srgb, var(--green) 15%, transparent);
    color: var(--green);
  }

  .status-purged {
    background: color-mix(in srgb, var(--yellow) 15%, transparent);
    color: var(--yellow);
  }

  .deploy-in-progress-badge {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px 10px;
    border-radius: 100px;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    background: color-mix(in srgb, var(--blue, #58a6ff) 15%, transparent);
    color: var(--blue, #58a6ff);
  }

  .deploy-pulse-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--blue, #58a6ff);
    animation: deploy-pulse 1.4s ease-in-out infinite;
  }

  @keyframes deploy-pulse {
    0%, 100% { opacity: 1; transform: scale(1); }
    50% { opacity: 0.4; transform: scale(0.75); }
  }

  .btn-deploy {
    position: relative;
    overflow: hidden;
    min-width: 100px;
  }

  .btn-deploy.deploying {
    min-width: 160px;
  }

  .deploy-progress-track {
    position: absolute;
    inset: 0;
    background: transparent;
  }

  .deploy-progress-fill {
    height: 100%;
    background: var(--accent-subtle);
    transition: width 0.2s var(--ease);
  }

  .deploy-progress-text {
    position: relative;
    z-index: 1;
    font-size: 12px;
    white-space: nowrap;
  }
</style>
