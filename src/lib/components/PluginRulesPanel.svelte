<script lang="ts">
  import { getPluginOrder } from "$lib/api";
  import { selectedGame, showError, showSuccess } from "$lib/stores";
  import type { PluginEntry } from "$lib/types";
  // TODO: Import from $lib/api when backend commands are ready
  // import { invoke } from "@tauri-apps/api/core";

  // ---- Types ----

  type RuleType = "load_after" | "load_before";

  interface CustomRule {
    id: string;
    plugin: string;
    ruleType: RuleType;
    reference: string;
  }

  // ---- State ----

  let plugins = $state<PluginEntry[]>([]);
  let rules = $state<CustomRule[]>([]);
  let loading = $state(false);
  let savingRule = $state(false);
  let deletingRuleId = $state<string | null>(null);

  // Add rule form state
  let showAddForm = $state(false);
  let formPlugin = $state("");
  let formRuleType = $state<RuleType>("load_after");
  let formReference = $state("");

  // Cycle detection
  let cycleWarning = $state<string | null>(null);

  const game = $derived($selectedGame);

  const availablePlugins = $derived(plugins.map(p => p.filename));

  // Filter out the selected plugin from reference options
  const referencePlugins = $derived(
    availablePlugins.filter(p => p !== formPlugin)
  );

  $effect(() => {
    if (game) {
      loadPluginsAndRules();
    }
  });

  async function loadPluginsAndRules() {
    if (!game) return;
    loading = true;
    try {
      plugins = await getPluginOrder(game.game_id, game.bottle_name);

      // TODO: Wire up when backend is ready
      // const savedRules = await invoke("get_custom_plugin_rules", {
      //   gameId: game.game_id,
      //   bottleName: game.bottle_name
      // });
      // rules = savedRules as CustomRule[];
      rules = []; // placeholder
    } catch (e: unknown) {
      showError(`Failed to load plugins: ${e}`);
    } finally {
      loading = false;
    }
  }

  function openAddForm() {
    showAddForm = true;
    formPlugin = availablePlugins[0] ?? "";
    formRuleType = "load_after";
    formReference = "";
    cycleWarning = null;
  }

  function cancelAddForm() {
    showAddForm = false;
    formPlugin = "";
    formReference = "";
    cycleWarning = null;
  }

  function detectCycle(newRule: Omit<CustomRule, "id">): boolean {
    // Build adjacency: plugin -> [plugins it must load after]
    const edges = new Map<string, Set<string>>();

    for (const rule of rules) {
      if (rule.ruleType === "load_after") {
        if (!edges.has(rule.plugin)) edges.set(rule.plugin, new Set());
        edges.get(rule.plugin)!.add(rule.reference);
      } else {
        if (!edges.has(rule.reference)) edges.set(rule.reference, new Set());
        edges.get(rule.reference)!.add(rule.plugin);
      }
    }

    // Add the proposed rule
    if (newRule.ruleType === "load_after") {
      if (!edges.has(newRule.plugin)) edges.set(newRule.plugin, new Set());
      edges.get(newRule.plugin)!.add(newRule.reference);
    } else {
      if (!edges.has(newRule.reference)) edges.set(newRule.reference, new Set());
      edges.get(newRule.reference)!.add(newRule.plugin);
    }

    // DFS cycle detection
    const visited = new Set<string>();
    const recursionStack = new Set<string>();

    function dfs(node: string): boolean {
      visited.add(node);
      recursionStack.add(node);

      const neighbors = edges.get(node);
      if (neighbors) {
        for (const neighbor of neighbors) {
          if (!visited.has(neighbor)) {
            if (dfs(neighbor)) return true;
          } else if (recursionStack.has(neighbor)) {
            return true;
          }
        }
      }

      recursionStack.delete(node);
      return false;
    }

    for (const node of edges.keys()) {
      if (!visited.has(node)) {
        if (dfs(node)) return true;
      }
    }

    return false;
  }

  async function handleSaveRule() {
    if (!game || !formPlugin || !formReference) return;
    if (formPlugin === formReference) {
      showError("Plugin and reference cannot be the same");
      return;
    }

    // Check for duplicate
    const duplicate = rules.find(
      r => r.plugin === formPlugin && r.ruleType === formRuleType && r.reference === formReference
    );
    if (duplicate) {
      showError("This rule already exists");
      return;
    }

    // Check for cycles
    const newRule = { plugin: formPlugin, ruleType: formRuleType, reference: formReference };
    if (detectCycle(newRule)) {
      cycleWarning = `Adding this rule would create a circular dependency between ${formPlugin} and ${formReference}`;
      return;
    }

    savingRule = true;
    cycleWarning = null;
    try {
      // TODO: Wire up when backend is ready
      // const ruleId = await invoke("add_custom_plugin_rule", {
      //   gameId: game.game_id,
      //   bottleName: game.bottle_name,
      //   plugin: formPlugin,
      //   ruleType: formRuleType,
      //   reference: formReference,
      // });

      const ruleId = crypto.randomUUID();
      rules = [...rules, { id: ruleId, ...newRule }];
      showSuccess("Rule added");
      cancelAddForm();
    } catch (e: unknown) {
      showError(`Failed to save rule: ${e}`);
    } finally {
      savingRule = false;
    }
  }

  async function handleDeleteRule(ruleId: string) {
    if (!game) return;
    deletingRuleId = ruleId;
    try {
      // TODO: Wire up when backend is ready
      // await invoke("remove_custom_plugin_rule", { ruleId });
      rules = rules.filter(r => r.id !== ruleId);
      showSuccess("Rule removed");
    } catch (e: unknown) {
      showError(`Failed to remove rule: ${e}`);
    } finally {
      deletingRuleId = null;
    }
  }

  function ruleTypeLabel(type: RuleType): string {
    return type === "load_after" ? "loads after" : "loads before";
  }

  function ruleTypeArrow(type: RuleType): string {
    return type === "load_after" ? "\u2192" : "\u2190";
  }
