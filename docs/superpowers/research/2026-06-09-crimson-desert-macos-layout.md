# Crimson Desert macOS Modding Layout — Research Findings
*2026-06-09 · Web research, no real install verification*

---

## Confidence summary

| # | Question | Confidence | Rationale |
|---|----------|------------|-----------|
| Q1 | CFBundleIdentifier same as App Store? | STRONG EVIDENCE | Steam crash dump shows `com.pearlabyss.CrimsonDesert.steam`; App Store almost certainly `com.pearlabyss.CrimsonDesert` — different suffixes |
| Q2 | Executable name inside Contents/MacOS/ | STRONG EVIDENCE | Steam crash dump explicitly names `CrimsonDesert_Steam` at that path |
| Q3 | .app is game root or launcher? | STRONG EVIDENCE | CDUMM MACOS.md: `.app` is game root, packages at `Contents/Resources/packages/`; no separate launcher |
| Q4 | Vanilla ships 0000–0035, 0036 is first mod slot? | CONFIRMED | NattKh MODDING_GUIDE.md: "Vanilla groups span 0000 through 0035. Custom overlays occupy 0036 and higher" |
| Q5 | PAZ overlay path (safe or inside .app)? | STRONG EVIDENCE | CDUMM + Enki013 both place overlays inside `.app/Contents/Resources/packages/`; code-signing caveat noted |
| Q6 | meta/0.papgt non-numeric group names? | CONFIRMED | MODDING_GUIDE: "PAPGT loader silently ignores overlay groups with non-numeric names" — confirmed empirically 2026-04-21 |
| Q7 | Denuvo behavior on Apple Silicon | CONFIRMED | macOS build ships WITHOUT Denuvo — confirmed by BiGMAC crack (March 2026) and multiple news sources |
| Q8 | User-level `~/Library/Application Support/Pearl Abyss/CD/mods/` path? | WEAK EVIDENCE | Save files at `~/Library/Application Support/Pearl Abyss/CD/save` (Steam) confirmed; no evidence of a separate `mods/` subdirectory |

---

## Per-question detailed findings

### Q1: CFBundleIdentifier

**STRONG EVIDENCE — Steam and App Store use different bundle IDs.**

A Steam Community crash dump for App 3321460 (June 1, 2026) shows:
> `"com.pearlabyss.CrimsonDesert.steam"` with version `1.0.0 (203)`

The App Store listing (id6747100856) does not display the bundle ID directly, but the standard Apple convention for a game published under both storefronts assigns `.steam` suffix to the Steam build. The App Store version almost certainly uses `com.pearlabyss.CrimsonDesert` (no suffix).

