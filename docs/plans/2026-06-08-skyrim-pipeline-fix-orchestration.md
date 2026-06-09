# Skyrim Modding Pipeline Fix Orchestration Plan

> **For Hermes:** Use `subagent-driven-development` to execute this plan. Fresh code agent per task, then self-review, then spec reviewer, then quality/security reviewer. Do not call any task done until reviewer pass + final Opus audit gate.

**Goal:** Fix every issue found in the full Skyrim modding pipeline review without creating new data-loss, Wine/Skyrim compatibility, Nexus compliance, or deployment-state regressions.

**Architecture:** Treat deploy target (`data` / `root` / `custom`) as a first-class invariant across DB, manifests, deploy, redeploy, incremental deploy, and collection installs. Split the work into isolated lanes to avoid agents racing on the same files. Every lane uses TDD/regression tests first, then minimal implementation, then two-stage review.

**Tech Stack:** Rust/Tauri backend, Svelte frontend only if needed, SQLite via `ModDatabase`, libloot, Nexus API, Wabbajack pipeline.

---

## Global Rules

1. **No broad rewrites.** Fix the smallest safe surface per task.
2. **TDD required.** Every code task starts with failing Rust tests or explicit fixture tests.
3. **No self-approval.** Each code agent must self-review, but that does not replace independent reviewers.
4. **Reviewer chain per task:**
   - Code Agent implements + self-review notes.
   - Spec Reviewer checks exact task requirements.
   - Quality/Security Reviewer checks edge cases, data loss, path safety, concurrency, and project invariants.
5. **Opus gate before “done.”** Final integration must be audited by Opus-style reviewer before reporting completion.
6. **Version bump last.** Do not bump until all gates pass.
7. **Current environment caveat:** this Hermes container lacks `cargo`/`rustc` and missing frontend deps. Code agents should still write tests; full verification must run in an environment with Rust toolchain. If toolchain becomes available, run `cargo test` from `src-tauri/` and `npx svelte-check --threshold error` from repo root.

---

## Agent Roles

### Orchestrator — Hermes/self
Owns sequencing, todo tracking, conflict avoidance, final merge decisions.

Responsibilities:
- Dispatch agents with complete task context.
- Ensure only one implementation lane touches a shared file at a time.
- Run available tests/checks.
- Re-dispatch fix agents for reviewer blockers.
- Maintain final issue matrix.

### Code Agent
Implements exactly one task.

Required output:
- Files changed.
- Tests added/updated.
- Commands run + pass/fail output.
- Self-review checklist:
  - Path safety
  - Data-loss safety
  - Deploy target consistency
  - Wine/Skyrim semantics
  - Nexus compliance if applicable

### Spec Reviewer
Checks only whether implementation satisfies the task spec.

Verdict schema:
```json
{
  "verdict": "PASS" | "REQUEST_CHANGES",
  "missing_requirements": [],
  "scope_creep": [],
  "required_fixes": []
}
```

### Quality/Security Reviewer
Checks code quality, security, regressions, edge cases.

Verdict schema:
```json
{
  "verdict": "APPROVED" | "REQUEST_CHANGES",
  "critical": [],
  "important": [],
  "minor": [],
  "test_gaps": []
}
```

### Final Integration Reviewer
Reviews cross-lane behavior after all tasks land.

Checks:
- Full deploy lifecycle: install → deploy → sort → disable plugin → sync → incremental deploy → full redeploy → purge.
- Root/data/custom separation.
- Manifest identity.
- Wabbajack partial failure behavior.
- SKSE uninstall/compatibility.
- Path traversal defenses.

---

## Dependency Graph

```text
Phase 0 Inventory
  ↓
Phase 1 Deploy target identity foundation
  ↓
Phase 2 Collection root/data split
  ↓
Phase 3 Incremental/full redeploy correctness
  ↓
Phase 4 Plugin/load-order state correctness
  ↓
Phase 5 SKSE safety/compatibility
  ↓
Phase 6 Wabbajack/Nexus/FOMOD hardening
  ↓
Phase 7 Archive/path safety hardening
  ↓
Phase 8 Integration tests + final audits + version bump
```

