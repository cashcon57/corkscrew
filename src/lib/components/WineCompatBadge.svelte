<script lang="ts">
  import type { VerifiedEntry, VerifiedStatus } from "$lib/types";
  import { getCollectionVerification, getWabbajackVerification } from "$lib/api";
  import { absoluteDate } from "$lib/relativeTime";

  interface Props {
    kind: "collection" | "wabbajack";
    /** For collections: game_domain (e.g. "skyrimspecialedition"). Ignored for wabbajack. */
    gameDomain?: string;
    /** For collections: slug. For wabbajack: modlist_name. */
    key: string;
    /** Compact mode renders just the badge without notes. */
    compact?: boolean;
  }

  let { kind, gameDomain = "", key, compact = false }: Props = $props();

  let entry = $state<VerifiedEntry | null>(null);
  let loading = $state(true);

  $effect(() => {
    const k = key;
    const kd = kind;
    const dom = gameDomain;
    if (!k) {
      entry = null;
      loading = false;
      return;
    }
    loading = true;
    const promise =
      kd === "collection"
        ? getCollectionVerification(dom, k)
        : getWabbajackVerification(k);
    promise
      .then((e) => {
        entry = e;
      })
      .catch((err) => {
        console.error("WineCompatBadge: fetch failed:", err);
        entry = null;
      })
      .finally(() => {
        loading = false;
      });
  });

  const label = $derived.by(() => {
    const s: VerifiedStatus | undefined = entry?.status;
    switch (s) {
      case "verified":
        return "Verified on Wine";
      case "partial":
        return "Partial on Wine";
      case "broken":
        return "Broken on Wine";
      case "untested":
      default:
        return "Untested on Wine";
    }
  });

  const tooltip = $derived.by(() => {
    if (!entry) return "";
    const parts: string[] = [];
    if (entry.version_tested) parts.push(`Tested: v${entry.version_tested}`);
    if (entry.last_verified) parts.push(`Verified: ${absoluteDate(entry.last_verified)}`);
    if (entry.notes) parts.push(entry.notes);
    if (entry.reporter) parts.push(`— ${entry.reporter}`);
    return parts.join(" · ");
  });
</script>

{#if !loading && entry}
  <span
    class="compat-badge"
    class:verified={entry.status === "verified"}
    class:partial={entry.status === "partial"}
    class:broken={entry.status === "broken"}
    class:untested={entry.status === "untested"}
    title={tooltip || label}
  >
    <span class="compat-dot" aria-hidden="true"></span>
    <span class="compat-label">{label}</span>
    {#if !compact && entry.notes}
      <span class="compat-notes">— {entry.notes}</span>
    {/if}
  </span>
{/if}

<style>
  .compat-badge {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 3px 8px;
    font-size: 11px;
    font-weight: 600;
    border-radius: var(--radius-sm);
    border: 1px solid var(--separator);
    background: var(--surface);
    color: var(--text-secondary);
    line-height: 1;
    white-space: nowrap;
    max-width: 100%;
  }

  .compat-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex-shrink: 0;
    background: var(--text-quaternary);
  }

  .compat-badge.verified {
    border-color: color-mix(in srgb, var(--green) 40%, var(--separator));
    background: color-mix(in srgb, var(--green) 10%, transparent);
    color: var(--green);
  }
  .compat-badge.verified .compat-dot {
    background: var(--green);
  }

  .compat-badge.partial {
    border-color: color-mix(in srgb, var(--yellow, #e8a82a) 40%, var(--separator));
    background: color-mix(in srgb, var(--yellow, #e8a82a) 10%, transparent);
    color: var(--yellow, #e8a82a);
  }
  .compat-badge.partial .compat-dot {
    background: var(--yellow, #e8a82a);
  }

  .compat-badge.broken {
    border-color: color-mix(in srgb, var(--red) 40%, var(--separator));
    background: color-mix(in srgb, var(--red) 10%, transparent);
    color: var(--red);
  }
  .compat-badge.broken .compat-dot {
    background: var(--red);
  }

  .compat-badge.untested {
    color: var(--text-tertiary);
  }

  .compat-notes {
    font-weight: 400;
    color: inherit;
    opacity: 0.85;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
