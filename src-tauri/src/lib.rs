pub mod commands;
pub mod commands_devcontainer;
pub mod config;
pub mod diagnostics;
pub mod git;
pub mod host;
pub mod menu;
pub mod permissions;
pub mod providers;
pub mod publishing_commands;
pub mod runtime_commands;
pub mod tauri_sink;
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
            // Probe for `git` upfront. Wiki3 shells out to git for
            // clone/status/commit/push; without the Xcode Command
            // Line Tools the user would hit a confusing
            // `No such file or directory` partway through an action.
            // Surface it as a native modal with a one-click Install.
            check_git_or_prompt(app.handle());

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
            // devcontainer-core state for the new container controls
            // (start/stop/restart/rebuild/remove). Lives alongside the
            // legacy `LocalSiteManager` for now — callers can pick.
            let registry = devcontainer_core::RuntimeRegistry::with_default_backends();
            // Honour the runtime the user pinned last session. This has to
            // happen before the registry is managed (and so before anything
            // can call `resolve()`), or the first operation after launch
            // would quietly use the availability default instead.
            runtime_commands::restore_persisted_choice(
                &registry,
                app.state::<WindowStateManager>()
                    .container_runtime()
                    .as_deref(),
            );
            app.manage(registry);
            // Lazy-started internal caching proxy. We bind on a port
            // distinct from Devcontainers.app (31280) so both can run
            // concurrently and each see their own per-host stats.
            // Bind failure is cached as "disabled" — containers still
            // launch, they just don't get an auto-injected proxy.
            let proxy_bind: std::net::SocketAddr = "192.168.64.1:31281"
                .parse()
                .expect("static proxy bind addr is well-formed");
            app.manage(devcontainer_core::LifecycleOrchestrator::with_proxy(
                devcontainer_core::ProxyManager::with_bind(proxy_bind),
            ));

            // Autostart of per-wiki containers deliberately does NOT happen
            // here. Starting one needs the parsed devcontainer, and parsing
            // lives in the frontend engine bundle; the old Rust path worked
            // around that by driving the Apple Container CLI directly, so
            // with Docker or Podman selected it could only fail. The dashboard
            // now runs it through the same submit-then-up sequence as the
            // Start button (`autostartContainers()` in `src/main.ts`).

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
            // Generic per-wiki container controls (devcontainer-core).
            wiki::container_controls::wiki_container_ctl_status,
            wiki::container_controls::wiki_container_ctl_up,
            wiki::container_controls::wiki_container_ctl_stop,
            wiki::container_controls::wiki_container_ctl_restart,
            wiki::container_controls::wiki_container_ctl_rebuild,
            wiki::container_controls::wiki_container_ctl_remove,
            wiki::container_controls::wiki_container_ctl_cancel,
            wiki::ports::wiki_container_ports,
            // Runtime selection (global — one engine serves every wiki).
            runtime_commands::runtime_list,
            runtime_commands::runtime_select,
            runtime_commands::runtime_use_auto,
            // Devcontainer engine bridge — fs sandbox + parsed-config
            // submission, called by the JS engine bundle running in
            // the WebView (see `src/devcontainer-engine.ts`).
            commands_devcontainer::fs_is_file,
            commands_devcontainer::fs_read_file,
            commands_devcontainer::fs_write_file,
            commands_devcontainer::fs_read_dir,
            commands_devcontainer::fs_mkdirp,
            commands_devcontainer::submit_parsed_devcontainer,
            commands_devcontainer::wiki_proxy_stats,
            // Managed tools: Apple Container is the only external
            // dependency, and we only detect it (never install it).
            tools::commands::detect_apple_container,
            tools::commands::detect_git,
            diagnostics::run_diagnostic_report,
            open_url,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, _event| {
            // Quit needs no teardown of its own.
            //
            // Exit used to be intercepted to stop the per-wiki containers and
            // optionally the Apple Container service, with a dialog about
            // "foreign" containers. That machinery belonged to the legacy
            // Apple `Serve` path: it only ran when `LocalSiteManager` had
            // pending cleanup, which nothing has populated since the container
            // controls moved onto `devcontainer-core`. Containers started
            // through the orchestrator are left running deliberately, so the
            // next launch can adopt them rather than paying for a rebuild.
        });
}

/// Probe for `git`. If absent, show a native modal offering one-click
/// Install (runs `xcode-select --install`) or Quit. Blocks setup
/// until the user picks; on Install we exit so the user can relaunch
/// once the system installer finishes.
fn check_git_or_prompt(app: &tauri::AppHandle) {
    if crate::tools::git_probe::detect().installed {
        return;
    }
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    let install = app
        .dialog()
        .message(
            "Wiki3 needs `git`, which is part of the macOS Command Line \
             Tools.\n\n\
             Click Install to launch Apple's installer. After it \
             finishes, relaunch Wiki3.",
        )
        .title("Git Required")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Install".into(),
            "Quit".into(),
        ))
        .blocking_show();
    if install {
        // `xcode-select --install` returns immediately and pops the
        // system installer GUI. Don't wait — just spawn and exit.
        let _ = std::process::Command::new("xcode-select")
            .arg("--install")
            .spawn();
    }
    // Either way, quit. The user must relaunch after install.
    std::process::exit(0);
}

/// Open a URL in the user's default browser. Used by the in-app
/// tools dialog so the "apple/container" link is actually clickable
/// from within the WebView (where `<a target="_blank">` is a no-op).
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    // Validate scheme — accept http(s) only to avoid being a generic
    // shell-out vector.
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("only http(s) URLs are allowed".into());
    }
    std::process::Command::new("/usr/bin/open")
        .arg(&url)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}
