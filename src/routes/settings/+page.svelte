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

  const game = $derived($selectedGame);
  const skse = $derived($skseStatus);
  const isSkyrim = $derived(game?.game_id === "skyrimse");

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
    color: var(--accent);
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
    border-color: var(--accent);
  }

  .settings-input::placeholder {
    color: var(--text-tertiary);
  }

  /* --- Buttons --- */

  .btn-primary {
    padding: var(--space-1) var(--space-3);
    background: var(--accent);
    color: #fff;
    font-size: 13px;
    font-weight: 500;
    border: none;
    border-radius: var(--radius-sm);
    white-space: nowrap;
    transition: background var(--duration-fast) var(--ease);
  }

  .btn-primary:hover:not(:disabled) {
    background: var(--accent-hover);
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
    color: var(--accent);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    padding: 0;
  }

  .btn-link:hover {
    text-decoration: underline;
  }
</style>
