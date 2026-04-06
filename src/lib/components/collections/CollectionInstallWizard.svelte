<script lang="ts">
  import { onDestroy } from "svelte";
  import { goto } from "$app/navigation";
  import { selectedGame, showError, showSuccess, collectionInstallStatus, modStateVersion } from "$lib/stores";
  import type { CollectionInfo, CollectionManifest, CollectionMod, CollectionModEntry } from "$lib/types";
  import type { RequiredTool, CleanReport, DlcStatus, CachedVersion } from "$lib/types";
  import type { DetectedGame } from "$lib/types";
  import {
    getAllGames,
    installCollection,
    detectCollectionTools,
    checkSkyrimVersion,
    listGameVersions,
    scanGameDirectory,
    hasGameSnapshot,
    checkDlcStatus,
    launchGame,
    getGameVersion,
  } from "$lib/api";
  import { startInstallTracking } from "$lib/installService";
  import RequiredToolsPrompt from "$lib/components/RequiredToolsPrompt.svelte";
  import OptionalModPicker from "$lib/components/collections/OptionalModPicker.svelte";
  import VersionMismatchDialog from "$lib/components/collections/VersionMismatchDialog.svelte";
  import PreInstallCleanup from "$lib/components/collections/PreInstallCleanup.svelte";

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

  // Version mismatch
  let showVersionMismatch = $state(false);
  let versionMismatchInfo = $state<{ expected: string[]; detected: string } | null>(null);
  let versionCache = $state<CachedVersion[]>([]);

  // DLC Detection
  let showDlcWarning = $state(false);
  let dlcStatus = $state<DlcStatus | null>(null);
  let dlcLaunching = $state(false);

  // Optional mod picker
  let showOptionalPicker = $state(false);
  let optionalPickerManifest = $state<(CollectionManifest & Record<string, unknown>) | null>(null);
  type OptionalModChoice = "install" | "install_disabled" | "skip";

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

  function confirmOptionalPicker(choices: Map<number, OptionalModChoice>) {
    if (!optionalPickerManifest) return;

    const mods = optionalPickerManifest.mods
      .map((m: CollectionModEntry, i: number) => {
        const choice = choices.get(i);
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

  function handleCleanupDone() {
    showCleanupModal = false;
    cleanReport = null;
    if (pendingManifest) proceedWithInstall(pendingManifest);
  }

  function handleSkipCleanup() {
    showCleanupModal = false;
    cleanReport = null;
    if (pendingManifest) proceedWithInstall(pendingManifest);
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
    // No cleanup needed — sub-components handle their own timers
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

<!-- Optional Mod Picker -->
{#if showOptionalPicker && optionalPickerManifest}
  <OptionalModPicker
    manifest={optionalPickerManifest}
    onconfirm={confirmOptionalPicker}
    oncancel={() => { showOptionalPicker = false; oncancel?.(); }}
  />
{/if}

<!-- Version Mismatch Dialog -->
{#if showVersionMismatch && versionMismatchInfo && $selectedGame}
  <VersionMismatchDialog
    game={$selectedGame}
    versionInfo={versionMismatchInfo}
    {versionCache}
    onproceed={() => {
      showVersionMismatch = false;
      versionMismatchInfo = null;
      if (pendingManifest) checkPreInstallCleanup(pendingManifest);
    }}
    oncancel={() => {
      showVersionMismatch = false;
      versionMismatchInfo = null;
      pendingManifest = null;
      oncancel?.();
    }}
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
{#if showCleanupModal && cleanReport && $selectedGame}
  <PreInstallCleanup
    game={$selectedGame}
    report={cleanReport}
    onclean={handleCleanupDone}
    onskip={handleSkipCleanup}
    oncancel={handleCancelCleanup}
  />
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

</style>
