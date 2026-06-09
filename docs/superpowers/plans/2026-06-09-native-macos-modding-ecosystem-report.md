# Native macOS Games with Modding Ecosystems — Corkscrew Plugin Roadmap Report

**Date:** 2026-06-09
**Author:** Research agent
**Purpose:** Drive Corkscrew native-plugin roadmap by cataloguing modable games that run natively on macOS Apple Silicon

---

## Section 1 — Methodology

Each game was assessed along three axes:

1. **Native macOS availability**: Steam store pages, AppleGamingWiki, MacGamingDB, and doesitarm.com were consulted to distinguish true ARM64 native binaries from Intel x86_64 binaries running under Rosetta 2, Wine/CrossOver wrappers, or cloud-only solutions. "Universal" means the app bundle ships both arm64 and x86_64 slices.

2. **Mod scene maturity**: NexusMods category pages, Thunderstore community hubs, Steam Workshop item counts, official mod portals (Factorio, Paradox Mods), and ModDB were used to gauge active community size. "Active" means new uploads within 60 days of this writing and a community of at least several hundred mods.

3. **Mod tool format complexity**: GitHub repositories for game-specific mod loaders (OWML, tModLoader, CKAN, ModTheSpire, BepInEx forks), mod structure documentation, and Corkscrew's existing Paralives/BG3/SMAPI plugins were used to estimate implementation cost relative to what is already built.

Uncertainty is flagged with **TODO: verify** where live data was unavailable or contradictory.

---

## Section 2 — Already Covered (4 Games)

| Game | Status |
|------|--------|
| **Stardew Valley** | SMAPI install/uninstall, codesign removal, launcher patch; Rosetta 2 (Intel binary), runs well |
| **Paralives** | BepInEx 5 Mono inject via Doorstop 4.5, Thunderstore plugin index; native Apple Silicon ARM64 |
| **Baldur's Gate 3** | `.pak` deploy to `~/Documents/Larian Studios/`; zero `.app` mutation; native Apple Silicon |
| **Crimson Desert** | Scaffold only; PAZ archive deploy pending; native Apple Silicon |

---

## Section 3 — High-Priority Candidates (Tier A: Should Add Next)

### RimWorld — Tier A

