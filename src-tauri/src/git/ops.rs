//! Git operations — shell-based wrapper around the `git` CLI.
//!
//! Shells out to git for maximum compatibility and reliability.
//! All credential-bearing URLs are only used transiently in command args
//! and are never persisted to config files.

use std::path::Path;

use crate::workspace::types::*;

/// Run an arbitrary command in a directory, returning stdout.
pub async fn run_command_in_dir(dir: &str, cmd: &str, args: &[&str]) -> Result<String, GitError> {
    let output = tokio::process::Command::new(cmd)
        .args(args)
        .current_dir(dir)
        .output()
        .await
        .map_err(|e| GitError::CommandFailed(format!("Failed to run {cmd}: {e}")))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Err(GitError::CommandFailed(format!(
            "{cmd} failed (exit {}): {stderr}\n{stdout}",
            output.status.code().unwrap_or(-1)
        )))
    }
}

/// Clone a repository.
pub async fn clone(url: &str, dest: &str) -> Result<(), GitError> {
    let parent = Path::new(dest)
        .parent()
        .ok_or_else(|| GitError::InvalidPath(dest.to_string()))?;
    std::fs::create_dir_all(parent)
        .map_err(|e| GitError::InvalidPath(format!("Cannot create parent dir: {e}")))?;

    let output = tokio::process::Command::new("git")
        .args(["clone", url, dest])
        .output()
        .await
        .map_err(|e| GitError::CommandFailed(format!("git clone failed: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GitError::CommandFailed(format!(
            "git clone failed: {stderr}"
        )));
    }
    Ok(())
}

/// Clone a repository with token-authenticated URL. Token is used only for
/// the clone command and is not persisted in any config.
pub async fn clone_authenticated(url: &str, dest: &str, token: &str) -> Result<(), GitError> {
    let auth_url = inject_token(url, token);
    clone(&auth_url, dest).await?;

    // After cloning, reset the remote URL to the non-authenticated version
    // so the token is not stored in .git/config.
    set_remote_url(dest, "origin", url).await?;
    Ok(())
}

/// Get the current branch name.
pub async fn current_branch(repo_path: &str) -> Result<String, GitError> {
    run_command_in_dir(repo_path, "git", &["rev-parse", "--abbrev-ref", "HEAD"]).await
}

/// Get repository status.
pub async fn status(repo_path: &str) -> Result<GitStatus, GitError> {
    let branch = current_branch(repo_path)
        .await
        .unwrap_or_else(|_| "main".to_string());

    // Get porcelain status
    let porcelain = run_command_in_dir(repo_path, "git", &["status", "--porcelain=v1"])
        .await
        .unwrap_or_default();

    let dirty_files: Vec<DirtyFile> = porcelain
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let status_code = &line[..2];
            let path = line[3..].to_string();
            let status = match status_code.trim() {
                "M" | "MM" => FileStatus::Modified,
                "A" | "AM" => FileStatus::Added,
                "D" => FileStatus::Deleted,
                "R" | "RM" => FileStatus::Renamed,
                "??" => FileStatus::Untracked,
                _ => FileStatus::Modified,
            };
            DirtyFile { path, status }
        })
        .collect();

    // Get ahead/behind counts
    let (ahead, behind) = get_ahead_behind(repo_path, &branch).await;

    // Get last commit info
    let last_commit = get_last_commit(repo_path).await.ok();

    Ok(GitStatus {
        branch,
        dirty_files,
        ahead,
        behind,
        last_commit,
    })
}

/// Stage files.
pub async fn add(repo_path: &str, paths: &[&str]) -> Result<(), GitError> {
    let mut args = vec!["add"];
    args.extend(paths);
    run_command_in_dir(repo_path, "git", &args).await?;
    Ok(())
}

/// Stage all changes.
pub async fn add_all(repo_path: &str) -> Result<(), GitError> {
    run_command_in_dir(repo_path, "git", &["add", "-A"]).await?;
    Ok(())
}

/// Create a commit.
pub async fn commit(repo_path: &str, message: &str) -> Result<CommitInfo, GitError> {
    // Ensure user config exists for the repo
    ensure_git_user_config(repo_path).await?;

    run_command_in_dir(repo_path, "git", &["commit", "-m", message]).await?;
    get_last_commit(repo_path).await
}

