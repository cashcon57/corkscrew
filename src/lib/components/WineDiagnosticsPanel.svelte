<script lang="ts">
  import { runWineDiagnostics, fixWineAppdata } from "$lib/api";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import type { DiagnosticResult, DiagnosticCheck, CheckStatus } from "$lib/types";

  interface Props {
    gameId: string;
    bottleName: string;
    gamePath?: string;
  }

  let { gameId, bottleName, gamePath }: Props = $props();
  let result = $state<DiagnosticResult | null>(null);
  let loading = $state(false);
  let fixing = $state(false);

  const categories = ["Structure", "Graphics", "Configuration", "Runtime"] as const;

  const groupedChecks = $derived.by(() => {
    if (!result) return new Map<string, DiagnosticCheck[]>();
    const groups = new Map<string, DiagnosticCheck[]>();
    for (const cat of categories) {
      const checks = result.checks.filter(c => c.category === cat);
      if (checks.length > 0) {
        groups.set(cat, checks);
      }
    }
    // Catch any checks with categories not in the predefined list
    const knownCategories = new Set<string>(categories);
    for (const check of result.checks) {
      if (!knownCategories.has(check.category)) {
        if (!groups.has(check.category)) {
          groups.set(check.category, []);
        }
        groups.get(check.category)!.push(check);
      }
    }
    return groups;
  });

  async function runDiag() {
    loading = true;
    try {
      result = await runWineDiagnostics(gameId, bottleName);
    } catch { result = null; }
    finally { loading = false; }
  }

  $effect(() => { if (gameId && bottleName) runDiag(); });

  function statusColor(status: CheckStatus): string {
    switch (status) {
      case "pass": return "var(--green)";
      case "warning": return "var(--yellow)";
      case "error": return "var(--red)";
      default: return "var(--text-tertiary)";
    }
  }

  function statusBg(status: CheckStatus): string {
    switch (status) {
      case "pass": return "var(--green-subtle, rgba(52, 199, 89, 0.12))";
      case "warning": return "rgba(255, 204, 0, 0.12)";
      case "error": return "var(--red-subtle, rgba(255, 69, 58, 0.12))";
      default: return "var(--surface)";
    }
  }

  function categoryIcon(category: string): string {
    switch (category) {
      case "Structure": return "folder";
      case "Graphics": return "gpu";
      case "Configuration": return "settings";
      case "Runtime": return "play";
      default: return "default";
    }
  }

  async function handleFix(check: DiagnosticCheck) {
    if (check.name === "AppData Local") {
      fixing = true;
      try {
        await fixWineAppdata(bottleName);
        await runDiag();
      } catch {} finally { fixing = false; }
    }
  }
</script>

