<script lang="ts">
  let { text }: { text: string } = $props();
  let visible = $state(false);
</script>

<span
  class="help-tooltip-trigger"
  onmouseenter={() => visible = true}
  onmouseleave={() => visible = false}
  onfocus={() => visible = true}
  onblur={() => visible = false}
  tabindex="0"
  role="button"
  aria-label="Help"
>
  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <circle cx="12" cy="12" r="10" />
    <path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3" />
    <line x1="12" y1="17" x2="12.01" y2="17" />
  </svg>
  {#if visible}
    <span class="help-tooltip-popup">{text}</span>
  {/if}
</span>

<style>
  .help-tooltip-trigger {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    position: relative;
    color: var(--text-quaternary);
    cursor: help;
    transition: color var(--duration-fast) var(--ease);
    outline: none;
  }

  .help-tooltip-trigger:hover,
  .help-tooltip-trigger:focus-visible {
    color: var(--text-secondary);
  }

  .help-tooltip-popup {
    position: absolute;
    bottom: calc(100% + 8px);
    left: 50%;
    transform: translateX(-50%);
    background: var(--bg-base);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    padding: var(--space-2) var(--space-3);
    font-size: 12px;
    font-weight: 400;
    color: var(--text-secondary);
    line-height: 1.45;
    white-space: normal;
    width: max-content;
    max-width: 260px;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.3);
    z-index: 1000;
    pointer-events: none;
    animation: tooltip-in 0.15s var(--ease);
  }

  @keyframes tooltip-in {
    from {
      opacity: 0;
      transform: translateX(-50%) translateY(4px);
    }
    to {
      opacity: 1;
      transform: translateX(-50%) translateY(0);
    }
  }
</style>
