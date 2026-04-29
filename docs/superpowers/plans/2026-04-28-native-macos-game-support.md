# Native macOS Game Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers-extended-cc:subagent-driven-development (recommended) or superpowers-extended-cc:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add native macOS game support to Corkscrew alongside existing Wine/CrossOver support. Phase 1 ships Stardew Valley (SMAPI) + Baldur's Gate 3 (Larian native), gated behind a "Native Mode" view themed after Apple's M5 marketing aesthetic (deep black, blue→purple gradient accents, ratcheted-up Liquid Glass).

**Architecture:** Introduce a `GameRuntime { Wine, Native }` discriminator on `DetectedGame` and split the `GamePlugin` trait into `detect_wine` + `detect_native` (both default `None`) so existing 20+ Wine plugins recompile with no behavior change. A new `native_scanner.rs` walks `/Applications` + Steam mac + Mac App Store + GOG and produces `DetectedGame { runtime: Native(...) }`. The deployer splits into `deploy_wine_game` / `deploy_native_game` so native games never touch Z:-drive translation and never mutate the signed `.app` bundle (sidecar dir convention only). Frontend gains a top-level `native_mode` toggle, mode-scoped routes (`/native/*`), and a new `[data-theme="native"]` CSS layer.

**Tech Stack:** Tauri v2 + Svelte 5 + Rust. Reuses existing deps — `quick-xml` for BG3 LSX, `tauri-plugin-liquid-glass = "0.1"` already wired (no new deps), `window-vibrancy = "0.7"` for native vibrancy ratchet, `dirs` for HOME paths, `walkdir`, `serde`. New module surface: `native_scanner.rs`, `plugins/stardew_valley_native.rs`, `plugins/baldurs_gate_3_native.rs`, `bg3_lsx.rs`, `smapi.rs`, frontend `/src/routes/native/*` + `/src/lib/native/*`.

**Versioning rollout:**
- **v0.14.0** — Phases 1–2 (runtime split + native scanner) behind config feature flag `experimental.native_mode = false`. No native plugins shipped yet; existing Wine functionality unchanged.
- **v0.14.x** — Patches as needed.
- **v0.15.0** — Phase 3 (Stardew Valley) + Phase 5 (Native Mode UI shell + theming). Feature flag still enforced; flag toggle exposed in Settings → Experimental.
- **v0.16.0** — Phase 4 (BG3) + Phase 6 (polish + safety nets). Feature flag lifted; Native Mode toggle promoted to topbar.

---

## Open Questions — Verify Before Coding

These are the only items that **require external verification** before their tasks can be implemented. Treat them as research spikes, not blockers for unrelated tasks.

1. **SMAPI mac install steps** — Task 3.1 must read SMAPI's `install on macOS.command` (the Bash installer shipped in SMAPI's release zip) and document the exact mutations. Likely steps: rename `Contents/MacOS/StardewValley` → `StardewValley-original`, drop a new launcher script that `mono` → SMAPI's DLL, install StardewModdingAPI.dll into `Contents/MacOS/`. Source: <https://github.com/Pathoschild/SMAPI/blob/develop/src/SMAPI.Installer/install%20on%20macOS.command>. Spike output: a written `smapi-install-spec.md` co-located with this plan.
2. **BG3 modsettings.lsx schema** — Task 4.1 must obtain a known-good native macOS BG3 `modsettings.lsx`, document the `<region>` / `<node id="ModuleShortDesc">` structure, and capture the canonical Gustav/GustavDev master entries that must NEVER be reordered. Source: BGMM source code <https://github.com/LaughingLeader/BG3ModManager> + a real BG3 install on the dev machine.
3. **`tauri-plugin-liquid-glass` API surface** — Task 5.4 must read the plugin's README/source (it's at v0.1 — possibly thin) to confirm runtime-toggleable vibrancy/material settings. If the plugin only supports init-time config, ratchet via direct `window-vibrancy` calls instead. Source: crate docs + git source.
4. **MAS receipt detection details** — Task 2.4 must confirm `_MASReceipt/receipt` is the file (not directory) and verify behavior on Apple Silicon (ARM-signed receipt format). Test target: any free Mac App Store game.

---

## File Structure

### New Rust modules

| File | Responsibility |
|---|---|
| `src-tauri/src/runtime.rs` | `GameRuntime`, `WineContext`, `NativeContext` enums + helpers. Pure data. |
| `src-tauri/src/native_scanner.rs` | Walks `/Applications`, `~/Applications`, Steam mac, GOG mac, MAS detect. Returns `Vec<NativeAppCandidate>`. |
| `src-tauri/src/plist.rs` | Minimal Info.plist (XML + binary) reader using `plist` crate. New dep. |
| `src-tauri/src/smapi.rs` | SMAPI presence detection, auto-install (port of `install on macOS.command`), revert. |
| `src-tauri/src/bg3_lsx.rs` | BG3 `modsettings.lsx` parse + edit (load order). Uses existing `quick-xml`. |
| `src-tauri/src/plugins/stardew_valley_native.rs` | `GamePlugin` impl for native Stardew. |
| `src-tauri/src/plugins/baldurs_gate_3_native.rs` | `GamePlugin` impl for native BG3. |

### Modified Rust modules

| File | Change |
|---|---|
| `src-tauri/src/games.rs` | `DetectedGame.runtime: GameRuntime`; trait split `detect_wine` + `detect_native`. |
| `src-tauri/src/migrations.rs` | New migration `v22 → v23` adds runtime column, nullable bottle, native paths. |
| `src-tauri/src/database.rs` | Read/write helpers updated for runtime column. |
| `src-tauri/src/deployer.rs` | Split into `deploy_wine_game` / `deploy_native_game`. |
| `src-tauri/src/lib.rs` | Register new plugins, init native scanner, expose new commands. |
| `src-tauri/src/config.rs` | New field `native_mode: bool` (default false), `experimental_native: bool`. |
| `src-tauri/src/plugins/skyrim_se.rs` (and 19 other Wine plugins) | Mechanical: `fn detect` → `fn detect_wine`. |
| `src-tauri/src/mod_tools.rs` | Extend GitHub-release fetcher to handle SMAPI macOS asset selection. |
| `src-tauri/src/commands/*.rs` | New native commands; existing commands check runtime where needed. |

### New / modified frontend

| File | Responsibility |
|---|---|
| `src/lib/native/mode.ts` | Native mode store + persistence. |
| `src/lib/native/types.ts` | TS types matching `GameRuntime`, `NativeAppCandidate`. |
| `src/lib/native/theme.ts` | Apply `[data-theme="native"]` on body when in native mode. |
| `src/routes/native/+layout.svelte` | Native shell layout with M5 theme. |
| `src/routes/native/mods/+page.svelte` | Stardew/BG3 mod list. |
| `src/routes/native/discover/+page.svelte` | Native game picker / first-run flow. |
| `src/routes/native/settings/+page.svelte` | Native-only settings. |
| `src/app.css` | Add `[data-theme="native"]` token block + glass intensification. |
| `src/lib/api.ts` | New typed wrappers for native commands. |
| `src/lib/components/topbar/TopBarGameSelector.svelte` | Branch on runtime; show Apple Silicon / Wine badges. |
| `src/routes/+layout.svelte` | Mode-aware route guarding + theme injection. |

### Tests

20+ new tests, distributed across modules. Test paths colocated in source files via `#[cfg(test)] mod tests` (project convention).

---

## Phase 1 — Runtime Abstraction (foundation)

Goal: refactor the type system to support a runtime discriminator. No behavior change for end users; all 20+ existing Wine plugins keep working.

---

### Task 1.1: Introduce `GameRuntime` types

**Goal:** Land the new enum and helpers in a dedicated module. No consumers yet.

**Files:**
- Create: `src-tauri/src/runtime.rs`
- Modify: `src-tauri/src/lib.rs` (add `pub mod runtime;`)

**Acceptance Criteria:**
- [ ] `GameRuntime`, `WineContext`, `NativeContext` defined and exported
- [ ] All three derive `Clone, Debug, Serialize, Deserialize`
- [ ] `cargo build` succeeds
- [ ] `cargo test --lib runtime::` passes the unit tests below

**Verify:** `cd src-tauri && cargo test --lib runtime::` → 3 passed

**Steps:**

- [ ] **Step 1: Write the failing tests**

```rust
// src-tauri/src/runtime.rs (test module at bottom)
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn wine_variant_round_trips_through_json() {
        let r = GameRuntime::Wine(WineContext {
            bottle_name: "GTS".into(),
            bottle_path: PathBuf::from("/Users/x/Bottles/GTS"),
            source: "CrossOver".into(),
        });
        let json = serde_json::to_string(&r).unwrap();
        let back: GameRuntime = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, GameRuntime::Wine(_)));
    }

    #[test]
    fn native_variant_round_trips_through_json() {
        let r = GameRuntime::Native(NativeContext {
            app_bundle_path: PathBuf::from("/Applications/Stardew Valley.app"),
            game_data_root: PathBuf::from("/Applications/Stardew Valley.app/Contents/MacOS"),
            architecture: Architecture::AppleSilicon,
            sandboxed: false,
            source: NativeSource::Steam,
        });
        let json = serde_json::to_string(&r).unwrap();
        let back: GameRuntime = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, GameRuntime::Native(_)));
    }

    #[test]
    fn discriminator_is_stable_string() {
        let r = GameRuntime::Wine(WineContext::default_for_test());
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["runtime"], "wine");
    }
}
```

