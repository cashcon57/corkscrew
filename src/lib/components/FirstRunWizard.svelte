<script lang="ts">
  import { onMount } from "svelte";
  import {
    getBottles,
    getAllGames,
    setConfigValue,
    startOAuthLogin,
    getNexusAccountStatus,
    checkSteamStatus,
    addToSteam,
  } from "$lib/api";
  import { bottles, games, selectedGame, currentPage } from "$lib/stores";
  import type { Bottle, DetectedGame, SteamStatus } from "$lib/types";

  let { onComplete }: { onComplete: () => void } = $props();

  let step = $state(0);
  let scanning = $state(false);
  let detectedBottles = $state<Bottle[]>([]);
  let detectedGames = $state<DetectedGame[]>([]);

  // NexusMods OAuth
  let nexusConnected = $state(false);
  let nexusName = $state<string | null>(null);
  let nexusConnecting = $state(false);
  let nexusError = $state<string | null>(null);

  // Steam integration (Linux only)
  let steamStatus = $state<SteamStatus | null>(null);
  let steamRegistering = $state(false);
  let isLinux = $state(false);

  const steps = $derived(isLinux && steamStatus?.installed
    ? ["Welcome", "Detect Games", "Steam", "Nexus Mods", "Ready"]
    : ["Welcome", "Detect Games", "Nexus Mods", "Ready"]
  );
  const currentStep = $derived(steps[step] ?? "Welcome");

  async function handleScan() {
    scanning = true;
    try {
      const [b, g] = await Promise.all([getBottles(), getAllGames()]);
      detectedBottles = b;
      detectedGames = g;
      bottles.set(b);
      games.set(g);
    } catch {
      // Scan failed — user can still proceed
    }
    scanning = false;
  }

  async function handleNexusOAuth() {
    nexusConnecting = true;
    nexusError = null;
    try {
      await startOAuthLogin();
      const status = await getNexusAccountStatus();
      if (status.connected) {
        nexusConnected = true;
        nexusName = status.name ?? null;
      } else {
        nexusError = "Authorization completed but account check failed.";
      }
    } catch (e: unknown) {
      const msg = typeof e === "string" ? e : (e instanceof Error ? e.message : String(e));
      if (!msg.includes("Cancelled") && !msg.includes("timed out")) {
        nexusError = `Sign-in failed: ${msg}`;
      }
    } finally {
      nexusConnecting = false;
    }
  }

  function selectGame(game: DetectedGame) {
    selectedGame.set(game);
  }

  async function finishSetup() {
    await setConfigValue("has_completed_setup", "true");
    if ($selectedGame) {
      currentPage.set("mods");
    }
    onComplete();
  }

  async function skipSetup() {
    await setConfigValue("has_completed_setup", "true").catch((err) =>
      console.error("FirstRunWizard: persist skip failed:", err),
    );
    onComplete();
  }

  function nextStep() {
    if (step < steps.length - 1) step++;
  }

  function prevStep() {
    if (step > 0) step--;
  }

  async function handleSteamRegister() {
    steamRegistering = true;
    try {
      steamStatus = await addToSteam();
    } catch (e) {
      console.error("Steam registration failed:", e);
    }
    steamRegistering = false;
  }

  onMount(async () => {
    // Detect platform for conditional Steam step
    try {
      const status = await checkSteamStatus();
      steamStatus = status;
      isLinux = status.installed; // Steam detection only works on Linux
    } catch {
      // Not on Linux or Steam not available
    }
  });
</script>

