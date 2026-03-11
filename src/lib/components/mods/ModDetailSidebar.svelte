<script lang="ts">
  import type { InstalledMod, ModUpdateInfo } from "$lib/types";

  interface Props {
    mod: InstalledMod;
    update: ModUpdateInfo | undefined;
    conflictNames: string[];
    nexusSlug: string | undefined;
    editingNotes: boolean;
    editingNotesValue: string;
    onClose: () => void;
    onToggle: () => void;
    onUninstall: () => void;
    onEditNotes: () => void;
    onSaveNotes: (value: string) => void;
    onCancelNotes: () => void;
  }

  let {
    mod, update, conflictNames, nexusSlug,
    editingNotes, editingNotesValue,
    onClose, onToggle, onUninstall, onEditNotes, onSaveNotes, onCancelNotes,
  }: Props = $props();

  let notesValue = $state(editingNotesValue);
  $effect(() => { notesValue = editingNotesValue; });

  function formatDate(iso: string): string {
    return new Date(iso).toLocaleDateString();
  }
</script>

<!-- Phase 5 will enhance with sticky positioning, slide-in transition, inline conflicts -->
<div class="mod-detail-panel">
  <div class="detail-header">
    <h3 class="detail-name">{mod.name}</h3>
    <button class="detail-close" onclick={onClose}>
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
        <span class="detail-value">{mod.file_count}</span>
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
      {#if mod.nexus_mod_id && nexusSlug}
        <div class="detail-row">
          <span class="detail-label">Nexus</span>
          <a class="detail-value detail-link" href="https://www.nexusmods.com/{nexusSlug}/mods/{mod.nexus_mod_id}" target="_blank" rel="noopener noreferrer">
            Mod #{mod.nexus_mod_id}
          </a>
        </div>
      {/if}
    </div>

    {#if update}
      <div class="detail-update-banner">
        <span class="detail-update-text">Update: v{update.current_version} &rarr; v{update.latest_version}</span>
      </div>
    {/if}

    {#if conflictNames.length > 0}
      <div class="detail-section">
        <h4 class="detail-section-title">Conflicts</h4>
        <div class="detail-conflict-list">
          {#each conflictNames as name}
            <span class="detail-conflict-badge">{name}</span>
          {/each}
        </div>
      </div>
    {/if}

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

    <div class="detail-section">
      <h4 class="detail-section-title">Notes</h4>
      {#if editingNotes}
        <textarea class="detail-notes-input" bind:value={notesValue} rows="3" placeholder="Add notes about this mod..."></textarea>
        <div class="detail-notes-actions">
          <button class="btn btn-primary btn-sm" onclick={() => onSaveNotes(notesValue)}>Save</button>
          <button class="btn btn-ghost btn-sm" onclick={onCancelNotes}>Cancel</button>
        </div>
      {:else}
        <button class="detail-notes-display" onclick={onEditNotes}>
          {mod.user_notes || "Click to add notes..."}
        </button>
      {/if}
    </div>

    <div class="detail-actions">
      <button class="btn btn-secondary btn-sm" onclick={onToggle}>
        {mod.enabled ? "Disable" : "Enable"}
      </button>
      <button class="btn btn-ghost-danger btn-sm" onclick={onUninstall}>Uninstall</button>
    </div>
  </div>
</div>
