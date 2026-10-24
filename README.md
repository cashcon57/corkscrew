<p align="center">
  <img src="brand-kit/corkscrew-icon-256.png" width="200" alt="Corkscrew">
</p>

<h1 align="center">Corkscrew</h1>

<p align="center">
  <strong>A native mod manager for Wine games on macOS and Linux.</strong>
</p>

<p align="center">
  <a href="https://corkscrewmodmanager.com">🌐 Website</a>
  &nbsp;•&nbsp;
  <a href="https://discord.gg/nufkm2b7pB"><strong>💬 Join the Discord</strong></a>
</p>

<p align="center">
  <a href="https://discord.gg/nufkm2b7pB">
    <img src="https://invidget.switchblade.xyz/nufkm2b7pB" alt="Join the Corkscrew Discord">
  </a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/macOS-10.15+-000000?logo=apple&logoColor=white" alt="macOS">
  <img src="https://img.shields.io/badge/Linux-SteamOS%20%7C%20Ubuntu%20%7C%20Fedora-FCC624?logo=linux&logoColor=black" alt="Linux">
  <img src="https://img.shields.io/badge/License-GPL--3.0-blue" alt="License">
  <a href="https://discord.gg/nufkm2b7pB"><img src="https://img.shields.io/discord/1498470295470739547?logo=discord&logoColor=white&label=Discord&color=5865F2" alt="Discord"></a>
  <a href="https://ko-fi.com/cash508287"><img src="https://img.shields.io/badge/Ko--fi-Support%20Corkscrew-FF5E5B?logo=ko-fi&logoColor=white" alt="Ko-fi"></a>
</p>

<br>