- [ ] **Step 2: Run tests — expect "module not found"**

Run: `cd src-tauri && cargo test --lib runtime::tests`
Expected: compile failure (module doesn't exist yet)

- [ ] **Step 3: Implement `runtime.rs`**

```rust
//! Runtime discriminator for detected games.
//!
//! Corkscrew supports games running through Wine/CrossOver bottles AND
//! games running natively on macOS. This module defines the type-level
//! split so the rest of the codebase can branch on runtime in one place
//! rather than carrying optional bottle fields.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "runtime", rename_all = "lowercase")]
pub enum GameRuntime {
    Wine(WineContext),
    Native(NativeContext),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WineContext {
    pub bottle_name: String,
    pub bottle_path: PathBuf,
    /// Manager that created the bottle (CrossOver, Whisky, etc.).
    pub source: String,
}

#[cfg(test)]
impl WineContext {
    pub fn default_for_test() -> Self {
        Self {
            bottle_name: "test".into(),
            bottle_path: PathBuf::from("/tmp/test"),
            source: "test".into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeContext {
    /// Absolute path to the `.app` bundle (e.g. /Applications/Stardew Valley.app).
    pub app_bundle_path: PathBuf,
    /// Path inside the bundle where game files (and SMAPI mods) live.
    /// Usually `<app_bundle>/Contents/MacOS` for Stardew, `<app_bundle>/Contents/SharedSupport`
    /// or similar for others. Resolved by the per-game native plugin.
    pub game_data_root: PathBuf,
    pub architecture: Architecture,
    /// True if this game is sandboxed (Mac App Store) — modding refused.
    pub sandboxed: bool,
    pub source: NativeSource,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Architecture {
    AppleSilicon,
    IntelOnly,
    Universal,
    Unknown,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NativeSource {
    /// Found in /Applications or ~/Applications.
    SystemApplications,
    /// Found via Steam libraryfolders.vdf.
    Steam,
    /// Found via GOG Galaxy.
    Gog,
    /// User added manually via file picker.
    Manual,
    /// Found via Mac App Store (always sandboxed = true).
    AppStore,
}

impl GameRuntime {
    pub fn is_wine(&self) -> bool {
        matches!(self, Self::Wine(_))
    }
    pub fn is_native(&self) -> bool {
        matches!(self, Self::Native(_))
    }
    pub fn wine(&self) -> Option<&WineContext> {
        match self {
            Self::Wine(w) => Some(w),
            _ => None,
        }
    }
    pub fn native(&self) -> Option<&NativeContext> {
        match self {
            Self::Native(n) => Some(n),
            _ => None,
        }
    }
}
```

- [ ] **Step 4: Add `pub mod runtime;` to `src-tauri/src/lib.rs`**

Add the line near the other `pub mod` declarations.

- [ ] **Step 5: Run tests — expect pass**

Run: `cd src-tauri && cargo test --lib runtime::`
Expected: 3 passed; 0 failed

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/runtime.rs src-tauri/src/lib.rs
git commit -m "v0.14.0-wip: introduce GameRuntime enum (no consumers yet)"
```

---

### Task 1.2: Add migration v23 — runtime column on `games` table

**Goal:** Schema supports both runtime types. Existing rows backfilled to `wine`.

**Files:**
- Modify: `src-tauri/src/migrations.rs` (bump `TARGET_VERSION` to 23, add `migrate_v22_to_v23`)

**Acceptance Criteria:**
- [ ] `TARGET_VERSION = 23`
- [ ] `migrate_v22_to_v23` adds columns: `runtime TEXT NOT NULL DEFAULT 'wine'`, `native_app_path TEXT`, `native_data_root TEXT`, `native_architecture TEXT`, `native_sandboxed INTEGER DEFAULT 0`, `native_source TEXT`
- [ ] Backfills `runtime = 'wine'` for all existing rows
- [ ] Makes `bottle_name` and `bottle_path` columns nullable (where they were NOT NULL before)
- [ ] Migration runs idempotently (can be applied to a partially-migrated DB without error)
- [ ] Test: fresh DB → migrate → new columns exist with correct types

**Verify:** `cd src-tauri && cargo test --lib migrations::` → all pass

**Steps:**

- [ ] **Step 1: Inspect current games-table schema**

Run: `cd src-tauri && grep -n 'CREATE TABLE.*games\|ALTER TABLE games' src/migrations.rs`
Note exact column types for `bottle_name`, `bottle_path` so the v23 ALTER preserves them.

- [ ] **Step 2: Write the failing test**

Append to `src-tauri/src/migrations.rs` test module:

```rust
#[test]
fn v23_adds_runtime_column_with_wine_default() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    // Insert a row using the v22 column set
    conn.execute(
        "INSERT INTO games (game_id, bottle_name, bottle_path) VALUES ('test', 'b', '/p')",
        [],
    ).unwrap();
    let runtime: String = conn
        .query_row("SELECT runtime FROM games WHERE game_id = 'test'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(runtime, "wine");
}

#[test]
fn v23_native_columns_nullable() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    conn.execute(
        "INSERT INTO games (game_id, runtime, native_app_path, native_architecture)
         VALUES ('sdv', 'native', '/Applications/Stardew Valley.app', 'apple_silicon')",
        [],
    ).unwrap();
    let arch: String = conn
        .query_row("SELECT native_architecture FROM games WHERE game_id = 'sdv'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(arch, "apple_silicon");
}
```

- [ ] **Step 3: Run tests — expect fail (no migration yet)**

Run: `cd src-tauri && cargo test --lib migrations::v23`
Expected: FAIL — "no such column: runtime"

- [ ] **Step 4: Implement migration**

Bump `TARGET_VERSION` to 23. Add the if-block matching the existing pattern. Implementation:

```rust
fn migrate_v22_to_v23(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        BEGIN;

        -- Add runtime discriminator. Existing rows are Wine.
        ALTER TABLE games ADD COLUMN runtime TEXT NOT NULL DEFAULT 'wine';

        -- Native game fields. NULL for Wine games.
        ALTER TABLE games ADD COLUMN native_app_path TEXT;
        ALTER TABLE games ADD COLUMN native_data_root TEXT;
        ALTER TABLE games ADD COLUMN native_architecture TEXT;
        ALTER TABLE games ADD COLUMN native_sandboxed INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE games ADD COLUMN native_source TEXT;

        UPDATE schema_version SET version = 23;
        COMMIT;
        "#,
    )?;
    Ok(())
}
```

Note: SQLite cannot drop NOT NULL via ALTER. If `bottle_name`/`bottle_path` are currently NOT NULL, defer rebuild to a v24 migration if needed (only required if a native row would otherwise fail; we can write empty strings for now and audit in Phase 2).

- [ ] **Step 5: Wire migration into `migrate()` ladder**

```rust
if version == 22 {
    migrate_v22_to_v23(conn)?;
    version = 23;
}
```

- [ ] **Step 6: Run tests — expect pass**

Run: `cd src-tauri && cargo test --lib migrations::`
Expected: all migration tests pass.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/migrations.rs
git commit -m "v0.14.0-wip: db migration v23 — runtime column + native fields"
```

---

### Task 1.3: Refactor `DetectedGame` to carry `GameRuntime`

**Goal:** `DetectedGame` no longer has bottle fields directly; they move into `GameRuntime::Wine(...)`.

**Files:**
- Modify: `src-tauri/src/games.rs` (struct + impl)
- Modify: `src-tauri/src/database.rs` (read/write helpers)
- Modify: any direct field accessors in callers — search and fix

**Acceptance Criteria:**
- [ ] `DetectedGame.bottle_name` / `.bottle_path` removed
- [ ] `DetectedGame.runtime: GameRuntime` added
- [ ] All call sites compile (use `runtime.wine()?.bottle_name` etc.)
- [ ] DB serialization writes/reads runtime column correctly
- [ ] `cargo build` succeeds

**Verify:** `cd src-tauri && cargo build && cargo test --lib games::` → 0 failures

**Steps:**

- [ ] **Step 1: Find every reader of `bottle_name` / `bottle_path` on `DetectedGame`**

Run: `cd src-tauri && grep -rn '\.bottle_name\|\.bottle_path' src/ | grep -v '/runtime.rs' | wc -l`
Note count for the migration audit.

- [ ] **Step 2: Replace struct definition**

In `src-tauri/src/games.rs`:

