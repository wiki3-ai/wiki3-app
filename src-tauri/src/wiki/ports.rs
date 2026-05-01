//! Per-wiki port monitoring.
//!
//! Reads `forwardPorts` and `portsAttributes` from the wiki's
//! `devcontainer.json` and pairs each forwarded port with a quick
//! TCP probe against `127.0.0.1:<port>` so the dashboard can show
//! which ports are actually being served.

use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{command, State};

use crate::tools::devcontainer_config::{find_devcontainer_configs, load_config};
use crate::wiki::commands::WikiState;

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
    /// Whether a TCP connect to `127.0.0.1:external` succeeded.
    pub serving: bool,
    /// Best-effort URL to open in the browser.
    pub url: String,
    /// Stable identifier for this port within the wiki, suitable for
    /// use as a window-target hint (`<repo>-<port-name>`).
    pub key: String,
}

/// Build port rows for `local_path` by reading its `devcontainer.json`.
/// On any error (missing config, parse failure, IO) returns an empty
/// vec — callers treat ports as a presentational, best-effort surface.
pub fn list_ports(local_path: &Path) -> Vec<PortRow> {
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
        let url = format!("{protocol}://localhost:{port}/");

        out.push(PortRow {
            external: port,
            internal: port,
            label,
            serving: probe_tcp(port),
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

/// Quick TCP connect to `127.0.0.1:port` with a short timeout.
/// True iff the connect succeeded.
fn probe_tcp(port: u16) -> bool {
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    TcpStream::connect_timeout(&addr, Duration::from_millis(150)).is_ok()
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
    // Run the (potentially blocking) probe in a blocking task so we
    // don't stall the async runtime if a port hangs.
    let rows = tokio::task::spawn_blocking(move || list_ports(&path))
        .await
        .map_err(|e| format!("port probe task failed: {e}"))?;
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
}
