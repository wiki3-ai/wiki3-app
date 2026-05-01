#!/usr/bin/env bash
# Build, sign, and notarize the Wiki3 macOS app + DMG.
#
# Run as a child process: `./scripts/build.sh`.
# Do NOT source it (`. scripts/build.sh`) — when sourced, the script
# runs inside your login shell, so Ctrl-C or any failure kills the
# terminal itself ("terminated with exit code 130/65").

# Refuse to run when sourced, regardless of zsh vs bash.
_wiki3_sourced=0
if [ -n "${ZSH_VERSION:-}" ]; then
  case "${ZSH_EVAL_CONTEXT:-}" in *:file*) _wiki3_sourced=1 ;; esac
elif [ -n "${BASH_VERSION:-}" ]; then
  [ "${BASH_SOURCE[0]}" != "$0" ] && _wiki3_sourced=1
fi
if [ "$_wiki3_sourced" = 1 ]; then
  echo "build.sh: do not source this script — run it as ./scripts/build.sh" >&2
  return 1 2>/dev/null || exit 1
fi
unset _wiki3_sourced

(
  set -euo pipefail

  cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.."

BUNDLE_DIR=src-tauri/target/aarch64-apple-darwin/release/bundle

# 0. Codesigning. Tauri only signs the binary when
# APPLE_SIGNING_IDENTITY is exported (tauri.conf.json sets
# `signingIdentity: null`). If it's missing, the build silently
# produces an unsigned binary that Apple's notary service rejects
# with "signature is invalid / no hardened runtime / no secure
# timestamp". Default to the team's Developer ID if the caller
# hasn't set one explicitly.
: "${APPLE_SIGNING_IDENTITY:=Developer ID Application: JAMES PAUL WHITE (8URBCZ87DT)}"
export APPLE_SIGNING_IDENTITY
if ! security find-identity -v -p codesigning | grep -qF "$APPLE_SIGNING_IDENTITY"; then
  echo "Signing identity not found in keychain: $APPLE_SIGNING_IDENTITY" >&2
  echo "Run: security find-identity -v -p codesigning" >&2
  exit 1
fi
echo "Signing identity: $APPLE_SIGNING_IDENTITY"

# 0a. Verify Cargo.lock is in sync with Cargo.toml. `--locked`
# fails fast if the resolver would change anything; that catches
# a `Cargo.toml` edit (e.g. bumped path-dep version) that wasn't
# committed alongside its lockfile update.
( cd src-tauri && cargo update --workspace --locked )

# 0b. Force a fresh rebuild of devcontainer-core so any change to
# its sources in the sibling checkout is picked up even if Cargo's
# fingerprint heuristics miss it (rare, but cheap insurance for
# release builds).
( cd src-tauri && cargo clean -p devcontainer-core -p wiki3-app )

# 0c. Clear any stale bundle from a previous version so we don't
# accidentally notarize+ship the old DMG. (The rest of the Cargo
# target dir is kept for incremental compile speed.)
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
)
