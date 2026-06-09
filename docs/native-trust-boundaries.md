# Native macOS Modding — Trust Boundaries

This document describes exactly what Corkscrew touches when it manages mods for macOS-native games, why each mutation is necessary, and how to revert.

## Stardew Valley (SMAPI)

### What Corkscrew touches

When SMAPI is installed via Corkscrew (`smapi::install`), the following files inside `<Stardew Valley.app>/Contents/MacOS/` are added or modified:

| Path | Action | Reason |
|---|---|---|
| `StardewValley` | Renamed to `StardewValley-original` | Vanilla launcher set aside |
| `StardewValley` (new) | Created — SMAPI's `unix-launcher.sh` renamed | Loads SMAPI runtime + mods |
| `StardewModdingAPI` | Added | Main SMAPI executable |
| `StardewModdingAPI.dll` | Added | SMAPI runtime DLL |
| `Stardew Valley.deps.json` | Added from SMAPI `install.dat` payload when present | Source deps file used by the installer step below |
| `StardewModdingAPI.deps.json` | Added — copy of `Stardew Valley.deps.json` | .NET dependency resolution |
| `smapi-internal/` | Created — directory with SMAPI's runtime files | Internal SMAPI state |
| `Mods/` | Created if missing | Mod load location |

The payload copy recursively writes every top-level entry from SMAPI's nested `install.dat` into `Contents/MacOS/` **except** `mcs/` and `Mods/`. In a real SMAPI release this includes additional SMAPI-managed files beside the main executable/DLL pair (for example `StardewModdingAPI.runtimeconfig.json`, `StardewModdingAPI.xml`, `steam_appid.txt`, and files under `smapi-internal/`). Corkscrew does not currently enumerate or uninstall every copied payload sidecar; see the implementation backlog below.

The installer's bundled `Mods/` entries are currently **excluded** during Corkscrew's payload copy. That differs from the SMAPI spike/spec, which expected bundled mods to be copied after creating `Mods/`. This is a known behavior gap, not an intentional trust-boundary expansion.

When mods are deployed (`StardewValleyNativePlugin::deploy_native`), mod files are copied (hardlink-first) into `Contents/MacOS/Mods/<mod-name>/`.

### What Corkscrew does NOT touch