- **Native macOS:** Universal binary — native ARM64 since v1.4 (verified via community benchmarks and Harmony Apple Silicon workshop item). Note: v1.6 ARM binaries have a known MonoMod/Harmony compat issue; patched Harmony exists.
- **Engine:** Unity (Mono)
- **Mod scene:** Extremely active — Steam Workshop (50,000+ items), Nexus with monthly round-ups; one of the largest PC mod scenes outside of Bethesda RPGs
- **Mod format:** C# DLL plugins via Harmony patching; loose XML/texture overrides; Steam Workshop zip packages
- **Required tooling beyond file ops:**
  - Harmony for Apple Silicon mod (Steam Workshop item 3515420793) must be injected or users must install it first — Corkscrew can automate this as a prerequisite
  - No BepInEx; uses own mod loader (Ludeon's built-in ModsConfig.xml + Doorstop/Harmony ecosystem)
  - Workshop download: Steam Workshop integration (shared need, see Section 6)
- **Trust boundary:** No `.app` mutation required — mods live in `~/Library/Application Support/RimWorld/Mods/` (out-of-bundle). The Harmony prerequisite mod is also dropped there, not into the bundle.
- **Implementation effort:** M — needs ModsConfig.xml read/write, mod enable/disable ordering, Harmony prerequisite bootstrap; no BepInEx injection needed
- **Bundle identifier / Steam App ID:** `294100`
- **Recommended Corkscrew approach:** New `plugins/rimworld_native.rs`; read/write `ModsConfig.xml` for load order; automate Harmony ARM prerequisite install on first use; Steam Workshop download integration

---

### Factorio — Tier A

- **Native macOS:** Universal binary, native ARM64 since v1.1.71 (November 2022); 19–25% faster on M-series than Rosetta
- **Engine:** Proprietary C++ engine
- **Mod scene:** Official mod portal at mods.factorio.com — 12,000+ mods; tightly integrated in-game downloader; extremely active
- **Mod format:** Lua scripts + data tables in `.zip` archives; purely additive/overriding game data; no native DLL injection
- **Required tooling beyond file ops:**
  - Factorio Mod Portal REST API (`https://mods.factorio.com/api/mods`) — unauthenticated browse, authenticated download (requires Factorio license)
  - OAuth/token auth for portal downloads — similar pattern to NexusMods OAuth already built
  - No runtime injection needed; mods are zips dropped in `~/Library/Application Support/factorio/mods/`
- **Trust boundary:** No `.app` mutation — mods install exclusively in user data directory
- **Implementation effort:** M — REST API client for mod portal, zip deploy to data dir, mod-list.json enable/disable management
- **Bundle identifier / Steam App ID:** `427520` (Steam) or standalone DRM-free
- **Recommended Corkscrew approach:** New `plugins/factorio_native.rs`; implement Factorio Mod Portal REST client (parallels NexusMods client pattern); deploy zips to `mods/` dir; read/write `mod-list.json` for enable states

---

### Crusader Kings III — Tier A

- **Native macOS:** Universal binary with native ARM64 since December 2025 (Paradox confirmed)
- **Engine:** Proprietary Clausewitz engine
- **Mod scene:** Active — Paradox Mods portal, Steam Workshop, ModDB; Total Conversion mods (CK2 Converter, AGOT) have thousands of subscribers
- **Mod format:** Plain-text `.mod` descriptor files + loose Clausewitz script/history/localization files in a folder; entirely file-based, no DLL injection. Case-sensitive paths on macOS are a known gotcha.
- **Required tooling beyond file ops:**
  - Paradox Mods portal has REST API (same one used by CS2 and other Paradox titles)
  - Mod folder at `~/Documents/Paradox Interactive/Crusader Kings III/mod/`
  - Steam Workshop download integration
- **Trust boundary:** No `.app` mutation — all mods out-of-bundle
- **Implementation effort:** S — purely file-deploy; `.mod` descriptor parse is trivial (key-value text). Biggest complexity is Paradox Mods portal auth if download integration is wanted.
- **Bundle identifier / Steam App ID:** `1158310`
- **Recommended Corkscrew approach:** New `plugins/ck3_native.rs`; copy mod folder + `.mod` descriptor into user data dir; optionally integrate Paradox Mods portal for browse/download; reuse Steam Workshop downloader when built

---

### The Sims 4 — Tier A

- **Native macOS:** Universal binary; native Apple Silicon confirmed (tested on macOS 15.2 as of early 2026)
- **Engine:** Proprietary (Maxis/EA engine); no DLL injection modding
- **Mod scene:** Massive — The Sims Resource, CurseForge, Patreon-distributed CC packs; millions of `.package` and `.ts4script` files. NOTE: game update 1.124 (May 2026) broke many mods — community actively updates.
- **Mod format:** `.package` binary database files (DBPF format) and `.ts4script` zip archives dropped into `~/Documents/Electronic Arts/The Sims 4/Mods/`. No special runtime injection.
- **Required tooling beyond file ops:**
  - DBPF is a well-documented format; Corkscrew only needs to copy files, not parse them
  - No DLL injection, no codesign removal
  - Conflict detection would require DBPF resource key analysis (future enhancement, not required for MVP)
- **Trust boundary:** No `.app` mutation — exclusively out-of-bundle file drop
- **Implementation effort:** S — simplest possible plugin: copy files to Mods folder, enable/disable by moving in/out of folder or renaming with `.disabled` suffix
- **Bundle identifier / Steam App ID:** `1222670` (Steam); also on EA App
- **Recommended Corkscrew approach:** New `plugins/sims4_native.rs`; file-copy deploy pattern; detect Mods folder automatically; conflict warning if two mods write the same resource key (future)

---

### Slay the Spire 2 — Tier A

- **Native macOS:** Native ARM64 confirmed; Godot 4 universal binary; arm64 DLL path at `Contents/Resources/data_sts2_macos_arm64/sts2.dll`
- **Engine:** Godot 4 (.NET / C#)
- **Mod scene:** Growing since March 2026 release — Nexus Mods active, community-built mod guides for macOS Apple Silicon
- **Mod format:** C#/.NET DLL mods referencing game's `sts2.dll`; Godot GDExtension pattern; no BepInEx required
- **Required tooling beyond file ops:**
  - Game-specific mod loader (community-developed); mod DLLs placed in game data directory
  - Mac arm64 mod building requires Godot 4.5.1 + dotnet SDK arm64 (build-side, not Corkscrew's concern)
  - Deploy: copy DLL + manifest files into game's mod directory
- **Trust boundary:** TODO: verify whether mods require `codesign --remove-signature` on the .app or are loaded from outside the bundle. Godot 4 can load GDExtension libraries from user data dirs without bundle modification in many configurations.
- **Implementation effort:** M — need to understand Godot 4 mod loading path; likely out-of-bundle for DLL drops but needs verification
- **Bundle identifier / Steam App ID:** `2868840`
- **Recommended Corkscrew approach:** New `plugins/slay_the_spire_2_native.rs`; research Godot 4 mod loading path on macOS; if out-of-bundle, straightforward file deploy

---

### Terraria (tModLoader) — Tier A

- **Native macOS:** Native ARM64 since v1.4.5 (January 2025 — official Re-Logic release); runs at full performance on M2/M3/M4
- **Engine:** XNA/.NET (MonoGame on macOS)
- **Mod scene:** Very large — Steam Workshop, tModLoader's own mod browser; thousands of mods including massive content packs (Calamity, Thorium)
- **Mod format:** tModLoader `.tmod` archives — self-contained mod packages with metadata, DLLs, assets
- **Required tooling beyond file ops:**
  - tModLoader is a separate Steam app (App ID `1281930`) that installs alongside Terraria; it has its own mod browser/downloader
  - Corkscrew's role: detect tModLoader install, manage `.tmod` files in `~/Documents/My Games/Terraria/tModLoader/Mods/`
  - tModLoader's in-game browser handles downloads from its own servers; Corkscrew can complement by managing which mods are enabled (enabled.json)
- **Trust boundary:** No `.app` mutation — `.tmod` files live in user Documents; tModLoader itself handles injection
- **Implementation effort:** M — `.tmod` is a custom binary format (documented); need to parse mod metadata for display; enable/disable via `enabled.json`
- **Bundle identifier / Steam App ID:** `105600` (Terraria), `1281930` (tModLoader)
- **Recommended Corkscrew approach:** New `plugins/terraria_native.rs`; detect both Terraria and tModLoader; manage `enabled.json`; display mod metadata from `.tmod` headers; leave download to tModLoader's browser for now

---

### Project Zomboid — Tier A

- **Native macOS:** Native ARM64 confirmed in Build 42 (early 2025); verified running at 120 FPS on M3 MacBook Pro at max settings
- **Engine:** Java (LWJGL)
- **Mod scene:** Active — Steam Workshop primary; modding wiki; hundreds of mods including map expansions, overhauls
- **Mod format:** Lua scripts + loose files in workshop folders; mods reference `workshop_id` and `mod_id` in `server.ini` / `options.ini`. No DLL injection.
- **Required tooling beyond file ops:**
  - Steam Workshop integration for download (see Section 6)
  - Enable/disable by editing `Mods=` and `WorkshopItems=` lines in `~/Zomboid/options.ini`
  - Workshop items already downloaded by Steam; Corkscrew manages enable state
- **Trust boundary:** No `.app` mutation
- **Implementation effort:** S-M — INI-based enable/disable; workshop folder scanning; Lua mod manifest parsing for display
- **Bundle identifier / Steam App ID:** `108600`
- **Recommended Corkscrew approach:** New `plugins/project_zomboid_native.rs`; parse `options.ini`; scan `~/Zomboid/mods/` and `~/Library/Application Support/Steam/steamapps/workshop/content/108600/`; enable/disable via INI edits

---

### Civilization VI — Tier A

- **Native macOS:** Universal binary with native Apple Silicon support since August 2024 (Aspyr patch); loads ~2x faster than Intel build
- **Engine:** Proprietary (Firaxis 2K Engine); Lua + XML mods
- **Mod scene:** Large — Steam Workshop (thousands of items), CivFanatics modding community; very long-tail active scene
- **Mod format:** Lua scripts + XML data files in folders dropped to `~/Library/Application Support/Sid Meier's Civilization VI/Mods/` (App Store) or Steam equivalent; no DLL injection
- **Required tooling beyond file ops:**
  - Steam Workshop download integration
  - Mod `.modinfo` XML descriptor parsing (simple key-value XML)
  - App Store vs Steam install have different data paths — need to detect which variant
- **Trust boundary:** No `.app` mutation
- **Implementation effort:** S-M — file-copy deploy; `.modinfo` XML parsing for display; path detection for App Store vs Steam
- **Bundle identifier / Steam App ID:** `289070` (Steam); `com.aspyr.civ6.appstore` (App Store)
- **Recommended Corkscrew approach:** New `plugins/civ6_native.rs`; detect install variant; parse `.modinfo`; deploy to Mods folder; Steam Workshop download when built

---

### Hollow Knight: Silksong — Tier A

- **Native macOS:** Universal binary with native ARM64; released September 4, 2025 on Steam and GOG; no Rosetta required
- **Engine:** Unity (Mono) — same as original Hollow Knight
- **Mod scene:** Growing rapidly since September 2025 release; BepInEx 5.x ecosystem being established (same pattern as original HK). NOTE: BepInEx 5.x is x86_64 on macOS; requires running game in Rosetta mode for mods. BepInEx 6.x arm64 support is experimental as of mid-2026.
- **Mod format:** BepInEx DLL plugins; some loose file replacements
- **Required tooling beyond file ops:**
  - BepInEx 5.x macOS install — sets game to run under Rosetta, installs doorstop. This is the Paralives Mono BepInEx pattern but for an Intel-mode game.
  - Alternatively wait for BepInEx 6.x arm64 to stabilize
  - `codesign --remove-signature` on the .app to allow library injection
- **Trust boundary:** `.app` mutation required — `codesign --remove-signature` + Rosetta mode flag set via `defaults write`. Same class of mutation as SMAPI but lighter.
- **Implementation effort:** M — reuse Paralives BepInEx Mono integration; add Rosetta mode coercion step; monitor BepInEx 6 arm64 for upgrade path
- **Bundle identifier / Steam App ID:** TODO: verify Silksong Steam App ID (original HK is `367520`)
- **Recommended Corkscrew approach:** New `plugins/silksong_native.rs`; reuse BepInEx Mono install path from Paralives plugin; add `arch -x86_64` launch wrapper or Rosetta flag; codesign removal

---

### Slay the Spire 1 — Tier A

- **Native macOS:** Available on App Store with Apple Silicon native (requires macOS 11+); Steam version runs via Rosetta
- **Engine:** Java/libGDX (desktop); C# not applicable
- **Mod scene:** Large — Steam Workshop, ModTheSpire framework, BaseMod; thousands of mods
- **Mod format:** `.jar` mod files loaded by ModTheSpire; placed in `SlayTheSpire.app/Contents/Resources/mods/`
- **Required tooling beyond file ops:**
  - ModTheSpire macOS: launch via `mts-launcher.jar` in app bundle — requires modifying launch arguments, not the bundle itself
  - Steam Workshop mod download (`.workshop` items are `.jar` files)
  - NOTE: ModTheSpire has known macOS issues; community workarounds exist
- **Trust boundary:** Mods placed inside .app bundle's Resources (not a code-signed section); technically bundle-touching but not re-signing required. TODO: verify whether quarantine removal is needed.
- **Implementation effort:** M — `.jar` copy into bundle Resources/mods; ModTheSpire launcher invocation; Steam Workshop download when built
- **Bundle identifier / Steam App ID:** `646570`
- **Recommended Corkscrew approach:** New `plugins/slay_the_spire_native.rs`; copy `.jar` mods into bundle Resources/mods dir; launch via ModTheSpire jar; lower priority than STS2 given STS2's cleaner modding story

---

### Valheim — Tier A

- **Native macOS:** Universal binary (ships both arm64 + x86_64 slices) but BepInEx 5.x is x86_64 only — community workaround forces Rosetta mode for the game process to run mods. A 2026 Steam guide documents this.
- **Engine:** Unity (IL2CPP in newer builds, was Mono earlier — TODO: verify current build type)
- **Mod scene:** Very large — Thunderstore (Valheim community has 4,000+ mods), NexusMods; r/valheim community
- **Mod format:** BepInEx DLL plugins (Thunderstore packages)
- **Required tooling beyond file ops:**
  - BepInEx macOS arm64 gap: must force game to run as x86_64 via Rosetta; `defaults write com.valheimgame UseRosetta YES` or equivalent
  - `codesign --remove-signature` on the .app
  - Thunderstore API integration for browse/download (Thunderstore has a documented API)
- **Trust boundary:** `.app` mutation — codesign removal + Rosetta flag; same pattern as Silksong above
- **Implementation effort:** M — BepInEx IL2CPP or Mono install (verify build type); Thunderstore download integration; Rosetta coercion. Thunderstore client is new work but unlocks many games.
- **Bundle identifier / Steam App ID:** `892970`
- **Recommended Corkscrew approach:** New `plugins/valheim_native.rs`; BepInEx install with Rosetta coercion; Thunderstore package download; codesign removal — this plugin effectively proves out the "Thunderstore game" pattern reusable by other entries

---

## Section 4 — Tier B (Worth Tracking, Lower Priority)

### Don't Starve Together

Intel-only binary as of mid-2026 (runs via Rosetta 2). Active mod scene on Steam Workshop (~10,000 items) using Lua scripts dropped into `mods/` folder — no DLL injection needed. Klei has not announced an ARM build. Corkscrew interest level: the file-deploy story would be trivial (Lua files, no codesign), but the Rosetta dependency is a yellow flag. Watch for a native ARM build announcement; upgrade to Tier A when it ships.

### RimWorld (1.6 Harmony compat note)

Already listed as Tier A. This sub-entry notes that v1.6 ARM binaries broke MonoMod/Harmony compatibility and required a community-patched Harmony. Corkscrew's plugin should bundle the patched Harmony as a required dependency and update it when Harmony mainline resolves ARM support. Track [pardeike/Harmony](https://github.com/pardeike/Harmony) releases.

### Kerbal Space Program 1

Intel-only on macOS (runs via Rosetta 2); no ARM64 build available. CKAN is the dominant mod manager (cross-platform, .NET). Large mod scene. Tier B because the Intel-only status is a long-term liability — Take-Two/Private Division sold the IP and development is effectively frozen. Worth adding if a native ARM binary ships, but don't prioritize for a dead-development title.

### Subnautica (original)

Intel-only on macOS (Rosetta 2 required). BepInEx macOS pack exists (toebeann/BepInEx.Subnautica) with documented macOS install; reported broken on Apple Silicon native mode but works in Rosetta mode. Monitor Subnautica 2 (Early Access, CurseForge mods) — if it ships a native ARM build, prioritize that instead.

### Among Us

Windows-only on Steam (no macOS native build). The Thunderstore Among Us community uses BepInEx IL2CPP. Tier B only as a Thunderstore integration proof-of-concept reference; no direct Corkscrew action warranted until a native macOS build exists.

### Outer Wilds

Windows-only on Steam (never received macOS build despite early promises). OWML (ow-mods/owml) and ow-mod-man are well-engineered open-source tools that support macOS, and the modding ecosystem is rich (~60 mods on outerwildsmods.com). If the game ever ships a native macOS build — Steam SteamDB has historically shown macOS depot activity — Corkscrew could implement OWML integration quickly given OWML's clean macOS support. Watch and fast-follow.

### Hollow Knight (original)

Intel-only binary on Steam (Rosetta 2 required). Superseded by Silksong (Tier A) for prioritization. The Silksong plugin will share 90% of its code with what a Hollow Knight plugin would need. Implement together.

---

## Section 5 — Tier C (Skip / Wait)

| Game | Reason |
|------|--------|
| **Cyberpunk 2077** | Native ARM64 landed July 2025. Modding on macOS is limited to redscript-only mods (no REDmod, no ArchiveXL, no CET). The macOS modding wiki explicitly warns that most popular mods are Windows-only. Mod scene on Mac is a small fraction of the Windows scene. Not worth the implementation cost for a thin slice of mods. Re-evaluate if CD PROJEKT RED expands macOS mod support. |
| **Lethal Company** | Windows-only — no macOS build. BepInEx IL2CPP community, highly active on Thunderstore, but irrelevant until a Mac port ships. |
| **Dyson Sphere Program** | Windows-only — confirmed no native Mac port planned. CrossOver-only on Mac. Skip. |
| **Cities: Skylines II** | No macOS version — Paradox confirmed this will not happen due to Apple Silicon architecture barriers. Skip entirely. |
| **Risk of Rain 2** | No native macOS build — CrossOver recommended. Large BepInEx/Thunderstore scene but inaccessible natively on Mac. Skip until native port confirmed. |
| **Disco Elysium** | Has macOS build (Rosetta 2, Intel-only). Mod scene is essentially nonexistent — ZA/UM dissolved the modding team, game abandoned post-development. Anti-mod from publisher perspective. Skip. |
| **Subnautica 2** | Early Access 2026, Apple Silicon compat uncertain. Monitor; revisit when out of Early Access. |
| **Cities: Skylines 1** | Superseded by CS2 discussion above; also Paradox Mods portal shift makes CS1 a lower priority. |
| **Two Point Hospital** | No active macOS mod scene; publisher does not support mods. Skip. |
| **Among Trees** | No mod ecosystem at all. Skip. |

---

## Section 6 — Cross-Cutting Tooling Needs

### BepInEx for Mono Unity (macOS, including ARM64)

Already integrated for Paralives. Paralives uses BepInEx 5 with Doorstop 4.5.0, which supports macOS ARM64 natively. Games this integration directly unlocks at Tier A with minimal new work: **Hollow Knight: Silksong** (with Rosetta coercion for existing mods), **Slay the Spire 2** (different loader but same DLL pattern). The Mono BepInEx path is proven; no new infrastructure needed, only per-game configuration (doorstop target DLL name, mod folder path).

### BepInEx for IL2CPP Unity (macOS arm64)

**Status as of mid-2026: arm64 builds NOT available in official BepInEx 6 releases.** The bleeding-edge builds (`builds.bepinex.dev`) ship `macos_x64` only. The GitHub issue #899 "MacOS arm64 Support" tracks this. The workaround is Rosetta mode coercion (force game to run as x86_64) — this is what the Valheim community does. Games that need this: **Valheim** (if IL2CPP), **Cult of the Lamb** (BepInEx 5 pack exists on Thunderstore/NexusMods). Corkscrew should implement the Rosetta coercion helper (`defaults write <bundle-id> UseRosetta YES`) as a shared utility, and monitor BepInEx for arm64 IL2CPP when it stabilizes.

**Unlock count if/when BepInEx arm64 IL2CPP ships:** Valheim, Cult of the Lamb, Subnautica (when ARM), and potentially a dozen more Unity IL2CPP games. This is a high-value future investment.

### MelonLoader on macOS

MelonLoader is an alternative to BepInEx used by some Unity games (VRChat ecosystem, a few others). As of 2026, MelonLoader's macOS support is limited and lags BepInEx significantly. None of the Tier A candidates require MelonLoader — BepInEx covers the Unity space for the games on this list. Do not implement MelonLoader integration at this time.

### Steam Workshop Integration

Multiple Tier A games benefit: **RimWorld**, **Valheim**, **Civilization VI**, **Project Zomboid**, **Terraria**, **Slay the Spire 1**, **Crusader Kings III**. The Steamworks Web API provides unauthenticated access to workshop item metadata and download URLs (`ISteamRemoteStorage/GetPublishedFileDetails`). Actual file delivery requires either: (a) Steam client download (invoke Steam URI), or (b) SteamCMD anonymous download for free workshop items. Implementing a Steam Workshop read-only download client in `steam_integration.rs` (already exists in Corkscrew) would unlock workshop browsing and download for all these games in one shot. Estimated effort: **L** for a robust implementation, **M** for a "open in Steam" browser-link approach.

### Thunderstore Integration

Valheim is the flagship Tier A game using Thunderstore, but the Thunderstore API is generic across all communities. Thunderstore has a REST API (R2ModMan uses it). Implementing a Thunderstore client in Corkscrew (`thunderstore.rs`) would unlock Valheim plus any future Thunderstore-hosted game. Estimated effort: **M** — the API is clean and well-documented, similar complexity to the existing NexusMods client. High ROI: Thunderstore is growing aggressively across Unity game communities.

### Factorio Mod Portal Client

Factorio's mod portal API is clean REST with token auth (similar to NexusMods API key). One-off client in `factorio_mod_portal.rs`. Low complexity, high reward for the Factorio community. Effort: **S-M**.

### Lua / Python Script Mod Runtimes

Don't Starve Together and Factorio use Lua embedded in the game engine — Corkscrew does not need to execute Lua, only to deploy Lua files to the correct locations. No runtime integration needed. This is a non-issue.

### Asset Bundle Merging (Unity)

Advanced Unity modding (custom characters, 3D assets) sometimes requires merging Unity `.assets` or `.bundle` files. None of the Tier A candidates require this for basic mod management — it is only needed for mod authoring workflows. Not a near-term Corkscrew concern.

### OWML (Outer Wilds Mod Loader)

OWML (ow-mods/owml, latest v2.16.2) and ow-mod-man are well-engineered open-source Rust tools that already support macOS. If Outer Wilds ever ships a native macOS build, wrapping ow-mod-man's install logic would be the fastest path (ow-mod-man is MIT licensed, written in Rust — could potentially be vendored). Defer until the game ships on Mac.

### Paradox Mods Portal

CK3 and potentially Stellaris / Victoria 3 (if they get native ARM builds) use Paradox Mods. The portal has a REST API. One shared `paradox_mods.rs` client unlocks all Paradox titles. Effort: **M**, but low urgency given CK3 mod deployment is file-copy and users can manually download from the portal.

---

## Section 7 — Trust Boundary Summary

| Game | Trust Boundary | Pattern | Notes |
|------|---------------|---------|-------|
| RimWorld | No `.app` mutation | Out-of-bundle only | Mods + Harmony to `~/Library/…/Mods/`; safest class |
| Factorio | No `.app` mutation | Out-of-bundle only | Mods to user data dir; no codesign touch |
| Crusader Kings III | No `.app` mutation | Out-of-bundle only | `~/Documents/Paradox Interactive/…/mod/` |
| The Sims 4 | No `.app` mutation | Out-of-bundle only | `~/Documents/EA/The Sims 4/Mods/` |
| Slay the Spire 2 | TODO: verify | Likely out-of-bundle | Godot 4 user data dir pattern; needs confirmation |
| Terraria / tModLoader | No `.app` mutation | Out-of-bundle only | tModLoader handles its own injection; Corkscrew manages `.tmod` files |
| Project Zomboid | No `.app` mutation | Out-of-bundle only | INI-based enable/disable; Java game, no native injection |
| Civilization VI | No `.app` mutation | Out-of-bundle only | Mods to Firaxis data dir; App Store vs Steam path differ |
| Hollow Knight: Silksong | `codesign --remove-signature` | Paralives BepInEx pattern | Also requires Rosetta mode flag for existing x86 mods |
| Slay the Spire 1 | Bundle-touching (Resources) | `.jar` into bundle Resources | Not a code-signed section; quarantine flag removal likely sufficient |
| Valheim | `codesign --remove-signature` | BepInEx Rosetta pattern | Codesign removal + Rosetta mode coercion |

**Corkscrew cannot do:** Full re-signing with a developer certificate. Any game requiring a re-signed bundle (App Store receipt validation, notarization enforcement) is out of scope. None of the Tier A games require re-signing.

---

## Section 8 — Recommended Roadmap Order

Ranked by: mod scene size × implementation simplicity × strategic value (BepInEx/Thunderstore unlock multiplier).

| Rank | Game | Rationale |
|------|------|-----------|
| 1 | **The Sims 4** | Effort S; no trust boundary concerns; enormous Mac user base; pure file copy; fastest win |
| 2 | **Factorio** | Effort M; native ARM64 with performance advantage; dedicated mod portal client is reusable pattern; passionate community |
| 3 | **Crusader Kings III** | Effort S; native ARM64 since Dec 2025; pure file deploy; Paradox portal client reusable for Stellaris/Vic3 |
| 4 | **RimWorld** | Effort M; one of the largest mod scenes on PC; requires ModsConfig.xml management + Harmony bootstrap; high community demand |
| 5 | **Valheim** | Effort M; proves out Thunderstore integration + Rosetta coercion pattern; unlocks future BepInEx games; large active scene |

**Next 5 after that:** Project Zomboid (S effort, clean INI story), Terraria/tModLoader (M effort, very large scene, native ARM64), Civilization VI (S-M, Aspyr port active), Slay the Spire 2 (M, Godot native ARM64, growing scene), Hollow Knight: Silksong (M, reuses Paralives BepInEx infra).

---

## Section 9 — Anti-Recommendations

**Do not add (even with native Mac builds):**

- **Cyberpunk 2077** — macOS mod support is a tiny subset of the Windows ecosystem (redscript-only, no REDmod/ArchiveXL/CET). The community explicitly docs this as second-class. Not worth the effort for a diminished modding experience.

- **Online-focused competitive games** — Among Us, Risk of Rain 2 online modes: anti-cheat considerations and server-side validation mean mods are host-only or require all players to match versions. The mod scene is primarily cosmetic/private-server; Corkscrew has no value to add.

- **Dead-development titles with frozen mod toolchains** — Disco Elysium (developer dissolved; no modding support from ZA/UM; publisher hostile to community); Two Point Hospital (no mod ecosystem).

- **Games without native Mac builds** — Lethal Company, Dyson Sphere Program, Cities: Skylines II, Risk of Rain 2: these are Wine/CrossOver targets, not native plugins. They belong in Corkscrew's Wine mode, not the native plugin system.

- **Early Access games with unstable mod APIs** — Subnautica 2: mod format and loader are not yet stable; implementing a plugin now risks churn. Revisit at 1.0.

- **Paralives-class games with micro mod scenes** — Adding a native plugin costs engineering time. A game with under 50 mods total doesn't justify a dedicated plugin unless it serves a strategic purpose (e.g., proving out a shared tooling pattern).

---

*Report generated 2026-06-09. Steam App IDs, mod counts, and Apple Silicon status should be re-verified at plugin implementation time as they change with game updates.*