Only parallelize tasks that do not touch overlapping files.

---

# Phase 0 — Inventory + Baseline

## Task 0.1: Capture Current State

**Objective:** Establish exact baseline and current failures before edits.

**Files:** none.

**Commands:**
```bash
cd /home/cashc/.cache/my-github-repos/corkscrew
git status --short
git rev-parse HEAD
git ls-files src-tauri/src | grep -E 'deployer|database|collection_installer|skyrim|skse|wabbajack|nexus|installer|staging|loot'
```

If toolchain exists:
```bash
cd src-tauri
cargo test --lib
cd ..
npx svelte-check --threshold error
```

**Reviewer:** none; orchestrator records baseline.

---

# Phase 1 — Deploy Target Identity Foundation

## Task 1.1: Make deployment identity include deploy target everywhere

**Objective:** Fix root/data/custom manifest collisions by making `(relative_path, deploy_target)` the deployment identity in DB maps/removals and deployer maps.

**Files:**
- Modify: `src-tauri/src/database.rs`
- Modify: `src-tauri/src/deployer.rs`
- Tests: existing tests in those files; add new tests near deployment manifest map tests.

**Known bugs covered:**
- Root/data manifest entries collide by `relative_path`.
- Batch removal ignores deploy target.

**Implementation requirements:**
1. Add/adjust DB helpers so deployment maps can key by `(relative_path, deploy_target)`.
2. Keep backwards-compatible APIs only where callers truly need path-only behavior.
3. Update deployer conflict map from `HashMap<String, i64>` to target-aware key.
4. Update batch removals to include target, or create target-aware removal helper.
5. Add regression test:
   - Insert two manifest entries with same `relative_path`, one `data`, one `root`.
   - Verify both survive map construction.
   - Remove only one target; verify the other remains.

**Code Agent prompt:**
```text
Implement Task 1.1 in Corkscrew. Focus only on target-aware deployment identity. Add failing tests first. Do not touch collection root/data split yet. Return files changed, tests, commands run, and self-review notes.
```

**Spec Reviewer prompt:**
```text
Review Task 1.1. Verify deployment identity is target-aware in database and deployer conflict/removal paths. Check tests prove same relative_path can exist for root and data without collision.
```

**Quality/Security Reviewer prompt:**
```text
Review Task 1.1 for migration safety, backward compatibility, stale path-only callsites, and data-loss risk. Look for any remaining HashMap keyed only by relative_path in deployment lifecycle.
```

---

## Task 1.2: Persist deploy target as durable mod metadata

**Objective:** Ensure full redeploy can recover original deploy target after manifest purge.

**Files:**
- Modify: `src-tauri/src/database.rs`
- Modify: `src-tauri/src/deployer.rs`
- Modify: `src-tauri/src/collection_installer.rs`
- Tests: `database.rs`, `deployer.rs`

**Known bugs covered:**
- `redeploy_all()` loses root/custom deploy targets after purge.

**Implementation requirements:**
1. Add durable per-mod deploy target metadata (`data`, `root`, `custom`) to DB schema or an associated table.
2. Add migration for existing DBs; default missing target to `data`.
3. When collection installs deploy a mod, persist `deploy_target_str` before/with manifest rows.
4. `get_deploy_target_for_mod()` must prefer durable mod metadata, not existing manifest rows.
5. Add regression test:
   - Mod deployed as `root`.
   - Purge manifest.
   - `get_deploy_target_for_mod(mod_id)` still returns `root`.

**Reviewer focus:** schema migration correctness; existing DB compatibility; no target loss during purge.

---

# Phase 2 — Collection Root/Data Split

## Task 2.1: Split Root/ and Data files into separate deploy batches

**Objective:** Fix mixed `Root/` + `Data/` mods so only Root files deploy to game root and Data files deploy to Data.

