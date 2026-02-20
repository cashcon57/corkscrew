<script lang="ts">
  import { getIniSettings, setIniSetting, getIniPresets, applyIniPreset } from "$lib/api";
  import type { IniFile, IniPreset } from "$lib/types";

  interface Props {
    gameId: string;
    bottleName: string;
  }

  let { gameId, bottleName }: Props = $props();
  let iniFiles = $state<IniFile[]>([]);
  let presets = $state<IniPreset[]>([]);
  let loading = $state(false);
  let applying = $state(false);
  let expandedFile = $state<string | null>(null);
  let editingKey = $state<{file: string, section: string, key: string} | null>(null);
  let editValue = $state("");
  let searchQuery = $state("");

  const totalSettings = $derived.by(() => {
    let count = 0;
    for (const file of iniFiles) {
      for (const section of Object.values(file.sections)) {
        count += Object.keys(section).length;
      }
    }
    return count;
  });

  const filteredFiles = $derived.by(() => {
    if (!searchQuery.trim()) return iniFiles;
    const query = searchQuery.toLowerCase();
    return iniFiles
      .map(file => {
        const filteredSections: Record<string, Record<string, string>> = {};
        for (const [sectionName, entries] of Object.entries(file.sections)) {
          const filteredEntries: Record<string, string> = {};
          for (const [key, value] of Object.entries(entries)) {
            if (key.toLowerCase().includes(query) || value.toLowerCase().includes(query)) {
              filteredEntries[key] = value;
            }
          }
          if (Object.keys(filteredEntries).length > 0) {
            filteredSections[sectionName] = filteredEntries;
          }
        }
        if (Object.keys(filteredSections).length > 0) {
          return { ...file, sections: filteredSections };
        }
        return null;
      })
      .filter((f): f is IniFile => f !== null);
  });

  async function load() {
    loading = true;
    try {
      iniFiles = await getIniSettings(gameId, bottleName);
      presets = await getIniPresets(gameId);
    } catch {} finally { loading = false; }
  }

  $effect(() => { if (gameId && bottleName) load(); });

  async function handleApplyPreset(preset: IniPreset) {
    applying = true;
    try {
      const count = await applyIniPreset(gameId, bottleName, preset.name);
      await load();
    } catch {} finally { applying = false; }
  }

  async function handleSaveSetting() {
    if (!editingKey) return;
    const file = iniFiles.find(f => f.file_name === editingKey!.file);
    if (!file) return;
    try {
      await setIniSetting(file.path, editingKey.section, editingKey.key, editValue);
      await load();
    } catch {}
    editingKey = null;
  }

  function startEdit(fileName: string, section: string, key: string, currentValue: string) {
    editingKey = { file: fileName, section, key };
    editValue = currentValue;
  }

  function cancelEdit() {
    editingKey = null;
    editValue = "";
  }

  function toggleFile(fileName: string) {
    expandedFile = expandedFile === fileName ? null : fileName;
  }

  function isEditing(fileName: string, section: string, key: string): boolean {
    return editingKey?.file === fileName && editingKey?.section === section && editingKey?.key === key;
  }

  function handleEditKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      handleSaveSetting();
    } else if (e.key === "Escape") {
      cancelEdit();
    }
  }
</script>

