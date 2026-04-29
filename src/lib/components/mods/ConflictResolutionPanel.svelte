<script lang="ts">
  import {
    analyzeConflicts,
    resolveAllConflicts,
    recordConflictWinner,
    setModPriority,
    redeployAllMods,
  } from "$lib/api";
  import type {
    DetectedGame,
    FileConflict,
    InstalledMod,
    ConflictSuggestion,
    ResolutionResult,
    IdenticalContentStats,
  } from "$lib/types";
  import { wineCtx } from "$lib/types";
  import {
    showError,
    showSuccess,
  } from "$lib/stores";
  import ConflictMap from "$lib/components/ConflictMap.svelte";

  interface Props {
    game: DetectedGame;
    conflicts: FileConflict[];
    installedMods: InstalledMod[];
    visible: boolean;
    showMap: boolean;
    deploying: boolean;
    onResolved: () => Promise<void>;
  }

  let {
    game,
    conflicts,
    installedMods: _installedMods,
    visible = $bindable(false),
    showMap = $bindable(false),
    deploying = $bindable(false),
    onResolved,
  }: Props = $props();

  let makingWinner = $state<number | null>(null);
  let suggestions = $state<ConflictSuggestion[]>([]);
  let analyzingConflicts = $state(false);
  let resolvingAll = $state(false);
  let resolutionResult = $state<ResolutionResult | null>(null);
  let identicalStats = $state<IdenticalContentStats | null>(null);

  async function handleMakeWinner(conflict: FileConflict, modId: number) {
    if (deploying) return;
    makingWinner = modId;
    deploying = true;
    try {
      const winner = conflict.mods.find(m => m.mod_id === conflict.winner_mod_id);
      const newPriority = winner ? winner.priority + 1 : 999;
      await setModPriority(modId, newPriority);
      const loserIds = conflict.mods
        .filter(m => m.mod_id !== modId)
        .map(m => m.mod_id);
      await recordConflictWinner(game.game_id, (wineCtx(game)?.bottle_name ?? ""), modId, loserIds);
      await redeployAllMods(game.game_id, (wineCtx(game)?.bottle_name ?? ""));
      await onResolved();
    } catch (e: unknown) {
      showError(`Failed to set winner: ${e}`);
    } finally {
      deploying = false;
      makingWinner = null;
    }
  }

  async function handleAnalyzeConflicts() {
    analyzingConflicts = true;
    resolutionResult = null;
    identicalStats = null;
    try {
      const response = await analyzeConflicts(game.game_id, (wineCtx(game)?.bottle_name ?? ""));
      suggestions = response.suggestions;
      identicalStats = response.identical_stats;
    } catch (e: unknown) {
      showError(`Conflict analysis failed: ${e}`);
    } finally {
      analyzingConflicts = false;
    }
  }

  async function handleMagicResolve() {
    resolvingAll = true;
    resolutionResult = null;
    identicalStats = null;
    try {
      resolutionResult = await resolveAllConflicts(game.game_id, (wineCtx(game)?.bottle_name ?? ""));
      await onResolved();
      // Re-analyze to show updated state
      const response = await analyzeConflicts(game.game_id, (wineCtx(game)?.bottle_name ?? ""));
      suggestions = response.suggestions;
      identicalStats = response.identical_stats;
      const autoCount = resolutionResult.author_resolved + resolutionResult.auto_suggested + resolutionResult.identical_content;
      showSuccess(`Resolved ${autoCount} conflicts automatically`);
    } catch (e: unknown) {
      showError(`Magic resolver failed: ${e}`);
    } finally {
      resolvingAll = false;
    }
  }

  function closePanel() {
    visible = false;
    suggestions = [];
    resolutionResult = null;
    identicalStats = null;
  }
</script>

