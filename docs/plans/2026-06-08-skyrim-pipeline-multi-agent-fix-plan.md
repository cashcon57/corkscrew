# Skyrim Modding Pipeline Multi-Agent Fix Plan

> **For Hermes:** Use `subagent-driven-development` to execute this plan. Every code agent must self-review, then hand off to independent spec + quality/security reviewers. No task is complete until reviewers approve. Final completion requires an external Opus/deep reviewer gate before reporting done.

**Goal:** Fix every blocking issue found in the Skyrim/Wabbajack/collection modding pipeline audit.

**Repo:** `/home/cashc/.cache/my-github-repos/corkscrew`

**Baseline audit date:** 2026-06-08

**Primary risks:** path traversal / arbitrary file write, false-success installs, non-fatal missing archives/directives, incorrect FOMOD/plugin state, insufficient regression coverage.

---

## Orchestration Model

### Agent Roles

| Role | Count | Responsibility |
|---|---:|---|
| Coordinator | 1 | Owns branch/worktree layout, dependencies, merge order, global verification |
| Code Agents | 5 | Implement scoped fixes in isolated worktrees/branches |
| Self-Reviewers | 5 | Same code agent must run local diff/test/security checklist before handing off |
| Spec Reviewers | 5 | Independent reviewer checks task acceptance criteria only |
| Quality/Security Reviewers | 5 | Independent reviewer checks code quality, security, regressions |
| Integration Reviewer | 1 | Reviews merged result across workstreams |
| Final Deep Reviewer | 1 | Opus/Qwen-class holistic audit before saying done |

### Branch / Worktree Layout

Use isolated worktrees to avoid agents clobbering each other:

```bash
cd /home/cashc/.cache/my-github-repos/corkscrew
git fetch origin
mkdir -p /tmp/corkscrew-agent-worktrees

git worktree add /tmp/corkscrew-agent-worktrees/tooling -b fix/skyrim-pipeline-tooling main
git worktree add /tmp/corkscrew-agent-worktrees/wj-paths -b fix/wj-path-safety main
git worktree add /tmp/corkscrew-agent-worktrees/wj-failclosed -b fix/wj-fail-closed main
git worktree add /tmp/corkscrew-agent-worktrees/collection-failclosed -b fix/collection-fail-closed main
git worktree add /tmp/corkscrew-agent-worktrees/plugin-state -b fix/plugin-disabled-state main
git worktree add /tmp/corkscrew-agent-worktrees/integration -b fix/skyrim-pipeline-integration main
```

Merge order into integration branch:
1. `fix/skyrim-pipeline-tooling`
2. `fix/wj-path-safety`
3. `fix/wj-fail-closed`
4. `fix/collection-fail-closed`
5. `fix/plugin-disabled-state`

---

## Global Gates

### Gate 0 — Tooling Preflight

Before code work starts:

```bash
cd /home/cashc/.cache/my-github-repos/corkscrew
command -v cargo
command -v node
command -v npm
test -d node_modules || npm ci
npm run prepare || true
cd src-tauri && cargo test --no-run
cd .. && npx svelte-check --threshold error
```

If `cargo` is absent, Tooling Agent fixes dev env first. No implementation agent may claim verification until this gate passes or the coordinator records a hard environment blocker.

### Gate 1 — Per-Agent Self Review

Every code agent must run before handoff:

```bash
git diff --stat main...HEAD
git diff main...HEAD
# targeted tests for changed modules
# targeted grep for unsafe patterns
```

Self-review checklist:
- [ ] New tests fail before fix where practical
- [ ] New tests pass after fix
- [ ] No new silent warning-only failure paths for critical operations
- [ ] No unsafe `Path::join` with external data without `is_safe_relative_path` + base validation
- [ ] No `unwrap()` on user-facing paths unless test-only
- [ ] Errors are surfaced to UI/progress events and DB status

### Gate 2 — Independent Spec Review

Spec reviewer gets:
- original task section from this plan
- changed file list
- diff
- test output

Spec reviewer returns only:

```json
{
  "verdict": "PASS" | "REQUEST_CHANGES",
  "missing_requirements": [],
  "extra_scope": [],
  "notes": []
}
```

### Gate 3 — Independent Quality/Security Review

Quality reviewer gets same package plus Corkscrew review checklist. Must review:
- path traversal / TOCTOU
- false-success states
- rollback / DB consistency
- Wabbajack/Nexus/FOMOD semantics
- tests actually assert failure modes

