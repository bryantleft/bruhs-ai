//! System notifications for agent updates, and the login service that keeps
//! agents running after the app quits.
use super::*;
use bruhs_models::api::{BackgroundService, BackgroundServiceAction};

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

fn toggle(id: &'static str, label: &'static str, on: bool, disabled: bool) -> Stateful<Div> {
    control(id, label, disabled)
        .role(Role::Switch)
        .aria_toggled(if on { Toggled::True } else { Toggled::False })
        .when(on, |button| button.bg(theme::blueberry_500().opacity(0.15)))
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

    pub(super) fn render_notifications(&self, cx: &Context<'_, Self>) -> Div {
        let mut section = div()
            .min_w_0()
            .w_full()
            .flex()
            .flex_col()
            .gap_3()
            .pt_4()
            .border_t_1()
            .border_color(theme::longan_700().opacity(0.6))
            .child(heading("Notifications"))
            .child(muted(
                "Sent by the local server, so they arrive while the app is closed when the background service is on. Off by default.",
            ));
        let Some(form) = &self.notify_form else {
            return section;
        };
        let busy = self.busy;
        section = section
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
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
            .children(self.scoped_error(&Scope::Notifications))
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
            );
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
        let mut section = div()
            .min_w_0()
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            .pb_4()
            .border_b_1()
            .border_color(theme::longan_700().opacity(0.6))
            .child(heading("Background service"));
        if let Some(status) = &self.service.status {
            section = section.child(muted(status.detail.clone()));
            if let Some((action, label)) = service_action(status) {
                section = section.child(
                    div().flex().child(
                        control("reactive-background-service", label, self.service.pending)
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.apply_service(action, cx)),
                            )
                            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                                if super::super::web_button::is_web_button_activation_key(
                                    event.keystroke.key.as_str(),
                                ) {
                                    cx.stop_propagation();
                                    this.apply_service(action, cx);
                                }
                            })),
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
    use super::{service_action, settings_from_form, BackgroundService, BackgroundServiceAction};
    use bruhs_models::reactive::{NotificationSettings, QuietHours};

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
            installed: false,
            loaded: false,
            binary: None,
            detail: "Unavailable on this platform.".into(),
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
