# Build, sign, notarize, release

Written because this is infrequent enough to forget. There are **two paths**:
local (for testing a real signed build) and CI (the one that actually publishes).

macOS only, Apple Silicon only. There is no Intel build and no Windows build —
`build-macos.yml` produces `aarch64-apple-darwin` and that is it.

## TL;DR

```bash
./scripts/bump-version.sh 0.5.7     # all five version files, then verifies Cargo.lock
./scripts/build.sh                  # signs + bundles .app/.dmg/.zip
./scripts/notarize.sh               # notarize, staple, verify
npm run release                     # gh release create + upload assets
```

Or let CI do the last three: **push a `v*` tag**.

## The version has to agree in five places

`release.sh` refuses to publish unless they match, precisely because the DMG
filename embeds the version Tauri saw at build time (`Wiki3_<ver>_aarch64.dmg`) —
so a mismatch means you ship a stale artifact under a new tag.

| File | Field |
|---|---|
| `package.json` | `version` |
| `package-lock.json` | `version` and `packages[""].version` |
| `src-tauri/Cargo.toml` | `[package] version` |
| `src-tauri/tauri.conf.json` | `version` |
| `src-tauri/Cargo.lock` | the `wiki3-app` entry's `version` |

`bump-version.sh` edits all five and then runs `cargo update --workspace --locked`
to prove the lockfile agrees.

## Local path

### 1. `./scripts/build.sh`

Run it as a child process; it refuses to run when sourced (that guard exists in
both `build.sh` and `notarize.sh`, and it is deliberate — sourcing breaks the
`cargo clean` / `cd` behaviour).

It:

1. Exports `APPLE_SIGNING_IDENTITY` (defaulting to the Developer ID Application
   identity) and **fails early if that identity is not in your keychain**.
   Tauri only signs when this is exported — `tauri.conf.json` sets
   `signingIdentity: null` — and if it is missing the bundler produces an
   unsigned binary that notarization later rejects. Better to fail here.
2. `cargo update --workspace --locked` to prove `Cargo.lock` is consistent.
3. `cargo clean -p devcontainer-core -p wiki3-app`. This is not paranoia:
   `devcontainer-core` is a **path dependency on a sibling checkout**, so
   Cargo's fingerprints can miss a change made in the other repo and you would
   ship a binary built against the old crate.
4. Deletes the existing bundle directory, so a stale DMG cannot be mistaken for
   this build.
5. `npm run tauri:build:arm64` (= `tauri build --target aarch64-apple-darwin`).
6. Produces a `.zip` next to the `.app` with `ditto -c -k --keepParent`, because
   `notarytool` wants an archive.

### 2. `./scripts/notarize.sh`

Needs a keychain profile made once:

```bash
xcrun notarytool store-credentials wiki3-notary \
  --apple-id "<your-apple-id>" --team-id <TEAM_ID> --password <app-specific-password>
```

Override with `NOTARY_PROFILE=<name>` if you used a different profile.

It submits the **zip**, staples the **.app**, rebuilds the zip so it contains the
stapled app, then submits and staples the **DMG**. Finally it verifies both with
`spctl`. Both submissions use `--wait`, so this blocks for a few minutes.

The order matters: notarizing the DMG alone does not cover the app inside it,
and the zip must be regenerated after stapling or it carries an unstapled app.

### 3. `npm run release`

`./scripts/release.sh` — `gh release create`, or `gh release upload --clobber`
if the tag already exists. Use `npm run release:draft` first if you want to
check the assets before anyone can see them.

Guards: version agreement across the five files, and the DMG must match this
version exactly (it prints any stale DMGs it found instead).

## CI path — the one that publishes

`.github/workflows/build-macos.yml`, on `push: tags: ["v*"]` or manual dispatch.

- `macos-15` runner.
- Imports the signing certificate from `APPLE_CERTIFICATE` /
  `APPLE_CERTIFICATE_PASSWORD` secrets into a temporary keychain.
- Writes the App Store Connect API key from `APPLE_API_KEY_CONTENT` to
  `~/private_keys/AuthKey_<id>.p8` — **API key, not keychain profile**, which is
  why CI does not need `notarytool store-credentials`.
- `npx tauri build --target aarch64-apple-darwin` with `APPLE_SIGNING_IDENTITY`.
- `xcrun notarytool submit --key ... --wait`, then `stapler staple`.
- Uploads the DMG as an artifact, and attaches it to the GitHub release when the
  ref is a tag.
- Deletes the keychain in an `always()` step.

Required secrets: `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`,
`APPLE_API_KEY_CONTENT`, `APPLE_API_KEY`, `APPLE_API_ISSUER`,
`APPLE_SIGNING_IDENTITY`.

**So the short version of "how do I release":** bump, commit, push the branch,
then push a `v*` tag. Do it locally first if you want to run the app before
anyone else gets it.

## Footguns

- **The engine bundle is a checked-in build artifact.** If you changed anything
  in `devcontainers-cli`, rebuild it *and hand-copy* `devcontainer-engine.js`
  **and** `.map` into `src/public/` before building the app. Nothing automates
  this, and forgetting it ships an app whose parser is a version behind its
  Rust. See [devcontainer-engine.md](devcontainer-engine.md).
- **`devcontainer-core` is a sibling checkout.** If that repo has uncommitted
  changes, you will build them. Commit and push there first.
- **A tag push is the publish button.** Nothing else publishes.
- **Notarization needs a network round trip to Apple** and will fail on a
  corp/VPN host that filters it; both scripts use `--wait` so the failure is at
  least explicit.
- The `Info.plist` is generated from `tauri.conf.json`; entitlements come from
  `src-tauri/Entitlements.plist`.

## Verifying a build without releasing it

```bash
codesign -dvv src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Wiki3.app
spctl -a -t exec -vv  src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Wiki3.app
spctl -a -t open --context context:primary-signature -v <the>.dmg
```

`release.sh` also greps `codesign -dvv` for `Authority=Developer ID` and labels
the release notes `signed` or `unsigned`, so an unsigned build is at least
visible rather than silent.
