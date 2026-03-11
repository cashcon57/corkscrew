<p align="center">
  <img src="brand-kit/corkscrew-icon-256.png" width="200" alt="Corkscrew">
</p>

<h1 align="center">Corkscrew</h1>

<p align="center">
  <strong>A native mod manager for Wine games on macOS and Linux.</strong>
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

### Known Issues
- **Large modlists don't work yet.** Gate to Sovngarde (1700+ plugins) installs and reaches main menu but freezes on New Game due to hash table corruption in Skyrim's engine under Wine. **This is the current bottleneck** — we are actively iterating on [SSE Engine Fixes for Wine](https://github.com/corkscrewmodding/SSEEngineFixesForWine) to solve this. Smaller modlists like [Immersive & Pure](https://next.nexusmods.com/skyrimspecialedition/collections/vaakhi) work end-to-end including New Game and gameplay.
- **Wabbajack modlists** — The install pipeline is built (multi-source downloads, BSDiff patching, BSA packing, directive processing), but game file source extraction is incomplete. Complex modlists that depend on vanilla game files as patch sources will partially fail. This is the other main blocker for v1.0.

### Untested
- **Every game except Skyrim SE.** 80+ games are auto-detected and support basic mod deployment, but only Skyrim SE and Fallout 4 have full-featured plugins (load order, LOOT, script extender, INI presets, crash logs). We haven't verified the mod workflow end-to-end for other games yet.
- **Linux.** The app builds for Linux, handles Linux paths, and supports Proton/Lutris/SteamOS bottles. But primary development and testing happens on macOS. Community testing and feedback on Linux is very welcome.

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

80+ games auto-detected via the [Vortex game registry](https://github.com/Nexus-Mods/vortex-games), plus any Steam game discovered via appmanifest scanning. Custom games can be added via CLI.

**Full-featured support** (load order, LOOT, script extender, INI, crash logs, mod tools):
- Skyrim Special Edition
- Fallout 4

**Basic support** (auto-detection + mod deployment): everything else. See the full list:

<details>
<summary>View all 80+ supported games</summary>

Skyrim, Fallout: New Vegas, Oblivion, Stardew Valley, Cyberpunk 2077, Fallout 3, Baldur's Gate 3, Morrowind, Starfield, Blade & Sorcery, The Witcher 3, 7 Days to Die, Monster Hunter: World, The Sims 4, Dragon Age: Origins, No Man's Sky, Enderal, Skyrim VR, Fallout 4 VR, Sekiro, Darkest Dungeon, Dragon Age 2, Kingdom Come: Deliverance, Dark Souls, Kenshi, Mount & Blade: Warband, X4: Foundations, and 50+ more.

</details>

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

src-tauri/src/                Rust backend (~53 modules, 715+ tests)
├── lib.rs                    ~249 IPC commands + CLI
├── bottles.rs                Bottle detection (9 sources)
├── games.rs                  Game detection + plugin registry
├── installer.rs              Archive extraction + data root detection
├── deployer.rs               Hardlink deployment + atomic rollback
├── database.rs               SQLite with versioned migrations (v1→v17)
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
- **Linux testing** — SteamOS, Steam Deck, Fedora, Ubuntu with Proton/Lutris
- **Non-Skyrim game testing** — Try the mod workflow for any of the 80+ supported games
- **Enhanced game plugins** — Adding full support for Oblivion, Fallout 3/NV, Starfield, Morrowind

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
