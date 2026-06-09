<script lang="ts">
  import { goto } from "$app/navigation";
  import { onMount } from "svelte";
  import {
    setNativeMode,
    getParalivesBepInExStatus,
    installParalivesBepInEx,
    uninstallParalivesBepInEx,
    rescanNativeGames,
    getStardewSmapiStatus,
    installStardewSmapi,
    uninstallStardewSmapi,
    getBg3seStatus,
    installBg3se,
    uninstallBg3se,
    type ParalivesBepInExStatus,
    type SmapiStatus,
    type Bg3seStatus,
  } from "$lib/api";
  import { nativeMode, selectedGame, games } from "$lib/stores";
  import { revealItemInDir, openUrl } from "@tauri-apps/plugin-opener";
  import { appDataDir } from "@tauri-apps/api/path";
  import type { DetectedGame } from "$lib/types";
  import NexusAccountSection from "$lib/components/NexusAccountSection.svelte";

  // ---------------------------------------------------------------------------
  // Native mode toggle
  // ---------------------------------------------------------------------------

  async function disableNativeMode() {
    await setNativeMode(false).catch((err) =>
      console.error("setNativeMode(false) failed:", err)
    );
    nativeMode.set(false);
    goto("/");
  }

  // ---------------------------------------------------------------------------
  // Rescan native games
  // ---------------------------------------------------------------------------

  let rescanning = $state(false);
  let rescanCount = $state<number | null>(null);
  let rescanError = $state<string | null>(null);

  async function handleRescan() {
    rescanning = true;
    rescanError = null;
    rescanCount = null;
    try {
      const candidates = await rescanNativeGames();
      rescanCount = candidates.length;
    } catch (err) {
      console.error("rescanNativeGames failed:", err);
      rescanError = String(err);
    } finally {
      rescanning = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Storage paths — native games from the games store
  // ---------------------------------------------------------------------------

  /** Detected native games for the storage paths section. */
  const nativeGames = $derived(
    $games.filter((g) => g.runtime.runtime === "native")
  );

  function openModDir(game: DetectedGame) {
    revealItemInDir(game.data_dir).catch((err) =>
      console.error("revealItemInDir failed:", err)
    );
  }

  // ---------------------------------------------------------------------------
  // BepInEx state machine (unchanged from original)
  // ---------------------------------------------------------------------------

  type BepInExState = "loading" | "idle" | "consent" | "installing" | "installed" | "error";

  let bepinexState = $state<BepInExState>("loading");
  let bepinexStatus = $state<ParalivesBepInExStatus | null>(null);
  let bepinexError = $state<string | null>(null);
  let consentChecked = $state(false);
  let showUninstallConfirm = $state(false);

  let gameInstallDir = $derived.by(() => {
    const g = $selectedGame;
    if (!g || g.game_id !== "paralives_native") return null;
    if (g.runtime.runtime !== "native") return null;
    const bundlePath: string = g.runtime.app_bundle_path;
    if (!bundlePath) return null;
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
      bepinexState =
        bepinexStatus.installed && bepinexStatus.mac_supported
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
    loadSmapiStatus();
    loadBg3seStatus();
  });

  $effect(() => {
    const _ = gameInstallDir;
    bepinexState = "loading";
    bepinexStatus = null;
    bepinexError = null;
    consentChecked = false;
    showUninstallConfirm = false;
    loadBepInExStatus();
  });

  // ---------------------------------------------------------------------------
  // BepInEx actions (unchanged)
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
    bepinexState = "installing";
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

  // ---------------------------------------------------------------------------
  // SMAPI (Stardew Valley) state machine
  // ---------------------------------------------------------------------------

  type SmapiUiState =
    | "loading"
    | "idle"
    | "consent"
    | "installing"
    | "uninstalling"
    | "installed"
    | "sandboxed"
    | "error";

  let smapiState = $state<SmapiUiState>("loading");
  let smapiStatus = $state<SmapiStatus | null>(null);
  let smapiError = $state<string | null>(null);
  let smapiConsentChecked = $state(false);
  let smapiShowUninstallConfirm = $state(false);

  /** Active Stardew Valley native game id, or null if not selected. */
  let stardewGameId = $derived.by(() => {
    const g = $selectedGame;
    if (!g || g.game_id !== "stardew_valley_native") return null;
    if (g.runtime.runtime !== "native") return null;
    return g.game_id;
  });

  async function loadSmapiStatus() {
    if (!stardewGameId) {
      smapiState = "idle";
      return;
    }
    try {
      smapiStatus = await getStardewSmapiStatus(stardewGameId);
      if (smapiStatus.sandboxed) {
        smapiState = "sandboxed";
      } else if (smapiStatus.installed) {
        smapiState = "installed";
      } else {
        smapiState = "idle";
      }
    } catch (err) {
      console.error("getStardewSmapiStatus failed:", err);
      smapiError = String(err);
      smapiState = "error";
    }
  }

  $effect(() => {
    const _ = stardewGameId;
    smapiState = "loading";
    smapiStatus = null;
    smapiError = null;
    smapiConsentChecked = false;
    smapiShowUninstallConfirm = false;
    loadSmapiStatus();
  });

  function beginSmapiConsent() {
    smapiConsentChecked = false;
    smapiState = "consent";
  }

  function cancelSmapiConsent() {
    smapiState = smapiStatus?.installed ? "installed" : "idle";
  }

  async function confirmSmapiInstall() {
    if (!stardewGameId) return;
    smapiState = "installing";
    smapiError = null;
    try {
      await installStardewSmapi(stardewGameId);
      await loadSmapiStatus();
    } catch (err) {
      console.error("installStardewSmapi failed:", err);
      smapiError = String(err);
      smapiState = "error";
    }
  }

  async function confirmSmapiUninstall() {
    if (!stardewGameId) return;
    smapiShowUninstallConfirm = false;
    smapiState = "uninstalling";
    smapiError = null;
    try {
      await uninstallStardewSmapi(stardewGameId);
      await loadSmapiStatus();
    } catch (err) {
      console.error("uninstallStardewSmapi failed:", err);
      smapiError = String(err);
      smapiState = "error";
    }
  }

  // ---------------------------------------------------------------------------
  // BG3SE (Baldur's Gate 3) state machine
  // ---------------------------------------------------------------------------

  type Bg3seUiState =
    | "loading"
    | "idle"
    | "installing"
    | "uninstalling"
    | "installed"
    | "windows_dll"
    | "manual_required"
    | "error";

  const BG3SE_MANUAL_URL = "https://github.com/Norbyte/bg3se";

  let bg3seState = $state<Bg3seUiState>("loading");
  let bg3seStatus = $state<Bg3seStatus | null>(null);
  let bg3seError = $state<string | null>(null);
  let bg3seShowUninstallConfirm = $state(false);

  /** `.app` bundle path for the active BG3 native game, or null. */
  let bg3AppBundle = $derived.by(() => {
    const g = $selectedGame;
    if (!g || g.game_id !== "baldurs_gate_3_native") return null;
    if (g.runtime.runtime !== "native") return null;
    return g.runtime.app_bundle_path ?? null;
  });

  async function loadBg3seStatus() {
    if (!bg3AppBundle) {
      bg3seState = "idle";
      return;
    }
    try {
      bg3seStatus = await getBg3seStatus(bg3AppBundle);
      if (bg3seStatus.installed && bg3seStatus.mac_supported) {
        bg3seState = "installed";
      } else if (!bg3seStatus.mac_supported) {
        bg3seState = "windows_dll";
      } else {
        bg3seState = "idle";
      }
    } catch (err) {
      console.error("getBg3seStatus failed:", err);
      bg3seError = String(err);
      bg3seState = "error";
    }
  }

  $effect(() => {
    const _ = bg3AppBundle;
    bg3seState = "loading";
    bg3seStatus = null;
    bg3seError = null;
    bg3seShowUninstallConfirm = false;
    loadBg3seStatus();
  });

  /**
   * Returns true when the backend error string indicates the install
   * pipeline is a research-blocker stub (BG3SE_INSTALL_BLOCKER in bg3se.rs).
   * The string starts with "BG3SE install is not yet supported".
   */
  function isBg3seBlockerError(msg: string): boolean {
    return (
      msg.includes("not yet supported") ||
      msg.toLowerCase().includes("verification pending") ||
      msg.toLowerCase().includes("install layout")
    );
  }

  async function installBg3seFlow() {
    if (!bg3AppBundle) return;
    bg3seState = "installing";
    bg3seError = null;
    try {
      await installBg3se(bg3AppBundle);
      await loadBg3seStatus();
    } catch (err) {
      const msg = String(err);
      console.error("installBg3se failed:", err);
      if (isBg3seBlockerError(msg)) {
        // Backend is currently a stub — surface a manual-install prompt
        // pointing at the upstream repo so the user can self-serve.
        bg3seState = "manual_required";
      } else {
        bg3seError = msg;
        bg3seState = "error";
      }
    }
  }

  async function confirmBg3seUninstall() {
    if (!bg3AppBundle) return;
    bg3seShowUninstallConfirm = false;
    bg3seState = "uninstalling";
    bg3seError = null;
    try {
      await uninstallBg3se(bg3AppBundle);
      await loadBg3seStatus();
    } catch (err) {
      const msg = String(err);
      console.error("uninstallBg3se failed:", err);
      if (isBg3seBlockerError(msg)) {
        bg3seState = "manual_required";
      } else {
        bg3seError = msg;
        bg3seState = "error";
      }
    }
  }

  function openBg3seManual() {
    openUrl(BG3SE_MANUAL_URL).catch((err) =>
      console.error("openUrl(BG3SE_MANUAL_URL) failed:", err)
    );
  }

  // ---------------------------------------------------------------------------
  // Corkscrew log directory
  // ---------------------------------------------------------------------------

  async function openLogDir() {
    // appDataDir() returns ~/Library/Application Support/corkscrew on macOS
    // (Tauri uses the bundle identifier to scope the directory).
    const dir = await appDataDir().catch((err) => {
      console.error("appDataDir() failed:", err);
      return null;
    });
    if (!dir) return;
    revealItemInDir(dir).catch((err) =>
      console.error("revealItemInDir (log dir) failed:", err)
    );
  }
</script>

<div class="page">
  <h1 class="m5-gradient-text">Native Settings</h1>
  <p class="subtitle">Native macOS game management and script-mod runtimes.</p>

  <!-- ── 1. Native Mode ── -->
  <h2 class="section-title">Native Mode</h2>
  <div class="native-glass-card section">
    <div class="card-inner">

      <div class="card-row toggle-row">
        <div class="row-info">
          <span class="row-label">Exit Native Mode</span>
          <span class="row-desc">Return to Wine/CrossOver workflow. Your installed mods are preserved.</span>
        </div>
        <button class="btn btn-danger" onclick={disableNativeMode} type="button">
          Exit Native Mode
        </button>
      </div>

      <div class="card-divider"></div>

      <div class="card-row">
        <div class="row-info">
          <span class="row-label">Visibility toggle</span>
          <span class="row-desc">
            Native mode is in active development. To hide the Native Mode toggle in the topbar,
            go to
            <button class="link-btn" onclick={() => goto("/settings")} type="button">
              Settings → About
            </button>
            and disable "Show Native Mode toggle (in development)".
          </span>
        </div>
      </div>

      <div class="card-divider"></div>

      <div class="card-row toggle-row">
        <div class="row-info">
          <span class="row-label">Rescan installed games</span>
          <span class="row-desc">
            Scan common locations (/Applications, ~/Applications, Steam) for supported native macOS games.
            {#if rescanCount !== null}
              Last scan found <strong>{rescanCount} candidate{rescanCount !== 1 ? "s" : ""}</strong>.
            {/if}
            {#if rescanError}
              <span class="inline-error">{rescanError}</span>
            {/if}
          </span>
        </div>
        <button
          class="btn btn-secondary"
          onclick={handleRescan}
          disabled={rescanning}
          type="button"
        >
          {#if rescanning}
            <span class="spinner"></span>
            Scanning…
          {:else}
            Rescan
          {/if}
        </button>
      </div>

    </div>
  </div>

  <!-- ── 2. Nexus Mods Account ── -->
  <NexusAccountSection variant="native" />

  <!-- ── 3. Storage Paths ── -->
  <h2 class="section-title">Storage Paths</h2>
  <div class="native-glass-card section">
    <div class="card-inner">

      {#if nativeGames.length === 0}
        <div class="card-row">
          <span class="row-desc muted">
            No native games detected. Use "Rescan installed games" above, or add a game manually from the sidebar.
          </span>
        </div>
      {:else}
        {#each nativeGames as game, i}
          {#if i > 0}
            <div class="card-divider"></div>
          {/if}
          <div class="card-row path-row">
            <div class="path-info">
              <span class="row-label">{game.display_name}</span>
              <code class="path-code">{game.data_dir}</code>
            </div>
            <button
              class="btn btn-secondary btn-sm"
              onclick={() => openModDir(game)}
              type="button"
            >
              Open in Finder
            </button>
          </div>
        {/each}
      {/if}

    </div>
  </div>

  <!-- ── 4. BepInEx (Paralives) ── -->
  <h2 class="section-title">BepInEx (Paralives)</h2>
  <div class="native-glass-card section">
    <div class="card-inner">

      <div class="section-header">
        <h3>BepInEx script mods <span class="badge experimental">Experimental</span></h3>
        <p class="section-desc">
          BepInEx 6.x IL2CPP macOS ARM64 is the community script-mod runtime for Paralives.
          Installing it places loader files in your Paralives directory and removes the
          Apple Developer ID signature from <code>Paralives.app</code>.
        </p>
      </div>

      {#if !gameInstallDir}
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
        <button class="btn btn-primary" onclick={beginConsent} type="button">Install BepInEx</button>

      {:else if bepinexState === "consent"}
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
            <button class="btn btn-secondary" onclick={cancelConsent} type="button">Cancel</button>
            <button
              class="btn btn-primary"
              disabled={!consentChecked}
              onclick={confirmInstall}
              type="button"
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
        <div class="status-row success">
          <span class="status-icon">✓</span>
          <div class="status-row-content">
            <span>BepInEx is <strong>installed</strong> and ARM64-ready.</span>
            {#if bepinexStatus?.version}
              <span class="version-badge">{bepinexStatus.version}</span>
            {/if}
          </div>
        </div>

        {#if !showUninstallConfirm}
          <button
            class="btn btn-danger-outline"
            onclick={() => (showUninstallConfirm = true)}
            type="button"
          >
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
              <button class="btn btn-secondary" onclick={() => (showUninstallConfirm = false)} type="button">
                Cancel
              </button>
              <button class="btn btn-danger" onclick={confirmUninstall} type="button">Remove BepInEx</button>
            </div>
          </div>
        {/if}

      {:else if bepinexState === "error"}
        <div class="status-row error">
          <span class="status-icon">✗</span>
          <span>{bepinexError}</span>
        </div>
        <div class="error-actions">
          <button class="btn btn-secondary" onclick={loadBepInExStatus} type="button">Retry</button>
          <button
            class="btn btn-secondary"
            onclick={() => { bepinexState = "idle"; bepinexError = null; }}
            type="button"
          >
            Cancel
          </button>
        </div>
      {/if}

    </div>
  </div>

  <!-- ── 4b. SMAPI (Stardew Valley) — only when stardew_valley_native is selected ── -->
  {#if $selectedGame?.game_id === "stardew_valley_native"}
    <h2 class="section-title">SMAPI (Stardew Valley)</h2>
    <div class="native-glass-card section">
      <div class="card-inner">

        <div class="section-header">
          <h3>SMAPI script-mod runtime <span class="badge experimental">Trust boundary</span></h3>
          <p class="section-desc">
            SMAPI (Stardew Modding API) is the community runtime for Stardew Valley script mods.
            Installing it renames <code>Contents/MacOS/StardewValley</code> to
            <code>StardewValley-original</code> and installs a new launcher, which
            invalidates the Apple Developer ID signature on <code>Stardew Valley.app</code>.
          </p>
        </div>

        {#if !stardewGameId}
          <div class="status-row notice">
            <span class="status-icon">ℹ</span>
            <span>Select <strong>Stardew Valley</strong> in the game picker to manage SMAPI.</span>
          </div>

        {:else if smapiState === "loading"}
          <div class="status-row">
            <span class="spinner"></span>
            <span>Checking SMAPI status…</span>
          </div>

        {:else if smapiState === "sandboxed"}
          <div class="status-row error">
            <span class="status-icon">✗</span>
            <span>
              This <code>Stardew Valley.app</code> is sandboxed (Mac App Store / system path).
              Corkscrew will not modify sandboxed bundles. Install the Steam or GOG build to use SMAPI.
            </span>
          </div>

        {:else if smapiState === "idle"}
          <div class="status-row notice">
            <span class="status-icon">○</span>
            <span>SMAPI is <strong>not installed</strong>.</span>
          </div>
          <button class="btn btn-primary" onclick={beginSmapiConsent} type="button">Install SMAPI</button>

        {:else if smapiState === "consent"}
          <div class="consent-card">
            <h4>Before installing SMAPI for Stardew Valley, please understand:</h4>
            <ol class="consent-list">
              <li>
                SMAPI is third-party software, not signed by Apple. It is MIT-licensed
                and developed openly at
                <button class="link-btn" onclick={() => { openUrl("https://github.com/Pathoschild/SMAPI").catch((err) => console.error("openUrl failed:", err)); }} type="button">github.com/Pathoschild/SMAPI</button>.
              </li>
              <li>
                Installing SMAPI <strong>patches the Stardew Valley bundle</strong> — the
                Apple Developer ID code signature is invalidated. macOS Gatekeeper may
                prompt you the first time you launch the patched bundle.
              </li>
              <li>
                SMAPI mods have <strong>full code execution</strong> on your Mac. Only install
                mods from sources you trust (e.g. the SMAPI-curated mod database on Nexus Mods).
              </li>
              <li>
                Corkscrew takes a <strong>snapshot of the bundle before patching</strong>. You can
                restore it from the game's state / rollback UI if anything goes wrong.
              </li>
            </ol>

            <label class="consent-checkbox">
              <input type="checkbox" bind:checked={smapiConsentChecked} />
              I understand SMAPI invalidates the Stardew Valley.app signature and runs third-party code.
            </label>

            <div class="consent-actions">
              <button class="btn btn-secondary" onclick={cancelSmapiConsent} type="button">Cancel</button>
              <button
                class="btn btn-primary"
                disabled={!smapiConsentChecked}
                onclick={confirmSmapiInstall}
                type="button"
              >
                Install SMAPI
              </button>
            </div>
          </div>

        {:else if smapiState === "installing"}
          <div class="status-row">
            <span class="spinner"></span>
            <span>Installing SMAPI… (downloading the latest release from GitHub)</span>
          </div>

        {:else if smapiState === "uninstalling"}
          <div class="status-row">
            <span class="spinner"></span>
            <span>Uninstalling SMAPI… (restoring the vanilla launcher)</span>
          </div>

        {:else if smapiState === "installed"}
          <div class="status-row success">
            <span class="status-icon">✓</span>
            <div class="status-row-content">
              <span>SMAPI is <strong>installed</strong>.</span>
              {#if smapiStatus?.version}
                <span class="version-badge">{smapiStatus.version}</span>
              {/if}
            </div>
          </div>

          {#if !smapiShowUninstallConfirm}
            <button
              class="btn btn-danger-outline"
              onclick={() => (smapiShowUninstallConfirm = true)}
              type="button"
            >
              Uninstall SMAPI
            </button>
          {:else}
            <div class="uninstall-confirm">
              <p>
                Restore the vanilla Stardew Valley launcher and remove SMAPI? Your
                <code>Mods/</code> directory is preserved — it will just stop being loaded.
              </p>
              <div class="consent-actions">
                <button class="btn btn-secondary" onclick={() => (smapiShowUninstallConfirm = false)} type="button">
                  Cancel
                </button>
                <button class="btn btn-danger" onclick={confirmSmapiUninstall} type="button">Remove SMAPI</button>
              </div>
            </div>
          {/if}

        {:else if smapiState === "error"}
          <div class="status-row error">
            <span class="status-icon">✗</span>
            <span>{smapiError}</span>
          </div>
          <div class="error-actions">
            <button class="btn btn-secondary" onclick={loadSmapiStatus} type="button">Retry</button>
            <button
              class="btn btn-secondary"
              onclick={() => { smapiState = smapiStatus?.installed ? "installed" : "idle"; smapiError = null; }}
              type="button"
            >
              Cancel
            </button>
          </div>
        {/if}

      </div>
    </div>
  {/if}

  <!-- ── 4c. BG3SE (Baldur's Gate 3) — only when baldurs_gate_3_native is selected ── -->
  {#if $selectedGame?.game_id === "baldurs_gate_3_native"}
    <h2 class="section-title">BG3 Script Extender</h2>
    <div class="native-glass-card section">
      <div class="card-inner">

        <div class="section-header">
          <h3>BG3SE script-mod runtime <span class="badge experimental">Experimental</span></h3>
          <p class="section-desc">
            The BG3 Script Extender (BG3SE) is the community runtime for Baldur's Gate 3
            script mods. The macOS install layout is still under upstream verification —
            install is currently manual. The <code>.app</code> bundle is not modified by Corkscrew;
            mods live in <code>~/Documents/Larian Studios/Baldur's Gate 3/</code>.
          </p>
        </div>

        {#if !bg3AppBundle}
          <div class="status-row notice">
            <span class="status-icon">ℹ</span>
            <span>Select <strong>Baldur's Gate 3</strong> in the game picker to manage BG3SE.</span>
          </div>

        {:else if bg3seState === "loading"}
          <div class="status-row">
            <span class="spinner"></span>
            <span>Checking BG3SE status…</span>
          </div>

        {:else if bg3seState === "windows_dll"}
          <div class="status-row warn">
            <span class="status-icon">⚠</span>
            <span>
              A Windows-only <code>DWrite.dll</code> was found in the bundle. That loader
              cannot run on macOS — remove it and use the macOS <code>.dylib</code> instead.
              See the upstream repository for the macOS install steps.
            </span>
          </div>
          <button class="btn btn-secondary" onclick={openBg3seManual} type="button">
            Open Norbyte/bg3se on GitHub
          </button>

        {:else if bg3seState === "idle"}
          <div class="status-row notice">
            <span class="status-icon">○</span>
            <span>BG3SE is <strong>not installed</strong>.</span>
          </div>
          <button class="btn btn-primary" onclick={installBg3seFlow} type="button">Install BG3SE</button>

        {:else if bg3seState === "installing"}
          <div class="status-row">
            <span class="spinner"></span>
            <span>Attempting BG3SE install…</span>
          </div>

        {:else if bg3seState === "uninstalling"}
          <div class="status-row">
            <span class="spinner"></span>
            <span>Uninstalling BG3SE…</span>
          </div>

        {:else if bg3seState === "manual_required"}
          <div class="status-row warn">
            <span class="status-icon">⚠</span>
            <span>
              <strong>Manual install required.</strong>
              BG3SE's macOS install layout is still being verified upstream. Automated
              install is not enabled in this build. Follow the upstream README to install
              the macOS <code>.dylib</code> into <code>Contents/MacOS/</code> of your
              Baldur's Gate 3 bundle.
            </span>
          </div>
          <div class="error-actions">
            <button class="btn btn-primary" onclick={openBg3seManual} type="button">
              Open Norbyte/bg3se on GitHub
            </button>
            <button class="btn btn-secondary" onclick={loadBg3seStatus} type="button">Re-check</button>
          </div>

        {:else if bg3seState === "installed"}
          <div class="status-row success">
            <span class="status-icon">✓</span>
            <div class="status-row-content">
              <span>BG3SE is <strong>installed</strong>.</span>
              {#if bg3seStatus?.version}
                <span class="version-badge">{bg3seStatus.version}</span>
              {/if}
            </div>
          </div>

          {#if !bg3seShowUninstallConfirm}
            <button
              class="btn btn-danger-outline"
              onclick={() => (bg3seShowUninstallConfirm = true)}
              type="button"
            >
              Uninstall BG3SE
            </button>
          {:else}
            <div class="uninstall-confirm">
              <p>
                Remove the BG3SE loader from <code>Contents/MacOS/</code>? Your script-mod
                files in <code>~/Documents/Larian Studios/</code> are not touched.
              </p>
              <div class="consent-actions">
                <button class="btn btn-secondary" onclick={() => (bg3seShowUninstallConfirm = false)} type="button">
                  Cancel
                </button>
                <button class="btn btn-danger" onclick={confirmBg3seUninstall} type="button">Remove BG3SE</button>
              </div>
            </div>
          {/if}

        {:else if bg3seState === "error"}
          <div class="status-row error">
            <span class="status-icon">✗</span>
            <span>{bg3seError}</span>
          </div>
          <div class="error-actions">
            <button class="btn btn-secondary" onclick={loadBg3seStatus} type="button">Retry</button>
            <button
              class="btn btn-secondary"
              onclick={() => { bg3seState = bg3seStatus?.installed ? "installed" : "idle"; bg3seError = null; }}
              type="button"
            >
              Cancel
            </button>
          </div>
        {/if}

      </div>
    </div>
  {/if}

  <!-- ── 5. Code Signing & Trust Boundaries ── -->
  <h2 class="section-title">Code Signing & Trust Boundaries</h2>
  <div class="native-glass-card section">
    <div class="card-inner">

      <div class="card-row">
        <div class="row-info">
          <span class="row-label">What Corkscrew mutates in native mode</span>
          <span class="row-desc">
            Corkscrew makes exactly one game-specific mutation to <code>.app</code> bundles.
            All other native games are modded entirely outside the bundle.
          </span>
        </div>
      </div>

      <div class="card-divider"></div>

      <div class="card-row">
        <div class="trust-table">
          <div class="trust-row">
            <span class="trust-game">Stardew Valley</span>
            <span class="trust-desc">
              SMAPI's launcher patch renames <code>Contents/MacOS/StardewValley</code>
              → <code>StardewValley-original</code> and installs a new launcher script.
              This <strong>invalidates the Apple Developer ID signature</strong> — this is
              intentional and expected SMAPI behavior, not a Corkscrew choice.
            </span>
          </div>
          <div class="trust-divider"></div>
          <div class="trust-row">
            <span class="trust-game">Baldur's Gate 3</span>
            <span class="trust-desc">
              Mods live entirely in <code>~/Documents/Larian Studios/Baldur's Gate 3/</code>.
              The <code>.app</code> bundle is <strong>never touched</strong>. Bundle signing is preserved.
            </span>
          </div>
          <div class="trust-divider"></div>
          <div class="trust-row">
            <span class="trust-game">Paralives</span>
            <span class="trust-desc">
              Data mods drop into <code>~/Library/Application Support/com.Paralives.Paralives/Mods/</code>
              — no <code>.app</code> mutation by default. BepInEx install (if initiated by you above) does
              modify the app, removing the signature to permit code injection.
            </span>
          </div>
          <div class="trust-divider"></div>
          <div class="trust-row">
            <span class="trust-game">Crimson Desert</span>
            <span class="trust-desc">
              Native support not yet implemented.
            </span>
          </div>
        </div>
      </div>

      <div class="card-divider"></div>

      <div class="card-row">
        <span class="row-desc">
          Sandboxed bundles (Mac App Store apps or <code>/System/Applications/</code> paths) are refused
          outright — Corkscrew will not modify them. Before any destructive native operation,
          Corkscrew creates a rollback snapshot.
          <button
            class="link-btn"
            onclick={() => { import("@tauri-apps/plugin-opener").then((m) => m.openUrl("https://github.com/cashcon57/corkscrew/blob/main/docs/native-trust-boundaries.md")).catch((err) => console.error("openUrl failed:", err)); }}
            type="button"
          >
            Full documentation →
          </button>
        </span>
      </div>

    </div>
  </div>

  <!-- ── 6. Experimental Features ── -->
  <h2 class="section-title">Experimental Features</h2>
  <div class="native-glass-card section">
    <div class="card-inner">

      <div class="card-row">
        <div class="warning-banner">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="warning-icon">
            <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
            <line x1="12" y1="9" x2="12" y2="13" />
            <line x1="12" y1="17" x2="12.01" y2="17" />
          </svg>
          <div class="warning-text">
            <strong>Native macOS Mode is experimental.</strong>
            It is in active development and may not function correctly for all games or mod types.
            Proceed with caution and always keep backups of your save files.
          </div>
        </div>
      </div>

      <div class="card-divider"></div>

      <div class="card-row">
        <div class="row-info">
          <span class="row-label">Disable Native Mode visibility</span>
          <span class="row-desc">
            To hide the Native Mode toggle from the topbar entirely, go to
            <button class="link-btn" onclick={() => goto("/settings")} type="button">
              Settings → About → "Show Native Mode toggle (in development)"
            </button>
            and turn it off.
          </span>
        </div>
      </div>

    </div>
  </div>

  <!-- ── 7. Diagnostics ── -->
  <h2 class="section-title">Diagnostics</h2>
  <div class="native-glass-card section">
    <div class="card-inner">

      <div class="card-row toggle-row">
        <div class="row-info">
          <span class="row-label">Corkscrew log directory</span>
          <span class="row-desc">
            Open the Corkscrew application data folder in Finder.
            Logs, config, mod database, and downloads index are stored here.
          </span>
        </div>
        <button class="btn btn-secondary" onclick={openLogDir} type="button">
          Open in Finder
        </button>
      </div>

    </div>
  </div>

</div>

<style>
  .page {
    max-width: 720px;
    margin: 0 auto;
    padding: 32px 24px 48px;
  }

  h1 {
    font-size: 36px;
    font-weight: 700;
    margin: 0 0 8px;
  }

  .subtitle {
    color: var(--text-secondary);
    margin: 0 0 32px;
    font-size: 14px;
  }

  /* Section titles — match Wine settings style */
  .section-title {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    padding: 0 4px;
    margin: 0 0 8px;
  }

  .section {
    margin-bottom: 24px;
  }

  /* Inner layout for glass cards */
  .card-inner {
    /* no additional padding; rows carry their own padding */
  }

  .card-row {
    padding: 14px 20px;
  }

  .card-divider {
    height: 1px;
    background: var(--separator-opaque, rgba(255, 255, 255, 0.08));
    margin: 0;
  }

  /* Toggle / action rows */
  .toggle-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }

  .row-info {
    display: flex;
    flex-direction: column;
    gap: 3px;
    flex: 1;
    min-width: 0;
  }

  .row-label {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-primary);
  }

  .row-desc {
    font-size: 12px;
    color: var(--text-secondary);
    line-height: 1.5;
  }

  .row-desc.muted {
    font-style: italic;
  }

  /* Section headers inside cards */
  .section-header {
    padding: 14px 20px 0;
  }

  h3 {
    font-size: 14px;
    font-weight: 600;
    margin: 0 0 6px;
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-primary);
  }

  h4 {
    font-size: 13px;
    font-weight: 600;
    margin: 0 0 12px;
    color: var(--text-primary);
  }

  .section-desc {
    color: var(--text-secondary);
    margin: 0 0 16px;
    font-size: 12px;
    line-height: 1.5;
    padding: 0 20px;
  }

  code {
    font-family: ui-monospace, "SF Mono", monospace;
    font-size: 11px;
    background: rgba(255, 255, 255, 0.08);
    border-radius: 4px;
    padding: 1px 5px;
  }

  /* ---- Centered row (loading) ---- */

  .centered-row {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 20px;
  }

  .loading-label {
    font-size: 13px;
    color: var(--text-tertiary);
  }

  /* ---- Auth row (logged in) ---- */

  .auth-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }

  .user-info {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
  }

  .user-avatar {
    width: 36px;
    height: 36px;
    border-radius: 50%;
    object-fit: cover;
    flex-shrink: 0;
  }

  .user-avatar-placeholder {
    background: var(--surface-hover);
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-tertiary);
  }

  .user-details {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }

  .user-name-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .user-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .premium-badge {
    display: inline-flex;
    align-items: center;
    padding: 1px 6px;
    border-radius: 100px;
    font-size: 10px;
    font-weight: 700;
    color: #ff9f0a;
    background: rgba(255, 159, 10, 0.15);
    text-transform: uppercase;
    letter-spacing: 0.02em;
    flex-shrink: 0;
  }

  .auth-method-label {
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .auth-actions {
    flex-shrink: 0;
  }

  /* ---- Connect flow ---- */

  .connect-flow {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .connect-description {
    font-size: 13px;
    color: var(--text-secondary);
    line-height: 1.5;
  }

  /* ---- Nexus button ---- */

  .btn-nexus {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    width: 100%;
    padding: 10px 16px;
    background: linear-gradient(135deg, #da8e35 0%, #c67a28 100%);
    color: #fff;
    font-size: 14px;
    font-weight: 600;
    border: none;
    border-radius: 8px;
    cursor: pointer;
    transition: filter 0.15s ease, transform 0.1s ease;
    letter-spacing: 0.01em;
  }

  .btn-nexus:hover {
    filter: brightness(1.1);
  }

  .btn-nexus:active {
    transform: scale(0.985);
  }

  .nexus-icon {
    flex-shrink: 0;
  }

  /* ---- OAuth waiting ---- */

  .oauth-waiting {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px;
    background: var(--surface-hover);
    border-radius: 8px;
    animation: fade-in 0.2s ease;
  }

  @keyframes fade-in {
    from { opacity: 0; transform: translateY(-4px); }
    to { opacity: 1; transform: translateY(0); }
  }

  .oauth-waiting-text {
    display: flex;
    flex-direction: column;
    gap: 1px;
    flex: 1;
    min-width: 0;
  }

  .oauth-waiting-title {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-primary);
  }

  .oauth-waiting-subtitle {
    font-size: 11px;
    color: var(--text-tertiary);
  }

  /* ---- Divider / API key fallback ---- */

  .divider-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .divider-line {
    flex: 1;
    height: 1px;
    background: var(--separator-opaque, rgba(255, 255, 255, 0.08));
  }

  .divider-toggle {
    font-size: 11px;
    color: var(--text-tertiary);
    background: none;
    border: none;
    cursor: pointer;
    white-space: nowrap;
    padding: 2px 0;
    transition: color 0.15s ease;
  }

  .divider-toggle:hover {
    color: var(--text-secondary);
  }

  .api-key-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
    animation: fade-in 0.2s ease;
  }

  .api-key-hint {
    font-size: 12px;
    color: var(--text-tertiary);
    line-height: 1.4;
  }

  .api-key-input-row {
    display: flex;
    gap: 8px;
  }

  .settings-input {
    flex: 1;
    min-width: 0;
    padding: 8px 12px;
    background: rgba(255, 255, 255, 0.06);
    border: 1px solid var(--separator-opaque, rgba(255, 255, 255, 0.12));
    border-radius: 6px;
    color: var(--text-primary);
    font-size: 13px;
    font-family: inherit;
    outline: none;
    transition: border-color 0.15s ease;
  }

  .settings-input:focus {
    border-color: var(--system-accent, #007aff);
    box-shadow: 0 0 0 3px rgba(0, 122, 255, 0.15);
  }

  .settings-input::placeholder {
    color: var(--text-tertiary);
  }

  .validation-error {
    font-size: 12px;
    color: var(--red, #ef5350);
    line-height: 1.4;
  }

  /* ---- Storage paths ---- */

  .path-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }

  .path-info {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    flex: 1;
  }

  .path-code {
    display: block;
    font-size: 11px;
    color: var(--text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }

  /* ---- Badges ---- */

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

  /* ---- Status rows ---- */

  .status-row {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 8px 20px 12px;
    font-size: 13px;
    color: var(--text-secondary);
  }

  .status-row.success { color: var(--green, #4caf50); }
  .status-row.warn    { color: #ffa000; }
  .status-row.error   { color: var(--red, #ef5350); }
  .status-row.notice  { color: var(--text-secondary); }

  .status-icon {
    font-size: 16px;
    flex-shrink: 0;
  }

  .status-row-content {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .version-badge {
    font-size: 11px;
    font-family: ui-monospace, "SF Mono", monospace;
    background: rgba(255, 255, 255, 0.08);
    border-radius: 4px;
    padding: 1px 6px;
  }

  /* ---- Consent card ---- */

  .consent-card {
    margin: 0 20px 16px;
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

  /* ---- Uninstall confirm ---- */

  .uninstall-confirm {
    margin: 0 20px 16px;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid rgba(239, 83, 80, 0.25);
    border-radius: 10px;
    padding: 16px;
  }

  .uninstall-confirm p {
    font-size: 13px;
    color: var(--text-secondary);
    margin: 0 0 14px;
    line-height: 1.5;
  }

  /* ---- Error actions ---- */

  .error-actions {
    display: flex;
    gap: 10px;
    margin: 0 20px 16px;
  }

  /* ---- Button actions (inside card rows) ---- */

  .card-row .btn {
    white-space: nowrap;
    flex-shrink: 0;
  }

  /* Stand-alone buttons inside card-inner (outside card-row) */
  .card-inner > .btn,
  .card-inner .btn.btn-primary:not(.api-key-input-row .btn) {
    margin: 0 20px 16px;
  }

  /* ---- Trust table (code signing section) ---- */

  .trust-table {
    display: flex;
    flex-direction: column;
    gap: 0;
    width: 100%;
  }

  .trust-row {
    display: grid;
    grid-template-columns: 140px 1fr;
    gap: 12px;
    align-items: start;
    padding: 4px 0;
  }

  .trust-game {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-primary);
    padding-top: 1px;
  }

  .trust-desc {
    font-size: 12px;
    color: var(--text-secondary);
    line-height: 1.55;
  }

  .trust-divider {
    height: 1px;
    background: var(--separator-opaque, rgba(255, 255, 255, 0.06));
    margin: 8px 0;
  }

  /* ---- Warning banner ---- */

  .warning-banner {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 12px 16px;
    background: rgba(255, 160, 0, 0.08);
    border: 1px solid rgba(255, 160, 0, 0.25);
    border-radius: 8px;
    font-size: 13px;
    color: var(--text-primary);
    line-height: 1.5;
  }

  .warning-icon {
    color: #ffa000;
    flex-shrink: 0;
    margin-top: 1px;
  }

  .warning-text {
    line-height: 1.5;
  }

  /* ---- Inline error ---- */

  .inline-error {
    color: var(--red, #ef5350);
    font-size: 12px;
    display: block;
    margin-top: 4px;
  }

  /* ---- Link button ---- */

  .link-btn {
    background: none;
    border: none;
    color: var(--system-accent, #007aff);
    font-size: inherit;
    cursor: pointer;
    padding: 0;
    text-decoration: underline;
    text-decoration-color: transparent;
    transition: text-decoration-color 0.15s ease;
  }

  .link-btn:hover {
    text-decoration-color: currentColor;
  }

  /* ---- Spinner ---- */

  .spinner {
    display: inline-block;
    width: 14px;
    height: 14px;
    border: 2px solid rgba(255, 255, 255, 0.15);
    border-top-color: var(--accent, var(--system-accent, #7c4dff));
    border-radius: 50%;
    animation: spin 0.75s linear infinite;
    flex-shrink: 0;
  }

  .spinner.spinner-white {
    border-color: rgba(255, 255, 255, 0.3);
    border-top-color: #fff;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  /* ---- Buttons ---- */

  .btn {
    border: none;
    border-radius: 8px;
    padding: 8px 14px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: opacity 0.15s, background 0.15s;
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .btn:not(:disabled):hover {
    opacity: 0.85;
  }

  .btn-sm {
    padding: 5px 10px;
    font-size: 12px;
  }

  .btn-primary {
    background: var(--accent, var(--system-accent, #7c4dff));
    color: #fff;
  }

  .btn-secondary {
    background: rgba(255, 255, 255, 0.1);
    color: var(--text-primary);
  }

  .btn-danger {
    background: var(--red, #ef5350);
    color: #fff;
  }

  .btn-danger-outline {
    background: transparent;
    border: 1px solid rgba(239, 83, 80, 0.5);
    color: var(--red, #ef5350);
  }
</style>
