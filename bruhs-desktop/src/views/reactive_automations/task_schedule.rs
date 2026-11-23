//! The four schedule modes in the Settings task dialog.
use super::*;

const MODES: [(&str, &str); 4] = [
    ("calendar", "At a time"),
    ("once", "Once"),
    ("schedule", "Every interval"),
    ("webhook", "On webhook"),
];
const DAYS: [(&str, &str); 7] = [
    ("Mo", "Monday"),
    ("Tu", "Tuesday"),
    ("We", "Wednesday"),
    ("Th", "Thursday"),
    ("Fr", "Friday"),
    ("Sa", "Saturday"),
    ("Su", "Sunday"),
];
pub(super) const MAX_EXACT_MS: u64 = (1 << 53) - 1;

#[derive(Clone, Copy)]
enum Group {
    Mode,
    Day,
}

pub(super) struct ScheduleFocus {
    modes: [FocusHandle; 4],
    days: [FocusHandle; 7],
    mode_index: usize,
    day_index: usize,
}

impl ScheduleFocus {
    pub(super) fn new(cx: &mut App) -> Self {
        Self {
            modes: std::array::from_fn(|_| cx.focus_handle()),
            days: std::array::from_fn(|_| cx.focus_handle()),
            mode_index: 0,
            day_index: 0,
        }
    }

    fn items(&self, group: Group) -> (&[FocusHandle], usize) {
        match group {
            Group::Mode => (&self.modes, self.mode_index),
            Group::Day => (&self.days, self.day_index),
        }
    }

    fn focus(&mut self, group: Group, index: usize, window: &mut Window, cx: &mut App) {
        match group {
            Group::Mode => self.mode_index = index,
            Group::Day => self.day_index = index,
        }
        self.items(group).0[index].focus(window, cx);
    }
}

pub(super) fn parse_clock(text: &str) -> Option<chrono::NaiveTime> {
    ["%H:%M", "%I:%M %p"]
        .iter()
        .find_map(|pattern| chrono::NaiveTime::parse_from_str(text.trim(), pattern).ok())
}

pub(super) fn parse_local_minute(text: &str) -> Option<chrono::NaiveDateTime> {
    let (date, clock) = text.trim().split_once(' ')?;
    Some(
        chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .ok()?
            .and_time(parse_clock(clock)?),
    )
}

pub(super) fn format_local_minute(dt: chrono::NaiveDateTime) -> String {
    format!(
        "{} {}",
        dt.date(),
        dt.time().format(crate::clock_format::pattern())
    )
}

fn local_minute(at: bruhs_models::EpochMs) -> String {
    chrono::DateTime::from_timestamp_millis(at.as_millis()).map_or_else(
        || once_time(at),
        |t| format_local_minute(t.with_timezone(&chrono::Local).naive_local()),
    )
}

