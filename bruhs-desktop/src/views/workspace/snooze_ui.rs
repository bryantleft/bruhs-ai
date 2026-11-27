//! The thread menu's "Custom…" snooze (web: the sidebar's "Snooze until"
//! dialog): pick a date and time, or a duration from now.

use super::date_picker_ui::{parse_iso_datetime, DatePickerTarget};
use super::*;
use bruhs_models::logic::sidebar::{custom_snooze_until, SnoozeUnit, SNOOZE_CUSTOM_TITLE};
use bruhs_ui::press::on_press_view;
use bruhs_ui::zoom::px;

/// Debug selector prefix of the unit dropdown.
pub(super) const SNOOZE_UNIT_DROPDOWN_ID: &str = "desktop-snooze-unit";
/// Web `w-24` on the amount field.
const SNOOZE_AMOUNT_WIDTH_PX: f32 = 96.0;
/// Web `gap-3` between the fields and `gap-2` inside the duration row.
const SNOOZE_BODY_GAP_PX: f32 = 12.0;
const SNOOZE_ROW_GAP_PX: f32 = 8.0;

/// The dialog's text fields. Its unit and open listbox live in
/// `Overlay::SnoozeCustom`.
pub(super) struct SnoozeInputs {
    /// `YYYY-MM-DDTHH:MM`, written by the date and time picker.
    pub when: Entity<InputState>,
    pub amount: Entity<InputState>,
}

impl SnoozeInputs {
    pub(super) fn new(window: &mut Window, cx: &mut Context<'_, Workspace>) -> Self {
        Self {
            when: cx.new(|cx| InputState::new(window, cx)),
            amount: cx.new(|cx| InputState::new(window, cx)),
        }
    }
}

/// Epoch ms of a local `YYYY-MM-DDTHH:MM`, as the web reads its picker.
pub(super) fn local_datetime_ms(value: &str) -> Option<i64> {
    let (year, month, day, hour, minute) = parse_iso_datetime(value)?;
    let naive = NaiveDate::from_ymd_opt(year, month, day)?.and_hms_opt(hour, minute, 0)?;
    Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|at| at.timestamp_millis())
}

impl Workspace {
    /// Open the dialog for thread `id` with empty fields and hours, like web.
    pub(super) fn open_snooze_custom(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        for input in [
            self.snooze_inputs.when.clone(),
            self.snooze_inputs.amount.clone(),
        ] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.open_overlay(Overlay::SnoozeCustom {
            thread_id: id.to_string(),
            unit: SnoozeUnit::Hours,
            unit_open: false,
        });
        self.focus_first_modal_tab_stop(window, cx);
        cx.notify();
    }

    /// The open dialog's thread, unit and whether the unit list is open.
    pub(super) fn snooze_custom(&self) -> Option<(&str, SnoozeUnit, bool)> {
        match self.overlay.as_ref()? {
            Overlay::SnoozeCustom {
                thread_id,
                unit,
                unit_open,
            } => Some((thread_id.as_str(), *unit, *unit_open)),
            _ => None,
        }
    }

    fn set_snooze_unit(
        &mut self,
        unit: Option<SnoozeUnit>,
        unit_open: bool,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(Overlay::SnoozeCustom {
            unit: current,
            unit_open: open,
            ..
        }) = self.overlay.as_mut()
        {
            if let Some(unit) = unit {
                *current = unit;
            }
            *open = unit_open;
            cx.notify();
        }
    }

    /// The Snooze button. A duration wins over the picked time, which must
    /// still be ahead; otherwise the dialog stays open, as on web.
    pub(super) fn submit_snooze_custom(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some((id, unit, _)) = self.snooze_custom() else {
            return;
        };
        let id = id.to_string();
        let amount = self.snooze_inputs.amount.read(cx).value().to_string();
        let chosen = local_datetime_ms(&self.snooze_inputs.when.read(cx).value());
        let Some(until) = custom_snooze_until(EpochMs::now().as_millis(), &amount, unit, chosen)
        else {
            return;
        };
        self.close_transient_surfaces();
        self.snooze_thread_by_id(&id, until, cx);
        // Web dialogs hand focus back to the control that opened them.
        self.focus_thread_menu_button(&id, window, cx);
        cx.notify();
    }

