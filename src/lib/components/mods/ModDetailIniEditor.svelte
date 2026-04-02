<script lang="ts">
  import { tick } from "svelte";
  import { readModFile, writeModFile } from "$lib/api";
  import { installedMods, showError, showSuccess } from "$lib/stores";
  import type { InstalledMod } from "$lib/types";

  interface Props {
    mod: InstalledMod;
    scrollToIni: string | null;
    onnavigatemod: (mod: InstalledMod, iniFile?: string) => void;
  }

  let { mod, scrollToIni, onnavigatemod }: Props = $props();

  function fileName(path: string): string {
    return path.split('/').pop() ?? path;
  }

  let iniFiles = $derived(
    mod.installed_files
      .filter(f => f.toLowerCase().endsWith('.ini'))
      .sort((a, b) => fileName(a).localeCompare(fileName(b), undefined, { sensitivity: 'base' }))
  );

  let editingIniFile = $state<string | null>(null);
  let iniContent = $state("");
  let iniOriginalContent = $state("");
  let iniLoadedOriginal = $state("");
  let iniLoading = $state(false);
  let iniSaving = $state(false);

  // INI conflict detection
  interface IniConflictInfo {
    overwrittenBy: InstalledMod[];
    overwrites: InstalledMod[];
  }

  let iniConflicts = $derived.by(() => {
    const conflicts = new Map<string, IniConflictInfo>();
    const myIniLower = new Set(iniFiles.map(f => f.toLowerCase()));
    if (myIniLower.size === 0) return conflicts;
    for (const other of $installedMods) {
      if (other.id === mod.id || !other.enabled) continue;
      for (const file of other.installed_files) {
        const key = file.toLowerCase();
        if (myIniLower.has(key)) {
          const info = conflicts.get(key) ?? { overwrittenBy: [], overwrites: [] };
          if (other.install_priority > mod.install_priority) {
            info.overwrittenBy.push(other);
          } else {
            info.overwrites.push(other);
          }
          conflicts.set(key, info);
        }
      }
    }
    return conflicts;
  });

  let iniCanRevert = $derived(iniLoadedOriginal !== "" && iniContent !== iniLoadedOriginal);
  let iniDirty = $derived(iniContent !== iniOriginalContent);

  async function openIniFile(relativePath: string) {
    if (!mod.staging_path) {
      showError("No staging path for this mod");
      return;
    }
    iniLoading = true;
    editingIniFile = relativePath;
    try {
      const content = await readModFile(mod.staging_path, relativePath);
      iniContent = content;
      iniOriginalContent = content;
      iniLoadedOriginal = content;
    } catch (e) {
      showError(`Failed to read ${fileName(relativePath)}: ${e}`);
      editingIniFile = null;
    } finally {
      iniLoading = false;
    }
  }

  async function saveIniFile() {
    if (!mod.staging_path || !editingIniFile) return;
    iniSaving = true;
    try {
      await writeModFile(mod.staging_path, editingIniFile, iniContent);
      iniOriginalContent = iniContent;
      showSuccess(`Saved ${fileName(editingIniFile)}`);
    } catch (e) {
      showError(`Failed to save: ${e}`);
    } finally {
      iniSaving = false;
    }
  }

  function closeIniEditor() {
    editingIniFile = null;
    iniContent = "";
    iniOriginalContent = "";
    iniLoadedOriginal = "";
  }

  function revertIniFile() {
    if (iniLoadedOriginal) {
      iniContent = iniLoadedOriginal;
    }
  }

  // Reset INI editor when mod changes
  $effect(() => {
    mod.id;
    editingIniFile = null;
    iniContent = "";
    iniOriginalContent = "";
    iniLoadedOriginal = "";
  });

  // Scroll to a specific INI file when navigating from a conflict link
  let iniFileRefs = new Map<string, HTMLElement>();
  let highlightedIni = $state<string | null>(null);

  function registerIniRef(node: HTMLElement, iniFile: string) {
    iniFileRefs.set(iniFile.toLowerCase(), node);
    return {
      destroy() { iniFileRefs.delete(iniFile.toLowerCase()); }
    };
  }

  $effect(() => {
    const target = scrollToIni;
    if (!target) return;
    tick().then(() => {
      const el = iniFileRefs.get(target.toLowerCase());
      if (el) {
        el.scrollIntoView({ behavior: "smooth", block: "center" });
        highlightedIni = target.toLowerCase();
        setTimeout(() => { highlightedIni = null; }, 1500);
      }
    });
  });
