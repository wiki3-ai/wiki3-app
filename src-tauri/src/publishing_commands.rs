//! Tauri commands for the publishing workflow.
//!
//! These commands expose the workspace, git, auth, and publish modules
//! to the TypeScript frontend via Tauri's IPC invoke mechanism.

use tauri::{command, AppHandle, Manager};

use crate::git::ops as git;
use crate::providers::github::auth::GitHubAuth;
use crate::providers::github::publish::{
    detect_publish_mode_local, enable_github_pages, GitHubPagesPublishProvider,
};
use crate::providers::github::repo::{poll_fork_ready, GitHubRepoProvider};
use crate::providers::traits::{PublishProvider, RepoProvider};
use crate::workspace::manager::WorkspaceManager;
use crate::workspace::types::*;

/// Shared state for publishing operations.
pub struct PublishingState {
    pub workspace_manager: WorkspaceManager,
    data_dir: std::path::PathBuf,
}

impl PublishingState {
    pub fn new(data_dir: std::path::PathBuf) -> Self {
        let workspaces_dir = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("Wiki3Sites");

        Self {
            workspace_manager: WorkspaceManager::new(
                data_dir.clone(),
                workspaces_dir,
            ),
            data_dir,
        }
    }

    fn auth(&self) -> GitHubAuth {
        GitHubAuth::new(self.data_dir.clone())
    }

    fn repo_provider(&self) -> GitHubRepoProvider {
        GitHubRepoProvider::new(self.auth())
    }

    fn publish_provider(&self) -> GitHubPagesPublishProvider {
        GitHubPagesPublishProvider::new(self.auth())
    }
}

// =============================================================================
// Auth commands
// =============================================================================

/// Store a GitHub token securely.
#[command]
pub async fn store_github_token(
    app: AppHandle,
    token: String,
) -> Result<serde_json::Value, String> {
    let state = app.state::<PublishingState>();
    let auth = state.auth();

    auth.store_token(&token).map_err(|e| e.to_string())?;

    // Validate the token
    match auth.validate_token().await {
        Ok(user) => Ok(serde_json::json!({
            "authenticated": true,
            "user": {
                "login": user.login,
                "name": user.name,
                "avatar_url": user.avatar_url,
            }
        })),
        Err(e) => {
            // Clear invalid token
            let _ = auth.clear_token();
            Err(format!("Token validation failed: {e}"))
        }
    }
}

/// Check current authentication status.
#[command]
pub async fn get_auth_status(
    app: AppHandle,
) -> Result<serde_json::Value, String> {
    let state = app.state::<PublishingState>();
    let auth = state.auth();

    if !auth.has_token() {
        return Ok(serde_json::json!({
            "authenticated": false,
        }));
    }

    match auth.validate_token().await {
        Ok(user) => Ok(serde_json::json!({
            "authenticated": true,
            "user": {
                "login": user.login,
                "name": user.name,
                "avatar_url": user.avatar_url,
            }
        })),
        Err(_) => Ok(serde_json::json!({
            "authenticated": false,
            "reason": "token_invalid",
        })),
    }
}

/// Clear stored GitHub credentials.
#[command]
pub async fn clear_github_auth(
    app: AppHandle,
) -> Result<serde_json::Value, String> {
    let state = app.state::<PublishingState>();
    state.auth().clear_token().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "cleared": true }))
}

// =============================================================================
// Workspace commands
// =============================================================================

/// List all known workspaces.
#[command]
pub async fn list_workspaces(
    app: AppHandle,
) -> Result<Vec<Workspace>, String> {
    let state = app.state::<PublishingState>();
    state
        .workspace_manager
        .list_workspaces()
        .map_err(|e| e.to_string())
}

/// Get a single workspace by id.
#[command]
pub async fn get_workspace(
    app: AppHandle,
    workspace_id: String,
) -> Result<Option<Workspace>, String> {
    let state = app.state::<PublishingState>();
    state
        .workspace_manager
        .get_workspace(&workspace_id)
        .map_err(|e| e.to_string())
}

/// Remove a workspace (metadata only, does not delete files).
#[command]
pub async fn remove_workspace(
    app: AppHandle,
    workspace_id: String,
) -> Result<serde_json::Value, String> {
    let state = app.state::<PublishingState>();
    state
        .workspace_manager
        .remove_workspace(&workspace_id)
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "removed": true }))
}

