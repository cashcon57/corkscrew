<script lang="ts">
  import { onMount } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import {
    getWabbajackModlists,
    fetchUrlText,
    parseWabbajackFile,
    downloadWabbajackFile,
    detectWabbajackTools,
  } from "$lib/api";
  import { showError, showSuccess, selectedGame } from "$lib/stores";
  import type { ModlistSummary, ParsedModlist, RequiredTool } from "$lib/types";
  import { marked } from "marked";
  import DOMPurify from "dompurify";
  import CompatibilityPanel from "$lib/components/CompatibilityPanel.svelte";
  import RequiredToolsPrompt from "$lib/components/RequiredToolsPrompt.svelte";
  import WabbajackLogo from "$lib/components/WabbajackLogo.svelte";

  let modlists = $state<ModlistSummary[]>([]);
  let filtered = $state<ModlistSummary[]>([]);
  let loading = $state(true);
  let searchQuery = $state("");
  let gameFilter = $state("all");
  let showNsfw = $state(false);
  let sortField = $state<"title" | "author" | "download_size" | "install_size">("title");
  let sortDirection = $state<"asc" | "desc">("asc");

  // Detail view state
  let selectedModlist = $state<ModlistSummary | null>(null);
  let readmeContent = $state("");
  let loadingDetail = $state(false);

  // Parsed file state (after loading .wabbajack)
  let parsedModlist = $state<ParsedModlist | null>(null);
  let wabbajackFilePath = $state<string | null>(null);
  let parsingFile = $state(false);

  // Download state
  let downloading = $state(false);
  let downloadError = $state<string | null>(null);

  // Tool detection state
  let pendingTools = $state<RequiredTool[]>([]);
  let showToolsPrompt = $state(false);

  // Install state
  let installing = $state(false);
  let installStep = $state("");

  // Derived unique games from the modlists
  const gameOptions = $derived.by(() => {
    const games = new Set(modlists.map((m) => m.game));
    return Array.from(games).sort();
  });

  $effect(() => {
    let result = modlists;

    // Filter by NSFW
    if (!showNsfw) {
      result = result.filter((m) => !m.nsfw);
    }

    // Filter by game
    if (gameFilter !== "all") {
      result = result.filter((m) => m.game === gameFilter);
    }

    // Filter by search
    if (searchQuery.trim()) {
      const q = searchQuery.toLowerCase();
      result = result.filter(
        (m) =>
          m.title.toLowerCase().includes(q) ||
          m.author.toLowerCase().includes(q) ||
          m.description.toLowerCase().includes(q) ||
          m.tags.some((t) => t.toLowerCase().includes(q))
      );
    }

    // Sort
    result = [...result].sort((a, b) => {
      let cmp = 0;
      if (sortField === "title") cmp = a.title.localeCompare(b.title);
      else if (sortField === "author") cmp = a.author.localeCompare(b.author);
      else cmp = (a[sortField] as number) - (b[sortField] as number);
      return sortDirection === "asc" ? cmp : -cmp;
    });

    filtered = result;
  });

  onMount(async () => {
    try {
      modlists = await getWabbajackModlists();
    } catch (e: unknown) {
      showError(`Failed to load modlists: ${e}`);
    } finally {
      loading = false;
    }
  });

  function formatSize(bytes: number): string {
    if (bytes === 0) return "N/A";
    const gb = bytes / (1024 * 1024 * 1024);
    if (gb >= 1) return `${gb.toFixed(1)} GB`;
    const mb = bytes / (1024 * 1024);
    return `${mb.toFixed(0)} MB`;
  }

  function gameDomainDisplay(domain: string): string {
    const map: Record<string, string> = {
      skyrim: "Skyrim LE",
      skyrimspecialedition: "Skyrim SE",
      skyrimvr: "Skyrim VR",
      fallout4: "Fallout 4",
      fallout4vr: "Fallout 4 VR",
      falloutnewvegas: "Fallout NV",
      fallout3: "Fallout 3",
      oblivion: "Oblivion",
      morrowind: "Morrowind",
      enderal: "Enderal",
      enderalspecialedition: "Enderal SE",
      cyberpunk2077: "Cyberpunk 2077",
      stardewvalley: "Stardew Valley",
      witcher3: "Witcher 3",
      thewitcher3: "Witcher 3",
      starfield: "Starfield",
      baldursgate3: "BG3",
    };
    return map[domain] || domain;
  }

  async function viewModlistDetail(modlist: ModlistSummary) {
    selectedModlist = modlist;
    readmeContent = "";
    loadingDetail = true;

    if (modlist.readme_url) {
      try {
        const raw = await fetchUrlText(modlist.readme_url);
        // If the content looks like a full HTML page, use it directly (sanitized).
        // Otherwise, treat it as markdown and render it.
        const trimmed = raw.trimStart();
        if (trimmed.startsWith("<!DOCTYPE") || trimmed.startsWith("<html")) {
          readmeContent = DOMPurify.sanitize(raw);
        } else {
          const html = await marked.parse(raw);
          readmeContent = DOMPurify.sanitize(html);
        }
      } catch (e: unknown) {
        const errMsg = String(e);
        const is404 = errMsg.includes("404");
        readmeContent = DOMPurify.sanitize(
          `<div style="color: var(--text-tertiary); text-align: center; padding: 24px 0;">` +
          `<p style="margin-bottom: 8px;">${is404 ? "Readme page is no longer available." : "Could not load readme."}</p>` +
          `<p style="font-size: 12px; opacity: 0.7;">${is404 ? "The author's readme link may have moved or been removed." : errMsg}</p>` +
          `</div>`,
        );
      }
    }

    loadingDetail = false;
  }

  function backToBrowse() {
    selectedModlist = null;
    readmeContent = "";
    parsedModlist = null;
    wabbajackFilePath = null;
  }

  /** Open a file picker to select a local .wabbajack file. */
  async function openLocalFile() {
    try {
      const selected = await open({
        multiple: false,
        filters: [{ name: "Wabbajack Modlist", extensions: ["wabbajack"] }],
      });
      if (!selected) return;

      const filePath = typeof selected === "string" ? selected : selected;
      await parseAndShowFile(filePath);
    } catch (e: unknown) {
      showError(`Failed to open file: ${e}`);
    }
  }

  /** Parse a .wabbajack file and show the summary view. */
  async function parseAndShowFile(filePath: string) {
    parsingFile = true;
    parsedModlist = null;
    wabbajackFilePath = filePath;
    // Clear gallery detail view
    selectedModlist = null;
    readmeContent = "";

    try {
      parsedModlist = await parseWabbajackFile(filePath);
    } catch (e: unknown) {
      showError(`Failed to parse .wabbajack file: ${e}`);
      parsedModlist = null;
      wabbajackFilePath = null;
    } finally {
      parsingFile = false;
    }
  }

  /** Group archives by source type for display. */
  function archiveBreakdown(modlist: ParsedModlist): { source: string; count: number; size: number }[] {
    const map = new Map<string, { count: number; size: number }>();
    for (const archive of modlist.archives) {
      const entry = map.get(archive.source_type) ?? { count: 0, size: 0 };
      entry.count += 1;
      entry.size += archive.size;
      map.set(archive.source_type, entry);
    }
    return Array.from(map.entries())
      .map(([source, data]) => ({ source, ...data }))
      .sort((a, b) => b.count - a.count);
  }

  /** Check for required tools and begin install. */
  async function handleBeginInstall() {
    const game = $selectedGame;
    if (!game || !wabbajackFilePath) return;

    try {
      const tools = await detectWabbajackTools(wabbajackFilePath, game.game_id, game.bottle_name);
      const uninstalled = tools.filter((t) => !t.is_detected);
      if (uninstalled.length > 0) {
        pendingTools = tools;
        showToolsPrompt = true;
        return;
      }
      proceedWithInstall();
    } catch (e: unknown) {
      showError(`Tool detection failed: ${e}`);
      proceedWithInstall();
    }
  }

  function proceedWithInstall() {
    showToolsPrompt = false;
    pendingTools = [];
    // The actual archive download + deploy pipeline would go here.
    // For now, show info about what the install would do.
    installing = true;
    installStep = "Modlist installation pipeline is in development. Archive download and deployment will be available in a future update.";
    showSuccess("Tool check complete. Full installation pipeline coming soon.");
    installing = false;
  }

  function sourceLabel(source: string): string {
    const labels: Record<string, string> = {
      Nexus: "Nexus Mods",
      HTTP: "Direct Download",
      GoogleDrive: "Google Drive",
      Mega: "Mega.nz",
      MediaFire: "MediaFire",
      ModDB: "ModDB",
      WabbajackCDN: "Wabbajack CDN",
      GameFile: "Game Files",
      Manual: "Manual Download",
    };
    return labels[source] ?? source;
  }

  function sourceColor(source: string): string {
    const colors: Record<string, string> = {
      Nexus: "var(--system-accent)",
      HTTP: "#22c55e",
      WabbajackCDN: "#a78bfa",
      GoogleDrive: "#f59e0b",
      Mega: "#ef4444",
      GameFile: "var(--text-tertiary)",
      Manual: "#f97316",
    };
    return colors[source] ?? "var(--text-secondary)";
  }

  async function handleDownloadWabbajack(modlist: ModlistSummary) {
    if (!modlist.download_url || downloading) return;
    downloading = true;
    downloadError = null;
    try {
      // Sanitize filename from title
      const safeName = modlist.title.replace(/[^a-zA-Z0-9_\- ]/g, "").trim();
      const filename = `${safeName}.wabbajack`;
      const filePath = await downloadWabbajackFile(modlist.download_url, filename);
      showSuccess(`Downloaded "${modlist.title}" — parsing...`);

      // Auto-parse the downloaded file
      parsedModlist = await parseWabbajackFile(filePath);
      wabbajackFilePath = filePath;
      selectedModlist = null;
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      downloadError = msg;
      showError(`Download failed: ${msg}`);
    } finally {
      downloading = false;
    }
  }

  /** Validate that a URL is a safe HTTP(S) URL before opening in browser. */
  function safeOpenUrl(url: string | null | undefined) {
    if (!url) return;
    try {
      const parsed = new URL(url);
      if (parsed.protocol === "http:" || parsed.protocol === "https:") {
        openUrl(url);
      }
    } catch {
      // ignore invalid URLs
    }
  }
