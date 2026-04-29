<script lang="ts">
  import { get } from "svelte/store";
  import { onDestroy } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import {
    installMod,
    onInstallProgress,
    getAvailableDiskSpace,
    setModCollectionName,
    listInstalledCollections,
  } from "$lib/api";
  import type { InstallProgressEvent, DetectedGame, InstalledMod } from "$lib/types";
  import { wineCtx } from "$lib/types";
  import {
    showError,
    showSuccess,
    activeCollection,
    collectionList,
    collectionInstallStatus,
    gameLock,
    gameLockOverridden,
  } from "$lib/stores";

  interface Props {
    game: DetectedGame;
    installing: boolean;
    installStep: string;
    installDetail: string;
    onInstallComplete: (mod: InstalledMod) => Promise<void>;
  }

  let {
    game,
    installing = $bindable(false),
    installStep = $bindable(""),
    installDetail = $bindable(""),
    onInstallComplete,
  }: Props = $props();
  let showModlistNamePrompt = $state(false);
  let modlistNameInput = $state("User");
  let pendingInstallFilePath = $state<string | null>(null);
  let installUnlisten: (() => void) | null = null;

  const stepLabels: Record<string, string> = {
    preparing: "Preparing...",
    extracting: "Extracting archive...",
    registering: "Recording files...",
    deploying: "Deploying to game...",
    "syncing-plugins": "Syncing plugins...",
  };

  onDestroy(() => {
    if (installUnlisten) { installUnlisten(); installUnlisten = null; }
  });

  /** Open file picker and start mod install. Exposed for parent use (e.g., reinstall). */
  export async function handleInstall() {
    const installStatus = get(collectionInstallStatus);
    if (installStatus?.active) {
      showError('Cannot modify mods while a collection is being installed');
      return;
    }

    const filePath = await open({
      multiple: false,
      filters: [
        {
          name: "Mod Archives",
          extensions: ["zip", "7z", "rar"],
        },
      ],
    });

    if (!filePath) return;

    // If no modlist is active, prompt the user to name their current modlist
    if (!$activeCollection) {
      pendingInstallFilePath = filePath as string;
      modlistNameInput = "User";
      showModlistNamePrompt = true;
      return;
    }

    await doInstallMod(filePath as string);
  }

  async function confirmModlistName() {
    const name = modlistNameInput.trim();
    if (!name) return;
    showModlistNamePrompt = false;
    activeCollection.set({ name, mod_count: 0, enabled_count: 0, slug: null, author: null, image_url: null, game_domain: null, installed_revision: null, original_mod_count: null, game_versions: [] });
    if (pendingInstallFilePath) {
      await doInstallMod(pendingInstallFilePath);
      pendingInstallFilePath = null;
      // Reload collections for the top bar
      try {
        const collections = await listInstalledCollections(game.game_id, (wineCtx(game)?.bottle_name ?? ""));
        collectionList.set(collections);
      } catch { /* non-critical */ }
    }
  }

  async function doInstallMod(filePath: string) {
    if ($gameLock && !$gameLockOverridden) {
      showError('Cannot install mods while the game is running. Close the game or click "Unlock Anyway".');
      return;
    }

    // Check available disk space before installing
    try {
      const parentDir = filePath.substring(0, filePath.lastIndexOf("/")) || "/";
      const freeBytes = await getAvailableDiskSpace(parentDir);
      const GB = 1024 * 1024 * 1024;
      if (freeBytes < 0.5 * GB) {
        showError("Not enough disk space (< 500 MB free). Free up space before installing.");
        return;
      }
      if (freeBytes < 2 * GB) {
        showError("Low disk space warning: less than 2 GB free. Install will proceed, but consider freeing space.");
      }
    } catch {
      // Non-critical — proceed even if space check fails
    }

    installing = true;
    installStep = "preparing";
    installDetail = "";

    // Subscribe to progress events
    try {
      installUnlisten = await onInstallProgress((event: InstallProgressEvent) => {
        if (event.kind === "stepChanged") {
          installStep = event.step;
          installDetail = event.detail ?? "";
        } else if (event.kind === "modCompleted") {
          installStep = "complete";
          installDetail = "";
        } else if (event.kind === "modFailed") {
          installStep = "failed";
          installDetail = event.error;
        }
      });

      const mod = await installMod(
        filePath,
        game.game_id,
        (wineCtx(game)?.bottle_name ?? "")
      );
      if (!mod || typeof mod !== 'object' || !('nexus_mod_id' in (mod as any))) {
        console.error('Unexpected install result:', mod);
        return;
      }
      const installed = mod as InstalledMod;

      // Associate mod with the active modlist
      if ($activeCollection) {
        try {
          await setModCollectionName(installed.id, $activeCollection.name);
        } catch { /* non-critical */ }
      }

      showSuccess(`Installed "${installed.name}" successfully`);
      await onInstallComplete(installed);
    } catch (e: unknown) {
      showError(`Install failed: ${e}`);
    } finally {
      installing = false;
      installStep = "";
      installDetail = "";
      if (installUnlisten) { installUnlisten(); installUnlisten = null; }
    }
  }
