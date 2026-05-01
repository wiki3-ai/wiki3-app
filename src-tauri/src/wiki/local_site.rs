//! Local site preview: runs the wiki's `_output/` through
//! `jupyter lite serve` (plus an optional watch process) inside
//! Apple Container, and tracks the running containers so they can be
//! stopped on app quit.
//!
//! UX model: one button ("Open Local Site") per wiki card. Clicking
//! it (a) ensures the devcontainer image is available, (b) starts a
//! detached watch container that rebuilds `_output/` on source
//! changes, (c) starts a detached serve container that publishes
//! `_output/` on `127.0.0.1:<port>`, and (d) returns the URL once
//! the port is accepting TCP connections.
//!
//! Port allocation: start at 8000 and increment until we find a free
//! one (up to 8099). Exposed as a constant so tests can narrow the
//! range if needed in the future.

use std::collections::HashMap;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use crate::tools::{apple_container, devcontainer_image};

const PORT_START: u16 = 8000;
const PORT_END: u16 = 8099;
const SERVE_READY_TIMEOUT: Duration = Duration::from_secs(60);

/// Default in-container port on which we tell the serve command to
/// listen. Chosen to match `jupyter lite serve`'s own default, so
/// most devcontainers won't need to override it.
const CONTAINER_SERVE_PORT: u16 = 8000;

/// Read the container-side serve port from a devcontainer's
/// `forwardPorts` (first numeric entry). The devcontainer spec
/// allows entries to be either bare numbers (`8000`) or strings
/// like `"host:8000"`/`"8000:8000"`/`"8000"`; in the latter case
/// the *container* port is the right-hand side.
fn container_port_from_forward_ports(
    forward_ports: &[serde_json::Value],
    default_port: u16,
) -> u16 {
    for v in forward_ports {
        if let Some(n) = v.as_u64() {
            if (1..=u16::MAX as u64).contains(&n) {
                return n as u16;
            }
        }
        if let Some(s) = v.as_str() {
            // "host:cport" → cport; bare "cport" → cport.
            let tail = s.rsplit(':').next().unwrap_or(s);
            if let Ok(n) = tail.trim().parse::<u16>() {
                if n != 0 {
                    return n;
                }
            }
        }
    }
    default_port
}

/// Information about a single running preview.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RunningSite {
    pub wiki_id: String,
    pub serve_container: String,
    pub watch_container: Option<String>,
    pub host_port: u16,
    pub url: String,
}

/// Per-app state tracking which preview containers we started and
/// whether we started the Apple Container service ourselves.
pub struct LocalSiteManager {
    inner: Mutex<Inner>,
}

struct Inner {
    sites: HashMap<String, RunningSite>,
    /// True iff a call to [`LocalSiteManager::ensure_service`]
    /// actually transitioned the service from down → up. Cleared
    /// after we stop the service on quit.
    started_service: bool,
    /// True once we've spawned at least one container this session.
    /// Stays true until a shutdown sweep runs, so we still do an
    /// orphan check on quit even if every tracked site was cleaned
    /// up via `wiki_stop_container` in the meantime.
    touched_containers: bool,
}

impl LocalSiteManager {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                sites: HashMap::new(),
                started_service: false,
                touched_containers: false,
            }),
        }
    }

    pub fn get(&self, wiki_id: &str) -> Option<RunningSite> {
        self.inner.lock().unwrap().sites.get(wiki_id).cloned()
    }

    /// True iff we started containers and/or the Apple Container
    /// service this session, i.e. the quit hook has work to do.
    pub fn has_pending_cleanup(&self) -> bool {
        let g = self.inner.lock().unwrap();
        !g.sites.is_empty() || g.started_service || g.touched_containers
    }

    fn insert(&self, site: RunningSite) {
        self.inner
            .lock()
            .unwrap()
            .sites
            .insert(site.wiki_id.clone(), site);
    }

    fn remove(&self, wiki_id: &str) -> Option<RunningSite> {
        self.inner.lock().unwrap().sites.remove(wiki_id)
    }

    fn mark_touched(&self) {
        self.inner.lock().unwrap().touched_containers = true;
    }

    fn snapshot(&self) -> (Vec<RunningSite>, bool) {
        let g = self.inner.lock().unwrap();
        (g.sites.values().cloned().collect(), g.started_service)
    }

    fn mark_service_started(&self, started_by_us: bool) {
        let mut g = self.inner.lock().unwrap();
        // Only transition false → true; if a later call finds the
        // service already up, that doesn't retroactively change who
        // started it first.
        if started_by_us {
            g.started_service = true;
        }
    }

    fn clear_all_sites(&self) {
        let mut g = self.inner.lock().unwrap();
        g.sites.clear();
        g.touched_containers = false;
    }

    fn take_started_service(&self) -> bool {
        let mut g = self.inner.lock().unwrap();
        let v = g.started_service;
        g.started_service = false;
        v
    }
}