</script>

<div class="modlists-page">
  {#if selectedModlist}
    <!-- Detail View -->
    <div class="detail-view">
      <div class="detail-header">
        <button class="btn btn-ghost" onclick={backToBrowse}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M19 12H5" />
            <polyline points="12 19 5 12 12 5" />
          </svg>
          Back to Browse
        </button>
      </div>

      <div class="detail-content">
        <!-- Title Section -->
        <div class="detail-title-section">
          <div class="detail-title-row">
            <h2 class="detail-name">{selectedModlist.title}</h2>
            <span class="game-badge">{gameDomainDisplay(selectedModlist.game)}</span>
            {#if selectedModlist.nsfw}
              <span class="nsfw-badge">NSFW</span>
            {/if}
          </div>
          <p class="detail-author">by {selectedModlist.author}</p>
          <span class="detail-version">v{selectedModlist.version}</span>
        </div>

        <!-- Stats Bar -->
        <div class="detail-stats-bar">
          <div class="detail-stat">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
              <polyline points="7 10 12 15 17 10" />
              <line x1="12" y1="15" x2="12" y2="3" />
            </svg>
            <span class="detail-stat-value">{formatSize(selectedModlist.download_size)}</span>
            <span class="detail-stat-label">Download</span>
          </div>
          <div class="detail-stat">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
            </svg>
            <span class="detail-stat-value">{formatSize(selectedModlist.install_size)}</span>
            <span class="detail-stat-label">Installed</span>
          </div>
          <div class="detail-stat">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z" />
            </svg>
            <span class="detail-stat-value">{selectedModlist.archive_count.toLocaleString()}</span>
            <span class="detail-stat-label">Archives</span>
          </div>
          <div class="detail-stat">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z" />
              <polyline points="13 2 13 9 20 9" />
            </svg>
            <span class="detail-stat-value">{selectedModlist.file_count.toLocaleString()}</span>
            <span class="detail-stat-label">Files</span>
          </div>
        </div>

        <!-- Description -->
        {#if selectedModlist.description}
          <div class="detail-section">
            <h3 class="detail-section-title">Description</h3>
            <div class="detail-description">
              <p>{selectedModlist.description}</p>
            </div>
          </div>
        {/if}

        <!-- Tags -->
        {#if selectedModlist.tags.length > 0}
          <div class="detail-section">
            <h3 class="detail-section-title">Tags</h3>
            <div class="detail-tags">
              {#each selectedModlist.tags as tag}
                <span class="tag">{tag}</span>
              {/each}
            </div>
          </div>
        {/if}

        <!-- Readme -->
        <div class="detail-section">
          <h3 class="detail-section-title">Readme</h3>
          {#if loadingDetail}
            <div class="readme-loading">
              <div class="spinner-sm-ring"></div>
              <span>Loading readme...</span>
            </div>
          {:else if readmeContent}
            <div class="rendered-markdown">
              {@html readmeContent}
            </div>
          {:else}
            <p class="detail-no-readme">No readme available for this modlist.</p>
          {/if}
        </div>

        <!-- Compatibility Check (Skyrim SE only) -->
        {#if selectedModlist.game === "skyrimspecialedition" && $selectedGame}
          <div class="detail-section">
            <CompatibilityPanel gameId={$selectedGame.game_id} bottleName={$selectedGame.bottle_name} />
          </div>
        {/if}

        <!-- Install Actions -->
        <div class="detail-install-bar">
          {#if selectedModlist.download_url}
            <button
              class="btn btn-primary btn-lg"
              onclick={() => selectedModlist && handleDownloadWabbajack(selectedModlist)}
              disabled={downloading}
            >
              {#if downloading}
                <span class="spinner spinner-sm"></span>
                Downloading...
              {:else}
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                  <polyline points="7 10 12 15 17 10" />
                  <line x1="12" y1="15" x2="12" y2="3" />
                </svg>
                Download .wabbajack
              {/if}
            </button>
            {#if downloadError}
              <p class="download-error">{downloadError}</p>
            {/if}
          {/if}
          <button class="btn btn-accent btn-lg" onclick={openLocalFile}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
            </svg>
            Open Local File
          </button>
        </div>
      </div>
    </div>

  {:else if parsedModlist}
    <!-- Parsed File View -->
    <div class="detail-view">
      <div class="detail-header">
        <button class="btn btn-ghost" onclick={backToBrowse}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M19 12H5" />
            <polyline points="12 19 5 12 12 5" />
          </svg>
          Back to Browse
        </button>
      </div>

      <div class="detail-content">
        <div class="detail-title-section">
          <div class="detail-title-row">
            <h2 class="detail-name">{parsedModlist.name || "Untitled Modlist"}</h2>
            <span class="game-badge">{parsedModlist.game_name}</span>
            {#if parsedModlist.is_nsfw}
              <span class="nsfw-badge">NSFW</span>
            {/if}
          </div>
          {#if parsedModlist.author}
            <p class="detail-author">by {parsedModlist.author}</p>
          {/if}
          {#if parsedModlist.version}
            <span class="detail-version">v{parsedModlist.version}</span>
          {/if}
        </div>

        {#if parsedModlist.description}
          <div class="detail-section">
            <h3 class="detail-section-title">Description</h3>
            <div class="detail-description"><p>{parsedModlist.description}</p></div>
          </div>
        {/if}

        <!-- Stats -->
        <div class="detail-stats-bar">
          <div class="detail-stat">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
              <polyline points="7 10 12 15 17 10" />
              <line x1="12" y1="15" x2="12" y2="3" />
            </svg>
            <span class="detail-stat-value">{formatSize(parsedModlist.total_download_size)}</span>
            <span class="detail-stat-label">Download</span>
          </div>
          <div class="detail-stat">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z" />
            </svg>
            <span class="detail-stat-value">{parsedModlist.archive_count.toLocaleString()}</span>
            <span class="detail-stat-label">Archives</span>
          </div>
          <div class="detail-stat">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z" />
              <polyline points="13 2 13 9 20 9" />
            </svg>
            <span class="detail-stat-value">{parsedModlist.directive_count.toLocaleString()}</span>
            <span class="detail-stat-label">Directives</span>
          </div>
        </div>

        <!-- Archive Sources Breakdown -->
        <div class="detail-section">
          <h3 class="detail-section-title">Archive Sources</h3>
          <div class="source-breakdown">
            {#each archiveBreakdown(parsedModlist) as item}
              <div class="source-row">
                <div class="source-info">
                  <span class="source-dot" style="background: {sourceColor(item.source)}"></span>
                  <span class="source-name">{sourceLabel(item.source)}</span>
                </div>
                <div class="source-stats">
                  <span class="source-count">{item.count}</span>
                  <span class="source-size">{formatSize(item.size)}</span>
                </div>
              </div>
            {/each}
          </div>
        </div>

        <!-- Directive Breakdown -->
        {#if Object.keys(parsedModlist.directive_breakdown).length > 0}
          <div class="detail-section">
            <h3 class="detail-section-title">Directive Breakdown</h3>
            <div class="directive-breakdown">
              {#each Object.entries(parsedModlist.directive_breakdown).sort((a, b) => b[1] - a[1]) as [kind, count]}
                <div class="directive-row">
                  <span class="directive-kind">{kind}</span>
                  <span class="directive-count">{count.toLocaleString()}</span>
                </div>
              {/each}
            </div>
          </div>
        {/if}

        <!-- Compatibility Check -->
        {#if parsedModlist.game_name.includes("Skyrim") && $selectedGame}
          <div class="detail-section">
            <CompatibilityPanel gameId={$selectedGame.game_id} bottleName={$selectedGame.bottle_name} />
          </div>
        {/if}

        <!-- Install -->
        <div class="detail-install-bar">
          {#if !$selectedGame}
            <p class="install-note">Select a game in the sidebar to begin installation.</p>
          {:else}
            <button
              class="btn btn-primary btn-lg"
              onclick={handleBeginInstall}
              disabled={installing}
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                <polyline points="7 10 12 15 17 10" />
                <line x1="12" y1="15" x2="12" y2="3" />
              </svg>
              {installing ? "Installing..." : "Begin Install"}
            </button>
          {/if}
          {#if installStep}
            <p class="install-note">{installStep}</p>
          {/if}
        </div>
      </div>
    </div>

    {#if showToolsPrompt && $selectedGame}
      <RequiredToolsPrompt
        tools={pendingTools}
        gameId={$selectedGame.game_id}
        bottleName={$selectedGame.bottle_name}
        oncontinue={proceedWithInstall}
        oncancel={() => { showToolsPrompt = false; }}
      />
    {/if}

  {:else if parsingFile}
    <div class="loading-container">
      <div class="loading-card">
        <div class="spinner"><div class="spinner-ring"></div></div>
        <div class="loading-text">
          <p class="loading-title">Parsing modlist</p>
          <p class="loading-detail">Reading .wabbajack file...</p>
        </div>
      </div>
    </div>

  {:else}
    <!-- Browse View -->
    <header class="page-header">
      <div class="header-text">
        <h2 class="page-title"><WabbajackLogo size={22} /> Wabbajack Lists</h2>
        <p class="page-subtitle">
          Browse Wabbajack modlists — curated, pre-configured mod setups
        </p>
      </div>
      <div class="header-right">
        <button class="btn btn-accent btn-sm" onclick={openLocalFile}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
          </svg>
          Open .wabbajack
        </button>
        {#if !loading}
          <div class="stat-pill">
            <span class="stat-value">{filtered.length}</span>
            <span class="stat-label">{filtered.length === 1 ? "List" : "Lists"}</span>
          </div>
        {/if}
      </div>
    </header>

    {#if loading}
      <div class="loading-container">
        <div class="loading-card">
          <div class="spinner"><div class="spinner-ring"></div></div>
          <div class="loading-text">
            <p class="loading-title">Fetching modlists</p>
            <p class="loading-detail">Loading gallery from Wabbajack repositories...</p>
          </div>
        </div>
      </div>
    {:else}
      <!-- Filters -->
      <div class="filters-bar">
        <input
          type="text"
          class="search-input"
          placeholder="Search modlists..."
          bind:value={searchQuery}
        />
        <select class="filter-select" bind:value={gameFilter}>
          <option value="all">All Games</option>
          {#each gameOptions as game}
            <option value={game}>{gameDomainDisplay(game)}</option>
          {/each}
        </select>
        <label class="nsfw-toggle">
          <input type="checkbox" bind:checked={showNsfw} />
          <span>NSFW</span>
        </label>
        <div class="sort-group">
          <select class="filter-select" bind:value={sortField}>
            <option value="title">Sort: Name</option>
            <option value="author">Sort: Author</option>
            <option value="download_size">Sort: Download Size</option>
            <option value="install_size">Sort: Install Size</option>
          </select>
          <button
            class="sort-direction-btn"
            onclick={() => sortDirection = sortDirection === "asc" ? "desc" : "asc"}
            title={sortDirection === "asc" ? "Ascending" : "Descending"}
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              {#if sortDirection === "asc"}
                <path d="M12 5v14M5 12l7-7 7 7" />
              {:else}
                <path d="M12 5v14M5 12l7 7 7-7" />
              {/if}
            </svg>
          </button>
        </div>
      </div>

      {#if filtered.length === 0}
        <div class="empty-state">
          <p class="empty-title">No modlists found</p>
          <p class="empty-detail">
            {#if searchQuery || gameFilter !== "all"}
              Try adjusting your search or filters.
            {:else}
              No modlists are currently available.
            {/if}
          </p>
        </div>
      {:else}
        <div class="modlist-grid">
          {#each filtered as modlist, i}
            <div class="modlist-card" style="animation-delay: {Math.min(i, 20) * 30}ms">
              {#if modlist.image_url}
                <div class="card-image">
                  <img src={modlist.image_url} alt={modlist.title} loading="lazy" />
                </div>
              {/if}
              <div class="card-body">
                <div class="card-top">
                  <span class="game-badge">{gameDomainDisplay(modlist.game)}</span>
                  {#if modlist.nsfw}
                    <span class="nsfw-badge">NSFW</span>
                  {/if}
                  <span class="version-badge">v{modlist.version}</span>
                </div>

                <h3 class="card-title">{modlist.title}</h3>
                <p class="card-author">by {modlist.author}</p>

                {#if modlist.description}
                  <p class="card-desc">{modlist.description}</p>
                {/if}

                {#if modlist.tags.length > 0}
                  <div class="card-tags">
                    {#each modlist.tags.slice(0, 5) as tag}
                      <span class="tag">{tag}</span>
                    {/each}
                  </div>
                {/if}

                <div class="card-stats">
                  <div class="stat-item">
                    <span class="stat-num">{formatSize(modlist.download_size)}</span>
                    <span class="stat-lbl">Download</span>
                  </div>
                  <div class="stat-item">
                    <span class="stat-num">{formatSize(modlist.install_size)}</span>
                    <span class="stat-lbl">Install</span>
                  </div>
                  <div class="stat-item">
                    <span class="stat-num">{modlist.archive_count.toLocaleString()}</span>
                    <span class="stat-lbl">Archives</span>
                  </div>
                  <div class="stat-item">
                    <span class="stat-num">{modlist.file_count.toLocaleString()}</span>
                    <span class="stat-lbl">Files</span>
                  </div>
                </div>

                <div class="card-actions">
                  <button
                    class="btn btn-accent btn-sm"
                    onclick={() => viewModlistDetail(modlist)}
                  >
                    View Details
                  </button>
                </div>
              </div>
            </div>
          {/each}
        </div>
      {/if}
    {/if}
  {/if}
</div>

<style>
  .modlists-page {
    padding: var(--space-2) 0 var(--space-12) 0;
  }

  .page-header {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    margin-bottom: var(--space-8);
    padding-bottom: var(--space-4);
    border-bottom: 1px solid var(--separator);
  }

  .page-title {
    font-size: 28px;
    font-weight: 700;
    color: var(--text-primary);
    letter-spacing: -0.025em;
  }

  .page-subtitle {
    font-size: 14px;
    color: var(--text-secondary);
    margin-top: var(--space-1);
  }

  .header-stats {
    display: flex;
    gap: var(--space-3);
  }

  .stat-pill {
    display: flex;
    align-items: baseline;
    gap: var(--space-1);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    padding: var(--space-2) var(--space-4);
  }

  .stat-value {
    font-size: 18px;
    font-weight: 700;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .stat-label {
    font-size: 12px;
    color: var(--text-tertiary);
    font-weight: 500;
  }

  /* Loading */
  .loading-container {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 280px;
  }

  .loading-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-6);
    padding: var(--space-12) var(--space-10);
  }

  .spinner { width: 36px; height: 36px; }

  .spinner-ring {
    width: 100%;
    height: 100%;
    border: 2.5px solid var(--separator);
    border-top-color: var(--system-accent);
    border-radius: 50%;
    animation: spin 0.9s cubic-bezier(0.4, 0, 0.2, 1) infinite;
  }

  @keyframes spin { to { transform: rotate(360deg); } }

  .loading-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
    text-align: center;
  }

  .loading-detail {
    font-size: 13px;
    color: var(--text-tertiary);
    text-align: center;
    margin-top: var(--space-1);
  }

  /* Filters */
  .filters-bar {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-bottom: var(--space-6);
  }

  .search-input {
    flex: 1;
    padding: var(--space-2) var(--space-3);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    color: var(--text-primary);
    font-size: 14px;
    outline: none;
    transition: border-color var(--duration-fast) var(--ease);
  }

  .search-input:focus {
    border-color: var(--system-accent);
  }

  .search-input::placeholder {
    color: var(--text-tertiary);
  }

  .filter-select {
    padding: var(--space-2) var(--space-3);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    color: var(--text-primary);
    font-size: 13px;
    outline: none;
    cursor: pointer;
    min-width: 140px;
  }

  .filter-select:focus {
    border-color: var(--system-accent);
  }

  .nsfw-toggle {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: 13px;
    color: var(--text-secondary);
    cursor: pointer;
    white-space: nowrap;
  }

  .nsfw-toggle input {
    accent-color: var(--system-accent);
  }

  .sort-group {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    margin-left: auto;
  }

  .sort-direction-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border-radius: var(--radius-sm);
    background: var(--surface);
    border: 1px solid var(--separator);
    color: var(--text-secondary);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
  }

  .sort-direction-btn:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  /* Grid */
  .modlist-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
    gap: var(--space-4);
  }

  /* Cards */
  .modlist-card {
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-lg);
    overflow: hidden;
    box-shadow: var(--glass-edge-shadow);
    transition: border-color var(--duration-fast) var(--ease),
                box-shadow var(--duration-fast) var(--ease);
    animation: cardFadeIn var(--duration-slow) var(--ease) both;
    display: flex;
    flex-direction: column;
  }

  .modlist-card:hover {
    border-color: rgba(255, 255, 255, 0.12);
  }

  @keyframes cardFadeIn {
    from { opacity: 0; transform: translateY(6px); }
    to { opacity: 1; transform: translateY(0); }
  }

  .card-image {
    width: 100%;
    height: 140px;
    overflow: hidden;
    background: var(--bg-secondary);
  }

  .card-image img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .card-body {
    padding: var(--space-4) var(--space-5);
    display: flex;
    flex-direction: column;
    flex: 1;
  }

  .card-top {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-2);
    flex-wrap: wrap;
  }

  .game-badge {
    font-size: 11px;
    font-weight: 600;
    color: var(--system-accent);
    background: var(--system-accent-subtle);
    padding: 1px var(--space-2);
    border-radius: var(--radius-sm);
  }

  .nsfw-badge {
    font-size: 10px;
    font-weight: 700;
    color: var(--red);
    background: var(--red-subtle);
    padding: 1px var(--space-2);
    border-radius: var(--radius-sm);
  }

  .version-badge {
    font-size: 10px;
    font-weight: 500;
    color: var(--text-tertiary);
    background: rgba(255, 255, 255, 0.05);
    padding: 1px var(--space-2);
    border-radius: var(--radius-sm);
  }

  .card-title {
    font-size: 15px;
    font-weight: 700;
    color: var(--text-primary);
    line-height: 1.3;
    margin-bottom: 2px;
  }

  .card-author {
    font-size: 12px;
    color: var(--text-secondary);
    margin-bottom: var(--space-2);
  }

  .card-desc {
    font-size: 12px;
    color: var(--text-tertiary);
    line-height: 1.5;
    margin-bottom: var(--space-3);
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .card-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-bottom: var(--space-3);
  }

  .tag {
    font-size: 10px;
    font-weight: 500;
    color: var(--text-secondary);
    background: rgba(255, 255, 255, 0.06);
    padding: 1px 6px;
    border-radius: var(--radius-sm);
  }

  .card-stats {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: var(--space-2);
    padding: var(--space-3) 0;
    border-top: 1px solid var(--separator);
    margin-bottom: var(--space-3);
  }

  .stat-item {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
  }

  .stat-num {
    font-size: 12px;
    font-weight: 700;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .stat-lbl {
    font-size: 10px;
    color: var(--text-tertiary);
    font-weight: 500;
  }

  .card-actions {
    display: flex;
    gap: var(--space-2);
    margin-top: auto;
  }

  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-1);
    border-radius: var(--radius);
    font-weight: 600;
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
  }

  .btn-sm {
    padding: var(--space-1) var(--space-3);
    font-size: 12px;
    min-height: 28px;
  }

  .btn-lg {
    padding: var(--space-3) var(--space-6);
    font-size: 14px;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-accent {
    background: var(--system-accent);
    color: #fff;
    flex: 1;
  }

  .btn-accent:hover:not(:disabled) {
    filter: brightness(1.1);
  }

  .btn-primary {
    background: var(--system-accent);
    color: var(--system-accent-on);
    padding: var(--space-2) var(--space-5);
    border-radius: var(--radius);
  }

  .btn-primary:hover:not(:disabled) {
    background: var(--system-accent-hover);
    box-shadow: 0 1px 6px rgba(0, 122, 255, 0.25);
  }

  .btn-ghost {
    background: transparent;
    color: var(--text-secondary);
    padding: var(--space-2) var(--space-3);
    font-size: 13px;
    font-weight: 500;
  }

  .btn-ghost:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  /* Empty state */
  .empty-state {
    text-align: center;
    padding: var(--space-12) var(--space-8);
    border: 1px dashed rgba(255, 255, 255, 0.1);
    border-radius: var(--radius-lg);
    background: rgba(255, 255, 255, 0.015);
    box-shadow: var(--glass-edge-shadow);
  }

  .empty-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-secondary);
    margin-bottom: var(--space-2);
  }

  .empty-detail {
    font-size: 13px;
    color: var(--text-tertiary);
    max-width: 360px;
    margin: 0 auto;
    line-height: 1.55;
  }

  /* ---- Detail View ---- */

  .detail-view {
    animation: fadeIn var(--duration-normal) var(--ease);
  }

  @keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .detail-header {
    margin-bottom: var(--space-4);
  }

  .detail-content {
    max-width: 800px;
  }

  .detail-title-section {
    margin-bottom: var(--space-6);
    padding-bottom: var(--space-4);
    border-bottom: 1px solid var(--separator);
  }

  .detail-title-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
    margin-bottom: var(--space-1);
  }

  .detail-name {
    font-size: 26px;
    font-weight: 700;
    color: var(--text-primary);
    letter-spacing: -0.02em;
  }

  .detail-author {
    font-size: 14px;
    color: var(--text-secondary);
    margin-bottom: var(--space-1);
  }

  .detail-version {
    font-size: 12px;
    color: var(--text-tertiary);
    background: rgba(255, 255, 255, 0.05);
    padding: 2px var(--space-2);
    border-radius: var(--radius-sm);
  }

  /* Stats Bar */
  .detail-stats-bar {
    display: flex;
    gap: var(--space-6);
    padding: var(--space-4) var(--space-5);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-lg);
    margin-bottom: var(--space-6);
    box-shadow: var(--glass-edge-shadow);
  }

  .detail-stat {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-tertiary);
  }

  .detail-stat svg {
    opacity: 0.6;
  }

  .detail-stat-value {
    font-size: 14px;
    font-weight: 700;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .detail-stat-label {
    font-size: 12px;
    color: var(--text-tertiary);
    font-weight: 500;
  }

  /* Detail Sections */
  .detail-section {
    margin-bottom: var(--space-6);
  }

  .detail-section-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    margin-bottom: var(--space-3);
  }

  .detail-description {
    font-size: 14px;
    line-height: 1.7;
    color: var(--text-secondary);
  }

  .detail-tags {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .detail-tags .tag {
    font-size: 12px;
    padding: var(--space-1) var(--space-3);
  }

  .readme-loading {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-6);
    color: var(--text-tertiary);
    font-size: 13px;
  }

  .spinner-sm-ring {
    width: 18px;
    height: 18px;
    border: 2px solid var(--separator);
    border-top-color: var(--system-accent);
    border-radius: 50%;
    animation: spin 0.9s cubic-bezier(0.4, 0, 0.2, 1) infinite;
  }

  .detail-no-readme {
    font-size: 13px;
    color: var(--text-tertiary);
    font-style: italic;
  }

  .detail-install-bar {
    padding: var(--space-6) 0;
    border-top: 1px solid var(--separator);
    margin-top: var(--space-4);
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    align-items: center;
  }

  .install-note {
    font-size: 13px;
    color: var(--text-tertiary);
    flex-basis: 100%;
    margin-top: var(--space-1);
  }

  .header-right {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  /* Source breakdown */
  .source-breakdown {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .source-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-2) var(--space-3);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
  }

  .source-info {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .source-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .source-name {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-primary);
  }

  .source-stats {
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }

  .source-count {
    font-size: 13px;
    font-weight: 700;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .source-size {
    font-size: 12px;
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
    min-width: 60px;
    text-align: right;
  }

  /* Directive breakdown */
  .directive-breakdown {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .directive-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-1) var(--space-3);
  }

  .directive-kind {
    font-size: 13px;
    color: var(--text-secondary);
  }

  .directive-count {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .download-error {
    color: #ef4444;
    font-size: 13px;
    margin-top: 8px;
    padding: 8px 12px;
    background: rgba(239, 68, 68, 0.1);
    border-radius: 6px;
    border: 1px solid rgba(239, 68, 68, 0.2);
  }
</style>
