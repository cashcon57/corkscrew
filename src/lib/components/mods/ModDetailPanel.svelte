<script lang="ts">
  import { tick } from "svelte";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import DOMPurify from "dompurify";
  import { bbcodeToHtml } from "$lib/bbcode";
  import { getNexusModDetail, getModDependencies, getModDependents, readModFile, writeModFile } from "$lib/api";
  import { installedMods, showError, showSuccess } from "$lib/stores";
  import type { InstalledMod, ModUpdateInfo, NexusModInfo, ModDependency } from "$lib/types";
  import ModVersionHistory from "$lib/components/ModVersionHistory.svelte";

  interface Props {
    mod: InstalledMod;
    nexusSlug: string | undefined;
    conflictModIds: Set<number>;
    conflictDetails: Map<number, Set<string>>;
    updateMap: Map<number, ModUpdateInfo>;
    endorsements: Map<number, string>;
    endorsingModId: number | null;
    confirmUninstall: number | null;
    onclose: () => void;
    ontoggle: (mod: InstalledMod) => void;
    onuninstall: (id: number) => void;
    onconfirmuninstall: (id: number | null) => void;
    onsavenotes: (id: number, value: string) => void;
    onendorse: (id: number, nexusModId: number, version: string) => void;
    onabstain: (id: number, nexusModId: number) => void;
    onreinstall: (mod: InstalledMod) => void;
    onreload: () => void;
    onnavigatemod: (mod: InstalledMod, iniFile?: string) => void;
    scrollToIni: string | null;
  }

  let {
    mod, nexusSlug, conflictModIds, conflictDetails, updateMap,
    endorsements, endorsingModId, confirmUninstall,
    onclose, ontoggle, onuninstall, onconfirmuninstall,
    onsavenotes, onendorse, onabstain, onreinstall, onreload, onnavigatemod,
    scrollToIni,
  }: Props = $props();

  // Internal state
  let nexusDetail = $state<NexusModInfo | null>(null);
  let nexusDetailLoading = $state(false);
  let nexusDetailModId = $state<number | null>(null);
  let detailDeps = $state<ModDependency[]>([]);
  let detailDependents = $state<ModDependency[]>([]);
  let detailDepsLoading = $state(false);
  let editingNotesId = $state<number | null>(null);
  let editingNotesValue = $state("");

  // File browser state (virtualized)
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

  // INI editor state
  let iniFiles = $derived(
    mod.installed_files
      .filter(f => f.toLowerCase().endsWith('.ini'))
      .sort((a, b) => fileName(a).localeCompare(fileName(b), undefined, { sensitivity: 'base' }))
  );
  let editingIniFile = $state<string | null>(null);
  let iniContent = $state("");
  let iniOriginalContent = $state("");  // updates on save (tracks "last saved")
  let iniLoadedOriginal = $state("");   // original content when first opened (for revert)
  let iniLoading = $state(false);
  let iniSaving = $state(false);

  // INI conflict detection: find other mods that provide the same INI file paths,
  // split into "overwritten by" (higher priority) and "overwrites" (lower priority)
  interface IniConflictInfo {
    overwrittenBy: InstalledMod[];  // higher priority — their file wins over ours
    overwrites: InstalledMod[];     // lower priority — our file wins over theirs
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

  function formatDate(iso: string): string {
    return new Date(iso).toLocaleDateString();
  }

  function getModSourceUrl(m: InstalledMod): string | null {
    if (m.source_url) return m.source_url;
    if (m.nexus_mod_id && nexusSlug) return `https://www.nexusmods.com/${nexusSlug}/mods/${m.nexus_mod_id}`;
    return null;
  }

  function originLabel(t: string): string {
    const labels: Record<string, string> = { nexus: "Nexus", loverslab: "LoversLab", moddb: "ModDB", curseforge: "CurseForge", direct: "Direct", manual: "Manual" };
    return labels[t] ?? t;
  }

  function fileName(path: string): string {
    return path.split('/').pop() ?? path;
  }

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

  let iniCanRevert = $derived(iniLoadedOriginal !== "" && iniContent !== iniLoadedOriginal);

  let iniDirty = $derived(iniContent !== iniOriginalContent);

  // Load NexusMods detail when mod changes
  $effect(() => {
    const m = mod;
    if (!m.nexus_mod_id || !nexusSlug) {
      nexusDetail = null;
      nexusDetailModId = null;
      return;
    }
    if (nexusDetailModId === m.id) return;
    nexusDetailModId = m.id;
    nexusDetailLoading = true;
    nexusDetail = null;
    getNexusModDetail(nexusSlug, m.nexus_mod_id)
      .then((info) => { if (mod.id === m.id) nexusDetail = info; })
      .catch((err) => console.error('Failed to load mod details:', err))
      .finally(() => { nexusDetailLoading = false; });
  });

  // Load dependencies when mod changes
  $effect(() => {
    const m = mod;
    detailDepsLoading = true;
    Promise.all([getModDependencies(m.id), getModDependents(m.id)])
      .then(([deps, dependents]) => {
        if (mod.id === m.id) { detailDeps = deps; detailDependents = dependents; }
      })
      .catch((err) => console.error('Failed to load mod dependencies:', err))
      .finally(() => { detailDepsLoading = false; });
  });

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
    // Wait for DOM to update after mod switch
    tick().then(() => {
      const el = iniFileRefs.get(target.toLowerCase());
      if (el) {
        el.scrollIntoView({ behavior: "smooth", block: "center" });
        highlightedIni = target.toLowerCase();
        // Clear highlight after animation
        setTimeout(() => { highlightedIni = null; }, 1500);
      }
    });
  });
