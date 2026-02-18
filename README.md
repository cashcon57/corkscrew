# Corkscrew 🍷

**A mod manager for CrossOver and Wine games on macOS.**

Corkscrew lets you install, manage, and organize mods for Windows games running through [CrossOver](https://www.codeweavers.com/crossover), [Whisky](https://getwhisky.app/), and other Wine-based compatibility layers on macOS — no Windows VM required.

It works by reading and writing directly to your Wine bottle's filesystem, the same way the game itself sees it. Your bottles, your mods, no middleman.

> **Status:** Early development. Skyrim Special Edition is the first supported game. Expect rough edges — and contributions.

---

## What It Does

- **Detects your bottles** — Finds CrossOver, Whisky, Moonshine, Heroic, and Mythic bottles automatically
- **Finds your games** — Scans bottles for supported titles (Skyrim SE via Steam or GOG to start)
- **Installs mods from archives** — Handles `.zip`, `.7z`, and `.rar` files, deploys to the right `Data/` directory
- **Downloads from Nexus Mods** — Paste an NXM link, Corkscrew fetches and installs it
- **Tracks what you've installed** — SQLite database keeps tabs on every file so uninstalls are clean
- **Manages plugin load order** — Reads and syncs `plugins.txt` for Bethesda games
- **Parses FOMOD installers** — Understands the standard XML-based mod installer format

## What It Doesn't Do (Yet)

- GUI — it's CLI-only for now
- Support games beyond Skyrim SE
- Handle mod conflicts with anything smarter than priority ordering
- Wabbajack or NexusMods collection/modlist support
- NXM protocol handler registration (you paste the link manually)

These are all on the roadmap. One bottle at a time.

---

## Installation

Requires Python 3.10+ and a macOS system with CrossOver or another Wine-based runner.

```bash
# Clone and install in development mode
git clone https://github.com/cashcon57/corkscrew.git
cd corkscrew
pip install -e .
```

## Quick Start

```bash
# See what bottles Corkscrew can find
corkscrew bottles

# See what games are in those bottles
corkscrew games

# Install a mod from a local archive
corkscrew install ~/Downloads/SomeSkryimMod.zip -g skyrimse -b "My Bottle"

# List installed mods
corkscrew mods -g skyrimse -b "My Bottle"

# Check plugin load order
corkscrew plugins -b "My Bottle"

# Uninstall by mod ID
corkscrew uninstall 1 -g skyrimse -b "My Bottle"
```

### Nexus Mods Integration

```bash
# Set your personal API key (get one from nexusmods.com → API Access)
corkscrew config set nexus_api_key YOUR_KEY_HERE

# Download and install from an NXM link
corkscrew nexus-download "nxm://skyrimspecialedition/mods/12345/files/67890?key=abc&expires=123" \
  -g skyrimse -b "My Bottle"
```

---

## Supported Bottle Sources

| Source | Path |
|--------|------|
| CrossOver | `~/Library/Application Support/CrossOver/Bottles/` |
| Whisky | `~/Library/Containers/com.isaacmarovitz.Whisky/Bottles/` |
| Moonshine | `~/Library/Containers/com.isaacmarovitz.Moonshine/Bottles/` |
| Heroic (Wine) | `~/Games/Heroic/Prefixes/` |
| Mythic | `~/Library/Application Support/Mythic/Bottles/` |

## Supported Games

| Game | ID | Status |
|------|----|--------|
| Skyrim Special Edition | `skyrimse` | Working |
| *More to come* | | Planned |

Adding a new game is a matter of writing a small plugin — see [skyrim_se.py](src/corkscrew/plugins/skyrim_se.py) for the pattern.

---

## How It Works

Wine bottles are just directories. A CrossOver bottle at `~/Library/Application Support/CrossOver/Bottles/MyBottle/` has a `drive_c/` folder that maps to the game's `C:\` drive. Corkscrew navigates this structure natively from macOS to find game installs and deploy mod files — no Wine runtime needed, no Windows tools required.

For Skyrim SE, mods typically go into `drive_c/Program Files (x86)/Steam/steamapps/common/Skyrim Special Edition/Data/`. Corkscrew figures out the right path, extracts your archive, and puts files where the game expects them.

---

## Project Structure

```
src/corkscrew/
├── bottles.py          # Bottle detection (CrossOver, Whisky, etc.)
├── games.py            # Game detection framework + plugin registry
├── installer.py        # Archive extraction and file deployment
├── database.py         # SQLite mod tracking
├── nexus.py            # Nexus Mods API client
├── config.py           # User configuration
├── cli.py              # CLI interface (Click + Rich)
├── plugins/
│   ├── skyrim_se.py    # Skyrim SE game plugin
│   └── skyrim_plugins.py  # Plugin load order management
└── fomod/
    └── parser.py       # FOMOD installer XML parser
```

## Contributing

This is a young project and there's plenty to do. If you're a Mac gamer who's tired of manually dragging files into Wine prefixes, you're the target audience — and probably the ideal contributor.

Bug reports, feature requests, and pull requests are all welcome. If you want to add support for a new game, the plugin system is designed to make that straightforward.

## Acknowledgments

Corkscrew wouldn't exist without the ecosystem it builds on:

- [CrossOver](https://www.codeweavers.com/crossover) by CodeWeavers — for making Windows games work on macOS
- [Nexus Mods](https://www.nexusmods.com/) — for the modding community and API
- [Mod Organizer 2](https://github.com/ModOrganizer2/modorganizer) and [Vortex](https://github.com/Nexus-Mods/Vortex) — for blazing the trail on mod management
- The [FOMOD](https://fomod-docs.readthedocs.io/) standard — for a sane installer format
- [Wine](https://www.winehq.org/) — for the compatibility layer that makes all of this possible

## License

GPL-3.0-or-later. See [LICENSE](LICENSE) for details.
