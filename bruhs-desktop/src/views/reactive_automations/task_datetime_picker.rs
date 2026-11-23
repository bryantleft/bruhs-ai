//! Native date and time controls for scheduled tasks. The text fields remain
//! editable for explicit UTC offsets and schedules with several daily times.
use super::*;
use crate::views::workspace::date_picker_ui::{month_grid, step_month, MONTHS};
use bruhs_ui::menu_popover::menu_popover;
use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use std::cell::Cell;

#[path = "task_datetime_picker/segments.rs"]
mod segments;

const CLOCK_ROW_HEIGHT: f32 = 25.;

#[derive(Clone, Copy)]
enum ClockPart {
    Hour,
    Minute,
    Period,
}

impl ClockPart {
    fn index(self) -> usize {
        self as usize
    }
    fn label(self) -> &'static str {
        match self {
            Self::Hour => "Hour",
            Self::Minute => "Minute",
            Self::Period => "AM/PM",
        }
    }
    fn count(self) -> u32 {
        match self {
            Self::Hour if crate::clock_format::uses_24_hours() => 24,
            Self::Hour => 12,
            Self::Minute => 60,
            Self::Period => 2,
        }
    }
    fn value(self, dt: NaiveDateTime) -> u32 {
        match self {
            Self::Hour => dt.hour() % self.count(),
            Self::Minute => dt.minute(),
            Self::Period => dt.hour() / 12,
        }
    }
    fn option(self, n: u32) -> String {
        match self {
            Self::Period => if n == 0 { "AM" } else { "PM" }.to_string(),
            Self::Hour if self.count() == 12 && n == 0 => "12".into(),
            _ => format!("{n:02}"),
        }
    }
}

#[derive(Clone, Copy)]
struct OpenPicker {
    value: NaiveDateTime,
    // The native time lists open on the selected hour and minute, then wrap.
    initial: NaiveDateTime,
    months_open: bool,
    month: (i32, u32),
}

pub(super) struct SchedulePicker {
    open: Option<OpenPicker>,
    original_text: String,
    segments: segments::Segments,
    trigger: FocusHandle,
    popup: FocusHandle,
    month: FocusHandle,
    day: FocusHandle,
    clock: [FocusHandle; 3],
    scroll: [ScrollHandle; 3],
}

impl SchedulePicker {
    pub(super) fn new(cx: &mut App) -> Self {
        Self {
            open: None,
            original_text: String::new(),
            segments: segments::Segments::new(cx),
            trigger: cx.focus_handle().tab_stop(true),
            popup: cx.focus_handle(),
            month: cx.focus_handle().tab_stop(true),
            day: cx.focus_handle(),
            clock: std::array::from_fn(|_| cx.focus_handle().tab_stop(true)),
            scroll: std::array::from_fn(|_| ScrollHandle::new()),
        }
    }
    pub(super) fn in_text_mode(&self) -> bool {
        self.segments.text_mode
    }

    pub(super) fn use_text_entry(&mut self, enabled: bool) {
        self.segments.use_text_entry(enabled);
    }
    pub(super) fn close(&mut self) {
        self.open = None;
    }
}

impl AutomationEditor {
    fn local_once_value(&self, cx: &App) -> Option<NaiveDateTime> {
        let text = value(&self.source.value, cx);
        task_schedule::parse_local_minute(&text).or_else(|| {
            chrono::DateTime::parse_from_rfc3339(text.trim())
                .ok()
                .map(|time| time.with_timezone(&Local).naive_local())
        })
    }

    fn picker_value(&self, cx: &App) -> NaiveDateTime {
        let now = Local::now().naive_local();
        if self.source.kind == "once" {
            return self.local_once_value(cx).unwrap_or(now);
        }
        let times = value(&self.source.times, cx);
        let first = times.split(',').next().unwrap_or_default().trim();
        let time = task_schedule::parse_clock(first)
            .unwrap_or_else(|| NaiveTime::from_hms_opt(9, 0, 0).expect("09:00:00 is valid"));
        now.date().and_time(time)
    }

