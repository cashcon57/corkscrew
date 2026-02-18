"""Archive extraction and mod file deployment."""

import shutil
import tempfile
import zipfile
from pathlib import Path

import py7zr

from .database import ModDatabase
from .games import DetectedGame


def extract_archive(archive_path: Path, dest_dir: Path) -> list[Path]:
    """Extract a mod archive and return the list of extracted files.

    Supports .zip, .7z, and .rar formats.
    """
    dest_dir.mkdir(parents=True, exist_ok=True)
    suffix = archive_path.suffix.lower()

    if suffix == ".zip":
        with zipfile.ZipFile(archive_path, "r") as zf:
            zf.extractall(dest_dir)
    elif suffix == ".7z":
        with py7zr.SevenZipFile(archive_path, "r") as sz:
            sz.extractall(dest_dir)
    elif suffix == ".rar":
        import rarfile
        with rarfile.RarFile(archive_path, "r") as rf:
            rf.extractall(dest_dir)
    else:
        raise ValueError(f"Unsupported archive format: {suffix}")

    return list(dest_dir.rglob("*"))


def _find_data_root(extracted_dir: Path, game: DetectedGame) -> Path:
    """Detect the actual mod root within extracted files.

    Many Skyrim mods have their files nested inside a subfolder,
    or contain a 'Data' folder that should be merged with the game's Data dir.
    """
    # Check if there's a single top-level directory (common packaging pattern)
    top_entries = [e for e in extracted_dir.iterdir() if not e.name.startswith(".")]
    if len(top_entries) == 1 and top_entries[0].is_dir():
        inner = top_entries[0]
        # If the inner folder IS named "Data", its contents go into Data/
        if inner.name.lower() == "data":
            return inner
        # If the inner folder contains typical mod files, use it
        if _looks_like_mod_content(inner):
            return inner

    # Check if extracted root contains a "Data" folder
    for entry in extracted_dir.iterdir():
        if entry.name.lower() == "data" and entry.is_dir():
            return entry

    # Check if the extracted root itself looks like mod content
    if _looks_like_mod_content(extracted_dir):
        return extracted_dir

    # Default: use the extracted root
    return extracted_dir


def _looks_like_mod_content(directory: Path) -> bool:
    """Heuristic: does this directory contain typical Skyrim mod files?"""
    mod_extensions = {
        ".esp", ".esm", ".esl",  # Plugins
        ".bsa", ".ba2",           # Archives
        ".nif", ".dds", ".tga",   # Meshes and textures
        ".hkx", ".pex",           # Animations and scripts
        ".seq", ".swf", ".fuz",   # Misc
    }
    mod_folders = {"meshes", "textures", "scripts", "interface", "sound", "skse"}

    for entry in directory.iterdir():
        if entry.is_file() and entry.suffix.lower() in mod_extensions:
            return True
        if entry.is_dir() and entry.name.lower() in mod_folders:
            return True
    return False


def install_mod(
    archive_path: Path,
    game: DetectedGame,
    db: ModDatabase,
    mod_name: str | None = None,
    mod_version: str = "",
    nexus_mod_id: int | None = None,
) -> int:
    """Install a mod from an archive into the game's data directory.

    Returns the mod ID in the database.
    """
    if mod_name is None:
        mod_name = archive_path.stem

    data_dir = game.data_dir
    data_dir.mkdir(parents=True, exist_ok=True)

    # Extract to a temp directory first
    with tempfile.TemporaryDirectory() as tmp:
        tmp_path = Path(tmp)
        extract_archive(archive_path, tmp_path)

        # Find the actual mod content root
        mod_root = _find_data_root(tmp_path, game)

        # Deploy files
        installed_files: list[str] = []

        for src_file in mod_root.rglob("*"):
            if src_file.is_dir():
                continue

            relative = src_file.relative_to(mod_root)
            dest_file = data_dir / relative

            dest_file.parent.mkdir(parents=True, exist_ok=True)

            # Copy the file (overwrite if exists)
            shutil.copy2(src_file, dest_file)
            installed_files.append(str(relative))

    # Record in database
    mod_id = db.add_mod(
        game_id=game.game_id,
        bottle_name=game.bottle_name,
        name=mod_name,
        version=mod_version,
        archive_name=archive_path.name,
        installed_files=installed_files,
        nexus_mod_id=nexus_mod_id,
    )

    return mod_id


def uninstall_mod(mod_id: int, game: DetectedGame, db: ModDatabase) -> list[str]:
    """Uninstall a mod by removing its files and database record.

    Returns list of files that were removed.
    """
    mod = db.remove_mod(mod_id)
    if not mod:
        raise ValueError(f"Mod with ID {mod_id} not found")

    removed: list[str] = []
    data_dir = game.data_dir

    # Remove files in reverse order (deepest first)
    for relative_path in sorted(mod.installed_files, reverse=True):
        full_path = data_dir / relative_path
        if full_path.exists() and full_path.is_file():
            full_path.unlink()
            removed.append(relative_path)

            # Clean up empty parent directories
            parent = full_path.parent
            while parent != data_dir:
                if parent.exists() and not any(parent.iterdir()):
                    parent.rmdir()
                    parent = parent.parent
                else:
                    break

    return removed
