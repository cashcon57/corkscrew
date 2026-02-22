<script lang="ts">
  import { selectedGame, activeProfile, profileList, showError, showSuccess } from "$lib/stores";
  import { activateProfile, deleteProfile, createProfile, saveProfileSnapshot, listProfiles } from "$lib/api";
  import { get } from "svelte/store";
  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";

  interface Props {
    isOpen: boolean;
    onToggle: () => void;
    onClose: () => void;
  }

  let {
    isOpen,
    onToggle,
    onClose,
  }: Props = $props();

  let switching = $state<number | null>(null);
  let confirmDeleteProfile = $state<{ id: number; name: string } | null>(null);
  let deleting = $state(false);
  let creatingNew = $state(false);
  let newProfileName = $state("");
  let saving = $state(false);

  async function reloadProfiles() {
    const game = get(selectedGame);
    if (!game) return;
    const profiles = await listProfiles(game.game_id, game.bottle_name);
    profileList.set(profiles);
    const active = profiles.find(p => p.is_active) ?? null;
    activeProfile.set(active);
  }

  async function handleSwitch(profileId: number, profileName: string) {
    const game = get(selectedGame);
    if (!game || switching) return;
    switching = profileId;
    try {
      await activateProfile(profileId, game.game_id, game.bottle_name);
      await reloadProfiles();
      showSuccess(`Switched to profile "${profileName}"`);
      onClose();
    } catch (e: unknown) {
      showError(`Failed to switch profile: ${e}`);
    } finally {
      switching = null;
    }
  }

  async function handleConfirmDelete() {
    const target = confirmDeleteProfile;
    if (!target) return;
    deleting = true;
    try {
      await deleteProfile(target.id);
      await reloadProfiles();
      showSuccess(`Deleted profile "${target.name}"`);
    } catch (e: unknown) {
      showError(`Failed to delete profile: ${e}`);
    } finally {
      deleting = false;
      confirmDeleteProfile = null;
    }
  }

  async function handleCreateProfile() {
    const name = newProfileName.trim();
    if (!name) return;
    const game = get(selectedGame);
    if (!game) return;
    try {
      await createProfile(game.game_id, game.bottle_name, name);
      await reloadProfiles();
      showSuccess(`Created profile "${name}"`);
      newProfileName = "";
      creatingNew = false;
    } catch (e: unknown) {
      showError(`Failed to create profile: ${e}`);
    }
  }

  async function handleSaveCurrent() {
    const profile = get(activeProfile);
    const game = get(selectedGame);
    if (!profile || !game) return;
    saving = true;
    try {
      await saveProfileSnapshot(profile.id, game.game_id, game.bottle_name);
      showSuccess(`Saved current state to "${profile.name}"`);
    } catch (e: unknown) {
      showError(`Failed to save profile: ${e}`);
    } finally {
      saving = false;
    }
  }
</script>

