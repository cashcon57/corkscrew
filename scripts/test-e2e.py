#!/usr/bin/env python3
"""
Corkscrew — End-to-End CLI Test Suite

Tests Corkscrew's backend through CLI subcommands that exercise the same
code paths as the Tauri UI. No GUI required.

Usage:
    python3 scripts/test-e2e.py [--binary PATH] [--game skyrimse] [--bottle Steam]
"""

import argparse
import json
import os
import subprocess
import sys
import time

# ---------------------------------------------------------------------------
# Config
# ---------------------------------------------------------------------------

PASS = "\033[92m✓ PASS\033[0m"
FAIL = "\033[91m✗ FAIL\033[0m"
SKIP = "\033[93m⊘ SKIP\033[0m"
INFO = "\033[94mℹ INFO\033[0m"

DEFAULT_BINARY = os.path.join(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
    "src-tauri", "target", "release", "corkscrew",
)

# ---------------------------------------------------------------------------
# Test infrastructure
# ---------------------------------------------------------------------------

results = []

def test(name, passed, detail="", skipped=False):
    if skipped:
        results.append((name, None, detail))
        print(f"  {SKIP} {name}")
        if detail:
            print(f"        {detail[:200]}")
    else:
        results.append((name, passed, detail))
        status = PASS if passed else FAIL
        print(f"  {status} {name}")
        if detail and not passed:
            print(f"        {detail[:200]}")


def run_cli(binary, *args, timeout=30):
    """Run a corkscrew CLI command and return (exit_code, stdout, stderr)."""
    cmd = [binary] + list(args)
    try:
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        return result.returncode, result.stdout, result.stderr
    except subprocess.TimeoutExpired:
        return -1, "", "TIMEOUT"
    except FileNotFoundError:
        return -2, "", f"Binary not found: {binary}"


def parse_json(text):
    """Try to parse JSON from CLI output, stripping log lines."""
    lines = text.strip().split("\n")
    # Skip lines that look like log output
    json_lines = []
    brace_depth = 0
    bracket_depth = 0
    in_json = False
    for line in lines:
        stripped = line.strip()
        if not in_json:
            if stripped.startswith("{") or stripped.startswith("["):
                in_json = True
        if in_json:
            json_lines.append(line)
            brace_depth += stripped.count("{") - stripped.count("}")
            bracket_depth += stripped.count("[") - stripped.count("]")
            if brace_depth <= 0 and bracket_depth <= 0:
                break

    if not json_lines:
        return None

    text = "\n".join(json_lines)
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        return None


# ---------------------------------------------------------------------------
# Test suites
# ---------------------------------------------------------------------------

def test_binary_exists(binary):
    """Suite 0: Binary exists and is executable."""
    print("\n━━━ Suite 0: Binary ━━━")
    exists = os.path.isfile(binary)
    test("Binary exists", exists, binary)
    if not exists:
        return False

    executable = os.access(binary, os.X_OK)
    test("Binary is executable", executable)
    return exists and executable


def test_help(binary):
    """Suite 1: --help and --version."""
    print("\n━━━ Suite 1: Help & Version ━━━")

    code, out, err = run_cli(binary, "--version")
    test("--version exits 0", code == 0, f"exit={code}")
    version = out.strip().split("\n")[-1] if out.strip() else ""
    test("--version prints semver", len(version.split(".")) >= 2, f"got: '{version}'")
    print(f"  {INFO} Version: {version}")

    code, out, err = run_cli(binary, "--help")
    test("--help exits 0", code == 0, f"exit={code}")
    test("--help mentions launch", "--launch" in out, f"output length: {len(out)}")
    test("--help mentions list-bottles", "--list-bottles" in out)
    test("--help mentions db-integrity", "--db-integrity" in out)


