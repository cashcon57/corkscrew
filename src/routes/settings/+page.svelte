<script lang="ts">
  import { onMount } from "svelte";
  import { getConfig, setConfigValue, checkSkse, installSkse } from "$lib/api";
  import { config, showError, showSuccess, selectedGame, skseStatus } from "$lib/stores";
  import type { AppConfig } from "$lib/types";
  import ThemeToggle from "$lib/components/ThemeToggle.svelte";

  let apiKey = $state("");
  let downloadDir = $state("");
  let savingApiKey = $state(false);
  let savingDownloadDir = $state(false);
  let showApiKey = $state(false);
  let installingSkse = $state(false);
  let showComparisonDialog = $state(false);

  const game = $derived($selectedGame);
  const skse = $derived($skseStatus);
  const isSkyrim = $derived(game?.game_id === "skyrimse");

  // Detect platform for comparison dialog
  const isMac = typeof navigator !== "undefined" && navigator.platform?.startsWith("Mac");

  interface LayerInfo {
    name: string;
    url: string;
    description: string;
    platforms: string[];
    cost: string;
    recommendation?: string;
  }

  const layers: LayerInfo[] = [
    { name: "CrossOver", url: "https://www.codeweavers.com/crossover", description: "Commercial Wine wrapper with excellent compatibility and support. Best plug-and-play experience.", platforms: ["macOS", "Linux"], cost: "Paid ($74)", recommendation: "Best for most users on macOS" },
    { name: "Whisky", url: "https://getwhisky.app/", description: "Free, native macOS wrapper with a clean UI. Uses Apple Game Porting Toolkit under the hood.", platforms: ["macOS"], cost: "Free" },
    { name: "Heroic", url: "https://heroicgameslauncher.com/", description: "Open-source game launcher for GOG and Epic Games. Bundles Wine/Proton for easy setup.", platforms: ["macOS", "Linux"], cost: "Free" },
    { name: "Moonshine", url: "https://github.com/nicosemp/Moonshine", description: "Lightweight macOS Wine prefix manager. Simple and focused.", platforms: ["macOS"], cost: "Free" },
    { name: "Mythic", url: "https://getmythic.app/", description: "Native macOS game launcher for Epic Games with built-in Wine support.", platforms: ["macOS"], cost: "Free" },
    { name: "Lutris", url: "https://lutris.net/", description: "Open-source gaming platform for Linux. Manages Wine, Proton, and native runners.", platforms: ["Linux"], cost: "Free", recommendation: "Best for Linux desktop users" },
    { name: "Proton / Steam", url: "https://store.steampowered.com/", description: "Valve's Wine fork built into Steam. Seamless for Steam library games.", platforms: ["Linux"], cost: "Free", recommendation: "Best for Steam Deck / SteamOS" },
    { name: "Bottles", url: "https://usebottles.com/", description: "Modern Linux app for creating and managing Wine prefixes with versioned runners.", platforms: ["Linux"], cost: "Free" },
    { name: "Wine", url: "https://www.winehq.org/", description: "The original compatibility layer. Manual setup, maximum flexibility.", platforms: ["macOS", "Linux"], cost: "Free" },
  ];

  const platformLayers = $derived(
    isMac
      ? layers.filter(l => l.platforms.includes("macOS"))
      : layers.filter(l => l.platforms.includes("Linux"))
  );

  onMount(async () => {
    try {
      const cfg = await getConfig();
      config.set(cfg);
      apiKey = cfg.nexus_api_key ?? "";
      downloadDir = cfg.download_dir ?? "";
    } catch (e: any) {
      showError(`Failed to load config: ${e}`);
    }

    // Check SKSE status if Skyrim is selected
    if (game && isSkyrim) {
      try {
        const status = await checkSkse(game.game_id, game.bottle_name);
        skseStatus.set(status);
      } catch { /* ignore */ }
    }
  });

  async function handleInstallSkse() {
    if (!game) return;
    installingSkse = true;
    try {
      const status = await installSkse(game.game_id, game.bottle_name);
      skseStatus.set(status);
      showSuccess("SKSE installed successfully");
    } catch (e: any) {
      showError(`Failed to install SKSE: ${e}`);
    } finally {
      installingSkse = false;
    }
  }

  async function saveApiKey() {
    savingApiKey = true;
    try {
      await setConfigValue("nexus_api_key", apiKey);
      showSuccess("API key saved");
    } catch (e: any) {
      showError(`Failed to save: ${e}`);
    } finally {
      savingApiKey = false;
    }
  }

  async function saveDownloadDir() {
    savingDownloadDir = true;
    try {
      await setConfigValue("download_dir", downloadDir);
      showSuccess("Download directory saved");
    } catch (e: any) {
      showError(`Failed to save: ${e}`);
    } finally {
      savingDownloadDir = false;
    }
  }
