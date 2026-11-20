//! Web `ui::DatePicker` / `ui::DateTimePicker`: a trigger button (calendar
//! glyph + value) that opens a month-grid popover; the date-time variant
//! adds hour / minute fields under the grid. Values stay in the dialogs'
//! input states in the web's wire formats (`YYYY-MM-DD`,
//! `YYYY-MM-DDTHH:MM`), so the create / update paths are unchanged.

use bruhs_ui::input::{Input, InputEvent, InputState};
use bruhs_ui::tooltip::Tooltip;
use bruhs_ui::zoom::px;
use chrono::{Datelike, Local, NaiveDate};
use gpui::prelude::*;
use gpui::*;

use super::Workspace;
use crate::{assets, theme};
use bruhs_models::time::days_in_month;
use bruhs_ui::press::on_press_view;

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

const TRIGGER_TEXT_PX: f32 = theme::APP_INPUT_TEXT_SIZE_PX;
const TRIGGER_ICON_PX: f32 = theme::ICON_PX;
const POPOVER_WIDTH_PX: f32 = 256.0;
const POPOVER_GAP_PX: f32 = 4.0;
/// Web `disabled:opacity-60` on the date-time trigger.
const DATE_FIELD_DISABLED_OPACITY: f32 = 0.6;
const CELL_PX: f32 = theme::APP_COMPACT_CONTROL_HEIGHT_PX;
const GRID_GAP_PX: f32 = 2.0;
const MONTH_BUTTON_PX: f32 = theme::CONTROL_SM_PX;
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
    /// The custom snooze's date and time (`YYYY-MM-DDTHH:MM`).
    SnoozeUntil,
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
            Self::SnoozeUntil => "desktop-snooze-until-picker",
        }
    }

    fn aria_label(self) -> &'static str {
        match self {
            Self::TaskDue | Self::TaskDetailDue => "Task due date",
            Self::EventStart => "Event start",
            Self::EventEnd => "Event end",
            Self::SnoozeUntil => "Snooze until",
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
    style.border_color(theme::persimmon_400())
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

/// The shared month navigator. A supplied focus handle makes the current day
/// the single tab stop in the day grid.
pub(crate) fn month_grid<V: 'static>(
    year: i32,
    month: u32,
    selected: Option<(i32, u32, u32)>,
    navigation: Option<(&FocusHandle, AnyElement)>,
    cx: &Context<'_, V>,
    step: impl Fn(&mut V, i32, &mut Window, &mut Context<'_, V>) + Copy + 'static,
    pick: impl Fn(&mut V, NaiveDate, &mut Window, &mut Context<'_, V>) + Copy + 'static,
) -> Div {
    let lead = first_weekday(year, month) as usize;
    // The open month is always 1–12; 30 is never used.
    let total = days_in_month(year, month).unwrap_or(30);
    let slots = if navigation.is_some() {
        42
    } else {
        (lead + total as usize).div_ceil(7) * 7
    };
    let first = NaiveDate::from_ymd_opt(year, month, 1);
    let cells: Vec<_> = (0..slots)
        .map(|i| {
            let date = first?.checked_add_signed(chrono::Duration::days(i as i64 - lead as i64))?;
            if navigation.is_some() {
                (1..=9999).contains(&date.year()).then_some(date)
            } else {
                (date.month() == month).then_some(date)
            }
        })
        .collect();
    let (day_focus, title) = match navigation {
        Some((focus, title)) => (Some(focus), title),
        None => (
            None,
            label(
                format!("{} {year}", MONTHS[(month - 1) as usize]),
                LABEL_PX,
                theme::rambutan_100(),
            )
            .into_any_element(),
        ),
    };

    let month_button = |id: &'static str, path: &'static str, delta: i32| {
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
            .border_1()
            .border_color(gpui::transparent_black())
            .focus_visible(focus_ring)
            .cursor_pointer()
            .hover(|el| el.bg(theme::longan_800()))
            .map(|el| {
                on_press_view(el, cx, move |this, window, cx| {
                    step(this, delta, window, cx);
                })
            })
            .child(
                svg()
                    .path(path)
                    .size(px(14.0))
                    .text_color(theme::rambutan_300()),
            )
    };

    let mut weekday_row = div().flex().gap(px(GRID_GAP_PX)).mb_1();
    for day in WEEKDAYS {
        weekday_row = weekday_row.child(
            div()
                .flex_1()
                .h(px(LABEL_SM_PX * 1.5))
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
            let Some(date) = *cell else {
                row = row.child(slot);
                continue;
            };
            let day = date.day();
            let adjacent = date.month() != month;
            let is_selected = selected == Some((date.year(), date.month(), day));
            let cell_ix = row_ix * 7 + col_ix;
            row = row.child(
                slot.child(
                    div()
                        .id(("desktop-date-picker-day", cell_ix))
                        .debug_selector(move || format!("desktop-date-picker-day-{date}"))
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
                                el.text_color(if adjacent {
                                    theme::rambutan_500()
                                } else {
                                    theme::rambutan_200()
                                })
                                .hover(|el| el.bg(theme::longan_800()))
                            }
                        })
                        .role(Role::Button)
                        .aria_label(format!(
                            "{} {day}, {}",
                            MONTHS[date.month0() as usize],
                            date.year()
                        ))
                        .aria_selected(is_selected)
                        .tab_index(0)
                        .when_some(day_focus, |el, handle| {
                            let cursor = !adjacent
                                && selected.map_or(1, |(_, _, day)| day.min(total)) == day;
                            el.tab_stop(cursor)
                                .when(cursor, |el| el.track_focus(&handle.clone().tab_stop(true)))
                        })
                        .border_1()
                        .border_color(gpui::transparent_black())
                        .focus_visible(focus_ring)
                        .cursor_pointer()
                        .map(|el| {
                            on_press_view(el, cx, move |this, window, cx| {
                                pick(this, date, window, cx);
                            })
                        })
                        .child(day.to_string()),
                ),
            );
        }
        grid = grid.child(row);
    }

    div()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .mb_2()
                .child(month_button(
                    "desktop-date-picker-prev",
                    assets::CHEVRON_LEFT_ICON_PATH,
                    -1,
                ))
                .child(title)
                .child(month_button(
                    "desktop-date-picker-next",
                    assets::CHEVRON_RIGHT_ICON_PATH,
                    1,
                )),
        )
        .child(weekday_row)
        .child(grid)
}

