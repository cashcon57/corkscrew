<script lang="ts">
  import { onMount } from "svelte";
  import {
    checkSkse,
    checkSkyrimVersion,
    checkSkseCompatibility,
    getSkseDownloadUrl,
    installSkseFromArchive,
  } from "$lib/api";
  import { open as dialogOpen } from "@tauri-apps/plugin-dialog";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import type { SkseStatus, DowngradeStatus, SkseCompatibility } from "$lib/types";

  interface Props {
    gameId: string;
    bottleName: string;
  }

  let { gameId, bottleName }: Props = $props();

  let skseStatus = $state<SkseStatus | null>(null);
  let gameVersion = $state<DowngradeStatus | null>(null);
  let compatibility = $state<SkseCompatibility | null>(null);
  let loading = $state(true);
  let installing = $state(false);
  let error = $state<string | null>(null);

  onMount(() => {
    runChecks();
  });

  async function runChecks() {
    loading = true;
    error = null;

    try {
      const [skse, version, compat] = await Promise.all([
        checkSkse(gameId, bottleName),
        checkSkyrimVersion(gameId, bottleName),
        checkSkseCompatibility(gameId, bottleName),
      ]);

      skseStatus = skse;
      gameVersion = version;
      compatibility = compat;
    } catch (e: unknown) {
      error = `Compatibility check failed: ${e}`;
    } finally {
      loading = false;
    }
  }

  async function handleDownloadSkse() {
    try {
      const url = await getSkseDownloadUrl();
      await openUrl(url);
    } catch {
      // fallback
      await openUrl("https://skse.silverlock.org/");
    }
  }

  async function handleInstallFromArchive() {
    const selected = await dialogOpen({
      title: "Select SKSE Archive",
      filters: [{ name: "Archives", extensions: ["7z", "zip"] }],
    });

    if (!selected) return;

    installing = true;
    try {
      const archivePath = selected as string;
      await installSkseFromArchive(gameId, bottleName, archivePath);
      // Re-run all checks after installation
      await runChecks();
    } catch (e: unknown) {
      error = `SKSE installation failed: ${e}`;
    } finally {
      installing = false;
    }
  }
</script>

