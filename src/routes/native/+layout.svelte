<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { applyNativeTheme } from "$lib/native/theme";
  import { setNativeMode } from "$lib/api";
  import { nativeMode } from "$lib/stores";

  let { children } = $props();

  onMount(() => {
    applyNativeTheme(true).catch((err) => console.warn("applyNativeTheme(true) failed:", err));
    setNativeMode(true).catch((err) => console.warn("setNativeMode failed:", err));
    nativeMode.set(true);
  });

  onDestroy(() => {
    applyNativeTheme(false).catch((err) => console.warn("applyNativeTheme(false) failed:", err));
    nativeMode.set(false);
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
