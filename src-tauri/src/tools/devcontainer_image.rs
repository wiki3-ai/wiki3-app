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
            .arg(&dockerfile_abs);

        // Forward HTTP(S)_PROXY / NO_PROXY from the host environment
        // as buildkit "proxy build args". Buildkit predeclares these
        // names so they reach RUN steps without requiring ARG lines
        // in the Dockerfile, and they are stripped from the recorded
        // image config so they don't bake into the layer metadata.
        //
        // localhost on the host is reachable from inside Apple
        // Container builds as `host.docker.internal` (registered by
        // devcontainer-core), so a Squid at `127.0.0.1:3128` on the
        // Mac can be reached by setting:
        //
        //     HTTPS_PROXY=http://host.docker.internal:3128
        for (build_arg_name, env_names) in [
            ("HTTP_PROXY", ["HTTP_PROXY", "http_proxy"]),
            ("HTTPS_PROXY", ["HTTPS_PROXY", "https_proxy"]),
            ("NO_PROXY", ["NO_PROXY", "no_proxy"]),
            ("FTP_PROXY", ["FTP_PROXY", "ftp_proxy"]),
            ("ALL_PROXY", ["ALL_PROXY", "all_proxy"]),
        ] {
            if let Some(value) = env_names
                .iter()
                .find_map(|n| std::env::var(n).ok().filter(|v| !v.is_empty()))
            {
                // Rewrite `localhost`/`127.0.0.1` to
                // `host.docker.internal` so values like Squid on
                // localhost still work from inside the build.
                let value = rewrite_localhost_to_host_internal(&value);
                cmd.arg("--build-arg")
                    .arg(format!("{build_arg_name}={value}"));
            }
        }

        cmd.arg(&context_abs);

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

/// Rewrite `localhost` / `127.0.0.1` / `::1` host components in a
/// proxy URL to `host.docker.internal` so the URL works from inside
/// a build container (where `localhost` is the build container
/// itself, not the developer's Mac).
fn rewrite_localhost_to_host_internal(url: &str) -> String {
    // String-level rewrite is sufficient: proxy URLs are simple and
    // pulling in a URL crate just for this would be overkill. We
    // only touch host literals that appear immediately after `://`
    // or `@` and stop at the next `:`/`/` so paths and ports are
    // preserved verbatim.
    let mut out = String::with_capacity(url.len() + 16);
    let bytes = url.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Look for `://` or `@` boundary.
        let host_start = if bytes[i..].starts_with(b"://") {
            out.push_str("://");
            i + 3
        } else if bytes[i] == b'@' {
            out.push('@');
            i + 1
        } else {
            out.push(bytes[i] as char);
            i += 1;
            continue;
        };
        // Determine end of host literal (stop at `:` `/` `?` `#`).
        let mut host_end = host_start;
        while host_end < bytes.len()
            && !matches!(bytes[host_end], b':' | b'/' | b'?' | b'#')
        {
            host_end += 1;
        }
        let host = &url[host_start..host_end];
        let rewritten = match host.to_ascii_lowercase().as_str() {
            "localhost" | "127.0.0.1" | "[::1]" | "::1" => "host.docker.internal",
            _ => host,
        };
        out.push_str(rewritten);
        i = host_end;
    }
    out
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

#[cfg(test)]
mod tests {
    use super::rewrite_localhost_to_host_internal as r;

    #[test]
    fn rewrites_localhost_with_port() {
        assert_eq!(
            r("http://localhost:3128"),
            "http://host.docker.internal:3128"
        );
    }

    #[test]
    fn rewrites_127_0_0_1() {
        assert_eq!(
            r("http://127.0.0.1:3128/"),
            "http://host.docker.internal:3128/"
        );
    }

    #[test]
    fn rewrites_userinfo_host() {
        assert_eq!(
            r("http://user:pass@localhost:3128"),
            "http://user:pass@host.docker.internal:3128"
        );
    }

    #[test]
    fn leaves_other_hosts_alone() {
        assert_eq!(
            r("http://proxy.corp:8080"),
            "http://proxy.corp:8080"
        );
    }

    #[test]
    fn handles_no_port() {
        assert_eq!(
            r("http://localhost"),
            "http://host.docker.internal"
        );
    }
}

