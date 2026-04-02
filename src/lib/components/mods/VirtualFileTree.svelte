<script lang="ts">
  import { getMergedFileTree } from "$lib/api";
  import { selectedGame, showError } from "$lib/stores";

  interface FileTreeNode {
    name: string;
    path: string;
    is_dir: boolean;
    children: FileTreeNode[];
    source_mod_id: number | null;
    source_mod_name: string | null;
    conflict_mod_names: string[];
    file_size: number | null;
  }

  let tree = $state<FileTreeNode[]>([]);
  let loading = $state(false);
  let expandedDirs = $state(new Set<string>());
  let searchQuery = $state("");

  export async function loadTree() {
    const game = $selectedGame;
    if (!game) return;
    loading = true;
    try {
      tree = await getMergedFileTree(game.game_id, game.bottle_name);
    } catch (e) {
      showError(`Failed to load file tree: ${e}`);
      tree = [];
    } finally {
      loading = false;
    }
  }

  function toggleDir(path: string) {
    const next = new Set(expandedDirs);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    expandedDirs = next;
  }

  // Flatten tree for rendering with indentation
  function flattenTree(nodes: FileTreeNode[], depth: number = 0): { node: FileTreeNode; depth: number }[] {
    const result: { node: FileTreeNode; depth: number }[] = [];
    for (const node of nodes) {
      // Search filter
      if (searchQuery && !node.name.toLowerCase().includes(searchQuery.toLowerCase()) && !node.is_dir) continue;

      result.push({ node, depth });
      if (node.is_dir && expandedDirs.has(node.path)) {
        result.push(...flattenTree(node.children, depth + 1));
      }
    }
    return result;
  }

  let flatList = $derived(flattenTree(tree));

  let totalFiles = $derived(tree.reduce((sum, n) => sum + (n.is_dir ? n.children.length : 1), 0));
  let totalConflicts = $derived(flatList.filter(({ node }) => node.conflict_mod_names.length > 0).length);
</script>

<div class="file-tree-panel">
  <div class="file-tree-header">
    <h3 class="file-tree-title">
      Merged File View
      {#if !loading}
        <span class="file-tree-count">{totalFiles} files</span>
        {#if totalConflicts > 0}
          <span class="file-tree-conflicts">{totalConflicts} conflicts</span>
        {/if}
      {/if}
    </h3>
    <input
      type="text"
      class="file-tree-search"
      placeholder="Filter files..."
      bind:value={searchQuery}
    />
  </div>

  {#if loading}
    <div class="file-tree-loading">
      <div class="spinner"><div class="spinner-ring"></div></div>
      <p>Building file tree...</p>
    </div>
  {:else if tree.length === 0}
    <div class="file-tree-empty">
      <p>No deployed files. Deploy mods first to see the merged file tree.</p>
    </div>
  {:else}
    <div class="file-tree-list">
      {#each flatList as { node, depth } (node.path)}
        <div
          class="file-tree-row"
          class:is-dir={node.is_dir}
          class:has-conflict={node.conflict_mod_names.length > 0}
          style="padding-left: {12 + depth * 16}px;"
        >
          {#if node.is_dir}
            <button class="dir-toggle" onclick={() => toggleDir(node.path)}>
              <svg
                class="dir-chevron"
                class:expanded={expandedDirs.has(node.path)}
                width="10" height="10" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="2"
              >
                <path d="M4 2l4 4-4 4" />
              </svg>
              <span class="dir-icon">📁</span>
              <span class="file-name">{node.name}</span>
              <span class="child-count">{node.children.length}</span>
            </button>
          {:else}
            <span class="file-icon">
              {#if node.conflict_mod_names.length > 0}⚠{:else}📄{/if}
            </span>
            <span class="file-name" title={node.path}>{node.name}</span>
            {#if node.source_mod_name}
              <span class="file-source" title="Provided by {node.source_mod_name}">
                {node.source_mod_name}
              </span>
            {/if}
            {#if node.conflict_mod_names.length > 0}
              <span class="file-conflicts" title="Also in: {node.conflict_mod_names.join(', ')}">
                +{node.conflict_mod_names.length} conflict{node.conflict_mod_names.length !== 1 ? 's' : ''}
              </span>
            {/if}
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .file-tree-panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--surface);
    border-radius: 8px;
    border: 1px solid var(--border);
    overflow: hidden;
  }

  .file-tree-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    gap: 8px;
  }

  .file-tree-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0;
    white-space: nowrap;
  }

  .file-tree-count {
    font-size: 11px;
    font-weight: 400;
    color: var(--text-secondary);
    background: var(--surface-subtle);
    padding: 1px 6px;
    border-radius: 4px;
  }

  .file-tree-conflicts {
    font-size: 11px;
    font-weight: 500;
    color: var(--amber);
    background: color-mix(in srgb, var(--amber) 10%, transparent);
    padding: 1px 6px;
    border-radius: 4px;
  }

  .file-tree-search {
    flex: 0 0 180px;
    height: 26px;
    padding: 0 8px;
    font-size: 12px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--surface-subtle);
    color: var(--text-primary);
    outline: none;
  }
  .file-tree-search:focus {
    border-color: var(--accent);
  }

  .file-tree-loading, .file-tree-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 24px;
    color: var(--text-secondary);
    font-size: 13px;
    gap: 8px;
  }

  .file-tree-list {
    overflow-y: auto;
    flex: 1;
  }

  .file-tree-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 12px;
    font-size: 12px;
    color: var(--text-primary);
    cursor: default;
    min-height: 24px;
  }

  .file-tree-row:hover {
    background: var(--surface-hover);
  }

  .file-tree-row.has-conflict {
    background: color-mix(in srgb, var(--amber) 5%, transparent);
  }
  .file-tree-row.has-conflict:hover {
    background: color-mix(in srgb, var(--amber) 10%, transparent);
  }

  .dir-toggle {
    display: flex;
    align-items: center;
    gap: 4px;
    background: none;
    border: none;
    color: inherit;
    cursor: pointer;
    padding: 0;
    font-size: inherit;
  }

  .dir-chevron {
    transition: transform var(--duration-fast) var(--ease);
    flex-shrink: 0;
  }
  .dir-chevron.expanded {
    transform: rotate(90deg);
  }

  .dir-icon, .file-icon {
    font-size: 12px;
    flex-shrink: 0;
    width: 16px;
    text-align: center;
  }

  .file-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  .child-count {
    font-size: 10px;
    color: var(--text-tertiary);
    margin-left: 4px;
  }

  .file-source {
    font-size: 11px;
    color: var(--text-secondary);
    margin-left: auto;
    padding: 0 6px;
    background: var(--surface-subtle);
    border-radius: 3px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 150px;
  }

  .file-conflicts {
    font-size: 10px;
    color: var(--amber);
    background: color-mix(in srgb, var(--amber) 10%, transparent);
    padding: 0 5px;
    border-radius: 3px;
    white-space: nowrap;
    flex-shrink: 0;
  }
</style>
