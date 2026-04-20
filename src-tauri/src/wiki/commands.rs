//! Tauri commands for wiki dashboard entries.

use tauri::{command, AppHandle, Manager};

use crate::wiki::manager::WikiManager;
use crate::wiki::types::*;

/// Shared state wrapping the `WikiManager`.
pub struct WikiState {
    pub manager: WikiManager,
    /// Default parent directory for new clones (e.g. `~/Wiki3`).
    pub default_base_dir: std::path::PathBuf,
}

impl WikiState {
    pub fn new(data_dir: std::path::PathBuf) -> Self {
        let default_base_dir = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("Wiki3");
        Self {
            manager: WikiManager::new(data_dir),
            default_base_dir,
        }
    }
}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// List all dashboard wikis.
#[command]
pub fn list_wikis(app: AppHandle) -> Result<Vec<Wiki>, String> {
    let state = app.state::<WikiState>();
    state.manager.list().map_err(err)
}

/// Get one wiki by id.
#[command]
pub fn get_wiki(app: AppHandle, wiki_id: String) -> Result<Option<Wiki>, String> {
    let state = app.state::<WikiState>();
    state.manager.get(&wiki_id).map_err(err)
}

/// Create a new dashboard wiki entry from loose parameters.
/// Any combination of local_path / remote_url / site_url may be provided
/// — at least one is required.
#[command]
pub fn add_wiki(app: AppHandle, params: AddWikiParams) -> Result<Wiki, String> {
    let state = app.state::<WikiState>();
    let wiki = state.manager.build_from_params(params).map_err(err)?;
    state.manager.add(wiki).map_err(err)
}

/// Update an existing wiki (partial).
#[command]
pub fn update_wiki(
    app: AppHandle,
    wiki_id: String,
    params: UpdateWikiParams,
) -> Result<Wiki, String> {
    let state = app.state::<WikiState>();
    state.manager.update(&wiki_id, params).map_err(err)
}

/// Remove a wiki from the dashboard (does not touch the local repo on disk).
#[command]
pub fn remove_wiki(app: AppHandle, wiki_id: String) -> Result<(), String> {
    let state = app.state::<WikiState>();
    state.manager.remove(&wiki_id).map_err(err)
}

/// Restore the default seeded wikis (adds them if not already present).
#[command]
pub fn restore_default_wikis(app: AppHandle) -> Result<Vec<Wiki>, String> {
    let state = app.state::<WikiState>();
    let existing = state.manager.list().map_err(err)?;
    let mut added = Vec::new();
    for mut seed in crate::wiki::manager::default_seeded_wikis() {
        let already = existing.iter().any(|w| {
            w.remote
                .as_ref()
                .zip(seed.remote.as_ref())
                .map(|(a, b)| a.owner == b.owner && a.repo == b.repo)
                .unwrap_or(false)
        });
        if !already {
            seed = state.manager.add(seed).map_err(err)?;
            added.push(seed);
        }
    }
    Ok(added)
}

/// Return the default base directory (e.g. `~/Wiki3`) for new clones.
#[command]
pub fn get_default_wikis_dir(app: AppHandle) -> Result<String, String> {
    let state = app.state::<WikiState>();
    // Ensure it exists so file dialogs can default into it.
    let _ = std::fs::create_dir_all(&state.default_base_dir);
    Ok(state.default_base_dir.to_string_lossy().to_string())
}

/// Open the site URL for a wiki in a new in-app window, recording the
/// wiki ownership so it can be tracked on the dashboard. If the wiki
/// has no `site_url` but has a GitHub remote, a conventional Pages URL
/// is derived and opened.
#[command]
pub fn open_wiki_site(app: AppHandle, wiki_id: String) -> Result<String, String> {
    let state = app.state::<WikiState>();
    let wiki = state
        .manager
        .get(&wiki_id)
        .map_err(err)?
        .ok_or_else(|| format!("Wiki not found: {wiki_id}"))?;

    let url = wiki
        .site_url
        .clone()
        .or_else(|| {
            wiki.remote
                .as_ref()
                .map(|r| derive_github_pages_url(&r.owner, &r.repo))
        })
        .ok_or_else(|| "Wiki has no site URL or remote to derive one from".to_string())?;

    crate::commands::open_new_window_with_geometry(
        app,
        url.clone(),
        None,
        None,
        None,
        None,
        Some(wiki_id),
    )?;
    Ok(url)
}

