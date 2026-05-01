//! Parse and normalise `.devcontainer/**/*.json` configuration files
//! using an embedded QuickJS interpreter (via `rquickjs`).
//!
//! File discovery follows the [devcontainer spec]:
//! 1. `<root>/.devcontainer.json`
//! 2. `<root>/.devcontainer/devcontainer.json`
//! 3. `<root>/.devcontainer/<subdir>/devcontainer.json`
//!
//! Discovery is done in Rust (no JS needed). Schema normalisation and
//! validation are delegated to the embedded JS module so the spec rules
//! live in one readable place and can be tested independently.
//!
//! [devcontainer spec]: https://containers.dev/implementors/spec/#devcontainerjson

use std::fs;
use std::path::{Path, PathBuf};

use rquickjs::{Context, Function, Runtime};
use serde::{Deserialize, Serialize};

use super::{Result, ToolsError};

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// A normalised devcontainer configuration derived from one
/// `devcontainer.json` file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DevcontainerConfig {
    /// Human-readable name for this devcontainer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Pre-built container image reference (e.g. `"ubuntu:22.04"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,

    /// Build instructions when there is no pre-built image.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build: Option<BuildConfig>,

    /// Ports to forward from the container to the host.
    #[serde(
        rename = "forwardPorts",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub forward_ports: Vec<serde_json::Value>,

    /// Per-port attributes (label, onAutoForward, protocol, etc.) keyed
    /// by the port number as a string. Carried through verbatim.
    #[serde(rename = "portsAttributes", skip_serializing_if = "Option::is_none")]
    pub ports_attributes: Option<serde_json::Value>,

    /// Command to run once after the container is created.
    #[serde(rename = "postCreateCommand", skip_serializing_if = "Option::is_none")]
    pub post_create_command: Option<serde_json::Value>,

    /// Command to run each time the container is started. In the
    /// devcontainer spec this is the long-running foreground
    /// process (e.g. a dev server) — for wiki3 sites that's
    /// typically `jupyter lite serve`.
    #[serde(rename = "postStartCommand", skip_serializing_if = "Option::is_none")]
    pub post_start_command: Option<serde_json::Value>,

    /// User to run as inside the container.
    #[serde(rename = "remoteUser", skip_serializing_if = "Option::is_none")]
    pub remote_user: Option<String>,

    /// devcontainer Features to install (OCI ref -> options map).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub features: Option<serde_json::Value>,

    /// VS Code / other tool customizations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub customizations: Option<serde_json::Value>,
}

/// Build configuration nested inside `devcontainer.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuildConfig {
    /// Relative path to the Dockerfile.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dockerfile: Option<String>,

    /// Build context relative to the repo root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,

    /// Build-time `--build-arg` values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<serde_json::Value>,

    /// Multi-stage build target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Locate every `devcontainer.json` under `repo_root` following the
/// devcontainer spec discovery rules.  Results are sorted for
/// deterministic ordering.
pub fn find_devcontainer_configs(repo_root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();

    // Rule 1: .devcontainer.json at the repo root.
    let root_cfg = repo_root.join(".devcontainer.json");
    if root_cfg.is_file() {
        found.push(root_cfg);
    }

    let dc_dir = repo_root.join(".devcontainer");
    if dc_dir.is_dir() {
        // Rule 2: .devcontainer/devcontainer.json
        let top = dc_dir.join("devcontainer.json");
        if top.is_file() {
            found.push(top);
        }

        // Rule 3: .devcontainer/<subdir>/devcontainer.json
        if let Ok(entries) = fs::read_dir(&dc_dir) {
            let mut subdirs: Vec<_> = entries
                .flatten()
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .collect();
            subdirs.sort_by_key(|e| e.file_name());
            for entry in subdirs {
                let sub_cfg = entry.path().join("devcontainer.json");
                if sub_cfg.is_file() {
                    found.push(sub_cfg);
                }
            }
        }
    }

    found
}