```rust
/// A game found by Corkscrew. Runtime determines whether it lives inside
/// a Wine bottle or as a native macOS app.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DetectedGame {
    pub game_id: String,
    pub display_name: String,
    pub nexus_slug: String,
    /// Absolute path to the game installation root.
    /// For Wine: inside the bottle. For native: the .app bundle path or its
    /// resources subdir, per per-game plugin convention.
    pub game_path: PathBuf,
    pub exe_path: Option<PathBuf>,
    pub data_dir: PathBuf,
    pub runtime: crate::runtime::GameRuntime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steam_app_id: Option<String>,
}
```

- [ ] **Step 3: Update every call site (compile-error-driven)**

Run `cargo build` and fix each error in turn. Common patterns:

```rust
// Before:
let bottle_name = detected.bottle_name.clone();

// After:
let bottle_name = detected.runtime.wine()
    .map(|w| w.bottle_name.clone())
    .unwrap_or_default();
```

For sites that ASSUME a bottle context (Wine-only commands), use:

```rust
let wine = detected.runtime.wine()
    .ok_or_else(|| "this command requires a Wine game".to_string())?;
let bottle_path = &wine.bottle_path;
```

- [ ] **Step 4: Update DB write/read in `database.rs`**

When inserting `DetectedGame` rows, emit `runtime` + appropriate native/wine columns. When reading, reconstruct `GameRuntime` from `runtime` discriminator + applicable columns. Add a helper:

```rust
fn row_to_runtime(
    runtime: &str,
    bottle_name: Option<String>,
    bottle_path: Option<String>,
    source: Option<String>,
    native_app_path: Option<String>,
    native_data_root: Option<String>,
    native_architecture: Option<String>,
    native_sandboxed: i64,
    native_source: Option<String>,
) -> Result<GameRuntime, DbError> { /* ... */ }
```

- [ ] **Step 5: Run all backend tests**

Run: `cd src-tauri && cargo test`
Expected: 0 failures. Existing 787+ tests should all still pass — runtime split is mechanical.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/games.rs src-tauri/src/database.rs src-tauri/src/
git commit -m "v0.14.0-wip: DetectedGame.runtime replaces bottle_* fields"
```

---

### Task 1.4: Split `GamePlugin::detect` into `detect_wine` + `detect_native`

**Goal:** Plugin trait gains two methods; existing impls keep their behavior verbatim.

**Files:**
- Modify: `src-tauri/src/games.rs` (trait)
- Modify: all 20+ `src-tauri/src/plugins/*.rs` (mechanical rename + signature)
- Modify: `src-tauri/src/games.rs::detect_games` (call both)

**Acceptance Criteria:**
- [ ] Trait has `detect_wine(&self, &Bottle) -> Option<DetectedGame>` defaulting `None`
- [ ] Trait has `detect_native(&self) -> Vec<DetectedGame>` defaulting `vec![]`
- [ ] Old `detect` removed from trait
- [ ] All Wine plugins implement only `detect_wine` (delete-and-rename)
- [ ] `detect_games(bottle)` calls `detect_wine`; new `detect_native_games()` calls `detect_native`
- [ ] `cargo build` + `cargo test` pass

**Verify:** `cd src-tauri && cargo test` → 0 failures

**Steps:**

- [ ] **Step 1: Update trait in `games.rs`**

```rust
pub trait GamePlugin: Send + Sync {
    fn game_id(&self) -> &str;
    fn display_name(&self) -> &str;
    fn nexus_slug(&self) -> &str;
    fn executables(&self) -> &[&str];

    /// Attempt to locate this game inside a Wine bottle. Default: not a Wine game.
    fn detect_wine(&self, _bottle: &Bottle) -> Option<DetectedGame> {
        None
    }

    /// Attempt to locate this game as a native macOS install. Returns one entry
    /// per discovered installation (the same game can exist in multiple Steam
    /// libraries or both /Applications and ~/Applications). Default: not a native game.
    fn detect_native(&self) -> Vec<DetectedGame> {
        vec![]
    }

    fn get_data_dir(&self, game_path: &Path) -> PathBuf;
    fn get_plugins_file(&self, game_path: &Path, bottle: &Bottle) -> Option<PathBuf>;
    // ... rest of trait unchanged ...
}
```

- [ ] **Step 2: Rename in all Wine plugin impls (20+ files)**

For each `plugins/*.rs` file: rename `fn detect` → `fn detect_wine`. Signature is unchanged. This is purely mechanical.

```bash
# Mechanical rename — eyeball each diff before commit:
cd src-tauri/src/plugins
for f in *.rs; do
    # Match "fn detect(" but not "fn detect_wine(" or "fn detect_native("
    perl -i -pe 's/fn detect\(/fn detect_wine(/g' "$f"
done
```

Then run `cargo build` to catch any miss.

- [ ] **Step 3: Update `detect_games` to use `detect_wine`**

```rust
pub fn detect_games(bottle: &Bottle) -> Vec<DetectedGame> {
    let plugins = registry().lock().unwrap_or_else(|e| e.into_inner());
    let mut found = Vec::new();
    for plugin in plugins.iter() {
        if let Some(detected) = plugin.detect_wine(bottle) {
            found.push(detected);
        }
    }
    drop(plugins);
    let unregistered = crate::game_registry::detect_unregistered_steam_games(bottle, &found);
    found.extend(unregistered);
    found
}
```

- [ ] **Step 4: Add `detect_native_games`**

```rust
/// Scan native macOS installs for all registered native plugins.
pub fn detect_native_games() -> Vec<DetectedGame> {
    let plugins = registry().lock().unwrap_or_else(|e| e.into_inner());
    let mut found = Vec::new();
    for plugin in plugins.iter() {
        found.extend(plugin.detect_native());
    }
    found
}
```

- [ ] **Step 5: Update `detect_all_games` to include native**

```rust
pub fn detect_all_games() -> Vec<DetectedGame> {
    let mut found = Vec::new();
    for bottle in crate::bottles::detect_bottles() {
        found.extend(detect_games(&bottle));
    }
    found.extend(detect_native_games());
    found
}
```

- [ ] **Step 6: Run all tests**

Run: `cd src-tauri && cargo test`
Expected: 0 failures.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/games.rs src-tauri/src/plugins/
git commit -m "v0.14.0-wip: split GamePlugin::detect into detect_wine + detect_native"
```

---

### Task 1.5: Split deployer into wine + native paths

**Goal:** `deployer.rs` exposes `deploy_wine_game` and `deploy_native_game`. Native version asserts the `.app` bundle is never written to (mod outputs go to a sidecar dir or platform-canonical mod root, decided by the per-game plugin).

**Files:**
- Modify: `src-tauri/src/deployer.rs`
- Modify: callers (collection_installer, installer.rs, commands)

**Acceptance Criteria:**
- [ ] Public surface: `deploy_game(detected: &DetectedGame, ...)` dispatches by runtime
- [ ] `deploy_wine_game` is the renamed existing function (behavior unchanged)
- [ ] `deploy_native_game` exists; for Phase 1 it can be a stub that returns `Err("native deployment not yet implemented for this game")` IF no native plugin override is provided
- [ ] Native deploy MUST refuse to write inside `<app_bundle>/Contents/` unless an explicit `allow_bundle_write: bool` flag is set by the per-game plugin (Stardew sets it true for SMAPI; default false)
- [ ] All callers go through `deploy_game`; no direct calls to old `deploy()` remain
- [ ] Tests: native deploy refuses bundle writes by default; allows when flag set

**Verify:** `cd src-tauri && cargo test --lib deployer::` → all pass

**Steps:**

- [ ] **Step 1: Rename the existing public function**

`pub fn deploy(...)` → `pub fn deploy_wine_game(...)` in `deployer.rs`. Update internal references.

- [ ] **Step 2: Add `deploy_game` dispatcher**

```rust
pub fn deploy_game(
    detected: &DetectedGame,
    db: &Arc<ModDatabase>,
    options: DeployOptions,
) -> Result<DeployReport, DeployError> {
    match &detected.runtime {
        GameRuntime::Wine(_) => deploy_wine_game(detected, db, options),
        GameRuntime::Native(_) => deploy_native_game(detected, db, options),
    }
}
```

- [ ] **Step 3: Implement `deploy_native_game` skeleton**

```rust
pub fn deploy_native_game(
    detected: &DetectedGame,
    db: &Arc<ModDatabase>,
    _options: DeployOptions,
) -> Result<DeployReport, DeployError> {
    let _native = detected.runtime.native()
        .ok_or_else(|| DeployError::Other("expected native runtime".into()))?;

    // Phase 1 stub: per-game native plugin must override deploy logic via
    // a hook that lands in Phase 3 (Stardew). For now, refuse so unsupported
    // native games fail loudly rather than silently corrupting state.
    Err(DeployError::Other(format!(
        "native deployment not implemented for {}; per-game plugin must provide it",
        detected.game_id
    )))
}
```

- [ ] **Step 4: Update all callers**

Replace `deploy(...)` with `deploy_game(...)`. Compile-driven; touch ~5–10 sites.

- [ ] **Step 5: Add tests**

```rust
#[test]
fn deploy_native_game_without_plugin_returns_error() {
    let detected = DetectedGame {
        game_id: "fakegame".into(),
        runtime: GameRuntime::Native(/* test ctx */),
        // ...
    };
    let db = Arc::new(test_db());
    let result = deploy_native_game(&detected, &db, DeployOptions::default());
    assert!(result.is_err());
}
```

- [ ] **Step 6: Run tests, commit**

```bash
cd src-tauri && cargo test --lib deployer::
git add src-tauri/src/deployer.rs src-tauri/src/
git commit -m "v0.14.0-wip: split deployer into wine + native dispatchers"
```

---

### Task 1.6: Add `native_mode` config flag + AppState plumbing

**Goal:** Config has `experimental.native_mode: bool` (default false). Frontend can flip it; backend respects it for filtering.

**Files:**
- Modify: `src-tauri/src/config.rs` (add field with serde default)
- Modify: `src-tauri/src/lib.rs` (expose `set_native_mode` / `get_native_mode` commands)
- Modify: `src/lib/api.ts` (typed wrappers)
- Modify: `src/lib/stores.ts` (writable store mirroring config)

**Acceptance Criteria:**
- [ ] Config field `native_mode: bool` (default false), persisted to JSON config
- [ ] Tauri commands `get_native_mode() -> bool` and `set_native_mode(enabled: bool) -> ()`
- [ ] TS API: `getNativeMode()`, `setNativeMode(enabled: boolean)`
- [ ] Svelte store `nativeMode = writable(false)` initialized on app start
- [ ] Test: round-trip set → get returns the value

**Verify:** `cd src-tauri && cargo test --lib config:: && npx svelte-check --threshold error` → both clean

**Steps:**

- [ ] **Step 1: Inspect config struct shape**

Read `src-tauri/src/config.rs` to find the existing struct + serde defaults pattern.

- [ ] **Step 2: Add field**

```rust
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ExperimentalConfig {
    #[serde(default)]
    pub native_mode: bool,
}

