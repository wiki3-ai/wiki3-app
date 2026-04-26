# Integration Testing — Plan & Constraints

Written 2026-04-26 after the `.quit()` menu-item shutdown bug, where
the predefined macOS Quit item bypassed Tauri's `RunEvent::ExitRequested`
and skipped our container cleanup. Manual verification missed it
twice. This doc captures what we'd need to catch a class of bugs like
that automatically and why we haven't built it yet.

## What we're trying to cover

The end-to-end **quit / cleanup** path:

1. App launches with a wiki whose `autostart_container` is true.
2. `start_site` runs: detects `container`, marks
   `started_service` if it had to start the service, marks
   `touched_containers`, inserts a `RunningSite`.
3. User hits **⌘Q** (or `Wiki3 → Quit Wiki3`, or chooses Quit from
   the Dock menu, or clicks the red close button when configured to
   exit-on-close).
4. `RunEvent::ExitRequested` fires; we `prevent_exit`, emit the
   "shutting down" overlay, run `shutdown_all`, conditionally pop
   the foreign-containers dialog, then `handle.exit(0)`.
5. By process exit, no `wiki3-*` containers are running, and the
   Apple Container service is stopped iff *we* started it and there
   are no foreign workloads.

The bug we missed: step 3 via the predefined macOS Quit menu routed
through `NSApplication.terminate(_:)` and skipped step 4 entirely.

## Options considered

### A. Pure Rust integration test of `shutdown_all`

Refactor `shutdown_all` to take a small `ContainerCli` trait
(`stop_container`, `list_running_container_names`, `stop_service`)
instead of calling `apple_container::*` directly. Write
`tests/shutdown.rs` against an in-memory fake.

**Catches:** orphan sweep, foreign-container detection, buildkit
allowlisting, `service_started_by_us` bookkeeping, idempotency.
**Does not catch:** menu wiring, `RunEvent::ExitRequested` plumbing,
the `.quit()` bug specifically.
**Cost:** small. ~1 day. No external deps.

### B. Rust test running real `container` CLI when present

Same as A but with a feature-gated test that actually shells out to
`container` on the developer machine to validate output parsing
hasn't drifted from upstream.

**Catches:** Apple Container CLI format drift.
**Cost:** slow (~30s+), needs Apple Container installed, conflicts
with whatever the dev is running locally. Only useful as a manual
preflight, not on CI.

### C. WebDriver E2E via `tauri-driver`

The "right" answer for the menu bug — actually boot the app, send a
keystroke, observe cleanup.

**Blocked.** Per the
[Tauri 2 WebDriver docs](https://v2.tauri.app/develop/tests/webdriver/):

> On desktop, only Windows and Linux are supported due to macOS not
> having a WKWebView driver tool available. iOS and Android work
> through Appium 2, but the process is not currently streamlined.

So tauri-driver is a non-starter for our macOS-only desktop quit
path.

### D. AppleScript / `osascript` driving the real `.app`

Build the bundle (`npm run tauri:build` or a debug equivalent),
launch it with a fake `container` shim earlier on `PATH`, then
script the quit:

```bash
osascript -e 'tell application "System Events" to tell process "Wiki3" to keystroke "q" using command down'
# or
osascript -e 'tell application "Wiki3" to quit'
```

After the process exits, inspect a log file written by the fake
`container` shim to assert:

- `container stop wiki3-site-<tag>` was invoked
- `container system stop` was invoked (when the test set up the
  "we started the service" precondition)
- `container ls` was queried at least twice (orphan sweep + foreign
  check)

**Catches:** The whole quit path, including menu wiring. Would have
caught the `.quit()` bug.
**Cost:** macOS-only; needs Accessibility permission for
`System Events` keystroke (cleaner: prefer `tell application … to
quit` which uses Apple Events and doesn't need Accessibility); needs
a built `.app` bundle (slow first time, fast on rebuilds);
flaky-prone (timing of dialog dismissal, focus stealing in CI);
won't run on hosted GitHub macOS runners without extra setup
because they lack a logged-in GUI session by default — but `tauri`'s
own CI examples show that `macos-latest` runners *can* run AppKit
GUIs headfully. Worth verifying.

**Implementation sketch:**

```
tests/e2e/
  fake-container.sh        # logs argv to $W3_FAKE_LOG, fakes service-state
  shutdown.test.sh         # boots .app with PATH=tests/e2e:$PATH,
                           # waits for ready file, sends quit, checks log
```

We'd add a tiny "ready file" the app touches once setup() finishes
so the test isn't racing against autostart. Same for "exited" — the
test waits for the process to be gone, then asserts on the log.

### E. Tauri `MockRuntime` test

Build a stripped-down `tauri::Builder` in a `#[test]` and drive it
with `tauri::test::MockRuntime`.

**Blocked.** `MockRuntime` doesn't fire `RunEvent::ExitRequested`
and doesn't model native menus, so it can't reach the code we want
to cover.

## Recommendation

When we come back to this:

1. **First:** Do **A** — refactor `shutdown_all` to take a
   `ContainerCli` trait and write Rust tests for the decision
   matrix (`{started_service, foreign_present, orphans_present} →
   {stopped_containers, service_stopped, foreign_containers}`).
   Cheap, fast, runs on CI, covers most regressions in the file
   that's most likely to drift.
2. **Then:** Do **D** — one AppleScript-driven smoke test that
   boots the real bundle with a fake `container` on PATH, sends
   `tell application "Wiki3" to quit`, and asserts the fake CLI's
   recorded calls. One test is enough; the value is *menu wiring*,
   not coverage breadth.
3. Skip B (manual preflight only), C (not supported on macOS), E
   (can't reach the code under test).

## Hooks already in place that make D easier

- `WIKI3_DEV_URL` env var is already plumbed to override the
  loaded site URL — useful for pointing the test app at a static
  file:// fixture so we don't depend on network.
- `apple_container::detect()` already takes the `container` binary
  off `$PATH` (no hardcoded path), so a `PATH`-prefix shim drops in
  cleanly.
- `LocalSiteManager::has_pending_cleanup` and the
  `wiki3://shutdown-begin` event give a stable signal we can
  observe from a fake binary's log.

## What NOT to do

- **Don't** over-mock the Rust side. We already have ~92 unit tests;
  more pure-logic tests on top of the trait extraction in (A) is
  the right amount.
- **Don't** chase the WebDriver path on macOS unless Apple ships a
  WKWebView driver. As of 2026-04, they have not.
- **Don't** wire the AppleScript test into `npm test` / `cargo
  test` blindly — make it `npm run test:e2e` so contributors
  without the bundle built / Accessibility granted aren't blocked.
