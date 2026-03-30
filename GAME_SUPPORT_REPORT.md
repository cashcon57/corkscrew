# Corkscrew — Top 80 NexusMods Games: Support & Feature Parity Report

**Date:** March 30, 2026
**Source:** NexusMods games page sorted by mod count (live data)

---

## Executive Summary

Of the **top 80 games** on NexusMods by mod count, Corkscrew currently has:

- **39 games in the registry** (direct support via `vortex_game_registry.json` or dedicated plugins)
- **41 games NOT in the registry** (no entry — would need to be added)
- **3 games with dedicated plugins** (Skyrim SE, Fallout 4, Hogwarts Legacy) — these have the deepest support
- **2 games with load order support** (Skyrim SE, Fallout 4 only)
- **Collections work for ANY game** with a valid `nexus_domain` — the system is game-agnostic
- **Wabbajack support is game-agnostic** in the pipeline, but modlists only exist for Bethesda games in practice

### What "Feature Parity" Means

On Windows, Vortex and MO2 are the primary mod managers. Feature parity means:

1. **Game detection** — auto-find the game in a Wine bottle
2. **Mod installation** — extract archives, handle FOMOD installers
3. **Mod deployment** — hardlink/copy files to the game's data directory
4. **Collections** — browse, download, and install NexusMods collections
5. **Load order** — manage plugin load order (Bethesda games only)
6. **Script extenders** — auto-install SKSE/F4SE/OBSE etc.
7. **Game-specific tools** — SSEEdit, BodySlide, LOOT, etc.
8. **Profiles** — per-profile mod/plugin states and saves

---

## Full Game Matrix

### Legend

| Symbol | Meaning |
|--------|---------|
| ✅ | Fully supported today |
| 🟡 | Partially supported / generic only |
| ❌ | Not supported — needs work |
| ➖ | Not applicable to this game |

### Tier 1: Bethesda RPGs (Top Priority — 7 of top 11)

These are Corkscrew's primary targets and where the most work has been done.

| # | Game | Mods | Registry | Detection | Install | Deploy | Collections | Load Order | Script Ext | Tools | Notes |
|---|------|------|----------|-----------|---------|--------|-------------|------------|------------|-------|-------|
| 1 | Skyrim SE | 129k | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ SKSE | ✅ | **Full support.** Dedicated plugin, LOOT, crash logs, Engine Fixes for Wine, Wabbajack |
| 2 | Skyrim LE | 72.9k | ✅ | ✅ | ✅ | ✅ | ✅ | 🟡 | ❌ | 🟡 | Registry entry but no dedicated plugin. Load order needs `implicit_plugins` for LE |
| 3 | Fallout 4 | 72.2k | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ F4SE | ✅ | Dedicated plugin. F4SE auto-install not implemented |
| 4 | Fallout NV | 40.4k | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ | ❌ NVSE | 🟡 | No load order support. No NVSE auto-install |
| 5 | Oblivion | 33k | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ | ❌ OBSE | 🟡 | No load order support. No OBSE auto-install |
| 9 | Fallout 3 | 17k | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ | ❌ FOSE | 🟡 | No load order support. No FOSE auto-install |
| 10 | Morrowind | 14.7k | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ | ❌ MWSE | 🟡 | No load order support. No MWSE auto-install |
| 11 | Starfield | 12.3k | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ | ❌ SFSE | 🟡 | No load order support. No SFSE auto-install |
| 35 | Fallout 76 | 2.9k | ❌ | ❌ | 🟡 | 🟡 | ❌ | ➖ | ➖ | ❌ | Online game, limited modding. Needs registry entry |
| 27 | Oblivion Remastered | 4k | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | **Brand new game** (UE5). Needs registry entry + mod path discovery |

### Tier 2: In Registry — Generic Support (30 games)

These games are in the registry and get generic mod install/deploy. Collections work via `game_domain`.

