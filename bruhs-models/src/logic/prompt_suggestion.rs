//! A next-message prediction is never draft text until the user accepts it.

use serde::{Deserialize, Serialize};

use crate::agent::{AgentProvider, Role};

// Product budgets: one short sentence, six recent conversational messages,
// at most 12k characters of context. Tool output and thinking never enter it.
pub const MAX_SUGGESTION_CHARS: usize = 240;
pub const MAX_CONTEXT_MESSAGES: usize = 6;
pub const MAX_MESSAGE_CHARS: usize = 2_000;
pub const SUGGESTION_HELP: &str = "Predict a next message after a reply finishes. Tab accepts, Escape dismisses. On by default. Uses one extra model request per reply and pauses when usage limits are reported.";

/// Stay on the thread's provider without inheriting its expensive chat model.
/// An empty id disables suggestions for runtimes without an isolated no-tools request.
pub fn default_suggestion_model(provider: AgentProvider) -> &'static str {
    match provider {
        AgentProvider::Claude => "claude-haiku-4-5",
        AgentProvider::Codex => "gpt-6-luna",
        AgentProvider::Cursor
        | AgentProvider::Antigravity
        | AgentProvider::Grok
        | AgentProvider::OpenCode
        | AgentProvider::Acp => "",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptLine {
    pub role: Role,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestionRequest {
    pub model: String,
    pub messages: Vec<PromptLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuggestionReply {
    pub suggestion: String,
}

/// Keep the end of recent messages: the answer's conclusion often names the
/// next step. Callers pass only user text and assistant prose, in order.
pub fn recent_context(lines: impl DoubleEndedIterator<Item = PromptLine>) -> Vec<PromptLine> {
    let mut lines: Vec<_> = lines
        .rev()
        .filter(|line| !line.text.trim().is_empty())
        .take(MAX_CONTEXT_MESSAGES)
        .map(|mut line| {
            let start = line
                .text
                .char_indices()
                .rev()
                .nth(MAX_MESSAGE_CHARS - 1)
                .map_or(0, |(offset, _)| offset);
            line.text = line.text[start..].to_string();
            line
        })
        .collect();
    lines.reverse();
    lines
}

impl SuggestionRequest {
    pub fn is_valid(&self) -> bool {
        !self.model.is_empty()
            && !self.messages.is_empty()
            && self.messages.len() <= MAX_CONTEXT_MESSAGES
            && self
                .messages
                .last()
                .is_some_and(|line| line.role == Role::Assistant && !line.text.trim().is_empty())
            && self.messages.iter().any(|line| line.role == Role::User)
            && self.messages.iter().all(|line| {
                line.role != Role::Tool && line.text.chars().count() <= MAX_MESSAGE_CHARS
            })
    }
}

impl SuggestionReply {
    pub fn is_valid(&self) -> bool {
        self.suggestion.chars().count() <= MAX_SUGGESTION_CHARS
            && !self.suggestion.chars().any(char::is_control)
            && self.suggestion == self.suggestion.trim()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestionKey {
    pub thread: String,
    pub turn: String,
    pub model: String,
}

#[derive(Debug, Clone, Default)]
pub struct PromptSuggestion {
    key: Option<SuggestionKey>,
    state: Prediction,
}

#[derive(Debug, Clone, Default)]
enum Prediction {
    #[default]
    Dismissed,
    Loading,
    Ready(String),
}

impl PromptSuggestion {
    /// At most one attempt for this completed reply, including failures and
    /// dismissals. A late result may only finish its own pending request.
    pub fn begin(&mut self, key: SuggestionKey) -> bool {
        if self.key.as_ref() == Some(&key) {
            return false;
        }
        self.key = Some(key);
        self.state = Prediction::Loading;
        true
    }

    pub fn finish(&mut self, key: &SuggestionKey, reply: SuggestionReply) {
        if self.key.as_ref() == Some(key) && matches!(self.state, Prediction::Loading) {
            self.state = if reply.is_valid() && !reply.suggestion.is_empty() {
                Prediction::Ready(reply.suggestion)
            } else {
                Prediction::Dismissed
            };
        }
    }

    pub fn dismiss(&mut self) {
        self.state = Prediction::Dismissed;
    }

    /// The real draft must be an exact prefix and the selection collapsed at
    /// its end. No replacement, cursor movement, or partial Unicode slicing.
    pub fn suffix(&self, draft: &str, selection: std::ops::Range<usize>) -> Option<&str> {
        if selection != (draft.len()..draft.len()) {
            return None;
        }
        let Prediction::Ready(text) = &self.state else {
            return None;
        };
        text.strip_prefix(draft).filter(|suffix| !suffix.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(turn: &str) -> SuggestionKey {
        SuggestionKey {
            thread: "thread".into(),
            turn: turn.into(),
            model: "model".into(),
        }
    }

    #[test]
    fn stale_and_dismissed_results_cannot_return() {
        let mut state = PromptSuggestion::default();
        assert!(state.begin(key("old")));
        assert!(state.begin(key("new")));
        state.finish(
            &key("old"),
            SuggestionReply {
                suggestion: "stale".into(),
            },
        );
        assert_eq!(state.suffix("", 0..0), None);
        state.dismiss();
        state.finish(
            &key("new"),
            SuggestionReply {
                suggestion: "dismissed".into(),
            },
        );
        assert_eq!(state.suffix("", 0..0), None);
        assert!(
            !state.begin(key("new")),
            "no repeated spend after dismissing"
        );
    }

    #[test]
    fn unicode_prefixes_selections_and_divergent_drafts() {
        let mut state = PromptSuggestion::default();
        state.begin(key("turn"));
        state.finish(
            &key("turn"),
            SuggestionReply {
                suggestion: "説明して 🦀".into(),
            },
        );
        assert_eq!(state.suffix("説明", 6..6), Some("して 🦀"));
        assert_eq!(state.suffix("説明", 0..6), None);
        assert_eq!(state.suffix("説明", 0..0), None);
        assert_eq!(state.suffix("other", 5..5), None);
        assert_eq!(state.suffix("説明して 🦀", 17..17), None);
    }

    #[test]
    fn context_is_bounded_without_splitting_unicode() {
        let context = recent_context((0..20).map(|_| PromptLine {
            role: Role::User,
            text: "🦀".repeat(3000),
        }));
        assert_eq!(context.len(), MAX_CONTEXT_MESSAGES);
        assert!(context
            .iter()
            .all(|line| line.text.chars().count() == MAX_MESSAGE_CHARS));
    }

    #[test]
    fn invalid_predictions_stay_out_of_the_composer() {
        for text in [
            "line\nbreak".into(),
            " padded ".into(),
            "x".repeat(MAX_SUGGESTION_CHARS + 1),
        ] {
            assert!(!SuggestionReply { suggestion: text }.is_valid());
        }
        assert!(SuggestionReply {
            suggestion: String::new()
        }
        .is_valid());
    }

    #[test]
    fn context_requires_a_user_and_a_completed_assistant_answer() {
        let mut request = SuggestionRequest {
            model: "model".into(),
            messages: vec![
                PromptLine {
                    role: Role::User,
                    text: "Fix the bug".into(),
                },
                PromptLine {
                    role: Role::Assistant,
                    text: "Fixed the bug.".into(),
                },
            ],
        };
        assert!(request.is_valid());
        request.messages[0].role = Role::Tool;
        assert!(!request.is_valid(), "tool content never enters predictions");
        request.messages[0].role = Role::User;
        request.messages[1].role = Role::User;
        assert!(!request.is_valid(), "a user draft is not a completed reply");
        request.messages[1].role = Role::Assistant;
        request.messages[1].text = "x".repeat(MAX_MESSAGE_CHARS + 1);
        assert!(
            !request.is_valid(),
            "the API enforces the client context budget"
        );
    }
}