    pub(super) fn render_snooze_custom(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let (unit, unit_open) = self
            .snooze_custom()
            .map_or((SnoozeUnit::Hours, false), |(_, unit, open)| (unit, open));
        let entity = cx.entity().downgrade();
        let toggle_entity = entity.clone();
        let on_toggle: ToggleHandler = Rc::new(move |_window, cx| {
            let _ = toggle_entity.update(cx, |this, cx| {
                let open = this.snooze_custom().is_some_and(|(_, _, open)| open);
                this.set_snooze_unit(None, !open, cx);
            });
        });
        let on_select: SelectHandler = Rc::new(move |value, _window, cx| {
            let _ = entity.update(cx, |this, cx| {
                this.set_snooze_unit(Some(SnoozeUnit::from_label(value)), false, cx);
            });
        });
        let units = vec![DropdownGroup {
            label: None,
            mark: None,
            options: SnoozeUnit::ALL
                .into_iter()
                .map(|unit| DropdownOption {
                    value: unit.label().into(),
                    label: unit.label().into(),
                    description: None,
                })
                .collect(),
        }];
        let field = |label: &'static str, control: AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap(px(FORM_FIELD_GAP_PX))
                .child(self.event_create_field_label(label))
                .child(control)
        };
        let duration = div()
            .flex()
            .items_start()
            .gap(px(SNOOZE_ROW_GAP_PX))
            .child(div().w(px(SNOOZE_AMOUNT_WIDTH_PX)).flex_shrink_0().child(
                form_text_input(Input::new(&self.snooze_inputs.amount)).aria_label("Snooze amount"),
            ))
            .child(div().min_w_0().flex_1().child(dropdown(
                SNOOZE_UNIT_DROPDOWN_ID,
                "Snooze unit",
                unit.label(),
                units,
                Listbox::open_if(unit_open),
                on_toggle,
                on_select,
            )));
        let submit = web_button(
            "desktop-snooze-submit",
            WebButtonVariant::Primary,
            WebButtonSize::Default,
            ButtonState::Enabled,
        )
        .debug_selector(|| "desktop-snooze-submit".to_string())
        .w_full()
        .aria_label("Snooze")
        .child("Snooze")
        .tab_index(0)
        .role(Role::Button)
        .focus_visible(|el| el.border_color(theme::persimmon_400()))
        .map(|el| {
            on_press_view(el, cx, |this, window, cx| {
                this.submit_snooze_custom(window, cx);
            })
        });
        let content = div()
            .debug_selector(|| "desktop-snooze-custom".to_string())
            .p_4()
            .flex()
            .flex_col()
            .gap(px(SNOOZE_BODY_GAP_PX))
            .child(field(
                "Date and time",
                self.render_date_field(DatePickerTarget::SnoozeUntil, cx),
            ))
            .child(field("Or a duration", duration.into_any_element()))
            .child(submit);
        self.render_modal_shell(SNOOZE_CUSTOM_TITLE, content, None, window, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::local_datetime_ms;
    use chrono::{Local, TimeZone};

    #[test]
    fn picked_times_read_as_local_time_like_the_web() {
        let expected = Local
            .with_ymd_and_hms(2026, 11, 17, 9, 30, 0)
            .earliest()
            .map(|at| at.timestamp_millis());
        assert_eq!(local_datetime_ms("2026-11-17T09:30"), expected);
        assert_eq!(local_datetime_ms(""), None);
        assert_eq!(local_datetime_ms("2026-13-01T09:00"), None);
    }
}