<div class="wizard-overlay">
  <div class="wizard-card">
    <button class="wizard-skip" onclick={skipSetup} type="button" aria-label="Skip setup">
      Skip setup
    </button>
    <!-- Progress dots -->
    <div class="wizard-progress">
      {#each steps as s, i}
        <button
          class="progress-dot"
          class:active={i === step}
          class:done={i < step}
          onclick={() => { if (i < step) step = i; }}
          disabled={i > step}
        >
          {#if i < step}
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round">
              <polyline points="20 6 9 17 4 12" />
            </svg>
          {:else}
            {i + 1}
          {/if}
        </button>
        {#if i < steps.length - 1}
          <div class="progress-line" class:filled={i < step}></div>
        {/if}
      {/each}
    </div>

    <!-- Step: Welcome -->
    {#if currentStep === "Welcome"}
      <div class="wizard-step">
        <div class="welcome-icon">
          <svg width="48" height="48" viewBox="0 0 48 48" fill="none" stroke="var(--accent)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M8 6h32l2 8H6L8 6z" />
            <path d="M6 14v24a4 4 0 0 0 4 4h28a4 4 0 0 0 4-4V14" />
            <path d="M18 24v6" />
            <path d="M30 24v6" />
          </svg>
        </div>
        <h2 class="wizard-title">Welcome to Corkscrew</h2>
        <p class="wizard-desc">
          The mod manager for Wine, CrossOver, and Proton games on macOS and Linux.
          Let's get you set up in just a few steps.
        </p>
        <div class="wizard-features">
          <div class="feature">
            <span class="feature-icon">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z"/></svg>
            </span>
            <span>Install, organize, and deploy mods</span>
          </div>
          <div class="feature">
            <span class="feature-icon">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/></svg>
            </span>
            <span>Manage plugin load order with LOOT</span>
          </div>
          <div class="feature">
            <span class="feature-icon">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/></svg>
            </span>
            <span>Profiles, rollback, and version history</span>
          </div>
        </div>
      </div>

    <!-- Step: Detect Games -->
    {:else if currentStep === "Detect Games"}
      <div class="wizard-step">
        <h2 class="wizard-title">Detect Games</h2>
        <p class="wizard-desc">
          Scan your Wine bottles to find installed games. Corkscrew supports CrossOver, Moonshine, Heroic, Proton, Lutris, and more.
        </p>

        {#if !scanning && detectedBottles.length === 0}
          <button class="btn-primary btn-lg" onclick={handleScan}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="11" cy="11" r="8" />
              <line x1="21" y1="21" x2="16.65" y2="16.65" />
            </svg>
            Scan for Games
          </button>
        {:else if scanning}
          <div class="scan-state">
            <span class="spinner-md"></span>
            <span>Scanning Wine bottles...</span>
          </div>
        {:else}
          <div class="scan-results">
            <div class="scan-stat">
              <span class="scan-stat-value">{detectedBottles.length}</span>
              <span class="scan-stat-label">{detectedBottles.length === 1 ? "Bottle" : "Bottles"}</span>
            </div>
            <div class="scan-stat">
              <span class="scan-stat-value">{detectedGames.length}</span>
              <span class="scan-stat-label">{detectedGames.length === 1 ? "Game" : "Games"}</span>
            </div>
          </div>

          {#if detectedGames.length > 0}
            <div class="game-list">
              {#each detectedGames as game}
                <button
                  class="game-item"
                  class:selected={$selectedGame?.game_id === game.game_id && $selectedGame?.bottle_name === game.bottle_name}
                  onclick={() => selectGame(game)}
                >
                  <span class="game-item-name">{game.display_name}</span>
                  <span class="game-item-bottle">{game.bottle_name}</span>
                  {#if $selectedGame?.game_id === game.game_id && $selectedGame?.bottle_name === game.bottle_name}
                    <svg class="game-check" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--green)" stroke-width="3" stroke-linecap="round" stroke-linejoin="round">
                      <polyline points="20 6 9 17 4 12" />
                    </svg>
                  {/if}
                </button>
              {/each}
            </div>
            <p class="hint">Select a default game to manage</p>
          {:else}
            <p class="wizard-desc dim">No games found. You can install games later and scan again from the Dashboard.</p>
          {/if}
        {/if}
      </div>

    <!-- Step: Steam Integration (Linux only) -->
    {:else if currentStep === "Steam"}
      <div class="wizard-step">
        <div class="welcome-icon">
          <svg width="48" height="48" viewBox="0 0 48 48" fill="none" stroke="var(--accent)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="24" cy="24" r="20" />
            <path d="M24 12v24" />
            <path d="M12 24h24" />
            <circle cx="24" cy="24" r="6" fill="none" />
          </svg>
        </div>
        <h2 class="wizard-title">Steam Integration</h2>
        <p class="wizard-desc">
          Add Corkscrew to your Steam library so you can launch it from Gaming Mode on Steam Deck or from Steam's Big Picture.
        </p>

        {#if steamStatus?.registered}
          <div class="steam-status registered">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="var(--green)" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
              <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14" />
              <polyline points="22 4 12 14.01 9 11.01" />
            </svg>
            <span>Corkscrew is registered in Steam</span>
          </div>
        {:else}
          <button
            class="btn-primary btn-lg"
            onclick={handleSteamRegister}
            disabled={steamRegistering}
          >
            {#if steamRegistering}
              <span class="spinner-xs"></span> Registering...
            {:else}
              Add to Steam Library
            {/if}
          </button>
        {/if}

        {#if steamStatus?.is_deck}
          <p class="hint">Steam Deck detected — this will make Corkscrew accessible in Gaming Mode.</p>
        {:else}
          <p class="hint">Optional — you can also do this later from Settings.</p>
        {/if}
      </div>

    <!-- Step: Nexus Mods OAuth Sign-In -->
    {:else if currentStep === "Nexus Mods"}
      <div class="wizard-step">
        <h2 class="wizard-title">Nexus Mods (Optional)</h2>
        <p class="wizard-desc">
          Connect your NexusMods account to browse mods, download, and install collections. Opens your browser for a secure sign-in.
        </p>

        {#if nexusConnected}
          <div class="steam-status registered">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="var(--green)" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
              <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14" />
              <polyline points="22 4 12 14.01 9 11.01" />
            </svg>
            <span>Signed in{nexusName ? ` as ${nexusName}` : ''}</span>
          </div>
        {:else}
          <button
            class="btn-primary btn-lg"
            onclick={handleNexusOAuth}
            disabled={nexusConnecting}
          >
            {#if nexusConnecting}
              <span class="spinner-xs"></span> Connecting...
            {:else}
              Sign in with NexusMods
            {/if}
          </button>
        {/if}

        {#if nexusError}
          <p class="api-status error">{nexusError}</p>
        {/if}

        <p class="hint">
          You can also do this later from Settings.
        </p>
      </div>

    <!-- Step: Ready -->
    {:else if currentStep === "Ready"}
      <div class="wizard-step">
        <div class="ready-icon">
          <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="var(--green)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14" />
            <polyline points="22 4 12 14.01 9 11.01" />
          </svg>
        </div>
        <h2 class="wizard-title">You're All Set</h2>
        <p class="wizard-desc">
          {#if $selectedGame}
            Ready to manage mods for <strong>{$selectedGame.display_name}</strong>.
          {:else if detectedGames.length > 0}
            Select a game from the Dashboard to start managing mods.
          {:else}
            Install a game in a Wine bottle, then scan from the Dashboard.
          {/if}
        </p>
        <div class="ready-tips">
          <div class="tip">Drag & drop archives onto the Mods page to install</div>
          <div class="tip">Use LOOT Sort on the Load Order page for optimal plugin order</div>
          <div class="tip">Create profiles to save and switch between mod setups</div>
        </div>
      </div>
    {/if}

    <!-- Navigation buttons -->
    <div class="wizard-nav">
      {#if step > 0}
        <button class="btn-ghost" onclick={prevStep}>Back</button>
      {:else}
        <div></div>
      {/if}

      {#if step < steps.length - 1}
        <button
          class="btn-primary"
          onclick={() => { if (currentStep === "Detect Games" && detectedBottles.length === 0) handleScan().then(nextStep); else nextStep(); }}
        >
          {currentStep === "Welcome" ? "Get Started" : "Continue"}
        </button>
      {:else}
        <button class="btn-primary" onclick={finishSetup}>
          Start Modding
        </button>
      {/if}
    </div>
  </div>
</div>

<style>
  .wizard-overlay {
    position: fixed;
    inset: 0;
    z-index: 9999;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.7);
  }

  .wizard-card {
    width: 520px;
    max-height: 90vh;
    overflow-y: auto;
    background: var(--bg-base);
    border: 1px solid var(--separator);
    border-radius: var(--radius-lg);
    padding: var(--space-8);
    box-shadow: 0 24px 48px rgba(0, 0, 0, 0.4);
    position: relative;
  }

  .wizard-skip {
    position: absolute;
    top: var(--space-3);
    right: var(--space-4);
    background: none;
    border: none;
    color: var(--text-tertiary);
    font-size: 12px;
    font-weight: 500;
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius);
    cursor: pointer;
    transition: color var(--duration-fast) var(--ease), background var(--duration-fast) var(--ease);
  }

  .wizard-skip:hover {
    color: var(--text-primary);
    background: var(--surface-hover);
  }

  /* Progress */
  .wizard-progress {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0;
    margin-bottom: var(--space-8);
  }

  .progress-dot {
    width: 28px;
    height: 28px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-quaternary);
    background: var(--surface);
    border: 2px solid var(--separator);
    cursor: default;
    transition: all var(--duration-fast) var(--ease);
    flex-shrink: 0;
  }

  .progress-dot.active {
    color: white;
    background: var(--accent);
    border-color: var(--accent);
  }

  .progress-dot.done {
    color: var(--green);
    background: var(--green-subtle);
    border-color: var(--green);
    cursor: pointer;
  }

  .progress-line {
    width: 40px;
    height: 2px;
    background: var(--separator);
    transition: background var(--duration) var(--ease);
  }

  .progress-line.filled {
    background: var(--green);
  }

  /* Steps */
  .wizard-step {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    min-height: 260px;
  }

  .welcome-icon, .ready-icon {
    margin-bottom: var(--space-4);
    opacity: 0.9;
  }

  .wizard-title {
    font-size: 22px;
    font-weight: 700;
    color: var(--text-primary);
    margin-bottom: var(--space-2);
    letter-spacing: -0.02em;
  }

  .wizard-desc {
    font-size: 14px;
    color: var(--text-secondary);
    line-height: 1.55;
    max-width: 400px;
    margin-bottom: var(--space-5);
  }

  .wizard-desc.dim {
    color: var(--text-tertiary);
  }

  /* Features list */
  .wizard-features {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    text-align: left;
    width: 100%;
    max-width: 340px;
  }

  .feature {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    font-size: 13px;
    color: var(--text-secondary);
    padding: var(--space-2) var(--space-3);
    background: var(--surface);
    border-radius: var(--radius);
  }

  .feature-icon {
    color: var(--accent);
    flex-shrink: 0;
    display: flex;
  }

  /* Scan results */
  .scan-state {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-6);
    font-size: 14px;
    color: var(--text-secondary);
  }

  .scan-results {
    display: flex;
    gap: var(--space-6);
    margin-bottom: var(--space-4);
  }

  .scan-stat {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
  }

  .scan-stat-value {
    font-size: 28px;
    font-weight: 700;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .scan-stat-label {
    font-size: 12px;
    color: var(--text-tertiary);
    font-weight: 500;
  }

  /* Game list */
  .game-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    width: 100%;
    max-width: 360px;
    margin-bottom: var(--space-2);
  }

  .game-item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    background: var(--surface);
    cursor: pointer;
    text-align: left;
    transition: all var(--duration-fast) var(--ease);
  }

  .game-item:hover {
    border-color: var(--accent);
    background: var(--surface-hover);
  }

  .game-item.selected {
    border-color: var(--green);
    background: var(--green-subtle);
  }

  .game-item-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    flex: 1;
  }

  .game-item-bottle {
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .game-check {
    flex-shrink: 0;
  }

  .hint {
    font-size: 12px;
    color: var(--text-quaternary);
  }

  /* API Key */
  .api-key-input {
    display: flex;
    gap: var(--space-2);
    width: 100%;
    max-width: 400px;
    margin-bottom: var(--space-3);
  }

  .input-field {
    flex: 1;
    padding: var(--space-2) var(--space-3);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    color: var(--text-primary);
    font-size: 13px;
    font-family: var(--font-mono);
  }

  .input-field:focus {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-subtle);
  }

  .api-status {
    font-size: 13px;
    font-weight: 500;
    margin-bottom: var(--space-3);
  }

  .api-status.success {
    color: var(--green);
  }

  .api-status.error {
    color: var(--red);
  }

  /* Steam step */
  .steam-status {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius);
    font-size: 14px;
    font-weight: 500;
  }
  .steam-status.registered {
    background: color-mix(in srgb, var(--green) 12%, transparent);
    color: var(--green);
  }

  /* Ready tips */
  .ready-tips {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    text-align: left;
    width: 100%;
    max-width: 360px;
  }

  .tip {
    font-size: 13px;
    color: var(--text-secondary);
    padding: var(--space-2) var(--space-3);
    background: var(--surface);
    border-radius: var(--radius);
    border-left: 3px solid var(--accent);
  }

  /* Navigation */
  .wizard-nav {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-top: var(--space-6);
    padding-top: var(--space-4);
    border-top: 1px solid var(--separator);
  }

  /* Buttons */
  .btn-primary {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-5);
    background: var(--accent);
    color: white;
    font-size: 13px;
    font-weight: 600;
    border-radius: var(--radius);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
  }

  .btn-primary:hover {
    filter: brightness(1.1);
  }

  .btn-primary.btn-lg {
    padding: var(--space-3) var(--space-6);
    font-size: 14px;
  }

  .btn-secondary {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    background: var(--surface);
    border: 1px solid var(--separator);
    color: var(--text-primary);
    font-size: 13px;
    font-weight: 500;
    border-radius: var(--radius);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
  }

  .btn-secondary:hover {
    background: var(--surface-hover);
  }

  .btn-secondary:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-ghost {
    padding: var(--space-2) var(--space-4);
    background: none;
    color: var(--text-secondary);
    font-size: 13px;
    font-weight: 500;
    border-radius: var(--radius);
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
  }

  .btn-ghost:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  /* Spinners */
  .spinner-xs {
    display: inline-block;
    width: 14px;
    height: 14px;
    border: 2px solid var(--separator-opaque);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.75s linear infinite;
  }

  .spinner-md {
    display: inline-block;
    width: 24px;
    height: 24px;
    border: 2.5px solid var(--separator-opaque);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.75s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }
</style>
