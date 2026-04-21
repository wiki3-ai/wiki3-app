//! Persistent window state — tracks open site windows, their
//! wiki-ownership, and app settings so they can be restored across
//! app launches.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const STATE_FILE: &str = "window_state.json";
pub const DEFAULT_REPO_URL: &str = "https://github.com/wiki3-ai/wiki3-ai-site";

/// Cascade offset in pixels for each new window.
const CASCADE_X: f64 = 28.0;
const CASCADE_Y: f64 = 28.0;
/// Starting position of the first site window.
const CASCADE_START_X: f64 = 80.0;
const CASCADE_START_Y: f64 = 60.0;
/// Max cascade steps before wrapping back to start.
const CASCADE_MAX: u32 = 12;

/// State of Wiki3-managed CLI tools that the user's device needs in
/// order to build/serve sites in a sandbox. Each entry maps a tool
/// name (e.g. `"deno"`) to the installed version as last seen by the
/// installer. This exists mainly so the UI can show "installed" state
/// without having to re-scan disk on every render; the source of
/// truth is still the filesystem under `<app_data>/tools/`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManagedToolsState {
    /// Tool name -> installed version string.
    #[serde(default)]
    pub installed: HashMap<String, String>,
}

/// User-facing app settings persisted across launches.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    /// Whether to reopen site windows from the previous session on launch.
    pub restore_windows: bool,
    /// Default repo URL shown in the dashboard input.
    pub default_repo_url: String,
    /// Default base directory for new wiki clones (e.g. `~/Wiki3`).
    #[serde(default)]
    pub default_wikis_dir: Option<String>,
    /// State of Wiki3-managed CLI tools (Deno, devcontainer CLI, …).
    /// Added in the Deno/devcontainers migration; defaults to empty
    /// so older persisted state loads cleanly.
    #[serde(default)]
    pub managed_tools: ManagedToolsState,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            restore_windows: true,
            default_repo_url: DEFAULT_REPO_URL.to_string(),
            default_wikis_dir: None,
            managed_tools: ManagedToolsState::default(),
        }
    }
}

/// Saved geometry for a single tracked window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub url: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// If this window was opened from a wiki entry, the wiki id.
    #[serde(default)]
    pub wiki_id: Option<String>,
    /// True if the window has been closed but its geometry is kept
    /// so the user can reopen it in the same place.
    #[serde(default)]
    pub closed: bool,
}

/// Persisted dashboard main-window geometry (separate from site windows).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardGeometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Full persisted state.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedState {
    #[serde(default)]
    settings: AppSettings,
    /// Site windows with their last-known position/size.
    #[serde(default)]
    open_windows: Vec<WindowGeometry>,
    /// Dashboard geometry (if it has ever been recorded).
    #[serde(default)]
    dashboard_geometry: Option<DashboardGeometry>,
}

/// Info returned to the frontend about a tracked window.
#[derive(Debug, Clone, Serialize)]
pub struct TrackedWindowInfo {
    pub label: String,
    pub url: String,
    pub wiki_id: Option<String>,
    pub closed: bool,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Manages window lifecycle tracking and persistence.
pub struct WindowStateManager {
    data_dir: PathBuf,
    /// Tracked site windows: label → geometry (includes both open and
    /// recently-closed-but-remembered windows).
    pub open_windows: Mutex<HashMap<String, WindowGeometry>>,
    /// Persisted settings.
    pub settings: Mutex<AppSettings>,
    /// Dashboard window geometry.
    pub dashboard_geometry: Mutex<Option<DashboardGeometry>>,
    /// Cascade counter for positioning new windows.
    cascade_counter: Mutex<u32>,
}

impl WindowStateManager {
    pub fn new(data_dir: PathBuf) -> Self {
        let state = Self::load(&data_dir);
        Self {
            data_dir,
            open_windows: Mutex::new(HashMap::new()),
            settings: Mutex::new(state.settings),
            dashboard_geometry: Mutex::new(state.dashboard_geometry),
            cascade_counter: Mutex::new(0),
        }
    }

    fn state_path(data_dir: &Path) -> PathBuf {
        data_dir.join(STATE_FILE)
    }

    fn load(data_dir: &Path) -> PersistedState {
        let path = Self::state_path(data_dir);
        if path.exists() {
            std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default()
        } else {
            PersistedState::default()
        }
    }

