<script lang="ts">
  import { onMount } from "svelte";
  import { selectedGame, showError, showSuccess } from "$lib/stores";
  import type {
    DetectedGame,
    CrashLogEntry,
    CrashReport,
    CrashDiagnosis,
    SuggestedAction,
    CrashSeverity,
    ActionType,
    Confidence,
  } from "$lib/types";
  import {
    findCrashLogs,
    analyzeCrashLog,
    sortPluginsLoot,
    verifyModIntegrity,
    toggleMod,
    checkModUpdates,
  } from "$lib/api";

  let logs = $state<CrashLogEntry[]>([]);
  let loading = $state(false);
  let refreshing = $state(false);
  let selectedLogFilename = $state<string | null>(null);
  let selectedReport = $state<CrashReport | null>(null);
  let analyzingLog = $state(false);
  let expandedCallStack = $state(false);
  let actionInProgress = $state<string | null>(null);

  const selectedLog = $derived(logs.find(l => l.filename === selectedLogFilename) ?? null);

  /** Flatten all suggested actions from every diagnosis in the report */
  const allSuggestedActions = $derived(
    selectedReport
      ? selectedReport.diagnosis.flatMap(d => d.suggested_actions)
      : []
  );

  const game = $derived($selectedGame);

  onMount(() => {
    if (game) {
      loadCrashLogs(game);
    }
  });

  $effect(() => {
    if (game) {
      loadCrashLogs(game);
    }
  });

  async function loadCrashLogs(g: DetectedGame) {
    loading = true;
    try {
      logs = await findCrashLogs(g.game_id, g.bottle_name);
    } catch (e: any) {
      showError(`Failed to load crash logs: ${e}`);
    } finally {
      loading = false;
    }
  }

  async function selectLog(log: CrashLogEntry) {
    if (selectedLogFilename === log.filename) {
      // Deselect
      selectedLogFilename = null;
      selectedReport = null;
      expandedCallStack = false;
      return;
    }
    selectedLogFilename = log.filename;
    selectedReport = null;
    expandedCallStack = false;
    analyzingLog = true;
    try {
      selectedReport = await analyzeCrashLog(log.filename);
    } catch (e: any) {
      showError(`Failed to analyze crash log: ${e}`);
    } finally {
      analyzingLog = false;
    }
  }

  async function handleRefresh() {
    if (!game || refreshing) return;
    refreshing = true;
    try {
      logs = await findCrashLogs(game.game_id, game.bottle_name);
      selectedLogFilename = null;
      selectedReport = null;
      showSuccess("Crash logs refreshed");
    } catch (e: any) {
      showError(`Failed to refresh crash logs: ${e}`);
    } finally {
      refreshing = false;
    }
  }

  async function handleAction(action: SuggestedAction) {
    if (!game || actionInProgress) return;
    actionInProgress = action.description;
    try {
      switch (action.action_type) {
        case "SortLoadOrder":
          await sortPluginsLoot(game.game_id, game.bottle_name);
          showSuccess("Load order sorted");
          break;
        case "UpdateMod":
          await checkModUpdates(game.game_id, game.bottle_name);
          showSuccess(`Checking for updates${action.target ? ` for ${action.target}` : ""}`);
          break;
        case "VerifyIntegrity":
          // verifyModIntegrity takes a modId; target may contain a mod name rather than an id.
          // Show a success message; the user may need to act from the Mods page for a specific mod.
          showSuccess(`Verify integrity${action.target ? `: ${action.target}` : ""}`);
          break;
        case "DisableMod":
          // DisableMod requires a numeric modId; target is typically a mod/plugin name.
          // Show guidance since we don't have the numeric id from the crash report.
          showSuccess(`Disable recommended${action.target ? `: ${action.target}` : ""}`);
          break;
        case "ReinstallMod":
          showSuccess(`Reinstall recommended${action.target ? `: ${action.target}` : ""}`);
          break;
        case "CheckVRAM":
          showSuccess("Check your VRAM usage - you may be exceeding GPU memory");
          break;
        case "UpdateDrivers":
          showSuccess("Update your graphics drivers to the latest version");
          break;
        case "CheckINI":
          showSuccess(`Check INI settings${action.target ? `: ${action.target}` : ""}`);
          break;
        case "ManualFix":
          showSuccess(action.description);
          break;
      }
    } catch (e: any) {
      showError(`Action failed: ${e}`);
    } finally {
      actionInProgress = null;
    }
  }

  function formatRelativeTime(iso: string): string {
    const date = new Date(iso);
    const now = new Date();
    const diffMs = now.getTime() - date.getTime();
    const diffMins = Math.floor(diffMs / 60000);
    const diffHours = Math.floor(diffMs / 3600000);
    const diffDays = Math.floor(diffMs / 86400000);

    if (diffMins < 1) return "Just now";
    if (diffMins < 60) return `${diffMins} minute${diffMins !== 1 ? "s" : ""} ago`;
    if (diffHours < 24) return `${diffHours} hour${diffHours !== 1 ? "s" : ""} ago`;
    if (diffDays === 1) return "Yesterday";
    if (diffDays < 7) return `${diffDays} days ago`;
    return date.toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
  }

  function severityColor(severity: CrashSeverity): string {
    switch (severity) {
      case "Critical": return "var(--red)";
      case "High": return "#ff9f0a";
      case "Medium": return "var(--yellow)";
      case "Low": return "var(--green)";
      case "Unknown": return "var(--text-tertiary)";
    }
  }

  function severityBg(severity: CrashSeverity): string {
    switch (severity) {
      case "Critical": return "var(--red-subtle)";
      case "High": return "rgba(255, 159, 10, 0.15)";
      case "Medium": return "var(--yellow-subtle)";
      case "Low": return "var(--green-subtle)";
      case "Unknown": return "var(--surface)";
    }
  }

  function confidenceColor(confidence: Confidence): string {
    switch (confidence) {
      case "High": return "var(--green)";
      case "Medium": return "var(--yellow)";
      case "Low": return "var(--text-tertiary)";
    }
  }

  function confidenceBg(confidence: Confidence): string {
    switch (confidence) {
      case "High": return "var(--green-subtle)";
      case "Medium": return "var(--yellow-subtle)";
      case "Low": return "var(--surface)";
    }
  }

  function actionIcon(actionType: ActionType): string {
    switch (actionType) {
      case "SortLoadOrder": return "sort";
      case "UpdateMod": return "update";
      case "VerifyIntegrity": return "verify";
      case "DisableMod": return "disable";
      case "ReinstallMod": return "reinstall";
      case "CheckVRAM": return "vram";
      case "UpdateDrivers": return "drivers";
      case "CheckINI": return "ini";
      case "ManualFix": return "manual";
    }
  }

  function formatRam(usedMb: number | null, totalMb: number | null): string {
    if (usedMb != null && totalMb != null) return `${usedMb}MB / ${totalMb}MB`;
    if (totalMb != null) return `${totalMb}MB total`;
    if (usedMb != null) return `${usedMb}MB used`;
    return "Unknown";
  }
