# Changelog

All notable changes to Corkscrew are documented here. This project follows [Semantic Versioning](https://semver.org/).

## [0.14.8] - 2026-05-11

### Fixed

- **Custom games failed to launch with `Game '<id>' not found in bottle '<name>'`**: every command that resolved a game/bottle pair (`launch_game_cmd`, install, deploy, mod ops, redeploy, tool actions, …) called `resolve_game`, which only iterated registered plugins' `detect_wine`. Custom games saved via the Wine Dashboard "Add Game" button were visible in the dashboard (`get_all_games` → `detect_all_games_with_custom`) but invisible to every action. `resolve_game` now falls back to `custom_games` rows scoped to the supplied `bottle_name` via a process-wide `GLOBAL_DB` `OnceLock` populated at startup.

### Added

- **Manage custom games from the Wine Dashboard**: game cards added via "Add Game" now show a `Custom` badge plus two new icons in the path row — a pencil to **edit** paths (folder + exe) and a trash icon to **remove** the registration (files untouched). Addresses user reports of "no way to even modify the game's path or remove and re-add it" after a botched Add Game submission or after moving a game folder.
- Backed by two new bottle-scoped helpers and Tauri commands: `remove_custom_game_for_bottle` / `remove_custom_game_cmd` and `update_custom_game_paths` / `update_custom_game_paths_cmd`. Update validates that new paths exist on disk and stay inside the bottle root (same containment check as `register_unregistered_game`). CLI `--remove-custom-game` keeps the legacy game-id-only path for power-user scripts.
- `DetectedGame.is_custom: bool` (serde default `false`) — set to `true` when the row came from `custom_games`; used by the frontend to decide whether to render the badge and the Edit/Remove buttons. All 16 in-tree `DetectedGame` initializers updated.

## [0.14.7] - 2026-05-10

### Security

- **Tar extraction path traversal**: tar archive entries now reject `..`, absolute, and Windows-prefix path components before joining onto `dest_dir`. The previous lexical `starts_with(dest_dir)` check could be bypassed by paths like `Data/../../outside` because the prefix matches before path normalization. ZIP and RAR already had component-level checks; tar now uses the same defence.

### Fixed

- **Deploy target drift in the deployment manifest** (schema migration v24): the manifest's UNIQUE constraint was `(game_id, bottle_name, relative_path)` and inserts preserved `deploy_target` via COALESCE-by-path. Two mods that legitimately shared a relative_path under different targets (`data/` vs `BepInEx/plugins/<mod>/`, vs game-root) collapsed into one row and lost their original target. The manifest is now rebuilt with `UNIQUE(game_id, bottle_name, deploy_target, relative_path)`, callers pass `deploy_target` explicitly through `deploy_mod` / `batch_add_deployment_entries[_with_hashes]`, and the on-disk DB is snapshotted to a timestamped `.backup` sibling before the rebuild runs.
- **Toggle/redeploy ignored stored deploy_target**: enabling a previously-disabled BepInEx/UE/Vortex/root mod redeployed its files to `data/` regardless of where they originally went. `toggle_mod`, batch toggle, and rollback now read the per-mod stored target and pick the matching effective directory (game root / vortex mod-type path / data dir).
- **Concurrent install temp-dir collision**: `install_mod` used `temp_dir/corkscrew_install_{PID}`, so two parallel installs in the same process collided on the same path and could delete each other's extracting files. Now uses `tempfile::Builder` for a unique per-install directory with RAII cleanup.
- **Non-ASCII ZIP filename corruption**: ZIP filenames containing UTF-8 bytes ≥ 0x80 (e.g. `café.txt`) were unconditionally re-decoded as CP437, corrupting valid UTF-8 names. The decoder now probes UTF-8 first and only falls back to CP437 when the byte sequence is not valid UTF-8 (legacy DOS archives).
- **Hardlink probe filename collision**: the hardlink-support test used a fixed `.corkscrew_hardlink_test` filename in both staging and data dirs. Concurrent probes (or stale files from an interrupted run) could interfere. Now uses a per-call unique stem with PID + nanosecond + counter suffix.
- **NXM auto-install bypassed routing**: Nexus 1-click downloads with auto-install always deployed straight to `data_dir`, ignoring the per-game routing layers used by the manual install path (Vortex mod-type detection, BepInEx plugins → `BepInEx/plugins/<modname>/`, UE paks → `~mods/`, etc.). Both paths now share the same `resolve_effective_deploy_dir` helper.
- **Collection completion hashed wrong game on game switch**: when an install finished after the user switched games, the post-install background hash ran against the currently selected game instead of the install target. Game ID and bottle name are now captured at install start.
- **Install logs grew unbounded**: `collectionInstallStatus.logEntries` had no cap; large collection installs (559+ mods) accumulated thousands of entries, slowing reactive updates and bloating memory. Now capped at 1000 entries via tail-slice.

### Changed

- `deploy_mod` / `deploy_mod_atomic` / `deploy_mod_atomic_with_progress` now take an explicit `deploy_target: &str` argument. All in-tree call sites (manual install, NXM auto-install, collection install, Wabbajack install, toggle, rollback, batch toggle, incremental deploy) updated to pass the correct value. Custom downstreams calling these functions need to add the argument.

## [0.14.6] - 2026-05-01

### Fixed

- Manually added games (via "Add Game") disappeared from the game list on rescan — `get_all_games` was calling `detect_all_games()` instead of `detect_all_games_with_custom()`, so custom games in the DB were never included

## [0.14.5] - 2026-05-01

### Added

- **Wine Dashboard: "Scan for Games" button** — re-runs the full bottle and game detection scan on demand, without restarting the app
- **Wine Dashboard: "Add Game" button** — manually register a game by selecting its folder; auto-identifies the game from known executables (game ID, display name, Nexus slug auto-filled), then saves it as a custom game associated with the chosen Wine bottle
- **Broad-scan fallback for all game plugins** — game detection now scans every subdirectory under `Documents/Games/` and `Documents/` for matching executables, catching non-standard folder names like `ELDEN.RING.v1.16.1.ALL.DLC`. Previously only Steam-standard folder names were matched. Applied to all 9 Wine game plugins (FromSoftware, Skyrim SE, Fallout 4, Hogwarts Legacy, GTA V, Crimson Desert, Genshin Impact, Sims 4, Hades 2, Thunderstore games)

## [0.14.4] - 2026-05-01

### Fixed

- `wineCtx()` crashes with "undefined is not an object" when called with a null/undefined game during reactive state initialization (Fixes RUST-G)
- `UnregisteredGamesBanner` crashes with "null is not an object" when backend returns null instead of empty array (Fixes RUST-C)

## [0.14.3] - 2026-05-01

### Fixed

- macOS: app fails to launch with "Library not loaded: /opt/homebrew/opt/xz/lib/liblzma.5.dylib"
  on machines without Homebrew's `xz` package installed. Switched `xz2` to static linking so
  liblzma is compiled into the binary instead of resolved at runtime.

## [0.14.2] - 2026-04-28

### Fixed

- macOS 26 Tahoe: "cannot be opened because of a problem" error on launch — the
  rebuilt DMG was not notarized (appdmg replaced Tauri's signed DMG without
  re-submitting to Apple). DMG is now notarized and stapled after rebuild.
  Workaround for existing v0.14.1 installs: `xattr -dr com.apple.quarantine /Applications/Corkscrew.app`

## [0.9.55] - 2026-03-31

### Added
- Hogwarts Legacy native mod merger (replaces Windows-only HLModMerger.exe entirely)
  - PAK file read/write via `repak` crate
  - SQLite database diffing and merging via `rusqlite`
  - Auto-merges PhoenixShipData.sqlite conflicts into `zMergedMods_P.pak`
- PakChunk conflict detection for Hogwarts Legacy (warns on duplicate chunk numbers)
- UE4SS auto-deploy for HL collections containing Lua/Logic mods
- Proactive error reporting infrastructure
  - Structured `error_events` SQLite table with dedup/count aggregation
  - Global error handlers (frontend `unhandledrejection` + Rust `panic::set_hook`)
  - Startup health check (DB integrity, staging dir, deploy journal)
  - Opt-in Sentry telemetry with consent banner and settings toggle
- Log file rotation (3 files at 5MB each, replaces 10MB truncation)
- Dependabot configuration for npm, Cargo, and GitHub Actions
- Auto-switch game when installing collection for a different game
- OAuth SSO as primary NexusMods sign-in method

### Fixed
- Stale "Game Is Running" banner after Hogwarts Legacy crash (added exe to game lock)
- Tool detection showing Skyrim tools (BodySlide, SKSE) for non-Skyrim games
- `each_key_duplicate` error in collection detail mod list and My Collections
- Stale mod list after collection uninstall (stores now refresh immediately)
- Collection detail timeout reduced from 30s to 15s with labeled errors
- Silent error swallows in layout (loadProfilesForGame, loadCollectionsForGame)
- Steam CDN icon fallback using wide banner instead of header image
- HL launch using correct root launcher for DRM compatibility

### Changed
- Top bar enlarged to match sidebar UI proportions
- Upgraded dependencies: rusqlite 0.39, rquickjs 0.11, vite 8, sha2 0.11, window-vibrancy 0.7, bzip2 0.6, lru 0.16, scraper 0.26, svelte 5.55, dompurify 3.3.3, marked 17.0.5
- GitHub Actions upgraded: checkout v6, upload/download-artifact v7/v8, configure-pages v6
- Privacy policy updated to disclose Sentry telemetry

## [0.9.49] - 2026-03-29

### Fixed
- Hardcoded Skyrim slug in Nexus URLs replaced with active game's nexus_slug
- WJ resume banner persisting after dismiss
- Collection detail timeout added (30s) to prevent infinite spinner

## [0.9.48] - 2026-03-29

### Fixed
- Collections page hang on load
- Auto-load collections on game switch
- Permanent dismiss with cleanup for install banners

## [0.9.47] - 2026-03-28

### Added
- Full Hogwarts Legacy support with collections, tool detection, and game-specific deployment

## [0.9.42] - 2026-03-15

### Added
- Shader conversion system (CS detection, ENB install, DXVK switch, FOMOD re-selection)
- DeployGuard RAII for all deploy operations

### Fixed
- CS disable removing essential SKSE plugins (po3_Tweaks, CrashLogger)
- Game lock zombie detection (SKSE loader → SkyrimSE.exe sibling process)
- ZIP Slip vulnerability in Wabbajack archive extraction
- 21 silent `.catch(() => {})` blocks replaced with proper error logging
- FOMOD `version_gte` wildcard handling for "1.6.x" patterns

---

For older releases, see [GitHub Releases](https://github.com/cashcon57/corkscrew/releases).
