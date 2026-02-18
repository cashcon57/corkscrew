"""Bottle detection for CrossOver, Whisky, Moonshine, and other Wine managers."""

from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterator


# Known bottle locations for various Wine managers on macOS
BOTTLE_SEARCH_PATHS = [
    # CrossOver
    Path.home() / "Library" / "Application Support" / "CrossOver" / "Bottles",
    # Whisky / Moonshine
    Path.home() / "Library" / "Containers" / "com.isaacmarovitz.Whisky" / "Bottles",
    Path.home() / "Library" / "Containers" / "com.ybmeng.moonshine" / "Bottles",
    # Heroic Games Launcher (uses Wine prefixes)
    Path.home() / "Library" / "Application Support" / "heroic" / "Prefixes",
    # Mythic
    Path.home() / "Library" / "Containers" / "io.getmythic.Mythic" / "Bottles",
]


@dataclass
class Bottle:
    """Represents a Wine bottle (prefix) on macOS."""

    name: str
    path: Path
    source: str  # Which manager created it (CrossOver, Whisky, etc.)

    @property
    def drive_c(self) -> Path:
        return self.path / "drive_c"

    @property
    def program_files(self) -> Path:
        return self.drive_c / "Program Files"

    @property
    def program_files_x86(self) -> Path:
        return self.drive_c / "Program Files (x86)"

    @property
    def users_dir(self) -> Path:
        return self.drive_c / "users"

    @property
    def appdata_local(self) -> Path:
        """Best-effort path to Local AppData."""
        # CrossOver typically symlinks to the bottle's user dir
        for user_dir in self.users_dir.iterdir() if self.users_dir.exists() else []:
            local = user_dir / "AppData" / "Local"
            if local.exists():
                return local
            # Some bottles use lowercase
            local = user_dir / "Local Settings" / "Application Data"
            if local.exists():
                return local
        return self.users_dir / "crossover" / "AppData" / "Local"

    def exists(self) -> bool:
        return self.drive_c.exists()

    def find_path(self, *parts: str) -> Path | None:
        """Find a path within the bottle, case-insensitively."""
        current = self.drive_c
        for part in parts:
            if not current.exists():
                return None
            # Try exact match first
            candidate = current / part
            if candidate.exists():
                current = candidate
                continue
            # Case-insensitive fallback
            found = False
            for child in current.iterdir():
                if child.name.lower() == part.lower():
                    current = child
                    found = True
                    break
            if not found:
                return None
        return current


def detect_bottles() -> list[Bottle]:
    """Scan all known locations for Wine bottles."""
    bottles: list[Bottle] = []

    source_map = {
        "CrossOver": BOTTLE_SEARCH_PATHS[0],
        "Whisky": BOTTLE_SEARCH_PATHS[1],
        "Moonshine": BOTTLE_SEARCH_PATHS[2],
        "Heroic": BOTTLE_SEARCH_PATHS[3],
        "Mythic": BOTTLE_SEARCH_PATHS[4],
    }

    for source, search_path in source_map.items():
        if not search_path.exists():
            continue
        for entry in sorted(search_path.iterdir()):
            if entry.is_dir():
                bottle = Bottle(name=entry.name, path=entry, source=source)
                if bottle.exists():
                    bottles.append(bottle)

    return bottles


def find_bottle_by_name(name: str) -> Bottle | None:
    """Find a specific bottle by name (case-insensitive)."""
    for bottle in detect_bottles():
        if bottle.name.lower() == name.lower():
            return bottle
    return None
