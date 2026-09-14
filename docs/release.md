# Build, sign, notarize, release

Written because this is infrequent enough to forget. The macOS release is cut
**locally**, by the scripts below — that is the only path that works today.
`.github/workflows/build-macos.yml` is parked and cannot run; see the CI section
further down.

macOS releases are **universal** (`universal-apple-darwin`), so one download
covers Apple Silicon and Intel; `minimumSystemVersion` is 15.0 (Sequoia), which is
what still lets Intel Macs in.

The **Windows** installer is built by CI, because it cannot be built on a Mac. See
"Windows installer" below.

## TL;DR

```bash
./scripts/bump-version.sh 0.5.7     # all five version files, then verifies Cargo.lock
./scripts/build.sh                  # universal by default; signs + bundles .app/.dmg/.zip
./scripts/notarize.sh               # notarize, staple, verify
npm run release                     # gh release create + upload assets
```

Needs a **Developer ID Application identity in your login keychain** — check with
`security find-identity -v -p codesigning`. `build.sh` refuses to run without it, on
purpose (see below). Note that it is not on every machine; if it is missing, this is
not a "fix the script" problem.

`./scripts/build.sh arm64` builds an Apple Silicon-only app in about half the time,
for iterating locally. That is not a release artifact — `release.sh` will not find
the DMG it produces (see below).

## Setting up a new Mac

Signing needs a **certificate *and its private key***. This is the part that
catches people out: a `.cer` downloaded from developer.apple.com contains only the
certificate, so it imports fine, shows up in Keychain Access, and still cannot sign
anything. `security find-identity -v -p codesigning` will not list it.

The portal's certificate list is therefore the wrong place to start on a new Mac —
downloading the existing certificate onto this machine pairs it with nothing. Either
bring the key across, or create a new pair here.

### Which certificate type

The page at <https://developer.apple.com/account/resources/certificates> lists
several, and only one is right for this:

- **Developer ID Application** — what you need. It is what signs an app you ship
  outside the App Store, and what notarization accepts.
- *Developer ID Installer* — for signing `.pkg` installers. Not this; we ship a DMG.
- *Apple Development* / *Mac Development* — for running debug builds on your own
  devices via Xcode. Not distributable.
- *Apple Distribution* / *Mac Installer Distribution* — for App Store submissions.

### Easiest: let Xcode manage it

Xcode → Settings → Accounts → sign in with the Apple ID → select the team →
**Manage Certificates…** → `+` → **Developer ID Application**.

This creates a fresh key pair and certificate on this machine and installs both —
including the certificate-signing-request dance, which is why it is the least
error-prone route. The name will be `Developer ID Application: <Your Name>
(<TEAMID>)`, which is what `build.sh` expects.

### A new certificate is not the old one

Creating a certificate here produces a **new key pair and a new certificate**, not a
copy of whatever the other Mac holds. It is not the *same* certificate.

It looks identical, though, because the identity string is derived from your name and
team — `Developer ID Application: JAMES PAUL WHITE (8URBCZ87DT)` — so the serial
number and key differ while the text does not. `find-identity` shows only the text,
which is why this is easy to get wrong.

Practically, for this project it does not matter:

- Any valid *Developer ID Application* certificate for the team signs and notarizes
  fine; Apple does not require the same one across releases.
- **Already-released builds are unaffected.** Users' Gatekeeper checks were satisfied
  when they downloaded, and the notarization ticket is stapled to the build, not to
  the certificate. Nothing installed breaks.
- The two can coexist. Signing with whichever is present is fine.

It matters in two places:

- **If you specifically need the previous identity** — for example to reuse a `.p12`
  already stored somewhere, or to keep an existing CI secret valid — then export it
  from the machine that has the key rather than creating a new one.
- **CI secrets hold one specific `.p12`.** A newly created certificate means
  re-exporting and replacing `APPLE_CERTIFICATE`. (Moot here: this repo currently has
  no secrets at all.)

Since both certificates share a name, the identity string cannot tell them apart. To
see what actually signed a build:

