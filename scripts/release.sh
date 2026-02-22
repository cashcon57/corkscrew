#!/usr/bin/env bash
# Hybrid release: build macOS locally on dev machine, push tag for CI to build Linux.
#
# Flow:
#   1. Bumps version in tauri.conf.json, package.json, Cargo.toml
#   2. Builds macOS Apple Silicon + Intel with signing
#   3. Commits version bump, tags, pushes
#   4. Creates draft GitHub release with macOS artifacts
#   5. CI triggers on tag push → builds Linux, generates latest.json, publishes
#
# Usage: ./scripts/release.sh <version>
#   e.g.: ./scripts/release.sh 0.10.0

set -euo pipefail

VERSION="${1:?Usage: $0 <version>}"
TAG="v$VERSION"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# --- Config ---
KEY_FILE="$HOME/.corkscrew-keys/corkscrew-signing-key-v3"
KEY_PASSWORD="corkscrew-updater-2024"
REPO="cashcon57/corkscrew"

# --- Preflight checks ---
echo "=== Corkscrew Release $TAG ==="
echo ""

errors=()
[[ -f "$KEY_FILE" ]] || errors+=("Signing key not found: $KEY_FILE")
command -v gh  >/dev/null 2>&1 || errors+=("gh CLI not installed")
command -v jq  >/dev/null 2>&1 || errors+=("jq not installed")
command -v cargo >/dev/null 2>&1 || errors+=("cargo not found — check PATH")

