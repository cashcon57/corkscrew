<script lang="ts">
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { checkModUpdates } from "$lib/api";
  import { showError, showSuccess } from "$lib/stores";
  import type { DetectedGame, ModUpdateInfo } from "$lib/types";

  let {
    game,
    onUpdatesChecked,
  }: {
    game: DetectedGame;
    onUpdatesChecked?: (updates: ModUpdateInfo[]) => void;
  } = $props();

  let modUpdates = $state<ModUpdateInfo[]>([]);
  let checkingUpdates = $state(false);

  async function handleCheckUpdates() {
    checkingUpdates = true;
    try {
      modUpdates = await checkModUpdates(game.game_id, game.bottle_name);
      if (modUpdates.length === 0) {
        showSuccess("All mods are up to date");
      } else {
        showSuccess(`${modUpdates.length} update${modUpdates.length > 1 ? "s" : ""} available`);
      }
      onUpdatesChecked?.(modUpdates);
    } catch (e: unknown) {
      showError(`Failed to check for updates: ${e}`);
    } finally {
      checkingUpdates = false;
    }
  }

  async function handleUpdateAll() {
    if (modUpdates.length === 0) return;
    const gameSlug = game.nexus_slug || game.game_id;
    let opened = 0;
    for (const update of modUpdates) {
      try {
        await openUrl(`https://www.nexusmods.com/${gameSlug}/mods/${update.nexus_mod_id}?tab=files`);
        opened++;
      } catch {
        // Skip mods that fail to open
      }
    }
    if (opened > 0) {
      showSuccess(`Opened ${opened} Nexus mod page${opened !== 1 ? "s" : ""} for updating`);
    }
  }
</script>

<button
  class="btn btn-ghost"
  onclick={handleCheckUpdates}
  disabled={checkingUpdates}
  title="Check Nexus for mod updates"
>
  {#if checkingUpdates}
    <span class="spinner spinner-sm"></span>
    Checking...
  {:else}
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <polyline points="23 4 23 10 17 10" />
      <path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10" />
    </svg>
    Updates
    {#if modUpdates.length > 0}
      <span class="update-count-badge">{modUpdates.length}</span>
    {/if}
  {/if}
</button>
{#if modUpdates.length > 0}
  <button
    class="btn btn-accent btn-sm"
    onclick={handleUpdateAll}
    title="Open Nexus download pages for all outdated mods"
  >
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
      <polyline points="7 10 12 15 17 10" />
      <line x1="12" y1="15" x2="12" y2="3" />
    </svg>
    Update All ({modUpdates.length})
  </button>
{/if}