</script>

<div class="panel">
  <!-- Header -->
  <div class="panel-header">
    <div class="header-top">
      <h3 class="mod-name">{mod.name}</h3>
      <button class="close-btn" onclick={onclose} title="Close">
        <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
          <line x1="3" y1="3" x2="11" y2="11" /><line x1="11" y1="3" x2="3" y2="11" />
        </svg>
      </button>
    </div>
    {#if nexusDetail}
      <p class="mod-summary">{nexusDetail.summary}</p>
    {/if}
    <div class="header-chips">
      <span class="chip">{mod.version || "—"}</span>
      <span class="chip chip-muted">{mod.file_count} files</span>
      {#if mod.collection_name}
        <span class="chip chip-collection">{mod.collection_name}</span>
      {/if}
      <span class="chip chip-source">
        {originLabel(mod.source_type)}
      </span>
    </div>
  </div>

  <!-- Scrollable Body -->
  <div class="panel-body">
    <!-- Update banner -->
    {#if updateMap.has(mod.id)}
      {@const update = updateMap.get(mod.id)!}
      <div class="update-banner">
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" /><polyline points="7 10 12 15 17 10" /><line x1="12" y1="15" x2="12" y2="3" /></svg>
        <span>v{update.current_version} &rarr; v{update.latest_version}</span>
      </div>
    {/if}

    <!-- Meta Grid -->
    <div class="section">
      <div class="meta-grid">
        <div class="meta-item">
          <span class="meta-label">Installed</span>
          <span class="meta-value">{formatDate(mod.installed_at)}</span>
        </div>
        <div class="meta-item">
          <span class="meta-label">Priority</span>
          <span class="meta-value">{mod.install_priority}</span>
        </div>
        {#if mod.archive_name}
          <div class="meta-item meta-full">
            <span class="meta-label">Archive</span>
            <span class="meta-value meta-mono">{mod.archive_name}</span>
          </div>
        {/if}
      </div>
    </div>

    <!-- Source & Nexus -->
    <div class="section">
      <h4 class="section-title">Source</h4>
      <div class="source-row">
        {#if getModSourceUrl(mod)}
          <button class="source-link" onclick={() => openUrl(getModSourceUrl(mod)!)}>
            {originLabel(mod.source_type)}
            <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" /><polyline points="15 3 21 3 21 9" /><line x1="10" y1="14" x2="21" y2="3" /></svg>
          </button>
        {:else}
          <span class="source-label">{originLabel(mod.source_type)}</span>
        {/if}
        {#if mod.nexus_mod_id}
          <a class="nexus-link" href="https://www.nexusmods.com/{nexusSlug}/mods/{mod.nexus_mod_id}" target="_blank" rel="noopener noreferrer">
            Mod #{mod.nexus_mod_id}
          </a>
        {/if}
      </div>

      {#if nexusDetail}
        <div class="nexus-stats">
          <span class="stat" title="Author">
            <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2" /><circle cx="12" cy="7" r="4" /></svg>
            {nexusDetail.author}
          </span>
          <span class="stat" title="Endorsements">
            <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3zM7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3" /></svg>
            {nexusDetail.endorsement_count.toLocaleString()}
          </span>
          <span class="stat" title="Downloads">
            <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" /><polyline points="7 10 12 15 17 10" /><line x1="12" y1="15" x2="12" y2="3" /></svg>
            {nexusDetail.unique_downloads.toLocaleString()}
          </span>
        </div>
      {:else if nexusDetailLoading}
        <span class="empty-text">Loading details...</span>
      {/if}

      {#if mod.nexus_mod_id}
        <div class="endorse-row">
          {#if endorsements.get(mod.nexus_mod_id) === "Endorsed"}
            <button class="endorse-btn endorsed" onclick={() => onabstain(mod.id, mod.nexus_mod_id!)} disabled={endorsingModId === mod.id}>
              <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor" stroke="none"><path d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3zM7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3" /></svg>
              Endorsed
            </button>
          {:else}
            <button class="endorse-btn" onclick={() => onendorse(mod.id, mod.nexus_mod_id!, mod.version)} disabled={endorsingModId === mod.id}>
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3zM7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3" /></svg>
              Endorse
            </button>
          {/if}
        </div>
      {/if}
    </div>

    <!-- Description -->
    {#if nexusDetail?.description}
      <div class="section">
        <details class="description-toggle">
          <summary class="section-title clickable">Full Description</summary>
          <div class="description-content">
            <!-- eslint-disable-next-line svelte/no-at-html-tags -->
            {@html DOMPurify.sanitize(bbcodeToHtml(nexusDetail.description))}
          </div>
        </details>
      </div>
    {/if}

    <!-- Conflicts -->
    {#if conflictModIds.has(mod.id)}
      <div class="section">
        <h4 class="section-title">Conflicts</h4>
        <div class="badge-list">
          {#each [...(conflictDetails.get(mod.id) ?? [])] as conflictName}
            <span class="badge badge-red">{conflictName}</span>
          {/each}
        </div>
      </div>
    {/if}

    <!-- Dependencies -->
    {#if detailDepsLoading}
      <div class="section">
        <h4 class="section-title">Dependencies</h4>
        <span class="empty-text">Loading...</span>
      </div>
    {:else if detailDeps.length > 0 || detailDependents.length > 0}
      <div class="section">
        <h4 class="section-title">Dependencies</h4>
        {#if detailDeps.length > 0}
          <div class="dep-group">
            <span class="dep-label">Depends on</span>
            <div class="badge-list">
              {#each detailDeps as dep (dep.id)}
                <span class="badge" class:badge-blue={dep.relationship === "requires"} class:badge-red={dep.relationship === "conflicts"} class:badge-yellow={dep.relationship === "patches"}>
                  <span class="dep-tag">{dep.relationship === "requires" ? "req" : dep.relationship === "conflicts" ? "conflict" : "patch"}</span>
                  {dep.dep_name}
                </span>
              {/each}
            </div>
          </div>
        {/if}
        {#if detailDependents.length > 0}
          <div class="dep-group">
            <span class="dep-label">Required by</span>
            <div class="badge-list">
              {#each detailDependents as dep (dep.id)}
                {@const depMod = $installedMods.find(m => m.id === dep.mod_id)}
                <span class="badge badge-blue">
                  {depMod?.name ?? dep.dep_name ?? `Mod #${dep.mod_id}`}
                </span>
              {/each}
            </div>
          </div>
        {/if}
      </div>
    {/if}

    <!-- INI Files -->
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

    <!-- File Browser (virtualized) -->
    {#if mod.installed_files.length > 0}
      <div class="section">
        <button class="section-title section-title-toggle" onclick={() => fileBrowserOpen = !fileBrowserOpen} type="button">
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" /></svg>
          Files ({mod.installed_files.length})
          <svg class="chevron-toggle" class:chevron-open={fileBrowserOpen} width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="6 9 12 15 18 9" /></svg>
        </button>
        {#if fileBrowserOpen}
          {#if fileSearchQuery}
            {@const filtered = mod.installed_files.filter(f => f.toLowerCase().includes(fileSearchQuery.toLowerCase()))}
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
              <div style="height: {mod.installed_files.length * 24}px; position: relative;">
                {#each mod.installed_files.slice(fileBrowserStart, fileBrowserStart + fileBrowserVisible) as file, i}
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

    <!-- Tags -->
    <div class="section">
      <h4 class="section-title">Tags</h4>
      <div class="badge-list">
        {#each mod.user_tags as tag}
          <span class="badge badge-accent">{tag}</span>
        {/each}
        {#if mod.user_tags.length === 0}
          <span class="empty-text">No tags</span>
        {/if}
      </div>
    </div>

    <!-- Notes -->
    <div class="section">
      <h4 class="section-title">Notes</h4>
      {#if editingNotesId === mod.id}
        <textarea class="notes-input" bind:value={editingNotesValue} rows="3" placeholder="Add notes about this mod..."></textarea>
        <div class="notes-actions">
          <button class="action-btn action-primary" onclick={() => { onsavenotes(mod.id, editingNotesValue); editingNotesId = null; }}>Save</button>
          <button class="action-btn action-ghost" onclick={() => editingNotesId = null}>Cancel</button>
        </div>
      {:else}
        <button class="notes-display" onclick={() => { editingNotesId = mod.id; editingNotesValue = mod.user_notes ?? ""; }}>
          {mod.user_notes || "Click to add notes..."}
        </button>
      {/if}
    </div>

    <!-- Version History -->
    <div class="section">
      <ModVersionHistory {mod} onrollback={onreload} />
    </div>
  </div>

  <!-- Footer Actions -->
  <div class="panel-footer">
    <button class="action-btn action-secondary" onclick={() => ontoggle(mod)}>
      {mod.enabled ? "Disable" : "Enable"}
    </button>
    {#if confirmUninstall === mod.id}
      <button class="action-btn action-danger" onclick={() => onuninstall(mod.id)}>Confirm</button>
      <button class="action-btn action-ghost" onclick={() => onconfirmuninstall(null)}>Cancel</button>
    {:else}
      <button class="action-btn action-ghost-danger" onclick={() => onconfirmuninstall(mod.id)}>Uninstall</button>
    {/if}
  </div>
</div>

<style>
  /* ==========================================
     Panel Container
     ========================================== */
  .panel {
    width: 320px;
    min-width: 280px;
    max-width: 380px;
    flex-shrink: 0;
    border-radius: var(--radius-lg);
    background: var(--bg-primary);
    box-shadow: var(--glass-refraction), var(--glass-edge-shadow);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    animation: slideIn 0.15s var(--ease-out);
  }

  @keyframes slideIn {
    from { opacity: 0; transform: translateX(8px); }
    to { opacity: 1; transform: translateX(0); }
  }

  /* ==========================================
     Header
     ========================================== */
  .panel-header {
    padding: var(--space-4);
    border-bottom: 1px solid var(--separator);
  }

  .header-top {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-2);
    margin-bottom: var(--space-2);
  }

  .mod-name {
    font-size: 15px;
    font-weight: 700;
    color: var(--text-primary);
    line-height: 1.3;
    word-break: break-word;
    margin: 0;
  }

  .mod-summary {
    font-size: 12px;
    color: var(--text-secondary);
    line-height: 1.45;
    margin: 0 0 var(--space-2) 0;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .close-btn {
    flex-shrink: 0;
    width: 24px;
    height: 24px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-sm);
    color: var(--text-tertiary);
    cursor: pointer;
    background: none;
    border: none;
    transition: all var(--duration-fast) var(--ease);
  }
  .close-btn:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .header-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }

  .chip {
    font-size: 11px;
    font-weight: 600;
    padding: 2px 8px;
    border-radius: var(--radius-sm);
    background: var(--accent-subtle);
    color: var(--accent);
  }
  .chip-muted {
    background: var(--surface-hover);
    color: var(--text-secondary);
  }
  .chip-collection {
    background: rgba(175, 82, 222, 0.12);
    color: rgb(175, 82, 222);
  }
  .chip-source {
    background: var(--surface-hover);
    color: var(--text-tertiary);
  }

  /* ==========================================
     Body (scrollable)
     ========================================== */
  .panel-body {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-3) var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

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

  .section-title.clickable {
    cursor: pointer;
  }

  .empty-text {
    font-size: 12px;
    color: var(--text-quaternary);
  }

  /* ==========================================
     Update Banner
     ========================================== */
  .update-banner {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: rgba(48, 209, 88, 0.08);
    border: 1px solid rgba(48, 209, 88, 0.15);
    border-radius: var(--radius-md);
    font-size: 12px;
    font-weight: 600;
    color: var(--green);
  }

  /* ==========================================
     Meta Grid
     ========================================== */
  .meta-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-2);
  }
  .meta-item {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .meta-full {
    grid-column: 1 / -1;
  }
  .meta-label {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-tertiary);
  }
  .meta-value {
    font-size: 12px;
    color: var(--text-primary);
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta-mono {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-secondary);
  }

  /* ==========================================
     Source
     ========================================== */
  .source-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-2);
  }

  .source-link {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
    font-weight: 600;
    color: var(--accent);
    background: none;
    border: none;
    cursor: pointer;
    padding: 0;
  }
  .source-link:hover { text-decoration: underline; }

  .source-label {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .nexus-link {
    font-size: 11px;
    color: var(--accent);
    text-decoration: none;
  }
  .nexus-link:hover { text-decoration: underline; }

  .nexus-stats {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    margin-bottom: var(--space-2);
  }

  .stat {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 11px;
    color: var(--text-tertiary);
  }
  .stat svg { opacity: 0.6; }

  .endorse-row {
    margin-top: var(--space-2);
  }

  .endorse-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    padding: 4px 10px;
    border-radius: var(--radius-sm);
    background: var(--bg-tertiary);
    color: var(--text-secondary);
    border: 1px solid var(--border-primary);
    cursor: pointer;
    transition: all 0.15s ease;
    font-family: inherit;
    font-weight: 500;
  }
  .endorse-btn:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }
  .endorse-btn.endorsed {
    background: rgba(48, 209, 88, 0.1);
    color: var(--green);
    border-color: rgba(48, 209, 88, 0.25);
  }
  .endorse-btn:disabled {
    opacity: 0.5;
    pointer-events: none;
  }

  /* ==========================================
     Description
     ========================================== */
  .description-toggle {
    margin: 0;
  }
  .description-toggle summary {
    list-style: none;
    cursor: pointer;
  }
  .description-toggle summary::-webkit-details-marker {
    display: none;
  }
  .description-content {
    font-size: 12px;
    color: var(--text-secondary);
    line-height: 1.6;
    max-height: 250px;
    overflow-y: auto;
    margin-top: var(--space-2);
    padding: var(--space-2);
    background: var(--surface-hover);
    border-radius: var(--radius-sm);
    word-break: break-word;
  }

  /* ==========================================
     Badges (conflicts, dependencies, tags)
     ========================================== */
  .badge-list {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }

  .badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    padding: 2px 7px;
    border-radius: var(--radius-sm);
    font-weight: 500;
    max-width: 200px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    background: var(--surface-hover);
    color: var(--text-secondary);
  }
  .badge-red {
    background: rgba(255, 69, 58, 0.1);
    color: var(--red);
  }
  .badge-blue {
    background: rgba(0, 122, 255, 0.1);
    color: var(--system-accent);
  }
  .badge-yellow {
    background: rgba(255, 214, 10, 0.1);
    color: var(--yellow);
  }
  .badge-accent {
    background: var(--accent-subtle);
    color: var(--accent);
  }

  .dep-group {
    margin-bottom: var(--space-2);
  }
  .dep-group:last-child { margin-bottom: 0; }
  .dep-label {
    display: block;
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-tertiary);
    margin-bottom: var(--space-1);
  }
  .dep-tag {
    font-size: 9px;
    font-weight: 700;
    text-transform: uppercase;
    opacity: 0.7;
  }

  /* ==========================================
     INI Editor
     ========================================== */
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

  /* ==========================================
     Notes
     ========================================== */
  .notes-input {
    width: 100%;
    padding: var(--space-2);
    background: var(--bg-base);
    border: 1px solid var(--separator);
    border-radius: var(--radius-sm);
    color: var(--text-primary);
    font-size: 12px;
    font-family: inherit;
    resize: vertical;
    box-sizing: border-box;
  }
  .notes-input:focus {
    outline: none;
    border-color: var(--accent);
  }

  .notes-actions {
    display: flex;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }

  .notes-display {
    width: 100%;
    text-align: left;
    padding: var(--space-2);
    font-size: 12px;
    color: var(--text-secondary);
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: background var(--duration-fast) var(--ease);
    white-space: pre-wrap;
    word-break: break-word;
    line-height: 1.5;
    background: none;
    border: none;
    font-family: inherit;
  }
  .notes-display:hover {
    background: var(--surface-hover);
  }

  /* ==========================================
     Footer Actions
     ========================================== */
  .panel-footer {
    display: flex;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-top: 1px solid var(--separator);
  }

  .action-btn {
    font-size: 12px;
    font-weight: 600;
    padding: 5px 12px;
    border-radius: var(--radius-sm);
    border: none;
    cursor: pointer;
    font-family: inherit;
    transition: all var(--duration-fast) var(--ease);
  }
  .action-primary {
    background: var(--accent);
    color: white;
  }
  .action-primary:hover { filter: brightness(1.1); }
  .action-secondary {
    background: var(--surface-hover);
    color: var(--text-primary);
    border: 1px solid var(--border-primary);
  }
  .action-secondary:hover { background: var(--bg-tertiary); }
  .action-ghost {
    background: none;
    color: var(--text-secondary);
  }
  .action-ghost:hover { color: var(--text-primary); background: var(--surface-hover); }
  .action-danger {
    background: var(--red);
    color: white;
  }
  .action-danger:hover { filter: brightness(1.1); }
  .action-ghost-danger {
    background: none;
    color: var(--text-tertiary);
  }
  .action-ghost-danger:hover {
    color: var(--red);
    background: rgba(255, 69, 58, 0.08);
  }

  /* ---- File Browser ---- */

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
