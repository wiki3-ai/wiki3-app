//! "Diagnose me" report generation.
//!
//! Collects the same evidence as `scripts/diagnose-loopback.sh` —
//! Apple Container state, host loopback behavior, packet filter
//! rules, network interfaces, etc. — and writes a plain-text
//! report to a timestamped file. Returned to the frontend so the
//! dashboard can reveal it in Finder for the user to attach to a
//! bug report.
//!
//! Each step is independent and best-effort: a failure in any one
//! check is captured in the output rather than aborting the run.

use std::fmt::Write as _;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::Command as StdCommand;
use std::time::Duration;

use tauri::{command, Manager};
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream as TokioTcpStream;
use tokio::process::Command;

const PROBE_CONNECT_TIMEOUT: Duration = Duration::from_millis(2000);
const PROBE_READ_TIMEOUT: Duration = Duration::from_millis(2000);

/// Run the diagnostics, write a report file under the app's log
/// directory, and return the absolute path. The frontend can then
/// pass the path to `reveal_path` to surface the file in Finder.
#[command]
pub async fn run_diagnostic_report(app: tauri::AppHandle) -> Result<String, String> {
    let report = build_report().await;

    let log_dir: PathBuf = app
        .path()
        .app_log_dir()
        .map_err(|e| format!("could not resolve app log dir: {e}"))?;
    std::fs::create_dir_all(&log_dir).map_err(|e| format!("create {}: {e}", log_dir.display()))?;

    let stamp = chrono_like_stamp();
    let path = log_dir.join(format!("wiki3-diagnostics-{stamp}.txt"));
    std::fs::write(&path, report).map_err(|e| format!("write report: {e}"))?;

    eprintln!("[diagnostics] wrote {}", path.display());
    Ok(path.to_string_lossy().into_owned())
}

async fn build_report() -> String {
    let mut buf = String::with_capacity(8 * 1024);
    let _ = writeln!(buf, "# wiki3-app diagnostics");
    let _ = writeln!(buf, "generated: {}", now_rfc3339());
    let _ = writeln!(buf);

    section(&mut buf, "system");
    capture(&mut buf, "sw_vers", &["sw_vers"]).await;
    capture(&mut buf, "uname -a", &["uname", "-a"]).await;

    let bin = crate::tools::apple_container::detect()
        .path
        .unwrap_or_else(|| PathBuf::from("container"));
    section(&mut buf, "apple container");
    capture(
        &mut buf,
        "container --version",
        &[bin.to_string_lossy().as_ref(), "--version"],
    )
    .await;
    capture(
        &mut buf,
        "container system status",
        &[bin.to_string_lossy().as_ref(), "system", "status"],
    )
    .await;
    capture(
        &mut buf,
        "container ls -a",
        &[bin.to_string_lossy().as_ref(), "ls", "-a"],
    )
    .await;
    let inventory = capture_string(&bin.to_string_lossy(), &["ls", "--format", "json"])
        .await
        .unwrap_or_default();
    let containers = parse_running_containers(&inventory);

    if containers.is_empty() {
        let _ = writeln!(
            buf,
            "(no running containers — host-port probe steps will be skipped)"
        );
    } else {
        for c in &containers {
            let _ = writeln!(
                buf,
                "container: name={} ipv4={:?} hostPorts={:?}",
                c.name, c.ipv4, c.host_ports
            );
        }
    }

    section(&mut buf, "host listening sockets");
    capture(
        &mut buf,
        "lsof -nP -iTCP -sTCP:LISTEN",
        &["lsof", "-nP", "-iTCP", "-sTCP:LISTEN"],
    )
    .await;
    capture(
        &mut buf,
        "netstat -an -p tcp (head)",
        &["sh", "-c", "netstat -an -p tcp | head -80"],
    )
    .await;

    section(&mut buf, "loopback probe (the failing path on bad hosts)");
    for c in &containers {
        for &port in &c.host_ports {
            probe_tcp_report(&mut buf, "127.0.0.1", port).await;
            probe_tcp_report(&mut buf, "::1", port).await;
        }
    }

    section(&mut buf, "direct vmnet probe (the working path)");
    for c in &containers {
        if let Some(ip) = c.ipv4.as_ref().and_then(|s| s.parse::<Ipv4Addr>().ok()) {
            for &port in &c.host_ports {
                probe_tcp_report(&mut buf, &ip.to_string(), port).await;
            }
        }
    }

    section(&mut buf, "control: our own loopback listener");
    self_test_loopback(&mut buf).await;

    section(&mut buf, "interfaces");
    capture(
        &mut buf,
        "ifconfig (ipv4 only)",
        &[
            "sh",
            "-c",
            "ifconfig | grep -E '^[a-z0-9]+:|inet ' | grep -v inet6",
        ],
    )
    .await;
    capture(
        &mut buf,
        "netstat -rn -f inet (head)",
        &["sh", "-c", "netstat -rn -f inet | head -40"],
    )
    .await;

    section(&mut buf, "vpn / network filter / EDR processes");
    capture(
        &mut buf,
        "ps + filter",
        &[
            "sh",
            "-c",
            "ps -axo pid,comm | grep -Ei 'forti|zscaler|netskope|crowdstrike|cisco|anyconnect|globalprotect|umbrella|cloudflare-warp|tailscale|cylance|sentinelone|jamf|carbonblack|splashtop' | grep -v grep || true",
        ],
    )
    .await;

    section(&mut buf, "launchd entries mentioning container");
    capture(
        &mut buf,
        "launchctl list | grep -i container",
        &[
            "sh",
            "-c",
            "launchctl list 2>&1 | grep -i container || true",
        ],
    )
    .await;

    section(&mut buf, "wiki3-app port poller cache snapshot");
    let _ = writeln!(
        buf,
        "(see app stderr for live `[port-poller]` / `[port-cmd]` lines)",
    );
    let _ = writeln!(
        buf,
        "(re-run wiki3-app with WIKI3_KEEP_PROBING=1 to keep the poller alive after settling)",
    );

    let _ = writeln!(buf);
    let _ = writeln!(buf, "# end of report");
    buf
}

