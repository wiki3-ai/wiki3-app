//! Per-wiki port monitoring.
//!
//! Reads `forwardPorts` and `portsAttributes` from the wiki's
//! `devcontainer.json` and pairs each forwarded port with a *cached*
//! reachability result so the dashboard can show which ports are
//! actually being served. The cache is fed by a per-wiki background
//! poller; the Tauri command never blocks on a probe — it just
//! reads whatever the poller has most recently written.
//!
//! On corp-laptop macOS hosts the loopback publish-proxy provided
//! by Apple Container is sometimes unreachable (network filters /
//! ZTNA agents accept-then-RST the connection on `127.0.0.1`),
//! while the container is still reachable directly via its vmnet
//! address. The poller probes both, and a port is only marked
//! `serving` once one of them has actually returned data — so we
//! never advertise a 127.0.0.1 URL that the user can't actually
//! reach, and we don't flip the UI to "ready" until the in-container
//! HTTP server has finished starting up.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{command, State};

use devcontainer_core::ParsedDevContainer;

use crate::wiki::commands::WikiState;
use crate::wiki::local_site::LocalSiteManager;

/// How often the background poller re-probes each port while it
/// is still searching for a working path.
const POLL_INTERVAL_FAST: Duration = Duration::from_millis(250);

/// How often we re-probe once at least one port is known to be
/// serving. Slower cadence reduces the in-container HTTP server
/// log spam (the JupyterLite "405 HEAD /" lines) and avoids
/// hammering Apple Container's publish-proxy.
const POLL_INTERVAL_STABLE: Duration = Duration::from_millis(1000);

/// How often we re-resolve the container's vmnet IPv4 via
/// `container inspect`. Spawning that subprocess every probe tick
/// flooded the API server during heavy `pip install`s and stalled
/// the publish-proxy. The IP only changes across container
/// stop/start, so 5s is plenty.
const INSPECT_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

/// How long the poller keeps running after the dashboard last asked
/// for this wiki's ports. Each `wiki_container_ports` call refreshes
/// the deadline; the dashboard refreshes every few seconds, so this
/// just needs to be comfortably longer than that.
const POLLER_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// How many consecutive failed probes we tolerate after a port has
/// been seen as `Loopback`/`Direct` before we downgrade the cached
/// status back to `No`. Apple Container's publish-proxy and the
/// in-container HTTP server can both glitch occasionally, and a
/// single dropped probe shouldn't yank the link grey in the UI.
const STICKY_OK_FAILURE_BUDGET: u32 = 6;

/// Where the port is reachable from the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reachable {
    /// Reached via `127.0.0.1:<host_port>` — Apple Container's
    /// publish-proxy is healthy.
    Loopback,
    /// Reached via the container's vmnet IPv4 directly. Used when
    /// the loopback proxy is being intercepted by a host-side
    /// network filter.
    Direct(Ipv4Addr),
    /// Reached via an in-process TCP forwarder bound on
    /// `127.0.0.1:<local>` that tunnels to `<container_ip>:<port>`.
    /// We promote `Direct` to this when loopback is broken so the
    /// dashboard can advertise a `localhost` URL that satisfies
    /// Chrome's secure-context / service-worker constraints.
    Forwarder { local: u16, target: Ipv4Addr },
    /// Neither path has responded yet, or the last probe failed.
    No,
}

/// One row per forwarded port. Field names are camelCase for JS.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortRow {
    /// Port number on the host (same as `internal` for Apple Container's
    /// `--publish p:p/tcp` mapping that the lifecycle uses).
    pub external: u16,
    /// Port number inside the container.
    pub internal: u16,
    /// Optional human label from `portsAttributes.<port>.label`.
    pub label: Option<String>,
    /// Whether the most recent background probe got a real response
    /// from the in-container HTTP server. Stays `false` until then —
    /// we never advertise a port as ready before it actually is.
    pub serving: bool,
    /// URL to open in the browser. Only meaningful when `serving` is
    /// `true`; until then it's a placeholder loopback URL the UI can
    /// hide.
    pub url: String,
    /// Stable identifier for this port within the wiki, suitable for
    /// use as a window-target hint (`<repo>-<port-name>`).
    pub key: String,
}

// ---------------------------------------------------------------------------
// Background poller + cache
// ---------------------------------------------------------------------------

/// Process-wide cache keyed by wiki id, holding the most recent
/// reachability per published port. Reads (from the Tauri command)
/// are non-blocking; writes happen on background poller ticks.
#[derive(Default)]
struct PortHealthCache {
    by_wiki: HashMap<String, HashMap<u16, Reachable>>,
    pollers: HashSet<String>,
    /// Last time the dashboard asked for this wiki's ports. Pollers
    /// keep running while this is recent; they exit once it ages out.
    last_request: HashMap<String, Instant>,
}

fn cache() -> &'static Arc<Mutex<PortHealthCache>> {
    static CELL: OnceLock<Arc<Mutex<PortHealthCache>>> = OnceLock::new();
    CELL.get_or_init(|| Arc::new(Mutex::new(PortHealthCache::default())))
}

fn read_cached(wiki_id: &str, port: u16) -> Reachable {
    let g = cache().lock().unwrap();
    g.by_wiki
        .get(wiki_id)
        .and_then(|m| m.get(&port))
        .copied()
        .unwrap_or(Reachable::No)
}

fn write_cached(wiki_id: &str, results: HashMap<u16, Reachable>) {
    let mut g = cache().lock().unwrap();
    g.by_wiki.insert(wiki_id.to_string(), results);
}

/// Forget everything cached for `wiki_id`.
///
/// Used when we learn that the container which produced those results is
/// gone: dropping the entries means the next poller run re-derives
/// reachability from scratch, instead of the sticky-OK rule holding up a
/// stale success. The `last_request` entry goes too, so the poller's idle
/// check starts fresh in the same breath; callers must `touch_request`
/// afterwards if they are about to spawn one.
fn evict(wiki_id: &str) {
    let mut g = cache().lock().unwrap();
    g.by_wiki.remove(wiki_id);
    g.last_request.remove(wiki_id);
}