impl Default for LocalSiteManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Pick the lowest free TCP port in `[PORT_START, PORT_END]` that we
/// can bind on `127.0.0.1`.
fn pick_free_port() -> Result<u16, String> {
    for port in PORT_START..=PORT_END {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }
    Err(format!(
        "No free port available in {PORT_START}..={PORT_END}"
    ))
}

/// Read `customizations.wiki3.serveCommand` from a parsed
/// devcontainer config. Missing or non-string values yield `None`,
/// in which case callers fall back to the standard
/// `postStartCommand` or a built-in default.
fn extract_wiki3_command(customizations: Option<&serde_json::Value>, key: &str) -> Option<String> {
    customizations?
        .get("wiki3")?
        .get(key)?
        .as_str()
        .map(|s| s.to_string())
}

/// Compose the default serve command, used when neither
/// `customizations.wiki3.serveCommand` nor the devcontainer's
/// `postStartCommand` is set. `jupyter lite serve` builds the site
/// on its own at start, so there is no separate up-front build —
/// the watch loop only handles *subsequent* content changes.
///
/// `--ip 0.0.0.0` so the `--publish` mapping reaches the process.
/// Assumes `jupyter` is on PATH in the image (true for the default
/// wiki3 template).
fn default_serve_command(port: u16) -> String {
    format!("jupyter lite serve --port {port} --ip 0.0.0.0")
}

/// Poll `127.0.0.1:<port>` until a TCP connect succeeds or we time
/// out. Lets us block the "Open Local Site" action just long enough
/// that the window opens onto a live server, not a loading spinner.
async fn wait_for_port(port: u16) -> Result<(), String> {
    use tokio::net::TcpStream;
    let deadline = tokio::time::Instant::now() + SERVE_READY_TIMEOUT;
    loop {
        if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "Timed out waiting for serve container to listen on 127.0.0.1:{port}"
            ));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Spawn a detached container and attach a log follower so its
/// stdout/stderr stream into the UI via `wiki:log` events. Errors
/// surface the full `container run` stderr so misconfigured
/// devcontainers give a clear message.
#[allow(clippy::too_many_arguments)]
async fn run_detached(
    app: &tauri::AppHandle,
    wiki_id: &str,
    source: &str,
    container_bin: &Path,
    name: &str,
    workspace: &Path,
    workspace_name: &str,
    image: &str,
    remote_user: Option<&str>,
    publish: Option<(u16, u16)>,
    cmd_str: &str,
) -> Result<(), String> {
    use crate::wiki::log_stream;
    use tokio::process::Command;

    let mount_target = format!("/workspaces/{workspace_name}");
    let volume_spec = format!("{}:{}", workspace.display(), mount_target);

    // The caller is expected to redirect stderr to stdout at the
    // top of its script (e.g. `exec 2>&1`) so `container logs`
    // sees a single ordered stream. We don't wrap here because
    // `{ …; } 2>&1` doesn't compose with multi-line scripts.

    let mut cmd = Command::new(container_bin);
    cmd.arg("run")
        .arg("--detach")
        .arg("--rm")
        .arg("--name")
        .arg(name)
        .arg("--volume")
        .arg(&volume_spec)
        .arg("--workdir")
        .arg(&mount_target);

    if let Some((host_port, cport)) = publish {
        cmd.arg("--publish")
            .arg(format!("127.0.0.1:{host_port}:{cport}"));
    }
    if let Some(user) = remote_user {
        cmd.arg("--user").arg(user);
    }
    cmd.arg(image).arg("bash").arg("-lc").arg(cmd_str);

    log_stream::emit_info(
        app,
        Some(wiki_id),
        source,
        format!("container run --name {name} -- bash -lc {cmd_str}"),
    );

    let out = cmd
        .output()
        .await
        .map_err(|e| format!("failed to spawn `container run`: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "container run ({name}) failed (exit {:?}):\n--- stderr ---\n{}\n--- stdout ---\n{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout),
        ));
    }

    // Kick off a detached follower so the UI sees the container's
    // output until the container exits.
    log_stream::spawn_log_follower(
        app,
        container_bin.to_path_buf(),
        name.to_string(),
        wiki_id.to_string(),
        source.to_string(),
    );

    Ok(())
}