</script>

<div class="rules-panel">
  <!-- Header -->
  <div class="rules-header">
    <div class="rules-title-row">
      <h4 class="rules-title">Custom Load Order Rules</h4>
      {#if rules.length > 0}
        <span class="rules-count">{rules.length}</span>
      {/if}
    </div>
    <button
      class="btn btn-accent btn-sm"
      onclick={openAddForm}
      disabled={showAddForm || plugins.length < 2}
      type="button"
    >
      <svg width="12" height="12" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
        <line x1="7" y1="2" x2="7" y2="12" />
        <line x1="2" y1="7" x2="12" y2="7" />
      </svg>
      Add Rule
    </button>
  </div>

  <!-- Cycle Warning -->
  {#if cycleWarning}
    <div class="cycle-warning" role="alert">
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="12" cy="12" r="10" />
        <line x1="12" y1="8" x2="12" y2="12" />
        <line x1="12" y1="16" x2="12.01" y2="16" />
      </svg>
      <span>{cycleWarning}</span>
      <button
        class="warning-dismiss"
        onclick={() => cycleWarning = null}
        aria-label="Dismiss warning"
        type="button"
      >
        <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
          <line x1="2" y1="2" x2="8" y2="8" />
          <line x1="8" y1="2" x2="2" y2="8" />
        </svg>
      </button>
    </div>
  {/if}

  <!-- Add Rule Form -->
  {#if showAddForm}
    <div class="add-rule-form">
      <div class="form-row">
        <label class="form-label" for="rule-plugin">Plugin</label>
        <select
          id="rule-plugin"
          class="form-select"
          bind:value={formPlugin}
        >
          <option value="" disabled>Select plugin...</option>
          {#each availablePlugins as plugin}
            <option value={plugin}>{plugin}</option>
          {/each}
        </select>
      </div>

      <div class="form-row">
        <label class="form-label">Rule Type</label>
        <div class="segmented-control" role="radiogroup" aria-label="Rule type">
          <button
            class="segment"
            class:segment-active={formRuleType === "load_after"}
            onclick={() => formRuleType = "load_after"}
            role="radio"
            aria-checked={formRuleType === "load_after"}
            type="button"
          >
            Load After
          </button>
          <button
            class="segment"
            class:segment-active={formRuleType === "load_before"}
            onclick={() => formRuleType = "load_before"}
            role="radio"
            aria-checked={formRuleType === "load_before"}
            type="button"
          >
            Load Before
          </button>
        </div>
      </div>

      <div class="form-row">
        <label class="form-label" for="rule-reference">Reference Plugin</label>
        <select
          id="rule-reference"
          class="form-select"
          bind:value={formReference}
        >
          <option value="" disabled>Select reference plugin...</option>
          {#each referencePlugins as plugin}
            <option value={plugin}>{plugin}</option>
          {/each}
        </select>
      </div>

      {#if formPlugin && formReference}
        <div class="form-preview">
          <span class="preview-plugin">{formPlugin}</span>
          <span class="preview-arrow">{ruleTypeArrow(formRuleType)}</span>
          <span class="preview-label">{ruleTypeLabel(formRuleType)}</span>
          <span class="preview-arrow">{ruleTypeArrow(formRuleType)}</span>
          <span class="preview-plugin">{formReference}</span>
        </div>
      {/if}

      <div class="form-actions">
        <button
          class="btn btn-accent btn-sm"
          onclick={handleSaveRule}
          disabled={savingRule || !formPlugin || !formReference}
          type="button"
        >
          {#if savingRule}
            <span class="spinner-sm"></span>
            Saving...
          {:else}
            Save Rule
          {/if}
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

  <!-- Rules List -->
  {#if loading}
    <div class="rules-loading">
      <span class="spinner-sm"></span>
      <span class="loading-label">Loading...</span>
    </div>
  {:else if rules.length === 0 && !showAddForm}
    <div class="rules-empty">
      <p class="empty-text">No custom rules defined. LOOT's default sorting will be used.</p>
    </div>
  {:else}
    <div class="rules-list" role="list">
      {#each rules as rule (rule.id)}
        <div class="rule-row" role="listitem">
          <div class="rule-content">
            <span class="rule-plugin">{rule.plugin}</span>
            <span class="rule-arrow">{ruleTypeArrow(rule.ruleType)}</span>
            <span class="rule-type-label">{ruleTypeLabel(rule.ruleType)}</span>
            <span class="rule-arrow">{ruleTypeArrow(rule.ruleType)}</span>
            <span class="rule-plugin">{rule.reference}</span>
          </div>
          <button
            class="rule-delete"
            onclick={() => handleDeleteRule(rule.id)}
            disabled={deletingRuleId === rule.id}
            title="Remove rule"
            aria-label="Remove rule"
            type="button"
          >
            {#if deletingRuleId === rule.id}
              <span class="spinner-xs"></span>
            {:else}
              <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
                <line x1="2" y1="2" x2="10" y2="10" />
                <line x1="10" y1="2" x2="2" y2="10" />
              </svg>
            {/if}
          </button>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  /* ---- Panel ---- */

  .rules-panel {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  /* ---- Header ---- */

  .rules-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .rules-title-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .rules-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .rules-count {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    border-radius: 100px;
    font-size: 10px;
    font-weight: 700;
    color: var(--text-tertiary);
    background: var(--surface);
    font-variant-numeric: tabular-nums;
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

  /* ---- Cycle Warning ---- */

  .cycle-warning {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: var(--red-subtle);
    border: 1px solid var(--red-subtle);
    border-radius: var(--radius);
    color: var(--red);
    font-size: 12px;
    line-height: 1.4;
  }

  .cycle-warning svg {
    flex-shrink: 0;
  }

  .cycle-warning span {
    flex: 1;
  }

  .warning-dismiss {
    flex-shrink: 0;
    padding: var(--space-1);
    border-radius: var(--radius-sm);
    opacity: 0.6;
    transition: opacity var(--duration-fast) var(--ease);
  }

  .warning-dismiss:hover {
    opacity: 1;
  }

  /* ---- Add Rule Form ---- */

  .add-rule-form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
    background: var(--surface);
    border-radius: var(--radius);
    border: 1px solid var(--separator);
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
    background: var(--bg-base);
    border: 1px solid var(--separator-opaque);
    border-radius: var(--radius-sm);
    color: var(--text-primary);
    font-size: 13px;
    font-family: var(--font-mono);
    letter-spacing: 0;
    outline: none;
    cursor: pointer;
    transition: border-color var(--duration-fast) var(--ease);
  }

  .form-select:focus {
    border-color: var(--system-accent);
    box-shadow: 0 0 0 3px rgba(0, 122, 255, 0.15);
  }

  /* ---- Segmented Control ---- */

  .segmented-control {
    display: flex;
    background: var(--bg-secondary);
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
    box-shadow: var(--shadow-sm);
  }

  /* ---- Form Preview ---- */

  .form-preview {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: var(--bg-secondary);
    border-radius: var(--radius-sm);
    font-size: 12px;
    flex-wrap: wrap;
  }

  .preview-plugin {
    font-family: var(--font-mono);
    font-weight: 500;
    color: var(--text-primary);
    letter-spacing: 0;
  }

  .preview-arrow {
    color: var(--text-quaternary);
  }

  .preview-label {
    color: var(--system-accent);
    font-weight: 500;
  }

  .form-actions {
    display: flex;
    gap: var(--space-2);
  }

  /* ---- Loading / Empty ---- */

  .rules-loading {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3);
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

  .spinner-xs {
    display: inline-block;
    width: 10px;
    height: 10px;
    border: 1.5px solid var(--separator-opaque);
    border-top-color: var(--red);
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

  .rules-empty {
    padding: var(--space-4);
    text-align: center;
  }

  .empty-text {
    font-size: 12px;
    color: var(--text-tertiary);
    line-height: 1.5;
  }

  /* ---- Rules List ---- */

  .rules-list {
    display: flex;
    flex-direction: column;
    gap: 1px;
    background: var(--surface);
    border-radius: var(--radius);
    overflow: hidden;
  }

  .rule-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: var(--bg-grouped-secondary);
    transition: background var(--duration-fast) var(--ease);
  }

  .rule-row:hover {
    background: var(--surface-hover);
  }

  .rule-content {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    flex: 1;
    flex-wrap: wrap;
  }

  .rule-plugin {
    font-family: var(--font-mono);
    font-size: 12px;
    font-weight: 500;
    color: var(--text-primary);
    letter-spacing: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .rule-arrow {
    color: var(--text-quaternary);
    font-size: 12px;
  }

  .rule-type-label {
    font-size: 11px;
    font-weight: 500;
    color: var(--system-accent);
    white-space: nowrap;
  }

  .rule-delete {
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

  .rule-delete:hover:not(:disabled) {
    background: var(--red-subtle);
    color: var(--red);
  }

  .rule-delete:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
