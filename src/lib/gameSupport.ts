/**
 * Helpers for working with game support tiers in the UI.
 *
 * The authoritative tier comes from the backend `get_game_support_tier`
 * command (registries + plugins live there). This module is purely about
 * presentation and minor synchronous heuristics — never duplicate the
 * registry checks here.
 */

import { getGameSupportTier as fetchTier } from "$lib/api";
import type { DetectedGame, GameSupportTier } from "$lib/types";

/** Per-game cache so badge components don't re-invoke for every render. */
const tierCache = new Map<string, GameSupportTier>();
const inflight = new Map<string, Promise<GameSupportTier>>();

/**
 * Fetch the support tier for a game, caching the result. Safe to call
 * frequently from Svelte components — concurrent calls coalesce.
 */
export async function getTier(gameId: string): Promise<GameSupportTier> {
  const cached = tierCache.get(gameId);
  if (cached) return cached;

  const existing = inflight.get(gameId);
  if (existing) return existing;

  const promise = fetchTier(gameId)
    .then((tier) => {
      tierCache.set(gameId, tier);
      inflight.delete(gameId);
      return tier;
    })
    .catch((err) => {
      inflight.delete(gameId);
      console.error(`gameSupport.getTier(${gameId}) failed:`, err);
      // Fall back to "unknown" so the UI degrades gracefully rather than
      // crashing or showing nothing.
      return "unknown" as GameSupportTier;
    });
  inflight.set(gameId, promise);
  return promise;
}

/**
 * Synchronously read a previously-cached tier. Returns `undefined` if no
 * fetch has resolved yet — callers should treat that as "not yet known"
 * and avoid blocking UI on it.
 */
export function getCachedTier(gameId: string): GameSupportTier | undefined {
  return tierCache.get(gameId);
}

/** Reset the cache. Tests + fresh-detection flows only. */
export function _clearTierCache(): void {
  tierCache.clear();
  inflight.clear();
}

/**
 * Tiers for which the user should see a first-install confirmation dialog.
 * "experimental" and "unknown" surface modding risk; the other tiers do not.
 */
export function tierRequiresWarning(tier: GameSupportTier): boolean {
  return tier === "experimental" || tier === "unknown";
}

/** Short, capitalised label for the badge component. */
export function tierLabel(tier: GameSupportTier): string {
  switch (tier) {
    case "verified":
      return "Verified";
    case "experimental":
      return "Experimental";
    case "vortex_extension":
      return "Community";
    case "vortex_registry":
      return "Generic";
    case "unknown":
      return "Untested";
  }
}

/** Longer tooltip explaining what the tier actually means. */
export function tierTooltip(tier: GameSupportTier): string {
  switch (tier) {
    case "verified":
      return "End-to-end tested: Corkscrew detects this game, installs mods, and launches with mods active.";
    case "experimental":
      return "A dedicated plugin exists but mod install + launch has not been verified. Expect rough edges.";
    case "vortex_extension":
      return "Supported via a Vortex extension fetched at runtime. Coverage depends on the upstream extension.";
    case "vortex_registry":
      return "Listed in the bundled Vortex registry but no Corkscrew-specific plugin matched. Treat as untested.";
    case "unknown":
      return "Discovered via Steam scan or added manually. Modding behaviour for this game has not been verified.";
  }
}

/** Convenience for cases where you have a `DetectedGame` already. */
export function getTierForGame(
  game: DetectedGame
): Promise<GameSupportTier> {
  return getTier(game.game_id);
}

// ---------------------------------------------------------------------------
// First-install dialog dismissal — keyed off the AppConfig `extra` map via
// the existing `set_config_value` plumbing. The key is per-game so dismissing
// the warning for, say, Mewgenics doesn't suppress it for Crimson Desert.
// ---------------------------------------------------------------------------

export function experimentalWarningDismissedKey(gameId: string): string {
  return `experimental_warning_dismissed_${gameId}`;
}