    /// Window geometries that were saved when the app last quit.
    /// Only returns windows that were *open* (not ones marked closed).
    pub fn saved_open_windows(&self) -> Vec<WindowGeometry> {
        Self::load(&self.data_dir)
            .open_windows
            .into_iter()
            .filter(|w| !w.closed)
            .collect()
    }

    /// Whether window restore is enabled.
    pub fn should_restore(&self) -> bool {
        self.settings.lock().unwrap().restore_windows
    }

    /// Get the next cascade position for a new window.
    pub fn next_cascade_position(&self) -> (f64, f64) {
        let mut counter = self.cascade_counter.lock().unwrap();
        let n = *counter % CASCADE_MAX;
        *counter += 1;
        (
            CASCADE_START_X + (n as f64) * CASCADE_X,
            CASCADE_START_Y + (n as f64) * CASCADE_Y,
        )
    }

    /// Record a newly opened site window with its geometry.
    pub fn record_window(&self, label: String, geom: WindowGeometry) {
        let mut windows = self.open_windows.lock().unwrap();
        windows.insert(label, geom);
        drop(windows);
        self.persist();
    }

    /// Update the geometry of an existing window (move/resize).
    pub fn update_window_geometry(&self, label: &str, x: f64, y: f64, width: f64, height: f64) {
        let mut windows = self.open_windows.lock().unwrap();
        if let Some(geom) = windows.get_mut(label) {
            geom.x = x;
            geom.y = y;
            geom.width = width;
            geom.height = height;
            geom.closed = false;
        }
        drop(windows);
        self.persist();
    }

    /// Called when the OS tells us a window was destroyed.
    /// If `keep_for_reopen` is true (e.g. because the window has a wiki
    /// owner and we want to allow Reopen), the entry is retained but
    /// flagged `closed`. Otherwise it is fully removed.
    pub fn on_window_destroyed(&self, label: &str, keep_for_reopen: bool) {
        let mut windows = self.open_windows.lock().unwrap();
        if keep_for_reopen {
            if let Some(geom) = windows.get_mut(label) {
                geom.closed = true;
            }
        } else {
            windows.remove(label);
        }
        drop(windows);
        self.persist();
    }

    /// Mark a window as closed (but keep its geometry remembered).
    /// Used when the user invokes "Close all" on a wiki.
    pub fn mark_closed(&self, label: &str) {
        let mut windows = self.open_windows.lock().unwrap();
        if let Some(geom) = windows.get_mut(label) {
            geom.closed = true;
        }
        drop(windows);
        self.persist();
    }

    /// Completely forget a tracked window.
    pub fn forget_window(&self, label: &str) {
        let mut windows = self.open_windows.lock().unwrap();
        windows.remove(label);
        drop(windows);
        self.persist();
    }

    /// All windows tracked for a given wiki id.
    pub fn windows_for_wiki(&self, wiki_id: &str) -> Vec<TrackedWindowInfo> {
        let windows = self.open_windows.lock().unwrap();
        windows
            .iter()
            .filter(|(_, g)| g.wiki_id.as_deref() == Some(wiki_id))
            .map(|(label, g)| TrackedWindowInfo {
                label: label.clone(),
                url: g.url.clone(),
                wiki_id: g.wiki_id.clone(),
                closed: g.closed,
                x: g.x,
                y: g.y,
                width: g.width,
                height: g.height,
            })
            .collect()
    }

    /// All tracked windows (open or closed-with-memory).
    pub fn all_tracked(&self) -> Vec<TrackedWindowInfo> {
        let windows = self.open_windows.lock().unwrap();
        windows
            .iter()
            .map(|(label, g)| TrackedWindowInfo {
                label: label.clone(),
                url: g.url.clone(),
                wiki_id: g.wiki_id.clone(),
                closed: g.closed,
                x: g.x,
                y: g.y,
                width: g.width,
                height: g.height,
            })
            .collect()
    }

    /// Save dashboard geometry.
    pub fn update_dashboard_geometry(&self, x: f64, y: f64, width: f64, height: f64) {
        let mut g = self.dashboard_geometry.lock().unwrap();
        *g = Some(DashboardGeometry {
            x,
            y,
            width,
            height,
        });
        drop(g);
        self.persist();
    }

    pub fn dashboard_geometry(&self) -> Option<DashboardGeometry> {
        self.dashboard_geometry.lock().unwrap().clone()
    }

