# Corkscrew — Game Support & Compatibility Report

**Auto-generated:** October 24, 2026
**Source:** NexusMods API (`/v1/games`) — 5,629 games indexed

---

## Executive Summary

- **85 games supported** in registry
  - 2 Full (load order, LOOT, script extenders, tools)
  - 1 Enhanced (dedicated plugin with game-specific features)
  - 82 Standard (auto-detect, install, deploy, collections)
- **136 high-potential games** NOT in registry (500+ mods on NexusMods)
- **801,914 total mods** covered by supported games
- **250,964 total mods** in unsupported gap games

---

## Full Support

| Game | Mods | Load Order | Script Ext | Tools | Wine | Notes |
|------|------|------------|------------|-------|------|-------|
| Skyrim Special Edition | 142,634 | ✅ LOOT | ✅ SKSE | ✅ | Excellent | Full — LOOT, SKSE, crash logs, Engine Fixes for Wine, Wabbajack |
| Fallout 4 | 77,582 | ✅ LOOT | 🟡 F4SE | ✅ | Excellent | Full — LOOT, dedicated plugin |

## Enhanced Support

| Game | Mods | Plugin Features | Wine |
|------|------|----------------|------|
| Hogwarts Legacy | 1,515 | Enhanced — UE4SS auto-install, PAK merger, Lua routing, deploy hooks | Good |

## Standard Support (Generic Pipeline)

These games are auto-detected and use the generic mod install/deploy pipeline.
Collections work via NexusMods `game_domain`.

