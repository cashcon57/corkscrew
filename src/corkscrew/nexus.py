"""Nexus Mods API client."""

from dataclasses import dataclass
from pathlib import Path
from urllib.parse import urlparse, parse_qs
from typing import Any

import httpx


NEXUS_API_BASE = "https://api.nexusmods.com/v1"


@dataclass
class NexusModFile:
    """A downloadable file from Nexus Mods."""

    mod_id: int
    file_id: int
    name: str
    version: str
    file_name: str
    size_kb: int
    description: str


@dataclass
class NXMLink:
    """Parsed nxm:// protocol link."""

    game_slug: str
    mod_id: int
    file_id: int
    key: str | None = None
    expires: str | None = None

    @classmethod
    def parse(cls, url: str) -> "NXMLink":
        """Parse an nxm:// URL into components.

        Format: nxm://<game>/mods/<mod_id>/files/<file_id>?key=xxx&expires=xxx
        """
        parsed = urlparse(url)
        parts = parsed.path.strip("/").split("/")

        if len(parts) < 4 or parts[0] != "mods" or parts[2] != "files":
            raise ValueError(f"Invalid NXM URL format: {url}")

        params = parse_qs(parsed.query)

        return cls(
            game_slug=parsed.netloc,
            mod_id=int(parts[1]),
            file_id=int(parts[3]),
            key=params.get("key", [None])[0],
            expires=params.get("expires", [None])[0],
        )


class NexusClient:
    """Client for the Nexus Mods API."""

    def __init__(self, api_key: str):
        self.api_key = api_key
        self._client = httpx.Client(
            base_url=NEXUS_API_BASE,
            headers={
                "apikey": api_key,
                "accept": "application/json",
                "User-Agent": "Corkscrew/0.1.0",
            },
            timeout=30.0,
        )

    def close(self):
        self._client.close()

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.close()

    def validate_key(self) -> dict[str, Any]:
        """Validate the API key and return user info."""
        resp = self._client.get("/users/validate.json")
        resp.raise_for_status()
        return resp.json()

    def get_mod(self, game_slug: str, mod_id: int) -> dict[str, Any]:
        """Get mod metadata."""
        resp = self._client.get(f"/games/{game_slug}/mods/{mod_id}.json")
        resp.raise_for_status()
        return resp.json()

    def get_mod_files(self, game_slug: str, mod_id: int) -> list[dict[str, Any]]:
        """List files available for a mod."""
        resp = self._client.get(f"/games/{game_slug}/mods/{mod_id}/files.json")
        resp.raise_for_status()
        return resp.json().get("files", [])

    def get_download_links(
        self,
        game_slug: str,
        mod_id: int,
        file_id: int,
        key: str | None = None,
        expires: str | None = None,
    ) -> list[dict[str, str]]:
        """Get download URLs for a specific file.

        For non-premium users, key and expires from the NXM link are required.
        """
        params = {}
        if key:
            params["key"] = key
        if expires:
            params["expires"] = expires

        resp = self._client.get(
            f"/games/{game_slug}/mods/{mod_id}/files/{file_id}/download_link.json",
            params=params,
        )
        resp.raise_for_status()
        return resp.json()

    def download_file(
        self,
        download_url: str,
        dest: Path,
        progress_callback=None,
    ) -> Path:
        """Download a file from a Nexus download URL."""
        with httpx.stream("GET", download_url, follow_redirects=True, timeout=300.0) as resp:
            resp.raise_for_status()
            total = int(resp.headers.get("content-length", 0))
            downloaded = 0

            with open(dest, "wb") as f:
                for chunk in resp.iter_bytes(chunk_size=65536):
                    f.write(chunk)
                    downloaded += len(chunk)
                    if progress_callback and total > 0:
                        progress_callback(downloaded, total)

        return dest

    def download_from_nxm(
        self,
        nxm: NXMLink,
        download_dir: Path,
        progress_callback=None,
    ) -> Path:
        """Download a mod file from an NXM link."""
        links = self.get_download_links(
            nxm.game_slug, nxm.mod_id, nxm.file_id, nxm.key, nxm.expires
        )
        if not links:
            raise RuntimeError("No download links returned from Nexus API")

        download_url = links[0]["URI"]

        # Get the filename from mod files list
        files = self.get_mod_files(nxm.game_slug, nxm.mod_id)
        filename = None
        for f in files:
            if f.get("file_id") == nxm.file_id:
                filename = f.get("file_name", f"mod_{nxm.mod_id}_{nxm.file_id}")
                break

        if not filename:
            filename = f"mod_{nxm.mod_id}_{nxm.file_id}.7z"

        dest = download_dir / filename
        download_dir.mkdir(parents=True, exist_ok=True)

        return self.download_file(download_url, dest, progress_callback)
