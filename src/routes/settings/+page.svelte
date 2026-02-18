<script lang="ts">
  import { onMount } from "svelte";
  import { getConfig, setConfigValue } from "$lib/api";
  import { config, showError, showSuccess } from "$lib/stores";
  import type { AppConfig } from "$lib/types";

  let apiKey = $state("");
  let downloadDir = $state("");
  let savingApiKey = $state(false);
  let savingDownloadDir = $state(false);
  let showApiKey = $state(false);

  onMount(async () => {
    try {
      const cfg = await getConfig();
      config.set(cfg);
      apiKey = cfg.nexus_api_key ?? "";
      downloadDir = cfg.download_dir ?? "";
    } catch (e: any) {
      showError(`Failed to load config: ${e}`);
    }
  });

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
        <span class="row-label">License</span>
        <span class="row-value">GPL-3.0-or-later</span>
      </div>
      <div class="card-divider"></div>
      <div class="card-row about-row">
        <span class="row-label">Platform</span>
        <span class="row-value">macOS / Linux</span>
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
</style>
