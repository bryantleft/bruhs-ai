use crate::task_style::{PriorityStyle, WorkItemStatusStyle};
use leptos::prelude::*;

use crate::state::{
    Priority, ProjectsSignal, RecurrenceRule, SelectedWorkItem, SetSelectedWorkItem,
    SetWorkItemsSignal, WorkItem, WorkItemStatus, WorkItemsSignal,
};
use crate::storage;
use crate::ui::{
    Button, ButtonSize, ButtonVariant, DatePicker, Dialog, Dropdown, DropdownOption, Input,
    Textarea,
};

/// Compute the entire optimistic transition before enqueueing either record.
/// Reading current state here also makes repeated Done clicks idempotent.
fn change_status(
    items: &mut Vec<WorkItem>,
    id: &str,
    status: WorkItemStatus,
    now: i64,
) -> Vec<(Option<WorkItem>, WorkItem)> {
    let Some(item) = items.iter_mut().find(|item| item.id == id) else {
        return Vec::new();
    };
    if item.status == status {
        return Vec::new();
    }
    let original = item.clone();
    let next_due = (status == WorkItemStatus::Done)
        .then_some(
            original
                .planning
                .recurrence
                .zip(original.due_date.as_deref()),
        )
        .flatten()
        .and_then(|(rule, due)| bruhs_models::next_recurrence_date(due, rule));
    let next = next_due.map(|due| {
        let mut next = WorkItem::new_at(original.title.clone(), now);
        next.project_id.clone_from(&original.project_id);
        next.description.clone_from(&original.description);
        next.status = WorkItemStatus::Todo;
        next.priority = original.priority;
        next.due_date = Some(due);
        next.planning.clone_from(&original.planning);
        next.planning.time_block_event_ids.clear();
        next
    });
    item.status = status;
    item.updated_at = now;
    let mut changes = vec![(Some(original), item.clone())];
    if let Some(next) = next {
        items.push(next.clone());
        changes.push((None, next));
    }
    changes
}

#[cfg(test)]
mod status_tests {
    use super::*;

    #[test]
    fn recurring_done_returns_one_logical_pair_and_repeated_done_is_noop() {
        let mut original = WorkItem::new_at("Recurring", 10);
        original.status = WorkItemStatus::Todo;
        original.due_date = Some("2026-10-01".into());
        original.priority = Priority::High;
        original.description = Some("Keep context".into());
        original.planning.recurrence = Some(RecurrenceRule::Daily);
        original
            .planning
            .time_block_event_ids
            .push("old-block".into());
        let mut items = vec![original.clone()];
        let changes = change_status(&mut items, &original.id, WorkItemStatus::Done, 20);
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].0.as_ref(), Some(&original));
        assert!(changes[1].0.is_none());
        assert_eq!(items[0].status, WorkItemStatus::Done);
        assert_eq!(items[0].updated_at, 20);
        assert_eq!(items[1].status, WorkItemStatus::Todo);
        assert_eq!(items[1].due_date.as_deref(), Some("2026-10-02"));
        assert_eq!(items[1].priority, original.priority);
        assert_eq!(items[1].description, original.description);
        assert!(items[1].planning.time_block_event_ids.is_empty());
        assert_eq!(items[1].created_at, 20);
        assert!(change_status(&mut items, &original.id, WorkItemStatus::Done, 30).is_empty());
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].updated_at, 20);
    }

    #[test]
    fn ordinary_status_changes_have_one_record_and_missing_ids_do_nothing() {
        let original = WorkItem::new_at("One task", 10);
        let mut items = vec![original.clone()];
        assert!(change_status(&mut items, "missing", WorkItemStatus::Done, 20).is_empty());
        let changes = change_status(&mut items, &original.id, WorkItemStatus::Todo, 20);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].0.as_ref(), Some(&original));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].status, WorkItemStatus::Todo);
    }
}

