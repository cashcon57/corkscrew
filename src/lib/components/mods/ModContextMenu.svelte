<script lang="ts">
  import type { InstalledMod } from "$lib/types";

  interface Props {
    mod: InstalledMod;
    x: number;
    y: number;
    onClose: () => void;
    onToggle: () => void;
    onReinstall: () => void;
    onCheckUpdate: () => void;
    onOpenNexus: () => void;
    onUninstall: () => void;
    onEditTags: () => void;
    onEditNotes: () => void;
  }

  let {
    mod, x, y, onClose, onToggle, onReinstall,
    onCheckUpdate, onOpenNexus, onUninstall,
    onEditTags, onEditNotes,
  }: Props = $props();

  // Close on click outside
  function handleWindowClick() {
    onClose();
  }

  // Close on escape
  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") onClose();
  }
</script>

<svelte:window onclick={handleWindowClick} onkeydown={handleKeydown} />

<!-- Phase 3 will implement full context menu -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="context-menu"
  style="left: {x}px; top: {y}px;"
  onclick={(e) => e.stopPropagation()}
>
  <button class="context-item" onclick={() => { onToggle(); onClose(); }}>
    {mod.enabled ? "Disable" : "Enable"}
  </button>
  <button class="context-item" onclick={() => { onEditTags(); onClose(); }}>
    Edit Tags
  </button>
  <button class="context-item" onclick={() => { onEditNotes(); onClose(); }}>
    Edit Notes
  </button>
  <div class="context-separator"></div>
  <button class="context-item" onclick={() => { onReinstall(); onClose(); }}>
    Reinstall
  </button>
  {#if mod.nexus_mod_id}
    <button class="context-item" onclick={() => { onCheckUpdate(); onClose(); }}>
      Check for Update
    </button>
    <button class="context-item" onclick={() => { onOpenNexus(); onClose(); }}>
      Open on Nexus
    </button>
  {/if}
  <div class="context-separator"></div>
  <button class="context-item context-danger" onclick={() => { onUninstall(); onClose(); }}>
    Uninstall
  </button>
</div>

<style>
  .context-menu {
    position: fixed;
    z-index: 200;
    min-width: 180px;
    background: color-mix(in srgb, var(--bg-primary) 72%, transparent);
    backdrop-filter: blur(32px) saturate(1.4);
    -webkit-backdrop-filter: blur(32px) saturate(1.4);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: var(--radius-md);
    box-shadow: var(--glass-refraction),
                var(--glass-edge-shadow),
                0 8px 32px rgba(0, 0, 0, 0.4);
    padding: var(--space-1) 0;
    animation: contextFadeIn 0.1s var(--ease-out);
  }

  @keyframes contextFadeIn {
    from { opacity: 0; transform: scale(0.96); }
    to { opacity: 1; transform: scale(1); }
  }

  .context-item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: var(--space-2) var(--space-3);
    font-size: 13px;
    color: var(--text-primary);
    cursor: pointer;
    text-align: left;
    transition: background var(--duration-fast) var(--ease);
  }

  .context-item:hover {
    background: var(--surface-hover);
  }

  .context-danger {
    color: var(--red);
  }

  .context-separator {
    height: 1px;
    background: var(--separator);
    margin: var(--space-1) 0;
  }
</style>
