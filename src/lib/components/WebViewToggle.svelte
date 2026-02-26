<script lang="ts">
  import { onDestroy } from "svelte";
  import { sidebarCollapsed } from "$lib/stores";
  import {
    createBrowserWebview,
    resizeBrowserWebview,
    closeBrowserWebview,
  } from "$lib/api";

  interface Props {
    url: string;
    defaultMode?: "app" | "website";
    onModeChange?: (mode: "app" | "website") => void;
  }

  let { url, defaultMode = "app", onModeChange }: Props = $props();
  let mode = $state<"app" | "website">(defaultMode);
  let webviewActive = $state(false);

  // Layout constants (match +layout.svelte)
  const APP_PADDING = 8;
  const GAP = 8;
  const TOPBAR_HEIGHT = 44;
  const SIDEBAR_EXPANDED = 220;
  const SIDEBAR_COLLAPSED = 56;

  function getContentBounds() {
    const sidebarWidth = $sidebarCollapsed ? SIDEBAR_COLLAPSED : SIDEBAR_EXPANDED;
    const x = APP_PADDING + sidebarWidth + GAP;
    const y = APP_PADDING + TOPBAR_HEIGHT;
    const width = window.innerWidth - x - APP_PADDING;
    const height = window.innerHeight - y - APP_PADDING;
    return { x, y, width, height };
  }

  async function activateWebview() {
    try {
      const b = getContentBounds();
      await createBrowserWebview(url, b.x, b.y, b.width, b.height);
      webviewActive = true;
    } catch (e) {
      // Webview creation failed — falls back to non-webview UI
    }
  }

  async function deactivateWebview() {
    try {
      await closeBrowserWebview();
    } catch {
      // ignore if already closed
    }
    webviewActive = false;
  }

  async function setMode(newMode: "app" | "website") {
    if (newMode === mode) return;
    mode = newMode;
    if (newMode === "website") {
      await activateWebview();
    } else {
      await deactivateWebview();
    }
    onModeChange?.(newMode);
  }

  // Handle window resize
  function handleResize() {
    if (!webviewActive) return;
    const b = getContentBounds();
    resizeBrowserWebview(b.x, b.y, b.width, b.height).catch(() => {});
  }

  // React to sidebar collapse changes
  $effect(() => {
    // Access the store to track it
    const _ = $sidebarCollapsed;
    if (webviewActive) {
      // Small delay to let CSS transition finish
      setTimeout(() => handleResize(), 200);
    }
  });

  // Cleanup on destroy
  onDestroy(() => {
    if (webviewActive) {
      closeBrowserWebview().catch(() => {});
    }
  });

  // Expose close method for parent components
  export function closeWebview() {
    if (webviewActive) {
      deactivateWebview();
      mode = "app";
    }
  }
</script>

<svelte:window onresize={handleResize} />

<div class="webview-toggle">
  <button
    class="toggle-btn"
    class:active={mode === "app"}
    onclick={() => setMode("app")}
  >
    In-App
  </button>
  <button
    class="toggle-btn"
    class:active={mode === "website"}
    onclick={() => setMode("website")}
  >
    Website
  </button>
</div>

<style>
  .webview-toggle {
    display: flex;
    background: var(--bg-tertiary);
    border-radius: 6px;
    padding: 2px;
    gap: 2px;
  }

  .toggle-btn {
    padding: 4px 12px;
    border: none;
    border-radius: 4px;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.15s ease;
    color: var(--text-secondary);
    background: transparent;
  }

  .toggle-btn.active {
    background: var(--system-accent);
    color: #fff;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.2);
  }

  .toggle-btn:not(.active):hover {
    color: var(--text-primary);
    background: var(--bg-quaternary);
  }
</style>
