"""Corkscrew CLI — mod manager for CrossOver/Wine games on macOS."""

from pathlib import Path

import click
from rich.console import Console
from rich.table import Table
from rich.progress import Progress, BarColumn, DownloadColumn, TransferSpeedColumn

from . import __version__
from .bottles import detect_bottles, find_bottle_by_name
from .games import detect_all_games, detect_games
from .database import ModDatabase
from .installer import install_mod, uninstall_mod
from .nexus import NexusClient, NXMLink
from .config import get_config, set_config_value

# Ensure game plugins are registered
from . import plugins as _plugins  # noqa: F401

console = Console()

DEFAULT_DB_PATH = Path.home() / ".local" / "share" / "corkscrew" / "mods.db"
DEFAULT_DOWNLOAD_DIR = Path.home() / ".local" / "share" / "corkscrew" / "downloads"


def get_db() -> ModDatabase:
    return ModDatabase(DEFAULT_DB_PATH)


@click.group()
@click.version_option(__version__)
def main():
    """Corkscrew — mod manager for CrossOver/Wine games on macOS."""
    pass


# --- Bottle commands ---


@main.command("bottles")
def list_bottles():
    """List detected Wine bottles."""
    bottles = detect_bottles()
    if not bottles:
        console.print("[yellow]No bottles found.[/yellow]")
        console.print("Corkscrew looks for bottles from CrossOver, Whisky, Moonshine, and Mythic.")
        return

    table = Table(title="Detected Bottles")
    table.add_column("Name", style="cyan")
    table.add_column("Source", style="green")
    table.add_column("Path", style="dim")

    for b in bottles:
        table.add_row(b.name, b.source, str(b.path))

    console.print(table)


# --- Game commands ---


@main.command("games")
@click.option("--bottle", "-b", help="Only scan a specific bottle")
def list_games(bottle: str | None):
    """List detected games in bottles."""
    if bottle:
        b = find_bottle_by_name(bottle)
        if not b:
            console.print(f"[red]Bottle '{bottle}' not found.[/red]")
            return
        games = detect_games(b)
    else:
        games = detect_all_games()

    if not games:
        console.print("[yellow]No supported games found.[/yellow]")
        return

    table = Table(title="Detected Games")
    table.add_column("Game", style="cyan")
    table.add_column("Bottle", style="green")
    table.add_column("Path", style="dim")

    for g in games:
        table.add_row(g.display_name, g.bottle_name, str(g.game_path))

    console.print(table)


# --- Mod management commands ---


@main.command("install")
@click.argument("archive", type=click.Path(exists=True, path_type=Path))
@click.option("--game", "-g", required=True, help="Game ID (e.g. skyrimse)")
@click.option("--bottle", "-b", required=True, help="Bottle name")
@click.option("--name", "-n", help="Mod name (defaults to archive filename)")
@click.option("--version", "-v", default="", help="Mod version")
def install_from_archive(archive: Path, game: str, bottle: str, name: str | None, version: str):
    """Install a mod from a local archive file."""
    b = find_bottle_by_name(bottle)
    if not b:
        console.print(f"[red]Bottle '{bottle}' not found.[/red]")
        return

    games = detect_games(b)
    detected = next((g for g in games if g.game_id == game), None)
    if not detected:
        console.print(f"[red]Game '{game}' not found in bottle '{bottle}'.[/red]")
        return

    with get_db() as db:
        # Check for conflicts
        console.print(f"Installing [cyan]{name or archive.stem}[/cyan] into [green]{detected.display_name}[/green]...")

        mod_id = install_mod(
            archive_path=archive,
            game=detected,
            db=db,
            mod_name=name,
            mod_version=version,
        )

        mod = db.get_mod(mod_id)
        console.print(f"[green]Installed![/green] {len(mod.installed_files)} files deployed. (Mod ID: {mod_id})")

        # Sync plugin load order for Skyrim
        if game == "skyrimse":
            _sync_skyrim_plugins(detected, b)


