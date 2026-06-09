# Paralives Native macOS Support — Corkscrew Implementation Spec

**Spike date:** 2026-06-08
**Author:** Research spike (Claude Code)
**Status:** RESEARCH ONLY — no code written

---

## 1. Game Engine + Platform Facts

**Engine:** Unity (version not publicly disclosed by Paralives Studio; "Unity 6" is widely reported by third-party hardware coverage sites but not confirmed in official Paralives or Unity press releases). **INFERRED** from: the game executable layout, BepInEx hooking via `libdoorstop.dylib`, and the `.catalog` animation format (Unity's built-in animation packaging step). **TODO: verify with real install** by reading `Paralives.app/Contents/Resources/Data/boot.config` for the Unity version string.

**Scripting backend:** **IL2CPP** — confirmed indirectly by:
1. The 6ix Plugin Hub requiring BepInEx IL2CPP flavor (`BepInEx_macos_universal_5.4.23.5.zip`) ([source: 6xvl/paralives-plugins-index](https://github.com/6xvl/paralives-plugins-index)).
2. Community documentation describing a first-run "IL2CPP file generation" delay.
3. The BepInEx IL2CPP macOS install docs (doorstop 4.5.0, `run_bepinex.sh` + `libdoorstop.dylib`) match the exact install procedure documented for Paralives.
**TODO: verify with real install** by checking for `GameAssembly.dylib` (IL2CPP) vs `Assembly-CSharp.dll` (Mono) inside the .app bundle.

**macOS platform support:**
- Apple Silicon only (ARM64 native). Intel Macs explicitly not supported; no Rosetta path announced.
- Minimum: Apple M2. Recommended: Apple M3.
- Minimum OS: macOS Big Sur 11.
- Source: [Steam store page](https://store.steampowered.com/app/1118520/Paralives/)

**Steam App ID:** `1118520`
Source: URL `https://store.steampowered.com/app/1118520/Paralives/`

**CFBundleIdentifier:** No community source directly published the literal plist value. However, Unity's `Application.persistentDataPath` on macOS encodes as `~/Library/Application Support/<CompanyName>/<ProductName>`. Community sources confirm the macOS save/mod path is `~/Library/Application Support/com.Paralives.Paralives/`, which Unity generates from `Application.companyName = "Paralives"` and `Application.productName = "Paralives"` when the company name is formatted as a reverse-DNS identifier in Player Settings. **BEST GUESS: `com.Paralives.Paralives`** — this is also consistent with the `com.<CompanyName>.<ProductName>` Unity macOS convention.
**TODO: verify with real install** by running: `defaults read ~/Library/Application\ Support/Steam/steamapps/common/Paralives/Paralives.app/Contents/Info.plist CFBundleIdentifier`

**macOS executable name:** **INFERRED** as `Paralives` (the binary at `Paralives.app/Contents/MacOS/Paralives`). Unity games on macOS always name the binary after the `ProductName` in Player Settings, and the product name here is "Paralives". The BepInEx macOS install step `codesign --remove-signature Paralives.app` confirms the bundle is named `Paralives.app`.
**TODO: verify with real install** by running: `ls ~/Library/Application\ Support/Steam/steamapps/common/Paralives/Paralives.app/Contents/MacOS/`

---

## 2. Paralives' Official Modding Posture

Paralives Studio has **first-class, officially supported modding** built into the game at launch (Early Access, May 2026).

**In-game mod manager:** YES. Accessed via `Main Menu → Mods`. The manager allows creating new mods, toggling mods on/off, and clicking a folder icon to open the mod directory on disk. Mods are enabled/disabled within the game UI — Corkscrew would need to deploy files to the folder and optionally surface this toggle. Source: [Paralives Wiki — Creating a Mod](https://paralives.wiki.gg/wiki/Creating_a_Mod_and_Uploading_to_the_Steam_Workshop), [simscommunity.info modding tools reveal](https://simscommunity.info/2026/03/22/paralives-modding-tools/).

**Official mod folder:** YES — `AppData\LocalLow\Paralives\Paralives\Mods` (Windows). Mods go in a `Mods/` subdirectory within the Unity `persistentDataPath`.

**Script mods:** NOT officially supported. Official position: "Paradevs will not provide tools to create them, nor will they allow these mods on the Steam Workshop." They may be developed externally and shared on third-party platforms. Source: [Paralives Wiki — Moddable features](https://paralives.wiki.gg/wiki/Moddable_features).

**Steam Workshop:** YES — the primary official distribution channel. Subscribed Workshop items load from a separate Steam content folder, not the user's custom `Mods/` folder.

---

## 3. Nexus Mods Paralives — Inventory

Nexus Mods URL: `https://www.nexusmods.com/paralives` (confirmed live).

**Category counts (as of June 2026):**

| Category | Mod count |
|---|---|
| Gameplay | 38 |
| User Interface | 19 |
| Clothing | 15 |
| Paras (characters) | 15 |
| Miscellaneous | 7 |
| Utilities | 6 |
| Items | 4 |
| Houses and Lots | 1 |
| Furniture | 0 (separate listing) |
| **Total** | **~173 mods, 1 collection** |

Source: [Nexus Mods Paralives categories](https://www.nexusmods.com/paralives/mods/categories)

**Note:** Nexus Mods returns HTTP 403 to automated fetches (Cloudflare bot protection). The following representative mod inventory is assembled from search results and cached descriptions rather than direct page fetches. Each entry is labeled as directly observed or inferred.

### Representative Mod Sample

| # | Nexus URL | Category | Install path / format | macOS notes |
|---|---|---|---|---|
| 1 | nexusmods.com/paralives/mods/15 | Gameplay (BepInEx script mod) | `BepInEx/plugins/<mod>.dll` in game root | macOS: BepInEx macOS build required; `run_bepinex.sh` + `libdoorstop.dylib` |
| 2 | nexusmods.com/paralives/mods/22 | Utilities (ParalivesModTool) | Third-party mod manager tool | Windows-centric; **TODO: verify macOS support** |
| 3 | nexusmods.com/paralives/mods/80 | Miscellaneous (Paralives Optimisations) | **INFERRED** data mod to mods folder | **INFERRED** cross-platform |
| 4 | nexusmods.com/paralives/mods/127 | Utilities (Paralives Mod Manager) | In-game tool / UI extension | **INFERRED** data mod |
| 5 | nexusmods.com/paralives/mods/160 | Utilities (Mod Organizer) | BepInEx DLL script mod (sorts in-game mod list) | macOS BepInEx required |
| 6 | nexusmods.com/paralives/mods/117 | Gameplay (NeonJay MasterCore Mod Suite) | BepInEx plugins bundle | macOS BepInEx required |
| 7 | nexusmods.com/paralives/mods/21 | Clothing (Expanded Outfit Categories) | **INFERRED** data/asset mod | **INFERRED** cross-platform |
| 8 | nexusmods.com/paralives/mods/132 | Paras (Paramaker Plus — copy clothes) | **INFERRED** data mod | **INFERRED** cross-platform |
| 9 | nexusmods.com/paralives/mods/185 | Items / Furniture (Laundry Day Clutter) | **INFERRED** asset mod (3D + textures) | **INFERRED** cross-platform |
| 10 | nexusmods.com/paralives/mods/105 | Gameplay (Direct Control — WASD movement) | BepInEx DLL | macOS BepInEx required |

**Key observation from mod #15 (Cheat Mod):** Author's manual install instructions explicitly document a macOS path:
> "Download `BepInEx_macos_universal_5.4.23.5.zip`, extract into game root, run `chmod +x run_bepinex.sh`, then `xattr -dr com.apple.quarantine .`, then `codesign --remove-signature Paralives.app`, then set Steam Launch Options to `./run_bepinex.sh %command%`. Place mod `.dll` in `BepInEx/plugins/`."

Source: search result excerpt from nexusmods.com/paralives/mods/15 description.

---

## 4. Mod File Format Inference

**Two distinct mod ecosystems coexist:**

### A. Official "feature/asset" mods (data-only)

These use Paralives' built-in modding tools and are distributed as folders (extracted from `.zip` archives) placed in the `Mods/` directory under the Unity `persistentDataPath`. Archive contents include:

- **3D assets:** `.fbx`, `.obj` files
- **Textures:** `.png`, `.jpg`
- **Animations:** `.catalog` (Unity animation packages)
- **Audio:** `.ogg`, `.mp3`, `.wav`
- **Fonts:** `.ttf`
- **Text/config:** `.txt`, and likely `.json` (confirmed for the Traits mod which drives its UI from a JSON file)

These mods are **data-only — no executable code**. They work cross-platform because Unity loads them via the game's own asset pipeline. No binary translation is needed for macOS ARM64.

Source: [Paralives Wiki — Creating a Mod](https://paralives.wiki.gg/wiki/Creating_a_Mod_and_Uploading_to_the_Steam_Workshop), [simscommunity.info modding tools reveal](https://simscommunity.info/2026/03/22/paralives-modding-tools/)

### B. Unofficial BepInEx script mods

These ship as `.dll` assemblies targeting `netstandard2.0`, placed in `BepInEx/plugins/` within the **game's Steam installation directory** (not the `persistentDataPath`). BepInEx injects them via Doorstop at startup.

- **Are these executable code?** YES — `.dll` assemblies compiled for .NET Standard 2.0.
- **Do they require a loader?** YES — BepInEx 5 (IL2CPP flavor) with Doorstop 4.5.0.
- **macOS status:** A BepInEx macOS build exists (`run_bepinex.sh` + `libdoorstop.dylib`). The 6ix Plugin Hub modpack explicitly states "Same zip now works on Windows, Steam Deck/Linux, and Mac." However, macOS support is documented as **experimental**.
- **ARM64 complication:** BepInEx 5.x stable provides only x86_64 macOS builds. ARM64 (Apple Silicon) is only in BepInEx 6.x (still in development/experimental). Since Paralives requires M2 minimum (ARM64), the BepInEx situation on Apple Silicon is **unconfirmed working** — the game's ARM64 build may run BepInEx x86_64 under Rosetta (which Steam can invoke), or BepInEx 6.x may be required. **TODO: verify with real install.**
- **No mods explicitly state "Windows only"** in the sources retrieved, but no source explicitly confirms BepInEx DLL mods working natively on Apple Silicon either.

Source: [6xvl/paralives-plugins-index](https://github.com/6xvl/paralives-plugins-index), [6xvl releases page](https://github.com/6xvl/paralives-plugins-index/releases)

---

## 5. macOS Install Paths

### Primary mod path (official mods)

Unity's `Application.persistentDataPath` on macOS resolves to:

```
~/Library/Application Support/com.Paralives.Paralives/
```

The `Mods/` subdirectory within this is where official/data mods are placed:

```
~/Library/Application Support/com.Paralives.Paralives/Mods/
```

This is confirmed by multiple community sources:
- Steam discussion thread (cross-platform saves): confirms `~/Library/Application Support/com.Paralives.Paralives/MySavedGames.mod` (source: [steamcommunity.com](https://steamcommunity.com/app/1118520/discussions/0/840629289174964929/))
- paralivesmod.com guide: `~/Library/Application Support/Paralives/Mods` (slightly different — **INFERRED** this may omit the `com.` prefix; the persistent data path form is more authoritative)
- Multiple guides reference `~/Library/Application Support/com.Paralives.Paralives/` as the base

**Reconciling path discrepancies:** Three variants appeared in sources:
1. `~/Library/Application Support/com.Paralives.Paralives/` — matches Unity's `persistentDataPath` formula for `companyName="Paralives"`, `productName="Paralives"` with a reverse-DNS company name. Most likely correct.
2. `~/Library/Application Support/Paralives/` — may be a simplified community reference omitting the `com.` prefix, or may reflect an older build. **INFERRED** secondary candidate.
3. `~/Library/Application Support/Paralives Studio/Paralives/Saves` — appeared in one source for saves; may reflect a different Unity Player Settings `companyName` value.

**TODO: verify with real install** by checking which of these paths actually exists after first launch, and confirming the `Mods/` subfolder name.

### BepInEx script mod path

```
~/Library/Application Support/Steam/steamapps/common/Paralives/BepInEx/plugins/
```

This is in the **Steam installation directory** (the game root), NOT the persistentDataPath. It is adjacent to (not inside) `Paralives.app`.

### Steam Workshop mods path (macOS)

```
~/Library/Application Support/Steam/steamapps/workshop/content/1118520/
```

Each subscribed Workshop item gets its own numbered subdirectory.

---

## 6. macOS Compatibility Matrix

| Category | Works on macOS unchanged | Should work, untested | Needs Corkscrew handling | Windows-only / incompatible |
|---|---|---|---|---|
| JSON / text config mods | YES | | | |
| PNG/JPG texture replacements | YES | | | |
| FBX/OBJ 3D asset mods | YES | | | |
| OGG/WAV/MP3 audio mods | YES | | | |
| `.catalog` animation mods | YES | | | |
| TTF font mods | YES | | | |
| BepInEx DLL plugins (netstandard2.0) | | YES (macOS BepInEx build exists; ARM64 status unconfirmed) | Corkscrew must refuse to inject BepInEx into .app; user must configure Steam Launch Options manually | |
| Archives containing `winhttp.dll` | | | | Windows-only — must be flagged and rejected |
| Archives containing `.exe` | | | | Windows-only — must be flagged and rejected |
| Path-traversal-unsafe archives | | | Must validate with `is_safe_relative_path()` | |
| Unity AssetBundle `.unity3d` / `.bundle` | | YES (if same Unity version + platform tags) | May need platform tag check | Potentially incompatible if built for Windows standalone |

---

## 7. Trust Boundary Recommendation

**Mods live outside the .app bundle:** YES — the official mod path is `~/Library/Application Support/com.Paralives.Paralives/Mods/`, entirely separate from `Paralives.app`. Corkscrew never needs to touch the .app bundle to deploy official/data mods.

**Does Paralives require .app mutation?** NO for data mods. YES for BepInEx script mods — the BepInEx macOS install procedure explicitly requires `codesign --remove-signature Paralives.app`. This invalidates the Apple Developer ID signature on the bundle.

**Recommended default for Corkscrew: NO .app mutation.**

- Deploy all official/data mods to `~/Library/Application Support/com.Paralives.Paralives/Mods/` only.
- The .app signing is preserved.
- Sandboxed bundles (`_MASReceipt` or `/System/Applications/`) should be refused outright per existing Corkscrew policy.

**BepInEx-style mods:** If BepInEx script mods become a major community format, this is a **SEPARATE Corkscrew feature** — parallel to the SMAPI pattern in `stardew_valley_native.rs`. It should NOT be the default deploy path. A dedicated `ParalivesBepInEx` plugin would handle: detecting whether BepInEx is already installed, warning the user that .app signature removal is required, and placing `.dll` files in `BepInEx/plugins/`. This is outside scope for the initial implementation.

**Rationale:** The existing Corkscrew invariant — "Sandboxed bundles refused outright, out-of-bundle mods preserve signing" — maps cleanly to Paralives data mods. BG3 (`baldurs_gate_3_native.rs`) is the direct analogy: mods in a user data folder, .app untouched.

---

## 8. Implementation Outline for Corkscrew

Map to existing patterns. The correct reference is `baldurs_gate_3_native.rs` (out-of-bundle deployment to a user data folder). `stardew_valley_native.rs` (SMAPI .app mutation) is NOT the right pattern.

### 8.1 New file: `src-tauri/src/plugins/paralives_native.rs`

```
// Scaffold mirrors BaldursGate3NativePlugin
pub struct ParalivesNativePlugin;

const BUNDLE_ID: &str = "com.Paralives.Paralives";  // BEST GUESS — verify
const EXECUTABLE: &str = "Paralives";               // INFERRED — verify
const NATIVE_BOTTLE_SENTINEL: &str = "";
```

### 8.2 Detection

Detection priority (mirrors BG3 pattern):
1. Primary: `CFBundleIdentifier == "com.Paralives.Paralives"` from scanned `.app` bundles.
2. Fallback: executable name match `== "Paralives"` inside `Contents/MacOS/`.

Detection is performed by `native_scanner::scan_all_native()`, same as BG3/Stardew.

Refuse detection of sandboxed bundles (`_MASReceipt` present or path under `/System/Applications/`).

### 8.3 `resolve_mods_dir()`

```rust
pub fn resolve_mods_dir() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    home.join("Library/Application Support/com.Paralives.Paralives/Mods")
}
```

**TODO: verify with real install.** The implementer should confirm this path exists after first launch and contains mod folders created by the in-game tool.

### 8.4 `deploy_native`

1. Walk each enabled mod's staging directory.
2. For each file, validate path with `is_safe_relative_path()` (blocks `..`, null bytes, drive letters — existing invariant).
3. **Reject Windows-only artifacts with typed errors:**
   - `.exe` files: `DeployerError::UnsupportedArtifact("Windows executable")`
   - `winhttp.dll`: `DeployerError::UnsupportedArtifact("Windows BepInEx loader")`
   - Any `.dll` that is not in a `BepInEx/plugins/` layout: warn but allow (some .dll files are data containers on Windows that happen to be cross-platform).
4. Copy/hardlink staging files into `resolve_mods_dir()` (hardlink-first per existing `deployer.rs` convention, copy fallback for cross-volume).
5. NO `.app` mutation of any kind.

### 8.5 Load order management

**None needed.** Paralives has no `plugins.txt` or equivalent. Mod activation/deactivation is managed by the in-game UI (the `Mods` menu). Corkscrew's role is file deployment only.

**INFERRED:** If Paralives adds a load-order file in a future Early Access update, this will need revisiting. **TODO: monitor Paralives patch notes during Early Access.**

### 8.6 Optional: BepInEx-mac detector

If/when BepInEx script mods become widespread, add a separate detection step:

```rust
pub fn is_bepinex_installed(game_root: &Path) -> bool {
    game_root.join("BepInEx").exists()
        && game_root.join("run_bepinex.sh").exists()
}
```

If detected, surface a UI warning: "This mod is a BepInEx plugin. BepInEx requires removing the app's code signature and is experimental on Apple Silicon. Install BepInEx manually before deploying this mod." Do NOT automate the `codesign --remove-signature` step.

### 8.7 Register in `src-tauri/src/plugins/mod.rs`

Add `paralives_native` to the module list and the game plugin registry (same pattern as `baldurs_gate_3_native` registration).

---

## 9. Open Questions

| # | Question | Test to run on real install |
|---|---|---|
| 1 | Is the Unity scripting backend IL2CPP or Mono? | `ls Paralives.app/Contents/Frameworks/` — IL2CPP has `GameAssembly.dylib`; Mono has `MonoBleedingEdge/` |
| 2 | Exact CFBundleIdentifier | `defaults read .../Paralives.app/Contents/Info.plist CFBundleIdentifier` |
| 3 | Exact macOS executable name inside `Contents/MacOS/` | `ls Paralives.app/Contents/MacOS/` |
| 4 | Is the Mods path `com.Paralives.Paralives/Mods/` or `Paralives/Mods/`? | Launch game once, open Mods menu, click folder icon — observe which path opens |
| 5 | Does the Mods folder exist pre-launch or only after first use? | Check immediately after fresh install before first launch |
| 6 | Do BepInEx x86_64 plugins run on Apple Silicon (Rosetta)? | Install BepInEx macOS build, place a simple plugin, launch game, check `BepInEx/LogOutput.log` for errors |
| 7 | Unity version (for AssetBundle platform-tag compatibility assessment) | `strings Paralives.app/Contents/MacOS/Paralives | grep "Unity"` OR read `boot.config` |
| 8 | Is there a load order config file anywhere? | `find ~/Library/Application\ Support/com.Paralives.Paralives/ -name "*.json" -o -name "*.xml" -o -name "*.txt"` |
| 9 | Does the Steam Workshop path (`steamapps/workshop/content/1118520/`) use the same folder structure as the manual Mods folder? | Subscribe to one Workshop mod, compare folder layout to manual mod structure |
| 10 | Does Paralives validate mod folder names (GUID-based or human-readable)? | Inspect a mod folder created by the in-game tool; check if name is a UUID or the author's chosen name |

---

## Sources

- [Paralives on Steam](https://store.steampowered.com/app/1118520/Paralives/)
- [Paralives Wikipedia](https://en.wikipedia.org/wiki/Paralives)
- [Paralives Wiki — Moddable features](https://paralives.wiki.gg/wiki/Moddable_features)
- [Paralives Wiki — Creating a Mod and Uploading to Steam Workshop](https://paralives.wiki.gg/wiki/Creating_a_Mod_and_Uploading_to_the_Steam_Workshop)
- [Paralives Wiki — Portal: Modding guides](https://paralives.wiki.gg/wiki/Portal:Modding_guides)
- [simscommunity.info — Paralives Team Reveals Modding Tools (March 2026)](https://simscommunity.info/2026/03/22/paralives-modding-tools/)
- [6xvl/paralives-plugins-index (GitHub)](https://github.com/6xvl/paralives-plugins-index)
- [paralivesmod.com — How to Install Paralives Mods](https://paralivesmod.com/guides/how-to-install-paralives-mods)
- [newwebplay.com — Paralives Save File Location](https://newwebplay.com/guides/paralives-save-location/)
- [Steam discussion — exporting saves between platforms](https://steamcommunity.com/app/1118520/discussions/0/840629289174964929/)
- [aestheticpixelz.com — Paralives Mods and Custom Content FAQ](https://aestheticpixelz.com/paralives-mods-and-custom-content-faq/)
- [allthings.how — Paralives on Mac: Compatibility](https://allthings.how/paralives-on-mac-compatibility-requirements-and-what-apple-silicon-you-need/)
- [beatcopgame.com — 12 Best Mods in Paralives](https://beatcopgame.com/12-best-mods-in-paralives/)
- [BepInEx docs — Installing on IL2CPP Unity](https://docs.bepinex.dev/master/articles/user_guide/installation/unity_il2cpp.html)
- [Medium — How I Fixed My Paralives Mods Not Working](https://medium.com/@sitirukoyah22/how-i-fixed-my-paralives-mods-not-working-and-bypassed-the-50-mod-limit-crash-cc646a3d29cb)
- [Nexus Mods — Paralives Vortex Extension](https://www.nexusmods.com/site/mods/1917)
