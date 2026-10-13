<script lang="ts">
  import { ddAuthenticate } from "$lib/api";

  interface Props {
    onauth: () => void;
    oncancel: () => void;
  }

  let { onauth, oncancel }: Props = $props();

  let username = $state("");
  let password = $state("");
  let steamGuardCode = $state("");
  let phase = $state<"credentials" | "steam_guard" | "mobile_confirm" | "authenticating" | "error">("credentials");
  let errorMessage = $state("");

  async function handleLogin() {
    if (!username.trim() || !password.trim()) return;
    phase = "authenticating";
    errorMessage = "";

    try {
      await ddAuthenticate(username.trim(), password.trim(), null);
      password = "";
      onauth();
    } catch (e: unknown) {
      const msg = String(e);
      if (msg === "STEAM_GUARD_REQUIRED" || msg.includes("STEAM_GUARD")) {
        phase = "steam_guard";
      } else if (msg === "STEAM_GUARD_MOBILE" || msg.includes("STEAM_GUARD_MOBILE")) {
        phase = "mobile_confirm";
        handleMobileConfirm();
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

  async function handleMobileConfirm() {
    phase = "mobile_confirm";
    errorMessage = "";

    try {
      await ddAuthenticate(username.trim(), password.trim(), null);
      password = "";
      onauth();
    } catch (e: unknown) {
      const msg = String(e);
      password = "";
      if (msg.includes("AUTH_FAILED") || msg.includes("timed out")) {
        phase = "error";
        errorMessage = "Steam Guard confirmation timed out or was denied. Try again.";
      } else {
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
      await ddAuthenticate(username.trim(), password.trim(), steamGuardCode.trim());
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

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="modal-overlay" onclick={oncancel} role="presentation">
  <div class="cleanup-modal" onclick={(e) => e.stopPropagation()} role="dialog" aria-label="Steam Login">
    <div class="cleanup-header">
      <h3 class="cleanup-title">Steam Login</h3>
      <button class="cleanup-close" onclick={oncancel}>&times;</button>
    </div>

    <div class="cleanup-body">
      <p class="cleanup-info">
        Corkscrew needs to connect to Steam to download older game versions.
        Your credentials are sent directly to Steam's servers via
        <a href="https://github.com/SteamRE/DepotDownloader" target="_blank" rel="noopener">DepotDownloader</a>
        &mdash; they are never stored or transmitted by Corkscrew.
      </p>

      {#if phase === "credentials" || phase === "error"}
        <div class="form-fields">
          <label class="form-label">
            <span class="form-label-text">Steam Username</span>
            <input
              type="text"
              class="form-input"
              bind:value={username}
              placeholder="Your Steam username"
              onkeydown={(e) => { if (e.key === "Enter") handleLogin(); }}
              autofocus
            />
          </label>
          <label class="form-label">
            <span class="form-label-text">Password</span>
            <input
              type="password"
              class="form-input"
              bind:value={password}
              placeholder="Your Steam password"
              onkeydown={(e) => { if (e.key === "Enter") handleLogin(); }}
            />
          </label>
          {#if errorMessage}
            <p class="form-error">{errorMessage}</p>
          {/if}
        </div>
      {:else if phase === "mobile_confirm"}
        <div class="form-fields">
          <p class="cleanup-info">
            Steam sent a confirmation to your phone. Open the <strong>Steam mobile app</strong> and approve the login request.
          </p>
          <div class="form-loading">
            <div class="spinner-sm"></div>
            <span>Waiting for approval...</span>
          </div>
        </div>
      {:else if phase === "steam_guard"}
        <div class="form-fields">
          <p class="cleanup-info">
            Enter the code from your Steam Guard email or authenticator app.
            If you got a push notification on your phone instead, approve it there &mdash; Corkscrew will detect it automatically.
          </p>
          <label class="form-label">
            <span class="form-label-text">Steam Guard Code</span>
            <input
              type="text"
              class="form-input form-input-code"
              bind:value={steamGuardCode}
              placeholder="XXXXX"
              maxlength="5"
              onkeydown={(e) => { if (e.key === "Enter") handleSteamGuard(); }}
              autofocus
            />
          </label>
        </div>
      {:else if phase === "authenticating"}
        <div class="form-loading">
          <div class="spinner-sm"></div>
          <span>Connecting to Steam...</span>
        </div>
      {/if}
    </div>

    <div class="cleanup-actions">
      <span class="dd-credit">
        Powered by <a href="https://github.com/SteamRE/DepotDownloader" target="_blank" rel="noopener">DepotDownloader</a> (GPL-2.0)
      </span>
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

<style>
  .modal-overlay {
    position: fixed;
    inset: 0;
    z-index: 10000;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    backdrop-filter: blur(4px);
  }

  .cleanup-modal {
    background: var(--bg-secondary);
    border: 1px solid var(--border-primary);
    border-radius: 12px;
    width: 480px;
    max-width: 90vw;
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5);
    display: flex;
    flex-direction: column;
  }

  .cleanup-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 20px;
    border-bottom: 1px solid var(--border-primary);
  }

  .cleanup-title {
    font-size: 16px;
    font-weight: 600;
    margin: 0;
    color: var(--text-primary);
  }

  .cleanup-close {
    background: none;
    border: none;
    color: var(--text-secondary);
    font-size: 20px;
    cursor: pointer;
    line-height: 1;
    padding: 4px 8px;
    border-radius: 6px;
    transition: all 0.15s;
  }
  .cleanup-close:hover {
    background: var(--bg-tertiary);
    color: var(--text-primary);
  }

  .cleanup-body {
    padding: 20px;
  }

  .cleanup-info {
    font-size: 13px;
    color: var(--text-secondary);
    margin: 0 0 12px 0;
    line-height: 1.5;
  }
  .cleanup-info strong {
    color: var(--text-primary);
  }
  .cleanup-info a {
    color: var(--accent);
    text-decoration: none;
  }
  .cleanup-info a:hover {
    text-decoration: underline;
  }

  .cleanup-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    justify-content: flex-end;
    padding: 12px 20px;
    border-top: 1px solid var(--border-primary);
  }

  .form-fields {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .form-label {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .form-label-text {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .form-input {
    padding: 10px 12px;
    border-radius: 8px;
    border: 1px solid var(--border-primary);
    background: var(--bg-primary);
    color: var(--text-primary);
    font-size: 14px;
    outline: none;
    transition: border-color 0.15s;
  }
  .form-input:focus {
    border-color: var(--accent);
  }
  .form-input::placeholder {
    color: var(--text-tertiary);
  }

  .form-input-code {
    font-size: 20px;
    letter-spacing: 6px;
    text-align: center;
    font-family: var(--font-mono, monospace);
    max-width: 180px;
  }

  .form-error {
    font-size: 12px;
    color: var(--red, #ef4444);
    margin: 0;
    padding: 8px 12px;
    background: rgba(239, 68, 68, 0.1);
    border-radius: 6px;
    border: 1px solid rgba(239, 68, 68, 0.2);
  }

  .form-loading {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 16px 0;
    font-size: 13px;
    color: var(--text-secondary);
  }

  .dd-credit {
    font-size: 11px;
    color: var(--text-tertiary);
    margin-right: auto;
  }
  .dd-credit a {
    color: var(--text-tertiary);
    text-decoration: underline;
  }
</style>
