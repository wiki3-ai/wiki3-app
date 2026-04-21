//! Tools subsystem: Apple Container detection + in-process
//! parsing/normalisation of `.devcontainer/**/*.json`.
//!
//! Wiki3 drives Apple Container directly via its `container` CLI. We
//! do NOT ship Deno, Node, npm, or `@devcontainers/cli`; the
//! devcontainer configuration format is read and normalised
//! in-process via an embedded QuickJS module (see
//! [`devcontainer_config`]).
//!
//! Apple Container itself is a separate OS-level install (`.pkg`)
//! and is only *detected* here, never managed.

pub mod apple_container;
pub mod commands;
pub mod devcontainer_config;
pub mod devcontainer_image;

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