// =============================================================================
// Create from template
// =============================================================================

/// Create a new site from a template repository.
#[command]
pub async fn create_site_from_template(
    app: AppHandle,
    owner: String,
    repo_name: String,
    visibility: RepoVisibility,
    description: Option<String>,
    template_owner: Option<String>,
    template_repo: Option<String>,
) -> Result<Workspace, String> {
    let state = app.state::<PublishingState>();
    let provider = state.repo_provider();

    let tmpl_owner = template_owner.unwrap_or_else(|| "wiki3-ai".to_string());
    let tmpl_repo = template_repo.unwrap_or_else(|| "wiki3-ai-template".to_string());

    let params = CreateFromTemplateParams {
        owner: owner.clone(),
        repo_name: repo_name.clone(),
        visibility: visibility.clone(),
        description: description.clone(),
        template_owner: tmpl_owner.clone(),
        template_repo: tmpl_repo.clone(),
    };

    // Create the repo via GitHub API
    let created = provider
        .create_from_template(&params)
        .await
        .map_err(|e| e.to_string())?;

    // Clone locally
    let local_path = state
        .workspace_manager
        .default_clone_path(&created.repo)
        .to_string_lossy()
        .to_string();

    let token = state.auth().get_token().map_err(|e| e.to_string())?;
    git::clone_authenticated(&created.clone_url, &local_path, &token)
        .await
        .map_err(|e| e.to_string())?;

    // Detect publish mode
    let publish_mode = detect_publish_mode_local(&local_path);
    let publish_provider = state.publish_provider();
    let site_url = publish_provider.site_url(&created.owner, &created.repo);

    // Create workspace record
    let workspace = Workspace {
        id: uuid::Uuid::new_v4().to_string(),
        name: created.repo.clone(),
        local_path,
        provider: ProviderType::GitHub,
        owner: created.owner.clone(),
        repo: created.repo.clone(),
        branch: created.default_branch,
        remotes: vec![RemoteInfo {
            name: "origin".to_string(),
            url: created.clone_url,
        }],
        publish_mode,
        site_url: Some(site_url),
        origin: WorkspaceOrigin::Template {
            template_owner: tmpl_owner,
            template_repo: tmpl_repo,
        },
        visibility,
        description,
        created_at: chrono::Utc::now(),
        last_opened_at: chrono::Utc::now(),
    };

    state
        .workspace_manager
        .add_workspace(workspace.clone())
        .map_err(|e| e.to_string())?;

    Ok(workspace)
}

// =============================================================================
// Fork
// =============================================================================

/// Fork an existing repository and clone it locally.
#[command]
pub async fn fork_site(
    app: AppHandle,
    source_owner: String,
    source_repo: String,
    target_owner: Option<String>,
    fork_name: Option<String>,
) -> Result<Workspace, String> {
    let state = app.state::<PublishingState>();
    let provider = state.repo_provider();
    let auth = state.auth();

    let params = ForkRepoParams {
        source_owner: source_owner.clone(),
        source_repo: source_repo.clone(),
        target_owner,
        fork_name,
    };

    // Create the fork via GitHub API
    let created = provider
        .fork_repo(&params)
        .await
        .map_err(|e| e.to_string())?;

    // Poll until the fork is ready (GitHub forks are asynchronous)
    poll_fork_ready(&auth, &created.owner, &created.repo, 15)
        .await
        .map_err(|e| e.to_string())?;

    // Clone locally
    let local_path = state
        .workspace_manager
        .default_clone_path(&created.repo)
        .to_string_lossy()
        .to_string();

    let token = auth.get_token().map_err(|e| e.to_string())?;
    git::clone_authenticated(&created.clone_url, &local_path, &token)
        .await
        .map_err(|e| e.to_string())?;

    // Add upstream remote pointing to the source repo
    let upstream_url = format!(
        "https://github.com/{source_owner}/{source_repo}.git"
    );
    git::add_remote(&local_path, "upstream", &upstream_url)
        .await
        .map_err(|e| e.to_string())?;

    // Detect publish mode
    let publish_mode = detect_publish_mode_local(&local_path);
    let publish_provider = state.publish_provider();
    let site_url = publish_provider.site_url(&created.owner, &created.repo);

    let workspace = Workspace {
        id: uuid::Uuid::new_v4().to_string(),
        name: created.repo.clone(),
        local_path: local_path.clone(),
        provider: ProviderType::GitHub,
        owner: created.owner.clone(),
        repo: created.repo.clone(),
        branch: created.default_branch,
        remotes: vec![
            RemoteInfo {
                name: "origin".to_string(),
                url: created.clone_url,
            },
            RemoteInfo {
                name: "upstream".to_string(),
                url: upstream_url,
            },
        ],
        publish_mode,
        site_url: Some(site_url),
        origin: WorkspaceOrigin::Fork {
            upstream_owner: source_owner,
            upstream_repo: source_repo,
        },
        visibility: RepoVisibility::Public,
        description: None,
        created_at: chrono::Utc::now(),
        last_opened_at: chrono::Utc::now(),
    };

    state
        .workspace_manager
        .add_workspace(workspace.clone())
        .map_err(|e| e.to_string())?;

    Ok(workspace)
}

