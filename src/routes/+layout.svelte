<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import "../app.css";
  import { currentPage, errorMessage, successMessage, selectedGame, selectedBottle, showError, showSuccess, appVersion, collectionInstallStatus, updateReady as updateReadyStore, updateVersion as updateVersionStore, updateChecking as updateCheckingStore, updateError as updateErrorStore, setUpdateCheckFn } from "$lib/stores";
  import { initTheme } from "$lib/theme";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { onOpenUrl } from "@tauri-apps/plugin-deep-link";
  import { getVersion } from "@tauri-apps/api/app";
  import { check } from "@tauri-apps/plugin-updater";
  import { relaunch } from "@tauri-apps/plugin-process";
  import { downloadFromNexus, getAllGames, getDownloadQueue, retryDownload, cancelDownload, clearFinishedDownloads, onDownloadQueueUpdate } from "$lib/api";
  import { get } from "svelte/store";
  import type { DetectedGame, QueueItem } from "$lib/types";
  import GameIcon from "$lib/components/GameIcon.svelte";

  const navItems = [
    { id: "dashboard", label: "Dashboard" },
    { id: "mods", label: "Mods" },
    { id: "plugins", label: "Load Order" },
    { id: "discover", label: "Discover" },
    { id: "profiles", label: "Profiles" },
    { id: "logs", label: "Crash Logs" },
    { id: "settings", label: "Settings" },
  ];

  let detectedGames = $state<DetectedGame[]>([]);
  let gameDropdownOpen = $state(false);

  // Download queue state
  let queueItems = $state<QueueItem[]>([]);
  let showQueue = $state(false);
  let queueUnlisten: (() => void) | null = null;

  let activeDownloads = $derived(queueItems.filter(i => i.status === "downloading" || i.status === "pending").length);
  let failedDownloads = $derived(queueItems.filter(i => i.status === "failed").length);

  // Auto-update state
  let updateAvailable = $state(false);
  let updateVersion = $state("");
  let updateDownloading = $state(false);
  let updateProgress = $state(0);
  let updateReady = $state(false);

  // Queue popover positioning (fixed to escape sidebar overflow:hidden)
  let queueBtnEl = $state<HTMLElement | null>(null);
  let popoverStyle = $state('');
  $effect(() => {
    if (showQueue && queueBtnEl) {
      const rect = queueBtnEl.getBoundingClientRect();
      popoverStyle = `bottom: ${window.innerHeight - rect.top + 8}px; left: ${rect.left}px;`;
    }
  });

  onMount(() => {
    initTheme();
    loadDetectedGames();
    getVersion().then(v => appVersion.set(v)).catch(() => {});

    // Check for app updates on startup
    checkForUpdates();

    // Subscribe to download queue updates
    getDownloadQueue().then(items => queueItems = items).catch(() => {});
    onDownloadQueueUpdate((items) => { queueItems = items; }).then(fn => queueUnlisten = fn).catch(() => {});

    // Listen for NXM deep-link URLs (e.g. nxm://skyrimspecialedition/mods/123/files/456?key=abc&expires=123)
    onOpenUrl((urls) => {
      for (const url of urls) {
        if (url.startsWith("nxm://")) {
          handleNxmLink(url);
        }
      }
    });

    // Close dropdown on click outside
    function handleClickOutside(e: MouseEvent) {
      const target = e.target as HTMLElement;
      if (!target.closest(".sidebar-game-section")) {
        gameDropdownOpen = false;
      }
      if (!target.closest(".queue-section") && !target.closest(".queue-popover")) {
        showQueue = false;
      }
    }

    // Global Escape key handler — dismiss toasts and close dropdowns
    function handleKeydown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        if (get(errorMessage)) { errorMessage.set(null); return; }
        if (get(successMessage)) { successMessage.set(null); return; }
        if (gameDropdownOpen) { gameDropdownOpen = false; return; }
        if (showQueue) { showQueue = false; return; }
      }
    }

    document.addEventListener("click", handleClickOutside);
    document.addEventListener("keydown", handleKeydown);
    return () => {
      document.removeEventListener("click", handleClickOutside);
      document.removeEventListener("keydown", handleKeydown);
      if (queueUnlisten) queueUnlisten();
    };
  });

  async function loadDetectedGames() {
    try {
      detectedGames = await getAllGames();
      // Auto-select the first game if none is selected
      if (!get(selectedGame) && detectedGames.length > 0) {
        pickGame(detectedGames[0]);
      }
    } catch {
      // Games will load when user navigates to Dashboard
    }
  }

  function pickGame(game: DetectedGame) {
    selectedGame.set(game);
    selectedBottle.set(game.bottle_name);
    gameDropdownOpen = false;
  }

  function toggleGameDropdown() {
    gameDropdownOpen = !gameDropdownOpen;
  }

  async function handleNxmLink(nxmUrl: string) {
    // Extract game slug from nxm://skyrimspecialedition/mods/...
    const slugMatch = nxmUrl.match(/^nxm:\/\/([^/]+)\//);
    if (!slugMatch) {
      showError("Invalid NXM link format.");
      return;
    }
    const nxmSlug = slugMatch[1].toLowerCase();

    // Find a detected game matching this NXM slug
    let game = get(selectedGame);
    let bottle = get(selectedBottle);

    if (!game || game.nexus_slug !== nxmSlug) {
      // Auto-detect: scan all games across all bottles for one matching this slug
      try {
        const allGames = await getAllGames();
        const match = allGames.find((g) => g.nexus_slug === nxmSlug);
        if (match) {
          game = match;
          bottle = match.bottle_name;
          selectedGame.set(match);
          selectedBottle.set(match.bottle_name);
        } else {
          showError(`No installed game found for NexusMods domain "${nxmSlug}". Make sure the game is detected on the Dashboard.`);
          return;
        }
      } catch {
        showError("Failed to scan games for NXM link. Select a game manually on the Dashboard.");
        return;
      }
    }

    if (!game || !bottle) {
      showError("Select a game first before installing from NexusMods links.");
      return;
    }

    // Navigate to mods page so user sees progress
    currentPage.set("mods");
    showSuccess("Downloading mod from NexusMods...");

    try {
      await downloadFromNexus(nxmUrl, game.game_id, bottle, true);
      showSuccess("Mod installed from NexusMods link!");
    } catch (err: unknown) {
      showError(`NXM download failed: ${err instanceof Error ? err.message : String(err)}`);
    }
  }

  async function checkForUpdates() {
    updateCheckingStore.set(true);
    updateErrorStore.set(null);
    try {
      const update = await check();
      if (update) {
        updateAvailable = true;
        updateVersion = update.version;
        updateVersionStore.set(update.version);
        update.downloadAndInstall((progress) => {
          if (progress.event === "Started" && progress.data.contentLength) {
            updateDownloading = true;
          } else if (progress.event === "Progress") {
            updateProgress += progress.data.chunkLength;
          } else if (progress.event === "Finished") {
            updateReady = true;
            updateDownloading = false;
            updateReadyStore.set(true);
          }
        }).then(() => {
          updateReady = true;
          updateDownloading = false;
          updateReadyStore.set(true);
        }).catch((e) => {
          updateDownloading = false;
          console.warn("[updater] Download failed:", e);
        });
      }
    } catch (e) {
      console.warn("[updater] Check failed:", e);
      updateErrorStore.set(String(e));
    } finally {
      updateCheckingStore.set(false);
    }
  }

  // Register so settings page can trigger manual checks
  setUpdateCheckFn(checkForUpdates);

  async function handleRelaunch() {
    await relaunch();
  }

  function navigate(page: string) {
    currentPage.set(page);
  }

  async function handleRetryDownload(id: number) {
    try {
      await retryDownload(id);
      queueItems = await getDownloadQueue();
    } catch { /* ignore */ }
  }

  async function handleCancelDownload(id: number) {
    try {
      await cancelDownload(id);
      queueItems = await getDownloadQueue();
    } catch { /* ignore */ }
  }

  async function handleClearFinished() {
    try {
      await clearFinishedDownloads();
      queueItems = await getDownloadQueue();
    } catch { /* ignore */ }
  }

  // Auto-dismiss success toasts after 4 seconds
  let successTimer: ReturnType<typeof setTimeout> | null = null;
  $effect(() => {
    if ($successMessage) {
      if (successTimer) clearTimeout(successTimer);
      successTimer = setTimeout(() => successMessage.set(null), 4000);
    }
    return () => { if (successTimer) clearTimeout(successTimer); };
  });

  function formatBytes(bytes: number): string {
    if (bytes === 0) return "0 B";
    const k = 1024;
    const sizes = ["B", "KB", "MB", "GB"];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + " " + sizes[i];
  }
</script>

<div class="app-shell">
  <nav class="sidebar">
    <!-- Traffic light zone (macOS window controls sit here) -->
    <div class="sidebar-traffic-zone" data-tauri-drag-region></div>

    <!-- Brand lockup: sits below traffic lights, above nav -->
    <div class="sidebar-brand-section">
      <button class="sidebar-brand-btn" onclick={() => navigate("dashboard")}>
        <img class="brand-icon" src="/corkscrew-icon.png" alt="" width="28" height="28" draggable="false" />
        <div class="brand-text">
          <span class="brand-name">Corkscrew</span>
          <span class="brand-tagline">Mod Manager</span>
        </div>
      </button>
    </div>

    <!-- Game selector -->
    <div class="sidebar-game-section">
      {#if detectedGames.length > 0}
        <button class="game-selector-btn" onclick={toggleGameDropdown}>
          {#if $selectedGame}
            <GameIcon gameId={$selectedGame.game_id} size={22} />
            <div class="game-selector-text">
              <span class="game-selector-name">{$selectedGame.display_name}</span>
              <span class="game-selector-bottle">{$selectedGame.bottle_name}</span>
            </div>
          {:else}
            <svg class="game-selector-placeholder-icon" width="22" height="22" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
              <rect x="2" y="4" width="12" height="8" rx="2" opacity="0.4" />
              <circle cx="6" cy="8" r="1.5" opacity="0.4" />
              <circle cx="10" cy="8" r="1.5" opacity="0.4" />
            </svg>
            <span class="game-selector-placeholder">Select a game</span>
          {/if}
          <svg class="game-selector-chevron" class:open={gameDropdownOpen} width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M3 4l2 2 2-2" />
          </svg>
        </button>

        {#if gameDropdownOpen}
          <div class="game-dropdown">
            {#each detectedGames as game}
              <button
                class="game-dropdown-item"
                class:active={$selectedGame?.game_id === game.game_id && $selectedGame?.bottle_name === game.bottle_name}
                onclick={() => pickGame(game)}
              >
                <GameIcon gameId={game.game_id} size={18} />
                <div class="game-dropdown-text">
                  <span class="game-dropdown-name">{game.display_name}</span>
                  <span class="game-dropdown-bottle">{game.bottle_name}</span>
                </div>
              </button>
            {/each}
          </div>
        {/if}
      {:else}
        <div class="game-selector-empty">
          <span class="game-selector-placeholder">No games detected</span>
        </div>
      {/if}
    </div>

    <ul class="nav-list">
      {#each navItems as item}
        <li>
          <button
            class="nav-item"
            class:active={$currentPage === item.id}
            onclick={() => navigate(item.id)}
          >
            <span class="nav-icon">
              {#if item.id === "dashboard"}
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <rect x="1.5" y="1.5" width="5" height="5" rx="1" />
                  <rect x="9.5" y="1.5" width="5" height="5" rx="1" />
                  <rect x="1.5" y="9.5" width="5" height="5" rx="1" />
                  <rect x="9.5" y="9.5" width="5" height="5" rx="1" />
                </svg>
              {:else if item.id === "mods"}
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <rect x="3" y="1.5" width="10" height="13" rx="1.5" />
                  <line x1="5.5" y1="4.5" x2="10.5" y2="4.5" />
                  <line x1="5.5" y1="7" x2="10.5" y2="7" />
                  <line x1="5.5" y1="9.5" x2="8.5" y2="9.5" />
                </svg>
              {:else if item.id === "plugins"}
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <rect x="2.5" y="2" width="11" height="3" rx="1" />
                  <rect x="2.5" y="6.5" width="11" height="3" rx="1" />
                  <rect x="2.5" y="11" width="11" height="3" rx="1" />
                </svg>
              {:else if item.id === "discover"}
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <circle cx="8" cy="8" r="6.5" />
                  <path d="M5.5 5.5l2 4.5 4.5 2-2-4.5z" />
                </svg>
              {:else if item.id === "logs"}
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M8 1.5L1.5 5v6L8 14.5 14.5 11V5L8 1.5z" />
                  <circle cx="8" cy="7.5" r="1.5" />
                  <path d="M8 9v2.5" />
                </svg>
              {:else if item.id === "profiles"}
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <rect x="2" y="2" width="12" height="4" rx="1" />
                  <rect x="2" y="10" width="12" height="4" rx="1" />
                  <line x1="5" y1="4" x2="5" y2="4" />
                  <line x1="5" y1="12" x2="5" y2="12" />
                </svg>
              {:else if item.id === "settings"}
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <circle cx="8" cy="8" r="2.5" />
                  <path d="M8 1.5v2M8 12.5v2M2.7 4.5l1.7 1M11.6 10.5l1.7 1M1.5 8h2M12.5 8h2M2.7 11.5l1.7-1M11.6 5.5l1.7-1" />
                </svg>
              {:else}
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <circle cx="8" cy="8" r="6.5" />
                  <line x1="8" y1="7" x2="8" y2="11" />
                  <circle cx="8" cy="5" r="0.5" fill="currentColor" />
                </svg>
              {/if}
            </span>
            <span class="nav-label">{item.label}</span>
          </button>
        </li>
      {/each}
    </ul>

    <div class="sidebar-footer">
      <button
        class="sidebar-gh-btn"
        onclick={() => openUrl("https://github.com/cashcon57/corkscrew")}
        title="View on GitHub"
      >
        <svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor">
          <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8z" />
        </svg>
        <span>GitHub</span>
      </button>

      <!-- Download Queue Indicator -->
      <div class="queue-section">
        <button
          bind:this={queueBtnEl}
          class="queue-btn"
          class:queue-active={activeDownloads > 0}
          class:queue-error={failedDownloads > 0 && activeDownloads === 0}
          onclick={(e) => { e.stopPropagation(); showQueue = !showQueue; }}
          title="Download Queue"
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
            <polyline points="7 10 12 15 17 10" />
            <line x1="12" y1="15" x2="12" y2="3" />
          </svg>
          {#if activeDownloads > 0}
            <span class="queue-badge queue-badge-active">{activeDownloads}</span>
          {:else if failedDownloads > 0}
            <span class="queue-badge queue-badge-error">{failedDownloads}</span>
          {/if}
        </button>

      </div>

      {#if updateReady}
        <button class="update-btn update-ready" onclick={handleRelaunch} title="Restart to apply update">
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="23 4 23 10 17 10" />
            <path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10" />
          </svg>
          Restart for v{updateVersion}
        </button>
      {:else if updateAvailable && updateDownloading}
        <span class="update-btn update-downloading">
          <span class="spinner spinner-sm"></span>
          Updating...
        </span>
      {:else}
        <span class="sidebar-version">v{$appVersion}</span>
      {/if}
    </div>
  </nav>

  <div class="content-column">
    <!-- Drag region for window titlebar (no content, just draggable) -->
    <div class="content-drag-region" data-tauri-drag-region></div>

    <main class="content">
      {#if $errorMessage}
        <div class="toast toast-error" role="alert">
          <svg class="toast-icon" width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="7" cy="7" r="6" />
            <line x1="7" y1="4" x2="7" y2="7.5" />
            <circle cx="7" cy="10" r="0.5" fill="currentColor" />
          </svg>
          <span class="toast-text">{$errorMessage}</span>
          <button class="toast-dismiss" onclick={() => errorMessage.set(null)} aria-label="Dismiss error">
            <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
              <line x1="2" y1="2" x2="8" y2="8" />
              <line x1="8" y1="2" x2="2" y2="8" />
            </svg>
          </button>
        </div>
      {/if}

      {#if $successMessage}
        <div class="toast toast-success" role="status">
          <svg class="toast-icon" width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="7" cy="7" r="6" />
            <path d="M4.5 7l2 2 3-3.5" />
          </svg>
          <span class="toast-text">{$successMessage}</span>
          <button class="toast-dismiss" onclick={() => successMessage.set(null)} aria-label="Dismiss notification">
            <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
              <line x1="2" y1="2" x2="8" y2="8" />
              <line x1="8" y1="2" x2="2" y2="8" />
            </svg>
          </button>
        </div>
      {/if}

      <slot />
    </main>
  </div>

  {#if $collectionInstallStatus?.active}
    <div class="global-status-bar">
      <div class="status-bar-content">
        <div class="status-spinner"></div>
        <div class="status-text">
          <span class="status-collection">{$collectionInstallStatus.collectionName}</span>
          <span class="status-detail">
            {$collectionInstallStatus.current}/{$collectionInstallStatus.total}
            {#if $collectionInstallStatus.currentMod}
              &mdash; {$collectionInstallStatus.currentMod}
            {/if}
          </span>
        </div>
      </div>
      <div class="status-progress-track">
        <div class="status-progress-fill"
          style="width: {$collectionInstallStatus.total > 0
            ? ($collectionInstallStatus.current / $collectionInstallStatus.total) * 100
            : 0}%">
        </div>
      </div>
    </div>
  {/if}

  <!-- Download queue popover — rendered at app-shell level to escape sidebar overflow:hidden -->
  {#if showQueue}
    <div class="queue-popover" style={popoverStyle}>
      <div class="queue-popover-header">
        <span class="queue-popover-title">Downloads</span>
        {#if queueItems.some(i => i.status === "completed" || i.status === "failed")}
          <button class="queue-clear-btn" onclick={handleClearFinished}>Clear finished</button>
        {/if}
      </div>
      {#if queueItems.length === 0}
        <div class="queue-empty">No downloads</div>
      {:else}
        <div class="queue-list">
          {#each queueItems as item}
            <div class="queue-item" class:queue-item-failed={item.status === "failed"} class:queue-item-done={item.status === "completed"}>
              <div class="queue-item-info">
                <span class="queue-item-name">{item.file_name}</span>
                <span class="queue-item-status">
                  {#if item.status === "downloading"}
                    {formatBytes(item.downloaded_bytes)} / {formatBytes(item.total_bytes)}
                  {:else if item.status === "pending"}
                    Waiting...
                  {:else if item.status === "completed"}
                    Done
                  {:else if item.status === "failed"}
                    Failed{item.error ? `: ${item.error}` : ""}
                  {/if}
                </span>
              </div>
              {#if item.status === "downloading" && item.total_bytes > 0}
                <div class="queue-progress-bar">
                  <div class="queue-progress-fill" style="width: {(item.downloaded_bytes / item.total_bytes) * 100}%"></div>
                </div>
              {/if}
              {#if item.status === "failed"}
                <div class="queue-item-actions">
                  <button class="queue-action-btn" title="Retry" onclick={() => handleRetryDownload(item.id)}>
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <polyline points="23 4 23 10 17 10" /><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10" />
                    </svg>
                  </button>
                  <button class="queue-action-btn" title="Cancel" onclick={() => handleCancelDownload(item.id)}>
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
                  </button>
                </div>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .app-shell {
    display: flex;
    height: 100vh;
    overflow: hidden;
    padding: 8px;
    gap: 8px;
    background: #18181b;
  }

  :global([data-theme="light"]) .app-shell {
    background: #d2d2d7;
  }

  :global(html.vibrancy-active) .app-shell {
    background: transparent;
  }

  /* --- Sidebar --- */

  .sidebar {
    width: 220px;
    min-width: 220px;
    background: var(--bg-grouped);
    border-radius: 14px;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    position: relative;
    z-index: 10;
    border: 1px solid rgba(255, 255, 255, 0.08);
    box-shadow:
      inset 0 1px 0 0 rgba(255, 255, 255, 0.08),
      0 0 0 0.5px rgba(255, 255, 255, 0.04),
      0 1px 4px rgba(0, 0, 0, 0.12),
      0 4px 16px rgba(0, 0, 0, 0.08);
    backdrop-filter: blur(20px) saturate(1.2);
    -webkit-backdrop-filter: blur(20px) saturate(1.2);
  }

  :global([data-theme="light"]) .sidebar {
    border-color: rgba(0, 0, 0, 0.08);
    box-shadow:
      0 0 0 0.5px rgba(0, 0, 0, 0.04),
      0 1px 4px rgba(0, 0, 0, 0.06),
      0 4px 16px rgba(0, 0, 0, 0.04);
  }

  :global(html.vibrancy-active) .sidebar {
    backdrop-filter: blur(24px);
    -webkit-backdrop-filter: blur(24px);
    border-color: rgba(255, 255, 255, 0.10);
  }

  /* Traffic light spacer — clears macOS window controls.
     With 8px app-shell padding, traffic lights sit at ~y=4 in sidebar,
     ending at ~y=16. 28px gives clean clearance. */
  .sidebar-traffic-zone {
    height: 28px;
    flex-shrink: 0;
  }

  /* --- Brand lockup (below traffic lights) --- */

  .sidebar-brand-section {
    padding: 0 var(--space-3) var(--space-3);
    border-bottom: 1px solid var(--separator);
    margin-bottom: var(--space-2);
  }

  .sidebar-brand-btn {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-2);
    border-radius: var(--radius);
    transition: background var(--duration-fast) var(--ease);
    cursor: pointer;
    width: 100%;
  }

  .sidebar-brand-btn:hover {
    background: var(--surface-hover);
  }

  .brand-icon {
    flex-shrink: 0;
    border-radius: 6px;
  }

  .brand-text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .brand-name {
    font-size: 15px;
    font-weight: 700;
    letter-spacing: -0.02em;
    color: var(--text-primary);
    line-height: 1.2;
  }

  .brand-tagline {
    font-size: 11px;
    font-weight: 400;
    color: var(--text-tertiary);
    line-height: 1.2;
  }

  /* --- Game selector --- */

  .sidebar-game-section {
    padding: 0 var(--space-2) var(--space-2);
    border-bottom: 1px solid var(--separator);
    margin-bottom: var(--space-2);
    position: relative;
  }

  .game-selector-btn {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 8px 10px;
    border-radius: var(--radius);
    color: var(--text-primary);
    font-size: 13px;
    transition: background var(--duration-fast) var(--ease);
    cursor: pointer;
    text-align: left;
  }

  .game-selector-btn:hover {
    background: var(--surface-hover);
  }

  .game-selector-text {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
  }

  .game-selector-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    line-height: 1.3;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .game-selector-bottle {
    font-size: 10px;
    font-weight: 400;
    color: var(--text-tertiary);
    line-height: 1.3;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .game-selector-chevron {
    flex-shrink: 0;
    color: var(--text-quaternary);
    transition: transform var(--duration-fast) var(--ease);
    margin-left: auto;
  }

  .game-selector-chevron.open {
    transform: rotate(180deg);
  }

  .game-selector-placeholder {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-tertiary);
  }

  .game-selector-placeholder-icon {
    color: var(--text-quaternary);
    flex-shrink: 0;
  }

  .game-selector-empty {
    padding: 8px 10px;
  }

  /* Dropdown */

  .game-dropdown {
    position: absolute;
    top: 100%;
    left: var(--space-2);
    right: var(--space-2);
    background: var(--bg-grouped);
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: var(--radius);
    padding: 4px;
    z-index: 100;
    box-shadow:
      0 4px 24px rgba(0, 0, 0, 0.3),
      0 1px 4px rgba(0, 0, 0, 0.15),
      inset 0 1px 0 0 rgba(255, 255, 255, 0.06);
    backdrop-filter: blur(24px) saturate(1.3);
    -webkit-backdrop-filter: blur(24px) saturate(1.3);
    animation: dropdownIn var(--duration-fast) var(--ease-out);
    max-height: 240px;
    overflow-y: auto;
  }

  :global([data-theme="light"]) .game-dropdown {
    border-color: rgba(0, 0, 0, 0.12);
    box-shadow:
      0 4px 24px rgba(0, 0, 0, 0.12),
      0 1px 4px rgba(0, 0, 0, 0.06);
  }

  @keyframes dropdownIn {
    from { transform: translateY(-4px); opacity: 0; }
    to { transform: translateY(0); opacity: 1; }
  }

  .game-dropdown-item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 7px 8px;
    border-radius: calc(var(--radius) - 2px);
    color: var(--text-secondary);
    font-size: 12px;
    transition: all var(--duration-fast) var(--ease);
    cursor: pointer;
    text-align: left;
  }

  .game-dropdown-item:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .game-dropdown-item.active {
    background: var(--accent-subtle);
    color: var(--accent);
  }

  .game-dropdown-text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .game-dropdown-name {
    font-size: 12px;
    font-weight: 500;
    line-height: 1.3;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .game-dropdown-bottle {
    font-size: 10px;
    font-weight: 400;
    color: var(--text-tertiary);
    line-height: 1.3;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .game-dropdown-item.active .game-dropdown-bottle {
    color: var(--accent);
    opacity: 0.7;
  }

  /* --- Nav list --- */

  .nav-list {
    list-style: none;
    padding: 0 var(--space-2);
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .nav-item {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 8px 12px;
    border-radius: var(--radius);
    color: var(--text-secondary);
    font-size: 13px;
    font-weight: 500;
    transition: all var(--duration-fast) var(--ease);
    text-align: left;
  }

  .nav-item:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .nav-item.active {
    background: var(--accent-subtle);
    color: var(--accent);
  }

  .nav-icon {
    width: 16px;
    height: 16px;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  /* --- Sidebar footer --- */

  .sidebar-footer {
    padding: var(--space-2) var(--space-3) var(--space-3);
    border-top: 1px solid var(--separator);
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .sidebar-gh-btn {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    padding: 3px 8px 3px 6px;
    border-radius: var(--radius-sm);
    color: var(--text-tertiary);
    font-size: 11px;
    font-weight: 500;
    transition: all var(--duration-fast) var(--ease);
    cursor: pointer;
  }

  .sidebar-gh-btn:hover {
    background: var(--surface-hover);
    color: var(--text-secondary);
  }

  .sidebar-version {
    font-size: 10px;
    color: var(--text-quaternary);
    font-weight: 500;
    letter-spacing: 0.02em;
  }

  .update-btn {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 10px;
    font-weight: 600;
    padding: 3px 8px;
    border-radius: var(--radius-sm);
    cursor: pointer;
  }

  .update-ready {
    background: var(--accent-subtle);
    color: var(--accent);
    transition: background var(--duration-fast) var(--ease);
  }

  .update-ready:hover {
    background: var(--accent);
    color: white;
  }

  .update-downloading {
    color: var(--text-tertiary);
    cursor: default;
  }

  /* --- Content column --- */

  .content-column {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    border-radius: 14px;
    overflow: hidden;
    background: var(--bg-base);
    border: 1px solid rgba(255, 255, 255, 0.04);
    position: relative;
    box-shadow: inset 0 1px 0 0 rgba(255, 255, 255, 0.06);
  }

  :global([data-theme="light"]) .content-column {
    border-color: rgba(0, 0, 0, 0.06);
  }

  :global(html.vibrancy-active) .content-column {
    backdrop-filter: blur(16px) saturate(1.1);
    -webkit-backdrop-filter: blur(16px) saturate(1.1);
  }

  /* Drag region overlays the top of the content area — doesn't take up flow space */
  .content-drag-region {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: 28px;
    -webkit-app-region: drag;
    z-index: 5;
    pointer-events: auto;
  }

  /* --- Content area --- */

  .content {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-3) var(--space-6) var(--space-6);
    position: relative;
  }

  /* --- Toasts --- */

  .toast {
    position: fixed;
    top: calc(52px + var(--space-2));
    right: var(--space-4);
    padding: 10px var(--space-4);
    border-radius: var(--radius);
    font-size: 13px;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    z-index: 1000;
    box-shadow: var(--glass-edge-shadow), var(--shadow-lg);
    animation: toastIn var(--duration-slow) var(--ease-out);
    backdrop-filter: blur(24px) saturate(1.3);
    -webkit-backdrop-filter: blur(24px) saturate(1.3);
    max-width: 400px;
  }

  .toast-error {
    background: rgba(255, 69, 58, 0.18);
    border: 1px solid rgba(255, 69, 58, 0.25);
    color: var(--red);
  }

  .toast-success {
    background: rgba(48, 209, 88, 0.18);
    border: 1px solid rgba(48, 209, 88, 0.25);
    color: var(--green);
    top: calc(52px + var(--space-2) + 52px);
  }

  .toast-icon {
    flex-shrink: 0;
  }

  .toast-text {
    flex: 1;
    font-weight: 500;
  }

  .toast-dismiss {
    flex-shrink: 0;
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    opacity: 0.5;
    transition: opacity var(--duration-fast) var(--ease);
    min-width: 28px;
    min-height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .toast-dismiss:hover {
    opacity: 1;
  }

  @keyframes toastIn {
    from { transform: translateY(-8px); opacity: 0; }
    to { transform: translateY(0); opacity: 1; }
  }

  /* ============================
     Download Queue
     ============================ */
  .queue-section {
    position: relative;
  }

  .queue-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border-radius: var(--radius-sm);
    color: var(--text-tertiary);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
    position: relative;
  }

  .queue-btn:hover {
    background: var(--surface-hover);
    color: var(--text-secondary);
  }

  .queue-btn.queue-active {
    color: var(--accent);
  }

  .queue-btn.queue-error {
    color: var(--red);
  }

  .queue-badge {
    position: absolute;
    top: -2px;
    right: -2px;
    min-width: 14px;
    height: 14px;
    border-radius: 7px;
    font-size: 9px;
    font-weight: 700;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0 3px;
  }

  .queue-badge-active {
    background: var(--accent);
    color: #fff;
  }

  .queue-badge-error {
    background: var(--red);
    color: #fff;
  }

  .queue-popover {
    position: fixed;
    width: 300px;
    max-height: 400px;
    background: var(--bg-elevated);
    border: 1px solid var(--separator-opaque);
    border-radius: var(--radius);
    box-shadow: var(--shadow-lg);
    z-index: 200;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    animation: dropdownIn var(--duration-fast) var(--ease-out);
  }

  .queue-popover-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-2) var(--space-3);
    border-bottom: 1px solid var(--separator);
    flex-shrink: 0;
  }

  .queue-popover-title {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .queue-clear-btn {
    font-size: 11px;
    color: var(--text-tertiary);
    cursor: pointer;
    transition: color var(--duration-fast) var(--ease);
  }

  .queue-clear-btn:hover {
    color: var(--text-primary);
  }

  .queue-empty {
    padding: var(--space-4);
    text-align: center;
    font-size: 12px;
    color: var(--text-tertiary);
  }

  .queue-list {
    overflow-y: auto;
    max-height: 340px;
  }

  .queue-item {
    padding: var(--space-2) var(--space-3);
    border-bottom: 1px solid var(--separator);
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .queue-item:last-child {
    border-bottom: none;
  }

  .queue-item-failed {
    background: color-mix(in srgb, var(--red) 5%, transparent);
  }

  .queue-item-done {
    opacity: 0.6;
  }

  .queue-item-info {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }

  .queue-item-name {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .queue-item-status {
    font-size: 10px;
    color: var(--text-tertiary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .queue-progress-bar {
    height: 3px;
    background: var(--bg-tertiary);
    border-radius: 2px;
    overflow: hidden;
  }

  .queue-progress-fill {
    height: 100%;
    background: var(--accent);
    border-radius: 2px;
    transition: width 0.2s ease;
  }

  .queue-item-actions {
    display: flex;
    gap: var(--space-1);
    align-self: flex-end;
  }

  .queue-action-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    border-radius: var(--radius-sm);
    color: var(--text-tertiary);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
  }

  .queue-action-btn:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  /* ---- Global collection install status bar ---- */
  .global-status-bar {
    position: fixed;
    bottom: 16px;
    left: 16px;
    width: 220px;
    background: var(--bg-secondary);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    padding: 10px 12px;
    z-index: 300;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.3);
  }

  .status-bar-content {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 8px;
  }

  .status-spinner {
    width: 14px;
    height: 14px;
    border: 2px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
    flex-shrink: 0;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .status-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    overflow: hidden;
  }

  .status-collection {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .status-detail {
    font-size: 10px;
    color: var(--text-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .status-progress-track {
    height: 3px;
    background: var(--bg-tertiary);
    border-radius: 2px;
    overflow: hidden;
  }

  .status-progress-fill {
    height: 100%;
    background: var(--accent);
    border-radius: 2px;
    transition: width 0.3s ease;
  }
</style>
