//! Shared helpers for driving Apple Container from the devcontainer
//! config. Used by both the one-shot build and the long-running
//! serve+watch flow.

use std::path::{Path, PathBuf};

use tauri::AppHandle;

use super::devcontainer_config::{self, DevcontainerConfig};
use crate::wiki::log_stream;

/// Result of resolving a devcontainer into a runnable image.
pub struct ResolvedImage {
    /// Tag or reference usable with `container run`.
    pub image_ref: String,
    /// The parsed devcontainer config, so callers can read
    /// `customizations`, `remoteUser`, etc.
    pub config: DevcontainerConfig,
    /// Directory containing `devcontainer.json`. Build contexts in
    /// the config are resolved relative to this.
    pub cfg_dir: PathBuf,
    /// Short name derived from the workspace directory — used for
    /// container names and image tags.
    pub workspace_name: String,
}

/// Discover `.devcontainer/devcontainer.json`, parse it, and make
/// sure the referenced image is available (building it on the fly
/// from any `build.dockerfile` stanza).
///
/// `app` and `wiki_id` are used to stream `container build` output
/// to the frontend log pane in real time. Without that, a slow
/// first-run pull of the FROM image looks like a hang to the user.
pub async fn ensure_devcontainer_image(
    container_bin: &Path,
    workspace: &Path,
    app: &AppHandle,
    wiki_id: Option<&str>,
) -> Result<ResolvedImage, String> {
    let configs = devcontainer_config::find_devcontainer_configs(workspace);
    let cfg_path = configs
        .first()
        .ok_or_else(|| {
            "No devcontainer.json found under .devcontainer/ — cannot \
             run in a sandbox."
                .to_string()
        })?
        .clone();
    let config = devcontainer_config::load_config(&cfg_path)
        .map_err(|e| format!("Failed to parse {}: {e}", cfg_path.display()))?;

    let cfg_dir = cfg_path
        .parent()
        .unwrap_or(workspace)
        .to_path_buf();

    let workspace_name = workspace
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("wiki")
        .to_string();

    let image_ref = if let Some(img) = config.image.as_deref() {
        img.to_string()
    } else if let Some(build) = config.build.as_ref() {
        let dockerfile = build.dockerfile.as_deref().unwrap_or("Dockerfile");
        let context_rel = build.context.as_deref().unwrap_or(".");
        let context_abs = cfg_dir.join(context_rel);
        let dockerfile_abs = cfg_dir.join(dockerfile);
        let tag = format!("wiki3-build-{}:latest", sanitize_tag(&workspace_name));

        log_stream::emit_info(
            app,
            wiki_id,
            "build",
            format!(
                "container build --tag {tag} --file {} {} (this includes pulling the FROM image on first run; expect several minutes for large bases)",
                dockerfile_abs.display(),
                context_abs.display(),
            ),
        );

        // `--progress plain` forces line-buffered text output so we
        // can stream it. With the default `auto` mode buildkit
        // suppresses output until completion when stdout isn't a
        // tty, which makes the UI look frozen during long pulls.
        let mut cmd = tokio::process::Command::new(container_bin);
        cmd.arg("build")
            .arg("--progress")
            .arg("plain")
            .arg("--tag")
            .arg(&tag)
            .arg("--file")
            .arg(&dockerfile_abs)
            .arg(&context_abs);

        let (status, stdout_tail, stderr_tail) =
            log_stream::run_and_stream(app, wiki_id, "build", cmd).await?;

        if !status.success() {
            return Err(format!(
                "container build failed (exit {:?}):\n--- stderr ---\n{}\n--- stdout ---\n{}",
                status.code(),
                stderr_tail,
                stdout_tail,
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

    Ok(ResolvedImage {
        image_ref,
        config,
        cfg_dir,
        workspace_name,
    })
}

/// Sanitize a string for use as an OCI image tag fragment or
/// container name.
pub fn sanitize_tag(s: &str) -> String {
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
