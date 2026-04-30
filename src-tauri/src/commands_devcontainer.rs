//! Bridge commands for the devcontainer engine running in the
//! WebView. Mirrors the surface that `devcontainer-cli` exposes so
//! the same `devcontainer-engine.js` bundle can run unchanged here.
//!
//! The engine reads `.devcontainer/devcontainer.json` via a
//! [`FileHost`](https://containers.dev) abstraction \u2014 these are the
//! Rust-side endpoints behind that bridge plus
//! [`submit_parsed_devcontainer`], which hands the parsed result to
//! [`devcontainer_core::LifecycleOrchestrator`].
//!
//! Filesystem access is sandboxed to paths under any registered
//! wiki's `local_path`. Writes are further restricted to the
//! `.devcontainer/` subtree, matching the upstream sandbox.

use std::path::{Path, PathBuf};

use tauri::{command, State};

use devcontainer_core::{LifecycleOrchestrator, ParsedDevContainer};

use crate::wiki::commands::WikiState;

/// Resolve `requested` to a canonical path that lives under one of
/// the wikis registered in [`WikiState`].
fn resolve_within_wikis(state: &WikiState, requested: &Path) -> Result<PathBuf, String> {
    let canonical =
        std::fs::canonicalize(requested).map_err(|e| format!("{}: {e}", requested.display()))?;
    let wikis = state
        .manager
        .list()
        .map_err(|e| format!("wiki manager unavailable: {e}"))?;
    for w in &wikis {
        let Some(lp) = w.local_path.as_deref() else {
            continue;
        };
        let Ok(root) = std::fs::canonicalize(lp) else {
            continue;
        };
        if canonical.starts_with(&root) {
            return Ok(canonical);
        }
    }
    Err(format!(
        "path {} is outside any registered wiki",
        requested.display()
    ))
}

/// Resolve `requested` for write. The file may not yet exist, so we
/// canonicalise the parent and require it to live under a wiki's
/// `.devcontainer/` directory (or the wiki root for `.devcontainer.json`).
fn resolve_for_write(state: &WikiState, requested: &Path) -> Result<PathBuf, String> {
    let wikis = state
        .manager
        .list()
        .map_err(|e| format!("wiki manager unavailable: {e}"))?;
    for w in &wikis {
        let Some(lp) = w.local_path.as_deref() else {
            continue;
        };
        let Ok(root) = std::fs::canonicalize(lp) else {
            continue;
        };
        let dc_dir = root.join(".devcontainer");
        let Some(parent) = requested.parent() else {
            continue;
        };
        if let Ok(canon_parent) = std::fs::canonicalize(parent) {
            if canon_parent.starts_with(&dc_dir) || canon_parent == root {
                return Ok(requested.to_path_buf());
            }
        }
    }
    Err(format!(
        "path {} is not a permitted write target",
        requested.display()
    ))
}

#[command]
pub async fn fs_is_file(state: State<'_, WikiState>, path: String) -> Result<bool, String> {
    let resolved = match resolve_within_wikis(&state, Path::new(&path)) {
        Ok(p) => p,
        Err(_) => return Ok(false),
    };
    Ok(tokio::fs::metadata(&resolved)
        .await
        .map(|m| m.is_file())
        .unwrap_or(false))
}

#[command]
pub async fn fs_read_file(state: State<'_, WikiState>, path: String) -> Result<Vec<u8>, String> {
    let resolved = resolve_within_wikis(&state, Path::new(&path))?;
    tokio::fs::read(&resolved).await.map_err(|e| e.to_string())
}

#[command]
pub async fn fs_write_file(
    state: State<'_, WikiState>,
    path: String,
    content: Vec<u8>,
) -> Result<(), String> {
    let resolved = resolve_for_write(&state, Path::new(&path))?;
    if let Some(parent) = resolved.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    tokio::fs::write(&resolved, content)
        .await
        .map_err(|e| e.to_string())
}

#[command]
pub async fn fs_read_dir(state: State<'_, WikiState>, path: String) -> Result<Vec<String>, String> {
    let resolved = resolve_within_wikis(&state, Path::new(&path))?;
    let mut entries = tokio::fs::read_dir(&resolved)
        .await
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        if let Some(name) = entry.file_name().to_str() {
            out.push(name.to_string());
        }
    }
    Ok(out)
}

#[command]
pub async fn fs_mkdirp(state: State<'_, WikiState>, path: String) -> Result<(), String> {
    let resolved = resolve_for_write(&state, Path::new(&path))?;
    tokio::fs::create_dir_all(&resolved)
        .await
        .map_err(|e| e.to_string())
}

/// Persist the parsed devcontainer.json for `wiki_id`. The frontend's
/// engine bundle calls this after `loadDevContainerConfig` resolves;
/// subsequent `wiki_container_ctl_*` commands then operate against it.
#[command]
pub async fn submit_parsed_devcontainer(
    wiki_state: State<'_, WikiState>,
    orchestrator: State<'_, LifecycleOrchestrator>,
    workspace_id: String,
    parsed: ParsedDevContainer,
) -> Result<(), String> {
    let wiki = wiki_state
        .manager
        .get(&workspace_id)
        .map_err(|e| format!("wiki lookup failed: {e}"))?
        .ok_or_else(|| format!("unknown wiki: {workspace_id}"))?;
    let path = wiki
        .local_path
        .as_ref()
        .map(PathBuf::from)
        .ok_or_else(|| format!("wiki {workspace_id} has no local path"))?;
    orchestrator.set_parsed_config(&workspace_id, parsed);
    orchestrator.record_host_workspace(&workspace_id, &path);
    Ok(())
}
