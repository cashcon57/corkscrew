<script lang="ts">
  import { onMount } from "svelte";
  import { getConfig, setConfigValue, checkSkse, getSkseDownloadUrl, installSkseFromArchive, listDownloadArchives, deleteDownloadArchive, getDownloadsStats, clearAllDownloadArchives, detectModTools, installModTool, uninstallModTool, launchModTool } from "$lib/api";
  import { config, showError, showSuccess, selectedGame, skseStatus, currentPage, appVersion, updateReady, updateVersion, updateChecking, updateError, triggerUpdateCheck } from "$lib/stores";
  import type { AppConfig, ModTool } from "$lib/types";
  import ThemeToggle from "$lib/components/ThemeToggle.svelte";
  import SettingsAuthSection from "./settings-auth-section.svelte";
  import IniManagerPanel from "$lib/components/IniManagerPanel.svelte";
  import WineDiagnosticsPanel from "$lib/components/WineDiagnosticsPanel.svelte";
  import { open as dialogOpen } from "@tauri-apps/plugin-dialog";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { relaunch } from "@tauri-apps/plugin-process";

  let manualCheckDone = $state(false);

  async function handleCheckForUpdates() {
    manualCheckDone = false;
    if (triggerUpdateCheck) {
      await triggerUpdateCheck();
    }
    manualCheckDone = true;
  }

  let downloadDir = $state("");
  let savingDownloadDir = $state(false);
  let autoDeleteArchives = $state(false);
  let savingAutoDelete = $state(false);
  let installingSkse = $state(false);
  let showComparisonDialog = $state(false);

  // Download archive management
  interface DownloadArchive {
    filename: string;
    path: string;
    size_bytes: number;
    modified_at: number;
  }
  let archives = $state<DownloadArchive[]>([]);
  let loadingArchives = $state(false);
  let deletingArchive = $state<string | null>(null);
  let clearingAll = $state(false);
  let showArchiveList = $state(false);
  let downloadsStats = $state<{ total_size_bytes: number; archive_count: number; directory: string } | null>(null);

  // Mod tools
  let modTools = $state<ModTool[]>([]);
  let loadingTools = $state(false);
  let installingTool = $state<string | null>(null);
  let launchingTool = $state<string | null>(null);
  let uninstallingTool = $state<string | null>(null);

  const game = $derived($selectedGame);
  const skse = $derived($skseStatus);
  const isSkyrim = $derived(game?.game_id === "skyrimse");
  const autoInstallTools = $derived(modTools.filter(t => t.can_auto_install));
  const manualTools = $derived(modTools.filter(t => !t.can_auto_install));

  // Detect platform for comparison dialog
  const isMac = typeof navigator !== "undefined" && navigator.platform?.startsWith("Mac");

  interface LayerInfo {
    name: string;
    url: string;
    description: string;
    platforms: string[];
    cost: string;
    color: string;
    bg: string;
    icon: string; // SVG icon key
    recommendation?: string;
  }

  const layers: LayerInfo[] = [
    { name: "CrossOver", url: "https://www.codeweavers.com/crossover", description: "Commercial Wine wrapper with excellent compatibility and support. Best plug-and-play experience. CodeWeavers funds Wine development.", platforms: ["macOS", "Linux"], cost: "Paid ($74)", color: "#c850c0", bg: "rgba(200, 80, 192, 0.14)", icon: "crossover", recommendation: "Best overall compatibility" },
    { name: "Moonshine", url: "https://github.com/ybmeng/moonshine", description: "Free, open-source Wine wrapper with Wine Staging 11.2, OpenGL 3.2+ support, and macOS 26 compatibility.", platforms: ["macOS"], cost: "Free", color: "#bf5af2", bg: "rgba(191, 90, 242, 0.14)", icon: "moonshine", recommendation: "Best free option for macOS" },
    { name: "Heroic", url: "https://heroicgameslauncher.com/", description: "Open-source game launcher for GOG and Epic Games. Bundles Wine/Proton for easy setup.", platforms: ["macOS", "Linux"], cost: "Free", color: "#0a84ff", bg: "rgba(10, 132, 255, 0.14)", icon: "heroic" },
    { name: "Mythic", url: "https://getmythic.app/", description: "Native macOS game launcher for Epic Games with built-in Wine support.", platforms: ["macOS"], cost: "Free", color: "#30d158", bg: "rgba(48, 209, 88, 0.14)", icon: "mythic" },
    { name: "Lutris", url: "https://lutris.net/", description: "Open-source gaming platform for Linux. Manages Wine, Proton, and native runners.", platforms: ["Linux"], cost: "Free", color: "#ff9f0a", bg: "rgba(255, 159, 10, 0.14)", icon: "lutris", recommendation: "Best for Linux desktop" },
    { name: "Proton / Steam", url: "https://store.steampowered.com/", description: "Valve's Wine fork built into Steam. The standard for gaming on Linux. Seamless for Steam library games.", platforms: ["Linux"], cost: "Free", color: "#1a9fff", bg: "rgba(26, 159, 255, 0.14)", icon: "proton", recommendation: "Best for Steam Deck / SteamOS" },
    { name: "Bottles", url: "https://usebottles.com/", description: "Modern Linux app for creating and managing Wine prefixes with versioned runners.", platforms: ["Linux"], cost: "Free", color: "#3584e4", bg: "rgba(53, 132, 228, 0.14)", icon: "bottles" },
    { name: "Wine", url: "https://www.winehq.org/", description: "The original compatibility layer. Manual setup, maximum flexibility.", platforms: ["macOS", "Linux"], cost: "Free", color: "#722F37", bg: "rgba(114, 47, 55, 0.14)", icon: "wine" },
  ];

  // Group layers into sections
  const recommended = $derived(
    isMac
      ? layers.filter(l => l.recommendation && l.platforms.includes("macOS"))
      : layers.filter(l => l.recommendation && l.platforms.includes("Linux"))
  );

  const otherOptions = $derived(
    isMac
      ? layers.filter(l => !l.recommendation && l.platforms.includes("macOS"))
      : layers.filter(l => !l.recommendation && l.platforms.includes("Linux"))
  );

  // For comparison dialog
  const platformLayers = $derived(
    isMac
      ? layers.filter(l => l.platforms.includes("macOS"))
      : layers.filter(l => l.platforms.includes("Linux"))
  );

  function formatBytes(bytes: number): string {
    if (bytes === 0) return "0 B";
    const units = ["B", "KB", "MB", "GB"];
    const i = Math.floor(Math.log(bytes) / Math.log(1024));
    return `${(bytes / Math.pow(1024, i)).toFixed(i > 0 ? 1 : 0)} ${units[i]}`;
  }

  function formatDate(unixSecs: number): string {
    if (!unixSecs) return "";
    return new Date(unixSecs * 1000).toLocaleDateString(undefined, {
      month: "short", day: "numeric", year: "numeric",
    });
  }

  onMount(async () => {
    try {
      const cfg = await getConfig();
      config.set(cfg);
      downloadDir = cfg.download_dir ?? "";
      autoDeleteArchives = (cfg as Record<string, unknown>).auto_delete_archives === "true";
    } catch (e: unknown) {
      showError(`Failed to load config: ${e}`);
    }

    if (game && isSkyrim) {
      try {
        const status = await checkSkse(game.game_id, game.bottle_name);
        skseStatus.set(status);
      } catch { /* ignore */ }
    }

    // Load download stats
    try {
      downloadsStats = await getDownloadsStats();
    } catch { /* ignore */ }

    // Load mod tools
    if (game) {
      try {
        loadingTools = true;
        modTools = await detectModTools(game.game_id, game.bottle_name);
      } catch { /* ignore */ } finally {
        loadingTools = false;
      }
    }
  });

  async function handleOpenSkseDownload() {
    try {
      const url = await getSkseDownloadUrl();
      await openUrl(url);
    } catch (e: unknown) {
      showError(`Failed to open SKSE download page: ${e}`);
    }
  }

  async function handleInstallSkseFromArchive() {
    if (!game) return;
    try {
      const selected = await dialogOpen({
        title: "Select SKSE Archive (.7z or .zip)",
        filters: [{ name: "Archives", extensions: ["7z", "zip"] }],
      });
      if (!selected) return;

      const archivePath = selected as string;
      installingSkse = true;
      const status = await installSkseFromArchive(game.game_id, game.bottle_name, archivePath);
      skseStatus.set(status);
      showSuccess("SKSE installed successfully");
    } catch (e: unknown) {
      showError(`Failed to install SKSE: ${e}`);
    } finally {
      installingSkse = false;
    }
  }

  async function handleInstallTool(toolId: string) {
    if (!game) return;
    installingTool = toolId;
    try {
      await installModTool(toolId, game.game_id, game.bottle_name);
      modTools = await detectModTools(game.game_id, game.bottle_name);
      showSuccess("Tool installed successfully");
    } catch (e: unknown) {
      showError(`Failed to install tool: ${e}`);
    } finally {
      installingTool = null;
    }
  }

  async function handleUninstallTool(toolId: string) {
    if (!game) return;
    uninstallingTool = toolId;
    try {
      await uninstallModTool(toolId, game.game_id, game.bottle_name);
      modTools = await detectModTools(game.game_id, game.bottle_name);
      showSuccess("Tool removed");
    } catch (e: unknown) {
      showError(`Failed to uninstall tool: ${e}`);
    } finally {
      uninstallingTool = null;
    }
  }

  async function handleLaunchTool(toolId: string) {
    if (!game) return;
    launchingTool = toolId;
    try {
      await launchModTool(toolId, game.game_id, game.bottle_name);
      showSuccess("Tool launched");
    } catch (e: unknown) {
      showError(`Failed to launch tool: ${e}`);
    } finally {
      launchingTool = null;
    }
  }

  async function browseDownloadDir() {
    try {
      const selected = await dialogOpen({
        directory: true,
        title: "Choose Download Folder",
        defaultPath: downloadDir || undefined,
      });
      if (selected) {
        downloadDir = selected as string;
        await saveDownloadDir();
      }
    } catch (e: unknown) {
      showError(`Failed to open folder picker: ${e}`);
    }
  }

  async function saveDownloadDir() {
    savingDownloadDir = true;
    try {
      await setConfigValue("download_dir", downloadDir);
      downloadsStats = await getDownloadsStats();
      showSuccess("Download directory saved");
    } catch (e: unknown) {
      showError(`Failed to save: ${e}`);
    } finally {
      savingDownloadDir = false;
    }
  }

  async function toggleAutoDelete() {
    savingAutoDelete = true;
    try {
      autoDeleteArchives = !autoDeleteArchives;
      await setConfigValue("auto_delete_archives", autoDeleteArchives ? "true" : "false");
      showSuccess(autoDeleteArchives ? "Archives will be deleted after install" : "Archives will be kept after install");
    } catch (e: unknown) {
      autoDeleteArchives = !autoDeleteArchives; // revert
      showError(`Failed to save setting: ${e}`);
    } finally {
      savingAutoDelete = false;
    }
  }

  async function loadArchives() {
    loadingArchives = true;
    try {
      archives = await listDownloadArchives();
      showArchiveList = true;
    } catch (e: unknown) {
      showError(`Failed to load archives: ${e}`);
    } finally {
      loadingArchives = false;
    }
  }

  async function handleDeleteArchive(archive: DownloadArchive) {
    deletingArchive = archive.path;
    try {
      await deleteDownloadArchive(archive.path);
      archives = archives.filter(a => a.path !== archive.path);
      downloadsStats = await getDownloadsStats();
      showSuccess(`Deleted ${archive.filename}`);
    } catch (e: unknown) {
      showError(`Failed to delete: ${e}`);
    } finally {
      deletingArchive = null;
    }
  }

  async function handleClearAll() {
    clearingAll = true;
    try {
      const count = await clearAllDownloadArchives();
      archives = [];
      downloadsStats = await getDownloadsStats();
      showSuccess(`Deleted ${count} archive${count !== 1 ? "s" : ""}`);
    } catch (e: unknown) {
      showError(`Failed to clear archives: ${e}`);
    } finally {
      clearingAll = false;
    }
  }