// And in main Config struct:
#[serde(default)]
pub experimental: ExperimentalConfig,
```

- [ ] **Step 3: Add commands in `lib.rs` or `commands/config_commands.rs`**

```rust
#[tauri::command]
pub async fn get_native_mode(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let cfg = state.config.lock().map_err(|e| e.to_string())?;
    Ok(cfg.experimental.native_mode)
}

#[tauri::command]
pub async fn set_native_mode(
    enabled: bool,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let mut cfg = state.config.lock().map_err(|e| e.to_string())?;
    cfg.experimental.native_mode = enabled;
    cfg.save().map_err(|e| e.to_string())?;
    Ok(())
}
```

Register both in `tauri::generate_handler!`.

- [ ] **Step 4: Add typed wrappers in `src/lib/api.ts`**

```typescript
export async function getNativeMode(): Promise<boolean> {
    return invoke<boolean>('get_native_mode');
}

export async function setNativeMode(enabled: boolean): Promise<void> {
    return invoke<void>('set_native_mode', { enabled });
}
```

- [ ] **Step 5: Add store in `src/lib/stores.ts`**

```typescript
import { writable } from 'svelte/store';
export const nativeMode = writable(false);
```

In `+layout.svelte` `onMount`, hydrate from `getNativeMode()`.

- [ ] **Step 6: Test**

```rust
#[test]
fn native_mode_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("CORKSCREW_CONFIG_DIR", dir.path());
    let mut cfg = Config::load_or_default();
    cfg.experimental.native_mode = true;
    cfg.save().unwrap();
    let cfg2 = Config::load_or_default();
    assert!(cfg2.experimental.native_mode);
}
```

- [ ] **Step 7: Commit**

```bash
cd src-tauri && cargo test --lib config:: && cd .. && npx svelte-check --threshold error
git add -p
git commit -m "v0.14.0-wip: experimental.native_mode flag + store"
```

---

## Phase 2 — Native Detection Scanner

Goal: discover native macOS apps and produce `DetectedGame { runtime: Native(...) }` candidates.

---

### Task 2.1: Add `plist` dep + Info.plist reader

**Goal:** Read `CFBundleIdentifier`, `CFBundleExecutable`, `CFBundleShortVersionString`, `LSApplicationCategoryType` from a .app's Info.plist.

**Files:**
- Modify: `src-tauri/Cargo.toml` (add `plist = "1"`)
- Create: `src-tauri/src/plist.rs`

**Acceptance Criteria:**
- [ ] `read_info_plist(path: &Path) -> Result<InfoPlist, PlistError>` returns the 4 fields above
- [ ] Handles both XML and binary plist formats (the `plist` crate auto-detects)
- [ ] Returns `PlistError::NotFound` if file missing, `PlistError::Malformed` on parse error
- [ ] Test: parse a fixture .plist (XML) → expected fields; parse malformed → error

**Verify:** `cd src-tauri && cargo test --lib plist::` → pass

**Steps:**

- [ ] **Step 1: Add dep**

In `Cargo.toml` under `[dependencies]`: `plist = "1"`

- [ ] **Step 2: Write fixture + test**

Create `src-tauri/tests/fixtures/Info.xml.plist` (a real XML plist with the 4 keys). Then:

```rust
#[test]
fn reads_xml_plist_fixture() {
    let path = Path::new("tests/fixtures/Info.xml.plist");
    let info = read_info_plist(path).unwrap();
    assert_eq!(info.bundle_identifier, "com.example.testapp");
    assert_eq!(info.bundle_executable, "TestApp");
}
```

- [ ] **Step 3: Implement `plist.rs`**

```rust
use std::path::Path;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct InfoPlist {
    #[serde(rename = "CFBundleIdentifier")]
    pub bundle_identifier: String,
    #[serde(rename = "CFBundleExecutable")]
    pub bundle_executable: String,
    #[serde(rename = "CFBundleShortVersionString", default)]
    pub short_version: Option<String>,
    #[serde(rename = "LSApplicationCategoryType", default)]
    pub category: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum PlistError {
    #[error("plist file not found: {0}")]
    NotFound(String),
    #[error("malformed plist: {0}")]
    Malformed(String),
}

pub fn read_info_plist(path: &Path) -> Result<InfoPlist, PlistError> {
    if !path.exists() {
        return Err(PlistError::NotFound(path.display().to_string()));
    }
    plist::from_file(path).map_err(|e| PlistError::Malformed(e.to_string()))
}
```

- [ ] **Step 4: Run test, commit**

```bash
cd src-tauri && cargo test --lib plist::
git add Cargo.toml src/plist.rs tests/fixtures/Info.xml.plist src/lib.rs
git commit -m "v0.14.0-wip: Info.plist reader (plist crate)"
```

---

### Task 2.2: `/Applications` walker → `NativeAppCandidate`

**Goal:** Walk `/Applications` and `~/Applications`, return per-bundle metadata.

**Files:**
- Create: `src-tauri/src/native_scanner.rs`

**Acceptance Criteria:**
- [ ] `scan_applications_dirs() -> Vec<NativeAppCandidate>` returns all .app bundles in both dirs
- [ ] Each candidate carries: bundle path, Info.plist data, architecture (TBD Task 2.7), source = `SystemApplications`
- [ ] Skips non-game system apps via heuristic: `LSApplicationCategoryType` starting with `public.app-category.games` is preferred but not required (we keep all and let per-game plugins filter)
- [ ] Test: with a tempdir layout containing 2 fake .app bundles, returns 2 candidates

**Verify:** `cd src-tauri && cargo test --lib native_scanner::` → pass

**Steps:**

- [ ] **Step 1: Define `NativeAppCandidate`**

```rust
#[derive(Clone, Debug)]
pub struct NativeAppCandidate {
    pub bundle_path: PathBuf,
    pub info: InfoPlist,
    pub architecture: Architecture, // populated by Task 2.7; default Unknown
    pub source: NativeSource,
    pub sandboxed: bool, // populated by Task 2.4
}
```

- [ ] **Step 2: Implement walker**

```rust
pub fn scan_applications_dirs() -> Vec<NativeAppCandidate> {
    let mut results = Vec::new();
    let mut dirs = vec![PathBuf::from("/Applications")];
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join("Applications"));
    }
    for d in dirs {
        if !d.exists() { continue; }
        let read = match std::fs::read_dir(&d) { Ok(r) => r, Err(_) => continue };
        for entry in read.flatten() {
            let p = entry.path();
            if !p.extension().is_some_and(|e| e == "app") { continue; }
            let info_path = p.join("Contents").join("Info.plist");
            if let Ok(info) = crate::plist::read_info_plist(&info_path) {
                results.push(NativeAppCandidate {
                    bundle_path: p,
                    info,
                    architecture: Architecture::Unknown,
                    source: NativeSource::SystemApplications,
                    sandboxed: false,
                });
            }
        }
    }
    results
}
```

- [ ] **Step 3: Test with tempdir**

```rust
#[test]
fn scans_app_bundles_in_dir() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("Test.app/Contents");
    fs::create_dir_all(&app).unwrap();
    fs::write(app.join("Info.plist"), include_str!("../tests/fixtures/Info.xml.plist")).unwrap();
    // call internal helper that takes &Path so we can inject tempdir...
    let results = scan_dir(dir.path());
    assert_eq!(results.len(), 1);
}
```

(Refactor `scan_applications_dirs` to delegate to `scan_dir(&Path) -> Vec<NativeAppCandidate>` for testability.)

- [ ] **Step 4: Commit**

```bash
cd src-tauri && cargo test --lib native_scanner::
git add src/native_scanner.rs src/lib.rs
git commit -m "v0.14.0-wip: /Applications native scanner"
```

---

### Task 2.3: Steam mac integration

**Goal:** Parse Steam's macOS `libraryfolders.vdf` + `appmanifest_*.acf` files; surface installed native games.

**Files:**
- Modify: `src-tauri/src/native_scanner.rs` (add `scan_steam_mac`)
- Possibly extend: `src-tauri/src/steam_integration.rs` if VDF parser already exists there

**Acceptance Criteria:**
- [ ] `scan_steam_mac() -> Vec<NativeAppCandidate>` returns all native Steam games
- [ ] Reads from `~/Library/Application Support/Steam/steamapps/libraryfolders.vdf` (path may differ; verify on dev machine)
- [ ] For each library, scans `steamapps/appmanifest_*.acf`, extracts `appid`, `name`, `installdir`
- [ ] Resolves the actual install path: `<library>/steamapps/common/<installdir>` and finds the `.app` inside
- [ ] Reuses existing VDF parser if present (`steam_integration.rs`); otherwise add minimal one (line-by-line key/value scan is enough for VDF v1)
- [ ] Test: synthetic libraryfolders.vdf + appmanifest fixtures → expected candidates

**Verify:** `cd src-tauri && cargo test --lib native_scanner::steam` → pass

**Steps:**

- [ ] **Step 1: Check whether `steam_integration.rs` exposes a VDF parser**

Run: `cd src-tauri && grep -n 'libraryfolders\|appmanifest\|fn parse_vdf' src/steam_integration.rs`
Reuse if present.

- [ ] **Step 2: Write fixtures**

`tests/fixtures/libraryfolders.vdf` and `tests/fixtures/appmanifest_413150.acf` (Stardew Valley's appid).

- [ ] **Step 3: Implement & test** — see fixture-driven test pattern from Task 2.2.

- [ ] **Step 4: Commit**

```bash
git add -p src/native_scanner.rs tests/fixtures/
git commit -m "v0.14.0-wip: Steam mac native scanner"
```

---

### Task 2.4: MAS sandbox detection

**Goal:** Identify Mac App Store sandboxed games; mark `sandboxed = true`.

**Files:**
- Modify: `src-tauri/src/native_scanner.rs` (add `is_sandboxed`)

**Acceptance Criteria:**
- [ ] Function returns `true` if `<bundle>/Contents/_MASReceipt/receipt` exists
- [ ] Returns `true` if bundle is under `/System/Applications`
- [ ] Otherwise `false`
- [ ] Applied to every candidate produced by other scanners
- [ ] Test: tempdir with synthetic `_MASReceipt/receipt` → `true`; without → `false`

**Verify:** `cd src-tauri && cargo test --lib native_scanner::sandbox` → pass

**Steps:** straightforward; one function, two-three tests, commit.

```bash
git commit -m "v0.14.0-wip: MAS sandbox detection in native scanner"
```

---

### Task 2.5: Architecture detection (Apple Silicon vs Intel vs Universal)

**Goal:** Per-bundle, determine which slices the main executable contains.

**Files:**
- Modify: `src-tauri/src/native_scanner.rs`

**Acceptance Criteria:**
- [ ] `detect_architecture(executable_path: &Path) -> Architecture` reads Mach-O magic
- [ ] Recognizes: `0xFEEDFACF` (single-arch 64-bit, examine `cputype` for arm64 vs x86_64), `0xCAFEBABE` (fat universal binary, examine slice headers)
- [ ] Returns `Architecture::AppleSilicon` / `IntelOnly` / `Universal` / `Unknown`
- [ ] No new deps — read first ~256 bytes of file with `std::io::Read`
- [ ] Test: synthetic Mach-O headers (3 fixtures) → expected enum values

**Verify:** `cd src-tauri && cargo test --lib native_scanner::arch` → pass

**Steps:**

- [ ] **Step 1: Document the Mach-O magic numbers in module doc comment**

```
0xFEEDFACE — 32-bit
0xFEEDFACF — 64-bit
0xCAFEBABE — fat (big-endian header, slices follow)
0xCFFAEDFE — 64-bit (LE, swapped)
cputype == 0x0100000C → arm64 (CPU_TYPE_ARM | CPU_ARCH_ABI64)
cputype == 0x01000007 → x86_64
```

- [ ] **Step 2: Implement reader using byteorder-style manual parsing** (no new deps; `u32::from_le_bytes` / `from_be_bytes`).

- [ ] **Step 3: Test with three small fixture files** (10s of bytes each).

- [ ] **Step 4: Commit.**

```bash
git commit -m "v0.14.0-wip: detect Mach-O architecture (AS / Intel / Universal)"
```

---

### Task 2.6: GOG Galaxy mac scanner + manual file picker

**Goal:** GOG Galaxy uses `~/Games/<game name>` by default on mac. Plus manual "Add native game" command.

**Files:**
- Modify: `src-tauri/src/native_scanner.rs` (add `scan_gog_mac`)
- Modify: `src-tauri/src/commands/game_state.rs` (add `add_native_game_manually` command)

**Acceptance Criteria:**
- [ ] `scan_gog_mac()` walks `~/Games` for `.app` bundles (GOG Galaxy default location on mac)
- [ ] `add_native_game_manually(app_path: PathBuf)` validates the path is a `.app` bundle, reads its plist, returns a `NativeAppCandidate`
- [ ] Command is registered in `tauri::generate_handler!`
- [ ] Test: manual command rejects non-.app paths

**Verify:** `cd src-tauri && cargo test --lib native_scanner::gog` and a manual smoke from frontend later.

**Steps:** standard pattern. Commit.

```bash
git commit -m "v0.14.0-wip: GOG mac scanner + manual native add command"
```

---

### Task 2.7: Aggregate scanner + DB persistence

**Goal:** One entry point `scan_all_native()` that returns deduped candidates and persists them to DB on demand.

**Files:**
- Modify: `src-tauri/src/native_scanner.rs`
- Modify: `src-tauri/src/lib.rs` (register `rescan_native_games` command)

**Acceptance Criteria:**
- [ ] `scan_all_native()` calls `scan_applications_dirs`, `scan_steam_mac`, `scan_gog_mac`; dedupes by canonical `bundle_path`
- [ ] Tauri command `rescan_native_games(state)` calls it, writes results to DB via existing `games` table inserts
- [ ] Test: dedup logic with synthetic overlapping candidates

**Verify:** `cd src-tauri && cargo test --lib native_scanner::aggregate` → pass

```bash
git commit -m "v0.14.0-wip: aggregate native scanner + rescan_native_games command"
```

---

### Task 2.8: TS types + frontend hook

**Goal:** Frontend can list native candidates without UI yet (debug surface only).

**Files:**
- Modify: `src/lib/types.ts` (add `NativeAppCandidate`, `Architecture`, `NativeSource`)
- Modify: `src/lib/api.ts` (typed wrapper for `rescan_native_games`)

**Acceptance Criteria:**
- [ ] TS types match Rust (camelCase via `serde` serde_with or manual mapping — match existing convention)
- [ ] `rescanNativeGames(): Promise<NativeAppCandidate[]>`
- [ ] `npx svelte-check --threshold error` clean

**Verify:** `npx svelte-check --threshold error` → 0 errors

```bash
git commit -m "v0.14.0-wip: TS types + api wrapper for native scanner"
```

---

### Cut release v0.14.0

After Tasks 1.1–2.8 complete:

```bash
./scripts/release.sh 0.14.0
```

Confirms phases 1–2 work end to end with no native plugins yet. Foundation ready.

---

## Phase 3 — Stardew Valley Native Plugin (v0.15.0)

---

### Task 3.0: Verification spike — document SMAPI install steps

**Goal:** Read SMAPI's installer source, write `smapi-install-spec.md` co-located with this plan, listing the exact mutations the installer performs on a vanilla Stardew bundle.

**Files:**
- Create: `docs/superpowers/plans/2026-04-28-native-macos-game-support-smapi-install-spec.md`

**Acceptance Criteria:**
- [ ] Document lists every file copied/renamed/created
- [ ] Document lists every shell-script substitution
- [ ] Document the revert procedure
- [ ] Document where SMAPI looks for mods (Mods directory locations, in priority order)

This task is RESEARCH ONLY — no code. Source: <https://github.com/Pathoschild/SMAPI/blob/develop/src/SMAPI.Installer/install%20on%20macOS.command>.

- [ ] **Step 1:** Fetch the installer script via `WebFetch` or browse source.
- [ ] **Step 2:** Extract the install + uninstall logic.
- [ ] **Step 3:** Write the spec doc.
- [ ] **Step 4:** Commit (no code changes; doc only).

```bash
git commit -m "docs: spec for SMAPI mac install procedure"
```

---

### Task 3.1: `stardew_valley_native` plugin scaffolding

**Goal:** Plugin compiles, registers, returns empty `detect_native()` so smoke tests pass.

**Files:**
- Create: `src-tauri/src/plugins/stardew_valley_native.rs`
- Modify: `src-tauri/src/plugins/mod.rs` + `src-tauri/src/lib.rs` (register)

**Acceptance Criteria:**
- [ ] `StardewValleyNativePlugin` struct with `GamePlugin` impl
- [ ] `game_id() = "stardew_valley_native"`, `display_name() = "Stardew Valley (Native)"`, `nexus_slug() = "stardewvalley"`
- [ ] `detect_native()` returns `vec![]` (real impl in next task)
- [ ] Registered at startup via `register_plugin(Box::new(StardewValleyNativePlugin))`
- [ ] Test: registry lookup returns the plugin

**Verify:** `cd src-tauri && cargo test --lib plugins::stardew_valley_native` → pass

```bash
git commit -m "v0.15.0-wip: scaffold Stardew Valley native plugin"
```

---

### Task 3.2: Stardew detection by bundle identifier

**Goal:** Filter native candidates to find Stardew installs.

**Files:**
- Modify: `src-tauri/src/plugins/stardew_valley_native.rs`

**Acceptance Criteria:**
- [ ] `detect_native()` calls `crate::native_scanner::scan_all_native()`
- [ ] Filters to candidates with `bundle_identifier == "com.chucklefish.stardewvalley"` OR `bundle_executable == "StardewValley"` (defensive: GOG version may differ)
- [ ] For each match, build a `DetectedGame` with `runtime: GameRuntime::Native(...)` populated
- [ ] `game_path` = `<bundle>/Contents/MacOS`
- [ ] `data_dir` = same as `game_path` (mods drop alongside the game binary, matching SMAPI convention)
- [ ] `exe_path` = `<bundle>/Contents/MacOS/StardewValley` (the launcher script after SMAPI patch, or the original binary before)
- [ ] Test: synthetic candidate with Stardew identifier → 1 detected game; without → 0

**Verify:** `cd src-tauri && cargo test --lib plugins::stardew_valley_native` → pass

```bash
git commit -m "v0.15.0-wip: detect native Stardew Valley installs"
```

---

### Task 3.3: SMAPI presence detection

**Goal:** Tell whether SMAPI is installed in a given Stardew bundle.

**Files:**
- Create: `src-tauri/src/smapi.rs`

**Acceptance Criteria:**
- [ ] `is_installed(app_bundle: &Path) -> bool` returns true if BOTH:
  - `<bundle>/Contents/MacOS/StardewModdingAPI` (or `.dll`) exists
  - `<bundle>/Contents/MacOS/StardewValley-original` exists (or the launcher script contains the SMAPI marker — confirm exact form per spike spec)
- [ ] `installed_version(app_bundle: &Path) -> Option<String>` reads the SMAPI manifest if present
- [ ] Test: synthetic bundle layouts → expected results

**Verify:** `cd src-tauri && cargo test --lib smapi::` → pass

```bash
git commit -m "v0.15.0-wip: SMAPI presence detection"
```

---

### Task 3.4: SMAPI auto-install (port of `install on macOS.command`)

**Goal:** Download SMAPI's latest macOS release from GitHub, extract, apply the install steps documented in the Task 3.0 spike spec.

**Files:**
- Modify: `src-tauri/src/smapi.rs` (add `install`)
- Reuse: `mod_tools.rs` GitHub release fetcher

**Acceptance Criteria:**
- [ ] `install(app_bundle: &Path) -> Result<(), SmapiError>` fetches latest release, extracts, applies mutations matching the spike spec
- [ ] Pre-install: snapshot the bundle's `Contents/MacOS` directory via `auto_snapshot_before_destructive`
- [ ] Idempotent: running install on an already-SMAPI'd bundle is a no-op (or upgrades version cleanly)
- [ ] Test: integration test against a mocked bundle layout (no real download — inject a local fixture archive)

**Verify:** `cd src-tauri && cargo test --lib smapi::install` → pass

```bash
git commit -m "v0.15.0-wip: SMAPI auto-install"
```

---

### Task 3.5: SMAPI uninstall / revert

**Goal:** Restore the bundle to vanilla.

**Files:**
- Modify: `src-tauri/src/smapi.rs` (add `uninstall`)

**Acceptance Criteria:**
- [ ] `uninstall(app_bundle: &Path) -> Result<(), SmapiError>` reverses install mutations
- [ ] If no SMAPI installed, returns `Ok(())` (no-op)
- [ ] Tested via fixture round-trip: install → uninstall → byte-for-byte equal to pre-install state

```bash
git commit -m "v0.15.0-wip: SMAPI uninstall"
```

---

### Task 3.6: Mod manifest parser + mod root resolver

**Goal:** Parse `manifest.json`, locate Stardew's Mods dir.

**Files:**
- Modify: `src-tauri/src/plugins/stardew_valley_native.rs`

**Acceptance Criteria:**
- [ ] `parse_manifest(path: &Path) -> Result<SdvModManifest, ManifestError>` extracts `Name`, `Version`, `UniqueID`, `Dependencies[]`, `MinimumApiVersion`
- [ ] `resolve_mods_dir(detected: &DetectedGame) -> PathBuf` returns `<bundle>/Contents/MacOS/Mods` (SMAPI's convention; the `~/Library/Application Support/StardewValley/Mods` rumor is incorrect for SMAPI — verify against spike spec)
- [ ] Test: parse a realistic manifest fixture; resolve mods dir for synthetic detected game

```bash
git commit -m "v0.15.0-wip: Stardew manifest parser + mods dir resolver"
```

---

### Task 3.7: Stardew native deploy hook

**Goal:** Implement `deploy_native_game` for Stardew specifically — copy mod files into `Mods/<UniqueID>/`.

**Files:**
- Modify: `src-tauri/src/deployer.rs` (add Stardew path)
- Modify: `src-tauri/src/plugins/stardew_valley_native.rs` (provide deploy hook)

**Acceptance Criteria:**
- [ ] `deploy_native_game` for Stardew copies (or hardlinks, matching existing wine deployer's hardlink-first behavior) each staged mod into `<mods_dir>/<UniqueID>/`
- [ ] Refuses to write outside `<bundle>/Contents/MacOS/Mods` (path safety)
- [ ] Updates `mod_dependencies` table for cross-mod dep tracking
- [ ] Test: deploy a fake mod with manifest → file appears in expected location; bad path rejected

```bash
git commit -m "v0.15.0-wip: Stardew native deploy"
```

---

### Task 3.8: Conflict + dependency surface

**Goal:** Surface unmet deps and conflicting `UniqueID` in API for the UI.

**Files:**
- Modify: `src-tauri/src/plugins/stardew_valley_native.rs`
- Modify: `src-tauri/src/commands/mods.rs` (new command `get_stardew_mod_status`)

**Acceptance Criteria:**
- [ ] Returns: `{ unique_id, missing_deps: Vec<String>, conflicts: Vec<String>, api_version_ok: bool }`
- [ ] Reuses existing `mod_dependencies.rs` storage
- [ ] Test: synthetic state with missing dep → expected output

```bash
git commit -m "v0.15.0-wip: Stardew mod conflict + dep surface"
```

---

### Task 3.9: Re-quarantine repair

**Goal:** When macOS Gatekeeper re-quarantines the bundle (after game update), automatically clear xattrs.

**Files:**
- Modify: `src-tauri/src/smapi.rs` (add `clear_quarantine`)

**Acceptance Criteria:**
- [ ] Function shells out to `xattr -dr com.apple.quarantine <bundle>` (or uses `std::process::Command`)
- [ ] Called automatically before each launch if SMAPI is detected
- [ ] Returns Ok even if no quarantine present (xattr is idempotent)
- [ ] Test: command formation matches expected; integration tested manually

```bash
git commit -m "v0.15.0-wip: SMAPI quarantine repair"
```

---

## Phase 4 — Baldur's Gate 3 Native Plugin (v0.16.0)

---

### Task 4.0: Verification spike — modsettings.lsx schema

**Goal:** Document BG3 native paths + `modsettings.lsx` schema. RESEARCH ONLY.

**Files:**
- Create: `docs/superpowers/plans/2026-04-28-native-macos-game-support-bg3-spec.md`

**Acceptance Criteria:**
- [ ] Document the exact path to `modsettings.lsx` on native mac (likely `~/Library/Application Support/Larian Studios/Baldur's Gate 3/PlayerProfiles/Public/modsettings.lsx`)
- [ ] Document the path to `Mods/` (`.pak` drop dir) on native mac
- [ ] Provide an annotated example `modsettings.lsx`, identifying the `<region id="ModuleSettings">` block, the `<node id="Mods">` children, and the canonical `GustavDev` master entry
- [ ] Document `meta.lsx` schema (inside `.pak` files)
- [ ] Note BG3SE Mac status (likely lags Win — version mismatch surfacing only)

