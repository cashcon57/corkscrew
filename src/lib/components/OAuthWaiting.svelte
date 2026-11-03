<script lang="ts">
  // Shown while a Nexus Mods sign-in waits for the browser. Offers the
  // sign-in link for pasting into another browser or profile, since the OS
  // opens whichever browser profile was used last.
  interface Props {
    authUrl: string | null;
    onCancel: () => void;
  }

  let { authUrl, onCancel }: Props = $props();

  let copied = $state(false);
  let copyError = $state(false);
  let copiedTimer: ReturnType<typeof setTimeout> | null = null;

  async function copyLink() {
    if (!authUrl) return;
    try {
      await navigator.clipboard.writeText(authUrl);
      copyError = false;
      copied = true;
      if (copiedTimer) clearTimeout(copiedTimer);
      copiedTimer = setTimeout(() => { copied = false; }, 2000);
    } catch (err) {
      console.error("Failed to copy sign-in link:", err);
      copyError = true;
    }
  }

  $effect(() => () => {
    if (copiedTimer) clearTimeout(copiedTimer);
  });
</script>

<div class="oauth-waiting">
  <div class="oauth-waiting-row">
    <span class="spinner-sm"></span>
    <div class="oauth-waiting-text">
      <span class="oauth-waiting-title">Waiting for sign-in...</span>
      <span class="oauth-waiting-subtitle">Finish signing in to Nexus Mods in your browser, then come back here.</span>
    </div>
    <button class="btn-ghost btn-sm" onclick={onCancel} type="button">Cancel</button>
  </div>

  {#if authUrl}
    <div class="oauth-link">
      <span class="oauth-link-hint">Wrong browser or profile? Copy this link and open it where you're signed in:</span>
      <div class="oauth-link-row">
        <input
          class="oauth-link-input"
          type="text"
          readonly
          value={authUrl}
          aria-label="Nexus Mods sign-in link"
          onfocus={(e) => (e.currentTarget as HTMLInputElement).select()}
        />
        <button class="btn-ghost btn-sm" onclick={copyLink} type="button">
          {copied ? "Copied" : "Copy link"}
        </button>
      </div>
      {#if copyError}
        <span class="oauth-link-error">Couldn't copy. Select the link and copy it manually.</span>
      {/if}
    </div>
  {/if}
</div>

<style>
  .oauth-waiting {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-3);
    background: var(--surface-hover);
    border-radius: var(--radius-md, 8px);
    text-align: left;
  }

  .oauth-waiting-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
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

  .oauth-waiting-subtitle,
  .oauth-link-hint {
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .oauth-link {
    display: flex;
    flex-direction: column;
    gap: var(--space-1, 4px);
  }

  .oauth-link-row {
    display: flex;
    gap: var(--space-2, 8px);
    align-items: center;
  }

  .oauth-link-input {
    flex: 1;
    min-width: 0;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 11px;
    padding: 4px 8px;
    color: var(--text-secondary);
    background: var(--bg-base, transparent);
    border: 1px solid var(--separator, rgba(255, 255, 255, 0.1));
    border-radius: var(--radius-sm, 4px);
    text-overflow: ellipsis;
  }

  .oauth-link-error {
    font-size: 11px;
    color: var(--red);
  }
</style>
