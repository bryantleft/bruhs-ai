//! System notifications for agent updates, and the login service that keeps
//! agents running after the app quits.
use super::*;
use bruhs_models::api::{BackgroundService, BackgroundServiceAction};
use bruhs_models::reactive::NOTIFICATIONS_NOTE;
use bruhs_ui::press::on_press_view;
use gpui_component::zoom::px;

pub(super) struct NotificationForm {
    /// The settings this form was opened from.
    pub(super) base: NotificationSettings,
    /// A toggle changed; keep the form even if the server's settings move.
    pub(super) dirty: bool,
    system_notify: bool,
    system_ask: bool,
    quiet: bool,
    start: Entity<InputState>,
    end: Entity<InputState>,
    time_zone: Entity<InputState>,
}

#[derive(Default)]
pub(super) struct ServiceState {
    pub(super) loaded: bool,
    status: Option<BackgroundService>,
    error: Option<String>,
    pending: bool,
    request: Option<Task<()>>,
}

/// The settings a form saves. Quiet hours keep their text as typed; the server validates it.
pub(super) fn settings_from_form(
    system_notify: bool,
    system_ask: bool,
    quiet: bool,
    start: &str,
    end: &str,
    time_zone: &str,
) -> NotificationSettings {
    NotificationSettings {
        system_notify,
        system_ask,
        quiet_hours: quiet.then(|| QuietHours {
            start: start.trim().to_string(),
            end: end.trim().to_string(),
            time_zone: time_zone.trim().to_string(),
        }),
    }
}

/// The service button, or none when this platform has no service manager.
pub(super) fn service_action(
    status: &BackgroundService,
) -> Option<(BackgroundServiceAction, &'static str)> {
    if !status.supported {
        None
    } else if status.installed {
        Some((
            BackgroundServiceAction::Uninstall,
            "Stop running after quit",
        ))
    } else {
        Some((
            BackgroundServiceAction::Install,
            "Keep agents running after quit",
        ))
    }
}

/// A labelled switch drawn as web's checkbox row, so its state is visible:
/// a filled persimmon box with a check when on, an empty outlined box when off.
fn toggle(id: &'static str, label: &'static str, on: bool, disabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Switch)
        .aria_label(label)
        .aria_toggled(if on { Toggled::True } else { Toggled::False })
        .debug_selector(move || format!("reactive-{label}"))
        .flex()
        .items_center()
        .gap_2()
        .min_h(px(32.0))
        .px_1()
        .rounded(px(4.0))
        .border_1()
        .border_color(gpui::transparent_black())
        .focus_visible(|el| el.border_color(theme::persimmon_400()))
        .text_size(px(12.0))
        .text_color(theme::rambutan_300())
        .when(disabled, |el| el.opacity(0.5).cursor_not_allowed())
        .when(!disabled, |el| el.tab_index(0).cursor_pointer())
        .child(
            div()
                .size(px(16.0))
                .flex_shrink_0()
                .rounded(px(3.0))
                .border_1()
                .border_color(if on {
                    theme::persimmon_500()
                } else {
                    theme::rambutan_500()
                })
                .bg(if on {
                    theme::persimmon_500()
                } else {
                    theme::longan_900()
                })
                .flex()
                .items_center()
                .justify_center()
                .when(on, |check| {
                    check
                        .debug_selector(move || format!("reactive-{label}-checked"))
                        .text_size(px(11.0))
                        .text_color(theme::longan_950())
                        .child("✓")
                }),
        )
        .child(label)
}

impl ReactiveAutomations {
    pub(super) fn ensure_notification_form(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.notify_form.is_some() {
            return;
        }
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let base = snapshot.notification_settings.clone();
        let zone = editor_fields::local_zone();
        let (start, end, time_zone) = base.quiet_hours.as_ref().map_or_else(
            || ("22:00".to_string(), "07:00".to_string(), zone.clone()),
            |quiet| {
                (
                    quiet.start.clone(),
                    quiet.end.clone(),
                    quiet.time_zone.clone(),
                )
            },
        );
        self.notify_form = Some(NotificationForm {
            dirty: false,
            system_notify: base.system_notify,
            system_ask: base.system_ask,
            quiet: base.quiet_hours.is_some(),
            start: edit_input(&start, false, window, cx),
            end: edit_input(&end, false, window, cx),
            time_zone: edit_input(&time_zone, false, window, cx),
            base,
        });
    }

    fn update_form(
        &mut self,
        change: impl FnOnce(&mut NotificationForm),
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(form) = &mut self.notify_form {
            change(form);
            form.dirty = true;
        }
        cx.notify();
    }

