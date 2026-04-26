pub mod commands;
pub mod config;
pub mod git;
pub mod host;
pub mod menu;
pub mod permissions;
pub mod providers;
pub mod publishing_commands;
pub mod tools;
pub mod wiki;
pub mod window_state;
pub mod workspace;

use tauri::Manager;

use crate::host::DesktopHostState;
use crate::publishing_commands::PublishingState;
use crate::wiki::commands::WikiState;
use crate::window_state::WindowStateManager;

/// Build and configure the Tauri application.
pub fn run() {
    env_logger::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .on_page_load(|webview, payload| {
            use tauri::webview::PageLoadEvent;
            if matches!(payload.event(), PageLoadEvent::Finished) {
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

            let host_state = DesktopHostState::new(data_dir.clone());
            let publishing_state = PublishingState::new(data_dir.clone());
            let window_state = WindowStateManager::new(data_dir.clone());
            let wiki_state = WikiState::new(data_dir.clone());

            // Apple Container detection state (no bundled tools).
            let tools_state = crate::tools::ToolsState::new();

            // One-time seed + migrate wikis from the legacy workspaces file.
            if let Err(e) = wiki_state
                .manager
                .init(Some(&publishing_state.workspace_manager))
            {
                log::warn!("Wiki state init failed: {}", e);
            }

            app.manage(host_state);
            app.manage(publishing_state);
            app.manage(window_state);
            app.manage(wiki_state);
            app.manage(tools_state);
            app.manage(crate::wiki::local_site::LocalSiteManager::new());

            // Autostart per-wiki preview containers for any wikis with
            // `autostart_container = true` and a still-existing local
            // path. Spawn each on the Tokio runtime so app launch is
            // not blocked by container/image setup.
            {
                let wiki_state = app.state::<WikiState>();
                if let Ok(wikis) = wiki_state.manager.list() {
                    for w in wikis
                        .into_iter()
                        .filter(|w| w.autostart_container)
                    {
                        let Some(path) = w
                            .local_path
                            .as_ref()
                            .map(std::path::PathBuf::from)
                            .filter(|p| p.exists())
                        else {
                            continue;
                        };
                        let handle = app.handle().clone();
                        let id = w.id.clone();
                        tauri::async_runtime::spawn(async move {
                            let mgr = handle.state::<crate::wiki::local_site::LocalSiteManager>();
                            if let Err(e) = crate::wiki::local_site::start_site(
                                &handle, mgr.inner(), &id, &path,
                            )
                            .await
                            {
                                log::warn!("autostart container for wiki {id} failed: {e}");
                            }
                        });
                    }
                }
            }

            // Install the native menu.
            match crate::menu::build_menu(app.handle()) {
                Ok(menu) => {
                    if let Err(e) = app.set_menu(menu) {
                        log::warn!("Failed to set menu: {}", e);
                    }
                }
                Err(e) => {
                    log::warn!("Failed to build menu: {}", e);
                }
            }
            let handle_for_menu = app.handle().clone();
            app.on_menu_event(move |_app, event| {
                crate::menu::handle_menu_event(&handle_for_menu, event);
            });

            // Apply saved dashboard geometry, if any.
            let ws = app.state::<WindowStateManager>();
            if let Some(g) = ws.dashboard_geometry() {
                if let Some(win) = app.get_webview_window(crate::commands::DASHBOARD_LABEL) {
                    let _ = win.set_position(tauri::PhysicalPosition::new(g.x, g.y));
                    let _ = win.set_size(tauri::PhysicalSize::new(g.width, g.height));
                }
            }

            // Restore site windows from the previous session.
            if ws.should_restore() {
                let saved = ws.saved_open_windows();
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
                            geom.wiki_id,
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
            let state = match window.app_handle().try_state::<WindowStateManager>() {
                Some(s) => s,
                None => return,
            };

            // Dashboard: track its own geometry separately.
            if label == crate::commands::DASHBOARD_LABEL {
                if let tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) = event {
                    if let (Ok(pos), Ok(size)) = (window.outer_position(), window.inner_size()) {
                        state.update_dashboard_geometry(
                            pos.x as f64,
                            pos.y as f64,
                            size.width as f64,
                            size.height as f64,
                        );
                    }
                }
                return;
            }

            if !label.starts_with("wiki3-") {
                return;
            }
            match event {
                tauri::WindowEvent::Destroyed => {
                    // Keep the entry around for reopen if it had a wiki owner.
                    let had_owner = state
                        .all_tracked()
                        .into_iter()
                        .find(|t| t.label == label)
                        .and_then(|t| t.wiki_id)
                        .is_some();
                    state.on_window_destroyed(&label, had_owner);
                }
                tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
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
            // Desktop host / permissions
            commands::detect_desktop_host,
            commands::get_permission_state,
            commands::set_execution_permission,
            commands::get_execution_state,
            commands::get_app_config,
            // Site window management
            commands::open_new_window,
            commands::open_new_window_for_wiki,
            commands::list_wiki_windows,
            commands::list_all_tracked_windows,
            commands::close_wiki_windows,
            commands::reopen_wiki_windows,
            commands::focus_window,
            commands::forget_tracked_window,
            // Dashboard lifecycle
            commands::toggle_dashboard,
            commands::show_dashboard,
            // External
            commands::open_external_url,
            commands::reveal_path,
            // Settings
            commands::get_settings,
            commands::update_settings,
            // Publishing (unchanged)
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
            // Wikis
            wiki::commands::list_wikis,
            wiki::commands::get_wiki,
            wiki::commands::add_wiki,
            wiki::commands::update_wiki,
            wiki::commands::remove_wiki,
            wiki::commands::reorder_wikis,
            wiki::commands::set_wiki_publish_on_commit,
            wiki::commands::set_wiki_autostart_container,
            wiki::commands::restore_default_wikis,
            wiki::commands::get_default_wikis_dir,
            wiki::commands::is_empty_dir,
            wiki::commands::open_wiki_site,
            wiki::commands::open_wiki_remote,
            wiki::commands::reveal_wiki_local,
            wiki::commands::open_local_repo_as_wiki,
            wiki::commands::clone_wiki,
            // Per-wiki git + publish
            wiki::git_commands::wiki_git_status,
            wiki::git_commands::wiki_commit,
            wiki::git_commands::wiki_push,
            wiki::git_commands::wiki_pull,
            wiki::git_commands::wiki_publish,
            wiki::git_commands::wiki_commit_and_maybe_publish,
            wiki::git_commands::wiki_build_site,
            // Local site preview (serve + watch inside Apple Container)
            wiki::local_site::wiki_start_container,
            wiki::local_site::wiki_stop_container,
            wiki::local_site::wiki_container_status,
            wiki::local_site::wiki_force_stop_container_service,
            // Managed tools: Apple Container is the only external
            // dependency, and we only detect it (never install it).
            tools::commands::detect_apple_container,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = &event {
                // Block the exit just long enough to stop any
                // containers we started and (optionally) the Apple
                // Container service. If foreign containers remain,
                // ask the user before stopping the service.
                let manager =
                    match app_handle.try_state::<crate::wiki::local_site::LocalSiteManager>() {
                        Some(m) => m,
                        None => return,
                    };
                if !manager.has_pending_cleanup() {
                    return;
                }

                api.prevent_exit();

                let handle = app_handle.clone();
                tauri::async_runtime::spawn(async move {
                    let manager =
                        handle.state::<crate::wiki::local_site::LocalSiteManager>();
                    let report =
                        crate::wiki::local_site::shutdown_all(manager.inner()).await;

                    if !report.foreign_containers.is_empty()
                        && report.service_started_by_us
                        && !report.service_stopped
                    {
                        use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
                        let list = report.foreign_containers.join("\n  • ");
                        let msg = format!(
                            "Wiki3 started the Apple Container service this session.\n\n\
                             The following containers were not started by Wiki3 and are \
                             still running:\n  • {list}\n\n\
                             Stop the container service anyway? (This will also stop those \
                             containers.)"
                        );
                        let stop_anyway = handle
                            .dialog()
                            .message(msg)
                            .title("Stop Apple Container service?")
                            .kind(MessageDialogKind::Warning)
                            .buttons(MessageDialogButtons::OkCancelCustom(
                                "Stop anyway".into(),
                                "Leave running".into(),
                            ))
                            .blocking_show();
                        if stop_anyway {
                            if let Err(e) = crate::wiki::local_site::force_stop_service(
                                handle.state::<crate::wiki::local_site::LocalSiteManager>()
                                    .inner(),
                            )
                            .await
                            {
                                log::warn!("force_stop_service failed: {e}");
                            }
                        }
                    }
                    for e in &report.errors {
                        log::warn!("shutdown: {e}");
                    }

                    handle.exit(0);
                });
            }
        });
}
