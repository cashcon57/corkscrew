<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { goto } from "$app/navigation";
  import { selectedGame, showError, showSuccess, collectionInstallStatus } from "$lib/stores";
  import type { CollectionInfo, CollectionManifest, CollectionMod, CollectionSearchResult, InstalledMod, NexusModInfo, NexusCategory, NexusSearchResult } from "$lib/types";
  import {
    browseCollections,
    browseNexusMods,
    searchNexusMods,
    getGameCategories,
    getCollection,
    getCollectionMods,
    getNexusAccountStatus,
    setConfigValue,
    getConfig,
    installCollection,
    onInstallProgress,
    listInstalledCollections,
    switchCollection,
    deleteCollection,
    getCollectionDiff,
    getInstalledMods,
    detectCollectionTools,
  } from "$lib/api";
  import type { CollectionSummary, CollectionDiff, RequiredTool } from "$lib/types";
  import type { InstallProgressEvent } from "$lib/types";
  import { config } from "$lib/stores";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { marked } from "marked";
  import DOMPurify from "dompurify";
  import CompatibilityPanel from "$lib/components/CompatibilityPanel.svelte";
  import RequiredToolsPrompt from "$lib/components/RequiredToolsPrompt.svelte";
  import NexusLogo from "$lib/components/NexusLogo.svelte";
  import WabbajackLogo from "$lib/components/WabbajackLogo.svelte";

  const NEXUS_API_KEY_URL = "https://www.nexusmods.com/users/myaccount?tab=api+access";

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

  // ---- Tab State ----
  let activeTab = $state<"my" | "nexus" | "wabbajack" | "browse_mods">("my");
  let myCollections = $state<CollectionSummary[]>([]);
  let loadingMyCollections = $state(false);
  let switchingCollection = $state<string | null>(null);
  let deletingCollection = $state<string | null>(null);
  let confirmDeleteCollection = $state<string | null>(null);
  let deleteKeepDownloads = $state(true);
  let collectionDiffs = $state<Record<string, CollectionDiff | "loading" | "error">>({});

  // Local detail view
  let selectedMyCollection = $state<CollectionSummary | null>(null);
  let localCollectionMods = $state<InstalledMod[]>([]);
  let loadingLocalDetail = $state(false);
  let localDiff = $state<CollectionDiff | "loading" | "error" | null>(null);

  async function viewLocalCollection(col: CollectionSummary) {
    const game = $selectedGame;
    if (!game) return;
    selectedMyCollection = col;
    loadingLocalDetail = true;
    localDiff = null;
    try {
      const allMods = await getInstalledMods(game.game_id, game.bottle_name);
      localCollectionMods = allMods.filter(m => m.collection_name === col.name);
    } catch {
      localCollectionMods = [];
    } finally {
      loadingLocalDetail = false;
    }
    // Auto-check diff if slug is available
    if (col.slug) {
      localDiff = "loading";
      try {
        localDiff = await getCollectionDiff(game.game_id, game.bottle_name, col.name);
      } catch {
        localDiff = "error";
      }
    }
  }

  function backToMyCollections() {
    selectedMyCollection = null;
    localCollectionMods = [];
    localDiff = null;
  }

  async function handleCheckDiff(colName: string) {
    const game = $selectedGame;
    if (!game) return;
    collectionDiffs = { ...collectionDiffs, [colName]: "loading" };
    try {
      const diff = await getCollectionDiff(game.game_id, game.bottle_name, colName);
      collectionDiffs = { ...collectionDiffs, [colName]: diff };
    } catch {
      collectionDiffs = { ...collectionDiffs, [colName]: "error" };
    }
  }

  async function loadMyCollections() {
    const game = $selectedGame;
    if (!game) return;
    loadingMyCollections = true;
    try {
      myCollections = await listInstalledCollections(game.game_id, game.bottle_name);
    } catch {
      myCollections = [];
    } finally {
      loadingMyCollections = false;
    }
  }

  async function handleSwitchCollection(name: string) {
    const game = $selectedGame;
    if (!game) return;
    switchingCollection = name;
    try {
      await switchCollection(game.game_id, game.bottle_name, name);
      showSuccess(`Switched to "${name}" — mods deployed`);
      await loadMyCollections();
    } catch (e: unknown) {
      showError(`Failed to switch: ${e}`);
    } finally {
      switchingCollection = null;
    }
  }

  async function handleDeleteCollection(name: string) {
    const game = $selectedGame;
    if (!game) return;
    deletingCollection = name;
    try {
      const result = await deleteCollection(game.game_id, game.bottle_name, name, !deleteKeepDownloads);
      showSuccess(`Removed "${name}" (${result.mods_removed} mods${result.downloads_removed > 0 ? `, ${result.downloads_removed} downloads` : ""})`);
      confirmDeleteCollection = null;
      await loadMyCollections();
    } catch (e: unknown) {
      showError(`Failed to delete: ${e}`);
    } finally {
      deletingCollection = null;
    }
  }

  $effect(() => {
    if (activeTab === "my" && $selectedGame) {
      loadMyCollections();
    }
  });

  $effect(() => {
    if (activeTab === "browse_mods" && $selectedGame) {
      loadBrowseMods();
      loadBrowseCategories();
      loadBrowseInstalledIds();
    }
  });

  // ---- Account State ----

  interface AccountStatus {
    connected: boolean;
    auth_type?: string;
    name?: string;
    is_premium?: boolean;
    avatar?: string | null;
  }

  let account = $state<AccountStatus | null>(null);
  let checkingAuth = $state(true);
  let signingIn = $state(false);
  let apiKeyInput = $state("");
  let validationError = $state<string | null>(null);

  // ---- Collections State ----

  let collections = $state<CollectionInfo[]>([]);
  let filtered = $state<CollectionInfo[]>([]);
  let loading = $state(false);
  let searchQuery = $state("");
  let gameFilter = $state("all");
  let showNsfw = $state(false);
  let sortField = $state<"endorsements" | "downloads" | "name" | "rating" | "created" | "updated" | "mods">("endorsements");
  let sortDirection = $state<"asc" | "desc">("desc");
  let collectionsTotalCount = $state(0);
  let collectionsOffset = $state(0);
  const COLLECTIONS_PAGE_SIZE = 20;
  let collectionsSearchTimer: ReturnType<typeof setTimeout> | null = null;
  const collectionsTotalPages = $derived(Math.max(1, Math.ceil(collectionsTotalCount / COLLECTIONS_PAGE_SIZE)));
  const collectionsCurrentPage = $derived(Math.floor(collectionsOffset / COLLECTIONS_PAGE_SIZE) + 1);
  let selectedCollection = $state<CollectionInfo | null>(null);
  let selectedMods = $state<CollectionMod[]>([]);
  let loadingDetail = $state(false);
  let installing = $state(false);
  let installStep = $state("");
  let installModName = $state("");
  let installProgress = $state({ current: 0, total: 0 });
  let installResult = $state<{ installed: number; already_installed: number; skipped: number; failed: number; details: { name: string; status: string; error: string | null; url: string | null; instructions: string | null }[] } | null>(null);
  let installStartTime = $state<number>(0);
  let installElapsed = $state("");
  let elapsedInterval: ReturnType<typeof setInterval> | null = null;
  let renderedDescription = $state("");
  let userActions = $state<Array<{mod_name: string, action: string, url: string | null, instructions: string | null}>>([]);
  let installUnlisten: (() => void) | null = null;

  // Tool requirement detection
  let pendingTools = $state<RequiredTool[]>([]);
  let showToolsPrompt = $state(false);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let pendingManifest = $state<(CollectionManifest & Record<string, unknown>) | null>(null);

  // ---- Mod Browse State ----
  let browseMods = $state<NexusModInfo[]>([]);
  let browseModsLoading = $state(false);
  let browseModsSearch = $state("");
  let browseModsShowNsfw = $state(false);
  let browseModsSort = $state<"endorsements" | "downloads" | "name" | "updated">("endorsements");
  let browseModsTotalCount = $state(0);
  let browseModsOffset = $state(0);
  let browseModsHasMore = $state(false);
  const BROWSE_PAGE_SIZE = 20;
  let browseCategories = $state<NexusCategory[]>([]);
  let browseCategoryId = $state<number | null>(null);
  let browseInstalledNexusIds = $state<Set<number>>(new Set());
  let browseSearchTimer: ReturnType<typeof setTimeout> | null = null;
  let browseUseGraphQL = $state(true);

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

  function getGameSlug(): string {
    const game = $selectedGame;
    if (!game) return "";
    return gameSlugMap[game.game_id] ?? game.game_id;
  }

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
        };
        const result = await searchNexusMods(
          slug,
          browseModsSearch.trim() || null,
          sortMap[browseModsSort] ?? "endorsements",
          browseModsSort === "name" ? "ASC" : "DESC",
          BROWSE_PAGE_SIZE,
          browseModsOffset,
          browseModsShowNsfw,
        );
        browseMods = result.mods;
        browseModsTotalCount = result.total_count;
        browseModsHasMore = result.has_more;
      } else {
        // Fallback to v1 REST browse
        browseMods = await browseNexusMods(slug, "all");
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
    const game = $selectedGame;
    if (!game) return;
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

  function openModPage(mod: NexusModInfo) {
    const game = $selectedGame;
    if (!game) return;
    const slugMap: Record<string, string> = {
      skyrimse: "skyrimspecialedition",
      skyrim: "skyrim",
      fallout4: "fallout4",
      fallout3: "fallout3",
      falloutnv: "newvegas",
      oblivion: "oblivion",
      morrowind: "morrowind",
      starfield: "starfield",
    };
    const slug = slugMap[game.game_id] ?? game.game_id;
    safeOpenUrl(`https://www.nexusmods.com/${slug}/mods/${mod.mod_id}`);
  }

  const gameOptions = $derived.by(() => {
    const gamesSet = new Set(collections.map(c => c.game_domain));
    return Array.from(gamesSet).sort();
  });

  $effect(() => {
    let result = collections;

    if (!showNsfw) {
      result = result.filter(c => !c.adult_content);
    }

    filtered = result;
  });

  onMount(async () => {
    await checkAccount();
  });

  onDestroy(() => {
    if (installUnlisten) { installUnlisten(); installUnlisten = null; }
    if (elapsedInterval) { clearInterval(elapsedInterval); elapsedInterval = null; }
  });

  async function checkAccount() {
    checkingAuth = true;
    try {
      account = await getNexusAccountStatus();
      if (account.connected) {
        await loadCollections();
      }
    } catch {
      account = { connected: false };
    } finally {
      checkingAuth = false;
    }
  }

  async function openNexusApiPage() {
    try {
      await openUrl(NEXUS_API_KEY_URL);
    } catch { /* fallback: link is visible in UI */ }
  }

  async function handleConnect() {
    if (!apiKeyInput.trim()) return;
    signingIn = true;
    validationError = null;
    try {
      await setConfigValue("nexus_api_key", apiKeyInput.trim());
      const cfg = await getConfig();
      config.set(cfg);
      const status = await getNexusAccountStatus();
      if (status.connected) {
        account = status;
        apiKeyInput = "";
        showSuccess(`Connected as ${status.name}`);
        await loadCollections();
      } else {
        await setConfigValue("nexus_api_key", "");
        const cfg2 = await getConfig();
        config.set(cfg2);
        validationError = "Invalid API key. Please check and try again.";
      }
    } catch (e: unknown) {
      try {
        await setConfigValue("nexus_api_key", "");
        const cfg2 = await getConfig();
        config.set(cfg2);
      } catch { /* ignore */ }
      const msg = typeof e === "string" ? e : (e instanceof Error ? e.message : String(e));
      validationError = `Connection failed: ${msg}`;
    } finally {
      signingIn = false;
    }
  }

  async function loadCollections(gameDomain: string = "skyrimspecialedition", resetOffset = true) {
    loading = true;
    if (resetOffset) collectionsOffset = 0;
    try {
      const searchText = searchQuery.trim() || undefined;
      const result: CollectionSearchResult = await browseCollections(
        gameDomain, COLLECTIONS_PAGE_SIZE, collectionsOffset,
        sortField, sortDirection, searchText,
      );
      collections = result.collections;
      collectionsTotalCount = result.total_count;
    } catch (e: unknown) {
      showError(`Failed to load collections: ${e}`);
    } finally {
      loading = false;
    }
  }

  function reloadWithSort() {
    const gd = gameFilter !== "all" ? gameFilter : "skyrimspecialedition";
    loadCollections(gd);
  }

  function collectionsGoToPage(page: number) {
    collectionsOffset = (page - 1) * COLLECTIONS_PAGE_SIZE;
    const gd = gameFilter !== "all" ? gameFilter : "skyrimspecialedition";
    loadCollections(gd, false);
  }

  async function viewCollectionDetail(collection: CollectionInfo) {
    loadingDetail = true;
    renderedDescription = "";
    try {
      const [detail, mods] = await Promise.all([
        getCollection(collection.slug, collection.game_domain),
        getCollectionMods(collection.slug, collection.latest_revision),
      ]);
      selectedCollection = detail;
      selectedMods = mods;

      // Pre-render the description as markdown
      if (detail.description) {
        const html = await marked.parse(detail.description);
        renderedDescription = DOMPurify.sanitize(html);
      }
    } catch (e: unknown) {
      showError(`Failed to load collection details: ${e}`);
    } finally {
      loadingDetail = false;
    }
  }

  const installStepLabels: Record<string, string> = {
    preparing: "Preparing...",
    downloading: "Downloading...",
    extracting: "Extracting...",
    registering: "Recording files...",
    deploying: "Deploying...",
    "syncing-plugins": "Syncing plugins...",
  };

  async function handleInstallCollection() {
    if (!selectedCollection || !$selectedGame) return;

    // Build manifest first so we can check for required tools
    const manifest = {
      name: selectedCollection.name,
      author: selectedCollection.author,
      description: selectedCollection.summary,
      game_domain: selectedCollection.game_domain,
      mods: selectedMods.map((m) => ({
        name: m.name,
        version: m.version,
        optional: m.optional,
        source: {
          type: m.source_type,
          url: m.download_url ?? null,
          instructions: m.instructions ?? null,
          modId: m.nexus_mod_id ?? null,
          fileId: m.nexus_file_id ?? null,
          updatePolicy: null,
          md5: null,
          fileSize: m.file_size ?? null,
        },
        choices: null,
        patches: null,
        instructions: m.instructions ?? null,
        phase: null,
        fileOverrides: [],
      })),
      modRules: [],
      plugins: [],
      installInstructions: null,
      slug: selectedCollection.slug ?? null,
      image_url: selectedCollection.image_url ?? null,
      revision: selectedCollection.latest_revision ?? null,
    };

    // Check for required tools before installing
    try {
      const manifestJson = JSON.stringify(manifest);
      const tools = await detectCollectionTools(manifestJson, $selectedGame.game_id, $selectedGame.bottle_name);
      const uninstalled = tools.filter((t) => !t.is_detected);
      if (uninstalled.length > 0) {
        pendingTools = tools;
        pendingManifest = manifest;
        showToolsPrompt = true;
        return;
      }
    } catch {
      // Tool detection is best-effort; proceed with install if it fails
    }

    await proceedWithInstall(manifest);
  }

  async function proceedWithInstall(manifest: CollectionManifest & Record<string, unknown>) {
    if (!selectedCollection || !$selectedGame) return;

    installing = true;
    installStep = "preparing";
    installModName = "";
    installProgress = { current: 0, total: 0 };
    installResult = null;
    userActions = [];
    installStartTime = Date.now();
    collectionInstallStatus.set({
      active: true,
      collectionName: selectedCollection.name,
      currentMod: "",
      step: "preparing",
      current: 0,
      total: 0,
    });
    installElapsed = "0s";
    elapsedInterval = setInterval(() => {
      const secs = Math.floor((Date.now() - installStartTime) / 1000);
      if (secs < 60) installElapsed = `${secs}s`;
      else installElapsed = `${Math.floor(secs / 60)}m ${secs % 60}s`;
    }, 1000);

    try {
      // Subscribe to progress events
      installUnlisten = await onInstallProgress((event: InstallProgressEvent) => {
        if (event.kind === "modStarted") {
          installModName = event.mod_name;
          installProgress = { current: event.mod_index + 1, total: event.total_mods };
          installStep = "preparing";
          collectionInstallStatus.set({
            active: true,
            collectionName: selectedCollection!.name,
            currentMod: event.mod_name,
            step: "preparing",
            current: event.mod_index + 1,
            total: event.total_mods,
          });
        } else if (event.kind === "stepChanged") {
          installStep = event.step;
          collectionInstallStatus.update(s => s ? { ...s, step: event.step } : s);
        } else if (event.kind === "downloadProgress") {
          installStep = "downloading";
          collectionInstallStatus.update(s => s ? { ...s, step: "downloading" } : s);
        } else if (event.kind === "modCompleted") {
          installStep = "";
        } else if (event.kind === "modFailed") {
          installStep = "";
        } else if (event.kind === "userActionRequired") {
          userActions = [...userActions, { mod_name: event.mod_name, action: event.action, url: event.url, instructions: event.instructions }];
        } else if (event.kind === "collectionCompleted") {
          installStep = "complete";
          collectionInstallStatus.set(null);
        }
      });

      const result = await installCollection(
        manifest,
        $selectedGame.game_id,
        $selectedGame.bottle_name
      );

      installResult = result;

      // Toast is minimal — details shown in the result panel below
      if (result.failed === 0 && result.skipped === 0) {
        showSuccess(`Collection "${selectedCollection.name}" installed successfully`);
      }
    } catch (e: unknown) {
      showError(`Collection install failed: ${e}`);
    } finally {
      installing = false;
      installStep = "";
      installModName = "";
      collectionInstallStatus.set(null);
      if (elapsedInterval) { clearInterval(elapsedInterval); elapsedInterval = null; }
      if (installUnlisten) { installUnlisten(); installUnlisten = null; }
    }
  }

  function backToBrowse() {
    selectedCollection = null;
    selectedMods = [];
    renderedDescription = "";
  }

  function formatSize(bytes: number): string {
    if (bytes >= 1073741824) return `${(bytes / 1073741824).toFixed(1)} GB`;
    if (bytes >= 1048576) return `${(bytes / 1048576).toFixed(0)} MB`;
    if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KB`;
    return `${bytes} B`;
  }

  function formatNumber(n: number): string {
    if (n >= 1000000) return `${(n / 1000000).toFixed(1)}M`;
    if (n >= 1000) return `${(n / 1000).toFixed(1)}K`;
    return n.toString();
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
    };
    return map[domain] || domain;
  }

  function sourceTypeLabel(type: string): string {
    switch (type) {
      case "nexus": return "Nexus";
      case "manual": return "Manual";
      case "bundled": return "Bundled";
      case "direct": return "Direct";
      case "browse": return "Browse";
      default: return type.charAt(0).toUpperCase() + type.slice(1);
    }
  }

  function sourceTypeColor(type: string): string {
    switch (type) {
      case "nexus": return "var(--system-accent)";
      case "manual": return "var(--yellow)";
      case "bundled": return "var(--green)";
      case "direct": return "var(--green)";
      case "browse": return "var(--yellow)";
      default: return "var(--text-tertiary)";
    }
  }

  function sourceTypeBg(type: string): string {
    switch (type) {
      case "nexus": return "var(--system-accent-subtle)";
      case "manual": return "var(--yellow-subtle)";
      case "bundled": return "var(--green-subtle)";
      case "direct": return "var(--green-subtle)";
      case "browse": return "var(--yellow-subtle)";
      default: return "var(--surface-hover)";
    }
  }
</script>

<div class="collections-page">
  <!-- Tab Switcher -->
  <div class="tab-bar">
    <button class="tab-btn" class:tab-active={activeTab === "my"} onclick={() => activeTab = "my"}>
      My Collections
      {#if myCollections.length > 0}
        <span class="tab-count">{myCollections.length}</span>
      {/if}
    </button>
    <button class="tab-btn" class:tab-active={activeTab === "nexus"} onclick={() => activeTab = "nexus"}>
      <NexusLogo size={14} />
      Nexus Mods Collections
    </button>
    <button class="tab-btn" class:tab-active={activeTab === "wabbajack"} onclick={() => activeTab = "wabbajack"}>
      <WabbajackLogo size={14} />
      Wabbajack Lists
    </button>
    <button class="tab-btn" class:tab-active={activeTab === "browse_mods"} onclick={() => activeTab = "browse_mods"}>
      <NexusLogo size={14} />
      Browse Nexus
    </button>
  </div>

  {#if activeTab === "my"}
    <!-- My Collections Tab -->
    {#if selectedMyCollection}
      <!-- Local Collection Detail View -->
      <div class="detail-view">
        <div class="detail-header">
          <button class="btn btn-ghost" onclick={backToMyCollections}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M19 12H5" />
              <polyline points="12 19 5 12 12 5" />
            </svg>
            Back to My Collections
          </button>
        </div>

        {#if selectedMyCollection.image_url}
          <div class="local-detail-hero">
            <img src={selectedMyCollection.image_url} alt={selectedMyCollection.name} />
          </div>
        {/if}

        <div class="detail-content">
          <div class="detail-title-section">
            <div class="detail-title-row">
              <h2 class="detail-name">{selectedMyCollection.name}</h2>
              {#if selectedMyCollection.game_domain}
                <span class="game-badge">{gameDomainDisplay(selectedMyCollection.game_domain)}</span>
              {/if}
            </div>
            {#if selectedMyCollection.author}
              <p class="detail-author">by {selectedMyCollection.author}</p>
            {/if}
            {#if selectedMyCollection.installed_revision}
              <span class="detail-revision">Revision {selectedMyCollection.installed_revision}</span>
            {/if}
          </div>

          <!-- Stats Bar -->
          <div class="detail-stats-bar">
            <div class="detail-stat">
              <span class="detail-stat-value">{selectedMyCollection.mod_count}</span>
              <span class="detail-stat-label">Total Mods</span>
            </div>
            <div class="detail-stat">
              <span class="detail-stat-value">{selectedMyCollection.enabled_count}</span>
              <span class="detail-stat-label">Active</span>
            </div>
            <div class="detail-stat">
              <span class="detail-stat-value">{selectedMyCollection.mod_count - selectedMyCollection.enabled_count}</span>
              <span class="detail-stat-label">Disabled</span>
            </div>
          </div>

          <!-- Diff Panel -->
          {#if localDiff && localDiff !== "loading" && localDiff !== "error"}
            {@const diff = localDiff}
            <div class="detail-section">
              <h3 class="detail-section-title">
                Update Status
                {#if diff.added.length === 0 && diff.removed.length === 0 && diff.updated.length === 0}
                  <span class="diff-badge diff-badge-ok">Up to date</span>
                {:else}
                  <span class="diff-badge diff-badge-changes">{diff.added.length + diff.removed.length + diff.updated.length} changes</span>
                {/if}
              </h3>
              <div class="local-diff-panel">
                <div class="diff-header">
                  <span class="diff-revisions">
                    {#if diff.installed_revision}Rev {diff.installed_revision}{:else}Installed{/if}
                    &rarr; Rev {diff.latest_revision}
                  </span>
                </div>
                {#if diff.added.length > 0}
                  <div class="diff-section diff-added">
                    <span class="diff-label">+ {diff.added.length} added</span>
                    {#each diff.added as entry}
                      <span class="diff-item">{entry.name} {entry.version}</span>
                    {/each}
                  </div>
                {/if}
                {#if diff.removed.length > 0}
                  <div class="diff-section diff-removed">
                    <span class="diff-label">- {diff.removed.length} removed</span>
                    {#each diff.removed as entry}
                      <span class="diff-item">{entry.name} {entry.version}</span>
                    {/each}
                  </div>
                {/if}
                {#if diff.updated.length > 0}
                  <div class="diff-section diff-updated">
                    <span class="diff-label">~ {diff.updated.length} updated</span>
                    {#each diff.updated as entry}
                      <span class="diff-item">{entry.name}: {entry.installed_version} &rarr; {entry.latest_version}</span>
                    {/each}
                  </div>
                {/if}
                {#if diff.unchanged > 0}
                  <span class="diff-unchanged">{diff.unchanged} unchanged</span>
                {/if}
              </div>
            </div>
          {:else if localDiff === "loading"}
            <div class="detail-section">
              <h3 class="detail-section-title">Update Status</h3>
              <div class="local-diff-panel">
                <div class="diff-loading">
                  <span class="spinner-sm"></span>
                  <span>Checking for updates...</span>
                </div>
              </div>
            </div>
          {:else if localDiff === "error"}
            <div class="detail-section">
              <h3 class="detail-section-title">Update Status</h3>
              <div class="local-diff-panel diff-error">
                <span>Could not check for updates.</span>
              </div>
            </div>
          {/if}

          <!-- Installed Mods List -->
          <div class="detail-section">
            <h3 class="detail-section-title">
              Installed Mods
              <span class="title-count">{localCollectionMods.length}</span>
            </h3>
            {#if loadingLocalDetail}
              <div class="local-mods-loading">
                <div class="spinner"><div class="spinner-ring"></div></div>
                <span>Loading mods...</span>
              </div>
            {:else if localCollectionMods.length === 0}
              <div class="local-mods-empty">
                <span>No mods found for this collection.</span>
              </div>
            {:else}
              <div class="mods-table-container">
                <div class="mods-table">
                  <div class="mods-table-header local-mods-header">
                    <span class="col-mod-name">Name</span>
                    <span class="col-mod-version">Version</span>
                    <span class="col-local-status">Status</span>
                    <span class="col-local-priority">Priority</span>
                  </div>
                  <div class="mods-table-body">
                    {#each localCollectionMods as mod}
                      <div class="mods-table-row local-mods-row">
                        <span class="col-mod-name">
                          <span class="mod-name-text">{mod.name}</span>
                        </span>
                        <span class="col-mod-version">{mod.version || "\u2014"}</span>
                        <span class="col-local-status">
                          {#if mod.enabled}
                            <span class="local-status-badge local-status-enabled">Enabled</span>
                          {:else}
                            <span class="local-status-badge local-status-disabled">Disabled</span>
                          {/if}
                        </span>
                        <span class="col-local-priority">{mod.install_priority}</span>
                      </div>
                    {/each}
                  </div>
                </div>
              </div>
            {/if}
          </div>

          <!-- Actions -->
          <div class="local-detail-actions">
            <button
              class="btn btn-primary"
              onclick={() => { handleSwitchCollection(selectedMyCollection!.name); }}
              disabled={switchingCollection === selectedMyCollection.name}
            >
              {switchingCollection === selectedMyCollection.name ? "Activating..." : "Activate Collection"}
            </button>
            {#if confirmDeleteCollection === selectedMyCollection.name}
              <div class="delete-confirm">
                <label class="keep-downloads-label">
                  <input type="checkbox" bind:checked={deleteKeepDownloads} />
                  Keep shared downloads
                </label>
                <button
                  class="btn btn-danger btn-sm"
                  onclick={() => { handleDeleteCollection(selectedMyCollection!.name); backToMyCollections(); }}
                  disabled={deletingCollection === selectedMyCollection.name}
                >
                  {deletingCollection === selectedMyCollection.name ? "Deleting..." : "Confirm Delete"}
                </button>
                <button class="btn btn-ghost btn-sm" onclick={() => confirmDeleteCollection = null}>
                  Cancel
                </button>
              </div>
            {:else}
              <button
                class="btn btn-ghost-danger"
                onclick={() => confirmDeleteCollection = selectedMyCollection!.name}
              >
                Delete Collection
              </button>
            {/if}
          </div>
        </div>
      </div>

    {:else}
    <!-- My Collections Grid View -->
    <header class="page-header">
      <div class="header-text">
        <h2 class="page-title">My Collections</h2>
        <p class="page-subtitle">Manage installed mod collections — switch between them or remove ones you no longer need</p>
      </div>
    </header>

    {#if !$selectedGame}
      <div class="my-collections-empty">
        <p>Select a game from the Mods page first to view your installed collections.</p>
      </div>
    {:else if loadingMyCollections}
      <div class="my-collections-empty">
        <div class="spinner"><div class="spinner-ring"></div></div>
        <p>Loading collections...</p>
      </div>
    {:else if myCollections.length === 0}
      <div class="my-collections-empty">
        <p>No collections installed yet.</p>
        <p class="muted">Install a collection from Nexus Mods Collections to get started.</p>
        <button class="btn btn-secondary" onclick={() => activeTab = "nexus"}>
          <NexusLogo size={14} />
          Browse Nexus Mods Collections
        </button>
      </div>
    {:else}
      <div class="my-collections-grid">
        {#each myCollections as col}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div class="my-collection-card" role="button" tabindex="0" onclick={() => viewLocalCollection(col)} onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') viewLocalCollection(col); }}>
            <div class="my-card-image">
              {#if col.image_url}
                <img src={col.image_url} alt={col.name} loading="lazy" />
              {:else}
                <div class="my-card-image-placeholder">
                  <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.93a2 2 0 0 1-1.66-.9l-.82-1.2A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13c0 1.1.9 2 2 2Z"/>
                  </svg>
                </div>
              {/if}
            </div>
            <div class="my-collection-body">
              <h3 class="my-collection-name">{col.name}</h3>
              {#if col.author}
                <p class="my-collection-author">by {col.author}</p>
              {/if}
              <div class="my-collection-stats">
                <span>{col.mod_count} mods</span>
                <span class="stat-separator">&middot;</span>
                <span class:stat-active={col.enabled_count > 0}>{col.enabled_count} active</span>
                {#if col.installed_revision}
                  <span class="stat-separator">&middot;</span>
                  <span>Rev {col.installed_revision}</span>
                {/if}
              </div>
              <div class="my-collection-actions" onclick={(e) => e.stopPropagation()}>
                <button
                  class="btn btn-primary btn-sm"
                  onclick={() => handleSwitchCollection(col.name)}
                  disabled={switchingCollection === col.name}
                >
                  {switchingCollection === col.name ? "Switching..." : "Activate"}
                </button>
                <button
                  class="btn btn-ghost-danger btn-sm"
                  onclick={(e) => { e.stopPropagation(); confirmDeleteCollection = col.name; viewLocalCollection(col); }}
                >
                  Delete
                </button>
              </div>
            </div>
          </div>
        {/each}
      </div>
    {/if}
    {/if}
  {:else if checkingAuth}
    <!-- Checking account status -->
    <header class="page-header">
      <div class="header-text">
        <h2 class="page-title">Collections</h2>
        <p class="page-subtitle">Browse and install curated mod collections from Nexus Mods</p>
      </div>
    </header>
    <div class="loading-container">
      <div class="loading-card">
        <div class="spinner"><div class="spinner-ring"></div></div>
        <div class="loading-text">
          <p class="loading-title">Checking account</p>
          <p class="loading-detail">Verifying Nexus Mods connection...</p>
        </div>
      </div>
    </div>
  {:else if !account?.connected}
    <!-- Not connected — show connect prompt -->
    <header class="page-header">
      <div class="header-text">
        <h2 class="page-title">Collections</h2>
        <p class="page-subtitle">Browse and install curated mod collections from Nexus Mods</p>
      </div>
    </header>
    <div class="connect-prompt">
      <div class="connect-card">
        <div class="connect-icon">
          <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <rect x="2" y="3" width="20" height="14" rx="2" ry="2" />
            <line x1="8" y1="21" x2="16" y2="21" />
            <line x1="12" y1="17" x2="12" y2="21" />
          </svg>
        </div>
        <h3 class="connect-title">Connect to Nexus Mods</h3>
        <p class="connect-desc">
          Connect your Nexus Mods account to browse and install curated mod collections.
          Premium members get faster downloads.
        </p>
        <div class="connect-steps">
          <button
            class="btn btn-secondary btn-step"
            onclick={openNexusApiPage}
            type="button"
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
              <polyline points="15 3 21 3 21 9" />
              <line x1="10" y1="14" x2="21" y2="3" />
            </svg>
            Get API Key from Nexus Mods
          </button>
          <div class="connect-input-row">
            <input
              type="password"
              class="connect-input"
              placeholder="Paste your API key here"
              bind:value={apiKeyInput}
              onkeydown={(e) => { if (e.key === "Enter") handleConnect(); }}
              oninput={() => { validationError = null; }}
            />
            <button
              class="btn btn-primary btn-connect"
              onclick={handleConnect}
              disabled={signingIn || !apiKeyInput.trim()}
            >
              {#if signingIn}
                <span class="spinner-sm"></span>
                Verifying...
              {:else}
                Connect
              {/if}
            </button>
          </div>
          {#if validationError}
            <span class="connect-error">{validationError}</span>
          {/if}
        </div>
      </div>
    </div>
  {:else if selectedCollection && !loadingDetail}
    <!-- Collection Detail View -->
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
            <h2 class="detail-name">{selectedCollection.name}</h2>
            <span class="game-badge">{gameDomainDisplay(selectedCollection.game_domain)}</span>
          </div>
          <p class="detail-author">by {selectedCollection.author}</p>
          <span class="detail-revision">Revision {selectedCollection.latest_revision}</span>
        </div>

        <!-- Stats Bar -->
        <div class="detail-stats-bar">
          <div class="detail-stat">
            <span class="detail-stat-value">{selectedCollection.total_mods}</span>
            <span class="detail-stat-label">Mods</span>
          </div>
          <div class="detail-stat">
            <span class="detail-stat-value">{formatNumber(selectedCollection.total_downloads)}</span>
            <span class="detail-stat-label">Downloads</span>
          </div>
          <div class="detail-stat">
            <span class="detail-stat-value">{formatNumber(selectedCollection.endorsements)}</span>
            <span class="detail-stat-label">Endorsements</span>
          </div>
          {#if selectedCollection.download_size}
            <div class="detail-stat">
              <span class="detail-stat-value">{formatSize(selectedCollection.download_size)}</span>
              <span class="detail-stat-label">Download Size</span>
            </div>
          {/if}
          <div class="detail-stat">
            <span class="detail-stat-value">Rev. {selectedCollection.latest_revision}</span>
            <span class="detail-stat-label">Latest</span>
          </div>
        </div>

        <!-- Description -->
        {#if renderedDescription}
          <div class="detail-section">
            <h3 class="detail-section-title">Description</h3>
            <div class="rendered-markdown">
              {@html renderedDescription}
            </div>
          </div>
        {/if}

        <!-- Mod List Table -->
        {#if selectedMods.length > 0}
          <div class="detail-section">
            <h3 class="detail-section-title">
              Mods
              <span class="title-count">{selectedMods.length}</span>
            </h3>
            <div class="mods-table-container">
              <div class="mods-table">
                <div class="mods-table-header">
                  <span class="col-mod-name">Name</span>
                  <span class="col-mod-version">Version</span>
                  <span class="col-mod-source">Source</span>
                  <span class="col-mod-optional">Required</span>
                </div>
                <div class="mods-table-body">
                  {#each selectedMods as mod, i}
                    <div class="mods-table-row">
                      <span class="col-mod-name">
                        <span class="mod-name-text">{mod.name}</span>
                      </span>
                      <span class="col-mod-version">{mod.version || "\u2014"}</span>
                      <span class="col-mod-source">
                        <span
                          class="source-badge"
                          style="color: {sourceTypeColor(mod.source_type)}; background: {sourceTypeBg(mod.source_type)};"
                        >
                          {sourceTypeLabel(mod.source_type)}
                        </span>
                      </span>
                      <span class="col-mod-optional">
                        {#if mod.optional}
                          <span class="optional-badge">Optional</span>
                        {:else}
                          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--green)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <path d="M20 6L9 17l-5-5" />
                          </svg>
                        {/if}
                      </span>
                    </div>
                  {/each}
                </div>
              </div>
            </div>
          </div>
        {/if}

        <!-- Compatibility Check (Skyrim SE only) -->
        {#if selectedCollection.game_domain === "skyrimspecialedition" && $selectedGame}
          <div class="detail-section">
            <CompatibilityPanel gameId={$selectedGame.game_id} bottleName={$selectedGame.bottle_name} />
          </div>
        {/if}

        <!-- Install Button -->
        <div class="detail-install-bar">
          {#if installing}
            <div class="install-progress-panel">
              <div class="install-progress-header">
                <span class="spinner-sm"></span>
                <span>
                  {#if installProgress.total > 0}
                    Installing mod {installProgress.current} of {installProgress.total}
                  {:else}
                    Preparing...
                  {/if}
                </span>
                {#if installProgress.total > 0}
                  <span class="install-progress-pct">
                    {Math.round((installProgress.current / installProgress.total) * 100)}%
                  </span>
                {/if}
              </div>
              {#if installModName}
                <div class="install-progress-mod">
                  {installModName}
                  {#if installStep && installStepLabels[installStep]}
                    <span class="install-progress-step-inline">{installStepLabels[installStep]}</span>
                  {/if}
                </div>
              {:else if installStep && installStepLabels[installStep]}
                <div class="install-progress-step">{installStepLabels[installStep]}</div>
              {/if}
              {#if installProgress.total > 0}
                <div class="install-progress-bar-row">
                  <div class="install-progress-bar">
                    <div
                      class="install-progress-fill"
                      style="width: {(installProgress.current / installProgress.total) * 100}%"
                    ></div>
                  </div>
                  <span class="install-progress-elapsed">{installElapsed}</span>
                </div>
              {/if}
              {#if userActions.length > 0}
                <div class="user-actions-list">
                  <h4 class="user-actions-title">Manual Downloads Required</h4>
                  {#each userActions as action}
                    <div class="user-action-item">
                      <div class="user-action-info">
                        <span class="user-action-mod">{action.mod_name}</span>
                        {#if action.instructions}
                          <span class="user-action-instructions">{action.instructions}</span>
                        {/if}
                      </div>
                      {#if action.url}
                        <button class="btn btn-secondary btn-sm" onclick={() => safeOpenUrl(action.url)}>
                          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
                            <polyline points="15 3 21 3 21 9" />
                            <line x1="10" y1="14" x2="21" y2="3" />
                          </svg>
                          Open in Browser
                        </button>
                      {/if}
                    </div>
                  {/each}
                </div>
              {/if}
            </div>
          {:else if installResult}
            <div class="install-result-panel">
              <!-- Header -->
              <div class="result-header">
                {#if installResult.failed === 0 && installResult.skipped === 0}
                  <svg class="result-header-icon result-header-icon--success" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14" />
                    <polyline points="22 4 12 14.01 9 11.01" />
                  </svg>
                {:else}
                  <svg class="result-header-icon result-header-icon--warning" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <circle cx="12" cy="12" r="10" />
                    <line x1="12" y1="8" x2="12" y2="12" />
                    <line x1="12" y1="16" x2="12.01" y2="16" />
                  </svg>
                {/if}
                <div class="result-header-text">
                  <h3 class="result-title">
                    {installResult.failed === 0 && installResult.skipped === 0
                      ? "Collection Installed"
                      : "Install Complete"}
                  </h3>
                  <div class="result-counts">
                    {#if installResult.installed > 0}
                      <span class="result-count result-count--installed">{installResult.installed} installed</span>
                    {/if}
                    {#if installResult.already_installed > 0}
                      <span class="result-count result-count--existing">{installResult.already_installed} already installed</span>
                    {/if}
                    {#if installResult.skipped > 0}
                      <span class="result-count result-count--action">{installResult.skipped} need action</span>
                    {/if}
                    {#if installResult.failed > 0}
                      <span class="result-count result-count--failed">{installResult.failed} failed</span>
                    {/if}
                  </div>
                </div>
              </div>

              <!-- Per-mod details -->
              <div class="result-mod-list">
                {#each installResult.details.filter(d => d.status === "installed") as detail}
                  <div class="result-mod-row">
                    <svg class="result-mod-icon result-mod-icon--installed" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                      <polyline points="20 6 9 17 4 12" />
                    </svg>
                    <span class="result-mod-name">{detail.name}</span>
                  </div>
                {/each}
                {#each installResult.details.filter(d => d.status === "already_installed") as detail}
                  <div class="result-mod-row">
                    <svg class="result-mod-icon result-mod-icon--existing" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                      <polyline points="20 6 9 17 4 12" />
                    </svg>
                    <span class="result-mod-name">{detail.name}</span>
                    <span class="result-mod-badge result-mod-badge--existing">Already installed</span>
                  </div>
                {/each}
                {#each installResult.details.filter(d => d.status === "user_action") as detail}
                  <div class="result-mod-card result-mod-card--action">
                    <div class="result-mod-card-header">
                      <svg class="result-mod-icon result-mod-icon--action" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
                        <line x1="12" y1="9" x2="12" y2="13" />
                        <line x1="12" y1="17" x2="12.01" y2="17" />
                      </svg>
                      <span class="result-mod-name">{detail.name}</span>
                    </div>
                    {#if detail.instructions}
                      <p class="result-mod-instructions">{detail.instructions}</p>
                    {:else if detail.error}
                      <p class="result-mod-instructions">{detail.error}</p>
                    {/if}
                    {#if detail.url}
                      <button class="btn btn-secondary btn-sm" onclick={() => safeOpenUrl(detail.url)}>
                        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                          <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
                          <polyline points="15 3 21 3 21 9" />
                          <line x1="10" y1="14" x2="21" y2="3" />
                        </svg>
                        Open in Browser
                      </button>
                    {/if}
                  </div>
                {/each}
                {#each installResult.details.filter(d => d.status === "failed") as detail}
                  <div class="result-mod-card result-mod-card--failed">
                    <div class="result-mod-card-header">
                      <svg class="result-mod-icon result-mod-icon--failed" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                        <line x1="18" y1="6" x2="6" y2="18" />
                        <line x1="6" y1="6" x2="18" y2="18" />
                      </svg>
                      <span class="result-mod-name">{detail.name}</span>
                    </div>
                    {#if detail.error}
                      <p class="result-mod-error">{detail.error}</p>
                    {/if}
                  </div>
                {/each}
              </div>

              <!-- Post-install actions -->
              <div class="result-actions">
                <button class="btn btn-primary btn-sm" onclick={() => goto("/mods")}>
                  View Installed Mods
                </button>
                <button class="btn btn-ghost btn-sm" onclick={() => installResult = null}>
                  Dismiss
                </button>
              </div>
            </div>
          {:else}
            <button
              class="btn btn-primary btn-lg"
              onclick={handleInstallCollection}
              disabled={!$selectedGame}
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                <polyline points="7 10 12 15 17 10" />
                <line x1="12" y1="15" x2="12" y2="3" />
              </svg>
              Install Collection
            </button>
            {#if !$selectedGame}
              <span class="install-hint">Select a game from the Mods page first</span>
            {/if}
          {/if}
        </div>
      </div>
    </div>

  {:else if activeTab === "wabbajack"}
    <!-- Wabbajack Tab (embedded) -->
    {#await import("../modlists/+page.svelte")}
      <div style="display:flex;align-items:center;justify-content:center;min-height:200px;">
        <div class="spinner"><div class="spinner-ring"></div></div>
      </div>
    {:then mod}
      <mod.default />
    {:catch}
      <p style="color: var(--text-tertiary); text-align: center; padding: 48px;">Failed to load Wabbajack Lists.</p>
    {/await}

  {:else if activeTab === "browse_mods"}
    <!-- Browse Nexus Tab -->
    <header class="page-header">
      <div class="header-text">
        <h2 class="page-title"><NexusLogo size={22} /> Browse Nexus</h2>
        <p class="page-subtitle">Discover mods on NexusMods for {$selectedGame?.display_name ?? "your game"}</p>
      </div>
      <div class="header-right">
        {#if !browseModsLoading && browseModsTotalCount > 0}
          <div class="stat-pill">
            <span class="stat-value">{browseModsTotalCount.toLocaleString()}</span>
            <span class="stat-label">{browseModsTotalCount === 1 ? "mod" : "mods"}</span>
          </div>
        {/if}
      </div>
    </header>

    {#if !$selectedGame}
      <div class="empty-state">
        <p class="empty-title">No game selected</p>
        <p class="empty-detail">Select a game from the sidebar to browse mods.</p>
      </div>
    {:else}
      <div class="filters-bar">
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
            <option value="endorsements">Sort: Endorsements</option>
            <option value="downloads">Sort: Downloads</option>
            <option value="name">Sort: Name</option>
            <option value="updated">Sort: Updated</option>
          </select>
        </div>
        <label class="nsfw-toggle">
          <input type="checkbox" bind:checked={browseModsShowNsfw} onchange={() => loadBrowseMods()} />
          <span>NSFW</span>
        </label>
      </div>

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
          {#each browseMods as mod}
            <button class="mod-browse-card" onclick={() => openModPage(mod)}>
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
              </div>
            </button>
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

        <p class="browse-mods-hint">Click a mod to view it on NexusMods. Free users download via the website; premium users can use NXM links.</p>
      {/if}
    {/if}

  {:else if activeTab === "nexus"}
    <!-- Nexus Mods Collections -->
    <header class="page-header">
      <div class="header-text">
        <h2 class="page-title"><NexusLogo size={22} /> Nexus Mods Collections</h2>
        <p class="page-subtitle">Browse and install curated mod collections from Nexus Mods</p>
      </div>
      <div class="header-right">
        {#if account?.connected}
          <div class="account-badge">
            <div class="account-avatar-sm">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2" />
                <circle cx="12" cy="7" r="4" />
              </svg>
            </div>
            <span class="account-name">{account.name}</span>
            {#if account.is_premium}
              <span class="premium-pill">Premium</span>
            {/if}
          </div>
        {/if}
        {#if !loading}
          <div class="stat-pill">
            <span class="stat-value">{filtered.length}</span>
            <span class="stat-label">{filtered.length === 1 ? "Collection" : "Collections"}</span>
          </div>
        {/if}
      </div>
    </header>

    {#if loading || loadingDetail}
      <div class="loading-container">
        <div class="loading-card">
          <div class="spinner"><div class="spinner-ring"></div></div>
          <div class="loading-text">
            <p class="loading-title">{loadingDetail ? "Loading collection" : "Fetching collections"}</p>
            <p class="loading-detail">{loadingDetail ? "Loading collection details..." : "Loading collections from Nexus Mods..."}</p>
          </div>
        </div>
      </div>
    {:else}
      <!-- Filters -->
      <div class="filters-bar">
        <div class="search-wrapper">
          <svg class="search-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="11" cy="11" r="8" />
            <line x1="21" y1="21" x2="16.65" y2="16.65" />
          </svg>
          <input
            type="text"
            class="search-input"
            placeholder="Search collections..."
            bind:value={searchQuery}
            oninput={() => {
              if (collectionsSearchTimer) clearTimeout(collectionsSearchTimer);
              collectionsSearchTimer = setTimeout(() => reloadWithSort(), 400);
            }}
          />
        </div>
        <select class="filter-select" bind:value={gameFilter} onchange={() => { if (gameFilter !== "all") loadCollections(gameFilter); }}>
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
          <select class="filter-select" bind:value={sortField} onchange={reloadWithSort}>
            <option value="endorsements">Sort: Endorsements</option>
            <option value="downloads">Sort: Downloads</option>
            <option value="name">Sort: Name</option>
            <option value="rating">Sort: Rating</option>
            <option value="created">Sort: Newest</option>
            <option value="updated">Sort: Updated</option>
            <option value="mods">Sort: Mod Count</option>
          </select>
          <button
            class="sort-direction-btn"
            onclick={() => { sortDirection = sortDirection === "asc" ? "desc" : "asc"; reloadWithSort(); }}
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
          <div class="empty-icon">
            <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
              <rect x="3" y="3" width="18" height="18" rx="2" ry="2" />
              <line x1="3" y1="9" x2="21" y2="9" />
              <line x1="9" y1="21" x2="9" y2="9" />
            </svg>
          </div>
          <p class="empty-title">No collections found</p>
          <p class="empty-detail">
            {#if searchQuery || gameFilter !== "all"}
              Try adjusting your search or filters.
            {:else}
              No collections are currently available. Connect your Nexus Mods API key in Settings.
            {/if}
          </p>
        </div>
      {:else}
        <div class="collection-grid">
          {#each filtered as collection, i (collection.slug)}
            <div
              class="collection-card"
              style="animation-delay: {Math.min(i, 20) * 30}ms"
            >
              {#if collection.image_url}
                <div class="card-image">
                  <img src={collection.image_url} alt={collection.name} loading="lazy" />
                </div>
              {:else}
                <div class="card-image card-image-placeholder">
                  <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                    <rect x="3" y="3" width="18" height="18" rx="2" ry="2" />
                    <circle cx="8.5" cy="8.5" r="1.5" />
                    <polyline points="21 15 16 10 5 21" />
                  </svg>
                </div>
              {/if}

              <div class="card-body">
                <div class="card-top">
                  <span class="game-badge">{gameDomainDisplay(collection.game_domain)}</span>
                  <span class="revision-badge">Rev {collection.latest_revision}</span>
                </div>

                <h3 class="card-title">{collection.name}</h3>
                <p class="card-author">by {collection.author}</p>

                {#if collection.summary}
                  <p class="card-desc">{collection.summary}</p>
                {/if}

                {#if collection.tags.length > 0}
                  <div class="card-tags">
                    {#each collection.tags.slice(0, 5) as tag}
                      <span class="tag">{tag}</span>
                    {/each}
                  </div>
                {/if}

                <div class="card-stats">
                  <div class="stat-item">
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <rect x="3" y="3" width="7" height="7" />
                      <rect x="14" y="3" width="7" height="7" />
                      <rect x="3" y="14" width="7" height="7" />
                      <rect x="14" y="14" width="7" height="7" />
                    </svg>
                    <span class="stat-num">{collection.total_mods}</span>
                    <span class="stat-lbl">mods</span>
                  </div>
                  <div class="stat-item">
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                      <polyline points="7 10 12 15 17 10" />
                      <line x1="12" y1="15" x2="12" y2="3" />
                    </svg>
                    <span class="stat-num">{formatNumber(collection.total_downloads)}</span>
                  </div>
                  <div class="stat-item">
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3zM7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3" />
                    </svg>
                    <span class="stat-num">{formatNumber(collection.endorsements)}</span>
                  </div>
                  {#if collection.download_size}
                    <div class="stat-item">
                      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z" />
                      </svg>
                      <span class="stat-num">{formatSize(collection.download_size)}</span>
                    </div>
                  {/if}
                </div>

                <div class="card-actions">
                  <button
                    class="btn btn-accent btn-sm"
                    onclick={() => viewCollectionDetail(collection)}
                  >
                    View Details
                  </button>
                </div>
              </div>
            </div>
          {/each}
        </div>

        <!-- Pagination -->
        {#if collectionsTotalPages > 1}
          <div class="pagination-bar">
            <button
              class="btn btn-sm"
              disabled={collectionsCurrentPage <= 1 || loading}
              onclick={() => collectionsGoToPage(collectionsCurrentPage - 1)}
            >Previous</button>
            <div class="page-numbers">
              {#each Array.from({ length: Math.min(collectionsTotalPages, 7) }, (_, i) => {
                if (collectionsTotalPages <= 7) return i + 1;
                if (collectionsCurrentPage <= 4) return i + 1;
                if (collectionsCurrentPage >= collectionsTotalPages - 3) return collectionsTotalPages - 6 + i;
                return collectionsCurrentPage - 3 + i;
              }) as page}
                <button
                  class="page-btn"
                  class:active={page === collectionsCurrentPage}
                  disabled={loading}
                  onclick={() => collectionsGoToPage(page)}
                >{page}</button>
              {/each}
            </div>
            <button
              class="btn btn-sm"
              disabled={collectionsCurrentPage >= collectionsTotalPages || loading}
              onclick={() => collectionsGoToPage(collectionsCurrentPage + 1)}
            >Next</button>
            <span class="page-info">{collectionsTotalCount} collections</span>
          </div>
        {/if}
      {/if}
    {/if}
  {/if}
</div>

{#if showToolsPrompt && $selectedGame}
  <RequiredToolsPrompt
    tools={pendingTools}
    gameId={$selectedGame.game_id}
    bottleName={$selectedGame.bottle_name}
    oncontinue={() => {
      showToolsPrompt = false;
      if (pendingManifest) proceedWithInstall(pendingManifest);
    }}
    oncancel={() => {
      showToolsPrompt = false;
      pendingManifest = null;
      pendingTools = [];
    }}
  />
{/if}

<style>
  /* ---- Page Layout ---- */

  .collections-page {
    padding: var(--space-2) 0 var(--space-12) 0;
  }

  /* ---- Connect Prompt ---- */

  .connect-prompt {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 400px;
  }

  .connect-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: var(--space-4);
    padding: var(--space-12) var(--space-10);
    max-width: 420px;
  }

  .connect-icon {
    color: var(--text-quaternary);
    margin-bottom: var(--space-2);
  }

  .connect-title {
    font-size: 20px;
    font-weight: 700;
    letter-spacing: -0.02em;
    color: var(--text-primary);
  }

  .connect-desc {
    font-size: 14px;
    color: var(--text-secondary);
    line-height: 1.6;
  }

  .connect-steps {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    margin-top: var(--space-2);
  }

  .btn-step {
    padding: var(--space-2) var(--space-4);
    font-size: 13px;
  }

  .connect-input-row {
    display: flex;
    gap: var(--space-2);
    width: 100%;
  }

  .connect-input {
    flex: 1;
    min-width: 0;
    padding: var(--space-2) var(--space-3);
    background: var(--bg-base);
    border: 1px solid var(--separator-opaque);
    border-radius: var(--radius-sm);
    color: var(--text-primary);
    font-size: 13px;
    font-family: var(--font-sans);
    outline: none;
    transition: border-color var(--duration) var(--ease);
  }

  .connect-input:focus {
    border-color: var(--system-accent);
    box-shadow: 0 0 0 3px rgba(0, 122, 255, 0.15);
  }

  .connect-input::placeholder {
    color: var(--text-tertiary);
  }

  .btn-connect {
    padding: var(--space-2) var(--space-4);
    font-size: 13px;
    flex-shrink: 0;
  }

  .connect-error {
    font-size: 12px;
    color: var(--red);
  }

  /* ---- Header ---- */

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

  .header-right {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .account-badge {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    padding: var(--space-2) var(--space-3);
  }

  .account-avatar-sm {
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: var(--surface-hover);
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-tertiary);
    flex-shrink: 0;
  }

  .account-name {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
  }

  .premium-pill {
    font-size: 9px;
    font-weight: 700;
    color: #ff9f0a;
    background: rgba(255, 159, 10, 0.15);
    padding: 1px 5px;
    border-radius: 100px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
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

  /* ---- Loading ---- */

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

  .spinner-sm {
    display: inline-block;
    width: 14px;
    height: 14px;
    border: 2px solid rgba(255, 255, 255, 0.3);
    border-top-color: #fff;
    border-radius: 50%;
    animation: spin 0.75s linear infinite;
    flex-shrink: 0;
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

  /* ---- Filters ---- */

  .filters-bar {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-bottom: var(--space-6);
  }

  .search-wrapper {
    flex: 1;
    position: relative;
  }

  .search-icon {
    position: absolute;
    left: var(--space-3);
    top: 50%;
    transform: translateY(-50%);
    color: var(--text-tertiary);
    pointer-events: none;
  }

  .search-input {
    width: 100%;
    padding: var(--space-2) var(--space-3) var(--space-2) 36px;
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

  /* ---- Grid ---- */

  .collection-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
    gap: var(--space-4);
  }

  /* ---- Cards ---- */

  .collection-card {
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

  .collection-card:hover {
    border-color: rgba(255, 255, 255, 0.12);
    box-shadow: var(--glass-edge-shadow), var(--shadow-sm);
  }

  @keyframes cardFadeIn {
    from { opacity: 0; transform: translateY(6px); }
    to { opacity: 1; transform: translateY(0); }
  }

  .card-image {
    width: 100%;
    aspect-ratio: 16 / 9;
    overflow: hidden;
    background: var(--bg-secondary);
  }

  .card-image img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .card-image-placeholder {
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-quaternary);
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

  .revision-badge {
    font-size: 10px;
    font-weight: 500;
    color: var(--text-tertiary);
    background: var(--surface-hover);
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
    display: flex;
    align-items: center;
    gap: var(--space-4);
    padding: var(--space-3) 0;
    border-top: 1px solid var(--separator);
    margin-bottom: var(--space-3);
  }

  .stat-item {
    display: flex;
    align-items: center;
    gap: 4px;
    color: var(--text-tertiary);
  }

  .stat-num {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
  }

  .stat-lbl {
    font-size: 11px;
    color: var(--text-tertiary);
  }

  /* removed .size-estimate — replaced by actual download sizes */

  .card-actions {
    display: flex;
    gap: var(--space-2);
    margin-top: auto;
  }

  /* ---- Buttons ---- */

  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    border-radius: var(--radius);
    font-weight: 600;
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
    white-space: nowrap;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
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

  .btn-accent {
    background: var(--system-accent);
    color: #fff;
    flex: 1;
  }

  .btn-accent:hover:not(:disabled) {
    filter: brightness(1.1);
    box-shadow: 0 1px 6px rgba(0, 122, 255, 0.25);
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

  .btn-secondary {
    background: var(--surface-hover);
    color: var(--text-primary);
    border: 1px solid var(--border);
  }

  .btn-secondary:hover {
    background: var(--surface-active);
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

  /* ---- Empty State ---- */

  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    text-align: center;
    padding: var(--space-12) var(--space-8);
    border: 1px dashed rgba(255, 255, 255, 0.1);
    border-radius: var(--radius-lg);
    background: rgba(255, 255, 255, 0.015);
    box-shadow: var(--glass-edge-shadow);
  }

  .empty-icon {
    color: var(--text-quaternary);
    margin-bottom: var(--space-1);
  }

  .empty-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .empty-detail {
    font-size: 13px;
    color: var(--text-tertiary);
    max-width: 360px;
    line-height: 1.55;
  }

  /* ---- Detail View ---- */

  .detail-view {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .detail-header {
    display: flex;
    align-items: center;
  }

  .detail-content {
    display: flex;
    flex-direction: column;
    gap: var(--space-6);
  }

  .detail-title-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .detail-title-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }

  .detail-name {
    font-size: 28px;
    font-weight: 700;
    letter-spacing: -0.025em;
  }

  .detail-author {
    font-size: 14px;
    color: var(--text-secondary);
  }

  .detail-revision {
    font-size: 12px;
    color: var(--text-tertiary);
    font-weight: 500;
  }

  .detail-stats-bar {
    display: flex;
    gap: var(--space-6);
    padding: var(--space-4) var(--space-5);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-lg);
  }

  .detail-stat {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .detail-stat-value {
    font-size: 15px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .detail-stat-label {
    font-size: 11px;
    font-weight: 500;
    color: var(--text-tertiary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .detail-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .detail-section-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.02em;
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .title-count {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-tertiary);
    background: var(--surface);
    padding: 0 var(--space-2);
    border-radius: 100px;
    font-variant-numeric: tabular-nums;
  }

  /* ---- Mod Table ---- */

  .mods-table-container {
    background: var(--surface);
    border-radius: var(--radius-lg);
    overflow: hidden;
    box-shadow: var(--glass-edge-shadow);
  }

  .mods-table {
    display: flex;
    flex-direction: column;
  }

  .mods-table-header {
    display: grid;
    grid-template-columns: 1fr 80px 80px 80px;
    padding: var(--space-2) var(--space-4);
    background: var(--bg-secondary);
    border-bottom: 1px solid var(--separator);
    font-size: 11px;
    font-weight: 500;
    color: var(--text-secondary);
    align-items: center;
  }

  .mods-table-body {
    max-height: 400px;
    overflow-y: auto;
  }

  .mods-table-row {
    display: grid;
    grid-template-columns: 1fr 80px 80px 80px;
    padding: var(--space-2) var(--space-4);
    align-items: center;
    font-size: 13px;
    transition: background var(--duration-fast) var(--ease);
  }

  .mods-table-row:nth-child(even) {
    background: rgba(255, 255, 255, 0.025);
  }

  :global([data-theme="light"]) .mods-table-row:nth-child(even) {
    background: rgba(0, 0, 0, 0.025);
  }

  .mods-table-row:hover {
    background: var(--surface-hover);
  }

  .col-mod-name {
    min-width: 0;
    overflow: hidden;
  }

  .mod-name-text {
    font-weight: 500;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    display: block;
  }

  .col-mod-version {
    font-size: 12px;
    color: var(--text-secondary);
    font-family: var(--font-mono);
    letter-spacing: 0;
  }

  .source-badge {
    display: inline-flex;
    align-items: center;
    padding: 1px 6px;
    border-radius: 4px;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.01em;
  }

  .col-mod-optional {
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .optional-badge {
    font-size: 10px;
    font-weight: 500;
    color: var(--text-tertiary);
    background: var(--surface-hover);
    padding: 1px 6px;
    border-radius: 4px;
  }

  /* ---- Install Bar ---- */

  .detail-install-bar {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-4) 0;
    border-top: 1px solid var(--separator);
  }

  .install-hint {
    font-size: 12px;
    color: var(--text-tertiary);
  }

  .install-progress-panel {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .install-progress-header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .install-progress-mod {
    font-size: 12px;
    color: var(--text-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .install-progress-pct {
    font-size: 12px;
    font-weight: 700;
    color: var(--system-accent);
    margin-left: auto;
    font-variant-numeric: tabular-nums;
  }

  .install-progress-step {
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .install-progress-step-inline {
    font-size: 11px;
    color: var(--text-tertiary);
    margin-left: var(--space-2);
  }

  .install-progress-bar-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .install-progress-elapsed {
    font-size: 11px;
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
    min-width: 32px;
    text-align: right;
  }

  .install-progress-bar {
    flex: 1;
    height: 4px;
    background: var(--surface-hover);
    border-radius: 2px;
    overflow: hidden;
  }

  .install-progress-fill {
    height: 100%;
    background: var(--system-accent, #007AFF);
    border-radius: 2px;
    transition: width 0.3s ease;
  }

  .user-actions-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-top: var(--space-2);
    padding: var(--space-3);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
  }

  .user-actions-title {
    font-size: 12px;
    font-weight: 600;
    color: #FF9500;
    margin-bottom: var(--space-1);
  }

  .user-action-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-2) 0;
    border-top: 1px solid var(--separator);
  }

  .user-action-item:first-of-type {
    border-top: none;
  }

  .user-action-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .user-action-mod {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .user-action-instructions {
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .install-result-panel {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-4);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-lg);
  }

  .result-header {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
  }

  .result-header-icon {
    flex-shrink: 0;
    margin-top: 2px;
  }

  .result-header-icon--success {
    color: #34C759;
  }

  .result-header-icon--warning {
    color: #FF9500;
  }

  .result-header-text {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .result-title {
    font-size: 15px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .result-counts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .result-count {
    font-size: 11px;
    font-weight: 600;
    padding: 1px 6px;
    border-radius: 4px;
  }

  .result-count--installed {
    color: #34C759;
    background: rgba(52, 199, 89, 0.12);
  }

  .result-count--existing {
    color: var(--system-accent);
    background: var(--system-accent-subtle);
  }

  .result-count--action {
    color: #FF9500;
    background: rgba(255, 149, 0, 0.12);
  }

  .result-count--failed {
    color: #FF3B30;
    background: rgba(255, 59, 48, 0.12);
  }

  .result-mod-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    max-height: 300px;
    overflow-y: auto;
  }

  .result-mod-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
  }

  .result-mod-row:hover {
    background: var(--surface-hover);
  }

  .result-mod-icon--installed {
    color: #34C759;
    flex-shrink: 0;
  }

  .result-mod-icon--existing {
    color: var(--system-accent);
    flex-shrink: 0;
  }

  .result-mod-icon--action {
    color: #FF9500;
    flex-shrink: 0;
  }

  .result-mod-icon--failed {
    color: #FF3B30;
    flex-shrink: 0;
  }

  .result-mod-name {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  .result-mod-badge {
    font-size: 10px;
    font-weight: 500;
    padding: 1px 5px;
    border-radius: 4px;
    flex-shrink: 0;
  }

  .result-mod-badge--existing {
    color: var(--system-accent);
    background: var(--system-accent-subtle);
  }

  .result-mod-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    border-radius: var(--radius);
    border: 1px solid var(--separator);
  }

  .result-mod-card--action {
    background: rgba(255, 149, 0, 0.04);
    border-color: rgba(255, 149, 0, 0.2);
  }

  .result-mod-card--failed {
    background: rgba(255, 59, 48, 0.04);
    border-color: rgba(255, 59, 48, 0.2);
  }

  .result-mod-card-header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .result-mod-instructions {
    font-size: 12px;
    color: var(--text-secondary);
    line-height: 1.5;
    margin: 0;
    padding-left: 22px;
  }

  .result-mod-error {
    font-size: 12px;
    color: var(--text-tertiary);
    line-height: 1.5;
    margin: 0;
    padding-left: 22px;
  }

  .result-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding-top: var(--space-2);
    border-top: 1px solid var(--separator);
  }

  /* ============================
     Tab Bar
     ============================ */
  .tab-bar {
    display: flex;
    gap: var(--space-1);
    padding: var(--space-1);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    flex-shrink: 0;
  }

  .tab-btn {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    border-radius: var(--radius-sm);
    font-size: 13px;
    font-weight: 600;
    color: var(--text-secondary);
    background: transparent;
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease), color var(--duration-fast) var(--ease);
  }

  .tab-btn:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .tab-active {
    background: var(--system-accent);
    color: var(--system-accent-on);
  }

  .tab-active:hover {
    background: var(--system-accent-hover);
    color: var(--system-accent-on);
  }

  .tab-count {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    border-radius: 100px;
    font-size: 10px;
    font-weight: 700;
    background: rgba(255, 255, 255, 0.2);
  }

  /* ============================
     My Collections
     ============================ */
  .my-collections-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    flex: 1;
    gap: var(--space-3);
    padding: var(--space-12);
    text-align: center;
    color: var(--text-secondary);
    font-size: 14px;
  }

  .my-collections-empty .muted {
    color: var(--text-tertiary);
    font-size: 13px;
  }

  .my-collections-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: var(--space-4);
  }

  .my-collection-card {
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    overflow: hidden;
    cursor: pointer;
    text-align: left;
    transition: background var(--duration-fast) var(--ease), border-color var(--duration-fast) var(--ease);
  }

  .my-collection-card:hover {
    background: var(--surface-hover);
    border-color: var(--accent-muted);
  }

  .my-card-image {
    width: 100%;
    height: 120px;
    overflow: hidden;
    background: var(--bg-tertiary);
  }

  .my-card-image img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .my-card-image-placeholder {
    width: 100%;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-quaternary);
  }

  .my-collection-body {
    padding: var(--space-3) var(--space-4);
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .my-collection-name {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .my-collection-author {
    font-size: 12px;
    color: var(--text-secondary);
    margin: 0;
  }

  .my-collection-stats {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: 12px;
    color: var(--text-tertiary);
    margin-top: 2px;
  }

  .stat-separator {
    color: var(--text-quaternary);
  }

  .stat-active {
    color: var(--green);
    font-weight: 500;
  }

  .my-collection-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }

  .delete-confirm {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .keep-downloads-label {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    font-size: 12px;
    color: var(--text-secondary);
    cursor: pointer;
  }

  .keep-downloads-label input {
    accent-color: var(--system-accent);
  }

  /* ---- Local Detail View ---- */

  .local-detail-hero {
    width: 100%;
    max-height: 200px;
    overflow: hidden;
    border-radius: var(--radius-lg);
    background: var(--bg-secondary);
  }

  .local-detail-hero img {
    width: 100%;
    height: 200px;
    object-fit: cover;
  }

  .local-diff-panel {
    padding: var(--space-3);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    font-size: 12px;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .diff-loading {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: 12px;
    color: var(--text-tertiary);
  }

  .local-mods-header {
    grid-template-columns: 1fr 80px 80px 60px;
  }

  .local-mods-row {
    grid-template-columns: 1fr 80px 80px 60px;
  }

  .col-local-status {
    display: flex;
    align-items: center;
  }

  .col-local-priority {
    font-size: 12px;
    color: var(--text-tertiary);
    text-align: center;
    font-variant-numeric: tabular-nums;
  }

  .local-status-badge {
    font-size: 10px;
    font-weight: 600;
    padding: 1px 6px;
    border-radius: 4px;
  }

  .local-status-enabled {
    color: var(--green);
    background: rgba(52, 199, 89, 0.12);
  }

  .local-status-disabled {
    color: var(--text-tertiary);
    background: var(--surface-hover);
  }

  .local-mods-loading {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    padding: var(--space-8);
    color: var(--text-tertiary);
    font-size: 13px;
  }

  .local-mods-empty {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--space-8);
    color: var(--text-tertiary);
    font-size: 13px;
  }

  .local-detail-actions {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-4) 0;
    border-top: 1px solid var(--separator);
  }

  .btn-ghost-danger {
    background: transparent;
    color: var(--red);
    padding: var(--space-2) var(--space-4);
    font-size: 13px;
    font-weight: 500;
  }

  .btn-ghost-danger:hover {
    background: rgba(255, 59, 48, 0.08);
  }

  .btn-danger {
    background: var(--red);
    color: #fff;
  }

  .btn-danger:hover:not(:disabled) {
    filter: brightness(1.1);
  }

  /* ---- Collection Diff ---- */
  .diff-panel {
    margin-top: var(--space-3);
    padding: var(--space-3);
    background: var(--bg-tertiary);
    border-radius: var(--radius-sm);
    font-size: 12px;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .diff-error {
    color: var(--text-tertiary);
    font-style: italic;
  }

  .diff-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .diff-revisions {
    font-weight: 500;
    color: var(--text-secondary);
  }

  .diff-badge {
    padding: 1px 6px;
    border-radius: 8px;
    font-size: 11px;
    font-weight: 500;
  }

  .diff-badge-ok {
    background: rgba(52, 199, 89, 0.15);
    color: var(--green);
  }

  .diff-badge-changes {
    background: rgba(255, 159, 10, 0.15);
    color: var(--yellow);
  }

  .diff-section {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .diff-label {
    font-weight: 600;
    font-size: 11px;
  }

  .diff-added .diff-label { color: var(--green); }
  .diff-removed .diff-label { color: var(--red); }
  .diff-updated .diff-label { color: var(--yellow); }

  .diff-item {
    color: var(--text-secondary);
    padding-left: var(--space-3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .diff-unchanged {
    color: var(--text-tertiary);
    font-size: 11px;
  }

  /* ---- Browse Mods Grid ---- */

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
    backdrop-filter: blur(4px);
    z-index: 1;
  }

  .browse-pagination {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-1);
    padding: var(--space-4) 0 var(--space-2);
  }

  .pagination-bar {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-4) 0 var(--space-2);
  }

  .page-numbers {
    display: flex;
    gap: 2px;
  }

  .page-btn {
    min-width: 32px;
    height: 32px;
    border: 1px solid var(--border);
    background: var(--surface-1);
    color: var(--text-secondary);
    border-radius: var(--radius-sm);
    cursor: pointer;
    font-size: var(--font-xs);
    transition: all 0.15s ease;
  }

  .page-btn:hover:not(:disabled) {
    background: var(--surface-2);
    color: var(--text-primary);
  }

  .page-btn.active {
    background: var(--accent);
    color: var(--text-on-accent, #fff);
    border-color: var(--accent);
  }

  .page-btn:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .page-info {
    font-size: var(--font-xs);
    color: var(--text-muted);
    margin-left: var(--space-2);
  }
</style>
