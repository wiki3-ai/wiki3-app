//! Provider-neutral workspace and publishing types.
//!
//! These types define the core domain model independent of any
//! specific hosting provider (GitHub, Codeberg, Cloudflare, etc.).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Identifies which hosting/repo provider backs a workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    GitHub,
}

/// How the static site is published from the repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublishMode {
    /// Publish from `/docs` folder on the main branch.
    DocsFolder,
    /// Publish from a dedicated `gh-pages` branch.
    GhPagesBranch,
    /// No publishing configured yet.
    None,
}

/// How the workspace was created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceOrigin {
    /// Created from a template repository.
    Template {
        template_owner: String,
        template_repo: String,
    },
    /// Forked from an existing repository.
    Fork {
        upstream_owner: String,
        upstream_repo: String,
    },
    /// Opened from an existing local directory.
    Existing,
}

/// Visibility of a repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepoVisibility {
    Public,
    Private,
}

/// Named remote (e.g. origin, upstream).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteInfo {
    pub name: String,
    pub url: String,
}

/// A workspace represents a local site project connected to a remote provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    /// Unique identifier for this workspace.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Absolute path to the local working directory.
    pub local_path: String,
    /// Which provider hosts the remote repo.
    pub provider: ProviderType,
    /// Repository owner (user or org).
    pub owner: String,
    /// Repository name.
    pub repo: String,
    /// Current branch.
    pub branch: String,
    /// Configured remotes.
    pub remotes: Vec<RemoteInfo>,
    /// How the site is published.
    pub publish_mode: PublishMode,
    /// URL where the published site is accessible.
    pub site_url: Option<String>,
    /// How this workspace was created.
    pub origin: WorkspaceOrigin,
    /// Visibility of the remote repo.
    pub visibility: RepoVisibility,
    /// Description of the repo.
    pub description: Option<String>,
    /// When this workspace was created.
    pub created_at: DateTime<Utc>,
    /// When this workspace was last opened/used.
    pub last_opened_at: DateTime<Utc>,
}

/// Parameters for creating a new site from a template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateFromTemplateParams {
    pub owner: String,
    pub repo_name: String,
    pub visibility: RepoVisibility,
    pub description: Option<String>,
    pub template_owner: String,
    pub template_repo: String,
}

/// Parameters for forking an existing repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForkRepoParams {
    pub source_owner: String,
    pub source_repo: String,
    /// Target owner for the fork (user or org). If None, forks to authenticated user.
    pub target_owner: Option<String>,
    /// Custom name for the fork. If None, uses the source repo name.
    pub fork_name: Option<String>,
}

/// Summary of local git status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitStatus {
    pub branch: String,
    pub dirty_files: Vec<DirtyFile>,
    pub ahead: u32,
    pub behind: u32,
    pub last_commit: Option<CommitInfo>,
}

/// A file with uncommitted changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirtyFile {
    pub path: String,
    pub status: FileStatus,
}

/// Type of change for a dirty file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
}

/// Information about a commit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitInfo {
    pub sha: String,
    pub message: String,
    pub author: String,
    pub date: String,
}

/// Result from a push operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushResult {
    pub success: bool,
    pub remote: String,
    pub branch: String,
    pub message: String,
}

/// Result from a publish operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishResult {
    pub success: bool,
    pub site_url: Option<String>,
    pub publish_mode: PublishMode,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workspace_serialization_roundtrip() {
        let ws = Workspace {
            id: "test-id".to_string(),
            name: "my-site".to_string(),
            local_path: "/tmp/my-site".to_string(),
            provider: ProviderType::GitHub,
            owner: "testuser".to_string(),
            repo: "my-site".to_string(),
            branch: "main".to_string(),
            remotes: vec![RemoteInfo {
                name: "origin".to_string(),
                url: "https://github.com/testuser/my-site.git".to_string(),
            }],
            publish_mode: PublishMode::GhPagesBranch,
            site_url: Some("https://testuser.github.io/my-site".to_string()),
            origin: WorkspaceOrigin::Template {
                template_owner: "wiki3-ai".to_string(),
                template_repo: "wiki3-ai-template".to_string(),
            },
            visibility: RepoVisibility::Public,
            description: Some("My wiki3 site".to_string()),
            created_at: Utc::now(),
            last_opened_at: Utc::now(),
        };

        let json = serde_json::to_string_pretty(&ws).unwrap();
        let deserialized: Workspace = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, ws.id);
        assert_eq!(deserialized.name, ws.name);
        assert_eq!(deserialized.provider, ProviderType::GitHub);
        assert_eq!(deserialized.publish_mode, PublishMode::GhPagesBranch);
        assert_eq!(deserialized.remotes.len(), 1);

        if let WorkspaceOrigin::Template {
            template_owner,
            template_repo,
        } = &deserialized.origin
        {
            assert_eq!(template_owner, "wiki3-ai");
            assert_eq!(template_repo, "wiki3-ai-template");
        } else {
            panic!("Expected Template origin");
        }
    }

    #[test]
    fn test_publish_mode_variants() {
        let modes = [
            (PublishMode::DocsFolder, "\"docs_folder\""),
            (PublishMode::GhPagesBranch, "\"gh_pages_branch\""),
            (PublishMode::None, "\"none\""),
        ];
        for (mode, expected_json) in &modes {
            let json = serde_json::to_string(mode).unwrap();
            assert_eq!(&json, expected_json);
        }
    }
}
