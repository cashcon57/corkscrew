<script lang="ts">
  import { onMount, onDestroy, untrack } from "svelte";
  import { goto } from "$app/navigation";
  import InstructionParser from "$lib/components/InstructionParser.svelte";
  import { selectedGame, showError, showSuccess, collectionInstallStatus, collectionUninstallStatus, modStateVersion, installedMods, collectionList, activeCollection } from "$lib/stores";
  import type { CollectionInfo, CollectionMod, CollectionSearchResult, InstalledMod, NexusModInfo, CollectionRevision } from "$lib/types";
  import {
    browseCollections,
    getCollection,
    getCollectionMods,
    getNexusAccountStatus,
    setConfigValue,
    getConfig,
    installCollection,
    listInstalledCollections,
    switchCollection,
    getCollectionDiff,
    getInstalledMods,
    closeBrowserWebview,
    checkDeploymentHealth,
    checkSkyrimVersion,
    listGameVersions,
    swapGameVersion,
    checkCachedFiles,
    quickCsModCount,
    getAllGames,
    startOAuthLogin,
    getCollectionRevisions,
  } from "$lib/api";
  import { listen } from "@tauri-apps/api/event";
  import type { CollectionSummary, CollectionDiff, DeploymentHealth } from "$lib/types";
  import { config } from "$lib/stores";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { marked } from "marked";
  import DOMPurify from "dompurify";
  import CompatibilityPanel from "$lib/components/CompatibilityPanel.svelte";
  import NexusLogo from "$lib/components/NexusLogo.svelte";
  import WabbajackLogo from "$lib/components/WabbajackLogo.svelte";
  import WineCompatBadge from "$lib/components/WineCompatBadge.svelte";
  import WebViewToggle from "$lib/components/WebViewToggle.svelte";
  import CollectionDeleteDialog from "$lib/components/collections/CollectionDeleteDialog.svelte";
  import InterruptedInstallBanner from "$lib/components/collections/InterruptedInstallBanner.svelte";
  import NexusBrowsePanel from "$lib/components/collections/NexusBrowsePanel.svelte";
  import SearchFilterBar from "$lib/components/SearchFilterBar.svelte";
  import CollectionInstallWizard from "$lib/components/collections/CollectionInstallWizard.svelte";
  import type { DetectedGame } from "$lib/types";

  const NEXUS_API_KEY_URL = "https://www.nexusmods.com/users/myaccount?tab=api+access";

  // ---- All detected games (for game selector dropdowns) ----
  let allDetectedGames = $state<DetectedGame[]>([]);
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
  let confirmDeleteCollection = $state<string | null>(null);
  let collectionDiffs = $state<Record<string, CollectionDiff | "loading" | "error">>({});
  let collectionHealth = $state<Record<string, DeploymentHealth | "loading" | "error">>({});
  let deployProgress = $state<{ current: number; total: number; mod_name: string; files_deployed: number; total_files: number } | null>(null);
  let healthCheckProgress = $state<{ step: string; message: string; current: number; total: number } | null>(null);

  async function handleVerifyCollection(colName: string) {
    const game = $selectedGame;
    if (!game) return;
    collectionHealth = { ...collectionHealth, [colName]: "loading" };
    healthCheckProgress = null;

    const { listen } = await import('@tauri-apps/api/event');
    const unlisten = await listen<typeof healthCheckProgress>('health-check-progress', (e) => {
      healthCheckProgress = e.payload as typeof healthCheckProgress;
    });

    try {
      const health = await checkDeploymentHealth(game.game_id, game.bottle_name);
      collectionHealth = { ...collectionHealth, [colName]: health };
    } catch {
      collectionHealth = { ...collectionHealth, [colName]: "error" };
    } finally {
      unlisten();
      healthCheckProgress = null;
    }
  }

  // Local detail view
  let selectedMyCollection = $state<CollectionSummary | null>(null);
  let localCollectionMods = $state<InstalledMod[]>([]);
  let loadingLocalDetail = $state(false);
  let localDiff = $state<CollectionDiff | "loading" | "error" | null>(null);

  // Virtual scrolling for local collection mods table
  const LOCAL_ROW_HEIGHT = 36;
  const LOCAL_SCROLL_BUFFER = 8;
  let localTableEl = $state<HTMLDivElement | null>(null);
  let localScrollTop = $state(0);
  let localContainerHeight = $state(400);

  let localVisibleRange = $derived((() => {
    const total = localCollectionMods.length;
    if (total === 0) return { start: 0, end: 0, paddingTop: 0, paddingBottom: 0 };
    const startRaw = Math.floor(localScrollTop / LOCAL_ROW_HEIGHT) - LOCAL_SCROLL_BUFFER;
    const visibleCount = Math.ceil(localContainerHeight / LOCAL_ROW_HEIGHT) + LOCAL_SCROLL_BUFFER * 2;
    const start = Math.max(0, startRaw);
    const end = Math.min(total, start + visibleCount);
    return {
      start,
      end,
      paddingTop: start * LOCAL_ROW_HEIGHT,
      paddingBottom: Math.max(0, (total - end) * LOCAL_ROW_HEIGHT),
    };
  })());

  function handleLocalTableScroll(e: Event) {
    localScrollTop = (e.target as HTMLDivElement).scrollTop;
  }

  $effect(() => {
    if (!localTableEl) return;
    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) localContainerHeight = entry.contentRect.height;
    });
    ro.observe(localTableEl);
    localContainerHeight = localTableEl.clientHeight;
    return () => ro.disconnect();
  });

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
    deployProgress = null;

    const { listen } = await import('@tauri-apps/api/event');
    const unlisten = await listen<typeof deployProgress>('deploy-progress', (e) => {
      deployProgress = e.payload as typeof deployProgress;
    });

    try {
      // Auto-swap game version if collection targets a different version
      if (game.game_id === "skyrimse") {
        const col = myCollections.find(c => c.name === name);
        if (col && col.game_versions.length > 0) {
          try {
            const status = await checkSkyrimVersion(game.game_id, game.bottle_name);
            const detectedIsSE = status.current_version.startsWith("1.5.");
            const colTargetsSE = col.game_versions.some(v => v.startsWith("1.5."));
            const colTargetsAE = col.game_versions.some(v => v.startsWith("1.6."));
            // Only swap if SE/AE categories differ
            const needsSwap = (detectedIsSE && !colTargetsSE && colTargetsAE)
              || (!detectedIsSE && colTargetsSE && !colTargetsAE);
            if (needsSwap) {
              const cached = await listGameVersions(game.game_id);
              const match = cached.find(cv =>
                colTargetsSE ? cv.version.startsWith("1.5.") : cv.version.startsWith("1.6.")
              );
              if (match) {
                await swapGameVersion(game.game_id, game.bottle_name, match.version);
                showSuccess(`Switched game to v${match.version} for this collection`);
              }
            }
          } catch { /* version swap is best-effort */ }
        }
      }

      await switchCollection(game.game_id, game.bottle_name, name);
      showSuccess(`Switched to "${name}" — mods deployed`);
      modStateVersion.update(n => n + 1);

      // Refresh deployment health after switching
      handleVerifyCollection(name);

      // Proactive Community Shaders detection after deploy
      if (game.game_id === "skyrimse") {
        try {
          const csCount = await quickCsModCount(game.game_id, game.bottle_name);
          if (csCount > 0) {
            showError(
              `Warning: ${csCount} Community Shaders mod(s) detected. These are incompatible with Wine/CrossOver. Go to Settings > Game > Shader Compatibility to convert or disable them.`
            );
          }
        } catch { /* CS check is best-effort */ }
      }

      await loadMyCollections();
    } catch (e: unknown) {
      showError(`Failed to switch: ${e}`);
    } finally {
      unlisten();
      switchingCollection = null;
      deployProgress = null;
    }
  }

  async function handleRepairCollection(col: CollectionSummary) {
    if (!col.slug || !col.game_domain || !$selectedGame) {
      showError("Cannot repair: collection metadata (slug/game domain) is missing. Try reinstalling from the Nexus tab.");
      return;
    }
    try {
      // Re-fetch collection detail and mod list from NexusMods
      const revision = col.installed_revision ?? 1;
      const [detail, modsResult] = await Promise.all([
        getCollection(col.slug, col.game_domain),
        getCollectionMods(col.slug, revision),
      ]);
      // Set as active selection and switch to detail view
      selectedCollection = detail;
      selectedMods = modsResult.mods;
      selectedGameVersions = modsResult.game_versions;
      if (detail.description) {
        const html = await marked.parse(detail.description);
        renderedDescription = DOMPurify.sanitize(html);
      }
      activeTab = "nexus";
      // Build manifest and start install (backend skips already-installed mods)
      await handleInstallCollection();
    } catch (e: unknown) {
      showError(`Repair failed: ${e}`);
    }
  }

  function humanizeUninstallStep(step: string): string {
    switch (step) {
      case "undeploying": return "Removing deployed files...";
      case "cleaning_staging": return "Cleaning staging files...";
      case "cleaning_downloads": return "Removing downloads...";
      case "removing_from_db": return "Removing from database...";
      default: return step;
    }
  }

  function showDeleteConfirmation(name: string) {
    confirmDeleteCollection = name;
  }

  function dismissUninstall() {
    collectionUninstallStatus.set(null);
    backToMyCollections();
    loadMyCollections();
  }

  $effect(() => {
    if (activeTab === "my" && $selectedGame) {
      loadMyCollections();
    }
  });

  // Auto-load Nexus Collections when switching to the nexus tab or changing game
  let collectionsInitializedForGame = $state<string | null>(null);
  $effect(() => {
    const game = $selectedGame;
    const tab = activeTab;
    const connected = untrack(() => account?.connected);
    if (tab === "nexus" && game && connected) {
      const slug = gameSlugMap[game.game_id] ?? game.game_id;
      if (untrack(() => collectionsInitializedForGame) !== slug) {
        collectionsInitializedForGame = slug;
        gameFilter = slug;
        loadCollections(slug);
      }
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
  let oauthConnecting = $state(false);
  let showApiKeyFallback = $state(false);
  let apiKeyInput = $state("");
  let validationError = $state<string | null>(null);

  // ---- Collections State ----

  let collections = $state<CollectionInfo[]>([]);
  let filtered = $state<CollectionInfo[]>([]);
  let loading = $state(false);
  let loadCollectionsGen = 0; // Generation counter to discard stale results
  let searchQuery = $state("");
  let gameFilter = $state("all");
  let nsfwFilter = $state<"hide" | "show" | "only">("hide");
  let sortField = $state<"endorsements" | "name" | "rating" | "created" | "updated" | "size">("endorsements");
  let sortDirection = $state<"asc" | "desc">("desc");
  let collectionsTotalCount = $state(0);
  let collectionsOffset = $state(0);
  let collectionsPerPage = $state(
    typeof localStorage !== 'undefined'
      ? parseInt(localStorage.getItem('corkscrew-collections-per-page') || '20', 10)
      : 20
  );
  let collectionsSearchTimer: ReturnType<typeof setTimeout> | null = null;
  const collectionsTotalPages = $derived(Math.max(1, Math.ceil(collectionsTotalCount / collectionsPerPage)));
  const collectionsCurrentPage = $derived(Math.floor(collectionsOffset / collectionsPerPage) + 1);

  // Advanced collections filters
  let collectionsAuthorFilter = $state("");
  let collectionsMinDownloads = $state<number | null>(null);
  let collectionsMinEndorsements = $state<number | null>(null);
  let collectionsMinSize = $state<number>(0);
  let collectionsMaxSize = $state<number>(500 * 1024 * 1024 * 1024); // 500 GB default max
  let sizeFilterActive = $state(false);
  let showCollectionsAdvancedFilters = $state(false);
  let collectionsAuthorTimer: ReturnType<typeof setTimeout> | null = null;

  // Download cache percentage per collection (slug → { cached, total })
  let cacheData = $state<Map<string, { cached: number; total: number }>>(new Map());
  let cacheFilter = $state<"all" | "90" | "100">("all");
  let loadingCache = $state(false);

  const collectionsActiveFilterCount = $derived(
    (collectionsAuthorFilter.trim() ? 1 : 0) +
    (collectionsMinDownloads !== null ? 1 : 0) +
    (collectionsMinEndorsements !== null ? 1 : 0) +
    (sizeFilterActive ? 1 : 0) +
    (cacheFilter !== "all" ? 1 : 0)
  );

  function clearAllCollectionsFilters() {
    collectionsAuthorFilter = "";
    collectionsMinDownloads = null;
    collectionsMinEndorsements = null;
    collectionsMinSize = 0;
    collectionsMaxSize = 500 * 1024 * 1024 * 1024;
    sizeFilterActive = false;
    cacheFilter = "all";
    reloadWithSort();
  }

  function collectionsAuthorDebounced() {
    if (collectionsAuthorTimer) clearTimeout(collectionsAuthorTimer);
    collectionsAuthorTimer = setTimeout(() => reloadWithSort(), 400);
  }

  let selectedCollection = $state<CollectionInfo | null>(null);
  let selectedMods = $state<CollectionMod[]>([]);
  let selectedGameVersions = $state<string[]>([]);
  let collectionRevisions = $state<CollectionRevision[]>([]);
  let loadingDetail = $state(false);
  let detailLoadStart = $state(0);
  let detailAbortController: AbortController | null = null;
  let detailCacheInfo = $state<{ cached: number; total: number; nexusTotal: number } | null>(null);
  let installing = $state(false);
  let installResult = $state<{ installed: number; already_installed: number; skipped: number; failed: number; details: { name: string; status: string; error: string | null; url: string | null; instructions: string | null }[] } | null>(null);
  let renderedDescription = $state("");
  let renderedInstallInstructions = $state("");
  let rawInstallInstructions = $state("");
  let userActions = $state<Array<{mod_name: string, action: string, url: string | null, instructions: string | null}>>([]);

  // Pre-grouped install result details to avoid inline .filter() in template
  const installResultInstalled = $derived(installResult?.details.filter(d => d.status === "installed") ?? []);
  const installResultAlreadyInstalled = $derived(installResult?.details.filter(d => d.status === "already_installed") ?? []);
  const installResultUserAction = $derived(installResult?.details.filter(d => d.status === "user_action") ?? []);
  const installResultFailed = $derived(installResult?.details.filter(d => d.status === "failed") ?? []);

  // Floating install button
  let statsBarEl = $state<HTMLElement | null>(null);
  let showFloatingInstall = $state(false);
  let statsBarObserver: IntersectionObserver | null = null;

  // CollectionInstallWizard ref
  let installWizard: CollectionInstallWizard | null = $state(null);

  // WebView toggle state (collections tab only — browse moved to NexusBrowsePanel)
  let collectionsWebviewToggle: WebViewToggle | null = $state(null);
  let collectionsWebviewAnchor: HTMLElement | null = $state(null);
  let collectionsViewMode = $state<"app" | "website">("app");

  // NexusBrowsePanel ref for LLM events
  let nexusBrowsePanel: NexusBrowsePanel | null = $state(null);

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
    hades2: "hades2",
    crimsondesert: "crimsondesert",
    silksong: "hollowknightsilksong",
    riskofrain2: "riskofrain2",
    lethalcompany: "lethalcompany",
    contentwarning: "contentwarning",
    repo: "repo",
    palworld: "palworld",
    valheim: "valheim",
  };

  function getGameSlug(): string {
    const game = $selectedGame;
    if (!game) return "";
    return gameSlugMap[game.game_id] ?? game.game_id;
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

  const gameOptions = $derived.by(() => {
    // Combine games from loaded collections with all detected games
    const gamesSet = new Set(collections.map(c => c.game_domain));
    for (const g of allDetectedGames) {
      const slug = gameSlugMap[g.game_id] ?? g.game_id;
      gamesSet.add(slug);
    }
    return Array.from(gamesSet).sort();
  });

  $effect(() => {
    let result = collections;

    // NSFW filter is applied server-side via the adultContent GraphQL filter.
    // No client-side NSFW filtering needed.

    // Cache filter
    if (cacheFilter !== "all") {
      const threshold = cacheFilter === "100" ? 100 : 90;
      result = result.filter(c => {
        const data = cacheData.get(c.slug);
        if (!data || data.total === 0) return false;
        const pct = Math.round((data.cached / data.total) * 100);
        return pct >= threshold;
      });
    }

    // Size filter (dual-handle range slider)
    if (sizeFilterActive) {
      result = result.filter(c => {
        if (c.download_size == null) return false;
        return c.download_size >= collectionsMinSize && c.download_size <= collectionsMaxSize;
      });
    }

    filtered = result;
  });

  onMount(async () => {
    // Load all detected games for the game selector dropdowns
    try {
      allDetectedGames = await getAllGames();
    } catch (err) {
      console.error("Failed to load games for selector:", err);
    }

    // Listen for LLM "open this mod" events
    window.addEventListener("corkscrew-open-nexus-mod", handleOpenNexusMod);

    await checkAccount();
    // Smart default tab: if user has no installed collections, show Nexus browse tab
    const game = $selectedGame;
    if (game) {
      try {
        const installed = await listInstalledCollections(game.game_id, game.bottle_name);
        if (installed.length === 0) {
          activeTab = "nexus";
        }
      } catch {
        // Silently ignore — fall back to default "my" tab
      }
    }
  });

  // Track when stats bar scrolls out of view for floating install button
  $effect(() => {
    if (statsBarEl) {
      statsBarObserver = new IntersectionObserver(
        ([entry]) => { showFloatingInstall = !entry.isIntersecting; },
        { threshold: 0 }
      );
      statsBarObserver.observe(statsBarEl);
      return () => {
        statsBarObserver?.disconnect();
        statsBarObserver = null;
        showFloatingInstall = false;
      };
    }
  });

  // Listen for LLM-triggered "open this Nexus mod" events
  function handleOpenNexusMod(e: Event) {
    const { mod_id, name } = (e as CustomEvent).detail;
    if (!mod_id) return;
    // Switch to browse_mods tab and open the mod detail
    activeTab = "browse_mods";
    const stub: NexusModInfo = {
      mod_id,
      name: name || `Mod ${mod_id}`,
      summary: "",
      description: null,
      author: "",
      category_id: 0,
      version: "",
      endorsement_count: 0,
      unique_downloads: 0,
      picture_url: null,
      updated_at: null,
      created_at: null,
      available: true,
      adult_content: false,
    };
    nexusBrowsePanel?.openModById(stub);
  }

  onDestroy(() => {
    window.removeEventListener("corkscrew-open-nexus-mod", handleOpenNexusMod);
    statsBarObserver?.disconnect();
    if (collectionsSearchTimer) { clearTimeout(collectionsSearchTimer); collectionsSearchTimer = null; }
    if (collectionsAuthorTimer) { clearTimeout(collectionsAuthorTimer); collectionsAuthorTimer = null; }
    // Close any active webviews when navigating away
    closeBrowserWebview().catch((err) => console.error('closeBrowserWebview:', err));
  });

  async function checkAccount() {
    checkingAuth = true;
    try {
      const status = await getNexusAccountStatus();
      account = status;
      if (status.connected && activeTab === "nexus") {
        const game = $selectedGame;
        const slug = game ? (gameSlugMap[game.game_id] ?? game.game_id) : "skyrimspecialedition";
        collectionsInitializedForGame = slug;
        gameFilter = slug;
        await loadCollections(slug);
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

  /** After successful auth, initialize collections for the current game. */
  async function initCollectionsAfterAuth(status: AccountStatus) {
    account = status;
    showSuccess(`Signed in as ${status.name}`);
    const game = $selectedGame;
    const slug = game ? (gameSlugMap[game.game_id] ?? game.game_id) : "skyrimspecialedition";
    collectionsInitializedForGame = slug;
    gameFilter = slug;
    await loadCollections(slug);
  }

  async function handleOAuthLogin() {
    oauthConnecting = true;
    validationError = null;
    try {
      await startOAuthLogin();
      const status = await getNexusAccountStatus();
      if (status.connected) {
        await initCollectionsAfterAuth(status);
      } else {
        validationError = "Authorization completed but account check failed. Try again.";
      }
    } catch (e: unknown) {
      const msg = typeof e === "string" ? e : (e instanceof Error ? e.message : String(e));
      if (!msg.includes("Cancelled") && !msg.includes("timed out")) {
        validationError = `Sign-in failed: ${msg}`;
      }
    } finally {
      oauthConnecting = false;
    }
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
        apiKeyInput = "";
        await initCollectionsAfterAuth(status);
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
    const gen = ++loadCollectionsGen;
    loading = true;
    if (resetOffset) collectionsOffset = 0;
    try {
      const searchText = searchQuery.trim() || undefined;
      // "size" is client-side only — use "endorsements" as server sort to get consistent results
      const serverSort = sortField === "size" ? "endorsements" : sortField;
      // Pass NSFW filter server-side so pagination reflects the correct count
      const adultContentFilter = nsfwFilter === "hide" ? false : nsfwFilter === "only" ? true : null;
      const browsePromise = browseCollections(
        gameDomain, collectionsPerPage, collectionsOffset,
        serverSort, sortDirection, searchText,
        collectionsAuthorFilter.trim() || undefined,
        collectionsMinDownloads || undefined,
        collectionsMinEndorsements || undefined,
        adultContentFilter,
      );
      const result: CollectionSearchResult = await Promise.race([
        browsePromise,
        new Promise<never>((_, reject) => setTimeout(() => reject(new Error("Request timed out — try again")), 30_000)),
      ]);
      // Discard result if a newer load was started while we were waiting
      if (gen !== loadCollectionsGen) return;
      // Apply client-side size sort if needed
      if (sortField === "size") {
        collections = [...result.collections].sort((a, b) => {
          const aSize = a.download_size ?? 0;
          const bSize = b.download_size ?? 0;
          return sortDirection === "asc" ? aSize - bSize : bSize - aSize;
        });
      } else {
        collections = result.collections;
      }
      collectionsTotalCount = result.total_count;
      // Compute download cache percentages in background
      computeCachePercentages(sortField === "size" ? collections : result.collections);
    } catch (e: unknown) {
      if (gen !== loadCollectionsGen) return;
      // Reset initialized guard so a game switch or tab switch can retry
      collectionsInitializedForGame = null;
      showError(`Failed to load collections: ${e}`);
    } finally {
      // Only clear loading if this is still the latest request
      if (gen === loadCollectionsGen) {
        loading = false;
      }
    }
  }

  /** Fetch mod lists for visible collections and compute cache percentages.
   *  Uses limited concurrency (3 at a time) to avoid NexusMods rate limiting. */
  async function computeCachePercentages(cols: CollectionInfo[]) {
    if (cols.length === 0) return;
    loadingCache = true;
    try {
      // Fetch mod lists with limited concurrency to avoid API rate limits
      const CONCURRENCY = 3;
      const modLists: (CollectionMod[] | null)[] = new Array(cols.length).fill(null);

      const withTimeout = <T>(p: Promise<T>, ms: number): Promise<T> =>
        Promise.race([p, new Promise<T>((_, reject) => setTimeout(() => reject(new Error("timeout")), ms))]);

      for (let i = 0; i < cols.length; i += CONCURRENCY) {
        const batch = cols.slice(i, i + CONCURRENCY);
        const results = await Promise.allSettled(
          batch.map(c => withTimeout(getCollectionMods(c.slug, c.latest_revision), 15_000))
        );
        for (let j = 0; j < results.length; j++) {
          const r = results[j];
          modLists[i + j] = r.status === "fulfilled" ? r.value.mods : null;
        }
      }

      // Build a global set of (mod_id, file_id) pairs + per-collection index
      const allPairs: [number, number][] = [];
      const collectionPairMap = new Map<string, [number, number][]>();

      for (let i = 0; i < cols.length; i++) {
        const mods = modLists[i];
        if (!mods) continue;

        const pairs: [number, number][] = [];
        for (const mod of mods) {
          if (mod.nexus_mod_id != null && mod.nexus_file_id != null) {
            pairs.push([mod.nexus_mod_id, mod.nexus_file_id]);
          }
        }
        collectionPairMap.set(cols[i].slug, pairs);
        allPairs.push(...pairs);
      }

      // Single batch call to backend
      const cachedPairs = allPairs.length > 0 ? await checkCachedFiles(allPairs) : [];
      const cachedSet = new Set(cachedPairs.map(p => `${p[0]}:${p[1]}`));

      // Compute per-collection stats
      const newCacheData = new Map<string, { cached: number; total: number }>();
      for (const [slug, pairs] of collectionPairMap) {
        const cached = pairs.filter(p => cachedSet.has(`${p[0]}:${p[1]}`)).length;
        newCacheData.set(slug, { cached, total: pairs.length });
      }

      cacheData = newCacheData;
    } catch (e) {
      // Cache computation failed — non-critical
    } finally {
      loadingCache = false;
    }
  }

  /** Get the active game domain for collections API calls. */
  function activeGameDomain(): string {
    return gameFilter !== "all" ? gameFilter : "skyrimspecialedition";
  }

  function reloadWithSort() {
    loadCollections(activeGameDomain());
  }

  function collectionsGoToPage(page: number) {
    collectionsOffset = (page - 1) * collectionsPerPage;
    loadCollections(activeGameDomain(), false);
  }

  function setCollectionsPerPage(n: number) {
    collectionsPerPage = n;
    if (typeof localStorage !== 'undefined') {
      localStorage.setItem('corkscrew-collections-per-page', String(n));
    }
    collectionsOffset = 0;
    loadCollections(activeGameDomain(), false);
  }

  function cancelDetailLoad() {
    if (detailAbortController) {
      detailAbortController.abort();
      detailAbortController = null;
    }
    loadingDetail = false;
  }

  async function viewCollectionDetail(collection: CollectionInfo) {
    // Cancel any in-flight load
    if (detailAbortController) detailAbortController.abort();
    const ac = new AbortController();
    detailAbortController = ac;

    loadingDetail = true;
    detailLoadStart = Date.now();
    renderedDescription = "";
    renderedInstallInstructions = "";
    rawInstallInstructions = "";
    detailCacheInfo = null;
    try {
      const detailTimeout = <T>(p: Promise<T>, ms: number, label: string): Promise<T> =>
        Promise.race([p, new Promise<T>((_, reject) => setTimeout(() => reject(new Error(`${label} timed out — NexusMods may be slow, try again`)), ms))]);

      const [detail, modsResult] = await Promise.all([
        detailTimeout(getCollection(collection.slug, collection.game_domain), 15_000, "Collection details"),
        detailTimeout(getCollectionMods(collection.slug, collection.latest_revision), 15_000, "Mod list"),
      ]);

      // Check if cancelled while waiting
      if (ac.signal.aborted) return;

      selectedCollection = detail;
      selectedMods = modsResult.mods;
      selectedGameVersions = modsResult.game_versions;

      // Load revision history (non-blocking)
      getCollectionRevisions(collection.slug)
        .then((revs) => { if (!ac.signal.aborted) collectionRevisions = revs; })
        .catch((e) => console.error("Failed to load revisions:", e));

      // Pre-render the description as markdown
      if (detail.description) {
        const html = await marked.parse(detail.description);
        renderedDescription = DOMPurify.sanitize(html);
      }

      // Store raw + rendered install instructions
      rawInstallInstructions = modsResult.install_instructions ?? "";
      if (modsResult.install_instructions) {
        const html = await marked.parse(modsResult.install_instructions);
        renderedInstallInstructions = DOMPurify.sanitize(html);
      } else {
        renderedInstallInstructions = "";
      }

      // Compute cache percentage for this collection
      computeDetailCacheInfo(modsResult.mods);
    } catch (e: unknown) {
      if (ac.signal.aborted) return;
      showError(`Failed to load collection details: ${e}`);
    } finally {
      if (!ac.signal.aborted) {
        loadingDetail = false;
      }
      if (detailAbortController === ac) detailAbortController = null;
    }
  }

  /** Compute cache info for the collection detail view. */
  async function computeDetailCacheInfo(mods: CollectionMod[]) {
    try {
      const pairs: [number, number][] = [];
      for (const mod of mods) {
        if (mod.nexus_mod_id != null && mod.nexus_file_id != null) {
          pairs.push([mod.nexus_mod_id, mod.nexus_file_id]);
        }
      }
      if (pairs.length === 0) {
        detailCacheInfo = { cached: 0, total: 0, nexusTotal: 0 };
        return;
      }
      const cachedPairs = await checkCachedFiles(pairs);
      const cachedSet = new Set(cachedPairs.map(p => `${p[0]}:${p[1]}`));
      const cached = pairs.filter(p => cachedSet.has(`${p[0]}:${p[1]}`)).length;
      detailCacheInfo = { cached, total: mods.length, nexusTotal: pairs.length };
    } catch (e) {
      // Cache detail computation failed — non-critical
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

  function handleInstallCollection() {
    if (!selectedCollection || !$selectedGame) return;
    installWizard?.startInstall(selectedCollection, selectedMods, selectedGameVersions, renderedInstallInstructions);
  }

  function backToBrowse() {
    selectedCollection = null;
    selectedMods = [];
    collectionRevisions = [];
    renderedDescription = "";
    renderedInstallInstructions = "";
    rawInstallInstructions = "";
  }

  // Logarithmic slider mapping: 0-100 slider → 0 to 500 GB
  // slider=0 maps to 0 bytes; slider=1..100 maps logarithmically from 100 MB to 500 GB
  const SIZE_LOG_FLOOR = 100 * 1024 * 1024;            // 100 MB
  const SIZE_LOG_CEIL = 500 * 1024 * 1024 * 1024;      // 500 GB
  const SIZE_LN_FLOOR = Math.log(SIZE_LOG_FLOOR);
  const SIZE_LN_CEIL = Math.log(SIZE_LOG_CEIL);

  function sizeToSlider(bytes: number): number {
    if (bytes <= 0) return 0;
    if (bytes >= SIZE_LOG_CEIL) return 100;
    if (bytes <= SIZE_LOG_FLOOR) return (bytes / SIZE_LOG_FLOOR) * 1; // 0-1% for sub-100 MB
    return 1 + ((Math.log(bytes) - SIZE_LN_FLOOR) / (SIZE_LN_CEIL - SIZE_LN_FLOOR)) * 99;
  }

  function sliderToSize(pct: number): number {
    if (pct <= 0) return 0;
    if (pct >= 100) return SIZE_LOG_CEIL;
    if (pct <= 1) return Math.round((pct / 1) * SIZE_LOG_FLOOR);
    const logVal = SIZE_LN_FLOOR + ((pct - 1) / 99) * (SIZE_LN_CEIL - SIZE_LN_FLOOR);
    return Math.round(Math.exp(logVal));
  }

  function formatDate(dateStr: string | null | undefined): string {
    if (!dateStr) return "Unknown";
    try {
      const d = new Date(dateStr);
      const now = new Date();
      const diffMs = now.getTime() - d.getTime();
      const diffDays = Math.floor(diffMs / (1000 * 60 * 60 * 24));
      if (diffDays === 0) return "Today";
      if (diffDays === 1) return "Yesterday";
      if (diffDays < 30) return `${diffDays} days ago`;
      if (diffDays < 365) return `${Math.floor(diffDays / 30)} months ago`;
      return `${Math.floor(diffDays / 365)}y ${Math.floor((diffDays % 365) / 30)}m ago`;
    } catch {
      return dateStr;
    }
  }

  function formatDateFull(dateStr: string | null | undefined): string {
    if (!dateStr) return "Unknown";
    try {
      return new Date(dateStr).toLocaleDateString(undefined, {
        year: "numeric",
        month: "short",
        day: "numeric",
      });
    } catch {
      return dateStr;
    }
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
      hogwartslegacy: "Hogwarts Legacy",
      hades2: "Hades II",
      crimsondesert: "Crimson Desert",
      hollowknightsilksong: "Hollow Knight: Silksong",
      riskofrain2: "Risk of Rain 2",
      lethalcompany: "Lethal Company",
      contentwarning: "Content Warning",
      repo: "R.E.P.O.",
      palworld: "Palworld",
      valheim: "Valheim",
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
    <button class="tab-btn" class:tab-active={activeTab === "my"} onclick={() => { closeBrowserWebview().catch((err) => console.error('closeBrowserWebview:', err)); activeTab = "my"; }}>
      My Collections
      {#if myCollections.length > 0}
        <span class="tab-count">{myCollections.length}</span>
      {/if}
    </button>
    <button class="tab-btn" class:tab-active={activeTab === "nexus"} onclick={() => { closeBrowserWebview().catch((err) => console.error('closeBrowserWebview:', err)); activeTab = "nexus"; }}>
      <NexusLogo size={14} />
      Nexus Mods Collections
    </button>
    <button class="tab-btn" class:tab-active={activeTab === "wabbajack"} onclick={() => { closeBrowserWebview().catch((err) => console.error('closeBrowserWebview:', err)); activeTab = "wabbajack"; }}>
      <WabbajackLogo size={14} />
      Wabbajack Lists
    </button>
    <button class="tab-btn" class:tab-active={activeTab === "browse_mods"} onclick={() => { closeBrowserWebview().catch((err) => console.error('closeBrowserWebview:', err)); activeTab = "browse_mods"; }}>
      <NexusLogo size={14} />
      Browse Nexus
    </button>
  </div>

  {#if $selectedGame}
    <InterruptedInstallBanner
      game={$selectedGame}
      onresume={() => {}}
      ondismiss={() => { loadMyCollections(); }}
    />
  {/if}

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
                  <div class="mods-table-body" bind:this={localTableEl} onscroll={handleLocalTableScroll}>
                    <div style="height: {localVisibleRange.paddingTop}px;" aria-hidden="true"></div>
                    {#each localCollectionMods.slice(localVisibleRange.start, localVisibleRange.end) as mod, sliceIdx (mod.id)}
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
                    <div style="height: {localVisibleRange.paddingBottom}px;" aria-hidden="true"></div>
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
              {switchingCollection === selectedMyCollection.name
                ? (deployProgress
                  ? `Deploying... ${Math.round((deployProgress.files_deployed / Math.max(deployProgress.total_files, 1)) * 100)}%`
                  : "Activating...")
                : "Activate Collection"}
            </button>
            <button
              class="btn btn-ghost-danger"
              onclick={() => showDeleteConfirmation(selectedMyCollection!.name)}
            >
              Delete Collection
            </button>
          </div>
        </div>
      </div>

    {:else}
    <!-- My Collections Grid View -->
    <header class="page-header">
      <div class="header-text">
        <h2 class="page-title">My Collections</h2>
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
        <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" style="color: var(--text-quaternary);">
          <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
        </svg>
        <h3 class="empty-heading">No Collections Yet</h3>
        <p class="muted">Browse Nexus Mods Collections to find and install curated mod setups for your games.</p>
        <button class="btn btn-secondary" onclick={() => activeTab = "nexus"}>
          Browse Collections
        </button>
      </div>
    {:else}
      <div class="my-collections-grid">
        {#each myCollections as col, i (col.name + ':' + i)}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div class="my-collection-card" role="button" tabindex="0" onclick={() => viewLocalCollection(col)} onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') viewLocalCollection(col); }} style="animation: glass-fade-in var(--duration-slow) var(--ease) both; animation-delay: {Math.min(i, 15) * 30}ms">
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
              {#if col.original_mod_count && col.mod_count < col.original_mod_count}
                <div class="my-collection-warning">
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="#f59e0b" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
                    <line x1="12" y1="9" x2="12" y2="13" />
                    <line x1="12" y1="17" x2="12.01" y2="17" />
                  </svg>
                  {col.original_mod_count - col.mod_count} mod{col.original_mod_count - col.mod_count !== 1 ? 's' : ''} failed to install
                </div>
              {/if}
              {#if collectionHealth[col.name] && collectionHealth[col.name] !== "loading" && collectionHealth[col.name] !== "error"}
                {@const h = collectionHealth[col.name] as DeploymentHealth}
                <div class="my-collection-health" onclick={(e) => e.stopPropagation()}>
                  {#if h.healthy}
                    <div class="health-status health-ok">
                      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="#22c55e" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
                      Healthy — {h.deployed_files_ok ?? 0} files deployed, {h.staging_ok ?? 0}/{h.enabled_mods ?? 0} mods OK
                    </div>
                  {:else}
                    <div class="health-status health-warn">
                      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="#f59e0b" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/><line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/></svg>
                      Issues found
                    </div>
                    <div class="health-details">
                      {#if (h.staging_missing ?? 0) > 0}<div class="health-issue">Missing staging: {h.staging_missing} mod(s)</div>{/if}
                      {#if (h.staging_empty ?? 0) > 0}<div class="health-issue">Empty staging: {h.staging_empty} mod(s)</div>{/if}
                      {#if (h.deployed_files_missing ?? 0) > 0}<div class="health-issue">Missing deployed files: {h.deployed_files_missing}</div>{/if}
                      {#if (h.hash_mismatches ?? 0) > 0}<div class="health-issue">Hash mismatches: {h.hash_mismatches}</div>{/if}
                      {#if (h.needs_reinstall ?? false)}<div class="health-issue">Needs reinstall — staging data missing</div>{/if}
                      {#if (h.needs_redeploy ?? false)}<div class="health-issue">Needs redeploy — mods staged but not deployed</div>{/if}
                      {#if h.problem_mods && h.problem_mods.length > 0}
                        <div class="health-problem-list">
                          {#each h.problem_mods.slice(0, 10) as pm}
                            <div class="health-problem-mod">{pm.name}: {pm.issue.replace(/_/g, " ")}</div>
                          {/each}
                          {#if h.problem_mods.length > 10}
                            <div class="health-problem-mod">...and {h.problem_mods.length - 10} more</div>
                          {/if}
                        </div>
                      {/if}
                    </div>
                  {/if}
                  <button class="btn-dismiss-health" onclick={() => { collectionHealth = { ...collectionHealth }; delete collectionHealth[col.name]; collectionHealth = collectionHealth; }}>Dismiss</button>
                </div>
              {:else if collectionHealth[col.name] === "error"}
                <div class="my-collection-health" onclick={(e) => e.stopPropagation()}>
                  <div class="health-status health-err">Health check failed</div>
                  <button class="btn-dismiss-health" onclick={() => { delete collectionHealth[col.name]; collectionHealth = { ...collectionHealth }; }}>Dismiss</button>
                </div>
              {/if}
              <div class="my-collection-actions" onclick={(e) => e.stopPropagation()}>
                <button
                  class="btn btn-primary btn-sm"
                  onclick={() => handleSwitchCollection(col.name)}
                  disabled={switchingCollection === col.name}
                >
                  {switchingCollection === col.name
                    ? (deployProgress
                      ? `Deploying ${deployProgress.mod_name.length > 25 ? deployProgress.mod_name.slice(0, 25) + '...' : deployProgress.mod_name} (${deployProgress.files_deployed}/${deployProgress.total_files})`
                      : "Switching...")
                    : "Activate"}
                </button>
                <button
                  class="btn btn-secondary btn-sm"
                  onclick={(e) => { e.stopPropagation(); handleVerifyCollection(col.name); }}
                  disabled={collectionHealth[col.name] === "loading"}
                  title="Verify staging files, deployed files, and file integrity"
                >
                  {collectionHealth[col.name] === "loading"
                    ? (healthCheckProgress
                      ? `${healthCheckProgress.step === 'staging' ? 'Checking staging...' : healthCheckProgress.step === 'deployment' ? `Files ${healthCheckProgress.current}/${healthCheckProgress.total}` : healthCheckProgress.step === 'verification' ? 'Hashing...' : 'Checking...'}`
                      : "Checking...")
                    : "Verify"}
                </button>
                {#if col.original_mod_count && col.mod_count < col.original_mod_count}
                  <button
                    class="btn btn-secondary btn-sm"
                    onclick={(e) => { e.stopPropagation(); handleRepairCollection(col); }}
                    title="Re-download and install failed mods"
                  >
                    Repair
                  </button>
                {/if}
                <button
                  class="btn btn-ghost-danger btn-sm"
                  onclick={(e) => { e.stopPropagation(); showDeleteConfirmation(col.name); }}
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
            class="btn btn-accent btn-step"
            onclick={handleOAuthLogin}
            disabled={oauthConnecting}
            type="button"
          >
            {#if oauthConnecting}
              <span class="spinner-sm"></span>
              Opening browser...
            {:else}
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M15 3h4a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2h-4" />
                <polyline points="10 17 15 12 10 7" />
                <line x1="15" y1="12" x2="3" y2="12" />
              </svg>
              Sign in with Nexus Mods
            {/if}
          </button>
          <button
            class="btn-text"
            onclick={() => { showApiKeyFallback = !showApiKeyFallback; }}
            type="button"
          >
            {showApiKeyFallback ? "Hide API key option" : "Use API key instead"}
          </button>
          {#if showApiKeyFallback}
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
          {/if}
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
            <WineCompatBadge
              kind="collection"
              gameDomain={selectedCollection.game_domain}
              key={selectedCollection.slug}
              compact
            />
          </div>
          <div class="detail-meta-row">
            <p class="detail-author">by {selectedCollection.author}</p>
            <span class="detail-separator">&middot;</span>
            <span class="detail-revision">Revision {selectedCollection.latest_revision}</span>
            {#if selectedCollection.updated_at}
              <span class="detail-separator">&middot;</span>
              <span class="detail-updated" title={formatDateFull(selectedCollection.updated_at)}>
                Updated {formatDate(selectedCollection.updated_at)}
              </span>
            {/if}
          </div>
          <div class="detail-actions-row">
            <button
              class="btn-link"
              onclick={() => selectedCollection && openUrl(`https://next.nexusmods.com/${selectedCollection.game_domain}/collections/${selectedCollection.slug}`)}
              type="button"
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
                <polyline points="15 3 21 3 21 9" />
                <line x1="10" y1="14" x2="21" y2="3" />
              </svg>
              View on Nexus Mods
            </button>
          </div>
        </div>

        <!-- Stats Bar -->
        <div class="detail-stats-bar" bind:this={statsBarEl}>
          <div class="detail-stats-left">
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
            {#if selectedGameVersions.length > 0}
              <div class="detail-stat">
                <span class="detail-stat-value">{selectedGameVersions.join(' / ')}</span>
                <span class="detail-stat-label">{selectedGameVersions.length > 1 ? 'Game Versions' : 'Game Version'}</span>
              </div>
            {/if}
            {#if detailCacheInfo && detailCacheInfo.nexusTotal > 0}
              {@const pct = Math.round((detailCacheInfo.cached / detailCacheInfo.nexusTotal) * 100)}
              <div class="detail-stat">
                <span class="detail-stat-value detail-cache-value" class:cache-full={pct === 100} class:cache-high={pct >= 90 && pct < 100}>
                  {pct}%
                </span>
                <span class="detail-stat-label">Cached ({detailCacheInfo.cached}/{detailCacheInfo.nexusTotal})</span>
              </div>
            {/if}
          </div>
          {#if !installing && !installResult}
            <button
              class="btn btn-primary stats-install-btn"
              onclick={handleInstallCollection}
              disabled={!$selectedGame}
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                <polyline points="7 10 12 15 17 10" />
                <line x1="12" y1="15" x2="12" y2="3" />
              </svg>
              Install
            </button>
          {/if}
        </div>

        <!-- Floating Install Button (appears on scroll) -->
        {#if showFloatingInstall && !installing && !installResult}
          <button
            class="floating-install-btn"
            onclick={handleInstallCollection}
            disabled={!$selectedGame}
            title="Install Collection"
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
              <polyline points="7 10 12 15 17 10" />
              <line x1="12" y1="15" x2="12" y2="3" />
            </svg>
            Install Collection
          </button>
        {/if}

        <!-- Install Instructions (parsed with action checklist) -->
        {#if rawInstallInstructions}
          <div class="detail-section install-instructions-section">
            <InstructionParser
              rawInstructions={rawInstallInstructions}
              modNames={selectedMods.map(m => m.name)}
              gameId={$selectedGame?.game_id ?? ""}
              bottleName={$selectedGame?.bottle_name ?? ""}
              platform="wine"
              gameVersion=""
            />
          </div>
        {/if}

        <!-- Description -->
        {#if renderedDescription}
          <div class="detail-section">
            <h3 class="detail-section-title">Description</h3>
            <div class="rendered-markdown" onclick={handleRenderedLinkClick}>
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
                  {#each selectedMods as mod, i (mod.name + ':' + i)}
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

        <!-- Revision History -->
        {#if collectionRevisions.length > 0}
          <div class="detail-section">
            <h3 class="detail-section-title">
              Revision History
              <span class="title-count">{collectionRevisions.length}</span>
            </h3>
            <div class="revision-list">
              {#each collectionRevisions.slice(0, 10) as rev (rev.revision_number)}
                <div class="revision-entry">
                  <div class="revision-header">
                    <span class="revision-number">Rev. {rev.revision_number}</span>
                    <span class="revision-date" title={formatDateFull(rev.created_at)}>
                      {formatDate(rev.created_at)}
                    </span>
                    <span class="revision-meta">{rev.mod_count} mods &middot; {formatSize(rev.download_size)}</span>
                  </div>
                  {#if rev.changelog}
                    <p class="revision-changelog">{rev.changelog}</p>
                  {/if}
                </div>
              {/each}
              {#if collectionRevisions.length > 10}
                <p class="revision-more">
                  + {collectionRevisions.length - 10} older revisions
                </p>
              {/if}
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
                  {#if $collectionInstallStatus?.phase === "downloading"}
                    Downloading {$collectionInstallStatus.downloadProgress.completed}/{$collectionInstallStatus.downloadProgress.total}
                  {:else if $collectionInstallStatus?.phase === "installing" && $collectionInstallStatus.installProgress.total > 0}
                    Installing mod {$collectionInstallStatus.installProgress.current} of {$collectionInstallStatus.installProgress.total}
                  {:else}
                    Preparing...
                  {/if}
                </span>
                {#if $collectionInstallStatus?.phase === "installing" && $collectionInstallStatus.installProgress.total > 0}
                  <span class="install-progress-pct">
                    {Math.round(($collectionInstallStatus.installProgress.current / $collectionInstallStatus.installProgress.total) * 100)}%
                  </span>
                {/if}
              </div>
              {#if $collectionInstallStatus?.installProgress.currentMod}
                <div class="install-progress-mod">
                  {$collectionInstallStatus.installProgress.currentMod}
                  {#if $collectionInstallStatus.installProgress.step && installStepLabels[$collectionInstallStatus.installProgress.step]}
                    <span class="install-progress-step-inline">{installStepLabels[$collectionInstallStatus.installProgress.step]}</span>
                  {/if}
                </div>
              {:else if $collectionInstallStatus?.installProgress.step && installStepLabels[$collectionInstallStatus.installProgress.step]}
                <div class="install-progress-step">{installStepLabels[$collectionInstallStatus.installProgress.step]}</div>
              {/if}
              {#if $collectionInstallStatus?.total && $collectionInstallStatus.total > 0}
                <div class="install-progress-bar-row">
                  <div class="install-progress-bar">
                    <div
                      class="install-progress-fill"
                      style="width: {($collectionInstallStatus.current / $collectionInstallStatus.total) * 100}%"
                    ></div>
                  </div>
                  <span class="install-progress-elapsed">{$collectionInstallStatus.elapsed}</span>
                </div>
              {/if}
              <button class="btn btn-secondary btn-sm" style="margin-top: 8px;" onclick={() => goto('/collections/progress')}>
                View Details
              </button>
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
                {#each installResultInstalled as detail}
                  <div class="result-mod-row">
                    <svg class="result-mod-icon result-mod-icon--installed" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                      <polyline points="20 6 9 17 4 12" />
                    </svg>
                    <span class="result-mod-name">{detail.name}</span>
                  </div>
                {/each}
                {#each installResultAlreadyInstalled as detail}
                  <div class="result-mod-row">
                    <svg class="result-mod-icon result-mod-icon--existing" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                      <polyline points="20 6 9 17 4 12" />
                    </svg>
                    <span class="result-mod-name">{detail.name}</span>
                    <span class="result-mod-badge result-mod-badge--existing">Already installed</span>
                  </div>
                {/each}
                {#each installResultUserAction as detail}
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
                {#each installResultFailed as detail}
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
    {#if $selectedGame}
      <NexusBrowsePanel
        bind:this={nexusBrowsePanel}
        game={$selectedGame}
        {account}
        {allDetectedGames}
      />
    {:else}
      <div class="empty-state">
        <p class="empty-title">No game selected</p>
        <p class="empty-detail">Select a game from the sidebar to browse mods.</p>
      </div>
    {/if}

  {:else if activeTab === "nexus"}
    <!-- Nexus Mods Collections -->
    <header class="page-header">
      <h2 class="page-title"><NexusLogo size={18} /> Nexus Mods Collections</h2>
      <div class="header-toolbar">
        <WebViewToggle
          bind:this={collectionsWebviewToggle}
          url={`https://next.nexusmods.com/${getGameSlug()}/collections`}
          defaultMode={account?.connected ? "app" : "website"}
          onModeChange={(m) => collectionsViewMode = m}
          anchorEl={collectionsWebviewAnchor}
        />
        {#if account?.connected}
          <div class="toolbar-sep"></div>
          <div class="account-badge">
            <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2" />
              <circle cx="12" cy="7" r="4" />
            </svg>
            <span class="account-name">{account.name}</span>
            {#if account.is_premium}
              <span class="premium-pill">PRO</span>
            {/if}
          </div>
        {/if}
        {#if !loading}
          <div class="toolbar-sep"></div>
          <span class="stat-badge">{filtered.length} {filtered.length === 1 ? "collection" : "collections"}</span>
        {/if}
      </div>
    </header>

    {#if collectionsViewMode === "website"}
      <div class="webview-placeholder" bind:this={collectionsWebviewAnchor}>
        <p class="webview-hint">Browsing NexusMods Collections directly. Switch to "In-App" to use built-in search and filters.</p>
      </div>
    {:else if !account?.connected}
      <div class="premium-gate">
        <div class="premium-gate-icon">
          <NexusLogo size={40} />
        </div>
        <h3 class="premium-gate-title">Connect to NexusMods</h3>
        <p class="premium-gate-desc">Connect your NexusMods account in Settings to browse collections in-app.</p>
        <button class="btn btn-accent" onclick={() => goto("/settings")}>Go to Settings</button>
        <p class="premium-gate-hint">Or switch to "Website" above to browse NexusMods Collections directly.</p>
      </div>
    {:else if loading || loadingDetail}
      <div class="loading-container">
        <div class="loading-card">
          <div class="spinner"><div class="spinner-ring"></div></div>
          <div class="loading-text">
            <p class="loading-title">{loadingDetail ? "Loading collection" : "Fetching collections"}</p>
            <p class="loading-detail">{loadingDetail ? "Fetching from NexusMods API..." : "Loading collections from Nexus Mods..."}</p>
          </div>
          {#if loadingDetail}
            <button class="btn btn-ghost loading-cancel" onclick={cancelDetailLoad}>Cancel</button>
          {/if}
        </div>
      </div>
    {:else}
      <!-- Filters -->
      <SearchFilterBar
        searchPlaceholder="Search collections..."
        bind:searchValue={searchQuery}
        onsearch={() => {
          if (collectionsSearchTimer) clearTimeout(collectionsSearchTimer);
          collectionsSearchTimer = setTimeout(() => reloadWithSort(), 400);
        }}
      >
        {#snippet gameSelector()}
          <select bind:value={gameFilter} onchange={() => { if (gameFilter !== "all") loadCollections(gameFilter); }}>
            <option value="all">All Games</option>
            {#each gameOptions as game}
              <option value={game}>{gameDomainDisplay(game)}</option>
            {/each}
          </select>
        {/snippet}
        {#snippet controls()}
          <select bind:value={sortField} onchange={reloadWithSort}>
            <option value="endorsements">Most Popular</option>
            <option value="name">Name</option>
            <option value="rating">Rating</option>
            <option value="created">Newest</option>
            <option value="updated">Updated</option>
            <option value="size">Size</option>
          </select>
          <button
            onclick={() => { sortDirection = sortDirection === "asc" ? "desc" : "asc"; reloadWithSort(); }}
            title={sortDirection === "asc" ? "Ascending" : "Descending"}
          >
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              {#if sortDirection === "asc"}
                <path d="M12 5v14M5 12l7-7 7 7" />
              {:else}
                <path d="M12 5v14M5 12l7 7 7-7" />
              {/if}
            </svg>
          </button>
          <div class="strip-sep"></div>
          <button
            class:nsfw-show={nsfwFilter === "show"}
            class:nsfw-only={nsfwFilter === "only"}
            onclick={() => { nsfwFilter = cycleNsfwFilter(nsfwFilter); const gd = gameFilter !== "all" ? gameFilter : "skyrimspecialedition"; loadCollections(gd); }}
            title={nsfwFilter === "hide" ? "NSFW hidden" : nsfwFilter === "show" ? "NSFW included" : "NSFW only"}
          >
            <span class="nsfw-indicator">{nsfwIcon(nsfwFilter)}</span>
            {nsfwLabel(nsfwFilter)}
          </button>
          <div class="strip-sep"></div>
          <button onclick={() => showCollectionsAdvancedFilters = !showCollectionsAdvancedFilters}>
            Filters {showCollectionsAdvancedFilters ? '\u25B2' : '\u25BC'}
            {#if collectionsActiveFilterCount > 0}<span class="filter-badge">{collectionsActiveFilterCount}</span>{/if}
          </button>
        {/snippet}
      </SearchFilterBar>

      {#if showCollectionsAdvancedFilters}
        <div class="advanced-filters">
          <div class="filter-section">
            <label class="filter-label">Author</label>
            <input type="text" class="filter-input" placeholder="Filter by author..." bind:value={collectionsAuthorFilter} oninput={collectionsAuthorDebounced} />
          </div>

          <div class="filter-section">
            <label class="filter-label">Min Downloads</label>
            <div class="filter-pills">
              <button class="filter-pill" class:active={collectionsMinDownloads === null} onclick={() => { collectionsMinDownloads = null; reloadWithSort(); }}>Any</button>
              <button class="filter-pill" class:active={collectionsMinDownloads === 1000} onclick={() => { collectionsMinDownloads = 1000; reloadWithSort(); }}>1K+</button>
              <button class="filter-pill" class:active={collectionsMinDownloads === 10000} onclick={() => { collectionsMinDownloads = 10000; reloadWithSort(); }}>10K+</button>
              <button class="filter-pill" class:active={collectionsMinDownloads === 100000} onclick={() => { collectionsMinDownloads = 100000; reloadWithSort(); }}>100K+</button>
            </div>
          </div>

          <div class="filter-section">
            <label class="filter-label">Min Endorsements</label>
            <div class="filter-pills">
              <button class="filter-pill" class:active={collectionsMinEndorsements === null} onclick={() => { collectionsMinEndorsements = null; reloadWithSort(); }}>Any</button>
              <button class="filter-pill" class:active={collectionsMinEndorsements === 100} onclick={() => { collectionsMinEndorsements = 100; reloadWithSort(); }}>100+</button>
              <button class="filter-pill" class:active={collectionsMinEndorsements === 1000} onclick={() => { collectionsMinEndorsements = 1000; reloadWithSort(); }}>1K+</button>
              <button class="filter-pill" class:active={collectionsMinEndorsements === 10000} onclick={() => { collectionsMinEndorsements = 10000; reloadWithSort(); }}>10K+</button>
            </div>
          </div>

          <div class="filter-section">
            <label class="filter-label">Download Cache {#if loadingCache}<span class="spinner-xs"></span>{/if}</label>
            <div class="filter-pills">
              <button class="filter-pill" class:active={cacheFilter === "all"} onclick={() => { cacheFilter = "all"; }}>All</button>
              <button class="filter-pill" class:active={cacheFilter === "90"} onclick={() => { cacheFilter = "90"; }}>90%+ Cached</button>
              <button class="filter-pill" class:active={cacheFilter === "100"} onclick={() => { cacheFilter = "100"; }}>100% Cached</button>
            </div>
          </div>

          <div class="filter-section size-range-filter">
            <label class="filter-label">Install Size Range</label>
            <div class="size-range-labels">
              <span class="size-range-value">{formatSize(collectionsMinSize)}</span>
              <span class="size-range-dash">—</span>
              <span class="size-range-value">{formatSize(collectionsMaxSize)}</span>
            </div>
            <div class="size-range-slider">
              <div class="range-track">
                <div
                  class="range-fill"
                  style="left: {(sizeToSlider(collectionsMinSize) / 100) * 100}%; right: {100 - (sizeToSlider(collectionsMaxSize) / 100) * 100}%"
                ></div>
              </div>
              <input
                type="range"
                class="range-input range-min"
                min="0"
                max="100"
                step="0.5"
                value={sizeToSlider(collectionsMinSize)}
                oninput={(e) => {
                  const val = parseFloat(e.currentTarget.value);
                  const maxSlider = sizeToSlider(collectionsMaxSize);
                  if (val < maxSlider) {
                    collectionsMinSize = sliderToSize(val);
                    sizeFilterActive = true;
                  }
                }}
              />
              <input
                type="range"
                class="range-input range-max"
                min="0"
                max="100"
                step="0.5"
                value={sizeToSlider(collectionsMaxSize)}
                oninput={(e) => {
                  const val = parseFloat(e.currentTarget.value);
                  const minSlider = sizeToSlider(collectionsMinSize);
                  if (val > minSlider) {
                    collectionsMaxSize = sliderToSize(val);
                    sizeFilterActive = true;
                  }
                }}
              />
            </div>
            <div class="size-range-presets">
              <button class="filter-pill" class:active={!sizeFilterActive} onclick={() => { collectionsMinSize = 0; collectionsMaxSize = 500 * 1024 * 1024 * 1024; sizeFilterActive = false; }}>Any</button>
              <button class="filter-pill" onclick={() => { collectionsMinSize = 0; collectionsMaxSize = 10 * 1024 * 1024 * 1024; sizeFilterActive = true; }}>{"< 10 GB"}</button>
              <button class="filter-pill" onclick={() => { collectionsMinSize = 0; collectionsMaxSize = 50 * 1024 * 1024 * 1024; sizeFilterActive = true; }}>{"< 50 GB"}</button>
              <button class="filter-pill" onclick={() => { collectionsMinSize = 50 * 1024 * 1024 * 1024; collectionsMaxSize = 500 * 1024 * 1024 * 1024; sizeFilterActive = true; }}>50+ GB</button>
            </div>
          </div>
        </div>
      {/if}

      {#if collectionsActiveFilterCount > 0}
        <div class="active-filters">
          {#if collectionsAuthorFilter.trim()}
            <span class="filter-chip">
              Author: {collectionsAuthorFilter}
              <button onclick={() => { collectionsAuthorFilter = ""; reloadWithSort(); }}>&times;</button>
            </span>
          {/if}
          {#if collectionsMinDownloads !== null}
            <span class="filter-chip">
              Downloads: {formatNumber(collectionsMinDownloads)}+
              <button onclick={() => { collectionsMinDownloads = null; reloadWithSort(); }}>&times;</button>
            </span>
          {/if}
          {#if collectionsMinEndorsements !== null}
            <span class="filter-chip">
              Endorsements: {formatNumber(collectionsMinEndorsements)}+
              <button onclick={() => { collectionsMinEndorsements = null; reloadWithSort(); }}>&times;</button>
            </span>
          {/if}
          {#if cacheFilter !== "all"}
            <span class="filter-chip">
              Cache: {cacheFilter === "100" ? "100%" : "90%+"}
              <button onclick={() => { cacheFilter = "all"; }}>&times;</button>
            </span>
          {/if}
          {#if sizeFilterActive}
            <span class="filter-chip">
              Size: {formatSize(collectionsMinSize)} — {formatSize(collectionsMaxSize)}
              <button onclick={() => { collectionsMinSize = 0; collectionsMaxSize = 500 * 1024 * 1024 * 1024; sizeFilterActive = false; }}>&times;</button>
            </span>
          {/if}
          <button class="filter-chip filter-chip-clear" onclick={clearAllCollectionsFilters}>
            Clear All &times;
          </button>
        </div>
      {/if}

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
              style="animation-delay: {Math.min(i, 15) * 30}ms"
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
                  <WineCompatBadge
                    kind="collection"
                    gameDomain={collection.game_domain}
                    key={collection.slug}
                    compact
                  />
                  <span class="revision-badge">Rev {collection.latest_revision}</span>
                  {#if collection.download_size}
                    <span class="size-badge" class:size-small={collection.download_size < 5 * 1024 * 1024 * 1024}
                      class:size-medium={collection.download_size >= 5 * 1024 * 1024 * 1024 && collection.download_size < 20 * 1024 * 1024 * 1024}
                      class:size-large={collection.download_size >= 20 * 1024 * 1024 * 1024 && collection.download_size < 50 * 1024 * 1024 * 1024}
                      class:size-huge={collection.download_size >= 50 * 1024 * 1024 * 1024}>
                      <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                        <polyline points="7 10 12 15 17 10" />
                        <line x1="12" y1="15" x2="12" y2="3" />
                      </svg>
                      {formatSize(collection.download_size)}
                    </span>
                  {:else}
                    <span class="size-badge size-unknown">Size unknown</span>
                  {/if}
                </div>

                <h3 class="card-title">{collection.name}</h3>
                <p class="card-author">
                  by {collection.author}
                  {#if collection.updated_at}
                    <span class="card-updated" title={formatDateFull(collection.updated_at)}>
                      &middot; Updated {formatDate(collection.updated_at)}
                    </span>
                  {/if}
                </p>

                {#if collection.summary}
                  <p class="card-desc">{collection.summary}</p>
                {/if}

                {#if collection.tags.length > 0}
                  <div class="card-tags">
                    {#each collection.tags.slice(0, 3) as tag}
                      <span class="tag">{tag}</span>
                    {/each}
                    {#if collection.tags.length > 3}
                      <span class="tag tag-overflow">+{collection.tags.length - 3}</span>
                    {/if}
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
                </div>

                {#if cacheData.has(collection.slug)}
                  {@const cd = cacheData.get(collection.slug)}
                  {#if cd && cd.total > 0}
                    {@const pct = Math.round((cd.cached / cd.total) * 100)}
                    <div class="cache-badge" class:cache-full={pct === 100} class:cache-high={pct >= 90 && pct < 100}>
                      <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                        {#if pct === 100}
                          <polyline points="20 6 9 17 4 12" />
                        {:else}
                          <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                          <polyline points="7 10 12 15 17 10" />
                          <line x1="12" y1="15" x2="12" y2="3" />
                        {/if}
                      </svg>
                      {pct}% cached
                    </div>
                  {/if}
                {/if}

                <div class="card-actions">
                  <button
                    class="btn btn-view-details btn-sm"
                    onclick={() => viewCollectionDetail(collection)}
                  >
                    View Details
                  </button>
                </div>
              </div>
            </div>
          {/each}
        </div>

        <!-- Per-page selector + Pagination -->
        <div class="per-page-selector">
          <span class="per-page-label">Per page:</span>
          {#each [12, 20, 40, 60] as n}
            <button
              class="per-page-btn"
              class:active={collectionsPerPage === n}
              onclick={() => setCollectionsPerPage(n)}
            >{n}</button>
          {/each}
        </div>
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

<!-- Collection Install Wizard (modals for tools, optional picker, version mismatch, DLC, cleanup) -->
{#if $selectedGame}
  <CollectionInstallWizard
    bind:this={installWizard}
    game={$selectedGame}
    oncomplete={(result) => {
      installResult = result;
    }}
  />
{/if}

<!-- Uninstall Progress Modal -->
{#if $collectionUninstallStatus?.active}
  {@const us = $collectionUninstallStatus}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="uninstall-overlay" onclick={(e) => { if (us.phase === "complete") dismissUninstall(); }}>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div class="uninstall-modal" onclick={(e) => e.stopPropagation()}>
      {#if us.phase === "complete"}
        <!-- Completion State -->
        <div class="uninstall-complete">
          <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="#22c55e" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14" />
            <polyline points="22 4 12 14.01 9 11.01" />
          </svg>
          <h3 class="uninstall-title">Collection Removed</h3>
          <p class="uninstall-subtitle">"{us.collectionName}" has been uninstalled</p>
          <div class="uninstall-result-chips">
            <span class="result-chip result-success">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="20 6 9 17 4 12" /></svg>
              {us.result?.modsRemoved ?? 0} mods removed
            </span>
            {#if (us.result?.downloadsRemoved ?? 0) > 0}
              <span class="result-chip result-neutral">
                {us.result?.downloadsRemoved} downloads cleaned
              </span>
            {/if}
            {#if us.failed > 0}
              <span class="result-chip result-error">
                {us.failed} errors
              </span>
            {/if}
          </div>
          {#if us.errors.length > 0}
            <div class="uninstall-errors">
              {#each us.errors.slice(0, 5) as err}
                <p class="uninstall-error-line">{err}</p>
              {/each}
              {#if us.errors.length > 5}
                <p class="uninstall-error-line">...and {us.errors.length - 5} more</p>
              {/if}
            </div>
          {/if}
          <button class="btn btn-primary" onclick={dismissUninstall}>Done</button>
        </div>
      {:else}
        <!-- In-Progress State -->
        <div class="uninstall-progress">
          <h3 class="uninstall-title">
            {us.phase === "redeploying" ? "Redeploying Remaining Mods" : `Removing "${us.collectionName}"`}
          </h3>

          {#if us.phase === "removing" && us.totalMods > 0}
            <div class="uninstall-bar-header">
              <span class="uninstall-bar-label">{us.currentMod} / {us.totalMods}</span>
              <span class="uninstall-bar-percent">{Math.round((us.currentMod / us.totalMods) * 100)}%</span>
            </div>
            <div class="uninstall-track">
              <div class="uninstall-fill uninstall-fill-active" style="width: {(us.currentMod / us.totalMods) * 100}%"></div>
            </div>
          {:else if us.phase === "redeploying"}
            <div class="uninstall-track">
              <div class="uninstall-fill uninstall-fill-active uninstall-fill-indeterminate"></div>
            </div>
          {/if}

          {#if us.currentModName}
            <div class="uninstall-current">
              <span class="uninstall-mod-name">{us.currentModName}</span>
              <span class="uninstall-step">{humanizeUninstallStep(us.currentStep)}</span>
            </div>
          {/if}

          {#if us.failed > 0}
            <div class="uninstall-fail-badge">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
              {us.failed} error{us.failed > 1 ? "s" : ""}
            </div>
          {/if}
        </div>
      {/if}
    </div>
  </div>
{/if}

<!-- Delete Confirmation Modal -->
{#if confirmDeleteCollection && $selectedGame}
  <CollectionDeleteDialog
    collectionName={confirmDeleteCollection}
    game={$selectedGame}
    ondelete={async () => { confirmDeleteCollection = null; await loadMyCollections(); }}
    oncancel={() => { confirmDeleteCollection = null; }}
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

  .btn-text {
    background: none;
    border: none;
    color: var(--text-secondary);
    font-size: 12px;
    cursor: pointer;
    padding: 4px 0;
    text-decoration: underline;
    text-decoration-color: transparent;
    transition: text-decoration-color 0.15s;
  }
  .btn-text:hover {
    text-decoration-color: var(--text-secondary);
  }

  /* ---- Header ---- */

  .page-header {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    margin-top: 20px;
    margin-bottom: var(--space-4);
    flex-wrap: wrap;
    row-gap: var(--space-2);
  }

  .page-title {
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

  .page-subtitle {
    font-size: 13px;
    color: var(--text-quaternary);
    white-space: nowrap;
  }

  .header-toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-left: auto;
    font-size: 12px;
    color: var(--text-tertiary);
    background: color-mix(in srgb, var(--surface-subtle) 60%, transparent);
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
    border: 1px solid color-mix(in srgb, var(--separator) 30%, transparent);
    border-radius: 8px;
    padding: 4px 10px;
  }

  .toolbar-sep {
    width: 1px;
    height: 14px;
    background: color-mix(in srgb, var(--separator) 50%, transparent);
    flex-shrink: 0;
  }

  .account-badge {
    display: flex;
    align-items: center;
    gap: 5px;
    color: var(--text-secondary);
  }

  .account-badge svg {
    opacity: 0.5;
    flex-shrink: 0;
  }

  .account-name {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
  }

  .premium-pill {
    font-size: 11px;
    font-weight: 700;
    color: #ff9f0a;
    background: rgba(255, 159, 10, 0.15);
    padding: 1px 5px;
    border-radius: 100px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }

  .stat-badge {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
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

  .loading-cancel {
    margin-top: var(--space-2);
    font-size: 13px;
    color: var(--text-secondary);
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
    border: 2px solid var(--text-tertiary);
    border-top-color: var(--text-primary);
    border-radius: 50%;
    animation: spin 0.75s linear infinite;
    flex-shrink: 0;
  }

  @keyframes spin { to { transform: rotate(360deg); } }
  .icon-spin { animation: spin 1.5s linear infinite; }

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
    gap: var(--space-2);
    margin-bottom: var(--space-6);
    flex-wrap: wrap;
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
    background: var(--bg-tertiary);
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

  .filters-right {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-left: auto;
  }

  .sort-group {
    display: flex;
    align-items: center;
    gap: 2px;
  }

  .sort-direction-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border-radius: var(--radius-sm);
    background: var(--surface);
    border: 1px solid var(--separator);
    color: var(--text-secondary);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
    opacity: 0.75;
  }

  .sort-direction-btn:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
    opacity: 1;
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
    box-shadow: var(--glass-refraction), var(--glass-edge-shadow);
    transition: border-color var(--duration-fast) var(--ease),
                box-shadow var(--duration-fast) var(--ease),
                transform var(--duration-fast) var(--ease);
    animation: glass-fade-in var(--duration-slow) var(--ease) both;
    display: flex;
    flex-direction: column;
  }

  .collection-card:hover {
    border-color: var(--accent);
    box-shadow: var(--glass-refraction), var(--glass-edge-shadow), 0 4px 12px rgba(0, 0, 0, 0.15);
    transform: translateY(-2px) rotate(-0.3deg);
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

  .size-badge {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 10px;
    font-weight: 600;
    padding: 2px 6px;
    border-radius: 4px;
    font-variant-numeric: tabular-nums;
  }

  .size-small {
    background: color-mix(in srgb, #34C759 15%, transparent);
    color: #34C759;
  }

  .size-medium {
    background: color-mix(in srgb, #FFD60A 15%, transparent);
    color: #FFD60A;
  }

  .size-large {
    background: color-mix(in srgb, #FF9F0A 15%, transparent);
    color: #FF9F0A;
  }

  .size-huge {
    background: color-mix(in srgb, #FF3B30 15%, transparent);
    color: #FF3B30;
  }

  .size-unknown {
    background: color-mix(in srgb, var(--text-tertiary) 10%, transparent);
    color: var(--text-tertiary);
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

  .card-updated {
    color: var(--text-tertiary);
    font-size: 11px;
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
    background: var(--surface);
    padding: 1px 6px;
    border-radius: var(--radius-sm);
  }

  .tag-overflow {
    color: var(--text-tertiary);
    font-style: italic;
    opacity: 0.7;
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

  .cache-badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-tertiary);
    padding: 3px 8px;
    border-radius: 10px;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid var(--separator);
    margin-bottom: var(--space-2);
  }

  .cache-badge.cache-full {
    color: var(--green, #30d158);
    background: rgba(48, 209, 88, 0.1);
    border-color: rgba(48, 209, 88, 0.25);
  }

  .cache-badge.cache-high {
    color: var(--accent, #d98f40);
    background: rgba(217, 143, 64, 0.1);
    border-color: rgba(217, 143, 64, 0.25);
  }

  .detail-cache-value.cache-full {
    color: var(--green, #30d158);
  }
  .detail-cache-value.cache-high {
    color: var(--accent, #d98f40);
  }

  .card-actions {
    display: flex;
    gap: var(--space-2);
    margin-top: auto;
    padding-top: var(--space-2);
  }

  /* ---- Buttons ---- */

  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 8px 16px;
    border: none;
    border-radius: var(--radius);
    font-size: 13px;
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
  }

  .btn-accent:hover:not(:disabled) {
    filter: brightness(1.1);
    box-shadow: 0 1px 6px rgba(0, 122, 255, 0.25);
  }

  /* .btn-view-details inherited from global app.css */

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
    border: 1px dashed var(--separator);
    border-radius: var(--radius-lg);
    background: var(--surface-subtle);
    box-shadow: var(--glass-refraction), var(--glass-edge-shadow);
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
    gap: var(--space-3);
  }

  .mod-detail-hero {
    width: 100%;
    height: 280px;
    background-size: cover;
    background-position: center;
    border-radius: var(--radius-lg);
    border: 1px solid var(--border-primary);
  }

  .detail-summary {
    color: var(--text-secondary);
    font-size: 13px;
    line-height: 1.5;
    margin-top: var(--space-2);
  }

  .badge-success {
    background: var(--color-green, #30d158);
    color: #000;
    padding: 4px 12px;
    border-radius: var(--radius);
    font-size: 12px;
    font-weight: 600;
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
    margin: 0;
  }

  .detail-meta-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .detail-separator {
    color: var(--text-tertiary);
    font-size: 12px;
  }

  .detail-revision {
    font-size: 12px;
    color: var(--text-tertiary);
    font-weight: 500;
  }

  .detail-updated {
    font-size: 12px;
    color: var(--text-tertiary);
    cursor: default;
  }

  .detail-actions-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: 4px;
  }

  .btn-link {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    background: none;
    border: none;
    color: var(--accent);
    font-size: 12px;
    cursor: pointer;
    padding: 2px 0;
    text-decoration: none;
    transition: opacity 0.15s;
  }
  .btn-link:hover {
    opacity: 0.8;
    text-decoration: underline;
  }

  .revision-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .revision-entry {
    padding: var(--space-2) var(--space-3);
    background: var(--bg-secondary);
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
  }

  .revision-header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .revision-number {
    font-weight: 600;
    font-size: 13px;
    color: var(--text-primary);
  }

  .revision-date {
    font-size: 12px;
    color: var(--text-tertiary);
    cursor: default;
  }

  .revision-meta {
    font-size: 12px;
    color: var(--text-tertiary);
  }

  .revision-changelog {
    font-size: 12px;
    color: var(--text-secondary);
    margin: 4px 0 0;
    line-height: 1.4;
  }

  .revision-more {
    font-size: 12px;
    color: var(--text-tertiary);
    text-align: center;
    padding: var(--space-1) 0;
  }


  .detail-stats-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    padding: var(--space-4) var(--space-5);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-lg);
  }

  .detail-stats-left {
    display: flex;
    gap: var(--space-6);
  }

  .stats-install-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 8px 18px;
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
    flex-shrink: 0;
  }

  .floating-install-btn {
    position: fixed;
    bottom: 28px;
    right: 28px;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 10px 20px;
    background: var(--system-accent);
    color: var(--system-accent-on);
    font-size: 13px;
    font-weight: 600;
    border: none;
    border-radius: var(--radius-lg);
    cursor: pointer;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.3), 0 1px 4px rgba(0, 0, 0, 0.15);
    z-index: 50;
    animation: floatIn 0.2s ease-out;
    transition: background 0.15s ease, transform 0.15s ease;
  }

  .floating-install-btn:hover:not(:disabled) {
    background: var(--system-accent-hover);
    transform: translateY(-1px);
  }

  .floating-install-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  @keyframes floatIn {
    from { opacity: 0; transform: translateY(8px); }
    to { opacity: 1; transform: translateY(0); }
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

  .install-instructions-section {
    background: color-mix(in srgb, var(--accent-blue) 8%, transparent);
    border: 1px solid color-mix(in srgb, var(--accent-blue) 25%, transparent);
    border-radius: var(--radius-md);
    padding: var(--space-3);
  }

  .install-instructions-section .detail-section-title {
    color: var(--accent-blue);
  }

  .install-instructions-content {
    font-size: 13px;
    line-height: 1.6;
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
    box-shadow: var(--glass-refraction), var(--glass-edge-shadow);
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
    background: var(--surface-subtle);
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
    margin-top: 0;
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
    background: var(--surface-active);
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

  .my-collections-empty .empty-heading {
    font-size: 18px;
    font-weight: 600;
    color: var(--text-primary);
    margin: 0;
  }

  .my-collections-empty .muted {
    color: var(--text-tertiary);
    font-size: 13px;
    max-width: 360px;
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
    transition: background var(--duration-fast) var(--ease), border-color var(--duration-fast) var(--ease), transform var(--duration-fast) var(--ease);
  }

  .my-collection-card:hover {
    background: var(--surface-hover);
    border-color: var(--accent-muted);
    transform: translateY(-2px) rotate(-0.3deg);
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

  .my-collection-warning {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    font-size: 11px;
    font-weight: 500;
    color: #f59e0b;
    margin-top: var(--space-1);
  }

  .my-collection-health {
    margin-top: var(--space-2);
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--bg-tertiary);
    font-size: 11px;
  }
  .health-status {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    font-weight: 600;
  }
  .health-ok { color: #22c55e; }
  .health-warn { color: #f59e0b; }
  .health-err { color: var(--red); }
  .health-details {
    margin-top: var(--space-1);
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .health-issue {
    color: var(--text-secondary);
    padding-left: 16px;
  }
  .health-problem-list {
    margin-top: 2px;
    padding-left: 16px;
    max-height: 80px;
    overflow-y: auto;
  }
  .health-problem-mod {
    color: var(--text-tertiary);
    font-size: 10px;
  }
  .btn-dismiss-health {
    margin-top: var(--space-1);
    background: none;
    border: none;
    color: var(--text-tertiary);
    font-size: 10px;
    cursor: pointer;
    padding: 0;
    text-decoration: underline;
  }
  .btn-dismiss-health:hover { color: var(--text-secondary); }

  .my-collection-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }

  /* ---- Delete Confirmation Modal ---- */

  .modal-dialog {
    background: color-mix(in srgb, var(--bg-grouped) 75%, transparent);
    backdrop-filter: var(--glass-blur-heavy);
    -webkit-backdrop-filter: var(--glass-blur-heavy);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: var(--radius-lg, 12px);
    width: min(440px, 90vw);
    padding: var(--space-6);
    box-shadow: var(--glass-refraction),
                var(--glass-edge-shadow),
                0 8px 32px rgba(0, 0, 0, 0.4);
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: var(--space-4);
  }

  .modal-icon {
    flex-shrink: 0;
  }

  .modal-title {
    font-size: 18px;
    font-weight: 700;
    color: var(--text-primary);
    margin: 0;
    letter-spacing: -0.02em;
  }

  .modal-desc {
    font-size: 13px;
    color: var(--text-secondary);
    line-height: 1.5;
    margin: 0;
    max-width: 360px;
  }

  .modal-option {
    width: 100%;
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    padding: var(--space-3) var(--space-4);
    text-align: left;
  }

  .modal-checkbox-label {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    cursor: pointer;
    font-size: 13px;
    font-weight: 500;
    color: var(--text-primary);
  }

  .modal-checkbox-label input {
    accent-color: var(--system-accent);
    width: 16px;
    height: 16px;
    flex-shrink: 0;
  }

  .modal-checkbox-text {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .modal-size-badge {
    font-size: 11px;
    font-weight: 600;
    color: #22c55e;
    background: rgba(34, 197, 94, 0.12);
    padding: 1px 8px;
    border-radius: 100px;
    font-family: var(--font-mono);
  }

  .modal-size-loading {
    font-size: 11px;
    color: var(--text-tertiary);
    font-style: italic;
  }

  .modal-size-note {
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .modal-option-hint {
    font-size: 11px;
    color: var(--text-tertiary);
    margin: var(--space-2) 0 0 24px;
    line-height: 1.4;
  }

  .modal-option-hint-warn {
    color: #f59e0b;
  }

  .modal-actions {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    justify-content: center;
  }

  .modal-actions .btn-danger {
    padding: var(--space-2) var(--space-5);
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


  .per-page-selector {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    margin-bottom: var(--space-3);
  }

  .per-page-label {
    font-size: 11px;
    color: var(--text-tertiary);
    margin-right: var(--space-1);
  }

  .per-page-btn {
    font-size: 11px;
    padding: 3px 8px;
    border-radius: 4px;
    border: 1px solid var(--border-primary, rgba(255,255,255,0.08));
    background: transparent;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all 0.15s;
  }

  .per-page-btn:hover {
    background: var(--surface-hover);
  }

  .per-page-btn.active {
    background: var(--system-accent, #007AFF);
    color: white;
    border-color: var(--system-accent, #007AFF);
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

  /* ---- Advanced Filters ---- */

  .filter-toggle {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    color: var(--text-secondary);
    font-size: 12px;
    cursor: pointer;
    transition: background 0.15s, color 0.15s;
    white-space: nowrap;
  }

  .filter-toggle:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .filter-badge {
    background: var(--system-accent);
    color: var(--system-accent-on);
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 10px;
    font-weight: 600;
  }

  .advanced-filters {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-4);
    padding: var(--space-3) var(--space-4);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    margin-bottom: var(--space-3);
  }

  .filter-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 140px;
  }

  .filter-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-tertiary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .filter-input {
    padding: var(--space-1) var(--space-2);
    background: var(--bg-tertiary);
    border: 1px solid var(--separator);
    border-radius: var(--radius-sm);
    color: var(--text-primary);
    font-size: 12px;
    outline: none;
    min-width: 160px;
    font-family: var(--font-sans);
  }

  .filter-input:focus {
    border-color: var(--system-accent);
  }

  .filter-input::placeholder {
    color: var(--text-tertiary);
  }

  .filter-pills {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
  }

  .filter-pill {
    padding: 3px 10px;
    background: var(--bg-tertiary);
    border: 1px solid var(--separator);
    border-radius: 12px;
    color: var(--text-secondary);
    font-size: 11px;
    cursor: pointer;
    transition: all 0.15s;
    white-space: nowrap;
  }

  .filter-pill:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .filter-pill.active {
    background: var(--system-accent-subtle);
    border-color: var(--system-accent);
    color: var(--system-accent);
    font-weight: 500;
  }

  /* Dual-handle range slider */
  .size-range-filter {
    min-width: 260px;
  }

  .size-range-labels {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    font-weight: 500;
    color: var(--text-primary);
  }

  .size-range-value {
    background: var(--bg-tertiary);
    padding: 2px 8px;
    border-radius: var(--radius-sm);
    font-variant-numeric: tabular-nums;
    font-size: 11px;
  }

  .size-range-dash {
    color: var(--text-tertiary);
  }

  .size-range-slider {
    position: relative;
    height: 28px;
    display: flex;
    align-items: center;
  }

  .range-track {
    position: absolute;
    width: 100%;
    height: 4px;
    background: var(--bg-tertiary);
    border-radius: 2px;
  }

  .range-fill {
    position: absolute;
    height: 100%;
    background: var(--system-accent);
    border-radius: 2px;
  }

  .range-input {
    position: absolute;
    width: 100%;
    height: 4px;
    appearance: none;
    -webkit-appearance: none;
    background: transparent;
    pointer-events: none;
    margin: 0;
  }

  .range-input::-webkit-slider-thumb {
    -webkit-appearance: none;
    height: 16px;
    width: 16px;
    border-radius: 50%;
    background: var(--system-accent);
    border: 2px solid var(--surface);
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
    cursor: pointer;
    pointer-events: all;
    position: relative;
    z-index: 1;
  }

  .range-input::-moz-range-thumb {
    height: 16px;
    width: 16px;
    border-radius: 50%;
    background: var(--system-accent);
    border: 2px solid var(--surface);
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
    cursor: pointer;
    pointer-events: all;
  }

  .range-input::-webkit-slider-thumb:hover {
    transform: scale(1.15);
  }

  .range-input::-moz-range-thumb:hover {
    transform: scale(1.15);
  }

  .size-range-presets {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
  }

  .active-filters {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    margin-bottom: var(--space-3);
  }

  .filter-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    background: var(--system-accent-subtle);
    border: 1px solid color-mix(in srgb, var(--system-accent) 30%, transparent);
    border-radius: 10px;
    color: var(--system-accent);
    font-size: 11px;
    font-weight: 500;
  }

  .filter-chip button {
    display: flex;
    align-items: center;
    background: none;
    border: none;
    color: inherit;
    cursor: pointer;
    padding: 0;
    opacity: 0.7;
    font-size: 13px;
    line-height: 1;
  }

  .filter-chip button:hover {
    opacity: 1;
  }

  .filter-chip-clear {
    background: var(--surface-hover);
    border-color: var(--separator);
    color: var(--text-secondary);
    cursor: pointer;
    font-weight: 500;
  }

  .filter-chip-clear:hover {
    background: var(--surface-active);
    color: var(--text-primary);
  }

  /* ---- Premium Gate ---- */

  .premium-gate {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: var(--space-4);
    padding: var(--space-16) var(--space-10);
    max-width: 480px;
    margin: 0 auto;
  }

  .premium-gate-icon {
    color: var(--text-quaternary);
    margin-bottom: var(--space-2);
  }

  .premium-gate-title {
    font-size: 20px;
    font-weight: 700;
    letter-spacing: -0.02em;
    color: var(--text-primary);
  }

  .premium-gate-desc {
    font-size: 14px;
    color: var(--text-secondary);
    line-height: 1.6;
  }

  .premium-gate-hint {
    font-size: 12px;
    color: var(--text-tertiary);
    margin-top: var(--space-2);
  }

  /* ---- Webview Placeholder ---- */

  .webview-placeholder {
    display: flex;
    align-items: center;
    justify-content: center;
    flex: 1;
    min-height: calc(100vh - 200px);
    padding: var(--space-8);
  }

  .webview-hint {
    font-size: 13px;
    color: var(--text-tertiary);
    text-align: center;
  }

  /* ---- NSFW 3-State Toggle ---- */

  .nsfw-cycle-btn {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    padding: 2px var(--space-2);
    background: var(--bg-tertiary);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    color: var(--text-secondary);
    font-size: 11px;
    font-weight: 500;
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
    white-space: nowrap;
    opacity: 0.75;
  }

  .nsfw-cycle-btn:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
    opacity: 1;
  }

  .nsfw-cycle-btn.nsfw-show {
    background: rgba(255, 159, 10, 0.1);
    border-color: rgba(255, 159, 10, 0.3);
    color: #ff9f0a;
  }

  .nsfw-cycle-btn.nsfw-only {
    background: rgba(255, 69, 58, 0.1);
    border-color: rgba(255, 69, 58, 0.3);
    color: #ff453a;
  }

  .nsfw-indicator {
    font-size: 11px;
    font-weight: 700;
    width: 14px;
    text-align: center;
  }

  /* ---- Download Button on Mod Cards ---- */

  .modal-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
  }

  /* ---- Uninstall Progress Modal ---- */

  .uninstall-overlay {
    position: fixed;
    inset: 0;
    z-index: 1000;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    animation: uninstall-fade-in 0.2s ease;
  }

  @keyframes uninstall-fade-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .uninstall-modal {
    background: var(--bg-secondary);
    border: 1px solid var(--separator);
    border-radius: 12px;
    padding: var(--space-8);
    width: 440px;
    max-width: 90vw;
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.4);
    animation: uninstall-modal-in 0.25s ease;
  }

  @keyframes uninstall-modal-in {
    from { opacity: 0; transform: scale(0.95) translateY(8px); }
    to { opacity: 1; transform: scale(1) translateY(0); }
  }

  .uninstall-progress,
  .uninstall-complete {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: var(--space-4);
  }

  .uninstall-title {
    font-size: 18px;
    font-weight: 700;
    color: var(--text-primary);
    margin: 0;
    letter-spacing: -0.02em;
  }

  .uninstall-subtitle {
    font-size: 13px;
    color: var(--text-secondary);
    margin: 0;
  }

  .uninstall-bar-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
  }

  .uninstall-bar-label {
    font-size: 13px;
    font-weight: 600;
    font-family: var(--font-mono);
    color: var(--text-secondary);
  }

  .uninstall-bar-percent {
    font-size: 14px;
    font-weight: 700;
    font-family: var(--font-mono);
    color: var(--text-primary);
  }

  .uninstall-track {
    width: 100%;
    height: 10px;
    background: var(--bg-tertiary);
    border-radius: 5px;
    overflow: hidden;
    position: relative;
  }

  .uninstall-fill {
    height: 100%;
    border-radius: 5px;
    background: var(--system-accent);
    transition: width 300ms ease;
  }

  .uninstall-fill-active {
    animation: uninstall-pulse 2s ease-in-out infinite;
  }

  @keyframes uninstall-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.7; }
  }

  .uninstall-fill-indeterminate {
    width: 40% !important;
    animation: uninstall-indeterminate 1.5s ease-in-out infinite;
  }

  @keyframes uninstall-indeterminate {
    0% { transform: translateX(-100%); }
    100% { transform: translateX(350%); }
  }

  .uninstall-current {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
  }

  .uninstall-mod-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .uninstall-step {
    font-size: 12px;
    color: var(--text-tertiary);
    font-style: italic;
  }

  .uninstall-fail-badge {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    font-size: 12px;
    font-weight: 600;
    color: #ef4444;
    background: rgba(239, 68, 68, 0.12);
    padding: 4px 12px;
    border-radius: 100px;
  }

  .uninstall-result-chips {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
    justify-content: center;
  }

  .result-chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: 4px 12px;
    border-radius: 100px;
    font-size: 12px;
    font-weight: 600;
  }

  .result-success {
    color: #22c55e;
    background: rgba(34, 197, 94, 0.12);
  }

  .result-neutral {
    color: var(--text-secondary);
    background: var(--surface-hover);
  }

  .result-error {
    color: #ef4444;
    background: rgba(239, 68, 68, 0.12);
  }

  .uninstall-errors {
    width: 100%;
    max-height: 120px;
    overflow-y: auto;
    text-align: left;
    padding: var(--space-2) var(--space-3);
    background: rgba(239, 68, 68, 0.06);
    border: 1px solid rgba(239, 68, 68, 0.15);
    border-radius: var(--radius-sm);
  }

  .uninstall-error-line {
    font-size: 11px;
    color: #ef4444;
    margin: 0 0 4px 0;
    line-height: 1.4;
  }

  /* Linux / non-macOS: opaque backgrounds */
  :global(html:not(.vibrancy-active)) .header-toolbar {
    background: var(--surface-subtle);
    backdrop-filter: none;
    -webkit-backdrop-filter: none;
  }
  :global(html:not(.vibrancy-active)) .modal-dialog {
    background: var(--bg-grouped);
    backdrop-filter: none;
    -webkit-backdrop-filter: none;
  }

</style>
