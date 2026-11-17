//! An agent's own memory and the follow-ups it scheduled for itself.
use super::calendar::local_time;
use super::runs::short_id;
use crate::ui::{Button, ButtonSize, ButtonVariant, Textarea};
use bruhs_models::reactive::*;
use leptos::prelude::*;

pub(super) fn provenance(entry: &MemoryEntry) -> String {
    let at = local_time(entry.updated_at);
    match entry.run_id.as_deref() {
        Some(run) => format!("From run {} on {at}", short_id(run)),
        None => format!("Edited by you on {at}"),
    }
}

/// The agent's entries as pretty JSON, for "Copy memory".
pub(super) fn memory_json(entries: &[MemoryEntry]) -> String {
    serde_json::to_string_pretty(entries)
        .unwrap_or_else(|error| format!("Cannot copy memory: {error}"))
}

pub(super) fn follow_up_heading(follow_up: &FollowUp) -> String {
    format!("{}: due {}", follow_up.key, local_time(follow_up.due_at))
}

fn follow_up_status(status: FollowUpStatus) -> &'static str {
    match status {
        FollowUpStatus::Pending => "Pending",
        FollowUpStatus::Fired => "Ran",
        FollowUpStatus::Cancelled => "Cancelled",
    }
}

/// The memory entry being edited, kept above the cards: they re-render
/// whenever the snapshot changes, and an edit in progress must survive that.
#[derive(Clone, Copy)]
pub(super) struct MemoryEditor {
    /// `(agent_id, key)` of the entry being edited.
    editing: RwSignal<Option<(String, String)>>,
    draft: RwSignal<String>,
}

impl MemoryEditor {
    pub(super) fn new() -> Self {
        Self {
            editing: RwSignal::new(None),
            draft: RwSignal::new(String::new()),
        }
    }
}

