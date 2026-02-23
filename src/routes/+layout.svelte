<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import "../app.css";
  import { goto } from "$app/navigation";
  import { currentPage, errorMessage, successMessage, selectedGame, selectedBottle, showError, showSuccess, appVersion, collectionInstallStatus, updateReady as updateReadyStore, updateVersion as updateVersionStore, updateNotes as updateNotesStore, updateChecking as updateCheckingStore, updateError as updateErrorStore, setUpdateCheckFn, notificationCount, showNotificationLog, activeProfile, profileList, activeCollection, collectionList, sidebarCollapsed, controllerMode } from "$lib/stores";
  import { initTheme } from "$lib/theme";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { onOpenUrl } from "@tauri-apps/plugin-deep-link";
  import { getVersion } from "@tauri-apps/api/app";
  import { check } from "@tauri-apps/plugin-updater";
  import { relaunch } from "@tauri-apps/plugin-process";
  import { downloadFromNexus, getAllGames, getDownloadQueue, retryDownload, cancelDownload, clearFinishedDownloads, onDownloadQueueUpdate, listProfiles, listInstalledCollections, getConfig, launchGame, getAllInterruptedInstalls, resumeCollectionInstall, abandonCollectionInstall, getPendingWabbajackInstalls, checkSkyrimVersion, getPinnedGameVersion, pinGameVersion } from "$lib/api";
  import { resumeInstallTracking } from "$lib/installService";
  import type { CollectionInstallCheckpoint, WabbajackInstallStatus } from "$lib/types";
  import { get } from "svelte/store";
  import type { DetectedGame, QueueItem } from "$lib/types";
  import NotificationLog from "$lib/components/mods/NotificationLog.svelte";
  import FirstRunWizard from "$lib/components/FirstRunWizard.svelte";
  import SpotlightSearch from "$lib/components/SpotlightSearch.svelte";
  import TopBar from "$lib/components/topbar/TopBar.svelte";
  import { GamepadManager } from "$lib/gamepad";
  import type { GamepadAction } from "$lib/gamepad";
  import { getNotificationCount, logNotification } from "$lib/api";

  const navItems = [
    { id: "mods", label: "Mods" },
    { id: "plugins", label: "Load Order" },
    { id: "discover", label: "Discover" },
    { id: "profiles", label: "Profiles" },
    { id: "logs", label: "Crash Logs" },
    { id: "settings", label: "Settings" },
  ];

  let detectedGames = $state<DetectedGame[]>([]);

  // Download queue state
  let queueItems = $state<QueueItem[]>([]);
  let showQueue = $state(false);
  let queueUnlisten: (() => void) | null = null;

  let activeDownloads = $derived(queueItems.filter(i => i.status === "downloading" || i.status === "pending").length);
  let failedDownloads = $derived(queueItems.filter(i => i.status === "failed").length);

  // Total download progress (0-100) for the mini bar
  let downloadProgress = $derived.by(() => {
    const active = queueItems.filter(i => i.status === "downloading");
    if (active.length === 0) return 0;
    const totalBytes = active.reduce((sum, i) => sum + i.total_bytes, 0);
    const downloadedBytes = active.reduce((sum, i) => sum + i.downloaded_bytes, 0);
    return totalBytes > 0 ? Math.round((downloadedBytes / totalBytes) * 100) : 0;
  });

  // Download ETA tracking
  let lastDownloadSnapshot = $state<{ time: number; bytes: number } | null>(null);
  let downloadSpeed = $state(0); // bytes per second

  $effect(() => {
    const active = queueItems.filter(i => i.status === "downloading");
    if (active.length === 0) {
      lastDownloadSnapshot = null;
      downloadSpeed = 0;
      return;
    }
    const currentBytes = active.reduce((sum, i) => sum + i.downloaded_bytes, 0);
    const now = Date.now();
    if (lastDownloadSnapshot) {
      const elapsed = (now - lastDownloadSnapshot.time) / 1000;
      if (elapsed > 0.5) {
        const bytesDelta = currentBytes - lastDownloadSnapshot.bytes;
        downloadSpeed = Math.max(0, bytesDelta / elapsed);
        lastDownloadSnapshot = { time: now, bytes: currentBytes };
      }
    } else {
      lastDownloadSnapshot = { time: now, bytes: currentBytes };
    }
  });

  const downloadEta = $derived.by(() => {
    if (downloadSpeed <= 0) return "";
    const active = queueItems.filter(i => i.status === "downloading");
    const remaining = active.reduce((sum, i) => sum + (i.total_bytes - i.downloaded_bytes), 0);
    if (remaining <= 0) return "";
    const seconds = remaining / downloadSpeed;
    if (seconds < 60) return "< 1 min";
    if (seconds < 3600) return `~${Math.ceil(seconds / 60)} min`;
    const hrs = Math.floor(seconds / 3600);
    const mins = Math.ceil((seconds % 3600) / 60);
    return `~${hrs}h ${mins}m`;
  });

  // First-run wizard state
  let showFirstRunWizard = $state(false);

  // Spotlight search
  let showSpotlight = $state(false);

  // Game launch state
  let launching = $state(false);

  // Keyboard shortcuts modal
  let showShortcuts = $state(false);

  // Interrupted install resume state
  let interruptedInstall = $state<CollectionInstallCheckpoint | null>(null);
  let interruptedWj = $state<WabbajackInstallStatus | null>(null);
  let resumingInstall = $state(false);

  // Game version change detection
  let versionWarning = $state<{ oldVersion: string; newVersion: string } | null>(null);

  // Friendly error message mapping for download errors
  function friendlyError(raw: string): string {
    const lower = raw.toLowerCase();
    if (lower.includes("403") || lower.includes("premium")) return "Premium membership required for API downloads";
    if (lower.includes("404")) return "File was removed from NexusMods";
    if (lower.includes("timeout") || lower.includes("etimedout")) return "Network timeout \u2014 check your connection";
    if (lower.includes("429") || lower.includes("rate limit")) return "API rate limit reached \u2014 wait a moment";
    return raw;
  }

  // Auto-update state
  let updateAvailable = $state(false);
  let updateVersion = $state("");
  let updateBody = $state<string | null>(null);
  let updateDownloading = $state(false);
  let updateProgress = $state(0);
  let updateReady = $state(false);
  let showUpdateBanner = $state(false);
  let updateNotesExpanded = $state(false);

  // Queue popover positioning (fixed to escape sidebar overflow:hidden)
  let queueBtnEl = $state<HTMLElement | null>(null);
  let popoverStyle = $state('');
  $effect(() => {
    if (showQueue && queueBtnEl) {
      const rect = queueBtnEl.getBoundingClientRect();
      popoverStyle = `bottom: ${window.innerHeight - rect.top + 8}px; left: ${rect.left}px;`;
    }
  });

  // Log toasts to persistent notification log
  function logToast(level: string, message: string) {
    logNotification(level, message).catch(() => {});
    getNotificationCount().then(c => notificationCount.set(c)).catch(() => {});
  }

  // Override showError/showSuccess to also persist
  const originalShowError = showError;
  const originalShowSuccess = showSuccess;
  function wrappedShowError(msg: string) {
    originalShowError(msg);
    logToast("error", msg);
  }
  function wrappedShowSuccess(msg: string) {
    originalShowSuccess(msg);
    logToast("success", msg);
  }

  onMount(() => {
    initTheme();
    loadDetectedGames();
    getVersion().then(v => appVersion.set(v)).catch(() => {});

    // Check if first-run wizard should show
    getConfig().then(config => {
      if (!config.has_completed_setup) {
        showFirstRunWizard = true;
      }
    }).catch(() => {});
    getNotificationCount().then(c => notificationCount.set(c)).catch(() => {});

    // Check for app updates on startup
    checkForUpdates();

    // Check for interrupted installs from previous session
    checkInterruptedInstalls();

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
      if (!target.closest(".queue-section") && !target.closest(".queue-popover")) {
        showQueue = false;
      }
    }

    // Global keyboard shortcuts
    const navPageIds = ["mods", "plugins", "discover", "profiles", "logs", "settings"];

    function handleKeydown(e: KeyboardEvent) {
      // Cmd+K / Ctrl+K: spotlight search
      if ((e.metaKey || e.ctrlKey) && e.key === "k") {
        e.preventDefault();
        showSpotlight = !showSpotlight;
        return;
      }
      // Cmd+B / Ctrl+B: toggle sidebar
      if ((e.metaKey || e.ctrlKey) && e.key === "b") {
        e.preventDefault();
        sidebarCollapsed.update(v => !v);
        return;
      }
      // Cmd+, / Ctrl+,: settings
      if ((e.metaKey || e.ctrlKey) && e.key === ",") {
        e.preventDefault();
        navigate("settings");
        return;
      }
      // Cmd+1-6 / Ctrl+1-6: navigate to page
      if ((e.metaKey || e.ctrlKey) && e.key >= "1" && e.key <= "6") {
        e.preventDefault();
        const idx = parseInt(e.key) - 1;
        if (idx < navPageIds.length) navigate(navPageIds[idx]);
        return;
      }
      // Cmd+/ or Cmd+?: keyboard shortcuts modal
      if ((e.metaKey || e.ctrlKey) && (e.key === "/" || e.key === "?")) {
        e.preventDefault();
        showShortcuts = !showShortcuts;
        return;
      }
      if (e.key === "Escape") {
        if (showShortcuts) { showShortcuts = false; return; }
        if (showSpotlight) { showSpotlight = false; return; }
        if (get(errorMessage)) { errorMessage.set(null); return; }
        if (get(successMessage)) { successMessage.set(null); return; }
        if (showQueue) { showQueue = false; return; }
      }
    }

    document.addEventListener("click", handleClickOutside);
    document.addEventListener("keydown", handleKeydown);

    // Intercept clicks on links inside rendered markdown to open externally
    // instead of navigating within the SPA (which would cause 404s)
    document.addEventListener("click", (e: MouseEvent) => {
      const target = (e.target as HTMLElement)?.closest(".rendered-markdown a");
      if (!target) return;
      const href = (target as HTMLAnchorElement).getAttribute("href");
      if (href && !href.startsWith("#")) {
        e.preventDefault();
        e.stopPropagation();
        try {
          const parsed = new URL(href, window.location.href);
          if (parsed.protocol === "http:" || parsed.protocol === "https:") {
            openUrl(parsed.href);
          }
        } catch { /* ignore invalid URLs */ }
      }
    }, true); // capture phase to intercept before SvelteKit routing

    // Gamepad / controller support
    const gamepad = new GamepadManager((action: GamepadAction) => {
      if (!get(controllerMode)) return;
      switch (action) {
        case "shoulder_left": {
          const idx = navPageIds.indexOf(get(currentPage));
          if (idx > 0) navigate(navPageIds[idx - 1]);
          break;
        }
        case "shoulder_right": {
          const idx = navPageIds.indexOf(get(currentPage));
          if (idx < navPageIds.length - 1) navigate(navPageIds[idx + 1]);
          break;
        }
        case "back":
          if (showSpotlight) showSpotlight = false;
          break;
        case "menu":
          showSpotlight = !showSpotlight;
          break;
        case "confirm": {
          const focused = document.activeElement as HTMLElement | null;
          focused?.click();
          break;
        }
        case "up":
        case "down":
        case "left":
        case "right": {
          // Focus navigation: move between focusable elements
          const focusables = Array.from(
            document.querySelectorAll<HTMLElement>(
              'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])'
            )
          ).filter(el => el.offsetParent !== null);
          const idx = focusables.indexOf(document.activeElement as HTMLElement);
          if (action === "down" || action === "right") {
            const next = idx < focusables.length - 1 ? idx + 1 : 0;
            focusables[next]?.focus();
          } else {
            const prev = idx > 0 ? idx - 1 : focusables.length - 1;
            focusables[prev]?.focus();
          }
          break;
        }
      }
    });
    gamepad.start();

    return () => {
      document.removeEventListener("click", handleClickOutside);
      document.removeEventListener("keydown", handleKeydown);
      gamepad.stop();
      if (queueUnlisten) queueUnlisten();
    };
  });

  async function checkInterruptedInstalls() {
    try {
      const checkpoints = await getAllInterruptedInstalls();
      if (checkpoints.length > 0) {
        interruptedInstall = checkpoints[0];
        return;
      }
    } catch { /* no checkpoint table yet */ }

    try {
      const pending = await getPendingWabbajackInstalls();
      if (pending.length > 0) {
        interruptedWj = pending[0];
      }
    } catch { /* no wj table yet */ }
  }

  async function handleResumeInterrupted() {
    if (!interruptedInstall) return;
    resumingInstall = true;
    try {
      const cp = interruptedInstall;
      const modStatuses: Record<string, string> = cp.mod_statuses ? JSON.parse(cp.mod_statuses) : {};
      await resumeInstallTracking(cp.collection_name, cp.total_mods, cp.completed_mods, modStatuses);
      interruptedInstall = null;
      resumeCollectionInstall(cp.id).catch((e: unknown) => wrappedShowError(`Resume failed: ${e}`));
      navigate("collections");
    } catch (e) {
      wrappedShowError(`Failed to resume install: ${e}`);
    } finally {
      resumingInstall = false;
    }
  }

  async function handleDismissInterrupted() {
    if (interruptedInstall) {
      try { await abandonCollectionInstall(interruptedInstall.id); } catch {}
      interruptedInstall = null;
    }
    if (interruptedWj) {
      interruptedWj = null;
    }
  }

  async function loadDetectedGames() {
    try {
      detectedGames = await getAllGames();
      // Auto-select the first game if none is selected
      if (!get(selectedGame) && detectedGames.length > 0) {
        pickGame(detectedGames[0]);
      } else {
        // If a game was already selected (restored from store), load its profiles + collections
        const currentGame = get(selectedGame);
        if (currentGame) {
          loadProfilesForGame(currentGame);
          loadCollectionsForGame(currentGame);
        }
      }
    } catch {
      // Games will load when user navigates to Dashboard
    }
  }

  function pickGame(game: DetectedGame) {
    selectedGame.set(game);
    selectedBottle.set(game.bottle_name);
    loadProfilesForGame(game);
    loadCollectionsForGame(game);
    checkGameVersion(game);
  }

  async function checkGameVersion(game: DetectedGame) {
    if (game.game_id !== "skyrimse") return;
    try {
      const status = await checkSkyrimVersion(game.game_id, game.bottle_name);
      const currentVersion = status.current_version;
      const pinned = await getPinnedGameVersion(game.game_id, game.bottle_name);
      if (!pinned) {
        // First time seeing this game — pin without warning
        await pinGameVersion(game.game_id, game.bottle_name, currentVersion);
      } else if (pinned !== currentVersion) {
        versionWarning = { oldVersion: pinned, newVersion: currentVersion };
      }
    } catch { /* version check not available */ }
  }

  function handleAcknowledgeVersion() {
    const game = $selectedGame;
    if (!game || !versionWarning) return;
    pinGameVersion(game.game_id, game.bottle_name, versionWarning.newVersion);
    versionWarning = null;
  }

  function handleDismissVersionWarning() {
    versionWarning = null;
  }

  async function loadProfilesForGame(game: DetectedGame) {
    try {
      const profiles = await listProfiles(game.game_id, game.bottle_name);
      profileList.set(profiles);
      const active = profiles.find(p => p.is_active) ?? null;
      activeProfile.set(active);
    } catch {
      profileList.set([]);
      activeProfile.set(null);
    }
  }

  async function loadCollectionsForGame(game: DetectedGame) {
    try {
      const collections = await listInstalledCollections(game.game_id, game.bottle_name);
      collectionList.set(collections);
      // Keep current active collection if it still exists in the new list,
      // otherwise clear it (no persistent is_active flag for collections)
      const current = get(activeCollection);
      if (current && !collections.find(c => c.name === current.name)) {
        activeCollection.set(null);
      }
    } catch {
      collectionList.set([]);
      activeCollection.set(null);
    }
  }

  async function handleLaunchGame(useSkse: boolean = false) {
    if (!$selectedGame || launching) return;
    launching = true;
    try {
      const result = await launchGame($selectedGame.game_id, $selectedGame.bottle_name, useSkse);
      if (result.success) {
        showSuccess(`Launched ${$selectedGame.display_name}`);
      } else {
        showError(`Failed to launch ${$selectedGame.display_name}`);
      }
    } catch (e: unknown) {
      showError(`Launch error: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      launching = false;
    }
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
        updateBody = update.body ?? null;
        updateVersionStore.set(update.version);
        updateNotesStore.set(update.body ?? null);
        showUpdateBanner = true;
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

<div class="app-shell" class:controller-mode={$controllerMode}>
  <nav class="sidebar" class:collapsed={$sidebarCollapsed}>
    <!-- Traffic light zone (macOS window controls sit here) -->
    <div class="sidebar-traffic-zone" data-tauri-drag-region></div>

    <!-- Brand lockup: sits below traffic lights, above nav -->
    <div class="sidebar-brand-section">
      <button class="sidebar-brand-btn" onclick={() => navigate("dashboard")} title="Dashboard">
        <img class="brand-icon" src="/corkscrew-icon.png" alt="" width="28" height="28" draggable="false" />
        {#if !$sidebarCollapsed}
          <div class="brand-text">
            <span class="brand-name">Corkscrew</span>
            <span class="brand-tagline">Wine Dashboard</span>
          </div>
        {/if}
      </button>
    </div>

    <ul class="nav-list">
      {#each navItems as item}
        <li>
          <button
            class="nav-item"
            class:active={$currentPage === item.id}
            onclick={() => navigate(item.id)}
            title={item.label}
          >
            <span class="nav-icon">
              {#if item.id === "mods"}
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
            {#if !$sidebarCollapsed}<span class="nav-label">{item.label}</span>{/if}
          </button>
        </li>
      {/each}
    </ul>

    <!-- Download progress mini-bar -->
    {#if activeDownloads > 0}
      <div class="download-mini-bar">
        <div class="download-mini-fill" style="width: {downloadProgress}%"></div>
      </div>
    {/if}

    <div class="sidebar-footer">
      <!-- Collapse toggle -->
      <button
        class="sidebar-collapse-btn"
        onclick={() => sidebarCollapsed.update(v => !v)}
        title={$sidebarCollapsed ? "Expand sidebar (Cmd+B)" : "Collapse sidebar (Cmd+B)"}
      >
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="transform: {$sidebarCollapsed ? 'rotate(180deg)' : 'none'}">
          <rect x="3" y="3" width="18" height="18" rx="2" />
          <line x1="9" y1="3" x2="9" y2="21" />
          <polyline points="14 9 11 12 14 15" />
        </svg>
      </button>

      {#if !$sidebarCollapsed}
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
        <button
          class="sidebar-gh-btn"
          onclick={() => showShortcuts = true}
          title="Keyboard Shortcuts (Cmd+/)"
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="2" y="4" width="20" height="16" rx="2" /><line x1="6" y1="8" x2="6" y2="8" /><line x1="10" y1="8" x2="10" y2="8" /><line x1="14" y1="8" x2="14" y2="8" /><line x1="18" y1="8" x2="18" y2="8" /><line x1="8" y1="12" x2="16" y2="12" /><line x1="6" y1="16" x2="6" y2="16" /><line x1="18" y1="16" x2="18" y2="16" />
          </svg>
          <span>Shortcuts</span>
        </button>
      {/if}

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

      <!-- Notification Bell -->
      <button
        class="queue-btn"
        onclick={(e) => { e.stopPropagation(); showNotificationLog.update(v => !v); }}
        title="Notification Log"
      >
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9" />
          <path d="M13.73 21a2 2 0 0 1-3.46 0" />
        </svg>
        {#if $notificationCount > 0}
          <span class="queue-badge queue-badge-active">{$notificationCount}</span>
        {/if}
      </button>

      {#if !$sidebarCollapsed}
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
      {/if}
    </div>
  </nav>

  <div class="content-column">
    <TopBar
      {detectedGames}
      onPickGame={pickGame}
      onLaunchGame={() => handleLaunchGame(false)}
      onNavigate={navigate}
      {launching}
    />

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

      {#if showUpdateBanner && updateAvailable}
        <div class="update-banner" role="status">
          <div class="update-banner-header">
            <div class="update-banner-title">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="23 4 23 10 17 10" />
                <path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10" />
              </svg>
              <span>Update available: <strong>v{updateVersion}</strong></span>
            </div>
            <div class="update-banner-actions">
              {#if updateReady}
                <button class="btn btn-accent btn-sm" onclick={handleRelaunch}>Restart to Update</button>
              {:else if updateDownloading}
                <span class="update-banner-downloading"><span class="spinner spinner-sm"></span> Downloading...</span>
              {/if}
              <button class="update-banner-dismiss" onclick={() => showUpdateBanner = false} aria-label="Dismiss">
                <svg width="12" height="12" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
                  <line x1="2" y1="2" x2="8" y2="8" />
                  <line x1="8" y1="2" x2="2" y2="8" />
                </svg>
              </button>
            </div>
          </div>
          {#if updateBody}
            <div class="update-banner-notes" class:expanded={updateNotesExpanded}>
              <div class="update-notes-content">
                {updateBody}
              </div>
            </div>
            {#if updateBody.length > 150}
              <button class="update-notes-toggle" onclick={() => updateNotesExpanded = !updateNotesExpanded}>
                {updateNotesExpanded ? "Show less" : "Read more..."}
              </button>
            {/if}
          {/if}
        </div>
      {/if}

      {#if versionWarning}
        <div class="resume-banner version-warning" role="alert">
          <div class="resume-banner-icon">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="12" cy="12" r="10" />
              <line x1="12" y1="8" x2="12" y2="12" />
              <line x1="12" y1="16" x2="12.01" y2="16" />
            </svg>
          </div>
          <div class="resume-banner-text">
            <strong>Game Version Changed</strong>
            {versionWarning.oldVersion} &rarr; {versionWarning.newVersion} &mdash; This may break SKSE and script-dependent mods
          </div>
          <div class="resume-banner-actions">
            <button class="btn btn-accent btn-sm" onclick={handleAcknowledgeVersion}>
              Acknowledge
            </button>
            <button class="btn btn-ghost btn-sm" onclick={handleDismissVersionWarning}>Dismiss</button>
          </div>
        </div>
      {/if}

      {#if interruptedInstall || interruptedWj}
        <div class="resume-banner" role="alert">
          <div class="resume-banner-icon">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
              <line x1="12" y1="9" x2="12" y2="13" />
              <line x1="12" y1="17" x2="12.01" y2="17" />
            </svg>
          </div>
          <div class="resume-banner-text">
            <strong>Interrupted Installation</strong>
            {#if interruptedInstall}
              "{interruptedInstall.collection_name}" &mdash; {interruptedInstall.completed_mods}/{interruptedInstall.total_mods} mods completed
            {:else if interruptedWj}
              "{interruptedWj.modlist_name}" &mdash; interrupted
            {/if}
          </div>
          <div class="resume-banner-actions">
            {#if interruptedInstall}
              <button class="btn btn-accent btn-sm" onclick={handleResumeInterrupted} disabled={resumingInstall}>
                {resumingInstall ? "Resuming..." : "Resume"}
              </button>
            {/if}
            <button class="btn btn-ghost btn-sm" onclick={handleDismissInterrupted}>Dismiss</button>
          </div>
        </div>
      {/if}

      <slot />
    </main>
  </div>

  {#if $collectionInstallStatus?.active}
    <button class="global-status-bar" onclick={() => goto('/collections/progress')}>
      <div class="status-bar-content">
        {#if $collectionInstallStatus.phase === "complete"}
          <svg class="status-check" width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="var(--green, #30d158)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="3 7 6 10 11 4" />
          </svg>
        {:else}
          <div class="status-spinner"></div>
        {/if}
        <div class="status-text">
          <span class="status-collection">{$collectionInstallStatus.collectionName}</span>
          <span class="status-detail">
            {#if $collectionInstallStatus.phase === "downloading"}
              Downloading {$collectionInstallStatus.downloadProgress.completed}/{$collectionInstallStatus.downloadProgress.total}
              {#if $collectionInstallStatus.downloadProgress.active.length > 0}
                &mdash; {$collectionInstallStatus.downloadProgress.active[0].modName}
              {/if}
            {:else if $collectionInstallStatus.phase === "installing"}
              Installing {$collectionInstallStatus.installProgress.current}/{$collectionInstallStatus.installProgress.total}
              {#if $collectionInstallStatus.installProgress.currentMod}
                &mdash; {$collectionInstallStatus.installProgress.currentMod}
              {/if}
            {:else if $collectionInstallStatus.phase === "complete"}
              Complete
            {:else}
              {$collectionInstallStatus.current}/{$collectionInstallStatus.total}
              {#if $collectionInstallStatus.currentMod}
                &mdash; {$collectionInstallStatus.currentMod}
              {/if}
            {/if}
          </span>
        </div>
        <svg class="status-chevron" width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="4.5 2.5 8 6 4.5 9.5" />
        </svg>
      </div>
      <div class="status-progress-track">
        {#if $collectionInstallStatus.phase === "downloading"}
          <div class="status-progress-fill"
            style="width: {$collectionInstallStatus.downloadProgress.total > 0
              ? ($collectionInstallStatus.downloadProgress.completed / $collectionInstallStatus.downloadProgress.total) * 100
              : 0}%">
          </div>
        {:else if $collectionInstallStatus.phase === "installing"}
          <div class="status-progress-fill"
            style="width: {$collectionInstallStatus.installProgress.total > 0
              ? ($collectionInstallStatus.installProgress.current / $collectionInstallStatus.installProgress.total) * 100
              : 0}%">
          </div>
        {:else if $collectionInstallStatus.phase === "complete"}
          <div class="status-progress-fill status-progress-complete" style="width: 100%"></div>
        {:else}
          <div class="status-progress-fill"
            style="width: {$collectionInstallStatus.total > 0
              ? ($collectionInstallStatus.current / $collectionInstallStatus.total) * 100
              : 0}%">
          </div>
        {/if}
      </div>
    </button>
  {/if}

  <!-- Download queue popover — rendered at app-shell level to escape sidebar overflow:hidden -->
  {#if showQueue}
    <div class="queue-popover" style={popoverStyle}>
      <div class="queue-popover-header">
        <span class="queue-popover-title">Downloads{#if downloadEta}<span class="queue-eta"> &mdash; {downloadEta} remaining</span>{/if}</span>
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
                    Failed{item.error ? `: ${friendlyError(item.error)}` : ""}
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

  <NotificationLog />
</div>

{#if showFirstRunWizard}
  <FirstRunWizard onComplete={() => showFirstRunWizard = false} />
{/if}

{#if showSpotlight}
  <SpotlightSearch onClose={() => showSpotlight = false} />
{/if}

{#if showShortcuts}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="shortcuts-overlay" onclick={() => showShortcuts = false} role="dialog" aria-label="Keyboard Shortcuts">
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="shortcuts-card" onclick={(e) => e.stopPropagation()}>
      <div class="shortcuts-header">
        <h3>Keyboard Shortcuts</h3>
        <button class="shortcuts-close" onclick={() => showShortcuts = false}>
          <svg width="14" height="14" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
            <line x1="3" y1="3" x2="9" y2="9" /><line x1="9" y1="3" x2="3" y2="9" />
          </svg>
        </button>
      </div>
      <div class="shortcuts-grid">
        <div class="shortcut-row"><kbd>Cmd</kbd><kbd>K</kbd><span>Spotlight Search</span></div>
        <div class="shortcut-row"><kbd>Cmd</kbd><kbd>B</kbd><span>Toggle Sidebar</span></div>
        <div class="shortcut-row"><kbd>Cmd</kbd><kbd>,</kbd><span>Settings</span></div>
        <div class="shortcut-row"><kbd>Cmd</kbd><kbd>1</kbd>-<kbd>6</kbd><span>Navigate Pages</span></div>
        <div class="shortcut-row"><kbd>Cmd</kbd><kbd>/</kbd><span>This Modal</span></div>
        <div class="shortcut-row"><kbd>Esc</kbd><span>Close Panel / Dismiss</span></div>
      </div>
    </div>
  </div>
{/if}

<style>
  .app-shell {
    display: flex;
    height: 100vh;
    overflow: hidden;
    padding: 8px;
    gap: 8px;
    background: var(--bg-base);
  }

  :global(html.vibrancy-active:not([data-theme="light"])) .app-shell {
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
    transition: width 0.2s var(--ease), min-width 0.2s var(--ease);
  }

  .sidebar.collapsed {
    width: 56px;
    min-width: 56px;
    z-index: 10;
    border: 1px solid var(--separator);
    box-shadow:
      inset 0 1px 0 0 var(--surface-hover),
      0 0 0 0.5px var(--surface-subtle),
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

  :global([data-theme="light"]) .sidebar.collapsed {
    border-color: rgba(0, 0, 0, 0.08);
    box-shadow:
      0 0 0 0.5px rgba(0, 0, 0, 0.04),
      0 1px 4px rgba(0, 0, 0, 0.06),
      0 4px 16px rgba(0, 0, 0, 0.04);
  }

  :global(html.vibrancy-active:not([data-theme="light"])) .sidebar {
    backdrop-filter: blur(24px);
    -webkit-backdrop-filter: blur(24px);
    border-color: var(--separator);
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

  /* --- Download mini-bar --- */

  .download-mini-bar {
    height: 2px;
    background: var(--separator);
    flex-shrink: 0;
    overflow: hidden;
  }

  .download-mini-fill {
    height: 100%;
    background: var(--system-accent);
    transition: width 0.3s var(--ease);
    min-width: 2%;
  }

  /* --- Sidebar footer --- */

  .sidebar-footer {
    padding: var(--space-2) var(--space-3) var(--space-3);
    border-top: 1px solid var(--separator);
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .sidebar-collapse-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border-radius: var(--radius-sm);
    color: var(--text-tertiary);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
    flex-shrink: 0;
  }

  .sidebar-collapse-btn:hover {
    background: var(--surface-hover);
    color: var(--text-secondary);
  }

  /* Collapsed sidebar adjustments */
  .sidebar.collapsed .sidebar-brand-section {
    padding: 0 var(--space-1) var(--space-3);
  }

  .sidebar.collapsed .sidebar-brand-btn {
    justify-content: center;
    padding: var(--space-2);
  }

  .sidebar.collapsed .nav-list {
    padding: 0 var(--space-1);
  }

  .sidebar.collapsed .nav-item {
    justify-content: center;
    padding: var(--space-2);
  }

  .sidebar.collapsed .sidebar-footer {
    justify-content: center;
    flex-wrap: wrap;
    gap: var(--space-1);
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

  /* --- Update banner --- */

  .update-banner {
    background: var(--bg-elevated, rgba(255, 255, 255, 0.04));
    border: 1px solid var(--accent-subtle, rgba(217, 143, 64, 0.2));
    border-radius: var(--radius-md, 8px);
    margin: var(--space-3, 12px) var(--space-4, 16px) 0;
    padding: var(--space-3, 12px) var(--space-4, 16px);
  }

  .update-banner-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3, 12px);
  }

  .update-banner-title {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
    font-size: 13px;
    color: var(--text-primary);
  }

  .update-banner-title svg {
    color: var(--accent, #d98f40);
    flex-shrink: 0;
  }

  .update-banner-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
    flex-shrink: 0;
  }

  .update-banner-downloading {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--text-tertiary);
  }

  .update-banner-dismiss {
    background: none;
    border: none;
    color: var(--text-tertiary);
    cursor: pointer;
    padding: 4px;
    border-radius: var(--radius-sm, 4px);
    opacity: 0.6;
    transition: opacity 0.15s;
  }

  .update-banner-dismiss:hover {
    opacity: 1;
  }

  .update-banner-notes {
    margin-top: var(--space-2, 8px);
    max-height: 60px;
    overflow: hidden;
    transition: max-height 0.3s ease;
  }

  .update-banner-notes.expanded {
    max-height: 400px;
  }

  .update-notes-content {
    font-size: 12px;
    line-height: 1.5;
    color: var(--text-secondary);
    white-space: pre-wrap;
    word-break: break-word;
  }

  .update-notes-toggle {
    background: none;
    border: none;
    color: var(--accent, #d98f40);
    cursor: pointer;
    font-size: 11px;
    padding: 2px 0;
    margin-top: 2px;
  }

  .update-notes-toggle:hover {
    text-decoration: underline;
  }

  /* --- Resume banner --- */

  .resume-banner {
    display: flex;
    align-items: center;
    gap: var(--space-3, 12px);
    padding: 10px 16px;
    margin-bottom: var(--space-3, 12px);
    background: rgba(255, 170, 0, 0.08);
    border: 1px solid rgba(255, 170, 0, 0.25);
    border-radius: var(--radius-md, 8px);
    font-size: 13px;
  }

  .resume-banner-icon {
    color: #ffaa00;
    flex-shrink: 0;
  }

  .resume-banner-text {
    flex: 1;
    color: var(--text-primary);
  }

  .resume-banner-actions {
    display: flex;
    gap: var(--space-2, 8px);
    flex-shrink: 0;
  }

  .version-warning {
    background: rgba(255, 59, 48, 0.08);
    border-color: rgba(255, 59, 48, 0.25);
  }

  .version-warning .resume-banner-icon {
    color: #ff3b30;
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
    border: 1px solid var(--separator);
    position: relative;
    box-shadow: inset 0 1px 0 0 var(--surface);
  }

  :global(html.vibrancy-active:not([data-theme="light"])) .content-column {
    backdrop-filter: blur(16px) saturate(1.1);
    -webkit-backdrop-filter: blur(16px) saturate(1.1);
  }

  /* --- Content area --- */

  .content {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-3) var(--space-6) var(--space-6);
    position: relative;
  }

  @media (max-width: 800px) {
    .content {
      padding: var(--space-2) var(--space-3) var(--space-3);
    }
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

  .queue-eta {
    font-weight: 400;
    color: var(--text-tertiary);
    font-size: 11px;
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
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease), box-shadow var(--duration-fast) var(--ease), border-color var(--duration-fast) var(--ease);
    text-align: left;
  }

  .global-status-bar:hover {
    background: var(--bg-elevated, var(--bg-secondary));
    border-color: var(--accent-subtle, var(--border));
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.4);
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

  .status-check {
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
    flex: 1;
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

  .status-chevron {
    flex-shrink: 0;
    color: var(--text-tertiary);
    transition: transform var(--duration-fast) var(--ease);
  }

  .global-status-bar:hover .status-chevron {
    color: var(--text-secondary);
    transform: translateX(2px);
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

  .status-progress-complete {
    background: var(--green, #30d158);
  }

  /* Controller mode: larger touch targets for Steam Deck */
  :global(.controller-mode) button,
  :global(.controller-mode) .btn,
  :global(.controller-mode) .nav-item {
    min-height: 44px;
    font-size: 15px;
  }

  :global(.controller-mode) input,
  :global(.controller-mode) select {
    min-height: 44px;
    font-size: 15px;
  }

  :global(.controller-mode) .nav-item {
    padding: var(--space-3) var(--space-4);
  }

  :global(.controller-mode) :focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  /* Keyboard Shortcuts Modal */
  .shortcuts-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
    backdrop-filter: blur(4px);
    animation: shortcutsFadeIn 150ms ease-out;
  }

  .shortcuts-card {
    background: var(--bg-secondary);
    border: 1px solid var(--separator-opaque);
    border-radius: var(--radius);
    padding: var(--space-6);
    max-width: 380px;
    width: 90vw;
    box-shadow: var(--shadow-lg);
    animation: shortcutsSlideUp 200ms var(--ease-out);
  }

  .shortcuts-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--space-5);
  }

  .shortcuts-header h3 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .shortcuts-close {
    background: none;
    border: none;
    color: var(--text-tertiary);
    cursor: pointer;
    padding: var(--space-1);
    border-radius: var(--radius-sm);
  }

  .shortcuts-close:hover {
    color: var(--text-primary);
    background: var(--surface-hover);
  }

  .shortcuts-grid {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .shortcut-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: 13px;
    color: var(--text-secondary);
  }

  .shortcut-row span {
    margin-left: auto;
  }

  .shortcut-row kbd {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 24px;
    height: 22px;
    padding: 0 6px;
    background: var(--surface);
    border: 1px solid var(--separator-opaque);
    border-radius: 4px;
    font-family: var(--font-mono, monospace);
    font-size: 11px;
    font-weight: 500;
    color: var(--text-primary);
    box-shadow: 0 1px 0 var(--separator-opaque);
  }

  @keyframes shortcutsFadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  @keyframes shortcutsSlideUp {
    from { opacity: 0; transform: translateY(8px); }
    to { opacity: 1; transform: translateY(0); }
  }
</style>
