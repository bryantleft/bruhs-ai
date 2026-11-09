//! `/handoff` and `/second-opinion`: two ways to buy intelligence per token.
//!
//! Handoff (after Amp's): instead of compacting a long thread into a summary
//! of a summary, the current agent writes a brief for a fresh thread, so the
//! next step starts with a small, focused context. The brief turn runs on the
//! thread's own session, so its long prefix is a cache read.
//!
//! Second opinion: the other local provider (Codex for a Claude thread and
//! the reverse) reviews the last answer read-only, and its review becomes the
//! next message to the main agent. bruhs runs both CLIs locally, so this
//! needs no extra service.

use crate::agent::AgentProvider;

/// What the next thread should do when `/handoff` has no goal.
pub const HANDOFF_DEFAULT_GOAL: &str = "continue the current work";

/// Shown when `/second-opinion` is sent while a turn is still running: a
/// review of a half-written answer is not worth its tokens, and the
/// command is not a turn, so it is not queued either.
pub const SECOND_OPINION_WAIT: &str =
    "Second opinion waits for the answer: send it once the turn finishes.";

/// Shown when `/second-opinion` has no answer to review.
pub const SECOND_OPINION_NOTHING: &str = "Nothing to review yet: send a message first.";

/// The reviewer's first line when it found nothing wrong.
pub const VERDICT_HOLDS_UP: &str = "VERDICT: holds up";
/// The reviewer's first line when it found problems.
pub const VERDICT_PROBLEMS: &str = "VERDICT: problems";

/// Whether `review` says the answer holds up. Then no follow-up turn is
/// worth its tokens: the surface shows [`second_opinion_agrees`] instead of
/// drafting one. Anything else, including a reviewer that ignored the
/// verdict line, is treated as having problems.
pub fn review_holds_up(review: &str) -> bool {
    review
        .lines()
        .map(|line| line.trim().trim_matches(|c| c == '*' || c == '`').trim())
        .find(|line| !line.is_empty())
        .is_some_and(|line| line.eq_ignore_ascii_case(VERDICT_HOLDS_UP))
}

/// The notice shown when the reviewer agrees.
pub fn second_opinion_agrees(reviewer: AgentProvider) -> String {
    format!(
        "{} checked the last answer and found no problems.",
        provider_name(reviewer)
    )
}

/// What the follow-up asks of the main agent. A chat answer has no code to
/// check, and a changed answer must be stated again in full.
pub const SECOND_OPINION_FOLLOWUP_ASK: &str = "Check each point (against the code, where there is \
code). Fix what is right, and say briefly why you disagree with the rest. If your answer \
changes, state the final answer again in full.";

/// Longest answer and task text sent to the reviewer, in characters. A
/// review needs the claims, not every token of a long transcript.
pub const SECOND_OPINION_MAX_CHARS: usize = 24_000;

/// Most model requests a Claude reviewer may make; bounds its cost.
pub const SECOND_OPINION_MAX_TURNS: u32 = 16;

/// The turn prompt that asks the current agent for a handoff brief.
pub fn handoff_prompt(goal: &str) -> String {
    let goal = match goal.trim() {
        "" => HANDOFF_DEFAULT_GOAL,
        goal => goal,
    };
    format!(
        "Write a handoff brief for a new agent thread that starts with no history. \
Its goal: {goal}.\n\n\
Do not use tools; write from this conversation. Address the next agent directly. Include only \
what that goal needs:\n\
- the goal, restated precisely\n\
- what is done and what was decided, with the reason for each decision\n\
- exact file paths, symbols, commands and values that matter\n\
- what was tried and failed, so it is not repeated\n\
- open problems and the next concrete steps\n\n\
Reply with the brief only: no preamble, no sign-off."
    )
}

/// The provider that gives a second opinion on `provider`'s work.
pub fn reviewer_for(provider: AgentProvider) -> AgentProvider {
    match provider {
        AgentProvider::Claude => AgentProvider::Codex,
        AgentProvider::Codex => AgentProvider::Claude,
    }
}

/// Display name of a provider in review text.
pub fn provider_name(provider: AgentProvider) -> &'static str {
    match provider {
        AgentProvider::Claude => "Claude",
        AgentProvider::Codex => "Codex",
    }
}

