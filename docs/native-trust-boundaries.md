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
| `StardewModdingAPI.deps.json` | Added — copy of `Stardew Valley.deps.json` | .NET dependency resolution |
| `smapi-internal/` | Created — directory with SMAPI's runtime files | Internal SMAPI state |
| `Mods/` | Created if missing | Mod load location |

The `mcs/` directory and the installer's bundled `Mods/` entries are explicitly **excluded** during the payload copy — only the files above are written.

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

The byte-equal round-trip property is pinned by the `uninstall_restores_vanilla_launcher` test in `smapi.rs`.

## Baldur's Gate 3

### What Corkscrew touches

| Path | Action | Reason |
|---|---|---|
| `~/Documents/Larian Studios/Baldur's Gate 3/Mods/<file>.pak` | Added | Mod content |
| `~/Documents/Larian Studios/Baldur's Gate 3/PlayerProfiles/Public/modsettings.lsx` | Read + edit | Load order |

**The `.app` bundle itself is NOT modified.** BG3 mods live entirely outside the bundle in `~/Documents/`. Bundle signing is preserved.

The `modsettings.lsx` edit upserts a `<node id="ModuleShortDesc">` block for each mod's UUID. The three mandatory Larian master entries (`GustavDev`, `Gustav`, `SharedDev`) are always preserved and placed before community mod entries. If `modsettings.lsx` does not exist, Corkscrew bootstraps it with the master entries.

### What Corkscrew does NOT touch

- `Baldur's Gate 3.app` and any of its contents
- Save games (`PlayerProfiles/<profile>/Savegames/`)
- Profile-level config (`PlayerProfiles/<profile>/profile.lsf`)

### BG3 Script Extender

If BG3SE is installed (community macOS port), it places a `.dylib` in `Contents/MacOS/`. Corkscrew **DETECTS** but does **NOT INSTALL** BG3SE. Detection is read-only (`bg3se::detect` in `bg3se.rs`).

### Reverting

**TODO: align** — A Corkscrew uninstall command for BG3 mods is not yet implemented. Manual revert:

1. Delete the mod's `.pak` from `~/Documents/Larian Studios/Baldur's Gate 3/Mods/`
2. Edit `modsettings.lsx` to remove the `<node id="ModuleShortDesc">` block matching the mod's UUID
3. Always preserve master entries (`GustavDev`, `Gustav`, `SharedDev`)

This will be automated in a follow-up task.

## Sandboxed games

Corkscrew refuses to mod sandboxed bundles outright. A bundle is considered sandboxed if any of these are true:

- It contains `Contents/_MASReceipt/receipt` (Mac App Store)
- Its path starts with `/System/Applications/` (Apple system apps)

The refusal is enforced at:

- `smapi::install` and `smapi::uninstall` (via `native_scanner::is_sandboxed`)
- `BaldursGate3NativePlugin::deploy_native_inner` (checks `native_ctx.sandboxed`)
- `StardewValleyNativePlugin::deploy_native` (checks `native_ctx.sandboxed`)
- `native_scanner::validate_manual_native_app` (explicit sandboxed check with user-readable error)

## Snapshots

Before any destructive native operation, Corkscrew calls `rollback::create_native_snapshot(db, game_id, name, desc)` to capture the relevant state. The call is best-effort: a snapshot failure is logged via `log::warn` and does **NOT** abort the operation.

Snapshot calls (all implemented as of Task 6.1 / commit `85e4307`):

| Operation | Snapshot name |
|---|---|
| SMAPI install | `smapi-install` |
| SMAPI uninstall | `smapi-uninstall` |
| Stardew deploy | `stardew-deploy` |
| BG3 deploy | `bg3-deploy` |

Snapshots are stored in the SQLite-backed `mod_snapshots` table (game_id + empty bottle_name sentinel for native games). Restore is exposed via the existing rollback UI.

**Note on snapshot scope:** `create_native_snapshot` records mod-list state from the database (enabled/disabled mods, load order). It does NOT copy raw file bytes from `Contents/MacOS/` or `~/Documents/`. The vanilla launcher byte-equality guarantee is provided by `smapi::uninstall`'s deterministic revert logic, not by snapshot file content.

## Threat model

Corkscrew's native mode is designed for trusted-user workflows. The following are NOT defended against:

- Maliciously crafted `.pak` or mod archives (Corkscrew reads metadata from them; BG3SE-using mods can run arbitrary code at game launch)
- Maliciously crafted `manifest.json` (parser is lenient; missing required fields produce typed errors, but malicious field values are not sanitized beyond what serde_json provides)
- Tampering with installed mods after deploy (Corkscrew does not continuously verify post-deploy state)

These align with how Wine modding has always been treated. Mod managers do not sandbox the mods themselves — that is the game runtime's responsibility.
