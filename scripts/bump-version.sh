#!/usr/bin/env bash
# Bump the Wiki3 app version across every file that has to agree
# (release.sh refuses to publish unless they all match):
#   - package.json
#   - package-lock.json (top-level + packages[""])
#   - src-tauri/Cargo.toml
#   - src-tauri/tauri.conf.json
#   - src-tauri/Cargo.lock (the wiki3-app entry)
#
# Then verifies Cargo.lock is consistent via `cargo update --locked`.
#
# Usage: ./scripts/bump-version.sh <new-version>     e.g. 0.5.2

_wiki3_sourced=0
if [ -n "${ZSH_VERSION:-}" ]; then
  case "${ZSH_EVAL_CONTEXT:-}" in *:file*) _wiki3_sourced=1 ;; esac
elif [ -n "${BASH_VERSION:-}" ]; then
  [ "${BASH_SOURCE[0]}" != "$0" ] && _wiki3_sourced=1
fi
if [ "$_wiki3_sourced" = 1 ]; then
  echo "bump-version.sh: do not source — run as ./scripts/bump-version.sh <ver>" >&2
  return 1 2>/dev/null || exit 1
fi
unset _wiki3_sourced

(
  set -euo pipefail

  if [ $# -ne 1 ]; then
    echo "Usage: $0 <new-version>   e.g. $0 0.5.2" >&2
    exit 2
  fi
  NEW="$1"
  if ! [[ "$NEW" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]]; then
    echo "Refusing: '$NEW' doesn't look like semver (e.g. 0.5.2 or 0.5.2-rc.1)." >&2
    exit 2
  fi

  cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.."

  OLD=$(python3 -c "import json; print(json.load(open('package.json'))['version'])")
  if [ "$OLD" = "$NEW" ]; then
    echo "Version is already $NEW. Nothing to do."
    exit 0
  fi
  echo "Bumping $OLD -> $NEW"

  python3 - "$NEW" <<'PY'
import json, sys
new = sys.argv[1]

with open("package.json") as f: pkg = json.load(f)
pkg["version"] = new
with open("package.json", "w") as f:
    json.dump(pkg, f, indent=2); f.write("\n")

with open("package-lock.json") as f: lock = json.load(f)
lock["version"] = new
if "" in lock.get("packages", {}):
    lock["packages"][""]["version"] = new
with open("package-lock.json", "w") as f:
    json.dump(lock, f, indent=2); f.write("\n")

with open("src-tauri/tauri.conf.json") as f: tconf = json.load(f)
tconf["version"] = new
with open("src-tauri/tauri.conf.json", "w") as f:
    json.dump(tconf, f, indent=2); f.write("\n")
PY

  # Cargo.toml: replace the first version = "..." line in [package].
  python3 - "$NEW" <<'PY'
import re, sys
new = sys.argv[1]
path = "src-tauri/Cargo.toml"
src = open(path).read()
# Only the first version assignment, which lives in [package].
out, n = re.subn(r'(?m)^version\s*=\s*"[^"]+"', f'version = "{new}"', src, count=1)
assert n == 1, "Could not find [package] version in Cargo.toml"
open(path, "w").write(out)
PY

  # Cargo.lock: bump only the wiki3-app entry's version line.
  python3 - "$NEW" <<'PY'
import re, sys
new = sys.argv[1]
path = "src-tauri/Cargo.lock"
src = open(path).read()
pat = re.compile(
    r'(\[\[package\]\]\nname = "wiki3-app"\nversion = ")[^"]+(")',
    re.MULTILINE,
)
out, n = pat.subn(rf'\g<1>{new}\g<2>', src, count=1)
assert n == 1, "Could not find wiki3-app entry in Cargo.lock"
open(path, "w").write(out)
PY

  # Sanity: every reference to the old version in tracked release files
  # should now be gone, and the new version should appear.
  echo
  echo "Updated:"
  for f in package.json package-lock.json src-tauri/Cargo.toml src-tauri/tauri.conf.json src-tauri/Cargo.lock; do
    printf '  %-32s ' "$f"
    grep -m1 -E "(\"version\"|^version)" "$f" | head -1
  done

  # Final consistency check: lockfile must agree with Cargo.toml.
  echo
  echo "Verifying Cargo.lock with --locked..."
  ( cd src-tauri && cargo update --workspace --locked )

  echo
  echo "Done. Suggested next steps:"
  echo "  git diff"
  echo "  git commit -am \"Bump version to $NEW\""
  echo "  ./scripts/build.sh && ./scripts/notarize.sh && npm run release"
)
