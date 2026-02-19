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
    color: string;
    bg: string;
    icon: string; // SVG icon key
    recommendation?: string;
  }

  const layers: LayerInfo[] = [
    { name: "CrossOver", url: "https://www.codeweavers.com/crossover", description: "Commercial Wine wrapper with excellent compatibility and support. Best plug-and-play experience. CodeWeavers funds Wine development.", platforms: ["macOS", "Linux"], cost: "Paid ($74)", color: "#c850c0", bg: "rgba(200, 80, 192, 0.14)", icon: "crossover", recommendation: "Best overall compatibility" },
    { name: "Moonshine", url: "https://github.com/ybmeng/moonshine", description: "Free, maintained fork of Whisky with Wine Staging 11.2, OpenGL 3.2+ support, and macOS 26 compatibility.", platforms: ["macOS"], cost: "Free", color: "#bf5af2", bg: "rgba(191, 90, 242, 0.14)", icon: "moonshine", recommendation: "Best free option for macOS" },
    { name: "Heroic", url: "https://heroicgameslauncher.com/", description: "Open-source game launcher for GOG and Epic Games. Bundles Wine/Proton for easy setup.", platforms: ["macOS", "Linux"], cost: "Free", color: "#0a84ff", bg: "rgba(10, 132, 255, 0.14)", icon: "heroic" },
    { name: "Whisky", url: "https://getwhisky.app/", description: "Archived (May 2025). Developer recommended CrossOver instead. Moonshine is the maintained fork.", platforms: ["macOS"], cost: "Free", color: "#e8a317", bg: "rgba(232, 163, 23, 0.14)", icon: "whisky" },
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

  const platformExclusive = $derived(
    isMac
      ? layers.filter(l => !l.recommendation && l.platforms.length === 1 && l.platforms[0] === "macOS")
      : layers.filter(l => !l.recommendation && l.platforms.length === 1 && l.platforms[0] === "Linux")
  );

  const crossPlatform = $derived(
    layers.filter(l => !l.recommendation && l.platforms.length > 1)
  );

  // For comparison dialog
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

{#snippet layerIcon(icon: string)}
  {#if icon === "crossover"}
    <svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
      <rect x="4" y="4" width="8" height="8" rx="1" transform="rotate(45 8 8)" />
      <rect x="8" y="8" width="8" height="8" rx="1" transform="rotate(45 12 12)" />
    </svg>
  {:else if icon === "whisky"}
    <svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
      <path d="M4.5 5h11l-1.5 11a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L4.5 5z" />
      <path d="M6.5 11h7" />
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

    <!-- Platform-exclusive -->
    {#if platformExclusive.length > 0}
      <div class="layers-group">
        <span class="layers-group-label">{isMac ? "macOS" : "Linux"} Only</span>
        <div class="section-card">
          {#each platformExclusive as layer, i}
            {#if i > 0}<div class="card-divider"></div>{/if}
            {@render layerRow(layer, false)}
          {/each}
        </div>
      </div>
    {/if}

    <!-- Cross-platform -->
    {#if crossPlatform.length > 0}
      <div class="layers-group">
        <span class="layers-group-label">Cross-Platform</span>
        <div class="section-card">
          {#each crossPlatform as layer, i}
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

  @keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  @keyframes dialogIn {
    from { transform: scale(0.95); opacity: 0; }
    to { transform: scale(1); opacity: 1; }
  }
</style>
