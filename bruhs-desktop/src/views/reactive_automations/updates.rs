//! What agents told the owner, newest first. Questions wait for a reply.
use super::*;
use bruhs_models::EpochMs;
use bruhs_ui::text::TextView;
use bruhs_ui::zoom::px;

/// The newest unread item not seen before. The first snapshot only records a
/// baseline, so updates already waiting at launch never raise a notice.
pub(super) fn new_arrival<'a>(
    seen: &mut Option<HashSet<String>>,
    outbox: &'a [OutboxItem],
) -> Option<&'a OutboxItem> {
    let unread = outbox.iter().filter(|item| item.is_unread());
    let Some(seen) = seen else {
        *seen = Some(unread.map(|item| item.id.clone()).collect());
        return None;
    };
    let mut newest = None;
    for item in unread {
        if seen.insert(item.id.clone()) && newest.is_none() {
            newest = Some(item);
        }
    }
    newest
}

/// Local wall-clock time for an outbox, memory or follow-up record.
pub(super) fn local_time(at: EpochMs) -> String {
    chrono::DateTime::from_timestamp_millis(at.as_millis()).map_or_else(
        || at.to_string(),
        |time| {
            let date = time.with_timezone(&chrono::Local);
            format!(
                "{}, {}",
                date.format("%b %-d, %Y"),
                date.format(crate::clock_format::pattern())
            )
        },
    )
}

/// Agent text as Markdown that renders literally: every punctuation mark is
/// escaped, each newline is a hard break, and leading spaces are kept.
pub(super) fn literal_markdown(text: &str) -> String {
    text.lines()
        .map(|line| {
            let mut out = String::with_capacity(line.len());
            let mut leading = true;
            for c in line.chars() {
                if leading && c == ' ' {
                    out.push('\u{a0}');
                    continue;
                }
                leading = false;
                if c.is_ascii_punctuation() {
                    out.push('\\');
                }
                out.push(c);
            }
            out
        })
        .collect::<Vec<_>>()
        .join("\\\n")
}

/// Selectable, wrapped, line-preserving text for agent-written messages.
pub(super) fn selectable_text(id: impl Into<ElementId>, text: &str) -> Div {
    div()
        .min_w_0()
        .w_full()
        .text_size(px(13.0))
        .line_height(relative(1.5))
        .text_color(theme::rambutan_200())
        .child(TextView::markdown(id, literal_markdown(text)).selectable(true))
}

/// An information-tone badge, shared with run cards.
pub(super) fn badge_info(text: &'static str) -> Div {
    bruhs_ui::badge::badge(text, bruhs_ui::badge::BadgeTone::Info)
}

pub(super) fn badge_plain(text: &'static str) -> Div {
    bruhs_ui::badge::badge(text, bruhs_ui::badge::BadgeTone::Neutral)
}

/// The unread count on the Agent workflows navigation entry.
pub(crate) fn unread_badge(count: usize) -> Div {
    bruhs_ui::badge::count_badge(count)
}

/// Accessible name for a navigation entry that carries the unread badge.
pub(crate) fn unread_label(label: &str, count: usize) -> String {
    match count {
        0 => label.to_string(),
        1 => format!("{label}, 1 unread update"),
        count => format!("{label}, {count} unread updates"),
    }
}