Corkscrew installs, manages, and organizes mods for Windows games running through [CrossOver](https://www.codeweavers.com/crossover), [Whisky](https://getwhisky.app/), [Lutris](https://lutris.net/), [Proton](https://github.com/ValveSoftware/Proton), and other Wine-based compatibility layers. No Windows VM required.

It works by reading and writing directly to your Wine bottle's filesystem — the same way the game sees it. Your bottles, your mods, no middleman.

---

## Quick Links

- **[Join the Discord](https://discord.gg/nufkm2b7pB)** — Live support, modding help, release announcements
- [What Works Today](#what-works-today) — Current status, tested features, known issues
- [Game Support Status](#game-support-status) — Full breakdown of 80 games: support tiers, planned additions, upcoming enhancements
- [Install](#install) — Download links and build-from-source instructions
- [Features](#features) — Core features, NexusMods, load order, Wabbajack, shader compat, AI assistant
- [Supported Platforms](#supported-platforms) — Wine sources (CrossOver, Proton, Lutris, etc.) and game list
- [SSE Engine Fixes for Wine](#sse-engine-fixes-for-wine) — Companion SKSE plugin for large modlists under Wine
- [Architecture](#architecture) — Tech stack, project structure, how mods are installed
- [Contributing](#contributing) — Setup, help wanted, how to add game support
- [Acknowledgments](#acknowledgments) — Projects and authors I build on

---

## What Works Today

Corkscrew has been **tested extensively with Skyrim Special Edition** on macOS (Apple Silicon, CrossOver). Here's where things stand:

### Tested & Working (Skyrim SE)
- Full mod lifecycle: install from archive, stage, deploy via hardlinks, toggle on/off, uninstall
- **NexusMods Collections** — Small-to-medium collections work. [Immersive & Pure](https://next.nexusmods.com/skyrimspecialedition/collections/vaakhi) by Canliberk is the reference tested collection (premium: fully automated; free: guided manual download). FOMOD replay, binary patches, INI tweaks, plugin sync, delta updates.
- **Plugin load order** — LOOT-powered sorting via [libloot](https://github.com/loot/libloot), drag-and-drop reorder, custom rules
- **SKSE auto-install** — Detects your game version, downloads the right SKSE build from GitHub
- **[SSE Engine Fixes for Wine](https://github.com/corkscrewmodding/SSEEngineFixesForWine)** — Auto-deployed before every launch (see [below](#sse-engine-fixes-for-wine))
- NXM protocol handling (click "Download with Mod Manager" on Nexus → mod appears in Corkscrew)
- Profiles, crash log analysis, INI editor with presets, mod tools management
- NexusMods OAuth sign-in + API key fallback
- Optional anonymous crash reporting via [Sentry](https://sentry.io/) (opt-in, disabled by default — see [Privacy Policy](PRIVACY.md))

### Known Issues
- **Large modlists don't work yet.** Gate to Sovngarde (1700+ plugins) installs and reaches main menu but freezes on New Game due to hash table corruption in Skyrim's engine under Wine. **This is the current bottleneck** — I am actively iterating on [SSE Engine Fixes for Wine](https://github.com/corkscrewmodding/SSEEngineFixesForWine) to solve this. Smaller modlists like [Immersive & Pure](https://next.nexusmods.com/skyrimspecialedition/collections/vaakhi) work end-to-end including New Game and gameplay.
- **Wabbajack modlists** — The install pipeline is built (multi-source downloads, BSDiff patching, BSA packing, directive processing), but game file source extraction is incomplete. Complex modlists that depend on vanilla game files as patch sources will partially fail. This is the other main blocker for v1.0.

### Untested
- **Every game except Skyrim SE.** 80+ games are auto-detected and support basic mod deployment, but only Skyrim SE and Fallout 4 have full-featured plugins (load order, LOOT, script extender, INI presets, crash logs). I haven't verified the mod workflow end-to-end for other games yet.
- **Linux.** The app builds for Linux, handles Linux paths, and supports Proton/Lutris/SteamOS bottles. But primary development and testing happens on macOS. Community testing and feedback on Linux is very welcome.

---

## Game Support Status

We're actively expanding support to cover the **top 80 most-modded games on NexusMods**. The architecture is game-agnostic — mod installation, staging, deployment, collections, and profiles work for any game. What varies is the depth of game-specific integration.

> **Note on platform compatibility:** Corkscrew is a cross-platform app. Some games listed below may not run under Wine/CrossOver on macOS or under Proton on Linux (due to anti-cheat, DRM, VR requirements, or engine limitations). I'm adding mod management support for all top 80 games regardless, because Corkscrew is designed to eventually support native Windows as well. If a game doesn't run on your platform today, the mod tooling will be ready when it does — or when you use Corkscrew on Windows.

### Support Tiers

| Tier | What It Means | Games |
|------|---------------|-------|
| **Full** | Load order, LOOT sorting, script extender auto-install, INI presets, crash logs, mod tools | Skyrim SE, Fallout 4 |
| **Enhanced** | Dedicated plugin with custom mod routing, framework auto-install, deploy hooks | Hogwarts Legacy |
| **Standard** | Auto-detection, mod install/deploy, collections, profiles, NexusMods integration | 36 games (see below) |
| **Planned** | Not yet in registry — will be added in upcoming releases | 41 games (see below) |

### Full Support

| Game | Mods on Nexus | Load Order | Script Extender | Collections |
|------|--------------|------------|-----------------|-------------|
| Skyrim Special Edition | 129k | LOOT | SKSE (auto-install) | Full |
| Fallout 4 | 72.2k | LOOT | F4SE (auto-install) | Full |

### Enhanced Support: Hogwarts Legacy

Hogwarts Legacy has a dedicated plugin that provides a **fully automated, zero-config modding experience** — better than the typical Windows workflow, which requires manual UE4SS setup, third-party merger tools, and config editing.

| Feature | Status | What Corkscrew Does |
|---------|--------|---------------------|
| PAK mods | Automatic | Routes to `~mods/` — detected by `.pak` extension |
| UE4SS framework | Auto-installed | Downloads from GitHub/NexusMods when Lua or Logic mods are detected |
| Lua script mods | Automatic | Routes to `Mods/`, auto-registers in `Mods.txt` via deploy hooks |
| Blueprint/Logic mods | Automatic | Detected by `.logicmod`/`.ue4sslogicmod` markers, routes to `LogicMods/` |
| Engine root (UE4SS DLLs, ReShade) | Automatic | Detected by known DLL names, routes to exe directory |
| Movie mods (.bk2) | Automatic | Detected and routed to `Phoenix/Content` |
| PAK database conflicts | Auto-merged | Native Rust merger (replaces Windows-only HLModMerger.exe) — detects conflicting pakchunk numbers and merges SQLite databases automatically |
| FOMOD installers | Automatic | Generic FOMOD wizard; mod type detection runs post-FOMOD |

**Collections like [Goblet](https://next.nexusmods.com/hogwartslegacy/collections) install with zero manual steps.** UE4SS is auto-deployed, Lua mods are auto-registered, PAK conflicts are auto-merged, and all 5 mod types route to the correct directories.

#### Unreal Engine Generalization

Most of this implementation is being generalized into a reusable `UnrealEnginePlugin` base that will provide PAK routing, UE4SS integration, Lua mod management, and database merging for **any UE4/5 game** with minimal per-game configuration. Games that will benefit:

Ready or Not | Palworld | Kingdom Hearts III | Ace Combat 7 | Sifu | FF VII Remake | FF VII Rebirth | S.T.A.L.K.E.R. 2 | Dying Light 2 | Oblivion Remastered

<!-- GAME_SUPPORT_TABLE_START -->
**85 games supported** — auto-updated daily from NexusMods API

| # | Game | NexusMods Domain | Mods | Tools | Stores |
|---|------|-----------------|------|-------|--------|
| 1 | Skyrim Special Edition | [skyrimspecialedition](https://www.nexusmods.com/skyrimspecialedition) | 142,634 | Yes | Steam, GOG, Epic |
| 2 | Skyrim VR | [skyrimspecialedition](https://www.nexusmods.com/skyrimspecialedition) | 142,634 | Yes | Steam |
| 3 | Fallout 4 | [fallout4](https://www.nexusmods.com/fallout4) | 77,582 | Yes | Steam, GOG, Epic |
| 4 | Fallout 4 VR | [fallout4](https://www.nexusmods.com/fallout4) | 77,582 | Yes | Steam |
| 5 | Skyrim | [skyrim](https://www.nexusmods.com/skyrim) | 73,231 | Yes | Steam |
| 6 | Fallout: New Vegas | [newvegas](https://www.nexusmods.com/newvegas) | 42,312 | Yes | Steam, GOG, Epic |
| 7 | Stardew Valley | [stardewvalley](https://www.nexusmods.com/stardewvalley) | 34,841 | Yes | Steam, GOG |
| 8 | The Elder Scrolls IV: Oblivion | [oblivion](https://www.nexusmods.com/oblivion) | 33,602 | Yes | Steam, GOG |
| 9 | Cyberpunk 2077 | [cyberpunk2077](https://www.nexusmods.com/cyberpunk2077) | 25,115 |  | Steam, GOG |
| 10 | Baldur's Gate 3 | [baldursgate3](https://www.nexusmods.com/baldursgate3) | 20,052 | Yes | Steam, GOG |
| 11 | Fallout 3 | [fallout3](https://www.nexusmods.com/fallout3) | 17,255 | Yes | Steam, GOG, Epic |
| 12 | Morrowind | [morrowind](https://www.nexusmods.com/morrowind) | 16,471 | Yes | Steam, GOG |
| 13 | Starfield | [starfield](https://www.nexusmods.com/starfield) | 13,989 |  | Steam |
| 14 | The Witcher 3 | [witcher3](https://www.nexusmods.com/witcher3) | 10,397 | Yes | Steam, GOG, Epic |
| 15 | 7 Days to Die | [7daystodie](https://www.nexusmods.com/7daystodie) | 9,385 |  | Steam |
| 16 | Blade & Sorcery | [bladeandsorcery](https://www.nexusmods.com/bladeandsorcery) | 8,957 |  | Steam |
| 17 | Monster Hunter: World | [monsterhunterworld](https://www.nexusmods.com/monsterhunterworld) | 6,582 | Yes | Steam |
| 18 | The Sims 4 | [thesims4](https://www.nexusmods.com/thesims4) | 5,507 |  | Steam |
| 19 | Dragon Age: Origins | [dragonage](https://www.nexusmods.com/dragonage) | 4,090 |  | Steam |
| 20 | No Man's Sky | [nomanssky](https://www.nexusmods.com/nomanssky) | 3,027 |  | Steam |
| 21 | Mount & Blade | [mountandblade](https://www.nexusmods.com/mountandblade) | 2,050 |  | Steam |
| 22 | Sekiro | [sekiro](https://www.nexusmods.com/sekiro) | 1,990 |  | Steam |
| 23 | X4: Foundations | [x4foundations](https://www.nexusmods.com/x4foundations) | 1,820 |  | Steam, GOG |
| 24 | Darkest Dungeon | [darkestdungeon](https://www.nexusmods.com/darkestdungeon) | 1,781 |  | Steam, GOG, Epic |
| 25 | War Thunder | [warthunder](https://www.nexusmods.com/warthunder) | 1,752 |  | Steam |
| 26 | Kingdom Come: Deliverance | [kingdomcomedeliverance](https://www.nexusmods.com/kingdomcomedeliverance) | 1,730 |  | Steam, Epic |
| 27 | Kenshi | [kenshi](https://www.nexusmods.com/kenshi) | 1,720 |  | Steam |
| 28 | Dragon Age 2 | [dragonage2](https://www.nexusmods.com/dragonage2) | 1,560 |  | Steam |
| 29 | Dark Souls | [darksouls](https://www.nexusmods.com/darksouls) | 1,522 |  | Steam |
| 30 | Hogwarts Legacy | [hogwartslegacy](https://www.nexusmods.com/hogwartslegacy) | 1,515 |  | Steam |
| 31 | Mount & Blade: Warband | [mbwarband](https://www.nexusmods.com/mbwarband) | 1,435 |  | Steam |
| 32 | Dark Souls II | [darksouls2](https://www.nexusmods.com/darksouls2) | 1,402 |  | Steam |
| 33 | Daggerfall Unity | [daggerfallunity](https://www.nexusmods.com/daggerfallunity) | 1,059 |  | - |
| 34 | Dragon's Dogma | [dragonsdogma](https://www.nexusmods.com/dragonsdogma) | 906 |  | Steam |
| 35 | Starbound | [starbound](https://www.nexusmods.com/starbound) | 859 |  | Steam |
| 36 | Pathfinder: Wrath of the Righteous | [pathfinderwrathoftherighteous](https://www.nexusmods.com/pathfinderwrathoftherighteous) | 827 |  | Steam, GOG |
| 37 | Code Vein | [codevein](https://www.nexusmods.com/codevein) | 739 |  | Steam |
| 38 | XCOM 2 | [xcom2](https://www.nexusmods.com/xcom2) | 700 | Yes | Steam, GOG |
| 39 | XCOM 2: War of the Chosen | [xcom2](https://www.nexusmods.com/xcom2) | 700 |  | Steam, GOG |
| 40 | Star Wars: Knights of the Old Republic | [kotor](https://www.nexusmods.com/kotor) | 696 |  | Steam, GOG |
| 41 | World Of Tanks | [worldoftanks](https://www.nexusmods.com/worldoftanks) | 659 |  | - |
| 42 | RimWorld | [rimworld](https://www.nexusmods.com/rimworld) | 641 |  | Steam |
| 43 | The Witcher 2 | [witcher2](https://www.nexusmods.com/witcher2) | 639 |  | Steam |
| 44 | Microsoft Flight Simulator | [microsoftflightsimulator](https://www.nexusmods.com/microsoftflightsimulator) | 542 |  | Steam |
| 45 | Neverwinter Nights 2 | [neverwinter2](https://www.nexusmods.com/neverwinter2) | 519 |  | - |
| 46 | The Witcher | [witcher](https://www.nexusmods.com/witcher) | 512 |  | Steam |
| 47 | BattleTech | [battletech](https://www.nexusmods.com/battletech) | 509 |  | Steam |
| 48 | Divinity: Original Sin 2 - Definitive Edition | [divinityoriginalsin2definitiveedition](https://www.nexusmods.com/divinityoriginalsin2definitiveedition) | 470 |  | Steam |
| 49 | State of Decay | [stateofdecay](https://www.nexusmods.com/stateofdecay) | 441 |  | Steam |
| 50 | Star Wars: Knights of the Old Republic II - The Sith Lords | [kotor2](https://www.nexusmods.com/kotor2) | 435 |  | Steam, GOG |
| 51 | Divinity: Original Sin 2 - Original Edition | [divinityoriginalsin2](https://www.nexusmods.com/divinityoriginalsin2) | 365 |  | Steam |
| 52 | Pathfinder: Kingmaker | [pathfinderkingmaker](https://www.nexusmods.com/pathfinderkingmaker) | 341 |  | Steam |
| 53 | Legend of Grimrock | [grimrock](https://www.nexusmods.com/grimrock) | 325 |  | Steam |
| 54 | Vampire the Masquerade: Bloodlines | [vampirebloodlines](https://www.nexusmods.com/vampirebloodlines) | 320 |  | Steam, GOG |
| 55 | X Rebirth | [xrebirth](https://www.nexusmods.com/xrebirth) | 303 |  | Steam |
| 56 | Enderal | [enderal](https://www.nexusmods.com/enderal) | 266 | Yes | Steam |
| 57 | Spyro Reignited Trilogy | [spyroreignitedtrilogy](https://www.nexusmods.com/spyroreignitedtrilogy) | 247 |  | Steam |
| 58 | Grim Dawn | [grimdawn](https://www.nexusmods.com/grimdawn) | 224 |  | Steam |
| 59 | Total War: Three Kingdoms | [totalwarthreekingdoms](https://www.nexusmods.com/totalwarthreekingdoms) | 207 | Yes | Steam, GOG, Epic |
| 60 | A Hat in Time | [ahatintime](https://www.nexusmods.com/ahatintime) | 201 | Yes | Steam |
| 61 | Bloodstained: Ritual of the Night | [bloodstainedritualofthenight](https://www.nexusmods.com/bloodstainedritualofthenight) | 192 |  | Steam, Epic |
| 62 | The Elder Scrolls Online | [elderscrollsonline](https://www.nexusmods.com/elderscrollsonline) | 184 |  | Steam |
| 63 | GreedFall | [greedfall](https://www.nexusmods.com/greedfall) | 180 |  | Steam |
| 64 | The Sims 3 | [thesims3](https://www.nexusmods.com/thesims3) | 158 |  | Steam |
| 65 | Elex | [elex](https://www.nexusmods.com/elex) | 148 |  | Steam |
| 66 | Galactic Civilizations III | [galacticcivilizations3](https://www.nexusmods.com/galacticcivilizations3) | 130 |  | Steam, GOG |
| 67 | Shadowrun Returns | [shadowrunreturns](https://www.nexusmods.com/shadowrunreturns) | 118 | Yes | Steam |
| 68 | Conan Exiles | [conanexiles](https://www.nexusmods.com/conanexiles) | 98 |  | Steam |
| 69 | Surviving Mars | [survivingmars](https://www.nexusmods.com/survivingmars) | 94 |  | Steam |
| 70 | Nehrim: At Fate's Edge | [nehrim](https://www.nexusmods.com/nehrim) | 77 | Yes | Steam |
| 71 | Team Fortress 2 | [teamfortress2](https://www.nexusmods.com/teamfortress2) | 76 | Yes | Steam |
| 72 | Wolcen: Lords Of Mayhem | [wolcenlordsofmayhem](https://www.nexusmods.com/wolcenlordsofmayhem) | 71 |  | Steam |
| 73 | Torchlight II | [torchlight2](https://www.nexusmods.com/torchlight2) | 53 |  | Steam, GOG |
| 74 | Mount & Blade: With Fire and Sword | [mbwithfireandsword](https://www.nexusmods.com/mbwithfireandsword) | 49 |  | Steam |
| 75 | Factorio | [factorio](https://www.nexusmods.com/factorio) | 24 |  | Steam |
| 76 | Kerbal Space Program | [kerbalspaceprogram](https://www.nexusmods.com/kerbalspaceprogram) | 20 |  | Steam |
| 77 | Oxygen Not Included | [oxygennotincluded](https://www.nexusmods.com/oxygennotincluded) | 20 |  | Steam |
| 78 | Dawn of Man | [dawnofman](https://www.nexusmods.com/dawnofman) | 19 |  | Steam, GOG |
| 79 | Prison Architect | [prisonarchitect](https://www.nexusmods.com/prisonarchitect) | 16 |  | Steam |
| 80 | Untitled Goose Game | [untitledgoosegame](https://www.nexusmods.com/untitledgoosegame) | 7 |  | - |
| 81 | Garden Paws | [gardenpaws](https://www.nexusmods.com/gardenpaws) | 3 |  | Steam |
| 82 | Neverwinter Nights: Enhanced Edition | [neverwinter](https://www.nexusmods.com/neverwinter) | 1 |  | Steam |
| 83 | Breaking Wheel | [breakingwheel](https://www.nexusmods.com/breakingwheel) | 1 |  | Steam |
| 84 | Neverwinter Nights | [neverwinter](https://www.nexusmods.com/neverwinter) | 1 |  | - |
| 85 | Halo: The Master Chief Collection | [masterchiefcollection](https://www.nexusmods.com/masterchiefcollection) | - | Yes | Steam |
<!-- GAME_SUPPORT_TABLE_END -->

### Planned Support (Coming Soon)

These are the remaining top 80 NexusMods games. Registry entries, game detection, and Vortex extension integration are being added. **Issue submissions and PRs welcome** for any of these games.

| Game | Mods | Slug | Engine/Notes |
|------|------|------|-------------|
| Star Wars: Battlefront II (2017) | 9.3k | `starwarsbattlefront22017` | Frostbite |
| Helldivers 2 | 9.1k | `helldivers2` | Simple file replacement |
| Mount & Blade II: Bannerlord | 7.3k | `mountandblade2bannerlord` | Module system |
| Elden Ring | 6.8k | `eldenring` | ModEngine2, EAC considerations |
| Marvel Rivals | 5.4k | `marvelrivals` | UE PAK mods |
| Red Dead Redemption 2 | 5k | `reddeadredemption2` | Script Hook |
| My Summer Car | 4.8k | `mysummercar` | MSCLoader |
| Resident Evil 4 (2023) | 4.6k | `residentevil42023` | RE Engine / REFramework |
| Ready or Not | 4.3k | `readyornot` | UE4 PAK mods |
| Marvel's Spider-Man Remastered | 4.3k | `marvelsspidermanremastered` | File replacement |
| Oblivion Remastered | 4k | `oblivionremastered` | UE5 — mod scene evolving |
| Devil May Cry 5 | 3.5k | `devilmaycry5` | RE Engine |
| Monster Hunter Wilds | 3.3k | `monsterhunterwilds` | REFramework |
| Dragon Age: Inquisition | 3.2k | `dragonageinquisition` | Frostbite / Frosty |
| Monster Hunter Rise | 3.1k | `monsterhunterrise` | REFramework |
| Blade & Sorcery: Nomad | 3.1k | `bladeandsorcerynomad` | Quest VR |
| Ace Combat 7 | 3k | `acecombat7skiesunknown` | UE4 PAK mods |
| Fallout 76 | 2.9k | `fallout76` | Online / limited modding |
| Stellar Blade | 2.4k | `stellarblade` | UE mods |
| Street Fighter 6 | 2.4k | `streetfighter6` | RE Engine |
| Mass Effect Legendary Edition | 2.4k | `masseffectlegendaryedition` | ME3Tweaks format |
| Dragon Age: The Veilguard | 2.4k | `dragonagetheveilguard` | Frostbite |
| Guitar Hero World Tour | 2.3k | `guitarheroworldtour` | Custom songs |
| Kingdom Come: Deliverance II | 2.3k | `kingdomcomedeliverance2` | New game |
| Valheim | 2.3k | `valheim` | BepInEx |
| Jurassic World Evolution 2 | 2.3k | `jurassicworldevolution2` | File mods |
| Batman: Arkham Knight | 2.2k | `batmanarkhamknight` | UE3 mods |
| Palworld | 2.1k | `palworld` | UE5 / BepInEx / UE4SS |
| Kingdom Hearts III | 2k | `kingdomhearts3` | UE4 PAK mods |
| Planet Zoo | 1.9k | `planetzoo` | File mods |
| Final Fantasy XIV | 1.9k | `finalfantasy14` | Penumbra / online game |
| Subnautica | 1.8k | `subnautica` | BepInEx / QMods |
| MGSV: The Phantom Pain | 1.8k | `metalgearsolidvtpp` | SnakeBite format |
| Final Fantasy VII Rebirth | 1.7k | `finalfantasy7rebirth` | UE4 PAK mods |
| Final Fantasy VII Remake | 1.6k | `finalfantasy7remake` | UE4 PAK mods |
| Resident Evil 2 (2019) | 1.5k | `residentevil22019` | RE Engine |
| S.T.A.L.K.E.R. 2 | 1.4k | `stalker2heartofchornobyl` | UE5 |
| Ghost Recon Breakpoint | 1.4k | `ghostreconbreakpoint` | File mods |
| Zoo Tycoon 2 | 1.4k | `zootycoon2` | Legacy |
| Jurassic World Evolution | 1.4k | `jurassicworldevolution` | File mods |
| Marvel's Spider-Man 2 | 1.3k | `marvelsspiderman2` | File replacement |
| DOOM Eternal | 1.3k | `doometernal` | File mods |
| Sifu | 1.3k | `sifu` | UE4 PAK mods |
| Mortal Kombat 1 | 1.2k | `mortalkombat` | UE mods |
| My Winter Car | 1.2k | `mywintercar` | MSCLoader |
| Dying Light 2 | 1.2k | `dyinglight2` | File mods |
| JoJo's ASBR | 1.2k | `jojosbizarreadventureallstarbattler` | UE4 |
| Yu-Gi-Oh Master Duel | 1.2k | `yugiohmasterduel` | Unity |

### Upcoming Enhancements for Existing Games

These features are in active development to bring more games up to **Full** support tier:

**Load order + LOOT sorting** (all Bethesda games):
Skyrim LE, Fallout: New Vegas, Fallout 3, Oblivion, Morrowind, Starfield, Enderal, Skyrim VR, Fallout 4 VR

**Script extender auto-install:**

| Extender | Game | Source |
|----------|------|--------|
| F4SE | Fallout 4 | [ianpatt/f4se](https://github.com/ianpatt/f4se) |
| NVSE | Fallout: New Vegas | [xNVSE/NVSE](https://github.com/xNVSE/NVSE) |
| MWSE | Morrowind | [MWSE/MWSE](https://github.com/MWSE/MWSE) |
| SFSE | Starfield | [gazzamc/sfse](https://github.com/gazzamc/sfse) |
| OBSE | Oblivion | Manual install |
| FOSE | Fallout 3 | Manual install |

**Mod framework auto-detection & install:**

| Framework | Games | Source |
|-----------|-------|--------|
| BepInEx | Valheim, Palworld, Subnautica, Unity games | [BepInEx/BepInEx](https://github.com/BepInEx/BepInEx) |
| REFramework | RE4, RE2, MH Rise, MH Wilds, DMC5, SF6 | [praydog/REFramework](https://github.com/praydog/REFramework) |
| ModEngine2 | Elden Ring, Dark Souls 3 | [soulsmods/ModEngine2](https://github.com/soulsmods/ModEngine2) |
| UE4SS | Palworld, Hogwarts Legacy, UE4/5 games | [UE4SS-RE/RE-UE4SS](https://github.com/UE4SS-RE/RE-UE4SS) |
| SMAPI | Stardew Valley | [Pathoschild/SMAPI](https://github.com/Pathoschild/SMAPI) |

**Beyond parity — planned "it just works" features:**

- **Auto-prerequisite detection** — Install a mod and Corkscrew tells you what frameworks are needed, offers one-click install
- **One-click "Make Moddable"** — Per-game button that installs all required frameworks and configures INIs
- **Mod bisect debugger** — Binary search wizard to find the mod causing crashes
- **Collection health monitor** — Background checks for mod updates, breaking changes, and Wine compatibility
- **Smart conflict resolution** — Semantic understanding of conflicts with auto-resolution suggestions

---

## Install

Download from the [Releases page](https://github.com/cashcon57/corkscrew/releases).

| Platform | Format | Notes |
|----------|--------|-------|
| **macOS** (Apple Silicon) | `.dmg` | Code-signed + notarized. Drag to Applications. |
| **macOS** (Intel) | `.dmg` | Code-signed + notarized. |
| **Linux** | `.AppImage` | Best for SteamOS / Steam Deck / any distro |
| **Linux** | `.deb` | Ubuntu 22.04+, Debian 12+ |
| **Linux** | `.rpm` | Fedora 37+ |

The app auto-updates — when a new version is available, a banner appears in-app. Updates are cryptographically signed.

<details>
<summary>Build from source</summary>

```bash
git clone https://github.com/cashcon57/corkscrew.git
cd corkscrew
npm install
cargo tauri build
```

Requires [Node.js](https://nodejs.org/) 18+ and [Rust](https://rustup.rs/). On Linux, install the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/#linux) first.

</details>

---

## Features

### Core
- **Staging-based deployment** — Mods are extracted to a staging folder, then deployed via hardlinks (copy fallback for cross-volume). Toggle mods without re-downloading.
- **Archive support** — `.zip`, `.7z`, `.rar`, `.tar.gz`, `.tar.xz`, `.tar.bz2` with smart data root detection
- **FOMOD wizard** — Interactive installer for mods with complex options. Choices can be saved as recipes and replayed.
- **Priority-based conflict resolution** — Drag-reorder mods to set who wins file conflicts
- **Profiles** — Snapshot mod states + plugin order, switch in one click. Optional per-profile save game backup.
- **Snapshots & rollback** — Auto-snapshot before destructive ops. One-click return to vanilla.

### NexusMods
- **OAuth sign-in** (browser-based PKCE) with API key fallback
- **NXM protocol handler** — Click "Download with Mod Manager" on the website
- **Browse & search** — Filter by category, author, endorsements. In-app mod detail pages.
- **Collections** — Browse, install, and delta-update NexusMods Collections (premium: automated; free: guided)
- **Endorsements** — Endorse mods directly from the app
- **Strict compliance** — Free users are always directed to the website for downloads. No automation for free accounts.

### Plugin Load Order
- LOOT-powered automatic sorting with masterlist fetching
- Manual drag-and-drop fine-tuning
- Custom LoadAfter/LoadBefore/Group rules
- Inline LOOT warnings per plugin

### Wabbajack
- Gallery browser with search, filters, NSFW toggle
- Local `.wabbajack` file parsing and analysis
- Full install pipeline: multi-source downloads (Nexus, HTTP, Mega, Google Drive, MediaFire, WJ CDN), BSDiff patching, BSA/BA2 packing, directive processing
- **Caveat:** Game file source extraction is incomplete — see [In Progress](#in-progress)

### Game Launching & Tools
- Launch through Wine/CrossOver/Whisky/Proton directly from the app
- Script extender auto-install (SKSE, F4SE) — version-aware
- UE4SS framework auto-install for Hogwarts Legacy (extending to all UE4/5 games)
- Native PAK database merger for UE games (replaces Windows-only HLModMerger.exe)
- Mod tools: detect, auto-install, launch (SSEEdit, Pandora, BodySlide, DynDOLOD, etc.)
- Wine bottle diagnostics with one-click fixes
- INI editor with presets (performance, ultra, Steam Deck)
- Crash log analysis with diagnosis and suggested fixes

### Game Version Rollback

- **Automatic version mismatch detection** — warns before installing a collection built for a different game version
- **Automated downgrade** via [DepotDownloader](https://github.com/SteamRE/DepotDownloader) — downloads older game versions directly from Steam's CDN
- **One-click version switching** — cached versions can be swapped instantly
- **Auto-capture** — Corkscrew records your game's current version on every launch, building a local version history for future rollbacks
- **Privacy-first** — Steam credentials are sent directly to Steam's servers via DepotDownloader and never stored by Corkscrew
- Works for any Steam game, not just Skyrim SE

### Shader Compatibility (Wine)
- **Community Shaders → ENB conversion wizard** — detects CS-dependent mods and helps swap to Wine-compatible ENB equivalents
- Smart detection: config-only files (harmless without CS DLL) are kept; actual CS ecosystem mods are disabled
- Essential SKSE plugin protection — po3_Tweaks, CrashLogger, BugFixesSSE, etc. are never disabled
- ENB binary auto-download and install with Wine compatibility patches
- Full revert support with snapshot restore

### AI Mod Assistant
- Local LLM chat (via [Ollama](https://ollama.com/)) — no cloud, fully private
- 20+ tool actions: list/enable/disable mods, search NexusMods, sort plugins, analyze crashes, switch profiles
- Memory-aware model recommendations (1.5 GB to 18 GB)
- Auto-installs Ollama if needed, auto-unloads models after 5 min

<details>
<summary>CLI tools</summary>

```bash
corkscrew --launch <game_id> <bottle_name> [--skse]
corkscrew --list-mods <game_id> <bottle_name>
corkscrew --search-mods <query> <game_id> <bottle_name>
corkscrew --find-file <pattern> <game_id> <bottle_name>
corkscrew --check-plugins <game_id> <bottle_name>
corkscrew --sync-plugins <game_id> <bottle_name>
corkscrew --mod-files <mod_name> <game_id> <bottle_name>
corkscrew --add-game <id> <name> <bottle> <path>
corkscrew --remove-game <id>
```

</details>

---

## Supported Platforms

### Wine Sources

| Source | macOS | Linux |
|--------|:-----:|:-----:|
| CrossOver | Yes | Yes |
| Whisky | Yes | — |
| Moonshine | Yes | — |
| Heroic (Wine) | Yes | Yes |
| Mythic | Yes | — |
| Lutris | — | Yes |
| Proton / Steam | — | Yes |
| Bottles | — | Yes |
| Native Wine | Yes | Yes |

### Games

80+ games auto-detected via the [Vortex game registry](https://github.com/Nexus-Mods/vortex-games), plus any Steam game discovered via appmanifest scanning. Custom games can be added via CLI. We're actively expanding to cover the top 80 most-modded games on NexusMods — see [Game Support Status](#game-support-status) for the full breakdown.

---

## SSE Engine Fixes for Wine

[SSE Engine Fixes for Wine](https://github.com/corkscrewmodding/SSEEngineFixesForWine) is a companion SKSE plugin maintained alongside Corkscrew. It's a Wine-compatible replacement for the original [SSE Engine Fixes](https://github.com/aers/EngineFixesSkyrim64), which crashes under Wine due to Intel TBB and d3dx9_42.dll preloader incompatibilities.

**What it does:**
- Fixes a Wine-specific bug that silently skips all form loading when plugin count exceeds ~600
- Provides a sentinel page architecture + Vectored Exception Handler for null-pointer and corrupted-vtable crashes
- Installs inline code-cave patches at hot crash sites for ~2ns validation vs ~50us per VEH fault
- Includes a watchdog thread that re-applies patches silently reverted by Wine's page management

**How Corkscrew uses it:**
Before every Skyrim SE launch on Wine, Corkscrew automatically:
1. Disables the original Engine Fixes (preloader + SKSE plugin + config hooks)
2. Downloads SSE Engine Fixes for Wine from GitHub if not present
3. Auto-updates the DLL when a new release is available
4. Preserves user config (`SSEEngineFixesForWine.toml`) across updates

This enables large modlists to load under Wine — 1700+ plugin lists reach main menu in ~2 minutes with 287K forms. However, very large modlists (Gate to Sovngarde scale) currently freeze on New Game due to hash table corruption in Skyrim's engine under Wine. Fixing this is active work.

---

## Architecture

Built with [Tauri v2](https://v2.tauri.app/) (Rust backend + web frontend), [Svelte 5](https://svelte.dev/) (SvelteKit, static adapter), and [SQLite](https://sqlite.org/) via rusqlite. ~15 MB app bundle vs 150+ MB for Electron.

<details>
<summary>How mods are installed</summary>

1. User drops an archive or clicks Install
2. Archive is extracted to a staging folder with smart data root detection
3. SHA-256 hashes are computed for every file and stored in the database
4. Hardlinks are created from staging to the game's Data directory (copy fallback for cross-volume)
5. Every deployed file is tracked in the deployment manifest
6. Disabling removes hardlinks; re-enabling recreates them
7. Uninstalling removes both deployment and staging

For **Collections**, the orchestrator resolves install order, downloads mods, applies FOMOD selections from the manifest, stages, deploys, and syncs plugin load order.

</details>

<details>
<summary>Project structure</summary>

```
src/                          Svelte 5 frontend (~57K lines)
├── lib/
│   ├── api.ts                Tauri IPC bindings (~223 commands)
│   ├── types.ts              TypeScript interfaces (~149 types)
│   ├── stores.ts             Svelte writable stores (global state)
│   └── components/
│       ├── mods/             Mod page workflow components (16 files)
│       │   ├── DeploymentPanel         Deploy/purge/health
│       │   ├── ConflictResolutionPanel AI conflict analysis + resolution
│       │   ├── SkseGameLaunchPanel     Game launch, SKSE, downgrade
│       │   ├── ModInstallFlow          Archive install + modlist naming
│       │   ├── ModBatchOperations      Bulk enable/disable/uninstall
│       │   ├── FomodReconfigurePanel   FOMOD recipe reconfiguration
│       │   ├── ModUpdatePanel          Update checking
│       │   ├── ModDetailPanel          Mod detail sidebar
│       │   └── ...                     Table row, header, filter, context menu
│       ├── collections/      Collection page workflow components (4 files)
│       │   ├── CollectionInstallWizard 6-step install flow
│       │   ├── NexusBrowsePanel        Nexus search/browse/download
│       │   ├── CollectionDeleteDialog  Multi-step delete confirmation
│       │   └── InterruptedInstallBanner Resume/dismiss partial installs
│       └── ...               Shared components (FOMOD wizard, shader wizard, etc.)
├── routes/
│   ├── mods/                 Mod management (~5.5K lines, orchestrator)
│   ├── plugins/              Plugin load order editor
│   ├── collections/          Collections browser + installer (~5K lines, orchestrator)
│   ├── modlists/             Wabbajack gallery + installer
│   ├── profiles/             Mod profiles
│   ├── logs/                 Crash log analysis
│   └── settings/             Config, tools, auth, diagnostics
└── app.css                   Design tokens + themes

src-tauri/src/                Rust backend (~92K lines, 101 modules, 938 tests)
├── lib.rs                    ~249 IPC commands + CLI
├── bottles.rs                Bottle detection (9 sources)
├── games.rs                  Game detection + plugin registry
├── installer.rs              Archive extraction + data root detection
├── deployer.rs               Hardlink deployment + atomic rollback
├── database.rs               SQLite with versioned migrations (v1→v22)
├── collections.rs            NexusMods Collections GraphQL client
├── collection_installer.rs   Collection install orchestrator
├── wabbajack_installer.rs    Wabbajack modlist pipeline
├── wabbajack_downloader.rs   Multi-source downloads (Nexus, CDN, MediaFire, etc.)
├── nexus.rs                  NexusMods REST API client
├── oauth.rs                  OAuth 2.0 + PKCE authentication
├── depot_downloader.rs       Steam game version rollback
├── loot.rs                   libloot integration
├── skse.rs                   Script extender management + Engine Fixes deploy
├── llm_chat.rs               Local LLM chat engine
├── vortex_runtime.rs         QuickJS sandbox for Vortex game extensions
└── plugins/                  Game-specific plugins (Skyrim SE, FO4)

Total: ~155K lines of code across 178 files
```

> **April 2, 2026 — Codebase structure refactor.** Decomposed two god components
> (mods page: 7,109→5,483 lines; collections page: 7,847→5,024 lines) into 11
> purpose-built workflow components. Applied 27 security and reliability fixes
> from a senior-engineer audit covering concurrent file writes, credential
> handling, path traversal, OAuth flow, JWT validation, and error propagation.
> Backend: 938 tests, 0 failures. Frontend: 0 type errors.

</details>

---

## Contributing

Bug reports, feature requests, and PRs welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for setup and guidelines.

```bash
git clone https://github.com/cashcon57/corkscrew.git
cd corkscrew
npm install
cargo tauri dev    # Dev mode with hot-reload
```

### Help Wanted

- **Game testing** — I need players to verify mod workflows for the 80 games listed in [Game Support Status](#game-support-status). If you play any of these games with mods under Wine/CrossOver/Proton, your testing and issue reports are invaluable.
- **Linux testing** — SteamOS, Steam Deck, Fedora, Ubuntu with Proton/Lutris
- **New game registry entries** — PRs adding games to `src-tauri/data/vortex_game_registry.json` are welcome. Each entry needs: `game_id`, `name`, `nexus_domain`, `steam_id`, `executable`, `mod_path`, `required_files`.
- **Dedicated game plugins** — Rust plugins for games needing special handling (load order, custom mod routing, script extender integration). See `src-tauri/src/plugins/` for examples.
- **Mod framework integration** — BepInEx, REFramework, ModEngine2, SMAPI auto-detection and install

---

## Acknowledgments

Corkscrew builds on many open-source projects:

- **[LOOT](https://loot.github.io/) / [libloot](https://github.com/loot/libloot)** — Plugin sorting engine (GPL-3.0, by [WrinklyNinja](https://github.com/Ortham))
- **[Wabbajack](https://www.wabbajack.org/)** — Pioneered automated modlist installation and the Stock Game approach (GPL-3.0)
- **[Vortex](https://github.com/Nexus-Mods/Vortex)** — Deployment model and Collections format (GPL-3.0)
- **[Mod Organizer 2](https://github.com/ModOrganizer2/modorganizer)** — Virtual filesystem concept and profile system (GPL-3.0)
- **[Wine Project](https://www.winehq.org/)** / **[CrossOver](https://www.codeweavers.com/crossover)** — The foundation for running Windows games on macOS and Linux
- **[Nexus Mods](https://www.nexusmods.com/)** — Mod hosting, API, and the modding community
- **[Jackify](https://github.com/Omni-guides/Jackify)** — Demonstrated Wabbajack modlist installation on Linux
- **[SulfurNitride](https://github.com/SulfurNitride)** — [NaK](https://github.com/SulfurNitride/NaK) (MO2 Linux setup automation), [Radium-Textures](https://github.com/SulfurNitride/Radium-Textures) (native Rust texture optimizer), [Nexus-Collection-To-MO2-Bridge](https://github.com/SulfurNitride/Nexus-Collection-To-MO2-Bridge) — pioneering Linux modding tooling
- **[DepotDownloader](https://github.com/SteamRE/DepotDownloader)** — Steam depot downloader for automated game version rollback (GPL-2.0, by [SteamRE](https://github.com/SteamRE)). Downloaded on demand, not bundled — your Steam credentials are sent directly to Steam's servers and never stored by Corkscrew.

<details>
<summary>Modding tool authors</summary>

If you use these tools, please consider supporting their creators:

- [SSEEdit / xEdit](https://github.com/TES5Edit/TES5Edit) by ElminsterAU — [Ko-fi](https://ko-fi.com/elminsterau)
- [DynDOLOD](https://dyndolod.info/) by Sheson — [Ko-fi](https://ko-fi.com/sheson)
- [Pandora Behaviour Engine](https://github.com/Monitor221hz/Pandora-Behaviour-Engine) by Monitor221hz — [Patreon](https://www.patreon.com/monitorhz)
- [Nemesis](https://github.com/ShikyoKira/Project-New-Reign---Nemesis-Main) by ShikyoKira — [Patreon](https://www.patreon.com/shikyokira)
- [Cathedral Assets Optimizer](https://github.com/Guekka/Cathedral-Assets-Optimizer) by Guekka — [GitHub Sponsors](https://github.com/sponsors/Guekka)
- [BodySlide](https://github.com/ousnius/BodySlide-and-Outfit-Studio) by ousnius
- [BethINI](https://www.nexusmods.com/skyrimspecialedition/mods/631) by DoubleYou
- [Wrye Bash](https://github.com/wrye-bash/wrye-bash)
- [SKSE Team](https://skse.silverlock.org/)

</details>

<details>
<summary>Third-party licenses</summary>

Corkscrew is GPL-3.0-or-later. Key dependencies:

- **libloot / esplugin / libloadorder** — GPL-3.0, Copyright Oliver Shercliff
- **Tauri** — Apache-2.0 / MIT, Copyright Tauri Programme
- **DOMPurify** — Apache-2.0 / MPL-2.0, Copyright Mario Heiderich, Cure53

Auto-downloaded tools (SSEEdit, Pandora, BodySlide, etc.) are standalone executables, not linked or redistributed. SKSE is downloaded from the [official GitHub repository](https://github.com/ianpatt/skse64/releases).

Full license text: https://www.gnu.org/licenses/gpl-3.0.html

</details>

---

## Special Thanks

Community members who've gone above and beyond helping improve Corkscrew:

| User          | Contribution                                                               |
|---------------|----------------------------------------------------------------------------|
| **@maq6teen** | Diagnosed the `liblzma` missing dependency crash on macOS Tahoe (v0.14.3) |

---

## Support

If Corkscrew is useful to you:

[![Ko-fi](https://img.shields.io/badge/Ko--fi-Support%20Corkscrew-FF5E5B?logo=ko-fi&logoColor=white&style=for-the-badge)](https://ko-fi.com/cash508287)

## License

GPL-3.0-or-later. See [LICENSE](LICENSE) for details.
