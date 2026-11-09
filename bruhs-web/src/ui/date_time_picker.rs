//! Custom date-and-time picker. Same calendar grid as `DatePicker`, with two
//! hour/minute fields below — replaces the browser-native `<input type=
//! "datetime-local">` UI. Value is `YYYY-MM-DDTHH:MM` (the wire format the
//! native input used), so callers don't change how they store datetimes.

use leptos::prelude::*;

use bruhs_models::time::days_in_month;

use crate::ui::date_picker::{first_weekday, parse_iso, MONTHS, WEEKDAYS};

/// Parse `YYYY-MM-DDTHH:MM` into `(year, month 1-12, day, hour 0-23, minute 0-59)`.
fn parse_iso_datetime(s: &str) -> Option<(i32, u32, u32, u32, u32)> {
    let (date, time) = s.split_once('T')?;
    let (y, m, d) = parse_iso(date)?;
    let (h_s, mi_s) = time.split_once(':')?;
    // Tolerate trailing seconds (some browsers emit `HH:MM:SS`).
    let mi_s = mi_s.split(':').next()?;
    let h = h_s.parse::<u32>().ok().filter(|h| *h < 24)?;
    let mi = mi_s.parse::<u32>().ok().filter(|x| *x < 60)?;
    Some((y, m, d, h, mi))
}

#[component]
pub fn DateTimePicker(
    /// ISO `YYYY-MM-DDTHH:MM` value (may be empty).
    #[prop(optional, into)]
    value: Signal<String>,
    /// Fired with the chosen ISO `YYYY-MM-DDTHH:MM` string.
    #[prop(optional)]
    on_change: Option<Callback<String>>,
    #[prop(optional, into)] disabled: Signal<bool>,
    #[prop(optional, into)] class: String,
) -> impl IntoView {
    let open = RwSignal::new(false);

    // Initial calendar view + time. View follows the value's date when present,
    // else today. Time defaults to 09:00 when there's no value to parse.
    let parsed = parse_iso_datetime(&value.get_untracked());
    let (init_year, init_month) = parsed.map_or_else(
        || {
            let now = js_sys::Date::new_0();
            (
                crate::clock::field(now.get_full_year()),
                now.get_month() + 1,
            )
        },
        |(y, m, _, _, _)| (y, m),
    );
    let view_year = RwSignal::new(init_year);
    let view_month = RwSignal::new(init_month);
    let (init_h, init_m) = parsed.map_or((9, 0), |(_, _, _, h, mi)| (h, mi));
    let hour = RwSignal::new(init_h);
    let minute = RwSignal::new(init_m);

    // Helper: emit a fresh combined ISO. Uses the value's current date if it
    // parses, else falls back to today.
    let emit_with_time = move |h: u32, mi: u32| {
        let (y, m, d) = parse_iso_datetime(&value.get_untracked()).map_or_else(
            || {
                let now = js_sys::Date::new_0();
                (
                    crate::clock::field(now.get_full_year()),
                    now.get_month() + 1,
                    now.get_date(),
                )
            },
            |(y, m, d, _, _)| (y, m, d),
        );
        if let Some(cb) = on_change {
            cb.run(format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}"));
        }
    };

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

    let trigger_label = move || match parse_iso_datetime(&value.get()) {
        Some((y, m, d, h, mi)) => format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}"),
        None => "Select date & time".to_string(),
    };

    let final_class = format!(
        "inline-flex items-center gap-2 rounded-grape bg-white/5 px-3 py-2 text-body text-rambutan-100 inset-ring inset-ring-white/10 hover:inset-ring-white/20 focus-visible:outline-2 -outline-offset-1 focus-visible:outline-persimmon-400 cursor-pointer disabled:opacity-60 disabled:cursor-not-allowed {class}"
    );

    let grid = move || {
        let (y, m) = (view_year.get(), view_month.get());
        let lead = first_weekday(y, m) as usize;
        // An out-of-range month (never produced by the month stepper) shows
        // no days rather than a guess.
        let total = days_in_month(y, m).unwrap_or(0);
        let selected = parse_iso_datetime(&value.get()).map(|(y, m, d, _, _)| (y, m, d));
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
                        let (h, mi) = (hour.get_untracked(), minute.get_untracked());
                        if let Some(cb) = on_change {
                            cb.run(format!("{y:04}-{m:02}-{day:02}T{h:02}:{mi:02}"));
                        }
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

    let on_hour_input = move |ev: web_sys::Event| {
        use wasm_bindgen::JsCast;
        if let Some(input) = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        {
            if let Ok(h) = input.value().parse::<u32>() {
                let h = h.min(23);
                hour.set(h);
                emit_with_time(h, minute.get_untracked());
            }
        }
    };
    let on_minute_input = move |ev: web_sys::Event| {
        use wasm_bindgen::JsCast;
        if let Some(input) = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        {
            if let Ok(mi) = input.value().parse::<u32>() {
                let mi = mi.min(59);
                minute.set(mi);
                emit_with_time(hour.get_untracked(), mi);
            }
        }
    };

    let time_field_class =
        "w-14 bg-longan-900 inset-ring inset-ring-longan-700 rounded-seed px-1.5 py-0.5 text-label text-rambutan-200 text-center focus-visible:outline-2 -outline-offset-1 focus-visible:outline-persimmon-400";

    view! {
        <div class="relative inline-block">
            <button
                type="button"
                class=final_class
                disabled=move || disabled.get()
                on:click=move |_| {
                    if !disabled.get_untracked() {
                        open.update(|o| *o = !*o);
                    }
                }
            >
                <crate::ui::Icon name="calendar" size=16 />
                <span>{trigger_label}</span>
            </button>

            <Show when=move || open.get() fallback=|| ()>
                <div class="fixed inset-0 z-40" on:click=move |_| open.set(false)></div>
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

                    // Time row — two number fields below the calendar grid.
                    <div class="mt-3 pt-3 border-t border-longan-800/60 flex items-center justify-center gap-1.5 text-label text-rambutan-400">
                        <span class="uppercase tracking-wide text-label-sm">"Time"</span>
                        <input
                            type="number"
                            min="0"
                            max="23"
                            class=time_field_class
                            prop:value=move || format!("{:02}", hour.get())
                            on:input=on_hour_input
                        />
                        <span>":"</span>
                        <input
                            type="number"
                            min="0"
                            max="59"
                            class=time_field_class
                            prop:value=move || format!("{:02}", minute.get())
                            on:input=on_minute_input
                        />
                    </div>
                </div>
            </Show>
        </div>
    }
}
