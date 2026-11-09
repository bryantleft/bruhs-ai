//! Provider presentation helpers shared by the composer and the model dialog.
//! Which provider serves a model is `bruhs_models::agent::provider_for_model_id`,
//! the routing the server uses too.

use bruhs_models::agent::{provider_for_model_id, AgentProvider};

/// Small monochrome provider mark: 12px, forced white, dimmed so it reads as
/// chrome next to `text-rambutan-400` labels.
pub const PROVIDER_LOGO_CLASS: &str =
    "size-3 shrink-0 object-contain brightness-0 invert opacity-60";

pub fn provider_logo_src_for_model(model_id: &str) -> &'static str {
    provider_logo_src(provider_for_model_id(model_id))
}

/// Display name for a provider.
pub fn provider_label(provider: AgentProvider) -> &'static str {
    match provider {
        AgentProvider::Claude => "Anthropic",
        AgentProvider::Codex => "OpenAI Codex",
    }
}

/// Monochrome mark for a provider.
pub fn provider_logo_src(provider: AgentProvider) -> &'static str {
    match provider {
        AgentProvider::Claude => "/logos/claude.svg",
        AgentProvider::Codex => "/logos/openai.svg",
    }
}

pub fn provider_agent_label_for_model(model_id: &str) -> &'static str {
    match provider_for_model_id(model_id) {
        AgentProvider::Claude => "claude agent",
        AgentProvider::Codex => "codex agent",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_logo_class_prevents_oversized_icons() {
        for token in ["size-3", "shrink-0", "object-contain", "opacity-60"] {
            assert!(
                PROVIDER_LOGO_CLASS.contains(token),
                "missing provider logo class token {token}"
            );
        }
    }

    #[test]
    fn providers_map_to_labels_and_logos() {
        assert_eq!(provider_label(AgentProvider::Claude), "Anthropic");
        assert_eq!(provider_label(AgentProvider::Codex), "OpenAI Codex");
        assert_eq!(
            provider_logo_src(AgentProvider::Claude),
            "/logos/claude.svg"
        );
        assert_eq!(provider_logo_src(AgentProvider::Codex), "/logos/openai.svg");
    }

    #[test]
    fn provider_label_follows_model() {
        assert_eq!(
            provider_logo_src_for_model("claude-sonnet-4-6"),
            "/logos/claude.svg"
        );
        assert_eq!(provider_logo_src_for_model("gpt-5.5"), "/logos/openai.svg");
        assert_eq!(
            provider_agent_label_for_model("claude-sonnet-4-6"),
            "claude agent"
        );
        assert_eq!(provider_agent_label_for_model("gpt-5.5"), "codex agent");
        // The shared routing: bare Claude aliases are Claude's.
        assert_eq!(provider_agent_label_for_model("opus"), "claude agent");
    }
}
