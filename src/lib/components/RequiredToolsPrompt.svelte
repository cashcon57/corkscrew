<script lang="ts">
  import type { RequiredTool } from "$lib/types";
  import { installModTool } from "$lib/api";

  interface Props {
    tools: RequiredTool[];
    gameId: string;
    bottleName: string;
    oncontinue: () => void;
    oncancel: () => void;
  }

  let { tools, gameId, bottleName, oncontinue, oncancel }: Props = $props();

  let installing = $state<Set<string>>(new Set());
  let installed = $state<Set<string>>(new Set());
  let errors = $state<Record<string, string>>({});

  const toolIcons: Record<string, string> = {
    sseedit: "/icons/xedit.png",
    pandora: "/icons/pandora.png",
    bodyslide: "/icons/bodyslide.png",
    cao: "/icons/cao.png",
    nifoptimizer: "/icons/nifoptimizer.png",
    wryebash: "/icons/wryebash.png",
    bethini: "/icons/bethini.png",
    dyndolod: "/icons/dyndolod.png",
    skse: "/icons/skse.png",
  };

  const uninstalledTools = $derived(
    tools.filter((t) => !t.is_detected && !installed.has(t.tool_id))
  );

  const autoInstallable = $derived(
    uninstalledTools.filter((t) => t.can_auto_install && !installing.has(t.tool_id))
  );

  async function handleInstallTool(tool: RequiredTool) {
    const next = new Set(installing);
    next.add(tool.tool_id);
    installing = next;
    delete errors[tool.tool_id];
    errors = { ...errors };

    try {
      await installModTool(tool.tool_id, gameId, bottleName);
      const done = new Set(installed);
      done.add(tool.tool_id);
      installed = done;
    } catch (e: unknown) {
      errors = { ...errors, [tool.tool_id]: String(e) };
    } finally {
      const fin = new Set(installing);
      fin.delete(tool.tool_id);
      installing = fin;
    }
  }

  async function handleInstallAll() {
    for (const tool of autoInstallable) {
      await handleInstallTool(tool);
    }
  }
</script>

