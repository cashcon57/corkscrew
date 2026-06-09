<script lang="ts">
  import { goto } from "$app/navigation";
  import {
    setNativeMode,
    getParalivesBepInExStatus,
    installParalivesBepInEx,
    uninstallParalivesBepInEx,
    type ParalivesBepInExStatus,
  } from "$lib/api";
  import { nativeMode, selectedGame } from "$lib/stores";
  import { onMount } from "svelte";

  // ---------------------------------------------------------------------------
  // Native mode toggle
  // ---------------------------------------------------------------------------

  async function disableNativeMode() {
    await setNativeMode(false).catch((err) => console.error("setNativeMode(false) failed:", err));
    nativeMode.set(false);
    goto("/");
  }

  // ---------------------------------------------------------------------------
  // BepInEx state machine
  // ---------------------------------------------------------------------------

  type BepInExState = "loading" | "idle" | "consent" | "installing" | "installed" | "error";

  let bepinexState = $state<BepInExState>("loading");
  let bepinexStatus = $state<ParalivesBepInExStatus | null>(null);
  let bepinexError = $state<string | null>(null);
  let consentChecked = $state(false);
  let showUninstallConfirm = $state(false);

  // Resolve game install dir and bundle path from the selected game.
  let gameInstallDir = $derived.by(() => {
    const g = $selectedGame;
    if (!g || g.game_id !== "paralives_native") return null;
    if (g.runtime.runtime !== "native") return null;
    const bundlePath: string = g.runtime.app_bundle_path;
    if (!bundlePath) return null;
    // install dir = parent of .app bundle
    const lastSlash = bundlePath.lastIndexOf("/");
    return lastSlash >= 0 ? bundlePath.slice(0, lastSlash) : null;
  });

  let appBundlePath = $derived.by(() => {
    const g = $selectedGame;
    if (!g || g.game_id !== "paralives_native") return null;
    if (g.runtime.runtime !== "native") return null;
    return g.runtime.app_bundle_path ?? null;
  });

  async function loadBepInExStatus() {
    if (!gameInstallDir) {
      bepinexState = "idle";
      return;
    }
    try {
      bepinexStatus = await getParalivesBepInExStatus(gameInstallDir);
      bepinexState = bepinexStatus.installed && bepinexStatus.mac_supported
        ? "installed"
        : "idle";
    } catch (err) {
      console.error("getParalivesBepInExStatus failed:", err);
      bepinexError = String(err);
      bepinexState = "error";
    }
  }

  onMount(() => {
    loadBepInExStatus();
  });

  // Reload status when game selection changes.
  $effect(() => {
    // Depend on gameInstallDir so this re-runs when the game changes.
    const _ = gameInstallDir;
    bepinexState = "loading";
    bepinexStatus = null;
    bepinexError = null;
    consentChecked = false;
    showUninstallConfirm = false;
    loadBepInExStatus();
  });

  // ---------------------------------------------------------------------------
  // Actions
  // ---------------------------------------------------------------------------

  function beginConsent() {
    consentChecked = false;
    bepinexState = "consent";
  }

  function cancelConsent() {
    bepinexState = "idle";
  }

  async function confirmInstall() {
    if (!gameInstallDir || !appBundlePath) return;
    bepinexState = "installing";
    bepinexError = null;
    try {
      await installParalivesBepInEx(gameInstallDir, appBundlePath);
      await loadBepInExStatus();
    } catch (err) {
      console.error("installParalivesBepInEx failed:", err);
      bepinexError = String(err);
      bepinexState = "error";
    }
  }

  async function confirmUninstall() {
    if (!gameInstallDir) return;
    showUninstallConfirm = false;
    bepinexState = "installing"; // reuse spinner
    bepinexError = null;
    try {
      await uninstallParalivesBepInEx(gameInstallDir);
      await loadBepInExStatus();
    } catch (err) {
      console.error("uninstallParalivesBepInEx failed:", err);
      bepinexError = String(err);
      bepinexState = "error";
    }
  }
</script>

