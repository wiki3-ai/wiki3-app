//! Runtime selection surface.
//!
//! The engine chooses a container runtime by availability — Docker, then
//! Podman, then Apple Containers (see `devcontainer_core::DEFAULT_PREFERENCE`)
//! — but the user can pin one instead. These commands expose both facts to the
//! dashboard: what is installed, what is actually in effect, and the ability
//! to override it.
//!
//! The choice is **global**, not per-wiki: one container engine serves every
//! wiki, so there is one control for the whole app rather than a setting on
//! each card.
//!
//! One caveat worth knowing: the registry memoises its automatic choice, so
//! installing a runtime mid-session will not be picked up until the selection
//! is cleared (or the app restarts). That is deliberate — the alternative is
//! probing three engines on every operation.

use devcontainer_core::{RuntimeId, RuntimeRegistry, DEFAULT_PREFERENCE};
use serde::Serialize;
use tauri::{command, State};

/// One runtime, as the dashboard needs to see it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
    /// Serialised kebab-case: `docker`, `podman`, `apple-containers`.
    pub id: RuntimeId,
    /// Human label for the picker.
    pub label: String,
    pub available: bool,
    pub version: Option<String>,
    /// Why it is unavailable, when it is.
    pub reason: Option<String>,
    /// The user has pinned this runtime explicitly.
    pub selected: bool,
    /// This is the runtime operations will actually use.
    pub effective: bool,
}

/// Display name for a runtime.
fn label(id: RuntimeId) -> &'static str {
    match id {
        RuntimeId::Docker => "Docker",
        RuntimeId::Podman => "Podman",
        RuntimeId::AppleContainers => "Apple Containers",
    }
}

/// Every runtime id in the order the engine prefers them, so the picker
/// matches the automatic policy instead of a hash-map iteration order.
fn ordered_ids() -> Vec<RuntimeId> {
    let mut ids = DEFAULT_PREFERENCE.to_vec();
    for id in [
        RuntimeId::Docker,
        RuntimeId::Podman,
        RuntimeId::AppleContainers,
    ] {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids
}

/// List every known runtime with its availability and selection state.
#[command]
pub async fn runtime_list(
    registry: State<'_, RuntimeRegistry>,
) -> Result<Vec<RuntimeInfo>, String> {
    let explicit = registry.selected_id();
    // `resolve()` is what operations will use, so this is the truthful
    // "effective" answer rather than a second guess at the policy.
    let effective = registry.resolve().await.id();

    let mut out = Vec::with_capacity(DEFAULT_PREFERENCE.len());
    for id in ordered_ids() {
        let Some(runtime) = registry.get(id) else {
            continue;
        };
        // Probe each one individually: `resolve()` stops at the first
        // available runtime, so anything after it in preference order was
        // never asked, and the picker needs to show all of them.
        let (available, version, reason) = match runtime.probe().await {
            Ok(a) => (a.available, a.version, a.reason),
            Err(e) => (false, None, Some(e.to_string())),
        };
        out.push(RuntimeInfo {
            id,
            label: label(id).to_string(),
            available,
            version,
            reason,
            selected: explicit == Some(id),
            effective: effective == id,
        });
    }
    Ok(out)
}

/// Pin a runtime explicitly, overriding the availability policy.
#[command]
pub async fn runtime_select(
    registry: State<'_, RuntimeRegistry>,
    id: RuntimeId,
) -> Result<(), String> {
    // Pinning an unavailable runtime is deliberately allowed: the user may be
    // about to start Docker Desktop or the Podman machine, and the failure
    // they get names the thing that is missing. Refusing here would be a
    // worse experience than letting them choose and telling them why.
    registry.select(id).map_err(|e| e.to_string())
}

/// Drop the explicit choice and let availability decide again.
#[command]
pub async fn runtime_use_auto(registry: State<'_, RuntimeRegistry>) -> Result<(), String> {
    registry.clear_selection();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_ids_follows_the_engine_preference() {
        // The picker's order must match the policy, so a user reading the list
        // top-to-bottom sees what "Automatic" would pick first.
        assert_eq!(
            ordered_ids(),
            vec![
                RuntimeId::Docker,
                RuntimeId::Podman,
                RuntimeId::AppleContainers
            ]
        );
    }

    #[test]
    fn every_runtime_has_a_human_label() {
        for id in ordered_ids() {
            assert!(!label(id).is_empty(), "{id:?} has no label");
        }
    }

    #[test]
    fn runtime_ids_round_trip_through_the_wire_format() {
        // The dashboard sends an id back verbatim in `runtime_select`, and
        // renders whatever `runtime_list` reports. If these spellings drift,
        // selecting a runtime fails with a serde error at runtime rather than
        // at build time — so pin them.
        assert_eq!(
            serde_json::to_string(&RuntimeId::Docker).unwrap(),
            "\"docker\""
        );
        assert_eq!(
            serde_json::to_string(&RuntimeId::Podman).unwrap(),
            "\"podman\""
        );
        assert_eq!(
            serde_json::to_string(&RuntimeId::AppleContainers).unwrap(),
            "\"apple-containers\""
        );

        for id in ordered_ids() {
            let wire = serde_json::to_string(&id).unwrap();
            let back: RuntimeId = serde_json::from_str(&wire).expect("id must deserialize");
            assert_eq!(back, id, "{wire} did not round-trip");
        }
    }
}
