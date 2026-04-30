//! Per-wiki container lifecycle controls (start / stop / restart /
//! rebuild / remove / status).
//!
//! These commands are thin wrappers around
//! [`devcontainer_core::LifecycleOrchestrator`]. They are *additive*
//! and intentionally separate from `local_site` (which still owns the
//! wiki-specific preview/serve flow). The wiki id is reused directly
//! as the orchestrator's `workspace_id`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{command, AppHandle, State};

use devcontainer_core::{
    DevContainerBuild, LifecycleOrchestrator, LifecycleStatus, ParsedDevContainer, RuntimeRegistry,
};

use crate::tauri_sink::TauriSink;
use crate::tools::devcontainer_config::{self, DevcontainerConfig};
use crate::wiki::commands::WikiState;

/// Frontend-facing snapshot. Mirrors core's [`LifecycleStatus`] minus
/// internal fields, with field names in camelCase for JS consumers.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerStatus {
    pub wiki_id: String,
    pub state: String,
    pub container_id: Option<String>,
    pub image_ref: Option<String>,
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_drift: Option<bool>,
}

impl ContainerStatus {
    fn from_core(s: LifecycleStatus) -> Self {
        Self {
            wiki_id: s.workspace_id,
            state: s.state.to_string(),
            container_id: s.container_id,
            image_ref: s.image_ref,
            error: s.error,
            config_drift: s.config_drift,
        }
    }
}

/// Resolve a wiki id, parse its `devcontainer.json`, and push the
/// parsed config + workspace path into the orchestrator. Returns the
/// local path so callers can pass it to lifecycle methods that need
/// it.
fn prepare(
    wiki_state: &WikiState,
    orchestrator: &LifecycleOrchestrator,
    wiki_id: &str,
) -> Result<PathBuf, String> {
    let wiki = wiki_state
        .manager
        .get(wiki_id)
        .map_err(|e| format!("wiki lookup failed: {e}"))?
        .ok_or_else(|| format!("unknown wiki: {wiki_id}"))?;
    let path = wiki
        .local_path
        .as_ref()
        .map(PathBuf::from)
        .ok_or_else(|| format!("wiki {wiki_id} has no local path"))?;
    if !path.exists() {
        return Err(format!("local path does not exist: {}", path.display()));
    }

    let configs = devcontainer_config::find_devcontainer_configs(&path);
    let cfg_path = configs
        .first()
        .cloned()
        .ok_or_else(|| "no devcontainer.json found under .devcontainer/".to_string())?;
    let cfg = devcontainer_config::load_config(&cfg_path)
        .map_err(|e| format!("failed to parse {}: {e}", cfg_path.display()))?;

    let parsed = to_parsed(cfg, cfg_path, &path);
    orchestrator.set_parsed_config(wiki_id, parsed);
    orchestrator.record_host_workspace(wiki_id, &path);
    Ok(path)
}

/// Lossy convert wiki3-app's [`DevcontainerConfig`] to core's
/// [`ParsedDevContainer`]. Only the fields core consumes today are
/// mapped.
fn to_parsed(
    cfg: DevcontainerConfig,
    cfg_path: PathBuf,
    workspace_path: &std::path::Path,
) -> ParsedDevContainer {
    let mut parsed = ParsedDevContainer {
        name: cfg.name,
        image: cfg.image,
        config_file_path: Some(cfg_path),
        workspace_folder: Some(
            std::path::PathBuf::from("/workspaces").join(
                workspace_path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("workspace"),
            ),
        ),
        remote_user: cfg.remote_user,
        ..Default::default()
    };

    if let Some(b) = cfg.build {
        let args: std::collections::HashMap<String, String> = b
            .args
            .as_ref()
            .and_then(|v| v.as_object())
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        parsed.build = Some(DevContainerBuild {
            dockerfile: b.dockerfile,
            context: b.context,
            target: b.target,
            args,
            cache_from: vec![],
        });
    }

    parsed.forward_ports = cfg
        .forward_ports
        .iter()
        .filter_map(|v| {
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
        })
        .collect();

    parsed.post_create_command = cfg
        .post_create_command
        .as_ref()
        .and_then(value_to_lifecycle_command);
    parsed.post_start_command = cfg
        .post_start_command
        .as_ref()
        .and_then(value_to_lifecycle_command);

    parsed
}

