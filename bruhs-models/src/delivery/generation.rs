//! Options offered by the bundled writer models. A selection belongs to its model.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionKind {
    Effort,
    ServiceTier,
    ContextWindow,
    FastMode,
    Ultracode,
    Thinking,
}

#[derive(Clone, Copy)]
pub struct ModelOption {
    pub kind: OptionKind,
    pub label: &'static str,
    pub default: &'static str,
    pub choices: &'static [(&'static str, &'static str)],
}

const LEVELS: &[(&str, &str)] = &[
    ("low", "Low"),
    ("medium", "Medium"),
    ("high", "High"),
    ("xhigh", "Extra High"),
    ("max", "Max"),
    ("ultra", "Ultra"),
];
const SONNET_LEVELS: &[(&str, &str)] = &[
    ("low", "Low"),
    ("medium", "Medium"),
    ("high", "High"),
    ("max", "Max"),
];
const TOGGLE: &[(&str, &str)] = &[("on", "On"), ("off", "Off")];

impl OptionKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Effort => "effort",
            Self::ServiceTier => "service_tier",
            Self::ContextWindow => "context_window",
            Self::FastMode => "fast_mode",
            Self::Ultracode => "ultracode",
            Self::Thinking => "thinking",
        }
    }
    fn descriptor(
        self,
        label: &'static str,
        default: &'static str,
        choices: &'static [(&'static str, &'static str)],
    ) -> ModelOption {
        ModelOption {
            kind: self,
            label,
            default,
            choices,
        }
    }
}

pub fn model_options(model: &str) -> Vec<ModelOption> {
    use OptionKind::*;
    let effort = |default, choices| Effort.descriptor("Reasoning", default, choices);
    let tier = ServiceTier.descriptor(
        "Service Tier",
        "default",
        &[("default", "Standard"), ("priority", "Fast")],
    );
    let ultra = Ultracode.descriptor("Ultracode", "off", TOGGLE);
    match model {
        "gpt-6.1-sol" => vec![effort("low", LEVELS), tier],
        "gpt-6-luna" => vec![effort("medium", &LEVELS[..5]), tier],
        "gpt-5.5" | "gpt-5.4" | "gpt-5.3-codex" => vec![effort("low", &LEVELS[..4])],
        "claude-fable-5" => vec![effort("medium", &LEVELS[..5]), ultra],
        "claude-opus-4-8" => vec![
            effort("high", &LEVELS[..5]),
            ultra,
            FastMode.descriptor("Fast Mode", "off", TOGGLE),
        ],
        "claude-sonnet-4-6" => vec![
            effort("high", SONNET_LEVELS),
            ContextWindow.descriptor("Context Window", "200k", &[("200k", "200k"), ("1m", "1M")]),
        ],
        "claude-haiku-4-5" => vec![Thinking.descriptor("Thinking", "on", TOGGLE)],
        _ => Vec::new(),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GenerationOptions(BTreeMap<OptionKind, String>);

impl GenerationOptions {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn effective<'a>(&'a self, option: &ModelOption) -> &'a str {
        self.0
            .get(&option.kind)
            .map(String::as_str)
            .unwrap_or(option.default)
    }
    pub fn value(&self, model: &str, kind: OptionKind) -> Option<&str> {
        model_options(model)
            .iter()
            .find(|option| option.kind == kind)
            .map(|option| self.effective(option))
    }
    pub fn set(&mut self, option: &ModelOption, value: &str) -> bool {
        if !option.choices.iter().any(|(id, _)| *id == value) {
            return false;
        }
        self.0.insert(option.kind, value.to_owned());
        true
    }
    pub fn valid_for(&self, model: &str) -> bool {
        let options = model_options(model);
        self.0.iter().all(|(key, value)| {
            options.iter().any(|option| {
                option.kind == *key && option.choices.iter().any(|(id, _)| id == value)
            })
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationTarget {
    Writer,
    ReviewGuide,
}

impl GenerationTarget {
    pub fn selection(
        self,
        prefs: &super::DeliveryPreferences,
    ) -> Option<(&str, &GenerationOptions)> {
        match self {
            Self::Writer => match &prefs.writer_model {
                super::WriterModel::Separate { model, options } => Some((model, options)),
                _ => None,
            },
            Self::ReviewGuide => match &prefs.review_guide_model {
                super::GuideModel::Separate { model, options } => Some((model, options)),
                _ => None,
            },
        }
    }
}
