//! The companion server owns browser capture storage and its cleanup policy.
use super::*;

pub(super) struct Retention {
    pub(super) days: Option<u16>,
    input: Entity<InputState>,
    busy: bool,
    message: Option<String>,
}
impl Retention {
    pub(super) fn new(
        window: &mut Window,
        cx: &mut Context<'_, Workspace>,
        subscriptions: &mut Vec<Subscription>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Days"));
        subscriptions.push(cx.subscribe_in(
            &input,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.save_capture_days(cx);
                }
            },
        ));
        Self {
            days: None,
            input,
            busy: false,
            message: None,
        }
    }
}

impl Workspace {
    pub(in crate::views::workspace) fn refresh_capture_retention(
        &mut self,
        desired: Option<u16>,
        cx: &mut Context<'_, Self>,
    ) {
        let state = &mut self.maintenance.captures;
        if state.busy {
            return;
        }
        state.busy = true;
        state.message = None;
        let task = spawn_tokio(cx, async move {
            crate::server_client::ServerClient::workbench_client()
                .browser_capture_retention(desired)
                .await
        });
        let window = self.workflow_window;
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = window.update(cx, |_, window, cx| {
                this.update(cx, |this, cx| {
                    let state = &mut this.maintenance.captures;
                    state.busy = false;
                    match result {
                        Ok(days) => {
                            state.days = Some(days);
                            state.input.update(cx, |field, cx| {
                                field.set_value(days.to_string(), window, cx)
                            });
                        }
                        Err(error) => {
                            state.message = Some(settings_ui::friendly_server_error(&error));
                        }
                    }
                    cx.notify();
                })
            });
        })
        .detach();
        cx.notify();
    }
    fn save_capture_days(&mut self, cx: &mut Context<'_, Self>) {
        let state = &mut self.maintenance.captures;
        if state.busy {
            return;
        }
        let value = state
            .input
            .read(cx)
            .value()
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|days| (1..=bruhs_core::uploads::browser_captures::MAX_DAYS).contains(days));
        if let Some(days) = value {
            self.refresh_capture_retention(Some(days), cx);
        } else {
            state.message = Some("Enter a retention period between 1 and 3650 days.".into());
            cx.notify();
        }
    }
    pub(super) fn capture_retention_row(&self, cx: &Context<'_, Self>) -> AnyElement {
        let state = &self.maintenance.captures;
        let label = "Delete old browser artifacts";
        let help = state.message.clone().unwrap_or_else(|| match state.days {
            Some(0) => "Keep captures until automatic deletion is turned on.".into(),
            Some(days) => format!(
                "Delete captures after {days} {}.",
                if days == 1 { "day" } else { "days" }
            ),
            None => "Loading capture policy…".into(),
        });
        let on = state.days.unwrap_or_default() > 0;
        let mut controls = div().min_w_0().flex().flex_col().items_end().gap_2();
        controls = controls.child(settings_ui::switch(
            "settings-browser-capture-retention",
            on,
            state.busy || state.days.is_none(),
            label,
            cx,
            move |this, _, cx| this.refresh_capture_retention(Some(if on { 0 } else { 30 }), cx),
        ));
        if on {
            controls = controls.child(
                div()
                    .flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div().w(px(76.)).child(
                            Input::new(&state.input)
                                .aria_label("Browser capture retention in days")
                                .disabled(state.busy),
                        ),
                    )
                    .child(on_press_view(
                        web_button(
                            "settings-save-browser-capture-days",
                            WebButtonVariant::Outline,
                            WebButtonSize::Small,
                            ButtonState::disabled_if(state.busy),
                        )
                        .aria_label("Save browser capture retention")
                        .child("Save days"),
                        cx,
                        |this, _, cx| this.save_capture_days(cx),
                    )),
            );
        }
        if state.message.is_some() && state.days.is_none() {
            controls = controls.child(on_press_view(
                web_button(
                    "settings-browser-capture-retry",
                    WebButtonVariant::Outline,
                    WebButtonSize::Small,
                    ButtonState::disabled_if(state.busy),
                )
                .aria_label("Retry browser capture settings")
                .child("Retry"),
                cx,
                |this, _, cx| this.refresh_capture_retention(None, cx),
            ));
        }
        self.environment_maintenance_row(
            "desktop-settings-row-browser-captures",
            label,
            help,
            match state.days {
                Some(0) => "Off".into(),
                Some(days) => format!("{days} days"),
                None => "Unavailable".into(),
            },
            controls.into_any_element(),
        )
    }
}
