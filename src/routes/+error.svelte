<script lang="ts">
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
</script>

<div class="error-page">
  <div class="error-content">
    <h1 class="error-code">{$page.status}</h1>
    <p class="error-message">{$page.error?.message || "Page not found"}</p>
    <p class="error-hint">This page doesn't exist in the app. You may have clicked an external link.</p>
    <div class="error-actions">
      <button class="btn btn-primary" onclick={() => goto("/collections")}>
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M19 12H5" />
          <polyline points="12 19 5 12 12 5" />
        </svg>
        Back to Discover
      </button>
      <button class="btn btn-ghost" onclick={() => history.back()}>Go Back</button>
    </div>
  </div>
</div>

<style>
  .error-page {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 60vh;
    padding: 48px;
  }

  .error-content {
    text-align: center;
    max-width: 400px;
  }

  .error-code {
    font-size: 64px;
    font-weight: 700;
    color: var(--text-primary);
    margin-bottom: 8px;
    line-height: 1;
  }

  .error-message {
    font-size: 16px;
    color: var(--text-secondary);
    margin-bottom: 8px;
  }

  .error-hint {
    font-size: 13px;
    color: var(--text-tertiary);
    margin-bottom: 24px;
  }

  .error-actions {
    display: flex;
    gap: 12px;
    justify-content: center;
  }
</style>