**Files:**
- Modify: `src-tauri/src/collection_installer.rs:3969-4658`
- Possibly modify: `src-tauri/src/deployer.rs` only if interface needs batch helper.
- Tests: add focused unit/integration test if collection installer has fixtures; otherwise add testable helper function.

**Known bugs covered:**
- Mixed `Root/` + `Data/` collection mods deploy Data files to game root.

**Implementation requirements:**
1. Do not flatten Root files into the same logical file list as Data files without tagging target.
2. Produce two batches:
   - `root_files`: deploy to `game_path`, target `root`.
   - `data_files`: deploy to `data_dir`, target `data`, unless Vortex custom routing applies.
3. Preserve collection file overrides for the correct target only.
4. Persist correct deploy target metadata for each mod or support multi-target mod metadata if needed.
5. If one mod can deploy both root and data files, manifest must represent both targets without collisions.
6. Add test fixture:
   - Staging contains `Root/skse64_loader.exe` and `Scripts/Foo.pex` / `SKSE/Plugins/Foo.dll`.
   - After deploy planning, root file target is `root`; data files target `data`.

**Reviewer focus:** no regression for pure root mods, pure data mods, Vortex custom route mods.

---

## Task 2.2: Fail zero-file deploys instead of false-success installs

**Objective:** Collection installs must not report success when no files deploy.

**Files:**
- Modify: `src-tauri/src/collection_installer.rs:4634-4651`
- Modify: `src-tauri/src/deployer.rs` if deployer needs stronger error semantics.
- Tests: collection deploy error handling test.

**Known bugs covered:**
- Collection deploy failure with zero files reported as installed.
- Atomic deploy partial failures swallowed.

**Implementation requirements:**
1. Remove the “0 of … files deployed” success downgrade.
2. Return `InstallError::Failed` or explicit staged-not-deployed state.
3. DB mod record cleanup must be deterministic on failure.
4. Keep staging only if explicit resumable status exists and UI understands it; otherwise clean up.
5. Add test that zero deploy returns error and does not mark mod installed.

---

# Phase 3 — Incremental + Full Redeploy Correctness

## Task 3.1: Make incremental deploy target-aware

**Objective:** Incremental deploy must add/update/remove files from root/data/custom base dirs based on `DesiredFile.deploy_target`.

**Files:**
- Modify: `src-tauri/src/deployer.rs:1104-1545`
- Tests: `deployer.rs`

**Known bugs covered:**
- Incremental deploy computes target but always uses `data_dir`.
- Stale root files remain; root files get added to Data.

**Implementation requirements:**
1. Add helper: `resolve_deploy_base(data_dir, game_path, deploy_target, maybe_custom_path)`.
2. Use it for remove/update/add paths.
3. Diff keys must include deploy target from Phase 1.
4. Add tests:
   - Root file added to game root.
   - Data file added to Data.
   - Removing disabled root mod removes root file, not Data file.
   - Same relative path in root and data can update independently.

---

## Task 3.2: Compare content/hash/target, not just mod_id, in incremental diff

**Objective:** Restaged or patched same-mod files must update deployed content.

**Files:**
- Modify: `src-tauri/src/deployer.rs:1168-1203`
- Tests: `deployer.rs`

**Known bugs covered:**
- Incremental diff ignores hash changes for same mod.

**Implementation requirements:**
1. Compare `mod_id`, `sha256`, `staging_path`, and `deploy_target`.
2. If current manifest lacks hash, decide safe fallback: update or verify filesystem hash.
3. Add regression test:
   - Mod ID same, relative path same, staging hash changed.
   - `compute_diff()` yields update.

---

## Task 3.3: Make deploy failures actually fail atomic deploy

**Objective:** `deploy_mod_atomic*` must roll back on failed expected files, copy/link failures, or zero successful deployments.

**Files:**
- Modify: `src-tauri/src/deployer.rs:244-585`
- Tests: `deployer.rs`

**Known bugs covered:**
- “Atomic” deploy swallows partial file failures.

