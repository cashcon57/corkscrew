<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { selectedGame, showError, showSuccess } from "$lib/stores";
  import type { DetectedGame } from "$lib/types";
  // TODO: Import from $lib/api when backend commands are ready
  // import { invoke } from "@tauri-apps/api/core";

  // ---- Types ----

  type ImportModStatus = "installed" | "auto_download" | "manual" | "version_mismatch";

  interface ModlistMetadata {
    name: string;
    game: string;
    gameId: string;
    modCount: number;
    exportDate: string;
    exporterName: string;
    exporterVersion: string;
  }

  interface ImportMod {
    name: string;
    version: string;
    nexusModId: number | null;
    nexusFileId: number | null;
    status: ImportModStatus;
    currentVersion?: string; // if version_mismatch
    message?: string;
  }

  interface ImportPlan {
    metadata: ModlistMetadata;
    mods: ImportMod[];
    readyCount: number;
    downloadCount: number;
    manualCount: number;
    mismatchCount: number;
  }

  // ---- Props ----

  interface Props {
    onclose?: () => void;
    oncomplete?: () => void;
  }

  let { onclose, oncomplete }: Props = $props();

  // ---- State ----

  let currentStep = $state<1 | 2 | 3>(1);
  let filePath = $state<string | null>(null);
  let validating = $state(false);
  let validationError = $state<string | null>(null);
  let metadata = $state<ModlistMetadata | null>(null);
  let gameCompatible = $state(true);

  // Step 2
  let importPlan = $state<ImportPlan | null>(null);
  let planLoading = $state(false);

  // Step 3
  let importing = $state(false);
  let importProgress = $state(0);
  let importTotal = $state(0);
  let importComplete = $state(false);
  let importErrors = $state<string[]>([]);

  const game = $derived($selectedGame);

  const statusCounts = $derived.by(() => {
    if (!importPlan) return { installed: 0, autoDownload: 0, manual: 0, mismatch: 0 };
    return {
      installed: importPlan.readyCount,
      autoDownload: importPlan.downloadCount,
      manual: importPlan.manualCount,
      mismatch: importPlan.mismatchCount,
    };
  });

  // ---- Step 1: File Selection ----

  async function handleSelectFile() {
    const selected = await open({
      multiple: false,
      filters: [
        {
          name: "Mod Lists",
          extensions: ["json", "txt", "modlist"],
        },
      ],
    });

    if (!selected) return;
    filePath = selected as string;
    await validateFile();
  }

  async function validateFile() {
    if (!filePath) return;
    validating = true;
    validationError = null;
    metadata = null;
    try {
      // TODO: Wire up when backend is ready
      // const result = await invoke("validate_modlist_file", { filePath });
      // metadata = result as ModlistMetadata;

      // Placeholder metadata
      const fileName = filePath.split("/").pop() ?? filePath;
      metadata = {
        name: fileName.replace(/\.[^/.]+$/, ""),
        game: game?.display_name ?? "Unknown Game",
        gameId: game?.game_id ?? "",
        modCount: 0,
        exportDate: new Date().toISOString(),
        exporterName: "Corkscrew",
        exporterVersion: "0.1.0",
      };

      // Check game compatibility
      if (game && metadata.gameId && metadata.gameId !== game.game_id) {
        gameCompatible = false;
      } else {
        gameCompatible = true;
      }
    } catch (e: any) {
      validationError = `Failed to validate file: ${e}`;
    } finally {
      validating = false;
    }
  }

  async function proceedToStep2() {
    if (!filePath || !metadata || !game) return;
    currentStep = 2;
    planLoading = true;
    try {
      // TODO: Wire up when backend is ready
      // const plan = await invoke("plan_modlist_import", {
      //   filePath,
      //   gameId: game.game_id,
      //   bottleName: game.bottle_name,
      // });
      // importPlan = plan as ImportPlan;

      // Placeholder plan
      importPlan = {
        metadata: metadata,
        mods: [],
        readyCount: 0,
        downloadCount: 0,
        manualCount: 0,
        mismatchCount: 0,
      };
    } catch (e: any) {
      showError(`Failed to generate import plan: ${e}`);
      currentStep = 1;
    } finally {
      planLoading = false;
    }
  }

  // ---- Step 3: Import ----

  async function proceedToStep3() {
    if (!importPlan || !game) return;
    currentStep = 3;
    importing = true;
    importProgress = 0;
    importTotal = importPlan.mods.length;
    importErrors = [];
    importComplete = false;

    try {
      // TODO: Wire up when backend is ready
      // Use an event listener for progress updates
      // await invoke("execute_modlist_import", {
      //   filePath,
      //   gameId: game.game_id,
      //   bottleName: game.bottle_name,
      // });

      // Simulate completion
      importProgress = importTotal;
      importComplete = true;
      showSuccess("Mod list import complete");

      if (oncomplete) {
        oncomplete();
      }
    } catch (e: any) {
      importErrors = [...importErrors, `Import failed: ${e}`];
    } finally {
      importing = false;
    }
  }

  function goBack() {
    if (currentStep === 2) {
      currentStep = 1;
      importPlan = null;
    } else if (currentStep === 3 && importComplete) {
      if (onclose) onclose();
    }
  }

  function formatDate(iso: string): string {
    return new Date(iso).toLocaleDateString(undefined, {
      month: "short",
      day: "numeric",
      year: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  }

  function statusLabel(status: ImportModStatus): string {
    switch (status) {
      case "installed": return "Installed";
      case "auto_download": return "Auto Download";
      case "manual": return "Manual";
      case "version_mismatch": return "Version Mismatch";
    }
  }

  function statusColor(status: ImportModStatus): string {
    switch (status) {
      case "installed": return "var(--green)";
      case "auto_download": return "var(--system-accent)";
      case "manual": return "var(--yellow)";
      case "version_mismatch": return "#ff9f0a";
    }
  }

  function statusBg(status: ImportModStatus): string {
    switch (status) {
      case "installed": return "var(--green-subtle)";
      case "auto_download": return "var(--system-accent-subtle)";
      case "manual": return "var(--yellow-subtle)";
      case "version_mismatch": return "rgba(255, 159, 10, 0.15)";
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && onclose) {
      onclose();
    }
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div
  class="wizard-backdrop"
  onclick={() => { if (onclose) onclose(); }}
  onkeydown={handleKeydown}
  role="presentation"
>
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_interactive_supports_focus -->
  <div class="wizard" onclick={(e) => e.stopPropagation()} role="dialog" aria-label="Import Mod List">
    <!-- Header -->
    <div class="wizard-header">
      <div class="wizard-title-row">
        <h3 class="wizard-title">Import Mod List</h3>
        {#if onclose}
          <button class="wizard-close" onclick={onclose} aria-label="Close" type="button">
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
              <line x1="2" y1="2" x2="10" y2="10" />
              <line x1="10" y1="2" x2="2" y2="10" />
            </svg>
          </button>
        {/if}
      </div>

      <!-- Step Indicator -->
      <div class="step-indicator">
        {#each [1, 2, 3] as step}
          <div
            class="step-dot"
            class:step-active={currentStep === step}
            class:step-complete={currentStep > step}
          >
            {#if currentStep > step}
              <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round">
                <path d="M20 6L9 17l-5-5" />
              </svg>
            {:else}
              {step}
            {/if}
          </div>
          {#if step < 3}
            <div class="step-line" class:step-line-active={currentStep > step}></div>
          {/if}
        {/each}
      </div>

      <div class="step-labels">
        <span class="step-label" class:step-label-active={currentStep === 1}>Select File</span>
        <span class="step-label" class:step-label-active={currentStep === 2}>Review Plan</span>
        <span class="step-label" class:step-label-active={currentStep === 3}>Import</span>
      </div>
    </div>

    <!-- Body -->
    <div class="wizard-body">
      <!-- Step 1: File Selection -->
      {#if currentStep === 1}
        <div class="step-content">
          {#if !filePath}
            <div class="file-select-zone">
              <div class="file-icon">
                <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
                  <polyline points="14 2 14 8 20 8" />
                  <line x1="12" y1="18" x2="12" y2="12" />
                  <line x1="9" y1="15" x2="15" y2="15" />
                </svg>
              </div>
              <p class="file-select-text">Select a mod list file to import</p>
              <p class="file-select-hint">Supports .json, .txt, and .modlist files</p>
              <button
                class="btn btn-accent"
                onclick={handleSelectFile}
                type="button"
              >
                Choose File
              </button>
            </div>
          {:else}
            <div class="file-info-section">
              <div class="file-path-card">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
                  <polyline points="14 2 14 8 20 8" />
                </svg>
                <span class="file-path-text">{filePath.split("/").pop()}</span>
                <button
                  class="btn-ghost-sm"
                  onclick={handleSelectFile}
                  type="button"
                >
                  Change
                </button>
              </div>

              {#if validating}
                <div class="validation-loading">
                  <span class="spinner-sm"></span>
                  <span>Validating mod list...</span>
                </div>
              {:else if validationError}
                <div class="validation-error">
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <circle cx="12" cy="12" r="10" />
                    <line x1="15" y1="9" x2="9" y2="15" />
                    <line x1="9" y1="9" x2="15" y2="15" />
                  </svg>
                  <span>{validationError}</span>
                </div>
              {:else if metadata}
                <div class="metadata-card">
                  <h4 class="metadata-title">{metadata.name}</h4>
                  <div class="metadata-grid">
                    <div class="metadata-item">
                      <span class="metadata-label">Game</span>
                      <span class="metadata-value">{metadata.game}</span>
                    </div>
                    <div class="metadata-item">
                      <span class="metadata-label">Mods</span>
                      <span class="metadata-value">{metadata.modCount}</span>
                    </div>
                    <div class="metadata-item">
                      <span class="metadata-label">Exported</span>
                      <span class="metadata-value">{formatDate(metadata.exportDate)}</span>
                    </div>
                    <div class="metadata-item">
                      <span class="metadata-label">Source</span>
                      <span class="metadata-value">{metadata.exporterName} {metadata.exporterVersion}</span>
                    </div>
                  </div>

                  {#if !gameCompatible}
                    <div class="compat-warning">
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
                        <line x1="12" y1="9" x2="12" y2="13" />
                        <line x1="12" y1="17" x2="12.01" y2="17" />
                      </svg>
                      <span>
                        This mod list is for <strong>{metadata.game}</strong> but the selected game is
                        <strong>{game?.display_name ?? "none"}</strong>. The import may not work correctly.
                      </span>
                    </div>
                  {/if}
                </div>
              {/if}
            </div>
          {/if}
        </div>

      <!-- Step 2: Import Plan Review -->
      {:else if currentStep === 2}
        <div class="step-content">
          {#if planLoading}
            <div class="plan-loading">
              <div class="spinner"></div>
              <p class="loading-text">Analyzing mod list...</p>
            </div>
          {:else if importPlan}
            <!-- Summary Bar -->
            <div class="plan-summary">
              <div class="summary-item summary-ready">
                <span class="summary-count">{statusCounts.installed}</span>
                <span class="summary-label">Already installed</span>
              </div>
              <div class="summary-item summary-download">
                <span class="summary-count">{statusCounts.autoDownload}</span>
                <span class="summary-label">Auto download</span>
              </div>
              <div class="summary-item summary-manual">
                <span class="summary-count">{statusCounts.manual}</span>
                <span class="summary-label">Manual action</span>
              </div>
              {#if statusCounts.mismatch > 0}
                <div class="summary-item summary-mismatch">
                  <span class="summary-count">{statusCounts.mismatch}</span>
                  <span class="summary-label">Version mismatch</span>
                </div>
              {/if}
            </div>

            <!-- Mod Table -->
            {#if importPlan.mods.length > 0}
              <div class="plan-table-container">
                <div class="plan-table">
                  <div class="plan-table-header">
                    <span class="col-plan-name">Mod</span>
                    <span class="col-plan-version">Version</span>
                    <span class="col-plan-status">Status</span>
                  </div>
                  <div class="plan-table-body">
                    {#each importPlan.mods as mod}
                      <div class="plan-table-row">
                        <span class="col-plan-name">
                          <span class="plan-mod-name">{mod.name}</span>
                        </span>
                        <span class="col-plan-version">
                          <span class="version-text">{mod.version}</span>
                          {#if mod.status === "version_mismatch" && mod.currentVersion}
                            <span class="version-current">current: {mod.currentVersion}</span>
                          {/if}
                        </span>
                        <span class="col-plan-status">
                          <span
                            class="status-badge"
                            style="color: {statusColor(mod.status)}; background: {statusBg(mod.status)};"
                          >
                            {statusLabel(mod.status)}
                          </span>
                        </span>
                      </div>
                    {/each}
                  </div>
                </div>
              </div>
            {:else}
              <div class="plan-empty">
                <p>No mods found in this mod list.</p>
              </div>
            {/if}
          {/if}
        </div>

      <!-- Step 3: Import Progress -->
      {:else if currentStep === 3}
        <div class="step-content step-content-center">
          {#if importing && !importComplete}
            <div class="import-progress-section">
              <div class="progress-icon">
                <div class="spinner-lg"></div>
              </div>
              <h4 class="progress-title">Importing mod list...</h4>
              <p class="progress-detail">
                {importProgress} of {importTotal} mods processed
              </p>
              <div class="progress-bar-track">
                <div
                  class="progress-bar-fill"
                  style="width: {importTotal > 0 ? (importProgress / importTotal * 100) : 0}%"
                ></div>
              </div>
            </div>
          {:else if importComplete}
            <div class="import-complete-section">
              <div class="complete-icon">
                <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="var(--green)" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <circle cx="12" cy="12" r="10" />
                  <path d="M9 12l2 2 4-4" />
                </svg>
              </div>
              <h4 class="complete-title">Import Complete</h4>
              <p class="complete-detail">
                {importTotal} mod{importTotal !== 1 ? "s" : ""} processed successfully.
              </p>
              {#if importErrors.length > 0}
                <div class="import-errors">
                  <h4 class="errors-title">Errors ({importErrors.length})</h4>
                  {#each importErrors as error}
                    <p class="error-item">{error}</p>
                  {/each}
                </div>
              {/if}
            </div>
          {/if}
        </div>
      {/if}
    </div>

    <!-- Footer -->
    <div class="wizard-footer">
      {#if currentStep === 1}
        <button
          class="btn btn-ghost"
          onclick={onclose}
          type="button"
        >
          Cancel
        </button>
        <button
          class="btn btn-accent"
          onclick={proceedToStep2}
          disabled={!metadata || !gameCompatible || validating}
          type="button"
        >
          Continue
        </button>
      {:else if currentStep === 2}
        <button
          class="btn btn-ghost"
          onclick={goBack}
          disabled={planLoading}
          type="button"
        >
          Back
        </button>
        <button
          class="btn btn-accent"
          onclick={proceedToStep3}
          disabled={planLoading || !importPlan}
          type="button"
        >
          Begin Import
        </button>
      {:else if currentStep === 3}
        {#if importComplete}
          <button
            class="btn btn-accent"
            onclick={() => { if (onclose) onclose(); }}
            type="button"
          >
            Done
          </button>
        {:else}
          <span class="footer-hint">Import in progress...</span>
        {/if}
      {/if}
    </div>
  </div>
</div>

<style>
  /* ---- Backdrop ---- */

  .wizard-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 2000;
    animation: fadeIn 0.2s var(--ease);
  }

  /* ---- Wizard Container ---- */

  .wizard {
    background: var(--bg-elevated);
    border: 1px solid var(--separator-opaque);
    border-radius: var(--radius-xl);
    box-shadow: var(--shadow-lg);
    width: 560px;
    max-width: calc(100vw - var(--space-8));
    max-height: calc(100vh - var(--space-12));
    display: flex;
    flex-direction: column;
    animation: dialogIn 0.25s var(--ease-out);
  }

  /* ---- Header ---- */

  .wizard-header {
    padding: var(--space-5) var(--space-5) var(--space-3);
    border-bottom: 1px solid var(--separator);
    flex-shrink: 0;
  }

  .wizard-title-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--space-4);
  }

  .wizard-title {
    font-size: 17px;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .wizard-close {
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    color: var(--text-tertiary);
    transition: all var(--duration-fast) var(--ease);
  }

  .wizard-close:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  /* ---- Step Indicator ---- */

  .step-indicator {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0;
    margin-bottom: var(--space-2);
  }

  .step-dot {
    width: 24px;
    height: 24px;
    border-radius: 50%;
    background: var(--surface-hover);
    color: var(--text-tertiary);
    font-size: 11px;
    font-weight: 700;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    transition: all var(--duration) var(--ease);
  }

  .step-active {
    background: var(--system-accent);
    color: white;
  }

  .step-complete {
    background: var(--green);
    color: white;
  }

  .step-line {
    width: 60px;
    height: 2px;
    background: var(--separator-opaque);
    transition: background var(--duration) var(--ease);
  }

  .step-line-active {
    background: var(--green);
  }

  .step-labels {
    display: flex;
    justify-content: space-between;
    padding: 0 var(--space-4);
  }

  .step-label {
    font-size: 11px;
    font-weight: 500;
    color: var(--text-tertiary);
    text-align: center;
    flex: 1;
  }

  .step-label-active {
    color: var(--system-accent);
  }

  /* ---- Body ---- */

  .wizard-body {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-4) var(--space-5);
  }

  .step-content {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .step-content-center {
    align-items: center;
    justify-content: center;
    min-height: 200px;
  }

  /* ---- Step 1: File Selection ---- */

  .file-select-zone {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-8) var(--space-6);
    border: 2px dashed var(--separator-opaque);
    border-radius: var(--radius-lg);
    text-align: center;
  }

  .file-icon {
    color: var(--text-quaternary);
  }

  .file-select-text {
    font-size: 14px;
    font-weight: 500;
    color: var(--text-primary);
  }

  .file-select-hint {
    font-size: 12px;
    color: var(--text-tertiary);
    margin-bottom: var(--space-2);
  }

  .file-info-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .file-path-card {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: var(--surface);
    border-radius: var(--radius);
    color: var(--text-tertiary);
  }

  .file-path-text {
    flex: 1;
    font-size: 13px;
    font-family: var(--font-mono);
    color: var(--text-primary);
    letter-spacing: 0;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .btn-ghost-sm {
    font-size: 12px;
    font-weight: 500;
    color: var(--system-accent);
    background: none;
    border: none;
    cursor: pointer;
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
    transition: background var(--duration-fast) var(--ease);
    flex-shrink: 0;
  }

  .btn-ghost-sm:hover {
    background: var(--system-accent-subtle);
  }

  .validation-loading {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3);
    font-size: 13px;
    color: var(--text-tertiary);
  }

  .validation-error {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3);
    background: var(--red-subtle);
    border-radius: var(--radius);
    color: var(--red);
    font-size: 13px;
  }

  /* ---- Metadata Card ---- */

  .metadata-card {
    background: var(--surface);
    border-radius: var(--radius);
    padding: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .metadata-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .metadata-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-2);
  }

  .metadata-item {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .metadata-label {
    font-size: 11px;
    font-weight: 500;
    color: var(--text-tertiary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .metadata-value {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-primary);
  }

  .compat-warning {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: var(--yellow-subtle);
    border-radius: var(--radius-sm);
    color: var(--yellow);
    font-size: 12px;
    line-height: 1.4;
  }

  .compat-warning svg {
    flex-shrink: 0;
    margin-top: 1px;
  }

  .compat-warning strong {
    font-weight: 600;
  }

  /* ---- Step 2: Plan Summary ---- */

  .plan-loading {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-8);
  }

  .plan-summary {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .summary-item {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-3);
    border-radius: var(--radius);
    font-size: 12px;
    font-weight: 500;
  }

  .summary-ready {
    background: var(--green-subtle);
    color: var(--green);
  }

  .summary-download {
    background: var(--system-accent-subtle);
    color: var(--system-accent);
  }

  .summary-manual {
    background: var(--yellow-subtle);
    color: var(--yellow);
  }

  .summary-mismatch {
    background: rgba(255, 159, 10, 0.15);
    color: #ff9f0a;
  }

  .summary-count {
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }

  .summary-label {
    font-weight: 500;
  }

  /* ---- Plan Table ---- */

  .plan-table-container {
    background: var(--surface);
    border-radius: var(--radius);
    overflow: hidden;
  }

  .plan-table {
    display: flex;
    flex-direction: column;
  }

  .plan-table-header {
    display: grid;
    grid-template-columns: 1fr 80px 120px;
    padding: var(--space-2) var(--space-3);
    background: var(--bg-secondary);
    border-bottom: 1px solid var(--separator);
    font-size: 11px;
    font-weight: 500;
    color: var(--text-secondary);
  }

  .plan-table-body {
    max-height: 260px;
    overflow-y: auto;
  }

  .plan-table-row {
    display: grid;
    grid-template-columns: 1fr 80px 120px;
    padding: var(--space-2) var(--space-3);
    align-items: center;
    font-size: 13px;
    transition: background var(--duration-fast) var(--ease);
  }

  .plan-table-row:nth-child(even) {
    background: rgba(255, 255, 255, 0.025);
  }

  :global([data-theme="light"]) .plan-table-row:nth-child(even) {
    background: rgba(0, 0, 0, 0.025);
  }

  .plan-table-row:hover {
    background: var(--surface-hover);
  }

  .col-plan-name {
    min-width: 0;
    overflow: hidden;
  }

  .plan-mod-name {
    font-weight: 500;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    display: block;
  }

  .col-plan-version {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .version-text {
    font-size: 12px;
    color: var(--text-secondary);
    font-family: var(--font-mono);
    letter-spacing: 0;
  }

  .version-current {
    font-size: 10px;
    color: #ff9f0a;
  }

  .status-badge {
    display: inline-flex;
    align-items: center;
    padding: 1px 8px;
    border-radius: 100px;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.01em;
  }

  .plan-empty {
    padding: var(--space-6);
    text-align: center;
    font-size: 13px;
    color: var(--text-tertiary);
  }

  /* ---- Step 3: Import Progress ---- */

  .import-progress-section {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
  }

  .progress-icon {
    margin-bottom: var(--space-2);
  }

  .progress-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .progress-detail {
    font-size: 13px;
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
  }

  .progress-bar-track {
    width: 100%;
    max-width: 300px;
    height: 4px;
    background: var(--surface-hover);
    border-radius: 2px;
    overflow: hidden;
  }

  .progress-bar-fill {
    height: 100%;
    background: var(--system-accent);
    border-radius: 2px;
    transition: width var(--duration) var(--ease);
  }

  /* ---- Import Complete ---- */

  .import-complete-section {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    text-align: center;
  }

  .complete-icon {
    margin-bottom: var(--space-1);
  }

  .complete-title {
    font-size: 17px;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .complete-detail {
    font-size: 13px;
    color: var(--text-tertiary);
  }

  .import-errors {
    width: 100%;
    background: var(--red-subtle);
    border-radius: var(--radius);
    padding: var(--space-3);
    margin-top: var(--space-2);
    text-align: left;
  }

  .errors-title {
    font-size: 12px;
    font-weight: 600;
    color: var(--red);
    margin-bottom: var(--space-2);
  }

  .error-item {
    font-size: 12px;
    color: var(--red);
    line-height: 1.4;
    padding: var(--space-1) 0;
  }

  /* ---- Footer ---- */

  .wizard-footer {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-5);
    border-top: 1px solid var(--separator);
    flex-shrink: 0;
  }

  .footer-hint {
    flex: 1;
    font-size: 12px;
    color: var(--text-tertiary);
  }

  /* ---- Buttons ---- */

  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: var(--space-2) var(--space-4);
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

  .btn-accent {
    background: var(--system-accent);
    color: white;
  }

  .btn-accent:hover:not(:disabled) {
    filter: brightness(1.1);
    box-shadow: 0 1px 6px rgba(0, 122, 255, 0.25);
  }

  .btn-ghost {
    background: transparent;
    color: var(--text-secondary);
  }

  .btn-ghost:hover:not(:disabled) {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  /* ---- Spinners ---- */

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

  .spinner-lg {
    width: 36px;
    height: 36px;
    border: 3px solid var(--separator-opaque);
    border-top-color: var(--system-accent);
    border-radius: 50%;
    animation: spin 0.75s linear infinite;
  }

  .loading-text {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-tertiary);
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  @keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  @keyframes dialogIn {
    from { transform: scale(0.95); opacity: 0; }
    to { transform: scale(1); opacity: 1; }
  }
</style>
