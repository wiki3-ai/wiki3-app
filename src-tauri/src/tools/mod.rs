//! Managed CLI tooling for Wiki3.
//!
//! Wiki3 users don't use Homebrew, npm, or a terminal, so every CLI the
//! app depends on is downloaded, verified, and managed by Wiki3 itself
//! under the Tauri app-data directory. This module is the foundation
//! for that: a pinned-version registry, a hash-verified installer, an
//! updater that compares installed vs. pinned, and an uninstaller.
//!
//! Layout on disk:
//! ```text
//! <app_data>/tools/
//!   deno/<version>/deno
//!   devcontainer-cli/<version>/devcontainer.js
//! ```
//!
//! Everything stays inside `<app_data>/tools/`. System `PATH` is never
//! modified.
//!
//! ## Supply-chain boundary
//!
//! The registry ships **pinned versions** with **pinned SHA-256 hashes**
//! of the release artifacts. `installer::ensure` hard-fails on hash
//! mismatch — this is the supply-chain boundary. Bumping a pinned
//! version is a code-review-gated change.

pub mod apple_container;
pub mod commands;
pub mod installer;
pub mod registry;
pub mod runner;
pub mod uninstall;
pub mod updater;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Well-known subdirectory under the Tauri app-data dir that holds all
/// managed tool installations.
pub const TOOLS_DIR_NAME: &str = "tools";

/// Resolve the managed-tools directory for a given app-data root.
pub fn tools_dir(app_data: &Path) -> PathBuf {
    app_data.join(TOOLS_DIR_NAME)
}

/// Subdir under `<tools>/deno/<ver>/` used as Deno's `--node-modules-dir`
/// so npm packages (the devcontainer CLI) are cached per-pin.
pub const NODE_MODULES_DIRNAME: &str = "node_modules";

/// Tauri-managed state for the tools subsystem. Holds the app-data
/// root so commands can resolve `<app_data>/tools/` without touching
/// the `AppHandle` path API on every call.
pub struct ToolsState {
    pub app_data: PathBuf,
    /// Memoized path of a successfully-probed Apple Container binary.
    /// Populated by [`commands::detect_apple_container`].
    pub apple_container_path: Mutex<Option<PathBuf>>,
}

impl ToolsState {
    pub fn new(app_data: PathBuf) -> Self {
        Self {
            app_data,
            apple_container_path: Mutex::new(None),
        }
    }

    pub fn tools_dir(&self) -> PathBuf {
        tools_dir(&self.app_data)
    }
}

/// Errors that can arise anywhere in the managed-tools subsystem.
#[derive(Debug, thiserror::Error)]
pub enum ToolsError {
    #[error("tool {name:?} is not in the registry")]
    UnknownTool { name: String },

    #[error("architecture {arch:?} is not supported for tool {name:?}")]
    UnsupportedArch { name: String, arch: String },

    #[error("network error while downloading {url}: {source}")]
    Download {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    #[error(
        "hash mismatch for {name} {version} (arch {arch}): expected {expected}, got {actual}"
    )]
    HashMismatch {
        name: String,
        version: String,
        arch: String,
        expected: String,
        actual: String,
    },

    #[error("archive extraction failed: {0}")]
    Extract(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result type for the managed-tools subsystem.
pub type Result<T> = std::result::Result<T, ToolsError>;
