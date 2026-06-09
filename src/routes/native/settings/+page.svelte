<script lang="ts">
  import { goto } from "$app/navigation";
  import { onMount } from "svelte";
  import {
    setNativeMode,
    getParalivesBepInExStatus,
    installParalivesBepInEx,
    uninstallParalivesBepInEx,
    rescanNativeGames,
    getConfig,
    setConfigValue,
    clearOAuthTokens,
    getNexusAccountStatus,
    startOAuthLogin,
    type ParalivesBepInExStatus,
  } from "$lib/api";
  import { nativeMode, selectedGame, games, config, showSuccess, showError } from "$lib/stores";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { appDataDir } from "@tauri-apps/api/path";
  import type { DetectedGame } from "$lib/types";

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
  // NexusMods account (copied from settings-auth-section.svelte)
  // TODO: extract to src/lib/components/settings/NexusAccountSection.svelte for DRY
  // ---------------------------------------------------------------------------

  const NEXUS_API_KEY_URL = "https://www.nexusmods.com/users/myaccount?tab=api+access";

  interface AccountStatus {
    connected: boolean;
    auth_type?: string;
    name?: string;
    email?: string | null;
    avatar?: string | null;
    is_premium?: boolean;
    membership_roles?: string[];
  }

  let account = $state<AccountStatus | null>(null);
  let loadingAuth = $state(true);
  let signingOut = $state(false);
  let oauthConnecting = $state(false);
  let showApiKeyFallback = $state(false);
  let apiKeyInput = $state("");
  let apiKeyConnecting = $state(false);
  let validationError = $state<string | null>(null);

  const isLoggedIn = $derived(account?.connected === true);
  const isPremium = $derived(account?.is_premium === true);
  const authLabel = $derived(
    account?.auth_type === "oauth"
      ? "Connected via Nexus Mods SSO"
      : "Connected via API key"
  );

  async function checkAuthStatus() {
    loadingAuth = true;
    try {
      account = await getNexusAccountStatus();
    } catch {
      account = { connected: false };
    } finally {
      loadingAuth = false;
    }
  }

  async function handleOAuthLogin() {
    oauthConnecting = true;
    validationError = null;
    try {
      await startOAuthLogin();
      const status = await getNexusAccountStatus();
      if (status.connected) {
        account = status;
        showSuccess(`Signed in as ${status.name}`);
      } else {
        validationError =
          "Authorization completed but account status check failed. Please try again.";
      }
    } catch (e: unknown) {
      const msg =
        typeof e === "string"
          ? e
          : e instanceof Error
            ? e.message
            : String(e);
      if (msg.includes("Cancelled") || msg.includes("timed out")) {
        validationError = null;
      } else {
        validationError = `Sign-in failed: ${msg}`;
      }
    } finally {
      oauthConnecting = false;
    }
  }

  async function handleApiKeyConnect() {
    if (!apiKeyInput.trim()) return;
    apiKeyConnecting = true;
    validationError = null;
    try {
      await setConfigValue("nexus_api_key", apiKeyInput.trim());
      const cfg = await getConfig();
      config.set(cfg);
      const status = await getNexusAccountStatus();
      if (status.connected) {
        account = status;
        apiKeyInput = "";
        showApiKeyFallback = false;
        showSuccess(`Connected as ${status.name}`);
      } else {
        await setConfigValue("nexus_api_key", "");
        const cfg2 = await getConfig();
        config.set(cfg2);
        validationError = "Invalid API key. Please check and try again.";
      }
    } catch (e: unknown) {
      try {
        await setConfigValue("nexus_api_key", "");
        const cfg2 = await getConfig();
        config.set(cfg2);
      } catch {
        /* ignore cleanup errors */
      }
      const msg =
        typeof e === "string"
          ? e
          : e instanceof Error
            ? e.message
            : String(e);
      validationError = `Connection failed: ${msg}`;
    } finally {
      apiKeyConnecting = false;
    }
  }

  async function handleSignOut() {
    signingOut = true;
    try {
      await clearOAuthTokens();
      await setConfigValue("nexus_api_key", "");
      const cfg = await getConfig();
      config.set(cfg);
      account = { connected: false };
      showSuccess("Signed out of Nexus Mods");
    } catch (e: unknown) {
      showError(`Sign-out failed: ${e}`);
    } finally {
      signingOut = false;
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
    checkAuthStatus();
    loadBepInExStatus();
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
  <!--
    TODO: extract this widget into src/lib/components/settings/NexusAccountSection.svelte
    so it can be shared between /settings and /native/settings without duplication.
  -->
  <h2 class="section-title">Nexus Mods Account</h2>
  <div class="native-glass-card section">
    <div class="card-inner">

      {#if loadingAuth}
        <div class="card-row centered-row">
          <span class="spinner"></span>
          <span class="loading-label">Checking account status…</span>
        </div>
      {:else if isLoggedIn && account}
        <!-- Logged-in state -->
        <div class="card-row auth-row">
          <div class="user-info">
            {#if account.avatar}
              <img class="user-avatar" src={account.avatar} alt={account.name} />
            {:else}
              <div class="user-avatar user-avatar-placeholder">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2" />
                  <circle cx="12" cy="7" r="4" />
                </svg>
              </div>
            {/if}
            <div class="user-details">
              <div class="user-name-row">
                <span class="user-name">{account.name}</span>
                {#if isPremium}
                  <span class="premium-badge">Premium</span>
                {/if}
              </div>
              <span class="auth-method-label">{authLabel}</span>
            </div>
          </div>
          <div class="auth-actions">
            <button
              class="btn btn-secondary"
              onclick={handleSignOut}
              disabled={signingOut}
              type="button"
            >
              {signingOut ? "Signing out…" : "Sign Out"}
            </button>
          </div>
        </div>
      {:else}
        <!-- Not logged in -->
        <div class="card-row">
          <div class="connect-flow">
            <span class="connect-description">
              Connect your Nexus Mods account to download mods and browse collections.
            </span>

            {#if oauthConnecting}
              <div class="oauth-waiting">
                <span class="spinner"></span>
                <div class="oauth-waiting-text">
                  <span class="oauth-waiting-title">Waiting for authorization…</span>
                  <span class="oauth-waiting-subtitle">Complete sign-in in your browser, then return here.</span>
                </div>
                <button
                  class="btn btn-secondary btn-sm"
                  onclick={() => { oauthConnecting = false; }}
                  type="button"
                >
                  Cancel
                </button>
              </div>
            {:else}
              <button class="btn-nexus" onclick={handleOAuthLogin} type="button">
                <svg class="nexus-icon" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M15 3h4a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2h-4" />
                  <polyline points="10 17 15 12 10 7" />
                  <line x1="15" y1="12" x2="3" y2="12" />
                </svg>
                Sign in with Nexus Mods
              </button>
            {/if}

            {#if validationError}
              <span class="validation-error">{validationError}</span>
            {/if}

            <div class="divider-row">
              <span class="divider-line"></span>
              <button
                class="divider-toggle"
                onclick={() => { showApiKeyFallback = !showApiKeyFallback; validationError = null; }}
                type="button"
              >
                {showApiKeyFallback ? "Hide API key option" : "Use API key instead"}
              </button>
              <span class="divider-line"></span>
            </div>

            {#if showApiKeyFallback}
              <div class="api-key-section">
                <span class="api-key-hint">
                  Paste a personal API key from your
                  <button
                    class="link-btn"
                    onclick={() => { import("@tauri-apps/plugin-opener").then((m) => m.openUrl(NEXUS_API_KEY_URL)).catch((err) => console.error("openUrl failed:", err)); }}
                    type="button"
                  >
                    Nexus Mods account
                  </button>
                </span>
                <div class="api-key-input-row">
                  <input
                    type="password"
                    class="settings-input"
                    placeholder="Paste your API key here"
                    bind:value={apiKeyInput}
                    onkeydown={(e) => { if (e.key === "Enter") handleApiKeyConnect(); }}
                    oninput={() => { validationError = null; }}
                  />
                  <button
                    class="btn btn-primary"
                    onclick={handleApiKeyConnect}
                    disabled={apiKeyConnecting || !apiKeyInput.trim()}
                    type="button"
                  >
                    {#if apiKeyConnecting}
                      <span class="spinner spinner-white"></span>
                      Verifying…
                    {:else}
                      Connect
                    {/if}
                  </button>
                </div>
              </div>
            {/if}
          </div>
        </div>
      {/if}

    </div>
  </div>

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
