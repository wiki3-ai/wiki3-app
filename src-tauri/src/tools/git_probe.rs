//! Detect whether `git` is available.
//!
//! Wiki3 shells out to `git` for clone/status/commit/push (see
//! `src/wiki/commands.rs` and `src/git/ops.rs`). On macOS `git` ships
//! as part of the Xcode Command Line Tools and lives at
//! `/usr/bin/git`, which is on launchd's minimal `PATH`, so a
//! Finder-launched `.app` normally finds it. But on a fresh Mac
//! without CLT installed, the spawn fails with a confusing
//! `No such file or directory` partway through a git operation. We
//! probe at startup so the UI can show a one-time friendly nudge.
//!
//! We probe both bare `git` (PATH lookup) and `/usr/bin/git`
//! (launchd-PATH stable location) so the result is the same in dev
//! and in the bundled `.app`.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    pub installed: bool,
    /// Absolute path of the binary that responded to `--version`.
    pub path: Option<String>,
    /// Trimmed first line of `git --version` output, or None.
    pub version: Option<String>,
}

/// Probe candidate paths in order: PATH, then `/usr/bin/git`.
pub fn detect() -> GitStatus {
    for candidate in ["git", "/usr/bin/git"] {
        if let Ok(out) = std::process::Command::new(candidate)
            .arg("--version")
            .output()
        {
            if out.status.success() {
                let version = String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .next()
                    .map(|s| s.trim().to_string());
                let path = if candidate.starts_with('/') {
                    Some(candidate.to_string())
                } else {
                    // Best-effort: resolve via `which`. Falls back to
                    // None rather than failing the probe.
                    std::process::Command::new("/usr/bin/which")
                        .arg("git")
                        .output()
                        .ok()
                        .and_then(|o| {
                            if o.status.success() {
                                String::from_utf8_lossy(&o.stdout).trim().to_string().into()
                            } else {
                                None
                            }
                        })
                };
                return GitStatus {
                    installed: true,
                    path,
                    version,
                };
            }
        }
    }
    GitStatus {
        installed: false,
        path: None,
        version: None,
    }
}