```bash
codesign -dvvv <path-to>Wiki3.app 2>&1 | grep -E 'Authority|TeamIdentifier'
```

### Or: create one from the portal with a CSR

If you do not have Xcode, the portal route works and is the manual version of the
above. The key is generated here and never leaves the machine:

1. Keychain Access → menu **Keychain Access** → **Certificate Assistant** →
   **Request a Certificate From a Certificate Authority…**
2. Enter your Apple ID email, leave *CA Email Address* blank, choose **Saved to
   disk**, and save the `.certSigningRequest`.
3. On the portal, **create** a new **Developer ID Application** certificate (not
   download an existing one) and upload that CSR.
4. Download the resulting `.cer` and double-click it to import.

It pairs automatically with the private key Keychain Access just created, so
`find-identity` will list it. Apple caps how many certificates of each type an
account can hold; if the portal refuses, revoke an unused one rather than deleting
anything locally.

### Or import an existing `.p12`

There is no standard location for a `.p12` — it is not a file macOS maintains.
The private key lives inside the old machine's *login keychain* as an opaque
entry, and a `.p12` only exists once somebody exports it. So the first question is
whether a `.p12` was ever exported; if not, there is nothing to copy, and you
either export one now or create a fresh identity with Xcode above.

**If you had exported one before**, it is wherever you saved it. On the old Mac:

```bash
find ~ -maxdepth 4 -name '*.p12' 2>/dev/null
mdfind -name .p12
```

Downloads, Desktop, and any keys/secrets folder are the usual suspects; check
1Password/Keychain notes too, since it is a credential rather than a document.

**To create one now**, on the machine that has the private key:

```bash
security export -k ~/Library/Keychains/login.keychain-db \
  -t identities -f pkcs12 -o ~/Desktop/wiki3-developer-id.p12
```

It prompts for a password to protect the export. Note that it exports **all**
identities in the keychain, not just the Developer ID one — prefer the GUI route
below if you want to be selective.

Or precisely, via the GUI: Keychain Access → **My Certificates** (not
"Certificates" — that category will not offer to export a private key) → select the
Developer ID Application identity → right-click → **Export** → `.p12` with a
password.

**Transfer it over an encrypted channel** — AirDrop or a USB stick. It is your
private key; treat it as such. Then import it here:

```bash
security import <path-to>.p12 \
  -k ~/Library/Keychains/login.keychain-db \
  -T /usr/bin/codesign -T /usr/bin/security
```

It prompts for the `.p12` password. Deliberately no `-P <password>`: that would put
the password in your shell history and in `ps`.

**Do not file it under `~/.ssh`.** That is where a `.p12` tends to get dropped,
because it is the directory people think of for keys — but it is a *signing* key
with nothing to do with SSH, the name invites exactly that confusion, and anything
that backs up or audits `~/.ssh` will pick it up. Put it somewhere deliberate, or
delete it once it is in the keychain and re-export when CI needs it. Check it is
what you think it is before importing:

```bash
xxd -l 16 <path-to>.p12      # PKCS#12 starts with 3082 (DER SEQUENCE)
```

The same `.p12` is what CI needs, base64-encoded, as the `APPLE_CERTIFICATE` secret
— so exporting one is worth doing regardless of which machine you build on.

### Then, the step everyone forgets

`codesign` needs non-interactive access to the private key. Without this, signing
works when you run it by hand and fails from a script or CI with an
`errSecInternalComponent` / "user interaction is not allowed" error:

```bash
security set-key-partition-list -S apple-tool:,apple: -s \
  ~/Library/Keychains/login.keychain-db
```

It prompts for your login password. (`build-macos.yml` runs the equivalent step in
CI.)

**Do this unconditionally — do not try to prove it unnecessary.** Importing with
`-T /usr/bin/codesign` looks like it makes this redundant, and a quick check can
appear to confirm that: signing a throwaway file with `codesign -s <identity>` will
succeed. Signing a full `.app` bundle then fails with `errSecInternalComponent`
anyway. That is exactly what happened here — a passing probe is not evidence, because
a one-off signing operation can succeed on an ACL grant that does not persist to the
next one.

### Verify

