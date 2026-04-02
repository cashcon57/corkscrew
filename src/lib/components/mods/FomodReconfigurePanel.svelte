<script lang="ts">
  import FomodWizard from "$lib/components/FomodWizard.svelte";
  import {
    detectFomod,
    getFomodRecipe,
    getFomodFiles,
    saveFomodRecipe,
    redeployAllMods,
  } from "$lib/api";
  import { showError, showSuccess } from "$lib/stores";
  import type { DetectedGame, InstalledMod, FomodInstaller } from "$lib/types";

  let {
    game,
    deploying = $bindable(false),
    onComplete,
  }: {
    game: DetectedGame;
    deploying: boolean;
    onComplete: () => Promise<void>;
  } = $props();

  let showFomodWizard = $state(false);
  let fomodInstaller = $state<FomodInstaller | null>(null);
  let fomodTargetMod = $state<InstalledMod | null>(null);

  /** Called by parent to open FOMOD reconfigure for a mod. */
  export function reconfigure(mod: InstalledMod) {
    handleReconfigureFomod(mod);
  }

  /** Called by parent to open FOMOD wizard directly with an installer and mod. */
  export function triggerFomod(mod: InstalledMod, installer: FomodInstaller) {
    fomodInstaller = installer;
    fomodTargetMod = mod;
    showFomodWizard = true;
  }

  async function handleReconfigureFomod(mod: InstalledMod) {
    if (!mod.staging_path) return;
    try {
      const installer = await detectFomod(mod.staging_path);
      if (!installer) {
        showError("No FOMOD installer found in this mod's staging folder.");
        return;
      }
      // Load previous recipe to pre-populate selections
      const recipe = await getFomodRecipe(mod.id);
      if (recipe) {
        // Pre-apply saved selections into the installer (the wizard's loadDefaults will handle this)
      }
      fomodInstaller = installer;
      fomodTargetMod = mod;
      showFomodWizard = true;
    } catch (e: unknown) {
      showError(`Failed to detect FOMOD: ${e}`);
    }
  }

  async function handleFomodComplete(selections: Record<string, string[]>) {
    if (!fomodTargetMod || !fomodInstaller) return;
    if (deploying) return;
    showFomodWizard = false;
    deploying = true;
    try {
      // Get the files for the new selections
      const files = await getFomodFiles(fomodInstaller, selections);
      // Save the recipe
      await saveFomodRecipe(fomodTargetMod.id, fomodTargetMod.name, "", selections);
      // Redeploy to apply changes
      await redeployAllMods(game.game_id, game.bottle_name);
      await onComplete();
      showSuccess(`Reconfigured FOMOD for "${fomodTargetMod.name}"`);
    } catch (e: unknown) {
      showError(`Failed to apply FOMOD configuration: ${e}`);
    } finally {
      deploying = false;
      fomodInstaller = null;
      fomodTargetMod = null;
    }
  }
</script>

{#if showFomodWizard && fomodInstaller}
  <div class="fomod-wizard-overlay">
    <FomodWizard
      installer={fomodInstaller}
      onComplete={handleFomodComplete}
      onCancel={() => { showFomodWizard = false; fomodInstaller = null; fomodTargetMod = null; }}
    />
  </div>
{/if}

<style>
  .fomod-wizard-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    backdrop-filter: var(--glass-blur-light);
    -webkit-backdrop-filter: var(--glass-blur-light);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 2000;
  }
</style>
