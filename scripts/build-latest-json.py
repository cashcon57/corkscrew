#!/usr/bin/env python3
"""Build latest.json for Tauri updater from build artifacts.

Usage: python3 scripts/build-latest-json.py artifacts/ > latest.json

Hybrid release: macOS built locally (release.sh), Linux built in CI.
Expects artifact directories with this layout:
  artifacts/
    macos-aarch64-apple-darwin/       (downloaded from draft GH release)
      Corkscrew_aarch64.app.tar.gz
      Corkscrew_aarch64.app.tar.gz.sig
    macos-x86_64-apple-darwin/        (downloaded from draft GH release)
      Corkscrew_x86_64.app.tar.gz
      Corkscrew_x86_64.app.tar.gz.sig
    linux-x86_64-unknown-linux-gnu/   (from CI build artifacts)
      Corkscrew-VERSION-Linux.AppImage.tar.gz + .sig  (or .AppImage + .sig)
"""

import json
import sys
from datetime import datetime, timezone
from pathlib import Path

REPO = "cashcon57/corkscrew"

# Map artifact directory names to Tauri platform keys
PLATFORM_MAP = {
    "macos-aarch64-apple-darwin": "darwin-aarch64",
    "macos-x86_64-apple-darwin": "darwin-x86_64",
    "linux-x86_64-unknown-linux-gnu": "linux-x86_64",
}


def find_updater_pair(dir_path: Path):
    """Find the updater artifact (.tar.gz) and its signature (.sig) in a directory.

    Handles two formats:
    - macOS: Corkscrew_ARCH.app.tar.gz + .sig
    - Linux: *.AppImage.tar.gz + .sig  (or *.AppImage + *.AppImage.sig)
    """
    sig_file = None
    bundle_file = None

    for f in sorted(dir_path.rglob("*")):
        name = f.name
        # Prefer .tar.gz updater bundles (macOS .app.tar.gz, Linux .AppImage.tar.gz)
        if name.endswith(".tar.gz.sig"):
            sig_file = f
        elif name.endswith(".tar.gz") and not name.endswith(".tar.gz.sig"):
            bundle_file = f

    # Fallback for Linux: use .AppImage + .AppImage.sig directly
    if not (sig_file and bundle_file):
        for f in sorted(dir_path.rglob("*")):
            name = f.name
            if name.endswith(".AppImage.sig") and not name.endswith(".tar.gz.sig"):
                sig_file = f
            elif name.endswith(".AppImage") and not name.endswith(".sig"):
                bundle_file = f

    return bundle_file, sig_file


def find_files(artifact_dir: Path) -> dict:
    """Walk artifact directories and collect updater bundle + sig pairs."""
    platforms = {}

    for dir_name, platform_key in PLATFORM_MAP.items():
        dir_path = artifact_dir / dir_name
        if not dir_path.is_dir():
            continue

        bundle_file, sig_file = find_updater_pair(dir_path)

        if sig_file and bundle_file:
            platforms[platform_key] = {
                "signature": sig_file.read_text().strip(),
                "url": f"https://github.com/{REPO}/releases/download/{{{{version}}}}/{bundle_file.name}",
            }

    return platforms


def main():
    if len(sys.argv) < 2:
        print(f"Usage: {sys.argv[0]} <artifacts_dir>", file=sys.stderr)
        sys.exit(1)

    artifact_dir = Path(sys.argv[1])
    if not artifact_dir.is_dir():
        print(f"Error: {artifact_dir} is not a directory", file=sys.stderr)
        sys.exit(1)

    # Read version from tauri.conf.json
    conf_path = Path(__file__).parent.parent / "src-tauri" / "tauri.conf.json"
    with open(conf_path) as f:
        version = json.load(f)["version"]

    platforms = find_files(artifact_dir)

    if not platforms:
        print("Error: No updater artifacts found", file=sys.stderr)
        sys.exit(1)

    # Replace version placeholder in URLs
    tag = f"v{version}"
    for key in platforms:
        platforms[key]["url"] = platforms[key]["url"].replace("{{version}}", tag)

    result = {
        "version": version,
        "notes": f"v{version}",
        "pub_date": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "platforms": platforms,
    }

    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