/// Push to a remote.
pub async fn push(repo_path: &str, remote: &str, branch: &str) -> Result<PushResult, GitError> {
    let result = run_command_in_dir(repo_path, "git", &["push", remote, branch]).await;
    match result {
        Ok(output) => Ok(PushResult {
            success: true,
            remote: remote.to_string(),
            branch: branch.to_string(),
            message: if output.is_empty() {
                "Push successful".to_string()
            } else {
                output
            },
        }),
        Err(e) => Ok(PushResult {
            success: false,
            remote: remote.to_string(),
            branch: branch.to_string(),
            message: e.to_string(),
        }),
    }
}

/// Push with token authentication. Token is injected into the remote URL
/// only for this single push command and is not persisted.
pub async fn push_authenticated(
    repo_path: &str,
    remote: &str,
    branch: &str,
    token: &str,
) -> Result<PushResult, GitError> {
    // Get the remote URL, inject token, push to that URL directly
    let remote_url = get_remote_url(repo_path, remote).await?;
    let auth_url = inject_token(&remote_url, token);

    let result = run_command_in_dir(repo_path, "git", &["push", &auth_url, branch]).await;
    match result {
        Ok(output) => Ok(PushResult {
            success: true,
            remote: remote.to_string(),
            branch: branch.to_string(),
            message: if output.is_empty() {
                "Push successful".to_string()
            } else {
                output
            },
        }),
        Err(e) => Ok(PushResult {
            success: false,
            remote: remote.to_string(),
            branch: branch.to_string(),
            message: e.to_string(),
        }),
    }
}

/// Fetch from a remote.
pub async fn fetch(repo_path: &str, remote: &str) -> Result<(), GitError> {
    run_command_in_dir(repo_path, "git", &["fetch", remote]).await?;
    Ok(())
}

/// Pull from a remote.
pub async fn pull(repo_path: &str, remote: &str, branch: &str) -> Result<String, GitError> {
    run_command_in_dir(repo_path, "git", &["pull", remote, branch]).await
}

/// Add a remote.
pub async fn add_remote(repo_path: &str, name: &str, url: &str) -> Result<(), GitError> {
    run_command_in_dir(repo_path, "git", &["remote", "add", name, url]).await?;
    Ok(())
}

/// Remove a remote.
pub async fn remove_remote(repo_path: &str, name: &str) -> Result<(), GitError> {
    run_command_in_dir(repo_path, "git", &["remote", "remove", name]).await?;
    Ok(())
}

/// Set a remote URL.
pub async fn set_remote_url(repo_path: &str, name: &str, url: &str) -> Result<(), GitError> {
    run_command_in_dir(repo_path, "git", &["remote", "set-url", name, url]).await?;
    Ok(())
}

/// Get a remote URL.
pub async fn get_remote_url(repo_path: &str, name: &str) -> Result<String, GitError> {
    run_command_in_dir(repo_path, "git", &["remote", "get-url", name]).await
}

/// List remotes.
pub async fn list_remotes(repo_path: &str) -> Result<Vec<RemoteInfo>, GitError> {
    let output = run_command_in_dir(repo_path, "git", &["remote", "-v"]).await?;
    let mut remotes = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for line in output.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            let name = parts[0].to_string();
            if seen.insert(name.clone()) {
                remotes.push(RemoteInfo {
                    name,
                    url: parts[1].to_string(),
                });
            }
        }
    }
    Ok(remotes)
}

/// Initialize a new git repo if not already initialized.
pub async fn init_if_needed(repo_path: &str) -> Result<(), GitError> {
    let git_dir = Path::new(repo_path).join(".git");
    if !git_dir.exists() {
        run_command_in_dir(repo_path, "git", &["init"]).await?;
    }
    Ok(())
}

/// Check if the path is inside a git repo.
pub async fn is_git_repo(path: &str) -> bool {
    run_command_in_dir(path, "git", &["rev-parse", "--git-dir"])
        .await
        .is_ok()
}

// -- Private helpers --

