//! Wiki data model — provider-neutral dashboard entries.
//!
//! A `Wiki` is a loose collection of up to three optional properties
//! (local path / remote repo / static site URL). At least one must be
//! set. None of them is required to actually exist on disk or remotely
//! — operations should fail gracefully when a property is missing.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Which hosting/repo provider backs the remote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WikiProvider {
    GitHub,
    /// Unknown or generic git remote (not GitHub-specific).
    Other,
}

/// Visibility of the remote repository, if known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WikiVisibility {
    Public,
    Private,
    Unknown,
}

/// How the wiki entry was created / where it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WikiOrigin {
    /// Pre-seeded default wiki.
    Seeded,
    /// Added manually via the "Add Wiki" dialog.
    Manual,
    /// Created from a template repository.
    Template {
        template_owner: String,
        template_repo: String,
    },
    /// Forked from an existing repository.
    Fork {
        upstream_owner: String,
        upstream_repo: String,
    },
    /// Opened from an existing local directory.
    Existing,
    /// Cloned from a remote URL.
    Clone,
}

/// Reference to a remote repository.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteRef {
    pub provider: WikiProvider,
    pub owner: String,
    pub repo: String,
    /// Canonical URL (e.g. `https://github.com/owner/repo`).
    pub url: String,
    #[serde(default = "default_visibility")]
    pub visibility: WikiVisibility,
}

fn default_visibility() -> WikiVisibility {
    WikiVisibility::Unknown
}

/// A dashboard wiki entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Wiki {
    /// Unique identifier (UUID).
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// Local working directory (may or may not exist on disk).
    #[serde(default)]
    pub local_path: Option<String>,
    /// Remote repository reference, if any.
    #[serde(default)]
    pub remote: Option<RemoteRef>,
    /// URL of the published static site, if known.
    #[serde(default)]
    pub site_url: Option<String>,
    /// How this entry was created.
    #[serde(default = "default_origin")]
    pub origin: WikiOrigin,
    /// Optional description.
    #[serde(default)]
    pub description: Option<String>,
    /// When this entry was created.
    pub created_at: DateTime<Utc>,
    /// When this entry was last opened/used.
    pub last_opened_at: DateTime<Utc>,
    /// If true, a successful commit via the dashboard also pushes and
    /// publishes the site. Only meaningful when `local_path` is set.
    #[serde(default)]
    pub publish_on_commit: bool,
    /// If true, the per-wiki preview container is started automatically
    /// when the app launches. Only meaningful when `local_path` is set.
    #[serde(default)]
    pub autostart_container: bool,
}

fn default_origin() -> WikiOrigin {
    WikiOrigin::Manual
}

impl Wiki {
    /// Validate the wiki has at least one identifying property.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.local_path.is_none() && self.remote.is_none() && self.site_url.is_none() {
            return Err("A wiki must have at least one of: local_path, remote, site_url");
        }
        Ok(())
    }

    /// Derive a display name from the available properties.
    pub fn derive_name(
        local_path: Option<&str>,
        remote: Option<&RemoteRef>,
        site_url: Option<&str>,
    ) -> String {
        if let Some(r) = remote {
            return r.repo.clone();
        }
        if let Some(p) = local_path {
            let path = std::path::Path::new(p);
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                return name.to_string();
            }
            return p.to_string();
        }
        if let Some(url) = site_url {
            // Try to extract the last path segment or host
            if let Ok(parsed) = url::Url::parse(url) {
                let seg = parsed
                    .path_segments()
                    .and_then(|mut s| s.next_back())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
                if let Some(s) = seg {
                    return s;
                }
                if let Some(h) = parsed.host_str() {
                    return h.to_string();
                }
            }
            return url.to_string();
        }
        "wiki".to_string()
    }
}

/// Parameters for creating a new dashboard entry manually.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AddWikiParams {
    pub name: Option<String>,
    pub local_path: Option<String>,
    pub remote_url: Option<String>,
    pub site_url: Option<String>,
    pub description: Option<String>,
}

