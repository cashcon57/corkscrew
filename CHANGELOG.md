# Changelog

All notable changes to Corkscrew are documented here. This project follows [Semantic Versioning](https://semver.org/).

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
