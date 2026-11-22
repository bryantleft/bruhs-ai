//! Model IDs discovered from a named Kiro runtime, shared by the picker and chat host.
use bruhs_models::agent_extensions::AcpInstance;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, io, path::PathBuf};

#[derive(Clone, Serialize, Deserialize)]
pub struct Model {
    pub model_id: String,
    pub model_name: Option<String>,
}

pub use bruhs_models::agent_extensions::KiroSelection as Selection;
#[derive(Clone, Serialize, Deserialize)]
pub struct Agent {
    pub value: String,
    pub name: String,
    pub description: Option<String>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Agents {
    pub choices: Vec<Agent>,
    pub current: Option<String>,
}
impl Agents {
    pub(crate) fn from_template(template: &serde_json::Value) -> Self {
        let Some(mode) = template["configOptions"].as_array().and_then(|options| {
            options
                .iter()
                .find(|o| o["id"] == "mode" && o["type"] == "select")
        }) else {
            return Self::default();
        };
        let text = |v: &serde_json::Value, max: usize| {
            v.as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control))
                .map(str::to_owned)
        };
        let mut seen = HashSet::new();
        let choices = mode["options"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|o| {
                let value = text(&o["value"], 256)?;
                if !seen.insert(value.clone()) {
                    return None;
                }
                Some(Agent {
                    name: text(&o["name"], 256).unwrap_or_else(|| value.clone()),
                    value,
                    description: text(&o["description"], 1024),
                })
            })
            .take(128)
            .collect::<Vec<_>>();
        let current =
            text(&mode["currentValue"], 256).filter(|v| choices.iter().any(|a| &a.value == v));
        Self { choices, current }
    }
}

/// Kiro's model listing omits reasoning capabilities. The session confirms each advertised level before use.
pub fn effort_levels(model: &str) -> (&'static [&'static str], Option<&'static str>) {
    const CLAUDE: &[&str] = &["low", "medium", "high", "xhigh", "max"];
    match model {
        "claude-opus-4.6" | "claude-sonnet-4.6" => {
            (&["low", "medium", "high", "max"], Some("high"))
        }
        "claude-opus-4.7" => (CLAUDE, Some("xhigh")),
        "claude-opus-5.5" => (CLAUDE, Some("medium")),
        "claude-fable-5.1" | "claude-opus-5" | "claude-opus-4.8" | "claude-sonnet-5.5"
        | "claude-sonnet-5" => (CLAUDE, Some("high")),
        "gpt-5.6-sol" | "gpt-5.6-terra" | "gpt-5.6-luna" => (
            &["none", "low", "medium", "high", "xhigh", "max"],
            Some("high"),
        ),
        "kimi-k3" => (&["low", "high", "max"], Some("max")),
        _ => (&[], None),
    }
}
#[derive(Default, Serialize, Deserialize)]
pub struct Catalog {
    #[serde(default)]
    pub agents: Agents,
    pub default_model: Option<String>,
    pub models: Vec<Model>,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    configuration: String,
    catalog: Catalog,
}

fn cache(instance: &AcpInstance) -> io::Result<(PathBuf, String)> {
    let directory = super::directory()?.join("kiro-models");
    let id = Sha256::digest(instance.id.as_bytes());
    let config = Sha256::digest(serde_json::to_vec(instance)?);
    Ok((
        directory.join(format!("{id:x}.json")),
        format!("{config:x}"),
    ))
}

impl Catalog {
    pub fn default_id(&self) -> &str {
        self.default_model.as_deref().unwrap_or("auto")
    }

    pub fn choices(&self, instance: &AcpInstance) -> Vec<(String, String)> {
        let default_name = self
            .models
            .iter()
            .find(|m| m.model_id == self.default_id())
            .and_then(|m| m.model_name.as_deref())
            .unwrap_or("Kiro default");
        let mut choices = vec![("default".into(), default_name.to_owned())];
        let mut seen = HashSet::from(["default", self.default_id()]);
        for (id, name) in self
            .models
            .iter()
            .map(|m| {
                (
                    m.model_id.as_str(),
                    m.model_name.as_deref().unwrap_or(&m.model_id),
                )
            })
            .chain(instance.models.iter().map(|m| (m.as_str(), m.as_str())))
        {
            if seen.insert(id) {
                choices.push((id.into(), name.into()));
            }
        }
        choices
    }

    pub fn load(instance: &AcpInstance) -> Self {
        let read = || -> Option<Self> {
            let (path, configuration) = cache(instance).ok()?;
            let record: Saved = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
            (record.configuration == configuration).then_some(record.catalog)
        };
        read().unwrap_or_default()
    }

    pub(crate) fn parse(bytes: &str, instance: &AcpInstance) -> Option<Self> {
        let mut catalog: Self = serde_json::from_str(bytes.trim()).ok()?;
        let valid = |id: &str| {
            !id.is_empty()
                && !id.starts_with(['-', '/'])
                && format!("acp/{}/{id}", instance.id)
                    .parse::<bruhs_models::agent::ModelId>()
                    .is_ok()
        };
        catalog.models.retain(|m| valid(&m.model_id));
        catalog.default_model = catalog.default_model.filter(|id| valid(id));
        for model in &mut catalog.models {
            model.model_name = model
                .model_name
                .take()
                .filter(|label| !label.trim().is_empty() && !label.chars().any(char::is_control));
        }
        Some(catalog)
    }

    pub(crate) fn save(self, instance: &AcpInstance) -> io::Result<()> {
        let (path, configuration) = cache(instance)?;
        let directory = path.parent().expect("cache file has a parent");
        super::private_directory(directory)?;
        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        serde_json::to_writer(
            &mut file,
            &Saved {
                configuration,
                catalog: self,
            },
        )?;
        file.persist(path)?;
        Ok(())
    }
}