```bash
security find-identity -v -p codesigning
```

You want a line ending in `"Developer ID Application: <Your Name> (<TEAMID>)"`. If
that prints `0 valid identities found`, `build.sh` will refuse to run — correctly,
because Tauri would otherwise bundle an unsigned app that notarization later
rejects.

**A Team ID does not mean a team account.** Every Apple Developer account carries
one, individual accounts included — for an individual, the certificate's `O` field is
simply your own name. So a personal account still gets `(XXXXXXXXXX)` in the identity
string. To see what the certificate actually says, and when it lapses:

```bash
security find-certificate -c "Developer ID Application" -p \
  | openssl x509 -noout -subject -dates
```

`subject=` is what `find-identity` shows; `notAfter=` is the expiry. Developer ID
certificates are short-lived (about a year), and an expired one stops signing the day
it lapses — worth a calendar note rather than discovering it during a release.

If your identity string differs from the default in `build.sh`, override it rather
than editing the script:

```bash
APPLE_SIGNING_IDENTITY="Developer ID Application: ..." ./scripts/build.sh
```

### The notary credential is separate

Signing and notarizing use different credentials, and a new machine needs the second
one too. Create an app-specific password at <https://appleid.apple.com> → Sign-In and
Security → App-Specific Passwords, then:

```bash
xcrun notarytool store-credentials wiki3-notary \
  --apple-id "<your-apple-id>" --team-id <TEAMID>
```

It prompts for the app-specific password, and stores it in the keychain.
`notarize.sh` reads the profile named `wiki3-notary` (override with
`NOTARY_PROFILE=<name>`).

## The version has to agree in five places

`release.sh` refuses to publish unless they match, precisely because the DMG
filename embeds the version Tauri saw at build time (`Wiki3_<ver>_universal.dmg`) —
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

## CI path — parked

`.github/workflows/build-macos.yml` now runs on `workflow_dispatch` only. It used
to trigger on `v*` tags, which meant publishing any release produced a failing
check from a job that never had a chance of passing. It is parked rather than
deleted because it is the natural starting point if the release is ever automated.

**It has never succeeded, because the repo has no secrets.** `gh secret list -R
wiki3-ai/wiki3-app` returns *"no secrets found"*, so the certificate-import step
fails immediately. The only run, on the `v0.4.0` tag, failed in 22 seconds. That
import step has therefore never executed once, so even with the secrets present
the workflow should be treated as unverified rather than ready.

To make it work, add these as repository secrets:

| Secret | What it is |
|---|---|
| `APPLE_CERTIFICATE` | Developer ID Application cert, base64-encoded `.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | its password |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: NAME (TEAMID)` |
| `APPLE_API_KEY` | App Store Connect key **id** (also used as the `.p8` filename stem) |
| `APPLE_API_KEY_CONTENT` | the `.p8` contents |
| `APPLE_API_ISSUER` | the App Store Connect issuer UUID |

**Credential trap.** This workflow notarizes with an App Store Connect **API key**
(the `.p8`). `scripts/notarize.sh` notarizes with an app-specific **password**
stored in the keychain as the `wiki3-notary` profile. Those are different
credential types, so the local credential that demonstrably works cannot simply be
copied into CI — it is not just a matter of encoding it into a secret.

Once they exist, the job does:

- `macos-15` runner.
- Imports the certificate from `APPLE_CERTIFICATE` / `APPLE_CERTIFICATE_PASSWORD`
  into a temporary keychain.
- Writes the App Store Connect API key from `APPLE_API_KEY_CONTENT` to
  `~/private_keys/AuthKey_<id>.p8` — **API key, not keychain profile**, which is
  why CI does not need `notarytool store-credentials`.
- `npx tauri build --target universal-apple-darwin` with `APPLE_SIGNING_IDENTITY`.
  Setup-Rust installs both `aarch64-apple-darwin` and `x86_64-apple-darwin`; a
  universal build needs both std libraries or it fails deep into the build.
- `xcrun notarytool submit --key ... --wait`, then `stapler staple`.
- Uploads the DMG as the `Wiki3-macOS-universal` artifact, and attaches it to the
  GitHub release when the ref is a tag.