    /// Web `NotificationsPanel`: a card with its switches, the quiet hours
    /// fields when on, then the note and Save.
    pub(super) fn render_notifications(&self, cx: &Context<'_, Self>) -> Div {
        let note = muted(NOTIFICATIONS_NOTE).text_color(theme::rambutan_500());
        let mut section = card()
            .debug_selector(|| "reactive-notifications".to_string())
            .child(heading("Notifications"));
        let Some(form) = &self.notify_form else {
            return section.child(note);
        };
        let busy = self.busy;
        section = section
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        toggle(
                            "reactive-notify-updates",
                            "System notification for updates",
                            form.system_notify,
                            busy,
                        )
                        .on_activate(cx, |this, _, cx| {
                            this.update_form(|form| form.system_notify = !form.system_notify, cx)
                        }),
                    )
                    .child(
                        toggle(
                            "reactive-notify-questions",
                            "System notification for questions",
                            form.system_ask,
                            busy,
                        )
                        .on_activate(cx, |this, _, cx| {
                            this.update_form(|form| form.system_ask = !form.system_ask, cx)
                        }),
                    )
                    .child(
                        toggle("reactive-quiet-hours", "Quiet hours", form.quiet, busy)
                            .on_activate(cx, |this, _, cx| {
                                this.update_form(|form| form.quiet = !form.quiet, cx)
                            }),
                    ),
            )
            .when(form.quiet, |section| {
                section
                    .child(field("Start", &form.start, false, busy))
                    .child(field("End", &form.end, false, busy))
                    .child(field("Time zone", &form.time_zone, false, busy))
            })
            .child(note)
            .child(
                div().flex().child(
                    control(
                        "reactive-save-notifications",
                        "Save notification settings",
                        busy,
                    )
                    .on_activate(cx, |this, _, cx| {
                        let Some(form) = &this.notify_form else {
                            return;
                        };
                        let settings = settings_from_form(
                            form.system_notify,
                            form.system_ask,
                            form.quiet,
                            &value(&form.start, cx),
                            &value(&form.end, cx),
                            &value(&form.time_zone, cx),
                        );
                        this.send_in(
                            ReactiveCommand::SetNotificationSettings { settings },
                            Scope::Notifications,
                            cx,
                        );
                    }),
                ),
            )
            // Web: a failed save is reported under the button.
            .children(self.scoped_error(&Scope::Notifications));
        section
    }

    pub(super) fn load_service(&mut self, cx: &mut Context<'_, Self>) {
        self.service.loaded = true;
        let client = self.client.clone();
        let operation = spawn_tokio(cx, async move { client.background_service().await });
        self.service.request = Some(cx.spawn(async move |this, cx| {
            let result = operation.await;
            let _ = this.update(cx, |this, cx| {
                if this.service.pending {
                    return;
                }
                match result {
                    Ok(status) => {
                        this.service.status = Some(status);
                        this.service.error = None;
                    }
                    Err(error) => this.service.error = Some(reactive_error(error)),
                }
                cx.notify();
            });
        }));
    }

    fn apply_service(&mut self, action: BackgroundServiceAction, cx: &mut Context<'_, Self>) {
        if self.service.pending {
            return;
        }
        self.service.pending = true;
        self.service.error = None;
        let client = self.client.clone();
        let operation = spawn_tokio(
            cx,
            async move { client.set_background_service(action).await },
        );
        self.service.request = Some(cx.spawn(async move |this, cx| {
            let result = operation.await;
            let _ = this.update(cx, |this, cx| {
                this.service.pending = false;
                match result {
                    Ok(status) => this.service.status = Some(status),
                    Err(error) => this.service.error = Some(reactive_error(error)),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn render_background_service(&self, cx: &Context<'_, Self>) -> Option<Div> {
        if self.service.status.is_none() && self.service.error.is_none() {
            return None;
        }
        // Web `BackgroundServicePanel`: a card beside Notifications.
        let mut section = card()
            .debug_selector(|| "reactive-background-service".to_string())
            .child(heading("Background service"));
        if let Some(status) = &self.service.status {
            section = section.child(
                div()
                    .min_w_0()
                    .font_family(theme::CODE_FONT_FAMILY)
                    .text_size(px(13.0))
                    .line_height(relative(1.5))
                    .text_color(theme::rambutan_300())
                    .child(status.detail.clone()),
            );
            if let Some((action, label)) = service_action(status) {
                section = section.child(
                    div().flex().child(
                        control_as(
                            "reactive-background-service",
                            label,
                            self.service.pending,
                            true,
                        )
                        .map(|el| {
                            on_press_view(el, cx, move |this, _, cx| this.apply_service(action, cx))
                        }),
                    ),
                );
            }
        }
        if let Some(error) = &self.service.error {
            section = section.child(
                div()
                    .id("reactive-service-error")
                    .role(Role::Alert)
                    .text_size(px(13.0))
                    .text_color(theme::lychee_300())
                    .child(error.clone()),
            );
        }
        Some(section)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        service_action, settings_from_form, BackgroundService, BackgroundServiceAction,
        ReactiveAutomations,
    };
    use crate::theme;
    use bruhs_models::reactive::{NotificationSettings, QuietHours, ReactiveSnapshot};
    use gpui::AppContext as _;

    /// PM-48: the web's order (Refresh under the introduction, Updates,
    /// Weekly engineering reports, then New automation and the empty line)
    /// and Notifications beside Background service from the `lg`
    /// breakpoint, stacked below it.
    #[gpui::test]
    fn agent_workflows_follow_the_web_order(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            theme::apply_bruhs_theme(cx);
        });
        let holder = std::rc::Rc::new(std::cell::RefCell::new(None));
        let capture = holder.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let panel = cx.new(ReactiveAutomations::new);
            *capture.borrow_mut() = Some(panel.clone());
            gpui_component::Root::new(panel, window, cx)
        });
        cx.simulate_resize(gpui::size(gpui::px(1280.0), gpui::px(2400.0)));
        let panel = holder.borrow().clone().unwrap();
        panel.update_in(cx, |panel, _, cx| {
            panel.service.status = Some(BackgroundService {
                supported: true,
                detail: "Automations stop when this server stops.".into(),
                ..BackgroundService::default()
            });
            panel.show_snapshot_for_test(ReactiveSnapshot::default(), cx);
        });
        cx.run_until_parked();
        let bounds = |cx: &mut gpui::VisualTestContext, selector: &'static str| {
            cx.debug_bounds(selector)
                .unwrap_or_else(|| panic!("{selector} is drawn"))
        };
        let introduction = bounds(cx, "reactive-introduction");
        let refresh = bounds(cx, "reactive-Refresh");
        let updates = bounds(cx, "reactive-updates");
        let weekly = bounds(cx, "reactive-New report preset");
        let new_automation = bounds(cx, "reactive-New automation");
        assert!(
            refresh.top() > introduction.bottom(),
            "Refresh under the introduction"
        );
        assert!(refresh.left() <= introduction.left() + gpui::px(1.0));
        assert!(refresh.bottom() < updates.top(), "then Updates");
        assert!(updates.bottom() < weekly.top(), "then the weekly reports");
        assert!(
            weekly.bottom() < new_automation.top(),
            "then New automation"
        );
        let notifications = bounds(cx, "reactive-notifications");
        let service = bounds(cx, "reactive-background-service");
        assert!(notifications.top() > new_automation.bottom());
        assert_eq!(notifications.top(), service.top(), "side by side");
        assert!(notifications.right() < service.left());
        assert_eq!(notifications.size.width, service.size.width);

        cx.simulate_resize(gpui::size(gpui::px(900.0), gpui::px(2400.0)));
        cx.run_until_parked();
        let notifications = bounds(cx, "reactive-notifications");
        let service = bounds(cx, "reactive-background-service");
        assert!(
            service.top() > notifications.bottom(),
            "stacked below the web's lg breakpoint"
        );
    }

    #[test]
    fn notification_form_saves_quiet_hours_only_when_enabled() {
        assert_eq!(
            settings_from_form(true, false, false, "22:00", "07:00", "UTC"),
            NotificationSettings {
                system_notify: true,
                system_ask: false,
                quiet_hours: None,
            }
        );
        assert_eq!(
            settings_from_form(false, true, true, " 22:00", "07:30 ", " Europe/Paris "),
            NotificationSettings {
                system_notify: false,
                system_ask: true,
                quiet_hours: Some(QuietHours {
                    start: "22:00".into(),
                    end: "07:30".into(),
                    time_zone: "Europe/Paris".into(),
                }),
            }
        );
    }

    #[test]
    fn service_button_follows_install_state_and_hides_when_unsupported() {
        let mut status = BackgroundService {
            supported: false,
            detail: "Unavailable on this platform.".into(),
            ..BackgroundService::default()
        };
        assert_eq!(service_action(&status), None);
        status.supported = true;
        assert_eq!(
            service_action(&status),
            Some((
                BackgroundServiceAction::Install,
                "Keep agents running after quit"
            ))
        );
        status.installed = true;
        assert_eq!(
            service_action(&status),
            Some((
                BackgroundServiceAction::Uninstall,
                "Stop running after quit"
            ))
        );
    }
}
