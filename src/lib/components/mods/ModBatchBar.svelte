<script lang="ts">
  interface Props {
    selectedCount: number;
    onEnableAll: () => void;
    onDisableAll: () => void;
    onUninstallSelected: () => void;
    onClearSelection: () => void;
  }

  let { selectedCount, onEnableAll, onDisableAll, onUninstallSelected, onClearSelection }: Props = $props();
</script>

<!-- Phase 3 will implement this floating batch action toolbar -->
{#if selectedCount > 0}
  <div class="batch-bar">
    <span class="batch-count">{selectedCount} mod{selectedCount === 1 ? "" : "s"} selected</span>
    <div class="batch-actions">
      <button class="btn btn-sm btn-secondary" onclick={onEnableAll}>Enable All</button>
      <button class="btn btn-sm btn-secondary" onclick={onDisableAll}>Disable All</button>
      <button class="btn btn-sm btn-ghost-danger" onclick={onUninstallSelected}>Uninstall</button>
      <button class="btn btn-sm btn-ghost" onclick={onClearSelection}>
        <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
          <line x1="3" y1="3" x2="9" y2="9" /><line x1="9" y1="3" x2="3" y2="9" />
        </svg>
      </button>
    </div>
  </div>
{/if}

<style>
  .batch-bar {
    position: fixed;
    bottom: 24px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-4);
    background: color-mix(in srgb, var(--bg-primary) 70%, transparent);
    backdrop-filter: blur(32px) saturate(1.4);
    -webkit-backdrop-filter: blur(32px) saturate(1.4);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: var(--radius-lg);
    box-shadow: var(--glass-refraction),
                var(--glass-edge-shadow),
                0 8px 32px rgba(0, 0, 0, 0.4);
    z-index: 100;
    animation: batchSlideUp 0.15s var(--ease-out);
  }

  @keyframes batchSlideUp {
    from { opacity: 0; transform: translateX(-50%) translateY(8px); }
    to { opacity: 1; transform: translateX(-50%) translateY(0); }
  }

  .batch-count {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    white-space: nowrap;
  }

  .batch-actions {
    display: flex;
    gap: var(--space-2);
    align-items: center;
  }
</style>
