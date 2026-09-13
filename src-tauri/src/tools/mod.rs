//! Tools subsystem: Apple Container detection + the *legacy* per-wiki
//! preview container flow.
//!
//! Wiki3 drives Apple Container directly via its `container` CLI, and
//! this is where the per-wiki **Build / Serve / Stop** preview lives.
//! The devcontainer configuration format is read and normalised
//! in-process via an embedded QuickJS module (see
//! [`devcontainer_config`]).
//!
//! **This is the older of two devcontainer parsing paths.** The
//! devcontainer *lifecycle* (start / stop / restart / rebuild / remove)
//! goes through the prebuilt engine bundle in `src/public/` and the
//! reusable `devcontainer-core` crate, which is runtime-agnostic and can
//! drive Docker or Podman as well as Apple Containers.
//!
//! This subsystem is Apple-only — it has no runtime seam — and its
//! [`devcontainer_config::DevcontainerConfig`] is a hand-rolled subset
//! that silently drops fields it does not declare (`mounts`, `runArgs`).
//! Prefer migrating `Build`/`Serve`/`Stop` onto `devcontainer-core` over
//! extending anything here. See `docs/devcontainer-engine.md`.
//!
//! Apple Container itself is a separate OS-level install (`.pkg`)
//! and is only *detected* here, never managed.

pub mod apple_container;
pub mod commands;
pub mod devcontainer_config;
pub mod devcontainer_image;
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
