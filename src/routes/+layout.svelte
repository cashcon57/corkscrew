<script lang="ts">
  import { onMount } from "svelte";
  import "../app.css";
  import { currentPage, errorMessage, successMessage } from "$lib/stores";
  import { initTheme } from "$lib/theme";

  const navItems = [
    { id: "dashboard", label: "Dashboard" },
    { id: "mods", label: "Mods" },
    { id: "plugins", label: "Load Order" },
    { id: "profiles", label: "Profiles" },
    { id: "settings", label: "Settings" },
    { id: "about", label: "About" },
  ];

  onMount(() => {
    initTheme();
  });

  function navigate(page: string) {
    currentPage.set(page);
  }
</script>

<div class="app-shell">
  <!-- Full-width drag region at top of window for window movement -->
  <div class="window-drag-region" data-tauri-drag-region></div>

  <nav class="sidebar">
    <!-- Traffic light zone: spacer for macOS traffic lights -->
    <div class="sidebar-traffic-zone"></div>

    <ul class="nav-list">
      {#each navItems as item}
        <li>
          <button
            class="nav-item"
            class:active={$currentPage === item.id}
            onclick={() => navigate(item.id)}
          >
            <span class="nav-icon">
              {#if item.id === "dashboard"}
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <rect x="1.5" y="1.5" width="5" height="5" rx="1" />
                  <rect x="9.5" y="1.5" width="5" height="5" rx="1" />
                  <rect x="1.5" y="9.5" width="5" height="5" rx="1" />
                  <rect x="9.5" y="9.5" width="5" height="5" rx="1" />
                </svg>
              {:else if item.id === "mods"}
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <rect x="3" y="1.5" width="10" height="13" rx="1.5" />
                  <line x1="5.5" y1="4.5" x2="10.5" y2="4.5" />
                  <line x1="5.5" y1="7" x2="10.5" y2="7" />
                  <line x1="5.5" y1="9.5" x2="8.5" y2="9.5" />
                </svg>
              {:else if item.id === "plugins"}
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <rect x="2.5" y="2" width="11" height="3" rx="1" />
                  <rect x="2.5" y="6.5" width="11" height="3" rx="1" />
                  <rect x="2.5" y="11" width="11" height="3" rx="1" />
                </svg>
              {:else if item.id === "profiles"}
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <rect x="2" y="2" width="12" height="4" rx="1" />
                  <rect x="2" y="10" width="12" height="4" rx="1" />
                  <line x1="5" y1="4" x2="5" y2="4" />
                  <line x1="5" y1="12" x2="5" y2="12" />
                </svg>
              {:else if item.id === "settings"}
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <circle cx="8" cy="8" r="2.5" />
                  <path d="M8 1.5v2M8 12.5v2M2.7 4.5l1.7 1M11.6 10.5l1.7 1M1.5 8h2M12.5 8h2M2.7 11.5l1.7-1M11.6 5.5l1.7-1" />
                </svg>
              {:else}
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <circle cx="8" cy="8" r="6.5" />
                  <line x1="8" y1="7" x2="8" y2="11" />
                  <circle cx="8" cy="5" r="0.5" fill="currentColor" />
                </svg>
              {/if}
            </span>
            <span class="nav-label">{item.label}</span>
          </button>
        </li>
      {/each}
    </ul>

    <div class="sidebar-footer">
      <div class="sidebar-brand">
        <span class="sidebar-title">Corkscrew</span>
        <span class="sidebar-subtitle">v0.1.0</span>
      </div>
    </div>
  </nav>

  <main class="content">

    {#if $errorMessage}
      <div class="toast toast-error" role="alert">
        <svg class="toast-icon" width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="7" cy="7" r="6" />
          <line x1="7" y1="4" x2="7" y2="7.5" />
          <circle cx="7" cy="10" r="0.5" fill="currentColor" />
        </svg>
        <span class="toast-text">{$errorMessage}</span>
        <button class="toast-dismiss" onclick={() => errorMessage.set(null)} aria-label="Dismiss error">
          <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
            <line x1="2" y1="2" x2="8" y2="8" />
            <line x1="8" y1="2" x2="2" y2="8" />
          </svg>
        </button>
      </div>
    {/if}

    {#if $successMessage}
      <div class="toast toast-success" role="status">
        <svg class="toast-icon" width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="7" cy="7" r="6" />
          <path d="M4.5 7l2 2 3-3.5" />
        </svg>
        <span class="toast-text">{$successMessage}</span>
        <button class="toast-dismiss" onclick={() => successMessage.set(null)} aria-label="Dismiss notification">
          <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
            <line x1="2" y1="2" x2="8" y2="8" />
            <line x1="8" y1="2" x2="2" y2="8" />
          </svg>
        </button>
      </div>
    {/if}

    <slot />
  </main>
</div>

<style>
  .app-shell {
    display: flex;
    height: 100vh;
    overflow: hidden;
  }

  /* --- Sidebar --- */

  .sidebar {
    width: 220px;
    min-width: 220px;
    background: var(--bg-grouped);
    border-right: 1px solid var(--separator);
    display: flex;
    flex-direction: column;
  }

  /* Remove sidebar border when vibrancy provides visual separation */
  :global(html.vibrancy-active) .sidebar {
    border-right: none;
  }

  /* Traffic light zone — macOS overlay titlebar puts
     close/minimize/maximize buttons in this area */
  .sidebar-traffic-zone {
    height: 52px;
    flex-shrink: 0;
    /* Entire zone is draggable for window movement */
  }

  .nav-list {
    list-style: none;
    padding: 0 var(--space-2);
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .nav-item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: 6px 10px;
    border-radius: var(--radius-sm);
    color: var(--text-secondary);
    font-size: 13px;
    font-weight: 500;
    transition: all var(--duration-fast) var(--ease);
    text-align: left;
  }

  .nav-item:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .nav-item.active {
    background: var(--system-accent-subtle);
    color: var(--system-accent);
  }

  .nav-icon {
    width: 16px;
    height: 16px;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  /* --- Sidebar footer (brand) --- */

  .sidebar-footer {
    padding: var(--space-3) var(--space-4) var(--space-4);
    border-top: 1px solid var(--separator);
  }

  .sidebar-brand {
    display: flex;
    flex-direction: column;
  }

  .sidebar-title {
    font-size: 12px;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--text-secondary);
    line-height: 1.2;
  }

  .sidebar-subtitle {
    font-size: 11px;
    color: var(--text-tertiary);
    font-weight: 400;
  }

  /* --- Content --- */

  .content {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-6);
    padding-top: calc(52px + var(--space-4));
    background: var(--bg-base);
    position: relative;
  }

  /* Full-width drag region — overlays the entire top of the window
     for dragging. Sits above sidebar + content in a fixed position. */
  .window-drag-region {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    height: 52px;
    z-index: 100;
    -webkit-app-region: drag;
    /* Transparent — just a drag target, no visual element */
  }

  /* --- Toast notifications --- */

  .toast {
    position: fixed;
    top: calc(52px + var(--space-2));
    right: var(--space-4);
    padding: 10px var(--space-4);
    border-radius: var(--radius);
    font-size: 13px;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    z-index: 1000;
    -webkit-app-region: no-drag;
    box-shadow: var(--shadow-lg);
    animation: toastIn var(--duration-slow) var(--ease-out);
    backdrop-filter: blur(20px);
    -webkit-backdrop-filter: blur(20px);
    max-width: 400px;
  }

  .toast-error {
    background: var(--red-subtle);
    border: 1px solid var(--red-subtle);
    color: var(--red);
  }

  .toast-success {
    background: var(--green-subtle);
    border: 1px solid var(--green-subtle);
    color: var(--green);
  }

  .toast-icon {
    flex-shrink: 0;
  }

  .toast-text {
    flex: 1;
    font-weight: 500;
  }

  .toast-dismiss {
    flex-shrink: 0;
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    opacity: 0.5;
    transition: opacity var(--duration-fast) var(--ease);
    min-width: 28px;
    min-height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .toast-dismiss:hover {
    opacity: 1;
  }

  @keyframes toastIn {
    from { transform: translateY(-8px); opacity: 0; }
    to { transform: translateY(0); opacity: 1; }
  }
</style>
