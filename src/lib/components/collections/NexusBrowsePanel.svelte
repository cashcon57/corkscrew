<script lang="ts">
  import { selectedGame, showError, showSuccess } from "$lib/stores";
  import type { DetectedGame, NexusModInfo, NexusCategory, NexusModFile, NexusSearchResult } from "$lib/types";
  import {
    browseNexusMods,
    searchNexusMods,
    getGameCategories,
    getInstalledMods,
    getModFiles,
    getNexusModDetail,
    downloadAndInstallNexusMod,
  } from "$lib/api";
  import { listen } from "@tauri-apps/api/event";
  import { goto } from "$app/navigation";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import DOMPurify from "dompurify";
  import { bbcodeToHtml } from "$lib/bbcode";
  import NexusLogo from "$lib/components/NexusLogo.svelte";
  import WebViewToggle from "$lib/components/WebViewToggle.svelte";

  interface Props {
    game: DetectedGame;
    account: { connected: boolean; is_premium?: boolean; name?: string; avatar?: string | null; auth_type?: string } | null;
    allDetectedGames?: DetectedGame[];
    onModInstalled?: () => Promise<void>;
  }

  let { game, account, allDetectedGames = [], onModInstalled }: Props = $props();

  const gameSlugMap: Record<string, string> = {
    skyrimse: "skyrimspecialedition",
    skyrim: "skyrim",
    fallout4: "fallout4",
    fallout3: "fallout3",
    falloutnv: "newvegas",
    oblivion: "oblivion",
    morrowind: "morrowind",
    starfield: "starfield",
    enderal: "enderal",
    enderalse: "enderalspecialedition",
  };

  // ---- Browse game override ----
  let browseGameOverride = $state<string | null>(null);

  function getGameSlug(): string {
    if (browseGameOverride) return browseGameOverride;
    return gameSlugMap[game.game_id] ?? game.game_id;
  }

  function getBrowseGameName(): string {
    if (browseGameOverride) {
      const g = allDetectedGames.find(g => (gameSlugMap[g.game_id] ?? g.game_id) === browseGameOverride);
      return g?.display_name ?? gameDomainDisplay(browseGameOverride);
    }
    return game.display_name ?? "your game";
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
      starfield: "Starfield",
      baldursgate3: "BG3",
      hogwartslegacy: "Hogwarts Legacy",
    };
    return map[domain] || domain;
  }

  // ---- Mod Browse State ----
  const BROWSE_PAGE_SIZE = 20;
  let browseMods = $state<NexusModInfo[]>([]);
  let browseModsLoading = $state(false);
  let browseModsSearch = $state("");
  let browseNsfwFilter = $state<"hide" | "show" | "only">("hide");
  let browseModsSort = $state<"endorsements" | "downloads" | "name" | "updated" | "createdAt">("endorsements");
  let browseModsTotalCount = $state(0);
  let browseModsOffset = $state(0);
  let browseModsHasMore = $state(false);
  let browseCategories = $state<NexusCategory[]>([]);
  let browseCategoryId = $state<number | null>(null);
  let browseInstalledNexusIds = $state<Set<number>>(new Set());
  let browseSearchTimer: ReturnType<typeof setTimeout> | null = null;
  let browseUseGraphQL = $state(true);

  // Advanced browse filters
  let browseAuthorFilter = $state("");
  let browseUpdatePeriod = $state<"all" | "24h" | "1w" | "1m">("all");
  let browseMinDownloads = $state<number | null>(null);
  let browseMinEndorsements = $state<number | null>(null);
  let showBrowseAdvancedFilters = $state(false);
  let browseAuthorTimer: ReturnType<typeof setTimeout> | null = null;

  // WebView toggle state
  let browseWebviewToggle: WebViewToggle | null = $state(null);
  let browseWebviewAnchor: HTMLElement | null = $state(null);
  let browseViewMode = $state<"app" | "website">("app");

  // Mod detail view state
  let selectedBrowseMod = $state<NexusModInfo | null>(null);
  let browseModDetail = $state<NexusModInfo | null>(null);
  let browseModFiles = $state<NexusModFile[]>([]);
  let loadingModDetail = $state(false);
  let renderedModDescription = $state("");

  // Download & file picker state
  let showFilePicker = $state(false);
  let filePickerMod = $state<NexusModInfo | null>(null);
  let filePickerFiles = $state<NexusModFile[]>([]);
  let loadingFiles = $state(false);
  let downloadingFile = $state<number | null>(null);
  let downloadProgress = $state<{ downloaded: number; total: number } | null>(null);

  const browseActiveFilterCount = $derived(
    (browseAuthorFilter.trim() ? 1 : 0) +
    (browseUpdatePeriod !== "all" ? 1 : 0) +
    (browseMinDownloads !== null ? 1 : 0) +
    (browseMinEndorsements !== null ? 1 : 0) +
    (browseCategoryId !== null ? 1 : 0)
  );

  const browseTotalPages = $derived(Math.max(1, Math.ceil(browseModsTotalCount / BROWSE_PAGE_SIZE)));
  const browseCurrentPage = $derived(Math.floor(browseModsOffset / BROWSE_PAGE_SIZE) + 1);

  // Build hierarchical category display list
  const browseCategoryOptions = $derived.by(() => {
    if (browseCategories.length === 0) return [];
    const topLevel = browseCategories.filter(c => !c.parent_category);
    const result: { id: number; name: string; depth: number }[] = [];
    for (const cat of topLevel.sort((a, b) => a.name.localeCompare(b.name))) {
      result.push({ id: cat.category_id, name: cat.name, depth: 0 });
      const children = browseCategories
        .filter(c => c.parent_category === cat.category_id)
        .sort((a, b) => a.name.localeCompare(b.name));
      for (const child of children) {
        result.push({ id: child.category_id, name: child.name, depth: 1 });
      }
    }
    return result;
  });

  // ---- Auto-init when game changes ----
  let browseInitializedForGame = $state<string | null>(null);
  $effect(() => {
    const g = game;
    const connected = account?.connected;
    if (g && connected) {
      const gameKey = `${g.game_id}:${g.bottle_name}`;
      if (browseInitializedForGame !== gameKey) {
        browseInitializedForGame = gameKey;
        loadBrowseMods();
        loadBrowseCategories();
        loadBrowseInstalledIds();
      }
    }
  });

  function computeUpdatedSince(period: "all" | "24h" | "1w" | "1m"): string | null {
    if (period === "all") return null;
    const msMap: Record<string, number> = {
      "24h": 24 * 60 * 60 * 1000,
      "1w": 7 * 24 * 60 * 60 * 1000,
      "1m": 30 * 24 * 60 * 60 * 1000,
    };
    return new Date(Date.now() - msMap[period]).toISOString();
  }

  function clearAllBrowseFilters() {
    browseAuthorFilter = "";
    browseUpdatePeriod = "all";
    browseMinDownloads = null;
    browseMinEndorsements = null;
    browseCategoryId = null;
    loadBrowseMods();
  }

  function browseAuthorDebounced() {
    if (browseAuthorTimer) clearTimeout(browseAuthorTimer);
    browseAuthorTimer = setTimeout(() => loadBrowseMods(), 400);
  }

  async function loadBrowseMods(resetOffset = true) {
    const slug = getGameSlug();
    if (!slug) return;
    if (resetOffset) browseModsOffset = 0;
    browseModsLoading = true;

    try {
      if (browseUseGraphQL) {
        const sortMap: Record<string, string> = {
          endorsements: "endorsements",
          downloads: "downloads",
          name: "name",
          updated: "updated",
          createdAt: "createdAt",
        };
        const result = await searchNexusMods(
          slug,
          browseModsSearch.trim() || null,
          sortMap[browseModsSort] ?? "endorsements",
          browseModsSort === "name" ? "ASC" : "DESC",
          BROWSE_PAGE_SIZE,
          browseModsOffset,
          browseNsfwFilter !== "hide",
          browseCategoryId || null,
          browseAuthorFilter.trim() || null,
          computeUpdatedSince(browseUpdatePeriod),
          browseMinDownloads,
          browseMinEndorsements,
        );
        let mods = result.mods;
        if (browseNsfwFilter === "only") {
          mods = mods.filter(m => m.adult_content);
        }
        browseMods = mods;
        browseModsTotalCount = result.total_count;
        browseModsHasMore = result.has_more;
      } else {
        let mods = await browseNexusMods(slug, "all");
        if (browseNsfwFilter === "only") {
          mods = mods.filter(m => m.adult_content);
        }
        browseMods = mods;
        browseModsTotalCount = browseMods.length;
        browseModsHasMore = false;
      }
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      if (browseUseGraphQL && msg.includes("mod search may not be available")) {
        browseUseGraphQL = false;
        await loadBrowseMods(resetOffset);
        return;
      }
      showError(`Failed to browse mods: ${msg}`);
      browseMods = [];
      browseModsTotalCount = 0;
      browseModsHasMore = false;
    } finally {
      browseModsLoading = false;
    }
  }

  async function loadBrowseCategories() {
    const slug = getGameSlug();
    if (!slug) return;
    try {
      browseCategories = await getGameCategories(slug);
    } catch {
      browseCategories = [];
    }
  }

  async function loadBrowseInstalledIds() {
    try {
      const mods = await getInstalledMods(game.game_id, game.bottle_name);
      browseInstalledNexusIds = new Set(
        mods.filter(m => m.nexus_mod_id != null).map(m => m.nexus_mod_id as number)
      );
    } catch {
      browseInstalledNexusIds = new Set();
    }
  }

  function browseGoToPage(page: number) {
    browseModsOffset = (page - 1) * BROWSE_PAGE_SIZE;
    loadBrowseMods(false);
  }

  function browseSearchDebounced() {
    if (browseSearchTimer) clearTimeout(browseSearchTimer);
    browseSearchTimer = setTimeout(() => loadBrowseMods(), 400);
  }

  function formatDownloads(n: number): string {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
    return n.toString();
  }

  function formatFileSize(kb: number): string {
    if (kb >= 1_048_576) return `${(kb / 1_048_576).toFixed(1)} GB`;
    if (kb >= 1024) return `${(kb / 1024).toFixed(1)} MB`;
    return `${kb} KB`;
  }

  /** Validate that a URL is a safe HTTP(S) URL before opening in browser. */
  function safeOpenUrl(url: string | null | undefined) {
    if (!url) return;
    try {
      const parsed = new URL(url);
      if (parsed.protocol === "http:" || parsed.protocol === "https:") {
        openUrl(url);
      } else {
        showError(`Blocked unsafe URL scheme: ${parsed.protocol}`);
      }
    } catch {
      showError("Invalid URL");
    }
  }

  async function openModDetail(mod: NexusModInfo) {
    const slug = getGameSlug();
    if (!slug) return;
    selectedBrowseMod = mod;
    browseModDetail = null;
    browseModFiles = [];
    renderedModDescription = "";
    loadingModDetail = true;
    try {
      const [detail, files] = await Promise.all([
        getNexusModDetail(slug, mod.mod_id),
        getModFiles(slug, mod.mod_id).catch(() => [] as NexusModFile[]),
      ]);
      browseModDetail = detail;
      const categoryOrder: Record<string, number> = { main: 0, update: 1, optional: 2, miscellaneous: 3, old_version: 4 };
      browseModFiles = files
        .filter((f: NexusModFile) => f.category !== "deleted" && f.category !== "archived")
        .sort((a: NexusModFile, b: NexusModFile) => (categoryOrder[a.category] ?? 5) - (categoryOrder[b.category] ?? 5));
      if (detail.description) {
        renderedModDescription = DOMPurify.sanitize(bbcodeToHtml(detail.description));
      }
    } catch (e) {
      showError(`Failed to load mod details: ${e}`);
      selectedBrowseMod = null;
    } finally {
      loadingModDetail = false;
    }
  }

  /** Public method: open a mod detail view by mod stub (used for LLM events). */
  export function openModById(mod: NexusModInfo) {
    openModDetail(mod);
  }

  function backToBrowseModList() {
    selectedBrowseMod = null;
    browseModDetail = null;
    browseModFiles = [];
    renderedModDescription = "";
  }

  /** Intercept clicks on links inside rendered markdown/HTML and open them externally. */
  function handleRenderedLinkClick(e: MouseEvent) {
    const target = (e.target as HTMLElement)?.closest("a");
    if (!target) return;
    const href = target.getAttribute("href");
    if (href) {
      e.preventDefault();
      e.stopPropagation();
      safeOpenUrl(href);
    }
  }

  function cycleNsfwFilter(current: "hide" | "show" | "only"): "hide" | "show" | "only" {
    if (current === "hide") return "show";
    if (current === "show") return "only";
    return "hide";
  }

  function nsfwLabel(state: "hide" | "show" | "only"): string {
    if (state === "hide") return "NSFW Off";
    if (state === "show") return "NSFW On";
    return "NSFW Only";
  }

  function nsfwIcon(state: "hide" | "show" | "only"): string {
    if (state === "hide") return "";
    if (state === "show") return "\u2713";
    return "\u2500";
  }

  // --- Download & File Picker ---
  async function openFilePicker(mod: NexusModInfo) {
    const slug = getGameSlug();
    if (!slug) return;
    filePickerMod = mod;
    showFilePicker = true;
    loadingFiles = true;
    try {
      const files = await getModFiles(slug, mod.mod_id);
      const categoryOrder: Record<string, number> = { main: 0, update: 1, optional: 2, miscellaneous: 3, old_version: 4 };
      filePickerFiles = files
        .filter(f => f.category !== "deleted" && f.category !== "archived")
        .sort((a, b) => (categoryOrder[a.category] ?? 5) - (categoryOrder[b.category] ?? 5));
    } catch (e) {
      showError(`Failed to load mod files: ${e}`);
      showFilePicker = false;
      filePickerMod = null;
    } finally {
      loadingFiles = false;
    }
  }

  function closeFilePicker() {
    showFilePicker = false;
    filePickerMod = null;
    filePickerFiles = [];
    downloadingFile = null;
    downloadProgress = null;
  }

  async function handleDownloadFile(file: NexusModFile) {
    if (!filePickerMod) return;
    const slug = getGameSlug();
    if (!slug) return;

    downloadingFile = file.file_id;
    downloadProgress = { downloaded: 0, total: 0 };

    const unlisten = await listen<{ downloaded: number; total: number; mod_name: string }>("download-progress", (e) => {
      downloadProgress = { downloaded: e.payload.downloaded, total: e.payload.total };
    });

    try {
      await downloadAndInstallNexusMod(slug, filePickerMod.mod_id, file.file_id, game.game_id, game.bottle_name);
      showSuccess(`Installed "${filePickerMod.name}" successfully`);
      browseInstalledNexusIds = new Set([...browseInstalledNexusIds, filePickerMod.mod_id]);
      closeFilePicker();
      if (onModInstalled) {
        await onModInstalled();
      }
    } catch (e) {
      showError(`Download failed: ${e}`);
    } finally {
      unlisten();
      downloadingFile = null;
      downloadProgress = null;
    }
  }

  /** Cleanup timers on destroy. */
  import { onDestroy } from "svelte";
  onDestroy(() => {
    if (browseSearchTimer) { clearTimeout(browseSearchTimer); browseSearchTimer = null; }
    if (browseAuthorTimer) { clearTimeout(browseAuthorTimer); browseAuthorTimer = null; }
  });
