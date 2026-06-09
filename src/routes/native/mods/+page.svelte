<script lang="ts">
  import { onMount } from "svelte";
  import { selectedGame, showError, showSuccess } from "$lib/stores";
  import {
    getInstalledMods,
    toggleMod,
    uninstallMod,
    installMod,
    analyzeConflicts,
  } from "$lib/api";
  import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
  import GameIcon from "$lib/components/GameIcon.svelte";
  import { wineCtx } from "$lib/types";
  import type {
    InstalledMod,
    DetectedGame,
    ConflictSuggestion,
  } from "$lib/types";

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  let mods = $state<InstalledMod[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let togglingMod = $state<number | null>(null);
  let confirmUninstall = $state<number | null>(null);

  // Search / sort
  let searchQuery = $state("");
  type SortKey = "name" | "version" | "files" | "date";
  let sortBy = $state<SortKey>("name");
  let sortDir = $state<"asc" | "desc">("asc");

  // Batch select
  let selectedModIds = $state(new Set<number>());

  // Drag-drop install state
  let draggingOver = $state(false);
  let installing = $state(false);

  // Conflict panel state
  let conflictsExpanded = $state(false);
  let conflictsLoading = $state(false);
  let conflictsError = $state<string | null>(null);
  let conflictSuggestions = $state<ConflictSuggestion[]>([]);

  // ---------------------------------------------------------------------------
  // Derived
  // ---------------------------------------------------------------------------

  /** Native games always use an empty bottle name sentinel. */
  function nativeBottleName(g: DetectedGame): string {
    return wineCtx(g)?.bottle_name ?? "";
  }

  let filteredMods = $derived.by(() => {
    const q = searchQuery.trim().toLowerCase();
    let list = q
      ? mods.filter(
          (m) =>
            m.name.toLowerCase().includes(q) ||
            (m.version ?? "").toLowerCase().includes(q) ||
            (m.auto_category ?? "").toLowerCase().includes(q) ||
            (m.source_type ?? "").toLowerCase().includes(q)
        )
      : [...mods];

    list.sort((a, b) => {
      let cmp = 0;
      switch (sortBy) {
        case "name":
          cmp = a.name.localeCompare(b.name);
          break;
        case "version":
          cmp = (a.version ?? "").localeCompare(b.version ?? "");
          break;
        case "files":
          cmp = (a.file_count ?? 0) - (b.file_count ?? 0);
          break;
        case "date":
          cmp = (a.installed_at ?? "").localeCompare(b.installed_at ?? "");
          break;
      }
      return sortDir === "asc" ? cmp : -cmp;
    });
    return list;
  });

  let enabledCount = $derived(mods.filter((m) => m.enabled).length);
  let selectAll = $derived(
    filteredMods.length > 0 && filteredMods.every((m) => selectedModIds.has(m.id))
  );

  // ---------------------------------------------------------------------------
  // Data loading
  // ---------------------------------------------------------------------------

  async function loadMods() {
    const g = $selectedGame;
    if (!g) {
      mods = [];
      return;
    }
    loading = true;
    error = null;
    try {
      mods = await getInstalledMods(g.game_id, nativeBottleName(g));
    } catch (e) {
      console.error("getInstalledMods failed:", e);
      error = String(e);
      mods = [];
    } finally {
      loading = false;
    }
  }

  onMount(loadMods);

  $effect(() => {
    const _ = $selectedGame?.game_id;
    loadMods();
  });

  // ---------------------------------------------------------------------------
  // Actions
  // ---------------------------------------------------------------------------

  function toggleSort(key: SortKey) {
    if (sortBy === key) {
      sortDir = sortDir === "asc" ? "desc" : "asc";
    } else {
      sortBy = key;
      sortDir = "asc";
    }
  }

  function toggleSelectAll() {
    if (selectAll) {
      selectedModIds = new Set();
    } else {
      selectedModIds = new Set(filteredMods.map((m) => m.id));
    }
  }

  function toggleSelectMod(id: number) {
    const next = new Set(selectedModIds);
    if (next.has(id)) {
      next.delete(id);
    } else {
      next.add(id);
    }
    selectedModIds = next;
  }

  async function handleToggle(m: InstalledMod) {
    const g = $selectedGame;
    if (!g) return;
    togglingMod = m.id;
    try {
      await toggleMod(m.id, g.game_id, nativeBottleName(g), !m.enabled);
      await loadMods();
    } catch (e) {
      console.error("toggleMod failed:", e);
      error = String(e);
    } finally {
      togglingMod = null;
    }
  }

  async function handleUninstall(id: number) {
    const g = $selectedGame;
    if (!g) return;
    confirmUninstall = null;
    try {
      await uninstallMod(id, g.game_id, nativeBottleName(g));
      selectedModIds.delete(id);
      selectedModIds = new Set(selectedModIds);
      await loadMods();
    } catch (e) {
      console.error("uninstallMod failed:", e);
      error = String(e);
    }
  }

  async function batchEnable() {
    const g = $selectedGame;
    if (!g) return;
    for (const id of selectedModIds) {
      const m = mods.find((x) => x.id === id);
      if (m && !m.enabled) {
        await toggleMod(id, g.game_id, nativeBottleName(g), true).catch((e) =>
          console.error("batchEnable toggleMod failed:", e)
        );
      }
    }
    selectedModIds = new Set();
    await loadMods();
  }

  async function batchDisable() {
    const g = $selectedGame;
    if (!g) return;
    for (const id of selectedModIds) {
      const m = mods.find((x) => x.id === id);
      if (m && m.enabled) {
        await toggleMod(id, g.game_id, nativeBottleName(g), false).catch((e) =>
          console.error("batchDisable toggleMod failed:", e)
        );
      }
    }
    selectedModIds = new Set();
    await loadMods();
  }

  async function batchUninstall() {
    const g = $selectedGame;
    if (!g) return;
    if (
      !confirm(
        `Delete ${selectedModIds.size} mod${selectedModIds.size === 1 ? "" : "s"}? This removes them from disk and your install records.`
      )
    )
      return;
    for (const id of [...selectedModIds]) {
      await uninstallMod(id, g.game_id, nativeBottleName(g)).catch((e) =>
        console.error("batchUninstall uninstallMod failed:", e)
      );
    }
    selectedModIds = new Set();
    await loadMods();
  }

  function openModsFolder() {
    const g = $selectedGame;
    if (!g) return;
    revealItemInDir(g.data_dir).catch((e) =>
      console.error("revealItemInDir failed:", e)
    );
  }

  // ---------------------------------------------------------------------------
  // Drag-and-drop install
  // ---------------------------------------------------------------------------

  function handleDragOver(e: DragEvent) {
    e.preventDefault();
    if (installing) return;
    draggingOver = true;
  }

  function handleDragLeave() {
    draggingOver = false;
  }

  async function handleDrop(e: DragEvent) {
    e.preventDefault();
    draggingOver = false;
    if (installing) return;
    const g = $selectedGame;
    if (!g || !e.dataTransfer?.files?.length) return;

    const file = e.dataTransfer.files[0];
    const ext = file.name.split(".").pop()?.toLowerCase();
    // Native mode: drop Bethesda .ba2/.bsa; only loose archives accepted
    if (!ext || !["zip", "7z", "rar"].includes(ext)) {
      showError("Unsupported file type. Use .zip, .7z, or .rar archives.");
      return;
    }

    const filePath = (file as File & { path?: string }).path;
    if (!filePath) {
      showError("Could not read file path from drop event.");
      return;
    }

    installing = true;
    try {
      const installed = await installMod(filePath, g.game_id, nativeBottleName(g));
      showSuccess(`Installed "${installed.name}" successfully`);
      await loadMods();
    } catch (e) {
      console.error("installMod failed:", e);
      showError(`Install failed: ${e}`);
    } finally {
      installing = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Conflict panel
  // ---------------------------------------------------------------------------

  async function loadConflicts() {
    const g = $selectedGame;
    if (!g) return;
    conflictsLoading = true;
    conflictsError = null;
    try {
      const resp = await analyzeConflicts(g.game_id, nativeBottleName(g));
      conflictSuggestions = resp.suggestions;
    } catch (e) {
      console.error("analyzeConflicts failed:", e);
      conflictsError = String(e);
      conflictSuggestions = [];
    } finally {
      conflictsLoading = false;
    }
  }

  function toggleConflictsPanel() {
    conflictsExpanded = !conflictsExpanded;
    if (conflictsExpanded && conflictSuggestions.length === 0 && !conflictsLoading) {
      void loadConflicts();
    }
  }

  function loserNames(s: ConflictSuggestion): string {
    return s.mods
      .filter((m) => m.mod_id !== s.suggested_winner_id)
      .map((m) => m.mod_name)
      .join(", ");
  }

  function formatDate(iso: string | null | undefined): string {
    if (!iso) return "—";
    try {
      return new Date(iso).toLocaleDateString(undefined, {
        year: "numeric",
        month: "short",
        day: "numeric",
      });
    } catch {
      return iso;
    }
  }

  function sourceLabel(src: string | null | undefined): string {
    switch (src) {
      case "nexus": return "Nexus";
      case "loverslab": return "LoversLab";
      case "moddb": return "ModDB";
      case "curseforge": return "CurseForge";
      case "direct": return "Direct";
      default: return "Manual";
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="mods-page"
  class:drag-active={draggingOver}
  ondragover={handleDragOver}
  ondragleave={handleDragLeave}
  ondrop={handleDrop}
  role="application"
>
  {#if draggingOver}
    <div class="drop-overlay">
      <div class="drop-overlay-content">
        <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
          <polyline points="7 10 12 15 17 10" />
          <line x1="12" y1="15" x2="12" y2="3" />
        </svg>
        <p>Drop mod archive to install</p>
      </div>
    </div>
  {/if}
  {#if installing}
    <div class="drop-overlay">
      <div class="drop-overlay-content">
        <span class="spinner"></span>
        <p>Installing...</p>
      </div>
    </div>
  {/if}
  {#if !$selectedGame}
    <!-- No game selected -->
    <div class="empty-state">
      <div class="empty-icon">
        <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <rect x="2" y="3" width="20" height="14" rx="2" ry="2" />
          <line x1="8" y1="21" x2="16" y2="21" />
          <line x1="12" y1="17" x2="12" y2="21" />
        </svg>
      </div>
      <h3 class="empty-title">No game selected</h3>
      <p class="empty-description">Pick a native game from the dropdown in the top bar.</p>
    </div>
  {:else}
    <!-- ======================================================
         Game Banner Header
         ====================================================== -->
    <div class="game-banner">
      <div class="game-banner-icon">
        <GameIcon gameId={$selectedGame.game_id} steamAppId={$selectedGame.steam_app_id} size={36} />
      </div>
      <div class="game-banner-info">
        <h2 class="game-banner-title">{$selectedGame.display_name}</h2>
        <div class="game-banner-meta">
          <span class="meta-native-badge">Native</span>
          {#if mods.length > 0}
            <span class="meta-separator">&middot;</span>
            <span class="meta-mods">{enabledCount}/{mods.length} mods active</span>
          {/if}
        </div>
      </div>
      <div class="game-banner-actions">
        <button class="btn btn-ghost" onclick={loadMods} title="Refresh mod list">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="23 4 23 10 17 10" />
            <path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10" />
          </svg>
          Refresh
        </button>
        <button class="btn btn-ghost" onclick={openModsFolder} title="Open mods folder in Finder">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
          </svg>
          Open Folder
        </button>
        {#if $selectedGame.nexus_slug}
          <a
            href="https://www.nexusmods.com/{$selectedGame.nexus_slug}"
            target="_blank"
            rel="noopener noreferrer"
            class="btn btn-ghost nexus-link"
            title="View on Nexus Mods"
          >
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
              <path d="M11 8v3a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h3" />
              <path d="M8 2h4v4" />
              <path d="M6 8L12 2" />
            </svg>
            Nexus
          </a>
        {/if}
      </div>
    </div>

    <!-- ======================================================
         Bulk Action Bar (shown when rows selected)
         ====================================================== -->
    {#if selectedModIds.size > 0}
      <div class="bulk-action-bar">
        <span class="bulk-count">{selectedModIds.size} selected</span>
        <button class="btn btn-sm btn-secondary" onclick={batchEnable}>Enable All</button>
        <button class="btn btn-sm btn-secondary" onclick={batchDisable}>Disable All</button>
        <button class="btn btn-sm btn-ghost-danger" onclick={batchUninstall}>Uninstall</button>
        <button class="btn btn-sm btn-ghost" onclick={() => (selectedModIds = new Set())}>Clear</button>
      </div>
    {/if}

    <!-- ======================================================
         Search & Filter Bar
         ====================================================== -->
    {#if mods.length > 0}
      <div class="filter-bar">
        <div class="search-box">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="11" cy="11" r="8" />
            <line x1="21" y1="21" x2="16.65" y2="16.65" />
          </svg>
          <input
            type="text"
            placeholder="Search mods..."
            bind:value={searchQuery}
            class="search-input"
          />
          {#if searchQuery}
            <button class="search-clear" onclick={() => (searchQuery = "")}>
              <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
                <line x1="3" y1="3" x2="9" y2="9" />
                <line x1="9" y1="3" x2="3" y2="9" />
              </svg>
            </button>
          {/if}
        </div>
        {#if searchQuery}
          <span class="filter-count">{filteredMods.length} of {mods.length}</span>
        {/if}
      </div>
    {/if}

    <!-- ======================================================
         Conflicts Panel (collapsible)
         ====================================================== -->
    {#if mods.length > 0}
      <div class="conflicts-panel" class:conflicts-expanded={conflictsExpanded}>
        <button
          type="button"
          class="conflicts-toggle"
          onclick={toggleConflictsPanel}
          aria-expanded={conflictsExpanded}
        >
          <svg
            class="conflicts-chevron"
            class:conflicts-chevron-open={conflictsExpanded}
            width="12"
            height="12"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2.5"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <polyline points="9 18 15 12 9 6" />
          </svg>
          <span class="conflicts-title">Conflicts</span>
          {#if conflictSuggestions.length > 0}
            <span class="conflicts-count">{conflictSuggestions.length}</span>
          {/if}
          {#if conflictsExpanded}
            <span class="conflicts-spacer"></span>
            <button
              type="button"
              class="btn btn-ghost btn-sm"
              onclick={(e) => { e.stopPropagation(); void loadConflicts(); }}
              title="Re-scan conflicts"
              disabled={conflictsLoading}
            >Rescan</button>
          {/if}
        </button>

        {#if conflictsExpanded}
          <div class="conflicts-body">
            {#if conflictsLoading}
              <div class="conflicts-empty">
                <span class="spinner"></span>
                <span>Analyzing conflicts...</span>
              </div>
            {:else if conflictsError}
              <div class="conflicts-empty conflicts-error">
                Failed to analyze: {conflictsError}
              </div>
            {:else if conflictSuggestions.length === 0}
              <div class="conflicts-empty">No file conflicts detected.</div>
            {:else}
              <div class="conflicts-list">
                {#each conflictSuggestions as s (s.relative_path)}
                  <div class="conflict-row">
                    <div class="conflict-path" title={s.relative_path}>{s.relative_path}</div>
                    <div class="conflict-meta">
                      <span class="conflict-label">Winner:</span>
                      <span class="conflict-winner">{s.suggested_winner_name}</span>
                      {#if loserNames(s)}
                        <span class="conflict-label">Losers:</span>
                        <span class="conflict-losers" title={loserNames(s)}>{loserNames(s)}</span>
                      {/if}
                    </div>
                  </div>
                {/each}
              </div>
              <div class="conflicts-footer">
                <button class="btn btn-ghost btn-sm" disabled title="Coming soon">
                  Resolve in advanced view
                </button>
              </div>
            {/if}
          </div>
        {/if}
      </div>
    {/if}

    <!-- ======================================================
         Content Area
         ====================================================== -->
    {#if loading}
      <div class="empty-state">
        <div class="empty-icon"><span class="spinner"></span></div>
        <h3 class="empty-title">Loading mods...</h3>
      </div>
    {:else if error}
      <div class="empty-state">
        <div class="empty-icon">
          <svg width="36" height="36" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" style="color: var(--red)">
            <circle cx="12" cy="12" r="10" />
            <line x1="15" y1="9" x2="9" y2="15" />
            <line x1="9" y1="9" x2="15" y2="15" />
          </svg>
        </div>
        <h3 class="empty-title">Failed to load mods</h3>
        <p class="empty-description">{error}</p>
        <button class="btn btn-secondary btn-sm" onclick={loadMods}>Retry</button>
      </div>
    {:else if mods.length === 0}
      <div class="empty-state">
        <div class="empty-icon">
          <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z" />
            <polyline points="3.27 6.96 12 12.01 20.73 6.96" />
            <line x1="12" y1="22.08" x2="12" y2="12" />
          </svg>
        </div>
        <h3 class="empty-title">No mods installed</h3>
        <p class="empty-description">
          Drop mod archives into the mods folder, or browse NexusMods to find mods for {$selectedGame.display_name}.
        </p>
        <div class="empty-actions">
          <button class="btn empty-action-btn" onclick={openModsFolder}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
            </svg>
            Open Mods Folder
          </button>
          {#if $selectedGame.nexus_slug}
            <a
              href="https://www.nexusmods.com/{$selectedGame.nexus_slug}"
              target="_blank"
              rel="noopener noreferrer"
              class="btn empty-action-btn"
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <circle cx="11" cy="11" r="8" />
                <line x1="21" y1="21" x2="16.65" y2="16.65" />
              </svg>
              Browse NexusMods
            </a>
          {/if}
        </div>
      </div>
    {:else if filteredMods.length === 0}
      <div class="empty-state">
        <h3 class="empty-title">No mods match your search.</h3>
        <button class="btn btn-ghost btn-sm" onclick={() => (searchQuery = "")}>Clear Filter</button>
      </div>
    {:else}
      <!-- ==================================================
           Mod Table
           ================================================== -->
      <div class="mod-table-container">
        <div class="mod-table">
          <!-- Sticky Header -->
          <div class="table-header">
            <!-- Checkbox -->
            <span
              class="col-check"
              role="checkbox"
              aria-checked={selectAll}
              onclick={toggleSelectAll}
            >
              <span class="check-box" class:check-box-checked={selectAll}>
                {#if selectAll}
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round">
                    <polyline points="20 6 9 17 4 12" />
                  </svg>
                {/if}
              </span>
            </span>
            <!-- Toggle icon header -->
            <span class="col-toggle header-sep-right">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="opacity: 0.5" aria-hidden="true">
                <rect x="1" y="5" width="22" height="14" rx="7" ry="7" />
                <circle cx="16" cy="12" r="3" />
              </svg>
            </span>
            <button type="button" class="col-name sortable-header" onclick={() => toggleSort("name")}>
              Mod Name
              {#if sortBy === "name"}
                <span class="sort-arrow">{sortDir === "asc" ? "▲" : "▼"}</span>
              {/if}
            </button>
            <span class="col-category">Category</span>
            <span class="col-origin">Source</span>
            <button type="button" class="col-version sortable-header" onclick={() => toggleSort("version")}>
              Version
              {#if sortBy === "version"}
                <span class="sort-arrow">{sortDir === "asc" ? "▲" : "▼"}</span>
              {/if}
            </button>
            <button type="button" class="col-files sortable-header" onclick={() => toggleSort("files")}>
              Files
              {#if sortBy === "files"}
                <span class="sort-arrow">{sortDir === "asc" ? "▲" : "▼"}</span>
              {/if}
            </button>
            <button type="button" class="col-date sortable-header" onclick={() => toggleSort("date")}>
              Installed
              {#if sortBy === "date"}
                <span class="sort-arrow">{sortDir === "asc" ? "▲" : "▼"}</span>
              {/if}
            </button>
            <span class="col-actions">Actions</span>
          </div>

          <!-- Table Body -->
          <div class="table-body">
            {#each filteredMods as m (m.id)}
              <div
                class="table-row"
                class:row-disabled={!m.enabled}
                class:row-checked={selectedModIds.has(m.id)}
              >
                <!-- Checkbox -->
                <span
                  class="col-check"
                  role="checkbox"
                  aria-checked={selectedModIds.has(m.id)}
                  onclick={(e) => { e.stopPropagation(); toggleSelectMod(m.id); }}
                >
                  <span class="check-box" class:check-box-checked={selectedModIds.has(m.id)}>
                    {#if selectedModIds.has(m.id)}
                      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round">
                        <polyline points="20 6 9 17 4 12" />
                      </svg>
                    {/if}
                  </span>
                </span>

                <!-- Toggle switch -->
                <span class="col-toggle">
                  <button
                    class="toggle-switch"
                    class:toggle-on={m.enabled}
                    class:toggle-busy={togglingMod === m.id}
                    onclick={() => handleToggle(m)}
                    title={m.enabled ? "Disable mod" : "Enable mod"}
                    aria-label="{m.enabled ? 'Disable' : 'Enable'} {m.name}"
                    aria-pressed={m.enabled}
                    role="switch"
                  >
                    <span class="toggle-track">
                      <span class="toggle-thumb"></span>
                    </span>
                  </button>
                </span>

                <!-- Name -->
                <span class="col-name">
                  <span class="mod-name">{m.name}</span>
                </span>

                <!-- Category -->
                <span class="col-category">
                  {#if m.auto_category}
                    <span class="category-label">{m.auto_category}</span>
                  {:else}
                    <span class="text-muted">&mdash;</span>
                  {/if}
                </span>

                <!-- Source -->
                <span class="col-origin">
                  <span class="origin-label origin-{m.source_type ?? 'manual'}">{sourceLabel(m.source_type)}</span>
                  {#if m.source_type === "nexus" && m.nexus_mod_id}
                    <button
                      class="origin-link-btn"
                      title="Open on NexusMods"
                      onclick={(e) => {
                        e.stopPropagation();
                        if ($selectedGame?.nexus_slug && m.nexus_mod_id) {
                          openUrl(`https://www.nexusmods.com/${$selectedGame.nexus_slug}/mods/${m.nexus_mod_id}`).catch((err) =>
                            console.error("openUrl failed:", err)
                          );
                        }
                      }}
                    >
                      <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71" />
                        <path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" />
                      </svg>
                    </button>
                  {/if}
                </span>

                <!-- Version -->
                <span class="col-version">
                  <span class="version-text">{m.version || "—"}</span>
                </span>

                <!-- Files -->
                <span class="col-files">{m.file_count ?? 0}</span>

                <!-- Date -->
                <span class="col-date">{formatDate(m.installed_at)}</span>

                <!-- Actions -->
                <span class="col-actions">
                  {#if confirmUninstall === m.id}
                    <div class="confirm-actions">
                      <button
                        class="btn btn-danger btn-sm"
                        onclick={() => handleUninstall(m.id)}
                      >Yes</button>
                      <button
                        class="btn btn-ghost btn-sm"
                        onclick={() => (confirmUninstall = null)}
                      >No</button>
                    </div>
                  {:else}
                    <div class="mod-action-group">
                      <button
                        class="mod-uninstall-btn"
                        onclick={(e) => { e.stopPropagation(); confirmUninstall = m.id; }}
                        title="Uninstall mod"
                      >
                        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                          <polyline points="3 6 5 6 21 6" />
                          <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
                        </svg>
                      </button>
                    </div>
                  {/if}
                </span>
              </div>
            {/each}
          </div>
        </div>
      </div>

      <!-- ==================================================
           Status Footer
           ================================================== -->
      <div class="status-footer">
        <span>{mods.length} mod{mods.length === 1 ? "" : "s"}</span>
        <span class="meta-separator">&middot;</span>
        <span>{enabledCount} enabled</span>
        {#if searchQuery}
          <span class="meta-separator">&middot;</span>
          <span>{filteredMods.length} matching filter</span>
        {/if}
        {#if selectedModIds.size > 0}
          <span class="meta-separator">&middot;</span>
          <span class="footer-selected">{selectedModIds.size} selected</span>
        {/if}
      </div>
    {/if}
  {/if}
</div>

<style>
  /* ============================
     Page Layout — mirrors .mods-page
     ============================ */
  .mods-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: var(--space-4) var(--space-5);
    gap: var(--space-3);
    overflow: hidden;
    position: relative;
  }

  @media (max-width: 800px) {
    .mods-page {
      padding: var(--space-3) var(--space-3);
      gap: var(--space-2);
    }
  }

  /* ============================
     Game Banner Header
     ============================ */
  .game-banner {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    box-shadow: var(--glass-refraction), var(--glass-edge-shadow);
    flex-shrink: 0;
  }

  .game-banner-icon {
    flex-shrink: 0;
    color: var(--text-primary);
    display: flex;
    align-items: center;
    justify-content: center;
    width: 40px;
    height: 40px;
  }

  .game-banner-info {
    flex: 1;
    min-width: 0;
  }

  .game-banner-title {
    font-size: 16px;
    font-weight: 700;
    letter-spacing: -0.02em;
    color: var(--text-primary);
    line-height: 1.2;
  }

  .game-banner-meta {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: 2px;
    font-size: 13px;
  }

  .meta-native-badge {
    background: var(--green-subtle, rgba(48, 209, 88, 0.12));
    color: var(--green, #30d158);
    border: 1px solid rgba(48, 209, 88, 0.25);
    padding: 1px 7px;
    border-radius: 100px;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .meta-separator {
    color: var(--text-quaternary);
  }

  .meta-mods {
    color: var(--text-secondary);
    font-weight: 500;
  }

  .game-banner-actions {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-2);
    flex-shrink: 0;
  }

  /* ============================
     Buttons
     ============================ */
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    border-radius: var(--radius-sm);
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
    transition:
      background var(--duration-fast) var(--ease),
      color var(--duration-fast) var(--ease),
      box-shadow var(--duration-fast) var(--ease),
      opacity var(--duration-fast) var(--ease);
  }

  .btn-secondary {
    background: var(--surface);
    color: var(--text-primary);
    border: 1px solid var(--separator);
  }

  .btn-secondary:hover {
    background: var(--surface-hover);
    border-color: var(--separator-opaque);
  }

  .btn-danger {
    background: var(--red-subtle);
    color: var(--red);
  }

  .btn-danger:hover {
    background: rgba(255, 69, 58, 0.25);
  }

  .btn-ghost {
    background: transparent;
    color: var(--text-secondary);
  }

  .btn-ghost:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .btn-ghost-danger {
    background: transparent;
    color: var(--text-tertiary);
  }

  .btn-ghost-danger:hover {
    background: var(--red-subtle);
    color: var(--red);
  }

  .btn-sm {
    padding: var(--space-1) var(--space-3);
    font-size: 12px;
    font-weight: 500;
    border-radius: var(--radius-sm);
  }

  .nexus-link {
    text-decoration: none !important;
  }

  /* ============================
     Spinner
     ============================ */
  .spinner {
    display: inline-block;
    width: 14px;
    height: 14px;
    border: 2px solid var(--text-tertiary);
    border-top-color: var(--text-primary);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  /* ============================
     Bulk Action Bar
     ============================ */
  .bulk-action-bar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: var(--system-accent-subtle);
    border: 1px solid rgba(10, 132, 255, 0.2);
    border-radius: var(--radius-sm);
    flex-shrink: 0;
  }

  .bulk-count {
    font-size: 12px;
    font-weight: 600;
    color: var(--system-accent);
    margin-right: var(--space-2);
  }

  /* ============================
     Search & Filter Bar
     ============================ */
  .filter-bar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-shrink: 0;
  }

  .search-box {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-3);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-sm);
    flex: 1;
    max-width: 480px;
    color: var(--text-tertiary);
    transition: border-color var(--duration-fast) var(--ease);
    min-height: 32px;
  }

  .search-box:focus-within {
    border-color: var(--accent-muted);
  }

  .search-input {
    background: transparent;
    border: none;
    outline: none;
    color: var(--text-primary);
    font-size: 13px;
    flex: 1;
    min-width: 0;
  }

  .search-input::placeholder {
    color: var(--text-quaternary);
  }

  .search-clear {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    color: var(--text-tertiary);
    cursor: pointer;
    border-radius: 50%;
    transition: color var(--duration-fast) var(--ease);
  }

  .search-clear:hover {
    color: var(--text-primary);
  }

  .filter-count {
    font-size: 12px;
    color: var(--text-tertiary);
    font-weight: 500;
    white-space: nowrap;
  }

  /* ============================
     Empty State
     ============================ */
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    flex: 1;
    padding: var(--space-12) var(--space-6);
    background: var(--surface-glass);
    border: 1px solid var(--separator);
    border-radius: var(--radius-lg);
    box-shadow: var(--glass-refraction), var(--glass-edge-shadow);
    backdrop-filter: var(--glass-blur-light);
    text-align: center;
    gap: var(--space-3);
  }

  .empty-icon {
    color: var(--text-quaternary);
    margin-bottom: var(--space-2);
  }

  .empty-title {
    font-size: 17px;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .empty-description {
    font-size: 13px;
    color: var(--text-tertiary);
    max-width: 320px;
    line-height: 1.5;
    margin-bottom: var(--space-2);
  }

  .empty-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    justify-content: center;
    margin-top: var(--space-2);
  }

  .empty-action-btn {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    font-size: 13px;
    font-weight: 600;
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text-secondary);
    border: 1px solid var(--separator);
    cursor: pointer;
    text-decoration: none;
    transition: background var(--duration-fast) var(--ease), border-color var(--duration-fast) var(--ease);
  }

  .empty-action-btn:hover {
    background: var(--surface-hover);
    border-color: var(--accent);
  }

  /* ============================
     Mod Table
     ============================ */
  .mod-table-container {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border-radius: var(--radius-lg);
    background: var(--bg-primary);
    box-shadow: var(--glass-refraction), var(--glass-edge-shadow);
    min-height: 200px;
  }

  .mod-table {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    --grid-cols: 24px 48px minmax(0, 1fr) 100px 80px 72px 48px 90px 64px;
  }

  /* Narrow: hide category, origin, files, date */
  @media (max-width: 1200px) {
    .mod-table {
      --grid-cols: 24px 48px minmax(0, 1fr) 0px 0px 64px 0px 0px 60px !important;
    }
    .col-category,
    .col-origin,
    .col-files,
    .col-date {
      display: none;
    }
  }

  .table-header {
    display: grid;
    grid-template-columns: var(--grid-cols);
    padding: var(--space-2) var(--space-3);
    background: var(--bg-secondary);
    border-bottom: 1px solid var(--separator);
    flex-shrink: 0;
    z-index: 2;
  }

  .table-header > span,
  .table-header > button {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .sortable-header {
    cursor: pointer;
    user-select: none;
    transition: color var(--duration-fast) var(--ease), background var(--duration-fast) var(--ease);
    background: none;
    border: none;
    padding: var(--space-1) var(--space-2);
    margin: calc(-1 * var(--space-1)) 0;
    display: flex;
    align-items: center;
    gap: 4px;
    font-family: inherit;
    text-align: left;
  }

  .sortable-header:hover {
    color: var(--text-primary);
    background: var(--surface);
  }

  .sort-arrow {
    font-size: 8px;
    margin-left: 2px;
    color: var(--accent);
  }

  .table-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }

  .table-row {
    display: grid;
    grid-template-columns: var(--grid-cols);
    padding: 0 var(--space-3);
    align-items: center;
    font-size: 13px;
    height: 36px;
    box-sizing: border-box;
    transition:
      transform var(--duration-fast) var(--ease),
      box-shadow var(--duration-fast) var(--ease),
      background var(--duration-fast) var(--ease),
      opacity var(--duration-fast) var(--ease);
  }

  .table-row:nth-child(even) {
    background: var(--surface-subtle);
  }

  .table-row:hover {
    background: var(--surface-hover);
    transform: translateY(-1px);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.15);
  }

  .table-row.row-disabled {
    opacity: 0.45;
  }

  .table-row.row-disabled:hover {
    opacity: 0.6;
  }

  .row-checked {
    background: color-mix(in srgb, var(--system-accent) 12%, transparent);
    box-shadow: inset 2px 0 0 var(--system-accent);
  }

  /* ============================
     Bulk Select Checkbox Column
     ============================ */
  .col-check {
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
  }

  .check-box {
    width: 16px;
    height: 16px;
    border-radius: 4px;
    border: 1.5px solid var(--separator-opaque, rgba(255, 255, 255, 0.2));
    display: flex;
    align-items: center;
    justify-content: center;
    transition: background var(--duration-fast, 0.1s) ease, border-color var(--duration-fast, 0.1s) ease;
    flex-shrink: 0;
  }

  .check-box-checked {
    background: var(--system-accent, #007aff);
    border-color: var(--system-accent, #007aff);
    color: white;
  }

  /* ============================
     Toggle Switch (Pill)
     ============================ */
  .col-toggle {
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .header-sep-right {
    border-right: 1px solid var(--separator);
    padding-right: var(--space-2);
  }

  .toggle-switch {
    display: inline-flex;
    align-items: center;
    padding: 0;
    background: transparent;
    cursor: pointer;
  }

  .toggle-track {
    position: relative;
    width: 32px;
    height: 18px;
    border-radius: 9px;
    background: var(--bg-tertiary);
    transition:
      background var(--duration) var(--ease),
      box-shadow var(--duration) var(--ease);
  }

  .toggle-on .toggle-track {
    background: var(--green);
    box-shadow: 0 0 8px rgba(48, 209, 88, 0.25);
  }

  .toggle-thumb {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
    transition: transform var(--duration-fast) var(--ease-spring, cubic-bezier(0.34, 1.56, 0.64, 1)),
                width var(--duration-fast) var(--ease);
  }

  .toggle-on .toggle-thumb {
    transform: translateX(14px);
  }

  .toggle-busy .toggle-track {
    opacity: 0.6;
  }

  .toggle-switch:active .toggle-thumb {
    width: 16px;
    border-radius: 7px;
  }

  .toggle-on:active .toggle-thumb {
    transform: translateX(12px);
  }

  /* ============================
     Mod Name & Columns
     ============================ */
  .col-name {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .mod-name {
    font-weight: 500;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
    flex: 1;
  }

  .col-category {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .category-label {
    font-size: 12px;
    color: var(--text-secondary);
  }

  .text-muted {
    color: var(--text-quaternary);
  }

  .col-origin {
    display: flex;
    align-items: center;
    gap: 4px;
    overflow: hidden;
  }

  .origin-label {
    font-size: 11px;
    font-weight: 600;
    padding: 1px 6px;
    border-radius: 4px;
    white-space: nowrap;
    text-overflow: ellipsis;
    overflow: hidden;
  }

  .origin-nexus {
    background: rgba(218, 165, 32, 0.12);
    color: #c8971e;
  }

  .origin-loverslab {
    background: rgba(255, 59, 48, 0.10);
    color: var(--red);
  }

  .origin-moddb {
    background: rgba(48, 209, 88, 0.10);
    color: var(--green);
  }

  .origin-curseforge {
    background: rgba(255, 149, 0, 0.10);
    color: #e07b00;
  }

  .origin-direct,
  .origin-manual {
    background: var(--surface-hover);
    color: var(--text-tertiary);
  }

  .origin-link-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    border-radius: 4px;
    background: transparent;
    color: var(--text-quaternary);
    cursor: pointer;
    flex-shrink: 0;
    transition: color var(--duration-fast) var(--ease);
  }

  .origin-link-btn:hover {
    color: var(--accent);
  }

  .col-version {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-secondary);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    overflow: hidden;
  }

  .version-text {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .col-files {
    color: var(--text-secondary);
    font-size: 12px;
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .col-date {
    color: var(--text-secondary);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .col-actions {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    overflow: visible;
    position: relative;
  }

  .mod-action-group {
    display: flex;
    align-items: center;
    gap: 2px;
  }

  .mod-uninstall-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    border-radius: var(--radius-sm);
    color: var(--red);
    background: transparent;
    border: 1px solid color-mix(in srgb, var(--red) 30%, transparent);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
  }

  .mod-uninstall-btn:hover {
    background: color-mix(in srgb, var(--red) 12%, transparent);
    border-color: color-mix(in srgb, var(--red) 50%, transparent);
  }

  .confirm-actions {
    display: flex;
    gap: var(--space-1);
    align-items: center;
    position: absolute;
    right: 0;
    top: 50%;
    transform: translateY(-50%);
    z-index: 10;
    background: var(--surface-primary);
    padding: 2px 6px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--separator);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3);
    white-space: nowrap;
  }

  /* ============================
     Status Footer
     ============================ */
  .status-footer {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: 12px;
    color: var(--text-tertiary);
    padding: var(--space-1) var(--space-2);
    flex-shrink: 0;
    font-variant-numeric: tabular-nums;
  }

  .footer-selected {
    color: var(--system-accent);
    font-weight: 600;
  }

  /* ============================
     Drop Overlay (drag-drop install)
     ============================ */
  .drag-active {
    outline: 2px dashed var(--accent);
    outline-offset: -2px;
    border-radius: var(--radius-lg);
  }

  .drop-overlay {
    position: absolute;
    inset: 0;
    z-index: 200;
    display: flex;
    align-items: center;
    justify-content: center;
    background: color-mix(in srgb, var(--bg-base, var(--bg-primary)) 85%, transparent);
    backdrop-filter: var(--glass-blur-light);
    border-radius: var(--radius-lg);
    pointer-events: none;
  }

  .drop-overlay-content {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    color: var(--accent);
  }

  .drop-overlay-content p {
    font-size: 16px;
    font-weight: 600;
    letter-spacing: -0.01em;
  }

  /* ============================
     Conflicts Panel
     ============================ */
  .conflicts-panel {
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    flex-shrink: 0;
    overflow: hidden;
  }

  .conflicts-toggle {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: var(--space-2) var(--space-3);
    background: transparent;
    border: none;
    color: var(--text-primary);
    cursor: pointer;
    font-family: inherit;
    font-size: 13px;
    font-weight: 600;
    text-align: left;
    transition: background var(--duration-fast) var(--ease);
  }

  .conflicts-toggle:hover {
    background: var(--surface-hover);
  }

  .conflicts-chevron {
    color: var(--text-tertiary);
    transition: transform var(--duration-fast) var(--ease);
    flex-shrink: 0;
  }

  .conflicts-chevron-open {
    transform: rotate(90deg);
  }

  .conflicts-title {
    color: var(--text-primary);
  }

  .conflicts-count {
    background: var(--red-subtle);
    color: var(--red);
    padding: 1px 7px;
    border-radius: 100px;
    font-size: 11px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }

  .conflicts-spacer {
    flex: 1;
  }

  .conflicts-body {
    border-top: 1px solid var(--separator);
    padding: var(--space-2) var(--space-3);
    max-height: 280px;
    overflow-y: auto;
  }

  .conflicts-empty {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-4);
    font-size: 13px;
    color: var(--text-tertiary);
  }

  .conflicts-error {
    color: var(--red);
  }

  .conflicts-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .conflict-row {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--space-2) var(--space-3);
    background: var(--surface-subtle, var(--bg-secondary));
    border-radius: var(--radius-sm);
    border: 1px solid var(--separator);
  }

  .conflict-path {
    font-family: var(--font-mono);
    font-size: 12px;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .conflict-meta {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-1) var(--space-2);
    font-size: 12px;
    color: var(--text-secondary);
  }

  .conflict-label {
    color: var(--text-tertiary);
    font-weight: 600;
    text-transform: uppercase;
    font-size: 10px;
    letter-spacing: 0.04em;
  }

  .conflict-winner {
    color: var(--green, #30d158);
    font-weight: 600;
  }

  .conflict-losers {
    color: var(--text-tertiary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 360px;
  }

  .conflicts-footer {
    display: flex;
    justify-content: flex-end;
    padding-top: var(--space-2);
    margin-top: var(--space-2);
    border-top: 1px solid var(--separator);
  }
</style>
