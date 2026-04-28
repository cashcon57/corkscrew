<script lang="ts">
  /**
   * regulation.bin conflict warning banner.
   *
   * `regulation.bin` is the master gameplay-rules archive in Sekiro,
   * Elden Ring, DS3, and AC6. When two enabled mods both ship a
   * regulation.bin, they silently overwrite each other at deploy
   * time. We can't safely auto-merge (multi-session RE work per game),
   * but we *can* warn the user and tell them what their options are.
   *
   * The banner refreshes whenever (gameId, bottleName) changes or when
   * the parent page passes a `refreshKey` (e.g. after toggling a mod).
   */

  import { getRegulationConflicts } from "$lib/api";
  import type { RegulationConflict } from "$lib/types";

  interface Props {
    gameId: string;
    bottleName: string;
    /** Bump to force re-fetch (e.g. after a mod toggle). */
    refreshKey?: number;
  }

  let { gameId, bottleName, refreshKey = 0 }: Props = $props();

  let conflicts = $state<RegulationConflict[]>([]);
  let expanded = $state(false);

  $effect(() => {
    // Re-fetch when any of the inputs change.
    void gameId;
    void bottleName;
    void refreshKey;
    void load();
  });

  async function load() {
    try {
      conflicts = await getRegulationConflicts(gameId, bottleName);
    } catch (err) {
      console.error("RegulationConflictBanner.load:", err);
      conflicts = [];
    }
  }
</script>

{#if conflicts.length > 0}
  <div class="reg-banner" role="alert">
    <div class="reg-row">
      <span class="reg-icon" aria-hidden="true">!</span>
      <div class="reg-text">
        <strong>regulation.bin conflict.</strong>
        {#each conflicts as c (c.game_id)}
          {c.mod_names.length} enabled mods both modify the master gameplay rules:
          <em>{c.mod_names.join(", ")}</em>.
        {/each}
      </div>
      <button class="reg-toggle" onclick={() => (expanded = !expanded)}>
        {expanded ? "Hide details" : "What does this mean?"}
      </button>
    </div>

    {#if expanded}
      <div class="reg-detail">
        <p>
          Mod Engine 2 deploys mods in order: when two mods both ship a
          <code>regulation.bin</code>, only the last one wins. The other
          mod's gameplay changes silently disappear.
        </p>
        <p><strong>Your options:</strong></p>
        <ul>
          <li>Disable one of the conflicting mods.</li>
          <li>
            Merge the two regulation files using <a
              href="https://github.com/vawser/Smithbox"
              target="_blank"
              rel="noopener noreferrer">Smithbox</a
            >
            or
            <a
              href="https://github.com/vawser/Yapped-Rune-Bear"
              target="_blank"
              rel="noopener noreferrer">Yapped Rune Bear</a
            >
            (auto-installable from the Tools panel).
          </li>
          <li>
            Reorder the mods in the Mod Engine 2 panel — later entries
            win, so place the more important regulation.bin last.
          </li>
        </ul>
      </div>
    {/if}
  </div>
{/if}

<style>
  .reg-banner {
    background: rgba(220, 180, 60, 0.08);
    border: 1px solid rgba(220, 180, 60, 0.5);
    border-radius: 6px;
    padding: 0.65rem 0.85rem;
    margin: 0.5rem 0 0.75rem 0;
    color: var(--fg, #e8e8ee);
  }
  .reg-row {
    display: flex;
    align-items: center;
    gap: 0.65rem;
  }
  .reg-icon {
    flex: 0 0 auto;
    width: 22px;
    height: 22px;
    line-height: 22px;
    text-align: center;
    border-radius: 50%;
    background: #d4a02a;
    color: #1a1a1f;
    font-weight: 700;
  }
  .reg-text {
    flex: 1;
    font-size: 0.9rem;
    line-height: 1.35;
  }
  .reg-toggle {
    flex: 0 0 auto;
    background: transparent;
    border: 1px solid rgba(220, 180, 60, 0.6);
    color: var(--fg, #e8e8ee);
    padding: 4px 10px;
    border-radius: 4px;
    cursor: pointer;
    font-size: 0.8rem;
  }
  .reg-toggle:hover {
    background: rgba(220, 180, 60, 0.12);
  }
  .reg-detail {
    margin-top: 0.6rem;
    padding-top: 0.6rem;
    border-top: 1px solid rgba(220, 180, 60, 0.25);
    font-size: 0.85rem;
    line-height: 1.45;
  }
  .reg-detail p {
    margin: 0.35rem 0;
  }
  .reg-detail ul {
    margin: 0.25rem 0 0 1rem;
    padding: 0;
  }
  .reg-detail code {
    background: rgba(255, 255, 255, 0.06);
    padding: 1px 4px;
    border-radius: 3px;
    font-family: var(--font-mono, monospace);
    font-size: 0.85em;
  }
  .reg-detail a {
    color: var(--accent, #66a8ff);
  }
</style>
