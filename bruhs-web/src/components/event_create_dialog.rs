//! Event editor — handles BOTH create and edit modes:
//!
//! - **Create**: opened with `selected_event_id = None`. Fields reset; if
//!   the user clicked a specific day in the month grid we prefill start
//!   to 9am that day, otherwise to "next half hour". Submit pushes a new
//!   event into the events signal.
//! - **Edit**: opened with `selected_event_id = Some(id)`. Fields prefill
//!   from the matched event; submit replaces it in-place. A Delete button
//!   appears in the action row.
//!
//! Title is unchanged across modes ("New event" vs "Edit event"). Both
//! flows persist via `storage::persist_event` — IDB writes are upserts.

use leptos::prelude::*;
use uuid::Uuid;

use crate::state::{
    CalendarEvent, CalendarEventsSignal, EventCreateOpen, EventCreateStart, EventSource,
    SelectedEventId, SetCalendarEventsSignal, SetEventCreateOpen, SetSelectedEventId,
};
use crate::storage;
use crate::ui::{
    Button, ButtonSize, ButtonVariant, Checkbox, DateTimePicker, Dialog, Dropdown, DropdownOption,
    Input, Textarea,
};

/// Format a local-time millis timestamp as `YYYY-MM-DDTHH:MM`, the format
/// expected by `<input type="datetime-local">`.
fn format_datetime_local(ms: f64) -> String {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms));
    let y = d.get_full_year();
    let m = d.get_month() + 1;
    let dd = d.get_date();
    let hh = d.get_hours();
    let mm = d.get_minutes();
    format!("{y:04}-{m:02}-{dd:02}T{hh:02}:{mm:02}")
}

/// Parse a `YYYY-MM-DDTHH:MM` string back into a millis timestamp. Returns
/// `None` on malformed input — callers should treat that as "leave field
/// unchanged" rather than crash.
fn parse_datetime_local(s: &str) -> Option<f64> {
    let (date_part, time_part) = s.split_once('T')?;
    let mut dp = date_part.split('-');
    let y: i32 = dp.next()?.parse().ok()?;
    let m: u32 = dp.next()?.parse().ok()?;
    let d: u32 = dp.next()?.parse().ok()?;
    let mut tp = time_part.split(':');
    let hh: u32 = tp.next()?.parse().ok()?;
    let mm: u32 = tp.next()?.parse().ok()?;

    let date = js_sys::Date::new_with_year_month_day_hr_min_sec(
        crate::clock::year_arg(y),
        crate::clock::field(m) - 1,
        crate::clock::field(d),
        crate::clock::field(hh),
        crate::clock::field(mm),
        0,
    );
    Some(date.get_time())
}

/// Default to "now rounded up to the next half-hour" for fresh events.
fn default_start_ms() -> f64 {
    let now = js_sys::Date::now();
    let half_hour = 30.0 * 60.0 * 1000.0;
    (now / half_hour).ceil() * half_hour
}

const COLOR_OPTIONS: &[(&str, &str)] = &[
    ("persimmon", "Persimmon"),
    ("blueberry", "Blueberry"),
    ("pandan", "Pandan"),
    ("lychee", "Lychee"),
    ("durian", "Durian"),
    ("mangosteen", "Mangosteen"),
    ("dragonfruit", "Dragonfruit"),
];

