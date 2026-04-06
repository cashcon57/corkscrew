<script lang="ts">
  import { setConfigValue } from "$lib/api";
  import { initSentry, teardownSentry } from "$lib/sentry";

  interface Props {
    visible: boolean;
  }

  let { visible = $bindable(false) }: Props = $props();

  async function accept() {
    try {
      await setConfigValue("telemetry_consent", "granted");
      await initSentry();
    } catch (err) {
      console.error("Failed to save telemetry consent:", err);
    }
    visible = false;
  }

  async function decline() {
    try {
      await setConfigValue("telemetry_consent", "denied");
      teardownSentry();
    } catch (err) {
      console.error("Failed to save telemetry consent:", err);
    }
    visible = false;
  }
</script>

{#if visible}
  <div class="telemetry-banner">
    <div class="banner-content">
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="flex-shrink: 0; opacity: 0.6;">
        <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
      </svg>
      <span class="banner-text">
        Help improve Corkscrew by sending anonymous crash reports. No personal data, mod lists, or file paths are collected.
      </span>
    </div>
    <div class="banner-actions">
      <button class="banner-btn banner-btn-accept" onclick={accept}>Accept</button>
      <button class="banner-btn banner-btn-decline" onclick={decline}>No thanks</button>
    </div>
  </div>
{/if}

<style>
  .telemetry-banner {
    position: fixed;
    bottom: var(--space-3);
    left: var(--space-4);
    right: var(--space-4);
    background: color-mix(in srgb, var(--bg-elevated) 75%, transparent);
    backdrop-filter: var(--glass-blur-heavy);
    -webkit-backdrop-filter: var(--glass-blur-heavy);
    border: 0.5px solid rgba(255, 255, 255, 0.12);
    border-radius: var(--radius-lg);
    padding: 12px 20px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    z-index: 9999;
    box-shadow:
      inset 0 1px 0 0 rgba(255, 255, 255, 0.12),
      inset 0 -1px 0 0 rgba(255, 255, 255, 0.04),
      0 8px 32px rgba(0, 0, 0, 0.3),
      0 2px 8px rgba(0, 0, 0, 0.15);
    animation: bannerSlideUp 0.3s ease-out;
  }

  .banner-content {
    display: flex;
    align-items: center;
    gap: 10px;
    flex: 1;
    min-width: 0;
  }

  .banner-text {
    font-size: 12.5px;
    color: var(--text-secondary);
    line-height: 1.4;
  }

  .banner-actions {
    display: flex;
    gap: 8px;
    flex-shrink: 0;
  }

  .banner-btn {
    padding: 6px 14px;
    border-radius: var(--radius);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    border: none;
    transition: all var(--duration-fast) var(--ease);
  }

  .banner-btn-accept {
    background: var(--accent);
    color: white;
  }

  .banner-btn-accept:hover {
    filter: brightness(1.1);
  }

  .banner-btn-decline {
    background: var(--surface-hover);
    color: var(--text-secondary);
  }

  .banner-btn-decline:hover {
    background: var(--surface-active);
    color: var(--text-primary);
  }

  @keyframes bannerSlideUp {
    from {
      opacity: 0;
      transform: translateY(100%);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }
</style>
