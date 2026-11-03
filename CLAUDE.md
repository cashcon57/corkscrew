# Corkscrew

A mod manager for CrossOver/Wine games on macOS and Linux. Tauri v2 + Svelte 5 + Rust.
License: GPL-3.0-or-later.

## Core Philosophy

**Goal: BETTER than Windows** — more performant, more optimized, more stable outcomes.
Acceptable minimum: parity with how modlists and mod managers work on Windows.
Any modlist that works on Windows should work at least as well under Wine/CrossOver.

## Build & Test

```bash
cargo tauri dev                        # Development mode (hot-reload frontend + backend)
cargo test                             # 787+ backend tests (run from src-tauri/)
npx svelte-check --threshold error     # Frontend type checking (MUST pass before commit)
./scripts/release.sh <version>         # Hybrid release (local macOS + CI Linux)
```

ALWAYS run `cargo test` and `npx svelte-check` after making changes. Fix failures before moving on.

## Coding Standards

- Write code to a senior-engineer standard
- Don't take existing conventions as gospel, but don't assume they're wrong either
- Always version-bump fixes (user preference)
- NEVER commit signing keys, tokens, or credentials — check MEMORY.md for what's sensitive

## Secrets storage (macOS Keychain)

Sensitive credentials live in the macOS Keychain on cashconway's machine, NEVER in the repo.

| Service name | Holds | Retrieve |
|---|---|---|
| `corkscrew-discord-bot` | Discord bot token (Corkscrew-Bot, App ID 1498475927167045642) | `security find-generic-password -s "corkscrew-discord-bot" -w` |
| `corkscrew-discord-guild` | Discord guild ID (1498470295470739547) | `security find-generic-password -s "corkscrew-discord-guild" -w` |

Discord bootstrap ([scripts/discord-bootstrap.mjs](scripts/discord-bootstrap.mjs)) reads `BOT_TOKEN` and `GUILD_ID` from env:
```bash
BOT_TOKEN="$(security find-generic-password -s 'corkscrew-discord-bot' -w)" \
GUILD_ID="$(security find-generic-password -s 'corkscrew-discord-guild' -w)" \
npm run discord:bootstrap
```

See `discord-server.md` in auto-memory for server structure + token recovery.

## Critical Invariants

These rules exist because violating them caused real bugs. Follow them exactly.

### Frontend (Svelte 5)
- **After ANY mod state change**: MUST call both `loadMods()` AND `refreshHealth()`
- **NEVER use `.catch(() => {})`** — always log errors: `.catch((err) => console.error('context:', err))`
- **Svelte 5 runes ONLY**: Use `$state`, `$derived`, `$effect` — NEVER old `$:` reactive syntax
- **`@const`** only inside `{#if}`/`{#each}` blocks
- **Event listeners before commands**: When listening for Tauri events from a backend command, register the listener BEFORE invoking the command to avoid race conditions
- **Type-safe invokes**: Always use typed wrappers from `api.ts`, never raw `invoke()`

### Backend (Rust)
- **Path safety**: All paths from external sources (archives, DB, user input) MUST be validated with `is_safe_relative_path()` or canonicalization before use. Check for traversal (`..`), null bytes, drive letters.
- **`DeployGuard` RAII**: Use for all deploy operations — sets `deploy_in_progress` flag, clears on Drop
- **`auto_snapshot_before_destructive()`** before purge/delete/clean ops
- **`AppState.db` is `Arc<ModDatabase>`** — internal Mutex, do NOT `.lock()` externally
- **Symlink checks BEFORE file operations**, never after (TOCTOU prevention)

