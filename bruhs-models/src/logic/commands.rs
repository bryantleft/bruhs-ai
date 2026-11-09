//! Slash commands, thread goals, and `$` skill mentions.
//!
//! `/goal` and `/compact` are Claude Code and Codex TUI commands. The goal
//! stored here is local: the row shows it, and the model prompt gains one
//! instruction. `/compact` on Claude starts a turn with that text. Codex
//! still has no compact switch.

use serde::{Deserialize, Serialize};

use crate::agent::AgentProvider;

/// Shown when Compact is chosen for Codex. Claude sends `/compact`.
pub const COMPACT_UNAVAILABLE: &str = "Compact waits until Codex exposes it on this machine.";

/// Claude has no pause or resume for a bruhs goal.
pub const GOAL_CODEX_ONLY: &str = "Pause and resume are Codex goals.";

pub const GOAL_NONE: &str = "No goal is set.";
pub const GOAL_CLEARED: &str = "Goal cleared.";
pub const GOAL_PAUSED: &str = "Goal paused.";

/// Composer placeholder. The empty-thread footer under it stays as it is.
pub const COMPOSER_PLACEHOLDER: &str =
    "Ask anything, @ threads or files, # pull requests, $ skills, or / for commands";

pub const COMMAND_HELP: &str = "\
Available commands: /clear, /help, /model, /settings, /goal, /compact, /handoff, /second-opinion.\n\
Keyboard: ⌘K command bar, ⌘B sidebar, ⌘⇧T tasks panel, / focus chat, c new task (in tasks view).\n\
/goal sets a goal for this thread. /goal clear removes it. Pause and resume are Codex goals. \
/compact summarizes a Claude thread. Codex still waits. \
/handoff <goal> writes a brief and opens a fresh thread with it. \
/second-opinion [focus] asks the other provider to review the last answer.";

/// `/` menu. Skills stay on `$`.
pub const SLASH_COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        name: "/clear",
        description: "Clear conversation",
    },
    SlashCommand {
        name: "/help",
        description: "Show available commands",
    },
    SlashCommand {
        name: "/model",
        description: "Change model",
    },
    SlashCommand {
        name: "/settings",
        description: "Open settings",
    },
    SlashCommand {
        name: "/goal",
        description: "Set a goal for this thread",
    },
    SlashCommand {
        name: "/compact",
        description: "Compact the conversation",
    },
    SlashCommand {
        name: "/handoff",
        description: "Continue in a fresh thread with a brief",
    },
    SlashCommand {
        name: "/second-opinion",
        description: "Ask the other provider to review the last answer",
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlashCommand {
    pub name: &'static str,
    pub description: &'static str,
}

/// A goal for this thread. It stays until `/goal clear`. bruhs does not
/// decide that the goal has been met.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadGoal {
    pub text: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub paused: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// What to do with the stored goal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalWrite {
    Keep,
    Set(ThreadGoal),
    Remove,
}

/// How the composer should treat this message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedCommand {
    pub write: GoalWrite,
    pub notice: Option<String>,
    /// Start a turn with the typed text.
    pub send: bool,
    /// Wait for the running turn, then send.
    pub queue: bool,
    /// Stop a turn that is working on an active goal.
    pub stop: bool,
}

/// `/name` at the start of `text`, after trimming. The rest is the argument
/// with its surrounding whitespace removed. `/goalpost` is not `/goal`.
fn command_argument<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let text = text.trim();
    let rest = text.strip_prefix(name)?;
    if rest.is_empty() {
        return Some("");
    }
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some(rest.trim())
}

