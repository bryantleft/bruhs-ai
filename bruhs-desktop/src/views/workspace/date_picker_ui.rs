//! Web `ui::DatePicker` / `ui::DateTimePicker`: a trigger button (calendar
//! glyph + value) that opens a month-grid popover; the date-time variant
//! adds hour / minute fields under the grid. Values stay in the dialogs'
//! input states in the web's wire formats (`YYYY-MM-DD`,
//! `YYYY-MM-DDTHH:MM`), so the create / update paths are unchanged.

use chrono::{Datelike, Local, NaiveDate};
use gpui::prelude::*;
use gpui::*;
use gpui_component::input::{Input, InputEvent, InputState};

use super::{is_web_button_activation_key, Workspace};
use crate::{assets, theme};
use bruhs_models::time::days_in_month;

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

/// Web trigger: `gap-2 rounded-grape bg-white/5 px-3 py-2 text-body`
/// with a 16px calendar icon; popover `w-64 rounded-grape p-3`, `size-8`
/// cells on a `gap-0.5` grid, `size-7` month buttons, `w-14` time fields.
const TRIGGER_TEXT_PX: f32 = 16.0;
const TRIGGER_ICON_PX: f32 = 16.0;
const POPOVER_WIDTH_PX: f32 = 256.0;
const POPOVER_GAP_PX: f32 = 4.0;
const CELL_PX: f32 = 32.0;
const GRID_GAP_PX: f32 = 2.0;
const MONTH_BUTTON_PX: f32 = 28.0;
const TIME_FIELD_WIDTH_PX: f32 = 56.0;
const LABEL_PX: f32 = 14.0;
const LABEL_SM_PX: f32 = 12.0;
const GRAPE_RADIUS_PX: f32 = 8.0;
const SEED_RADIUS_PX: f32 = 4.0;
/// Web `DateTimePicker`: the time defaults to 09:00 when nothing parses.
const DEFAULT_HOUR: u32 = 9;
const DEFAULT_MINUTE: u32 = 0;

/// Which dialog field a picker edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DatePickerTarget {
    /// Task due date in the create dialog (`YYYY-MM-DD`, web `DatePicker`).
    TaskDue,
    /// Task due date in the detail dialog.
    TaskDetailDue,
    /// Event start (`YYYY-MM-DDTHH:MM`, web `DateTimePicker`).
    EventStart,
    /// Event end (`YYYY-MM-DDTHH:MM`, web `DateTimePicker`).
    EventEnd,
}

impl DatePickerTarget {
    fn with_time(self) -> bool {
        !matches!(self, Self::TaskDue | Self::TaskDetailDue)
    }

    fn selector(self) -> &'static str {
        match self {
            Self::TaskDue => "desktop-task-due-picker",
            Self::TaskDetailDue => "desktop-task-detail-due-picker",
            Self::EventStart => "desktop-event-start-picker",
            Self::EventEnd => "desktop-event-end-picker",
        }
    }

    fn aria_label(self) -> &'static str {
        match self {
            Self::TaskDue | Self::TaskDetailDue => "Task due date",
            Self::EventStart => "Event start",
            Self::EventEnd => "Event end",
        }
    }
}

/// An open popover: which field, and the month in view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DatePickerOpen {
    pub target: DatePickerTarget,
    pub year: i32,
    pub month: u32,
}

/// Web `date_picker::parse_iso`: `YYYY-MM-DD` → `(year, month, day)`.
pub fn parse_iso_date(s: &str) -> Option<(i32, u32, u32)> {
    let mut parts = s.trim().split('-');
    let y = parts.next()?.parse::<i32>().ok()?;
    let m = parts.next()?.parse::<u32>().ok()?;
    let d = parts.next()?.parse::<u32>().ok()?;
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then_some((y, m, d))
}

/// Web `date_time_picker::parse_iso_datetime`: `YYYY-MM-DDTHH:MM[:SS]`.
pub fn parse_iso_datetime(s: &str) -> Option<(i32, u32, u32, u32, u32)> {
    let (date, time) = s.trim().split_once('T')?;
    let (y, m, d) = parse_iso_date(date)?;
    let (h, mi) = time.split_once(':')?;
    let mi = mi.split(':').next()?;
    let h = h.parse::<u32>().ok().filter(|h| *h < 24)?;
    let mi = mi.parse::<u32>().ok().filter(|x| *x < 60)?;
    Some((y, m, d, h, mi))
}

