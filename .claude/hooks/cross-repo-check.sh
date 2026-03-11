#!/usr/bin/env bash
# Hook: SessionStart (startup|resume) — Check for cross-repo version drift
# Compares SSEEngineFixesForWine version in CMakeLists.txt vs memory files.

SSEEF_DIR="/Users/cashconway/SSEEngineFixesForWine"
MEMORY_DIR="$HOME/.claude/projects/-Users-cashconway-Corkscrew/memory"

# Skip if SSEEngineFixesForWine repo doesn't exist
if [[ ! -d "$SSEEF_DIR" ]]; then
  exit 0
fi

# Get actual version from CMakeLists.txt
ACTUAL_VERSION=""
if [[ -f "$SSEEF_DIR/CMakeLists.txt" ]]; then
  ACTUAL_VERSION=$(grep -oP 'VERSION\s+\K[0-9]+\.[0-9]+\.[0-9]+' "$SSEEF_DIR/CMakeLists.txt" 2>/dev/null | head -1)
fi

if [[ -z "$ACTUAL_VERSION" ]]; then
  exit 0
fi

# Check memory files for version references
DRIFT_FOUND=false
DRIFT_FILES=()

for file in "$MEMORY_DIR/sseef-wine.md" "$MEMORY_DIR/engine-fixes-wine.md" "$MEMORY_DIR/MEMORY.md"; do
  if [[ -f "$file" ]]; then
    # Look for version references like "v1.22.50" or "1.22.50"
    MEMORY_VERSIONS=$(grep -oP 'v?1\.\d+\.\d+' "$file" 2>/dev/null | grep -v "$ACTUAL_VERSION" | sort -u)
    if [[ -n "$MEMORY_VERSIONS" ]]; then
      # Check if any of these are SSEEngineFixesForWine versions (1.22.x pattern)
      OLD_VERSIONS=$(echo "$MEMORY_VERSIONS" | grep -P 'v?1\.22\.\d+' | head -3)
      if [[ -n "$OLD_VERSIONS" ]]; then
        DRIFT_FOUND=true
        DRIFT_FILES+=("$(basename "$file")")
      fi
    fi
  fi
done

if [[ "$DRIFT_FOUND" == "true" ]]; then
  echo "Cross-repo drift detected: SSEEngineFixesForWine is v${ACTUAL_VERSION} but memory files [${DRIFT_FILES[*]}] reference older versions. Consider updating memory files if you're working on SSEEngineFixesForWine."
fi

exit 0
