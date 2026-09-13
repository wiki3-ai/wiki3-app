# Devcontainer parsing — one parser, and how to change it

Last updated 2026-09-13, when the legacy parser was deleted.

`devcontainer.json` is parsed in **one** place: the prebuilt engine bundle in
`src/public/`, built from the upstream `@devcontainers/cli` spec slice. The
result is submitted to `devcontainer-core`'s `LifecycleOrchestrator`, and every
other consumer reads it back from there.

If you only read one section, read [Changing the parsed contract](#changing-the-parsed-contract)
— getting that wrong fails silently.

## TL;DR

| | |
|---|---|
| Parser | prebuilt `src/public/devcontainer-engine.js`, built from the upstream `@devcontainers/cli` spec slice |
| Rust shape | `devcontainer_core::ParsedDevContainer` |
| Runtime | pluggable `ContainerRuntime` (`apple_containers` / `docker` / `podman`) |
| Entry point | `commands_devcontainer.rs::submit_parsed_devcontainer` → `LifecycleOrchestrator` |
| Read back | `LifecycleOrchestrator::parsed_config()` |

The dashboard submits every wiki's config at startup (`submitAllDevcontainers()`
in `src/main.ts`), because the host cannot parse for itself. Without that, the
port panel would be empty for a container created in an earlier session until
the user happened to Start or Restart something.

## What used to be here

There was a second path: an inlined JavaScript module
(`tools/devcontainer_config.{rs,js}`) run in-process by **rquickjs**, driving
the Apple `container` CLI directly to serve the per-wiki **Build / Serve /
Stop** preview. It arrived in this order, and outlived its own replacement:

1. **2026-04-21 `d64f85a`** — "Remove Deno stuff and use rquickjs to run
   devcontainers code". Parsing was inlined so the app shipped no external
   toolchain.
2. **2026-04-29 `f116b12`** — "Adopt devcontainer-core's apple_containers
   helpers". Container work starts moving into the reusable crate.
3. **2026-04-30 `4e51301`** — "Use the devcontainer-cli code from core". The
   prebuilt engine bundle replaces the inlined-JS approach.

The preview feature was simply never migrated off it, so it lingered as a
second opinion about the same file. Its `DevcontainerConfig` was also a
hand-rolled *subset*: it declared `image`, `build`, `forwardPorts`,
`remoteUser` and `customizations`, but **not** `mounts` or `runArgs`, even
though the JS emitted both — serde dropped them silently, with no error. That
is the same class of bug the compatibility gate exists to prevent, and it is
why the path was deleted rather than extended.

Removed: `tools/devcontainer_config.{rs,js}`, `tools/devcontainer_image.rs`,
`wiki/local_site.rs`, the `wiki_build_site` / `wiki_start_container` /
`wiki_stop_container` / `wiki_container_status` /
`wiki_force_stop_container_service` commands, the quit-time teardown and its
"foreign containers" dialog, and the `rquickjs` dependency.

## Apple-specific code that remains, deliberately

Two things still name Apple Container, and both concern *reaching* a
container rather than parsing a config:

- `tools/apple_container.rs` — detection for the Tools dialog, plus
  `inspect_container_ipv4`.
- The port poller's vmnet fallback in `wiki/ports.rs` and `wiki/forwarder.rs`,
  which tunnels loopback to the container's vmnet address. This exists for
  corp-managed macOS hosts where a network filter accept-then-RSTs Apple's
  loopback publish-proxy, leaving the port unreachable on `127.0.0.1` even
  though the container is healthy. It is gated on the effective runtime
  actually being Apple Containers, so Docker and Podman never pay for it.

If Apple Containers is ever dropped, both go with it.

## The engine bundle

`src/public/devcontainer-engine.js` is a **checked-in build artifact**
(~3.9 MB, ~106k lines). The WebView fetches it at runtime as
`/devcontainer-engine.js`; `src/devcontainer-engine.ts` is only a thin
loader around it.

It is built from a *different repository*:

```
# source of truth
devcontainers-cli/frontend/src/devcontainer-engine/index.ts

# build (writes dist/, and copies to frontend/public/)
cd devcontainers-cli && deno task build

# then hand-copy both files into this repo — there is no automation
cp dist/devcontainer-engine.js     wiki3-app/src/public/
cp dist/devcontainer-engine.js.map wiki3-app/src/public/
```

Deno is a documented prerequisite of that repo (`scripts/mac-setup.sh`
installs it via Homebrew). Useful guards:

```
deno task check                     # typecheck spec slice + engine source
deno task build:check-no-node-builtins   # CI guard: no fs/child_process in the bundle
```

**The parity trap.** The Rust host deserialises whatever the bundle posts,
so if you add a field on only one side nothing errors — the field simply
arrives as `None`/`default` and the feature silently does nothing. Any
change to the parsed shape must touch all four of:

1. the TS interface and `toParsed()` in
   `devcontainers-cli/frontend/src/devcontainer-engine/index.ts`
2. the bundle, rebuilt and re-copied per above
3. `ParsedDevContainer` in
   `devcontainers-cli/src-tauri/crates/devcontainer-core/src/devcontainer/translate.rs`
4. the `ParsedDevContainer` interface in this repo's
   `src/devcontainer-engine.ts` (kept for parity/clarity only)

## Changing the parsed contract

`ParsedDevContainer` is `#[serde(rename_all = "camelCase")]`, so the JS
keys are camelCase (`runArgs`, `overrideCommand`, `containerEnv`) even
though the Rust fields are snake_case. Add `#[serde(default)]` to anything
optional so an older bundle keeps working.

`to_container_spec()` in `translate.rs` is the single place the parsed
config becomes a runtime-agnostic `ContainerSpec`. It returns
`Result<_, TranslateError>`: anything it cannot express faithfully is an
error rather than a silently dropped setting.

## Compatibility validation

Runtimes declare what they cannot do via
`ContainerRuntime::validate_devcontainer(&ParsedDevContainer) ->
CompatibilityReport`. The orchestrator calls it at the very start of
`up_inner`, **before** pulling an image or running a Dockerfile build, so
an unsupported configuration is refused cheaply instead of after a
multi-minute build.

`AppleContainersRuntime` overrides it to reject `runArgs` flags its CLI
does not implement (currently `--add-host`), naming the offending value and
suggesting an alternative. The list is deliberately **evidence-based** —
an entry should only be added once the CLI has been observed to reject the
flag, because a speculative entry would block a configuration that
actually works.