/// The reviewer's prompt: the task, the answer under review, and how to
/// report. Long text keeps its end, where conclusions usually are.
pub fn second_opinion_prompt(focus: &str, task: &str, answer: &str) -> String {
    let focus = match focus.trim() {
        "" => String::new(),
        focus => format!("\nPay particular attention to: {focus}\n"),
    };
    format!(
        "You are a second reviewer of another agent's answer. You can read files in this \
directory and run read-only commands; you cannot edit.\n\n\
<task>\n{}\n</task>\n\n<answer>\n{}\n</answer>\n{focus}\n\
Verify the answer independently: work the problem out yourself before comparing, and check \
claims about code against the files. Start your reply with exactly one line: \
`{VERDICT_HOLDS_UP}` if you found no problems, or `{VERDICT_PROBLEMS}` followed by the \
concrete problems, most important first: wrong results, bugs, wrong assumptions, missed cases, \
a simpler approach. At most 8 bullets, each with file:line when it applies. Say nothing about \
what is fine.",
        keep_tail(task.trim(), SECOND_OPINION_MAX_CHARS / 4),
        keep_tail(answer.trim(), SECOND_OPINION_MAX_CHARS),
    )
}

/// The composer draft that hands the review back to the main agent.
pub fn second_opinion_followup(reviewer: AgentProvider, review: &str) -> String {
    format!(
        "A second reviewer ({}) looked at your last answer:\n\n{}\n\n\
{SECOND_OPINION_FOLLOWUP_ASK}",
        provider_name(reviewer),
        review.trim()
    )
}

/// `text` cut to its last `max` characters, marked when cut.
fn keep_tail(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let tail: String = text.chars().skip(count - max).collect();
    format!("[…earlier text omitted]\n{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handoff_names_the_goal_and_forbids_tools() {
        let prompt = handoff_prompt("  ship the meter  ");
        assert!(prompt.contains("Its goal: ship the meter."));
        assert!(prompt.contains("Do not use tools"));
        assert!(handoff_prompt("").contains(HANDOFF_DEFAULT_GOAL));
    }

    #[test]
    fn the_reviewer_is_the_other_provider() {
        assert_eq!(reviewer_for(AgentProvider::Claude), AgentProvider::Codex);
        assert_eq!(reviewer_for(AgentProvider::Codex), AgentProvider::Claude);
    }

    #[test]
    fn second_opinion_keeps_the_end_of_a_long_answer() {
        let answer = format!("{}CONCLUSION", "x".repeat(SECOND_OPINION_MAX_CHARS * 2));
        let prompt = second_opinion_prompt("races", "fix it", &answer);
        assert!(prompt.contains("CONCLUSION"));
        assert!(prompt.contains("[…earlier text omitted]"));
        assert!(prompt.contains("Pay particular attention to: races"));
        assert!(prompt.len() < SECOND_OPINION_MAX_CHARS * 2);
        assert!(!second_opinion_prompt("", "t", "a").contains("particular attention"));
    }

    #[test]
    fn the_followup_names_the_reviewer() {
        let draft = second_opinion_followup(AgentProvider::Codex, " - bug at a.rs:3 ");
        assert!(draft.starts_with("A second reviewer (Codex)"));
        assert!(draft.contains("- bug at a.rs:3\n"));
        assert!(draft.ends_with(SECOND_OPINION_FOLLOWUP_ASK));
    }

    /// A review that agrees ends there: no follow-up turn. Measured: of 15
    /// Claude reviews of Codex chat answers, 14 agreed, and each follow-up
    /// turn then cost ~25k tokens for nothing.
    #[test]
    fn an_agreeing_review_needs_no_follow_up() {
        assert!(review_holds_up("VERDICT: holds up\n"));
        assert!(review_holds_up("\n**VERDICT: holds up**"));
        assert!(review_holds_up("`verdict: Holds Up`"));
        assert!(!review_holds_up("VERDICT: problems\n- calc.py:2 subtracts"));
        assert!(
            !review_holds_up("Working through it: the answer holds up."),
            "no verdict line"
        );
        assert!(!review_holds_up(""));
        assert_eq!(
            second_opinion_agrees(AgentProvider::Codex),
            "Codex checked the last answer and found no problems."
        );
        assert!(second_opinion_prompt("", "t", "a").contains(VERDICT_HOLDS_UP));
    }

    /// The reviewer solves the problem itself, so a chat answer (no code to
    /// read) still gets a real check.
    #[test]
    fn the_reviewer_works_the_problem_out_independently() {
        let prompt = second_opinion_prompt("", "How many km is 26.2 miles?", "40 km");
        assert!(prompt.contains("work the problem out yourself"));
        assert!(!prompt.contains("coding agent"), "not only for code");
    }
}