// ---------------------------------------------------------------------------
// Anti-cheat acknowledgment — distinct from the experimental warning. Required
// for games whose modding stack injects a graphics/runtime hook into a process
// that ships anti-cheat (e.g. HoyoProtect for Genshin). Per-game, persistent
// via the same `set_config_value` plumbing.
// ---------------------------------------------------------------------------

/**
 * Game IDs that require a one-time anti-cheat acknowledgment before the user
 * can install mods. Phase 1 ships Genshin only; Star Rail / ZZZ / Honkai 3rd
 * are tracked separately and will extend this set.
 *
 * Frozen so callers cannot mutate it accidentally.
 */
export const ANTI_CHEAT_GATED_GAMES: ReadonlySet<string> = new Set(["genshin"]);

/** Convenience predicate. */
export function gameRequiresAntiCheatAck(gameId: string): boolean {
  return ANTI_CHEAT_GATED_GAMES.has(gameId);
}

/** Config key holding the acknowledgment flag for a given game. */
export function antiCheatAckKey(gameId: string): string {
  return `anti_cheat_warning_accepted_${gameId}`;
}

// ---------------------------------------------------------------------------
// FromSoftware support — Sekiro / Elden Ring / DS3 / DS:R / AC6.
// All five share the Mod Engine 2 architecture, so we treat them uniformly.
// ---------------------------------------------------------------------------

/** Game IDs handled by the FromSoft / Mod Engine 2 plugin family. */
export const FROMSOFT_GAMES: ReadonlySet<string> = new Set([
  "sekiro",
  "eldenring",
  "darksouls3",
  "darksouls_remastered",
  "armoredcore6",
]);

export function isFromSoftGame(gameId: string): boolean {
  return FROMSOFT_GAMES.has(gameId);
}

/**
 * Per-(game, bottle) config key for the Mod Engine 2 first-launch wizard
 * dismissal flag. Composed with the bottle name so dismissing the wizard
 * in one bottle doesn't suppress it in a sibling bottle that hasn't been
 * set up yet.
 */
export function me2SetupDismissedKey(gameId: string, bottleName: string): string {
  return `me2_setup_dismissed_${gameId}:${bottleName}`;
}

/**
 * Legacy game_id → Nexus slug overrides. Most slugs come from the backend
 * `DetectedGame.nexus_slug` field — this map only covers cases where the
 * backend ships the wrong value or where the call site has nothing but
 * a game_id (no full DetectedGame). Keep this map small.
 */
const NEXUS_SLUG_OVERRIDES: Record<string, string> = {
  skyrimse: "skyrimspecialedition",
  skyrim: "skyrim",
  fallout4: "fallout4",
  fallout3: "fallout3",
  falloutnv: "newvegas",
  oblivion: "oblivion",
  morrowind: "morrowind",
  starfield: "starfield",
  enderal: "enderal",
  enderalse: "enderalspecialedition",
};

/**
 * Resolve the Nexus Mods slug for a detected game.
 *
 * Priority:
 *   1. The backend-supplied `nexus_slug` field (canonical — comes from
 *      registered plugins, vortex_index lookup, or a dash-stripped fallback
 *      derived from the Steam display name).
 *   2. Hardcoded legacy overrides (`skyrimse` → `skyrimspecialedition`).
 *   3. The `game_id` with dashes stripped (Nexus's slug convention is
 *      no-dashes-no-spaces, so this is a reasonable last resort for
 *      auto-detected Steam games not covered by the index).
 */
export function gameToNexusSlug(g: { game_id: string; nexus_slug?: string }): string {
  if (g.nexus_slug && g.nexus_slug.length > 0) return g.nexus_slug;
  if (NEXUS_SLUG_OVERRIDES[g.game_id]) return NEXUS_SLUG_OVERRIDES[g.game_id];
  return g.game_id.replace(/-/g, "");
}