**Implementation requirements:**
1. Track expected deploy count after junk/path-safety filters.
2. Treat missing source/copy failure as deploy error unless explicitly skipped for symlink safety.
3. Return structured error with failed file list.
4. Ensure atomic wrapper rollback runs on partial failure.
5. Add tests:
   - One source missing among two expected files returns Err and rolls back first file.
   - Zero deployed returns Err.

---

## Task 3.4: Stop deleting unmanaged loose files blindly

**Objective:** Prevent deploy from deleting unmanaged/vanilla loose files.

**Files:**
- Modify: `src-tauri/src/deployer.rs:392-407`
- Tests: `deployer.rs`

**Known bugs covered:**
- Deploy deletes unmanaged loose files before linking/copying mod file.

**Implementation requirements:**
1. If destination exists and is not manifest-owned, do not remove by default.
2. Either skip with conflict warning or backup/snapshot first.
3. Respect vanilla baseline where available.
4. Add test: unmanaged `textures/foo.dds` survives deploy attempt and is not overwritten/deleted.

---

## Task 3.5: Avoid hardlinks for mutable Skyrim file classes

**Objective:** Prevent game/runtime mutation from corrupting staging source of truth.

**Files:**
- Modify: `src-tauri/src/deployer.rs`
- Tests: `deployer.rs`

**Known bugs covered:**
- Hardlink deploy lets game mutate staging files.

**Implementation requirements:**
1. Add function `should_copy_instead_of_hardlink(rel_path)`.
2. Copy instead of hardlink for `.ini`, `.toml`, `.json`, `.yaml`, `.yml`, `.log`, `.cfg`, `.txt`, SKSE plugin config paths, ENB/ReShade config files.
3. Keep large immutable assets hardlink-capable.
4. Add test: `.ini` deploy produces distinct inode or mutation of dest does not change staging content.

---

# Phase 4 — Plugin / Load Order Correctness

## Task 4.1: Preserve disabled ESP state in plugin sync

**Objective:** `sync_plugins()` must not re-enable user-disabled ESPs.

**Files:**
- Modify: `src-tauri/src/plugins/skyrim_plugins.rs:280-407`
- Tests: `plugins/skyrim_plugins.rs`

**Known bugs covered:**
- `sync_plugins()` re-enables disabled ESPs.

**Implementation requirements:**
1. Force implicit plugins enabled.
2. Force `.esm` and `.esl` enabled if that is intentional.
3. Preserve existing enabled state for `.esp`.
4. New `.esp` default can remain enabled if that is product choice, but existing disabled must stay disabled.
5. Add regression test with existing `MyMod.esp` disabled, sync, remains disabled.

---

## Task 4.2: Validate plugin names from frontend/commands

**Objective:** Prevent newline/comment/path injection into `plugins.txt`/`loadorder.txt`.

**Files:**
- Modify: `src-tauri/src/plugins/skyrim_plugins.rs`
- Modify: `src-tauri/src/commands/plugins.rs`
- Modify: `src-tauri/src/loot.rs:500-503`
- Tests: `plugins/skyrim_plugins.rs`, `loot.rs` if practical.

**Known bugs covered:**
- Plugin names from frontend can inject lines.
- `get_plugin_messages()` joins untrusted `plugin_name` under Data.

**Implementation requirements:**
1. Add validator for plugin filename:
   - basename only
   - extension `.esp/.esm/.esl`
   - no `/`, `\`, `\0`, `\n`, `\r`, leading `*`, leading `#`
2. Apply to reorder/toggle/move/get messages.
3. Prefer requiring file exists in Data for commands that mutate state.
4. Add tests for rejected malicious names.

---

## Task 4.3: Make Bethesda plugin file writes atomic

**Objective:** Avoid empty/partial plugin files on crash/disk-full.

**Files:**
- Modify: `src-tauri/src/plugins/skyrim_plugins.rs:155-220`
- Tests: `plugins/skyrim_plugins.rs`

**Known bugs covered:**
- `plugins.txt` and `loadorder.txt` direct truncate writes.

**Implementation requirements:**
1. Reuse or duplicate atomic sibling-temp rename helper.
2. Ensure parent dirs created.
3. Add test proving existing file remains if temp write/rename path fails where practical.

