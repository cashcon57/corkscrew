<script lang="ts">
  import { selectedGame } from "$lib/stores";
  import { toggleMod, redeployAllMods, getInstalledMods } from "$lib/api";
  import type { InstalledMod } from "$lib/types";

  let { mods, onClose, onComplete }: {
    mods: InstalledMod[];
    onClose: () => void;
    onComplete: (culprit: InstalledMod) => void;
  } = $props();

  type BisectStep = "intro" | "testing" | "found" | "not-found";

  let step = $state<BisectStep>("intro");
  let deploying = $state(false);
  let error = $state("");

  // Bisect state
  let enabledMods = $state<InstalledMod[]>([]);
  let lo = $state(0);
  let hi = $state(0);
  let iteration = $state(0);
  let totalIterations = $state(0);
  let activeBatch = $state<InstalledMod[]>([]);
  let culprit = $state<InstalledMod | null>(null);

  // Original enabled state to restore later
  let originalEnabledIds = $state<Set<number>>(new Set());

  function startBisect() {
    enabledMods = mods.filter(m => m.enabled);
    if (enabledMods.length < 2) {
      error = "Need at least 2 enabled mods to bisect.";
      return;
    }
    originalEnabledIds = new Set(enabledMods.map(m => m.id));
    lo = 0;
    hi = enabledMods.length - 1;
    iteration = 1;
    totalIterations = Math.ceil(Math.log2(enabledMods.length));
    step = "testing";
    applyBisectHalf("first");
  }

  async function applyBisectHalf(half: "first" | "second") {
    if (!$selectedGame) return;
    deploying = true;
    error = "";

    const mid = Math.floor((lo + hi) / 2);
    if (half === "first") {
      activeBatch = enabledMods.slice(lo, mid + 1);
    } else {
      activeBatch = enabledMods.slice(mid + 1, hi + 1);
    }

    try {
      // Disable all enabled mods first
      for (const mod of enabledMods) {
        if (mod.enabled) {
          await toggleMod(mod.id, $selectedGame.game_id, $selectedGame.bottle_name, false);
        }
      }
      // Enable only the active batch
      for (const mod of activeBatch) {
        await toggleMod(mod.id, $selectedGame.game_id, $selectedGame.bottle_name, true);
      }
      await redeployAllMods($selectedGame.game_id, $selectedGame.bottle_name);
    } catch (e: unknown) {
      error = `Deploy failed: ${e instanceof Error ? e.message : String(e)}`;
    } finally {
      deploying = false;
    }
  }

  function reportResult(crashed: boolean) {
    const mid = Math.floor((lo + hi) / 2);

    if (crashed) {
      // The problem is in the active batch
      if (activeBatch.length === 1) {
        culprit = activeBatch[0];
        step = "found";
        return;
      }
      // Narrow to the active batch's range
      const batchStart = enabledMods.indexOf(activeBatch[0]);
      const batchEnd = enabledMods.indexOf(activeBatch[activeBatch.length - 1]);
      lo = batchStart;
      hi = batchEnd;
    } else {
      // The problem is in the OTHER half
      const batchStart = enabledMods.indexOf(activeBatch[0]);
      const batchEnd = enabledMods.indexOf(activeBatch[activeBatch.length - 1]);

      if (batchStart === lo) {
        // We tested first half, problem is in second
        lo = mid + 1;
      } else {
        // We tested second half, problem is in first
        hi = mid;
      }

      if (lo > hi || lo >= enabledMods.length) {
        step = "not-found";
        return;
      }
    }

    if (lo === hi) {
      culprit = enabledMods[lo];
      step = "found";
      return;
    }

    iteration++;
    applyBisectHalf("first");
  }

  async function restoreOriginal() {
    if (!$selectedGame) return;
    deploying = true;
    try {
      const current = await getInstalledMods($selectedGame.game_id, $selectedGame.bottle_name);
      for (const mod of current) {
        const shouldBeEnabled = originalEnabledIds.has(mod.id);
        if (mod.enabled !== shouldBeEnabled) {
          await toggleMod(mod.id, $selectedGame.game_id, $selectedGame.bottle_name, shouldBeEnabled);
        }
      }
      await redeployAllMods($selectedGame.game_id, $selectedGame.bottle_name);
    } catch {
      // Best effort restore
    } finally {
      deploying = false;
      onClose();
    }
  }

  function handleFoundComplete() {
    if (culprit) onComplete(culprit);
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div class="bisect-overlay" onclick={onClose} onkeydown={(e) => e.key === "Escape" && onClose()} role="dialog">
  <div class="bisect-card" onclick={(e) => e.stopPropagation()}>
    {#if step === "intro"}
      <div class="bisect-header">
        <h3 class="bisect-title">Mod Bisect</h3>
        <button class="bisect-close" onclick={onClose}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" />
          </svg>
        </button>
      </div>
      <div class="bisect-body">
        <p class="bisect-description">
          Binary search through your enabled mods to find which one is causing crashes or issues.
          The wizard will enable/disable groups of mods and ask you to test after each step.
        </p>
        <div class="bisect-info">
          <span class="bisect-info-label">Enabled mods:</span>
          <span class="bisect-info-value">{mods.filter(m => m.enabled).length}</span>
        </div>
        <div class="bisect-info">
          <span class="bisect-info-label">Estimated steps:</span>
          <span class="bisect-info-value">{Math.ceil(Math.log2(Math.max(mods.filter(m => m.enabled).length, 2)))}</span>
        </div>
        {#if error}
          <p class="bisect-error">{error}</p>
        {/if}
      </div>
      <div class="bisect-actions">
        <button class="btn-secondary" onclick={onClose}>Cancel</button>
        <button class="btn-primary" onclick={startBisect} disabled={mods.filter(m => m.enabled).length < 2}>Start Bisect</button>
      </div>

    {:else if step === "testing"}
      <div class="bisect-header">
        <h3 class="bisect-title">Step {iteration} of ~{totalIterations}</h3>
        <button class="bisect-close" onclick={restoreOriginal} disabled={deploying}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" />
          </svg>
        </button>
      </div>
      <div class="bisect-body">
        {#if deploying}
          <div class="bisect-deploying">
            <span class="spinner"></span>
            <p>Deploying mod configuration...</p>
          </div>
        {:else}
          <p class="bisect-description">
            <strong>{activeBatch.length}</strong> mod{activeBatch.length !== 1 ? "s" : ""} are currently enabled.
            Launch your game and test if the issue occurs.
          </p>
          <div class="bisect-mod-list">
            {#each activeBatch as mod}
              <div class="bisect-mod-item">{mod.name}</div>
            {/each}
          </div>
          {#if error}
            <p class="bisect-error">{error}</p>
          {/if}
          <p class="bisect-question">Did the issue occur?</p>
        {/if}
      </div>
      {#if !deploying}
        <div class="bisect-actions">
          <button class="btn-secondary" onclick={restoreOriginal}>Abort & Restore</button>
          <button class="btn-success" onclick={() => reportResult(false)}>No Issue</button>
          <button class="btn-danger" onclick={() => reportResult(true)}>Still Crashes</button>
        </div>
      {/if}

    {:else if step === "found"}
      <div class="bisect-header">
        <h3 class="bisect-title">Culprit Found</h3>
      </div>
      <div class="bisect-body">
        <div class="bisect-found">
          <svg class="bisect-found-icon" width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="10" />
            <line x1="12" y1="8" x2="12" y2="12" />
            <line x1="12" y1="16" x2="12.01" y2="16" />
          </svg>
          <p class="bisect-found-name">{culprit?.name}</p>
          <p class="bisect-found-detail">This mod is likely causing the issue. Found in {iteration} step{iteration !== 1 ? "s" : ""}.</p>
        </div>
      </div>
      <div class="bisect-actions">
        <button class="btn-secondary" onclick={restoreOriginal} disabled={deploying}>
          {deploying ? "Restoring..." : "Restore & Close"}
        </button>
        <button class="btn-primary" onclick={handleFoundComplete}>Disable Culprit</button>
      </div>

    {:else if step === "not-found"}
      <div class="bisect-header">
        <h3 class="bisect-title">No Single Culprit Found</h3>
      </div>
      <div class="bisect-body">
        <p class="bisect-description">
          The bisect could not isolate a single mod. The issue might be caused by a combination of mods interacting,
          or may not be mod-related.
        </p>
      </div>
      <div class="bisect-actions">
        <button class="btn-primary" onclick={restoreOriginal} disabled={deploying}>
          {deploying ? "Restoring..." : "Restore & Close"}
        </button>
      </div>
    {/if}
  </div>
</div>

<style>
  .bisect-overlay {
    position: fixed;
    inset: 0;
    z-index: 9999;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.5);
    backdrop-filter: blur(4px);
    -webkit-backdrop-filter: blur(4px);
  }

  .bisect-card {
    width: 480px;
    max-height: 80vh;
    background: var(--bg-base);
    border: 1px solid var(--separator);
    border-radius: var(--radius-lg);
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5);
    overflow: hidden;
    animation: bisect-in 0.15s var(--ease);
    display: flex;
    flex-direction: column;
  }

  @keyframes bisect-in {
    from { opacity: 0; transform: scale(0.98); }
    to { opacity: 1; transform: scale(1); }
  }

  .bisect-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-4);
    border-bottom: 1px solid var(--separator);
  }

  .bisect-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
    margin: 0;
  }

  .bisect-close {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border-radius: var(--radius);
    color: var(--text-tertiary);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease);
  }

  .bisect-close:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .bisect-body {
    padding: var(--space-4);
    overflow-y: auto;
    flex: 1;
  }

  .bisect-description {
    font-size: 13px;
    color: var(--text-secondary);
    line-height: 1.5;
    margin: 0 0 var(--space-3);
  }

  .bisect-info {
    display: flex;
    justify-content: space-between;
    padding: var(--space-2) 0;
    font-size: 13px;
    border-bottom: 1px solid var(--separator);
  }

  .bisect-info-label {
    color: var(--text-tertiary);
  }

  .bisect-info-value {
    color: var(--text-primary);
    font-weight: 600;
  }

  .bisect-deploying {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-6) 0;
    color: var(--text-tertiary);
    font-size: 13px;
  }

  .bisect-mod-list {
    max-height: 200px;
    overflow-y: auto;
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    margin-bottom: var(--space-3);
  }

  .bisect-mod-item {
    padding: var(--space-2) var(--space-3);
    font-size: 12px;
    color: var(--text-secondary);
    border-bottom: 1px solid var(--separator);
  }

  .bisect-mod-item:last-child {
    border-bottom: none;
  }

  .bisect-question {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-primary);
    margin: var(--space-3) 0 0;
  }

  .bisect-error {
    font-size: 12px;
    color: var(--red);
    margin: var(--space-2) 0;
  }

  .bisect-found {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-4) 0;
    text-align: center;
  }

  .bisect-found-icon {
    color: var(--yellow);
  }

  .bisect-found-name {
    font-size: 16px;
    font-weight: 600;
    color: var(--text-primary);
    margin: 0;
  }

  .bisect-found-detail {
    font-size: 13px;
    color: var(--text-tertiary);
    margin: 0;
  }

  .bisect-actions {
    display: flex;
    gap: var(--space-2);
    justify-content: flex-end;
    padding: var(--space-3) var(--space-4);
    border-top: 1px solid var(--separator);
  }

  .btn-secondary {
    padding: 6px 14px;
    border-radius: var(--radius);
    font-size: 13px;
    font-weight: 500;
    color: var(--text-secondary);
    background: var(--surface);
    border: 1px solid var(--separator);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease);
  }

  .btn-secondary:hover {
    background: var(--surface-hover);
  }

  .btn-primary {
    padding: 6px 14px;
    border-radius: var(--radius);
    font-size: 13px;
    font-weight: 500;
    color: var(--accent-text, #fff);
    background: var(--accent);
    cursor: pointer;
    transition: opacity var(--duration-fast) var(--ease);
  }

  .btn-primary:hover {
    opacity: 0.9;
  }

  .btn-primary:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-success {
    padding: 6px 14px;
    border-radius: var(--radius);
    font-size: 13px;
    font-weight: 500;
    color: var(--green);
    background: var(--green-subtle);
    cursor: pointer;
    transition: opacity var(--duration-fast) var(--ease);
  }

  .btn-success:hover {
    opacity: 0.85;
  }

  .btn-danger {
    padding: 6px 14px;
    border-radius: var(--radius);
    font-size: 13px;
    font-weight: 500;
    color: var(--red);
    background: var(--red-subtle, rgba(255, 80, 80, 0.1));
    cursor: pointer;
    transition: opacity var(--duration-fast) var(--ease);
  }

  .btn-danger:hover {
    opacity: 0.85;
  }
</style>