if (( ${#errors[@]} )); then
  for e in "${errors[@]}"; do echo "ERROR: $e"; done
  exit 1
fi

# Must be on main
BRANCH=$(git -C "$ROOT" rev-parse --abbrev-ref HEAD)
if [[ "$BRANCH" != "main" ]]; then
  echo "ERROR: Must be on main branch (currently on $BRANCH)"
  exit 1
fi

# No uncommitted changes
if ! git -C "$ROOT" diff --quiet || ! git -C "$ROOT" diff --cached --quiet; then
  echo "ERROR: Working tree has uncommitted changes. Commit or stash first."
  exit 1
fi

# Tag must not exist
git -C "$ROOT" fetch --tags --quiet
if git -C "$ROOT" rev-parse "$TAG" >/dev/null 2>&1; then
  echo "ERROR: Tag $TAG already exists"
  exit 1
fi

echo "Preflight checks passed."

# --- Bump versions ---
echo ""
echo "=== Bumping version to $VERSION ==="

# tauri.conf.json
jq --arg v "$VERSION" '.version = $v' "$ROOT/src-tauri/tauri.conf.json" > /tmp/corkscrew-tc.json \
  && mv /tmp/corkscrew-tc.json "$ROOT/src-tauri/tauri.conf.json"

# package.json + package-lock.json
cd "$ROOT"
npm version "$VERSION" --no-git-tag-version --allow-same-version >/dev/null

# Cargo.toml (first version = line, under [package])
sed -i '' '1,/^version = /s/^version = .*/version = "'"$VERSION"'"/' "$ROOT/src-tauri/Cargo.toml"

echo "  tauri.conf.json  → $VERSION"
echo "  package.json     → $VERSION"
echo "  Cargo.toml       → $VERSION"

# --- Build macOS ---
export TAURI_SIGNING_PRIVATE_KEY
TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY_FILE")"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$KEY_PASSWORD"

echo ""
echo "=== Building macOS (Apple Silicon) ==="
echo ""
cargo tauri build --target aarch64-apple-darwin

echo ""
echo "=== Building macOS (Intel) ==="
echo ""
cargo tauri build --target x86_64-apple-darwin

# --- Stage artifacts ---
STAGE="$ROOT/target/release-stage"
rm -rf "$STAGE" && mkdir -p "$STAGE"

AARCH64="$ROOT/src-tauri/target/aarch64-apple-darwin/release/bundle"
X86_64="$ROOT/src-tauri/target/x86_64-apple-darwin/release/bundle"

# User-facing DMGs
cp "$AARCH64"/dmg/*.dmg "$STAGE/Corkscrew-${VERSION}-macOS-Apple-Silicon.dmg"
cp "$X86_64"/dmg/*.dmg  "$STAGE/Corkscrew-${VERSION}-macOS-Intel.dmg"

# Updater tarballs + signatures (arch-suffixed for latest.json)
cp "$AARCH64/macos/Corkscrew.app.tar.gz"     "$STAGE/Corkscrew_aarch64.app.tar.gz"
cp "$AARCH64/macos/Corkscrew.app.tar.gz.sig" "$STAGE/Corkscrew_aarch64.app.tar.gz.sig"
cp "$X86_64/macos/Corkscrew.app.tar.gz"      "$STAGE/Corkscrew_x86_64.app.tar.gz"
cp "$X86_64/macos/Corkscrew.app.tar.gz.sig"  "$STAGE/Corkscrew_x86_64.app.tar.gz.sig"

echo ""
echo "=== Staged artifacts ==="
ls -lh "$STAGE/"

# --- Generate latest.json (macOS-only, CI will merge Linux later) ---
echo ""
echo "=== Generating latest.json (macOS platforms) ==="

LATEST_TMP="$ROOT/target/latest-stage"
rm -rf "$LATEST_TMP" && mkdir -p \
  "$LATEST_TMP/macos-aarch64-apple-darwin" \
  "$LATEST_TMP/macos-x86_64-apple-darwin"

cp "$STAGE/Corkscrew_aarch64.app.tar.gz"     "$LATEST_TMP/macos-aarch64-apple-darwin/"
cp "$STAGE/Corkscrew_aarch64.app.tar.gz.sig" "$LATEST_TMP/macos-aarch64-apple-darwin/"
cp "$STAGE/Corkscrew_x86_64.app.tar.gz"      "$LATEST_TMP/macos-x86_64-apple-darwin/"
cp "$STAGE/Corkscrew_x86_64.app.tar.gz.sig"  "$LATEST_TMP/macos-x86_64-apple-darwin/"

python3 "$ROOT/scripts/build-latest-json.py" "$LATEST_TMP" > "$STAGE/latest.json"
echo "  latest.json generated with macOS platforms"
cat "$STAGE/latest.json" | python3 -m json.tool 2>/dev/null || cat "$STAGE/latest.json"
rm -rf "$LATEST_TMP"

# --- Commit, tag, push ---
echo ""
echo "=== Committing version bump ==="
cd "$ROOT"
git add \
  src-tauri/tauri.conf.json \
  package.json \
  package-lock.json \
  src-tauri/Cargo.toml

git commit -m "$(cat <<EOF
v${VERSION}

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>
EOF
)"

git tag "$TAG"
git push origin main "$TAG"

# --- Generate changelog ---
echo ""
echo "=== Generating changelog ==="

# Find previous tag for diff
PREV_TAG=$(git tag --sort=-v:refname | grep -E '^v[0-9]' | head -n 2 | tail -n 1)
if [[ -z "$PREV_TAG" ]]; then
  PREV_TAG=$(git rev-list --max-parents=0 HEAD)
  echo "  No previous tag found, using initial commit"
else
  echo "  Changes since $PREV_TAG"
fi

# Build changelog from commit messages (skip version bump commits)
CHANGELOG=$(git log "${PREV_TAG}..HEAD~1" --pretty=format:"- %s" --no-merges \
  | grep -v "^- v[0-9]" \
  | grep -v "^- Co-Authored-By" \
  || true)

if [[ -z "$CHANGELOG" ]]; then
  CHANGELOG="- Bug fixes and improvements"
fi

RELEASE_NOTES="## What's Changed

${CHANGELOG}

**Full Changelog**: https://github.com/${REPO}/compare/${PREV_TAG}...${TAG}"

echo "$RELEASE_NOTES"
echo ""

# --- Create draft release ---
echo ""
echo "=== Creating draft release $TAG ==="
gh release create "$TAG" \
  --repo "$REPO" \
  --draft \
  --title "$TAG" \
  --notes "$RELEASE_NOTES" \
  "$STAGE"/*

echo ""
echo "========================================="
echo "  Release $TAG draft created"
echo "========================================="
echo ""
echo "  macOS artifacts + latest.json uploaded to draft release."
echo "  macOS users can update immediately."
echo ""
echo "  CI is now building Linux — it will add Linux artifacts"
echo "  and merge Linux platforms into latest.json."
echo ""
echo "  Draft:   https://github.com/$REPO/releases/tag/$TAG"
echo "  CI:      https://github.com/$REPO/actions"
echo ""
