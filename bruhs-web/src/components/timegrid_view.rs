//! Time-grid surface — renders a 24-row hour grid with one or seven day
//! columns. Used by both Day and Week views; the difference is just the
//! column count (1 vs 7) and how many days we anchor to.
//!
//! Layout strategy:
//! - Each hour row is a fixed height (HOUR_PX). The whole grid scrolls
//!   inside its parent.
//! - Events render as absolutely-positioned blocks per column, sized by
//!   their duration. Top = (start - day_start) / 1ms × pixels-per-ms.
//! - Overlapping events split their column horizontally so neither hides
//!   the other (simple "lanes" algorithm).
//! - "Now" line shows when the visible range includes today, anchored to
//!   the current local time.
//!
//! Interactions:
//! - Click empty area → open create dialog at that hour.
//! - Click event block → open edit dialog.
//! - Drag event block → move event (start time changes; duration kept).
//!   Cross-column drops in Week view are supported (Mon → Wed reschedules).
//! - Drag bottom-edge handle → resize event (end time changes only).

use bruhs_models::num::{ToFloatLossy, ToIntSaturating};
use bruhs_models::EpochMs;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::state::{
    CalendarAnchor, CalendarEvent, CalendarEventsSignal, EventSource, SetCalendarEventsSignal,
    SetEventCreateOpen, SetEventCreateStart, SetSelectedEventId,
};
use crate::storage;

const DAY_MS: f64 = 1000.0 * 60.0 * 60.0 * 24.0;
const HOUR_MS: f64 = 1000.0 * 60.0 * 60.0;
const HOUR_PX: f64 = 40.0;
/// Snap interval for drag-move and resize, in ms. 15 minutes feels like
/// the right "sticky" feel — fine enough for real scheduling, coarse
/// enough that you never land on a noisy 17:43.
const SNAP_MS: f64 = 15.0 * 60.0 * 1000.0;
/// Minimum event duration enforced during resize.
const MIN_DURATION_MS: f64 = 15.0 * 60.0 * 1000.0;

/// offsetY is relative to the event target, which can be a title or gridline
/// child. Client coordinates and the handler element's rect share one space,
/// including when the time grid has scrolled.
fn handler_offset_y(client_y: i32, target: Option<web_sys::EventTarget>) -> Option<f64> {
    target
        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
        .map(|element| f64::from(client_y) - element.get_bounding_client_rect().top())
}

/// Local-midnight ms for the day containing `ms`.
fn day_start_ms(ms: f64) -> f64 {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms));
    js_sys::Date::new_with_year_month_day(
        d.get_full_year(),
        crate::clock::field(d.get_month()),
        crate::clock::field(d.get_date()),
    )
    .get_time()
}

/// Local-midnight ms of the most recent Sunday on-or-before `ms`.
fn week_start_ms(ms: f64) -> f64 {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms));
    let dow = f64::from(d.get_day()); // 0 = Sun
    day_start_ms(ms) - dow * DAY_MS
}

/// Day-of-month abbreviation row (e.g. "Sun 3"). We always include the
/// day-of-week — in Day view it's just one column but the label is still
/// useful context.
fn format_day_header(day_ms: f64) -> String {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(day_ms));
    let dow = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    format!("{} {}", dow[d.get_day() as usize], d.get_date())
}

/// Map a stored color token to its bruhs Tailwind bg class. Mirrors
/// `month_view::color_dot_class` but for the larger event blocks.
fn color_block_class(color: Option<&str>) -> &'static str {
    match color.unwrap_or("") {
        "persimmon" => {
            "bg-persimmon-500/30 inset-ring inset-ring-persimmon-400/40 text-persimmon-100"
        }
        "blueberry" => {
            "bg-blueberry-500/30 inset-ring inset-ring-blueberry-400/40 text-blueberry-100"
        }
        "pandan" => "bg-pandan-500/30 inset-ring inset-ring-pandan-400/40 text-pandan-100",
        "lychee" => "bg-lychee-500/30 inset-ring inset-ring-lychee-400/40 text-lychee-100",
        "durian" => "bg-durian-500/30 inset-ring inset-ring-durian-400/40 text-durian-100",
        "mangosteen" => {
            "bg-mangosteen-500/30 inset-ring inset-ring-mangosteen-400/40 text-mangosteen-100"
        }
        "dragonfruit" => {
            "bg-dragonfruit-500/30 inset-ring inset-ring-dragonfruit-400/40 text-dragonfruit-100"
        }
        _ => "bg-rambutan-500/30 inset-ring inset-ring-rambutan-400/40 text-rambutan-100",
    }
}

