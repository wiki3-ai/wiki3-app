#!/usr/bin/env bash
# Build the Wiki3 macOS .app, .dmg, and .zip for the current version.
#
# Tauri codesigns the binary during bundling (that step can't be
# deferred), so APPLE_SIGNING_IDENTITY must be exported here.
# Notarization + stapling + verification live in scripts/notarize.sh.
#
# Run as a child process: `./scripts/build.sh`. Do NOT source it.

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

  # Tauri only signs the binary when APPLE_SIGNING_IDENTITY is
  # exported (tauri.conf.json sets `signingIdentity: null`). If
  # it's missing, the bundler silently produces an unsigned binary
  # that Apple's notary service later rejects.
  : "${APPLE_SIGNING_IDENTITY:=Developer ID Application: JAMES PAUL WHITE (8URBCZ87DT)}"
  export APPLE_SIGNING_IDENTITY
  if ! security find-identity -v -p codesigning | grep -qF "$APPLE_SIGNING_IDENTITY"; then
    echo "Signing identity not found in keychain: $APPLE_SIGNING_IDENTITY" >&2
    echo "Run: security find-identity -v -p codesigning" >&2
    exit 1
  fi
  echo "Signing identity: $APPLE_SIGNING_IDENTITY"

  # Verify Cargo.lock is in sync with Cargo.toml. `--locked` fails
  # fast if the resolver would change anything.
  ( cd src-tauri && cargo update --workspace --locked )

  # Force a fresh rebuild of devcontainer-core so any change to its
  # sources in the sibling checkout is picked up even if Cargo's
  # fingerprint heuristics miss it.
  ( cd src-tauri && cargo clean -p devcontainer-core -p wiki3-app )

  # Clear any stale bundle from a previous version so we don't
  # accidentally ship the old DMG.
  rm -rf "$BUNDLE_DIR"

  npm run tauri:build:arm64

  APP=$(ls -d $BUNDLE_DIR/macos/*.app | head -1)
  DMG=$(ls -t $BUNDLE_DIR/dmg/*.dmg | head -1)
  ZIP="${APP%.app}.zip"

  # Produce the .zip alongside the .app (notarytool needs an archive).
  ditto -c -k --keepParent "$APP" "$ZIP"

  echo
  echo "Built:"
  echo "  $APP"
  echo "  $DMG"
  echo "  $ZIP"
  echo
  echo "Next: ./scripts/notarize.sh"
)