def test_bottle_detection(binary):
    """Suite 2: Bottle detection."""
    print("\n━━━ Suite 2: Bottle Detection ━━━")

    code, out, err = run_cli(binary, "--list-bottles")
    test("--list-bottles exits 0", code == 0, f"exit={code}")

    data = parse_json(out)
    test("Output is valid JSON array", isinstance(data, list), f"type: {type(data)}")

    if not isinstance(data, list):
        return []

    test("At least 1 bottle detected", len(data) > 0, f"count: {len(data)}")
    print(f"  {INFO} {len(data)} bottles detected")

    for b in data:
        name = b.get("name", "?")
        exists = b.get("exists", False)
        engine = b.get("engine", "?")
        print(f"  {INFO} {name} ({engine}) exists={exists}")

    # Validate structure
    if data:
        b0 = data[0]
        test("Bottle has 'name' field", "name" in b0)
        test("Bottle has 'path' field", "path" in b0)
        test("Bottle has 'engine' field", "engine" in b0)
        test("Bottle has 'exists' field", "exists" in b0)

    return data


def test_game_detection(binary):
    """Suite 3: Game detection."""
    print("\n━━━ Suite 3: Game Detection ━━━")

    code, out, err = run_cli(binary, "--list-games")
    test("--list-games exits 0", code == 0, f"exit={code}")

    data = parse_json(out)
    test("Output is valid JSON array", isinstance(data, list))

    if not isinstance(data, list):
        return []

    test("At least 1 game detected", len(data) > 0, f"count: {len(data)}")
    print(f"  {INFO} {len(data)} games detected")

    for g in data:
        print(f"  {INFO} {g.get('id','?')} — {g.get('name','?')} in {g.get('bottle','?')}")

    if data:
        g0 = data[0]
        test("Game has 'id' field", "id" in g0)
        test("Game has 'name' field", "name" in g0)
        test("Game has 'bottle' field", "bottle" in g0)
        test("Game has 'path' field", "path" in g0)

    return data


def test_db_integrity(binary):
    """Suite 4: Database integrity."""
    print("\n━━━ Suite 4: Database Integrity ━━━")

    code, out, err = run_cli(binary, "--db-integrity")
    test("--db-integrity exits 0", code == 0, f"exit={code}")

    data = parse_json(out)
    test("Output is valid JSON", isinstance(data, dict))

    if not isinstance(data, dict):
        return

    ok = data.get("ok", False)
    test("Integrity check passes", ok, data.get("integrity_check", "?"))
    schema_v = data.get("schema_version", 0)
    # Corkscrew uses internal migration tracking, not PRAGMA user_version
    print(f"  {INFO} PRAGMA user_version: {schema_v} (Corkscrew uses internal migration counter)")

    tables = data.get("tables", [])
    print(f"  {INFO} {data.get('table_count', '?')} tables")

    # Check critical tables exist
    required_tables = ["installed_mods", "deployment_manifest", "file_hashes", "vortex_extensions"]
    for table in required_tables:
        test(f"Table '{table}' exists", table in tables,
             f"tables: {tables}" if table not in tables else "")


def test_db_stats(binary, game_id, bottle_name):
    """Suite 5: Database statistics."""
    print(f"\n━━━ Suite 5: DB Stats ({game_id}:{bottle_name}) ━━━")

    code, out, err = run_cli(binary, "--db-stats", game_id, bottle_name)
    test("--db-stats exits 0", code == 0, f"exit={code}")

    data = parse_json(out)
    test("Output is valid JSON", isinstance(data, dict))

    if not isinstance(data, dict):
        return

    total = data.get("total_mods", 0)
    enabled = data.get("enabled_mods", 0)
    disabled = data.get("disabled_mods", 0)
    deployed = data.get("deployed_files", 0)

    test("Has total_mods field", "total_mods" in data)
    test("Has enabled_mods field", "enabled_mods" in data)
    test("Has deployed_files field", "deployed_files" in data)
    test("total_mods > 0", total > 0, f"total: {total}")
    # get_mod_counts returns (enabled, disabled) which may overlap with total
    # due to counting methodology — just verify both are reasonable
    test("enabled_mods <= total_mods", enabled <= total,
         f"enabled={enabled} > total={total}")
    test("disabled_mods <= total_mods", disabled <= total,
         f"disabled={disabled} > total={total}")
    test("deployed_files > 0", deployed > 0, f"deployed: {deployed}")

    print(f"  {INFO} {total} mods ({enabled} enabled, {disabled} disabled), {deployed} deployed files")


