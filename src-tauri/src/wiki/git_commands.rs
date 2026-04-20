//! Per-wiki git and publishing commands.
//!
//! These commands adapt the existing `git::ops` and `publishing_commands`
//! logic so they can be driven directly by a `Wiki`'s `local_path`
//! (and optional remote), without first creating a matching `Workspace`.
//!
//! The per-card **Commit**, **Push**, **Pull**, **Publish** buttons and
//! the **Publish-on-Commit** checkbox on the dashboard all invoke these.

use tauri::{command, AppHandle, Manager};

use crate::git::ops as git;
use crate::providers::github::auth::GitHubAuth;
use crate::providers::github::publish::enable_github_pages;
use crate::publishing_commands::PublishingState;
use crate::wiki::commands::WikiState;
use crate::wiki::types::Wiki;
use crate::workspace::types::{GitStatus, PublishMode, PushResult};

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn get_wiki(app: &AppHandle, wiki_id: &str) -> Result<Wiki, String> {
    app.state::<WikiState>()
        .manager
        .get(wiki_id)
        .map_err(err)?
        .ok_or_else(|| format!("Wiki not found: {wiki_id}"))
}

fn require_local(wiki: &Wiki) -> Result<String, String> {
    wiki.local_path
        .clone()
        .filter(|p| !p.trim().is_empty())
        .ok_or_else(|| "This wiki has no local path".to_string())
}

/// Get git status for the wiki's local path.
#[command]
pub async fn wiki_git_status(app: AppHandle, wiki_id: String) -> Result<GitStatus, String> {
    let wiki = get_wiki(&app, &wiki_id)?;
    let path = require_local(&wiki)?;
    if !git::is_git_repo(&path).await {
        return Err(format!("Not a git repository: {path}"));
    }
    git::status(&path).await.map_err(err)
}

/// Stage all changes and commit.
#[command]
pub async fn wiki_commit(
    app: AppHandle,
    wiki_id: String,
    message: String,
) -> Result<serde_json::Value, String> {
    let message = message.trim().to_string();
    if message.is_empty() {
        return Err("Commit message is required".into());
    }
    let wiki = get_wiki(&app, &wiki_id)?;
    let path = require_local(&wiki)?;
    if !git::is_git_repo(&path).await {
        return Err(format!("Not a git repository: {path}"));
    }

    git::add_all(&path).await.map_err(err)?;
    let commit = git::commit(&path, &message).await.map_err(err)?;
    Ok(serde_json::json!({ "commit": commit }))
}

/// Push the wiki's current branch to `origin` (authenticated if a token exists).
#[command]
pub async fn wiki_push(app: AppHandle, wiki_id: String) -> Result<PushResult, String> {
    let wiki = get_wiki(&app, &wiki_id)?;
    let path = require_local(&wiki)?;
    let branch = git::current_branch(&path).await.map_err(err)?;

    let state = app.state::<PublishingState>();
    let token = GitHubAuth::new(state_data_dir(&state).to_path_buf())
        .get_token()
        .ok();

    if let Some(tok) = token {
        git::push_authenticated(&path, "origin", &branch, &tok)
            .await
            .map_err(err)
    } else {
        // Fall back to unauthenticated push — works if git credentials
        // are configured outside the app (SSH keys, helper, etc.)
        git::push(&path, "origin", &branch).await.map_err(err)
    }
}

/// Pull the wiki's current branch from `origin`.
#[command]
pub async fn wiki_pull(app: AppHandle, wiki_id: String) -> Result<String, String> {
    let wiki = get_wiki(&app, &wiki_id)?;
    let path = require_local(&wiki)?;
    let branch = git::current_branch(&path).await.map_err(err)?;
    git::pull(&path, "origin", &branch).await.map_err(err)
}

