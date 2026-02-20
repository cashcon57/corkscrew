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
  } from "$lib/api";
  import type { InstallProgressEvent } from "$lib/types";
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

  onDestroy(() => { if (installUnlisten) { installUnlisten(); installUnlisten = null; } });

  // Sorted mods by install_priority ascending
  let sortedMods = $derived(
    [...$installedMods].sort((a, b) => a.install_priority - b.install_priority)
  );

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
    } catch (e: any) {
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
    } catch (e: any) {
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
    } catch (e: any) {
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
    } catch (e: any) {
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
    } catch (e: any) {
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
    } catch (e: any) {
      showError(`Display fix failed: ${e}`);
    } finally {
      fixingDisplay = false;
    }
  }

  async function handleOpenSkseDownload() {
    try {
      const url = await getSkseDownloadUrl();
      await openUrl(url);
    } catch (e: any) {
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
    } catch (e: any) {
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
    } catch (e: any) {
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
    } catch (e: any) {
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
    } catch (e: any) {
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
    } catch (e: any) {
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
    } catch (e: any) {
      showError(`Failed to check for updates: ${e}`);
    } finally {
      checkingUpdates = false;
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
        <GameIcon gameId={activeGame.game_id} size={56} />
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

    <!-- SKSE Prompt Banner -->
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
            Skyrim Script Extender (SKSE) is required by most Skyrim mods.
            Download it from the official site, then install from the archive.
          </p>
        </div>
        <div class="skse-banner-actions">
          <button class="btn btn-secondary btn-sm" onclick={handleOpenSkseDownload}>
            Download SKSE
          </button>
          <button class="btn btn-primary btn-sm" onclick={handleInstallSkse} disabled={installingSkse}>
            {installingSkse ? "Installing..." : "Install from Archive"}
          </button>
          <button class="btn btn-ghost btn-sm" onclick={dismissSksePrompt}>
            Dismiss
          </button>
        </div>
      </div>
    {/if}

    <!-- Skyrim Downgrade Banner -->
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
            {#if downgradeStatus.current_version !== "1.5.97"}
              — Downgrade Available
            {/if}
          </p>
          <p class="skse-banner-text">
            Most mods target v1.5.97. Downgrade creates a protected copy of your game files that won't be overwritten by Steam updates.
          </p>
        </div>
        <div class="skse-banner-actions">
          <button class="btn btn-primary btn-sm" onclick={handleDowngrade} disabled={downgrading}>
            {downgrading ? "Downgrading..." : "Downgrade to v1.5.97"}
          </button>
          <button class="btn btn-ghost btn-sm" onclick={dismissDowngradeBanner}>
            Dismiss
          </button>
        </div>
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
            {#each sortedMods as mod, i (mod.id)}
              <div
                class="table-row"
                class:row-disabled={!mod.enabled}
                class:row-dragging={dragRowIndex === i}
                class:row-drag-over={dragOverIndex === i && dragRowIndex !== null && dragRowIndex !== i}
                class:row-drag-above={dragOverIndex === i && dragRowIndex !== null && dragRowIndex > i}
                class:row-drag-below={dragOverIndex === i && dragRowIndex !== null && dragRowIndex < i}
                draggable="true"
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
                      <button
                        class="btn btn-danger btn-sm"
                        onclick={() => handleUninstall(mod.id)}
                      >
                        Confirm
                      </button>
                      <button
                        class="btn btn-ghost btn-sm"
                        onclick={() => (confirmUninstall = null)}
                      >
                        Cancel
                      </button>
                    </div>
                  {:else}
                    <button
                      class="btn btn-ghost-danger btn-sm"
                      onclick={() => (confirmUninstall = mod.id)}
                    >
                      Uninstall
                    </button>
                  {/if}
                </span>
              </div>
            {/each}
          </div>
        </div>
      </div>
    {/if}
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
    padding: var(--space-6);
    gap: var(--space-6);
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
    gap: var(--space-4);
    padding: var(--space-4) var(--space-5);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-lg);
    box-shadow: var(--glass-edge-shadow);
    flex-shrink: 0;
  }

  .game-banner-icon {
    flex-shrink: 0;
    color: var(--text-primary);
    display: flex;
    align-items: center;
    justify-content: center;
    width: 56px;
    height: 56px;
  }

  .game-banner-info {
    flex: 1;
    min-width: 0;
  }

  .game-banner-title {
    font-size: 22px;
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
     Mod Table
     ============================ */
  .mod-table-container {
    flex: 1;
    overflow: hidden;
    border-radius: var(--radius-lg);
    background: var(--bg-primary);
    box-shadow: var(--glass-edge-shadow);
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
    grid-template-columns: 32px 52px 1fr 100px 64px 110px 120px;
    padding: var(--space-3) var(--space-4);
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
  }

  .table-body {
    flex: 1;
    overflow-y: auto;
  }

  .table-row {
    display: grid;
    grid-template-columns: 32px 52px 1fr 100px 64px 110px 120px;
    padding: var(--space-3) var(--space-4);
    align-items: center;
    font-size: 13px;
    transition:
      background var(--duration-fast) var(--ease),
      opacity var(--duration-fast) var(--ease),
      box-shadow var(--duration-fast) var(--ease);
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

  .mods-page {
    position: relative;
  }

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
</style>
