<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import "../app.css";
  import { goto } from "$app/navigation";
  import { currentPage, errorMessage, successMessage, selectedGame, selectedBottle, showError, showSuccess, appVersion, collectionInstallStatus, collectionUninstallStatus, updateReady as updateReadyStore, updateVersion as updateVersionStore, updateNotes as updateNotesStore, updateChecking as updateCheckingStore, updateError as updateErrorStore, setUpdateCheckFn, notificationCount, showNotificationLog, activeProfile, profileList, activeCollection, collectionList, sidebarCollapsed, controllerMode, pendingNxmInstall, nxmInstallComplete } from "$lib/stores";
  import { initTheme } from "$lib/theme";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { onOpenUrl } from "@tauri-apps/plugin-deep-link";
  import { getVersion } from "@tauri-apps/api/app";
  import { downloadFromNexus, installMod, getAllGames, getDownloadQueue, retryDownload, cancelDownload, clearFinishedDownloads, onDownloadQueueUpdate, listProfiles, listInstalledCollections, getConfig, setConfigValue, launchGame, getAllInterruptedInstalls, resumeCollectionInstall, abandonCollectionInstall, getCheckpointModNames, getPendingWabbajackInstalls, checkSkyrimVersion, getPinnedGameVersion, pinGameVersion, checkSteamStatus, addToSteam, fetchUpdate, installUpdate } from "$lib/api";
  import { resumeInstallTracking } from "$lib/installService";
  import { initHashingListener, destroyHashingListener, dismissHashingBanner } from "$lib/hashingService";
  import { hashingProgress } from "$lib/stores";
  import { cancelBackgroundHashing } from "$lib/api";
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
  import LlmChat from "$lib/components/LlmChat.svelte";

  const navItems = [
    { id: "discover", label: "Discover" },
    { id: "mods", label: "Mods" },
    { id: "plugins", label: "Load Order" },
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
  let disableGameFixesLayout = $state(false);

  // Keyboard shortcuts modal
  let showShortcuts = $state(false);

  // Interrupted install resume state
  let interruptedInstall = $state<CollectionInstallCheckpoint | null>(null);
  let interruptedWj = $state<WabbajackInstallStatus | null>(null);
  let resumingInstall = $state(false);

  // Game version change detection
  let versionWarning = $state<{ oldVersion: string; newVersion: string } | null>(null);

  // Steam integration prompt (Linux only)
  let showSteamPrompt = $state(false);

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
  let showChat = $state(false);
  let sidebarWidth = $state(300);
  let resizing = $state(false);

  function onResizeStart(e: MouseEvent) {
    e.preventDefault();
    resizing = true;
    const startX = e.clientX;
    const startW = sidebarWidth;

    function onMove(ev: MouseEvent) {
      sidebarWidth = Math.min(480, Math.max(200, startW + (ev.clientX - startX)));
    }
    function onUp() {
      resizing = false;
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    }
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }

  let updateNotesExpanded = $state(false);
  let updateObject = $state<any>(null); // Holds the Tauri Update object
  let multiVersionChangelog = $state<Array<{ version: string; body: string; date: string }>>([]);
  let changelogLoading = $state(false);
  let changelogExpanded = $state(false);
  let manualCheckDone = $state(false);

  // Sidebar update button state machine
  const sidebarUpdateState = $derived.by(() => {
    if (updateReady) return "ready" as const;
    if (updateAvailable && updateDownloading) return "downloading" as const;
    if (updateAvailable) return "available" as const;
    if (manualCheckDone && !$updateCheckingStore) return "up-to-date" as const;
    return "idle" as const;
  });

  async function handleSidebarUpdateClick() {
    if ($updateCheckingStore) return;
    if (sidebarUpdateState === "ready") { handleStartUpdate(); return; }
    if (sidebarUpdateState === "available") { handleStartUpdate(); return; }
    if (sidebarUpdateState === "downloading") return;

    // Idle or up-to-date → run check
    manualCheckDone = false;
    await checkForUpdates();
    if (!updateAvailable) {
      manualCheckDone = true;
      // Auto-clear "up to date" after 4s
      setTimeout(() => { manualCheckDone = false; }, 4000);
    }
  }

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
    getVersion().then(async (v) => {
      appVersion.set(v);
      // Show "Updated successfully!" toast if the app version changed since last launch
      try {
        const cfg = await getConfig();
        const lastVersion = (cfg as Record<string, unknown>).last_known_version;
        if (lastVersion && lastVersion !== v) {
          wrappedShowSuccess(`Updated from v${lastVersion} to v${v} successfully!`);
          // Clear any stale update state so we don't show "update available" for the version we just installed
          updateReadyStore.set(false);
          updateVersionStore.set("");
          updateNotesStore.set(null);
        }
        await setConfigValue("last_known_version", v);
      } catch { /* config not available yet */ }
    }).catch(() => {});

    // Check if first-run wizard should show + load game fixes preference
    getConfig().then(config => {
      if (!config.has_completed_setup) {
        showFirstRunWizard = true;
      }
      disableGameFixesLayout = config.disable_game_fixes === "true";
    }).catch(() => {});
    getNotificationCount().then(c => notificationCount.set(c)).catch(() => {});

    // Check for app updates on startup
    checkForUpdates();

    // Check for interrupted installs from previous session
    checkInterruptedInstalls();

    // Steam integration: auto-register on SteamOS, prompt on regular Linux
    checkSteamIntegration();

    // Background hashing event listener
    initHashingListener();

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
      destroyHashingListener();
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
      let modNames: string[] = [];
      try { modNames = await getCheckpointModNames(cp.id); } catch { /* fallback to Mod N */ }
      await resumeInstallTracking(cp.collection_name, cp.total_mods, cp.completed_mods, modStatuses, modNames);
      interruptedInstall = null;
      resumeCollectionInstall(cp.id).catch((e: unknown) => wrappedShowError(`Resume failed: ${e}`));
      goto("/collections/progress");
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

  async function checkSteamIntegration() {
    try {
      const status = await checkSteamStatus();
      if (!status.installed) return; // No Steam = nothing to do

      if (status.registered) return; // Already registered

      // Check if user already declined
      const cfg = await getConfig();
      if ((cfg as Record<string, unknown>).steam_integration_declined) return;

      if (status.is_deck) {
        // Auto-register on Steam Deck — it's the expected behavior
        try {
          await addToSteam();
          wrappedShowSuccess("Added Corkscrew to your Steam library");
          controllerMode.set(true); // Auto-enable controller mode on Deck
        } catch (e) {
          // Silent — Steam auto-registration is best-effort
        }
      } else {
        // Regular Linux — show a one-time prompt
        showSteamPrompt = true;
      }
    } catch { /* Steam integration not available */ }
  }

  async function handleSteamPromptAccept() {
    showSteamPrompt = false;
    try {
      await addToSteam();
      wrappedShowSuccess("Added Corkscrew to your Steam library");
    } catch (e) {
      wrappedShowError(`Failed to add to Steam: ${e}`);
    }
  }

  async function handleSteamPromptDecline() {
    showSteamPrompt = false;
    try { await setConfigValue("steam_integration_declined", "true"); } catch {}
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
      // Auto-select if exactly one collection is installed and none is active
      if (!get(activeCollection) && collections.length === 1) {
        activeCollection.set(collections[0]);
      }
    } catch {
      collectionList.set([]);
      activeCollection.set(null);
    }
  }

  async function handleLaunchGame(useSkse: boolean = false) {
    if (!$selectedGame || launching) return;

    doLaunchGame(useSkse);
  }

  async function doLaunchGame(useSkse: boolean) {
    if (!$selectedGame || launching) return;
    launching = true;
    try {
      const result = await launchGame($selectedGame.game_id, $selectedGame.bottle_name, useSkse);
      if (result.success) {
        showSuccess(result.warning
          ? `Launched ${$selectedGame.display_name}. ${result.warning}`
          : `Launched ${$selectedGame.display_name}`);
      } else {
        showError(`Failed to launch ${$selectedGame.display_name}`);
      }
    } catch (e: unknown) {
      showError(`Launch error: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      launching = false;
    }
  }

  // NXM install state
  let nxmInstalling = $state(false);

  async function handleNxmLink(nxmUrl: string) {
    // Extract game slug from nxm://skyrimspecialedition/mods/...
    const slugMatch = nxmUrl.match(/^nxm:\/\/([^/]+)\//);
    if (!slugMatch) {
      showError("Invalid NXM link format.");
      return;
    }
    const nxmSlug = slugMatch[1].toLowerCase();

    // Extract mod ID from URL for source tracking
    const modIdMatch = nxmUrl.match(/\/mods\/(\d+)\//);
    const nexusModId = modIdMatch ? parseInt(modIdMatch[1]) : undefined;

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

    // Navigate to mods page
    currentPage.set("mods");
    showSuccess("Downloading mod from NexusMods...");

    try {
      // Download only — don't auto-install
      const result = await downloadFromNexus(nxmUrl, game.game_id, bottle, false);
      const downloadInfo = result as unknown as { downloaded: string; mod_name: string; mod_version: string };

      // Show confirmation toast — user must click Install or Cancel
      pendingNxmInstall.set({
        archivePath: downloadInfo.downloaded,
        modName: downloadInfo.mod_name,
        modVersion: downloadInfo.mod_version,
        gameId: game.game_id,
        bottleName: bottle,
        nexusModId,
        nxmUrl,
      });
      // Clear the downloading toast
      successMessage.set(null);
    } catch (err: unknown) {
      showError(`NXM download failed: ${err instanceof Error ? err.message : String(err)}`);
    }
  }

  async function confirmNxmInstall() {
    const pending = get(pendingNxmInstall);
    if (!pending) return;
    nxmInstalling = true;
    try {
      await installMod(
        pending.archivePath,
        pending.gameId,
        pending.bottleName,
        pending.modName,
        pending.modVersion,
        "nexus",
        pending.nexusModId ? `https://www.nexusmods.com/mods/${pending.nexusModId}` : undefined,
        pending.nexusModId,
      );
      showSuccess(`Installed "${pending.modName}" successfully!`);
      pendingNxmInstall.set(null);
      // Signal mods page to reload
      nxmInstallComplete.update(n => n + 1);
    } catch (err: unknown) {
      showError(`Install failed: ${err instanceof Error ? err.message : String(err)}`);
    } finally {
      nxmInstalling = false;
    }
  }

  function cancelNxmInstall() {
    pendingNxmInstall.set(null);
  }

  async function checkForUpdates() {
    updateCheckingStore.set(true);
    updateErrorStore.set(null);
    try {
      const update = await fetchUpdate();
      if (update) {
        // Check if user dismissed this specific version
        try {
          const cfg = await getConfig();
          const dismissed = (cfg as Record<string, unknown>).dismissed_update_version;
          if (dismissed === update.version) {
            updateCheckingStore.set(false);
            return;
          }
        } catch { /* config not available */ }

        updateAvailable = true;
        updateVersion = update.version;
        updateBody = update.body ?? null;
        updateObject = update;
        updateVersionStore.set(update.version);
        updateNotesStore.set(update.body ?? null);
        showUpdateBanner = true;

        fetchMultiVersionChangelog();
      }
    } catch (e) {
      updateErrorStore.set(String(e));
    } finally {
      updateCheckingStore.set(false);
    }
  }

  async function fetchMultiVersionChangelog() {
    changelogLoading = true;
    try {
      const res = await fetch("https://api.github.com/repos/cashcon57/corkscrew/releases");
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const releases: Array<{ tag_name: string; body: string; published_at: string }> = await res.json();

      const current = $appVersion;
      const latest = updateVersion;

      // Filter releases between current (exclusive) and latest (inclusive)
      const changelog = releases
        .filter((r) => {
          const v = r.tag_name.replace(/^v/, "");
          return v !== current && compareVersions(v, current) > 0 && compareVersions(v, latest) <= 0;
        })
        .map((r) => ({
          version: r.tag_name,
          body: r.body || "No release notes.",
          date: new Date(r.published_at).toLocaleDateString(),
        }))
        .sort((a, b) => compareVersions(b.version.replace(/^v/, ""), a.version.replace(/^v/, "")));

      multiVersionChangelog = changelog;
    } catch (e) {
      // Changelog fetch failed — non-critical, silently continue
    } finally {
      changelogLoading = false;
    }
  }

  /** Compare semver strings: returns >0 if a > b, <0 if a < b, 0 if equal */
  function compareVersions(a: string, b: string): number {
    const pa = a.split(".").map(Number);
    const pb = b.split(".").map(Number);
    for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
      const na = pa[i] ?? 0;
      const nb = pb[i] ?? 0;
      if (na !== nb) return na - nb;
    }
    return 0;
  }

  async function handleStartUpdate() {
    if (!updateObject) return;
    updateDownloading = true;
    updateProgress = 0;
    console.log("[updater] Starting download+install via Rust, version:", updateObject.version);
    try {
      // Rust-side handles download, install, AND app restart
      await installUpdate((event) => {
        if (event.event === "Started") {
          console.log("[updater] Download started, contentLength:", event.data?.contentLength);
        } else if (event.event === "Progress") {
          updateProgress += event.data?.chunkLength ?? 0;
        } else if (event.event === "Finished") {
          console.log("[updater] Download finished");
        }
      });
      // If we reach here, download_and_install succeeded but app.restart() hasn't fired yet
      // (in practice, app.restart() exits the process so this line may not execute)
      console.log("[updater] Update installed — app is restarting...");
    } catch (e) {
      console.error("[updater] Update failed:", e);
      updateDownloading = false;
      updateErrorStore.set(
        `Update failed: ${e}. Please download the latest version manually from https://github.com/cashcon57/corkscrew/releases`
      );
    }
  }

  async function handleDismissUpdate() {
    showUpdateBanner = false;
    // Persist dismissal for this version
    try { await setConfigValue("dismissed_update_version", updateVersion); } catch {}
  }

  // Register so settings page can trigger manual checks
  setUpdateCheckFn(checkForUpdates);

  // handleStartUpdate now handles download+install+restart from Rust in one call.
  // No separate relaunch step needed.

  function navigate(page: string) {
    currentPage.set(page);
    // If we're on a sub-route (e.g. /collections/progress), navigate back to root
    if (window.location.pathname !== "/") {
      goto("/");
    }
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

  // NOTE: Post-install results persist until user explicitly dismisses them.
  // No auto-dismiss — the user clicks "View Mods", "Back to Collections", etc.

  function formatBytes(bytes: number): string {
    if (bytes === 0) return "0 B";
    const k = 1024;
    const sizes = ["B", "KB", "MB", "GB"];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + " " + sizes[i];
  }
</script>

<div class="app-shell" class:controller-mode={$controllerMode}>
  <nav class="sidebar" class:collapsed={$sidebarCollapsed} class:resizing style={!$sidebarCollapsed ? `width:${sidebarWidth}px;min-width:${sidebarWidth}px` : ''}>
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
              {#if item.id === "discover"}
                <!-- Compass — exploration/discovery -->
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <circle cx="8" cy="8" r="6.5" />
                  <path d="M5.5 5.5l2 4.5 4.5 2-2-4.5z" />
                </svg>
              {:else if item.id === "mods"}
                <!-- Cube/package — mod archives -->
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M8 1.5L2.5 4.5v7l5.5 3 5.5-3v-7L8 1.5z" />
                  <path d="M2.5 4.5L8 7.5l5.5-3" />
                  <line x1="8" y1="7.5" x2="8" y2="14.5" />
                </svg>
              {:else if item.id === "plugins"}
                <!-- Stacked layers — load order -->
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <rect x="2.5" y="2" width="11" height="3" rx="1" />
                  <rect x="2.5" y="6.5" width="11" height="3" rx="1" />
                  <rect x="2.5" y="11" width="11" height="3" rx="1" />
                </svg>
              {:else if item.id === "profiles"}
                <!-- Person — user profiles -->
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <circle cx="8" cy="5" r="2.5" />
                  <path d="M3 14c0-2.8 2.2-5 5-5s5 2.2 5 5" />
                </svg>
              {:else if item.id === "logs"}
                <!-- Document with warning — crash log analysis -->
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M4 1.5h5l3 3v10H4V1.5z" />
                  <path d="M9 1.5v3h3" />
                  <line x1="8" y1="7.5" x2="8" y2="10.5" />
                  <circle cx="8" cy="12" r="0.5" fill="currentColor" stroke="none" />
                </svg>
              {:else if item.id === "settings"}
                <!-- Gear — settings -->
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" />
                  <circle cx="12" cy="12" r="3" />
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

    <!-- Install/uninstall status — docked above footer -->
    {#if $collectionInstallStatus?.active}
      <div class="sidebar-status-bar">
        <button class="sidebar-status-btn" onclick={() => goto('/collections/progress')}>
          {#if $collectionInstallStatus.phase === "complete"}
            <svg class="status-check" width="12" height="12" viewBox="0 0 14 14" fill="none" stroke="var(--green, #30d158)" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
              <polyline points="3 7 6 10 11 4" />
            </svg>
          {:else}
            <div class="status-spinner status-spinner-sm"></div>
          {/if}
          <div class="status-text">
            <span class="status-collection">{$collectionInstallStatus.collectionName}</span>
            <span class="status-detail">
              {#if $collectionInstallStatus.phase === "downloading"}
                Downloading {$collectionInstallStatus.downloadProgress.completed}/{$collectionInstallStatus.downloadProgress.total}
                {#if $collectionInstallStatus.downloadSpeed > 0}
                  &mdash; {formatBytes($collectionInstallStatus.downloadSpeed)}/s
                {/if}
              {:else if $collectionInstallStatus.phase === "staging"}
                Extracting {$collectionInstallStatus.modDetails?.filter(m => m.status === "extracting").length ?? 0} mods...
              {:else if $collectionInstallStatus.phase === "installing"}
                Installing {$collectionInstallStatus.installProgress.current}/{$collectionInstallStatus.installProgress.total}
              {:else if $collectionInstallStatus.phase === "complete"}
                {$collectionInstallStatus.result?.installed ?? 0} installed{#if ($collectionInstallStatus.result?.failed ?? 0) > 0}, {$collectionInstallStatus.result?.failed} failed{/if}
              {:else}
                {$collectionInstallStatus.current}/{$collectionInstallStatus.total}
              {/if}
            </span>
          </div>
          {#if $collectionInstallStatus.phase !== "complete"}
            <span class="status-percent">{$collectionInstallStatus.overallProgress}%</span>
          {/if}
          <svg class="status-chevron" width="10" height="10" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="4.5 2.5 8 6 4.5 9.5" />
          </svg>
        </button>
        <div class="status-progress-track">
          <div class="status-progress-fill" class:status-progress-complete={$collectionInstallStatus.phase === "complete"}
            style="width: {$collectionInstallStatus.overallProgress}%">
          </div>
        </div>
      </div>
    {/if}

    {#if $collectionUninstallStatus?.active && $collectionUninstallStatus.phase !== "complete"}
      <div class="sidebar-status-bar sidebar-status-bar-uninstall">
        <div class="sidebar-status-btn" style="cursor: default;">
          <div class="status-spinner status-spinner-sm status-spinner-red"></div>
          <div class="status-text">
            <span class="status-collection">{$collectionUninstallStatus.collectionName}</span>
            <span class="status-detail">
              {#if $collectionUninstallStatus.phase === "removing"}
                Removing {$collectionUninstallStatus.currentMod}/{$collectionUninstallStatus.totalMods}
              {:else if $collectionUninstallStatus.phase === "redeploying"}
                Redeploying...
              {/if}
            </span>
          </div>
          {#if $collectionUninstallStatus.totalMods > 0 && $collectionUninstallStatus.phase === "removing"}
            <span class="status-percent">{Math.round(($collectionUninstallStatus.currentMod / $collectionUninstallStatus.totalMods) * 100)}%</span>
          {/if}
        </div>
        {#if $collectionUninstallStatus.totalMods > 0}
          <div class="status-progress-track">
            <div class="status-progress-fill status-progress-fill-red"
              style="width: {$collectionUninstallStatus.phase === "redeploying" ? 100 : ($collectionUninstallStatus.currentMod / $collectionUninstallStatus.totalMods) * 100}%">
            </div>
          </div>
        {/if}
      </div>
    {/if}

    <!-- Inline chat panel — fills remaining sidebar space -->
    {#if showChat && !$sidebarCollapsed}
      <LlmChat visible={showChat} onclose={() => showChat = false} />
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
          class="sidebar-gh-btn sidebar-kofi-btn"
          onclick={() => openUrl("https://ko-fi.com/cash508287")}
          title="Support Corkscrew on Ko-fi"
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none">
            <rect x="1" y="6" width="16" height="13" rx="3" stroke="currentColor" stroke-width="2" fill="none" />
            <path d="M17 9h2.5a3.5 3.5 0 0 1 0 7H17" stroke="currentColor" stroke-width="2" fill="none" />
            <path d="M9 10c-2.5 0-4 1.5-4 3.5S6.5 17 9 17s4-1.5 4-3.5S11.5 10 9 10z" fill="#FF5E5B" />
          </svg>
          <span>Ko-fi</span>
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
      </div>

      {#if !$sidebarCollapsed}
        <div class="version-update-row">
          <!-- Update check / status button -->
          <button
            class="update-check-btn"
            class:checking={$updateCheckingStore}
            class:up-to-date={sidebarUpdateState === "up-to-date"}
            class:update-avail={sidebarUpdateState === "available" || sidebarUpdateState === "downloading"}
            class:ready={sidebarUpdateState === "ready"}
            onclick={handleSidebarUpdateClick}
            title={sidebarUpdateState === "ready"
              ? `Restart for v${updateVersion}`
              : sidebarUpdateState === "available"
                ? `Update to v${updateVersion}`
                : sidebarUpdateState === "downloading"
                  ? "Downloading update..."
                  : sidebarUpdateState === "up-to-date"
                    ? "Up to date"
                    : "Check for updates"}
          >
            <!-- Rainbow ripple background -->
            {#if $updateCheckingStore}
              <span class="ripple-ring"></span>
            {/if}
            {#if sidebarUpdateState === "up-to-date"}
              <svg class="update-icon check-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="20 6 9 17 4 12" />
              </svg>
            {:else if sidebarUpdateState === "ready"}
              <svg class="update-icon restart-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="23 4 23 10 17 10" />
                <path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10" />
              </svg>
            {:else if sidebarUpdateState === "downloading"}
              <span class="spinner-update"></span>
            {:else if sidebarUpdateState === "available"}
              <svg class="update-icon arrow-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                <polyline points="7 10 12 15 17 10" />
                <line x1="12" y1="15" x2="12" y2="3" />
              </svg>
            {:else}
              <svg class="update-icon refresh-icon" class:spin={$updateCheckingStore} width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="23 4 23 10 17 10" />
                <path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10" />
              </svg>
            {/if}
          </button>

          <span class="sidebar-version">v{$appVersion}</span>

          <!-- Status pill -->
          {#if sidebarUpdateState === "ready"}
            <button class="update-pill ready" onclick={handleStartUpdate}>
              Restart
            </button>
          {:else if sidebarUpdateState === "available"}
            <button class="update-pill available" onclick={handleStartUpdate}>
              v{updateVersion}
            </button>
          {:else if sidebarUpdateState === "downloading"}
            <span class="update-pill downloading">
              Updating...
            </span>
          {/if}

          <!-- Chat button -->
          <button
            class="chat-toggle-btn"
            class:chat-active={showChat}
            onclick={() => showChat = !showChat}
            title={showChat ? "Close AI Chat" : "Open AI Chat"}
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
            </svg>
          </button>
        </div>
      {/if}
    </div>
    <!-- Resize handle -->
    {#if !$sidebarCollapsed}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="sidebar-resize-handle" onmousedown={onResizeStart}></div>
    {/if}
  </nav>

  <div class="content-column">
    <main class="content">
    <TopBar
      {detectedGames}
      onPickGame={pickGame}
      onLaunchGame={() => handleLaunchGame(false)}
      onNavigate={navigate}
      {launching}
    />
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

      {#if $pendingNxmInstall}
        <div class="toast toast-nxm" role="alertdialog">
          <svg class="toast-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
            <polyline points="7 10 12 15 17 10" />
            <line x1="12" y1="15" x2="12" y2="3" />
          </svg>
          <span class="toast-text">
            <strong>{$pendingNxmInstall.modName}</strong>
            {#if $pendingNxmInstall.modVersion}
              <span class="toast-version">v{$pendingNxmInstall.modVersion}</span>
            {/if}
            downloaded. Install this mod?
          </span>
          <div class="toast-actions">
            <button class="toast-action-btn toast-install-btn" onclick={confirmNxmInstall} disabled={nxmInstalling}>
              {nxmInstalling ? "Installing..." : "Install"}
            </button>
            <button class="toast-action-btn toast-cancel-btn" onclick={cancelNxmInstall} disabled={nxmInstalling}>
              Cancel
            </button>
          </div>
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
              <span>Corkscrew <strong>v{updateVersion}</strong> available</span>
            </div>
            <div class="update-banner-actions">
              {#if updateReady}
                <button class="btn btn-accent btn-sm update-ready-btn" onclick={handleStartUpdate}>Restart to Update</button>
              {:else if updateDownloading}
                <span class="update-banner-downloading"><span class="spinner spinner-sm"></span> Downloading...</span>
              {:else}
                <button class="btn btn-accent btn-sm" onclick={handleStartUpdate}>Update Now</button>
              {/if}
              <button class="update-banner-dismiss" onclick={handleDismissUpdate} aria-label="Dismiss update" title="Remind me later">
                <svg width="12" height="12" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
                  <line x1="2" y1="2" x2="8" y2="8" />
                  <line x1="8" y1="2" x2="2" y2="8" />
                </svg>
              </button>
            </div>
          </div>

          <!-- Multi-version changelog -->
          {#if multiVersionChangelog.length > 0}
            <button class="update-notes-toggle" onclick={() => changelogExpanded = !changelogExpanded}>
              {changelogExpanded ? "Hide changelog" : `View changelog (${multiVersionChangelog.length} version${multiVersionChangelog.length > 1 ? "s" : ""})`}
            </button>
            {#if changelogExpanded}
              <div class="update-changelog">
                {#each multiVersionChangelog as release}
                  <div class="changelog-entry">
                    <div class="changelog-version-header">
                      <strong>{release.version}</strong>
                      <span class="changelog-date">{release.date}</span>
                    </div>
                    <div class="changelog-body">{release.body}</div>
                  </div>
                {/each}
              </div>
            {/if}
          {:else if changelogLoading}
            <span class="update-banner-downloading" style="margin-top: 6px;"><span class="spinner spinner-sm"></span> Loading changelog...</span>
          {:else if updateBody}
            <div class="update-banner-notes" class:expanded={updateNotesExpanded}>
              <div class="update-notes-content">{updateBody}</div>
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

      {#if showSteamPrompt}
        <div class="resume-banner" role="alert" style="background: rgba(102, 192, 244, 0.08); border-color: rgba(102, 192, 244, 0.25);">
          <div class="resume-banner-icon" style="color: #66c0f4;">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
              <path d="M11.979 0C5.678 0 .511 4.86.022 11.037l6.432 2.658c.545-.371 1.203-.59 1.912-.59.063 0 .125.004.188.006l2.861-4.142V8.91c0-2.495 2.028-4.524 4.524-4.524 2.494 0 4.524 2.031 4.524 4.527s-2.03 4.525-4.524 4.525h-.105l-4.076 2.911c0 .052.004.105.004.159 0 1.875-1.515 3.396-3.39 3.396-1.635 0-3.016-1.173-3.331-2.727L.436 15.27C1.862 20.307 6.486 24 11.979 24c6.627 0 12-5.373 12-12s-5.372-12-12-12z"/>
            </svg>
          </div>
          <div class="resume-banner-text">
            <strong>Add to Steam Library?</strong>
            Access Corkscrew from Steam's game mode and your library
          </div>
          <div class="resume-banner-actions">
            <button class="btn btn-accent btn-sm" onclick={handleSteamPromptAccept}>Add to Steam</button>
            <button class="btn btn-ghost btn-sm" onclick={handleSteamPromptDecline}>No thanks</button>
          </div>
        </div>
      {/if}

      {#if interruptedInstall || interruptedWj}
        <div class="resume-banner resume-banner-prominent" role="alert">
          <div class="resume-banner-icon resume-icon-pulse">
            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="12" cy="12" r="10" />
              <polygon points="10 8 16 12 10 16" fill="currentColor" stroke="none" />
            </svg>
          </div>
          <div class="resume-banner-text">
            <strong class="resume-title">Resume Installation</strong>
            {#if interruptedInstall}
              <span class="resume-detail">"{interruptedInstall.collection_name}" &mdash; {interruptedInstall.completed_mods}/{interruptedInstall.total_mods} mods completed</span>
            {:else if interruptedWj}
              <span class="resume-detail">"{interruptedWj.modlist_name}" &mdash; interrupted</span>
            {/if}
          </div>
          <div class="resume-banner-actions">
            {#if interruptedInstall}
              <button class="btn btn-accent btn-sm resume-btn-cta" onclick={handleResumeInterrupted} disabled={resumingInstall}>
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                  <polygon points="5 3 19 12 5 21" />
                </svg>
                {resumingInstall ? "Resuming..." : "Resume Now"}
              </button>
            {/if}
            <button class="btn btn-ghost btn-sm" onclick={handleDismissInterrupted}>Dismiss</button>
          </div>
        </div>
      {/if}

      {#if $hashingProgress && !$hashingProgress.done}
        {@const pct = $hashingProgress.totalFiles > 0 ? Math.round(($hashingProgress.hashedFiles / $hashingProgress.totalFiles) * 100) : 0}
        <div class="hashing-banner" role="status">
          <div class="hashing-banner-content">
            <div class="hashing-banner-icon">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="hashing-spinner">
                <path d="M12 2v4M12 18v4M4.93 4.93l2.83 2.83M16.24 16.24l2.83 2.83M2 12h4M18 12h4M4.93 19.07l2.83-2.83M16.24 7.76l2.83-2.83" />
              </svg>
            </div>
            <div class="hashing-banner-text">
              <span class="hashing-banner-title">
                Verifying files&hellip; {pct}%
                {#if $hashingProgress.gameRunning}
                  <span class="hashing-throttled">(throttled &mdash; game running)</span>
                {/if}
              </span>
              <span class="hashing-banner-detail">
                You can start the game now. If you run into issues, the integrity checker will surface why.
                {#if $hashingProgress.modsTotal > 0}
                  &mdash; {$hashingProgress.modsDone}/{$hashingProgress.modsTotal} mods verified
                {/if}
              </span>
            </div>
            <div class="hashing-banner-actions">
              <button class="btn btn-ghost btn-sm" onclick={() => { cancelBackgroundHashing(); dismissHashingBanner(); }}>Dismiss</button>
            </div>
          </div>
          <div class="hashing-progress-track">
            <div class="hashing-progress-fill" style="width: {pct}%"></div>
          </div>
        </div>
      {/if}

      <slot />
    </main>
  </div>


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
    width: 300px;
    min-width: 300px;
    background: color-mix(in srgb, var(--bg-grouped) 75%, transparent);
    backdrop-filter: blur(24px) saturate(1.3);
    -webkit-backdrop-filter: blur(24px) saturate(1.3);
    border-radius: 14px;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    position: relative;
    transition: width 0.2s var(--ease), min-width 0.2s var(--ease);
    border: 0.5px solid rgba(255, 255, 255, 0.06);
  }

  .sidebar.resizing {
    transition: none;
    user-select: none;
  }

  .sidebar-resize-handle {
    position: absolute;
    top: 0;
    right: -4px;
    width: 8px;
    height: 100%;
    cursor: col-resize;
    z-index: 20;
  }

  .sidebar-resize-handle:hover,
  .sidebar-resize-handle:active {
    background: color-mix(in srgb, var(--accent) 30%, transparent);
    border-radius: 4px;
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
    backdrop-filter: var(--glass-blur);
    -webkit-backdrop-filter: var(--glass-blur);
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
    backdrop-filter: var(--glass-blur);
    -webkit-backdrop-filter: var(--glass-blur);
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
    overflow-y: auto;
    min-height: 0;
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
    flex-wrap: wrap;
    gap: var(--space-1);
    flex-shrink: 0;
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

  /* --- Version + Update Row --- */

  .version-update-row {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 24px;
  }

  .chat-toggle-btn {
    margin-left: auto;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    border: none;
    background: transparent;
    color: var(--text-quaternary);
    cursor: pointer;
    transition: color 0.2s ease, background 0.2s ease;
  }
  .chat-toggle-btn:hover {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  .chat-toggle-btn.chat-active {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 18%, transparent);
  }

  .sidebar-version {
    font-size: 10px;
    color: var(--text-quaternary);
    font-weight: 500;
    letter-spacing: 0.02em;
  }

  /* --- Update check button (refresh icon) --- */

  .update-check-btn {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    border: none;
    background: transparent;
    color: var(--text-quaternary);
    cursor: pointer;
    overflow: hidden;
    transition: color 0.2s ease, background 0.2s ease;
    flex-shrink: 0;
  }

  .update-check-btn:hover {
    background: var(--surface-hover);
    color: var(--text-secondary);
  }

  .update-check-btn.checking {
    color: var(--text-tertiary);
    cursor: default;
  }

  .update-check-btn.up-to-date {
    color: #34c759;
  }

  .update-check-btn.update-avail {
    color: var(--accent, #da8e35);
    animation: pulse-glow 2s ease-in-out infinite;
  }

  .update-check-btn.ready {
    color: #34c759;
    animation: pulse-glow-green 1.5s ease-in-out infinite;
  }

  @keyframes pulse-glow {
    0%, 100% { background: transparent; }
    50% { background: rgba(218, 142, 53, 0.1); }
  }

  @keyframes pulse-glow-green {
    0%, 100% { background: transparent; }
    50% { background: rgba(52, 199, 89, 0.1); }
  }

  /* --- Rainbow ripple (shown during check) --- */

  .ripple-ring {
    position: absolute;
    inset: 0;
    border-radius: 50%;
    pointer-events: none;
  }

  .ripple-ring::before,
  .ripple-ring::after {
    content: "";
    position: absolute;
    inset: -2px;
    border-radius: 50%;
    border: 2px solid transparent;
    animation: rainbow-spin 1.2s linear infinite;
    border-top-color: #ff6b6b;
    border-right-color: #ffd93d;
    border-bottom-color: #6bcb77;
    border-left-color: #4d96ff;
    opacity: 0.7;
  }

  .ripple-ring::after {
    inset: -6px;
    animation-duration: 1.8s;
    animation-direction: reverse;
    opacity: 0.3;
    border-top-color: #c084fc;
    border-right-color: #60a5fa;
    border-bottom-color: #34d399;
    border-left-color: #fbbf24;
  }

  @keyframes rainbow-spin {
    to { transform: rotate(360deg); }
  }

  /* --- Icon animations --- */

  .update-icon {
    position: relative;
    z-index: 1;
    flex-shrink: 0;
  }

  .update-icon.spin {
    animation: spin-refresh 1s linear infinite;
  }

  .check-icon {
    animation: check-pop 0.4s cubic-bezier(0.34, 1.56, 0.64, 1) both;
  }

  @keyframes spin-refresh {
    to { transform: rotate(360deg); }
  }

  @keyframes check-pop {
    0% { transform: scale(0); opacity: 0; }
    60% { transform: scale(1.2); }
    100% { transform: scale(1); opacity: 1; }
  }

  .spinner-update {
    width: 12px;
    height: 12px;
    border: 2px solid rgba(218, 142, 53, 0.2);
    border-top-color: var(--accent, #da8e35);
    border-radius: 50%;
    animation: spin-refresh 0.8s linear infinite;
    position: relative;
    z-index: 1;
  }

  /* --- Status pills --- */

  .update-pill {
    font-size: 9px;
    font-weight: 700;
    padding: 2px 7px;
    border-radius: 100px;
    letter-spacing: 0.02em;
    border: none;
    cursor: pointer;
    white-space: nowrap;
    transition: filter 0.15s ease;
  }

  .update-pill:hover {
    filter: brightness(1.15);
  }

  .update-pill.available {
    background: rgba(218, 142, 53, 0.15);
    color: #da8e35;
  }

  .update-pill.ready {
    background: rgba(52, 199, 89, 0.15);
    color: #34c759;
    animation: glass-glow-pulse 2s ease-in-out infinite;
  }

  .update-pill.downloading {
    background: rgba(255, 255, 255, 0.06);
    color: var(--text-tertiary);
    cursor: default;
  }

  .update-pill.downloading:hover {
    filter: none;
  }

  /* --- Update banner --- */

  .update-banner {
    background: var(--surface-glass, rgba(255, 255, 255, 0.04));
    border: 1px solid var(--accent-subtle, rgba(217, 143, 64, 0.2));
    border-radius: var(--radius-md, 8px);
    margin: var(--space-3, 12px) var(--space-4, 16px) var(--space-4, 16px);
    padding: var(--space-3, 12px) var(--space-4, 16px);
    backdrop-filter: var(--glass-blur-light);
    box-shadow: var(--glass-refraction), var(--glass-edge-shadow);
    animation: glass-banner-enter 350ms var(--ease-out);
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

  .update-ready-btn {
    animation: glass-scale-pop 400ms var(--ease-spring);
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

  .update-changelog {
    margin-top: var(--space-2, 8px);
    max-height: 300px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
  }

  .changelog-entry {
    padding: var(--space-2, 8px);
    background: rgba(255, 255, 255, 0.03);
    border-radius: var(--radius-sm, 4px);
    border: 1px solid var(--separator);
  }

  .changelog-version-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 4px;
    font-size: 12px;
    color: var(--text-primary);
  }

  .changelog-date {
    font-size: 11px;
    color: var(--text-tertiary);
    font-weight: 400;
  }

  .changelog-body {
    font-size: 12px;
    line-height: 1.5;
    color: var(--text-secondary);
    white-space: pre-wrap;
    word-break: break-word;
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

  .resume-banner-prominent {
    padding: 14px 20px;
    background: linear-gradient(135deg, rgba(0, 122, 255, 0.10), rgba(0, 122, 255, 0.04));
    border: 1px solid rgba(0, 122, 255, 0.35);
    border-radius: var(--radius-lg, 12px);
    animation: resume-fade-in 0.3s ease-out;
    box-shadow: 0 0 20px rgba(0, 122, 255, 0.08);
  }

  @keyframes resume-fade-in {
    from { opacity: 0; transform: translateY(-8px); }
    to { opacity: 1; transform: translateY(0); }
  }

  .resume-icon-pulse {
    color: var(--system-accent, #007aff);
    animation: icon-glow 2s ease-in-out infinite;
  }

  @keyframes icon-glow {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.6; }
  }

  .resume-title {
    font-size: 14px;
  }

  .resume-detail {
    display: block;
    font-size: 12px;
    color: var(--text-secondary);
    margin-top: 2px;
  }

  .resume-btn-cta {
    display: inline-flex;
    align-items: center;
    gap: 6px;
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
    backdrop-filter: var(--glass-blur);
    -webkit-backdrop-filter: var(--glass-blur);
  }

  /* --- Content area --- */

  .content {
    flex: 1;
    overflow-y: auto;
    padding: 0 var(--space-6) var(--space-6);
    position: relative;
  }

  @media (max-width: 800px) {
    .content {
      padding: 0 var(--space-3) var(--space-3);
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
    box-shadow: var(--glass-refraction), var(--glass-edge-shadow), var(--shadow-lg);
    animation: toastIn var(--duration-slow) var(--ease-out);
    backdrop-filter: var(--glass-blur-heavy);
    -webkit-backdrop-filter: var(--glass-blur-heavy);
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

  .toast-nxm {
    background: rgba(10, 132, 255, 0.18);
    border: 1px solid rgba(10, 132, 255, 0.3);
    color: var(--text-primary);
    max-width: 480px;
    flex-wrap: wrap;
  }

  .toast-nxm .toast-text {
    color: var(--text-primary);
    line-height: 1.4;
  }

  .toast-nxm .toast-text strong {
    color: var(--accent);
  }

  .toast-version {
    font-size: 11px;
    color: var(--text-tertiary);
    margin-left: 2px;
  }

  .toast-actions {
    display: flex;
    gap: var(--space-2);
    flex-shrink: 0;
  }

  .toast-action-btn {
    padding: 4px 12px;
    border-radius: var(--radius-sm);
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
  }

  .toast-action-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .toast-install-btn {
    background: var(--system-accent);
    color: var(--system-accent-on);
  }

  .toast-install-btn:hover:not(:disabled) {
    background: var(--system-accent-hover);
  }

  .toast-cancel-btn {
    background: transparent;
    color: var(--text-secondary);
    border: 1px solid var(--separator);
  }

  .toast-cancel-btn:hover:not(:disabled) {
    background: var(--surface-hover);
    color: var(--text-primary);
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
    display: flex;
    align-items: center;
    gap: 2px;
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
    background: color-mix(in srgb, var(--bg-elevated) 72%, transparent);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: var(--radius);
    box-shadow: var(--glass-refraction),
                var(--glass-edge-shadow),
                var(--shadow-lg);
    z-index: 200;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    animation: dropdownIn var(--duration-fast) var(--ease-out);
    backdrop-filter: var(--glass-blur-heavy);
    -webkit-backdrop-filter: var(--glass-blur-heavy);
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

  /* ---- Sidebar-docked install/uninstall status bar ---- */
  .sidebar-status-bar {
    padding: 8px var(--space-3);
    border-top: 1px solid var(--separator);
    flex-shrink: 0;
  }

  .sidebar-status-bar-uninstall {
    border-top-color: rgba(239, 68, 68, 0.3);
  }

  .sidebar-status-btn {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 0;
    margin-bottom: 6px;
    background: none;
    border: none;
    cursor: pointer;
    text-align: left;
    color: inherit;
  }

  .sidebar-status-btn:hover .status-chevron {
    color: var(--text-secondary);
    transform: translateX(2px);
  }

  .sidebar-status-btn:hover .status-collection {
    color: var(--accent);
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

  .status-spinner-sm {
    width: 12px;
    height: 12px;
    border-width: 1.5px;
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
    gap: 1px;
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
    transition: color var(--duration-fast) var(--ease);
  }

  .status-detail {
    font-size: 10px;
    color: var(--text-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .status-percent {
    font-size: 10px;
    font-weight: 700;
    font-family: var(--font-mono);
    color: var(--system-accent);
    flex-shrink: 0;
  }

  .status-chevron {
    flex-shrink: 0;
    color: var(--text-tertiary);
    transition: transform var(--duration-fast) var(--ease), color var(--duration-fast) var(--ease);
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

  .status-spinner-red {
    border-top-color: #ef4444;
  }

  .status-progress-fill-red {
    background: #ef4444;
  }

  .sidebar.collapsed .sidebar-status-bar {
    padding: 6px var(--space-1);
  }

  .sidebar.collapsed .sidebar-status-btn .status-text,
  .sidebar.collapsed .sidebar-status-btn .status-chevron {
    display: none;
  }

  .sidebar.collapsed .sidebar-status-btn {
    justify-content: center;
  }

  /* Controller mode: larger touch targets for Steam Deck */
  :global(.controller-mode) button,
  :global(.controller-mode) .btn,
  :global(.controller-mode) .nav-item {
    min-height: 44px;
    font-size: 15px;
  }

  :global(.controller-mode) .nav-item {
    padding: var(--space-3) var(--space-4);
  }

  :global(.controller-mode) .sidebar-footer button,
  :global(.controller-mode) .sidebar-footer .sidebar-gh-btn,
  :global(.controller-mode) .sidebar-footer .sidebar-collapse-btn,
  :global(.controller-mode) .sidebar-footer .queue-btn {
    min-height: unset;
    font-size: unset;
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
    backdrop-filter: var(--glass-blur-light);
    animation: shortcutsFadeIn 150ms ease-out;
  }

  .shortcuts-card {
    background: color-mix(in srgb, var(--bg-secondary) 75%, transparent);
    backdrop-filter: blur(40px) saturate(1.5);
    -webkit-backdrop-filter: blur(40px) saturate(1.5);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: var(--radius);
    padding: var(--space-6);
    max-width: 380px;
    width: 90vw;
    box-shadow: var(--glass-refraction),
                var(--glass-edge-shadow),
                var(--shadow-lg);
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

  /* Background hashing banner */
  .hashing-banner {
    margin-bottom: var(--space-3, 12px);
    background: linear-gradient(135deg, rgba(52, 199, 89, 0.08), rgba(52, 199, 89, 0.03));
    border: 1px solid rgba(52, 199, 89, 0.25);
    border-radius: var(--radius-md, 8px);
    overflow: hidden;
    animation: resume-fade-in 0.3s ease-out;
  }
  .hashing-banner-content {
    display: flex;
    align-items: center;
    gap: var(--space-3, 12px);
    padding: 10px 16px 8px;
  }
  .hashing-banner-icon {
    color: #34c759;
    flex-shrink: 0;
  }
  .hashing-spinner {
    animation: hashing-spin 1.5s linear infinite;
  }
  @keyframes hashing-spin {
    from { transform: rotate(0deg); }
    to { transform: rotate(360deg); }
  }
  .hashing-banner-text {
    flex: 1;
    min-width: 0;
  }
  .hashing-banner-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
  }
  .hashing-throttled {
    font-weight: 400;
    color: var(--text-tertiary, #888);
    font-size: 12px;
  }
  .hashing-banner-detail {
    display: block;
    font-size: 12px;
    color: var(--text-secondary);
    margin-top: 2px;
    line-height: 1.4;
  }
  .hashing-banner-actions {
    flex-shrink: 0;
  }
  .hashing-progress-track {
    height: 3px;
    background: rgba(52, 199, 89, 0.12);
  }
  .hashing-progress-fill {
    height: 100%;
    background: #34c759;
    transition: width 0.4s ease-out;
    border-radius: 0 2px 2px 0;
  }
</style>