| # | Game | Mods | Slug | Load Order | Script Ext | Special Needs |
|---|------|------|------|------------|------------|---------------|
| 6 | Stardew Valley | 29.8k | `stardewvalley` | ➖ | ➖ | SMAPI framework (not auto-installed). Mods need SMAPI loader |
| 7 | Cyberpunk 2077 | 20.5k | `cyberpunk2077` | ➖ | ➖ | REDmod tool support. Some mods need `red4ext` |
| 8 | Baldur's Gate 3 | 17.3k | `baldursgate3` | ➖ | ➖ | BG3 Mod Manager compatibility. PAK mod format |
| 14 | The Witcher 3 | 8.4k | `witcher3` | ➖ | ➖ | Script Merger tool. DLC vs base mod routing |
| 15 | Blade & Sorcery | 8.4k | `bladeandsorcery` | ➖ | ➖ | VR game — may not run well under Wine |
| 17 | 7 Days To Die | 7.1k | `7daystodie` | ➖ | ➖ | Unity game — mod loader varies |
| 19 | Monster Hunter World | 6.3k | `monsterhunterworld` | ➖ | ➖ | Stracker's Loader needed for some mods |
| 24 | The Sims 4 | 4.4k | `thesims4` | ➖ | ➖ | Package/script mods go to specific paths |
| 28 | Dragon Age: Origins | 3.9k | `dragonage` | ➖ | ➖ | DAI Mod Manager format differences |
| 36 | No Man's Sky | 2.6k | `nomanssky` | ➖ | ➖ | Mods go to `GAMEDATA/PCBANKS/MODS/` |
| 54 | Sekiro | 1.7k | `sekiro` | ➖ | ➖ | Simple mod structure |
| 57 | Darkest Dungeon | 1.6k | `darkestdungeon` | ➖ | ➖ | Steam Workshop overlap |
| 58 | Dark Souls 3 | 1.5k | `darksouls3` | ➖ | ➖ | ModEngine2 needed |
| 59 | Kingdom Come 1 | 1.5k | `kingdomcomedeliverance` | ➖ | ➖ | PAK-based mods |
| 61 | Dragon Age 2 | 1.5k | `dragonage2` | ➖ | ➖ | |
| 62 | X4: Foundations | 1.5k | `x4foundations` | ➖ | ➖ | Extensions folder |
| 63 | Dark Souls | 1.4k | `darksouls` | ➖ | ➖ | DSFix/DSDPT needed |
| 64 | Kenshi | 1.4k | `kenshi` | ➖ | ➖ | |
| 65 | Hogwarts Legacy | 1.4k | `hogwartslegacy` | ➖ | ➖ | **Dedicated plugin** — UE5 PAK mods |
| 70 | Mount & Blade: Warband | 1.4k | `mbwarband` | ➖ | ➖ | Module system |
| 74 | War Thunder | 1.2k | `warthunder` | ➖ | ➖ | Skin/sound mods only |
| 76 | Dark Souls 2 | 1.2k | `darksouls2` | ➖ | ➖ | |
| 48 | Mount & Blade | 2k | `mountandblade` | ➖ | ➖ | Module system |
| 50 | Halo: MCC | 1.9k | `masterchiefcollection` | ➖ | ➖ | Assembly tool |

### Tier 3: NOT in Registry — Need Adding (41 games)

These games have no registry entry. Adding them requires at minimum a JSON entry in `vortex_game_registry.json`.

