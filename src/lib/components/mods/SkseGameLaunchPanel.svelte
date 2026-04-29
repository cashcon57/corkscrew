<script lang="ts">
  import { onDestroy } from "svelte";
  import { get } from "svelte/store";
  import { open } from "@tauri-apps/plugin-dialog";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import {
    launchGame,
    checkSkse,
    getSkseDownloadUrl,
    installSkseFromArchive,
    installSkseAuto,
    setSksePreference,
    checkSkyrimVersion,
    downgradeSkyrim,
    fixSkyrimDisplay,
    getGameLockStatus,
    forceUnlockGame,
    setConfigValue,
  } from "$lib/api";
  import {
    selectedGame,
    showError,
    showSuccess,
    skseStatus,
    gameLock,
    gameLockOverridden,
  } from "$lib/stores";
  import type { DetectedGame, SkseStatus, DowngradeStatus } from "$lib/types";
  import { wineCtx } from "$lib/types";

  let {
    game,
    launching = $bindable(false),
    disableGameFixes = $bindable(false),
    onSkseStatusChange,
  }: {
    game: DetectedGame;
    launching: boolean;
    disableGameFixes: boolean;
    onSkseStatusChange: (status: SkseStatus | null) => void;
  } = $props();

  let skse = $state<SkseStatus | null>(null);
  let showSksePrompt = $state(false);
  let showSkseMenu = $state(false);
  let showSkseInstallPrompt = $state(false);
  let installingSkse = $state(false);
  let downgradeStatus = $state<DowngradeStatus | null>(null);
  let downgrading = $state(false);
  let showDowngradeBanner = $state(false);
  let fixingDisplay = $state(false);
  let gameLockPollInterval: ReturnType<typeof setInterval> | null = null;
  let gameLockPollFailCount = 0;

  // SKSE & version detection
  let skseCheckGeneration = 0;

  $effect(() => {
    const g = game;
    const gen = ++skseCheckGeneration;
    if (g && g.game_id === "skyrimse") {
      checkSkseStatus(g, gen);
      checkVersionStatus(g, gen);
    } else {
      skse = null;
      showSksePrompt = false;
      downgradeStatus = null;
      showDowngradeBanner = false;
      onSkseStatusChange(null);
    }
  });

  // Re-check game lock when game changes
  $effect(() => {
    const g = game;
    if (g) {
      getGameLockStatus(g.game_id, (wineCtx(g)?.bottle_name ?? "")).then(lock => {
        if (!get(gameLockOverridden)) {
          gameLock.set(lock);
          if (lock) {
            startGameLockPolling(g.game_id, (wineCtx(g)?.bottle_name ?? ""));
          } else {
            stopGameLockPolling();
          }
        }
      }).catch((err) => console.error('Game lock status check failed:', err));
    } else {
      gameLock.set(null);
      stopGameLockPolling();
    }
    return () => {
      stopGameLockPolling();
    };
  });

  onDestroy(() => {
    stopGameLockPolling();
  });

  async function checkSkseStatus(g: DetectedGame, gen: number) {
    try {
      const status = await checkSkse(g.game_id, (wineCtx(g)?.bottle_name ?? ""));
      if (gen !== skseCheckGeneration) return;
      skse = status;
      skseStatus.set(skse);
      onSkseStatusChange(skse);
      if (!skse.installed) {
        const dismissed = localStorage.getItem(`skse_dismissed:${g.game_id}:${(wineCtx(g)?.bottle_name ?? "")}`);
        if (!dismissed) showSksePrompt = true;
      }
    } catch {
      // Non-critical
    }
  }

  async function handlePlay() {
    const wantsSkse = !!(skse?.use_skse && game.game_id === "skyrimse");
    if (wantsSkse && !skse?.installed) {
      showSkseInstallPrompt = true;
      return;
    }
    doLaunch(wantsSkse);
  }

  async function doLaunch(useSkse: boolean) {
    launching = true;
    try {
      const result = await launchGame(game.game_id, (wineCtx(game)?.bottle_name ?? ""), useSkse);
      if (result.success) {
        showSuccess(`Launched ${game.display_name}${useSkse ? " via SKSE" : ""} — Wine cursor fix applied`);
        if (result.warning) {
          showError(`SKSE warning: ${result.warning}`);
        }
        gameLockOverridden.set(false);
        startGameLockPolling(game.game_id, (wineCtx(game)?.bottle_name ?? ""));
      }
    } catch (e: unknown) {
      showError(`Failed to launch: ${e}`);
    } finally {
      launching = false;
    }
  }

  function startGameLockPolling(gameId: string, bottleName: string) {
    stopGameLockPolling();
    gameLockPollFailCount = 0;
    pollGameLock(gameId, bottleName);
    gameLockPollInterval = setInterval(() => pollGameLock(gameId, bottleName), 5000);
  }

  function stopGameLockPolling() {
    if (gameLockPollInterval) {
      clearInterval(gameLockPollInterval);
      gameLockPollInterval = null;
    }
  }

  async function pollGameLock(gameId: string, bottleName: string) {
    try {
      if (get(gameLockOverridden)) return;
      const lock = await getGameLockStatus(gameId, bottleName);
      if (get(gameLockOverridden)) return;
      gameLockPollFailCount = 0;
      gameLock.set(lock);
      if (!lock) {
        stopGameLockPolling();
        gameLockOverridden.set(false);
      }
    } catch (e) {
      gameLockPollFailCount++;
      if (gameLockPollFailCount >= 3) {
        console.warn('Game lock poll failed 3 times, clearing lock:', e);
        gameLock.set(null);
        stopGameLockPolling();
        gameLockOverridden.set(false);
        gameLockPollFailCount = 0;
      }
    }
  }

  async function handleForceUnlock() {
    await forceUnlockGame(game.game_id, (wineCtx(game)?.bottle_name ?? ""));
    gameLock.set(null);
    gameLockOverridden.set(true);
    stopGameLockPolling();
  }

  async function toggleGameFixes() {
    disableGameFixes = !disableGameFixes;
    try {
      await setConfigValue("disable_game_fixes", disableGameFixes ? "true" : "false");
    } catch { /* best-effort */ }
  }

  async function handleFixDisplay() {
    fixingDisplay = true;
    try {
      const result = await fixSkyrimDisplay((wineCtx(game)?.bottle_name ?? ""));
      if (result.fixed) {
        showSuccess(`Display fixed: ${result.applied.width}x${result.applied.height} fullscreen — game will open in exclusive fullscreen`);
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
    try {
      const selected = await open({
        title: "Select SKSE Archive (.7z or .zip)",
        filters: [{ name: "Archives", extensions: ["7z", "zip"] }],
      });
      if (!selected) return;

      const archivePath = typeof selected === "string" ? selected : String(selected);
      installingSkse = true;
      skse = await installSkseFromArchive(game.game_id, (wineCtx(game)?.bottle_name ?? ""), archivePath);
      skseStatus.set(skse);
      onSkseStatusChange(skse);
      showSksePrompt = false;
      showSuccess("SKSE installed successfully");
    } catch (e: unknown) {
      showError(`SKSE installation failed: ${e}`);
    } finally {
      installingSkse = false;
    }
  }

  async function handleAutoInstallSkse() {
    try {
      installingSkse = true;
      skse = await installSkseAuto(game.game_id, (wineCtx(game)?.bottle_name ?? ""));
      skseStatus.set(skse);
      onSkseStatusChange(skse);
      showSksePrompt = false;
      showSuccess("SKSE auto-installed successfully");
    } catch (e: unknown) {
      showError(`SKSE auto-install failed: ${e}`);
    } finally {
      installingSkse = false;
    }
  }

  async function handleInstallSkseAndLaunch() {
    try {
      installingSkse = true;
      showSkseInstallPrompt = false;
      skse = await installSkseAuto(game.game_id, (wineCtx(game)?.bottle_name ?? ""));
      skseStatus.set(skse);
      onSkseStatusChange(skse);
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
    try {
      const selected = await open({
        title: "Select SKSE Archive (.7z or .zip)",
        filters: [{ name: "Archives", extensions: ["7z", "zip"] }],
      });
      if (!selected) return;

      const archivePath = typeof selected === "string" ? selected : String(selected);
      installingSkse = true;
      showSkseInstallPrompt = false;
      skse = await installSkseFromArchive(game.game_id, (wineCtx(game)?.bottle_name ?? ""), archivePath);
      skseStatus.set(skse);
      onSkseStatusChange(skse);
      showSksePrompt = false;
      showSuccess("SKSE installed — launching game");
      doLaunch(true);
    } catch (e: unknown) {
      showError(`SKSE installation failed: ${e}`);
    } finally {
      installingSkse = false;
    }
  }

  async function checkVersionStatus(g: DetectedGame, gen: number) {
    try {
      const status = await checkSkyrimVersion(g.game_id, (wineCtx(g)?.bottle_name ?? ""));
      if (gen !== skseCheckGeneration) return;
      downgradeStatus = status;
      if (!status.is_downgraded) {
        const dismissed = localStorage.getItem(`downgrade_dismissed:${g.game_id}:${(wineCtx(g)?.bottle_name ?? "")}`);
        if (!dismissed) showDowngradeBanner = true;
      }
    } catch {
      // Non-critical
    }
  }

  async function handleDowngrade() {
    downgrading = true;
    try {
      const status = await downgradeSkyrim(game.game_id, (wineCtx(game)?.bottle_name ?? ""), "full");
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
    localStorage.setItem(`downgrade_dismissed:${game.game_id}:${(wineCtx(game)?.bottle_name ?? "")}`, "true");
    showDowngradeBanner = false;
  }

  function dismissSksePrompt() {
    localStorage.setItem(`skse_dismissed:${game.game_id}:${(wineCtx(game)?.bottle_name ?? "")}`, "true");
    showSksePrompt = false;
  }

  async function toggleSksePreference() {
    if (!skse) return;
    const newValue = !skse.use_skse;
    try {
      await setSksePreference(game.game_id, (wineCtx(game)?.bottle_name ?? ""), newValue);
      skse = { ...skse, use_skse: newValue };
      skseStatus.set(skse);
      onSkseStatusChange(skse);
    } catch (e: unknown) {
      showError(`Failed to update SKSE preference: ${e}`);
    }
    showSkseMenu = false;
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->

<!-- Fix Display button (Skyrim only, rendered in banner actions area) -->
{#if game.game_id === "skyrimse"}
  <button
    class="btn btn-ghost"
    onclick={handleFixDisplay}
    disabled={fixingDisplay}
    title="Fix display: native resolution, exclusive fullscreen at native resolution"
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

<!-- Play button group -->
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
      Play{#if skse?.use_skse && game.game_id === "skyrimse"} (SKSE){/if}
    {/if}
  </button>
  {#if game.game_id === "skyrimse"}
    <button
      class="btn btn-play-dropdown"
      onclick={(e) => { e.stopPropagation(); showSkseMenu = !showSkseMenu; showSkseInstallPrompt = false; }}
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

<!-- Game Lock Banner -->
{#if $gameLock && !$gameLockOverridden}
  <div class="game-lock-banner">
    <div class="skse-banner-icon game-lock-icon">
      <svg width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
        <rect x="4" y="9" width="12" height="9" rx="1.5" />
        <path d="M7 9V6a3 3 0 0 1 6 0v3" />
      </svg>
    </div>
    <div class="skse-banner-content">
      <p class="skse-banner-title">Game Is Running</p>
      <p class="skse-banner-text">
        Mod changes are locked while the game is running (pid {$gameLock.pid}).
        Close the game to make changes, or unlock to override.
      </p>
    </div>
    <div class="skse-banner-actions">
      <button class="btn btn-ghost-danger btn-sm" onclick={handleForceUnlock}>
        Unlock Anyway
      </button>
    </div>
  </div>
{/if}

<!-- SKSE Not Installed Banner -->
{#if showSksePrompt && game.game_id === "skyrimse"}
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

<!-- Downgrade Banner -->
{#if showDowngradeBanner && game.game_id === "skyrimse" && downgradeStatus && !downgradeStatus.is_downgraded}
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
