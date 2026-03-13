<script lang="ts">
  import {
    scanShaderCompatibility,
    discoverShaderSwapOptions,
    executeShaderConversion,
    revertShaderConversion,
    isNexusPremium
  } from '$lib/api';
  import { listen } from '@tauri-apps/api/event';
  import type {
    CsDetectedMod,
    CsDetectionReason,
    CsModAction,
    ShaderScanResult,
    ConversionConfig,
    ConversionProgress,
    ConversionResult,
    EnbPresetChoice
  } from '$lib/types';

  // ---- Props ----

  interface Props {
    gameId: string;
    bottleName: string;
    onComplete: () => void;
    onCancel: () => void;
  }

  let { gameId, bottleName, onComplete, onCancel }: Props = $props();

  // ---- Constants ----

  const STEP_LABELS = ['Scan', 'Mode', 'ENB Options', 'Find Alternatives', 'Review', 'Execute'];
  const TOTAL_STEPS = 6;

  // ---- State ----

  let wizardEl = $state<HTMLDivElement | null>(null);
  let currentStep = $state(0);

  // Step 1: Scan
  let scanLoading = $state(true);
  let scanError = $state<string | null>(null);
  let scanResult = $state<ShaderScanResult | null>(null);

  // Step 2: Mode
  let conversionMode = $state<'disable' | 'enb'>('enb');

  // Step 3: ENB Options
  let installEnbBinary = $state(true);
  let enbPreset = $state<EnbPresetChoice>('balanced');
  let installEnbEcosystem = $state(true);
  let switchToDxvk = $state(false);
  let isMacOS = $state(false);

  // Step 4: Enrichment
  let enrichLoading = $state(false);
  let enrichError = $state<string | null>(null);
  let enrichedMods = $state<CsDetectedMod[]>([]);
  // Step 5: Review confirmation gate
  let confirmConvert = $state(false);

  // Step 5/6: Execution
  let executing = $state(false);
  let executionDone = $state(false);
  let executionError = $state<string | null>(null);
  let conversionProgress = $state<ConversionProgress | null>(null);
  let conversionResult = $state<ConversionResult | null>(null);
  let reverting = $state(false);
  let revertError = $state<string | null>(null);

  // NexusMods premium status
  let isPremium = $state(false);

  // Event listener cleanup
  let unlisten: (() => void) | null = null;

  // ---- Derived ----

  const effectiveSteps = $derived(
    conversionMode === 'disable'
      ? [0, 1, 4, 5] // skip ENB options (step 2) and enrichment (step 3)
      : [0, 1, 2, 3, 4, 5]
  );

  const effectiveStepIndex = $derived(effectiveSteps.indexOf(currentStep));
  const effectiveTotalSteps = $derived(effectiveSteps.length);
  const isFirstStep = $derived(currentStep === 0);
  const isLastContentStep = $derived(currentStep === 4); // review is last before execute

  const canProceed = $derived.by(() => {
    if (currentStep === 0) return scanResult !== null && !scanLoading;
    if (currentStep === 1) return true;
    if (currentStep === 2) return true;
    if (currentStep === 3) return enrichedMods.length > 0 && !enrichLoading;
    if (currentStep === 4) return true;
    return false;
  });

  const disableCount = $derived(
    enrichedMods.filter(m => m.action.type === 'disable').length
  );
  const swapCount = $derived(
    enrichedMods.filter(m => m.action.type === 'swap_to_enb_variant').length
  );
  const fomodCount = $derived(
    enrichedMods.filter(m => m.action.type === 'rerun_fomod').length
  );
  const keepCount = $derived(
    enrichedMods.filter(m => m.action.type === 'keep').length
  );

  const progressPercent = $derived.by(() => {
    if (!conversionProgress) return 0;
    const phase = conversionProgress.phase;
    // Phases with counters — scale within the phase's range
    if (conversionProgress.current && conversionProgress.total) {
      const phaseProgress = Math.round((conversionProgress.current / conversionProgress.total) * 100);
      if (phase === 'disabling_mods') return Math.round(5 + (phaseProgress * 0.30));   // 5-35%
      if (phase === 'swapping_mods') return Math.round(40 + (phaseProgress * 0.25));   // 40-65%
      if (phase === 'rerunning_fomods') return Math.round(65 + (phaseProgress * 0.15)); // 65-80%
      if (phase === 'redeploying') return Math.round(82 + (phaseProgress * 0.16));      // 82-98%
      return phaseProgress;
    }
    // Phases without counters — show approximate position
    switch (phase) {
      case 'scanning': return 2;
      case 'disabling_mods': return 5;
      case 'installing_enb': return 38;
      case 'swapping_mods': return 40;
      case 'installing_ecosystem': return 82;
      case 'redeploying': return 88;
      case 'complete': return 100;
      default: return 0;
    }
  });

  const totalChanges = $derived(disableCount + swapCount + fomodCount);

  const hasChanges = $derived(
    enrichedMods.some(m => m.action.type !== 'keep') ||
    (conversionMode === 'enb' && installEnbBinary) ||
    switchToDxvk
  );

  // ---- Lifecycle ----

  $effect(() => {
    if (wizardEl) {
      wizardEl.focus();
    }
  });

  $effect(() => {
    // Detect macOS
    isMacOS = navigator.userAgent.includes('Mac') || navigator.platform.includes('Mac');
    // Check NexusMods premium status
    isNexusPremium().then(v => { isPremium = v; }).catch(() => { isPremium = false; });
    // Auto-scan on mount
    runScan();

    return () => {
      unlisten?.();
    };
  });

  // ---- Step 1: Scan ----

  async function runScan() {
    scanLoading = true;
    scanError = null;
    try {
      scanResult = await scanShaderCompatibility(gameId, bottleName);
      if (scanResult) {
        enrichedMods = scanResult.detected_mods.map((m: CsDetectedMod) => ({ ...m }));
      }
    } catch (err: unknown) {
      scanError = err instanceof Error ? err.message : String(err);
    } finally {
      scanLoading = false;
    }
  }

  // ---- Step 4: Enrichment ----

  async function runEnrichment() {
    enrichLoading = true;
    enrichError = null;
    try {
      const result = await discoverShaderSwapOptions(gameId, bottleName, enrichedMods);
      if (result) {
        enrichedMods = result;
      }
    } catch (err: unknown) {
      enrichError = err instanceof Error ? err.message : String(err);
    } finally {
      enrichLoading = false;
    }
  }

  // ---- Step 6: Execute ----

  async function runConversion() {
    executing = true;
    executionDone = false;
    executionError = null;
    conversionProgress = null;
    conversionResult = null;

    // Set up event listener BEFORE starting backend command
    // so we don't miss early events
    const fn = await listen<ConversionProgress>('shader-conversion-progress', (event) => {
      conversionProgress = event.payload;
      if (event.payload.result) {
        conversionResult = event.payload.result;
        executionDone = true;
        executing = false;
      }
      if (event.payload.error) {
        executionError = event.payload.error;
        executionDone = true;
        executing = false;
      }
    });
    unlisten = fn;

    const config: ConversionConfig = {
      install_enb_binary: conversionMode === 'enb' ? installEnbBinary : false,
      enb_preset: conversionMode === 'enb' ? enbPreset : null,
      mod_actions: enrichedMods.map(m => [m.mod_id, m.action]),
      install_enb_ecosystem: conversionMode === 'enb' ? installEnbEcosystem : false,
      switch_to_dxvk: conversionMode === 'enb' ? switchToDxvk : false
    };

    try {
      await executeShaderConversion(gameId, bottleName, config);
      // If we got here without event-based completion, mark done
      if (!executionDone && !executionError) {
        executionDone = true;
        executing = false;
      }
    } catch (err: unknown) {
      executionError = err instanceof Error ? err.message : String(err);
      executionDone = true;
      executing = false;
    } finally {
      unlisten?.();
      unlisten = null;
    }
  }

  async function handleRevert() {
    if (!conversionResult) return;
    reverting = true;
    revertError = null;
    try {
      await revertShaderConversion(gameId, bottleName, conversionResult.conversion_id);
      onComplete();
    } catch (err: unknown) {
      revertError = err instanceof Error ? err.message : String(err);
    } finally {
      reverting = false;
    }
  }

  // ---- Navigation ----

  function nextStep() {
    if (currentStep === 4) {
      // Move to execute step and begin
      currentStep = 5;
      runConversion();
      return;
    }

    const idx = effectiveSteps.indexOf(currentStep);
    if (idx < effectiveSteps.length - 1) {
      const next = effectiveSteps[idx + 1];
      currentStep = next;

      // Auto-run enrichment when entering step 4
      if (next === 3 && enrichedMods.length > 0 && !enrichLoading) {
        runEnrichment();
      }
    }
  }

  function prevStep() {
    confirmConvert = false;
    const idx = effectiveSteps.indexOf(currentStep);
    if (idx > 0) {
      currentStep = effectiveSteps[idx - 1];
    }
  }

  function goToStep(step: number) {
    confirmConvert = false;
    const idx = effectiveSteps.indexOf(step);
    const currentIdx = effectiveSteps.indexOf(currentStep);
    if (idx >= 0 && idx <= currentIdx) {
      currentStep = step;
    }
  }

  // ---- Action Override ----

  function setModAction(modId: number, action: CsModAction) {
    enrichedMods = enrichedMods.map(m =>
      m.mod_id === modId ? { ...m, action } : m
    );
  }

  // ---- Keyboard ----

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      if (executing || enrichLoading) return; // Don't close during execution or enrichment
      onCancel();
    }
  }

  // ---- Helpers ----

  function reasonIcon(reason: CsDetectionReason): string {
    switch (reason) {
      case 'core_dll': return '\u{1F9E9}';
      case 'cs_config_files': return '\u{2699}\u{FE0F}';
      case 'light_placer_configs': return '\u{1F4A1}';
      case 'pbr_textures': return '\u{1F3A8}';
      case 'fomod_cs_selection': return '\u{1F4E6}';
      case 'known_cs_ecosystem_mod': return '\u{1F50C}';
      case 'cs_only_files': return '\u{1F4C4}';
      default: return '\u{2753}';
    }
  }

  function reasonLabel(reason: CsDetectionReason): string {
    switch (reason) {
      case 'core_dll': return 'Core DLL';
      case 'cs_config_files': return 'CS Config';
      case 'light_placer_configs': return 'Light Placer';
      case 'pbr_textures': return 'PBR Textures';
      case 'fomod_cs_selection': return 'FOMOD Selection';
      case 'known_cs_ecosystem_mod': return 'CS Ecosystem';
      case 'cs_only_files': return 'CS-Only Files';
      default: return reason;
    }
  }

  function actionLabel(action: CsModAction): string {
    switch (action.type) {
      case 'disable': return 'Disable';
      case 'swap_to_enb_variant': return 'Swap to ENB';
      case 'rerun_fomod': return 'Re-run FOMOD';
      case 'keep': return 'Safe to keep';
      default: return 'Unknown';
    }
  }

  function actionBadgeClass(action: CsModAction): string {
    switch (action.type) {
      case 'disable': return 'badge-red';
      case 'swap_to_enb_variant': return 'badge-blue';
      case 'rerun_fomod': return 'badge-yellow';
      case 'keep': return 'badge-green';
      default: return '';
    }
  }

  function presetDescription(preset: string): string {
    switch (preset) {
      case 'performance': return 'Minimal visual overhead. Best for lower-end hardware or maximizing FPS.';
      case 'balanced': return 'Good visual quality with reasonable performance cost. Recommended for most setups.';
      case 'quality': return 'Maximum visual fidelity. Best for high-end hardware with DXVK.';
      default: return '';
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="shader-overlay" onclick={() => { if (!executing && !enrichLoading) onCancel(); }} onkeydown={handleKeydown}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="shader-wizard" role="dialog" aria-modal="true" aria-labelledby="shader-wizard-title" bind:this={wizardEl} tabindex="-1" onclick={(e) => e.stopPropagation()}>

    <!-- Header -->
    <div class="wizard-header">
      <div class="header-top">
        <h2 id="shader-wizard-title" class="wizard-title">Shader Conversion Wizard</h2>
        {#if !executing}
          <button class="close-btn" onclick={onCancel} aria-label="Close" type="button">
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
              <line x1="2" y1="2" x2="10" y2="10" />
              <line x1="10" y1="2" x2="2" y2="10" />
            </svg>
          </button>
        {/if}
      </div>

      <!-- Step dots -->
      <div class="step-progress">
        <div class="step-dots">
          {#each effectiveSteps as stepNum, i}
            {@const isActive = stepNum === currentStep}
            {@const isComplete = effectiveSteps.indexOf(currentStep) > i}
            <button
              class="step-dot"
              class:step-dot-active={isActive}
              class:step-dot-complete={isComplete}
              disabled={!isComplete}
              onclick={() => goToStep(stepNum)}
              aria-label="Step {i + 1}: {STEP_LABELS[stepNum]}"
              type="button"
            >
              {#if isComplete}
                <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M20 6L9 17l-5-5" />
                </svg>
              {:else}
                {i + 1}
              {/if}
            </button>
            {#if i < effectiveTotalSteps - 1}
              <div class="step-connector" class:step-connector-complete={isComplete}></div>
            {/if}
          {/each}
        </div>
        <span class="step-label">Step {effectiveStepIndex + 1} of {effectiveTotalSteps}</span>
      </div>
    </div>

    <!-- Step name bar -->
    <div class="step-name-bar">
      <span class="step-name">{STEP_LABELS[currentStep]}</span>
    </div>

    <!-- Content -->
    <div class="wizard-content">

      <!-- ============ STEP 0: SCAN ============ -->
      {#if currentStep === 0}
        {#if scanLoading}
          <div class="wizard-loading">
            <div class="spinner"></div>
            <p class="loading-text">Scanning for Community Shaders mods...</p>
          </div>
        {:else if scanError}
          <div class="error-card">
            <div class="error-icon">
              <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="var(--red)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <circle cx="12" cy="12" r="10" />
                <line x1="15" y1="9" x2="9" y2="15" />
                <line x1="9" y1="9" x2="15" y2="15" />
              </svg>
            </div>
            <div class="error-body">
              <p class="error-title">Scan Failed</p>
              <p class="error-message">{scanError}</p>
            </div>
            <button class="btn btn-secondary" onclick={runScan} type="button">Retry</button>
          </div>
        {:else if scanResult && scanResult.total_cs_mods === 0}
          <div class="empty-state">
            <div class="empty-icon">
              <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="var(--green)" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14" />
                <polyline points="22 4 12 14.01 9 11.01" />
              </svg>
            </div>
            <h3 class="empty-title">No Community Shaders mods detected</h3>
            <p class="empty-subtitle">Your mod setup does not appear to include Community Shaders dependencies.</p>
          </div>
        {:else if scanResult}
          <div class="scan-summary">
            <div class="stat-row">
              <div class="stat-card">
                <span class="stat-value">{scanResult.total_cs_mods}</span>
                <span class="stat-label">CS Mods Found</span>
              </div>
              <div class="stat-card">
                <span class="stat-value stat-blue">{scanResult.swappable_count}</span>
                <span class="stat-label">Swappable</span>
              </div>
              <div class="stat-card">
                <span class="stat-value stat-yellow">{scanResult.fomod_rerun_count}</span>
                <span class="stat-label">FOMOD Re-run</span>
              </div>
              <div class="stat-card">
                <span class="stat-value stat-red">{scanResult.disable_only_count}</span>
                <span class="stat-label">Disable Only</span>
              </div>
              {#if scanResult.keep_count > 0}
                <div class="stat-card">
                  <span class="stat-value stat-green">{scanResult.keep_count}</span>
                  <span class="stat-label">Safe to Keep</span>
                </div>
              {/if}
            </div>

            {#if scanResult.enb_already_installed}
              <div class="info-banner info-banner-blue">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <circle cx="12" cy="12" r="10" />
                  <line x1="12" y1="16" x2="12" y2="12" />
                  <line x1="12" y1="8" x2="12.01" y2="8" />
                </svg>
                <span>ENB binary is already installed in this game directory.</span>
              </div>
            {/if}

            <div class="mod-list">
              <div class="mod-list-header">
                <span>Detected Community Shaders Mods</span>
              </div>
              {#each scanResult.detected_mods as mod (mod.mod_id)}
                <div class="mod-list-row">
                  <div class="mod-info">
                    <span class="mod-name">{mod.mod_name}</span>
                    <div class="reason-badges">
                      {#each mod.reasons as reason}
                        <span class="reason-badge" title={reasonLabel(reason)}>
                          <span class="reason-icon">{reasonIcon(reason)}</span>
                          {reasonLabel(reason)}
                        </span>
                      {/each}
                    </div>
                  </div>
                  <span class="badge {actionBadgeClass(mod.action)}">{actionLabel(mod.action)}</span>
                </div>
              {/each}
            </div>
          </div>
        {/if}

      <!-- ============ STEP 1: CHOOSE MODE ============ -->
      {:else if currentStep === 1}
        <div class="mode-selection">
          <p class="mode-intro">Choose how to handle Community Shaders dependencies:</p>

          <button
            class="mode-card"
            class:mode-card-selected={conversionMode === 'disable'}
            onclick={() => { conversionMode = 'disable'; }}
            type="button"
          >
            <div class="mode-radio">
              <div class="radio" class:radio-checked={conversionMode === 'disable'}>
                {#if conversionMode === 'disable'}
                  <div class="radio-dot"></div>
                {/if}
              </div>
            </div>
            <div class="mode-body">
              <h3 class="mode-title">Disable CS Only</h3>
              <p class="mode-description">
                Remove Community Shaders mods and use vanilla lighting. Simpler setup with no
                additional renderer overhead. Best if you prefer stability over visual enhancements.
              </p>
            </div>
          </button>

          <button
            class="mode-card"
            class:mode-card-selected={conversionMode === 'enb'}
            onclick={() => { conversionMode = 'enb'; }}
            type="button"
          >
            <div class="mode-radio">
              <div class="radio" class:radio-checked={conversionMode === 'enb'}>
                {#if conversionMode === 'enb'}
                  <div class="radio-dot"></div>
                {/if}
              </div>
            </div>
            <div class="mode-body">
              <h3 class="mode-title">Convert to ENB</h3>
              <p class="mode-description">
                Replace Community Shaders with ENB for enhanced visuals. Swap compatible mods to
                their ENB variants where available. Requires DXVK on macOS for DirectX translation.
              </p>
              {#if isMacOS}
                <div class="mode-warning">
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--yellow)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
                    <line x1="12" y1="9" x2="12" y2="13" />
                    <line x1="12" y1="17" x2="12.01" y2="17" />
                  </svg>
                  <span>macOS detected: ENB requires DXVK which may have a performance tradeoff compared to native D3D translation.</span>
                </div>
              {/if}
            </div>
          </button>
        </div>

      <!-- ============ STEP 2: ENB OPTIONS ============ -->
      {:else if currentStep === 2}
        <div class="enb-options">
          <!-- ENB Binary -->
          <div class="option-section">
            <div class="option-toggle-row">
              <div class="option-toggle-info">
                <h3 class="option-toggle-title">Install ENB Binary</h3>
                <p class="option-toggle-desc">Download and install the latest ENB binary from enbdev.com into the game directory.</p>
              </div>
              <button
                class="toggle-switch"
                class:toggle-on={installEnbBinary}
                onclick={() => { installEnbBinary = !installEnbBinary; }}
                type="button"
                role="switch"
                aria-checked={installEnbBinary}
              >
                <div class="toggle-knob"></div>
              </button>
            </div>
          </div>

          <!-- ENB Preset -->
          <div class="option-section">
            <h3 class="option-section-title">ENB Preset</h3>
            <div class="preset-cards">
              {#each ['performance', 'balanced', 'quality'] as preset}
                <button
                  class="preset-card"
                  class:preset-card-selected={enbPreset === preset}
                  onclick={() => { enbPreset = preset as EnbPresetChoice; }}
                  type="button"
                >
                  <div class="radio" class:radio-checked={enbPreset === preset}>
                    {#if enbPreset === preset}
                      <div class="radio-dot"></div>
                    {/if}
                  </div>
                  <div class="preset-body">
                    <span class="preset-name">{preset.charAt(0).toUpperCase() + preset.slice(1)}</span>
                    <span class="preset-desc">{presetDescription(preset)}</span>
                  </div>
                </button>
              {/each}
            </div>
          </div>

          <!-- ENB Ecosystem -->
          <div class="option-section">
            <div class="option-toggle-row">
              <div class="option-toggle-info">
                <h3 class="option-toggle-title">Install ENB Ecosystem Mods</h3>
                <p class="option-toggle-desc">Install ENB Helper SE and ENB Light for better compatibility with ENB presets.</p>
              </div>
              <button
                class="toggle-switch"
                class:toggle-on={installEnbEcosystem}
                onclick={() => { installEnbEcosystem = !installEnbEcosystem; }}
                type="button"
                role="switch"
                aria-checked={installEnbEcosystem}
              >
                <div class="toggle-knob"></div>
              </button>
            </div>
          </div>

          <!-- DXVK Switch (macOS only) -->
          {#if isMacOS}
            <div class="option-section">
              <div class="option-toggle-row">
                <div class="option-toggle-info">
                  <h3 class="option-toggle-title">Auto-switch to DXVK</h3>
                  <p class="option-toggle-desc">
                    Configure the Wine bottle to use DXVK for DirectX translation.
                    Required for ENB on macOS.
                  </p>
                  <div class="mode-warning" style="margin-top: 6px;">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--yellow)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
                      <line x1="12" y1="9" x2="12" y2="13" />
                      <line x1="12" y1="17" x2="12.01" y2="17" />
                    </svg>
                    <span>DXVK may reduce performance on some configurations. A snapshot will be created for easy reversal.</span>
                  </div>
                </div>
                <button
                  class="toggle-switch"
                  class:toggle-on={switchToDxvk}
                  onclick={() => { switchToDxvk = !switchToDxvk; }}
                  type="button"
                  role="switch"
                  aria-checked={switchToDxvk}
                >
                  <div class="toggle-knob"></div>
                </button>
              </div>
            </div>
          {/if}
        </div>

      <!-- ============ STEP 3: API ENRICHMENT ============ -->
      {:else if currentStep === 3}
        {#if enrichLoading}
          <div class="wizard-loading">
            <div class="spinner"></div>
            <p class="loading-text">Checking NexusMods for ENB variants...</p>
          </div>
        {:else if enrichError}
          <div class="error-card">
            <div class="error-icon">
              <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="var(--red)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <circle cx="12" cy="12" r="10" />
                <line x1="15" y1="9" x2="9" y2="15" />
                <line x1="9" y1="9" x2="15" y2="15" />
              </svg>
            </div>
            <div class="error-body">
              <p class="error-title">Enrichment Failed</p>
              <p class="error-message">{enrichError}</p>
            </div>
            <button class="btn btn-secondary" onclick={runEnrichment} type="button">Retry</button>
          </div>
        {:else}
          <div class="enrichment-results">
            {#if !isPremium}
              <div class="info-banner info-banner-yellow">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <circle cx="12" cy="12" r="10" />
                  <line x1="12" y1="16" x2="12" y2="12" />
                  <line x1="12" y1="8" x2="12.01" y2="8" />
                </svg>
                <span>NexusMods Premium is required to swap mod files. Non-premium users can only disable CS mods.</span>
              </div>
            {/if}
            <div class="mod-list">
              <div class="mod-list-header enrichment-header">
                <span class="col-name">Mod</span>
                <span class="col-action">Action</span>
              </div>
              {#each enrichedMods as mod (mod.mod_id)}
                <div class="mod-list-row enrichment-row">
                  <div class="mod-info">
                    <span class="mod-name">{mod.mod_name}</span>
                    <div class="reason-badges">
                      {#each mod.reasons as reason}
                        <span class="reason-badge" title={reasonLabel(reason)}>
                          <span class="reason-icon">{reasonIcon(reason)}</span>
                          {reasonLabel(reason)}
                        </span>
                      {/each}
                    </div>
                  </div>
                  <div class="action-select-wrapper">
                    <select
                      class="action-select"
                      value={mod.action.type}
                      onchange={(e) => {
                        const target = e.target as HTMLSelectElement;
                        const val = target.value;
                        if (val === 'disable') {
                          setModAction(mod.mod_id, { type: 'disable' });
                        } else if (val === 'keep') {
                          setModAction(mod.mod_id, { type: 'keep' });
                        } else if (val === 'rerun_fomod') {
                          setModAction(mod.mod_id, { type: 'rerun_fomod', suggested_selections: {} });
                        } else if (val === 'swap_to_enb_variant' && mod.action.type === 'swap_to_enb_variant') {
                          // Keep existing swap details
                          setModAction(mod.mod_id, mod.action);
                        }
                      }}
                    >
                      <option value="disable">Disable</option>
                      {#if mod.action.type === 'swap_to_enb_variant'}
                        <option value="swap_to_enb_variant">Swap to ENB</option>
                      {/if}
                      <option value="rerun_fomod">Re-run FOMOD</option>
                      <option value="keep">Keep</option>
                    </select>
                    <span class="badge {actionBadgeClass(mod.action)} badge-small">{actionLabel(mod.action)}</span>
                  </div>
                </div>
              {/each}
            </div>
          </div>
        {/if}

      <!-- ============ STEP 4: REVIEW ============ -->
      {:else if currentStep === 4}
        <div class="review-section">
          <div class="review-summary">
            <h3 class="review-heading">Conversion Summary</h3>

            <div class="review-items">
              {#if disableCount > 0}
                <div class="review-item">
                  <div class="review-item-icon review-icon-red">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <circle cx="12" cy="12" r="10" />
                      <line x1="4.93" y1="4.93" x2="19.07" y2="19.07" />
                    </svg>
                  </div>
                  <span>Will disable <strong>{disableCount}</strong> mod{disableCount !== 1 ? 's' : ''}</span>
                </div>
              {/if}
              {#if swapCount > 0}
                <div class="review-item">
                  <div class="review-item-icon review-icon-blue">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <polyline points="17 1 21 5 17 9" />
                      <path d="M3 11V9a4 4 0 0 1 4-4h14" />
                      <polyline points="7 23 3 19 7 15" />
                      <path d="M21 13v2a4 4 0 0 1-4 4H3" />
                    </svg>
                  </div>
                  <span>Will swap <strong>{swapCount}</strong> mod{swapCount !== 1 ? 's' : ''} to ENB variants</span>
                </div>
              {/if}
              {#if fomodCount > 0}
                <div class="review-item">
                  <div class="review-item-icon review-icon-yellow">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z" />
                    </svg>
                  </div>
                  <span>Will re-run <strong>{fomodCount}</strong> FOMOD installer{fomodCount !== 1 ? 's' : ''}</span>
                </div>
              {/if}
              {#if keepCount > 0}
                <div class="review-item">
                  <div class="review-item-icon review-icon-green">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <polyline points="20 6 9 17 4 12" />
                    </svg>
                  </div>
                  <span>Will keep <strong>{keepCount}</strong> mod{keepCount !== 1 ? 's' : ''} unchanged</span>
                </div>
              {/if}

              {#if conversionMode === 'enb'}
                {#if installEnbBinary}
                  <div class="review-item">
                    <div class="review-item-icon review-icon-purple">
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                        <polyline points="7 10 12 15 17 10" />
                        <line x1="12" y1="15" x2="12" y2="3" />
                      </svg>
                    </div>
                    <span>Will install ENB binary</span>
                  </div>
                {/if}
                {#if switchToDxvk}
                  <div class="review-item">
                    <div class="review-item-icon review-icon-yellow">
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <rect x="2" y="3" width="20" height="14" rx="2" ry="2" />
                        <line x1="8" y1="21" x2="16" y2="21" />
                        <line x1="12" y1="17" x2="12" y2="21" />
                      </svg>
                    </div>
                    <span>Will switch renderer to DXVK</span>
                  </div>
                {/if}
              {/if}
            </div>
          </div>

          {#if !isPremium && swapCount > 0}
            <div class="info-banner info-banner-yellow">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
                <line x1="12" y1="9" x2="12" y2="13" />
                <line x1="12" y1="17" x2="12.01" y2="17" />
              </svg>
              <span>You have {swapCount} mod{swapCount !== 1 ? 's' : ''} set to swap, but NexusMods Premium is required for automated downloads. These mods will be disabled instead. To swap, upgrade to NexusMods Premium.</span>
            </div>
          {/if}

          <div class="info-banner info-banner-green">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
            </svg>
            <span>A snapshot will be created before any changes for easy reversal.</span>
          </div>
        </div>

      <!-- ============ STEP 5: EXECUTE ============ -->
      {:else if currentStep === 5}
        <div class="execution-section">
          {#if executing || (!executionDone && !executionError)}
            <div class="execution-progress">
              {#if conversionProgress}
                <div class="phase-label">{
                  conversionProgress.phase === 'scanning' ? 'Scanning mods...' :
                  conversionProgress.phase === 'disabling_mods' ? 'Disabling CS Mods' :
                  conversionProgress.phase === 'installing_enb' ? 'Installing ENB' :
                  conversionProgress.phase === 'swapping_mods' ? 'Swapping to ENB variants' :
                  conversionProgress.phase === 'rerunning_fomods' ? 'Re-running FOMOD installers' :
                  conversionProgress.phase === 'installing_ecosystem' ? 'Installing ENB ecosystem' :
                  conversionProgress.phase === 'redeploying' ? `Redeploying mods${conversionProgress.current && conversionProgress.total ? ` (${conversionProgress.current}/${conversionProgress.total})` : ''}...` :
                  conversionProgress.phase === 'complete' ? 'Complete!' :
                  conversionProgress.phase
                }</div>
                {#if conversionProgress.message}
                  <p class="phase-message">{conversionProgress.message}</p>
                {/if}
                <div class="progress-bar-container">
                  <div class="progress-bar">
                    <div class="progress-bar-fill" style="width: {progressPercent}%"></div>
                  </div>
                  <span class="progress-text">{progressPercent}%</span>
                </div>
                {#if conversionProgress.mod_name}
                  <div class="current-mod">
                    <span class="current-mod-label">Processing:</span>
                    <span class="current-mod-name">{conversionProgress.mod_name}</span>
                    {#if conversionProgress.step}
                      <span class="badge badge-blue badge-small">{conversionProgress.step}</span>
                    {/if}
                  </div>
                {/if}
              {:else}
                <div class="wizard-loading">
                  <div class="spinner"></div>
                  <p class="loading-text">Starting conversion...</p>
                </div>
              {/if}
            </div>

          {:else if executionError}
            <div class="error-card">
              <div class="error-icon">
                <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="var(--red)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <circle cx="12" cy="12" r="10" />
                  <line x1="15" y1="9" x2="9" y2="15" />
                  <line x1="9" y1="9" x2="15" y2="15" />
                </svg>
              </div>
              <div class="error-body">
                <p class="error-title">Conversion Failed</p>
                <p class="error-message">{executionError}</p>
              </div>
            </div>

            {#if conversionResult}
              <div class="revert-section">
                <p class="revert-prompt">Would you like to revert to the pre-conversion snapshot?</p>
                <button
                  class="btn btn-danger"
                  onclick={handleRevert}
                  disabled={reverting}
                  type="button"
                >
                  {#if reverting}
                    <div class="spinner spinner-small"></div>
                    Reverting...
                  {:else}
                    Revert Changes
                  {/if}
                </button>
                {#if revertError}
                  <p class="error-message" style="margin-top: 8px;">Revert failed: {revertError}</p>
                {/if}
              </div>
            {/if}

          {:else if executionDone && conversionResult}
            <div class="completion-section">
              <div class="completion-icon">
                <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="var(--green)" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14" />
                  <polyline points="22 4 12 14.01 9 11.01" />
                </svg>
              </div>
              <h3 class="completion-title">Conversion Complete</h3>

              <div class="completion-stats">
                {#if conversionResult.mods_disabled > 0}
                  <div class="completion-stat">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--green)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <polyline points="20 6 9 17 4 12" />
                    </svg>
                    <span>{conversionResult.mods_disabled} mod{conversionResult.mods_disabled !== 1 ? 's' : ''} disabled</span>
                  </div>
                {/if}
                {#if conversionResult.mods_swapped > 0}
                  <div class="completion-stat">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--green)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <polyline points="20 6 9 17 4 12" />
                    </svg>
                    <span>{conversionResult.mods_swapped} mod{conversionResult.mods_swapped !== 1 ? 's' : ''} swapped to ENB variants</span>
                  </div>
                {/if}
                {#if conversionResult.fomods_rerun > 0}
                  <div class="completion-stat">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--green)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <polyline points="20 6 9 17 4 12" />
                    </svg>
                    <span>{conversionResult.fomods_rerun} FOMOD installer{conversionResult.fomods_rerun !== 1 ? 's' : ''} re-run</span>
                  </div>
                {/if}
                {#if conversionResult.enb_installed}
                  <div class="completion-stat">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--green)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <polyline points="20 6 9 17 4 12" />
                    </svg>
                    <span>ENB binary installed</span>
                  </div>
                {/if}
                {#if conversionResult.dxvk_switched}
                  <div class="completion-stat">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--green)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <polyline points="20 6 9 17 4 12" />
                    </svg>
                    <span>Renderer switched to DXVK</span>
                  </div>
                {/if}
              </div>

              {#if conversionResult.errors.length > 0}
                <div class="completion-warnings">
                  <h4 class="warnings-title">Warnings ({conversionResult.errors.length})</h4>
                  {#each conversionResult.errors as warning}
                    <div class="warning-row">
                      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="var(--yellow)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
                        <line x1="12" y1="9" x2="12" y2="13" />
                        <line x1="12" y1="17" x2="12.01" y2="17" />
                      </svg>
                      <span>{warning}</span>
                    </div>
                  {/each}
                </div>
              {/if}

              <p class="completion-snapshot-note">
                Snapshot #{conversionResult.snapshot_id} created. You can revert this conversion
                from Settings &gt; Game &gt; Snapshots at any time.
              </p>
            </div>
          {/if}
        </div>
      {/if}
    </div>

    <!-- Footer -->
    <div class="wizard-footer">
      {#if currentStep === 5}
        {#if executionDone && !executionError}
          <div></div>
          <button class="btn btn-accent" onclick={onComplete} type="button">
            Done
          </button>
        {:else if executionError}
          <button class="btn btn-ghost" onclick={onCancel} type="button">Close</button>
          <div></div>
        {:else}
          <div></div>
          <span class="footer-hint">Conversion in progress...</span>
        {/if}
      {:else}
        <button class="btn btn-ghost" onclick={onCancel} type="button">Cancel</button>
        <div class="footer-nav">
          {#if !isFirstStep}
            <button class="btn btn-secondary" onclick={prevStep} type="button">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="15 18 9 12 15 6" />
              </svg>
              Back
            </button>
          {/if}
          {#if currentStep === 0 && scanResult && scanResult.total_cs_mods === 0}
            <button class="btn btn-accent" onclick={onComplete} type="button">
              Done
            </button>
          {:else if currentStep === 4}
            {#if !hasChanges}
              <span class="footer-hint">No changes configured. All mods will be kept as-is.</span>
            {:else if !confirmConvert}
              <button
                class="btn btn-accent"
                onclick={() => { confirmConvert = true; }}
                disabled={!canProceed}
                type="button"
              >
                Confirm & Convert
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <polyline points="9 18 15 12 9 6" />
                </svg>
              </button>
            {:else}
              <div class="confirm-inline">
                <p>This will modify {totalChanges} mod{totalChanges !== 1 ? 's' : ''} and your game directory. A snapshot will be created first for safety.</p>
                <div class="confirm-buttons">
                  <button class="btn btn-secondary" onclick={() => { confirmConvert = false; }} type="button">Cancel</button>
                  <button class="btn btn-accent" onclick={() => { confirmConvert = false; nextStep(); }} type="button">Begin Conversion</button>
                </div>
              </div>
            {/if}
          {:else}
            <button
              class="btn btn-accent"
              onclick={nextStep}
              disabled={!canProceed}
              type="button"
            >
              Next
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="9 18 15 12 9 6" />
              </svg>
            </button>
          {/if}
        </div>
      {/if}
    </div>
  </div>
</div>

<style>
  /* ---- Overlay ---- */

  .shader-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    backdrop-filter: var(--glass-blur-light);
    -webkit-backdrop-filter: var(--glass-blur-light);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 2000;
    animation: fadeIn var(--duration) var(--ease);
  }

  @keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  /* ---- Wizard Container ---- */

  .shader-wizard {
    background: color-mix(in srgb, var(--bg-elevated) 75%, transparent);
    backdrop-filter: blur(40px) saturate(1.5);
    -webkit-backdrop-filter: blur(40px) saturate(1.5);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: var(--radius-xl);
    box-shadow: var(--glass-refraction),
                var(--glass-edge-shadow),
                var(--shadow-lg);
    width: 760px;
    max-width: calc(100vw - var(--space-8));
    max-height: 85vh;
    display: flex;
    flex-direction: column;
    animation: dialogIn 0.25s var(--ease-out);
    overflow: hidden;
  }

  @keyframes dialogIn {
    from {
      transform: translateY(8px) scale(0.98);
      opacity: 0;
    }
    to {
      transform: translateY(0) scale(1);
      opacity: 1;
    }
  }

  /* ---- Header ---- */

  .wizard-header {
    padding: var(--space-5) var(--space-6) var(--space-3);
    border-bottom: 1px solid var(--separator);
    flex-shrink: 0;
  }

  .header-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .wizard-title {
    font-size: 17px;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
    flex: 1;
    min-width: 0;
  }

  .close-btn {
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    color: var(--text-tertiary);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
    flex-shrink: 0;
    background: none;
    border: none;
  }

  .close-btn:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  /* ---- Step Progress ---- */

  .step-progress {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-top: var(--space-3);
    gap: var(--space-3);
  }

  .step-dots {
    display: flex;
    align-items: center;
    gap: 0;
  }

  .step-dot {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    background: var(--surface-hover);
    color: var(--text-tertiary);
    font-size: 10px;
    font-weight: 700;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    border: none;
    cursor: pointer;
    transition: all var(--duration) var(--ease);
  }

  .step-dot:disabled {
    cursor: default;
  }

  .step-dot-active {
    background: var(--system-accent);
    color: var(--system-accent-on);
  }

  .step-dot-complete {
    background: var(--green);
    color: white;
  }

  .step-connector {
    width: 16px;
    height: 2px;
    background: var(--separator-opaque);
    transition: background var(--duration) var(--ease);
  }

  .step-connector-complete {
    background: var(--green);
  }

  .step-label {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-tertiary);
    flex-shrink: 0;
    font-variant-numeric: tabular-nums;
  }

  /* ---- Step Name Bar ---- */

  .step-name-bar {
    padding: var(--space-3) var(--space-6);
    background: var(--surface);
    border-bottom: 1px solid var(--separator);
    flex-shrink: 0;
  }

  .step-name {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-secondary);
    letter-spacing: -0.005em;
  }

  /* ---- Content ---- */

  .wizard-content {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-5) var(--space-6);
    scroll-behavior: smooth;
  }

  /* ---- Loading ---- */

  .wizard-loading {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    padding: var(--space-12);
  }

  .spinner {
    width: 28px;
    height: 28px;
    border: 2.5px solid var(--separator-opaque);
    border-top-color: var(--system-accent);
    border-radius: 50%;
    animation: spin 0.75s linear infinite;
  }

  .spinner-small {
    width: 14px;
    height: 14px;
    border-width: 2px;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .loading-text {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-tertiary);
  }

  /* ---- Error Card ---- */

  .error-card {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-4);
    background: var(--surface);
    border: 1px solid color-mix(in srgb, var(--red) 30%, transparent);
    border-radius: var(--radius);
  }

  .error-icon {
    flex-shrink: 0;
    margin-top: 2px;
  }

  .error-body {
    flex: 1;
    min-width: 0;
  }

  .error-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--red);
    margin-bottom: 4px;
  }

  .error-message {
    font-size: 12px;
    color: var(--text-tertiary);
    line-height: 1.4;
    word-break: break-word;
  }

  /* ---- Empty State ---- */

  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    padding: var(--space-12) var(--space-6);
    text-align: center;
  }

  .empty-icon {
    margin-bottom: var(--space-2);
  }

  .empty-title {
    font-size: 16px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .empty-subtitle {
    font-size: 13px;
    color: var(--text-tertiary);
    max-width: 360px;
  }

  /* ---- Scan Summary ---- */

  .scan-summary {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .stat-row {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: var(--space-3);
  }

  .stat-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding: var(--space-3) var(--space-2);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
  }

  .stat-value {
    font-size: 24px;
    font-weight: 700;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .stat-blue { color: var(--blue); }
  .stat-yellow { color: var(--yellow); }
  .stat-red { color: var(--red); }
  .stat-green { color: var(--green); }

  .stat-label {
    font-size: 11px;
    font-weight: 500;
    color: var(--text-tertiary);
    text-align: center;
  }

  /* ---- Info Banner ---- */

  .info-banner {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius);
    font-size: 12px;
    font-weight: 500;
    line-height: 1.4;
  }

  .info-banner svg {
    flex-shrink: 0;
  }

  .info-banner-blue {
    background: color-mix(in srgb, var(--blue) 10%, transparent);
    border: 1px solid color-mix(in srgb, var(--blue) 25%, transparent);
    color: var(--blue);
  }

  .info-banner-green {
    background: color-mix(in srgb, var(--green) 10%, transparent);
    border: 1px solid color-mix(in srgb, var(--green) 25%, transparent);
    color: var(--green);
  }

  .info-banner-yellow {
    background: color-mix(in srgb, var(--yellow) 10%, transparent);
    border: 1px solid color-mix(in srgb, var(--yellow) 25%, transparent);
    color: var(--yellow);
  }

  /* ---- Mod List ---- */

  .mod-list {
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    overflow: hidden;
  }

  .mod-list-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-2) var(--space-4);
    background: var(--surface);
    border-bottom: 1px solid var(--separator);
    font-size: 11px;
    font-weight: 600;
    color: var(--text-tertiary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .mod-list-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--separator);
    transition: background var(--duration-fast) var(--ease);
  }

  .mod-list-row:last-child {
    border-bottom: none;
  }

  .mod-list-row:hover {
    background: var(--surface-hover);
  }

  .mod-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .mod-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .reason-badges {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }

  .reason-badge {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 10px;
    font-weight: 500;
    color: var(--text-tertiary);
    background: var(--surface);
    padding: 1px 6px;
    border-radius: 3px;
    border: 1px solid var(--separator);
  }

  .reason-icon {
    font-size: 10px;
    line-height: 1;
  }

  /* ---- Badges ---- */

  .badge {
    display: inline-flex;
    align-items: center;
    font-size: 11px;
    font-weight: 600;
    padding: 2px 8px;
    border-radius: 4px;
    white-space: nowrap;
  }

  .badge-small {
    font-size: 10px;
    padding: 1px 6px;
  }

  .badge-green {
    background: color-mix(in srgb, var(--green) 15%, transparent);
    color: var(--green);
  }

  .badge-blue {
    background: color-mix(in srgb, var(--blue) 15%, transparent);
    color: var(--blue);
  }

  .badge-red {
    background: color-mix(in srgb, var(--red) 15%, transparent);
    color: var(--red);
  }

  .badge-yellow {
    background: color-mix(in srgb, var(--yellow) 15%, transparent);
    color: var(--yellow);
  }

  /* ---- Mode Selection (Step 1) ---- */

  .mode-selection {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .mode-intro {
    font-size: 13px;
    color: var(--text-secondary);
    margin-bottom: var(--space-1);
  }

  .mode-card {
    display: flex;
    align-items: flex-start;
    gap: var(--space-4);
    padding: var(--space-5);
    background: var(--surface);
    border: 2px solid var(--separator);
    border-radius: var(--radius-lg);
    cursor: pointer;
    text-align: left;
    transition: all var(--duration-fast) var(--ease);
    width: 100%;
    font-family: var(--font-sans);
  }

  .mode-card:hover {
    border-color: var(--text-tertiary);
    background: var(--surface-hover);
  }

  .mode-card-selected {
    border-color: var(--system-accent);
    background: var(--system-accent-subtle);
  }

  .mode-card-selected:hover {
    background: var(--system-accent-subtle);
  }

  .mode-radio {
    flex-shrink: 0;
    margin-top: 3px;
  }

  .mode-body {
    flex: 1;
    min-width: 0;
  }

  .mode-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
    margin-bottom: 4px;
  }

  .mode-description {
    font-size: 13px;
    color: var(--text-tertiary);
    line-height: 1.5;
  }

  .mode-warning {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    margin-top: var(--space-3);
    padding: var(--space-2) var(--space-3);
    background: color-mix(in srgb, var(--yellow) 8%, transparent);
    border: 1px solid color-mix(in srgb, var(--yellow) 20%, transparent);
    border-radius: var(--radius-sm);
    font-size: 12px;
    color: var(--yellow);
    line-height: 1.4;
  }

  .mode-warning svg {
    flex-shrink: 0;
    margin-top: 1px;
  }

  /* ---- Radio / Checkbox shared ---- */

  .radio {
    width: 16px;
    height: 16px;
    border: 2px solid var(--text-tertiary);
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all var(--duration-fast) var(--ease);
  }

  .radio-checked {
    border-color: var(--system-accent);
    background: var(--system-accent);
  }

  .radio-dot {
    width: 6px;
    height: 6px;
    background: var(--system-accent-on);
    border-radius: 50%;
  }

  /* ---- ENB Options (Step 2) ---- */

  .enb-options {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
  }

  .option-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .option-section-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.005em;
  }

  .option-toggle-row {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-4);
    padding: var(--space-4);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
  }

  .option-toggle-info {
    flex: 1;
    min-width: 0;
  }

  .option-toggle-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-primary);
    margin-bottom: 2px;
  }

  .option-toggle-desc {
    font-size: 12px;
    color: var(--text-tertiary);
    line-height: 1.4;
  }

  /* ---- Toggle Switch ---- */

  .toggle-switch {
    position: relative;
    width: 40px;
    height: 22px;
    border-radius: 11px;
    background: var(--surface-hover);
    border: 1px solid var(--separator);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
    flex-shrink: 0;
    padding: 0;
    margin-top: 2px;
  }

  .toggle-switch.toggle-on {
    background: var(--system-accent);
    border-color: var(--system-accent);
  }

  .toggle-knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: white;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
    transition: transform var(--duration-fast) var(--ease);
  }

  .toggle-on .toggle-knob {
    transform: translateX(18px);
  }

  /* ---- Preset Cards ---- */

  .preset-cards {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .preset-card {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    cursor: pointer;
    text-align: left;
    transition: all var(--duration-fast) var(--ease);
    width: 100%;
    font-family: var(--font-sans);
  }

  .preset-card:hover {
    border-color: var(--text-tertiary);
    background: var(--surface-hover);
  }

  .preset-card-selected {
    border-color: var(--system-accent);
    background: var(--system-accent-subtle);
  }

  .preset-card-selected:hover {
    background: var(--system-accent-subtle);
  }

  .preset-card .radio {
    margin-top: 2px;
  }

  .preset-body {
    flex: 1;
    min-width: 0;
  }

  .preset-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    display: block;
  }

  .preset-desc {
    font-size: 12px;
    color: var(--text-tertiary);
    line-height: 1.4;
    display: block;
    margin-top: 2px;
  }

  /* ---- Enrichment (Step 3) ---- */

  .enrichment-results {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .enrichment-header {
    display: grid;
    grid-template-columns: 1fr 180px;
  }

  .enrichment-header .col-name,
  .enrichment-header .col-action {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-tertiary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .enrichment-row {
    display: grid;
    grid-template-columns: 1fr 180px;
    align-items: center;
  }

  .action-select-wrapper {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .action-select {
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-sm);
    color: var(--text-primary);
    font-size: 12px;
    font-weight: 500;
    padding: 4px 8px;
    cursor: pointer;
    font-family: var(--font-sans);
    min-width: 0;
  }

  .action-select:hover {
    border-color: var(--text-tertiary);
  }

  .action-select:focus {
    outline: none;
    border-color: var(--system-accent);
  }

  /* ---- Review (Step 4) ---- */

  .review-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .review-summary {
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    padding: var(--space-5);
  }

  .review-heading {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
    margin-bottom: var(--space-4);
  }

  .review-items {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .review-item {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    font-size: 13px;
    color: var(--text-secondary);
  }

  .review-item strong {
    color: var(--text-primary);
    font-weight: 700;
  }

  .review-item-icon {
    width: 28px;
    height: 28px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .review-icon-red {
    background: color-mix(in srgb, var(--red) 15%, transparent);
    color: var(--red);
  }

  .review-icon-blue {
    background: color-mix(in srgb, var(--blue) 15%, transparent);
    color: var(--blue);
  }

  .review-icon-yellow {
    background: color-mix(in srgb, var(--yellow) 15%, transparent);
    color: var(--yellow);
  }

  .review-icon-green {
    background: color-mix(in srgb, var(--green) 15%, transparent);
    color: var(--green);
  }

  .review-icon-purple {
    background: color-mix(in srgb, var(--purple) 15%, transparent);
    color: var(--purple);
  }

  /* ---- Execution (Step 5) ---- */

  .execution-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .execution-progress {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
  }

  .phase-label {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .phase-message {
    font-size: 13px;
    color: var(--text-tertiary);
    line-height: 1.4;
  }

  .progress-bar-container {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .progress-bar {
    flex: 1;
    height: 8px;
    background: var(--surface-hover);
    border-radius: 4px;
    overflow: hidden;
  }

  .progress-bar-fill {
    height: 100%;
    background: var(--system-accent);
    border-radius: 4px;
    transition: width 0.3s var(--ease);
  }

  .progress-text {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
    min-width: 36px;
    text-align: right;
  }

  .current-mod {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-sm);
    font-size: 12px;
  }

  .current-mod-label {
    color: var(--text-tertiary);
    font-weight: 500;
  }

  .current-mod-name {
    color: var(--text-primary);
    font-weight: 600;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* ---- Revert Section ---- */

  .revert-section {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-3);
    margin-top: var(--space-4);
    padding: var(--space-4);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
  }

  .revert-prompt {
    font-size: 13px;
    color: var(--text-secondary);
  }

  /* ---- Completion ---- */

  .completion-section {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-4);
    padding: var(--space-6) var(--space-4);
    text-align: center;
  }

  .completion-icon {
    margin-bottom: var(--space-1);
  }

  .completion-title {
    font-size: 18px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .completion-stats {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    align-items: flex-start;
    width: 100%;
    max-width: 400px;
  }

  .completion-stat {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: 13px;
    color: var(--text-secondary);
  }

  .completion-stat svg {
    flex-shrink: 0;
  }

  .completion-warnings {
    width: 100%;
    max-width: 400px;
    margin-top: var(--space-2);
    padding: var(--space-3);
    background: color-mix(in srgb, var(--yellow) 6%, transparent);
    border: 1px solid color-mix(in srgb, var(--yellow) 20%, transparent);
    border-radius: var(--radius);
    text-align: left;
  }

  .warnings-title {
    font-size: 12px;
    font-weight: 600;
    color: var(--yellow);
    margin-bottom: var(--space-2);
  }

  .warning-row {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    font-size: 11px;
    color: var(--text-tertiary);
    line-height: 1.4;
    margin-bottom: 4px;
  }

  .warning-row:last-child {
    margin-bottom: 0;
  }

  .warning-row svg {
    flex-shrink: 0;
    margin-top: 1px;
  }

  .completion-snapshot-note {
    font-size: 12px;
    color: var(--text-tertiary);
    margin-top: var(--space-2);
    max-width: 400px;
  }

  /* ---- Footer ---- */

  .wizard-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-3) var(--space-6);
    border-top: 1px solid var(--separator);
    background: var(--surface);
    border-radius: 0 0 var(--radius-xl) var(--radius-xl);
    flex-shrink: 0;
  }

  .footer-nav {
    display: flex;
    gap: var(--space-2);
  }

  .footer-hint {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-tertiary);
  }

  /* ---- Buttons ---- */

  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    padding: var(--space-2) var(--space-4);
    border-radius: var(--radius);
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
    min-height: 32px;
    border: none;
    font-family: var(--font-sans);
    white-space: nowrap;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-accent {
    background: var(--system-accent);
    color: var(--system-accent-on);
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

  .btn-ghost {
    background: transparent;
    color: var(--text-secondary);
  }

  .btn-ghost:hover:not(:disabled) {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .btn-danger {
    background: var(--red);
    color: white;
  }

  .btn-danger:hover:not(:disabled) {
    filter: brightness(1.1);
  }

  /* ---- Confirm Inline ---- */

  .confirm-inline {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    background: color-mix(in srgb, var(--yellow) 8%, transparent);
    border: 1px solid color-mix(in srgb, var(--yellow) 25%, transparent);
    border-radius: var(--radius);
  }

  .confirm-inline p {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
    line-height: 1.4;
    margin: 0;
  }

  .confirm-buttons {
    display: flex;
    gap: var(--space-2);
    justify-content: flex-end;
  }

  /* ---- Responsive ---- */

  @media (max-width: 640px) {
    .shader-wizard {
      width: 100%;
      max-width: 100%;
      max-height: 100vh;
      border-radius: 0;
    }

    .stat-row {
      grid-template-columns: repeat(2, 1fr);
    }

    .enrichment-header,
    .enrichment-row {
      grid-template-columns: 1fr;
      gap: var(--space-2);
    }

    .action-select-wrapper {
      padding-left: var(--space-4);
    }
  }
</style>
