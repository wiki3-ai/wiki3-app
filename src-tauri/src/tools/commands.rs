//! Tauri command surface for the managed-tools subsystem.
//!
//! These commands are the API the dashboard / first-run UI uses to
//! drive the managed-tools flow. They intentionally do not do any
//! policy (which tools must be present, when to prompt, etc.) — that
//! lives in the frontend. Here we only translate JSON-level requests
//! into filesystem work and stream progress.

use std::path::PathBuf;

use serde::Serialize;
use tauri::{command, AppHandle, Emitter, Manager, State};

use super::apple_container::{self, AppleContainerStatus};
use super::installer::{self, InstallProgress};
use super::registry::{self, ToolId, ToolManifest};
use super::uninstall;
use super::updater::{self, ToolStatus};
use super::ToolsState;
use crate::window_state::WindowStateManager;

/// Tauri event channel names. Two channels keep payloads small and
/// frontends simple: one for streamed install progress, one for the
/// terminal states.
const EVT_INSTALL_PROGRESS: &str = "wiki3://tools/install-progress";
const EVT_INSTALL_DONE: &str = "wiki3://tools/install-done";

/// Public status for a single tool, suitable for the UI.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolStatusPayload {
    NotInstalled,
    UpToDate { version: String },
    OutOfDate { installed: String, pinned: String },
}

impl From<ToolStatus> for ToolStatusPayload {
    fn from(s: ToolStatus) -> Self {
        match s {
            ToolStatus::NotInstalled => ToolStatusPayload::NotInstalled,
            ToolStatus::UpToDate { version } => ToolStatusPayload::UpToDate { version },
            ToolStatus::OutOfDate { installed, pinned } => {
                ToolStatusPayload::OutOfDate { installed, pinned }
            }
        }
    }
}

/// Report entry returned by [`tools_status`].
#[derive(Debug, Clone, Serialize)]
pub struct ToolStatusEntry {
    pub name: String,
    pub pinned_version: String,
    pub status: ToolStatusPayload,
}

/// Return every registered tool's pinned version and current status.
#[command]
pub fn tools_status(state: State<'_, ToolsState>) -> Result<Vec<ToolStatusEntry>, String> {
    let tools_dir = state.tools_dir();
    Ok(registry::all_manifests()
        .into_iter()
        .map(|m| {
            let status = updater::status_for(&tools_dir, &m);
            ToolStatusEntry {
                name: m.name().to_string(),
                pinned_version: m.version.clone(),
                status: status.into(),
            }
        })
        .collect())
}

fn manifest_by_name(name: &str) -> Result<ToolManifest, String> {
    match name {
        "deno" => Ok(registry::manifest_for(ToolId::Deno)),
        _ => Err(format!("unknown tool: {name}")),
    }
}

/// Download + verify + extract the named tool if not already installed.
/// Streams [`InstallProgress`] events to the frontend.
#[command]
pub async fn tools_ensure(
    app: AppHandle,
    name: String,
) -> Result<String, String> {
    let (tools_dir, manifest, arch) = {
        let state = app.state::<ToolsState>();
        let tools_dir = state.tools_dir();
        let manifest = manifest_by_name(&name)?;
        let arch = registry::current_arch_triple().ok_or_else(|| {
            "managed tools are only supported on macOS (aarch64/x86_64) in this build".to_string()
        })?;
        (tools_dir, manifest, arch)
    };

    std::fs::create_dir_all(&tools_dir).map_err(|e| format!("create tools dir: {e}"))?;

    let emit_app = app.clone();
    let progress = move |p: InstallProgress| {
        let _ = emit_app.emit(EVT_INSTALL_PROGRESS, serialize_progress(&p));
    };

    let exe = installer::ensure(&tools_dir, &manifest, arch, progress)
        .await
        .map_err(|e| e.to_string())?;

    // Record the installed version in settings so the UI can show it
    // without re-scanning the filesystem.
    if let Some(ws) = app.try_state::<WindowStateManager>() {
        if let Ok(mut settings) = ws.settings.lock() {
            settings
                .managed_tools
                .installed
                .insert(manifest.name().to_string(), manifest.version.clone());
        }
        ws.persist();
    }

    let exe_str = exe.to_string_lossy().to_string();
    let _ = app.emit(
        EVT_INSTALL_DONE,
        serde_json::json!({
            "name": manifest.name(),
            "version": manifest.version,
            "path": &exe_str,
        }),
    );
    Ok(exe_str)
}

fn serialize_progress(p: &InstallProgress) -> serde_json::Value {
    serialize_progress_payload(p)
}

