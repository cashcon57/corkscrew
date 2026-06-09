<script lang="ts">
  import type { NexusModInfo, NexusModFile } from "$lib/types";
  import { showError } from "$lib/stores";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import DOMPurify from "dompurify";
  import { bbcodeToHtml } from "$lib/bbcode";
  import { getNexusModDetail, getModFiles, getModRequirements, type ModRequirements, type ModRef } from "$lib/api";
  import { relativeTime, absoluteDate } from "$lib/relativeTime";

  interface Props {
    mod: NexusModInfo;
    gameSlug: string;
    account: { connected: boolean; is_premium?: boolean } | null;
    installedNexusIds: Set<number>;
    onback: () => void;
    oninstall: (mod: NexusModInfo) => void;
    ondownloadfile: (mod: NexusModInfo, file: NexusModFile) => void;
    downloadingFileId: number | null;
  }

  let { mod, gameSlug, account, installedNexusIds, onback, oninstall, ondownloadfile, downloadingFileId }: Props = $props();

  let modDetail = $state<NexusModInfo | null>(null);
  let modFiles = $state<NexusModFile[]>([]);
  let loading = $state(false);
  let renderedDescription = $state("");
  let requirements = $state<ModRequirements | null>(null);
  let requirementsLoading = $state(false);

  function formatDownloads(n: number): string {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
    return n.toString();
  }

  function formatFileSize(kb: number): string {
    if (kb >= 1_048_576) return `${(kb / 1_048_576).toFixed(1)} GB`;
    if (kb >= 1024) return `${(kb / 1024).toFixed(1)} MB`;
    return `${kb} KB`;
  }

  function safeOpenUrl(url: string | null | undefined) {
    if (!url) return;
    try {
      const parsed = new URL(url);
      if (parsed.protocol === "http:" || parsed.protocol === "https:") {
        openUrl(url);
      } else {
        showError(`Blocked unsafe URL scheme: ${parsed.protocol}`);
      }
    } catch {
      showError("Invalid URL");
    }
  }

  function handleRenderedLinkClick(e: MouseEvent) {
    const target = (e.target as HTMLElement)?.closest("a");
    if (!target) return;
    const href = target.getAttribute("href");
    if (href) {
      e.preventDefault();
      e.stopPropagation();
      safeOpenUrl(href);
    }
  }

  $effect(() => {
    // Load detail when mod changes
    const currentMod = mod;
    const slug = gameSlug;
    if (currentMod && slug) {
      loadDetail(currentMod, slug);
    }
  });

  async function loadDetail(m: NexusModInfo, slug: string) {
    modDetail = null;
    modFiles = [];
    renderedDescription = "";
    requirements = null;
    loading = true;
    try {
      const [detail, files] = await Promise.all([
        getNexusModDetail(slug, m.mod_id),
        getModFiles(slug, m.mod_id).catch((err) => {
          console.error("getModFiles failed:", err);
          return [] as NexusModFile[];
        }),
      ]);
      modDetail = detail;
      const categoryOrder: Record<string, number> = { main: 0, update: 1, optional: 2, miscellaneous: 3, old_version: 4 };
      modFiles = files
        .filter((f: NexusModFile) => f.category !== "deleted" && f.category !== "archived")
        .sort((a: NexusModFile, b: NexusModFile) => (categoryOrder[a.category] ?? 5) - (categoryOrder[b.category] ?? 5));
      if (detail.description) {
        renderedDescription = DOMPurify.sanitize(bbcodeToHtml(detail.description));
      }
      // Kick off requirements fetch separately — non-blocking + gracefully hidden on failure
      loadRequirements(slug, m.mod_id);
    } catch (e) {
      showError(`Failed to load mod details: ${e}`);
      onback();
    } finally {
      loading = false;
    }
  }

  async function loadRequirements(slug: string, modId: number) {
    requirementsLoading = true;
    try {
      requirements = await getModRequirements(slug, modId);
    } catch (err) {
      console.error("getModRequirements failed (hiding section):", err);
      requirements = null;
    } finally {
      requirementsLoading = false;
    }
  }

  function openModRef(ref: ModRef) {
    if (ref.url) safeOpenUrl(ref.url);
  }
