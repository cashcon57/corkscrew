<script lang="ts">
  import { onMount } from "svelte";
  import { getGameLogo } from "$lib/api";

  interface Props {
    gameId: string;
    size?: number;
  }

  let { gameId, size = 48 }: Props = $props();

  let logoUrl = $state<string | null>(null);

  onMount(async () => {
    try {
      const dataUrl = await getGameLogo(gameId);
      if (dataUrl) {
        logoUrl = dataUrl;
      }
    } catch {
      // Fetch failed — use SVG fallback
    }
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
{:else if gameId === "skyrimse" || gameId === "skyrim"}
  <!-- Skyrim Dragon logo -->
  <img
    src="/skyrim-logo-for-your-skyrim-needs-silver-dragon-png-clipart.jpg"
    alt="Skyrim"
    width={size}
    height={size}
    style="object-fit: contain; filter: drop-shadow(0 1px 3px rgba(0,0,0,0.3));"
  />
{:else}
  <!-- Generic game icon fallback -->
  <svg width={size} height={size} viewBox="0 0 64 64" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <rect x="8" y="14" width="48" height="36" rx="6" opacity="0.3" />
    <circle cx="22" cy="32" r="6" opacity="0.3" />
    <circle cx="42" cy="32" r="6" opacity="0.3" />
    <line x1="32" y1="20" x2="32" y2="44" opacity="0.2" />
  </svg>
{/if}