| # | Game | Mods | Slug | Effort | Notes |
|---|------|------|------|--------|-------|
| 12 | SW Battlefront II (2017) | 9.3k | `starwarsbattlefront22017` | Low | Frosty Mod Manager format — may need custom installer |
| 13 | Helldivers 2 | 9.1k | `helldivers2` | Low | Simple file replacement mods |
| 16 | M&B II: Bannerlord | 7.3k | `mountandblade2bannerlord` | Medium | Module system, launcher integration |
| 18 | Elden Ring | 6.8k | `eldenring` | Medium | ModEngine2 needed, EAC bypass |
| 20 | Marvel Rivals | 5.4k | `marvelrivals` | Low | PAK swap mods (UE) |
| 21 | Red Dead Redemption 2 | 5k | `reddeadredemption2` | Medium | Script Hook, ASI loader |
| 22 | My Summer Car | 4.8k | `mysummercar` | Low | MSCLoader mods |
| 23 | Resident Evil 4 (2023) | 4.6k | `residentevil42023` | Low | RE Framework / Fluffy Manager |
| 25 | Ready or Not | 4.3k | `readyornot` | Low | UE4 PAK mods |
| 26 | Spider-Man Remastered | 4.3k | `marvelsspidermanremastered` | Low | Simple file replacements |
| 27 | Oblivion Remastered | 4k | `oblivionremastered` | Medium | **New UE5 game** — mod scene evolving rapidly |
| 29 | Devil May Cry 5 | 3.5k | `devilmaycry5` | Low | RE Engine mods |
| 30 | Monster Hunter Wilds | 3.3k | `monsterhunterwilds` | Medium | REFramework, new game |
| 31 | Dragon Age: Inquisition | 3.2k | `dragonageinquisition` | Medium | Frosty Mod Manager |
| 32 | Monster Hunter Rise | 3.1k | `monsterhunterrise` | Low | REFramework |
| 33 | B&S: Nomad | 3.1k | `bladeandsorcerynomad` | Low | Quest VR — unlikely Wine target |
| 34 | Ace Combat 7 | 3k | `acecombat7skiesunknown` | Low | UE4 PAK mods |
| 37 | Stellar Blade | 2.4k | `stellarblade` | Low | UE mods |
| 38 | Street Fighter 6 | 2.4k | `streetfighter6` | Low | RE Engine mods |
| 39 | Mass Effect LE | 2.4k | `masseffectlegendaryedition` | Medium | ME3Tweaks Mod Manager format |
| 40 | DA: The Veilguard | 2.4k | `dragonagetheveilguard` | Medium | Frosty/new Frostbite tooling |
| 41 | Guitar Hero WT | 2.3k | `guitarheroworldtour` | Low | Niche — custom songs |
| 42 | Kingdom Come 2 | 2.3k | `kingdomcomedeliverance2` | Medium | New game — mod scene evolving |
| 43 | Valheim | 2.3k | `valheim` | Medium | BepInEx framework |
| 44 | Jurassic World Evo 2 | 2.3k | `jurassicworldevolution2` | Low | Simple file mods |
| 45 | Batman: Arkham Knight | 2.2k | `batmanarkhamknight` | Low | UE3 mods |
| 46 | Palworld | 2.1k | `palworld` | Medium | UE5, BepInEx/UE4SS |
| 47 | Kingdom Hearts III | 2k | `kingdomhearts3` | Low | UE4 PAK mods |
| 49 | Planet Zoo | 1.9k | `planetzoo` | Low | |
| 51 | Final Fantasy XIV | 1.9k | `finalfantasy14` | Medium | Penumbra/TexTools, online game |
| 52 | Subnautica | 1.8k | `subnautica` | Low | BepInEx/QMods |
| 53 | MGSV: TPP | 1.8k | `metalgearsolidvtpp` | Low | SnakeBite mod manager format |
| 55 | FF VII Rebirth | 1.7k | `finalfantasy7rebirth` | Low | UE4 PAK mods |
| 56 | FF VII Remake | 1.6k | `finalfantasy7remake` | Low | UE4 PAK mods |
| 60 | Resident Evil 2 (2019) | 1.5k | `residentevil22019` | Low | RE Engine, Fluffy Manager |
| 66 | S.T.A.L.K.E.R. 2 | 1.4k | `stalker2heartofchornobyl` | Medium | UE5, new mod scene |
| 67 | Ghost Recon Breakpoint | 1.4k | `ghostreconbreakpoint` | Low | |
| 68 | Zoo Tycoon 2 | 1.4k | `zootycoon2` | Low | Very old game |
| 69 | Jurassic World Evo | 1.4k | `jurassicworldevolution` | Low | |
| 71 | Spider-Man 2 | 1.3k | `marvelsspiderman2` | Low | |
| 72 | DOOM Eternal | 1.3k | `doometernal` | Low | |
| 73 | Sifu | 1.3k | `sifu` | Low | UE4 PAK mods |
| 75 | Mortal Kombat 1 | 1.2k | `mortalkombat` | Low | UE mods |
| 77 | My Winter Car | 1.2k | `mywintercar` | Low | |
| 78 | Dying Light 2 | 1.2k | `dyinglight2` | Low | |
| 79 | JoJo's ASBR | 1.2k | `jojosbizarreadventureallstarbattler` | Low | UE4 |
| 80 | Yu-Gi-Oh Master Duel | 1.2k | `yugiohmasterduel` | Low | Unity |

---

## Collections Support Analysis

Collections are **game-agnostic** in Corkscrew's architecture. The `browse_collections()` function takes a `game_domain` string and queries the NexusMods GraphQL API. Collection installation uses the generic mod install/deploy pipeline.

