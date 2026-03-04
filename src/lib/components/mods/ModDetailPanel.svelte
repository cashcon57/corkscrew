<script lang="ts">
  import { openUrl } from "@tauri-apps/plugin-opener";
  import DOMPurify from "dompurify";
  import { bbcodeToHtml } from "$lib/bbcode";
  import { getNexusModDetail, getModDependencies, getModDependents } from "$lib/api";
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
  }

  let {
    mod, nexusSlug, conflictModIds, conflictDetails, updateMap,
    endorsements, endorsingModId, confirmUninstall,
    onclose, ontoggle, onuninstall, onconfirmuninstall,
    onsavenotes, onendorse, onabstain, onreinstall, onreload,
  }: Props = $props();

  // Internal state — isolated from parent component's reactivity
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

  function getModSourceUrl(m: InstalledMod): string | null {
    if (m.source_url) return m.source_url;
    if (m.nexus_mod_id && nexusSlug) return `https://www.nexusmods.com/${nexusSlug}/mods/${m.nexus_mod_id}`;
    return null;
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
      .catch(() => {})
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
      .catch(() => {})
      .finally(() => { detailDepsLoading = false; });
  });
</script>

<div class="mod-detail-panel">
  <div class="detail-header">
    <h3 class="detail-name">{mod.name}</h3>
    <button class="detail-close" onclick={onclose}>
      <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
        <line x1="3" y1="3" x2="11" y2="11" /><line x1="11" y1="3" x2="3" y2="11" />
      </svg>
    </button>
  </div>

  <div class="detail-body">
    <div class="detail-meta">
      <div class="detail-row">
        <span class="detail-label">Version</span>
        <span class="detail-value">{mod.version || "\u2014"}</span>
      </div>
      <div class="detail-row">
        <span class="detail-label">Installed</span>
        <span class="detail-value">{formatDate(mod.installed_at)}</span>
      </div>
      <div class="detail-row">
        <span class="detail-label">Files</span>
        <span class="detail-value">{mod.installed_files.length}</span>
      </div>
      <div class="detail-row">
        <span class="detail-label">Priority</span>
        <span class="detail-value">{mod.install_priority}</span>
      </div>
      {#if mod.archive_name}
        <div class="detail-row">
          <span class="detail-label">Archive</span>
          <span class="detail-value detail-archive">{mod.archive_name}</span>
        </div>
      {/if}
      {#if mod.collection_name}
        <div class="detail-row">
          <span class="detail-label">Collection</span>
          <span class="detail-value collection-badge">{mod.collection_name}</span>
        </div>
      {/if}
      <div class="detail-row">
        <span class="detail-label">Source</span>
        <span class="detail-value">
          {#if getModSourceUrl(mod)}
            <button class="origin-label origin-{mod.source_type} origin-link" onclick={() => openUrl(getModSourceUrl(mod)!)}>
              {originLabel(mod.source_type)}
              <svg class="origin-link-icon" width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" /><polyline points="15 3 21 3 21 9" /><line x1="10" y1="14" x2="21" y2="3" /></svg>
            </button>
          {:else}
            <span class="origin-label origin-{mod.source_type}">{originLabel(mod.source_type)}</span>
          {/if}
        </span>
      </div>
      {#if mod.nexus_mod_id}
        <div class="detail-row">
          <span class="detail-label">Nexus</span>
          <a class="detail-value detail-link" href="https://www.nexusmods.com/{nexusSlug}/mods/{mod.nexus_mod_id}" target="_blank" rel="noopener noreferrer">
            Mod #{mod.nexus_mod_id}
          </a>
        </div>
        <div class="detail-row">
          <span class="detail-label">Endorse</span>
          <span class="detail-value">
            {#if endorsements.get(mod.nexus_mod_id) === "Endorsed"}
              <button class="btn btn-sm endorse-btn endorsed" onclick={() => onabstain(mod.id, mod.nexus_mod_id!)} disabled={endorsingModId === mod.id}>
                <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor" stroke="none"><path d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3zM7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3" /></svg>
                Endorsed
              </button>
            {:else}
              <button class="btn btn-sm endorse-btn" onclick={() => onendorse(mod.id, mod.nexus_mod_id!, mod.version)} disabled={endorsingModId === mod.id}>
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3zM7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3" /></svg>
                Endorse
              </button>
            {/if}
          </span>
        </div>
      {/if}
    </div>

    <!-- NexusMods Detail (loaded internally) -->
    {#if nexusDetailLoading}
      <div class="detail-section">
        <span class="detail-empty">Loading mod details...</span>
      </div>
    {:else if nexusDetail}
      <div class="detail-section nexus-detail-section">
        {#if nexusDetail.summary}
          <p class="nexus-summary">{nexusDetail.summary}</p>
        {/if}
        <div class="nexus-stats">
          <span class="nexus-stat" title="Author">
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2" /><circle cx="12" cy="7" r="4" /></svg>
            {nexusDetail.author}
          </span>
          <span class="nexus-stat" title="Endorsements">
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3zM7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3" /></svg>
            {nexusDetail.endorsement_count.toLocaleString()}
          </span>
          <span class="nexus-stat" title="Unique downloads">
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" /><polyline points="7 10 12 15 17 10" /><line x1="12" y1="15" x2="12" y2="3" /></svg>
            {nexusDetail.unique_downloads.toLocaleString()}
          </span>
        </div>
        {#if nexusDetail.description}
          <details class="nexus-description-toggle">
            <summary>Show full description</summary>
            <div class="nexus-description">
              <!-- eslint-disable-next-line svelte/no-at-html-tags -->
              {@html DOMPurify.sanitize(bbcodeToHtml(nexusDetail.description))}
            </div>
          </details>
        {/if}
      </div>
    {/if}

    {#if updateMap.has(mod.id)}
      {@const update = updateMap.get(mod.id)!}
      <div class="detail-update-banner">
        <span class="detail-update-text">Update: v{update.current_version} &rarr; v{update.latest_version}</span>
      </div>
    {/if}

    {#if conflictModIds.has(mod.id)}
      <div class="detail-section">
        <h4 class="detail-section-title">Conflicts</h4>
        <div class="detail-conflict-list">
          {#each [...(conflictDetails.get(mod.id) ?? [])] as conflictName}
            <span class="detail-conflict-badge">{conflictName}</span>
          {/each}
        </div>
      </div>
    {/if}

    <!-- Dependencies -->
    {#if detailDepsLoading}
      <div class="detail-section">
        <h4 class="detail-section-title">Dependencies</h4>
        <span class="detail-empty">Loading...</span>
      </div>
    {:else if detailDeps.length > 0 || detailDependents.length > 0}
      <div class="detail-section">
        <h4 class="detail-section-title">Dependencies</h4>
        {#if detailDeps.length > 0}
          <div class="detail-dep-group">
            <span class="detail-dep-label">Depends on</span>
            <div class="detail-dep-list">
              {#each detailDeps as dep (dep.id)}
                <span class="detail-dep-badge" class:dep-requires={dep.relationship === "requires"} class:dep-conflicts={dep.relationship === "conflicts"} class:dep-patches={dep.relationship === "patches"}>
                  <span class="dep-rel-tag">{dep.relationship === "requires" ? "req" : dep.relationship === "conflicts" ? "conflict" : "patch"}</span>
                  {dep.dep_name}
                </span>
              {/each}
            </div>
          </div>
        {/if}
        {#if detailDependents.length > 0}
          <div class="detail-dep-group">
            <span class="detail-dep-label">Required by</span>
            <div class="detail-dep-list">
              {#each detailDependents as dep (dep.id)}
                {@const depMod = $installedMods.find(m => m.id === dep.mod_id)}
                <span class="detail-dep-badge dep-requires">
                  {depMod?.name ?? dep.dep_name ?? `Mod #${dep.mod_id}`}
                </span>
              {/each}
            </div>
          </div>
        {/if}
      </div>
    {/if}

    <!-- Tags -->
    <div class="detail-section">
      <h4 class="detail-section-title">Tags</h4>
      <div class="detail-tags">
        {#each mod.user_tags as tag}
          <span class="detail-tag">{tag}</span>
        {/each}
        {#if mod.user_tags.length === 0}
          <span class="detail-empty">No tags</span>
        {/if}
      </div>
    </div>

    <!-- Notes -->
    <div class="detail-section">
      <h4 class="detail-section-title">Notes</h4>
      {#if editingNotesId === mod.id}
        <textarea class="detail-notes-input" bind:value={editingNotesValue} rows="3" placeholder="Add notes about this mod..."></textarea>
        <div class="detail-notes-actions">
          <button class="btn btn-primary btn-sm" onclick={() => { onsavenotes(mod.id, editingNotesValue); editingNotesId = null; }}>Save</button>
          <button class="btn btn-ghost btn-sm" onclick={() => editingNotesId = null}>Cancel</button>
        </div>
      {:else}
        <button class="detail-notes-display" onclick={() => { editingNotesId = mod.id; editingNotesValue = mod.user_notes ?? ""; }}>
          {mod.user_notes || "Click to add notes..."}
        </button>
      {/if}
    </div>

    <!-- Version History -->
    <div class="detail-section">
      <ModVersionHistory {mod} onrollback={onreload} />
    </div>

    <!-- Actions -->
    <div class="detail-actions">
      <button class="btn btn-secondary btn-sm" onclick={() => ontoggle(mod)}>
        {mod.enabled ? "Disable" : "Enable"}
      </button>
      {#if confirmUninstall === mod.id}
        <button class="btn btn-danger btn-sm" onclick={() => onuninstall(mod.id)}>Confirm</button>
        <button class="btn btn-ghost btn-sm" onclick={() => onconfirmuninstall(null)}>Cancel</button>
      {:else}
        <button class="btn btn-ghost-danger btn-sm" onclick={() => onconfirmuninstall(mod.id)}>Uninstall</button>
      {/if}
    </div>
  </div>
</div>
