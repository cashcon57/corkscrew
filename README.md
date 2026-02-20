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

<p align="center">
  <a href="#features">Features</a>&nbsp;&nbsp;&bull;&nbsp;&nbsp;
  <a href="#screenshots">Screenshots</a>&nbsp;&nbsp;&bull;&nbsp;&nbsp;
  <a href="#installation">Installation</a>&nbsp;&nbsp;&bull;&nbsp;&nbsp;
  <a href="#supported-platforms">Platforms</a>&nbsp;&nbsp;&bull;&nbsp;&nbsp;
  <a href="#keyboard-shortcuts">Keyboard Shortcuts</a>&nbsp;&nbsp;&bull;&nbsp;&nbsp;
  <a href="#architecture">Architecture</a>&nbsp;&nbsp;&bull;&nbsp;&nbsp;
  <a href="#acknowledgments">Acknowledgments</a>&nbsp;&nbsp;&bull;&nbsp;&nbsp;
  <a href="#contributing">Contributing</a>&nbsp;&nbsp;&bull;&nbsp;&nbsp;
  <a href="#support">Support</a>
</p>

<br>

Corkscrew installs, manages, and organizes mods for Windows games running through [CrossOver](https://www.codeweavers.com/crossover), [Whisky](https://getwhisky.app/), [Moonshine](https://github.com/nicholasknight/moonshine), [Lutris](https://lutris.net/), [Proton](https://github.com/ValveSoftware/Proton), and other Wine-based compatibility layers — no Windows VM required.

It works by reading and writing directly to your Wine bottle's filesystem, the same way the game itself sees it. Your bottles, your mods, no middleman.

> **Status:** v0.8.0 — Skyrim Special Edition is the first fully supported game. More games coming soon.

---

## Features

### Mod Management
- **Staging-based deployment** — Mods are extracted to a staging folder first, then deployed to the game directory via hardlinks (with copy fallback). Toggle mods on/off without re-downloading or re-extracting.
- **Mod installation** — Handles `.zip`, `.7z`, `.rar`, `.tar.gz`, `.tar.xz`, and `.tar.bz2` archives with smart data root detection, or drag-and-drop files directly onto the app.
- **Priority-based conflict resolution** — Drag-reorder mods to set deployment priority. Higher-priority mods win when files overlap, with a visual conflict panel showing which mods override which files.
- **FOMOD wizard** — Step-by-step interactive installer for mods using the FOMOD XML format, with radio/checkbox groups, option descriptions, and type badges.
- **FOMOD choice replay** — Save and export FOMOD installer choices as shareable JSON recipes, then replay them on reinstall or share with others.
- **Mod integrity verification** — SHA-256 hashes are stored per file; verify staging integrity on demand.
- **Mod version rollback** — Track mod versions and roll back to previous versions with snapshot support.
- **Modlist export/import** — Export your mod setup as a portable JSON modlist and import it on another machine or share it with others, with diff comparison between modlists.
- **Mod dependency tracking** — Define requires/conflicts/patches relationships between mods. The dependency checker surfaces missing requirements and active conflicts before you launch.
- **Contextual mod recommendations** — Get suggestions for commonly co-installed mods based on what others use in similar setups.
- **Pre-flight installation checks** — Run a comprehensive pre-deployment check covering disk space, staging integrity, bottle health, and file conflicts before deploying.
- **Disk space budget** — Real-time disk usage dashboard showing staging, deployment, and available space with per-install impact estimates.
- **Auto-categorization** — Mods are automatically classified by content type (Plugins, Textures, Models, SKSE Plugins, UI, Audio, Scripts, ENB, ReShade) using file-path heuristics.

### Mods Page UX
- **Sortable column headers** — Click any column header (Name, Version, Files, Date) to sort. Click again to reverse. Semantic version comparison handles `1.2.3` vs `1.10.0` correctly.
- **Three view modes** — Switch between Flat (standard table), Collection (grouped by collection with collapsible headers), and Category (MO2-style tree grouped by auto-category).
- **Collection dropdown** — Filter the mod list by installed collection or view standalone mods, via a dropdown below the game banner.
- **Category chips** — Color-coded pills next to mod names showing their auto-detected content category.
- **Search highlighting** — Search terms are highlighted in mod names as you type.
- **Right-click context menu** — Right-click any mod row for quick access to Toggle, Edit Tags, Edit Notes, Reinstall, Check for Update, Open on Nexus, and Uninstall.
- **Batch selection** — Select multiple mods with checkboxes for bulk Enable All, Disable All, or Uninstall via a floating action bar.
- **Keyboard navigation** — Full keyboard support: arrow keys to navigate rows, Space to toggle, Enter for details, Ctrl/Cmd+F to search, Ctrl/Cmd+A to select all, Ctrl/Cmd+D to deploy.
- **Deploy progress** — The Deploy button shows a real-time progress bar with mod-by-mod status during deployment.
- **Persistent notification log** — All success/error/warning notifications are logged to a persistent database. Click the bell icon in the sidebar to review past notifications with timestamps.
- **Hover actions** — Action buttons appear on row hover for a cleaner table appearance.
- **Empty state guidance** — Contextual CTAs when no mods are installed: Install from Archive, Browse NexusMods, or Install a Collection.
- **Auto-profile on collection install** — A named profile snapshot is automatically created after each collection installation.

### Nexus Mods Integration
- **API key authentication** — Connect your Nexus Mods account to access premium features.
- **NXM link handling** — Download mods directly from NXM links on the Nexus Mods website.
- **Update checking** — Check installed mods against Nexus for available updates.
- **Collections browser** — Browse NexusMods Collections with search, sorting, filtering, and detailed mod/revision views. Download sizes and mod counts shown per collection.
- **Collection installation** — Premium users can install entire NexusMods Collections with one click. The orchestrator resolves install order, downloads mods, handles FOMOD selections from the collection manifest, deploys files, and applies the collection's plugin load order. Free users see a list of mods to download manually from the Nexus website.
- **Collection diff** — Compare your locally installed collection against the author's latest revision to see added, removed, and updated mods at a glance.
- **My Collections** — Card grid with collection thumbnails, author info, and revision tracking. Check for updates with one click.
- **Global install status bar** — Collection install progress is visible from any page via a persistent status bar overlay.
- **Premium enforcement** — Free users are directed to the Nexus Mods website for downloads; only premium users get API-initiated downloads, in full compliance with NexusMods policies.
- **Install progress events** — Real-time step-by-step progress feedback during mod and collection installation (preparing, extracting, deploying, syncing plugins) via Tauri event system.

### Plugin Load Order
- **LOOT-powered sorting** — Automatic plugin sorting using [libloot](https://github.com/loot/libloot) (the same engine behind LOOT), with masterlist fetching from GitHub.
- **Manual drag-and-drop reorder** — Fine-tune your load order after LOOT sorts.
- **Plugin enable/disable** — Toggle individual plugins without touching the mod.
- **Plugin warnings** — LOOT messages (info, warnings, errors) displayed inline per plugin.
- **Custom plugin rules** — Define LoadAfter, LoadBefore, and Group rules for per-plugin ordering beyond what LOOT provides.

### Profiles
- **Save and switch** — Snapshot your current mod states, priorities, and plugin load order into named profiles.
- **Instant activation** — Switch profiles in one click: purges current deployment, applies the target profile's states, redeploys, and restores plugin order.

### Wabbajack Modlists
- **Gallery browser** — Browse the full Wabbajack modlist gallery with search, game filtering, and NSFW toggle.
- **Modlist metadata** — View archive counts, download/install sizes, tags, and version info.
- **Local .wabbajack parsing** — Open and analyze downloaded .wabbajack files to see directive breakdowns and archive sources.
- *Installation coming in a future release.*

### Crash Log Analysis
- **Automatic detection** — Scans for Skyrim crash logs (from .NET Script Framework or Crash Logger) in your bottle.
- **Crash diagnosis** — Parses crash logs to identify exception types, faulting modules, involved plugins, and SKSE plugins.
- **Suggested actions** — Provides actionable recommendations (update mod, disable mod, sort load order, check VRAM, etc.) with confidence ratings.
- **System info extraction** — Displays OS, CPU, GPU, RAM, and VRAM usage at crash time.
- **Game session tracking** — Log play sessions with automatic duration tracking, crash detection, and stability summaries. Track which mods were changed between sessions to correlate changes with crashes.

### Game Launching & Tools
- **Game launching** — Play your modded game straight from Corkscrew, through whatever Wine layer the bottle uses.
- **SKSE integration** — Auto-detect, download, and install the Skyrim Script Extender; launch through SKSE with one click. Compatibility checks against your game version.
- **Skyrim SE downgrade** — Detect your Skyrim version via SHA-256 hash and create a "Stock Game" copy to lock v1.5.97 and prevent Steam auto-updates (same approach pioneered by Wabbajack).
- **Display scaling fix** — Automatically fix Skyrim SE display scaling issues in CrossOver on macOS by detecting your screen resolution and forcing exclusive fullscreen mode to bypass Wine/Retina DPI scaling issues.
- **INI settings manager** — Browse, search, and edit game INI files (Skyrim.ini, SkyrimPrefs.ini, etc.) with built-in presets for common configurations like Steam Deck optimization, ultra graphics, and performance profiles.
- **Wine bottle diagnostics** — Comprehensive health check for Wine bottles: validates drive_c, AppData, DXVK (Linux) / D3DMetal (macOS), DLL overrides, Visual C++ redistributables, .NET, Windows version, Retina/HiDPI display, and user directories, with one-click auto-fixes for common issues.
- **Mod tools detection** — Automatically scans for known modding tools (SSEEdit, BethINI, DynDOLOD, BodySlide, Nemesis, Wrye Bash, etc.) in your game directory.
- **Custom executables** — Define custom .exe launch targets per game.
- **Game file integrity** — Take snapshots of your game directory to detect modified, unknown, or missing files later.
- **Bottle configuration** — View and modify Wine bottle settings (Windows version, MSync, MetalFX, DXMT, environment variables) directly from Corkscrew.

### Platform & UI
- **Automatic bottle detection** — Finds CrossOver, Whisky, Moonshine, Heroic, Mythic, Lutris, Proton, Bottles, and native Wine prefixes.
- **Game scanning** — Discovers supported titles across all bottles (Skyrim SE via Steam or GOG to start).
- **macOS vibrancy** — Native translucent materials that follow the active window state.
- **Light and dark themes** — System-following by default with manual toggle.
- **Cross-platform** — Native app for both macOS and Linux (SteamOS, Fedora, Ubuntu).
- **Unified icon system** — Consistent SVG icon component used across the entire UI.
- **In-app auto-updater** — Check for and install updates directly from within Corkscrew.

---

## Screenshots

> Screenshots coming soon — Corkscrew features a dark-themed native UI with translucent vibrancy on macOS.

**Mods Page** — The primary workspace with sortable table, search highlighting, category chips, view mode toggle (Flat / Collection / Category), and collection dropdown filter.

**Collections Browser** — Browse NexusMods Collections with search, sort by endorsements or downloads, and one-click install for premium users.

**Plugin Load Order** — Drag-and-drop reordering with LOOT-powered auto-sort, inline plugin warnings, and custom rules.

**FOMOD Wizard** — Step-by-step interactive installer with radio/checkbox groups, descriptions, and type badges.

**Settings** — Nexus API key auth, INI editor with presets, Wine bottle diagnostics with auto-fixes, display scaling configuration, and mod tools detection.

---

## Installation

### Requirements

- macOS 10.15+ or Linux with GTK 3 / WebKitGTK
- A Wine-based runner (CrossOver, Whisky, Lutris, Proton, etc.)

### From Release

Download the latest release for your platform from the [Releases page](https://github.com/cashcon57/corkscrew/releases):

| Platform | Format |
|----------|--------|
| macOS | `.dmg` (drag to Applications) |
| Linux | AppImage, `.deb`, `.rpm` |

> **IMPORTANT (macOS):** The app is not yet code-signed with an Apple Developer certificate. macOS Gatekeeper will show "Corkscrew is damaged" when you first open it. After dragging to Applications, run:
> ```bash
> xattr -cr /Applications/Corkscrew.app
> ```
> Or: right-click the app → **Open** → click "Open" in the dialog.

### From Source

```bash
git clone https://github.com/cashcon57/corkscrew.git
cd corkscrew
npm install
cargo tauri build
```

Requires [Node.js](https://nodejs.org/) and a [Rust toolchain](https://rustup.rs/).

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

| Game | ID | Status |
|------|----|--------|
| Skyrim Special Edition | `skyrimse` | Full support |
| *More to come* | | Planned |

Adding a new game is a matter of writing a small plugin — see [`plugins/skyrim_se.rs`](src-tauri/src/plugins/skyrim_se.rs) for the pattern.

---

## Keyboard Shortcuts

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

## Architecture

### Why these technologies

**[Tauri v2](https://v2.tauri.app/)** was chosen over Electron because mod managers are filesystem-heavy tools. Tauri gives us a Rust backend that can walk Wine prefix directories, compute SHA-256 hashes, extract archives, and manage SQLite databases at native speed — all without shipping a bundled Chromium. The result is a ~15 MB app bundle instead of 150+ MB.

**[Svelte 5](https://svelte.dev/)** with SvelteKit (static adapter) provides the frontend. Svelte compiles to vanilla JS with no virtual DOM, which keeps the webview snappy even on lower-end hardware like the Steam Deck. The runes-based reactivity (`$state`, `$derived`, `$effect`) maps naturally to the kind of UI state a mod manager needs.

**Rust** handles everything that touches the filesystem or network: bottle discovery across nine different Wine sources, archive extraction, staging-based mod deployment via hardlinks, LOOT plugin sorting, Nexus Mods API calls, NexusMods Collections GraphQL queries, SKSE downloads, Skyrim SE version detection, crash log analysis, and Wabbajack modlist gallery fetching. The plugin-based game detection system (`GamePlugin` trait) makes adding new game support straightforward.

**SQLite** (via `rusqlite`) with a versioned migration system (v1→v7) tracks installed mods, deployment manifests, file hashes, profiles, plugin rules, conflict rules, mod version history, game file snapshots, mod dependencies, FOMOD recipes, game sessions, collection metadata, auto-categories, and notification logs.

### Project structure

```
src/                          Svelte frontend
├── lib/
│   ├── api.ts                Tauri IPC bindings (~110 typed invoke wrappers)
│   ├── types.ts              Shared TypeScript interfaces (~110 types)
│   ├── stores.ts             Svelte stores (game selection, mods, toasts, notifications)
│   ├── theme.ts              Theme detection, persistence, and vibrancy
│   └── components/
│       ├── Icon.svelte              Unified SVG icon component (15+ icons)
│       ├── ThemeToggle.svelte       Light / Auto / Dark segmented control
│       ├── FomodWizard.svelte       Multi-step FOMOD installer wizard
│       ├── ConflictPanel.svelte     Mod file conflict visualization
│       ├── CompatibilityPanel.svelte  SKSE + game version compatibility
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
│   ├── collections/+page.svelte  NexusMods Collections browser
│   ├── modlists/+page.svelte Wabbajack modlist gallery browser
│   ├── logs/+page.svelte     Crash log analysis and diagnosis
│   ├── profiles/+page.svelte Mod profile management
│   └── settings/+page.svelte Config, appearance, game tools, auth, INI, diagnostics
└── app.css                   Design system (tokens, themes, vibrancy, animations)

src-tauri/src/                Rust backend (~45 modules, 483 tests)
├── lib.rs              Tauri command handlers (~100 IPC commands)
├── bottles.rs          Bottle detection (9 sources, macOS + Linux)
├── bottle_config.rs    Wine bottle settings (MSync, MetalFX, env vars)
├── games.rs            Game detection framework + plugin registry
├── installer.rs        Archive extraction (.zip, .7z, .rar, .tar.gz/xz/bz2) + data root detection
├── staging.rs          Staging folder management + SHA-256 hashing
├── deployer.rs         Hardlink/copy deployment engine + manifest tracking + progress events
├── database.rs         SQLite mod tracking with versioned migrations + notification log
├── migrations.rs       Schema versioning + migration runner (v1→v7)
├── loot.rs             libloot wrapper + masterlist management
├── loot_rules.rs       Custom plugin load order rules
├── profiles.rs         Mod profile CRUD + activation flow
├── integrity.rs        Game file snapshots + integrity verification
├── collections.rs      NexusMods Collections GraphQL API client
├── collection_installer.rs  Collection install orchestrator + auto-profile creation
├── wabbajack.rs        Wabbajack gallery fetching + .wabbajack file parsing
├── launcher.rs         Game launching through Wine/CrossOver/Whisky/Proton
├── skse.rs             SKSE detection, download, installation + compat checks
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
├── mod_tools.rs        Mod tool detection (SSEEdit, BethINI, DynDOLOD, etc.)
├── session_tracker.rs  Game session logging + stability analysis
├── download_queue.rs   Download queue with retry + progress events
└── plugins/
    ├── skyrim_se.rs          Skyrim SE detection (Steam + GOG paths)
    └── skyrim_plugins.rs     Plugin load order management
```

### How mods are installed

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

## What Works

Everything listed above is implemented and functional. The app has been tested primarily on macOS (Apple Silicon) with CrossOver and Whisky bottles running Skyrim SE via Steam. Key workflows that are end-to-end tested:

- Bottle discovery and game detection across all supported Wine sources
- Full mod lifecycle: install from archive → stage → deploy → enable/disable → uninstall
- Drag-and-drop mod installation with real-time progress events
- NXM protocol link handling (click on Nexus website → mod downloads in Corkscrew)
- FOMOD installer wizard for mods with complex install options
- NexusMods Collection installation (premium: automated download + deploy; free: guided manual download)
- LOOT-powered plugin sorting with masterlist fetching
- Profile save/load/switch with full deployment cycling
- Crash log analysis with actionable diagnosis
- SKSE detection, download, install, and launch-through-SKSE
- Collection browsing, filtering by game, and metadata viewing
- Pre-flight checks and disk space budgeting before deployment
- INI file browsing, editing, and preset application
- Wine bottle diagnostics with automated fixes
- Mod dependency tracking and conflict detection
- Game session logging with stability summaries
- FOMOD choice recipes (save, export, import, replay)
- Sortable columns with semantic version comparison
- Keyboard navigation and batch mod operations
- Context menus and search highlighting
- View modes: flat table, collection grouping, category tree
- Deploy progress bar with real-time feedback
- Persistent notification logging

### Known Limitations

- **Linux testing is limited** — The app builds and the code handles Linux paths, but testing has been primarily on macOS. SteamOS/Proton testing is planned.
- **Single game support** — Only Skyrim SE is supported currently. The plugin architecture is ready for more games, but each needs a detection plugin.
- **Wabbajack installation** — You can browse the Wabbajack gallery and parse `.wabbajack` files, but automated installation of Wabbajack modlists is not yet implemented.
- **NexusMods SSO** — The SSO module is built but requires NexusMods to approve the "Corkscrew" application slug. Currently uses API key authentication.
- **OAuth flow** — OAuth 2.0 + PKCE module is implemented but depends on the same NexusMods app approval as SSO.

### Roadmap

**Near-term:**
- Resizable table columns with persistent widths
- Inline conflict resolution popovers
- Enhanced dependency visualization with tree view
- Mod tools launching — run detected tools (SSEEdit, DynDOLOD, etc.) through Wine from within Corkscrew
- SKSE/Address Library pre-flight compatibility checks

**Medium-term:**
- Wabbajack modlist installation (FromArchive directives, download orchestration)
- More game plugins (Fallout 4, Oblivion, Starfield, etc.)
- NexusMods SSO/OAuth authentication (pending NM app approval)
- Same-volume staging for reliable hardlink deployment

**Long-term:**
- Linux/SteamOS testing and distribution (AppImage, .deb, .rpm)
- Collection update installation from diff view

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

### Communities & Services

- **[SKSE Team](https://skse.silverlock.org/)** — For the Skyrim Script Extender, essential for most Skyrim mods.
- **[Wine Project](https://www.winehq.org/)** — The foundation that makes running Windows games on macOS and Linux possible.
- **[CrossOver](https://www.codeweavers.com/crossover)** by CodeWeavers — A polished Wine implementation and major Wine contributor.
- **[Nexus Mods](https://www.nexusmods.com/)** — For the modding community, mod hosting, and the API and GraphQL endpoints that mod managers depend on.
- **[Jackify](https://github.com/Omni-guides/Jackify)** — For demonstrating that Wabbajack modlist installation on Linux is possible, and for pioneering the approach with SteamOS/Steam Deck.

---

## Third-Party Licenses

Corkscrew is licensed under GPL-3.0-or-later. The following third-party components are incorporated and require copyright notice:

### GPL-3.0 — LOOT Stack

Corkscrew links against the LOOT plugin sorting libraries. These are licensed under the GNU General Public License v3.0 and are Copyright (C) Oliver Shercliff (WrinklyNinja).

- **[libloot](https://github.com/loot/libloot)** (GPL-3.0-or-later) — Load order sorting engine
- **[esplugin](https://github.com/Ortham/esplugin)** (GPL-3.0) — Bethesda plugin file parser
- **[libloadorder](https://github.com/Ortham/libloadorder)** (GPL-3.0) — Load order management library
- **[loot-condition-interpreter](https://github.com/loot/loot-condition-interpreter)** (MIT) — Metadata condition evaluator

Full license text: https://www.gnu.org/licenses/gpl-3.0.html

### Apache-2.0 / MPL-2.0 — DOMPurify

Copyright 2025 Dr.-Ing. Mario Heiderich, Cure53. Licensed under Apache License 2.0 or Mozilla Public License 2.0.

- **[DOMPurify](https://github.com/cure53/DOMPurify)** — HTML sanitization for collection and modlist descriptions

### Apache-2.0 / MIT — Tauri

Copyright (c) Tauri Programme within The Commons Conservancy. Licensed under Apache License 2.0 or MIT.

- **[Tauri](https://github.com/tauri-apps/tauri)** and official plugins (opener, dialog, deep-link, fs)

### MPL-2.0 — Servo Components

Copyright (c) Mozilla Foundation and contributors. Licensed under Mozilla Public License 2.0.

- **[cssparser](https://github.com/servo/rust-cssparser)** / **[selectors](https://github.com/servo/servo)** — CSS parsing (transitive dependency via Tauri/wry)

---

## Contributing

This is a young project and there's plenty to do. If you're a Mac or Linux gamer who's tired of manually dragging files into Wine prefixes, you're the target audience — and probably the ideal contributor.

Bug reports, feature requests, and pull requests are all welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, PR guidelines, and coding conventions.

## Support

If Corkscrew is useful to you, consider buying me a coffee:

[![Ko-fi](https://img.shields.io/badge/Ko--fi-Support%20Corkscrew-FF5E5B?logo=ko-fi&logoColor=white&style=for-the-badge)](https://ko-fi.com/cash508287)

## License

GPL-3.0-or-later. See [LICENSE](LICENSE) for details.