**What works for ANY game with a registry entry:**
- Browse collections by game domain
- Download collection bundles (7z)
- Parse `collection.json` manifests
- Install mods from collection (download from Nexus, extract, stage)
- Apply mod rules (load order rules between mods)
- Apply INI tweaks from collection bundles
- Binary patches (collection-specific file patches)
- Delta updates (revision diffs)

**What requires game-specific work:**
- **Plugin load order rules** — collections can specify plugin ordering, but Corkscrew only enforces this for Skyrim SE and Fallout 4
- **Mod type routing** — collections reference Vortex mod types (e.g., `dinput`, `root`, `engine`). These are resolved via Vortex extensions or dedicated plugins. Without them, all files go to the default `mod_path`
- **Game-specific post-install hooks** — some collections expect tools to run after install (FNIS, Nemesis, BodySlide)

### Collections Availability by Game (NexusMods data)

Not all games have active collection communities. The games with significant collections are:

| Game | Collections Support in Corkscrew | Notes |
|------|----------------------------------|-------|
| Skyrim SE | ✅ Full | Primary target, most collections on NexusMods |
| Fallout 4 | ✅ Full | Second most popular for collections |
| Cyberpunk 2077 | 🟡 Generic | Collections exist; may need REDmod type routing |
| Baldur's Gate 3 | 🟡 Generic | Growing collection scene |
| Starfield | 🟡 Generic | Needs load order for full parity |
| Skyrim LE | 🟡 Generic | Needs LE-specific implicit plugins |
| Fallout NV | 🟡 Generic | Needs load order |
| Oblivion | 🟡 Generic | Needs load order |
| Stardew Valley | 🟡 Generic | SMAPI dependency chain |
| The Witcher 3 | 🟡 Generic | Script Merger integration |

---

## Effort Breakdown: What It Takes for Full Parity

### Phase 1: Registry Expansion (Low Effort — ~2-3 days)

Add the 41 missing games to `vortex_game_registry.json`. Each entry needs:
- `game_id`, `name`, `nexus_domain`, `steam_id`
- `executable` (main game .exe)
- `mod_path` (where mods are deployed)
- `required_files` (verification)

This alone enables: detection, mod install, deployment, collections browsing, profiles.

**Games requiring only a registry entry (simple file-replacement modding):**
Helldivers 2, Marvel Rivals, Ready or Not, Spider-Man Remastered, Devil May Cry 5, Ace Combat 7, Street Fighter 6, Batman: Arkham Knight, Kingdom Hearts III, Planet Zoo, DOOM Eternal, Sifu, Mortal Kombat 1, Ghost Recon Breakpoint, Zoo Tycoon 2, Jurassic World Evo 1 & 2, Spider-Man 2, My Winter Car, Dying Light 2, JoJo's ASBR, Yu-Gi-Oh Master Duel, Guitar Hero WT, Stellar Blade

### Phase 2: Bethesda Load Order Expansion (Medium Effort — ~1-2 weeks)

Extend `skyrim_plugins.rs` to support all Bethesda games:

| Game | Plugin Format | Implicit Plugins | Effort |
|------|--------------|-----------------|--------|
| Skyrim LE | Same as SE | Skyrim.esm, Update.esm + DLCs | Low |
| Fallout NV | Similar | FalloutNV.esm + DLCs | Low |
| Fallout 3 | Similar | Fallout3.esm + DLCs | Low |
| Oblivion | Slightly different | Oblivion.esm + DLCs | Low |
| Morrowind | Different (no plugins.txt) | Morrowind.esm + expansions | Medium |
| Starfield | New format | Starfield.esm + DLCs | Medium |
| Oblivion Remastered | TBD — new game | Unknown | High (wait for modding scene to mature) |

This is the **highest-impact work** — these 7 games represent 380k+ mods combined.

### Phase 3: Script Extender Auto-Install (Medium Effort — ~1 week)

Generalize `skse.rs` to support multiple script extenders:

| Extender | Game | GitHub Repo | Effort |
|----------|------|-------------|--------|
| F4SE | Fallout 4 | `ianpatt/f4se` | Low (same pattern as SKSE) |
| NVSE | Fallout NV | `xNVSE/NVSE` | Low |
| OBSE | Oblivion | N/A (manual) | Medium |
| FOSE | Fallout 3 | N/A (manual) | Medium |
| MWSE | Morrowind | `MWSE/MWSE` | Low |
| SFSE | Starfield | `gazzamc/sfse` | Low |