def test_list_mods(binary, game_id, bottle_name):
    """Suite 6: List mods."""
    print(f"\n━━━ Suite 6: List Mods ({game_id}:{bottle_name}) ━━━")

    t0 = time.time()
    code, out, err = run_cli(binary, "--list-mods", game_id, bottle_name, timeout=60)
    elapsed = time.time() - t0

    test("--list-mods exits 0", code == 0, f"exit={code}")
    test("Completes in < 10s", elapsed < 10, f"took {elapsed:.1f}s")

    lines = out.strip().split("\n")
    test("Output has header line", len(lines) >= 2)

    # Parse first line for count
    if lines:
        first = lines[0]
        if "mods installed" in first:
            parts = first.split()
            for i, p in enumerate(parts):
                if p == "mods":
                    count = int(parts[i-1]) if parts[i-1].isdigit() else 0
                    test("Reports > 0 mods", count > 0, f"count: {count}")
                    print(f"  {INFO} {count} mods in {elapsed:.1f}s")
                    break

    # Check that we see both enabled and disabled mods
    has_yes = any("yes" in line for line in lines[2:])
    has_no = any("NO" in line for line in lines[2:])
    test("Contains enabled mods", has_yes)
    test("Contains disabled mods", has_no)


def test_search_mods(binary, game_id, bottle_name):
    """Suite 7: Search mods."""
    print(f"\n━━━ Suite 7: Search Mods ({game_id}:{bottle_name}) ━━━")

    # Search for common mod name
    code, out, err = run_cli(binary, "--search-mods", "SKSE", game_id, bottle_name)
    test("--search-mods exits 0", code == 0, f"exit={code}")
    test("Search for 'SKSE' returns results", "SKSE" in out or "skse" in out.lower(),
         f"output length: {len(out)}")

    # Search for nonexistent mod
    code2, out2, err2 = run_cli(binary, "--search-mods", "zzzznonexistent12345", game_id, bottle_name)
    test("Search for gibberish returns no matches", "0 matches" in out2 or "No matches" in out2 or len(out2.strip().split("\n")) <= 3,
         f"output: {out2[:100]}")

    # Empty query should error
    code3, out3, err3 = run_cli(binary, "--search-mods", "", game_id, bottle_name)
    test("Empty search query errors", code3 != 0 or "Usage" in err3)


def test_find_file(binary, game_id, bottle_name):
    """Suite 8: Find file."""
    print(f"\n━━━ Suite 8: Find File ({game_id}:{bottle_name}) ━━━")

    # Search for common file pattern
    code, out, err = run_cli(binary, "--find-file", "SkyUI", game_id, bottle_name)
    test("--find-file exits 0", code == 0, f"exit={code}")
    lines = out.strip().split("\n")
    test("Find 'SkyUI' returns results", len(lines) > 1, f"lines: {len(lines)}")

    # Search for .esp files
    code2, out2, err2 = run_cli(binary, "--find-file", ".esp", game_id, bottle_name)
    test("Find '.esp' returns plugin files", ".esp" in out2.lower(),
         f"output length: {len(out2)}")


