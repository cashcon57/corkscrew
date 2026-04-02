---
name: review
description: Perform a code review of pending or specified changes. Checks for bugs, security issues, performance problems, Wine/CrossOver compatibility, NexusMods compliance, and Corkscrew-specific patterns. Use when the user asks to review code, check changes, or audit quality.
argument-hint: [file-or-branch]
allowed-tools: Read, Grep, Glob, Bash(git diff*), Bash(git log*), Bash(git show*), Bash(cargo check*), Bash(npx svelte-check*)
context: fork
agent: Explore
---

# Corkscrew Code Review

Review the changes specified by `$ARGUMENTS`. If no argument is given, review all uncommitted changes (staged + unstaged).

## How to gather changes

- No args: `git diff HEAD` for all pending changes
- File path: read the file and `git diff HEAD -- <path>`
- Branch name: `git diff main...<branch>`
- `--staged`: `git diff --cached`

## Review checklist

### 1. Correctness
- Logic errors, off-by-one, null/None handling
- Rust: unwrap() on user-facing paths (should use map_err or ?)
- Svelte 5: using old `$:` syntax instead of `$state`/`$derived`/`$effect`
- TypeScript: missing null checks on optional fields from Tauri invoke
- Edge cases: empty lists, missing files, disconnected state

### 2. Security
- **NEVER commit signing keys, tokens, or credentials** — check for `.keys/`, passwords, API keys
- Command injection via user-supplied paths in Bash/shell calls
- SQL injection in raw rusqlite queries (should use `params![]`)
- XSS in Svelte templates (unescaped `{@html}`)
- Path traversal: `..` in file paths that could escape game directory

### 3. NexusMods Compliance (CRITICAL)
- **NEVER automate downloads for free NM users** — must check `is_nexus_premium`
- API rate limit respect (delays between bulk API calls)
- Correct headers: `Application-Name: Corkscrew`, proper User-Agent
- No caching of NM API responses, no scraping
- GraphQL: don't use removed sort fields like `totalDownloads`

### 4. Wine/CrossOver Compatibility
- ENB requires `LinuxVersion=true` in enblocal.ini
- ENB requires DXVK (not D3DMetal) on macOS
- Community Shaders is incompatible with Wine — should be detected/warned
- DLL overrides: only touch `[Software\\Wine\\DllOverrides]`, not other Wine registry sections
- Launch pipeline must not revert DLL overrides set by other features

### 5. Corkscrew Patterns
- After mod state change: must call both `loadMods()` AND `refreshHealth()`
- `AppState.db` is `Arc<ModDatabase>` with internal Mutex — do NOT `.lock()` externally
- Svelte `@const` only inside `{#if}`/`{#each}` blocks
- Confirm-actions use `position: absolute` with z-index
- `auto_snapshot_before_destructive()` before purge/delete/clean operations
- Deploy journal for crash recovery on redeploy operations
- Game lock check (`check_game_lock`) before mod-modifying commands
- Deploy-in-progress flag should block game launch

### 6. Performance
- Unnecessary `redeploy_all()` when `deploy_incremental()` would suffice
- Missing progress events on long operations (>2s)
- N+1 database queries in loops — prefer batch operations
- Large file lists serialized across Tauri FFI boundary
- `spawn_blocking` for CPU-bound work in async Tauri commands

### 7. Error Handling
- Tauri commands should return `Result<T, String>` with descriptive errors
- Don't silently swallow errors (`let _ = ...`) on user-visible operations
- Silent failures OK for best-effort operations (snapshots, plugin sync)
- Frontend should show error toast on catch, not silently fail

### 8. Testing
- New Rust functions should have tests
- In-memory DB tests: use `Connection::open_in_memory()` + `migrations::migrate()`
- Test edge cases: empty input, missing files, concurrent access

## Output format

For each finding:

```
### [SEVERITY] Category — Short description
**File:** `path/to/file.rs:123`
**Issue:** Clear explanation of the problem
**Fix:** Specific suggested fix (code if appropriate)
```

Severities:
- **CRITICAL** — Will crash, lose data, violate NM compliance, or leak credentials
- **MAJOR** — Bug, missing error handling, or significant UX regression
- **MINOR** — Suboptimal pattern, missing progress feedback, style issue
- **NOTE** — Observation, suggestion, or question (not necessarily wrong)

## Final summary

End with a summary table:

| Severity | Count |
|----------|-------|
| Critical | X |
| Major | X |
| Minor | X |
| Note | X |

And an overall verdict: **APPROVE**, **APPROVE WITH NOTES**, or **REQUEST CHANGES**.