/// Start (or re-use) the local site preview for a wiki.
pub async fn start_site(
    app: &tauri::AppHandle,
    manager: &LocalSiteManager,
    wiki_id: &str,
    workspace: &Path,
) -> Result<RunningSite, String> {
    use crate::wiki::log_stream;

    // Idempotent: if we already have one running, return it.
    if let Some(existing) = manager.get(wiki_id) {
        return Ok(existing);
    }

    log_stream::emit_info(app, Some(wiki_id), "serve", "Detecting Apple Container…");

    // 1. Apple Container must be installed.
    let ac = apple_container::detect();
    let container_bin = ac.path.clone().ok_or_else(|| {
        "Apple Container is not installed. Install it from the signed \
         installer at https://github.com/apple/container/releases \
         before running Open Local Site."
            .to_string()
    })?;

    // 2. Ensure the service is running. Record whether we started it.
    let was_running = apple_container::is_service_running(&container_bin).await;
    if !was_running {
        log_stream::emit_info(
            app,
            Some(wiki_id),
            "serve",
            "Starting container service (may take a moment on first run)…",
        );
        apple_container::ensure_service_running(&container_bin).await?;
        manager.mark_service_started(true);
    }

    // 3. Resolve / build the devcontainer image.
    log_stream::emit_info(app, Some(wiki_id), "serve", "Resolving devcontainer image…");
    let resolved = devcontainer_image::ensure_devcontainer_image(
        &container_bin,
        workspace,
        app,
        Some(wiki_id),
    )
    .await?;
    log_stream::emit_info(
        app,
        Some(wiki_id),
        "serve",
        format!("Using image: {}", resolved.image_ref),
    );

    // 4. Pick a free host port and compose the startup script.
    //
    // We follow devcontainer semantics: `postCreateCommand` runs
    // once after create (we re-run it every start because we use
    // `--rm` containers), then `postStartCommand` is the
    // long-running foreground process. There is intentionally no
    // separate up-front build — `jupyter lite serve` builds on
    // start, so building first would do the work twice.
    //
    // Running everything in one container means one `pip install`,
    // one bind-mount, one log stream, and no virtio-fs
    // cross-container visibility questions.
    let host_port = pick_free_port()?;
    let cport =
        container_port_from_forward_ports(&resolved.config.forward_ports, CONTAINER_SERVE_PORT);
    // The foreground (serve) command, in priority order:
    //   1. `customizations.wiki3.serveCommand` — wiki3-specific
    //      override that knows about port/ip.
    //   2. `postStartCommand` — standard devcontainer hook.
    //   3. Built-in default that listens on 0.0.0.0:<cport>.
    let serve_cmd = extract_wiki3_command(resolved.config.customizations.as_ref(), "serveCommand")
        .or_else(|| {
            resolved
                .config
                .post_start_command
                .as_ref()
                .and_then(crate::wiki::git_commands::value_to_shell_command)
        })
        .unwrap_or_else(|| default_serve_command(cport));

    // The serve container runs `--rm`, so any `postCreateCommand`
    // side-effects (e.g. `pip install jupyterlite-core`) don't
    // persist. Re-run it up-front so `jupyter` & co. are on PATH.
    // Opt out by setting `customizations.wiki3.runPostCreate =
    // false` (useful if you move the install into the Dockerfile).
    let run_post_create = resolved
        .config
        .customizations
        .as_ref()
        .and_then(|c| c.get("wiki3"))
        .and_then(|w| w.get("runPostCreate"))
        .and_then(|b| b.as_bool())
        .unwrap_or(true);
    let post_create = if run_post_create {
        resolved
            .config
            .post_create_command
            .as_ref()
            .and_then(crate::wiki::git_commands::value_to_shell_command)
    } else {
        None
    };

    let mut script = String::new();
    script.push_str("exec 2>&1\n");
    script.push_str("set -e\n");
    if let Some(pc) = post_create.as_deref() {
        if !pc.trim().is_empty() {
            script.push_str(pc);
            script.push('\n');
        }
    }
    // `exec` so the serve process becomes PID 1 of the container:
    // signals forwarded cleanly, no extra bash wrapper in `ps`.
    script.push_str("exec ");
    script.push_str(&serve_cmd);
    script.push('\n');
    let combined_cmd = script;

    let tag = devcontainer_image::sanitize_tag(&resolved.workspace_name);
    let serve_name = format!("wiki3-site-{tag}");

    // Clean up any stale container (including old split
    // serve/watch names from earlier versions) from a previous
    // crashed run.
    let _ = apple_container::stop_container(&container_bin, &serve_name).await;
    let _ = apple_container::stop_container(&container_bin, &format!("wiki3-serve-{tag}")).await;
    let _ = apple_container::stop_container(&container_bin, &format!("wiki3-watch-{tag}")).await;

    // 5. Start the single serve+watch container.
    run_detached(
        app,
        wiki_id,
        "serve",
        &container_bin,
        &serve_name,
        workspace,
        &resolved.workspace_name,
        &resolved.image_ref,
        resolved.config.remote_user.as_deref(),
        Some((host_port, cport)),
        &combined_cmd,
    )
    .await?;
    // Remember that we launched a container this session so the
    // quit hook runs its sweep even if the caller never reaches
    // `manager.insert` (e.g. port-wait timeout).
    manager.mark_touched();

    // 6. Wait for the serve port.
    log_stream::emit_info(
        app,
        Some(wiki_id),
        "serve",
        format!("Waiting for 127.0.0.1:{host_port} to accept connections…"),
    );
    if let Err(e) = wait_for_port(host_port).await {
        log_stream::emit_error(
            app,
            Some(wiki_id),
            "serve",
            "Serve container did not start listening — see earlier lines.",
        );
        let _ = apple_container::stop_container(&container_bin, &serve_name).await;
        return Err(e);
    }

    let host = crate::wiki::loopback_host::loopback_hostname();
    let site = RunningSite {
        wiki_id: wiki_id.to_string(),
        serve_container: serve_name,
        watch_container: None,
        host_port,
        url: format!("http://{host}:{host_port}/"),
    };
    log_stream::emit_info(app, Some(wiki_id), "serve", format!("Ready: {}", site.url));
    manager.insert(site.clone());
    Ok(site)
}

