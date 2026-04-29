<script lang="ts">
  import { getGameLogo } from "$lib/api";

  interface Props {
    gameId: string;
    /**
     * Optional Steam App ID. When provided, the backend uses it directly
     * instead of looking it up in the bundled Vortex registry — this
     * matters for games discovered via the Steam appmanifest scanner that
     * have no registry entry (e.g. Tainted Grail, The Midnight Walk,
     * Mewgenics, anything indie/recent).
     */
    steamAppId?: string;
    size?: number;
  }

  let { gameId, steamAppId, size = 48 }: Props = $props();

  let logoUrl = $state<string | null>(null);

  // Use $effect so the icon updates when either prop changes.
  $effect(() => {
    const id = gameId;
    const sid = steamAppId;
    logoUrl = null;
    getGameLogo(id, sid)
      .then((dataUrl) => {
        if (dataUrl) logoUrl = dataUrl;
      })
      .catch((err) => console.warn('Failed to fetch game logo:', err));
  });
</script>

{#if logoUrl}
  <img
    src={logoUrl}
    alt={gameId}
    width={size}
    height={size}
    style="object-fit: contain; filter: drop-shadow(0 1px 3px rgba(0,0,0,0.3));"
  />
{:else}
  <!-- NES cartridge placeholder -->
  <svg width={size} height={size} viewBox="0 0 24 28" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" opacity="0.35">
    <rect x="1.5" y="1.5" width="21" height="20" rx="2" />
    <rect x="3.5" y="3" width="17" height="13" rx="1" />
    <line x1="1.5" y1="21.5" x2="22.5" y2="21.5" />
    <line x1="5" y1="21.5" x2="5" y2="26.5" />
    <line x1="8" y1="21.5" x2="8" y2="26.5" />
    <line x1="11" y1="21.5" x2="11" y2="26.5" />
    <line x1="14" y1="21.5" x2="14" y2="26.5" />
    <line x1="17" y1="21.5" x2="17" y2="26.5" />
    <line x1="20" y1="21.5" x2="20" y2="26.5" />
  </svg>
{/if}
