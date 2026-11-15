//! Month grid — 6 weeks × 7 days. Days from neighboring months are dimmed.
//! Each cell shows the date number and any events that intersect that day
//! as small color-dot pills. Clicking an empty area opens the create
//! dialog with that day pre-selected.
//!
//! Times in this file are local-time. We compute the month grid by walking
//! back from the first-of-month to the previous Sunday, then producing 42
//! consecutive days. Sunday-first matches US conventions; can be made
//! locale-aware later.

use leptos::prelude::*;

use crate::state::{
    AppView, CalendarAnchor, CalendarEvent, CalendarEventsSignal, EventSource, SetActiveView,
    SetEventCreateOpen, SetEventCreateStart, SetSelectedEventId, SetSelectedWorkItem, WorkItem,
    WorkItemsSignal,
};
use crate::ui::Dialog;

#[derive(Clone)]
struct GridCell {
    /// Local-midnight ms of this day.
    day_ms: f64,
    /// 1..31 day-of-month label.
    day_num: u32,
    /// True for days that fall in the anchor month; false for leading/
    /// trailing days from neighboring months.
    in_month: bool,
    /// True for today (host's local date).
    is_today: bool,
}

/// Build 42 consecutive days (6 weeks × 7) anchored on the month containing
/// `anchor_ms`. The first cell is the most recent Sunday on-or-before the
/// 1st of the anchor month.
fn build_grid(anchor_ms: f64) -> Vec<GridCell> {
    let anchor = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(anchor_ms));
    let year = crate::clock::field(anchor.get_full_year());
    let month = crate::clock::field(anchor.get_month());

    // First-of-month at local midnight.
    let first = js_sys::Date::new_with_year_month_day(crate::clock::year_arg(year), month, 1);
    let first_dow = crate::clock::field(first.get_day()); // 0 = Sun
    let first_ms = first.get_time();

    // Walk back to the most recent Sunday.
    let start_ms = crate::clock::add_local_days(first_ms, -first_dow);

    let today = js_sys::Date::new_0();
    let today_y = crate::clock::field(today.get_full_year());
    let today_m = crate::clock::field(today.get_month());
    let today_d = crate::clock::field(today.get_date());
    let today_ms =
        js_sys::Date::new_with_year_month_day(crate::clock::year_arg(today_y), today_m, today_d)
            .get_time();

    (0..42)
        .map(|i| {
            // Calendar steps, not 24-hour ones: a DST change makes one day
            // 23 or 25 hours long.
            let day_ms = crate::clock::add_local_days(start_ms, i);
            let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(day_ms));
            let cell_y = crate::clock::field(d.get_full_year());
            let cell_m = crate::clock::field(d.get_month());
            let cell_d = crate::clock::field(d.get_date());
            GridCell {
                day_ms,
                day_num: cell_d.unsigned_abs(),
                in_month: cell_m == month && cell_y == year,
                is_today: day_ms == today_ms,
            }
        })
        .collect()
}

/// Events that overlap the local day starting at `day_ms`.
fn events_for_day(events: &[CalendarEvent], day_ms: f64) -> Vec<CalendarEvent> {
    let day_end = crate::clock::add_local_days(day_ms, 1);
    events
        .iter()
        .filter(|e| e.start_at_ms < day_end && e.end_at_ms > day_ms)
        .cloned()
        .collect()
}

/// Tasks whose `due_date` (a YYYY-MM-DD local date string) falls on this
/// day. Returned read-only — task markers are surfaced for context but
/// can't be edited from the calendar.
fn tasks_due_on(items: &[WorkItem], day_ms: f64) -> Vec<WorkItem> {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(day_ms));
    let want = format!(
        "{:04}-{:02}-{:02}",
        d.get_full_year(),
        d.get_month() + 1,
        d.get_date()
    );
    items
        .iter()
        .filter(|t| t.due_date.as_deref() == Some(want.as_str()))
        .cloned()
        .collect()
}

const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTHS: [&str; 12] = [
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

/// "Sunday, November 1" and "2026-11-01" for a local-midnight day.
fn day_names(day_ms: f64) -> (String, String) {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(day_ms));
    let weekday = WEEKDAYS
        .get(usize::try_from(d.get_day()).unwrap_or(0))
        .copied()
        .unwrap_or("");
    let month = MONTHS
        .get(usize::try_from(d.get_month()).unwrap_or(0))
        .copied()
        .unwrap_or("");
    (
        format!("{weekday}, {month} {}", d.get_date()),
        format!(
            "{:04}-{:02}-{:02}",
            d.get_full_year(),
            d.get_month() + 1,
            d.get_date()
        ),
    )
}