<div class="topbar-selector-wrap">
  <button
    class="topbar-selector"
    onclick={(e) => { e.stopPropagation(); onToggle(); }}
    title={$activeProfile?.name ?? "No Profile"}
  >
    <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" opacity="0.5">
      <rect x="2" y="2" width="12" height="4" rx="1" />
      <rect x="2" y="10" width="12" height="4" rx="1" />
    </svg>
    <span class="topbar-selector-label" class:placeholder={!$activeProfile}>
      {$activeProfile?.name ?? "No Profile"}
    </span>
    <svg class="topbar-chevron" class:open={isOpen} width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
      <path d="M3 4l2 2 2-2" />
    </svg>
  </button>

  {#if isOpen}
    <div class="topbar-dropdown" onclick={(e) => e.stopPropagation()}>
      {#if $profileList.length > 0}
        {#each $profileList as profile}
          <div class="dropdown-item-row">
            <button
              class="dropdown-item"
              class:active={profile.is_active}
              disabled={switching === profile.id}
              onclick={() => handleSwitch(profile.id, profile.name)}
            >
              <span class="dropdown-item-name">
                {#if switching === profile.id}
                  <span class="spinner spinner-xs"></span>
                {/if}
                {profile.name}
              </span>
              {#if profile.is_active}
                <svg class="active-check" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                  <polyline points="20 6 9 17 4 12" />
                </svg>
              {/if}
            </button>
            <button
              class="dropdown-delete-btn"
              title="Delete profile"
              onclick={(e) => { e.stopPropagation(); confirmDeleteProfile = { id: profile.id, name: profile.name }; }}
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="3 6 5 6 21 6" /><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
              </svg>
            </button>
          </div>
        {/each}
      {:else}
        <div class="dropdown-empty">No profiles</div>
      {/if}

      <div class="dropdown-footer">
        {#if creatingNew}
          <form class="new-profile-form" onsubmit={(e) => { e.preventDefault(); handleCreateProfile(); }}>
            <input
              class="new-profile-input"
              type="text"
              placeholder="Profile name..."
              bind:value={newProfileName}
              autofocus
              onkeydown={(e) => { if (e.key === "Escape") { creatingNew = false; newProfileName = ""; } }}
            />
            <button class="new-profile-submit" type="submit" disabled={!newProfileName.trim()}>
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="20 6 9 17 4 12" />
              </svg>
            </button>
          </form>
        {:else}
          <button class="dropdown-action" onclick={() => { creatingNew = true; }}>
            <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
              <line x1="8" y1="3" x2="8" y2="13" /><line x1="3" y1="8" x2="13" y2="8" />
            </svg>
            New Profile
          </button>
        {/if}

        {#if $activeProfile}
          <button class="dropdown-action" onclick={handleSaveCurrent} disabled={saving}>
            <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
              <path d="M12.5 2.5H3.5a1 1 0 00-1 1v9a1 1 0 001 1h9a1 1 0 001-1v-7l-2-2z" />
              <path d="M10.5 12.5v-3h-5v3" /><path d="M5.5 2.5v3h4" />
            </svg>
            {saving ? "Saving..." : "Save Current"}
          </button>
        {/if}
      </div>
    </div>
  {/if}
</div>

<ConfirmDialog
  open={confirmDeleteProfile !== null}
  title="Delete Profile"
  message={`Are you sure you want to delete profile "${confirmDeleteProfile?.name}"?`}
  confirmLabel={deleting ? "Deleting..." : "Delete"}
  confirmDanger={true}
  onConfirm={handleConfirmDelete}
  onCancel={() => confirmDeleteProfile = null}
/>

<style>
  .topbar-selector-wrap {
    position: relative;
    display: flex;
    align-items: center;
    -webkit-app-region: no-drag;
  }

  .topbar-selector {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 10px;
    border-radius: var(--radius);
    font-size: 13px;
    color: var(--text-primary);
    background: none;
    border: none;
    cursor: pointer;
    -webkit-app-region: no-drag;
    transition: background 0.15s ease;
    max-width: 200px;
  }

  .topbar-selector:hover {
    background: var(--surface-hover);
  }

  .topbar-selector-label {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-weight: 500;
  }

  .topbar-selector-label.placeholder {
    color: var(--text-tertiary);
    font-weight: 400;
  }

  .topbar-chevron {
    flex-shrink: 0;
    opacity: 0.4;
    transition: transform 0.15s ease;
  }

  .topbar-chevron.open {
    transform: rotate(180deg);
  }

  .topbar-dropdown {
    position: absolute;
    top: calc(100% + 6px);
    left: 0;
    min-width: 220px;
    max-width: 300px;
    background: var(--bg-elevated);
    border: 1px solid var(--separator);
    border-radius: var(--radius);
    padding: 4px;
    z-index: 100;
    box-shadow:
      0 4px 24px rgba(0, 0, 0, 0.3),
      0 1px 4px rgba(0, 0, 0, 0.15),
      inset 0 1px 0 0 var(--surface);
    animation: dropdownIn 0.15s ease-out;
    max-height: 320px;
    overflow-y: auto;
  }

  .dropdown-item-row {
    display: flex;
    align-items: center;
  }

  .dropdown-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex: 1;
    padding: 7px 10px;
    border-radius: calc(var(--radius) - 2px);
    background: none;
    border: none;
    color: var(--text-primary);
    cursor: pointer;
    font-size: 13px;
    text-align: left;
    transition: background 0.1s ease;
    min-width: 0;
  }

  .dropdown-item:hover {
    background: var(--surface-hover);
  }

  .dropdown-item.active {
    background: var(--accent-subtle);
    color: var(--accent);
  }

  .dropdown-item:disabled {
    opacity: 0.6;
    cursor: default;
  }

  .dropdown-item-name {
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .active-check {
    flex-shrink: 0;
    color: var(--accent);
  }

  .dropdown-delete-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    border-radius: var(--radius);
    background: none;
    border: none;
    color: var(--text-quaternary);
    cursor: pointer;
    opacity: 0;
    transition: opacity 0.1s ease, background 0.1s ease, color 0.1s ease;
    flex-shrink: 0;
  }

  .dropdown-item-row:hover .dropdown-delete-btn {
    opacity: 1;
  }

  .dropdown-delete-btn:hover {
    background: rgba(255, 59, 48, 0.15);
    color: var(--red);
  }

  .dropdown-empty {
    padding: 12px 10px;
    font-size: 12px;
    color: var(--text-tertiary);
    text-align: center;
  }

  .dropdown-footer {
    border-top: 1px solid var(--separator);
    margin-top: 4px;
    padding-top: 4px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .dropdown-action {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 7px 10px;
    border-radius: calc(var(--radius) - 2px);
    background: none;
    border: none;
    color: var(--text-secondary);
    cursor: pointer;
    font-size: 12px;
    text-align: left;
    transition: background 0.1s ease, color 0.1s ease;
  }

  .dropdown-action:hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }

  .dropdown-action:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .new-profile-form {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 6px;
  }

  .new-profile-input {
    flex: 1;
    background: var(--surface);
    border: 1px solid var(--separator-opaque);
    border-radius: calc(var(--radius) - 2px);
    padding: 5px 8px;
    font-size: 12px;
    color: var(--text-primary);
    outline: none;
    min-width: 0;
  }

  .new-profile-input:focus {
    border-color: var(--accent);
  }

  .new-profile-submit {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    border-radius: calc(var(--radius) - 2px);
    background: var(--accent-subtle);
    border: none;
    color: var(--accent);
    cursor: pointer;
    transition: background 0.1s ease;
    flex-shrink: 0;
  }

  .new-profile-submit:hover {
    background: var(--accent);
    color: white;
  }

  .new-profile-submit:disabled {
    opacity: 0.3;
    cursor: default;
  }

  @keyframes dropdownIn {
    from { opacity: 0; transform: translateY(-4px); }
    to { opacity: 1; transform: translateY(0); }
  }
</style>
