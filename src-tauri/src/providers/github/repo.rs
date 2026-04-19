//! GitHub repository provider — create from template, fork, metadata.

use crate::providers::traits::*;
use crate::workspace::types::*;

use super::auth::{build_github_client, AuthError, GitHubAuth};

/// GitHub implementation of `RepoProvider`.
pub struct GitHubRepoProvider {
    auth: GitHubAuth,
}

impl GitHubRepoProvider {
    pub fn new(auth: GitHubAuth) -> Self {
        Self { auth }
    }

    /// Get an authenticated HTTP client.
    fn client(&self) -> Result<reqwest::Client, ProviderError> {
        let token = self
            .auth
            .get_token()
            .map_err(|e| match e {
                AuthError::NoToken => ProviderError::AuthRequired,
                other => ProviderError::AuthFailed(other.to_string()),
            })?;
        build_github_client(&token).map_err(|e| ProviderError::AuthFailed(e.to_string()))
    }
}

impl RepoProvider for GitHubRepoProvider {
    async fn create_from_template(
        &self,
        params: &CreateFromTemplateParams,
    ) -> Result<RepoCreatedInfo, ProviderError> {
        let client = self.client()?;
        let url = format!(
            "https://api.github.com/repos/{}/{}/generate",
            params.template_owner, params.template_repo,
        );

        let mut body = serde_json::json!({
            "owner": params.owner,
            "name": params.repo_name,
            "private": params.visibility == RepoVisibility::Private,
        });
        if let Some(desc) = &params.description {
            body["description"] = serde_json::Value::String(desc.clone());
        }

        let resp = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Network(e.to_string()))?;

        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED
            || status == reqwest::StatusCode::FORBIDDEN
        {
            return Err(ProviderError::AuthFailed(format!("HTTP {status}")));
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(ProviderError::RepoNotFound {
                owner: params.template_owner.clone(),
                repo: params.template_repo.clone(),
            });
        }
        if status == reqwest::StatusCode::UNPROCESSABLE_ENTITY {
            let text = resp.text().await.unwrap_or_default();
            if text.contains("already exists") {
                return Err(ProviderError::RepoAlreadyExists {
                    owner: params.owner.clone(),
                    repo: params.repo_name.clone(),
                });
            }
            return Err(ProviderError::Api(format!("Validation error: {text}")));
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Api(format!("HTTP {status}: {text}")));
        }

        let repo_json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Api(e.to_string()))?;

        Ok(RepoCreatedInfo {
            owner: repo_json["owner"]["login"]
                .as_str()
                .unwrap_or(&params.owner)
                .to_string(),
            repo: repo_json["name"]
                .as_str()
                .unwrap_or(&params.repo_name)
                .to_string(),
            clone_url: repo_json["clone_url"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            html_url: repo_json["html_url"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            default_branch: repo_json["default_branch"]
                .as_str()
                .unwrap_or("main")
                .to_string(),
        })
    }

    async fn fork_repo(&self, params: &ForkRepoParams) -> Result<RepoCreatedInfo, ProviderError> {
        let client = self.client()?;
        let url = format!(
            "https://api.github.com/repos/{}/{}/forks",
            params.source_owner, params.source_repo,
        );

        let mut body = serde_json::json!({});
        if let Some(org) = &params.target_owner {
            body["organization"] = serde_json::Value::String(org.clone());
        }
        if let Some(name) = &params.fork_name {
            body["name"] = serde_json::Value::String(name.clone());
        }

        let resp = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Network(e.to_string()))?;

        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED
            || status == reqwest::StatusCode::FORBIDDEN
        {
            return Err(ProviderError::AuthFailed(format!("HTTP {status}")));
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(ProviderError::RepoNotFound {
                owner: params.source_owner.clone(),
                repo: params.source_repo.clone(),
            });
        }
        if !status.is_success() && status != reqwest::StatusCode::ACCEPTED {
            let text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Api(format!("HTTP {status}: {text}")));
        }

        let fork_json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Api(e.to_string()))?;

        Ok(RepoCreatedInfo {
            owner: fork_json["owner"]["login"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            repo: fork_json["name"].as_str().unwrap_or("").to_string(),
            clone_url: fork_json["clone_url"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            html_url: fork_json["html_url"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            default_branch: fork_json["default_branch"]
                .as_str()
                .unwrap_or("main")
                .to_string(),
        })
    }

    async fn get_repo_info(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<RepoMetadata, ProviderError> {
        let client = self.client()?;
        let url = format!("https://api.github.com/repos/{owner}/{repo}");

        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| ProviderError::Network(e.to_string()))?;

        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(ProviderError::RepoNotFound {
                owner: owner.to_string(),
                repo: repo.to_string(),
            });
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Api(format!("HTTP {status}: {text}")));
        }

        let repo_json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Api(e.to_string()))?;

        let is_fork = repo_json["fork"].as_bool().unwrap_or(false);
        Ok(RepoMetadata {
            owner: repo_json["owner"]["login"]
                .as_str()
                .unwrap_or(owner)
                .to_string(),
            repo: repo_json["name"]
                .as_str()
                .unwrap_or(repo)
                .to_string(),
            clone_url: repo_json["clone_url"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            html_url: repo_json["html_url"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            default_branch: repo_json["default_branch"]
                .as_str()
                .unwrap_or("main")
                .to_string(),
            is_fork,
            parent_owner: if is_fork {
                repo_json["parent"]["owner"]["login"]
                    .as_str()
                    .map(String::from)
            } else {
                None
            },
            parent_repo: if is_fork {
                repo_json["parent"]["name"].as_str().map(String::from)
            } else {
                None
            },
        })
    }
}

