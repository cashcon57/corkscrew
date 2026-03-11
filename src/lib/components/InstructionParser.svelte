<script lang="ts">
  import { selectedGame, showError, showSuccess } from "$lib/stores";
  import {
    parseInstructions,
    validateInstructionActions,
    parseInstructionsLlm,
    parseInstructionsCloud,
    checkOllamaStatus,
    getRecommendedModels,
    getCloudProviders,
    pullOllamaModel,
    deleteOllamaModel,
    unloadOllamaModel,
  } from "$lib/api";
  import type {
    ParsedInstructions,
    ValidatedAction,
    ConditionalAction,
    OllamaStatus,
    OllamaModel,
    CloudProvider,
    LlmPreference,
  } from "$lib/types";
  import { marked } from "marked";
  import DOMPurify from "dompurify";

  // Props
  let {
    rawInstructions = "",
    modNames = [] as string[],
    gameId = "",
    bottleName = "",
    platform = "wine",
    gameVersion = "",
    onActionsApplied = () => {},
  }: {
    rawInstructions: string;
    modNames: string[];
    gameId: string;
    bottleName: string;
    platform?: string;
    gameVersion?: string;
    onActionsApplied?: () => void;
  } = $props();

  // State
  let mode = $state<"auto" | "llm-setup" | "manual" | "results">("auto");
  let parsed = $state<ParsedInstructions | null>(null);
  let validated = $state<ValidatedAction[]>([]);
  let loading = $state(false);
  let llmLoading = $state(false);
  let error = $state<string | null>(null);

  // Checklist state — which actions are checked for execution
  let checkedActions = $state<Set<number>>(new Set());

  // LLM state
  let ollamaStatus = $state<OllamaStatus | null>(null);
  let recommendedModels = $state<OllamaModel[]>([]);
  let cloudProviders = $state<CloudProvider[]>([]);
  let selectedModel = $state<string>("");
  let pullingModel = $state(false);
  let pullProgress = $state("");
  let selectedProvider = $state<string>("");
  let cloudApiKey = $state("");

  // Rendered fallback (for manual mode)
  let renderedHtml = $state("");

  // Auto-parse on mount
  $effect(() => {
    if (rawInstructions && modNames.length > 0) {
      autoParse();
    }
  });

  async function autoParse() {
    loading = true;
    error = null;
    try {
      parsed = await parseInstructions(rawInstructions, modNames);
      if (parsed.actions.length > 0) {
        // Validate actions
        validated = await validateInstructionActions(parsed.actions, gameId, bottleName);
        // Auto-check all valid actions
        checkedActions = new Set(
          validated
            .map((v, i) => (v.status === "valid" ? i : -1))
            .filter((i) => i >= 0)
        );
        mode = "results";
      } else {
        // Nothing parsed deterministically — offer fallback options
        mode = "auto";
      }
    } catch (e) {
      error = `Parse failed: ${e}`;
      mode = "auto";
    } finally {
      loading = false;
    }
  }

  async function tryLlmParse(llmMode: "local" | "cloud") {
    llmLoading = true;
    error = null;
    try {
      let actions: ConditionalAction[];
      if (llmMode === "local") {
        if (!selectedModel) {
          error = "Select a model first";
          return;
        }
        actions = await parseInstructionsLlm(
          rawInstructions,
          modNames,
          selectedModel,
          platform,
          gameVersion
        );
        // Unload model after use
        try { await unloadOllamaModel(selectedModel); } catch { /* ok */ }
      } else {
        if (!selectedProvider) {
          error = "Select a provider first";
          return;
        }
        actions = await parseInstructionsCloud(
          rawInstructions,
          modNames,
          selectedProvider,
          cloudApiKey,
          platform,
          gameVersion
        );
      }

      // Merge with any existing deterministic results
      const allActions = [...(parsed?.actions ?? []), ...actions];
      validated = await validateInstructionActions(allActions, gameId, bottleName);
      checkedActions = new Set(
        validated
          .map((v, i) => (v.status === "valid" ? i : -1))
          .filter((i) => i >= 0)
      );
      mode = "results";
    } catch (e) {
      error = `LLM parse failed: ${e}`;
    } finally {
      llmLoading = false;
    }
  }

  async function showManualMode() {
    const html = await marked.parse(rawInstructions);
    renderedHtml = DOMPurify.sanitize(html);
    mode = "manual";
  }

  async function openLlmSetup() {
    mode = "llm-setup";
    // Load Ollama status and model list in parallel
    const [status, models, providers] = await Promise.all([
      checkOllamaStatus().catch(() => ({ installed: false, running: false, available_models: [] } as OllamaStatus)),
      getRecommendedModels().catch(() => []),
      getCloudProviders().catch(() => []),
    ]);
    ollamaStatus = status;
    recommendedModels = models;
    cloudProviders = providers;
    // Pre-select first available model if any
    if (status.available_models.length > 0) {
      selectedModel = status.available_models[0].name;
    }
  }

  async function handlePullModel(modelName: string) {
    pullingModel = true;
    pullProgress = `Downloading ${modelName}...`;
    try {
      await pullOllamaModel(modelName);
      pullProgress = "";
      // Refresh status
      ollamaStatus = await checkOllamaStatus();
      selectedModel = modelName;
    } catch (e) {
      error = `Failed to download model: ${e}`;
    } finally {
      pullingModel = false;
    }
  }

  async function handleDeleteModel(modelName: string) {
    try {
      await deleteOllamaModel(modelName);
      ollamaStatus = await checkOllamaStatus();
      if (selectedModel === modelName) selectedModel = "";
    } catch (e) {
      error = `Failed to delete model: ${e}`;
    }
  }

  function toggleAction(index: number) {
    const next = new Set(checkedActions);
    if (next.has(index)) {
      next.delete(index);
    } else {
      next.add(index);
    }
    checkedActions = next;
  }

  function selectAllValid() {
    checkedActions = new Set(
      validated.map((v, i) => (v.status !== "rejected" ? i : -1)).filter((i) => i >= 0)
    );
  }

  function deselectAll() {
    checkedActions = new Set();
  }

  function getActionLabel(action: ConditionalAction): string {
    const a = action.action;
    switch (a.type) {
      case "enable_mod": return `Enable "${a.mod_name}"`;
      case "disable_mod": return `Disable "${a.mod_name}"`;
      case "enable_all_optional": return "Enable all optional mods";
      case "disable_all_optional": return "Disable all optional mods";
      case "set_fomod_choice": return `FOMOD: ${a.mod_name} → ${a.option}`;
      case "set_ini_setting": return `INI: [${a.section}] ${a.key}=${a.value} in ${a.file}`;
      case "set_load_order": return `Load order: ${a.plugin} → ${a.position.type}`;
      case "manual_step": return `Manual: ${a.description}`;
      default: return "Unknown action";
    }
  }

  function getConditionLabel(action: ConditionalAction): string | null {
    const c = action.condition;
    if (c.type === "always") return null;
    switch (c.type) {
      case "game_version":
        return typeof c.version === "string"
          ? `${c.version.toUpperCase()} only`
          : `Version: ${c.version.pattern}`;
      case "platform": return `${c.platform.replace(/_/g, "/")} only`;
      case "dlc_present": return `Requires DLC: ${c.dlc_name}`;
      case "mod_installed": return `If ${c.mod_name} installed`;
      case "mod_not_installed": return `If ${c.mod_name} not installed`;
      default: return "Conditional";
    }
  }

  function statusIcon(status: string): string {
    switch (status) {
      case "valid": return "check";
      case "needs_confirmation": return "alert";
      case "rejected": return "x";
      default: return "?";
    }
  }
