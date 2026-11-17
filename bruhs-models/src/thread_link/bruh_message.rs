//! `message_bruh`: an agent in a chat thread hands work to a bruh, an
//! always-on agent of the crew.
//!
//! A bruh's chat lives in the background service, not in the app, so the
//! app sends the message there as a `Chat` with a run id it picked, and a
//! sender that waits reads that run until it ends ([`chat_answer`]). The
//! message carries the same header as a message between threads, so the
//! hop limit holds across both. Bruhs have no thread tools, so a chain
//! never goes on from one.

use super::{
    bruh_delivery_prompt, clip, header_title, hop_limit_reply, next_hop, prompt_hops, short_id,
    Answer, ThreadReply, ANSWER_CHARS, MIN_PREFIX_CHARS, READ_MESSAGE_CHARS,
};
use crate::crew::{BruhState, BruhStatus, STOPPED_LINE};
use crate::reactive::{ReactiveRun, RunStatus};

/// How often an app reads a waited-for bruh run while it has no news.
pub const BRUH_WAIT_POLL_MS: u32 = 2_000;

/// One `message_bruh` call, as the app sees it.
#[derive(Debug, Clone, Copy)]
pub struct BruhOutgoing<'a> {
    pub caller_id: &'a str,
    pub caller_title: &'a str,
    /// The prompt of the turn that is sending.
    pub caller_prompt: &'a str,
    /// Every thread the app has, to shorten the caller's id.
    pub ids: &'a [&'a str],
    pub name_or_id: &'a str,
    pub message: &'a str,
    /// The sender will wait for the reply, and the app can let it.
    pub wait: bool,
    /// The user's hop limit.
    pub max_hops: u32,
}

/// A message ready to send to a bruh: [`plan_bruh_message`]'s approval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BruhDelivery {
    pub agent_id: String,
    pub name: String,
    /// The chat message to send, header included.
    pub prompt: String,
}