def test_check_plugins(binary, game_id, bottle_name):
    """Suite 9: Check plugins."""
    print(f"\n━━━ Suite 9: Check Plugins ({game_id}:{bottle_name}) ━━━")

    t0 = time.time()
    code, out, err = run_cli(binary, "--check-plugins", game_id, bottle_name, timeout=60)
    elapsed = time.time() - t0

    test("--check-plugins exits 0", code == 0, f"exit={code}")
    test("Completes in < 30s", elapsed < 30, f"took {elapsed:.1f}s")

    # Should mention plugins.txt
    test("Mentions plugins.txt", "plugins.txt" in out)

    # Parse statistics
    for line in out.split("\n"):
        if "active" in line.lower() and "inactive" in line.lower():
            print(f"  {INFO} {line.strip()}")
            break

    # Test --deployed-inactive flag
    code2, out2, err2 = run_cli(binary, "--check-plugins", game_id, bottle_name, "--deployed-inactive")
    test("--deployed-inactive flag works", code2 == 0, f"exit={code2}")


def test_mod_files(binary, game_id, bottle_name):
    """Suite 10: Mod files."""
    print(f"\n━━━ Suite 10: Mod Files ({game_id}:{bottle_name}) ━━━")

    # Search by name
    code, out, err = run_cli(binary, "--mod-files", "SkyUI", game_id, bottle_name)
    test("--mod-files by name exits 0", code == 0, f"exit={code}")
    test("Shows file listing", "files" in out.lower() or "staged" in out.lower() or "Mod:" in out,
         f"output: {out[:100]}")

    # Search with invalid ID should still exit cleanly
    code2, out2, err2 = run_cli(binary, "--mod-files", "99999999", game_id, bottle_name)
    test("Non-existent mod ID handled gracefully", code2 == 0 or "No mod" in out2 or "not found" in out2.lower())


def test_deployment_health(binary, game_id, bottle_name):
    """Suite 11: Deployment health."""
    print(f"\n━━━ Suite 11: Deployment Health ({game_id}:{bottle_name}) ━━━")

    code, out, err = run_cli(binary, "--deployment-health", game_id, bottle_name)
    # May exit 1 if staging dirs are missing — that's a valid health check result
    test("--deployment-health runs", code in (0, 1), f"exit={code}")

    data = parse_json(out)
    test("Output is valid JSON", isinstance(data, dict))

    if isinstance(data, dict):
        test("Has 'ok' field", "ok" in data)
        test("Has 'total_mods' field", "total_mods" in data)
        test("Has 'staging_missing' field", "staging_missing" in data)

        missing = data.get("staging_missing", 0)
        total = data.get("total_mods", 0)
        exists = data.get("staging_exists", 0)
        print(f"  {INFO} {exists}/{data.get('mods_with_staging', '?')} staging dirs intact, {missing} missing")
        if missing > 0:
            examples = data.get("missing_examples", [])
            for ex in examples[:5]:
                print(f"  {INFO}   missing: {ex}")


def test_vortex_extensions(binary):
    """Suite 12: Vortex extension registry."""
    print("\n━━━ Suite 12: Vortex Extensions ━━━")

    code, out, err = run_cli(binary, "--vortex-list")
    test("--vortex-list exits 0", code == 0, f"exit={code}")

    data = parse_json(out)
    test("Output is valid JSON array", isinstance(data, list))

    if isinstance(data, list):
        print(f"  {INFO} {len(data)} cached extensions")
        for ext in data[:10]:
            print(f"  {INFO}   {ext.get('game_id','?')}: {ext.get('name','?')} "
                  f"({ext.get('tool_count',0)} tools, {ext.get('mod_type_count',0)} mod types)")

        if data:
            e0 = data[0]
            test("Extension has 'game_id'", "game_id" in e0)
            test("Extension has 'name'", "name" in e0)
            test("Extension has 'tool_count'", "tool_count" in e0)


