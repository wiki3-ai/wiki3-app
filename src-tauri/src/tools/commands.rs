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

/// Probe for `git`. Wiki3 shells out to git for clone/status/commit;
/// on a fresh Mac without Xcode Command Line Tools the spawn fails
/// mid-operation with a confusing `No such file or directory`. The
/// frontend calls this once at startup so we can show a one-time
/// friendly nudge to install the CLT.
#[command]
pub fn detect_git() -> super::git_probe::GitStatus {
    super::git_probe::detect()
}