impl Workspace {
    fn date_picker_input(&self, target: DatePickerTarget) -> &Entity<InputState> {
        match target {
            DatePickerTarget::TaskDue => &self.task_create_due_input,
            DatePickerTarget::TaskDetailDue => &self.task_detail_due_input,
            DatePickerTarget::EventStart => &self.event_create_start_input,
            DatePickerTarget::EventEnd => &self.event_create_end_input,
            DatePickerTarget::SnoozeUntil => &self.snooze_inputs.when,
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
        // The field's name and its date, as the web's picker announces both.
        let name = format!("{}, {text}", target.aria_label());
        let tooltip = text.clone();
        let selector = target.selector();
        let open = self.date_picker.filter(|open| open.target == target);
        let align_right = !matches!(
            target,
            DatePickerTarget::EventStart | DatePickerTarget::SnoozeUntil
        );
        // A read-only provider event shows its times without a picker.
        let disabled = matches!(
            target,
            DatePickerTarget::EventStart | DatePickerTarget::EventEnd
        ) && self.event_dialog_read_only();
        // Web: the trigger fits its text and never outgrows its field.
        let trigger = div()
            .id(selector)
            .debug_selector(move || selector.to_string())
            .max_w_full()
            .min_w_0()
            .min_h(px(theme::APP_CONTROL_HEIGHT_PX))
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(GRAPE_RADIUS_PX))
            .bg(gpui::white().opacity(0.05))
            .border_1()
            .border_color(gpui::white().opacity(0.10))
            .px_3()
            .py_1()
            .font_family(theme::CODE_FONT_FAMILY)
            .text_size(px(TRIGGER_TEXT_PX))
            .line_height(relative(1.5))
            .text_color(theme::rambutan_100())
            .role(Role::Button)
            .aria_label(name)
            .aria_expanded(open.is_some())
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .when(disabled, |el| el.opacity(DATE_FIELD_DISABLED_OPACITY))
            .when(!disabled, |el| {
                el.tab_index(0)
                    .focus_visible(focus_ring)
                    .cursor_pointer()
                    .hover(|el| el.border_color(gpui::white().opacity(0.20)))
                    .map(|el| {
                        on_press_view(el, cx, move |this, window, cx| {
                            this.toggle_date_picker(target, window, cx);
                        })
                    })
            })
            .child(
                svg()
                    .path(assets::CALENDAR_ICON_PATH)
                    .size(px(TRIGGER_ICON_PX))
                    .flex_shrink_0()
                    .text_color(theme::rambutan_100()),
            )
            .child(div().min_w_0().truncate().child(text));
        div()
            .relative()
            .flex()
            .flex_col()
            .items_start()
            .child(trigger)
            .when_some(open, |el, open| {
                el.child(
                    div().relative().h_0().w_full().child(
                        div()
                            .absolute()
                            .top_0()
                            .size_0()
                            .when(align_right, |el| el.right_0())
                            .when(!align_right, |el| el.left_0())
                            .child(
                                deferred(
                                    anchored()
                                        .anchor(if align_right {
                                            Anchor::TopRight
                                        } else {
                                            Anchor::TopLeft
                                        })
                                        .offset(point(px(0.0), px(POPOVER_GAP_PX)))
                                        .snap_to_window_with_margin(Edges {
                                            top: px(theme::APP_HEADER_HEIGHT_PX + 8.0),
                                            right: px(8.0),
                                            bottom: px(8.0),
                                            left: px(8.0),
                                        })
                                        .child(self.render_date_picker_popover(open, cx)),
                                )
                                .with_priority(1),
                            ),
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
        let time_field = |input: &Entity<InputState>, aria: &'static str| {
            div()
                .w(px(TIME_FIELD_WIDTH_PX))
                .rounded(px(SEED_RADIUS_PX))
                .bg(theme::longan_900())
                .border_1()
                .border_color(theme::longan_700())
                // Web `px-1.5`, less the 1px border.
                .px(px(5.0))
                .py(px(2.0))
                .text_size(px(LABEL_PX))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::rambutan_200())
                .child(
                    Input::new(input)
                        .appearance(false)
                        // Web `px-1.5` is the wrapper's; the field has none.
                        .px_0()
                        .text_center()
                        .w_full()
                        .aria_label(aria),
                )
        };

        div()
            .id(SharedString::from(format!("{}-popover", target.selector())))
            .debug_selector(|| "desktop-date-picker-popover".to_string())
            .occlude()
            .w(px(POPOVER_WIDTH_PX))
            .rounded(px(GRAPE_RADIUS_PX))
            .bg(theme::longan_900())
            .border_1()
            .border_color(theme::longan_700())
            .p_3()
            .flex()
            .flex_col()
            .child(self.blur_scrim.keep_clear())
            .on_mouse_down_out(cx.listener(|this, _ev, _window, cx| this.close_date_picker(cx)))
            .child(month_grid(
                year,
                month,
                selected,
                None,
                cx,
                |this, delta, _, cx| this.step_date_picker_month(delta, cx),
                |this, date, window, cx| this.pick_date_picker_day(date.day(), window, cx),
            ))
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