- Deletes the keychain in an `always()` step.

Because it has never run green, **do not assume a tag push publishes a macOS
build** — verify the run before telling anyone a release is out.

## Windows installer

`.github/workflows/build-windows.yml` is the one build that has to be CI. You
cannot build a Windows installer on a Mac, and `cargo check --target
x86_64-pc-windows-msvc` stops at the build script because it needs a Windows
resource compiler — so a local check proves nothing about bundling.

It needs **no secrets**: there is no signing or notarization step. The installer
is unsigned, so Windows shows a SmartScreen warning on first run, which is
accepted for now.

On a `v*` tag push it builds the NSIS installer and attaches
`Wiki3_<version>_x64-setup.exe` to the release. On `docker`/`main` pushes and pull
requests it builds the same thing but uploads it as the `Wiki3-Windows-x64`
artifact instead of touching a release.

**Its trigger reads the workflow from the tagged commit, so where the tag points
matters.** A tag on a commit that predates `build-windows.yml` runs nothing at all —
no workflow file exists there to run — and adding a workflow later cannot rescue an
already-placed tag. v0.6.0 is in exactly that position: its tag landed on `main` at
`8809bea` (see the footgun below), which is why that release's `.exe` had to be
built and attached by hand.

To build one by hand and fetch it:

```bash
gh workflow run build-windows.yml -R wiki3-ai/wiki3-app
gh run watch -R wiki3-ai/wiki3-app
gh run download -R wiki3-ai/wiki3-app -n Wiki3-Windows-x64 -D /tmp/wiki3-win
```

That is how the Windows asset was added to the v0.6.0 draft, which predates the
tag-triggered path. `src-tauri/tauri.windows.conf.json` retargets the bundle from
the macOS `app`/`dmg` pair to `nsis`.

Both Rust jobs in CI run `cargo clippy --all-targets -- -D warnings`. Worth
knowing when touching macOS-only code: an import used only inside
`#[cfg(target_os = "macos")]` is a hard error on the Windows job, even though it
compiles fine here.

## Footguns

- **A tag names a commit, and `gh release create` picks one you did not choose.**
  Without `--target` it targets the repository's **default branch**, not the commit you
  just built. That is how v0.6.0 shipped tagged on `8809bea` — which is `v0.5.6`, from
  2026-05-15, 38 commits behind — so the release's "Source code" download was not the
  released code. `release.sh` now resolves `HEAD` once and passes it explicitly, and
  refuses to run if a tracked file is modified, because in that case no commit at all
  describes what was built. Untracked files are ignored on purpose: this repo carries
  stray screenshots and log files.
- **GitHub reads workflow files from the tagged commit, not from the branch tip.** So a
  tag placed on an old commit runs the workflows as they were then — possibly none.
  This is why a correct tag is not just cosmetic.
- **The engine bundle is a checked-in build artifact.** If you changed anything
  in `devcontainers-cli`, rebuild it *and hand-copy* `devcontainer-engine.js`
  **and** `.map` into `src/public/` before building the app. Nothing automates
  this, and forgetting it ships an app whose parser is a version behind its
  Rust. See [devcontainer-engine.md](devcontainer-engine.md).
- **`devcontainer-core` is a sibling checkout.** If that repo has uncommitted
  changes, you will build them. Commit and push there first.
- **The DMG must match the arch you built.** `release.sh` looks for
  `Wiki3_<version>_universal.dmg` by default. An arm64 iteration build leaves an
  `..._aarch64.dmg` that it will deliberately *not* pick up — set
  `ARCH_SUFFIX=aarch64` if that is genuinely what you mean to ship.
- **A tag push publishes the Windows installer, but not the Mac build.** A `v*` tag
  runs `build-windows.yml`, which attaches `Wiki3_<version>_x64-setup.exe` to the
  release for that tag. The macOS DMG is still published locally with
  `npm run release`; the parked workflow contributes nothing. Note that
  `gh release create --draft` does **not** create the tag — publishing the draft
  does — so the Windows build only runs once you publish.
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
