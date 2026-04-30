//! Per-wiki container lifecycle controls (start / stop / restart /
//! rebuild / remove / status).
//!
//! Thin wrappers around [`devcontainer_core::LifecycleOrchestrator`].
//! The parsed `devcontainer.json` is supplied separately by the
//! frontend engine via
//! [`crate::commands_devcontainer::submit_parsed_devcontainer`] —
//! this file does no devcontainer.json parsing. The wiki id is reused
//! directly as the orchestrator's `workspace_id`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{command, AppHandle, State};

use devcontainer_core::{LifecycleOrchestrator, LifecycleStatus, RuntimeRegistry};

use crate::tauri_sink::TauriSink;
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

/// Resolve a wiki id to its on-disk path.
fn wiki_path(wiki_state: &WikiState, wiki_id: &str) -> Result<PathBuf, String> {
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
    Ok(path)
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
    let path = wiki_path(&wiki_state, &wiki_id)?;
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
    let path = wiki_path(&wiki_state, &wiki_id)?;
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
    wiki_path(&wiki_state, &wiki_id)?;
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
    let path = wiki_path(&wiki_state, &wiki_id)?;
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
    let path = wiki_path(&wiki_state, &wiki_id)?;
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
    wiki_path(&wiki_state, &wiki_id)?;
    orchestrator
        .remove_with_sink(&TauriSink(app.clone()), &registry, &wiki_id)
        .await
        .map(ContainerStatus::from_core)
        .map_err(|e| e.to_string())
}