/// Stop the preview for a single wiki (best-effort). Does nothing
/// when the wiki has no running preview.
pub async fn stop_site(manager: &LocalSiteManager, wiki_id: &str) -> Result<(), String> {
    let Some(site) = manager.remove(wiki_id) else {
        return Ok(());
    };
    let ac = apple_container::detect();
    let Some(bin) = ac.path else {
        return Ok(());
    };
    let _ = apple_container::stop_container(&bin, &site.serve_container).await;
    if let Some(w) = site.watch_container.as_deref() {
        let _ = apple_container::stop_container(&bin, w).await;
    }
    Ok(())
}

/// Outcome of the quit-time cleanup pass, so the UI layer can decide
/// whether to prompt the user about foreign containers.
#[derive(Debug, Clone, serde::Serialize, Default)]
pub struct ShutdownReport {
    /// Names of containers we stopped successfully.
    pub stopped_containers: Vec<String>,
    /// Errors (best-effort surface).
    pub errors: Vec<String>,
    /// True iff we started the Apple Container service and were
    /// able to stop it cleanly (or decided not to because of
    /// foreign containers).
    pub service_started_by_us: bool,
    /// If non-empty AND `service_started_by_us` is true, the UI
    /// should ask the user whether to also stop the service despite
    /// these foreign containers. Stopping the service will stop them
    /// too.
    pub foreign_containers: Vec<String>,
    /// True iff we actually ran `container system stop`.
    pub service_stopped: bool,
}