<div class="compat-panel">
  <h4 class="compat-title">
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
    </svg>
    Pre-Install Compatibility Check
  </h4>

  {#if loading}
    <div class="compat-loading">
      <div class="compat-spinner"></div>
      <span>Running compatibility checks...</span>
    </div>
  {:else if error}
    <div class="compat-row compat-error">
      <div class="compat-icon">
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="10" />
          <line x1="15" y1="9" x2="9" y2="15" />
          <line x1="9" y1="9" x2="15" y2="15" />
        </svg>
      </div>
      <div class="compat-text">{error}</div>
    </div>
  {:else}
    <!-- Game Version Check -->
    {#if gameVersion}
      <div class="compat-row" class:compat-ok={true}>
        <div class="compat-icon compat-icon-ok">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M20 6L9 17l-5-5" />
          </svg>
        </div>
        <div class="compat-text">
          <span class="compat-label">Game Version:</span>
          <span class="compat-value">{gameVersion.current_version}</span>
        </div>
      </div>
    {/if}

    <!-- SKSE Status -->
    {#if skseStatus}
      <div
        class="compat-row"
        class:compat-ok={skseStatus.installed}
        class:compat-error={!skseStatus.installed}
      >
        <div class="compat-icon" class:compat-icon-ok={skseStatus.installed} class:compat-icon-error={!skseStatus.installed}>
          {#if skseStatus.installed}
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M20 6L9 17l-5-5" />
            </svg>
          {:else}
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="12" cy="12" r="10" />
              <line x1="15" y1="9" x2="9" y2="15" />
              <line x1="9" y1="9" x2="15" y2="15" />
            </svg>
          {/if}
        </div>
        <div class="compat-text">
          {#if skseStatus.installed}
            <span class="compat-label">SKSE:</span>
            <span class="compat-value">
              {skseStatus.version ? `v${skseStatus.version}` : "Installed (version unknown)"}
            </span>
          {:else}
            <span class="compat-label">SKSE:</span>
            <span class="compat-value">Not installed</span>
          {/if}
        </div>
      </div>
    {/if}

    <!-- Compatibility Verdict -->
    {#if compatibility}
      <div
        class="compat-row"
        class:compat-ok={compatibility.severity === "ok"}
        class:compat-warn={compatibility.severity === "warning"}
        class:compat-error={compatibility.severity === "error"}
      >
        <div
          class="compat-icon"
          class:compat-icon-ok={compatibility.severity === "ok"}
          class:compat-icon-warn={compatibility.severity === "warning"}
          class:compat-icon-error={compatibility.severity === "error"}
        >
          {#if compatibility.severity === "ok"}
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M20 6L9 17l-5-5" />
            </svg>
          {:else if compatibility.severity === "warning"}
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
              <line x1="12" y1="9" x2="12" y2="13" />
              <line x1="12" y1="17" x2="12.01" y2="17" />
            </svg>
          {:else}
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="12" cy="12" r="10" />
              <line x1="15" y1="9" x2="9" y2="15" />
              <line x1="9" y1="9" x2="15" y2="15" />
            </svg>
          {/if}
        </div>
        <div class="compat-text">
          <span class="compat-message">{compatibility.message}</span>
        </div>
      </div>
    {/if}

    <!-- Remediation Actions -->
    {#if compatibility && !compatibility.compatible}
      <div class="compat-actions">
        {#if !skseStatus?.installed || compatibility.severity === "error"}
          <button class="compat-btn compat-btn-secondary" onclick={handleDownloadSkse}>
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
              <polyline points="15 3 21 3 21 9" />
              <line x1="10" y1="14" x2="21" y2="3" />
            </svg>
            Download SKSE
          </button>
          <button
            class="compat-btn compat-btn-primary"
            onclick={handleInstallFromArchive}
            disabled={installing}
          >
            {#if installing}
              <div class="compat-spinner-sm"></div>
              Installing...
            {:else}
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                <polyline points="7 10 12 15 17 10" />
                <line x1="12" y1="15" x2="12" y2="3" />
              </svg>
              Install from Archive
            {/if}
          </button>
        {/if}
      </div>
    {/if}

    <!-- Summary Footer -->
    <div class="compat-footer">
      {#if compatibility?.compatible}
        <span class="compat-footer-ok">All checks passed</span>
      {:else if compatibility}
        <span class="compat-footer-issue">Action required before install</span>
      {/if}
    </div>
  {/if}
</div>

<style>
  .compat-panel {
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-lg);
    overflow: hidden;
    box-shadow: var(--glass-refraction), var(--glass-edge-shadow);
  }

  .compat-title {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    background: var(--surface-subtle);
    border-bottom: 1px solid var(--separator);
  }

  .compat-title svg {
    color: var(--system-accent);
  }

  .compat-loading {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-4);
    font-size: 13px;
    color: var(--text-tertiary);
  }

  .compat-spinner {
    width: 16px;
    height: 16px;
    border: 2px solid var(--separator);
    border-top-color: var(--system-accent);
    border-radius: 50%;
    animation: compat-spin 0.9s cubic-bezier(0.4, 0, 0.2, 1) infinite;
  }

  .compat-spinner-sm {
    width: 12px;
    height: 12px;
    border: 2px solid rgba(255, 255, 255, 0.3);
    border-top-color: #fff;
    border-radius: 50%;
    animation: compat-spin 0.9s cubic-bezier(0.4, 0, 0.2, 1) infinite;
  }

  @keyframes compat-spin { to { transform: rotate(360deg); } }

  .compat-row {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--separator);
  }

  .compat-row:last-of-type {
    border-bottom: none;
  }

  .compat-icon {
    flex-shrink: 0;
    width: 20px;
    height: 20px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 50%;
    margin-top: 1px;
  }

  .compat-icon-ok {
    color: var(--green);
    background: var(--green-subtle, rgba(52, 199, 89, 0.12));
  }

  .compat-icon-warn {
    color: var(--orange);
    background: var(--orange-subtle, rgba(255, 159, 10, 0.12));
  }

  .compat-icon-error {
    color: var(--red);
    background: var(--red-subtle, rgba(255, 69, 58, 0.12));
  }

  .compat-text {
    flex: 1;
    font-size: 13px;
    line-height: 1.4;
    color: var(--text-secondary);
  }

  .compat-label {
    font-weight: 600;
    color: var(--text-primary);
    margin-right: var(--space-1);
  }

  .compat-value {
    color: var(--text-secondary);
  }

  .compat-message {
    color: var(--text-secondary);
    line-height: 1.5;
  }

  .compat-actions {
    display: flex;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-top: 1px solid var(--separator);
  }

  .compat-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: var(--space-1) var(--space-3);
    font-size: 12px;
    font-weight: 600;
    border-radius: var(--radius);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
    white-space: nowrap;
  }

  .compat-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .compat-btn-primary {
    background: var(--system-accent);
    color: #fff;
  }

  .compat-btn-primary:hover:not(:disabled) {
    filter: brightness(1.1);
  }

  .compat-btn-secondary {
    background: var(--surface-hover);
    color: var(--text-primary);
    border: 1px solid var(--separator);
  }

  .compat-btn-secondary:hover {
    background: var(--surface-active);
  }

  .compat-footer {
    padding: var(--space-2) var(--space-4);
    border-top: 1px solid var(--separator);
    background: var(--surface-subtle);
    font-size: 12px;
    font-weight: 500;
  }

  .compat-footer-ok {
    color: var(--green);
  }

  .compat-footer-issue {
    color: var(--orange);
  }
</style>
