//! Line-oriented log streaming for container commands.
//!
//! We emit a single Tauri event, `wiki:log`, for every line of
//! stdout/stderr from any `container build` / `container run` /
//! `container logs -f` invocation we spawn. The frontend listens on
//! this event and renders a live log pane, so users can see the
//! real-time progress of a build or the request log of the running
//! serve container.

use std::process::Stdio;

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

/// Payload delivered with every `wiki:log` event. Kept small so the
/// frontend can render thousands of lines without choking.
#[derive(Debug, Clone, Serialize)]
pub struct LogLine {
    /// Which wiki this line belongs to. `None` for shared messages
    /// (e.g. service-start output that's not tied to a specific wiki).
    pub wiki_id: Option<String>,
    /// Short label for the producer — `"build"`, `"serve"`, etc. —
    /// used by the UI to group / colour lines.
    pub source: String,
    /// One of `"stdout" | "stderr" | "info" | "error"`.
    pub level: String,
    /// The raw line text (no trailing newline).
    pub line: String,
    /// Milliseconds since the unix epoch, for ordering.
    pub ts: u64,
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Emit a single synthetic log line (not tied to a spawned process).
/// Handy for progress markers like "Building image…" or "Waiting
/// for serve container…".
pub fn emit_info(app: &AppHandle, wiki_id: Option<&str>, source: &str, line: impl Into<String>) {
    let payload = LogLine {
        wiki_id: wiki_id.map(|s| s.to_string()),
        source: source.to_string(),
        level: "info".to_string(),
        line: line.into(),
        ts: now_ms(),
    };
    let _ = app.emit("wiki:log", payload);
}

/// Emit an error line.
pub fn emit_error(app: &AppHandle, wiki_id: Option<&str>, source: &str, line: impl Into<String>) {
    let payload = LogLine {
        wiki_id: wiki_id.map(|s| s.to_string()),
        source: source.to_string(),
        level: "error".to_string(),
        line: line.into(),
        ts: now_ms(),
    };
    let _ = app.emit("wiki:log", payload);
}

/// Spawn a process with stdout/stderr piped and stream both back as
/// `wiki:log` events. Returns `(exit_status, captured_stdout,
/// captured_stderr)` so callers can still inspect the full output
/// on failure. We capture in addition to streaming because a small
/// tail of the log is useful in error messages.
///
/// The captured strings are bounded at ~64 KiB each so a runaway
/// command can't eat all memory.
pub async fn run_and_stream(
    app: &AppHandle,
    wiki_id: Option<&str>,
    source: &str,
    mut cmd: Command,
) -> Result<(std::process::ExitStatus, String, String), String> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child: Child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn child process: {e}"))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "child stdout missing".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "child stderr missing".to_string())?;

    let app_out = app.clone();
    let wid_out = wiki_id.map(|s| s.to_string());
    let src_out = source.to_string();
    let out_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        let mut buf = String::new();
        while let Ok(Some(line)) = reader.next_line().await {
            if buf.len() < 64 * 1024 {
                buf.push_str(&line);
                buf.push('\n');
            }
            let _ = app_out.emit(
                "wiki:log",
                LogLine {
                    wiki_id: wid_out.clone(),
                    source: src_out.clone(),
                    level: "stdout".to_string(),
                    line,
                    ts: now_ms(),
                },
            );
        }
        buf
    });

    let app_err = app.clone();
    let wid_err = wiki_id.map(|s| s.to_string());
    let src_err = source.to_string();
    let err_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        let mut buf = String::new();
        while let Ok(Some(line)) = reader.next_line().await {
            if buf.len() < 64 * 1024 {
                buf.push_str(&line);
                buf.push('\n');
            }
            let _ = app_err.emit(
                "wiki:log",
                LogLine {
                    wiki_id: wid_err.clone(),
                    source: src_err.clone(),
                    level: "stderr".to_string(),
                    line,
                    ts: now_ms(),
                },
            );
        }
        buf
    });

    let status = child
        .wait()
        .await
        .map_err(|e| format!("child wait failed: {e}"))?;
    let stdout_buf = out_task.await.unwrap_or_default();
    let stderr_buf = err_task.await.unwrap_or_default();
    Ok((status, stdout_buf, stderr_buf))
}

/// Detach a follower task that streams `container logs --follow
/// <name>` until the container exits. Does not block the caller.
pub fn spawn_log_follower(
    app: &AppHandle,
    container_bin: std::path::PathBuf,
    container_name: String,
    wiki_id: String,
    source: String,
) {
    let app = app.clone();
    tokio::spawn(async move {
        let mut cmd = Command::new(&container_bin);
        cmd.arg("logs").arg("--follow").arg(&container_name);
        if let Err(e) = run_and_stream(&app, Some(&wiki_id), &source, cmd).await {
            emit_error(
                &app,
                Some(&wiki_id),
                &source,
                format!("log follower for {container_name} exited: {e}"),
            );
        }
    });
}