</script>

<div class="logs-page">
  <!-- Page Header -->
  <div class="page-header">
    <div class="header-text">
      <h2>Crash Logs</h2>
      <p class="page-subtitle">Analyze game crashes and get fix suggestions</p>
    </div>
    <div class="header-actions">
      {#if game}
        <span class="header-game-context">{game.display_name}</span>
      {/if}
      <button
        class="btn btn-secondary"
        onclick={handleRefresh}
        disabled={refreshing || !game}
        aria-label="Refresh crash logs"
      >
        {#if refreshing}
          <span class="spinner-sm"></span>
          Scanning...
        {:else}
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M21 2v6h-6M3 12a9 9 0 0 1 15-6.7L21 8M3 22v-6h6M21 12a9 9 0 0 1-15 6.7L3 16" />
          </svg>
          Refresh
        {/if}
      </button>
    </div>
  </div>

  {#if !game}
    <!-- No Game Selected -->
    <div class="empty-state">
      <div class="empty-icon">
        <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
          <polyline points="14 2 14 8 20 8" />
          <line x1="16" y1="13" x2="8" y2="13" />
          <line x1="16" y1="17" x2="8" y2="17" />
        </svg>
      </div>
      <h3 class="empty-title">No game selected</h3>
      <p class="empty-description">Select a game from the Mods page to analyze its crash logs.</p>
    </div>

  {:else if loading}
    <!-- Loading -->
    <div class="loading-state">
      <div class="spinner"></div>
      <p class="loading-text">Scanning for crash logs...</p>
    </div>

  {:else if logs.length === 0}
    <!-- No Crash Logs -->
    <div class="empty-state">
      <div class="empty-icon">
        <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="10" />
          <path d="M9 12l2 2 4-4" />
        </svg>
      </div>
      <h3 class="empty-title">No crash logs found</h3>
      <p class="empty-description">
        No crash logs were detected for {game.display_name}. Crash logs are generated
        when the game encounters a fatal error. If you've experienced crashes,
        try clicking Refresh to rescan.
      </p>
      <button class="btn btn-secondary" onclick={handleRefresh} disabled={refreshing}>
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M21 2v6h-6M3 12a9 9 0 0 1 15-6.7L21 8M3 22v-6h6M21 12a9 9 0 0 1-15 6.7L3 16" />
        </svg>
        Scan for Crash Logs
      </button>
    </div>

  {:else}
    <!-- Crash Log Layout: List + Detail -->
    <div class="logs-layout" class:has-selection={selectedLog !== null}>
      <!-- Crash Log Timeline -->
      <div class="logs-list" role="listbox" aria-label="Crash log entries">
        {#each logs as log (log.filename)}
          <button
            class="log-entry"
            class:log-entry-selected={selectedLogFilename === log.filename}
            onclick={() => selectLog(log)}
            role="option"
            aria-selected={selectedLogFilename === log.filename}
          >
            <div class="log-entry-header">
              <span
                class="severity-badge"
                style="color: {severityColor(log.severity)}; background: {severityBg(log.severity)};"
              >
                {log.severity}
              </span>
              <span class="log-timestamp">{formatRelativeTime(log.timestamp)}</span>
            </div>
            <p class="log-summary">{log.summary}</p>
          </button>
        {/each}
      </div>

      <!-- Crash Detail Panel -->
      {#if selectedLog}
        <div class="log-detail">
          <!-- Detail Header -->
          <div class="detail-header">
            <div class="detail-header-top">
              <span
                class="severity-badge severity-badge-lg"
                style="color: {severityColor(selectedLog.severity)}; background: {severityBg(selectedLog.severity)};"
              >
                {selectedLog.severity}
              </span>
              <span class="detail-timestamp">
                {new Date(selectedLog.timestamp).toLocaleString()}
              </span>
            </div>
            <h3 class="detail-summary">{selectedLog.summary}</h3>
          </div>

          {#if analyzingLog}
            <!-- Loading analysis -->
            <div class="loading-state">
              <div class="spinner"></div>
              <p class="loading-text">Analyzing crash log...</p>
            </div>
          {:else if selectedReport}
            <!-- Diagnoses -->
            {#if selectedReport.diagnosis.length > 0}
              <div class="detail-section">
                <h4 class="detail-section-title">Diagnosis</h4>
                <div class="diagnosis-cards">
                  {#each selectedReport.diagnosis as diagnosis}
                    <div class="diagnosis-card">
                      <div class="diagnosis-header">
                        <span class="diagnosis-title">{diagnosis.title}</span>
                        <span
                          class="confidence-badge"
                          style="color: {confidenceColor(diagnosis.confidence)}; background: {confidenceBg(diagnosis.confidence)};"
                        >
                          {diagnosis.confidence} confidence
                        </span>
                      </div>
                      <p class="diagnosis-description">{diagnosis.description}</p>
                    </div>
                  {/each}
                </div>
              </div>
            {/if}

            <!-- Suggested Actions (flattened from all diagnoses) -->
            {#if allSuggestedActions.length > 0}
              <div class="detail-section">
                <h4 class="detail-section-title">Suggested Actions</h4>
                <div class="actions-grid">
                  {#each allSuggestedActions as action}
                    <button
                      class="action-card"
                      onclick={() => handleAction(action)}
                      disabled={actionInProgress !== null}
                    >
                      <div class="action-icon">
                        {#if action.action_type === "SortLoadOrder"}
                          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <path d="M3 6h18M6 12h12M9 18h6" />
                          </svg>
                        {:else if action.action_type === "UpdateMod"}
                          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <path d="M21 2v6h-6M3 12a9 9 0 0 1 15-6.7L21 8M3 22v-6h6M21 12a9 9 0 0 1-15 6.7L3 16" />
                          </svg>
                        {:else if action.action_type === "VerifyIntegrity"}
                          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
                            <path d="M9 12l2 2 4-4" />
                          </svg>
                        {:else if action.action_type === "DisableMod"}
                          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <circle cx="12" cy="12" r="10" />
                            <line x1="15" y1="9" x2="9" y2="15" />
                            <line x1="9" y1="9" x2="15" y2="15" />
                          </svg>
                        {:else if action.action_type === "CheckVRAM"}
                          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <rect x="2" y="3" width="20" height="14" rx="2" />
                            <line x1="8" y1="21" x2="16" y2="21" />
                            <line x1="12" y1="17" x2="12" y2="21" />
                          </svg>
                        {:else if action.action_type === "ReinstallMod"}
                          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <path d="M21 2v6h-6M3 12a9 9 0 0 1 15-6.7L21 8M3 22v-6h6M21 12a9 9 0 0 1-15 6.7L3 16" />
                          </svg>
                        {:else}
                          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <circle cx="12" cy="12" r="10" />
                            <path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3" />
                            <line x1="12" y1="17" x2="12.01" y2="17" />
                          </svg>
                        {/if}
                      </div>
                      <div class="action-text">
                        <span class="action-label">{action.description}</span>
                        {#if action.target}
                          <span class="action-target">{action.target}</span>
                        {/if}
                      </div>
                      {#if actionInProgress === action.description}
                        <span class="spinner-sm"></span>
                      {/if}
                    </button>
                  {/each}
                </div>
              </div>
            {/if}

            <!-- Involved Plugins -->
            {#if selectedReport.involved_plugins.length > 0}
              <div class="detail-section">
                <h4 class="detail-section-title">Involved Plugins</h4>
                <div class="plugin-list">
                  {#each selectedReport.involved_plugins as plugin}
                    <span class="plugin-tag plugin-tag-esp">{plugin}</span>
                  {/each}
                </div>
              </div>
            {/if}

            <!-- Involved SKSE Plugins -->
            {#if selectedReport.involved_skse_plugins.length > 0}
              <div class="detail-section">
                <h4 class="detail-section-title">Involved SKSE Plugins</h4>
                <div class="plugin-list">
                  {#each selectedReport.involved_skse_plugins as plugin}
                    <span class="plugin-tag plugin-tag-dll">{plugin}</span>
                  {/each}
                </div>
              </div>
            {/if}

            <!-- System Info -->
            {#if selectedReport.system_info}
              <div class="detail-section">
                <h4 class="detail-section-title">System Information</h4>
                <div class="system-info-card">
                  <div class="sysinfo-row">
                    <span class="sysinfo-label">RAM</span>
                    <span class="sysinfo-value">{formatRam(selectedReport.system_info.ram_used_mb, selectedReport.system_info.ram_total_mb)}</span>
                  </div>
                  <div class="sysinfo-divider"></div>
                  <div class="sysinfo-row">
                    <span class="sysinfo-label">VRAM</span>
                    <span class="sysinfo-value">{formatRam(selectedReport.system_info.vram_used_mb, selectedReport.system_info.vram_total_mb)}</span>
                  </div>
                  <div class="sysinfo-divider"></div>
                  <div class="sysinfo-row">
                    <span class="sysinfo-label">GPU</span>
                    <span class="sysinfo-value">{selectedReport.system_info.gpu ?? "Unknown"}</span>
                  </div>
                </div>
              </div>
            {/if}

            <!-- Call Stack -->
            {#if selectedReport.call_stack_summary.length > 0}
              <div class="detail-section">
                <button
                  class="callstack-toggle"
                  onclick={() => expandedCallStack = !expandedCallStack}
                  aria-expanded={expandedCallStack}
                >
                  <svg
                    class="callstack-chevron"
                    class:callstack-chevron-open={expandedCallStack}
                    width="12"
                    height="12"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2.5"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                  >
                    <polyline points="6 9 12 15 18 9" />
                  </svg>
                  <h4 class="detail-section-title">Call Stack</h4>
                </button>
                {#if expandedCallStack}
                  <div class="callstack-container">
                    <pre class="callstack-content">{selectedReport.call_stack_summary.join("\n")}</pre>
                  </div>
                {/if}
              </div>
            {/if}
          {/if}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  /* ---- Page Layout ---- */

  .logs-page {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
  }

  /* ---- Page Header ---- */

  .page-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-4);
  }

  .header-text h2 {
    font-size: 22px;
    font-weight: 700;
    letter-spacing: -0.02em;
  }

  .page-subtitle {
    font-size: 14px;
    color: var(--text-secondary);
    margin-top: var(--space-1);
  }

  .header-actions {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-shrink: 0;
  }

  .header-game-context {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-tertiary);
    background: var(--surface);
    padding: var(--space-1) var(--space-3);
    border-radius: var(--radius-sm);
  }

  /* ---- Buttons ---- */

  .btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 14px;
    font-size: 13px;
    font-weight: 500;
    border: none;
    border-radius: var(--radius);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
    white-space: nowrap;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-secondary {
    background: var(--surface-hover);
    color: var(--text-secondary);
  }

  .btn-secondary:hover:not(:disabled) {
    background: var(--surface-active);
    color: var(--text-primary);
  }

  /* ---- Empty State ---- */

  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-12) var(--space-8);
    background: var(--surface);
    border-radius: var(--radius-lg);
    box-shadow: var(--glass-edge-shadow);
    text-align: center;
  }

  .empty-icon {
    color: var(--text-quaternary);
    margin-bottom: var(--space-1);
  }

  .empty-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .empty-description {
    font-size: 13px;
    color: var(--text-tertiary);
    max-width: 400px;
    line-height: 1.5;
    margin-bottom: var(--space-2);
  }

  /* ---- Loading State ---- */

  .loading-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-4);
    padding: var(--space-12);
  }

  .loading-text {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-tertiary);
  }

  .spinner {
    width: 28px;
    height: 28px;
    border: 2.5px solid var(--separator-opaque);
    border-top-color: var(--system-accent);
    border-radius: 50%;
    animation: spin 0.75s linear infinite;
  }

  .spinner-sm {
    display: inline-block;
    width: 14px;
    height: 14px;
    border: 2px solid var(--separator-opaque);
    border-top-color: var(--system-accent);
    border-radius: 50%;
    animation: spin 0.75s linear infinite;
    flex-shrink: 0;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  /* ---- Logs Layout (List + Detail) ---- */

  .logs-layout {
    display: grid;
    grid-template-columns: 1fr;
    gap: var(--space-4);
  }

  .logs-layout.has-selection {
    grid-template-columns: 340px 1fr;
  }

  /* ---- Logs List (Timeline) ---- */

  .logs-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    max-height: calc(100vh - 240px);
    overflow-y: auto;
  }

  .log-entry {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    text-align: left;
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
  }

  .log-entry:hover {
    background: var(--surface-hover);
    border-color: var(--separator-opaque);
  }

  .log-entry-selected {
    background: var(--system-accent-subtle);
    border-color: var(--system-accent);
  }

  .log-entry-selected:hover {
    background: var(--system-accent-subtle);
  }

  .log-entry-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .severity-badge {
    display: inline-flex;
    align-items: center;
    padding: 1px 8px;
    border-radius: 100px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.01em;
  }

  .severity-badge-lg {
    padding: 2px 10px;
    font-size: 12px;
  }

  .log-timestamp {
    font-size: 12px;
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
  }

  .log-summary {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-primary);
    line-height: 1.4;
  }

  /* ---- Log Detail Panel ---- */

  .log-detail {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    max-height: calc(100vh - 240px);
    overflow-y: auto;
    padding-right: var(--space-2);
  }

  .detail-header {
    padding: var(--space-4) var(--space-5);
    background: var(--surface);
    border-radius: var(--radius-lg);
    box-shadow: var(--glass-edge-shadow);
  }

  .detail-header-top {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-bottom: var(--space-2);
  }

  .detail-timestamp {
    font-size: 12px;
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
  }

  .detail-summary {
    font-size: 17px;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  /* ---- Detail Sections ---- */

  .detail-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .detail-section-title {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    margin: 0;
  }

  /* ---- Diagnosis Cards ---- */

  .diagnosis-cards {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .diagnosis-card {
    padding: var(--space-3) var(--space-4);
    background: var(--surface);
    border-radius: var(--radius);
    box-shadow: var(--glass-edge-shadow);
  }

  .diagnosis-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    margin-bottom: var(--space-1);
  }

  .diagnosis-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .confidence-badge {
    display: inline-flex;
    align-items: center;
    padding: 1px 8px;
    border-radius: 100px;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.01em;
    flex-shrink: 0;
  }

  .diagnosis-description {
    font-size: 12px;
    color: var(--text-secondary);
    line-height: 1.5;
  }

  /* ---- Suggested Actions ---- */

  .actions-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: var(--space-2);
  }

  .action-card {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    box-shadow: var(--glass-edge-shadow);
    text-align: left;
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
  }

  .action-card:hover:not(:disabled) {
    background: var(--surface-hover);
    border-color: var(--system-accent);
  }

  .action-card:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .action-icon {
    width: 32px;
    height: 32px;
    border-radius: var(--radius-sm);
    background: var(--system-accent-subtle);
    color: var(--system-accent);
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .action-text {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
    flex: 1;
  }

  .action-label {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-primary);
  }

  .action-target {
    font-size: 11px;
    color: var(--text-tertiary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* ---- Plugin Tags ---- */

  .plugin-list {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }

  .plugin-tag {
    display: inline-flex;
    align-items: center;
    padding: 2px 8px;
    border-radius: var(--radius-sm);
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0;
  }

  .plugin-tag-esp {
    background: var(--system-accent-subtle);
    color: var(--system-accent);
  }

  .plugin-tag-dll {
    background: rgba(255, 159, 10, 0.15);
    color: #ff9f0a;
  }

  /* ---- System Info Card ---- */

  .system-info-card {
    background: var(--surface);
    border-radius: var(--radius);
    overflow: hidden;
    box-shadow: var(--glass-edge-shadow);
  }

  .sysinfo-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-2) var(--space-4);
  }

  .sysinfo-label {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
  }

  .sysinfo-value {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-primary);
    font-family: var(--font-mono);
    letter-spacing: 0;
  }

  .sysinfo-divider {
    height: 1px;
    background: var(--separator);
    margin-left: var(--space-4);
  }

  /* ---- Call Stack ---- */

  .callstack-toggle {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    cursor: pointer;
    padding: var(--space-1) 0;
    background: none;
    border: none;
    color: inherit;
  }

  .callstack-toggle:hover .detail-section-title {
    color: var(--text-primary);
  }

  .callstack-chevron {
    transition: transform var(--duration-fast) var(--ease);
    color: var(--text-tertiary);
    flex-shrink: 0;
  }

  .callstack-chevron-open {
    transform: rotate(0deg);
  }

  .callstack-chevron:not(.callstack-chevron-open) {
    transform: rotate(-90deg);
  }

  .callstack-container {
    background: var(--bg-primary);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    overflow: auto;
    max-height: 300px;
  }

  .callstack-content {
    padding: var(--space-3) var(--space-4);
    font-family: var(--font-mono);
    font-size: 11px;
    line-height: 1.6;
    color: var(--text-secondary);
    white-space: pre;
    margin: 0;
    letter-spacing: 0;
  }
</style>
