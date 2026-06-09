# Native macOS Mode vs Wine Mode — Parity Audit
*HEAD: `7a80310` · 2026-06-09 · branch `feature/native-mode` · worktree `/Users/cashconway/Corkscrew-native-mode`*

## Summary

Native mode covers roughly **35-40%** of Wine-mode functionality. The plumbing
work is solid: a clean `GameRuntime::{Wine,Native}` discriminator
([src-tauri/src/runtime.rs](../../src-tauri/src/runtime.rs)), a dual
`resolve_game` / `resolve_game_any_runtime` split in
[src-tauri/src/lib.rs:168-216](../../src-tauri/src/lib.rs#L168), four
registered native plugins (Stardew, Paralives, BG3, Crimson Desert), and a
working install-and-launch path on `/native/mods`. The headline gaps are:
**(1)** only three of ~30 Tauri command files were updated to use
`resolve_game_any_runtime`, so most surfaces (conflicts, profiles, snapshots,
load order, deployment manifest, custom EXEs, tools, depot, diagnostics)
explicitly Wine-only — they will throw `Bottle '' not found` when called for
a native game; **(2)** Stardew SMAPI install/uninstall and BG3SE install are
implemented in Rust but **not wired as Tauri commands** — the native page
is a status viewer only for those runtimes; **(3)** Crimson Desert
deploy_native is intentionally blocked pending PAZ-overlay verification;
**(4)** there is no native equivalent of `/plugins`, `/profiles`, `/logs`,
`/collections`, `/modlists`, or the dashboard. Top priority gaps to close
are SMAPI/BG3SE command wiring, `resolve_game_any_runtime` propagation to
deployment + conflicts + snapshots, and a per-runtime load-order page for
BG3.

---

## 1. Game detection & registration

| Concern | Wine | Native | Status |
|---|---|---|---|
| Plugin trait | `detect_wine(&Bottle)` default `None` | `detect_native()` default `Vec::new()` | OK — split by [src-tauri/src/games.rs:47-57](../../src-tauri/src/games.rs#L47) |
| Scan entry | `detect_bottles()` -> `detect_games()` | `detect_native_games()` aggregated by [src-tauri/src/games.rs:378](../../src-tauri/src/games.rs#L378) | OK |
| Combined scan | `detect_all_games_with_custom` walks both | same fn aggregates native + custom | OK ✅ |
| Steam manifest scan | `detect_unregistered_steam_games` for each bottle | no equivalent — Steam mac libraries scanned only by `native_scanner::scan_all_native` | 🟡 native Steam library discovery is plugin-by-plugin, not a generic Steam-on-mac sweep |
| Manual add | adds custom-game DB row | `add_manual_native_game` + `register_manual_native_game` ([native_cmds.rs:188](../../src-tauri/src/commands/native_cmds.rs#L188)) | ✅ Works — plugin-match + generic registration |
| Mach-O arch detection | n/a (Wine PEs only) | `native_scanner::detect_architecture` recognizes single-arch + fat ([native_scanner.rs:49](../../src-tauri/src/native_scanner.rs#L49)) | ✅ native-only feature |
| MAS sandbox refusal | n/a | `validate_manual_native_app` + per-plugin deploy guards | ✅ native-only |
| Persistence | bottle re-scanned each call (cheap) | `PersistedGame` upserted into `games` table on every rescan | ✅ |

**Registered native plugins** (from [lib.rs:455-461](../../src-tauri/src/lib.rs#L455)):
`stardew_valley_native`, `paralives_native`, `baldurs_gate_3_native`,
`crimson_desert_native`.

---

## 2. Mod install pipeline

| Step | Wine | Native | Status | Notes |
|---|---|---|---|---|
| Browse Nexus | `/collections` discover panel | `/native/discover` reuses `NexusBrowsePanel` | ✅ Works | shared component, gates on `runtime !== "native"` |
| Download archive (premium) | `download_and_install_nexus_mod` uses `resolve_game_any_runtime` | same command | ✅ Works | [src-tauri/src/commands/nexus.rs:307](../../src-tauri/src/commands/nexus.rs#L307) |
| Download via browser (free) | premium-gated identically | premium-gated identically | ✅ Works | same `get_download_links` enforcement |
| Extract archive | `installer::extract_archive` | same — shared code | ✅ Works | |
| Stage files | `staging::stage_mod` keyed on `(game_id, bottle_name)` with `""` sentinel for native | same | ✅ Works | |
| Validate / classify | `installer` mod-type detection | same — plugins return `use_legacy_data_dir = false` to opt in | 🟡 Partial | Paralives uses mod-type routing for BepInEx DLLs vs data; Stardew/BG3 use bespoke deploy paths |
| Deploy (Wine) | `deployer::deploy_wine_game` → hardlinks into `<game>/Data` | not invoked | n/a | |
| Deploy (Native) | n/a | `deployer::deploy_native_game` → dispatches to `plugin.deploy_native` ([deployer.rs:1695](../../src-tauri/src/deployer.rs#L1695)) | ✅ Works for Stardew + Paralives; 🔴 blocked for Crimson Desert; ✅ Works for BG3 |
| Manual install via "Install from archive" | `install_mod_cmd` in [mods.rs:308](../../src-tauri/src/commands/mods.rs#L308) | same command, uses `resolve_game_any_runtime` at [mods.rs:336](../../src-tauri/src/commands/mods.rs#L336) | ✅ Works | But no UI button on `/native/mods` exposes it — drag-drop / file picker missing |
| Drag-drop install | wired on `/mods` | **🔴 Missing — needed** on `/native/mods` (no drop zone) | 🔴 | |
| NXM handler | `nxm_handler` routes to install pipeline | same handler, but only routes to slug-matched games — Paralives/Stardew slugs covered | ✅ Works for matched games |

---

## 3. Mod management UI (`/mods` vs `/native/mods`)

| Feature | Wine `/mods` | Native `/native/mods` | Status |
|---|---|---|---|
| List installed mods | ✅ | ✅ | ✅ Works ([+page.svelte:89](../../src/routes/native/mods/+page.svelte#L89)) |
| Search | ✅ | ✅ | ✅ |
| Sort (name/version/files/date) | ✅ | ✅ | ✅ |
| Toggle enable/disable | ✅ | ✅ | ✅ — calls `toggle_mod` which uses `resolve_game_any_runtime` |
| Uninstall single | ✅ | ✅ | ✅ |
| Batch enable/disable/uninstall | ✅ | ✅ | ✅ |
| Open mods folder in Finder | ✅ | ✅ | ✅ |
| Open on NexusMods | ✅ | ✅ | ✅ |
| Drag-to-reorder priority | ✅ | 🔴 not present | 🔴 backend `reorder_mods` requires Wine `resolve_game` |
| Conflict resolution panel | ✅ | 🔴 not present | 🔴 `analyze_conflicts_cmd` is Wine-only ([deployment.rs:42](../../src-tauri/src/commands/deployment.rs#L42)) |
| Mod notes / tags / categories | ✅ | 🔴 not present | 🔴 backend cmds are Wine-only (`set_mod_notes`, `set_mod_tags`) |
| Version rollback | ✅ | 🔴 not present | 🔴 `rollback_mod_version` is Wine-only |
| Dependency tree | ✅ | 🔴 not present | 🔴 |
| Pinned game version | ✅ | n/a (no depot downgrade on native) | ❌ Wine/Steam-specific |
| SKSE / EngineFixes panel | ✅ Skyrim-only | n/a | ❌ Bethesda+Wine specific |
| Game launch button | ✅ in topbar | ✅ in topbar (same `launchGame` invoke with empty `bottle_name`) | ✅ Works — see §4 |
| Drag-drop archive install | ✅ | 🔴 not present | 🔴 needs drop zone in `/native/mods` |

`/native/mods` total: **1385 lines**. Wine `/mods` is 6263 lines. Diff
roughly correlates with the feature gap above.

---

## 4. Game launch

| Concern | Wine | Native | Status |
|---|---|---|---|
| Topbar play button | uses `launchGame(game_id, bottle_name, useSkse)` ([+layout.svelte:787](../../src/routes/+layout.svelte#L787)) | same — `wineCtx(g)?.bottle_name ?? ""` collapses to `""` sentinel for native | ✅ Works |
| Resolve | `resolve_game(bottle_name)` | `resolve_game_any_runtime` ([mods.rs:1702](../../src-tauri/src/commands/mods.rs#L1702)) | ✅ |
| Pre-launch self-heal | redeploy missing files from manifest | runs for native too — shared code in [mods.rs:1709](../../src-tauri/src/commands/mods.rs#L1709) | ✅ |
| Steam native launch | n/a | `steam://run/<app_id>` when `NativeSource::Steam` + `steam_app_id` set ([mods.rs:1748](../../src-tauri/src/commands/mods.rs#L1748)) | ✅ Works — Steamworks injection preserved |
| Non-Steam native launch | n/a | `open <bundle>` via Launch Services | ✅ Works |
| Custom executables | ✅ `executables::get_default_executable` per (game, bottle) | 🟡 **never checked on native path** — the native branch returns before custom-exe lookup at [mods.rs:1788](../../src-tauri/src/commands/mods.rs#L1788) | 🟡 backend ready, native path skips it |
| SKSE launch | ✅ `skse64_loader.exe` lookup | ❌ Bethesda+Wine concept | n/a |
| Display fix (Skyrim) | ✅ pre-launch | ❌ | n/a |
| Cursor clamp | ✅ via event-tap on Wine | ❌ Wine compositor-specific | n/a |
| Wine registry tweaks | ✅ `fix_cursor_grab` | ❌ | n/a |
| PID capture / game-lock | ✅ child PID tracked, mod ops blocked during play | 🟡 **PID is `None` because `open` detaches** ([mods.rs:1779](../../src-tauri/src/commands/mods.rs#L1779)) | 🟡 game-lock therefore not enforced for native — destructive mod ops during play are not blocked |

---

## 5. Collections / Wabbajack modlists

| Surface | Wine | Native | Status |
|---|---|---|---|
| `/collections` discover | ✅ | n/a — replaced by `/native/discover` Nexus browse | 🟡 native has Nexus browse but no NM Collections support |
| Install Collection | ✅ `list_installed_collections_cmd` etc. | 🔴 all 35+ collection commands in [collections.rs](../../src-tauri/src/commands/collections.rs) use `resolve_game` (Wine-only) | 🔴 would error `Bottle '' not found` |
| Delete / switch / restore-snapshot collection | ✅ | 🔴 same Wine-only resolve | 🔴 |
| `/modlists` Wabbajack gallery | ✅ | 🔴 no `/native/modlists` route | ❌ Wabbajack is Bethesda+Wine — sensible to remain Wine-only |
| Install Wabbajack modlist | ✅ 7-phase pipeline | ❌ Wabbajack targets are 100% Bethesda/Wine | ❌ Not applicable |
| Collection revision diff | ✅ | 🔴 not exposed | 🔴 (if any future native collection support lands) |
| Profile share / Verified lists | ✅ | 🔴 not exposed | 🟡 cross-cutting; reusable for native |

For native games, NexusMods collections COULD apply (e.g. a Paralives or BG3
collection on NM) — the data model already treats `collection_name` as a
string, but the pipeline references bottle paths throughout.

---

## 6. Plugin / load order management

| Concern | Wine | Native | Status |
|---|---|---|---|
| `/plugins` page | ✅ full ESP/ESL/ESM UI | 🔴 not present in `/native/*` | 🔴 — needed for BG3 |
| Backend trait | `LoadOrderKind::Plugins` (Bethesda) | `LoadOrderKind::FileBased(Bg3ModSettings)` for BG3 ([baldurs_gate_3_native.rs:440](../../src-tauri/src/plugins/baldurs_gate_3_native.rs#L440)) | ✅ backend complete |
| `get_file_based_load_order` cmd | n/a | exists ([load_order.rs:77](../../src-tauri/src/commands/load_order.rs#L77)) | 🟡 backend ready, UI missing |
| `set_file_based_load_order` cmd | n/a | exists ([load_order.rs:117](../../src-tauri/src/commands/load_order.rs#L117)) | 🟡 backend ready, UI missing |
| BG3 `modsettings.lsx` write | n/a | encoded in `deploy_native_inner` and `Bg3ModSettings` format | ✅ backend complete |
| LOOT integration | ✅ `libloot v0.29` | ❌ LOOT only knows Bethesda games | ❌ Not applicable |
| Plugin warnings / masterlist | ✅ | ❌ | ❌ Not applicable |
| Stardew dep / conflict analyzer | n/a | `analyze_mod_status` implemented; `get_stardew_mod_status` cmd is a **Phase 1 stub** returning `Vec::new()` ([stardew_cmds.rs:26](../../src-tauri/src/commands/stardew_cmds.rs#L26)) | 🟡 backend complete, **command not wired** |

---

## 7. Profile management

| Feature | Wine | Native | Status |
|---|---|---|---|
| `/profiles` page | ✅ | 🔴 no `/native/profiles` | 🔴 |
| `list_profiles_cmd` | ✅ uses DB key `(game_id, bottle_name)` | 🔴 all commands in [profiles.rs](../../src-tauri/src/commands/profiles.rs) use `resolve_game` (Wine-only) | 🔴 |
| `activate_profile` / `save_profile_snapshot` | ✅ | 🔴 same Wine-only resolve | 🔴 |
| Per-profile save backup | ✅ via `get_saves_dir` | 🔴 `get_saves_dir` default returns `None` for native plugins | 🟡 backend hookable, no plugin overrides |
| Profile sharing | ✅ `profile_sharing.rs` | 🔴 not exposed | 🔴 |

DB schema treats `bottle_name = ""` as a valid key, so the database layer
itself is runtime-agnostic. The blocker is the resolver, not storage.

---

## 8. Snapshots & rollback

| Feature | Wine | Native | Status |
|---|---|---|---|
| Auto-snapshot before destructive | ✅ `auto_snapshot_before_destructive` | 🟡 called from native deploy paths (Paralives + Stardew + BG3 plugins explicitly create snapshots) | ✅ for the three native runtimes that deploy |
| `create_mod_snapshot` | ✅ | 🔴 Wine-only ([game_state.rs:337](../../src-tauri/src/commands/game_state.rs#L337)) | 🔴 |
| `restore_mod_snapshot` | ✅ | 🔴 Wine-only ([collections.rs:892](../../src-tauri/src/commands/collections.rs#L892)) | 🔴 |
| `return_to_vanilla` | ✅ | 🔴 Wine-only | 🔴 |
| `rollback_mod_version` | ✅ | 🔴 Wine-only ([game_state.rs:274](../../src-tauri/src/commands/game_state.rs#L274)) | 🔴 |
| Native bundle snapshot (codesign-aware) | n/a | `rollback::create_native_snapshot` invoked by SMAPI install + Paralives BepInEx | ✅ native-only feature |

---

## 9. Settings page

| Section | Wine `/settings` | Native `/native/settings` | Status |
|---|---|---|---|
| Wine bottle settings | ✅ | n/a | ❌ |
| Bottle property editor | ✅ | n/a | ❌ |
| NM OAuth sign-in | ✅ | ✅ duplicated inline ([+page.svelte:60-200](../../src/routes/native/settings/+page.svelte#L60)) | 🟡 TODO comment to extract shared `NexusAccountSection.svelte` |
| API-key fallback | ✅ | ✅ | ✅ |
| Rescan games | ✅ | ✅ `rescanNativeGames()` | ✅ |
| BepInEx (Paralives) install/uninstall + consent | n/a | ✅ Layer 3 install + signature-strip warning ([+page.svelte:269-305](../../src/routes/native/settings/+page.svelte#L269)) | ✅ native-only feature |
| BG3SE consent / install | n/a | 🔴 `get_bg3se_status` is read-only; no install/uninstall cmd | 🔴 backend in [bg3se.rs](../../src-tauri/src/bg3se.rs) only does `detect()` |
| SMAPI install/uninstall | n/a | 🔴 `smapi::install` + `smapi::uninstall` exist ([smapi.rs:115,229](../../src-tauri/src/smapi.rs#L115)) but **no Tauri command exposes them** | 🔴 |
| Storage paths viewer | ✅ | ✅ lists native games with "Open in Finder" | ✅ |
| Exit Native Mode | n/a | ✅ toggle | ✅ |
| Native-mode visibility flag | n/a | ✅ readme in /settings/about | ✅ |
| Update checker / appcast | ✅ | (shared) | ✅ |
| Diagnostics (Wine health) | ✅ | ❌ | ❌ |
| Discord / About | ✅ | partial — about not duplicated | 🟡 |
| Crash-log analyzer | ✅ Skyrim-specific | ❌ | ❌ Not applicable |

---

## 10. Cross-cutting infrastructure

| Subsystem | Works for native? | Evidence |
|---|---|---|
| **Hardlink deployment** | ✅ Yes for Stardew + BG3 + Paralives (same `deployer` helpers, hardlink-first with copy fallback) | per-plugin `deploy_native` impls |
| **Conflict resolver** | 🔴 No — `commands/deployment.rs` uses `resolve_game` exclusively | [deployment.rs:42-368](../../src-tauri/src/commands/deployment.rs#L42) |
| **FOMOD installer** | ❌ Not applicable (Bethesda XML installer format) | |
| **LOOT** | ❌ Not applicable | |
| **INI editor** (`ini_manager`) | 🔴 not exposed for native — backend is content-agnostic but commands gate on Wine resolve | |
| **Crash-log analyzer** | ❌ Not applicable (Skyrim NetScriptFramework / Crash Logger format) | |
| **Session tracker** | 🟡 logs sessions, but `pid: None` from `open` breaks duration tracking | [mods.rs:1779](../../src-tauri/src/commands/mods.rs#L1779) |
| **Download queue** | ✅ Shared — orthogonal to runtime | |
| **Background hashing** | ✅ Shared | |
| **Path safety (`is_safe_relative_path`)** | ✅ Used by all native deploy paths | each native plugin imports it |
| **DeployGuard RAII** | 🟡 not used by per-plugin `deploy_native` impls; they call snapshot manually but no `deploy_in_progress` flag is set | risk: launch button isn't blocked during native deploy |
| **Auto-snapshot before destructive** | ✅ for Stardew, Paralives, BG3 deploy paths | each plugin calls `rollback::create_snapshot` or `create_native_snapshot` |
| **NXM handler** | ✅ Shared | |
| **Verified lists** | 🔴 cmds gate on Wine resolve in places | spot-check needed |

---

## 11. Native-only surfaces (features Wine mode doesn't have)

| Feature | Where | Status |
|---|---|---|
| `NativeSource` discriminator (Steam/GOG/Mac App Store/Manual/Applications) | [runtime.rs:58](../../src-tauri/src/runtime.rs#L58) | ✅ used for launch routing |
| Mach-O architecture detection | [native_scanner.rs:49](../../src-tauri/src/native_scanner.rs#L49) | ✅ |
| Mac App Store sandbox refusal | `validate_manual_native_app` + each plugin's `deploy_native` checks `native.sandboxed` | ✅ |
| Apple Developer ID code-signing trust-boundary warnings | consent dialog in `/native/settings` for Paralives BepInEx | ✅ |
| `create_native_snapshot` (bundle-aware snapshot) | `rollback::create_native_snapshot` called by SMAPI + Paralives BepInEx install | ✅ |
| Liquid Glass variant ratcheting (`apply_native_window_effect`) | [native_cmds.rs:67](../../src-tauri/src/commands/native_cmds.rs#L67) | ✅ no-op on Linux/Windows |
| `steam://run/<id>` launch | [mods.rs:1748](../../src-tauri/src/commands/mods.rs#L1748) | ✅ |
| Per-mod `.mod/` container (v0.16.1) | most recent commit `7a80310` | ✅ |

---

## 12. What doesn't apply to native (and why)

| Feature | Why it's N/A |
|---|---|
| Bottle management UI | No bottle in native mode (`""` sentinel) |
| Wine registry tweaks | No wine prefix |
| CrossOver display / cursor fixes | No Wine compositor layer |
| Wine diagnostic / version pinning | No Wine binary |
| DXVK / shader conversion | Wine graphics layer concept |
| Depot downloader / downgrader | Wine-side game files; native games use Steam mac depot directly |
| FOMOD installer | Bethesda XML installer format |
| ESM/ESL/ESP load order | Bethesda-only file format |
| Plugins.txt / LOOT | Bethesda-only |
| Wabbajack | Targets Bethesda games on Windows; modlist authors do not publish for native macOS |
| SKSE / F4SE / OBSE / Engine Fixes for Wine | Bethesda script extenders |
| Crash log analyzer | Skyrim NetScriptFramework + Crash Logger SSE-specific format |
| Skyrim downgrade copy isolation | Wine-side Steam install copy |
| ModEngine2 | FromSoftware Wine setup |
| BG3SE (Windows) configuration | macOS uses dylib variant tracked in [bg3se.rs](../../src-tauri/src/bg3se.rs) instead |

---

## 13. Priority recommendations

1. **Wire SMAPI install/uninstall as Tauri commands** — Size: **S**.
   `smapi::install` + `smapi::uninstall` are implemented and tested
   ([smapi.rs:115,229](../../src-tauri/src/smapi.rs#L115)) but no
   `#[tauri::command]` exposes them. Stardew users currently can't enable
   modding through Corkscrew — even though deploy_native works, the SMAPI
   runtime install step is missing from the UI. Add commands analogous to
   `install_paralives_bepinex` in [native_cmds.rs:130](../../src-tauri/src/commands/native_cmds.rs#L130).
   Add SMAPI section to `/native/settings`.

2. **Wire `get_stardew_mod_status` to the real analyzer** — Size: **S**.
   The command is a stub returning `Vec::new()`
   ([stardew_cmds.rs:26](../../src-tauri/src/commands/stardew_cmds.rs#L26)).
   The analyzer (`parse_manifest` + `analyze_mod_status`) is complete.
   Add SMAPI dependency-warning UI to `/native/mods`.

3. **Add BG3SE install/uninstall commands + consent UI** — Size: **M**.
   [bg3se.rs](../../src-tauri/src/bg3se.rs) only ships `detect()`. Build
   the macOS-dylib install/uninstall and surface them in
   `/native/settings` matching the Paralives BepInEx flow. Otherwise BG3
   script mods are unreachable.

4. **Propagate `resolve_game_any_runtime` through deployment + game_state**
   — Size: **M**. Files: [commands/deployment.rs](../../src-tauri/src/commands/deployment.rs),
   [commands/game_state.rs](../../src-tauri/src/commands/game_state.rs),
   [commands/profiles.rs](../../src-tauri/src/commands/profiles.rs). 25+
   `resolve_game` call sites will fail with `Bottle '' not found` for
   native games. Mechanical change — each call needs to handle
   `Option<Bottle>`. Unblocks: conflict resolution, snapshots, rollback,
   redeploy, profiles, mod priority reorder, return-to-vanilla.

5. **Native load-order page for BG3** — Size: **M**. Backend complete
   (`get_file_based_load_order`/`set_file_based_load_order` +
   `LoadOrderFormat::Bg3ModSettings`). Build `/native/plugins` or extend
   `/native/mods` with a drag-reorder list. Without this, BG3 users can
   install mods but can't reorder them inside Corkscrew.

6. **Hook custom-executable + game-lock paths into native launch** — Size: **S**.
   [mods.rs:1740-1783](../../src-tauri/src/commands/mods.rs#L1740) returns
   before custom-exe lookup; PID isn't captured because `open` detaches,
   so the game-lock never fires. Either (a) launch the bundle binary
   directly when no Steam integration is needed, or (b) poll for the
   spawned PID via `pgrep`. Without this, destructive mod operations
   during native play are not blocked.

7. **Drag-drop archive install on `/native/mods`** — Size: **S**.
   `install_mod_cmd` already supports native via `resolve_game_any_runtime`.
   Add the drop-zone component used by Wine `/mods` (already a shared
   building block).

8. **Unblock Crimson Desert deploy** — Size: **L** (research, not code).
   [crimson_desert_native.rs:210](../../src-tauri/src/plugins/crimson_desert_native.rs#L210)
   intentionally returns `Err` pending verification of PAZ-overlay
   location (`<game_install>/Paz/` vs inside `.app/Contents/Resources/`).
   Need a real install to verify. Spec at
   `docs/superpowers/plans/2026-05-02-crimson-desert-native-spec.md §10.5`.

9. **Extract `NexusAccountSection.svelte`** — Size: **XS**. The TODO
   comment at [native/settings/+page.svelte:59](../../src/routes/native/settings/+page.svelte#L59)
   already names the target. Roughly 140 duplicated lines.

10. **DeployGuard for native deploy paths** — Size: **S**. Per-plugin
    `deploy_native` impls don't take the RAII guard, so
    `deploy_in_progress` isn't set during native install. The launch
    button doesn't get blocked, and concurrent installs are theoretically
    possible. Wrap the dispatch in [deployer.rs:1695](../../src-tauri/src/deployer.rs#L1695)
    with a `DeployGuard` instance.
