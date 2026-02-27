<script lang="ts">
  import { getFomodDefaults } from "$lib/api";
  import { convertFileSrc } from "@tauri-apps/api/core";
  import type { FomodInstaller, FomodGroup, FomodOption } from "$lib/types";

  // ---- Props ----

  interface Props {
    installer: FomodInstaller;
    onComplete: (selections: Record<string, string[]>) => void;
    onCancel: () => void;
  }

  let { installer, onComplete, onCancel }: Props = $props();

  // ---- State ----

  let currentStep = $state(0);
  let selections = $state<Record<string, string[]>>({});
  let loading = $state(true);
  let previewImage = $state<string | null>(null);

  // ---- Derived ----

  const step = $derived(installer.steps[currentStep]);
  const totalSteps = $derived(installer.steps.length);
  const isFirstStep = $derived(currentStep === 0);
  const isLastStep = $derived(currentStep === installer.steps.length - 1);

  // ---- Initialize from backend defaults ----

  $effect(() => {
    loadDefaults();
  });

  async function loadDefaults() {
    loading = true;
    try {
      const defaults = await getFomodDefaults(installer);
      selections = defaults;
    } catch {
      // Fall back to client-side defaults if backend call fails
      const fallback: Record<string, string[]> = {};
      for (const s of installer.steps) {
        for (const group of s.groups) {
          fallback[group.name] = computeFallbackDefaults(group);
        }
      }
      selections = fallback;
    } finally {
      loading = false;
    }
  }

  function computeFallbackDefaults(group: FomodGroup): string[] {
    if (group.group_type === "SelectAll") {
      return group.options
        .filter((o) => o.type_descriptor !== "NotUsable")
        .map((o) => o.name);
    }
    if (group.group_type === "SelectExactlyOne" || group.group_type === "SelectAtMostOne") {
      const pick =
        group.options.find((o) => o.type_descriptor === "Required") ||
        group.options.find((o) => o.type_descriptor === "Recommended") ||
        (group.group_type === "SelectExactlyOne" ? group.options.find((o) => o.type_descriptor !== "NotUsable") : null);
      return pick ? [pick.name] : [];
    }
    if (group.group_type === "SelectAtLeastOne" || group.group_type === "SelectAny") {
      const selected = group.options
        .filter((o) => o.type_descriptor === "Required" || o.type_descriptor === "Recommended")
        .map((o) => o.name);
      if (selected.length === 0 && group.group_type === "SelectAtLeastOne" && group.options.length > 0) {
        const first = group.options.find((o) => o.type_descriptor !== "NotUsable");
        return first ? [first.name] : [];
      }
      return selected;
    }
    return [];
  }

  // ---- Selection logic ----

  function toggleOption(groupName: string, option: FomodOption, group: FomodGroup) {
    if (isOptionDisabled(option, group)) return;

    const current = selections[groupName] || [];

    if (group.group_type === "SelectExactlyOne") {
      selections[groupName] = [option.name];
    } else if (group.group_type === "SelectAtMostOne") {
      if (current.includes(option.name)) {
        selections[groupName] = [];
      } else {
        selections[groupName] = [option.name];
      }
    } else {
      // Multi-select: SelectAny, SelectAtLeastOne, SelectAll
      if (current.includes(option.name)) {
        // Prevent deselecting the last option in SelectAtLeastOne
        if (group.group_type === "SelectAtLeastOne" && current.length <= 1) {
          return;
        }
        selections[groupName] = current.filter((n) => n !== option.name);
      } else {
        selections[groupName] = [...current, option.name];
      }
    }
    // Force reactivity
    selections = { ...selections };
  }

  function isSelected(groupName: string, optionName: string): boolean {
    return (selections[groupName] || []).includes(optionName);
  }

  function isOptionDisabled(option: FomodOption, group: FomodGroup): boolean {
    if (option.type_descriptor === "NotUsable") return true;
    if (group.group_type === "SelectAll") return true;
    return false;
  }

  function isMultiSelect(type: string): boolean {
    return type === "SelectAny" || type === "SelectAtLeastOne" || type === "SelectAll";
  }

  // ---- Navigation ----

  function next() {
    if (isLastStep) {
      onComplete(selections);
    } else {
      currentStep++;
    }
  }

  function prev() {
    if (!isFirstStep) {
      currentStep--;
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      if (previewImage) {
        previewImage = null;
      } else {
        onCancel();
      }
    }
  }

  // ---- Helpers ----

  function groupTypeLabel(type: string): string {
    switch (type) {
      case "SelectExactlyOne": return "Select one";
      case "SelectAtMostOne": return "Select one or none";
      case "SelectAtLeastOne": return "Select one or more";
      case "SelectAny": return "Select any";
      case "SelectAll": return "All required";
      default: return type;
    }
  }

  function descriptorColor(descriptor: string): string {
    switch (descriptor) {
      case "Required": return "var(--green)";
      case "Recommended": return "var(--blue)";
      case "Optional": return "var(--text-tertiary)";
      case "NotUsable": return "var(--red)";
      case "CouldBeUsable": return "var(--text-tertiary)";
      default: return "var(--text-tertiary)";
    }
  }

  function imageUrl(path: string | undefined | null): string | undefined {
    if (!path) return undefined;
    // Absolute path from backend → convert to asset: protocol
    if (path.startsWith("/")) return convertFileSrc(path);
    // Already a URL (http/https/asset) → use as-is
    if (path.startsWith("http") || path.startsWith("asset:")) return path;
    // Relative path fallback (shouldn't happen with the backend fix)
    return path;
  }

  function descriptorBg(descriptor: string): string {
    switch (descriptor) {
      case "Required": return "var(--green-subtle)";
      case "Recommended": return "var(--blue-subtle)";
      case "Optional": return "var(--surface-hover)";
      case "NotUsable": return "var(--red-subtle)";
      case "CouldBeUsable": return "var(--surface-hover)";
      default: return "var(--surface-hover)";
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="fomod-overlay"
  onkeydown={handleKeydown}
  onclick={onCancel}
>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="fomod-wizard" onclick={(e) => e.stopPropagation()}>

    {#if loading}
      <!-- Loading State -->
      <div class="wizard-loading">
        <div class="spinner"></div>
        <p class="loading-text">Loading installer options...</p>
      </div>
    {:else}
      <!-- Header -->
      <div class="wizard-header">
        <div class="header-top">
          <h2 class="wizard-title">{installer.module_name}</h2>
          <button class="close-btn" onclick={onCancel} aria-label="Close" type="button">
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
              <line x1="2" y1="2" x2="10" y2="10" />
              <line x1="10" y1="2" x2="2" y2="10" />
            </svg>
          </button>
        </div>

        <!-- Step progress -->
        {#if totalSteps > 1}
          <div class="step-progress">
            <div class="step-dots">
              {#each installer.steps as s, i}
                <button
                  class="step-dot"
                  class:step-dot-active={i === currentStep}
                  class:step-dot-complete={i < currentStep}
                  disabled={i > currentStep}
                  onclick={() => { if (i <= currentStep) currentStep = i; }}
                  aria-label="Step {i + 1}: {s.name}"
                  type="button"
                >
                  {#if i < currentStep}
                    <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M20 6L9 17l-5-5" />
                    </svg>
                  {:else}
                    {i + 1}
                  {/if}
                </button>
                {#if i < totalSteps - 1}
                  <div class="step-connector" class:step-connector-complete={i < currentStep}></div>
                {/if}
              {/each}
            </div>
            <span class="step-label">Step {currentStep + 1} of {totalSteps}</span>
          </div>
        {/if}
      </div>

      <!-- Step name bar -->
      <div class="step-name-bar">
        <span class="step-name">{step.name}</span>
      </div>

      <!-- Content -->
      <div class="wizard-content">
        {#each step.groups as group (group.name)}
          <div class="group-section">
            <div class="group-header">
              <h3 class="group-name">{group.name}</h3>
              <span class="group-type-badge">{groupTypeLabel(group.group_type)}</span>
            </div>

            <div class="options-list">
              {#each group.options as option (option.name)}
                {@const selected = isSelected(group.name, option.name)}
                {@const disabled = isOptionDisabled(option, group)}
                <button
                  class="option-card"
                  class:option-selected={selected}
                  class:option-disabled={disabled}
                  class:option-not-usable={option.type_descriptor === "NotUsable"}
                  onclick={() => toggleOption(group.name, option, group)}
                  disabled={disabled}
                  type="button"
                >
                  <!-- Selection indicator -->
                  <div class="option-indicator">
                    {#if isMultiSelect(group.group_type)}
                      <div class="checkbox" class:checkbox-checked={selected}>
                        {#if selected}
                          <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <polyline points="2,5 4,7.5 8,2.5" />
                          </svg>
                        {/if}
                      </div>
                    {:else}
                      <div class="radio" class:radio-checked={selected}>
                        {#if selected}
                          <div class="radio-dot"></div>
                        {/if}
                      </div>
                    {/if}
                  </div>

                  <!-- Option content -->
                  <div class="option-body">
                    <div class="option-title-row">
                      <span class="option-name">{option.name}</span>
                      {#if option.type_descriptor !== "Optional"}
                        <span
                          class="descriptor-badge"
                          style="color: {descriptorColor(option.type_descriptor)}; background: {descriptorBg(option.type_descriptor)};"
                        >
                          {option.type_descriptor}
                        </span>
                      {/if}
                    </div>
                    {#if option.description}
                      <p class="option-description">{option.description}</p>
                    {/if}
                  </div>

                  <!-- Option image thumbnail -->
                  {#if option.image}
                    <button
                      class="option-thumbnail"
                      onclick={(e) => { e.stopPropagation(); previewImage = option.image; }}
                      type="button"
                      aria-label="Preview image for {option.name}"
                    >
                      <img
                        src={imageUrl(option.image)}
                        alt={option.name}
                        loading="lazy"
                      />
                      <div class="thumbnail-overlay">
                        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                          <circle cx="11" cy="11" r="8" />
                          <line x1="21" y1="21" x2="16.65" y2="16.65" />
                          <line x1="11" y1="8" x2="11" y2="14" />
                          <line x1="8" y1="11" x2="14" y2="11" />
                        </svg>
                      </div>
                    </button>
                  {/if}
                </button>
              {/each}
            </div>
          </div>
        {/each}
      </div>

      <!-- Footer -->
      <div class="wizard-footer">
        <button class="btn btn-ghost" onclick={onCancel} type="button">
          Cancel
        </button>
        <div class="footer-nav">
          {#if !isFirstStep}
            <button class="btn btn-secondary" onclick={prev} type="button">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="15 18 9 12 15 6" />
              </svg>
              Back
            </button>
          {/if}
          <button class="btn btn-accent" onclick={next} type="button">
            {#if isLastStep}
              Install
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                <polyline points="7 10 12 15 17 10" />
                <line x1="12" y1="15" x2="12" y2="3" />
              </svg>
            {:else}
              Next
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="9 18 15 12 9 6" />
              </svg>
            {/if}
          </button>
        </div>
      </div>
    {/if}
  </div>
</div>

<!-- Image preview lightbox -->
{#if previewImage}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="lightbox-overlay" onclick={() => { previewImage = null; }}>
    <div class="lightbox-content">
      <img src={imageUrl(previewImage)} alt="Preview" />
      <button
        class="lightbox-close"
        onclick={() => { previewImage = null; }}
        aria-label="Close preview"
        type="button"
      >
        <svg width="16" height="16" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
          <line x1="2" y1="2" x2="10" y2="10" />
          <line x1="10" y1="2" x2="2" y2="10" />
        </svg>
      </button>
    </div>
  </div>
{/if}

<style>
  /* ---- Overlay ---- */

  .fomod-overlay {
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

  .fomod-wizard {
    background: color-mix(in srgb, var(--bg-elevated) 75%, transparent);
    backdrop-filter: blur(40px) saturate(1.5);
    -webkit-backdrop-filter: blur(40px) saturate(1.5);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: var(--radius-xl);
    box-shadow: var(--glass-refraction),
                var(--glass-edge-shadow),
                var(--shadow-lg);
    width: 700px;
    max-width: calc(100vw - var(--space-8));
    max-height: 80vh;
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

  /* ---- Loading State ---- */

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

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .loading-text {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-tertiary);
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
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
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

  /* ---- Group Sections ---- */

  .group-section {
    margin-bottom: var(--space-6);
  }

  .group-section:last-child {
    margin-bottom: 0;
  }

  .group-header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-3);
  }

  .group-name {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.005em;
  }

  .group-type-badge {
    font-size: 11px;
    font-weight: 500;
    color: var(--text-tertiary);
    background: var(--surface);
    padding: 1px var(--space-2);
    border-radius: var(--radius-sm);
    white-space: nowrap;
  }

  /* ---- Options List ---- */

  .options-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  /* ---- Option Card ---- */

  .option-card {
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

  .option-card:hover:not(:disabled) {
    border-color: var(--text-tertiary);
    background: var(--surface-hover);
  }

  .option-card.option-selected {
    border-color: var(--system-accent);
    background: var(--system-accent-subtle);
  }

  .option-card.option-selected:hover:not(:disabled) {
    background: var(--system-accent-subtle);
  }

  .option-card.option-disabled {
    cursor: default;
  }

  .option-card.option-disabled.option-selected {
    opacity: 0.85;
  }

  .option-card.option-not-usable {
    opacity: 0.4;
    cursor: not-allowed;
  }

  /* ---- Selection Indicator ---- */

  .option-indicator {
    flex-shrink: 0;
    margin-top: 2px;
  }

  .checkbox,
  .radio {
    width: 16px;
    height: 16px;
    border: 2px solid var(--text-tertiary);
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all var(--duration-fast) var(--ease);
  }

  .checkbox {
    border-radius: 4px;
  }

  .radio {
    border-radius: 50%;
  }

  .checkbox-checked,
  .radio-checked {
    border-color: var(--system-accent);
    background: var(--system-accent);
  }

  .checkbox-checked svg {
    color: var(--system-accent-on);
  }

  .radio-dot {
    width: 6px;
    height: 6px;
    background: var(--system-accent-on);
    border-radius: 50%;
  }

  /* ---- Option Body ---- */

  .option-body {
    flex: 1;
    min-width: 0;
  }

  .option-title-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .option-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    line-height: 1.3;
  }

  .descriptor-badge {
    display: inline-flex;
    align-items: center;
    font-size: 10px;
    font-weight: 600;
    padding: 0 5px;
    border-radius: 3px;
    line-height: 1.6;
    letter-spacing: 0.01em;
    white-space: nowrap;
  }

  .option-description {
    font-size: 12px;
    color: var(--text-tertiary);
    margin-top: 3px;
    line-height: 1.45;
  }

  /* ---- Option Thumbnail ---- */

  .option-thumbnail {
    flex-shrink: 0;
    width: 56px;
    height: 56px;
    border-radius: var(--radius-sm);
    overflow: hidden;
    border: 1px solid var(--separator);
    cursor: pointer;
    position: relative;
    background: var(--bg-tertiary);
    padding: 0;
    transition: border-color var(--duration-fast) var(--ease);
  }

  .option-thumbnail:hover {
    border-color: var(--system-accent);
  }

  .option-thumbnail img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }

  .thumbnail-overlay {
    position: absolute;
    inset: 0;
    background: rgba(0, 0, 0, 0.4);
    display: flex;
    align-items: center;
    justify-content: center;
    color: white;
    opacity: 0;
    transition: opacity var(--duration-fast) var(--ease);
  }

  .option-thumbnail:hover .thumbnail-overlay {
    opacity: 1;
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

  /* ---- Lightbox ---- */

  .lightbox-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.8);
    backdrop-filter: var(--glass-blur-heavy);
    -webkit-backdrop-filter: var(--glass-blur-heavy);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 3000;
    animation: fadeIn 0.15s var(--ease);
    cursor: pointer;
  }

  .lightbox-content {
    position: relative;
    max-width: 80vw;
    max-height: 80vh;
  }

  .lightbox-content img {
    max-width: 100%;
    max-height: 80vh;
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-lg);
    display: block;
  }

  .lightbox-close {
    position: absolute;
    top: var(--space-2);
    right: var(--space-2);
    width: 32px;
    height: 32px;
    border-radius: 50%;
    background: rgba(0, 0, 0, 0.6);
    border: none;
    color: white;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: background var(--duration-fast) var(--ease);
  }

  .lightbox-close:hover {
    background: rgba(0, 0, 0, 0.8);
  }
</style>