// =============================================================================
// Git operations
// =============================================================================

/// Get git status for a workspace.
#[command]
pub async fn get_git_status(
    app: AppHandle,
    workspace_id: String,
) -> Result<GitStatus, String> {
    let state = app.state::<PublishingState>();
    let ws = state
        .workspace_manager
        .get_workspace(&workspace_id)
        .map_err(|e| e.to_string())?
        .ok_or("Workspace not found")?;

    git::status(&ws.local_path)
        .await
        .map_err(|e| e.to_string())
}

/// Commit all changes in a workspace.
#[command]
pub async fn commit_changes(
    app: AppHandle,
    workspace_id: String,
    message: String,
) -> Result<CommitInfo, String> {
    let state = app.state::<PublishingState>();
    let ws = state
        .workspace_manager
        .get_workspace(&workspace_id)
        .map_err(|e| e.to_string())?
        .ok_or("Workspace not found")?;

    git::add_all(&ws.local_path)
        .await
        .map_err(|e| e.to_string())?;

    git::commit(&ws.local_path, &message)
        .await
        .map_err(|e| e.to_string())
}

/// Push current branch to origin.
#[command]
pub async fn push_changes(
    app: AppHandle,
    workspace_id: String,
) -> Result<PushResult, String> {
    let state = app.state::<PublishingState>();
    let ws = state
        .workspace_manager
        .get_workspace(&workspace_id)
        .map_err(|e| e.to_string())?
        .ok_or("Workspace not found")?;

    let token = state.auth().get_token().map_err(|e| e.to_string())?;
    git::push_authenticated(&ws.local_path, "origin", &ws.branch, &token)
        .await
        .map_err(|e| e.to_string())
}

/// Commit and push in one operation.
#[command]
pub async fn commit_and_push(
    app: AppHandle,
    workspace_id: String,
    message: String,
) -> Result<serde_json::Value, String> {
    let state = app.state::<PublishingState>();
    let ws = state
        .workspace_manager
        .get_workspace(&workspace_id)
        .map_err(|e| e.to_string())?
        .ok_or("Workspace not found")?;

    // Stage all
    git::add_all(&ws.local_path)
        .await
        .map_err(|e| e.to_string())?;

    // Commit
    let commit_info = git::commit(&ws.local_path, &message)
        .await
        .map_err(|e| e.to_string())?;

    // Push
    let token = state.auth().get_token().map_err(|e| e.to_string())?;
    let push_result = git::push_authenticated(&ws.local_path, "origin", &ws.branch, &token)
        .await
        .map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "commit": commit_info,
        "push": push_result,
    }))
}

// =============================================================================
// Publish
// =============================================================================

/// Publish/update the site for a workspace.
#[command]
pub async fn publish_site(
    app: AppHandle,
    workspace_id: String,
) -> Result<PublishResult, String> {
    let state = app.state::<PublishingState>();
    let ws = state
        .workspace_manager
        .get_workspace(&workspace_id)
        .map_err(|e| e.to_string())?
        .ok_or("Workspace not found")?;

    let publish_provider = state.publish_provider();

    // Try to enable GitHub Pages if not already configured
    let auth = state.auth();
    let _ = enable_github_pages(&auth, &ws.owner, &ws.repo, &ws.publish_mode).await;

    publish_provider
        .publish(&ws)
        .await
        .map_err(|e| e.to_string())
}

