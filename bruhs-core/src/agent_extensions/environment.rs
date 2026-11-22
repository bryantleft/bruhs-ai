//! Provider-scoped process variables. Device secrets never enter agents.json.
use super::{load_config_at, lock_config, write_config};
use bruhs_models::agent_extensions::{
    same_environment_name, AcpInstance, AcpVariable, ExtensionConfig,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::Path,
};
use uuid::Uuid;
use zeroize::Zeroizing;

pub type DeviceValues = BTreeMap<Uuid, Zeroizing<String>>;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct DeviceKey {
    instance: String,
    variable: String,
    revision: Uuid,
}

impl DeviceKey {
    fn entry(&self) -> io::Result<keyring::Entry> {
        keyring::Entry::new(
            "bruhs-ai-acp-environment",
            &format!("{}/{}/{}", self.instance, self.variable, self.revision),
        )
        .map_err(|_| key_error())
    }

    fn remove(&self) -> io::Result<()> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(key_error()),
        }
    }
}

fn key_error() -> io::Error {
    io::Error::other("Could not access the ACP variable in the system credential store.")
}

fn keys(config: &ExtensionConfig) -> BTreeSet<DeviceKey> {
    config
        .acp
        .iter()
        .flat_map(|agent| {
            agent.environment.iter().filter_map(|(name, value)| {
                let AcpVariable::Device { revision } = value else {
                    return None;
                };
                Some(DeviceKey {
                    instance: agent.id.clone(),
                    variable: name.clone(),
                    revision: *revision,
                })
            })
        })
        .collect()
}

// Journal both newly staged and retired credentials before touching the keychain.
// After a crash the committed configuration decides which keys are still live.
fn journal(root: &Path, entries: &BTreeSet<DeviceKey>) -> io::Result<()> {
    let mut temp = tempfile::NamedTempFile::new_in(root)?;
    serde_json::to_writer(&mut temp, entries)?;
    temp.as_file().sync_all()?;
    temp.persist(root.join("acp-key-journal.json"))?;
    Ok(())
}

fn cleanup(root: &Path, pending: &mut BTreeSet<DeviceKey>, live: &BTreeSet<DeviceKey>) {
    pending.retain(|key| !live.contains(key) && key.remove().is_err());
    if journal(root, pending).is_err() || !pending.is_empty() {
        tracing::warn!("ACP credential cleanup will retry on the next configuration edit");
    }
}

