//! Apple Container detection.
//!
//! Apple Container is distributed as a signed `.pkg` installer that
//! places a system-wide `container` binary. Wiki3 does NOT silently
//! install `.pkg`s — when missing, we `open` the signed installer and
//! let macOS show its normal prompt. This module is just the
//! detection half: probe well-known paths plus the user's `PATH` and
//! report whether and where `container` is present.
//!
//! Uninstall of Apple Container is an OS responsibility and is
//! deliberately not modeled here.

use std::path::{Path, PathBuf};

/// Standard locations to probe before falling back to `PATH`. Order
/// matters only for reporting — any hit is treated as equivalent.
const STANDARD_PATHS: &[&str] = &[
    "/usr/local/bin/container",
    "/opt/homebrew/bin/container",
];

/// Result of probing for Apple Container on this system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppleContainerStatus {
    /// Whether a `container` binary was found.
    pub installed: bool,
    /// Absolute path of the resolved binary, if any.
    pub path: Option<PathBuf>,
}

/// Probe for Apple Container using a caller-supplied directory list.
/// Exposed so tests can exercise the logic without touching `/usr/…`.
pub fn probe_with_dirs(standard_paths: &[&Path], path_env: Option<&str>) -> AppleContainerStatus {
    for p in standard_paths {
        if is_runnable(p) {
            return AppleContainerStatus {
                installed: true,
                path: Some(p.to_path_buf()),
            };
        }
    }
    if let Some(path_env) = path_env {
        for dir in split_path_env(path_env) {
            let candidate = dir.join("container");
            if is_runnable(&candidate) {
                return AppleContainerStatus {
                    installed: true,
                    path: Some(candidate),
                };
            }
        }
    }
    AppleContainerStatus {
        installed: false,
        path: None,
    }
}

/// Detect Apple Container on the current system.
pub fn detect() -> AppleContainerStatus {
    let standard: Vec<PathBuf> = STANDARD_PATHS.iter().map(PathBuf::from).collect();
    let standard_refs: Vec<&Path> = standard.iter().map(|p| p.as_path()).collect();
    let path_env = std::env::var("PATH").ok();
    probe_with_dirs(&standard_refs, path_env.as_deref())
}

/// Ensure the Apple Container system service (which owns the UNIX
/// socket at `~/Library/Containers/com.apple.container/Data/container.sock`)
/// is running. Runs `container system start` which is idempotent —
/// it's a no-op if the service is already up.
///
/// On first start Apple Container prompts on stdin to download its
/// default Kata kernel. We auto-accept by feeding `y\n`; the
/// alternative (failing with "failed to read user input") leaves the
/// service half-initialized and unusable.
///
/// On first start Apple may also show a system authorization prompt
/// (sometimes several seconds), and the kernel download itself can
/// take a while, so we impose a generous timeout.
pub async fn ensure_service_running(container_bin: &Path) -> Result<(), String> {
    use std::time::Duration;
    use tokio::io::AsyncWriteExt;
    use tokio::process::Command;

    let mut cmd = Command::new(container_bin);
    cmd.arg("system").arg("start");

    let mut child = cmd
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn `container system start`: {e}"))?;

    // Pre-answer any interactive prompts (e.g. the first-run
    // "Install the recommended default kernel? [Y/n]"). Closing stdin
    // afterwards makes non-interactive runs exit cleanly too.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(b"y\n").await;
        drop(stdin);
    }

    // First-run kernel download can take minutes on a slow link.
    let output = tokio::time::timeout(Duration::from_secs(600), child.wait_with_output())
        .await
        .map_err(|_| "`container system start` timed out after 10 minutes".to_string())?
        .map_err(|e| format!("wait for `container system start`: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "`container system start` failed (exit {:?}):\n--- stderr ---\n{}\n--- stdout ---\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout),
        ));
    }
    Ok(())
}

fn split_path_env(path_env: &str) -> Vec<PathBuf> {
    // Mirrors std::env::split_paths without requiring an OsString
    // round-trip; Unix-only because the rest of the tool stack is
    // macOS-only anyway.
    path_env
        .split(':')
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn is_runnable(p: &Path) -> bool {
    // A plain existence check is sufficient on macOS — Apple's
    // installer always sets `container` executable, and the
    // `open`-the-`.pkg` flow is what would fix a broken install. On
    // Unix we additionally require the executable bit so stray files
    // named `container` don't masquerade as installed.
    match std::fs::metadata(p) {
        Ok(md) => {
            if !md.is_file() {
                return false;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                md.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                true
            }
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[cfg(unix)]
    fn make_executable(p: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(p).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(p, perms).unwrap();
    }

    #[test]
    fn reports_not_installed_when_nothing_found() {
        let tmp = tempfile::tempdir().unwrap();
        let bogus = tmp.path().join("does-not-exist");
        let status = probe_with_dirs(&[bogus.as_path()], Some(""));
        assert_eq!(
            status,
            AppleContainerStatus {
                installed: false,
                path: None
            }
        );
    }

    #[test]
    fn finds_binary_in_standard_path() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("container");
        fs::write(&exe, b"#!/bin/sh\necho hi\n").unwrap();
        #[cfg(unix)]
        make_executable(&exe);

        let status = probe_with_dirs(&[exe.as_path()], None);
        assert!(status.installed);
        assert_eq!(status.path.as_deref(), Some(exe.as_path()));
    }

    #[test]
    fn falls_back_to_path_env() {
        let tmp = tempfile::tempdir().unwrap();
        let bin_dir = tmp.path().join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let exe = bin_dir.join("container");
        fs::write(&exe, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        make_executable(&exe);

        let path_env = format!("/nowhere:{}", bin_dir.display());
        let status = probe_with_dirs(&[], Some(&path_env));
        assert!(status.installed);
        assert_eq!(status.path.as_deref(), Some(exe.as_path()));
    }

    #[cfg(unix)]
    #[test]
    fn non_executable_file_is_not_accepted() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("container");
        fs::write(&exe, b"not actually a binary").unwrap();
        // Do not set +x.
        let status = probe_with_dirs(&[exe.as_path()], None);
        assert!(!status.installed);
    }

    #[test]
    fn directory_named_container_is_not_accepted() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("container");
        fs::create_dir(&path).unwrap();
        let status = probe_with_dirs(&[path.as_path()], None);
        assert!(!status.installed);
    }

    #[test]
    fn empty_path_env_is_safe() {
        let status = probe_with_dirs(&[], Some(""));
        assert!(!status.installed);
    }

    #[test]
    fn standard_path_wins_over_path_env() {
        let tmp = tempfile::tempdir().unwrap();
        let standard = tmp.path().join("container");
        fs::write(&standard, b"").unwrap();
        #[cfg(unix)]
        make_executable(&standard);

        let bin_dir = tmp.path().join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let path_exe = bin_dir.join("container");
        fs::write(&path_exe, b"").unwrap();
        #[cfg(unix)]
        make_executable(&path_exe);

        let path_env = bin_dir.display().to_string();
        let status = probe_with_dirs(&[standard.as_path()], Some(&path_env));
        assert_eq!(status.path.as_deref(), Some(standard.as_path()));
    }
}