/// Open a wiki's remote repository URL in the system browser.
#[command]
pub fn open_wiki_remote(app: AppHandle, wiki_id: String) -> Result<String, String> {
    let state = app.state::<WikiState>();
    let wiki = state
        .manager
        .get(&wiki_id)
        .map_err(err)?
        .ok_or_else(|| format!("Wiki not found: {wiki_id}"))?;
    let url = wiki
        .remote
        .as_ref()
        .map(|r| r.url.clone())
        .ok_or_else(|| "Wiki has no remote".to_string())?;
    crate::commands::open_external_url(url.clone())?;
    Ok(url)
}

/// Reveal the wiki's local path in the OS file manager.
#[command]
pub fn reveal_wiki_local(app: AppHandle, wiki_id: String) -> Result<String, String> {
    let state = app.state::<WikiState>();
    let wiki = state
        .manager
        .get(&wiki_id)
        .map_err(err)?
        .ok_or_else(|| format!("Wiki not found: {wiki_id}"))?;
    let path = wiki
        .local_path
        .clone()
        .ok_or_else(|| "Wiki has no local path".to_string())?;
    crate::commands::reveal_path(path.clone())?;
    Ok(path)
}

/// Register an existing local git repo as a new wiki.
/// Detects the `origin` remote (if any) and populates the wiki's
/// `remote` and `site_url` fields accordingly.
#[command]
pub async fn open_local_repo_as_wiki(app: AppHandle, local_path: String) -> Result<Wiki, String> {
    use crate::git::ops as git;

    let trimmed = local_path.trim().to_string();
    if trimmed.is_empty() {
        return Err("No path provided".into());
    }

    let (remote, site_url) = if git::is_git_repo(&trimmed).await {
        let remotes = git::list_remotes(&trimmed).await.unwrap_or_default();
        let origin_url = remotes
            .iter()
            .find(|r| r.name == "origin")
            .map(|r| r.url.clone());
        let remote = origin_url.as_deref().and_then(remote_from_url);
        let site = remote
            .as_ref()
            .map(|r| derive_github_pages_url(&r.owner, &r.repo));
        (remote, site)
    } else {
        (None, None)
    };

    let state = app.state::<WikiState>();
    let name = std::path::Path::new(&trimmed)
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string());

    let wiki = Wiki {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.unwrap_or_else(|| "wiki".into()),
        local_path: Some(trimmed),
        remote,
        site_url,
        origin: WikiOrigin::Existing,
        description: None,
        created_at: chrono::Utc::now(),
        last_opened_at: chrono::Utc::now(),
    };

    state.manager.add(wiki).map_err(err)
}

/// Clone a remote repo to a user-chosen local folder and register it as a wiki.
/// The caller (frontend) is responsible for presenting the file dialog and
/// passing the selected absolute `target_path`. The backend verifies the
/// path does not already exist and runs `git clone` (unauthenticated).
#[command]
pub async fn clone_wiki(
    app: AppHandle,
    remote_url: String,
    target_path: String,
) -> Result<Wiki, String> {
    use tokio::process::Command;

    let remote_url = remote_url.trim().to_string();
    let target_path = target_path.trim().to_string();
    if remote_url.is_empty() {
        return Err("Remote URL is required".into());
    }
    if target_path.is_empty() {
        return Err("Target path is required".into());
    }

    let target = std::path::Path::new(&target_path);
    if target.exists() {
        // Accept an existing empty directory; reject otherwise.
        let is_empty = target
            .read_dir()
            .map(|mut d| d.next().is_none())
            .unwrap_or(false);
        if !is_empty {
            return Err(format!(
                "Target path already exists and is not empty: {target_path}"
            ));
        }
    } else if let Some(parent) = target.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }

    let output = Command::new("git")
        .arg("clone")
        .arg(&remote_url)
        .arg(&target_path)
        .output()
        .await
        .map_err(|e| format!("Failed to run git: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git clone failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let remote = remote_from_url(&remote_url);
    let site_url = remote
        .as_ref()
        .map(|r| derive_github_pages_url(&r.owner, &r.repo));
    let name = remote
        .as_ref()
        .map(|r| r.repo.clone())
        .or_else(|| {
            std::path::Path::new(&target_path)
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "wiki".into());

    let state = app.state::<WikiState>();
    let wiki = Wiki {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        local_path: Some(target_path),
        remote,
        site_url,
        origin: WikiOrigin::Clone,
        description: None,
        created_at: chrono::Utc::now(),
        last_opened_at: chrono::Utc::now(),
    };
    state.manager.add(wiki).map_err(err)
}
