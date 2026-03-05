# Corkscrew

A mod manager for CrossOver/Wine games on macOS and Linux. Tauri v2 + Svelte 5 + Rust.
License: GPL-3.0-or-later.

## Core Philosophy

**Goal: BETTER than Windows** — more performant, more optimized, more stable outcomes.
Acceptable minimum: parity with how modlists and mod managers work on Windows.
Any modlist that works on Windows should work at least as well under Wine/CrossOver.

## Coding Standards

- Write code to a senior-engineer standard
- Don't take existing conventions as gospel, but don't assume they're wrong either
- Always version-bump fixes (user preference)
- Never commit signing keys, tokens, or credentials — check MEMORY.md for what's sensitive

## Key Commands

```bash
cargo tauri dev                        # Development mode
cargo test                             # 706+ backend tests (run from src-tauri/)
npx svelte-check --threshold error     # Frontend type checking
./scripts/release.sh <version>         # Hybrid release (local macOS + CI Linux)
```

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

### Auto-Memory System

This project uses Claude Code's persistent auto-memory at:
`~/.claude/projects/-Users-cashconway-Corkscrew/memory/`

The main index (`MEMORY.md`, ~200 line limit) is loaded into every conversation automatically. Detailed knowledge lives in topic files linked from the index.

### Memory File Conventions

| File | Purpose |
|------|---------|
| `MEMORY.md` | Concise index. Project structure, key patterns, commands, version info. |
| `engine-fixes-wine.md` | Cross-repo integration: Corkscrew ↔ SSEEngineFixesForWine |
| `sseef-wine.md` | SSEEngineFixesForWine RE findings, architecture, crash history |
| `audit-findings.md` | Code quality audit results and fix tracking |
| `skse-plugin-compat.md` | SKSE plugin PE parsing and version compatibility system |
| `backlog.md` | Deferred work items and future features |

### When to Update Memory

**DO update after:**
- Fixing a bug — record root cause and fix in the appropriate topic file
- Completing a feature — update MEMORY.md module list and feature status
- A release — update version numbers across all memory files that reference them
- RE discoveries — update sseef-wine.md with new offsets, crash sites, findings
- Architecture changes — update MEMORY.md patterns and structure sections
- Cross-repo changes — update engine-fixes-wine.md if the integration surface changes

**DO NOT save:**
- Mid-task temporary state (current WIP, debugging attempts in progress)
- Information already in CLAUDE.md or existing memory files (check first)
- Speculative conclusions from reading a single file — verify before persisting

### Compaction Survival

When context is compressed mid-session:
1. Re-read the relevant memory files to re-anchor context
2. Check the conversation summary (provided by the system) for in-progress work state
3. Auto-memory files persist across sessions — use them to recover context
4. If working on SSEEngineFixesForWine, re-read both `engine-fixes-wine.md` and `sseef-wine.md`

### Cross-Repo Memory Hygiene

After changes to SSEEngineFixesForWine:
- Update version references in `engine-fixes-wine.md` and `sseef-wine.md`
- Update MEMORY.md's SSEEngineFixesForWine summary section
- If crash behavior changed, update the crash history in `sseef-wine.md`
- If TOML schema changed, update `engine-fixes-wine.md` config section