/// Stop every container we started. If we also started the Apple
/// Container service, either stop it (when nothing else is running)
/// or surface the list of foreign containers so the UI can prompt.
///
/// This is idempotent: calling it twice is safe, but the second call
/// will report no work.
pub async fn shutdown_all(manager: &LocalSiteManager) -> ShutdownReport {
    let mut report = ShutdownReport::default();
    let (sites, started_service) = manager.snapshot();
    report.service_started_by_us = started_service;

    let ac = apple_container::detect();
    let Some(bin) = ac.path else {
        // Apple Container somehow vanished between start and quit.
        // Nothing we can do; clear our bookkeeping so we don't try
        // again next call.
        manager.clear_all_sites();
        let _ = manager.take_started_service();
        return report;
    };

    // Stop our containers first.
    let our_names: Vec<String> = sites
        .iter()
        .flat_map(|s| std::iter::once(s.serve_container.clone()).chain(s.watch_container.clone()))
        .collect();

    for name in &our_names {
        match apple_container::stop_container(&bin, name).await {
            Ok(()) => report.stopped_containers.push(name.clone()),
            Err(e) => report.errors.push(format!("stop {name}: {e}")),
        }
    }
    manager.clear_all_sites();

    // Sweep any orphan `wiki3-*` containers — e.g. from a previous
    // crashed run where `run_detached` started the container but
    // `wait_for_port` failed before `manager.insert`, so the
    // manager never knew about it.
    let running_now = apple_container::list_running_container_names(&bin).await;
    let orphans: Vec<String> = running_now
        .iter()
        .filter(|n| {
            (n.starts_with("wiki3-site-")
                || n.starts_with("wiki3-serve-")
                || n.starts_with("wiki3-watch-"))
                && !our_names.contains(n)
                && !report.stopped_containers.contains(n)
        })
        .cloned()
        .collect();
    for name in &orphans {
        match apple_container::stop_container(&bin, name).await {
            Ok(()) => report.stopped_containers.push(name.clone()),
            Err(e) => report.errors.push(format!("stop orphan {name}: {e}")),
        }
    }

    if !started_service {
        return report;
    }

    // Check for foreign containers before (conditionally) stopping
    // the service. `buildkit` is Apple Container's own builder
    // helper — it's started automatically by the service and goes
    // away when the service stops, so it doesn't count as "foreign
    // user workload".
    let running_now = apple_container::list_running_container_names(&bin).await;
    let foreign: Vec<String> = running_now
        .into_iter()
        .filter(|n| {
            !our_names.contains(n)
                && !orphans.contains(n)
                && n != "buildkit"
                && !n.starts_with("buildkit-")
        })
        .collect();

    if foreign.is_empty() {
        match apple_container::stop_service(&bin).await {
            Ok(()) => {
                report.service_stopped = true;
                let _ = manager.take_started_service();
            }
            Err(e) => report.errors.push(format!("stop service: {e}")),
        }
    } else {
        report.foreign_containers = foreign;
        // Leave `started_service` set so a follow-up call (e.g.
        // after the user confirms the popup) can still stop it.
    }

    report
}

/// Unconditionally stop the service, used after the user confirms
/// the "foreign containers" prompt with "Stop anyway".
pub async fn force_stop_service(manager: &LocalSiteManager) -> Result<(), String> {
    let ac = apple_container::detect();
    let bin = ac
        .path
        .ok_or_else(|| "Apple Container not installed".to_string())?;
    apple_container::stop_service(&bin).await?;
    let _ = manager.take_started_service();
    Ok(())
}

// ── Tauri commands ──────────────────────────────────────────────────────

