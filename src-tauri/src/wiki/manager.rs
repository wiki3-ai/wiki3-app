//! Persistence and CRUD for `Wiki` dashboard entries.
//!
//! Wikis are stored in `wikis.json` under the app data directory.
//! On first launch (no file present), the manager seeds a couple
//! well-known default wikis. On upgrade, entries from the older
//! `workspaces.json` are imported once (without removing the original).

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::wiki::types::*;
use crate::workspace::manager::WorkspaceManager;
use crate::workspace::types::{RepoVisibility, Workspace};

const WIKIS_FILE: &str = "wikis.json";
const MIGRATION_MARKER: &str = "wikis.migrated";

/// Manages the set of wiki dashboard entries.
pub struct WikiManager {
    storage_dir: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum WikiError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Wiki not found: {0}")]
    NotFound(String),
    #[error("Invalid wiki: {0}")]
    Invalid(String),
}

impl WikiManager {
    pub fn new(storage_dir: PathBuf) -> Self {
        Self { storage_dir }
    }

    fn path(&self) -> PathBuf {
        self.storage_dir.join(WIKIS_FILE)
    }

    fn migration_marker(&self) -> PathBuf {
        self.storage_dir.join(MIGRATION_MARKER)
    }

    /// Load all wikis (without any seeding / migration side-effects).
    pub fn list(&self) -> Result<Vec<Wiki>, WikiError> {
        let path = self.path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let s = fs::read_to_string(&path)?;
        let wikis: Vec<Wiki> = serde_json::from_str(&s)?;
        Ok(wikis)
    }

    fn save(&self, wikis: &[Wiki]) -> Result<(), WikiError> {
        fs::create_dir_all(&self.storage_dir)?;
        let json = serde_json::to_string_pretty(wikis)?;
        fs::write(self.path(), json)?;
        Ok(())
    }

    /// First-time initialization: if the wikis file does not exist,
    /// optionally migrate from `workspaces.json` and seed default
    /// entries. Idempotent — a marker file prevents re-seeding
    /// even if the user later deletes all their wikis.
    pub fn init(&self, workspace_manager: Option<&WorkspaceManager>) -> Result<(), WikiError> {
        let path = self.path();
        if path.exists() || self.migration_marker().exists() {
            return Ok(());
        }

        let mut wikis: Vec<Wiki> = Vec::new();

        if let Some(wm) = workspace_manager {
            if let Ok(workspaces) = wm.list_workspaces() {
                for ws in workspaces {
                    wikis.push(workspace_to_wiki(&ws));
                }
            }
        }

        // Seed defaults if nothing was migrated.
        if wikis.is_empty() {
            wikis.extend(default_seeded_wikis());
        }

        self.save(&wikis)?;

        // Write the migration marker so re-seeding never happens again.
        fs::create_dir_all(&self.storage_dir)?;
        let _ = fs::write(self.migration_marker(), b"1");
        Ok(())
    }

    /// Add or replace (by id) a wiki, validating it first.
    /// New wikis are inserted at the top of the list so they appear
    /// first on the dashboard.
    pub fn add(&self, wiki: Wiki) -> Result<Wiki, WikiError> {
        wiki.validate().map_err(|e| WikiError::Invalid(e.into()))?;
        let mut wikis = self.list()?;
        wikis.retain(|w| w.id != wiki.id);
        wikis.insert(0, wiki.clone());
        self.save(&wikis)?;
        Ok(wiki)
    }

    /// Reorder wikis by the supplied list of ids. Any wiki ids not
    /// mentioned are appended at the end (in their original order) so
    /// no entry can be lost by a partial list.
    pub fn reorder(&self, order: &[String]) -> Result<(), WikiError> {
        let wikis = self.list()?;
        let mut by_id: std::collections::HashMap<String, Wiki> =
            wikis.iter().map(|w| (w.id.clone(), w.clone())).collect();
        let mut result: Vec<Wiki> = Vec::with_capacity(wikis.len());
        for id in order {
            if let Some(w) = by_id.remove(id) {
                result.push(w);
            }
        }
        // Append any remaining wikis (not mentioned in `order`) in their
        // original order.
        for w in wikis {
            if by_id.contains_key(&w.id) {
                result.push(w);
            }
        }
        self.save(&result)
    }