fn section(buf: &mut String, title: &str) {
    let _ = writeln!(buf);
    let _ = writeln!(buf, "========== {title} ==========");
}

async fn capture(buf: &mut String, label: &str, argv: &[&str]) {
    let _ = writeln!(buf, "\n$ {label}");
    let (program, args) = match argv.split_first() {
        Some(s) => s,
        None => {
            let _ = writeln!(buf, "(no command)");
            return;
        }
    };
    let fut = Command::new(program).args(args).output();
    match tokio::time::timeout(Duration::from_secs(8), fut).await {
        Ok(Ok(out)) => {
            let _ = buf.write_str(&String::from_utf8_lossy(&out.stdout));
            let stderr = String::from_utf8_lossy(&out.stderr);
            if !stderr.trim().is_empty() {
                let _ = writeln!(buf, "[stderr] {}", stderr.trim());
            }
            let _ = writeln!(buf, "(exit={})", out.status.code().unwrap_or(-1));
        }
        Ok(Err(e)) => {
            let _ = writeln!(buf, "(spawn failed: {e})");
        }
        Err(_) => {
            let _ = writeln!(buf, "(timed out after 8s)");
        }
    }
}

async fn capture_string(program: &str, args: &[&str]) -> Option<String> {
    let fut = Command::new(program).args(args).output();
    let out = tokio::time::timeout(Duration::from_secs(5), fut)
        .await
        .ok()?
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[derive(Debug)]
struct ContainerSummary {
    name: String,
    ipv4: Option<String>,
    host_ports: Vec<u16>,
}

fn parse_running_containers(inventory_json: &str) -> Vec<ContainerSummary> {
    let v: serde_json::Value = match serde_json::from_str(inventory_json.trim()) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let arr = match v.as_array() {
        Some(a) => a,
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    for obj in arr {
        let cfg = obj.get("configuration").unwrap_or(obj);
        let name = obj
            .get("name")
            .or_else(|| obj.get("Name"))
            .or_else(|| cfg.get("id"))
            .or_else(|| obj.get("id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let networks = obj
            .get("networks")
            .or_else(|| cfg.get("networks"))
            .and_then(|n| n.as_array());
        let ipv4 = networks
            .and_then(|nets| nets.first())
            .and_then(|n| n.get("ipv4Address"))
            .and_then(|s| s.as_str())
            .and_then(|raw| raw.split('/').next())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let host_ports = cfg
            .get("publishedPorts")
            .and_then(|p| p.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| p.get("hostPort").and_then(|v| v.as_u64()))
                    .filter(|n| (1..=u16::MAX as u64).contains(n))
                    .map(|n| n as u16)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if let Some(name) = name {
            out.push(ContainerSummary {
                name,
                ipv4,
                host_ports,
            });
        }
    }
    out
}

/// Connect to `<host>:<port>`, send a HEAD request, classify the
/// failure mode (connection refused / accept-then-RST / timeout /
/// HTTP response). This is the core signal: ACCEPT-then-RST on
/// `127.0.0.1` with a working response over the vmnet IP is the
/// fingerprint of a broken host-side publish-proxy.
async fn probe_tcp_report(buf: &mut String, host: &str, port: u16) {
    let label = format!("{host}:{port}");
    let _ = writeln!(buf, "\n$ probe http://{label}/");
    let addr_str = format!("{host}:{port}");
    let connect =
        tokio::time::timeout(PROBE_CONNECT_TIMEOUT, TokioTcpStream::connect(&addr_str)).await;
    let mut stream = match connect {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            let _ = writeln!(buf, "  connect failed: {e}");
            return;
        }
        Err(_) => {
            let _ = writeln!(
                buf,
                "  connect timed out after {}ms",
                PROBE_CONNECT_TIMEOUT.as_millis()
            );
            return;
        }
    };
    let _ = writeln!(buf, "  connect OK");

    use tokio::io::AsyncWriteExt;
    let req = format!("HEAD / HTTP/1.0\r\nHost: {host}\r\n\r\n");
    if let Err(e) = stream.write_all(req.as_bytes()).await {
        let _ = writeln!(buf, "  write failed: {e}");
        return;
    }
    let mut tmp = [0u8; 256];
    match tokio::time::timeout(PROBE_READ_TIMEOUT, stream.read(&mut tmp)).await {
        Ok(Ok(0)) => {
            let _ = writeln!(buf, "  EOF before any bytes (ACCEPT-then-RST/FIN)");
        }
        Ok(Ok(n)) => {
            let head: String = String::from_utf8_lossy(&tmp[..n.min(120)]).into_owned();
            let head = head.replace('\r', "");
            let head = head.lines().next().unwrap_or("").to_string();
            let _ = writeln!(buf, "  read {n} bytes; first line: {head}");
        }
        Ok(Err(e)) => {
            let _ = writeln!(buf, "  read failed: {e}  (kind={:?})", e.kind());
        }
        Err(_) => {
            let _ = writeln!(
                buf,
                "  read timed out after {}ms",
                PROBE_READ_TIMEOUT.as_millis()
            );
        }
    }
}

/// Bind a `TcpListener` on an ephemeral loopback port, accept once
/// from a probe-side socket in a background thread, and report
/// success. This isolates whether *any* loopback HTTP-style traffic
/// works at all on the host, independent of Apple Container.
async fn self_test_loopback(buf: &mut String) {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(e) => {
            let _ = writeln!(buf, "could not bind 127.0.0.1:0: {e}");
            return;
        }
    };
    let addr: SocketAddr = match listener.local_addr() {
        Ok(a) => a,
        Err(e) => {
            let _ = writeln!(buf, "local_addr failed: {e}");
            return;
        }
    };
    listener.set_nonblocking(false).ok();
    let _ = writeln!(buf, "listening on {addr}");

    // Server thread: accept once, send a tiny response, close.
    let server = std::thread::spawn(move || -> std::io::Result<()> {
        listener.set_nonblocking(false)?;
        let (mut s, _) = listener.accept()?;
        s.set_read_timeout(Some(Duration::from_millis(2000))).ok();
        let mut tmp = [0u8; 256];
        use std::io::{Read, Write};
        let _ = s.read(&mut tmp);
        s.write_all(b"HTTP/1.0 200 OK\r\nContent-Length: 2\r\n\r\nOK")?;
        Ok(())
    });

    // Client side: connect, send HEAD, read.
    let mut client = match TcpStream::connect_timeout(&addr, PROBE_CONNECT_TIMEOUT) {
        Ok(c) => c,
        Err(e) => {
            let _ = writeln!(buf, "self-test connect failed: {e}");
            let _ = server.join();
            return;
        }
    };
    use std::io::{Read, Write};
    client.set_read_timeout(Some(PROBE_READ_TIMEOUT)).ok();
    if let Err(e) = client.write_all(b"HEAD / HTTP/1.0\r\n\r\n") {
        let _ = writeln!(buf, "self-test write failed: {e}");
        let _ = server.join();
        return;
    }
    let mut tmp = [0u8; 256];
    match client.read(&mut tmp) {
        Ok(0) => {
            let _ = writeln!(
                buf,
                "self-test got EOF before any bytes — host loopback itself is broken"
            );
        }
        Ok(n) => {
            let head = String::from_utf8_lossy(&tmp[..n.min(64)]).replace('\r', "");
            let _ = writeln!(
                buf,
                "self-test OK: read {n} bytes; first line: {}",
                head.lines().next().unwrap_or("")
            );
        }
        Err(e) => {
            let _ = writeln!(buf, "self-test read failed: {e}  (kind={:?})", e.kind());
        }
    }
    let _ = server.join();
}

fn now_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Local seconds-since-epoch is fine; humans reading the report
    // care more about the relative ordering than the exact tz.
    let out = StdCommand::new("date")
        .arg("-u")
        .arg("-r")
        .arg(secs.to_string())
        .arg("+%Y-%m-%dT%H:%M:%SZ")
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => format!("epoch={secs}"),
    }
}

