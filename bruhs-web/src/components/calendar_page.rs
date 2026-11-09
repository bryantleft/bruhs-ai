//! Top-level Calendar surface — header (range title + Today + nav + mode
//! switch + + New) above a body that swaps between Month, Week, and Day
//! grids. The mode switch toggles the grain; the prev/next buttons step
//! by 1 unit of that grain.

use leptos::prelude::*;

use crate::components::event_create_dialog::EventCreateDialog;
use crate::components::month_view::MonthView;
use crate::components::smart_schedule_panel::SmartSchedulePanel;
use crate::components::timegrid_view::TimegridView;
use crate::state::{
    CalendarAnchor, CalendarMode, CalendarModeSignal, SchedulingOpenRequest, SetCalendarAnchor,
    SetCalendarModeSignal, SetEventCreateOpen, SetEventCreateStart,
};

/// Shift `anchor_ms` by `delta_months` (signed). Keeps day-of-month stable
/// when the target month has enough days; otherwise clamps.
fn shift_month(anchor_ms: f64, delta_months: i32) -> f64 {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(anchor_ms));
    let year = crate::clock::field(date.get_full_year());
    let month = crate::clock::field(date.get_month());
    let day = date.get_date();

    let total = year * 12 + month + delta_months;
    let new_year = total.div_euclid(12);
    let new_month = total.rem_euclid(12);

    // `new_month` is 0-based (JS `Date`); the shared helper is 1-based.
    // `rem_euclid(12)` keeps it in range, so the fallback never applies.
    let last_day =
        bruhs_models::time::days_in_month(new_year, new_month.unsigned_abs() + 1).unwrap_or(28);
    let clamped_day = day.min(last_day);

    js_sys::Date::new_with_year_month_day(
        crate::clock::year_arg(new_year),
        new_month,
        crate::clock::field(clamped_day),
    )
    .get_time()
}

fn format_month_year(anchor_ms: f64) -> String {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(anchor_ms));
    let names = [
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
    let m = date.get_month() as usize;
    let y = crate::clock::field(date.get_full_year());
    format!("{} {y}", names.get(m).copied().unwrap_or(""))
}

/// "May 3, 2026" — used as the Day-view title.
fn format_full_date(anchor_ms: f64) -> String {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(anchor_ms));
    let names = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    format!(
        "{} {}, {}",
        names[d.get_month() as usize],
        d.get_date(),
        d.get_full_year()
    )
}

/// "May 3 — 9, 2026" or "Apr 28 — May 4, 2026" — used as the Week-view title.
fn format_week_range(anchor_ms: f64) -> String {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(anchor_ms));
    let start = crate::clock::add_local_days(anchor_ms, -crate::clock::field(d.get_day()));
    let end = crate::clock::add_local_days(start, 6);

    let s = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(start));
    let e = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(end));
    let names = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];

    if s.get_month() == e.get_month() {
        format!(
            "{} {} — {}, {}",
            names[s.get_month() as usize],
            s.get_date(),
            e.get_date(),
            e.get_full_year()
        )
    } else {
        format!(
            "{} {} — {} {}, {}",
            names[s.get_month() as usize],
            s.get_date(),
            names[e.get_month() as usize],
            e.get_date(),
            e.get_full_year()
        )
    }
}

