//! GitHub authentication — secure token storage and API client.
//!
//! Uses the OS keychain (macOS Keychain, Linux Secret Service) for token
//! storage. Falls back to file-based storage in the app data directory
//! when the system keychain is not available (e.g. CI environments).

use keyring::Entry;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, USER_AGENT};

const SERVICE_NAME: &str = "wiki3-app";
const KEYRING_USER: &str = "github-token";
const FALLBACK_FILE: &str = "github_auth.json";

/// Manages GitHub authentication credentials.
pub struct GitHubAuth {
    /// App data directory for fallback storage.
    data_dir: std::path::PathBuf,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct FallbackToken {
    token: String,
}

impl GitHubAuth {
    pub fn new(data_dir: std::path::PathBuf) -> Self {
        Self { data_dir }
    }

    /// Store a GitHub personal access token securely.
    pub fn store_token(&self, token: &str) -> Result<(), AuthError> {
        // Try keychain first
        match Entry::new(SERVICE_NAME, KEYRING_USER) {
            Ok(entry) => match entry.set_password(token) {
                Ok(()) => {
                    log::info!("Token stored in system keychain");
                    // Remove fallback file if it exists
                    let fallback = self.data_dir.join(FALLBACK_FILE);
                    let _ = std::fs::remove_file(fallback);
                    return Ok(());
                }
                Err(e) => {
                    log::warn!("Keychain store failed, using fallback: {e}");
                }
            },
            Err(e) => {
                log::warn!("Keychain not available, using fallback: {e}");
            }
        }

        // Fallback to file-based storage
        self.store_token_file(token)
    }

    /// Retrieve the stored GitHub token.
    pub fn get_token(&self) -> Result<String, AuthError> {
        // Try keychain first
        if let Ok(entry) = Entry::new(SERVICE_NAME, KEYRING_USER) {
            if let Ok(token) = entry.get_password() {
                if !token.is_empty() {
                    return Ok(token);
                }
            }
        }

        // Fallback to file
        self.get_token_file()
    }

    /// Remove the stored token.
    pub fn clear_token(&self) -> Result<(), AuthError> {
        // Try keychain
        if let Ok(entry) = Entry::new(SERVICE_NAME, KEYRING_USER) {
            let _ = entry.delete_credential();
        }
        // Remove fallback file
        let fallback = self.data_dir.join(FALLBACK_FILE);
        let _ = std::fs::remove_file(fallback);
        Ok(())
    }

    /// Check if a token is stored (without revealing it).
    pub fn has_token(&self) -> bool {
        self.get_token().is_ok()
    }

    /// Validate the stored token by calling the GitHub API.
    pub async fn validate_token(&self) -> Result<GitHubUser, AuthError> {
        let token = self.get_token()?;
        let client = build_github_client(&token)?;
        let resp = client
            .get("https://api.github.com/user")
            .send()
            .await
            .map_err(|e| AuthError::Network(e.to_string()))?;

        if resp.status() == 401 {
            return Err(AuthError::TokenExpired);
        }
        if !resp.status().is_success() {
            return Err(AuthError::Api(format!("HTTP {}", resp.status())));
        }

        let user: GitHubUser = resp
            .json()
            .await
            .map_err(|e| AuthError::Api(e.to_string()))?;
        Ok(user)
    }

    // -- Private helpers --

    fn store_token_file(&self, token: &str) -> Result<(), AuthError> {
        std::fs::create_dir_all(&self.data_dir)
            .map_err(|e| AuthError::Storage(e.to_string()))?;
        let path = self.data_dir.join(FALLBACK_FILE);
        let data = FallbackToken {
            token: token.to_string(),
        };
        let json = serde_json::to_string(&data)
            .map_err(|e| AuthError::Storage(e.to_string()))?;
        std::fs::write(&path, json).map_err(|e| AuthError::Storage(e.to_string()))?;

        // Restrict file permissions on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(&path, perms)
                .map_err(|e| AuthError::Storage(e.to_string()))?;
        }

        Ok(())
    }

    fn get_token_file(&self) -> Result<String, AuthError> {
        let path = self.data_dir.join(FALLBACK_FILE);
        if !path.exists() {
            return Err(AuthError::NoToken);
        }
        let data = std::fs::read_to_string(&path)
            .map_err(|e| AuthError::Storage(e.to_string()))?;
        let fb: FallbackToken = serde_json::from_str(&data)
            .map_err(|e| AuthError::Storage(e.to_string()))?;
        if fb.token.is_empty() {
            return Err(AuthError::NoToken);
        }
        Ok(fb.token)
    }
}

/// Build an authenticated reqwest client for GitHub API calls.
pub fn build_github_client(token: &str) -> Result<reqwest::Client, AuthError> {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|e| AuthError::Api(e.to_string()))?,
    );
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/vnd.github+json"),
    );
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("wiki3-app/0.1"),
    );
    headers.insert(
        "X-GitHub-Api-Version",
        HeaderValue::from_static("2022-11-28"),
    );

    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .map_err(|e| AuthError::Api(e.to_string()))
}

/// Authenticated GitHub user info.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GitHubUser {
    pub login: String,
    pub id: u64,
    pub avatar_url: Option<String>,
    pub name: Option<String>,
}

/// Authentication errors.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("No token stored — please authenticate first")]
    NoToken,
    #[error("Token has expired or been revoked")]
    TokenExpired,
    #[error("Insufficient token scopes: {0}")]
    InsufficientScopes(String),
    #[error("Storage error: {0}")]
    Storage(String),
    #[error("Network error: {0}")]
    Network(String),
    #[error("API error: {0}")]
    Api(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fallback_token_storage() {
        let dir = tempfile::tempdir().unwrap();
        let auth = GitHubAuth::new(dir.path().to_path_buf());

        // No token initially
        assert!(!auth.has_token());
        assert!(matches!(auth.get_token_file(), Err(AuthError::NoToken)));

        // Store and retrieve
        auth.store_token_file("ghp_test123").unwrap();
        assert_eq!(auth.get_token_file().unwrap(), "ghp_test123");

        // Overwrite
        auth.store_token_file("ghp_new456").unwrap();
        assert_eq!(auth.get_token_file().unwrap(), "ghp_new456");
    }

    #[test]
    fn test_build_github_client() {
        let client = build_github_client("ghp_test123");
        assert!(client.is_ok());
    }
}
