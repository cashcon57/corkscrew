<script lang="ts">
  import { onMount } from "svelte";
  import { getWabbajackModlists } from "$lib/api";
  import { showError } from "$lib/stores";
  import type { ModlistSummary } from "$lib/types";

  let modlists = $state<ModlistSummary[]>([]);
  let filtered = $state<ModlistSummary[]>([]);
  let loading = $state(true);
  let searchQuery = $state("");
  let gameFilter = $state("all");
  let showNsfw = $state(false);

  // Derived unique games from the modlists
  const gameOptions = $derived(() => {
    const games = new Set(modlists.map((m) => m.game));
    return Array.from(games).sort();
  });

  $effect(() => {
    let result = modlists;

    // Filter by NSFW
    if (!showNsfw) {
      result = result.filter((m) => !m.nsfw);
    }

    // Filter by game
    if (gameFilter !== "all") {
      result = result.filter((m) => m.game === gameFilter);
    }

    // Filter by search
    if (searchQuery.trim()) {
      const q = searchQuery.toLowerCase();
      result = result.filter(
        (m) =>
          m.title.toLowerCase().includes(q) ||
          m.author.toLowerCase().includes(q) ||
          m.description.toLowerCase().includes(q) ||
          m.tags.some((t) => t.toLowerCase().includes(q))
      );
    }

    filtered = result;
  });

  onMount(async () => {
    try {
      modlists = await getWabbajackModlists();
    } catch (e: any) {
      showError(`Failed to load modlists: ${e}`);
    } finally {
      loading = false;
    }
  });

  function formatSize(bytes: number): string {
    if (bytes === 0) return "N/A";
    const gb = bytes / (1024 * 1024 * 1024);
    if (gb >= 1) return `${gb.toFixed(1)} GB`;
    const mb = bytes / (1024 * 1024);
    return `${mb.toFixed(0)} MB`;
  }

  function gameDomainDisplay(domain: string): string {
    const map: Record<string, string> = {
      skyrim: "Skyrim LE",
      skyrimspecialedition: "Skyrim SE",
      skyrimvr: "Skyrim VR",
      fallout4: "Fallout 4",
      fallout4vr: "Fallout 4 VR",
      falloutnewvegas: "Fallout NV",
      fallout3: "Fallout 3",
      oblivion: "Oblivion",
      morrowind: "Morrowind",
      enderal: "Enderal",
      enderalspecialedition: "Enderal SE",
      cyberpunk2077: "Cyberpunk 2077",
      stardewvalley: "Stardew Valley",
      witcher3: "Witcher 3",
      thewitcher3: "Witcher 3",
      starfield: "Starfield",
      baldursgate3: "BG3",
    };
    return map[domain] || domain;
  }

  function openUrl(url: string) {
    if (url) {
      window.open(url, "_blank");
    }
  }
</script>

