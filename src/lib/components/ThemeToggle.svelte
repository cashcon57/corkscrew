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
          <!-- Sun icon -->
          <svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor">
            <path d="M8 1a.5.5 0 01.5.5v1a.5.5 0 01-1 0v-1A.5.5 0 018 1zm0 11a.5.5 0 01.5.5v1a.5.5 0 01-1 0v-1A.5.5 0 018 12zm7-4a.5.5 0 01-.5.5h-1a.5.5 0 010-1h1A.5.5 0 0115 8zM4 8a.5.5 0 01-.5.5h-1a.5.5 0 010-1h1A.5.5 0 014 8zm8.95-3.54a.5.5 0 010 .7l-.71.71a.5.5 0 11-.7-.7l.7-.71a.5.5 0 01.71 0zM5.17 10.46a.5.5 0 010 .7l-.71.71a.5.5 0 01-.7-.7l.7-.71a.5.5 0 01.71 0zm7.07 1.41a.5.5 0 01-.7 0l-.71-.7a.5.5 0 01.7-.71l.71.7a.5.5 0 010 .71zM4.46 5.17a.5.5 0 01-.7 0l-.71-.71a.5.5 0 01.7-.7l.71.7a.5.5 0 010 .71zM8 5a3 3 0 100 6 3 3 0 000-6z" />
          </svg>
        {:else if opt.id === "system"}
          <!-- Monitor icon -->
          <svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor">
            <path d="M2.5 2A1.5 1.5 0 001 3.5v7A1.5 1.5 0 002.5 12H6v1.5H4.5a.5.5 0 000 1h7a.5.5 0 000-1H10V12h3.5a1.5 1.5 0 001.5-1.5v-7A1.5 1.5 0 0013.5 2h-11zM2.5 3h11a.5.5 0 01.5.5v7a.5.5 0 01-.5.5h-11a.5.5 0 01-.5-.5v-7a.5.5 0 01.5-.5zM7 12h2v1.5H7V12z" />
          </svg>
        {:else}
          <!-- Moon icon -->
          <svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor">
            <path d="M6.2 1.74A7 7 0 0014.26 9.8a.5.5 0 01-.62.62A6.5 6.5 0 015.58 2.36a.5.5 0 01.62-.62zM4.68 3.32a5.5 5.5 0 107.99 8A8 8 0 014.68 3.32z" />
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
    padding: var(--space-1) var(--space-3);
    border-radius: calc(var(--radius) - 2px);
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
    background: transparent;
    transition:
      background var(--duration-fast) var(--ease),
      color var(--duration-fast) var(--ease);
    white-space: nowrap;
    line-height: 1.4;
  }

  .toggle-option:hover:not(.active) {
    color: var(--text-primary);
    background: var(--surface-hover);
  }

  .toggle-option.active {
    background: var(--accent-subtle);
    color: var(--accent);
  }

  .toggle-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 14px;
    height: 14px;
    flex-shrink: 0;
  }

  .toggle-label {
    user-select: none;
  }
</style>
