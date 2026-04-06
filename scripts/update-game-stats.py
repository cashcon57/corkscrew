#!/usr/bin/env python3
"""Fetch NexusMods game stats, update README table, and generate support report.

Usage: NEXUS_API_KEY=xxx python3 scripts/update-game-stats.py

- Fetches mod counts from the NexusMods API (/v1/games)
- Cross-references with our game registry + dedicated plugins
- Regenerates the game support table in README.md
- Generates GAME_SUPPORT_REPORT.md with gap analysis and priority recommendations
- Appends to stats/game-stats-history.json for trend tracking
"""

import json
import os
import re
import sys
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).parent.parent
REGISTRY_PATH = ROOT / "src-tauri" / "data" / "vortex_game_registry.json"
README_PATH = ROOT / "README.md"
REPORT_PATH = ROOT / "GAME_SUPPORT_REPORT.md"
HISTORY_PATH = ROOT / "stats" / "game-stats-history.json"

NEXUS_API_BASE = "https://api.nexusmods.com"

# Games with dedicated Rust plugins (beyond generic registry support)
DEDICATED_PLUGINS = {
    "skyrimse": "Full — LOOT, SKSE, crash logs, Engine Fixes for Wine, Wabbajack",
    "fallout4": "Full — LOOT, dedicated plugin",
    "hogwartslegacy": "Enhanced — UE4SS auto-install, PAK merger, Lua routing, deploy hooks",
}

# Games with load order support
LOAD_ORDER_GAMES = {"skyrimse", "fallout4"}

# Script extender status per game
SCRIPT_EXTENDERS = {
    "skyrimse": ("SKSE", True),     # (name, auto-install)
    "fallout4": ("F4SE", False),
    "falloutnv": ("NVSE", False),
    "oblivion": ("OBSE", False),
    "fallout3": ("FOSE", False),
    "morrowind": ("MWSE", False),
    "starfield": ("SFSE", False),
}

# Known mod framework requirements for non-registry games
FRAMEWORK_NOTES = {
    "valheim": "BepInEx",
    "palworld": "UE4SS / BepInEx",
    "subnautica": "BepInEx / QMods",
    "stardewvalley": "SMAPI",
    "eldenring": "ModEngine2, EAC considerations",
    "cyberpunk2077": "REDmod / red4ext",
    "baldursgate3": "BG3 Mod Manager, PAK format",
    "monsterhunterworld": "Stracker's Loader",
    "monsterhunterrise": "REFramework",
    "residentevil42023": "REFramework / Fluffy Manager",
    "residentevil22019": "REFramework",
    "devilmaycry5": "REFramework",
    "streetfighter6": "REFramework",
    "reddeadredemption2": "Script Hook",
    "mysummercar": "MSCLoader",
    "mywintercar": "MSCLoader",
    "dragonageinquisition": "Frosty Mod Manager",
    "dragonagetheveilguard": "Frosty Mod Manager",
    "starwarsbattlefront22017": "Frosty Mod Manager",
    "mountandblade2bannerlord": "Module system",
    "darksouls3": "ModEngine2",
    "darksouls": "DSFix / DSDPT",
    "witcher3": "Script Merger",
}

# Wine/CrossOver compatibility tiers
WINE_COMPAT = {
    "excellent": [
        "skyrimse", "skyrim", "fallout4", "falloutnv", "oblivion", "fallout3",
        "morrowind", "witcher3", "darksouls", "darksouls2", "darksouls3",
        "kingdomcomedeliverance", "stardewvalley",
    ],
    "good": [
        "cyberpunk2077", "eldenring", "hogwartslegacy", "baldursgate3",
        "residentevil42023", "residentevil22019", "monsterhunterworld",
        "nomanssky", "sekiro", "starfield",
    ],
    "problematic": [
        "helldivers2", "fallout76", "warthunder", "finalfantasy14",
    ],
}


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