### Download/Install Pipelines (CRITICAL)
- **`collection_installer.rs` is the reference architecture** — MUST read `pipeline-architecture.md` in auto-memory before modifying `wabbajack_installer.rs` or `wabbajack_downloader.rs`
- **Semaphore-gated `tokio::spawn`** for concurrent downloads — NOT `buffer_unordered`
- **Dual semaphores**: downloads and extractions overlap via separate semaphores
- **`Arc<Notify>` signaling**: install loop processes items as they complete, does NOT wait for all downloads
- **Multi-level retry**: per-item exponential backoff → extraction re-download → global retry pass
- **Transient vs permanent errors**: MUST distinguish (don't retry 404s)
- **Cancellation**: `AtomicBool` checked before each major operation, not just between phases

### NexusMods (Compliance — CRITICAL)
- **NEVER automate downloads for free users** — enforced in `nexus.rs::get_download_links()`
- Premium-only API downloads; free users get browser links
- API headers: `Application-Name: Corkscrew`, no caching, no scraping
- OAuth: PKCE flow, redirect `http://127.0.0.1:{port}/callback` (NOT localhost)
- `@tauri-apps/plugin-opener` exports `openUrl` (NOT `open`)

### Wine/CrossOver
- File lookups MUST be case-insensitive (Wine targets NTFS/APFS)
- Path separators: normalize to `/` in all comparisons and HashMap keys
- WJ installs: `collection_name = "wj:{modlist_name}"`

### Native mode trust boundaries

Corkscrew mutates `.app` bundles in exactly one game-specific way: **Stardew Valley** — SMAPI's launcher patch renames `Contents/MacOS/StardewValley` → `StardewValley-original` and installs a new launcher script. This invalidates the bundle's Apple Developer ID signature; that is intentional and expected SMAPI behavior, not a Corkscrew choice. **Baldur's Gate 3** mods live entirely outside the bundle in `~/Documents/` — the `.app` itself is never touched, and bundle signing is preserved.

Sandboxed bundles (Mac App Store `_MASReceipt/receipt` or `/System/Applications/` path) are **refused outright** — enforced in `smapi::install`, `smapi::uninstall`, both native `deploy_native` paths, and `native_scanner::validate_manual_native_app`.

Before any destructive native operation, Corkscrew calls `rollback::create_native_snapshot(...)` (best-effort; failure logs via `log::warn` and does NOT abort). Full per-game file-mutation tables, revert procedures, and the threat model are in [`docs/native-trust-boundaries.md`](docs/native-trust-boundaries.md).

## Cross-Repo Awareness

### SSEEngineFixesForWine (retired 2026-11-03)

`/Users/cashconway/SSEEngineFixesForWine/` (cashcon57/SSEEngineFixesForWine) was a Wine fork of SSE Engine Fixes, made because stock Engine Fixes' `d3dx9_42.dll` preloader crashed under Wine. Corkscrew no longer integrates with it: nothing downloads, deploys or configures the fork, and there is no Settings option for it.

- **Why retired**: the fork only supports Skyrim 1.6.1170 and hasn't changed since March 2026. Stock SSE Engine Fixes 7.0.21+ (upstream aers/EngineFixesSkyrim64, which uses SKSE's preloader via `EngineFixes_preload.txt` on 1.7+) was verified on 2026-11-03 to load under CrossOver 26.3 Wine / NotProton on Skyrim SE 1.7.104 + SKSE 2.3.1 with every patch, including the TBB memory manager, and reach the main menu. **Stock Engine Fixes is the path for everyone.**
- **Migration** (`skse::migrate_from_engine_fixes_wine`, runs after deploys, collection installs and before launch): removes `0_SSEEngineFixesForWine.dll` and its helper files from `Data/SKSE/Plugins` unless a user-installed mod owns them, un-renames `.disabled` stock `EngineFixes.dll` / `d3dx9_42.dll` in staging and Data, re-deploys withheld stock files, re-links a Corkscrew-patched `EngineFixes.toml` from staging, and warns (reinstall Engine Fixes) when older versions patched the staging copy.
- **Config**: `use_wine_engine_fixes` / `use_original_engine_fixes` are still accepted when reading old `config.json` files and are dropped on the next save. Don't reintroduce them.

## Memory Management

Auto-memory lives at `~/.claude/projects/-Users-cashconway-Corkscrew/memory/`. The index (`MEMORY.md`, ~200 line limit) loads every conversation; detailed knowledge in topic files linked from it.

**Update memory after:** bug fixes (root cause + fix), feature completion (module list), releases (version numbers), RE discoveries, architecture changes, cross-repo changes.

**Do NOT save:** mid-task WIP, info already in CLAUDE.md or memory files, speculative conclusions.

**On compaction:** Re-read relevant memory files to recover context.
