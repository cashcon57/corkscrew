<script lang="ts">
  import { getNexusModDetail, getModDependencies, getModDependents } from "$lib/api";
  import { installedMods } from "$lib/stores";
  import type { InstalledMod, ModUpdateInfo, NexusModInfo, ModDependency } from "$lib/types";
  import ModVersionHistory from "$lib/components/ModVersionHistory.svelte";
  import ModDetailInfo from "$lib/components/mods/ModDetailInfo.svelte";
  import ModDetailFiles from "$lib/components/mods/ModDetailFiles.svelte";
  import ModDetailIniEditor from "$lib/components/mods/ModDetailIniEditor.svelte";

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

  function formatDate(iso: string): string {
    return new Date(iso).toLocaleDateString();
  }

  function originLabel(t: string): string {
    const labels: Record<string, string> = { nexus: "Nexus", loverslab: "LoversLab", moddb: "ModDB", curseforge: "CurseForge", direct: "Direct", manual: "Manual" };
    return labels[t] ?? t;
  }

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

    <!-- Source & Nexus & Description -->
    <ModDetailInfo
      {mod}
      {nexusSlug}
      {nexusDetail}
      {nexusDetailLoading}
      {endorsements}
      {endorsingModId}
      {onendorse}
      {onabstain}
    />

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
    <ModDetailIniEditor {mod} {scrollToIni} {onnavigatemod} />

    <!-- File Browser (virtualized) -->
    <ModDetailFiles files={mod.installed_files} />

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

</style>
