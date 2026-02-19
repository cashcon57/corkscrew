<script lang="ts">
  import type { FomodInstaller, FomodGroup } from "$lib/types";

  interface Props {
    installer: FomodInstaller;
    onComplete: (selections: Record<string, string[]>) => void;
    onCancel: () => void;
  }

  let { installer, onComplete, onCancel }: Props = $props();

  let currentStep = $state(0);
  let selections = $state<Record<string, string[]>>({});

  // Initialize selections with defaults
  $effect(() => {
    const defaults: Record<string, string[]> = {};
    for (const step of installer.steps) {
      for (const group of step.groups) {
        defaults[group.name] = getDefaultForGroup(group);
      }
    }
    selections = defaults;
  });

  function getDefaultForGroup(group: FomodGroup): string[] {
    if (group.group_type === "SelectAll") {
      return group.options.map((o) => o.name);
    }
    if (group.group_type === "SelectExactlyOne" || group.group_type === "SelectAtMostOne") {
      const pick = group.options.find((o) => o.type_descriptor === "Required")
        || group.options.find((o) => o.type_descriptor === "Recommended")
        || group.options[0];
      return pick ? [pick.name] : [];
    }
    if (group.group_type === "SelectAtLeastOne" || group.group_type === "SelectAny") {
      const selected = group.options
        .filter((o) => o.type_descriptor === "Required" || o.type_descriptor === "Recommended")
        .map((o) => o.name);
      if (selected.length === 0 && group.group_type === "SelectAtLeastOne" && group.options.length > 0) {
        return [group.options[0].name];
      }
      return selected;
    }
    return [];
  }

  const step = $derived(installer.steps[currentStep]);
  const isLastStep = $derived(currentStep === installer.steps.length - 1);
  const isFirstStep = $derived(currentStep === 0);
  const totalSteps = $derived(installer.steps.length);

  function toggleOption(groupName: string, optionName: string, groupType: string) {
    const current = selections[groupName] || [];

    if (groupType === "SelectExactlyOne") {
      selections[groupName] = [optionName];
    } else if (groupType === "SelectAtMostOne") {
      if (current.includes(optionName)) {
        selections[groupName] = [];
      } else {
        selections[groupName] = [optionName];
      }
    } else {
      // Multi-select (SelectAny, SelectAtLeastOne, SelectAll)
      if (current.includes(optionName)) {
        selections[groupName] = current.filter((n) => n !== optionName);
      } else {
        selections[groupName] = [...current, optionName];
      }
    }
    // Force reactivity
    selections = { ...selections };
  }

  function isSelected(groupName: string, optionName: string): boolean {
    return (selections[groupName] || []).includes(optionName);
  }

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

  function isMultiSelect(type: string): boolean {
    return type === "SelectAny" || type === "SelectAtLeastOne" || type === "SelectAll";
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
<div class="fomod-overlay" onclick={onCancel}>
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <div class="fomod-wizard" onclick={(e) => e.stopPropagation()}>
    <!-- Header -->
    <div class="wizard-header">
      <h2 class="wizard-title">{installer.module_name}</h2>
      <div class="step-indicator">
        Step {currentStep + 1} of {totalSteps}
      </div>
      <button class="close-btn" onclick={onCancel} aria-label="Close">
        <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
          <line x1="3" y1="3" x2="11" y2="11" />
          <line x1="11" y1="3" x2="3" y2="11" />
        </svg>
      </button>
    </div>

    <!-- Step name -->
    <div class="step-name">{step.name}</div>

    <!-- Step content -->
    <div class="wizard-content">
      {#each step.groups as group}
        <div class="group-section">
          <div class="group-header">
            <h3 class="group-name">{group.name}</h3>
            <span class="group-type">{groupTypeLabel(group.group_type)}</span>
          </div>

          <div class="options-list">
            {#each group.options as option}
              {@const selected = isSelected(group.name, option.name)}
              {@const disabled = group.group_type === "SelectAll" || option.type_descriptor === "Required"}
              <button
                class="option-card"
                class:selected
                class:disabled
                onclick={() => !disabled && toggleOption(group.name, option.name, group.group_type)}
              >
                <div class="option-check">
                  {#if isMultiSelect(group.group_type)}
                    <div class="checkbox" class:checked={selected}>
                      {#if selected}
                        <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                          <polyline points="2,5 4,7.5 8,2.5" />
                        </svg>
                      {/if}
                    </div>
                  {:else}
                    <div class="radio" class:checked={selected}>
                      {#if selected}<div class="radio-dot"></div>{/if}
                    </div>
                  {/if}
                </div>
                <div class="option-info">
                  <span class="option-name">{option.name}</span>
                  {#if option.type_descriptor !== "Optional"}
                    <span class="type-badge" class:required={option.type_descriptor === "Required"} class:recommended={option.type_descriptor === "Recommended"}>
                      {option.type_descriptor}
                    </span>
                  {/if}
                  {#if option.description}
                    <p class="option-desc">{option.description}</p>
                  {/if}
                </div>
              </button>
            {/each}
          </div>
        </div>
      {/each}
    </div>

    <!-- Footer / Navigation -->
    <div class="wizard-footer">
      <button class="btn btn-ghost" onclick={onCancel}>Cancel</button>
      <div class="footer-nav">
        {#if !isFirstStep}
          <button class="btn btn-secondary" onclick={prev}>Back</button>
        {/if}
        <button class="btn btn-accent" onclick={next}>
          {isLastStep ? "Install" : "Next"}
        </button>
      </div>
    </div>
  </div>
</div>

<style>
  .fomod-overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: rgba(0, 0, 0, 0.6);
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 2000;
    animation: fadeIn 0.2s ease;
  }

  @keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .fomod-wizard {
    background: var(--bg-base);
    border: 1px solid var(--separator);
    border-radius: var(--radius-lg);
    width: 600px;
    max-width: 90vw;
    max-height: 80vh;
    display: flex;
    flex-direction: column;
    box-shadow: var(--shadow-lg);
    animation: slideUp 0.25s ease;
  }

  @keyframes slideUp {
    from { transform: translateY(12px); opacity: 0; }
    to { transform: translateY(0); opacity: 1; }
  }

  .wizard-header {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-5) var(--space-6);
    border-bottom: 1px solid var(--separator);
  }

  .wizard-title {
    font-size: 16px;
    font-weight: 700;
    color: var(--text-primary);
    flex: 1;
  }

  .step-indicator {
    font-size: 12px;
    color: var(--text-tertiary);
    font-weight: 500;
  }

  .close-btn {
    padding: var(--space-1);
    border-radius: var(--radius-sm);
    color: var(--text-tertiary);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
  }

  .close-btn:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .step-name {
    padding: var(--space-3) var(--space-6);
    font-size: 14px;
    font-weight: 600;
    color: var(--text-secondary);
    background: var(--surface);
    border-bottom: 1px solid var(--separator);
  }

  .wizard-content {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-5) var(--space-6);
  }

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
  }

  .group-type {
    font-size: 11px;
    color: var(--text-tertiary);
    background: var(--surface);
    padding: 1px var(--space-2);
    border-radius: var(--radius-sm);
  }

  .options-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

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
  }

  .option-card:hover:not(.disabled) {
    border-color: var(--text-quaternary);
  }

  .option-card.selected {
    border-color: var(--system-accent);
    background: rgba(232, 128, 42, 0.04);
  }

  .option-card.disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }

  .option-check {
    flex-shrink: 0;
    margin-top: 2px;
  }

  .checkbox, .radio {
    width: 16px;
    height: 16px;
    border: 2px solid var(--text-quaternary);
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all var(--duration-fast) var(--ease);
  }

  .checkbox {
    border-radius: 3px;
  }

  .radio {
    border-radius: 50%;
  }

  .checkbox.checked, .radio.checked {
    border-color: var(--system-accent);
    background: var(--system-accent);
  }

  .checkbox.checked svg {
    color: #fff;
  }

  .radio-dot {
    width: 6px;
    height: 6px;
    background: #fff;
    border-radius: 50%;
  }

  .option-info {
    flex: 1;
    min-width: 0;
  }

  .option-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .type-badge {
    display: inline-block;
    font-size: 10px;
    font-weight: 600;
    padding: 0px 4px;
    border-radius: 3px;
    margin-left: var(--space-2);
    vertical-align: middle;
  }

  .type-badge.required {
    color: var(--red);
    background: var(--red-subtle);
  }

  .type-badge.recommended {
    color: var(--green);
    background: var(--green-subtle);
  }

  .option-desc {
    font-size: 12px;
    color: var(--text-tertiary);
    margin-top: 2px;
    line-height: 1.4;
  }

  .wizard-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-4) var(--space-6);
    border-top: 1px solid var(--separator);
    background: var(--surface);
    border-radius: 0 0 var(--radius-lg) var(--radius-lg);
  }

  .footer-nav {
    display: flex;
    gap: var(--space-2);
  }

  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    border-radius: var(--radius);
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
    min-height: 32px;
  }

  .btn-accent {
    background: var(--system-accent);
    color: #fff;
  }

  .btn-accent:hover {
    filter: brightness(1.1);
  }

  .btn-secondary {
    background: var(--surface-hover);
    color: var(--text-primary);
    border: 1px solid var(--separator);
  }

  .btn-secondary:hover {
    background: var(--surface-active);
  }

  .btn-ghost {
    background: transparent;
    color: var(--text-secondary);
  }

  .btn-ghost:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }
</style>
