use std::sync::atomic::{AtomicU32, Ordering};

use tauri::{command, AppHandle, Manager, State};
use tauri::webview::WebviewWindowBuilder;

use crate::config;
use crate::host::DesktopHostState;
use crate::permissions::PermissionChoice;
use crate::window_state::{AppSettings, WindowGeometry, WindowStateManager};

/// Counter for generating unique window labels.
static WINDOW_COUNTER: AtomicU32 = AtomicU32::new(1);

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

/// Open a URL in a new app window. Only allows trusted wiki3.ai origins.
/// Also allows any *.github.io site (for GitHub Pages sites opened via repo URL).
#[command]
pub fn open_new_window(app: AppHandle, url: String) -> Result<(), String> {
    open_new_window_with_geometry(app, url, None, None, None, None)
}

/// Internal: open a window with optional saved geometry. If no geometry is
/// provided, the window cascades from the previous one.
pub fn open_new_window_with_geometry(
    app: AppHandle,
    url: String,
    x: Option<f64>,
    y: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
) -> Result<(), String> {
    let parsed: tauri::Url = url.parse().map_err(|e: <tauri::Url as std::str::FromStr>::Err| e.to_string())?;

    // Verify the URL is from a trusted origin
    let host = parsed.host_str().unwrap_or("");
    let is_trusted = host == "wiki3.ai"
        || host.ends_with(".wiki3.ai")
        || host.ends_with(".github.io");

    // Also allow the dev URL origin when set
    let is_dev = std::env::var(config::DEV_URL_ENV)
        .ok()
        .and_then(|dev| dev.parse::<tauri::Url>().ok())
        .map(|dev_parsed| {
            parsed.host() == dev_parsed.host()
                && parsed.port() == dev_parsed.port()
                && parsed.scheme() == dev_parsed.scheme()
        })
        .unwrap_or(false);

    if !is_trusted && !is_dev {
        return Err(format!("Refusing to open untrusted URL: {}", url));
    }

    let n = WINDOW_COUNTER.fetch_add(1, Ordering::Relaxed);
    let label = format!("wiki3-{}", n);

    // Use the URL as the window title
    let title = format!("Wiki3 — {}", &url);

    let w = width.unwrap_or(1280.0);
    let h = height.unwrap_or(800.0);

    let mut builder = WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::External(parsed))
        .title(&title)
        .inner_size(w, h)
        .min_inner_size(800.0, 600.0);

    // Position: use saved geometry or cascade
    let (pos_x, pos_y) = if let (Some(sx), Some(sy)) = (x, y) {
        (sx, sy)
    } else if let Some(state) = app.try_state::<WindowStateManager>() {
        state.next_cascade_position()
    } else {
        (80.0, 60.0)
    };
    builder = builder.position(pos_x, pos_y);

    builder.build().map_err(|e| e.to_string())?;

    // Record the window so it can be restored on next launch
    if let Some(state) = app.try_state::<WindowStateManager>() {
        state.record_window(label, WindowGeometry {
            url,
            x: pos_x,
            y: pos_y,
            width: w,
            height: h,
        });
    }

    Ok(())
}

// =============================================================================
// Settings commands
// =============================================================================

/// Return the current app settings.
#[command]
pub fn get_settings(app: AppHandle) -> Result<AppSettings, String> {
    let state = app.state::<WindowStateManager>();
    let settings = state.settings.lock().map_err(|e| e.to_string())?;
    Ok(settings.clone())
}

/// Update app settings (partial — only provided fields are changed).
#[command]
pub fn update_settings(
    app: AppHandle,
    restore_windows: Option<bool>,
    default_repo_url: Option<String>,
) -> Result<AppSettings, String> {
    let state = app.state::<WindowStateManager>();
    let mut settings = state.settings.lock().map_err(|e| e.to_string())?;
    if let Some(v) = restore_windows {
        settings.restore_windows = v;
    }
    if let Some(v) = default_repo_url {
        settings.default_repo_url = v;
    }
    let result = settings.clone();
    drop(settings);
    state.persist();
    Ok(result)
}