<div class="ini-panel">
  <!-- Header -->
  <div class="panel-header">
    <div class="panel-title-row">
      <svg class="panel-icon" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
        <polyline points="14 2 14 8 20 8" />
        <line x1="16" y1="13" x2="8" y2="13" />
        <line x1="16" y1="17" x2="8" y2="17" />
      </svg>
      <h3 class="panel-title">INI Settings</h3>
      {#if !loading && iniFiles.length > 0}
        <span class="stats-badge">{iniFiles.length} file{iniFiles.length !== 1 ? "s" : ""}</span>
        <span class="stats-badge">{totalSettings} setting{totalSettings !== 1 ? "s" : ""}</span>
      {/if}
    </div>
  </div>

  {#if loading}
    <div class="panel-loading">
      <div class="spinner"></div>
      <p class="loading-text">Loading INI settings...</p>
    </div>
  {:else}
    <!-- Search -->
    {#if totalSettings > 0}
      <div class="search-bar">
        <svg class="search-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="11" cy="11" r="8" />
          <line x1="21" y1="21" x2="16.65" y2="16.65" />
        </svg>
        <input
          type="text"
          class="search-input"
          placeholder="Filter settings by key or value..."
          bind:value={searchQuery}
        />
        {#if searchQuery}
          <button
            class="search-clear"
            onclick={() => searchQuery = ""}
            type="button"
            aria-label="Clear search"
          >
            <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
              <line x1="2" y1="2" x2="8" y2="8" />
              <line x1="8" y1="2" x2="2" y2="8" />
            </svg>
          </button>
        {/if}
      </div>
    {/if}

    <!-- Presets -->
    {#if presets.length > 0}
      <div class="section">
        <h4 class="section-label">Presets</h4>
        <div class="presets-grid">
          {#each presets as preset (preset.name)}
            <div class="preset-card">
              <div class="preset-info">
                <span class="preset-name">{preset.name}</span>
                <span class="preset-description">{preset.description}</span>
                <span class="preset-count">{preset.settings.length} setting{preset.settings.length !== 1 ? "s" : ""}</span>
              </div>
              <button
                class="btn btn-accent btn-sm"
                onclick={() => handleApplyPreset(preset)}
                disabled={applying}
                type="button"
              >
                {#if applying}
                  <span class="spinner-sm"></span>
                  Applying...
                {:else}
                  Apply
                {/if}
              </button>
            </div>
          {/each}
        </div>
      </div>
    {/if}

    <!-- INI Files -->
    {#if filteredFiles.length === 0 && searchQuery}
      <div class="panel-empty">
        <p class="empty-text">No settings match "{searchQuery}"</p>
      </div>
    {:else if filteredFiles.length === 0}
      <div class="panel-empty">
        <div class="empty-icon">
          <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
            <polyline points="14 2 14 8 20 8" />
          </svg>
        </div>
        <p class="empty-title">No INI files found</p>
        <p class="empty-description">INI configuration files were not detected for this game.</p>
      </div>
    {:else}
      <div class="files-list">
        {#each filteredFiles as file (file.file_name)}
          <div class="file-group">
            <!-- File Header (collapsible) -->
            <button
              class="file-header"
              onclick={() => toggleFile(file.file_name)}
              aria-expanded={expandedFile === file.file_name}
              type="button"
            >
              <svg
                class="chevron"
                class:chevron-open={expandedFile === file.file_name}
                width="10"
                height="10"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2.5"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <polyline points="6 9 12 15 18 9" />
              </svg>
              <svg class="file-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
                <polyline points="14 2 14 8 20 8" />
              </svg>
              <span class="file-name">{file.file_name}</span>
              <span class="file-section-count">
                {Object.keys(file.sections).length} section{Object.keys(file.sections).length !== 1 ? "s" : ""}
              </span>
            </button>

            <!-- File Content (expanded) -->
            {#if expandedFile === file.file_name}
              <div class="file-content">
                {#each Object.entries(file.sections) as [sectionName, entries], sIdx}
                  {#if sIdx > 0}
                    <div class="section-divider"></div>
                  {/if}
                  <div class="ini-section">
                    <div class="ini-section-header">
                      <span class="ini-section-name">[{sectionName}]</span>
                      <span class="ini-section-count">{Object.keys(entries).length}</span>
                    </div>
                    <div class="ini-entries">
                      {#each Object.entries(entries) as [key, value]}
                        <div class="ini-entry" class:ini-entry-editing={isEditing(file.file_name, sectionName, key)}>
                          <span class="ini-key">{key}</span>
                          {#if isEditing(file.file_name, sectionName, key)}
                            <div class="edit-controls">
                              <input
                                type="text"
                                class="edit-input"
                                bind:value={editValue}
                                onkeydown={handleEditKeydown}
                              />
                              <button
                                class="btn-icon btn-save"
                                onclick={handleSaveSetting}
                                type="button"
                                aria-label="Save"
                                title="Save (Enter)"
                              >
                                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                                  <path d="M20 6L9 17l-5-5" />
                                </svg>
                              </button>
                              <button
                                class="btn-icon btn-cancel"
                                onclick={cancelEdit}
                                type="button"
                                aria-label="Cancel"
                                title="Cancel (Esc)"
                              >
                                <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
                                  <line x1="2" y1="2" x2="8" y2="8" />
                                  <line x1="8" y1="2" x2="2" y2="8" />
                                </svg>
                              </button>
                            </div>
                          {:else}
                            <span class="ini-equals">=</span>
                            <span class="ini-value" title={value}>{value}</span>
                            <button
                              class="btn-edit"
                              onclick={() => startEdit(file.file_name, sectionName, key, value)}
                              type="button"
                              aria-label="Edit {key}"
                              title="Edit value"
                            >
                              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7" />
                                <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z" />
                              </svg>
                            </button>
                          {/if}
                        </div>
                      {/each}
                    </div>
                  </div>
                {/each}
              </div>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  /* ---- Panel ---- */

  .ini-panel {
    display: flex;
    flex-direction: column;
    background: var(--bg-grouped-secondary);
    border-radius: var(--radius-lg);
    overflow: hidden;
    box-shadow: var(--glass-edge-shadow);
  }

  /* ---- Header ---- */

  .panel-header {
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--separator);
    flex-shrink: 0;
  }

  .panel-title-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .panel-icon {
    color: var(--system-accent);
    flex-shrink: 0;
  }

  .panel-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .stats-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 1px 6px;
    border-radius: 100px;
    font-size: 10px;
    font-weight: 600;
    color: var(--text-tertiary);
    background: var(--surface);
    font-variant-numeric: tabular-nums;
  }

  /* ---- Search ---- */

  .search-bar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    border-bottom: 1px solid var(--separator);
  }

  .search-icon {
    color: var(--text-tertiary);
    flex-shrink: 0;
  }

  .search-input {
    flex: 1;
    min-width: 0;
    padding: var(--space-1) 0;
    background: transparent;
    border: none;
    color: var(--text-primary);
    font-size: 13px;
    font-family: var(--font-sans);
    outline: none;
  }

  .search-input::placeholder {
    color: var(--text-tertiary);
  }

  .search-clear {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    padding: 0;
    background: var(--surface-hover);
    border: none;
    border-radius: 50%;
    color: var(--text-tertiary);
    cursor: pointer;
    flex-shrink: 0;
    transition: all var(--duration-fast) var(--ease);
  }

  .search-clear:hover {
    background: var(--surface-active);
    color: var(--text-primary);
  }

  /* ---- Presets Section ---- */

  .section {
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--separator);
  }

  .section-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-tertiary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    margin-bottom: var(--space-2);
  }

  .presets-grid {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .preset-card {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    box-shadow: var(--glass-edge-shadow);
  }

  .preset-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .preset-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .preset-description {
    font-size: 12px;
    color: var(--text-tertiary);
    line-height: 1.4;
  }

  .preset-count {
    font-size: 11px;
    color: var(--text-quaternary);
    font-variant-numeric: tabular-nums;
  }

  /* ---- Buttons ---- */

  .btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 12px;
    font-weight: 500;
    border: none;
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
    white-space: nowrap;
  }

  .btn-sm {
    padding: var(--space-1) var(--space-3);
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-accent {
    background: var(--system-accent);
    color: white;
  }

  .btn-accent:hover:not(:disabled) {
    filter: brightness(1.1);
  }

  .btn-edit {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    padding: 0;
    background: transparent;
    color: var(--text-quaternary);
    border: none;
    border-radius: var(--radius-sm);
    cursor: pointer;
    flex-shrink: 0;
    opacity: 0;
    transition: all var(--duration-fast) var(--ease);
  }

  .btn-edit:hover {
    background: var(--system-accent-subtle);
    color: var(--system-accent);
  }

  .ini-entry:hover .btn-edit {
    opacity: 1;
  }

  .btn-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    padding: 0;
    background: transparent;
    border: none;
    border-radius: var(--radius-sm);
    cursor: pointer;
    flex-shrink: 0;
    transition: all var(--duration-fast) var(--ease);
  }

  .btn-save {
    color: var(--green);
  }

  .btn-save:hover {
    background: var(--green-subtle, rgba(52, 199, 89, 0.12));
  }

  .btn-cancel {
    color: var(--text-tertiary);
  }

  .btn-cancel:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  /* ---- Loading / Empty ---- */

  .panel-loading {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-8);
  }

  .spinner {
    width: 24px;
    height: 24px;
    border: 2px solid var(--separator-opaque);
    border-top-color: var(--system-accent);
    border-radius: 50%;
    animation: spin 0.75s linear infinite;
  }

  .spinner-sm {
    display: inline-block;
    width: 12px;
    height: 12px;
    border: 2px solid rgba(255, 255, 255, 0.3);
    border-top-color: #fff;
    border-radius: 50%;
    animation: spin 0.75s linear infinite;
    flex-shrink: 0;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .loading-text {
    font-size: 12px;
    color: var(--text-tertiary);
  }

  .panel-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-8);
    text-align: center;
  }

  .empty-icon {
    color: var(--text-quaternary);
    margin-bottom: var(--space-1);
  }

  .empty-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .empty-description {
    font-size: 12px;
    color: var(--text-tertiary);
  }

  .empty-text {
    font-size: 12px;
    color: var(--text-tertiary);
    font-style: italic;
  }

  /* ---- File List ---- */

  .files-list {
    flex: 1;
    overflow-y: auto;
  }

  .file-group {
    border-bottom: 1px solid var(--separator);
  }

  .file-group:last-child {
    border-bottom: none;
  }

  .file-header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: var(--space-3) var(--space-4);
    text-align: left;
    background: transparent;
    border: none;
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease);
  }

  .file-header:hover {
    background: var(--surface-hover);
  }

  .chevron {
    transition: transform var(--duration-fast) var(--ease);
    color: var(--text-tertiary);
    flex-shrink: 0;
  }

  .chevron-open {
    transform: rotate(0deg);
  }

  .chevron:not(.chevron-open) {
    transform: rotate(-90deg);
  }

  .file-icon {
    color: var(--text-tertiary);
    flex-shrink: 0;
  }

  .file-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    font-family: var(--font-mono);
    letter-spacing: 0;
    flex: 1;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .file-section-count {
    flex-shrink: 0;
    font-size: 11px;
    font-weight: 500;
    color: var(--text-tertiary);
    background: var(--surface);
    padding: 1px 6px;
    border-radius: 100px;
  }

  /* ---- File Content (expanded) ---- */

  .file-content {
    padding: 0 var(--space-4) var(--space-3);
    padding-left: calc(var(--space-4) + 10px + var(--space-2));
  }

  .section-divider {
    height: 1px;
    background: var(--separator);
    margin: var(--space-2) 0;
  }

  .ini-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .ini-section-header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) 0;
  }

  .ini-section-name {
    font-size: 12px;
    font-weight: 600;
    color: var(--system-accent);
    font-family: var(--font-mono);
    letter-spacing: 0;
  }

  .ini-section-count {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    border-radius: 100px;
    font-size: 9px;
    font-weight: 700;
    color: var(--text-quaternary);
    background: var(--surface);
    font-variant-numeric: tabular-nums;
  }

  /* ---- INI Entries ---- */

  .ini-entries {
    display: flex;
    flex-direction: column;
  }

  .ini-entry {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
    transition: background var(--duration-fast) var(--ease);
    min-height: 28px;
  }

  .ini-entry:hover {
    background: var(--surface-hover);
  }

  .ini-entry-editing {
    background: var(--surface);
    border: 1px solid var(--system-accent);
    box-shadow: 0 0 0 3px rgba(0, 122, 255, 0.1);
  }

  .ini-entry-editing:hover {
    background: var(--surface);
  }

  .ini-key {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-primary);
    font-family: var(--font-mono);
    letter-spacing: 0;
    white-space: nowrap;
    flex-shrink: 0;
  }

  .ini-equals {
    font-size: 12px;
    color: var(--text-quaternary);
    font-family: var(--font-mono);
    letter-spacing: 0;
    flex-shrink: 0;
  }

  .ini-value {
    font-size: 12px;
    color: var(--text-secondary);
    font-family: var(--font-mono);
    letter-spacing: 0;
    flex: 1;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* ---- Edit Controls ---- */

  .edit-controls {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    flex: 1;
    min-width: 0;
  }

  .edit-input {
    flex: 1;
    min-width: 0;
    padding: var(--space-1) var(--space-2);
    background: var(--bg-base);
    border: 1px solid var(--separator-opaque);
    border-radius: var(--radius-sm);
    color: var(--text-primary);
    font-size: 12px;
    font-family: var(--font-mono);
    letter-spacing: 0;
    outline: none;
    transition: border-color var(--duration-fast) var(--ease);
  }

  .edit-input:focus {
    border-color: var(--system-accent);
    box-shadow: 0 0 0 3px rgba(0, 122, 255, 0.15);
  }
</style>
