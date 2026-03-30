<p align="center">
  <img src="brand-kit/corkscrew-icon-256.png" width="200" alt="Corkscrew">
</p>

<h1 align="center">Corkscrew</h1>

<p align="center">
  <strong>A native mod manager for Wine games on macOS and Linux.</strong>
</p>

<p align="center">
  <a href="https://corkscrewmodmanager.com">🌐 Website</a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/macOS-10.15+-000000?logo=apple&logoColor=white" alt="macOS">
  <img src="https://img.shields.io/badge/Linux-SteamOS%20%7C%20Ubuntu%20%7C%20Fedora-FCC624?logo=linux&logoColor=black" alt="Linux">
  <img src="https://img.shields.io/badge/License-GPL--3.0-blue" alt="License">
  <a href="https://ko-fi.com/cash508287"><img src="https://img.shields.io/badge/Ko--fi-Support%20Corkscrew-FF5E5B?logo=ko-fi&logoColor=white" alt="Ko-fi"></a>
</p>

<br>

Corkscrew installs, manages, and organizes mods for Windows games running through [CrossOver](https://www.codeweavers.com/crossover), [Whisky](https://getwhisky.app/), [Lutris](https://lutris.net/), [Proton](https://github.com/ValveSoftware/Proton), and other Wine-based compatibility layers. No Windows VM required.

It works by reading and writing directly to your Wine bottle's filesystem — the same way the game sees it. Your bottles, your mods, no middleman.

---

## Quick Links

- [What Works Today](#what-works-today) — Current status, tested features, known issues
- [Game Support Status](#game-support-status) — Full breakdown of 80 games: support tiers, planned additions, upcoming enhancements
- [Install](#install) — Download links and build-from-source instructions
- [Features](#features) — Core features, NexusMods, load order, Wabbajack, shader compat, AI assistant
- [Supported Platforms](#supported-platforms) — Wine sources (CrossOver, Proton, Lutris, etc.) and game list
- [SSE Engine Fixes for Wine](#sse-engine-fixes-for-wine) — Companion SKSE plugin for large modlists under Wine
- [Architecture](#architecture) — Tech stack, project structure, how mods are installed
- [Contributing](#contributing) — Setup, help wanted, how to add game support
- [Acknowledgments](#acknowledgments) — Projects and authors we build on

---

## What Works Today

Corkscrew has been **tested extensively with Skyrim Special Edition** on macOS (Apple Silicon, CrossOver). That's the honest baseline. Here's where things stand:

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
- **Large modlists don't work yet.** Gate to Sovngarde (1700+ plugins) installs and reaches main menu but freezes on New Game due to hash table corruption in Skyrim's engine under Wine. **This is the current bottleneck** — we are actively iterating on [SSE Engine Fixes for Wine](https://github.com/corkscrewmodding/SSEEngineFixesForWine) to solve this. Smaller modlists like [Immersive & Pure](https://next.nexusmods.com/skyrimspecialedition/collections/vaakhi) work end-to-end including New Game and gameplay.
- **Wabbajack modlists** — The install pipeline is built (multi-source downloads, BSDiff patching, BSA packing, directive processing), but game file source extraction is incomplete. Complex modlists that depend on vanilla game files as patch sources will partially fail. This is the other main blocker for v1.0.

### Untested
- **Every game except Skyrim SE.** 80+ games are auto-detected and support basic mod deployment, but only Skyrim SE and Fallout 4 have full-featured plugins (load order, LOOT, script extender, INI presets, crash logs). We haven't verified the mod workflow end-to-end for other games yet.
- **Linux.** The app builds for Linux, handles Linux paths, and supports Proton/Lutris/SteamOS bottles. But primary development and testing happens on macOS. Community testing and feedback on Linux is very welcome.

---

## Game Support Status

We're actively expanding support to cover the **top 80 most-modded games on NexusMods**. The architecture is game-agnostic — mod installation, staging, deployment, collections, and profiles work for any game. What varies is the depth of game-specific integration.

> **Note on platform compatibility:** Corkscrew is a cross-platform app. Some games listed below may not run under Wine/CrossOver on macOS or under Proton on Linux (due to anti-cheat, DRM, VR requirements, or engine limitations). We're adding mod management support for all top 80 games regardless, because Corkscrew is designed to eventually support native Windows as well. If a game doesn't run on your platform today, the mod tooling will be ready when it does — or when you use Corkscrew on Windows.

### Support Tiers

| Tier | What It Means | Games |
|------|---------------|-------|
| **Full** | Load order, LOOT sorting, script extender auto-install, INI presets, crash logs, mod tools | Skyrim SE, Fallout 4 |
| **Enhanced** | Dedicated plugin with custom mod routing, saves dir, deploy hooks | Hogwarts Legacy |
| **Standard** | Auto-detection, mod install/deploy, collections, profiles, NexusMods integration | 36 games (see below) |
| **Planned** | Not yet in registry — will be added in upcoming releases | 41 games (see below) |

### Full Support

| Game | Mods on Nexus | Load Order | Script Extender | Collections |
|------|--------------|------------|-----------------|-------------|
| Skyrim Special Edition | 129k | LOOT | SKSE (auto-install) | Full |
| Fallout 4 | 72.2k | LOOT | — | Full |

### Standard Support (In Registry)

These games are auto-detected and support the full generic mod workflow. **Testing is needed** to verify the mod pipeline works end-to-end for each game. If you play any of these, please test and report issues!

Skyrim LE (72.9k mods) | Fallout: New Vegas (40.4k) | Oblivion (33k) | Stardew Valley (29.8k) | Cyberpunk 2077 (20.5k) | Fallout 3 (17k) | Baldur's Gate 3 (17.3k) | Morrowind (14.7k) | Starfield (12.3k) | Blade & Sorcery (8.4k) | The Witcher 3 (8.4k) | 7 Days to Die (7.1k) | Monster Hunter: World (6.3k) | The Sims 4 (4.4k) | Dragon Age: Origins (3.9k) | No Man's Sky (2.6k) | Sekiro (1.7k) | Darkest Dungeon (1.6k) | Dark Souls 3 (1.5k) | Kingdom Come: Deliverance (1.5k) | Dragon Age 2 (1.5k) | X4: Foundations (1.5k) | Dark Souls (1.4k) | Kenshi (1.4k) | Hogwarts Legacy (1.4k) | Mount & Blade: Warband (1.4k) | War Thunder (1.2k) | Dark Souls 2 (1.2k) | Mount & Blade (2k) | Halo: MCC (1.9k)

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
- Mod tools: detect, auto-install, launch (SSEEdit, Pandora, BodySlide, DynDOLOD, etc.)
- Wine bottle diagnostics with one-click fixes
- INI editor with presets (performance, ultra, Steam Deck)
- Crash log analysis with diagnosis and suggested fixes

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
src/                          Svelte frontend
├── lib/
│   ├── api.ts                Tauri IPC bindings (~223 commands)
│   ├── types.ts              TypeScript interfaces
│   └── components/           UI components (FOMOD wizard, conflict panel, etc.)
├── routes/
│   ├── mods/                 Mod management (table, batch ops, keyboard nav)
│   ├── plugins/              Plugin load order editor
│   ├── collections/          NexusMods Collections browser + installer
│   ├── modlists/             Wabbajack gallery + installer
│   ├── profiles/             Mod profiles
│   ├── logs/                 Crash log analysis
│   └── settings/             Config, tools, auth, diagnostics
└── app.css                   Design tokens + themes

src-tauri/src/                Rust backend (~54 modules, 874+ tests)
├── lib.rs                    ~249 IPC commands + CLI
├── bottles.rs                Bottle detection (9 sources)
├── games.rs                  Game detection + plugin registry
├── installer.rs              Archive extraction + data root detection
├── deployer.rs               Hardlink deployment + atomic rollback
├── database.rs               SQLite with versioned migrations (v1→v19)
├── collections.rs            NexusMods Collections GraphQL client
├── collection_installer.rs   Collection install orchestrator
├── wabbajack_installer.rs    Wabbajack modlist pipeline
├── nexus.rs                  NexusMods REST API client
├── loot.rs                   libloot integration
├── skse.rs                   Script extender management + Engine Fixes deploy
├── llm_chat.rs               Local LLM chat engine
├── vortex_runtime.rs         QuickJS sandbox for Vortex game extensions
└── plugins/                  Game-specific plugins (Skyrim SE, FO4)
```

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

- **Game testing** — We need players to verify mod workflows for the 80 games listed in [Game Support Status](#game-support-status). If you play any of these games with mods under Wine/CrossOver/Proton, your testing and issue reports are invaluable.
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

## Support

If Corkscrew is useful to you:

[![Ko-fi](https://img.shields.io/badge/Ko--fi-Support%20Corkscrew-FF5E5B?logo=ko-fi&logoColor=white&style=for-the-badge)](https://ko-fi.com/cash508287)

## License

GPL-3.0-or-later. See [LICENSE](LICENSE) for details.
