<script lang="ts">
  import { openUrl } from "@tauri-apps/plugin-opener";
  import DOMPurify from "dompurify";
  import { bbcodeToHtml } from "$lib/bbcode";
  import type { InstalledMod, NexusModInfo } from "$lib/types";

  interface Props {
    mod: InstalledMod;
    nexusSlug: string | undefined;
    nexusDetail: NexusModInfo | null;
    nexusDetailLoading: boolean;
    endorsements: Map<number, string>;
    endorsingModId: number | null;
    onendorse: (id: number, nexusModId: number, version: string) => void;
    onabstain: (id: number, nexusModId: number) => void;
  }

  let {
    mod, nexusSlug, nexusDetail, nexusDetailLoading,
    endorsements, endorsingModId,
    onendorse, onabstain,
  }: Props = $props();

  function getModSourceUrl(m: InstalledMod): string | null {
    if (m.source_url) return m.source_url;
    if (m.nexus_mod_id && nexusSlug) return `https://www.nexusmods.com/${nexusSlug}/mods/${m.nexus_mod_id}`;
    return null;
  }

  function originLabel(t: string): string {
    const labels: Record<string, string> = { nexus: "Nexus", loverslab: "LoversLab", moddb: "ModDB", curseforge: "CurseForge", direct: "Direct", manual: "Manual" };
    return labels[t] ?? t;
  }
</script>

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

  .section-title.clickable {
    cursor: pointer;
  }

  .empty-text {
    font-size: 12px;
    color: var(--text-quaternary);
  }

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
</style>
