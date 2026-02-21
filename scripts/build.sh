#!/usr/bin/env bash
# Build Corkscrew with updater signing key auto-loaded.
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
exec cargo tauri build "$@"