@main.command("uninstall")
@click.argument("mod_id", type=int)
@click.option("--game", "-g", required=True, help="Game ID")
@click.option("--bottle", "-b", required=True, help="Bottle name")
def uninstall(mod_id: int, game: str, bottle: str):
    """Uninstall a mod by its ID."""
    b = find_bottle_by_name(bottle)
    if not b:
        console.print(f"[red]Bottle '{bottle}' not found.[/red]")
        return

    games = detect_games(b)
    detected = next((g for g in games if g.game_id == game), None)
    if not detected:
        console.print(f"[red]Game '{game}' not found in bottle '{bottle}'.[/red]")
        return

    with get_db() as db:
        mod = db.get_mod(mod_id)
        if not mod:
            console.print(f"[red]Mod ID {mod_id} not found.[/red]")
            return

        removed = uninstall_mod(mod_id, detected, db)
        console.print(f"[green]Uninstalled[/green] {mod.name} — {len(removed)} files removed.")

        if game == "skyrimse":
            _sync_skyrim_plugins(detected, b)


@main.command("mods")
@click.option("--game", "-g", required=True, help="Game ID")
@click.option("--bottle", "-b", required=True, help="Bottle name")
def list_mods(game: str, bottle: str):
    """List installed mods for a game."""
    with get_db() as db:
        mods = db.list_mods(game, bottle)

    if not mods:
        console.print("[yellow]No mods installed.[/yellow]")
        return

    table = Table(title=f"Installed Mods ({game} in {bottle})")
    table.add_column("ID", style="dim")
    table.add_column("Name", style="cyan")
    table.add_column("Version")
    table.add_column("Files", justify="right")
    table.add_column("Enabled", justify="center")

    for mod in mods:
        enabled_str = "[green]Yes[/green]" if mod.enabled else "[red]No[/red]"
        table.add_row(str(mod.id), mod.name, mod.version, str(len(mod.installed_files)), enabled_str)

    console.print(table)


# --- Nexus Mods commands ---


@main.command("nexus-download")
@click.argument("nxm_url")
@click.option("--game", "-g", required=True, help="Game ID")
@click.option("--bottle", "-b", required=True, help="Bottle name")
@click.option("--install/--no-install", default=True, help="Auto-install after download")
def nexus_download(nxm_url: str, game: str, bottle: str, install: bool):
    """Download (and optionally install) a mod from an NXM link."""
    config = get_config()
    api_key = config.get("nexus_api_key")
    if not api_key:
        console.print("[red]No Nexus API key configured.[/red]")
        console.print("Get your API key from https://www.nexusmods.com/users/myaccount?tab=api+access")
        console.print("Then run: corkscrew config set nexus_api_key YOUR_KEY")
        return

    nxm = NXMLink.parse(nxm_url)
    console.print(f"Downloading mod [cyan]{nxm.mod_id}[/cyan] file [cyan]{nxm.file_id}[/cyan]...")

    download_dir = Path(config.get("download_dir", str(DEFAULT_DOWNLOAD_DIR)))

    with NexusClient(api_key) as client:
        # Get mod info for display
        try:
            mod_info = client.get_mod(nxm.game_slug, nxm.mod_id)
            mod_name = mod_info.get("name", f"Mod {nxm.mod_id}")
            mod_version = mod_info.get("version", "")
            console.print(f"Mod: [cyan]{mod_name}[/cyan] v{mod_version}")
        except Exception:
            mod_name = f"Mod {nxm.mod_id}"
            mod_version = ""

        with Progress(
            "[progress.description]{task.description}",
            BarColumn(),
            DownloadColumn(),
            TransferSpeedColumn(),
            console=console,
        ) as progress:
            task = progress.add_task("Downloading...", total=None)

            def on_progress(downloaded: int, total: int):
                progress.update(task, completed=downloaded, total=total)

            archive_path = client.download_from_nxm(nxm, download_dir, on_progress)

        console.print(f"[green]Downloaded:[/green] {archive_path.name}")

        if install:
            b = find_bottle_by_name(bottle)
            if not b:
                console.print(f"[red]Bottle '{bottle}' not found. File saved to {archive_path}[/red]")
                return

            games = detect_games(b)
            detected = next((g for g in games if g.game_id == game), None)
            if not detected:
                console.print(f"[red]Game '{game}' not found in bottle '{bottle}'. File saved to {archive_path}[/red]")
                return

            with get_db() as db:
                mod_id = install_mod(
                    archive_path=archive_path,
                    game=detected,
                    db=db,
                    mod_name=mod_name,
                    mod_version=mod_version,
                    nexus_mod_id=nxm.mod_id,
                )
                mod = db.get_mod(mod_id)
                console.print(f"[green]Installed![/green] {len(mod.installed_files)} files deployed. (Mod ID: {mod_id})")

                if game == "skyrimse":
                    _sync_skyrim_plugins(detected, b)


