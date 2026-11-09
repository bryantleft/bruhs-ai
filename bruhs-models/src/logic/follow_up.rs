//! What the composer does with the next message while a turn is running.
//!
//! Steer writes into the live Claude turn. Codex cannot take a mid-turn
//! message, so Steer stays unavailable there and Queue is the follow-up.

use crate::agent::AgentProvider;

/// The action the composer button will take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComposerAction {
    /// The thread is idle and the draft will start a turn.
    Send,
    /// The draft waits until the running turn ends.
    Queue,
    /// The draft is empty and the running turn can be stopped.
    Interrupt,
    /// A stopped turn left queued messages, and the draft is empty.
    Resume,
    /// The draft goes into the running Claude turn.
    Steer,
    /// Cmd+Enter asked to steer. This provider cannot take it.
    SteerUnavailable,
}

/// Whether the stored follow-up is Steer, and whether this provider can do it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FollowUp {
    pub wants_steer: bool,
    pub can_steer: bool,
}

/// Claude's streaming input can take another user message. Codex cannot.
pub fn provider_can_steer(provider: AgentProvider) -> bool {
    matches!(provider, AgentProvider::Claude)
}

/// The follow-up this client will use. A stored `steer` is Queue when the
/// provider cannot steer.
pub fn effective_follow_up(stored: &str, can_steer: bool) -> &'static str {
    if can_steer && stored == "steer" {
        "steer"
    } else {
        "queue"
    }
}

/// `opposite` is Cmd+Enter or a click while Cmd is held. It flips Steer and Queue.
pub fn composer_action(
    streaming: bool,
    draft_empty: bool,
    opposite: bool,
    queue_held: bool,
    has_queued: bool,
    follow_up: FollowUp,
) -> ComposerAction {
    if streaming {
        if draft_empty {
            return ComposerAction::Interrupt;
        }
        let steer = if opposite {
            !follow_up.wants_steer
        } else {
            follow_up.wants_steer
        };
        if steer {
            return if follow_up.can_steer {
                ComposerAction::Steer
            } else {
                ComposerAction::SteerUnavailable
            };
        }
        return ComposerAction::Queue;
    }
    if queue_held && draft_empty && has_queued {
        return ComposerAction::Resume;
    }
    ComposerAction::Send
}

/// Send, Queue, and Steer need text. Interrupt and Resume work on an empty
/// composer. Steer stays disabled when this provider cannot take it.
pub fn action_enabled_for(action: ComposerAction, draft_empty: bool) -> bool {
    match action {
        ComposerAction::Send | ComposerAction::Queue | ComposerAction::Steer => !draft_empty,
        ComposerAction::Interrupt | ComposerAction::Resume => true,
        ComposerAction::SteerUnavailable => false,
    }
}

pub fn action_label(action: ComposerAction) -> &'static str {
    match action {
        ComposerAction::Send => "Send",
        ComposerAction::Queue => "Queue",
        ComposerAction::Interrupt => "Interrupt",
        ComposerAction::Resume => "Resume",
        ComposerAction::Steer | ComposerAction::SteerUnavailable => "Steer",
    }
}

pub fn action_title(action: ComposerAction) -> &'static str {
    match action {
        ComposerAction::Send => "Send message (Enter)",
        ComposerAction::Queue => "Queue message (Enter). Sends when this turn ends",
        ComposerAction::Interrupt => "Interrupt",
        ComposerAction::Resume => "Resume queued messages",
        ComposerAction::Steer => "Steer the running turn (Enter)",
        ComposerAction::SteerUnavailable => STEER_UNAVAILABLE,
    }
}

pub const STEER_UNAVAILABLE: &str = "This provider queues the next message until the turn ends";

/// The queued message disappeared while it was open in the composer.
/// Keep the in-progress text when the draft it replaced was empty.
pub fn composer_after_lost_edit(prior_draft: &str, in_progress: &str) -> String {
    if prior_draft.is_empty() {
        in_progress.to_string()
    } else {
        prior_draft.to_string()
    }
}

/// A queued message opened in the composer, plus the draft it replaced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueEdit {
    pub id: String,
    pub prior_draft: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_send_and_running_queue() {
        assert_eq!(
            composer_action(false, false, false, false, false, FollowUp::default()),
            ComposerAction::Send
        );
        assert_eq!(
            composer_action(true, false, false, false, true, FollowUp::default()),
            ComposerAction::Queue
        );
        assert_eq!(
            composer_action(true, true, false, false, true, FollowUp::default()),
            ComposerAction::Interrupt
        );
        assert_eq!(
            composer_action(true, false, true, false, true, FollowUp::default()),
            ComposerAction::SteerUnavailable
        );
        assert_eq!(
            composer_action(true, true, true, false, true, FollowUp::default()),
            ComposerAction::Interrupt
        );
    }

    #[test]
    fn held_queue_resumes_from_an_empty_composer() {
        assert_eq!(
            composer_action(false, true, false, true, true, FollowUp::default()),
            ComposerAction::Resume
        );
        assert_eq!(
            composer_action(false, false, false, true, true, FollowUp::default()),
            ComposerAction::Send
        );
        assert_eq!(
            composer_action(false, true, false, true, false, FollowUp::default()),
            ComposerAction::Send
        );
    }

    #[test]
    fn stored_steer_is_queue_unless_the_provider_can_steer() {
        assert_eq!(effective_follow_up("steer", false), "queue");
        assert_eq!(effective_follow_up("steer", true), "steer");
        assert_eq!(effective_follow_up("queue", true), "queue");
        assert_eq!(effective_follow_up("", true), "queue");
        assert!(provider_can_steer(AgentProvider::Claude));
        assert!(!provider_can_steer(AgentProvider::Codex));
    }

    #[test]
    fn claude_steer_flips_with_the_opposite_key() {
        let steer = FollowUp {
            wants_steer: true,
            can_steer: true,
        };
        let queue = FollowUp {
            wants_steer: false,
            can_steer: true,
        };
        assert_eq!(
            composer_action(true, false, false, false, false, steer),
            ComposerAction::Steer
        );
        assert_eq!(
            composer_action(true, false, true, false, false, steer),
            ComposerAction::Queue
        );
        assert_eq!(
            composer_action(true, false, true, false, false, queue),
            ComposerAction::Steer
        );
        assert!(action_enabled_for(ComposerAction::Steer, false));
        assert!(!action_enabled_for(ComposerAction::Steer, true));
    }

    #[test]
    fn lost_edit_keeps_text_only_when_the_prior_draft_was_empty() {
        assert_eq!(composer_after_lost_edit("", "edited"), "edited");
        assert_eq!(composer_after_lost_edit("old draft", "edited"), "old draft");
    }

    #[test]
    fn steer_and_empty_send_are_the_disabled_actions() {
        assert!(!action_enabled_for(ComposerAction::SteerUnavailable, false));
        assert!(!action_enabled_for(ComposerAction::Send, true));
        assert!(action_enabled_for(ComposerAction::Queue, false));
        assert!(action_enabled_for(ComposerAction::Interrupt, true));
        assert!(action_enabled_for(ComposerAction::Resume, true));
    }
}