/// Wait for a fork to be fully provisioned by polling the API.
pub async fn poll_fork_ready(
    auth: &GitHubAuth,
    owner: &str,
    repo: &str,
    max_attempts: u32,
) -> Result<(), ProviderError> {
    let token = auth
        .get_token()
        .map_err(|_| ProviderError::AuthRequired)?;
    let client = build_github_client(&token)
        .map_err(|e| ProviderError::AuthFailed(e.to_string()))?;

    for attempt in 1..=max_attempts {
        let url = format!("https://api.github.com/repos/{owner}/{repo}");
        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| ProviderError::Network(e.to_string()))?;

        if resp.status().is_success() {
            log::info!("Fork {owner}/{repo} is ready (attempt {attempt})");
            return Ok(());
        }

        if attempt < max_attempts {
            log::info!(
                "Fork not ready yet (attempt {attempt}/{max_attempts}), waiting..."
            );
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }

    Err(ProviderError::ForkInProgress)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_from_template_request_body() {
        let params = CreateFromTemplateParams {
            owner: "testuser".to_string(),
            repo_name: "my-site".to_string(),
            visibility: RepoVisibility::Public,
            description: Some("My site".to_string()),
            template_owner: "wiki3-ai".to_string(),
            template_repo: "wiki3-ai-template".to_string(),
        };

        let mut body = serde_json::json!({
            "owner": params.owner,
            "name": params.repo_name,
            "private": params.visibility == RepoVisibility::Private,
        });
        if let Some(desc) = &params.description {
            body["description"] = serde_json::Value::String(desc.clone());
        }

        assert_eq!(body["owner"], "testuser");
        assert_eq!(body["name"], "my-site");
        assert_eq!(body["private"], false);
        assert_eq!(body["description"], "My site");
    }

    #[test]
    fn test_fork_request_body() {
        let params = ForkRepoParams {
            source_owner: "wiki3-ai".to_string(),
            source_repo: "wiki3-ai-site".to_string(),
            target_owner: Some("myorg".to_string()),
            fork_name: Some("my-fork".to_string()),
        };

        let mut body = serde_json::json!({});
        if let Some(org) = &params.target_owner {
            body["organization"] = serde_json::Value::String(org.clone());
        }
        if let Some(name) = &params.fork_name {
            body["name"] = serde_json::Value::String(name.clone());
        }

        assert_eq!(body["organization"], "myorg");
        assert_eq!(body["name"], "my-fork");
    }

    #[test]
    fn test_api_url_construction() {
        let template_owner = "wiki3-ai";
        let template_repo = "wiki3-ai-template";
        let url = format!(
            "https://api.github.com/repos/{template_owner}/{template_repo}/generate"
        );
        assert_eq!(
            url,
            "https://api.github.com/repos/wiki3-ai/wiki3-ai-template/generate"
        );

        let source_owner = "wiki3-ai";
        let source_repo = "wiki3-ai-site";
        let fork_url = format!(
            "https://api.github.com/repos/{source_owner}/{source_repo}/forks"
        );
        assert_eq!(
            fork_url,
            "https://api.github.com/repos/wiki3-ai/wiki3-ai-site/forks"
        );
    }
}
