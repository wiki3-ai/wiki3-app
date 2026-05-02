//! In-process TCP forwarder for Apple Container's vmnet path.
//!
//! Some macOS hosts — Tahoe in particular, when paired with certain
//! corporate network filters / EDR agents / `pfctl` rules — break
//! Apple Container's host-port publish-proxy on `127.0.0.1`: bare
//! `connect()` succeeds but the proxy then ACCEPT-then-RSTs once it
//! tries to dial its in-VM backend. The container itself remains
//! reachable directly via its vmnet IPv4 (`192.168.64.x`), so curl
//! against that address works, but Chrome refuses to load it for
//! two reasons:
//!
//! 1. Service workers (which JupyterLite needs) only register on
//!    `localhost`, `127.0.0.1`, or HTTPS — not arbitrary RFC1918
//!    addresses.
//! 2. HTTPS-First Mode silently upgrades `http://192.168.64.x:port`
//!    and the resulting `https://...` connect fails, leaving the
//!    user on `chrome-error://chromewebdata` with the cryptic
//!    "Unsafe attempt to load URL ... from frame with URL
//!    chrome-error://chromewebdata" message.
//!
//! This module bridges the gap by binding a TCP listener on
//! `127.0.0.1:<ephemeral>` inside the wiki3-app process and shovelling
//! bytes between each accepted connection and `<container_ip>:<port>`.
//! From Chrome's perspective the URL is `http://localhost:<local>/`,
//! which is a secure context (SW works) and not subject to the
//! HTTPS-First upgrade. From the publish-proxy's perspective there
//! is no proxy involved — we go straight from the host kernel out
//! over the vmnet bridge to the container.
//!
//! Lifecycle:
//! * The port poller (`wiki/ports.rs`) calls [`ensure`] when its
//!   probe sequence finds that loopback is broken but the direct
//!   vmnet address answers.
//! * Each forwarder is keyed by `(wiki_id, container_port)`. If the
//!   target IP changes (container restart), the existing tunnel is
//!   torn down and a new one is started.
//! * [`stop`] / [`stop_all_for_wiki`] tear forwarders down — the
//!   poller calls these when loopback recovers (rare) or when the
//!   poller itself is shutting down because the wiki was closed.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::{Arc, Mutex, OnceLock};

use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

#[derive(Debug)]
struct Forwarder {
    local_port: u16,
    target: SocketAddrV4,
    /// Shutdown channel — setting `true` is observed by the
    /// accept loop and every per-connection task via
    /// `Receiver::changed()`. Using `watch` rather than `Notify`
    /// avoids the race where a `notify_waiters()` call lands
    /// before the spawned task has registered its waiter, which
    /// would silently leak the listener and its port.
    stop: watch::Sender<bool>,
}

#[derive(Default)]
struct Registry {
    /// Keyed by (wiki_id, container_port). Container port is part
    /// of the key because a single wiki can publish more than one
    /// port (Jupyter on 8888 + an app on 8000, say) and each gets
    /// its own tunnel.
    entries: HashMap<(String, u16), Forwarder>,
}

fn registry() -> &'static Arc<Mutex<Registry>> {
    static CELL: OnceLock<Arc<Mutex<Registry>>> = OnceLock::new();
    CELL.get_or_init(|| Arc::new(Mutex::new(Registry::default())))
}

/// Ensure a forwarder exists from `127.0.0.1:<ephemeral>` to
/// `target_ip:container_port` for `(wiki_id, container_port)`.
/// Returns the local loopback port the dashboard should send users
/// to. If a forwarder for this key already exists *and* its target
/// matches, that port is returned unchanged (so dashboard URLs are
/// stable across poller ticks). If the target changed (e.g. the
/// container was restarted on a new IP), the old forwarder is torn
/// down and a new one is started on a fresh ephemeral port.
///
/// Returns `None` only if binding the local listener fails — in
/// that case the caller should fall back to the direct vmnet URL.
pub async fn ensure(wiki_id: &str, container_port: u16, target_ip: Ipv4Addr) -> Option<u16> {
    let target = SocketAddrV4::new(target_ip, container_port);

    // Fast path: existing forwarder with the same target.
    {
        let g = registry().lock().unwrap();
        if let Some(f) = g.entries.get(&(wiki_id.to_string(), container_port)) {
            if f.target == target {
                return Some(f.local_port);
            }
        }
    }

    // Either no forwarder, or target changed. Stop the old one (if
    // any) before binding a new listener so the port is released.
    stop(wiki_id, container_port);

    let listener = match TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "[forwarder] bind 127.0.0.1:0 failed for wiki={wiki_id} port={container_port}: {e}"
            );
            return None;
        }
    };
    let local_port = match listener.local_addr() {
        Ok(a) => a.port(),
        Err(e) => {
            eprintln!("[forwarder] local_addr failed: {e}");
            return None;
        }
    };
    let stop_notify = watch::channel(false).0;
    let stop_rx = stop_notify.subscribe();

    {
        let mut g = registry().lock().unwrap();
        g.entries.insert(
            (wiki_id.to_string(), container_port),
            Forwarder {
                local_port,
                target,
                stop: stop_notify,
            },
        );
    }

    eprintln!(
        "[forwarder] start wiki={wiki_id} container_port={container_port} \
         localhost:{local_port} -> {target}"
    );

    let wiki_id_owned = wiki_id.to_string();
    tokio::spawn(async move {
        run_accept_loop(wiki_id_owned, container_port, target, listener, stop_rx).await;
    });

    Some(local_port)
}