Sources:
- [Steam Community crash thread](https://steamcommunity.com/app/3321460/discussions/0/653730358120080524/)
- [App Store listing](https://apps.apple.com/us/app/crimson-desert/id6747100856)

**Working assumption for Corkscrew**: Corkscrew targets the Steam build. Use `com.pearlabyss.CrimsonDesert.steam` for Steam, `com.pearlabyss.CrimsonDesert` for App Store. The spec's hypothesis of `com.pearlabyss.CrimsonDesert` is correct only for the App Store build.

---

### Q2: Executable name inside Contents/MacOS/

**STRONG EVIDENCE: `CrimsonDesert_Steam`**

The same Steam crash dump specifies the full path:
> `/Users/USER/Library/Application Support/Steam/.../CrimsonDesert_Steam.app/Contents/MacOS/CrimsonDesert_Steam`

Note: this also reveals the `.app` bundle name appears to be `CrimsonDesert_Steam.app` for the Steam build (vs. `Crimson Desert.app` by display name — likely an alias).

Sources:
- [Steam Community crash thread](https://steamcommunity.com/app/3321460/discussions/0/653730358120080524/)

---

### Q3: .app structure — launcher or game root?

**STRONG EVIDENCE: `.app` IS the game root. Packages live inside at `Contents/Resources/packages/`.**

CDUMM MACOS.md states:
> "The `Crimson Desert.app` bundle itself is the game root on macOS. There is no separate launcher—the `.app` is the complete game installation."

The inner validation check looks for `0008/0.paz` and `meta/0.papgt` within `Contents/Resources/packages/`.

The spec's §8 working hypothesis that ".app is only a launcher; mods deploy to game_root outside the bundle" appears to be INCORRECT based on this evidence. All PAZ data — including mod overlays — resides inside the `.app` bundle at `Contents/Resources/packages/`.

Sources:
- [CDUMM MACOS.md](https://github.com/faisalkindi/CrimsonDesert-UltimateModsManager/blob/master/MACOS.md)

---

### Q4: Vanilla groups 0000–0035; 0036 is first mod slot

**CONFIRMED.**

NattKh MODDING_GUIDE.md explicitly states:
> "Vanilla groups span `0000` through `0035`. Custom overlays occupy `0036` and higher numeric designations."

The reference implementation uses `0036/0037` for folder exports and `0062/0063` for stacked applies.

Sources:
- [NattKh MODDING_GUIDE.md](https://github.com/NattKh/CRIMSON-DESERT-SAVE-EDITOR-AND-GAME-MODS/blob/main/CrimsonGameMods/MODDING_GUIDE.md)

---

### Q5: PAZ overlay path — inside `.app/Contents/Resources/` (breaks signing)

**STRONG EVIDENCE: Overlays go INSIDE the .app bundle, at `Contents/Resources/packages/0036/`.**

Multiple independent sources converge on this:

1. **Enki013 JSON Mod Manager (macOS-explicit tool)**: Outputs `0036/0.paz` and `0036/0.pamt` relative to the game packages path, which is `Contents/Resources/packages/`.
2. **CDUMM MACOS.md**: Mod storage is at `<Crimson Desert.app>/Contents/Resources/packages/CDMods/`. Notes: *"putting CDMods/ inside the .app bundle invalidates the bundle's code signature,"* but accepts this because macOS does not re-verify previously-launched apps.
3. **faisalkindi CDUMM README**: "CDUMM walks into `Crimson Desert.app` to find the inner `Contents/Resources/packages/` directory automatically."

This is the CRITICAL finding for Q5: the path is INSIDE the bundle (`Contents/Resources/packages/`), not at a sibling `Paz/` directory. This means Corkscrew must handle the code-signing invalidation the same way CDUMM does (accept it; macOS won't block launch of a previously-run app).

Sources:
- [CDUMM MACOS.md](https://github.com/faisalkindi/CrimsonDesert-UltimateModsManager/blob/master/MACOS.md)
- [Enki013 macOS Mod Manager](https://github.com/Enki013/Crimson-Desert-JSON-Mod-Manager-MacOS)

---

### Q6: meta/0.papgt non-numeric group names

**CONFIRMED: No non-numeric names exist or function.**

NattKh MODDING_GUIDE.md:
> "The PAPGT loader silently ignores overlay groups with non-numeric names."
> "A group named `stk1`, `mymod`, `patch_v2` will be ignored — confirmed empirically on 2026-04-21."

meta/0.papgt entries require: `group_name` (numeric string), `pack_meta_checksum`, `language` (typically `0x3FFF`), `is_optional` boolean.

Sources:
- [NattKh MODDING_GUIDE.md](https://github.com/NattKh/CRIMSON-DESERT-SAVE-EDITOR-AND-GAME-MODS/blob/main/CrimsonGameMods/MODDING_GUIDE.md)

---

### Q7: Denuvo on Apple Silicon

**CONFIRMED: macOS build has NO Denuvo.**

Multiple news outlets (March 2026) confirmed that Pearl Abyss did not integrate Denuvo into the macOS builds distributed via App Store and Steam. The hacker group BiGMAC cracked it trivially as a result. PC (Windows) has Denuvo; macOS does not.

**Impact on modding**: No Denuvo on macOS means no DRM interference with PAZ overlay loading, no anti-tamper hooks that might detect modified bundles, and no performance overhead. Modding is significantly safer on the Mac build than the Windows build.

Sources:
- [en.gamegpu.com — pirates cracked Mac version](https://en.gamegpu.com/news/igry/piraty-vzlomali-versiyu-crimson-desert-dlya-mac)
- [insider-gaming.com — Denuvo DRM status](https://insider-gaming.com/crimson-desert-denuvo-drm/)

---

### Q8: User-level `~/Library/Application Support/Pearl Abyss/CD/mods/`

**WEAK EVIDENCE: Save path confirmed; mods subdir unconfirmed.**

Save files are documented at:
- Steam: `~/Library/Application Support/Pearl Abyss/CD/save`
- App Store: `~/Library/Containers/com.pearlabyss.CrimsonDesert/Data/Library/Application Support/Pearl Abyss/CD/save`

No source mentions a `mods/` subdirectory under `Pearl Abyss/CD/`. All community tools write overlays directly into the `.app` bundle's `Contents/Resources/packages/` rather than a user-level mod directory.

**Working assumption**: No user-level mod path exists. Pearl Abyss has not provided an official modding SDK or user mod directory. Mods must be injected into the bundle.

Sources:
- [xmodhub.com — save file locations](https://www.xmodhub.com/info/blog/crimson-desert-save-file-location/)

---

## Recommendation for deploy code

| Item | Best-guess value | Confidence |
|------|-----------------|------------|
| VERIFIED const | **false** — use runtime detection, not a hardcoded path |
| PAZ overlay path | `<app_bundle>/Contents/Resources/packages/0036/` | Strong evidence |
| Bundle ID (Steam) | `com.pearlabyss.CrimsonDesert.steam` | Strong evidence |
| Bundle ID (App Store) | `com.pearlabyss.CrimsonDesert` | Inferred |
| Executable (Steam) | `CrimsonDesert_Steam` | Strong evidence |
| Code-signing impact | Expected and acceptable — write to bundle, accept invalidation | Confirmed by CDUMM |
| Denuvo concern | None on macOS | Confirmed |

**Spec §8 correction needed**: The spec's working hypothesis that ".app is only a launcher; mods deploy to game_root outside the bundle" is contradicted by strong evidence. The `.app` bundle IS the game root on macOS, and mods deploy to `Contents/Resources/packages/0036+` inside the bundle. Corkscrew should handle the `App Management` permission prompt (System Settings → Privacy & Security → App Management) that macOS will show when writing into another `.app`.

---

## Manual verification procedure

When the user installs Crimson Desert on macOS, run these commands to verify all assumptions in under 5 minutes:

```bash
# 1. Locate the Steam .app bundle
CDAPP=$(find ~/Library/Application\ Support/Steam/steamapps/common -name "*.app" -maxdepth 2 | grep -i "crimson" | head -1)
echo "App bundle: $CDAPP"

# 2. Confirm executable name and bundle ID
/usr/libexec/PlistBuddy -c "Print CFBundleIdentifier" "$CDAPP/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Print CFBundleExecutable" "$CDAPP/Contents/Info.plist"

# 3. Verify PAZ packages are inside the bundle
ls "$CDAPP/Contents/Resources/packages/" | head -20
# Expect: 0000 0001 ... 0035 meta (and no 0036 in vanilla)

# 4. Confirm vanilla top group and 0036 absence
ls "$CDAPP/Contents/Resources/packages/" | sort -n | tail -5
# Expect last numeric dir to be 0035; no 0036 present

# 5. Check for user-level mod directory
ls ~/Library/Application\ Support/Pearl\ Abyss/CD/ 2>/dev/null
# Expect: only 'save' subdir; no 'mods' subdir
```