</script>

<!-- Install Button -->
<button class="btn btn-primary" onclick={handleInstall} disabled={installing}>
  {#if installing}
    <span class="spinner"></span>
    {stepLabels[installStep] ?? "Installing..."}
  {:else}
    <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
      <line x1="7" y1="2" x2="7" y2="12" />
      <line x1="2" y1="7" x2="12" y2="7" />
    </svg>
    Install Mod
  {/if}
</button>

<!-- Modlist Name Prompt Dialog -->
{#if showModlistNamePrompt}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="modlist-prompt-overlay" onclick={() => { showModlistNamePrompt = false; pendingInstallFilePath = null; }}>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="modlist-prompt-card" onclick={(e) => e.stopPropagation()}>
      <h3 class="modlist-prompt-title">Name Your Modlist</h3>
      <p class="modlist-prompt-desc">You don't have an active modlist. Name your current mod setup so new mods are grouped together.</p>
      <form onsubmit={(e) => { e.preventDefault(); confirmModlistName(); }}>
        <input
          class="modlist-prompt-input"
          type="text"
          bind:value={modlistNameInput}
          placeholder="Modlist name..."
          autofocus
          onkeydown={(e) => { if (e.key === "Escape") { showModlistNamePrompt = false; pendingInstallFilePath = null; } }}
        />
        <div class="modlist-prompt-actions">
          <button type="button" class="btn btn-ghost" onclick={() => { showModlistNamePrompt = false; pendingInstallFilePath = null; }}>Cancel</button>
          <button type="submit" class="btn btn-primary" disabled={!modlistNameInput.trim()}>Continue</button>
        </div>
      </form>
    </div>
  </div>
{/if}

<style>
  .modlist-prompt-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 200;
    animation: fadeIn 0.15s ease-out;
  }

  .modlist-prompt-card {
    background: var(--bg-grouped);
    border: 1px solid var(--separator-opaque);
    border-radius: var(--radius-lg, 12px);
    padding: 24px;
    max-width: 400px;
    width: 90%;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
  }

  .modlist-prompt-title {
    font-size: 16px;
    font-weight: 600;
    margin: 0 0 8px;
  }

  .modlist-prompt-desc {
    font-size: 13px;
    color: var(--text-secondary);
    margin: 0 0 16px;
    line-height: 1.5;
  }

  .modlist-prompt-input {
    width: 100%;
    padding: 8px 12px;
    font-size: 14px;
    background: var(--surface);
    border: 1px solid var(--separator-opaque);
    border-radius: var(--radius);
    color: var(--text-primary);
    outline: none;
    margin-bottom: 16px;
  }

  .modlist-prompt-input:focus {
    border-color: var(--accent);
  }

  .modlist-prompt-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