Reviewer returns:

```json
{
  "verdict": "APPROVED" | "REQUEST_CHANGES",
  "critical": [],
  "major": [],
  "minor": [],
  "tests_to_add": []
}
```

Any `critical` or `major` blocks merge.

### Gate 4 — Integration Review

After all branches merge into `fix/skyrim-pipeline-integration`:

```bash
cd /tmp/corkscrew-agent-worktrees/integration
npm ci
npm run prepare || true
npx svelte-check --threshold error
cd src-tauri
cargo test
cargo test wabbajack
cargo test collection
cargo test fomod
cargo test skyrim
cargo test loot
```

Integration reviewer checks cross-module behavior and DB/progress consistency.

### Gate 5 — Final Deep Reviewer

Before saying done, send full merged diff + test output + this plan to a final deep reviewer. Required verdict: ship/no-ship.

If Opus tool is unavailable in this Hermes runtime, use the strongest available reviewer (`delegate_task` with fresh context + Qwen bridge) and explicitly label that Opus was unavailable. Do not call the implementation complete without a final independent deep review.

---

# Workstream A — Tooling + Regression Harness

## Agent A: Tooling Code Agent

**Objective:** Make the repo testable on `homelab-desktop` and add focused regression test harness scaffolding where needed.

**Files:**
- Modify only if needed: `.github/workflows/ci.yml`, `package.json`, `src-tauri/Cargo.toml`
- Add tests under existing module test blocks, not new framework unless needed

**Tasks:**
1. Diagnose missing `cargo` on host.
2. If allowed, install user-local Rust toolchain or document exact blocker.
3. Run `npm ci` to restore frontend deps.
4. Run `npm run prepare || true` to generate `.svelte-kit/tsconfig.json` if needed.
5. Establish baseline test commands and capture output.
6. Do not change production behavior except minimal test/tooling fixes.

**Acceptance:**
- `npx svelte-check --threshold error` runs to completion.
- `cargo test --no-run` runs, or blocker is explicit and reproducible.
- Coordinator has exact commands for all later agents.

**Reviewers:**
- Spec Reviewer A: verifies tooling gate reproducibility.
- Quality Reviewer A: verifies no unrelated production changes.

---

# Workstream B — Wabbajack Path Safety

## Agent B: WJ Path-Safety Code Agent

**Objective:** Fail closed on unsafe Wabbajack paths and prevent archive/game-file traversal.

**Files:**
- Modify: `src-tauri/src/wabbajack_installer.rs`
- Modify: `src-tauri/src/wabbajack_directives.rs`
- Possibly modify/create helper in: `src-tauri/src/staging.rs` or `src-tauri/src/wabbajack_types.rs`

**Required fixes:**
1. `wabbajack_installer.rs:1348-1358`
   - Validate `GameFileSource.game_file` after slash normalization.
   - Reject absolute paths, parent traversal, null bytes, drive prefixes.
   - Validate final `dest_file` remains under `extract_dest` before copy.
2. `wabbajack_directives.rs:1408-1453`
   - Replace basename sanitization with `PathTraversal` error.
   - Do not silently redirect unsafe `to` paths.
3. `wabbajack_directives.rs:1356-1404`
   - Validate assembled archive-relative path before lookup.
   - Validate paths returned by `case_insensitive_find` remain under `archive_dir`.
   - Disable filename fallback for unsafe archive paths.

**Tests to add:**
- Unsafe `GameFileSource` path `../escape.dll` fails and does not create file outside extraction dir.
- Absolute `GameFileSource` path fails.
- Unsafe directive `to = "../../evil.esp"` returns `PathTraversal`, not basename output.
- Unsafe archive path part fails before exact/case-insensitive/fallback lookup.
- Safe normal Windows-style path still works.

**Self-review grep:**

```bash
grep -RIn "join(&inner_name)\|join(inner_name)\|file_name().*unknown\|sanitizing" src-tauri/src/wabbajack_*.rs src-tauri/src/wabbajack_installer.rs
```

**Acceptance:**
- No external WJ metadata path reaches `Path::join` without validation.
- Unsafe WJ paths produce hard errors/progress failures, not silent rewrites.
- Tests cover traversal and safe-path compatibility.