</script>

<div class="instruction-parser">
  {#if loading}
    <div class="loading-state">
      <div class="spinner"></div>
      <span>Parsing instructions...</span>
    </div>
  {:else if mode === "auto" && (!parsed || parsed.actions.length === 0)}
    <!-- Deterministic parse found nothing (or partial) — offer options -->
    <div class="fallback-options">
      <div class="fallback-header">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="10" />
          <line x1="12" y1="16" x2="12" y2="12" />
          <line x1="12" y1="8" x2="12.01" y2="8" />
        </svg>
        <h3>Install Instructions</h3>
      </div>
      {#if parsed && parsed.unparsed_lines.length > 0}
        <p class="fallback-description">
          Some instructions couldn't be parsed automatically ({parsed.unparsed_lines.length} line{parsed.unparsed_lines.length === 1 ? '' : 's'}).
          Choose how to handle them:
        </p>
      {:else}
        <p class="fallback-description">
          Instructions are available but couldn't be parsed automatically. Choose how to handle them:
        </p>
      {/if}

      <div class="fallback-choices">
        <button class="choice-btn llm-btn" onclick={openLlmSetup}>
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 2a4 4 0 0 1 4 4v1h1a3 3 0 0 1 3 3v1a3 3 0 0 1-3 3h-1v4a4 4 0 0 1-8 0v-4H7a3 3 0 0 1-3-3v-1a3 3 0 0 1 3-3h1V6a4 4 0 0 1 4-4z"/>
          </svg>
          <div class="choice-text">
            <strong>AI-Assisted Parsing</strong>
            <span>Use a local or cloud LLM to parse automatically</span>
          </div>
        </button>

        <button class="choice-btn manual-btn" onclick={showManualMode}>
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/>
            <polyline points="14 2 14 8 20 8"/>
            <line x1="16" y1="13" x2="8" y2="13"/>
            <line x1="16" y1="17" x2="8" y2="17"/>
          </svg>
          <div class="choice-text">
            <strong>Read & Follow Manually</strong>
            <span>View raw instructions and apply changes yourself</span>
          </div>
        </button>
      </div>
    </div>

  {:else if mode === "llm-setup"}
    <!-- LLM setup panel -->
    <div class="llm-setup">
      <div class="llm-setup-header">
        <button class="back-btn" onclick={() => mode = "auto"}>
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="15 18 9 12 15 6"/></svg>
        </button>
        <h3>AI-Assisted Parsing</h3>
      </div>

      {#if error}
        <div class="error-banner">{error}</div>
      {/if}

      <!-- Local LLM Section -->
      <div class="llm-section">
        <h4>Local LLM (via Ollama)</h4>
        {#if !ollamaStatus?.installed}
          <p class="hint">Ollama is not installed. <a href="https://ollama.com" target="_blank" rel="noopener">Install Ollama</a> to use local models.</p>
        {:else if !ollamaStatus?.running}
          <p class="hint">Ollama is installed but not running. Start it to use local models.</p>
        {:else}
          <div class="model-list">
            {#each recommendedModels as model}
              {@const installed = ollamaStatus?.available_models.some(m => m.name === model.name)}
              <div class="model-row" class:selected={selectedModel === model.name}>
                <div class="model-info">
                  <span class="model-name">{model.name}</span>
                  <span class="model-size">{model.size_display}</span>
                  <div class="model-desc">{model.description}</div>
                  <div class="model-accuracy">
                    <div class="accuracy-bar">
                      <div class="accuracy-fill" style="width: {model.expected_accuracy * 100}%"></div>
                    </div>
                    <span class="accuracy-label">{Math.round(model.expected_accuracy * 100)}% accuracy</span>
                  </div>
                </div>
                <div class="model-actions">
                  {#if installed}
                    <button
                      class="btn-sm btn-primary"
                      class:active={selectedModel === model.name}
                      onclick={() => selectedModel = model.name}
                    >
                      {selectedModel === model.name ? "Selected" : "Select"}
                    </button>
                    <button class="btn-sm btn-danger" onclick={() => handleDeleteModel(model.name)}>Delete</button>
                  {:else}
                    <button
                      class="btn-sm btn-secondary"
                      disabled={pullingModel}
                      onclick={() => handlePullModel(model.name)}
                    >
                      {pullingModel ? "..." : "Download"}
                    </button>
                  {/if}
                </div>
              </div>
            {/each}
          </div>
          {#if pullProgress}
            <p class="pull-progress">{pullProgress}</p>
          {/if}
          <button
            class="btn-action"
            disabled={!selectedModel || llmLoading}
            onclick={() => tryLlmParse("local")}
          >
            {llmLoading ? "Parsing..." : `Parse with ${selectedModel || "..."}`}
          </button>
        {/if}
      </div>

      <!-- Cloud LLM Section -->
      <div class="llm-section">
        <h4>Free Cloud LLM</h4>
        <div class="provider-list">
          {#each cloudProviders as provider}
            <label class="provider-row">
              <input type="radio" name="cloud-provider" value={provider.name}
                bind:group={selectedProvider} />
              <div class="provider-info">
                <strong>{provider.display_name}</strong>
                <span class="provider-desc">{provider.description}</span>
                <span class="provider-free">{provider.free_tier_info}</span>
              </div>
            </label>
          {/each}
        </div>
        {#if selectedProvider}
          {#if cloudProviders.find(p => p.name === selectedProvider)?.requires_api_key}
            <input
              type="password"
              class="api-key-input"
              placeholder="API Key"
              bind:value={cloudApiKey}
            />
          {/if}
          <button
            class="btn-action"
            disabled={llmLoading}
            onclick={() => tryLlmParse("cloud")}
          >
            {llmLoading ? "Parsing..." : "Parse with Cloud LLM"}
          </button>
        {/if}
      </div>

      <button class="btn-text" onclick={showManualMode}>
        Or read instructions manually
      </button>
    </div>

  {:else if mode === "manual"}
    <!-- Manual mode: rendered markdown + back button -->
    <div class="manual-mode">
      <div class="manual-header">
        <button class="back-btn" onclick={() => mode = "auto"}>
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="15 18 9 12 15 6"/></svg>
        </button>
        <h3>Install Instructions</h3>
        <button class="btn-text" onclick={openLlmSetup}>Try AI parsing</button>
      </div>
      <div class="rendered-markdown install-instructions-content">
        {@html renderedHtml}
      </div>
    </div>

  {:else if mode === "results"}
    <!-- Action checklist -->
    <div class="action-results">
      <div class="results-header">
        <h3>
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M9 11l3 3L22 4"/>
            <path d="M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11"/>
          </svg>
          Parsed Actions
          <span class="action-count">{validated.length}</span>
        </h3>
        <div class="results-controls">
          <button class="btn-text" onclick={selectAllValid}>Select all</button>
          <button class="btn-text" onclick={deselectAll}>Deselect all</button>
        </div>
      </div>

      {#if parsed && parsed.unparsed_lines.length > 0}
        <div class="unparsed-notice">
          <span>{parsed.unparsed_lines.length} line{parsed.unparsed_lines.length === 1 ? '' : 's'} not parsed</span>
          <button class="btn-text" onclick={openLlmSetup}>Try AI</button>
          <button class="btn-text" onclick={showManualMode}>View raw</button>
        </div>
      {/if}

      <div class="action-list">
        {#each validated as va, i}
          <div
            class="action-item"
            class:valid={va.status === "valid"}
            class:needs-confirm={va.status === "needs_confirmation"}
            class:rejected={va.status === "rejected"}
          >
            <span
              class="action-check"
              role="checkbox"
              aria-checked={checkedActions.has(i)}
              tabindex="0"
              onclick={() => toggleAction(i)}
              onkeydown={(e) => { if (e.key === ' ' || e.key === 'Enter') toggleAction(i); }}
            >
              <span class="check-box" class:check-box-checked={checkedActions.has(i)}></span>
            </span>
            <div class="action-body">
              <span class="action-label">{getActionLabel(va.action)}</span>
              {#if getConditionLabel(va.action)}
                <span class="action-condition">{getConditionLabel(va.action)}</span>
              {/if}
              {#if va.reason}
                <span class="action-reason">{va.reason}</span>
              {/if}
              {#if va.status === "rejected"}
                <span class="action-rejected-tag">Rejected</span>
              {/if}
            </div>
            <span class="action-status status-{va.status}">
              {#if va.status === "valid"}
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="20 6 9 17 4 12"/></svg>
              {:else if va.status === "needs_confirmation"}
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/></svg>
              {:else}
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
              {/if}
            </span>
          </div>
        {/each}
      </div>

      {#if parsed?.source}
        <div class="source-tag">
          Parsed by: {parsed.source.type === "deterministic" ? "Auto" : parsed.source.type === "local_llm" ? `Local LLM (${(parsed.source as any).model})` : parsed.source.type === "cloud_llm" ? `Cloud (${(parsed.source as any).provider})` : "Manual"}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .instruction-parser {
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface);
    overflow: hidden;
  }

  .loading-state {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 16px;
    color: var(--text-muted);
  }

  .spinner {
    width: 16px;
    height: 16px;
    border: 2px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  /* Fallback options */
  .fallback-options {
    padding: 16px;
  }

  .fallback-header {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 8px;
    color: var(--accent);
  }

  .fallback-header h3 {
    margin: 0;
    font-size: 14px;
  }

  .fallback-description {
    font-size: 13px;
    color: var(--text-muted);
    margin: 0 0 12px;
  }

  .fallback-choices {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .choice-btn {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface-hover);
    cursor: pointer;
    text-align: left;
    transition: border-color 0.15s, background 0.15s;
  }

  .choice-btn:hover {
    border-color: var(--accent);
    background: var(--surface-active);
  }

  .choice-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .choice-text strong {
    font-size: 13px;
    color: var(--text);
  }

  .choice-text span {
    font-size: 12px;
    color: var(--text-muted);
  }

  /* LLM setup */
  .llm-setup {
    padding: 16px;
  }

  .llm-setup-header, .manual-header {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 12px;
  }

  .llm-setup-header h3, .manual-header h3 {
    margin: 0;
    font-size: 14px;
    flex: 1;
  }

  .back-btn {
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-muted);
    padding: 4px;
    border-radius: 4px;
  }

  .back-btn:hover {
    background: var(--surface-hover);
  }

  .llm-section {
    margin-bottom: 16px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: 8px;
  }

  .llm-section h4 {
    margin: 0 0 8px;
    font-size: 13px;
    color: var(--text);
  }

  .hint {
    font-size: 12px;
    color: var(--text-muted);
    margin: 0;
  }

  .hint a {
    color: var(--accent);
  }

  .model-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-bottom: 8px;
  }

  .model-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    font-size: 12px;
    transition: border-color 0.15s;
  }

  .model-row.selected {
    border-color: var(--accent);
    background: rgba(var(--accent-rgb, 59, 130, 246), 0.05);
  }

  .model-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
  }

  .model-name {
    font-weight: 600;
    color: var(--text);
  }

  .model-size {
    color: var(--text-muted);
  }

  .model-desc {
    color: var(--text-muted);
    font-size: 11px;
  }

  .model-accuracy {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 4px;
  }

  .accuracy-bar {
    width: 60px;
    height: 4px;
    background: var(--border);
    border-radius: 2px;
    overflow: hidden;
  }

  .accuracy-fill {
    height: 100%;
    background: var(--accent);
    border-radius: 2px;
  }

  .accuracy-label {
    font-size: 10px;
    color: var(--text-muted);
  }

  .model-actions {
    display: flex;
    gap: 4px;
    flex-shrink: 0;
  }

  .btn-sm {
    padding: 4px 8px;
    font-size: 11px;
    border: 1px solid var(--border);
    border-radius: 4px;
    cursor: pointer;
    background: var(--surface);
    color: var(--text);
  }

  .btn-sm.btn-primary {
    background: var(--accent);
    color: white;
    border-color: var(--accent);
  }

  .btn-sm.btn-primary.active {
    opacity: 0.7;
  }

  .btn-sm.btn-danger {
    color: var(--danger, #ef4444);
    border-color: var(--danger, #ef4444);
  }

  .btn-sm.btn-secondary {
    background: var(--surface-hover);
  }

  .btn-action {
    width: 100%;
    padding: 8px;
    margin-top: 8px;
    font-size: 13px;
    font-weight: 500;
    border: none;
    border-radius: 6px;
    background: var(--accent);
    color: white;
    cursor: pointer;
    transition: opacity 0.15s;
  }

  .btn-action:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-text {
    background: none;
    border: none;
    cursor: pointer;
    color: var(--accent);
    font-size: 12px;
    padding: 4px;
  }

  .btn-text:hover {
    text-decoration: underline;
  }

  .provider-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-bottom: 8px;
  }

  .provider-row {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    cursor: pointer;
    font-size: 12px;
  }

  .provider-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .provider-desc {
    color: var(--text-muted);
    font-size: 11px;
  }

  .provider-free {
    color: var(--success, #22c55e);
    font-size: 11px;
  }

  .api-key-input {
    width: 100%;
    padding: 6px 8px;
    font-size: 12px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--surface);
    color: var(--text);
    margin-bottom: 4px;
  }

  .pull-progress {
    font-size: 11px;
    color: var(--text-muted);
    margin: 4px 0;
  }

  .error-banner {
    padding: 8px 12px;
    margin-bottom: 8px;
    font-size: 12px;
    background: rgba(239, 68, 68, 0.1);
    color: var(--danger, #ef4444);
    border-radius: 6px;
  }

  /* Manual mode */
  .manual-mode {
    padding: 16px;
  }

  .install-instructions-content {
    font-size: 13px;
    line-height: 1.6;
    color: var(--text);
  }

  /* Results */
  .action-results {
    padding: 16px;
  }

  .results-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 8px;
  }

  .results-header h3 {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    font-size: 14px;
    color: var(--text);
  }

  .action-count {
    background: var(--accent);
    color: white;
    font-size: 11px;
    padding: 1px 6px;
    border-radius: 10px;
  }

  .results-controls {
    display: flex;
    gap: 8px;
  }

  .unparsed-notice {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    margin-bottom: 8px;
    font-size: 12px;
    color: var(--text-muted);
    background: rgba(234, 179, 8, 0.08);
    border-radius: 6px;
  }

  .action-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .action-item {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 8px;
    border-radius: 6px;
    border: 1px solid var(--border);
    transition: background 0.1s;
  }

  .action-item:hover {
    background: var(--surface-hover);
  }

  .action-item.rejected {
    opacity: 0.6;
  }

  .action-check {
    cursor: pointer;
    flex-shrink: 0;
    padding-top: 1px;
  }

  .check-box {
    display: inline-block;
    width: 16px;
    height: 16px;
    border: 2px solid var(--border);
    border-radius: 4px;
    transition: all 0.15s;
  }

  .check-box-checked {
    background: var(--accent);
    border-color: var(--accent);
    background-image: url("data:image/svg+xml,%3Csvg viewBox='0 0 16 16' fill='white' xmlns='http://www.w3.org/2000/svg'%3E%3Cpath d='M12.207 4.793a1 1 0 010 1.414l-5 5a1 1 0 01-1.414 0l-2-2a1 1 0 011.414-1.414L6.5 9.086l4.293-4.293a1 1 0 011.414 0z'/%3E%3C/svg%3E");
  }

  .action-body {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .action-label {
    font-size: 13px;
    color: var(--text);
  }

  .action-condition {
    font-size: 11px;
    color: var(--accent);
    font-style: italic;
  }

  .action-reason {
    font-size: 11px;
    color: var(--text-muted);
  }

  .action-rejected-tag {
    font-size: 10px;
    color: var(--danger, #ef4444);
    font-weight: 600;
    text-transform: uppercase;
  }

  .action-status {
    flex-shrink: 0;
    padding-top: 1px;
  }

  .status-valid { color: var(--success, #22c55e); }
  .status-needs_confirmation { color: var(--warning, #eab308); }
  .status-rejected { color: var(--danger, #ef4444); }

  .source-tag {
    margin-top: 8px;
    font-size: 11px;
    color: var(--text-muted);
    text-align: right;
  }
</style>
