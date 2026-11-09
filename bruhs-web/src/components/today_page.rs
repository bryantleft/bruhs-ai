use crate::task_style::PriorityStyle;
use bruhs_models::num::ToIntSaturating;
use leptos::prelude::*;

use crate::state::{
    AppView, AutomationStateSignal, CalendarEvent, CalendarEventsSignal, CalendarMode, EventSource,
    MeetingsSignal, PeopleSignal, PlanDayRequest, SetActiveView, SetCalendarAnchor,
    SetCalendarEventsSignal, SetCalendarModeSignal, SetSelectedEventId, SetSelectedWorkItem,
    SetWorkItemsSignal, TodaySnapshot, WorkItem, WorkItemsSignal,
};
use crate::storage;
use crate::ui::{Button, ButtonSize, ButtonVariant, Dialog};
use bruhs_models::{
    propose_day_plan, AgentDomain, DayPlanProposal, ProactiveSuggestion, WorkflowOperation,
};

#[derive(Clone)]
struct AppliedPlan {
    previous_tasks: Vec<WorkItem>,
    event_ids: Vec<String>,
    saving: bool,
}

fn local_day_bounds(now_ms: f64) -> (f64, f64) {
    let now = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(now_ms));
    let start = js_sys::Date::new_with_year_month_day(
        now.get_full_year(),
        crate::clock::field(now.get_month()),
        crate::clock::field(now.get_date()),
    )
    .get_time();
    let end = js_sys::Date::new_with_year_month_day(
        now.get_full_year(),
        crate::clock::field(now.get_month()),
        crate::clock::field(now.get_date()) + 1,
    )
    .get_time();
    (start, end)
}

fn local_date_key(ms: f64) -> String {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms));
    format!(
        "{:04}-{:02}-{:02}",
        date.get_full_year(),
        date.get_month() + 1,
        date.get_date()
    )
}

fn greeting_date(ms: f64) -> String {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms));
    let days = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    let months = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    format!(
        "{}, {} {}",
        days[date.get_day() as usize],
        months[date.get_month() as usize],
        date.get_date()
    )
}

fn format_time(ms: f64) -> String {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms));
    let hour = date.get_hours();
    let minute = date.get_minutes();
    let suffix = if hour >= 12 { "pm" } else { "am" };
    let display_hour = match hour % 12 {
        0 => 12,
        value => value,
    };
    if minute == 0 {
        format!("{display_hour}{suffix}")
    } else {
        format!("{display_hour}:{minute:02}{suffix}")
    }
}

fn duration_label(start_at_ms: f64, end_at_ms: f64) -> String {
    let minutes = ((end_at_ms - start_at_ms) / 60_000.0)
        .round()
        .to_i64_saturating();
    if minutes >= 60 && minutes % 60 == 0 {
        format!("{}h", minutes / 60)
    } else {
        format!("{minutes}m")
    }
}

fn empty_row(label: &'static str) -> impl IntoView {
    view! {
        <div class="rounded-grape border border-dashed border-longan-700/60 px-3 py-3 text-body-sm text-rambutan-400">
            {label}
        </div>
    }
}

