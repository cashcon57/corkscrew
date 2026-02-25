<script lang="ts">
  import { onMount } from "svelte";
  import { config, showError, showSuccess } from "$lib/stores";
  import {
    getConfig,
    setConfigValue,
    clearOAuthTokens,
    getNexusAccountStatus,
    startOAuthLogin,
  } from "$lib/api";

  const NEXUS_API_KEY_URL = "https://www.nexusmods.com/users/myaccount?tab=api+access";

  // ---- State ----

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
    account?.auth_type === "oauth" ? "Connected via Nexus Mods SSO" : "Connected via API key"
  );

  onMount(async () => {
    await checkAuthStatus();
  });

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
      // OAuth flow completed — tokens are saved, refresh account status
      const status = await getNexusAccountStatus();
      if (status.connected) {
        account = status;
        showSuccess(`Signed in as ${status.name}`);
      } else {
        validationError = "Authorization completed but account status check failed. Please try again.";
      }
    } catch (e: unknown) {
      const msg = typeof e === "string" ? e : (e instanceof Error ? e.message : String(e));
      if (msg.includes("Cancelled") || msg.includes("timed out")) {
        // User cancelled or it timed out — not an error worth showing
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
      } catch { /* ignore cleanup errors */ }
      const msg = typeof e === "string" ? e : (e instanceof Error ? e.message : String(e));
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
</script>

<!-- Nexus Mods Account Section -->
<div class="section">
  <h2 class="section-title">Nexus Mods Account</h2>
  <div class="section-card">
    {#if loadingAuth}
      <div class="card-row centered-row">
        <span class="spinner-sm"></span>
        <span class="loading-label">Checking account status...</span>
      </div>
    {:else if isLoggedIn && account}
      <!-- Logged in state -->
      <div class="card-row auth-row">
        <div class="user-info">
          {#if account.avatar}
            <img
              class="user-avatar"
              src={account.avatar}
              alt={account.name}
            />
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
            class="btn-ghost"
            onclick={handleSignOut}
            disabled={signingOut}
            type="button"
          >
            {signingOut ? "Signing out..." : "Sign Out"}
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

          <!-- Primary: OAuth Sign In -->
          {#if oauthConnecting}
            <div class="oauth-waiting">
              <span class="spinner-sm"></span>
              <div class="oauth-waiting-text">
                <span class="oauth-waiting-title">Waiting for authorization...</span>
                <span class="oauth-waiting-subtitle">Complete sign-in in your browser, then return here.</span>
              </div>
              <button
                class="btn-ghost btn-sm"
                onclick={() => { oauthConnecting = false; }}
                type="button"
              >
                Cancel
              </button>
            </div>
          {:else}
            <button
              class="btn-nexus"
              onclick={handleOAuthLogin}
              type="button"
            >
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

          <!-- Fallback: API Key -->
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
                <button class="link-btn" onclick={() => { import("@tauri-apps/plugin-opener").then(m => m.openUrl(NEXUS_API_KEY_URL)); }} type="button">
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
                  class="btn-primary"
                  onclick={handleApiKeyConnect}
                  disabled={apiKeyConnecting || !apiKeyInput.trim()}
                  type="button"
                >
                  {#if apiKeyConnecting}
                    <span class="spinner-sm spinner-white"></span>
                    Verifying...
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

<style>
  /* ---- Sections ---- */

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

  .card-row {
    padding: var(--space-3) var(--space-4);
  }

  /* ---- Centered Row (loading) ---- */

  .centered-row {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    padding: var(--space-5);
  }

  .loading-label {
    font-size: 13px;
    color: var(--text-tertiary);
  }

  .spinner-sm {
    display: inline-block;
    width: 14px;
    height: 14px;
    border: 2px solid var(--separator-opaque);
    border-top-color: var(--system-accent);
    border-radius: 50%;
    animation: spin 0.75s linear infinite;
    flex-shrink: 0;
  }

  .spinner-white {
    border-color: rgba(255, 255, 255, 0.3);
    border-top-color: #fff;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  /* ---- Auth Row (logged in) ---- */

  .auth-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
  }

  .user-info {
    display: flex;
    align-items: center;
    gap: var(--space-3);
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
    gap: var(--space-2);
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
    display: flex;
    gap: var(--space-2);
    flex-shrink: 0;
  }

  /* ---- Connect Flow ---- */

  .connect-flow {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .connect-description {
    font-size: 13px;
    color: var(--text-secondary);
    line-height: 1.5;
  }

  /* ---- OAuth Button ---- */

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
    border-radius: var(--radius-md, 8px);
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

  /* ---- OAuth Waiting ---- */

  .oauth-waiting {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3);
    background: var(--surface-hover);
    border-radius: var(--radius-md, 8px);
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

  /* ---- Divider ---- */

  .divider-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .divider-line {
    flex: 1;
    height: 1px;
    background: var(--separator-opaque);
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

  /* ---- API Key Fallback ---- */

  .api-key-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    animation: fade-in 0.2s ease;
  }

  .api-key-hint {
    font-size: 12px;
    color: var(--text-tertiary);
    line-height: 1.4;
  }

  .link-btn {
    background: none;
    border: none;
    color: var(--system-accent);
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

  /* ---- API Key Input ---- */

  .api-key-input-row {
    display: flex;
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

  .validation-error {
    font-size: 12px;
    color: var(--red);
    line-height: 1.4;
  }

  /* ---- Buttons ---- */

  .btn-primary {
    padding: var(--space-1) var(--space-3);
    background: var(--system-accent);
    color: var(--system-accent-on);
    font-size: 13px;
    font-weight: 500;
    border: none;
    border-radius: var(--radius-sm);
    white-space: nowrap;
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    cursor: pointer;
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
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease), color var(--duration-fast) var(--ease);
  }

  .btn-ghost:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .btn-ghost:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-sm {
    padding: 4px 10px;
    font-size: 12px;
  }
</style>
