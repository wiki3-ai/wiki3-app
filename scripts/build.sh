#!/usr/bin/env bash
# Build the Wiki3 macOS .app, .dmg, and .zip for the current version.
#
# Tauri codesigns the binary during bundling (that step can't be
# deferred), so APPLE_SIGNING_IDENTITY must be exported here.
# Notarization + stapling + verification live in scripts/notarize.sh.
#
# Usage: ./scripts/build.sh [universal|arm64]
#   universal (default) — one fat binary, runs on Apple Silicon and Intel.
#   arm64               — Apple Silicon only; about twice as fast to build,
#                         for iterating locally.
#
# Releases ship universal: Intel Macs are still supported at the 15.0
# deployment target, and one download avoids "which Mac do I have?".
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

  BUILD_KIND="${1:-universal}"
  case "$BUILD_KIND" in
    universal)
      TARGET_TRIPLE=universal-apple-darwin
      BUILD_SCRIPT=tauri:build:universal
      NEEDED_TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
      ;;
    arm64)
      TARGET_TRIPLE=aarch64-apple-darwin
      BUILD_SCRIPT=tauri:build:arm64
      NEEDED_TARGETS=(aarch64-apple-darwin)
      ;;
    *)
      echo "Unknown build kind: $BUILD_KIND (expected 'universal' or 'arm64')" >&2
      exit 2
      ;;
  esac

  # A universal build needs both Rust std libraries; without them cargo
  # fails deep into the build with a confusing message.
  INSTALLED=$(rustup target list --installed)
  for t in "${NEEDED_TARGETS[@]}"; do
    if ! grep -qx "$t" <<<"$INSTALLED"; then
      echo "Rust target $t is not installed. Run:" >&2
      echo "  rustup target add $t" >&2
      exit 1
    fi
  done

  BUNDLE_DIR=src-tauri/target/$TARGET_TRIPLE/release/bundle

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

  npm run "$BUILD_SCRIPT"

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