fn chrono_like_stamp() -> String {
    let out = StdCommand::new("date").arg("+%Y%m%d-%H%M%S").output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => {
            use std::time::{SystemTime, UNIX_EPOCH};
            format!(
                "{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke-test the report builder end-to-end. Doesn't need a Tauri
    /// AppHandle — just exercises every subprocess, every probe, and
    /// the self-test loopback. If any branch panics or hangs, this
    /// will fail. Output is best-effort, so we just assert that all
    /// the section headers appear.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn build_report_runs_to_completion() {
        let report = tokio::time::timeout(std::time::Duration::from_secs(60), build_report())
            .await
            .expect("build_report timed out");

        for header in [
            "system",
            "apple container",
            "host listening sockets",
            "loopback probe",
            "direct vmnet probe",
            "control: our own loopback listener",
            "interfaces",
            "vpn / network filter / EDR processes",
            "launchd entries mentioning container",
            "# end of report",
        ] {
            assert!(
                report.contains(header),
                "report missing section {header:?}.\n----\n{report}\n----"
            );
        }

        // The self-test must succeed on a healthy dev box: we own
        // both ends of that loopback connection.
        assert!(
            report.contains("self-test OK:"),
            "self-test loopback did not succeed; host loopback may be broken.\n----\n{report}\n----"
        );

        eprintln!("--- diagnostic report (test) ---\n{report}");
    }

    #[test]
    fn parse_running_containers_handles_empty() {
        assert!(parse_running_containers("").is_empty());
        assert!(parse_running_containers("[]").is_empty());
        assert!(parse_running_containers("not json").is_empty());
    }

    #[test]
    fn parse_running_containers_extracts_fields() {
        let json = r#"[
            {
                "configuration": {
                    "id": "wiki3-site-abc",
                    "publishedPorts": [
                        { "hostPort": 8000, "containerPort": 80 },
                        { "hostPort": 9229 }
                    ]
                },
                "networks": [
                    { "ipv4Address": "192.168.64.7/24" }
                ]
            }
        ]"#;
        let v = parse_running_containers(json);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].name, "wiki3-site-abc");
        assert_eq!(v[0].ipv4.as_deref(), Some("192.168.64.7"));
        assert_eq!(v[0].host_ports, vec![8000, 9229]);
    }
}