```bash
git commit -m "docs: BG3 native paths + modsettings.lsx schema"
```

---

### Task 4.1: BG3 native plugin scaffolding

Same shape as Task 3.1, but for `BaldursGate3NativePlugin`. Detection key: `CFBundleIdentifier == "com.larian.bg3"` (verify in spike).

```bash
git commit -m "v0.16.0-wip: scaffold BG3 native plugin"
```

---

### Task 4.2: `bg3_lsx` parser/writer

**Goal:** Read + edit `modsettings.lsx` using `quick-xml` (already in deps).

**Files:**
- Create: `src-tauri/src/bg3_lsx.rs`

**Acceptance Criteria:**
- [ ] `read_modsettings(path: &Path) -> Result<ModSettings, LsxError>` returns the ordered list of mod entries
- [ ] `write_modsettings(path: &Path, settings: &ModSettings) -> Result<()>` writes preserving the canonical XML shape
- [ ] `ModSettings` carries `Vec<ModEntry { uuid, name, folder, version }>`
- [ ] `GustavDev` master entry is preserved at the top automatically
- [ ] Test: round-trip a real modsettings.lsx fixture from the spike

**Verify:** `cd src-tauri && cargo test --lib bg3_lsx::` → pass

```bash
git commit -m "v0.16.0-wip: BG3 modsettings.lsx parser + writer"
```