</script>

{#if iniFiles.length > 0}
  <div class="section">
    <h4 class="section-title">
      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" /><polyline points="14 2 14 8 20 8" /><line x1="16" y1="13" x2="8" y2="13" /><line x1="16" y1="17" x2="8" y2="17" /><polyline points="10 9 9 9 8 9" /></svg>
      INI Files
    </h4>
    {#if editingIniFile}
      <div class="ini-editor">
        <div class="ini-editor-header">
          <span class="ini-filename">{fileName(editingIniFile)}</span>
          <div class="ini-editor-actions">
            {#if iniCanRevert}
              <button class="ini-btn ini-btn-revert" onclick={revertIniFile} title="Revert to original">Revert</button>
            {/if}
            {#if iniDirty}
              <button class="ini-btn ini-btn-save" onclick={saveIniFile} disabled={iniSaving}>
                {iniSaving ? "Saving..." : "Save"}
              </button>
            {/if}
            <button class="ini-btn ini-btn-close" onclick={closeIniEditor}>Close</button>
          </div>
        </div>
        {#if iniLoading}
          <div class="ini-loading">Loading...</div>
        {:else}
          <textarea
            class="ini-textarea"
            bind:value={iniContent}
            spellcheck="false"
            wrap="off"
          ></textarea>
        {/if}
      </div>
    {:else}
      <div class="ini-file-list">
        {#each iniFiles as iniFile}
          {@const info = iniConflicts.get(iniFile.toLowerCase())}
          <div
            class="ini-file-entry"
            class:ini-highlight={highlightedIni === iniFile.toLowerCase()}
            use:registerIniRef={iniFile}
          >
            <button class="ini-file-item" onclick={() => openIniFile(iniFile)} disabled={!mod.staging_path}>
              <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" /><polyline points="14 2 14 8 20 8" /></svg>
              <span class="ini-file-name">{fileName(iniFile)}</span>
              {#if info?.overwrittenBy.length}
                <span class="ini-status-inactive" title="A higher-priority mod overwrites this file">inactive</span>
              {/if}
              <span class="ini-file-path">{iniFile}</span>
            </button>
            {#if info?.overwrittenBy.length}
              <div class="ini-conflict-row">
                <span class="ini-conflict-label ini-conflict-warn">Overwritten by:</span>
                {#each info.overwrittenBy as conflictMod}
                  <button class="ini-conflict-link ini-conflict-link-warn" onclick={() => onnavigatemod(conflictMod, iniFile)} title="Open {conflictMod.name}'s INI editor (priority {conflictMod.install_priority})">
                    {conflictMod.name}
                  </button>
                {/each}
              </div>
            {/if}
            {#if info?.overwrites.length}
              <div class="ini-conflict-row">
                <span class="ini-conflict-label">Overwrites:</span>
                {#each info.overwrites as conflictMod}
                  <button class="ini-conflict-link" onclick={() => onnavigatemod(conflictMod, iniFile)} title="Open {conflictMod.name}'s INI editor (priority {conflictMod.install_priority})">
                    {conflictMod.name}
                  </button>
                {/each}
              </div>
            {/if}
          </div>
        {/each}
      </div>
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

  .ini-file-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .ini-file-item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-2);
    border-radius: var(--radius-sm);
    background: none;
    border: none;
    cursor: pointer;
    font-family: inherit;
    transition: background var(--duration-fast) var(--ease);
    text-align: left;
    color: var(--text-primary);
    width: 100%;
  }
  .ini-file-item:hover {
    background: var(--surface-hover);
  }
  .ini-file-item:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .ini-file-item svg {
    flex-shrink: 0;
    color: var(--text-tertiary);
  }
  .ini-file-name {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-primary);
    white-space: nowrap;
  }
  .ini-file-path {
    font-size: 10px;
    color: var(--text-tertiary);
    font-family: var(--font-mono);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    margin-left: auto;
  }

  .ini-file-entry {
    display: flex;
    flex-direction: column;
    border-radius: var(--radius-sm);
    transition: background 0.3s ease;
  }
  .ini-highlight {
    background: var(--accent-subtle);
    animation: iniFadeHighlight 1.5s ease-out forwards;
  }
  @keyframes iniFadeHighlight {
    0% { background: var(--accent-subtle); }
    70% { background: var(--accent-subtle); }
    100% { background: transparent; }
  }

  .ini-conflict-row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px;
    padding: 0 var(--space-2) var(--space-1) 23px;
  }

  .ini-conflict-label {
    font-size: 10px;
    color: var(--text-quaternary);
    font-weight: 500;
  }
  .ini-conflict-warn {
    color: var(--orange, #ff9f0a);
  }

  .ini-conflict-link {
    font-size: 10px;
    font-weight: 600;
    color: var(--accent);
    background: var(--accent-subtle);
    border: none;
    border-radius: var(--radius-sm);
    padding: 1px 6px;
    cursor: pointer;
    font-family: inherit;
    transition: all var(--duration-fast) var(--ease);
    max-width: 150px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ini-conflict-link:hover {
    filter: brightness(1.15);
    text-decoration: underline;
  }
  .ini-conflict-link-warn {
    background: rgba(255, 159, 10, 0.1);
    color: var(--orange, #ff9f0a);
  }

  .ini-status-inactive {
    font-size: 9px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--orange, #ff9f0a);
    background: rgba(255, 159, 10, 0.1);
    padding: 0 5px;
    border-radius: var(--radius-sm);
    flex-shrink: 0;
  }

  .ini-editor {
    border: 1px solid var(--separator);
    border-radius: var(--radius-md);
    overflow: hidden;
  }

  .ini-editor-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-1) var(--space-2);
    background: var(--bg-secondary);
    border-bottom: 1px solid var(--separator);
  }

  .ini-filename {
    font-size: 11px;
    font-weight: 600;
    font-family: var(--font-mono);
    color: var(--text-secondary);
  }

  .ini-editor-actions {
    display: flex;
    gap: 4px;
  }

  .ini-btn {
    font-size: 10px;
    font-weight: 600;
    padding: 2px 8px;
    border-radius: var(--radius-sm);
    border: none;
    cursor: pointer;
    font-family: inherit;
    transition: all var(--duration-fast) var(--ease);
  }
  .ini-btn-save {
    background: var(--accent);
    color: white;
  }
  .ini-btn-save:hover {
    filter: brightness(1.1);
  }
  .ini-btn-save:disabled {
    opacity: 0.5;
    pointer-events: none;
  }
  .ini-btn-revert {
    background: rgba(255, 159, 10, 0.1);
    color: var(--orange, #ff9f0a);
  }
  .ini-btn-revert:hover {
    background: rgba(255, 159, 10, 0.18);
  }
  .ini-btn-close {
    background: var(--surface-hover);
    color: var(--text-secondary);
  }
  .ini-btn-close:hover {
    color: var(--text-primary);
  }

  .ini-textarea {
    width: 100%;
    min-height: 200px;
    max-height: 400px;
    padding: var(--space-2);
    background: var(--bg-base);
    color: var(--text-primary);
    font-family: var(--font-mono);
    font-size: 11px;
    line-height: 1.5;
    border: none;
    resize: vertical;
    tab-size: 4;
    outline: none;
    box-sizing: border-box;
  }

  .ini-loading {
    padding: var(--space-4);
    text-align: center;
    font-size: 12px;
    color: var(--text-tertiary);
  }
</style>