/// Patch for updating a wiki entry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateWikiParams {
    pub name: Option<String>,
    pub local_path: Option<Option<String>>,
    pub remote_url: Option<Option<String>>,
    pub site_url: Option<Option<String>>,
    pub description: Option<Option<String>>,
    pub publish_on_commit: Option<bool>,
    pub autostart_container: Option<bool>,
}

/// Parse a GitHub HTTPS / SSH URL into `(owner, repo)`.
pub fn parse_github_url(url: &str) -> Option<(String, String)> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = if trimmed.contains("github.com/") {
        trimmed.split("github.com/").nth(1)?
    } else if trimmed.contains("github.com:") {
        trimmed.split("github.com:").nth(1)?
    } else {
        return None;
    };
    let path = path.trim_end_matches(".git").trim_end_matches('/');
    let mut parts = path.split('/');
    let owner = parts.next()?.to_string();
    let repo = parts.next()?.to_string();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner, repo))
}

/// Build a `RemoteRef` from a URL string.
pub fn remote_from_url(url: &str) -> Option<RemoteRef> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    if let Some((owner, repo)) = parse_github_url(url) {
        let canonical = format!("https://github.com/{owner}/{repo}");
        return Some(RemoteRef {
            provider: WikiProvider::GitHub,
            owner,
            repo,
            url: canonical,
            visibility: WikiVisibility::Unknown,
        });
    }
    None
}

/// Derive a conventional GitHub Pages site URL from owner/repo.
pub fn derive_github_pages_url(owner: &str, repo: &str) -> String {
    if repo == format!("{owner}.github.io") {
        format!("https://{owner}.github.io")
    } else {
        format!("https://{owner}.github.io/{repo}")
    }
}