/// Caller holds agents.lock. Check ownership before accepting any keychain handle
/// from a portable config and write replacement values under new identities.
pub(super) fn persist(
    root: &Path,
    previous: &ExtensionConfig,
    next: &ExtensionConfig,
    mut values: DeviceValues,
) -> io::Result<()> {
    let previous_keys = keys(previous);
    let next_keys = keys(next);
    let added = next_keys.difference(&previous_keys).collect::<Vec<_>>();
    if added.len() != values.len()
        || added.iter().any(|key| !values.contains_key(&key.revision))
        || values.values().any(|v| v.contains('\0'))
    {
        return Err(io::Error::other(
            "Enter a new value for each new or renamed device variable.",
        ));
    }
    let mut sources = None;
    for agent in &next.acp {
        let previous = previous.acp.iter().find(|p| p.id == agent.id);
        for (name, variable) in &agent.environment {
            if previous.and_then(|p| p.environment.get(name)) == Some(variable) {
                continue;
            }
            if sources.is_none() {
                sources = Some(crate::secret_sources::configured()?);
            }
            let settings = sources.as_ref().unwrap();
            let target = format!("acp/{}", agent.id);
            if settings
                .bindings
                .iter()
                .any(|b| b.target == target && same_environment_name(&b.variable, name))
            {
                return Err(io::Error::other(format!(
                    "{name} is managed in Secrets. Choose another name."
                )));
            }
            if let AcpVariable::OnePassword { source, reference } = variable {
                crate::secret_sources::validate_reference(reference)?;
                if !settings.sources.contains_key(source) {
                    return Err(io::Error::other("Choose an available 1Password source."));
                }
            }
        }
    }
    let retired = previous_keys.difference(&next_keys);
    let mut pending: BTreeSet<DeviceKey> = match fs::read(root.join("acp-key-journal.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => BTreeSet::new(),
        Err(e) => return Err(e),
    };
    pending.extend(retired.cloned());
    pending.extend(added.iter().map(|k| (*k).clone()));
    if pending.is_empty() {
        return write_config(root, next);
    }
    journal(root, &pending)?;
    let result = (|| {
        for key in added {
            let value = values.remove(&key.revision).ok_or_else(key_error)?;
            key.entry()?.set_password(&value).map_err(|_| key_error())?;
        }
        write_config(root, next)
    })();
    cleanup(
        root,
        &mut pending,
        if result.is_ok() {
            &next_keys
        } else {
            &previous_keys
        },
    );
    result
}

pub(crate) async fn apply(
    cmd: &mut tokio::process::Command,
    agent: &AcpInstance,
    managed: &[(String, String)],
) -> io::Result<()> {
    for (name, variable) in &agent.environment {
        if managed
            .iter()
            .any(|(key, _)| same_environment_name(key, name))
        {
            continue;
        }
        match variable {
            AcpVariable::Plain { value } => {
                cmd.env(name, value);
            }
            AcpVariable::Device { revision } => {
                let key = DeviceKey {
                    instance: agent.id.clone(),
                    variable: name.clone(),
                    revision: *revision,
                };
                let secret = tokio::task::spawn_blocking(move || {
                    key.entry()?
                        .get_password()
                        .map(Zeroizing::new)
                        .map_err(|_| key_error())
                })
                .await
                .map_err(io::Error::other)??;
                cmd.env(name, secret.as_str());
            }
            AcpVariable::OnePassword { source, reference } => {
                let field = crate::secret_sources::SecretReference {
                    source: source.clone(),
                    reference: reference.clone(),
                };
                let secret = field.read().await?;
                cmd.env(name, secret.as_str());
            }
        }
    }
    for (name, value) in managed {
        cmd.env(name, value);
    }
    Ok(())
}

pub fn source_consumers(id: &str) -> io::Result<Vec<String>> {
    let mut uses = Vec::new();
    for agent in super::load_config()?.acp {
        for (name, v) in &agent.environment {
            if matches!(v,AcpVariable::OnePassword{source,..} if source==id) {
                uses.push(format!("{} · {name}", agent.name));
            }
        }
    }
    Ok(uses)
}

pub(crate) fn unset_source(id: &str) -> io::Result<()> {
    unset_source_at(&super::directory()?, id)
}

fn unset_source_at(root: &Path, id: &str) -> io::Result<()> {
    if !root.try_exists()? {
        return Ok(());
    }
    let _lock = lock_config(root)?;
    let mut config = load_config_at(root)?;
    let mut changed = false;
    for agent in &mut config.acp {
        let len = agent.environment.len();
        agent
            .environment
            .retain(|_, v| !matches!(v,AcpVariable::OnePassword{source,..} if source==id));
        changed |= len != agent.environment.len();
    }
    if changed {
        write_config(root, &config)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use bruhs_models::agent_extensions::AcpEnvironment;

    fn instance() -> AcpInstance {
        serde_json::from_value(serde_json::json!({"id":"work","name":"Work","command":"agent"}))
            .unwrap()
    }

    #[test]
    fn portable_config_cannot_copy_or_rename_a_saved_device_key() {
        let root = tempfile::tempdir().unwrap();
        let mut original = instance();
        let key = AcpVariable::Device {
            revision: Uuid::new_v4(),
        };
        original.environment.insert("TOKEN".into(), key.clone());
        let before = ExtensionConfig {
            acp: vec![original],
            ..Default::default()
        };
        write_config(root.path(), &before).unwrap();
        for new_name in ["TOKEN", "RENAMED_TOKEN"] {
            let mut after = before.clone();
            if new_name == "TOKEN" {
                after.acp[0].id = "peer".into();
            }
            after.acp[0].environment = AcpEnvironment::from([(new_name.into(), key.clone())]);
            assert!(persist(root.path(), &before, &after, Default::default()).is_err());
            assert_eq!(load_config_at(root.path()).unwrap().acp, before.acp);
        }
    }

    #[test]
    fn removing_a_source_preserves_other_variables_and_drops_its_references() {
        let root = tempfile::tempdir().unwrap();
        let mut agent = instance();
        let field = |id: &str| AcpVariable::OnePassword {
            source: id.into(),
            reference: "op://vault/item/field".into(),
        };
        agent.environment = AcpEnvironment::from([
            ("A".into(), field("old")),
            ("B".into(), field("other")),
            ("C".into(), AcpVariable::Plain { value: "".into() }),
        ]);
        write_config(
            root.path(),
            &ExtensionConfig {
                acp: vec![agent.clone()],
                ..Default::default()
            },
        )
        .unwrap();
        unset_source_at(root.path(), "old").unwrap();
        unset_source_at(root.path(), "old").unwrap();
        agent.environment.shift_remove("A");
        assert_eq!(load_config_at(root.path()).unwrap().acp, [agent]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn variables_apply_to_one_child_and_managed_secrets_take_precedence() {
        let mut instance = instance();
        instance.environment = AcpEnvironment::from([
            (
                "PROFILE".into(),
                AcpVariable::Plain {
                    value: "  $HOME ; test  ".into(),
                },
            ),
            (
                "EMPTY".into(),
                AcpVariable::Plain {
                    value: String::new(),
                },
            ),
            (
                "TOKEN".into(),
                AcpVariable::Device {
                    revision: Uuid::new_v4(),
                },
            ),
        ]);
        let mut cmd = tokio::process::Command::new("/usr/bin/env");
        cmd.env_clear();
        apply(&mut cmd, &instance, &[("TOKEN".into(), "managed".into())])
            .await
            .unwrap();
        let out = cmd.output().await.unwrap();
        assert!(out.status.success());
        let output = String::from_utf8(out.stdout).unwrap();
        let actual = output.lines().collect::<BTreeSet<_>>();
        assert_eq!(
            actual,
            BTreeSet::from(["PROFILE=  $HOME ; test  ", "EMPTY=", "TOKEN=managed"])
        );
        let mut peer = tokio::process::Command::new("/usr/bin/env");
        peer.env_clear();
        instance.environment.clear();
        apply(&mut peer, &instance, &[]).await.unwrap();
        assert!(peer.output().await.unwrap().stdout.is_empty());
    }
}
