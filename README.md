<p align="center">
  <img src="graphics/icon-readme.png" width="200" alt="Corkscrew">
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
</p>

<p align="center">
  <a href="#features">Features</a>&nbsp;&nbsp;&bull;&nbsp;&nbsp;
  <a href="#installation">Installation</a>&nbsp;&nbsp;&bull;&nbsp;&nbsp;
  <a href="#supported-platforms">Platforms</a>&nbsp;&nbsp;&bull;&nbsp;&nbsp;
  <a href="#architecture">Architecture</a>&nbsp;&nbsp;&bull;&nbsp;&nbsp;
  <a href="#contributing">Contributing</a>
</p>

<br>

Corkscrew installs, manages, and organizes mods for Windows games running through [CrossOver](https://www.codeweavers.com/crossover), [Whisky](https://getwhisky.app/), [Lutris](https://lutris.net/), [Proton](https://github.com/ValveSoftware/Proton), and other Wine-based compatibility layers — no Windows VM required.

It works by reading and writing directly to your Wine bottle's filesystem, the same way the game itself sees it. Your bottles, your mods, no middleman.

> **Status:** Active development (v0.1.0). Skyrim Special Edition is the first supported game with full mod management, SKSE integration, and game launching. More games coming soon.

---

## Features

- **Automatic bottle detection** — Finds CrossOver, Whisky, Moonshine, Heroic, Mythic, Lutris, Proton, Bottles, and native Wine prefixes
- **Game scanning** — Discovers supported titles across all bottles (Skyrim SE via Steam or GOG to start)
- **Mod installation** — Handles `.zip`, `.7z`, and `.rar` archives with smart root detection, or drag-and-drop files directly onto the app
- **Game launching** — Play your modded game straight from Corkscrew, through whatever Wine layer the bottle uses
- **SKSE integration** — Auto-detect, download, and install the Skyrim Script Extender; launch through SKSE with one click
- **Skyrim SE downgrade** — Detect your Skyrim version via SHA-256 hash and create a "Stock Game" copy to lock v1.5.97 and prevent Steam auto-updates (same approach as Wabbajack)
- **Nexus Mods integration** — Download and install directly from NXM links, with a quick link to each game's Nexus page
- **Mod tracking** — SQLite database records every installed file for clean enables, disables, and uninstalls
- **Plugin load order** — Reads and syncs `plugins.txt` and `loadorder.txt` for Bethesda games
- **FOMOD support** — Parses the standard XML-based mod installer format
- **macOS vibrancy** — Native translucent materials that follow the active window state, matching the latest macOS design language
- **Light and dark themes** — System-following by default, with a manual toggle; both themes tuned to the Corkscrew brand palette
- **Cross-platform** — Native app for both macOS and Linux (SteamOS, Fedora, Ubuntu)

### Planned

- FOMOD wizard UI with option selection
- Mod profiles
- Nexus Mods modlist support
- Wabbajack modlist support
- Mod conflict detection and resolution

---

## Installation

### Requirements

- macOS 10.15+ or Linux with GTK 3 / WebKitGTK
- A Wine-based runner (CrossOver, Whisky, Lutris, Proton, etc.)

### From Release

Download the latest release for your platform:

| Platform | Format |
|----------|--------|
| macOS | `.dmg` (drag to Applications) |
| Linux | AppImage, `.deb`, `.rpm` |

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
| CrossOver | &check; | &check; |
| Whisky | &check; | — |
| Moonshine | &check; | — |
| Heroic (Wine) | &check; | &check; |
| Mythic | &check; | — |
| Lutris | — | &check; |
| Proton / Steam | — | &check; |
| Bottles | — | &check; |
| Native Wine | &check; | &check; |

### Games

| Game | ID | Status |
|------|----|--------|
| Skyrim Special Edition | `skyrimse` | Working |
| *More to come* | | Planned |

Adding a new game is a matter of writing a small plugin — see [`plugins/skyrim_se.rs`](src-tauri/src/plugins/skyrim_se.rs) for the pattern.

---

## Architecture

### Why these technologies

**[Tauri v2](https://v2.tauri.app/)** was chosen over Electron because mod managers are filesystem-heavy tools. Tauri gives us a Rust backend that can walk Wine prefix directories, compute SHA-256 hashes, extract archives, and manage SQLite databases at native speed — all without shipping a bundled Chromium. The result is a ~15 MB app bundle instead of 150+ MB.

**[Svelte 5](https://svelte.dev/)** with SvelteKit (static adapter) provides the frontend. Svelte compiles to vanilla JS with no virtual DOM, which keeps the webview snappy even on lower-end hardware like the Steam Deck. The runes-based reactivity (`$state`, `$derived`, `$effect`) maps naturally to the kind of UI state a mod manager needs: game selection cascading into mod lists, SKSE status checks, and drag-and-drop interactions.

**Rust** handles everything that touches the filesystem or network: bottle discovery across nine different Wine sources, archive extraction via `sevenz-rust` and `zip`, mod file deployment, Nexus Mods API calls, SKSE downloads from silverlock.org, and Skyrim SE version detection via executable hashing. The plugin-based game detection system (`GamePlugin` trait) makes adding new game support straightforward without touching core logic.

**SQLite** (via `rusqlite`) tracks installed mods and their files, enabling clean uninstalls and conflict detection. This was chosen over flat files because mod installs can involve hundreds of files across nested directories, and we need reliable queries for "which mod owns this file?"

**CSS custom properties** power the theme system rather than a CSS-in-JS library. A single set of semantic tokens (`--bg-base`, `--surface`, `--accent`, `--separator`) is redefined under `[data-theme="dark"]` and `[data-theme="light"]` selectors, with vibrancy overrides for macOS transparency. This keeps the styling framework-free and fast.

### Project structure

```
src/                          Svelte frontend
├── lib/
│   ├── api.ts                Tauri IPC bindings (typed invoke wrappers)
│   ├── types.ts              Shared TypeScript interfaces
│   ├── stores.ts             Svelte stores (game selection, mods, toasts)
│   ├── theme.ts              Theme detection, persistence, and vibrancy
│   └── components/
│       └── ThemeToggle.svelte  Light / Auto / Dark segmented control
├── routes/
│   ├── +layout.svelte        Shell: sidebar nav, toast system, theme init
│   ├── +page.svelte          Dashboard (bottle scanning, game discovery)
│   ├── mods/+page.svelte     Mod management, Play button, SKSE, drag-and-drop
│   ├── plugins/+page.svelte  Plugin load order editor
│   ├── settings/+page.svelte Config, appearance, game tools
│   └── about/+page.svelte    Version, credits, acknowledgments
└── app.css                   Design system (tokens, themes, vibrancy)

src-tauri/src/
├── lib.rs              Tauri command handlers (18 IPC commands)
├── bottles.rs          Bottle detection (9 sources, macOS + Linux)
├── games.rs            Game detection framework + plugin registry
├── installer.rs        Archive extraction (.zip, .7z) + mod deployment
├── database.rs         SQLite mod tracking (installs, files, conflicts)
├── launcher.rs         Game launching through Wine/CrossOver/Whisky/Proton
├── skse.rs             SKSE detection, download, and installation
├── downgrader.rs       Skyrim version detection + Stock Game creation
├── nexus.rs            Nexus Mods API client (async/reqwest)
├── config.rs           JSON configuration (dirs crate for platform paths)
├── fomod.rs            FOMOD XML installer parser (quick-xml)
└── plugins/
    ├── skyrim_se.rs          Skyrim SE detection (Steam + GOG paths)
    └── skyrim_plugins.rs     Plugin load order management
```

### How mods are installed

1. User drops an archive or clicks Install — the frontend calls `install_mod_cmd` via Tauri IPC
2. The `installer` module extracts the archive to a temp directory and uses heuristics to find the mod root (looking for `Data/`, `.esp`/`.esm` files, or a single wrapper folder)
3. Files are copied into the game's `Data/` directory inside the Wine prefix
4. Every deployed file path is recorded in SQLite, linked to the mod entry
5. Enabling/disabling a mod moves files between the active `Data/` directory and a staging area
6. Uninstalling deletes exactly the files that were recorded, nothing more

### How game launching works

The `launcher` module resolves the correct Wine binary based on the bottle source — CrossOver uses its bundled Wine, Whisky uses its container Wine, Proton uses its bundled Wine, and everything else falls back to the system `wine` on PATH. The `WINEPREFIX` environment variable is set to the bottle path, and the game executable is spawned detached so Corkscrew doesn't block.

For Skyrim with SKSE enabled, the launcher swaps the executable from `SkyrimSE.exe` to `skse64_loader.exe`.

---

## Contributing

This is a young project and there's plenty to do. If you're a Mac or Linux gamer who's tired of manually dragging files into Wine prefixes, you're the target audience — and probably the ideal contributor.

Bug reports, feature requests, and pull requests are all welcome.

## Acknowledgments

- [SKSE Team](https://skse.silverlock.org/) for the Skyrim Script Extender
- [CrossOver](https://www.codeweavers.com/crossover) by CodeWeavers
- [Wine](https://www.winehq.org/) and all the compatibility layer projects
- [Nexus Mods](https://www.nexusmods.com/) for the modding community and API
- [Wabbajack](https://www.wabbajack.org/) for pioneering automated modlist installation and the Stock Game approach
- [Mod Organizer 2](https://github.com/ModOrganizer2/modorganizer) and [Vortex](https://github.com/Nexus-Mods/Vortex) for blazing the trail
- The [FOMOD](https://fomod-docs.readthedocs.io/) standard

## License

GPL-3.0-or-later. See [LICENSE](LICENSE) for details.