</script>

<header class="page-header">
  <div class="header-text">
    <h2 class="page-title"><NexusLogo size={22} /> Browse Nexus</h2>
    <p class="page-subtitle">Discover mods on NexusMods for {getBrowseGameName()}</p>
  </div>
  <div class="header-right">
    <WebViewToggle
      bind:this={browseWebviewToggle}
      url={`https://www.nexusmods.com/${getGameSlug()}/mods/`}
      defaultMode={account?.connected ? "app" : "website"}
      onModeChange={(m) => browseViewMode = m}
      anchorEl={browseWebviewAnchor}
    />
    {#if !browseModsLoading && browseModsTotalCount > 0 && browseViewMode === "app"}
      <div class="stat-pill">
        <span class="stat-value">{browseModsTotalCount.toLocaleString()}</span>
        <span class="stat-label">{browseModsTotalCount === 1 ? "mod" : "mods"}</span>
      </div>
    {/if}
  </div>
</header>

{#if browseViewMode === "website"}
  <div class="webview-placeholder" bind:this={browseWebviewAnchor}>
    <p class="webview-hint">Browsing NexusMods directly. Switch to "In-App" to use built-in search and filters.</p>
  </div>
{:else if !account?.connected}
  <div class="premium-gate">
    <div class="premium-gate-icon">
      <NexusLogo size={40} />
    </div>
    <h3 class="premium-gate-title">Connect to NexusMods</h3>
    <p class="premium-gate-desc">Connect your NexusMods account in Settings to browse mods in-app.</p>
    <button class="btn btn-accent" onclick={() => goto("/settings")}>Go to Settings</button>
    <p class="premium-gate-hint">Or switch to "Website" above to browse NexusMods directly.</p>
  </div>
{:else}
  {#if selectedBrowseMod}
    <!-- Mod Detail View -->
    <div class="detail-view">
      <div class="detail-header">
        <button class="btn btn-ghost" onclick={backToBrowseModList}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M19 12H5" />
            <polyline points="12 19 5 12 12 5" />
          </svg>
          Back to Browse
        </button>
        <button class="btn btn-ghost btn-sm" onclick={() => safeOpenUrl(`https://www.nexusmods.com/${getGameSlug()}/mods/${selectedBrowseMod?.mod_id}`)}>
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
            <polyline points="15 3 21 3 21 9" />
            <line x1="10" y1="14" x2="21" y2="3" />
          </svg>
          View on NexusMods
        </button>
      </div>

      {#if loadingModDetail}
        <div class="loading-container">
          <div class="loading-card">
            <div class="spinner"><div class="spinner-ring"></div></div>
            <div class="loading-text">
              <p class="loading-title">Loading mod details</p>
              <p class="loading-detail">{selectedBrowseMod.name}</p>
            </div>
          </div>
        </div>
      {:else if browseModDetail}
        <div class="detail-content">
          {#if browseModDetail.picture_url}
            <div class="mod-detail-hero" style="background-image: url({browseModDetail.picture_url})"></div>
          {/if}

          <div class="detail-title-section">
            <div class="detail-title-row">
              <h2 class="detail-name">{browseModDetail.name}</h2>
            </div>
            <p class="detail-author">by {browseModDetail.author}</p>
            {#if browseModDetail.summary}
              <p class="detail-summary">{browseModDetail.summary}</p>
            {/if}
          </div>

          <!-- Stats Bar -->
          <div class="detail-stats-bar">
            <div class="detail-stats-left">
              <div class="detail-stat">
                <span class="detail-stat-value">{formatDownloads(browseModDetail.endorsement_count)}</span>
                <span class="detail-stat-label">Endorsements</span>
              </div>
              <div class="detail-stat">
                <span class="detail-stat-value">{formatDownloads(browseModDetail.unique_downloads)}</span>
                <span class="detail-stat-label">Downloads</span>
              </div>
              <div class="detail-stat">
                <span class="detail-stat-value">v{browseModDetail.version}</span>
                <span class="detail-stat-label">Version</span>
              </div>
              {#if browseModDetail.updated_at}
                <div class="detail-stat">
                  <span class="detail-stat-value">{browseModDetail.updated_at}</span>
                  <span class="detail-stat-label">Updated</span>
                </div>
              {/if}
            </div>
            {#if account?.is_premium && !browseInstalledNexusIds.has(browseModDetail.mod_id)}
              <button
                class="btn btn-primary stats-install-btn"
                onclick={() => { if (browseModDetail) openFilePicker(browseModDetail); }}
              >
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                  <polyline points="7 10 12 15 17 10" />
                  <line x1="12" y1="15" x2="12" y2="3" />
                </svg>
                Install
              </button>
            {:else if browseInstalledNexusIds.has(browseModDetail.mod_id)}
              <span class="badge badge-success">Installed</span>
            {/if}
          </div>

          <!-- Description -->
          {#if renderedModDescription}
            <div class="detail-section">
              <h3 class="detail-section-title">Description</h3>
              <div class="rendered-markdown" onclick={handleRenderedLinkClick}>
                {@html renderedModDescription}
              </div>
            </div>
          {/if}

          <!-- Files Table (premium only) -->
          {#if account?.is_premium && browseModFiles.length > 0}
            <div class="detail-section">
              <h3 class="detail-section-title">
                Files
                <span class="title-count">{browseModFiles.length}</span>
              </h3>
              <div class="mods-table-container">
                <div class="mods-table">
                  <div class="mods-table-header">
                    <span class="col-name">Name</span>
                    <span class="col-version">Version</span>
                    <span class="col-size">Size</span>
                    <span class="col-category">Category</span>
                    <span class="col-actions">Actions</span>
                  </div>
                  {#each browseModFiles as file}
                    <div class="mods-table-row">
                      <span class="col-name" title={file.name}>{file.name}</span>
                      <span class="col-version">{file.version}</span>
                      <span class="col-size">{formatFileSize(file.size_kb)}</span>
                      <span class="col-category"><span class="tag">{file.category}</span></span>
                      <span class="col-actions">
                        <button
                          class="btn btn-accent btn-sm"
                          onclick={() => {
                            if (browseModDetail) {
                              filePickerMod = browseModDetail;
                              handleDownloadFile(file);
                            }
                          }}
                          disabled={downloadingFile === file.file_id}
                        >
                          {#if downloadingFile === file.file_id}
                            <div class="spinner-sm-ring"></div>
                          {:else}
                            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                              <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                              <polyline points="7 10 12 15 17 10" />
                              <line x1="12" y1="15" x2="12" y2="3" />
                            </svg>
                            Install
                          {/if}
                        </button>
                      </span>
                    </div>
                  {/each}
                </div>
              </div>
            </div>
          {/if}
        </div>
      {/if}
    </div>
  {:else}
  <div class="filters-bar">
    {#if allDetectedGames.length > 1}
      <select class="filter-select" bind:value={browseGameOverride} onchange={() => { browseInitializedForGame = ""; loadBrowseMods(); loadBrowseCategories(); }}>
        <option value={null}>{game.display_name ?? "Current Game"}</option>
        {#each allDetectedGames as g}
          {@const slug = gameSlugMap[g.game_id] ?? g.game_id}
          {#if slug !== getGameSlug() || browseGameOverride}
            <option value={slug}>{g.display_name}</option>
          {/if}
        {/each}
      </select>
    {/if}
    <div class="search-wrapper">
      <svg class="search-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="11" cy="11" r="8" />
        <line x1="21" y1="21" x2="16.65" y2="16.65" />
      </svg>
      <input type="text" class="search-input" placeholder="Search NexusMods..." bind:value={browseModsSearch} oninput={browseSearchDebounced} />
    </div>
    {#if browseCategoryOptions.length > 0}
      <div class="filter-group">
        <select class="filter-select" bind:value={browseCategoryId} onchange={() => loadBrowseMods()}>
          <option value={null}>All Categories</option>
          {#each browseCategoryOptions as cat}
            <option value={cat.id}>{cat.depth > 0 ? "\u00A0\u00A0" : ""}{cat.name}</option>
          {/each}
        </select>
      </div>
    {/if}
    <div class="filter-group">
      <select class="filter-select" bind:value={browseModsSort} onchange={() => loadBrowseMods()}>
        <option value="endorsements">Sort: Most Popular</option>
        <option value="name">Sort: Name</option>
        <option value="updated">Sort: Updated</option>
        <option value="createdAt">Sort: Recently Added</option>
      </select>
    </div>
    <button
      class="nsfw-cycle-btn"
      class:nsfw-show={browseNsfwFilter === "show"}
      class:nsfw-only={browseNsfwFilter === "only"}
      onclick={() => { browseNsfwFilter = cycleNsfwFilter(browseNsfwFilter); loadBrowseMods(); }}
      title={browseNsfwFilter === "hide" ? "NSFW hidden" : browseNsfwFilter === "show" ? "NSFW included" : "NSFW only"}
    >
      <span class="nsfw-indicator">{nsfwIcon(browseNsfwFilter)}</span>
      {nsfwLabel(browseNsfwFilter)}
    </button>
    <button class="filter-toggle" onclick={() => showBrowseAdvancedFilters = !showBrowseAdvancedFilters}>
      Filters {showBrowseAdvancedFilters ? '\u25B2' : '\u25BC'}
      {#if browseActiveFilterCount > 0}<span class="filter-badge">{browseActiveFilterCount}</span>{/if}
    </button>
  </div>

  {#if showBrowseAdvancedFilters}
    <div class="advanced-filters">
      <div class="filter-section">
        <label class="filter-label">Author</label>
        <input type="text" class="filter-input" placeholder="Filter by author..." bind:value={browseAuthorFilter} oninput={browseAuthorDebounced} />
      </div>

      <div class="filter-section">
        <label class="filter-label">Updated</label>
        <div class="filter-pills">
          <button class="filter-pill" class:active={browseUpdatePeriod === "all"} onclick={() => { browseUpdatePeriod = "all"; loadBrowseMods(); }}>All Time</button>
          <button class="filter-pill" class:active={browseUpdatePeriod === "24h"} onclick={() => { browseUpdatePeriod = "24h"; loadBrowseMods(); }}>Last 24h</button>
          <button class="filter-pill" class:active={browseUpdatePeriod === "1w"} onclick={() => { browseUpdatePeriod = "1w"; loadBrowseMods(); }}>Last Week</button>
          <button class="filter-pill" class:active={browseUpdatePeriod === "1m"} onclick={() => { browseUpdatePeriod = "1m"; loadBrowseMods(); }}>Last Month</button>
        </div>
      </div>

      <div class="filter-section">
        <label class="filter-label">Min Downloads</label>
        <div class="filter-pills">
          <button class="filter-pill" class:active={browseMinDownloads === null} onclick={() => { browseMinDownloads = null; loadBrowseMods(); }}>Any</button>
          <button class="filter-pill" class:active={browseMinDownloads === 1000} onclick={() => { browseMinDownloads = 1000; loadBrowseMods(); }}>1K+</button>
          <button class="filter-pill" class:active={browseMinDownloads === 10000} onclick={() => { browseMinDownloads = 10000; loadBrowseMods(); }}>10K+</button>
          <button class="filter-pill" class:active={browseMinDownloads === 100000} onclick={() => { browseMinDownloads = 100000; loadBrowseMods(); }}>100K+</button>
        </div>
      </div>

      <div class="filter-section">
        <label class="filter-label">Min Endorsements</label>
        <div class="filter-pills">
          <button class="filter-pill" class:active={browseMinEndorsements === null} onclick={() => { browseMinEndorsements = null; loadBrowseMods(); }}>Any</button>
          <button class="filter-pill" class:active={browseMinEndorsements === 100} onclick={() => { browseMinEndorsements = 100; loadBrowseMods(); }}>100+</button>
          <button class="filter-pill" class:active={browseMinEndorsements === 1000} onclick={() => { browseMinEndorsements = 1000; loadBrowseMods(); }}>1K+</button>
          <button class="filter-pill" class:active={browseMinEndorsements === 10000} onclick={() => { browseMinEndorsements = 10000; loadBrowseMods(); }}>10K+</button>
        </div>
      </div>
    </div>
  {/if}

  {#if browseActiveFilterCount > 0}
    <div class="active-filters">
      {#if browseCategoryId !== null}
        <span class="filter-chip">
          Category: {browseCategoryOptions.find(c => c.id === browseCategoryId)?.name ?? browseCategoryId}
          <button onclick={() => { browseCategoryId = null; loadBrowseMods(); }}>&times;</button>
        </span>
      {/if}
      {#if browseAuthorFilter.trim()}
        <span class="filter-chip">
          Author: {browseAuthorFilter}
          <button onclick={() => { browseAuthorFilter = ""; loadBrowseMods(); }}>&times;</button>
        </span>
      {/if}
      {#if browseUpdatePeriod !== "all"}
        <span class="filter-chip">
          Updated: {browseUpdatePeriod === "24h" ? "Last 24h" : browseUpdatePeriod === "1w" ? "Last Week" : "Last Month"}
          <button onclick={() => { browseUpdatePeriod = "all"; loadBrowseMods(); }}>&times;</button>
        </span>
      {/if}
      {#if browseMinDownloads !== null}
        <span class="filter-chip">
          Downloads: {formatDownloads(browseMinDownloads)}+
          <button onclick={() => { browseMinDownloads = null; loadBrowseMods(); }}>&times;</button>
        </span>
      {/if}
      {#if browseMinEndorsements !== null}
        <span class="filter-chip">
          Endorsements: {formatDownloads(browseMinEndorsements)}+
          <button onclick={() => { browseMinEndorsements = null; loadBrowseMods(); }}>&times;</button>
        </span>
      {/if}
      <button class="filter-chip filter-chip-clear" onclick={clearAllBrowseFilters}>
        Clear All &times;
      </button>
    </div>
  {/if}

  {#if browseModsLoading}
    <div class="loading-container">
      <div class="loading-card">
        <div class="spinner"><div class="spinner-ring"></div></div>
        <div class="loading-text">
          <p class="loading-title">Searching NexusMods</p>
          <p class="loading-detail">{browseModsSearch ? `Searching for "${browseModsSearch}"...` : "Loading popular mods..."}</p>
        </div>
      </div>
    </div>
  {:else if browseMods.length === 0}
    <div class="empty-state">
      <p class="empty-title">No mods found</p>
      <p class="empty-detail">{browseModsSearch ? "Try a different search term." : "No mods available for this selection."}</p>
    </div>
  {:else}
    <div class="mod-browse-grid">
      {#each browseMods as mod (mod.mod_id)}
        <div class="mod-browse-card" onclick={() => openModDetail(mod)} role="button" tabindex="0" onkeydown={(e) => { if (e.key === "Enter") openModDetail(mod); }}>
          {#if browseInstalledNexusIds.has(mod.mod_id)}
            <div class="browse-installed-badge">Installed</div>
          {/if}
          {#if mod.picture_url}
            <div class="mod-browse-img" style="background-image: url({mod.picture_url})"></div>
          {:else}
            <div class="mod-browse-img mod-browse-img-placeholder">
              <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" opacity="0.3">
                <rect x="3" y="3" width="18" height="18" rx="2" />
                <circle cx="8.5" cy="8.5" r="1.5" />
                <polyline points="21 15 16 10 5 21" />
              </svg>
            </div>
          {/if}
          <div class="mod-browse-body">
            <h4 class="mod-browse-name">{mod.name}</h4>
            <p class="mod-browse-author">by {mod.author}</p>
            <p class="mod-browse-summary">{mod.summary}</p>
            <div class="mod-browse-stats">
              <span class="mod-browse-stat" title="Endorsements">
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3H14z" />
                  <path d="M7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3" />
                </svg>
                {formatDownloads(mod.endorsement_count)}
              </span>
              <span class="mod-browse-stat" title="Downloads">
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                  <polyline points="7 10 12 15 17 10" />
                  <line x1="12" y1="15" x2="12" y2="3" />
                </svg>
                {formatDownloads(mod.unique_downloads)}
              </span>
              {#if mod.version}
                <span class="mod-browse-stat mod-browse-version">v{mod.version}</span>
              {/if}
            </div>
            {#if account?.is_premium && !browseInstalledNexusIds.has(mod.mod_id)}
              <button
                class="btn btn-accent btn-sm mod-download-btn"
                onclick={(e) => { e.stopPropagation(); openFilePicker(mod); }}
                title="Download & Install"
              >
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                  <polyline points="7 10 12 15 17 10" />
                  <line x1="12" y1="15" x2="12" y2="3" />
                </svg>
                Install
              </button>
            {/if}
          </div>
        </div>
      {/each}
    </div>

    <!-- Pagination -->
    {#if browseTotalPages > 1}
      <div class="browse-pagination">
        <button
          class="btn btn-ghost btn-sm"
          disabled={browseCurrentPage <= 1}
          onclick={() => browseGoToPage(browseCurrentPage - 1)}
        >Previous</button>
        {#each Array.from({ length: Math.min(browseTotalPages, 7) }, (_, i) => {
          const total = browseTotalPages;
          const current = browseCurrentPage;
          if (total <= 7) return i + 1;
          if (i === 0) return 1;
          if (i === 6) return total;
          if (current <= 4) return i + 1;
          if (current >= total - 3) return total - 6 + i;
          return current - 3 + i;
        }) as page}
          <button
            class="btn btn-sm"
            class:btn-primary={page === browseCurrentPage}
            class:btn-ghost={page !== browseCurrentPage}
            onclick={() => browseGoToPage(page)}
          >{page}</button>
        {/each}
        <button
          class="btn btn-ghost btn-sm"
          disabled={!browseModsHasMore}
          onclick={() => browseGoToPage(browseCurrentPage + 1)}
        >Next</button>
      </div>
    {/if}

    <p class="browse-mods-hint">Click a mod to view details. Use the Install button to download and install directly.</p>
  {/if}
  {/if}
{/if}

<!-- File Picker Modal -->
{#if showFilePicker && filePickerMod}
  <div class="modal-overlay" onclick={closeFilePicker} role="presentation">
    <div class="file-picker-modal" onclick={(e) => e.stopPropagation()} role="dialog" aria-label="Select file to download">
      <div class="file-picker-header">
        <h3 class="file-picker-title">Download: {filePickerMod.name}</h3>
        <button class="file-picker-close" onclick={closeFilePicker}>&times;</button>
      </div>

      {#if loadingFiles}
        <div class="file-picker-loading">
          <div class="spinner-sm"></div>
          <span>Loading available files...</span>
        </div>
      {:else if filePickerFiles.length === 0}
        <div class="file-picker-empty">
          <p>No downloadable files found for this mod.</p>
        </div>
      {:else}
        <div class="file-picker-list">
          {#each filePickerFiles as file}
            <div class="file-picker-item" class:file-downloading={downloadingFile === file.file_id}>
              <div class="file-picker-info">
                <div class="file-picker-name">{file.name}</div>
                <div class="file-picker-meta">
                  <span class="file-category-badge" class:file-cat-main={file.category === "main"} class:file-cat-optional={file.category === "optional"} class:file-cat-update={file.category === "update"}>
                    {file.category}
                  </span>
                  {#if file.version}<span class="file-version">v{file.version}</span>{/if}
                  <span class="file-size">{formatFileSize(file.size_kb)}</span>
                </div>
                {#if file.description}
                  <p class="file-picker-desc">{file.description}</p>
                {/if}
              </div>
              <div class="file-picker-action">
                {#if downloadingFile === file.file_id}
                  <div class="download-progress-bar">
                    <div class="download-progress-fill" style="width: {downloadProgress && downloadProgress.total > 0 ? Math.round((downloadProgress.downloaded / downloadProgress.total) * 100) : 0}%"></div>
                  </div>
                  <span class="download-progress-text">
                    {#if downloadProgress && downloadProgress.total > 0}
                      {Math.round((downloadProgress.downloaded / downloadProgress.total) * 100)}%
                    {:else}
                      Starting...
                    {/if}
                  </span>
                {:else}
                  <button
                    class="btn btn-accent btn-sm"
                    disabled={downloadingFile !== null}
                    onclick={() => handleDownloadFile(file)}
                  >
                    Install
                  </button>
                {/if}
              </div>
            </div>
          {/each}
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .mod-browse-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: var(--space-3);
    padding: 0 0 var(--space-4);
  }

  .mod-browse-card {
    position: relative;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-lg);
    overflow: hidden;
    cursor: pointer;
    transition: border-color var(--duration-fast) var(--ease), box-shadow var(--duration-fast) var(--ease);
    text-align: left;
  }

  .mod-browse-card:hover {
    border-color: var(--accent);
    box-shadow: 0 2px 12px rgba(0, 0, 0, 0.2);
  }

  .mod-browse-img {
    width: 100%;
    height: 120px;
    background-size: cover;
    background-position: center;
    background-color: var(--bg-base);
  }

  .mod-browse-img-placeholder {
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .mod-browse-body {
    padding: var(--space-3);
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex: 1;
  }

  .mod-browse-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    margin: 0;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .mod-browse-author {
    font-size: 11px;
    color: var(--text-tertiary);
    margin: 0;
  }

  .mod-browse-summary {
    font-size: 11px;
    color: var(--text-secondary);
    line-height: 1.4;
    margin: 2px 0 0;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .mod-browse-stats {
    display: flex;
    gap: var(--space-3);
    margin-top: auto;
    padding-top: var(--space-2);
  }

  .mod-browse-stat {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .mod-browse-version {
    margin-left: auto;
    color: var(--text-quaternary);
    font-family: var(--font-mono);
    font-size: 10px;
  }

  .browse-mods-hint {
    font-size: 11px;
    color: var(--text-quaternary);
    text-align: center;
    padding: var(--space-2) 0 var(--space-4);
  }

  .browse-installed-badge {
    position: absolute;
    top: var(--space-2);
    right: var(--space-2);
    padding: 2px 8px;
    border-radius: 100px;
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    background: color-mix(in srgb, var(--green) 20%, transparent);
    color: var(--green);
    backdrop-filter: var(--glass-blur-light);
    z-index: 1;
  }

  .browse-pagination {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-1);
    padding: var(--space-4) 0 var(--space-2);
  }

  .mod-download-btn {
    margin-top: var(--space-2);
    display: flex;
    align-items: center;
    gap: var(--space-1);
    width: 100%;
    justify-content: center;
  }

  .mod-detail-hero {
    width: 100%;
    height: 280px;
    background-size: cover;
    background-position: center;
    border-radius: var(--radius-lg);
    border: 1px solid var(--border-primary);
  }

  .modal-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
    backdrop-filter: var(--glass-blur-light);
  }

  .file-picker-modal {
    background: color-mix(in srgb, var(--bg-grouped) 75%, transparent);
    backdrop-filter: blur(40px) saturate(1.5);
    -webkit-backdrop-filter: blur(40px) saturate(1.5);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: var(--radius-lg, 12px);
    width: min(560px, 90vw);
    max-height: 70vh;
    display: flex;
    flex-direction: column;
    box-shadow: var(--glass-refraction),
                var(--glass-edge-shadow),
                0 8px 32px rgba(0, 0, 0, 0.4);
  }

  .file-picker-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-4) var(--space-6);
    border-bottom: 1px solid var(--separator);
    flex-shrink: 0;
  }

  .file-picker-title {
    font-size: 16px;
    font-weight: 600;
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 420px;
  }

  .file-picker-close {
    width: 28px;
    height: 28px;
    border-radius: var(--radius-sm);
    background: transparent;
    border: none;
    color: var(--text-tertiary);
    font-size: 18px;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .file-picker-close:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .file-picker-loading {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    padding: var(--space-10);
    color: var(--text-secondary);
    font-size: 13px;
  }

  .file-picker-empty {
    padding: var(--space-10);
    text-align: center;
    color: var(--text-tertiary);
    font-size: 13px;
  }

  .file-picker-list {
    overflow-y: auto;
    padding: var(--space-2);
  }

  .file-picker-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius);
    transition: background var(--duration-fast) var(--ease);
  }

  .file-picker-item:hover {
    background: var(--surface-hover);
  }

  .file-picker-item.file-downloading {
    background: rgba(0, 122, 255, 0.05);
  }

  .file-picker-info {
    flex: 1;
    min-width: 0;
  }

  .file-picker-name {
    font-size: 14px;
    font-weight: 500;
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .file-picker-meta {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-1);
    font-size: 12px;
    color: var(--text-tertiary);
  }

  .file-category-badge {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    padding: 1px 5px;
    border-radius: 100px;
    background: var(--surface-hover);
    color: var(--text-secondary);
  }

  .file-cat-main {
    background: rgba(48, 209, 88, 0.15);
    color: #30d158;
  }

  .file-cat-optional {
    background: rgba(0, 122, 255, 0.15);
    color: var(--system-accent);
  }

  .file-cat-update {
    background: rgba(255, 159, 10, 0.15);
    color: #ff9f0a;
  }

  .file-version {
    color: var(--text-tertiary);
  }

  .file-size {
    color: var(--text-tertiary);
  }

  .file-picker-desc {
    font-size: 12px;
    color: var(--text-tertiary);
    margin-top: var(--space-1);
    line-height: 1.4;
    overflow: hidden;
    text-overflow: ellipsis;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
  }

  .file-picker-action {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-1);
    min-width: 80px;
  }

  .download-progress-bar {
    width: 80px;
    height: 4px;
    background: var(--separator);
    border-radius: 2px;
    overflow: hidden;
  }

  .download-progress-fill {
    height: 100%;
    background: var(--system-accent);
    border-radius: 2px;
    transition: width 0.3s ease;
  }

  .download-progress-text {
    font-size: 11px;
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
  }
</style>