---

## Task 4.4: Fix LOOT sort write gate and default enabled semantics

**Objective:** LOOT sort must write when order differs by length/addition, and must not disable newly sorted existing deployed plugins accidentally.

**Files:**
- Modify: `src-tauri/src/loot.rs:451-456`
- Modify: `src-tauri/src/commands/plugins.rs:27-66`
- Tests: `loot.rs`, `commands/plugins.rs` or helper tests.

**Known bugs covered:**
- LOOT sort may discard additions.
- LOOT sort writes unknown plugins disabled.

**Implementation requirements:**
1. Compare full current order vs sorted order, including length.
2. In command layer, for sorted plugin not in existing map but exists on disk, default enabled unless product explicitly requires disabled.
3. Add test: current `[A]`, sorted `[A, B]` counts moved/changed and writes.

---

# Phase 5 — SKSE Safety + Compatibility

## Task 5.1: Safe SKSE uninstall

**Objective:** Uninstall SKSE without deleting unrelated `Data/SKSE/` plugin/config files.

**Files:**
- Modify: `src-tauri/src/skse.rs:272-300`
- Tests: `skse.rs`

**Known bugs covered:**
- `uninstall_skse()` deletes whole `Data/SKSE/`.

**Implementation requirements:**
1. Remove only SKSE-distributed files:
   - game-root SKSE loader/exe/dll files
   - known SKSE scripts if owned by SKSE archive
2. Do not remove `Data/SKSE/Plugins` directory or third-party plugin configs.
3. Add regression test: `Data/SKSE/Plugins/Foo.dll` survives uninstall.

---

## Task 5.2: Fix SKSE version compatibility classification

**Objective:** Use exact runtime compatibility data, not broad SE/AE labels.

**Files:**
- Modify: `src-tauri/src/skse.rs:697-795`
- Tests: `skse.rs`

**Known bugs covered:**
- `2.1.x` misclassified.
- Compatibility ignores exact min/max runtime.

**Implementation requirements:**
1. Make `skse_game_compatibility()` derive from version DB used by installer.
2. `check_skse_compatibility()` must compare detected game runtime to exact supported range.
3. Add tests for:
   - `2.1.5` maps to AE `1.6.353` family, not SE.
   - SKSE `2.2.3` rejected for unsupported AE runtime if not in range.

---

## Task 5.3: Select EngineFixes Wine asset by detected runtime

**Objective:** Avoid installing AE EngineFixes Wine on downgraded SE.

**Files:**
- Modify: `src-tauri/src/skse.rs:2026-2311`
- Tests: `skse.rs`

**Known bugs covered:**
- EngineFixes Wine asset selection always prefers AE.

**Implementation requirements:**
1. Detect Skyrim runtime before choosing release asset.
2. Pick SE vs AE asset by runtime.
3. If no matching asset, return actionable error instead of wrong install.
4. Add unit tests around asset-selection helper.

---

## Task 5.4: Make SKSE/EngineFixes temp dirs unique and safe

**Objective:** Avoid concurrent install races and predictable temp dir cleanup.

**Files:**
- Modify: `src-tauri/src/skse.rs:349-353`, `528-536`, `2124-2131`, `2304-2311`
- Tests: `skse.rs`

**Known bugs covered:**
- Fixed global temp dirs under `std::env::temp_dir()`.

**Implementation requirements:**
1. Use `tempfile::TempDir` or `Builder` for each operation.
2. No hardcoded `/tmp/skse_extract`-style dirs.
3. Add test or helper coverage verifying unique temp paths.

---

## Task 5.5: Harden SKSE recursive copy against symlinks/path escape

**Objective:** Do not follow symlinks or copy outside extraction tree.

**Files:**
- Modify: `src-tauri/src/skse.rs:895-912`
- Tests: `skse.rs`

**Known bugs covered:**
- `copy_dir_recursive()` follows symlinks from extracted archives.