def classify_game(entry: dict, nexus_stats: dict) -> dict:
    """Classify a registry game's support level."""
    gid = entry["game_id"]
    domain = entry["nexus_domain"]
    stats = nexus_stats.get(domain, {})

    has_plugin = gid in DEDICATED_PLUGINS
    has_load_order = gid in LOAD_ORDER_GAMES
    se_name, se_auto = SCRIPT_EXTENDERS.get(gid, (None, False))
    framework = FRAMEWORK_NOTES.get(gid, FRAMEWORK_NOTES.get(domain, ""))

    # Determine support tier
    if has_plugin and has_load_order:
        tier = "Full"
    elif has_plugin:
        tier = "Enhanced"
    else:
        tier = "Standard"

    # Wine compat
    wine = "Unknown"
    for level, games in WINE_COMPAT.items():
        if gid in games or domain in games:
            wine = level.capitalize()
            break

    return {
        "game_id": gid,
        "name": entry["name"],
        "domain": domain,
        "mods": stats.get("mods", 0),
        "downloads": stats.get("downloads", 0),
        "tier": tier,
        "has_plugin": has_plugin,
        "plugin_notes": DEDICATED_PLUGINS.get(gid, ""),
        "has_load_order": has_load_order,
        "script_extender": se_name,
        "se_auto_install": se_auto,
        "framework": framework,
        "has_tools": bool(entry.get("tools")),
        "steam_id": entry.get("steam_id"),
        "wine_compat": wine,
        "stores": _get_stores(entry),
    }


def _get_stores(entry: dict) -> str:
    stores = []
    if entry.get("steam_id"):
        stores.append("Steam")
    if entry.get("gog_id"):
        stores.append("GOG")
    if entry.get("epic_id"):
        stores.append("Epic")
    return ", ".join(stores) if stores else "-"


def find_gaps(registry: list[dict], nexus_games: list[dict]) -> list[dict]:
    """Find top NexusMods games NOT in our registry."""
    registry_domains = {e["nexus_domain"] for e in registry if e.get("executable")}

    gaps = []
    for game in nexus_games:
        domain = game.get("domain_name", "")
        if domain in registry_domains:
            continue
        mods = game.get("mods", 0)
        if mods < 500:  # Skip very small games
            continue
        framework = FRAMEWORK_NOTES.get(domain, "")
        gaps.append({
            "name": game.get("name", "Unknown"),
            "domain": domain,
            "mods": mods,
            "downloads": game.get("downloads", 0),
            "framework": framework,
            "nexus_id": game.get("id", 0),
        })

    gaps.sort(key=lambda g: g["mods"], reverse=True)
    return gaps


def build_readme_table(registry: list[dict], nexus_lookup: dict) -> str:
    """Build the README game support table."""
    rows = []
    for entry in registry:
        if entry.get("_note") or not entry.get("executable"):
            continue
        domain = entry["nexus_domain"]
        stats = nexus_lookup.get(domain, {})
        mod_count = stats.get("mods", 0)
        rows.append({
            "name": entry["name"],
            "domain": domain,
            "mods": mod_count,
            "tools": "Yes" if entry.get("tools") else "",
            "stores": _get_stores(entry),
            "game_id": entry["game_id"],
        })

    rows.sort(key=lambda r: r["mods"], reverse=True)

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
    """Update the game support section in README.md."""
    readme = README_PATH.read_text()
    start_marker = "<!-- GAME_SUPPORT_TABLE_START -->"
    end_marker = "<!-- GAME_SUPPORT_TABLE_END -->"

    if start_marker not in readme:
        print("Warning: Game support markers not found in README.md", file=sys.stderr)
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