/// Detect the publish mode for a workspace.
#[command]
pub async fn detect_workspace_publish_mode(
    app: AppHandle,
    workspace_id: String,
) -> Result<PublishMode, String> {
    let state = app.state::<PublishingState>();
    let ws = state
        .workspace_manager
        .get_workspace(&workspace_id)
        .map_err(|e| e.to_string())?
        .ok_or("Workspace not found")?;

    let publish_provider = state.publish_provider();
    publish_provider
        .detect_publish_mode(&ws.owner, &ws.repo, &ws.local_path)
        .await
        .map_err(|e| e.to_string())
}

// =============================================================================
// Open existing local workspace
// =============================================================================

/// Register an existing local git repo as a workspace.
#[command]
pub async fn open_local_workspace(
    app: AppHandle,
    local_path: String,
) -> Result<Workspace, String> {
    let state = app.state::<PublishingState>();

    if !git::is_git_repo(&local_path).await {
        return Err("The specified path is not a git repository".to_string());
    }

    // Extract repo info from remotes
    let remotes = git::list_remotes(&local_path)
        .await
        .map_err(|e| e.to_string())?;

    let origin_url = remotes
        .iter()
        .find(|r| r.name == "origin")
        .map(|r| r.url.clone())
        .unwrap_or_default();

    // Parse owner/repo from GitHub URL
    let (owner, repo) = parse_github_url(&origin_url).unwrap_or(("unknown".to_string(), "unknown".to_string()));

    let branch = git::current_branch(&local_path)
        .await
        .unwrap_or_else(|_| "main".to_string());

    let publish_mode = detect_publish_mode_local(&local_path);
    let publish_provider = state.publish_provider();
    let site_url = if owner != "unknown" {
        Some(publish_provider.site_url(&owner, &repo))
    } else {
        None
    };

    let name = std::path::Path::new(&local_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("workspace")
        .to_string();

    let workspace = Workspace {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        local_path,
        provider: ProviderType::GitHub,
        owner,
        repo,
        branch,
        remotes,
        publish_mode,
        site_url,
        origin: WorkspaceOrigin::Existing,
        visibility: RepoVisibility::Public,
        description: None,
        created_at: chrono::Utc::now(),
        last_opened_at: chrono::Utc::now(),
    };

    state
        .workspace_manager
        .add_workspace(workspace.clone())
        .map_err(|e| e.to_string())?;

    Ok(workspace)
}

/// Parse owner/repo from a GitHub URL.
fn parse_github_url(url: &str) -> Option<(String, String)> {
    // Handle https://github.com/owner/repo.git
    // Handle git@github.com:owner/repo.git
    let path = if url.contains("github.com/") {
        url.split("github.com/").nth(1)?
    } else if url.contains("github.com:") {
        url.split("github.com:").nth(1)?
    } else {
        return None;
    };

    let path = path.trim_end_matches(".git");
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() >= 2 {
        Some((parts[0].to_string(), parts[1].to_string()))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_github_url_https() {
        let result = parse_github_url("https://github.com/wiki3-ai/wiki3-app.git");
        assert_eq!(result, Some(("wiki3-ai".to_string(), "wiki3-app".to_string())));
    }

    #[test]
    fn test_parse_github_url_ssh() {
        let result = parse_github_url("git@github.com:wiki3-ai/wiki3-app.git");
        assert_eq!(result, Some(("wiki3-ai".to_string(), "wiki3-app".to_string())));
    }

    #[test]
    fn test_parse_github_url_no_git_suffix() {
        let result = parse_github_url("https://github.com/user/repo");
        assert_eq!(result, Some(("user".to_string(), "repo".to_string())));
    }

    #[test]
    fn test_parse_github_url_invalid() {
        assert!(parse_github_url("https://gitlab.com/user/repo").is_none());
        assert!(parse_github_url("not-a-url").is_none());
    }
}