#[component]
pub fn EventCreateDialog() -> impl IntoView {
    let EventCreateOpen(open) = expect_context::<EventCreateOpen>();
    let SetEventCreateOpen(set_open) = expect_context::<SetEventCreateOpen>();
    let EventCreateStart(create_start) = expect_context::<EventCreateStart>();
    let CalendarEventsSignal(events) = expect_context::<CalendarEventsSignal>();
    let SetCalendarEventsSignal(set_events) = expect_context::<SetCalendarEventsSignal>();
    let SelectedEventId(selected_event_id) = expect_context::<SelectedEventId>();
    let SetSelectedEventId(set_selected_event_id) = expect_context::<SetSelectedEventId>();
    let recording_targets = crate::auto_record::RecordingTargets::from_context();
    let capture_settings = crate::auto_record::CaptureSettings::from_context();

    let (title, set_title) = signal(String::new());
    let (notes, set_notes) = signal(String::new());
    let (start_str, set_start_str) = signal(String::new());
    let (end_str, set_end_str) = signal(String::new());
    let (all_day, set_all_day) = signal(false);
    let (color, set_color) = signal("persimmon".to_string());

    // Reset every field when the dialog opens. Branches:
    // - If selected_event_id matches an event → EDIT mode, prefill from it.
    // - Else if a calendar cell was clicked → CREATE mode at its requested time.
    // - Else → CREATE mode at "next half hour".
    Effect::new(move |_| {
        if !open.get() {
            return;
        }
        if let Some(id) = selected_event_id.get() {
            if let Some(ev) = events.with_untracked(|evs| evs.iter().find(|e| e.id == id).cloned())
            {
                set_title.set(ev.title);
                set_notes.set(ev.notes.unwrap_or_default());
                set_all_day.set(ev.all_day);
                set_color.set(ev.color.unwrap_or_else(|| "persimmon".to_string()));
                set_start_str.set(format_datetime_local(ev.start_at_ms));
                set_end_str.set(format_datetime_local(ev.end_at_ms));
                return;
            }
        }
        // CREATE mode reset.
        set_title.set(String::new());
        set_notes.set(String::new());
        set_all_day.set(false);
        set_color.set("persimmon".to_string());
        let start_ms = create_start.get().unwrap_or_else(default_start_ms);
        let end_ms = start_ms + 60.0 * 60.0 * 1000.0; // +1 hour
        set_start_str.set(format_datetime_local(start_ms));
        set_end_str.set(format_datetime_local(end_ms));
    });

    let close_dialog = move || {
        set_open.set(false);
        // Always clear the selection on close so the next open defaults
        // back to CREATE mode.
        set_selected_event_id.set(None);
    };
    let on_close_cb = Callback::new(move |_| close_dialog());

    let submit = move || {
        let t = title.get().trim().to_string();
        if t.is_empty() {
            return;
        }
        let start_ms = parse_datetime_local(&start_str.get()).unwrap_or_else(default_start_ms);
        let mut end_ms = parse_datetime_local(&end_str.get()).unwrap_or(start_ms + 3_600_000.0);
        if end_ms <= start_ms {
            end_ms = start_ms + 3_600_000.0;
        }
        let (start_ms, end_ms) = if all_day.get() {
            let midnight = crate::clock::local_day_start(start_ms);
            (midnight, crate::clock::add_local_days(midnight, 1))
        } else {
            (start_ms, end_ms)
        };

        let now = crate::clock::now_ms();
        let notes_val = notes.get();
        let notes_opt = if notes_val.trim().is_empty() {
            None
        } else {
            Some(notes_val)
        };
        let color_opt = Some(color.get());
        let editing_id = selected_event_id.get_untracked();

        match editing_id {
            Some(id) => {
                // EDIT — find and replace in-place; preserve created_at.
                let mut updated = None;
                set_events.update(|evs| {
                    if let Some(ev) = evs.iter_mut().find(|e| e.id == id) {
                        let original = ev.clone();
                        ev.title = t.clone();
                        ev.start_at_ms = start_ms;
                        ev.end_at_ms = end_ms;
                        ev.all_day = all_day.get();
                        ev.color = color_opt.clone();
                        ev.notes = notes_opt.clone();
                        ev.updated_at = now;
                        updated = Some((original, ev.clone()));
                    }
                });
                if let Some((original, ev)) = updated {
                    storage::persist_event(Some(original), ev);
                }
            }
            None => {
                // CREATE
                let event = CalendarEvent {
                    id: Uuid::new_v4().to_string(),
                    title: t,
                    start_at_ms: start_ms,
                    end_at_ms: end_ms,
                    all_day: all_day.get(),
                    color: color_opt,
                    notes: notes_opt,
                    created_at: now,
                    updated_at: now,
                    source: bruhs_models::EventSource::Local,
                    google_event_id: None,
                    calendar_id: None,
                    attendees: Vec::new(),
                    organizer: None,
                    location: None,
                    conferencing: None,
                    recurring_event_id: None,
                    html_link: None,
                };
                set_events.update(|evs| evs.push(event.clone()));
                storage::persist_event(None, event);
            }
        }
        close_dialog();
    };

    let on_submit_btn = Callback::new(move |_: web_sys::MouseEvent| submit());
    let on_cancel = Callback::new(move |_: web_sys::MouseEvent| close_dialog());

    let on_delete = move |_: web_sys::MouseEvent| {
        let Some(id) = selected_event_id.get_untracked() else {
            return;
        };
        let original = events.with_untracked(|evs| evs.iter().find(|e| e.id == id).cloned());
        set_events.update(|evs| evs.retain(|e| e.id != id));
        if let Some(original) = original {
            storage::forget_event(original);
        }
        close_dialog();
    };

    // Is the currently-selected event from Google Calendar?
    let is_google_event = Memo::new(move |_| {
        selected_event_id.get().is_some_and(|id| {
            events.with(|evs| {
                evs.iter()
                    .any(|e| e.id == id && matches!(e.source, EventSource::Google))
            })
        })
    });
    // The html_link of the currently-selected event (for "Open in Google Calendar").
    let html_link_for_dialog = Memo::new(move |_| {
        selected_event_id.get().and_then(|id| {
            events.with(|evs| {
                evs.iter()
                    .find(|e| e.id == id)
                    .and_then(|e| e.html_link.clone())
            })
        })
    });

    // Reactive title + button label based on mode.
    let header_text = move || {
        if is_google_event.get() {
            "Google event (read-only)"
        } else if selected_event_id.get().is_some() {
            "Edit event"
        } else {
            "New event"
        }
    };
    let submit_label = move || {
        if selected_event_id.get().is_some() {
            "Save"
        } else {
            "Create"
        }
    };
    let editing = move || selected_event_id.get().is_some();

    view! {
        <Dialog
            open=Signal::derive(move || open.get())
            on_close=on_close_cb
            max_width="max-w-md".to_string()
            label="Calendar event"
        >
            <div class="border-b border-longan-800 px-4 py-3">
                <h2 class="text-h4 text-rambutan-100">{header_text}</h2>
            </div>

            <div class="px-4 py-3 space-y-3">
                <div class="space-y-1.5">
                    <label class="text-label-sm text-rambutan-400">"Title"</label>
                    <Input
                        value=Signal::derive(move || title.get())
                        on_input=Callback::new(move |v| set_title.set(v))
                        placeholder="What's happening?".to_string()
                        autofocus=true
                        disabled=Signal::derive(move || is_google_event.get())
                    />
                </div>

                <div class="grid grid-cols-2 gap-3">
                    <div class="space-y-1.5">
                        <label class="text-label-sm text-rambutan-400">"Start"</label>
                        <DateTimePicker
                            value=Signal::derive(move || start_str.get())
                            on_change=Callback::new(move |v| set_start_str.set(v))
                            disabled=Signal::derive(move || is_google_event.get())
                        />
                    </div>
                    <div class="space-y-1.5">
                        <label class="text-label-sm text-rambutan-400">"End"</label>
                        <DateTimePicker
                            value=Signal::derive(move || end_str.get())
                            on_change=Callback::new(move |v| set_end_str.set(v))
                            disabled=Signal::derive(move || is_google_event.get())
                        />
                    </div>
                </div>

                <label class="inline-flex items-center gap-2 text-label-sm text-rambutan-300 cursor-pointer">
                    <Checkbox
                        checked=Signal::derive(move || all_day.get())
                        disabled=Signal::derive(move || is_google_event.get())
                        on_change=Callback::new(move |c: bool| set_all_day.set(c))
                    />
                    "All-day"
                </label>

                <div class="space-y-1.5">
                    <label class="text-label-sm text-rambutan-400">"Color"</label>
                    <Dropdown
                        options={COLOR_OPTIONS
                            .iter()
                            .map(|(value, label)| DropdownOption::new(*value, *label))
                            .collect::<Vec<_>>()}
                        value=Signal::derive(move || color.get())
                        on_change=Callback::new(move |v: String| set_color.set(v))
                        disabled=Signal::derive(move || is_google_event.get())
                        label="Color"
                        class="w-full"
                    />
                </div>

                <div class="space-y-1.5">
                    <label class="text-label-sm text-rambutan-400">"Notes"</label>
                    <Textarea
                        value=Signal::derive(move || notes.get())
                        on_input=Callback::new(move |v| set_notes.set(v))
                        placeholder="Add details (optional)".to_string()
                        rows=3
                        disabled=Signal::derive(move || is_google_event.get())
                    />
                </div>
            </div>

            // Action row — Delete and Record meeting are only present in EDIT
            // mode and live on the left so they're visually separated from
            // Save / Cancel.
            <div class="flex items-center justify-between gap-2 border-t border-longan-800 bg-longan-900/40 px-4 py-3">
                <div class="flex items-center gap-2">
                    {move || editing().then(|| {
                        if is_google_event.get() {
                            // The link comes from Google; only follow a web URL.
                            let link = html_link_for_dialog
                                .get()
                                .and_then(|link| crate::markdown::safe_url(&link).map(str::to_owned));
                            view! {
                                <a
                                    href=link
                                    target="_blank"
                                    rel="noopener"
                                    class="rounded-grape px-3 py-2 text-label-sm text-blueberry-400 hover:bg-blueberry-500/10 cursor-pointer"
                                >
                                    "Open in Google Calendar"
                                </a>
                            }.into_any()
                        } else {
                            let event_for_record = selected_event_id.get_untracked().and_then(|id| {
                                events.with_untracked(|evs| evs.iter().find(|e| e.id == id).cloned())
                            });
                            view! {
                                <div class="flex items-center gap-2">
                                    <button
                                        type="button"
                                        on:click=move |_| {
                                            let Some(event) = event_for_record.clone() else { return };
                                            // The same start as auto-record: create the meeting,
                                            // start capture, and roll back with a notice when
                                            // capture does not start.
                                            crate::auto_record::start_recording_for_event(
                                                event,
                                                capture_settings.current(),
                                                recording_targets,
                                            );
                                        }
                                        class="rounded-grape px-3 py-2 text-label-sm text-persimmon-400 hover:bg-persimmon-500/10 cursor-pointer"
                                    >
                                        "Record meeting"
                                    </button>
                                    <button
                                        type="button"
                                        on:click=on_delete
                                        class="rounded-grape px-3 py-2 text-label-sm text-lychee-400 hover:bg-lychee-500/10 cursor-pointer"
                                    >
                                        "Delete"
                                    </button>
                                </div>
                            }.into_any()
                        }
                    })}
                </div>
                <div class="flex items-center gap-2">
                    <Button
                        variant=ButtonVariant::Ghost
                        size=ButtonSize::Default
                        on_click=on_cancel
                    >
                        "Cancel"
                    </Button>
                    <Button
                        variant=ButtonVariant::Primary
                        size=ButtonSize::Default
                        on_click=on_submit_btn
                        disabled=Signal::derive(move || title.get().trim().is_empty() || is_google_event.get())
                    >
                        {submit_label}
                    </Button>
                </div>
            </div>
        </Dialog>
    }
}
