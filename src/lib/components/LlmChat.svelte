<script lang="ts">
  import { onDestroy } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { selectedGame, showError, showSuccess, currentPage, installedMods } from "$lib/stores";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { goto } from "$app/navigation";
  import { getInstalledMods, startOllama, checkMlxStatus, installMlx, deleteModel, getCachedMlxModels } from "$lib/api";
  import type { LlmBackend } from "$lib/types";
  import {
    chatGetState,
    chatLoadModel,
    chatUnloadModel,
    chatSendMessage,
    chatClearHistory,
    chatGetHistory,
    chatGetStarters,
    chatCheckNewCrashes,
    chatValidateCloudKey,
    checkOllamaStatus,
    getRecommendedModels,
    pullOllamaModel,
    getSystemMemory,
    installOllama,
    getRecommendedModel,
    googleSignIn,
    googleSignOut,
    googleAuthStatus,
  } from "$lib/api";
  import type { ChatMessage, ChatState, ChatStarter, NewCrashInfo, OllamaModel, OllamaStatus, ChatResponse, MentionedMod, GoogleAuthStatus } from "$lib/types";

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

  // Cloud backend state
  let cloudProvider = $state<string>("groq");
  let cloudApiKey = $state("");
  let cloudKeyVisible = $state(false);
  let cloudValidating = $state(false);
  let cloudKeyValid = $state<boolean | null>(null);
  let cloudConnecting = $state(false);

  // Google OAuth state
  let googleAuth = $state<GoogleAuthStatus | null>(null);
  let googleSigningIn = $state(false);
  let googleConnecting = $state(false);

  // Cloud provider metadata
  const cloudProviders = [
    { id: "gemini", name: "Google Gemini", model: "Gemini 2.5 Flash", free: "Free: 60 req/min, 1K req/day (Google sign-in)", url: "https://aistudio.google.com/apikey" },
    { id: "groq", name: "Groq", model: "Llama 3.3 70B", free: "Free: 30 req/min, ~500K tokens/day", url: "https://console.groq.com/keys" },
    { id: "cerebras", name: "Cerebras", model: "Llama 3.3 70B", free: "Free: 1M tokens/day, 30 req/min", url: "https://cloud.cerebras.ai/" },
  ];

  // Load saved cloud config from localStorage
  function loadCloudConfig() {
    try {
      const saved = localStorage.getItem("corkscrew_cloud_llm_config");
      if (saved) {
        const config = JSON.parse(saved);
        if (config.provider) cloudProvider = config.provider;
        if (config.api_key) cloudApiKey = config.api_key;
        cloudKeyValid = true; // Assume valid if saved
      }
    } catch {}
  }

  function saveCloudConfig() {
    try {
      localStorage.setItem("corkscrew_cloud_llm_config", JSON.stringify({
        provider: cloudProvider,
        api_key: cloudApiKey,
      }));
    } catch {}
  }

  function clearCloudConfig() {
    cloudApiKey = "";
    cloudKeyValid = null;
    cloudKeyVisible = false;
    try { localStorage.removeItem("corkscrew_cloud_llm_config"); } catch {}
  }
  let customModelName = $state("");
  let autoUnloadTimer = $state<ReturnType<typeof setTimeout> | null>(null);
  let messagesDiv: HTMLDivElement | undefined = $state();

  // Conversation starters & crash detection
  let conversationStarters = $state<ChatStarter[]>([]);
  let crashBanner = $state<NewCrashInfo | null>(null);

  // Persisted history shown before model is loaded
  let historyMessages = $state<ChatMessage[]>([]);
  let historyCount = $state(0);

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

  // Tool call history for current response (shown inline like Claude web)
  interface ToolCallEntry {
    name: string;
    displayText: string;
    status: "running" | "complete";
  }
  let activeToolCalls = $state<ToolCallEntry[]>([]);
  // Completed tool call sets, keyed by the index of the assistant message they precede
  let completedToolSets = $state<Map<number, ToolCallEntry[]>>(new Map());
  let expandedToolSets = $state<Set<number>>(new Set());

  listen<{ tool_name: string; status: string; display_text: string }>("chat-tool-status", (event) => {
    if (event.payload.status === "running") {
      streamPhase = "tools";
      toolStatusText = event.payload.display_text;
      activeToolCalls = [...activeToolCalls, {
        name: event.payload.tool_name,
        displayText: event.payload.display_text,
        status: "running",
      }];
    } else if (event.payload.status === "complete") {
      toolStatusText = "";
      activeToolCalls = activeToolCalls.map(tc =>
        tc.name === event.payload.tool_name && tc.status === "running"
          ? { ...tc, status: "complete" as const }
          : tc
      );
    }
  }).then((unlisten) => { unlistenToolStatus = unlisten; });

  // Listen for LLM-triggered UI navigation
  let unlistenNavigate: (() => void) | null = null;
  let unlistenOpenMod: (() => void) | null = null;

  listen<{ page: string }>("chat-navigate", (event) => {
    const page = event.payload.page;
    currentPage.set(page);
    if (window.location.pathname !== "/") {
      goto("/");
    }
  }).then((unlisten) => { unlistenNavigate = unlisten; });

  listen<{ mod_id: number; name: string }>("chat-open-nexus-mod", (event) => {
    const { mod_id, name } = event.payload;
    // Navigate to Discover page and emit an event for the collections page to open the mod
    currentPage.set("discover");
    if (window.location.pathname !== "/") {
      goto("/");
    }
    // Give the page time to mount, then emit the open-mod event
    setTimeout(() => {
      window.dispatchEvent(new CustomEvent("corkscrew-open-nexus-mod", {
        detail: { mod_id, name },
      }));
    }, 300);
  }).then((unlisten) => { unlistenOpenMod = unlisten; });
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
    if (unlistenNavigate) unlistenNavigate();
    if (unlistenOpenMod) unlistenOpenMod();
    if (autoUnloadTimer) clearTimeout(autoUnloadTimer);
    if (charTimerId) clearTimeout(charTimerId);
    if (thinkingTimer) clearInterval(thinkingTimer);
  });

  // Current page name from store
  let currentPageName = $derived($currentPage || "Mods");

  // Display messages (skip system, skip empty assistant messages from tool-call-only turns)
  // Display messages with original indices preserved (for tool call association)
  let displayMessages = $derived(
    (chatState?.messages ?? []).reduce<Array<{ msg: ChatMessage; origIdx: number }>>((acc, m, i) => {
      if (m.role === "system") return acc;
      if (m.role === "tool") return acc;
      if (m.role === "assistant" && !m.content?.trim()) return acc;
      acc.push({ msg: m, origIdx: i });
      return acc;
    }, [])
  );

  function toggleToolSet(idx: number) {
    const s = new Set(expandedToolSets);
    if (s.has(idx)) s.delete(idx); else s.add(idx);
    expandedToolSets = s;
  }

  function nexusGameSlug(gameId: string): string {
    const slugs: Record<string, string> = {
      skyrimse: "skyrimspecialedition",
      skyrim: "skyrim",
      fallout4: "fallout4",
      fallout76: "fallout76",
      oblivion: "oblivion",
      morrowind: "morrowind",
      starfield: "starfield",
    };
    return slugs[gameId] ?? gameId;
  }

  function openModInCorkscrew(mod: MentionedMod) {
    if (!mod.nexus_mod_id) return;
    currentPage.set("discover");
    if (window.location.pathname !== "/") {
      goto("/");
    }
    setTimeout(() => {
      window.dispatchEvent(new CustomEvent("corkscrew-open-nexus-mod", {
        detail: { mod_id: mod.nexus_mod_id, name: mod.name },
      }));
    }, 300);
  }

  $effect(() => {
    if (visible) {
      loadState();
      checkForCrashes();
    }
  });

  async function checkForCrashes() {
    const game = $selectedGame;
    if (!game) return;
    try {
      const [crashes, starters] = await Promise.all([
        chatCheckNewCrashes(game.game_id, game.bottle_name),
        chatGetStarters(game.game_id, game.bottle_name, currentPageName),
      ]);
      conversationStarters = starters;
      if (crashes.count > 0) {
        crashBanner = crashes;
      } else {
        crashBanner = null;
      }
    } catch {
      // Non-critical — don't block chat
    }
  }

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
      if (state.loaded) {
        if (state.cloud_provider === "gemini_oauth") {
          selectedBackend = "cloud";
          cloudProvider = "gemini";
        } else if (state.cloud_provider) {
          selectedBackend = "cloud";
        } else if (typeof state.backend === "string") {
          selectedBackend = state.backend as LlmBackend;
        }
      }
      if (!state.loaded) {
        showModelPicker = true;
        // Load persisted history for read-only display
        const game = $selectedGame;
        if (game) {
          try {
            const history = await chatGetHistory(game.game_id, game.bottle_name);
            historyMessages = history;
            historyCount = history.length;
          } catch { /* non-critical */ }
        }
      } else {
        // Model loaded — history already restored by backend
        historyMessages = [];
        historyCount = 0;
      }
      loadCloudConfig();
      // Load Google auth status
      try {
        googleAuth = await googleAuthStatus();
        if (googleAuth?.signed_in) {
          // Default to Gemini when signed in with Google
          cloudProvider = "gemini";
        }
      } catch { /* non-critical */ }
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
      if (selectedBackend === "cloud") {
        await chatLoadModel(modelName, game.game_id, game.bottle_name, currentPageName, "cloud", cloudProvider, cloudApiKey);
      } else {
        await chatLoadModel(modelName, game.game_id, game.bottle_name, currentPageName, selectedBackend);
      }
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

  async function handleCloudConnect() {
    if (!cloudApiKey.trim()) {
      showError("Please enter an API key.");
      return;
    }
    const game = $selectedGame;
    if (!game) return;
    cloudConnecting = true;
    try {
      await chatLoadModel("cloud", game.game_id, game.bottle_name, currentPageName, "cloud", cloudProvider, cloudApiKey);
      saveCloudConfig();
      chatState = await chatGetState();
      showModelPicker = false;
      resetAutoUnload();
    } catch (e) {
      showError(`Failed to connect: ${e}`);
    } finally {
      cloudConnecting = false;
    }
  }

  async function handleValidateKey() {
    if (!cloudApiKey.trim()) return;
    cloudValidating = true;
    cloudKeyValid = null;
    try {
      await chatValidateCloudKey(cloudProvider, cloudApiKey);
      cloudKeyValid = true;
      showSuccess("API key is valid!");
    } catch (e) {
      cloudKeyValid = false;
      showError(`${e}`);
    } finally {
      cloudValidating = false;
    }
  }

  async function handleGoogleSignIn() {
    const game = $selectedGame;
    if (!game) return;
    googleSigningIn = true;
    try {
      const status = await googleSignIn();
      googleAuth = status;
      if (status.signed_in) {
        // Auto-connect to Gemini via OAuth
        googleConnecting = true;
        try {
          await chatLoadModel("gemini_oauth", game.game_id, game.bottle_name, currentPageName, "gemini_oauth");
          chatState = await chatGetState();
          showModelPicker = false;
          resetAutoUnload();
        } catch (e) {
          showError(`Failed to connect to Gemini: ${e}`);
        } finally {
          googleConnecting = false;
        }
      }
    } catch (e) {
      showError(`Google sign-in failed: ${e}`);
    } finally {
      googleSigningIn = false;
    }
  }

  async function handleGoogleSignOut() {
    try {
      await googleSignOut();
      googleAuth = { signed_in: false };
      // If currently using GeminiOAuth, unload
      if (chatState?.cloud_provider === "gemini_oauth") {
        await chatUnloadModel();
        chatState = await chatGetState();
      }
    } catch (e) {
      showError(`Sign-out failed: ${e}`);
    }
  }

  async function handleGoogleConnect() {
    const game = $selectedGame;
    if (!game) return;
    googleConnecting = true;
    try {
      await chatLoadModel("gemini_oauth", game.game_id, game.bottle_name, currentPageName, "gemini_oauth");
      chatState = await chatGetState();
      showModelPicker = false;
      resetAutoUnload();
    } catch (e) {
      showError(`Failed to connect: ${e}`);
    } finally {
      googleConnecting = false;
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
    activeToolCalls = [];
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

      // Save tool calls for this response (associate with the last assistant message index)
      if (activeToolCalls.length > 0 && chatState) {
        const lastAssistantIdx = chatState.messages.reduce((acc, m, i) =>
          m.role === "assistant" && m.content?.trim() ? i : acc, -1);
        if (lastAssistantIdx >= 0) {
          const newMap = new Map(completedToolSets);
          newMap.set(lastAssistantIdx, [...activeToolCalls]);
          completedToolSets = newMap;
        }
      }

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

      // Extract structured_data from tool results and attach to the last assistant message
      const healthResult = resp.tool_results?.find(tr => tr.structured_data?.type === "health_score");
      if (healthResult?.structured_data && chatState) {
        const lastIdx = chatState.messages.length - 1;
        if (lastIdx >= 0 && chatState.messages[lastIdx].role === "assistant") {
          chatState.messages[lastIdx].structured_data = healthResult.structured_data;
          chatState = { ...chatState };
        }
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
    const game = $selectedGame;
    if (game) {
      await chatClearHistory(game.game_id, game.bottle_name);
    }
    chatState = await chatGetState();
    historyMessages = [];
    historyCount = 0;
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
    let html = text.trim();
    // Escape HTML
    html = html.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
    // Code blocks (``` ... ```)
    html = html.replace(/```(\w*)\n([\s\S]*?)```/g, '<pre class="md-codeblock"><code>$2</code></pre>');
    // Headers
    html = html.replace(/^### (.+)$/gm, '<strong class="md-h3">$1</strong>');
    html = html.replace(/^## (.+)$/gm, '<strong class="md-h2">$1</strong>');
    html = html.replace(/^# (.+)$/gm, '<strong class="md-h1">$1</strong>');
    // Bold + italic
    html = html.replace(/\*\*\*(.+?)\*\*\*/g, "<strong><em>$1</em></strong>");
    html = html.replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>");
    html = html.replace(/\*(.+?)\*/g, "<em>$1</em>");
    // Inline code
    html = html.replace(/`([^`]+)`/g, '<code class="inline-code">$1</code>');
    // Links [text](url)
    html = html.replace(/\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g, '<a class="md-link" href="$2" target="_blank">$1</a>');
    // Bare URLs
    html = html.replace(/(?<!")https?:\/\/[^\s<)]+/g, '<a class="md-link" href="$&" target="_blank">$&</a>');
    // Bullet lists
    html = html.replace(/^[-•]\s+(.+)$/gm, '<span class="md-bullet">• $1</span>');
    // Numbered lists
    html = html.replace(/^\d+\.\s+(.+)$/gm, '<span class="md-bullet">$&</span>');
    // Horizontal rules
    html = html.replace(/^---+$/gm, '<hr class="md-hr">');
    // Line breaks (preserve paragraph spacing)
    html = html.replace(/\n\n/g, '<br><br>');
    html = html.replace(/\n/g, '<br>');
    return html;
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
          <button
            class="backend-btn"
            class:backend-active={selectedBackend === "cloud"}
            onclick={() => selectedBackend = "cloud"}
          >Cloud</button>
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
                  <div class="model-actions-group">
                    {#if isMlxModelCached(model.name)}
                      <button
                        class="model-btn model-btn-load"
                        disabled={loading || !fitsMemory}
                        onclick={() => handleLoadModel(model.name)}
                      >
                        {loadingModelName === model.name ? "Loading..." : "Load"}
                      </button>
                      <button
                        class="model-btn-icon model-btn-danger-icon"
                        disabled={deletingModel !== null}
                        onclick={() => handleDeleteModel(model.name, "mlx")}
                        title="Delete model files"
                      >
                        <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
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
        {:else if selectedBackend === "ollama"}
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
                  <div class="model-actions-group">
                    {#if installed}
                      <button
                        class="model-btn model-btn-load"
                        disabled={loading || !fitsMemory}
                        onclick={() => handleLoadModel(model.name)}
                      >
                        {loadingModelName === model.name ? "Loading..." : "Load"}
                      </button>
                      <button
                        class="model-btn-icon model-btn-danger-icon"
                        disabled={deletingModel !== null}
                        onclick={() => handleDeleteModel(model.name, "ollama")}
                        title="Delete model"
                      >
                        <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
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
                  <div class="model-actions-group">
                    <button
                      class="model-btn model-btn-load"
                      disabled={loading}
                      onclick={() => handleLoadModel(model.name)}
                    >
                      {loadingModelName === model.name ? "Loading..." : "Load"}
                    </button>
                    <button
                      class="model-btn-icon model-btn-danger-icon"
                      disabled={deletingModel !== null}
                      onclick={() => handleDeleteModel(model.name, "ollama")}
                      title="Delete model"
                    >
                      <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
                    </button>
                  </div>
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
        {:else if selectedBackend === "cloud"}
          <!-- Cloud backend -->
          <div class="cloud-setup">
            <div class="cloud-notice">
              Cloud mode sends your chat messages to the selected provider. Your data is sent directly to the provider — never through Corkscrew servers.
            </div>

            <div class="cloud-provider-select">
              <label class="cloud-label">Provider</label>
              <div class="cloud-provider-options">
                {#each cloudProviders as provider}
                  <button
                    class="cloud-provider-btn"
                    class:cloud-provider-active={cloudProvider === provider.id}
                    onclick={() => { cloudProvider = provider.id; cloudKeyValid = null; }}
                  >
                    <span class="cloud-provider-name">{provider.name}</span>
                    <span class="cloud-provider-model">{provider.model}</span>
                  </button>
                {/each}
              </div>
              {#each cloudProviders.filter(p => p.id === cloudProvider) as activeProvider}
                <div class="cloud-provider-info">
                  <span class="cloud-free-tier">{activeProvider.free}</span>
                  {#if cloudProvider !== "gemini"}
                    <a class="cloud-get-key" href={activeProvider.url} onclick={(e) => { e.preventDefault(); openUrl(activeProvider.url); }}>
                      Get free API key &rarr;
                    </a>
                  {/if}
                </div>
              {/each}
            </div>

            {#if cloudProvider === "gemini"}
              <!-- Gemini: Google Sign-In (primary) -->
              {#if googleAuth?.signed_in}
                <div class="google-signed-in">
                  <div class="google-user-info">
                    <span class="google-email">{googleAuth.email ?? googleAuth.name ?? "Signed in"}</span>
                    <button class="model-btn model-btn-secondary google-signout-btn" onclick={handleGoogleSignOut}>Sign out</button>
                  </div>
                  <div class="cloud-actions">
                    <button
                      class="model-btn model-btn-load cloud-connect-btn"
                      disabled={googleConnecting}
                      onclick={handleGoogleConnect}
                    >
                      {#if googleConnecting}
                        <span class="mini-spinner"></span> Connecting...
                      {:else}
                        Start chatting
                      {/if}
                    </button>
                  </div>
                </div>
              {:else}
                <div class="google-signin-section">
                  <button
                    class="model-btn model-btn-load google-signin-btn"
                    disabled={googleSigningIn}
                    onclick={handleGoogleSignIn}
                  >
                    {#if googleSigningIn}
                      <span class="mini-spinner"></span> Signing in...
                    {:else}
                      Sign in with Google
                    {/if}
                  </button>
                  <span class="google-signin-hint">No API key needed — sign in with your Google account</span>
                </div>
              {/if}
            {/if}

            {#if cloudProvider !== "gemini" || (cloudProvider === "gemini" && !googleAuth?.signed_in)}
              <div class="cloud-key-input">
                <label class="cloud-label">{cloudProvider === "gemini" ? "Or use API key" : "API Key"}</label>
                <div class="cloud-key-row">
                  <div class="cloud-key-field">
                    <input
                      type={cloudKeyVisible ? "text" : "password"}
                      class="cloud-key-input-field"
                      class:cloud-key-valid={cloudKeyValid === true}
                      class:cloud-key-invalid={cloudKeyValid === false}
                      placeholder="Paste your API key here"
                      bind:value={cloudApiKey}
                      onkeydown={(e) => { if (e.key === "Enter") handleCloudConnect(); }}
                    />
                    <button class="cloud-key-toggle" onclick={() => cloudKeyVisible = !cloudKeyVisible} title={cloudKeyVisible ? "Hide" : "Show"}>
                      {cloudKeyVisible ? "Hide" : "Show"}
                    </button>
                  </div>
                  <button
                    class="model-btn model-btn-secondary"
                    disabled={!cloudApiKey.trim() || cloudValidating}
                    onclick={handleValidateKey}
                  >
                    {cloudValidating ? "..." : "Validate"}
                  </button>
                </div>
              </div>

              <div class="cloud-actions">
                <button
                  class="model-btn model-btn-load cloud-connect-btn"
                  disabled={!cloudApiKey.trim() || cloudConnecting}
                  onclick={handleCloudConnect}
                >
                  {#if cloudConnecting}
                    <span class="mini-spinner"></span> Connecting...
                  {:else}
                    Connect
                  {/if}
                </button>
                {#if cloudApiKey.trim()}
                  <button class="model-btn model-btn-secondary" onclick={clearCloudConfig}>Clear</button>
                {/if}
              </div>
            {/if}
          </div>
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
          <button
            class="backend-btn"
            class:backend-active={selectedBackend === "cloud"}
            onclick={() => selectedBackend = "cloud"}
          >Cloud</button>
        </div>
        <div class="model-list">
          <!-- Current active model -->
          {#if chatState?.model}
            <div class="model-row model-active">
              <div class="model-info">
                <span class="model-name">{chatState.model.split("/").pop()}</span>
                <span class="model-meta">{chatState.cloud_provider ? cloudProviders.find(p => p.id === chatState!.cloud_provider)?.name ?? "Cloud" : (chatState.backend === "mlx" ? "MLX" : "Ollama")} &middot; Active</span>
              </div>
              <div class="model-actions-group">
                <button class="model-btn model-btn-secondary" onclick={handleUnload} title="Unload from memory">Unload</button>
                <button
                  class="model-btn model-btn-danger"
                  disabled={deletingModel !== null}
                  onclick={() => handleDeleteModel(chatState!.model!, typeof chatState!.backend === "string" ? chatState!.backend : "cloud")}
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
      {#if crashBanner && crashBanner.count > 0 && chatState?.loaded}
        <div class="crash-banner" role="alert">
          <span class="crash-banner-dot"></span>
          <span class="crash-banner-text">
            New crash log detected &mdash; want me to diagnose it?
          </span>
          <button class="crash-banner-btn" onclick={() => {
            inputText = "I just crashed. Can you analyze my latest crash log and tell me what went wrong?";
            crashBanner = null;
            handleSend();
          }}>Diagnose</button>
          <button class="crash-banner-dismiss" onclick={() => { crashBanner = null; }} title="Dismiss">
            <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
          </button>
        </div>
      {/if}
      <!-- Persisted history (shown when no model loaded) -->
      {#if !chatState?.loaded && historyMessages.length > 0}
        <div class="history-divider"><span>Previous session</span></div>
        {#each historyMessages.filter(m => m.role === "user" || (m.role === "assistant" && m.content?.trim())) as msg}
          <div class="chat-msg chat-msg-{msg.role} chat-msg-history" title={msg.timestamp ? new Date(msg.timestamp + "Z").toLocaleString() : ""}>
            <div class="msg-bubble">
              {#if msg.role === "assistant"}
                <span class="msg-content">{@html renderMarkdown(msg.content)}</span>
              {:else}
                {msg.content}
              {/if}
            </div>
          </div>
        {/each}
        <div class="history-notice">Load a model to continue this conversation</div>
      {/if}
      {#if displayMessages.length === 0 && chatState?.loaded}
        <div class="chat-empty">
          <p>Ask about your mods:</p>
          <div class="empty-suggestions">
            {#if conversationStarters.length > 0}
              {#each conversationStarters as starter}
                <button
                  class="suggestion"
                  class:suggestion-crash={starter.label.includes("🔴")}
                  onclick={() => { inputText = starter.prompt; handleSend(); }}
                >{starter.label}</button>
              {/each}
            {:else}
              <button class="suggestion" onclick={() => { inputText = "List all my enabled mods"; handleSend(); }}>List enabled mods</button>
              <button class="suggestion" onclick={() => { inputText = "Are there any mod conflicts?"; handleSend(); }}>Check conflicts</button>
              <button class="suggestion" onclick={() => { inputText = "What's my load order?"; handleSend(); }}>Show load order</button>
            {/if}
          </div>
        </div>
      {/if}
      {#each displayMessages as { msg, origIdx }, idx}
        <!-- "Previous session" header before restored history messages -->
        {#if idx === 0 && msg.timestamp}
          <div class="history-divider"><span>Previous session</span></div>
        {/if}
        <!-- Session boundary divider between restored and new messages -->
        {#if idx > 0 && displayMessages[idx - 1]?.msg.timestamp && !msg.timestamp}
          <div class="history-divider"><span>Current session</span></div>
        {/if}
        <!-- Collapsible tool call summary (shown above assistant messages that used tools) -->
        {#if msg.role === "assistant" && completedToolSets.has(origIdx)}
          {@const tools = completedToolSets.get(origIdx)!}
          {@const isExpanded = expandedToolSets.has(origIdx)}
          <div class="chat-msg chat-msg-assistant">
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div class="tool-summary" onclick={() => toggleToolSet(origIdx)}>
              <span class="tool-summary-icon">
                <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/></svg>
              </span>
              <span class="tool-summary-label">Used {tools.length} tool{tools.length !== 1 ? "s" : ""}</span>
              <span class="tool-summary-chevron" class:expanded={isExpanded}>
                <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><polyline points="6 9 12 15 18 9"/></svg>
              </span>
            </div>
            {#if isExpanded}
              <div class="tool-summary-details">
                {#each tools as tc}
                  <div class="tool-detail-row">
                    <svg width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="var(--accent)" stroke-width="3" stroke-linecap="round"><polyline points="20 6 9 17 4 12"/></svg>
                    <span>{tc.displayText}</span>
                  </div>
                {/each}
              </div>
            {/if}
          </div>
        {/if}
        <div class="chat-msg chat-msg-{msg.role}" class:chat-msg-history={!!msg.timestamp} title={msg.timestamp ? new Date(msg.timestamp + "Z").toLocaleString() : ""}>
          <div class="msg-bubble">
            {#if msg.role === "assistant"}
              <span class="msg-content">{@html renderMarkdown(msg.content)}</span>
            {:else}
              {msg.content}
            {/if}
          </div>
          {#if msg.role === "assistant" && msg.mentioned_mods?.length}
            <div class="mod-cards">
              {#each msg.mentioned_mods as mod}
                <button
                  class="mod-card"
                  class:mod-installed={mod.installed}
                  onclick={() => mod.nexus_mod_id ? openModInCorkscrew(mod) : null}
                  title={mod.nexus_mod_id ? `Open in Corkscrew (ID: ${mod.nexus_mod_id})` : mod.name}
                  disabled={!mod.nexus_mod_id}
                >
                  {#if mod.picture_url}
                    <img class="mod-card-img" src={mod.picture_url} alt="" />
                  {/if}
                  <div class="mod-card-body">
                    <span class="mod-card-name">{mod.name}</span>
                    <div class="mod-card-actions">
                      {#if mod.installed}
                        <span class="mod-card-badge mod-card-installed">Installed</span>
                      {:else if mod.nexus_mod_id}
                        <span class="mod-card-badge mod-card-nexus">View in Corkscrew</span>
                      {/if}
                    </div>
                  </div>
                </button>
              {/each}
            </div>
            <!-- Quick action buttons for mentioned mods -->
            {#if !sending && origIdx === displayMessages.length - 1}
              {@const firstMod = msg.mentioned_mods.find((m: MentionedMod) => m.nexus_mod_id && !m.installed)}
              {@const installedMod = msg.mentioned_mods.find((m: MentionedMod) => m.installed)}
              {@const contentLower = (msg.content || "").toLowerCase()}
              {@const isDestructivePrompt = /\b(uninstall|delete|remove|disable|are you sure)\b/.test(contentLower)}
              <div class="quick-actions">
                {#if isDestructivePrompt}
                  <button class="quick-action quick-action-danger" onclick={() => { inputText = "Yes, proceed"; handleSend(); }}>
                    Yes, proceed
                  </button>
                  <button class="quick-action" onclick={() => { inputText = "No, cancel"; handleSend(); }}>
                    Cancel
                  </button>
                {:else}
                  {#if firstMod?.nexus_mod_id}
                    <button class="quick-action" onclick={() => { inputText = `Install ${firstMod.name}`; handleSend(); }}>
                      Install {firstMod.name}
                    </button>
                    <button class="quick-action" onclick={() => openModInCorkscrew(firstMod)}>
                      Open in Discover
                    </button>
                  {/if}
                  {#if installedMod}
                    <button class="quick-action" onclick={() => { inputText = `Tell me more about ${installedMod.name}`; handleSend(); }}>
                      More about {installedMod.name}
                    </button>
                  {/if}
                  <button class="quick-action" onclick={() => { inputText = "Find me something similar"; handleSend(); }}>
                    Similar mods
                  </button>
                {/if}
              </div>
            {/if}
          {/if}
          {#if msg.role === "assistant" && msg.structured_data?.type === "health_score"}
            {@const hs = msg.structured_data}
            <div class="health-score-card health-{hs.color}">
              <div class="health-score-number">{hs.score}</div>
              <div class="health-score-label">Mod Health</div>
              {#if Array.isArray(hs.issues)}
                {#each hs.issues as issue}
                  <div class="health-issue">{issue.message} ({issue.points})</div>
                {/each}
              {/if}
            </div>
          {/if}
        </div>
      {/each}
      {#if sending}
        <!-- Tool call history (shown during and after tool execution) -->
        {#if activeToolCalls.length > 0}
          <div class="chat-msg chat-msg-assistant">
            <div class="tool-calls-container">
              {#each activeToolCalls as tc, i}
                <div class="tool-call-entry" class:tool-complete={tc.status === "complete"}>
                  <span class="tool-call-icon">
                    {#if tc.status === "running"}
                      <span class="tool-spinner"></span>
                    {:else}
                      <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round"><polyline points="20 6 9 17 4 12"/></svg>
                    {/if}
                  </span>
                  <span class="tool-call-label">
                    {#each tc.displayText.split("") as char, ci}
                      <span class="tool-char" style="animation-delay: {ci * 15}ms">{char}</span>
                    {/each}
                  </span>
                </div>
              {/each}
            </div>
          </div>
        {/if}
        {#if isStreaming && streamHasContent}
          <div class="chat-msg chat-msg-assistant">
            <div class="msg-bubble streaming-bubble">{settledText}{#each recentTokens as token}<span class="stream-token">{token}</span>{/each}</div>
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

  /* ---- Cloud setup ---- */
  .cloud-setup {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 4px 0;
  }

  .cloud-notice {
    font-size: 0.72rem;
    color: var(--text-tertiary);
    line-height: 1.4;
    padding: 8px 10px;
    background: var(--bg-tertiary);
    border-radius: 6px;
    border-left: 2px solid var(--accent);
  }

  .cloud-label {
    font-size: 0.72rem;
    font-weight: 600;
    color: var(--text-secondary);
    margin-bottom: 4px;
    display: block;
  }

  .cloud-provider-options {
    display: flex;
    gap: 6px;
  }

  .cloud-provider-btn {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding: 8px 6px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-secondary);
    cursor: pointer;
    transition: all 0.15s;
  }

  .cloud-provider-btn:hover {
    border-color: var(--accent);
  }

  .cloud-provider-btn.cloud-provider-active {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }

  .cloud-provider-name {
    font-size: 0.75rem;
    font-weight: 600;
    color: var(--text-primary);
  }

  .cloud-provider-model {
    font-size: 0.65rem;
    color: var(--text-tertiary);
  }

  .cloud-provider-info {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-top: 4px;
  }

  .cloud-free-tier {
    font-size: 0.68rem;
    color: var(--text-tertiary);
  }

  .cloud-get-key {
    font-size: 0.68rem;
    color: var(--accent);
    cursor: pointer;
    text-decoration: none;
  }

  .cloud-get-key:hover {
    text-decoration: underline;
  }

  .cloud-key-row {
    display: flex;
    gap: 6px;
    align-items: stretch;
  }

  .cloud-key-field {
    flex: 1;
    position: relative;
    display: flex;
  }

  .cloud-key-input-field {
    flex: 1;
    padding: 6px 52px 6px 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-secondary);
    color: var(--text-primary);
    font-size: 0.75rem;
    font-family: monospace;
    outline: none;
  }

  .cloud-key-input-field:focus {
    border-color: var(--accent);
  }

  .cloud-key-input-field.cloud-key-valid {
    border-color: #22c55e;
  }

  .cloud-key-input-field.cloud-key-invalid {
    border-color: #ef4444;
  }

  .cloud-key-toggle {
    position: absolute;
    right: 4px;
    top: 50%;
    transform: translateY(-50%);
    background: none;
    border: none;
    color: var(--text-tertiary);
    font-size: 0.65rem;
    cursor: pointer;
    padding: 2px 6px;
  }

  .cloud-key-toggle:hover {
    color: var(--text-secondary);
  }

  .cloud-actions {
    display: flex;
    gap: 6px;
  }

  .cloud-connect-btn {
    flex: 1;
  }

  /* Google Sign-In */
  .google-signin-section {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 8px;
  }

  .google-signin-btn {
    background: #4285f4 !important;
    border-color: #4285f4 !important;
    color: white !important;
    font-weight: 500;
  }
  .google-signin-btn:hover:not(:disabled) {
    background: #3367d6 !important;
    border-color: #3367d6 !important;
  }

  .google-signin-hint {
    font-size: 11px;
    color: var(--text-tertiary);
    text-align: center;
  }

  .google-signed-in {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 8px;
  }

  .google-user-info {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 8px;
    background: var(--bg-secondary);
    border-radius: 6px;
  }

  .google-email {
    font-size: 12px;
    color: var(--text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .google-signout-btn {
    font-size: 11px !important;
    padding: 2px 8px !important;
    flex-shrink: 0;
  }

  .api-key-fallback {
    margin-top: 10px;
  }

  .api-key-fallback-toggle {
    font-size: 11px;
    color: var(--text-tertiary);
    cursor: pointer;
    padding: 4px 0;
  }
  .api-key-fallback-toggle:hover {
    color: var(--text-secondary);
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

  .model-btn-icon {
    background: none;
    border: none;
    cursor: pointer;
    padding: 3px;
    border-radius: 4px;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: background 0.1s, opacity 0.1s;
  }

  .model-btn-icon:disabled { opacity: 0.3; cursor: not-allowed; }

  .model-btn-danger-icon {
    color: #ef4444;
  }

  .model-btn-danger-icon:hover:not(:disabled) {
    background: color-mix(in srgb, #ef4444 15%, transparent);
  }

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
    word-break: break-word;
  }

  .chat-msg-user .msg-bubble {
    white-space: pre-wrap;
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

  /* ---- Tool call summary (collapsible, like Claude web) ---- */
  .tool-summary {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 10px;
    border-radius: 8px;
    background: color-mix(in srgb, var(--accent) 8%, var(--bg-base));
    border: 1px solid color-mix(in srgb, var(--accent) 15%, transparent);
    cursor: pointer;
    transition: background 0.15s;
    font-size: 11.5px;
    color: var(--text-secondary);
    user-select: none;
  }

  .tool-summary:hover {
    background: color-mix(in srgb, var(--accent) 14%, var(--bg-base));
  }

  .tool-summary-icon {
    display: flex;
    align-items: center;
    color: var(--accent);
    flex-shrink: 0;
  }

  .tool-summary-label {
    flex: 1;
    font-weight: 500;
  }

  .tool-summary-chevron {
    display: flex;
    align-items: center;
    color: var(--text-quaternary);
    transition: transform 0.2s;
  }

  .tool-summary-chevron.expanded {
    transform: rotate(180deg);
  }

  .tool-summary-details {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 4px 10px 6px 28px;
    font-size: 11px;
    color: var(--text-quaternary);
  }

  .tool-detail-row {
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 1px 0;
  }

  /* ---- Tool call history (live, during streaming) ---- */
  .tool-calls-container {
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding: 6px 10px;
    border-radius: 10px;
    background: color-mix(in srgb, var(--bg-base) 80%, transparent);
    border: 1px solid var(--border);
    border-bottom-left-radius: 3px;
  }

  .tool-call-entry {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11.5px;
    color: var(--text-secondary);
    padding: 2px 0;
  }

  .tool-call-entry.tool-complete {
    color: var(--text-quaternary);
  }

  .tool-call-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 14px;
    height: 14px;
    flex-shrink: 0;
  }

  .tool-call-icon svg {
    color: var(--accent);
  }

  .tool-call-label {
    font-style: italic;
    display: inline-flex;
    flex-wrap: wrap;
  }

  .tool-char {
    animation: tool-char-in 0.4s ease-out forwards;
    opacity: 0;
  }

  @keyframes tool-char-in {
    0% { opacity: 0; text-shadow: 0 0 6px var(--accent); }
    50% { opacity: 0.7; text-shadow: 0 0 3px color-mix(in srgb, var(--accent) 40%, transparent); }
    100% { opacity: 1; text-shadow: none; }
  }

  .tool-spinner {
    display: inline-block;
    width: 10px;
    height: 10px;
    border: 1.5px solid color-mix(in srgb, var(--accent) 30%, transparent);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  /* ---- Markdown styles ---- */
  .msg-bubble :global(.md-h1) {
    display: block;
    font-size: 1.15em;
    margin: 4px 0 2px;
  }

  .msg-bubble :global(.md-h2) {
    display: block;
    font-size: 1.05em;
    margin: 4px 0 2px;
  }

  .msg-bubble :global(.md-h3) {
    display: block;
    font-size: 1em;
    margin: 3px 0 1px;
  }

  .msg-bubble :global(.md-codeblock) {
    background: color-mix(in srgb, var(--text) 6%, transparent);
    border-radius: 6px;
    padding: 6px 8px;
    margin: 4px 0;
    overflow-x: auto;
    font-size: 0.85em;
    line-height: 1.5;
  }

  .msg-bubble :global(.md-codeblock code) {
    font-family: var(--font-mono, monospace);
  }

  .msg-bubble :global(.md-link) {
    color: var(--accent);
    text-decoration: none;
  }

  .msg-bubble :global(.md-link:hover) {
    text-decoration: underline;
  }

  .msg-bubble :global(.md-hr) {
    border: none;
    border-top: 1px solid var(--border);
    margin: 6px 0;
  }

  .msg-bubble :global(em) {
    font-style: italic;
  }

  /* ---- Mod cards (mentioned mods below assistant messages) ---- */
  .mod-cards {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 6px;
  }

  .mod-card {
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border-radius: 10px;
    border: 1px solid var(--border);
    background: var(--bg-base);
    cursor: pointer;
    font-size: 12px;
    color: var(--text-secondary);
    transition: border-color 0.15s, background 0.15s, box-shadow 0.15s;
    text-align: left;
    font-family: inherit;
    padding: 0;
  }

  .mod-card:hover:not(:disabled) {
    border-color: var(--accent);
    box-shadow: 0 2px 8px color-mix(in srgb, var(--accent) 15%, transparent);
  }

  .mod-card:disabled {
    cursor: default;
    opacity: 0.7;
  }

  .mod-card-img {
    width: 100%;
    height: 80px;
    object-fit: cover;
    border-radius: 9px 9px 0 0;
    display: block;
  }

  .mod-card-body {
    padding: 6px 8px;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .mod-card-name {
    font-weight: 600;
    color: var(--text);
    font-size: 12px;
    line-height: 1.3;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .mod-card-actions {
    display: flex;
    gap: 4px;
  }

  .mod-card-badge {
    font-size: 9px;
    font-weight: 600;
    padding: 1px 6px;
    border-radius: 4px;
    white-space: nowrap;
  }

  .mod-card-installed {
    background: color-mix(in srgb, #22c55e 15%, transparent);
    color: #22c55e;
  }

  .mod-card-nexus {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--accent);
  }

  .quick-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 6px;
  }

  .quick-action {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    color: var(--accent);
    border: 1px solid color-mix(in srgb, var(--accent) 25%, transparent);
    border-radius: 16px;
    padding: 5px 14px;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.15s ease;
    font-family: inherit;
    white-space: nowrap;
  }

  .quick-action:hover {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
    border-color: var(--accent);
  }

  .quick-action:active {
    transform: scale(0.96);
  }

  .quick-action-danger {
    background: color-mix(in srgb, #ef4444 10%, transparent);
    color: #ef4444;
    border-color: color-mix(in srgb, #ef4444 25%, transparent);
  }

  .quick-action-danger:hover {
    background: color-mix(in srgb, #ef4444 20%, transparent);
    border-color: #ef4444;
  }

  /* --- Crash banner --- */
  .crash-banner {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    margin: 8px;
    background: color-mix(in srgb, #ef4444 8%, var(--bg-base));
    border: 1px solid color-mix(in srgb, #ef4444 30%, transparent);
    border-radius: 10px;
    font-size: 12px;
    color: var(--text);
    animation: crash-banner-in 0.3s ease;
  }

  @keyframes crash-banner-in {
    from { opacity: 0; transform: translateY(-6px); }
    to   { opacity: 1; transform: translateY(0); }
  }

  .crash-banner-dot {
    width: 8px;
    height: 8px;
    min-width: 8px;
    border-radius: 50%;
    background: #ef4444;
    animation: crash-pulse 1.5s ease-in-out infinite;
  }

  @keyframes crash-pulse {
    0%, 100% { opacity: 1; transform: scale(1); }
    50%      { opacity: 0.5; transform: scale(0.8); }
  }

  .crash-banner-text {
    flex: 1;
    color: var(--text-secondary);
  }

  .crash-banner-btn {
    font-size: 11px;
    padding: 3px 10px;
    border-radius: 6px;
    border: 1px solid #ef4444;
    background: color-mix(in srgb, #ef4444 15%, transparent);
    color: #ef4444;
    cursor: pointer;
    font-weight: 500;
    font-family: inherit;
    transition: background 0.15s;
  }

  .crash-banner-btn:hover {
    background: color-mix(in srgb, #ef4444 25%, transparent);
  }

  .crash-banner-dismiss {
    background: none;
    border: none;
    color: var(--text-quaternary);
    cursor: pointer;
    padding: 2px;
    display: flex;
    align-items: center;
  }

  .crash-banner-dismiss:hover {
    color: var(--text-secondary);
  }

  /* --- Crash-related suggestion starter --- */
  .suggestion-crash {
    border-color: color-mix(in srgb, #ef4444 40%, transparent);
    background: color-mix(in srgb, #ef4444 8%, var(--bg-base));
    color: #ef4444;
  }

  .suggestion-crash:hover {
    background: color-mix(in srgb, #ef4444 18%, var(--bg-base));
    border-color: #ef4444;
    color: #ef4444;
  }

  /* --- Chat history persistence styles --- */

  .history-divider {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 12px 0 8px;
    font-size: 11px;
    color: var(--text-quaternary);
  }

  .history-divider::before,
  .history-divider::after {
    content: "";
    flex: 1;
    border-bottom: 1px solid var(--border-light, rgba(255, 255, 255, 0.06));
  }

  .history-divider span {
    white-space: nowrap;
    opacity: 0.7;
  }

  .chat-msg-history {
    opacity: 0.55;
  }

  .chat-msg-history:hover {
    opacity: 0.8;
  }

  .history-notice {
    text-align: center;
    font-size: 11px;
    color: var(--text-quaternary);
    margin: 8px 0 4px;
    opacity: 0.7;
    font-style: italic;
  }

  .health-score-card { padding: 12px; border-radius: 8px; margin: 8px 0; }
  .health-green { background: rgba(34, 197, 94, 0.1); border-left: 3px solid #22c55e; }
  .health-yellow { background: rgba(234, 179, 8, 0.1); border-left: 3px solid #eab308; }
  .health-red { background: rgba(239, 68, 68, 0.1); border-left: 3px solid #ef4444; }
  .health-score-number { font-size: 2em; font-weight: bold; }
  .health-score-label { font-size: 0.8em; opacity: 0.7; text-transform: uppercase; }
  .health-issue { font-size: 0.85em; margin-top: 4px; }
</style>
