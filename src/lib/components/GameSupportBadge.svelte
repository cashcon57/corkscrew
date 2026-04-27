<script lang="ts">
  /**
   * Tiny coloured-dot badge advertising how well-supported a game is.
   *
   * The badge fetches the tier asynchronously via `getGameSupportTier` and
   * caches the result, so it is safe to render many of these at once
   * (game grids, dropdowns, etc.). Style is modelled on `WineCompatBadge`.
   */
  import { getTier, tierLabel, tierTooltip } from "$lib/gameSupport";
  import type { GameSupportTier } from "$lib/types";

  interface Props {
    gameId: string;
    /** Compact mode: only the dot, no text. Default `false`. */
    compact?: boolean;
    /**
     * If true, hide the badge entirely for the "verified" tier. Reduces
     * visual noise on the dashboard where most games will be verified.
     */
    hideWhenVerified?: boolean;
  }

  let {
    gameId,
    compact = false,
    hideWhenVerified = false,
  }: Props = $props();

  let tier = $state<GameSupportTier | null>(null);

  $effect(() => {
    const id = gameId;
    if (!id) {
      tier = null;
      return;
    }
    let cancelled = false;
    getTier(id).then((t) => {
      if (!cancelled) tier = t;
    });
    return () => {
      cancelled = true;
    };
  });

  const label = $derived(tier ? tierLabel(tier) : "");
  const tooltip = $derived(tier ? tierTooltip(tier) : "");
  const hidden = $derived(
    tier === null || (hideWhenVerified && tier === "verified")
  );
</script>

{#if !hidden && tier}
  <span
    class="support-badge"
    class:verified={tier === "verified"}
    class:experimental={tier === "experimental"}
    class:vortex-extension={tier === "vortex_extension"}
    class:vortex-registry={tier === "vortex_registry"}
    class:unknown={tier === "unknown"}
    class:compact
    title={tooltip}
  >
    <span class="support-dot" aria-hidden="true"></span>
    {#if !compact}
      <span class="support-label">{label}</span>
    {/if}
  </span>
{/if}

<style>
  .support-badge {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 2px 7px;
    font-size: 10.5px;
    font-weight: 600;
    border-radius: var(--radius-sm);
    border: 1px solid var(--separator);
    background: var(--surface);
    color: var(--text-secondary);
    line-height: 1;
    white-space: nowrap;
    letter-spacing: 0.01em;
  }

  .support-badge.compact {
    padding: 0;
    border: none;
    background: transparent;
    gap: 0;
  }

  .support-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex-shrink: 0;
    background: var(--text-quaternary);
  }

  .support-badge.verified {
    border-color: color-mix(in srgb, var(--green) 40%, var(--separator));
    background: color-mix(in srgb, var(--green) 10%, transparent);
    color: var(--green);
  }
  .support-badge.verified .support-dot {
    background: var(--green);
  }

  .support-badge.experimental {
    border-color: color-mix(in srgb, var(--yellow, #e8a82a) 40%, var(--separator));
    background: color-mix(in srgb, var(--yellow, #e8a82a) 10%, transparent);
    color: var(--yellow, #e8a82a);
  }
  .support-badge.experimental .support-dot {
    background: var(--yellow, #e8a82a);
  }

  /* Vortex extension — distinct from "experimental" so the user can tell
     "we plugged in someone else's code" apart from "we wrote untested code". */
  .support-badge.vortex-extension {
    border-color: color-mix(in srgb, var(--purple, #a371f7) 40%, var(--separator));
    background: color-mix(in srgb, var(--purple, #a371f7) 10%, transparent);
    color: var(--purple, #a371f7);
  }
  .support-badge.vortex-extension .support-dot {
    background: var(--purple, #a371f7);
  }

  .support-badge.vortex-registry {
    color: var(--text-tertiary);
  }
  .support-badge.vortex-registry .support-dot {
    background: var(--text-tertiary);
  }

  .support-badge.unknown {
    border-color: color-mix(in srgb, var(--red) 40%, var(--separator));
    background: color-mix(in srgb, var(--red) 10%, transparent);
    color: var(--red);
  }
  .support-badge.unknown .support-dot {
    background: var(--red);
  }
</style>
