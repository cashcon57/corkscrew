<script lang="ts">
  import { openUrl } from "@tauri-apps/plugin-opener";

  interface Props {
    fromVersion: string;
    toVersion: string;
    onclose: () => void;
  }

  let { fromVersion, toVersion, onclose }: Props = $props();

  // Changelog entries — add new versions at the top.
  // Keep it casual, quippy, and helpful.
  const changelogs: Record<string, { headline: string; changes: string[] }> = {
    "0.9.57": {
      headline: "Hogwarts Legacy got the glow-up it deserved.",
      changes: [
        "Collection pages now show 'View on Nexus Mods' link, last updated date, and full revision history — so you know what you're getting into before you install",
        "UE mod deployment completely rewritten — PAK files go to ~mods, Lua mods go to Win64/Mods, DLLs go to Win64, and junk files get tossed in the bin where they belong",
        "Fixed a crash caused by UE4SS setting up camp in Paks/Tools/ — turns out the game scans that directory and loads any DLLs it finds. Surprise!",
        "PAK files now deploy flat to ~mods/ root instead of preserving archive directory structures (Modern_Glasses/Style A/Normal Size/Clear/... no thanks)",
        "Collection uninstall now properly cleans up UE4SS, merged PAK databases, and leftover Tools directories",
        "OAuth SSO is now the primary sign-in on the Collections page — API key is still there if you need it",
        "Auto-switches to the correct game when installing a collection for a different game than what's selected",
        "Removed the external Mod Merger tool notification — Corkscrew handles PAK database merging natively",
      ],
    },
    "0.9.56": {
      headline: "Major plumbing work. You'll barely notice, but your mods will.",
      changes: [
        "Comprehensive UE PAK deploy filter for all Unreal Engine games",
        "Toxic Paks/Tools directory detection and cleanup",
        "UE4SS install path corrected — no more DLLs where they shouldn't be",
        "HL crash diagnostic script for isolating mod issues",
      ],
    },
    "0.9.55": {
      headline: "Quick fixes, because we care about the details.",
      changes: [
        "OAuth SSO added to Collections page",
        "Removed unnecessary Mod Merger external tool notification",
        "Game icon MIME type fix for cached images",
      ],
    },
    "0.9.54": {
      headline: "Hogwarts Legacy can now actually launch. You're welcome.",
      changes: [
        "Fixed HL launch — uses root launcher for Steam DRM instead of Phoenix binary",
        "Game detection vs launch executable separation via new GamePlugin trait",
      ],
    },
    "0.9.53": {
      headline: "A whole lot of things that should have been working are now working.",
      changes: [
        "Game directory cleaner made game-agnostic — works for any game, not just Skyrim",
        "SSE Engine Fixes for Wine is now opt-in (it's in development, might cause issues)",
        "Wabbajack: game file sources, single-file archives, and Nexus domain mapping all fixed",
        "Wabbajack: progress reporting won't go silent anymore — 2-second heartbeat ensures the UI stays responsive",
        "HL: UE4SS auto-install, PAK database merging, Mods.txt sync — all working",
        "HL: SQLite merger now disables FK constraints (no more missing creature definitions)",
        "Collection bundled mods now extract from the .7z archive automatically",
        "Duplicate mod names get suffixed so you can tell them apart",
      ],
    },
  };

  // Compare version strings numerically (not lexicographically)
  function compareVersions(a: string, b: string): number {
    const pa = a.split(".").map(Number);
    const pb = b.split(".").map(Number);
    for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
      const na = pa[i] ?? 0;
      const nb = pb[i] ?? 0;
      if (na !== nb) return na - nb;
    }
    return 0;
  }

  // Find which versions are relevant (between from and to)
  const relevantVersions = Object.keys(changelogs).filter((v) => {
    return v === toVersion
      || (compareVersions(v, fromVersion) > 0 && compareVersions(v, toVersion) <= 0);
  });
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="whats-new-overlay" onclick={onclose}>
  <div class="whats-new-modal" onclick={(e) => e.stopPropagation()}>
    <div class="whats-new-header">
      <div class="whats-new-badge">NEW</div>
      <h2 class="whats-new-title">What's New in v{toVersion}</h2>
      <button class="whats-new-close" onclick={onclose} type="button">&times;</button>
    </div>

    <div class="whats-new-body">
      {#each relevantVersions as version (version)}
        {@const entry = changelogs[version]}
        {#if entry}
          <div class="whats-new-version">
            {#if relevantVersions.length > 1}
              <h3 class="version-label">v{version}</h3>
            {/if}
            <p class="version-headline">{entry.headline}</p>
            <ul class="version-changes">
              {#each entry.changes as change}
                <li>{change}</li>
              {/each}
            </ul>
          </div>
        {/if}
      {/each}

      {#if relevantVersions.length === 0}
        <p class="version-headline">Bug fixes and improvements. Nothing too exciting, but everything's a little better now.</p>
      {/if}
    </div>

    <div class="whats-new-footer">
      <button
        class="btn-link-footer"
        onclick={() => openUrl(`https://github.com/cashcon57/corkscrew/releases/tag/v${toVersion}`)}
        type="button"
      >
        Full release notes on GitHub
      </button>
      <button class="btn btn-accent" onclick={onclose} type="button">
        Got it!
      </button>
    </div>
  </div>
</div>

<style>
  .whats-new-overlay {
    position: fixed;
    inset: 0;
    z-index: 9999;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    backdrop-filter: blur(4px);
  }

  .whats-new-modal {
    background: var(--bg-primary);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg, 12px);
    width: min(520px, 90vw);
    max-height: 80vh;
    display: flex;
    flex-direction: column;
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.4);
  }

  .whats-new-header {
    display: flex;
    align-items: center;
    gap: var(--space-3, 12px);
    padding: var(--space-4, 16px) var(--space-5, 20px);
    border-bottom: 1px solid var(--border);
  }

  .whats-new-badge {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.5px;
    color: white;
    background: var(--accent, #4a9eff);
    padding: 2px 8px;
    border-radius: 4px;
  }

  .whats-new-title {
    flex: 1;
    font-size: 16px;
    font-weight: 600;
    margin: 0;
  }

  .whats-new-close {
    background: none;
    border: none;
    color: var(--text-tertiary);
    font-size: 22px;
    cursor: pointer;
    padding: 0 4px;
    line-height: 1;
  }
  .whats-new-close:hover {
    color: var(--text-primary);
  }

  .whats-new-body {
    padding: var(--space-4, 16px) var(--space-5, 20px);
    overflow-y: auto;
    flex: 1;
  }

  .whats-new-version {
    margin-bottom: var(--space-4, 16px);
  }
  .whats-new-version:last-child {
    margin-bottom: 0;
  }

  .version-label {
    font-size: 13px;
    font-weight: 600;
    color: var(--accent, #4a9eff);
    margin: 0 0 4px;
  }

  .version-headline {
    font-size: 14px;
    font-weight: 500;
    color: var(--text-primary);
    margin: 0 0 8px;
    font-style: italic;
  }

  .version-changes {
    margin: 0;
    padding-left: 20px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .version-changes li {
    font-size: 13px;
    color: var(--text-secondary);
    line-height: 1.45;
  }

  .whats-new-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-3, 12px) var(--space-5, 20px);
    border-top: 1px solid var(--border);
  }

  .btn-link-footer {
    background: none;
    border: none;
    color: var(--text-tertiary);
    font-size: 12px;
    cursor: pointer;
    text-decoration: underline;
    text-decoration-color: transparent;
    transition: text-decoration-color 0.15s;
  }
  .btn-link-footer:hover {
    text-decoration-color: var(--text-tertiary);
  }
</style>
