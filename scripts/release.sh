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
VERSION=$(python3 -c "import json; print(json.load(open('package.json'))['version'])")
if [ -z "$TAG" ]; then
  TAG="v${VERSION}"
fi

# Sanity-check that all version sources agree, otherwise we'd ship
# a tag that doesn't match the artifact metadata.
CARGO_VERSION=$(awk -F'"' '/^version *= *"/ {print $2; exit}' src-tauri/Cargo.toml)
TAURI_VERSION=$(python3 -c "import json; print(json.load(open('src-tauri/tauri.conf.json'))['version'])")
if [ "$VERSION" != "$CARGO_VERSION" ] || [ "$VERSION" != "$TAURI_VERSION" ]; then
  echo "Version mismatch:"
  echo "  package.json:      $VERSION"
  echo "  src-tauri/Cargo.toml: $CARGO_VERSION"
  echo "  tauri.conf.json:   $TAURI_VERSION"
  echo "Bump them all to the same value before releasing."
  exit 1
fi

# Find DMG matching this version. The bundle filename embeds the
# version Tauri saw at build time (`Wiki3_<ver>_aarch64.dmg`), so
# requiring an exact match catches the "I bumped versions but didn't
# rebuild" footgun.
DMG=$(find src-tauri/target -name "Wiki3_${VERSION}_*.dmg" 2>/dev/null | head -1)
if [ -z "$DMG" ]; then
  echo "No DMG found for version ${VERSION}."
  echo "Looked for: src-tauri/target/**/Wiki3_${VERSION}_*.dmg"
  STALE=$(find src-tauri/target -name 'Wiki3_*.dmg' 2>/dev/null | head -3)
  if [ -n "$STALE" ]; then
    echo "Other DMGs present (stale builds):"
    echo "$STALE" | sed 's/^/  /'
  fi
  echo "Run a fresh notarized build first."
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