</script>

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
                class="btn-primary"
                onclick={handleInstallSkse}
                disabled={installingSkse}
                type="button"
              >
                {installingSkse ? "Installing..." : "Install SKSE"}
              </button>
            {/if}
          </div>
        </div>
      </div>
    </div>
  {/if}

  <!-- Nexus Mods -->
  <div class="section">
    <h2 class="section-title">Nexus Mods</h2>
    <div class="section-card">
      <div class="card-row">
        <div class="row-content full">
          <span class="row-description">
            Connect your Nexus Mods account to download mods directly.
            Get your API key from your
            <a
              href="https://www.nexusmods.com/users/myaccount?tab=api+access"
              target="_blank"
              rel="noopener noreferrer"
            >Nexus Mods account settings</a>.
          </span>
        </div>
      </div>
      <div class="card-divider"></div>
      <div class="card-row">
        <label class="row-label" for="api-key">API Key</label>
        <div class="row-control">
          <div class="input-with-actions">
            <input
              id="api-key"
              type={showApiKey ? "text" : "password"}
              bind:value={apiKey}
              placeholder="Enter your API key"
              class="settings-input"
            />
            <button
              class="btn-ghost"
              onclick={() => (showApiKey = !showApiKey)}
              type="button"
            >
              {showApiKey ? "Hide" : "Show"}
            </button>
            <button
              class="btn-primary"
              onclick={saveApiKey}
              disabled={savingApiKey}
              type="button"
            >
              {savingApiKey ? "Saving..." : "Save"}
            </button>
          </div>
        </div>
      </div>
    </div>
  </div>

  <!-- Downloads -->
  <div class="section">
    <h2 class="section-title">Downloads</h2>
    <div class="section-card">
      <div class="card-row">
        <label class="row-label" for="download-dir">Download Directory</label>
        <div class="row-control">
          <div class="input-with-actions">
            <input
              id="download-dir"
              type="text"
              bind:value={downloadDir}
              placeholder="~/.local/share/corkscrew/downloads"
              class="settings-input"
            />
            <button
              class="btn-primary"
              onclick={saveDownloadDir}
              disabled={savingDownloadDir}
              type="button"
            >
              {savingDownloadDir ? "Saving..." : "Save"}
            </button>
          </div>
        </div>
      </div>
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
      <div class="card-divider"></div>
      {#each layers as layer, i}
        {#if i > 0}
          <div class="card-divider"></div>
        {/if}
        <div class="card-row layer-row">
          <div class="layer-info">
            <div class="layer-name-row">
              <a
                href={layer.url}
                target="_blank"
                rel="noopener noreferrer"
                class="layer-name"
              >{layer.name}</a>
              <div class="layer-badges">
                {#each layer.platforms as platform}
                  <span class="platform-badge" class:mac={platform === "macOS"} class:linux={platform === "Linux"}>{platform}</span>
                {/each}
                {#if layer.cost !== "Free"}
                  <span class="cost-badge">{layer.cost}</span>
                {/if}
              </div>
            </div>
            <span class="layer-description">{layer.description}</span>
          </div>
        </div>
      {/each}
    </div>
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
                <span class="comparison-name">{layer.name}</span>
                {#if layer.recommendation}
                  <span class="recommendation-badge">{layer.recommendation}</span>
                {/if}
                <span class="comparison-cost">{layer.cost}</span>
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
        <span class="row-value">0.1.0</span>
      </div>
      <div class="card-divider"></div>
      <div class="card-row about-row">
        <span class="row-label">More Info & Credits</span>
        <button
          class="btn-link"
          onclick={() => { import('$lib/stores').then(m => m.currentPage.set('about')); }}
          type="button"
        >
          View About Page
        </button>
      </div>
    </div>
  </div>
</div>

<style>
  .settings-page {
    max-width: 620px;
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

  .layer-row {
    display: flex;
    flex-direction: column;
  }

  .layer-info {
    display: flex;
    flex-direction: column;
    gap: 4px;
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
    color: var(--system-accent);
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
    color: var(--text-tertiary);
    background: var(--surface-hover);
  }

  .layer-description {
    font-size: 12px;
    color: var(--text-tertiary);
    line-height: 1.4;
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
    box-shadow: var(--shadow-lg);
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
  }

  .comparison-item.recommended {
    border-color: var(--system-accent);
    box-shadow: 0 0 0 1px rgba(0, 122, 255, 0.1);
  }

  .comparison-header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-2);
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
    font-size: 12px;
    color: var(--text-tertiary);
    margin-left: auto;
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

  @keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  @keyframes dialogIn {
    from { transform: scale(0.95); opacity: 0; }
    to { transform: scale(1); opacity: 1; }
  }
</style>