/// Weekday (0 = Sunday) of the 1st of the month.
pub fn first_weekday(year: i32, month: u32) -> u32 {
    NaiveDate::from_ymd_opt(year, month, 1).map_or(0, |d| d.weekday().num_days_from_sunday())
}

/// Web trigger labels: the value, or the placeholder.
pub fn trigger_label(with_time: bool, value: &str) -> String {
    if with_time {
        match parse_iso_datetime(value) {
            Some((y, m, d, h, mi)) => format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}"),
            None => "Select date & time".to_string(),
        }
    } else if value.trim().is_empty() {
        "Select date".to_string()
    } else {
        value.trim().to_string()
    }
}

/// Month after stepping `delta` (±1) from `(year, month)`.
pub fn step_month(year: i32, month: u32, delta: i32) -> (i32, u32) {
    let index = year * 12 + (i32::try_from(month).unwrap_or(1) - 1) + delta;
    (index.div_euclid(12), (index.rem_euclid(12) + 1) as u32)
}

fn focus_ring(style: StyleRefinement) -> StyleRefinement {
    style.border_2().border_color(theme::persimmon_400())
}

fn label(text: impl Into<SharedString>, size: f32, color: Hsla) -> Div {
    div()
        // Web `text-label-sm` is tracked .025em; the wider Argon instance matches it.
        .when(size <= 12.0, |el| {
            el.font_family(theme::UI_SMALL_FONT_FAMILY)
        })
        .text_size(px(size))
        .font_weight(FontWeight::MEDIUM)
        .line_height(relative(1.5))
        .text_color(color)
        .child(text.into())
}

impl Workspace {
    fn date_picker_input(&self, target: DatePickerTarget) -> &Entity<InputState> {
        match target {
            DatePickerTarget::TaskDue => &self.task_create_due_input,
            DatePickerTarget::TaskDetailDue => &self.task_detail_due_input,
            DatePickerTarget::EventStart => &self.event_create_start_input,
            DatePickerTarget::EventEnd => &self.event_create_end_input,
        }
    }

    fn date_picker_value(&self, target: DatePickerTarget, cx: &App) -> String {
        self.date_picker_input(target).read(cx).value().to_string()
    }

    fn set_date_picker_value(
        &mut self,
        target: DatePickerTarget,
        value: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.date_picker_input(target)
            .clone()
            .update(cx, |input, cx| input.set_value(value.clone(), window, cx));
        // `set_value` does not emit `InputEvent::Change`, so the detail dialog's
        // live save (web `on_due`) runs here.
        if target == DatePickerTarget::TaskDetailDue {
            self.update_selected_work_item_due_date(value, cx);
        }
        cx.notify();
    }

    /// Trigger click: open on the value's month (else today) with the
    /// value's time (else 09:00), or close if this picker is already open.
    pub(super) fn toggle_date_picker(
        &mut self,
        target: DatePickerTarget,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.date_picker.is_some_and(|open| open.target == target) {
            self.date_picker = None;
            cx.notify();
            return;
        }
        let value = self.date_picker_value(target, cx);
        let today = Local::now().date_naive();
        let (year, month) = if target.with_time() {
            parse_iso_datetime(&value).map_or((today.year(), today.month()), |(y, m, ..)| (y, m))
        } else {
            parse_iso_date(&value).map_or((today.year(), today.month()), |(y, m, _)| (y, m))
        };
        let (hour, minute) = parse_iso_datetime(&value)
            .map_or((DEFAULT_HOUR, DEFAULT_MINUTE), |(.., h, mi)| (h, mi));
        self.date_picker_hour_input.update(cx, |input, cx| {
            input.set_value(format!("{hour:02}"), window, cx)
        });
        self.date_picker_minute_input.update(cx, |input, cx| {
            input.set_value(format!("{minute:02}"), window, cx)
        });
        self.date_picker = Some(DatePickerOpen {
            target,
            year,
            month,
        });
        cx.notify();
    }

    pub(super) fn close_date_picker(&mut self, cx: &mut Context<'_, Self>) {
        if self.date_picker.take().is_some() {
            cx.notify();
        }
    }

