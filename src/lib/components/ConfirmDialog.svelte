<script lang="ts">
  interface Props {
    open: boolean;
    title: string;
    message: string;
    details?: string[];
    confirmLabel?: string;
    confirmDanger?: boolean;
    onConfirm: () => void;
    onCancel: () => void;
  }

  let {
    open,
    title,
    message,
    details,
    confirmLabel = "Confirm",
    confirmDanger = false,
    onConfirm,
    onCancel,
  }: Props = $props();

  function handleKeydown(e: KeyboardEvent) {
    if (!open) return;
    if (e.key === "Escape") {
      e.preventDefault();
      onCancel();
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

{#if open}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="confirm-overlay" onclick={onCancel} role="dialog" aria-label={title}>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="confirm-card" onclick={(e) => e.stopPropagation()}>
      <h3 class="confirm-title">{title}</h3>
      <p class="confirm-message">{message}</p>

      {#if details && details.length > 0}
        <ul class="confirm-details">
          {#each details as detail}
            <li>{detail}</li>
          {/each}
        </ul>
      {/if}

      <div class="confirm-actions">
        <button class="btn btn-ghost" onclick={onCancel}>Cancel</button>
        <button
          class="btn {confirmDanger ? 'btn-danger' : 'btn-primary'}"
          onclick={onConfirm}
        >
          {confirmLabel}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .confirm-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
    backdrop-filter: var(--glass-blur-light);
    animation: fadeIn 150ms ease-out;
  }

  .confirm-card {
    background: color-mix(in srgb, var(--bg-secondary) 75%, transparent);
    backdrop-filter: blur(40px) saturate(1.5);
    -webkit-backdrop-filter: blur(40px) saturate(1.5);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: var(--radius);
    padding: var(--space-6);
    max-width: 420px;
    width: 90vw;
    animation: slideUp 200ms var(--ease-out);
    box-shadow: var(--glass-refraction),
                var(--glass-edge-shadow),
                var(--shadow-lg);
  }

  .confirm-title {
    margin: 0 0 var(--space-2);
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .confirm-message {
    margin: 0 0 var(--space-4);
    font-size: 13px;
    color: var(--text-secondary);
    line-height: 1.5;
  }

  .confirm-details {
    margin: 0 0 var(--space-4);
    padding-left: var(--space-5);
    font-size: 12px;
    color: var(--text-tertiary);
    line-height: 1.6;
  }

  .confirm-actions {
    display: flex;
    gap: var(--space-2);
    justify-content: flex-end;
  }

  @keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  @keyframes slideUp {
    from { opacity: 0; transform: translateY(8px); }
    to { opacity: 1; transform: translateY(0); }
  }
</style>
