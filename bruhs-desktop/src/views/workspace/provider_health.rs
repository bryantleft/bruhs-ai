//! Local CLI sign-in health, independent of release and package-registry checks.
use super::*;
use crate::app_state::spawn_tokio;
use std::time::Duration;

impl Workspace {
    pub(super) fn new_health_interval(
        seconds: u32,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> Entity<InputState> {
        let field = cx.new(|cx| InputState::new(window, cx).default_value(seconds.to_string()));
        subscriptions.push(cx.subscribe_in(
            &field,
            window,
            |this: &mut Self, field, event: &InputEvent, window, cx| {
                if !matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    return;
                }
                let raw = field.read(cx).value().to_string();
                let next = raw
                    .trim()
                    .parse::<u32>()
                    .map(|n| if n == 0 { 0 } else { n.clamp(30, 86400) })
                    .unwrap_or(this.health_interval);
                if raw != next.to_string() {
                    field.update(cx, |input, cx| {
                        input.set_value(next.to_string(), window, cx)
                    });
                }
                if this.health_interval != next {
                    this.custom_background();
                    this.health_interval = next;
                    this.schedule_health_checks(cx);
                    this.persist_and_notify(cx);
                }
            },
        ));
        field
    }
    pub(super) fn check_provider_health(&mut self, cx: &mut Context<'_, Self>) {
        if self.health_check.is_some() {
            return;
        }
        let probe = spawn_tokio(cx, crate::views::chat::cli_health(cx));
        self.health_check = Some(cx.spawn(async move |this, cx| {
            let health = probe.await;
            let _ = this.update(cx, |this, cx| {
                this.health_result = Some(health);
                this.health_check = None;
                cx.notify();
            });
        }));
        cx.notify();
    }
    pub(super) fn schedule_health_checks(&mut self, cx: &mut Context<'_, Self>) {
        self.health_timer = None;
        let seconds = self.health_interval;
        if seconds == 0 {
            return;
        }
        if !self.background_paused() {
            self.check_provider_health(cx);
        }
        self.health_timer = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(u64::from(seconds)))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        if !this.background_paused() {
                            this.check_provider_health(cx)
                        }
                    })
                    .is_err()
                {
                    return;
                }
            }
        }));
    }
    pub(super) fn provider_health_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let status = match self.health_result {
            Some([claude, codex]) => format!("Claude · {claude}    Codex · {codex}"),
            None if self.health_check.is_some() => "Checking the local providers…".into(),
            None => "Ready to check".into(),
        };
        let button = web_button(
            "provider-health-check",
            WebButtonVariant::Outline,
            WebButtonSize::Small,
            ButtonState::disabled_if(self.health_check.is_some()),
        )
        .aria_label("Check provider health")
        .child("Check now");
        vec![
            self.settings_row(
                "settings-row-provider-health",
                "Provider health",
                "Check sign-in on this computer.",
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .items_end()
                    .child(
                        div()
                            .id("provider-health-result")
                            .role(Role::Status)
                            .aria_label(status.clone())
                            .text_xs()
                            .text_color(theme::rambutan_400())
                            .child(status),
                    )
                    .child(on_press_view(button, cx, |this, _, cx| {
                        this.check_provider_health(cx)
                    }))
                    .into_any_element(),
            ),
            self.settings_row(
                "settings-row-provider-health-interval",
                "Health check interval",
                "Seconds between background sign-in checks. Set 0 to check manually.",
                settings_ui::control_column(
                    Input::new(&self.health_interval_input)
                        .aria_label("Provider health check interval in seconds")
                        .into_any_element(),
                ),
            ),
        ]
    }
}