#[component]
pub fn TaskDetail() -> impl IntoView {
    let saves = expect_context::<crate::task_saves::TaskSaves>();
    let SelectedWorkItem(selected) = expect_context::<SelectedWorkItem>();
    let SetSelectedWorkItem(set_selected) = expect_context::<SetSelectedWorkItem>();
    let WorkItemsSignal(work_items) = expect_context::<WorkItemsSignal>();
    let SetWorkItemsSignal(set_work_items) = expect_context::<SetWorkItemsSignal>();
    let ProjectsSignal(projects) = expect_context::<ProjectsSignal>();

    let current_item = Signal::derive(move || -> Option<WorkItem> {
        let id = selected.get()?;
        work_items.with(|items| items.iter().find(|w| w.id == id).cloned())
    });

    let update_item = move |update: Box<dyn FnOnce(&mut WorkItem)>| {
        let Some(id) = selected.get() else { return };
        let now = crate::clock::now_ms();
        let mut snapshot = None;
        set_work_items.update(|items| {
            if let Some(it) = items.iter_mut().find(|w| w.id == id) {
                let original = it.clone();
                update(it);
                it.updated_at = now;
                snapshot = Some((original, it.clone()));
            }
        });
        if let Some((original, it)) = snapshot {
            storage::persist_work_item(Some(original), it);
        }
    };

    let close = move |_: web_sys::MouseEvent| set_selected.set(None);

    // Destructive action goes through a confirmation dialog (same contract
    // as project deletion in project_sidebar.rs — never delete on one click).
    let pending_delete = RwSignal::new(false);
    let request_delete = move |_: web_sys::MouseEvent| pending_delete.set(true);
    let confirm_delete = move |_: web_sys::MouseEvent| {
        pending_delete.set(false);
        let Some(id) = selected.get_untracked() else {
            return;
        };
        let original =
            work_items.with_untracked(|items| items.iter().find(|w| w.id == id).cloned());
        if let Some(original) = original {
            set_work_items.update(|items| items.retain(|w| w.id != id));
            storage::forget_work_item(original);
        }
        set_selected.set(None);
    };
    let cancel_delete = Callback::new(move |_: web_sys::MouseEvent| pending_delete.set(false));

    view! {
        <Show when=move || current_item.get().is_some() fallback=|| ()>
            {move || {
                let item = match current_item.get() {
                    Some(i) => i,
                    None => return ().into_any(),
                };

                let title_init = item.title.clone();
                let desc_init = item.description.clone().unwrap_or_default();
                let due_init = item.due_date.clone().unwrap_or_default();
                let project_init = item.project_id.clone().unwrap_or_default();
                let priority_init = item.priority.as_str().to_string();
                let cycle_init = item.planning.cycle.clone().unwrap_or_default();
                let waiting_init = item.planning.waiting_on.clone().unwrap_or_default();
                let estimate_init = item
                    .planning
                    .estimate_minutes
                    .map_or_else(String::new, |minutes| minutes.to_string());
                let recurrence_init = item
                    .planning
                    .recurrence
                    .map_or_else(String::new, |rule| rule.as_str().to_string());
                let time_block_count = item.planning.time_block_event_ids.len();

                let on_title = Callback::new(move |v: String| {
                    update_item(Box::new(move |it| it.title = v));
                });
                let on_desc = Callback::new(move |v: String| {
                    update_item(Box::new(move |it| {
                        it.description = if v.trim().is_empty() { None } else { Some(v) };
                    }));
                });
                let on_due = Callback::new(move |v: String| {
                    update_item(Box::new(move |it| {
                        it.due_date = if v.trim().is_empty() { None } else { Some(v) };
                    }));
                });
                let on_project = Callback::new(move |v: String| {
                    update_item(Box::new(move |it| {
                        it.project_id = if v.is_empty() { None } else { Some(v) };
                    }));
                });
                let on_priority = Callback::new(move |v: String| {
                    let p = v.parse().unwrap_or(Priority::None);
                    update_item(Box::new(move |it| it.priority = p));
                });
                let on_cycle = Callback::new(move |v: String| {
                    update_item(Box::new(move |it| {
                        it.planning.cycle = (!v.trim().is_empty()).then_some(v);
                    }));
                });
                let on_waiting = Callback::new(move |v: String| {
                    update_item(Box::new(move |it| {
                        it.planning.waiting_on = (!v.trim().is_empty()).then_some(v);
                    }));
                });
                let on_estimate = Callback::new(move |v: String| {
                    let estimate = v.parse::<u16>().ok();
                    update_item(Box::new(move |it| it.planning.estimate_minutes = estimate));
                });
                let on_recurrence = Callback::new(move |v: String| {
                    let recurrence = RecurrenceRule::from_str(&v);
                    update_item(Box::new(move |it| it.planning.recurrence = recurrence));
                });
                let current_id = item.id.clone();
                let dependency_options = Signal::derive(move || {
                    let mut options = vec![DropdownOption::new("", "Add dependency…")];
                    options.extend(
                        work_items.with(|items| {
                            items
                                .iter()
                                .filter(|candidate| candidate.id != current_id)
                                .map(|candidate| {
                                    DropdownOption::new(candidate.id.clone(), candidate.title.clone())
                                })
                                .collect::<Vec<_>>()
                        }),
                    );
                    options
                });
                let on_dependency = Callback::new(move |dependency_id: String| {
                    if dependency_id.is_empty() {
                        return;
                    }
                    update_item(Box::new(move |it| {
                        if !it.planning.dependency_ids.contains(&dependency_id) {
                            it.planning.dependency_ids.push(dependency_id);
                        }
                    }));
                });

                view! {
                    <div
                        aria-hidden="true"
                        class="fixed inset-0 z-30 bg-longan-950/70 sm:hidden"
                        on:click=close
                    ></div>
                    <aside class="fixed inset-y-0 right-0 z-40 flex h-full w-[min(100vw,24rem)] shrink-0 flex-col bg-longan-900 inset-ring inset-ring-white/5 text-rambutan-300 sm:static sm:z-auto sm:w-96">
                        <div class="flex items-center justify-between px-4 py-3 border-b border-white/5">
                            <span class="text-label text-rambutan-100">"Task"</span>
                            <Button
                                variant=ButtonVariant::Ghost
                                size=ButtonSize::IconSmall
                                title="Close".to_string()
                                on_click=Callback::new(close)
                            >
                                "×"
                            </Button>
                        </div>

                        <div class="flex-1 overflow-y-auto px-4 py-4 space-y-4">
                            // Quick status switcher
                            <div class="flex flex-wrap gap-1.5">
                                {WorkItemStatus::all().iter().map(|&s| {
                                    let label = s.label();
                                    let glyph = s.glyph();
                                    let color = s.color_class();
                                    let item_status = item.status;
                                    let is_active = s == item_status;
                                    let active_class = if is_active {
                                        "bg-longan-700 inset-ring inset-ring-persimmon-400/30"
                                    } else {
                                        "bg-longan-800 inset-ring inset-ring-white/5 hover:inset-ring-white/15"
                                    };
                                    let on_click = move |_| {
                                        let Some(id) = selected.get_untracked() else {
                                            return;
                                        };
                                        let now = crate::clock::now_ms();
                                        let mut changes = Vec::new();
                                        set_work_items.update(|items| changes = change_status(items, &id, s, now));
                                        if !changes.is_empty() {
                                            saves.edit_group(changes);
                                        }
                                    };
                                    view! {
                                        <button
                                            on:click=on_click
                                            class=format!(
                                                "inline-flex items-center gap-1.5 px-2.5 py-1 rounded-grape text-label-sm transition {active_class}"
                                            )
                                        >
                                            <span class=color>{glyph}</span>
                                            <span class="text-rambutan-200">{label}</span>
                                        </button>
                                    }
                                }).collect_view()}
                            </div>

                            <div class="space-y-1.5">
                                <label class="text-label-sm text-rambutan-400">"Title"</label>
                                <Input
                                    id="task-detail-title"
                                    value=Signal::derive(move || title_init.clone())
                                    on_input=on_title
                                    placeholder="Title".to_string()
                                />
                            </div>

                            <div class="space-y-1.5">
                                <label class="text-label-sm text-rambutan-400">"Description"</label>
                                <Textarea
                                    value=Signal::derive(move || desc_init.clone())
                                    on_input=on_desc
                                    placeholder="Add description".to_string()
                                    rows=5
                                />
                            </div>

                            <div class="grid grid-cols-2 gap-3">
                                <div class="space-y-1.5">
                                    <label class="text-label-sm text-rambutan-400">"Project"</label>
                                    <Dropdown
                                        options=Signal::derive(move || {
                                            let mut opts = vec![DropdownOption::new("", "No project")];
                                            opts.extend(
                                                projects.get().into_iter().map(|p| DropdownOption::new(p.id, p.name)),
                                            );
                                            opts
                                        })
                                        value=Signal::derive(move || project_init.clone())
                                        on_change=on_project
                                        label="Project"
                                        class="w-full"
                                    />
                                </div>

                                <div class="space-y-1.5">
                                    <label class="text-label-sm text-rambutan-400">"Priority"</label>
                                    <Dropdown
                                        options={Priority::all()
                                            .iter()
                                            .map(|p| DropdownOption::new(p.as_str(), format!("{} {}", p.glyph(), p.label())))
                                            .collect::<Vec<_>>()}
                                        value=Signal::derive(move || priority_init.clone())
                                        on_change=on_priority
                                        label="Priority"
                                        class="w-full"
                                    />
                                </div>

                                <div class="space-y-1.5">
                                    <label class="text-label-sm text-rambutan-400">"Due date"</label>
                                    <DatePicker
                                        value=Signal::derive(move || due_init.clone())
                                        on_change=on_due
                                    />
                                </div>
                            </div>

                            <div class="border-t border-white/5 pt-4 space-y-3">
                                <div>
                                    <h3 class="text-label text-rambutan-200">"Planning"</h3>
                                    <p class="mt-0.5 text-body-sm text-rambutan-400">"Scheduling intent stays local and updates instantly."</p>
                                </div>
                                <div class="grid grid-cols-2 gap-3">
                                    <div class="space-y-1.5">
                                        <label class="text-label-sm text-rambutan-400">"Estimate"</label>
                                        <Dropdown
                                            options=vec![
                                                DropdownOption::new("", "No estimate"),
                                                DropdownOption::new("15", "15 minutes"),
                                                DropdownOption::new("30", "30 minutes"),
                                                DropdownOption::new("45", "45 minutes"),
                                                DropdownOption::new("60", "1 hour"),
                                                DropdownOption::new("90", "1.5 hours"),
                                                DropdownOption::new("120", "2 hours"),
                                                DropdownOption::new("240", "4 hours"),
                                            ]
                                            value=Signal::derive(move || estimate_init.clone())
                                            on_change=on_estimate
                                            label="Estimate"
                                            class="w-full"
                                        />
                                    </div>
                                    <div class="space-y-1.5">
                                        <label class="text-label-sm text-rambutan-400">"Repeats"</label>
                                        <Dropdown
                                            options=vec![
                                                DropdownOption::new("", "Does not repeat"),
                                                DropdownOption::new("daily", RecurrenceRule::Daily.label()),
                                                DropdownOption::new("weekdays", RecurrenceRule::Weekdays.label()),
                                                DropdownOption::new("weekly", RecurrenceRule::Weekly.label()),
                                                DropdownOption::new("monthly", RecurrenceRule::Monthly.label()),
                                            ]
                                            value=Signal::derive(move || recurrence_init.clone())
                                            on_change=on_recurrence
                                            label="Recurrence"
                                            class="w-full"
                                        />
                                    </div>
                                    <div class="space-y-1.5">
                                        <label class="text-label-sm text-rambutan-400">"Cycle"</label>
                                        <Input
                                            value=Signal::derive(move || cycle_init.clone())
                                            on_input=on_cycle
                                            placeholder="e.g. 2026 W37".to_string()
                                        />
                                    </div>
                                    <div class="space-y-1.5">
                                        <label class="text-label-sm text-rambutan-400">"Waiting on"</label>
                                        <Input
                                            value=Signal::derive(move || waiting_init.clone())
                                            on_input=on_waiting
                                            placeholder="Person or condition".to_string()
                                        />
                                    </div>
                                </div>

                                <div class="space-y-1.5">
                                    <label class="text-label-sm text-rambutan-400">"Dependencies"</label>
                                    <Dropdown
                                        options=dependency_options
                                        value=Signal::derive(String::new)
                                        on_change=on_dependency
                                        label="Add dependency"
                                        class="w-full"
                                    />
                                    <div class="flex flex-wrap gap-1.5">
                                        {item.planning.dependency_ids.into_iter().map(|dependency_id| {
                                            let title = work_items.with_untracked(|items| {
                                                items
                                                    .iter()
                                                    .find(|candidate| candidate.id == dependency_id)
                                                    .map_or_else(|| "Missing task".to_string(), |candidate| candidate.title.clone())
                                            });
                                            let remove_id = dependency_id;
                                            view! {
                                                <button
                                                    type="button"
                                                    class="inline-flex max-w-full items-center gap-1 rounded-grape bg-longan-800 px-2 py-1 text-label-sm text-rambutan-400 hover:text-lychee-300 cursor-pointer"
                                                    title="Remove dependency"
                                                    on:click=move |_| update_item(Box::new({
                                                        let remove_id = remove_id.clone();
                                                        move |it| it.planning.dependency_ids.retain(|id| id != &remove_id)
                                                    }))
                                                >
                                                    <span class="truncate">{title}</span><span aria-hidden="true">"×"</span>
                                                </button>
                                            }
                                        }).collect_view()}
                                    </div>
                                </div>

                                <Show when=move || time_block_count != 0 fallback=|| ()>
                                    <div class="rounded-grape bg-persimmon-500/10 inset-ring inset-ring-persimmon-400/20 px-3 py-2 text-body-sm text-persimmon-200">
                                        {format!("Scheduled in {time_block_count} focus block(s)")}
                                    </div>
                                </Show>
                            </div>
                        </div>

                        <div class="px-4 py-3 border-t border-white/5">
                            <Button
                                variant=ButtonVariant::Destructive
                                size=ButtonSize::Small
                                on_click=Callback::new(request_delete)
                            >
                                "Delete task"
                            </Button>
                        </div>
                    </aside>
                }.into_any()
            }}
        </Show>

        <Dialog
            open=Signal::derive(move || pending_delete.get())
            on_close=Callback::new(move |_| pending_delete.set(false))
            max_width="max-w-sm"
            label="Confirm task deletion"
        >
            <div class="p-4 space-y-4">
                <div class="text-body text-rambutan-100">
                    {move || format!(
                        "Delete task \"{}\"?",
                        current_item.get().map(|i| i.title).unwrap_or_default()
                    )}
                </div>
                <div class="text-label-sm text-rambutan-300">
                    "This can't be undone."
                </div>
                <div class="flex justify-end gap-2">
                    <Button
                        variant=ButtonVariant::Ghost
                        size=ButtonSize::Small
                        on_click=cancel_delete
                    >
                        "Cancel"
                    </Button>
                    <Button
                        variant=ButtonVariant::Destructive
                        size=ButtonSize::Small
                        on_click=Callback::new(confirm_delete)
                    >
                        "Delete"
                    </Button>
                </div>
            </div>
        </Dialog>
    }
}
