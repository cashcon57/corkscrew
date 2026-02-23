<script lang="ts">
  import {
    listProfiles,
    createProfile,
    deleteProfile,
    deactivateProfile,
    renameProfile,
    saveProfileSnapshot,
    activateProfile,
  } from "$lib/api";
  import { selectedGame, showError, showSuccess } from "$lib/stores";
  import type { Profile } from "$lib/types";
  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import SkeletonRows from "$lib/components/SkeletonRows.svelte";

  let profiles = $state<Profile[]>([]);
  let loading = $state(true);
  let creating = $state(false);
  let activating = $state<number | null>(null);
  let newProfileName = $state("");
  let editingId = $state<number | null>(null);
  let editingName = $state("");
  let confirmDeleteId = $state<number | null>(null);

  const game = $derived($selectedGame);

  $effect(() => {
    if (game) loadProfiles();
  });

  async function loadProfiles() {
    if (!game) return;
    loading = true;
    try {
      profiles = await listProfiles(game.game_id, game.bottle_name);
    } catch (e: unknown) {
      showError(`Failed to load profiles: ${e}`);
    } finally {
      loading = false;
    }
  }

  async function handleCreate() {
    if (!game || !newProfileName.trim()) return;
    creating = true;
    try {
      const id = await createProfile(game.game_id, game.bottle_name, newProfileName.trim());
      // Save current state into the new profile
      await saveProfileSnapshot(id, game.game_id, game.bottle_name);
      newProfileName = "";
      showSuccess("Profile created");
      await loadProfiles();
    } catch (e: unknown) {
      showError(`Failed to create profile: ${e}`);
    } finally {
      creating = false;
    }
  }

  async function handleDelete(id: number) {
    if (!game) return;
    confirmDeleteId = id;
  }

  async function confirmDelete() {
    if (confirmDeleteId === null) return;
    const id = confirmDeleteId;
    confirmDeleteId = null;
    try {
      await deleteProfile(id);
      showSuccess("Profile deleted");
      await loadProfiles();
    } catch (e: unknown) {
      showError(`Failed to delete profile: ${e}`);
    }
  }

  async function handleActivate(id: number) {
    if (!game) return;
    activating = id;
    try {
      await activateProfile(id, game.game_id, game.bottle_name);
      showSuccess("Profile activated");
      await loadProfiles();
    } catch (e: unknown) {
      showError(`Failed to activate profile: ${e}`);
    } finally {
      activating = null;
    }
  }

  async function handleDeactivate(id: number) {
    if (!game) return;
    try {
      await deactivateProfile(game.game_id, game.bottle_name);
      showSuccess("Profile deactivated");
      await loadProfiles();
    } catch (e: unknown) {
      showError(`Failed to deactivate profile: ${e}`);
    }
  }

  async function handleSaveSnapshot(id: number) {
    if (!game) return;
    try {
      await saveProfileSnapshot(id, game.game_id, game.bottle_name);
      showSuccess("Profile state saved");
    } catch (e: unknown) {
      showError(`Failed to save profile: ${e}`);
    }
  }

  function startRename(profile: Profile) {
    editingId = profile.id;
    editingName = profile.name;
  }

  async function commitRename() {
    if (!editingId || !editingName.trim()) {
      editingId = null;
      return;
    }
    try {
      await renameProfile(editingId, editingName.trim());
      editingId = null;
      await loadProfiles();
    } catch (e: unknown) {
      showError(`Failed to rename: ${e}`);
      editingId = null;
    }
  }

  function handleRenameKey(e: KeyboardEvent) {
    if (e.key === "Enter") commitRename();
    if (e.key === "Escape") editingId = null;
  }
</script>

