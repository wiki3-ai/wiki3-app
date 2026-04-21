//! Runner that invokes the managed Deno on the managed devcontainer
//! CLI. The invocation shape is:
//!
//! ```text
//! <tools>/deno/<ver>/deno run -A --node-modules-dir=auto \
//!     npm:@devcontainers/cli@<pin> <subcommand> [args…]
//! ```
//!
//! Deno's Node-compat layer fetches `@devcontainers/cli` from npm on
//! first use and caches it per-Deno-install. We point
//! `--node-modules-dir` at a Wiki3-owned path so the cache lives
//! under `<app_data>/tools/` like everything else and is purged
//! cleanly by uninstall.
//!
//! `DOCKER_HOST` is pointed at Apple Container's UNIX socket so the
//! devcontainer CLI talks to Apple Container instead of Docker
//! Desktop. The path Apple Container publishes for its socket is
//! resolved from the detected binary's install prefix; see
//! [`apple_container_docker_host`].

use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::process::Command;

use super::installer::InstallProgress;
use super::registry::{self, ToolId};
use super::{installer, ToolsState, NODE_MODULES_DIRNAME};

/// Pinned `@devcontainers/cli` version. Bumping this is a release
/// event, same rules as the Deno pin: code review gated.
pub const DEVCONTAINER_CLI_VERSION: &str = "0.80.0";

/// Ensure Deno is installed, then run the devcontainer CLI under it.
///
/// Returns captured stdout/stderr and the exit status. Progress
/// events from the Deno install (first-run only) flow through
/// `install_progress`.
pub async fn run_devcontainer(
    state: &ToolsState,
    subcommand: &str,
    args: &[&str],
    workspace: &Path,
    docker_host: Option<&str>,
    install_progress: impl FnMut(InstallProgress),
) -> Result<DevcontainerRunOutput, String> {
    let deno = ensure_deno(state, install_progress).await?;
    let node_modules_dir = deno
        .parent()
        .map(|p| p.join(NODE_MODULES_DIRNAME))
        .ok_or_else(|| "could not resolve deno install dir".to_string())?;
    std::fs::create_dir_all(&node_modules_dir)
        .map_err(|e| format!("create node_modules dir: {e}"))?;

    let mut cmd = Command::new(&deno);
    cmd.arg("run")
        .arg("-A")
        .arg(format!(
            "--node-modules-dir={}",
            node_modules_dir.display()
        ))
        .arg(format!(
            "npm:@devcontainers/cli@{}",
            DEVCONTAINER_CLI_VERSION
        ))
        .arg(subcommand)
        .args(args)
        .current_dir(workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(host) = docker_host {
        cmd.env("DOCKER_HOST", host);
    }

    // Never inherit an ambient DENO_DIR that would leak outside our
    // managed tree; keep Deno's cache rooted next to the pinned install.
    let deno_dir = deno
        .parent()
        .map(|p| p.join("deno_dir"))
        .ok_or_else(|| "could not resolve deno_dir".to_string())?;
    std::fs::create_dir_all(&deno_dir).map_err(|e| format!("create deno_dir: {e}"))?;
    cmd.env("DENO_DIR", &deno_dir);

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
    /// For logging / telemetry; the absolute path of the Deno binary used.
    pub deno_path: PathBuf,
}

/// Ensure the pinned Deno is installed under `<tools>/deno/<ver>/`
/// and return the path to its executable.
pub async fn ensure_deno(
    state: &ToolsState,
    progress: impl FnMut(InstallProgress),
) -> Result<PathBuf, String> {
    let tools_dir = state.tools_dir();
    std::fs::create_dir_all(&tools_dir).map_err(|e| format!("create tools dir: {e}"))?;
    let manifest = registry::manifest_for(ToolId::Deno);
    let arch = registry::current_arch_triple()
        .ok_or_else(|| "managed Deno is only supported on macOS in this build".to_string())?;
    installer::ensure(&tools_dir, &manifest, arch, progress)
        .await
        .map_err(|e| e.to_string())
}

/// Convert a resolved Apple Container binary path into a `DOCKER_HOST`
/// URL pointing at its UNIX socket.
///
/// Apple Container publishes its socket at
/// `~/Library/Containers/com.apple.container/Data/container.sock`
/// (user-scoped; no root), and the CLI is also happy to accept
/// `unix://` URLs. We resolve it per-user rather than hard-coding
/// `/var/run/docker.sock`, which does not exist on Apple Container.
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
    fn devcontainer_cli_version_is_pinned() {
        assert!(!DEVCONTAINER_CLI_VERSION.trim().is_empty());
        // Must be a semver-ish string (major.minor.patch) — a simple
        // guard against a future bump that accidentally leaves a
        // placeholder.
        let parts: Vec<_> = DEVCONTAINER_CLI_VERSION.split('.').collect();
        assert_eq!(parts.len(), 3, "expected X.Y.Z, got {DEVCONTAINER_CLI_VERSION}");
        for p in parts {
            assert!(
                p.chars().all(|c| c.is_ascii_digit()),
                "non-numeric version component: {p}"
            );
        }
    }

    #[test]
    fn apple_container_docker_host_is_unix_url() {
        // On any platform where HOME is set (including CI Linux), the
        // helper should produce a well-formed unix:// URL pointing at
        // a user-scoped socket.
        if let Some(url) = apple_container_docker_host() {
            assert!(url.starts_with("unix://"), "got {url}");
            assert!(url.contains("com.apple.container"), "got {url}");
        }
    }
}