#[component]
pub fn TodayPage() -> impl IntoView {
    let WorkItemsSignal(work_items) = expect_context::<WorkItemsSignal>();
    let SetWorkItemsSignal(set_work_items) = expect_context::<SetWorkItemsSignal>();
    let CalendarEventsSignal(events) = expect_context::<CalendarEventsSignal>();
    let SetCalendarEventsSignal(set_events) = expect_context::<SetCalendarEventsSignal>();
    let MeetingsSignal(meetings) = expect_context::<MeetingsSignal>();
    let PeopleSignal(people) = expect_context::<PeopleSignal>();
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();
    let SetSelectedWorkItem(set_selected_work_item) = expect_context::<SetSelectedWorkItem>();
    let SetSelectedEventId(set_selected_event_id) = expect_context::<SetSelectedEventId>();
    let SetCalendarAnchor(set_calendar_anchor) = expect_context::<SetCalendarAnchor>();
    let SetCalendarModeSignal(set_calendar_mode) = expect_context::<SetCalendarModeSignal>();
    let PlanDayRequest(plan_day_request) = expect_context::<PlanDayRequest>();
    let AutomationStateSignal(automation) = expect_context::<AutomationStateSignal>();

    let now_ms = RwSignal::new(js_sys::Date::now());
    let proposal = RwSignal::new(Option::<DayPlanProposal>::None);
    let applied_plan = RwSignal::new(Option::<AppliedPlan>::None);
    let persistence_error = RwSignal::new(Option::<String>::None);
    let (day_start, day_end) = local_day_bounds(now_ms.get_untracked());
    let today_key = local_date_key(now_ms.get_untracked());
    let date_label = greeting_date(now_ms.get_untracked());

    Effect::new(move |_| {
        if !automation.ready() {
            return;
        }
        let unfinished =
            work_items.with(|items| items.iter().filter(|item| !item.is_completed()).count());
        if unfinished == 0 {
            return;
        }
        let event_count = events.with(Vec::len);
        automation.update(|state| {
            if state
                .suggestions
                .iter()
                .all(|suggestion| suggestion.id != "protect-focus")
            {
                let now = crate::clock::now_ms();
                let start_at_ms = ((bruhs_models::EpochMs::saturating_from_i64(now).as_f64() / 1_800_000.0).ceil() + 1.0) * 1_800_000.0;
                state.suggestions.push(ProactiveSuggestion {
                    id: "protect-focus".into(),
                    title: "Protect a focus block".into(),
                    explanation: "A task, calendar block, and local note can be reviewed as one reversible transaction."
                        .into(),
                    evidence: format!(
                        "{unfinished} unfinished tasks and {event_count} calendar entries are stored locally."
                    ),
                    domain: AgentDomain::Tasks,
                    operations: vec![
                        WorkflowOperation::CreateTask {
                            title: "Finish the highest-impact task".into(),
                        },
                        WorkflowOperation::CreateCalendarEvent {
                            title: "Protected focus".into(),
                            start_at_ms,
                            end_at_ms: start_at_ms + 45.0 * 60_000.0,
                        },
                        WorkflowOperation::AddCapture {
                            content: "Focus plan\nProtected 45 minutes for the highest-impact task."
                                .into(),
                        },
                    ],
                    dismissed: false,
                    shown_count: 0,
                    last_shown_at_ms: None,
                });
            }
        });
    });

    let snapshot = Memo::new(move |_| {
        TodaySnapshot::build(
            now_ms.get(),
            day_start,
            day_end,
            &today_key,
            &work_items.read(),
            &events.read(),
            &meetings.read(),
            &people.read(),
        )
    });

    let open_task = move |id: String| {
        set_selected_work_item.set(Some(id));
        set_active_view.set(AppView::Tasks);
    };
    let open_event = move |id: String, at_ms: f64| {
        set_selected_event_id.set(Some(id));
        set_calendar_anchor.set(at_ms);
        set_calendar_mode.set(CalendarMode::Day);
        set_active_view.set(AppView::Calendar);
    };
    let draft_plan = move |_| {
        let plan = propose_day_plan(
            now_ms.get_untracked(),
            day_start,
            &work_items.get_untracked(),
            &events.get_untracked(),
        );
        proposal.set(Some(plan));
    };
    Effect::new(move |_| {
        if plan_day_request.take() {
            let plan = propose_day_plan(
                now_ms.get_untracked(),
                day_start,
                &work_items.get_untracked(),
                &events.get_untracked(),
            );
            proposal.set(Some(plan));
        }
    });
    let close_plan = Callback::new(move |_| proposal.set(None));

    let apply_plan = move |_| {
        let Some(plan) = proposal.get_untracked() else {
            return;
        };
        let previous_tasks = work_items.with_untracked(|tasks| {
            tasks
                .iter()
                .filter(|task| plan.blocks.iter().any(|block| block.task_id == task.id))
                .cloned()
                .collect::<Vec<_>>()
        });
        let now = crate::clock::now_ms();
        let planned_events = plan
            .blocks
            .iter()
            .map(|block| CalendarEvent {
                id: block.id.clone(),
                title: format!("Focus · {}", block.task_title),
                start_at_ms: block.start_at_ms,
                end_at_ms: block.end_at_ms,
                all_day: false,
                color: Some("persimmon".to_string()),
                notes: Some(format!("Planned from task {}", block.task_id)),
                created_at: now,
                updated_at: now,
                source: EventSource::Local,
                google_event_id: None,
                calendar_id: None,
                attendees: Vec::new(),
                organizer: None,
                location: None,
                conferencing: None,
                recurring_event_id: None,
                html_link: None,
            })
            .collect::<Vec<_>>();
        let event_ids = planned_events
            .iter()
            .map(|event| event.id.clone())
            .collect::<Vec<_>>();
        set_work_items.update(|items| {
            for block in &plan.blocks {
                if let Some(task) = items.iter_mut().find(|task| task.id == block.task_id) {
                    task.planning.time_block_event_ids.push(block.id.clone());
                    task.updated_at = now;
                }
            }
        });
        set_events.update(|items| items.extend(planned_events.clone()));
        let persisted_tasks = work_items.with_untracked(|tasks| {
            tasks
                .iter()
                .filter(|task| plan.blocks.iter().any(|block| block.task_id == task.id))
                .cloned()
                .collect::<Vec<_>>()
        });
        let applied = AppliedPlan {
            previous_tasks,
            event_ids,
            saving: true,
        };
        applied_plan.set(Some(applied.clone()));
        proposal.set(None);
        persistence_error.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(error) = storage::save_day_plan(&persisted_tasks, &planned_events).await {
                set_work_items.update(|items| {
                    for previous in &applied.previous_tasks {
                        if let Some(current) = items.iter_mut().find(|task| task.id == previous.id)
                        {
                            *current = previous.clone();
                        }
                    }
                });
                set_events.update(|items| {
                    items.retain(|event| !applied.event_ids.contains(&event.id));
                });
                applied_plan.set(None);
                persistence_error.set(Some(format!("Plan could not be saved: {error:?}")));
            } else {
                applied_plan.update(|current| {
                    if let Some(current) = current {
                        current.saving = false;
                    }
                });
            }
        });
    };

    let undo_plan = move |_| {
        let Some(applied) = applied_plan.get_untracked() else {
            return;
        };
        if applied.saving {
            return;
        }
        set_work_items.update(|items| {
            for previous in &applied.previous_tasks {
                if let Some(current) = items.iter_mut().find(|task| task.id == previous.id) {
                    *current = previous.clone();
                }
            }
        });
        set_events.update(|items| items.retain(|event| !applied.event_ids.contains(&event.id)));
        applied_plan.set(None);
        let previous_tasks = applied.previous_tasks;
        let event_ids = applied.event_ids;
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(error) = storage::undo_day_plan(&previous_tasks, &event_ids).await {
                persistence_error.set(Some(format!("Undo could not be saved: {error:?}")));
            }
        });
    };

    view! {
        <section class="flex flex-1 min-h-0 min-w-0 flex-col bg-longan-950" aria-labelledby="today-title">
            <header class="flex flex-wrap shrink-0 items-center justify-between gap-3 border-b border-longan-800/50 px-4 py-3 sm:px-6">
                <div class="min-w-0">
                    <h1 id="today-title" class="text-h3 text-rambutan-100">"Today"</h1>
                    <p class="mt-0.5 text-body-sm text-rambutan-500">{date_label}</p>
                </div>
                <div class="flex flex-wrap items-center gap-2">
                    <Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| set_active_view.set(AppView::Automations))>
                        "Agent workflows"
                    </Button>
                    <Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| set_active_view.set(AppView::Life))>
                        "Life"
                    </Button>
                    <Button variant=ButtonVariant::Primary size=ButtonSize::Small on_click=Callback::new(draft_plan)>
                        "Plan my day"
                    </Button>
                </div>
            </header>

            <div class="flex-1 overflow-y-auto px-4 py-4 sm:px-6 sm:py-5">
                <div class="mx-auto max-w-6xl space-y-4">
                    <Show when=move || persistence_error.get().is_some() fallback=|| ()>
                        <div role="alert" class="flex items-center justify-between gap-3 rounded-grape bg-lychee-500/10 inset-ring inset-ring-lychee-400/20 px-3 py-2 text-body-sm text-lychee-200">
                            <span>{move || persistence_error.get().unwrap_or_default()}</span>
                            <button class="text-button hover:text-lychee-100 cursor-pointer" on:click=move |_| persistence_error.set(None)>"Dismiss"</button>
                        </div>
                    </Show>
                    <Show when=move || applied_plan.get().is_some() fallback=|| ()>
                        <div role="status" class="flex items-center justify-between gap-3 rounded-grape bg-pandan-500/10 inset-ring inset-ring-pandan-400/20 px-3 py-2 text-body-sm text-pandan-200">
                            <span>"Day plan applied as one local transaction."</span>
                            <Show
                                when=move || applied_plan.get().is_some_and(|plan| !plan.saving)
                                fallback=|| view! { <span class="text-label-sm text-pandan-300/70">"Saving…"</span> }
                            >
                                <button class="text-button text-pandan-100 hover:text-white cursor-pointer" on:click=undo_plan>"Undo"</button>
                            </Show>
                        </div>
                    </Show>

                    {move || automation.with(|state| state.surfaceable_suggestions(crate::clock::now_ms()).into_iter().next().cloned().map(|suggestion| {
                        let id = suggestion.id.clone();
                        view! {
                            <article class="flex flex-wrap items-start justify-between gap-3 rounded-2xl bg-blueberry-500/10 px-4 py-3 inset-ring inset-ring-blueberry-400/20" aria-label="Proactive agent suggestion">
                                <div class="min-w-0 w-full break-words sm:w-auto sm:flex-1">
                                    <p class="text-label-sm uppercase tracking-wider text-blueberry-300">"Suggested locally"</p>
                                    <p class="mt-1 text-label text-rambutan-100">{suggestion.title}</p>
                                    <p class="mt-1 text-body-sm text-rambutan-400">{suggestion.explanation}</p>
                                    <p class="mt-1 text-label-sm text-blueberry-300">{format!("Evidence · {}", suggestion.evidence)}</p>
                                </div>
                                <div class="flex items-center gap-2">
                                    <button type="button" class="text-button text-rambutan-500 hover:text-rambutan-200 cursor-pointer" aria-label="Dismiss proactive suggestion" on:click=move |_| automation.update(|state| { let _ = state.dismiss_suggestion(&id, crate::clock::now_ms()); })>"Dismiss"</button>
                                    <button type="button" class="rounded-grape bg-blueberry-500/15 px-3 py-1.5 text-button text-blueberry-200 inset-ring inset-ring-blueberry-400/20 hover:bg-blueberry-500/20 cursor-pointer" on:click=move |_| set_active_view.set(AppView::Automations)>"Review"</button>
                                </div>
                            </article>
                        }
                    }))}

                    <article class="rounded-2xl bg-longan-900 inset-ring inset-ring-white/5 px-4 py-4 sm:px-5">
                        <div class="flex items-start gap-3">
                            <div class="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-grape bg-persimmon-500/15 text-persimmon-300 inset-ring inset-ring-persimmon-400/20" aria-hidden="true">"✦"</div>
                            <div class="min-w-0">
                                <p class="text-label-sm uppercase tracking-wider text-persimmon-300">"Briefing"</p>
                                <p class="mt-1 text-body text-rambutan-200">{move || snapshot.get().briefing}</p>
                            </div>
                        </div>
                    </article>

                    <div class="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)]">
                        <section class="min-w-0 rounded-2xl bg-longan-900 inset-ring inset-ring-white/5 p-4" aria-labelledby="schedule-heading">
                            <div class="mb-3 flex items-center justify-between">
                                <h2 id="schedule-heading" class="text-h4 text-rambutan-100">"Now & next"</h2>
                                <button class="text-button text-rambutan-500 hover:text-rambutan-200 cursor-pointer" on:click=move |_| set_active_view.set(AppView::Calendar)>"Calendar →"</button>
                            </div>
                            <div class="space-y-2">
                                {move || snapshot.get().now.map(|entry| {
                                    let event_id = entry.event_id.clone();
                                    let at_ms = entry.start_at_ms;
                                    view! {
                                        <button type="button" class="flex w-full items-center gap-3 rounded-grape bg-persimmon-500/10 inset-ring inset-ring-persimmon-400/20 px-3 py-2.5 text-left hover:bg-persimmon-500/15 cursor-pointer" on:click=move |_| open_event(event_id.clone(), at_ms)>
                                            <span class="w-12 shrink-0 text-label-sm text-persimmon-300">"Now"</span>
                                            <span class="min-w-0 flex-1 truncate text-body text-rambutan-100">{entry.title}</span>
                                            <span class="text-label-sm text-rambutan-500">{duration_label(entry.start_at_ms, entry.end_at_ms)}</span>
                                        </button>
                                    }
                                })}
                                {move || {
                                    let next = snapshot.get().next;
                                    if next.is_empty() {
                                        empty_row("Nothing else on the calendar today.").into_any()
                                    } else {
                                        next.into_iter().map(|entry| {
                                            let event_id = entry.event_id.clone();
                                            let at_ms = entry.start_at_ms;
                                            let time = if entry.all_day { "All day".to_string() } else { format_time(entry.start_at_ms) };
                                            view! {
                                                <button type="button" class="flex w-full items-center gap-3 rounded-grape px-3 py-2.5 text-left hover:bg-longan-800/60 cursor-pointer" on:click=move |_| open_event(event_id.clone(), at_ms)>
                                                    <span class="w-12 shrink-0 text-label-sm text-rambutan-500">{time}</span>
                                                    <span class="min-w-0 flex-1 truncate text-body text-rambutan-200">{entry.title}</span>
                                                    <span class="text-label-sm text-rambutan-400">{duration_label(entry.start_at_ms, entry.end_at_ms)}</span>
                                                </button>
                                            }
                                        }).collect_view().into_any()
                                    }
                                }}
                            </div>
                        </section>

                        <section class="min-w-0 rounded-2xl bg-longan-900 inset-ring inset-ring-white/5 p-4" aria-labelledby="focus-heading">
                            <div class="mb-3 flex items-center justify-between">
                                <h2 id="focus-heading" class="text-h4 text-rambutan-100">"Focus queue"</h2>
                                <span class="text-label-sm text-rambutan-400">{move || format!("{} ready", snapshot.get().focus_queue.len())}</span>
                            </div>
                            <div class="space-y-1">
                                {move || {
                                    let queue = snapshot.get().focus_queue;
                                    if queue.is_empty() {
                                        empty_row("No ready tasks. Add one or clear a dependency.").into_any()
                                    } else {
                                        queue.into_iter().enumerate().map(|(index, task)| {
                                            let task_id = task.id.clone();
                                            let estimate = task.planning.estimate_minutes.map_or("45m".to_string(), |minutes| format!("{minutes}m"));
                                            view! {
                                                <button type="button" class="group flex w-full items-center gap-3 rounded-grape px-2.5 py-2 text-left hover:bg-longan-800/60 cursor-pointer" on:click=move |_| open_task(task_id.clone())>
                                                    <span class=if index == 0 { "flex size-6 shrink-0 items-center justify-center rounded-full bg-persimmon-500/15 text-label-sm text-persimmon-300" } else { "flex size-6 shrink-0 items-center justify-center rounded-full bg-longan-800 text-label-sm text-rambutan-500" }>{index + 1}</span>
                                                    <span class="min-w-0 flex-1 truncate text-body text-rambutan-200 group-hover:text-rambutan-100">{task.title}</span>
                                                    <span class=format!("text-label-sm {}", task.priority.color_class())>{task.priority.label()}</span>
                                                    <span class="w-9 text-right text-label-sm text-rambutan-400">{estimate}</span>
                                                </button>
                                            }
                                        }).collect_view().into_any()
                                    }
                                }}
                            </div>
                        </section>
                    </div>

                    <div class="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-4">
                        <TodayTaskSection title="Overdue" tone="lychee" empty="Nothing overdue." items=Signal::derive(move || snapshot.get().overdue) on_open=Callback::new(open_task) />
                        <TodayTaskSection title="Waiting on" tone="blueberry" empty="Nothing is blocked on someone else." items=Signal::derive(move || snapshot.get().waiting_on) on_open=Callback::new(open_task) />

                        <section class="rounded-2xl bg-longan-900 inset-ring inset-ring-white/5 p-4" aria-labelledby="followups-heading">
                            <div class="mb-3 flex items-center justify-between"><h2 id="followups-heading" class="text-h4 text-rambutan-100">"Follow-ups"</h2><span class="size-2 rounded-full bg-durian-400"></span></div>
                            <div class="space-y-1.5">
                                {move || {
                                    let items = snapshot.get().follow_ups;
                                    if items.is_empty() { empty_row("No recent follow-ups.").into_any() } else {
                                        items.into_iter().take(4).map(|item| view! {
                                            <button type="button" class="w-full rounded-grape px-2 py-2 text-left hover:bg-longan-800/60 cursor-pointer" on:click=move |_| set_active_view.set(AppView::People)>
                                                <div class="truncate text-label text-rambutan-200">{item.person_name}</div>
                                                <div class="mt-0.5 line-clamp-2 text-body-sm text-rambutan-500">{item.text}</div>
                                            </button>
                                        }).collect_view().into_any()
                                    }
                                }}
                            </div>
                        </section>

                        <section class="rounded-2xl bg-longan-900 inset-ring inset-ring-white/5 p-4" aria-labelledby="decisions-heading">
                            <div class="mb-3 flex items-center justify-between"><h2 id="decisions-heading" class="text-h4 text-rambutan-100">"Decisions"</h2><span class="size-2 rounded-full bg-pandan-400"></span></div>
                            <div class="space-y-1.5">
                                {move || {
                                    let items = snapshot.get().recent_decisions;
                                    if items.is_empty() { empty_row("No recent meeting decisions.").into_any() } else {
                                        items.into_iter().take(4).map(|item| {
                                            let meeting_id = item.meeting_id.clone();
                                            view! {
                                                <button type="button" class="w-full rounded-grape px-2 py-2 text-left hover:bg-longan-800/60 cursor-pointer" on:click=move |_| set_active_view.set(AppView::MeetingDetail { meeting_id: meeting_id.clone() })>
                                                    <div class="line-clamp-2 text-body-sm text-rambutan-300">{item.text}</div>
                                                    <div class="mt-0.5 truncate text-label-sm text-rambutan-400">{item.meeting_title}</div>
                                                </button>
                                            }
                                        }).collect_view().into_any()
                                    }
                                }}
                            </div>
                        </section>
                    </div>
                </div>
            </div>
        </section>

        <Dialog open=Signal::derive(move || proposal.get().is_some()) on_close=close_plan max_width="max-w-lg" label="Review day plan">
            {move || proposal.get().map(|plan| {
                let unscheduled_count = plan.unscheduled_count;
                let has_unscheduled = unscheduled_count != 0;
                view! {
                <div class="border-b border-longan-800 px-4 py-3">
                    <h2 class="text-h4 text-rambutan-100">"Review day plan"</h2>
                    <p class="mt-1 text-body-sm text-rambutan-500">{format!("{} focus block(s), respecting {} calendar commitment(s)", plan.blocks.len(), plan.busy_blocks_respected)}</p>
                </div>
                <div class="max-h-[55vh] overflow-y-auto p-4 space-y-2">
                    {if plan.blocks.is_empty() {
                        empty_row("No unscheduled ready work fits in the rest of today.").into_any()
                    } else {
                        plan.blocks.into_iter().map(|block| view! {
                            <div class="flex items-center gap-3 rounded-grape bg-longan-800/50 px-3 py-2.5">
                                <span class="w-24 shrink-0 text-label-sm text-persimmon-300">{format!("{}–{}", format_time(block.start_at_ms), format_time(block.end_at_ms))}</span>
                                <span class="min-w-0 flex-1 truncate text-body text-rambutan-200">{block.task_title}</span>
                                <span class="text-label-sm text-rambutan-400">{duration_label(block.start_at_ms, block.end_at_ms)}</span>
                            </div>
                        }).collect_view().into_any()
                    }}
                    <Show when=move || has_unscheduled fallback=|| ()>
                        <p class="pt-1 text-body-sm text-rambutan-400">{format!("{unscheduled_count} ready task(s) stay unscheduled.")}</p>
                    </Show>
                </div>
                <div class="flex items-center justify-end gap-2 border-t border-longan-800 px-4 py-3">
                    <Button variant=ButtonVariant::Ghost size=ButtonSize::Default on_click=Callback::new(move |_| proposal.set(None))>"Cancel"</Button>
                    <Button variant=ButtonVariant::Primary size=ButtonSize::Default disabled=Signal::derive(move || proposal.get().is_none_or(|plan| plan.blocks.is_empty())) on_click=Callback::new(apply_plan)>"Apply plan"</Button>
                </div>
                }
            })}
        </Dialog>
    }
}

