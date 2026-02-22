<script lang="ts">
  import { getModDependencies, checkDependencyIssues, addModDependency, removeModDependency } from "$lib/api";
  import type { ModDependency, DependencyIssue, InstalledMod } from "$lib/types";

  interface Props {
    gameId: string;
    bottleName: string;
    mods: InstalledMod[];
    selectedModId?: number;
  }

  let { gameId, bottleName, mods, selectedModId }: Props = $props();
  let dependencies = $state<ModDependency[]>([]);
  let issues = $state<DependencyIssue[]>([]);
  let loading = $state(false);
  let showAddForm = $state(false);
  let addDepName = $state("");
  let addRelationship = $state("requires");
  let addTargetId = $state<number | null>(null);

  async function load() {
    loading = true;
    try {
      issues = await checkDependencyIssues(gameId, bottleName);
      if (selectedModId) {
        dependencies = await getModDependencies(selectedModId);
      }
    } catch {} finally { loading = false; }
  }

  $effect(() => { if (gameId && bottleName) load(); });

  async function handleAdd() {
    if (!selectedModId || !addDepName) return;
    try {
      await addModDependency(gameId, bottleName, selectedModId, addTargetId, null, addDepName, addRelationship);
      showAddForm = false;
      addDepName = "";
      await load();
    } catch {}
  }

  async function handleRemove(depId: number) {
    try {
      await removeModDependency(depId);
      await load();
    } catch {}
  }

  function issueIcon(type: string): string {
    if (type === "missing_requirement") return "red";
    if (type === "active_conflict") return "yellow";
    return "orange";
  }

  function relationshipLabel(rel: string): string {
    if (rel === "requires") return "Requires";
    if (rel === "conflicts") return "Conflicts";
    if (rel === "patches") return "Patches";
    return rel;
  }

  function openAddForm() {
    showAddForm = true;
    addDepName = "";
    addRelationship = "requires";
    addTargetId = null;
  }

  function cancelAddForm() {
    showAddForm = false;
    addDepName = "";
  }

  const selectedModName = $derived(
    mods.find(m => m.id === selectedModId)?.name ?? null
  );
</script>

