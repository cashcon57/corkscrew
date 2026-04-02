<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";

  interface Props {
    onauth: () => void;
    oncancel: () => void;
  }

  let { onauth, oncancel }: Props = $props();

  let username = $state("");
  let password = $state("");
  let steamGuardCode = $state("");
  let phase = $state<"credentials" | "steam_guard" | "authenticating" | "error">("credentials");
  let errorMessage = $state("");

  async function handleLogin() {
    if (!username.trim() || !password.trim()) return;
    phase = "authenticating";
    errorMessage = "";

    try {
      await invoke("dd_authenticate", {
        username: username.trim(),
        password: password.trim(),
        steamGuardCode: null,
      });
      // Auth succeeded without Steam Guard — clear credentials before callback
      password = "";
      onauth();
    } catch (e: unknown) {
      const msg = String(e);
      if (msg === "STEAM_GUARD_REQUIRED" || msg.includes("STEAM_GUARD")) {
        phase = "steam_guard";
      } else if (msg.includes("AUTH_FAILED")) {
        password = "";
        phase = "error";
        errorMessage = "Invalid username or password. Double-check and try again.";
      } else {
        password = "";
        phase = "error";
        errorMessage = msg;
      }
    }
  }

  async function handleSteamGuard() {
    if (!steamGuardCode.trim()) return;
    phase = "authenticating";
    errorMessage = "";

    try {
      await invoke("dd_authenticate", {
        username: username.trim(),
        password: password.trim(),
        steamGuardCode: steamGuardCode.trim(),
      });
      // Clear credentials before callback
      password = "";
      steamGuardCode = "";
      onauth();
    } catch (e: unknown) {
      const msg = String(e);
      steamGuardCode = "";
      if (msg.includes("AUTH_FAILED")) {
        phase = "error";
        errorMessage = "Invalid Steam Guard code. Check your email or authenticator app.";
      } else {
        password = "";
        phase = "error";
        errorMessage = msg;
      }
    }
  }
</script>

<div class="auth-overlay" onclick={oncancel} role="presentation">
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="auth-modal" onclick={(e) => e.stopPropagation()}>
    <div class="auth-header">
      <h3 class="auth-title">Steam Login</h3>
      <button class="auth-close" onclick={oncancel}>&times;</button>
    </div>

    <div class="auth-body">
      <p class="auth-description">
        Corkscrew needs to connect to Steam to download older game versions.
        Your credentials are sent directly to Steam's servers via
        <a href="https://github.com/SteamRE/DepotDownloader" target="_blank" rel="noopener">DepotDownloader</a>
        &mdash; they are never stored or transmitted by Corkscrew.
      </p>

      {#if phase === "credentials" || phase === "error"}
        <div class="auth-form">
          <label class="auth-label">
            Steam Username
            <input
              type="text"
              class="auth-input"
              bind:value={username}
              placeholder="Your Steam username"
              onkeydown={(e) => { if (e.key === "Enter") handleLogin(); }}
              autofocus
            />
          </label>
          <label class="auth-label">
            Password
            <input
              type="password"
              class="auth-input"
              bind:value={password}
              placeholder="Your Steam password"
              onkeydown={(e) => { if (e.key === "Enter") handleLogin(); }}
            />
          </label>
          {#if errorMessage}
            <p class="auth-error">{errorMessage}</p>
          {/if}
        </div>
      {:else if phase === "steam_guard"}
        <div class="auth-form">
          <p class="auth-steam-guard-hint">
            Steam sent a verification code to your email or authenticator app.
            Enter it below.
          </p>
          <label class="auth-label">
            Steam Guard Code
            <input
              type="text"
              class="auth-input auth-input-code"
              bind:value={steamGuardCode}
              placeholder="XXXXX"
              maxlength="5"
              onkeydown={(e) => { if (e.key === "Enter") handleSteamGuard(); }}
              autofocus
            />
          </label>
        </div>
      {:else if phase === "authenticating"}
        <div class="auth-loading">
          <div class="spinner-sm"></div>
          <span>Connecting to Steam...</span>
        </div>
      {/if}
    </div>

    <div class="auth-footer">
      <p class="auth-credit">
        Downgrades powered by <a href="https://github.com/SteamRE/DepotDownloader" target="_blank" rel="noopener">DepotDownloader</a> (GPL-2.0)
      </p>
      <div class="auth-actions">
        <button class="btn btn-ghost" onclick={oncancel}>Cancel</button>
        {#if phase === "credentials" || phase === "error"}
          <button
            class="btn btn-accent"
            onclick={handleLogin}
            disabled={!username.trim() || !password.trim()}
          >
            Sign In
          </button>
        {:else if phase === "steam_guard"}
          <button
            class="btn btn-accent"
            onclick={handleSteamGuard}
            disabled={!steamGuardCode.trim()}
          >
            Verify
          </button>
        {/if}
      </div>
    </div>
  </div>
</div>

<style>
  .auth-overlay {
    position: fixed;
    inset: 0;
    z-index: 10000;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    backdrop-filter: blur(4px);
  }

  .auth-modal {
    background: var(--bg-primary);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg, 12px);
    width: min(440px, 90vw);
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.4);
  }

  .auth-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-4, 16px) var(--space-5, 20px);
    border-bottom: 1px solid var(--border);
  }

  .auth-title {
    font-size: 16px;
    font-weight: 600;
    margin: 0;
  }

  .auth-close {
    background: none;
    border: none;
    color: var(--text-tertiary);
    font-size: 22px;
    cursor: pointer;
    line-height: 1;
  }

  .auth-body {
    padding: var(--space-4, 16px) var(--space-5, 20px);
  }

  .auth-description {
    font-size: 13px;
    color: var(--text-secondary);
    line-height: 1.5;
    margin: 0 0 var(--space-4, 16px);
  }

  .auth-description a {
    color: var(--accent);
    text-decoration: none;
  }
  .auth-description a:hover {
    text-decoration: underline;
  }

  .auth-form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3, 12px);
  }

  .auth-label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
  }

  .auth-input {
    padding: 8px 12px;
    border-radius: var(--radius, 6px);
    border: 1px solid var(--border);
    background: var(--bg-secondary);
    color: var(--text-primary);
    font-size: 14px;
    outline: none;
    transition: border-color 0.15s;
  }
  .auth-input:focus {
    border-color: var(--accent);
  }

  .auth-input-code {
    font-size: 20px;
    letter-spacing: 4px;
    text-align: center;
    font-family: monospace;
    max-width: 160px;
  }

  .auth-steam-guard-hint {
    font-size: 13px;
    color: var(--text-secondary);
    margin: 0 0 var(--space-2, 8px);
  }

  .auth-error {
    font-size: 12px;
    color: var(--red, #f44);
    margin: 0;
  }

  .auth-loading {
    display: flex;
    align-items: center;
    gap: var(--space-3, 12px);
    padding: var(--space-4, 16px) 0;
    font-size: 14px;
    color: var(--text-secondary);
  }

  .auth-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-3, 12px) var(--space-5, 20px);
    border-top: 1px solid var(--border);
  }

  .auth-credit {
    font-size: 11px;
    color: var(--text-tertiary);
    margin: 0;
  }
  .auth-credit a {
    color: var(--text-tertiary);
    text-decoration: underline;
  }

  .auth-actions {
    display: flex;
    gap: var(--space-2, 8px);
  }
</style>