{#snippet statusDot(status: CheckStatus)}
  <div
    class="status-dot"
    style="background: {statusColor(status)}; box-shadow: 0 0 6px {statusColor(status)}40;"
    role="img"
    aria-label={status}
  ></div>
{/snippet}

{#snippet statusSvg(status: CheckStatus)}
  {#if status === "pass"}
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
      <path d="M20 6L9 17l-5-5" />
    </svg>
  {:else if status === "warning"}
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
      <line x1="12" y1="9" x2="12" y2="13" />
      <line x1="12" y1="17" x2="12.01" y2="17" />
    </svg>
  {:else if status === "error"}
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <circle cx="12" cy="12" r="10" />
      <line x1="15" y1="9" x2="9" y2="15" />
      <line x1="9" y1="9" x2="15" y2="15" />
    </svg>
  {:else}
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <circle cx="12" cy="12" r="10" />
      <line x1="8" y1="12" x2="16" y2="12" />
    </svg>
  {/if}
{/snippet}

<div class="diag-panel">
  <!-- Header -->
  <div class="panel-header">
    <div class="panel-title-row">
      <svg class="panel-icon" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
      </svg>
      <h3 class="panel-title">Wine Diagnostics</h3>
    </div>

    {#if result && !loading}
      <div class="summary-row">
        <span class="summary-item summary-pass">
          <span class="summary-count">{result.passed}</span> passed
        </span>
        <span class="summary-sep">&middot;</span>
        <span class="summary-item summary-warn">
          <span class="summary-count">{result.warnings}</span> warning{result.warnings !== 1 ? "s" : ""}
        </span>
        <span class="summary-sep">&middot;</span>
        <span class="summary-item summary-error">
          <span class="summary-count">{result.errors}</span> error{result.errors !== 1 ? "s" : ""}
        </span>
      </div>
    {/if}
  </div>

  <!-- Toolbar -->
  <div class="panel-toolbar">
    <button
      class="btn btn-secondary btn-sm"
      onclick={runDiag}
      disabled={loading}
      type="button"
    >
      {#if loading}
        <span class="spinner-sm"></span>
        Running...
      {:else}
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M21 2v6h-6M3 12a9 9 0 0 1 15-6.7L21 8M3 22v-6h6M21 12a9 9 0 0 1-15 6.7L3 16" />
        </svg>
        Run Diagnostics
      {/if}
    </button>
    {#if gamePath}
      <button
        class="btn btn-secondary btn-sm"
        onclick={() => revealItemInDir(gamePath!)}
        type="button"
        title="Open game directory in Finder"
      >
        <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="M2 4h3l2-2h5a2 2 0 012 2v8a2 2 0 01-2 2H4a2 2 0 01-2-2V4z" />
        </svg>
        Open Game Directory
      </button>
    {/if}
  </div>

  <!-- Content -->
  {#if loading && !result}
    <div class="panel-loading">
      <div class="spinner"></div>
      <p class="loading-text">Running diagnostics...</p>
    </div>
  {:else if !result}
    <div class="panel-empty">
      <div class="empty-icon">
        <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
        </svg>
      </div>
      <p class="empty-title">Diagnostics unavailable</p>
      <p class="empty-description">Could not run Wine diagnostics for this bottle. Try again.</p>
    </div>
  {:else}
    <div class="checks-list">
      {#each [...groupedChecks.entries()] as [category, checks], gIdx}
        {#if gIdx > 0}
          <div class="category-divider"></div>
        {/if}
        <div class="category-group">
          <div class="category-header">
            <span class="category-icon">
              {#if categoryIcon(category) === "folder"}
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
                </svg>
              {:else if categoryIcon(category) === "gpu"}
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <rect x="2" y="6" width="20" height="12" rx="2" />
                  <line x1="6" y1="10" x2="6" y2="14" />
                  <line x1="10" y1="10" x2="10" y2="14" />
                  <line x1="14" y1="10" x2="14" y2="14" />
                </svg>
              {:else if categoryIcon(category) === "settings"}
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <circle cx="12" cy="12" r="3" />
                  <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z" />
                </svg>
              {:else if categoryIcon(category) === "play"}
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <polygon points="5 3 19 12 5 21 5 3" />
                </svg>
              {:else}
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <circle cx="12" cy="12" r="10" />
                  <line x1="12" y1="16" x2="12" y2="12" />
                  <line x1="12" y1="8" x2="12.01" y2="8" />
                </svg>
              {/if}
            </span>
            <span class="category-name">{category}</span>
            <span class="category-count">{checks.length}</span>
          </div>

          <div class="category-checks">
            {#each checks as check, cIdx (check.name)}
              {#if cIdx > 0}
                <div class="check-divider"></div>
              {/if}
              <div class="check-row">
                <div
                  class="check-status-icon"
                  style="color: {statusColor(check.status)}; background: {statusBg(check.status)};"
                >
                  {@render statusSvg(check.status)}
                </div>
                <div class="check-info">
                  <div class="check-header">
                    <span class="check-name">{check.name}</span>
                    <span class="check-category-badge">{check.category}</span>
                  </div>
                  <span class="check-message">{check.message}</span>
                  {#if check.fix_available && check.fix_description}
                    <span class="check-fix-hint">{check.fix_description}</span>
                  {/if}
                </div>
                {#if check.fix_available}
                  <button
                    class="btn btn-fix"
                    onclick={() => handleFix(check)}
                    disabled={fixing}
                    type="button"
                  >
                    {#if fixing}
                      <span class="spinner-xs"></span>
                    {:else}
                      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z" />
                      </svg>
                    {/if}
                    Fix
                  </button>
                {/if}
              </div>
            {/each}
          </div>
        </div>
      {/each}
    </div>

    <!-- Footer Summary -->
    <div class="panel-footer">
      {#if result.errors === 0 && result.warnings === 0}
        <span class="footer-ok">
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M20 6L9 17l-5-5" />
          </svg>
          All checks passed
        </span>
      {:else if result.errors > 0}
        <span class="footer-error">
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="10" />
            <line x1="15" y1="9" x2="9" y2="15" />
            <line x1="9" y1="9" x2="15" y2="15" />
          </svg>
          {result.errors} issue{result.errors !== 1 ? "s" : ""} require attention
        </span>
      {:else}
        <span class="footer-warn">
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
            <line x1="12" y1="9" x2="12" y2="13" />
            <line x1="12" y1="17" x2="12.01" y2="17" />
          </svg>
          {result.warnings} warning{result.warnings !== 1 ? "s" : ""} found
        </span>
      {/if}
    </div>
  {/if}
</div>

<style>
  /* ---- Panel ---- */

  .diag-panel {
    display: flex;
    flex-direction: column;
    background: var(--bg-grouped-secondary);
    border-radius: var(--radius-lg);
    overflow: hidden;
    box-shadow: var(--glass-refraction), var(--glass-edge-shadow);
  }

  /* ---- Header ---- */

  .panel-header {
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--separator);
    flex-shrink: 0;
  }

  .panel-title-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .panel-icon {
    color: var(--system-accent);
    flex-shrink: 0;
  }

  .panel-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .summary-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }

  .summary-item {
    font-size: 12px;
    font-weight: 500;
  }

  .summary-count {
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }

  .summary-pass {
    color: var(--green);
  }

  .summary-warn {
    color: var(--yellow);
  }

  .summary-error {
    color: var(--red);
  }

  .summary-sep {
    color: var(--text-quaternary);
    font-size: 12px;
  }

  /* ---- Toolbar ---- */

  .panel-toolbar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    border-bottom: 1px solid var(--separator);
    flex-shrink: 0;
  }

  /* ---- Buttons ---- */

  .btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 12px;
    font-weight: 500;
    border: none;
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
    white-space: nowrap;
  }

  .btn-sm {
    padding: var(--space-1) var(--space-3);
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

  .btn-fix {
    padding: var(--space-1) var(--space-3);
    background: transparent;
    color: var(--system-accent);
    border: 1px solid var(--system-accent);
    flex-shrink: 0;
  }

  .btn-fix:hover:not(:disabled) {
    background: var(--system-accent-subtle);
  }

  /* ---- Loading / Empty ---- */

  .panel-loading {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-8);
  }

  .spinner {
    width: 24px;
    height: 24px;
    border: 2px solid var(--separator-opaque);
    border-top-color: var(--system-accent);
    border-radius: 50%;
    animation: spin 0.75s linear infinite;
  }

  .spinner-sm {
    display: inline-block;
    width: 12px;
    height: 12px;
    border: 2px solid var(--separator-opaque);
    border-top-color: var(--system-accent);
    border-radius: 50%;
    animation: spin 0.75s linear infinite;
    flex-shrink: 0;
  }

  .spinner-xs {
    display: inline-block;
    width: 10px;
    height: 10px;
    border: 1.5px solid var(--separator-opaque);
    border-top-color: var(--system-accent);
    border-radius: 50%;
    animation: spin 0.75s linear infinite;
    flex-shrink: 0;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .loading-text {
    font-size: 12px;
    color: var(--text-tertiary);
  }

  .panel-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-8);
    text-align: center;
  }

  .empty-icon {
    color: var(--text-quaternary);
    margin-bottom: var(--space-1);
  }

  .empty-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .empty-description {
    font-size: 12px;
    color: var(--text-tertiary);
  }

  /* ---- Category Groups ---- */

  .checks-list {
    flex: 1;
    overflow-y: auto;
  }

  .category-divider {
    height: 1px;
    background: var(--separator);
  }

  .category-group {
    display: flex;
    flex-direction: column;
  }

  .category-header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    background: var(--surface-subtle);
    border-bottom: 1px solid var(--separator);
  }

  .category-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-tertiary);
    flex-shrink: 0;
  }

  .category-name {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .category-count {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    border-radius: 100px;
    font-size: 9px;
    font-weight: 700;
    color: var(--text-quaternary);
    background: var(--surface);
    font-variant-numeric: tabular-nums;
  }

  /* ---- Check Rows ---- */

  .category-checks {
    display: flex;
    flex-direction: column;
  }

  .check-divider {
    height: 1px;
    background: var(--separator);
    margin-left: var(--space-4);
  }

  .check-row {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    transition: background var(--duration-fast) var(--ease);
  }

  .check-row:hover {
    background: var(--surface-hover);
  }

  .check-status-icon {
    flex-shrink: 0;
    width: 24px;
    height: 24px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 50%;
    margin-top: 1px;
  }

  .check-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .check-header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .check-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .check-category-badge {
    font-size: 10px;
    font-weight: 600;
    padding: 1px 6px;
    border-radius: 3px;
    color: var(--text-tertiary);
    background: var(--surface);
    letter-spacing: 0.01em;
  }

  .check-message {
    font-size: 12px;
    color: var(--text-secondary);
    line-height: 1.4;
  }

  .check-fix-hint {
    font-size: 11px;
    color: var(--system-accent);
    font-weight: 500;
    margin-top: 1px;
  }

  /* ---- Status Dot (used in snippets) ---- */

  .status-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  /* ---- Footer ---- */

  .panel-footer {
    padding: var(--space-2) var(--space-4);
    border-top: 1px solid var(--separator);
    background: var(--surface-subtle);
    flex-shrink: 0;
  }

  .footer-ok,
  .footer-warn,
  .footer-error {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: 12px;
    font-weight: 500;
  }

  .footer-ok {
    color: var(--green);
  }

  .footer-warn {
    color: var(--yellow);
  }

  .footer-error {
    color: var(--red);
  }
</style>
