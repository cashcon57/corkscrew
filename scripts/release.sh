#!/usr/bin/env bash
# Hybrid release: build macOS locally on dev machine, push tag for CI to build Linux.
#
# Flow:
#   1. Bumps version in tauri.conf.json, package.json, Cargo.toml
#   2. Builds macOS Apple Silicon + Intel with signing
#   3. Commits version bump, tags, pushes
#   4. Publishes GitHub release with macOS artifacts (auto-updater works immediately)
#   5. CI triggers on tag push → builds Linux, adds Linux artifacts + updates latest.json
#
# Usage: ./scripts/release.sh <version>
#   e.g.: ./scripts/release.sh 0.10.0

set -euo pipefail

VERSION="${1:?Usage: $0 <version>}"
TAG="v$VERSION"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# --- Config ---
KEY_FILE="$ROOT/.keys/updater-signing-key"
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

# --- Load signing credentials ---
export TAURI_SIGNING_PRIVATE_KEY
TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY_FILE")"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$KEY_PASSWORD"

# CRITICAL: Verify the signing key matches the pubkey in tauri.conf.json.
# A mismatch means auto-update will silently fail for all users.
EXPECTED_PUBKEY="dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDdBMzhEMDdFOUM4MDRBMDAKUldRQVNvQ2NmdEE0ZW1YWWdsZjFkMEdTTWxFeHd4Y1IwTHhaV1M5VmU4VEJGb3lWdDhIbGNkWWsK"
CONF_PUBKEY=$(python3 -c "import json; print(json.load(open('$ROOT/src-tauri/tauri.conf.json'))['plugins']['updater']['pubkey'])")
if [[ "$CONF_PUBKEY" != "$EXPECTED_PUBKEY" ]]; then
  echo "FATAL: tauri.conf.json pubkey does NOT match expected key."
  echo "  Expected: $EXPECTED_PUBKEY"
  echo "  Got:      $CONF_PUBKEY"
  echo "  DO NOT change the signing key or pubkey without a migration plan."
  exit 1
fi
echo "Signing key verified against tauri.conf.json pubkey."

# Apple code signing + notarization
APPLE_CREDS="$HOME/.corkscrew-keys/apple-signing.env"
if [[ -f "$APPLE_CREDS" ]]; then
  # shellcheck disable=SC1090
  source "$APPLE_CREDS"
  echo "Apple signing credentials loaded — builds will be signed + notarized."
else
  echo "WARNING: Apple signing credentials not found at $APPLE_CREDS"
  echo "         Release builds will NOT be signed/notarized."
  echo "         Run ./scripts/setup-apple-signing.sh to configure."
  read -r -p "Continue without signing? [y/N] " confirm
  [[ "$confirm" =~ ^[Yy]$ ]] || exit 1
fi

echo ""
echo "=== Building macOS (Apple Silicon) ==="
echo ""
cargo tauri build --target aarch64-apple-darwin
"$ROOT/scripts/rebuild-dmg.sh" aarch64-apple-darwin

echo ""
echo "=== Building macOS (Intel) ==="
echo ""
cargo tauri build --target x86_64-apple-darwin
"$ROOT/scripts/rebuild-dmg.sh" x86_64-apple-darwin

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

# CRITICAL: Verify signatures in latest.json match our pubkey BEFORE uploading.
# This catches the exact bug where CI or a wrong key signs the artifacts.
echo ""
echo "=== Verifying latest.json signatures ==="
python3 "$ROOT/scripts/verify-latest-json.py" "$STAGE/latest.json"

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

# --- Create release (published immediately) ---
echo ""
echo "=== Creating release $TAG ==="
gh release create "$TAG" \
  --repo "$REPO" \
  --title "$TAG" \
  --notes "$RELEASE_NOTES" \
  "$STAGE"/*

echo ""
echo "========================================="
echo "  Release $TAG published"
echo "========================================="
echo ""
echo "  macOS artifacts + latest.json uploaded."
echo "  macOS auto-updater will detect this release immediately."
echo ""
echo "  CI is now building Linux — it will add Linux artifacts"
echo "  and update latest.json with Linux platforms when done."
echo ""
echo "  Release: https://github.com/$REPO/releases/tag/$TAG"
echo "  CI:      https://github.com/$REPO/actions"
echo ""