// ---------------------------------------------------------------------------
// Config loading
// ---------------------------------------------------------------------------

/// Read and normalise a `devcontainer.json` file.
///
/// File I/O is done here in Rust; normalisation and validation are
/// handled by the embedded QuickJS module.
pub fn load_config(path: &Path) -> Result<DevcontainerConfig> {
    let raw = fs::read_to_string(path)?;
    resolve_config_js(&raw)
}

/// Run the embedded JS `resolveConfig` function on a raw JSON string and
/// deserialise the result into [`DevcontainerConfig`].
fn resolve_config_js(json_str: &str) -> Result<DevcontainerConfig> {
    let rt = Runtime::new().map_err(|e| ToolsError::Script(e.to_string()))?;
    let ctx = Context::full(&rt).map_err(|e| ToolsError::Script(e.to_string()))?;

    let normalised_json: String = ctx
        .with(|ctx| -> rquickjs::Result<String> {
            // Load the embedded module — defines `resolveConfig` in global scope.
            ctx.eval::<(), _>(JS_SOURCE)?;
            let f: Function = ctx.globals().get("resolveConfig")?;
            f.call((json_str.to_string(),))
        })
        .map_err(|e| {
            ToolsError::Script(format!("QuickJS error processing devcontainer.json: {e}"))
        })?;

    serde_json::from_str::<DevcontainerConfig>(&normalised_json)
        .map_err(|e| ToolsError::Script(format!("failed to deserialise resolved config: {e}")))
}