    fn step_date_picker_month(&mut self, delta: i32, cx: &mut Context<'_, Self>) {
        if let Some(open) = self.date_picker.as_mut() {
            let (year, month) = step_month(open.year, open.month, delta);
            open.year = year;
            open.month = month;
            cx.notify();
        }
    }

    fn date_picker_time(&self, cx: &App) -> (u32, u32) {
        let hour = self
            .date_picker_hour_input
            .read(cx)
            .value()
            .trim()
            .parse::<u32>()
            .map_or(DEFAULT_HOUR, |h| h.min(23));
        let minute = self
            .date_picker_minute_input
            .read(cx)
            .value()
            .trim()
            .parse::<u32>()
            .map_or(DEFAULT_MINUTE, |m| m.min(59));
        (hour, minute)
    }

    /// Day cell click: web `pick` — the date picker closes, the date-time
    /// picker keeps the popover open for the time fields.
    fn pick_date_picker_day(&mut self, day: u32, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(open) = self.date_picker else {
            return;
        };
        let (y, m) = (open.year, open.month);
        if open.target.with_time() {
            let (h, mi) = self.date_picker_time(cx);
            self.set_date_picker_value(
                open.target,
                format!("{y:04}-{m:02}-{day:02}T{h:02}:{mi:02}"),
                window,
                cx,
            );
        } else {
            self.set_date_picker_value(open.target, format!("{y:04}-{m:02}-{day:02}"), window, cx);
            self.date_picker = None;
            cx.notify();
        }
    }