### Phase 4: Mod Framework Integration (High Effort — ~2-4 weeks)

Many non-Bethesda games require mod loaders/frameworks:

| Framework | Games Using It | What's Needed |
|-----------|---------------|---------------|
| **BepInEx** | Valheim, Palworld, Subnautica, others | Auto-detect/install BepInEx, route mods to `BepInEx/plugins/` |
| **REFramework** | RE4, RE2, MH Rise, MH Wilds, DMC5, SF6 | Auto-detect/install, route mods correctly |
| **ModEngine2** | Elden Ring, Dark Souls 3 | Auto-detect/install, configure `modengine2_launcher.exe` |
| **UE4SS/UE Mod Loader** | Palworld, Hogwarts Legacy, many UE games | PAK mod routing already works for HL; generalize |
| **SMAPI** | Stardew Valley | Auto-detect/install, SMAPI manifest parsing |
| **Frosty Mod Manager** | DA:I, DA:V, SW Battlefront II | Frosty format is complex — may need custom installer |
| **MSCLoader** | My Summer Car | Auto-detect/install |
| **Script Hook** | RDR2, GTA V | Auto-detect/install |

### Phase 5: Vortex Extension Coverage (Low-Medium Effort — ~1 week)

Many of the 41 missing games already have Vortex extensions at `github.com/Nexus-Mods/vortex-games`. Corkscrew's QuickJS runtime can execute these, giving us:
- Correct mod paths
- Mod type routing (for collections)
- Tool definitions
- Custom installers

**Action:** Verify which of the 41 missing games have Vortex extensions and ensure they execute correctly in the sandbox.

---

## Priority Ranking

### Must-Have (Phase 1-2): Covers 90%+ of modding activity

1. **Add 41 missing registry entries** — unlocks basic support for all top 80 games
2. **Bethesda load order for all games** — Skyrim LE, FNV, FO3, Oblivion, Morrowind, Starfield
3. **Script extender generalization** — F4SE, NVSE, MWSE, SFSE at minimum

### Should-Have (Phase 3): Covers key non-Bethesda games

4. **BepInEx auto-install** — unlocks Valheim, Subnautica, Palworld
5. **REFramework integration** — unlocks Elden Ring (via ModEngine2), RE games, Monster Hunter
6. **Vortex extension validation** — ensure all 80 games work via cached extensions

### Nice-to-Have (Phase 4-5): Edge cases and polish

7. **Frosty Mod Manager format** — Dragon Age Inquisition, Battlefront II
8. **SMAPI integration** — Stardew Valley (large community but very different modding model)
9. **Game-specific tools** — SSEEdit equivalents for each game ecosystem

---

## Wine/CrossOver Compatibility Notes

Not all of these games run well (or at all) under Wine/CrossOver. Key considerations:

| Category | Games | Wine Status |
|----------|-------|-------------|
| **Excellent** | Bethesda games, Witcher 3, Dark Souls series, Kingdom Come | Well-tested, gold/platinum on ProtonDB |
| **Good** | Cyberpunk 2077, Elden Ring, RE games, Stardew Valley | Work with some tweaks |
| **Problematic** | Online/anti-cheat games (Helldivers 2, Fallout 76, War Thunder) | EAC/BattlEye may block Wine |
| **VR Only** | Blade & Sorcery, B&S Nomad | VR under Wine is experimental |
| **Console Port** | Stellar Blade | PC port may work, needs testing |
| **Not Applicable** | Guitar Hero WT, Zoo Tycoon 2 | Very old/niche |

**Recommendation:** Prioritize games with proven Wine/CrossOver compatibility. The Bethesda expansion (Phase 2) is the highest-ROI work since those games are both the most modded AND the best-supported under Wine.

---

## Summary: Total Effort Estimate

| Phase | Scope | Impact |
|-------|-------|--------|
| **Phase 1** | 41 registry entries | All 80 games get basic mod support |
| **Phase 2** | Bethesda load order | 380k+ mods get full plugin management |
| **Phase 3** | Script extenders | 6 more games get auto-install |
| **Phase 4** | Mod frameworks | ~15 games get proper mod loader support |
| **Phase 5** | Vortex extensions | Mod type routing for collections |

The architecture is sound — Corkscrew's generic pipeline (install → stage → deploy) works for any game. The gaps are primarily in **game-specific features** (load order, script extenders, mod frameworks) and **registry coverage**.