<div class="modlists-page">
  <header class="page-header">
    <div class="header-text">
      <h2 class="page-title">Modlists</h2>
      <p class="page-subtitle">
        Browse Wabbajack modlists — curated, pre-configured mod setups
      </p>
    </div>
    {#if !loading}
      <div class="header-stats">
        <div class="stat-pill">
          <span class="stat-value">{filtered.length}</span>
          <span class="stat-label">{filtered.length === 1 ? "List" : "Lists"}</span>
        </div>
      </div>
    {/if}
  </header>

  {#if loading}
    <div class="loading-container">
      <div class="loading-card">
        <div class="spinner"><div class="spinner-ring"></div></div>
        <div class="loading-text">
          <p class="loading-title">Fetching modlists</p>
          <p class="loading-detail">Loading gallery from Wabbajack repositories...</p>
        </div>
      </div>
    </div>
  {:else}
    <!-- Filters -->
    <div class="filters-bar">
      <input
        type="text"
        class="search-input"
        placeholder="Search modlists..."
        bind:value={searchQuery}
      />
      <select class="filter-select" bind:value={gameFilter}>
        <option value="all">All Games</option>
        {#each gameOptions() as game}
          <option value={game}>{gameDomainDisplay(game)}</option>
        {/each}
      </select>
      <label class="nsfw-toggle">
        <input type="checkbox" bind:checked={showNsfw} />
        <span>NSFW</span>
      </label>
    </div>

    {#if filtered.length === 0}
      <div class="empty-state">
        <p class="empty-title">No modlists found</p>
        <p class="empty-detail">
          {#if searchQuery || gameFilter !== "all"}
            Try adjusting your search or filters.
          {:else}
            No modlists are currently available.
          {/if}
        </p>
      </div>
    {:else}
      <div class="modlist-grid">
        {#each filtered as modlist, i}
          <div class="modlist-card" style="animation-delay: {Math.min(i, 20) * 30}ms">
            {#if modlist.image_url}
              <div class="card-image">
                <img src={modlist.image_url} alt={modlist.title} loading="lazy" />
              </div>
            {/if}
            <div class="card-body">
              <div class="card-top">
                <span class="game-badge">{gameDomainDisplay(modlist.game)}</span>
                {#if modlist.nsfw}
                  <span class="nsfw-badge">NSFW</span>
                {/if}
                <span class="version-badge">v{modlist.version}</span>
              </div>

              <h3 class="card-title">{modlist.title}</h3>
              <p class="card-author">by {modlist.author}</p>

              {#if modlist.description}
                <p class="card-desc">{modlist.description}</p>
              {/if}

              {#if modlist.tags.length > 0}
                <div class="card-tags">
                  {#each modlist.tags.slice(0, 5) as tag}
                    <span class="tag">{tag}</span>
                  {/each}
                </div>
              {/if}

              <div class="card-stats">
                <div class="stat-item">
                  <span class="stat-num">{formatSize(modlist.download_size)}</span>
                  <span class="stat-lbl">Download</span>
                </div>
                <div class="stat-item">
                  <span class="stat-num">{formatSize(modlist.install_size)}</span>
                  <span class="stat-lbl">Install</span>
                </div>
                <div class="stat-item">
                  <span class="stat-num">{modlist.archive_count.toLocaleString()}</span>
                  <span class="stat-lbl">Archives</span>
                </div>
                <div class="stat-item">
                  <span class="stat-num">{modlist.file_count.toLocaleString()}</span>
                  <span class="stat-lbl">Files</span>
                </div>
              </div>

              <div class="card-actions">
                {#if modlist.readme_url}
                  <button class="btn btn-secondary btn-sm" onclick={() => openUrl(modlist.readme_url)}>
                    <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                      <rect x="3" y="1.5" width="10" height="13" rx="1.5" />
                      <line x1="5.5" y1="4.5" x2="10.5" y2="4.5" />
                      <line x1="5.5" y1="7" x2="10.5" y2="7" />
                      <line x1="5.5" y1="9.5" x2="8.5" y2="9.5" />
                    </svg>
                    Readme
                  </button>
                {/if}
                <button
                  class="btn btn-accent btn-sm"
                  disabled
                  title="Modlist installation coming soon"
                >
                  Install (Coming Soon)
                </button>
              </div>
            </div>
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .modlists-page {
    max-width: 1040px;
    padding: var(--space-2) 0 var(--space-12) 0;
  }

  .page-header {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    margin-bottom: var(--space-8);
    padding-bottom: var(--space-4);
    border-bottom: 1px solid var(--separator);
  }

  .page-title {
    font-size: 28px;
    font-weight: 700;
    color: var(--text-primary);
    letter-spacing: -0.025em;
  }

  .page-subtitle {
    font-size: 14px;
    color: var(--text-secondary);
    margin-top: var(--space-1);
  }

  .header-stats {
    display: flex;
    gap: var(--space-3);
  }

  .stat-pill {
    display: flex;
    align-items: baseline;
    gap: var(--space-1);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    padding: var(--space-2) var(--space-4);
  }

  .stat-value {
    font-size: 18px;
    font-weight: 700;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .stat-label {
    font-size: 12px;
    color: var(--text-tertiary);
    font-weight: 500;
  }

  /* Loading */
  .loading-container {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 280px;
  }

  .loading-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-6);
    padding: var(--space-12) var(--space-10);
  }

  .spinner { width: 36px; height: 36px; }

  .spinner-ring {
    width: 100%;
    height: 100%;
    border: 2.5px solid var(--separator);
    border-top-color: var(--system-accent);
    border-radius: 50%;
    animation: spin 0.9s cubic-bezier(0.4, 0, 0.2, 1) infinite;
  }

  @keyframes spin { to { transform: rotate(360deg); } }

  .loading-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
    text-align: center;
  }

  .loading-detail {
    font-size: 13px;
    color: var(--text-tertiary);
    text-align: center;
    margin-top: var(--space-1);
  }

  /* Filters */
  .filters-bar {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-bottom: var(--space-6);
  }

  .search-input {
    flex: 1;
    padding: var(--space-2) var(--space-3);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    color: var(--text-primary);
    font-size: 14px;
    outline: none;
    transition: border-color var(--duration-fast) var(--ease);
  }

  .search-input:focus {
    border-color: var(--system-accent);
  }

  .search-input::placeholder {
    color: var(--text-tertiary);
  }

  .filter-select {
    padding: var(--space-2) var(--space-3);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    color: var(--text-primary);
    font-size: 13px;
    outline: none;
    cursor: pointer;
    min-width: 140px;
  }

  .filter-select:focus {
    border-color: var(--system-accent);
  }

  .nsfw-toggle {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: 13px;
    color: var(--text-secondary);
    cursor: pointer;
    white-space: nowrap;
  }

  .nsfw-toggle input {
    accent-color: var(--system-accent);
  }

  /* Grid */
  .modlist-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
    gap: var(--space-4);
  }

  /* Cards */
  .modlist-card {
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-lg);
    overflow: hidden;
    transition: border-color var(--duration-fast) var(--ease),
                box-shadow var(--duration-fast) var(--ease);
    animation: cardFadeIn var(--duration-slow) var(--ease) both;
  }

  .modlist-card:hover {
    border-color: rgba(255, 255, 255, 0.12);
  }

  @keyframes cardFadeIn {
    from { opacity: 0; transform: translateY(6px); }
    to { opacity: 1; transform: translateY(0); }
  }

  .card-image {
    width: 100%;
    height: 140px;
    overflow: hidden;
    background: var(--bg-secondary);
  }

  .card-image img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .card-body {
    padding: var(--space-4) var(--space-5);
  }

  .card-top {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-2);
    flex-wrap: wrap;
  }

  .game-badge {
    font-size: 11px;
    font-weight: 600;
    color: var(--accent);
    background: var(--accent-subtle);
    padding: 1px var(--space-2);
    border-radius: var(--radius-sm);
  }

  .nsfw-badge {
    font-size: 10px;
    font-weight: 700;
    color: var(--red);
    background: var(--red-subtle);
    padding: 1px var(--space-2);
    border-radius: var(--radius-sm);
  }

  .version-badge {
    font-size: 10px;
    font-weight: 500;
    color: var(--text-tertiary);
    background: rgba(255, 255, 255, 0.05);
    padding: 1px var(--space-2);
    border-radius: var(--radius-sm);
  }

  .card-title {
    font-size: 15px;
    font-weight: 700;
    color: var(--text-primary);
    line-height: 1.3;
    margin-bottom: 2px;
  }

  .card-author {
    font-size: 12px;
    color: var(--text-secondary);
    margin-bottom: var(--space-2);
  }

  .card-desc {
    font-size: 12px;
    color: var(--text-tertiary);
    line-height: 1.5;
    margin-bottom: var(--space-3);
    display: -webkit-box;
    -webkit-line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .card-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-bottom: var(--space-3);
  }

  .tag {
    font-size: 10px;
    font-weight: 500;
    color: var(--text-secondary);
    background: rgba(255, 255, 255, 0.06);
    padding: 1px 6px;
    border-radius: var(--radius-sm);
  }

  .card-stats {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: var(--space-2);
    padding: var(--space-3) 0;
    border-top: 1px solid var(--separator);
    margin-bottom: var(--space-3);
  }

  .stat-item {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
  }

  .stat-num {
    font-size: 12px;
    font-weight: 700;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .stat-lbl {
    font-size: 10px;
    color: var(--text-tertiary);
    font-weight: 500;
  }

  .card-actions {
    display: flex;
    gap: var(--space-2);
  }

  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-1);
    border-radius: var(--radius);
    font-weight: 600;
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
  }

  .btn-sm {
    padding: var(--space-1) var(--space-3);
    font-size: 12px;
    min-height: 28px;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-accent {
    background: var(--system-accent);
    color: #fff;
    flex: 1;
  }

  .btn-accent:hover:not(:disabled) {
    filter: brightness(1.1);
  }

  .btn-secondary {
    background: var(--surface-hover);
    color: var(--text-primary);
    border: 1px solid var(--separator);
  }

  .btn-secondary:hover:not(:disabled) {
    background: var(--surface-active);
  }

  /* Empty state */
  .empty-state {
    text-align: center;
    padding: var(--space-12) var(--space-8);
    border: 1px dashed rgba(255, 255, 255, 0.1);
    border-radius: var(--radius-lg);
    background: rgba(255, 255, 255, 0.015);
  }

  .empty-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-secondary);
    margin-bottom: var(--space-2);
  }

  .empty-detail {
    font-size: 13px;
    color: var(--text-tertiary);
    max-width: 360px;
    margin: 0 auto;
    line-height: 1.55;
  }
</style>