/// Embedded JS source — included verbatim at compile time.
const JS_SOURCE: &str = include_str!("devcontainer_config.js");

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(dir: &Path, rel: &str, content: &str) {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }

    // --- discovery ----------------------------------------------------------

    #[test]
    fn finds_root_devcontainer_json() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), ".devcontainer.json", r#"{"image":"ubuntu"}"#);
        let found = find_devcontainer_configs(tmp.path());
        assert_eq!(found.len(), 1);
        assert!(found[0].file_name().unwrap() == ".devcontainer.json");
    }

    #[test]
    fn finds_dot_devcontainer_dir() {
        let tmp = TempDir::new().unwrap();
        write(
            tmp.path(),
            ".devcontainer/devcontainer.json",
            r#"{"image":"ubuntu"}"#,
        );
        let found = find_devcontainer_configs(tmp.path());
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn finds_subdirectory_configs_sorted() {
        let tmp = TempDir::new().unwrap();
        write(
            tmp.path(),
            ".devcontainer/python/devcontainer.json",
            r#"{"image":"python:3"}"#,
        );
        write(
            tmp.path(),
            ".devcontainer/node/devcontainer.json",
            r#"{"image":"node:20"}"#,
        );
        let found = find_devcontainer_configs(tmp.path());
        assert_eq!(found.len(), 2);
        // Should be sorted: node before python
        assert!(found[0].to_string_lossy().contains("node"));
        assert!(found[1].to_string_lossy().contains("python"));
    }

    #[test]
    fn ignores_non_devcontainer_files() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), ".devcontainer/other.json", "{}");
        write(tmp.path(), "devcontainer.json", "{}");
        assert!(find_devcontainer_configs(tmp.path()).is_empty());
    }

    #[test]
    fn empty_repo_returns_empty() {
        let tmp = TempDir::new().unwrap();
        assert!(find_devcontainer_configs(tmp.path()).is_empty());
    }

    // --- JS resolver --------------------------------------------------------

    #[test]
    fn resolves_image_config() {
        let cfg = resolve_config_js(r#"{"image":"ubuntu:22.04","name":"Test"}"#).unwrap();
        assert_eq!(cfg.image.as_deref(), Some("ubuntu:22.04"));
        assert_eq!(cfg.name.as_deref(), Some("Test"));
        assert!(cfg.build.is_none());
    }

    #[test]
    fn resolves_build_config() {
        let cfg =
            resolve_config_js(r#"{"build":{"dockerfile":"Dockerfile","context":".."}}"#).unwrap();
        assert!(cfg.image.is_none());
        let build = cfg.build.unwrap();
        assert_eq!(build.dockerfile.as_deref(), Some("Dockerfile"));
        assert_eq!(build.context.as_deref(), Some(".."));
    }

    #[test]
    fn normalises_legacy_docker_file_key() {
        // The spec allows the deprecated root-level "dockerFile" key.
        let cfg = resolve_config_js(r#"{"dockerFile":"Dockerfile.dev"}"#).unwrap();
        let build = cfg.build.unwrap();
        assert_eq!(build.dockerfile.as_deref(), Some("Dockerfile.dev"));
        assert!(cfg.image.is_none());
    }

    #[test]
    fn rejects_config_without_image_or_build() {
        assert!(resolve_config_js(r#"{"name":"no-runtime"}"#).is_err());
    }

    #[test]
    fn rejects_invalid_json() {
        assert!(resolve_config_js("not json at all").is_err());
    }

    #[test]
    fn rejects_non_object_root() {
        assert!(resolve_config_js(r#"["image","ubuntu"]"#).is_err());
    }

    #[test]
    fn accepts_jsonc_with_comments_and_trailing_commas() {
        // devcontainer.json is officially JSONC; VS Code and the
        // upstream CLI both tolerate // line comments, /* */ block
        // comments, and trailing commas. We mirror that.
        let src = r#"{
            // line comment
            "image": "ubuntu:22.04", /* block // comment */
            "name": "test", // with a "quoted" snippet
            "forwardPorts": [
                8888,
            ],
        }"#;
        let cfg = resolve_config_js(src).unwrap();
        assert_eq!(cfg.image.as_deref(), Some("ubuntu:22.04"));
        assert_eq!(cfg.name.as_deref(), Some("test"));
        assert_eq!(cfg.forward_ports.len(), 1);
    }

    #[test]
    fn preserves_comment_like_substrings_inside_strings() {
        let cfg = resolve_config_js(
            r#"{"image":"ubuntu","name":"https://example.com/path // not a comment"}"#,
        )
        .unwrap();
        assert_eq!(
            cfg.name.as_deref(),
            Some("https://example.com/path // not a comment")
        );
    }

    #[test]
    fn resolves_forward_ports() {
        let cfg = resolve_config_js(r#"{"image":"img","forwardPorts":[3000,8080]}"#).unwrap();
        assert_eq!(cfg.forward_ports.len(), 2);
    }

    #[test]
    fn resolves_features() {
        let cfg = resolve_config_js(
            r#"{"image":"img","features":{"ghcr.io/devcontainers/features/git:1":{}}}"#,
        )
        .unwrap();
        assert!(cfg.features.is_some());
    }

    #[test]
    fn resolves_post_create_command_string() {
        let cfg =
            resolve_config_js(r#"{"image":"img","postCreateCommand":"npm install"}"#).unwrap();
        let cmd = cfg.post_create_command.unwrap();
        assert_eq!(cmd.as_str(), Some("npm install"));
    }

    #[test]
    fn resolves_remote_user() {
        let cfg = resolve_config_js(r#"{"image":"img","remoteUser":"vscode"}"#).unwrap();
        assert_eq!(cfg.remote_user.as_deref(), Some("vscode"));
    }

    #[test]
    fn load_config_reads_file_and_resolves() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("devcontainer.json");
        fs::write(&p, r#"{"image":"alpine","name":"My Wiki"}"#).unwrap();
        let cfg = load_config(&p).unwrap();
        assert_eq!(cfg.image.as_deref(), Some("alpine"));
        assert_eq!(cfg.name.as_deref(), Some("My Wiki"));
    }

    #[test]
    fn js_source_is_embedded_and_non_empty() {
        assert!(!JS_SOURCE.is_empty());
        assert!(JS_SOURCE.contains("resolveConfig"));
    }
}
