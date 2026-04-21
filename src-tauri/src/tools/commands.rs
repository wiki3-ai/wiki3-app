//! Tauri command surface for the tools subsystem.
//!
//! Currently this is just Apple Container detection. Wiki3 does not
//! install or manage Apple Container itself — when missing, the UI
//! directs the user to the signed `.pkg` installer.

use serde::Serialize;
use tauri::{command, State};

use super::apple_container::{self, AppleContainerStatus};
use super::ToolsState;

/// Result of an Apple Container probe, suitable for the UI.
#[derive(Debug, Clone, Serialize)]
pub struct AppleContainerPayload {
    pub installed: bool,
    pub path: Option<String>,
}

impl From<AppleContainerStatus> for AppleContainerPayload {
    fn from(s: AppleContainerStatus) -> Self {
        AppleContainerPayload {
            installed: s.installed,
            path: s.path.map(|p| p.to_string_lossy().into_owned()),
        }
    }
}

/// Probe for Apple Container. Memoizes the resolved path on
/// [`ToolsState`] so later commands can reuse it.
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
