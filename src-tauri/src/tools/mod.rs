//! Managed CLI tooling for Wiki3.
//!
//! **Deno ships inside the Wiki3 app bundle.** There is no
//! user-facing install step; the user never sees "install Deno". The
//! Deno binary is fetched at **build time** by `build.rs` (from the
//! pinned URL + SHA-256 in [`registry`]) and shipped under the macOS
//! `.app`'s Resources directory. At runtime the `tools` subsystem
//! merely locates that bundled binary.
//!
//! Disposable caches (Deno's module cache, the npm package cache for
//! `@devcontainers/cli`) live under `<app_data>/tools/cache/` and can
//! be cleared by the user from the Tools dialog without affecting the
//! bundled binary itself.
//!
//! Apple Container is a separate OS-level install (`.pkg`) and is
//! only *detected* here, never managed.

pub mod apple_container;
pub mod commands;
pub mod registry;
pub mod runner;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Subdirectory under the Tauri app-data dir holding disposable
/// per-user caches for the bundled tools. Clearing this directory is
/// always safe; it will be repopulated on next build.
pub const CACHE_DIR_NAME: &str = "tools-cache";

/// Per-Deno subdir for `DENO_DIR` (Deno's own module cache, including
/// its `npm/` subtree where the `@devcontainers/cli` package lands).
pub const DENO_DIR_NAME: &str = "deno_dir";

/// Tauri-managed state for the tools subsystem. Holds the two roots
/// commands need to resolve: the app-data dir (for caches) and the
/// bundled-resources dir (for the Deno binary).
pub struct ToolsState {
    /// Writable per-user data root (`<app_data>/`).
    pub app_data: PathBuf,
    /// Read-only bundled resources dir inside the installed app (on
    /// macOS, `<Wiki3.app>/Contents/Resources/`). This is where
    /// `build.rs` staged the Deno binary as `deno-<target-triple>`.
    pub resource_dir: PathBuf,
    /// Memoized path of a successfully-probed Apple Container binary.
    pub apple_container_path: Mutex<Option<PathBuf>>,
}

impl ToolsState {
    pub fn new(app_data: PathBuf, resource_dir: PathBuf) -> Self {
        Self {
            app_data,
            resource_dir,
            apple_container_path: Mutex::new(None),
        }
    }

    /// Root of the disposable cache tree. Safe to `rm -rf`.
    pub fn cache_dir(&self) -> PathBuf {
        self.app_data.join(CACHE_DIR_NAME)
    }
}

/// Resolve the path of the bundled Deno binary for the current host
/// arch, given a Tauri `Resource` base directory. Does not check that
/// the file exists — call [`bundled_deno_exists`] for that.
pub fn bundled_deno_path(resource_dir: &Path) -> Option<PathBuf> {
    let triple = registry::current_arch_triple()?;
    Some(resource_dir.join(format!("deno-{triple}")))
}

/// Whether the bundled Deno binary is present on disk. Returns false
/// on non-macOS hosts (where the app is not shipped).
pub fn bundled_deno_exists(resource_dir: &Path) -> bool {
    bundled_deno_path(resource_dir)
        .map(|p| p.is_file())
        .unwrap_or(false)
}

/// Errors surfaced by the tools subsystem at runtime. Installer-level
/// errors (download/hash-mismatch) live in `build.rs` now — if those
/// ever happen, the build fails and no app ships.
#[derive(Debug, thiserror::Error)]
pub enum ToolsError {
    #[error("bundled Deno not found at {path:?}; this app build is incomplete")]
    BundledDenoMissing { path: PathBuf },

    #[error("architecture {arch:?} is not a supported Wiki3 target")]
    UnsupportedArch { arch: String },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, ToolsError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_deno_path_uses_arch_triple() {
        let dir = Path::new("/Applications/Wiki3.app/Contents/Resources");
        // On non-macOS hosts (CI sandbox) current_arch_triple() is None,
        // so the resolver returns None as well. That's expected — the
        // app is only shipped for macOS.
        match bundled_deno_path(dir) {
            Some(p) => {
                let name = p.file_name().unwrap().to_string_lossy().into_owned();
                assert!(name.starts_with("deno-"), "got {name}");
                assert!(name.contains("apple-darwin"), "got {name}");
            }
            None => {
                assert!(registry::current_arch_triple().is_none());
            }
        }
    }

    #[test]
    fn bundled_deno_exists_is_false_for_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!bundled_deno_exists(tmp.path()));
    }
}