/// Publish the wiki: push and best-effort enable GitHub Pages. Does not
/// block waiting for the Pages build to complete (that can take minutes);
/// the UI should call `wiki_pull` later to refresh the local copy.
#[command]
pub async fn wiki_publish(
    app: AppHandle,
    wiki_id: String,
) -> Result<serde_json::Value, String> {
    let wiki = get_wiki(&app, &wiki_id)?;
    let path = require_local(&wiki)?;
    let remote = wiki
        .remote
        .as_ref()
        .ok_or_else(|| "This wiki has no remote to publish to".to_string())?;

    // Push first.
    let push_result = wiki_push(app.clone(), wiki_id.clone()).await?;

    // Best-effort: enable Pages. Requires an auth token; log & continue on failure.
    let state = app.state::<PublishingState>();
    let auth = GitHubAuth::new(state_data_dir(&state).to_path_buf());
    if auth.has_token() {
        let publish_mode = crate::providers::github::publish::detect_publish_mode_local(&path);
        if !matches!(publish_mode, PublishMode::None) {
            if let Err(e) =
                enable_github_pages(&auth, &remote.owner, &remote.repo, &publish_mode).await
            {
                log::warn!("enable_github_pages failed: {e}");
            }
        }
    }

    Ok(serde_json::json!({
        "push": push_result,
        "site_url": wiki.site_url,
    }))
}

/// Convenience: commit, then (if the wiki has `publish_on_commit` set, or
/// the caller overrides) push and publish. This is what the "Commit" button
/// on the dashboard calls so one click can do the whole chain.
#[command]
pub async fn wiki_commit_and_maybe_publish(
    app: AppHandle,
    wiki_id: String,
    message: String,
    also_publish: Option<bool>,
) -> Result<serde_json::Value, String> {
    let wiki = get_wiki(&app, &wiki_id)?;
    let publish = also_publish.unwrap_or(wiki.publish_on_commit);

    let commit_result = wiki_commit(app.clone(), wiki_id.clone(), message).await?;

    if publish {
        let publish_result = wiki_publish(app, wiki_id).await?;
        Ok(serde_json::json!({
            "committed": true,
            "published": true,
            "commit": commit_result,
            "publish": publish_result,
        }))
    } else {
        Ok(serde_json::json!({
            "committed": true,
            "published": false,
            "commit": commit_result,
        }))
    }
}

/// Build the JupyterLite static site for a wiki by invoking
/// `jupyter lite build` in the wiki's local directory.
///
/// Requires `jupyter` and the `jupyterlite-core` plugin to be installed
/// and available on PATH. The built site is written to `_output/` by
/// default (JupyterLite's convention). Local serving of that directory
/// is a planned follow-up.
#[command]
pub async fn wiki_build_site(
    app: AppHandle,
    wiki_id: String,
) -> Result<serde_json::Value, String> {
    use tokio::process::Command;

    let wiki = get_wiki(&app, &wiki_id)?;
    let path = require_local(&wiki)?;
    if !std::path::Path::new(&path).exists() {
        return Err(format!("Local path does not exist: {path}"));
    }

    let output = Command::new("jupyter")
        .args(["lite", "build"])
        .current_dir(&path)
        .output()
        .await
        .map_err(|e| {
            format!(
                "Failed to run `jupyter lite build`: {e}. \
                 Make sure `jupyter` and `jupyterlite-core` are installed and on PATH \
                 (e.g. `pip install jupyterlite-core`)."
            )
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        return Err(format!(
            "jupyter lite build failed:\n{}",
            if stderr.trim().is_empty() { &stdout } else { &stderr }
        ));
    }

    Ok(serde_json::json!({
        "success": true,
        "output_dir": std::path::Path::new(&path).join("_output").to_string_lossy(),
        "stdout": stdout,
        "stderr": stderr,
    }))
}

/// `PublishingState` holds `data_dir` privately; expose a helper that
/// constructs the auth using our local path convention.
fn state_data_dir(state: &tauri::State<'_, PublishingState>) -> std::path::PathBuf {
    // PublishingState stores `data_dir` as a private field. The workspace
    // manager's storage_dir mirrors it — use that to avoid a breaking API
    // change on PublishingState.
    state.workspace_manager.storage_dir().to_path_buf()
}
