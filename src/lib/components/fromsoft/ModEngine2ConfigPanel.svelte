<script lang="ts">
  /**
   * Mod Engine 2 (`modengine2.toml`) editor panel.
   *
   * Lets users:
   * - reorder the [[extension.mod_loader.mods]] array (ME2 honors order
   *   for conflict resolution: later entries win)
   * - toggle each entry's `enabled` flag
   * - edit `[modengine].external_dlls` (the list of DLLs ME2 forces alongside)
   *
   * Path-of-discovery + atomic save are handled in the Rust backend; this
   * panel is a thin UI wrapper over `getModEngine2Config` /
   * `saveModEngine2Config`. Re-fetches after each save to stay in sync
   * with whatever the backend wrote (e.g. ME2's TOML formatter pass).
   */

  import {
    getModEngine2Config,
    saveModEngine2Config,
  } from "$lib/api";
  import type {
    ModEngine2Config,
    ModEngine2ModEntry,
  } from "$lib/types";

  interface Props {
    gameId: string;
    bottleName: string;
  }

  let { gameId, bottleName }: Props = $props();

  let config = $state<ModEngine2Config | null>(null);
  let loading = $state(false);
  let saving = $state(false);
  let error = $state<string | null>(null);
  let dirty = $state(false);
  let newDll = $state("");

  // Drag-drop reorder state
  let dragSourceIdx = $state<number | null>(null);

  $effect(() => {
    void load();
  });

  async function load() {
    loading = true;
    error = null;
    try {
      config = await getModEngine2Config(gameId, bottleName);
      dirty = false;
    } catch (err) {
      console.error("ModEngine2ConfigPanel.load:", err);
      error = String(err);
      config = null;
    } finally {
      loading = false;
    }
  }

  async function save() {
    if (!config) return;
    saving = true;
    error = null;
    try {
      await saveModEngine2Config(gameId, bottleName, config);
      dirty = false;
      // Re-fetch to pick up backend canonicalization.
      config = await getModEngine2Config(gameId, bottleName);
    } catch (err) {
      console.error("ModEngine2ConfigPanel.save:", err);
      error = String(err);
    } finally {
      saving = false;
    }
  }

  function toggleMod(idx: number) {
    if (!config) return;
    config.mod_loader.mods[idx].enabled = !config.mod_loader.mods[idx].enabled;
    dirty = true;
  }

  function moveMod(fromIdx: number, toIdx: number) {
    if (!config) return;
    const mods = [...config.mod_loader.mods];
    if (fromIdx < 0 || fromIdx >= mods.length || toIdx < 0 || toIdx >= mods.length) return;
    if (fromIdx === toIdx) return;
    const [item] = mods.splice(fromIdx, 1);
    mods.splice(toIdx, 0, item);
    config.mod_loader.mods = mods;
    dirty = true;
  }

  function moveUp(idx: number) {
    moveMod(idx, idx - 1);
  }
  function moveDown(idx: number) {
    moveMod(idx, idx + 1);
  }

  function onDragStart(idx: number) {
    dragSourceIdx = idx;
  }
  function onDragOver(e: DragEvent) {
    e.preventDefault();
  }
  function onDrop(idx: number) {
    if (dragSourceIdx === null) return;
    moveMod(dragSourceIdx, idx);
    dragSourceIdx = null;
  }

  function addExternalDll() {
    if (!config) return;
    const trimmed = newDll.trim();
    if (!trimmed) return;
    if (!config.modengine.external_dlls.includes(trimmed)) {
      config.modengine.external_dlls = [...config.modengine.external_dlls, trimmed];
      dirty = true;
    }
    newDll = "";
  }

  function removeExternalDll(dll: string) {
    if (!config) return;
    config.modengine.external_dlls = config.modengine.external_dlls.filter((d) => d !== dll);
    dirty = true;
  }

  function toggleDebug() {
    if (!config) return;
    config.modengine.debug = !config.modengine.debug;
    dirty = true;
  }

  function toggleLooseParams() {
    if (!config) return;
    config.mod_loader.loose_params = !config.mod_loader.loose_params;
    dirty = true;
  }
</script>

