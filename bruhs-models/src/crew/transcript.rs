//! One bruh's conversation as every surface shows it, the Stop fallback for
//! older services and the commands a settings edit turns into.

use super::{crew_commands_ready, BruhAccess, BruhDraft, BruhLook, BruhState};
use crate::reactive::{
    OutboxItem, OutboxKind, ReactiveAgent, ReactiveChange, ReactiveCommand, ReactiveRun, RunStatus,
    RunTrigger,
};
use crate::thread_link::parse_delivery;
use crate::EpochMs;

/// One line of a bruh's conversation.
#[cfg_attr(
    not(target_arch = "wasm32"),
    expect(
        clippy::large_enum_variant,
        reason = "cards are a minority of lines; boxing would change the shared API"
    )
)]
#[derive(Debug, Clone, PartialEq)]
pub enum TranscriptItem {
    /// The owner's own words.
    Owner {
        run_id: String,
        text: String,
        at: EpochMs,
        /// The bruh has not answered yet.
        pending: bool,
        /// Who sent it when another thread's agent did, as
        /// [`crate::thread_link::DeliveryView::caption`] words it; `text` is then its body.
        caption: Option<String>,
    },
    /// What the bruh said or reported.
    Bruh {
        run_id: String,
        text: String,
        at: EpochMs,
        /// What woke the bruh; `None` for a reply to the owner.
        trigger: Option<&'static str>,
        failed: bool,
    },
    /// A question or approval waiting inline.
    Card { item: OutboxItem },
    /// A run waiting for the owner's review before it starts: Run sends
    /// `ApproveRun`, Dismiss sends `CancelRun`.
    Review {
        run_id: String,
        /// What woke the bruh.
        trigger: &'static str,
        /// What the run was asked to do.
        prompt: String,
        at: EpochMs,
    },
}

impl TranscriptItem {
    fn at(&self) -> EpochMs {
        match self {
            Self::Owner { at, .. } | Self::Bruh { at, .. } | Self::Review { at, .. } => *at,
            Self::Card { item } => item.created_at,
        }
    }

    fn order(&self) -> (u8, &str) {
        match self {
            Self::Owner { run_id, .. } => (0, run_id),
            Self::Bruh { run_id, .. } | Self::Review { run_id, .. } => (1, run_id),
            Self::Card { item } => (2, &item.id),
        }
    }
}

/// What a bruh says for a run that ended without a result or an error.
pub const STOPPED_LINE: &str = "Stopped.";

fn is_owner_line(run: &ReactiveRun) -> bool {
    match &run.trigger {
        Some(RunTrigger::Chat) => true,
        Some(_) => false,
        None => run.merged_runs.is_empty() && run.owner_message.as_deref() == Some(&run.prompt),
    }
}

