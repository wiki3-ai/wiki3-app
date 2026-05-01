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

/// Build a wiki's static site.
///
/// If the repo has a `.devcontainer/` directory, the build runs
/// inside Apple Container: we parse `devcontainer.json` in-process
/// (via an embedded QuickJS module — see
/// `tools::devcontainer_config`), build the image if needed, and
/// invoke `container run` with the workspace bind-mounted. No
/// `docker` CLI, no Node/Deno, no third-party devcontainer CLI.
///
/// If the repo has no `.devcontainer/`, the legacy path runs
/// `jupyter lite build` directly on the host, for backward
/// compatibility with existing wikis.
#[command]
pub async fn wiki_build_site(app: AppHandle, wiki_id: String) -> Result<serde_json::Value, String> {
    use tokio::process::Command;

    let wiki = get_wiki(&app, &wiki_id)?;
    let path = require_local(&wiki)?;
    let path_ref = std::path::Path::new(&path);
    if !path_ref.exists() {
        return Err(format!("Local path does not exist: {path}"));
    }

    // Prefer the sandboxed path when the repo declares a devcontainer.
    if path_ref.join(".devcontainer").exists() || path_ref.join(".devcontainer.json").exists() {
        return build_site_in_devcontainer(&app, &wiki_id, &path).await;
    }

    crate::wiki::log_stream::emit_info(
        &app,
        Some(&wiki_id),
        "build",
        format!("host build: jupyter lite build in {path}"),
    );
    let mut cmd = Command::new("jupyter");
    cmd.args(["lite", "build"]).current_dir(&path);
    let (status, stdout, stderr) =
        crate::wiki::log_stream::run_and_stream(&app, Some(&wiki_id), "build", cmd)
            .await
            .map_err(|e| {
                format!(
                    "Failed to run `jupyter lite build`: {e}. \
                     Make sure `jupyter` and `jupyterlite-core` are installed and on PATH \
                     (e.g. `pip install jupyterlite-core`)."
                )
            })?;

    if !status.success() {
        return Err(format!(
            "jupyter lite build failed:\n{}",
            if stderr.trim().is_empty() {
                &stdout
            } else {
                &stderr
            }
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

/// Build the site inside Apple Container.
///
/// 1. Verify Apple Container is installed.
/// 2. Ensure `container system start` has been run (idempotent).
/// 3. Parse `.devcontainer/devcontainer.json` in-process via QuickJS.
/// 4. If the config specifies a `build` (Dockerfile), run
///    `container build` to produce a tagged image.
/// 5. Run `container run` with the workspace bind-mounted and execute
///    the `postCreateCommand` (or `jupyter lite build` as fallback).
async fn build_site_in_devcontainer(
    app: &AppHandle,
    wiki_id: &str,
    workspace: &str,
) -> Result<serde_json::Value, String> {
    use crate::tools::{apple_container, devcontainer_config};
    use crate::wiki::log_stream;

    let workspace_path = std::path::Path::new(workspace);

    log_stream::emit_info(app, Some(wiki_id), "build", "Detecting Apple Container…");

    // --- 1. Apple Container must be installed ---
    let ac = apple_container::detect();
    if !ac.installed {
        return Err(
            "Apple Container is not installed. Install it from the signed \
             installer at https://github.com/apple/container/releases before \
             building in a sandbox."
                .to_string(),
        );
    }
    let container_bin = ac
        .path
        .clone()
        .ok_or_else(|| "Apple Container detection succeeded but returned no path".to_string())?;

    log_stream::emit_info(
        app,
        Some(wiki_id),
        "build",
        format!("Using container binary: {}", container_bin.display()),
    );
    log_stream::emit_info(
        app,
        Some(wiki_id),
        "build",
        "Ensuring container service is running (may take a while on first run)…",
    );

    // --- 2. Ensure the service (and its socket) are up ---
    apple_container::ensure_service_running(&container_bin)
        .await
        .map_err(|e| format!("Could not start Apple Container service: {e}"))?;

    // --- 3. Discover + parse devcontainer.json ---
    let configs = devcontainer_config::find_devcontainer_configs(workspace_path);
    let cfg_path = configs
        .first()
        .ok_or_else(|| {
            "No devcontainer.json found under .devcontainer/ — cannot \
             build in a sandbox."
                .to_string()
        })?
        .clone();
    let cfg = devcontainer_config::load_config(&cfg_path)
        .map_err(|e| format!("Failed to parse {}: {e}", cfg_path.display()))?;

    log_stream::emit_info(
        app,
        Some(wiki_id),
        "build",
        format!("Parsed devcontainer: {}", cfg_path.display()),
    );

    // The directory containing devcontainer.json — build paths in the
    // config are relative to this.
    let cfg_dir = cfg_path.parent().unwrap_or(workspace_path).to_path_buf();

    // --- 4. Resolve image: either explicit `image` or build a Dockerfile ---
    let workspace_name = workspace_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("wiki");
    let image_ref = if let Some(img) = cfg.image.as_deref() {
        log_stream::emit_info(
            app,
            Some(wiki_id),
            "build",
            format!("Using pre-built image: {img}"),
        );
        img.to_string()
    } else if let Some(build) = cfg.build.as_ref() {
        let dockerfile = build.dockerfile.as_deref().unwrap_or("Dockerfile");
        let context_rel = build.context.as_deref().unwrap_or(".");
        let context_abs = cfg_dir.join(context_rel);
        let dockerfile_abs = cfg_dir.join(dockerfile);
        let tag = format!("wiki3-build-{}:latest", sanitize_tag(workspace_name));

        log_stream::emit_info(
            app,
            Some(wiki_id),
            "image-build",
            format!("Building image {tag} from {}", dockerfile_abs.display()),
        );

        let mut build_cmd = tokio::process::Command::new(&container_bin);
        build_cmd
            .arg("build")
            .arg("--tag")
            .arg(&tag)
            .arg("--file")
            .arg(&dockerfile_abs)
            .arg(&context_abs);

        let (status, stdout, stderr) =
            log_stream::run_and_stream(app, Some(wiki_id), "image-build", build_cmd).await?;

        if !status.success() {
            return Err(format!(
                "container build failed (exit {:?}):\n--- stderr ---\n{}\n--- stdout ---\n{}",
                status.code(),
                stderr,
                stdout,
            ));
        }
        tag
    } else {
        return Err(format!(
            "devcontainer.json at {} has neither `image` nor `build` — \
             cannot produce a runtime image.",
            cfg_path.display()
        ));
    };

    // --- 5. Build the shell command to run inside the container ---
    let cmd_str = cfg
        .post_create_command
        .as_ref()
        .and_then(value_to_shell_command)
        .unwrap_or_else(|| "jupyter lite build".to_string());

    let mount_arg = format!("{}:/workspaces/{workspace_name}", workspace);
    let workdir = format!("/workspaces/{workspace_name}");

    log_stream::emit_info(
        app,
        Some(wiki_id),
        "build",
        format!("container run --volume {mount_arg} -- bash -lc {cmd_str}"),
    );

    let mut run_cmd = tokio::process::Command::new(&container_bin);
    run_cmd
        .arg("run")
        .arg("--rm")
        .arg("--volume")
        .arg(&mount_arg)
        .arg("--workdir")
        .arg(&workdir);

    if let Some(user) = cfg.remote_user.as_deref() {
        run_cmd.arg("--user").arg(user);
    }

    run_cmd.arg(&image_ref).arg("bash").arg("-lc").arg(&cmd_str);

    let (status, stdout, stderr) =
        log_stream::run_and_stream(app, Some(wiki_id), "build", run_cmd).await?;

    if !status.success() {
        return Err(format!(
            "container run failed (exit {:?}):\n--- stderr ---\n{}\n--- stdout ---\n{}",
            status.code(),
            stderr,
            stdout,
        ));
    }

    log_stream::emit_info(
        app,
        Some(wiki_id),
        "build",
        format!(
            "Built successfully → {}",
            workspace_path.join("_output").display()
        ),
    );

    Ok(serde_json::json!({
        "success": true,
        "mode": "apple-container",
        "apple_container_path": ac.path.map(|p| p.to_string_lossy().to_string()),
        "image": image_ref,
        "command": cmd_str,
        "output_dir": workspace_path.join("_output").to_string_lossy(),
        "stdout": stdout,
        "stderr": stderr,
    }))
}

/// Convert a `postCreateCommand` JSON value into a single shell
/// command string. The devcontainer spec allows string, array, or
/// object forms; we support the two common ones.
pub(crate) fn value_to_shell_command(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(items) => {
            let parts: Vec<String> = items
                .iter()
                .filter_map(|i| i.as_str().map(shell_quote))
                .collect();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join(" "))
            }
        }
        _ => None,
    }
}

/// Minimal POSIX shell quoter — wraps in single quotes, escaping any
/// embedded single quotes.
pub(crate) fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Sanitize a string for use as an OCI image tag fragment.
fn sanitize_tag(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

/// `PublishingState` holds `data_dir` privately; expose a helper that
/// constructs the auth using our local path convention.
fn state_data_dir(state: &tauri::State<'_, PublishingState>) -> std::path::PathBuf {
    // PublishingState stores `data_dir` as a private field. The workspace
    // manager's storage_dir mirrors it — use that to avoid a breaking API
    // change on PublishingState.
    state.workspace_manager.storage_dir().to_path_buf()
}
