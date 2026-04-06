<script lang="ts">
  import { showError, showSuccess } from "$lib/stores";
  import type { DetectedGame, NexusModInfo, NexusCategory, NexusModFile } from "$lib/types";
  import {
    browseNexusMods,
    searchNexusMods,
    getGameCategories,
    getInstalledMods,
    downloadAndInstallNexusMod,
  } from "$lib/api";
  import { listen } from "@tauri-apps/api/event";
  import { goto } from "$app/navigation";
  import NexusLogo from "$lib/components/NexusLogo.svelte";
  import WebViewToggle from "$lib/components/WebViewToggle.svelte";
  import SearchFilterBar from "$lib/components/SearchFilterBar.svelte";
  import BrowseModDetail from "$lib/components/collections/BrowseModDetail.svelte";
  import BrowseFilePicker from "$lib/components/collections/BrowseFilePicker.svelte";

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

  // Download & file picker state
  let showFilePicker = $state(false);
  let filePickerMod = $state<NexusModInfo | null>(null);
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

  function openModDetail(mod: NexusModInfo) {
    selectedBrowseMod = mod;
  }

  /** Public method: open a mod detail view by mod stub (used for LLM events). */
  export function openModById(mod: NexusModInfo) {
    openModDetail(mod);
  }

  function backToBrowseModList() {
    selectedBrowseMod = null;
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
  function openFilePicker(mod: NexusModInfo) {
    filePickerMod = mod;
    showFilePicker = true;
  }

  function closeFilePicker() {
    showFilePicker = false;
    filePickerMod = null;
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

  /** Handle download from the detail view's file table (no separate picker needed). */
  async function handleDetailDownloadFile(mod: NexusModInfo, file: NexusModFile) {
    filePickerMod = mod;
    await handleDownloadFile(file);
  }

  /** Cleanup timers on destroy. */
  import { onDestroy } from "svelte";
  onDestroy(() => {
    if (browseSearchTimer) { clearTimeout(browseSearchTimer); browseSearchTimer = null; }
    if (browseAuthorTimer) { clearTimeout(browseAuthorTimer); browseAuthorTimer = null; }
  });
</script>

<header class="browse-header">
  <h2 class="browse-title"><NexusLogo size={18} /> Browse Nexus</h2>
  <div class="browse-toolbar">
    <WebViewToggle
      bind:this={browseWebviewToggle}
      url={`https://www.nexusmods.com/${getGameSlug()}/mods/`}
      defaultMode={account?.connected ? "app" : "website"}
      onModeChange={(m) => browseViewMode = m}
      anchorEl={browseWebviewAnchor}
    />
    {#if !browseModsLoading && browseModsTotalCount > 0 && browseViewMode === "app"}
      <div class="browse-toolbar-sep"></div>
      <span class="browse-stat-badge">{browseModsTotalCount.toLocaleString()} {browseModsTotalCount === 1 ? "mod" : "mods"}</span>
    {/if}
  </div>
</header>

{#if browseViewMode === "website"}
  <div class="browse-webview-anchor" bind:this={browseWebviewAnchor}>
    <p class="browse-webview-hint">Browsing NexusMods directly. Switch to "In-App" to use built-in search and filters.</p>
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
    <BrowseModDetail
      mod={selectedBrowseMod}
      gameSlug={getGameSlug()}
      {account}
      installedNexusIds={browseInstalledNexusIds}
      onback={backToBrowseModList}
      oninstall={openFilePicker}
      ondownloadfile={handleDetailDownloadFile}
      downloadingFileId={downloadingFile}
    />
  {:else}
  <SearchFilterBar
    searchPlaceholder="Search NexusMods..."
    bind:searchValue={browseModsSearch}
    onsearch={() => browseSearchDebounced()}
  >
    {#snippet gameSelector()}
      {#if allDetectedGames.length > 1}
        <select bind:value={browseGameOverride} onchange={() => { browseInitializedForGame = ""; loadBrowseMods(); loadBrowseCategories(); }}>
          <option value={null}>{game.display_name ?? "Current Game"}</option>
          {#each allDetectedGames as g}
            {@const slug = gameSlugMap[g.game_id] ?? g.game_id}
            {#if slug !== getGameSlug() || browseGameOverride}
              <option value={slug}>{g.display_name}</option>
            {/if}
          {/each}
        </select>
      {/if}
    {/snippet}
    {#snippet controls()}
      {#if browseCategoryOptions.length > 0}
        <select bind:value={browseCategoryId} onchange={() => loadBrowseMods()}>
          <option value={null}>All Categories</option>
          {#each browseCategoryOptions as cat}
            <option value={cat.id}>{cat.depth > 0 ? "\u00A0\u00A0" : ""}{cat.name}</option>
          {/each}
        </select>
        <div class="strip-sep"></div>
      {/if}
      <select bind:value={browseModsSort} onchange={() => loadBrowseMods()}>
        <option value="endorsements">Most Popular</option>
        <option value="name">Name</option>
        <option value="updated">Updated</option>
        <option value="createdAt">Recently Added</option>
      </select>
      <div class="strip-sep"></div>
      <button
        class:nsfw-show={browseNsfwFilter === "show"}
        class:nsfw-only={browseNsfwFilter === "only"}
        onclick={() => { browseNsfwFilter = cycleNsfwFilter(browseNsfwFilter); loadBrowseMods(); }}
        title={browseNsfwFilter === "hide" ? "NSFW hidden" : browseNsfwFilter === "show" ? "NSFW included" : "NSFW only"}
      >
        <span class="nsfw-indicator">{nsfwIcon(browseNsfwFilter)}</span>
        {nsfwLabel(browseNsfwFilter)}
      </button>
      <div class="strip-sep"></div>
      <button onclick={() => showBrowseAdvancedFilters = !showBrowseAdvancedFilters}>
        Filters {showBrowseAdvancedFilters ? '\u25B2' : '\u25BC'}
        {#if browseActiveFilterCount > 0}<span class="filter-badge">{browseActiveFilterCount}</span>{/if}
      </button>
    {/snippet}
  </SearchFilterBar>

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
  <BrowseFilePicker
    mod={filePickerMod}
    gameSlug={getGameSlug()}
    downloadingFileId={downloadingFile}
    {downloadProgress}
    ondownload={handleDownloadFile}
    onclose={closeFilePicker}
  />
{/if}

<style>
  /* --- Browse Header (matches collections page-header) --- */
  .browse-header {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-top: var(--space-4);
    margin-bottom: var(--space-4);
    flex-wrap: wrap;
  }

  .browse-title {
    font-size: 20px;
    font-weight: 700;
    color: var(--text-primary);
    letter-spacing: -0.02em;
    display: flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
    margin: 0;
  }

  .browse-toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-left: auto;
    background: color-mix(in srgb, var(--surface-subtle) 60%, transparent);
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
    border: 1px solid color-mix(in srgb, var(--separator) 30%, transparent);
    border-radius: 8px;
    padding: 4px 10px;
  }

  .browse-toolbar-sep {
    width: 1px;
    height: 16px;
    background: color-mix(in srgb, var(--separator) 50%, transparent);
    flex-shrink: 0;
  }

  .browse-stat-badge {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .browse-webview-anchor {
    display: flex;
    align-items: center;
    justify-content: center;
    flex: 1;
    min-height: calc(100vh - 200px);
    padding: var(--space-8);
  }

  .browse-webview-hint {
    font-size: 13px;
    color: var(--text-tertiary);
    text-align: center;
  }

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

  /* Strip separator used inside SearchFilterBar controls snippet */
  .strip-sep {
    width: 1px;
    height: 16px;
    background: var(--separator);
    flex-shrink: 0;
  }

  .advanced-filters {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-4);
    padding: var(--space-3) var(--space-4);
    margin-bottom: var(--space-4);
    background: var(--surface-subtle);
    border-radius: var(--radius);
    border: 1px solid var(--separator);
  }

  .filter-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 120px;
  }

  .filter-label {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-tertiary);
  }

  .filter-input {
    padding: var(--space-1) var(--space-2);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-sm);
    color: var(--text-primary);
    font-size: 12px;
    outline: none;
  }
  .filter-input:focus { border-color: var(--system-accent); }

  .filter-pills {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
  }

  .filter-pill {
    padding: 2px 8px;
    font-size: 11px;
    border-radius: 100px;
    background: var(--surface);
    border: 1px solid var(--separator);
    color: var(--text-secondary);
    cursor: pointer;
    font-family: inherit;
    transition: all var(--duration-fast) var(--ease);
  }
  .filter-pill.active {
    background: var(--accent);
    color: white;
    border-color: var(--accent);
  }
  .filter-pill:not(.active):hover {
    border-color: var(--text-tertiary);
  }

</style>
