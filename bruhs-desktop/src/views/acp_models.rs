//! Model choices from installed ACP instances, shared by Settings and the composer.
use bruhs_core::agent_extensions::availability::ProviderAvailability;
use bruhs_models::agent::AgentProvider;
use bruhs_models::runtime_models::ModelChoice;

pub(super) struct Instance {
    pub id: String,
    pub name: String,
    pub accent: Option<u32>,
    pub choices: Vec<ModelChoice>,
    pub kiro_agents: Option<bruhs_core::agent_extensions::kiro::Agents>,
    device_enabled: bool,
}

impl Instance {
    pub fn enabled(&self, prefs: &ProviderAvailability) -> bool {
        prefs.acp_enabled_by_id(&self.id, self.device_enabled)
    }
}

pub(super) fn load() -> std::io::Result<Vec<Instance>> {
    Ok(bruhs_core::agent_extensions::load_config()?
        .acp
        .into_iter()
        .map(|agent| {
            let mut kiro_agents = None;
            let models = if agent.driver == Some(bruhs_models::agent_extensions::AcpDriver::Kiro) {
                let catalog = bruhs_core::agent_extensions::kiro::Catalog::load(&agent);
                let choices = catalog.choices(&agent);
                kiro_agents = Some(catalog.agents);
                choices
            } else {
                let ids = if agent.models.is_empty() {
                    vec!["default".to_owned()]
                } else {
                    agent.models
                };
                ids.into_iter().map(|id| (id.clone(), id)).collect()
            };
            let choices = models
                .into_iter()
                .map(|(id, label)| ModelChoice {
                    id: format!("acp/{}/{id}", agent.id),
                    label: format!("{} · {label}", agent.name),
                    provider: AgentProvider::Acp,
                })
                .collect();
            Instance {
                id: agent.id,
                name: agent.name,
                accent: agent
                    .accent_color
                    .as_deref()
                    .and_then(bruhs_models::agent_extensions::parse_accent_color),
                device_enabled: agent.enabled,
                choices,
                kiro_agents,
            }
        })
        .collect())
}
