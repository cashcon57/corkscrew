<script lang="ts">
  /**
   * One-time anti-cheat acknowledgment dialog.
   *
   * Shown before installing a mod for any game whose modding stack involves
   * injecting a graphics or runtime hook into a process that ships
   * anti-cheat. Currently used for Genshin Impact (HoyoProtect detects
   * 3DMigoto / GIMI loader injection); future phases extend this to
   * Honkai: Star Rail, Zenless Zone Zero, and Honkai 3rd.
   *
   * The acceptance state is persisted by the caller via the existing
   * `set_config_value` plumbing under
   * `anti_cheat_warning_accepted_<game_id>`. Once accepted, the parent
   * route bypasses this dialog for that game.
   */

  interface Props {
    gameName: string;
    onAccept: () => void;
    onCancel: () => void;
  }

  let { gameName, onAccept, onCancel }: Props = $props();

  let dismissing = $state(false);

  function handleDismiss(callback: () => void) {
    if (dismissing) return;
    dismissing = true;
    setTimeout(() => {
      dismissing = false;
      callback();
    }, 200);
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      handleDismiss(onCancel);
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="ac-overlay" onclick={() => handleDismiss(onCancel)} role="dialog" aria-label="Anti-cheat risk acknowledgment">
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="ac-card" class:ac-dismissing={dismissing} onclick={(e) => e.stopPropagation()}>
    <div class="ac-header">
      <span class="ac-warning-icon" aria-hidden="true">!</span>
      <h3 class="ac-title">Anti-cheat risk acknowledgment</h3>
    </div>

    <p class="ac-message">
      Modifying {gameName} requires injecting a graphics hook (3DMigoto / GIMI)
      into the running game. {gameName} uses anti-cheat (HoyoProtect) which
      <em>may</em> detect this and result in account suspension or ban.
    </p>

    <ul class="ac-details">
      <li>
        Bans are <strong>rare</strong> for cosmetic-only mods (texture and
        character swaps), but the risk is non-zero.
      </li>
      <li>Do not install gameplay-altering mods.</li>
      <li>
        Use a separate account if you want to be safe.
      </li>
      <li>You won't see this warning again for this game.</li>
    </ul>

    <div class="ac-actions">
      <button class="btn btn-ghost" onclick={() => handleDismiss(onCancel)}>
        Cancel
      </button>
      <button class="btn btn-danger" onclick={() => handleDismiss(onAccept)}>
        I understand the risk &mdash; continue
      </button>
    </div>
  </div>
</div>

<style>
  .ac-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.65);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
    animation: ac-fade-in 150ms ease-out;
  }

  .ac-card {
    background: color-mix(in srgb, var(--bg-secondary) 75%, transparent);
    backdrop-filter: var(--glass-blur-heavy);
    -webkit-backdrop-filter: var(--glass-blur-heavy);
    border: 1px solid rgba(255, 69, 58, 0.35);
    border-radius: var(--radius);
    padding: var(--space-6);
    max-width: 480px;
    width: 90vw;
    animation: ac-slide-up 200ms var(--ease-out);
    box-shadow:
      var(--glass-refraction),
      var(--glass-edge-shadow),
      var(--shadow-lg),
      0 0 24px rgba(255, 69, 58, 0.15);
  }

  .ac-header {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin: 0 0 var(--space-3);
  }

  .ac-warning-icon {
    flex: 0 0 auto;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    background: rgba(255, 69, 58, 0.18);
    color: rgb(255, 99, 88);
    border: 1px solid rgba(255, 69, 58, 0.5);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-weight: 700;
    font-size: 16px;
    line-height: 1;
  }

  .ac-title {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .ac-message {
    margin: 0 0 var(--space-3);
    font-size: 13px;
    color: var(--text-secondary);
    line-height: 1.5;
  }

  .ac-message em {
    color: rgb(255, 149, 0);
    font-style: normal;
    font-weight: 600;
  }

  .ac-details {
    margin: 0 0 var(--space-4);
    padding-left: var(--space-5);
    font-size: 12px;
    color: var(--text-tertiary);
    line-height: 1.6;
  }

  .ac-actions {
    display: flex;
    gap: var(--space-2);
    justify-content: flex-end;
  }

  .ac-card.ac-dismissing {
    animation: glass-dialog-dismiss 0.2s var(--ease-in) forwards;
  }

  .ac-actions :global(.btn-danger):hover {
    box-shadow: 0 0 12px rgba(255, 69, 58, 0.35);
  }

  @keyframes ac-fade-in {
    from {
      opacity: 0;
      backdrop-filter: blur(0);
    }
    to {
      opacity: 1;
      backdrop-filter: blur(8px);
    }
  }

  @keyframes ac-slide-up {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  :global(html:not(.vibrancy-active)) .ac-card {
    background: var(--bg-secondary);
    backdrop-filter: none;
    -webkit-backdrop-filter: none;
  }

  :global(html:not(.vibrancy-active)) .ac-overlay {
    backdrop-filter: none;
  }
</style>
