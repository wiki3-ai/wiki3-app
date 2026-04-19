use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::config::AppConfig;
use crate::permissions::ExecutionPolicy;

/// Desktop host state managed by Tauri.
/// This is the central state object that holds configuration and execution policy.
pub struct DesktopHostState {
    pub config: Mutex<AppConfig>,
    pub policy: Mutex<ExecutionPolicy>,
    data_dir: PathBuf,
}

impl DesktopHostState {
    /// Create a new desktop host state, loading persisted policy if available.
    pub fn new(data_dir: PathBuf) -> Self {
        let config = AppConfig::default();

        // Load persisted execution policy
        let policy_path = data_dir.join("execution_policy.json");
        let mut policy = if policy_path.exists() {
            fs::read_to_string(&policy_path)
                .ok()
                .and_then(|s| serde_json::from_str::<ExecutionPolicy>(&s).ok())
                .unwrap_or_default()
        } else {
            ExecutionPolicy::new()
        };

        // Reset session-only permissions on app start
        policy.reset_session_permissions();

        Self {
            config: Mutex::new(config),
            policy: Mutex::new(policy),
            data_dir,
        }
    }

    /// Save the current execution policy to disk.
    pub fn save_policy(&self) -> Result<(), Box<dyn std::error::Error>> {
        let policy = self.policy.lock().map_err(|e| e.to_string())?;
        let policy_path = self.data_dir.join("execution_policy.json");

        // Ensure directory exists
        if let Some(parent) = policy_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_string_pretty(&*policy)?;
        fs::write(&policy_path, json)?;
        Ok(())
    }
}