    /// Hour / minute field input: web `emit_with_time` — keeps the value's
    /// date (else today) and rewrites the time.
    pub(super) fn date_picker_time_changed(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(open) = self.date_picker else {
            return;
        };
        if !open.target.with_time() {
            return;
        }
        let (h, mi) = self.date_picker_time(cx);
        let value = self.date_picker_value(open.target, cx);
        let today = Local::now().date_naive();
        let (y, m, d) = parse_iso_datetime(&value).map_or(
            (today.year(), today.month(), today.day()),
            |(y, m, d, ..)| (y, m, d),
        );
        self.set_date_picker_value(
            open.target,
            format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}"),
            window,
            cx,
        );
    }

    /// The trigger (and, while open, the popover under it).
    pub(super) fn render_date_field(
        &self,
        target: DatePickerTarget,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let value = self.date_picker_value(target, cx);
        let text = trigger_label(target.with_time(), &value);
        let selector = target.selector();
        let open = self.date_picker.filter(|open| open.target == target);
        let trigger = div()
            .id(selector)
            .debug_selector(move || selector.to_string())
            .max_w_full()
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(GRAPE_RADIUS_PX))
            .bg(gpui::white().opacity(0.05))
            .border_1()
            .border_color(gpui::white().opacity(0.10))
            .px_3()
            .py_2()
            .font_family(theme::CODE_FONT_FAMILY)
            .text_size(px(TRIGGER_TEXT_PX))
            .line_height(relative(1.5))
            .text_color(theme::rambutan_100())
            .role(Role::Button)
            .aria_label(target.aria_label())
            .aria_expanded(open.is_some())
            .tab_index(0)
            .focus_visible(focus_ring)
            .cursor_pointer()
            .hover(|el| el.border_color(gpui::white().opacity(0.20)))
            .on_click(cx.listener(move |this, _ev, window, cx| {
                this.toggle_date_picker(target, window, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.toggle_date_picker(target, window, cx);
                }
            }))
            .child(
                svg()
                    .path(assets::CALENDAR_ICON_PATH)
                    .size(px(TRIGGER_ICON_PX))
                    .flex_shrink_0()
                    .text_color(theme::rambutan_100()),
            )
            .child(div().min_w_0().child(text));
        div()
            .relative()
            .flex()
            .flex_col()
            .items_start()
            .child(trigger)
            .when_some(open, |el, open| {
                // A zero-height slot keeps the popover in flow right under the
                // trigger (web `top-full mt-1`); deferred so it paints over the
                // fields below it.
                el.child(
                    div().relative().h_0().w_full().child(
                        deferred(self.render_date_picker_popover(open, cx)).with_priority(1),
                    ),
                )
            })
            .into_any_element()
    }

    fn render_date_picker_popover(
        &self,
        open: DatePickerOpen,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let DatePickerOpen {
            target,
            year,
            month,
        } = open;
        let value = self.date_picker_value(target, cx);
        let selected = if target.with_time() {
            parse_iso_datetime(&value).map(|(y, m, d, ..)| (y, m, d))
        } else {
            parse_iso_date(&value)
        };
        let lead = first_weekday(year, month) as usize;
        // The open month is always 1–12; 30 is never used.
        let total = days_in_month(year, month).unwrap_or(30);
        let mut cells: Vec<Option<u32>> = vec![None; lead];
        cells.extend((1..=total).map(Some));
        while !cells.len().is_multiple_of(7) {
            cells.push(None);
        }

        let month_button = |id: &'static str, glyph: &'static str, delta: i32| {
            div()
                .id(id)
                .debug_selector(move || id.to_string())
                .size(px(MONTH_BUTTON_PX))
                .rounded(px(SEED_RADIUS_PX))
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme::rambutan_300())
                .role(Role::Button)
                .aria_label(if delta < 0 {
                    "Previous month"
                } else {
                    "Next month"
                })
                .tab_index(0)
                .focus_visible(focus_ring)
                .cursor_pointer()
                .hover(|el| el.bg(theme::longan_800()))
                .on_click(cx.listener(move |this, _ev, _window, cx| {
                    this.step_date_picker_month(delta, cx);
                }))
                .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                    if is_web_button_activation_key(event.keystroke.key.as_str()) {
                        window.prevent_default();
                        cx.stop_propagation();
                        this.step_date_picker_month(delta, cx);
                    }
                }))
                .child(glyph)
        };

        let mut weekday_row = div().flex().gap(px(GRID_GAP_PX)).mb_1();
        for day in WEEKDAYS {
            weekday_row = weekday_row.child(
                div()
                    .flex_1()
                    .h(px(CELL_PX))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(label(day, LABEL_SM_PX, theme::rambutan_500())),
            );
        }

        let mut grid = div().flex().flex_col().gap(px(GRID_GAP_PX));
        for (row_ix, week) in cells.chunks(7).enumerate() {
            let mut row = div().flex().gap(px(GRID_GAP_PX));
            for (col_ix, cell) in week.iter().enumerate() {
                let slot = div()
                    .flex_1()
                    .h(px(CELL_PX))
                    .flex()
                    .items_center()
                    .justify_center();
                let Some(day) = *cell else {
                    row = row.child(slot);
                    continue;
                };
                let is_selected = selected == Some((year, month, day));
                let cell_ix = row_ix * 7 + col_ix;
                row = row.child(
                    slot.child(
                        div()
                            .id(("desktop-date-picker-day", cell_ix))
                            .debug_selector(move || {
                                format!("desktop-date-picker-day-{year:04}-{month:02}-{day:02}")
                            })
                            .size(px(CELL_PX))
                            .rounded(px(SEED_RADIUS_PX))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(LABEL_PX))
                            .font_weight(FontWeight::MEDIUM)
                            .map(|el| {
                                if is_selected {
                                    el.bg(theme::persimmon_500())
                                        .text_color(theme::longan_950())
                                } else {
                                    el.text_color(theme::rambutan_200())
                                        .hover(|el| el.bg(theme::longan_800()))
                                }
                            })
                            .role(Role::Button)
                            .aria_label(format!("{} {day}, {year}", MONTHS[(month - 1) as usize]))
                            .tab_index(0)
                            .focus_visible(focus_ring)
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _ev, window, cx| {
                                this.pick_date_picker_day(day, window, cx);
                            }))
                            .on_key_down(cx.listener(
                                move |this, event: &KeyDownEvent, window, cx| {
                                    if is_web_button_activation_key(event.keystroke.key.as_str()) {
                                        window.prevent_default();
                                        cx.stop_propagation();
                                        this.pick_date_picker_day(day, window, cx);
                                    }
                                },
                            ))
                            .child(day.to_string()),
                    ),
                );
            }
            grid = grid.child(row);
        }

        let time_field = |input: &Entity<InputState>, aria: &'static str| {
            div()
                .w(px(TIME_FIELD_WIDTH_PX))
                .rounded(px(SEED_RADIUS_PX))
                .bg(theme::longan_900())
                .border_1()
                .border_color(theme::longan_700())
                .px(px(6.0))
                .py(px(2.0))
                .text_size(px(LABEL_PX))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::rambutan_200())
                .child(
                    Input::new(input)
                        .appearance(false)
                        .text_center()
                        .w_full()
                        .aria_label(aria),
                )
        };

        div()
            .id(SharedString::from(format!("{}-popover", target.selector())))
            .debug_selector(|| "desktop-date-picker-popover".to_string())
            .occlude()
            .absolute()
            .left_0()
            .top(px(POPOVER_GAP_PX))
            .w(px(POPOVER_WIDTH_PX))
            .rounded(px(GRAPE_RADIUS_PX))
            .bg(theme::longan_900())
            .border_1()
            .border_color(theme::longan_700())
            .p_3()
            .flex()
            .flex_col()
            .on_mouse_down_out(cx.listener(|this, _ev, _window, cx| this.close_date_picker(cx)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .mb_2()
                    .child(month_button("desktop-date-picker-prev", "‹", -1))
                    .child(label(
                        format!("{} {year}", MONTHS[(month - 1) as usize]),
                        LABEL_PX,
                        theme::rambutan_100(),
                    ))
                    .child(month_button("desktop-date-picker-next", "›", 1)),
            )
            .child(weekday_row)
            .child(grid)
            .when(target.with_time(), |el| {
                el.child(
                    div()
                        .mt_3()
                        .pt_3()
                        .border_t_1()
                        .border_color(theme::longan_800().opacity(0.6))
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(6.0))
                        .text_color(theme::rambutan_400())
                        .child(label("TIME", LABEL_SM_PX, theme::rambutan_400()))
                        .child(time_field(&self.date_picker_hour_input, "Hour"))
                        .child(label(":", LABEL_PX, theme::rambutan_400()))
                        .child(time_field(&self.date_picker_minute_input, "Minute")),
                )
            })
            .into_any_element()
    }
}

