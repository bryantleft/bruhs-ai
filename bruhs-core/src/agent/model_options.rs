//! Apply explicit model choices to an existing provider request.
use super::ProviderQueryOptions;
use bruhs_models::delivery::generation::{GenerationOptions, OptionKind};
use serde_json::Value;

pub(super) fn apply(options: &mut ProviderQueryOptions, selected: &GenerationOptions) {
    if selected.is_empty() {
        return;
    }
    match options {
        ProviderQueryOptions::Claude(options) => apply_claude(options, selected),
        ProviderQueryOptions::Codex(options) => apply_codex(options, selected),
        ProviderQueryOptions::External(_) => {}
    }
}

pub(crate) fn apply_claude(
    options: &mut claude_sdk_rs::QueryOptions,
    selected: &GenerationOptions,
) {
    // Lean turns already carry settings such as permission rules and hooks.
    let mut flags = serde_json::Map::new();
    for (kind, key) in [
        (OptionKind::Thinking, "alwaysThinkingEnabled"),
        (OptionKind::FastMode, "fastMode"),
        (OptionKind::Ultracode, "ultracode"),
    ] {
        if let Some(value @ ("on" | "off")) = selected.configured(kind) {
            flags.insert(key.into(), (value == "on").into());
        }
    }
    if !flags.is_empty() {
        let existing = options
            .settings
            .as_deref()
            .map(serde_json::from_str::<serde_json::Map<String, Value>>)
            .transpose();
        if let Ok(existing) = existing {
            let mut merged = existing.unwrap_or_default();
            merged.extend(flags);
            options.settings = Some(Value::Object(merged).to_string());
        }
    }
    if let Some(value) = selected
        .configured(OptionKind::Effort)
        .filter(|s| *s != "default")
    {
        options.effort = Some(value.to_owned());
    }
    if let Some(model) = options.model.as_mut() {
        match selected.configured(OptionKind::ContextWindow) {
            Some("1m") if !model.ends_with("[1m]") => model.push_str("[1m]"),
            Some("200k") if model.ends_with("[1m]") => model.truncate(model.len() - 4),
            _ => {}
        }
    }
}

pub(crate) fn apply_codex(options: &mut codex_sdk_rs::QueryOptions, selected: &GenerationOptions) {
    for (kind, key) in [
        (OptionKind::Effort, "model_reasoning_effort"),
        (OptionKind::ServiceTier, "service_tier"),
    ] {
        if let Some(value) = selected.configured(kind) {
            if value == "default" && (kind == OptionKind::Effort || options.ignore_user_config) {
                continue;
            }
            options.config.retain(|o| o.key() != key);
            options.config.push(
                codex_sdk_rs::ConfigOverride::new(key, Value::from(value).to_string())
                    .expect("fixed configuration key"),
            );
        }
    }
    if selected.configured(OptionKind::CyberAccessProgram) == Some("standard") {
        options.standard_cyber_access = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn claude_model_options_merge_into_lean_settings_and_set_the_context_window() {
        let mut request = claude_sdk_rs::QueryOptions::new("draft");
        request.model = Some("claude-sonnet-4-6".into());
        request.settings = Some(r#"{"permissions":{"deny":["existing"]},"hooks":{}}"#.into());
        let values = serde_json::from_value(
            serde_json::json!({"effort":"high","context_window":"1m","thinking":"off"}),
        )
        .unwrap();
        apply_claude(&mut request, &values);
        apply_claude(&mut request, &values);
        assert_eq!(request.model.as_deref(), Some("claude-sonnet-4-6[1m]"));
        assert_eq!(request.effort.as_deref(), Some("high"));
        assert_eq!(
            serde_json::from_str::<Value>(request.settings.as_deref().unwrap()).unwrap(),
            serde_json::json!({"permissions":{"deny":["existing"]},"hooks":{},"alwaysThinkingEnabled":false})
        );
    }
}