<div class="tools-prompt-overlay" role="dialog" aria-label="Required tools">
  <div class="tools-prompt">
    <h3>Required Modding Tools</h3>
    <p class="tools-subtitle">
      This modlist requires the following tools. Install them before proceeding for best results.
    </p>

    <div class="tools-list">
      {#each tools as tool (tool.tool_id)}
        <div class="tool-row" class:detected={tool.is_detected || installed.has(tool.tool_id)}>
          {#if toolIcons[tool.tool_id]}
            <img src={toolIcons[tool.tool_id]} alt="" width="22" height="22" class="tool-icon" />
          {/if}
          <div class="tool-info">
            <span class="tool-name">{tool.tool_name}</span>
            {#if errors[tool.tool_id]}
              <span class="tool-error">{errors[tool.tool_id]}</span>
            {:else if tool.wine_compat === "not_recommended" && tool.recommended_alternative}
              <span class="tool-warning">
                Not recommended for Wine — use {tools.find(t => t.tool_id === tool.recommended_alternative)?.tool_name ?? tool.recommended_alternative} instead
              </span>
            {:else if tool.wine_compat === "limited"}
              <span class="tool-note">Limited Wine compatibility</span>
            {/if}
          </div>

          <div class="tool-status">
            {#if tool.is_detected || installed.has(tool.tool_id)}
              <span class="status-badge installed">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"></polyline></svg>
                Installed
              </span>
            {:else if installing.has(tool.tool_id)}
              <span class="status-badge installing">Installing...</span>
            {:else if errors[tool.tool_id]}
              <button class="status-badge error retry-btn" title={errors[tool.tool_id]} onclick={() => handleInstallTool(tool)}>
                Failed — Retry
              </button>
            {:else if tool.can_auto_install}
              <button class="btn btn-sm btn-accent" onclick={() => handleInstallTool(tool)}>Install</button>
            {:else if tool.download_url}
              <a
                href={tool.download_url}
                target="_blank"
                rel="noopener noreferrer"
                class="btn btn-sm btn-secondary"
              >Download</a>
            {:else}
              <span class="status-badge manual">Manual</span>
            {/if}
          </div>
        </div>
      {/each}
    </div>

    <div class="tools-actions">
      {#if autoInstallable.length > 0}
        <button
          class="btn btn-accent"
          onclick={handleInstallAll}
          disabled={installing.size > 0}
        >
          Install All Available ({autoInstallable.length})
        </button>
      {/if}
      <button class="btn btn-secondary" onclick={oncontinue} disabled={installing.size > 0}>
        {uninstalledTools.length > 0 ? "Continue Anyway" : "Continue"}
      </button>
      <button class="btn btn-ghost" onclick={oncancel} disabled={installing.size > 0}>
        Cancel
      </button>
    </div>
  </div>
</div>

<style>
  .tools-prompt-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
    backdrop-filter: blur(4px);
  }

  .tools-prompt {
    background: var(--bg-secondary, #1e1e2e);
    border: 1px solid var(--border, #333);
    border-radius: 12px;
    padding: 24px;
    max-width: 520px;
    width: 90vw;
    max-height: 80vh;
    overflow-y: auto;
  }

  h3 {
    margin: 0 0 4px;
    font-size: 1.1rem;
    color: var(--text-primary, #fff);
  }

  .tools-subtitle {
    margin: 0 0 16px;
    font-size: 0.85rem;
    color: var(--text-secondary, #aaa);
  }

  .tools-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-bottom: 20px;
  }

  .tool-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 12px;
    border-radius: 8px;
    background: var(--bg-tertiary, #252535);
    border: 1px solid var(--border, #333);
  }

  .tool-icon {
    object-fit: contain;
    border-radius: 4px;
    flex-shrink: 0;
    background: transparent;
  }

  .tool-row.detected {
    opacity: 0.6;
  }

  .tool-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }

  .tool-name {
    font-size: 0.9rem;
    font-weight: 500;
    color: var(--text-primary, #fff);
  }

  .tool-warning {
    font-size: 0.75rem;
    color: var(--amber, #f59e0b);
  }

  .tool-note {
    font-size: 0.75rem;
    color: var(--text-secondary, #aaa);
  }

  .tool-error {
    font-size: 0.72rem;
    color: #ef4444;
    line-height: 1.3;
    max-width: 280px;
    overflow: hidden;
    text-overflow: ellipsis;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
  }

  .tool-status {
    flex-shrink: 0;
  }

  .status-badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 0.8rem;
    padding: 4px 8px;
    border-radius: 4px;
  }

  .status-badge.installed {
    color: #22c55e;
  }

  .status-badge.installing {
    color: var(--accent, #6366f1);
  }

  .status-badge.error {
    color: #ef4444;
  }

  .retry-btn {
    background: transparent;
    border: 1px solid #ef4444;
    cursor: pointer;
    border-radius: 4px;
    transition: background 0.15s;
  }

  .retry-btn:hover {
    background: rgba(239, 68, 68, 0.1);
  }

  .status-badge.manual {
    color: var(--text-secondary, #aaa);
  }

  .tools-actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
    flex-wrap: wrap;
  }

  .btn {
    padding: 8px 16px;
    border: none;
    border-radius: 6px;
    font-size: 0.85rem;
    cursor: pointer;
    transition: opacity 0.15s;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-accent {
    background: var(--accent, #6366f1);
    color: #fff;
  }

  .btn-secondary {
    background: var(--bg-tertiary, #252535);
    color: var(--text-primary, #fff);
    border: 1px solid var(--border, #333);
    text-decoration: none;
  }

  .btn-ghost {
    background: transparent;
    color: var(--text-secondary, #aaa);
  }

  .btn-sm {
    padding: 5px 12px;
    font-size: 0.8rem;
  }
</style>
