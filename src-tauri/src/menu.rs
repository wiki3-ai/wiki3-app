//! Native application menu — File / View / Window / Help.
//!
//! File actions mirror the dashboard buttons (create from template,
//! clone, fork, open local repo, open site from URL). View actions
//! control dashboard visibility and per-wiki window groups.

use tauri::menu::{
    AboutMetadata, Menu, MenuBuilder, MenuEvent, MenuItemBuilder, PredefinedMenuItem,
    SubmenuBuilder,
};
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

// Menu item IDs (use stable string ids for dispatching).
pub const ID_NEW_FROM_TEMPLATE: &str = "wiki3.new_from_template";
pub const ID_CLONE_WIKI: &str = "wiki3.clone_wiki";
pub const ID_FORK_WIKI: &str = "wiki3.fork_wiki";
pub const ID_OPEN_LOCAL: &str = "wiki3.open_local";
pub const ID_OPEN_URL: &str = "wiki3.open_url";
pub const ID_SHOW_DASHBOARD: &str = "wiki3.show_dashboard";
pub const ID_CLOSE_ALL_WIKI_WINDOWS: &str = "wiki3.close_all_wiki_windows";
pub const ID_REOPEN_ALL_WIKI_WINDOWS: &str = "wiki3.reopen_all_wiki_windows";
pub const ID_HELP_README: &str = "wiki3.help_readme";
pub const ID_QUIT: &str = "wiki3.quit";
pub const ID_RELOAD: &str = "wiki3.reload";
pub const ID_FORCE_RELOAD: &str = "wiki3.force_reload";
pub const ID_BRING_ALL_TO_FRONT: &str = "wiki3.bring_all_to_front";

/// Build the application menu.
pub fn build_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    #[cfg(target_os = "macos")]
    let app_name = "Wiki3";

    // macOS application submenu.
    //
    // Note: we deliberately use a custom Quit MenuItem instead of
    // `PredefinedMenuItem::quit`. The predefined one routes through
    // NSApplication.terminate(_:), which bypasses Tauri's
    // `RunEvent::ExitRequested` handler — meaning `api.prevent_exit()`
    // can't hold the process open while we tear down preview
    // containers. Going through `app.exit(0)` instead fires the
    // event normally, so our shutdown hook in lib.rs runs.
    #[cfg(target_os = "macos")]
    let quit_item = MenuItemBuilder::with_id(ID_QUIT, format!("Quit {app_name}"))
        .accelerator("CmdOrCtrl+Q")
        .build(app)?;
    // Repo URL surfaced in the About box. We deliberately do NOT
    // pass it via `AboutMetadata.credits`: muda passes that string
    // to AppKit as a plain (non-link) NSAttributedString, so the URL
    // would render as dead text. Instead we ship `Credits.html` in
    // the app bundle's Resources directory; macOS's standard About
    // panel auto-loads that file and renders its <a href> as a
    // clickable link. (`website` is ignored on macOS by AppKit, but
    // we still set it for Windows/Linux clickable-link support.)
    #[cfg(target_os = "macos")]
    const REPO_URL: &str = "https://github.com/wiki3-ai/wiki3-app";
    // The default window icon Tauri loads is 32×32 — far too small
    // for the macOS About panel (which renders the icon at ~96pt).
    // Bundle the 256×256 PNG at compile time and pass it through so
    // the dialog has a properly-sized icon.
    #[cfg(target_os = "macos")]
    const ABOUT_ICON_PNG: &[u8] = include_bytes!("../icons/128x128@2x.png");
    #[cfg(target_os = "macos")]
    let about_icon = tauri::image::Image::from_bytes(ABOUT_ICON_PNG).ok();
    #[cfg(target_os = "macos")]
    let app_submenu = SubmenuBuilder::new(app, app_name)
        .about(Some(AboutMetadata {
            name: Some(app_name.to_string()),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
            icon: about_icon,
            website: Some(REPO_URL.to_string()),
            website_label: Some("github.com/wiki3-ai/wiki3-app".to_string()),
            ..Default::default()
        }))
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .item(&quit_item)
        .build()?;

    // File menu
    let new_from_template =
        MenuItemBuilder::with_id(ID_NEW_FROM_TEMPLATE, "New Wiki from Template…")
            .accelerator("CmdOrCtrl+N")
            .build(app)?;
    let clone_wiki = MenuItemBuilder::with_id(ID_CLONE_WIKI, "Clone Wiki…")
        .accelerator("CmdOrCtrl+Shift+O")
        .build(app)?;
    let fork_wiki = MenuItemBuilder::with_id(ID_FORK_WIKI, "Fork Wiki…").build(app)?;
    let open_local = MenuItemBuilder::with_id(ID_OPEN_LOCAL, "Open Local Repo…").build(app)?;
    let open_url = MenuItemBuilder::with_id(ID_OPEN_URL, "Open Site from URL…")
        .accelerator("CmdOrCtrl+L")
        .build(app)?;

    let file_menu = SubmenuBuilder::new(app, "File")
        .item(&new_from_template)
        .item(&clone_wiki)
        .item(&fork_wiki)
        .item(&open_local)
        .separator()
        .item(&open_url)
        .separator()
        .item(&PredefinedMenuItem::close_window(
            app,
            Some("Close Window"),
        )?)
        .build()?;

    // Edit (standard items)
    let edit_menu = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;

    // View
    let show_dashboard = MenuItemBuilder::with_id(ID_SHOW_DASHBOARD, "Show Dashboard")
        .accelerator("CmdOrCtrl+0")
        .build(app)?;
    let close_all = MenuItemBuilder::with_id(
        ID_CLOSE_ALL_WIKI_WINDOWS,
        "Close All Windows for Active Wiki",
    )
    .build(app)?;
    let reopen_all = MenuItemBuilder::with_id(
        ID_REOPEN_ALL_WIKI_WINDOWS,
        "Reopen All Windows for Active Wiki",
    )
    .build(app)?;

    let view_menu = SubmenuBuilder::new(app, "View")
        .item(&show_dashboard)
        .separator()
        .item(&close_all)
        .item(&reopen_all)
        .separator()
        .item(
            &MenuItemBuilder::with_id(ID_RELOAD, "Reload")
                .accelerator("CmdOrCtrl+R")
                .build(app)?,
        )
        .item(
            &MenuItemBuilder::with_id(ID_FORCE_RELOAD, "Force Reload")
                .accelerator("CmdOrCtrl+Shift+R")
                .build(app)?,
        )
        .build()?;

    // Window — standard macOS items.
    let window_menu = SubmenuBuilder::new(app, "Window")
        .minimize()
        .maximize()
        .separator()
        .fullscreen()
        .separator()
        .item(&MenuItemBuilder::with_id(ID_BRING_ALL_TO_FRONT, "Bring All to Front").build(app)?)
        .build()?;

    // Help
    let help_readme = MenuItemBuilder::with_id(ID_HELP_README, "Wiki3 on GitHub").build(app)?;
    let help_menu = SubmenuBuilder::new(app, "Help")
        .item(&help_readme)
        .build()?;

    #[cfg(target_os = "macos")]
    let menu = MenuBuilder::new(app)
        .item(&app_submenu)
        .item(&file_menu)
        .item(&edit_menu)
        .item(&view_menu)
        .item(&window_menu)
        .item(&help_menu)
        .build()?;

    #[cfg(not(target_os = "macos"))]
    let menu = MenuBuilder::new(app)
        .item(&file_menu)
        .item(&edit_menu)
        .item(&view_menu)
        .item(&window_menu)
        .item(&help_menu)
        .build()?;

    Ok(menu)
}

