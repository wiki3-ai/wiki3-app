//! GitHub Pages publish provider — detect mode, publish static sites.

use std::path::Path;

use crate::git::ops as git;
use crate::providers::traits::*;
use crate::workspace::types::*;

use super::auth::{build_github_client, GitHubAuth};

/// GitHub Pages implementation of `PublishProvider`.
pub struct GitHubPagesPublishProvider {
    auth: GitHubAuth,
}

impl GitHubPagesPublishProvider {
    pub fn new(auth: GitHubAuth) -> Self {
        Self { auth }
    }
}

impl PublishProvider for GitHubPagesPublishProvider {
    async fn detect_publish_mode(
        &self,
        owner: &str,
        repo: &str,
        local_path: &str,
    ) -> Result<PublishMode, ProviderError> {
        // First check the GitHub Pages API for existing configuration.
        if let Ok(mode) = self.detect_from_api(owner, repo).await {
            if mode != PublishMode::None {
                return Ok(mode);
            }
        }

        // Fall back to local repo heuristics.
        Ok(detect_publish_mode_local(local_path))
    }

    async fn publish(
        &self,
        workspace: &Workspace,
    ) -> Result<PublishResult, ProviderError> {
        match &workspace.publish_mode {
            PublishMode::GhPagesBranch => {
                self.publish_gh_pages_branch(workspace).await
            }
            PublishMode::DocsFolder => {
                self.publish_docs_folder(workspace).await
            }
            PublishMode::None => Err(ProviderError::Other(
                "No publish mode configured. Set publish_mode to 'gh_pages_branch' or 'docs_folder'.".to_string(),
            )),
        }
    }

    fn site_url(&self, owner: &str, repo: &str) -> String {
        // GitHub Pages convention: owner.github.io/repo
        // For user/org sites (repo == owner.github.io), it's just owner.github.io
        if repo == format!("{owner}.github.io") {
            format!("https://{owner}.github.io")
        } else {
            format!("https://{owner}.github.io/{repo}")
        }
    }
}

impl GitHubPagesPublishProvider {
    /// Query the GitHub Pages API for an existing config.
    async fn detect_from_api(&self, owner: &str, repo: &str) -> Result<PublishMode, ProviderError> {
        let token = self.auth.get_token().map_err(|_| ProviderError::AuthRequired)?;
        let client =
            build_github_client(&token).map_err(|e| ProviderError::AuthFailed(e.to_string()))?;

        let url = format!("https://api.github.com/repos/{owner}/{repo}/pages");
        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| ProviderError::Network(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(PublishMode::None);
        }
        if !resp.status().is_success() {
            return Ok(PublishMode::None);
        }

        let pages: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Api(e.to_string()))?;

        // Check source configuration
        if let Some(source) = pages.get("source") {
            let branch = source["branch"].as_str().unwrap_or("");
            let path = source["path"].as_str().unwrap_or("/");

            if branch == "gh-pages" {
                return Ok(PublishMode::GhPagesBranch);
            }
            if path == "/docs" {
                return Ok(PublishMode::DocsFolder);
            }
        }

        // If build_type is "workflow" (GitHub Actions), still treat as gh-pages for now
        if pages.get("build_type").and_then(|v| v.as_str()) == Some("workflow") {
            return Ok(PublishMode::GhPagesBranch);
        }