def generate_report(
    supported: list[dict],
    gaps: list[dict],
    total_nexus: int,
) -> str:
    """Generate GAME_SUPPORT_REPORT.md with full analysis."""
    now = datetime.now(timezone.utc).strftime("%B %d, %Y")

    full_tier = [g for g in supported if g["tier"] == "Full"]
    enhanced_tier = [g for g in supported if g["tier"] == "Enhanced"]
    standard_tier = [g for g in supported if g["tier"] == "Standard"]

    total_supported_mods = sum(g["mods"] for g in supported)
    total_gap_mods = sum(g["mods"] for g in gaps)

    lines = []
    lines.append("# Corkscrew — Game Support & Compatibility Report")
    lines.append("")
    lines.append(f"**Auto-generated:** {now}")
    lines.append(f"**Source:** NexusMods API (`/v1/games`) — {total_nexus:,} games indexed")
    lines.append("")
    lines.append("---")
    lines.append("")

    # Executive summary
    lines.append("## Executive Summary")
    lines.append("")
    lines.append(f"- **{len(supported)} games supported** in registry")
    lines.append(f"  - {len(full_tier)} Full (load order, LOOT, script extenders, tools)")
    lines.append(f"  - {len(enhanced_tier)} Enhanced (dedicated plugin with game-specific features)")
    lines.append(f"  - {len(standard_tier)} Standard (auto-detect, install, deploy, collections)")
    lines.append(f"- **{len(gaps)} high-potential games** NOT in registry (500+ mods on NexusMods)")
    lines.append(f"- **{total_supported_mods:,} total mods** covered by supported games")
    lines.append(f"- **{total_gap_mods:,} total mods** in unsupported gap games")
    lines.append("")
    lines.append("---")
    lines.append("")

    # Full support tier
    lines.append("## Full Support")
    lines.append("")
    lines.append("| Game | Mods | Load Order | Script Ext | Tools | Wine | Notes |")
    lines.append("|------|------|------------|------------|-------|------|-------|")
    for g in sorted(full_tier, key=lambda x: x["mods"], reverse=True):
        se = f"{'✅' if g['se_auto_install'] else '🟡'} {g['script_extender']}" if g["script_extender"] else "—"
        lines.append(
            f"| {g['name']} | {g['mods']:,} | ✅ LOOT | {se} | "
            f"{'✅' if g['has_tools'] else '—'} | {g['wine_compat']} | {g['plugin_notes']} |"
        )
    lines.append("")

    # Enhanced support tier
    if enhanced_tier:
        lines.append("## Enhanced Support")
        lines.append("")
        lines.append("| Game | Mods | Plugin Features | Wine |")
        lines.append("|------|------|----------------|------|")
        for g in sorted(enhanced_tier, key=lambda x: x["mods"], reverse=True):
            lines.append(f"| {g['name']} | {g['mods']:,} | {g['plugin_notes']} | {g['wine_compat']} |")
        lines.append("")

    # Standard support tier
    lines.append("## Standard Support (Generic Pipeline)")
    lines.append("")
    lines.append("These games are auto-detected and use the generic mod install/deploy pipeline.")
    lines.append("Collections work via NexusMods `game_domain`.")
    lines.append("")
    lines.append("| # | Game | Mods | Script Ext | Framework Needed | Wine | Stores |")
    lines.append("|---|------|------|------------|-----------------|------|--------|")
    for i, g in enumerate(sorted(standard_tier, key=lambda x: x["mods"], reverse=True), 1):
        se = g["script_extender"] or "—"
        fw = g["framework"] or "—"
        lines.append(
            f"| {i} | {g['name']} | {g['mods']:,} | {se} | {fw} | {g['wine_compat']} | {g['stores']} |"
        )
    lines.append("")

    # Gap analysis
    lines.append("---")
    lines.append("")
    lines.append("## Gap Analysis — Unsupported Games by Impact")
    lines.append("")
    lines.append("These games are in the NexusMods top tier but have **no Corkscrew registry entry**.")
    lines.append("Adding a registry entry enables: detection, mod install, deployment, collections, profiles.")
    lines.append("")
    lines.append("| # | Game | Mods | Domain | Framework | Effort | Priority |")
    lines.append("|---|------|------|--------|-----------|--------|----------|")
    for i, g in enumerate(gaps[:40], 1):
        effort = "Medium" if g["framework"] else "Low"
        # Priority based on mod count
        if g["mods"] >= 5000:
            priority = "🔴 High"
        elif g["mods"] >= 2000:
            priority = "🟡 Medium"
        else:
            priority = "🟢 Low"
        fw = g["framework"] or "File replacement"
        lines.append(
            f"| {i} | {g['name']} | {g['mods']:,} | "
            f"[{g['domain']}](https://www.nexusmods.com/{g['domain']}) | "
            f"{fw} | {effort} | {priority} |"
        )
    lines.append("")

    # Priority recommendations
    lines.append("---")
    lines.append("")
    lines.append("## Priority Recommendations")
    lines.append("")

    high_impact_gaps = [g for g in gaps if g["mods"] >= 3000]
    low_effort_gaps = [g for g in gaps if not g["framework"] and g["mods"] >= 1000]

    lines.append("### Highest Impact (3,000+ mods, not yet supported)")
    lines.append("")
    if high_impact_gaps:
        for g in high_impact_gaps:
            fw = f" — requires {g['framework']}" if g["framework"] else ""
            lines.append(f"- **{g['name']}** ({g['mods']:,} mods){fw}")
    else:
        lines.append("All high-impact games are supported! 🎉")
    lines.append("")

    lines.append("### Quick Wins (1,000+ mods, simple file replacement, no framework needed)")
    lines.append("")
    if low_effort_gaps:
        for g in low_effort_gaps[:15]:
            lines.append(f"- **{g['name']}** ({g['mods']:,} mods)")
    else:
        lines.append("All quick-win games are supported! 🎉")
    lines.append("")

    # Script extender status
    lines.append("---")
    lines.append("")
    lines.append("## Script Extender Status")
    lines.append("")
    lines.append("| Game | Extender | Auto-Install | Status |")
    lines.append("|------|----------|-------------|--------|")
    for gid, (se_name, auto) in sorted(SCRIPT_EXTENDERS.items()):
        game_name = next((g["name"] for g in supported if g["game_id"] == gid), gid)
        status = "✅ Implemented" if auto else "❌ Manual only"
        lines.append(f"| {game_name} | {se_name} | {'✅' if auto else '❌'} | {status} |")
    lines.append("")

    # Wine compat summary
    lines.append("---")
    lines.append("")
    lines.append("## Wine/CrossOver Compatibility")
    lines.append("")
    for level in ["excellent", "good", "problematic"]:
        emoji = {"excellent": "🟢", "good": "🟡", "problematic": "🔴"}[level]
        game_ids = WINE_COMPAT[level]
        game_names = []
        for gid in game_ids:
            name = next((g["name"] for g in supported if g["game_id"] == gid), gid)
            game_names.append(name)
        lines.append(f"**{emoji} {level.capitalize()}:** {', '.join(game_names)}")
        lines.append("")

    # Footer
    lines.append("---")
    lines.append("")
    lines.append("*This report is auto-generated daily by `scripts/update-game-stats.py`.*")
    lines.append("*Data source: NexusMods API. Registry: `src-tauri/data/vortex_game_registry.json`.*")

    return "\n".join(lines)


