//! Updater: compare what's installed on disk against what the registry
//! currently pins, and report which tools are out of date.
//!
//! Per the plan, Wiki3 does NOT auto-fetch new upstream versions at
//! runtime. "An update is available" here means "a new Wiki3 release
//! has bumped the pinned version in [`crate::tools::registry`]." This
//! keeps the supply chain reproducible and makes every bump a
//! code-review-gated event.

use std::path::Path;

use super::registry::{self, ToolManifest};

/// Status of one tool on disk vs. the current registry pin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolStatus {
    /// Nothing is installed yet for this tool.
    NotInstalled,
    /// The installed version matches the pinned version.
    UpToDate { version: String },
    /// A different (older) version is installed; `installed` is one of
    /// the versions found under `<tools_dir>/<name>/`.
    OutOfDate {
        installed: String,
        pinned: String,
    },
}

/// Inspect `tools_dir` and return the [`ToolStatus`] for `manifest`.
///
/// Tools are considered "installed" if the pinned version directory
/// exists. Any other directory under `<tools_dir>/<name>/` is treated
/// as a stale prior install that an update would supersede.
pub fn status_for(tools_dir: &Path, manifest: &ToolManifest) -> ToolStatus {
    let tool_dir = tools_dir.join(manifest.name());
    if !tool_dir.exists() {
        return ToolStatus::NotInstalled;
    }

    let pinned = &manifest.version;
    if tool_dir.join(pinned).exists() {
        return ToolStatus::UpToDate {
            version: pinned.clone(),
        };
    }

    // Find any other version-looking subdirectory. We don't try to
    // sort semver — any non-pinned install is "out of date" by
    // definition.
    if let Ok(rd) = std::fs::read_dir(&tool_dir) {
        for ent in rd.flatten() {
            if !ent.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = ent.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') {
                // Skip staging / hidden dirs.
                continue;
            }
            return ToolStatus::OutOfDate {
                installed: name.into_owned(),
                pinned: pinned.clone(),
            };
        }
    }

    ToolStatus::NotInstalled
}

/// One entry per managed tool, covering the whole registry.
#[derive(Debug, Clone)]
pub struct ToolStatusReport {
    pub name: String,
    pub status: ToolStatus,
}

/// Produce a status report for every tool in the registry.
pub fn check_for_updates(tools_dir: &Path) -> Vec<ToolStatusReport> {
    registry::all_manifests()
        .into_iter()
        .map(|m| ToolStatusReport {
            name: m.name().to_string(),
            status: status_for(tools_dir, &m),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deno() -> ToolManifest {
        registry::deno_manifest()
    }

    #[test]
    fn not_installed_when_dir_absent() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(status_for(tmp.path(), &deno()), ToolStatus::NotInstalled);
    }

    #[test]
    fn up_to_date_when_pinned_version_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let m = deno();
        std::fs::create_dir_all(tmp.path().join("deno").join(&m.version)).unwrap();
        assert_eq!(
            status_for(tmp.path(), &m),
            ToolStatus::UpToDate {
                version: m.version.clone()
            }
        );
    }

    #[test]
    fn out_of_date_when_other_version_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let m = deno();
        std::fs::create_dir_all(tmp.path().join("deno").join("1.0.0")).unwrap();
        assert_eq!(
            status_for(tmp.path(), &m),
            ToolStatus::OutOfDate {
                installed: "1.0.0".to_string(),
                pinned: m.version.clone(),
            }
        );
    }

    #[test]
    fn staging_dir_is_not_treated_as_installed_version() {
        let tmp = tempfile::tempdir().unwrap();
        let m = deno();
        std::fs::create_dir_all(tmp.path().join("deno").join(".staging")).unwrap();
        assert_eq!(status_for(tmp.path(), &m), ToolStatus::NotInstalled);
    }

    #[test]
    fn check_for_updates_covers_every_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let report = check_for_updates(tmp.path());
        assert_eq!(report.len(), registry::all_manifests().len());
        assert!(report.iter().any(|r| r.name == "deno"));
    }
}
