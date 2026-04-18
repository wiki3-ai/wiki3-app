use tauri::{command, AppHandle, State};

use crate::host::DesktopHostState;
use crate::permissions::PermissionChoice;

/// Detect whether the app is running as a desktop host.
/// Returns host information if called from a trusted origin.
#[command]
pub fn detect_desktop_host(
    _app: AppHandle,
    origin: String,
    state: State<'_, DesktopHostState>,
) -> Result<serde_json::Value, String> {
    let config = state.config.lock().map_err(|e| e.to_string())?;

    if !config.is_trusted_origin(&origin) {
        return Ok(serde_json::json!({
            "detected": false,
            "reason": "untrusted_origin"
        }));
    }

    Ok(serde_json::json!({
        "detected": true,
        "host": "wiki3-desktop",
        "version": env!("CARGO_PKG_VERSION"),
        "origin": origin
    }))
}

/// Get the current permission state for the given origin.
#[command]
pub fn get_permission_state(
    origin: String,
    state: State<'_, DesktopHostState>,
) -> Result<serde_json::Value, String> {
    let config = state.config.lock().map_err(|e| e.to_string())?;
    if !config.is_trusted_origin(&origin) {
        return Err("Untrusted origin".to_string());
    }

    let policy = state.policy.lock().map_err(|e| e.to_string())?;
    let allowed = policy.is_execution_allowed(&origin);

    let choice = policy
        .permissions
        .iter()
        .find(|p| p.origin == origin)
        .and_then(|p| p.choice);

    Ok(serde_json::json!({
        "origin": origin,
        "execution_allowed": allowed,
        "choice": choice,
    }))
}

/// Set the execution permission for a given origin.
#[command]
pub fn set_execution_permission(
    origin: String,
    choice: PermissionChoice,
    state: State<'_, DesktopHostState>,
) -> Result<serde_json::Value, String> {
    let config = state.config.lock().map_err(|e| e.to_string())?;
    if !config.is_trusted_origin(&origin) {
        return Err("Untrusted origin".to_string());
    }

    let mut policy = state.policy.lock().map_err(|e| e.to_string())?;
    policy.set_permission(&origin, choice);
    let allowed = policy.is_execution_allowed(&origin);

    // Persist the policy
    drop(policy);
    drop(config);
    state.save_policy().map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "origin": origin,
        "execution_allowed": allowed,
        "choice": choice,
    }))
}

/// Get the execution enablement state for a given origin.
/// This is the execution policy layer that the frontend extension consults.
#[command]
pub fn get_execution_state(
    origin: String,
    state: State<'_, DesktopHostState>,
) -> Result<serde_json::Value, String> {
    let config = state.config.lock().map_err(|e| e.to_string())?;
    if !config.is_trusted_origin(&origin) {
        return Ok(serde_json::json!({
            "trusted": false,
            "execution_allowed": false,
            "reason": "untrusted_origin"
        }));
    }

    let policy = state.policy.lock().map_err(|e| e.to_string())?;
    let allowed = policy.is_execution_allowed(&origin);

    Ok(serde_json::json!({
        "trusted": true,
        "execution_allowed": allowed,
        "needs_permission": !allowed,
    }))
}

/// Get the app configuration (non-sensitive parts).
#[command]
pub fn get_app_config(
    state: State<'_, DesktopHostState>,
) -> Result<serde_json::Value, String> {
    let config = state.config.lock().map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "site_url": config.effective_url(),
        "trusted_origins": config.trusted_origins,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}