def save_history(supported: list[dict], gaps: list[dict]):
    """Append today's stats to the history file for trend tracking."""
    HISTORY_PATH.parent.mkdir(parents=True, exist_ok=True)

    history = []
    if HISTORY_PATH.exists():
        try:
            history = json.loads(HISTORY_PATH.read_text())
        except (json.JSONDecodeError, ValueError):
            history = []

    today = datetime.now(timezone.utc).strftime("%Y-%m-%d")

    # Don't duplicate today's entry
    if history and history[-1].get("date") == today:
        history.pop()

    entry = {
        "date": today,
        "supported_count": len(supported),
        "gap_count": len(gaps),
        "total_supported_mods": sum(g["mods"] for g in supported),
        "total_gap_mods": sum(g["mods"] for g in gaps),
        "top_gaps": [{"name": g["name"], "domain": g["domain"], "mods": g["mods"]} for g in gaps[:10]],
        "top_supported": [{"name": g["name"], "mods": g["mods"], "tier": g["tier"]} for g in
                          sorted(supported, key=lambda x: x["mods"], reverse=True)[:10]],
    }

    history.append(entry)

    # Keep last 365 days
    history = history[-365:]

    HISTORY_PATH.write_text(json.dumps(history, indent=2) + "\n")


def main():
    api_key = os.environ.get("NEXUS_API_KEY", "")
    if not api_key:
        print("Error: NEXUS_API_KEY environment variable not set", file=sys.stderr)
        sys.exit(1)

    print("Fetching NexusMods game list...")
    nexus_games = fetch_nexus_games(api_key)
    print(f"  Found {len(nexus_games)} games on NexusMods")

    # Build lookup
    nexus_lookup = {}
    for game in nexus_games:
        domain = game.get("domain_name", "")
        nexus_lookup[domain] = {
            "name": game.get("name", ""),
            "mods": game.get("mods", 0),
            "downloads": game.get("downloads", 0),
            "id": game.get("id", 0),
        }

    registry = load_registry()
    active = [e for e in registry if e.get("executable") and not e.get("_note")]
    print(f"  {len(active)} games in our registry")

    # Classify supported games
    supported = [classify_game(e, nexus_lookup) for e in active]

    # Find gaps
    gaps = find_gaps(registry, nexus_games)
    print(f"  {len(gaps)} unsupported games with 500+ mods")

    # 1. Update README table
    table = build_readme_table(registry, nexus_lookup)
    readme_changed = update_readme(table, len(active))
    if readme_changed:
        print("✓ README.md updated with new game stats")
    else:
        print("  README.md unchanged")

    # 2. Generate support report
    report = generate_report(supported, gaps, len(nexus_games))
    old_report = REPORT_PATH.read_text() if REPORT_PATH.exists() else ""
    if report != old_report:
        REPORT_PATH.write_text(report)
        print("✓ GAME_SUPPORT_REPORT.md regenerated")
    else:
        print("  GAME_SUPPORT_REPORT.md unchanged")

    # 3. Save history for trends
    save_history(supported, gaps)
    print("✓ Stats history updated")

    # Print quick summary
    print()
    print("=== Quick Summary ===")
    high_gaps = [g for g in gaps if g["mods"] >= 3000]
    if high_gaps:
        print(f"  {len(high_gaps)} high-impact games NOT supported:")
        for g in high_gaps[:5]:
            print(f"    - {g['name']} ({g['mods']:,} mods)")
    else:
        print("  All high-impact games are covered!")


if __name__ == "__main__":
    main()
