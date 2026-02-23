#!/usr/bin/env bash
# Build Corkscrew with updater signing key + Apple code signing auto-loaded.
# Usage: ./scripts/build.sh [extra cargo tauri build args...]

set -euo pipefail

KEY_FILE="$HOME/.corkscrew-keys/corkscrew-signing-key-v3"

if [[ ! -f "$KEY_FILE" ]]; then
  echo "ERROR: Signing key not found at $KEY_FILE"
  exit 1
fi

export TAURI_SIGNING_PRIVATE_KEY
TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY_FILE")"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="corkscrew-updater-2024"

echo "Signing key loaded from $KEY_FILE"

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

exec cargo tauri build "$@"