    pub fn remove(&self, id: &str) -> Result<(), WikiError> {
        let mut wikis = self.list()?;
        let before = wikis.len();
        wikis.retain(|w| w.id != id);
        if wikis.len() == before {
            return Err(WikiError::NotFound(id.to_string()));
        }
        self.save(&wikis)?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<Option<Wiki>, WikiError> {
        Ok(self.list()?.into_iter().find(|w| w.id == id))
    }

    /// Update an existing wiki. Missing patch fields leave values unchanged.
    pub fn update(&self, id: &str, patch: UpdateWikiParams) -> Result<Wiki, WikiError> {
        let mut wikis = self.list()?;
        let idx = wikis
            .iter()
            .position(|w| w.id == id)
            .ok_or_else(|| WikiError::NotFound(id.to_string()))?;
        let w = &mut wikis[idx];

        if let Some(name) = patch.name {
            w.name = name;
        }
        if let Some(lp) = patch.local_path {
            w.local_path = lp.filter(|s| !s.trim().is_empty());
        }
        if let Some(ru) = patch.remote_url {
            w.remote = ru
                .and_then(|u| if u.trim().is_empty() { None } else { Some(u) })
                .and_then(|u| remote_from_url(&u));
        }
        if let Some(s) = patch.site_url {
            w.site_url = s.filter(|v| !v.trim().is_empty());
        }
        if let Some(d) = patch.description {
            w.description = d.filter(|v| !v.is_empty());
        }
        if let Some(b) = patch.publish_on_commit {
            w.publish_on_commit = b;
        }
        if let Some(b) = patch.autostart_container {
            w.autostart_container = b;
        }
        w.last_opened_at = Utc::now();
        w.validate().map_err(|e| WikiError::Invalid(e.into()))?;
        let updated = w.clone();
        self.save(&wikis)?;
        Ok(updated)
    }

    /// Convenience: build a new wiki from free-form params.
    pub fn build_from_params(&self, params: AddWikiParams) -> Result<Wiki, WikiError> {
        let remote = params.remote_url.as_deref().and_then(remote_from_url);
        let local_path =
            params
                .local_path
                .and_then(|p| if p.trim().is_empty() { None } else { Some(p) });
        let site_url = params
            .site_url
            .and_then(|p| if p.trim().is_empty() { None } else { Some(p) });

        if local_path.is_none() && remote.is_none() && site_url.is_none() {
            return Err(WikiError::Invalid(
                "Provide at least one of: local path, remote URL, site URL".into(),
            ));
        }

        let name = params
            .name
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| {
                Wiki::derive_name(local_path.as_deref(), remote.as_ref(), site_url.as_deref())
            });

        let now = Utc::now();
        Ok(Wiki {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            local_path,
            remote,
            site_url,
            origin: WikiOrigin::Manual,
            description: params.description.filter(|d| !d.is_empty()),
            created_at: now,
            last_opened_at: now,
            publish_on_commit: false,
            autostart_container: false,
        })
    }

    /// Return the underlying storage path (for tests and debug).
    #[allow(dead_code)]
    pub fn storage_path(&self) -> &Path {
        &self.storage_dir
    }
}

/// Convert a legacy workspace into a wiki entry.
fn workspace_to_wiki(ws: &Workspace) -> Wiki {
    let remote_url = ws
        .remotes
        .iter()
        .find(|r| r.name == "origin")
        .map(|r| r.url.clone())
        .unwrap_or_else(|| format!("https://github.com/{}/{}", ws.owner, ws.repo));
    let remote = remote_from_url(&remote_url).or(Some(RemoteRef {
        provider: WikiProvider::GitHub,
        owner: ws.owner.clone(),
        repo: ws.repo.clone(),
        url: format!("https://github.com/{}/{}", ws.owner, ws.repo),
        visibility: match ws.visibility {
            RepoVisibility::Public => WikiVisibility::Public,
            RepoVisibility::Private => WikiVisibility::Private,
        },
    }));
    Wiki {
        id: ws.id.clone(),
        name: ws.name.clone(),
        local_path: Some(ws.local_path.clone()),
        remote,
        site_url: ws.site_url.clone(),
        origin: WikiOrigin::Existing,
        description: ws.description.clone(),
        created_at: ws.created_at,
        last_opened_at: ws.last_opened_at,
        publish_on_commit: false,
        autostart_container: false,
    }
}