/// Layout-resolved block: which "lane" it occupies inside its column,
/// the total lanes covering it, plus the y-pixel range. Multi-lane events
/// share their column horizontally so none overlaps another.
#[derive(Clone)]
struct LaidOutEvent {
    ev: CalendarEvent,
    top_px: f64,
    height_px: f64,
    lane: usize,
    lanes: usize,
}

/// Lay out a day's events. Greedy lane assignment lives in
/// `bruhs_models::calendar`; this wrapper just maps to/from millisecond
/// coordinates and computes the px geometry.
fn lay_out(events: &[CalendarEvent], day_start: f64) -> Vec<LaidOutEvent> {
    let timed: Vec<&CalendarEvent> = events.iter().filter(|e| !e.all_day).collect();
    let starts_ends: Vec<(i64, i64)> = timed
        .iter()
        .map(|e| {
            (
                EpochMs::saturating_from_f64(e.start_at_ms).as_millis(),
                EpochMs::saturating_from_f64(e.end_at_ms).as_millis(),
            )
        })
        .collect();
    let layout = bruhs_models::calendar::assign_lanes(&starts_ends);

    layout
        .placements
        .into_iter()
        .map(|placement| {
            let ev = timed[placement.index];
            let from = (ev.start_at_ms.max(day_start) - day_start).max(0.0);
            let to = (ev.end_at_ms.min(day_start + DAY_MS) - day_start).max(from + HOUR_MS / 2.0);
            let top_px = (from / HOUR_MS) * HOUR_PX;
            let height_px = ((to - from) / HOUR_MS) * HOUR_PX;
            LaidOutEvent {
                ev: ev.clone(),
                top_px,
                height_px: height_px.max(20.0),
                lane: placement.lane,
                lanes: layout.lanes,
            }
        })
        .collect()
}

/// Filter events that intersect `[day_start, day_start + DAY_MS)`.
fn events_for_day(events: &[CalendarEvent], day_start: f64) -> Vec<CalendarEvent> {
    let day_end = day_start + DAY_MS;
    events
        .iter()
        .filter(|e| e.start_at_ms < day_end && e.end_at_ms > day_start)
        .cloned()
        .collect()
}

/// Snap a millis offset to the nearest SNAP_MS boundary, preserving sign.
fn snap_ms(ms: f64) -> f64 {
    (ms / SNAP_MS).round() * SNAP_MS
}