<div class="page">
  <h1 class="m5-gradient-text">Native Settings</h1>
  <p class="subtitle">Native macOS game management and script-mod runtimes.</p>

  <!-- ── Disable Native Mode ── -->
  <div class="native-glass-card section">
    <h3>Disable Native Mode</h3>
    <p>Return to Wine/CrossOver workflow. Your installed mods are preserved.</p>
    <button class="danger" onclick={disableNativeMode}>Exit Native Mode</button>
  </div>

  <!-- ── BepInEx script mods (experimental) ── -->
  <div class="native-glass-card section">
    <div class="section-header">
      <h3>BepInEx script mods <span class="badge experimental">Experimental</span></h3>
      <p class="section-desc">
        BepInEx 6.x IL2CPP macOS ARM64 is the community script-mod runtime for Paralives.
        Installing it places loader files in your Paralives directory and removes the
        Apple Developer ID signature from <code>Paralives.app</code>.
      </p>
    </div>

    {#if !gameInstallDir}
      <!-- No Paralives game selected -->
      <div class="status-row notice">
        <span class="status-icon">ℹ</span>
        <span>Select <strong>Paralives</strong> in the game picker to manage BepInEx.</span>
      </div>

    {:else if bepinexState === "loading"}
      <div class="status-row">
        <span class="spinner"></span>
        <span>Checking BepInEx status…</span>
      </div>

    {:else if bepinexState === "idle"}
      <!-- Not installed -->
      {#if bepinexStatus && !bepinexStatus.mac_supported && bepinexStatus.version}
        <div class="status-row warn">
          <span class="status-icon">⚠</span>
          <span>
            BepInEx is installed but not ARM64-compatible (likely BepInEx 5.x or a Windows build).
            Re-install to get the correct ARM64 IL2CPP build.
          </span>
        </div>
      {:else}
        <div class="status-row notice">
          <span class="status-icon">○</span>
          <span>BepInEx is <strong>not installed</strong>.</span>
        </div>
      {/if}
      <button class="primary" onclick={beginConsent}>Install BepInEx</button>

    {:else if bepinexState === "consent"}
      <!-- Consent dialog -->
      <div class="consent-card">
        <h4>Before installing BepInEx for Paralives, please understand:</h4>
        <ol class="consent-list">
          <li>
            BepInEx enables .dll script mods. Installing it places loader files
            in your Paralives install directory AND removes the Apple Developer
            ID signature from <code>Paralives.app</code> to permit code injection.
          </li>
          <li>
            Paralives Studio does <strong>NOT</strong> endorse BepInEx. Paralives game updates
            may break BepInEx until the macOS upstream catches up. BepInEx 6.x
            IL2CPP for Apple Silicon is <strong>EXPERIMENTAL</strong>.
          </li>
          <li>
            macOS Gatekeeper will warn you the first time you launch Paralives
            after this change. Subsequent launches work normally.
          </li>
          <li>
            To restore the original Paralives.app: use Steam's "Verify integrity
            of game files" or reinstall the game. Corkscrew tracks a snapshot
            you can also use to roll back.
          </li>
        </ol>

        <label class="consent-checkbox">
          <input type="checkbox" bind:checked={consentChecked} />
          I understand BepInEx is experimental and modifies my Paralives.app signature.
        </label>

        <div class="consent-actions">
          <button class="secondary" onclick={cancelConsent}>Cancel</button>
          <button
            class="primary"
            disabled={!consentChecked}
            onclick={confirmInstall}
          >
            Install BepInEx
          </button>
        </div>
      </div>

    {:else if bepinexState === "installing"}
      <div class="status-row">
        <span class="spinner"></span>
        <span>Installing BepInEx… (downloading from GitHub, this may take a moment)</span>
      </div>

    {:else if bepinexState === "installed"}
      <!-- Installed and mac_supported -->
      <div class="status-row success">
        <span class="status-icon">✓</span>
        <div>
          <span>BepInEx is <strong>installed</strong> and ARM64-ready.</span>
          {#if bepinexStatus?.version}
            <span class="version-badge">{bepinexStatus.version}</span>
          {/if}
        </div>
      </div>

      {#if !showUninstallConfirm}
        <button class="secondary danger-outline" onclick={() => (showUninstallConfirm = true)}>
          Uninstall BepInEx
        </button>
      {:else}
        <div class="uninstall-confirm">
          <p>
            Remove BepInEx loader files from your Paralives install directory?
            Your BepInEx plugin mods will no longer load. The .app signature
            cannot be restored by Corkscrew — use Steam "Verify integrity" to restore it.
          </p>
          <div class="consent-actions">
            <button class="secondary" onclick={() => (showUninstallConfirm = false)}>
              Cancel
            </button>
            <button class="danger" onclick={confirmUninstall}>Remove BepInEx</button>
          </div>
        </div>
      {/if}

    {:else if bepinexState === "error"}
      <div class="status-row error">
        <span class="status-icon">✗</span>
        <span>{bepinexError}</span>
      </div>
      <div class="error-actions">
        <button class="secondary" onclick={loadBepInExStatus}>Retry</button>
        <button class="secondary" onclick={() => { bepinexState = "idle"; bepinexError = null; }}>
          Cancel
        </button>
      </div>
    {/if}
  </div>
</div>

<style>
  .page {
    max-width: 720px;
    margin: 0 auto;
    padding: 32px 24px;
  }

  h1 {
    font-size: 36px;
    font-weight: 700;
    margin: 0 0 8px;
  }

  .subtitle {
    color: var(--text-secondary);
    margin: 0 0 24px;
  }

  .native-glass-card {
    padding: 24px;
  }

  .section {
    margin-bottom: 16px;
  }

  .section-header {
    margin-bottom: 16px;
  }

  h3 {
    font-size: 16px;
    font-weight: 600;
    margin: 0 0 8px;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  h4 {
    font-size: 14px;
    font-weight: 600;
    margin: 0 0 12px;
  }

  p,
  .section-desc {
    color: var(--text-secondary);
    margin: 0 0 16px;
    font-size: 13px;
    line-height: 1.5;
  }

  code {
    font-family: monospace;
    font-size: 12px;
    background: rgba(255, 255, 255, 0.08);
    border-radius: 4px;
    padding: 1px 5px;
  }

  /* Badges */
  .badge {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 2px 7px;
    border-radius: 10px;
  }

  .badge.experimental {
    background: rgba(255, 160, 0, 0.18);
    color: #ffa000;
    border: 1px solid rgba(255, 160, 0, 0.35);
  }

  /* Status rows */
  .status-row {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 16px;
    font-size: 13px;
    color: var(--text-secondary);
  }

  .status-row.success {
    color: var(--green, #4caf50);
  }

  .status-row.warn {
    color: #ffa000;
  }

  .status-row.error {
    color: var(--red, #ef5350);
  }

  .status-row.notice {
    color: var(--text-secondary);
  }

  .status-icon {
    font-size: 16px;
    flex-shrink: 0;
  }

  .version-badge {
    font-size: 11px;
    font-family: monospace;
    background: rgba(255, 255, 255, 0.08);
    border-radius: 4px;
    padding: 1px 6px;
    margin-left: 6px;
  }

  /* Spinner */
  .spinner {
    display: inline-block;
    width: 14px;
    height: 14px;
    border: 2px solid rgba(255, 255, 255, 0.15);
    border-top-color: var(--accent, #7c4dff);
    border-radius: 50%;
    animation: spin 0.75s linear infinite;
    flex-shrink: 0;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  /* Consent card */
  .consent-card {
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid rgba(255, 160, 0, 0.25);
    border-radius: 10px;
    padding: 20px;
  }

  .consent-list {
    padding-left: 20px;
    margin: 0 0 16px;
    color: var(--text-secondary);
    font-size: 13px;
    line-height: 1.6;
  }

  .consent-list li {
    margin-bottom: 10px;
  }

  .consent-checkbox {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    font-size: 13px;
    color: var(--text-primary);
    cursor: pointer;
    margin-bottom: 18px;
    line-height: 1.4;
  }

  .consent-checkbox input[type="checkbox"] {
    margin-top: 2px;
    flex-shrink: 0;
  }

  .consent-actions {
    display: flex;
    gap: 10px;
    justify-content: flex-end;
  }

  /* Uninstall confirm */
  .uninstall-confirm {
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid rgba(239, 83, 80, 0.25);
    border-radius: 10px;
    padding: 16px;
  }

  .uninstall-confirm p {
    margin-bottom: 14px;
  }

  /* Error actions */
  .error-actions {
    display: flex;
    gap: 10px;
  }

  /* Buttons */
  button {
    border: none;
    border-radius: 8px;
    padding: 9px 16px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: opacity 0.15s;
  }

  button:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  button:not(:disabled):hover {
    opacity: 0.85;
  }

  .primary {
    background: var(--accent, #7c4dff);
    color: white;
  }

  .secondary {
    background: rgba(255, 255, 255, 0.1);
    color: var(--text-primary);
  }

  .danger {
    background: var(--red, #ef5350);
    color: white;
  }

  .danger-outline {
    background: transparent;
    border: 1px solid rgba(239, 83, 80, 0.5);
    color: var(--red, #ef5350);
  }
</style>