**Implementation requirements:**
1. Skip symlink entries.
2. Canonicalize source/dest containment where possible.
3. Add malicious symlink fixture test.

---

# Phase 6 — Wabbajack / Nexus / FOMOD Hardening

## Task 6.1: Fix Wabbajack extraction retry path

**Objective:** Re-download retry must extract the new path.

**Files:**
- Modify: `src-tauri/src/wabbajack_installer.rs:1390-1428`
- Tests: add helper test if direct async integration is too heavy.

**Known bugs covered:**
- Extraction retry ignores `_new_path`.

**Implementation requirements:**
1. Make per-archive download path mutable.
2. Use returned path from `download_archive()` on retry.
3. Re-verify hash after re-download if metadata has hash.
4. Add test around retry helper or extract loop state machine.

---

## Task 6.2: Fail Wabbajack install on missing required archives

**Objective:** Prevent partial WJ modlists from proceeding.

**Files:**
- Modify: `src-tauri/src/wabbajack_installer.rs:1591-1622`
- Tests: `wabbajack_installer.rs`

**Known bugs covered:**
- Wabbajack installs can complete with missing required archives.

**Implementation requirements:**
1. Model optional/manual archives explicitly if supported.
2. Otherwise any failed required archive returns install failure before directives.
3. Emit clear progress event + DB failed status.
4. Test one failed of N required archives => Err.

---

## Task 6.3: Validate Wabbajack GameFileSource paths

**Objective:** Stop WJ metadata from reading arbitrary local files or writing outside extraction temp.

**Files:**
- Modify: `src-tauri/src/wabbajack_downloader.rs:511-520`
- Modify: `src-tauri/src/wabbajack_installer.rs:1348-1359`
- Tests: both files or shared helper tests.

**Known bugs covered:**
- `GameFileSource.game_file` path traversal.

