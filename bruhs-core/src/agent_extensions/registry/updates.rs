//! Update private registry installations while retaining each instance's overrides.
use super::{catalog, prepare, Agent, Catalog, Prepared};
use crate::agent_extensions::{instances, load_config};
use bruhs_models::agent_extensions::{same_environment_name, AcpInstance, AcpVariable};
use semver::Version;
use std::{cmp::Ordering, io, path::Path};

#[derive(Clone)]
pub struct Installed {
    instance: AcpInstance,
    pub state: State,
}

#[derive(Clone)]
pub enum State {
    Installed,
    Current,
    Unavailable,
    Ahead,
    UnknownVersion,
    Update(Agent),
    Repair(Agent),
}

impl Installed {
    pub fn id(&self) -> &str {
        &self.instance.id
    }
    pub fn name(&self) -> &str {
        &self.instance.name
    }
    pub fn version(&self) -> &str {
        &self
            .instance
            .registry
            .as_ref()
            .expect("registry instance")
            .version
    }
    pub fn target(&self) -> Option<&Agent> {
        match &self.state {
            State::Update(target) | State::Repair(target) => Some(target),
            _ => None,
        }
    }
    /// Use the exact entry reviewed by the user. The final compare-and-swap
    /// also catches changes made in another window during package preparation.
    pub async fn apply(&self) -> io::Result<Self> {
        let agent = self
            .target()
            .ok_or_else(|| io::Error::other("Check this provider for updates first."))?;
        let installed = load_config()?
            .acp
            .into_iter()
            .find(|i| {
                i.id == self.id()
                    && i.command == self.instance.command
                    && i.registry == self.instance.registry
            })
            .ok_or_else(|| {
                io::Error::other("This installation changed. Check for updates again.")
            })?;
        let prepared = prepare(agent).await?;
        let next = upgraded(&installed, agent, prepared);
        instances::edit(Some(&installed), Some(&next))?;
        Ok(Self {
            instance: next,
            state: State::Current,
        })
    }
}

pub struct Report {
    pub installed: Vec<Installed>,
    pub notice: Option<String>,
}

pub async fn check(online: bool, refresh: bool) -> io::Result<Report> {
    let instances: Vec<_> = load_config()?
        .acp
        .into_iter()
        .filter(|i| i.registry.is_some())
        .collect();
    let mut notice = None;
    let catalog = if online && !instances.is_empty() {
        match catalog(refresh).await {
            Ok(c) => {
                if c.cached {
                    notice = Some("Showing the last available ACP Registry index.".into());
                }
                Some(c)
            }
            Err(e) => {
                notice = Some(format!("Could not check the ACP Registry. {e}"));
                None
            }
        }
    } else {
        None
    };
    Ok(Report {
        installed: instances
            .into_iter()
            .map(|instance| {
                let state = classify(&instance, catalog.as_ref());
                Installed { instance, state }
            })
            .collect(),
        notice,
    })
}

fn classify(instance: &AcpInstance, catalog: Option<&Catalog>) -> State {
    let Some(catalog) = catalog else {
        return State::Installed;
    };
    let origin = instance.registry.as_ref().expect("registry instance");
    let Some(agent) = catalog.agents.iter().find(|a| a.id == origin.agent_id) else {
        return State::Unavailable;
    };
    let version = |s: &str| Version::parse(s.trim_start_matches('v')).ok();
    let order = if origin.version == agent.version {
        Some(Ordering::Equal)
    } else {
        version(&agent.version)
            .zip(version(&origin.version))
            .map(|(a, b)| a.cmp_precedence(&b))
    };
    match order {
        Some(Ordering::Greater) => State::Update(agent.clone()),
        Some(Ordering::Less) => State::Ahead,
        None => State::UnknownVersion,
        Some(Ordering::Equal) if !executable(Path::new(&instance.command)) => {
            State::Repair(agent.clone())
        }
        Some(Ordering::Equal) => State::Current,
    }
}

fn executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return false;
        }
    }
    metadata.is_file()
}

