#!/usr/bin/env python3
"""Verify that latest.json signatures match the pubkey in tauri.conf.json.

Usage: python3 scripts/verify-latest-json.py <latest.json>

Exits 0 if all signatures match, 1 if any mismatch.
This catches the #1 cause of silent auto-update failure:
signing with the wrong key (e.g., CI key != local key).
"""

import base64
import json
import struct
import sys
from pathlib import Path


def extract_key_id(raw_bytes: bytes) -> bytes:
    """Extract the key ID (bytes 2-10) from a minisign signature or pubkey."""
    # minisign format: 2-byte algorithm + 8-byte key ID + payload
    if len(raw_bytes) < 10:
        return b""
    return raw_bytes[2:10]


def main():
    if len(sys.argv) < 2:
        print(f"Usage: {sys.argv[0]} <latest.json>", file=sys.stderr)
        sys.exit(1)

    latest_path = Path(sys.argv[1])
    if not latest_path.exists():
        print(f"Error: {latest_path} not found", file=sys.stderr)
        sys.exit(1)

    # Load pubkey from tauri.conf.json
    conf_path = Path(__file__).parent.parent / "src-tauri" / "tauri.conf.json"
    with open(conf_path) as f:
        pubkey_b64 = json.load(f)["plugins"]["updater"]["pubkey"]

    pubkey_decoded = base64.b64decode(pubkey_b64).decode()
    pubkey_lines = pubkey_decoded.strip().split("\n")
    # Second line is the actual key
    pubkey_raw = base64.b64decode(pubkey_lines[1])
    expected_key_id = extract_key_id(pubkey_raw)

    print(f"Expected key ID: {expected_key_id.hex()}")

    # Load latest.json
    with open(latest_path) as f:
        latest = json.load(f)

    errors = 0
    for platform, info in latest.get("platforms", {}).items():
        sig_b64 = info.get("signature", "")
        url = info.get("url", "")

        sig_decoded = base64.b64decode(sig_b64).decode()
        sig_lines = sig_decoded.strip().split("\n")
        # Second line is the actual signature
        sig_raw = base64.b64decode(sig_lines[1])
        sig_key_id = extract_key_id(sig_raw)

        match = sig_key_id == expected_key_id
        status = "OK" if match else "MISMATCH"

        print(f"  {platform}: key_id={sig_key_id.hex()} [{status}]")
        if not match:
            print(f"    ERROR: Signature was made with a DIFFERENT key!")
            print(f"    Expected: {expected_key_id.hex()}")
            print(f"    Got:      {sig_key_id.hex()}")
            print(f"    URL: {url}")
            errors += 1

    if errors:
        print(f"\nFATAL: {errors} platform(s) have signature key mismatches!")
        print("Auto-update WILL FAIL for users. Do NOT publish this release.")
        print("Fix: Ensure TAURI_SIGNING_PRIVATE_KEY matches the key in .keys/updater-signing-key")
        sys.exit(1)
    else:
        print(f"\nAll {len(latest['platforms'])} platform signatures verified.")
        sys.exit(0)


if __name__ == "__main__":
    main()
