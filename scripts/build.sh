#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

BUNDLE_DIR=src-tauri/target/aarch64-apple-darwin/release/bundle

# 0. Clear any stale bundle from a previous version so we don't
# accidentally notarize+ship the old DMG. (The Cargo target dir
# itself is kept for incremental compile speed; only the built
# bundle is removed.)
rm -rf "$BUNDLE_DIR"

# 1. Build the signed app + DMG for the current version in
# package.json / Cargo.toml / tauri.conf.json.
npm run tauri:build:arm64

APP=$(ls -d $BUNDLE_DIR/macos/*.app | head -1)
DMG=$(ls -t $BUNDLE_DIR/dmg/*.dmg | head -1)
ZIP="${APP%.app}.zip"

echo "Built: $APP"
echo "       $DMG"
echo

# Notarization credentials are read from the keychain profile created with:
#   xcrun notarytool store-credentials wiki3-notary \
#     --apple-id "<your-apple-id>" --team-id <TEAM_ID> --password <app-specific-password>
NOTARY_PROFILE="${NOTARY_PROFILE:-wiki3-notary}"

# 2. Notarize the .app (zipped, since notarytool wants an archive).
ditto -c -k --keepParent "$APP" "$ZIP"
xcrun notarytool submit "$ZIP" \
  --keychain-profile "$NOTARY_PROFILE" \
  --wait
xcrun stapler staple "$APP"

# 3. Notarize the DMG too.
xcrun notarytool submit "$DMG" \
  --keychain-profile "$NOTARY_PROFILE" \
  --wait
xcrun stapler staple "$DMG"

# 4. Verify.
spctl -a -t exec -vv "$APP"
spctl -a -t open --context context:primary-signature -v "$DMG"
