# Crimson Desert Native macOS Support — Research Spike

**Date:** 2026-05-02  
**Status:** Research only — no code  
**Scope:** Corkscrew native macOS plugin for Crimson Desert  

---

## 1. Game Engine + Platform Facts

### BlackSpace Engine

Crimson Desert runs on Pearl Abyss's proprietary **BlackSpace Engine**, an in-house engine they have been iterating on since Black Desert Online (2014). Key traits relevant to modding:

- **Container archive format:** Game data is stored in numbered group directories (`0000/`, `0001/`, … `0035/` in the vanilla ship) under the game install root. Each group contains `0.paz` (archive data) and `0.pamt` (PackMeta index). A master registry at `meta/0.papgt` controls which groups the game loads at startup. The PAPGT loader silently ignores overlay groups with non-numeric names.
- **Encryption + compression:** PAZ archives use ChaCha20-Poly1305 encryption (per-file keys derived from filename hashes) plus LZ4 compression for certain data blocks. DDS textures and MP4 video files are frequently unencrypted. *(Source: NattKh/CrimsonDesertModdingTools, ResHax RE thread.)*
- **Internal path depth:** When patching files in, the internal PAZ directory entry must use the **full** `gamedata/binary__/client/bin/` path hierarchy, not a truncated version. Shorter paths cause silent load failures.
- **No loose-file fallback (confirmed):** The engine has no documented loose-file override path analogous to Bethesda's `Data/` folder. All mod deployment requires writing into or alongside PAZ archives. *(Inference confirmed by ResHax thread; no contradicting evidence found.)*
- **No scripting layer:** No Lua, Python, or in-game scripting surface has been documented. ASI plugin injection via `xinput1_3.dll` sideload is the only runtime-code path — but that is Windows-only (DLL injection).

### macOS Native Build Details

**CONFIRMED from Steam store page and Pearl Abyss announcement (March 2026):**

| Property | Value |
|---|---|
| Steam App ID | `3321460` |
| macOS release date | March 19, 2026 (alongside Windows/console) |
| macOS minimum OS | macOS 15.0 (Sequoia) |
| Recommended OS | macOS 26 "Tahoe" or later (per Pearl Abyss notice) |
| Processor support | Apple M2 Pro minimum, M3 Pro recommended |
| Intel Mac support | **None** — Apple Silicon only |
| Architecture | Apple Silicon native (ARM64). NOT a Universal binary — x86_64 is absent. |
| RAM minimum | 16 GB |
| Storage | 150 GB |
| Distribution | Steam + Mac App Store |

The game is Apple Silicon-only, not Universal. This simplifies detection: any macOS install is ARM64.

### CFBundleIdentifier

**App Store confirmed:** `com.pearlabyss.CrimsonDesert`  
*(Derived from the App Store container path: `~/Library/Containers/com.pearlabyss.CrimsonDesert/…`)*

For Steam builds, CFBundleIdentifier is conventionally the same but must be read from the installed Info.plist. **TODO: verify with real Steam install** — use `defaults read /path/to/CrimsonDesert.app/Contents/Info.plist CFBundleIdentifier`.

### Executable Name on macOS

Community mod tools auto-detect at `"/Applications/Crimson Desert.app"` and `"~/Applications/Crimson Desert.app"`. Steam version observed at `CrimsonDesert_Steam.app` in one community reference. The executable binary inside `Contents/MacOS/` is **most likely `CrimsonDesert`** (without suffix) — standard Pearl Abyss pattern and consistent with the Windows name `CrimsonDesert.exe`.

**TODO: verify with real install.** Candidates for `EXECUTABLES` constant:
- `CrimsonDesert` (primary prediction)
- `CrimsonDesert_Steam` (Steam wrapper variant)

---

## 2. Pearl Abyss / Crimson Desert Modding Posture

### EULA Stance

Pearl Abyss has **not officially endorsed modding** and released no mod SDK. Marketing Chief Will Powers (pre-launch): "There is enough content to keep you busy at launch, and let's revisit this conversation after the launch." CEO Heo Jin-young at a 2026 shareholder meeting acknowledged mods could be a "strong advantage" but said "providing mod tools would require disclosing a significant portion of the engine, so there are no concrete plans yet."

