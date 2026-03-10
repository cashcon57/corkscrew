<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { get } from "svelte/store";
  import { open } from "@tauri-apps/plugin-dialog";
  import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
  import ModlistImportWizard from "$lib/components/ModlistImportWizard.svelte";
  import FomodWizard from "$lib/components/FomodWizard.svelte";
  import {
    getInstalledMods,
    installMod,
    uninstallMod,
    toggleMod,
    batchToggleMods,
    launchGame,
    checkSkse,
    getSkseDownloadUrl,
    installSkseFromArchive,
    installSkseAuto,
    setSksePreference,
    checkSkyrimVersion,
    downgradeSkyrim,
    reorderMods,
    getConflicts,
    checkModUpdates,
    fixSkyrimDisplay,
    onInstallProgress,
    redeployAllMods,
    deployIncremental,
    purgeDeployment,
    getDeploymentHealth,
    getDeploymentStats,
    setModNotes,
    setModTags,
    setModPriority,
    analyzeConflicts,
    resolveAllConflicts,
    recordConflictWinner,
    onDeployProgress,
    backfillCategories,
    exportModlist,
    detectFomod,
    getFomodRecipe,
    getFomodFiles,
    saveFomodRecipe,
    detectModTools,
    launchModTool,
    getAvailableDiskSpace,
    setModCollectionName,
    listInstalledCollections,
    endorseMod,
    abstainMod,
    getUserEndorsements,
    getConfig,
    setConfigValue,
  } from "$lib/api";
  import type { InstallProgressEvent, DeploymentHealth, IncrementalDeployResult, ConflictSuggestion, ResolutionResult, DeployProgress, ModTool, UserEndorsement, IdenticalContentStats } from "$lib/types";
  import {
    selectedGame,
    installedMods,
    games,
    currentPage,
    showError,
    showSuccess,
    skseStatus,
    activeCollection,
    collectionList,
    nxmInstallComplete,
    collectionInstallStatus,
  } from "$lib/stores";
  import type { InstalledMod, DetectedGame, SkseStatus, DowngradeStatus, FileConflict, ModUpdateInfo, FomodInstaller } from "$lib/types";
  import GameIcon from "$lib/components/GameIcon.svelte";
  import DiskBudgetPanel from "$lib/components/DiskBudgetPanel.svelte";
  import PreflightPanel from "$lib/components/PreflightPanel.svelte";
  import DependencyPanel from "$lib/components/DependencyPanel.svelte";
  import SessionHistoryPanel from "$lib/components/SessionHistoryPanel.svelte";
  import ModCategoryView from "$lib/components/mods/ModCategoryView.svelte";
  import ModBisect from "$lib/components/ModBisect.svelte";
  import SkeletonRows from "$lib/components/SkeletonRows.svelte";
  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import ConflictMap from "$lib/components/ConflictMap.svelte";
  import ModDetailPanel from "$lib/components/mods/ModDetailPanel.svelte";

  // Game launch fixes opt-out (display fix + cursor clamp)
  let disableGameFixes = $state(false);

  let installing = $state(false);
  let installStep = $state("");
  let installDetail = $state("");
  let showModlistNamePrompt = $state(false);
  let modlistNameInput = $state("User");
  let pendingInstallFilePath = $state<string | null>(null);
  let loadingMods = $state(false);
  let confirmUninstall = $state<number | null>(null);
  let togglingMod = $state<number | null>(null);
  let launching = $state(false);
  let skse = $state<SkseStatus | null>(null);
  let showSksePrompt = $state(false);
  let installingSkse = $state(false);
  let showSkseMenu = $state(false);
  let showSkseInstallPrompt = $state(false);
  let downgradeStatus = $state<DowngradeStatus | null>(null);
  let downgrading = $state(false);
  let showDowngradeBanner = $state(false);
  let draggingOver = $state(false);
  let fixingDisplay = $state(false);
  let installUnlisten: (() => void) | null = null;

  // Drag reorder state
  let dragRowIndex = $state<number | null>(null);
  let dragOverIndex = $state<number | null>(null);
  let reordering = $state(false);

  // Import/export state
  let showImportWizard = $state(false);
  let exporting = $state(false);

  // FOMOD wizard state
  let showFomodWizard = $state(false);
  let fomodInstaller = $state<FomodInstaller | null>(null);
  let fomodTargetMod = $state<InstalledMod | null>(null);

  // Conflict state
  let conflicts = $state<FileConflict[]>([]);

  // Update check state
  let modUpdates = $state<ModUpdateInfo[]>([]);
  let checkingUpdates = $state(false);

  // Deploy/purge state
  let deploying = $state(false);
  let purging = $state(false);
  let deployHealth = $state<DeploymentHealth | null>(null);
  let showHealthPanel = $state(false);

  // Search/filter state
  let searchQuery = $state("");
  let filterStatus = $state<"all" | "enabled" | "disabled" | "conflicts" | "has-updates">("all");
  let filterSource = $state<"all" | "nexus" | "loverslab" | "moddb" | "curseforge" | "direct" | "manual">("all");
  let filterCollection = $state<string | null>(null);
  let sortBy = $state<"priority" | "name" | "date" | "version" | "files">("priority");
  let sortDir = $state<"asc" | "desc">("asc");
  let filterCategory = $state<string | null>(null);
  let showCategoryPopover = $state(false);

  // Persistent search: restore from sessionStorage on mount
  function searchStorageKey(game: DetectedGame) {
    return `corkscrew-search-${game.game_id}-${game.bottle_name}`;
  }
  let searchRestored = false;

  // Detail panel
  let detailMod = $state<InstalledMod | null>(null);
  let detailScrollToIni = $state<string | null>(null);

  // Conflict panel state
  let showConflictPanel = $state(false);
  let showConflictMap = $state(false);
  let makingWinner = $state<number | null>(null);

  // Mod overflow menu state
  let overflowMenuModId = $state<number | null>(null);

  // Endorsements
  let endorsements = $state<Map<number, string>>(new Map()); // mod_id -> status
  let endorsingModId = $state<number | null>(null);

  // Duplicate mod detection
  let duplicateDialog = $state<{ newMod: InstalledMod; oldMod: InstalledMod } | null>(null);

  // Proactive issue banner dismissals (per session per game)
  let dismissedBanners = $state<Set<string>>(new Set());
  function dismissBanner(key: string) {
    const next = new Set(dismissedBanners);
    next.add(key);
    dismissedBanners = next;
    if (activeGame) {
      const storeKey = `corkscrew-banners-${activeGame.game_id}-${activeGame.bottle_name}`;
      sessionStorage.setItem(storeKey, JSON.stringify([...next]));
    }
  }

  // Bulk selection state (functions defined after filteredMods)
  let selectedModIds = $state<Set<number>>(new Set());
  let suggestions = $state<ConflictSuggestion[]>([]);
  let analyzingConflicts = $state(false);
  let resolvingAll = $state(false);
  let resolutionResult = $state<ResolutionResult | null>(null);
  let identicalStats = $state<IdenticalContentStats | null>(null);

  // Selected mod for dependency panel
  let selectedModId = $state<number | undefined>(undefined);

  // Virtual scrolling for mods table
  const ROW_HEIGHT = 36;
  const SCROLL_BUFFER = 10; // extra rows above/below viewport
  let tableBodyEl = $state<HTMLDivElement | null>(null);
  let scrollTop = $state(0);
  let containerHeight = $state(600);

  // Deploy progress
  let deployProgress = $state(0);
  let deployProgressText = $state("");
  let deployUnlisten: (() => void) | null = null;

  // Keyboard focus tracking
  let focusedIndex = $state<number>(-1);

  // Context menu state
  let contextMenuMod = $state<InstalledMod | null>(null);
  let contextMenuX = $state(0);
  let contextMenuY = $state(0);

  // Collapsible panels
  let showPreflightPanel = $state(false);
  let showSessionPanel = $state(false);
  let showDependencyPanel = $state(false);
  let showBisect = $state(false);

  // Tools quick-launch
  let modTools = $state<ModTool[]>([]);
  let showToolsMenu = $state(false);
  let launchingToolId = $state<string | null>(null);
  let installedTools = $derived(modTools.filter(t => t.detected_path));

  // Resizable column widths (px) — null means flex (1fr)
  let colWidths = $state({
    name: null as number | null,    // 1fr by default
    category: 100,
    origin: 68,
    source: 110,
    version: 72,
    files: 48,
    date: 90,
    actions: 64,
  });
  let resizingCol = $state<string | null>(null);
  let resizeStartX = 0;
  let resizeStartW = 0;

  function gridCols() {
    const w = colWidths;
    const name = w.name ? `${w.name}px` : 'minmax(0, 1fr)';
    return `24px 28px 48px ${name} ${w.category}px ${w.origin}px ${w.source}px ${w.version}px ${w.files}px ${w.date}px ${w.actions}px`;
  }

  let gridTemplate = $derived(gridCols());

  function onResizeStart(e: PointerEvent, col: string) {
    e.preventDefault();
    e.stopPropagation();
    resizingCol = col;
    resizeStartX = e.clientX;
    const key = col as keyof typeof colWidths;
    if (key === 'name') {
      // Measure actual rendered width of the name column
      const headerEl = (e.target as HTMLElement).closest('.table-header');
      const nameCol = headerEl?.querySelector('.col-name') as HTMLElement | null;
      resizeStartW = nameCol?.offsetWidth ?? 300;
    } else {
      resizeStartW = colWidths[key] as number;
    }
    const onMove = (ev: PointerEvent) => {
      const delta = ev.clientX - resizeStartX;
      const newW = Math.max(32, resizeStartW + delta);
      colWidths[key] = newW;
    };
    const onUp = () => {
      resizingCol = null;
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
  }

  // Derived: set of mod IDs that have conflicts
  let conflictModIds = $derived((() => {
    const ids = new Set<number>();
    for (const conflict of conflicts) {
      for (const mod of conflict.mods) {
        ids.add(mod.mod_id);
      }
    }
    return ids;
  })());

  // Derived: map from mod_id to list of conflicting mod names (excluding self)
  let conflictDetails = $derived((() => {
    const details = new Map<number, Set<string>>();
    for (const conflict of conflicts) {
      for (const mod of conflict.mods) {
        if (!details.has(mod.mod_id)) {
          details.set(mod.mod_id, new Set());
        }
        for (const other of conflict.mods) {
          if (other.mod_id !== mod.mod_id) {
            details.get(mod.mod_id)!.add(other.mod_name);
          }
        }
      }
    }
    return details;
  })());

  // Derived: map from mod_id to ModUpdateInfo
  let updateMap = $derived((() => {
    const map = new Map<number, ModUpdateInfo>();
    for (const update of modUpdates) {
      map.set(update.mod_id, update);
    }
    return map;
  })());

  // Game picker state
  let pickedGame = $state<DetectedGame | null>(null);
  let hoveredGame = $state<string | null>(null);

  onMount(() => {
    function closeOverflow(e: MouseEvent) {
      if (overflowMenuModId !== null && !(e.target as HTMLElement)?.closest(".mod-overflow-wrap")) {
        overflowMenuModId = null;
      }
    }
    document.addEventListener("click", closeOverflow);

    // Load game fixes preference from config
    getConfig().then(cfg => {
      disableGameFixes = cfg.disable_game_fixes === "true";
    }).catch(() => {});

    return () => document.removeEventListener("click", closeOverflow);
  });

  onDestroy(() => {
    if (installUnlisten) { installUnlisten(); installUnlisten = null; }
    if (deployUnlisten) { deployUnlisten(); deployUnlisten = null; }
  });

  // View mode state
  let viewMode = $state<"flat" | "collection" | "category">(
    (() => { try { const v = localStorage.getItem("corkscrew:viewMode"); if (v === "flat" || v === "collection" || v === "category") return v; } catch {} return "flat"; })()
  );

  // Persist view mode preference
  $effect(() => { try { localStorage.setItem("corkscrew:viewMode", viewMode); } catch {} });

  // Semantic version comparison
  function compareVersions(a: string, b: string): number {
    const pa = a.split(".").map(Number);
    const pb = b.split(".").map(Number);
    for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
      const diff = (pa[i] || 0) - (pb[i] || 0);
      if (diff !== 0) return diff;
    }
    return 0;
  }

  // Toggle sort: click same column toggles direction, different column sets ascending
  function toggleSort(key: typeof sortBy) {
    if (sortBy === key) {
      sortDir = sortDir === "asc" ? "desc" : "asc";
    } else {
      sortBy = key;
      sortDir = "asc";
    }
  }

  // Sorted mods with secondary sort keys for stability
  let sortedMods = $derived((() => {
    const mods = [...$installedMods];
    const dir = sortDir === "asc" ? 1 : -1;
    mods.sort((a, b) => {
      let primary: number;
      switch (sortBy) {
        case "name":
          primary = a.name.localeCompare(b.name);
          return dir * primary || (a.install_priority - b.install_priority);
        case "date":
          primary = new Date(a.installed_at).getTime() - new Date(b.installed_at).getTime();
          return dir * primary || a.name.localeCompare(b.name);
        case "version":
          primary = compareVersions(a.version || "0", b.version || "0");
          return dir * primary || a.name.localeCompare(b.name);
        case "files":
          primary = a.installed_files.length - b.installed_files.length;
          return dir * primary || a.name.localeCompare(b.name);
        default:
          primary = a.install_priority - b.install_priority;
          return dir * primary || a.name.localeCompare(b.name);
      }
    });
    return mods;
  })());

  // Faceted search parser
  const FACET_PREFIXES = ["tag", "source", "enabled", "conflict", "update", "category", "collection", "priority", "files"] as const;
  type FacetKey = typeof FACET_PREFIXES[number];

  interface ParsedSearch {
    facets: Map<FacetKey, string>;
    freeText: string;
  }

  function parseFacets(query: string): ParsedSearch {
    const facets = new Map<FacetKey, string>();
    const freeWords: string[] = [];
    const tokens = query.match(/(?:[^\s"]+|"[^"]*")+/g) ?? [];

    for (const token of tokens) {
      const colonIdx = token.indexOf(":");
      if (colonIdx > 0) {
        const prefix = token.slice(0, colonIdx).toLowerCase();
        if (FACET_PREFIXES.includes(prefix as FacetKey)) {
          facets.set(prefix as FacetKey, token.slice(colonIdx + 1).replace(/^"|"$/g, ""));
          continue;
        }
      }
      freeWords.push(token);
    }

    return { facets, freeText: freeWords.join(" ") };
  }

  // Derived parsed search for use in UI (facet pills)
  let parsedSearch = $derived(parseFacets(searchQuery));

  // Filtered mods based on faceted search and dropdown filters — single-pass filter
  let filteredMods = $derived((() => {
    const { facets, freeText } = parsedSearch;

    // Pre-compute facet values outside the loop
    const tagFacet = facets.get("tag")?.toLowerCase() ?? null;
    const sourceFacet = facets.get("source")?.toLowerCase() ?? null;
    const enabledFacet = facets.has("enabled") ? facets.get("enabled")!.toLowerCase() === "true" : null;
    const conflictFacet = facets.get("conflict")?.toLowerCase() === "true" || false;
    const updateFacet = facets.get("update")?.toLowerCase() === "true" || false;
    const categoryFacet = facets.get("category")?.toLowerCase() ?? null;
    const collectionFacet = facets.get("collection")?.toLowerCase() ?? null;

    const priorityFacet = facets.get("priority");
    let priorityOp: ">" | "<" | null = null;
    let priorityN = 0;
    if (priorityFacet) {
      const match = priorityFacet.match(/^([><])(\d+)$/);
      if (match) { priorityOp = match[1] as ">" | "<"; priorityN = parseInt(match[2]); }
    }

    const filesFacet = facets.get("files");
    let filesOp: ">" | "<" | null = null;
    let filesN = 0;
    if (filesFacet) {
      const match = filesFacet.match(/^([><])(\d+)$/);
      if (match) { filesOp = match[1] as ">" | "<"; filesN = parseInt(match[2]); }
    }

    const q = freeText.trim() ? freeText.toLowerCase() : null;

    // Local copies of dropdown filter state for the closure
    const fStatus = filterStatus;
    const fSource = filterSource;
    const fCollection = filterCollection;
    const fCategory = filterCategory;

    return sortedMods.filter(m => {
      // Facet: tag
      if (tagFacet !== null && !m.user_tags.some(tag => tag.toLowerCase().includes(tagFacet))) return false;
      // Facet: source
      if (sourceFacet !== null && (m.source_type || "manual").toLowerCase() !== sourceFacet) return false;
      // Facet: enabled
      if (enabledFacet !== null && m.enabled !== enabledFacet) return false;
      // Facet: conflict
      if (conflictFacet && !conflictModIds.has(m.id)) return false;
      // Facet: update
      if (updateFacet && !updateMap.has(m.id)) return false;
      // Facet: category
      if (categoryFacet !== null && !m.auto_category?.toLowerCase().includes(categoryFacet)) return false;
      // Facet: collection
      if (collectionFacet !== null && !m.collection_name?.toLowerCase().includes(collectionFacet)) return false;
      // Facet: priority
      if (priorityOp === ">" && m.install_priority <= priorityN) return false;
      if (priorityOp === "<" && m.install_priority >= priorityN) return false;
      // Facet: files
      if (filesOp === ">" && m.installed_files.length <= filesN) return false;
      if (filesOp === "<" && m.installed_files.length >= filesN) return false;
      // Free text search across name, tags, notes, collection, category
      if (q !== null &&
        !m.name.toLowerCase().includes(q) &&
        !m.user_tags.some(t => t.toLowerCase().includes(q)) &&
        !(m.user_notes && m.user_notes.toLowerCase().includes(q)) &&
        !(m.collection_name && m.collection_name.toLowerCase().includes(q)) &&
        !(m.auto_category && m.auto_category.toLowerCase().includes(q))
      ) return false;
      // Dropdown: status filter
      if (fStatus === "enabled" && !m.enabled) return false;
      if (fStatus === "disabled" && m.enabled) return false;
      if (fStatus === "conflicts" && !conflictModIds.has(m.id)) return false;
      if (fStatus === "has-updates" && !updateMap.has(m.id)) return false;
      // Dropdown: source filter
      if (fSource !== "all" && (m.source_type || "manual") !== fSource) return false;
      // Dropdown: collection filter
      if (fCollection !== null) {
        if (fCollection === "__standalone__") { if (m.collection_name) return false; }
        else { if (m.collection_name !== fCollection) return false; }
      }
      // Dropdown: category filter
      if (fCategory !== null && (m.auto_category || "Miscellaneous") !== fCategory) return false;

      return true;
    });
  })());

  // Split filtered mods into enabled and disabled — single partition pass
  let disabledSectionCollapsed = $state(true);
  let partitionedFilteredMods = $derived((() => {
    if (filterStatus !== "all") return { enabled: filteredMods, disabled: [] as typeof filteredMods };
    const enabled: typeof filteredMods = [];
    const disabled: typeof filteredMods = [];
    for (const m of filteredMods) {
      if (m.enabled) enabled.push(m);
      else disabled.push(m);
    }
    return { enabled, disabled };
  })());
  let enabledFilteredMods = $derived(partitionedFilteredMods.enabled);
  let disabledFilteredMods = $derived(partitionedFilteredMods.disabled);

  // Mod stats (computed from full installed list, not filtered) — single pass
  let modStats = $derived((() => {
    let enabled = 0;
    for (const m of $installedMods) {
      if (m.enabled) enabled++;
    }
    return {
      total: $installedMods.length,
      enabled,
      disabled: $installedMods.length - enabled,
      conflicts: conflictModIds.size,
      updates: modUpdates.length,
    };
  })());

  // Pre-computed index map for O(1) lookup of mod position in filteredMods
  let filteredModIndex = $derived((() => {
    const map = new Map<number, number>();
    for (let i = 0; i < filteredMods.length; i++) {
      map.set(filteredMods[i].id, i);
    }
    return map;
  })());

  // Unique categories across all installed mods (for filter chips)
  let uniqueCategories = $derived((() => {
    const counts = new Map<string, number>();
    for (const mod of $installedMods) {
      const cat = mod.auto_category || "Miscellaneous";
      counts.set(cat, (counts.get(cat) ?? 0) + 1);
    }
    return [...counts.entries()]
      .sort(([, a], [, b]) => b - a); // most popular first
  })());

  // Virtual scrolling derived — uses enabledFilteredMods in flat view to exclude disabled mods
  let flatViewMods = $derived(viewMode === "flat" ? enabledFilteredMods : filteredMods);
  let visibleRange = $derived((() => {
    const totalItems = flatViewMods.length;
    if (totalItems === 0) return { start: 0, end: 0, paddingTop: 0, paddingBottom: 0 };
    const startRaw = Math.floor(scrollTop / ROW_HEIGHT) - SCROLL_BUFFER;
    const visibleCount = Math.ceil(containerHeight / ROW_HEIGHT) + SCROLL_BUFFER * 2;
    const start = Math.max(0, startRaw);
    const end = Math.min(totalItems, start + visibleCount);
    return {
      start,
      end,
      paddingTop: start * ROW_HEIGHT,
      paddingBottom: Math.max(0, (totalItems - end) * ROW_HEIGHT),
    };
  })());

  function handleTableScroll(e: Event) {
    const el = e.target as HTMLDivElement;
    scrollTop = el.scrollTop;
  }

  // Measure container height on mount and resize
  $effect(() => {
    if (!tableBodyEl) return;
    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) {
        containerHeight = entry.contentRect.height;
      }
    });
    ro.observe(tableBodyEl);
    containerHeight = tableBodyEl.clientHeight;
    return () => ro.disconnect();
  });

  // Bulk selection derived and functions (after filteredMods is defined)
  let selectAll = $derived(
    selectedModIds.size > 0 && filteredMods.length > 0 && selectedModIds.size >= filteredMods.length &&
    filteredMods.every(m => selectedModIds.has(m.id))
  );

  function toggleSelectAll() {
    if (selectAll) {
      selectedModIds = new Set();
    } else {
      selectedModIds = new Set(filteredMods.map(m => m.id));
    }
  }

  let lastSelectedModId = $state<number | null>(null);

  function toggleSelectMod(id: number) {
    const next = new Set(selectedModIds);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    selectedModIds = next;
    lastSelectedModId = id;
  }

  /** Handle row click with shift/cmd modifier support for multi-select. */
  function handleRowClick(e: MouseEvent, mod: InstalledMod, index: number) {
    if (e.shiftKey && lastSelectedModId !== null) {
      // Shift+click: range select from last selected to this mod
      e.preventDefault(); // Prevent text selection
      const allMods = viewMode === "flat"
        ? [...flatViewMods, ...disabledFilteredMods]
        : filteredMods;
      const lastIdx = allMods.findIndex(m => m.id === lastSelectedModId);
      const curIdx = allMods.findIndex(m => m.id === mod.id);
      if (lastIdx !== -1 && curIdx !== -1) {
        const start = Math.min(lastIdx, curIdx);
        const end = Math.max(lastIdx, curIdx);
        const next = new Set(selectedModIds);
        for (let i = start; i <= end; i++) {
          next.add(allMods[i].id);
        }
        selectedModIds = next;
      }
      lastSelectedModId = mod.id;
    } else if (e.metaKey || e.ctrlKey) {
      // Cmd/Ctrl+click: toggle individual selection without clearing others
      toggleSelectMod(mod.id);
      lastSelectedModId = mod.id;
    } else {
      // Normal click: select this mod (add to selection) and open detail
      const next = new Set(selectedModIds);
      if (next.has(mod.id) && selectedModId === mod.id) {
        // Clicking the already-active mod: deselect and close detail
        next.delete(mod.id);
        selectedModIds = next;
        selectedModId = undefined;
        detailMod = null;
      } else {
        next.add(mod.id);
        selectedModIds = next;
        selectedModId = mod.id;
        detailMod = mod;
      }
      lastSelectedModId = mod.id;
    }
    focusedIndex = index;
  }

  let bulkOperating = $state<"enabling" | "disabling" | "uninstalling" | null>(null);

  async function batchEnable() {
    if (!activeGame) return;
    bulkOperating = "enabling";
    try {
      const ids = Array.from(selectedModIds);
      await batchToggleMods(ids, activeGame.game_id, activeGame.bottle_name, true);
      selectedModIds = new Set();
      await loadMods(activeGame);
      await refreshHealth(activeGame);
    } finally {
      bulkOperating = null;
    }
  }

  async function batchDisable() {
    if (!activeGame) return;
    bulkOperating = "disabling";
    try {
      const ids = Array.from(selectedModIds);
      await batchToggleMods(ids, activeGame.game_id, activeGame.bottle_name, false);
      selectedModIds = new Set();
      await loadMods(activeGame);
      await refreshHealth(activeGame);
    } finally {
      bulkOperating = null;
    }
  }

  async function batchUninstall() {
    bulkOperating = "uninstalling";
    try {
      for (const id of selectedModIds) {
        if (activeGame) {
          await uninstallMod(id, activeGame.game_id, activeGame.bottle_name);
        }
      }
      selectedModIds = new Set();
      if (activeGame) {
        await loadMods(activeGame);
        await refreshHealth(activeGame);
      }
    } finally {
      bulkOperating = null;
    }
  }

  // Unique collection names for filter dropdown
  let collectionNames = $derived((() => {
    const names = new Set<string>();
    for (const m of $installedMods) {
      if (m.collection_name) names.add(m.collection_name);
    }
    return [...names].sort();
  })());

  // Collection-grouped mods for collection view mode
  let collapsedGroups = $state<Set<string>>(new Set());
  let groupedMods = $derived((() => {
    if (viewMode !== "collection") return null;
    const groups = new Map<string, typeof filteredMods>();
    for (const mod of filteredMods) {
      const key = mod.collection_name || "Standalone";
      if (!groups.has(key)) groups.set(key, []);
      groups.get(key)!.push(mod);
    }
    return groups;
  })());

  function toggleGroup(name: string) {
    const next = new Set(collapsedGroups);
    if (next.has(name)) next.delete(name);
    else next.add(name);
    collapsedGroups = next;
  }

  const activeGame = $derived(pickedGame ?? $selectedGame);

  // Track the current load to avoid stale race conditions
  let loadGeneration = 0;

  // Restore search + banners from sessionStorage when game changes
  $effect(() => {
    if (activeGame) {
      const saved = sessionStorage.getItem(searchStorageKey(activeGame));
      searchQuery = saved ?? "";
      searchRestored = true;
      // Restore dismissed banners
      const bannerKey = `corkscrew-banners-${activeGame.game_id}-${activeGame.bottle_name}`;
      const savedBanners = sessionStorage.getItem(bannerKey);
      dismissedBanners = savedBanners ? new Set(JSON.parse(savedBanners)) : new Set();
      // Clear selection
      selectedModIds = new Set();
      // Load mods and deployment stats in parallel
      Promise.all([loadMods(activeGame), refreshHealth(activeGame)]);
    }
  });

  // Persist search to sessionStorage
  $effect(() => {
    if (searchRestored && activeGame) {
      sessionStorage.setItem(searchStorageKey(activeGame), searchQuery);
    }
  });

  // Reload mods when an NXM install completes (triggered from layout)
  $effect(() => {
    const _count = $nxmInstallComplete;
    if (_count > 0 && activeGame) {
      loadMods(activeGame);
      refreshHealth(activeGame);
    }
  });

  async function loadMods(game: DetectedGame) {
    const thisLoad = ++loadGeneration;
    loadingMods = true;
    try {
      // Load mods first (fast DB query), then conflicts
      const [mods, newConflicts] = await Promise.all([
        getInstalledMods(game.game_id, game.bottle_name),
        getConflicts(game.game_id, game.bottle_name).catch(() => [] as import('$lib/types').FileConflict[]),
      ]);
      // Only update state if this is still the latest load request
      if (thisLoad !== loadGeneration) return;
      installedMods.set(mods);
      conflicts = newConflicts;
      // Load endorsements in background (best-effort)
      loadEndorsements(game.nexus_slug);
      // Load external tools in background — don't block the page
      detectModTools(game.game_id, game.bottle_name)
        .then(tools => { if (thisLoad === loadGeneration) modTools = tools; })
        .catch(() => {});
    } catch (e: unknown) {
      if (thisLoad !== loadGeneration) return;
      showError(`Failed to load mods: ${e}`);
    } finally {
      if (thisLoad === loadGeneration) {
        loadingMods = false;
      }
    }
  }

  async function loadEndorsements(gameSlug: string) {
    try {
      const userEndorsements = await getUserEndorsements();
      const map = new Map<number, string>();
      for (const e of userEndorsements) {
        if (e.domain_name === gameSlug) {
          map.set(e.mod_id, e.status);
        }
      }
      endorsements = map;
    } catch {
      // Endorsement loading is best-effort
    }
  }

  async function handleEndorseMod(modId: number, nexusModId: number, version: string) {
    if (!activeGame) return;
    endorsingModId = modId;
    try {
      const result = await endorseMod(activeGame.nexus_slug, nexusModId, version);
      const map = new Map(endorsements);
      map.set(nexusModId, result.status);
      endorsements = map;
      showSuccess(`Endorsed! ${result.message}`);
    } catch (e) {
      showError(`Failed to endorse: ${e}`);
    } finally {
      endorsingModId = null;
    }
  }

  async function handleAbstainMod(modId: number, nexusModId: number) {
    if (!activeGame) return;
    endorsingModId = modId;
    try {
      const result = await abstainMod(activeGame.nexus_slug, nexusModId);
      const map = new Map(endorsements);
      map.set(nexusModId, result.status);
      endorsements = map;
      showSuccess(`Endorsement removed. ${result.message}`);
    } catch (e) {
      showError(`Failed to remove endorsement: ${e}`);
    } finally {
      endorsingModId = null;
    }
  }

  const stepLabels: Record<string, string> = {
    preparing: "Preparing...",
    extracting: "Extracting archive...",
    registering: "Recording files...",
    deploying: "Deploying to game...",
    "syncing-plugins": "Syncing plugins...",
  };

  async function handleInstall() {
    const installStatus = get(collectionInstallStatus);
    if (installStatus?.active) {
      showError('Cannot modify mods while a collection is being installed');
      return;
    }
    const game = pickedGame ?? $selectedGame;
    if (!game) return;

    const filePath = await open({
      multiple: false,
      filters: [
        {
          name: "Mod Archives",
          extensions: ["zip", "7z", "rar"],
        },
      ],
    });

    if (!filePath) return;

    // If no modlist is active, prompt the user to name their current modlist
    if (!$activeCollection) {
      pendingInstallFilePath = filePath as string;
      modlistNameInput = "User";
      showModlistNamePrompt = true;
      return;
    }

    await doInstallMod(filePath as string);
  }

  async function confirmModlistName() {
    const name = modlistNameInput.trim();
    if (!name) return;
    showModlistNamePrompt = false;
    const game = pickedGame ?? $selectedGame;
    if (!game) return;
    activeCollection.set({ name, mod_count: 0, enabled_count: 0, slug: null, author: null, image_url: null, game_domain: null, installed_revision: null, original_mod_count: null, game_versions: [] });
    if (pendingInstallFilePath) {
      await doInstallMod(pendingInstallFilePath);
      pendingInstallFilePath = null;
      // Reload collections for the top bar
      try {
        const collections = await listInstalledCollections(game.game_id, game.bottle_name);
        collectionList.set(collections);
      } catch { /* non-critical */ }
    }
  }

  async function doInstallMod(filePath: string) {
    const game = pickedGame ?? $selectedGame;
    if (!game) return;

    // Check available disk space before installing
    try {
      const homeDir = filePath as string;
      const parentDir = homeDir.substring(0, homeDir.lastIndexOf("/")) || "/";
      const freeBytes = await getAvailableDiskSpace(parentDir);
      const GB = 1024 * 1024 * 1024;
      if (freeBytes < 0.5 * GB) {
        showError("Not enough disk space (< 500 MB free). Free up space before installing.");
        return;
      }
      if (freeBytes < 2 * GB) {
        showError("Low disk space warning: less than 2 GB free. Install will proceed, but consider freeing space.");
      }
    } catch {
      // Non-critical — proceed even if space check fails
    }

    installing = true;
    installStep = "preparing";
    installDetail = "";

    // Subscribe to progress events
    try {
      installUnlisten = await onInstallProgress((event: InstallProgressEvent) => {
        if (event.kind === "stepChanged") {
          installStep = event.step;
          installDetail = event.detail ?? "";
        } else if (event.kind === "modCompleted") {
          installStep = "complete";
          installDetail = "";
        } else if (event.kind === "modFailed") {
          installStep = "failed";
          installDetail = event.error;
        }
      });

      const mod = await installMod(
        filePath,
        game.game_id,
        game.bottle_name
      );
      const installed = mod as InstalledMod;

      // Associate mod with the active modlist
      if ($activeCollection) {
        try {
          await setModCollectionName(installed.id, $activeCollection.name);
        } catch { /* non-critical */ }
      }

      showSuccess(`Installed "${installed.name}" successfully`);
      await loadMods(game);

      // Check for duplicate mod (same nexus_mod_id)
      if (installed.nexus_mod_id) {
        const existing = $installedMods.find(
          m => m.nexus_mod_id === installed.nexus_mod_id && m.id !== installed.id
        );
        if (existing) {
          duplicateDialog = { newMod: installed, oldMod: existing };
        }
      }

      // Auto-detect FOMOD after install
      if (installed.staging_path) {
        try {
          const installer = await detectFomod(installed.staging_path);
          if (installer) {
            fomodInstaller = installer;
            fomodTargetMod = installed;
            showFomodWizard = true;
          }
        } catch {
          // FOMOD detection is optional, don't show errors
        }
      }
    } catch (e: unknown) {
      showError(`Install failed: ${e}`);
    } finally {
      installing = false;
      installStep = "";
      installDetail = "";
      if (installUnlisten) { installUnlisten(); installUnlisten = null; }
    }
  }

  async function handleUninstall(modId: number) {
    const installStatus = get(collectionInstallStatus);
    if (installStatus?.active) {
      showError('Cannot modify mods while a collection is being installed');
      return;
    }
    const game = pickedGame ?? $selectedGame;
    if (!game) return;

    try {
      const removed = await uninstallMod(modId, game.game_id, game.bottle_name);
      showSuccess(`Uninstalled — ${(removed as string[]).length} files removed`);
      confirmUninstall = null;
      await loadMods(game);
      await refreshHealth(game);
    } catch (e: unknown) {
      showError(`Uninstall failed: ${e}`);
    }
  }

  async function handleToggle(mod: InstalledMod) {
    const installStatus = get(collectionInstallStatus);
    if (installStatus?.active) {
      showError('Cannot modify mods while a collection is being installed');
      return;
    }
    const game = pickedGame ?? $selectedGame;
    if (!game) return;
    togglingMod = mod.id;
    const newEnabled = !mod.enabled;

    // Optimistic UI update — flip immediately so the toggle feels instant
    mod.enabled = newEnabled;
    installedMods.set($installedMods);

    try {
      await toggleMod(mod.id, game.game_id, game.bottle_name, newEnabled);
      // Refresh data in parallel, non-blocking (UI already updated)
      Promise.all([loadMods(game), refreshHealth(game)]).catch(() => {});
    } catch (e: unknown) {
      // Revert optimistic update on failure
      mod.enabled = !newEnabled;
      installedMods.set($installedMods);
      showError(`Failed to toggle mod: ${e}`);
    } finally {
      togglingMod = null;
    }
  }

  function selectGameForMods(game: DetectedGame) {
    pickedGame = game;
    selectedGame.set(game);
  }

  // SKSE & version detection
  let skseCheckGeneration = 0;

  $effect(() => {
    const game = activeGame;
    const gen = ++skseCheckGeneration;
    if (game && game.game_id === "skyrimse") {
      checkSkseStatus(game, gen);
      checkVersionStatus(game, gen);
    } else {
      skse = null;
      showSksePrompt = false;
      downgradeStatus = null;
      showDowngradeBanner = false;
    }
  });

  async function checkSkseStatus(game: DetectedGame, gen: number) {
    try {
      const status = await checkSkse(game.game_id, game.bottle_name);
      if (gen !== skseCheckGeneration) return; // stale
      skse = status;
      skseStatus.set(skse);
      if (!skse.installed) {
        const dismissed = localStorage.getItem(`skse_dismissed:${game.game_id}:${game.bottle_name}`);
        if (!dismissed) showSksePrompt = true;
      }
    } catch {
      // Non-critical
    }
  }

  async function handlePlay() {
    const game = pickedGame ?? $selectedGame;
    if (!game) return;

    const wantsSkse = !!(skse?.use_skse && game.game_id === "skyrimse");

    if (wantsSkse && !skse?.installed) {
      // SKSE preference is on but not installed — prompt to install
      showSkseInstallPrompt = true;
      return;
    }

    doLaunch(wantsSkse);
  }

  async function doLaunch(useSkse: boolean) {
    const game = pickedGame ?? $selectedGame;
    if (!game) return;
    launching = true;
    try {
      const result = await launchGame(game.game_id, game.bottle_name, useSkse);
      if (result.success) {
        showSuccess(`Launched ${game.display_name}${useSkse ? " via SKSE" : ""} — Wine cursor fix applied`);
        if (result.warning) {
          showError(`SKSE warning: ${result.warning}`);
        }
      }
    } catch (e: unknown) {
      showError(`Failed to launch: ${e}`);
    } finally {
      launching = false;
    }
  }

  async function toggleGameFixes() {
    disableGameFixes = !disableGameFixes;
    try {
      await setConfigValue("disable_game_fixes", disableGameFixes ? "true" : "false");
    } catch { /* best-effort */ }
  }

  async function handleFixDisplay() {
    const game = pickedGame ?? $selectedGame;
    if (!game) return;
    fixingDisplay = true;
    try {
      const result = await fixSkyrimDisplay(game.bottle_name);
      if (result.fixed) {
        showSuccess(`Display fixed: ${result.applied.width}x${result.applied.height} fullscreen — game will open in its own Space (swipe to switch)`);
      } else {
        showSuccess(`Display settings already correct: ${result.applied.width}x${result.applied.height}`);
      }
    } catch (e: unknown) {
      showError(`Display fix failed: ${e}`);
    } finally {
      fixingDisplay = false;
    }
  }

  async function handleOpenSkseDownload() {
    try {
      const url = await getSkseDownloadUrl();
      await openUrl(url);
    } catch (e: unknown) {
      showError(`Failed to open SKSE download page: ${e}`);
    }
  }

  async function handleInstallSkse() {
    const game = pickedGame ?? $selectedGame;
    if (!game) return;
    try {
      const selected = await open({
        title: "Select SKSE Archive (.7z or .zip)",
        filters: [{ name: "Archives", extensions: ["7z", "zip"] }],
      });
      if (!selected) return;

      const archivePath = typeof selected === "string" ? selected : (selected as any).path;
      installingSkse = true;
      skse = await installSkseFromArchive(game.game_id, game.bottle_name, archivePath);
      skseStatus.set(skse);
      showSksePrompt = false;
      showSuccess("SKSE installed successfully");
    } catch (e: unknown) {
      showError(`SKSE installation failed: ${e}`);
    } finally {
      installingSkse = false;
    }
  }

  async function handleAutoInstallSkse() {
    const game = pickedGame ?? $selectedGame;
    if (!game) return;
    try {
      installingSkse = true;
      skse = await installSkseAuto(game.game_id, game.bottle_name);
      skseStatus.set(skse);
      showSksePrompt = false;
      showSuccess("SKSE auto-installed successfully");
    } catch (e: unknown) {
      showError(`SKSE auto-install failed: ${e}`);
    } finally {
      installingSkse = false;
    }
  }

  async function handleInstallSkseAndLaunch() {
    const game = pickedGame ?? $selectedGame;
    if (!game) return;
    try {
      installingSkse = true;
      showSkseInstallPrompt = false;
      skse = await installSkseAuto(game.game_id, game.bottle_name);
      skseStatus.set(skse);
      showSksePrompt = false;
      showSuccess("SKSE installed — launching game");
      doLaunch(true);
    } catch (e: unknown) {
      showError(`SKSE auto-install failed: ${e}`);
    } finally {
      installingSkse = false;
    }
  }

  async function handleInstallSkseArchiveAndLaunch() {
    const game = pickedGame ?? $selectedGame;
    if (!game) return;
    try {
      const selected = await open({
        title: "Select SKSE Archive (.7z or .zip)",
        filters: [{ name: "Archives", extensions: ["7z", "zip"] }],
      });
      if (!selected) return;

      const archivePath = typeof selected === "string" ? selected : (selected as any).path;
      installingSkse = true;
      showSkseInstallPrompt = false;
      skse = await installSkseFromArchive(game.game_id, game.bottle_name, archivePath);
      skseStatus.set(skse);
      showSksePrompt = false;
      showSuccess("SKSE installed — launching game");
      doLaunch(true);
    } catch (e: unknown) {
      showError(`SKSE installation failed: ${e}`);
    } finally {
      installingSkse = false;
    }
  }

  async function checkVersionStatus(game: DetectedGame, gen: number) {
    try {
      const status = await checkSkyrimVersion(game.game_id, game.bottle_name);
      if (gen !== skseCheckGeneration) return; // stale
      downgradeStatus = status;
      if (!status.is_downgraded) {
        const dismissed = localStorage.getItem(`downgrade_dismissed:${game.game_id}:${game.bottle_name}`);
        if (!dismissed) showDowngradeBanner = true;
      }
    } catch {
      // Non-critical
    }
  }

  async function handleDowngrade() {
    const game = pickedGame ?? $selectedGame;
    if (!game) return;
    downgrading = true;
    try {
      const status = await downgradeSkyrim(game.game_id, game.bottle_name, "full");
      downgradeStatus = status;
      showDowngradeBanner = false;
      showSuccess(`Game downgraded to v${status.target_version}`);
    } catch (e: unknown) {
      showError(`Downgrade failed: ${e}`);
    } finally {
      downgrading = false;
    }
  }

  function dismissDowngradeBanner() {
    const game = pickedGame ?? $selectedGame;
    if (game) {
      localStorage.setItem(`downgrade_dismissed:${game.game_id}:${game.bottle_name}`, "true");
    }
    showDowngradeBanner = false;
  }

  // Drag-and-drop mod install
  function handleDragOver(e: DragEvent) {
    // Only activate the file drop overlay if files are being dragged (not row reorder)
    if (dragRowIndex !== null) return;
    e.preventDefault();
    draggingOver = true;
  }

  function handleDragLeave() {
    draggingOver = false;
  }

  async function handleDrop(e: DragEvent) {
    // Don't intercept row reorder drops
    if (dragRowIndex !== null) return;
    e.preventDefault();
    draggingOver = false;
    // Prevent concurrent installs
    if (installing) return;
    const game = pickedGame ?? $selectedGame;
    if (!game || !e.dataTransfer?.files?.length) return;

    const file = e.dataTransfer.files[0];
    const ext = file.name.split(".").pop()?.toLowerCase();
    if (!ext || !["zip", "7z", "rar"].includes(ext)) {
      showError("Unsupported file type. Use .zip, .7z, or .rar archives.");
      return;
    }

    // Tauri needs the file path from the drop event
    // dataTransfer.files[0].path is available in Tauri webview
    const filePath = (file as any).path;
    if (!filePath) {
      showError("Could not read file path from drop event.");
      return;
    }

    installing = true;
    installStep = "preparing";
    installDetail = "";

    try {
      installUnlisten = await onInstallProgress((event: InstallProgressEvent) => {
        if (event.kind === "stepChanged") {
          installStep = event.step;
          installDetail = event.detail ?? "";
        } else if (event.kind === "modCompleted") {
          installStep = "complete";
          installDetail = "";
        } else if (event.kind === "modFailed") {
          installStep = "failed";
          installDetail = event.error;
        }
      });

      const mod = await installMod(filePath, game.game_id, game.bottle_name);
      const installed = mod as InstalledMod;
      showSuccess(`Installed "${installed.name}" successfully`);
      await loadMods(game);

      // Auto-detect FOMOD after drag-and-drop install
      if (installed.staging_path) {
        try {
          const installer = await detectFomod(installed.staging_path);
          if (installer) {
            fomodInstaller = installer;
            fomodTargetMod = installed;
            showFomodWizard = true;
          }
        } catch {
          // FOMOD detection is optional
        }
      }
    } catch (e: unknown) {
      showError(`Install failed: ${e}`);
    } finally {
      installing = false;
      installStep = "";
      installDetail = "";
      if (installUnlisten) { installUnlisten(); installUnlisten = null; }
    }
  }

  function dismissSksePrompt() {
    const game = pickedGame ?? $selectedGame;
    if (game) {
      localStorage.setItem(`skse_dismissed:${game.game_id}:${game.bottle_name}`, "true");
    }
    showSksePrompt = false;
  }

  async function toggleSksePreference() {
    const game = pickedGame ?? $selectedGame;
    if (!game || !skse) return;
    const newValue = !skse.use_skse;
    try {
      await setSksePreference(game.game_id, game.bottle_name, newValue);
      skse = { ...skse, use_skse: newValue };
      skseStatus.set(skse);
    } catch (e: unknown) {
      showError(`Failed to update SKSE preference: ${e}`);
    }
    showSkseMenu = false;
  }

  function formatDate(iso: string): string {
    return new Date(iso).toLocaleDateString(undefined, {
      month: "short",
      day: "numeric",
      year: "numeric",
    });
  }

  function getModSourceUrl(mod: InstalledMod): string | null {
    if (mod.source_url) return mod.source_url;
    if (mod.nexus_mod_id && activeGame) {
      return `https://www.nexusmods.com/${activeGame.nexus_slug}/mods/${mod.nexus_mod_id}`;
    }
    return null;
  }

  const SOURCE_LABELS: Record<string, string> = {
    nexus: "Nexus",
    loverslab: "LoversLab",
    moddb: "ModDB",
    curseforge: "CurseForge",
    github: "GitHub",
    mega: "Mega",
    google_drive: "Google Drive",
    mediafire: "MediaFire",
    direct: "Direct",
    manual: "Manual",
  };

  function originLabel(sourceType: string): string {
    return SOURCE_LABELS[sourceType] ?? sourceType;
  }

  async function handleLaunchTool(toolId: string) {
    const game = pickedGame ?? $selectedGame;
    if (!game) return;
    launchingToolId = toolId;
    showToolsMenu = false;
    try {
      await launchModTool(toolId, game.game_id, game.bottle_name);
    } catch (e: unknown) {
      showError(`Failed to launch tool: ${e}`);
    } finally {
      launchingToolId = null;
    }
  }

  // --- Drag reorder handlers ---
  function handleRowDragStart(e: DragEvent, index: number) {
    dragRowIndex = index;
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = "move";
      // Set minimal drag data so HTML5 DnD works
      e.dataTransfer.setData("text/plain", String(index));
    }
    // Add a small delay so the browser captures the row as drag image
    requestAnimationFrame(() => {
      // The row being dragged gets a class via dragRowIndex
    });
  }

  function handleRowDragOver(e: DragEvent, index: number) {
    if (dragRowIndex === null) return;
    e.preventDefault();
    if (e.dataTransfer) {
      e.dataTransfer.dropEffect = "move";
    }
    dragOverIndex = index;
  }

  function handleRowDragEnd() {
    dragRowIndex = null;
    dragOverIndex = null;
  }

  async function handleRowDrop(e: DragEvent, dropIndex: number) {
    e.preventDefault();
    e.stopPropagation();
    const installStatus = get(collectionInstallStatus);
    if (installStatus?.active) {
      showError('Cannot modify mods while a collection is being installed');
      dragRowIndex = null;
      dragOverIndex = null;
      return;
    }
    if (dragRowIndex === null || dragRowIndex === dropIndex) {
      dragRowIndex = null;
      dragOverIndex = null;
      return;
    }

    const game = pickedGame ?? $selectedGame;
    if (!game) {
      dragRowIndex = null;
      dragOverIndex = null;
      return;
    }

    // Reorder the local array
    const items = [...sortedMods];
    const [moved] = items.splice(dragRowIndex, 1);
    items.splice(dropIndex, 0, moved);

    // Persist new order
    const orderedIds = items.map((m) => m.id);

    dragRowIndex = null;
    dragOverIndex = null;
    reordering = true;

    try {
      await reorderMods(game.game_id, game.bottle_name, orderedIds);
      await loadMods(game);
    } catch (e: unknown) {
      showError(`Failed to reorder mods: ${e}`);
    } finally {
      reordering = false;
    }
  }

  // --- Check for updates ---
  async function handleCheckUpdates() {
    const game = pickedGame ?? $selectedGame;
    if (!game) return;
    checkingUpdates = true;
    try {
      modUpdates = await checkModUpdates(game.game_id, game.bottle_name);
      if (modUpdates.length === 0) {
        showSuccess("All mods are up to date");
      } else {
        showSuccess(`${modUpdates.length} update${modUpdates.length > 1 ? "s" : ""} available`);
      }
    } catch (e: unknown) {
      showError(`Failed to check for updates: ${e}`);
    } finally {
      checkingUpdates = false;
    }
  }

  // --- Batch Update All ---
  async function handleUpdateAll() {
    const game = pickedGame ?? $selectedGame;
    if (!game || modUpdates.length === 0) return;
    // NexusMods compliance: open browser pages for each update
    // Free users must manually download from the Nexus website
    const gameSlug = game.nexus_slug || game.game_id;
    let opened = 0;
    for (const update of modUpdates) {
      try {
        await openUrl(`https://www.nexusmods.com/${gameSlug}/mods/${update.nexus_mod_id}?tab=files`);
        opened++;
      } catch {
        // Skip mods that fail to open
      }
    }
    if (opened > 0) {
      showSuccess(`Opened ${opened} Nexus mod page${opened !== 1 ? "s" : ""} for updating`);
    }
  }

  // --- FOMOD Reconfigure ---
  async function handleReconfigureFomod(mod: InstalledMod) {
    if (!mod.staging_path) return;
    try {
      const installer = await detectFomod(mod.staging_path);
      if (!installer) {
        showError("No FOMOD installer found in this mod's staging folder.");
        return;
      }
      // Load previous recipe to pre-populate selections
      const recipe = await getFomodRecipe(mod.id);
      if (recipe) {
        // Pre-apply saved selections into the installer (the wizard's loadDefaults will handle this)
      }
      fomodInstaller = installer;
      fomodTargetMod = mod;
      showFomodWizard = true;
    } catch (e: unknown) {
      showError(`Failed to detect FOMOD: ${e}`);
    }
  }

  async function handleFomodComplete(selections: Record<string, string[]>) {
    const game = pickedGame ?? $selectedGame;
    if (!game || !fomodTargetMod || !fomodInstaller) return;
    if (deploying) return;
    showFomodWizard = false;
    deploying = true;
    try {
      // Get the files for the new selections
      const files = await getFomodFiles(fomodInstaller, selections);
      // Save the recipe
      await saveFomodRecipe(fomodTargetMod.id, fomodTargetMod.name, "", selections);
      // Redeploy to apply changes
      await redeployAllMods(game.game_id, game.bottle_name);
      await loadMods(game);
      await refreshHealth(game);
      showSuccess(`Reconfigured FOMOD for "${fomodTargetMod.name}"`);
    } catch (e: unknown) {
      showError(`Failed to apply FOMOD configuration: ${e}`);
    } finally {
      deploying = false;
      fomodInstaller = null;
      fomodTargetMod = null;
    }
  }

  // --- Import / Export ---
  async function handleExport() {
    const game = pickedGame ?? $selectedGame;
    if (!game) return;
    exporting = true;
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const outputPath = await save({
        filters: [{ name: "Mod List", extensions: ["json"] }],
        defaultPath: `${game.display_name.replace(/[^a-zA-Z0-9]/g, "_")}_modlist.json`,
      });
      if (!outputPath) return;
      await exportModlist(game.game_id, game.bottle_name, outputPath);
      showSuccess(`Mod list exported to ${outputPath}`);
    } catch (e: unknown) {
      showError(`Export failed: ${e}`);
    } finally {
      exporting = false;
    }
  }

  // --- Deploy / Purge / Health ---
  async function handleDeploy() {
    const installStatus = get(collectionInstallStatus);
    if (installStatus?.active) {
      showError('Cannot modify mods while a collection is being installed');
      return;
    }
    const game = pickedGame ?? $selectedGame;
    if (!game) return;
    deploying = true;
    deployProgress = 0;
    deployProgressText = "Computing diff...";
    try {
      // Try incremental deployment first (much faster for small changes)
      const incrResult = await deployIncremental(game.game_id, game.bottle_name);
      const totalChanged = incrResult.files_added + incrResult.files_removed + incrResult.files_updated;
      if (incrResult.fallback_used) {
        // Incremental fell back to full redeploy internally
        showSuccess(`Deployed ${incrResult.files_added} files (full redeploy)${incrResult.fallback_used ? " (copy fallback used)" : ""}`);
      } else if (totalChanged === 0) {
        showSuccess("Deployment is already up to date");
      } else {
        const parts: string[] = [];
        if (incrResult.files_added > 0) parts.push(`${incrResult.files_added} added`);
        if (incrResult.files_updated > 0) parts.push(`${incrResult.files_updated} updated`);
        if (incrResult.files_removed > 0) parts.push(`${incrResult.files_removed} removed`);
        parts.push(`${incrResult.files_unchanged} unchanged`);
        showSuccess(`Incremental deploy: ${parts.join(", ")}`);
      }
      if (incrResult.verification_failures.length > 0) {
        showError(`${incrResult.verification_failures.length} file(s) failed to deploy`);
      }
      await refreshHealth(game);
    } catch {
      // Incremental failed — fall back to full redeploy
      deployProgressText = "Falling back to full deploy...";
      try {
        deployUnlisten = await onDeployProgress((p: DeployProgress) => {
          if (p.total_files > 0) {
            deployProgress = Math.round((p.files_deployed / p.total_files) * 100);
            deployProgressText = `${p.mod_name} (${p.files_deployed}/${p.total_files} files)`;
          } else {
            deployProgress = p.total > 0 ? Math.round((p.current / p.total) * 100) : 0;
            deployProgressText = `${p.current}/${p.total} ${p.mod_name}`;
          }
        });
        const result = await redeployAllMods(game.game_id, game.bottle_name);
        showSuccess(`Deployed ${result.deployed_count} files (full redeploy)${result.fallback_used ? " (copy fallback used)" : ""}`);
        await refreshHealth(game);
      } catch (e2: unknown) {
        showError(`Deploy failed: ${e2}`);
      }
    } finally {
      deploying = false;
      deployProgress = 0;
      deployProgressText = "";
      if (deployUnlisten) { deployUnlisten(); deployUnlisten = null; }
    }
  }

  async function handlePurge() {
    const game = pickedGame ?? $selectedGame;
    if (!game) return;
    purging = true;
    try {
      const removed = await purgeDeployment(game.game_id, game.bottle_name);
      showSuccess(`Purged ${removed.length} deployed files`);
      await refreshHealth(game);
    } catch (e: unknown) {
      showError(`Purge failed: ${e}`);
    } finally {
      purging = false;
    }
  }

  async function refreshHealth(game: DetectedGame) {
    try {
      // Use lightweight stats (skips expensive find_all_conflicts — conflicts
      // are already loaded by loadMods → getConflicts)
      const stats = await getDeploymentStats(game.game_id, game.bottle_name);
      deployHealth = { ...stats, conflict_count: conflicts.length };
    } catch {
      deployHealth = null;
    }
  }

  // Health is now loaded in parallel with mods in the activeGame $effect above

  // --- Notes ---
  async function handleSaveNotes(modId: number, value: string) {
    try {
      await setModNotes(modId, value || null);
      const game = pickedGame ?? $selectedGame;
      if (game) await loadMods(game);
    } catch (e: unknown) {
      showError(`Failed to save notes: ${e}`);
    }
  }

  async function handleInstallOverMod(mod: InstalledMod) {
    // Re-install: open file picker and install over the same mod
    showSuccess(`Select a new archive to reinstall "${mod.name}"`);
    await handleInstall();
  }

  async function handleCheckSingleUpdate(mod: InstalledMod) {
    const game = pickedGame ?? $selectedGame;
    if (!game || !mod.nexus_mod_id) return;
    try {
      const updates = await checkModUpdates(game.game_id, game.bottle_name);
      const update = updates.find(u => u.mod_id === mod.id);
      if (update) {
        showSuccess(`Update available for "${mod.name}": v${update.latest_version}`);
      } else {
        showSuccess(`"${mod.name}" is up to date`);
      }
    } catch (e: unknown) {
      showError(`Update check failed: ${e}`);
    }
  }

  async function handleMakeWinner(conflict: FileConflict, modId: number) {
    const game = pickedGame ?? $selectedGame;
    if (!game) return;
    if (deploying) return;
    makingWinner = modId;
    deploying = true;
    try {
      // Find current winner's priority and set this mod 1 higher
      const winner = conflict.mods.find(m => m.mod_id === conflict.winner_mod_id);
      const newPriority = winner ? winner.priority + 1 : 999;
      await setModPriority(modId, newPriority);
      // Record this resolution so the conflict disappears from the list
      const loserIds = conflict.mods
        .filter(m => m.mod_id !== modId)
        .map(m => m.mod_id);
      await recordConflictWinner(game.game_id, game.bottle_name, modId, loserIds);
      await redeployAllMods(game.game_id, game.bottle_name);
      await loadMods(game);
      await refreshHealth(game);
    } catch (e: unknown) {
      showError(`Failed to set winner: ${e}`);
    } finally {
      deploying = false;
      makingWinner = null;
    }
  }

  async function handleAnalyzeConflicts() {
    const game = pickedGame ?? $selectedGame;
    if (!game) return;
    analyzingConflicts = true;
    resolutionResult = null;
    identicalStats = null;
    try {
      const response = await analyzeConflicts(game.game_id, game.bottle_name);
      suggestions = response.suggestions;
      identicalStats = response.identical_stats;
    } catch (e: unknown) {
      showError(`Conflict analysis failed: ${e}`);
    } finally {
      analyzingConflicts = false;
    }
  }

  async function handleMagicResolve() {
    const game = pickedGame ?? $selectedGame;
    if (!game) return;
    resolvingAll = true;
    resolutionResult = null;
    identicalStats = null;
    try {
      resolutionResult = await resolveAllConflicts(game.game_id, game.bottle_name);
      await loadMods(game);
      await refreshHealth(game);
      // Re-analyze to show updated state
      const response = await analyzeConflicts(game.game_id, game.bottle_name);
      suggestions = response.suggestions;
      identicalStats = response.identical_stats;
      const autoCount = resolutionResult.author_resolved + resolutionResult.auto_suggested + resolutionResult.identical_content;
      showSuccess(`Resolved ${autoCount} conflicts automatically`);
    } catch (e: unknown) {
      showError(`Magic resolver failed: ${e}`);
    } finally {
      resolvingAll = false;
    }
  }

  function getConflictTooltip(modId: number): string {
    const names = conflictDetails.get(modId);
    if (!names || names.size === 0) return "File conflicts detected";
    return `File conflicts with: ${[...names].join(", ")}`;
  }

  const modCount = $derived($installedMods.length);
  const enabledCount = $derived(modStats.enabled);

  // Keyboard shortcuts
  function handleKeydown(e: KeyboardEvent) {
    const isCmd = e.metaKey || e.ctrlKey;
    const target = e.target as HTMLElement;
    // Don't intercept if user is typing in an input
    if (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.tagName === "SELECT") return;

    if (isCmd && e.key === "f") {
      e.preventDefault();
      const input = document.querySelector<HTMLInputElement>(".search-input");
      if (input) input.focus();
      return;
    }
    if (isCmd && e.key === "d") {
      e.preventDefault();
      handleDeploy();
      return;
    }
    if (e.key === "Escape") {
      if (contextMenuMod) { contextMenuMod = null; return; }
      if (detailMod) { detailMod = null; selectedModId = undefined; return; }
      focusedIndex = -1;
      return;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (focusedIndex < filteredMods.length - 1) focusedIndex++;
      scrollFocusedIntoView();
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      if (focusedIndex > 0) focusedIndex--;
      scrollFocusedIntoView();
      return;
    }
    if (e.key === " " && focusedIndex >= 0 && focusedIndex < filteredMods.length) {
      e.preventDefault();
      handleToggle(filteredMods[focusedIndex]);
      return;
    }
    if (e.key === "Enter" && focusedIndex >= 0 && focusedIndex < filteredMods.length) {
      e.preventDefault();
      const mod = filteredMods[focusedIndex];
      selectedModId = mod.id;
      detailMod = mod;
      return;
    }
    if (isCmd && e.key === "a") {
      e.preventDefault();
      if (selectAll) {
        selectedModIds = new Set();
      } else {
        selectedModIds = new Set(filteredMods.map(m => m.id));
      }
      return;
    }
    if ((e.key === "Delete" || e.key === "Backspace") && selectedModIds.size > 0) {
      e.preventDefault();
      batchUninstall();
      return;
    }
  }

  function scrollFocusedIntoView() {
    if (!tableBodyEl || focusedIndex < 0) return;
    const rowTop = focusedIndex * ROW_HEIGHT;
    const rowBottom = rowTop + ROW_HEIGHT;
    const viewTop = tableBodyEl.scrollTop;
    const viewBottom = viewTop + tableBodyEl.clientHeight;
    if (rowTop < viewTop) {
      tableBodyEl.scrollTop = rowTop;
    } else if (rowBottom > viewBottom) {
      tableBodyEl.scrollTop = rowBottom - tableBodyEl.clientHeight;
    }
  }

  // Context menu handler
  function handleRowContextMenu(e: MouseEvent, mod: InstalledMod) {
    e.preventDefault();
    e.stopPropagation();
    contextMenuMod = mod;
    contextMenuX = e.clientX;
    contextMenuY = e.clientY;
  }

  function closeContextMenu() {
    contextMenuMod = null;
  }

  function escapeHtml(s: string): string {
    return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  }

  function highlightMatch(text: string, query: string): string {
    if (!query) return escapeHtml(text);
    const escaped = query.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    return escapeHtml(text).replace(new RegExp(`(${escaped})`, "gi"), "<mark>$1</mark>");
  }

  // Category colors — intuitive color associations
  const categoryColors: Record<string, string> = {
    "Gameplay":          "#6366f1",  // Indigo — core game content
    "Texture":           "#22c55e",  // Green — visual surfaces
    "Model & Mesh":      "#f59e0b",  // Amber — 3D shapes
    "Framework":         "#ef4444",  // Red — critical infrastructure
    "Audio":             "#8b5cf6",  // Violet — sound/music
    "UI":                "#06b6d4",  // Cyan — interface elements
    "Script":            "#f97316",  // Orange — code/logic
    "Lighting & Weather":"#ec4899",  // Pink — ENB/ReShade/atmosphere
    "Animation":         "#3b82f6",  // Blue — motion/movement
    "Misc":              "#6b7280",  // Gray — uncategorized
  };

  // Category SVG icon paths (monochrome, 16x16 viewBox="0 0 24 24")
  const categoryIcons: Record<string, string> = {
    // Sword/shield — core gameplay content
    "Gameplay": '<path d="M14.5 17.5L3 6V3h3l11.5 11.5" /><path d="M13 19l6-6" /><path d="M16 16l4 4" /><path d="M19 21a2 2 0 0 0 2-2" />',
    // Image/texture icon
    "Texture": '<rect x="3" y="3" width="18" height="18" rx="2" /><circle cx="8.5" cy="8.5" r="1.5" /><path d="m21 15-5-5L5 21" />',
    // Wireframe cube for 3D models
    "Model & Mesh": '<path d="M12 2 2 7l10 5 10-5-10-5z" /><path d="m2 17 10 5 10-5" /><path d="m2 12 10 5 10-5" />',
    // Wrench/cog — critical framework (SKSE etc.)
    "Framework": '<path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z" />',
    // Music note
    "Audio": '<path d="M9 18V5l12-2v13" /><circle cx="6" cy="18" r="3" /><circle cx="18" cy="16" r="3" />',
    // Layout/panel for UI
    "UI": '<rect x="3" y="3" width="18" height="18" rx="2" /><line x1="3" y1="9" x2="21" y2="9" /><line x1="9" y1="21" x2="9" y2="9" />',
    // Code brackets for scripts
    "Script": '<polyline points="16 18 22 12 16 6" /><polyline points="8 6 2 12 8 18" />',
    // Sun/palette for ENB/ReShade/lighting
    "Lighting & Weather": '<circle cx="12" cy="12" r="4" /><path d="M12 2v2" /><path d="M12 20v2" /><path d="m4.93 4.93 1.41 1.41" /><path d="m17.66 17.66 1.41 1.41" /><path d="M2 12h2" /><path d="M20 12h2" /><path d="m6.34 17.66-1.41 1.41" /><path d="m19.07 4.93-1.41 1.41" />',
    // Running figure for animation
    "Animation": '<path d="M13 4a1.5 1.5 0 1 1 3 0 1.5 1.5 0 0 1-3 0z" /><path d="M7 21l3-9 2.5 2v7" /><path d="M17 14l-3-3-3 3-2-4 5-3 3 2z" />',
    // Puzzle piece for misc
    "Misc": '<path d="M19.439 7.85c-.049.322.059.648.289.878l1.568 1.568c.47.47.706 1.087.706 1.704s-.235 1.233-.706 1.704l-1.611 1.611a.98.98 0 0 1-.837.276c-.47-.07-.802-.48-.968-.925a2.501 2.501 0 1 0-3.214 3.214c.446.166.855.497.925.968a.979.979 0 0 1-.276.837l-1.61 1.611a2.404 2.404 0 0 1-1.705.706 2.404 2.404 0 0 1-1.704-.706l-1.568-1.568a1.026 1.026 0 0 0-.877-.29c-.493.074-.84.504-1.02.968a2.5 2.5 0 1 1-3.237-3.237c.464-.18.894-.527.967-1.02a1.026 1.026 0 0 0-.289-.877l-1.568-1.568A2.404 2.404 0 0 1 1.998 12c0-.617.236-1.234.706-1.704L4.315 8.685a.98.98 0 0 1 .837-.276c.47.07.802.48.968.925a2.501 2.501 0 1 0 3.214-3.214c-.446-.166-.855-.497-.925-.968a.979.979 0 0 1 .276-.837l1.611-1.611a2.404 2.404 0 0 1 1.704-.706c.617 0 1.234.236 1.704.706l1.568 1.568c.23.23.556.338.877.29.493-.074.84-.504 1.02-.968a2.5 2.5 0 1 1 3.237 3.237c-.464.18-.894.527-.967 1.02z" />',
  };

  // Auto-backfill/reclassify categories on load
  let categoriesBackfilled = false;
  $effect(() => {
    if (activeGame && !categoriesBackfilled && $installedMods.length > 0) {
      categoriesBackfilled = true;
      backfillCategories(activeGame.game_id, activeGame.bottle_name)
        .then((count) => { if (count > 0) loadMods(activeGame!); })
        .catch(() => {});
    }
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<svelte:window onkeydown={handleKeydown} onclick={() => { showToolsMenu = false; showSkseMenu = false; }} />
<div
  class="mods-page"
  class:drag-active={draggingOver}
  ondragover={handleDragOver}
  ondragleave={handleDragLeave}
  ondrop={handleDrop}
  role="application"
>
  {#if draggingOver}
    <div class="drop-overlay">
      <div class="drop-overlay-content">
        <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
          <polyline points="7 10 12 15 17 10" />
          <line x1="12" y1="15" x2="12" y2="3" />
        </svg>
        <p>Drop mod archive to install</p>
      </div>
    </div>
  {/if}
  {#if !activeGame}
    <!-- Game Picker -->
    <div class="picker-container">
      <div class="picker-icon">
        <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <rect x="2" y="6" width="20" height="12" rx="2" />
          <circle cx="8" cy="12" r="2" />
          <circle cx="16" cy="12" r="2" />
          <line x1="12" y1="8" x2="12" y2="16" />
        </svg>
      </div>
      <h2 class="picker-title">Select a Game</h2>
      <p class="picker-subtitle">Choose a game to view and manage its installed mods.</p>

      {#if $games.length === 0}
        <div class="picker-empty">
          <p>No games detected yet.</p>
          <p class="picker-empty-hint">Scan for bottles and games from the Dashboard first.</p>
          <button class="btn btn-secondary" onclick={() => currentPage.set("dashboard")}>
            Open Dashboard
          </button>
        </div>
      {:else}
        <div class="game-cards">
          {#each $games as game (game.game_id + game.bottle_name)}
            <button
              class="game-card"
              class:game-card-hovered={hoveredGame === game.game_id + game.bottle_name}
              onmouseenter={() => (hoveredGame = game.game_id + game.bottle_name)}
              onmouseleave={() => (hoveredGame = null)}
              onclick={() => selectGameForMods(game)}
            >
              <div class="game-card-content">
                <span class="game-card-name">{game.display_name}</span>
                <span class="game-card-bottle">{game.bottle_name}</span>
              </div>
              <div class="game-card-chevron">
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
                  <path d="M6 3.5L10.5 8L6 12.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>
                </svg>
              </div>
            </button>
          {/each}
        </div>
      {/if}
    </div>
  {:else}
    <!-- Game Banner Header -->
    <div class="game-banner">
      <div class="game-banner-icon">
        <GameIcon gameId={activeGame.game_id} size={36} />
      </div>
      <div class="game-banner-info">
        <h2 class="game-banner-title">{activeGame.display_name}</h2>
        <div class="game-banner-meta">
          <span class="meta-bottle">{activeGame.bottle_name}</span>
          {#if modCount > 0}
            <span class="meta-separator">&middot;</span>
            <span class="meta-mods">{enabledCount}/{modCount} mods active</span>
          {/if}
          {#if skse?.installed}
            <span class="meta-separator">&middot;</span>
            <span class="meta-skse">SKSE {skse.version ?? ""}</span>
          {/if}
        </div>
        {#if collectionNames.length > 0}
          <div class="modlist-selector">
            <select class="modlist-dropdown" bind:value={filterCollection}>
              <option value={null}>All Mods ({modCount})</option>
              <option value="__standalone__">Standalone ({$installedMods.filter(m => !m.collection_name).length})</option>
              {#each collectionNames as name}
                <option value={name}>{name} ({$installedMods.filter(m => m.collection_name === name).length})</option>
              {/each}
            </select>
          </div>
        {/if}
      </div>
      <div class="game-banner-actions">
        <button
          class="btn btn-ghost"
          onclick={handleCheckUpdates}
          disabled={checkingUpdates}
          title="Check Nexus for mod updates"
        >
          {#if checkingUpdates}
            <span class="spinner spinner-sm"></span>
            Checking...
          {:else}
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <polyline points="23 4 23 10 17 10" />
              <path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10" />
            </svg>
            Updates
            {#if modUpdates.length > 0}
              <span class="update-count-badge">{modUpdates.length}</span>
            {/if}
          {/if}
        </button>
        {#if modUpdates.length > 0}
          <button
            class="btn btn-accent btn-sm"
            onclick={handleUpdateAll}
            title="Open Nexus download pages for all outdated mods"
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
              <polyline points="7 10 12 15 17 10" />
              <line x1="12" y1="15" x2="12" y2="3" />
            </svg>
            Update All ({modUpdates.length})
          </button>
        {/if}
        <a
          href="https://www.nexusmods.com/{activeGame.nexus_slug}"
          target="_blank"
          rel="noopener noreferrer"
          class="btn btn-ghost nexus-link"
          title="View on Nexus Mods"
        >
          <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M11 8v3a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h3" />
            <path d="M8 2h4v4" />
            <path d="M6 8L12 2" />
          </svg>
          Nexus
        </a>
        {#if activeGame.game_id === "skyrimse"}
          <button
            class="btn btn-ghost"
            onclick={handleFixDisplay}
            disabled={fixingDisplay}
            title="Fix display: native resolution, fullscreen in its own Space (3-finger swipe to switch)"
          >
            {#if fixingDisplay}
              <span class="spinner spinner-sm"></span>
            {:else}
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <rect x="2" y="3" width="20" height="14" rx="2" ry="2" />
                <line x1="8" y1="21" x2="16" y2="21" />
                <line x1="12" y1="17" x2="12" y2="21" />
              </svg>
            {/if}
            Fix Display
          </button>
        {/if}
        <button class="btn btn-ghost" onclick={() => { pickedGame = null; selectedGame.set(null); }}>
          Change Game
        </button>
      </div>
    </div>

    <!-- Action Bar -->
    <div class="action-bar">
      <button class="btn btn-primary" onclick={handleInstall} disabled={installing}>
        {#if installing}
          <span class="spinner"></span>
          {stepLabels[installStep] ?? "Installing..."}
        {:else}
          <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
            <line x1="7" y1="2" x2="7" y2="12" />
            <line x1="2" y1="7" x2="12" y2="7" />
          </svg>
          Install Mod
        {/if}
      </button>

      <!-- Deploy / Purge Buttons -->
      {#if $installedMods.length > 0}
        <button
          class="btn btn-secondary btn-deploy"
          class:deploying
          onclick={handleDeploy}
          disabled={deploying || purging}
          title="Deploy all enabled mods to the game directory"
        >
          {#if deploying}
            <div class="deploy-progress-track">
              <div class="deploy-progress-fill" style="width: {deployProgress}%"></div>
            </div>
            <span class="deploy-progress-text">{deployProgressText || "Deploying..."}</span>
          {:else}
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <polyline points="20 6 9 17 4 12" />
            </svg>
            Deploy
          {/if}
        </button>
        <button
          class="btn btn-ghost-danger"
          onclick={handlePurge}
          disabled={deploying || purging}
          title="Remove all deployed files from the game directory"
        >
          {#if purging}
            <span class="spinner spinner-sm"></span>
            Purging...
          {:else}
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <polyline points="3 6 5 6 21 6" />
              <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
            </svg>
            Purge
          {/if}
        </button>
        {#if deployHealth}
          <span class="deploy-status" class:status-deployed={deployHealth.is_deployed} class:status-purged={!deployHealth.is_deployed}>
            {deployHealth.is_deployed ? "Deployed" : "Purged"}
          </span>
        {/if}
      {/if}
      <div class="tools-dropdown-wrap">
        <button
          class="btn btn-ghost"
          onclick={(e) => { e.stopPropagation(); showToolsMenu = !showToolsMenu; showSkseMenu = false; }}
          disabled={launchingToolId !== null}
          title="Tools"
        >
          {#if launchingToolId}
            <span class="spinner spinner-sm"></span>
          {:else}
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z" />
            </svg>
          {/if}
          Tools
          <svg width="8" height="8" viewBox="0 0 10 10" fill="currentColor"><path d="M2 3.5L5 7L8 3.5H2z" /></svg>
        </button>
        {#if showToolsMenu}
          <div class="tools-dropdown">
            <div class="dropdown-section-label">Tools</div>
            <button
              class="dropdown-item"
              onclick={() => { showToolsMenu = false; showImportWizard = true; }}
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                <polyline points="7 10 12 15 17 10" />
                <line x1="12" y1="15" x2="12" y2="3" />
              </svg>
              Import
            </button>
            <button
              class="dropdown-item"
              onclick={() => { showToolsMenu = false; handleExport(); }}
              disabled={exporting || $installedMods.length === 0}
            >
              {#if exporting}
                <span class="spinner spinner-sm"></span>
                Exporting...
              {:else}
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                  <polyline points="17 8 12 3 7 8" />
                  <line x1="12" y1="3" x2="12" y2="15" />
                </svg>
                Export
              {/if}
            </button>
            {#if modStats.enabled >= 10}
              <button
                class="dropdown-item"
                onclick={() => { showToolsMenu = false; showBisect = true; }}
              >
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <circle cx="11" cy="11" r="8" />
                  <line x1="21" y1="21" x2="16.65" y2="16.65" />
                  <line x1="8" y1="11" x2="14" y2="11" />
                </svg>
                Bisect
              </button>
            {/if}
            {#if activeGame}
              <button
                class="dropdown-item"
                onclick={() => { showToolsMenu = false; revealItemInDir(activeGame!.game_path); }}
              >
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
                </svg>
                Open Folder
              </button>
            {/if}
            {#if installedTools.length > 0}
              <div class="dropdown-divider"></div>
              <div class="dropdown-section-label">External Tools</div>
              {#each installedTools as tool (tool.id)}
                <button
                  class="dropdown-item"
                  onclick={() => handleLaunchTool(tool.id)}
                  disabled={launchingToolId === tool.id}
                >
                  <span class="tool-launch-name">{tool.name}</span>
                  <span class="tool-launch-cat">{tool.category}</span>
                </button>
              {/each}
              <div class="dropdown-divider"></div>
              <button class="dropdown-item dropdown-item-muted" onclick={() => { showToolsMenu = false; currentPage.set("settings"); }}>
                Manage Tools...
              </button>
            {/if}
          </div>
        {/if}
      </div>
      <div class="play-button-group">
        <button class="btn btn-play" onclick={handlePlay} disabled={launching || installingSkse}>
          {#if launching}
            <span class="spinner spinner-play"></span>
            Launching...
          {:else if installingSkse}
            <span class="spinner spinner-play"></span>
            Installing SKSE...
          {:else}
            <svg width="14" height="14" viewBox="0 0 14 14" fill="currentColor">
              <path d="M3 1.5v11l9-5.5L3 1.5z" />
            </svg>
            Play{#if skse?.use_skse && activeGame?.game_id === "skyrimse"} (SKSE){/if}
          {/if}
        </button>
        {#if activeGame?.game_id === "skyrimse"}
          <button
            class="btn btn-play-dropdown"
            onclick={(e) => { e.stopPropagation(); showSkseMenu = !showSkseMenu; showToolsMenu = false; showSkseInstallPrompt = false; }}
            aria-label="Launch options"
          >
            <svg width="10" height="10" viewBox="0 0 10 10" fill="currentColor">
              <path d="M2 3.5L5 7L8 3.5H2z" />
            </svg>
          </button>
        {/if}
        {#if showSkseMenu}
          <div class="skse-dropdown">
            <button class="dropdown-item" onclick={toggleSksePreference}>
              <span class="dropdown-check">
                {#if skse?.use_skse}
                  <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M10 3L4.5 8.5L2 6" />
                  </svg>
                {/if}
              </span>
              Launch via SKSE
            </button>
            <button class="dropdown-item" onclick={() => { showSkseMenu = false; doLaunch(false); }}>
              <span class="dropdown-check"></span>
              Launch Game Directly
            </button>
            <div class="dropdown-divider"></div>
            <div class="dropdown-info">
              {#if skse?.installed}
                SKSE {skse.version ?? ""} installed
              {:else}
                SKSE not installed
              {/if}
            </div>
          </div>
        {/if}
        {#if showSkseInstallPrompt}
          <div class="skse-dropdown skse-install-prompt">
            <div class="dropdown-info" style="font-weight: 600; color: var(--text-primary);">SKSE is not installed</div>
            <div class="dropdown-divider"></div>
            <button class="dropdown-item" onclick={handleInstallSkseAndLaunch} disabled={installingSkse}>
              <span class="dropdown-check">
                <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M6 2v8M2 6l4 4 4-4" />
                </svg>
              </span>
              {installingSkse ? "Installing..." : "Auto Install SKSE"}
            </button>
            <button class="dropdown-item" onclick={handleInstallSkseArchiveAndLaunch} disabled={installingSkse}>
              <span class="dropdown-check">
                <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <rect x="2" y="2" width="8" height="8" rx="1" />
                </svg>
              </span>
              Install from Archive
            </button>
            <div class="dropdown-divider"></div>
            <button class="dropdown-item dropdown-item-muted" onclick={() => { showSkseInstallPrompt = false; doLaunch(false); }}>
              <span class="dropdown-check"></span>
              Launch Without SKSE
            </button>
          </div>
        {/if}
      </div>
    </div>

    <!-- Banners (full-width, above the main content grid) -->
    {#if showSksePrompt && activeGame?.game_id === "skyrimse"}
      <div class="skse-banner">
        <div class="skse-banner-icon">
          <svg width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="10" cy="10" r="9" />
            <path d="M10 6v4" />
            <circle cx="10" cy="14" r="0.5" fill="currentColor" />
          </svg>
        </div>
        <div class="skse-banner-content">
          <p class="skse-banner-title">SKSE Not Installed</p>
          <p class="skse-banner-text">
            SKSE is required by most Skyrim mods.
          </p>
        </div>
        <div class="skse-banner-actions">
          <button class="btn btn-primary btn-sm" onclick={handleAutoInstallSkse} disabled={installingSkse}>
            {installingSkse ? "Installing..." : "Auto Install"}
          </button>
          <button class="btn btn-secondary btn-sm" onclick={handleInstallSkse} disabled={installingSkse}>
            From Archive
          </button>
          <button class="btn btn-ghost btn-sm" onclick={dismissSksePrompt}>Dismiss</button>
        </div>
      </div>
    {/if}

    {#if showDowngradeBanner && activeGame?.game_id === "skyrimse" && downgradeStatus && !downgradeStatus.is_downgraded}
      <div class="downgrade-banner">
        <div class="skse-banner-icon">
          <svg width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M10 2v12" />
            <polyline points="6 10 10 14 14 10" />
            <path d="M4 18h12" />
          </svg>
        </div>
        <div class="skse-banner-content">
          <p class="skse-banner-title">
            Skyrim SE {downgradeStatus.current_version}
            {#if downgradeStatus.current_version !== "1.5.97"} — Downgrade Available{/if}
          </p>
          <p class="skse-banner-text">Most mods target v1.5.97.</p>
        </div>
        <div class="skse-banner-actions">
          <button class="btn btn-primary btn-sm" onclick={handleDowngrade} disabled={downgrading}>
            {downgrading ? "Downgrading..." : "Downgrade"}
          </button>
          <button class="btn btn-ghost btn-sm" onclick={dismissDowngradeBanner}>Dismiss</button>
        </div>
      </div>
    {/if}

    <!-- Two-column content area -->
    <div class="content-grid">
      <!-- LEFT: Mod list (primary focus) -->
      <div class="content-main">
        <!-- Visual Conflict Map -->
        {#if showConflictMap}
          <ConflictMap visible={showConflictMap} onclose={() => { showConflictMap = false; }} />
        {/if}

        <!-- Smart Conflict Resolution Panel -->
        {#if showConflictPanel && conflicts.length > 0}
          <div class="conflict-panel">
            <div class="conflict-panel-header">
              <h3 class="conflict-panel-title">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
                  <line x1="12" y1="9" x2="12" y2="13" />
                  <line x1="12" y1="17" x2="12.01" y2="17" />
                </svg>
                File Conflicts ({conflicts.length})
              </h3>
              <div class="conflict-panel-actions">
                <button
                  class="btn btn-accent btn-sm magic-resolve-btn"
                  onclick={handleMagicResolve}
                  disabled={resolvingAll || analyzingConflicts}
                  title="Automatically resolve all conflicts using LOOT data and collection authorship"
                >
                  {#if resolvingAll}
                    <span class="spinner spinner-sm"></span>
                    Resolving...
                  {:else}
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M12 2l3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01L12 2z" />
                    </svg>
                    Magic Resolver
                  {/if}
                </button>
                <button
                  class="btn btn-ghost btn-sm"
                  onclick={handleAnalyzeConflicts}
                  disabled={analyzingConflicts}
                  title="Analyze conflicts without applying changes"
                >
                  {#if analyzingConflicts}
                    <span class="spinner spinner-sm"></span>
                  {:else}
                    Analyze
                  {/if}
                </button>
                <button class="btn btn-ghost btn-sm" onclick={() => { showConflictPanel = false; suggestions = []; resolutionResult = null; identicalStats = null; }}>
                  <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
                    <line x1="3" y1="3" x2="11" y2="11" />
                    <line x1="11" y1="3" x2="3" y2="11" />
                  </svg>
                </button>
              </div>
            </div>

            <!-- Resolution summary banner -->
            {#if resolutionResult}
              <div class="resolution-banner">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14" />
                  <polyline points="22 4 12 14.01 9 11.01" />
                </svg>
                <span>
                  {resolutionResult.author_resolved} author-resolved,
                  {resolutionResult.auto_suggested} auto-fixed,
                  {#if resolutionResult.identical_content > 0}
                    {resolutionResult.identical_content} identical files,
                  {/if}
                  {resolutionResult.manual_needed} need review
                  {#if resolutionResult.priorities_changed > 0}
                    &mdash; {resolutionResult.priorities_changed} priorities adjusted
                  {/if}
                </span>
              </div>
            {/if}

            <!-- Identical content auto-resolution banner -->
            {#if identicalStats && identicalStats.identical_files_total > 0}
              <div class="identical-banner">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M16 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2" />
                  <circle cx="8.5" cy="7" r="4" />
                  <polyline points="17 11 19 13 23 9" />
                </svg>
                <span>
                  {identicalStats.fully_identical} conflict{identicalStats.fully_identical === 1 ? "" : "s"} auto-resolved (identical files across mods){#if identicalStats.partially_identical > 0}, {identicalStats.partially_identical} partially identical{/if}
                </span>
              </div>
            {/if}

            <!-- Smart suggestions view -->
            {#if suggestions.length > 0}
              <div class="conflict-list">
                {#each suggestions as s (s.relative_path)}
                  <div class="conflict-item" class:conflict-resolved={s.status === "AuthorResolved"} class:conflict-suggested={s.status === "Suggested"} class:conflict-identical={s.status === "IdenticalContent"}>
                    <div class="conflict-path">
                      <span class="conflict-status-badge" class:status-author={s.status === "AuthorResolved"} class:status-suggested={s.status === "Suggested"} class:status-manual={s.status === "Manual"} class:status-identical={s.status === "IdenticalContent"}>
                        {#if s.status === "AuthorResolved"}OK
                        {:else if s.status === "Suggested"}Auto
                        {:else if s.status === "IdenticalContent"}Same
                        {:else}Manual{/if}
                      </span>
                      <span class="conflict-filepath">{s.relative_path}</span>
                    </div>
                    <div class="conflict-reason">{s.reason}</div>
                    <div class="conflict-mods">
                      {#each s.mods as mod (mod.mod_id)}
                        <div class="conflict-mod" class:conflict-winner={mod.mod_id === s.suggested_winner_id}>
                          <span class="conflict-mod-name">
                            {mod.mod_name}
                            {#if mod.mod_id === s.suggested_winner_id}
                              <span class="winner-badge">{s.status === "AuthorResolved" ? "Author" : s.status === "Suggested" ? "Suggested" : s.status === "IdenticalContent" ? "Identical" : "Winner"}</span>
                            {/if}
                          </span>
                          <span class="conflict-mod-priority">Priority {mod.priority}</span>
                        </div>
                      {/each}
                    </div>
                  </div>
                {/each}
              </div>

            <!-- Fallback: raw conflict view (before analysis) -->
            {:else}
              <div class="conflict-list">
                {#each conflicts as conflict (conflict.relative_path)}
                  <div class="conflict-item">
                    <div class="conflict-path">
                      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z" />
                        <polyline points="13 2 13 9 20 9" />
                      </svg>
                      <span class="conflict-filepath">{conflict.relative_path}</span>
                    </div>
                    <div class="conflict-mods">
                      {#each conflict.mods as mod (mod.mod_id)}
                        <div class="conflict-mod" class:conflict-winner={mod.mod_id === conflict.winner_mod_id}>
                          <span class="conflict-mod-name">
                            {mod.mod_name}
                            {#if mod.mod_id === conflict.winner_mod_id}
                              <span class="winner-badge">Winner</span>
                            {/if}
                          </span>
                          <span class="conflict-mod-priority">Priority {mod.priority}</span>
                          {#if mod.mod_id !== conflict.winner_mod_id}
                            <button
                              class="btn btn-ghost btn-sm make-winner-btn"
                              onclick={() => handleMakeWinner(conflict, mod.mod_id)}
                              disabled={makingWinner !== null}
                            >
                              {#if makingWinner === mod.mod_id}
                                <span class="spinner spinner-sm"></span>
                              {:else}
                                Make Winner
                              {/if}
                            </button>
                          {/if}
                        </div>
                      {/each}
                    </div>
                  </div>
                {/each}
              </div>
            {/if}
          </div>
        {/if}

        <!-- Proactive Issue Banners -->
        {#if conflictModIds.size > 0 && !dismissedBanners.has("conflicts")}
          <div class="issue-banner issue-banner-yellow">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
              <line x1="12" y1="9" x2="12" y2="13" /><line x1="12" y1="17" x2="12.01" y2="17" />
            </svg>
            <span>You have {conflictModIds.size} mod{conflictModIds.size === 1 ? "" : "s"} with unresolved file conflicts</span>
            <button class="banner-action" onclick={() => { showConflictPanel = true; }}>View Conflicts</button>
            <button class="banner-action" onclick={() => { showConflictMap = true; }}>View Map</button>
            <button class="banner-dismiss" onclick={() => dismissBanner("conflicts")}>
              <svg width="10" height="10" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
                <line x1="3" y1="3" x2="9" y2="9" /><line x1="9" y1="3" x2="3" y2="9" />
              </svg>
            </button>
          </div>
        {/if}
        {#if $skseStatus && !$skseStatus.installed && activeGame?.game_id === "skyrimse" && !dismissedBanners.has("skse")}
          <div class="issue-banner issue-banner-blue">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="12" cy="12" r="10" /><line x1="12" y1="16" x2="12" y2="12" /><line x1="12" y1="8" x2="12.01" y2="8" />
            </svg>
            <span>SKSE not detected — many mods require it</span>
            <button class="banner-action" onclick={() => { showSksePrompt = true; }}>Install SKSE</button>
            <button class="banner-dismiss" onclick={() => dismissBanner("skse")}>
              <svg width="10" height="10" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
                <line x1="3" y1="3" x2="9" y2="9" /><line x1="9" y1="3" x2="3" y2="9" />
              </svg>
            </button>
          </div>
        {/if}

        <!-- Bulk Action Bar -->
        {#if selectedModIds.size > 0}
          <div class="bulk-action-bar">
            <span class="bulk-count">{selectedModIds.size} selected</span>
            <button class="btn btn-sm btn-secondary" disabled={bulkOperating !== null} onclick={batchEnable}>
              {bulkOperating === "enabling" ? "Enabling..." : "Enable All"}
            </button>
            <button class="btn btn-sm btn-secondary" disabled={bulkOperating !== null} onclick={batchDisable}>
              {bulkOperating === "disabling" ? "Disabling..." : "Disable All"}
            </button>
            <button class="btn btn-sm btn-ghost-danger" disabled={bulkOperating !== null} onclick={batchUninstall}>
              {bulkOperating === "uninstalling" ? "Uninstalling..." : "Uninstall"}
            </button>
            <button class="btn btn-sm btn-ghost" disabled={bulkOperating !== null} onclick={() => selectedModIds = new Set()}>Clear</button>
          </div>
        {/if}

        <!-- Search & Filter Bar -->
        {#if $installedMods.length > 0}
          <div class="filter-bar">
        <div class="search-box">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="11" cy="11" r="8" />
            <line x1="21" y1="21" x2="16.65" y2="16.65" />
          </svg>
          {#each [...parsedSearch.facets] as [key, value] (key)}
            <button
              class="facet-pill"
              onclick={() => {
                const regex = new RegExp(`${key}:(?:"[^"]*"|\\S+)\\s*`, "i");
                searchQuery = searchQuery.replace(regex, "").trim();
              }}
              title="Click to remove"
            >
              {key}:{value}
              <svg width="10" height="10" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
                <line x1="3" y1="3" x2="9" y2="9" /><line x1="9" y1="3" x2="3" y2="9" />
              </svg>
            </button>
          {/each}
          <input
            type="text"
            placeholder={parsedSearch.facets.size > 0 ? "Add more filters..." : "Search mods... (try tag: source: enabled:)"}
            bind:value={searchQuery}
            class="search-input"
          />
          {#if searchQuery}
            <button class="search-clear" onclick={() => searchQuery = ""}>
              <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
                <line x1="3" y1="3" x2="9" y2="9" />
                <line x1="9" y1="3" x2="3" y2="9" />
              </svg>
            </button>
          {/if}
        </div>
        <select class="filter-select" bind:value={filterStatus}>
          <option value="all">All Status</option>
          <option value="enabled">Enabled</option>
          <option value="disabled">Disabled</option>
          <option value="conflicts">Has Conflicts</option>
          <option value="has-updates">Has Updates</option>
        </select>
        <select class="filter-select" bind:value={filterSource}>
          <option value="all">All Sources</option>
          <option value="nexus">Nexus</option>
          <option value="loverslab">LoversLab</option>
          <option value="moddb">ModDB</option>
          <option value="curseforge">CurseForge</option>
          <option value="direct">Direct</option>
          <option value="manual">Manual</option>
        </select>
        <!-- Category dropdown button -->
        {#if uniqueCategories.length > 1}
          <div class="category-dropdown-wrapper">
            <button
              class="filter-select category-dropdown-btn"
              class:has-active={filterCategory !== null}
              onclick={() => showCategoryPopover = !showCategoryPopover}
            >
              <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <rect x="3" y="3" width="7" height="7" /><rect x="14" y="3" width="7" height="7" />
                <rect x="14" y="14" width="7" height="7" /><rect x="3" y="14" width="7" height="7" />
              </svg>
              Categories
              <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="6 9 12 15 18 9" />
              </svg>
            </button>
            {#if filterCategory}
              {@const catColor = categoryColors[filterCategory] ?? '#6b7280'}
              <button
                class="active-category-chip"
                style="--chip-color: {catColor};"
                onclick={() => filterCategory = null}
                title="Clear category filter"
              >
                {#if categoryIcons[filterCategory]}
                  <!-- eslint-disable-next-line svelte/no-at-html-tags -->
                  <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">{@html categoryIcons[filterCategory]}</svg>
                {/if}
                {filterCategory}
                <svg width="9" height="9" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
                  <line x1="3" y1="3" x2="9" y2="9" /><line x1="9" y1="3" x2="3" y2="9" />
                </svg>
              </button>
            {/if}
            {#if showCategoryPopover}
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div class="category-popover-backdrop" onclick={() => showCategoryPopover = false} onkeydown={(e) => { if (e.key === 'Escape') showCategoryPopover = false; }}></div>
              <div class="category-popover">
                {#each uniqueCategories as [cat, count]}
                  {@const catColor = categoryColors[cat] ?? '#6b7280'}
                  <button
                    class="category-filter-chip"
                    class:active={filterCategory === cat}
                    style="--chip-color: {catColor};"
                    onclick={() => { filterCategory = filterCategory === cat ? null : cat; showCategoryPopover = false; }}
                    title="{cat} ({count} mods)"
                  >
                    {#if categoryIcons[cat]}
                      <!-- eslint-disable-next-line svelte/no-at-html-tags -->
                      <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">{@html categoryIcons[cat]}</svg>
                    {/if}
                    {cat}
                    <span class="chip-count">{count}</span>
                  </button>
                {/each}
              </div>
            {/if}
          </div>
        {/if}
        <!-- Stat badges (inline) -->
        <div class="stats-badges">
          <span class="stat-badge stat-enabled">{modStats.enabled} Enabled</span>
          <span class="stat-badge stat-disabled">{modStats.disabled} Disabled</span>
          {#if modStats.conflicts > 0}
            <span class="stat-badge stat-conflicts">{modStats.conflicts} Conflicts</span>
          {/if}
          {#if modStats.updates > 0}
            <span class="stat-badge stat-updates">{modStats.updates} Updates</span>
          {/if}
        </div>
        <!-- View mode toggle -->
        <div class="view-mode-toggle">
          <button
            class="view-mode-btn"
            class:active={viewMode === "flat"}
            onclick={() => viewMode = "flat"}
            title="List View"
          >
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <line x1="8" y1="6" x2="21" y2="6" /><line x1="8" y1="12" x2="21" y2="12" /><line x1="8" y1="18" x2="21" y2="18" />
              <line x1="3" y1="6" x2="3.01" y2="6" /><line x1="3" y1="12" x2="3.01" y2="12" /><line x1="3" y1="18" x2="3.01" y2="18" />
            </svg>
          </button>
          <button
            class="view-mode-btn"
            class:active={viewMode === "collection"}
            onclick={() => viewMode = "collection"}
            title="Collection View"
          >
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
            </svg>
          </button>
          <button
            class="view-mode-btn"
            class:active={viewMode === "category"}
            onclick={() => viewMode = "category"}
            title="Category View"
          >
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <rect x="3" y="3" width="7" height="7" /><rect x="14" y="3" width="7" height="7" />
              <rect x="14" y="14" width="7" height="7" /><rect x="3" y="14" width="7" height="7" />
            </svg>
          </button>
        </div>
        {#if searchQuery || filterStatus !== "all" || filterSource !== "all" || filterCollection !== null || filterCategory !== null}
          <span class="filter-count">{filteredMods.length} of {$installedMods.length}</span>
        {/if}
      </div>
    {/if}

    <!-- Content Area -->
    {#if loadingMods}
      <div class="empty-state">
        <div class="empty-icon">
          <span class="spinner"></span>
        </div>
        <h3 class="empty-title">Loading mods...</h3>
      </div>
    {:else if $installedMods.length === 0}
      <div class="empty-state">
        <div class="empty-icon">
          <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z" />
            <polyline points="3.27 6.96 12 12.01 20.73 6.96" />
            <line x1="12" y1="22.08" x2="12" y2="12" />
          </svg>
        </div>
        <h3 class="empty-title">No mods installed</h3>
        <p class="empty-description">
          Install mods from .zip, .7z, or .rar archives, or use NXM links via Settings.
        </p>
        <div class="empty-actions">
          <button class="btn btn-primary" onclick={handleInstall}>
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
              <line x1="7" y1="2" x2="7" y2="12" />
              <line x1="2" y1="7" x2="12" y2="7" />
            </svg>
            Install from Archive
          </button>
          {#if activeGame}
            <a
              href="https://www.nexusmods.com/{activeGame.nexus_slug}"
              target="_blank"
              rel="noopener noreferrer"
              class="btn btn-secondary"
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" /><polyline points="15 3 21 3 21 9" /><line x1="10" y1="14" x2="21" y2="3" />
              </svg>
              Browse NexusMods
            </a>
          {/if}
          <button class="btn btn-ghost" onclick={() => currentPage.set("collections")}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
            </svg>
            Install a Collection
          </button>
        </div>
      </div>
    {:else}
      <div class="mod-layout" class:has-detail={detailMod !== null}>
      <div class="mod-table-container" class:reordering-active={reordering}>
        <div class="mod-table" style="--grid-cols: {gridTemplate}">
          <!-- Sticky Header — click to sort -->
          <div class="table-header" class:resizing={resizingCol !== null}>
            <label class="col-check">
              <input type="checkbox" checked={selectAll} onchange={toggleSelectAll} />
            </label>
            <span class="col-grip" title="Drag to reorder"></span>
            <span class="col-toggle header-sep-right">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="opacity: 0.5" aria-hidden="true">
                <rect x="1" y="5" width="22" height="14" rx="7" ry="7" />
                <circle cx="16" cy="12" r="3" />
              </svg>
            </span>
            <button type="button" class="col-name sortable-header" onclick={() => toggleSort("name")}>
              Mod Name
              {#if sortBy === "name"}
                <span class="sort-arrow">{sortDir === "asc" ? "\u25B2" : "\u25BC"}</span>
              {/if}
              <span class="col-resize" onpointerdown={(e) => onResizeStart(e, 'name')} role="separator" aria-orientation="vertical"></span>
            </button>
            <span class="col-category">
              Category
              <span class="col-resize" onpointerdown={(e) => onResizeStart(e, 'category')} role="separator" aria-orientation="vertical"></span>
            </span>
            <span class="col-origin">
              DL Origin
              <span class="col-resize" onpointerdown={(e) => onResizeStart(e, 'origin')} role="separator" aria-orientation="vertical"></span>
            </span>
            <span class="col-source">
              Installed By
              <span class="col-resize" onpointerdown={(e) => onResizeStart(e, 'source')} role="separator" aria-orientation="vertical"></span>
            </span>
            <button type="button" class="col-version sortable-header" onclick={() => toggleSort("version")}>
              Version
              {#if sortBy === "version"}
                <span class="sort-arrow">{sortDir === "asc" ? "\u25B2" : "\u25BC"}</span>
              {/if}
              <span class="col-resize" onpointerdown={(e) => onResizeStart(e, 'version')} role="separator" aria-orientation="vertical"></span>
            </button>
            <button type="button" class="col-files sortable-header" onclick={() => toggleSort("files")}>
              Files
              {#if sortBy === "files"}
                <span class="sort-arrow">{sortDir === "asc" ? "\u25B2" : "\u25BC"}</span>
              {/if}
              <span class="col-resize" onpointerdown={(e) => onResizeStart(e, 'files')} role="separator" aria-orientation="vertical"></span>
            </button>
            <button type="button" class="col-date sortable-header" onclick={() => toggleSort("date")}>
              Installed
              {#if sortBy === "date"}
                <span class="sort-arrow">{sortDir === "asc" ? "\u25B2" : "\u25BC"}</span>
              {/if}
              <span class="col-resize" onpointerdown={(e) => onResizeStart(e, 'date')} role="separator" aria-orientation="vertical"></span>
            </button>
            <span class="col-actions">Actions</span>
          </div>

          <!-- Mod Rows -->
          <div class="table-body" bind:this={tableBodyEl} onscroll={handleTableScroll}>
            {#if filteredMods.length === 0 && disabledFilteredMods.length === 0 && $installedMods.length > 0}
              <div class="empty-filter-state">
                <p>No mods match your filters.</p>
                <button class="btn btn-ghost btn-sm" onclick={() => { searchQuery = ""; filterStatus = "all"; filterSource = "all"; filterCollection = null; filterCategory = null; }}>
                  Clear Filters
                </button>
              </div>
            {:else if viewMode === "category"}
              <ModCategoryView
                mods={filteredMods}
                {categoryColors}
                {categoryIcons}
                {conflictModIds}
                {togglingMod}
                {selectedModId}
                onselect={(mod) => { selectedModId = selectedModId === mod.id ? undefined : mod.id; detailMod = detailMod?.id === mod.id ? null : mod; }}
                ontoggle={handleToggle}
              />
            {:else if viewMode === "collection" && groupedMods}
              {#each [...groupedMods.entries()] as [groupName, groupMods] (groupName)}
                <button class="group-header" onclick={() => toggleGroup(groupName)}>
                  <svg
                    class="group-chevron"
                    class:expanded={!collapsedGroups.has(groupName)}
                    width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
                  >
                    <path d="M4 2l4 4-4 4" />
                  </svg>
                  <span class="group-name">{groupName}</span>
                  <span class="group-count">{groupMods.length}</span>
                </button>
                {#if !collapsedGroups.has(groupName)}
                  {@const requiredMods = groupMods.filter(m => !m.collection_optional)}
                  {@const optionalMods = groupMods.filter(m => m.collection_optional)}
                  {#each requiredMods as mod, i (mod.id)}
                    {@const globalIndex = filteredModIndex.get(mod.id) ?? 0}
                    <!-- Re-use the same row markup with global index for DnD -->
                    <div
                      class="table-row"
                      class:row-disabled={!mod.enabled}
                      class:row-selected={selectedModId === mod.id}
                      class:row-checked={selectedModIds.has(mod.id)}
                      class:row-has-conflict={conflictModIds.has(mod.id)}
                      class:row-dragging={dragRowIndex === globalIndex}
                      class:row-drag-over={dragOverIndex === globalIndex && dragRowIndex !== null && dragRowIndex !== globalIndex}
                      class:row-drag-above={dragOverIndex === globalIndex && dragRowIndex !== null && dragRowIndex > globalIndex}
                      class:row-drag-below={dragOverIndex === globalIndex && dragRowIndex !== null && dragRowIndex < globalIndex}
                      draggable="true"
                      onclick={(e) => handleRowClick(e, mod, globalIndex)}
                      ondragstart={(e) => handleRowDragStart(e, globalIndex)}
                      ondragover={(e) => handleRowDragOver(e, globalIndex)}
                      ondragend={handleRowDragEnd}
                      ondrop={(e) => handleRowDrop(e, globalIndex)}
                    >
                      <label class="col-check" onclick={(e) => e.stopPropagation()}><input type="checkbox" checked={selectedModIds.has(mod.id)} onchange={() => toggleSelectMod(mod.id)} /></label>
                      <span class="col-grip"><span class="drag-handle" title="Drag to reorder" aria-label="Drag to reorder {mod.name}"><svg width="12" height="12" viewBox="0 0 12 12" fill="currentColor"><circle cx="4" cy="2.5" r="1" /><circle cx="8" cy="2.5" r="1" /><circle cx="4" cy="6" r="1" /><circle cx="8" cy="6" r="1" /><circle cx="4" cy="9.5" r="1" /><circle cx="8" cy="9.5" r="1" /></svg></span></span>
                      <span class="col-toggle"><button class="toggle-switch" class:toggle-on={mod.enabled} class:toggle-busy={togglingMod === mod.id} onclick={() => handleToggle(mod)} title={mod.enabled ? "Disable mod" : "Enable mod"} aria-label="{mod.enabled ? 'Disable' : 'Enable'} {mod.name}" aria-pressed={mod.enabled} role="switch"><span class="toggle-track"><span class="toggle-thumb"></span></span></button></span>
                      <span class="col-name"><span class="mod-name">{mod.name}</span>{#if conflictModIds.has(mod.id)}<span class="conflict-icon" title={getConflictTooltip(mod.id)}><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" /><line x1="12" y1="9" x2="12" y2="13" /><line x1="12" y1="17" x2="12.01" y2="17" /></svg></span>{/if}{#if mod.user_notes}<span class="notes-icon" title={mod.user_notes}><svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" /><polyline points="14 2 14 8 20 8" /><line x1="16" y1="13" x2="8" y2="13" /><line x1="16" y1="17" x2="8" y2="17" /></svg></span>{/if}</span>
                      <span class="col-category">{#if mod.auto_category}<span class="category-cell" style="color: {categoryColors[mod.auto_category] ?? '#6b7280'};" title={mod.auto_category}>{#if categoryIcons[mod.auto_category]}<svg class="category-icon" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html categoryIcons[mod.auto_category]}</svg>{/if}<span class="category-label">{mod.auto_category}</span></span>{:else}<span class="text-muted">&mdash;</span>{/if}</span>
                      <span class="col-origin">{#if mod.source_type === "nexus"}<span class="origin-label origin-nexus">Nexus</span>{:else if mod.source_type === "loverslab"}<span class="origin-label origin-loverslab">LoversLab</span>{:else if mod.source_type === "moddb"}<span class="origin-label origin-moddb">ModDB</span>{:else if mod.source_type === "curseforge"}<span class="origin-label origin-curseforge">CurseForge</span>{:else if mod.source_type === "direct"}<span class="origin-label origin-direct">Direct</span>{:else}<span class="origin-label origin-manual">Manual</span>{/if}{#if getModSourceUrl(mod)}<button class="origin-link-btn" title="Open mod page" onclick={(e) => { e.stopPropagation(); openUrl(getModSourceUrl(mod)!); }}><svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71" /><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" /></svg></button>{/if}</span>
                      <span class="col-source">{#if mod.collection_name}<span class="source-label source-collection" title={mod.collection_name}>{mod.collection_name}</span>{:else}<span class="source-label source-user">User</span>{/if}</span>
                      <span class="col-version"><span class="version-text">{mod.version || "\u2014"}</span>{#if updateMap.has(mod.id)}{@const update = updateMap.get(mod.id)!}<span class="update-badge" title={`Update available: v${update.latest_version}`}>Update</span>{/if}</span>
                      <span class="col-files">{mod.installed_files.length}</span>
                      <span class="col-date">{formatDate(mod.installed_at)}</span>
                      <span class="col-actions">
                        {#if confirmUninstall === mod.id}
                          <div class="confirm-actions"><button class="btn btn-danger btn-sm" onclick={() => handleUninstall(mod.id)}>Yes</button><button class="btn btn-ghost btn-sm" onclick={() => (confirmUninstall = null)}>No</button></div>
                        {:else}
                          <div class="mod-action-group">
                            <button class="mod-uninstall-btn" onclick={(e) => { e.stopPropagation(); confirmUninstall = mod.id; }} title="Uninstall mod"><svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6" /><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" /></svg></button>
                            <div class="mod-overflow-wrap">
                              <button class="mod-overflow-btn" onclick={(e) => { e.stopPropagation(); overflowMenuModId = overflowMenuModId === mod.id ? null : mod.id; }} title="More actions"><svg width="13" height="13" viewBox="0 0 24 24" fill="currentColor"><circle cx="12" cy="5" r="2" /><circle cx="12" cy="12" r="2" /><circle cx="12" cy="19" r="2" /></svg></button>
                            </div>
                          </div>
                        {/if}
                      </span>
                    </div>
                  {/each}
                  {#if optionalMods.length > 0}
                    <button class="optional-separator" onclick={() => toggleGroup(`${groupName}__optional`)}>
                      <svg
                        class="group-chevron"
                        class:expanded={!collapsedGroups.has(`${groupName}__optional`)}
                        width="10" height="10" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
                      >
                        <path d="M4 2l4 4-4 4" />
                      </svg>
                      <span class="optional-label">Optional</span>
                      <span class="optional-count">{optionalMods.length}</span>
                      <span class="optional-line"></span>
                    </button>
                    {#if !collapsedGroups.has(`${groupName}__optional`)}
                      {#each optionalMods as mod, i (mod.id)}
                        {@const globalIndex = filteredModIndex.get(mod.id) ?? 0}
                        <div
                          class="table-row row-optional"
                          class:row-disabled={!mod.enabled}
                          class:row-selected={selectedModId === mod.id}
                          class:row-checked={selectedModIds.has(mod.id)}
                          class:row-has-conflict={conflictModIds.has(mod.id)}
                          class:row-dragging={dragRowIndex === globalIndex}
                          class:row-drag-over={dragOverIndex === globalIndex && dragRowIndex !== null && dragRowIndex !== globalIndex}
                          class:row-drag-above={dragOverIndex === globalIndex && dragRowIndex !== null && dragRowIndex > globalIndex}
                          class:row-drag-below={dragOverIndex === globalIndex && dragRowIndex !== null && dragRowIndex < globalIndex}
                          draggable="true"
                          onclick={(e) => handleRowClick(e, mod, globalIndex)}
                          ondragstart={(e) => handleRowDragStart(e, globalIndex)}
                          ondragover={(e) => handleRowDragOver(e, globalIndex)}
                          ondragend={handleRowDragEnd}
                          ondrop={(e) => handleRowDrop(e, globalIndex)}
                        >
                          <label class="col-check" onclick={(e) => e.stopPropagation()}><input type="checkbox" checked={selectedModIds.has(mod.id)} onchange={() => toggleSelectMod(mod.id)} /></label>
                          <span class="col-grip"><span class="drag-handle" title="Drag to reorder" aria-label="Drag to reorder {mod.name}"><svg width="12" height="12" viewBox="0 0 12 12" fill="currentColor"><circle cx="4" cy="2.5" r="1" /><circle cx="8" cy="2.5" r="1" /><circle cx="4" cy="6" r="1" /><circle cx="8" cy="6" r="1" /><circle cx="4" cy="9.5" r="1" /><circle cx="8" cy="9.5" r="1" /></svg></span></span>
                          <span class="col-toggle"><button class="toggle-switch" class:toggle-on={mod.enabled} class:toggle-busy={togglingMod === mod.id} onclick={() => handleToggle(mod)} title={mod.enabled ? "Disable mod" : "Enable mod"} aria-label="{mod.enabled ? 'Disable' : 'Enable'} {mod.name}" aria-pressed={mod.enabled} role="switch"><span class="toggle-track"><span class="toggle-thumb"></span></span></button></span>
                          <span class="col-name"><span class="mod-name">{mod.name}</span>{#if conflictModIds.has(mod.id)}<span class="conflict-icon" title={getConflictTooltip(mod.id)}><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" /><line x1="12" y1="9" x2="12" y2="13" /><line x1="12" y1="17" x2="12.01" y2="17" /></svg></span>{/if}{#if mod.user_notes}<span class="notes-icon" title={mod.user_notes}><svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" /><polyline points="14 2 14 8 20 8" /><line x1="16" y1="13" x2="8" y2="13" /><line x1="16" y1="17" x2="8" y2="17" /></svg></span>{/if}</span>
                          <span class="col-category">{#if mod.auto_category}<span class="category-cell" style="color: {categoryColors[mod.auto_category] ?? '#6b7280'};" title={mod.auto_category}>{#if categoryIcons[mod.auto_category]}<svg class="category-icon" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html categoryIcons[mod.auto_category]}</svg>{/if}<span class="category-label">{mod.auto_category}</span></span>{:else}<span class="text-muted">&mdash;</span>{/if}</span>
                          <span class="col-origin">{#if mod.source_type === "nexus"}<span class="origin-label origin-nexus">Nexus</span>{:else if mod.source_type === "loverslab"}<span class="origin-label origin-loverslab">LoversLab</span>{:else if mod.source_type === "moddb"}<span class="origin-label origin-moddb">ModDB</span>{:else if mod.source_type === "curseforge"}<span class="origin-label origin-curseforge">CurseForge</span>{:else if mod.source_type === "direct"}<span class="origin-label origin-direct">Direct</span>{:else}<span class="origin-label origin-manual">Manual</span>{/if}{#if getModSourceUrl(mod)}<button class="origin-link-btn" title="Open mod page" onclick={(e) => { e.stopPropagation(); openUrl(getModSourceUrl(mod)!); }}><svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71" /><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" /></svg></button>{/if}</span>
                          <span class="col-source">{#if mod.collection_name}<span class="source-label source-collection" title={mod.collection_name}>{mod.collection_name}</span>{:else}<span class="source-label source-user">User</span>{/if}</span>
                          <span class="col-version"><span class="version-text">{mod.version || "\u2014"}</span>{#if updateMap.has(mod.id)}{@const update = updateMap.get(mod.id)!}<span class="update-badge" title={`Update available: v${update.latest_version}`}>Update</span>{/if}</span>
                          <span class="col-files">{mod.installed_files.length}</span>
                          <span class="col-date">{formatDate(mod.installed_at)}</span>
                          <span class="col-actions">
                            {#if confirmUninstall === mod.id}
                              <div class="confirm-actions"><button class="btn btn-danger btn-sm" onclick={() => handleUninstall(mod.id)}>Yes</button><button class="btn btn-ghost btn-sm" onclick={() => (confirmUninstall = null)}>No</button></div>
                            {:else}
                              <div class="mod-action-group">
                                <button class="mod-uninstall-btn" onclick={(e) => { e.stopPropagation(); confirmUninstall = mod.id; }} title="Uninstall mod"><svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6" /><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" /></svg></button>
                                <div class="mod-overflow-wrap">
                                  <button class="mod-overflow-btn" onclick={(e) => { e.stopPropagation(); overflowMenuModId = overflowMenuModId === mod.id ? null : mod.id; }} title="More actions"><svg width="13" height="13" viewBox="0 0 24 24" fill="currentColor"><circle cx="12" cy="5" r="2" /><circle cx="12" cy="12" r="2" /><circle cx="12" cy="19" r="2" /></svg></button>
                                </div>
                              </div>
                            {/if}
                          </span>
                        </div>
                      {/each}
                    {/if}
                  {/if}
                {/if}
              {/each}
            {:else}
            <div style="height: {visibleRange.paddingTop}px;" aria-hidden="true"></div>
            {#each flatViewMods.slice(visibleRange.start, visibleRange.end) as mod, sliceIdx (mod.id)}
              {@const i = visibleRange.start + sliceIdx}
              <div
                class="table-row"
                class:row-disabled={!mod.enabled}
                class:row-selected={selectedModId === mod.id}
                class:row-checked={selectedModIds.has(mod.id)}
                class:row-focused={focusedIndex === i}
                class:row-has-conflict={conflictModIds.has(mod.id)}
                class:row-dragging={dragRowIndex === i}
                class:row-drag-over={dragOverIndex === i && dragRowIndex !== null && dragRowIndex !== i}
                class:row-drag-above={dragOverIndex === i && dragRowIndex !== null && dragRowIndex > i}
                class:row-drag-below={dragOverIndex === i && dragRowIndex !== null && dragRowIndex < i}
                draggable="true"
                onclick={(e) => handleRowClick(e, mod, i)}
                oncontextmenu={(e) => handleRowContextMenu(e, mod)}
                ondragstart={(e) => handleRowDragStart(e, i)}
                ondragover={(e) => handleRowDragOver(e, i)}
                ondragend={handleRowDragEnd}
                ondrop={(e) => handleRowDrop(e, i)}
              >
                <!-- Bulk Select Checkbox -->
                <label class="col-check" onclick={(e) => e.stopPropagation()}>
                  <input type="checkbox" checked={selectedModIds.has(mod.id)} onchange={() => toggleSelectMod(mod.id)} />
                </label>

                <!-- Drag Handle -->
                <span class="col-grip">
                  <span class="drag-handle" title="Drag to reorder">
                    <svg width="12" height="12" viewBox="0 0 12 12" fill="currentColor">
                      <circle cx="4" cy="2.5" r="1" />
                      <circle cx="8" cy="2.5" r="1" />
                      <circle cx="4" cy="6" r="1" />
                      <circle cx="8" cy="6" r="1" />
                      <circle cx="4" cy="9.5" r="1" />
                      <circle cx="8" cy="9.5" r="1" />
                    </svg>
                  </span>
                </span>

                <!-- Toggle Switch -->
                <span class="col-toggle">
                  <button
                    class="toggle-switch"
                    class:toggle-on={mod.enabled}
                    class:toggle-busy={togglingMod === mod.id}
                    onclick={() => handleToggle(mod)}
                    title={mod.enabled ? "Disable mod" : "Enable mod"}
                    aria-label="{mod.enabled ? 'Disable' : 'Enable'} {mod.name}"
                    aria-pressed={mod.enabled}
                    role="switch"
                  >
                    <span class="toggle-track">
                      <span class="toggle-thumb"></span>
                    </span>
                  </button>
                </span>

                <!-- Name -->
                <span class="col-name">
                  <!-- eslint-disable-next-line svelte/no-at-html-tags -->
                  <span class="mod-name">{@html highlightMatch(mod.name, searchQuery)}</span>
                  {#if conflictModIds.has(mod.id)}
                    <span class="conflict-icon" title={getConflictTooltip(mod.id)}>
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
                        <line x1="12" y1="9" x2="12" y2="13" />
                        <line x1="12" y1="17" x2="12.01" y2="17" />
                      </svg>
                    </span>
                  {/if}
                  {#if mod.user_notes}
                    <span class="notes-icon" title={mod.user_notes}>
                      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
                        <polyline points="14 2 14 8 20 8" />
                        <line x1="16" y1="13" x2="8" y2="13" />
                        <line x1="16" y1="17" x2="8" y2="17" />
                      </svg>
                    </span>
                  {/if}
                </span>

                <!-- Category -->
                <span class="col-category">
                  {#if mod.auto_category}
                    {@const catColor = categoryColors[mod.auto_category] ?? '#6b7280'}
                    <span class="category-cell" style="color: {catColor};" title={mod.auto_category}>
                      {#if categoryIcons[mod.auto_category]}
                        <!-- eslint-disable-next-line svelte/no-at-html-tags -->
                        <svg class="category-icon" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html categoryIcons[mod.auto_category]}</svg>
                      {/if}
                      <span class="category-label">{mod.auto_category}</span>
                    </span>
                  {:else}
                    <span class="text-muted">\u2014</span>
                  {/if}
                </span>

                <!-- DL Origin -->
                <span class="col-origin">
                  {#if getModSourceUrl(mod)}
                    <button class="origin-label origin-{mod.source_type} origin-link" title="Open mod page" onclick={(e) => { e.stopPropagation(); openUrl(getModSourceUrl(mod)!); }}>
                      {originLabel(mod.source_type)}
                      <svg class="origin-link-icon" width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" /><polyline points="15 3 21 3 21 9" /><line x1="10" y1="14" x2="21" y2="3" /></svg>
                    </button>
                  {:else}
                    <span class="origin-label origin-{mod.source_type}">{originLabel(mod.source_type)}</span>
                  {/if}
                </span>

                <!-- Installed By -->
                <span class="col-source">
                  {#if mod.collection_name}
                    <span class="source-label source-collection" title={mod.collection_name}>{mod.collection_name}</span>
                  {:else}
                    <span class="source-label source-user">User</span>
                  {/if}
                </span>

                <!-- Version -->
                <span class="col-version">
                  <span class="version-text">{mod.version || "\u2014"}</span>
                  {#if updateMap.has(mod.id)}
                    {@const update = updateMap.get(mod.id)!}
                    <span class="update-badge" title={`Update available: v${update.latest_version}`}>
                      Update
                    </span>
                  {/if}
                </span>

                <!-- File Count -->
                <span class="col-files">
                  {mod.installed_files.length}
                </span>

                <!-- Date -->
                <span class="col-date">
                  {formatDate(mod.installed_at)}
                </span>

                <!-- Actions -->
                <span class="col-actions">
                  {#if confirmUninstall === mod.id}
                    <div class="confirm-actions">
                      <button class="btn btn-danger btn-sm" onclick={() => handleUninstall(mod.id)}>Yes</button>
                      <button class="btn btn-ghost btn-sm" onclick={() => (confirmUninstall = null)}>No</button>
                    </div>
                  {:else}
                    <div class="mod-action-group">
                      <button
                        class="mod-uninstall-btn"
                        onclick={(e) => { e.stopPropagation(); confirmUninstall = mod.id; }}
                        title="Uninstall mod"
                      >
                        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                          <polyline points="3 6 5 6 21 6" />
                          <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
                        </svg>
                      </button>
                      <div class="mod-overflow-wrap">
                        <button
                          class="mod-overflow-btn"
                          onclick={(e) => { e.stopPropagation(); overflowMenuModId = overflowMenuModId === mod.id ? null : mod.id; }}
                          title="More actions"
                        >
                          <svg width="13" height="13" viewBox="0 0 24 24" fill="currentColor">
                            <circle cx="12" cy="5" r="2" />
                            <circle cx="12" cy="12" r="2" />
                            <circle cx="12" cy="19" r="2" />
                          </svg>
                        </button>
                        {#if overflowMenuModId === mod.id}
                          <div class="mod-overflow-menu">
                            <button class="overflow-item" onclick={(e) => { e.stopPropagation(); overflowMenuModId = null; handleInstallOverMod(mod); }}>
                              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                <polyline points="23 4 23 10 17 10" /><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10" />
                              </svg>
                              Reinstall
                            </button>
                            {#if mod.nexus_mod_id}
                              <button class="overflow-item" onclick={(e) => { e.stopPropagation(); overflowMenuModId = null; handleCheckSingleUpdate(mod); }}>
                                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                  <circle cx="12" cy="12" r="10" /><line x1="12" y1="16" x2="12" y2="12" /><line x1="12" y1="8" x2="12.01" y2="8" />
                                </svg>
                                Check for Update
                              </button>
                              {#if endorsements.get(mod.nexus_mod_id!) === "Endorsed"}
                                <button class="overflow-item" onclick={(e) => { e.stopPropagation(); overflowMenuModId = null; handleAbstainMod(mod.id, mod.nexus_mod_id!); }} disabled={endorsingModId === mod.id}>
                                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                    <path d="M10 15v4a3 3 0 0 0 3 3l4-9V2H5.72a2 2 0 0 0-2 1.7l-1.38 9a2 2 0 0 0 2 2.3zm7-13h2.67A2.31 2.31 0 0 1 22 4v7a2.31 2.31 0 0 1-2.33 2H17" />
                                  </svg>
                                  Remove Endorsement
                                </button>
                              {:else}
                                <button class="overflow-item" onclick={(e) => { e.stopPropagation(); overflowMenuModId = null; handleEndorseMod(mod.id, mod.nexus_mod_id!, mod.version); }} disabled={endorsingModId === mod.id}>
                                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                    <path d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3zM7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3" />
                                  </svg>
                                  Endorse Mod
                                </button>
                              {/if}
                              <button class="overflow-item" onclick={(e) => { e.stopPropagation(); overflowMenuModId = null; openUrl(`https://www.nexusmods.com/skyrimspecialedition/mods/${mod.nexus_mod_id}`); }}>
                                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                  <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" /><polyline points="15 3 21 3 21 9" /><line x1="10" y1="14" x2="21" y2="3" />
                                </svg>
                                Open on Nexus
                              </button>
                            {/if}
                            {#if mod.staging_path}
                              <button class="overflow-item" onclick={(e) => { e.stopPropagation(); overflowMenuModId = null; handleReconfigureFomod(mod); }}>
                                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                  <circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z" />
                                </svg>
                                Reconfigure FOMOD
                              </button>
                            {/if}
                          </div>
                        {/if}
                      </div>
                    </div>
                  {/if}
                </span>
              </div>
            {/each}
            <div style="height: {visibleRange.paddingBottom}px;" aria-hidden="true"></div>

            <!-- Disabled Mods Separator -->
            {#if disabledFilteredMods.length > 0 && viewMode === "flat"}
              <button class="disabled-separator" onclick={() => disabledSectionCollapsed = !disabledSectionCollapsed}>
                <svg
                  class="group-chevron"
                  class:expanded={!disabledSectionCollapsed}
                  width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
                >
                  <path d="M4 2l4 4-4 4" />
                </svg>
                <span class="disabled-separator-label">Disabled</span>
                <span class="group-count">{disabledFilteredMods.length}</span>
                <span class="disabled-separator-line"></span>
              </button>
              {#if !disabledSectionCollapsed}
                {#each disabledFilteredMods as mod, di (mod.id)}
                  <div
                    class="table-row row-disabled"
                    class:row-selected={selectedModId === mod.id}
                    class:row-checked={selectedModIds.has(mod.id)}
                    class:row-has-conflict={conflictModIds.has(mod.id)}
                    onclick={(e) => handleRowClick(e, mod, -1 - di)}
                  >
                    <label class="col-check" onclick={(e) => e.stopPropagation()}>
                      <input type="checkbox" checked={selectedModIds.has(mod.id)} onchange={() => toggleSelectMod(mod.id)} />
                    </label>
                    <span class="col-grip"></span>
                    <span class="col-toggle"><button class="toggle-switch" class:toggle-on={mod.enabled} class:toggle-busy={togglingMod === mod.id} onclick={() => handleToggle(mod)} title="Enable mod" aria-label="Enable {mod.name}" aria-pressed={mod.enabled} role="switch"><span class="toggle-track"><span class="toggle-thumb"></span></span></button></span>
                    <span class="col-name"><span class="mod-name">{mod.name}</span></span>
                    <span class="col-category">{#if mod.auto_category}<span class="category-cell" style="color: {categoryColors[mod.auto_category] ?? '#6b7280'};" title={mod.auto_category}>{#if categoryIcons[mod.auto_category]}<svg class="category-icon" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html categoryIcons[mod.auto_category]}</svg>{/if}<span class="category-label">{mod.auto_category}</span></span>{:else}<span class="text-muted">&mdash;</span>{/if}</span>
                    <span class="col-origin">{#if mod.source_type === "nexus"}<span class="origin-label origin-nexus">Nexus</span>{:else}<span class="origin-label origin-manual">Manual</span>{/if}</span>
                    <span class="col-source">{#if mod.collection_name}<span class="source-label source-collection" title={mod.collection_name}>{mod.collection_name}</span>{:else}<span class="source-label source-user">User</span>{/if}</span>
                    <span class="col-version"><span class="version-text">{mod.version || "\u2014"}</span></span>
                    <span class="col-files">{mod.installed_files.length}</span>
                    <span class="col-date">{formatDate(mod.installed_at)}</span>
                    <span class="col-actions">
                      {#if confirmUninstall === mod.id}
                        <div class="confirm-actions"><button class="btn btn-danger btn-sm" onclick={() => handleUninstall(mod.id)}>Yes</button><button class="btn btn-ghost btn-sm" onclick={() => (confirmUninstall = null)}>No</button></div>
                      {:else}
                        <div class="mod-action-group">
                          <button class="mod-uninstall-btn" onclick={(e) => { e.stopPropagation(); confirmUninstall = mod.id; }} title="Uninstall mod"><svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6" /><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" /></svg></button>
                        </div>
                      {/if}
                    </span>
                  </div>
                {/each}
              {/if}
            {/if}
            {/if}
          </div>
        </div>
      </div>

      <!-- Detail Sidebar -->
      {#if detailMod}
        <ModDetailPanel
          mod={detailMod}
          nexusSlug={activeGame?.nexus_slug}
          {conflictModIds}
          {conflictDetails}
          {updateMap}
          {endorsements}
          {endorsingModId}
          {confirmUninstall}
          onclose={() => { detailMod = null; selectedModId = undefined; }}
          ontoggle={handleToggle}
          onuninstall={handleUninstall}
          onconfirmuninstall={(id) => confirmUninstall = id}
          onsavenotes={handleSaveNotes}
          onendorse={handleEndorseMod}
          onabstain={handleAbstainMod}
          onreinstall={handleInstallOverMod}
          onreload={() => { const g = pickedGame ?? $selectedGame; if (g) loadMods(g); }}
          onnavigatemod={(targetMod, iniFile) => { detailScrollToIni = iniFile ?? null; detailMod = targetMod; selectedModId = targetMod.id; if (iniFile) setTimeout(() => { detailScrollToIni = null; }, 2000); }}
          scrollToIni={detailScrollToIni}
        />
      {/if}
      </div>
    {/if}
    </div><!-- end content-main -->

    <!-- RIGHT: Info sidebar -->
    {#if !loadingMods && $installedMods.length > 0}
      <div class="content-sidebar">
        <!-- Deployment Status Card -->
        {#if deployHealth}
          <div class="sidebar-card">
            <div class="sidebar-card-header">
              <span class="sidebar-card-title">Deployment</span>
              <span class="sidebar-chip" class:chip-green={deployHealth.is_deployed} class:chip-amber={!deployHealth.is_deployed}>
                {deployHealth.is_deployed ? "Active" : "Purged"}
              </span>
            </div>
            <div class="sidebar-stats">
              <div class="sidebar-stat">
                <span class="sidebar-stat-value">{deployHealth.total_enabled}</span>
                <span class="sidebar-stat-label">Enabled</span>
              </div>
              <div class="sidebar-stat">
                <span class="sidebar-stat-value">{deployHealth.total_deployed}</span>
                <span class="sidebar-stat-label">Files</span>
              </div>
              <div class="sidebar-stat">
                <span class="sidebar-stat-value" class:health-warning={(deployHealth.conflict_count ?? 0) > 0}>{deployHealth.conflict_count ?? 0}</span>
                <span class="sidebar-stat-label">Conflicts</span>
              </div>
            </div>
            {#if deployHealth.deploy_method && deployHealth.deploy_method !== "none"}
              <span class="sidebar-detail">{deployHealth.deploy_method === "hardlink" ? "Hardlinks (0 extra disk)" : "File copies"}</span>
            {/if}
            {#if (deployHealth.conflict_count ?? 0) > 0}
              <button class="sidebar-link" onclick={() => showConflictPanel = !showConflictPanel}>
                Resolve conflicts
              </button>
            {/if}
          </div>
        {/if}

        <!-- Disk Budget -->
        {#if activeGame}
          <DiskBudgetPanel gameId={activeGame.game_id} bottleName={activeGame.bottle_name} />
        {/if}

        <!-- Pre-flight Checks -->
        {#if activeGame}
          <button class="panel-section-toggle" onclick={() => showPreflightPanel = !showPreflightPanel}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
            </svg>
            <span>Pre-Deployment Checks</span>
            <svg class="section-chevron" class:open={showPreflightPanel} width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
              <path d="M3 4.5L6 7.5L9 4.5" />
            </svg>
          </button>
          {#if showPreflightPanel}
            <PreflightPanel gameId={activeGame.game_id} bottleName={activeGame.bottle_name} />
          {/if}
        {/if}

        <!-- Dependencies -->
        <button class="panel-section-toggle" onclick={() => showDependencyPanel = !showDependencyPanel}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="18" cy="5" r="3" />
            <circle cx="6" cy="12" r="3" />
            <circle cx="18" cy="19" r="3" />
            <line x1="8.59" y1="13.51" x2="15.42" y2="17.49" />
            <line x1="15.41" y1="6.51" x2="8.59" y2="10.49" />
          </svg>
          <span>Dependencies</span>
          <svg class="section-chevron" class:open={showDependencyPanel} width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M3 4.5L6 7.5L9 4.5" />
          </svg>
        </button>
        {#if showDependencyPanel && activeGame}
          <DependencyPanel
            gameId={activeGame.game_id}
            bottleName={activeGame.bottle_name}
            mods={$installedMods}
            {selectedModId}
          />
        {/if}

        <!-- Session History -->
        <button class="panel-section-toggle" onclick={() => showSessionPanel = !showSessionPanel}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="10" />
            <polyline points="12 6 12 12 16 14" />
          </svg>
          <span>Session History</span>
          <svg class="section-chevron" class:open={showSessionPanel} width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M3 4.5L6 7.5L9 4.5" />
          </svg>
        </button>
        {#if showSessionPanel && activeGame}
          <SessionHistoryPanel gameId={activeGame.game_id} bottleName={activeGame.bottle_name} />
        {/if}
      </div><!-- end content-sidebar -->
    {/if}
    </div><!-- end content-grid -->
  {/if}

  <!-- Context Menu -->
  {#if contextMenuMod}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="context-overlay" onclick={closeContextMenu}></div>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="context-menu" style="left: {contextMenuX}px; top: {contextMenuY}px;" onclick={(e) => e.stopPropagation()}>
      <button class="context-item" onclick={() => { handleToggle(contextMenuMod!); closeContextMenu(); }}>
        {contextMenuMod.enabled ? "Disable" : "Enable"}
      </button>
      <button class="context-item" onclick={() => { detailMod = contextMenuMod; selectedModId = contextMenuMod!.id; closeContextMenu(); }}>
        Edit Notes
      </button>
      <div class="context-separator"></div>
      <button class="context-item" onclick={() => { handleInstallOverMod(contextMenuMod!); closeContextMenu(); }}>
        Reinstall
      </button>
      {#if contextMenuMod.nexus_mod_id}
        <button class="context-item" onclick={() => { handleCheckSingleUpdate(contextMenuMod!); closeContextMenu(); }}>
          Check for Update
        </button>
        {#if endorsements.get(contextMenuMod.nexus_mod_id) === "Endorsed"}
          <button class="context-item" onclick={() => { handleAbstainMod(contextMenuMod!.id, contextMenuMod!.nexus_mod_id!); closeContextMenu(); }}>
            Remove Endorsement
          </button>
        {:else}
          <button class="context-item" onclick={() => { handleEndorseMod(contextMenuMod!.id, contextMenuMod!.nexus_mod_id!, contextMenuMod!.version); closeContextMenu(); }}>
            Endorse Mod
          </button>
        {/if}
        <button class="context-item" onclick={() => { openUrl(`https://www.nexusmods.com/skyrimspecialedition/mods/${contextMenuMod!.nexus_mod_id}`); closeContextMenu(); }}>
          Open on Nexus
        </button>
      {/if}
      <div class="context-separator"></div>
      <button class="context-item context-danger" onclick={() => { confirmUninstall = contextMenuMod!.id; closeContextMenu(); }}>
        Uninstall
      </button>
    </div>
  {/if}
</div>

{#if showFomodWizard && fomodInstaller}
  <div class="fomod-wizard-overlay">
    <FomodWizard
      installer={fomodInstaller}
      onComplete={handleFomodComplete}
      onCancel={() => { showFomodWizard = false; fomodInstaller = null; fomodTargetMod = null; }}
    />
  </div>
{/if}

<ConfirmDialog
  open={duplicateDialog !== null}
  title="Duplicate Mod Detected"
  message={duplicateDialog ? `"${duplicateDialog.oldMod.name}" (v${duplicateDialog.oldMod.version || "?"}) is already installed. Would you like to uninstall the old version?` : ""}
  details={duplicateDialog ? [`New: v${duplicateDialog.newMod.version || "?"}`, `Old: v${duplicateDialog.oldMod.version || "?"}`] : []}
  confirmLabel="Uninstall Old Version"
  confirmDanger={true}
  onConfirm={async () => {
    if (duplicateDialog && activeGame) {
      try {
        await uninstallMod(duplicateDialog.oldMod.id, activeGame.game_id, activeGame.bottle_name);
        showSuccess(`Removed old version of "${duplicateDialog.oldMod.name}"`);
        await loadMods(activeGame);
      } catch (e) {
        showError(`Failed to uninstall old version: ${e}`);
      }
    }
    duplicateDialog = null;
  }}
  onCancel={() => duplicateDialog = null}
/>

{#if showModlistNamePrompt}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="modlist-prompt-overlay" onclick={() => { showModlistNamePrompt = false; pendingInstallFilePath = null; }}>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="modlist-prompt-card" onclick={(e) => e.stopPropagation()}>
      <h3 class="modlist-prompt-title">Name Your Modlist</h3>
      <p class="modlist-prompt-desc">You don't have an active modlist. Name your current mod setup so new mods are grouped together.</p>
      <form onsubmit={(e) => { e.preventDefault(); confirmModlistName(); }}>
        <input
          class="modlist-prompt-input"
          type="text"
          bind:value={modlistNameInput}
          placeholder="Modlist name..."
          autofocus
          onkeydown={(e) => { if (e.key === "Escape") { showModlistNamePrompt = false; pendingInstallFilePath = null; } }}
        />
        <div class="modlist-prompt-actions">
          <button type="button" class="btn btn-ghost" onclick={() => { showModlistNamePrompt = false; pendingInstallFilePath = null; }}>Cancel</button>
          <button type="submit" class="btn btn-primary" disabled={!modlistNameInput.trim()}>Continue</button>
        </div>
      </form>
    </div>
  </div>
{/if}

{#if showImportWizard}
  <ModlistImportWizard
    onclose={() => showImportWizard = false}
    oncomplete={() => { showImportWizard = false; const g = pickedGame ?? $selectedGame; if (g) loadMods(g); }}
  />
{/if}

{#if showBisect}
  <ModBisect
    mods={$installedMods}
    onClose={() => showBisect = false}
    onComplete={async (culprit) => {
      showBisect = false;
      if (deploying) return;
      const g = pickedGame ?? $selectedGame;
      if (g) {
        deploying = true;
        try {
          await toggleMod(culprit.id, g.game_id, g.bottle_name, false);
          await redeployAllMods(g.game_id, g.bottle_name);
          loadMods(g);
          showSuccess(`Disabled "${culprit.name}" — the likely culprit.`);
        } finally {
          deploying = false;
        }
      }
    }}
  />
{/if}


<style>
  .game-fixes-toggle {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--text-secondary);
    cursor: pointer;
    user-select: none;
  }

  .game-fixes-toggle input[type="checkbox"] {
    accent-color: var(--accent);
    cursor: pointer;
  }

  /* ============================
     FOMOD Wizard Overlay
     ============================ */
  .fomod-wizard-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    backdrop-filter: var(--glass-blur-light);
    -webkit-backdrop-filter: var(--glass-blur-light);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 2000;
  }

  /* ============================
     Page Layout
     ============================ */
  .mods-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: var(--space-4) var(--space-5);
    gap: var(--space-3);
    overflow: hidden;
    position: relative;
  }

  @media (max-width: 800px) {
    .mods-page {
      padding: var(--space-3) var(--space-3);
      gap: var(--space-2);
    }
  }

  /* Two-column content grid */
  .content-grid {
    display: grid;
    grid-template-columns: 1fr 260px;
    gap: var(--space-4);
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }

  @media (max-width: 1100px) {
    .content-grid {
      grid-template-columns: 1fr 220px;
      gap: var(--space-3);
    }
  }

  @media (max-width: 900px) {
    .content-grid {
      grid-template-columns: 1fr;
    }
    .content-sidebar {
      display: none;
    }
  }

  .content-main {
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
  }

  .content-sidebar {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    overflow-y: auto;
    min-height: 0;
  }

  /* Sidebar cards */
  .sidebar-card {
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    padding: var(--space-3) var(--space-4);
  }

  .sidebar-card-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--space-3);
  }

  .sidebar-card-title {
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-secondary);
  }

  .sidebar-chip {
    font-size: 11px;
    font-weight: 600;
    padding: 1px 8px;
    border-radius: 999px;
  }

  .sidebar-chip.chip-green {
    color: var(--green);
    background: rgba(48, 209, 88, 0.12);
  }

  .sidebar-chip.chip-amber {
    color: var(--yellow);
    background: rgba(255, 214, 10, 0.12);
  }

  .sidebar-stats {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: var(--space-2);
    text-align: center;
  }

  .sidebar-stat-value {
    display: block;
    font-size: 18px;
    font-weight: 700;
    color: var(--text-primary);
    line-height: 1.2;
  }

  .sidebar-stat-label {
    display: block;
    font-size: 11px;
    color: var(--text-tertiary);
    margin-top: 1px;
  }

  .sidebar-detail {
    display: block;
    font-size: 11px;
    color: var(--text-tertiary);
    text-align: center;
    margin-top: var(--space-2);
  }

  .sidebar-link {
    display: block;
    font-size: 12px;
    color: var(--accent);
    text-align: center;
    margin-top: var(--space-2);
    background: none;
    border: none;
    cursor: pointer;
    padding: 0;
  }

  .sidebar-link:hover {
    text-decoration: underline;
  }

  /* ============================
     Game Picker
     ============================ */
  .picker-container {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    flex: 1;
    padding: var(--space-12) var(--space-6);
    text-align: center;
  }

  .picker-icon {
    color: var(--text-tertiary);
    margin-bottom: var(--space-5);
  }

  .picker-title {
    font-size: 22px;
    font-weight: 700;
    letter-spacing: -0.02em;
    margin-bottom: var(--space-2);
  }

  .picker-subtitle {
    color: var(--text-secondary);
    font-size: 14px;
    line-height: 1.5;
    margin-bottom: var(--space-8);
    max-width: 340px;
  }

  .picker-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
  }

  .picker-empty p {
    color: var(--text-secondary);
    font-size: 14px;
  }

  .picker-empty-hint {
    color: var(--text-tertiary) !important;
    font-size: 13px !important;
    margin-bottom: var(--space-2);
  }

  .game-cards {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    width: 100%;
    max-width: 460px;
  }

  .game-card {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    padding: var(--space-4) var(--space-5);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    box-shadow: var(--glass-refraction), var(--glass-edge-shadow);
    text-align: left;
    transition:
      background var(--duration) var(--ease),
      border-color var(--duration) var(--ease),
      box-shadow var(--duration) var(--ease);
  }

  .game-card:hover {
    background: var(--surface-hover);
    border-color: var(--accent-muted);
    box-shadow: var(--glass-refraction), var(--glass-edge-shadow), var(--shadow-sm);
  }

  .game-card:active {
    background: var(--surface-active);
  }

  .game-card-content {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .game-card-name {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .game-card-bottle {
    font-size: 12px;
    color: var(--text-tertiary);
    font-family: var(--font-mono);
    letter-spacing: 0;
  }

  .game-card-chevron {
    color: var(--text-quaternary);
    transition: color var(--duration-fast) var(--ease), transform var(--duration-fast) var(--ease);
  }

  .game-card:hover .game-card-chevron {
    color: var(--accent);
    transform: translateX(2px);
  }

  /* ============================
     Game Banner Header
     ============================ */
  .game-banner {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    box-shadow: var(--glass-refraction), var(--glass-edge-shadow);
    flex-shrink: 0;
  }

  .game-banner-icon {
    flex-shrink: 0;
    color: var(--text-primary);
    display: flex;
    align-items: center;
    justify-content: center;
    width: 40px;
    height: 40px;
  }

  .game-banner-info {
    flex: 1;
    min-width: 0;
  }

  .game-banner-title {
    font-size: 16px;
    font-weight: 700;
    letter-spacing: -0.02em;
    color: var(--text-primary);
    line-height: 1.2;
  }

  .game-banner-meta {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: 2px;
    font-size: 13px;
  }

  .meta-bottle {
    color: var(--text-tertiary);
    font-family: var(--font-mono);
    font-size: 12px;
    letter-spacing: 0;
  }

  .meta-separator {
    color: var(--text-quaternary);
  }

  .meta-mods {
    color: var(--text-secondary);
    font-weight: 500;
  }

  .meta-skse {
    color: var(--green);
    font-size: 12px;
    font-weight: 500;
  }

  .modlist-selector {
    margin-top: 4px;
  }

  .modlist-dropdown {
    padding: 3px 8px;
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-sm);
    color: var(--text-secondary);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    min-width: 140px;
  }

  .modlist-dropdown:focus {
    outline: none;
    border-color: var(--accent-muted);
  }

  .game-banner-actions {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-2);
    flex-shrink: 0;
  }

  /* ============================
     Action Bar
     ============================ */
  .action-bar {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    flex-wrap: wrap;
    gap: var(--space-2);
    flex-shrink: 0;
  }

  /* ============================
     Buttons
     ============================ */
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    border-radius: var(--radius-sm);
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
    transition:
      background var(--duration-fast) var(--ease),
      color var(--duration-fast) var(--ease),
      box-shadow var(--duration-fast) var(--ease),
      opacity var(--duration-fast) var(--ease);
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

  .btn-primary:active:not(:disabled) {
    filter: brightness(0.92);
  }

  .btn-primary:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-secondary {
    background: var(--surface);
    color: var(--text-primary);
    border: 1px solid var(--separator);
  }

  .btn-secondary:hover {
    background: var(--surface-hover);
    border-color: var(--separator-opaque);
  }

  .btn-danger {
    background: var(--red-subtle);
    color: var(--red);
  }

  .btn-danger:hover {
    background: rgba(255, 69, 58, 0.25);
  }

  .btn-ghost {
    background: transparent;
    color: var(--text-secondary);
  }

  .btn-ghost:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .btn-ghost:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-ghost-danger {
    background: transparent;
    color: var(--text-tertiary);
  }

  .btn-ghost-danger:hover {
    background: var(--red-subtle);
    color: var(--red);
  }

  .btn-sm {
    padding: var(--space-1) var(--space-3);
    font-size: 12px;
    font-weight: 500;
    border-radius: var(--radius-sm);
  }

  /* ============================
     Spinner
     ============================ */
  .spinner {
    display: inline-block;
    width: 14px;
    height: 14px;
    border: 2px solid var(--text-tertiary);
    border-top-color: var(--text-primary);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  .spinner-sm {
    width: 12px;
    height: 12px;
    border-width: 1.5px;
    border-color: var(--text-tertiary);
    border-top-color: var(--text-primary);
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  /* ============================
     Empty State
     ============================ */
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    flex: 1;
    padding: var(--space-12) var(--space-6);
    background: var(--surface-glass);
    border: 1px solid var(--separator);
    border-radius: var(--radius-lg);
    box-shadow: var(--glass-refraction), var(--glass-edge-shadow);
    backdrop-filter: var(--glass-blur-light);
    text-align: center;
    gap: var(--space-3);
  }

  .empty-icon {
    color: var(--text-quaternary);
    margin-bottom: var(--space-2);
  }

  .empty-title {
    font-size: 17px;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .empty-description {
    font-size: 13px;
    color: var(--text-tertiary);
    max-width: 320px;
    line-height: 1.5;
    margin-bottom: var(--space-2);
  }

  /* ============================
     Mod Layout (list + detail)
     ============================ */
  .mod-layout {
    display: flex;
    gap: var(--space-4);
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }

  .mod-layout.has-detail .mod-table-container {
    flex: 1;
    min-width: 0;
  }

  /* ============================
     Mod Table
     ============================ */
  .mod-table-container {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border-radius: var(--radius-lg);
    background: var(--bg-primary);
    box-shadow: var(--glass-refraction), var(--glass-edge-shadow);
    min-height: 200px;
  }

  .reordering-active {
    opacity: 0.7;
    pointer-events: none;
  }

  .mod-table {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }

  .table-header {
    display: grid;
    grid-template-columns: var(--grid-cols, 24px 28px 48px minmax(0, 1fr) 100px 68px 110px 72px 48px 90px 64px);
    padding: var(--space-2) var(--space-3);
    background: var(--bg-secondary);
    border-bottom: 1px solid var(--separator);
    flex-shrink: 0;
    z-index: 2;
  }

  .table-header.resizing {
    user-select: none;
  }

  .table-header > span,
  .table-header > button {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }


  .sortable-header {
    cursor: pointer;
    user-select: none;
    transition: color var(--duration-fast) var(--ease), background var(--duration-fast) var(--ease);
    background: none;
    border: none;
    padding: var(--space-1) var(--space-2);
    margin: calc(-1 * var(--space-1)) 0;
    display: flex;
    align-items: center;
    gap: 4px;
    font-family: inherit;
    text-align: left;
  }

  .sortable-header:hover {
    color: var(--text-primary);
    background: var(--surface);
  }

  .sort-arrow {
    font-size: 8px;
    margin-left: 2px;
    color: var(--accent);
  }

  /* Make header cells position:relative for the resize handle */
  .table-header > span,
  .table-header > button.sortable-header {
    position: relative;
  }

  /* Column resize handles — use .table-header .col-resize to beat
     the .table-header span rule that would otherwise override
     position/overflow with relative/hidden */
  .table-header .col-resize {
    position: absolute;
    right: 0;
    top: 0;
    bottom: 0;
    width: 6px;
    cursor: col-resize;
    z-index: 3;
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: visible;
    text-overflow: clip;
    white-space: normal;
    font-size: inherit;
    font-weight: inherit;
    color: inherit;
    text-transform: none;
    letter-spacing: normal;
  }

  .table-header .col-resize::after {
    content: '';
    width: 1px;
    height: 60%;
    background: var(--separator);
    transition: background 0.15s ease, width 0.15s ease;
    border-radius: 1px;
  }

  .table-header .col-resize:hover::after {
    background: var(--accent);
    width: 2px;
  }

  .table-header.resizing .col-resize::after {
    background: var(--accent);
    width: 2px;
  }

  .table-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }

  .table-row {
    display: grid;
    grid-template-columns: var(--grid-cols, 24px 28px 48px minmax(0, 1fr) 100px 68px 110px 72px 48px 90px 64px);
    padding: var(--space-2) var(--space-3);
    align-items: center;
    font-size: 13px;
    height: 36px;
    box-sizing: border-box;
    transition:
      background var(--duration-fast) var(--ease),
      opacity var(--duration-fast) var(--ease),
      box-shadow var(--duration-fast) var(--ease);
  }

  /* Narrow window: hide Category, Origin, Source, Files, Date columns */
  @media (max-width: 1200px) {
    .mod-table {
      --grid-cols: 24px 28px 48px minmax(0, 1fr) 0px 0px 0px 64px 0px 0px 60px !important;
    }
    .col-category,
    .col-origin,
    .col-source,
    .col-files,
    .col-date {
      display: none;
    }
  }

  .table-row:nth-child(even) {
    background: var(--surface-subtle);
  }

  .table-row:hover {
    background: var(--surface-hover);
  }

  .table-row.row-disabled {
    opacity: 0.45;
  }

  .table-row.row-disabled:hover {
    opacity: 0.6;
  }

  /* --- Drag reorder visual feedback --- */

  .table-row.row-dragging {
    opacity: 0.35;
    background: var(--surface-active);
  }

  .table-row.row-drag-above {
    box-shadow: inset 0 2px 0 0 var(--system-accent);
  }

  .table-row.row-drag-below {
    box-shadow: inset 0 -2px 0 0 var(--system-accent);
  }

  /* ============================
     Drag Handle
     ============================ */
  .col-grip {
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .drag-handle {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    color: var(--text-quaternary);
    cursor: grab;
    border-radius: 4px;
    transition: color var(--duration-fast) var(--ease), background var(--duration-fast) var(--ease);
  }

  .drag-handle:hover {
    color: var(--text-secondary);
    background: var(--surface-hover);
  }

  .drag-handle:active {
    cursor: grabbing;
    color: var(--text-primary);
  }

  /* ============================
     Toggle Switch (Pill)
     ============================ */
  .toggle-switch {
    display: inline-flex;
    align-items: center;
    padding: 0;
    background: transparent;
    cursor: pointer;
  }

  .toggle-track {
    position: relative;
    width: 32px;
    height: 18px;
    border-radius: 9px;
    background: var(--bg-tertiary);
    transition:
      background var(--duration) var(--ease),
      box-shadow var(--duration) var(--ease);
  }

  .toggle-on .toggle-track {
    background: var(--green);
    box-shadow: 0 0 8px rgba(48, 209, 88, 0.25);
  }

  .toggle-thumb {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
    transition: transform var(--duration) var(--ease);
  }

  .toggle-on .toggle-thumb {
    transform: translateX(14px);
  }

  .toggle-busy .toggle-track {
    opacity: 0.6;
  }

  /* ============================
     Mod Name & Badges
     ============================ */
  .col-name {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .mod-name {
    font-weight: 500;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
    flex: 1;
  }

  .nexus-badge {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    padding: 1px 6px;
    border-radius: 4px;
    background: var(--accent-subtle);
    color: var(--accent);
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }

  /* ============================
     Conflict Indicator
     ============================ */
  .table-row.row-has-conflict {
    border-left: 2px solid var(--yellow);
  }

  .conflict-icon {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--yellow);
    cursor: help;
    position: relative;
  }

  .conflict-icon:hover {
    color: #ffcc00;
  }

  /* ============================
     Update Badge
     ============================ */
  .col-version {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-secondary);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    overflow: hidden;
  }

  .version-text {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .version-text {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .update-badge {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    padding: 1px 7px;
    border-radius: 100px;
    background: var(--system-accent-subtle);
    color: var(--system-accent);
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    cursor: help;
    transition: background var(--duration-fast) var(--ease);
  }

  .update-badge:hover {
    background: rgba(10, 132, 255, 0.25);
  }

  .update-count-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    border-radius: 100px;
    background: var(--system-accent);
    color: white;
    font-size: 10px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    margin-left: 2px;
  }

  .btn-sm {
    padding: var(--space-1) var(--space-3);
    font-size: 12px;
  }

  /* ============================
     Table Columns
     ============================ */
  .col-toggle {
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .col-files,
  .col-date {
    color: var(--text-secondary);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .col-files {
    font-family: var(--font-mono);
    font-size: 12px;
    letter-spacing: 0;
  }

  .col-actions {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    overflow: visible;
    position: relative;
  }

  .mod-action-group {
    display: flex;
    align-items: center;
    gap: 2px;
  }

  .mod-uninstall-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    border-radius: var(--radius-sm);
    color: var(--red);
    background: transparent;
    border: 1px solid color-mix(in srgb, var(--red) 30%, transparent);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
  }

  .mod-uninstall-btn:hover {
    background: color-mix(in srgb, var(--red) 12%, transparent);
    border-color: color-mix(in srgb, var(--red) 50%, transparent);
  }

  .mod-overflow-wrap {
    position: relative;
  }

  .mod-overflow-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    border-radius: var(--radius-sm);
    color: var(--text-quaternary);
    background: transparent;
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
  }

  .mod-overflow-btn:hover {
    background: var(--surface-hover);
    color: var(--text-secondary);
  }

  .mod-overflow-menu {
    position: absolute;
    right: 0;
    top: 100%;
    margin-top: 4px;
    min-width: 180px;
    background: var(--bg-elevated);
    border: 1px solid var(--separator-opaque);
    border-radius: var(--radius);
    box-shadow: var(--shadow-lg);
    z-index: 100;
    padding: 4px;
    animation: dropdownIn var(--duration-fast) var(--ease-out);
  }

  @keyframes dropdownIn {
    from { transform: translateY(-4px); opacity: 0; }
    to { transform: translateY(0); opacity: 1; }
  }

  .overflow-item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 10px;
    border-radius: calc(var(--radius) - 2px);
    font-size: 12px;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
    text-align: left;
    white-space: nowrap;
  }

  .overflow-item:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .confirm-actions {
    display: flex;
    gap: var(--space-1);
    align-items: center;
    position: absolute;
    right: 0;
    top: 50%;
    transform: translateY(-50%);
    z-index: 10;
    background: var(--surface-primary);
    padding: 2px 6px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border-primary);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3);
    white-space: nowrap;
  }

  /* --- Nexus link --- */

  .nexus-link {
    text-decoration: none !important;
  }

  /* --- Play Button --- */

  .btn-icon-sm {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    padding: 0;
    background: var(--surface-hover);
    color: var(--text-secondary);
    border: none;
    border-radius: var(--radius);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease), color var(--duration-fast) var(--ease);
    flex-shrink: 0;
  }

  .btn-icon-sm:hover {
    background: var(--surface-active);
    color: var(--text-primary);
  }

  .play-button-group {
    display: flex;
    align-items: stretch;
    position: relative;
  }

  .btn-play {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-5);
    background: var(--green);
    color: #fff;
    font-size: 13px;
    font-weight: 600;
    border-radius: var(--radius) 0 0 var(--radius);
    white-space: nowrap;
    transition: background var(--duration-fast) var(--ease),
                box-shadow var(--duration-fast) var(--ease);
  }

  .btn-play:hover:not(:disabled) {
    filter: brightness(1.1);
    box-shadow: 0 1px 6px rgba(48, 209, 88, 0.3);
  }

  .btn-play:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .play-button-group .btn-play:only-child {
    border-radius: var(--radius);
  }

  .btn-play-dropdown {
    display: inline-flex;
    align-items: center;
    padding: 0 var(--space-2);
    background: var(--green);
    color: #fff;
    border-left: 1px solid rgba(255, 255, 255, 0.2);
    border-radius: 0 var(--radius) var(--radius) 0;
    transition: background var(--duration-fast) var(--ease);
  }

  .btn-play-dropdown:hover {
    filter: brightness(1.1);
  }

  .spinner-play {
    border-color: rgba(255, 255, 255, 0.3);
    border-top-color: #fff;
  }

  /* --- SKSE Dropdown --- */

  .skse-dropdown {
    position: absolute;
    top: 100%;
    right: 0;
    margin-top: var(--space-1);
    background: var(--bg-elevated);
    border: 1px solid var(--separator-opaque);
    border-radius: var(--radius);
    box-shadow: var(--shadow-lg);
    min-width: 180px;
    z-index: 100;
    overflow: hidden;
  }

  .dropdown-item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: var(--space-2) var(--space-3);
    font-size: 13px;
    color: var(--text-primary);
    text-align: left;
    transition: background var(--duration-fast) var(--ease);
  }

  .dropdown-item:hover {
    background: var(--surface-hover);
  }

  .dropdown-check {
    width: 16px;
    height: 16px;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--green);
  }

  .dropdown-divider {
    height: 1px;
    background: var(--separator);
  }

  .dropdown-info {
    padding: var(--space-2) var(--space-3);
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .dropdown-item-muted {
    color: var(--text-tertiary);
    font-size: 12px;
  }

  /* --- Tools dropdown --- */

  .tools-dropdown-wrap {
    position: relative;
  }

  .tools-dropdown {
    position: absolute;
    top: 100%;
    right: 0;
    margin-top: var(--space-1);
    background: var(--bg-elevated);
    border: 1px solid var(--separator-opaque);
    border-radius: var(--radius);
    box-shadow: var(--shadow-lg);
    min-width: 220px;
    z-index: 100;
    overflow: hidden;
  }

  .tools-dropdown .dropdown-item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .dropdown-section-label {
    padding: var(--space-2) var(--space-3);
    font-size: 10px;
    font-weight: 600;
    color: var(--text-tertiary);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    user-select: none;
  }

  .tool-launch-name {
    flex: 1;
  }

  .tool-launch-cat {
    font-size: 10px;
    color: var(--text-tertiary);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  /* --- SKSE Banner --- */

  .skse-banner {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    padding: var(--space-3) var(--space-4);
    background: var(--yellow-subtle);
    border: 1px solid var(--yellow-subtle);
    border-radius: var(--radius);
  }

  .skse-banner-icon {
    color: var(--yellow);
    flex-shrink: 0;
  }

  .skse-banner-content {
    flex: 1;
    min-width: 0;
  }

  .skse-banner-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .skse-banner-text {
    font-size: 12px;
    color: var(--text-secondary);
    line-height: 1.4;
  }

  .skse-banner-actions {
    display: flex;
    gap: var(--space-2);
    flex-shrink: 0;
  }

  /* --- Downgrade Banner --- */

  .downgrade-banner {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    padding: var(--space-3) var(--space-4);
    background: color-mix(in srgb, var(--blue) 10%, transparent);
    border: 1px solid color-mix(in srgb, var(--blue) 20%, transparent);
    border-radius: var(--radius);
  }

  .downgrade-banner .skse-banner-icon {
    color: var(--blue);
  }

  /* --- Drag & Drop Overlay --- */

  /* position: relative merged into main .mods-page rule */

  .drag-active {
    outline: 2px dashed var(--accent);
    outline-offset: -2px;
    border-radius: var(--radius-lg);
  }

  .drop-overlay {
    position: absolute;
    inset: 0;
    z-index: 200;
    display: flex;
    align-items: center;
    justify-content: center;
    background: color-mix(in srgb, var(--bg-base) 85%, transparent);
    backdrop-filter: var(--glass-blur-light);
    border-radius: var(--radius-lg);
  }

  .drop-overlay-content {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    color: var(--accent);
  }

  .drop-overlay-content p {
    font-size: 16px;
    font-weight: 600;
    letter-spacing: -0.01em;
  }

  /* ============================
     Deploy Status Badge
     ============================ */
  .deploy-status {
    display: inline-flex;
    align-items: center;
    padding: 2px 8px;
    border-radius: 100px;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }

  .status-deployed {
    background: color-mix(in srgb, var(--green) 15%, transparent);
    color: var(--green);
  }

  .status-purged {
    background: color-mix(in srgb, var(--yellow) 15%, transparent);
    color: var(--yellow);
  }

  /* ============================
     Deployment Health Panel
     ============================ */
  .health-toggle {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    padding: var(--space-2) var(--space-4);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease);
  }

  .health-toggle:hover {
    background: var(--surface-hover);
  }

  .health-summary {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    font-size: 12px;
  }

  .health-chip {
    display: inline-flex;
    align-items: center;
    padding: 1px 8px;
    border-radius: 100px;
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }

  .chip-green {
    background: color-mix(in srgb, var(--green) 15%, transparent);
    color: var(--green);
  }

  .chip-amber {
    background: color-mix(in srgb, var(--yellow) 15%, transparent);
    color: var(--yellow);
  }

  .health-stat {
    color: var(--text-secondary);
    font-weight: 500;
  }

  .health-warning {
    color: var(--yellow) !important;
  }

  .health-method {
    color: var(--green);
  }

  .health-chevron {
    color: var(--text-tertiary);
    transition: transform var(--duration-fast) var(--ease);
  }

  .health-chevron.open {
    transform: rotate(180deg);
  }

  .health-details {
    padding: var(--space-3) var(--space-4);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-top: none;
    border-radius: 0 0 var(--radius) var(--radius);
    margin-top: -1px;
  }

  .health-grid {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: var(--space-4);
  }

  .health-item {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .health-label {
    font-size: 11px;
    color: var(--text-tertiary);
    font-weight: 500;
  }

  .health-value {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  /* ============================
     Panel Section Toggles
     ============================ */
  .panel-section-toggle {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: var(--space-2) var(--space-4);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    cursor: pointer;
    font-size: 13px;
    font-weight: 600;
    color: var(--text-secondary);
    transition: background var(--duration-fast) var(--ease);
  }

  .panel-section-toggle:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .panel-section-toggle > svg:first-child {
    color: var(--accent);
    flex-shrink: 0;
  }

  .panel-section-toggle > span {
    flex: 1;
    text-align: left;
  }

  .section-chevron {
    color: var(--text-tertiary);
    transition: transform var(--duration-fast) var(--ease);
    flex-shrink: 0;
  }

  .section-chevron.open {
    transform: rotate(180deg);
  }

  /* ============================
     Selected Row
     ============================ */
  .row-selected {
    background: color-mix(in srgb, var(--system-accent) 8%, transparent) !important;
    border-left: 2px solid var(--system-accent);
  }

  .row-checked {
    background: color-mix(in srgb, var(--system-accent) 12%, transparent);
    box-shadow: inset 2px 0 0 var(--system-accent);
  }

  .row-checked.row-selected {
    background: color-mix(in srgb, var(--system-accent) 18%, transparent) !important;
    box-shadow: inset 2px 0 0 var(--system-accent);
  }

  /* ============================
     Search & Filter Bar
     ============================ */
  .filter-bar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-shrink: 0;
    margin-bottom: var(--space-3);
  }

  .search-box {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-3);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-sm);
    flex: 1;
    max-width: 480px;
    color: var(--text-tertiary);
    transition: border-color var(--duration-fast) var(--ease);
    flex-wrap: wrap;
    min-height: 32px;
  }

  .search-box:focus-within {
    border-color: var(--accent-muted);
  }

  .facet-pill {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    padding: 2px 8px;
    background: var(--system-accent-subtle);
    color: var(--system-accent);
    border: none;
    border-radius: 4px;
    font-size: 11px;
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
    transition: background var(--duration-fast) var(--ease);
  }

  .facet-pill:hover {
    background: color-mix(in srgb, var(--system-accent) 25%, transparent);
  }

  .search-input {
    background: transparent;
    border: none;
    outline: none;
    color: var(--text-primary);
    font-size: 13px;
    flex: 1;
    min-width: 0;
  }

  .search-input::placeholder {
    color: var(--text-quaternary);
  }

  .search-clear {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    color: var(--text-tertiary);
    cursor: pointer;
    border-radius: 50%;
    transition: color var(--duration-fast) var(--ease);
  }

  .search-clear:hover {
    color: var(--text-primary);
  }

  .filter-select {
    padding: var(--space-2) var(--space-3);
    background: var(--bg-tertiary);
    border: 1px solid var(--separator);
    border-radius: var(--radius-sm);
    color: var(--text-secondary);
    font-size: 12px;
    cursor: pointer;
  }

  .filter-select:focus {
    outline: none;
    border-color: var(--accent-muted);
  }

  .filter-count {
    font-size: 12px;
    color: var(--text-tertiary);
    font-weight: 500;
    white-space: nowrap;
  }

  /* ============================
     Stats Badges (inline in filter bar)
     ============================ */
  .stats-badges {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    flex-shrink: 0;
    margin-left: auto;
  }

  .stat-badge {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 11px;
    font-weight: 600;
    padding: 1px 7px;
    border-radius: 4px;
    white-space: nowrap;
  }

  .stat-enabled {
    background: color-mix(in srgb, #22c55e 12%, transparent);
    color: #22c55e;
  }

  .stat-disabled {
    background: color-mix(in srgb, var(--text-tertiary) 10%, transparent);
    color: var(--text-tertiary);
  }

  .stat-conflicts {
    background: color-mix(in srgb, #f59e0b 12%, transparent);
    color: #f59e0b;
  }

  .stat-updates {
    background: color-mix(in srgb, #3b82f6 12%, transparent);
    color: #3b82f6;
  }

  /* ============================
     Category Dropdown & Popover
     ============================ */
  .category-dropdown-wrapper {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }

  .category-dropdown-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    white-space: nowrap;
  }

  .category-dropdown-btn.has-active {
    border-color: var(--accent-muted);
    color: var(--text-primary);
  }

  .active-category-chip {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 11px;
    font-weight: 500;
    padding: 1px 7px;
    border-radius: 4px;
    cursor: pointer;
    white-space: nowrap;
    color: var(--chip-color);
    background: color-mix(in srgb, var(--chip-color) 20%, transparent);
    border: 1px solid color-mix(in srgb, var(--chip-color) 40%, transparent);
    transition: all var(--duration-fast) var(--ease);
  }

  .active-category-chip:hover {
    background: color-mix(in srgb, var(--chip-color) 30%, transparent);
  }

  .category-popover-backdrop {
    position: fixed;
    inset: 0;
    z-index: 99;
  }

  .category-popover {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    z-index: 100;
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    padding: var(--space-2);
    background: var(--bg-secondary);
    border: 1px solid var(--separator);
    border-radius: var(--radius-sm);
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.3);
    min-width: 200px;
    max-width: 400px;
  }

  .category-filter-chip {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 11px;
    font-weight: 500;
    padding: 1px 7px;
    border-radius: 4px;
    cursor: pointer;
    white-space: nowrap;
    color: var(--chip-color);
    background: color-mix(in srgb, var(--chip-color) 8%, transparent);
    border: 1px solid transparent;
    transition: all var(--duration-fast) var(--ease);
  }

  .category-filter-chip:hover {
    background: color-mix(in srgb, var(--chip-color) 15%, transparent);
  }

  .category-filter-chip.active {
    background: color-mix(in srgb, var(--chip-color) 20%, transparent);
    border-color: color-mix(in srgb, var(--chip-color) 40%, transparent);
  }

  .chip-count {
    font-size: 10px;
    opacity: 0.6;
  }

  /* ============================
     Collection & Notes Badges
     ============================ */
  .collection-badge {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    padding: 1px 6px;
    border-radius: 4px;
    background: color-mix(in srgb, var(--purple, #bf5af2) 15%, transparent);
    color: var(--purple, #bf5af2);
    font-size: 10px;
    font-weight: 600;
    max-width: 100px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .notes-icon {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    color: var(--text-quaternary);
    cursor: help;
  }

  .notes-icon:hover {
    color: var(--text-secondary);
  }

  /* ============================
     Conflict Resolution Panel
     ============================ */
  .conflict-link {
    background: none;
    border: none;
    cursor: pointer;
    text-decoration: underline;
    text-underline-offset: 2px;
    font-size: 12px;
    padding: 0;
  }

  .conflict-link:hover {
    color: #ffcc00 !important;
  }

  .conflict-panel {
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    max-height: 360px;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    margin-bottom: var(--space-3);
  }

  .conflict-panel-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--separator);
    flex-shrink: 0;
  }

  .conflict-panel-title {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: 13px;
    font-weight: 600;
    color: var(--yellow);
  }

  .conflict-list {
    overflow-y: auto;
    padding: var(--space-2);
  }

  .conflict-item {
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    margin-bottom: var(--space-1);
  }

  .conflict-item:hover {
    background: var(--surface-hover);
  }

  .conflict-path {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-tertiary);
    margin-bottom: var(--space-2);
  }

  .conflict-filepath {
    font-size: 11px;
    font-family: var(--font-mono);
    word-break: break-all;
  }

  .conflict-mods {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding-left: var(--space-5);
  }

  .conflict-mod {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
    font-size: 12px;
  }

  .conflict-winner {
    background: color-mix(in srgb, var(--green) 8%, transparent);
  }

  .conflict-mod-name {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-primary);
    font-weight: 500;
    flex: 1;
    min-width: 0;
  }

  .winner-badge {
    display: inline-flex;
    align-items: center;
    padding: 0 5px;
    border-radius: 3px;
    background: color-mix(in srgb, var(--green) 15%, transparent);
    color: var(--green);
    font-size: 9px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    flex-shrink: 0;
  }

  .conflict-mod-priority {
    color: var(--text-tertiary);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }

  .make-winner-btn {
    flex-shrink: 0;
    color: var(--accent) !important;
  }

  .make-winner-btn:hover {
    background: var(--accent-subtle) !important;
  }

  .conflict-panel-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .magic-resolve-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    background: var(--accent);
    color: #fff;
    border-radius: var(--radius-sm);
    padding: 4px 10px;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    transition: opacity var(--duration-fast) var(--ease);
  }

  .magic-resolve-btn:hover:not(:disabled) {
    opacity: 0.85;
  }

  .magic-resolve-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .resolution-banner {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    background: color-mix(in srgb, var(--green) 8%, transparent);
    border-bottom: 1px solid color-mix(in srgb, var(--green) 20%, transparent);
    color: var(--green);
    font-size: 12px;
    font-weight: 500;
    flex-shrink: 0;
  }

  .conflict-status-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 1px 6px;
    border-radius: 3px;
    font-size: 9px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    flex-shrink: 0;
  }

  .status-author {
    background: color-mix(in srgb, var(--green) 15%, transparent);
    color: var(--green);
  }

  .status-suggested {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--accent);
  }

  .status-manual {
    background: color-mix(in srgb, var(--yellow) 15%, transparent);
    color: var(--yellow);
  }

  .status-identical {
    background: color-mix(in srgb, var(--text-tertiary) 15%, transparent);
    color: var(--text-tertiary);
  }

  .conflict-resolved {
    opacity: 0.6;
  }

  .conflict-identical {
    opacity: 0.45;
  }

  .identical-banner {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    background: color-mix(in srgb, var(--text-tertiary) 8%, transparent);
    border-bottom: 1px solid color-mix(in srgb, var(--text-tertiary) 20%, transparent);
    color: var(--text-secondary);
    font-size: 12px;
    font-weight: 500;
    flex-shrink: 0;
  }

  .conflict-reason {
    font-size: 11px;
    color: var(--text-tertiary);
    padding-left: var(--space-5);
    margin-bottom: var(--space-1);
    font-style: italic;
  }

  /* ============================
     Sort Direction Button
     ============================ */
  .sort-dir-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border-radius: var(--radius-sm);
    color: var(--text-tertiary);
    transition: all var(--duration-fast) var(--ease);
    cursor: pointer;
    flex-shrink: 0;
  }

  .sort-dir-btn:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  /* ============================
     View Mode Toggle
     ============================ */
  .view-mode-toggle {
    display: flex;
    gap: 0;
    border: 1px solid var(--separator);
    border-radius: var(--radius-sm);
    overflow: hidden;
    flex-shrink: 0;
  }

  .view-mode-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 30px;
    height: 28px;
    color: var(--text-tertiary);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
    border-right: 1px solid var(--separator);
  }

  .view-mode-btn:last-child {
    border-right: none;
  }

  .view-mode-btn:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .view-mode-btn.active {
    background: var(--accent-subtle);
    color: var(--accent);
  }

  /* ============================
     Collection Group Headers
     ============================ */
  .group-header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: var(--space-2) var(--space-3);
    background: var(--bg-secondary);
    border-bottom: 1px solid var(--separator);
    cursor: pointer;
    text-align: left;
    transition: background var(--duration-fast) var(--ease);
  }

  .group-header:hover {
    background: var(--surface-hover);
  }

  .group-chevron {
    color: var(--text-tertiary);
    transition: transform var(--duration-fast) var(--ease);
    flex-shrink: 0;
  }

  .group-chevron.expanded {
    transform: rotate(90deg);
  }

  .group-name {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .group-count {
    font-size: 11px;
    color: var(--text-tertiary);
    font-weight: 500;
  }

  .disabled-separator {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: var(--space-2) var(--space-3);
    background: color-mix(in srgb, var(--bg-secondary) 80%, transparent);
    border-top: 1px solid var(--separator);
    border-bottom: 1px solid var(--separator);
    cursor: pointer;
    text-align: left;
    transition: background var(--duration-fast) var(--ease);
  }

  .disabled-separator:hover {
    background: var(--surface-hover);
  }

  .disabled-separator-label {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-tertiary);
  }

  .disabled-separator-line {
    flex: 1;
    height: 1px;
    background: var(--separator);
    margin-left: var(--space-2);
  }

  /* ============================
     Optional Mods Separator
     ============================ */
  .optional-separator {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: var(--space-1) var(--space-3) var(--space-1) calc(var(--space-3) + 24px);
    background: color-mix(in srgb, var(--bg-secondary) 60%, transparent);
    border-top: 1px solid var(--separator);
    border-bottom: 1px solid var(--separator);
    cursor: pointer;
    text-align: left;
    transition: background var(--duration-fast) var(--ease);
    border-left: none;
    border-right: none;
    font-family: inherit;
  }

  .optional-separator:hover {
    background: var(--surface-hover);
  }

  .optional-label {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-quaternary);
  }

  .optional-count {
    font-size: 10px;
    font-weight: 500;
    color: var(--text-quaternary);
  }

  .optional-line {
    flex: 1;
    height: 1px;
    background: var(--separator);
    margin-left: var(--space-2);
  }

  .row-optional {
    opacity: 0.75;
  }

  /* ============================
     Context Menu
     ============================ */
  .context-overlay {
    position: fixed;
    inset: 0;
    z-index: 199;
  }

  .context-menu {
    position: fixed;
    z-index: 200;
    min-width: 180px;
    background: var(--bg-primary);
    border: 1px solid var(--separator);
    border-radius: var(--radius-md);
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
    padding: var(--space-1) 0;
    animation: contextFadeIn 0.1s var(--ease-out);
  }

  @keyframes contextFadeIn {
    from { opacity: 0; transform: scale(0.96); }
    to { opacity: 1; transform: scale(1); }
  }

  .context-item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: var(--space-2) var(--space-3);
    font-size: 13px;
    color: var(--text-primary);
    cursor: pointer;
    text-align: left;
    transition: background var(--duration-fast) var(--ease);
  }

  .context-item:hover {
    background: var(--surface-hover);
  }

  .context-danger {
    color: var(--red);
  }

  .context-separator {
    height: 1px;
    background: var(--separator);
    margin: var(--space-1) 0;
  }

  /* ============================
     Deploy Progress
     ============================ */
  .btn-deploy {
    position: relative;
    overflow: hidden;
    min-width: 100px;
  }

  .btn-deploy.deploying {
    min-width: 160px;
  }

  .deploy-progress-track {
    position: absolute;
    inset: 0;
    background: transparent;
  }

  .deploy-progress-fill {
    height: 100%;
    background: var(--accent-subtle);
    transition: width 0.2s var(--ease);
  }

  .deploy-progress-text {
    position: relative;
    z-index: 1;
    font-size: 12px;
    white-space: nowrap;
  }

  /* ============================
     Row Focus + Hover Actions
     ============================ */
  .table-row.row-focused {
    outline: 1px solid var(--accent-muted);
    outline-offset: -1px;
  }

  .table-row .mod-action-group {
    opacity: 0;
    transition: opacity 0.15s var(--ease);
  }

  .table-row:hover .mod-action-group,
  .table-row.row-focused .mod-action-group,
  .table-row.row-selected .mod-action-group {
    opacity: 1;
  }

  /* ============================
     Category Column
     ============================ */
  .col-category {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .category-cell {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .category-icon {
    flex-shrink: 0;
    opacity: 0.85;
  }

  .category-label {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* ============================
     DL Origin Column
     ============================ */
  .col-origin {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .origin-label {
    font-size: 11px;
    font-weight: 500;
    background: none;
    border: none;
    padding: 0;
  }

  .origin-link {
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: 3px;
    transition: opacity 0.15s;
  }
  .origin-link:hover { opacity: 0.8; }

  .origin-link-icon {
    opacity: 0.5;
    flex-shrink: 0;
  }
  .origin-link:hover .origin-link-icon { opacity: 1; }

  .origin-nexus {
    color: var(--accent, #d98f40);
  }

  .origin-loverslab {
    color: #e06090;
  }

  .origin-moddb {
    color: #6cb4ee;
  }

  .origin-curseforge {
    color: #f16436;
  }

  .origin-github {
    color: #c9d1d9;
  }

  .origin-mega {
    color: #d9534f;
  }

  .origin-google_drive {
    color: #4285f4;
  }

  .origin-mediafire {
    color: #4c9aff;
  }

  .origin-direct {
    color: #8bc34a;
  }

  .origin-manual {
    color: var(--text-tertiary);
  }

  .origin-link-btn {
    background: none;
    border: none;
    cursor: pointer;
    padding: 1px 3px;
    margin-left: 2px;
    color: var(--text-tertiary);
    opacity: 0.6;
    transition: opacity 0.15s, color 0.15s;
    vertical-align: middle;
    display: inline-flex;
    align-items: center;
  }

  .origin-link-btn:hover {
    opacity: 1;
    color: var(--accent, #d98f40);
  }

  /* ============================
     Installed By Column
     ============================ */
  .col-source {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .source-label {
    font-size: 11px;
    font-weight: 500;
  }

  .source-collection {
    color: var(--blue, #60a5fa);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .source-user {
    color: var(--text-tertiary);
  }

  .text-muted {
    color: var(--text-tertiary);
    font-size: 11px;
  }

  /* ============================
     Search Highlight
     ============================ */
  .mod-name :global(mark) {
    background: rgba(255, 214, 10, 0.3);
    color: inherit;
    border-radius: 2px;
    padding: 0 1px;
  }

  /* ============================
     Empty Filter State
     ============================ */
  .empty-filter-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    padding: var(--space-8) var(--space-4);
    color: var(--text-tertiary);
    font-size: 13px;
  }

  .empty-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
    justify-content: center;
  }

  /* Bulk Select Checkbox Column */
  .col-check {
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
  }

  .col-check input[type="checkbox"] {
    width: 14px;
    height: 14px;
    accent-color: var(--system-accent);
    cursor: pointer;
  }

  /* Proactive Issue Banners */
  .issue-banner {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    font-size: 12px;
    margin-bottom: var(--space-2);
  }

  .issue-banner span {
    flex: 1;
  }

  .issue-banner-yellow {
    background: var(--yellow-subtle);
    color: var(--yellow);
    border: 1px solid rgba(255, 214, 10, 0.2);
  }

  .issue-banner-blue {
    background: var(--blue-subtle);
    color: var(--blue);
    border: 1px solid rgba(10, 132, 255, 0.2);
  }

  .banner-action {
    background: none;
    border: none;
    color: inherit;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    text-decoration: underline;
    padding: 0;
    white-space: nowrap;
  }

  .banner-dismiss {
    background: none;
    border: none;
    color: inherit;
    opacity: 0.6;
    cursor: pointer;
    padding: 2px;
    border-radius: 3px;
  }

  .banner-dismiss:hover {
    opacity: 1;
    background: var(--surface-hover);
  }

  /* Bulk Action Bar */
  .bulk-action-bar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: var(--system-accent-subtle);
    border: 1px solid rgba(10, 132, 255, 0.2);
    border-radius: var(--radius-sm);
    margin-bottom: var(--space-2);
  }

  .bulk-count {
    font-size: 12px;
    font-weight: 600;
    color: var(--system-accent);
    margin-right: var(--space-2);
  }

  .modlist-prompt-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 200;
    backdrop-filter: var(--glass-blur-light);
    animation: fadeIn 0.15s ease-out;
  }

  .modlist-prompt-card {
    background: var(--bg-grouped);
    border: 1px solid var(--separator-opaque);
    border-radius: var(--radius-lg, 12px);
    padding: 24px;
    max-width: 400px;
    width: 90%;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
  }

  .modlist-prompt-title {
    font-size: 16px;
    font-weight: 600;
    margin: 0 0 8px;
  }

  .modlist-prompt-desc {
    font-size: 13px;
    color: var(--text-secondary);
    margin: 0 0 16px;
    line-height: 1.5;
  }

  .modlist-prompt-input {
    width: 100%;
    padding: 8px 12px;
    font-size: 14px;
    background: var(--surface);
    border: 1px solid var(--separator-opaque);
    border-radius: var(--radius);
    color: var(--text-primary);
    outline: none;
    margin-bottom: 16px;
  }

  .modlist-prompt-input:focus {
    border-color: var(--accent);
  }

  .modlist-prompt-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

</style>