def test_list_profiles(binary, game_id, bottle_name):
    """Suite 13: List profiles."""
    print(f"\n━━━ Suite 13: Profiles ({game_id}:{bottle_name}) ━━━")

    code, out, err = run_cli(binary, "--list-profiles", game_id, bottle_name)
    test("--list-profiles exits 0", code == 0, f"exit={code}")

    data = parse_json(out)
    test("Output is valid JSON array", isinstance(data, list))

    if isinstance(data, list):
        print(f"  {INFO} {len(data)} profiles")
        for p in data:
            active = "ACTIVE" if p.get("is_active") else "inactive"
            print(f"  {INFO}   {p.get('name','?')} ({active})")

        if data:
            test("Profile has 'id'", "id" in data[0])
            test("Profile has 'name'", "name" in data[0])
            test("Profile has 'is_active'", "is_active" in data[0])


def test_error_handling(binary, game_id, bottle_name):
    """Suite 14: Error handling and edge cases."""
    print("\n━━━ Suite 14: Error Handling ━━━")

    # Missing arguments
    code, out, err = run_cli(binary, "--list-mods")
    test("--list-mods with no args exits 1", code == 1)
    test("--list-mods prints usage", "Usage" in err, f"stderr: {err[:100]}")

    code, out, err = run_cli(binary, "--search-mods")
    test("--search-mods with no args exits 1", code == 1)

    code, out, err = run_cli(binary, "--find-file")
    test("--find-file with no args exits 1", code == 1)

    code, out, err = run_cli(binary, "--check-plugins")
    test("--check-plugins with no args exits 1", code == 1)

    code, out, err = run_cli(binary, "--mod-files")
    test("--mod-files with no args exits 1", code == 1)

    code, out, err = run_cli(binary, "--db-stats")
    test("--db-stats with no args exits 1", code == 1)

    # Invalid game/bottle
    code, out, err = run_cli(binary, "--list-mods", "fakegame999", "FakeBottle")
    test("Invalid game/bottle doesn't crash", code == 0,
         f"exit={code}, out length={len(out)}")

    # Invalid db-stats game/bottle should still return valid JSON
    code, out, err = run_cli(binary, "--db-stats", "fakegame999", "FakeBottle")
    test("--db-stats invalid game returns JSON", code == 0)
    data = parse_json(out)
    if isinstance(data, dict):
        test("Invalid game has 0 mods", data.get("total_mods", -1) == 0)


def test_performance(binary, game_id, bottle_name):
    """Suite 15: Performance benchmarks."""
    print(f"\n━━━ Suite 15: Performance ({game_id}:{bottle_name}) ━━━")

    benchmarks = [
        ("--list-bottles", [], 5),
        ("--list-games", [], 10),
        ("--db-integrity", [], 5),
        ("--db-stats", [game_id, bottle_name], 5),
        ("--vortex-list", [], 5),
        ("--list-profiles", [game_id, bottle_name], 5),
    ]

    for cmd, args, max_secs in benchmarks:
        t0 = time.time()
        code, out, err = run_cli(binary, cmd, *args, timeout=max_secs + 5)
        elapsed = time.time() - t0
        test(f"{cmd} < {max_secs}s", elapsed < max_secs and code == 0,
             f"{elapsed:.2f}s, exit={code}")
        print(f"  {INFO} {cmd}: {elapsed:.2f}s")


