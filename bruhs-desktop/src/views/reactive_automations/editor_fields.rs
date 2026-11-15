//! Calendar times and the busy policy, shared by the source, subscription and
//! automation editors.
use super::*;

pub(super) const BUSY_QUEUE_LABEL: &str = "Wait, then handle new events together";
pub(super) const BUSY_RESTART_LABEL: &str = "Restart with the new event";

/// The zone new calendar sources and quiet hours start with.
pub(super) fn local_zone() -> String {
    bruhs_core::reactive::clock::local_zone_name()
}

/// `"08:00, 12:30"` → `["08:00", "12:30"]`; blanks between commas are ignored.
pub(super) fn parse_times(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|time| !time.is_empty())
        .map(str::to_string)
        .collect()
}

/// Build a calendar source, or the shape error the owner should see.
pub(super) fn calendar_kind(
    times: &str,
    weekdays: &[u8],
    time_zone: &str,
) -> Result<SourceKind, String> {
    let times = parse_times(times);
    let mut weekdays = weekdays.to_vec();
    weekdays.sort_unstable();
    weekdays.dedup();
    let time_zone = time_zone.trim();
    if let Some(error) = calendar_shape_error(&times, &weekdays, time_zone) {
        return Err(error);
    }
    Ok(SourceKind::Calendar {
        times,
        weekdays,
        time_zone: time_zone.to_string(),
    })
}

pub(super) fn toggle_weekday(weekdays: &mut Vec<u8>, day: u8) {
    if let Some(index) = weekdays.iter().position(|selected| *selected == day) {
        weekdays.remove(index);
    } else {
        weekdays.push(day);
        weekdays.sort_unstable();
    }
}

pub(super) fn busy_label(policy: BusyPolicy) -> &'static str {
    match policy {
        BusyPolicy::Queue => BUSY_QUEUE_LABEL,
        BusyPolicy::Restart => BUSY_RESTART_LABEL,
    }
}

impl ReactiveAutomations {
    /// The source fields being edited, alone or inside a complete automation.
    fn source_editor_mut(&mut self) -> Option<&mut SourceEditor> {
        match self.editor.as_mut()? {
            Editor::Source(editor) => Some(editor),
            Editor::Automation(editor) => Some(editor.source_mut()),
            _ => None,
        }
    }

    fn subscription_editor_mut(&mut self) -> Option<&mut SubscriptionEditor> {
        match self.editor.as_mut()? {
            Editor::Subscription(editor) => Some(editor),
            Editor::Automation(editor) => Some(editor.subscription_mut()),
            _ => None,
        }
    }

    pub(super) fn render_calendar_fields(
        &self,
        editor: &SourceEditor,
        cx: &Context<'_, Self>,
    ) -> Div {
        let busy = self.busy;
        div()
            .min_w_0()
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            .child(field(
                "Times (HH:MM, comma-separated)",
                &editor.times,
                false,
                busy,
            ))
            .child(muted("Days"))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .children(WEEKDAY_NAMES.iter().zip(1u8..).map(|(name, day)| {
                        let on = editor.weekdays.contains(&day);
                        control(
                            SharedString::from(format!("reactive-calendar-day-{day}")),
                            name,
                            busy,
                        )
                        .role(Role::CheckBox)
                        .aria_toggled(if on { Toggled::True } else { Toggled::False })
                        .when(on, |button| button.bg(theme::blueberry_500().opacity(0.15)))
                        .on_activate(cx, move |this, _, cx| {
                            if let Some(source) = this.source_editor_mut() {
                                toggle_weekday(&mut source.weekdays, day);
                            }
                            cx.notify();
                        })
                    })),
            )
            .child(muted("None selected runs every day."))
            .child(field("Time zone", &editor.time_zone, false, busy))
    }

    pub(super) fn render_busy_choice(&self, policy: BusyPolicy, cx: &Context<'_, Self>) -> Div {
        div()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1()
            .child(muted("While the agent is busy"))
            .child(div().flex().flex_wrap().gap_2().children(
                [BusyPolicy::Queue, BusyPolicy::Restart].map(|choice| {
                    let selected = policy == choice;
                    control(
                        SharedString::from(format!("reactive-busy-{choice:?}")),
                        busy_label(choice),
                        self.busy,
                    )
                    .aria_selected(selected)
                    .when(selected, |button| {
                        button.bg(theme::blueberry_500().opacity(0.15))
                    })
                    .on_activate(cx, move |this, _, cx| {
                        if let Some(editor) = this.subscription_editor_mut() {
                            editor.subscription.on_busy = choice;
                        }
                        cx.notify();
                    })
                }),
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::{busy_label, calendar_kind, parse_times, toggle_weekday, BusyPolicy, SourceKind};

    #[test]
    fn calendar_times_parse_from_a_comma_list_and_validate_before_save() {
        assert_eq!(parse_times(" 08:00, ,12:30 ,"), vec!["08:00", "12:30"]);
        assert_eq!(
            calendar_kind("08:00, 17:30", &[5, 1, 5], " Europe/Paris "),
            Ok(SourceKind::Calendar {
                times: vec!["08:00".into(), "17:30".into()],
                weekdays: vec![1, 5],
                time_zone: "Europe/Paris".into(),
            })
        );
        assert_eq!(
            calendar_kind("8:00", &[], "UTC"),
            Err("“8:00” is not a 24-hour HH:MM time.".into())
        );
        assert!(calendar_kind("", &[], "UTC").is_err());
        assert!(calendar_kind("08:00", &[], " ").is_err());
    }

    #[test]
    fn weekday_toggles_stay_sorted_and_busy_labels_match_the_web() {
        let mut days = vec![3];
        toggle_weekday(&mut days, 1);
        toggle_weekday(&mut days, 7);
        assert_eq!(days, vec![1, 3, 7]);
        toggle_weekday(&mut days, 3);
        assert_eq!(days, vec![1, 7]);
        assert_eq!(
            busy_label(BusyPolicy::default()),
            "Wait, then handle new events together"
        );
        assert_eq!(
            busy_label(BusyPolicy::Restart),
            "Restart with the new event"
        );
    }
}