/// Classify `text` for `provider`. `/goal pause` on Claude does not store a
/// goal named "pause".
pub fn classify_command(text: &str, provider: AgentProvider) -> CommandKind {
    if command_argument(text, "/compact") == Some("") {
        return CommandKind::Compact;
    }
    if let Some(goal) = command_argument(text, "/handoff") {
        return CommandKind::Handoff(goal.to_string());
    }
    if let Some(focus) = command_argument(text, "/second-opinion") {
        return CommandKind::SecondOpinion(focus.to_string());
    }
    let Some(argument) = command_argument(text, "/goal") else {
        return CommandKind::Send;
    };
    if argument.is_empty() {
        return CommandKind::Show;
    }
    match argument {
        "clear" => CommandKind::Clear,
        "pause" if provider == AgentProvider::Codex => CommandKind::Pause,
        "resume" if provider == AgentProvider::Codex => CommandKind::Resume,
        "pause" | "resume" => CommandKind::CodexOnly,
        _ => CommandKind::SetGoal(argument.to_string()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandKind {
    Send,
    Show,
    Clear,
    Pause,
    Resume,
    CodexOnly,
    Compact,
    SetGoal(String),
    /// `/handoff <goal>`: a turn that writes a brief, then a fresh thread.
    Handoff(String),
    /// `/second-opinion [focus]`: a read-only review by the other provider.
    /// Never a turn of this thread; the surface starts the review.
    SecondOpinion(String),
}

/// Decide the goal write, the notice, and whether this message sends.
/// A new goal or a resume while a turn is running is queued. Clear and pause
/// stop the turn only when it is working on an active goal.
pub fn prepare_command(
    text: &str,
    provider: AgentProvider,
    goal: Option<&ThreadGoal>,
    streaming: bool,
) -> PreparedCommand {
    let working = streaming && goal.is_some_and(|goal| !goal.paused);
    match classify_command(text, provider) {
        CommandKind::Send => PreparedCommand {
            write: GoalWrite::Keep,
            notice: None,
            send: !streaming,
            queue: streaming,
            stop: false,
        },
        CommandKind::SetGoal(text) => PreparedCommand {
            write: if streaming {
                GoalWrite::Keep
            } else {
                GoalWrite::Set(ThreadGoal {
                    text,
                    paused: false,
                })
            },
            notice: None,
            send: !streaming,
            queue: streaming,
            stop: false,
        },
        CommandKind::Resume => match goal {
            None => local(GOAL_NONE),
            Some(_) if streaming => PreparedCommand {
                write: GoalWrite::Keep,
                notice: None,
                send: false,
                queue: true,
                stop: false,
            },
            Some(goal) => PreparedCommand {
                write: GoalWrite::Set(ThreadGoal {
                    text: goal.text.clone(),
                    paused: false,
                }),
                notice: None,
                send: true,
                queue: false,
                stop: false,
            },
        },
        CommandKind::Show => PreparedCommand {
            write: GoalWrite::Keep,
            notice: Some(goal_notice(goal)),
            send: false,
            queue: false,
            stop: false,
        },
        CommandKind::Clear => match goal {
            None => local(GOAL_NONE),
            Some(_) => PreparedCommand {
                write: GoalWrite::Remove,
                notice: Some(GOAL_CLEARED.to_string()),
                send: false,
                queue: false,
                stop: working,
            },
        },
        CommandKind::Pause => match goal {
            None => local(GOAL_NONE),
            Some(goal) if goal.paused => local(GOAL_PAUSED),
            Some(goal) => PreparedCommand {
                write: GoalWrite::Set(ThreadGoal {
                    text: goal.text.clone(),
                    paused: true,
                }),
                notice: Some(GOAL_PAUSED.to_string()),
                send: false,
                queue: false,
                stop: working,
            },
        },
        CommandKind::CodexOnly => local(GOAL_CODEX_ONLY),
        CommandKind::Handoff(_) => PreparedCommand {
            write: GoalWrite::Keep,
            notice: None,
            send: !streaming,
            queue: streaming,
            stop: false,
        },
        CommandKind::SecondOpinion(_) => PreparedCommand {
            write: GoalWrite::Keep,
            notice: None,
            send: false,
            queue: false,
            stop: false,
        },
        CommandKind::Compact => {
            if provider == AgentProvider::Claude {
                PreparedCommand {
                    write: GoalWrite::Keep,
                    notice: None,
                    send: !streaming,
                    queue: streaming,
                    stop: false,
                }
            } else {
                local(COMPACT_UNAVAILABLE)
            }
        }
    }
}

fn local(notice: &str) -> PreparedCommand {
    PreparedCommand {
        write: GoalWrite::Keep,
        notice: Some(notice.to_string()),
        send: false,
        queue: false,
        stop: false,
    }
}

pub fn goal_notice(goal: Option<&ThreadGoal>) -> String {
    match goal {
        None => GOAL_NONE.to_string(),
        Some(goal) if goal.paused => format!("Goal paused: {}", goal.text),
        Some(goal) => format!("Goal: {}", goal.text),
    }
}

/// One instruction appended to the model prompt. A paused goal adds nothing.
/// The user bubble keeps the typed text.
pub fn append_goal(prompt: &str, goal: Option<&ThreadGoal>) -> String {
    let Some(goal) = goal else {
        return prompt.to_string();
    };
    if goal.paused || goal.text.trim().is_empty() {
        return prompt.to_string();
    }
    let line = format!(
        "The goal for this thread is: {}\nWork until that is true.",
        goal.text.trim()
    );
    if prompt.is_empty() {
        line
    } else {
        format!("{prompt}\n\n{line}")
    }
}

/// Replace the `$` token that ends at `caret` with `$name`, and leave the
/// caret after the space that closes the token.
pub fn insert_skill_mention(text: &str, caret: usize, name: &str) -> (String, usize) {
    let caret = caret.min(text.len());
    let before = &text[..caret];
    let after = &text[caret..];
    let mut trigger_pos = before.len();
    for (index, ch) in before.char_indices().rev() {
        if ch.is_whitespace() {
            break;
        }
        trigger_pos = index;
        if ch == '$' {
            break;
        }
    }
    let replacement = format!("${name}");
    let mut output = String::with_capacity(text.len() + replacement.len() + 1);
    output.push_str(&before[..trigger_pos]);
    output.push_str(&replacement);
    let mut new_caret = output.len();
    if after.starts_with(char::is_whitespace) {
        if let Some(ch) = after.chars().next() {
            new_caret += ch.len_utf8();
        }
    } else {
        output.push(' ');
        new_caret = output.len();
    }
    output.push_str(after);
    (output, new_caret)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn goal(text: &str, paused: bool) -> ThreadGoal {
        ThreadGoal {
            text: text.to_string(),
            paused,
        }
    }

    #[test]
    fn goal_requires_a_word_boundary() {
        assert_eq!(
            classify_command("/goalpost", AgentProvider::Claude),
            CommandKind::Send
        );
        assert_eq!(
            classify_command("  /goal  ", AgentProvider::Claude),
            CommandKind::Show
        );
        assert_eq!(
            classify_command("/goal clear", AgentProvider::Codex),
            CommandKind::Clear
        );
        assert_eq!(
            classify_command("/goal ship the tests", AgentProvider::Claude),
            CommandKind::SetGoal("ship the tests".into())
        );
        assert_eq!(
            classify_command("hello\n/goal hi", AgentProvider::Claude),
            CommandKind::Send
        );
    }

    #[test]
    fn pause_and_resume_are_codex_goals() {
        assert_eq!(
            classify_command("/goal pause", AgentProvider::Claude),
            CommandKind::CodexOnly
        );
        assert_eq!(
            classify_command("/goal resume", AgentProvider::Claude),
            CommandKind::CodexOnly
        );
        assert_eq!(
            classify_command("/goal pause", AgentProvider::Codex),
            CommandKind::Pause
        );
        assert_eq!(
            classify_command("/goal resume", AgentProvider::Codex),
            CommandKind::Resume
        );
    }

    #[test]
    fn handoff_and_second_opinion_take_an_argument() {
        assert_eq!(
            classify_command("/handoff ship it", AgentProvider::Claude),
            CommandKind::Handoff("ship it".into())
        );
        assert_eq!(
            classify_command("/handoff", AgentProvider::Codex),
            CommandKind::Handoff(String::new())
        );
        assert_eq!(
            classify_command("/second-opinion races", AgentProvider::Claude),
            CommandKind::SecondOpinion("races".into())
        );
        assert_eq!(
            classify_command("/handoffs", AgentProvider::Claude),
            CommandKind::Send
        );
        let review = prepare_command("/second-opinion", AgentProvider::Claude, None, false);
        assert!(!review.send && !review.queue, "a review is never a turn");
        let handoff = prepare_command("/handoff x", AgentProvider::Claude, None, true);
        assert!(handoff.queue && !handoff.send, "waits for the running turn");
    }

    #[test]
    fn compact_is_only_the_bare_command() {
        assert_eq!(
            classify_command("/compact", AgentProvider::Claude),
            CommandKind::Compact
        );
        assert_eq!(
            classify_command("/compact now", AgentProvider::Codex),
            CommandKind::Send
        );
    }

    #[test]
    fn claude_compact_sends_and_codex_stays_local() {
        let claude = prepare_command("/compact", AgentProvider::Claude, None, false);
        assert!(claude.send);
        assert!(claude.notice.is_none());
        let queued = prepare_command("/compact", AgentProvider::Claude, None, true);
        assert!(queued.queue);
        assert!(!queued.send);
        let codex = prepare_command("/compact", AgentProvider::Codex, None, false);
        assert_eq!(codex.notice.as_deref(), Some(COMPACT_UNAVAILABLE));
        assert!(!codex.send);
    }

    #[test]
    fn clear_and_pause_stop_only_while_the_goal_is_active() {
        let active = goal("ship it", false);
        let paused = goal("ship it", true);
        let clearing = prepare_command("/goal clear", AgentProvider::Claude, Some(&active), true);
        assert!(clearing.stop);
        assert!(matches!(clearing.write, GoalWrite::Remove));
        assert!(!prepare_command("/goal clear", AgentProvider::Claude, Some(&paused), true).stop);
        let pausing = prepare_command("/goal pause", AgentProvider::Codex, Some(&active), true);
        assert!(pausing.stop);
        assert!(!prepare_command("/goal pause", AgentProvider::Codex, Some(&paused), true).stop);
    }

    #[test]
    fn a_new_goal_sends_when_idle_and_queues_while_streaming() {
        let idle = prepare_command("/goal ship it", AgentProvider::Claude, None, false);
        assert!(idle.send);
        assert!(matches!(idle.write, GoalWrite::Set(_)));
        let busy = prepare_command("/goal ship it", AgentProvider::Claude, None, true);
        assert!(busy.queue);
        assert!(matches!(busy.write, GoalWrite::Keep));
    }

    #[test]
    fn resume_sends_when_a_goal_exists() {
        let missing = prepare_command("/goal resume", AgentProvider::Codex, None, false);
        assert!(!missing.send);
        assert_eq!(missing.notice.as_deref(), Some(GOAL_NONE));
        let active = goal("ship it", true);
        let resume = prepare_command("/goal resume", AgentProvider::Codex, Some(&active), false);
        assert!(resume.send);
        assert_eq!(
            resume.write,
            GoalWrite::Set(ThreadGoal {
                text: "ship it".into(),
                paused: false,
            })
        );
        let queued = prepare_command("/goal resume", AgentProvider::Codex, Some(&active), true);
        assert!(queued.queue);
        assert!(matches!(queued.write, GoalWrite::Keep));
    }

    #[test]
    fn append_goal_skips_a_paused_goal_and_keeps_the_typed_prompt() {
        assert_eq!(append_goal("hello", None), "hello");
        assert_eq!(append_goal("hello", Some(&goal("ship it", true))), "hello");
        assert_eq!(
            append_goal("hello", Some(&goal(" ship it ", false))),
            "hello\n\nThe goal for this thread is: ship it\nWork until that is true."
        );
    }

    #[test]
    fn skill_insertion_closes_the_token() {
        assert_eq!(
            insert_skill_mention("use $re please", 7, "review"),
            ("use $review please".into(), 12)
        );
        assert_eq!(
            insert_skill_mention("$re", 3, "review"),
            ("$review ".into(), 8)
        );
    }
}