/// Tear down the forwarder for `(wiki_id, container_port)` if one
/// exists. Idempotent.
pub fn stop(wiki_id: &str, container_port: u16) {
    let removed = {
        let mut g = registry().lock().unwrap();
        g.entries.remove(&(wiki_id.to_string(), container_port))
    };
    if let Some(f) = removed {
        eprintln!(
            "[forwarder] stop wiki={wiki_id} container_port={container_port} \
             localhost:{} -> {}",
            f.local_port, f.target
        );
        let _ = f.stop.send(true);
    }
}

/// Tear down every forwarder belonging to `wiki_id`.
pub fn stop_all_for_wiki(wiki_id: &str) {
    let removed: Vec<((String, u16), Forwarder)> = {
        let mut g = registry().lock().unwrap();
        let keys: Vec<(String, u16)> = g
            .entries
            .keys()
            .filter(|(w, _)| w == wiki_id)
            .cloned()
            .collect();
        keys.into_iter()
            .filter_map(|k| g.entries.remove(&k).map(|v| (k, v)))
            .collect()
    };
    for ((_, port), f) in removed {
        eprintln!(
            "[forwarder] stop wiki={wiki_id} container_port={port} localhost:{} -> {}",
            f.local_port, f.target
        );
        let _ = f.stop.send(true);
    }
}

/// Look up the local loopback port currently servicing
/// `(wiki_id, container_port)`, if any.
pub fn local_port(wiki_id: &str, container_port: u16) -> Option<u16> {
    let g = registry().lock().unwrap();
    g.entries
        .get(&(wiki_id.to_string(), container_port))
        .map(|f| f.local_port)
}

/// Snapshot of the live forwarder registry: every entry as
/// `(wiki_id, container_port, local_port, target)`. Used by the
/// diagnostic report so a Tahoe user can see exactly which
/// loopback listeners belong to wiki3-app and which container
/// each one is bridging to.
pub fn snapshot() -> Vec<(String, u16, u16, SocketAddrV4)> {
    let g = registry().lock().unwrap();
    let mut out: Vec<_> = g
        .entries
        .iter()
        .map(|((wiki, port), f)| (wiki.clone(), *port, f.local_port, f.target))
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    out
}

