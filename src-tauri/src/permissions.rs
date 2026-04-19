use serde::{Deserialize, Serialize};

/// Permission choices for desktop execution gating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionChoice {
    /// Allow execution for this session only.
    AllowOnce,
    /// Allow execution always for this trusted origin/workspace.
    AllowAlways,
    /// Deny execution.
    Deny,
}

/// Persisted permission state for a trusted origin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionState {
    /// The origin this permission applies to.
    pub origin: String,
    /// The user's permission choice, if any.
    pub choice: Option<PermissionChoice>,
    /// Whether the permission was set to persist across launches.
    pub persistent: bool,
}

impl PermissionState {
    /// Create a new unset permission state for the given origin.
    pub fn new(origin: &str) -> Self {
        Self {
            origin: origin.to_string(),
            choice: None,
            persistent: false,
        }
    }

    /// Update the permission based on a user choice.
    pub fn set_choice(&mut self, choice: PermissionChoice) {
        match choice {
            PermissionChoice::AllowAlways => {
                self.choice = Some(choice);
                self.persistent = true;
            }
            PermissionChoice::AllowOnce => {
                self.choice = Some(choice);
                self.persistent = false;
            }
            PermissionChoice::Deny => {
                self.choice = Some(choice);
                self.persistent = true;
            }
        }
    }

    /// Whether execution is currently allowed.
    pub fn is_execution_allowed(&self) -> bool {
        matches!(
            self.choice,
            Some(PermissionChoice::AllowOnce) | Some(PermissionChoice::AllowAlways)
        )
    }

    /// Reset non-persistent choices (called on app restart).
    pub fn reset_session_permissions(&mut self) {
        if !self.persistent {
            self.choice = None;
        }
    }
}

/// Execution policy that mediates between the desktop host and the frontend extension.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPolicy {
    /// Per-origin permission states.
    pub permissions: Vec<PermissionState>,
}

impl Default for ExecutionPolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecutionPolicy {
    pub fn new() -> Self {
        Self {
            permissions: Vec::new(),
        }
    }

    /// Get or create the permission state for a given origin.
    pub fn get_permission(&mut self, origin: &str) -> &mut PermissionState {
        if !self.permissions.iter().any(|p| p.origin == origin) {
            self.permissions.push(PermissionState::new(origin));
        }
        self.permissions
            .iter_mut()
            .find(|p| p.origin == origin)
            .unwrap()
    }

    /// Check if execution is allowed for a given origin.
    pub fn is_execution_allowed(&self, origin: &str) -> bool {
        self.permissions
            .iter()
            .find(|p| p.origin == origin)
            .is_some_and(|p| p.is_execution_allowed())
    }

    /// Set permission choice for a given origin.
    pub fn set_permission(&mut self, origin: &str, choice: PermissionChoice) {
        let perm = self.get_permission(origin);
        perm.set_choice(choice);
    }

    /// Reset session-only permissions (called on app restart).
    pub fn reset_session_permissions(&mut self) {
        for perm in &mut self.permissions {
            perm.reset_session_permissions();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permission_lifecycle() {
        let mut policy = ExecutionPolicy::new();

        // Initially no permission
        assert!(!policy.is_execution_allowed("https://wiki3.ai"));

        // Allow once
        policy.set_permission("https://wiki3.ai", PermissionChoice::AllowOnce);
        assert!(policy.is_execution_allowed("https://wiki3.ai"));

        // Reset session — allow_once should be cleared
        policy.reset_session_permissions();
        assert!(!policy.is_execution_allowed("https://wiki3.ai"));

        // Allow always
        policy.set_permission("https://wiki3.ai", PermissionChoice::AllowAlways);
        assert!(policy.is_execution_allowed("https://wiki3.ai"));

        // Reset session — allow_always should persist
        policy.reset_session_permissions();
        assert!(policy.is_execution_allowed("https://wiki3.ai"));

        // Deny
        policy.set_permission("https://wiki3.ai", PermissionChoice::Deny);
        assert!(!policy.is_execution_allowed("https://wiki3.ai"));
    }

    #[test]
    fn test_per_origin_isolation() {
        let mut policy = ExecutionPolicy::new();
        policy.set_permission("https://wiki3.ai", PermissionChoice::AllowAlways);
        assert!(policy.is_execution_allowed("https://wiki3.ai"));
        assert!(!policy.is_execution_allowed("https://evil.com"));
    }
}
