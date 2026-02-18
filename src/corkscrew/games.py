"""Game detection within Wine bottles."""

from dataclasses import dataclass, field
from pathlib import Path
from typing import Protocol

from .bottles import Bottle


class GamePlugin(Protocol):
    """Interface for game-specific mod management logic."""

    game_id: str
    display_name: str
    nexus_slug: str
    executables: list[str]

    def detect(self, bottle: Bottle) -> "DetectedGame | None":
        """Check if this game exists in the given bottle."""
        ...

    def get_data_dir(self, game_path: Path) -> Path:
        """Return the directory where mods should be deployed."""
        ...

    def get_plugins_file(self, game_path: Path, bottle: Bottle) -> Path | None:
        """Return path to plugin load order file, if applicable."""
        ...


@dataclass
class DetectedGame:
    """A game found inside a Wine bottle."""

    game_id: str
    display_name: str
    nexus_slug: str
    game_path: Path
    data_dir: Path
    bottle: Bottle
    plugin: "GamePlugin"

    @property
    def bottle_name(self) -> str:
        return self.bottle.name


# Registry of all known game plugins
_game_plugins: list[GamePlugin] = []


def register_plugin(plugin: GamePlugin) -> None:
    _game_plugins.append(plugin)


def detect_games(bottle: Bottle) -> list[DetectedGame]:
    """Scan a bottle for all recognized games."""
    found: list[DetectedGame] = []
    for plugin in _game_plugins:
        result = plugin.detect(bottle)
        if result is not None:
            found.append(result)
    return found


def detect_all_games() -> list[DetectedGame]:
    """Scan all bottles for all recognized games."""
    from .bottles import detect_bottles

    found: list[DetectedGame] = []
    for bottle in detect_bottles():
        found.extend(detect_games(bottle))
    return found


def get_plugin_for_game(game_id: str) -> GamePlugin | None:
    for plugin in _game_plugins:
        if plugin.game_id == game_id:
            return plugin
    return None
