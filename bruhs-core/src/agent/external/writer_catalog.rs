//! Inspect CLI metadata without sending a model turn or opening a project.
use super::AgentProvider;
use bruhs_models::delivery::{
    generation::OptionKind,
    writer_catalog::{selection, WriterCatalog, WriterChoice, WriterCliModel, WriterOption},
};
use serde_json::Value;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

#[derive(Default)]
struct Listing {
    models: Vec<WriterCliModel>,
    next: Option<Instant>,
}
static CLAUDE: Mutex<Listing> = Mutex::const_new(Listing {
    models: Vec::new(),
    next: None,
});
static CODEX: Mutex<Listing> = Mutex::const_new(Listing {
    models: Vec::new(),
    next: None,
});

pub async fn writer_catalog() -> WriterCatalog {
    writer_catalog_for_project(std::path::Path::new("")).await
}

pub async fn writer_catalog_for_project(project: &std::path::Path) -> WriterCatalog {
    let Ok(prefs) =
        crate::agent_extensions::availability::ProviderAvailability::load_for_project(project)
    else {
        return WriterCatalog { models: Vec::new() };
    };
    let (claude, codex) = tokio::join!(
        listed(AgentProvider::Claude, &CLAUDE, &prefs),
        listed(AgentProvider::Codex, &CODEX, &prefs)
    );
    let mut bundled = WriterCatalog::default().models;
    bundled.retain(|m| prefs.enabled(m.provider));
    if !codex.is_empty() {
        bundled.retain(|m| m.provider != AgentProvider::Codex);
    }
    let mut models = codex;
    models.extend(claude);
    bundled.retain(|m| !models.iter().any(|found| found.id == m.id));
    models.extend(bundled);
    WriterCatalog { models }
}

async fn listed(
    provider: AgentProvider,
    state: &Mutex<Listing>,
    prefs: &crate::agent_extensions::availability::ProviderAvailability,
) -> Vec<WriterCliModel> {
    if !prefs.enabled(provider) {
        return Vec::new();
    }
    let mut state = state.lock().await;
    if state.next.is_none_or(|time| time <= Instant::now()) {
        let found = tokio::time::timeout(Duration::from_secs(6), probe(provider))
            .await
            .ok()
            .flatten();
        let retry = if let Some(found) = found {
            state.models = found;
            600
        } else {
            30
        };
        state.next = Some(Instant::now() + Duration::from_secs(retry));
    }
    state.models.clone()
}

async fn probe(provider: AgentProvider) -> Option<Vec<WriterCliModel>> {
    parse(provider, &super::metadata::model_list(provider).await?)
}

fn choices(rows: &Value, id: &str, label: &str) -> Vec<WriterChoice> {
    let mut choices = Vec::new();
    for row in rows.as_array().into_iter().flatten().take(64) {
        let Some(value) = row.as_str().or_else(|| row[id].as_str()) else {
            continue;
        };
        if value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
            || choices.iter().any(|c: &WriterChoice| c.value == value)
        {
            continue;
        }
        let title = row[label].as_str().filter(|s| !s.is_empty()).map_or_else(
            || match value {
                "xhigh" => "Extra High".into(),
                s => {
                    let mut label = s.to_owned();
                    label[..1].make_ascii_uppercase();
                    label
                }
            },
            str::to_owned,
        );
        let mut choice = WriterChoice::new(value, &title.chars().take(160).collect::<String>());
        choice.description = row["description"]
            .as_str()
            .map(|s| s.chars().take(320).collect());
        choices.push(choice);
    }
    choices
}