#[component]
pub fn TimegridView(
    /// Number of day columns: 1 for Day, 7 for Week. Anything else just
    /// renders that many days starting from the anchor's week-start.
    #[prop()]
    columns: usize,
) -> impl IntoView {
    let CalendarAnchor(anchor) = expect_context::<CalendarAnchor>();
    let CalendarEventsSignal(events) = expect_context::<CalendarEventsSignal>();
    let SetCalendarEventsSignal(set_events) = expect_context::<SetCalendarEventsSignal>();
    let SetEventCreateOpen(set_event_create_open) = expect_context::<SetEventCreateOpen>();
    let SetEventCreateStart(set_event_create_start) = expect_context::<SetEventCreateStart>();
    let SetSelectedEventId(set_selected_event_id) = expect_context::<SetSelectedEventId>();

    // Days to render: for Week start at week-start; for Day use the
    // anchor's own day. Anything else falls back to anchor-day-aligned.
    let days = Signal::derive(move || {
        let a = anchor.get();
        let start = if columns == 7 {
            week_start_ms(a)
        } else {
            day_start_ms(a)
        };
        (0..columns)
            .map(|i| start + i.to_f64_lossy() * DAY_MS)
            .collect::<Vec<_>>()
    });

    // Today's local-midnight, recomputed reactively on anchor change.
    let today_start = Signal::derive(move || {
        let _ = anchor.get();
        day_start_ms(js_sys::Date::now())
    });
    let now_offset_ms = Signal::derive(move || {
        let _ = anchor.get();
        js_sys::Date::now() - day_start_ms(js_sys::Date::now())
    });

    // Move-event helper, captured by every column's `drop` handler.
    // - id: which event was dropped
    // - drop_offset_y: drop site Y in column-local px
    // - grab_offset_y: where in the source block the user grabbed (px from block top)
    // - day_start: the destination column's day-start (lets cross-day moves work)
    let move_event_to =
        move |id: String, drop_offset_y: f64, grab_offset_y: f64, day_start: f64| {
            let mut updated = None;
            set_events.update(|evs| {
                if let Some(ev) = evs.iter_mut().find(|e| e.id == id) {
                    let duration = ev.end_at_ms - ev.start_at_ms;
                    // Pixels → ms relative to column top.
                    let new_top_px = (drop_offset_y - grab_offset_y).max(0.0);
                    let new_offset_ms = (new_top_px / HOUR_PX) * HOUR_MS;
                    let snapped = snap_ms(new_offset_ms);
                    // Clamp so the event stays inside the column day.
                    let max_start = DAY_MS - duration.min(DAY_MS);
                    let clamped = snapped.clamp(0.0, max_start.max(0.0));
                    let new_start = day_start + clamped;
                    if new_start != ev.start_at_ms {
                        let original = ev.clone();
                        ev.start_at_ms = new_start;
                        ev.end_at_ms = new_start + duration;
                        ev.updated_at = crate::clock::now_ms();
                        updated = Some((original, ev.clone()));
                    }
                }
            });
            if let Some((original, ev)) = updated {
                storage::persist_event(Some(original), ev);
            }
        };

    // Resize-end helper: drag the bottom edge to change end_at_ms only.
    // - new_end_offset_ms: ms from the event's day_start after drag. Snapped
    //   and clamped to start + MIN_DURATION_MS .. day_end inside this fn.
    let resize_event_end = move |id: String, new_end_offset_ms: f64, day_start: f64| {
        let mut updated = None;
        set_events.update(|evs| {
            if let Some(ev) = evs.iter_mut().find(|e| e.id == id) {
                let snapped = snap_ms(new_end_offset_ms);
                let candidate = day_start + snapped;
                let min_end = ev.start_at_ms + MIN_DURATION_MS;
                let max_end = day_start + DAY_MS;
                let new_end = candidate.max(min_end).min(max_end);
                if new_end != ev.end_at_ms {
                    let original = ev.clone();
                    ev.end_at_ms = new_end;
                    ev.updated_at = crate::clock::now_ms();
                    updated = Some((original, ev.clone()));
                }
            }
        });
        if let Some((original, ev)) = updated {
            storage::persist_event(Some(original), ev);
        }
    };

    // Build hour-row labels (00–23). Pre-rendered once.
    let hour_rows: Vec<_> = (0..24)
        .map(|h| {
            view! {
                <div
                    class="border-b border-longan-800/40 text-label-sm text-rambutan-500 px-1.5 pt-0.5 select-none"
                    style=format!("height: {HOUR_PX}px;")
                >
                    {format!("{h:02}:00")}
                </div>
            }
        })
        .collect();

    let total_height_px = HOUR_PX * 24.0;

    // Open scrolled to ~2h before now instead of midnight, so the useful
    // hours (and the now-line) are on screen without manual scrolling.
    let scroll_ref: NodeRef<leptos::html::Div> = NodeRef::new();
    let did_autoscroll = StoredValue::new(false);
    Effect::new(move |_| {
        if did_autoscroll.get_value() {
            return;
        }
        if let Some(el) = scroll_ref.get() {
            let now_hour = f64::from(js_sys::Date::new_0().get_hours());
            el.set_scroll_top(((now_hour - 2.0).max(0.0) * HOUR_PX).to_i32_saturating());
            did_autoscroll.set_value(true);
        }
    });

    view! {
        <div class="flex flex-col flex-1 overflow-hidden">
            // Day-header strip. Sits above the scrollable body so the grid
            // can scroll under it without pushing labels out of view.
            <div
                class="grid border-b border-longan-800/40 shrink-0 text-label"
                style=format!("grid-template-columns: 56px repeat({columns}, 1fr);")
            >
                <div></div>
                {move || days.get().into_iter().map(|d| {
                    let is_today = d == today_start.get();
                    let label_class = if is_today {
                        "text-persimmon-300"
                    } else {
                        "text-rambutan-400"
                    };
                    view! {
                        <div class=format!("px-2 py-1.5 text-center border-l border-longan-800/30 {label_class}")>
                            {format_day_header(d)}
                        </div>
                    }
                }).collect_view()}
            </div>

            // Scrollable body: time-axis column + day columns.
            <div node_ref=scroll_ref class="flex-1 overflow-y-auto">
                <div
                    class="grid relative"
                    style=format!("grid-template-columns: 56px repeat({columns}, 1fr); height: {total_height_px}px;")
                >
                    // Time axis (left gutter).
                    <div class="border-r border-longan-800/30">
                        {hour_rows}
                    </div>

                    // Day columns.
                    {move || {
                        let day_list = days.get();
                        let evs = events.get();
                        day_list.into_iter().map(|day_start| {
                            let day_events = events_for_day(&evs, day_start);
                            let laid = lay_out(&day_events, day_start);
                            let on_click_col = move |ev: web_sys::MouseEvent| {
                                let Some(y) = handler_offset_y(ev.client_y(), ev.current_target()) else { return; };
                                let hour = (y / HOUR_PX).floor().clamp(0.0, 23.0).to_i32_saturating();
                                let prefill_ms = day_start + f64::from(hour) * HOUR_MS;
                                set_event_create_start.set(Some(prefill_ms));
                                set_selected_event_id.set(None);
                                set_event_create_open.set(true);
                            };
                            let is_today = day_start == today_start.get();
                            let now_y = now_offset_ms.get() / HOUR_MS * HOUR_PX;

                            // Drop handler — re-uses move_event_to. We need
                            // coordinates relative to the day column, even
                            // when the drop target is an event or gridline child.
                            let on_drop = move |ev: web_sys::DragEvent| {
                                ev.prevent_default();
                                let dt = match ev.data_transfer() {
                                    Some(d) => d,
                                    None => return,
                                };
                                let payload = dt.get_data("text/plain").unwrap_or_default();
                                // Payload format: "move:{id}|{grab_y}"
                                let Some(rest) = payload.strip_prefix("move:") else {
                                    return;
                                };
                                let mut parts = rest.splitn(2, '|');
                                let id = parts.next().unwrap_or("").to_string();
                                let grab_y: f64 = parts
                                    .next()
                                    .and_then(|s| s.parse().ok())
                                    .unwrap_or(0.0);
                                if id.is_empty() {
                                    return;
                                }
                                let Some(drop_y) = handler_offset_y(ev.client_y(), ev.current_target()) else { return; };
                                move_event_to(id, drop_y, grab_y, day_start);
                            };

                            view! {
                                <div
                                    class="relative border-l border-longan-800/30 cursor-pointer"
                                    on:click=on_click_col
                                    on:dragover=move |ev: web_sys::DragEvent| {
                                        ev.prevent_default();
                                        if let Some(dt) = ev.data_transfer() {
                                            dt.set_drop_effect("move");
                                        }
                                    }
                                    on:drop=on_drop
                                >
                                    // Hour gridlines.
                                    {(0..24).map(|h| view! {
                                        <div
                                            class="border-b border-longan-800/30 hover:bg-longan-800/15"
                                            style=format!("height: {HOUR_PX}px;")
                                            data-hour=h
                                        ></div>
                                    }).collect_view()}

                                    // Now indicator — a thin persimmon
                                    // line across today's column.
                                    {is_today.then(|| view! {
                                        <div
                                            class="absolute left-0 right-0 pointer-events-none"
                                            style=format!("top: {now_y}px;")
                                        >
                                            <div class="h-px bg-persimmon-500 relative">
                                                <div class="absolute -left-1 -top-1 size-2 rounded-orb bg-persimmon-500"></div>
                                            </div>
                                        </div>
                                    })}

                                    // Event blocks.
                                    {laid.into_iter().map(|loe| {
                                        let id = loe.ev.id.clone();
                                        let id_for_drag = id.clone();
                                        let id_for_resize = id.clone();
                                        let id_for_click = id;
                                        let title = loe.ev.title.clone();
                                        let is_google = matches!(loe.ev.source, EventSource::Google);
                                        let bg = color_block_class(loe.ev.color.as_deref());
                                        let pct = 100.0 / loe.lanes.to_f64_lossy();
                                        let left_pct = pct * loe.lane.to_f64_lossy();
                                        let event_day_start = day_start;

                                        let on_block_click = move |ev: web_sys::MouseEvent| {
                                            ev.stop_propagation();
                                            set_selected_event_id.set(Some(id_for_click.clone()));
                                            set_event_create_open.set(true);
                                        };
                                        // Drag-source: write the event id +
                                        // grab Y-offset (within the block)
                                        // so the column drop handler can
                                        // figure out what the new top should
                                        // be. effectAllowed = move so the
                                        // ghost cursor reflects the action.
                                        let on_dragstart = move |ev: web_sys::DragEvent| {
                                            ev.stop_propagation();
                                            if let Some(dt) = ev.data_transfer() {
                                                let Some(grab_y) = handler_offset_y(ev.client_y(), ev.current_target()) else { return; };
                                                let payload = format!(
                                                    "move:{}|{:.2}",
                                                    id_for_drag, grab_y
                                                );
                                                let _ = dt.set_data("text/plain", &payload);
                                                dt.set_effect_allowed("move");
                                            }
                                        };
                                        // Resize handle — pointer-down
                                        // captures the pointer so all moves
                                        // route here until release.
                                        let resize_id = id_for_resize;
                                        let on_resize_pointerdown = move |ev: web_sys::PointerEvent| {
                                            ev.stop_propagation();
                                            ev.prevent_default();
                                            let target = match ev.target()
                                                .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                                            {
                                                Some(t) => t,
                                                None => return,
                                            };
                                            let _ = target.set_pointer_capture(ev.pointer_id());
                                        };
                                        let resize_id_move = resize_id;
                                        let on_resize_pointermove = move |ev: web_sys::PointerEvent| {
                                            // Only act while the pointer is
                                            // captured (i.e. we initiated a
                                            // resize).
                                            let target = match ev.target()
                                                .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                                            {
                                                Some(t) => t,
                                                None => return,
                                            };
                                            if !target.has_pointer_capture(ev.pointer_id()) {
                                                return;
                                            }
                                            ev.prevent_default();
                                            ev.stop_propagation();
                                            // Use clientY mapped against the
                                            // column's bounding rect for
                                            // accurate end-time across the
                                            // full viewport, not just the
                                            // event-block-relative coords.
                                            let column = target
                                                .closest(".relative.border-l")
                                                .ok()
                                                .flatten();
                                            let Some(column) = column else { return };
                                            let rect = column.get_bounding_client_rect();
                                            let y = f64::from(ev.client_y()) - rect.top();
                                            let new_end_ms = (y / HOUR_PX) * HOUR_MS;
                                            resize_event_end(
                                                resize_id_move.clone(),
                                                new_end_ms,
                                                event_day_start,
                                            );
                                        };
                                        let on_resize_pointerup = move |ev: web_sys::PointerEvent| {
                                            let target = match ev.target()
                                                .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                                            {
                                                Some(t) => t,
                                                None => return,
                                            };
                                            let _ = target.release_pointer_capture(ev.pointer_id());
                                        };

                                        view! {
                                            <div
                                                draggable="true"
                                                on:dragstart=on_dragstart
                                                on:click=on_block_click
                                                class=format!(
                                                    "absolute rounded-grape px-1.5 py-1 text-label-sm text-left truncate {bg} cursor-grab active:cursor-grabbing select-none"
                                                )
                                                style=format!(
                                                    "top: {top}px; height: {h}px; left: calc({l}% + 2px); width: calc({w}% - 4px);",
                                                    top = loe.top_px,
                                                    h = loe.height_px,
                                                    l = left_pct,
                                                    w = pct,
                                                )
                                            >
                                                <div class="flex items-center gap-1 pointer-events-none min-w-0">
                                                    {if is_google {
                                                        view! {
                                                            <span title="Synced from Google Calendar" class="text-label-sm text-blueberry-300/80 bg-blueberry-500/15 inset-ring inset-ring-blueberry-500/40 rounded-seed px-0.5 leading-none shrink-0">"G"</span>
                                                        }.into_any()
                                                    } else {
                                                        ().into_any()
                                                    }}
                                                    <span class="truncate">{title}</span>
                                                </div>
                                                // Resize handle — small
                                                // strip at the bottom of the
                                                // block. ns-resize cursor
                                                // signals the affordance.
                                                <div
                                                    on:pointerdown=on_resize_pointerdown
                                                    on:pointermove=on_resize_pointermove
                                                    on:pointerup=on_resize_pointerup
                                                    on:click=move |ev: web_sys::MouseEvent| ev.stop_propagation()
                                                    // Suppress the parent's
                                                    // native HTML5 drag when
                                                    // the user grabs the
                                                    // resize handle — we
                                                    // already track the
                                                    // gesture via pointer
                                                    // events.
                                                    on:dragstart=move |ev: web_sys::DragEvent| {
                                                        ev.prevent_default();
                                                        ev.stop_propagation();
                                                    }
                                                    class="absolute left-0 right-0 bottom-0 h-1.5 cursor-ns-resize"
                                                ></div>
                                            </div>
                                        }
                                    }).collect_view()}
                                </div>
                            }
                        }).collect_view()
                    }}
                </div>
            </div>
        </div>
    }
}
