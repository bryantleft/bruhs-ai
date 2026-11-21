use super::*;
use secret_sources::SecretReference;

struct Running(tokio::task::AbortHandle);

impl Drop for Running {
    fn drop(&mut self) {
        self.0.abort();
    }
}

enum Status {
    Ready,
    Checking(Running),
    Resolved,
    Failed(String),
}

struct FieldCheck {
    reference: SecretReference,
    account: String,
    status: Status,
}

impl FieldCheck {
    fn start(&mut self, cx: &mut Context<'_, Self>) {
        if matches!(self.status, Status::Checking(_)) {
            return;
        }
        let reference = self.reference.clone();
        let job = cx
            .global::<DesktopState>()
            .runtime
            .spawn(async move { reference.check().await });
        let id = job.id();
        self.status = Status::Checking(Running(job.abort_handle()));
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                if !matches!(&this.status, Status::Checking(running) if running.0.id() == id) {
                    return;
                }
                this.status = match result {
                    Ok(Ok(())) => Status::Resolved,
                    Ok(Err(error)) => Status::Failed(error.to_string()),
                    Err(_) => Status::Failed("The check was interrupted. Try again.".into()),
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

#[derive(IntoElement)]
pub(super) struct CheckButton {
    pub id: String,
    pub label: String,
    pub reference: SecretReference,
    pub account: String,
    pub disabled: bool,
}

impl RenderOnce for CheckButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key = SharedString::from(format!("{}-state", self.id));
        let state = window.use_keyed_state(key, cx, |_, _| FieldCheck {
            reference: self.reference.clone(),
            account: self.account.clone(),
            status: Status::Ready,
        });
        let previous = state.read(cx);
        if previous.reference != self.reference || previous.account != self.account {
            state.update(cx, |field, _| {
                field.reference = self.reference;
                field.account = self.account;
                field.status = Status::Ready;
            });
        }
        let current = &state.read(cx).status;
        let (icon, color, hint, accessible, suffix) = match current {
            Status::Ready => (
                "icons/shield-check.svg",
                theme::rambutan_400(),
                "Check that the secret resolves",
                self.label,
                "ready",
            ),
            Status::Checking(_) => (
                assets::LOADER_ICON_PATH,
                theme::rambutan_400(),
                "Checking the secret",
                format!("{} · Checking", self.label),
                "checking",
            ),
            Status::Resolved => (
                assets::CHECK_ICON_PATH,
                theme::pandan_400(),
                "Secret resolved",
                format!("{} resolved", self.label),
                "resolved",
            ),
            Status::Failed(detail) => (
                assets::ALERT_TRIANGLE_ICON_PATH,
                theme::lychee_300(),
                detail.as_str(),
                format!("{} failed. {detail}", self.label),
                "failed",
            ),
        };
        let disabled = self.disabled || matches!(current, Status::Checking(_));
        let hint = hint.to_owned();
        let mark = icon_svg(icon, 14., color);
        let mark = if matches!(current, Status::Checking(_)) && !crate::motion::reduce_motion(cx) {
            mark.with_animation(
                "secret-check-spin",
                Animation::new(std::time::Duration::from_secs(1)).repeat(),
                |el, delta| el.with_transformation(Transformation::rotate(percentage(delta))),
            )
            .into_any_element()
        } else {
            mark.into_any_element()
        };
        let id = self.id;
        let selector = format!("{id}-{suffix}");
        let trigger = web_button(
            SharedString::from(id.clone()),
            WebButtonVariant::Ghost,
            WebButtonSize::IconSmall,
            ButtonState::disabled_if(disabled),
        )
        .debug_selector(move || id.clone())
        .aria_label(accessible)
        .tooltip(move |w, cx| Tooltip::new(hint.clone()).build(w, cx))
        .child(
            div()
                .id("secret-check-mark")
                .debug_selector(move || selector.clone())
                .flex()
                .child(mark),
        );
        if disabled {
            return trigger;
        }
        bruhs_ui::press::on_press(trigger, move |_, cx| {
            state.update(cx, |field, cx| field.start(cx))
        })
    }
}