<div class="dep-panel">
  <!-- Issues Section -->
  {#if issues.length > 0}
    <div class="section-card">
      <div class="card-header">
        <h4 class="card-title">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
            <line x1="12" y1="9" x2="12" y2="13" />
            <line x1="12" y1="17" x2="12.01" y2="17" />
          </svg>
          Dependency Issues
        </h4>
        <span class="issue-count-badge">{issues.length}</span>
      </div>

      <div class="issue-list" role="list">
        {#each issues as issue, i (i)}
          <div class="issue-row" role="listitem">
            <div
              class="issue-icon"
              class:issue-icon-red={issue.issue_type === "missing_requirement"}
              class:issue-icon-yellow={issue.issue_type === "active_conflict"}
              class:issue-icon-orange={issue.issue_type === "orphaned_patch"}
            >
              {#if issue.issue_type === "missing_requirement"}
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                  <circle cx="12" cy="12" r="10" />
                  <line x1="15" y1="9" x2="9" y2="15" />
                  <line x1="9" y1="9" x2="15" y2="15" />
                </svg>
              {:else if issue.issue_type === "active_conflict"}
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
                  <line x1="12" y1="9" x2="12" y2="13" />
                  <line x1="12" y1="17" x2="12.01" y2="17" />
                </svg>
              {:else}
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                  <circle cx="12" cy="12" r="10" />
                  <line x1="12" y1="8" x2="12" y2="12" />
                  <line x1="12" y1="16" x2="12.01" y2="16" />
                </svg>
              {/if}
            </div>
            <div class="issue-content">
              <span class="issue-mod-name">{issue.mod_name}</span>
              <span class="issue-message">{issue.message}</span>
            </div>
          </div>
        {/each}
      </div>
    </div>
  {/if}

  <!-- Dependencies for Selected Mod -->
  <div class="section-card">
    <div class="card-header">
      <h4 class="card-title">
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="18" cy="5" r="3" />
          <circle cx="6" cy="12" r="3" />
          <circle cx="18" cy="19" r="3" />
          <line x1="8.59" y1="13.51" x2="15.42" y2="17.49" />
          <line x1="15.41" y1="6.51" x2="8.59" y2="10.49" />
        </svg>
        Dependencies
      </h4>
      {#if selectedModName}
        <span class="card-subtitle">{selectedModName}</span>
      {/if}
      {#if selectedModId}
        <button
          class="btn btn-accent btn-sm"
          onclick={openAddForm}
          disabled={showAddForm}
          type="button"
        >
          <svg width="12" height="12" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
            <line x1="7" y1="2" x2="7" y2="12" />
            <line x1="2" y1="7" x2="12" y2="7" />
          </svg>
          Add
        </button>
      {/if}
    </div>

    {#if loading}
      <div class="card-loading">
        <span class="spinner-sm"></span>
        <span class="loading-label">Loading dependencies...</span>
      </div>
    {:else if !selectedModId}
      <div class="card-empty">
        <div class="empty-icon">
          <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="18" cy="5" r="3" />
            <circle cx="6" cy="12" r="3" />
            <circle cx="18" cy="19" r="3" />
            <line x1="8.59" y1="13.51" x2="15.42" y2="17.49" />
            <line x1="15.41" y1="6.51" x2="8.59" y2="10.49" />
          </svg>
        </div>
        <p class="empty-title">Select a mod to view its dependencies</p>
        <p class="empty-description">Choose a mod from the list to see and manage dependency relationships.</p>
      </div>
    {:else}
      <!-- Add Dependency Form -->
      {#if showAddForm}
        <div class="add-form">
          <div class="form-row">
            <label class="form-label" for="dep-target">Target Mod (optional)</label>
            <select
              id="dep-target"
              class="form-select"
              bind:value={addTargetId}
            >
              <option value={null}>None (external dependency)</option>
              {#each mods.filter(m => m.id !== selectedModId) as mod (mod.id)}
                <option value={mod.id}>{mod.name}</option>
              {/each}
            </select>
          </div>

          <div class="form-row">
            <label class="form-label">Relationship</label>
            <div class="segmented-control" role="radiogroup" aria-label="Dependency relationship">
              <button
                class="segment"
                class:segment-active={addRelationship === "requires"}
                onclick={() => addRelationship = "requires"}
                role="radio"
                aria-checked={addRelationship === "requires"}
                type="button"
              >
                Requires
              </button>
              <button
                class="segment"
                class:segment-active={addRelationship === "conflicts"}
                onclick={() => addRelationship = "conflicts"}
                role="radio"
                aria-checked={addRelationship === "conflicts"}
                type="button"
              >
                Conflicts
              </button>
              <button
                class="segment"
                class:segment-active={addRelationship === "patches"}
                onclick={() => addRelationship = "patches"}
                role="radio"
                aria-checked={addRelationship === "patches"}
                type="button"
              >
                Patches
              </button>
            </div>
          </div>

          <div class="form-row">
            <label class="form-label" for="dep-name">Dependency Name</label>
            <input
              id="dep-name"
              class="form-input"
              type="text"
              placeholder="e.g. SKSE, SkyUI, Address Library..."
              bind:value={addDepName}
            />
          </div>

          <div class="form-actions">
            <button
              class="btn btn-accent btn-sm"
              onclick={handleAdd}
              disabled={!addDepName.trim()}
              type="button"
            >
              Save
            </button>
            <button
              class="btn btn-ghost btn-sm"
              onclick={cancelAddForm}
              type="button"
            >
              Cancel
            </button>
          </div>
        </div>
      {/if}

      <!-- Dependencies List -->
      {#if dependencies.length === 0 && !showAddForm}
        <div class="card-empty-inline">
          <span class="empty-text">No dependencies defined for this mod.</span>
        </div>
      {:else if dependencies.length > 0}
        <div class="dep-list" role="list">
          {#each dependencies as dep (dep.id)}
            <div class="dep-row" role="listitem">
              <span
                class="rel-badge"
                class:rel-requires={dep.relationship === "requires"}
                class:rel-conflicts={dep.relationship === "conflicts"}
                class:rel-patches={dep.relationship === "patches"}
              >
                {relationshipLabel(dep.relationship)}
              </span>
              <span class="dep-name">{dep.dep_name}</span>
              <button
                class="dep-remove"
                onclick={() => handleRemove(dep.id)}
                title="Remove dependency"
                aria-label="Remove dependency"
                type="button"
              >
                <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
                  <line x1="2" y1="2" x2="10" y2="10" />
                  <line x1="10" y1="2" x2="2" y2="10" />
                </svg>
              </button>
            </div>
          {/each}
        </div>
      {/if}
    {/if}
  </div>
</div>

<style>
  /* ---- Panel Container ---- */

  .dep-panel {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  /* ---- Section Card ---- */

  .section-card {
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-lg);
    overflow: hidden;
    box-shadow: var(--glass-edge-shadow);
  }

  .card-header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    background: var(--surface-subtle);
    border-bottom: 1px solid var(--separator);
  }

  .card-title {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .card-title svg {
    color: var(--system-accent);
    flex-shrink: 0;
  }

  .card-subtitle {
    font-size: 12px;
    color: var(--text-tertiary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
    flex: 1;
  }

  .issue-count-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    border-radius: 100px;
    font-size: 10px;
    font-weight: 700;
    color: var(--red);
    background: var(--red-subtle, rgba(255, 69, 58, 0.12));
    font-variant-numeric: tabular-nums;
    margin-left: auto;
  }

  /* ---- Buttons ---- */

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

  .btn-accent {
    background: var(--system-accent);
    color: white;
    margin-left: auto;
  }

  .btn-accent:hover:not(:disabled) {
    filter: brightness(1.1);
  }

  .btn-ghost {
    background: transparent;
    color: var(--text-secondary);
  }

  .btn-ghost:hover:not(:disabled) {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  /* ---- Loading ---- */

  .card-loading {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-4);
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

  .loading-label {
    font-size: 12px;
    color: var(--text-tertiary);
  }

  /* ---- Empty States ---- */

  .card-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-8);
    text-align: center;
  }

  .card-empty-inline {
    padding: var(--space-4);
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

  .empty-text {
    font-size: 12px;
    color: var(--text-tertiary);
  }

  /* ---- Issues List ---- */

  .issue-list {
    overflow-y: auto;
    max-height: 300px;
  }

  .issue-row {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--separator);
    transition: background var(--duration-fast) var(--ease);
  }

  .issue-row:last-child {
    border-bottom: none;
  }

  .issue-row:hover {
    background: var(--surface-hover);
  }

  .issue-icon {
    flex-shrink: 0;
    width: 20px;
    height: 20px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 50%;
    margin-top: 1px;
  }

  .issue-icon-red {
    color: var(--red);
    background: var(--red-subtle, rgba(255, 69, 58, 0.12));
  }

  .issue-icon-yellow {
    color: var(--yellow);
    background: var(--yellow-subtle, rgba(255, 214, 10, 0.12));
  }

  .issue-icon-orange {
    color: var(--orange, #ff9f0a);
    background: var(--orange-subtle, rgba(255, 159, 10, 0.12));
  }

  .issue-content {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .issue-mod-name {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .issue-message {
    font-size: 12px;
    color: var(--text-tertiary);
    line-height: 1.4;
  }

  /* ---- Dependencies List ---- */

  .dep-list {
    overflow-y: auto;
    max-height: 400px;
  }

  .dep-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    border-bottom: 1px solid var(--separator);
    transition: background var(--duration-fast) var(--ease);
  }

  .dep-row:last-child {
    border-bottom: none;
  }

  .dep-row:hover {
    background: var(--surface-hover);
  }

  /* ---- Relationship Badges ---- */

  .rel-badge {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    padding: 1px 6px;
    border-radius: 4px;
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }

  .rel-requires {
    background: var(--accent-subtle, rgba(0, 122, 255, 0.12));
    color: var(--accent, var(--system-accent));
  }

  .rel-conflicts {
    background: var(--red-subtle, rgba(255, 69, 58, 0.12));
    color: var(--red);
  }

  .rel-patches {
    background: var(--yellow-subtle, rgba(255, 214, 10, 0.1));
    color: var(--yellow);
  }

  .dep-name {
    flex: 1;
    font-size: 12px;
    font-weight: 500;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  .dep-remove {
    flex-shrink: 0;
    padding: var(--space-1);
    border-radius: var(--radius-sm);
    color: var(--text-quaternary);
    transition: all var(--duration-fast) var(--ease);
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
  }

  .dep-remove:hover {
    background: var(--red-subtle, rgba(255, 69, 58, 0.12));
    color: var(--red);
  }

  /* ---- Add Form ---- */

  .add-form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
    border-bottom: 1px solid var(--separator);
    background: var(--surface-subtle);
  }

  .form-row {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .form-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .form-select {
    padding: var(--space-2) var(--space-3);
    background: var(--bg-base, var(--bg-grouped-secondary));
    border: 1px solid var(--separator-opaque);
    border-radius: var(--radius-sm);
    color: var(--text-primary);
    font-size: 13px;
    outline: none;
    cursor: pointer;
    transition: border-color var(--duration-fast) var(--ease);
  }

  .form-select:focus {
    border-color: var(--system-accent);
    box-shadow: 0 0 0 3px rgba(0, 122, 255, 0.15);
  }

  .form-input {
    padding: var(--space-2) var(--space-3);
    background: var(--bg-base, var(--bg-grouped-secondary));
    border: 1px solid var(--separator-opaque);
    border-radius: var(--radius-sm);
    color: var(--text-primary);
    font-size: 13px;
    outline: none;
    transition: border-color var(--duration-fast) var(--ease);
  }

  .form-input::placeholder {
    color: var(--text-quaternary);
  }

  .form-input:focus {
    border-color: var(--system-accent);
    box-shadow: 0 0 0 3px rgba(0, 122, 255, 0.15);
  }

  /* ---- Segmented Control ---- */

  .segmented-control {
    display: flex;
    background: var(--bg-secondary, var(--bg-grouped-secondary));
    border-radius: var(--radius-sm);
    padding: 2px;
    gap: 2px;
  }

  .segment {
    flex: 1;
    padding: var(--space-1) var(--space-3);
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
    border-radius: calc(var(--radius-sm) - 2px);
    transition: all var(--duration-fast) var(--ease);
    text-align: center;
  }

  .segment:hover:not(.segment-active) {
    color: var(--text-primary);
  }

  .segment-active {
    background: var(--system-accent);
    color: white;
    box-shadow: var(--shadow-sm, 0 1px 2px rgba(0, 0, 0, 0.2));
  }

  .form-actions {
    display: flex;
    gap: var(--space-2);
  }
</style>
