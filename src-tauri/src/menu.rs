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

/// Build the application menu.
pub fn build_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    #[cfg(target_os = "macos")]
    let app_name = "Wiki3";

    // macOS application submenu.
    #[cfg(target_os = "macos")]
    let app_submenu = SubmenuBuilder::new(app, app_name)
        .about(Some(AboutMetadata {
            name: Some(app_name.to_string()),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
            ..Default::default()
        }))
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;

    // File menu
    let new_from_template = MenuItemBuilder::with_id(ID_NEW_FROM_TEMPLATE, "New Wiki from Template…")
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
        .item(&PredefinedMenuItem::close_window(app, Some("Close Window"))?)
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
        .build()?;

    // Window
    let window_menu = SubmenuBuilder::new(app, "Window")
        .minimize()
        .maximize()
        .separator()
        .fullscreen()
        .build()?;

    // Help
    let help_readme = MenuItemBuilder::with_id(ID_HELP_README, "Wiki3 on GitHub").build(app)?;
    let help_menu = SubmenuBuilder::new(app, "Help").item(&help_readme).build()?;

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
