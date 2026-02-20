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
  <!-- Skyrim Dragon (fallback SVG) -->
  <svg width={size} height={size} viewBox="0 0 64 64" fill="none" xmlns="http://www.w3.org/2000/svg">
    <g opacity="0.85">
      <path d="M32 3 L36 14 L32 18 L28 14 Z" fill="currentColor" opacity="0.6" />
      <path d="M28 14 L22 8 L26 15Z" fill="currentColor" opacity="0.45" />
      <path d="M36 14 L42 8 L38 15Z" fill="currentColor" opacity="0.45" />
      <path d="M32 18 L36 14 L38 22 L32 28 L26 22 L28 14Z" fill="currentColor" opacity="0.5" />
      <path d="M26 22 L18 16 L8 12 L12 20 L6 26 L14 28 L10 36 L18 32 L24 30 L26 22Z"
        fill="currentColor" opacity="0.35" stroke="currentColor" stroke-width="0.5" stroke-linejoin="round" />
      <path d="M38 22 L46 16 L56 12 L52 20 L58 26 L50 28 L54 36 L46 32 L40 30 L38 22Z"
        fill="currentColor" opacity="0.35" stroke="currentColor" stroke-width="0.5" stroke-linejoin="round" />
      <path d="M32 28 L36 26 L38 34 L32 42 L26 34 L28 26Z" fill="currentColor" opacity="0.45" />
      <path d="M26 34 L20 38 L18 44 L22 40 L26 42Z" fill="currentColor" opacity="0.3" />
      <path d="M38 34 L44 38 L46 44 L42 40 L38 42Z" fill="currentColor" opacity="0.3" />
      <path d="M32 42 L34 48 L36 52 L34 56 L32 60 L30 56 L28 52 L30 48Z" fill="currentColor" opacity="0.35" />
      <circle cx="30" cy="15" r="1" fill="currentColor" opacity="0.7" />
      <circle cx="34" cy="15" r="1" fill="currentColor" opacity="0.7" />
    </g>
  </svg>
{:else}
  <!-- Generic game icon fallback -->
  <svg width={size} height={size} viewBox="0 0 64 64" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <rect x="8" y="14" width="48" height="36" rx="6" opacity="0.3" />
    <circle cx="22" cy="32" r="6" opacity="0.3" />
    <circle cx="42" cy="32" r="6" opacity="0.3" />
    <line x1="32" y1="20" x2="32" y2="44" opacity="0.2" />
  </svg>
{/if}