impl ReactiveAutomations {
    pub(super) fn render_updates(&self, cx: &Context<'_, Self>) -> Stateful<Div> {
        let outbox = self
            .snapshot
            .as_ref()
            .map_or(&[][..], |snapshot| snapshot.outbox.as_slice());
        let unread = outbox.iter().filter(|item| item.is_unread()).count();
        let mut section = div()
            .id("reactive-updates")
            .debug_selector(|| "reactive-updates".into())
            .min_w_0()
            .w_full()
            .flex()
            .flex_col()
            .gap_3()
            .pb_4()
            .border_b_1()
            .border_color(theme::longan_700().opacity(0.6))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(heading("Updates"))
                    .child(
                        control(
                            "reactive-mark-all-read",
                            "Mark all read",
                            self.busy || unread == 0,
                        )
                        .on_activate(cx, move |this, _, cx| {
                            if unread > 0 {
                                this.send_in(
                                    ReactiveCommand::MarkOutboxRead { ids: Vec::new() },
                                    Scope::Updates,
                                    cx,
                                );
                            }
                        }),
                    ),
            )
            .child(muted(
                "What your agents told you. Questions wait for your reply.",
            ))
            .children(self.scoped_error(&Scope::Updates));
        if outbox.is_empty() {
            return section.child(muted(
                "No updates yet. Agents stay quiet until something needs you.",
            ));
        }
        for item in outbox {
            section = section.child(self.render_update(item, cx));
        }
        section
    }

    fn render_update(&self, item: &OutboxItem, cx: &Context<'_, Self>) -> Stateful<Div> {
        let unread = item.is_unread();
        let open_question = item.is_open_question();
        let kind = match item.kind {
            OutboxKind::Ask => "Question",
            OutboxKind::Notify => "Update",
        };
        let agent = self.agent_name(&item.agent_id).to_string();
        let resolved = item.resolved_at.map(|_| {
            if item.answer.is_some() {
                "Answered"
            } else {
                "Dismissed"
            }
        });
        let mut row = card()
            .id(SharedString::from(format!("reactive-update-{}", item.id)))
            .debug_selector({
                let id = item.id.clone();
                move || format!("reactive-update-{id}")
            })
            .role(Role::Group)
            .aria_label(format!(
                "{kind} from {agent}{}",
                if unread { ", unread" } else { "" }
            ))
            .when(unread, |row| {
                row.bg(theme::blueberry_500().opacity(0.08))
                    .border_color(theme::blueberry_400().opacity(0.5))
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    // Web order: kind and resolution chips, then the unread label.
                    .child(badge_plain(kind))
                    .children(resolved.map(badge_plain))
                    .when(unread, |row| {
                        row.child(
                            div()
                                .text_size(px(12.0))
                                .text_color(theme::blueberry_200())
                                .child("Unread"),
                        )
                    })
                    .child(muted(agent))
                    .child(muted(local_time(item.created_at))),
            )
            .child(heading(item.title.clone()))
            .child(selectable_text(
                SharedString::from(format!("reactive-update-message-{}", item.id)),
                &item.message,
            ));
        if let Some(answer) = &item.answer {
            row = row.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_2()
                    .rounded(px(6.0))
                    .bg(theme::longan_900())
                    .border_1()
                    .border_color(theme::longan_700().opacity(0.5))
                    .child(muted("Your reply"))
                    .child(selectable_text(
                        SharedString::from(format!("reactive-update-answer-{}", item.id)),
                        answer,
                    )),
            );
        }
        if let SystemDelivery::Failed { error } = &item.system {
            row = row.child(
                div()
                    .text_size(px(12.0))
                    .text_color(theme::lychee_300())
                    .child(format!("System notification failed: {error}")),
            );
        }
        let replying = self
            .reply
            .as_ref()
            .filter(|(id, _)| *id == item.id)
            .map(|(_, input)| input);
        let mut actions = div().flex().flex_wrap().gap_2();
        if item.resolved_at.is_none() {
            let id = item.id.clone();
            actions = actions.child(
                control_as(
                    SharedString::from(format!("reactive-reply-{}", item.id)),
                    "Reply",
                    self.busy,
                    open_question,
                )
                .aria_expanded(replying.is_some())
                .on_activate(cx, move |this, window, cx| {
                    if this.reply.as_ref().is_some_and(|(open, _)| *open == id) {
                        this.reply = None;
                    } else {
                        let input = edit_area("", window, cx);
                        this.reply_watch =
                            Some(cx.subscribe(&input, |_, _, event: &InputEvent, cx| {
                                if matches!(event, InputEvent::Change) {
                                    cx.notify();
                                }
                            }));
                        this.reply = Some((id.clone(), input));
                    }
                    cx.notify();
                }),
            );
        }
        if unread {
            actions = actions.child(self.ghost_command_in(
                format!("reactive-read-{}", item.id),
                "Mark read",
                ReactiveCommand::MarkOutboxRead {
                    ids: vec![item.id.clone()],
                },
                Scope::Updates,
                cx,
            ));
        }
        if item.resolved_at.is_none() {
            actions = actions.child(self.ghost_command_in(
                format!("reactive-dismiss-{}", item.id),
                "Dismiss",
                ReactiveCommand::DismissOutbox {
                    id: item.id.clone(),
                },
                Scope::Updates,
                cx,
            ));
        }
        row = row.child(actions);
        if let Some(input) = replying {
            let id = item.id.clone();
            let empty = value(input, cx).trim().is_empty();
            row = row.child(field("Reply", input, true, self.busy)).child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        control_as(
                            "reactive-send-reply",
                            "Send reply",
                            self.busy || empty,
                            true,
                        )
                        .on_activate(cx, move |this, _, cx| {
                            let Some((_, input)) = &this.reply else {
                                return;
                            };
                            let message = value(input, cx);
                            // The button only looks disabled; an empty reply never leaves.
                            if message.trim().is_empty() {
                                return;
                            }
                            this.send_in(
                                ReactiveCommand::AnswerOutbox {
                                    id: id.clone(),
                                    message,
                                },
                                Scope::Updates,
                                cx,
                            );
                        }),
                    )
                    .child(
                        control("reactive-cancel-reply", "Cancel", self.busy).on_activate(
                            cx,
                            |this, _, cx| {
                                this.reply = None;
                                cx.notify();
                            },
                        ),
                    ),
            );
        }
        row
    }
}