        Ok(PublishMode::None)
    }

    /// Publish using the gh-pages branch strategy.
    ///
    /// This mirrors the approach in wiki3-ai-template/deploy.sh:
    /// build the site, then force-push the output to the gh-pages branch.
    async fn publish_gh_pages_branch(
        &self,
        workspace: &Workspace,
    ) -> Result<PublishResult, ProviderError> {
        let local_path = &workspace.local_path;
        let site_url = self.site_url(&workspace.owner, &workspace.repo);

        // Check for deploy.sh in the workspace (template convention)
        let deploy_script = Path::new(local_path).join("deploy.sh");
        if deploy_script.exists() {
            log::info!("Found deploy.sh, using it for publish");
            let output = git::run_command_in_dir(
                local_path,
                "bash",
                &["deploy.sh", "Publish from Wiki3 app"],
            )
            .await
            .map_err(|e| ProviderError::Git(e.to_string()))?;

            return Ok(PublishResult {
                success: true,
                site_url: Some(site_url),
                publish_mode: PublishMode::GhPagesBranch,
                message: format!("Published via deploy.sh:\n{output}"),
            });
        }

        // If no deploy.sh, check for _output directory (JupyterLite build output)
        let output_dir = Path::new(local_path).join("_output");
        if output_dir.exists() && output_dir.is_dir() {
            return self
                .force_push_directory_to_branch(
                    local_path,
                    "_output",
                    "gh-pages",
                    "Publish from Wiki3 app",
                )
                .await
                .map(|msg| PublishResult {
                    success: true,
                    site_url: Some(site_url),
                    publish_mode: PublishMode::GhPagesBranch,
                    message: msg,
                });
        }

        // If no build output, push current state to gh-pages
        // (suitable for static sites that don't need a build step)
        self.force_push_directory_to_branch(local_path, ".", "gh-pages", "Publish from Wiki3 app")
            .await
            .map(|msg| PublishResult {
                success: true,
                site_url: Some(site_url),
                publish_mode: PublishMode::GhPagesBranch,
                message: msg,
            })
    }

    /// Publish using the docs folder strategy.
    ///
    /// Ensures the /docs folder exists and is committed, then pushes to main.
    async fn publish_docs_folder(
        &self,
        workspace: &Workspace,
    ) -> Result<PublishResult, ProviderError> {
        let local_path = &workspace.local_path;
        let site_url = self.site_url(&workspace.owner, &workspace.repo);

        let docs_path = Path::new(local_path).join("docs");
        if !docs_path.exists() {
            return Err(ProviderError::Other(
                "No /docs folder found. Create a /docs folder with your site content.".to_string(),
            ));
        }

        // Stage docs, commit if dirty, push
        git::add(local_path, &["docs/"]).await.map_err(|e| ProviderError::Git(e.to_string()))?;

        let status = git::status(local_path)
            .await
            .map_err(|e| ProviderError::Git(e.to_string()))?;
        if !status.dirty_files.is_empty() {
            git::commit(local_path, "Update docs — publish from Wiki3 app")
                .await
                .map_err(|e| ProviderError::Git(e.to_string()))?;
        }

        let push_result = git::push(local_path, "origin", &status.branch)
            .await
            .map_err(|e| ProviderError::Git(e.to_string()))?;

        Ok(PublishResult {
            success: push_result.success,
            site_url: Some(site_url),
            publish_mode: PublishMode::DocsFolder,
            message: push_result.message,
        })
    }

    /// Force-push a directory's contents to a branch (used for gh-pages).
    async fn force_push_directory_to_branch(
        &self,
        repo_path: &str,
        source_dir: &str,
        branch: &str,
        message: &str,
    ) -> Result<String, ProviderError> {
        // Use a temporary worktree approach: create orphan branch, copy files, push
        let temp_dir = tempfile::tempdir()
            .map_err(|e| ProviderError::Other(e.to_string()))?;
        let temp_path = temp_dir.path().to_string_lossy().to_string();

        // Get remote URL
        let remote_url = git::get_remote_url(repo_path, "origin")
            .await
            .map_err(|e| ProviderError::Git(e.to_string()))?;

        // Init a new repo, create orphan branch
        git::run_command_in_dir(&temp_path, "git", &["init"])
            .await
            .map_err(|e| ProviderError::Git(e.to_string()))?;
        git::run_command_in_dir(&temp_path, "git", &["checkout", "--orphan", branch])
            .await
            .map_err(|e| ProviderError::Git(e.to_string()))?;

        // Copy source files
        let source_path = if source_dir == "." {
            repo_path.to_string()
        } else {
            format!("{repo_path}/{source_dir}")
        };
        git::run_command_in_dir(
            &temp_path,
            "bash",
            &["-c", &format!("cp -a {source_path}/. .")],
        )
        .await
        .map_err(|e| ProviderError::Git(e.to_string()))?;

        // Ensure .nojekyll exists
        let nojekyll = Path::new(&temp_path).join(".nojekyll");
        if !nojekyll.exists() {
            std::fs::write(&nojekyll, "")
                .map_err(|e| ProviderError::Other(e.to_string()))?;
        }

        // Remove .git from copied content if it got copied
        let dot_git = Path::new(&temp_path).join(".git-source");
        let _ = std::fs::remove_dir_all(dot_git);

        // Set git user for the temp repo
        git::run_command_in_dir(&temp_path, "git", &["config", "user.email", "wiki3-app@wiki3.ai"])
            .await
            .map_err(|e| ProviderError::Git(e.to_string()))?;
        git::run_command_in_dir(&temp_path, "git", &["config", "user.name", "Wiki3 App"])
            .await
            .map_err(|e| ProviderError::Git(e.to_string()))?;

        // Stage and commit
        git::run_command_in_dir(&temp_path, "git", &["add", "-A"])
            .await
            .map_err(|e| ProviderError::Git(e.to_string()))?;
        git::run_command_in_dir(&temp_path, "git", &["commit", "-m", message])
            .await
            .map_err(|e| ProviderError::Git(e.to_string()))?;

        // Add remote and force push
        // Inject token into URL for push auth
        let auth_url = self.inject_token_into_url(&remote_url)?;
        git::run_command_in_dir(&temp_path, "git", &["remote", "add", "origin", &auth_url])
            .await
            .map_err(|e| ProviderError::Git(e.to_string()))?;
        git::run_command_in_dir(
            &temp_path,
            "git",
            &["push", "--force", "origin", branch],
        )
        .await
        .map_err(|e| ProviderError::Git(e.to_string()))?;

        Ok(format!("Published to {branch} branch"))
    }

    /// Inject the GitHub token into a clone URL for authenticated push.
    fn inject_token_into_url(&self, url: &str) -> Result<String, ProviderError> {
        let token = self
            .auth
            .get_token()
            .map_err(|_| ProviderError::AuthRequired)?;

        if url.starts_with("https://github.com/") {
            // https://github.com/owner/repo.git →
            // https://x-access-token:{token}@github.com/owner/repo.git
            let rest = url.strip_prefix("https://").unwrap_or(url);
            Ok(format!("https://x-access-token:{token}@{rest}"))
        } else {
            // For SSH or other URLs, return as-is (SSH keys handle auth)
            Ok(url.to_string())
        }
    }
}

