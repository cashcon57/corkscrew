<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { setNativeMode, rescanNativeGames } from "$lib/api";
  import { nativeMode } from "$lib/stores";

  const DISMISSED_KEY = "native-banner-dismissed";
  let visible = $state(false);

  onMount(async () => {
    if (typeof localStorage === "undefined") return;
    if (localStorage.getItem(DISMISSED_KEY) === "1") return;
    if ($nativeMode) return;
    try {
      const candidates = await rescanNativeGames();
      if (candidates.length > 0) visible = true;
    } catch (err) {
      console.warn("NativeModeBanner: scan failed:", err);
    }
  });

  async function enable() {
    visible = false;
    try {
      await setNativeMode(true);
      nativeMode.set(true);
      goto("/native");
    } catch (err) {
      console.error("NativeModeBanner: enable failed:", err);
    }
  }

  function dismiss() {
    visible = false;
    try { localStorage.setItem(DISMISSED_KEY, "1"); } catch {}
  }
</script>

{#if visible}
  <div class="native-banner native-glass-card" role="alert">
    <div class="content">
      <h3 class="m5-gradient-text">Try Native Mode (beta)</h3>
      <p>Mod your macOS-native Stardew Valley and Baldur's Gate 3 installs.</p>
    </div>
    <div class="actions">
      <button class="cta" onclick={enable}>Enable Native Mode</button>
      <button class="dismiss" onclick={dismiss} aria-label="Dismiss">×</button>
    </div>
  </div>
{/if}

<style>
  .native-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 16px 20px;
    margin: 12px 24px;
    border-radius: 12px;
  }
  .content { flex: 1; }
  h3 {
    font-size: 16px;
    font-weight: 600;
    margin: 0 0 4px;
    letter-spacing: -0.01em;
  }
  p {
    margin: 0;
    color: var(--text-secondary);
    font-size: 13px;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .cta {
    background: var(--m5-gradient, var(--accent));
    color: white;
    border: none;
    padding: 8px 14px;
    border-radius: 8px;
    font-weight: 600;
    cursor: pointer;
    font-size: 13px;
  }
  .dismiss {
    background: transparent;
    border: none;
    color: var(--text-tertiary);
    font-size: 22px;
    cursor: pointer;
    padding: 0 8px;
    line-height: 1;
  }
  .dismiss:hover { color: var(--text-primary); }
</style>