/// The conversation with one bruh: the owner's messages, what it answered or
/// reported, its open questions and approvals, and the runs waiting for the
/// owner's review, oldest first. A run that ended without a word reads as
/// [`STOPPED_LINE`], or as its reason when it was cancelled with one.
#[must_use]
pub fn transcript(
    agent_id: &str,
    runs: &[ReactiveRun],
    outbox: &[OutboxItem],
) -> Vec<TranscriptItem> {
    let mut items = Vec::new();
    for run in runs.iter().filter(|run| run.agent_id == agent_id) {
        let chat = matches!(run.trigger, Some(RunTrigger::Chat));
        if is_owner_line(run) {
            let said = run.owner_message.as_deref().unwrap_or(&run.prompt);
            let (text, caption) = match parse_delivery(said) {
                Some(view) => (view.body.to_string(), Some(view.caption())),
                None => (said.to_string(), None),
            };
            items.push(TranscriptItem::Owner {
                run_id: run.id.clone(),
                text,
                at: run.created_at,
                pending: run.status.is_pending() || run.status == RunStatus::Running,
                caption,
            });
        }
        if run.status == RunStatus::Merged {
            continue;
        }
        let trigger = run.trigger.as_ref().map_or("Event", RunTrigger::label);
        if run.status == RunStatus::AwaitingReview {
            items.push(TranscriptItem::Review {
                run_id: run.id.clone(),
                trigger,
                prompt: run.prompt.clone(),
                at: run.created_at,
            });
            continue;
        }
        let room_reply = run.status == RunStatus::Completed
            && run.trigger.as_ref().and_then(RunTrigger::room_id).is_some()
            && run
                .result
                .as_deref()
                .is_none_or(|text| text.trim().is_empty());
        if room_reply && super::room::asked_only(run, outbox) {
            continue;
        }
        let (text, failed) = match (&run.result, &run.error) {
            // A room run that wrote no reply says what the room shows.
            _ if room_reply => (super::room::room_reply_text(run), false),
            (Some(result), _) => (result.clone(), false),
            (None, Some(error))
                if matches!(run.status, RunStatus::Failed | RunStatus::Interrupted) =>
            {
                (error.clone(), true)
            }
            (None, None) if matches!(run.status, RunStatus::Failed | RunStatus::Interrupted) => {
                (STOPPED_LINE.to_string(), run.status == RunStatus::Failed)
            }
            (None, error) if run.status == RunStatus::Cancelled => (
                error.clone().unwrap_or_else(|| STOPPED_LINE.to_string()),
                false,
            ),
            _ => continue,
        };
        items.push(TranscriptItem::Bruh {
            run_id: run.id.clone(),
            text,
            at: run.updated_at,
            trigger: (!chat).then_some(trigger),
            failed,
        });
    }
    items.extend(
        outbox
            .iter()
            .filter(|item| item.agent_id == agent_id && item.kind == OutboxKind::Ask)
            .map(|item| TranscriptItem::Card { item: item.clone() }),
    );
    items.sort_by(|a, b| a.at().cmp(&b.at()).then_with(|| a.order().cmp(&b.order())));
    items
}

/// What Stop sends: one command to a current service, a cancel per live run
/// to an older one that rejects the newer commands.
#[must_use]
pub fn stop_commands(protocol: u32, runs: &[ReactiveRun]) -> Vec<ReactiveCommand> {
    if crew_commands_ready(protocol) {
        return vec![ReactiveCommand::StopAll];
    }
    runs.iter()
        .filter(|run| run.status.is_pending() || run.status == RunStatus::Running)
        .map(|run| ReactiveCommand::CancelRun { id: run.id.clone() })
        .collect()
}

/// What ⌘. says once every stop command went through.
pub const STOPPED_ALL: &str = "Stopped all bruhs.";

/// Send each stop command in order, going on past a failure, so one
/// rejected cancel never keeps the rest from stopping. Returns each error.
pub async fn send_stop_commands<F, Fut>(commands: Vec<ReactiveCommand>, mut send: F) -> Vec<String>
where
    F: FnMut(ReactiveCommand) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let mut failed = Vec::new();
    for command in commands {
        if let Err(error) = send(command).await {
            failed.push(error);
        }
    }
    failed
}

/// What ⌘. says after its sends: [`STOPPED_ALL`] only when none failed.
pub fn stop_all_result(failed: &[String]) -> Result<&'static str, String> {
    match failed {
        [] => Ok(STOPPED_ALL),
        [only] => Err(format!("Couldn't stop every bruh: {only}")),
        [first, rest @ ..] => Err(format!(
            "Couldn't stop every bruh: {first} ({} more failed)",
            rest.len()
        )),
    }
}

/// A bruh's settings as the editor holds them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BruhSettings {
    pub name: String,
    pub instructions: String,
    pub model: String,
    pub effort: Option<String>,
    pub look: BruhLook,
    pub access: BruhAccess,
}

