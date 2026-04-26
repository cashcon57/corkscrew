#!/usr/bin/env bash
# CI-driven release: bump versions, tag, push. CI builds + publishes.
#
# Flow:
#   1. Preflight checks (on main, clean tree, tag doesn't exist, signing
#      pubkey matches tauri.conf.json).
#   2. Bump version in tauri.conf.json, package.json, Cargo.toml, Cargo.lock.
#   3. Commit the bump, tag, push both.
#   4. `.github/workflows/release.yml` fires on the tag and builds
#      macOS (aarch64 + x86_64) + Linux in parallel, generates latest.json,
#      and publishes the GitHub release.
#
# No local builds. No signing credentials needed on dev machine — all secrets
# live in GitHub. Dev machine just needs: clean main, git push access, gh CLI.
#
# Usage: ./scripts/release.sh <version>
#   e.g.: ./scripts/release.sh 0.13.1

set -euo pipefail

VERSION="${1:?Usage: $0 <version>}"
TAG="v$VERSION"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPO="cashcon57/corkscrew"

# --- Preflight ---
echo "=== Corkscrew Release $TAG ==="
echo ""

errors=()
command -v gh    >/dev/null 2>&1 || errors+=("gh CLI not installed")
command -v jq    >/dev/null 2>&1 || errors+=("jq not installed")
command -v cargo >/dev/null 2>&1 || errors+=("cargo not found — check PATH")

if (( ${#errors[@]} )); then
  for e in "${errors[@]}"; do echo "ERROR: $e"; done
  exit 1
fi

# Must be on main
BRANCH=$(git -C "$ROOT" rev-parse --abbrev-ref HEAD)
if [[ "$BRANCH" != "main" ]]; then
  echo "ERROR: Must be on main (currently on $BRANCH)"
  exit 1
fi

# No uncommitted changes
if ! git -C "$ROOT" diff --quiet || ! git -C "$ROOT" diff --cached --quiet; then
  echo "ERROR: Working tree has uncommitted changes. Commit or stash first."
  exit 1
fi

# Main up-to-date with origin (CI will skip if we push a divergent tag anyway)
git -C "$ROOT" fetch origin main --quiet
LOCAL_SHA=$(git -C "$ROOT" rev-parse HEAD)
REMOTE_SHA=$(git -C "$ROOT" rev-parse origin/main)
if [[ "$LOCAL_SHA" != "$REMOTE_SHA" ]]; then
  if ! git -C "$ROOT" merge-base --is-ancestor "$REMOTE_SHA" "$LOCAL_SHA"; then
    echo "ERROR: Local main is behind origin/main. Pull and re-run."
    echo "       Local:  $LOCAL_SHA"
    echo "       Remote: $REMOTE_SHA"
    exit 1
  fi
fi

# Tag must not exist locally or remotely
git -C "$ROOT" fetch --tags --quiet
if git -C "$ROOT" rev-parse "$TAG" >/dev/null 2>&1; then
  echo "ERROR: Tag $TAG already exists"
  exit 1
fi

# Verify the pubkey hasn't drifted. CI builds with this key; if tauri.conf.json
# disagrees with what CI signs with, the auto-updater silently breaks for users.
EXPECTED_PUBKEY="dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDdBMzhEMDdFOUM4MDRBMDAKUldRQVNvQ2NmdEE0ZW1YWWdsZjFkMEdTTWxFeHd4Y1IwTHhaV1M5VmU4VEJGb3lWdDhIbGNkWWsK"
CONF_PUBKEY=$(python3 -c "import json; print(json.load(open('$ROOT/src-tauri/tauri.conf.json'))['plugins']['updater']['pubkey'])")
if [[ "$CONF_PUBKEY" != "$EXPECTED_PUBKEY" ]]; then
  echo "FATAL: tauri.conf.json pubkey does NOT match expected key."
  echo "  Expected: $EXPECTED_PUBKEY"
  echo "  Got:      $CONF_PUBKEY"
  echo "  DO NOT change the signing key or pubkey without a migration plan."
  exit 1
fi

echo "Preflight passed."

# --- Bump versions ---
echo ""
echo "=== Bumping to $VERSION ==="

jq --arg v "$VERSION" '.version = $v' "$ROOT/src-tauri/tauri.conf.json" > /tmp/corkscrew-tc.json \
  && mv /tmp/corkscrew-tc.json "$ROOT/src-tauri/tauri.conf.json"

cd "$ROOT"
npm version "$VERSION" --no-git-tag-version --allow-same-version >/dev/null

# First `version = ` under [package] in Cargo.toml.
# Use python3 (already a hard dep above for jq+pubkey check) instead of sed so
# this works on both BSD/macOS (`sed -i ''`) and GNU/Linux (`sed -i`) without
# branching on $(uname).
VERSION="$VERSION" python3 - "$ROOT/src-tauri/Cargo.toml" <<'PY'
import os, re, sys
path = sys.argv[1]
version = os.environ["VERSION"]
with open(path, "r", encoding="utf-8") as f:
    content = f.read()
new_content, n = re.subn(
    r'^version = .*',
    f'version = "{version}"',
    content,
    count=1,
    flags=re.MULTILINE,
)
if n != 1:
    sys.stderr.write(f"ERROR: failed to find `version = ...` line in {path}\n")
    sys.exit(1)
with open(path, "w", encoding="utf-8") as f:
    f.write(new_content)
PY

# Refresh Cargo.lock so the package version there matches (no network fetch
# needed — `cargo check --offline` only updates the local workspace entry).
cargo check --offline --manifest-path "$ROOT/src-tauri/Cargo.toml" --message-format=short >/dev/null 2>&1 || \
cargo check --manifest-path "$ROOT/src-tauri/Cargo.toml" --message-format=short >/dev/null

echo "  tauri.conf.json  → $VERSION"
echo "  package.json     → $VERSION"
echo "  Cargo.toml       → $VERSION"
echo "  Cargo.lock       → $VERSION"

# --- Commit + tag + push ---
echo ""
echo "=== Committing + tagging ==="

git add \
  src-tauri/tauri.conf.json \
  package.json \
  package-lock.json \
  src-tauri/Cargo.toml \
  src-tauri/Cargo.lock

git commit -m "$(cat <<EOF
v${VERSION}

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"

git tag "$TAG"
git push origin main "$TAG"

# --- Done — CI takes over ---
echo ""
echo "========================================="
echo "  Release $TAG pushed"
echo "========================================="
echo ""
echo "  CI is now building macOS (aarch64 + x86_64) + Linux in parallel."
echo "  When done, it will publish $TAG with latest.json + all artifacts,"
echo "  and the auto-updater will pick it up."
echo ""
echo "  Release: https://github.com/$REPO/releases/tag/$TAG"
echo "  CI:      https://github.com/$REPO/actions"
echo ""
