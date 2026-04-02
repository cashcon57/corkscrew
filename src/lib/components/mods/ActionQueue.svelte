<script lang="ts">
  import type { DeploymentHealth, ModUpdateInfo, FileConflict } from "$lib/types";

  interface Props {
    deployHealth: DeploymentHealth | null;
    conflicts: FileConflict[];
    modUpdates: ModUpdateInfo[];
    deploying: boolean;
    onDeploy: () => void;
    onOpenConflicts: () => void;
    onOpenUpdates: () => void;
    onSort: () => void;
  }

  let {
    deployHealth,
    conflicts,
    modUpdates,
    deploying,
    onDeploy,
    onOpenConflicts,
    onOpenUpdates,
    onSort,
  }: Props = $props();

  // Track dismissed actions per session
  let dismissed = $state(new Set<string>());

  function dismiss(key: string) {
    dismissed = new Set([...dismissed, key]);
  }

  // Derive which actions are active
  let needsDeploy = $derived(
    deployHealth?.needs_redeploy === true ||
    (deployHealth?.is_deployed === false && (deployHealth?.total_mods ?? 0) > 0)
  );
  let hasConflicts = $derived(
    conflicts.filter(c => !c.same_collection).length > 0
  );
  let hasUpdates = $derived(modUpdates.length > 0);

  let actions = $derived((() => {
    const items: { key: string; icon: string; label: string; detail: string; btnLabel: string; btnAction: () => void; severity: "info" | "warn" | "accent" }[] = [];

    if (needsDeploy && !dismissed.has("deploy")) {
      items.push({
        key: "deploy",
        icon: "⟳",
        label: "Deploy needed",
        detail: "Mods have changed since last deployment",
        btnLabel: deploying ? "Deploying..." : "Deploy Now",
        btnAction: onDeploy,
        severity: "accent",
      });
    }

    if (hasConflicts && !dismissed.has("conflicts")) {
      const count = conflicts.filter(c => !c.same_collection).length;
      items.push({
        key: "conflicts",
        icon: "⚠",
        label: `${count} file conflict${count !== 1 ? "s" : ""} detected`,
        detail: "Some mods overwrite the same files — review to ensure correct priority",
        btnLabel: "Review Conflicts",
        btnAction: onOpenConflicts,
        severity: "warn",
      });
    }

    if (hasUpdates && !dismissed.has("updates")) {
      items.push({
        key: "updates",
        icon: "↑",
        label: `${modUpdates.length} mod update${modUpdates.length !== 1 ? "s" : ""} available`,
        detail: "Newer versions found on NexusMods",
        btnLabel: "View Updates",
        btnAction: onOpenUpdates,
        severity: "info",
      });
    }

    return items;
  })());
</script>

{#if actions.length > 0}
  <div class="action-queue">
    {#each actions as action (action.key)}
      <div class="action-card action-{action.severity}">
        <span class="action-icon">{action.icon}</span>
        <div class="action-text">
          <span class="action-label">{action.label}</span>
          <span class="action-detail">{action.detail}</span>
        </div>
        <button
          class="btn btn-sm action-btn action-btn-{action.severity}"
          onclick={action.btnAction}
          disabled={action.key === "deploy" && deploying}
        >
          {action.btnLabel}
        </button>
        <button class="action-dismiss" onclick={() => dismiss(action.key)} title="Dismiss">
          ✕
        </button>
      </div>
    {/each}
  </div>
{/if}

<style>
  .action-queue {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 0 var(--space-4) var(--space-2);
  }

  .action-card {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 8px 12px;
    border-radius: 8px;
    font-size: 13px;
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
    border: 1px solid transparent;
  }

  .action-accent {
    background: color-mix(in srgb, var(--accent) 8%, transparent);
    border-color: color-mix(in srgb, var(--accent) 20%, transparent);
  }
  .action-warn {
    background: color-mix(in srgb, var(--amber) 8%, transparent);
    border-color: color-mix(in srgb, var(--amber) 20%, transparent);
  }
  .action-info {
    background: color-mix(in srgb, var(--blue) 8%, transparent);
    border-color: color-mix(in srgb, var(--blue) 20%, transparent);
  }

  .action-icon {
    font-size: 16px;
    flex-shrink: 0;
    width: 20px;
    text-align: center;
  }

  .action-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .action-label {
    font-weight: 500;
    color: var(--text-primary);
  }

  .action-detail {
    font-size: 11px;
    color: var(--text-secondary);
  }

  .action-btn {
    flex-shrink: 0;
    padding: 4px 12px;
    border-radius: 6px;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    border: none;
    transition: all var(--duration-fast) var(--ease);
  }

  .action-btn-accent {
    background: var(--accent);
    color: white;
  }
  .action-btn-accent:hover { opacity: 0.85; }

  .action-btn-warn {
    background: var(--amber);
    color: var(--surface);
  }
  .action-btn-warn:hover { opacity: 0.85; }

  .action-btn-info {
    background: var(--blue);
    color: white;
  }
  .action-btn-info:hover { opacity: 0.85; }

  .action-dismiss {
    flex-shrink: 0;
    background: none;
    border: none;
    color: var(--text-tertiary);
    cursor: pointer;
    padding: 2px 4px;
    font-size: 12px;
    border-radius: 4px;
    transition: color var(--duration-fast) var(--ease);
  }
  .action-dismiss:hover {
    color: var(--text-primary);
    background: var(--surface-hover);
  }
</style>
