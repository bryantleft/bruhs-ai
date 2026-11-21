//! Editable snooze menu choices for the environment and individual projects.
use super::*;
use chrono::{DateTime, Datelike, Days, Local, NaiveTime, TimeZone};

pub(super) const MAX_OPTIONS: usize = 8;
const DAYS: &[(&str, &str)] = &[
    ("after", "In"),
    ("today", "Today"),
    ("tomorrow", "Tomorrow"),
    ("monday", "Monday"),
    ("tuesday", "Tuesday"),
    ("wednesday", "Wednesday"),
    ("thursday", "Thursday"),
    ("friday", "Friday"),
    ("saturday", "Saturday"),
    ("sunday", "Sunday"),
    ("week", "Next week"),
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(super) enum Choice {
    After { minutes: u32 },
    At { day: String, time: NaiveTime },
}

fn defaults() -> Vec<Choice> {
    let at = |day: &str, hour| Choice::At {
        day: day.into(),
        time: NaiveTime::from_hms_opt(hour, 0, 0).unwrap(),
    };
    vec![
        Choice::After { minutes: 60 },
        at("today", 18),
        at("tomorrow", 9),
        at("week", 9),
    ]
}

impl Choice {
    fn day(&self) -> &str {
        match self {
            Self::After { .. } => "after",
            Self::At { day, .. } => day,
        }
    }
    fn value(&self) -> String {
        match self {
            Self::After { minutes } => minutes.to_string(),
            Self::At { time, .. } => time.format("%H:%M").to_string(),
        }
    }
    pub(super) fn label(&self) -> String {
        match self {
            Self::After { minutes } => {
                let (amount, unit) = if minutes.is_multiple_of(1440) {
                    (minutes / 1440, "day")
                } else if minutes.is_multiple_of(60) {
                    (minutes / 60, "hour")
                } else {
                    (*minutes, "minute")
                };
                format!("In {amount} {unit}{}", if amount == 1 { "" } else { "s" })
            }
            Self::At { day, time } => format!(
                "{} at {}",
                DAYS.iter()
                    .find(|(k, _)| *k == day)
                    .map(|(_, d)| *d)
                    .unwrap_or(day),
                time.format(crate::clock_format::pattern())
            ),
        }
    }
    pub(super) fn until(&self, now_ms: i64) -> Option<i64> {
        self.until_in(Local.timestamp_millis_opt(now_ms).single()?)
    }
    fn until_in<T: TimeZone>(&self, now: DateTime<T>) -> Option<i64> {
        match self {
            Self::After { minutes } if *minutes > 0 => now
                .timestamp_millis()
                .checked_add(i64::from(*minutes) * 60_000),
            Self::After { .. } => None,
            Self::At { day, time } => {
                let days = match day.as_str() {
                    "today" => 0,
                    "tomorrow" => 1,
                    "week" => 7,
                    weekday => {
                        let offset =
                            DAYS[3..10].iter().position(|(key, _)| *key == weekday)? as u32;
                        (offset + 6 - now.weekday().num_days_from_monday()) % 7 + 1
                    }
                };
                let local = now
                    .date_naive()
                    .checked_add_days(Days::new(days.into()))?
                    .and_time(*time);
                // For a skipped local time, use the first valid minute after it.
                // At a repeated hour, choose the first occurrence still in the future.
                for offset in 0..=180 {
                    let candidate = local.checked_add_signed(chrono::Duration::minutes(offset))?;
                    let (first, last) = match now.timezone().from_local_datetime(&candidate) {
                        chrono::LocalResult::None => continue,
                        chrono::LocalResult::Single(date) => {
                            (date.timestamp_millis(), date.timestamp_millis())
                        }
                        chrono::LocalResult::Ambiguous(early, late) => {
                            (early.timestamp_millis(), late.timestamp_millis())
                        }
                    };
                    return [first, last]
                        .into_iter()
                        .filter(|ms| *ms > now.timestamp_millis())
                        .min();
                }
                None
            }
        }
    }
}

pub(super) struct Editor {
    pub open: bool,
    index: Option<usize>,
    pub day: String,
    value: Entity<InputState>,
    error: Option<String>,
}
impl Editor {
    pub(super) fn new(window: &mut Window, cx: &mut Context<'_, Workspace>) -> Self {
        Self {
            open: false,
            index: None,
            day: "after".into(),
            value: cx.new(|cx| InputState::new(window, cx)),
            error: None,
        }
    }
}

impl Workspace {
    pub(super) fn snooze_choices(&self, path: &str) -> Option<&[Choice]> {
        self.project_defaults
            .get(path)
            .and_then(|p| p.snooze_choices.as_deref())
            .or(self.snooze_options.as_deref())
    }

    fn edited_snooze_choices(&self) -> Vec<Choice> {
        self.snooze_choices(&self.settings_project_scope)
            .map(|choices| choices.iter().take(MAX_OPTIONS).cloned().collect())
            .unwrap_or_else(defaults)
    }

    fn set_snooze_choices(&mut self, choices: Option<Vec<Choice>>, cx: &mut Context<'_, Self>) {
        if self.settings_project_scope.is_empty() {
            self.snooze_options = choices;
        } else {
            let scope = self.settings_project_scope.clone();
            let prefs = self.project_defaults.entry(scope.clone()).or_default();
            prefs.snooze_choices = choices;
            if *prefs == Default::default() {
                self.project_defaults.remove(&scope);
            }
        }
        self.snooze_editor.open = false;
        self.persist_and_notify(cx);
    }

    fn edit_snooze(
        &mut self,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let choice = index
            .and_then(|i| self.edited_snooze_choices().get(i).cloned())
            .unwrap_or(Choice::After { minutes: 30 });
        self.snooze_editor.index = index;
        self.snooze_editor.day = choice.day().into();
        self.snooze_editor.open = true;
        self.snooze_editor.error = None;
        self.snooze_editor.value.update(cx, |input, cx| {
            input.set_value(choice.value(), window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn choose_snooze_day(
        &mut self,
        day: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !DAYS.iter().any(|(key, _)| *key == day) {
            return;
        }
        if (self.snooze_editor.day == "after") != (day == "after") {
            self.snooze_editor.value.update(cx, |input, cx| {
                input.set_value(if day == "after" { "60" } else { "09:00" }, window, cx)
            });
        }
        self.snooze_editor.day = day.into();
        self.snooze_editor.error = None;
        cx.notify();
    }

    fn save_snooze(&mut self, cx: &mut Context<'_, Self>) {
        let value = self.snooze_editor.value.read(cx).value();
        let day = &self.snooze_editor.day;
        let choice = if day == "after" {
            value
                .trim()
                .parse::<u32>()
                .ok()
                .filter(|n| *n > 0)
                .map(|minutes| Choice::After { minutes })
        } else {
            NaiveTime::parse_from_str(value.trim(), "%H:%M")
                .ok()
                .map(|time| Choice::At {
                    day: day.clone(),
                    time,
                })
        };
        let Some(choice) = choice else {
            self.snooze_editor.error = Some(
                if day == "after" {
                    "Enter a positive number of minutes."
                } else {
                    "Enter a local time in HH:MM format."
                }
                .into(),
            );
            cx.notify();
            return;
        };
        let mut choices = self.edited_snooze_choices();
        let index = self.snooze_editor.index;
        if choices
            .iter()
            .enumerate()
            .any(|(i, existing)| Some(i) != index && existing == &choice)
        {
            self.snooze_editor.error = Some("This choice already exists.".into());
            cx.notify();
            return;
        }
        if let Some(slot) = index.and_then(|i| choices.get_mut(i)) {
            *slot = choice;
        } else if choices.len() < MAX_OPTIONS {
            choices.push(choice);
        }
        self.set_snooze_choices(Some(choices), cx);
    }

    pub(super) fn render_snooze_settings(&self, cx: &Context<'_, Self>) -> AnyElement {
        let choices = self.edited_snooze_choices();
        let button = |id: SharedString, label: &'static str, off: bool| {
            web_button(
                id,
                WebButtonVariant::Outline,
                WebButtonSize::Small,
                ButtonState::disabled_if(off),
            )
            .child(label)
        };
        let scoped = !self.settings_project_scope.is_empty();
        let overridden = if scoped {
            self.project_defaults
                .get(&self.settings_project_scope)
                .is_some_and(|p| p.snooze_choices.is_some())
        } else {
            self.snooze_options.is_some()
        };
        let mut controls = div().flex().flex_wrap().gap_2().child(on_press_view(
            button(
                "snooze-add".into(),
                "Add option",
                choices.len() >= MAX_OPTIONS,
            ),
            cx,
            |this, w, cx| this.edit_snooze(None, w, cx),
        ));
        if overridden {
            controls = controls.child(on_press_view(
                button(
                    "snooze-reset".into(),
                    if scoped {
                        "Use environment defaults"
                    } else {
                        "Reset"
                    },
                    false,
                ),
                cx,
                |this, _, cx| this.set_snooze_choices(None, cx),
            ));
        }
        let mut rows = vec![self.settings_row(
            "desktop-settings-row-snooze-options",
            "Snooze options",
            "Choose up to eight times for the thread menu. Available choices appear soonest first.",
            controls.into_any_element(),
        )];
        let mut display: Vec<_> = choices.iter().enumerate().collect();
        let now = EpochMs::now().as_millis();
        display.sort_by_key(|(_, c)| c.until(now).unwrap_or(i64::MAX));
        for (i, choice) in display {
            rows.push(
                self.settings_row(
                    "desktop-settings-snooze-choice",
                    choice.label(),
                    "",
                    div()
                        .flex()
                        .gap_2()
                        .child(on_press_view(
                            button(format!("edit-snooze-{i}").into(), "Edit", false)
                                .aria_label(format!("Edit {}", choice.label())),
                            cx,
                            move |this, w, cx| this.edit_snooze(Some(i), w, cx),
                        ))
                        .child(on_press_view(
                            button(format!("remove-snooze-{i}").into(), "Remove", false)
                                .aria_label(format!("Remove {}", choice.label())),
                            cx,
                            move |this, _, cx| {
                                let mut choices = this.edited_snooze_choices();
                                if i < choices.len() {
                                    choices.remove(i);
                                    this.set_snooze_choices(Some(choices), cx);
                                }
                            },
                        ))
                        .into_any_element(),
                ),
            );
        }
        if self.snooze_editor.open {
            let duration = self.snooze_editor.day == "after";
            rows.push(self.settings_row(
                "desktop-settings-snooze-when",
                "When",
                "",
                settings_ui::control_column(self.choice_control(
                    "settings-snooze-day",
                    "When",
                    &self.snooze_editor.day,
                    DAYS,
                    cx,
                )),
            ));
            rows.push(
                self.settings_row(
                    "desktop-settings-snooze-time",
                    if duration { "Minutes" } else { "Local time" },
                    self.snooze_editor.error.clone().unwrap_or_else(|| {
                        if duration {
                            String::new()
                        } else {
                            "Enter a time in HH:MM format.".into()
                        }
                    }),
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            Input::new(&self.snooze_editor.value).aria_label(if duration {
                                "Minutes"
                            } else {
                                "Local time"
                            }),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(on_press_view(
                                    button("snooze-save".into(), "Save", false),
                                    cx,
                                    |this, _, cx| this.save_snooze(cx),
                                ))
                                .child(on_press_view(
                                    button("snooze-cancel".into(), "Cancel", false),
                                    cx,
                                    |this, _, cx| {
                                        this.snooze_editor.open = false;
                                        cx.notify();
                                    },
                                )),
                        )
                        .into_any_element(),
                ),
            );
        }
        self.settings_card("Snooze", "", rows)
    }
}

#[cfg(test)]
mod tests {
    use super::Choice;
    use chrono::{NaiveTime, TimeZone, Utc};
    #[test]
    fn same_weekday_means_next_week_and_elapsed_today_is_omitted() {
        let monday = Utc.with_ymd_and_hms(2026, 11, 16, 10, 0, 0).unwrap();
        let at = |day: &str| Choice::At {
            day: day.into(),
            time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        };
        assert_eq!(
            at("monday").until_in(monday),
            Some(
                Utc.with_ymd_and_hms(2026, 11, 23, 9, 0, 0)
                    .unwrap()
                    .timestamp_millis()
            )
        );
        assert_eq!(at("today").until_in(monday), None);
        assert_eq!(
            Choice::After { minutes: 30 }.until_in(monday),
            Some(monday.timestamp_millis() + 1_800_000)
        );
    }
}
