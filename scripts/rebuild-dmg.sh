#!/usr/bin/env bash
# Rebuild DMG using appdmg for correct icon positioning and custom background.
# Tauri's built-in DMG builder uses AppleScript which silently fails on modern macOS.
#
# Usage: ./scripts/rebuild-dmg.sh <target-triple>
#   e.g.: ./scripts/rebuild-dmg.sh aarch64-apple-darwin
#         ./scripts/rebuild-dmg.sh x86_64-apple-darwin

set -euo pipefail

TARGET="${1:?Usage: $0 <target-triple>}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

APP_DIR="$ROOT/src-tauri/target/$TARGET/release/bundle/macos"
DMG_DIR="$ROOT/src-tauri/target/$TARGET/release/bundle/dmg"
BACKGROUND="$ROOT/src-tauri/icons/dmg-background.png"

# Find the .app bundle
APP_PATH=$(find "$APP_DIR" -maxdepth 1 -name '*.app' -type d | head -1)
if [[ -z "$APP_PATH" ]]; then
  echo "ERROR: No .app found in $APP_DIR"
  exit 1
fi
APP_NAME=$(basename "$APP_PATH")

# Find the existing DMG (to replace it)
EXISTING_DMG=$(find "$DMG_DIR" -maxdepth 1 -name '*.dmg' -type f | head -1)
if [[ -z "$EXISTING_DMG" ]]; then
  echo "ERROR: No existing DMG found in $DMG_DIR"
  exit 1
fi

echo "Rebuilding DMG with appdmg..."
echo "  App:        $APP_PATH"
echo "  Background: $BACKGROUND"
echo "  Output:     $EXISTING_DMG"

# Generate appdmg config
CONFIG=$(mktemp /tmp/corkscrew-dmg-XXXXXX.json)
cat > "$CONFIG" <<ENDJSON
{
  "title": "Install Corkscrew",
  "background": "$BACKGROUND",
  "icon-size": 128,
  "window": {
    "size": {
      "width": 660,
      "height": 400
    }
  },
  "contents": [
    {
      "x": 180,
      "y": 190,
      "type": "file",
      "path": "$APP_PATH"
    },
    {
      "x": 480,
      "y": 190,
      "type": "link",
      "path": "/Applications"
    }
  ]
}
ENDJSON

# Remove old DMG (appdmg won't overwrite)
rm -f "$EXISTING_DMG"

# Build new DMG
npx appdmg "$CONFIG" "$EXISTING_DMG"

# Clean up
rm -f "$CONFIG"

echo "DMG rebuilt: $EXISTING_DMG"