fn value_to_lifecycle_command(
    v: &serde_json::Value,
) -> Option<devcontainer_core::LifecycleCommand> {
    if let Some(s) = v.as_str() {
        Some(devcontainer_core::LifecycleCommand::Single(s.to_string()))
    } else if let Some(arr) = v.as_array() {
        let parts: Vec<String> = arr
            .iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect();
        if parts.is_empty() {
            None
        } else {
            Some(devcontainer_core::LifecycleCommand::Multiple(parts))
        }
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[command]
pub async fn wiki_container_ctl_status(
    wiki_state: State<'_, WikiState>,
    registry: State<'_, RuntimeRegistry>,
    orchestrator: State<'_, LifecycleOrchestrator>,
    wiki_id: String,
) -> Result<ContainerStatus, String> {
    let path = prepare(&wiki_state, &orchestrator, &wiki_id)?;
    Ok(ContainerStatus::from_core(
        orchestrator
            .status_with_drift(&registry, &wiki_id, &path)
            .await,
    ))
}

#[command]
pub async fn wiki_container_ctl_up(
    app: AppHandle,
    wiki_state: State<'_, WikiState>,
    registry: State<'_, RuntimeRegistry>,
    orchestrator: State<'_, LifecycleOrchestrator>,
    wiki_id: String,
) -> Result<ContainerStatus, String> {
    let path = prepare(&wiki_state, &orchestrator, &wiki_id)?;
    orchestrator
        .up_with_sink(
            std::sync::Arc::new(TauriSink(app.clone())),
            &registry,
            &wiki_id,
            &path,
        )
        .await
        .map(ContainerStatus::from_core)
        .map_err(|e| e.to_string())
}

#[command]
pub async fn wiki_container_ctl_stop(
    app: AppHandle,
    wiki_state: State<'_, WikiState>,
    registry: State<'_, RuntimeRegistry>,
    orchestrator: State<'_, LifecycleOrchestrator>,
    wiki_id: String,
) -> Result<ContainerStatus, String> {
    prepare(&wiki_state, &orchestrator, &wiki_id)?;
    orchestrator
        .stop_with_sink(&TauriSink(app.clone()), &registry, &wiki_id)
        .await
        .map(ContainerStatus::from_core)
        .map_err(|e| e.to_string())
}

#[command]
pub async fn wiki_container_ctl_restart(
    app: AppHandle,
    wiki_state: State<'_, WikiState>,
    registry: State<'_, RuntimeRegistry>,
    orchestrator: State<'_, LifecycleOrchestrator>,
    wiki_id: String,
) -> Result<ContainerStatus, String> {
    let path = prepare(&wiki_state, &orchestrator, &wiki_id)?;
    let _ = orchestrator
        .stop_with_sink(&TauriSink(app.clone()), &registry, &wiki_id)
        .await;
    orchestrator
        .up_with_sink(
            std::sync::Arc::new(TauriSink(app.clone())),
            &registry,
            &wiki_id,
            &path,
        )
        .await
        .map(ContainerStatus::from_core)
        .map_err(|e| e.to_string())
}

#[command]
pub async fn wiki_container_ctl_rebuild(
    app: AppHandle,
    wiki_state: State<'_, WikiState>,
    registry: State<'_, RuntimeRegistry>,
    orchestrator: State<'_, LifecycleOrchestrator>,
    wiki_id: String,
) -> Result<ContainerStatus, String> {
    let path = prepare(&wiki_state, &orchestrator, &wiki_id)?;
    orchestrator
        .rebuild_with_sink(
            std::sync::Arc::new(TauriSink(app.clone())),
            &registry,
            &wiki_id,
            &path,
        )
        .await
        .map(ContainerStatus::from_core)
        .map_err(|e| e.to_string())
}

#[command]
pub async fn wiki_container_ctl_remove(
    app: AppHandle,
    wiki_state: State<'_, WikiState>,
    registry: State<'_, RuntimeRegistry>,
    orchestrator: State<'_, LifecycleOrchestrator>,
    wiki_id: String,
) -> Result<ContainerStatus, String> {
    prepare(&wiki_state, &orchestrator, &wiki_id)?;
    orchestrator
        .remove_with_sink(&TauriSink(app.clone()), &registry, &wiki_id)
        .await
        .map(ContainerStatus::from_core)
        .map_err(|e| e.to_string())
}
