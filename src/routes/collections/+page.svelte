<script lang="ts">
  import { onMount, onDestroy, untrack } from "svelte";
  import { goto } from "$app/navigation";
  import InstructionParser from "$lib/components/InstructionParser.svelte";
  import { selectedGame, showError, showSuccess, collectionInstallStatus, collectionUninstallStatus } from "$lib/stores";
  import type { CollectionUninstallStatus } from "$lib/stores";
  import type { UninstallProgressEvent } from "$lib/types";
  import type { CollectionInfo, CollectionManifest, CollectionMod, CollectionModEntry, CollectionSearchResult, InstalledMod, NexusModInfo, NexusCategory, NexusSearchResult, NexusModFile, CollectionInstallCheckpoint } from "$lib/types";
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
    listInstalledCollections,
    switchCollection,
    deleteCollection,
    getCollectionDownloadSize,
    getCollectionDiff,
    getInstalledMods,
    detectCollectionTools,
    getModFiles,
    getNexusModDetail,
    downloadAndInstallNexusMod,
    closeBrowserWebview,
    checkDeploymentHealth,
    checkSkyrimVersion,
    listGameVersions,
    swapGameVersion,
    startDepotDownload,
    checkDepotReady,
    applyDowngrade,
    getDepotDownloadCommand,
    getIncompleteCollectionInstalls,
    resumeCollectionInstall,
    abandonCollectionInstall,
    checkCachedFiles,
    scanGameDirectory,
    cleanGameDirectory,
    hasGameSnapshot,
    checkDlcStatus,
    launchGame,
  } from "$lib/api";
  import { startInstallTracking, stopInstallTracking, resumeInstallTracking } from "$lib/installService";
  import { listen } from "@tauri-apps/api/event";
  import type { CollectionSummary, CollectionDiff, RequiredTool, CleanReport, CleanOptions, DlcStatus, DeploymentHealth, CachedVersion, DepotDownloadInfo } from "$lib/types";
  import { config } from "$lib/stores";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { marked } from "marked";
  import DOMPurify from "dompurify";
  import { bbcodeToHtml } from "$lib/bbcode";
  import CompatibilityPanel from "$lib/components/CompatibilityPanel.svelte";
  import RequiredToolsPrompt from "$lib/components/RequiredToolsPrompt.svelte";
  import NexusLogo from "$lib/components/NexusLogo.svelte";
  import WabbajackLogo from "$lib/components/WabbajackLogo.svelte";
  import WebViewToggle from "$lib/components/WebViewToggle.svelte";

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
  let deleteDownloads = $state(false);
  let deleteRemoveAllMods = $state(false);
  let deleteCleanGameDir = $state(false);
  let deleteHasSnapshot = $state(false);
  let deleteDownloadSize = $state<number | null>(null);
  let deleteDownloadSizeLoading = $state(false);
  let collectionDiffs = $state<Record<string, CollectionDiff | "loading" | "error">>({});
  let collectionHealth = $state<Record<string, DeploymentHealth | "loading" | "error">>({});

  async function handleVerifyCollection(colName: string) {
    const game = $selectedGame;
    if (!game) return;
    collectionHealth = { ...collectionHealth, [colName]: "loading" };
    try {
      const health = await checkDeploymentHealth(game.game_id, game.bottle_name);
      collectionHealth = { ...collectionHealth, [colName]: health };
    } catch {
      collectionHealth = { ...collectionHealth, [colName]: "error" };
    }
  }

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
      await loadMyCollections();
    } catch (e: unknown) {
      showError(`Failed to switch: ${e}`);
    } finally {
      switchingCollection = null;
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

  let unlistenUninstall: (() => void) | null = null;

  function humanizeUninstallStep(step: string): string {
    switch (step) {
      case "undeploying": return "Removing deployed files...";
      case "cleaning_staging": return "Cleaning staging files...";
      case "cleaning_downloads": return "Removing downloads...";
      case "removing_from_db": return "Removing from database...";
      default: return step;
    }
  }

  function formatDiskSize(bytes: number): string {
    if (bytes >= 1_073_741_824) return (bytes / 1_073_741_824).toFixed(1) + " GB";
    if (bytes >= 1_048_576) return (bytes / 1_048_576).toFixed(1) + " MB";
    if (bytes >= 1024) return (bytes / 1024).toFixed(1) + " KB";
    return bytes + " B";
  }

  async function showDeleteConfirmation(name: string) {
    const game = $selectedGame;
    if (!game) return;
    confirmDeleteCollection = name;
    deleteDownloads = false;
    deleteCleanGameDir = false;
    deleteHasSnapshot = false;
    deleteDownloadSize = null;
    deleteDownloadSizeLoading = true;
    try {
      const [size, snap] = await Promise.all([
        getCollectionDownloadSize(game.game_id, game.bottle_name, name).catch(() => null),
        hasGameSnapshot(game.game_id, game.bottle_name).catch(() => false),
      ]);
      deleteDownloadSize = size;
      deleteHasSnapshot = snap;
    } catch {
      deleteDownloadSize = null;
    } finally {
      deleteDownloadSizeLoading = false;
    }
  }

  async function handleDeleteCollection(name: string) {
    const game = $selectedGame;
    if (!game) return;
    const shouldCleanGameDir = deleteCleanGameDir;
    deletingCollection = name;
    confirmDeleteCollection = null;

    // Initialize uninstall status
    collectionUninstallStatus.set({
      active: true,
      collectionName: name,
      totalMods: 0,
      currentMod: 0,
      currentModName: "",
      currentStep: "",
      completed: 0,
      failed: 0,
      phase: "removing",
      errors: [],
      result: null,
    });

    // Listen for progress events
    unlistenUninstall = await listen<UninstallProgressEvent>("uninstall-progress", (event) => {
      const e = event.payload;
      collectionUninstallStatus.update((s) => {
        if (!s) return s;
        const next = { ...s };

        switch (e.kind) {
          case "uninstallStarted":
            next.totalMods = e.total_mods;
            break;
          case "modUninstalling":
            next.currentMod = e.mod_index + 1;
            next.currentModName = e.mod_name;
            next.currentStep = e.step;
            break;
          case "modUninstalled":
            next.completed = next.completed + 1;
            break;
          case "modUninstallFailed":
            next.failed = next.failed + 1;
            next.errors = [...next.errors, `${e.mod_name}: ${e.error}`];
            break;
          case "redeployStarted":
            next.phase = "redeploying";
            next.currentModName = "";
            next.currentStep = "Redeploying remaining mods...";
            break;
          case "redeployCompleted":
            break;
          case "uninstallCompleted":
            next.phase = "complete";
            next.result = { modsRemoved: e.mods_removed, downloadsRemoved: e.downloads_removed };
            if (e.errors.length > 0) {
              next.errors = e.errors;
            }
            break;
        }
        return next;
      });
    });

    try {
      await deleteCollection(game.game_id, game.bottle_name, name, deleteDownloads, deleteRemoveAllMods);

      // After successful uninstall, optionally clean non-stock files (preserving SKSE)
      if (shouldCleanGameDir) {
        collectionUninstallStatus.update((s) => {
          if (!s) return s;
          return { ...s, currentStep: "Cleaning non-stock files from game directory...", phase: "redeploying" };
        });
        try {
          const cleanResult = await cleanGameDirectory(game.game_id, game.bottle_name, {
            remove_loose_files: true,
            remove_archives: true,
            remove_enb: false,
            remove_saves: false,
            remove_skse: false,
            orphans_only: false,
            dry_run: false,
            exclude_patterns: [],
          });
          collectionUninstallStatus.update((s) => {
            if (!s) return s;
            return { ...s, currentStep: `Cleaned ${cleanResult.removed_files.length} non-stock files` };
          });
        } catch (cleanErr: unknown) {
          collectionUninstallStatus.update((s) => {
            if (!s) return s;
            return { ...s, errors: [...s.errors, `Game dir cleanup: ${cleanErr}`] };
          });
        }
      }
    } catch (e: unknown) {
      showError(`Failed to delete: ${e}`);
      collectionUninstallStatus.set(null);
    } finally {
      unlistenUninstall?.();
      unlistenUninstall = null;
      deletingCollection = null;
    }
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

  let browseInitializedForGame = $state<string | null>(null);
  $effect(() => {
    const game = $selectedGame;
    const tab = activeTab;
    const connected = untrack(() => account?.connected);
    if (tab === "browse_mods" && game && connected) {
      // Only reload when the game actually changes, not on every account update
      const gameKey = `${game.game_id}:${game.bottle_name}`;
      if (untrack(() => browseInitializedForGame) !== gameKey) {
        browseInitializedForGame = gameKey;
        loadBrowseMods();
        loadBrowseCategories();
        loadBrowseInstalledIds();
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
  let apiKeyInput = $state("");
  let validationError = $state<string | null>(null);

  // ---- Collections State ----

  let collections = $state<CollectionInfo[]>([]);
  let filtered = $state<CollectionInfo[]>([]);
  let loading = $state(false);
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
  let loadingDetail = $state(false);
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

  // Resume interrupted install
  let interruptedInstall = $state<CollectionInstallCheckpoint | null>(null);
  let resuming = $state(false);

  // Tool requirement detection
  let pendingTools = $state<RequiredTool[]>([]);
  let showToolsPrompt = $state(false);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let pendingManifest = $state<(CollectionManifest & Record<string, unknown>) | null>(null);

  // Pre-install cleanup
  let showCleanupModal = $state(false);
  let cleanReport = $state<CleanReport | null>(null);
  let cleanScanning = $state(false);
  let cleanRunning = $state(false);
  let cleanOptions = $state<CleanOptions>({
    remove_loose_files: true,
    remove_archives: true,
    remove_enb: false,
    remove_saves: false,
    remove_skse: false,
    orphans_only: false,
    dry_run: false,
    exclude_patterns: [],
  });
  let cleanExcludeInput = $state("");

  // Version mismatch
  let showVersionMismatch = $state(false);
  let versionMismatchInfo = $state<{ expected: string[]; detected: string } | null>(null);
  let versionSwapping = $state(false);
  let versionCache = $state<CachedVersion[]>([]);
  let depotDownloading = $state(false);
  let depotPollTimer = $state<ReturnType<typeof setInterval> | null>(null);

  // DLC Detection
  let showDlcWarning = $state(false);
  let dlcStatus = $state<DlcStatus | null>(null);
  let dlcLaunching = $state(false);

  // Optional mod picker
  let showOptionalPicker = $state(false);
  let optionalPickerManifest = $state<(CollectionManifest & Record<string, unknown>) | null>(null);
  type OptionalModChoice = "install" | "install_disabled" | "skip";
  let optionalChoices = $state<Map<number, OptionalModChoice>>(new Map());

  // Pre-computed counts for optional picker to avoid inline .filter() in template
  const optionalPickerRequiredCount = $derived(
    optionalPickerManifest?.mods.filter((m: { optional: boolean }) => !m.optional).length ?? 0
  );
  const optionalPickerOptionalCount = $derived(
    optionalPickerManifest?.mods.filter((m: { optional: boolean }) => m.optional).length ?? 0
  );
  const optionalPickerInstallCount = $derived(
    optionalPickerManifest
      ? optionalPickerManifest.mods.length - Array.from(optionalChoices.values()).filter(v => v === "skip").length
      : 0
  );

  // ---- Mod Browse State ----
  let browseMods = $state<NexusModInfo[]>([]);
  let browseModsLoading = $state(false);
  let browseModsSearch = $state("");
  let browseNsfwFilter = $state<"hide" | "show" | "only">("hide");
  let browseModsSort = $state<"endorsements" | "downloads" | "name" | "updated" | "createdAt">("endorsements");
  let browseModsTotalCount = $state(0);
  let browseModsOffset = $state(0);
  let browseModsHasMore = $state(false);
  const BROWSE_PAGE_SIZE = 20;
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
  let collectionsWebviewToggle: WebViewToggle | null = $state(null);
  let browseViewMode = $state<"app" | "website">("app");
  let collectionsViewMode = $state<"app" | "website">("app");

  // Mod detail view state (Browse Nexus tab)
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
        // Fallback to v1 REST browse
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
      // Filter out deleted/archived, sort by category
      const categoryOrder: Record<string, number> = { main: 0, update: 1, optional: 2, miscellaneous: 3, old_version: 4 };
      browseModFiles = files
        .filter((f: NexusModFile) => f.category !== "deleted" && f.category !== "archived")
        .sort((a: NexusModFile, b: NexusModFile) => (categoryOrder[a.category] ?? 5) - (categoryOrder[b.category] ?? 5));
      // Render description (NexusMods returns BBCode)
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
      // Filter out deleted/archived, sort: main first
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
    const game = $selectedGame;
    if (!game || !filePickerMod) return;
    const slug = getGameSlug();
    if (!slug) return;

    downloadingFile = file.file_id;
    downloadProgress = { downloaded: 0, total: 0 };

    // Listen for download progress events
    const unlisten = await listen<{ downloaded: number; total: number; mod_name: string }>("download-progress", (e) => {
      downloadProgress = { downloaded: e.payload.downloaded, total: e.payload.total };
    });

    try {
      await downloadAndInstallNexusMod(slug, filePickerMod.mod_id, file.file_id, game.game_id, game.bottle_name);
      showSuccess(`Installed "${filePickerMod.name}" successfully`);
      browseInstalledNexusIds = new Set([...browseInstalledNexusIds, filePickerMod.mod_id]);
      closeFilePicker();
    } catch (e) {
      showError(`Download failed: ${e}`);
    } finally {
      unlisten();
      downloadingFile = null;
      downloadProgress = null;
    }
  }

  function formatFileSize(kb: number): string {
    if (kb >= 1_048_576) return `${(kb / 1_048_576).toFixed(1)} GB`;
    if (kb >= 1024) return `${(kb / 1024).toFixed(1)} MB`;
    return `${kb} KB`;
  }

  const gameOptions = $derived.by(() => {
    const gamesSet = new Set(collections.map(c => c.game_domain));
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
    await checkAccount();
    // Check for interrupted collection installs
    const game = $selectedGame;
    if (game) {
      try {
        const incomplete = await getIncompleteCollectionInstalls(game.game_id, game.bottle_name);
        if (incomplete.length > 0) {
          interruptedInstall = incomplete[0];
        }
      } catch {
        // Silently ignore — not critical
      }
      // Smart default tab: if user has no installed collections, show Nexus browse tab
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

  onDestroy(() => {
    statsBarObserver?.disconnect();
    if (collectionsSearchTimer) { clearTimeout(collectionsSearchTimer); collectionsSearchTimer = null; }
    if (collectionsAuthorTimer) { clearTimeout(collectionsAuthorTimer); collectionsAuthorTimer = null; }
    if (browseSearchTimer) { clearTimeout(browseSearchTimer); browseSearchTimer = null; }
    if (browseAuthorTimer) { clearTimeout(browseAuthorTimer); browseAuthorTimer = null; }
    if (depotPollTimer) { clearInterval(depotPollTimer); depotPollTimer = null; }
    // Close any active webviews when navigating away
    closeBrowserWebview().catch(() => {});
  });

  async function handleResumeInstall() {
    if (!interruptedInstall) return;
    resuming = true;
    try {
      const modStatuses = JSON.parse(interruptedInstall.mod_statuses) as Record<string, string>;
      await resumeInstallTracking(
        interruptedInstall.collection_name,
        interruptedInstall.total_mods,
        interruptedInstall.completed_mods,
        modStatuses,
      );
      goto("/collections/progress");
      resumeCollectionInstall(interruptedInstall.id).catch(() => {});
    } catch (e: unknown) {
      showError(`Failed to resume: ${e}`);
      resuming = false;
    }
  }

  async function handleDismissInstall() {
    if (!interruptedInstall) return;
    try {
      await abandonCollectionInstall(interruptedInstall.id);
    } catch {
      // ignore
    }
    interruptedInstall = null;
  }

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
      // "size" is client-side only — use "endorsements" as server sort to get consistent results
      const serverSort = sortField === "size" ? "endorsements" : sortField;
      // Pass NSFW filter server-side so pagination reflects the correct count
      const adultContentFilter = nsfwFilter === "hide" ? false : nsfwFilter === "only" ? true : null;
      const result: CollectionSearchResult = await browseCollections(
        gameDomain, collectionsPerPage, collectionsOffset,
        serverSort, sortDirection, searchText,
        collectionsAuthorFilter.trim() || undefined,
        collectionsMinDownloads || undefined,
        collectionsMinEndorsements || undefined,
        adultContentFilter,
      );
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
      showError(`Failed to load collections: ${e}`);
    } finally {
      loading = false;
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

      for (let i = 0; i < cols.length; i += CONCURRENCY) {
        const batch = cols.slice(i, i + CONCURRENCY);
        const results = await Promise.allSettled(
          batch.map(c => getCollectionMods(c.slug, c.latest_revision))
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

  function reloadWithSort() {
    const gd = gameFilter !== "all" ? gameFilter : "skyrimspecialedition";
    loadCollections(gd);
  }

  function collectionsGoToPage(page: number) {
    collectionsOffset = (page - 1) * collectionsPerPage;
    const gd = gameFilter !== "all" ? gameFilter : "skyrimspecialedition";
    loadCollections(gd, false);
  }

  function setCollectionsPerPage(n: number) {
    collectionsPerPage = n;
    if (typeof localStorage !== 'undefined') {
      localStorage.setItem('corkscrew-collections-per-page', String(n));
    }
    collectionsOffset = 0;
    const gd = gameFilter !== "all" ? gameFilter : "skyrimspecialedition";
    loadCollections(gd, false);
  }

  async function viewCollectionDetail(collection: CollectionInfo) {
    loadingDetail = true;
    renderedDescription = "";
    renderedInstallInstructions = "";
    rawInstallInstructions = "";
    detailCacheInfo = null;
    try {
      const [detail, modsResult] = await Promise.all([
        getCollection(collection.slug, collection.game_domain),
        getCollectionMods(collection.slug, collection.latest_revision),
      ]);
      selectedCollection = detail;
      selectedMods = modsResult.mods;
      selectedGameVersions = modsResult.game_versions;

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
      showError(`Failed to load collection details: ${e}`);
    } finally {
      loadingDetail = false;
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
      installInstructions: renderedInstallInstructions ? renderedInstallInstructions : null,
      slug: selectedCollection.slug ?? null,
      image_url: selectedCollection.image_url ?? null,
      revision: selectedCollection.latest_revision ?? null,
      gameVersions: selectedGameVersions,
    };

    // Check for optional mods — show picker if any exist
    const optionalMods = manifest.mods.filter((m: { optional: boolean }) => m.optional);
    if (optionalMods.length > 0) {
      optionalPickerManifest = manifest;
      const choices = new Map<number, OptionalModChoice>();
      manifest.mods.forEach((m: { optional: boolean }, i: number) => {
        if (m.optional) choices.set(i, "install_disabled");
      });
      optionalChoices = choices;
      showOptionalPicker = true;
      return;
    }

    // Check for required tools before installing
    await checkToolsAndProceed(manifest);
  }

  async function checkToolsAndProceed(manifest: CollectionManifest & Record<string, unknown>) {
    if (!$selectedGame) return;

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

    // Check game version compatibility
    await checkGameVersionAndProceed(manifest);
  }

  async function checkGameVersionAndProceed(manifest: CollectionManifest & Record<string, unknown>) {
    if (!$selectedGame) return;

    const versions = manifest.gameVersions ?? [];
    // Skip if no game versions specified or not Skyrim SE
    if (versions.length === 0 || $selectedGame.game_id !== "skyrimse") {
      await checkPreInstallCleanup(manifest);
      return;
    }

    try {
      const status = await checkSkyrimVersion($selectedGame.game_id, $selectedGame.bottle_name);
      const detected = status.current_version;

      // Compare SE vs AE categories, not exact version strings.
      // SE = 1.5.x, AE = 1.6.x. If both are in the same category, no mismatch.
      const detectedIsSE = detected.startsWith("1.5.");
      const targetsSE = versions.some(v => v.startsWith("1.5."));
      const targetsAE = versions.some(v => v.startsWith("1.6."));

      // Mismatch only when categories differ (e.g., user has AE but collection targets SE only)
      const mismatch = (detectedIsSE && !targetsSE && targetsAE)
        || (!detectedIsSE && targetsSE && !targetsAE);

      if (mismatch) {
        try {
          versionCache = await listGameVersions($selectedGame.game_id);
        } catch { versionCache = []; }
        versionMismatchInfo = { expected: versions, detected };
        pendingManifest = manifest;
        showVersionMismatch = true;
        return;
      }
    } catch {
      // Version check is best-effort; proceed if it fails
    }

    await checkPreInstallCleanup(manifest);
  }

  function confirmOptionalPicker() {
    if (!optionalPickerManifest) return;

    const mods = optionalPickerManifest.mods
      .map((m: CollectionModEntry, i: number) => {
        const choice = optionalChoices.get(i);
        if (m.optional && choice === "skip") return null;
        if (m.optional && choice === "install_disabled") {
          return { ...m, install_disabled: true };
        }
        return m; // required mods + "install" optionals pass through
      })
      .filter((m): m is CollectionModEntry => m !== null);

    const filtered = { ...optionalPickerManifest, mods };

    showOptionalPicker = false;
    optionalPickerManifest = null;
    checkToolsAndProceed(filtered as CollectionManifest & Record<string, unknown>);
  }

  async function checkPreInstallCleanup(manifest: CollectionManifest & Record<string, unknown>) {
    if (!$selectedGame) return;

    try {
      // Check DLC status first
      const dlc = await checkDlcStatus($selectedGame.game_id, $selectedGame.bottle_name);
      if (!dlc.all_present && dlc.dlcs.length > 0) {
        dlcStatus = dlc;
        pendingManifest = manifest;
        showDlcWarning = true;
        return;
      }

      // Only offer cleanup if a baseline snapshot exists
      const hasSnap = await hasGameSnapshot($selectedGame.game_id, $selectedGame.bottle_name);
      if (!hasSnap) {
        // No snapshot = can't determine stock vs non-stock, skip cleanup
        await proceedWithInstall(manifest);
        return;
      }

      cleanScanning = true;
      pendingManifest = manifest;

      const report = await scanGameDirectory($selectedGame.game_id, $selectedGame.bottle_name);
      cleanScanning = false;

      // Only prompt cleanup if there are unmanaged (orphaned) non-stock files.
      // Files deployed by Corkscrew (is_managed) are fine — don't nag the user about those.
      const orphanedFiles = report.non_stock_files.filter((f: { is_managed: boolean }) => !f.is_managed);
      if (orphanedFiles.length === 0) {
        await proceedWithInstall(manifest);
        return;
      }

      // Show cleanup modal
      cleanReport = report;
      showCleanupModal = true;
    } catch {
      // Cleanup scan is best-effort; proceed if it fails
      cleanScanning = false;
      await proceedWithInstall(manifest);
    }
  }

  async function handleCleanAndInstall() {
    if (!$selectedGame || !pendingManifest) return;

    cleanRunning = true;
    try {
      // Parse exclude patterns from the input
      const patterns = cleanExcludeInput
        .split("\n")
        .map((p) => p.trim())
        .filter((p) => p.length > 0);
      const options: CleanOptions = {
        ...cleanOptions,
        exclude_patterns: patterns,
        dry_run: false,
      };

      const result = await cleanGameDirectory(
        $selectedGame.game_id,
        $selectedGame.bottle_name,
        options
      );

      showSuccess(`Cleaned ${result.removed_files.length} files (${formatSize(result.bytes_freed)} freed)`);
    } catch (e) {
      showError(`Cleanup failed: ${e}`);
    } finally {
      cleanRunning = false;
      showCleanupModal = false;
      cleanReport = null;
    }

    // Proceed with the install
    await proceedWithInstall(pendingManifest);
  }

  function handleSkipCleanup() {
    showCleanupModal = false;
    cleanReport = null;
    if (pendingManifest) {
      proceedWithInstall(pendingManifest);
    }
  }

  function handleCancelCleanup() {
    showCleanupModal = false;
    cleanReport = null;
    pendingManifest = null;
  }

  async function handleDlcContinue() {
    // User chose to continue despite missing DLC
    showDlcWarning = false;
    dlcStatus = null;
    if (pendingManifest) {
      // Continue to cleanup check
      const manifest = pendingManifest;
      if (!$selectedGame) return;
      try {
        const hasSnap = await hasGameSnapshot($selectedGame.game_id, $selectedGame.bottle_name);
        if (!hasSnap) {
          await proceedWithInstall(manifest);
          return;
        }
        cleanScanning = true;
        const report = await scanGameDirectory($selectedGame.game_id, $selectedGame.bottle_name);
        cleanScanning = false;
        const orphanedFiles = report.non_stock_files.filter((f: { is_managed: boolean }) => !f.is_managed);
        if (orphanedFiles.length === 0) {
          await proceedWithInstall(manifest);
          return;
        }
        cleanReport = report;
        showCleanupModal = true;
      } catch {
        cleanScanning = false;
        await proceedWithInstall(manifest);
      }
    }
  }

  async function handleDlcLaunchGame() {
    if (!$selectedGame) return;
    dlcLaunching = true;
    try {
      await launchGame($selectedGame.game_id, $selectedGame.bottle_name, false);
      showSuccess("Game launched. Close it after reaching the main menu, then try installing again.");
    } catch (e) {
      showError(`Failed to launch game: ${e}`);
    } finally {
      dlcLaunching = false;
    }
  }

  function handleDlcCancel() {
    showDlcWarning = false;
    dlcStatus = null;
    pendingManifest = null;
  }

  async function proceedWithInstall(manifest: CollectionManifest & Record<string, unknown>) {
    if (!selectedCollection || !$selectedGame) return;

    installing = true;
    installResult = null;
    userActions = [];

    // Start the centralized install tracking service
    const modNames = manifest.mods.map((m: { name: string }) => m.name);
    await startInstallTracking(selectedCollection.name, modNames.length, modNames, selectedCollection.description || selectedCollection.summary);

    // Navigate to the progress page
    goto('/collections/progress');

    // Fire-and-forget: don't await the Tauri command since we've navigated away.
    // The progress page tracks status via Tauri events in the install service.
    // Errors update the store directly so the progress page can display them.
    const collectionName = selectedCollection.name;
    const gameId = $selectedGame.game_id;
    const bottleName = $selectedGame.bottle_name;

    installCollection(manifest, gameId, bottleName)
      .then((result) => {
        installResult = result;
        if (result.failed === 0 && result.skipped === 0) {
          showSuccess(`Collection "${collectionName}" installed successfully`);
        }
      })
      .catch((e: unknown) => {
        showError(`Collection install failed: ${e}`);
        collectionInstallStatus.update(s => s ? { ...s, phase: "failed" as const } : s);
      })
      .finally(() => {
        installing = false;
      });
  }

  function backToBrowse() {
    selectedCollection = null;
    selectedMods = [];
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
    <button class="tab-btn" class:tab-active={activeTab === "my"} onclick={() => { closeBrowserWebview().catch(() => {}); activeTab = "my"; }}>
      My Collections
      {#if myCollections.length > 0}
        <span class="tab-count">{myCollections.length}</span>
      {/if}
    </button>
    <button class="tab-btn" class:tab-active={activeTab === "nexus"} onclick={() => { closeBrowserWebview().catch(() => {}); activeTab = "nexus"; }}>
      <NexusLogo size={14} />
      Nexus Mods Collections
    </button>
    <button class="tab-btn" class:tab-active={activeTab === "wabbajack"} onclick={() => { closeBrowserWebview().catch(() => {}); activeTab = "wabbajack"; }}>
      <WabbajackLogo size={14} />
      Wabbajack Lists
    </button>
    <button class="tab-btn" class:tab-active={activeTab === "browse_mods"} onclick={() => { closeBrowserWebview().catch(() => {}); activeTab = "browse_mods"; }}>
      <NexusLogo size={14} />
      Browse Nexus
    </button>
  </div>

  {#if interruptedInstall}
    <div class="resume-banner">
      <div class="resume-info">
        <div class="resume-icon-wrap">
          <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="#f59e0b" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
            <line x1="12" y1="9" x2="12" y2="13" />
            <line x1="12" y1="17" x2="12.01" y2="17" />
          </svg>
        </div>
        <div class="resume-text">
          <span class="resume-title">Interrupted Installation Detected</span>
          <span class="resume-detail">
            "{interruptedInstall.collection_name}" — {interruptedInstall.completed_mods} of {interruptedInstall.total_mods} mods completed
            {#if interruptedInstall.failed_mods > 0}
              <span class="resume-failed">({interruptedInstall.failed_mods} failed)</span>
            {/if}
          </span>
          <div class="resume-progress-mini">
            <div class="resume-progress-fill" style="width: {Math.round((interruptedInstall.completed_mods / interruptedInstall.total_mods) * 100)}%"></div>
          </div>
        </div>
      </div>
      <div class="resume-actions">
        <button class="btn btn-primary" onclick={handleResumeInstall} disabled={resuming}>
          {#if resuming}
            <span class="spinner spinner-sm"></span> Resuming...
          {:else}
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="5 3 19 12 5 21 5 3" /></svg>
            Resume Installation
          {/if}
        </button>
        <button class="btn btn-ghost" onclick={handleDismissInstall}>Dismiss</button>
      </div>
    </div>
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
        {#each myCollections as col (col.name)}
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
                  {switchingCollection === col.name ? "Switching..." : "Activate"}
                </button>
                <button
                  class="btn btn-secondary btn-sm"
                  onclick={(e) => { e.stopPropagation(); handleVerifyCollection(col.name); }}
                  disabled={collectionHealth[col.name] === "loading"}
                  title="Verify staging files, deployed files, and file integrity"
                >
                  {collectionHealth[col.name] === "loading" ? "Checking..." : "Verify"}
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
                  {#each selectedMods as mod, i (mod.name)}
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
    <!-- Browse Nexus Tab -->
    <header class="page-header">
      <div class="header-text">
        <h2 class="page-title"><NexusLogo size={22} /> Browse Nexus</h2>
        <p class="page-subtitle">Discover mods on NexusMods for {$selectedGame?.display_name ?? "your game"}</p>
      </div>
      <div class="header-right">
        <WebViewToggle
          bind:this={browseWebviewToggle}
          url={`https://www.nexusmods.com/${getGameSlug()}/mods/`}
          defaultMode={account?.connected ? "app" : "website"}
          onModeChange={(m) => browseViewMode = m}
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
      <div class="webview-placeholder">
        <p class="webview-hint">Browsing NexusMods directly. Switch to "In-App" to use built-in search and filters.</p>
      </div>
    {:else if !$selectedGame}
      <div class="empty-state">
        <p class="empty-title">No game selected</p>
        <p class="empty-detail">Select a game from the sidebar to browse mods.</p>
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
                    disabled={!$selectedGame}
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

  {:else if activeTab === "nexus"}
    <!-- Nexus Mods Collections -->
    <header class="page-header">
      <div class="header-text">
        <h2 class="page-title"><NexusLogo size={22} /> Nexus Mods Collections</h2>
        <p class="page-subtitle">Browse and install curated mod collections from Nexus Mods</p>
      </div>
      <div class="header-right">
        <WebViewToggle
          bind:this={collectionsWebviewToggle}
          url={`https://next.nexusmods.com/${getGameSlug()}/collections`}
          defaultMode={account?.connected ? "app" : "website"}
          onModeChange={(m) => collectionsViewMode = m}
        />
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

    {#if collectionsViewMode === "website"}
      <div class="webview-placeholder">
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
        <button
          class="nsfw-cycle-btn"
          class:nsfw-show={nsfwFilter === "show"}
          class:nsfw-only={nsfwFilter === "only"}
          onclick={() => { nsfwFilter = cycleNsfwFilter(nsfwFilter); const gd = gameFilter !== "all" ? gameFilter : "skyrimspecialedition"; loadCollections(gd); }}
          title={nsfwFilter === "hide" ? "NSFW hidden" : nsfwFilter === "show" ? "NSFW included" : "NSFW only"}
        >
          <span class="nsfw-indicator">{nsfwIcon(nsfwFilter)}</span>
          {nsfwLabel(nsfwFilter)}
        </button>
        <div class="sort-group">
          <select class="filter-select" bind:value={sortField} onchange={reloadWithSort}>
            <option value="endorsements">Sort: Most Popular</option>
            <option value="name">Sort: Name</option>
            <option value="rating">Sort: Rating</option>
            <option value="created">Sort: Newest</option>
            <option value="updated">Sort: Updated</option>
            <option value="size">Sort: Size</option>
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
        <button class="filter-toggle" onclick={() => showCollectionsAdvancedFilters = !showCollectionsAdvancedFilters}>
          Filters {showCollectionsAdvancedFilters ? '\u25B2' : '\u25BC'}
          {#if collectionsActiveFilterCount > 0}<span class="filter-badge">{collectionsActiveFilterCount}</span>{/if}
        </button>
      </div>

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

{#if showToolsPrompt && $selectedGame}
  <RequiredToolsPrompt
    tools={pendingTools}
    gameId={$selectedGame.game_id}
    bottleName={$selectedGame.bottle_name}
    oncontinue={() => {
      showToolsPrompt = false;
      if (pendingManifest) checkGameVersionAndProceed(pendingManifest);
    }}
    oncancel={() => {
      showToolsPrompt = false;
      pendingManifest = null;
      pendingTools = [];
    }}
  />
{/if}

<!-- Optional Mod Picker Modal -->
{#if showOptionalPicker && optionalPickerManifest}
  <div class="modal-overlay" onclick={(e) => { if (e.target === e.currentTarget) { showOptionalPicker = false; } }} role="presentation">
    <div class="optional-picker-modal" onclick={(e) => e.stopPropagation()} role="dialog" aria-label="Configure optional mods">
      <div class="optional-picker-header">
        <h3>Configure Installation</h3>
        <p class="optional-picker-subtitle">
          {optionalPickerRequiredCount} required
          &middot; {optionalPickerOptionalCount} optional mods
        </p>
      </div>

      <div class="optional-picker-body">
        <!-- Required mods section (collapsed summary) -->
        <div class="optional-section">
          <div class="optional-section-header">
            <span class="optional-section-label">
              <svg class="optional-check" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--green, #22c55e)" stroke-width="2.5" stroke-linecap="round"><path d="M20 6L9 17l-5-5" /></svg>
              {optionalPickerRequiredCount} required mods will be installed
            </span>
          </div>
        </div>

        <!-- Optional mods section -->
        <div class="optional-section">
          <div class="optional-section-header">
            <span class="optional-section-label">Optional</span>
            <div class="optional-section-actions">
              <button class="btn btn-ghost btn-xs" onclick={() => {
                const c = new Map(optionalChoices);
                optionalPickerManifest?.mods.forEach((m: { optional: boolean }, i: number) => { if (m.optional) c.set(i, "install"); });
                optionalChoices = c;
              }}>All</button>
              <button class="btn btn-ghost btn-xs" onclick={() => {
                const c = new Map(optionalChoices);
                optionalPickerManifest?.mods.forEach((m: { optional: boolean }, i: number) => { if (m.optional) c.set(i, "install_disabled"); });
                optionalChoices = c;
              }}>All (Disabled)</button>
              <button class="btn btn-ghost btn-xs" onclick={() => {
                const c = new Map(optionalChoices);
                optionalPickerManifest?.mods.forEach((m: { optional: boolean }, i: number) => { if (m.optional) c.set(i, "skip"); });
                optionalChoices = c;
              }}>None</button>
            </div>
          </div>
          {#each optionalPickerManifest.mods as mod, i}
            {#if mod.optional}
              <div class="optional-mod-row">
                <span class="optional-mod-name">{mod.name}</span>
                <span class="optional-mod-version">{mod.version || ""}</span>
                <select
                  class="optional-mod-select"
                  value={optionalChoices.get(i) ?? "install_disabled"}
                  onchange={(e) => {
                    const c = new Map(optionalChoices);
                    c.set(i, (e.currentTarget as HTMLSelectElement).value as OptionalModChoice);
                    optionalChoices = c;
                  }}
                >
                  <option value="install">Install</option>
                  <option value="install_disabled">Install (Disabled)</option>
                  <option value="skip">Skip</option>
                </select>
              </div>
            {/if}
          {/each}
        </div>
      </div>

      <div class="optional-picker-footer">
        <button class="btn btn-ghost" onclick={() => { showOptionalPicker = false; }}>Cancel</button>
        <button class="btn btn-accent" onclick={confirmOptionalPicker}>
          Install ({optionalPickerInstallCount} mods)
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- Version Mismatch Modal -->
{#if showVersionMismatch && versionMismatchInfo}
  <div class="modal-overlay" onclick={() => { showVersionMismatch = false; versionMismatchInfo = null; }} role="presentation">
    <div class="cleanup-modal" onclick={(e) => e.stopPropagation()} role="dialog" aria-label="Version mismatch warning">
      <div class="cleanup-header">
        <h3 class="cleanup-title">Game Version Mismatch</h3>
        <button class="cleanup-close" onclick={() => { showVersionMismatch = false; versionMismatchInfo = null; }}>&times;</button>
      </div>

      <div class="cleanup-body">
        {#if versionMismatchInfo}
        {@const targetsSE = versionMismatchInfo.expected.some(v => v.startsWith("1.5."))}
        {@const cachedMatch = versionCache.some(cv => targetsSE ? cv.version.startsWith("1.5.") : cv.version.startsWith("1.6."))}
        <div class="cleanup-summary">
          <p class="cleanup-info">
            This collection was built for <strong>Skyrim {versionMismatchInfo.expected.join(' / ')}</strong>{targetsSE ? " (SE)" : " (AE)"},
            but your game is <strong>{versionMismatchInfo.detected}</strong>{versionMismatchInfo.detected.startsWith("1.5.") ? " (SE)" : " (AE)"}.
          </p>
          <p class="cleanup-info" style="margin-top: 0.5rem;">
            SKSE plugins in this collection may not be compatible with your game version.
            {#if cachedMatch}
              A compatible version is cached and can be swapped instantly.
            {:else if depotDownloading}
              Downloading the correct game version from Steam...
            {:else}
              You can download and switch to the correct version, or continue at your own risk.
            {/if}
          </p>
          {#if depotDownloading}
            <div style="display: flex; align-items: center; gap: 0.75rem; margin: 0.75rem 0; padding: 0.75rem; background: var(--surface-1); border-radius: 6px;">
              <div class="spinner" style="width: 20px; height: 20px; border: 2px solid var(--border); border-top-color: var(--blue); border-radius: 50%; animation: spin 1s linear infinite;"></div>
              <span style="font-size: 0.9rem;">Downloading via Steam depot... this may take several minutes.</span>
            </div>
          {/if}
        </div>
      {/if}
      </div>

      <div class="cleanup-actions">
        {#if versionMismatchInfo}
        {@const targetsSEBtn = versionMismatchInfo.expected.some(v => v.startsWith("1.5."))}
        {@const matchingCached = versionCache.filter(cv => targetsSEBtn ? cv.version.startsWith("1.5.") : cv.version.startsWith("1.6."))}
        <button class="btn btn-ghost" disabled={depotDownloading} onclick={() => { showVersionMismatch = false; versionMismatchInfo = null; pendingManifest = null; if (depotPollTimer) { clearInterval(depotPollTimer); depotPollTimer = null; } depotDownloading = false; }}>Cancel</button>
        {#each matchingCached as matchingVersion}
          <button class="btn btn-secondary" disabled={versionSwapping} onclick={async () => {
            if (!$selectedGame) return;
            versionSwapping = true;
            try {
              await swapGameVersion($selectedGame.game_id, $selectedGame.bottle_name, matchingVersion.version);
              showVersionMismatch = false;
              versionMismatchInfo = null;
              showSuccess(`Switched to v${matchingVersion.version}`);
              if (pendingManifest) await checkPreInstallCleanup(pendingManifest);
            } catch (e) {
              showError(`Version swap failed: ${e}`);
            } finally {
              versionSwapping = false;
            }
          }}>
            {versionSwapping ? "Switching..." : `Switch to ${matchingVersion.version.startsWith("1.5.") ? "SE" : "AE"} (v${matchingVersion.version})`}
          </button>
        {/each}
        {#if matchingCached.length === 0 && !depotDownloading && targetsSEBtn}
          <button class="btn btn-secondary" onclick={async () => {
            if (!$selectedGame) return;
            depotDownloading = true;
            try {
              const automated = await startDepotDownload($selectedGame.game_id);
              if (!automated) {
                // Fallback: open Steam console and copy command to clipboard
                try {
                  const info = await getDepotDownloadCommand($selectedGame.game_id, $selectedGame.bottle_name);
                  await navigator.clipboard.writeText(info.command);
                  showSuccess("Command copied! Paste it in the Steam console that was opened.");
                } catch { /* ignore clipboard errors */ }
              }
              // Start polling for depot files
              depotPollTimer = setInterval(async () => {
                if (!$selectedGame) return;
                try {
                  const result = await checkDepotReady($selectedGame.game_id, $selectedGame.bottle_name);
                  if (result) {
                    if (depotPollTimer) clearInterval(depotPollTimer);
                    depotPollTimer = null;
                    // Auto-apply downgrade
                    const status = await applyDowngrade($selectedGame.game_id, $selectedGame.bottle_name);
                    depotDownloading = false;
                    showVersionMismatch = false;
                    versionMismatchInfo = null;
                    showSuccess(`Switched to v${status.current_version}`);
                    if (pendingManifest) await checkPreInstallCleanup(pendingManifest);
                  }
                } catch { /* keep polling */ }
              }, 3000);
            } catch (e) {
              depotDownloading = false;
              showError(`Download failed: ${e}`);
            }
          }}>
            Download & Switch to SE
          </button>
        {/if}
        <button class="btn btn-primary" disabled={depotDownloading} onclick={async () => {
          showVersionMismatch = false;
          versionMismatchInfo = null;
          if (pendingManifest) await checkPreInstallCleanup(pendingManifest);
        }}>Continue Anyway</button>
        {/if}
      </div>
    </div>
  </div>
{/if}

<!-- DLC Warning Modal -->
{#if showDlcWarning && dlcStatus}
  <div class="modal-overlay" onclick={handleDlcCancel} role="presentation">
    <div class="cleanup-modal" onclick={(e) => e.stopPropagation()} role="dialog" aria-label="DLC warning">
      <div class="cleanup-header">
        <h3 class="cleanup-title">Missing DLC Files</h3>
        <button class="cleanup-close" onclick={handleDlcCancel}>&times;</button>
      </div>

      <div class="cleanup-body">
        <div class="cleanup-summary">
          {#if !dlcStatus.game_initialized}
            <p class="cleanup-info">
              The game hasn't been initialized yet. You need to <strong>launch the game at least once</strong>
              so it can create its configuration files and extract DLC content.
            </p>
          {:else}
            <p class="cleanup-info">
              Some DLC files are missing from the game directory. Many collection mods depend on DLC content.
              You may need to <strong>launch the game once</strong> to initialize DLC, or verify your game files through Steam/GOG.
            </p>
          {/if}
        </div>

        <div class="dlc-list">
          {#each dlcStatus.dlcs as dlc}
            <div class="dlc-item" class:dlc-present={dlc.present} class:dlc-missing={!dlc.present}>
              <span class="dlc-icon">{dlc.present ? '✓' : '✗'}</span>
              <span class="dlc-name">{dlc.name}</span>
              {#if !dlc.present && dlc.missing_files.length > 0}
                <span class="dlc-detail">Missing: {dlc.missing_files.join(', ')}</span>
              {/if}
            </div>
          {/each}
        </div>
      </div>

      <div class="cleanup-actions">
        <button class="btn btn-ghost" onclick={handleDlcCancel}>Cancel</button>
        <button class="btn btn-secondary" onclick={handleDlcLaunchGame} disabled={dlcLaunching}>
          {dlcLaunching ? 'Launching...' : 'Launch Game to Initialize'}
        </button>
        <button class="btn btn-primary" onclick={handleDlcContinue}>Install Anyway</button>
      </div>
    </div>
  </div>
{/if}

<!-- Pre-Install Cleanup Modal -->
{#if showCleanupModal && cleanReport}
  <div class="modal-overlay" onclick={handleCancelCleanup} role="presentation">
    <div class="cleanup-modal" onclick={(e) => e.stopPropagation()} role="dialog" aria-label="Pre-install cleanup">
      <div class="cleanup-header">
        <h3 class="cleanup-title">Pre-Install Cleanup</h3>
        <button class="cleanup-close" onclick={handleCancelCleanup}>&times;</button>
      </div>

      <div class="cleanup-body">
        <div class="cleanup-summary">
          <p class="cleanup-info">
            Found <strong>{cleanReport.non_stock_files.length}</strong> non-stock files
            ({formatSize(cleanReport.total_size)}) in the game directory.
            Cleaning these before installing a collection ensures a fresh start.
          </p>

          <div class="cleanup-stats">
            <div class="cleanup-stat">
              <span class="cleanup-stat-value">{cleanReport.orphaned_count}</span>
              <span class="cleanup-stat-label">Orphaned</span>
            </div>
            <div class="cleanup-stat">
              <span class="cleanup-stat-value">{cleanReport.managed_count}</span>
              <span class="cleanup-stat-label">Managed</span>
            </div>
            {#if cleanReport.enb_files.length > 0}
              <div class="cleanup-stat">
                <span class="cleanup-stat-value">{cleanReport.enb_files.length}</span>
                <span class="cleanup-stat-label">ENB Files</span>
              </div>
            {/if}
            {#if cleanReport.save_files.length > 0}
              <div class="cleanup-stat">
                <span class="cleanup-stat-value">{cleanReport.save_files.length}</span>
                <span class="cleanup-stat-label">Saves (safe)</span>
              </div>
            {/if}
          </div>
        </div>

        <div class="cleanup-options">
          <h4>Clean Options</h4>
          <label class="cleanup-checkbox">
            <input type="checkbox" bind:checked={cleanOptions.remove_loose_files} />
            Remove loose mod files (plugins, meshes, textures, scripts)
          </label>
          <label class="cleanup-checkbox">
            <input type="checkbox" bind:checked={cleanOptions.remove_archives} />
            Remove non-stock BSA/BA2 archives
          </label>
          <label class="cleanup-checkbox">
            <input type="checkbox" bind:checked={cleanOptions.remove_enb} />
            Remove ENB files ({cleanReport.enb_files.length} found)
          </label>
          <label class="cleanup-checkbox">
            <input type="checkbox" bind:checked={cleanOptions.orphans_only} />
            Only remove orphaned files (skip Corkscrew-managed files)
          </label>
        </div>

        <details class="cleanup-advanced">
          <summary>Exclude Patterns</summary>
          <p class="cleanup-hint">One glob pattern per line (e.g., <code>SKSE/Plugins/*</code>)</p>
          <textarea
            class="cleanup-exclude-input"
            bind:value={cleanExcludeInput}
            placeholder="SKSE/Plugins/*&#10;SkyUI_SE.bsa"
            rows="3"
          ></textarea>
        </details>

        {#if cleanReport.save_files.length > 0}
          <div class="cleanup-save-notice">
            Save files ({cleanReport.save_files.length}) are automatically excluded from cleanup.
          </div>
        {/if}
      </div>

      <div class="cleanup-footer">
        <button class="btn btn-ghost" onclick={handleCancelCleanup} disabled={cleanRunning}>Cancel</button>
        <button class="btn btn-secondary" onclick={handleSkipCleanup} disabled={cleanRunning}>Skip & Install</button>
        <button class="btn btn-primary" onclick={handleCleanAndInstall} disabled={cleanRunning}>
          {#if cleanRunning}
            <div class="spinner-sm"></div>
            Cleaning...
          {:else}
            Clean & Install
          {/if}
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- Scanning overlay -->
{#if cleanScanning}
  <div class="modal-overlay" role="presentation">
    <div class="cleanup-scanning">
      <div class="spinner-sm"></div>
      <span>Scanning game directory...</span>
    </div>
  </div>
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
{#if confirmDeleteCollection}
  <div class="modal-overlay" onclick={() => { if (!deletingCollection) confirmDeleteCollection = null; }} role="dialog" aria-modal="true" aria-label="Confirm deletion">
    <div class="modal-dialog" onclick={(e) => e.stopPropagation()} role="document">
      <div class="modal-icon">
        <svg width="36" height="36" viewBox="0 0 24 24" fill="none" stroke="#ef4444" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
          <line x1="12" y1="9" x2="12" y2="13" />
          <line x1="12" y1="17" x2="12.01" y2="17" />
        </svg>
      </div>
      <h3 class="modal-title">Delete '{confirmDeleteCollection}'?</h3>
      <p class="modal-desc">This will uninstall all mods in this collection, remove staging files, and clean up the database. This cannot be undone.</p>

      <div class="modal-option">
        <label class="modal-checkbox-label">
          <input type="checkbox" bind:checked={deleteDownloads} />
          <span class="modal-checkbox-text">
            Also delete downloaded archives
            {#if deleteDownloadSizeLoading}
              <span class="modal-size-loading">calculating...</span>
            {:else if deleteDownloadSize != null && deleteDownloadSize > 0}
              <span class="modal-size-badge">saves {formatDiskSize(deleteDownloadSize)}</span>
            {:else if deleteDownloadSize === 0}
              <span class="modal-size-note">no unique downloads</span>
            {/if}
          </span>
        </label>
        {#if deleteDownloads}
          <p class="modal-option-hint modal-option-hint-warn">Archives unique to this collection will be permanently deleted.</p>
        {:else}
          <p class="modal-option-hint">Downloaded archives are kept so you can reinstall later without re-downloading.</p>
        {/if}
      </div>

      <div class="modal-option">
        <label class="modal-checkbox-label">
          <input type="checkbox" bind:checked={deleteRemoveAllMods} />
          <span class="modal-checkbox-text">
            Remove ALL mods, not just this collection
            <span class="modal-size-note">fastest</span>
          </span>
        </label>
        {#if deleteRemoveAllMods}
          <p class="modal-option-hint modal-option-hint-warn">Removes every installed mod for this game, including any manually installed mods outside the collection.</p>
        {:else}
          <p class="modal-option-hint">Only removes mods that belong to this collection.</p>
        {/if}
      </div>

      {#if deleteHasSnapshot}
        <div class="modal-option">
          <label class="modal-checkbox-label">
            <input type="checkbox" bind:checked={deleteCleanGameDir} />
            <span class="modal-checkbox-text">
              Clean non-stock files from game directory
              <span class="modal-size-note">preserves SKSE</span>
            </span>
          </label>
          {#if deleteCleanGameDir}
            <p class="modal-option-hint modal-option-hint-warn">Removes leftover loose files (meshes, textures, scripts, plugins) that aren't part of the original game. SKSE files are preserved.</p>
          {:else}
            <p class="modal-option-hint">Leave the game directory as-is after uninstalling the collection.</p>
          {/if}
        </div>
      {/if}

      <div class="modal-actions">
        <button
          class="btn btn-danger"
          onclick={() => handleDeleteCollection(confirmDeleteCollection!)}
          disabled={deletingCollection === confirmDeleteCollection}
        >
          {#if deletingCollection === confirmDeleteCollection}
            <svg class="icon-spin" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
              <line x1="12" y1="2" x2="12" y2="6" />
              <line x1="12" y1="18" x2="12" y2="22" />
              <line x1="4.93" y1="4.93" x2="7.76" y2="7.76" />
              <line x1="16.24" y1="16.24" x2="19.07" y2="19.07" />
              <line x1="2" y1="12" x2="6" y2="12" />
              <line x1="18" y1="12" x2="22" y2="12" />
              <line x1="4.93" y1="19.07" x2="7.76" y2="16.24" />
              <line x1="16.24" y1="7.76" x2="19.07" y2="4.93" />
            </svg>
            Deleting...
          {:else}
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <polyline points="3 6 5 6 21 6" />
              <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
            </svg>
            Delete Collection
          {/if}
        </button>
        <button
          class="btn btn-ghost"
          onclick={() => confirmDeleteCollection = null}
          disabled={deletingCollection === confirmDeleteCollection}
        >
          Cancel
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  /* ---- Page Layout ---- */

  .collections-page {
    padding: var(--space-2) 0 var(--space-12) 0;
  }

  /* ---- Resume Banner ---- */

  .resume-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    padding: var(--space-4) var(--space-5);
    background: rgba(255, 159, 10, 0.08);
    border: 2px solid rgba(255, 159, 10, 0.4);
    border-radius: var(--radius-md);
    margin-bottom: var(--space-4);
    animation: resume-attention 2s ease-in-out 2;
  }

  @keyframes resume-attention {
    0%, 100% { border-color: rgba(255, 159, 10, 0.4); }
    50% { border-color: rgba(255, 159, 10, 0.8); background: rgba(255, 159, 10, 0.12); }
  }

  .resume-info { display: flex; align-items: center; gap: var(--space-3); min-width: 0; }
  .resume-icon-wrap { flex-shrink: 0; }
  .resume-text { display: flex; flex-direction: column; gap: 4px; min-width: 0; }
  .resume-title { font-weight: 700; color: var(--text-primary); font-size: 14px; }
  .resume-detail { font-size: 12px; color: var(--text-secondary); }
  .resume-failed { color: #ef4444; font-weight: 600; }
  .resume-progress-mini {
    width: 100%;
    max-width: 200px;
    height: 4px;
    background: rgba(255, 159, 10, 0.15);
    border-radius: 2px;
    overflow: hidden;
  }
  .resume-progress-fill {
    height: 100%;
    background: #f59e0b;
    border-radius: 2px;
    transition: width 300ms ease;
  }
  .resume-actions { display: flex; gap: var(--space-2); flex-shrink: 0; align-items: center; }

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
    box-shadow: var(--glass-refraction), var(--glass-edge-shadow);
    transition: border-color var(--duration-fast) var(--ease),
                box-shadow var(--duration-fast) var(--ease);
    animation: cardFadeIn var(--duration-slow) var(--ease) both;
    display: flex;
    flex-direction: column;
  }

  .collection-card:hover {
    border-color: var(--separator);
    box-shadow: var(--glass-refraction), var(--glass-edge-shadow), var(--shadow-sm);
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
  }

  .detail-revision {
    font-size: 12px;
    color: var(--text-tertiary);
    font-weight: 500;
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
    backdrop-filter: blur(40px) saturate(1.5);
    -webkit-backdrop-filter: blur(40px) saturate(1.5);
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
    min-height: 120px;
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
    padding: var(--space-2) var(--space-3);
    background: var(--bg-tertiary);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    color: var(--text-secondary);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
    white-space: nowrap;
  }

  .nsfw-cycle-btn:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
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

  .mod-download-btn {
    margin-top: var(--space-2);
    display: flex;
    align-items: center;
    gap: var(--space-1);
    width: 100%;
    justify-content: center;
  }

  /* ---- File Picker Modal ---- */

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

  /* ---- Uninstall Progress Modal ---- */

  .uninstall-overlay {
    position: fixed;
    inset: 0;
    z-index: 1000;
    background: rgba(0, 0, 0, 0.6);
    backdrop-filter: var(--glass-blur-light);
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

  /* Pre-Install Cleanup Modal */
  .cleanup-modal {
    background: var(--bg-secondary);
    border: 1px solid var(--border-primary);
    border-radius: 12px;
    width: 560px;
    max-width: 90vw;
    max-height: 80vh;
    display: flex;
    flex-direction: column;
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.5);
  }

  .cleanup-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 20px;
    border-bottom: 1px solid var(--border-primary);
  }

  .cleanup-title {
    font-size: 16px;
    font-weight: 600;
    margin: 0;
    color: var(--text-primary);
  }

  .cleanup-close {
    background: none;
    border: none;
    color: var(--text-secondary);
    font-size: 20px;
    cursor: pointer;
    padding: 4px 8px;
    border-radius: 4px;
    line-height: 1;
  }

  .cleanup-close:hover {
    background: var(--bg-tertiary);
    color: var(--text-primary);
  }

  .cleanup-body {
    padding: 20px;
    overflow-y: auto;
    flex: 1;
  }

  .cleanup-summary {
    margin-bottom: 16px;
  }

  .cleanup-info {
    font-size: 13px;
    color: var(--text-secondary);
    margin: 0 0 12px 0;
    line-height: 1.5;
  }

  .cleanup-info strong {
    color: var(--text-primary);
  }

  .cleanup-stats {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
  }

  .cleanup-stat {
    display: flex;
    flex-direction: column;
    align-items: center;
    background: var(--bg-tertiary);
    padding: 8px 16px;
    border-radius: 8px;
    min-width: 80px;
  }

  .cleanup-stat-value {
    font-size: 18px;
    font-weight: 700;
    color: var(--text-primary);
    font-family: var(--font-mono);
  }

  .cleanup-stat-label {
    font-size: 11px;
    color: var(--text-tertiary);
    margin-top: 2px;
  }

  .cleanup-options {
    margin-bottom: 16px;
  }

  .cleanup-options h4 {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    margin: 0 0 8px 0;
  }

  .cleanup-checkbox {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    color: var(--text-secondary);
    padding: 4px 0;
    cursor: pointer;
  }

  .cleanup-checkbox input[type="checkbox"] {
    accent-color: var(--system-accent);
    width: 16px;
    height: 16px;
  }

  .cleanup-advanced {
    margin-bottom: 12px;
  }

  .cleanup-advanced summary {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-secondary);
    cursor: pointer;
    padding: 4px 0;
  }

  .cleanup-advanced summary:hover {
    color: var(--text-primary);
  }

  .cleanup-hint {
    font-size: 11px;
    color: var(--text-tertiary);
    margin: 8px 0 4px 0;
  }

  .cleanup-hint code {
    background: var(--bg-tertiary);
    padding: 1px 4px;
    border-radius: 3px;
    font-size: 11px;
  }

  .cleanup-exclude-input {
    width: 100%;
    background: var(--bg-primary);
    border: 1px solid var(--border-primary);
    border-radius: 6px;
    color: var(--text-primary);
    font-family: var(--font-mono);
    font-size: 12px;
    padding: 8px;
    resize: vertical;
  }

  .cleanup-exclude-input:focus {
    outline: none;
    border-color: var(--system-accent);
  }

  .cleanup-save-notice {
    font-size: 12px;
    color: #22c55e;
    background: rgba(34, 197, 94, 0.1);
    border: 1px solid rgba(34, 197, 94, 0.2);
    border-radius: 6px;
    padding: 8px 12px;
  }

  .cleanup-footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 16px 20px;
    border-top: 1px solid var(--border-primary);
  }

  .cleanup-scanning {
    display: flex;
    align-items: center;
    gap: 10px;
    background: var(--bg-secondary);
    border: 1px solid var(--border-primary);
    border-radius: 8px;
    padding: 16px 24px;
    color: var(--text-secondary);
    font-size: 13px;
  }

  /* DLC Warning Modal */
  .dlc-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 12px;
  }

  .dlc-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    background: var(--bg-tertiary);
    border-radius: 6px;
    font-size: 13px;
  }

  .dlc-icon {
    font-size: 14px;
    width: 18px;
    text-align: center;
    flex-shrink: 0;
  }

  .dlc-present .dlc-icon {
    color: #22c55e;
  }

  .dlc-missing .dlc-icon {
    color: #ef4444;
  }

  .dlc-name {
    font-weight: 500;
    color: var(--text-primary);
  }

  .dlc-present .dlc-name {
    opacity: 0.6;
  }

  .dlc-detail {
    font-size: 11px;
    color: var(--text-tertiary);
    margin-left: auto;
    font-family: var(--font-mono);
  }

  /* ---- Optional Mod Picker ---- */

  .optional-picker-modal {
    background: color-mix(in srgb, var(--bg-grouped) 75%, transparent);
    backdrop-filter: blur(40px) saturate(1.5);
    -webkit-backdrop-filter: blur(40px) saturate(1.5);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: var(--radius-lg, 12px);
    width: min(600px, 90vw);
    max-height: 80vh;
    display: flex;
    flex-direction: column;
    box-shadow: var(--glass-refraction, none), var(--glass-edge-shadow, none), 0 8px 32px rgba(0, 0, 0, 0.4);
  }

  .optional-picker-header {
    padding: var(--space-4) var(--space-5);
    border-bottom: 1px solid var(--separator);
    flex-shrink: 0;
  }

  .optional-picker-header h3 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .optional-picker-subtitle {
    margin: 4px 0 0;
    font-size: 12px;
    color: var(--text-tertiary);
  }

  .optional-picker-body {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-3) var(--space-5);
  }

  .optional-section {
    margin-bottom: var(--space-4);
  }

  .optional-section-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--space-2);
  }

  .optional-section-label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-tertiary);
  }

  .optional-section-actions {
    display: flex;
    gap: 4px;
  }

  .optional-mod-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 6px 0;
    border-bottom: 1px solid var(--separator);
    font-size: 13px;
  }

  .optional-mod-name {
    flex: 1;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .optional-mod-version {
    color: var(--text-tertiary);
    font-size: 11px;
    flex-shrink: 0;
  }

  .optional-check {
    flex-shrink: 0;
  }

  .optional-mod-select {
    flex-shrink: 0;
    background: var(--bg-tertiary);
    border: 1px solid var(--separator);
    border-radius: var(--radius-sm);
    color: var(--text-primary);
    font-size: 11px;
    padding: 3px 8px;
    cursor: pointer;
    font-family: var(--font-sans);
  }

  .optional-picker-footer {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-5);
    border-top: 1px solid var(--separator);
    flex-shrink: 0;
  }
</style>