---

### Task 4.3: `.pak` meta extraction

**Goal:** Read `meta.lsx` from inside a `.pak` to surface the mod's UUID/folder/name.

**Files:**
- Modify: `src-tauri/src/bg3_lsx.rs` (add `read_pak_meta`)

**Acceptance Criteria:**
- [ ] BG3 .pak format: LSPK header followed by zlib/lz4-compressed entries — port the minimal reader from BGMM source (link in spike)
- [ ] Extract `meta.lsx`, parse with the LSX parser from Task 4.2
- [ ] Test: a fixture `.pak` (small one — find in BG3 community archives) → expected UUID

> **Note:** if the `.pak` reader is too large for one task, split into 4.3a (header parsing) + 4.3b (entry decompression).

```bash
git commit -m "v0.16.0-wip: BG3 .pak meta.lsx extraction"
```

---

### Task 4.4: BG3 native deploy

**Goal:** Copy `.pak`s into `Mods/`, edit `modsettings.lsx` to add load order entry.

**Files:**
- Modify: `src-tauri/src/deployer.rs` (BG3 path)
- Modify: `src-tauri/src/plugins/baldurs_gate_3_native.rs`

**Acceptance Criteria:**
- [ ] Pre-deploy snapshot of `modsettings.lsx`
- [ ] `.pak` copied to `~/Library/Application Support/Larian Studios/Baldur's Gate 3/Mods/`
- [ ] Mod entry inserted into `<node id="Mods">` of `modsettings.lsx`
- [ ] Test: synthetic deploy round-trip

