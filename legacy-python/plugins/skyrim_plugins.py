"""Skyrim SE plugin load order management.

Manages plugins.txt and loadorder.txt for Bethesda games.
"""

from dataclasses import dataclass
from pathlib import Path


# Skyrim SE always loads these first, in this order
IMPLICIT_PLUGINS = [
    "Skyrim.esm",
    "Update.esm",
    "Dawnguard.esm",
    "HearthFires.esm",
    "Dragonborn.esm",
]


@dataclass
class PluginEntry:
    """A single plugin in the load order."""

    filename: str
    enabled: bool = True

    @property
    def is_esl(self) -> bool:
        return self.filename.lower().endswith(".esl")

    @property
    def is_esm(self) -> bool:
        return self.filename.lower().endswith(".esm")


def read_plugins_txt(plugins_file: Path) -> list[PluginEntry]:
    """Parse plugins.txt format.

    Lines starting with * are enabled, others are disabled.
    Lines starting with # are comments.
    """
    entries: list[PluginEntry] = []
    if not plugins_file.exists():
        return entries

    for line in plugins_file.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("*"):
            entries.append(PluginEntry(filename=line[1:], enabled=True))
        else:
            entries.append(PluginEntry(filename=line, enabled=False))

    return entries


def write_plugins_txt(plugins_file: Path, entries: list[PluginEntry]):
    """Write plugins.txt format."""
    plugins_file.parent.mkdir(parents=True, exist_ok=True)
    lines = ["# This file is used by Skyrim Special Edition to keep track of your downloaded content."]
    lines.append("# Please do not modify this file.")

    for entry in entries:
        prefix = "*" if entry.enabled else ""
        lines.append(f"{prefix}{entry.filename}")

    plugins_file.write_text("\n".join(lines) + "\n", encoding="utf-8")


def read_loadorder_txt(loadorder_file: Path) -> list[str]:
    """Parse loadorder.txt — just a list of plugin filenames."""
    if not loadorder_file.exists():
        return []
    return [
        line.strip()
        for line in loadorder_file.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.startswith("#")
    ]


def write_loadorder_txt(loadorder_file: Path, plugins: list[str]):
    """Write loadorder.txt."""
    loadorder_file.parent.mkdir(parents=True, exist_ok=True)
    loadorder_file.write_text("\n".join(plugins) + "\n", encoding="utf-8")


def discover_plugins(data_dir: Path) -> list[str]:
    """Find all plugin files (.esp, .esm, .esl) in a game's Data directory."""
    extensions = {".esp", ".esm", ".esl"}
    plugins: list[str] = []

    if not data_dir.exists():
        return plugins

    for f in sorted(data_dir.iterdir()):
        if f.is_file() and f.suffix.lower() in extensions:
            plugins.append(f.name)

    return plugins


def sync_plugins(data_dir: Path, plugins_file: Path, loadorder_file: Path | None = None):
    """Sync plugin files on disk with plugins.txt.

    - New plugins found on disk are added as enabled
    - Plugins in the list but not on disk are removed
    - Existing order and enabled state are preserved
    """
    on_disk = set(discover_plugins(data_dir))
    existing = read_plugins_txt(plugins_file)

    # Build lookup of current state
    state: dict[str, bool] = {e.filename.lower(): e.enabled for e in existing}
    order: list[str] = [e.filename for e in existing if e.filename.lower() in {p.lower() for p in on_disk}]

    # Add new plugins not yet in the list
    ordered_lower = {p.lower() for p in order}
    for plugin in sorted(on_disk):
        if plugin.lower() not in ordered_lower:
            order.append(plugin)

    # Build final entries
    final: list[PluginEntry] = []
    for plugin in order:
        enabled = state.get(plugin.lower(), True)
        # Implicit plugins are always enabled
        if plugin in IMPLICIT_PLUGINS:
            enabled = True
        final.append(PluginEntry(filename=plugin, enabled=enabled))

    write_plugins_txt(plugins_file, final)

    if loadorder_file:
        write_loadorder_txt(loadorder_file, [e.filename for e in final])
