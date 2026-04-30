//! Provider trait definitions.
//!
//! These traits define the abstract interface that any hosting provider
//! (GitHub, Codeberg, bare Git, Cloudflare, etc.) must implement.

use crate::workspace::types::*;

/// Trait for repository operations (create, fork, clone, push, etc.).
///
/// Each provider implements this to talk to its specific API/protocol.
#[allow(async_fn_in_trait)]
pub trait RepoProvider {
    /// Create a new repository from a template.
    async fn create_from_template(
        &self,
        params: &CreateFromTemplateParams,
    ) -> Result<RepoCreatedInfo, ProviderError>;

    /// Fork an existing repository.
    async fn fork_repo(&self, params: &ForkRepoParams) -> Result<RepoCreatedInfo, ProviderError>;

    /// Get metadata for a remote repository.
    async fn get_repo_info(&self, owner: &str, repo: &str) -> Result<RepoMetadata, ProviderError>;
}

/// Trait for publishing a static site from a workspace.
#[allow(async_fn_in_trait)]
pub trait PublishProvider {
    /// Detect the current publishing mode for a workspace.
    async fn detect_publish_mode(
        &self,
        owner: &str,
        repo: &str,
        local_path: &str,
    ) -> Result<PublishMode, ProviderError>;

    /// Publish or update the site.
    async fn publish(&self, workspace: &Workspace) -> Result<PublishResult, ProviderError>;

    /// Get the URL where the site would be published.
    fn site_url(&self, owner: &str, repo: &str) -> String;
}

/// Information returned after creating a repo (from template or fork).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepoCreatedInfo {
    pub owner: String,
    pub repo: String,
    pub clone_url: String,
    pub html_url: String,
    pub default_branch: String,
}

/// Remote repository metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepoMetadata {
    pub owner: String,
    pub repo: String,
    pub clone_url: String,
    pub html_url: String,
    pub default_branch: String,
    pub is_fork: bool,
    pub parent_owner: Option<String>,
    pub parent_repo: Option<String>,
}

/// Errors from provider operations.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("Authentication required")]
    AuthRequired,
    #[error("Authentication failed: {0}")]
    AuthFailed(String),
    #[error("Repository not found: {owner}/{repo}")]
    RepoNotFound { owner: String, repo: String },
    #[error("Repository already exists: {owner}/{repo}")]
    RepoAlreadyExists { owner: String, repo: String },
    #[error("Fork is still being created, try again shortly")]
    ForkInProgress,
    #[error("API error: {0}")]
    Api(String),
    #[error("Network error: {0}")]
    Network(String),
    #[error("Git error: {0}")]
    Git(String),
    #[error("{0}")]
    Other(String),
}