    fn open_picker(&mut self, window: &mut Window, cx: &mut App) {
        let dt = self.picker_value(cx);
        let picker = &mut self.date_time_picker;
        picker.original_text = if self.source.kind == "once" {
            value(&self.source.value, cx)
        } else {
            value(&self.source.times, cx)
        };
        picker.open = Some(OpenPicker {
            value: dt,
            initial: dt,
            months_open: false,
            month: (dt.year(), dt.month()),
        });
        for scroll in &picker.scroll {
            scroll.set_offset(point(px(0.), px(0.)));
        }
        let focus = if self.source.kind == "once" {
            &picker.day
        } else {
            &picker.clock[0]
        };
        focus.focus(window, cx);
    }

    fn pick_datetime(&mut self, dt: NaiveDateTime, window: &mut Window, cx: &mut App) {
        let Some(picker) = &mut self.date_time_picker.open else {
            return;
        };
        picker.value = dt;
        picker.month = (dt.year(), dt.month());
        if self.source.kind == "once" {
            // Opening or selecting the saved local minute preserves its precise
            // instant (including the UTC offset during a repeated hour).
            if self.local_once_value(cx) != Some(dt) {
                self.source
                    .value
                    .set_value(task_schedule::format_local_minute(dt), window, cx);
            }
        } else {
            let previous = value(&self.source.times, cx);
            let mut time = dt.format(crate::clock_format::pattern()).to_string();
            if let Some((_, rest)) = previous.split_once(',') {
                time.push(',');
                time.push_str(rest);
            }
            self.source.times.set_text_value(time.into(), window, cx);
        }
    }
}

impl ReactiveAutomations {
    fn schedule_editor_mut(&mut self) -> Option<&mut AutomationEditor> {
        match &mut self.editor {
            Some(Editor::Automation(editor)) if editor.scheduled_form => Some(editor),
            _ => None,
        }
    }

    pub(in crate::views::reactive_automations) fn close_schedule_picker(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let Some(editor) = self.schedule_editor_mut() else {
            return false;
        };
        if editor.date_time_picker.open.take().is_none() {
            return false;
        }
        editor.date_time_picker.trigger.focus(window, cx);
        cx.notify();
        true
    }

    pub(in crate::views::reactive_automations) fn escape_schedule_picker(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let Some(editor) = self.schedule_editor_mut() else {
            return false;
        };
        let picker = &mut editor.date_time_picker;
        let Some(open) = &mut picker.open else {
            return false;
        };
        if open.months_open {
            open.months_open = false;
            open.month = (open.value.year(), open.value.month());
            picker.day.focus(window, cx);
            cx.notify();
            return true;
        }
        if open.value != open.initial {
            open.value = open.initial;
            open.month = (open.initial.year(), open.initial.month());
            for scroll in &picker.scroll {
                scroll.set_offset(point(px(0.), px(0.)));
            }
            if editor.source.kind == "once" {
                editor
                    .source
                    .value
                    .set_value(picker.original_text.clone(), window, cx);
            } else {
                editor
                    .source
                    .times
                    .set_text_value(picker.original_text.clone().into(), window, cx);
            }
            cx.notify();
            return true;
        }
        self.close_schedule_picker(window, cx)
    }

    pub(crate) fn dismiss_unfocused_schedule_picker(
        &mut self,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(editor) = self.schedule_editor_mut() {
            let picker = &mut editor.date_time_picker;
            if picker.open.is_some() && !picker.popup.contains_focused(window, cx) {
                picker.close();
                cx.notify();
            }
        }
    }

