<script lang="ts">
  interface Props {
    rows?: number;
    columns?: number;
  }

  let { rows = 5, columns = 4 }: Props = $props();

  const widths = ["40%", "60%", "30%", "50%", "45%", "55%", "35%", "65%"];
</script>

<div class="skeleton-rows">
  {#each Array(rows) as _, r}
    <div class="skeleton-row">
      {#each Array(columns) as _, c}
        <div
          class="skeleton skeleton-cell"
          style="width: {widths[(r + c) % widths.length]}"
        ></div>
      {/each}
    </div>
  {/each}
</div>

<style>
  .skeleton-rows {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
  }

  .skeleton-row {
    display: flex;
    gap: var(--space-3);
    align-items: center;
    padding: var(--space-3) var(--space-3);
  }

  .skeleton-cell {
    height: 14px;
    min-width: 40px;
    flex-shrink: 0;
  }
</style>
