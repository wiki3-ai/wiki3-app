//! Workspace manager — persists and manages local workspace metadata.

use std::fs;
use std::path::{Path, PathBuf};

use crate::workspace::types::Workspace;

/// Manages the set of known workspaces and their on-disk metadata.
pub struct WorkspaceManager {
    /// Directory where workspace metadata is stored.
    storage_dir: PathBuf,
    /// Directory where workspace working copies live by default.
    workspaces_dir: PathBuf,
}

const WORKSPACES_FILE: &str = "workspaces.json";

impl WorkspaceManager {
    /// Create a new workspace manager.
    ///
    /// `storage_dir` — app data directory for metadata files.
    /// `workspaces_dir` — default parent directory for new workspace clones.
    pub fn new(storage_dir: PathBuf, workspaces_dir: PathBuf) -> Self {
        Self {
            storage_dir,
            workspaces_dir,
        }
    }

    /// Default directory for new workspace clones.
    pub fn workspaces_dir(&self) -> &Path {
        &self.workspaces_dir
    }

    /// Directory where workspace metadata is stored (the app data dir).
    pub fn storage_dir(&self) -> &Path {
        &self.storage_dir
    }

    /// Load all persisted workspaces.
    pub fn list_workspaces(&self) -> Result<Vec<Workspace>, WorkspaceError> {
        let path = self.storage_dir.join(WORKSPACES_FILE);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let data = fs::read_to_string(&path).map_err(WorkspaceError::Io)?;
        let workspaces: Vec<Workspace> =
            serde_json::from_str(&data).map_err(WorkspaceError::Json)?;
        Ok(workspaces)
    }

    /// Persist the full workspace list to disk.
    fn save_workspaces(&self, workspaces: &[Workspace]) -> Result<(), WorkspaceError> {
        fs::create_dir_all(&self.storage_dir).map_err(WorkspaceError::Io)?;
        let json = serde_json::to_string_pretty(workspaces).map_err(WorkspaceError::Json)?;
        let path = self.storage_dir.join(WORKSPACES_FILE);
        fs::write(&path, json).map_err(WorkspaceError::Io)?;
        Ok(())
    }

    /// Add a new workspace and persist.
    pub fn add_workspace(&self, workspace: Workspace) -> Result<(), WorkspaceError> {
        let mut workspaces = self.list_workspaces()?;
        // Replace if a workspace with the same id already exists.
        workspaces.retain(|w| w.id != workspace.id);
        workspaces.push(workspace);
        self.save_workspaces(&workspaces)
    }

    /// Remove a workspace by id and persist.
    pub fn remove_workspace(&self, id: &str) -> Result<(), WorkspaceError> {
        let mut workspaces = self.list_workspaces()?;
        workspaces.retain(|w| w.id != id);
        self.save_workspaces(&workspaces)
    }

    /// Get a workspace by id.
    pub fn get_workspace(&self, id: &str) -> Result<Option<Workspace>, WorkspaceError> {
        let workspaces = self.list_workspaces()?;
        Ok(workspaces.into_iter().find(|w| w.id == id))
    }

    /// Update a workspace in place.
    pub fn update_workspace(&self, workspace: Workspace) -> Result<(), WorkspaceError> {
        self.add_workspace(workspace)
    }

    /// Generate the default local path for a new workspace.
    pub fn default_clone_path(&self, repo_name: &str) -> PathBuf {
        self.workspaces_dir.join(repo_name)
    }
}

/// Errors from workspace management operations.
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::types::*;
    use chrono::Utc;
    use tempfile::tempdir;

    fn make_workspace(id: &str, name: &str) -> Workspace {
        Workspace {
            id: id.to_string(),
            name: name.to_string(),
            local_path: format!("/tmp/{name}"),
            provider: ProviderType::GitHub,
            owner: "testuser".to_string(),
            repo: name.to_string(),
            branch: "main".to_string(),
            remotes: vec![],
            publish_mode: PublishMode::None,
            site_url: None,
            origin: WorkspaceOrigin::Existing,
            visibility: RepoVisibility::Public,
            description: None,
            created_at: Utc::now(),
            last_opened_at: Utc::now(),
        }
    }

    #[test]
    fn test_workspace_crud() {
        let dir = tempdir().unwrap();
        let mgr = WorkspaceManager::new(
            dir.path().to_path_buf(),
            dir.path().join("workspaces"),
        );

        // Empty initially
        assert!(mgr.list_workspaces().unwrap().is_empty());

        // Add
        mgr.add_workspace(make_workspace("1", "site-a")).unwrap();
        mgr.add_workspace(make_workspace("2", "site-b")).unwrap();
        assert_eq!(mgr.list_workspaces().unwrap().len(), 2);

        // Get
        let ws = mgr.get_workspace("1").unwrap().unwrap();
        assert_eq!(ws.name, "site-a");

        // Update (re-add with same id)
        let mut updated = make_workspace("1", "site-a-updated");
        updated.branch = "dev".to_string();
        mgr.update_workspace(updated).unwrap();
        let ws = mgr.get_workspace("1").unwrap().unwrap();
        assert_eq!(ws.name, "site-a-updated");
        assert_eq!(ws.branch, "dev");
        assert_eq!(mgr.list_workspaces().unwrap().len(), 2);

        // Remove
        mgr.remove_workspace("1").unwrap();
        assert_eq!(mgr.list_workspaces().unwrap().len(), 1);
        assert!(mgr.get_workspace("1").unwrap().is_none());
    }

    #[test]
    fn test_default_clone_path() {
        let dir = tempdir().unwrap();
        let mgr = WorkspaceManager::new(
            dir.path().to_path_buf(),
            dir.path().join("workspaces"),
        );
        let path = mgr.default_clone_path("my-site");
        assert!(path.ends_with("workspaces/my-site"));
    }
}
