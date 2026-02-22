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
  aria-label="Wabbajack"
>
  <!-- Stylized Wabbajack hare/rabbit silhouette -->
  <path
    d="M12 2C10 2 9 3 9 5v3c-2 0-4 2-4 5 0 2 1 3.5 2.5 4.5L6 22h3l1-3h4l1 3h3l-1.5-4.5C18 16.5 19 15 19 13c0-3-2-5-4-5V5c0-2-1-3-3-3z"
    fill="#6D9FD6"
    opacity="0.15"
  />
  <path
    d="M12 2C10 2 9 3 9 5v3c-2 0-4 2-4 5 0 2 1 3.5 2.5 4.5L6 22h3l1-3h4l1 3h3l-1.5-4.5C18 16.5 19 15 19 13c0-3-2-5-4-5V5c0-2-1-3-3-3z"
    stroke="#6D9FD6"
    stroke-width="1.5"
    stroke-linejoin="round"
    fill="none"
  />
  <!-- Ears -->
  <path
    d="M10 5V2.5M14 5V2.5"
    stroke="#6D9FD6"
    stroke-width="1.5"
    stroke-linecap="round"
  />
  <!-- Eye -->
  <circle cx="12" cy="11" r="1.5" fill="#6D9FD6" />
</svg>
