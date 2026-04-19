pub mod commands;
pub mod config;
pub mod git;
pub mod host;
pub mod permissions;
pub mod providers;
pub mod publishing_commands;
pub mod workspace;

use tauri::Manager;

use crate::host::DesktopHostState;
use crate::publishing_commands::PublishingState;

/// Build and configure the Tauri application.
///
/// This sets up the main window to load the configured wiki3.ai URL
/// with persistent webview state, and registers all desktop host commands.
pub fn run() {
    env_logger::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::default().build())
        .on_page_load(|webview, payload| {
            use tauri::webview::PageLoadEvent;
            if matches!(payload.event(), PageLoadEvent::Finished) {
                // Inject script to intercept target="_blank" links and window.open()
                // and route them through a Tauri command that creates new app windows
                // (WKWebView silently ignores new-window requests by default)
                let _ = webview.eval(r#"
                    (function() {
                        if (window.__wiki3NavHandler) return;
                        window.__wiki3NavHandler = true;

                        function openInNewWindow(href) {
                            if (window.__TAURI_INTERNALS__) {
                                window.__TAURI_INTERNALS__.invoke('open_new_window', { url: href });
                            }
                        }

                        // Intercept target="_blank" link clicks
                        document.addEventListener('click', function(e) {
                            var link = e.target.closest('a[target="_blank"], a[target="_new"]');
                            if (link && link.href) {
                                e.preventDefault();
                                e.stopPropagation();
                                openInNewWindow(link.href);
                            }
                        }, true);

                        // Override window.open to create new app windows for trusted origins
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
            let publishing_state = PublishingState::new(data_dir);

            // Get the effective URL to load
            let site_url = host_state
                .config
                .lock()
                .expect("Failed to lock config")
                .effective_url();

            log::info!("Loading site URL: {}", site_url);

            // Store the host state in Tauri's managed state
            app.manage(host_state);
            app.manage(publishing_state);

            // Navigate the main window to the wiki3.ai site
            if let Some(window) = app.get_webview_window("main") {
                // Navigate to the wiki3.ai URL
                let url: tauri::Url = site_url
                    .parse()
                    .expect("Failed to parse site URL");
                window
                    .navigate(url)
                    .expect("Failed to navigate to site URL");
            }

            Ok(())
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