fn parse(provider: AgentProvider, data: &Value) -> Option<Vec<WriterCliModel>> {
    use OptionKind::*;
    let mut models = Vec::new();
    let bundled = WriterCatalog::default();
    for r in data["models"].as_array()?.iter().take(512) {
        let (wire, label) = if provider == AgentProvider::Codex {
            if r["visibility"].as_str().is_some_and(|s| s != "list") {
                continue;
            }
            (r["slug"].as_str(), r["display_name"].as_str())
        } else {
            if r["value"] == "default" {
                continue;
            }
            (r["value"].as_str(), r["displayName"].as_str())
        };
        let Some(wire) = wire else { continue };
        // Resolved Claude aliases keep their identity when the provider rolls an alias forward.
        let resolved = r["resolvedModel"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or(wire);
        let wire = if wire.ends_with("[1m]") && !resolved.ends_with("[1m]") {
            format!("{resolved}[1m]")
        } else {
            resolved.to_owned()
        };
        let Some(id) = selection(provider, &wire) else {
            continue;
        };
        if models.iter().any(|m: &WriterCliModel| m.id == id) {
            continue;
        }
        let mut options = Vec::new();
        if provider == AgentProvider::Codex {
            let levels = choices(&r["supported_reasoning_levels"], "effort", "label");
            if let Some(first) = levels.first() {
                let default = r["default_reasoning_level"]
                    .as_str()
                    .filter(|d| levels.iter().any(|v| v.value == *d))
                    .unwrap_or(&first.value)
                    .to_owned();
                options.push(WriterOption {
                    kind: Effort,
                    default,
                    choices: levels,
                });
            }
            let mut tiers = choices(&r["service_tiers"], "id", "name");
            if tiers.is_empty() {
                tiers = choices(&r["additional_speed_tiers"], "id", "name");
            }
            if !tiers.is_empty() {
                tiers.retain(|c| c.value != "default");
                tiers.insert(0, WriterChoice::new("default", "Standard"));
                let default = r["default_service_tier"]
                    .as_str()
                    .filter(|id| tiers.iter().any(|v| v.value == *id))
                    .unwrap_or("default");
                options.push(WriterOption {
                    kind: ServiceTier,
                    default: default.into(),
                    choices: tiers,
                });
            }
            if r["available_access_programs"]["cyber"]
                .as_array()
                .is_some_and(|programs| programs.iter().any(|id| id == "standard"))
            {
                options.push(WriterOption {
                    kind: CyberAccessProgram,
                    default: "standard".into(),
                    choices: vec![WriterChoice::new("standard", "Standard")],
                });
            }
        } else {
            if let Some(existing) = bundled.model(&id) {
                options = existing.options.clone();
            }
            if let Some(supported) = r["supportsEffort"].as_bool() {
                let prior = options
                    .iter()
                    .find(|o| o.kind == Effort)
                    .map(|o| o.default.clone());
                options.retain(|o| o.kind != Effort);
                let mut levels = choices(&r["supportedEffortLevels"], "id", "label");
                if supported && !levels.is_empty() {
                    let default = prior
                        .filter(|d| levels.iter().any(|v| v.value == *d))
                        .unwrap_or_else(|| "default".into());
                    if default == "default" && !levels.iter().any(|v| v.value == "default") {
                        levels.insert(0, WriterChoice::new("default", "Default"));
                    }
                    options.insert(
                        0,
                        WriterOption {
                            kind: Effort,
                            default,
                            choices: levels,
                        },
                    );
                }
            }
            for (key, kind, default) in [
                ("supportsFastMode", FastMode, "off"),
                ("supportsThinkingOff", Thinking, "on"),
            ] {
                if let Some(on) = r[key].as_bool() {
                    options.retain(|o| o.kind != kind);
                    if on {
                        options.push(WriterOption {
                            kind,
                            default: default.into(),
                            choices: vec![
                                WriterChoice::new("on", "On"),
                                WriterChoice::new("off", "Off"),
                            ],
                        });
                    }
                }
            }
        }
        models.push(WriterCliModel {
            id,
            label: label
                .filter(|s| !s.is_empty())
                .unwrap_or(&wire)
                .chars()
                .take(160)
                .collect(),
            provider,
            options,
        });
    }
    (!models.is_empty()).then_some(models)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::delivery::generation::GenerationOptions;
    use serde_json::json;

    #[test]
    fn codex_exposes_visible_models_and_resolves_their_reported_capabilities() {
        let list = parse(
            AgentProvider::Codex,
            &json!({"models":[
                {"slug":"ignored","visibility":"hide"},
                {"slug":"--invalid"},
                {"slug":"new-codex","display_name":"New Codex","visibility":"list",
                 "supported_reasoning_levels":[{"effort":"medium"},{"effort":"high"}],
                 "default_reasoning_level":"high","service_tiers":[
                     {"id":"ultrafast","name":"Ultra Fast","description":"Uses more quota"}],
                 "available_access_programs":{"cyber":["standard"]}},
                {"slug":"ordinary","visibility":"list"}
            ]}),
        )
        .unwrap();
        assert_eq!(list.len(), 2);
        assert!(list[1].options.is_empty());
        let m = &list[0];
        assert_eq!(m.id, "codex/new-codex");
        let options = serde_json::from_value(json!({"service_tier":"ultrafast"})).unwrap();
        let resolved = m.resolve(&options).unwrap();
        assert_eq!(resolved.configured(OptionKind::Effort), Some("high"));
        assert_eq!(
            resolved.configured(OptionKind::CyberAccessProgram),
            Some("standard")
        );
        assert_eq!(
            resolved.configured(OptionKind::ServiceTier),
            Some("ultrafast")
        );
        assert_eq!(
            m.options[1].choices[1].description.as_deref(),
            Some("Uses more quota")
        );
        assert!(m
            .resolve(&serde_json::from_value(json!({"effort":"unlisted"})).unwrap())
            .is_none());
    }

    #[test]
    fn claude_resolves_aliases_and_only_offers_reported_switches() {
        let list = parse(
            AgentProvider::Claude,
            &json!({"models":[
                {"value":"default"},
                {"value":"opus[1m]","resolvedModel":"claude-new","displayName":"New Claude",
                 "supportsEffort":true,"supportedEffortLevels":["low","max"],
                 "supportsFastMode":true,"supportsThinkingOff":false}
            ]}),
        )
        .unwrap();
        assert_eq!(list.len(), 1);
        let m = &list[0];
        assert_eq!(m.id, "claude/claude-new[1m]");
        let defaults = m.resolve(&GenerationOptions::default()).unwrap();
        assert_eq!(defaults.configured(OptionKind::Effort), Some("default"));
        assert_eq!(defaults.configured(OptionKind::FastMode), Some("off"));
        assert_eq!(defaults.configured(OptionKind::Thinking), None);
    }
}
