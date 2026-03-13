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

## Cross-Repo Awareness

This workspace includes two tightly integrated repositories:

| Repo | Path | Purpose |
|------|------|---------|
| **Corkscrew** | `/Users/cashconway/Corkscrew/` | Mod manager (this repo) |
| **SSEEngineFixesForWine** | `/Users/cashconway/SSEEngineFixesForWine/` | Wine-compatible SKSE plugin |

### Integration Surface

- **Auto-deploy**: Corkscrew downloads and deploys SSEEngineFixesForWine DLL on game launch, collection install, and redeploy (`skse.rs`)
- **TOML patching**: Corkscrew disables original EngineFixes patches and deploys Wine version's TOML
- **DLL naming**: `0_SSEEngineFixesForWine.dll` — the `0_` prefix ensures early SKSE load order
- **Version tracking**: GitHub Releases API used to fetch latest version; no hardcoded versions

When modifying either repo, check if the change affects this integration surface. See `engine-fixes-wine.md` in auto-memory for full details.

## Memory Management

Auto-memory lives at `~/.claude/projects/-Users-cashconway-Corkscrew/memory/`. The index (`MEMORY.md`, ~200 line limit) loads every conversation; detailed knowledge in topic files linked from it.

**Update memory after:** bug fixes (root cause + fix), feature completion (module list), releases (version numbers), RE discoveries, architecture changes, cross-repo changes.

**Do NOT save:** mid-task WIP, info already in CLAUDE.md or memory files, speculative conclusions.

**On compaction:** Re-read relevant memory files to recover context. If working on SSEEngineFixesForWine, re-read both `engine-fixes-wine.md` and `sseef-wine.md`.
