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

/// Build a wiki's static site.
///
/// If the repo has a `.devcontainer/` directory, the build runs
/// inside a Wiki3-managed sandbox: the pinned Deno is ensured,
/// `@devcontainers/cli` is invoked under it (`devcontainer up` then
/// `devcontainer exec jupyter lite build`), with `DOCKER_HOST`
/// pointed at Apple Container's user socket. The user never installs
/// Node, npm, Homebrew, or the devcontainer CLI themselves — Wiki3
/// manages those under `<app_data>/tools/`.
///
/// If the repo has no `.devcontainer/`, the legacy path runs
/// `jupyter lite build` directly on the host, for backward
/// compatibility with existing wikis.
#[command]
pub async fn wiki_build_site(
    app: AppHandle,
    wiki_id: String,
) -> Result<serde_json::Value, String> {
    use tokio::process::Command;

    let wiki = get_wiki(&app, &wiki_id)?;
    let path = require_local(&wiki)?;
    let path_ref = std::path::Path::new(&path);
    if !path_ref.exists() {
        return Err(format!("Local path does not exist: {path}"));
    }

    // Prefer the sandboxed path when the repo declares a devcontainer.
    if path_ref.join(".devcontainer").exists()
        || path_ref.join(".devcontainer.json").exists()
    {
        return build_site_in_devcontainer(&app, &path).await;
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
        "mode": "host",
        "output_dir": std::path::Path::new(&path).join("_output").to_string_lossy(),
        "stdout": stdout,
        "stderr": stderr,
    }))
}

/// Build via `deno run … npm:@devcontainers/cli …`.
///
/// Deno ships inside the Wiki3 app bundle (see `build.rs` and
/// `tools::runner::require_bundled_deno`) — no install step is
/// involved. Apple Container is the only runtime prerequisite.
///
/// 1. Verify Apple Container is installed (otherwise return a
///    structured error so the UI can launch the signed `.pkg`).
/// 2. Run `devcontainer up` to bring the sandbox up.
/// 3. Run `devcontainer exec jupyter lite build` inside it.
async fn build_site_in_devcontainer(
    app: &AppHandle,
    workspace: &str,
) -> Result<serde_json::Value, String> {
    use crate::tools::{apple_container, runner, ToolsState};

    // Gate: Apple Container is required. The UI will turn this into
    // the "macOS will now ask to install Apple Container" modal.
    let ac = apple_container::detect();
    if !ac.installed {
        return Err(
            "Apple Container is not installed. Open the Tools dialog to run the \
             detection probe and install it before building in a sandbox."
                .to_string(),
        );
    }
    let container_bin = ac.path.clone().ok_or_else(|| {
        "Apple Container detection succeeded but returned no path".to_string()
    })?;

    // Ensure the service (and its UNIX socket) is up. Idempotent —
    // `container system start` is a no-op if already running. On a
    // fresh reboot this is what wakes the socket `devcontainer up`
    // talks to via `DOCKER_HOST`.
    apple_container::ensure_service_running(&container_bin)
        .await
        .map_err(|e| format!("Could not start Apple Container service: {e}"))?;

    let state = app.state::<ToolsState>();
    let docker_host = runner::apple_container_docker_host();

    // devcontainer up
    let up = runner::run_devcontainer(
        &state,
        "up",
        &["--workspace-folder", workspace],
        std::path::Path::new(workspace),
        docker_host.as_deref(),
    )
    .await?;
    if !up.success {
        return Err(format!(
            "devcontainer up failed (exit {:?}):\n--- stderr ---\n{}\n--- stdout ---\n{}",
            up.status, up.stderr, up.stdout,
        ));
    }

    // devcontainer exec jupyter lite build
    let exec = runner::run_devcontainer(
        &state,
        "exec",
        &[
            "--workspace-folder",
            workspace,
            "jupyter",
            "lite",
            "build",
        ],
        std::path::Path::new(workspace),
        docker_host.as_deref(),
    )
    .await?;

    if !exec.success {
        return Err(format!(
            "jupyter lite build (in devcontainer) failed (exit {:?}):\n--- stderr ---\n{}\n--- stdout ---\n{}",
            exec.status, exec.stderr, exec.stdout,
        ));
    }

    Ok(serde_json::json!({
        "success": true,
        "mode": "devcontainer",
        "deno_path": exec.deno_path.to_string_lossy(),
        "apple_container_path": ac.path.map(|p| p.to_string_lossy().to_string()),
        "output_dir": std::path::Path::new(workspace).join("_output").to_string_lossy(),
        "stdout": exec.stdout,
        "stderr": exec.stderr,
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
