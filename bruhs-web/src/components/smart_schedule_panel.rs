//! Local scheduling controls layered onto the calendar.

use bruhs_models::{
    AvailabilityRule, ConflictResolution, FocusProtection, SchedulingLink, WorkItem,
};
use leptos::prelude::*;

use crate::state::{
    AppView, CalendarEventsSignal, ComposerDraft, SchedulingPreferencesSignal, SetActiveView,
    WorkItemsSignal,
};
use crate::ui::{Dialog, Dropdown, DropdownOption, Input};

#[derive(Clone)]
enum Notice {
    Saved(String),
    Error(String),
}

#[component]
pub fn SmartSchedulePanel(open: RwSignal<bool>) -> impl IntoView {
    let SchedulingPreferencesSignal(preferences) = expect_context::<SchedulingPreferencesSignal>();
    Effect::new(move |_| {
        if open.get() {
            wasm_bindgen_futures::spawn_local(async move {
                let _ = preferences.reload().await;
            });
        }
    });
    let CalendarEventsSignal(events) = expect_context::<CalendarEventsSignal>();
    let WorkItemsSignal(work_items) = expect_context::<WorkItemsSignal>();
    let ComposerDraft(composer_draft) = expect_context::<ComposerDraft>();
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();
    let saves = expect_context::<crate::task_saves::TaskSaves>();
    let link_title = RwSignal::new(String::new());
    let link_duration = RwSignal::new("30".to_string());
    let selected_task = RwSignal::new(String::new());
    let notice = RwSignal::new(Option::<Notice>::None);
    let scheduled_event = RwSignal::new(Option::<String>::None);

    let add_link = move || {
        scheduled_event.set(None);
        let title = link_title.get_untracked();
        if title.trim().is_empty() {
            notice.set(Some(Notice::Error(
                "Give the scheduling link a title.".to_string(),
            )));
            return;
        }
        let duration = link_duration_minutes(&link_duration.get_untracked());
        wasm_bindgen_futures::spawn_local(async move {
            if preferences
                .edit(|prefs| {
                    prefs.add_link(
                        SchedulingLink::new(title.clone(), duration),
                        crate::clock::now_ms(),
                    )
                })
                .await
                .is_err()
            {
                notice.set(None);
                return;
            }
            if link_title.get_untracked() == title {
                link_title.set(String::new());
            }
            notice.set(Some(Notice::Saved(
                "Scheduling link saved locally.".to_string(),
            )));
        });
    };

    let schedule_task = move || {
        scheduled_event.set(None);
        saves.place_task(
            selected_task.get_untracked(),
            Callback::new(move |result| match result {
                Ok(id) => {
                    selected_task.set(String::new());
                    notice.set(None);
                    scheduled_event.set(Some(id));
                }
                Err(message) => notice.set(Some(Notice::Error(message))),
            }),
        );
    };

    let ask_agent = move |_| {
        if !preferences.ready() {
            return;
        }
        let prompt = preferences.with_untracked(|prefs| {
            work_items.with_untracked(|tasks| {
                events
                    .with_untracked(|evs| bruhs_models::scheduling_agent_prompt(prefs, tasks, evs))
            })
        });
        composer_draft.set(prompt);
        open.set(false);
        set_active_view.set(AppView::Chat);
    };
    let toggle_focus = move |_| {
        wasm_bindgen_futures::spawn_local(async move {
            let _ = preferences
                .edit(|prefs| {
                    prefs.focus.enabled = !prefs.focus.enabled;
                    prefs.updated_at_ms = crate::clock::now_ms();
                })
                .await;
        });
    };
    let focus_on = move || preferences.with(|prefs| prefs.focus.enabled);
    let nudge_button = move |nudge: Nudge, label: &'static str| {
        let on_click = move |_| {
            wasm_bindgen_futures::spawn_local(async move {
                let _ = preferences
                    .edit(|prefs| {
                        if let Some(rule) = prefs.availability.first_mut() {
                            nudge.apply(rule);
                            prefs.updated_at_ms = crate::clock::now_ms();
                        }
                    })
                    .await;
            });
        };
        view! {
            <button
                type="button"
                class="rounded-seed px-2 py-1 text-label-sm text-rambutan-400 inset-ring inset-ring-longan-700 hover:bg-longan-800 cursor-pointer"
                disabled=move || preferences.busy() || saves.day_plan.with(|state|state.busy)
                on:click=on_click
            >
                {label}
            </button>
        }
    };
    let link_rows = move || {
        preferences
            .with(|prefs| prefs.links.clone())
            .into_iter()
            .map(|link| {
                let href = format!("#/book/{}", link.slug);
                let text = href.clone();
                view! {
                    <div class="flex items-center justify-between rounded-seed bg-longan-900/55 px-2.5 py-2">
                        <span class="truncate text-body-sm text-rambutan-300">
                            {format!("{}, {} min", link.title, link.duration_minutes)}
                        </span>
                        <a class="text-label-sm text-blueberry-300 hover:text-blueberry-200" href=href>{text}</a>
                    </div>
                }
            })
            .collect_view()
    };
    let conflict_rows = move || {
        let open_conflicts = preferences.with(|prefs| {
            prefs
                .conflicts
                .iter()
                .filter(|conflict| conflict.resolution.is_none())
                .cloned()
                .collect::<Vec<_>>()
        });
        open_conflicts
            .into_iter()
            .map(|conflict| {
                let resolve_button =
                    move |resolution: ConflictResolution, label: &'static str, class: &'static str| {
                        let id = conflict.id.clone();
                        let on_click = move |_| {
                            saves.resolve_calendar_conflict(id.clone(), resolution)
                        };
                        view! { <button type="button" class=class disabled=move || preferences.busy() || saves.day_plan.with(|state|state.busy) on:click=on_click>{label}</button> }
                    };
                let neutral = "rounded-seed px-2 py-1 text-label-sm text-rambutan-300 inset-ring inset-ring-longan-700 hover:bg-longan-800 cursor-pointer";
                view! {
                    <div class="rounded-seed bg-lychee-500/8 px-2.5 py-2">
                        <p class="text-body-sm text-rambutan-300">
                            {format!(
                                "{} changed remotely. Your local time is preserved until you choose.",
                                conflict.event_title
                            )}
                        </p>
                        <div class="mt-2 flex flex-wrap gap-1">
                            {resolve_button(ConflictResolution::KeepLocal, "Keep local", neutral)}
                            {resolve_button(ConflictResolution::AcceptRemote, "Accept remote", neutral)}
                            {resolve_button(
                                ConflictResolution::DuplicateLocal,
                                "Keep both",
                                "rounded-seed px-2 py-1 text-label-sm text-persimmon-300 inset-ring inset-ring-persimmon-500/25 hover:bg-persimmon-500/10 cursor-pointer",
                            )}
                        </div>
                    </div>
                }
            })
            .collect_view()
    };
    let save_status = move |id: String| {
        move || {
            match saves.record_status(crate::task_saves::Kind::Event, &id) {
            crate::durable_metadata::SaveState::Saved => "Task scheduled in the first protected opening.",
            crate::durable_metadata::SaveState::Saving => "Saving task and calendar event…",
            crate::durable_metadata::SaveState::Error { .. } => {
                "Task and event not saved. Close this panel to retry from the save recovery notice; your changes are kept."
            }
            _ => "Preparing task and calendar event…",
        }
        }
    };

    view! {
        <Show when=move || open.get()>
            <Dialog open=Signal::derive(move || open.get()) label="Scheduling controls" on_close=Callback::new(move |_| open.set(false))>
                // Leave the shared dialog's 5rem top inset and 1rem below the card.
                <div class="flex max-h-[calc(100dvh-6rem)] flex-col">
                <div class="shrink-0 border-b border-longan-700/70 px-4 py-3">
                    <div class="flex items-start justify-between gap-3">
                        <div>
                            <h2 class="text-h4 text-rambutan-100">"Smart scheduling"</h2>
                            <p class="mt-1 text-body-sm text-rambutan-500">
                                "Protect intent first. Saved availability guides local task placement; remote conflicts wait for your decision."
                            </p>
                        </div>
                        <button
                            type="button"
                            class="shrink-0 rounded-grape bg-blueberry-500/15 px-3 py-2 text-label-sm text-blueberry-300 hover:bg-blueberry-500/25 cursor-pointer"
                            disabled=move || !preferences.ready()
                            on:click=ask_agent
                        >
                            "Ask agent to plan"
                        </button>
                    </div>
                </div>
                <super::scheduling_status::SchedulingStatus />
                <Show when=move || preferences.error().is_some() || saves.day_plan.with(|state|state.error.is_some())>
                    <section aria-label="Scheduling data recovery" class="shrink-0 border-b border-longan-700 px-4 py-3 text-body-sm text-rambutan-300">
                        {move || saves.day_plan.with(|state|state.error.clone()).map(|message|view! {<p role="alert" class="text-lychee-300">{message}</p>})}
                        <p>"If calendar data changed in another tab, refresh tasks and calendar, then reload saved preferences before repeating the conflict decision."</p>
                        <button type="button" class="bruhs-control mt-2 rounded-grape bg-longan-800 px-3 py-2 cursor-pointer"
                            disabled=move || saves.pending()
                            on:click=move |_| saves.refresh_day_plan_sources()>
                            "Refresh saved tasks and calendar"
                        </button>
                    </section>
                </Show>
                <div class="min-h-0 max-h-[70vh] space-y-4 overflow-auto px-4 py-3">
                    <Show when=move || preferences.ready()>
                    <section class="rounded-grape bg-longan-900/55 p-3 inset-ring inset-ring-longan-800">
                        <div class="flex items-center justify-between gap-3">
                            <div>
                                <h3 class="text-label text-rambutan-200">"Focus protection"</h3>
                                <p class="text-label-sm text-rambutan-500">
                                    {move || preferences.with(|prefs| focus_label(&prefs.focus))}
                                </p>
                            </div>
                            <button
                                type="button"
                                aria-pressed=move || focus_on().to_string()
                                class=move || if focus_on() {
                                    "rounded-seed bg-pandan-500/15 px-2.5 py-1.5 text-label-sm text-pandan-300 cursor-pointer"
                                } else {
                                    "rounded-seed bg-longan-800 px-2.5 py-1.5 text-label-sm text-rambutan-500 cursor-pointer"
                                }
                                disabled=move || preferences.busy() || saves.day_plan.with(|state|state.busy)
                                on:click=toggle_focus
                            >
                                {move || if focus_on() { "Protected" } else { "Off" }}
                            </button>
                        </div>
                        <p class="mt-2 text-body-sm text-rambutan-400">
                            {move || preferences.with(|prefs| availability_label(prefs.availability.first()))}
                        </p>
                        <div class="mt-2 flex flex-wrap gap-1">
                            {nudge_button(Nudge::StartEarlier, "Start −30m")}
                            {nudge_button(Nudge::StartLater, "Start +30m")}
                            {nudge_button(Nudge::EndEarlier, "End −30m")}
                            {nudge_button(Nudge::EndLater, "End +30m")}
                        </div>
                    </section>
                    <section class="space-y-2">
                        <h3 class="text-label text-rambutan-200">"Schedule a task"</h3>
                        <div class="flex gap-2">
                            <Dropdown
                                options=Signal::derive(move || work_items.with(|tasks| unscheduled_task_options(tasks)))
                                value=Signal::derive(move || selected_task.get())
                                on_change=Callback::new(move |value| selected_task.set(value))
                                placeholder="Choose an unscheduled task"
                                label="Task to schedule"
                                class="min-w-0 flex-1"
                            />
                            <button
                                type="button"
                                class="rounded-grape bg-blueberry-500/15 px-3 py-2 text-label-sm text-blueberry-300 hover:bg-blueberry-500/25 cursor-pointer"
                                disabled=move || preferences.busy() || saves.pending()
                                on:click=move |_| schedule_task()
                            >
                                "Find time"
                            </button>
                        </div>
                    </section>
                    <section class="space-y-2">
                        <h3 class="text-label text-rambutan-200">"Scheduling links"</h3>
                        <div class="grid gap-2 sm:grid-cols-[1fr_7rem_auto]">
                            <Input
                                value=Signal::derive(move || link_title.get())
                                on_input=Callback::new(move |value| link_title.set(value))
                                placeholder="Product review".to_string()
                            />
                            <Input
                                input_type="number".to_string()
                                value=Signal::derive(move || link_duration.get())
                                on_input=Callback::new(move |value| link_duration.set(value))
                                placeholder="30 min".to_string()
                            />
                            <button
                                type="button"
                                class="rounded-grape bg-persimmon-500 px-3 py-2 text-label text-longan-950 hover:bg-persimmon-400 cursor-pointer"
                                disabled=move || preferences.busy() || saves.day_plan.with(|state|state.busy)
                                on:click=move |_| add_link()
                            >
                                "Create"
                            </button>
                        </div>
                        <div class="space-y-1">{link_rows}</div>
                    </section>
                    <Show when=move || preferences.with(|prefs| prefs.conflicts.iter().any(|conflict| conflict.resolution.is_none()))>
                        <section class="space-y-2">
                            <h3 class="text-label text-lychee-300">"Remote changes need review"</h3>
                            {conflict_rows}
                        </section>
                    </Show>
                    <Show when=move || notice.get().is_some()>
                        {move || notice.get().map(|notice| {
                            let (role,class,message)=match notice {
                                Notice::Saved(message)=>("status","text-body-sm text-pandan-300",message),
                                Notice::Error(message)=>("alert","text-body-sm text-lychee-300",message),
                            };
                            view! {<p role=role class=class>{message}</p>}
                        })}
                    </Show>
                    {move || scheduled_event.get().map(|id| view! {
                        <p role="status" class="text-body-sm text-rambutan-300">{save_status(id)}</p>
                    })}
                    </Show>
                </div>
                </div>
            </Dialog>
        </Show>
    }
}