**Implementation requirements:**
1. Use shared safe-relative path validator after normalizing `\` to `/`.
2. Reject absolute paths, `..`, drive letters, nulls.
3. Canonicalize source under detected game root.
4. Canonicalize destination under extraction dest.
5. Add tests for `../secret`, `C:\foo`, `/etc/passwd`, normal `Data/foo.bsa`.

---

## Task 6.4: Make collection bundle/FOMOD metadata failure blocking when unsafe

**Objective:** Avoid silent default FOMOD choices for curated collections.

**Files:**
- Modify: `src-tauri/src/collection_installer.rs:829-854`, `4080-4123`
- Tests: `collection_installer.rs`

**Known bugs covered:**
- Collection bundle/FOMOD metadata fetch failure falls back to defaults.

**Implementation requirements:**
1. If bundle fetch fails and collection contains FOMODs/patches/rules or unknown metadata, fail or require explicit user confirmation.
2. Safe fallback only if proven no choices/patches/rules needed.
3. Add test: bundle fetch failure + FOMOD mod => install blocks.

---

## Task 6.5: Reuse Nexus compliance headers in Wabbajack downloader

**Objective:** Ensure WJ Nexus API calls comply with Corkscrew Nexus invariant.

**Files:**
- Modify: `src-tauri/src/wabbajack_downloader.rs:130-199`
- Possibly modify: `src-tauri/src/nexus.rs` to expose header builder.
- Tests: WJ downloader client/header unit test.

**Known bugs covered:**
- Wabbajack Nexus calls omit required Nexus application headers.

**Implementation requirements:**
1. Reuse `NexusClient` or shared header construction.
2. Include `Application-Name`, `Application-Version`, `Protocol-Version`, no-cache if required.
3. Preserve Premium-only behavior.

---

## Task 6.6: Fix Nexus resume filename mismatch

**Objective:** Resume must append to the same file whose length was measured.

**Files:**
- Modify: `src-tauri/src/nexus.rs:774-858`
- Tests: `nexus.rs` with mocked response/headers if practical.

**Known bugs covered:**
- Resume can append to wrong filename when `Content-Disposition` differs from URL.

**Implementation requirements:**
1. Decide canonical filename before resume or store partial metadata.
2. If response filename differs, restart safely rather than append corruptly.
3. Add test for URL filename `download` + Content-Disposition `Mod.7z`.

---

# Phase 7 — Archive / Staging Safety

## Task 7.1: Pre-validate or sandbox 7z/RAR extraction

**Objective:** Prevent archive symlink/path escapes from writing outside dest before cleanup.

**Files:**
- Modify: `src-tauri/src/installer.rs:817-1016`
- Tests: `installer.rs`

**Known bugs covered:**
- 7z/RAR extraction validates after extraction.

**Implementation requirements:**
1. Prefer list entries first (`7z l -slt` or library equivalent) and reject unsafe entries before extraction.
2. If library cannot pre-list safely, extract in hardened temp sandbox, reject symlink entries, then copy safe regular files into final dest using safe copy.
3. Add tests/fixtures for traversal and symlink archive if feasible.

---

## Task 7.2: Make staging temp dirs unique

**Objective:** Avoid same-process staging races.

**Files:**
- Modify: `src-tauri/src/staging.rs:251-257`
- Tests: `staging.rs`

**Known bugs covered:**
- `stage_mod()` uses process-wide temp dir.

**Implementation requirements:**
1. Replace `std::env::temp_dir().join(format!("corkscrew_stage_{}", pid))` with `tempfile::Builder`.
2. Ensure cleanup via guard.
3. Add test that two calls get distinct temp roots if helper extracted.

---

# Phase 8 — Final Integration + Release Hygiene

## Task 8.1: End-to-end deployment lifecycle tests

**Objective:** Prove the fixed pipeline works as a system.

**Files:**
- Add/modify tests in `src-tauri/src/deployer.rs`, `collection_installer.rs`, `plugins/skyrim_plugins.rs` as appropriate.

**Scenario tests:**
1. Install mixed root/data mod.
2. Deploy root + data files to correct bases.
3. Disable ESP, sync plugins, disabled state persists.
4. Incremental deploy after priority change preserves targets.
5. Full redeploy after purge preserves targets.
6. Same relative path in root/data remains distinct.
7. SKSE uninstall preserves third-party plugin file.
8. Wabbajack one missing required archive fails before directives.

---

## Task 8.2: Full local verification

**Objective:** Run every available project check.

**Commands:**
```bash
cd /home/cashc/.cache/my-github-repos/corkscrew/src-tauri
cargo test --lib
cargo test

cd /home/cashc/.cache/my-github-repos/corkscrew
npx svelte-check --threshold error
npm test -- --runInBand 2>/dev/null || true
```

If current environment still lacks toolchain, record exact missing tool output and require running on dev machine/CI before merge.

---

## Task 8.3: Final independent audits

**Objective:** Do not report done until final reviewers pass.

**Final Reviewers:**
1. **Integration Reviewer** — fresh subagent reviews entire diff and test results.
2. **Security/Data-loss Reviewer** — fresh subagent focuses only on path traversal, symlink, deploy deletes, manifest collisions, Nexus compliance.
3. **Opus Audit Gate** — final architectural/security audit. Required before marking complete.

**Final reviewer prompt:**
```text
Review the full Corkscrew Skyrim modding pipeline fix diff. Original findings included deploy target loss/collision, mixed Root/Data misdeploy, plugin disabled-state loss, SKSE uninstall data loss, Wabbajack partial install success, path traversal, archive extraction escape, Nexus compliance, and non-atomic writes. Verify every original finding is fixed and no new regression exists. Verdict: APPROVED or REQUEST_CHANGES with exact blockers.
```

---

## Task 8.4: Version bump + changelog

**Objective:** Follow project preference to version-bump fixes after all gates pass.

**Files:**
- Modify: `package.json`
- Modify: `src-tauri/tauri.conf.json` or Cargo metadata if version lives there.
- Modify: `CHANGELOG.md`

**Requirements:**
1. Only after all tests/audits pass.
2. Changelog groups:
   - Fixed: Skyrim deployment target handling.
   - Fixed: Plugin disabled-state preservation.
   - Fixed: SKSE uninstall safety.
   - Security: path traversal/archive extraction hardening.
   - Compliance: Wabbajack Nexus headers.

---

# Orchestrator Execution Template

For each task:

```text
1. Dispatch Code Agent:
   - Full task text
   - Relevant prior findings
   - Files allowed to touch
   - TDD requirement
   - Self-review checklist