fn touch_request(wiki_id: &str) {
    let mut g = cache().lock().unwrap();
    g.last_request.insert(wiki_id.to_string(), Instant::now());
}

fn last_request_age(wiki_id: &str) -> Option<Duration> {
    let g = cache().lock().unwrap();
    g.last_request.get(wiki_id).map(|t| t.elapsed())
}

/// Mark `wiki_id`'s poller as live. Returns `true` if this call is
/// responsible for spawning the task (i.e. no poller was already
/// running for that wiki).
fn try_claim_poller(wiki_id: &str) -> bool {
    let mut g = cache().lock().unwrap();
    g.pollers.insert(wiki_id.to_string())
}

fn release_poller(wiki_id: &str) {
    let mut g = cache().lock().unwrap();
    g.pollers.remove(wiki_id);
}

/// Combine the previous cached reachability with a fresh probe
/// result, applying sticky-OK semantics: an already-good port stays
/// good for up to `budget` consecutive failed probes before being
/// downgraded to `No`. Mutates `streak` to track the current
/// failure streak — callers thread one streak counter per port.
///
/// Pure function, intentionally separate from cache I/O so it can
/// be unit tested without spinning up the runtime.
fn apply_sticky(prev: Reachable, fresh: Reachable, streak: &mut u32, budget: u32) -> Reachable {
    match (prev, fresh) {
        (
            Reachable::Loopback | Reachable::Direct(_) | Reachable::Forwarder { .. },
            Reachable::No,
        ) => {
            *streak = streak.saturating_add(1);
            if *streak >= budget {
                Reachable::No
            } else {
                prev
            }
        }
        (_, ok) => {
            *streak = 0;
            ok
        }
    }
}

/// Long-running task: every [`POLL_INTERVAL`], probe every published
/// port for the wiki and update the cache. Stays running as long as
/// the dashboard has asked for this wiki's ports recently — this
/// decouples polling from `LocalSiteManager` registration, which is
/// only populated by some of the start paths. The cache is *not*
/// evicted on exit, so the dashboard keeps showing the last known
/// Which container, if any, is bound to `local_path` via the
/// devcontainer-controls path (start / stop / rebuild).
///
/// Two sources, because neither alone is sufficient:
///
/// * the orchestrator's in-process record for `wiki_id` — cheapest, and
///   correct the moment the user hits Up;
/// * a scan of the selected runtime's containers matched by bind-mount
///   source — the orchestrator's slots are in-memory, so a container that
///   outlived an app restart is still running and still publishing ports
///   with nothing in-process pointing at it.
///
/// The scan is also what keeps this runtime-agnostic: the runtime reports
/// the host paths it was given, whether it is Docker, Podman or Apple
/// Containers, so no per-runtime discovery code is needed here.
async fn devcontainer_container_for(
    app: &tauri::AppHandle,
    wiki_id: &str,
    local_path: &Path,
) -> Option<String> {
    use tauri::Manager;

    if let Some(orchestrator) = app.try_state::<devcontainer_core::LifecycleOrchestrator>() {
        let snapshot = orchestrator.snapshot(wiki_id);
        if snapshot.state == "running" {
            if let Some(id) = snapshot.container_id {
                return Some(id);
            }
        }
    }

    let registry = app.try_state::<devcontainer_core::RuntimeRegistry>()?;
    let runtime = registry.resolve().await;
    let containers = runtime.list().await.ok()?;
    containers
        .into_iter()
        // Only a *running* container has ports published — a stopped one has
        // already had its bindings released, so counting it as present would
        // keep a stale "serving" verdict alive.
        .find(|c| {
            c.state == devcontainer_core::ContainerState::Running
                && c.host_mounts.iter().any(|m| same_path(m, local_path))
        })
        .map(|c| c.container_id)
}

/// Whether a container is still running for this wiki, judged by the most
/// direct evidence available.
///
/// Conservative by design: `LocalSiteManager` is authoritative for the
/// legacy Apple `Serve` flow (whose container the poller reaches by vmnet
/// IP), so when it has an entry we report running and leave that path
/// exactly as it was.
async fn container_running_for(app: &tauri::AppHandle, wiki_id: &str, local_path: &Path) -> bool {
    use tauri::Manager;

    if let Some(site) = app.try_state::<LocalSiteManager>() {
        if site.get(wiki_id).is_some() {
            return true;
        }
    }
    devcontainer_container_for(app, wiki_id, local_path)
        .await
        .is_some()
}

