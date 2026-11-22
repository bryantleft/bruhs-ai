//! Host preferences for built-in runtimes. One marker per disabled runtime lets
//! independent desktop and server processes change different switches atomically.
mod projects;
use bruhs_models::agent::AgentProvider;
use std::sync::Arc;
use std::{collections::HashSet, fs, io, path::Path};

#[derive(Clone, Debug, Default)]
pub struct ProviderAvailability {
    disabled: HashSet<AgentProvider>,
    projects: Arc<projects::Projects>,
    effective_overrides: projects::Switches,
    local_overrides: projects::Switches,
}

impl ProviderAvailability {
    pub fn load() -> io::Result<Self> {
        Self::load_at(&super::directory()?)
    }

    fn load_at(root: &Path) -> io::Result<Self> {
        let mut prefs = Self::default();
        match fs::read_dir(root.join("disabled-providers")) {
            Ok(files) => {
                for file in files {
                    let name = file?.file_name();
                    if let Some(provider) = name.to_str().and_then(|name| name.parse().ok()) {
                        if provider != AgentProvider::Acp {
                            prefs.disabled.insert(provider);
                        }
                    }
                }
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
        prefs.projects = Arc::new(projects::load(root)?);
        Ok(prefs)
    }

    pub fn enabled(&self, provider: AgentProvider) -> bool {
        self.effective_overrides
            .get(provider.as_str())
            .copied()
            .unwrap_or_else(|| self.environment_enabled(provider))
    }

    pub fn environment_enabled(&self, provider: AgentProvider) -> bool {
        !self.disabled.contains(&provider)
    }

    pub fn for_project(&self, project: &Path) -> Self {
        let (effective_overrides, local_overrides) = projects::resolve(&self.projects, project);
        Self {
            disabled: self.disabled.clone(),
            projects: self.projects.clone(),
            effective_overrides,
            local_overrides,
        }
    }

    pub fn load_for_project(project: &Path) -> io::Result<Self> {
        Ok(Self::load()?.for_project(project))
    }

    pub fn acp_enabled(&self, instance: &bruhs_models::agent_extensions::AcpInstance) -> bool {
        self.acp_enabled_by_id(&instance.id, instance.enabled)
    }

    pub fn acp_enabled_by_id(&self, id: &str, environment_enabled: bool) -> bool {
        self.effective_overrides
            .get(&format!("acp/{id}"))
            .copied()
            .unwrap_or(environment_enabled)
    }

    pub fn project_override(&self, id: &str) -> Option<bool> {
        self.local_overrides.get(id).copied()
    }

    pub fn set_project_override(project: &Path, id: &str, enabled: Option<bool>) -> io::Result<()> {
        projects::save(&super::directory()?, project, id, enabled)
    }

    pub fn set_enabled(provider: AgentProvider, enabled: bool) -> io::Result<()> {
        Self::set_at(&super::directory()?, provider, enabled)
    }

    fn set_at(root: &Path, provider: AgentProvider, enabled: bool) -> io::Result<()> {
        if provider == AgentProvider::Acp {
            return Err(io::Error::other("Choose an individual ACP instance."));
        }
        let directory = root.join("disabled-providers");
        let path = directory.join(provider.as_str());
        if enabled {
            match fs::remove_file(path) {
                Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
                result => result,
            }
        } else {
            super::private_directory(&directory)?;
            match fs::File::create_new(path) {
                Ok(_) => Ok(()),
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => Ok(()),
                Err(err) => Err(err),
            }
        }
    }
}

/// Check immediately before launching a new provider turn, including turns
/// initiated by a different process from the one that changed the setting.
pub fn require_enabled(provider: AgentProvider) -> Result<(), String> {
    require_enabled_for_project(provider, Path::new(""))
}

pub fn require_enabled_for_project(provider: AgentProvider, project: &Path) -> Result<(), String> {
    let prefs = ProviderAvailability::load_for_project(project)
        .map_err(|err| format!("Could not read provider settings: {err}"))?;
    if prefs.enabled(provider) {
        Ok(())
    } else {
        Err(format!(
            "{} is disabled. Enable it in Settings → Providers & updates.",
            provider.label()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_rejects_disabled_turn_before_launch() {
        use crate::agent::{AgentOrchestrator, AgentTurn, SendMessageError};
        use bruhs_models::{agent::PermissionMode, ids::ThreadId};
        if std::env::var_os("BRUHS_TEST_DISABLED_TURN").is_some() {
            let host = AgentOrchestrator::new(Default::default());
            let turn = AgentTurn::new(
                ThreadId::new_v4(),
                "fixture",
                ".",
                "grok/test",
                AgentProvider::Grok,
                PermissionMode::Plan,
            );
            assert!(
                matches!(host.send_message(turn, None), Err(SendMessageError::ProviderUnavailable(message)) if message.contains("Grok is disabled"))
            );
            return;
        }
        let root = tempfile::tempdir().unwrap();
        ProviderAvailability::set_at(root.path(), AgentProvider::Grok, false).unwrap();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "agent_extensions::availability::tests::host_rejects_disabled_turn_before_launch",
            ])
            .env("BRUHS_WORKBENCH_DIR", root.path())
            .env("BRUHS_TEST_DISABLED_TURN", "1")
            .status()
            .unwrap();
        assert!(result.success());
    }

    #[test]
    fn switches_survive_reload_and_independent_updates() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        let (a, b) = (AgentProvider::Claude, AgentProvider::Codex);
        assert!(ProviderAvailability::load_at(root).unwrap().enabled(a));
        std::thread::scope(|threads| {
            for provider in [a, b, a] {
                threads.spawn(move || ProviderAvailability::set_at(root, provider, false).unwrap());
            }
        });
        let prefs = ProviderAvailability::load_at(root).unwrap();
        assert!(!prefs.enabled(a) && !prefs.enabled(b));
        ProviderAvailability::set_at(root, a, true).unwrap();
        ProviderAvailability::set_at(root, a, true).unwrap();
        let prefs = ProviderAvailability::load_at(root).unwrap();
        assert!(prefs.enabled(a) && !prefs.enabled(b) && prefs.enabled(AgentProvider::Acp));
    }
}