#[component]
pub fn CalendarPage() -> impl IntoView {
    let executor = expect_context::<crate::workflow_executor::WorkflowExecutor>();
    let scheduling_open = RwSignal::new(false);
    let SchedulingOpenRequest(scheduling_open_request) = expect_context::<SchedulingOpenRequest>();
    Effect::new(move |_| {
        if scheduling_open_request.take() {
            scheduling_open.set(true);
        }
    });
    let CalendarAnchor(anchor) = expect_context::<CalendarAnchor>();
    let SetCalendarAnchor(set_anchor) = expect_context::<SetCalendarAnchor>();
    let CalendarModeSignal(mode) = expect_context::<CalendarModeSignal>();
    let SetCalendarModeSignal(set_mode) = expect_context::<SetCalendarModeSignal>();
    let SetEventCreateOpen(set_event_create_open) = expect_context::<SetEventCreateOpen>();
    let SetEventCreateStart(set_event_create_start) = expect_context::<SetEventCreateStart>();

    // Step-by-mode: a click on ‹ / › advances by 1 unit of the current
    // grain. Month-grain uses calendar-month math; week/day are simple
    // ms arithmetic.
    let on_prev = move |_| {
        let m = mode.get();
        set_anchor.update(|a| {
            *a = match m {
                CalendarMode::Month => shift_month(*a, -1),
                CalendarMode::Week => crate::clock::add_local_days(*a, -7),
                CalendarMode::Day => crate::clock::add_local_days(*a, -1),
            }
        });
    };
    let on_next = move |_| {
        let m = mode.get();
        set_anchor.update(|a| {
            *a = match m {
                CalendarMode::Month => shift_month(*a, 1),
                CalendarMode::Week => crate::clock::add_local_days(*a, 7),
                CalendarMode::Day => crate::clock::add_local_days(*a, 1),
            }
        });
    };
    let on_today = move |_| set_anchor.set(js_sys::Date::now());
    let on_new = move |_| {
        set_event_create_start.set(None);
        set_event_create_open.set(true);
    };

    let title = move || match mode.get() {
        CalendarMode::Month => format_month_year(anchor.get()),
        CalendarMode::Week => format_week_range(anchor.get()),
        CalendarMode::Day => format_full_date(anchor.get()),
    };

    // Mode pill: persimmon-soft when active; ghost otherwise.
    let mode_btn_class = |is_active: bool| {
        if is_active {
            "rounded-grape bg-persimmon-500/15 text-persimmon-200 inset-ring inset-ring-persimmon-400/20 px-2 py-0.5 text-label cursor-pointer"
        } else {
            "rounded-grape px-2 py-0.5 text-label text-rambutan-500 hover:bg-longan-800/40 hover:text-rambutan-200 cursor-pointer"
        }
    };

    view! {
        <div class="flex flex-1 h-full overflow-hidden">
            <div class="flex min-w-0 flex-1 flex-col overflow-hidden">
                // Header — slim, mirrors tasks_page header proportions.
                <div class="flex flex-wrap items-center justify-between gap-2 border-b border-longan-800/40 px-4 py-2 shrink-0">
                    <div class="flex min-w-0 flex-wrap items-center gap-3">
                        <h1 class="text-body-sm tracking-wide text-rambutan-400">
                            {title}
                        </h1>
                        <div class="flex items-center gap-1">
                            <button
                                type="button"
                                on:click=on_prev
                                aria-label="Previous"
                                class="inline-flex items-center justify-center size-6 rounded-grape text-body-sm text-rambutan-500 hover:bg-longan-800/40 hover:text-rambutan-300 cursor-pointer"
                            >
                                "‹"
                            </button>
                            <button
                                type="button"
                                on:click=on_today
                                class="rounded-grape px-2 py-0.5 text-label text-rambutan-400 hover:bg-longan-800/40 hover:text-rambutan-200 cursor-pointer"
                            >
                                "Today"
                            </button>
                            <button
                                type="button"
                                on:click=on_next
                                aria-label="Next"
                                class="inline-flex items-center justify-center size-6 rounded-grape text-body-sm text-rambutan-500 hover:bg-longan-800/40 hover:text-rambutan-300 cursor-pointer"
                            >
                                "›"
                            </button>
                        </div>
                    </div>

                    // Mode switcher + New on the right.
                    <div class="flex flex-wrap items-center gap-2">
                        <button type="button" on:click=move |_| scheduling_open.set(true) class="rounded-grape px-2.5 py-1 text-body-sm text-blueberry-300 inset-ring inset-ring-blueberry-500/20 hover:bg-blueberry-500/10 cursor-pointer">"Schedule"</button>
                        <div class="flex items-center gap-0.5 rounded-grape bg-longan-900/40 inset-ring inset-ring-longan-800/60 p-0.5">
                            <button
                                type="button"
                                on:click=move |_| set_mode.set(CalendarMode::Day)
                                class=move || mode_btn_class(matches!(mode.get(), CalendarMode::Day))
                            >
                                "Day"
                            </button>
                            <button
                                type="button"
                                on:click=move |_| set_mode.set(CalendarMode::Week)
                                class=move || mode_btn_class(matches!(mode.get(), CalendarMode::Week))
                            >
                                "Week"
                            </button>
                            <button
                                type="button"
                                on:click=move |_| set_mode.set(CalendarMode::Month)
                                class=move || mode_btn_class(matches!(mode.get(), CalendarMode::Month))
                            >
                                "Month"
                            </button>
                        </div>
                        <button
                            type="button"
                            on:click=on_new
                            title="New event"
                            class="rounded-grape bg-persimmon-500/15 text-persimmon-200 inset-ring inset-ring-persimmon-400/20 px-2.5 py-1 text-body-sm hover:bg-persimmon-500/25 cursor-pointer"
                        >
                            "+ New"
                        </button>
                    </div>
                </div>

                <Show when=move || executor.load_error.get().is_some()>
                    <section aria-label="Calendar loading status" class="shrink-0 border-b border-longan-800 px-4 py-3 text-body-sm text-rambutan-300">
                        <p role="alert">"Local records could not be loaded. Existing storage was left unchanged; event changes made here are kept."</p>
                        <button type="button" class="mt-2 min-h-11 rounded-grape bg-longan-800 px-3 py-2 text-label text-rambutan-200 hover:bg-longan-700 disabled:opacity-40 cursor-pointer"
                            disabled=move || executor.loading.get()
                            on:click=move |_| executor.load()>"Retry loading records"</button>
                    </section>
                </Show>

                // Body switches by mode. Each branch is its own view tree;
                // we don't need shared state across them since the anchor +
                // events are read from context.
                <div class="flex flex-1 overflow-hidden">
                    {move || match mode.get() {
                        CalendarMode::Month => view! { <MonthView /> }.into_any(),
                        CalendarMode::Week => view! { <TimegridView columns=7 /> }.into_any(),
                        CalendarMode::Day => view! { <TimegridView columns=1 /> }.into_any(),
                    }}
                </div>
            </div>

            <EventCreateDialog />
            <SmartSchedulePanel open=scheduling_open />
        </div>
    }
}
