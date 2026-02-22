<script lang="ts">
  let { size = 16, class: className = "" }: { size?: number; class?: string } = $props();
</script>

<img
  src="/icons/nexusmods.png"
  alt="Nexus Mods"
  width={size}
  height={size}
  class={className}
  style="display: inline-block; vertical-align: middle; object-fit: contain;"
/>