| # | Game | Mods | Script Ext | Framework Needed | Wine | Stores |
|---|------|------|------------|-----------------|------|--------|
| 1 | Skyrim VR | 142,634 | — | — | Unknown | Steam |
| 2 | Fallout 4 VR | 77,582 | — | — | Excellent | Steam |
| 3 | Skyrim | 73,231 | — | — | Excellent | Steam |
| 4 | Fallout: New Vegas | 42,312 | NVSE | — | Excellent | Steam, GOG, Epic |
| 5 | Stardew Valley | 34,841 | — | SMAPI | Excellent | Steam, GOG |
| 6 | The Elder Scrolls IV: Oblivion | 33,602 | OBSE | — | Excellent | Steam, GOG |
| 7 | Cyberpunk 2077 | 25,115 | — | REDmod / red4ext | Good | Steam, GOG |
| 8 | Baldur's Gate 3 | 20,052 | — | BG3 Mod Manager, PAK format | Good | Steam, GOG |
| 9 | Fallout 3 | 17,255 | FOSE | — | Excellent | Steam, GOG, Epic |
| 10 | Morrowind | 16,471 | MWSE | — | Excellent | Steam, GOG |
| 11 | Starfield | 13,989 | SFSE | — | Good | Steam |
| 12 | The Witcher 3 | 10,397 | — | Script Merger | Excellent | Steam, GOG, Epic |
| 13 | 7 Days to Die | 9,385 | — | — | Unknown | Steam |
| 14 | Blade & Sorcery | 8,957 | — | — | Unknown | Steam |
| 15 | Monster Hunter: World | 6,582 | — | Stracker's Loader | Good | Steam |
| 16 | The Sims 4 | 5,507 | — | — | Unknown | Steam |
| 17 | Dragon Age: Origins | 4,090 | — | — | Unknown | Steam |
| 18 | No Man's Sky | 3,027 | — | — | Good | Steam |
| 19 | Mount & Blade | 2,050 | — | — | Unknown | Steam |
| 20 | Sekiro | 1,990 | — | — | Good | Steam |
| 21 | X4: Foundations | 1,820 | — | — | Unknown | Steam, GOG |
| 22 | Darkest Dungeon | 1,781 | — | — | Unknown | Steam, GOG, Epic |
| 23 | War Thunder | 1,752 | — | — | Problematic | Steam |
| 24 | Kingdom Come: Deliverance | 1,730 | — | — | Excellent | Steam, Epic |
| 25 | Kenshi | 1,720 | — | — | Unknown | Steam |
| 26 | Dragon Age 2 | 1,560 | — | — | Unknown | Steam |
| 27 | Dark Souls | 1,522 | — | DSFix / DSDPT | Excellent | Steam |
| 28 | Mount & Blade: Warband | 1,435 | — | — | Unknown | Steam |
| 29 | Dark Souls II | 1,402 | — | — | Excellent | Steam |
| 30 | Daggerfall Unity | 1,059 | — | — | Unknown | - |
| 31 | Dragon's Dogma | 906 | — | — | Unknown | Steam |
| 32 | Starbound | 859 | — | — | Unknown | Steam |
| 33 | Pathfinder: Wrath of the Righteous | 827 | — | — | Unknown | Steam, GOG |
| 34 | Code Vein | 739 | — | — | Unknown | Steam |
| 35 | XCOM 2 | 700 | — | — | Unknown | Steam, GOG |
| 36 | XCOM 2: War of the Chosen | 700 | — | — | Unknown | Steam, GOG |
| 37 | Star Wars: Knights of the Old Republic | 696 | — | — | Unknown | Steam, GOG |
| 38 | World Of Tanks | 659 | — | — | Unknown | - |
| 39 | RimWorld | 641 | — | — | Unknown | Steam |
| 40 | The Witcher 2 | 639 | — | — | Unknown | Steam |
| 41 | Microsoft Flight Simulator | 542 | — | — | Unknown | Steam |
| 42 | Neverwinter Nights 2 | 519 | — | — | Unknown | - |
| 43 | The Witcher | 512 | — | — | Unknown | Steam |
| 44 | BattleTech | 509 | — | — | Unknown | Steam |
| 45 | Divinity: Original Sin 2 - Definitive Edition | 470 | — | — | Unknown | Steam |
| 46 | State of Decay | 441 | — | — | Unknown | Steam |
| 47 | Star Wars: Knights of the Old Republic II - The Sith Lords | 435 | — | — | Unknown | Steam, GOG |
| 48 | Divinity: Original Sin 2 - Original Edition | 365 | — | — | Unknown | Steam |
| 49 | Pathfinder: Kingmaker | 341 | — | — | Unknown | Steam |
| 50 | Legend of Grimrock | 325 | — | — | Unknown | Steam |
| 51 | Vampire the Masquerade: Bloodlines | 320 | — | — | Unknown | Steam, GOG |
| 52 | X Rebirth | 303 | — | — | Unknown | Steam |
| 53 | Enderal | 266 | — | — | Unknown | Steam |
| 54 | Spyro Reignited Trilogy | 247 | — | — | Unknown | Steam |
| 55 | Grim Dawn | 224 | — | — | Unknown | Steam |
| 56 | Total War: Three Kingdoms | 207 | — | — | Unknown | Steam, GOG, Epic |
| 57 | A Hat in Time | 201 | — | — | Unknown | Steam |
| 58 | Bloodstained: Ritual of the Night | 192 | — | — | Unknown | Steam, Epic |
| 59 | The Elder Scrolls Online | 184 | — | — | Unknown | Steam |
| 60 | GreedFall | 180 | — | — | Unknown | Steam |
| 61 | The Sims 3 | 158 | — | — | Unknown | Steam |
| 62 | Elex | 148 | — | — | Unknown | Steam |
| 63 | Galactic Civilizations III | 130 | — | — | Unknown | Steam, GOG |
| 64 | Shadowrun Returns | 118 | — | — | Unknown | Steam |
| 65 | Conan Exiles | 98 | — | — | Unknown | Steam |
| 66 | Surviving Mars | 94 | — | — | Unknown | Steam |
| 67 | Nehrim: At Fate's Edge | 77 | — | — | Unknown | Steam |
| 68 | Team Fortress 2 | 76 | — | — | Unknown | Steam |
| 69 | Wolcen: Lords Of Mayhem | 71 | — | — | Unknown | Steam |
| 70 | Torchlight II | 53 | — | — | Unknown | Steam, GOG |
| 71 | Mount & Blade: With Fire and Sword | 49 | — | — | Unknown | Steam |
| 72 | Factorio | 24 | — | — | Unknown | Steam |
| 73 | Kerbal Space Program | 20 | — | — | Unknown | Steam |
| 74 | Oxygen Not Included | 20 | — | — | Unknown | Steam |
| 75 | Dawn of Man | 19 | — | — | Unknown | Steam, GOG |
| 76 | Prison Architect | 16 | — | — | Unknown | Steam |
| 77 | Untitled Goose Game | 7 | — | — | Unknown | - |
| 78 | Garden Paws | 3 | — | — | Unknown | Steam |
| 79 | Neverwinter Nights: Enhanced Edition | 1 | — | — | Unknown | Steam |
| 80 | Breaking Wheel | 1 | — | — | Unknown | Steam |
| 81 | Neverwinter Nights | 1 | — | — | Unknown | - |
| 82 | Halo: The Master Chief Collection | 0 | — | — | Unknown | Steam |

