<script lang="ts">
  import { onMount } from "svelte";
  import { rescanNativeGames } from "$lib/api";
  import type { NativeAppCandidate } from "$lib/types";

  let candidates: NativeAppCandidate[] = $state([]);
  let scanning = $state(false);
  let error: string | null = $state(null);

  async function scan() {
    scanning = true;
    error = null;
    try {
      candidates = await rescanNativeGames();
    } catch (e) {
      error = String(e);
      console.error("rescanNativeGames failed:", e);
    } finally {
      scanning = false;
    }
  }

  onMount(scan);

  function archLabel(a: string): string {
    return (
      ({
        apple_silicon: "Apple Silicon",
        intel_only: "Intel (Rosetta)",
        universal: "Universal",
        unknown: "Unknown arch",
      } as Record<string, string>)[a] ?? a
    );
  }

  function sourceLabel(s: string): string {
    return (
      ({
        system_applications: "/Applications",
        steam: "Steam",
        gog: "GOG",
        manual: "Added manually",
        app_store: "Mac App Store",
      } as Record<string, string>)[s] ?? s
    );
  }
</script>

<div class="page">
  <h1 class="m5-gradient-text">Discover Native Games</h1>
  <p class="subtitle">macOS-native installs Corkscrew can mod.</p>

  <button class="scan-btn" onclick={scan} disabled={scanning}>
    {scanning ? "Scanning\u2026" : "Rescan"}
  </button>

  {#if error}
    <div class="error">Scan failed: {error}</div>
  {/if}

  {#if candidates.length === 0 && !scanning}
    <div class="native-glass-card empty">
      <p>No native macOS games found yet.</p>
    </div>
  {:else}
    <div class="grid">
      {#each candidates as c}
        <div class="native-glass-card candidate">
          <h3>{c.info.bundle_executable}</h3>
          <p class="path">{c.bundle_path}</p>
          <div class="badges">
            <span class="badge">{archLabel(c.architecture)}</span>
            <span class="badge">{sourceLabel(c.source)}</span>
            {#if c.sandboxed}
              <span class="badge warn">Sandboxed (unmoddable)</span>
            {/if}
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .page {
    max-width: 1200px;
    margin: 0 auto;
    padding: 32px 24px;
  }
  h1 {
    font-size: 36px;
    font-weight: 700;
    letter-spacing: -0.02em;
    margin: 0 0 8px;
  }
  .subtitle {
    color: var(--text-secondary);
    margin: 0 0 24px;
  }
  .scan-btn {
    background: var(--m5-gradient);
    color: white;
    border: none;
    padding: 10px 18px;
    border-radius: 8px;
    font-weight: 600;
    cursor: pointer;
    margin-bottom: 24px;
  }
  .scan-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .error {
    color: var(--red);
    background: var(--red-subtle);
    padding: 12px 16px;
    border-radius: 8px;
    margin-bottom: 24px;
  }
  .empty {
    padding: 48px 32px;
    text-align: center;
    color: var(--text-secondary);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
    gap: 16px;
  }
  .candidate {
    padding: 20px;
  }
  .candidate h3 {
    font-size: 16px;
    font-weight: 600;
    margin: 0 0 6px;
    color: var(--text-primary);
  }
  .path {
    font-size: 12px;
    color: var(--text-tertiary);
    margin: 0 0 14px;
    word-break: break-all;
    font-family: ui-monospace, monospace;
  }
  .badges {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .badge {
    background: var(--surface);
    color: var(--text-secondary);
    padding: 3px 8px;
    border-radius: 12px;
    font-size: 11px;
    font-weight: 500;
  }
  .badge.warn {
    background: var(--yellow-subtle);
    color: var(--yellow);
  }
</style>