<!-- Visual Conflict Map -->
{#if showMap}
  <ConflictMap visible={showMap} onclose={() => { showMap = false; }} />
{/if}

<!-- Smart Conflict Resolution Panel -->
{#if visible && conflicts.length > 0}
  <div class="conflict-panel">
    <div class="conflict-panel-header">
      <h3 class="conflict-panel-title">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
          <line x1="12" y1="9" x2="12" y2="13" />
          <line x1="12" y1="17" x2="12.01" y2="17" />
        </svg>
        File Conflicts ({conflicts.length})
      </h3>
      <div class="conflict-panel-actions">
        <button
          class="btn btn-accent btn-sm magic-resolve-btn"
          onclick={handleMagicResolve}
          disabled={resolvingAll || analyzingConflicts}
          title="Automatically resolve all conflicts using LOOT data and collection authorship"
        >
          {#if resolvingAll}
            <span class="spinner spinner-sm"></span>
            Resolving...
          {:else}
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M12 2l3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01L12 2z" />
            </svg>
            Magic Resolver
          {/if}
        </button>
        <button
          class="btn btn-ghost btn-sm"
          onclick={handleAnalyzeConflicts}
          disabled={analyzingConflicts}
          title="Analyze conflicts without applying changes"
        >
          {#if analyzingConflicts}
            <span class="spinner spinner-sm"></span>
          {:else}
            Analyze
          {/if}
        </button>
        <button class="btn btn-ghost btn-sm" onclick={closePanel}>
          <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
            <line x1="3" y1="3" x2="11" y2="11" />
            <line x1="11" y1="3" x2="3" y2="11" />
          </svg>
        </button>
      </div>
    </div>

    <!-- Resolution summary banner -->
    {#if resolutionResult}
      <div class="resolution-banner">
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14" />
          <polyline points="22 4 12 14.01 9 11.01" />
        </svg>
        <span>
          {resolutionResult.author_resolved} author-resolved,
          {resolutionResult.auto_suggested} auto-fixed,
          {#if resolutionResult.identical_content > 0}
            {resolutionResult.identical_content} identical files,
          {/if}
          {resolutionResult.manual_needed} need review
          {#if resolutionResult.priorities_changed > 0}
            &mdash; {resolutionResult.priorities_changed} priorities adjusted
          {/if}
        </span>
      </div>
    {/if}

    <!-- Identical content auto-resolution banner -->
    {#if identicalStats && identicalStats.identical_files_total > 0}
      <div class="identical-banner">
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M16 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2" />
          <circle cx="8.5" cy="7" r="4" />
          <polyline points="17 11 19 13 23 9" />
        </svg>
        <span>
          {identicalStats.fully_identical} conflict{identicalStats.fully_identical === 1 ? "" : "s"} auto-resolved (identical files across mods){#if identicalStats.partially_identical > 0}, {identicalStats.partially_identical} partially identical{/if}
        </span>
      </div>
    {/if}

    <!-- Smart suggestions view -->
    {#if suggestions.length > 0}
      <div class="conflict-list">
        {#each suggestions as s (s.relative_path)}
          <div class="conflict-item" class:conflict-resolved={s.status === "AuthorResolved"} class:conflict-suggested={s.status === "Suggested"} class:conflict-identical={s.status === "IdenticalContent"}>
            <div class="conflict-path">
              <span class="conflict-status-badge" class:status-author={s.status === "AuthorResolved"} class:status-suggested={s.status === "Suggested"} class:status-manual={s.status === "Manual"} class:status-identical={s.status === "IdenticalContent"}>
                {#if s.status === "AuthorResolved"}OK
                {:else if s.status === "Suggested"}Auto
                {:else if s.status === "IdenticalContent"}Same
                {:else}Manual{/if}
              </span>
              <span class="conflict-filepath">{s.relative_path}</span>
            </div>
            <div class="conflict-reason">{s.reason}</div>
            <div class="conflict-mods">
              {#each s.mods as mod (mod.mod_id)}
                <div class="conflict-mod" class:conflict-winner={mod.mod_id === s.suggested_winner_id}>
                  <span class="conflict-mod-name">
                    {mod.mod_name}
                    {#if mod.mod_id === s.suggested_winner_id}
                      <span class="winner-badge">{s.status === "AuthorResolved" ? "Author" : s.status === "Suggested" ? "Suggested" : s.status === "IdenticalContent" ? "Identical" : "Winner"}</span>
                    {/if}
                  </span>
                  <span class="conflict-mod-priority">Priority {mod.priority}</span>
                </div>
              {/each}
            </div>
          </div>
        {/each}
      </div>

    <!-- Fallback: raw conflict view (before analysis) -->
    {:else}
      <div class="conflict-list">
        {#each conflicts as conflict (conflict.relative_path)}
          <div class="conflict-item">
            <div class="conflict-path">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z" />
                <polyline points="13 2 13 9 20 9" />
              </svg>
              <span class="conflict-filepath">{conflict.relative_path}</span>
            </div>
            <div class="conflict-mods">
              {#each conflict.mods as mod (mod.mod_id)}
                <div class="conflict-mod" class:conflict-winner={mod.mod_id === conflict.winner_mod_id}>
                  <span class="conflict-mod-name">
                    {mod.mod_name}
                    {#if mod.mod_id === conflict.winner_mod_id}
                      <span class="winner-badge">Winner</span>
                    {/if}
                  </span>
                  <span class="conflict-mod-priority">Priority {mod.priority}</span>
                  {#if mod.mod_id !== conflict.winner_mod_id}
                    <button
                      class="btn btn-ghost btn-sm make-winner-btn"
                      onclick={() => handleMakeWinner(conflict, mod.mod_id)}
                      disabled={makingWinner !== null}
                    >
                      {#if makingWinner === mod.mod_id}
                        <span class="spinner spinner-sm"></span>
                      {:else}
                        Make Winner
                      {/if}
                    </button>
                  {/if}
                </div>
              {/each}
            </div>
          </div>
        {/each}
      </div>
    {/if}
  </div>
{/if}

<style>
  .conflict-panel {
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    max-height: 360px;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    margin-bottom: var(--space-3);
  }

  .conflict-panel-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--separator);
    flex-shrink: 0;
  }

  .conflict-panel-title {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: 13px;
    font-weight: 600;
    color: var(--yellow);
  }

  .conflict-list {
    overflow-y: auto;
    padding: var(--space-2);
  }

  .conflict-item {
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    margin-bottom: var(--space-1);
  }

  .conflict-item:hover {
    background: var(--surface-hover);
  }

  .conflict-path {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-tertiary);
    margin-bottom: var(--space-2);
  }

  .conflict-filepath {
    font-size: 11px;
    font-family: var(--font-mono);
    word-break: break-all;
  }

  .conflict-mods {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding-left: var(--space-5);
  }

  .conflict-mod {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
    font-size: 12px;
  }

  .conflict-winner {
    background: color-mix(in srgb, var(--green) 8%, transparent);
  }

  .conflict-mod-name {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-primary);
    font-weight: 500;
    flex: 1;
    min-width: 0;
  }

  .winner-badge {
    display: inline-flex;
    align-items: center;
    padding: 0 5px;
    border-radius: 3px;
    background: color-mix(in srgb, var(--green) 15%, transparent);
    color: var(--green);
    font-size: 9px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    flex-shrink: 0;
  }

  .conflict-mod-priority {
    color: var(--text-tertiary);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }

  .make-winner-btn {
    flex-shrink: 0;
    color: var(--accent) !important;
  }

  .make-winner-btn:hover {
    background: var(--accent-subtle) !important;
  }

  .conflict-panel-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .magic-resolve-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    background: var(--accent);
    color: #fff;
    border-radius: var(--radius-sm);
    padding: 4px 10px;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    transition: opacity var(--duration-fast) var(--ease);
  }

  .magic-resolve-btn:hover:not(:disabled) {
    opacity: 0.85;
  }

  .magic-resolve-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .resolution-banner {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    background: color-mix(in srgb, var(--green) 8%, transparent);
    border-bottom: 1px solid color-mix(in srgb, var(--green) 20%, transparent);
    color: var(--green);
    font-size: 12px;
    font-weight: 500;
    flex-shrink: 0;
  }

  .conflict-status-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 1px 6px;
    border-radius: 3px;
    font-size: 9px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    flex-shrink: 0;
  }

  .status-author {
    background: color-mix(in srgb, var(--green) 15%, transparent);
    color: var(--green);
  }

  .status-suggested {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--accent);
  }

  .status-manual {
    background: color-mix(in srgb, var(--yellow) 15%, transparent);
    color: var(--yellow);
  }

  .status-identical {
    background: color-mix(in srgb, var(--text-tertiary) 15%, transparent);
    color: var(--text-tertiary);
  }

  .conflict-resolved {
    opacity: 0.6;
  }

  .conflict-identical {
    opacity: 0.45;
  }

  .identical-banner {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    background: color-mix(in srgb, var(--text-tertiary) 8%, transparent);
    border-bottom: 1px solid color-mix(in srgb, var(--text-tertiary) 20%, transparent);
    color: var(--text-secondary);
    font-size: 12px;
    font-weight: 500;
    flex-shrink: 0;
  }

  .conflict-reason {
    font-size: 11px;
    color: var(--text-tertiary);
    padding-left: var(--space-5);
    margin-bottom: var(--space-1);
    font-style: italic;
  }
</style>