</script>

<div class="detail-view">
  <div class="detail-header">
    <button class="btn btn-ghost" onclick={onback}>
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M19 12H5" />
        <polyline points="12 19 5 12 12 5" />
      </svg>
      Back to Browse
    </button>
    <button class="btn btn-ghost btn-sm" onclick={() => safeOpenUrl(`https://www.nexusmods.com/${gameSlug}/mods/${mod.mod_id}`)}>
      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
        <polyline points="15 3 21 3 21 9" />
        <line x1="10" y1="14" x2="21" y2="3" />
      </svg>
      View on NexusMods
    </button>
  </div>

  {#if loading}
    <div class="loading-container">
      <div class="loading-card">
        <div class="spinner"><div class="spinner-ring"></div></div>
        <div class="loading-text">
          <p class="loading-title">Loading mod details</p>
          <p class="loading-detail">{mod.name}</p>
        </div>
      </div>
    </div>
  {:else if modDetail}
    <div class="detail-content">
      {#if modDetail.picture_url}
        <div class="mod-detail-hero" style="background-image: url({modDetail.picture_url})"></div>
      {/if}

      <div class="detail-title-section">
        <div class="detail-title-row">
          <h2 class="detail-name">{modDetail.name}</h2>
        </div>
        <p class="detail-author">by {modDetail.author}</p>
        {#if modDetail.summary}
          <p class="detail-summary">{modDetail.summary}</p>
        {/if}
      </div>

      <!-- Stats Bar -->
      <div class="detail-stats-bar">
        <div class="detail-stats-left">
          <div class="detail-stat">
            <span class="detail-stat-value">{formatDownloads(modDetail.endorsement_count)}</span>
            <span class="detail-stat-label">Endorsements</span>
          </div>
          <div class="detail-stat">
            <span class="detail-stat-value">{formatDownloads(modDetail.unique_downloads)}</span>
            <span class="detail-stat-label">Downloads</span>
          </div>
          <div class="detail-stat">
            <span class="detail-stat-value">v{modDetail.version}</span>
            <span class="detail-stat-label">Version</span>
          </div>
          {#if modDetail.updated_at}
            <div class="detail-stat" title={absoluteDate(modDetail.updated_at)}>
              <span class="detail-stat-value">{relativeTime(modDetail.updated_at)}</span>
              <span class="detail-stat-label">Updated</span>
            </div>
          {/if}
        </div>
        {#if account?.is_premium && !installedNexusIds.has(modDetail.mod_id)}
          <button
            class="btn stats-install-btn"
            onclick={() => { if (modDetail) oninstall(modDetail); }}
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
              <polyline points="7 10 12 15 17 10" />
              <line x1="12" y1="15" x2="12" y2="3" />
            </svg>
            Install
          </button>
        {:else if installedNexusIds.has(modDetail.mod_id)}
          <span class="badge badge-success">Installed</span>
        {/if}
      </div>

      <!-- Description -->
      {#if renderedDescription}
        <div class="detail-section">
          <h3 class="detail-section-title">Description</h3>
          <div class="rendered-markdown" onclick={handleRenderedLinkClick}>
            {@html renderedDescription}
          </div>
        </div>
      {/if}

      <!-- Required mods (forward dependencies) -->
      {#if requirementsLoading}
        <div class="detail-section">
          <h3 class="detail-section-title">Required mods</h3>
          <div class="req-skeleton">
            <div class="req-skel-row"></div>
            <div class="req-skel-row"></div>
          </div>
        </div>
      {:else if requirements && requirements.requires.length > 0}
        <div class="detail-section">
          <h3 class="detail-section-title">
            Required mods
            <span class="title-count">{requirements.requires.length}</span>
          </h3>
          <div class="req-list">
            {#each requirements.requires as ref (ref.mod_id)}
              <button
                type="button"
                class="req-card"
                onclick={() => openModRef(ref)}
                title="Open on NexusMods: {ref.name}"
              >
                <span class="req-name">{ref.name}</span>
                {#if ref.author}
                  <span class="req-author">by {ref.author}</span>
                {/if}
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="req-icon">
                  <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
                  <polyline points="15 3 21 3 21 9" />
                  <line x1="10" y1="14" x2="21" y2="3" />
                </svg>
              </button>
            {/each}
          </div>
        </div>
      {/if}

      <!-- Mods that require this -->
      {#if !requirementsLoading && requirements && requirements.required_by.length > 0}
        <div class="detail-section">
          <h3 class="detail-section-title">
            Mods that require this
            <span class="title-count">{requirements.required_by.length}</span>
          </h3>
          <div class="req-list">
            {#each requirements.required_by as ref (ref.mod_id)}
              <button
                type="button"
                class="req-card"
                onclick={() => openModRef(ref)}
                title="Open on NexusMods: {ref.name}"
              >
                <span class="req-name">{ref.name}</span>
                {#if ref.author}
                  <span class="req-author">by {ref.author}</span>
                {/if}
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="req-icon">
                  <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
                  <polyline points="15 3 21 3 21 9" />
                  <line x1="10" y1="14" x2="21" y2="3" />
                </svg>
              </button>
            {/each}
          </div>
        </div>
      {/if}

      <!-- Files Table (premium only) -->
      {#if account?.is_premium && modFiles.length > 0}
        <div class="detail-section">
          <h3 class="detail-section-title">
            Files
            <span class="title-count">{modFiles.length}</span>
          </h3>
          <div class="mods-table-container">
            <div class="mods-table">
              <div class="mods-table-header">
                <span class="col-name">Name</span>
                <span class="col-version">Version</span>
                <span class="col-size">Size</span>
                <span class="col-category">Category</span>
                <span class="col-actions">Actions</span>
              </div>
              {#each modFiles as file}
                <div class="mods-table-row">
                  <span class="col-name" title={file.name}>{file.name}</span>
                  <span class="col-version">{file.version}</span>
                  <span class="col-size">{formatFileSize(file.size_kb)}</span>
                  <span class="col-category"><span class="tag">{file.category}</span></span>
                  <span class="col-actions">
                    <button
                      class="btn btn-accent btn-sm"
                      onclick={() => { if (modDetail) ondownloadfile(modDetail, file); }}
                      disabled={downloadingFileId === file.file_id}
                    >
                      {#if downloadingFileId === file.file_id}
                        <div class="spinner-sm-ring"></div>
                      {:else}
                        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                          <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                          <polyline points="7 10 12 15 17 10" />
                          <line x1="12" y1="15" x2="12" y2="3" />
                        </svg>
                        Install
                      {/if}
                    </button>
                  </span>
                </div>
              {/each}
            </div>
          </div>
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .detail-view {
    animation: glass-fade-in var(--duration-slow) var(--ease-out);
  }

  .detail-header {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-bottom: var(--space-4);
  }

  /* --- Button overrides (base .btn / variants from global app.css) --- */

  .detail-header .btn-ghost {
    background: var(--surface);
    border: 1px solid var(--separator);
  }

  .detail-header .btn-ghost:hover {
    border-color: var(--text-quaternary);
  }

  .stats-install-btn {
    background: var(--accent);
    color: #fff;
    padding: var(--space-2) var(--space-5);
  }

  .stats-install-btn:hover:not(:disabled) {
    filter: brightness(1.1);
    box-shadow: 0 1px 6px rgba(232, 128, 42, 0.3);
  }

  /* --- Hero Image --- */

  .mod-detail-hero {
    width: 100%;
    height: 280px;
    background-size: cover;
    background-position: center;
    border-radius: var(--radius-lg);
    border: 1px solid var(--separator);
    margin-bottom: var(--space-4);
  }

  /* --- Title Section --- */

  .detail-title-section {
    margin-bottom: var(--space-4);
  }

  .detail-name {
    font-size: 22px;
    font-weight: 700;
    color: var(--text-primary);
    margin: 0 0 var(--space-1) 0;
    letter-spacing: -0.02em;
  }

  .detail-author {
    font-size: 13px;
    color: var(--text-secondary);
    margin: 0 0 var(--space-2) 0;
  }

  .detail-summary {
    font-size: 13px;
    color: var(--text-tertiary);
    line-height: 1.5;
    margin: 0;
  }

  /* --- Stats Bar --- */

  .detail-stats-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-3) 0;
    border-top: 1px solid var(--separator);
    border-bottom: 1px solid var(--separator);
    margin-bottom: var(--space-5);
  }

  .detail-stats-left {
    display: flex;
    gap: var(--space-5);
  }

  .detail-stat {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .detail-stat-value {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .detail-stat-label {
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .stats-install-btn {
    flex-shrink: 0;
  }

  /* .badge / .badge-success inherited from global app.css */

  /* --- Sections --- */

  .detail-section {
    margin-bottom: var(--space-6);
  }

  .detail-section-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
    margin: 0 0 var(--space-3) 0;
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .title-count {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-tertiary);
    background: var(--surface);
    padding: 1px 6px;
    border-radius: var(--radius-sm);
  }

  /* --- Files Table --- */

  .mods-table-container {
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    overflow: hidden;
  }

  .mods-table-header {
    display: grid;
    grid-template-columns: 1fr 80px 80px 100px 80px;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: var(--surface);
    font-size: 11px;
    font-weight: 600;
    color: var(--text-tertiary);
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }

  .mods-table-row {
    display: grid;
    grid-template-columns: 1fr 80px 80px 100px 80px;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-top: 1px solid var(--separator);
    font-size: 12px;
    color: var(--text-secondary);
    align-items: center;
    transition: background var(--duration-fast) var(--ease);
  }

  .mods-table-row:hover {
    background: var(--surface-hover);
  }

  .col-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-primary);
    font-weight: 500;
  }

  .tag {
    display: inline-block;
    padding: 1px 6px;
    border-radius: var(--radius-sm);
    font-size: 11px;
    font-weight: 500;
    background: var(--surface);
    color: var(--text-secondary);
  }

  .spinner-sm-ring {
    width: 12px;
    height: 12px;
    border: 2px solid rgba(255, 255, 255, 0.3);
    border-top-color: #fff;
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  /* --- Loading State --- */

  .loading-container {
    display: flex;
    justify-content: center;
    padding: var(--space-12) 0;
  }

  .loading-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
  }

  .spinner {
    width: 32px;
    height: 32px;
  }

  .spinner-ring {
    width: 100%;
    height: 100%;
    border: 3px solid var(--separator);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  .loading-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-primary);
    margin: 0;
  }

  .loading-detail {
    font-size: 12px;
    color: var(--text-tertiary);
    margin: 0;
  }

  /* --- Requirements (forward + reverse) --- */

  .req-list {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: var(--space-2);
  }

  .req-card {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    text-align: left;
    cursor: pointer;
    transition: border-color var(--duration-fast) var(--ease),
                background var(--duration-fast) var(--ease);
    font-family: inherit;
  }

  .req-card:hover {
    border-color: var(--accent);
    background: var(--surface-hover);
  }

  .req-name {
    flex: 1;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .req-author {
    font-size: 11px;
    color: var(--text-tertiary);
    flex-shrink: 0;
  }

  .req-icon {
    flex-shrink: 0;
    color: var(--text-tertiary);
  }

  .req-skeleton {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .req-skel-row {
    height: 36px;
    background: var(--surface);
    border-radius: var(--radius);
    animation: skel-pulse 1.2s ease-in-out infinite;
  }

  @keyframes skel-pulse {
    0%, 100% { opacity: 0.6; }
    50% { opacity: 1; }
  }
</style>
