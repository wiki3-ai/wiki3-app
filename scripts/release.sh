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

# The tag must name the commit the artifacts were built from. Left to
# itself `gh release create` targets the repository's *default branch*,
# which is how v0.6.0 came to be tagged on a commit 38 behind the code it
# shipped. Resolve the commit once, here, and pass it explicitly.
RELEASE_COMMIT=$(git rev-parse HEAD)

# The artifacts were produced by build.sh from the working tree. If a
# tracked file is modified, no commit describes what was actually built
# and the tag would point at code that does not match the DMG. Untracked
# files are ignored deliberately: this repo carries stray screenshots and
# log files that have nothing to do with a release.
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  echo "Tracked files have uncommitted changes:"
  git status --short --untracked-files=no | sed 's/^/  /'
  echo "Commit or stash them so the tag matches what was built."
  exit 1
fi

# Find DMG matching this version. The bundle filename embeds the
# version Tauri saw at build time (`Wiki3_<ver>_<arch>.dmg`), so
# requiring an exact match catches the "I bumped versions but didn't
# rebuild" footgun.
#
# Releases are universal; a leftover arm64 DMG from a local iteration
# build must not be picked up by accident, so the arch segment is
# matched explicitly rather than globbed.
ARCH_SUFFIX="${ARCH_SUFFIX:-universal}"
DMG=$(find src-tauri/target -name "Wiki3_${VERSION}_${ARCH_SUFFIX}.dmg" 2>/dev/null | head -1)
if [ -z "$DMG" ]; then
  echo "No ${ARCH_SUFFIX} DMG found for version ${VERSION}."
  echo "Looked for: src-tauri/target/**/Wiki3_${VERSION}_${ARCH_SUFFIX}.dmg"
  STALE=$(find src-tauri/target -name 'Wiki3_*.dmg' 2>/dev/null | head -3)
  if [ -n "$STALE" ]; then
    echo "Other DMGs present (stale builds):"
    echo "$STALE" | sed 's/^/  /'
  fi
  echo "Run a fresh notarized build first."
  exit 1
fi

# The notarized .app zip lives next to the .app bundle. Tauri
# names it `Wiki3.app` (no version), and `build.sh` produces
# `Wiki3.zip` next to it via `ditto -c -k --keepParent`. Rename
# on upload so the asset filename is unambiguous on the release
# page.
ARCH_SUFFIX="${ARCH_SUFFIX:-universal}"
case "$ARCH_SUFFIX" in
  universal) TARGET_TRIPLE=universal-apple-darwin ;;
  aarch64)   TARGET_TRIPLE=aarch64-apple-darwin ;;
  *) echo "Unknown ARCH_SUFFIX: $ARCH_SUFFIX (expected 'universal' or 'aarch64')" >&2; exit 2 ;;
esac
APP_DIR="$REPO_ROOT/src-tauri/target/$TARGET_TRIPLE/release/bundle/macos"
SRC_ZIP="$APP_DIR/Wiki3.zip"
if [ ! -f "$SRC_ZIP" ]; then
  echo "No notarized app zip found at $SRC_ZIP."
  echo "Run scripts/build.sh first (it produces the zip during notarization)."
  exit 1
fi
ZIP="$APP_DIR/Wiki3_${VERSION}_${ARCH_SUFFIX}.zip"
cp -f "$SRC_ZIP" "$ZIP"

# Check signing status
SIGNED="unsigned"
if CODESIGN_OUT=$(codesign -dvv "$APP_DIR/Wiki3.app" 2>&1); then
  if echo "$CODESIGN_OUT" | grep -q "Authority=Developer ID"; then
    SIGNED="signed"
  fi
fi

DMG_NAME=$(basename "$DMG")
DMG_SIZE=$(du -h "$DMG" | cut -f1 | xargs)
ZIP_NAME=$(basename "$ZIP")
ZIP_SIZE=$(du -h "$ZIP" | cut -f1 | xargs)

echo "Release: $TAG"
echo "Commit:  $RELEASE_COMMIT"
echo "Assets:  $DMG_NAME ($DMG_SIZE, $SIGNED)"
echo "         $ZIP_NAME ($ZIP_SIZE, $SIGNED)"
echo ""

# Check if release already exists
if gh release view "$TAG" &>/dev/null; then
  echo "Release $TAG already exists. Uploading assets..."

  # A tag can point somewhere other than the code just built — that is
  # exactly how v0.6.0 shipped with its tag on the default branch. Say so
  # loud enough to notice before the assets land under the wrong source.
  EXISTING_COMMIT=$(git rev-list -n1 "$TAG" 2>/dev/null || true)
  if [ -n "$EXISTING_COMMIT" ] && [ "$EXISTING_COMMIT" != "$RELEASE_COMMIT" ]; then
    echo "WARNING: tag $TAG points at ${EXISTING_COMMIT:0:7}, not the current HEAD ${RELEASE_COMMIT:0:7}."
    echo "         The release's source code will not match these assets."
  fi

  gh release upload "$TAG" "$DMG" "$ZIP" --clobber
else
  TITLE="Wiki3 ${TAG}"
  NOTES="macOS (Universal — Apple Silicon and Intel, Sequoia 15.0+) — ${SIGNED}"

  echo "Creating release $TAG..."
  gh release create "$TAG" \
    --title "$TITLE" \
    --notes "$NOTES" \
    --target "$RELEASE_COMMIT" \
    $DRAFT \
    "$DMG" "$ZIP"
fi

echo ""
echo "Done. https://github.com/$(gh repo view --json nameWithOwner -q .nameWithOwner)/releases/tag/$TAG"