---

## Gap Analysis — Unsupported Games by Impact

These games are in the NexusMods top tier but have **no Corkscrew registry entry**.
Adding a registry entry enables: detection, mod install, deployment, collections, profiles.

| # | Game | Mods | Domain | Framework | Effort | Priority |
|---|------|------|--------|-----------|--------|----------|
| 1 | Helldivers 2 | 12,842 | [helldivers2](https://www.nexusmods.com/helldivers2) | File replacement | Low | 🔴 High |
| 2 | Star Wars: Battlefront II (2017) | 9,865 | [starwarsbattlefront22017](https://www.nexusmods.com/starwarsbattlefront22017) | Frosty Mod Manager | Medium | 🔴 High |
| 3 | Marvel Rivals | 9,802 | [marvelrivals](https://www.nexusmods.com/marvelrivals) | File replacement | Low | 🔴 High |
| 4 | Mount & Blade II: Bannerlord | 9,584 | [mountandblade2bannerlord](https://www.nexusmods.com/mountandblade2bannerlord) | Module system | Medium | 🔴 High |
| 5 | Elden Ring | 7,931 | [eldenring](https://www.nexusmods.com/eldenring) | ModEngine2, EAC considerations | Medium | 🔴 High |
| 6 | Red Dead Redemption 2 | 6,377 | [reddeadredemption2](https://www.nexusmods.com/reddeadredemption2) | Script Hook | Medium | 🔴 High |
| 7 | Resident Evil 4 (2023) | 5,664 | [residentevil42023](https://www.nexusmods.com/residentevil42023) | REFramework / Fluffy Manager | Medium | 🔴 High |
| 8 | My Summer Car | 5,456 | [mysummercar](https://www.nexusmods.com/mysummercar) | MSCLoader | Medium | 🔴 High |
| 9 | Ready or Not | 5,153 | [readyornot](https://www.nexusmods.com/readyornot) | File replacement | Low | 🔴 High |
| 10 | Marvel's Spider-Man Remastered | 4,483 | [marvelsspidermanremastered](https://www.nexusmods.com/marvelsspidermanremastered) | File replacement | Low | 🟡 Medium |
| 11 | Oblivion Remastered | 4,324 | [oblivionremastered](https://www.nexusmods.com/oblivionremastered) | File replacement | Low | 🟡 Medium |
| 12 | Monster Hunter Wilds | 3,930 | [monsterhunterwilds](https://www.nexusmods.com/monsterhunterwilds) | File replacement | Low | 🟡 Medium |
| 13 | Palworld | 3,835 | [palworld](https://www.nexusmods.com/palworld) | UE4SS / BepInEx | Medium | 🟡 Medium |
| 14 | Devil May Cry 5 | 3,720 | [devilmaycry5](https://www.nexusmods.com/devilmaycry5) | REFramework | Medium | 🟡 Medium |
| 15 | Blade & Sorcery: Nomad | 3,409 | [bladeandsorcerynomad](https://www.nexusmods.com/bladeandsorcerynomad) | File replacement | Low | 🟡 Medium |
| 16 | Dragon Age: Inquisition | 3,361 | [dragonageinquisition](https://www.nexusmods.com/dragonageinquisition) | Frosty Mod Manager | Medium | 🟡 Medium |
| 17 | Monster Hunter Rise | 3,322 | [monsterhunterrise](https://www.nexusmods.com/monsterhunterrise) | REFramework | Medium | 🟡 Medium |
| 18 | Fallout 76 | 3,299 | [fallout76](https://www.nexusmods.com/fallout76) | File replacement | Low | 🟡 Medium |
| 19 | Valheim | 3,253 | [valheim](https://www.nexusmods.com/valheim) | BepInEx | Medium | 🟡 Medium |
| 20 | Ace Combat 7: Skies Unknown | 3,239 | [acecombat7skiesunknown](https://www.nexusmods.com/acecombat7skiesunknown) | File replacement | Low | 🟡 Medium |
| 21 | Street Fighter 6 | 3,218 | [streetfighter6](https://www.nexusmods.com/streetfighter6) | REFramework | Medium | 🟡 Medium |
| 22 | Stellar Blade | 3,132 | [stellarblade](https://www.nexusmods.com/stellarblade) | File replacement | Low | 🟡 Medium |
| 23 | Kingdom Come: Deliverance II | 2,913 | [kingdomcomedeliverance2](https://www.nexusmods.com/kingdomcomedeliverance2) | File replacement | Low | 🟡 Medium |
| 24 | Mass Effect Legendary Edition | 2,625 | [masseffectlegendaryedition](https://www.nexusmods.com/masseffectlegendaryedition) | File replacement | Low | 🟡 Medium |
| 25 | Dragon Age: The Veilguard | 2,574 | [dragonagetheveilguard](https://www.nexusmods.com/dragonagetheveilguard) | Frosty Mod Manager | Medium | 🟡 Medium |
| 26 | Guitar Hero World Tour | 2,572 | [guitarheroworldtour](https://www.nexusmods.com/guitarheroworldtour) | File replacement | Low | 🟡 Medium |
| 27 | Batman: Arkham Knight | 2,499 | [batmanarkhamknight](https://www.nexusmods.com/batmanarkhamknight) | File replacement | Low | 🟡 Medium |
| 28 | Resident Evil Requiem | 2,396 | [residentevilrequiem](https://www.nexusmods.com/residentevilrequiem) | File replacement | Low | 🟡 Medium |
| 29 | Jurassic World Evolution 2 | 2,383 | [jurassicworldevolution2](https://www.nexusmods.com/jurassicworldevolution2) | File replacement | Low | 🟡 Medium |
| 30 | Subnautica | 2,264 | [subnautica](https://www.nexusmods.com/subnautica) | BepInEx / QMods | Medium | 🟡 Medium |
| 31 | Kingdom Hearts III | 2,128 | [kingdomhearts3](https://www.nexusmods.com/kingdomhearts3) | File replacement | Low | 🟡 Medium |
| 32 | Final Fantasy VII Rebirth | 2,069 | [finalfantasy7rebirth](https://www.nexusmods.com/finalfantasy7rebirth) | File replacement | Low | 🟡 Medium |
| 33 | Planet Zoo | 2,054 | [planetzoo](https://www.nexusmods.com/planetzoo) | File replacement | Low | 🟡 Medium |
| 34 | S.T.A.L.K.E.R. 2: Heart of Chornobyl | 2,053 | [stalker2heartofchornobyl](https://www.nexusmods.com/stalker2heartofchornobyl) | File replacement | Low | 🟡 Medium |
| 35 | Marvel's Spider-Man 2 | 2,032 | [marvelsspiderman2](https://www.nexusmods.com/marvelsspiderman2) | File replacement | Low | 🟡 Medium |
| 36 | Resident Evil 2 (2019) | 2,031 | [residentevil22019](https://www.nexusmods.com/residentevil22019) | REFramework | Medium | 🟡 Medium |
| 37 | Metal Gear Solid V: The Phantom Pain | 2,006 | [metalgearsolidvtpp](https://www.nexusmods.com/metalgearsolidvtpp) | File replacement | Low | 🟡 Medium |
| 38 | Halo: The Master Chief Collection | 1,971 | [halothemasterchiefcollection](https://www.nexusmods.com/halothemasterchiefcollection) | File replacement | Low | 🟢 Low |
| 39 | Final Fantasy XIV | 1,955 | [finalfantasy14](https://www.nexusmods.com/finalfantasy14) | File replacement | Low | 🟢 Low |
| 40 | Avatar: Frontiers of Pandora | 1,951 | [avatarfrontiersofpandora](https://www.nexusmods.com/avatarfrontiersofpandora) | File replacement | Low | 🟢 Low |

---

## Priority Recommendations

### Highest Impact (3,000+ mods, not yet supported)

- **Helldivers 2** (12,842 mods)
- **Star Wars: Battlefront II (2017)** (9,865 mods) — requires Frosty Mod Manager
- **Marvel Rivals** (9,802 mods)
- **Mount & Blade II: Bannerlord** (9,584 mods) — requires Module system
- **Elden Ring** (7,931 mods) — requires ModEngine2, EAC considerations
- **Red Dead Redemption 2** (6,377 mods) — requires Script Hook
- **Resident Evil 4 (2023)** (5,664 mods) — requires REFramework / Fluffy Manager
- **My Summer Car** (5,456 mods) — requires MSCLoader
- **Ready or Not** (5,153 mods)
- **Marvel's Spider-Man Remastered** (4,483 mods)
- **Oblivion Remastered** (4,324 mods)
- **Monster Hunter Wilds** (3,930 mods)
- **Palworld** (3,835 mods) — requires UE4SS / BepInEx
- **Devil May Cry 5** (3,720 mods) — requires REFramework
- **Blade & Sorcery: Nomad** (3,409 mods)
- **Dragon Age: Inquisition** (3,361 mods) — requires Frosty Mod Manager
- **Monster Hunter Rise** (3,322 mods) — requires REFramework
- **Fallout 76** (3,299 mods)
- **Valheim** (3,253 mods) — requires BepInEx
- **Ace Combat 7: Skies Unknown** (3,239 mods)
- **Street Fighter 6** (3,218 mods) — requires REFramework
- **Stellar Blade** (3,132 mods)

### Quick Wins (1,000+ mods, simple file replacement, no framework needed)

- **Helldivers 2** (12,842 mods)
- **Marvel Rivals** (9,802 mods)
- **Ready or Not** (5,153 mods)
- **Marvel's Spider-Man Remastered** (4,483 mods)
- **Oblivion Remastered** (4,324 mods)
- **Monster Hunter Wilds** (3,930 mods)
- **Blade & Sorcery: Nomad** (3,409 mods)
- **Fallout 76** (3,299 mods)
- **Ace Combat 7: Skies Unknown** (3,239 mods)
- **Stellar Blade** (3,132 mods)
- **Kingdom Come: Deliverance II** (2,913 mods)
- **Mass Effect Legendary Edition** (2,625 mods)
- **Guitar Hero World Tour** (2,572 mods)
- **Batman: Arkham Knight** (2,499 mods)
- **Resident Evil Requiem** (2,396 mods)

---

## Script Extender Status

| Game | Extender | Auto-Install | Status |
|------|----------|-------------|--------|
| Fallout 3 | FOSE | ❌ | ❌ Manual only |
| Fallout 4 | F4SE | ❌ | ❌ Manual only |
| Fallout: New Vegas | NVSE | ❌ | ❌ Manual only |
| Morrowind | MWSE | ❌ | ❌ Manual only |
| The Elder Scrolls IV: Oblivion | OBSE | ❌ | ❌ Manual only |
| Skyrim Special Edition | SKSE | ✅ | ✅ Implemented |
| Starfield | SFSE | ❌ | ❌ Manual only |

---

## Wine/CrossOver Compatibility

**🟢 Excellent:** Skyrim Special Edition, Skyrim, Fallout 4, Fallout: New Vegas, The Elder Scrolls IV: Oblivion, Fallout 3, Morrowind, The Witcher 3, Dark Souls, Dark Souls II, darksouls3, Kingdom Come: Deliverance, Stardew Valley

**🟡 Good:** Cyberpunk 2077, eldenring, Hogwarts Legacy, Baldur's Gate 3, residentevil42023, residentevil22019, Monster Hunter: World, No Man's Sky, Sekiro, Starfield

**🔴 Problematic:** helldivers2, fallout76, War Thunder, finalfantasy14

---

*This report is auto-generated daily by `scripts/update-game-stats.py`.*
*Data source: NexusMods API. Registry: `src-tauri/data/vortex_game_registry.json`.*