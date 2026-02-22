<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    title: string;
    description?: string;
    icon?: Snippet;
    children?: Snippet;
  }

  let { title, description, icon, children }: Props = $props();
</script>

<div class="empty-state-container">
  {#if icon}
    <div class="empty-icon">
      {@render icon()}
    </div>
  {/if}

  <p class="empty-title">{title}</p>

  {#if description}
    <p class="empty-description">{description}</p>
  {/if}

  {#if children}
    <div class="empty-actions">
      {@render children()}
    </div>
  {/if}
</div>

<style>
  .empty-state-container {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    padding: var(--space-12) var(--space-6);
    max-width: 360px;
    margin: 0 auto;
    text-align: center;
  }

  .empty-icon {
    color: var(--text-quaternary);
    width: 48px;
    height: 48px;
  }

  .empty-icon :global(svg) {
    width: 100%;
    height: 100%;
  }

  .empty-title {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .empty-description {
    margin: 0;
    font-size: 13px;
    color: var(--text-tertiary);
    line-height: 1.5;
  }

  .empty-actions {
    display: flex;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }
</style>
