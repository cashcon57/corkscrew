<script lang="ts">
  import { selectedGame, showError, showSuccess } from "$lib/stores";
  import type { CachedVersion, DetectedGame } from "$lib/types";
  import { wineCtx } from "$lib/types";
  import {
    swapGameVersion,
    startDepotDownload,
    checkDepotReady,
    applyDowngrade,
    getDepotDownloadCommand,
    ddStatus,
    ddInstall,
    ddListManifests,
    ddDownloadDepot,
    ddCheckPartialDownload,
    ddApplyDepot,
  } from "$lib/api";
  import { listen } from "@tauri-apps/api/event";
  import { onDestroy } from "svelte";
  import SteamAuthDialog from "$lib/components/SteamAuthDialog.svelte";

  interface Props {
    game: DetectedGame;
    versionInfo: { expected: string[]; detected: string };
    versionCache: CachedVersion[];
    onproceed: () => void;
    oncancel: () => void;
  }

  let { game, versionInfo, versionCache, onproceed, oncancel }: Props = $props();

  let versionSwapping = $state(false);
  let depotDownloading = $state(false);
  let depotPollTimer = $state<ReturnType<typeof setInterval> | null>(null);
  let showSteamAuth = $state(false);
  let depotDowngradePhase = $state("");

  const gameName = $derived(game.display_name ?? "this game");
  const isSkyrim = $derived(game.game_id === "skyrimse");
  const targetsSE = $derived(isSkyrim && versionInfo.expected.some((v: string) => v.startsWith("1.5.")));
  const matchingCached = $derived(
    isSkyrim
      ? versionCache.filter(cv => targetsSE ? cv.version.startsWith("1.5.") : cv.version.startsWith("1.6."))
      : versionCache.filter(cv => versionInfo.expected.some((v: string) => cv.version === v || cv.version.startsWith(v)))
  );

  function handleCancel() {
    if (depotPollTimer) { clearInterval(depotPollTimer); depotPollTimer = null; }
    depotDownloading = false;
    oncancel();
  }

  async function handleSwapVersion(version: string) {
    versionSwapping = true;
    try {
      await swapGameVersion(game.game_id, (wineCtx(game)?.bottle_name ?? ""), version);
      showSuccess(`Switched to v${version}. Nice.`);
      onproceed();
    } catch (e) {
      showError(`Version swap failed: ${e}`);
    } finally {
      versionSwapping = false;
    }
  }

  async function handleSkyrimDepotDownload() {
    depotDownloading = true;
    try {
      const automated = await startDepotDownload(game.game_id);
      if (!automated) {
        try {
          const info = await getDepotDownloadCommand(game.game_id, (wineCtx(game)?.bottle_name ?? ""));
          await navigator.clipboard.writeText(info.command);
          showSuccess("Command copied! Paste it in the Steam console that opened.");
        } catch { /* ignore clipboard errors */ }
      }
      depotPollTimer = setInterval(async () => {
        try {
          const result = await checkDepotReady(game.game_id, (wineCtx(game)?.bottle_name ?? ""));
          if (result) {
            if (depotPollTimer) clearInterval(depotPollTimer);
            depotPollTimer = null;
            const status = await applyDowngrade(game.game_id, (wineCtx(game)?.bottle_name ?? ""));
            depotDownloading = false;
            showSuccess(`Switched to v${status.current_version}. Let's go.`);
            onproceed();
          }
        } catch { /* keep polling */ }
      }, 3000);
    } catch (e) {
      depotDownloading = false;
      showError(`Download failed: ${e}`);
    }
  }

  const steamAppIds: Record<string, { app: number; depot: number }> = {
    hogwartslegacy: { app: 990080, depot: 990081 },
    skyrimse: { app: 489830, depot: 489833 },
    skyrim: { app: 72850, depot: 72851 },
    fallout4: { app: 377160, depot: 377162 },
    falloutnv: { app: 22380, depot: 22381 },
    fallout3: { app: 22300, depot: 22301 },
    oblivion: { app: 22330, depot: 22331 },
    morrowind: { app: 22320, depot: 22321 },
    starfield: { app: 1716740, depot: 1716741 },
    baldursgate3: { app: 1086940, depot: 1086941 },
    cyberpunk2077: { app: 1091500, depot: 1091501 },
    witcher3: { app: 292030, depot: 292031 },
    eldenring: { app: 1245620, depot: 1245621 },
    stardewvalley: { app: 413150, depot: 413151 },
  };

  async function handleDDDepotDownload() {
    depotDownloading = true;
    try {
      depotDowngradePhase = "Setting up DepotDownloader...";

      const status = await ddStatus();
      if (!status.installed) {
        depotDowngradePhase = "Downloading DepotDownloader...";
        await ddInstall();
      }

      const authStatus = await ddStatus();
      if (authStatus.auth_state !== "ready") {
        depotDownloading = false;
        depotDowngradePhase = "";
        showSteamAuth = true;
        return;
      }

      const ids = steamAppIds[game.game_id];
      if (!ids) {
        depotDownloading = false;
        depotDowngradePhase = "";
        showError("Game not yet supported for automated downgrade. Check SteamDB manually.");
        return;
      }

      const partial = await ddCheckPartialDownload(ids.app, ids.depot);
      if (partial && partial.file_count > 0) {
        depotDowngradePhase = `Resuming download (${partial.file_count} files already downloaded)...`;
      } else {
        depotDowngradePhase = "Listing available versions...";
      }

      const manifests = await ddListManifests(ids.app, ids.depot);
      if (manifests.length === 0) {
        depotDownloading = false;
        depotDowngradePhase = "";
        showError("No manifests found. You may need to authenticate with Steam first.");
        showSteamAuth = true;
        return;
      }

      const targetManifest = manifests[0];
      depotDowngradePhase = "Downloading... 0%";

      const unlisten = await listen<{ phase: string; detail: string; percent: number | null }>(
        "dd-download-progress",
        (event) => {
          const { detail, percent } = event.payload;
          if (percent != null) {
            depotDowngradePhase = `Downloading... ${percent.toFixed(1)}%`;
          } else if (detail) {
            depotDowngradePhase = detail;
          }
        }
      );

      try {
        const depotDir = await ddDownloadDepot(
          ids.app, ids.depot, targetManifest.manifest_id, game.game_id
        );

        depotDowngradePhase = "Applying downgrade...";
        const filesCopied = await ddApplyDepot(
          game.game_id, (wineCtx(game)?.bottle_name ?? ""), depotDir
        );

        depotDownloading = false;
        depotDowngradePhase = "";
        showSuccess(`Downgraded successfully (${filesCopied} files). Let's go.`);
        onproceed();
      } finally {
        unlisten();
      }
    } catch (e) {
      depotDownloading = false;
      showError(`Downgrade failed: ${e}`);
    }
  }

  onDestroy(() => {
    if (depotPollTimer) { clearInterval(depotPollTimer); depotPollTimer = null; }
  });