/// Start (or re-attach to) the per-wiki preview container. Idempotent:
/// returns the existing `RunningSite` if one is already tracked.
#[tauri::command]
pub async fn wiki_start_container(
    app: tauri::AppHandle,
    manager: tauri::State<'_, LocalSiteManager>,
    wiki_id: String,
) -> Result<RunningSite, String> {
    use tauri::Manager;
    let wiki = app
        .state::<crate::wiki::commands::WikiState>()
        .manager
        .get(&wiki_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Wiki not found: {wiki_id}"))?;
    let local = wiki
        .local_path
        .as_ref()
        .ok_or_else(|| "Wiki has no local path".to_string())?;
    let local_path = PathBuf::from(local);
    if !local_path.exists() {
        return Err(format!(
            "Local path does not exist: {}",
            local_path.display()
        ));
    }
    start_site(&app, manager.inner(), &wiki_id, &local_path).await
}

/// Stop the per-wiki preview container (best-effort, idempotent).
#[tauri::command]
pub async fn wiki_stop_container(
    manager: tauri::State<'_, LocalSiteManager>,
    wiki_id: String,
) -> Result<(), String> {
    stop_site(manager.inner(), &wiki_id).await
}

/// Current status of the per-wiki preview container, or `None` if
/// nothing is running.
#[tauri::command]
pub async fn wiki_container_status(
    manager: tauri::State<'_, LocalSiteManager>,
    wiki_id: String,
) -> Result<Option<RunningSite>, String> {
    Ok(manager.get(&wiki_id))
}

/// Called by the frontend after the user confirms the "foreign
/// containers" modal with "Stop anyway".
#[tauri::command]
pub async fn wiki_force_stop_container_service(
    manager: tauri::State<'_, LocalSiteManager>,
) -> Result<(), String> {
    force_stop_service(manager.inner()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_picker_returns_bindable_port() {
        let p = pick_free_port().unwrap();
        assert!((PORT_START..=PORT_END).contains(&p));
    }

    #[test]
    fn extract_wiki3_command_reads_nested_string() {
        let v = serde_json::json!({
            "wiki3": { "serveCommand": "jlpm dev:serve --port 8000" }
        });
        assert_eq!(
            extract_wiki3_command(Some(&v), "serveCommand").as_deref(),
            Some("jlpm dev:serve --port 8000"),
        );
        assert_eq!(extract_wiki3_command(Some(&v), "watchCommand"), None);
    }

    #[test]
    fn extract_wiki3_command_none_when_missing() {
        assert_eq!(extract_wiki3_command(None, "serveCommand"), None);
        let v = serde_json::json!({ "vscode": { "extensions": [] } });
        assert_eq!(extract_wiki3_command(Some(&v), "serveCommand"), None);
    }

    #[test]
    fn default_serve_command_includes_port() {
        assert!(default_serve_command(8042).contains("--port 8042"));
    }

    #[test]
    fn container_port_prefers_first_forward_port_number() {
        let v = vec![serde_json::json!(8888), serde_json::json!(9000)];
        assert_eq!(container_port_from_forward_ports(&v, 8000), 8888);
    }

    #[test]
    fn container_port_parses_host_colon_container_string() {
        let v = vec![serde_json::json!("3000:8888")];
        assert_eq!(container_port_from_forward_ports(&v, 8000), 8888);
    }

    #[test]
    fn container_port_parses_bare_string() {
        let v = vec![serde_json::json!("8888")];
        assert_eq!(container_port_from_forward_ports(&v, 8000), 8888);
    }

    #[test]
    fn container_port_falls_back_to_default_when_empty() {
        assert_eq!(container_port_from_forward_ports(&[], 8000), 8000);
    }

    #[test]
    fn container_port_falls_back_when_unparseable() {
        let v = vec![serde_json::json!("notaport"), serde_json::json!(0)];
        assert_eq!(container_port_from_forward_ports(&v, 8000), 8000);
    }

    #[test]
    fn manager_tracks_and_removes_sites() {
        let m = LocalSiteManager::new();
        let site = RunningSite {
            wiki_id: "w1".into(),
            serve_container: "wiki3-serve-w1".into(),
            watch_container: Some("wiki3-watch-w1".into()),
            host_port: 8000,
            url: "http://127.0.0.1:8000/".into(),
        };
        m.insert(site.clone());
        assert_eq!(m.get("w1").unwrap().host_port, 8000);
        let taken = m.remove("w1").unwrap();
        assert_eq!(taken.serve_container, "wiki3-serve-w1");
        assert!(m.get("w1").is_none());
    }

    #[test]
    fn manager_tracks_service_ownership() {
        let m = LocalSiteManager::new();
        assert!(!m.take_started_service());
        m.mark_service_started(true);
        assert!(m.take_started_service());
        // take_ resets it so next call is false.
        assert!(!m.take_started_service());
    }
}