fn format_times(text: &str, pattern: &str) -> String {
    text.split(',')
        .map(|t| {
            parse_clock(t).map_or_else(
                || t.trim().to_string(),
                |time| time.format(pattern).to_string(),
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

impl AutomationEditor {
    pub(in crate::views::reactive_automations) fn prepare_task_schedule(
        &mut self,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.scheduled_form = true;
        self.task_name.update(cx, |input, cx| {
            input.set_placeholder("e.g. Check for Sentry issues", window, cx)
        });
        if self.is_new {
            self.source.kind = "calendar";
        }
        self.schedule_focus.mode_index = MODES
            .iter()
            .position(|(kind, _)| *kind == self.source.kind)
            .unwrap_or_default();
        let minutes = self.schedule_uses_minutes();
        let source = &mut self.source;
        let times = if matches!(source.source.kind, SourceKind::Calendar { .. }) {
            value(&source.times, cx)
        } else {
            source.weekdays = (1..=5).collect();
            "09:00".into()
        };
        self.date_time_picker.use_text_entry(source.kind == "calendar" && times.contains(','));
        source.times.set_text_value(
            format_times(&times, crate::clock_format::pattern()).into(),
            window,
            cx,
        );
        if let SourceKind::Once { due_at } = source.source.kind {
            self.once_instant = Some(due_at);
            source.value.set_value(local_minute(due_at), window, cx);
        } else {
            source.value.set_value("", window, cx);
        }
        let interval = match source.source.kind {
            SourceKind::Schedule { every_ms } if minutes => (bruhs_models::num::ToFloatLossy::to_f64_lossy(every_ms) / 60_000.).to_string(),
            SourceKind::Schedule { every_ms } => seconds(every_ms),
            _ => "15".into(),
        };
        self.source
            .interval
            .set_text_value(interval.into(), window, cx);
    }

    pub(super) fn calendar_times(&self, cx: &App) -> String {
        let times = value(&self.source.times, cx);
        if self.scheduled_form {
            format_times(&times, "%H:%M")
        } else {
            times
        }
    }

    pub(super) fn seed_once(&mut self, window: &mut Window, cx: &mut App) {
        const HOUR: i64 = 3_600_000;
        let now = bruhs_models::EpochMs::now().as_millis();
        let next = bruhs_models::EpochMs::new(((now + HOUR - 1) / HOUR) * HOUR)
            .expect("next hour on the system clock");
        self.source.value.set_value(local_minute(next), window, cx);
        self.once_instant = Some(next);
    }

    pub(super) fn once_schedule(&self, text: &str) -> Result<SourceKind, String> {
        use chrono::{Local, TimeZone, Timelike};
        let Some(local) = parse_local_minute(text) else {
            return once_source(text).map_err(|error| {
                if self.date_time_picker.in_text_mode() {
                    error
                } else {
                    "Enter a valid date and time.".into()
                }
            });
        };
        let saved = self.once_instant.and_then(|at| {
            chrono::DateTime::from_timestamp_millis(at.as_millis())
                .map(|dt| (at, dt.with_timezone(&Local)))
        });
        // Compare wall-clock minutes so changing display format cannot lose
        // the seconds or the UTC offset of an existing task.
        if let Some((due_at, dt)) = saved {
            if dt.date_naive() == local.date()
                && dt.hour() == local.hour()
                && dt.minute() == local.minute()
            {
                return Ok(SourceKind::Once { due_at });
            }
        }
        let saved_offset = saved.map(|(_, dt)| *dt.offset());
        let occurrences = Local
            .from_local_datetime(&local)
            .map(|at| at.fixed_offset());
        let at = schedule_occurrence(occurrences, saved_offset)?;
        let due_at =
            bruhs_models::EpochMs::new(at.timestamp_millis()).map_err(|err| err.to_string())?;
        Ok(SourceKind::Once { due_at })
    }
}

/// Match the saved UTC offset when editing within a repeated local hour.
/// A fresh selection uses the first occurrence. Skipped wall times cannot run.
fn schedule_occurrence(
    occurrences: chrono::MappedLocalTime<chrono::DateTime<chrono::FixedOffset>>,
    saved_offset: Option<chrono::FixedOffset>,
) -> Result<chrono::DateTime<chrono::FixedOffset>, &'static str> {
    use chrono::MappedLocalTime;
    match occurrences {
        MappedLocalTime::Single(time) => Ok(time),
        MappedLocalTime::Ambiguous(a, b) => Ok([a, b]
            .into_iter()
            .find(|time| Some(*time.offset()) == saved_offset)
            // The platform can return the offset pair in either instant order.
            .unwrap_or_else(|| a.min(b))),
        MappedLocalTime::None => Err("That time does not exist in your time zone."),
    }
}

pub(super) fn interval_milliseconds(text: &str, original: &SourceKind) -> Result<u64, String> {
    if let SourceKind::Schedule { every_ms } = original {
        if text.trim() == (bruhs_models::num::ToFloatLossy::to_f64_lossy(*every_ms) / 60_000.).to_string() {
            return Ok(*every_ms);
        }
    }
    let millis = text.trim().parse::<f64>().unwrap_or(f64::NAN) * 60_000.;
    let millis = millis.round();
    if !(60_000. ..=bruhs_models::num::ToFloatLossy::to_f64_lossy(MAX_EXACT_MS)).contains(&millis) {
        return Err(
            "Enter an interval of at least one minute within the supported clock range.".into(),
        );
    }
    Ok(bruhs_models::num::ToIntSaturating::to_u64_saturating(millis))
}

impl ReactiveAutomations {
    pub(super) fn render_task_enabled(
        &self,
        editor: &AutomationEditor,
        cx: &Context<'_, Self>,
    ) -> Div {
        use bruhs_ui::{press::Interactivity, switch::switch};
        div()
            .min_w_0()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(heading("Enabled"))
                    .child(muted("Disabled tasks stay saved and do not run.")),
            )
            .child(
                switch(
                    "schedule-task-enabled",
                    "Enable task",
                    editor.task_enabled,
                    Interactivity::disabled_if(self.busy),
                )
                .on_activate(cx, |this, _, cx| {
                    if let Some(Editor::Automation(editor)) = &mut this.editor {
                        editor.task_enabled = !editor.task_enabled;
                        cx.notify();
                    }
                }),
            )
    }

    pub(super) fn schedule_selector(
        &self,
        editor: &AutomationEditor,
        cx: &Context<'_, Self>,
    ) -> Div {
        div()
            .min_w_0()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_2()
            .child(heading("Schedule"))
            .child(self.schedule_group(editor, Group::Mode, cx))
    }

    fn schedule_group(
        &self,
        editor: &AutomationEditor,
        group: Group,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        let (handles, cursor) = editor.schedule_focus.items(group);
        let (id, label) = match group {
            Group::Mode => ("task-schedule-type", "Schedule type"),
            Group::Day => ("task-schedule-days", "Days to run"),
        };
        div()
            .id(id)
            .debug_selector(move || id.into())
            .role(Role::Group)
            .aria_label(label)
            .min_w_0()
            .flex()
            .flex_wrap()
            .p(px(2.))
            .gap(px(2.))
            .rounded(px(7.))
            .bg(theme::longan_900())
            .border_1()
            .border_color(theme::longan_700())
            .children(handles.iter().enumerate().map(|(index, handle)| {
                let (text, name, selected) = match group {
                    Group::Mode => (
                        MODES[index].1,
                        MODES[index].1,
                        editor.source.kind == MODES[index].0,
                    ),
                    Group::Day => (
                        DAYS[index].0,
                        DAYS[index].1,
                        editor.source.weekdays.is_empty()
                            || u8::try_from(index)
                                .is_ok_and(|i| editor.source.weekdays.contains(&(i + 1))),
                    ),
                };
                let stop = cursor == index && !self.busy;
                let key = format!("{id}-{index}");
                div()
                    .id(SharedString::from(key.clone()))
                    .debug_selector(move || key)
                    .role(Role::Button)
                    .aria_label(name)
                    .aria_toggled(selected.into())
                    .when(self.busy, |el| el.opacity(0.5).cursor_not_allowed())
                    .track_focus(&handle.clone().tab_stop(stop))
                    .tab_index(0)
                    .tab_stop(stop)
                    .flex()
                    .items_center()
                    .justify_center()
                    .h(px(26.))
                    .px(px(8.))
                    .rounded(px(5.))
                    .text_size(px(12.))
                    .whitespace_nowrap()
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .when(selected, |b| {
                        b.bg(theme::longan_700()).text_color(theme::rambutan_100())
                    })
                    .when(!selected, |b| {
                        b.text_color(theme::rambutan_400())
                            .hover(|b| b.bg(theme::longan_800()).text_color(theme::rambutan_100()))
                    })
                    .focus_visible(|b| b.border_color(theme::persimmon_400()))
                    .when(!self.busy, |b| b.cursor_pointer())
                    .child(text)
                    .on_activate(cx, move |this, window, cx| {
                        let Some(Editor::Automation(editor)) = &mut this.editor else {
                            return;
                        };
                        editor.schedule_focus.focus(group, index, window, cx);
                        match group {
                            Group::Mode => editor.select_trigger(MODES[index].0, window, cx),
                            Group::Day => {
                                let day = u8::try_from(index).unwrap_or(0) + 1;
                                let days = &mut editor.source.weekdays;
                                if days.is_empty() {
                                    *days = (1..=7).collect();
                                }
                                if days.len() != 1 || days[0] != day {
                                    editor_fields::toggle_weekday(days, day);
                                    if days.len() == 7 {
                                        days.clear();
                                    }
                                }
                            }
                        }
                        cx.notify();
                    })
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                let key = match event.keystroke.key.as_str() {
                    "left" => "up",
                    "right" => "down",
                    "home" => "home",
                    "end" => "end",
                    _ => return,
                };
                if this.busy {
                    return;
                }
                let Some(Editor::Automation(editor)) = &mut this.editor else {
                    return;
                };
                let (handles, _) = editor.schedule_focus.items(group);
                let current = handles.iter().position(|handle| handle.is_focused(window));
                if let Some(index) =
                    bruhs_models::logic::menu::menu_step(current, handles.len(), key)
                {
                    window.prevent_default();
                    cx.stop_propagation();
                    editor.schedule_focus.focus(group, index, window, cx);
                    cx.notify();
                }
            }))
    }

    pub(super) fn schedule_timing_fields(
        &self,
        editor: &AutomationEditor,
        cx: &Context<'_, Self>,
    ) -> Div {
        let input = |label: &'static str, field: &dyn TextInput, width| {
            div().w(px(width)).min_w_0().child(
                field
                    .text_element(label.into(), self.busy)
                    .h(px(32.))
                    .text_size(px(14.)),
            )
        };
        let kind = editor.source.kind;
        let row = div()
            .debug_selector(move || format!("task-schedule-fields-{kind}"))
            .min_w_0()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2();
        match editor.source.kind {
            "calendar" => row
                .child(muted("Run at"))
                .child(self.schedule_datetime_input(editor, cx))
                .child(muted(value(&editor.source.time_zone, cx)))
                .child(muted("on"))
                .child(self.schedule_group(editor, Group::Day, cx)),
            "once" => row
                .child(muted("Run on"))
                .child(self.schedule_datetime_input(editor, cx)),
            _ => row
                .child(muted("Run every"))
                .child(input("Run every", &editor.source.interval, 84.))
                .child(muted(if editor.schedule_uses_minutes() {
                    "minutes"
                } else {
                    "seconds"
                })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::schedule_occurrence;
    use chrono::{DateTime, MappedLocalTime};

    #[test]
    fn scheduled_repeated_hour_keeps_the_saved_offset_and_skipped_times_are_rejected() {
        let early = DateTime::parse_from_rfc3339("2032-11-07T01:30:00-05:00").unwrap();
        let late = DateTime::parse_from_rfc3339("2032-11-07T01:30:00-06:00").unwrap();
        for occurrences in [
            MappedLocalTime::Ambiguous(early, late),
            MappedLocalTime::Ambiguous(late, early),
        ] {
            assert_eq!(schedule_occurrence(occurrences, None), Ok(early));
            assert_eq!(
                schedule_occurrence(occurrences, Some(*early.offset())),
                Ok(early)
            );
            assert_eq!(
                schedule_occurrence(occurrences, Some(*late.offset())),
                Ok(late)
            );
        }
        assert!(schedule_occurrence(MappedLocalTime::None, None).is_err());
    }
}