    /// Write current state to disk.
    pub fn persist(&self) {
        let windows = self.open_windows.lock().unwrap();
        let settings = self.settings.lock().unwrap();
        let dashboard_geometry = self.dashboard_geometry.lock().unwrap();
        let state = PersistedState {
            settings: settings.clone(),
            open_windows: windows.values().cloned().collect(),
            dashboard_geometry: dashboard_geometry.clone(),
        };
        drop(windows);
        drop(settings);
        drop(dashboard_geometry);

        let path = Self::state_path(&self.data_dir);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&state) {
            let _ = std::fs::write(&path, json);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn geom(url: &str, wiki_id: Option<&str>) -> WindowGeometry {
        WindowGeometry {
            url: url.into(),
            x: 10.0,
            y: 20.0,
            width: 800.0,
            height: 600.0,
            wiki_id: wiki_id.map(|s| s.to_string()),
            closed: false,
        }
    }

    #[test]
    fn records_and_filters_by_wiki() {
        let dir = tempdir().unwrap();
        let m = WindowStateManager::new(dir.path().to_path_buf());
        m.record_window("wiki3-1".into(), geom("https://a/", Some("w1")));
        m.record_window("wiki3-2".into(), geom("https://b/", Some("w1")));
        m.record_window("wiki3-3".into(), geom("https://c/", Some("w2")));
        m.record_window("wiki3-4".into(), geom("https://d/", None));

        let w1 = m.windows_for_wiki("w1");
        assert_eq!(w1.len(), 2);
        assert!(w1.iter().all(|w| w.wiki_id.as_deref() == Some("w1")));

        let w2 = m.windows_for_wiki("w2");
        assert_eq!(w2.len(), 1);
        assert_eq!(w2[0].url, "https://c/");
    }

    #[test]
    fn mark_closed_keeps_geometry_saved_filters_out() {
        let dir = tempdir().unwrap();
        let m = WindowStateManager::new(dir.path().to_path_buf());
        m.record_window("wiki3-1".into(), geom("https://a/", Some("w1")));
        m.update_window_geometry("wiki3-1", 100.0, 200.0, 900.0, 700.0);
        m.mark_closed("wiki3-1");

        // Still tracked for reopen logic
        let tracked = m.windows_for_wiki("w1");
        assert_eq!(tracked.len(), 1);
        assert!(tracked[0].closed);
        assert_eq!(tracked[0].x, 100.0);
        assert_eq!(tracked[0].width, 900.0);

        // Not in the "restore on startup" set since it's closed.
        assert!(m.saved_open_windows().is_empty());
    }

    #[test]
    fn destroyed_forgets_without_keep() {
        let dir = tempdir().unwrap();
        let m = WindowStateManager::new(dir.path().to_path_buf());
        m.record_window("wiki3-1".into(), geom("https://a/", None));
        m.on_window_destroyed("wiki3-1", false);
        assert!(m.all_tracked().is_empty());
    }

    #[test]
    fn destroyed_with_keep_retains_entry() {
        let dir = tempdir().unwrap();
        let m = WindowStateManager::new(dir.path().to_path_buf());
        m.record_window("wiki3-1".into(), geom("https://a/", Some("w1")));
        m.on_window_destroyed("wiki3-1", true);
        let tracked = m.windows_for_wiki("w1");
        assert_eq!(tracked.len(), 1);
        assert!(tracked[0].closed);
    }

    #[test]
    fn dashboard_geometry_roundtrip() {
        let dir = tempdir().unwrap();
        {
            let m = WindowStateManager::new(dir.path().to_path_buf());
            m.update_dashboard_geometry(5.0, 6.0, 1000.0, 800.0);
        }
        let m2 = WindowStateManager::new(dir.path().to_path_buf());
        let g = m2.dashboard_geometry().unwrap();
        assert_eq!(g.x, 5.0);
        assert_eq!(g.width, 1000.0);
    }

    #[test]
    fn settings_backcompat_missing_default_wikis_dir() {
        // Simulate reading old state files (missing default_wikis_dir).
        let json = r#"{
            "settings": { "restore_windows": true, "default_repo_url": "https://github.com/x/y" },
            "open_windows": []
        }"#;
        let s: PersistedState = serde_json::from_str(json).unwrap();
        assert_eq!(s.settings.default_wikis_dir, None);
        assert_eq!(s.settings.default_repo_url, "https://github.com/x/y");
    }

    #[test]
    fn window_geometry_backcompat_missing_wiki_id_closed() {
        let json = r#"{
            "url": "https://a/", "x": 1, "y": 2, "width": 3, "height": 4
        }"#;
        let g: WindowGeometry = serde_json::from_str(json).unwrap();
        assert_eq!(g.wiki_id, None);
        assert!(!g.closed);
    }
}
