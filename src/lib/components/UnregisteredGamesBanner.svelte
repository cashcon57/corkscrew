<script lang="ts">
  // Banner that surfaces CrossOver `.lnk` shortcuts for games we detected
  // but the user hasn't registered yet (typical case: GOG / itch.io / DRM-
  // free games dropped into `drive_c/Games/`). Steam appmanifest scanner
  // can't see those.
  //
  // - "Register all matched": one-click registers every shortcut whose
  //   `match_hint` filled in by the backend (plugin or vortex_index match).
  // - "Review individually": opens a modal with editable per-shortcut fields
  //   so the user can tweak game_id / display_name / nexus_slug before
  //   committing.
  // - "Dismiss for now": session-scoped (sessionStorage). Banner reappears
  //   on next launch — there's no per-shortcut "never show again" because
  //   the right outcome is to register or fix the shortcut.

  import { onMount } from "svelte";
  import {
    listUnregisteredCrossoverGames,
    registerUnregisteredGame,
    getAllGames,
  } from "$lib/api";
  import type { UnregisteredGame } from "$lib/types";
  import { games, showError } from "$lib/stores";

  const DISMISS_KEY = "unregistered_banner_dismissed_session";

  let unregistered = $state<UnregisteredGame[]>([]);
  let loading = $state(false);
  let dismissed = $state(false);
  let reviewing = $state<UnregisteredGame | null>(null);

  // Editable form state when "Review individually" is open. Mirrors the
  // backend `register_unregistered_game` command's parameters.
  let editForm = $state<{
    gameId: string;
    displayName: string;
    nexusSlug: string;
    steamAppId: string;
  }>({ gameId: "", displayName: "", nexusSlug: "", steamAppId: "" });

  let registering = $state(false);

  function isDismissed(): boolean {
    if (typeof window === "undefined") return false;
    try {
      return window.sessionStorage.getItem(DISMISS_KEY) === "true";
    } catch {
      return false;
    }
  }

  function persistDismissed() {
    if (typeof window === "undefined") return;
    try {
      window.sessionStorage.setItem(DISMISS_KEY, "true");
    } catch (err) {
      console.error("UnregisteredGamesBanner: persist dismissed failed:", err);
    }
  }

  async function refresh() {
    loading = true;
    try {
      unregistered = (await listUnregisteredCrossoverGames()) ?? [];
    } catch (err: unknown) {
      console.error(
        "UnregisteredGamesBanner: list_unregistered_crossover_games failed:",
        err,
      );
      unregistered = [];
    } finally {
      loading = false;
    }
  }

  async function reloadGames() {
    try {
      const g = await getAllGames();
      games.set(g);
    } catch (err: unknown) {
      console.error("UnregisteredGamesBanner: getAllGames failed:", err);
    }
  }

  async function registerOne(entry: UnregisteredGame): Promise<boolean> {
    const hint = entry.match_hint;
    if (!hint) return false;

    // Derive the game's install dir from the working dir if available, else
    // the parent of the exe.
    const gamePath =
      entry.shortcut.working_directory ?? deriveDirOf(entry.shortcut.host_target);

    try {
      await registerUnregisteredGame({
        bottleName: entry.shortcut.bottle_name,
        gameId: hint.game_id,
        displayName: hint.display_name,
        nexusSlug: hint.nexus_slug,
        steamAppId: hint.steam_app_id,
        gamePath,
        exePath: entry.shortcut.host_target,
      });
      return true;
    } catch (err: unknown) {
      console.error(
        `UnregisteredGamesBanner: register failed for ${hint.game_id}:`,
        err,
      );
      showError(`Failed to register ${hint.display_name}: ${err}`);
      return false;
    }
  }

  function deriveDirOf(p: string): string {
    const idx = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\"));
    return idx > 0 ? p.slice(0, idx) : p;
  }

  async function registerAllMatched() {
    registering = true;
    try {
      const matched = unregistered.filter((u) => u.match_hint !== null);
      let okCount = 0;
      for (const entry of matched) {
        if (await registerOne(entry)) okCount += 1;
      }
      if (okCount > 0) {
        await reloadGames();
        await refresh();
      }
    } finally {
      registering = false;
    }
  }

  function startReview(entry: UnregisteredGame) {
    reviewing = entry;
    const hint = entry.match_hint;
    editForm = {
      gameId: hint?.game_id ?? slugify(entry.shortcut.display_name),
      displayName: hint?.display_name ?? entry.shortcut.display_name,
      nexusSlug: hint?.nexus_slug ?? slugify(entry.shortcut.display_name),
      steamAppId: hint?.steam_app_id ?? "",
    };
  }

  function cancelReview() {
    reviewing = null;
  }

  function slugify(s: string): string {
    return s
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "")
      .slice(0, 32);
  }

  async function submitReview() {
    if (!reviewing) return;
    const entry = reviewing;
    if (!editForm.gameId.trim() || !editForm.displayName.trim()) {
      showError("Game ID and display name are required.");
      return;
    }
    registering = true;
    try {
      const gamePath =
        entry.shortcut.working_directory ??
        deriveDirOf(entry.shortcut.host_target);
      await registerUnregisteredGame({
        bottleName: entry.shortcut.bottle_name,
        gameId: editForm.gameId.trim(),
        displayName: editForm.displayName.trim(),
        nexusSlug: editForm.nexusSlug.trim() || editForm.gameId.trim(),
        steamAppId: editForm.steamAppId.trim() || null,
        gamePath,
        exePath: entry.shortcut.host_target,
      });
      reviewing = null;
      await reloadGames();
      await refresh();
    } catch (err: unknown) {
      console.error("UnregisteredGamesBanner: review submit failed:", err);
      showError(`Failed to register: ${err}`);
    } finally {
      registering = false;
    }
  }

  function dismiss() {
    persistDismissed();
    dismissed = true;
  }

  const matchedCount = $derived(
    unregistered.filter((u) => u.match_hint !== null).length,
  );
  const visible = $derived(
    !dismissed && !loading && unregistered.length > 0,
  );

  onMount(() => {
    dismissed = isDismissed();
    refresh().catch((err) =>
      console.error("UnregisteredGamesBanner: initial refresh failed:", err),
    );
  });
