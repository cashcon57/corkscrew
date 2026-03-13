#!/usr/bin/env bash
# Build Corkscrew with updater signing key + Apple code signing auto-loaded.
# Usage: ./scripts/build.sh [extra cargo tauri build args...]

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
KEY_FILE="$ROOT/.keys/updater-signing-key"

if [[ ! -f "$KEY_FILE" ]]; then
  echo "ERROR: Signing key not found at $KEY_FILE"
  exit 1
fi

export TAURI_SIGNING_PRIVATE_KEY
TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY_FILE")"

# Signing key password: prefer env var, fall back to interactive prompt
if [[ -z "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}" ]]; then
    echo -n "Enter signing key password: "
    read -rs KEY_PASSWORD
    echo
else
    KEY_PASSWORD="$TAURI_SIGNING_PRIVATE_KEY_PASSWORD"
fi
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$KEY_PASSWORD"

# CRITICAL: Verify the signing key matches the pubkey in tauri.conf.json.
# A mismatch means auto-update will silently fail for all users.
EXPECTED_PUBKEY="dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDdBMzhEMDdFOUM4MDRBMDAKUldRQVNvQ2NmdEE0ZW1YWWdsZjFkMEdTTWxFeHd4Y1IwTHhaV1M5VmU4VEJGb3lWdDhIbGNkWWsK"
CONF_PUBKEY=$(python3 -c "import json; print(json.load(open('$(cd "$(dirname "$0")/.." && pwd)/src-tauri/tauri.conf.json'))['plugins']['updater']['pubkey'])")
if [[ "$CONF_PUBKEY" != "$EXPECTED_PUBKEY" ]]; then
  echo "FATAL: tauri.conf.json pubkey does NOT match expected key."
  echo "  Expected: $EXPECTED_PUBKEY"
  echo "  Got:      $CONF_PUBKEY"
  echo "  DO NOT change the signing key or pubkey without a migration plan."
  exit 1
fi

echo "Signing key loaded and verified from $KEY_FILE"

# Apple code signing + notarization (macOS only)
APPLE_CREDS="$HOME/.corkscrew-keys/apple-signing.env"
if [[ -f "$APPLE_CREDS" ]]; then
  # shellcheck disable=SC1090
  source "$APPLE_CREDS"
  echo "Apple signing credentials loaded from $APPLE_CREDS"
else
  echo "NOTE: Apple signing credentials not found at $APPLE_CREDS"
  echo "      Build will proceed WITHOUT code signing/notarization."
  echo "      Run ./scripts/setup-apple-signing.sh to configure."
fi

cargo tauri build "$@"

# Rebuild DMG with appdmg for correct icon positioning (macOS only)
if [[ "$(uname)" == "Darwin" ]]; then
  # Detect target triple from args, default to native arch
  TARGET=""
  for arg in "$@"; do
    case "$arg" in
      aarch64-apple-darwin|x86_64-apple-darwin) TARGET="$arg" ;;
    esac
  done
  if [[ -z "$TARGET" ]]; then
    if [[ "$(uname -m)" == "arm64" ]]; then
      TARGET="aarch64-apple-darwin"
    else
      TARGET="x86_64-apple-darwin"
    fi
  fi
  "$ROOT/scripts/rebuild-dmg.sh" "$TARGET"
fi
