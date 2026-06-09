<script lang="ts">
  import { onMount } from "svelte";
  import { selectedGame } from "$lib/stores";
  import { getInstalledMods, toggleMod, uninstallMod } from "$lib/api";
  import { wineCtx } from "$lib/types";
  import type { InstalledMod, DetectedGame } from "$lib/types";

  let mods = $state<InstalledMod[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let busyId = $state<number | null>(null);

  function nativeBottleName(g: DetectedGame): string {
    // Native games use empty string as the bottle_name sentinel.
    // wineCtx is null for native — returns the empty string we want.
    return wineCtx(g)?.bottle_name ?? "";
  }

  async function loadMods() {
    const g = $selectedGame;
    if (!g) { mods = []; return; }
    loading = true;
    error = null;
    try {
      mods = await getInstalledMods(g.game_id, nativeBottleName(g));
    } catch (e) {
      console.error("getInstalledMods failed:", e);
      error = String(e);
      mods = [];
    } finally {
      loading = false;
    }
  }

  onMount(loadMods);

  // Reload whenever the selected game changes.
  $effect(() => {
    const _ = $selectedGame?.game_id;
    loadMods();
  });

  async function handleToggle(m: InstalledMod) {
    const g = $selectedGame;
    if (!g) return;
    busyId = m.id;
    try {
      await toggleMod(m.id, g.game_id, nativeBottleName(g), !m.enabled);
      await loadMods();
    } catch (e) {
      console.error("toggleMod failed:", e);
      error = String(e);
    } finally {
      busyId = null;
    }
  }

  async function handleDelete(m: InstalledMod) {
    const g = $selectedGame;
    if (!g) return;
    if (!confirm(`Delete "${m.name}"? This removes the mod from disk and your install record.`)) return;
    busyId = m.id;
    try {
      await uninstallMod(m.id, g.game_id, nativeBottleName(g));
      await loadMods();
    } catch (e) {
      console.error("uninstallMod failed:", e);
      error = String(e);
    } finally {
      busyId = null;
    }
  }
</script>

<div class="page">
  <h1 class="m5-gradient-text">Mods</h1>
  <p class="subtitle">
    {#if $selectedGame}
      Installed mods for <strong>{$selectedGame.display_name}</strong>.
    {:else}
      macOS-native modding for the selected game.
    {/if}
  </p>

  {#if !$selectedGame}
    <div class="native-glass-card empty">
      <h2>No game selected</h2>
      <p>Pick a native game from the dropdown in the top bar.</p>
    </div>
  {:else if loading}
    <div class="native-glass-card empty"><p>Loading mods…</p></div>
  {:else if error}
    <div class="native-glass-card empty error"><p>Failed to load mods: {error}</p></div>
  {:else if mods.length === 0}
    <div class="native-glass-card empty">
      <h2>No mods installed yet</h2>
      <p>Visit <a href="/native/discover">Discover</a> to browse mods on NexusMods.</p>
    </div>
  {:else}
    <div class="mod-list">
      {#each mods as m (m.id)}
        <div class="native-glass-card mod-row" class:disabled={!m.enabled}>
          <div class="mod-main">
            <div class="mod-name">{m.name}</div>
            <div class="mod-meta">
              {#if m.version}<span class="meta-chip">v{m.version}</span>{/if}
              <span class="meta-chip">{m.file_count} files</span>
              {#if m.source_type === "nexus"}<span class="meta-chip">Nexus</span>{/if}
              {#if m.collection_name}<span class="meta-chip">in {m.collection_name}</span>{/if}
              {#if m.auto_category}<span class="meta-chip">{m.auto_category}</span>{/if}
            </div>
          </div>
          <div class="mod-actions">
            <button
              class="toggle"
              class:on={m.enabled}
              onclick={() => handleToggle(m)}
              disabled={busyId === m.id}
              title={m.enabled ? "Disable mod" : "Enable mod"}
            >
              {m.enabled ? "Enabled" : "Disabled"}
            </button>
            <button
              class="danger"
              onclick={() => handleDelete(m)}
              disabled={busyId === m.id}
              title="Delete mod"
            >
              Delete
            </button>
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .page {
    max-width: 1200px;
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
  }
  .empty.error p { color: var(--red, #ff453a); }
  .empty a {
    color: var(--m5-cyan, var(--accent));
    text-decoration: none;
  }
  .empty a:hover { text-decoration: underline; }

  .mod-list {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .mod-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 16px 20px;
    border-radius: 12px;
  }
  .mod-row.disabled {
    opacity: 0.55;
  }
  .mod-main {
    flex: 1;
    min-width: 0;
  }
  .mod-name {
    font-size: 16px;
    font-weight: 600;
    color: var(--text-primary);
    margin: 0 0 6px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mod-meta {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
  .meta-chip {
    background: var(--surface, rgba(255,255,255,0.04));
    color: var(--text-secondary);
    padding: 2px 8px;
    border-radius: 10px;
    font-size: 11px;
  }

  .mod-actions {
    display: flex;
    gap: 8px;
    flex-shrink: 0;
  }
  .toggle, .danger {
    border: 1px solid var(--separator);
    background: transparent;
    color: var(--text-secondary);
    padding: 6px 12px;
    border-radius: 8px;
    cursor: pointer;
    font-size: 13px;
  }
  .toggle.on {
    background: var(--accent-subtle, rgba(91,115,255,0.16));
    color: var(--accent, var(--m5-blue));
    border-color: var(--accent-muted, rgba(91,115,255,0.4));
  }
  .toggle:disabled, .danger:disabled {
    opacity: 0.4;
    cursor: wait;
  }
  .danger {
    color: var(--red, #ff453a);
    border-color: rgba(255,69,58,0.3);
  }
  .danger:hover:not(:disabled) {
    background: rgba(255,69,58,0.1);
  }
</style>