**Reviewers:**
- Spec Reviewer B: checks all three path-safety findings are fixed.
- Quality/Security Reviewer B: adversarial path traversal review.

---

# Workstream C — Wabbajack Fail-Closed Semantics

## Agent C: WJ Fail-Closed Code Agent

**Objective:** Prevent Wabbajack installs from completing when required archives/directives failed.

**Files:**
- Modify: `src-tauri/src/wabbajack_installer.rs`
- Modify: `src-tauri/src/wabbajack_directives.rs`
- Possibly modify: `src-tauri/src/wabbajack_types.rs`

**Required fixes:**
1. `wabbajack_installer.rs:1591-1622`
   - Partial archive failures must fail install unless archive/directive is explicitly optional/ignored by WJ semantics.
2. `wabbajack_directives.rs:199-390`
   - Missing required archive hashes must be errors, not warning-only skips.
   - `process_all()` must return `Err` or a result that the installer treats as fatal when required directives fail.
3. `wabbajack_installer.rs:1730-1736`, `1904`
   - Directive errors must prevent `completed` DB status.
   - DB status should be `failed` with summarized root causes.

**Tests to add:**
- One missing required archive among many causes `install_wabbajack_modlist()` to fail.
- Directive referencing missing archive returns fatal error.
- Hash mismatch remains warning only only if existing intended behavior truly writes usable file; otherwise fatal.
- DB status becomes `failed`, not `completed`, on directive failure.

**Acceptance:**
- No path where required archive/directive failure produces completed install.
- Progress event reports actionable failure summary.
- Resume/checkpoint behavior still works for successful archives.

**Reviewers:**
- Spec Reviewer C: verifies fail-closed acceptance cases.
- Quality Reviewer C: checks no over-broad failure breaks explicitly ignored directives.

---

# Workstream D — Collection/FOMOD/Patch/Deploy Fail-Closed Semantics

## Agent D: Collection Code Agent

**Objective:** Fix false-success and unsafe fallback states in Nexus collection install pipeline.

**Files:**
- Modify: `src-tauri/src/collection_installer.rs`
- Possibly modify: `src-tauri/src/fomod.rs`, `src-tauri/src/fomod_recipes.rs`

**Required fixes:**
1. `collection_installer.rs:829-854`
   - If bundle fetch fails for slug+revision, fail install unless user explicitly selected an override mode.
2. `collection_installer.rs:4044-4200`
   - If FOMOD detected but no valid selection/file map exists, fail that mod.
   - Remove deploy-all fallback for detected FOMODs.
3. `collection_installer.rs:4221-4303`
   - BSDiff patch apply/decode/write failure should fail mod install unless patch marked optional.
4. `collection_installer.rs:4634-4651`
   - Zero-file deploy must fail the mod/install. Keep staging for recovery if useful, but status cannot be installed/success.
5. `collection_installer.rs:1792-1923`
   - Retry-pass extraction failure must create an install failure for that mod, not only signal extraction_done.

**Tests to add:**
- Bundle fetch failure returns error for slug+revision manifest.
- FOMOD detected + invalid choices + no valid defaults returns failure, not all files deployed.
- Patch failure returns mod install failure.
- Zero-file deploy error propagates failure and removes/marks DB mod appropriately.
- Retry-pass extraction failure prevents successful install detail.

**Acceptance:**
- No known collection pipeline error is warning-only if it affects required output files.
- User-facing details identify which mod failed and why.
- DB does not contain installed mods with zero deployed files.

**Reviewers:**
- Spec Reviewer D: maps every audit finding to a code/test change.
- Quality/Security Reviewer D: checks FOMOD/patch behavior for false positives and rollback consistency.

---

# Workstream E — Skyrim Plugin Disabled-State Semantics

## Agent E: Plugin State Code Agent

**Objective:** Preserve explicit collection/modlist disabled plugin state while still enforcing engine-required implicit plugins.

**Files:**
- Modify: `src-tauri/src/plugins/skyrim_plugins.rs`
- Modify: `src-tauri/src/collection_installer.rs` around `apply_collection_plugin_order`
- Possibly modify frontend if UI assumes all on-disk plugins enabled

**Required fixes:**
1. Keep implicit masters enabled (`Skyrim.esm`, `Update.esm`, DLC masters, ESL engine-required behavior as appropriate).
2. Preserve collection-authored disabled `.esp` plugin entries even if file exists.
3. Do not force all on-disk plugins enabled during `sync_plugins()` when explicit desired state exists.
4. Add clear distinction:
   - discovered new plugin default state
   - existing user state
   - collection manifest state
   - implicit engine-required state

