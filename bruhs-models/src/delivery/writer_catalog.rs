//! The host's writer model choices and the capabilities of each installed CLI.
use super::generation::{model_options, GenerationOptions, OptionKind};
use crate::agent::{model_info, AgentProvider, ModelId, MODEL_CATALOG};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WriterChoice {
    pub value: String,
    pub label: String,
    pub description: Option<String>,
}
impl WriterChoice {
    pub fn new(value: &str, label: &str) -> Self {
        Self {
            value: value.to_owned(),
            label: label.to_owned(),
            description: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WriterOption {
    pub kind: OptionKind,
    pub default: String,
    pub choices: Vec<WriterChoice>,
}
impl WriterOption {
    pub fn label(&self) -> &'static str {
        match self.kind {
            OptionKind::Effort => "Reasoning",
            OptionKind::ServiceTier => "Service Tier",
            OptionKind::CyberAccessProgram => "Cyber Access",
            OptionKind::FastMode => "Fast Mode",
            OptionKind::Ultracode => "Ultracode",
            OptionKind::Thinking => "Thinking",
            OptionKind::ContextWindow => "Context Window",
        }
    }
    pub fn accepts(&self, value: &str) -> bool {
        self.choices.iter().any(|c| c.value == value)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WriterCliModel {
    pub id: String,
    pub label: String,
    pub provider: AgentProvider,
    pub options: Vec<WriterOption>,
    /// An older model that remains selectable through search or disclosure.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_legacy: bool,
}
impl WriterCliModel {
    pub fn resolve(&self, saved: &GenerationOptions) -> Option<GenerationOptions> {
        let mut result = GenerationOptions::default();
        if saved
            .iter()
            .any(|(k, v)| !self.options.iter().any(|o| o.kind == *k && o.accepts(v)))
        {
            return None;
        }
        for opt in &self.options {
            result.set_choice(opt, saved.configured(opt.kind).unwrap_or(&opt.default));
        }
        Some(result)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WriterCatalog {
    pub models: Vec<WriterCliModel>,
}

impl Default for WriterCatalog {
    fn default() -> Self {
        Self {
            models: MODEL_CATALOG
                .iter()
                .filter(|m| !m.provider.is_external())
                .map(|m| WriterCliModel {
                    id: m.id.to_owned(),
                    label: m.label.to_owned(),
                    provider: m.provider,
                    is_legacy: false,
                    options: model_options(m.id)
                        .into_iter()
                        .map(|o| WriterOption {
                            kind: o.kind,
                            default: o.default.into(),
                            choices: o
                                .choices
                                .iter()
                                .map(|(v, l)| {
                                    let mut c = WriterChoice::new(v, l);
                                    if *v == "priority" {
                                        c.description = Some("2x speed, increased usage".into());
                                    }
                                    c
                                })
                                .collect(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}
impl WriterCatalog {
    pub fn model(&self, id: &str) -> Option<&WriterCliModel> {
        let provider = AgentProvider::for_model(id);
        self.models
            .iter()
            .find(|m| m.provider == provider && provider.model_id(&m.id) == provider.model_id(id))
    }
}

/// Keep existing IDs readable. New IDs use the normal provider/model syntax
/// so that two providers may report the same wire ID without sharing a setting.
pub fn selection(provider: AgentProvider, wire: &str) -> Option<String> {
    if provider.is_external() {
        return None;
    }
    wire.parse::<ModelId>().ok()?;
    if model_info(wire).is_some_and(|m| m.provider == provider) {
        return Some(wire.to_owned());
    }
    format!("{}{wire}", provider.model_prefix())
        .parse::<ModelId>()
        .ok()
        .map(ModelId::into_inner)
}
pub fn cli_provider(id: &str) -> Option<AgentProvider> {
    if let Some(info) = model_info(id) {
        return (!info.provider.is_external()).then_some(info.provider);
    }
    id.parse::<ModelId>().ok()?;
    [AgentProvider::Claude, AgentProvider::Codex]
        .into_iter()
        .find(|p| id.starts_with(p.model_prefix()))
}