/// Handle a menu event. For most items, we emit an event to the dashboard
/// frontend which is better positioned to show the relevant dialogs.
pub fn handle_menu_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    match event.id().0.as_str() {
        ID_SHOW_DASHBOARD => {
            show_dashboard_generic(app);
            let _ = app.emit("wiki3://menu", ID_SHOW_DASHBOARD.to_string());
        }
        ID_HELP_README => {
            let _ = open_external_url_generic("https://github.com/wiki3-ai/wiki3-app");
        }
        ID_QUIT => {
            // Goes through `RunEvent::ExitRequested`, so the
            // shutdown hook in lib.rs runs and we can stop preview
            // containers before the process actually exits.
            app.exit(0);
        }
        ID_RELOAD | ID_FORCE_RELOAD => {
            // Reload the focused window (or the dashboard as a
            // fallback). Tauri's `reload` ignores HTTP/asset caches
            // already, so Cmd-R and Cmd-Shift-R behave identically;
            // we expose both because users expect Cmd-Shift-R to
            // exist.
            let win = app
                .webview_windows()
                .into_values()
                .find(|w| w.is_focused().unwrap_or(false))
                .or_else(|| app.get_webview_window(crate::commands::DASHBOARD_LABEL));
            if let Some(w) = win {
                let _ = w.eval("window.location.reload()");
            }
        }
        ID_BRING_ALL_TO_FRONT => {
            // Show every Wiki3 window and focus them. Mirrors
            // AppKit's standard Window → Bring All to Front.
            for (_, w) in app.webview_windows() {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }
        ID_NEW_FROM_TEMPLATE
        | ID_CLONE_WIKI
        | ID_FORK_WIKI
        | ID_OPEN_LOCAL
        | ID_OPEN_URL
        | ID_CLOSE_ALL_WIKI_WINDOWS
        | ID_REOPEN_ALL_WIKI_WINDOWS => {
            // Forward to the dashboard window (bring it to the front first)
            show_dashboard_generic(app);
            let _ = app.emit("wiki3://menu", event.id().0.clone());
        }
        _ => {}
    }
}

/// Generic "show dashboard" usable without the concrete AppHandle<Wry>.
fn show_dashboard_generic<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window(crate::commands::DASHBOARD_LABEL) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let mut builder = WebviewWindowBuilder::new(
        app,
        crate::commands::DASHBOARD_LABEL,
        WebviewUrl::App("index.html".into()),
    )
    .title("Wiki3 — Dashboard")
    .min_inner_size(800.0, 600.0)
    .inner_size(1100.0, 750.0);
    if let Some(state) = app.try_state::<crate::window_state::WindowStateManager>() {
        if let Some(g) = state.dashboard_geometry() {
            builder = builder.inner_size(g.width, g.height).position(g.x, g.y);
        }
    }
    let _ = builder.build();
}

fn open_external_url_generic(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = url;
        Err("Unsupported platform".into())
    }
}
