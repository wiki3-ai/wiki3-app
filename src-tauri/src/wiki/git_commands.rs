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
use crate::publishing_commands::PublishingState;
use crate::wiki::commands::WikiState;
use crate::wiki::types::Wiki;
use crate::workspace::types::{GitStatus, PushResult};

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

/// Publish the wiki: push the current branch to `origin`. Site
/// hosting (e.g. GitHub Pages) is intentionally not enabled here —
/// users wire up their own CI / hosting per-repo.
#[command]
pub async fn wiki_publish(app: AppHandle, wiki_id: String) -> Result<serde_json::Value, String> {
    let wiki = get_wiki(&app, &wiki_id)?;
    // Require a remote so we fail clearly when there's nothing to push to.
    let _ = wiki
        .remote
        .as_ref()
        .ok_or_else(|| "This wiki has no remote to publish to".to_string())?;

    let push_result = wiki_push(app.clone(), wiki_id.clone()).await?;

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

/// `PublishingState` holds `data_dir` privately; expose a helper that
/// constructs the auth using our local path convention.
fn state_data_dir(state: &tauri::State<'_, PublishingState>) -> std::path::PathBuf {
    // PublishingState stores `data_dir` as a private field. The workspace
    // manager's storage_dir mirrors it — use that to avoid a breaking API
    // change on PublishingState.
    state.workspace_manager.storage_dir().to_path_buf()
}