# --- Config commands ---


@main.group("config")
def config_group():
    """Manage Corkscrew configuration."""
    pass


@config_group.command("set")
@click.argument("key")
@click.argument("value")
def config_set(key: str, value: str):
    """Set a configuration value."""
    set_config_value(key, value)
    console.print(f"[green]Set[/green] {key} = {value}")


@config_group.command("show")
def config_show():
    """Show current configuration."""
    config = get_config()
    if not config:
        console.print("[yellow]No configuration set.[/yellow]")
        return

    table = Table(title="Configuration")
    table.add_column("Key", style="cyan")
    table.add_column("Value")

    for key, value in sorted(config.items()):
        display_value = value
        if "key" in key.lower() or "token" in key.lower():
            display_value = value[:8] + "..." if len(value) > 8 else "***"
        table.add_row(key, display_value)

    console.print(table)


# --- Plugin order commands ---


@main.command("plugins")
@click.option("--game", "-g", default="skyrimse", help="Game ID")
@click.option("--bottle", "-b", required=True, help="Bottle name")
def list_plugins(game: str, bottle: str):
    """List plugin load order (Skyrim SE)."""
    if game != "skyrimse":
        console.print("[yellow]Plugin management is currently only supported for Skyrim SE.[/yellow]")
        return

    from .plugins.skyrim_se import SkyrimSEPlugin
    from .plugins.skyrim_plugins import read_plugins_txt

    b = find_bottle_by_name(bottle)
    if not b:
        console.print(f"[red]Bottle '{bottle}' not found.[/red]")
        return

    plugin = SkyrimSEPlugin()
    detected = plugin.detect(b)
    if not detected:
        console.print("[red]Skyrim SE not found in this bottle.[/red]")
        return

    plugins_file = plugin.get_plugins_file(detected.game_path, b)
    if not plugins_file or not plugins_file.exists():
        console.print("[yellow]No plugins.txt found.[/yellow]")
        return

    entries = read_plugins_txt(plugins_file)
    table = Table(title="Plugin Load Order")
    table.add_column("#", justify="right", style="dim")
    table.add_column("Plugin", style="cyan")
    table.add_column("Type")
    table.add_column("Enabled", justify="center")

    for i, entry in enumerate(entries):
        ptype = "ESM" if entry.is_esm else ("ESL" if entry.is_esl else "ESP")
        enabled = "[green]Yes[/green]" if entry.enabled else "[red]No[/red]"
        table.add_row(str(i), entry.filename, ptype, enabled)

    console.print(table)


# --- Helpers ---


def _sync_skyrim_plugins(game, bottle):
    """Sync Skyrim plugin load order after mod changes."""
    from .plugins.skyrim_se import SkyrimSEPlugin
    from .plugins.skyrim_plugins import sync_plugins

    plugin = SkyrimSEPlugin()
    plugins_file = plugin.get_plugins_file(game.game_path, bottle)
    loadorder_file = plugin.get_loadorder_file(game.game_path, bottle)

    if plugins_file:
        sync_plugins(game.data_dir, plugins_file, loadorder_file)
        console.print("[dim]Plugin load order synced.[/dim]")