/// The commands a settings edit needs: the agent first, then the bruh. A look
/// only edit never touches the agent.
#[must_use]
pub fn settings_commands(
    agent: &ReactiveAgent,
    state: &BruhState,
    edited: &BruhSettings,
) -> Vec<ReactiveCommand> {
    let mut commands = Vec::new();
    if agent.name != edited.name
        || agent.instructions != edited.instructions
        || agent.model != edited.model
    {
        commands.push(ReactiveCommand::Apply {
            change: ReactiveChange::SaveAgent {
                agent: ReactiveAgent {
                    name: edited.name.clone(),
                    instructions: edited.instructions.clone(),
                    model: edited.model.clone(),
                    ..agent.clone()
                },
            },
        });
    }
    if state.look != edited.look || state.access != edited.access || state.effort != edited.effort {
        commands.push(ReactiveCommand::SetBruh {
            agent_id: agent.id.clone(),
            look: edited.look.clone(),
            access: edited.access.clone(),
            effort: edited.effort.clone(),
        });
    }
    commands
}

/// The command that makes a new bruh from a draft.
#[must_use]
pub fn create_command(
    id: String,
    draft: &BruhDraft,
    look: BruhLook,
    project_path: Option<String>,
) -> ReactiveCommand {
    ReactiveCommand::CreateBruh {
        agent: ReactiveAgent {
            id,
            name: draft.name.clone(),
            instructions: draft.instructions.clone(),
            memory: String::new(),
            model: draft.model.clone(),
            project_path: project_path.unwrap_or_default(),
            enabled: true,
        },
        look,
        access: BruhAccess::default(),
        effort: draft.effort.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crew::{AccessLevel, BruhColor, BruhStatus, CREW_PROTOCOL};
    use std::future::Future as _;

    fn at(ms: i64) -> EpochMs {
        EpochMs::saturating_from_i64(ms)
    }

    fn run(id: &str, agent: &str, status: &str, created: i64, updated: i64) -> ReactiveRun {
        serde_json::from_value(serde_json::json!({
            "id": id, "agent_id": agent, "source_id": null, "subscription_id": null,
            "event_id": null, "status": status, "prompt": id, "thread_id": "t",
            "session_id": null, "created_at": created, "updated_at": updated,
            "not_before": created, "result": null, "error": null,
        }))
        .unwrap()
    }

    fn item(id: &str, agent: &str, kind: OutboxKind, created: i64) -> OutboxItem {
        serde_json::from_value(serde_json::json!({
            "id": id, "agent_id": agent, "run_id": "r", "kind": kind, "title": "t",
            "message": "m", "created_at": created,
        }))
        .unwrap()
    }

    fn chat(id: &str, status: &str, created: i64, updated: i64, text: &str) -> ReactiveRun {
        let mut r = run(id, "a", status, created, updated);
        r.trigger = Some(RunTrigger::Chat);
        r.prompt = text.to_string();
        r
    }

    #[test]
    fn chat_runs_read_as_a_conversation_with_cards_inline() {
        let mut hi = chat("c1", "completed", 1, 2, "hi");
        hi.result = Some("hello".into());
        let mut event = run("e1", "a", "completed", 3, 4);
        event.trigger = Some(RunTrigger::Event);
        event.result = Some("Bill found".into());
        let mut merged = chat("c2", "merged", 5, 5, "and this");
        merged.merged_into = Some("c3".into());
        let mut other = run("o1", "b", "completed", 1, 2);
        other.result = Some("not mine".into());

        let mut approval = item("i1", "a", OutboxKind::Ask, 6);
        approval.approval = Some(crate::crew::ApprovalRequest {
            tool: "Bash".into(),
            summary: "Run ls".into(),
            fingerprint: "f".into(),
            state: crate::crew::ApprovalState::Waiting,
            detail: None,
        });
        let notify = item("i2", "a", OutboxKind::Notify, 7);
        let elsewhere = item("i3", "b", OutboxKind::Ask, 8);

        let got = transcript(
            "a",
            &[hi, event, merged, other],
            &[approval.clone(), notify, elsewhere],
        );
        assert_eq!(
            got,
            vec![
                TranscriptItem::Owner {
                    run_id: "c1".into(),
                    text: "hi".into(),
                    at: at(1),
                    pending: false,
                    caption: None
                },
                TranscriptItem::Bruh {
                    run_id: "c1".into(),
                    text: "hello".into(),
                    at: at(2),
                    trigger: None,
                    failed: false
                },
                TranscriptItem::Bruh {
                    run_id: "e1".into(),
                    text: "Bill found".into(),
                    at: at(4),
                    trigger: Some("Event"),
                    failed: false
                },
                TranscriptItem::Owner {
                    run_id: "c2".into(),
                    text: "and this".into(),
                    at: at(5),
                    pending: false,
                    caption: None
                },
                TranscriptItem::Card { item: approval },
            ]
        );
    }

    #[test]
    fn a_message_from_a_thread_reads_as_its_body_under_a_caption() {
        let prompt = crate::thread_link::delivery_prompt("Plan", "11111111", 1, 6, false, "check");
        let mut sent = chat("c1", "queued", 1, 1, &prompt);
        sent.owner_message = Some(prompt);
        let mut waited = chat(
            "c2",
            "completed",
            2,
            2,
            &crate::thread_link::delivery_prompt("Plan", "11111111", 1, 6, true, "sum"),
        );
        waited.result = Some("4".into());
        let got = transcript("a", &[sent, waited], &[]);
        assert_eq!(
            got[0],
            TranscriptItem::Owner {
                run_id: "c1".into(),
                text: "check".into(),
                at: at(1),
                pending: true,
                caption: Some("From thread “Plan”".into()),
            }
        );
        assert!(matches!(
            &got[1],
            TranscriptItem::Owner { text, caption: Some(caption), .. }
                if text == "sum" && caption == "From thread “Plan”. Waiting for this reply"
        ));
        // The owner's own words carry no caption.
        let typed = transcript("a", &[chat("c3", "queued", 1, 1, "hi")], &[]);
        assert!(
            matches!(&typed[0], TranscriptItem::Owner { caption: None, text, .. } if text == "hi")
        );
    }

    #[test]
    fn a_batched_chat_keeps_the_owners_words() {
        let mut batched = chat("c1", "running", 1, 1, "2 items arrived: first, second");
        batched.owner_message = Some("first".into());
        let got = transcript("a", &[batched], &[]);
        assert_eq!(
            got,
            vec![TranscriptItem::Owner {
                run_id: "c1".into(),
                text: "first".into(),
                at: at(1),
                pending: true,
                caption: None
            }]
        );
    }

    #[test]
    fn failed_runs_show_their_error_and_same_time_lines_sort_owner_first() {
        let mut failed = chat("c1", "failed", 5, 5, "go");
        failed.error = Some("no network".into());
        let mut quiet = run("e1", "a", "completed", 1, 1);
        quiet.trigger = Some(RunTrigger::Consolidation);
        let got = transcript("a", &[failed, quiet], &[]);
        assert_eq!(got.len(), 2);
        assert!(matches!(&got[0], TranscriptItem::Owner { text, .. } if text == "go"));
        assert_eq!(
            got[1],
            TranscriptItem::Bruh {
                run_id: "c1".into(),
                text: "no network".into(),
                at: at(5),
                trigger: None,
                failed: true
            }
        );
    }

    #[test]
    fn a_run_stopped_without_a_word_still_reads_as_stopped() {
        let stopped = chat("c1", "interrupted", 1, 2, "go");
        let mut failed = run("e1", "a", "failed", 3, 4);
        failed.trigger = Some(RunTrigger::Event);
        let cancelled = chat("c2", "cancelled", 5, 6, "never mind");
        let mut access = run("e2", "a", "cancelled", 7, 8);
        access.trigger = Some(RunTrigger::Event);
        access.error = Some("Access changed.".into());
        let got = transcript("a", &[stopped, failed, cancelled, access], &[]);
        let lines: Vec<_> = got
            .iter()
            .filter_map(|item| match item {
                TranscriptItem::Bruh {
                    run_id,
                    text,
                    failed,
                    ..
                } => Some((run_id.as_str(), text.as_str(), *failed)),
                _ => None,
            })
            .collect();
        assert_eq!(
            lines,
            vec![
                ("c1", STOPPED_LINE, false),
                ("e1", STOPPED_LINE, true),
                ("c2", STOPPED_LINE, false),
                ("e2", "Access changed.", false),
            ]
        );
    }

    #[test]
    fn a_room_run_without_a_reply_reads_as_what_the_room_shows() {
        use crate::reactive::{RunOutcome, RunReport};
        let room = |id: &str, result: Option<&str>, report: Option<(RunOutcome, &str, &str)>| {
            let mut r = run(id, "a", "completed", 1, 2);
            r.trigger = Some(RunTrigger::Room {
                room_id: "room".into(),
                message_id: "m".into(),
                hop: 0,
                resumed: false,
            });
            r.result = result.map(str::to_string);
            r.report = report.map(|(outcome, title, message)| RunReport {
                outcome,
                title: title.into(),
                message: message.into(),
                ..RunReport::default()
            });
            r
        };
        let runs = [
            room(
                "n",
                Some(""),
                Some((RunOutcome::Notify, "Hi", "Hello back.")),
            ),
            room(
                "t",
                Some("  "),
                Some((RunOutcome::Notify, "Only a title", "")),
            ),
            room("s", Some(""), None),
            room(
                "x",
                None,
                Some((RunOutcome::Silent, "Quiet", "Nothing to say.")),
            ),
            room(
                "q",
                Some(""),
                Some((RunOutcome::Ask, "Which?", "Work or personal?")),
            ),
            room(
                "r",
                Some("Done."),
                Some((RunOutcome::Notify, "Hi", "Hello back.")),
            ),
        ];
        let mut question = item("i1", "a", OutboxKind::Ask, 2);
        question.run_id = "q".into();
        let got = transcript("a", &runs, &[question]);
        let mut lines: Vec<_> = got
            .iter()
            .filter_map(|item| match item {
                TranscriptItem::Bruh { run_id, text, .. } => Some((run_id.as_str(), text.as_str())),
                _ => None,
            })
            .collect();
        lines.sort_unstable();
        assert_eq!(
            lines,
            vec![
                ("n", "Hello back."),
                ("r", "Done."),
                ("s", STOPPED_LINE),
                ("t", "Only a title"),
                ("x", STOPPED_LINE),
            ],
            "a question shows as its card only"
        );
    }

    #[test]
    fn a_run_awaiting_review_asks_the_owner_inline() {
        let mut waiting = run("e1", "a", "awaiting_review", 3, 3);
        waiting.trigger = Some(RunTrigger::Event);
        waiting.prompt = "Invoice arrived".into();
        let mut queued = run("e2", "a", "queued", 4, 4);
        queued.trigger = Some(RunTrigger::Event);
        let got = transcript("a", &[waiting, queued], &[]);
        assert_eq!(
            got,
            vec![TranscriptItem::Review {
                run_id: "e1".into(),
                trigger: "Event",
                prompt: "Invoice arrived".into(),
                at: at(3),
            }]
        );
    }

    #[test]
    fn stop_commands_fall_back_to_cancelling_each_run_on_an_old_service() {
        let runs = [
            run("q", "a", "queued", 1, 1),
            run("r", "a", "running", 1, 1),
            run("a", "a", "awaiting_review", 1, 1),
            run("c", "a", "completed", 1, 1),
        ];
        assert_eq!(
            stop_commands(CREW_PROTOCOL, &runs),
            vec![ReactiveCommand::StopAll]
        );
        let cancel = |id: &str| ReactiveCommand::CancelRun { id: id.into() };
        assert_eq!(
            stop_commands(0, &runs),
            vec![cancel("q"), cancel("r"), cancel("a")]
        );
    }

    #[test]
    fn stop_commands_go_on_past_a_failure_and_say_so() {
        let cancel = |id: &str| ReactiveCommand::CancelRun { id: id.into() };
        let mut tried = Vec::new();
        let failed = {
            let sending =
                send_stop_commands(vec![cancel("a"), cancel("b"), cancel("c")], |command| {
                    tried.push(command.clone());
                    std::future::ready(if command == cancel("b") {
                        Err("already ended".to_string())
                    } else {
                        Ok(())
                    })
                });
            let mut sending = std::pin::pin!(sending);
            let std::task::Poll::Ready(failed) = sending
                .as_mut()
                .poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
            else {
                panic!("ready sends finish at once");
            };
            failed
        };
        assert_eq!(tried, vec![cancel("a"), cancel("b"), cancel("c")]);
        assert_eq!(failed, vec!["already ended".to_string()]);
        assert_eq!(
            stop_all_result(&failed),
            Err("Couldn't stop every bruh: already ended".to_string())
        );
        assert_eq!(stop_all_result(&[]), Ok(STOPPED_ALL));
        assert_eq!(
            stop_all_result(&["x".into(), "y".into(), "z".into()]),
            Err("Couldn't stop every bruh: x (2 more failed)".to_string())
        );
    }

    fn current() -> (ReactiveAgent, BruhState, BruhSettings) {
        let agent = ReactiveAgent {
            id: "a".into(),
            name: "Bill".into(),
            instructions: "watch".into(),
            memory: "m".into(),
            model: "sonnet".into(),
            project_path: "/p".into(),
            enabled: true,
        };
        let state = BruhState {
            agent_id: "a".into(),
            name: "Bill".into(),
            look: BruhLook::default(),
            access: BruhAccess::default(),
            effort: None,
            status: BruhStatus::Asleep,
            next_wake_ms: None,
            unread: 0,
        };
        let edited = BruhSettings {
            name: "Bill".into(),
            instructions: "watch".into(),
            model: "sonnet".into(),
            effort: None,
            look: BruhLook::default(),
            access: BruhAccess::default(),
        };
        (agent, state, edited)
    }

    #[test]
    fn a_look_only_edit_never_touches_the_agent() {
        let (agent, state, mut edited) = current();
        assert_eq!(settings_commands(&agent, &state, &edited), vec![]);

        edited.look.face = BruhColor::Pandan;
        assert_eq!(
            settings_commands(&agent, &state, &edited),
            vec![ReactiveCommand::SetBruh {
                agent_id: "a".into(),
                look: edited.look.clone(),
                access: BruhAccess::default(),
                effort: None,
            }]
        );

        let (agent, state, mut edited) = current();
        edited.name = "Bob".into();
        let renamed = ReactiveAgent {
            name: "Bob".into(),
            ..agent.clone()
        };
        assert_eq!(
            settings_commands(&agent, &state, &edited),
            vec![ReactiveCommand::Apply {
                change: ReactiveChange::SaveAgent { agent: renamed }
            }]
        );

        edited.access.level = AccessLevel::Full;
        let both = settings_commands(&agent, &state, &edited);
        assert_eq!(both.len(), 2);
        assert!(matches!(&both[0], ReactiveCommand::Apply { .. }));
        assert!(matches!(
            &both[1],
            ReactiveCommand::SetBruh { access, .. } if access.level == AccessLevel::Full
        ));
    }

    #[test]
    fn a_new_bruh_starts_enabled_with_default_access() {
        let draft = BruhDraft {
            name: "Bill".into(),
            instructions: "watch".into(),
            model: "sonnet".into(),
            effort: Some("high".into()),
        };
        let command = create_command("a".into(), &draft, BruhLook::default(), None);
        assert_eq!(
            command,
            ReactiveCommand::CreateBruh {
                agent: ReactiveAgent {
                    id: "a".into(),
                    name: "Bill".into(),
                    instructions: "watch".into(),
                    memory: String::new(),
                    model: "sonnet".into(),
                    project_path: String::new(),
                    enabled: true,
                },
                look: BruhLook::default(),
                access: BruhAccess::default(),
                effort: Some("high".into()),
            }
        );
    }
}
