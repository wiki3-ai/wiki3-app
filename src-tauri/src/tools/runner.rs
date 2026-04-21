//! Runner that invokes the **bundled** Deno on the pinned
//! `@devcontainers/cli` npm package. The invocation shape is:
//!
//! ```text
//! <Resources>/deno-<triple> run -A --node-modules-dir=<cache>/node_modules \
//!     npm:@devcontainers/cli@<pin> <subcommand> [args…]
//! ```
//!
//! There is no runtime Deno install. Deno is placed in
//! `<Wiki3.app>/Contents/Resources/` by `build.rs` and resolved via
//! Tauri's `BaseDirectory::Resource` at command-entry time.
//!
//! `DOCKER_HOST` is pointed at Apple Container's user-scoped UNIX
//! socket so the devcontainer CLI talks to Apple Container instead of
//! a non-existent Docker daemon.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::process::Command;

use super::{
    bundled_deno_path, DENO_DIR_NAME, NODE_MODULES_DIRNAME, ToolsError, ToolsState,
};

/// Pinned `@devcontainers/cli` version. Bumping this is a release
/// event; CI should re-verify compatibility with the bundled Deno.
pub const DEVCONTAINER_CLI_VERSION: &str = "0.80.0";

/// Resolve the absolute path of the bundled Deno binary, erroring if
/// it is missing (which only happens in broken app builds).
pub fn require_bundled_deno(state: &ToolsState) -> Result<PathBuf, ToolsError> {
    let path = bundled_deno_path(&state.resource_dir).ok_or_else(|| {
        ToolsError::UnsupportedArch {
            arch: std::env::consts::ARCH.to_string(),
        }
    })?;
    if !path.is_file() {
        return Err(ToolsError::BundledDenoMissing { path });
    }
    Ok(path)
}

/// Run the bundled devcontainer CLI under the bundled Deno.
///
/// Returns captured stdout/stderr and the exit status.
pub async fn run_devcontainer(
    state: &ToolsState,
    subcommand: &str,
    args: &[&str],
    workspace: &Path,
    docker_host: Option<&str>,
) -> Result<DevcontainerRunOutput, String> {
    let deno = require_bundled_deno(state).map_err(|e| e.to_string())?;

    // Both caches live under <app_data>/tools-cache/ so "Clear cache"
    // in the UI can nuke them without touching the bundled binary.
    let cache_root = state.cache_dir();
    let node_modules_dir = cache_root.join(NODE_MODULES_DIRNAME);
    let deno_dir = cache_root.join(DENO_DIR_NAME);
    std::fs::create_dir_all(&node_modules_dir)
        .map_err(|e| format!("create node_modules dir: {e}"))?;
    std::fs::create_dir_all(&deno_dir).map_err(|e| format!("create deno_dir: {e}"))?;

    let mut cmd = Command::new(&deno);
    cmd.arg("run")
        .arg("-A")
        .arg(format!("--node-modules-dir={}", node_modules_dir.display()))
        .arg(format!(
            "npm:@devcontainers/cli@{}",
            DEVCONTAINER_CLI_VERSION
        ))
        .arg(subcommand)
        .args(args)
        .current_dir(workspace)
        .env("DENO_DIR", &deno_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(host) = docker_host {
        cmd.env("DOCKER_HOST", host);
    }

    let output = cmd
        .output()
        .await
        .map_err(|e| format!("spawn deno: {e}"))?;

    Ok(DevcontainerRunOutput {
        status: output.status.code(),
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        deno_path: deno,
    })
}

/// Return value of [`run_devcontainer`].
#[derive(Debug, Clone)]
pub struct DevcontainerRunOutput {
    pub status: Option<i32>,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub deno_path: PathBuf,
}

/// Convert Apple Container's install into a `DOCKER_HOST` URL
/// pointing at its per-user UNIX socket.
pub fn apple_container_docker_host() -> Option<String> {
    let home = dirs::home_dir()?;
    let sock = home
        .join("Library")
        .join("Containers")
        .join("com.apple.container")
        .join("Data")
        .join("container.sock");
    Some(format!("unix://{}", sock.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devcontainer_cli_version_is_a_pinned_semver() {
        let parts: Vec<_> = DEVCONTAINER_CLI_VERSION.split('.').collect();
        assert_eq!(parts.len(), 3);
        for p in parts {
            assert!(p.chars().all(|c| c.is_ascii_digit()), "non-numeric: {p}");
        }
    }

    #[test]
    fn apple_container_docker_host_is_unix_url() {
        if let Some(url) = apple_container_docker_host() {
            assert!(url.starts_with("unix://"), "got {url}");
            assert!(url.contains("com.apple.container"), "got {url}");
        }
    }

    #[test]
    fn require_bundled_deno_errors_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let app_data = tmp.path().join("data");
        let resource_dir = tmp.path().join("resources");
        std::fs::create_dir_all(&app_data).unwrap();
        std::fs::create_dir_all(&resource_dir).unwrap();
        let state = ToolsState::new(app_data, resource_dir);

        let err = require_bundled_deno(&state).unwrap_err();
        // On macOS hosts we get BundledDenoMissing (arch resolves);
        // on other hosts we get UnsupportedArch (arch is None). Both
        // are acceptable "not usable" signals.
        assert!(matches!(
            err,
            ToolsError::BundledDenoMissing { .. } | ToolsError::UnsupportedArch { .. }
        ));
    }
}
