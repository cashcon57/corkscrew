<script lang="ts">
  import { goto } from "$app/navigation";
  import { setNativeMode } from "$lib/api";
  import { nativeMode } from "$lib/stores";

  async function disableNativeMode() {
    await setNativeMode(false).catch((err) => console.error("setNativeMode(false) failed:", err));
    nativeMode.set(false);
    goto("/");
  }
</script>

<div class="page">
  <h1 class="m5-gradient-text">Native Settings</h1>
  <p class="subtitle">Phase 1 stub &mdash; full settings land in Phase 6.</p>

  <div class="native-glass-card">
    <h3>Disable Native Mode</h3>
    <p>Return to Wine/CrossOver workflow. Your installed mods are preserved.</p>
    <button class="danger" onclick={disableNativeMode}>Exit Native Mode</button>
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
  h3 {
    font-size: 16px;
    margin: 0 0 8px;
  }
  p {
    color: var(--text-secondary);
    margin: 0 0 16px;
  }
  .danger {
    background: var(--red);
    color: white;
    border: none;
    padding: 10px 16px;
    border-radius: 8px;
    cursor: pointer;
  }
</style>
