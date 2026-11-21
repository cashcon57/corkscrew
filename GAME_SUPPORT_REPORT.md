# Corkscrew — Game Support & Compatibility Report

**Auto-generated:** November 21, 2026
**Source:** NexusMods API (`/v1/games`) — 5,902 games indexed

---

## Executive Summary

- **85 games supported** in registry
  - 2 Full (load order, LOOT, script extenders, tools)
  - 1 Enhanced (dedicated plugin with game-specific features)
  - 82 Standard (auto-detect, install, deploy, collections)
- **141 high-potential games** NOT in registry (500+ mods on NexusMods)
- **814,256 total mods** covered by supported games
- **263,591 total mods** in unsupported gap games

---

## Full Support

| Game | Mods | Load Order | Script Ext | Tools | Wine | Notes |
|------|------|------------|------------|-------|------|-------|
| Skyrim Special Edition | 144,951 | ✅ LOOT | ✅ SKSE | ✅ | Excellent | Full — LOOT, SKSE, crash logs, Engine Fixes, Wabbajack |
| Fallout 4 | 78,175 | ✅ LOOT | 🟡 F4SE | ✅ | Excellent | Full — LOOT, dedicated plugin |

## Enhanced Support

| Game | Mods | Plugin Features | Wine |
|------|------|----------------|------|
| Hogwarts Legacy | 1,541 | Enhanced — UE4SS auto-install, PAK merger, Lua routing, deploy hooks | Good |

## Standard Support (Generic Pipeline)

These games are auto-detected and use the generic mod install/deploy pipeline.
Collections work via NexusMods `game_domain`.

