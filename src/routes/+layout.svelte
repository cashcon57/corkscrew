<script lang="ts">
  import { onMount } from "svelte";
  import "../app.css";
  import { currentPage, errorMessage, successMessage } from "$lib/stores";
  import { initTheme } from "$lib/theme";

  const navItems = [
    { id: "dashboard", label: "Dashboard" },
    { id: "mods", label: "Mods" },
    { id: "plugins", label: "Load Order" },
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
  <nav class="sidebar">
    <div class="sidebar-drag-region" data-tauri-drag-region></div>

    <div class="sidebar-brand">
      <div class="sidebar-brand-text">
        <span class="sidebar-title">Corkscrew</span>
        <span class="sidebar-subtitle">v0.1.0</span>
      </div>
    </div>

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
                <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor">
                  <rect x="1" y="1" width="6" height="6" rx="1.5" />
                  <rect x="9" y="1" width="6" height="6" rx="1.5" />
                  <rect x="1" y="9" width="6" height="6" rx="1.5" />
                  <rect x="9" y="9" width="6" height="6" rx="1.5" />
                </svg>
              {:else if item.id === "mods"}
                <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor">
                  <path d="M3 2.5A1.5 1.5 0 014.5 1h7A1.5 1.5 0 0113 2.5v11a1.5 1.5 0 01-1.5 1.5h-7A1.5 1.5 0 013 13.5v-11zM5.5 4a.5.5 0 000 1h5a.5.5 0 000-1h-5zm0 3a.5.5 0 000 1h5a.5.5 0 000-1h-5zm0 3a.5.5 0 000 1h3a.5.5 0 000-1h-3z" />
                </svg>
              {:else if item.id === "plugins"}
                <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor">
                  <path d="M2.5 3.5a1 1 0 011-1h9a1 1 0 011 1v1a1 1 0 01-1 1h-9a1 1 0 01-1-1v-1zm0 4a1 1 0 011-1h9a1 1 0 011 1v1a1 1 0 01-1 1h-9a1 1 0 01-1-1v-1zm0 4a1 1 0 011-1h9a1 1 0 011 1v1a1 1 0 01-1 1h-9a1 1 0 01-1-1v-1z" />
                </svg>
              {:else if item.id === "settings"}
                <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor">
                  <path d="M8 0a3 3 0 00-1.17 5.76A5.5 5.5 0 002.5 11v.5a1 1 0 001 1h9a1 1 0 001-1V11a5.5 5.5 0 00-4.33-5.24A3 3 0 008 0zM5 14.25a.75.75 0 01.75-.75h4.5a.75.75 0 010 1.5h-4.5a.75.75 0 01-.75-.75z" />
                </svg>
              {:else}
                <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor">
                  <path d="M8 1a7 7 0 110 14A7 7 0 018 1zm0 3a.75.75 0 00-.75.75v.5a.75.75 0 001.5 0v-.5A.75.75 0 008 4zm0 3a.75.75 0 00-.75.75v3.5a.75.75 0 001.5 0v-3.5A.75.75 0 008 7z" />
                </svg>
              {/if}
            </span>
            <span class="nav-label">{item.label}</span>
          </button>
        </li>
      {/each}
    </ul>

    <div class="sidebar-footer">
      <span class="sidebar-footer-text">Mod Manager for Wine Games</span>
    </div>
  </nav>

  <main class="content">
    {#if $errorMessage}
      <div class="toast toast-error" role="alert">
        <svg class="toast-icon" width="14" height="14" viewBox="0 0 14 14" fill="currentColor">
          <path d="M7 0a7 7 0 110 14A7 7 0 017 0zm-.5 4a.75.75 0 00-.75.75v3.5a.75.75 0 001.5 0v-3.5A.75.75 0 006.5 4zM7 10a.75.75 0 100 1.5.75.75 0 000-1.5z" />
        </svg>
        <span class="toast-text">{$errorMessage}</span>
        <button class="toast-dismiss" onclick={() => errorMessage.set(null)} aria-label="Dismiss error">
          <svg width="10" height="10" viewBox="0 0 10 10" fill="currentColor">
            <path d="M1.7 1.7a.6.6 0 01.85 0L5 4.15 7.45 1.7a.6.6 0 01.85.85L5.85 5l2.45 2.45a.6.6 0 01-.85.85L5 5.85 2.55 8.3a.6.6 0 01-.85-.85L4.15 5 1.7 2.55a.6.6 0 010-.85z" />
          </svg>
        </button>
      </div>
    {/if}

    {#if $successMessage}
      <div class="toast toast-success" role="status">
        <svg class="toast-icon" width="14" height="14" viewBox="0 0 14 14" fill="currentColor">
          <path d="M7 0a7 7 0 110 14A7 7 0 017 0zm2.85 5.15a.6.6 0 00-.85-.85L6.25 7.05 5 5.8a.6.6 0 00-.85.85l1.68 1.67a.6.6 0 00.84 0l3.18-3.17z" />
        </svg>
        <span class="toast-text">{$successMessage}</span>
        <button class="toast-dismiss" onclick={() => successMessage.set(null)} aria-label="Dismiss notification">
          <svg width="10" height="10" viewBox="0 0 10 10" fill="currentColor">
            <path d="M1.7 1.7a.6.6 0 01.85 0L5 4.15 7.45 1.7a.6.6 0 01.85.85L5.85 5l2.45 2.45a.6.6 0 01-.85.85L5 5.85 2.55 8.3a.6.6 0 01-.85-.85L4.15 5 1.7 2.55a.6.6 0 010-.85z" />
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
    width: 240px;
    min-width: 240px;
    background: var(--bg-grouped);
    border-right: 1px solid var(--separator);
    display: flex;
    flex-direction: column;
  }

  .sidebar-drag-region {
    height: 28px;
    flex-shrink: 0;
  }

  .sidebar-brand {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 0 var(--space-4) var(--space-4);
  }

  .sidebar-brand-text {
    display: flex;
    flex-direction: column;
  }

  .sidebar-title {
    font-size: 15px;
    font-weight: 700;
    letter-spacing: -0.02em;
    color: var(--text-primary);
    line-height: 1.2;
  }

  .sidebar-subtitle {
    font-size: 11px;
    color: var(--text-tertiary);
    font-weight: 500;
  }

  .nav-list {
    list-style: none;
    padding: 0 var(--space-3);
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .nav-item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: 7px 10px;
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
    background: var(--accent-subtle);
    color: var(--accent);
  }

  .nav-icon {
    width: 18px;
    height: 18px;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    opacity: 0.8;
  }

  .nav-item.active .nav-icon {
    opacity: 1;
  }

  .sidebar-footer {
    padding: var(--space-3) var(--space-4);
    border-top: 1px solid var(--separator);
  }

  .sidebar-footer-text {
    font-size: 11px;
    color: var(--text-quaternary);
    font-weight: 500;
  }

  /* --- Content --- */

  .content {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-8);
    padding-top: calc(28px + var(--space-6));
    background: var(--bg-base);
    position: relative;
  }

  /* --- Toast notifications --- */

  .toast {
    position: fixed;
    top: var(--space-4);
    right: var(--space-4);
    padding: 10px var(--space-4);
    border-radius: var(--radius);
    font-size: 13px;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    z-index: 1000;
    box-shadow: var(--shadow-lg);
    animation: toastIn var(--duration) var(--ease);
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
    padding: var(--space-1);
    border-radius: var(--radius-sm);
    opacity: 0.5;
    transition: opacity var(--duration-fast) var(--ease);
  }

  .toast-dismiss:hover {
    opacity: 1;
  }

  @keyframes toastIn {
    from { transform: translateY(-8px); opacity: 0; }
    to { transform: translateY(0); opacity: 1; }
  }
</style>
