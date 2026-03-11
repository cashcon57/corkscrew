<script lang="ts">
  import { onDestroy } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { selectedGame, showError, showSuccess, currentPage, installedMods } from "$lib/stores";
  import { getInstalledMods, startOllama, checkMlxStatus, installMlx, deleteModel, getCachedMlxModels } from "$lib/api";
  import type { LlmBackend } from "$lib/types";
  import {
    chatGetState,
    chatLoadModel,
    chatUnloadModel,
    chatSendMessage,
    chatClearHistory,
    checkOllamaStatus,
    getRecommendedModels,
    pullOllamaModel,
    getSystemMemory,
    installOllama,
    getRecommendedModel,
  } from "$lib/api";
  import type { ChatMessage, ChatState, OllamaModel, OllamaStatus, ChatResponse } from "$lib/types";

  let {
    visible = false,
    onclose = () => {},
  }: {
    visible: boolean;
    onclose: () => void;
  } = $props();

  let chatState = $state<ChatState | null>(null);
  let ollamaStatus = $state<OllamaStatus | null>(null);
  let recommendedModels = $state<OllamaModel[]>([]);
  let recommendedModelName = $state<string | null>(null);
  let systemMemoryGB = $state<number | null>(null);
  let inputText = $state("");
  let sending = $state(false);
  let loading = $state(false);
  let loadingModelName = $state<string | null>(null);
  let pullingModel = $state<string | null>(null);
  let installing = $state(false);
  let startingOllama = $state(false);
  let ollamaStartFailed = $state(false);
  let mlxAvailable = $state(false);
  let cachedMlxModels = $state<string[]>([]);
  let selectedBackend = $state<LlmBackend>("mlx");
  let installingMlx = $state(false);
  let showModelPicker = $state(false);
  let customModelName = $state("");
  let autoUnloadTimer = $state<ReturnType<typeof setTimeout> | null>(null);
  let messagesDiv: HTMLDivElement | undefined = $state();

  // Streaming state
  let isStreaming = $state(false);
  let settledText = $state("");
  let recentTokens = $state<string[]>([]);
  let streamHasContent = $derived(settledText.length > 0 || recentTokens.length > 0);
  let streamPhase = $state<"idle" | "thinking" | "content" | "tools">("idle");
  let thinkingDots = $state(0);
  let thinkingTimer: ReturnType<typeof setInterval> | null = null;

  // Listen for streaming tokens from backend
  let unlistenStream: (() => void) | null = null;
  let unlistenToolStatus: (() => void) | null = null;
  let toolStatusText = $state("");

  listen<{ tool_name: string; status: string; display_text: string }>("chat-tool-status", (event) => {
    if (event.payload.status === "running") {
      streamPhase = "tools";
      toolStatusText = event.payload.display_text;
    } else if (event.payload.status === "complete") {
      toolStatusText = "";
    }
  }).then((unlisten) => { unlistenToolStatus = unlisten; });
  // Queue of individual characters waiting to be revealed
  let charQueue: string[] = [];
  let charTimerId: ReturnType<typeof setTimeout> | null = null;
  let streamStarted = false; // true once first non-whitespace char seen
  const CHAR_INTERVAL_MS = 12; // ms between each character reveal

  function drainCharQueue() {
    if (charQueue.length === 0) {
      charTimerId = null;
      return;
    }
    const ch = charQueue.shift()!;
    // Skip leading whitespace/newlines before any real content
    if (!streamStarted) {
      if (ch.trim() === "") {
        charTimerId = setTimeout(drainCharQueue, 0);
        return;
      }
      streamStarted = true;
    }
    recentTokens = [...recentTokens, ch];
    // Settle old chars to keep DOM light (keep last 40 animated)
    if (recentTokens.length > 50) {
      settledText += recentTokens.slice(0, recentTokens.length - 40).join("");
      recentTokens = recentTokens.slice(recentTokens.length - 40);
    }
    // Smooth auto-scroll
    if (messagesDiv) {
      messagesDiv.scrollTo({ top: messagesDiv.scrollHeight, behavior: "smooth" });
    }
    charTimerId = setTimeout(drainCharQueue, CHAR_INTERVAL_MS);
  }

  listen<{ text: string; phase: string }>("chat-stream-token", (event) => {
    if (isStreaming) {
      const { text, phase } = event.payload;
      if (phase === "thinking") {
        // Model is in thinking/reasoning mode — show indicator, don't queue text
        if (streamPhase !== "thinking") {
          streamPhase = "thinking";
          thinkingDots = 0;
          if (!thinkingTimer) {
            thinkingTimer = setInterval(() => { thinkingDots = (thinkingDots + 1) % 4; }, 400);
          }
        }
      } else {
        // Content phase — stream characters to user
        if (streamPhase === "thinking" && thinkingTimer) {
          clearInterval(thinkingTimer);
          thinkingTimer = null;
        }
        streamPhase = "content";
        for (const ch of text) {
          charQueue.push(ch);
        }
        if (charTimerId === null) {
          drainCharQueue();
        }
      }
    }
  }).then((unlisten) => { unlistenStream = unlisten; });

  onDestroy(() => {
    if (unlistenStream) unlistenStream();
    if (unlistenToolStatus) unlistenToolStatus();
    if (autoUnloadTimer) clearTimeout(autoUnloadTimer);
    if (charTimerId) clearTimeout(charTimerId);
    if (thinkingTimer) clearInterval(thinkingTimer);
  });

  // Current page name from store
  let currentPageName = $derived($currentPage || "Mods");

  // Display messages (skip system, skip empty assistant messages from tool-call-only turns)
  let displayMessages = $derived(
    chatState?.messages.filter((m) => {
      if (m.role === "system") return false;
      if (m.role === "assistant" && !m.content?.trim()) return false;
      return true;
    }) ?? []
  );

  $effect(() => {
    if (visible) {
      loadState();
    }
  });

  // Auto-scroll on new messages
  $effect(() => {
    if (displayMessages.length > 0 && messagesDiv) {
      requestAnimationFrame(() => {
        if (messagesDiv) messagesDiv.scrollTop = messagesDiv.scrollHeight;
      });
    }
  });

  async function loadState() {
    try {
      const [state, status, models, memBytes, recModel, hasMlx, mlxCached] = await Promise.all([
        chatGetState(),
        checkOllamaStatus().catch(() => ({ installed: false, running: false, available_models: [] }) as OllamaStatus),
        getRecommendedModels().catch(() => []),
        getSystemMemory().catch(() => 0),
        getRecommendedModel().catch(() => null),
        checkMlxStatus().catch(() => false),
        getCachedMlxModels().catch(() => [] as string[]),
      ]);
      chatState = state;
      ollamaStatus = status;
      recommendedModels = models;
      recommendedModelName = recModel;
      systemMemoryGB = memBytes > 0 ? Math.round(memBytes / (1024 * 1024 * 1024)) : null;
      mlxAvailable = hasMlx;
      cachedMlxModels = mlxCached;
      // Restore backend from active session, otherwise keep default (mlx)
      if (state.loaded) selectedBackend = state.backend;
      if (!state.loaded) showModelPicker = true;
    } catch (e) {
      showError(`${e}`);
    }
  }

  function resetAutoUnload() {
    if (autoUnloadTimer) clearTimeout(autoUnloadTimer);
    autoUnloadTimer = setTimeout(async () => {
      if (chatState?.loaded) {
        await chatUnloadModel();
        chatState = { ...chatState!, model: null, loaded: false, messages: [], available_models: chatState!.available_models };
      }
    }, 5 * 60 * 1000);
  }

  async function handleLoadModel(modelName: string) {
    const game = $selectedGame;
    if (!game) return;
    loading = true;
    loadingModelName = modelName;
    try {
      await chatLoadModel(modelName, game.game_id, game.bottle_name, currentPageName, selectedBackend);
      chatState = await chatGetState();
      showModelPicker = false;
      resetAutoUnload();
    } catch (e) {
      showError(`Failed to load model: ${e}`);
    } finally {
      loading = false;
      loadingModelName = null;
    }
  }

  async function handleUnload() {
    try {
      await chatUnloadModel();
      chatState = await chatGetState();
      showModelPicker = true;
      if (autoUnloadTimer) { clearTimeout(autoUnloadTimer); autoUnloadTimer = null; }
    } catch (e) {
      showError(`${e}`);
    }
  }

  let deletingModel = $state<string | null>(null);

  async function handleDeleteModel(modelName: string, backend: string) {
    if (!confirm(`Delete ${modelName}? This removes the model files from disk.`)) return;
    deletingModel = modelName;
    try {
      // Unload first if it's the active model
      if (chatState?.model === modelName) {
        await chatUnloadModel();
        chatState = await chatGetState();
        showModelPicker = true;
      }
      const msg = await deleteModel(modelName, backend);
      showSuccess(msg);
      // Refresh ollama model list
      if (backend === "ollama") {
        ollamaStatus = await checkOllamaStatus().catch(() => ollamaStatus);
      }
    } catch (e) {
      showError(`${e}`);
    } finally {
      deletingModel = null;
    }
  }

  async function handlePull(modelName: string) {
    pullingModel = modelName;
    try {
      await pullOllamaModel(modelName);
      ollamaStatus = await checkOllamaStatus();
    } catch (e) {
      showError(`${e}`);
    } finally {
      pullingModel = null;
    }
  }

  async function handleInstallOllama() {
    installing = true;
    try {
      const msg = await installOllama();
      showSuccess(msg);
      // Re-check status after install
      setTimeout(async () => {
        ollamaStatus = await checkOllamaStatus().catch(() => ({ installed: false, running: false, available_models: [] }) as OllamaStatus);
        installing = false;
      }, 3000);
    } catch (e) {
      showError(`${e}`);
      installing = false;
    }
  }

  async function tryStartOllama() {
    startingOllama = true;
    ollamaStartFailed = false;
    try {
      await startOllama();
      await loadState();
    } catch (e) {
      ollamaStartFailed = true;
    } finally {
      startingOllama = false;
    }
  }

  async function handleInstallMlx() {
    installingMlx = true;
    try {
      const msg = await installMlx();
      showSuccess(msg);
      mlxAvailable = true;
    } catch (e) {
      showError(`Failed to install MLX: ${e}`);
    } finally {
      installingMlx = false;
    }
  }

  // Auto-start Ollama when installed but not running (only when Ollama backend selected)
  $effect(() => {
    if (visible && selectedBackend === "ollama" && ollamaStatus && ollamaStatus.installed && !ollamaStatus.running && !startingOllama && !ollamaStartFailed) {
      tryStartOllama();
    }
  });

  async function handleSend() {
    if (!inputText.trim() || sending) return;
    const game = $selectedGame;
    if (!game) return;

    const msg = inputText.trim();
    inputText = "";
    sending = true;
    settledText = "";
    recentTokens = [];
    streamStarted = false;
    isStreaming = true;
    streamPhase = "idle";
    thinkingDots = 0;
    resetAutoUnload();

    if (chatState) {
      chatState = {
        ...chatState,
        messages: [...chatState.messages, { role: "user", content: msg }],
      };
    }

    try {
      const resp: ChatResponse = await chatSendMessage(msg, game.game_id, game.bottle_name, currentPageName);
      isStreaming = false;
      streamPhase = "idle";
      if (thinkingTimer) { clearInterval(thinkingTimer); thinkingTimer = null; }
      charQueue = [];
      if (charTimerId) { clearTimeout(charTimerId); charTimerId = null; }
      settledText = "";
      recentTokens = [];
      chatState = await chatGetState();

      // Detect empty response (model returned no content and no tool calls)
      const hasContent = resp.message?.content?.trim();
      const hasToolCalls = resp.tool_results && resp.tool_results.length > 0;
      if (!hasContent && !hasToolCalls) {
        showError("No response received. The model may need more context space — try a shorter prompt or restart the model.");
      }

      // If any tool modified state, refresh the mods list so the UI updates
      const modifyingTools = ["enable_mod", "disable_mod", "sort_load_order", "activate_profile", "redeploy_mods", "download_and_install_mod"];
      if (resp.tool_results?.some(tr => modifyingTools.includes(tr.tool_name))) {
        try {
          const mods = await getInstalledMods(game.game_id, game.bottle_name);
          installedMods.set(mods);
        } catch (_) { /* non-critical */ }
      }
    } catch (e) {
      showError(`Chat error: ${e}`);
    } finally {
      sending = false;
      isStreaming = false;
      streamPhase = "idle";
      if (thinkingTimer) { clearInterval(thinkingTimer); thinkingTimer = null; }
      charQueue = [];
      if (charTimerId) { clearTimeout(charTimerId); charTimerId = null; }
      settledText = "";
      recentTokens = [];
    }
  }

  async function handleClear() {
    await chatClearHistory();
    chatState = await chatGetState();
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  }

  let isOpMode = $derived(systemMemoryGB !== null && systemMemoryGB >= 64);

  function formatMemory(gb: number): string {
    return `${gb} GB`;
  }

  /** Map Ollama model name to MLX HuggingFace ID (mirrors Rust mlx_model_name) */
  const mlxModelMap: Record<string, string> = {
    "qwen3:32b": "mlx-community/Qwen3.5-27B-4bit",
    "qwen3:30b-a3b": "mlx-community/Qwen3.5-35B-A3B-4bit",
    "qwen3:8b": "mlx-community/Qwen3.5-9B-4bit",
    "qwen3:4b": "mlx-community/Qwen3.5-4B-4bit",
    "qwen3:1.7b": "mlx-community/Qwen3.5-2B-4bit",
  };

  function isMlxModelCached(modelName: string): boolean {
    const hfId = mlxModelMap[modelName] ?? modelName;
    return cachedMlxModels.includes(hfId);
  }

  /** Simple markdown-ish rendering for assistant messages: bold, bullet lists, code */
  function renderMarkdown(text: string): string {
    if (!text) return "";
    return text.trim()
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>")
      .replace(/`([^`]+)`/g, '<code class="inline-code">$1</code>')
      .replace(/^[-•]\s+(.+)$/gm, '<span class="md-bullet">• $1</span>')
      .replace(/^\d+\.\s+(.+)$/gm, '<span class="md-bullet">$&</span>');
  }
</script>

{#if visible}
  <div class="chat-container">
    <!-- Header bar -->
    <div class="chat-header">
      <span class="chat-header-label">
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>
        </svg>
        {#if chatState?.model}
          <span class="header-model">{chatState.model}</span>
        {:else}
          Mod Assistant
        {/if}
      </span>
      <div class="chat-header-actions">
        {#if chatState?.loaded}
          <button class="hdr-btn" title="Switch model" onclick={() => showModelPicker = !showModelPicker}>
            <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>
          </button>
          <button class="hdr-btn" title="Clear history" onclick={handleClear}>
            <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
          </button>
          <button class="hdr-btn" title="Unload model" onclick={handleUnload}>
            <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M18.36 6.64A9 9 0 1 1 5.64 5.64"/><line x1="12" y1="2" x2="12" y2="12"/></svg>
          </button>
        {/if}
        <button class="hdr-btn" title="Close" onclick={onclose}>
          <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
        </button>
      </div>
    </div>

    <!-- Setup / Model Picker -->
    {#if showModelPicker && !chatState?.loaded}
      <div class="setup-area">
        <!-- Backend toggle -->
        <div class="backend-toggle">
          <button
            class="backend-btn"
            class:backend-active={selectedBackend === "mlx"}
            onclick={() => selectedBackend = "mlx"}
          >MLX</button>
          <button
            class="backend-btn"
            class:backend-active={selectedBackend === "ollama"}
            onclick={() => selectedBackend = "ollama"}
          >Ollama</button>
        </div>

        {#if selectedBackend === "mlx"}
          <!-- MLX backend -->
          {#if !mlxAvailable}
            <div class="setup-message">
              <p>MLX LM is required for Apple Silicon inference.</p>
              <button class="setup-btn" onclick={handleInstallMlx} disabled={installingMlx}>
                {#if installingMlx}
                  <span class="mini-spinner"></span> Installing...
                {:else}
                  Install MLX LM
                {/if}
              </button>
              <p class="setup-hint">Installs via pip3 (requires Python 3).</p>
            </div>
          {:else}
            {#if systemMemoryGB}
              <div class="memory-info">{formatMemory(systemMemoryGB)} unified memory{#if isOpMode} <span class="op-badge">OP Mode</span>{/if}</div>
            {/if}
            <div class="model-list">
              {#each recommendedModels as model}
                {@const isRecommended = model.name === recommendedModelName}
                {@const fitsMemory = systemMemoryGB ? (model.min_memory_bytes / (1024*1024*1024)) <= systemMemoryGB : true}
                <div class="model-row" class:model-recommended={isRecommended} class:model-too-large={!fitsMemory}>
                  <div class="model-info">
                    <div class="model-name-row">
                      <span class="model-name">{model.name}</span>
                      {#if isRecommended}
                        <span class="rec-badge">Best fit</span>
                      {/if}
                      {#if !fitsMemory}
                        <span class="too-large-badge">Too large</span>
                      {/if}
                    </div>
                    <span class="model-meta">{model.size_display} &middot; {model.description}</span>
                  </div>
                  <div class="model-actions">
                    {#if isMlxModelCached(model.name)}
                      <button
                        class="model-btn model-btn-load"
                        disabled={loading || !fitsMemory}
                        onclick={() => handleLoadModel(model.name)}
                      >
                        {loadingModelName === model.name ? "Loading..." : "Load"}
                      </button>
                    {:else}
                      <button
                        class="model-btn model-btn-dl"
                        disabled={loading || !fitsMemory}
                        onclick={() => handleLoadModel(model.name)}
                      >
                        {#if loadingModelName === model.name}
                          <span class="mini-spinner"></span> Downloading...
                        {:else}
                          Download &amp; Load ({model.size_display})
                        {/if}
                      </button>
                    {/if}
                  </div>
                </div>
              {/each}
              <!-- Custom MLX model input -->
              <div class="custom-model-row">
                <input
                  class="custom-model-input"
                  type="text"
                  placeholder="HuggingFace model (e.g. mlx-community/Qwen3.5-9B-4bit)"
                  bind:value={customModelName}
                  onkeydown={(e) => { if (e.key === "Enter" && customModelName.trim()) { handleLoadModel(customModelName.trim()); customModelName = ""; } }}
                />
                {#if customModelName.trim()}
                  <button
                    class="model-btn model-btn-dl"
                    disabled={loading}
                    onclick={() => { handleLoadModel(customModelName.trim()); customModelName = ""; }}
                  >
                    Download &amp; Load
                  </button>
                {/if}
              </div>
            </div>
          {/if}
        {:else}
          <!-- Ollama backend -->
          {#if !ollamaStatus?.installed}
            <div class="setup-message">
              <p>Ollama is required to run local AI models.</p>
              <button class="setup-btn" onclick={handleInstallOllama} disabled={installing}>
                {#if installing}
                  <span class="mini-spinner"></span> Installing...
                {:else}
                  Install Ollama
                {/if}
              </button>
              <p class="setup-hint">
                {#if navigator.platform?.includes("Mac")}
                  Opens the Ollama download page.
                {:else}
                  Runs the official install script.
                {/if}
              </p>
            </div>
          {:else if !ollamaStatus?.running}
            <div class="setup-message">
              {#if startingOllama}
                <span class="mini-spinner"></span>
                <p>Starting Ollama...</p>
              {:else if ollamaStartFailed}
                <p>Could not start Ollama automatically.</p>
                <p class="setup-hint">Launch Ollama from your Applications, then click retry.</p>
                <button class="setup-btn" onclick={() => { ollamaStartFailed = false; tryStartOllama(); }}>Retry</button>
              {:else}
                <p>Ollama is installed but not running.</p>
                <button class="setup-btn" onclick={tryStartOllama}>Start Ollama</button>
              {/if}
            </div>
          {:else}
            {#if systemMemoryGB}
              <div class="memory-info">{formatMemory(systemMemoryGB)} unified memory{#if isOpMode} <span class="op-badge">OP Mode</span>{/if}</div>
            {/if}
            <div class="model-list">
              {#each recommendedModels as model}
                {@const installed = ollamaStatus?.available_models.some(m => m.name === model.name || m.name.startsWith(model.name.split(":")[0] + ":" + model.name.split(":")[1]))}
                {@const isRecommended = model.name === recommendedModelName}
                {@const fitsMemory = systemMemoryGB ? (model.min_memory_bytes / (1024*1024*1024)) <= systemMemoryGB : true}
                <div class="model-row" class:model-recommended={isRecommended} class:model-too-large={!fitsMemory}>
                  <div class="model-info">
                    <div class="model-name-row">
                      <span class="model-name">{model.name}</span>
                      {#if isRecommended}
                        <span class="rec-badge">Best fit</span>
                      {/if}
                      {#if !fitsMemory}
                        <span class="too-large-badge">Too large</span>
                      {/if}
                    </div>
                    <span class="model-meta">{model.size_display} &middot; {model.description}</span>
                  </div>
                  <div class="model-actions">
                    {#if installed}
                      <button
                        class="model-btn model-btn-load"
                        disabled={loading || !fitsMemory}
                        onclick={() => handleLoadModel(model.name)}
                      >
                        {loadingModelName === model.name ? "Loading..." : "Load"}
                      </button>
                    {:else}
                      <button
                        class="model-btn model-btn-dl"
                        disabled={pullingModel !== null || !fitsMemory}
                        onclick={() => handlePull(model.name)}
                      >
                        {pullingModel === model.name ? "Downloading..." : "Download"}
                      </button>
                    {/if}
                  </div>
                </div>
              {/each}
              <!-- Other installed models not in recommended list -->
              {#each (ollamaStatus?.available_models ?? []).filter(m => !recommendedModels.some(r => m.name === r.name || m.name.startsWith(r.name.split(":")[0] + ":"))) as model}
                <div class="model-row">
                  <div class="model-info">
                    <span class="model-name">{model.name}</span>
                    <span class="model-meta">{model.size_display}</span>
                  </div>
                  <button
                    class="model-btn model-btn-load"
                    disabled={loading}
                    onclick={() => handleLoadModel(model.name)}
                  >
                    {loadingModelName === model.name ? "Loading..." : "Load"}
                  </button>
                </div>
              {/each}
              <!-- Custom model input -->
              <div class="custom-model-row">
                <input
                  class="custom-model-input"
                  type="text"
                  placeholder="Or enter model name (e.g. llama3.2:3b)"
                  bind:value={customModelName}
                  onkeydown={(e) => { if (e.key === "Enter" && customModelName.trim()) { handlePull(customModelName.trim()); customModelName = ""; } }}
                />
                {#if customModelName.trim()}
                  <button
                    class="model-btn model-btn-dl"
                    disabled={pullingModel !== null}
                    onclick={() => { handlePull(customModelName.trim()); customModelName = ""; }}
                  >
                    {pullingModel === customModelName.trim() ? "Downloading..." : "Download"}
                  </button>
                {/if}
              </div>
            </div>
          {/if}
        {/if}
      </div>
    {:else if showModelPicker && chatState?.loaded}
      <!-- Inline model switcher when already loaded -->
      <div class="setup-area compact">
        <!-- Backend toggle -->
        <div class="backend-toggle">
          <button
            class="backend-btn"
            class:backend-active={selectedBackend === "mlx"}
            onclick={() => selectedBackend = "mlx"}
          >MLX</button>
          <button
            class="backend-btn"
            class:backend-active={selectedBackend === "ollama"}
            onclick={() => selectedBackend = "ollama"}
          >Ollama</button>
        </div>
        <div class="model-list">
          <!-- Current active model -->
          {#if chatState?.model}
            <div class="model-row model-active">
              <div class="model-info">
                <span class="model-name">{chatState.model.split("/").pop()}</span>
                <span class="model-meta">{chatState.backend === "mlx" ? "MLX" : "Ollama"} &middot; Active</span>
              </div>
              <div class="model-actions-group">
                <button class="model-btn model-btn-secondary" onclick={handleUnload} title="Unload from memory">Unload</button>
                <button
                  class="model-btn model-btn-danger"
                  disabled={deletingModel !== null}
                  onclick={() => handleDeleteModel(chatState!.model!, chatState!.backend)}
                  title="Delete model files from disk"
                >{deletingModel ? "..." : "Delete"}</button>
              </div>
            </div>
          {/if}
          <!-- Other recommended models to switch to -->
          {#each recommendedModels.filter(m => systemMemoryGB ? (m.min_memory_bytes / (1024*1024*1024)) <= systemMemoryGB : true) as model}
            <div class="model-row">
              <span class="model-name">{model.name}</span>
              <button class="model-btn model-btn-load" disabled={loading} onclick={() => handleLoadModel(model.name)}>
                {loadingModelName === model.name ? "..." : "Switch"}
              </button>
            </div>
          {/each}
        </div>
      </div>
    {/if}

    <!-- Messages (hidden when model picker is showing and no model loaded) -->
    <div class="chat-messages" class:chat-messages-hidden={showModelPicker && !chatState?.loaded} bind:this={messagesDiv}>
      {#if displayMessages.length === 0 && chatState?.loaded}
        <div class="chat-empty">
          <p>Ask about your mods:</p>
          <div class="empty-suggestions">
            <button class="suggestion" onclick={() => { inputText = "List all my enabled mods"; handleSend(); }}>List enabled mods</button>
            <button class="suggestion" onclick={() => { inputText = "Are there any mod conflicts?"; handleSend(); }}>Check conflicts</button>
            <button class="suggestion" onclick={() => { inputText = "What's my load order?"; handleSend(); }}>Show load order</button>
          </div>
        </div>
      {/if}
      {#each displayMessages as msg}
        <div class="chat-msg chat-msg-{msg.role}">
          <div class="msg-bubble">
            {#if msg.role === "tool"}
              <pre class="tool-output">{msg.content}</pre>
            {:else if msg.role === "assistant"}
              <span class="msg-content">{@html renderMarkdown(msg.content)}</span>
            {:else}
              {msg.content}
            {/if}
          </div>
        </div>
      {/each}
      {#if sending}
        {#if isStreaming && streamHasContent}
          <div class="chat-msg chat-msg-assistant">
            <div class="msg-bubble streaming-bubble">{settledText}{#each recentTokens as token}<span class="stream-token">{token}</span>{/each}<span class="stream-cursor"></span></div>
          </div>
        {:else if streamPhase === "thinking"}
          <div class="chat-msg chat-msg-assistant">
            <div class="msg-bubble thinking-bubble">
              <span class="thinking-text">
                {#each "Thinking...".split("") as char, i}
                  <span class="thinking-char" style="animation-delay: {i * 0.09}s">{char}</span>
                {/each}
              </span>
            </div>
          </div>
        {:else if streamPhase === "tools"}
          <div class="chat-msg chat-msg-assistant">
            <div class="msg-bubble thinking-bubble">
              <span class="tool-status-text">
                {#each (toolStatusText || "Using tools...").split("") as char, i}
                  <span class="thinking-char" style="animation-delay: {i * 0.06}s">{char}</span>
                {/each}
              </span>
            </div>
          </div>
        {:else}
          <div class="chat-msg chat-msg-assistant">
            <div class="msg-bubble typing">
              <span class="dot"></span><span class="dot"></span><span class="dot"></span>
            </div>
          </div>
        {/if}
      {/if}
    </div>

    <!-- Input -->
    {#if chatState?.loaded}
      <div class="chat-input-row">
        <textarea
          class="chat-input"
          placeholder="Ask about your mods..."
          rows="1"
          bind:value={inputText}
          onkeydown={handleKeydown}
          disabled={sending}
        ></textarea>
        <button
          class="send-btn"
          disabled={!inputText.trim() || sending}
          onclick={handleSend}
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor"><path d="M2.01 21L23 12 2.01 3 2 10l15 2-15 2z"/></svg>
        </button>
      </div>
    {/if}
  </div>
{/if}

<style>
  /* ---- Container: fills sidebar flex space ---- */
  .chat-container {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    border-top: 1px solid var(--border);
    background: var(--bg-grouped);
  }

  /* ---- Header ---- */
  .chat-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-grouped);
    flex-shrink: 0;
  }

  .chat-header-label {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 13px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .header-model {
    color: var(--text);
    font-weight: 500;
  }

  .chat-header-actions {
    display: flex;
    gap: 1px;
  }

  .hdr-btn {
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-quaternary);
    padding: 3px;
    border-radius: 4px;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: color 0.1s, background 0.1s;
  }

  .hdr-btn:hover {
    color: var(--text);
    background: color-mix(in srgb, var(--text) 8%, transparent);
  }

  /* ---- Backend toggle ---- */
  .backend-toggle {
    display: flex;
    gap: 1px;
    background: var(--border);
    border-radius: 6px;
    overflow: hidden;
    margin-bottom: 6px;
  }

  .backend-btn {
    flex: 1;
    padding: 5px 0;
    font-size: 11px;
    font-weight: 600;
    border: none;
    cursor: pointer;
    background: var(--bg-base);
    color: var(--text-quaternary);
    transition: background 0.15s, color 0.15s;
  }

  .backend-btn:hover:not(.backend-active) {
    color: var(--text-secondary);
  }

  .backend-btn.backend-active {
    background: var(--accent);
    color: white;
  }

  /* ---- Setup / Model picker ---- */
  .setup-area {
    padding: 8px;
    flex: 1;
    overflow-y: auto;
    background: var(--bg-grouped);
  }

  .setup-area.compact {
    padding: 6px 8px;
    flex: 0 1 auto;
    max-height: 50%;
    overflow-y: auto;
  }

  .setup-message {
    text-align: center;
    padding: 12px 4px;
  }

  .setup-message p {
    margin: 0 0 8px;
    font-size: 11px;
    color: var(--text-secondary);
  }

  .setup-hint {
    font-size: 10px !important;
    color: var(--text-quaternary) !important;
  }

  .setup-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 5px 14px;
    font-size: 11px;
    font-weight: 600;
    border: none;
    border-radius: 6px;
    cursor: pointer;
    background: var(--accent);
    color: white;
    transition: opacity 0.15s;
  }

  .setup-btn:hover:not(:disabled) { opacity: 0.85; }
  .setup-btn:disabled { opacity: 0.5; cursor: not-allowed; }

  .mini-spinner {
    display: inline-block;
    width: 12px;
    height: 12px;
    border: 2px solid color-mix(in srgb, currentColor 30%, transparent);
    border-top-color: currentColor;
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
    flex-shrink: 0;
  }

  @keyframes spin { to { transform: rotate(360deg); } }

  .memory-info {
    font-size: 10px;
    color: var(--text-quaternary);
    text-align: center;
    padding: 2px 0 6px;
  }

  .op-badge {
    font-size: 9px;
    font-weight: 700;
    padding: 1px 5px;
    border-radius: 3px;
    background: linear-gradient(135deg, #f59e0b, #ef4444);
    color: white;
    letter-spacing: 0.5px;
    text-transform: uppercase;
  }

  .model-list {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .model-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 10px;
    border-radius: 6px;
    background: color-mix(in srgb, var(--bg-base) 60%, transparent);
    gap: 8px;
    min-height: 52px;
  }

  .model-row.model-recommended {
    background: color-mix(in srgb, var(--accent) 10%, var(--bg-base));
    border: 1px solid color-mix(in srgb, var(--accent) 25%, transparent);
  }

  .model-row.model-too-large {
    opacity: 0.45;
  }

  .model-row.model-active {
    background: color-mix(in srgb, var(--accent) 12%, var(--bg-base));
  }

  .model-info {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
    flex: 1;
  }

  .model-name-row {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .model-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
    white-space: nowrap;
  }

  .rec-badge {
    font-size: 9px;
    font-weight: 600;
    padding: 0 4px;
    border-radius: 3px;
    background: var(--accent);
    color: white;
    white-space: nowrap;
  }

  .too-large-badge {
    font-size: 9px;
    padding: 0 4px;
    border-radius: 3px;
    background: color-mix(in srgb, var(--text-quaternary) 20%, transparent);
    color: var(--text-quaternary);
    white-space: nowrap;
  }

  .active-label {
    font-size: 10px;
    color: var(--accent);
    font-weight: 500;
  }

  .model-meta {
    font-size: 12px;
    color: var(--text-secondary);
    line-height: 1.4;
    white-space: normal;
    word-break: break-word;
  }

  .model-actions {
    flex-shrink: 0;
  }

  .model-btn {
    padding: 3px 10px;
    font-size: 11px;
    font-weight: 600;
    border: none;
    border-radius: 4px;
    cursor: pointer;
    transition: opacity 0.1s;
  }

  .model-btn:disabled { opacity: 0.4; cursor: not-allowed; }

  .model-btn-load {
    background: var(--accent);
    color: white;
    border: 1px solid var(--accent);
  }

  .model-btn-dl {
    background: color-mix(in srgb, var(--text) 10%, transparent);
    color: var(--text-secondary);
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: 1px solid var(--accent);
  }

  .model-btn-secondary {
    background: color-mix(in srgb, var(--text) 10%, transparent);
    color: var(--text-secondary);
  }

  .model-btn-danger {
    background: color-mix(in srgb, #ef4444 15%, transparent);
    color: #ef4444;
  }

  .model-actions-group {
    display: flex;
    gap: 3px;
    flex-shrink: 0;
  }

  .model-btn:hover:not(:disabled) { opacity: 0.8; }

  .custom-model-row {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 0 0;
  }

  .custom-model-input {
    flex: 1;
    font-size: 10px;
    padding: 4px 6px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg-base);
    color: var(--text);
    outline: none;
    font-family: var(--font-mono, monospace);
  }

  .custom-model-input:focus {
    border-color: var(--accent);
  }

  .custom-model-input::placeholder {
    color: var(--text-quaternary);
  }

  /* ---- Messages ---- */
  .chat-messages {
    flex: 1;
    overflow-y: auto;
    padding: 8px 8px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-height: 60px;
    background: var(--bg-grouped);
    scroll-behavior: smooth;
  }

  .chat-messages-hidden {
    display: none;
  }

  .chat-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 16px 4px;
  }

  .chat-empty p {
    margin: 0;
    font-size: 12px;
    color: var(--text-quaternary);
  }

  .empty-suggestions {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    justify-content: center;
  }

  .suggestion {
    font-size: 11.5px;
    padding: 3px 8px;
    border-radius: 10px;
    border: 1px solid var(--border);
    background: var(--bg-base);
    color: var(--text-secondary);
    cursor: pointer;
    transition: background 0.1s, border-color 0.1s;
  }

  .suggestion:hover {
    background: color-mix(in srgb, var(--accent) 10%, var(--bg-base));
    border-color: var(--accent);
    color: var(--text);
  }

  .chat-msg {
    max-width: 90%;
  }

  .chat-msg-user { align-self: flex-end; }
  .chat-msg-assistant, .chat-msg-tool { align-self: flex-start; }

  .msg-bubble {
    padding: 8px 12px;
    border-radius: 10px;
    font-size: 13.5px;
    line-height: 1.55;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .chat-msg-user .msg-bubble {
    background: var(--accent);
    color: white;
    border-bottom-right-radius: 3px;
  }

  .chat-msg-assistant .msg-bubble {
    background: var(--bg-base);
    color: var(--text);
    border-bottom-left-radius: 3px;
  }

  .streaming-bubble {
    white-space: pre-wrap;
    word-break: break-word;
  }

  .stream-token {
    animation: token-glow-in 1.4s ease-out forwards;
    opacity: 0;
  }

  @keyframes token-glow-in {
    0% {
      opacity: 0;
      text-shadow: 0 0 10px var(--accent), 0 0 18px color-mix(in srgb, var(--accent) 40%, transparent);
    }
    15% {
      opacity: 0.7;
      text-shadow: 0 0 8px var(--accent), 0 0 14px color-mix(in srgb, var(--accent) 35%, transparent);
    }
    30% {
      opacity: 0.9;
      text-shadow: 0 0 6px color-mix(in srgb, var(--accent) 55%, transparent), 0 0 10px color-mix(in srgb, var(--accent) 25%, transparent);
    }
    50% {
      opacity: 1;
      text-shadow: 0 0 4px color-mix(in srgb, var(--accent) 35%, transparent);
    }
    75% {
      opacity: 1;
      text-shadow: 0 0 2px color-mix(in srgb, var(--accent) 18%, transparent);
    }
    100% {
      opacity: 1;
      text-shadow: none;
    }
  }

  .stream-cursor {
    display: inline-block;
    width: 2px;
    height: 1.1em;
    background: var(--accent);
    margin-left: 1px;
    vertical-align: text-bottom;
    border-radius: 1px;
    animation: cursor-pulse 0.9s ease-in-out infinite;
    box-shadow: 0 0 6px var(--accent), 0 0 12px color-mix(in srgb, var(--accent) 40%, transparent);
  }

  @keyframes cursor-pulse {
    0%, 100% { opacity: 1; box-shadow: 0 0 6px var(--accent), 0 0 12px color-mix(in srgb, var(--accent) 40%, transparent); }
    50% { opacity: 0.3; box-shadow: 0 0 2px color-mix(in srgb, var(--accent) 20%, transparent); }
  }

  .chat-msg-tool .msg-bubble {
    background: color-mix(in srgb, var(--bg-base) 80%, transparent);
    border: 1px solid var(--border);
    padding: 6px 10px;
    border-bottom-left-radius: 3px;
  }

  .tool-output {
    margin: 0;
    font-size: 11.5px;
    font-family: var(--font-mono, monospace);
    color: var(--text-secondary);
    white-space: pre-wrap;
    line-height: 1.5;
  }

  .msg-content {
    display: block;
  }

  /* Markdown-ish inline styles for assistant messages */
  .msg-bubble :global(.inline-code) {
    font-family: var(--font-mono, monospace);
    font-size: 0.9em;
    background: color-mix(in srgb, var(--text) 8%, transparent);
    padding: 1px 4px;
    border-radius: 3px;
  }

  .msg-bubble :global(.md-bullet) {
    display: block;
    padding-left: 4px;
  }

  .msg-bubble :global(strong) {
    font-weight: 700;
  }

  /* Typing dots */
  .typing {
    display: flex;
    gap: 3px;
    padding: 8px 12px;
  }

  .dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--text-quaternary);
    animation: typing 1.2s ease-in-out infinite;
  }

  .dot:nth-child(2) { animation-delay: 0.2s; }
  .dot:nth-child(3) { animation-delay: 0.4s; }

  @keyframes typing {
    0%, 60%, 100% { opacity: 0.3; transform: translateY(0); }
    30% { opacity: 1; transform: translateY(-3px); }
  }

  /* Thinking indicator — glow-wave animation matching streaming style */
  .thinking-bubble {
    padding: 8px 12px;
  }

  .thinking-text,
  .tool-status-text {
    display: inline-flex;
    font-size: 13px;
    font-style: italic;
    letter-spacing: 0.3px;
    flex-wrap: wrap;
  }

  .thinking-char {
    animation: thinking-glow-wave 2s ease-in-out infinite;
    color: var(--text-quaternary);
  }

  @keyframes thinking-glow-wave {
    0%, 100% {
      color: var(--text-quaternary);
      text-shadow: none;
    }
    40% {
      color: var(--text-secondary);
      text-shadow: 0 0 8px var(--accent), 0 0 16px color-mix(in srgb, var(--accent) 35%, transparent);
    }
    60% {
      color: var(--text-secondary);
      text-shadow: 0 0 6px color-mix(in srgb, var(--accent) 50%, transparent);
    }
  }

  /* ---- Input ---- */
  .chat-input-row {
    display: flex;
    align-items: flex-end;
    gap: 4px;
    padding: 6px 8px;
    border-top: 1px solid var(--border);
    background: var(--bg-grouped);
    flex-shrink: 0;
  }

  .chat-input {
    flex: 1;
    resize: none;
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 7px 10px;
    font-size: 13.5px;
    font-family: inherit;
    background: var(--bg-base);
    color: var(--text);
    outline: none;
    max-height: 60px;
    transition: border-color 0.15s;
  }

  .chat-input:focus {
    border-color: var(--accent);
  }

  .send-btn {
    background: var(--accent);
    color: white;
    border: none;
    border-radius: 6px;
    padding: 5px 6px;
    cursor: pointer;
    transition: opacity 0.1s;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .send-btn:disabled { opacity: 0.3; cursor: not-allowed; }
  .send-btn:hover:not(:disabled) { opacity: 0.85; }
</style>