/// The bruh `reference` names: its whole id, else the one bruh with that
/// name (any case), else the one bruh whose id starts with it.
///
/// # Errors
///
/// The refusal to return to the model when no bruh, or more than one, fits.
pub fn resolve_bruh<'a>(
    reference: &str,
    bruhs: &'a [BruhState],
) -> Result<&'a BruhState, ThreadReply> {
    let reference = reference.trim();
    if let Some(exact) = bruhs.iter().find(|bruh| bruh.agent_id == reference) {
        return Ok(exact);
    }
    let wanted = reference.to_lowercase();
    let named: Vec<&BruhState> = bruhs
        .iter()
        .filter(|bruh| bruh.name.trim().to_lowercase() == wanted)
        .collect();
    let candidates = if named.is_empty() && reference.chars().count() >= MIN_PREFIX_CHARS {
        bruhs
            .iter()
            .filter(|bruh| bruh.agent_id.starts_with(reference))
            .collect()
    } else {
        named
    };
    match candidates.as_slice() {
        [only] => Ok(only),
        [] => Err(ThreadReply::refused(format!(
            "Not sent: no bruh is called \"{}\". list_threads names them.",
            header_title(reference)
        ))),
        several => Err(ThreadReply::refused(format!(
            "Not sent: \"{}\" matches more than one bruh: {}. Use the id from list_threads.",
            header_title(reference),
            several
                .iter()
                .map(|bruh| header_title(&bruh.name))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

/// Decide a `message_bruh` call.
///
/// # Errors
///
/// The refusal to return to the model: the hop limit, an unknown or
/// ambiguous bruh, or one that is paused or off.
pub fn plan_bruh_message(
    outgoing: &BruhOutgoing<'_>,
    bruhs: &[BruhState],
) -> Result<BruhDelivery, ThreadReply> {
    let max_hops = outgoing.max_hops;
    let Some(hops) = next_hop(prompt_hops(outgoing.caller_prompt), max_hops) else {
        return Err(hop_limit_reply(max_hops));
    };
    let bruh = resolve_bruh(outgoing.name_or_id, bruhs)?;
    let name = header_title(&bruh.name);
    match bruh.status {
        BruhStatus::Paused => {
            return Err(ThreadReply::refused(format!(
                "Not sent: {name} is paused. Tell the user; they can resume it."
            )))
        }
        BruhStatus::Off => {
            return Err(ThreadReply::refused(format!(
                "Not sent: {name} is off. Tell the user."
            )))
        }
        BruhStatus::Asleep | BruhStatus::Working | BruhStatus::NeedsYou => {}
    }
    Ok(BruhDelivery {
        agent_id: bruh.agent_id.clone(),
        name,
        prompt: bruh_delivery_prompt(
            outgoing.caller_title,
            short_id(outgoing.caller_id, outgoing.ids),
            hops,
            max_hops,
            outgoing.wait,
            outgoing.message,
        ),
    })
}

/// The `message_bruh` reply once the message is sent and nobody waits.
pub fn bruh_sent_reply(name: &str) -> ThreadReply {
    let name = header_title(name);
    ThreadReply::ok(format!(
        "Sent to {name}. Its answer stays in {name}'s chat in the crew and does not come back here, so finish this turn."
    ))
}

/// The `message_bruh` reply when the sender waited: the bruh's answer, or
/// why there is none.
pub fn bruh_answer_reply(name: &str, answer: Answer<'_>) -> ThreadReply {
    let name = header_title(name);
    match answer {
        Answer::Replied(text) if text.trim().is_empty() => ThreadReply::ok(format!(
            "{name} finished without writing a reply. Its chat in the crew shows what it did."
        )),
        Answer::Replied(text) => ThreadReply::ok(format!(
            "{name} answered:\n\n{}",
            clip(text.trim(), ANSWER_CHARS)
        )),
        Answer::Stopped => ThreadReply::refused(format!("{name} was stopped before it answered.")),
        Answer::Failed(error) => ThreadReply::refused(format!(
            "{name} could not answer: {}",
            clip(error.trim(), READ_MESSAGE_CHARS)
        )),
        Answer::Removed => ThreadReply::refused(format!("{name} never ran the message.")),
    }
}

/// How the chat run `run_id` ended, following the run it was merged into.
/// `None` while it is queued or running, or while `runs` does not show it.
pub fn chat_answer<'a>(runs: &'a [ReactiveRun], run_id: &str) -> Option<Answer<'a>> {
    let mut id = run_id;
    // Each step moves to another run, so a loop of merges ends here.
    for _ in 0..=runs.len() {
        let run = runs.iter().find(|run| run.id == id)?;
        match run.status {
            RunStatus::Merged => id = run.merged_into.as_deref()?,
            RunStatus::Completed => {
                return Some(Answer::Replied(run.result.as_deref().unwrap_or("")))
            }
            RunStatus::Cancelled => return Some(Answer::Stopped),
            RunStatus::Failed | RunStatus::Interrupted => {
                return Some(Answer::Failed(run.error.as_deref().unwrap_or(STOPPED_LINE)))
            }
            RunStatus::AwaitingReview | RunStatus::Queued | RunStatus::Running => return None,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crew::{BruhAccess, BruhLook, BruhState, BruhStatus};
    use crate::reactive::{ReactiveRun, RunStatus};
    use crate::thread_link::{
        delivery_prompt, parse_delivery, Answer, ThreadOperation, MAX_HOPS, MAX_MESSAGE_CHARS,
    };

    const ADA: &str = "a1b2c3d4-0000-4000-8000-000000000000";
    const LOWER_ADA: &str = "ffff0000-0000-4000-8000-000000000000";
    const CALLER: &str = "11111111-aaaa-4aaa-8aaa-aaaaaaaaaaaa";

    fn bruh(id: &str, name: &str, status: BruhStatus) -> BruhState {
        BruhState {
            agent_id: id.into(),
            name: name.into(),
            look: BruhLook::default(),
            access: BruhAccess::default(),
            effort: None,
            status,
            next_wake_ms: None,
            unread: 0,
        }
    }

    fn crew() -> Vec<BruhState> {
        vec![
            bruh(ADA, "Ada", BruhStatus::Asleep),
            bruh(LOWER_ADA, "ada", BruhStatus::Working),
            bruh("b9", "Bo", BruhStatus::Asleep),
        ]
    }

    fn outgoing<'a>(
        to: &'a str,
        message: &'a str,
        wait: bool,
        caller_prompt: &'a str,
    ) -> BruhOutgoing<'a> {
        BruhOutgoing {
            caller_id: CALLER,
            caller_title: "Sender",
            caller_prompt,
            ids: &[CALLER],
            name_or_id: to,
            message,
            wait,
            max_hops: MAX_HOPS,
        }
    }

    fn run(id: &str, status: RunStatus) -> ReactiveRun {
        let mut run: ReactiveRun = serde_json::from_value(serde_json::json!({
            "id": id, "agent_id": "b9", "source_id": null, "subscription_id": null,
            "event_id": null, "status": "queued", "prompt": id, "thread_id": "t",
            "session_id": null, "created_at": 1, "updated_at": 1,
            "not_before": 1, "result": null, "error": null,
        }))
        .unwrap();
        run.status = status;
        run
    }

    #[test]
    fn a_bruh_is_found_by_id_name_or_prefix_and_ambiguity_is_refused() {
        let bruhs = crew();
        assert_eq!(resolve_bruh("Bo", &bruhs).unwrap().agent_id, "b9");
        assert_eq!(resolve_bruh(" bo ", &bruhs).unwrap().agent_id, "b9");
        assert_eq!(resolve_bruh("b9", &bruhs).unwrap().name, "Bo");
        assert_eq!(resolve_bruh(ADA, &bruhs).unwrap().name, "Ada");
        assert_eq!(resolve_bruh("a1b2", &bruhs).unwrap().name, "Ada");
        // Three characters are too few for a prefix.
        assert!(!resolve_bruh("a1b", &bruhs).unwrap_err().ok);
        let ambiguous = resolve_bruh("ADA", &bruhs).unwrap_err();
        assert!(!ambiguous.ok);
        assert_eq!(
            ambiguous.text,
            "Not sent: \"ADA\" matches more than one bruh: Ada, ada. Use the id from list_threads."
        );
        let unknown = resolve_bruh("Zed", &bruhs).unwrap_err();
        assert!(!unknown.ok);
        assert_eq!(
            unknown.text,
            "Not sent: no bruh is called \"Zed\". list_threads names them."
        );
        let prefixes = [
            bruh("abcd0001", "One", BruhStatus::Asleep),
            bruh("abcd0002", "Two", BruhStatus::Asleep),
        ];
        assert!(resolve_bruh("abcd", &prefixes)
            .unwrap_err()
            .text
            .contains("more than one bruh: One, Two"));
    }

    #[test]
    fn a_bruh_message_carries_the_thread_header_and_the_hop_rule() {
        let bruhs = crew();
        let plan =
            plan_bruh_message(&outgoing("Bo", "check the notes", false, ""), &bruhs).unwrap();
        assert_eq!(plan.agent_id, "b9");
        assert_eq!(plan.name, "Bo");
        let view = parse_delivery(&plan.prompt).unwrap();
        assert_eq!(view.hops, 1);
        assert_eq!(view.from_title, "Sender");
        assert_eq!(view.from_id, "11111111");
        assert_eq!(view.body, "check the notes");
        assert!(!view.awaited);
        // A bruh has no thread tools: nothing tells it to message back.
        assert!(!plan.prompt.contains("message_thread"));
        assert!(plan
            .prompt
            .contains("Its agent does not see your reply; it stays in this chat."));

        let waited =
            plan_bruh_message(&outgoing("Bo", "sum it", true, &plan.prompt), &bruhs).unwrap();
        let view = parse_delivery(&waited.prompt).unwrap();
        assert_eq!(view.hops, 2);
        assert!(view.awaited);
        assert_eq!(
            (view.from_title, view.from_id, view.body),
            ("Sender", "11111111", "sum it")
        );
        assert!(waited
            .prompt
            .contains("your final reply to this message is returned to it"));
        assert!(!waited.prompt.contains("message_thread"));

        let at_limit = delivery_prompt("Sender", "11111111", MAX_HOPS, MAX_HOPS, false, "x");
        let refused =
            plan_bruh_message(&outgoing("Bo", "again", false, &at_limit), &bruhs).unwrap_err();
        assert!(!refused.ok);
        assert!(refused.text.contains("reached its limit of 6 messages"));

        let mut held = crew();
        held[2].status = BruhStatus::Paused;
        assert_eq!(
            plan_bruh_message(&outgoing("Bo", "go", false, ""), &held)
                .unwrap_err()
                .text,
            "Not sent: Bo is paused. Tell the user; they can resume it."
        );
        held[2].status = BruhStatus::Off;
        assert_eq!(
            plan_bruh_message(&outgoing("Bo", "go", false, ""), &held)
                .unwrap_err()
                .text,
            "Not sent: Bo is off. Tell the user."
        );
        // Busy bruhs still take work: it queues behind what they are doing.
        for status in [BruhStatus::Working, BruhStatus::NeedsYou] {
            held[2].status = status;
            assert!(plan_bruh_message(&outgoing("Bo", "go", false, ""), &held).is_ok());
        }
        assert!(
            !plan_bruh_message(&outgoing("Zed", "go", false, ""), &bruhs)
                .unwrap_err()
                .ok
        );
    }

    #[test]
    fn a_waited_chat_answers_from_its_run_or_the_run_it_merged_into() {
        let mut done = run("r1", RunStatus::Completed);
        done.result = Some("done".into());
        assert_eq!(chat_answer(&[done], "r1"), Some(Answer::Replied("done")));

        let cancelled = run("r1", RunStatus::Cancelled);
        assert_eq!(chat_answer(&[cancelled], "r1"), Some(Answer::Stopped));

        let mut failed = run("r1", RunStatus::Failed);
        failed.error = Some("x".into());
        assert_eq!(chat_answer(&[failed], "r1"), Some(Answer::Failed("x")));
        let interrupted = run("r1", RunStatus::Interrupted);
        assert_eq!(
            chat_answer(&[interrupted], "r1"),
            Some(Answer::Failed(STOPPED_LINE))
        );

        let mut merged = run("r2", RunStatus::Merged);
        merged.merged_into = Some("r3".into());
        let mut both = run("r3", RunStatus::Completed);
        both.result = Some("both".into());
        assert_eq!(
            chat_answer(&[merged.clone(), both], "r2"),
            Some(Answer::Replied("both"))
        );
        // Merged into a run that is still going: no answer yet.
        assert_eq!(
            chat_answer(&[merged.clone(), run("r3", RunStatus::Running)], "r2"),
            None
        );
        // A loop of merges never answers and never spins.
        let mut back = run("r3", RunStatus::Merged);
        back.merged_into = Some("r2".into());
        assert_eq!(chat_answer(&[merged, back], "r2"), None);

        assert_eq!(chat_answer(&[run("r1", RunStatus::Queued)], "r1"), None);
        assert_eq!(chat_answer(&[run("r1", RunStatus::Running)], "r1"), None);
        assert_eq!(chat_answer(&[], "r1"), None);

        assert_eq!(
            bruh_answer_reply("Bo", Answer::Replied("done")).text,
            "Bo answered:\n\ndone"
        );
        assert!(bruh_answer_reply("Bo", Answer::Replied("done")).ok);
        assert!(bruh_answer_reply("Bo", Answer::Replied("  ")).ok);
        let stopped = bruh_answer_reply("Bo", Answer::Stopped);
        assert_eq!(
            (stopped.ok, stopped.text.as_str()),
            (false, "Bo was stopped before it answered.")
        );
        let failed = bruh_answer_reply("Bo", Answer::Failed("no network"));
        assert_eq!(
            (failed.ok, failed.text.as_str()),
            (false, "Bo could not answer: no network")
        );
        let removed = bruh_answer_reply("Bo", Answer::Removed);
        assert_eq!(
            (removed.ok, removed.text.as_str()),
            (false, "Bo never ran the message.")
        );
        let sent = bruh_sent_reply("Bo");
        assert!(sent.ok);
        assert_eq!(
            sent.text,
            "Sent to Bo. Its answer stays in Bo's chat in the crew and does not come back here, so finish this turn."
        );
    }

    #[test]
    fn message_bruh_is_a_write_and_round_trips() {
        let op: ThreadOperation = serde_json::from_str(
            r#"{"op":"message_bruh","name_or_id":"Bo","message":"hi","wait":true}"#,
        )
        .unwrap();
        assert!(op.writes() && op.waits() && op.validate().is_ok());
        assert_eq!(
            op,
            ThreadOperation::MessageBruh {
                name_or_id: "Bo".into(),
                message: "hi".into(),
                wait: true,
            }
        );
        let plain: ThreadOperation =
            serde_json::from_str(r#"{"op":"message_bruh","name_or_id":"Bo","message":"hi"}"#)
                .unwrap();
        assert!(plain.writes() && !plain.waits());
        // An unset wait stays off the wire.
        assert_eq!(
            serde_json::to_string(&plain).unwrap(),
            r#"{"op":"message_bruh","name_or_id":"Bo","message":"hi"}"#
        );
        assert!(serde_json::from_str::<ThreadOperation>(
            r#"{"op":"message_bruh","name_or_id":"Bo","message":"hi","access":"full"}"#
        )
        .is_err());

        let bruh_op = |name_or_id: &str, message: String| ThreadOperation::MessageBruh {
            name_or_id: name_or_id.into(),
            message,
            wait: false,
        };
        let not_a_bruh = Err("name_or_id is not a bruh name or id.");
        assert_eq!(bruh_op(" ", "hi".into()).validate(), not_a_bruh);
        assert_eq!(
            bruh_op(&"x".repeat(129), "hi".into()).validate(),
            not_a_bruh
        );
        assert!(bruh_op(&"x".repeat(128), "hi".into()).validate().is_ok());
        assert_eq!(
            bruh_op("Bo", " ".into()).validate(),
            Err("message is empty.")
        );
        assert_eq!(
            bruh_op("Bo", "m".repeat(MAX_MESSAGE_CHARS + 1)).validate(),
            Err("message is too long.")
        );
    }
}