<div class="profiles-page">
  <header class="page-header">
    <div class="header-text">
      <h2 class="page-title">Profiles</h2>
      <p class="page-subtitle">
        {#if game}
          Save and switch between mod configurations for {game.display_name}
        {:else}
          Select a game from the Dashboard first
        {/if}
      </p>
    </div>
  </header>

  {#if !game}
    <div class="empty-state">
      <p class="empty-title">No game selected</p>
      <p class="empty-detail">Select a game from the Dashboard to manage profiles.</p>
    </div>
  {:else if loading}
    <SkeletonRows rows={4} columns={3} />
  {:else}
    <!-- Create New Profile -->
    <div class="create-section">
      <form class="create-form" onsubmit={(e) => { e.preventDefault(); handleCreate(); }}>
        <input
          type="text"
          class="profile-input"
          placeholder="New profile name..."
          bind:value={newProfileName}
          disabled={creating}
        />
        <button
          type="submit"
          class="btn btn-accent"
          disabled={creating || !newProfileName.trim()}
        >
          {#if creating}
            <div class="btn-spinner"></div>
          {:else}
            Create Profile
          {/if}
        </button>
      </form>
      <p class="create-hint">Creating a profile snapshots your current mod states and plugin load order.</p>
    </div>

    <!-- Profile List -->
    {#if profiles.length === 0}
      <div class="empty-state">
        <p class="empty-title">No profiles yet</p>
        <p class="empty-detail">Create a profile to save your current mod configuration. You can switch between profiles to quickly change your mod setup.</p>
      </div>
    {:else}
      <div class="profile-list">
        {#each profiles as profile}
          <div class="profile-card" class:active={profile.is_active}>
            <div class="profile-info">
              {#if editingId === profile.id}
                <input
                  type="text"
                  class="rename-input"
                  bind:value={editingName}
                  onblur={() => commitRename()}
                  onkeydown={(e) => handleRenameKey(e)}
                  autofocus
                />
              {:else}
                <h3 class="profile-name" ondblclick={() => startRename(profile)}>
                  {profile.name}
                  {#if profile.is_active}
                    <span class="active-badge">Active</span>
                  {/if}
                </h3>
              {/if}
              <p class="profile-date">Created {new Date(profile.created_at).toLocaleDateString()}</p>
            </div>

            <div class="profile-actions">
              {#if !profile.is_active}
                <button
                  class="btn btn-sm btn-accent"
                  onclick={() => handleActivate(profile.id)}
                  disabled={activating !== null}
                >
                  {#if activating === profile.id}
                    <div class="btn-spinner"></div>
                  {:else}
                    Activate
                  {/if}
                </button>
              {:else}
                <button
                  class="btn btn-sm btn-secondary"
                  onclick={() => handleSaveSnapshot(profile.id)}
                >
                  Save State
                </button>
                <button
                  class="btn btn-sm btn-ghost"
                  onclick={() => handleDeactivate(profile.id)}
                  title="Deactivate profile"
                >
                  Deactivate
                </button>
              {/if}

              <button
                class="btn btn-sm btn-ghost"
                onclick={() => startRename(profile)}
                title="Rename"
              >
                <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M11 2l3 3-8 8H3v-3l8-8z" />
                </svg>
              </button>

              <button
                class="btn btn-sm btn-ghost btn-danger"
                onclick={() => handleDelete(profile.id)}
                title="Delete"
              >
                <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <line x1="3" y1="4" x2="13" y2="4" />
                  <path d="M5 4V3a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v1" />
                  <path d="M4 4l.7 9.1a1.5 1.5 0 0 0 1.5 1.4h3.6a1.5 1.5 0 0 0 1.5-1.4L12 4" />
                </svg>
              </button>
            </div>
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<ConfirmDialog
  open={confirmDeleteId !== null}
  title="Delete Profile"
  message="Are you sure you want to delete this profile? This action cannot be undone."
  details={confirmDeleteId ? [profiles.find(p => p.id === confirmDeleteId)?.name ?? ""].filter(Boolean) : []}
  confirmLabel="Delete"
  confirmDanger={true}
  onConfirm={confirmDelete}
  onCancel={() => confirmDeleteId = null}
/>

<style>
  .profiles-page {
    width: 100%;
    padding: var(--space-2) 0 var(--space-12) 0;
  }

  .page-header {
    margin-bottom: var(--space-8);
    padding-bottom: var(--space-4);
    border-bottom: 1px solid var(--separator);
  }

  .page-title {
    font-size: 28px;
    font-weight: 700;
    color: var(--text-primary);
    letter-spacing: -0.025em;
  }

  .page-subtitle {
    font-size: 14px;
    color: var(--text-secondary);
    margin-top: var(--space-1);
  }

  /* Create section */
  .create-section {
    margin-bottom: var(--space-8);
  }

  .create-form {
    display: flex;
    gap: var(--space-3);
  }

  .profile-input {
    flex: 1;
    padding: var(--space-2) var(--space-3);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    color: var(--text-primary);
    font-size: 14px;
    outline: none;
    transition: border-color var(--duration-fast) var(--ease);
  }

  .profile-input:focus {
    border-color: var(--system-accent);
  }

  .create-hint {
    font-size: 12px;
    color: var(--text-tertiary);
    margin-top: var(--space-2);
  }

  /* Profile list */
  .profile-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .profile-card {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    padding: var(--space-4) var(--space-5);
    background: var(--surface);
    border: 1px solid var(--separator);
    border-radius: var(--radius-lg);
    box-shadow: var(--glass-edge-shadow);
    transition: border-color var(--duration-fast) var(--ease);
  }

  .profile-card.active {
    border-color: var(--system-accent);
    background: rgba(232, 128, 42, 0.04);
  }

  .profile-info {
    flex: 1;
    min-width: 0;
  }

  .profile-name {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
    display: flex;
    align-items: center;
    gap: var(--space-2);
    cursor: default;
  }

  .active-badge {
    font-size: 11px;
    font-weight: 600;
    color: var(--system-accent);
    background: var(--system-accent-subtle);
    padding: 1px var(--space-2);
    border-radius: var(--radius-sm);
  }

  .profile-date {
    font-size: 12px;
    color: var(--text-tertiary);
    margin-top: 2px;
  }

  .rename-input {
    padding: 2px var(--space-2);
    background: var(--surface);
    border: 1px solid var(--system-accent);
    border-radius: var(--radius-sm);
    color: var(--text-primary);
    font-size: 15px;
    font-weight: 600;
    outline: none;
    width: 200px;
  }

  .profile-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-shrink: 0;
  }

  /* Buttons */
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    border-radius: var(--radius);
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: all var(--duration-fast) var(--ease);
    white-space: nowrap;
    min-height: 32px;
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-sm {
    padding: var(--space-1) var(--space-3);
    font-size: 12px;
    min-height: 28px;
  }

  .btn-accent {
    background: var(--system-accent);
    color: #fff;
  }

  .btn-accent:hover:not(:disabled) {
    filter: brightness(1.1);
  }

  .btn-secondary {
    background: var(--surface-hover);
    color: var(--text-primary);
    border: 1px solid var(--separator);
  }

  .btn-secondary:hover:not(:disabled) {
    background: var(--surface-active);
  }

  .btn-ghost {
    background: transparent;
    color: var(--text-secondary);
    padding: var(--space-1);
    min-width: 28px;
  }

  .btn-ghost:hover:not(:disabled) {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .btn-danger:hover:not(:disabled) {
    color: var(--red);
  }

  .btn-spinner {
    width: 14px;
    height: 14px;
    border: 2px solid rgba(255, 255, 255, 0.3);
    border-top-color: #fff;
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  /* Empty & loading states */
  .empty-state {
    text-align: center;
    padding: var(--space-12) var(--space-8);
    border: 1px dashed var(--separator);
    border-radius: var(--radius-lg);
    background: var(--surface-subtle);
    box-shadow: var(--glass-edge-shadow);
  }

  .empty-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-secondary);
    margin-bottom: var(--space-2);
  }

  .empty-detail {
    font-size: 13px;
    color: var(--text-tertiary);
    max-width: 360px;
    margin: 0 auto;
    line-height: 1.55;
  }

  .loading-container {
    display: flex;
    justify-content: center;
    padding: var(--space-12);
  }

  .spinner {
    width: 28px;
    height: 28px;
  }

  .spinner-ring {
    width: 100%;
    height: 100%;
    border: 2.5px solid var(--separator);
    border-top-color: var(--system-accent);
    border-radius: 50%;
    animation: spin 0.9s cubic-bezier(0.4, 0, 0.2, 1) infinite;
  }
</style>
