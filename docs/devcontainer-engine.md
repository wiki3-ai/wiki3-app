# Devcontainer parsing — the two paths, why they exist, and how to change them

Last updated 2026-09-13 while adding the runtime compatibility gate.

There are **two independent code paths** that read
`.devcontainer/devcontainer.json`. This is not an accident, but it is also
not finished work: one is the predecessor of the other, and the older one
is still load-bearing for a live feature.

If you only read one section, read [Changing the parsed contract](#changing-the-parsed-contract)
— getting that wrong fails silently.

## TL;DR

| | Legacy *preview* path | Devcontainer *lifecycle* path |
|---|---|---|
| Parser | `src-tauri/src/tools/devcontainer_config.js`, run in-process by **rquickjs** | prebuilt `src/public/devcontainer-engine.js`, built from the upstream `@devcontainers/cli` spec slice |
| Rust shape | `tools::devcontainer_config::DevcontainerConfig` | `devcontainer_core::ParsedDevContainer` |
| Drives | per-wiki **Build / Serve / Stop** preview container | `wiki_container_ctl_*` (start / stop / restart / rebuild / remove) |
| Runtime | the Apple `container` CLI, invoked directly | pluggable `ContainerRuntime` (`apple_containers` / `docker` / `podman`) |
| Entry point | `tools::devcontainer_image::ensure_devcontainer_image`, called from `wiki/local_site.rs` | `commands_devcontainer.rs::submit_parsed_devcontainer` → `LifecycleOrchestrator` |

Both are alive. Neither is dead code.

## History — why there are two

The older path is genuinely a leftover of a superseded approach, in this
order:

1. **2026-04-21 `d64f85a`** — "Remove Deno stuff and use rquickjs to run
   devcontainers code". The devcontainers parsing was inlined into the
   binary as a JavaScript module run by an embedded QuickJS interpreter,
   so the app shipped no external toolchain.
2. **2026-04-29 `f116b12`** — "Adopt devcontainer-core's apple_containers
   helpers". The container work starts moving into the reusable
   `devcontainer-core` crate.
3. **2026-04-30 `4e51301`** — "Use the devcontainer-cli code from core".
   The prebuilt engine bundle replaces the inlined-JS approach.

So the rquickjs parser is the **predecessor** of the bundle, and the
per-wiki preview feature was never migrated off it. The step-1 rationale is
also no longer true: we now do ship a build of the upstream spec slice (as
a prebuilt ES module, not as a Deno toolchain).

## Why the legacy path can't just be deleted

Two reasons, one practical and one strategic:

1. **It is load-bearing.** `Build` / `Serve` / `Stop` on a wiki card runs
   through `tools::devcontainer_image::ensure_devcontainer_image`
   (via `wiki/local_site.rs`), which parses with
   `tools::devcontainer_config`. Deleting the parser takes the per-wiki
   preview feature with it.
2. **It cannot serve the Docker work.** It drives the Apple `container`
   CLI directly (`apple_container::ensure_service_running`,
   `is_service_running`, and raw `Command::new(&container_bin)` calls in
   `wiki/git_commands.rs` and `wiki/local_site.rs`). There is no runtime
   seam, so no Docker or Podman backend can plug into it.

Its `DevcontainerConfig` struct is also a hand-rolled *subset* parser: it
declares `image`, `build`, `forwardPorts`, `remoteUser`, and
`customizations`, but **not** `mounts` or `runArgs` — even though
`devcontainer_config.js` emits both. Serde drops unknown fields silently,
so those settings vanish with no error. That is the same class of bug the
compatibility gate exists to prevent, and it is a good argument for
retiring this path rather than extending it.

## Migration plan

Keep the legacy path as-is while the Docker/Podman backends land, because
that work is orthogonal to it. Once a non-Apple runtime can actually run a
container:

1. Move `Build` / `Serve` / `Stop` onto `devcontainer-core`'s
   `LifecycleOrchestrator` and `ContainerRuntime`.
2. Delete `tools/devcontainer_config.{rs,js}`,
   `tools/devcontainer_image.rs`, and the direct-CLI container code in
   `wiki/git_commands.rs` / `wiki/local_site.rs`.
3. Fix the field-dropping subset semantics by construction — they go away
   with the parser.

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