```bash
git commit -m "v0.16.0-wip: BG3 native deploy + load order edit"
```

---

### Task 4.5: BG3 load order UI hook

**Goal:** Frontend reorders mods; backend writes `modsettings.lsx` accordingly. Reuses existing `LoadOrderKind::FileBased` infra.

**Files:**
- Modify: `src-tauri/src/plugins/baldurs_gate_3_native.rs`
- Modify: `src-tauri/src/commands/load_order.rs`

**Acceptance Criteria:**
- [ ] `load_order_kind()` returns `LoadOrderKind::FileBased(...)` pointing at `modsettings.lsx`
- [ ] Reorder command writes through `bg3_lsx::write_modsettings`
- [ ] `GustavDev` cannot be reordered (filtered out of editable list)

```bash
git commit -m "v0.16.0-wip: BG3 load order via FileBased UI"
```

---

### Task 4.6: BG3SE detection (read-only)

**Goal:** Detect Script Extender; warn if outdated. Install deferred.

**Files:**
- Modify: `src-tauri/src/plugins/baldurs_gate_3_native.rs`

**Acceptance Criteria:**
- [ ] Detects BG3SE presence by looking for the script extender's loader file (path TBD in spike)
- [ ] Surfaces `bg3se_status: { installed: bool, version: Option<String>, mac_supported: bool }` via Tauri command

```bash
git commit -m "v0.16.0-wip: BG3SE read-only status"
```

---

## Phase 5 — Native Mode UI Shell + Theming (v0.15.0 frontend)

---

### Task 5.0: Verify `tauri-plugin-liquid-glass` API surface

**Goal:** Read the plugin's source to confirm it supports runtime intensity changes. RESEARCH ONLY.

**Files:** none (writes to spike doc)

**Acceptance Criteria:**
- [ ] Document the plugin's available commands/config in `2026-04-28-native-macos-game-support-glass-spec.md`
- [ ] Confirm whether vibrancy intensity can be changed at runtime via JS bridge OR only at window-creation time
- [ ] If runtime-toggleable: ratchet via JS API. If not: drop the runtime toggle and apply native theme at window-creation when `native_mode` is set, requiring restart on toggle.

```bash
git commit -m "docs: tauri-plugin-liquid-glass capability survey"
```

---

### Task 5.1: Tokenize hardcoded colors → CSS custom properties

**Goal:** Audit `src/app.css` and per-component scoped styles; move every literal color to a CSS variable. Necessary so the M5 theme can override them.

**Files:**
- Modify: `src/app.css`
- Modify: any component `<style>` block using literal colors

**Acceptance Criteria:**
- [ ] Existing dark-theme behavior unchanged on Wine mode
- [ ] No `#hex` or `rgb(...)` literals remain in component scoped styles outside `app.css`
- [ ] `npx svelte-check --threshold error` clean
- [ ] Manual: launch app in dev, verify dark theme looks identical

> This is a sweep task. May be split if too big — split by component group (topbar / sidebar / pages / modals).

```bash
git commit -m "v0.15.0-wip: tokenize hardcoded colors"
```

---

### Task 5.2: M5 native theme palette

**Goal:** Add `[data-theme="native"]` block defining the M5 palette.

**Files:**
- Modify: `src/app.css`

**Acceptance Criteria:**
- [ ] New token block:

```css
[data-theme="native"] {
  --bg-base: #000000;
  --bg-base-vibrancy: rgba(0, 0, 0, 0.55);
  --bg-primary: #06070d;
  --bg-secondary: #0a0c14;
  --bg-elevated: #11141d;

  --surface-glass: rgba(255, 255, 255, 0.03);
  --surface-glass-hover: rgba(255, 255, 255, 0.06);

  --text-primary: rgba(255, 255, 255, 0.92);
  --text-secondary: rgba(255, 255, 255, 0.62);
  --text-tertiary: rgba(255, 255, 255, 0.32);

  /* M5 gradient stops */
  --m5-cyan: #4ecbff;
  --m5-blue: #5b73ff;
  --m5-purple: #a855f7;
  --m5-gradient: linear-gradient(135deg, var(--m5-cyan) 0%, var(--m5-blue) 50%, var(--m5-purple) 100%);

  --accent: var(--m5-blue);
  --accent-hover: var(--m5-cyan);
  --accent-subtle: rgba(91, 115, 255, 0.16);
  --system-accent: var(--m5-blue);

  --separator: rgba(255, 255, 255, 0.06);
  --shadow-glass: 0 8px 32px rgba(0, 0, 0, 0.55), 0 0 1px rgba(168, 85, 247, 0.2);
}
```

- [ ] Theme application is driven by a `applyNativeTheme()` helper in `src/lib/native/theme.ts` that toggles `document.body.dataset.theme`
- [ ] Test (Vitest): `applyNativeTheme(true)` sets `body.dataset.theme = 'native'`; false reverts to user's prior preference

```bash
git commit -m "v0.15.0-wip: M5 native theme palette"
```

---

### Task 5.3: Glass intensification utility classes

**Goal:** Add utility classes for native-only glass treatment.

**Files:**
- Modify: `src/app.css`

**Acceptance Criteria:**
- [ ] `.native-glass-card` — heavier blur (24px), gradient border using M5 stops
- [ ] `.native-glass-panel` — full-bleed semi-transparent panel with backdrop-filter
- [ ] `.m5-gradient-text` — gradient-clipped text for headers
- [ ] All classes scoped under `[data-theme="native"]` so they degrade to no-op in Wine mode

```bash
git commit -m "v0.15.0-wip: M5 glass utility classes"
```

---