fn upgraded(previous: &AcpInstance, agent: &Agent, prepared: Prepared) -> AcpInstance {
    let mut next = previous.clone();
    next.command = prepared.command.to_string_lossy().into_owned();
    let mut origin = agent.origin();
    if let Some(previous) = &previous.registry {
        origin
            .executable_override
            .clone_from(&previous.executable_override);
        origin.auth_method_id.clone_from(&previous.auth_method_id);
    }
    next.registry = Some(origin);
    if let Some(old) = previous
        .registry
        .as_ref()
        .and_then(|origin| origin.defaults.as_ref())
    {
        if next.args == old.args {
            next.args = prepared.args;
        }
        next.environment.retain(|name, variable| {
            let AcpVariable::Plain { value } = variable else {
                return true;
            };
            let previous = old
                .environment
                .iter()
                .find(|(k, _)| same_environment_name(k, name));
            if !previous.is_some_and(|(_, v)| v == value) {
                return true;
            }
            if let Some((_, new)) = prepared
                .environment
                .iter()
                .find(|(k, _)| same_environment_name(k, name))
            {
                value.clone_from(new);
                true
            } else {
                false
            }
        });
        for (name, value) in prepared.environment {
            if !old
                .environment
                .keys()
                .chain(next.environment.keys())
                .any(|k| same_environment_name(k, &name))
            {
                next.environment.insert(name, AcpVariable::Plain { value });
            }
        }
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn catalog(version: &str) -> Catalog {
        let data = json!({"agents":[{"id":"example","name":"Example","version":version,
            "distribution":{"npx":{"package":"example@2.0.0","args":["new"],
            "env":{"DEFAULT":"new", "OVERRIDE":"new","REMOVED":"new","SECRET":"new","ADDED":"new"}}}}]});
        Catalog {
            agents: super::super::parse(&serde_json::to_vec(&data).unwrap(), "test", true, false)
                .unwrap(),
            cached: false,
        }
    }

    fn instance() -> AcpInstance {
        serde_json::from_value(json!({"id":"work","name":"My agent","command":"absent","enabled":true,
            "accent_color":"#123456","modes":{"default":"ask"},"models":["custom"],
            "registry":{"agent_id":"example","version":"v1.2.3","distribution":"npm",
            "defaults":{"args":["old"],"environment":{"DEFAULT":"old","OVERRIDE":"old","REMOVED":"old","OBSOLETE":"old","SECRET":"old"}}},
            "args":["old"],"environment":{"DEFAULT":{"kind":"plain","value":"old"},"OVERRIDE":{"kind":"plain","value":"mine"},
            "OBSOLETE":{"kind":"plain","value":"old"},"SECRET":{"kind":"one_password","source":"work","reference":"op://vault/item/value"}}})).unwrap()
    }

    fn upgrade(i: &AcpInstance) -> AcpInstance {
        let agent = &catalog("2.0.0").agents[0];
        let launch = agent.origin().defaults.unwrap();
        upgraded(
            i,
            agent,
            Prepared {
                command: "new-launcher".into(),
                args: launch.args,
                environment: launch.environment,
            },
        )
    }

    #[test]
    fn registry_check_offline_missing_repair_and_semantic_versions() {
        let i = instance();
        assert!(matches!(classify(&i, None), State::Installed));
        assert!(matches!(
            classify(
                &i,
                Some(&Catalog {
                    agents: Vec::new(),
                    cached: false
                })
            ),
            State::Unavailable
        ));
        assert!(matches!(
            classify(&i, Some(&catalog("1.3.0-rc.1"))),
            State::Update(_)
        ));
        assert!(matches!(
            classify(&i, Some(&catalog("1.2.3-rc.1"))),
            State::Ahead
        ));
        assert!(matches!(
            classify(&i, Some(&catalog("next"))),
            State::UnknownVersion
        ));
        assert!(matches!(
            classify(&i, Some(&catalog("1.2.3"))),
            State::Repair(_)
        ));
        let mut i = i;
        i.command = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(matches!(
            classify(&i, Some(&catalog("1.2.3+build.5"))),
            State::Current
        ));
    }

    #[test]
    fn upgrade_merges_launch_defaults_and_preserves_custom_settings_and_secret_bindings() {
        let mut i = instance();
        let origin = i.registry.as_mut().unwrap();
        origin.executable_override = Some("custom-agent".into());
        origin.auth_method_id = Some("work".into());
        let n = upgrade(&i);
        assert_eq!(n.executable(), "custom-agent");
        assert_eq!(n.command, "new-launcher");
        assert_eq!(
            n.registry.as_ref().unwrap().auth_method_id.as_deref(),
            Some("work")
        );
        let plain = |s: &str| AcpVariable::Plain { value: s.into() };
        assert_eq!(n.args, ["new"]);
        assert_eq!(
            n.environment.keys().map(String::as_str).collect::<Vec<_>>(),
            ["DEFAULT", "OVERRIDE", "SECRET", "ADDED"]
        );
        assert_eq!(n.environment["DEFAULT"], plain("new"));
        assert_eq!(n.environment["OVERRIDE"], plain("mine"));
        assert_eq!(n.environment["SECRET"], i.environment["SECRET"]);
        assert_eq!(n.environment["ADDED"], plain("new"));
        assert_eq!(n.registry.unwrap().version, "2.0.0");
        assert_eq!(n.models, i.models);
        assert_eq!(n.modes, i.modes);
        assert_eq!(n.accent_color, i.accent_color);
        assert_eq!(n.enabled, i.enabled);
        let mut i = i;
        i.args = vec!["user flag".into()];
        assert_eq!(upgrade(&i).args, i.args);
        i.registry.as_mut().unwrap().defaults = None;
        let n = upgrade(&i);
        assert_eq!(n.args, i.args);
        assert_eq!(n.environment.as_slice(), i.environment.as_slice());
    }
}
