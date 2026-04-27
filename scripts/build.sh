cd /Users/jim/Projects/wiki3-app

BUNDLE_DIR=src-tauri/target/aarch64-apple-darwin/release/bundle
APP=$(ls -d $BUNDLE_DIR/macos/*.app | head -1)
DMG=$(ls -t $BUNDLE_DIR/dmg/*.dmg | head -1)
ZIP="${APP%.app}.zip"

# Notarization credentials are read from the keychain profile created with:
#   xcrun notarytool store-credentials wiki3-notary \
#     --apple-id "<your-apple-id>" --team-id <TEAM_ID> --password <app-specific-password>
NOTARY_PROFILE="${NOTARY_PROFILE:-wiki3-notary}"

# 1. Notarize the .app (zipped, since notarytool wants an archive).
ditto -c -k --keepParent "$APP" "$ZIP"
xcrun notarytool submit "$ZIP" \
  --keychain-profile "$NOTARY_PROFILE" \
  --wait
xcrun stapler staple "$APP"

# 2. Notarize the DMG too.
xcrun notarytool submit "$DMG" \
  --keychain-profile "$NOTARY_PROFILE" \
  --wait
xcrun stapler staple "$DMG"

# 3. Verify.
spctl -a -t exec -vv "$APP"
spctl -a -t open --context context:primary-signature -v "$DMG"
