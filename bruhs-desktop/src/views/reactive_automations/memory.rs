//! An agent's own memory and the follow-ups it scheduled, editable by the owner.
use super::updates::{local_time, selectable_text};
use super::*;
use gpui_component::zoom::px;

pub(super) struct MemoryEdit {
    pub(super) agent_id: String,
    pub(super) key: String,
    kind: MemoryKind,
    text: Entity<TextareaState>,
}

/// "From run 1a2b3c4d on Nov 14, 3:05 PM" or "Edited by you on …".
pub(super) fn provenance(entry: &MemoryEntry) -> String {
    let at = local_time(entry.updated_at);
    match &entry.run_id {
        Some(run) => format!("From run {} on {at}", runs::short_id(run)),
        None => format!("Edited by you on {at}"),
    }
}

/// What "Copy memory" puts on the clipboard.
pub(super) fn memory_json(entries: &[&MemoryEntry]) -> String {
    serde_json::to_string_pretty(entries).unwrap_or_else(|error| format!("{error}"))
}

fn follow_up_status(status: FollowUpStatus) -> &'static str {
    match status {
        FollowUpStatus::Pending => "Pending",
        FollowUpStatus::Fired => "Ran",
        FollowUpStatus::Cancelled => "Cancelled",
    }
}

impl ReactiveAutomations {
    pub(super) fn render_agent_records(&self, agent_id: &str, cx: &Context<'_, Self>) -> Div {
        let Some(snapshot) = &self.snapshot else {
            return div();
        };
        let entries: Vec<&MemoryEntry> = snapshot
            .memories
            .iter()
            .filter(|entry| entry.agent_id == agent_id)
            .collect();
        let scope = Scope::Agent(agent_id.to_string());
        let mut memory = div()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .pt_2()
            .border_t_1()
            .border_color(theme::longan_700().opacity(0.5))
            .child(heading("Agent memory"))
            .child(muted(
                "Written by the agent. Edit or forget anything; changes reach its next run.",
            ))
            .children(self.scoped_error(&scope));
        if entries.is_empty() {
            memory = memory.child(muted("Nothing saved yet."));
        }
        for entry in &entries {
            memory = memory.child(self.render_memory_entry(entry, cx));
        }
        let copy = memory_json(&entries);
        memory = memory.child(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .child(
                    control(
                        SharedString::from(format!("reactive-tidy-memory-{agent_id}")),
                        "Tidy memory",
                        self.busy || entries.is_empty(),
                    )
                    .on_activate(cx, {
                        let agent_id = agent_id.to_string();
                        move |this, _, cx| {
                            this.send_in(
                                ReactiveCommand::ConsolidateMemory {
                                    agent_id: agent_id.clone(),
                                },
                                scope.clone(),
                                cx,
                            )
                        }
                    }),
                )
                .child(
                    control(
                        SharedString::from(format!("reactive-copy-memory-{agent_id}")),
                        "Copy memory",
                        entries.is_empty(),
                    )
                    .on_activate(cx, move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                    }),
                ),
        );
        div()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_3()
            .child(memory)
            .child(self.render_follow_ups(snapshot, agent_id, cx))
    }

    fn render_memory_entry(&self, entry: &MemoryEntry, cx: &Context<'_, Self>) -> Div {
        let editing = self
            .memory_edit
            .as_ref()
            .filter(|edit| edit.agent_id == entry.agent_id && edit.key == entry.key);
        let scope = Scope::Agent(entry.agent_id.clone());
        let id = format!("{}-{}", entry.agent_id, entry.key);
        let mut row = div().min_w_0().flex().flex_col().gap_1().child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(updates::badge_info(entry.kind.label()))
                .child(
                    div()
                        .min_w_0()
                        .text_size(px(13.0))
                        .text_color(theme::rambutan_100())
                        .child(entry.key.clone()),
                ),
        );
        if let Some(edit) = editing {
            return row
                .child(field("Memory text", &edit.text, true, self.busy))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            control(
                                SharedString::from(format!("reactive-save-memory-{id}")),
                                "Save",
                                self.busy,
                            )
                            .on_activate(cx, move |this, _, cx| {
                                let Some(edit) = &this.memory_edit else {
                                    return;
                                };
                                let command = ReactiveCommand::EditMemory {
                                    agent_id: edit.agent_id.clone(),
                                    key: edit.key.clone(),
                                    kind: edit.kind,
                                    text: value(&edit.text, cx),
                                };
                                this.send_in(command, scope.clone(), cx);
                            }),
                        )
                        .child(
                            control(
                                SharedString::from(format!("reactive-cancel-memory-{id}")),
                                "Cancel",
                                self.busy,
                            )
                            .on_activate(cx, |this, _, cx| {
                                this.memory_edit = None;
                                cx.notify();
                            }),
                        ),
                );
        }
        let edit = (
            entry.agent_id.clone(),
            entry.key.clone(),
            entry.kind,
            entry.text.clone(),
        );
        row = row
            .child(selectable_text(
                SharedString::from(format!("reactive-memory-text-{id}")),
                &entry.text,
            ))
            .child(muted(provenance(entry)))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        control(
                            SharedString::from(format!("reactive-edit-memory-{id}")),
                            "Edit",
                            self.busy,
                        )
                        .on_activate(cx, move |this, window, cx| {
                            let (agent_id, key, kind, text) = edit.clone();
                            this.memory_edit = Some(MemoryEdit {
                                agent_id,
                                key,
                                kind,
                                text: edit_area(&text, window, cx),
                            });
                            cx.notify();
                        }),
                    )
                    .child(self.command_in(
                        format!("reactive-forget-memory-{id}"),
                        "Forget",
                        ReactiveCommand::ForgetMemory {
                            agent_id: entry.agent_id.clone(),
                            key: entry.key.clone(),
                        },
                        scope,
                        cx,
                    )),
            );
        row
    }

    fn render_follow_ups(
        &self,
        snapshot: &ReactiveSnapshot,
        agent_id: &str,
        cx: &Context<'_, Self>,
    ) -> Div {
        let (pending, earlier): (Vec<&FollowUp>, Vec<&FollowUp>) = snapshot
            .follow_ups
            .iter()
            .filter(|follow_up| follow_up.agent_id == agent_id)
            .partition(|follow_up| follow_up.status == FollowUpStatus::Pending);
        let scope = Scope::Agent(agent_id.to_string());
        let mut section = div()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(heading("Follow-ups"));
        if pending.is_empty() {
            section = section.child(muted("No follow-ups scheduled."));
        }
        for follow_up in pending {
            section = section.child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(theme::rambutan_100())
                            .child(format!(
                                "{}: due {}",
                                follow_up.key,
                                local_time(follow_up.due_at)
                            )),
                    )
                    .child(muted(follow_up.note.clone()))
                    .child(div().flex().child(self.command_in(
                        format!("reactive-cancel-follow-up-{}", follow_up.id),
                        "Cancel follow-up",
                        ReactiveCommand::CancelFollowUp {
                            id: follow_up.id.clone(),
                        },
                        scope.clone(),
                        cx,
                    ))),
            );
        }
        if earlier.is_empty() {
            return section;
        }
        let key = format!("follow-ups:{agent_id}");
        let expanded = self.expanded.contains(&key);
        section = section.child(
            div().flex().child(
                control(
                    SharedString::from(format!("reactive-earlier-follow-ups-{agent_id}")),
                    "Earlier follow-ups",
                    false,
                )
                .aria_expanded(expanded)
                .on_activate(cx, move |this, _, cx| {
                    if !this.expanded.remove(&key) {
                        this.expanded.insert(key.clone());
                    }
                    cx.notify();
                }),
            ),
        );
        if expanded {
            for follow_up in earlier {
                section = section.child(
                    div()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(muted(format!(
                            "{}: due {}",
                            follow_up.key,
                            local_time(follow_up.due_at)
                        )))
                        .child(muted(follow_up.note.clone()))
                        .child(muted(follow_up_status(follow_up.status))),
                );
            }
        }
        section
    }
}

#[cfg(test)]
mod tests {
    use super::{memory_json, provenance, MemoryEntry, MemoryKind};
    use bruhs_models::EpochMs;

    #[test]
    fn memory_provenance_names_the_run_or_the_owner_and_copies_as_json() {
        let mut entry = MemoryEntry {
            agent_id: "agent".into(),
            key: "invoice-42".into(),
            kind: MemoryKind::Progress,
            text: "Paid half.".into(),
            updated_at: EpochMs::UNIX_EPOCH,
            run_id: Some("0123456789abcdef".into()),
        };
        assert!(provenance(&entry).starts_with("From run 01234567 on "));
        entry.run_id = None;
        assert!(provenance(&entry).starts_with("Edited by you on "));
        let copied: serde_json::Value = serde_json::from_str(&memory_json(&[&entry])).unwrap();
        assert_eq!(copied[0]["key"], "invoice-42");
        assert_eq!(copied[0]["kind"], "progress");
    }
}