#[component]
fn TodayTaskSection(
    title: &'static str,
    tone: &'static str,
    empty: &'static str,
    items: Signal<Vec<WorkItem>>,
    on_open: Callback<String>,
) -> impl IntoView {
    let dot_class = match tone {
        "lychee" => "size-2 rounded-full bg-lychee-400",
        "blueberry" => "size-2 rounded-full bg-blueberry-400",
        _ => "size-2 rounded-full bg-rambutan-500",
    };
    view! {
        <section class="rounded-2xl bg-longan-900 inset-ring inset-ring-white/5 p-4">
            <div class="mb-3 flex items-center justify-between"><h2 class="text-h4 text-rambutan-100">{title}</h2><span class=dot_class></span></div>
            <div class="space-y-1.5">
                {move || {
                    let tasks = items.get();
                    if tasks.is_empty() { empty_row(empty).into_any() } else {
                        tasks.into_iter().take(4).map(|task| {
                            let id = task.id.clone();
                            let detail = if title == "Waiting on" {
                                task.planning.waiting_on.unwrap_or_default()
                            } else {
                                task.due_date.unwrap_or_default()
                            };
                            view! {
                                <button type="button" class="w-full rounded-grape px-2 py-2 text-left hover:bg-longan-800/60 cursor-pointer" on:click=move |_| on_open.run(id.clone())>
                                    <div class="truncate text-label text-rambutan-200">{task.title}</div>
                                    <div class="mt-0.5 truncate text-label-sm text-rambutan-400">{detail}</div>
                                </button>
                            }
                        }).collect_view().into_any()
                    }
                }}
            </div>
        </section>
    }
}