</script>

{#snippet layerIcon(icon: string)}
  {#if icon === "crossover"}
    <svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
      <rect x="4" y="4" width="8" height="8" rx="1" transform="rotate(45 8 8)" />
      <rect x="8" y="8" width="8" height="8" rx="1" transform="rotate(45 12 12)" />
    </svg>
  {:else if icon === "moonshine"}
    <svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
      <path d="M15 4a7 7 0 1 0-1 12A5 5 0 0 1 15 4z" />
    </svg>
  {:else if icon === "heroic"}
    <svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
      <path d="M10 2L3 5v5c0 4.4 3 7.5 7 9 4-1.5 7-4.6 7-9V5l-7-3z" />
      <line x1="10" y1="7" x2="10" y2="14" />
      <line x1="8" y1="9" x2="12" y2="9" />
    </svg>
  {:else if icon === "mythic"}
    <svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
      <rect x="3" y="11" width="5" height="5" rx="0.5" transform="rotate(45 5.5 13.5)" />
      <rect x="6" y="8" width="5" height="5" rx="0.5" transform="rotate(45 8.5 10.5)" />
      <rect x="9" y="5" width="5" height="5" rx="0.5" transform="rotate(45 11.5 7.5)" />
    </svg>
  {:else if icon === "wine"}
    <svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
      <path d="M7 2h6l.5 5a3.5 3.5 0 0 1-7 0L7 2z" /><line x1="10" y1="12" x2="10" y2="17" />
      <line x1="7" y1="17" x2="13" y2="17" />
    </svg>
  {:else if icon === "lutris"}
    <svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
      <circle cx="10" cy="11" r="4" />
      <path d="M6.5 8.5C5 6 6.5 3 10 3c3.5 0 5 3 3.5 5.5" />
      <circle cx="8" cy="5.5" r="0.5" fill="currentColor" />
    </svg>
  {:else if icon === "proton"}
    <svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
      <circle cx="10" cy="10" r="1.5" fill="currentColor" stroke="none" />
      <ellipse cx="10" cy="10" rx="7" ry="3" />
      <ellipse cx="10" cy="10" rx="7" ry="3" transform="rotate(60 10 10)" />
      <ellipse cx="10" cy="10" rx="7" ry="3" transform="rotate(120 10 10)" />
    </svg>
  {:else if icon === "bottles"}
    <svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
      <path d="M8 2h4v3l2 2v8a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V7l2-2V2z" />
    </svg>
  {/if}
{/snippet}

{#snippet layerRow(layer: LayerInfo, isRecommended: boolean)}
  <div class="card-row layer-row">
    <div class="layer-icon" style="color: {layer.color}; background: {layer.bg};">
      {@render layerIcon(layer.icon)}
    </div>
    <div class="layer-info">
      <div class="layer-name-row">
        <a
          href={layer.url}
          target="_blank"
          rel="noopener noreferrer"
          class="layer-name"
          style="color: {layer.color};"
        >{layer.name}</a>
        <div class="layer-badges">
          {#each layer.platforms as platform}
            <span class="platform-badge" class:mac={platform === "macOS"} class:linux={platform === "Linux"}>{platform}</span>
          {/each}
          <span class="cost-badge" class:cost-free={layer.cost === "Free"} class:cost-paid={layer.cost !== "Free"}>
            {layer.cost === "Free" ? "Free" : layer.cost}
          </span>
        </div>
        <a
          href={layer.url}
          target="_blank"
          rel="noopener noreferrer"
          class="layer-download-link"
        >
          <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M6 2v6.5" /><path d="M3 6.5L6 9.5 9 6.5" /><line x1="2" y1="11" x2="10" y2="11" />
          </svg>
          Download
        </a>
      </div>
      <span class="layer-description">{layer.description}</span>
      {#if isRecommended && layer.recommendation}
        <span class="layer-rec-note">{layer.recommendation}</span>
      {/if}
    </div>
  </div>
{/snippet}

<div class="settings-page">
  <h1 class="page-title">Settings</h1>

  <!-- Appearance -->
  <div class="section">
    <h2 class="section-title">Appearance</h2>
    <div class="section-card">
      <div class="card-row appearance-row">
        <span class="row-label">Theme</span>
        <ThemeToggle />
      </div>
    </div>
  </div>

  {#if isSkyrim}
    <!-- Game Tools -->
    <div class="section">
      <h2 class="section-title">Game Tools</h2>
      <div class="section-card">
        <div class="card-row tool-row">
          <div class="tool-info">
            <span class="row-label">SKSE (Script Extender)</span>
            <span class="tool-description">
              {#if skse?.installed}
                Installed{skse.version ? ` — v${skse.version}` : ""}
              {:else}
                Required by most Skyrim mods
              {/if}
            </span>
          </div>
          <div class="tool-action">
            {#if skse?.installed}
              <span class="badge badge-green">Installed</span>
            {:else}
              <button
                class="btn-secondary"
                onclick={handleOpenSkseDownload}
                type="button"
                title="Opens skse.silverlock.org in your browser"
              >
                Download SKSE
              </button>
              <button
                class="btn-primary"
                onclick={handleInstallSkseFromArchive}
                disabled={installingSkse}
                type="button"
                title="Install from a .7z or .zip you already downloaded"
              >
                {installingSkse ? "Installing..." : "Install from Archive"}
              </button>
            {/if}
          </div>
        </div>
      </div>
    </div>
  {/if}

  <!-- Modding Tools -->
  {#if game}
    <div class="section">
      <h2 class="section-title">Modding Tools</h2>

      {#if loadingTools}
        <div class="section-card">
          <div class="card-row"><span class="tool-description">Scanning for tools...</span></div>
        </div>
      {:else if modTools.length === 0}
        <div class="section-card">
          <div class="card-row"><span class="tool-description">No game selected or no tools available.</span></div>
        </div>
      {:else}
        <!-- Auto-installable tools -->
        {#if autoInstallTools.length > 0}
          <div class="layers-group">
            <span class="layers-group-label">Auto-Install from GitHub</span>
            <div class="section-card">
              {#each autoInstallTools as tool, i (tool.id)}
                {#if i > 0}<div class="card-divider"></div>{/if}
                <div class="card-row tool-row">
                  <div class="tool-info">
                    <span class="row-label">{tool.name}</span>
                    <span class="tool-description">
                      {tool.description}
                      <span class="tool-license">{tool.license}</span>
                    </span>
                    {#if tool.wine_notes}
                      <span class="tool-wine-note">{tool.wine_notes}</span>
                    {/if}
                  </div>
                  <div class="tool-action">
                    {#if tool.detected_path}
                      <span class="badge badge-green">Installed</span>
                      <button
                        class="btn-primary btn-sm"
                        onclick={() => handleLaunchTool(tool.id)}
                        disabled={launchingTool === tool.id}
                        type="button"
                      >
                        {launchingTool === tool.id ? "..." : "Launch"}
                      </button>
                      <button
                        class="btn-delete-sm"
                        onclick={() => handleUninstallTool(tool.id)}
                        disabled={uninstallingTool === tool.id}
                        type="button"
                        aria-label="Uninstall {tool.name}"
                      >
                        {#if uninstallingTool === tool.id}
                          <span class="spinner-xs"></span>
                        {:else}
                          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <polyline points="3 6 5 6 21 6" /><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
                          </svg>
                        {/if}
                      </button>
                    {:else}
                      <button
                        class="btn-primary btn-sm"
                        onclick={() => handleInstallTool(tool.id)}
                        disabled={installingTool === tool.id}
                        type="button"
                      >
                        {#if installingTool === tool.id}
                          <span class="spinner-xs"></span> Installing...
                        {:else}
                          Install
                        {/if}
                      </button>
                    {/if}
                  </div>
                </div>
              {/each}
            </div>
          </div>
        {/if}

        <!-- Manual download tools -->
        {#if manualTools.length > 0}
          <div class="layers-group">
            <span class="layers-group-label">Manual Download Required</span>
            <div class="section-card">
              {#each manualTools as tool, i (tool.id)}
                {#if i > 0}<div class="card-divider"></div>{/if}
                <div class="card-row tool-row">
                  <div class="tool-info">
                    <span class="row-label">{tool.name}</span>
                    <span class="tool-description">
                      {tool.description}
                      <span class="tool-license">{tool.license}</span>
                    </span>
                    {#if tool.wine_notes}
                      <span class="tool-wine-note">{tool.wine_notes}</span>
                    {/if}
                  </div>
                  <div class="tool-action">
                    {#if tool.detected_path}
                      <span class="badge badge-green">Installed</span>
                      <button
                        class="btn-primary btn-sm"
                        onclick={() => handleLaunchTool(tool.id)}
                        disabled={launchingTool === tool.id}
                        type="button"
                      >
                        {launchingTool === tool.id ? "..." : "Launch"}
                      </button>
                    {:else if tool.download_url}
                      <button
                        class="btn-secondary btn-sm"
                        onclick={() => openUrl(tool.download_url!)}
                        type="button"
                      >
                        Download
                      </button>
                    {/if}
                  </div>
                </div>
              {/each}
            </div>
          </div>
        {/if}
      {/if}
    </div>
  {/if}

  <!-- INI Settings (Skyrim) -->
  {#if isSkyrim && game}
    <div class="section">
      <h2 class="section-title">INI Settings</h2>
      <IniManagerPanel gameId={game.game_id} bottleName={game.bottle_name} />
    </div>
  {/if}

  <!-- Wine Diagnostics -->
  {#if game}
    <div class="section">
      <h2 class="section-title">Wine Diagnostics</h2>
      <WineDiagnosticsPanel gameId={game.game_id} bottleName={game.bottle_name} />
    </div>
  {/if}

  <!-- Nexus Mods Account -->
  <SettingsAuthSection />

  <!-- Downloads -->
  <div class="section">
    <h2 class="section-title">Downloads</h2>
    <div class="section-card">
      <!-- Download Directory -->
      <div class="card-row">
        <label class="row-label" for="download-dir">Download Folder</label>
        <div class="row-control">
          <div class="input-with-actions">
            <input
              id="download-dir"
              type="text"
              bind:value={downloadDir}
              placeholder={downloadsStats?.directory ?? "Default location"}
              class="settings-input"
            />
            <button
              class="btn-ghost"
              onclick={browseDownloadDir}
              type="button"
            >
              Browse
            </button>
            <button
              class="btn-primary"
              onclick={saveDownloadDir}
              disabled={savingDownloadDir || !downloadDir}
              type="button"
            >
              {savingDownloadDir ? "Saving..." : "Save"}
            </button>
          </div>
          {#if !downloadDir && downloadsStats?.directory}
            <span class="input-hint">Default: {downloadsStats.directory}</span>
          {/if}
        </div>
      </div>
      <div class="card-divider"></div>

      <!-- Auto-Delete Setting -->
      <div class="card-row toggle-row">
        <div class="toggle-info">
          <span class="row-label">Delete archives after install</span>
          <span class="toggle-description">Automatically remove .zip/.7z files once mods are installed. Saves disk space but prevents reinstalling from local cache.</span>
        </div>
        <button
          class="toggle-switch"
          class:toggle-on={autoDeleteArchives}
          onclick={toggleAutoDelete}
          disabled={savingAutoDelete}
          type="button"
          role="switch"
          aria-checked={autoDeleteArchives}
        >
          <span class="toggle-thumb"></span>
        </button>
      </div>
      <div class="card-divider"></div>

      <!-- Archives Management -->
      <div class="card-row">
        <div class="archives-summary">
          <div class="archives-info">
            <span class="row-label">Downloaded Archives</span>
            {#if downloadsStats}
              <span class="archives-stats">
                {downloadsStats.archive_count} file{downloadsStats.archive_count !== 1 ? "s" : ""}
                &middot;
                {formatBytes(downloadsStats.total_size_bytes)}
              </span>
            {/if}
          </div>
          <div class="archives-actions">
            {#if !showArchiveList}
              <button
                class="btn-ghost"
                onclick={loadArchives}
                disabled={loadingArchives}
                type="button"
              >
                {loadingArchives ? "Loading..." : "Manage"}
              </button>
            {:else}
              <button
                class="btn-ghost"
                onclick={() => showArchiveList = false}
                type="button"
              >
                Hide
              </button>
              {#if archives.length > 0}
                <button
                  class="btn-danger"
                  onclick={handleClearAll}
                  disabled={clearingAll}
                  type="button"
                >
                  {clearingAll ? "Deleting..." : "Delete All"}
                </button>
              {/if}
            {/if}
          </div>
        </div>
      </div>

      {#if showArchiveList}
        {#if archives.length === 0}
          <div class="card-divider"></div>
          <div class="card-row">
            <span class="archives-empty">No downloaded archives found.</span>
          </div>
        {:else}
          {#each archives as archive (archive.path)}
            <div class="card-divider"></div>
            <div class="card-row archive-row">
              <div class="archive-info">
                <span class="archive-name" title={archive.filename}>{archive.filename}</span>
                <span class="archive-meta">
                  {formatBytes(archive.size_bytes)}
                  {#if archive.modified_at}
                    &middot; {formatDate(archive.modified_at)}
                  {/if}
                </span>
              </div>
              <button
                class="btn-delete-sm"
                onclick={() => handleDeleteArchive(archive)}
                disabled={deletingArchive === archive.path}
                type="button"
                aria-label="Delete {archive.filename}"
              >
                {#if deletingArchive === archive.path}
                  <span class="spinner-xs"></span>
                {:else}
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <polyline points="3 6 5 6 21 6" />
                    <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
                  </svg>
                {/if}
              </button>
            </div>
          {/each}
        {/if}
      {/if}
    </div>
  </div>

  <!-- Compatibility Layers -->
  <div class="section">
    <h2 class="section-title">Compatibility Layers</h2>
    <div class="section-card">
      <div class="card-row">
        <div class="row-content full">
          <span class="row-description">
            Corkscrew works with games running through Wine-based compatibility layers.
            Install one of these to run Windows games on your system.
          </span>
        </div>
      </div>
    </div>

    <!-- Recommended -->
    {#if recommended.length > 0}
      <div class="layers-group">
        <span class="layers-group-label">Recommended</span>
        <div class="section-card">
          {#each recommended as layer, i}
            {#if i > 0}<div class="card-divider"></div>{/if}
            {@render layerRow(layer, true)}
          {/each}
        </div>
      </div>
    {/if}

    <!-- Other compatible options -->
    {#if otherOptions.length > 0}
      <div class="layers-group">
        <span class="layers-group-label">Other Compatible Options</span>
        <div class="section-card">
          {#each otherOptions as layer, i}
            {#if i > 0}<div class="card-divider"></div>{/if}
            {@render layerRow(layer, false)}
          {/each}
        </div>
      </div>
    {/if}

    <div class="section-action">
      <button
        class="btn-comparison"
        onclick={() => showComparisonDialog = true}
        type="button"
      >
        <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="7" cy="7" r="6" />
          <line x1="7" y1="9.5" x2="7" y2="6.5" />
          <circle cx="7" cy="4.5" r="0.5" fill="currentColor" />
        </svg>
        Which should I use?
      </button>
    </div>
  </div>

  <!-- Comparison Dialog -->
  {#if showComparisonDialog}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div class="dialog-backdrop" onclick={() => showComparisonDialog = false} role="presentation">
      <!-- svelte-ignore a11y_interactive_supports_focus -->
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div class="dialog" onclick={(e) => e.stopPropagation()} role="dialog" aria-label="Compatibility layer comparison">
        <div class="dialog-header">
          <h3 class="dialog-title">Which layer should I use?</h3>
          <span class="dialog-platform-label">{isMac ? "macOS" : "Linux"} recommendations</span>
          <button class="dialog-close" onclick={() => showComparisonDialog = false} type="button" aria-label="Close">
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
              <line x1="2" y1="2" x2="10" y2="10" /><line x1="10" y1="2" x2="2" y2="10" />
            </svg>
          </button>
        </div>
        <div class="dialog-body">
          {#each platformLayers as layer, i}
            <div class="comparison-item" class:recommended={!!layer.recommendation}>
              <div class="comparison-header">
                <div class="comparison-icon" style="color: {layer.color}; background: {layer.bg};">
                  {@render layerIcon(layer.icon)}
                </div>
                <div class="comparison-title">
                  <span class="comparison-name">{layer.name}</span>
                  {#if layer.recommendation}
                    <span class="recommendation-badge">{layer.recommendation}</span>
                  {/if}
                </div>
                <span class="comparison-cost" class:cost-free={layer.cost === "Free"} class:cost-paid={layer.cost !== "Free"}>
                  {layer.cost === "Free" ? "Free" : layer.cost}
                </span>
              </div>
              <p class="comparison-description">{layer.description}</p>
              <a
                href={layer.url}
                target="_blank"
                rel="noopener noreferrer"
                class="comparison-link"
              >Download</a>
            </div>
          {/each}
        </div>
      </div>
    </div>
  {/if}

  <!-- About -->
  <div class="section">
    <h2 class="section-title">About</h2>
    <div class="section-card">
      <div class="card-row about-row">
        <span class="row-label">Version</span>
        <div class="version-row-right">
          <span class="row-value">v{$appVersion}</span>
          {#if $updateReady}
            <button class="btn-update-ready" onclick={() => relaunch()} type="button">
              Restart for v{$updateVersion}
            </button>
          {:else}
            <button
              class="btn-ghost btn-sm"
              onclick={handleCheckForUpdates}
              disabled={$updateChecking}
              type="button"
            >
              {#if $updateChecking}
                <span class="spinner-xs"></span> Checking...
              {:else if manualCheckDone && $updateError}
                <span title={$updateError}>Check failed</span>
              {:else if manualCheckDone && !$updateReady}
                Up to date
              {:else}
                Check for Updates
              {/if}
            </button>
          {/if}
        </div>
      </div>
      <div class="card-divider"></div>
      <div class="card-row about-row">
        <span class="row-label">Author</span>
        <span class="row-value">cashconway</span>
      </div>
      <div class="card-divider"></div>
      <div class="card-row about-row">
        <span class="row-label">License</span>
        <span class="row-value">GPL-3.0-or-later</span>
      </div>
      <div class="card-divider"></div>
      <div class="card-row about-row">
        <span class="row-label">Platform</span>
        <span class="row-value">macOS / Linux</span>
      </div>
      <div class="card-divider"></div>
      <div class="card-row about-row">
        <span class="row-label">Links</span>
        <div class="row-value about-links">
          <button class="btn-link" onclick={() => openUrl("https://github.com/cashconway/Corkscrew")}>GitHub</button>
          <button class="btn-link" onclick={() => openUrl("https://ko-fi.com/cash508287")}>Support</button>
        </div>
      </div>
    </div>
  </div>
</div>

<style>
  .settings-page {
    width: 100%;
    max-width: 860px;
    padding: var(--space-8) var(--space-6);
  }

  .page-title {
    font-size: 28px;
    font-weight: 700;
    letter-spacing: -0.025em;
    color: var(--text-primary);
    margin-bottom: var(--space-8);
  }

  /* --- Sections --- */

  .section {
    margin-bottom: var(--space-6);
  }

  .section-title {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.02em;
    padding: 0 var(--space-4);
    margin-bottom: var(--space-2);
  }

  .section-card {
    background: var(--bg-grouped-secondary);
    border-radius: var(--radius-lg);
    overflow: hidden;
    box-shadow: var(--glass-edge-shadow);
  }

  /* --- Card rows --- */

  .card-row {
    padding: var(--space-3) var(--space-4);
  }

  .card-divider {
    height: 1px;
    background: var(--separator);
    margin-left: var(--space-4);
  }

  .row-label {
    font-size: 13px;
    font-weight: 400;
    color: var(--text-primary);
    white-space: nowrap;
    flex-shrink: 0;
  }

  .row-content.full {
    width: 100%;
  }

  .row-description {
    font-size: 13px;
    color: var(--text-secondary);
    line-height: 1.5;
  }

  .row-description a {
    color: var(--system-accent);
    text-decoration: none;
  }

  .row-description a:hover {
    text-decoration: underline;
  }

  /* --- Form controls --- */

  .row-control {
    margin-top: var(--space-2);
    width: 100%;
  }

  .input-with-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .settings-input {
    flex: 1;
    min-width: 0;
    padding: var(--space-2) var(--space-3);
    background: var(--bg-base);
    border: 1px solid var(--separator-opaque);
    border-radius: var(--radius-sm);
    color: var(--text-primary);
    font-size: 13px;
    font-family: var(--font-sans);
    outline: none;
    transition: border-color var(--duration) var(--ease);
  }

  .settings-input:focus {
    border-color: var(--system-accent);
    box-shadow: 0 0 0 3px rgba(0, 122, 255, 0.15);
  }

  .settings-input::placeholder {
    color: var(--text-tertiary);
  }

  /* --- Buttons --- */

  .btn-primary {
    padding: var(--space-1) var(--space-3);
    background: var(--system-accent);
    color: var(--system-accent-on);
    font-size: 13px;
    font-weight: 500;
    border: none;
    border-radius: var(--radius-sm);
    white-space: nowrap;
    transition: background var(--duration-fast) var(--ease);
  }

  .btn-primary:hover:not(:disabled) {
    background: var(--system-accent-hover);
  }

  .btn-primary:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-secondary {
    padding: var(--space-1) var(--space-3);
    background: var(--surface-hover);
    color: var(--text-secondary);
    font-size: 13px;
    font-weight: 500;
    border: 1px solid var(--separator);
    border-radius: var(--radius-sm);
    white-space: nowrap;
    transition: all var(--duration-fast) var(--ease);
  }

  .btn-secondary:hover {
    background: var(--bg-tertiary);
    color: var(--text-primary);
  }

  .btn-ghost {
    padding: var(--space-1) var(--space-3);
    background: transparent;
    color: var(--text-secondary);
    font-size: 13px;
    font-weight: 500;
    border: 1px solid var(--separator-opaque);
    border-radius: var(--radius-sm);
    white-space: nowrap;
    transition:
      background var(--duration-fast) var(--ease),
      color var(--duration-fast) var(--ease);
  }

  .btn-ghost:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  /* --- Appearance --- */

  .appearance-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  /* --- Game Tools --- */

  .tool-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
  }

  .tool-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .tool-description {
    font-size: 12px;
    color: var(--text-tertiary);
  }

  .tool-action {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .badge-green {
    display: inline-block;
    padding: 1px var(--space-2);
    font-size: 11px;
    font-weight: 600;
    color: var(--green);
    background: color-mix(in srgb, var(--green) 15%, transparent);
    border-radius: var(--radius-sm);
  }

  /* --- About rows --- */

  .about-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .about-row .row-label {
    color: var(--text-secondary);
    font-weight: 400;
  }

  .row-value {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-primary);
  }

  .btn-link {
    background: none;
    border: none;
    color: var(--system-accent);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    padding: 0;
  }

  .btn-link:hover {
    text-decoration: underline;
  }

  /* --- Compatibility Layers --- */

  .layers-group {
    margin-top: var(--space-3);
  }

  .layers-group-label {
    display: block;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-tertiary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 0 var(--space-4);
    margin-bottom: var(--space-1);
  }

  .layer-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .layer-icon {
    width: 32px;
    height: 32px;
    border-radius: var(--radius-sm);
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .layer-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .layer-name-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .layer-name {
    font-size: 13px;
    font-weight: 600;
    text-decoration: none;
  }

  .layer-name:hover {
    text-decoration: underline;
  }

  .layer-badges {
    display: flex;
    gap: 4px;
    align-items: center;
  }

  .platform-badge {
    font-size: 10px;
    font-weight: 600;
    padding: 1px 6px;
    border-radius: 3px;
    letter-spacing: 0.01em;
  }

  .platform-badge.mac {
    color: var(--system-accent);
    background: var(--system-accent-subtle);
  }

  .platform-badge.linux {
    color: var(--yellow);
    background: rgba(255, 204, 0, 0.12);
  }

  .cost-badge {
    font-size: 10px;
    font-weight: 600;
    padding: 1px 6px;
    border-radius: 3px;
  }

  .cost-free {
    color: var(--green);
    background: var(--green-subtle);
  }

  .cost-paid {
    color: var(--text-tertiary);
    background: var(--surface-hover);
  }

  .layer-description {
    font-size: 12px;
    color: var(--text-tertiary);
    line-height: 1.4;
  }

  .layer-rec-note {
    font-size: 11px;
    font-weight: 500;
    color: var(--system-accent);
    margin-top: 1px;
  }

  .layer-download-link {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    margin-left: auto;
    font-size: 11px;
    font-weight: 500;
    color: var(--system-accent);
    text-decoration: none;
    padding: 2px var(--space-2);
    border-radius: var(--radius-sm);
    transition: background var(--duration-fast) var(--ease);
    flex-shrink: 0;
  }

  .layer-download-link:hover {
    background: var(--system-accent-subtle);
    text-decoration: none;
  }

  .section-action {
    padding: var(--space-3) var(--space-4) 0;
  }

  .btn-comparison {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: transparent;
    color: var(--system-accent);
    font-size: 13px;
    font-weight: 500;
    border: none;
    cursor: pointer;
    border-radius: var(--radius-sm);
    transition: background var(--duration-fast) var(--ease);
  }

  .btn-comparison:hover {
    background: var(--system-accent-subtle);
  }

  /* --- Comparison Dialog --- */

  .dialog-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 2000;
    animation: fadeIn 0.2s var(--ease);
  }

  .dialog {
    background: var(--bg-elevated);
    border: 1px solid var(--separator-opaque);
    border-radius: var(--radius-xl);
    box-shadow: var(--glass-edge-shadow), var(--shadow-lg);
    width: 520px;
    max-width: calc(100vw - var(--space-8));
    max-height: calc(100vh - var(--space-12));
    display: flex;
    flex-direction: column;
    animation: dialogIn 0.25s var(--ease-out);
  }

  .dialog-header {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-5) var(--space-5) var(--space-3);
    flex-wrap: wrap;
  }

  .dialog-title {
    font-size: 17px;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .dialog-platform-label {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-tertiary);
    background: var(--surface);
    padding: 2px var(--space-2);
    border-radius: var(--radius-sm);
  }

  .dialog-close {
    margin-left: auto;
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    color: var(--text-tertiary);
    transition: all var(--duration-fast) var(--ease);
  }

  .dialog-close:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .dialog-body {
    padding: 0 var(--space-5) var(--space-5);
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .comparison-item {
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    border: 1px solid var(--separator);
    box-shadow: var(--glass-edge-shadow);
  }

  .comparison-item.recommended {
    border-color: var(--system-accent);
    box-shadow: var(--glass-edge-shadow), 0 0 0 1px rgba(0, 122, 255, 0.1);
  }

  .comparison-header {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-bottom: var(--space-2);
  }

  .comparison-icon {
    width: 28px;
    height: 28px;
    border-radius: var(--radius-sm);
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .comparison-title {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex: 1;
    min-width: 0;
    flex-wrap: wrap;
  }

  .comparison-name {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .recommendation-badge {
    font-size: 10px;
    font-weight: 600;
    padding: 2px 8px;
    border-radius: 10px;
    color: var(--system-accent);
    background: var(--system-accent-subtle);
  }

  .comparison-cost {
    font-size: 11px;
    font-weight: 600;
    padding: 1px 6px;
    border-radius: 3px;
    flex-shrink: 0;
  }

  .comparison-description {
    font-size: 13px;
    color: var(--text-secondary);
    line-height: 1.5;
    margin-bottom: var(--space-2);
  }

  .comparison-link {
    font-size: 12px;
    font-weight: 500;
    color: var(--system-accent);
    text-decoration: none;
  }

  .comparison-link:hover {
    text-decoration: underline;
  }

  /* --- Downloads Section --- */

  .input-hint {
    font-size: 11px;
    color: var(--text-tertiary);
    margin-top: var(--space-1);
    word-break: break-all;
  }

  .toggle-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
  }

  .toggle-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
  }

  .toggle-description {
    font-size: 12px;
    color: var(--text-tertiary);
    line-height: 1.4;
  }

  .toggle-switch {
    position: relative;
    width: 42px;
    height: 24px;
    background: var(--separator-opaque);
    border: none;
    border-radius: 12px;
    cursor: pointer;
    flex-shrink: 0;
    transition: background var(--duration-fast) var(--ease);
    padding: 0;
  }

  .toggle-switch:hover {
    background: var(--surface-active);
  }

  .toggle-switch.toggle-on {
    background: var(--system-accent);
  }

  .toggle-switch.toggle-on:hover {
    background: var(--system-accent-hover);
  }

  .toggle-switch:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .toggle-thumb {
    position: absolute;
    top: 3px;
    left: 3px;
    width: 18px;
    height: 18px;
    background: #fff;
    border-radius: 50%;
    transition: transform var(--duration-fast) var(--ease);
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.2);
  }

  .toggle-on .toggle-thumb {
    transform: translateX(18px);
  }

  /* --- Archives Management --- */

  .archives-summary {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    width: 100%;
  }

  .archives-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .archives-stats {
    font-size: 12px;
    color: var(--text-tertiary);
  }

  .archives-actions {
    display: flex;
    gap: var(--space-2);
    flex-shrink: 0;
  }

  .archives-empty {
    font-size: 13px;
    color: var(--text-tertiary);
    font-style: italic;
  }

  .archive-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .archive-info {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
    flex: 1;
  }

  .archive-name {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .archive-meta {
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .btn-delete-sm {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    padding: 0;
    background: transparent;
    color: var(--text-tertiary);
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    cursor: pointer;
    flex-shrink: 0;
    transition: all var(--duration-fast) var(--ease);
  }

  .btn-delete-sm:hover {
    color: var(--red);
    background: rgba(255, 69, 58, 0.1);
    border-color: rgba(255, 69, 58, 0.2);
  }

  .btn-delete-sm:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-danger {
    padding: var(--space-1) var(--space-3);
    background: transparent;
    color: var(--red);
    font-size: 13px;
    font-weight: 500;
    border: 1px solid rgba(255, 69, 58, 0.3);
    border-radius: var(--radius-sm);
    white-space: nowrap;
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
  }

  .btn-danger:hover {
    background: rgba(255, 69, 58, 0.1);
    border-color: rgba(255, 69, 58, 0.5);
  }

  .btn-danger:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .spinner-xs {
    display: inline-block;
    width: 12px;
    height: 12px;
    border: 2px solid var(--separator-opaque);
    border-top-color: var(--text-tertiary);
    border-radius: 50%;
    animation: spin 0.75s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  @keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  @keyframes dialogIn {
    from { transform: scale(0.95); opacity: 0; }
    to { transform: scale(1); opacity: 1; }
  }

  .about-links {
    display: flex;
    gap: var(--space-3);
  }

  .version-row-right {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .btn-update-ready {
    padding: 2px var(--space-3);
    background: var(--green);
    color: #fff;
    font-size: 12px;
    font-weight: 600;
    border: none;
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: opacity var(--duration-fast) var(--ease);
  }

  .btn-update-ready:hover {
    opacity: 0.85;
  }

  /* --- Mod Tools --- */

  .btn-sm {
    padding: 2px var(--space-2);
    font-size: 12px;
  }

  .tool-license {
    display: inline-block;
    font-size: 10px;
    font-weight: 600;
    color: var(--text-tertiary);
    background: var(--surface-hover);
    padding: 0 4px;
    border-radius: 3px;
    margin-left: 4px;
    vertical-align: middle;
  }

  .tool-wine-note {
    font-size: 11px;
    color: var(--yellow);
    opacity: 0.8;
  }
</style>
