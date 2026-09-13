//! Tools subsystem: Apple Container detection, and helpers to probe the
//! rest of the host toolchain.
//!
//! Apple Container is a separate OS-level install (`.pkg`) and is only
//! *detected* here, never managed.
//!
//! This module used to also host a second devcontainer parser (an embedded
//! QuickJS build of a resolver) plus the per-wiki Build / Serve / Stop preview
//! flow that depended on it. Both are gone: `devcontainer.json` is parsed once,
//! by the frontend engine bundle, and the result is handed to
//! `devcontainer-core`'s orchestrator. See `docs/devcontainer-engine.md`.

pub mod apple_container;
pub mod commands;
pub mod git_probe;

use std::path::PathBuf;
use std::sync::Mutex;

/// Tauri-managed state for the tools subsystem.
pub struct ToolsState {
    /// Memoized path of a successfully-probed Apple Container binary.
    pub apple_container_path: Mutex<Option<PathBuf>>,
}

impl ToolsState {
    pub fn new() -> Self {
        Self {
            apple_container_path: Mutex::new(None),
        }
    }
}

impl Default for ToolsState {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors that can arise in the tools subsystem.
#[derive(Debug, thiserror::Error)]
pub enum ToolsError {
    #[error("javascript error in devcontainer config processing: {0}")]
    Script(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result type for the tools subsystem.
pub type Result<T> = std::result::Result<T, ToolsError>;
