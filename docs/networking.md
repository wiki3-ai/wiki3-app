# Networking — How `Serve`, `Site`, and the local preview work

Last updated 2026-05-02 after a multi-day investigation on a macOS
Tahoe MacBook where the Site button opened to a `chrome-error://`
page even though `curl` against the same URL worked. This doc is
meant to be the canonical reference for how a wiki's local preview
gets from "container is up" to "the user's browser shows the site"
— including all the failure modes we now know about.

## TL;DR

When you click **Serve** on a wiki:

1. We start an Apple Container running JupyterLite with `--publish
   8000:8000`. Apple Container's *publish-proxy* binds `*:8000` on
   the host and is *supposed* to forward each accepted connection
   into the container's vmnet IPv4 (`192.168.64.x:8000`).
2. The per-wiki port poller (`src-tauri/src/wiki/ports.rs`) probes
   three paths in priority order:
   * `127.0.0.1:8000` (the publish-proxy's loopback path)
   * `<container_ip>:8000` (the direct vmnet path, via
     `bridge100`)
   * an in-process TCP forwarder (`src-tauri/src/wiki/forwarder.rs`)
     that bridges `127.0.0.1:<ephemeral>` → `<container_ip>:8000`.
3. The first path that actually returns HTTP bytes (not just a
   completed `connect()`) is recorded as `serving=true` and the
   matching URL goes into the dashboard's port row.
4. **Site** opens that URL in the in-app WebView (Chrome on Tahoe
   when launched as a debug build).

Most users only ever see (1) and never know the rest exist. The
Tahoe investigation covered why steps 2 and 3 are necessary at all.

---

## Why the publish-proxy isn't enough on Tahoe

Apple Container's publish-proxy is a per-port host-side listener
the `container` daemon spins up for each `--publish` flag. On
healthy hosts it does this transparently:

```
host:127.0.0.1:8000 ──[publish-proxy]──> container:192.168.64.2:8000
                                               (over bridge100)
```

On the Tahoe MacBook used during this investigation, the
publish-proxy accepted TCP connections on both `127.0.0.1` *and*
the host's primary LAN IP (`192.168.86.52`) but RST'd as soon as
the kernel tried to forward bytes:

```
$ probe http://127.0.0.1:8000/
  connect OK
  read failed: Connection reset by peer (os error 54)

$ probe http://192.168.86.52:8000/
  connect OK
  read failed: Connection reset by peer (os error 54)

$ probe http://192.168.64.2:8000/
  connect OK
  read 256 bytes; first line: HTTP/1.1 200 OK
```

That last line is the smoking gun: the *container itself* is fine.
It's the proxy's host-side forwarding that's broken. Both loopback
and the host's LAN IP are affected — so this is not a "loopback
filter" problem, it's the publish-proxy being broken end-to-end.

The investigation never identified the exact mechanism (it
correlates with the macOS 26 build but other Tahoe builds don't
exhibit it; no obvious EDR/ZTNA process is involved). We now
treat the publish-proxy as **best-effort** rather than depending on
it.

## The three paths the poller probes

Defined in `wiki/ports.rs` as `enum Reachable`:

| Variant | Connects to | When it's used |
|---|---|---|
| `Loopback` | `127.0.0.1:host_port` | Default. Fastest to establish; doesn't require the vmnet IP. Works on healthy hosts. |
| `Direct(ip)` | `<container_ip>:port` over `bridge100` | When `Loopback` probes RST. The container's HTTP server is reachable but the URL (`http://192.168.64.2:8000/`) is unfit for browsers. |
| `Forwarder { local, target }` | `127.0.0.1:<ephemeral>` (in-process) → `<container_ip>:port` | Promoted from `Direct` so the dashboard has a *loopback* URL Chrome will accept. |

A probe is only counted as "alive" if it returns bytes — a
completed `connect()` followed by RST does **not** count. This
distinction is what lets the poller tell `Loopback` apart from
`Direct` on Tahoe.

The container's vmnet IPv4 is read once per `INSPECT_REFRESH_INTERVAL`
(5 s) via `container inspect <name>`. Spawning that subprocess on
every probe tick used to flood the API server during heavy `pip
install`s and stall the publish-proxy further.

## The TCP forwarder (`wiki/forwarder.rs`)

When `Direct` is the only working path, we promote it to
`Forwarder` by binding a TCP listener inside the wiki3-app process
on an OS-assigned ephemeral port and shovelling bytes between each
accepted connection and the container.

```
Chrome ──> 127.0.0.1:59364 ──[wiki3-app forwarder]──> 192.168.64.2:8000
```

Why a forwarder rather than just handing Chrome the
`192.168.64.x` URL:

1. **Service workers**: JupyterLite registers a service worker, and
   Chrome only allows SWs on `localhost`, `127.0.0.1`, or HTTPS.
   `http://192.168.64.2:8000/` is none of those.
2. **HTTPS-First Mode**: Chrome silently upgrades
   `http://<RFC1918>:<port>` to HTTPS, the upgrade fails, and the
   user lands on `chrome-error://chromewebdata` with the misleading
   error "Unsafe attempt to load URL ... from frame with URL
   chrome-error://chromewebdata". This was the original symptom on
   Tahoe and is the reason the forwarder exists.

Implementation notes:

* Each forwarder is keyed on `(wiki_id, container_port)` and holds
  a `tokio::sync::watch` shutdown channel. We use `watch` rather
  than `Notify` because `notify_waiters()` has a race where a
  notification can fire before a spawned task registers its waiter,
  silently leaking the listener and its port.
* Per-connection bytes flow through `tokio::io::copy_bidirectional`.
* Lifecycle:
  * `ensure(wiki_id, container_port, target)` — idempotent. If a
    forwarder for that key already points at the same target, it's
    reused. If the target changed (container restart picked up a
    different IP), the old one is torn down and a new one bound.
  * `stop_all_for_wiki(wiki_id)` — called when the poller's
    idle-timeout fires (the dashboard hasn't asked about this wiki
    in a while). The poller itself does *not* tear forwarders down
    on a "settled" exit because the user is presumably about to
    open the site.
  * `snapshot()` — exposed for diagnostics (see below).

## What "Site" actually does

`Site` reads the `url` field of the wiki's serving port row and
opens it in a new in-app WebView window. By the time the button
appears, the poller has already verified that *some* path is alive
and chosen a URL that is browser-safe:

* `Loopback` → `http://localhost:<host_port>/`
* `Forwarder` → `http://localhost:<local_port>/`
* (`Direct` is never advertised as a Site URL — it's only used as
  evidence the container is alive while we set up a Forwarder.)

## Diagnostics

The dashboard has a **Diagnose…** button (top-right action row)
that runs `run_diagnostic_report` and writes a timestamped text
file under the app's log directory:

```
~/Library/Logs/ai.wiki3.studio/wiki3-diagnostics-<stamp>.txt
```

The report covers:

| Section | What it shows |
|---|---|
| `system` | `sw_vers`, `uname -a` |
| `apple container` | `container --version`, `container system status`, `container ls -a`, parsed running containers |
| `host listening sockets` | `lsof -nP -iTCP -sTCP:LISTEN`, plus a scoped `lsof -p $wiki3-app-pid` and a `netstat -an -p tcp` head |
| `wiki3-app forwarder registry` | The current contents of `forwarder::snapshot()`, with each entry probed |
| `wiki3-app registered wikis` | `manager.list()` — id, name, local_path |
| `loopback probe` | `probe http://127.0.0.1:<port>/` against each container's host_ports — this is the path that fails first on Tahoe |
| `publish-proxy on host LAN address` | Same probes against the primary LAN IPv4 (resolved from `route -n get default` + `ifconfig`). If both this and loopback fail, the proxy is broken end-to-end, not just on loopback |
| `direct vmnet probe` | `probe http://<container_ip>:<port>/` — should always succeed if the container is up |
| `control: our own loopback listener` | A self-test where wiki3-app binds `127.0.0.1:<ephemeral>`, accepts a connection, and reads back its own response. If this fails, *something* is breaking arbitrary loopback (network filter, AV, etc.) — not just Apple Container's proxy |
| `interfaces` | `ifconfig` (IPv4-only filter), `netstat -rn -f inet` head |
| `vpn / network filter / EDR processes` | `ps` filtered for known VPN/ZTNA/EDR vendors |
| `launchd entries mentioning container` | `launchctl list | grep -i container` |
| `wiki3-app port poller cache snapshot` | Pointer to live stderr `[port-poller]`/`[port-cmd]` lines |

`build_report` takes an optional `&AppHandle` so the smoke test can
exercise the same code path without spinning up a Tauri context.

## Things that confused us during the investigation

* **The publish-proxy listening doesn't mean it's working.** A
  successful `connect()` to `*:8000` doesn't tell you anything
  beyond "the proxy accepted TCP". You need to read at least one
  byte to confirm the byte path is intact.
* **Two `127.0.0.1:LISTEN` sockets owned by wiki3-app is normal**
  if you have two registered wikis pointing at the same container
  path. The forwarder is keyed on `(wiki_id, container_port)`, so
  duplicate-path wikis each get their own listener. (We had a bug
  where the *Clone…* button on a remote-only entry mutated that
  entry's `local_path` rather than just adding a new entry — fixed
  in `src/main.ts` after the Tahoe report surfaced the duplicate.)
* **Chrome's `chrome-error://chromewebdata` page is misleading.**
  It says "Unsafe attempt to load URL ... from frame with URL
  chrome-error://chromewebdata" which sounds like a CSP/CSRF
  problem but is actually just the failed HTTPS-First upgrade
  retrying inside the error page's own frame.
* **`container 17955 jim *:8000 (LISTEN)`** is Apple Container's
  publish-proxy. PID 17955 is the per-container runtime helper
  spawned by `com.apple.container.container-runtime-linux.<name>`
  in launchd. When the proxy is broken, this socket is *live but
  unused* — our forwarder bypasses it entirely.

## Future work

* Detecting "loopback works but reads RST" earlier so the poller
  can skip straight to the forwarder rather than going through the
  Direct stage. Right now we wait for two failed reads before
  promoting.
* Surface forwarder activity in the dashboard (e.g. a small
  "(via forwarder)" hint next to the Site button) so users on
  affected hosts know a non-default path is in use.
* If the publish-proxy bug turns out to be reproducible on stock
  Tahoe (no corporate filters), file it with Apple. So far we've
  only seen it on one host and don't have a clean repro.
