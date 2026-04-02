<script lang="ts">
  import { onDestroy } from "svelte";
  import { goto } from "$app/navigation";
  import { selectedGame, showError, showSuccess, collectionInstallStatus, modStateVersion } from "$lib/stores";
  import type { CollectionInfo, CollectionManifest, CollectionMod, CollectionModEntry } from "$lib/types";
  import type { RequiredTool, CleanReport, CleanOptions, DlcStatus, CachedVersion } from "$lib/types";
  import type { DetectedGame } from "$lib/types";
  import {
    getAllGames,
    installCollection,
    detectCollectionTools,
    checkSkyrimVersion,
    listGameVersions,
    swapGameVersion,
    startDepotDownload,
    checkDepotReady,
    applyDowngrade,
    getDepotDownloadCommand,
    scanGameDirectory,
    cleanGameDirectory,
    hasGameSnapshot,
    checkDlcStatus,
    launchGame,
    getGameVersion,
    ddStatus,
    ddInstall,
    ddListManifests,
    ddDownloadDepot,
    ddCheckPartialDownload,
    ddDeletePartialDownload,
  } from "$lib/api";
  import { startInstallTracking } from "$lib/installService";
  import { listen } from "@tauri-apps/api/event";
  import RequiredToolsPrompt from "$lib/components/RequiredToolsPrompt.svelte";
  import SteamAuthDialog from "$lib/components/SteamAuthDialog.svelte";

  interface Props {
    game: DetectedGame;
    oncomplete?: (result: InstallResult) => void;
    oncancel?: () => void;
  }

  let { game, oncomplete, oncancel }: Props = $props();

  // Install result type
  type InstallResult = {
    installed: number;
    already_installed: number;
    skipped: number;
    failed: number;
    details: { name: string; status: string; error: string | null; url: string | null; instructions: string | null }[];
  };

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

  // ---- Install workflow state ----
  let installing = $state(false);
  let activeCollection = $state<CollectionInfo | null>(null);

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
  let showSteamAuth = $state(false);
  let depotDowngradePhase = $state("");

  // DLC Detection
  let showDlcWarning = $state(false);
  let dlcStatus = $state<DlcStatus | null>(null);
  let dlcLaunching = $state(false);

  // Optional mod picker
  let showOptionalPicker = $state(false);
  let optionalPickerManifest = $state<(CollectionManifest & Record<string, unknown>) | null>(null);
  type OptionalModChoice = "install" | "install_disabled" | "skip";
  let optionalChoices = $state<Map<number, OptionalModChoice>>(new Map());

  // Pre-computed counts for optional picker
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

  function formatSize(bytes: number): string {
    if (bytes >= 1073741824) return `${(bytes / 1073741824).toFixed(1)} GB`;
    if (bytes >= 1048576) return `${(bytes / 1048576).toFixed(0)} MB`;
    if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KB`;
    return `${bytes} B`;
  }

  /**
   * Public entry point: starts the install workflow for a collection.
   * Called by the parent when the user clicks "Install Collection".
   */
  export function startInstall(
    collection: CollectionInfo,
    mods: CollectionMod[],
    gameVersions: string[],
    renderedInstructions?: string,
  ) {
    activeCollection = collection;
    handleInstallCollection(collection, mods, gameVersions, renderedInstructions ?? "");
  }

  async function handleInstallCollection(
    collection: CollectionInfo,
    mods: CollectionMod[],
    gameVersions: string[],
    renderedInstructions: string,
  ) {
    // Auto-switch to the correct game if the collection targets a different game
    const collectionDomain = collection.game_domain;
    const currentGame = $selectedGame;
    if (currentGame) {
      const currentSlug = gameSlugMap[currentGame.game_id] ?? currentGame.game_id;
      if (collectionDomain && currentSlug !== collectionDomain && currentGame.nexus_slug !== collectionDomain) {
        try {
          const allGames = await getAllGames();
          const targetGame = allGames.find(
            (g: { nexus_slug: string; game_id: string }) =>
              g.nexus_slug === collectionDomain ||
              (gameSlugMap[g.game_id] ?? g.game_id) === collectionDomain
          );
          if (targetGame) {
            selectedGame.set(targetGame);
            showSuccess(`Switched to ${targetGame.display_name} for collection install`);
          }
        } catch (e) {
          console.error("Failed to auto-switch game:", e);
        }
      }
    }

    if (!$selectedGame) return;

    // Build manifest
    const manifest = {
      name: collection.name,
      author: collection.author,
      description: collection.summary,
      game_domain: collection.game_domain,
      mods: mods.map((m) => ({
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
      installInstructions: renderedInstructions || null,
      slug: collection.slug ?? null,
      image_url: collection.image_url ?? null,
      revision: collection.latest_revision ?? null,
      gameVersions,
    };

    // Check for optional mods
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

    await checkGameVersionAndProceed(manifest);
  }

  async function checkGameVersionAndProceed(manifest: CollectionManifest & Record<string, unknown>) {
    if (!$selectedGame) return;

    const versions = manifest.gameVersions ?? [];
    if (versions.length === 0) {
      await checkPreInstallCleanup(manifest);
      return;
    }

    try {
      let detected: string | null = null;

      if ($selectedGame.game_id === "skyrimse") {
        const status = await checkSkyrimVersion($selectedGame.game_id, $selectedGame.bottle_name);
        detected = status.current_version;
      } else {
        const ver = await getGameVersion($selectedGame.game_id, $selectedGame.bottle_name);
        detected = ver ?? null;
      }

      if (detected) {
        let mismatch = false;

        if ($selectedGame.game_id === "skyrimse") {
          const detectedIsSE = detected.startsWith("1.5.");
          const targetsSE = versions.some((v: string) => v.startsWith("1.5."));
          const targetsAE = versions.some((v: string) => v.startsWith("1.6."));
          mismatch = (detectedIsSE && !targetsSE && targetsAE)
            || (!detectedIsSE && targetsSE && !targetsAE);
        } else {
          mismatch = !versions.some((v: string) =>
            detected === v
            || detected!.startsWith(v + ".")
            || v.startsWith(detected! + ".")
          );
        }

        if (mismatch) {
          try {
            versionCache = await listGameVersions($selectedGame.game_id);
          } catch { versionCache = []; }
          versionMismatchInfo = { expected: versions, detected };
          pendingManifest = manifest;
          showVersionMismatch = true;
          return;
        }
      }
    } catch {
      // Version check is best-effort
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
        return m;
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
      const dlc = await checkDlcStatus($selectedGame.game_id, $selectedGame.bottle_name);
      if (!dlc.all_present && dlc.dlcs.length > 0) {
        dlcStatus = dlc;
        pendingManifest = manifest;
        showDlcWarning = true;
        return;
      }

      const hasSnap = await hasGameSnapshot($selectedGame.game_id, $selectedGame.bottle_name);
      if (!hasSnap) {
        await proceedWithInstall(manifest);
        return;
      }

      cleanScanning = true;
      pendingManifest = manifest;

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

  async function handleCleanAndInstall() {
    if (!$selectedGame || !pendingManifest) return;

    cleanRunning = true;
    try {
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
    oncancel?.();
  }

  async function handleDlcContinue() {
    showDlcWarning = false;
    dlcStatus = null;
    if (pendingManifest) {
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
    oncancel?.();
  }

  async function proceedWithInstall(manifest: CollectionManifest & Record<string, unknown>) {
    if (!activeCollection || !$selectedGame) return;

    installing = true;

    const modNames = manifest.mods.map((m: { name: string }) => m.name);
    await startInstallTracking(activeCollection.name, modNames.length, modNames, activeCollection.description || activeCollection.summary);

    goto('/collections/progress');

    const collectionName = activeCollection.name;
    const gameId = $selectedGame.game_id;
    const bottleName = $selectedGame.bottle_name;

    installCollection(manifest, gameId, bottleName)
      .then((result) => {
        if (result.failed === 0 && result.skipped === 0) {
          showSuccess(`Collection "${collectionName}" installed successfully`);
        }
        oncomplete?.(result);
      })
      .catch((e: unknown) => {
        showError(`Collection install failed: ${e}`);
        collectionInstallStatus.update(s => s ? { ...s, phase: "failed" as const } : s);
      })
      .finally(() => {
        installing = false;
      });
  }

  onDestroy(() => {
    if (depotPollTimer) { clearInterval(depotPollTimer); depotPollTimer = null; }
  });
</script>

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
      oncancel?.();
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
        <button class="btn btn-ghost" onclick={() => { showOptionalPicker = false; oncancel?.(); }}>Cancel</button>
        <button class="btn btn-accent" onclick={confirmOptionalPicker}>
          Install ({optionalPickerInstallCount} mods)
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- Version Mismatch Modal -->
{#if showVersionMismatch && versionMismatchInfo}
  {@const gameName = $selectedGame?.display_name ?? "this game"}
  {@const isSkyrim = $selectedGame?.game_id === "skyrimse"}
  {@const targetsSE = isSkyrim && versionMismatchInfo.expected.some((v: string) => v.startsWith("1.5."))}
  {@const matchingCached = isSkyrim
    ? versionCache.filter(cv => targetsSE ? cv.version.startsWith("1.5.") : cv.version.startsWith("1.6."))
    : versionCache.filter(cv => versionMismatchInfo!.expected.some((v: string) => cv.version === v || cv.version.startsWith(v)))
  }
  <div class="modal-overlay" onclick={() => { showVersionMismatch = false; versionMismatchInfo = null; }} role="presentation">
    <div class="cleanup-modal" onclick={(e) => e.stopPropagation()} role="dialog" aria-label="Version mismatch warning">
      <div class="cleanup-header">
        <h3 class="cleanup-title">Heads Up &mdash; Wrong Game Version</h3>
        <button class="cleanup-close" onclick={() => { showVersionMismatch = false; versionMismatchInfo = null; }}>&times;</button>
      </div>

      <div class="cleanup-body">
        <div class="cleanup-summary">
          <p class="cleanup-info">
            This collection was built for
            <strong>{gameName} v{versionMismatchInfo.expected.join(' / ')}</strong>{isSkyrim && targetsSE ? " (SE)" : isSkyrim ? " (AE)" : ""},
            but you're running
            <strong>v{versionMismatchInfo.detected}</strong>{isSkyrim ? (versionMismatchInfo.detected.startsWith("1.5.") ? " (SE)" : " (AE)") : ""}.
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
        <button class="btn btn-ghost" disabled={depotDownloading} onclick={() => {
          showVersionMismatch = false;
          versionMismatchInfo = null;
          pendingManifest = null;
          if (depotPollTimer) { clearInterval(depotPollTimer); depotPollTimer = null; }
          depotDownloading = false;
          oncancel?.();
        }}>Cancel</button>

        {#each matchingCached as matchingVersion}
          <button class="btn btn-accent" disabled={versionSwapping} onclick={async () => {
            if (!$selectedGame) return;
            versionSwapping = true;
            try {
              await swapGameVersion($selectedGame.game_id, $selectedGame.bottle_name, matchingVersion.version);
              showVersionMismatch = false;
              versionMismatchInfo = null;
              showSuccess(`Switched to v${matchingVersion.version}. Nice.`);
              if (pendingManifest) await checkPreInstallCleanup(pendingManifest);
            } catch (e) {
              showError(`Version swap failed: ${e}`);
            } finally {
              versionSwapping = false;
            }
          }}>
            {versionSwapping ? "Switching..." : `Switch to v${matchingVersion.version} (Recommended)`}
          </button>
        {/each}

        {#if matchingCached.length === 0 && !depotDownloading}
          {#if isSkyrim}
            <!-- Skyrim SE: automated Steam depot download -->
            <button class="btn btn-accent" onclick={async () => {
              if (!$selectedGame) return;
              depotDownloading = true;
              try {
                const automated = await startDepotDownload($selectedGame.game_id);
                if (!automated) {
                  try {
                    const info = await getDepotDownloadCommand($selectedGame.game_id, $selectedGame.bottle_name);
                    await navigator.clipboard.writeText(info.command);
                    showSuccess("Command copied! Paste it in the Steam console that opened.");
                  } catch { /* ignore clipboard errors */ }
                }
                depotPollTimer = setInterval(async () => {
                  if (!$selectedGame) return;
                  try {
                    const result = await checkDepotReady($selectedGame.game_id, $selectedGame.bottle_name);
                    if (result) {
                      if (depotPollTimer) clearInterval(depotPollTimer);
                      depotPollTimer = null;
                      const status = await applyDowngrade($selectedGame.game_id, $selectedGame.bottle_name);
                      depotDownloading = false;
                      showVersionMismatch = false;
                      versionMismatchInfo = null;
                      showSuccess(`Switched to v${status.current_version}. Let's go.`);
                      if (pendingManifest) await checkPreInstallCleanup(pendingManifest);
                    }
                  } catch { /* keep polling */ }
                }, 3000);
              } catch (e) {
                depotDownloading = false;
                showError(`Download failed: ${e}`);
              }
            }}>
              Download & Switch to v{versionMismatchInfo.expected[0]} (Recommended)
            </button>
          {:else}
            <!-- Other games: use DepotDownloader -->
            <button class="btn btn-accent" disabled={depotDownloading} onclick={async () => {
              if (!$selectedGame || !versionMismatchInfo) return;
              depotDownloading = true;
              try {
                  depotDowngradePhase = "Setting up DepotDownloader...";

                  const status = await ddStatus();
                  if (!status.installed) {
                    depotDowngradePhase = "Downloading DepotDownloader...";
                    await ddInstall();
                  }

                  // Live auth check — passes saved username for accurate detection
                  const authStatus = await ddStatus();
                  if (authStatus.auth_state !== "ready") {
                    depotDownloading = false;
                    depotDowngradePhase = "";
                    showSteamAuth = true;
                    return;
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
                  const ids = steamAppIds[$selectedGame.game_id];
                  if (!ids) {
                    depotDownloading = false;
                    depotDowngradePhase = "";
                    showError("Game not yet supported for automated downgrade. Check SteamDB manually.");
                    return;
                  }

                  // Check for interrupted partial download
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

                  // Listen for real-time progress BEFORE starting download
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
                      ids.app, ids.depot, targetManifest.manifest_id, $selectedGame.game_id
                    );

                    depotDowngradePhase = "Applying downgrade...";
                    const { invoke } = await import("@tauri-apps/api/core");
                    const filesCopied = await invoke("dd_apply_depot", {
                      gameId: $selectedGame.game_id,
                      bottleName: $selectedGame.bottle_name,
                      depotDir,
                    });

                    depotDownloading = false;
                    depotDowngradePhase = "";
                    showVersionMismatch = false;
                    versionMismatchInfo = null;
                    showSuccess(`Downgraded successfully (${filesCopied} files). Let's go.`);
                    if (pendingManifest) await checkPreInstallCleanup(pendingManifest);
                  } finally {
                    unlisten();
                  }
              } catch (e) {
                depotDownloading = false;
                showError(`Downgrade failed: ${e}`);
              }
            }}>
              {depotDownloading
                ? (depotDowngradePhase || "Downloading...")
                : `Downgrade to v${versionMismatchInfo.expected[0]} (Recommended)`}
            </button>
          {/if}
        {/if}

        <button class="btn btn-ghost version-yolo-btn" disabled={depotDownloading} onclick={async () => {
          showVersionMismatch = false;
          versionMismatchInfo = null;
          if (pendingManifest) await checkPreInstallCleanup(pendingManifest);
        }}>
          Install Anyway (Good Luck)
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- Steam Auth Dialog (for DepotDownloader) -->
{#if showSteamAuth}
  <SteamAuthDialog
    onauth={() => {
      showSteamAuth = false;
      showSuccess("Steam authentication successful. Retrying downgrade...");
    }}
    oncancel={() => { showSteamAuth = false; }}
  />
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
              <span class="dlc-icon">{dlc.present ? '\u2713' : '\u2717'}</span>
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

<style>
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

  .cleanup-actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 20px;
    border-top: 1px solid var(--border-primary);
    flex-wrap: wrap;
  }

  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 8px 16px;
    border: none;
    border-radius: var(--radius, 6px);
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s ease;
    white-space: nowrap;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-accent {
    background: var(--system-accent);
    color: #fff;
  }

  .btn-accent:hover:not(:disabled) {
    filter: brightness(1.1);
    box-shadow: 0 1px 6px rgba(0, 122, 255, 0.25);
  }

  .btn-primary {
    background: var(--system-accent);
    color: var(--system-accent-on, #fff);
    padding: 8px 18px;
  }

  .btn-primary:hover:not(:disabled) {
    filter: brightness(1.1);
    box-shadow: 0 1px 6px rgba(0, 122, 255, 0.25);
  }

  .btn-secondary {
    background: var(--surface-hover);
    color: var(--text-primary);
    border: 1px solid var(--border);
  }

  .btn-secondary:hover:not(:disabled) {
    background: var(--surface-active);
  }

  .btn-ghost {
    background: transparent;
    color: var(--text-secondary);
    padding: 8px 12px;
  }

  .btn-ghost:hover:not(:disabled) {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .version-yolo-btn {
    color: var(--text-tertiary) !important;
    font-size: 12px !important;
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