/// One of the availability nudges (±30 minutes on either edge).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Nudge {
    StartEarlier,
    StartLater,
    EndEarlier,
    EndLater,
}

impl Nudge {
    /// Move the edge by 30 minutes, keeping at least 15 minutes open and the
    /// end within the day.
    fn apply(self, rule: &mut AvailabilityRule) {
        match self {
            Self::StartEarlier => rule.start_minute = rule.start_minute.saturating_sub(30),
            Self::StartLater => {
                rule.start_minute =
                    (rule.start_minute + 30).min(rule.end_minute.saturating_sub(15));
            }
            Self::EndEarlier => {
                rule.end_minute = rule
                    .end_minute
                    .saturating_sub(30)
                    .max(rule.start_minute + 15);
            }
            Self::EndLater => rule.end_minute = (rule.end_minute + 30).min(1_440),
        }
    }
}

/// "120 min/day · blocks of at least 45 min".
fn focus_label(focus: &FocusProtection) -> String {
    format!(
        "{} min/day, blocks of at least {} min",
        focus.daily_target_minutes, focus.minimum_block_minutes
    )
}

/// "Working hours, weekdays, 09:00 to 17:30", or a note that there is none.
fn availability_label(rule: Option<&AvailabilityRule>) -> String {
    rule.map_or("No availability rule".to_string(), |rule| {
        format!(
            "{}, weekdays, {:02}:{:02} to {:02}:{:02}",
            rule.label,
            rule.start_minute / 60,
            rule.start_minute % 60,
            rule.end_minute / 60,
            rule.end_minute % 60
        )
    })
}

