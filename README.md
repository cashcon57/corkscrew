<p align="center">
  <img src="brand-kit/corkscrew-icon-256.png" width="200" alt="Corkscrew">
</p>

<h1 align="center">Corkscrew</h1>

<p align="center">
  <strong>A native mod manager for Wine games on macOS and Linux.</strong>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Tauri-v2-24C8D8?logo=tauri&logoColor=white" alt="Tauri v2">
  <img src="https://img.shields.io/badge/Svelte-5-FF3E00?logo=svelte&logoColor=white" alt="Svelte 5">
  <img src="https://img.shields.io/badge/Rust-2021-DEA584?logo=rust&logoColor=white" alt="Rust">
  <img src="https://img.shields.io/badge/TypeScript-5-3178C6?logo=typescript&logoColor=white" alt="TypeScript">
  <img src="https://img.shields.io/badge/SQLite-3-003B57?logo=sqlite&logoColor=white" alt="SQLite">
  <img src="https://img.shields.io/badge/macOS-10.15+-000000?logo=apple&logoColor=white" alt="macOS">
  <img src="https://img.shields.io/badge/Linux-SteamOS%20%7C%20Fedora%20%7C%20Ubuntu-FCC624?logo=linux&logoColor=black" alt="Linux">
  <img src="https://img.shields.io/badge/License-GPL--3.0-blue" alt="License">
  <a href="https://ko-fi.com/cash508287"><img src="https://img.shields.io/badge/Ko--fi-Support%20Corkscrew-FF5E5B?logo=ko-fi&logoColor=white" alt="Ko-fi"></a>
</p>

<br>

