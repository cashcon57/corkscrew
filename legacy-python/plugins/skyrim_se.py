"""Skyrim Special Edition game plugin."""

from dataclasses import dataclass
from pathlib import Path

from ..bottles import Bottle
from ..games import DetectedGame, register_plugin


# Known install locations within a Wine bottle
STEAM_COMMON = ("Program Files (x86)", "Steam", "steamapps", "common")
GOG_PATHS = [
    ("GOG Galaxy", "Games", "Skyrim Special Edition"),
    ("Program Files (x86)", "GOG Galaxy", "Games", "Skyrim Special Edition"),
]

SKYRIM_FOLDER_NAMES = [
    "Skyrim Special Edition",
    "Skyrim Special Edition GOG",
]


@dataclass
class SkyrimSEPlugin:
    game_id: str = "skyrimse"
    display_name: str = "Skyrim Special Edition"
    nexus_slug: str = "skyrimspecialedition"
    executables: list[str] = None

    def __post_init__(self):
        if self.executables is None:
            self.executables = ["SkyrimSE.exe", "SkyrimSELauncher.exe"]

    def detect(self, bottle: Bottle) -> DetectedGame | None:
        # Check Steam install locations
        for folder_name in SKYRIM_FOLDER_NAMES:
            game_path = bottle.find_path(*STEAM_COMMON, folder_name)
            if game_path and self._verify_game(game_path):
                return self._make_result(game_path, bottle)

        # Check GOG install locations
        for gog_parts in GOG_PATHS:
            game_path = bottle.find_path(*gog_parts)
            if game_path and self._verify_game(game_path):
                return self._make_result(game_path, bottle)

        # Broad search: look for SkyrimSE.exe anywhere in common Steam library folders
        steam_root = bottle.find_path("Program Files (x86)", "Steam")
        if steam_root:
            libraryfolders = steam_root / "steamapps" / "libraryfolders.vdf"
            if libraryfolders.exists():
                for lib_path in self._parse_library_folders(libraryfolders, bottle):
                    for folder_name in SKYRIM_FOLDER_NAMES:
                        game_path = lib_path / "steamapps" / "common" / folder_name
                        if game_path.exists() and self._verify_game(game_path):
                            return self._make_result(game_path, bottle)

        return None

    def _verify_game(self, game_path: Path) -> bool:
        """Verify the game directory contains the expected executable."""
        for exe in self.executables:
            # Case-insensitive check
            for f in game_path.iterdir() if game_path.exists() else []:
                if f.name.lower() == exe.lower():
                    return True
        return False

    def get_data_dir(self, game_path: Path) -> Path:
        """Skyrim mods go into the Data/ subdirectory."""
        data = game_path / "Data"
        if not data.exists():
            # Case-insensitive fallback
            for child in game_path.iterdir():
                if child.name.lower() == "data" and child.is_dir():
                    return child
        return data

    def get_plugins_file(self, game_path: Path, bottle: Bottle) -> Path | None:
        """Return path to plugins.txt in the bottle's AppData."""
        appdata = bottle.appdata_local
        plugins = appdata / "Skyrim Special Edition" / "plugins.txt"
        if plugins.exists():
            return plugins
        # Also check Plugins.txt (capitalization varies)
        parent = appdata / "Skyrim Special Edition"
        if parent.exists():
            for f in parent.iterdir():
                if f.name.lower() == "plugins.txt":
                    return f
        return plugins  # Return expected path even if it doesn't exist yet

    def get_loadorder_file(self, game_path: Path, bottle: Bottle) -> Path | None:
        """Return path to loadorder.txt in the bottle's AppData."""
        appdata = bottle.appdata_local
        return appdata / "Skyrim Special Edition" / "loadorder.txt"

    def _make_result(self, game_path: Path, bottle: Bottle) -> DetectedGame:
        return DetectedGame(
            game_id=self.game_id,
            display_name=self.display_name,
            nexus_slug=self.nexus_slug,
            game_path=game_path,
            data_dir=self.get_data_dir(game_path),
            bottle=bottle,
            plugin=self,
        )

    def _parse_library_folders(self, vdf_path: Path, bottle: Bottle) -> list[Path]:
        """Parse Steam's libraryfolders.vdf for additional library paths."""
        paths: list[Path] = []
        try:
            content = vdf_path.read_text(encoding="utf-8")
            import re
            # Match "path" entries in the VDF
            for match in re.finditer(r'"path"\s+"([^"]+)"', content):
                raw_path = match.group(1).replace("\\\\", "/").replace("\\", "/")
                # Convert Windows paths to bottle paths
                if raw_path.lower().startswith("c:"):
                    real_path = bottle.drive_c / raw_path[3:]
                elif raw_path.lower().startswith("d:"):
                    # Some bottles mount additional drives
                    real_path = bottle.path / "drive_d" / raw_path[3:]
                else:
                    continue
                if real_path.exists():
                    paths.append(real_path)
        except Exception:
            pass
        return paths


# Auto-register on import
_plugin = SkyrimSEPlugin()
register_plugin(_plugin)