</script>

{#if visible}
  <div class="banner" role="region" aria-label="Unregistered games">
    <div class="banner-head">
      <div class="banner-title-block">
        <svg
          width="18"
          height="18"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="1.6"
          stroke-linecap="round"
          stroke-linejoin="round"
          class="banner-icon"
        >
          <circle cx="12" cy="12" r="10" />
          <line x1="12" y1="8" x2="12" y2="12" />
          <circle cx="12" cy="16" r="0.6" fill="currentColor" />
        </svg>
        <div class="banner-text">
          <p class="banner-title">
            Found {unregistered.length}
            {unregistered.length === 1 ? "game" : "games"} in your CrossOver
            bottles that {unregistered.length === 1 ? "isn't" : "aren't"} registered
            yet
          </p>
          <p class="banner-sub">
            {#if matchedCount > 0}
              {matchedCount} auto-matched. Register them in one click, or review
              individually.
            {:else}
              Review each one to set its game ID and Nexus slug.
            {/if}
          </p>
        </div>
      </div>
      <div class="banner-actions">
        {#if matchedCount > 0}
          <button
            class="btn btn-primary"
            onclick={registerAllMatched}
            disabled={registering}
          >
            {registering ? "Registering..." : `Register all matched (${matchedCount})`}
          </button>
        {/if}
        <button class="btn btn-secondary" onclick={dismiss} disabled={registering}>
          Dismiss for now
        </button>
      </div>
    </div>

    <ul class="shortcut-list">
      {#each unregistered as entry (entry.shortcut.host_target)}
        <li class="shortcut-row">
          <div class="shortcut-info">
            <div class="shortcut-name-row">
              <span class="shortcut-name">{entry.shortcut.display_name}</span>
              <span class="shortcut-bottle">{entry.shortcut.bottle_name}</span>
              {#if entry.match_hint}
                <span class="match-badge" title="Auto-matched">
                  {entry.match_hint.display_name}
                  <span class="match-source"
                    >· {entry.match_hint.source === "Plugin"
                      ? "plugin"
                      : "vortex"}</span
                  >
                </span>
              {:else}
                <span class="match-badge unknown">No match</span>
              {/if}
            </div>
            <p class="shortcut-path" title={entry.shortcut.host_target}>
              {entry.shortcut.host_target}
            </p>
          </div>
          <div class="shortcut-actions">
            <button class="btn btn-secondary btn-sm" onclick={() => startReview(entry)}>
              Review
            </button>
          </div>
        </li>
      {/each}
    </ul>
  </div>
{/if}

{#if reviewing}
  <div
    class="modal-backdrop"
    role="presentation"
    onclick={cancelReview}
    onkeydown={(e) => {
      if (e.key === "Escape") cancelReview();
    }}
  >
    <div
      class="modal"
      role="dialog"
      aria-modal="true"
      aria-label="Register game"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => e.stopPropagation()}
    >
      <header class="modal-head">
        <h3 class="modal-title">Register {reviewing.shortcut.display_name}</h3>
        <p class="modal-sub">
          Bottle: <span class="modal-bottle">{reviewing.shortcut.bottle_name}</span>
        </p>
      </header>

      <form class="modal-body" onsubmit={(e) => { e.preventDefault(); submitReview(); }}>
        <label class="field">
          <span class="field-label">Game ID</span>
          <input
            class="field-input"
            type="text"
            bind:value={editForm.gameId}
            placeholder="eldenring"
            autocomplete="off"
          />
          <span class="field-hint">Lowercase, no spaces. Used internally.</span>
        </label>

        <label class="field">
          <span class="field-label">Display name</span>
          <input
            class="field-input"
            type="text"
            bind:value={editForm.displayName}
            placeholder="Elden Ring"
            autocomplete="off"
          />
        </label>

        <label class="field">
          <span class="field-label">Nexus slug</span>
          <input
            class="field-input"
            type="text"
            bind:value={editForm.nexusSlug}
            placeholder="eldenring"
            autocomplete="off"
          />
          <span class="field-hint">Matches the URL on nexusmods.com.</span>
        </label>

        <label class="field">
          <span class="field-label">Steam App ID (optional)</span>
          <input
            class="field-input"
            type="text"
            bind:value={editForm.steamAppId}
            placeholder="1245620"
            autocomplete="off"
          />
        </label>

        <div class="field readonly">
          <span class="field-label">Executable</span>
          <code class="field-readonly">{reviewing.shortcut.host_target}</code>
        </div>

        <footer class="modal-foot">
          <button
            type="button"
            class="btn btn-secondary"
            onclick={cancelReview}
            disabled={registering}
          >
            Cancel
          </button>
          <button type="submit" class="btn btn-primary" disabled={registering}>
            {registering ? "Registering..." : "Register"}
          </button>
        </footer>
      </form>
    </div>
  </div>
{/if}

<style>
  .banner {
    background: var(--surface-glass);
    border: 1px solid var(--accent-muted);
    border-radius: var(--radius-md, 10px);
    padding: var(--space-4, 16px);
    margin-bottom: var(--space-4, 16px);
    backdrop-filter: blur(20px);
    -webkit-backdrop-filter: blur(20px);
    animation: fade-in 240ms ease-out;
  }

  @keyframes fade-in {
    from {
      opacity: 0;
      transform: translateY(-4px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  .banner-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-4, 16px);
    flex-wrap: wrap;
  }

  .banner-title-block {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3, 12px);
    flex: 1;
    min-width: 240px;
  }

  .banner-icon {
    color: var(--accent, #e8802a);
    flex-shrink: 0;
    margin-top: 2px;
  }

  .banner-title {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .banner-sub {
    margin: 4px 0 0;
    font-size: 12px;
    color: var(--text-secondary);
  }

  .banner-actions {
    display: flex;
    gap: var(--space-2, 8px);
    align-items: center;
  }

  .btn {
    border: 1px solid transparent;
    border-radius: var(--radius-sm, 6px);
    padding: 6px 12px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: all 120ms ease;
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .btn-primary {
    background: var(--accent, #e8802a);
    color: var(--accent-on, #fff);
  }
  .btn-primary:hover:not(:disabled) {
    background: var(--accent-hover, #f09040);
  }
  .btn-secondary {
    background: var(--surface-glass);
    color: var(--text-primary);
    border-color: var(--text-quaternary);
  }
  .btn-secondary:hover:not(:disabled) {
    background: var(--surface-glass-hover);
  }
  .btn-sm {
    padding: 4px 10px;
    font-size: 12px;
  }

  .shortcut-list {
    list-style: none;
    padding: 0;
    margin: var(--space-3, 12px) 0 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .shortcut-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3, 12px);
    padding: 8px 12px;
    background: var(--surface-subtle);
    border-radius: var(--radius-sm, 6px);
  }

  .shortcut-info {
    flex: 1;
    min-width: 0;
  }

  .shortcut-name-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }

  .shortcut-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .shortcut-bottle {
    font-size: 11px;
    color: var(--text-tertiary);
    padding: 1px 6px;
    border-radius: 3px;
    background: var(--surface-glass);
  }

  .match-badge {
    font-size: 11px;
    color: var(--accent, #e8802a);
    background: var(--accent-subtle);
    padding: 1px 6px;
    border-radius: 3px;
  }
  .match-badge.unknown {
    color: var(--text-tertiary);
    background: var(--surface-glass);
  }
  .match-source {
    color: var(--text-tertiary);
    font-weight: 400;
  }

  .shortcut-path {
    margin: 2px 0 0;
    font-size: 11px;
    color: var(--text-tertiary);
    font-family: ui-monospace, "SF Mono", Menlo, monospace;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .shortcut-actions {
    flex-shrink: 0;
  }

  /* Modal */
  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    backdrop-filter: blur(4px);
    -webkit-backdrop-filter: blur(4px);
    z-index: 1000;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--space-4, 16px);
  }

  .modal {
    background: var(--surface-glass);
    border: 1px solid var(--text-quaternary);
    border-radius: var(--radius-md, 10px);
    padding: var(--space-5, 20px);
    width: 100%;
    max-width: 480px;
    backdrop-filter: blur(40px);
    -webkit-backdrop-filter: blur(40px);
  }

  .modal-head {
    margin-bottom: var(--space-4, 16px);
  }

  .modal-title {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .modal-sub {
    margin: 4px 0 0;
    font-size: 12px;
    color: var(--text-secondary);
  }

  .modal-bottle {
    color: var(--accent, #e8802a);
    font-weight: 500;
  }

  .modal-body {
    display: flex;
    flex-direction: column;
    gap: var(--space-3, 12px);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .field.readonly {
    gap: 4px;
  }

  .field-label {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
  }

  .field-input {
    padding: 6px 10px;
    background: var(--surface-subtle);
    border: 1px solid var(--text-quaternary);
    border-radius: var(--radius-sm, 6px);
    color: var(--text-primary);
    font-size: 13px;
    font-family: inherit;
  }
  .field-input:focus {
    outline: none;
    border-color: var(--accent, #e8802a);
  }

  .field-hint {
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .field-readonly {
    display: block;
    padding: 6px 10px;
    background: var(--surface-subtle);
    border-radius: var(--radius-sm, 6px);
    font-family: ui-monospace, "SF Mono", Menlo, monospace;
    font-size: 11px;
    color: var(--text-tertiary);
    overflow-wrap: anywhere;
  }

  .modal-foot {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2, 8px);
    margin-top: var(--space-3, 12px);
  }
</style>