// We need a tiny URL parser for derive_name; use `url` crate? Not in deps.
// Implement a minimal substitute to avoid adding a dep.
mod url {
    pub struct Url {
        host: Option<String>,
        path: String,
    }
    impl Url {
        pub fn parse(s: &str) -> Result<Self, ()> {
            let without_scheme = s
                .strip_prefix("https://")
                .or_else(|| s.strip_prefix("http://"))
                .ok_or(())?;
            let (authority, path) = match without_scheme.find('/') {
                Some(i) => (&without_scheme[..i], &without_scheme[i..]),
                None => (without_scheme, ""),
            };
            let host = if authority.is_empty() {
                None
            } else {
                // strip user@ and :port
                let a = authority.rsplit('@').next().unwrap_or(authority);
                let a = a.split(':').next().unwrap_or(a);
                Some(a.to_string())
            };
            Ok(Url {
                host,
                path: path.to_string(),
            })
        }
        pub fn host_str(&self) -> Option<&str> {
            self.host.as_deref()
        }
        pub fn path_segments(&self) -> Option<PathSegments<'_>> {
            let trimmed = self.path.trim_start_matches('/').trim_end_matches('/');
            if trimmed.is_empty() {
                return Some(PathSegments { parts: vec![] });
            }
            Some(PathSegments {
                parts: trimmed.split('/').collect(),
            })
        }
    }
    pub struct PathSegments<'a> {
        parts: Vec<&'a str>,
    }
    impl<'a> PathSegments<'a> {
        pub fn next_back(&mut self) -> Option<&'a str> {
            self.parts.pop()
        }
        #[allow(dead_code)]
        pub fn next(&mut self) -> Option<&'a str> {
            if self.parts.is_empty() {
                None
            } else {
                Some(self.parts.remove(0))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_github_url_variants() {
        assert_eq!(
            parse_github_url("https://github.com/wiki3-ai/wiki3-ai-site"),
            Some(("wiki3-ai".into(), "wiki3-ai-site".into()))
        );
        assert_eq!(
            parse_github_url("https://github.com/wiki3-ai/wiki3-ai-site.git"),
            Some(("wiki3-ai".into(), "wiki3-ai-site".into()))
        );
        assert_eq!(
            parse_github_url("git@github.com:wiki3-ai/x.git"),
            Some(("wiki3-ai".into(), "x".into()))
        );
        assert!(parse_github_url("https://gitlab.com/a/b").is_none());
        assert!(parse_github_url("").is_none());
    }

    #[test]
    fn remote_from_url_github() {
        let r = remote_from_url("https://github.com/wiki3-ai/wiki3-ai-site").unwrap();
        assert_eq!(r.provider, WikiProvider::GitHub);
        assert_eq!(r.owner, "wiki3-ai");
        assert_eq!(r.repo, "wiki3-ai-site");
        assert_eq!(r.url, "https://github.com/wiki3-ai/wiki3-ai-site");
    }

    #[test]
    fn derive_pages_url_project_and_user() {
        assert_eq!(
            derive_github_pages_url("wiki3-ai", "wiki3-ai-site"),
            "https://wiki3-ai.github.io/wiki3-ai-site"
        );
        assert_eq!(
            derive_github_pages_url("me", "me.github.io"),
            "https://me.github.io"
        );
    }

    #[test]
    fn wiki_validate() {
        let now = Utc::now();
        let bad = Wiki {
            id: "1".into(),
            name: "x".into(),
            local_path: None,
            remote: None,
            site_url: None,
            origin: WikiOrigin::Manual,
            description: None,
            created_at: now,
            last_opened_at: now,
            publish_on_commit: false,
            autostart_container: false,
        };
        assert!(bad.validate().is_err());

        let good = Wiki {
            site_url: Some("https://example.com".into()),
            ..bad.clone()
        };
        assert!(good.validate().is_ok());
    }

    #[test]
    fn derive_name_prefers_remote_then_local_then_site() {
        let remote = RemoteRef {
            provider: WikiProvider::GitHub,
            owner: "o".into(),
            repo: "my-repo".into(),
            url: "https://github.com/o/my-repo".into(),
            visibility: WikiVisibility::Unknown,
        };
        assert_eq!(
            Wiki::derive_name(Some("/tmp/garden"), Some(&remote), Some("https://x.y/site")),
            "my-repo"
        );
        assert_eq!(
            Wiki::derive_name(Some("/tmp/garden"), None, Some("https://x.y/site")),
            "garden"
        );
        assert_eq!(
            Wiki::derive_name(None, None, Some("https://x.example.com/abc")),
            "abc"
        );
        assert_eq!(
            Wiki::derive_name(None, None, Some("https://just-a-host.example.com/")),
            "just-a-host.example.com"
        );
    }

    #[test]
    fn wiki_roundtrip_serde() {
        let now = Utc::now();
        let w = Wiki {
            id: "abc".into(),
            name: "test".into(),
            local_path: Some("/tmp/test".into()),
            remote: Some(RemoteRef {
                provider: WikiProvider::GitHub,
                owner: "me".into(),
                repo: "test".into(),
                url: "https://github.com/me/test".into(),
                visibility: WikiVisibility::Public,
            }),
            site_url: Some("https://me.github.io/test".into()),
            origin: WikiOrigin::Seeded,
            description: None,
            created_at: now,
            last_opened_at: now,
            publish_on_commit: false,
            autostart_container: false,
        };
        let j = serde_json::to_string(&w).unwrap();
        let back: Wiki = serde_json::from_str(&j).unwrap();
        assert_eq!(back.id, "abc");
        assert_eq!(back.origin, WikiOrigin::Seeded);
        assert_eq!(back.remote.unwrap().visibility, WikiVisibility::Public);
        assert!(!back.publish_on_commit);
    }

    /// Old JSON (pre-`publish_on_commit`) must still deserialize.
    #[test]
    fn wiki_deserialize_without_publish_on_commit() {
        let now = Utc::now().to_rfc3339();
        let json = format!(
            r#"{{
                "id": "x",
                "name": "x",
                "local_path": null,
                "remote": null,
                "site_url": "https://example.com",
                "origin": "manual",
                "description": null,
                "created_at": "{now}",
                "last_opened_at": "{now}"
            }}"#
        );
        let w: Wiki = serde_json::from_str(&json).unwrap();
        assert!(!w.publish_on_commit);
    }
}
