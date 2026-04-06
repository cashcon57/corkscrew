<script lang="ts">
  import type { CollectionManifest } from "$lib/types";

  type OptionalModChoice = "install" | "install_disabled" | "skip";

  interface Props {
    manifest: CollectionManifest & Record<string, unknown>;
    onconfirm: (choices: Map<number, OptionalModChoice>) => void;
    oncancel: () => void;
  }

  let { manifest, onconfirm, oncancel }: Props = $props();

  let choices = $state<Map<number, OptionalModChoice>>(new Map());

  // Initialize choices on mount
  $effect(() => {
    const c = new Map<number, OptionalModChoice>();
    manifest.mods.forEach((m: { optional: boolean }, i: number) => {
      if (m.optional) c.set(i, "install_disabled");
    });
    choices = c;
  });

  const requiredCount = $derived(
    manifest.mods.filter((m: { optional: boolean }) => !m.optional).length
  );
  const optionalCount = $derived(
    manifest.mods.filter((m: { optional: boolean }) => m.optional).length
  );
  const installCount = $derived(
    manifest.mods.length - Array.from(choices.values()).filter(v => v === "skip").length
  );

  function setAll(value: OptionalModChoice) {
    const c = new Map(choices);
    manifest.mods.forEach((m: { optional: boolean }, i: number) => {
      if (m.optional) c.set(i, value);
    });
    choices = c;
  }
</script>

<div class="modal-overlay" onclick={(e) => { if (e.target === e.currentTarget) oncancel(); }} role="presentation">
  <div class="optional-picker-modal" onclick={(e) => e.stopPropagation()} role="dialog" aria-label="Configure optional mods">
    <div class="optional-picker-header">
      <h3>Configure Installation</h3>
      <p class="optional-picker-subtitle">
        {requiredCount} required
        &middot; {optionalCount} optional mods
      </p>
    </div>

    <div class="optional-picker-body">
      <!-- Required mods section (collapsed summary) -->
      <div class="optional-section">
        <div class="optional-section-header">
          <span class="optional-section-label">
            <svg class="optional-check" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--green, #22c55e)" stroke-width="2.5" stroke-linecap="round"><path d="M20 6L9 17l-5-5" /></svg>
            {requiredCount} required mods will be installed
          </span>
        </div>
      </div>

      <!-- Optional mods section -->
      <div class="optional-section">
        <div class="optional-section-header">
          <span class="optional-section-label">Optional</span>
          <div class="optional-section-actions">
            <button class="btn btn-ghost btn-xs" onclick={() => setAll("install")}>All</button>
            <button class="btn btn-ghost btn-xs" onclick={() => setAll("install_disabled")}>All (Disabled)</button>
            <button class="btn btn-ghost btn-xs" onclick={() => setAll("skip")}>None</button>
          </div>
        </div>
        {#each manifest.mods as mod, i}
          {#if mod.optional}
            <div class="optional-mod-row">
              <span class="optional-mod-name">{mod.name}</span>
              <span class="optional-mod-version">{mod.version || ""}</span>
              <select
                class="optional-mod-select"
                value={choices.get(i) ?? "install_disabled"}
                onchange={(e) => {
                  const c = new Map(choices);
                  c.set(i, (e.currentTarget as HTMLSelectElement).value as OptionalModChoice);
                  choices = c;
                }}
              >
                <option value="install">Install</option>
                <option value="install_disabled">Install (Disabled)</option>
                <option value="skip">Skip</option>
              </select>
            </div>
          {/if}
        {/each}
      </div>
    </div>

    <div class="optional-picker-footer">
      <button class="btn btn-ghost" onclick={oncancel}>Cancel</button>
      <button class="btn btn-accent" onclick={() => onconfirm(choices)}>
        Install ({installCount} mods)
      </button>
    </div>
  </div>
</div>

<style>
  .modal-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
  }

  .optional-picker-modal {
    background: color-mix(in srgb, var(--bg-grouped) 75%, transparent);
    backdrop-filter: var(--glass-blur-heavy);
    -webkit-backdrop-filter: var(--glass-blur-heavy);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: var(--radius-lg, 12px);
    width: min(600px, 90vw);
    max-height: 80vh;
    display: flex;
    flex-direction: column;
    box-shadow: var(--glass-refraction, none), var(--glass-edge-shadow, none), 0 8px 32px rgba(0, 0, 0, 0.4);
  }

  .optional-picker-header {
    padding: var(--space-4) var(--space-5);
    border-bottom: 1px solid var(--separator);
    flex-shrink: 0;
  }

  .optional-picker-header h3 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .optional-picker-subtitle {
    margin: 4px 0 0;
    font-size: 12px;
    color: var(--text-tertiary);
  }

  .optional-picker-body {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-3) var(--space-5);
  }

  .optional-section {
    margin-bottom: var(--space-4);
  }

  .optional-section-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--space-2);
  }

  .optional-section-label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-tertiary);
  }

  .optional-section-actions {
    display: flex;
    gap: 4px;
  }

  .optional-mod-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 6px 0;
    border-bottom: 1px solid var(--separator);
    font-size: 13px;
  }

  .optional-mod-name {
    flex: 1;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .optional-mod-version {
    color: var(--text-tertiary);
    font-size: 11px;
    flex-shrink: 0;
  }

  .optional-check {
    flex-shrink: 0;
  }

  .optional-mod-select {
    flex-shrink: 0;
    background: var(--bg-tertiary);
    border: 1px solid var(--separator);
    border-radius: var(--radius-sm);
    color: var(--text-primary);
    font-size: 11px;
    padding: 3px 8px;
    cursor: pointer;
    font-family: var(--font-sans);
  }

  .optional-picker-footer {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-5);
    border-top: 1px solid var(--separator);
    flex-shrink: 0;
  }
</style>
