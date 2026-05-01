#!/usr/bin/env bash
# Notarize, staple, and verify the Wiki3 .app and .dmg produced by
# scripts/build.sh. Run after build.sh succeeds.
#
# Notarization credentials are read from the keychain profile
# created with:
#   xcrun notarytool store-credentials wiki3-notary \
#     --apple-id "<your-apple-id>" --team-id <TEAM_ID> --password <app-specific-password>
#
# Run as a child process: `./scripts/notarize.sh`. Do NOT source it.

_wiki3_sourced=0
if [ -n "${ZSH_VERSION:-}" ]; then
  case "${ZSH_EVAL_CONTEXT:-}" in *:file*) _wiki3_sourced=1 ;; esac
elif [ -n "${BASH_VERSION:-}" ]; then
  [ "${BASH_SOURCE[0]}" != "$0" ] && _wiki3_sourced=1
fi
if [ "$_wiki3_sourced" = 1 ]; then
  echo "notarize.sh: do not source this script — run it as ./scripts/notarize.sh" >&2
  return 1 2>/dev/null || exit 1
fi
unset _wiki3_sourced

(
  set -euo pipefail

  cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.."

  BUNDLE_DIR=src-tauri/target/aarch64-apple-darwin/release/bundle
  NOTARY_PROFILE="${NOTARY_PROFILE:-wiki3-notary}"

  APP=$(ls -d "$BUNDLE_DIR"/macos/*.app 2>/dev/null | head -1 || true)
  DMG=$(ls -t "$BUNDLE_DIR"/dmg/*.dmg 2>/dev/null | head -1 || true)
  if [ -z "$APP" ] || [ -z "$DMG" ]; then
    echo "No build artifacts found in $BUNDLE_DIR. Run ./scripts/build.sh first." >&2
    exit 1
  fi
  ZIP="${APP%.app}.zip"
  if [ ! -f "$ZIP" ]; then
    # build.sh produces this, but recreate if missing.
    ditto -c -k --keepParent "$APP" "$ZIP"
  fi

  echo "Notarizing:"
  echo "  $APP"
  echo "  $DMG"
  echo

  # 1. Notarize the .app via its zip and staple the .app.
  xcrun notarytool submit "$ZIP" \
    --keychain-profile "$NOTARY_PROFILE" \
    --wait
  xcrun stapler staple "$APP"

  # Refresh the zip so it contains the stapled .app.
  rm -f "$ZIP"
  ditto -c -k --keepParent "$APP" "$ZIP"

  # 2. Notarize the DMG and staple it.
  xcrun notarytool submit "$DMG" \
    --keychain-profile "$NOTARY_PROFILE" \
    --wait
  xcrun stapler staple "$DMG"

  # 3. Verify.
  spctl -a -t exec -vv "$APP"
  spctl -a -t open --context context:primary-signature -v "$DMG"

  echo
  echo "Notarized & stapled:"
  echo "  $APP"
  echo "  $DMG"
  echo "  $ZIP"
)
