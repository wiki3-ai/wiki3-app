use std::sync::atomic::{AtomicU32, Ordering};

use tauri::{command, AppHandle, Manager, State};
use tauri::webview::WebviewWindowBuilder;

use crate::config;
use crate::host::DesktopHostState;
use crate::permissions::PermissionChoice;
use crate::window_state::{
    AppSettings, DashboardGeometry, TrackedWindowInfo, WindowGeometry, WindowStateManager,
};

/// Counter for generating unique window labels.
static WINDOW_COUNTER: AtomicU32 = AtomicU32::new(1);

/// Label of the main dashboard window.
pub const DASHBOARD_LABEL: &str = "main";

/// Detect whether the app is running as a desktop host.
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

    drop(policy);
    drop(config);
    state.save_policy().map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "origin": origin,
        "execution_allowed": allowed,
        "choice": choice,
    }))
}

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

/// Whether a URL is allowed to be opened as an in-app site window.
///
/// We allow:
/// - `wiki3.ai` and its subdomains (historical trust).
/// - Any `*.github.io` site.
/// - The dev URL from `WIKI3_DEV_URL`, if set.
/// - Any `https://` URL whose origin matches the `site_url` of a
///   registered wiki (user-opt-in allowlist for custom domains).
fn is_url_allowed_for_site_window(app: &AppHandle, url: &tauri::Url) -> bool {
    let host = url.host_str().unwrap_or("");
    if host == "wiki3.ai" || host.ends_with(".wiki3.ai") || host.ends_with(".github.io") {
        return true;
    }

    if let Ok(dev) = std::env::var(config::DEV_URL_ENV) {
        if let Ok(dev_parsed) = dev.parse::<tauri::Url>() {
            if url.host() == dev_parsed.host()
                && url.port() == dev_parsed.port()
                && url.scheme() == dev_parsed.scheme()
            {
                return true;
            }
        }
    }

    if url.scheme() == "https" {
        if let Some(wiki_state) = app.try_state::<crate::wiki::commands::WikiState>() {
            if let Ok(wikis) = wiki_state.manager.list() {
                let target_origin = format!("{}://{}", url.scheme(), host);
                for w in wikis {
                    if let Some(site) = w.site_url.as_deref() {
                        if let Ok(site_url) = site.parse::<tauri::Url>() {
                            let site_origin = format!(
                                "{}://{}",
                                site_url.scheme(),
                                site_url.host_str().unwrap_or("")
                            );
                            if site_origin == target_origin {
                                return true;
                            }
                        }
                    }
                }
            }
        }
    }

    false
}

/// Open a URL in a new app window.
#[command]
pub fn open_new_window(app: AppHandle, url: String) -> Result<(), String> {
    open_new_window_with_geometry(app, url, None, None, None, None, None)
}

/// Open a URL in a new app window tagged to a specific wiki.
#[command]
pub fn open_new_window_for_wiki(
    app: AppHandle,
    url: String,
    wiki_id: String,
) -> Result<(), String> {
    open_new_window_with_geometry(app, url, None, None, None, None, Some(wiki_id))
}

/// Internal: open a window with optional saved geometry and wiki owner.
#[allow(clippy::too_many_arguments)]
pub fn open_new_window_with_geometry(
    app: AppHandle,
    url: String,
    x: Option<f64>,
    y: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
    wiki_id: Option<String>,
) -> Result<(), String> {
    let parsed: tauri::Url = url
        .parse()
        .map_err(|e: <tauri::Url as std::str::FromStr>::Err| e.to_string())?;

    if !is_url_allowed_for_site_window(&app, &parsed) {
        return Err(format!("Refusing to open untrusted URL: {}", url));
    }

    let n = WINDOW_COUNTER.fetch_add(1, Ordering::Relaxed);
    let label = format!("wiki3-{}", n);

    let title = format!("Wiki3 — {}", &url);

    let w = width.unwrap_or(1280.0);
    let h = height.unwrap_or(800.0);

    let mut builder = WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::External(parsed))
        .title(&title)
        .inner_size(w, h)
        .min_inner_size(800.0, 600.0);

    let (pos_x, pos_y) = if let (Some(sx), Some(sy)) = (x, y) {
        (sx, sy)
    } else if let Some(state) = app.try_state::<WindowStateManager>() {
        state.next_cascade_position()
    } else {
        (80.0, 60.0)
    };
    builder = builder.position(pos_x, pos_y);

    builder.build().map_err(|e| e.to_string())?;

    if let Some(state) = app.try_state::<WindowStateManager>() {
        state.record_window(
            label,
            WindowGeometry {
                url,
                x: pos_x,
                y: pos_y,
                width: w,
                height: h,
                wiki_id,
                closed: false,
            },
        );
    }

    Ok(())
}

// =============================================================================
// Per-wiki window management
// =============================================================================

#[command]
pub fn list_wiki_windows(app: AppHandle, wiki_id: String) -> Result<Vec<TrackedWindowInfo>, String> {
    let state = app.state::<WindowStateManager>();
    Ok(state.windows_for_wiki(&wiki_id))
}

#[command]
pub fn list_all_tracked_windows(app: AppHandle) -> Result<Vec<TrackedWindowInfo>, String> {
    let state = app.state::<WindowStateManager>();
    Ok(state.all_tracked())
}