<section class="me2-panel">
  <header class="me2-header">
    <h3 class="me2-title">Mod Engine 2 Configuration</h3>
    <div class="me2-actions">
      <button class="btn btn-ghost" onclick={load} disabled={loading || saving}>
        Reload
      </button>
      <button
        class="btn btn-primary"
        onclick={save}
        disabled={!dirty || saving || loading || !config}
      >
        {saving ? "Saving…" : dirty ? "Save changes" : "Saved"}
      </button>
    </div>
  </header>

  {#if loading}
    <p class="me2-loading">Loading modengine2.toml…</p>
  {:else if error}
    <p class="me2-error">Error: {error}</p>
  {:else if !config}
    <p class="me2-empty">No Mod Engine 2 configuration found yet. Install Mod Engine 2 first.</p>
  {:else}
    <div class="me2-section">
      <h4>Loader settings</h4>
      <label class="me2-toggle">
        <input type="checkbox" checked={config.modengine.debug} onchange={toggleDebug} />
        Debug logging
      </label>
      <label class="me2-toggle">
        <input
          type="checkbox"
          checked={config.mod_loader.loose_params}
          onchange={toggleLooseParams}
        />
        Loose params (load .param overrides from disk)
      </label>
    </div>

    <div class="me2-section">
      <h4>Active mods (load order)</h4>
      <p class="me2-hint">
        Order matters — later entries override earlier ones at deploy time.
        Drag to reorder, or use the arrow buttons.
      </p>

      {#if config.mod_loader.mods.length === 0}
        <p class="me2-empty">No mods registered with Mod Engine 2 yet.</p>
      {:else}
        <ul class="me2-mod-list">
          {#each config.mod_loader.mods as mod, idx (mod.name + idx)}
            <li
              class="me2-mod-row"
              class:disabled={!mod.enabled}
              draggable="true"
              ondragstart={() => onDragStart(idx)}
              ondragover={onDragOver}
              ondrop={() => onDrop(idx)}
            >
              <span class="me2-grip" aria-hidden="true">⋮⋮</span>
              <input
                class="me2-checkbox"
                type="checkbox"
                checked={mod.enabled}
                onchange={() => toggleMod(idx)}
                aria-label={`Enable ${mod.name}`}
              />
              <span class="me2-mod-name">{mod.name}</span>
              <span class="me2-mod-path">{mod.path}</span>
              <div class="me2-mod-controls">
                <button
                  class="btn btn-ghost btn-sm"
                  onclick={() => moveUp(idx)}
                  disabled={idx === 0}
                  aria-label="Move up"
                >
                  ↑
                </button>
                <button
                  class="btn btn-ghost btn-sm"
                  onclick={() => moveDown(idx)}
                  disabled={idx === config.mod_loader.mods.length - 1}
                  aria-label="Move down"
                >
                  ↓
                </button>
              </div>
            </li>
          {/each}
        </ul>
      {/if}
    </div>

    <div class="me2-section">
      <h4>External DLLs</h4>
      <p class="me2-hint">
        DLLs Mod Engine 2 force-loads into the game process alongside the loader.
      </p>
      <div class="me2-dll-add">
        <input
          type="text"
          class="me2-input"
          placeholder="e.g. dinput8.dll"
          bind:value={newDll}
          onkeydown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              addExternalDll();
            }
          }}
        />
        <button class="btn btn-ghost" onclick={addExternalDll} disabled={!newDll.trim()}>
          Add
        </button>
      </div>

      {#if config.modengine.external_dlls.length === 0}
        <p class="me2-empty-inline">No external DLLs configured.</p>
      {:else}
        <ul class="me2-dll-list">
          {#each config.modengine.external_dlls as dll (dll)}
            <li class="me2-dll-row">
              <code>{dll}</code>
              <button
                class="btn btn-ghost btn-sm"
                onclick={() => removeExternalDll(dll)}
                aria-label={`Remove ${dll}`}
              >
                Remove
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {/if}
</section>

<style>
  .me2-panel {
    background: var(--bg-card, #1e1e22);
    border: 1px solid var(--border, #2c2c33);
    border-radius: 8px;
    padding: 1rem 1.25rem;
    margin: 1rem 0;
  }
  .me2-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 1rem;
  }
  .me2-title {
    margin: 0;
    font-size: 1.05rem;
    font-weight: 600;
  }
  .me2-actions {
    display: flex;
    gap: 0.5rem;
  }
  .me2-section {
    margin-bottom: 1.25rem;
  }
  .me2-section h4 {
    font-size: 0.9rem;
    font-weight: 600;
    margin: 0 0 0.5rem 0;
    color: var(--fg-muted, #b0b0b8);
  }
  .me2-hint {
    font-size: 0.8rem;
    color: var(--fg-muted, #888);
    margin: 0 0 0.5rem 0;
  }
  .me2-toggle {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin: 0.25rem 0;
    font-size: 0.9rem;
    cursor: pointer;
  }
  .me2-mod-list {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .me2-mod-row {
    display: grid;
    grid-template-columns: 24px 24px 1fr 2fr auto;
    gap: 0.5rem;
    align-items: center;
    padding: 0.4rem 0.5rem;
    background: var(--bg-row, #25252b);
    border: 1px solid transparent;
    border-radius: 4px;
    cursor: grab;
  }
  .me2-mod-row:hover {
    border-color: var(--border-hover, #3c3c44);
  }
  .me2-mod-row.disabled {
    opacity: 0.55;
  }
  .me2-grip {
    color: var(--fg-muted, #555);
    user-select: none;
  }
  .me2-mod-name {
    font-weight: 500;
  }
  .me2-mod-path {
    font-family: var(--font-mono, monospace);
    font-size: 0.8rem;
    color: var(--fg-muted, #888);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .me2-mod-controls {
    display: flex;
    gap: 4px;
  }
  .me2-dll-add {
    display: flex;
    gap: 0.5rem;
    margin-bottom: 0.5rem;
  }
  .me2-input {
    flex: 1;
    background: var(--bg-input, #1a1a1f);
    border: 1px solid var(--border, #333);
    color: var(--fg, #e8e8ee);
    padding: 0.35rem 0.5rem;
    border-radius: 4px;
    font-size: 0.9rem;
  }
  .me2-dll-list {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .me2-dll-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0.3rem 0.5rem;
    background: var(--bg-row, #25252b);
    border-radius: 4px;
  }
  .me2-dll-row code {
    font-size: 0.85rem;
  }
  .me2-loading,
  .me2-error,
  .me2-empty,
  .me2-empty-inline {
    font-size: 0.9rem;
    color: var(--fg-muted, #888);
  }
  .me2-error {
    color: var(--danger, #e35d5d);
  }
  .btn-sm {
    padding: 2px 6px;
    font-size: 0.8rem;
  }
</style>
