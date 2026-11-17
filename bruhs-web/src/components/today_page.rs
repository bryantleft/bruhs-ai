use crate::task_style::PriorityStyle;
use bruhs_models::num::ToIntSaturating;
use leptos::prelude::*;

use crate::state::{
    AppView, AutomationStateSignal, CalendarEventsSignal, CalendarMode, MeetingsSignal,
    PeopleSignal, PlanDayRequest, SelectedPersonSignal, SetActiveView, SetCalendarAnchor,
    SetCalendarModeSignal, SetSelectedEventId, SetSelectedWorkItem, TodaySnapshot, WorkItem,
    WorkItemsSignal,
};
use crate::task_saves::TaskSaves;
use crate::ui::{Button, ButtonSize, ButtonVariant, Dialog};
use bruhs_models::{propose_day_plan_in_window, today_copy, DayPlanProposal};

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

/// A follow-up opens its person's profile on People, the way search
/// results and whiteboard links open one.
fn open_person(
    person_id: String,
    selected_person: RwSignal<Option<String>>,
    set_active_view: WriteSignal<AppView>,
) {
    selected_person.set(Some(person_id));
    set_active_view.set(AppView::People);
}

/// A card's empty state: a quiet line of text, not a box inside the card.
fn empty_row(label: &'static str) -> impl IntoView {
    view! { <p class="py-1 text-body-sm text-rambutan-500">{label}</p> }
}

fn install_clock(now_ms: RwSignal<f64>) {
    use wasm_bindgen::JsCast as _;
    let tick = move || {
        let _ = now_ms.try_set(js_sys::Date::now());
    };
    let timer = StoredValue::new_local(Some(gloo_timers::callback::Interval::new(30_000, tick)));
    let focus =
        leptos::leptos_dom::helpers::window_event_listener(leptos::ev::focus, move |_| tick());
    let pageshow =
        leptos::leptos_dom::helpers::window_event_listener(leptos::ev::pageshow, move |_| tick());
    let visibility = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| {
            let callback =
                wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(move |_| tick());
            document
                .add_event_listener_with_callback(
                    "visibilitychange",
                    callback.as_ref().unchecked_ref(),
                )
                .ok()?;
            Some((document, callback))
        });
    let visibility = StoredValue::new_local(visibility);
    on_cleanup(move || {
        timer.try_update_value(|timer| {
            timer.take();
        });
        focus.remove();
        pageshow.remove();
        visibility.try_update_value(|listener| {
            if let Some((document, callback)) = listener.take() {
                let _ = document.remove_event_listener_with_callback(
                    "visibilitychange",
                    callback.as_ref().unchecked_ref(),
                );
            }
        });
    });
}