**Tests to add:**
- `sync_plugins()` preserves disabled `.esp` from existing `plugins.txt`.
- `sync_plugins()` still forces implicit masters enabled.
- `apply_collection_plugin_order()` keeps manifest-disabled optional patch disabled even when file exists.
- New discovered plugin default behavior remains intentional and documented.

**Acceptance:**
- Modlist-authored disabled plugins remain disabled.
- Engine-required masters still cannot be disabled.
- Toggle/reorder behavior still round-trips.

**Reviewers:**
- Spec Reviewer E: checks disabled-state matrix.
- Quality Reviewer E: checks compatibility with current UI/load-order commands.

---

# Workstream F — Integration + End-to-End Regression

## Agent F: Integration Code Agent

**Objective:** Merge reviewed branches and prove the pipeline is fail-closed and still supports valid installs.

**Files:**
- Integration branch only: `fix/skyrim-pipeline-integration`
- Add integration tests if missing under `src-tauri` module tests or `e2e-wdio/specs/game-harness/`

**Tasks:**
1. Merge reviewed workstreams in order.
2. Resolve conflicts by preserving stricter safety behavior.
3. Run focused Rust tests:
   ```bash
   cd src-tauri
   cargo test wabbajack
   cargo test collection
   cargo test fomod
   cargo test skyrim
   cargo test loot
   cargo test deploy
   ```
4. Run full Rust tests:
   ```bash
   cargo test
   ```
5. Run frontend typecheck:
   ```bash
   cd ..
   npx svelte-check --threshold error
   ```
6. Add one high-level regression test or harness scenario for:
   - bad WJ path fails
   - partial missing WJ archive fails
   - collection FOMOD invalid choices fails
   - disabled plugin remains disabled

**Acceptance:**
- All tests pass.
- Diff has no remaining false-success patterns for audited issues.
- Integration reviewer approves.

**Reviewers:**
- Integration Reviewer: fresh subagent reviews full merged diff + test outputs.
- Final Deep Reviewer: Opus/Qwen-class final gate.

---

## Reviewer Prompt Templates

### Code Agent Prompt Template

```text
You are Code Agent <X> for Corkscrew.
Repo/worktree: <path>
Branch: <branch>

Implement ONLY Workstream <X> from docs/plans/2026-06-08-skyrim-pipeline-multi-agent-fix-plan.md.

Rules:
- Follow TDD where practical: add failing regression tests first, then fix.
- Do not touch unrelated workstreams.
- Fail closed for security/correctness problems.
- Run targeted tests and self-review before reporting.
- Return: changed files, tests run, self-review checklist, remaining risks.
```

### Spec Reviewer Prompt Template

```text
You are the independent spec reviewer for Workstream <X>.
Review ONLY whether the implementation satisfies the plan acceptance criteria.
Do not nitpick style.
Input: task spec, changed file list, diff, test output.
Return JSON with verdict PASS or REQUEST_CHANGES.
```

### Quality/Security Reviewer Prompt Template

```text
You are the independent quality/security reviewer for Workstream <X>.
Review the implementation for security, false-success states, DB consistency, error propagation, test strength, and Corkscrew conventions.
Return JSON: verdict, critical, major, minor, tests_to_add.
Any critical/major blocks merge.
```

### Integration Reviewer Prompt Template

```text
You are the integration reviewer for all Skyrim pipeline fixes.
Review the merged diff against the original audit findings and this plan.
Check for cross-workstream regressions and inconsistent semantics.
Verdict must be APPROVED or REQUEST_CHANGES.
```

---

## Done Criteria

Do not report this project as done until all are true:

- [ ] Tooling gate is green or documented as an external blocker.
- [ ] Workstream B path safety approved by spec + quality reviewers.
- [ ] Workstream C Wabbajack fail-closed approved by spec + quality reviewers.
- [ ] Workstream D collection fail-closed approved by spec + quality reviewers.
- [ ] Workstream E plugin disabled-state approved by spec + quality reviewers.
- [ ] Integration branch contains all approved workstreams.
- [ ] Full available test suite passes.
- [ ] Final deep reviewer gives ship verdict.
- [ ] Final report includes exact commits/branches, tests, and any known limitations.
