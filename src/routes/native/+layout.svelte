<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { get } from "svelte/store";
  import { applyNativeTheme } from "$lib/native/theme";
  import { setNativeMode } from "$lib/api";
  import { nativeMode } from "$lib/stores";

  let { children } = $props();

  // Safety net for direct-URL navigation to /native/* (back button, bookmark,
  // refresh). When the topbar toggle drives the transition, the store is
  // already correct — we skip redundant work to avoid racing the toggle's
  // own applyNativeTheme call.
  onMount(() => {
    if (!get(nativeMode)) {
      setNativeMode(true).catch((err) => console.warn("setNativeMode failed:", err));
      nativeMode.set(true);
      applyNativeTheme(true).catch((err) => console.warn("applyNativeTheme(true) failed:", err));
    }
  });

  // Auto-revert ONLY when leaving /native/* via a non-toggle path
  // (sidebar click, direct URL change). The topbar toggle handles its own
  // theme revert before navigating away, so this is a fallback only.
  onDestroy(() => {
    if (get(nativeMode)) {
      nativeMode.set(false);
      applyNativeTheme(false).catch((err) => console.warn("applyNativeTheme(false) failed:", err));
    }
  });
</script>

<div class="native-shell">
  {@render children()}
</div>

<style>
  .native-shell {
    min-height: 100vh;
    padding: 0;
    background: var(--bg-base);
    color: var(--text-primary);
  }
</style>