#[component]
pub fn TodayPage() -> impl IntoView {
    let WorkItemsSignal(work_items) = expect_context::<WorkItemsSignal>();
    let CalendarEventsSignal(events) = expect_context::<CalendarEventsSignal>();
    let MeetingsSignal(meetings) = expect_context::<MeetingsSignal>();
    let PeopleSignal(people) = expect_context::<PeopleSignal>();
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();
    let SelectedPersonSignal(selected_person) = expect_context::<SelectedPersonSignal>();
    let SetSelectedWorkItem(set_selected_work_item) = expect_context::<SetSelectedWorkItem>();
    let SetSelectedEventId(set_selected_event_id) = expect_context::<SetSelectedEventId>();
    let SetCalendarAnchor(set_calendar_anchor) = expect_context::<SetCalendarAnchor>();
    let SetCalendarModeSignal(set_calendar_mode) = expect_context::<SetCalendarModeSignal>();
    let PlanDayRequest(plan_day_request) = expect_context::<PlanDayRequest>();
    let AutomationStateSignal(automation) = expect_context::<AutomationStateSignal>();

    let now_ms = RwSignal::new(js_sys::Date::now());
    install_clock(now_ms);
    let proposal = RwSignal::new(Option::<DayPlanProposal>::None);
    let reviewed_window = RwSignal::new(None::<bruhs_models::DayPlanWindow>);
    let saves = expect_context::<TaskSaves>();
    let planning = saves.day_plan;
    let persistence_error = Signal::derive(move || planning.with(|state| state.error.clone()));
    let applied_plan = Signal::derive(move || planning.with(|state| state.receipt.clone()));
    let date_label = Memo::new(move |_| greeting_date(now_ms.get()));
    Effect::new(move |_| {
        let window = crate::clock::workday_window(now_ms.get());
        if proposal.with_untracked(Option::is_some)
            && reviewed_window
                .get_untracked()
                .is_some_and(|reviewed| reviewed != window)
        {
            proposal.set(None);
            reviewed_window.set(None);
            planning.update(|state| {
                state.error =
                    Some("The local date or timezone changed. Review a new day plan.".into())
            });
        }
    });

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
        // Shared with desktop and Agent workflows: written only when added.
        automation.update(|state| {
            state.offer_focus_suggestion(unfinished, event_count, crate::clock::now_ms());
        });
    });

    let snapshot = Memo::new(move |_| {
        let now = now_ms.get();
        let (day_start, day_end) = local_day_bounds(now);
        TodaySnapshot::build(
            now,
            day_start,
            day_end,
            &local_date_key(now),
            &work_items.read(),
            &events.read(),
            &meetings.read(),
            people.read().iter(),
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
    let draft = move || {
        let now = js_sys::Date::now();
        now_ms.set(now);
        let window = crate::clock::workday_window(now);
        match propose_day_plan_in_window(
            now,
            window,
            &work_items.get_untracked(),
            &events.get_untracked(),
        ) {
            Ok(plan) => {
                reviewed_window.set(Some(window));
                proposal.set(Some(plan));
            }
            Err(error) => planning.update(|state| state.error = Some(error.to_string())),
        }
    };
    let draft_plan = move |_| draft();
    Effect::new(move |_| {
        if plan_day_request.take() {
            draft();
        }
    });
    let close_plan = Callback::new(move |_| proposal.set(None));
    let apply_plan = move |_| {
        let (Some(plan), Some(window)) =
            (proposal.get_untracked(), reviewed_window.get_untracked())
        else {
            return;
        };
        if saves.apply_day_plan(plan, window) {
            proposal.set(None);
        }
    };
    let undo_plan = move |_| saves.undo_day_plan();

    view! {
        <section class="flex flex-1 min-h-0 min-w-0 flex-col bg-longan-950" aria-labelledby="today-title">
            <header class="flex flex-wrap shrink-0 items-center justify-between gap-3 border-b border-longan-800/50 px-4 py-3 sm:px-6">
                <div class="min-w-0">
                    <h1 id="today-title" class="text-h3 text-rambutan-100">"Today"</h1>
                    <p class="mt-0.5 text-body-sm text-rambutan-500">{move || date_label.get()}</p>
                </div>
                <div class="flex flex-wrap items-center gap-2">
                    <Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| set_active_view.set(AppView::Automations))>
                        {today_copy::AGENT_WORKFLOWS}
                    </Button>
                    <Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| set_active_view.set(AppView::Life))>
                        {today_copy::LIFE}
                    </Button>
                    <Button variant=ButtonVariant::Primary size=ButtonSize::Small disabled=Signal::derive(move || planning.with(|state| state.busy)) on_click=Callback::new(draft_plan)>
                        "Plan my day"
                    </Button>
                </div>
            </header>

            <div class="flex-1 overflow-y-auto px-4 py-4 sm:px-6 sm:py-5">
                <div class="mx-auto max-w-6xl space-y-4">
                    <Show when=move || persistence_error.get().is_some() && proposal.get().is_none() fallback=|| ()>
                        <div role="alert" class="flex items-center justify-between gap-3 rounded-grape bg-lychee-500/10 inset-ring inset-ring-lychee-400/20 px-3 py-2 text-body-sm text-lychee-200">
                            <span>{move || persistence_error.get().unwrap_or_default()}</span>
                            <button class="text-button hover:text-lychee-100 cursor-pointer" on:click=move |_| planning.update(|state| state.error = None)>"Dismiss"</button>
                        </div>
                    </Show>
                    <Show when=move || persistence_error.get().is_some() && proposal.get().is_none() && !planning.with(|state| state.busy) fallback=|| ()>
                        <button class="bruhs-control rounded-seed px-2 py-2 text-button text-persimmon-300 focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400" on:click=move |_| saves.refresh_day_plan_sources()>"Refresh saved tasks and calendar"</button>
                    </Show>
                    <Show when=move || planning.with(|state| state.busy) fallback=|| ()>
                        <p role="status" class="text-body-sm text-rambutan-400">"Saving day plan…"</p>
                    </Show>
                    <Show when=move || !planning.with(|state| state.loaded || state.busy) fallback=|| ()>
                        <button class="bruhs-control text-button text-persimmon-300" on:click=move |_| saves.load_day_plan()>"Retry day-plan recovery"</button>
                    </Show>
                    <Show when=move || applied_plan.get().is_some() fallback=|| ()>
                        <div role="status" class="flex items-center justify-between gap-3 rounded-grape bg-pandan-500/10 inset-ring inset-ring-pandan-400/20 px-3 py-2 text-body-sm text-pandan-200">
                            <span>"Day plan applied."</span>
                            <Show
                                when=move || !planning.with(|state| state.busy)
                                fallback=|| view! { <span class="text-label-sm text-pandan-300/70">"Saving…"</span> }
                            >
                                <button class="bruhs-control min-h-[32px] rounded-seed px-2 py-1 text-button text-pandan-100 hover:text-white focus-visible:outline focus-visible:outline-2 focus-visible:outline-persimmon-400 cursor-pointer" on:click=undo_plan>"Undo"</button>
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
                                    <p class="mt-1 text-label-sm text-blueberry-300">{format!("Evidence: {}", suggestion.evidence)}</p>
                                </div>
                                <div class="flex items-center gap-2">
                                    <button type="button" class="bruhs-control text-button text-rambutan-500 hover:text-rambutan-200 cursor-pointer" aria-label="Dismiss proactive suggestion" on:click=move |_| automation.update(|state| { let _ = state.dismiss_suggestion(&id, crate::clock::now_ms()); })>"Dismiss"</button>
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
                                <button class="bruhs-control text-button text-rambutan-500 hover:text-rambutan-200 cursor-pointer" on:click=move |_| set_active_view.set(AppView::Calendar)>"Calendar →"</button>
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
                                        empty_row(today_copy::SCHEDULE_EMPTY).into_any()
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
                                <span class="text-label-sm text-rambutan-400">{move || today_copy::ready(snapshot.get().focus_queue.len())}</span>
                            </div>
                            <div class="space-y-1">
                                {move || {
                                    let queue = snapshot.get().focus_queue;
                                    if queue.is_empty() {
                                        empty_row(today_copy::FOCUS_QUEUE_EMPTY).into_any()
                                    } else {
                                        queue.into_iter().enumerate().map(|(index, task)| {
                                            let task_id = task.id.clone();
                                            let estimate = today_copy::estimate(task.planning.estimate_minutes);
                                            let label = today_copy::focus_row_label(&task.title, task.priority.label(), &estimate);
                                            view! {
                                                <button type="button" aria-label=label class="group flex w-full items-center gap-3 rounded-grape px-2.5 py-2 text-left hover:bg-longan-800/60 cursor-pointer" on:click=move |_| open_task(task_id.clone())>
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
                        <TodayTaskSection title="Overdue" tone="lychee" empty=today_copy::OVERDUE_EMPTY items=Signal::derive(move || snapshot.get().overdue) on_open=Callback::new(open_task) />
                        <TodayTaskSection title="Waiting on" tone="blueberry" empty=today_copy::WAITING_ON_EMPTY items=Signal::derive(move || snapshot.get().waiting_on) on_open=Callback::new(open_task) />

                        <section class="rounded-2xl bg-longan-900 inset-ring inset-ring-white/5 p-4" aria-labelledby="followups-heading">
                            <div class="mb-3 flex items-center justify-between"><h2 id="followups-heading" class="text-h4 text-rambutan-100">"Follow ups"</h2><span class="size-2 rounded-full bg-durian-400"></span></div>
                            <div class="space-y-1.5">
                                {move || {
                                    let items = snapshot.get().follow_ups;
                                    if items.is_empty() { empty_row(today_copy::FOLLOW_UPS_EMPTY).into_any() } else {
                                        items.into_iter().take(4).map(|item| {
                                            let person_id = item.person_id.clone();
                                            view! {
                                                <button type="button" class="w-full rounded-grape px-2 py-2 text-left hover:bg-longan-800/60 cursor-pointer" on:click=move |_| open_person(person_id.clone(), selected_person, set_active_view)>
                                                    <div class="truncate text-label text-rambutan-200">{item.person_name}</div>
                                                    <div class="mt-0.5 line-clamp-2 text-body-sm text-rambutan-500">{item.text}</div>
                                                </button>
                                            }
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
                                    if items.is_empty() { empty_row(today_copy::DECISIONS_EMPTY).into_any() } else {
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
                <Show when=move || persistence_error.get().is_some() fallback=|| ()>
                    <p role="alert" class="px-4 pt-3 text-body-sm text-lychee-200">{move || persistence_error.get().unwrap_or_default()}</p>
                </Show>
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
                                <span class="w-24 shrink-0 text-label-sm text-persimmon-300">{format!("{} to {}", format_time(block.start_at_ms), format_time(block.end_at_ms))}</span>
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
                    <Button variant=ButtonVariant::Primary size=ButtonSize::Default disabled=Signal::derive(move || proposal.get().is_none_or(|plan| plan.blocks.is_empty()) || planning.with(|state| state.busy)) on_click=Callback::new(apply_plan)>"Apply plan"</Button>
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
                            let row_label = today_copy::task_row_label(&task.title);
                            let detail = if title == "Waiting on" {
                                task.planning.waiting_on.unwrap_or_default()
                            } else {
                                task.due_date.unwrap_or_default()
                            };
                            view! {
                                <button type="button" aria-label=row_label class="w-full rounded-grape px-2 py-2 text-left hover:bg-longan-800/60 cursor-pointer" on:click=move |_| on_open.run(id.clone())>
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

#[cfg(test)]
mod tests {
    use super::open_person;
    use crate::state::AppView;
    use leptos::prelude::{GetUntracked, Owner, RwSignal};

    #[test]
    fn a_follow_up_opens_its_persons_profile_like_desktop() {
        let owner = Owner::new();
        owner.with(|| {
            let selected = RwSignal::new(None::<String>);
            let view = RwSignal::new(AppView::Today);
            open_person("ada".into(), selected, view.write_only());
            assert_eq!(selected.get_untracked().as_deref(), Some("ada"));
            assert_eq!(view.get_untracked(), AppView::People);
        });
    }
}