/// Open tasks without a time block, as "Title · 60 min" options.
fn unscheduled_task_options(tasks: &[WorkItem]) -> Vec<DropdownOption> {
    tasks
        .iter()
        .filter(|task| !task.status.is_completed() && task.planning.time_block_event_ids.is_empty())
        .map(|task| {
            DropdownOption::new(
                task.id.clone(),
                format!(
                    "{}, {} min",
                    task.title,
                    task.planning.estimate_minutes.unwrap_or(60)
                ),
            )
        })
        .collect()
}

/// A scheduling link's length: the typed minutes, else 30.
fn link_duration_minutes(text: &str) -> u16 {
    text.parse::<u16>().unwrap_or(30)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(start: u16, end: u16) -> AvailabilityRule {
        AvailabilityRule {
            id: "r".into(),
            label: "Working hours".into(),
            weekdays: vec![1, 2, 3, 4, 5],
            start_minute: start,
            end_minute: end,
            enabled: true,
        }
    }

    #[test]
    fn nudges_keep_a_quarter_hour_open_and_stay_in_the_day() {
        let nudged = |start, end, nudge: Nudge| {
            let mut r = rule(start, end);
            nudge.apply(&mut r);
            (r.start_minute, r.end_minute)
        };
        assert_eq!(nudged(540, 1_020, Nudge::StartEarlier), (510, 1_020));
        assert_eq!(nudged(10, 1_020, Nudge::StartEarlier), (0, 1_020));
        assert_eq!(nudged(540, 1_020, Nudge::StartLater), (570, 1_020));
        assert_eq!(nudged(1_000, 1_020, Nudge::StartLater), (1_005, 1_020));
        assert_eq!(nudged(540, 1_020, Nudge::EndEarlier), (540, 990));
        assert_eq!(nudged(540, 560, Nudge::EndEarlier), (540, 555));
        assert_eq!(nudged(540, 1_430, Nudge::EndLater), (540, 1_440));
    }

    #[test]
    fn labels_and_link_lengths() {
        assert_eq!(
            availability_label(Some(&rule(540, 1_050))),
            "Working hours, weekdays, 09:00 to 17:30"
        );
        assert_eq!(availability_label(None), "No availability rule");
        assert_eq!(link_duration_minutes("45"), 45);
        assert_eq!(link_duration_minutes("abc"), 30);
        assert_eq!(link_duration_minutes("-5"), 30);
        assert_eq!(link_duration_minutes("70000"), 30);
    }
}
