<script lang="ts">
  import type { NexusModInfo, NexusModFile } from "$lib/types";
  import { showError } from "$lib/stores";
  import { getModFiles } from "$lib/api";

  interface Props {
    mod: NexusModInfo;
    gameSlug: string;
    downloadingFileId: number | null;
    downloadProgress: { downloaded: number; total: number } | null;
    ondownload: (file: NexusModFile) => void;
    onclose: () => void;
  }

  let { mod, gameSlug, downloadingFileId, downloadProgress, ondownload, onclose }: Props = $props();

  let files = $state<NexusModFile[]>([]);
  let loading = $state(false);

  function formatFileSize(kb: number): string {
    if (kb >= 1_048_576) return `${(kb / 1_048_576).toFixed(1)} GB`;
    if (kb >= 1024) return `${(kb / 1024).toFixed(1)} MB`;
    return `${kb} KB`;
  }

  $effect(() => {
    const m = mod;
    const slug = gameSlug;
    if (m && slug) {
      loadFiles(slug, m.mod_id);
    }
  });

  async function loadFiles(slug: string, modId: number) {
    loading = true;
    try {
      const result = await getModFiles(slug, modId);
      const categoryOrder: Record<string, number> = { main: 0, update: 1, optional: 2, miscellaneous: 3, old_version: 4 };
      files = result
        .filter(f => f.category !== "deleted" && f.category !== "archived")
        .sort((a, b) => (categoryOrder[a.category] ?? 5) - (categoryOrder[b.category] ?? 5));
    } catch (e) {
      showError(`Failed to load mod files: ${e}`);
      onclose();
    } finally {
      loading = false;
    }
  }
</script>

<div class="modal-overlay" onclick={onclose} role="presentation">
  <div class="file-picker-modal" onclick={(e) => e.stopPropagation()} role="dialog" aria-label="Select file to download">
    <div class="file-picker-header">
      <h3 class="file-picker-title">Download: {mod.name}</h3>
      <button class="file-picker-close" onclick={onclose}>&times;</button>
    </div>

    {#if loading}
      <div class="file-picker-loading">
        <div class="spinner-sm"></div>
        <span>Loading available files...</span>
      </div>
    {:else if files.length === 0}
      <div class="file-picker-empty">
        <p>No downloadable files found for this mod.</p>
      </div>
    {:else}
      <div class="file-picker-list">
        {#each files as file}
          <div class="file-picker-item" class:file-downloading={downloadingFileId === file.file_id}>
            <div class="file-picker-info">
              <div class="file-picker-name">{file.name}</div>
              <div class="file-picker-meta">
                <span class="file-category-badge" class:file-cat-main={file.category === "main"} class:file-cat-optional={file.category === "optional"} class:file-cat-update={file.category === "update"}>
                  {file.category}
                </span>
                {#if file.version}<span class="file-version">v{file.version}</span>{/if}
                <span class="file-size">{formatFileSize(file.size_kb)}</span>
              </div>
              {#if file.description}
                <p class="file-picker-desc">{file.description}</p>
              {/if}
            </div>
            <div class="file-picker-action">
              {#if downloadingFileId === file.file_id}
                <div class="download-progress-bar">
                  <div class="download-progress-fill" style="width: {downloadProgress && downloadProgress.total > 0 ? Math.round((downloadProgress.downloaded / downloadProgress.total) * 100) : 0}%"></div>
                </div>
                <span class="download-progress-text">
                  {#if downloadProgress && downloadProgress.total > 0}
                    {Math.round((downloadProgress.downloaded / downloadProgress.total) * 100)}%
                  {:else}
                    Starting...
                  {/if}
                </span>
              {:else}
                <button
                  class="btn btn-accent btn-sm"
                  disabled={downloadingFileId !== null}
                  onclick={() => ondownload(file)}
                >
                  Install
                </button>
              {/if}
            </div>
          </div>
        {/each}
      </div>
    {/if}
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
    backdrop-filter: var(--glass-blur-light);
  }

  .file-picker-modal {
    background: color-mix(in srgb, var(--bg-grouped) 75%, transparent);
    backdrop-filter: blur(40px) saturate(1.5);
    -webkit-backdrop-filter: blur(40px) saturate(1.5);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: var(--radius-lg, 12px);
    width: min(560px, 90vw);
    max-height: 70vh;
    display: flex;
    flex-direction: column;
    box-shadow: var(--glass-refraction),
                var(--glass-edge-shadow),
                0 8px 32px rgba(0, 0, 0, 0.4);
  }

  .file-picker-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-4) var(--space-6);
    border-bottom: 1px solid var(--separator);
    flex-shrink: 0;
  }

  .file-picker-title {
    font-size: 16px;
    font-weight: 600;
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 420px;
  }

  .file-picker-close {
    width: 28px;
    height: 28px;
    border-radius: var(--radius-sm);
    background: transparent;
    border: none;
    color: var(--text-tertiary);
    font-size: 18px;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .file-picker-close:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .file-picker-loading {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    padding: var(--space-10);
    color: var(--text-secondary);
    font-size: 13px;
  }

  .file-picker-empty {
    padding: var(--space-10);
    text-align: center;
    color: var(--text-tertiary);
    font-size: 13px;
  }

  .file-picker-list {
    overflow-y: auto;
    padding: var(--space-2);
  }

  .file-picker-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius);
    transition: background var(--duration-fast) var(--ease);
  }

  .file-picker-item:hover {
    background: var(--surface-hover);
  }

  .file-picker-item.file-downloading {
    background: rgba(0, 122, 255, 0.05);
  }

  .file-picker-info {
    flex: 1;
    min-width: 0;
  }

  .file-picker-name {
    font-size: 14px;
    font-weight: 500;
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .file-picker-meta {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-1);
    font-size: 12px;
    color: var(--text-tertiary);
  }

  .file-category-badge {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    padding: 1px 5px;
    border-radius: 100px;
    background: var(--surface-hover);
    color: var(--text-secondary);
  }

  .file-cat-main {
    background: rgba(48, 209, 88, 0.15);
    color: #30d158;
  }

  .file-cat-optional {
    background: rgba(0, 122, 255, 0.15);
    color: var(--system-accent);
  }

  .file-cat-update {
    background: rgba(255, 159, 10, 0.15);
    color: #ff9f0a;
  }

  .file-version {
    color: var(--text-tertiary);
  }

  .file-size {
    color: var(--text-tertiary);
  }

  .file-picker-desc {
    font-size: 12px;
    color: var(--text-tertiary);
    margin-top: var(--space-1);
    line-height: 1.4;
    overflow: hidden;
    text-overflow: ellipsis;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
  }

  .file-picker-action {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-1);
    min-width: 80px;
  }

  .download-progress-bar {
    width: 80px;
    height: 4px;
    background: var(--separator);
    border-radius: 2px;
    overflow: hidden;
  }

  .download-progress-fill {
    height: 100%;
    background: var(--system-accent);
    border-radius: 2px;
    transition: width 0.3s ease;
  }

  .download-progress-text {
    font-size: 11px;
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
  }
</style>