impl Workspace {
    /// The date-time picker's hour and minute fields.
    pub(super) fn new_date_picker_inputs(
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> (Entity<InputState>, Entity<InputState>) {
        let date_picker_hour_input = cx.new(|cx| InputState::new(window, cx));
        let date_picker_minute_input = cx.new(|cx| InputState::new(window, cx));
        for input in [&date_picker_hour_input, &date_picker_minute_input] {
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                |this: &mut Self, _input, ev: &InputEvent, window, cx| {
                    if matches!(ev, InputEvent::Change) {
                        this.date_picker_time_changed(window, cx);
                    }
                },
            ));
        }
        (date_picker_hour_input, date_picker_minute_input)
    }
}

#[cfg(test)]
mod tests {
    use super::{first_weekday, parse_iso_date, parse_iso_datetime, step_month, trigger_label};

    #[test]
    fn iso_parsers_match_web() {
        assert_eq!(parse_iso_date("2026-09-06"), Some((2026, 9, 6)));
        assert_eq!(parse_iso_date("2026-13-06"), None);
        assert_eq!(
            parse_iso_datetime("2026-09-06T09:30"),
            Some((2026, 9, 6, 9, 30))
        );
        assert_eq!(
            parse_iso_datetime("2026-09-06T09:30:15"),
            Some((2026, 9, 6, 9, 30))
        );
        assert_eq!(parse_iso_datetime("2026-09-06T24:00"), None);
        assert_eq!(parse_iso_datetime("2026-09-06"), None);
    }

    #[test]
    fn calendar_math_handles_leap_years_and_wraps() {
        // 1 September 2026 is a Tuesday.
        assert_eq!(first_weekday(2026, 9), 2);
        assert_eq!(step_month(2026, 12, 1), (2027, 1));
        assert_eq!(step_month(2026, 1, -1), (2025, 12));
    }

    #[test]
    fn trigger_labels_match_web() {
        assert_eq!(trigger_label(false, ""), "Select date");
        assert_eq!(trigger_label(false, "2026-09-06"), "2026-09-06");
        assert_eq!(trigger_label(true, ""), "Select date & time");
        assert_eq!(trigger_label(true, "2026-09-06T00:00"), "2026-09-06 00:00");
    }
}
