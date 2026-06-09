<script lang="ts">
  import { onMount } from "svelte";
  import { selectedGame } from "$lib/stores";
  import { games as detectedGames } from "$lib/stores";
  import { getNexusAccountStatus } from "$lib/api";
  import NexusBrowsePanel from "$lib/components/collections/NexusBrowsePanel.svelte";

  type AccountStatus = {
    connected: boolean;
    is_premium?: boolean;
    name?: string;
    avatar?: string | null;
    auth_type?: string;
  };

  let account = $state<AccountStatus | null>(null);

  onMount(async () => {
    try {
      account = await getNexusAccountStatus();
    } catch (err) {
      console.warn("getNexusAccountStatus failed:", err);
      account = { connected: false };
    }
  });
</script>

<div class="page">
  <h1 class="m5-gradient-text">Discover</h1>
  <p class="subtitle">Browse and install mods from NexusMods.</p>

  {#if !$selectedGame}
    <div class="native-glass-card empty">
      <h2>Select a game first</h2>
      <p>
        Pick a native game from the dropdown in the top bar. Browsing requires
        a selected game so Corkscrew knows which NexusMods category to query.
      </p>
    </div>
  {:else if $selectedGame.runtime.runtime !== "native"}
    <div class="native-glass-card empty">
      <h2>Selected game isn't a native game</h2>
      <p>
        <strong>{$selectedGame.display_name}</strong> is a Wine game. Switch to Wine Mode
        from the top bar to browse mods for it, or pick a native game from the dropdown.
      </p>
    </div>
  {:else}
    <NexusBrowsePanel
      game={$selectedGame}
      {account}
      allDetectedGames={$detectedGames}
    />
  {/if}
</div>

<style>
  .page {
    max-width: 1400px;
    margin: 0 auto;
    padding: 32px 24px;
  }
  h1 {
    font-size: 36px;
    font-weight: 700;
    letter-spacing: -0.02em;
    margin: 0 0 8px;
  }
  .subtitle {
    color: var(--text-secondary);
    margin: 0 0 24px;
    font-size: 16px;
  }
  .empty {
    padding: 48px 32px;
    text-align: center;
  }
  .empty h2 {
    font-size: 20px;
    font-weight: 600;
    margin: 0 0 8px;
    color: var(--text-primary);
  }
  .empty p {
    color: var(--text-secondary);
    margin: 0;
    line-height: 1.5;
  }
  .empty strong {
    color: var(--text-primary);
  }
</style>