Corkscrew's position is the same as for any EULA-restricted game (e.g., `genshin.rs`): we surface a clear warning that modding is unsupported and users assume all risk, but we do not enforce Pearl Abyss's choices on the user's own machine.

### Informal Modding Tolerance

In practice Pearl Abyss has taken a **hands-off approach**: no players have been reported banned for single-player mods. The game launched without competitive multiplayer; if online/co-op features are added, tolerance may change. The modding community grew to hundreds of mods on Nexus within weeks of launch.

### Official Modding Framework / SDK

**None exists.** Community tools fill the gap (CDUMM, CrimsonForge, MrIkso's tools, etc.).

### Online-Only vs. Offline

Crimson Desert launched as a **single-player offline game**. No server-side validation at launch; client-side mods are viable without ban risk under the current architecture. Pearl Abyss has indicated multiplayer features may be added post-launch — at that point, mods touching gameplay-affecting data tables would become risky.

### Anti-Cheat

Crimson Desert ships **Denuvo Anti-Tamper** (DRM), NOT Denuvo Anti-Cheat. Community consensus and multi-game evidence (RE9, Monster Hunter Wilds, Stellar Blade) confirm that Anti-Tamper alone does not block file-replacement mods. Cheat Engine remains functional against Anti-Tamper.

**No XignCode3 or EQU8 detected.** BDO (Black Desert Online) uses XignCode3, but that is an always-online competitive game. Crimson Desert, being offline at launch, was not shipped with a runtime anti-cheat system.

**ASI/DLL injection on macOS is a separate issue:** dylib injection on macOS requires SIP-related workarounds and is not a viable mod vector regardless of Denuvo. Corkscrew will not support ASI/DLL mods on the native macOS plugin.

---

## 3. Existing Wine Plugin Findings

From `/Users/cashconway/Corkscrew-native-mode/src-tauri/src/plugins/crimson_desert.rs`:

| Property | Value |
|---|---|
| Steam App ID | `3321460` |
| `get_data_dir()` | Game root directory (NOT a `Data/` subdirectory) |
| `LoadOrderKind` | `None` — no plugin system |
| Executable candidates | `CrimsonDesert.exe`, `Crimson Desert.exe`, `CD.exe` |
| Also checks subdirs | `bin64/`, `binaries/`, `Bin/`, `Binaries/` |
| Protected extensions | `.exe`, `.dll`, `.pathc`, `.pamt`, `.papgt` |
| Mod categories | `asi` (.asi/.dll), `sound` (.bnk), `archive` (.pathc/.pamt/.papgt), `data` (.json), `patch` (.bsdiff/.xdelta) |
| `VERIFIED` constant | `false` — pre-release; not end-to-end tested |

**Key insight for native plugin:** The Wine plugin's `get_data_dir()` returns the game root. For native macOS, the analogous "deployment root" is the PAZ group directory structure at the game install root, not `~/Library/Application Support/`. However, mod management tooling (staging, PAMT patching) needs write access into the game install directory — which is standard Steam install layout, not an `.app` bundle interior.

The Wine plugin already identifies the correct archive extensions (`.pathc`, `.pamt`, `.papgt`) and deferral note about `pathc`/`BNK`/JSON overlay tooling. The native plugin inherits these findings.

---

## 4. Nexus Mods Crimson Desert Inventory

**Confirmed URL:** `https://www.nexusmods.com/crimsondesert` (active, large community)

### Representative Mods

| Mod | Category | Format |
|---|---|---|
| CDUMM (mod #207) | Tools | Windows app; macOS run-from-source |
| Definitive Mod Manager (#633) | Tools | PAZ overlays, JSON patches, ASI, textures |
| Crimson Desert Forge (#446) | Tools | 3D mesh, audio, localization editor |
| SWISS Knife Save Editor (#20) | Save editing | Save file binary edit |
| Crimson Desert Modding Guide (#366) | Documentation | — |
| JSON Mod Manager (#113) | Tools | JSON byte-patches to PABGB tables |
| Crimson Game Mods Tool (#1292) | Tools | Standalone toolset |
| Crimson Browser (#84) | Tools | PAZ archive browser + basic deploy |
| Mod Organizer 2 Plugin (NM Site #1782) | Tools | MO2 integration (Windows) |

### Mod Format Categories (from CDUMM README)

1. **PAZ/PAMT overlay mods** — New group directories (numbered `0036+`) containing patched `0.paz` + `0.pamt` pairs. These are the primary format for texture, mesh, and audio replacements.
2. **JSON byte-patch mods** (`.field.json`) — Human-readable patches against `.pabgb` data tables (items, mounts, skills, etc.). Applied by CDUMM/DMM at deploy time.
3. **Audio mods** — Wwise soundbank `.bnk` replacement files inside PAZ archives.
4. **ASI plugin mods** — DLLs deployed to `bin64/`. **Windows-only; incompatible with macOS native.**
5. **Binary patch mods** — `.bsdiff` / `.xdelta` patches against specific PAZ entries.
6. **Save file mods** — Direct save file edits (cross-platform).

### Manual Install Observation

Community macOS JSON mod manager (Enki013/Crimson-Desert-JSON-Mod-Manager-MacOS) uses this workflow on macOS:

1. Reads `<game_root>/0008/0.pamt` for original file index
2. Extracts target entries, applies JSON byte-patch, recompresses with LZ4
3. Writes patched output to `<game_root>/0036/0.paz` + `<game_root>/0036/0.pamt`
4. Updates `<game_root>/meta/0.papgt` to register group `0036`

This confirms: **mod deployment targets the game install directory directly**, not any user Library path.

---

## 5. PAZ/PAD Format on macOS

### Public Documentation

No official Pearl Abyss documentation. Community reverse-engineering is the sole source:

- **ResHax thread** — PAMT header structure (32 bytes: CRC, PAZ count), PazInfo array, FileInfo entries (20 bytes each: name, offset, compressed/decompressed sizes, PAZ index, flags)
- **NattKh/CrimsonDesertModdingTools** (GitHub) — Python 3.10+ tools: PAZ extractor + asset repacker, PABGB parsers for 434 tables/3,708 fields, game data schemas. Cross-platform (pure Python + `lz4` + `cryptography` deps).
- **AMGarkin/UnPAZ** (GitHub) — C# CLI for unpacking PAZ archives (originally for BDO, adapted for CD).
- **MrIkso/CrimsonDesertTools** (GitHub) — C# GUI unpacker with LZ4 decompression + ChaCha20 decryption.

### Cross-Platform Status

The Python tools (NattKh) are cross-platform by nature. The C# tools require .NET runtime (available on macOS via .NET 8). No tool is macOS-exclusive.

### Repack Strategy for Corkscrew

There are two deployment models in the community:

1. **Overlay group (PAZ-based):** CDUMM creates `<game_root>/0036/` as a new numbered overlay group. The tool extracts the relevant PAZ entries from the vanilla `0008/0.paz`, applies the mod's patch, packs a new `0036/0.paz` + `0036/0.pamt`, and updates `meta/0.papgt`. This is the safest approach — vanilla PAZ archives are never modified.
2. **In-place PAZ mutation:** Some tools modify original PAZ files directly. Risky; breaks verify-game-files.

**Corkscrew recommendation:** Use the overlay group approach. For v1 native support, Corkscrew does NOT need to implement PAZ extraction or PAMT patching internally. Mods distributed as pre-built overlay directories (i.e., a folder containing `0036/0.paz` + `0036/0.pamt` already generated by the mod author) can be **deployed by simple file copy** into the game install dir with a PAPGT registration step.

For JSON byte-patch mods (the `.field.json` format), Corkscrew would need to call out to community tooling or defer — this is a Phase 2 concern.

---

## 6. macOS Install Paths

### Game Install Directory (Steam)

`~/Library/Application Support/Steam/steamapps/common/Crimson Desert/`

The game root contains numbered group directories directly (e.g., `0000/`, `0008/`, `0035/`) and the `meta/` subdirectory. There is no separate `Data/` subdirectory.

**TODO: verify exact directory listing with real install.**

### App Bundle

`/Applications/Crimson Desert.app` (primary)  
`~/Applications/Crimson Desert.app` (alternate)  
Steam: `~/Library/Application Support/Steam/steamapps/common/Crimson Desert/CrimsonDesert_Steam.app` *(predicted, unconfirmed)*

### User Data / Save Files (CONFIRMED)

| Distribution | Save path |
|---|---|
| Steam | `~/Library/Application Support/Pearl Abyss/CD/save` |
| App Store (sandboxed) | `~/Library/Containers/com.pearlabyss.CrimsonDesert/Data/Library/Application Support/Pearl Abyss/CD/save` |

### Mod Deployment Root

**No dedicated `Mods/` folder exists.** Mods deploy into the game install directory as numbered overlay groups:

| Path | Purpose |
|---|---|
| `<game_root>/0036/0.paz` | Mod overlay archive (lowest-priority overlay) **TODO: verify with real install** |
| `<game_root>/0036/0.pamt` | Mod overlay index |
| `<game_root>/meta/0.papgt` | Master group registry (must be updated to register `0036`) |
| `<game_root>/meta/0.papgt.bak` | Backup of original PAPGT before first mod |

Higher-numbered groups take priority over lower-numbered groups. Multiple mods can use sequential groups (`0036`, `0037`, etc.) — a conflict-detection concern for future Corkscrew work.

There is no `~/Documents/Pearl Abyss/Crimson Desert/Mods/` path — save data goes to Library, not Documents. **TODO: verify with real install.**

---

## 7. macOS Compatibility Matrix

| Category | Works on macOS unchanged | Should work, untested | Needs Corkscrew handling | Windows-only / incompatible |
|---|---|---|---|---|
| Pre-built PAZ overlay mods (`.paz` + `.pamt` pair) | | Yes — file copy + PAPGT register | PAPGT update required | |
| JSON byte-patch mods (`.field.json`) | | | Requires PAZ read/patch/repack tooling | |
| Audio (`.bnk`) replacements inside PAZ | | Yes — same as overlay flow | Same PAPGT step | |
| DDS texture replacements packed in PAZ overlay | | Yes | Same PAPGT step | |
| ASI / DLL injection mods | | | | Windows-only; dylib injection on macOS is not equivalent |
| Binary patches (`.bsdiff` / `.xdelta`) | | | Requires bsdiff/xdelta apply to PAZ entry | |
| Save file edits / templates | Yes — binary format, cross-platform | | | |
| Character appearance presets | Yes — save/config data | | | |
| 3D mesh / model replacement (packed in PAZ) | | Yes | Same overlay flow | |
| ReShade / ENB-equivalent graphics injection | | | | Windows-only (D3D hooks) |
| MO2 / CDUMM Windows-only manager features | | | | Windows-only tool |
| macOS JSON mod manager (Enki013) | Yes | | | |

---

## 8. Trust Boundary Recommendation

> **2026-06-09 REVISION:** Claims 1 and 2 below are OVERTURNED by web-research findings. The `.app` IS the game root on macOS; PAZ overlays live INSIDE the bundle at `Contents/Resources/packages/`. See [docs/superpowers/research/2026-06-09-crimson-desert-macos-layout.md](../research/2026-06-09-crimson-desert-macos-layout.md) for evidence (CDUMM `MACOS.md` + Enki013 macOS tool).

1. **Mods deploy INSIDE the `.app` bundle at `Contents/Resources/packages/`.** The BlackSpace Engine on macOS reads numbered group directories from `<bundle>/Contents/Resources/packages/`. The `.app` IS the game root — there is no separate launcher. Corkscrew copies mod overlay files into `<bundle>/Contents/Resources/packages/0036/` (etc.) and updates `<packages>/meta/0.papgt`. This is acceptable and matches the SMAPI (Stardew Valley) and BepInEx (Paralives) trust-boundary precedent. Take a snapshot via `rollback::create_native_snapshot` before mutating.

2. **Apple Developer ID signature WILL be invalidated by deploy.** Writing into the bundle breaks the signature. This is the SAME class of mutation as SMAPI on Stardew Valley and BepInEx on Paralives. macOS continues to launch previously-run apps after invalidation, but Gatekeeper will not re-verify. **A consent dialog is required before first deploy.** The plugin exposes `bundle_signing_will_be_invalidated() -> bool` and `BUNDLE_MUTATION_NOTICE` for the frontend dialog. macOS may also prompt for App Management permission (System Settings → Privacy & Security → App Management) on first deploy into another `.app`.

3. **Sandboxed App Store builds are refused.** The App Store version runs inside `~/Library/Containers/com.pearlabyss.CrimsonDesert/` — Corkscrew cannot write to another app's container. If `NativeContext.sandboxed` is true, `deploy_native` must return an error with a user-facing message directing the user to use the Steam version.

4. **PAPGT mutation is the one mutable system file.** `meta/0.papgt` lives in the game install directory (not in the `.app`) and must be rewritten to register mod overlay groups. Corkscrew MUST take a snapshot of `0.papgt` before first mutation and offer restore.

5. **Anti-cheat / EULA warning.** Surface a prominent, non-dismissable warning (analogous to the sandboxed-game notice pattern) informing users that:
   - Pearl Abyss does not officially support modding
   - Modding may violate the Crimson Desert EULA
   - If Pearl Abyss adds online/multiplayer features, mods may result in account action
   - Corkscrew is not responsible for save corruption or account consequences

---

## 9. Implementation Outline

Following the BG3 native pattern in `baldurs_gate_3_native.rs`:

### File: `plugins/crimson_desert_native.rs`

**Step 1 — Scaffold + constants**
```
game_id:     "crimsondesert_native"
display_name: "Crimson Desert (Native)"
nexus_slug:  "crimsondesert"
STEAM_APP_ID: "3321460"
BUNDLE_ID:   "com.pearlabyss.CrimsonDesert"  (TODO: verify Steam build)
EXECUTABLES: ["CrimsonDesert", "CrimsonDesert_Steam"]
VERIFIED:    false
```

**Step 2 — `detect_native()`**

Scan bundle candidates:
1. `/Applications/Crimson Desert.app`
2. `~/Applications/Crimson Desert.app`
3. Steam appmanifest scan for App ID `3321460` → derive `.app` path
4. For each candidate: read `Info.plist` → confirm `CFBundleIdentifier` == `com.pearlabyss.CrimsonDesert` (or prefix match). Check for `NativeContext.sandboxed` via `_MASReceipt` presence.

**Step 3 — `resolve_game_root()`**

For a Steam build, the game data root is the `common/Crimson Desert/` directory (parent of the `.app` or alongside it). This is distinct from the `.app` path — PAZ groups live in the game root, not inside the bundle.

```
pub fn resolve_game_root(detected: &DetectedGame) -> PathBuf {
    // game_path = .app bundle path; game_data_root = PAZ group parent
    detected.runtime.native().map(|c| c.game_data_root.clone())
        .unwrap_or(detected.game_path.clone())
}
```

**Step 4 — `deploy_native`**

Deploy algorithm for PAZ overlay mods:

1. Reject sandboxed installs.
2. Call `rollback::create_native_snapshot()` — back up `meta/0.papgt`.
3. Walk enabled mods' staging directories.
4. For each mod, identify the deployment type:
   - **Pre-built overlay dir** (contains `*.paz` + `*.pamt`): assign next available group number ≥ 0036, copy files into `<game_root>/<NNNN>/`.
   - **Loose DDS / other assets**: defer — out of scope for Phase 1.
   - **JSON patch mods**: defer — requires PAMT read/patch/repack tooling.
   - **ASI/DLL**: reject with error (Windows-only).
5. Update `meta/0.papgt` to register new group numbers (append group entries; do NOT remove vanilla entries).
6. Write a `meta/0.papgt.bak` if one doesn't already exist.

**Step 5 — PAZ archive handling philosophy**

Phase 1 only copies pre-built overlay directories. Corkscrew does NOT implement PAZ extraction, PAMT parsing, or repacking internally. Mods must be packaged as ready-to-deploy overlay folders by their authors (this is the norm for distribution on Nexus).

PAZ reading/writing is a separate research project (analogous to `bg3_pak.rs`) — flag as `// TODO: Phase 2 — PAZ packing (see crimson-desert-paz-research.md)`.

**Step 6 — Anti-cheat warning notice component**

Create `src/lib/components/CrimsonDesertModNotice.svelte` (or extend the existing sandboxed-game notice pattern). Display on the Mods page when `game_id == "crimsondesert_native"`. Non-dismissable on first view; requires explicit acknowledgement stored in config.

Warning text (draft):
> Crimson Desert does not have official mod support. Modding may violate Pearl Abyss's End User License Agreement. If the game adds online or co-op features in the future, mods may result in account penalties. Corkscrew is not responsible for save corruption, bans, or other consequences. Continue only if you accept these risks.

**Step 7 — `get_plugins_file()` / `load_order_kind()`**

Return `None` / `LoadOrderKind::None` — no plugin manifest exists (same as Wine plugin).

**Step 8 — Protected file extensions**

Mirror Wine plugin: `.pathc`, `.pamt`, `.papgt`, and add native-specific concern about cleaner not touching `meta/0.papgt`.

---

## 10. Open Questions

> **2026-06-09 UPDATE:** Web research closed Q1–Q7. See [docs/superpowers/research/2026-06-09-crimson-desert-macos-layout.md](../research/2026-06-09-crimson-desert-macos-layout.md) for primary sources and per-question detail. Real-install verification is still required to flip the `VERIFIED` const to true, but the path layout, bundle IDs, and executable names are now known with strong evidence. Q8 remains weak (no evidence of a user-level mods path).

1. **RESOLVED.** Steam build uses `com.pearlabyss.CrimsonDesert.steam`; App Store uses `com.pearlabyss.CrimsonDesert`. Detection accepts both via `starts_with("com.pearlabyss.crimsondesert")`. Source: June 2026 Steam crash dump.

2. **RESOLVED.** Steam executable is `CrimsonDesert_Steam` inside `CrimsonDesert_Steam.app/Contents/MacOS/`. App Store / retail uses `Crimson Desert` or `CrimsonDesert`. All three are now in `executables()`. Source: June 2026 Steam crash dump.

3. **RESOLVED.** The `.app` IS the game root on macOS — no separate launcher. PAZ groups live INSIDE the bundle at `Contents/Resources/packages/`. Source: CDUMM `MACOS.md`.

4. **RESOLVED.** Vanilla ships groups `0000`–`0035`; `0036` is the first mod slot. Source: NattKh `MODDING_GUIDE.md` (confirmed empirically 2026-04-21).

5. **RESOLVED.** PAZ groups are INSIDE the `.app` bundle at `Contents/Resources/packages/`. Writing breaks code signing — accepted under the SMAPI / BepInEx trust-boundary precedent. Frontend consent dialog required. Source: CDUMM `MACOS.md` + Enki013 macOS mod manager.

6. **RESOLVED.** PAPGT loader silently ignores non-numeric group names. Confirmed empirically 2026-04-21 with `stk1`, `mymod`, `patch_v2`. Source: NattKh `MODDING_GUIDE.md`.

7. **RESOLVED.** macOS build ships WITHOUT Denuvo Anti-Tamper. Confirmed by BiGMAC crack (March 2026) and multiple news sources. PC has Denuvo; macOS does not. No DRM interference with PAZ overlay loading expected.

8. **WEAK EVIDENCE — still open.** No community source mentions a `~/Library/Application Support/Pearl Abyss/CD/mods/` or similar user-level mod path. All community tools (CDUMM, Enki013) write into the bundle. Working assumption: no user-level mod path exists. Verify on a real install before flipping `VERIFIED`.

---

## Summary Assessment: Does Corkscrew Need a PAZ Reader/Writer?

**For Phase 1 (initial native support): No.**

The dominant mod distribution format on Nexus is pre-built PAZ overlay directories — the mod author runs CDUMM or CrimsonForge to generate the `0036/0.paz` + `0036/0.pamt` pair, then packages it as a zip. Corkscrew's deploy step is:

1. Extract the mod archive (already supported by `installer.rs`)
2. Copy the overlay directory into `<game_root>/0036/` (simple `fs::copy` / hardlink)
3. Update `meta/0.papgt` to register group `0036` (small binary/text edit)

This is **file copy + minimal PAPGT registration** — no PAZ format knowledge needed.

**For Phase 2 (JSON byte-patch mods and fresh texture replacements): Yes, a minimal PAZ reader is needed.** This is analogous to the `bg3_pak.rs` work done for BG3. It requires:
- PAMT index parsing (read FileInfo entries to locate target files within PAZ)
- LZ4 decompress + patch + recompress
- PAZ write-back with updated PAMT checksums and offsets

Phase 2 scope is substantial and should be treated as a separate research + implementation project. The Python tools in `NattKh/CrimsonDesertModdingTools` are a viable reference implementation (MIT-licensed Python, readable format documentation).

---

*Observations marked **CONFIRMED** are from verifiable primary sources (Steam store, Pearl Abyss official notices, App Store listing, community tool READMEs). Items marked **TODO: verify with real install** require hands-on access to the installed game. Items marked *(Inference)* are logical extrapolations from related titles or partial evidence.*
