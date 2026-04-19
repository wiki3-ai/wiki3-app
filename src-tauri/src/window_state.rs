//! Persistent window state — tracks open site windows and app settings
//! so they can be restored across app launches.

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

/// User-facing app settings persisted across launches.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    /// Whether to reopen site windows from the previous session on launch.
    pub restore_windows: bool,
    /// Default repo URL shown in the dashboard input.
    pub default_repo_url: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            restore_windows: true,
            default_repo_url: DEFAULT_REPO_URL.to_string(),
        }
    }
}

/// Saved geometry for a single window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub url: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Full persisted state (settings + list of open site windows with geometry).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedState {
    settings: AppSettings,
    /// Windows with their last-known position and size.
    open_windows: Vec<WindowGeometry>,
}

impl Default for PersistedState {
    fn default() -> Self {
        Self {
            settings: AppSettings::default(),
            open_windows: Vec::new(),
        }
    }
}

/// Manages window lifecycle tracking and persistence.
pub struct WindowStateManager {
    data_dir: PathBuf,
    /// Currently open site windows: label → geometry.
    pub open_windows: Mutex<HashMap<String, WindowGeometry>>,
    /// Persisted settings.
    pub settings: Mutex<AppSettings>,
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
    pub fn saved_windows(&self) -> Vec<WindowGeometry> {
        Self::load(&self.data_dir).open_windows
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

    /// Update the geometry of an existing window (e.g. after move/resize).
    pub fn update_window_geometry(&self, label: &str, x: f64, y: f64, width: f64, height: f64) {
        let mut windows = self.open_windows.lock().unwrap();
        if let Some(geom) = windows.get_mut(label) {
            geom.x = x;
            geom.y = y;
            geom.width = width;
            geom.height = height;
        }
        drop(windows);
        self.persist();
    }

    /// Remove a closed site window.
    pub fn remove_window(&self, label: &str) {
        let mut windows = self.open_windows.lock().unwrap();
        windows.remove(label);
        drop(windows);
        self.persist();
    }

    /// Write current state to disk.
    pub fn persist(&self) {
        let windows = self.open_windows.lock().unwrap();
        let settings = self.settings.lock().unwrap();
        let state = PersistedState {
            settings: settings.clone(),
            open_windows: windows.values().cloned().collect(),
        };
        drop(windows);
        drop(settings);

        let path = Self::state_path(&self.data_dir);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&state) {
            let _ = std::fs::write(&path, json);
        }
    }
}
