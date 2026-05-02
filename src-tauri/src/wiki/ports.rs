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

use crate::tools::devcontainer_config::{find_devcontainer_configs, load_config};
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

#[cfg(test)]
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
        (Reachable::Loopback | Reachable::Direct(_), Reachable::No) => {
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
            let needs_refresh = last_inspect_at
                .map(|t| t.elapsed() >= INSPECT_REFRESH_INTERVAL)
                .unwrap_or(true);

            let site_state = app.state::<LocalSiteManager>();
            let known_name = site_state.get(&wiki_id).map(|s| s.serve_container);
            let needs_refresh =
                needs_refresh || cached_for_container.as_deref() != known_name.as_deref();

            if needs_refresh {
                let bin = crate::tools::apple_container::detect()
                    .path
                    .unwrap_or_else(|| std::path::PathBuf::from("container"));
                if let Some(name) = known_name {
                    let resolved =
                        crate::tools::apple_container::inspect_container_ipv4(&bin, &name)
                            .await
                            .and_then(|s| s.parse::<Ipv4Addr>().ok());
                    cached_ip = resolved;
                    cached_for_container = Some(name);
                } else {
                    // Fall back to discovering the container by
                    // mount source. One `container ls` per refresh
                    // interval — same load profile as inspecting a
                    // known name.
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
        // so an edit to `devcontainer.json` is picked up without a
        // restart.
        let ports = configured_ports(&local_path);
        if ports.is_empty() {
            tokio::time::sleep(POLL_INTERVAL_FAST).await;
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
            && effective
                .values()
                .all(|r| matches!(r, Reachable::Loopback | Reachable::Direct(_)));
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

/// Read `devcontainer.json` and return the parsed list of forwarded
/// ports. Returns an empty vec on any parse / IO error.
fn configured_ports(local_path: &Path) -> Vec<u16> {
    let configs = find_devcontainer_configs(local_path);
    let Some(cfg_path) = configs.first() else {
        return Vec::new();
    };
    let Ok(cfg) = load_config(cfg_path) else {
        return Vec::new();
    };
    cfg.forward_ports.iter().filter_map(parse_port).collect()
}

// ---------------------------------------------------------------------------
// Row building (cache → PortRow[])
// ---------------------------------------------------------------------------

/// Build port rows for `local_path` by reading its `devcontainer.json`
/// and the cache populated by the background poller. Pure read — no
/// network I/O happens here, so the Tauri command stays snappy.
fn rows_from_cache(wiki_id: &str, local_path: &Path) -> Vec<PortRow> {
    let configs = find_devcontainer_configs(local_path);
    let Some(cfg_path) = configs.first() else {
        return Vec::new();
    };
    let Ok(cfg) = load_config(cfg_path) else {
        return Vec::new();
    };

    let repo_slug = local_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("wiki")
        .to_string();

    let mut out = Vec::new();
    for v in &cfg.forward_ports {
        let Some(port) = parse_port(v) else { continue };
        let attr = cfg
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

        let (serving, host) = match read_cached(wiki_id, port) {
            // Use `localhost` for the loopback path so the URL
            // displayed in the dashboard matches what users
            // typically expect to see (and what most documentation
            // for in-container tools uses). Apple Container's
            // publish-proxy responds on the IPv4 loopback, which
            // `localhost` resolves to first on macOS.
            Reachable::Loopback => (true, "localhost".to_string()),
            Reachable::Direct(ip) => (true, ip.to_string()),
            Reachable::No => (false, "localhost".to_string()),
        };
        let url = format!("{protocol}://{host}:{port}/");

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

/// Parse a single `forwardPorts` entry. Spec allows bare integers
/// (`8000`) or strings (`"8000"`, `"host:8000"`, `"8000:8000"`); we
/// treat the right-hand side as the container port.
fn parse_port(v: &serde_json::Value) -> Option<u16> {
    if let Some(n) = v.as_u64() {
        if (1..=u16::MAX as u64).contains(&n) {
            return Some(n as u16);
        }
    }
    if let Some(s) = v.as_str() {
        let tail = s.rsplit(':').next().unwrap_or(s);
        if let Ok(n) = tail.trim().parse::<u16>() {
            if n != 0 {
                return Some(n);
            }
        }
    }
    None
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
    let rows = rows_from_cache(&wiki_id, &path);
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
    fn parse_port_handles_int_and_string() {
        assert_eq!(parse_port(&serde_json::json!(8000)), Some(8000));
        assert_eq!(parse_port(&serde_json::json!("8000")), Some(8000));
        assert_eq!(parse_port(&serde_json::json!("host:8000")), Some(8000));
        assert_eq!(parse_port(&serde_json::json!("8000:8001")), Some(8001));
        assert_eq!(parse_port(&serde_json::json!(0)), None);
        assert_eq!(parse_port(&serde_json::json!("nope")), None);
    }

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

    #[test]
    fn rows_from_cache_marks_serving_only_when_cached_ok() {
        // Build a temp wiki dir with a `.devcontainer/devcontainer.json`
        // that forwards a single port, then verify `rows_from_cache`
        // reflects whatever's in the cache.
        let tmp = tempfile::tempdir().expect("tempdir");
        let dc = tmp.path().join(".devcontainer");
        std::fs::create_dir_all(&dc).unwrap();
        let port = pick_unused_port();
        let cfg = format!(
            r#"{{
                "name": "test",
                "image": "scratch",
                "forwardPorts": [{port}],
                "portsAttributes": {{
                    "{port}": {{ "label": "Test", "protocol": "http" }}
                }}
            }}"#
        );
        std::fs::write(dc.join("devcontainer.json"), cfg).unwrap();

        let wiki_id = "test-rows-from-cache";
        evict(wiki_id);

        // No cache entry → not serving.
        let rows = rows_from_cache(wiki_id, tmp.path());
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].serving);
        assert!(rows[0].url.starts_with("http://localhost:"));

        // Cache says Loopback → serving=true with localhost host.
        let mut m = HashMap::new();
        m.insert(port, Reachable::Loopback);
        write_cached(wiki_id, m);
        let rows = rows_from_cache(wiki_id, tmp.path());
        assert!(rows[0].serving);
        assert_eq!(rows[0].url, format!("http://localhost:{port}/"));

        // Cache says Direct(ip) → serving=true with that IP as host.
        let mut m = HashMap::new();
        m.insert(port, Reachable::Direct(Ipv4Addr::new(192, 168, 64, 7)));
        write_cached(wiki_id, m);
        let rows = rows_from_cache(wiki_id, tmp.path());
        assert!(rows[0].serving);
        assert_eq!(rows[0].url, format!("http://192.168.64.7:{port}/"));

        evict(wiki_id);
    }
}
