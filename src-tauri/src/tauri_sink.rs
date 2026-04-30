//! [`TauriSink`] — wiki3-app's [`EventSink`] impl.
//!
//! Forwards orchestrator events onto the Tauri event bus as
//! `devcontainer://status` and `devcontainer://log` events.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use devcontainer_core::events::EventSink;
use devcontainer_core::LogStreamKind;

pub struct TauriSink(pub AppHandle);

impl EventSink for TauriSink {
    fn status(
        &self,
        workspace_id: &str,
        state: &str,
        container_id: Option<&str>,
        image_ref: Option<&str>,
        error: Option<&str>,
    ) {
        let _ = self.0.emit(
            "devcontainer://status",
            StatusEvent {
                workspace_id,
                state,
                container_id,
                image_ref,
                error,
            },
        );
    }

    fn log(&self, workspace_id: &str, stream: LogStreamKind, line: &str) {
        let stream_str = match stream {
            LogStreamKind::Stdout => "stdout",
            LogStreamKind::Stderr => "stderr",
            LogStreamKind::System => "system",
        };
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let _ = self.0.emit(
            "devcontainer://log",
            LogEvent {
                workspace_id,
                stream: stream_str,
                line,
                ts,
            },
        );
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusEvent<'a> {
    workspace_id: &'a str,
    state: &'a str,
    container_id: Option<&'a str>,
    image_ref: Option<&'a str>,
    error: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LogEvent<'a> {
    workspace_id: &'a str,
    stream: &'a str,
    line: &'a str,
    ts: u128,
}
