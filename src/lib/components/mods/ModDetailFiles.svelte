<script lang="ts">
  interface Props {
    files: string[];
  }

  let { files }: Props = $props();

  // Virtual scrolling state
  let fileBrowserOpen = $state(false);
  let fileSearchQuery = $state("");
  let fileBrowserEl = $state<HTMLElement | null>(null);
  let fileBrowserStart = $state(0);
  const fileBrowserVisible = 50; // render ~50 rows at a time (24px each = 1200px)

  function handleFileBrowserScroll() {
    if (fileBrowserEl) {
      fileBrowserStart = Math.max(0, Math.floor(fileBrowserEl.scrollTop / 24) - 5);
    }
  }
</script>

{#if files.length > 0}
  <div class="section">
    <button class="section-title section-title-toggle" onclick={() => fileBrowserOpen = !fileBrowserOpen} type="button">
      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" /></svg>
      Files ({files.length})
      <svg class="chevron-toggle" class:chevron-open={fileBrowserOpen} width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="6 9 12 15 18 9" /></svg>
    </button>
    {#if fileBrowserOpen}
      {#if fileSearchQuery}
        {@const filtered = files.filter(f => f.toLowerCase().includes(fileSearchQuery.toLowerCase()))}
        <input class="file-search" type="text" placeholder="Filter files..." bind:value={fileSearchQuery} />
        <div class="file-browser" bind:this={fileBrowserEl} onscroll={handleFileBrowserScroll}>
          <div style="height: {filtered.length * 24}px; position: relative;">
            {#each filtered.slice(fileBrowserStart, fileBrowserStart + fileBrowserVisible) as file, i}
              <div class="file-entry" style="position: absolute; top: {(fileBrowserStart + i) * 24}px; height: 24px;">
                <span class="file-path" title={file}>{file}</span>
              </div>
            {/each}
          </div>
        </div>
      {:else}
        <input class="file-search" type="text" placeholder="Filter files..." bind:value={fileSearchQuery} />
        <div class="file-browser" bind:this={fileBrowserEl} onscroll={handleFileBrowserScroll}>
          <div style="height: {files.length * 24}px; position: relative;">
            {#each files.slice(fileBrowserStart, fileBrowserStart + fileBrowserVisible) as file, i}
              <div class="file-entry" style="position: absolute; top: {(fileBrowserStart + i) * 24}px; height: 24px;">
                <span class="file-path" title={file}>{file}</span>
              </div>
            {/each}
          </div>
        </div>
      {/if}
    {/if}
  </div>
{/if}

<style>
  .section {
    padding-bottom: var(--space-3);
    border-bottom: 1px solid var(--separator);
  }
  .section:last-child {
    border-bottom: none;
    padding-bottom: 0;
  }

  .section-title {
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-tertiary);
    margin: 0 0 var(--space-2) 0;
    display: flex;
    align-items: center;
    gap: 5px;
  }

  .section-title-toggle {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    cursor: pointer;
    width: 100%;
    text-align: left;
    padding: 0;
    background: none;
    border: none;
    font: inherit;
    color: inherit;
  }

  .section-title-toggle:hover {
    color: var(--text-primary);
  }

  .chevron-toggle {
    margin-left: auto;
    transition: transform var(--duration-fast) var(--ease);
    transform: rotate(-90deg);
  }

  .chevron-open {
    transform: rotate(0deg);
  }

  .file-search {
    width: 100%;
    padding: var(--space-1) var(--space-2);
    margin-bottom: var(--space-1);
    background: var(--bg-base);
    border: 1px solid var(--separator);
    border-radius: var(--radius-sm);
    font-size: 11px;
    color: var(--text-primary);
  }

  .file-search:focus {
    outline: none;
    border-color: var(--accent);
  }

  .file-browser {
    max-height: 300px;
    overflow-y: auto;
    border: 1px solid var(--separator);
    border-radius: var(--radius-sm);
    background: var(--bg-base);
  }

  .file-entry {
    display: flex;
    align-items: center;
    padding: 0 var(--space-2);
    width: 100%;
    box-sizing: border-box;
  }

  .file-path {
    font-size: 11px;
    font-family: var(--font-mono, "SF Mono", "Menlo", monospace);
    color: var(--text-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