2. Wait for completion.

3. Dispatch Spec Reviewer:
   - Original task text
   - Claimed changed files
   - Ask for JSON verdict

4. If REQUEST_CHANGES:
   - Dispatch Fix Agent with exact reviewer blockers
   - Repeat Spec Reviewer

5. Dispatch Quality/Security Reviewer:
   - Original task text
   - Changed files/diff
   - Ask for JSON verdict

6. If REQUEST_CHANGES:
   - Dispatch Fix Agent
   - Repeat Spec + Quality reviews

7. Mark task complete only after both pass.
```

---

# Parallelization Plan

Safe parallel lanes after Phase 1 foundation lands:

```text
Lane A: Collection root/data split + zero deploy failures
  files: collection_installer.rs, deployer call boundaries

Lane B: Plugin/load-order fixes
  files: plugins/skyrim_plugins.rs, commands/plugins.rs, loot.rs

Lane C: SKSE fixes
  files: skse.rs

Lane D: Wabbajack/Nexus/FOMOD fixes
  files: wabbajack_installer.rs, wabbajack_downloader.rs, nexus.rs, collection_installer.rs

Lane E: Archive/staging safety
  files: installer.rs, staging.rs
```

Conflict rule: if two lanes need `collection_installer.rs` or `deployer.rs`, serialize them.

---

# Acceptance Criteria

All original review findings are closed:

- [ ] Mixed Root/Data mods deploy to correct targets.
- [ ] Deploy target survives purge/full redeploy.
- [ ] Incremental deploy uses target-aware bases.
- [ ] Manifest identity includes target.
- [ ] Disabled ESPs stay disabled after sync.
- [ ] Wabbajack fails on missing required archive.
- [ ] Wabbajack extraction retry uses new path.
- [ ] SKSE uninstall preserves third-party SKSE files.
- [ ] Unmanaged loose files are not deleted blindly.
- [ ] Atomic deploy errors on partial/zero deployment and rolls back.
- [ ] Mutable config classes are not hardlinked.
- [ ] Bundle/FOMOD metadata failure does not silently default unsafe installs.
- [ ] GameFileSource paths are validated/canonicalized.
- [ ] SKSE compatibility is exact-runtime aware.
- [ ] EngineFixes Wine asset matches detected runtime.
- [ ] 7z/RAR extraction cannot write outside destination.
- [ ] Plugin file writes are atomic.
- [ ] Plugin command input validates filenames.
- [ ] LOOT sort writes on full order diff.
- [ ] Wabbajack Nexus headers match compliance invariant.
- [ ] Nexus resume cannot append to wrong file.
- [ ] Full Rust tests pass in toolchain-enabled environment.
- [ ] `npx svelte-check --threshold error` passes.
- [ ] Final integration reviewer approves.
- [ ] Final security/data-loss reviewer approves.
- [ ] Opus audit gate approves.

---

# Rollback Plan

Before implementation:
```bash
git switch -c fix/skyrim-pipeline-integrity
```

Rollback individual task:
```bash
git diff --name-only
git restore <files from failed task>
```

Rollback full branch:
```bash
git switch main
git branch -D fix/skyrim-pipeline-integrity
```

If DB migration is added:
- Migration must be additive or reversible.
- Include downgrade note in changelog if app has no migration rollback system.

---

# Notes for Agents

- Do not ask user for credentials.
- Do not touch signing/notarization/secrets.
- Do not run release script.
- Do not broaden game support; this is Skyrim pipeline integrity only.
- Preserve Wine/CrossOver case-insensitive path behavior.
- Collection installer is reference architecture; changes to Wabbajack should converge toward it, not invent a separate model.
