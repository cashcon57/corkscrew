<script lang="ts">
  import { themePreference, setThemePreference } from "$lib/theme";
  import type { ThemePreference } from "$lib/theme";

  let current = $state<ThemePreference>("system");

  themePreference.subscribe((val) => {
    current = val;
  });

  const options: { id: ThemePreference; label: string }[] = [
    { id: "light", label: "Light" },
    { id: "system", label: "Auto" },
    { id: "dark", label: "Dark" },
  ];

  function select(pref: ThemePreference) {
    setThemePreference(pref);
  }
</script>

<div class="theme-toggle" role="radiogroup" aria-label="Theme preference">
  {#each options as opt}
    <button
      class="toggle-option"
      class:active={current === opt.id}
      role="radio"
      aria-checked={current === opt.id}
      onclick={() => select(opt.id)}
    >
      <span class="toggle-icon">
        {#if opt.id === "light"}
          <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="8" cy="8" r="3" />
            <line x1="8" y1="1.5" x2="8" y2="3" />
            <line x1="8" y1="13" x2="8" y2="14.5" />
            <line x1="1.5" y1="8" x2="3" y2="8" />
            <line x1="13" y1="8" x2="14.5" y2="8" />
            <line x1="3.4" y1="3.4" x2="4.5" y2="4.5" />
            <line x1="11.5" y1="11.5" x2="12.6" y2="12.6" />
            <line x1="3.4" y1="12.6" x2="4.5" y2="11.5" />
            <line x1="11.5" y1="4.5" x2="12.6" y2="3.4" />
          </svg>
        {:else if opt.id === "system"}
          <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <rect x="1.5" y="2.5" width="13" height="8.5" rx="1" />
            <line x1="6" y1="13.5" x2="10" y2="13.5" />
            <line x1="8" y1="11" x2="8" y2="13.5" />
          </svg>
        {:else}
          <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M13.5 9.5a6 6 0 01-7-7 6 6 0 107 7z" />
          </svg>
        {/if}
      </span>
      <span class="toggle-label">{opt.label}</span>
    </button>
  {/each}
</div>

<style>
  .theme-toggle {
    display: inline-flex;
    align-items: center;
    background: var(--bg-tertiary);
    border-radius: var(--radius);
    padding: 2px;
    gap: 2px;
  }

  .toggle-option {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    padding: 5px var(--space-4);
    border-radius: calc(var(--radius) - 2px);
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
    background: transparent;
    transition:
      background var(--duration-fast) var(--ease),
      color var(--duration-fast) var(--ease),
      box-shadow var(--duration-fast) var(--ease);
    white-space: nowrap;
    line-height: 1.4;
  }

  .toggle-option:hover:not(.active) {
    color: var(--text-primary);
    background: var(--surface-hover);
  }

  /* NSSegmentedControl style: elevated white segment with shadow */
  .toggle-option.active {
    background: var(--bg-elevated);
    color: var(--text-primary);
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.15);
  }

  .toggle-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 12px;
    height: 12px;
    flex-shrink: 0;
  }

  .toggle-label {
    user-select: none;
  }
</style>