async fn run_accept_loop(
    wiki_id: String,
    container_port: u16,
    target: SocketAddrV4,
    listener: TcpListener,
    mut stop: watch::Receiver<bool>,
) {
    // If `stop()` was called before this task got scheduled, the
    // current value will already be `true` and we exit immediately.
    if *stop.borrow() {
        return;
    }
    loop {
        tokio::select! {
            biased;
            _ = stop.changed() => {
                break;
            }
            res = listener.accept() => {
                match res {
                    Ok((client, _peer)) => {
                        let stop = stop.clone();
                        tokio::spawn(async move {
                            handle_connection(client, target, stop).await;
                        });
                    }
                    Err(e) => {
                        // accept errors are usually transient (file
                        // descriptor exhaustion, etc.); log and try
                        // again rather than dying silently.
                        eprintln!(
                            "[forwarder] accept error wiki={wiki_id} port={container_port}: {e}"
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                }
            }
        }
    }
    eprintln!(
        "[forwarder] accept loop exit wiki={wiki_id} container_port={container_port} target={target}"
    );
}

async fn handle_connection(
    mut client: TcpStream,
    target: SocketAddrV4,
    mut stop: watch::Receiver<bool>,
) {
    let connect = TcpStream::connect(target);
    let mut server = match tokio::time::timeout(std::time::Duration::from_secs(5), connect).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            eprintln!("[forwarder] dial {target} failed: {e}");
            return;
        }
        Err(_) => {
            eprintln!("[forwarder] dial {target} timed out");
            return;
        }
    };
    // Disable Nagle on both ends — most wiki traffic is interactive
    // (XHR + small JSON), and a 40ms ack delay on top of bridge100
    // adds up fast for SPA loads.
    let _ = client.set_nodelay(true);
    let _ = server.set_nodelay(true);

    tokio::select! {
        biased;
        _ = stop.changed() => {}
        res = tokio::io::copy_bidirectional(&mut client, &mut server) => {
            if let Err(e) = res {
                // EOF / reset is expected for ordinary HTTP/1.1
                // close-after-response; only log unusual cases.
                let kind = e.kind();
                use std::io::ErrorKind::*;
                if !matches!(kind, UnexpectedEof | BrokenPipe | ConnectionReset | NotConnected) {
                    eprintln!("[forwarder] copy_bidirectional error: {e}  (kind={kind:?})");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::Duration;

    /// Tiny synchronous backend to act as the "container" side of
    /// the tunnel. Listens on `127.0.0.1:0`, replies to one HEAD
    /// request per connection.
    fn spawn_backend() -> (SocketAddrV4, Arc<AtomicBool>, thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_c = stop.clone();
        let h = thread::spawn(move || {
            while !stop_c.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut s, _)) => {
                        let _ = s.set_read_timeout(Some(Duration::from_millis(100)));
                        let mut buf = [0u8; 256];
                        let _ = s.read(&mut buf);
                        let _ = s.write_all(
                            b"HTTP/1.0 200 OK\r\nContent-Length: 7\r\n\r\nHELLO!!",
                        );
                        let _ = s.flush();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        (SocketAddrV4::new(Ipv4Addr::LOCALHOST, port), stop, h)
    }

    fn drive_request(port: u16) -> std::io::Result<String> {
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port))?;
        s.set_read_timeout(Some(Duration::from_secs(2)))?;
        s.write_all(b"HEAD / HTTP/1.0\r\n\r\n")?;
        let mut buf = String::new();
        s.read_to_string(&mut buf).ok();
        Ok(buf)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ensure_forwards_bytes_to_target() {
        let (target, stop_be, h_be) = spawn_backend();
        let local = ensure("test-fwd-basic", target.port(), *target.ip())
            .await
            .expect("forwarder bound");
        // Tiny grace period so the accept loop is parked on accept.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let body = tokio::task::spawn_blocking(move || drive_request(local).unwrap())
            .await
            .unwrap();
        assert!(body.contains("200 OK"), "expected proxied response, got: {body}");
        assert!(body.contains("HELLO!!"), "expected proxied body, got: {body}");

        stop("test-fwd-basic", target.port());
        stop_be.store(true, Ordering::SeqCst);
        let _ = h_be.join();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ensure_is_idempotent_for_same_target() {
        let (target, stop_be, h_be) = spawn_backend();
        let p1 = ensure("test-fwd-idem", target.port(), *target.ip())
            .await
            .unwrap();
        let p2 = ensure("test-fwd-idem", target.port(), *target.ip())
            .await
            .unwrap();
        assert_eq!(p1, p2, "second ensure should return same local port");

        stop("test-fwd-idem", target.port());
        stop_be.store(true, Ordering::SeqCst);
        let _ = h_be.join();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stop_releases_local_port() {
        let (target, stop_be, h_be) = spawn_backend();
        let local = ensure("test-fwd-stop", target.port(), *target.ip())
            .await
            .unwrap();
        assert_eq!(local_port("test-fwd-stop", target.port()), Some(local));

        stop("test-fwd-stop", target.port());
        // Give the accept loop a tick to unwind.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(local_port("test-fwd-stop", target.port()), None);

        // New connections to the (now-closed) local port must fail.
        let blocked = tokio::task::spawn_blocking(move || {
            std::net::TcpStream::connect_timeout(
                &SocketAddr::from(([127, 0, 0, 1], local)),
                Duration::from_millis(200),
            )
            .is_err()
        })
        .await
        .unwrap();
        assert!(blocked, "local port should be closed after stop()");

        stop_be.store(true, Ordering::SeqCst);
        let _ = h_be.join();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stop_all_for_wiki_clears_every_port() {
        let (t1, s1, h1) = spawn_backend();
        let (t2, s2, h2) = spawn_backend();
        ensure("test-fwd-multi", t1.port(), *t1.ip()).await.unwrap();
        ensure("test-fwd-multi", t2.port(), *t2.ip()).await.unwrap();
        assert!(local_port("test-fwd-multi", t1.port()).is_some());
        assert!(local_port("test-fwd-multi", t2.port()).is_some());

        stop_all_for_wiki("test-fwd-multi");
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(local_port("test-fwd-multi", t1.port()).is_none());
        assert!(local_port("test-fwd-multi", t2.port()).is_none());

        s1.store(true, Ordering::SeqCst);
        s2.store(true, Ordering::SeqCst);
        let _ = h1.join();
        let _ = h2.join();
    }
}