/// Map a stored color token to its bruhs Tailwind bg class. Unknown tokens
/// fall back to a neutral chrome dot.
fn color_dot_class(color: Option<&str>) -> &'static str {
    match color.unwrap_or("") {
        "persimmon" => "bg-persimmon-400",
        "blueberry" => "bg-blueberry-400",
        "pandan" => "bg-pandan-400",
        "lychee" => "bg-lychee-400",
        "durian" => "bg-durian-400",
        "mangosteen" => "bg-mangosteen-400",
        "dragonfruit" => "bg-dragonfruit-400",
        _ => "bg-rambutan-500",
    }
}

#[component]
pub fn MonthView() -> impl IntoView {
    let CalendarAnchor(anchor) = expect_context::<CalendarAnchor>();
    let CalendarEventsSignal(events) = expect_context::<CalendarEventsSignal>();
    let SetEventCreateOpen(set_event_create_open) = expect_context::<SetEventCreateOpen>();
    let SetEventCreateStart(set_event_create_start) = expect_context::<SetEventCreateStart>();
    let SetSelectedEventId(set_selected_event_id) = expect_context::<SetSelectedEventId>();
    // Read tasks so we can render due-date markers alongside events.
    let WorkItemsSignal(work_items) = expect_context::<WorkItemsSignal>();
    // Optional: clicking a task marker focuses it in the Tasks view; we
    // grab the setter so the click handler can wire it up.
    let SetSelectedWorkItem(set_selected_work_item) = expect_context::<SetSelectedWorkItem>();
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();

    let cells = Signal::derive(move || build_grid(anchor.get()));
    // The day whose full list is open. A cell shows three records at most
    // and clips on compact widths, so the date and "+N more" open every
    // event and due task of that day.
    let day_list = RwSignal::new(None::<f64>);

    view! {
        <div class="flex flex-col flex-1 overflow-hidden">
            // Day-of-week header row.
            <div class="grid grid-cols-7 border-b border-longan-800/40 text-label-sm uppercase tracking-widest text-rambutan-500">
                {["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"].iter().map(|d| view! {
                    <div class="px-2 py-1.5 text-center">{*d}</div>
                }).collect_view()}
            </div>

            // 6×7 grid. Border-collapse via subtle right/bottom borders so
            // cells share lines without doubling-up.
            <div class="grid grid-cols-7 grid-rows-6 flex-1 overflow-hidden">
                {move || {
                    let cells = cells.get();
                    let evs = events.get();
                    let items = work_items.get();
                    cells.into_iter().enumerate().map(|(i, cell)| {
                        let day_events = events_for_day(&evs, cell.day_ms);
                        let day_tasks = tasks_due_on(&items, cell.day_ms);
                        let day_ms = cell.day_ms;
                        let on_click_cell = move |_| {
                            let day = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(day_ms));
                            let start = js_sys::Date::new_with_year_month_day_hr_min_sec(
                                day.get_full_year(), crate::clock::field(day.get_month()), crate::clock::field(day.get_date()),
                                9, 0, 0,
                            ).get_time();
                            set_event_create_start.set(Some(start));
                            set_selected_event_id.set(None);
                            set_event_create_open.set(true);
                        };
                        // Neighboring-month cells: muted text so the user
                        // visually focuses on in-month days.
                        let muted = !cell.in_month;
                        let date_class = if cell.is_today {
                            "inline-flex items-center justify-center size-5 rounded-orb bg-persimmon-500 text-label text-longan-950"
                        } else if muted {
                            "text-label text-rambutan-500"
                        } else {
                            "text-label text-rambutan-300"
                        };
                        // Right/bottom borders only — left and top come from
                        // the previous cell. Last column drops right; last
                        // row drops bottom.
                        let mut classes = vec!["flex min-h-0 flex-col gap-1 overflow-hidden p-1.5 cursor-pointer transition-colors hover:bg-longan-800/20"];
                        if i % 7 != 6 {
                            classes.push("border-r border-longan-800/30");
                        }
                        if i / 7 != 5 {
                            classes.push("border-b border-longan-800/30");
                        }
                        let class_str = classes.join(" ");
                        // Cap visible rows at 3 across both events + task
                        // markers so the grid stays uncluttered.
                        let total = day_events.len() + day_tasks.len();
                        let event_take = day_events.len().min(3);
                        let task_take = (3usize.saturating_sub(event_take)).min(day_tasks.len());
                        let extra = total.saturating_sub(event_take + task_take);
                        let (day_label, day_iso) = day_names(day_ms);
                        let open_day = move |ev: web_sys::MouseEvent| {
                            ev.stop_propagation();
                            day_list.set(Some(day_ms));
                        };
                        let noun = if total == 1 { "record" } else { "records" };
                        let date_view = if total > 0 {
                            view! {
                                <button
                                    type="button"
                                    on:click=open_day
                                    aria-label=format!("Show {total} {noun} on {day_label}")
                                    data-calendar-day=day_iso.clone()
                                    class=format!("{date_class} shrink-0 self-start cursor-pointer rounded-seed hover:text-rambutan-100 focus-visible:outline-2 focus-visible:outline-persimmon-400")
                                >
                                    {cell.day_num}
                                </button>
                            }.into_any()
                        } else {
                            view! { <div class=format!("{date_class} shrink-0 self-start")>{cell.day_num}</div> }.into_any()
                        };
                        view! {
                            <div class=class_str on:click=on_click_cell>
                                {date_view}
                                {
                                    let event_pills = day_events.into_iter().take(event_take).map(|e| {
                                        let dot = color_dot_class(e.color.as_deref());
                                        let id = e.id.clone();
                                        let title = e.title.clone();
                                        let on_pill_click = move |ev: web_sys::MouseEvent| {
                                            // Don't bubble to the cell's
                                            // create handler.
                                            ev.stop_propagation();
                                            set_selected_event_id.set(Some(id.clone()));
                                            set_event_create_open.set(true);
                                        };
                                        let is_google = matches!(e.source, EventSource::Google);
                                        let pill_label = if is_google {
                                            format!("Open Google event {title}, read only")
                                        } else {
                                            format!("Edit event {title}")
                                        };
                                        let data_event_id = e.id.clone();
                                        view! {
                                            <button
                                                type="button"
                                                aria-label=pill_label
                                                data-event-id=data_event_id
                                                on:click=on_pill_click
                                                class="flex shrink-0 items-center gap-1 truncate text-label-sm text-rambutan-400 hover:text-rambutan-200 transition-colors text-left cursor-pointer"
                                            >
                                                <span class=format!("size-1.5 rounded-orb shrink-0 {dot}")></span>
                                                {if is_google {
                                                    view! {
                                                        <span title="Synced from Google Calendar" class="text-label-sm text-blueberry-300/80 bg-blueberry-500/15 inset-ring inset-ring-blueberry-500/40 rounded-seed px-0.5 leading-none shrink-0">"G"</span>
                                                    }.into_any()
                                                } else {
                                                    ().into_any()
                                                }}
                                                <span class="truncate">{title}</span>
                                            </button>
                                        }
                                    }).collect_view();
                                    let task_pills = day_tasks.into_iter().take(task_take).map(|t| {
                                        // Task markers are visually distinct
                                        // from events: open ring (not filled
                                        // dot) + muted color so the eye
                                        // separates them at a glance.
                                        let id = t.id.clone();
                                        let data_task_id = t.id.clone();
                                        let title = t.title;
                                        let task_label = format!("Open task {title}, due this day");
                                        let on_task_click = move |ev: web_sys::MouseEvent| {
                                            ev.stop_propagation();
                                            // Open the linked task, as Search
                                            // and Today do.
                                            set_selected_work_item.set(Some(id.clone()));
                                            set_active_view.set(AppView::Tasks);
                                            super::record_focus::focus_record("task-detail-title".into());
                                        };
                                        view! {
                                            <button
                                                type="button"
                                                on:click=on_task_click
                                                title="Task due"
                                                aria-label=task_label
                                                data-calendar-task=data_task_id
                                                class="flex shrink-0 items-center gap-1 truncate text-label-sm text-rambutan-500 italic hover:text-rambutan-300 transition-colors text-left cursor-pointer"
                                            >
                                                <span class="size-1.5 rounded-orb shrink-0 inset-ring inset-ring-rambutan-500"></span>
                                                <span class="truncate">{title}</span>
                                            </button>
                                        }
                                    }).collect_view();
                                    view! {
                                        <>
                                            {event_pills}
                                            {task_pills}
                                            {(extra > 0).then(|| view! {
                                                <button
                                                    type="button"
                                                    on:click=open_day
                                                    data-calendar-more=day_iso.clone()
                                                    class="shrink-0 self-start rounded-seed text-left text-label-sm text-rambutan-500 hover:text-rambutan-300 cursor-pointer focus-visible:outline-2 focus-visible:outline-persimmon-400"
                                                >
                                                    {format!("+{extra} more")}
                                                </button>
                                            })}
                                        </>
                                    }
                                }
                            </div>
                        }
                    }).collect_view()
                }}
            </div>
            <Dialog
                open=Signal::derive(move || day_list.get().is_some())
                on_close=Callback::new(move |_| day_list.set(None))
                max_width="max-w-sm".to_string()
                label="Calendar day"
                padding_top="1rem"
            >
                {move || day_list.get().map(|day_ms| {
                    let (day_label, _) = day_names(day_ms);
                    let day_events = events.with(|evs| events_for_day(evs, day_ms));
                    let day_tasks = work_items.with(|items| tasks_due_on(items, day_ms));
                    let event_rows = day_events.into_iter().map(|e| {
                        let id = e.id.clone();
                        let is_google = matches!(e.source, EventSource::Google);
                        let title = e.title.clone();
                        let label = if is_google {
                            format!("Open Google event {title}, read only")
                        } else {
                            format!("Edit event {title}")
                        };
                        let dot = color_dot_class(e.color.as_deref());
                        let open_event = move |_| {
                            day_list.set(None);
                            set_selected_event_id.set(Some(id.clone()));
                            set_event_create_open.set(true);
                        };
                        view! {
                            <li>
                                <button
                                    type="button"
                                    on:click=open_event
                                    aria-label=label
                                    data-day-list-event=e.id
                                    class="bruhs-control flex min-h-[32px] w-full items-center gap-2 rounded-grape px-2 text-left text-label-sm text-rambutan-300 hover:bg-longan-700 hover:text-rambutan-100 cursor-pointer"
                                >
                                    <span class=format!("size-1.5 rounded-orb shrink-0 {dot}")></span>
                                    {is_google.then(|| view! {
                                        <span title="Synced from Google Calendar" class="text-label-sm text-blueberry-300/80 bg-blueberry-500/15 inset-ring inset-ring-blueberry-500/40 rounded-seed px-0.5 leading-none shrink-0">"G"</span>
                                    })}
                                    <span class="truncate">{title}</span>
                                </button>
                            </li>
                        }
                    }).collect_view();
                    let task_rows = day_tasks.into_iter().map(|t| {
                        let id = t.id.clone();
                        let title = t.title;
                        let open_task = move |_| {
                            day_list.set(None);
                            set_selected_work_item.set(Some(id.clone()));
                            set_active_view.set(AppView::Tasks);
                            super::record_focus::focus_record("task-detail-title".into());
                        };
                        view! {
                            <li>
                                <button
                                    type="button"
                                    on:click=open_task
                                    title="Task due"
                                    aria-label=format!("Open task {title}, due this day")
                                    data-day-list-task=t.id
                                    class="bruhs-control flex min-h-[32px] w-full items-center gap-2 rounded-grape px-2 text-left text-label-sm italic text-rambutan-300 hover:bg-longan-700 hover:text-rambutan-100 cursor-pointer"
                                >
                                    <span class="size-1.5 rounded-orb shrink-0 inset-ring inset-ring-rambutan-300"></span>
                                    <span class="truncate">{title}</span>
                                </button>
                            </li>
                        }
                    }).collect_view();
                    view! {
                        <div class="border-b border-longan-800 px-4 py-3">
                            <h2 class="text-h4 text-rambutan-100">{day_label}</h2>
                        </div>
                        <ul class="max-h-[calc(100dvh-10rem)] space-y-0.5 overflow-y-auto px-2 py-2">
                            {event_rows}
                            {task_rows}
                        </ul>
                    }
                })}
            </Dialog>
        </div>
    }
}
