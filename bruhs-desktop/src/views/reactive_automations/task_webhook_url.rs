//! A saved task's local webhook address belongs to this editor instance.
use super::*;
use bruhs_ui::{copy_feedback::CopyFeedback, press::on_press_view};

pub(super) struct TaskWebhookUrl {
    client: ServerClient,
    task: String,
    input: Entity<InputState>,
    address: Option<reqwest::Url>,
    error: Option<String>,
    loading: bool,
    copied: CopyFeedback,
    _focus: gpui::Subscription,
}

impl TaskWebhookUrl {
    pub(super) fn new(
        client: ServerClient,
        task: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let input = edit_input("", window, cx);
        let focus = cx.on_focus(&input.focus_handle(cx), window, |this: &mut Self, _, cx| {
            this.input.update(cx, |field, cx| {
                field.set_selected_range(0..field.value().len(), cx)
            });
        });
        let mut view = Self {
            client,
            task,
            input,
            address: None,
            error: None,
            loading: false,
            copied: CopyFeedback::default(),
            _focus: focus,
        };
        view.fetch(None, window, cx);
        view
    }

    fn fetch(&mut self, previous: Option<String>, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.loading {
            return;
        }
        self.loading = true;
        self.error = None;
        let client = self.client.clone();
        let task = self.task.clone();
        let result = spawn_tokio(cx, async move {
            client
                .task_webhook_address(&task, previous.as_deref())
                .await
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = result.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.loading = false;
                match result {
                    Ok(url) => {
                        this.input.update(cx, |field, cx| {
                            field.set_value(url.as_str().to_owned(), window, cx)
                        });
                        this.address = Some(url);
                    }
                    Err(error) => {
                        this.address = None;
                        this.input
                            .update(cx, |field, cx| field.set_value("", window, cx));
                        this.error = Some(reactive_error(error));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn rotate(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.loading {
            return;
        }
        let Some(token) = self
            .address
            .as_ref()
            .and_then(|url| url.path_segments()?.next_back())
            .map(str::to_owned)
        else {
            return;
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            "Rotate this webhook URL?",
            Some("The current URL stops working."),
            &[
                PromptButton::Cancel("Cancel".into()),
                PromptButton::Ok("Rotate".into()),
            ],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(1) {
                let _ = this.update_in(cx, |this, window, cx| this.fetch(Some(token), window, cx));
            }
        })
        .detach();
    }
}

impl Render for TaskWebhookUrl {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let mut body = div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1()
            .child(muted("Webhook URL"));
        if let Some(error) = &self.error {
            return body.child(muted(error.clone())).child(on_press_view(
                control("retry-task-url", "Retry", false),
                cx,
                |this, window, cx| this.fetch(None, window, cx),
            ));
        }
        let Some(address) = &self.address else {
            return body.child(muted("Loading URL…"));
        };
        let local = address.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        let (copy_label, icon, tint) = if self.copied.is_shown("url") {
            (
                "Copied",
                crate::assets::CHECK_ICON_PATH,
                theme::pandan_400(),
            )
        } else {
            ("Copy", crate::assets::COPY_ICON_PATH, theme::rambutan_500())
        };
        body = body.child(
            div()
                .w_full()
                .min_w_0()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .debug_selector(|| "task-webhook-url-field".into())
                        .child(
                            Input::new(&self.input)
                                .aria_label("Webhook URL")
                                .readonly(true),
                        ),
                )
                .child(on_press_view(
                    web_button(
                        "copy-task-url",
                        WebButtonVariant::Outline,
                        WebButtonSize::Small,
                        ButtonState::disabled_if(self.loading),
                    )
                    .aria_label(copy_label)
                    .child(svg().path(icon).size(px(theme::ICON_PX)).text_color(tint))
                    .child(copy_label)
                    .debug_selector(|| "copy-task-webhook-url".into()),
                    cx,
                    |this, _, cx| {
                        if this.loading {
                            return;
                        }
                        if let Some(url) = &this.address {
                            cx.write_to_clipboard(ClipboardItem::new_string(
                                url.as_str().to_owned(),
                            ));
                            CopyFeedback::show(this, |view| &mut view.copied, "url", cx);
                        }
                    },
                ))
                .child(on_press_view(
                    control(
                        "rotate-task-url",
                        if self.loading {
                            "Rotating…"
                        } else {
                            "Rotate"
                        },
                        self.loading,
                    )
                    .debug_selector(|| "rotate-task-webhook-url".into()),
                    cx,
                    Self::rotate,
                )),
        );
        body.child(muted(if local {
            "Only this computer can call this URL."
        } else {
            "Call this URL from a device that can reach the task host."
        }))
    }
}