#[cfg(test)]
mod tests {
    use super::{
        literal_markdown, new_arrival, unread_label, Editor, OutboxItem, OutboxKind,
        ReactiveAutomations,
    };
    use crate::theme;
    use bruhs_models::{
        reactive::{
            AutomationSetup, BusyPolicy, EventSource, FollowUp, FollowUpStatus, MemoryEntry,
            MemoryKind, ReactiveAgent, ReactiveSnapshot, SourceKind, Subscription, SystemDelivery,
        },
        EpochMs,
    };
    use gpui::AppContext as _;

    fn item(id: &str, read: bool) -> OutboxItem {
        OutboxItem {
            id: id.into(),
            agent_id: "agent".into(),
            run_id: "run".into(),
            kind: OutboxKind::Notify,
            title: id.into(),
            message: String::new(),
            created_at: EpochMs::UNIX_EPOCH,
            read_at: read.then_some(EpochMs::UNIX_EPOCH),
            resolved_at: None,
            answer: None,
            system: SystemDelivery::NotRequested,
            approval: None,
        }
    }

    #[test]
    fn arrivals_skip_the_launch_baseline_and_report_only_new_unread_items() {
        let mut seen = None;
        let first = [item("waiting", false), item("old", true)];
        assert!(new_arrival(&mut seen, &first).is_none(), "baseline");
        assert!(new_arrival(&mut seen, &first).is_none(), "nothing new");
        let next = [
            item("newest", false),
            item("newer", false),
            item("waiting", false),
            item("read", true),
        ];
        assert_eq!(
            new_arrival(&mut seen, &next).map(|i| i.id.as_str()),
            Some("newest")
        );
        assert!(new_arrival(&mut seen, &next).is_none(), "each item once");
    }

    #[test]
    fn agent_text_renders_literally_and_badge_labels_count_updates() {
        assert_eq!(
            literal_markdown("# Bill *due*\n  - pay $40"),
            "\\# Bill \\*due\\*\\\n\u{a0}\u{a0}\\- pay \\$40"
        );
        assert_eq!(unread_label("Agent workflows", 0), "Agent workflows");
        assert_eq!(
            unread_label("Agent workflows", 1),
            "Agent workflows, 1 unread update"
        );
        assert_eq!(
            unread_label("Agent workflows", 3),
            "Agent workflows, 3 unread updates"
        );
    }

    fn seeded() -> ReactiveSnapshot {
        let agent = ReactiveAgent {
            id: "agent-1".into(),
            name: "Bill assistant".into(),
            instructions: "Read bills.".into(),
            memory: String::new(),
            model: "claude-haiku-4-5".into(),
            project_path: "/tmp".into(),
            enabled: true,
        };
        let source = EventSource {
            id: "source-1".into(),
            name: "Mornings".into(),
            enabled: true,
            kind: SourceKind::Calendar {
                times: vec!["08:00".into()],
                weekdays: vec![1, 2, 3, 4, 5],
                time_zone: "UTC".into(),
            },
        };
        let subscription = Subscription {
            id: "sub-1".into(),
            source_id: source.id.clone(),
            agent_id: agent.id.clone(),
            instructions: "Check bills.".into(),
            enabled: true,
            auto_run: true,
            debounce_ms: 0,
            throttle_ms: 0,
            webhook_filters: Vec::new(),
            webhook_prompt: None,
            webhook_security: Default::default(),
            webhook_dedupe: Default::default(),
            task_title: None,
            task_conversation: Default::default(),
            task_checkout: Default::default(),
            on_busy: BusyPolicy::Restart,
        };
        let mut question = item("question-1", false);
        question.agent_id = agent.id.clone();
        question.kind = OutboxKind::Ask;
        question.title = "Pay the water bill?".into();
        question.message = "It is due Friday.\nAmount: $40".into();
        question.system = SystemDelivery::Failed {
            error: "Notification permission denied.".into(),
        };
        let mut done = item("update-1", true);
        done.resolved_at = Some(EpochMs::UNIX_EPOCH);
        ReactiveSnapshot {
            memories: vec![MemoryEntry {
                agent_id: agent.id.clone(),
                key: "payee".into(),
                kind: MemoryKind::Note,
                text: "Water bills go to City Utilities.".into(),
                updated_at: EpochMs::UNIX_EPOCH,
                run_id: Some("run-12345678".into()),
            }],
            follow_ups: vec![FollowUp {
                id: "follow-1".into(),
                agent_id: agent.id.clone(),
                key: "water-bill".into(),
                note: "Check it was paid.".into(),
                due_at: EpochMs::UNIX_EPOCH,
                status: FollowUpStatus::Pending,
                created_at: EpochMs::UNIX_EPOCH,
                run_id: None,
                auto_run: false,
                fired_run_id: None,
            }],
            agents: vec![agent],
            sources: vec![source],
            subscriptions: vec![subscription],
            outbox: vec![question, done],
            ..ReactiveSnapshot::default()
        }
    }