/// Detect publish mode from local repo structure (no API call needed).
pub fn detect_publish_mode_local(local_path: &str) -> PublishMode {
    let path = Path::new(local_path);

    // Check for deploy.sh (template convention = gh-pages branch)
    if path.join("deploy.sh").exists() {
        return PublishMode::GhPagesBranch;
    }

    // Check for _output directory (JupyterLite build)
    if path.join("_output").exists() {
        return PublishMode::GhPagesBranch;
    }

    // Check for docs folder
    if path.join("docs").is_dir() {
        return PublishMode::DocsFolder;
    }

    // Check for .github/workflows with pages deployment
    let workflows_dir = path.join(".github").join("workflows");
    if workflows_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&workflows_dir) {
            for entry in entries.flatten() {
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    if content.contains("gh-pages") || content.contains("github-pages") {
                        return PublishMode::GhPagesBranch;
                    }
                }
            }
        }
    }

    PublishMode::None
}

/// Enable GitHub Pages for a repository via the API.
pub async fn enable_github_pages(
    auth: &GitHubAuth,
    owner: &str,
    repo: &str,
    mode: &PublishMode,
) -> Result<(), ProviderError> {
    let token = auth.get_token().map_err(|_| ProviderError::AuthRequired)?;
    let client =
        build_github_client(&token).map_err(|e| ProviderError::AuthFailed(e.to_string()))?;

    let body = match mode {
        PublishMode::GhPagesBranch => serde_json::json!({
            "source": {
                "branch": "gh-pages",
                "path": "/"
            }
        }),
        PublishMode::DocsFolder => serde_json::json!({
            "source": {
                "branch": "main",
                "path": "/docs"
            }
        }),
        PublishMode::None => return Ok(()),
    };

    let url = format!("https://api.github.com/repos/{owner}/{repo}/pages");
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| ProviderError::Network(e.to_string()))?;

    let status = resp.status();
    // 201 = created, 409 = already exists (fine)
    if status.is_success() || status == reqwest::StatusCode::CONFLICT {
        Ok(())
    } else {
        let text = resp.text().await.unwrap_or_default();
        Err(ProviderError::Api(format!(
            "Failed to enable Pages: HTTP {status}: {text}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_publish_mode_with_deploy_script() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("deploy.sh"), "#!/bin/bash\necho deploy").unwrap();
        let mode = detect_publish_mode_local(&dir.path().to_string_lossy());
        assert_eq!(mode, PublishMode::GhPagesBranch);
    }

    #[test]
    fn test_detect_publish_mode_with_docs_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("docs")).unwrap();
        let mode = detect_publish_mode_local(&dir.path().to_string_lossy());
        assert_eq!(mode, PublishMode::DocsFolder);
    }

    #[test]
    fn test_detect_publish_mode_with_output_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("_output")).unwrap();
        let mode = detect_publish_mode_local(&dir.path().to_string_lossy());
        assert_eq!(mode, PublishMode::GhPagesBranch);
    }

    #[test]
    fn test_detect_publish_mode_none() {
        let dir = tempfile::tempdir().unwrap();
        let mode = detect_publish_mode_local(&dir.path().to_string_lossy());
        assert_eq!(mode, PublishMode::None);
    }

    #[test]
    fn test_site_url_for_project_repo() {
        let auth = GitHubAuth::new(tempfile::tempdir().unwrap().path().to_path_buf());
        let provider = GitHubPagesPublishProvider::new(auth);
        assert_eq!(
            provider.site_url("testuser", "my-site"),
            "https://testuser.github.io/my-site"
        );
    }

    #[test]
    fn test_site_url_for_user_site() {
        let auth = GitHubAuth::new(tempfile::tempdir().unwrap().path().to_path_buf());
        let provider = GitHubPagesPublishProvider::new(auth);
        assert_eq!(
            provider.site_url("testuser", "testuser.github.io"),
            "https://testuser.github.io"
        );
    }

    #[test]
    fn test_detect_publish_mode_with_workflow() {
        let dir = tempfile::tempdir().unwrap();
        let wf_dir = dir.path().join(".github").join("workflows");
        std::fs::create_dir_all(&wf_dir).unwrap();
        std::fs::write(
            wf_dir.join("deploy.yml"),
            "name: Deploy\non:\n  push:\njobs:\n  deploy:\n    - uses: gh-pages",
        )
        .unwrap();
        let mode = detect_publish_mode_local(&dir.path().to_string_lossy());
        assert_eq!(mode, PublishMode::GhPagesBranch);
    }
}