/// The default wikis seeded on a fresh install.
pub fn default_seeded_wikis() -> Vec<Wiki> {
    let now = Utc::now();
    vec![
        seeded("wiki3-ai", "wiki3-ai-site", "The public Wiki3 site", now),
        seeded(
            "wiki3-ai",
            "wiki3-ai-template",
            "The Wiki3 starter template",
            now,
        ),
    ]
}

fn seeded(owner: &str, repo: &str, description: &str, now: chrono::DateTime<Utc>) -> Wiki {
    Wiki {
        id: uuid::Uuid::new_v4().to_string(),
        name: repo.to_string(),
        local_path: None,
        remote: Some(RemoteRef {
            provider: WikiProvider::GitHub,
            owner: owner.to_string(),
            repo: repo.to_string(),
            url: format!("https://github.com/{owner}/{repo}"),
            visibility: WikiVisibility::Public,
        }),
        site_url: Some(derive_github_pages_url(owner, repo)),
        origin: WikiOrigin::Seeded,
        description: Some(description.to_string()),
        created_at: now,
        last_opened_at: now,
        publish_on_commit: false,
        autostart_container: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn init_seeds_on_empty_fs() {
        let dir = tempdir().unwrap();
        let mgr = WikiManager::new(dir.path().to_path_buf());
        mgr.init(None).unwrap();
        let wikis = mgr.list().unwrap();
        assert_eq!(wikis.len(), 2);
        assert!(wikis.iter().any(|w| w.name == "wiki3-ai-site"));
        assert!(wikis.iter().any(|w| w.name == "wiki3-ai-template"));
        for w in &wikis {
            assert_eq!(w.origin, WikiOrigin::Seeded);
            assert!(w.remote.is_some());
            assert!(w.site_url.is_some());
            assert!(w.local_path.is_none());
        }
    }

    #[test]
    fn init_does_not_reseed_after_removal() {
        let dir = tempdir().unwrap();
        let mgr = WikiManager::new(dir.path().to_path_buf());
        mgr.init(None).unwrap();
        let wikis = mgr.list().unwrap();
        for w in wikis {
            mgr.remove(&w.id).unwrap();
        }
        assert!(mgr.list().unwrap().is_empty());

        // Re-run init: should not reseed.
        mgr.init(None).unwrap();
        assert!(mgr.list().unwrap().is_empty());
    }

    #[test]
    fn init_migrates_workspaces() {
        let dir = tempdir().unwrap();
        let wm = WorkspaceManager::new(dir.path().to_path_buf(), dir.path().join("ws"));
        // Seed a workspace
        let ws = Workspace {
            id: "w1".into(),
            name: "my-site".into(),
            local_path: "/tmp/my-site".into(),
            provider: crate::workspace::types::ProviderType::GitHub,
            owner: "me".into(),
            repo: "my-site".into(),
            branch: "main".into(),
            remotes: vec![crate::workspace::types::RemoteInfo {
                name: "origin".into(),
                url: "https://github.com/me/my-site.git".into(),
            }],
            publish_mode: crate::workspace::types::PublishMode::None,
            site_url: Some("https://me.github.io/my-site".into()),
            origin: crate::workspace::types::WorkspaceOrigin::Existing,
            visibility: RepoVisibility::Public,
            description: None,
            created_at: Utc::now(),
            last_opened_at: Utc::now(),
        };
        wm.add_workspace(ws).unwrap();

        let mgr = WikiManager::new(dir.path().to_path_buf());
        mgr.init(Some(&wm)).unwrap();
        let wikis = mgr.list().unwrap();
        assert_eq!(wikis.len(), 1);
        let w = &wikis[0];
        assert_eq!(w.id, "w1");
        assert_eq!(w.local_path.as_deref(), Some("/tmp/my-site"));
        let r = w.remote.as_ref().unwrap();
        assert_eq!(r.owner, "me");
        assert_eq!(r.repo, "my-site");
    }

    #[test]
    fn crud_roundtrip() {
        let dir = tempdir().unwrap();
        let mgr = WikiManager::new(dir.path().to_path_buf());
        let w = mgr
            .build_from_params(AddWikiParams {
                name: None,
                remote_url: Some("https://github.com/a/b".into()),
                local_path: None,
                site_url: None,
                description: None,
            })
            .unwrap();
        assert_eq!(w.name, "b");
        let id = w.id.clone();
        mgr.add(w).unwrap();

        let got = mgr.get(&id).unwrap().unwrap();
        assert_eq!(got.name, "b");

        let updated = mgr
            .update(
                &id,
                UpdateWikiParams {
                    name: Some("renamed".into()),
                    site_url: Some(Some("https://example.com".into())),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(updated.name, "renamed");
        assert_eq!(updated.site_url.as_deref(), Some("https://example.com"));

        mgr.remove(&id).unwrap();
        assert!(mgr.get(&id).unwrap().is_none());
    }

    #[test]
    fn add_rejects_empty_wiki() {
        let dir = tempdir().unwrap();
        let mgr = WikiManager::new(dir.path().to_path_buf());
        let res = mgr.build_from_params(AddWikiParams::default());
        assert!(res.is_err());
    }

    #[test]
    fn remove_nonexistent_returns_error() {
        let dir = tempdir().unwrap();
        let mgr = WikiManager::new(dir.path().to_path_buf());
        assert!(mgr.remove("nope").is_err());
    }

    fn mk(mgr: &WikiManager, name: &str) -> String {
        let w = mgr
            .build_from_params(AddWikiParams {
                name: Some(name.into()),
                remote_url: Some(format!("https://github.com/x/{name}")),
                ..Default::default()
            })
            .unwrap();
        let id = w.id.clone();
        mgr.add(w).unwrap();
        id
    }

    #[test]
    fn add_prepends_new_wikis() {
        let dir = tempdir().unwrap();
        let mgr = WikiManager::new(dir.path().to_path_buf());
        let a = mk(&mgr, "a");
        let b = mk(&mgr, "b");
        let c = mk(&mgr, "c");
        let names: Vec<_> = mgr
            .list()
            .unwrap()
            .into_iter()
            .map(|w| (w.id, w.name))
            .collect();
        // Newest (c) should be first.
        assert_eq!(names[0].0, c);
        assert_eq!(names[1].0, b);
        assert_eq!(names[2].0, a);
    }

    #[test]
    fn reorder_moves_entries_by_id() {
        let dir = tempdir().unwrap();
        let mgr = WikiManager::new(dir.path().to_path_buf());
        let a = mk(&mgr, "a");
        let b = mk(&mgr, "b");
        let c = mk(&mgr, "c");

        // Ask for order [a, b, c]; current order is [c, b, a].
        mgr.reorder(&[a.clone(), b.clone(), c.clone()]).unwrap();
        let ids: Vec<_> = mgr.list().unwrap().into_iter().map(|w| w.id).collect();
        assert_eq!(ids, vec![a.clone(), b.clone(), c.clone()]);
    }

    #[test]
    fn reorder_appends_unmentioned_ids() {
        let dir = tempdir().unwrap();
        let mgr = WikiManager::new(dir.path().to_path_buf());
        let a = mk(&mgr, "a");
        let b = mk(&mgr, "b");
        let c = mk(&mgr, "c");

        // Only mention c; a and b should remain at the end in their
        // original relative order (which is [b, a]).
        mgr.reorder(&[c.clone()]).unwrap();
        let ids: Vec<_> = mgr.list().unwrap().into_iter().map(|w| w.id).collect();
        assert_eq!(ids, vec![c, b, a]);
    }

    #[test]
    fn publish_on_commit_roundtrip() {
        let dir = tempdir().unwrap();
        let mgr = WikiManager::new(dir.path().to_path_buf());
        let id = mk(&mgr, "p");
        assert!(!mgr.get(&id).unwrap().unwrap().publish_on_commit);
        mgr.update(
            &id,
            UpdateWikiParams {
                publish_on_commit: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(mgr.get(&id).unwrap().unwrap().publish_on_commit);
    }
}