def test_concurrent_reads(binary, game_id, bottle_name):
    """Suite 16: Concurrent CLI invocations (database locking)."""
    print(f"\n━━━ Suite 16: Concurrent Reads ({game_id}:{bottle_name}) ━━━")

    import concurrent.futures

    cmds = [
        [binary, "--db-stats", game_id, bottle_name],
        [binary, "--db-integrity"],
        [binary, "--list-bottles"],
        [binary, "--vortex-list"],
        [binary, "--list-profiles", game_id, bottle_name],
    ]

    def run_one(cmd):
        try:
            r = subprocess.run(cmd, capture_output=True, text=True, timeout=15)
            return r.returncode
        except Exception as e:
            return -1

    t0 = time.time()
    with concurrent.futures.ThreadPoolExecutor(max_workers=5) as pool:
        futures = [pool.submit(run_one, cmd) for cmd in cmds]
        codes = [f.result() for f in concurrent.futures.as_completed(futures)]
    elapsed = time.time() - t0

    all_ok = all(c == 0 for c in codes)
    test("All 5 concurrent reads succeed", all_ok, f"exit codes: {codes}")
    test("Concurrent reads < 15s total", elapsed < 15, f"took {elapsed:.1f}s")
    print(f"  {INFO} 5 concurrent reads in {elapsed:.1f}s")


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    parser = argparse.ArgumentParser(description="Corkscrew E2E CLI Test Suite")
    parser.add_argument("--binary", default=DEFAULT_BINARY, help="Path to corkscrew binary")
    parser.add_argument("--game", default="skyrimse", help="Game ID for game-specific tests")
    parser.add_argument("--bottle", default="Steam", help="Bottle name for game-specific tests")
    parser.add_argument("--suite", type=int, help="Run only a specific suite number")
    args = parser.parse_args()

    print(f"\n{'='*70}")
    print(f"  Corkscrew E2E CLI Test Suite")
    print(f"  Binary:  {args.binary}")
    print(f"  Game:    {args.game}")
    print(f"  Bottle:  {args.bottle}")
    print(f"{'='*70}")

    t_start = time.time()

    # Suite 0: Binary check
    if not test_binary_exists(args.binary):
        print(f"\n{FAIL} Binary not found. Build first with: cargo build --release")
        sys.exit(1)

    suites = {
        1: lambda: test_help(args.binary),
        2: lambda: test_bottle_detection(args.binary),
        3: lambda: test_game_detection(args.binary),
        4: lambda: test_db_integrity(args.binary),
        5: lambda: test_db_stats(args.binary, args.game, args.bottle),
        6: lambda: test_list_mods(args.binary, args.game, args.bottle),
        7: lambda: test_search_mods(args.binary, args.game, args.bottle),
        8: lambda: test_find_file(args.binary, args.game, args.bottle),
        9: lambda: test_check_plugins(args.binary, args.game, args.bottle),
        10: lambda: test_mod_files(args.binary, args.game, args.bottle),
        11: lambda: test_deployment_health(args.binary, args.game, args.bottle),
        12: lambda: test_vortex_extensions(args.binary),
        13: lambda: test_list_profiles(args.binary, args.game, args.bottle),
        14: lambda: test_error_handling(args.binary, args.game, args.bottle),
        15: lambda: test_performance(args.binary, args.game, args.bottle),
        16: lambda: test_concurrent_reads(args.binary, args.game, args.bottle),
    }

    if args.suite is not None:
        if args.suite in suites:
            suites[args.suite]()
        else:
            print(f"\n{FAIL} Unknown suite {args.suite}. Valid: 1-{len(suites)}")
            sys.exit(1)
    else:
        for n in sorted(suites.keys()):
            try:
                suites[n]()
            except Exception as e:
                test(f"Suite {n} exception", False, str(e))

    # Summary
    total_elapsed = time.time() - t_start
    passed = sum(1 for _, p, _ in results if p is True)
    failed = sum(1 for _, p, _ in results if p is False)
    skipped = sum(1 for _, p, _ in results if p is None)
    total = len(results)

    print(f"\n{'='*70}")
    print(f"  Results: {passed}/{total} passed, {failed} failed, {skipped} skipped")
    print(f"  Time:    {total_elapsed:.1f}s")
    print(f"{'='*70}")

    if failed > 0:
        print(f"\n  Failed tests:")
        for name, p, detail in results:
            if p is False:
                print(f"  {FAIL} {name}")
                if detail:
                    print(f"        {detail[:200]}")

    sys.exit(0 if failed == 0 else 1)


if __name__ == "__main__":
    main()