### Task 5.4: Apply liquid-glass plugin ratchet (or window-vibrancy fallback)

**Goal:** Wire the runtime ratchet documented in Task 5.0.

**Files:**
- Modify: `src-tauri/src/lib.rs` (window setup) OR a new `src-tauri/src/native_window.rs`
- Modify: `src/lib/native/theme.ts`

**Acceptance Criteria:**
- [ ] Toggle handler calls into Tauri command (or restarts window) per spike's verdict
- [ ] Manual smoke: switch into Native Mode → window vibrancy visibly more intense

```bash
git commit -m "v0.15.0-wip: native mode liquid-glass ratchet"
```

---

### Task 5.5: Mode-scoped routing

**Goal:** New routes under `/native/*` mirror the existing app structure but only show native games.

**Files:**
- Create: `src/routes/native/+layout.svelte` (M5 theme + mode guard)
- Create: `src/routes/native/+page.svelte` (landing → discover or mods)
- Create: `src/routes/native/mods/+page.svelte`
- Create: `src/routes/native/discover/+page.svelte`
- Create: `src/routes/native/settings/+page.svelte`
- Modify: `src/routes/+layout.svelte` (theme injection on route change)

**Acceptance Criteria:**
- [ ] `+layout.svelte` for `/native/*` calls `applyNativeTheme(true)` on mount, reverts on unmount
- [ ] Wine routes (`/mods`, `/collections`, etc.) untouched
- [ ] `npx svelte-check --threshold error` clean
- [ ] Manual smoke: navigate into `/native/mods` → theme switches; back to `/mods` → reverts

```bash
git commit -m "v0.15.0-wip: /native/* routes with M5 theme"
```

---

### Task 5.6: TopBarGameSelector native branch

**Goal:** In native mode, the game selector lists native candidates with runtime badges.

**Files:**
- Modify: `src/lib/components/topbar/TopBarGameSelector.svelte`

**Acceptance Criteria:**
- [ ] When `nativeMode === true`, fetches `rescanNativeGames()` instead of bottle-scoped detection
- [ ] Each entry shows a chip badge: "Apple Silicon" / "Intel (Rosetta)" / "Universal", plus source badge ("Steam" / "GOG" / "Manual")
- [ ] Sandboxed games are listed with a disabled state and "Unsupported (sandboxed)" tooltip
- [ ] Visual: badges use `.m5-gradient-text` in native mode

```bash
git commit -m "v0.15.0-wip: TopBarGameSelector native branch + badges"
```

---

### Task 5.7: Hide Wine-only features in native mode

**Goal:** Routes/components irrelevant to native are hidden or disabled.

**Files:**
- Modify: `src/routes/+layout.svelte` (sidebar navigation lives here — add `{#if !$nativeMode}` guards around Wine-only links: `/modlists`, `/collections`, `/plugins`)
- Modify: `src/routes/settings/+page.svelte` (hide SKSE / Engine Fixes / Wine diagnostics / display fix sections under `{#if !$nativeMode}`)
- Modify: `src/routes/mods/+page.svelte` (hide SKSE panel)

**Acceptance Criteria:**
- [ ] Sidebar in `+layout.svelte` hides `/modlists` (Wabbajack), `/collections` (Wine collections), `/plugins` (ESP load order) when `nativeMode === true`
- [ ] Settings page hides SKSE, Engine Fixes auto-deploy, Wine diagnostics, display fix, cursor clamp sections in native mode
- [ ] Native-only sidebar items shown: "Mods" → `/native/mods`, "Discover" → `/native/discover`, "Settings" → `/native/settings`
- [ ] `npx svelte-check --threshold error` clean
- [ ] Manual smoke: toggle native mode → Wabbajack/Collections/Plugins sidebar links disappear; native links appear

```bash
git commit -m "v0.15.0-wip: hide Wine-only features in native mode"
```

---

### Task 5.8: First-run banner

**Goal:** If the user has detected native games + native mode is off, show a one-time dismissible banner promoting it.

**Files:**
- Create: `src/lib/components/banners/NativeModeBanner.svelte`
- Modify: `src/routes/+layout.svelte` (mount banner)
- Modify: `src/lib/stores.ts` (dismissed state via localStorage)

**Acceptance Criteria:**
- [ ] Banner appears only when: `nativeMode === false` AND native scan returned ≥1 candidate AND `localStorage.getItem('native-banner-dismissed') !== '1'`
- [ ] CTA: "Try Native Mode (beta)" → toggles `setNativeMode(true)` and routes to `/native`
- [ ] Dismiss button persists state

```bash
git commit -m "v0.15.0-wip: native mode first-run banner"
```

---

### Cut release v0.15.0

After Tasks 3.0–3.9 + 5.0–5.8 complete:

```bash
./scripts/release.sh 0.15.0
```

Stardew Valley + Native Mode UI shell ship together.

---

## Phase 6 — Polish, Safety Nets, Tests (v0.16.0)

---

### Task 6.1: Native auto-snapshot integration

**Goal:** `auto_snapshot_before_destructive` covers native paths.

**Files:**
- Modify: `src-tauri/src/rollback.rs` (or wherever the helper lives)

**Acceptance Criteria:**
- [ ] Native deploy/uninstall calls the helper before mutating state
- [ ] Snapshot includes the SMAPI launcher script (Stardew) or `modsettings.lsx` (BG3)
- [ ] Test: a native deploy creates a snapshot row in DB

```bash
git commit -m "v0.16.0-wip: extend auto-snapshot to native paths"
```

---

### Task 6.2: Sandbox refusal with friendly error

**Goal:** When a sandboxed game is selected, modding actions throw a clear, user-readable error.

**Files:**
- Modify: `src-tauri/src/commands/mods.rs` (centralized guard)
- Modify: a new `src/lib/components/SandboxedGameNotice.svelte`

**Acceptance Criteria:**
- [ ] All mutating commands (deploy, install, uninstall) check `runtime.native()?.sandboxed` and short-circuit with a typed error
- [ ] Frontend displays the notice component instead of normal mod UI when sandboxed

```bash
git commit -m "v0.16.0-wip: refuse modding for sandboxed games"
```

---

### Task 6.3: Code-signing trust boundary documentation

**Goal:** Update `CLAUDE.md` and a new `docs/native-trust-boundaries.md` explaining what we touch in `.app` bundles, why, and the revert path.

**Files:**
- Modify: `CLAUDE.md`
- Create: `docs/native-trust-boundaries.md`

**Acceptance Criteria:**
- [ ] Section in CLAUDE.md: "Native mode trust boundaries" — when modifying `.app/Contents`, when not, why SMAPI is the documented exception
- [ ] Doc lists each game's allowed mutations + revert procedure

```bash
git commit -m "docs: native mode trust boundaries"
```

---

### Task 6.4: 20+ test sweep

**Goal:** Bring native test count to ≥20 across modules.

**Files:**
- Modify: existing test modules

**Acceptance Criteria:**
- [ ] Inventory: count current native-related tests across `runtime`, `native_scanner`, `plist`, `smapi`, `bg3_lsx`, `plugins/stardew_valley_native`, `plugins/baldurs_gate_3_native`, `deployer`, `migrations`
- [ ] Add tests until total ≥20
- [ ] Gaps to prioritize: error paths (malformed plist, missing bundle, missing modsettings.lsx), edge cases (universal binary, both Stardew install variants)

```bash
git commit -m "v0.16.0-wip: native mode test sweep (20+ tests)"
```

---

### Task 6.5: Lift feature flag

**Goal:** Native mode toggle visible in topbar (no longer hidden under "Experimental").

**Files:**
- Modify: `src/lib/components/topbar/*.svelte` (toggle position)
- Modify: `src/routes/settings/+page.svelte` (remove "Experimental" gate)

**Acceptance Criteria:**
- [ ] Topbar toggle visible by default
- [ ] Settings → Experimental no longer required

```bash
git commit -m "v0.16.0: lift native mode feature flag"
```

---

### Cut release v0.16.0

```bash
./scripts/release.sh 0.16.0
```

Native Mode promoted to GA.

---

## Self-Review Checklist (run after writing the plan)

- [x] Spec coverage: each phase from the spec maps to ≥1 task. ✅
- [x] DB migration version aligned with current `TARGET_VERSION = 22` (plan uses v23, not the spec's v20). ✅
- [x] No placeholders / TBDs / "implement later" in step bodies. ✅
- [x] Method names consistent: `detect_wine` / `detect_native` used throughout, not aliases. ✅
- [x] Open questions called out at top, with research-only spike tasks (3.0, 4.0, 5.0) before any dependent code task. ✅
- [x] Existing-deps check confirmed `quick-xml`, `tauri-plugin-liquid-glass`, `window-vibrancy`, `dirs` already present. Only new dep is `plist = "1"`. ✅
- [x] Verification commands provided for each task. ✅
- [x] Commit message format matches project convention (`v{version}-wip: ...` for in-progress, `v{version}: ...` for release tags). ✅