/// Same payload shape as the `wiki3://tools/install-progress` event
/// emitted by [`tools_ensure`]. Exposed so other commands that drive
/// `installer::ensure` (e.g. the build-site flow when it has to
/// install Deno on the fly) emit identical JSON to the frontend.
pub fn serialize_progress_payload(p: &InstallProgress) -> serde_json::Value {
    match p {
        InstallProgress::Starting { name, version } => serde_json::json!({
            "phase": "starting", "name": name, "version": version,
        }),
        InstallProgress::CacheHit { name, version } => serde_json::json!({
            "phase": "cache_hit", "name": name, "version": version,
        }),
        InstallProgress::Downloading { name, downloaded, total } => serde_json::json!({
            "phase": "downloading", "name": name,
            "downloaded": downloaded, "total": total,
        }),
        InstallProgress::Verifying { name } => serde_json::json!({
            "phase": "verifying", "name": name,
        }),
        InstallProgress::Extracting { name } => serde_json::json!({
            "phase": "extracting", "name": name,
        }),
        InstallProgress::Done { name, version } => serde_json::json!({
            "phase": "done", "name": name, "version": version,
        }),
    }
}

/// Remove one tool's entire install tree. Idempotent.
#[command]
pub fn tools_uninstall(state: State<'_, ToolsState>, name: String) -> Result<(), String> {
    // Validate the name so callers can't nuke arbitrary subdirs.
    manifest_by_name(&name)?;
    uninstall::uninstall_tool(&state.tools_dir(), &name).map_err(|e| e.to_string())
}

/// Remove every managed tool. Idempotent. The app-data root itself
/// and unrelated sibling files are left untouched.
#[command]
pub fn tools_uninstall_all(
    app: AppHandle,
    state: State<'_, ToolsState>,
) -> Result<(), String> {
    uninstall::uninstall_all(&state.app_data).map_err(|e| e.to_string())?;
    if let Some(ws) = app.try_state::<WindowStateManager>() {
        if let Ok(mut settings) = ws.settings.lock() {
            settings.managed_tools.installed.clear();
        }
        ws.persist();
    }
    Ok(())
}

/// Apple Container probe result, suitable for the UI.
#[derive(Debug, Clone, Serialize)]
pub struct AppleContainerPayload {
    pub installed: bool,
    pub path: Option<String>,
}

impl From<AppleContainerStatus> for AppleContainerPayload {
    fn from(s: AppleContainerStatus) -> Self {
        AppleContainerPayload {
            installed: s.installed,
            path: s.path.map(|p| p.to_string_lossy().to_string()),
        }
    }
}

/// Probe for Apple Container. Memoizes the resolved path on
/// [`ToolsState`] so later commands (runner, build, serve) can reuse
/// it without re-walking `PATH`.
#[command]
pub fn detect_apple_container(
    state: State<'_, ToolsState>,
) -> Result<AppleContainerPayload, String> {
    let status = apple_container::detect();
    if let Ok(mut slot) = state.apple_container_path.lock() {
        *slot = status.path.clone();
    }
    Ok(status.into())
}

/// Resolve the path of a previously-installed tool without doing any
/// work. Returns `None` if not installed.
#[command]
pub fn tools_resolve(state: State<'_, ToolsState>, name: String) -> Result<Option<String>, String> {
    let manifest = manifest_by_name(&name)?;
    let arch = match registry::current_arch_triple() {
        Some(a) => a,
        None => return Ok(None),
    };
    let artifact = match manifest.artifact_for(arch) {
        Some(a) => a,
        None => return Ok(None),
    };
    let exe: PathBuf = state
        .tools_dir()
        .join(manifest.name())
        .join(&manifest.version)
        .join(&artifact.exe_path);
    if exe.is_file() {
        Ok(Some(exe.to_string_lossy().to_string()))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_payload_round_trip() {
        let s: ToolStatusPayload = ToolStatus::UpToDate {
            version: "1.2.3".to_string(),
        }
        .into();
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["kind"], "up_to_date");
        assert_eq!(v["version"], "1.2.3");
    }

    #[test]
    fn out_of_date_payload_serializes() {
        let s: ToolStatusPayload = ToolStatus::OutOfDate {
            installed: "1.0.0".to_string(),
            pinned: "2.0.0".to_string(),
        }
        .into();
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["kind"], "out_of_date");
        assert_eq!(v["installed"], "1.0.0");
        assert_eq!(v["pinned"], "2.0.0");
    }

    #[test]
    fn unknown_tool_rejected() {
        assert!(manifest_by_name("not-a-real-tool").is_err());
    }

    #[test]
    fn known_tool_returns_manifest() {
        let m = manifest_by_name("deno").unwrap();
        assert_eq!(m.name(), "deno");
    }
}