| # | Game | Mods | Script Ext | Framework Needed | Wine | Stores |
|---|------|------|------------|-----------------|------|--------|
| 1 | Skyrim VR | 144,951 | — | — | Unknown | Steam |
| 2 | Fallout 4 VR | 78,175 | — | — | Excellent | Steam |
| 3 | Skyrim | 73,230 | — | — | Excellent | Steam |
| 4 | Fallout: New Vegas | 42,596 | NVSE | — | Excellent | Steam, GOG, Epic |
| 5 | Stardew Valley | 35,701 | — | SMAPI | Excellent | Steam, GOG |
| 6 | The Elder Scrolls IV: Oblivion | 33,688 | OBSE | — | Excellent | Steam, GOG |
| 7 | Cyberpunk 2077 | 27,307 | — | REDmod / red4ext | Good | Steam, GOG |
| 8 | Baldur's Gate 3 | 20,429 | — | BG3 Mod Manager, PAK format | Good | Steam, GOG |
| 9 | Fallout 3 | 17,300 | FOSE | — | Excellent | Steam, GOG, Epic |
| 10 | Morrowind | 16,633 | MWSE | — | Excellent | Steam, GOG |
| 11 | Starfield | 14,167 | SFSE | — | Good | Steam |
| 12 | The Witcher 3 | 11,002 | — | Script Merger | Excellent | Steam, GOG, Epic |
| 13 | 7 Days to Die | 9,991 | — | — | Unknown | Steam |
| 14 | Blade & Sorcery | 9,119 | — | — | Unknown | Steam |
| 15 | Monster Hunter: World | 6,661 | — | Stracker's Loader | Good | Steam |
| 16 | The Sims 4 | 5,696 | — | — | Unknown | Steam |
| 17 | Dragon Age: Origins | 4,122 | — | — | Unknown | Steam |
| 18 | No Man's Sky | 3,109 | — | — | Good | Steam |
| 19 | Mount & Blade | 2,050 | — | — | Unknown | Steam |
| 20 | Sekiro | 2,024 | — | — | Good | Steam |
| 21 | X4: Foundations | 1,900 | — | — | Unknown | Steam, GOG |
| 22 | Darkest Dungeon | 1,807 | — | — | Unknown | Steam, GOG, Epic |
| 23 | War Thunder | 1,785 | — | — | Problematic | Steam |
| 24 | Kingdom Come: Deliverance | 1,767 | — | — | Excellent | Steam, Epic |
| 25 | Kenshi | 1,757 | — | — | Unknown | Steam |
| 26 | Dragon Age 2 | 1,569 | — | — | Unknown | Steam |
| 27 | Dark Souls | 1,523 | — | DSFix / DSDPT | Excellent | Steam |
| 28 | Mount & Blade: Warband | 1,454 | — | — | Unknown | Steam |
| 29 | Dark Souls II | 1,429 | — | — | Excellent | Steam |
| 30 | Daggerfall Unity | 1,106 | — | — | Unknown | - |
| 31 | Dragon's Dogma | 918 | — | — | Unknown | Steam |
| 32 | Pathfinder: Wrath of the Righteous | 863 | — | — | Unknown | Steam, GOG |
| 33 | Starbound | 859 | — | — | Unknown | Steam |
| 34 | Code Vein | 742 | — | — | Unknown | Steam |
| 35 | Star Wars: Knights of the Old Republic | 719 | — | — | Unknown | Steam, GOG |
| 36 | XCOM 2 | 701 | — | — | Unknown | Steam, GOG |
| 37 | XCOM 2: War of the Chosen | 701 | — | — | Unknown | Steam, GOG |
| 38 | The Witcher 2 | 661 | — | — | Unknown | Steam |
| 39 | World Of Tanks | 659 | — | — | Unknown | - |
| 40 | RimWorld | 637 | — | — | Unknown | Steam |
| 41 | Microsoft Flight Simulator | 542 | — | — | Unknown | Steam |
| 42 | Neverwinter Nights 2 | 521 | — | — | Unknown | - |
| 43 | The Witcher | 519 | — | — | Unknown | Steam |
| 44 | BattleTech | 514 | — | — | Unknown | Steam |
| 45 | Divinity: Original Sin 2 - Definitive Edition | 474 | — | — | Unknown | Steam |
| 46 | Star Wars: Knights of the Old Republic II - The Sith Lords | 447 | — | — | Unknown | Steam, GOG |
| 47 | State of Decay | 445 | — | — | Unknown | Steam |
| 48 | Divinity: Original Sin 2 - Original Edition | 365 | — | — | Unknown | Steam |
| 49 | Pathfinder: Kingmaker | 354 | — | — | Unknown | Steam |
| 50 | Vampire the Masquerade: Bloodlines | 336 | — | — | Unknown | Steam, GOG |
| 51 | Legend of Grimrock | 325 | — | — | Unknown | Steam |
| 52 | X Rebirth | 303 | — | — | Unknown | Steam |
| 53 | Enderal | 266 | — | — | Unknown | Steam |
| 54 | Spyro Reignited Trilogy | 248 | — | — | Unknown | Steam |
| 55 | Grim Dawn | 235 | — | — | Unknown | Steam |
| 56 | Total War: Three Kingdoms | 210 | — | — | Unknown | Steam, GOG, Epic |
| 57 | A Hat in Time | 201 | — | — | Unknown | Steam |
| 58 | Bloodstained: Ritual of the Night | 194 | — | — | Unknown | Steam, Epic |
| 59 | The Elder Scrolls Online | 192 | — | — | Unknown | Steam |
| 60 | GreedFall | 180 | — | — | Unknown | Steam |
| 61 | The Sims 3 | 166 | — | — | Unknown | Steam |
| 62 | Elex | 150 | — | — | Unknown | Steam |
| 63 | Galactic Civilizations III | 130 | — | — | Unknown | Steam, GOG |
| 64 | Shadowrun Returns | 118 | — | — | Unknown | Steam |
| 65 | Conan Exiles | 99 | — | — | Unknown | Steam |
| 66 | Surviving Mars | 94 | — | — | Unknown | Steam |
| 67 | Nehrim: At Fate's Edge | 81 | — | — | Unknown | Steam |
| 68 | Team Fortress 2 | 76 | — | — | Unknown | Steam |
| 69 | Wolcen: Lords Of Mayhem | 71 | — | — | Unknown | Steam |
| 70 | Torchlight II | 53 | — | — | Unknown | Steam, GOG |
| 71 | Mount & Blade: With Fire and Sword | 49 | — | — | Unknown | Steam |
| 72 | Factorio | 26 | — | — | Unknown | Steam |
| 73 | Kerbal Space Program | 25 | — | — | Unknown | Steam |
| 74 | Oxygen Not Included | 21 | — | — | Unknown | Steam |
| 75 | Dawn of Man | 20 | — | — | Unknown | Steam, GOG |
| 76 | Prison Architect | 16 | — | — | Unknown | Steam |
| 77 | Untitled Goose Game | 9 | — | — | Unknown | - |
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
| 1 | Helldivers 2 | 13,727 | [helldivers2](https://www.nexusmods.com/helldivers2) | File replacement | Low | 🔴 High |
| 2 | Marvel Rivals | 10,225 | [marvelrivals](https://www.nexusmods.com/marvelrivals) | File replacement | Low | 🔴 High |
| 3 | Mount & Blade II: Bannerlord | 10,062 | [mountandblade2bannerlord](https://www.nexusmods.com/mountandblade2bannerlord) | Module system | Medium | 🔴 High |
| 4 | Star Wars: Battlefront II (2017) | 9,936 | [starwarsbattlefront22017](https://www.nexusmods.com/starwarsbattlefront22017) | Frosty Mod Manager | Medium | 🔴 High |
| 5 | Elden Ring | 8,033 | [eldenring](https://www.nexusmods.com/eldenring) | ModEngine2, EAC considerations | Medium | 🔴 High |
| 6 | Red Dead Redemption 2 | 6,670 | [reddeadredemption2](https://www.nexusmods.com/reddeadredemption2) | Script Hook | Medium | 🔴 High |
| 7 | Resident Evil 4 (2023) | 5,904 | [residentevil42023](https://www.nexusmods.com/residentevil42023) | REFramework / Fluffy Manager | Medium | 🔴 High |
| 8 | My Summer Car | 5,550 | [mysummercar](https://www.nexusmods.com/mysummercar) | MSCLoader | Medium | 🔴 High |
| 9 | Ready or Not | 5,380 | [readyornot](https://www.nexusmods.com/readyornot) | File replacement | Low | 🔴 High |
| 10 | Marvel's Spider-Man Remastered | 4,514 | [marvelsspidermanremastered](https://www.nexusmods.com/marvelsspidermanremastered) | File replacement | Low | 🟡 Medium |
| 11 | Oblivion Remastered | 4,391 | [oblivionremastered](https://www.nexusmods.com/oblivionremastered) | File replacement | Low | 🟡 Medium |
| 12 | Monster Hunter Wilds | 4,086 | [monsterhunterwilds](https://www.nexusmods.com/monsterhunterwilds) | File replacement | Low | 🟡 Medium |
| 13 | Palworld | 3,942 | [palworld](https://www.nexusmods.com/palworld) | UE4SS / BepInEx | Medium | 🟡 Medium |
| 14 | Devil May Cry 5 | 3,761 | [devilmaycry5](https://www.nexusmods.com/devilmaycry5) | REFramework | Medium | 🟡 Medium |
| 15 | Valheim | 3,529 | [valheim](https://www.nexusmods.com/valheim) | BepInEx | Medium | 🟡 Medium |
| 16 | Blade & Sorcery: Nomad | 3,426 | [bladeandsorcerynomad](https://www.nexusmods.com/bladeandsorcerynomad) | File replacement | Low | 🟡 Medium |
| 17 | Monster Hunter Rise | 3,381 | [monsterhunterrise](https://www.nexusmods.com/monsterhunterrise) | REFramework | Medium | 🟡 Medium |
| 18 | Fallout 76 | 3,371 | [fallout76](https://www.nexusmods.com/fallout76) | File replacement | Low | 🟡 Medium |
| 19 | Dragon Age: Inquisition | 3,365 | [dragonageinquisition](https://www.nexusmods.com/dragonageinquisition) | Frosty Mod Manager | Medium | 🟡 Medium |
| 20 | Street Fighter 6 | 3,355 | [streetfighter6](https://www.nexusmods.com/streetfighter6) | REFramework | Medium | 🟡 Medium |
| 21 | Ace Combat 7: Skies Unknown | 3,238 | [acecombat7skiesunknown](https://www.nexusmods.com/acecombat7skiesunknown) | File replacement | Low | 🟡 Medium |
| 22 | Stellar Blade | 3,218 | [stellarblade](https://www.nexusmods.com/stellarblade) | File replacement | Low | 🟡 Medium |
| 23 | Kingdom Come: Deliverance II | 3,022 | [kingdomcomedeliverance2](https://www.nexusmods.com/kingdomcomedeliverance2) | File replacement | Low | 🟡 Medium |
| 24 | Mass Effect Legendary Edition | 2,706 | [masseffectlegendaryedition](https://www.nexusmods.com/masseffectlegendaryedition) | File replacement | Low | 🟡 Medium |
| 25 | Guitar Hero World Tour | 2,597 | [guitarheroworldtour](https://www.nexusmods.com/guitarheroworldtour) | File replacement | Low | 🟡 Medium |
| 26 | Dragon Age: The Veilguard | 2,583 | [dragonagetheveilguard](https://www.nexusmods.com/dragonagetheveilguard) | Frosty Mod Manager | Medium | 🟡 Medium |
| 27 | Batman: Arkham Knight | 2,532 | [batmanarkhamknight](https://www.nexusmods.com/batmanarkhamknight) | File replacement | Low | 🟡 Medium |
| 28 | Resident Evil Requiem | 2,509 | [residentevilrequiem](https://www.nexusmods.com/residentevilrequiem) | File replacement | Low | 🟡 Medium |
| 29 | Jurassic World Evolution 2 | 2,379 | [jurassicworldevolution2](https://www.nexusmods.com/jurassicworldevolution2) | File replacement | Low | 🟡 Medium |
| 30 | Subnautica | 2,304 | [subnautica](https://www.nexusmods.com/subnautica) | BepInEx / QMods | Medium | 🟡 Medium |
| 31 | S.T.A.L.K.E.R. 2: Heart of Chornobyl | 2,267 | [stalker2heartofchornobyl](https://www.nexusmods.com/stalker2heartofchornobyl) | File replacement | Low | 🟡 Medium |
| 32 | Marvel's Spider-Man 2 | 2,164 | [marvelsspiderman2](https://www.nexusmods.com/marvelsspiderman2) | File replacement | Low | 🟡 Medium |
| 33 | Final Fantasy VII Rebirth | 2,140 | [finalfantasy7rebirth](https://www.nexusmods.com/finalfantasy7rebirth) | File replacement | Low | 🟡 Medium |
| 34 | Kingdom Hearts III | 2,137 | [kingdomhearts3](https://www.nexusmods.com/kingdomhearts3) | File replacement | Low | 🟡 Medium |
| 35 | Resident Evil 2 (2019) | 2,133 | [residentevil22019](https://www.nexusmods.com/residentevil22019) | REFramework | Medium | 🟡 Medium |
| 36 | Crimson Desert | 2,105 | [crimsondesert](https://www.nexusmods.com/crimsondesert) | File replacement | Low | 🟡 Medium |
| 37 | Avatar: Frontiers of Pandora | 2,077 | [avatarfrontiersofpandora](https://www.nexusmods.com/avatarfrontiersofpandora) | File replacement | Low | 🟡 Medium |
| 38 | Planet Zoo | 2,065 | [planetzoo](https://www.nexusmods.com/planetzoo) | File replacement | Low | 🟡 Medium |
| 39 | Metal Gear Solid V: The Phantom Pain | 2,038 | [metalgearsolidvtpp](https://www.nexusmods.com/metalgearsolidvtpp) | File replacement | Low | 🟡 Medium |
| 40 | My Winter Car | 2,034 | [mywintercar](https://www.nexusmods.com/mywintercar) | MSCLoader | Medium | 🟡 Medium |

---

## Priority Recommendations

### Highest Impact (3,000+ mods, not yet supported)

- **Helldivers 2** (13,727 mods)
- **Marvel Rivals** (10,225 mods)
- **Mount & Blade II: Bannerlord** (10,062 mods) — requires Module system
- **Star Wars: Battlefront II (2017)** (9,936 mods) — requires Frosty Mod Manager
- **Elden Ring** (8,033 mods) — requires ModEngine2, EAC considerations
- **Red Dead Redemption 2** (6,670 mods) — requires Script Hook
- **Resident Evil 4 (2023)** (5,904 mods) — requires REFramework / Fluffy Manager
- **My Summer Car** (5,550 mods) — requires MSCLoader
- **Ready or Not** (5,380 mods)
- **Marvel's Spider-Man Remastered** (4,514 mods)
- **Oblivion Remastered** (4,391 mods)
- **Monster Hunter Wilds** (4,086 mods)
- **Palworld** (3,942 mods) — requires UE4SS / BepInEx
- **Devil May Cry 5** (3,761 mods) — requires REFramework
- **Valheim** (3,529 mods) — requires BepInEx
- **Blade & Sorcery: Nomad** (3,426 mods)
- **Monster Hunter Rise** (3,381 mods) — requires REFramework
- **Fallout 76** (3,371 mods)
- **Dragon Age: Inquisition** (3,365 mods) — requires Frosty Mod Manager
- **Street Fighter 6** (3,355 mods) — requires REFramework
- **Ace Combat 7: Skies Unknown** (3,238 mods)
- **Stellar Blade** (3,218 mods)
- **Kingdom Come: Deliverance II** (3,022 mods)

### Quick Wins (1,000+ mods, simple file replacement, no framework needed)

- **Helldivers 2** (13,727 mods)
- **Marvel Rivals** (10,225 mods)
- **Ready or Not** (5,380 mods)
- **Marvel's Spider-Man Remastered** (4,514 mods)
- **Oblivion Remastered** (4,391 mods)
- **Monster Hunter Wilds** (4,086 mods)
- **Blade & Sorcery: Nomad** (3,426 mods)
- **Fallout 76** (3,371 mods)
- **Ace Combat 7: Skies Unknown** (3,238 mods)
- **Stellar Blade** (3,218 mods)
- **Kingdom Come: Deliverance II** (3,022 mods)
- **Mass Effect Legendary Edition** (2,706 mods)
- **Guitar Hero World Tour** (2,597 mods)
- **Batman: Arkham Knight** (2,532 mods)
- **Resident Evil Requiem** (2,509 mods)

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