#[component]
pub(crate) fn AgentMemoryPanel(
    agent_id: String,
    snapshot: RwSignal<Option<ReactiveSnapshot>>,
    send: Callback<ReactiveCommand>,
) -> impl IntoView {
    let agent = StoredValue::new(agent_id);
    let entries = Memo::new(move |_| {
        snapshot.with(|s| {
            s.as_ref()
                .map(|s| {
                    s.memories
                        .iter()
                        .filter(|entry| agent.with_value(|id| entry.agent_id == *id))
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
    });
    let follow_ups = Memo::new(move |_| {
        snapshot.with(|s| {
            s.as_ref()
                .map(|s| {
                    s.follow_ups
                        .iter()
                        .filter(|follow_up| agent.with_value(|id| follow_up.agent_id == *id))
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
    });
    let MemoryEditor { editing, draft } =
        use_context::<MemoryEditor>().unwrap_or_else(MemoryEditor::new);
    view! {
        <div class="mt-3 min-w-0 space-y-4 border-t border-longan-700/50 pt-3">
            <section class="min-w-0 space-y-2" aria-label="Agent memory">
                <div class="flex flex-wrap items-start justify-between gap-2">
                    <div class="min-w-0">
                        <h4 class="text-label text-rambutan-100">"Agent memory"</h4>
                        <p class="mt-1 text-label-sm text-rambutan-500">"Written by the agent. Edit or forget anything; changes reach its next run."</p>
                    </div>
                    <div class="flex flex-wrap gap-2">
                        <Button variant=ButtonVariant::Outline size=ButtonSize::Small disabled=Signal::derive(move || entries.with(Vec::is_empty)) on_click=Callback::new(move |_| send.run(ReactiveCommand::ConsolidateMemory { agent_id: agent.get_value() }))>"Tidy memory"</Button>
                        <Button variant=ButtonVariant::Ghost size=ButtonSize::Small disabled=Signal::derive(move || entries.with(Vec::is_empty)) on_click=Callback::new(move |_| crate::ui::copy_to_clipboard(&entries.with_untracked(|entries| memory_json(entries))))>"Copy memory"</Button>
                    </div>
                </div>
                <Show when=move || entries.with(Vec::is_empty)>
                    <p class="text-body-sm text-rambutan-500">"Nothing saved yet."</p>
                </Show>
                <ul class="m-0 min-w-0 list-none space-y-2 p-0">
                    {move || entries.get().into_iter().map(|entry| {
                        let key = entry.key.clone();
                        let is_editing = {
                            let target = (agent.get_value(), key.clone());
                            Signal::derive(move || editing.with(|editing| editing.as_ref() == Some(&target)))
                        };
                        let edit_key = key.clone();
                        let edit_text = entry.text.clone();
                        let save_key = key.clone();
                        let kind = entry.kind;
                        let forget_key = key.clone();
                        let label = format!("Memory {key}");
                        view! {
                            <li class="min-w-0 rounded-grape bg-longan-900 p-2 inset-ring inset-ring-longan-700/50">
                                <div class="flex flex-wrap items-center gap-2">
                                    <span class="rounded-seed bg-longan-800 px-2 py-0.5 text-label-sm text-rambutan-200">{entry.kind.label()}</span>
                                    <span class="min-w-0 break-all text-label-sm text-rambutan-300">{entry.key.clone()}</span>
                                </div>
                                <Show
                                    when=move || is_editing.get()
                                    fallback={
                                        let text = entry.text.clone();
                                        move || view! { <p class="mt-1 whitespace-pre-wrap break-words select-text text-body-sm text-rambutan-200">{text.clone()}</p> }
                                    }
                                >
                                    <div class="mt-2"><Textarea aria_label=label.clone() value=Signal::derive(move || draft.get()) on_input=Callback::new(move |text| draft.set(text)) rows=3 autofocus=true /></div>
                                </Show>
                                <p class="mt-1 text-label-sm text-rambutan-500">{provenance(&entry)}</p>
                                <div class="mt-2 flex flex-wrap gap-2">
                                    <Show
                                        when=move || is_editing.get()
                                        fallback={
                                            move || {
                                                let edit_key = edit_key.clone();
                                                let edit_text = edit_text.clone();
                                                view! { <Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| { draft.set(edit_text.clone()); editing.set(Some((agent.get_value(), edit_key.clone()))); })>"Edit"</Button> }
                                            }
                                        }
                                    >
                                        <Button size=ButtonSize::Small disabled=Signal::derive(move || draft.with(|text| text.trim().is_empty())) on_click=Callback::new({
                                            let save_key = save_key.clone();
                                            move |_| {
                                                send.run(ReactiveCommand::EditMemory { agent_id: agent.get_value(), key: save_key.clone(), kind, text: draft.get_untracked() });
                                                editing.set(None);
                                            }
                                        })>"Save"</Button>
                                        <Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| editing.set(None))>"Cancel"</Button>
                                    </Show>
                                    <Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| send.run(ReactiveCommand::ForgetMemory { agent_id: agent.get_value(), key: forget_key.clone() }))>"Forget"</Button>
                                </div>
                            </li>
                        }
                    }).collect_view()}
                </ul>
            </section>
            <section class="min-w-0 space-y-2" aria-label="Follow-ups">
                <h4 class="text-label text-rambutan-100">"Follow-ups"</h4>
                <Show when=move || follow_ups.with(|items| !items.iter().any(|item| item.status == FollowUpStatus::Pending))>
                    <p class="text-body-sm text-rambutan-500">"No follow-ups scheduled."</p>
                </Show>
                <ul class="m-0 min-w-0 list-none space-y-2 p-0">
                    {move || follow_ups.get().into_iter().filter(|item| item.status == FollowUpStatus::Pending).map(|item| {
                        let id = item.id.clone();
                        view! {
                            <li class="min-w-0 rounded-grape bg-longan-900 p-2 inset-ring inset-ring-longan-700/50">
                                <p class="break-words text-label-sm text-rambutan-300">{follow_up_heading(&item)}</p>
                                <p class="mt-1 whitespace-pre-wrap break-words text-body-sm text-rambutan-200">{item.note.clone()}</p>
                                <div class="mt-2"><Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| send.run(ReactiveCommand::CancelFollowUp { id: id.clone() }))>"Cancel follow-up"</Button></div>
                            </li>
                        }
                    }).collect_view()}
                </ul>
                <Show when=move || follow_ups.with(|items| items.iter().any(|item| item.status != FollowUpStatus::Pending))>
                    <details>
                        <summary class="cursor-pointer text-label-sm text-rambutan-400">"Earlier follow-ups"</summary>
                        <ul class="m-0 mt-2 min-w-0 list-none space-y-2 p-0">
                            {move || follow_ups.get().into_iter().filter(|item| item.status != FollowUpStatus::Pending).map(|item| view! {
                                <li class="min-w-0">
                                    <p class="break-words text-label-sm text-rambutan-400">{format!("{}: {}", follow_up_status(item.status), follow_up_heading(&item))}</p>
                                    <p class="mt-1 whitespace-pre-wrap break-words text-body-sm text-rambutan-300">{item.note}</p>
                                </li>
                            }).collect_view()}
                        </ul>
                    </details>
                </Show>
            </section>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::EpochMs;

    fn entry(run_id: Option<&str>) -> MemoryEntry {
        MemoryEntry {
            agent_id: "a".into(),
            key: "invoice-42".into(),
            kind: MemoryKind::Progress,
            text: "Waiting on the payee.".into(),
            updated_at: EpochMs::UNIX_EPOCH,
            run_id: run_id.map(str::to_owned),
        }
    }

    #[test]
    fn provenance_names_the_run_or_the_owner() {
        assert!(provenance(&entry(Some("0123456789abcdef"))).starts_with("From run 01234567 on "));
        assert!(provenance(&entry(None)).starts_with("Edited by you on "));
    }

    #[test]
    fn copied_memory_is_pretty_json_of_the_entries() {
        let json = memory_json(&[entry(None)]);
        assert!(json.contains("\n"));
        let parsed: Vec<MemoryEntry> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, vec![entry(None)]);
    }

    #[test]
    fn follow_up_heading_shows_key_and_due_time() {
        let follow_up = FollowUp {
            id: "f".into(),
            agent_id: "a".into(),
            key: "check-payment".into(),
            note: String::new(),
            due_at: EpochMs::UNIX_EPOCH,
            status: FollowUpStatus::Pending,
            created_at: EpochMs::UNIX_EPOCH,
            run_id: None,
            auto_run: false,
            fired_run_id: None,
        };
        assert!(follow_up_heading(&follow_up).starts_with("check-payment: due "));
    }
}