    #[gpui::test]
    fn updates_render_at_the_top_with_reply_memory_and_follow_ups(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            bruhs_ui::init(cx);
            theme::apply_bruhs_theme(cx);
        });
        let holder = std::rc::Rc::new(std::cell::RefCell::new(None));
        let capture = holder.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let panel = cx.new(ReactiveAutomations::new);
            *capture.borrow_mut() = Some(panel.clone());
            bruhs_ui::Root::new(panel, window, cx)
        });
        cx.simulate_resize(gpui::size(gpui::px(900.0), gpui::px(2400.0)));
        let panel = holder.borrow().clone().unwrap();
        panel.update_in(cx, |panel, _, cx| {
            panel.loaded = true;
            panel.set_snapshot(seeded(), cx);
            assert_eq!(panel.unread_count(), 1);
            cx.notify();
        });
        cx.run_until_parked();
        let updates = cx
            .debug_bounds("reactive-updates")
            .expect("updates section");
        let automation = cx
            .debug_bounds("reactive-New automation")
            .expect("automation header");
        assert!(
            updates.origin.y < automation.origin.y,
            "updates lead the panel"
        );
        assert!(cx.debug_bounds("reactive-update-question-1").is_some());
        assert!(cx.debug_bounds("reactive-update-update-1").is_some());
        for selector in [
            "reactive-Mark all read",
            "reactive-Mark read",
            "reactive-Dismiss",
            "reactive-Tidy memory",
            "reactive-Copy memory",
            "reactive-Edit",
            "reactive-Forget",
            "reactive-Cancel follow-up",
            "reactive-Save notification settings",
        ] {
            assert!(cx.debug_bounds(selector).is_some(), "{selector}");
        }
        let reply = cx.debug_bounds("reactive-Reply").expect("reply");
        cx.simulate_click(reply.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("reactive-Send reply").is_some());
        panel.update_in(cx, |panel, _, _| {
            assert_eq!(
                panel.reply.as_ref().map(|(id, _)| id.as_str()),
                Some("question-1")
            );
            assert!(!panel.busy, "opening a reply sends nothing");
        });
        // An empty reply is never sent, whatever the click.
        let send = cx.debug_bounds("reactive-Send reply").expect("send reply");
        cx.simulate_click(send.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        panel.update_in(cx, |panel, _, _| {
            assert!(!panel.busy, "an empty reply sends nothing");
            assert!(panel.reply.is_some(), "the empty reply stays open");
        });
        // Notification switches show their state, not just their role.
        assert!(cx
            .debug_bounds("reactive-System notification for updates-checked")
            .is_none());
        let switch = cx
            .debug_bounds("reactive-System notification for updates")
            .expect("updates switch");
        cx.simulate_click(switch.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(cx
            .debug_bounds("reactive-System notification for updates-checked")
            .is_some());
        panel.update_in(cx, |panel, _, _| {
            let snapshot = panel.snapshot.as_ref().unwrap();
            let setup =
                AutomationSetup::from_subscription(snapshot, &snapshot.subscriptions[0]).unwrap();
            assert_eq!(setup.subscription.on_busy, BusyPolicy::Restart);
            assert!(!matches!(panel.editor, Some(Editor::Automation(_))));
        });
    }
}
