//! Apple Container detection + lifecycle helpers.
//!
//! This module is now a thin re-export of
//! `devcontainer_core::container::apple_containers` — the canonical
//! implementation lives in the shared `devcontainer-core` crate so
//! it can be reused by other Tauri apps.

pub use devcontainer_core::container::apple_containers::{
    detect, ensure_service_running, find_container_by_mount_source, inspect_container_ipv4,
    is_service_running, list_running_container_names, probe_with_dirs,
    stop_container_by_name as stop_container, stop_service, AppleContainerStatus,
};
