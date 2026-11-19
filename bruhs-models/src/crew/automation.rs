//! How automations describe the runs they start. A run gets its agent's own
//! access level, so the copy names that level, the same on both apps.

use super::{effective_level, AccessLevel};
use crate::agent::AgentProvider;
use crate::reactive::ReactiveSnapshot;

/// The level an agent's runs get: its bruh's access (read only when it has
/// none yet, such as a new agent) at the effective level for `model`.
#[must_use]
pub fn agent_run_level(
    snapshot: Option<&ReactiveSnapshot>,
    agent_id: &str,
    model: &str,
) -> AccessLevel {
    let stored = snapshot
        .and_then(|snapshot| snapshot.bruhs.iter().find(|bruh| bruh.agent_id == agent_id))
        .map(|bruh| bruh.access.level)
        .unwrap_or_default();
    effective_level(stored, AgentProvider::for_model(model))
}

/// The level a saved agent's runs get, by id.
#[must_use]
pub fn saved_agent_level(snapshot: Option<&ReactiveSnapshot>, agent_id: &str) -> AccessLevel {
    let model = snapshot
        .and_then(|snapshot| snapshot.agents.iter().find(|agent| agent.id == agent_id))
        .map_or("", |agent| agent.model.as_str());
    agent_run_level(snapshot, agent_id, model)
}

/// A level as automation copy names it.
#[must_use]
pub fn level_phrase(level: AccessLevel) -> &'static str {
    match level {
        AccessLevel::ReadOnly => "Read only",
        AccessLevel::Ask => "Ask first",
        AccessLevel::Full => "Full access",
    }
}

/// The switch that runs each event without a review.
pub const AUTO_RUN_LABEL: &str = "Run each event automatically";
/// The event policy with that switch off.
pub const REVIEW_EACH_EVENT: &str = "Review each event before running";
/// What a command source may do, which no agent level limits.
pub const COMMAND_SOURCE_ACCESS: &str = "Source command access: Runs with the local server account's permissions. The agent's access level does not restrict this command.";

/// Short event policy, for status lines and switches.
#[must_use]
pub fn event_policy(auto_run: bool, level: AccessLevel) -> String {
    if auto_run {
        format!("Automatic runs at {}", level_phrase(level))
    } else {
        REVIEW_EACH_EVENT.to_string()
    }
}

/// The event policy as a sentence, for reviews.
#[must_use]
pub fn event_policy_line(auto_run: bool, level: AccessLevel) -> String {
    if auto_run {
        format!("Runs each event automatically at {}.", level_phrase(level))
    } else {
        format!(
            "Each event waits for your review, then runs at {}.",
            level_phrase(level)
        )
    }
}

/// The note under the auto run switch.
#[must_use]
pub fn auto_run_note(level: AccessLevel) -> String {
    format!(
        "Leave it off to review each event first. Runs get this agent's access: {}.",
        level_phrase(level)
    )
}

/// What a run at `level` may do, for the Access row of a review.
#[must_use]
pub fn access_summary(level: AccessLevel) -> &'static str {
    match level {
        AccessLevel::ReadOnly => {
            "Read only. Reads its folder and returns drafts for review. No writes or payments."
        }
        AccessLevel::Ask => "Ask first. Each change waits for your approval.",
        AccessLevel::Full => {
            "Full access. Edits its folder and runs commands there without asking."
        }
    }
}

/// The button that starts a run waiting for review.
#[must_use]
pub fn run_reviewed_label(level: AccessLevel) -> String {
    format!("Run at {}", level_phrase(level))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crew::{BruhAccess, BruhLook, BruhState, BruhStatus};

    const LEVELS: [AccessLevel; 3] = [AccessLevel::ReadOnly, AccessLevel::Ask, AccessLevel::Full];

    #[test]
    fn the_copy_names_the_level_runs_actually_get() {
        for level in LEVELS {
            let phrase = level_phrase(level);
            for text in [
                event_policy(true, level),
                event_policy_line(true, level),
                event_policy_line(false, level),
                auto_run_note(level),
                access_summary(level).to_string(),
                run_reviewed_label(level),
            ] {
                assert!(text.contains(phrase), "{text}");
                assert!(
                    !text.contains(['–', '—', '•', '·']) && !text.contains(" - "),
                    "{text}"
                );
            }
        }
        assert_eq!(event_policy(false, AccessLevel::Full), REVIEW_EACH_EVENT);
        assert_eq!(
            event_policy_line(true, AccessLevel::Full),
            "Runs each event automatically at Full access."
        );
        assert!(!access_summary(AccessLevel::Full).contains("read"));
    }

    #[test]
    fn an_agent_runs_at_its_bruhs_effective_level() {
        let snapshot = ReactiveSnapshot {
            bruhs: vec![BruhState {
                agent_id: "a".into(),
                name: "A".into(),
                look: BruhLook::default(),
                access: BruhAccess {
                    level: AccessLevel::Ask,
                    ..BruhAccess::default()
                },
                effort: None,
                status: BruhStatus::Asleep,
                next_wake_ms: None,
                unread: 0,
                computer: false,
            }],
            ..ReactiveSnapshot::default()
        };
        let level = |id: &str, model: &str| agent_run_level(Some(&snapshot), id, model);
        assert_eq!(level("a", "claude-sonnet-4-6"), AccessLevel::Ask);
        assert_eq!(level("a", "gpt-5.3-codex"), AccessLevel::ReadOnly);
        assert_eq!(level("new", "claude-sonnet-4-6"), AccessLevel::ReadOnly);
        assert_eq!(
            agent_run_level(None, "a", "claude-sonnet-4-6"),
            AccessLevel::ReadOnly
        );
        let mut saved = snapshot.clone();
        saved.agents = vec![crate::reactive::ReactiveAgent {
            id: "a".into(),
            name: "A".into(),
            instructions: String::new(),
            memory: String::new(),
            model: "claude-sonnet-4-6".into(),
            project_path: "/w".into(),
            enabled: true,
        }];
        assert_eq!(saved_agent_level(Some(&saved), "a"), AccessLevel::Ask);
        assert_eq!(saved_agent_level(Some(&saved), "b"), AccessLevel::ReadOnly);
    }
}
