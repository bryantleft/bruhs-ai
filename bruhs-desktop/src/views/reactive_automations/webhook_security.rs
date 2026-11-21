//! Webhook admission controls in the native task form.
use super::*;
use bruhs_ui::{
    dropdown::{dropdown, DropdownGroup, DropdownOption, Listbox},
    press::Interactivity,
    switch::switch,
};

pub(super) struct SecurityFields {
    pub(super) enabled: bool,
    pub(super) encoding: WebhookSignatureEncoding,
    pub(super) encoding_open: bool,
    pub(super) header: Entity<InputState>,
    pub(super) prefix: Entity<InputState>,
    pub(super) secret: Entity<InputState>,
    pub(super) age: Entity<InputState>,
    has_saved_key: bool,
}

impl SecurityFields {
    pub(super) fn new(saved: &WebhookSecurity, window: &mut Window, cx: &mut App) -> Self {
        let signature = saved.signature.clone().unwrap_or_default();
        let enabled = saved.signature.is_some();
        let secret = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(if enabled {
                    "Unchanged"
                } else {
                    "Shared secret"
                })
        });
        let age = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("Run every request");
            if let Some(age) = saved.max_delivery_age_minutes {
                input.set_value(age.to_string(), window, cx);
            }
            input
        });
        Self {
            enabled,
            has_saved_key: enabled,
            encoding_open: false,
            header: edit_input(&signature.header, window, cx),
            prefix: edit_input(&signature.prefix, window, cx),
            encoding: signature.encoding,
            secret,
            age,
        }
    }

    pub(super) fn replacement(&self, cx: &App) -> Option<WebhookSigningSecret> {
        if !self.enabled {
            return None;
        }
        let text = value(&self.secret, cx);
        (!text.is_empty()).then_some(WebhookSigningSecret(text))
    }

    pub(super) fn build(&self, cx: &App) -> Result<WebhookSecurity, String> {
        let age = value(&self.age, cx);
        let policy = WebhookSecurity {
            signature: self.enabled.then(|| WebhookSignature {
                header: value(&self.header, cx).trim().to_ascii_lowercase(),
                prefix: value(&self.prefix, cx),
                encoding: self.encoding,
            }),
            max_delivery_age_minutes: (!age.trim().is_empty())
                .then(|| age.trim().parse::<u16>())
                .transpose()
                .map_err(|_| "Enter whole minutes from 1 to 1440, or leave the age limit blank.")?,
        };
        policy.validate()?;
        if let Some(secret) = self.replacement(cx) {
            secret.validate()?;
        } else if self.enabled && !self.has_saved_key {
            return Err("Enter the shared signing secret.".into());
        }
        Ok(policy)
    }

    pub(super) fn age(&self, disabled: bool) -> Div {
        field(
            "Skip requests older than (minutes, optional)",
            &self.age,
            false,
            disabled,
        )
    }
}

impl ReactiveAutomations {
    pub(super) fn render_webhook_security(
        &self,
        draft: &SecurityFields,
        cx: &Context<'_, Self>,
    ) -> Div {
        let row = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(heading("Require signature"))
                    .child(muted(
                        "Verify the HMAC-SHA256 signature of the request body.",
                    )),
            )
            .child(
                switch(
                    "task-webhook-signature",
                    "Require signature",
                    draft.enabled,
                    Interactivity::disabled_if(self.busy),
                )
                .on_activate(cx, |this, _, cx| {
                    if let Some(Editor::Automation(editor)) = &mut this.editor {
                        editor.security.enabled = !editor.security.enabled;
                        editor.security.encoding_open = false;
                        cx.notify();
                    }
                }),
            );
        let group = div().flex().flex_col().gap_3().child(row);
        if !draft.enabled {
            return group;
        }
        let weak = cx.weak_entity();
        let select = weak.clone();
        let encoding = dropdown(
            "task-signature-encoding",
            "Signature encoding",
            match draft.encoding {
                WebhookSignatureEncoding::Hex => "hex",
                WebhookSignatureEncoding::Base64 => "base64",
            },
            vec![DropdownGroup {
                label: None,
                mark: None,
                options: ["hex", "base64"]
                    .into_iter()
                    .map(|value| DropdownOption {
                        value,
                        label: value.into(),
                        description: None,
                    })
                    .collect(),
            }],
            if draft.encoding_open {
                Listbox::Popover
            } else {
                Listbox::Closed
            },
            Rc::new(move |_, cx| {
                let _ = weak.update(cx, |this, cx| {
                    if this.busy {
                        return;
                    }
                    if let Some(Editor::Automation(editor)) = &mut this.editor {
                        editor.checkout.close();
                        editor.picker.open = None;
                        editor.open_filter = None;
                        editor.task_thread_open = false;
                        editor.security.encoding_open = !editor.security.encoding_open;
                        cx.notify();
                    }
                });
            }),
            Rc::new(move |selected, _, cx| {
                let _ = select.update(cx, |this, cx| {
                    if this.busy {
                        return;
                    }
                    if let Some(Editor::Automation(editor)) = &mut this.editor {
                        editor.security.encoding = if selected == "base64" {
                            WebhookSignatureEncoding::Base64
                        } else {
                            WebhookSignatureEncoding::Hex
                        };
                        editor.security.encoding_open = false;
                        cx.notify();
                    }
                });
            }),
        );
        let pair = || div().flex().flex_wrap().gap_3();
        let cell = || div().flex_1().min_w(px(155.0));
        group
            .child(
                pair()
                    .child(cell().child(field("Header", &draft.header, false, self.busy)))
                    .child(cell().child(field("Prefix", &draft.prefix, false, self.busy))),
            )
            .child(
                pair()
                    .child(
                        cell()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(muted("Encoding"))
                            .child(encoding),
                    )
                    .child(cell().child(field("Secret", &draft.secret, false, self.busy))),
            )
    }
}
