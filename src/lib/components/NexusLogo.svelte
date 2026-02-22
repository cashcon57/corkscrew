<script lang="ts">
  let { size = 16, class: className = "" }: { size?: number; class?: string } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  xmlns="http://www.w3.org/2000/svg"
  class={className}
  role="img"
  aria-label="Nexus Mods"
>
  <!-- Stylized Nexus vortex/arrow mark -->
  <path
    d="M12 2L4 6v12l8 4 8-4V6l-8-4z"
    fill="#D98F40"
    opacity="0.15"
  />
  <path
    d="M12 2L4 6v12l8 4 8-4V6l-8-4z"
    stroke="#D98F40"
    stroke-width="1.5"
    stroke-linejoin="round"
    fill="none"
  />
  <path
    d="M8 15V9l8 6V9"
    stroke="#D98F40"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
  />
</svg>
