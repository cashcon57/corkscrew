<script lang="ts">
  import { getNotificationLog, clearNotificationLog } from "$lib/api";
  import { showNotificationLog, notificationCount } from "$lib/stores";
  import type { NotificationEntry } from "$lib/types";

  let notifications = $state<NotificationEntry[]>([]);
  let loading = $state(false);

  $effect(() => {
    if ($showNotificationLog) {
      loadNotifications();
    }
  });

  async function loadNotifications() {
    loading = true;
    try {
      notifications = await getNotificationLog(100);
    } catch {
      notifications = [];
    } finally {
      loading = false;
    }
  }

  async function handleClear() {
    await clearNotificationLog();
    notifications = [];
    notificationCount.set(0);
  }

  function close() {
    showNotificationLog.set(false);
  }

  function levelIcon(level: string): string {
    switch (level) {
      case "success": return "\u2714";
      case "error": return "\u2716";
      case "warning": return "\u26A0";
      default: return "\u2139";
    }
  }

  function levelColor(level: string): string {
    switch (level) {
      case "success": return "var(--green)";
      case "error": return "var(--red)";
      case "warning": return "var(--amber, #f59e0b)";
      default: return "var(--accent)";
    }
  }

  function formatTime(iso: string): string {
    const d = new Date(iso);
    return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }
</script>

<!-- Phase 6 will implement full notification log panel -->
{#if $showNotificationLog}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="notif-overlay" onclick={close}></div>
  <div class="notif-panel">
    <div class="notif-header">
      <h3 class="notif-title">Notifications</h3>
      <div class="notif-header-actions">
        {#if notifications.length > 0}
          <button class="btn btn-ghost btn-sm" onclick={handleClear}>Clear All</button>
        {/if}
        <button class="notif-close" onclick={close}>
          <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
            <line x1="3" y1="3" x2="11" y2="11" /><line x1="11" y1="3" x2="3" y2="11" />
          </svg>
        </button>
      </div>
    </div>
    <div class="notif-body">
      {#if loading}
        <div class="notif-empty">Loading...</div>
      {:else if notifications.length === 0}
        <div class="notif-empty">No notifications yet</div>
      {:else}
        {#each notifications as notif (notif.id)}
          <div class="notif-entry">
            <span class="notif-icon" style="color: {levelColor(notif.level)}">{levelIcon(notif.level)}</span>
            <div class="notif-content">
              <span class="notif-message">{notif.message}</span>
              {#if notif.detail}
                <span class="notif-detail">{notif.detail}</span>
              {/if}
            </div>
            <span class="notif-time">{formatTime(notif.created_at)}</span>
          </div>
        {/each}
      {/if}
    </div>
  </div>
{/if}

<style>
  .notif-overlay {
    position: fixed;
    inset: 0;
    z-index: 149;
  }

  .notif-panel {
    position: fixed;
    top: 0;
    right: 0;
    width: 360px;
    height: 100vh;
    background: color-mix(in srgb, var(--bg-primary) 75%, transparent);
    backdrop-filter: blur(32px) saturate(1.4);
    -webkit-backdrop-filter: blur(32px) saturate(1.4);
    border-left: 1px solid rgba(255, 255, 255, 0.08);
    box-shadow: var(--glass-refraction),
                -4px 0 24px rgba(0, 0, 0, 0.3);
    z-index: 150;
    display: flex;
    flex-direction: column;
    animation: notifSlideIn 0.15s var(--ease-out);
  }

  @keyframes notifSlideIn {
    from { transform: translateX(100%); }
    to { transform: translateX(0); }
  }

  .notif-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-4);
    border-bottom: 1px solid var(--separator);
  }

  .notif-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .notif-header-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .notif-close {
    padding: 4px;
    border-radius: var(--radius-sm);
    color: var(--text-tertiary);
    cursor: pointer;
  }

  .notif-close:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .notif-body {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-2);
  }

  .notif-empty {
    padding: var(--space-8);
    text-align: center;
    font-size: 13px;
    color: var(--text-tertiary);
  }

  .notif-entry {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    transition: background var(--duration-fast) var(--ease);
  }

  .notif-entry:hover {
    background: var(--surface-hover);
  }

  .notif-icon {
    font-size: 14px;
    flex-shrink: 0;
    margin-top: 1px;
  }

  .notif-content {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .notif-message {
    font-size: 13px;
    color: var(--text-primary);
    line-height: 1.4;
  }

  .notif-detail {
    font-size: 12px;
    color: var(--text-tertiary);
    line-height: 1.4;
  }

  .notif-time {
    font-size: 11px;
    color: var(--text-quaternary);
    flex-shrink: 0;
    white-space: nowrap;
  }
</style>
