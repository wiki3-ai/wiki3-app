//! Tauri command surface for the managed-tools subsystem.
//!
//! The user does not install Deno. Deno is bundled inside the app by
//! `build.rs`. These commands exist only to:
//!
//! * report bundled-tool status to the UI (`tools_status`),
//! * detect Apple Container (`detect_apple_container`),
//! * clear the disposable Deno / npm caches (`tools_clear_cache`).

use std::path::PathBuf;

use serde::Serialize;
use tauri::{command, State};

use super::apple_container::{self, AppleContainerStatus};
use super::registry::{self, ToolId};
use super::{bundled_deno_path, ToolsState};

/// Status entry returned by [`tools_status`].
#[derive(Debug, Clone, Serialize)]
pub struct ToolStatusEntry {
    pub name: String,
    pub version: String,
    /// Always true for tools that ship with the app; kept as an
    /// explicit field so the UI can display "Bundled" without
    /// guessing.
    pub bundled: bool,
    /// Absolute path of the bundled binary, or `None` if this build
    /// is broken (build.rs did not stage the file). The UI should
    /// treat `None` as a hard error worth surfacing to the user.
    pub path: Option<String>,
}

/// Report the version + resolved path of every tool shipped with the
/// app. Currently: just Deno.
#[command]
pub fn tools_status(state: State<'_, ToolsState>) -> Result<Vec<ToolStatusEntry>, String> {
    let deno = registry::manifest_for(ToolId::Deno);
    let path = bundled_deno_path(&state.resource_dir).and_then(|p| {
        if p.is_file() {
            Some(p.to_string_lossy().into_owned())
        } else {
            None
        }
    });
    Ok(vec![ToolStatusEntry {
        name: deno.name().to_string(),
        version: deno.version.clone(),
        bundled: true,
        path,
    }])
}

/// Result of an Apple Container probe, suitable for the UI.
#[derive(Debug, Clone, Serialize)]
pub struct AppleContainerPayload {
    pub installed: bool,
    pub path: Option<String>,
}

impl From<AppleContainerStatus> for AppleContainerPayload {
    fn from(s: AppleContainerStatus) -> Self {
        AppleContainerPayload {
            installed: s.installed,
            path: s.path.map(|p| p.to_string_lossy().into_owned()),
        }
    }
}

/// Probe for Apple Container. Memoizes the resolved path on
/// [`ToolsState`] so later commands can reuse it.
#[command]
pub fn detect_apple_container(
    state: State<'_, ToolsState>,
) -> Result<AppleContainerPayload, String> {
    let status = apple_container::detect();
    if let Ok(mut slot) = state.apple_container_path.lock() {
        *slot = status.path.clone();
    }
    Ok(status.into())
}

/// Size report for [`tools_cache_info`].
#[derive(Debug, Clone, Serialize)]
pub struct CacheInfoPayload {
    pub path: String,
    pub exists: bool,
    pub size_bytes: u64,
}

/// Report the location and size on disk of the disposable tools
/// cache, so the UI can show "Clear cache (12.4 MB)".
#[command]
pub fn tools_cache_info(state: State<'_, ToolsState>) -> Result<CacheInfoPayload, String> {
    let dir = state.cache_dir();
    let exists = dir.exists();
    let size_bytes = if exists { dir_size(&dir) } else { 0 };
    Ok(CacheInfoPayload {
        path: dir.to_string_lossy().into_owned(),
        exists,
        size_bytes,
    })
}

/// Delete every file under the disposable tools cache. Idempotent;
/// never touches the bundled Deno binary (which lives in
/// `<Resources>/`, not `<app_data>/`).
#[command]
pub fn tools_clear_cache(state: State<'_, ToolsState>) -> Result<(), String> {
    let dir = state.cache_dir();
    if !dir.exists() {
        return Ok(());
    }
    std::fs::remove_dir_all(&dir).map_err(|e| format!("clear cache: {e}"))
}

fn dir_size(path: &std::path::Path) -> u64 {
    fn walk(path: &std::path::Path, total: &mut u64) {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                walk(&entry.path(), total);
            } else if ft.is_file() {
                if let Ok(meta) = entry.metadata() {
                    *total += meta.len();
                }
            }
        }
    }
    let mut total = 0;
    walk(path, &mut total);
    total
}

/// Resolve the path of the bundled Deno binary without hitting the
/// filesystem; returns `None` if this build is missing the resource.
/// Primarily used by other backend code (e.g. `wiki_build_site`);
/// exposed as a command mainly for test harnesses.
#[command]
pub fn tools_bundled_deno_path(
    state: State<'_, ToolsState>,
) -> Result<Option<String>, String> {
    let Some(path) = bundled_deno_path(&state.resource_dir) else {
        return Ok(None);
    };
    if path.is_file() {
        Ok(Some(path.to_string_lossy().into_owned()))
    } else {
        Ok(None)
    }
}

// Re-export the type alias for tests without pulling in the full
// ToolsState setup.
#[allow(dead_code)]
pub(crate) type BundledDenoPath = PathBuf;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_entry_serializes_expected_shape() {
        let e = ToolStatusEntry {
            name: "deno".to_string(),
            version: "2.4.5".to_string(),
            bundled: true,
            path: Some("/x/deno".to_string()),
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["name"], "deno");
        assert_eq!(v["bundled"], true);
        assert_eq!(v["path"], "/x/deno");
    }

    #[test]
    fn dir_size_sums_files() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a"), b"12345").unwrap();
        let sub = tmp.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("b"), b"1234567890").unwrap();
        assert_eq!(dir_size(tmp.path()), 15);
    }

    #[test]
    fn dir_size_missing_is_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("nope");
        assert_eq!(dir_size(&missing), 0);
    }
}
