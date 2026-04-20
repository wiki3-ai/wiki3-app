#!/usr/bin/env bash
set -euo pipefail

# Release the built Wiki3 DMG to GitHub.
# Usage:
#   ./scripts/release.sh              # release current version from package.json
#   ./scripts/release.sh v0.2.0       # release with explicit tag
#   ./scripts/release.sh --draft      # create a draft release
#   ./scripts/release.sh v0.2.0 --draft

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

# Parse args
TAG=""
DRAFT=""
for arg in "$@"; do
  case "$arg" in
    --draft) DRAFT="--draft" ;;
    v*)      TAG="$arg" ;;
    *)       echo "Unknown argument: $arg"; exit 1 ;;
  esac
done

# Default tag from package.json version
if [ -z "$TAG" ]; then
  VERSION=$(python3 -c "import json; print(json.load(open('package.json'))['version'])")
  TAG="v${VERSION}"
fi

# Find DMG
DMG=$(find src-tauri/target -name '*.dmg' -newer src-tauri/Cargo.toml 2>/dev/null | head -1)
if [ -z "$DMG" ]; then
  echo "No DMG found. Run 'npm run tauri:build:arm64' first."
  exit 1
fi

# Check signing status
SIGNED="unsigned"
if CODESIGN_OUT=$(codesign -dvv "$REPO_ROOT/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Wiki3.app" 2>&1); then
  if echo "$CODESIGN_OUT" | grep -q "Authority=Developer ID"; then
    SIGNED="signed"
  fi
fi

DMG_NAME=$(basename "$DMG")
DMG_SIZE=$(du -h "$DMG" | cut -f1 | xargs)

echo "Release: $TAG"
echo "Asset:   $DMG_NAME ($DMG_SIZE, $SIGNED)"
echo ""

# Check if release already exists
if gh release view "$TAG" &>/dev/null; then
  echo "Release $TAG already exists. Uploading asset..."
  gh release upload "$TAG" "$DMG" --clobber
else
  TITLE="Wiki3 ${TAG}"
  NOTES="macOS (Apple Silicon, Sequoia+) — ${SIGNED}"

  echo "Creating release $TAG..."
  gh release create "$TAG" \
    --title "$TITLE" \
    --notes "$NOTES" \
    $DRAFT \
    "$DMG"
fi

echo ""
echo "Done. https://github.com/$(gh repo view --json nameWithOwner -q .nameWithOwner)/releases/tag/$TAG"
