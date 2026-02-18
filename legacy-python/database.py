"""Mod tracking database using SQLite."""

import json
import sqlite3
from dataclasses import dataclass
from pathlib import Path
from datetime import datetime, timezone


@dataclass
class InstalledMod:
    """Record of an installed mod."""

    id: int
    game_id: str
    bottle_name: str
    nexus_mod_id: int | None
    name: str
    version: str
    archive_name: str
    installed_files: list[str]
    installed_at: str
    enabled: bool = True


class ModDatabase:
    """SQLite database for tracking installed mods."""

    def __init__(self, db_path: Path):
        self.db_path = db_path
        db_path.parent.mkdir(parents=True, exist_ok=True)
        self._conn = sqlite3.connect(str(db_path))
        self._conn.row_factory = sqlite3.Row
        self._init_schema()

    def _init_schema(self):
        self._conn.executescript("""
            CREATE TABLE IF NOT EXISTS installed_mods (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                game_id TEXT NOT NULL,
                bottle_name TEXT NOT NULL,
                nexus_mod_id INTEGER,
                name TEXT NOT NULL,
                version TEXT DEFAULT '',
                archive_name TEXT DEFAULT '',
                installed_files TEXT NOT NULL DEFAULT '[]',
                installed_at TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1
            );

            CREATE INDEX IF NOT EXISTS idx_mods_game_bottle
                ON installed_mods(game_id, bottle_name);
        """)
        self._conn.commit()

    def close(self):
        self._conn.close()

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.close()

    def add_mod(
        self,
        game_id: str,
        bottle_name: str,
        name: str,
        version: str,
        archive_name: str,
        installed_files: list[str],
        nexus_mod_id: int | None = None,
    ) -> int:
        """Record a newly installed mod. Returns the mod ID."""
        cursor = self._conn.execute(
            """INSERT INTO installed_mods
               (game_id, bottle_name, nexus_mod_id, name, version, archive_name,
                installed_files, installed_at, enabled)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1)""",
            (
                game_id,
                bottle_name,
                nexus_mod_id,
                name,
                version,
                archive_name,
                json.dumps(installed_files),
                datetime.now(timezone.utc).isoformat(),
            ),
        )
        self._conn.commit()
        return cursor.lastrowid

    def remove_mod(self, mod_id: int) -> InstalledMod | None:
        """Remove a mod record and return it (for file cleanup)."""
        mod = self.get_mod(mod_id)
        if mod:
            self._conn.execute("DELETE FROM installed_mods WHERE id = ?", (mod_id,))
            self._conn.commit()
        return mod

    def get_mod(self, mod_id: int) -> InstalledMod | None:
        """Get a specific mod by ID."""
        row = self._conn.execute(
            "SELECT * FROM installed_mods WHERE id = ?", (mod_id,)
        ).fetchone()
        return self._row_to_mod(row) if row else None

    def list_mods(self, game_id: str, bottle_name: str) -> list[InstalledMod]:
        """List all mods installed for a game in a bottle."""
        rows = self._conn.execute(
            "SELECT * FROM installed_mods WHERE game_id = ? AND bottle_name = ? ORDER BY installed_at",
            (game_id, bottle_name),
        ).fetchall()
        return [self._row_to_mod(row) for row in rows]

    def get_all_installed_files(self, game_id: str, bottle_name: str) -> dict[str, int]:
        """Get a mapping of installed file paths to the mod ID that owns them.

        Later mods win (overwrite tracking).
        """
        file_owners: dict[str, int] = {}
        for mod in self.list_mods(game_id, bottle_name):
            if mod.enabled:
                for f in mod.installed_files:
                    file_owners[f] = mod.id
        return file_owners

    def find_conflicts(self, game_id: str, bottle_name: str, new_files: list[str]) -> dict[str, str]:
        """Check which files from a new mod would overwrite existing ones.

        Returns {file_path: existing_mod_name}.
        """
        conflicts: dict[str, str] = {}
        existing = self.get_all_installed_files(game_id, bottle_name)
        for f in new_files:
            if f in existing:
                owner = self.get_mod(existing[f])
                if owner:
                    conflicts[f] = owner.name
        return conflicts

    def set_enabled(self, mod_id: int, enabled: bool):
        self._conn.execute(
            "UPDATE installed_mods SET enabled = ? WHERE id = ?",
            (1 if enabled else 0, mod_id),
        )
        self._conn.commit()

    def _row_to_mod(self, row: sqlite3.Row) -> InstalledMod:
        return InstalledMod(
            id=row["id"],
            game_id=row["game_id"],
            bottle_name=row["bottle_name"],
            nexus_mod_id=row["nexus_mod_id"],
            name=row["name"],
            version=row["version"],
            archive_name=row["archive_name"],
            installed_files=json.loads(row["installed_files"]),
            installed_at=row["installed_at"],
            enabled=bool(row["enabled"]),
        )
