//! Individual edits from the desktop provider editor share the Workbench lock.
use super::{environment, load_config_at, lock_config};
use bruhs_models::agent_extensions::AcpInstance;
use std::{io, path::Path};

/// Compare the instance shown in the editor with its current value before
/// saving. Changes to other agents and prices are retained.
pub fn edit(expected: Option<&AcpInstance>, next: Option<&AcpInstance>) -> io::Result<()> {
    edit_with_values(expected, next, Default::default())
}

pub fn edit_with_values(
    expected: Option<&AcpInstance>,
    next: Option<&AcpInstance>,
    values: environment::DeviceValues,
) -> io::Result<()> {
    edit_at(&super::directory()?, expected, next, values)
}

fn edit_at(
    root: &Path,
    expected: Option<&AcpInstance>,
    next: Option<&AcpInstance>,
    values: environment::DeviceValues,
) -> io::Result<()> {
    let id = expected
        .or(next)
        .ok_or_else(|| io::Error::other("Choose an ACP instance."))?
        .id
        .as_str();
    if let Some(agent) = next {
        if agent.id != id {
            return Err(io::Error::other("An existing instance ID cannot change."));
        }
        agent.validate().map_err(io::Error::other)?;
    }
    let _lock = lock_config(root)?;
    let previous = load_config_at(root)?;
    let mut config = previous.clone();
    let index = config.acp.iter().position(|agent| agent.id == id);
    let current = index.map(|index| &config.acp[index]);
    // IndexMap equality ignores order, but the order of the editor rows is saved.
    let reordered = current
        .zip(expected)
        .is_some_and(|(a, b)| a.environment.as_slice() != b.environment.as_slice());
    if current != expected || reordered {
        let message = if expected.is_none() {
            "That instance ID is already in use. Choose another ID."
        } else {
            "This instance changed in another editor. Reopen it to use the current settings."
        };
        return Err(io::Error::other(message));
    }
    if let (None, Some(origin)) = (index, next.and_then(|a| a.registry.as_ref())) {
        if config
            .acp
            .iter()
            .filter_map(|a| a.registry.as_ref())
            .any(|o| o.agent_id == origin.agent_id)
        {
            return Err(io::Error::other(
                "This registry agent has already been added.",
            ));
        }
    }
    match (index, next) {
        (Some(index), Some(agent)) => config.acp[index] = agent.clone(),
        (None, Some(agent)) => config.acp.push(agent.clone()),
        (Some(index), None) => {
            config.acp.remove(index);
        }
        (None, None) => unreachable!(),
    }
    environment::persist(root, &previous, &config, values)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(
        root: &Path,
        expected: Option<&AcpInstance>,
        next: Option<&AcpInstance>,
    ) -> io::Result<()> {
        edit_at(root, expected, next, Default::default())
    }

    #[test]
    fn two_registry_additions_with_different_instance_ids_share_one_origin() {
        let dir = tempfile::tempdir().unwrap();
        let first: AcpInstance=serde_json::from_value(serde_json::json!({"id":"one","name":"One","command":"agent","registry":{"agent_id":"example","version":"1.0.0","distribution":"npm"}})).unwrap();
        let second = AcpInstance {
            id: "two".into(),
            ..first.clone()
        };
        edit(dir.path(), None, Some(&first)).unwrap();
        assert!(edit(dir.path(), None, Some(&second))
            .unwrap_err()
            .to_string()
            .contains("already been added"));
        edit(dir.path(), Some(&first), None).unwrap();
        edit(dir.path(), None, Some(&second)).unwrap();
    }

    #[test]
    fn stale_editor_cannot_overwrite_variable_order() {
        let root = tempfile::tempdir().unwrap();
        let original: AcpInstance=serde_json::from_str(r#"{"id":"work","name":"Work","command":"agent","environment":{"B":{"kind":"plain","value":""},"A":{"kind":"plain","value":""}}}"#).unwrap();
        let mut changed = original.clone();
        changed.environment.swap_indices(0, 1);
        let config = bruhs_models::agent_extensions::ExtensionConfig {
            acp: vec![changed],
            ..Default::default()
        };
        super::super::write_config(root.path(), &config).unwrap();
        assert!(edit(root.path(), Some(&original), None).is_err());
        assert_eq!(
            load_config_at(root.path()).unwrap().acp[0]
                .environment
                .get_index(0)
                .unwrap()
                .0,
            "A"
        );
    }

    #[test]
    fn edits_preserve_peers_and_reject_stale_or_conflicting_changes() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        let instance = |id: &str| {
            serde_json::from_value::<AcpInstance>(serde_json::json!({
            "id":id,"name":"Work","command":"tool","args":["literal argument",""],"enabled":false,
            "models":["sample"],"mode_config_id":"behavior","modes":{"plan":"readonly"}
        })).unwrap()
        };
        let start = instance("first");
        let second = instance("second");
        edit(root, None, Some(&start)).unwrap();
        edit(root, None, Some(&second)).unwrap();
        assert!(edit(root, None, Some(&start)).is_err());
        assert!(edit(root, Some(&start), Some(&second)).is_err());
        let changed = AcpInstance {
            name: "Renamed".into(),
            ..start.clone()
        };
        edit(root, Some(&start), Some(&changed)).unwrap();
        assert!(edit(root, Some(&start), None).is_err());
        assert_eq!(
            load_config_at(root).unwrap().acp,
            [changed.clone(), second.clone()]
        );
        edit(root, Some(&changed), None).unwrap();
        assert_eq!(load_config_at(root).unwrap().acp, [second]);
    }
}