    fn select_clock(
        &mut self,
        part: ClockPart,
        number: u32,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(editor) = self.schedule_editor_mut() else {
            return;
        };
        let Some(open) = editor.date_time_picker.open else {
            return;
        };
        let dt = match part {
            ClockPart::Hour if part.count() == 12 => {
                open.value.with_hour(open.value.hour() / 12 * 12 + number)
            }
            ClockPart::Hour => open.value.with_hour(number),
            ClockPart::Minute => open.value.with_minute(number),
            ClockPart::Period => open.value.with_hour(open.value.hour() % 12 + number * 12),
        };
        if let Some(dt) = dt {
            editor.pick_datetime(dt, window, cx);
            let picker = &editor.date_time_picker;
            picker.clock[part.index()].focus(window, cx);
            let start = part.value(open.initial);
            picker.scroll[part.index()]
                .scroll_to_item(((number + part.count() - start) % part.count()) as usize);
            cx.notify();
        }
    }

    fn clock_column(
        &self,
        editor: &AutomationEditor,
        open: OpenPicker,
        part: ClockPart,
        cx: &Context<'_, Self>,
    ) -> Div {
        let label = part.label();
        let current = part.value(open.value);
        let count = part.count();
        let id = match part {
            ClockPart::Hour => "schedule-hours",
            ClockPart::Minute => "schedule-minutes",
            ClockPart::Period => "schedule-period",
        };
        let handle = &editor.date_time_picker.clock[part.index()];
        let list = div()
            .id(id)
            .debug_selector(move || id.into())
            .track_focus(handle)
            .tab_index(0)
            .role(Role::ListBox)
            .aria_label(label)
            .h(px(if matches!(part, ClockPart::Period) {
                52.
            } else {
                152.
            }))
            .overflow_y_scroll()
            .track_scroll(&editor.date_time_picker.scroll[part.index()])
            .flex()
            .flex_col()
            .border_1()
            .border_color(theme::longan_700())
            .rounded(px(5.))
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                let key = event.keystroke.key.as_str();
                if matches!(key, "left" | "right") {
                    let index = part.index().cast_signed() + if key == "left" { -1 } else { 1 };
                    let columns = if crate::clock_format::uses_24_hours() {
                        2
                    } else {
                        3
                    };
                    if (0..columns).contains(&index) {
                        if let Some(editor) = this.schedule_editor_mut() {
                            editor.date_time_picker.clock[index.cast_unsigned()].focus(window, cx);
                        }
                    }
                    window.prevent_default();
                    cx.stop_propagation();
                    return;
                }
                let twelve_hour = matches!(part, ClockPart::Hour) && count == 12;
                let next = match key {
                    "up" => (current + count - 1) % count,
                    "down" => (current + 1) % count,
                    "home" if twelve_hour => 1,
                    "end" if twelve_hour => 0,
                    "home" => 0,
                    "end" => count - 1,
                    _ => return,
                };
                window.prevent_default();
                cx.stop_propagation();
                this.select_clock(part, next, window, cx);
            }))
            .children((0..count).map(|index| {
                let n = (part.value(open.initial) + index) % count;
                let key = format!("{id}-{n}");
                let option = part.option(n);
                div()
                    .id((id, n as usize))
                    .debug_selector(move || key)
                    .role(Role::ListBoxOption)
                    .aria_label(format!("{label} {option}"))
                    .aria_selected(n == current)
                    .tab_stop(false)
                    .h(px(CLOCK_ROW_HEIGHT))
                    .flex_shrink_0()
                    .flex()
                    .justify_center()
                    .items_center()
                    .text_size(px(13.))
                    .rounded(px(3.))
                    .cursor_pointer()
                    .when(n == current, |e| {
                        e.bg(theme::persimmon_500()).text_color(theme::longan_950())
                    })
                    .when(n != current, |e| e.hover(|e| e.bg(theme::longan_700())))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.select_clock(part, n, window, cx);
                    }))
                    .child(option)
            }));
        div()
            .w(px(52.))
            .flex()
            .flex_col()
            .gap_1()
            .child(muted(label))
            .child(list)
    }

    fn choose_schedule_month(
        &mut self,
        year: i32,
        month: u32,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(editor) = self.schedule_editor_mut() else {
            return;
        };
        let Some(open) = &mut editor.date_time_picker.open else {
            return;
        };
        if !(1..=9999).contains(&year) {
            return;
        }
        let Some(total) = bruhs_models::time::days_in_month(year, month) else {
            return;
        };
        let Some(date) = NaiveDate::from_ymd_opt(year, month, open.value.day().min(total)) else {
            return;
        };
        let value = date.and_time(open.value.time());
        let from_month_grid = open.months_open;
        open.months_open = false;
        editor.pick_datetime(value, window, cx);
        if from_month_grid {
            editor.date_time_picker.day.focus(window, cx);
        }
        cx.notify();
    }

    fn schedule_months(
        &self,
        editor: &AutomationEditor,
        open: OpenPicker,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        let (year, month) = open.month;
        let year_button =
            |delta: i32, id: &'static str, label: &'static str, icon: &'static str| {
                web_button(
                    id,
                    WebButtonVariant::Ghost,
                    WebButtonSize::IconSmall,
                    ButtonState::disabled_if(!(1..=9999).contains(&(year + delta))),
                )
                .debug_selector(move || String::from(id))
                .aria_label(label)
                .size(px(26.))
                .child(svg().path(icon).size(px(14.)))
                .on_activate(cx, move |this, _, cx| {
                    if let Some(open) = this
                        .schedule_editor_mut()
                        .and_then(|e| e.date_time_picker.open.as_mut())
                    {
                        open.month.0 = (year + delta).clamp(1, 9999);
                        cx.notify();
                    }
                })
            };
        let mut chooser = div()
            .id("schedule-months")
            .debug_selector(|| "schedule-months".into())
            .w(px(228.))
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(year_button(
                        -1,
                        "schedule-previous-year",
                        "Previous year",
                        crate::assets::CHEVRON_LEFT_ICON_PATH,
                    ))
                    .child(heading(year.to_string()))
                    .child(year_button(
                        1,
                        "schedule-next-year",
                        "Next year",
                        crate::assets::CHEVRON_RIGHT_ICON_PATH,
                    )),
            );
        for (row, names) in MONTHS.chunks(3).enumerate() {
            chooser = chooser.child(div().flex().gap_1().children(names.iter().enumerate().map(
                |(col, &name)| {
                    let m = u32::try_from(row * 3 + col + 1).unwrap_or(1);
                    ghost_control(("schedule-month", m as usize), &name[..3], false)
                        .debug_selector(move || format!("schedule-month-{m}"))
                        .aria_label(format!("{name} {year}"))
                        .flex_1()
                        .aria_selected(m == month)
                        .when(m == month, |el| {
                            el.track_focus(&editor.date_time_picker.month)
                                .bg(theme::longan_700())
                        })
                        .on_activate(cx, move |this, window, cx| {
                            this.choose_schedule_month(year, m, window, cx)
                        })
                },
            )));
        }
        chooser
    }

    fn schedule_calendar(
        &self,
        editor: &AutomationEditor,
        open: OpenPicker,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        let (year, month) = open.month;
        let dt = open.value;
        if open.months_open {
            return self.schedule_months(editor, open, cx);
        }
        let title = ghost_control(
            "schedule-select-month",
            &format!("{} {year}", MONTHS[month as usize - 1]),
            false,
        )
        .debug_selector(|| "schedule-select-month".into())
        .aria_label("Choose month and year")
        .aria_expanded(false)
        .child(
            svg()
                .path(crate::assets::CHEVRON_DOWN_ICON_PATH)
                .size(px(12.)),
        )
        .on_activate(cx, |this, window, cx| {
            if let Some(editor) = this.schedule_editor_mut() {
                if let Some(open) = &mut editor.date_time_picker.open {
                    open.months_open = true;
                    editor.date_time_picker.month.focus(window, cx);
                    cx.notify();
                }
            }
        });
        let grid = month_grid(
            year,
            month,
            Some((dt.year(), dt.month(), dt.day())),
            Some((&editor.date_time_picker.day, title.into_any_element())),
            cx,
            move |this, delta, window, cx| {
                let (year, month) = step_month(year, month, delta);
                this.choose_schedule_month(year, month, window, cx);
            },
            move |this, date, window, cx| {
                if let Some(editor) = this.schedule_editor_mut() {
                    editor.pick_datetime(date.and_time(dt.time()), window, cx);
                    editor.date_time_picker.day.focus(window, cx);
                    cx.notify();
                }
            },
        );
        div()
            .id("schedule-calendar-grid")
            .w(px(228.))
            .child(grid)
            .child(
                div()
                    .mt_1()
                    .flex()
                    .justify_between()
                    .child(
                        ghost_control("schedule-clear-date", "Clear", false)
                            .debug_selector(|| "schedule-clear-date".into())
                            .on_activate(cx, |this, window, cx| {
                                if let Some(editor) = this.schedule_editor_mut() {
                                    editor.source.value.set_value("", window, cx);
                                }
                                this.close_schedule_picker(window, cx);
                            }),
                    )
                    .child(
                        ghost_control("schedule-today", "Today", false)
                            .debug_selector(|| "schedule-today".into())
                            .on_activate(cx, move |this, window, cx| {
                                if let Some(editor) = this.schedule_editor_mut() {
                                    editor.pick_datetime(
                                        Local::now().date_naive().and_time(dt.time()),
                                        window,
                                        cx,
                                    );
                                    cx.notify();
                                }
                            }),
                    ),
            )
            .capture_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                let Some(editor) = this.schedule_editor_mut() else {
                    return;
                };
                if !editor.date_time_picker.day.is_focused(window) {
                    return;
                }
                let end = bruhs_models::time::days_in_month(year, month).unwrap_or(31);
                let Some(date) = NaiveDate::from_ymd_opt(year, month, dt.day().min(end)) else {
                    return;
                };
                let key = event.keystroke.key.as_str();
                let months = chrono::Months::new(if event.keystroke.modifiers.shift {
                    12
                } else {
                    1
                });
                let next = match key {
                    "left" => date.pred_opt(),
                    "right" => date.succ_opt(),
                    "up" => date.checked_sub_days(chrono::Days::new(7)),
                    "down" => date.checked_add_days(chrono::Days::new(7)),
                    "home" if date.day() == 1 => date.checked_sub_months(chrono::Months::new(1)),
                    "home" => date.with_day(1),
                    "end" if date.day() == end => date
                        .succ_opt()
                        .and_then(|first| first.checked_add_months(chrono::Months::new(1)))
                        .and_then(|next| next.pred_opt()),
                    "end" => date.with_day(end),
                    "pageup" => date.checked_sub_months(months),
                    "pagedown" => date.checked_add_months(months),
                    "enter" => Some(date),
                    _ => return,
                };
                window.prevent_default();
                cx.stop_propagation();
                if let Some(next) = next.filter(|date| (1..=9999).contains(&date.year())) {
                    editor.pick_datetime(next.and_time(dt.time()), window, cx);
                    if key == "enter" {
                        this.close_schedule_picker(window, cx);
                    }
                    cx.notify();
                }
            }))
    }

    pub(super) fn schedule_datetime_input(
        &self,
        editor: &AutomationEditor,
        cx: &Context<'_, Self>,
    ) -> Div {
        let once = editor.source.kind == "once";
        let (field, label, icon, width): (&dyn TextInput, _, _, _) = if once {
            (
                &editor.source.value,
                "Run on",
                crate::assets::CALENDAR_ICON_PATH,
                224.,
            )
        } else {
            let multiple = value(&editor.source.times, cx).contains(',');
            (
                &editor.source.times,
                "Run at",
                crate::assets::CLOCK_ICON_PATH,
                if multiple && editor.date_time_picker.in_text_mode() { 196. } else { 128. },
            )
        };
        let picker = &editor.date_time_picker;
        let trigger_bounds = Rc::new(Cell::new(Bounds::default()));
        let measured = Rc::clone(&trigger_bounds);
        let name = if once {
            "Choose date and time"
        } else {
            "Choose run time"
        };
        let toggle = web_button(
            "schedule-picker-toggle",
            WebButtonVariant::Ghost,
            WebButtonSize::IconSmall,
            ButtonState::disabled_if(self.busy),
        )
        .debug_selector(|| "schedule-picker-toggle".into())
        .size(px(26.))
        .aria_label(name)
        .aria_expanded(picker.open.is_some())
        .track_focus(&picker.trigger)
        .child(
            svg()
                .path(icon)
                .size(px(15.))
                .text_color(theme::rambutan_300()),
        )
        .on_activate(cx, |this, window, cx| {
            if !this.close_schedule_picker(window, cx) {
                if let Some(editor) = this.schedule_editor_mut() {
                    editor.open_picker(window, cx);
                    cx.notify();
                }
            }
        });
        let text_mode = picker.segments.text_mode;
        let field = if text_mode {
            field
                .text_element(label.into(), self.busy)
                .h(px(32.))
                .pr(px(31.))
                .text_size(px(14.))
                .into_any_element()
        } else {
            self.schedule_segments(editor, cx).into_any_element()
        };
        div()
            .relative()
            .w(px(width))
            .child(field)
            .child(
                div()
                    .absolute()
                    .right(px(3.))
                    .top(px(3.))
                    .on_children_prepainted(move |children, _, _| {
                        if let Some(bounds) = children.first() {
                            measured.set(*bounds);
                        }
                    })
                    .child(toggle),
            )
            .when_some(picker.open, |el, open| {
                let popup = div()
                    .id("schedule-datetime-popup")
                    .debug_selector(|| "schedule-datetime-popup".into())
                    .role(Role::Dialog)
                    .track_focus(&picker.popup)
                    .aria_label(name)
                    .occlude()
                    .p_2()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(theme::ring_floating())
                    .bg(theme::longan_900())
                    .flex()
                    .flex_col()
                    .gap_1()
                    .on_mouse_down_out(cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        if !trigger_bounds.get().contains(&event.position) {
                            if let Some(editor) = this.schedule_editor_mut() {
                                editor.date_time_picker.close();
                                cx.notify();
                            }
                        }
                    }))
                    .on_action(cx.listener(
                        |this, _: &bruhs_ui::actions::CloseOverlays, window, cx| {
                            cx.stop_propagation();
                            this.escape_schedule_picker(window, cx);
                        },
                    ))
                    .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                        if ev.keystroke.key == "enter"
                            && this.schedule_editor_mut().is_some_and(|editor| {
                                editor
                                    .date_time_picker
                                    .clock
                                    .iter()
                                    .any(|part| part.is_focused(window))
                            })
                        {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.close_schedule_picker(window, cx);
                        }
                    }))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .items_start()
                            .when(once, |el| {
                                el.child(self.schedule_calendar(editor, open, cx))
                            })
                            .child(self.clock_column(editor, open, ClockPart::Hour, cx))
                            .child(self.clock_column(editor, open, ClockPart::Minute, cx))
                            .when(!crate::clock_format::uses_24_hours(), |el| {
                                el.child(self.clock_column(editor, open, ClockPart::Period, cx))
                            }),
                    )
                    .child(
                        ghost_control(
                            "schedule-entry-mode",
                            if text_mode {
                                "Edit individual fields"
                            } else {
                                "Edit as text"
                            },
                            self.busy,
                        )
                        .debug_selector(|| "schedule-entry-mode".into())
                        .on_activate(cx, move |this, window, cx| {
                            if let Some(editor) = this.schedule_editor_mut() {
                                editor.date_time_picker.close();
                                editor.date_time_picker.use_text_entry(!text_mode);
                                if text_mode {
                                    editor
                                        .date_time_picker
                                        .segments
                                        .focus_first(once, window, cx);
                                } else if once {
                                    editor.source.value.focus(window, cx);
                                } else {
                                    editor.source.times.focus_handle(cx).focus(window, cx);
                                }
                                cx.notify();
                            }
                        }),
                    );
                el.child(div().absolute().top_0().left_0().child(menu_popover(
                    size(px(width), px(32.)),
                    popup.into_any_element(),
                )))
            })
    }
}