Corkscrew installs, manages, and organizes mods for Windows games running through [CrossOver](https://www.codeweavers.com/crossover), [Moonshine](https://github.com/ybmeng/moonshine), [Lutris](https://lutris.net/), [Proton](https://github.com/ValveSoftware/Proton), and other Wine-based compatibility layers — no Windows VM required.

It works by reading and writing directly to your Wine bottle's filesystem, the same way the game itself sees it. Your bottles, your mods, no middleman.

> **v1.7** — In-app mod detail pages, SKSE/tool detection fixes, Pandora→Nemesis/FNIS suppression, README rendering fixes, full Wabbajack install pipeline, and 80+ supported games.

---

## Table of Contents

- [Installation](#installation)
  - [Requirements](#requirements)
  - [From Release](#from-release) — [macOS](#macos) · [Linux](#linux)
  - [From Source](#from-source)
  - [Auto-Updates](#auto-updates)
- [Features](#features)
  - [Mod Management](#mod-management)
  - [Mods Page UX](#mods-page-ux)
  - [Nexus Mods Integration](#nexus-mods-integration)
  - [Plugin Load Order](#plugin-load-order)
  - [Profiles](#profiles)
  - [Wabbajack Modlists](#wabbajack-modlists)
  - [Crash Log Analysis](#crash-log-analysis)
  - [Game Launching & Tools](#game-launching--tools)
  - [Platform & UI](#platform--ui)
- [Supported Platforms](#supported-platforms)
  - [Bottle Sources](#bottle-sources)
  - [Games](#games)
  - [Keyboard Shortcuts](#keyboard-shortcuts)
- [Current Status](#current-status)
  - [What Works](#what-works)
  - [Known Limitations](#known-limitations)
  - [Roadmap](#roadmap)
- [Architecture](#architecture)
  - [Why These Technologies](#why-these-technologies)
  - [Project Structure](#project-structure)
  - [How Mods Are Installed](#how-mods-are-installed)
- [Contributing](#contributing)
  - [Development Quick Start](#development-quick-start)
  - [Areas Where Help Is Wanted](#areas-where-help-is-wanted)
- [Acknowledgments](#acknowledgments)
  - [Projects We Build Upon](#projects-we-build-upon)
  - [Libraries & Tools](#libraries--tools)
  - [Modding Tool Authors](#modding-tool-authors)
  - [Communities & Services](#communities--services)
- [Third-Party Licenses](#third-party-licenses)
- [Support](#support)
- [License](#license)

---

## Installation

### Requirements

- **macOS** 10.15+ (Catalina or later)
- **Linux** with GTK 3 and WebKitGTK 4.1 (Ubuntu 22.04+, Fedora 37+, SteamOS 3.x)
- A Wine-based runner (CrossOver, Moonshine, Lutris, Proton, etc.)

### From Release

Download the latest release for your platform from the [Releases page](https://github.com/cashcon57/corkscrew/releases):

#### macOS

| Format | Architecture |
|--------|-------------|
| `.dmg` | Apple Silicon (M1+) and Intel |

After dragging to Applications:

> **Note:** The app is not yet code-signed with an Apple Developer certificate. macOS Gatekeeper will block it on first launch. Run:
> ```bash
> xattr -cr /Applications/Corkscrew.app
> ```
> Or: right-click the app → **Open** → click "Open" in the dialog.

#### Linux

| Format | Best for |
|--------|----------|
| **AppImage** | SteamOS / Steam Deck, Arch, any distro |
| `.deb` | Ubuntu 22.04+, Debian 12+, Pop!_OS, Linux Mint |
| `.rpm` | Fedora 37+, openSUSE |

**SteamOS / Steam Deck:**
```bash
# Download the AppImage, then make it executable
chmod +x Corkscrew_*.AppImage

# SteamOS may need this if FUSE is unavailable (Gaming Mode):
APPIMAGE_EXTRACT_AND_RUN=1 ./Corkscrew_*.AppImage

# Or extract and run directly:
./Corkscrew_*.AppImage --appimage-extract
./squashfs-root/AppRun
```

**Ubuntu / Debian:**
```bash
sudo apt install ./corkscrew_*.deb
```
Requires `libwebkit2gtk-4.1-0` and `libgtk-3-0` (installed automatically as dependencies).

**Fedora:**
```bash
sudo dnf install ./corkscrew-*.rpm
```
Requires `webkit2gtk4.1` and `gtk3` (installed automatically as dependencies).

**Arch Linux:**

Use the AppImage, or build from source.

### From Source

```bash
git clone https://github.com/cashcon57/corkscrew.git
cd corkscrew
npm install
cargo tauri build
```

Requires [Node.js](https://nodejs.org/) 18+ and a [Rust toolchain](https://rustup.rs/). On Linux, install the [Tauri system dependencies](https://v2.tauri.app/start/prerequisites/#linux) first.

### Auto-Updates

Corkscrew includes an in-app auto-updater. When a new version is published on GitHub, the app will detect it and offer to update. Updates are cryptographically signed and verified before installation.

---

## Features

### Mod Management
- **Staging-based deployment** — Mods are extracted to a staging folder first, then deployed to the game directory via hardlinks (with copy fallback). Toggle mods on/off without re-downloading or re-extracting. Atomic deployment with rollback on partial failure ensures the game directory is never left in a broken state.
- **Mod installation** — Handles `.zip`, `.7z`, `.rar`, `.tar.gz`, `.tar.xz`, and `.tar.bz2` archives with smart data root detection, or drag-and-drop files directly onto the app.
- **Priority-based conflict resolution** — Drag-reorder mods to set deployment priority. Higher-priority mods win when files overlap, with a visual conflict panel showing which mods override which files.
- **FOMOD wizard** — Step-by-step interactive installer for mods using the FOMOD XML format, with radio/checkbox groups, option descriptions, and type badges.
- **FOMOD choice replay** — Save and export FOMOD installer choices as shareable JSON recipes, then replay them on reinstall or share with others.
- **Mod integrity verification** — SHA-256 hashes are stored per file; verify staging integrity on demand.
- **Mod version rollback** — Track mod versions and roll back to previous versions with snapshot support.
- **Modlist export/import** — Export your mod setup as a portable JSON modlist and import it on another machine or share it with others, with diff comparison between modlists.
- **Mod dependency tracking** — Define requires/conflicts/patches relationships between mods. The dependency checker surfaces missing requirements and active conflicts before you launch.
- **Pre-flight installation checks** — Run a comprehensive pre-deployment check covering disk space, staging integrity, bottle health, and file conflicts before deploying.
- **Disk space budget** — Real-time disk usage dashboard showing staging, deployment, and available space with per-install impact estimates.
- **Auto-categorization** — Mods are automatically classified by content type (Plugins, Textures, Models, SKSE Plugins, UI, Audio, Scripts, ENB, ReShade) using file-path heuristics.

### Mods Page UX
- **Sortable column headers** — Click any column header (Name, Version, Category, Files, Date) to sort. Click again to reverse. Semantic version comparison handles `1.2.3` vs `1.10.0` correctly.
- **Three view modes** — Switch between Flat (standard table), Collection (grouped by collection with collapsible headers), and Category (MO2-style tree grouped by auto-category).
- **Collection dropdown** — Filter the mod list by installed collection or view standalone mods, via a dropdown below the game banner.
- **Category chips** — Color-coded pills next to mod names showing their auto-detected content category.
- **Search highlighting** — Search terms are highlighted in mod names as you type.
- **Right-click context menu** — Right-click any mod row for quick access to Toggle, Edit Tags, Edit Notes, Reinstall, Check for Update, Open on Nexus, and Uninstall.
- **Batch selection** — Select multiple mods with checkboxes for bulk Enable All, Disable All, or Uninstall via a floating action bar.
- **Keyboard navigation** — Full keyboard support: arrow keys to navigate rows, Space to toggle, Enter for details, Ctrl/Cmd+F to search, Ctrl/Cmd+A to select all, Ctrl/Cmd+D to deploy.
- **Deploy progress** — The Deploy button shows a real-time progress bar with mod-by-mod status during deployment.
- **Persistent notification log** — All success/error/warning notifications are logged to a persistent database. Click the bell icon in the sidebar to review past notifications with timestamps.

### Nexus Mods Integration
- **API key authentication** — Connect your Nexus Mods account via API key to access premium features. SSO/OAuth module is implemented and ready for use once NexusMods approves the application.
- **NXM protocol handling** — Registered as an `nxm://` protocol handler. Users click "Download with Mod Manager" on the Nexus Mods website, and Corkscrew receives and processes the NXM link.
- **Strict download compliance** — Corkscrew **never automates downloads for free NexusMods users**. Free users are always directed to the Nexus Mods website to click "Slow Download" manually. Only premium users get API-initiated downloads. This is enforced at the API layer in `nexus.rs::get_download_links()`.
- **Browse Nexus Mods** — Premium users can search and filter NexusMods mods with advanced filters (category, author, update period, min downloads/endorsements), multiple sort options, and NexusMods-style filter pills. Free users browse via an embedded NexusMods web view within the app.
- **In-app mod detail pages** — Click any mod in the Browse grid to view its full detail page with hero image, description (sanitized HTML), stats, and file list — without leaving the app. "View on NexusMods" link for the full website experience.
- **Direct mod download & install** — Premium users can download and install mods directly from the Browse page: pick a file from the mod's files list (grouped by category), download with real-time progress, and auto-deploy in one step.
- **Embedded web views** — Toggle between in-app API browsing and an embedded NexusMods/Wabbajack website view. Powered by Tauri v2 multi-webview (native child webview, not an iframe). Available on Browse Nexus, Collections, and Wabbajack Gallery pages.
- **Collections browser** — Browse NexusMods Collections via the GraphQL v2 API with search, sorting, advanced filtering, and detailed mod/revision views.
- **Collection installation** — Premium users can install entire NexusMods Collections with one click. The orchestrator resolves install order, downloads mods via the NexusMods API, handles FOMOD selections from the collection manifest, deploys files, and applies the collection's plugin load order. Plugin load order sync works for all games with plugin support (Skyrim SE, Fallout 4, etc.), not just Skyrim SE. Free users see a list of mods with links to download manually from the Nexus website.
- **Update checking** — Check installed mods against Nexus for available updates.
- **Collection diff** — Compare your locally installed collection against the author's latest revision to see added, removed, and updated mods.
- **Tool requirement detection** — Before installing a Collection or Wabbajack modlist, Corkscrew scans for required modding tools (SKSE, Nemesis, BodySlide, etc.) and prompts you to install missing tools before proceeding. Integrated tools (LOOT) are hidden since they're built into Corkscrew. Pandora automatically suppresses Nemesis/FNIS requirements since it's backwards-compatible with both.
- **Rate limit compliance** — All API calls respect NexusMods rate limits with graceful error handling (skip on failure, no aggressive retries).

### Plugin Load Order
- **LOOT-powered sorting** — Automatic plugin sorting using [libloot](https://github.com/loot/libloot) (the same engine behind LOOT), with masterlist fetching from GitHub.
- **Manual drag-and-drop reorder** — Fine-tune your load order after LOOT sorts.
- **Plugin enable/disable** — Toggle individual plugins without touching the mod.
- **Plugin warnings** — LOOT messages (info, warnings, errors) displayed inline per plugin.
- **Custom plugin rules** — Define LoadAfter, LoadBefore, and Group rules for per-plugin ordering beyond what LOOT provides.

### Profiles
- **Save and switch** — Snapshot your current mod states, priorities, and plugin load order into named profiles.
- **Instant activation** — Switch profiles in one click: purges current deployment, applies the target profile's states, redeploys, and restores plugin order.
- **Auto-profile on collection install** — A named profile snapshot is automatically created after each collection installation.

### Wabbajack Modlists
- **Gallery browser** — Browse the full Wabbajack modlist gallery with search, game filtering, NSFW 3-state toggle (hide/show/only), and advanced filters (install size, tags). Toggle between in-app gallery and embedded Wabbajack website.
- **Modlist metadata** — View archive counts, download/install sizes, tags, and version info.
- **Local .wabbajack parsing** — Open and analyze downloaded `.wabbajack` files to see directive breakdowns, archive source breakdown (Nexus, HTTP, Mega, Google Drive, etc.), and compatibility info.
- **Full modlist installation** — Complete end-to-end pipeline with real downloads and deployment:
  - **Download phase** — Multi-source download engine supporting NexusMods (premium), HTTP, Google Drive, MEGA, MediaFire (link scraping), Wabbajack CDN, ModDB, game file sources, and manual downloads. Semaphore-based concurrency (8 parallel), xxHash64 verification, download caching, and progress events.
  - **Directive phase** — Processes all Wabbajack directive types: FromArchive (file extraction), PatchedFromArchive (BSDiff binary patching), InlineFile/RemappedInlineFile (embedded data with path substitution), MergedPatch (multi-source merge patching), CreateBSA (staging for BSA packing), TransformedTexture (texture copy), and IgnoredDirectly.
  - **Deploy phase** — Hardlink-first deployment with atomic rollback on partial failure. Creates a mod record and deploys all processed files to the game's data directory.
  - Pre-flight checks, cancellation support, and real-time progress tracking throughout.
- **Tool detection** — Scans the modlist for required tools and prompts for installation before proceeding.

### Crash Log Analysis
- **Automatic detection** — Scans for Skyrim crash logs (from .NET Script Framework or Crash Logger) in your bottle.
- **Crash diagnosis** — Parses crash logs to identify exception types, faulting modules, involved plugins, and SKSE plugins.
- **Suggested actions** — Provides actionable recommendations (update mod, disable mod, sort load order, check VRAM, etc.) with confidence ratings.
- **Game session tracking** — Log play sessions with automatic duration tracking, crash detection, and stability summaries. Track which mods were changed between sessions to correlate changes with crashes.

### Game Launching & Tools
- **Game launching** — Play your modded game straight from Corkscrew, through whatever Wine layer the bottle uses.
- **SKSE auto-install** — Auto-detect your Skyrim version and install the correct SKSE build from GitHub with one click. Game-version-aware: picks the right SKSE release for SE (1.5.97), AE (1.6.x), or latest AE builds. Correctly detected when already installed.
- **SKSE launching** — Launch through SKSE with one click after installation. Compatibility checks against your game version.
- **Skyrim SE downgrade** — Detect your Skyrim version via SHA-256 hash and create a "Stock Game" copy to lock v1.5.97 and prevent Steam auto-updates (same approach pioneered by Wabbajack).
- **Display scaling fix** — Automatically fix Skyrim SE display scaling issues in CrossOver on macOS by detecting your screen resolution and forcing exclusive fullscreen mode.
- **INI settings manager** — Browse, search, and edit game INI files (Skyrim.ini, SkyrimPrefs.ini, etc.) with built-in presets for common configurations like Steam Deck optimization, ultra graphics, and performance profiles.
- **Wine bottle diagnostics** — Comprehensive health check for Wine bottles: validates drive_c, AppData, DXVK (Linux) / D3DMetal (macOS), DLL overrides, Visual C++ redistributables, .NET, Windows version, Retina/HiDPI display, and user directories, with one-click auto-fixes for common issues.
- **Mod tools management** — Detect, auto-install, launch, and uninstall modding tools (SSEEdit, BethINI, DynDOLOD, BodySlide, Nemesis, Pandora, Wrye Bash, etc.) directly from the settings page. Support links for tool authors included where available.
- **Custom executables** — Define custom .exe launch targets per game.
- **Game file integrity** — Take snapshots of your game directory to detect modified, unknown, or missing files later.
- **Bottle configuration** — View and modify Wine bottle settings (Windows version, MSync, MetalFX, DXMT, environment variables) directly from Corkscrew.

### Platform & UI
- **Automatic bottle detection** — Finds CrossOver, Whisky, Moonshine, Heroic, Mythic, Lutris, Proton, Bottles, and native Wine prefixes.
- **Game scanning** — Discovers 80+ supported titles across all bottles via Steam and GOG path scanning.
- **Platform-aware settings** — Detects macOS, Linux, and SteamOS, showing platform-relevant compatibility layers and recommendations.
- **macOS vibrancy** — Native translucent materials that follow the active window state.
- **Light and dark themes** — System-following by default with manual toggle.
- **Cross-platform** — Native app for both macOS and Linux (SteamOS, Fedora, Ubuntu).
- **In-app auto-updater** — Check for and install signed updates directly from within Corkscrew.

---

## Supported Platforms

### Bottle Sources

| Source | macOS | Linux |
|--------|:-----:|:-----:|
| CrossOver | ✓ | ✓ |
| Whisky | ✓ | — |
| Moonshine | ✓ | — |
| Heroic (Wine) | ✓ | ✓ |
| Mythic | ✓ | — |
| Lutris | — | ✓ |
| Proton / Steam | — | ✓ |
| Bottles | — | ✓ |
| Native Wine | ✓ | ✓ |

### Games

Corkscrew supports **80+ games** via an auto-generated game registry extracted from [Vortex's game extensions](https://github.com/Nexus-Mods/vortex-games). Games are auto-detected inside Wine bottles by scanning Steam and GOG installation paths.

<details>
<summary><strong>View full supported games list</strong> (click to expand)</summary>

<!-- GAME_SUPPORT_TABLE_START -->
**80+ games supported** — auto-updated daily from NexusMods API

| # | Game | NexusMods Domain | Mods | Tools | Stores |
|---|------|-----------------|------|-------|--------|
| 1 | Skyrim Special Edition | [skyrimspecialedition](https://www.nexusmods.com/skyrimspecialedition) | 126,900 | Yes | Steam, GOG, Epic |
| 2 | Skyrim | [skyrim](https://www.nexusmods.com/skyrim) | 72,800 | Yes | Steam |
| 3 | Fallout 4 | [fallout4](https://www.nexusmods.com/fallout4) | 71,200 | Yes | Steam, GOG, Epic |
| 4 | Fallout: New Vegas | [newvegas](https://www.nexusmods.com/newvegas) | 39,900 | Yes | Steam, GOG, Epic |
| 5 | The Elder Scrolls IV: Oblivion | [oblivion](https://www.nexusmods.com/oblivion) | 33,000 | Yes | Steam, GOG |
| 6 | Stardew Valley | [stardewvalley](https://www.nexusmods.com/stardewvalley) | 28,600 | Yes | Steam, GOG |
| 7 | Cyberpunk 2077 | [cyberpunk2077](https://www.nexusmods.com/cyberpunk2077) | 20,000 | | Steam, GOG |
| 8 | Fallout 3 | [fallout3](https://www.nexusmods.com/fallout3) | 16,900 | Yes | Steam, GOG, Epic |
| 9 | Baldur's Gate 3 | [baldursgate3](https://www.nexusmods.com/baldursgate3) | 16,800 | Yes | Steam, GOG |
| 10 | Morrowind | [morrowind](https://www.nexusmods.com/morrowind) | 14,600 | Yes | Steam, GOG |
| 11 | Starfield | [starfield](https://www.nexusmods.com/starfield) | 12,200 | | Steam |
| 12 | Blade & Sorcery | [bladeandsorcery](https://www.nexusmods.com/bladeandsorcery) | 8,400 | | Steam |
| 13 | The Witcher 3 | [witcher3](https://www.nexusmods.com/witcher3) | 8,400 | Yes | Steam, GOG, Epic |
| 14 | 7 Days to Die | [7daystodie](https://www.nexusmods.com/7daystodie) | 6,900 | | Steam |
| 15 | Monster Hunter: World | [monsterhunterworld](https://www.nexusmods.com/monsterhunterworld) | 6,300 | Yes | Steam |
| 16 | The Sims 4 | [thesims4](https://www.nexusmods.com/thesims4) | 4,200 | | Steam |
| 17 | Dragon Age: Origins | [dragonage](https://www.nexusmods.com/dragonage) | 3,900 | | Steam |
| 18 | No Man's Sky | [nomanssky](https://www.nexusmods.com/nomanssky) | 2,600 | | Steam |
| 19 | Enderal | [enderal](https://www.nexusmods.com/enderal) | 877 | Yes | Steam |
| 20 | Skyrim VR | [skyrimspecialedition](https://www.nexusmods.com/skyrimspecialedition) | - | Yes | Steam |
| 21 | Fallout 4 VR | [fallout4](https://www.nexusmods.com/fallout4) | - | Yes | Steam |
| 22 | Sekiro | [sekiro](https://www.nexusmods.com/sekiro) | 1,700 | | Steam |
| 23 | Darkest Dungeon | [darkestdungeon](https://www.nexusmods.com/darkestdungeon) | 1,600 | | Steam, GOG, Epic |
| 24 | Dragon Age 2 | [dragonage2](https://www.nexusmods.com/dragonage2) | 1,500 | | Steam |
| 25 | Kingdom Come: Deliverance | [kingdomcomedeliverance](https://www.nexusmods.com/kingdomcomedeliverance) | 1,500 | | Steam, Epic |
| 26 | Dark Souls | [darksouls](https://www.nexusmods.com/darksouls) | 1,400 | | Steam |
| 27 | Kenshi | [kenshi](https://www.nexusmods.com/kenshi) | 1,400 | | Steam |
| 28 | Mount & Blade: Warband | [mbwarband](https://www.nexusmods.com/mbwarband) | 1,400 | | Steam |
| 29 | X4: Foundations | [x4foundations](https://www.nexusmods.com/x4foundations) | 1,400 | | Steam, GOG |
| 30+ | ...and 50+ more games | | | | |
<!-- GAME_SUPPORT_TABLE_END -->

</details>

Games with **dedicated plugins** (Skyrim SE, Fallout 4) have full support including plugin load order management, LOOT sorting, SKSE integration, and plugins.txt handling. All other registry games get automatic detection and basic mod deployment.

Adding a new game with enhanced support is a matter of writing a small plugin — see [`plugins/skyrim_se.rs`](src-tauri/src/plugins/skyrim_se.rs) for the pattern. The game support table is auto-updated daily via the [NexusMods API](https://www.nexusmods.com/) (requires `NEXUS_API_KEY` secret).

### Keyboard Shortcuts

These shortcuts are available on the Mods page:

| Shortcut | Action |
|----------|--------|
| `Ctrl/Cmd + F` | Focus search input |
| `Ctrl/Cmd + A` | Select all visible mods |
| `Ctrl/Cmd + D` | Deploy all mods |
| `↑` / `↓` | Navigate between mod rows |
| `Space` | Toggle enable/disable on focused mod |
| `Enter` | Open detail panel for focused mod |
| `Escape` | Clear selection / close panels |
| `Delete` / `Backspace` | Uninstall selected mods |
| Right-click | Open context menu on mod row |

---

## Current Status

### What Works

Everything listed in [Features](#features) is implemented and functional. The app has been tested primarily on macOS (Apple Silicon) with CrossOver and Whisky bottles. 80+ games are auto-detected; Skyrim SE and Fallout 4 have full-featured support including load order management.

Key workflows tested end-to-end:

- Full mod lifecycle: install from archive → stage → deploy → enable/disable → uninstall
- NXM protocol link handling (click on Nexus website → mod downloads in Corkscrew)
- FOMOD installer wizard for mods with complex install options
- NexusMods Collection installation (premium: automated; free: guided manual download)
- LOOT-powered plugin sorting with masterlist fetching
- Profile save/load/switch with full deployment cycling
- SKSE auto-download and installation (game-version-aware)
- Mod tools detection, auto-install, launching, and uninstalling
- Tool requirement detection for Collections and Wabbajack modlists
- Crash log analysis with actionable diagnosis
- INI file browsing, editing, and preset application
- Wine bottle diagnostics with automated fixes
- Wabbajack gallery browsing and local `.wabbajack` file parsing
- In-app auto-updater with signed releases

### Known Limitations

- **Linux testing is limited** — The app builds for Linux and handles Linux paths throughout, but primary testing has been on macOS. Community feedback on SteamOS/Proton setups is especially welcome.
- **Enhanced game support** — 80+ games are detected and support basic mod deployment. Full-featured support (plugin load order, LOOT sorting, SKSE, plugins.txt) currently exists for Skyrim SE and Fallout 4. Other Bethesda games are next in line.
- **Collections installation** — Works well for most public NexusMods collections (10–150 mods). Install order resolution, FOMOD replay, plugin sync, and profile snapshots are all functional. Binary patch application from collection manifests is not yet implemented. Collection updates require full re-download (no delta updates).
- **Wabbajack installation** — The full Wabbajack install pipeline is implemented with real downloads (NexusMods, HTTP, MediaFire, ModDB), directive processing (BSDiff patching, inline files), and deployment. **Not yet implemented**: BSA/BA2 packing (CreateBSA), DDS texture transformation (TransformedTexture), merged patches (MergedPatch), Google Drive downloads, Wabbajack CDN downloads, and game file source extraction. Complex modlists using these features will partially fail. Install resume after interruption is stubbed but not yet functional.
- **FOMOD conditionals** — The FOMOD installer handles group selection, type badges, and file mapping. Conditional visibility, option dependencies, and mutually-exclusive groups are not yet supported — the installer uses defaults for these cases.
- **NexusMods SSO** — The SSO/OAuth2 module (with PKCE) is fully implemented and ready to use. Currently awaiting NexusMods approval of the "Corkscrew" application slug. In the meantime, API key authentication works.
- **macOS code signing** — The app is not signed with an Apple Developer certificate. Users need to bypass Gatekeeper on first launch (see [Installation](#installation)).

### Roadmap

**In progress:**
- BSA/BA2 archive packing for Wabbajack CreateBSA directives
- DDS texture transformation for Wabbajack TransformedTexture directives
- MergedPatch directive processing for Wabbajack
- Google Drive + Wabbajack CDN download sources
- Collection binary patch application
- FOMOD conditional visibility and option dependencies
- Wabbajack install resume/recovery after interruption

**Near-term:**
- Enhanced game plugins for more Bethesda titles (Oblivion, Fallout 3, Fallout NV, Starfield, Morrowind)
- NexusMods SSO/OAuth authentication (pending NM app approval)
- Collection delta updates (download only changed mods)
- Per-game tool configuration for non-Bethesda games
- File conflict resolution UI (manual override of priority-based resolution)

**Medium-term:**
- Same-volume staging for reliable hardlink deployment
- Enhanced dependency visualization with tree view
- Install simulation / dry-run preview
- Download bandwidth throttling
- Resizable table columns with persistent widths

**Long-term:**
- Apple Developer code signing
- Flatpak distribution
- Community modlist sharing

---

## Architecture

### Why These Technologies

**[Tauri v2](https://v2.tauri.app/)** was chosen over Electron because mod managers are filesystem-heavy tools. Tauri gives us a Rust backend that can walk Wine prefix directories, compute SHA-256 hashes, extract archives, and manage SQLite databases at native speed — all without shipping a bundled Chromium. The result is a ~15 MB app bundle instead of 150+ MB.

**[Svelte 5](https://svelte.dev/)** with SvelteKit (static adapter) provides the frontend. Svelte compiles to vanilla JS with no virtual DOM, which keeps the webview snappy even on lower-end hardware like the Steam Deck. The runes-based reactivity (`$state`, `$derived`, `$effect`) maps naturally to the kind of UI state a mod manager needs.

**Rust** handles everything that touches the filesystem or network: bottle discovery across nine different Wine sources, archive extraction, staging-based mod deployment via hardlinks, LOOT plugin sorting, Nexus Mods API calls, NexusMods Collections GraphQL queries, SKSE auto-download from GitHub, Skyrim SE version detection, crash log analysis, mod tool management, and Wabbajack modlist gallery fetching. The plugin-based game detection system (`GamePlugin` trait) makes adding new game support straightforward.

**SQLite** (via `rusqlite`) with a versioned migration system (v1→v9) tracks installed mods, deployment manifests, file hashes, profiles, plugin rules, conflict rules, mod version history, game file snapshots, mod dependencies, FOMOD recipes, game sessions, collection metadata, auto-categories, download registry, and notification logs.

### Project Structure

```
src/                          Svelte frontend
├── lib/
│   ├── api.ts                Tauri IPC bindings (~171 typed invoke wrappers)
│   ├── types.ts              Shared TypeScript interfaces (~118 types)
│   ├── stores.ts             Svelte stores (game selection, mods, toasts, notifications)
│   ├── theme.ts              Theme detection, persistence, and vibrancy
│   └── components/
│       ├── Icon.svelte              Unified SVG icon component (15+ icons)
│       ├── ThemeToggle.svelte       Light / Auto / Dark segmented control
│       ├── FomodWizard.svelte       Multi-step FOMOD installer wizard
│       ├── ConflictPanel.svelte     Mod file conflict visualization
│       ├── CompatibilityPanel.svelte  SKSE + game version compatibility
│       ├── RequiredToolsPrompt.svelte Pre-install tool requirement check
│       ├── GameIcon.svelte          Per-game icon component
│       ├── ModVersionHistory.svelte Version rollback UI
│       ├── ModlistImportWizard.svelte Modlist import + diff wizard
│       ├── PluginRulesPanel.svelte  Custom plugin load order rules
│       ├── DiskBudgetPanel.svelte   Disk space budget + impact estimates
│       ├── PreflightPanel.svelte    Pre-deployment health checks
│       ├── DependencyPanel.svelte   Mod dependency graph + issue checker
│       ├── SessionHistoryPanel.svelte Game session log + stability summary
│       ├── IniManagerPanel.svelte   INI file editor with presets
│       ├── WineDiagnosticsPanel.svelte Wine bottle health diagnostics
│       ├── WebViewToggle.svelte     In-App / Website toggle with native webview
│       └── mods/
│           ├── ModTableHeader.svelte    Sortable column headers
│           ├── ModTableRow.svelte       Single mod row with hover actions
│           ├── ModFilterBar.svelte      Search, filters, view mode toggle
│           ├── ModDetailSidebar.svelte  Right-side detail panel
│           ├── ModBatchBar.svelte       Floating batch action toolbar
│           ├── ModContextMenu.svelte    Right-click context menu
│           ├── ModCategoryView.svelte   Category tree with collapsible groups
│           └── NotificationLog.svelte   Persistent notification panel
├── routes/
│   ├── +layout.svelte        Shell: sidebar nav, toast system, notification bell
│   ├── +page.svelte          Dashboard (bottle scanning, game discovery)
│   ├── mods/+page.svelte     Mod management with sortable table, view modes, keyboard nav
│   ├── plugins/+page.svelte  Plugin load order editor with LOOT sorting
│   ├── collections/+page.svelte  NexusMods Collections browser + installer
│   ├── modlists/+page.svelte Wabbajack modlist gallery + .wabbajack parser
│   ├── logs/+page.svelte     Crash log analysis and diagnosis
│   ├── profiles/+page.svelte Mod profile management
│   └── settings/+page.svelte Config, game tools, auth, INI, diagnostics
└── app.css                   Design system (tokens, themes, vibrancy, animations)

src-tauri/src/                Rust backend (~48 modules, 566 tests)
├── lib.rs              Tauri command handlers (~171 IPC commands)
├── bottles.rs          Bottle detection (9 sources, macOS + Linux)
├── bottle_config.rs    Wine bottle settings (MSync, MetalFX, env vars)
├── games.rs            Game detection framework + plugin registry
├── game_registry.rs    Auto-generated game plugins from Vortex data (80+ games)
├── installer.rs        Archive extraction (.zip, .7z, .rar, .tar.gz/xz/bz2) + data root detection
├── staging.rs          Staging folder management + SHA-256 hashing
├── deployer.rs         Hardlink/copy deployment engine + atomic rollback + manifest tracking
├── database.rs         SQLite mod tracking with versioned migrations + notification log
├── migrations.rs       Schema versioning + migration runner (v1→v9)
├── loot.rs             libloot wrapper + masterlist management
├── loot_rules.rs       Custom plugin load order rules
├── profiles.rs         Mod profile CRUD + activation flow
├── integrity.rs        Game file snapshots + integrity verification
├── collections.rs      NexusMods Collections GraphQL API client
├── collection_installer.rs  Collection install orchestrator + auto-profile creation
├── wabbajack.rs        Wabbajack gallery fetching + .wabbajack file parsing
├── wabbajack_types.rs  Wabbajack type definitions + BSA/BA2 archive handling
├── wabbajack_directives.rs  Directive execution (copy, patch, create, inline)
├── wabbajack_downloader.rs  Multi-source download engine (Nexus, HTTP, Mega, GDrive)
├── wabbajack_installer.rs   Full modlist install pipeline + cancellation support
├── launcher.rs         Game launching through Wine/CrossOver/Whisky/Proton
├── skse.rs             SKSE detection, auto-download, installation + version-aware builds
├── downgrader.rs       Skyrim version detection + Stock Game creation
├── display_fix.rs      Skyrim display scaling fix (exclusive fullscreen for Wine/Retina)
├── nexus.rs            Nexus Mods API client + update checking
├── nexus_sso.rs        WebSocket SSO authentication (pending NM approval)
├── oauth.rs            OAuth 2.0 + PKCE authentication
├── crashlog.rs         Crash log parser + diagnosis engine
├── conflict_resolver.rs Automated conflict resolution heuristics
├── progress.rs         Install progress event types (Tauri event system)
├── rollback.rs         Mod version rollback + snapshot management
├── modlist_io.rs       Modlist export/import + diff comparison
├── executables.rs      Custom executable management
├── config.rs           JSON configuration (dirs crate for platform paths)
├── fomod.rs            FOMOD XML installer parser (quick-xml)
├── fomod_recipes.rs    FOMOD choice save/export/import/replay
├── disk_budget.rs      Disk space tracking + install impact estimates
├── ini_manager.rs      INI file parser/editor + game-specific presets
├── wine_diagnostic.rs  Wine bottle health checks + automated fixes
├── preflight.rs        Pre-deployment validation checks
├── mod_dependencies.rs Mod dependency graph + issue detection
├── mod_recommendations.rs  Co-install recommendations engine
├── mod_tools.rs        Mod tool detection, auto-install, launch + tool signatures
├── session_tracker.rs  Game session logging + stability analysis
├── download_queue.rs   Download queue with retry + progress events
├── plugins/
│   ├── skyrim_se.rs          Skyrim SE detection (Steam + GOG paths)
│   ├── fallout4.rs           Fallout 4 detection (Steam + GOG paths)
│   └── skyrim_plugins.rs     Plugin load order management
└── data/
    └── vortex_game_registry.json  80+ game definitions (auto-updated daily)
```

### How Mods Are Installed

1. User drops an archive or clicks Install — the frontend calls `install_mod_cmd` via Tauri IPC
2. Progress events are emitted at each step via the Tauri event system, providing real-time UI feedback
3. The `staging` module extracts the archive to a staging folder and uses heuristics to find the mod root (looking for `Data/`, `.esp`/`.esm` files, or a single wrapper folder)
4. SHA-256 hashes are computed for every file in staging and stored in the database
5. The `deployer` module creates hardlinks from the staging folder to the game's `Data/` directory (with copy fallback for cross-volume installs)
6. Every deployed file path is recorded in the `deployment_manifest` table
7. Disabling a mod removes its hardlinks from the game directory while keeping staging intact
8. Re-enabling re-creates the hardlinks from staging
9. Uninstalling removes both deployment and staging, cascading all DB records

For **NexusMods Collections**, the `collection_installer` orchestrator handles the full pipeline: resolving install order via topological sort, downloading each mod (premium only), applying FOMOD selections from the collection manifest, staging and deploying each mod, and applying the collection's plugin load order. A profile snapshot is automatically created after successful collection installs.

---

## Contributing

This is a young project and there's plenty to do. If you're a Mac or Linux gamer who's tired of manually dragging files into Wine prefixes, you're the target audience — and probably the ideal contributor.

Bug reports, feature requests, and pull requests are all welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, PR guidelines, and coding conventions.

### Development Quick Start

```bash
git clone https://github.com/cashcon57/corkscrew.git
cd corkscrew
npm install
cargo tauri dev    # Development mode with hot-reload
```

```bash
# Run tests
cd src-tauri && cargo test           # 566 Rust tests
npx svelte-check --threshold error   # Frontend type checking
```

### Areas Where Help Is Wanted

- **Linux testing** — Especially SteamOS/Steam Deck, Fedora, and Ubuntu with Proton/Lutris bottles
- **Enhanced game plugins** — Adding full load order support for Oblivion, Fallout 3, Fallout NV, Starfield, and Morrowind
- **Non-Bethesda game testing** — Testing mod deployment for the 70+ newly supported games
- **Accessibility** — Improving screen reader support and keyboard navigation

---

## Acknowledgments

Corkscrew stands on the shoulders of many open source projects. We are deeply grateful to:

### Projects We Build Upon

- **[LOOT](https://loot.github.io/) / [libloot](https://github.com/loot/libloot)** — The Load Order Optimization Tool provides the plugin sorting engine that powers Corkscrew's automatic load order management. libloot (GPL-3.0, pure Rust) is integrated directly into Corkscrew for sorting Bethesda game plugins. Created by [WrinklyNinja](https://github.com/Ortham).
- **[Wabbajack](https://www.wabbajack.org/)** ([GitHub](https://github.com/wabbajack-tools/wabbajack)) — Pioneered automated modlist compilation and installation, and developed the "Stock Game" approach for version-locked game copies. Corkscrew's modlist gallery browses Wabbajack's repository index, and the downgrade/Stock Game feature follows the approach Wabbajack popularized. GPL-3.0.
- **[Vortex](https://github.com/Nexus-Mods/Vortex)** by Nexus Mods — Vortex's deployment model, conflict resolution patterns, and the Nexus Collections format informed Corkscrew's staging-based deployment engine and priority system. GPL-3.0.
- **[Mod Organizer 2](https://github.com/ModOrganizer2/modorganizer)** — MO2's virtual filesystem concept, profile system, and category view inspired Corkscrew's staging/deploy architecture, mod profiles, and category grouping. GPL-3.0.

### Libraries & Tools

- **[Tauri](https://tauri.app/)** — The framework that makes native cross-platform apps possible with a Rust backend and web frontend.
- **[Svelte](https://svelte.dev/)** — The frontend framework that keeps the UI fast and reactive.
- **[rusqlite](https://github.com/rusqlite/rusqlite)** — SQLite bindings for Rust.
- **[quick-xml](https://github.com/tafia/quick-xml)** — Fast XML parsing for FOMOD installers.
- **[FOMOD](https://fomod-docs.readthedocs.io/)** standard — The XML-based mod installer format used by many mod authors.
- **[reqwest](https://github.com/seanmonstar/reqwest)** — HTTP client for Nexus Mods API and NexusMods Collections GraphQL.

### Modding Tool Authors

Corkscrew integrates with many community-built modding tools. If you use these tools, please consider supporting their authors:

- **[SSEEdit / xEdit](https://github.com/TES5Edit/TES5Edit)** by ElminsterAU — The essential plugin editor for Bethesda games. [Support on Ko-fi](https://ko-fi.com/elminsterau)
- **[DynDOLOD](https://dyndolod.info/)** by Sheson — Dynamic distant object LOD generation. [Support on Ko-fi](https://ko-fi.com/sheson)
- **[Pandora Behaviour Engine](https://github.com/Monitor221hz/Pandora-Behaviour-Engine)** by Monitor221hz — Modern animation engine, Wine-compatible alternative to FNIS/Nemesis. [Support on Patreon](https://www.patreon.com/monitorhz)
- **[Nemesis Unlimited Behavior Engine](https://github.com/ShikyoKira/Project-New-Reign---Nemesis-Main)** by ShikyoKira — Animation engine for Skyrim. [Support on Patreon](https://www.patreon.com/shikyokira)
- **[Cathedral Assets Optimizer](https://github.com/Guekka/Cathedral-Assets-Optimizer)** by Guekka — Texture and mesh optimization. [Support on GitHub Sponsors](https://github.com/sponsors/Guekka)
- **[BodySlide and Outfit Studio](https://github.com/ousnius/BodySlide-and-Outfit-Studio)** by ousnius — Body and outfit customization tool.
- **[BethINI](https://www.nexusmods.com/skyrimspecialedition/mods/631)** by DoubleYou — INI configuration tool for Bethesda games.
- **[Wrye Bash](https://github.com/wrye-bash/wrye-bash)** — Bashed Patch creation and mod management.
- **[SKSE Team](https://skse.silverlock.org/)** — The Skyrim Script Extender, essential for most Skyrim mods.

### Communities & Services

- **[Wine Project](https://www.winehq.org/)** — The foundation that makes running Windows games on macOS and Linux possible.
- **[CrossOver](https://www.codeweavers.com/crossover)** by CodeWeavers — A polished Wine implementation and major Wine contributor.
- **[Nexus Mods](https://www.nexusmods.com/)** — For the modding community, mod hosting, and the API and GraphQL endpoints that mod managers depend on.
- **[Jackify](https://github.com/Omni-guides/Jackify)** — For demonstrating that Wabbajack modlist installation on Linux is possible, and for pioneering the approach with SteamOS/Steam Deck.

---

## Third-Party Licenses

Corkscrew is licensed under GPL-3.0-or-later. The following third-party components are incorporated and require copyright notice.

### Linked Libraries

#### GPL-3.0 — LOOT Stack

Corkscrew links against the LOOT plugin sorting libraries as a Rust dependency. These are licensed under the GNU General Public License v3.0 and are Copyright (C) Oliver Shercliff (WrinklyNinja).

- **[libloot](https://github.com/loot/libloot)** (GPL-3.0-or-later) — Load order sorting engine
- **[esplugin](https://github.com/Ortham/esplugin)** (GPL-3.0) — Bethesda plugin file parser
- **[libloadorder](https://github.com/Ortham/libloadorder)** (GPL-3.0) — Load order management library
- **[loot-condition-interpreter](https://github.com/loot/loot-condition-interpreter)** (MIT) — Metadata condition evaluator

#### Apache-2.0 / MIT — Tauri

Copyright (c) Tauri Programme within The Commons Conservancy. Licensed under Apache License 2.0 or MIT.

- **[Tauri](https://github.com/tauri-apps/tauri)** and official plugins (opener, dialog, deep-link, fs, updater, process)

#### Apache-2.0 / MPL-2.0 — DOMPurify

Copyright 2025 Dr.-Ing. Mario Heiderich, Cure53. Licensed under Apache License 2.0 or Mozilla Public License 2.0.

- **[DOMPurify](https://github.com/cure53/DOMPurify)** — HTML sanitization for collection and modlist descriptions

#### MPL-2.0 — Servo Components

Copyright (c) Mozilla Foundation and contributors. Licensed under Mozilla Public License 2.0.

- **[cssparser](https://github.com/servo/rust-cssparser)** / **[selectors](https://github.com/servo/servo)** — CSS parsing (transitive dependency via Tauri/wry)

### Auto-Downloaded Modding Tools

Corkscrew can auto-download the following tools from their official GitHub releases. These tools are downloaded and installed as standalone executables — Corkscrew does not link against or modify them.

| Tool | License | Source | How Corkscrew uses it |
|------|---------|--------|----------------------|
| [xEdit / SSEEdit](https://github.com/TES5Edit/TES5Edit) | MPL 1.1 | GitHub Releases | Auto-download, detect, launch |
| [Pandora Behaviour Engine](https://github.com/Monitor221hz/Pandora-Behaviour-Engine-Plus) | GPL-3.0 | GitHub Releases | Auto-download, detect, launch |
| [BodySlide / Outfit Studio](https://github.com/ousnius/BodySlide-and-Outfit-Studio) | GPL-3.0+ | GitHub Releases | Auto-download, detect, launch |
| [Cathedral Assets Optimizer](https://github.com/Guekka/Cathedral-Assets-Optimizer) | MPL 2.0 | GitHub Releases | Auto-download, detect, launch |
| [Wrye Bash](https://github.com/wrye-bash/wrye-bash) | GPL-3.0+ | GitHub Releases | Auto-download, detect, launch |
| [Nemesis](https://github.com/ShikyoKira/Project-New-Reign---Nemesis-Main) | GPL-3.0 | GitHub Releases | Auto-download, detect, launch |
| [BethINI Pie](https://github.com/DoubleYouC/Bethini-Pie-Performance-INI-Editor) | CC BY-NC-SA 4.0 | GitHub Releases | Auto-download, detect, launch |

### Tools Downloaded From Official Sources

| Tool | License | Source | How Corkscrew uses it |
|------|---------|--------|----------------------|
| [SKSE](https://skse.silverlock.org/) | Proprietary | [github.com/ianpatt/skse64](https://github.com/ianpatt/skse64/releases) (authorized source) | Auto-download from official GitHub, detect, launch |

SKSE is downloaded directly from the official `ianpatt/skse64` GitHub repository, which is one of the three distribution channels authorized by the SKSE team (alongside skse.silverlock.org and Steam). Corkscrew does not redistribute, mirror, or host SKSE files.

### Tools Linked Only (Not Redistributed)

The following tools are detected if already installed but are not auto-downloaded due to their distribution terms:

| Tool | License | Official download |
|------|---------|------------------|
| [DynDOLOD](https://dyndolod.info/) | Proprietary freeware | [dyndolod.info](https://dyndolod.info/) or [Nexus Mods](https://www.nexusmods.com/skyrimspecialedition/mods/68518) |
| [FNIS](https://www.nexusmods.com/skyrimspecialedition/mods/3038) | Closed source | Nexus Mods only |

Full GPL-3.0 license text: https://www.gnu.org/licenses/gpl-3.0.html

---

## Support

If Corkscrew is useful to you, consider buying me a coffee:

[![Ko-fi](https://img.shields.io/badge/Ko--fi-Support%20Corkscrew-FF5E5B?logo=ko-fi&logoColor=white&style=for-the-badge)](https://ko-fi.com/cash508287)

## License

GPL-3.0-or-later. See [LICENSE](LICENSE) for details.
