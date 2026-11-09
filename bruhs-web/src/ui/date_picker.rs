//! Custom calendar date picker — a button that opens a month-grid popover,
//! replacing the browser-native `<input type="date">` UI. Value is an ISO
//! `YYYY-MM-DD` string (same wire format the native input used), so callers
//! don't have to change how they store dates.

use bruhs_models::time::days_in_month;
use leptos::prelude::*;

pub(crate) const MONTHS: [&str; 12] = [
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
pub(crate) const WEEKDAYS: [&str; 7] = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];

/// Parse `YYYY-MM-DD` into `(year, month 1-12, day)`.
pub(crate) fn parse_iso(s: &str) -> Option<(i32, u32, u32)> {
    let mut parts = s.split('-');
    let y = parts.next()?.parse::<i32>().ok()?;
    let m = parts.next()?.parse::<u32>().ok()?;
    let d = parts.next()?.parse::<u32>().ok()?;
    if (1..=12).contains(&m) && (1..=31).contains(&d) {
        Some((y, m, d))
    } else {
        None
    }
}

/// Weekday (0=Sun..6=Sat) of the 1st of `month` (1-12) in `year`.
pub(crate) fn first_weekday(year: i32, month: u32) -> u32 {
    let d = js_sys::Date::new_with_year_month_day(
        crate::clock::year_arg(year),
        crate::clock::field(month) - 1,
        1,
    );
    d.get_day()
}

#[component]
pub fn DatePicker(
    /// ISO `YYYY-MM-DD` value (may be empty).
    #[prop(optional, into)]
    value: Signal<String>,
    /// Fired with the chosen ISO `YYYY-MM-DD` string.
    #[prop(optional)]
    on_change: Option<Callback<String>>,
    #[prop(optional, into)] class: String,
) -> impl IntoView {
    let open = RwSignal::new(false);

    // Initial calendar view: the value's month, else today.
    let (init_year, init_month) = parse_iso(&value.get_untracked()).map_or_else(
        || {
            let now = js_sys::Date::new_0();
            (
                crate::clock::field(now.get_full_year()),
                now.get_month() + 1,
            )
        },
        |(y, m, _)| (y, m),
    );
    let view_year = RwSignal::new(init_year);
    let view_month = RwSignal::new(init_month);

    let go_prev = move |_| {
        let (mut y, mut m) = (view_year.get(), view_month.get());
        if m == 1 {
            m = 12;
            y -= 1;
        } else {
            m -= 1;
        }
        view_year.set(y);
        view_month.set(m);
    };
    let go_next = move |_| {
        let (mut y, mut m) = (view_year.get(), view_month.get());
        if m == 12 {
            m = 1;
            y += 1;
        } else {
            m += 1;
        }
        view_year.set(y);
        view_month.set(m);
    };

    let trigger_label = move || {
        let v = value.get();
        if v.is_empty() {
            "Select date".to_string()
        } else {
            v
        }
    };

    let final_class = format!(
        "inline-flex items-center gap-2 rounded-grape bg-white/5 px-3 py-2 text-body text-rambutan-100 inset-ring inset-ring-white/10 hover:inset-ring-white/20 focus-visible:outline-2 -outline-offset-1 focus-visible:outline-persimmon-400 cursor-pointer {class}"
    );

    // The day grid for the current view month (reactive on view_year/month).
    let grid = move || {
        let (y, m) = (view_year.get(), view_month.get());
        let lead = first_weekday(y, m) as usize;
        // An out-of-range month (never produced by the month stepper) shows
        // no days rather than a guess.
        let total = days_in_month(y, m).unwrap_or(0);
        let selected = parse_iso(&value.get());
        let mut cells: Vec<Option<u32>> = vec![None; lead];
        cells.extend((1..=total).map(Some));
        cells
            .into_iter()
            .map(|cell| match cell {
                None => view! { <div></div> }.into_any(),
                Some(day) => {
                    let is_selected = selected == Some((y, m, day));
                    let day_class = if is_selected {
                        "size-8 rounded-seed text-label bg-persimmon-500 text-longan-950"
                    } else {
                        "size-8 rounded-seed text-label text-rambutan-200 hover:bg-longan-800 transition-colors"
                    };
                    let pick = move |_| {
                        let iso = format!("{y:04}-{m:02}-{day:02}");
                        if let Some(cb) = on_change {
                            cb.run(iso);
                        }
                        open.set(false);
                    };
                    view! {
                        <button type="button" class=day_class on:click=pick>
                            {day.to_string()}
                        </button>
                    }
                    .into_any()
                }
            })
            .collect_view()
    };

    view! {
        <div class="relative inline-block">
            <button
                type="button"
                class=final_class
                on:click=move |_| open.update(|o| *o = !*o)
            >
                <crate::ui::Icon name="calendar" size=16 />
                <span>{trigger_label}</span>
            </button>

            <Show when=move || open.get() fallback=|| ()>
                // Click-catcher backdrop closes the popover on outside click.
                <div
                    class="fixed inset-0 z-40"
                    on:click=move |_| open.set(false)
                ></div>
                <div class="absolute left-0 top-full mt-1 z-50 w-64 rounded-grape bg-longan-900 inset-ring inset-ring-longan-700 p-3 shadow-none">
                    <div class="flex items-center justify-between mb-2">
                        <button
                            type="button"
                            class="size-7 rounded-seed text-rambutan-300 hover:bg-longan-800 transition-colors"
                            on:click=go_prev
                        >
                            "‹"
                        </button>
                        <span class="text-label text-rambutan-100">
                            {move || format!("{} {}", MONTHS[(view_month.get() - 1) as usize], view_year.get())}
                        </span>
                        <button
                            type="button"
                            class="size-7 rounded-seed text-rambutan-300 hover:bg-longan-800 transition-colors"
                            on:click=go_next
                        >
                            "›"
                        </button>
                    </div>
                    <div class="grid grid-cols-7 gap-0.5 mb-1">
                        {WEEKDAYS
                            .iter()
                            .map(|d| {
                                view! {
                                    <div class="size-8 grid place-items-center text-label-sm text-rambutan-500">
                                        {*d}
                                    </div>
                                }
                            })
                            .collect_view()}
                    </div>
                    <div class="grid grid-cols-7 gap-0.5 place-items-center">
                        {grid}
                    </div>
                </div>
            </Show>
        </div>
    }
}