async fn get_ahead_behind(repo_path: &str, branch: &str) -> (u32, u32) {
    let result = run_command_in_dir(
        repo_path,
        "git",
        &[
            "rev-list",
            "--left-right",
            "--count",
            &format!("{branch}...origin/{branch}"),
        ],
    )
    .await;

    match result {
        Ok(output) => {
            let parts: Vec<&str> = output.split_whitespace().collect();
            if parts.len() == 2 {
                let ahead = parts[0].parse().unwrap_or(0);
                let behind = parts[1].parse().unwrap_or(0);
                (ahead, behind)
            } else {
                (0, 0)
            }
        }
        Err(_) => (0, 0),
    }
}

async fn get_last_commit(repo_path: &str) -> Result<CommitInfo, GitError> {
    let sha = run_command_in_dir(repo_path, "git", &["rev-parse", "--short", "HEAD"]).await?;
    let message = run_command_in_dir(repo_path, "git", &["log", "-1", "--format=%s"]).await?;
    let author = run_command_in_dir(repo_path, "git", &["log", "-1", "--format=%an"]).await?;
    let date = run_command_in_dir(repo_path, "git", &["log", "-1", "--format=%ai"]).await?;

    Ok(CommitInfo {
        sha,
        message,
        author,
        date,
    })
}

async fn ensure_git_user_config(repo_path: &str) -> Result<(), GitError> {
    // Check if user.name is set
    let name_result = run_command_in_dir(repo_path, "git", &["config", "user.name"]).await;
    if name_result.is_err() {
        run_command_in_dir(repo_path, "git", &["config", "user.name", "Wiki3 User"]).await?;
    }

    let email_result = run_command_in_dir(repo_path, "git", &["config", "user.email"]).await;
    if email_result.is_err() {
        run_command_in_dir(repo_path, "git", &["config", "user.email", "user@wiki3.ai"]).await?;
    }

    Ok(())
}

/// Inject a token into a GitHub HTTPS URL for authenticated operations.
/// The token is only used for the single operation and never persisted.
fn inject_token(url: &str, token: &str) -> String {
    if url.starts_with("https://github.com/") {
        let rest = url.strip_prefix("https://").unwrap_or(url);
        format!("https://x-access-token:{token}@{rest}")
    } else {
        url.to_string()
    }
}

/// Git operation errors.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("Git command failed: {0}")]
    CommandFailed(String),
    #[error("Invalid path: {0}")]
    InvalidPath(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inject_token() {
        let url = "https://github.com/user/repo.git";
        let result = inject_token(url, "ghp_abc123");
        assert_eq!(
            result,
            "https://x-access-token:ghp_abc123@github.com/user/repo.git"
        );
    }

    #[test]
    fn test_inject_token_ssh() {
        let url = "git@github.com:user/repo.git";
        let result = inject_token(url, "ghp_abc123");
        assert_eq!(result, "git@github.com:user/repo.git");
    }

    #[tokio::test]
    async fn test_init_and_status() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().to_string();

        // Init a new repo
        init_if_needed(&path).await.unwrap();
        assert!(is_git_repo(&path).await);

        // Configure git user for the test
        ensure_git_user_config(&path).await.unwrap();

        // Create a file
        std::fs::write(dir.path().join("test.txt"), "hello").unwrap();

        let status = status(&path).await.unwrap();
        assert_eq!(status.dirty_files.len(), 1);
        assert_eq!(status.dirty_files[0].path, "test.txt");
        assert_eq!(status.dirty_files[0].status, FileStatus::Untracked);
    }

    #[tokio::test]
    async fn test_add_and_commit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().to_string();

        init_if_needed(&path).await.unwrap();
        ensure_git_user_config(&path).await.unwrap();

        std::fs::write(dir.path().join("test.txt"), "hello").unwrap();
        add_all(&path).await.unwrap();
        let info = commit(&path, "Initial commit").await.unwrap();
        assert_eq!(info.message, "Initial commit");
        assert!(!info.sha.is_empty());

        // Should be clean now
        let s = status(&path).await.unwrap();
        assert!(s.dirty_files.is_empty());
    }

    #[tokio::test]
    async fn test_list_remotes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().to_string();

        init_if_needed(&path).await.unwrap();
        add_remote(&path, "origin", "https://github.com/user/repo.git")
            .await
            .unwrap();

        let remotes = list_remotes(&path).await.unwrap();
        assert_eq!(remotes.len(), 1);
        assert_eq!(remotes[0].name, "origin");
        assert_eq!(remotes[0].url, "https://github.com/user/repo.git");
    }
}