/// Compare a host path reported by a runtime against the wiki's local
/// path. The two can differ by a trailing slash or by a symlinked prefix
/// (`/var` vs `/private/var` on macOS), so fall back to canonicalising.
fn same_path(reported: &str, wanted: &Path) -> bool {
    let reported = Path::new(reported);
    if reported == wanted {
        return true;
    }
    match (reported.canonicalize(), wanted.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// state across brief poller restarts.
async fn poll_wiki_ports(wiki_id: String, local_path: std::path::PathBuf, app: tauri::AppHandle) {
    use tauri::Manager;
    eprintln!(
        "[port-poller] start wiki_id={wiki_id} path={}",
        local_path.display()
    );
    let mut last_logged: HashMap<u16, Reachable> = HashMap::new();
    // Track per-port consecutive failure counts since the last
    // successful probe — used to delay the `Loopback`/`Direct` →
    // `No` downgrade so a single flaky probe doesn't yank the link.
    let mut fail_streak: HashMap<u16, u32> = HashMap::new();
    // Cache of the container's vmnet IP plus the container name it
    // was resolved for, refreshed every `INSPECT_REFRESH_INTERVAL`
    // rather than every tick. Spawning `container inspect` 4×/sec
    // during a build saturated Apple Container's API server.
    let mut cached_ip: Option<Ipv4Addr> = None;
    let mut cached_for_container: Option<String> = None;
    let mut last_inspect_at: Option<Instant> = None;
    loop {
        // Did the dashboard stop caring? Bail out and keep the
        // current cache contents in place — if the dashboard comes
        // back, the next request re-spawns us with a clean slate.
        match last_request_age(&wiki_id) {
            Some(age) if age < POLLER_IDLE_TIMEOUT => { /* keep going */ }
            _ => {
                eprintln!("[port-poller] exit (idle) wiki_id={wiki_id}");
                crate::wiki::forwarder::stop_all_for_wiki(&wiki_id);
                release_poller(&wiki_id);
                return;
            }
        }

        // Resolve the running container's IPv4 only when we don't
        // have a recent answer for the current container name. This
        // keeps `container inspect` calls down to roughly one every
        // few seconds (or immediately on container swap), instead
        // of one per probe tick.
        //
        // We try `LocalSiteManager` first (cheap, in-memory) and
        // fall back to `find_container_by_mount_source`, which
        // walks `container ls --format json` and matches on the
        // workspace path. The fallback is necessary because
        // `LocalSiteManager` is process-local: if wiki3-app is
        // restarted while the container is still running (very
        // common with the bundled .app on a Tahoe machine), the
        // map will be empty even though the container is healthy
        // on its vmnet IP.
        let container_ipv4 = {
            let site_state = app.state::<LocalSiteManager>();
            let site_name = site_state.get(&wiki_id).map(|s| s.serve_container);

            // The devcontainer-controls path is runtime-agnostic and
            // records its container with the orchestrator, not with
            // `LocalSiteManager`. Asking only the Apple-specific sources
            // therefore concludes "no container" for a Docker container —
            // and the gate below then writes `Reachable::No` for every port
            // *without probing at all*, leaving the dashboard permanently
            // grey even though the ports are published and answering on
            // loopback.
            let known_name = match site_name.clone() {
                Some(name) => Some(name),
                None => devcontainer_container_for(&app, &wiki_id, &local_path).await,
            };

            let needs_refresh = last_inspect_at
                .map(|t| t.elapsed() >= INSPECT_REFRESH_INTERVAL)
                .unwrap_or(true)
                || cached_for_container.as_deref() != known_name.as_deref();

            if needs_refresh {
                if site_name.is_some() {
                    // Legacy Apple `Serve` flow: resolve the container's
                    // vmnet IPv4 so the Direct fallback works when the
                    // publish-proxy is broken.
                    let bin = crate::tools::apple_container::detect()
                        .path
                        .unwrap_or_else(|| std::path::PathBuf::from("container"));
                    let name = known_name.clone().unwrap_or_default();
                    cached_ip = crate::tools::apple_container::inspect_container_ipv4(&bin, &name)
                        .await
                        .and_then(|s| s.parse::<Ipv4Addr>().ok());
                    cached_for_container = Some(name);
                } else if let Some(name) = known_name {
                    // A container from the devcontainer-controls path.
                    // Docker and Podman publish their ports on the host, so
                    // a loopback probe is sufficient and there is no vmnet
                    // address to resolve. Apple Containers reached through
                    // this path still work over loopback or the
                    // publish-proxy.
                    cached_ip = None;
                    cached_for_container = Some(name);
                } else {
                    // Last resort: the legacy Apple discovery by mount
                    // source. One `container ls` per refresh interval —
                    // the same load profile as inspecting a known name.
                    let bin = crate::tools::apple_container::detect()
                        .path
                        .unwrap_or_else(|| std::path::PathBuf::from("container"));
                    match crate::tools::apple_container::find_container_by_mount_source(
                        &bin,
                        &local_path,
                    )
                    .await
                    {
                        Some((name, ipv4)) => {
                            cached_ip = ipv4.parse::<Ipv4Addr>().ok();
                            cached_for_container = Some(name);
                        }
                        None => {
                            cached_ip = None;
                            cached_for_container = None;
                        }
                    }
                }
                last_inspect_at = Some(Instant::now());
            }
            cached_ip
        };

        // Read the configured ports each tick rather than caching them,
        // so a re-submit (Start / Restart / Rebuild after an edit to
        // `devcontainer.json`) is picked up without a restart.
        let ports = configured_ports(submitted_config(&app, &wiki_id).as_ref());
        if ports.is_empty() {
            tokio::time::sleep(POLL_INTERVAL_FAST).await;
            continue;
        }

        // If no container is running for this wiki, skip the probe
        // entirely. Otherwise the loopback probe at `127.0.0.1:<port>`
        // would happily succeed against an *unrelated* wiki's
        // publish-proxy when both wikis declare the same
        // `forwardPorts` (e.g. two devcontainers both forwarding
        // 8888) — conflating their port status — and the poller
        // would run forever at the fast cadence with no live
        // container to settle against. We still tick at the slow
        // cadence so the dashboard reflects "nothing serving" if
        // the container is stopped, and we pick up a freshly-
        // started container within `INSPECT_REFRESH_INTERVAL`.
        if cached_for_container.is_none() {
            let mut empty: HashMap<u16, Reachable> = HashMap::with_capacity(ports.len());
            for port in &ports {
                let prev = read_cached(&wiki_id, *port);
                let streak = fail_streak.entry(*port).or_insert(0);
                let resolved = apply_sticky(prev, Reachable::No, streak, STICKY_OK_FAILURE_BUDGET);
                empty.insert(*port, resolved);
            }
            for (port, reach) in &empty {
                if last_logged.get(port) != Some(reach) {
                    eprintln!(
                        "[port-poller] wiki_id={wiki_id} port={port} (no container) -> {:?}",
                        reach
                    );
                    last_logged.insert(*port, *reach);
                }
            }
            write_cached(&wiki_id, empty);
            crate::wiki::forwarder::stop_all_for_wiki(&wiki_id);
            tokio::time::sleep(POLL_INTERVAL_STABLE).await;
            continue;
        }

        // Run the probes off the async runtime — `TcpStream` is
        // blocking and can hang up to its full timeout.
        let to_probe = ports.clone();
        let raw_results = tokio::task::spawn_blocking(move || {
            let mut out = HashMap::with_capacity(to_probe.len());
            for port in to_probe {
                out.insert(port, probe_reachability(port, container_ipv4, port));
            }
            out
        })
        .await
        .unwrap_or_default();

        // Promote any `Direct(ip)` result to `Forwarder` by
        // standing up an in-process TCP tunnel on `127.0.0.1:0`.
        // This is the workaround for Tahoe hosts where Apple
        // Container's publish-proxy ACCEPT-then-RSTs on loopback
        // and Chrome refuses to load the working vmnet URL
        // because RFC1918 isn't a service-worker-secure context.
        // `forwarder::ensure` is idempotent for an unchanged
        // target, so this runs every tick without churning ports.
        let mut promoted: HashMap<u16, Reachable> = HashMap::with_capacity(raw_results.len());
        for (port, fresh) in raw_results {
            let final_fresh = match fresh {
                Reachable::Direct(ip) => {
                    match crate::wiki::forwarder::ensure(&wiki_id, port, ip).await {
                        Some(local) => Reachable::Forwarder { local, target: ip },
                        None => Reachable::Direct(ip),
                    }
                }
                Reachable::Loopback => {
                    // Loopback recovered — drop the tunnel so we
                    // stop holding the local port and the URL
                    // stabilises back on the publish-proxy.
                    crate::wiki::forwarder::stop(&wiki_id, port);
                    Reachable::Loopback
                }
                other => other,
            };
            promoted.insert(port, final_fresh);
        }
        let raw_results = promoted;

        // Apply the sticky-OK rule: if the previous cache entry was
        // a successful path and this probe came back `No`, hold the
        // previous result for up to `STICKY_OK_FAILURE_BUDGET`
        // consecutive failures. Any successful probe resets the
        // streak.
        let mut effective: HashMap<u16, Reachable> = HashMap::with_capacity(raw_results.len());
        for (port, fresh) in raw_results.into_iter() {
            let prev = read_cached(&wiki_id, port);
            let streak = fail_streak.entry(port).or_insert(0);
            let resolved = apply_sticky(prev, fresh, streak, STICKY_OK_FAILURE_BUDGET);
            effective.insert(port, resolved);
        }

        for (port, reach) in &effective {
            if last_logged.get(port) != Some(reach) {
                eprintln!(
                    "[port-poller] wiki_id={wiki_id} port={port} ipv4={:?} -> {:?}",
                    container_ipv4, reach
                );
                last_logged.insert(*port, *reach);
            }
        }

        // While any port is still showing as `No`, keep the tight
        // 250ms cadence so the dashboard flips green the instant
        // the in-container HTTP server starts responding. Once
        // every port has settled to a known-good result, exit the
        // poller entirely: the in-container HTTP server's access
        // log was filling with `HEAD /` requests, and re-checking
        // a known-good port adds no information the dashboard can
        // act on. Set `WIKI3_KEEP_PROBING=1` to keep polling at
        // 1Hz indefinitely (useful for diagnosing flaky links).
        let all_ok = !effective.is_empty()
            && effective.values().all(|r| {
                matches!(
                    r,
                    Reachable::Loopback | Reachable::Direct(_) | Reachable::Forwarder { .. }
                )
            });
        write_cached(&wiki_id, effective);
        if all_ok && !keep_probing_after_settled() {
            eprintln!(
                "[port-poller] exit (settled) wiki_id={wiki_id} — set WIKI3_KEEP_PROBING=1 to keep checking"
            );
            // Deliberately do *not* call `release_poller` here:
            // the cache already holds a successful result, and we
            // want subsequent dashboard refreshes to keep showing
            // it without re-spawning a poller (which would put
            // the `HEAD /` lines back into the container log).
            return;
        }
        let interval = if all_ok {
            POLL_INTERVAL_STABLE
        } else {
            POLL_INTERVAL_FAST
        };
        tokio::time::sleep(interval).await;
    }
}

/// Returns true if the user has opted into continuing to probe
/// ports after at least one working address has been identified.
/// Default is to stop, which silences the in-container access log
/// and avoids any further publish-proxy traffic.
fn keep_probing_after_settled() -> bool {
    matches!(
        std::env::var("WIKI3_KEEP_PROBING").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

// ── Submitted config ─────────────────────────────────────────────────────

/// The config the container for `wiki_id` was created from, or `None` if the
/// dashboard has not submitted one yet.
///
/// This is the single source of truth for "what ports does this wiki declare":
/// the frontend engine parses `devcontainer.json` and hands the result to the
/// orchestrator, and everything downstream reads it from there. Parsing again
/// here would be a second opinion that could disagree with the config the
/// container was actually made from.
fn submitted_config(app: &tauri::AppHandle, wiki_id: &str) -> Option<ParsedDevContainer> {
    use tauri::Manager;
    app.state::<devcontainer_core::LifecycleOrchestrator>()
        .parsed_config(wiki_id)
}

/// Forwarded ports from a submitted config.
///
/// The engine's `toParsed` has already normalised `forwardPorts` — both the
/// bare-integer and string forms — into validated `u16`s, so unlike the
/// previous file-parsing version there is nothing to re-interpret here. An
/// empty vec means "no config submitted yet" *or* "no ports declared";
/// callers treat both as "nothing to show".
fn configured_ports(parsed: Option<&ParsedDevContainer>) -> Vec<u16> {
    parsed.map(|p| p.forward_ports.clone()).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Row building (cache → PortRow[])
// ---------------------------------------------------------------------------

/// Build port rows from the config the container was created from — see
/// [`submitted_config`] — plus the reachability cache the poller feeds.
///
/// Pure read: no file I/O and no network I/O, so the Tauri command stays
/// snappy.
///
/// `local_path` is only used to name the rows; the ports themselves come from
/// the config, so the panel describes what the container was actually built
/// from rather than whatever `devcontainer.json` says right now.
fn rows_from_cache(
    wiki_id: &str,
    local_path: &Path,
    parsed: Option<&ParsedDevContainer>,
) -> Vec<PortRow> {
    let Some(parsed) = parsed else {
        return Vec::new();
    };

    let repo_slug = local_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("wiki")
        .to_string();

    let mut out = Vec::new();
    for &port in &parsed.forward_ports {
        let attr = parsed
            .ports_attributes
            .as_ref()
            .and_then(|m| m.get(port.to_string()));
        let label = attr
            .and_then(|a| a.get("label"))
            .and_then(|s| s.as_str())
            .map(|s| s.to_string());
        let protocol = attr
            .and_then(|a| a.get("protocol"))
            .and_then(|s| s.as_str())
            .unwrap_or("http")
            .to_string();
        let key_name = label
            .as_deref()
            .map(slugify)
            .unwrap_or_else(|| port.to_string());
        let key = format!("{}-{}", slugify(&repo_slug), key_name);

        let (serving, host, port_for_url) = match read_cached(wiki_id, port) {
            // Use `localhost` for the loopback path so the URL
            // displayed in the dashboard matches what users
            // typically expect to see (and what most documentation
            // for in-container tools uses). Apple Container's
            // publish-proxy responds on the IPv4 loopback, which
            // `localhost` resolves to first on macOS.
            Reachable::Loopback => (true, "localhost".to_string(), port),
            Reachable::Direct(ip) => (true, ip.to_string(), port),
            // Forwarder path: we own a local listener that proxies
            // to the container's vmnet address. The URL points at
            // `localhost:<local>` so Chrome treats the page as a
            // secure context (service workers register, no
            // HTTPS-First upgrade), while the bytes flow over the
            // working vmnet bridge under the hood.
            Reachable::Forwarder { local, .. } => (true, "localhost".to_string(), local),
            Reachable::No => (false, "localhost".to_string(), port),
        };
        let url = format!("{protocol}://{host}:{port_for_url}/");

        out.push(PortRow {
            external: port,
            internal: port,
            label,
            serving,
            url,
            key,
        });
    }
    out
}

/// Probe `<ip>:<port>` for an actually-working HTTP service.
///
/// We deliberately go beyond a bare `connect()`: Apple Container's
/// host-side publish-proxy on `127.0.0.1:<host_port>` will *accept*
/// the connection (so `connect` succeeds) and only RST once it tries
/// to dial its backend after we send bytes. On corp-laptop hosts
/// where a network filter intercepts loopback that RST is the only
/// observable signal that the proxy is unhealthy, so a pure connect
/// probe falsely reports the proxy as working.
///
/// Strategy: connect with a short timeout, send a minimal `HEAD /`
/// request, and require *at least one* byte back. A RST surfaces as
/// an `Err` from `read`/`write`; an `Ok(0)` (EOF before any bytes)
/// also counts as failure. This catches the publish-proxy RST while
/// still being fast enough to run inline in the dashboard refresh.
fn probe_http_at(ip: Ipv4Addr, port: u16) -> bool {
    let addr: SocketAddr = (ip, port).into();
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(250)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(250)));
    // Minimal HTTP/1.0 HEAD: avoids any virtual-host concerns and
    // skips chunked-body handling. We don't care about the status
    // code — any byte means there's a live HTTP server on the other
    // end.
    let req = b"HEAD / HTTP/1.0\r\n\r\n";
    if stream.write_all(req).is_err() {
        return false;
    }
    let mut buf = [0u8; 16];
    matches!(stream.read(&mut buf), Ok(n) if n > 0)
}

/// Try loopback first; on failure, fall back to the container's
/// direct vmnet address (if known). Sequential — each probe has a
/// short timeout, and a RST from the publish-proxy comes back
/// effectively instantly so the typical "loopback works" path stays
/// fast.
fn probe_reachability(
    host_port: u16,
    container_ipv4: Option<Ipv4Addr>,
    container_port: u16,
) -> Reachable {
    if probe_http_at(Ipv4Addr::LOCALHOST, host_port) {
        return Reachable::Loopback;
    }
    if let Some(ip) = container_ipv4 {
        if probe_http_at(ip, container_port) {
            return Reachable::Direct(ip);
        }
    }
    Reachable::No
}

/// Lowercase, replace runs of non-alphanumeric with `-`, trim `-`.
fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_dash = true;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

// ---------------------------------------------------------------------------
// Tauri command
// ---------------------------------------------------------------------------

#[command]
pub async fn wiki_container_ports(
    app: tauri::AppHandle,
    wiki_state: State<'_, WikiState>,
    wiki_id: String,
) -> Result<Vec<PortRow>, String> {
    let wiki = wiki_state
        .manager
        .get(&wiki_id)
        .map_err(|e| format!("wiki lookup failed: {e}"))?
        .ok_or_else(|| format!("unknown wiki: {wiki_id}"))?;
    let Some(local) = wiki.local_path.as_ref() else {
        return Ok(Vec::new());
    };
    let path = std::path::PathBuf::from(local);
    if !path.exists() {
        return Ok(Vec::new());
    }

    // Stale-green guard.
    //
    // Once every port settles, the poller retires itself and — by design —
    // deliberately keeps its claim, so later refreshes keep showing the last
    // result without re-spawning it (that was to stop `HEAD /` lines filling
    // the in-container access log). Nothing then notices when the container
    // goes away: Stop, a crash, or a `docker stop` run outside the app. A
    // once-green port therefore stayed green forever.
    //
    // So: whenever the cache claims something is serving, confirm a running
    // container still backs it. If not, drop the cache and release the claim
    // so the poller re-runs, finds no container and writes `No`. Skipped
    // entirely when nothing is green, so the steady state costs nothing.
    if rows_from_cache(&wiki_id, &path, submitted_config(&app, &wiki_id).as_ref())
        .iter()
        .any(|r| r.serving)
        && !container_running_for(&app, &wiki_id, &path).await
    {
        evict(&wiki_id);
        release_poller(&wiki_id);
        crate::wiki::forwarder::stop_all_for_wiki(&wiki_id);
    }

    // Record that the dashboard is interested in this wiki's ports
    // right now. The poller's idle-timeout check uses this so it
    // keeps running across the dashboard's refresh cycle, regardless
    // of whether the wiki is registered with `LocalSiteManager`.
    touch_request(&wiki_id);

    // Make sure a background poller is running for this wiki. The
    // poller exits on its own once the dashboard has stopped asking
    // for a while, so calling this on every dashboard refresh is
    // safe and cheap.
    if try_claim_poller(&wiki_id) {
        let wiki_id_owned = wiki_id.clone();
        let path_for_poller = path.clone();
        let app_handle = app.clone();
        tokio::spawn(async move {
            poll_wiki_ports(wiki_id_owned, path_for_poller, app_handle).await;
        });
    }

    // Pure read against the cache the poller is feeding — no probes
    // happen on the request path, so the dashboard never blocks on a
    // slow connect/RST cycle.
    let rows = rows_from_cache(&wiki_id, &path, submitted_config(&app, &wiki_id).as_ref());
    if rows.iter().any(|r| r.serving) {
        // Only log the "interesting" case — we don't want to spam
        // stderr on every 4s dashboard refresh while everything is
        // still grey, but we do want to confirm in the log that the
        // command is returning a serving=true row to the frontend
        // so we can tell command-side bugs apart from render-side
        // bugs.
        eprintln!(
            "[port-cmd] wiki_id={wiki_id} returning {} row(s) (serving): {:?}",
            rows.len(),
            rows.iter()
                .map(|r| (r.external, r.serving, r.url.as_str()))
                .collect::<Vec<_>>()
        );
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_basics() {
        assert_eq!(slugify("Jupyter Lab"), "jupyter-lab");
        assert_eq!(slugify("  weird??name!! "), "weird-name");
        assert_eq!(slugify("ALL_CAPS"), "all-caps");
    }

    // -----------------------------------------------------------------
    // Live network integration tests.
    //
    // These spin up real `std::net::TcpListener`s on `127.0.0.1:0`
    // (kernel-assigned port) and exercise `probe_http_at` /
    // `probe_reachability` against them. They cover the three cases we
    // actually care about on corp laptops:
    //   * a healthy HTTP server  → `Loopback`
    //   * a closed port          → `No`
    //   * accept-then-RST proxy  → `No`  (the Apple Container
    //     publish-proxy failure mode on the M2)
    // and the cache flip from `No` → `Loopback` once the server
    // starts answering.
    // -----------------------------------------------------------------

    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;

    /// Spawn a minimal HTTP/1.0 server bound to an ephemeral port.
    /// Returns the port and a stop flag — set the flag and the
    /// accept loop exits on its next iteration. Each connection
    /// gets a `200 OK` with a tiny body, which is enough for
    /// `probe_http_at` to see at least one byte.
    fn spawn_live_http_server() -> (u16, Arc<AtomicBool>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
        let port = listener.local_addr().unwrap().port();
        listener
            .set_nonblocking(true)
            .expect("set listener nonblocking");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = stop.clone();
        let handle = thread::spawn(move || {
            while !stop_clone.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut s, _)) => {
                        // Drain anything the client wrote (HEAD request)
                        // so we don't RST it on close.
                        let _ = s.set_read_timeout(Some(Duration::from_millis(50)));
                        let mut buf = [0u8; 256];
                        let _ = s.read(&mut buf);
                        let _ = s.write_all(b"HTTP/1.0 200 OK\r\nContent-Length: 2\r\n\r\nOK");
                        let _ = s.flush();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        (port, stop, handle)
    }

    /// Spawn a server that accepts and then immediately drops the
    /// connection without writing anything. Mirrors the Apple
    /// Container publish-proxy on the M2 closely enough for our
    /// probe: bare `connect()` succeeds, but the probe's `read`
    /// sees EOF (or in the real-world case a RST) — either way no
    /// bytes come back, which is what `probe_http_at` rejects.
    fn spawn_accept_then_close_server() -> (u16, Arc<AtomicBool>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).expect("set nonblocking");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = stop.clone();
        let handle = thread::spawn(move || {
            while !stop_clone.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((s, _)) => {
                        // Close immediately without writing. The probe
                        // will see EOF (`read` returns `Ok(0)`) which
                        // our `n > 0` requirement rejects — same outcome
                        // as a real RST from the publish-proxy.
                        drop(s);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        (port, stop, handle)
    }

    fn pick_unused_port() -> u16 {
        // Bind, read port, drop. There's a TOCTOU window but it's
        // fine for a unit test.
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    }

    #[test]
    fn probe_http_at_succeeds_against_live_server() {
        let (port, stop, handle) = spawn_live_http_server();
        // Tiny grace period so the listener is definitely accepting.
        thread::sleep(Duration::from_millis(20));
        let ok = probe_http_at(Ipv4Addr::LOCALHOST, port);
        stop.store(true, Ordering::SeqCst);
        let _ = handle.join();
        assert!(ok, "probe_http_at should succeed against live HTTP server");
    }

    #[test]
    fn probe_http_at_fails_against_closed_port() {
        let port = pick_unused_port();
        let ok = probe_http_at(Ipv4Addr::LOCALHOST, port);
        assert!(!ok, "probe_http_at should fail against closed port");
    }

    #[test]
    fn probe_http_at_fails_against_accept_then_rst() {
        // This is the Apple Container publish-proxy bug we're
        // working around. A bare `connect()` would falsely report
        // success here; we require at least one response byte.
        let (port, stop, handle) = spawn_accept_then_close_server();
        thread::sleep(Duration::from_millis(20));
        let ok = probe_http_at(Ipv4Addr::LOCALHOST, port);
        stop.store(true, Ordering::SeqCst);
        let _ = handle.join();
        assert!(
            !ok,
            "probe_http_at must reject accept-then-close (publish-proxy failure mode)"
        );
    }

    #[test]
    fn probe_reachability_prefers_loopback_when_both_work() {
        let (port, stop, handle) = spawn_live_http_server();
        thread::sleep(Duration::from_millis(20));
        // Pretend the same loopback server is also reachable as a
        // "direct" address — loopback should still win.
        let r = probe_reachability(port, Some(Ipv4Addr::LOCALHOST), port);
        stop.store(true, Ordering::SeqCst);
        let _ = handle.join();
        assert_eq!(r, Reachable::Loopback);
    }

    #[test]
    fn probe_reachability_falls_back_to_direct_when_loopback_fails() {
        // Closed loopback port + working "direct" server (also on
        // loopback, just to keep the test self-contained).
        let closed_loopback = pick_unused_port();
        let (direct_port, stop, handle) = spawn_live_http_server();
        thread::sleep(Duration::from_millis(20));
        let r = probe_reachability(closed_loopback, Some(Ipv4Addr::LOCALHOST), direct_port);
        stop.store(true, Ordering::SeqCst);
        let _ = handle.join();
        assert_eq!(r, Reachable::Direct(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn probe_reachability_returns_no_when_both_paths_fail() {
        let host_port = pick_unused_port();
        let direct_port = pick_unused_port();
        let r = probe_reachability(host_port, Some(Ipv4Addr::LOCALHOST), direct_port);
        assert_eq!(r, Reachable::No);
    }

    #[test]
    fn cache_round_trips() {
        // Use unique wiki id so we don't collide with other tests
        // running in parallel against the process-wide cache.
        let wiki_id = "test-cache-roundtrip";
        evict(wiki_id);
        assert_eq!(read_cached(wiki_id, 1234), Reachable::No);

        let mut m = HashMap::new();
        m.insert(1234u16, Reachable::Loopback);
        m.insert(5678u16, Reachable::Direct(Ipv4Addr::new(192, 168, 64, 5)));
        write_cached(wiki_id, m);

        assert_eq!(read_cached(wiki_id, 1234), Reachable::Loopback);
        assert_eq!(
            read_cached(wiki_id, 5678),
            Reachable::Direct(Ipv4Addr::new(192, 168, 64, 5))
        );
        assert_eq!(read_cached(wiki_id, 9999), Reachable::No);

        evict(wiki_id);
        assert_eq!(read_cached(wiki_id, 1234), Reachable::No);
    }

    #[test]
    fn poller_claim_is_idempotent() {
        let wiki_id = "test-claim-once";
        // Make sure we start clean even if a previous test crashed
        // mid-flight.
        release_poller(wiki_id);
        assert!(try_claim_poller(wiki_id), "first claim should win");
        assert!(
            !try_claim_poller(wiki_id),
            "second claim should be a no-op while first holds"
        );
        release_poller(wiki_id);
        assert!(try_claim_poller(wiki_id), "claim works again after release");
        release_poller(wiki_id);
    }

    #[test]
    fn sticky_holds_loopback_through_transient_failures() {
        // Regression: Apple Container's publish-proxy occasionally
        // glitches; a single failed probe must not flip the
        // dashboard link grey.
        let mut streak = 0u32;
        let prev = Reachable::Loopback;
        // Three flaky misses in a row — under budget, hold the OK.
        let r1 = apply_sticky(prev, Reachable::No, &mut streak, 4);
        assert_eq!(r1, Reachable::Loopback);
        assert_eq!(streak, 1);
        let r2 = apply_sticky(r1, Reachable::No, &mut streak, 4);
        assert_eq!(r2, Reachable::Loopback);
        assert_eq!(streak, 2);
        let r3 = apply_sticky(r2, Reachable::No, &mut streak, 4);
        assert_eq!(r3, Reachable::Loopback);
        assert_eq!(streak, 3);
        // Fourth miss exceeds budget → downgrade.
        let r4 = apply_sticky(r3, Reachable::No, &mut streak, 4);
        assert_eq!(r4, Reachable::No);
    }

    #[test]
    fn sticky_resets_streak_on_any_success() {
        let mut streak = 5u32; // pretend we were close to giving up
        let r = apply_sticky(Reachable::Loopback, Reachable::Loopback, &mut streak, 4);
        assert_eq!(r, Reachable::Loopback);
        assert_eq!(streak, 0);

        let mut streak = 3u32;
        let ip = Ipv4Addr::new(192, 168, 64, 9);
        let r = apply_sticky(Reachable::No, Reachable::Direct(ip), &mut streak, 4);
        assert_eq!(r, Reachable::Direct(ip));
        assert_eq!(streak, 0);
    }

    #[test]
    fn sticky_lets_no_pass_through_when_no_prior_success() {
        // If we've never seen a port serving, a failed probe stays
        // failed — no false positives.
        let mut streak = 0u32;
        let r = apply_sticky(Reachable::No, Reachable::No, &mut streak, 4);
        assert_eq!(r, Reachable::No);
        assert_eq!(streak, 0);
    }

    /// Drives the same logic the background poller runs each tick,
    /// without needing a Tauri `AppHandle` / `LocalSiteManager`. We
    /// start with a closed port (cache should report `No`), flip a
    /// live server on, run another "tick", and verify the cache
    /// flips to `Loopback`. This is the regression guard for "row
    /// stays grey forever" on the dashboard.
    #[tokio::test(flavor = "multi_thread")]
    async fn cache_flips_to_loopback_when_server_starts() {
        let wiki_id = "test-flip";
        evict(wiki_id);

        // 1. No server yet → tick should record `No`.
        let port = pick_unused_port();
        let initial = tokio::task::spawn_blocking(move || {
            let mut out = HashMap::new();
            out.insert(port, probe_reachability(port, None, port));
            out
        })
        .await
        .unwrap();
        write_cached(wiki_id, initial);
        assert_eq!(read_cached(wiki_id, port), Reachable::No);

        // 2. Bring up a live server on the same port. We can't
        // reuse `port` here because it's already been "consumed" by
        // `pick_unused_port`'s drop, so just bind a fresh one and
        // re-tick.
        let (live_port, stop, handle) = spawn_live_http_server();
        tokio::time::sleep(Duration::from_millis(20)).await;
        let p = live_port;
        let next = tokio::task::spawn_blocking(move || {
            let mut out = HashMap::new();
            out.insert(p, probe_reachability(p, None, p));
            out
        })
        .await
        .unwrap();
        write_cached(wiki_id, next);
        assert_eq!(read_cached(wiki_id, live_port), Reachable::Loopback);

        stop.store(true, Ordering::SeqCst);
        let _ = handle.join();
        evict(wiki_id);
    }

    /// A single-port config with optional attributes, standing in for what the
    /// dashboard submits.
    fn parsed_with_port(port: u16, label: Option<&str>) -> ParsedDevContainer {
        let attrs = label.map(|l| {
            let mut m = serde_json::Map::new();
            m.insert(
                port.to_string(),
                serde_json::json!({ "label": l, "protocol": "http" }),
            );
            serde_json::Value::Object(m)
        });
        ParsedDevContainer {
            forward_ports: vec![port],
            ports_attributes: attrs,
            ..Default::default()
        }
    }

    #[test]
    fn rows_follow_the_submitted_config_not_the_file_on_disk() {
        // The panel describes the config the container was created from,
        // because a container still running on the old config should not be
        // documented by a file the user is part-way through editing.
        let wiki_id = "test-submitted-config";
        let path = Path::new("/tmp/test-submitted-config");
        evict(wiki_id);

        let ports = |parsed: Option<&ParsedDevContainer>| {
            rows_from_cache(wiki_id, path, parsed)
                .iter()
                .map(|r| r.external)
                .collect::<Vec<_>>()
        };

        assert_eq!(ports(Some(&parsed_with_port(1111, None))), vec![1111]);

        // Re-submitting — which Start / Restart / Rebuild do after a config
        // change — is what moves the rows.
        let changed = ParsedDevContainer {
            forward_ports: vec![2222, 3333],
            ..Default::default()
        };
        assert_eq!(ports(Some(&changed)), vec![2222, 3333]);

        // "Nothing submitted yet" is the normal state just after launch, and
        // must read as "no rows" rather than as an error.
        assert!(ports(None).is_empty());
        evict(wiki_id);
    }

    #[test]
    fn rows_take_their_label_from_the_submitted_ports_attributes() {
        // The label comes from `portsAttributes`, which the engine carries
        // through verbatim. If that field stopped arriving this would show a
        // bare port number instead.
        let wiki_id = "test-port-label";
        evict(wiki_id);

        let rows = rows_from_cache(
            wiki_id,
            Path::new("/tmp/test-port-label"),
            Some(&parsed_with_port(9119, Some("Dashboard"))),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label.as_deref(), Some("Dashboard"));
        assert_eq!(rows[0].url, "http://localhost:9119/");

        // Unknown keys are carried through without disturbing the ones we use.
        let mut attrs = serde_json::Map::new();
        attrs.insert(
            "9119".to_string(),
            serde_json::json!({ "label": "Dashboard", "onAutoForward": "openBrowser" }),
        );
        let with_extra = ParsedDevContainer {
            forward_ports: vec![9119],
            ports_attributes: Some(serde_json::Value::Object(attrs)),
            ..Default::default()
        };
        let rows = rows_from_cache(wiki_id, Path::new("/tmp/test-port-label"), Some(&with_extra));
        assert_eq!(rows[0].label.as_deref(), Some("Dashboard"));

        evict(wiki_id);
    }

    #[test]
    fn rows_from_cache_marks_serving_only_when_cached_ok() {
        let wiki_id = "test-rows-from-cache";
        let port = pick_unused_port();
        let parsed = parsed_with_port(port, Some("Test"));
        let path = Path::new("/tmp/test-rows-from-cache");
        evict(wiki_id);

        // No cache entry → not serving.
        let rows = rows_from_cache(wiki_id, path, Some(&parsed));
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].serving);
        assert!(rows[0].url.starts_with("http://localhost:"));

        // Cache says Loopback → serving=true with localhost host.
        let mut m = HashMap::new();
        m.insert(port, Reachable::Loopback);
        write_cached(wiki_id, m);
        let rows = rows_from_cache(wiki_id, path, Some(&parsed));
        assert!(rows[0].serving);
        assert_eq!(rows[0].url, format!("http://localhost:{port}/"));

        // Cache says Direct(ip) → serving=true with that IP as host.
        let mut m = HashMap::new();
        m.insert(port, Reachable::Direct(Ipv4Addr::new(192, 168, 64, 7)));
        write_cached(wiki_id, m);
        let rows = rows_from_cache(wiki_id, path, Some(&parsed));
        assert!(rows[0].serving);
        assert_eq!(rows[0].url, format!("http://192.168.64.7:{port}/"));

        // Cache says Forwarder { local, .. } → serving=true with
        // localhost host and the *local* port (so Chrome treats it
        // as a secure context for service workers).
        let mut m = HashMap::new();
        m.insert(
            port,
            Reachable::Forwarder {
                local: 54321,
                target: Ipv4Addr::new(192, 168, 64, 7),
            },
        );
        write_cached(wiki_id, m);
        let rows = rows_from_cache(wiki_id, path, Some(&parsed));
        assert!(rows[0].serving);
        assert_eq!(rows[0].url, "http://localhost:54321/");

        evict(wiki_id);
    }

    #[test]
    fn same_path_matches_equivalent_spellings() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().canonicalize().unwrap();
        let real_str = real.to_str().unwrap();

        assert!(same_path(real_str, &real));
        // A runtime may well report the same directory with a trailing
        // slash, and the wiki's stored path can differ by a symlinked
        // prefix, so the comparison must not be a naive string equality.
        assert!(same_path(&format!("{real_str}/"), &real), "trailing slash");
        assert!(!same_path("/definitely/not/a/real/path", &real));
    }

    /// Mirror of `cache_flips_to_loopback_when_server_starts`: once a port
    /// has gone green it must be able to go back to grey.
    ///
    /// The regression this guards is the poller retiring itself *and holding
    /// its claim* once every port settled, so nothing ever re-probed and a
    /// stopped container left the row green forever. `wiki_container_ports`
    /// breaks that by evicting the cache when it finds the backing container
    /// gone; this asserts that eviction really does drop the serving verdict.
    #[test]
    fn evict_clears_a_serving_row_so_a_stopped_container_goes_grey() {
        let wiki_id = "test-stale-green";
        let parsed = parsed_with_port(8642, None);
        let path = Path::new("/tmp/test-stale-green");
        evict(wiki_id);

        let mut served = HashMap::new();
        served.insert(8642u16, Reachable::Loopback);
        write_cached(wiki_id, served);
        assert!(
            rows_from_cache(wiki_id, path, Some(&parsed))[0].serving,
            "precondition: the cached row reads as serving"
        );

        // This is what `wiki_container_ports` does once it establishes that
        // no running container backs the cache any more.
        evict(wiki_id);
        let rows = rows_from_cache(wiki_id, path, Some(&parsed));
        assert_eq!(rows.len(), 1);
        assert!(
            !rows[0].serving,
            "after eviction a port must not still read as serving"
        );

        evict(wiki_id);
    }
}
