pub mod commands;
pub mod config;
pub mod host;
pub mod permissions;

use tauri::Manager;

use crate::host::DesktopHostState;

/// Build and configure the Tauri application.
///
/// This sets up the main window to load the configured wiki3.ai URL
/// with persistent webview state, and registers all desktop host commands.
pub fn run() {
    env_logger::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::default().build())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("Failed to resolve app data directory");

            log::info!("App data directory: {:?}", data_dir);

            // Initialize desktop host state
            let host_state = DesktopHostState::new(data_dir);

            // Get the effective URL to load
            let site_url = host_state
                .config
                .lock()
                .expect("Failed to lock config")
                .effective_url();

            log::info!("Loading site URL: {}", site_url);

            // Store the host state in Tauri's managed state
            app.manage(host_state);

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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
