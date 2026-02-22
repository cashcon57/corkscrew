#!/usr/bin/env python3
"""Fetch NexusMods game stats and update the README game support table.

Usage: NEXUS_API_KEY=xxx python3 scripts/update-game-stats.py

Fetches mod counts from the NexusMods API (/v1/games), cross-references
with our game registry, and regenerates the game support section in README.md.
"""

import json
import os
import re
import sys
import urllib.request
from pathlib import Path

ROOT = Path(__file__).parent.parent
REGISTRY_PATH = ROOT / "src-tauri" / "data" / "vortex_game_registry.json"
README_PATH = ROOT / "README.md"

NEXUS_API_BASE = "https://api.nexusmods.com"


def fetch_nexus_games(api_key: str) -> list[dict]:
    """Fetch all games from NexusMods API."""
    url = f"{NEXUS_API_BASE}/v1/games"
    req = urllib.request.Request(url, headers={
        "apikey": api_key,
        "accept": "application/json",
    })
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.loads(resp.read())


def load_registry() -> list[dict]:
    """Load our game registry JSON."""
    with open(REGISTRY_PATH) as f:
        return json.load(f)


def build_game_table(registry: list[dict], nexus_games: list[dict]) -> str:
    """Build a markdown table of supported games with mod counts."""
    # Build lookup: nexus_domain -> game stats
    nexus_lookup = {}
    for game in nexus_games:
        domain = game.get("domain_name", "")
        nexus_lookup[domain] = {
            "name": game.get("name", ""),
            "mods": game.get("mods", 0),
            "downloads": game.get("downloads", 0),
            "id": game.get("id", 0),
        }

    # Match registry entries to NexusMods stats
    rows = []
    for entry in registry:
        if entry.get("_note"):
            continue  # Skip stubs
        if not entry.get("executable"):
            continue

        domain = entry["nexus_domain"]
        stats = nexus_lookup.get(domain, {})
        mod_count = stats.get("mods", 0)
        downloads = stats.get("downloads", 0)

        has_tools = "Yes" if entry.get("tools") else ""
        store_ids = []
        if entry.get("steam_id"):
            store_ids.append("Steam")
        if entry.get("gog_id"):
            store_ids.append("GOG")
        if entry.get("epic_id"):
            store_ids.append("Epic")
        stores = ", ".join(store_ids) if store_ids else "-"

        rows.append({
            "name": entry["name"],
            "domain": domain,
            "mods": mod_count,
            "downloads": downloads,
            "tools": has_tools,
            "stores": stores,
            "game_id": entry["game_id"],
        })

    # Sort by mod count descending
    rows.sort(key=lambda r: r["mods"], reverse=True)

    # Build markdown
    lines = []
    lines.append("| # | Game | NexusMods Domain | Mods | Tools | Stores |")
    lines.append("|---|------|-----------------|------|-------|--------|")

    for i, row in enumerate(rows, 1):
        mod_str = f"{row['mods']:,}" if row["mods"] > 0 else "-"
        lines.append(
            f"| {i} | {row['name']} | "
            f"[{row['domain']}](https://www.nexusmods.com/{row['domain']}) | "
            f"{mod_str} | {row['tools']} | {row['stores']} |"
        )

    return "\n".join(lines)


def update_readme(table: str, total_games: int) -> bool:
    """Update the game support section in README.md. Returns True if changed."""
    readme = README_PATH.read_text()

    # Markers for the auto-generated section
    start_marker = "<!-- GAME_SUPPORT_TABLE_START -->"
    end_marker = "<!-- GAME_SUPPORT_TABLE_END -->"

    if start_marker not in readme:
        print("Warning: Game support markers not found in README.md", file=sys.stderr)
        print("Add these markers to README.md where you want the table:", file=sys.stderr)
        print(f"  {start_marker}", file=sys.stderr)
        print(f"  {end_marker}", file=sys.stderr)
        return False

    header = f"**{total_games} games supported** — auto-updated daily from NexusMods API\n\n"
    new_section = f"{start_marker}\n{header}{table}\n{end_marker}"

    pattern = re.compile(
        re.escape(start_marker) + r".*?" + re.escape(end_marker),
        re.DOTALL,
    )
    new_readme = pattern.sub(new_section, readme)

    if new_readme == readme:
        return False

    README_PATH.write_text(new_readme)
    return True


def main():
    api_key = os.environ.get("NEXUS_API_KEY", "")
    if not api_key:
        print("Error: NEXUS_API_KEY environment variable not set", file=sys.stderr)
        sys.exit(1)

    print("Fetching NexusMods game list...")
    nexus_games = fetch_nexus_games(api_key)
    print(f"  Found {len(nexus_games)} games on NexusMods")

    registry = load_registry()
    active_games = [e for e in registry if e.get("executable") and not e.get("_note")]
    print(f"  {len(active_games)} games in our registry")

    table = build_game_table(registry, nexus_games)
    changed = update_readme(table, len(active_games))

    if changed:
        print("README.md updated with new game stats")
    else:
        print("No changes to README.md")


if __name__ == "__main__":
    main()