- `Contents/Resources/`
- `Contents/Info.plist`
- `Contents/_CodeSignature/` (already invalidated by SMAPI's edits; Corkscrew does not actively delete it)
- Anything in `~/Library/Application Support/StardewValley/` (saves, settings)

### Code signing

SMAPI's installer modifies `Contents/MacOS/`, which invalidates the bundle's Apple Developer ID signature. This is intentional and expected — SMAPI itself ships unsigned. Gatekeeper prompts the user once after modification; subsequent launches work without prompt.

Corkscrew does NOT re-sign the bundle. If a user wants to ad-hoc sign:

```
codesign --deep --force -s - "/Applications/Stardew Valley.app"
```

This is community-documented — not a Corkscrew workflow step.

### Reverting

Run `smapi::uninstall(&app_bundle, &db)`. The procedure:

1. Delete the SMAPI launcher script (`Contents/MacOS/StardewValley`)
2. Rename `StardewValley-original` back to `StardewValley`
3. Delete `StardewModdingAPI`, `StardewModdingAPI.dll`, `StardewModdingAPI.deps.json`, `smapi-internal/`
4. Preserves `Mods/` (user mods are intentionally kept — consistent with SMAPI's own uninstaller behavior)

Current limitation: because install copies most non-`mcs`/non-`Mods` payload sidecars but uninstall only removes the files listed in step 3 plus `smapi-internal/`, sidecars such as `StardewModdingAPI.runtimeconfig.json`, `StardewModdingAPI.xml`, `steam_appid.txt`, and the copied `Stardew Valley.deps.json` may remain after uninstall. Track this in the backlog below before claiming a fully clean SMAPI uninstall.

The byte-equal round-trip property is pinned by the `uninstall_restores_vanilla_launcher` test in `smapi.rs`.

## Baldur's Gate 3

### What Corkscrew touches

| Path | Action | Reason |
|---|---|---|
| `~/Documents/Larian Studios/Baldur's Gate 3/Mods/<file>.pak` | Added | Mod content |
| `~/Documents/Larian Studios/Baldur's Gate 3/PlayerProfiles/Public/modsettings.lsx` | Read + edit | Load order |

**The `.app` bundle itself is NOT modified by BG3 mod deployment.** BG3 mods live entirely outside the bundle in `~/Documents/`. Bundle signing is preserved unless the user separately installs BG3SE or another third-party bundle-level hook outside Corkscrew.

The `modsettings.lsx` edit upserts a `<node id="ModuleShortDesc">` block for each mod's UUID. The current implementation ensures the three known Larian master entries (`GustavDev`, `Gustav`, `SharedDev`) are present before writing and filters them out of the user-editable load-order UI. If `modsettings.lsx` does not exist, Corkscrew bootstraps it with those entries.

Current limitation: the bootstrap path uses `Version64 = 36028797018963968` for all three masters. The BG3 spike documented distinct versions for `Gustav` (`36029301681017806`) and `SharedDev` (`36028797022722506`). Existing real `modsettings.lsx` files preserve their parsed values, but newly bootstrapped files should be fixed before BG3 native deploy is treated as production-ready.

### What Corkscrew does NOT touch

- `Baldur's Gate 3.app` and any of its contents
- Save games (`PlayerProfiles/<profile>/Savegames/`)
- Profile-level config (`PlayerProfiles/<profile>/profile.lsf`)

### BG3 Script Extender

If BG3SE is installed (community macOS port), it places a `.dylib` in `Contents/MacOS/`. Corkscrew has a read-only helper (`bg3se::detect` in `bg3se.rs`) that can detect `.dylib` variants and flag a Windows-only `DWrite.dll`, but BG3's native game plugin still has a stub `detect_native()` and does not currently surface BG3SE status as part of functional BG3 game detection. Corkscrew does **NOT INSTALL** BG3SE.

### Reverting

**TODO: align** — A Corkscrew uninstall command for BG3 mods is not yet implemented. Manual revert:

1. Delete the mod's `.pak` from `~/Documents/Larian Studios/Baldur's Gate 3/Mods/`
2. Edit `modsettings.lsx` to remove the `<node id="ModuleShortDesc">` block matching the mod's UUID
3. Always preserve master entries (`GustavDev`, `Gustav`, `SharedDev`)

This will be automated in a follow-up task.

## Paralives (BepInEx)

### What Corkscrew touches

Layer 1 detection (`paralives_bepinex::detect`) is read-only. It checks the native game install directory for `BepInEx/`, `doorstop_config.ini`, `run_bepinex.sh`, `BepInEx/core/BepInEx.Core.dll`, a version marker, and a macOS doorstop `.dylib`.

Layer 2 deployment routes staged mod files by shape:

| Path | Action | Reason |
|---|---|---|
| `<Paralives.app>/Contents/MacOS/BepInEx/plugins/<mod_name>/*.dll` | Added when BepInEx is already detected and mac-supported | BepInEx script plugin load path |
| `~/Library/Application Support/com.Paralives.Paralives/Mods/<file>` | Added | Paralives data mod load path |

Layer 3 install (`paralives_bepinex::install_latest` / `install_from_archive`) is opt-in and mutates the native app trust boundary:

| Path | Action | Reason |
|---|---|---|
| `<Paralives.app>/Contents/MacOS/BepInEx/` | Added from BepInEx 6 IL2CPP macOS ARM64/universal release | Loader runtime |
| `<Paralives.app>/Contents/MacOS/doorstop_config.ini` | Added | Doorstop loader config |
| `<Paralives.app>/Contents/MacOS/run_bepinex.sh` | Added when present in release | Launch helper |
| `<Paralives.app>` code signature | `codesign --remove-signature` on macOS | Allows Doorstop/BepInEx code injection |

### Code signing

Installing BepInEx invalidates Paralives Studio's Apple signature. Corkscrew cannot restore that signature because only Paralives Studio has the certificate. Uninstall removes the BepInEx files Corkscrew knows about, but the signature remains removed/ad-hoc until the user runs Steam "Verify integrity of game files" or reinstalls Paralives.

UI must present this as an explicit experimental consent path before calling install: BepInEx enables arbitrary `.dll` script mods, modifies the app's signature, may trigger Gatekeeper on first launch, and may break after game/BepInEx updates.

### Reverting

Run `paralives_bepinex::uninstall(&game_install_dir, &app_bundle, &db)` to delete the BepInEx tree and loader files. Then run Steam Verify/reinstall to restore the original signed app bundle.

## Sandboxed games

Corkscrew refuses to mod sandboxed bundles outright. A bundle is considered sandboxed if any of these are true:

- It contains `Contents/_MASReceipt/receipt` (Mac App Store)
- Its path starts with `/System/Applications/` (Apple system apps)

The refusal is enforced at:

- `smapi::install` and `smapi::uninstall` (via `native_scanner::is_sandboxed`)
- `BaldursGate3NativePlugin::deploy_native_inner` (checks `native_ctx.sandboxed`)
- `StardewValleyNativePlugin::deploy_native` (checks `native_ctx.sandboxed`)
- `ParalivesNativePlugin::deploy_native` and `paralives_bepinex::install*`/`uninstall` (checks app bundle sandbox status)
- `native_scanner::validate_manual_native_app` (explicit sandboxed check with user-readable error)

Scanner note: `native_scanner::scan_dir` currently uses a strict `LSApplicationCategoryType` filter and skips bundles without a game category. This differs from the original native-mode plan, which said category should be preferred but not required. Steam scanning is separate and still discovers `.app` bundles under Steam libraries.

## Snapshots

Before any destructive native operation, Corkscrew calls `rollback::create_native_snapshot(db, game_id, name, desc)` to capture the relevant state. The call is best-effort: a snapshot failure is logged via `log::warn` and does **NOT** abort the operation.

Snapshot calls (all implemented as of Task 6.1 / commit `85e4307`):

| Operation | Snapshot name |
|---|---|
| SMAPI install | `smapi-install` |
| SMAPI uninstall | `smapi-uninstall` |
| Stardew deploy | `stardew-deploy` |
| BG3 deploy | `bg3-deploy` |
| Paralives BepInEx install | `paralives-bepinex-install` |
| Paralives BepInEx uninstall | `paralives-bepinex-uninstall` |
| Paralives deploy | `paralives-deploy` |

Snapshots are stored in the SQLite-backed `mod_snapshots` table (game_id + empty bottle_name sentinel for native games). Restore is exposed via the existing rollback UI.

**Note on snapshot scope:** `create_native_snapshot` records mod-list state from the database (enabled/disabled mods, load order). It does NOT copy raw file bytes from `Contents/MacOS/` or `~/Documents/`. The vanilla launcher byte-equality guarantee is provided by `smapi::uninstall`'s deterministic revert logic, not by snapshot file content.

## Threat model

Corkscrew's native mode is designed for trusted-user workflows. The following are NOT defended against:

- Maliciously crafted `.pak` or mod archives (Corkscrew reads metadata from them; BG3SE-using mods can run arbitrary code at game launch)
- Maliciously crafted `manifest.json` (parser is lenient; missing required fields produce typed errors, but malicious field values are not sanitized beyond what serde_json provides)
- Tampering with installed mods after deploy (Corkscrew does not continuously verify post-deploy state)

These align with how Wine modding has always been treated. Mod managers do not sandbox the mods themselves — that is the game runtime's responsibility.

## Implementation backlog: native trust-boundary mismatches

1. **SMAPI payload manifest parity**
   - Decide whether Corkscrew should exactly mirror SMAPI's bundled-mod behavior.
   - If yes: copy bundled `Mods/` entries after creating `Contents/MacOS/Mods/`, add tests for expected bundled mod names, and document which bundled mods are Corkscrew-managed vs user-managed.
   - If no: keep excluding bundled `Mods/`, but document the user-facing impact and add an explicit compatibility note in the SMAPI installer UI.

2. **SMAPI uninstall completeness**
   - Build an allowlist/manifest of every payload file copied from `install.dat` during install.
   - Uninstall should remove Corkscrew/SMAPI-managed sidecars copied from that manifest while preserving `Mods/` and user-created files.
   - Add round-trip tests covering `StardewModdingAPI.runtimeconfig.json`, `StardewModdingAPI.xml`, `steam_appid.txt`, copied `Stardew Valley.deps.json`, and nested `smapi-internal/` content.

3. **BG3 native detection**
   - Replace `BaldursGate3NativePlugin::detect_native()`'s `Vec::new()` scaffold with scanner-backed detection.
   - Filter by confirmed macOS bundle identifier/executable names and Steam app id, populate `GameRuntime::Native`, and add tests mirroring the Stardew native detection suite.
   - Only then should docs/UI claim functional BG3 native game detection.

4. **BG3 master bootstrap correctness**
   - Update `bootstrap_master_entries()` to use the documented per-master `Version64` values or, preferably, bootstrap from a checked-in known-good fixture.
   - Fix missing-master insertion so the final order is deterministic (`GustavDev`, `Gustav`, `SharedDev`, then community mods); repeated `insert(0, ...)` can reverse missing masters depending on which entries are absent.
   - Add tests for missing-all and missing-some master cases.

5. **Native scanner category policy**
   - Reconcile `native_scanner::scan_dir` with the original plan: either keep strict `LSApplicationCategoryType` filtering and document the compatibility tradeoff, or include non-game-category bundles and let per-game plugins filter.
   - Add tests for no-category and non-game-category bundles so the chosen trust boundary is explicit.

6. **Deployer trust-boundary wording**
   - Keep `deploy_game`/`deploy_native_game` comments aligned with the current dispatcher behavior: the dispatcher is implemented, while unsupported native games fail via the default per-plugin `deploy_native` error.