#[command]
pub fn close_wiki_windows(app: AppHandle, wiki_id: String) -> Result<u32, String> {
    let state = app.state::<WindowStateManager>();
    let tracked = state.windows_for_wiki(&wiki_id);
    let mut closed_count = 0u32;
    for info in tracked {
        if info.closed {
            continue;
        }
        if let Some(w) = app.get_webview_window(&info.label) {
            state.mark_closed(&info.label);
            let _ = w.close();
            closed_count += 1;
        } else {
            state.mark_closed(&info.label);
        }
    }
    Ok(closed_count)
}

#[command]
pub fn reopen_wiki_windows(app: AppHandle, wiki_id: String) -> Result<u32, String> {
    let state = app.state::<WindowStateManager>();
    let tracked = state.windows_for_wiki(&wiki_id);
    let closed: Vec<TrackedWindowInfo> = tracked.into_iter().filter(|t| t.closed).collect();
    let mut reopened = 0u32;
    for t in closed {
        state.forget_window(&t.label);
        open_new_window_with_geometry(
            app.clone(),
            t.url,
            Some(t.x),
            Some(t.y),
            Some(t.width),
            Some(t.height),
            Some(wiki_id.clone()),
        )?;
        reopened += 1;
    }
    Ok(reopened)
}

#[command]
pub fn focus_window(app: AppHandle, label: String) -> Result<(), String> {
    let state = app.state::<WindowStateManager>();
    let tracked = state.all_tracked();
    let info = tracked
        .iter()
        .find(|t| t.label == label)
        .cloned()
        .ok_or_else(|| format!("Unknown window label: {label}"))?;

    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }

    state.forget_window(&label);
    open_new_window_with_geometry(
        app,
        info.url,
        Some(info.x),
        Some(info.y),
        Some(info.width),
        Some(info.height),
        info.wiki_id,
    )
}

#[command]
pub fn forget_tracked_window(app: AppHandle, label: String) -> Result<(), String> {
    let state = app.state::<WindowStateManager>();
    state.forget_window(&label);
    Ok(())
}

// =============================================================================
// Dashboard window lifecycle
// =============================================================================

#[command]
pub fn toggle_dashboard(app: AppHandle) -> Result<(), String> {
    show_dashboard_impl(&app, true)
}

#[command]
pub fn show_dashboard(app: AppHandle) -> Result<(), String> {
    show_dashboard_impl(&app, false)
}

fn show_dashboard_impl(app: &AppHandle, toggle: bool) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(DASHBOARD_LABEL) {
        if toggle && w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false) {
            let _ = w.hide();
            return Ok(());
        }
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }

    let state = app.try_state::<WindowStateManager>();
    let geom: Option<DashboardGeometry> = state.as_ref().and_then(|s| s.dashboard_geometry());

    let mut builder = WebviewWindowBuilder::new(
        app,
        DASHBOARD_LABEL,
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title("Wiki3 — Dashboard")
    .min_inner_size(800.0, 600.0);
    if let Some(g) = geom {
        builder = builder.inner_size(g.width, g.height).position(g.x, g.y);
    } else {
        builder = builder.inner_size(1100.0, 750.0);
    }
    builder.build().map_err(|e| e.to_string())?;
    Ok(())
}

// =============================================================================
// External URL / reveal
// =============================================================================

#[command]
pub fn open_external_url(url: String) -> Result<(), String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("Empty URL".into());
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(format!("Refusing to open non-http URL: {url}"));
    }
    open_os_default(url)
}

#[command]
pub fn reveal_path(path: String) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    if !p.exists() {
        return Err(format!("Path does not exist: {path}"));
    }
    open_os_default(&path)
}

#[cfg(target_os = "macos")]
fn open_os_default(arg: &str) -> Result<(), String> {
    std::process::Command::new("open")
        .arg(arg)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(target_os = "linux")]
fn open_os_default(arg: &str) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(arg)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(target_os = "windows")]
fn open_os_default(arg: &str) -> Result<(), String> {
    std::process::Command::new("cmd")
        .args(["/C", "start", "", arg])
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn open_os_default(_arg: &str) -> Result<(), String> {
    Err("Unsupported platform".into())
}

// =============================================================================
// Settings
// =============================================================================

#[command]
pub fn get_settings(app: AppHandle) -> Result<AppSettings, String> {
    let state = app.state::<WindowStateManager>();
    let settings = state.settings.lock().map_err(|e| e.to_string())?;
    Ok(settings.clone())
}

#[command]
pub fn update_settings(
    app: AppHandle,
    restore_windows: Option<bool>,
    default_repo_url: Option<String>,
    default_wikis_dir: Option<String>,
) -> Result<AppSettings, String> {
    let state = app.state::<WindowStateManager>();
    let mut settings = state.settings.lock().map_err(|e| e.to_string())?;
    if let Some(v) = restore_windows {
        settings.restore_windows = v;
    }
    if let Some(v) = default_repo_url {
        settings.default_repo_url = v;
    }
    if let Some(v) = default_wikis_dir {
        settings.default_wikis_dir = if v.trim().is_empty() { None } else { Some(v) };
    }
    let result = settings.clone();
    drop(settings);
    state.persist();
    Ok(result)
}
