#!/usr/bin/env bash
# Build Corkscrew with updater signing key auto-loaded.
# Usage: ./scripts/build.sh [extra cargo tauri build args...]
#
# The signing key is read from ~/.tauri/corkscrew-tauri2 (rsign format).
# Tauri does NOT auto-load .env files, so this script handles it.

set -euo pipefail

KEY_FILE="$HOME/.tauri/corkscrew-tauri2"

if [[ ! -f "$KEY_FILE" ]]; then
  echo "ERROR: Signing key not found at $KEY_FILE"
  echo "Generate one with: cargo tauri signer generate --ci -w ~/.tauri/corkscrew-tauri2 -p 'corkscrew' -f"
  exit 1
fi

export TAURI_SIGNING_PRIVATE_KEY
TAURI_SIGNING_PRIVATE_KEY="$KEY_FILE"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="corkscrew"

echo "Signing key loaded from $KEY_FILE"
exec cargo tauri build "$@"
