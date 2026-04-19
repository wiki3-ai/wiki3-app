pub mod commands;
pub mod config;
pub mod git;
pub mod host;
pub mod permissions;
pub mod providers;
pub mod publishing_commands;
pub mod window_state;
pub mod workspace;

use tauri::Manager;

use crate::host::DesktopHostState;
use crate::publishing_commands::PublishingState;
use crate::window_state::WindowStateManager;

/// Build and configure the Tauri application.
///
/// The main window loads the local dashboard UI (index.html).
/// Site windows (wiki3.ai) are opened in separate windows on demand,
/// and restored from the previous session if the setting is enabled.
pub fn run() {
    env_logger::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .on_page_load(|webview, payload| {
            use tauri::webview::PageLoadEvent;
            if matches!(payload.event(), PageLoadEvent::Finished) {
                // Inject navigation handler for wiki3.ai site windows
                // (WKWebView silently ignores new-window requests by default)
                let url = payload.url().to_string();
                let is_site_window = url.contains("wiki3.ai");
                if is_site_window {
                    let _ = webview.eval(r#"
                        (function() {
                            if (window.__wiki3NavHandler) return;
                            window.__wiki3NavHandler = true;

                            function openInNewWindow(href) {
                                if (window.__TAURI_INTERNALS__) {
                                    window.__TAURI_INTERNALS__.invoke('open_new_window', { url: href });
                                }
                            }

                            document.addEventListener('click', function(e) {
                                var link = e.target.closest('a[target="_blank"], a[target="_new"]');
                                if (link && link.href) {
                                    e.preventDefault();
                                    e.stopPropagation();
                                    openInNewWindow(link.href);
                                }
                            }, true);

                            var _open = window.open;
                            window.open = function(url) {
                                if (url) {
                                    try {
                                        var u = new URL(url, window.location.href);
                                        if (u.origin === 'https://wiki3.ai' || u.origin === 'https://www.wiki3.ai') {
                                            openInNewWindow(u.href);
                                            return null;
                                        }
                                    } catch(e) {}
                                }
                                return _open.apply(window, arguments);
                            };
                        })();
                    "#);
                }
            }
        })
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("Failed to resolve app data directory");

            log::info!("App data directory: {:?}", data_dir);

            // Initialize desktop host state
            let host_state = DesktopHostState::new(data_dir.clone());

            // Initialize publishing state
            let publishing_state = PublishingState::new(data_dir.clone());

            // Initialize window state manager (persists open windows + settings)
            let window_state = WindowStateManager::new(data_dir);

            // Store states in Tauri's managed state
            app.manage(host_state);
            app.manage(publishing_state);
            app.manage(window_state);

            // Restore site windows from the previous session
            let ws = app.state::<WindowStateManager>();
            if ws.should_restore() {
                let saved = ws.saved_windows();
                if !saved.is_empty() {
                    log::info!("Restoring {} window(s) from previous session", saved.len());
                    let handle = app.handle().clone();
                    for geom in saved {
                        if let Err(e) = crate::commands::open_new_window_with_geometry(
                            handle.clone(),
                            geom.url,
                            Some(geom.x),
                            Some(geom.y),
                            Some(geom.width),
                            Some(geom.height),
                        ) {
                            log::warn!("Failed to restore window: {}", e);
                        }
                    }
                }
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            let label = window.label().to_string();
            if !label.starts_with("wiki3-") {
                return;
            }
            let state = match window.app_handle().try_state::<WindowStateManager>() {
                Some(s) => s,
                None => return,
            };
            match event {
                tauri::WindowEvent::Destroyed => {
                    state.remove_window(&label);
                }
                tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
                    // Persist current position and size
                    if let (Ok(pos), Ok(size)) = (window.outer_position(), window.inner_size()) {
                        state.update_window_geometry(
                            &label,
                            pos.x as f64,
                            pos.y as f64,
                            size.width as f64,
                            size.height as f64,
                        );
                    }
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::detect_desktop_host,
            commands::get_permission_state,
            commands::set_execution_permission,
            commands::get_execution_state,
            commands::get_app_config,
            commands::open_new_window,
            publishing_commands::store_github_token,
            publishing_commands::get_auth_status,
            publishing_commands::clear_github_auth,
            publishing_commands::list_workspaces,
            publishing_commands::get_workspace,
            publishing_commands::remove_workspace,
            publishing_commands::create_site_from_template,
            publishing_commands::fork_site,
            publishing_commands::get_git_status,
            publishing_commands::commit_changes,
            publishing_commands::push_changes,
            publishing_commands::commit_and_push,
            publishing_commands::publish_site,
            publishing_commands::detect_workspace_publish_mode,
            publishing_commands::open_local_workspace,
            publishing_commands::open_repo_site,
            commands::get_settings,
            commands::update_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
