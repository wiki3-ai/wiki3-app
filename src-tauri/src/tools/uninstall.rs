//! Uninstall: purge managed-tool installations from the Tauri
//! app-data dir. Wiki3 only touches its own `<app_data>/tools/`
//! subtree — the system `PATH` and anything outside that directory
//! are never modified.

use std::path::Path;

use super::{Result, TOOLS_DIR_NAME};

/// Remove a single tool's installation directory (all versions).
/// No-op if the directory doesn't exist.
pub fn uninstall_tool(tools_dir: &Path, tool_name: &str) -> Result<()> {
    let dir = tools_dir.join(tool_name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    Ok(())
}

/// Remove every managed tool. The caller passes the app-data root,
/// not the tools dir, to make it impossible to pass a path that
/// points outside our sandboxed subtree by accident.
pub fn uninstall_all(app_data: &Path) -> Result<()> {
    let dir = app_data.join(TOOLS_DIR_NAME);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uninstall_tool_removes_only_that_tool() {
        let tmp = tempfile::tempdir().unwrap();
        let tools = tmp.path().join("tools");
        std::fs::create_dir_all(tools.join("deno").join("2.4.5")).unwrap();
        std::fs::create_dir_all(tools.join("other").join("1.0.0")).unwrap();

        uninstall_tool(&tools, "deno").unwrap();
        assert!(!tools.join("deno").exists());
        assert!(tools.join("other").join("1.0.0").exists());
    }

    #[test]
    fn uninstall_tool_is_noop_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let tools = tmp.path().join("tools");
        std::fs::create_dir_all(&tools).unwrap();
        uninstall_tool(&tools, "deno").expect("no error for missing");
    }

    #[test]
    fn uninstall_all_removes_tools_dir_only() {
        let tmp = tempfile::tempdir().unwrap();
        let app_data = tmp.path();
        std::fs::create_dir_all(app_data.join("tools").join("deno")).unwrap();
        std::fs::create_dir_all(app_data.join("other_app_state")).unwrap();

        uninstall_all(app_data).unwrap();
        assert!(!app_data.join("tools").exists());
        assert!(
            app_data.join("other_app_state").exists(),
            "must not touch siblings of tools/"
        );
    }

    #[test]
    fn uninstall_all_is_noop_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        uninstall_all(tmp.path()).expect("no error when tools dir missing");
    }
}
