<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import {
    getInstalledMods,
    installMod,
    uninstallMod,
    toggleMod,
    launchGame,
    checkSkse,
    getSkseDownloadUrl,
    installSkseFromArchive,
    setSksePreference,
    checkSkyrimVersion,
    downgradeSkyrim,
    reorderMods,
    getConflicts,
    checkModUpdates,
    fixSkyrimDisplay,
    onInstallProgress,
    redeployAllMods,
    purgeDeployment,
    getDeploymentHealth,
    setModNotes,
    setModTags,
    setModPriority,
    analyzeConflicts,
    resolveAllConflicts,
  } from "$lib/api";
  import type { InstallProgressEvent, DeploymentHealth, ConflictSuggestion, ResolutionResult } from "$lib/types";
  import {
    selectedGame,
    installedMods,
    games,
    currentPage,
    showError,
    showSuccess,
    skseStatus,
  } from "$lib/stores";
  import type { InstalledMod, DetectedGame, SkseStatus, DowngradeStatus, FileConflict, ModUpdateInfo } from "$lib/types";
  import GameIcon from "$lib/components/GameIcon.svelte";
  import DiskBudgetPanel from "$lib/components/DiskBudgetPanel.svelte";
  import PreflightPanel from "$lib/components/PreflightPanel.svelte";
  import DependencyPanel from "$lib/components/DependencyPanel.svelte";
  import SessionHistoryPanel from "$lib/components/SessionHistoryPanel.svelte";

  let installing = $state(false);
  let installStep = $state("");
  let installDetail = $state("");
  let loadingMods = $state(false);
  let confirmUninstall = $state<number | null>(null);
  let togglingMod = $state<number | null>(null);
  let launching = $state(false);
  let skse = $state<SkseStatus | null>(null);
  let showSksePrompt = $state(false);
  let installingSkse = $state(false);
  let showSkseMenu = $state(false);
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
  let filterCollection = $state<string | null>(null);
  let sortBy = $state<"priority" | "name" | "date" | "version" | "files">("priority");
  let sortDir = $state<"asc" | "desc">("asc");

  // Detail panel
  let detailMod = $state<InstalledMod | null>(null);

  // Notes/tags editing
  let editingNotesId = $state<number | null>(null);
  let editingNotesValue = $state("");

  // Conflict panel state
  let showConflictPanel = $state(false);
  let makingWinner = $state<number | null>(null);

  // Mod overflow menu state
  let overflowMenuModId = $state<number | null>(null);
  let suggestions = $state<ConflictSuggestion[]>([]);
  let analyzingConflicts = $state(false);
  let resolvingAll = $state(false);
  let resolutionResult = $state<ResolutionResult | null>(null);

  // Selected mod for dependency panel
  let selectedModId = $state<number | undefined>(undefined);

  // Collapsible panels
  let showPreflightPanel = $state(false);
  let showSessionPanel = $state(false);
  let showDependencyPanel = $state(false);

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
    return () => document.removeEventListener("click", closeOverflow);
  });

  onDestroy(() => { if (installUnlisten) { installUnlisten(); installUnlisten = null; } });

  // Sorted mods
  let sortedMods = $derived((() => {
    const mods = [...$installedMods];
    const dir = sortDir === "asc" ? 1 : -1;
    mods.sort((a, b) => {
      switch (sortBy) {
        case "name": return dir * a.name.localeCompare(b.name);
        case "date": return dir * (new Date(a.installed_at).getTime() - new Date(b.installed_at).getTime());
        case "version": return dir * (a.version || "").localeCompare(b.version || "");
        case "files": return dir * (a.installed_files.length - b.installed_files.length);
        default: return dir * (a.install_priority - b.install_priority);
      }
    });
    return mods;
  })());

  // Filtered mods based on search and filters
  let filteredMods = $derived((() => {
    let mods = sortedMods;
    // Text search (name, tags, notes, collection)
    if (searchQuery.trim()) {
      const q = searchQuery.toLowerCase();
      mods = mods.filter(m =>
        m.name.toLowerCase().includes(q) ||
        m.user_tags.some(t => t.toLowerCase().includes(q)) ||
        (m.user_notes && m.user_notes.toLowerCase().includes(q)) ||
        (m.collection_name && m.collection_name.toLowerCase().includes(q))
      );
    }
    // Status filter
    if (filterStatus === "enabled") {
      mods = mods.filter(m => m.enabled);
    } else if (filterStatus === "disabled") {
      mods = mods.filter(m => !m.enabled);
    } else if (filterStatus === "conflicts") {
      mods = mods.filter(m => conflictModIds.has(m.id));
    } else if (filterStatus === "has-updates") {
      mods = mods.filter(m => updateMap.has(m.id));
    }
    // Collection filter
    if (filterCollection !== null) {
      if (filterCollection === "__standalone__") {
        mods = mods.filter(m => !m.collection_name);
      } else {
        mods = mods.filter(m => m.collection_name === filterCollection);
      }
    }
    return mods;
  })());

  // Unique collection names for filter dropdown
  let collectionNames = $derived((() => {
    const names = new Set<string>();
    for (const m of $installedMods) {
      if (m.collection_name) names.add(m.collection_name);
    }
    return [...names].sort();
  })());

  const activeGame = $derived(pickedGame ?? $selectedGame);

  // Track the current load to avoid stale race conditions
  let loadGeneration = 0;

  $effect(() => {
    if (activeGame) {
      loadMods(activeGame);
    }
  });

  async function loadMods(game: DetectedGame) {
    const thisLoad = ++loadGeneration;
    loadingMods = true;
    try {
      const mods = await getInstalledMods(game.game_id, game.bottle_name);
      // Only update state if this is still the latest load request
      if (thisLoad !== loadGeneration) return;
      installedMods.set(mods);
      // Also load conflicts
      try {
        conflicts = await getConflicts(game.game_id, game.bottle_name);
      } catch {
        conflicts = [];
      }
    } catch (e: unknown) {
      if (thisLoad !== loadGeneration) return;
      showError(`Failed to load mods: ${e}`);
    } finally {
      if (thisLoad === loadGeneration) {
        loadingMods = false;
      }
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
        filePath as string,
        game.game_id,
        game.bottle_name
      );
      showSuccess(`Installed "${(mod as InstalledMod).name}" successfully`);
      await loadMods(game);
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
    const game = pickedGame ?? $selectedGame;
    if (!game) return;

    try {
      const removed = await uninstallMod(modId, game.game_id, game.bottle_name);
      showSuccess(`Uninstalled — ${(removed as string[]).length} files removed`);
      confirmUninstall = null;
      await loadMods(game);
    } catch (e: unknown) {
      showError(`Uninstall failed: ${e}`);
    }
  }

  async function handleToggle(mod: InstalledMod) {
    const game = pickedGame ?? $selectedGame;
    if (!game) return;
    togglingMod = mod.id;
    try {
      await toggleMod(mod.id, game.game_id, game.bottle_name, !mod.enabled);
      await loadMods(game);
    } catch (e: unknown) {
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
    launching = true;
    try {
      const useSkse = !!(skse?.installed && skse?.use_skse && game.game_id === "skyrimse");
      const result = await launchGame(game.game_id, game.bottle_name, useSkse);
      if (result.success) {
        showSuccess(`Launched ${game.display_name}${useSkse ? " via SKSE" : ""}`);
      }
    } catch (e: unknown) {
      showError(`Failed to launch: ${e}`);
    } finally {
      launching = false;
    }
  }

  async function handleFixDisplay() {
    const game = pickedGame ?? $selectedGame;
    if (!game) return;
    fixingDisplay = true;
    try {
      const result = await fixSkyrimDisplay(game.bottle_name);
      if (result.fixed) {
        showSuccess(`Display fixed: ${result.applied.width}x${result.applied.height} borderless fullscreen (was ${result.previous.width}x${result.previous.height})`);
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
      showSuccess(`Installed "${(mod as InstalledMod).name}" successfully`);
      await loadMods(game);
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

  // --- Deploy / Purge / Health ---
  async function handleDeploy() {
    const game = pickedGame ?? $selectedGame;
    if (!game) return;
    deploying = true;
    try {
      const result = await redeployAllMods(game.game_id, game.bottle_name);
      showSuccess(`Deployed ${result.deployed_count} files${result.fallback_used ? " (copy fallback used)" : ""}`);
      await refreshHealth(game);
    } catch (e: unknown) {
      showError(`Deploy failed: ${e}`);
    } finally {
      deploying = false;
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
      deployHealth = await getDeploymentHealth(game.game_id, game.bottle_name);
    } catch {
      deployHealth = null;
    }
  }

  // Load health on game change
  $effect(() => {
    if (activeGame) {
      refreshHealth(activeGame);
    }
  });

  // --- Notes ---
  async function handleSaveNotes(modId: number) {
    try {
      await setModNotes(modId, editingNotesValue || null);
      editingNotesId = null;
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
    makingWinner = modId;
    try {
      // Find current winner's priority and set this mod 1 higher
      const winner = conflict.mods.find(m => m.mod_id === conflict.winner_mod_id);
      const newPriority = winner ? winner.priority + 1 : 999;
      await setModPriority(modId, newPriority);
      await redeployAllMods(game.game_id, game.bottle_name);
      await loadMods(game);
      await refreshHealth(game);
    } catch (e: unknown) {
      showError(`Failed to set winner: ${e}`);
    } finally {
      makingWinner = null;
    }
  }

  async function handleAnalyzeConflicts() {
    const game = pickedGame ?? $selectedGame;
    if (!game) return;
    analyzingConflicts = true;
    resolutionResult = null;
    try {
      suggestions = await analyzeConflicts(game.game_id, game.bottle_name);
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
    try {
      resolutionResult = await resolveAllConflicts(game.game_id, game.bottle_name);
      await loadMods(game);
      await refreshHealth(game);
      // Re-analyze to show updated state
      suggestions = await analyzeConflicts(game.game_id, game.bottle_name);
      showSuccess(`Resolved ${resolutionResult.author_resolved + resolutionResult.auto_suggested} conflicts automatically`);
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
  const enabledCount = $derived($installedMods.filter((m) => m.enabled).length);
</script>

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
          {/if}
        </button>
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
            title="Fix zoomed-in or improperly scaled display in CrossOver"
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
          class="btn btn-secondary"
          onclick={handleDeploy}
          disabled={deploying || purging}
          title="Deploy all enabled mods to the game directory"
        >
          {#if deploying}
            <span class="spinner spinner-sm"></span>
            Deploying...
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
      <div class="play-button-group">
        <button class="btn btn-play" onclick={handlePlay} disabled={launching}>
          {#if launching}
            <span class="spinner spinner-play"></span>
            Launching...
          {:else}
            <svg width="14" height="14" viewBox="0 0 14 14" fill="currentColor">
              <path d="M3 1.5v11l9-5.5L3 1.5z" />
            </svg>
            Play{#if skse?.installed && skse?.use_skse} (SKSE){/if}
          {/if}
        </button>
        {#if activeGame?.game_id === "skyrimse" && skse?.installed}
          <button
            class="btn btn-play-dropdown"
            onclick={() => showSkseMenu = !showSkseMenu}
            aria-label="SKSE options"
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
            <div class="dropdown-divider"></div>
            <div class="dropdown-info">
              SKSE {skse?.version ?? ""}
            </div>
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
          <button class="btn btn-secondary btn-sm" onclick={handleOpenSkseDownload}>Download</button>
          <button class="btn btn-primary btn-sm" onclick={handleInstallSkse} disabled={installingSkse}>
            {installingSkse ? "Installing..." : "Install from Archive"}
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
                <button class="btn btn-ghost btn-sm" onclick={() => { showConflictPanel = false; suggestions = []; resolutionResult = null; }}>
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
                  {resolutionResult.manual_needed} need review
                  {#if resolutionResult.priorities_changed > 0}
                    &mdash; {resolutionResult.priorities_changed} priorities adjusted
                  {/if}
                </span>
              </div>
            {/if}

            <!-- Smart suggestions view -->
            {#if suggestions.length > 0}
              <div class="conflict-list">
                {#each suggestions as s (s.relative_path)}
                  <div class="conflict-item" class:conflict-resolved={s.status === "AuthorResolved"} class:conflict-suggested={s.status === "Suggested"}>
                    <div class="conflict-path">
                      <span class="conflict-status-badge" class:status-author={s.status === "AuthorResolved"} class:status-suggested={s.status === "Suggested"} class:status-manual={s.status === "Manual"}>
                        {#if s.status === "AuthorResolved"}OK
                        {:else if s.status === "Suggested"}Auto
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
                              <span class="winner-badge">{s.status === "AuthorResolved" ? "Author" : s.status === "Suggested" ? "Suggested" : "Winner"}</span>
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

        <!-- Search & Filter Bar -->
        {#if $installedMods.length > 0}
          <div class="filter-bar">
        <div class="search-box">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="11" cy="11" r="8" />
            <line x1="21" y1="21" x2="16.65" y2="16.65" />
          </svg>
          <input
            type="text"
            placeholder="Search mods..."
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
        {#if collectionNames.length > 0}
          <select class="filter-select" bind:value={filterCollection}>
            <option value={null}>All Sources</option>
            <option value="__standalone__">Standalone</option>
            {#each collectionNames as name}
              <option value={name}>{name}</option>
            {/each}
          </select>
        {/if}
        <select class="filter-select" bind:value={sortBy}>
          <option value="priority">Sort: Priority</option>
          <option value="name">Sort: Name</option>
          <option value="date">Sort: Date</option>
          <option value="files">Sort: File Count</option>
        </select>
        <button class="sort-dir-btn" onclick={() => sortDir = sortDir === "asc" ? "desc" : "asc"} title={sortDir === "asc" ? "Ascending" : "Descending"}>
          <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" style="transform: {sortDir === 'desc' ? 'rotate(180deg)' : 'none'}">
            <path d="M6 2v8M3 7l3 3 3-3" />
          </svg>
        </button>
        {#if searchQuery || filterStatus !== "all" || filterCollection !== null}
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
        <button class="btn btn-primary" onclick={handleInstall}>
          <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
            <line x1="7" y1="2" x2="7" y2="12" />
            <line x1="2" y1="7" x2="12" y2="7" />
          </svg>
          Install Your First Mod
        </button>
      </div>
    {:else}
      <div class="mod-layout" class:has-detail={detailMod !== null}>
      <div class="mod-table-container" class:reordering-active={reordering}>
        <div class="mod-table">
          <!-- Sticky Header -->
          <div class="table-header">
            <span class="col-grip"></span>
            <span class="col-toggle"></span>
            <span class="col-name">Name</span>
            <span class="col-version">Version</span>
            <span class="col-files">Files</span>
            <span class="col-date">Installed</span>
            <span class="col-actions"></span>
          </div>

          <!-- Mod Rows -->
          <div class="table-body">
            {#each filteredMods as mod, i (mod.id)}
              <div
                class="table-row"
                class:row-disabled={!mod.enabled}
                class:row-selected={selectedModId === mod.id}
                class:row-dragging={dragRowIndex === i}
                class:row-drag-over={dragOverIndex === i && dragRowIndex !== null && dragRowIndex !== i}
                class:row-drag-above={dragOverIndex === i && dragRowIndex !== null && dragRowIndex > i}
                class:row-drag-below={dragOverIndex === i && dragRowIndex !== null && dragRowIndex < i}
                draggable="true"
                onclick={() => { selectedModId = selectedModId === mod.id ? undefined : mod.id; detailMod = detailMod?.id === mod.id ? null : mod; }}
                ondragstart={(e) => handleRowDragStart(e, i)}
                ondragover={(e) => handleRowDragOver(e, i)}
                ondragend={handleRowDragEnd}
                ondrop={(e) => handleRowDrop(e, i)}
              >
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
                  >
                    <span class="toggle-track">
                      <span class="toggle-thumb"></span>
                    </span>
                  </button>
                </span>

                <!-- Name -->
                <span class="col-name">
                  <span class="mod-name">{mod.name}</span>
                  {#if conflictModIds.has(mod.id)}
                    <span class="conflict-icon" title={getConflictTooltip(mod.id)}>
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
                        <line x1="12" y1="9" x2="12" y2="13" />
                        <line x1="12" y1="17" x2="12.01" y2="17" />
                      </svg>
                    </span>
                  {/if}
                  {#if mod.nexus_mod_id}
                    <span class="nexus-badge">Nexus</span>
                  {/if}
                  {#if mod.collection_name}
                    <span class="collection-badge" title={mod.collection_name}>{mod.collection_name}</span>
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
                              <button class="overflow-item" onclick={(e) => { e.stopPropagation(); overflowMenuModId = null; openUrl(`https://www.nexusmods.com/skyrimspecialedition/mods/${mod.nexus_mod_id}`); }}>
                                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                  <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" /><polyline points="15 3 21 3 21 9" /><line x1="10" y1="14" x2="21" y2="3" />
                                </svg>
                                Open on Nexus
                              </button>
                            {/if}
                            {#if mod.staging_path}
                              <button class="overflow-item" onclick={(e) => { e.stopPropagation(); overflowMenuModId = null; /* TODO: reconfigure FOMOD */ }}>
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
          </div>
        </div>
      </div>

      <!-- Detail Sidebar -->
      {#if detailMod}
        <div class="mod-detail-panel">
          <div class="detail-header">
            <h3 class="detail-name">{detailMod.name}</h3>
            <button class="detail-close" onclick={() => { detailMod = null; selectedModId = undefined; }}>
              <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
                <line x1="3" y1="3" x2="11" y2="11" /><line x1="11" y1="3" x2="3" y2="11" />
              </svg>
            </button>
          </div>

          <div class="detail-body">
            <div class="detail-meta">
              <div class="detail-row">
                <span class="detail-label">Version</span>
                <span class="detail-value">{detailMod.version || "\u2014"}</span>
              </div>
              <div class="detail-row">
                <span class="detail-label">Installed</span>
                <span class="detail-value">{formatDate(detailMod.installed_at)}</span>
              </div>
              <div class="detail-row">
                <span class="detail-label">Files</span>
                <span class="detail-value">{detailMod.installed_files.length}</span>
              </div>
              <div class="detail-row">
                <span class="detail-label">Priority</span>
                <span class="detail-value">{detailMod.install_priority}</span>
              </div>
              {#if detailMod.archive_name}
                <div class="detail-row">
                  <span class="detail-label">Archive</span>
                  <span class="detail-value detail-archive">{detailMod.archive_name}</span>
                </div>
              {/if}
              {#if detailMod.collection_name}
                <div class="detail-row">
                  <span class="detail-label">Collection</span>
                  <span class="detail-value collection-badge">{detailMod.collection_name}</span>
                </div>
              {/if}
              {#if detailMod.nexus_mod_id}
                <div class="detail-row">
                  <span class="detail-label">Nexus</span>
                  <a class="detail-value detail-link" href="https://www.nexusmods.com/{activeGame?.nexus_slug}/mods/{detailMod.nexus_mod_id}" target="_blank" rel="noopener noreferrer">
                    Mod #{detailMod.nexus_mod_id}
                  </a>
                </div>
              {/if}
            </div>

            {#if updateMap.has(detailMod.id)}
              {@const update = updateMap.get(detailMod.id)!}
              <div class="detail-update-banner">
                <span class="detail-update-text">Update: v{update.current_version} &rarr; v{update.latest_version}</span>
              </div>
            {/if}

            {#if conflictModIds.has(detailMod.id)}
              <div class="detail-section">
                <h4 class="detail-section-title">Conflicts</h4>
                <div class="detail-conflict-list">
                  {#each [...(conflictDetails.get(detailMod.id) ?? [])] as conflictName}
                    <span class="detail-conflict-badge">{conflictName}</span>
                  {/each}
                </div>
              </div>
            {/if}

            <!-- Tags -->
            <div class="detail-section">
              <h4 class="detail-section-title">Tags</h4>
              <div class="detail-tags">
                {#each detailMod.user_tags as tag}
                  <span class="detail-tag">{tag}</span>
                {/each}
                {#if detailMod.user_tags.length === 0}
                  <span class="detail-empty">No tags</span>
                {/if}
              </div>
            </div>

            <!-- Notes -->
            <div class="detail-section">
              <h4 class="detail-section-title">Notes</h4>
              {#if editingNotesId === detailMod.id}
                <textarea class="detail-notes-input" bind:value={editingNotesValue} rows="3" placeholder="Add notes about this mod..."></textarea>
                <div class="detail-notes-actions">
                  <button class="btn btn-primary btn-sm" onclick={() => handleSaveNotes(detailMod!.id)}>Save</button>
                  <button class="btn btn-ghost btn-sm" onclick={() => editingNotesId = null}>Cancel</button>
                </div>
              {:else}
                <button class="detail-notes-display" onclick={() => { editingNotesId = detailMod!.id; editingNotesValue = detailMod!.user_notes ?? ""; }}>
                  {detailMod.user_notes || "Click to add notes..."}
                </button>
              {/if}
            </div>

            <!-- Actions -->
            <div class="detail-actions">
              <button class="btn btn-secondary btn-sm" onclick={() => handleToggle(detailMod!)}>
                {detailMod.enabled ? "Disable" : "Enable"}
              </button>
              {#if confirmUninstall === detailMod.id}
                <button class="btn btn-danger btn-sm" onclick={() => handleUninstall(detailMod!.id)}>Confirm</button>
                <button class="btn btn-ghost btn-sm" onclick={() => confirmUninstall = null}>Cancel</button>
              {:else}
                <button class="btn btn-ghost-danger btn-sm" onclick={() => confirmUninstall = detailMod!.id}>Uninstall</button>
              {/if}
            </div>
          </div>
        </div>
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
                <span class="sidebar-stat-value" class:health-warning={deployHealth.conflict_count > 0}>{deployHealth.conflict_count}</span>
                <span class="sidebar-stat-label">Conflicts</span>
              </div>
            </div>
            {#if deployHealth.deploy_method && deployHealth.deploy_method !== "none"}
              <span class="sidebar-detail">{deployHealth.deploy_method === "hardlink" ? "Hardlinks (0 extra disk)" : "File copies"}</span>
            {/if}
            {#if deployHealth.conflict_count > 0}
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
</div>

<style>
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
    box-shadow: var(--glass-edge-shadow);
    text-align: left;
    transition:
      background var(--duration) var(--ease),
      border-color var(--duration) var(--ease),
      box-shadow var(--duration) var(--ease);
  }

  .game-card:hover {
    background: var(--surface-hover);
    border-color: var(--accent-muted);
    box-shadow: var(--glass-edge-shadow), var(--shadow-sm);
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
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    box-shadow: var(--glass-edge-shadow);
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

  .game-banner-actions {
    display: flex;
    align-items: center;
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
    border: 2px solid rgba(255, 255, 255, 0.3);
    border-top-color: #fff;
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
    background: var(--surface);
    border: 1px dashed var(--separator-opaque);
    border-radius: var(--radius-lg);
    box-shadow: var(--glass-edge-shadow);
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
    overflow: hidden;
    border-radius: var(--radius-lg);
    background: var(--bg-primary);
    box-shadow: var(--glass-edge-shadow);
    min-height: 200px;
  }

  .reordering-active {
    opacity: 0.7;
    pointer-events: none;
  }

  .mod-table {
    display: flex;
    flex-direction: column;
    height: 100%;
  }

  .table-header {
    display: grid;
    grid-template-columns: 28px 48px minmax(0, 1fr) 72px 48px 90px 64px;
    padding: var(--space-2) var(--space-3);
    background: var(--bg-secondary);
    border-bottom: 1px solid var(--separator);
    position: sticky;
    top: 0;
    z-index: 2;
  }

  .table-header span {
    font-size: 11px;
    font-weight: 500;
    color: var(--text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .table-body {
    flex: 1;
    overflow-y: auto;
  }

  .table-row {
    display: grid;
    grid-template-columns: 28px 48px minmax(0, 1fr) 72px 48px 90px 64px;
    padding: var(--space-2) var(--space-3);
    align-items: center;
    font-size: 13px;
    transition:
      background var(--duration-fast) var(--ease),
      opacity var(--duration-fast) var(--ease),
      box-shadow var(--duration-fast) var(--ease);
  }

  /* Narrow window: hide Files and Date columns, shrink right sidebar */
  @media (max-width: 1100px) {
    .table-header,
    .table-row {
      grid-template-columns: 28px 48px minmax(0, 1fr) 64px 0px 0px 60px;
    }
    .col-files,
    .col-date {
      display: none;
    }
  }

  .table-row:nth-child(even) {
    background: rgba(255, 255, 255, 0.025);
  }

  :global([data-theme="light"]) .table-row:nth-child(even) {
    background: rgba(0, 0, 0, 0.025);
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
  }

  /* --- Nexus link --- */

  .nexus-link {
    text-decoration: none !important;
  }

  /* --- Play Button --- */

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
    backdrop-filter: blur(8px);
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

  /* ============================
     Search & Filter Bar
     ============================ */
  .filter-bar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-shrink: 0;
  }

  .search-box {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-sm);
    flex: 1;
    max-width: 280px;
    color: var(--text-tertiary);
    transition: border-color var(--duration-fast) var(--ease);
  }

  .search-box:focus-within {
    border-color: var(--accent-muted);
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
    background: var(--surface);
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

  .conflict-resolved {
    opacity: 0.6;
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
     Detail Panel
     ============================ */
  .mod-detail-panel {
    width: 280px;
    min-width: 240px;
    flex-shrink: 0;
    border-radius: var(--radius-lg);
    background: var(--bg-primary);
    box-shadow: var(--glass-edge-shadow);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    animation: detailSlideIn 0.15s var(--ease-out);
  }

  @keyframes detailSlideIn {
    from { opacity: 0; transform: translateX(8px); }
    to { opacity: 1; transform: translateX(0); }
  }

  .detail-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-4);
    border-bottom: 1px solid var(--separator);
  }

  .detail-name {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-primary);
    line-height: 1.35;
    word-break: break-word;
  }

  .detail-close {
    flex-shrink: 0;
    padding: 4px;
    border-radius: var(--radius-sm);
    color: var(--text-tertiary);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
  }

  .detail-close:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .detail-body {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .detail-meta {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .detail-row {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    padding: 3px 0;
  }

  .detail-label {
    font-size: 12px;
    color: var(--text-tertiary);
    font-weight: 500;
  }

  .detail-value {
    font-size: 12px;
    color: var(--text-primary);
    font-weight: 500;
    text-align: right;
    max-width: 60%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .detail-archive {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-secondary);
  }

  .detail-link {
    color: var(--accent);
    text-decoration: none;
  }

  .detail-link:hover {
    text-decoration: underline;
  }

  .detail-update-banner {
    padding: var(--space-2) var(--space-3);
    background: rgba(48, 209, 88, 0.1);
    border: 1px solid rgba(48, 209, 88, 0.2);
    border-radius: var(--radius-sm);
  }

  .detail-update-text {
    font-size: 12px;
    font-weight: 600;
    color: var(--green);
  }

  .detail-section {
    border-top: 1px solid var(--separator);
    padding-top: var(--space-3);
  }

  .detail-section-title {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-tertiary);
    margin-bottom: var(--space-2);
  }

  .detail-conflict-list {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }

  .detail-conflict-badge {
    font-size: 11px;
    padding: 2px 6px;
    background: rgba(255, 69, 58, 0.1);
    color: var(--red);
    border-radius: var(--radius-sm);
    font-weight: 500;
  }

  .detail-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }

  .detail-tag {
    font-size: 11px;
    padding: 2px 8px;
    background: var(--accent-subtle);
    color: var(--accent);
    border-radius: var(--radius-sm);
    font-weight: 500;
  }

  .detail-empty {
    font-size: 12px;
    color: var(--text-quaternary);
  }

  .detail-notes-input {
    width: 100%;
    padding: var(--space-2);
    background: var(--bg-base);
    border: 1px solid var(--separator);
    border-radius: var(--radius-sm);
    color: var(--text-primary);
    font-size: 12px;
    font-family: inherit;
    resize: vertical;
  }

  .detail-notes-input:focus {
    outline: none;
    border-color: var(--accent);
  }

  .detail-notes-actions {
    display: flex;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }

  .detail-notes-display {
    width: 100%;
    text-align: left;
    padding: var(--space-2);
    font-size: 12px;
    color: var(--text-secondary);
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease);
    white-space: pre-wrap;
    word-break: break-word;
    line-height: 1.5;
  }

  .detail-notes-display:hover {
    background: var(--surface-hover);
  }

  .detail-actions {
    display: flex;
    gap: var(--space-2);
    border-top: 1px solid var(--separator);
    padding-top: var(--space-3);
  }
</style>