</script>

{#if showSteamAuth}
  <SteamAuthDialog
    onauth={() => {
      showSteamAuth = false;
      showSuccess("Steam authentication successful. Retrying downgrade...");
    }}
    oncancel={() => { showSteamAuth = false; }}
  />
{/if}

<div class="modal-overlay" onclick={handleCancel} role="presentation">
  <div class="cleanup-modal" onclick={(e) => e.stopPropagation()} role="dialog" aria-label="Version mismatch warning">
    <div class="cleanup-header">
      <h3 class="cleanup-title">Heads Up &mdash; Wrong Game Version</h3>
      <button class="cleanup-close" onclick={handleCancel}>&times;</button>
    </div>

    <div class="cleanup-body">
      <div class="cleanup-summary">
        <p class="cleanup-info">
          This collection was built for
          <strong>{gameName} v{versionInfo.expected.join(' / ')}</strong>{isSkyrim && targetsSE ? " (SE)" : isSkyrim ? " (AE)" : ""},
          but you're running
          <strong>v{versionInfo.detected}</strong>{isSkyrim ? (versionInfo.detected.startsWith("1.5.") ? " (SE)" : " (AE)") : ""}.
        </p>
        <p class="cleanup-info version-warning-detail">
          {#if matchingCached.length > 0}
            Good news &mdash; you have a compatible version cached. One click and you're golden.
          {:else if depotDownloading}
            Downloading the correct version from Steam... grab a coffee, this might take a minute.
          {:else if isSkyrim}
            You can download the correct version from Steam, or roll the dice and install anyway.
            Fair warning: mismatched versions usually end in tears (and crashes).
          {:else}
            Installing on a different version than what the collection author tested is... brave.
            It might work. It probably won't. Your call.
          {/if}
        </p>
        {#if depotDownloading}
          <div class="depot-download-status">
            <div class="spinner-sm"></div>
            <span>Downloading via Steam depot... this may take several minutes.</span>
          </div>
        {/if}
      </div>
    </div>

    <div class="cleanup-actions">
      <button class="btn btn-ghost" disabled={depotDownloading} onclick={handleCancel}>Cancel</button>

      {#each matchingCached as matchingVersion}
        <button class="btn btn-accent" disabled={versionSwapping} onclick={() => handleSwapVersion(matchingVersion.version)}>
          {versionSwapping ? "Switching..." : `Switch to v${matchingVersion.version} (Recommended)`}
        </button>
      {/each}

      {#if matchingCached.length === 0 && !depotDownloading}
        {#if isSkyrim}
          <button class="btn btn-accent" onclick={handleSkyrimDepotDownload}>
            Download & Switch to v{versionInfo.expected[0]} (Recommended)
          </button>
        {:else}
          <button class="btn btn-accent" disabled={depotDownloading} onclick={handleDDDepotDownload}>
            {depotDownloading
              ? (depotDowngradePhase || "Downloading...")
              : `Downgrade to v${versionInfo.expected[0]} (Recommended)`}
          </button>
        {/if}
      {/if}

      <button class="btn btn-ghost version-yolo-btn" disabled={depotDownloading} onclick={onproceed}>
        Install Anyway (Good Luck)
      </button>
    </div>
  </div>
</div>

<style>
  .modal-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
  }

  .version-warning-detail {
    margin-top: 0.5rem;
    line-height: 1.5;
  }

  .depot-download-status {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin: 0.75rem 0;
    padding: 0.75rem;
    background: var(--bg-secondary);
    border-radius: 6px;
    font-size: 0.9rem;
  }

  .version-yolo-btn {
    color: var(--text-tertiary) !important;
    font-size: 12px !important;
  }

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

  .cleanup-info :global(strong) {
    color: var(--text-primary);
  }

  .cleanup-actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 20px;
    border-top: 1px solid var(--border-primary);
    flex-wrap: wrap;
  }
</style>
