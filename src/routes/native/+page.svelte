<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { rescanNativeGames } from "$lib/api";

  onMount(async () => {
    try {
      const candidates = await rescanNativeGames();
      if (candidates.length > 0) {
        goto("/native/mods");
      } else {
        goto("/native/discover");
      }
    } catch (err) {
      console.warn("rescanNativeGames failed:", err);
      goto("/native/discover");
    }
  });
</script>

<div class="native-loading">
  <p>Loading native games&hellip;</p>
</div>

<style>
  .native-loading {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 60vh;
    color: var(--text-secondary);
  }
</style>
