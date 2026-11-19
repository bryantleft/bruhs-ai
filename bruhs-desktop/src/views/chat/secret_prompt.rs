//! A disposable private input entity, kept out of chat snapshots and drafts.

use super::*;
use bruhs_models::secrets::{forget_offered, saved_source_label, SecretDecision, SecretRequest};
use bruhs_ui::input::InputContentType;
use bruhs_ui::press::on_press_view;
use bruhs_ui::zoom::px;

pub(super) struct SecretPrompt {
    request: SecretRequest,
    input: Entity<InputState>,
    busy: bool,
    error: Option<String>,
    task: Option<Task<()>>,
    _input_subscription: Subscription,
    connections: Option<Entity<crate::views::service_connections::ServiceConnections>>,
    _connection_subscription: Option<Subscription>,
}

impl ChatView {
    pub fn has_private_input(&self) -> bool {
        self.secret_prompt.is_some()
    }

    /// Called when this thread leaves the screen. Pending metadata can be shown again.
    pub fn clear_private_input(&mut self) {
        self.secret_prompt = None;
    }

    /// `BRUHS_DESKTOP_SMOKE_SCENE=secret-use-saved`: the open credential
    /// form's Use saved, as its button does. False when no form is open.
    #[cfg(debug_assertions)]
    pub(crate) fn smoke_use_saved(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let Some(prompt) = self.secret_prompt.clone() else {
            return false;
        };
        prompt.update(cx, |prompt, cx| prompt.respond("saved", window, cx));
        true
    }

    pub(super) fn render_secret_prompt(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let Some(request) = self.pending_secret.clone() else {
            return div().into_any_element();
        };
        let replace = self
            .secret_prompt
            .as_ref()
            .is_none_or(|prompt| prompt.read(cx).request.id != request.id);
        if replace {
            self.secret_prompt = Some(cx.new(|cx| {
                let input = cx.new(|cx| {
                    InputState::new(window, cx)
                        .masked(true)
                        .placeholder("Enter privately")
                });
                let subscription = cx.subscribe_in(
                    &input,
                    window,
                    |this: &mut SecretPrompt, _, event, window, cx| {
                        if matches!(event, InputEvent::PressEnter { .. }) {
                            this.respond("once", window, cx);
                        }
                    },
                );
                let connections = request.connection.as_ref().map(|connection| {
                    cx.new(|cx| {
                        crate::views::service_connections::ServiceConnections::new(
                            Some(connection.provider),
                            Some((request.thread_id.clone(), request.id.clone())),
                            cx,
                        )
                    })
                });
                let connection_subscription = connections.as_ref().map(|connections| {
                    cx.observe(connections, |_: &mut SecretPrompt, _, cx| cx.notify())
                });
                SecretPrompt {
                    request: request.clone(),
                    input,
                    busy: false,
                    error: None,
                    task: None,
                    _input_subscription: subscription,
                    connections,
                    _connection_subscription: connection_subscription,
                }
            }));
        }
        if let Some(prompt) = &self.secret_prompt {
            prompt.update(cx, |prompt, _| prompt.request = request);
            div().px_4().py_2().child(prompt.clone()).into_any_element()
        } else {
            div().into_any_element()
        }
    }
}

impl SecretPrompt {
    #[cfg(test)]
    pub(super) fn set_mobile_fixture_value(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.input.update(cx, |input, cx| {
            input.set_value("PRIVATE_PHONE_FORM_CANARY", window, cx)
        });
    }
    fn respond(&mut self, action: &'static str, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        let value = self.input.read(cx).value().to_string();
        if matches!(action, "once" | "save") && value.is_empty() {
            self.error = Some("Enter a credential to continue.".into());
            cx.notify();
            return;
        }
        let decision = match action {
            "connection" => {
                let id = self
                    .request
                    .connection
                    .as_ref()
                    .and_then(|connection| {
                        self.connections
                            .as_ref()?
                            .read(cx)
                            .current(connection.provider)
                    })
                    .filter(|connection| {
                        connection.state == bruhs_models::connections::ConnectionState::Connected
                    })
                    .and_then(|connection| connection.id.clone());
                let Some(id) = id else {
                    self.error = Some("Connect an account before approving this operation.".into());
                    cx.notify();
                    return;
                };
                SecretDecision::UseConnection { id }
            }
            "once" | "save" => SecretDecision::Provide {
                value,
                save: action == "save",
            },
            "saved" => SecretDecision::UseSaved,
            "forget" => SecretDecision::ForgetSaved,
            _ => SecretDecision::Decline,
        };
        // set_value also clears the native input's undo history.
        self.input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.set_masked(true, window, cx);
        });
        self.busy = true;
        self.error = None;
        let manager = cx.global::<DesktopState>().app.secrets().clone();
        let id = self.request.id.clone();
        let thread = self.request.thread_id.clone();
        let operation = spawn_tokio(
            cx,
            async move { manager.resolve(&thread, &id, decision).await },
        );
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = operation.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    // Forgotten. A refresh that already offers a shared
                    // entry instead stays.
                    Ok(false) if forget_offered(&this.request) => {
                        this.request.saved_available = false;
                        this.request.saved_source = None;
                    }
                    Ok(false) => {}
                    Ok(true) | Err(bruhs_core::secrets::SecretError::NotPending) => {}
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn action_button(
        &self,
        action: &'static str,
        label: &'static str,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        web_button(
            action,
            WebButtonVariant::Outline,
            WebButtonSize::Small,
            ButtonState::disabled_if(self.busy),
        )
        .min_h(px(theme::CONTROL_PX))
        .debug_selector(move || format!("secret-{action}"))
        .aria_label(label)
        .child(label)
        .map(|el| {
            on_press_view(el, cx, move |this, window, cx| {
                this.respond(action, window, cx)
            })
        })
    }
}

impl Render for SecretPrompt {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let command = serde_json::to_string(&self.request.operation.command).unwrap_or_default();
        if let Some(connections) = &self.connections {
            let connected = self
                .request
                .connection
                .as_ref()
                .and_then(|connection| connections.read(cx).current(connection.provider))
                .is_some_and(|connection| {
                    connection.state == bruhs_models::connections::ConnectionState::Connected
                        && connection.configured
                });
            return div().id("private-service-request").role(Role::Group).aria_label("Service connection request")
                .max_h(window.viewport_size().height * 0.6).overflow_y_scroll().p_3().rounded(px(8.0)).border_1().border_color(theme::blueberry_400().opacity(0.3)).bg(theme::longan_900())
                .font_family(theme::UI_SMALL_FONT_FAMILY).text_size(px(12.0)).text_color(theme::rambutan_200()).flex().flex_col().gap_2()
                .child(div().font_weight(FontWeight::MEDIUM).child("Account access needed"))
                .child(self.request.operation.reason.clone())
                .child(format!("Command: {command}\nFolder: {}\nVariable: {}", bruhs_models::secrets::review_text(&self.request.cwd), self.request.operation.env_var))
                .child(connections.clone())
                .child("This command can access the account token. Only its exit status returns to the AI. Connecting an account does not approve this command.")
                .when_some(self.error.clone(), |element, error| element.child(div().id("service-request-error").role(Role::Alert).text_color(theme::lychee_300()).child(error)))
                .child(div().flex().flex_wrap().gap_2().when(connected, |element| element.child(self.action_button("connection", "Approve operation", cx))).child(self.action_button("decline", "Decline", cx)))
                .into_any_element();
        }
        div().id("private-credential-request").role(Role::Group).aria_label("Private credential request")
            .max_h(window.viewport_size().height * 0.6).overflow_y_scroll()
            .p_3().rounded(px(8.0)).border_1().border_color(theme::blueberry_400().opacity(0.3)).bg(theme::longan_900())
            .font_family(theme::UI_SMALL_FONT_FAMILY).text_size(px(12.0)).text_color(theme::rambutan_200())
            .flex().flex_col().gap_2()
            .child(div().font_weight(FontWeight::MEDIUM).child(format!("Credential needed: {}", self.request.operation.name)))
            .child(div().child(self.request.operation.reason.clone()))
            .when(self.request.storage_unavailable, |element| element.child(div().text_color(theme::durian_300()).child("Saved credentials could not be checked. Unlock your OS keychain or use a credential once.")))
            .child(div().id("credential-operation").max_h(px(160.0)).overflow_y_scroll().child(format!("Command: {command}\nFolder: {}\nVariable: {}", bruhs_models::secrets::review_text(&self.request.cwd), self.request.operation.env_var)))
            .child(div().text_color(theme::rambutan_400()).child("This command can access the credential. Only its exit status returns to the AI."))
            .child(Input::new(&self.input).aria_label("Credential").content_type(InputContentType::Password).mask_toggle().disabled(self.busy))
            .when_some(self.error.clone(), |element, error| element.child(div().id("credential-error").role(Role::Alert).text_color(theme::lychee_300()).child(format!("{error} Enter the credential again or decline."))))
            .child(div().flex().flex_wrap().gap_2()
                .child(self.action_button("once", "Use once", cx))
                .child(self.action_button("save", "Save securely and use", cx))
                .when(self.request.saved_available, |row| row.child(self.action_button("saved", "Use saved", cx)))
                .when_some(self.request.saved_source.filter(|_| self.request.saved_available), |row, source| row.child(div().id("secret-saved-source").debug_selector(move || format!("secret-source-{}", saved_source_label(source))).flex().items_center().min_h(px(theme::CONTROL_PX)).text_color(theme::rambutan_400()).child(saved_source_label(source))))
                .when(forget_offered(&self.request), |row| row.child(self.action_button("forget", "Forget saved", cx)))
                .child(self.action_button("decline", "Decline", cx)))
            .child(div().text_color(theme::rambutan_400()).child(if self.busy { "Submitting privately…" } else { bruhs_models::secrets::SECRET_FORM_FOOTER })).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{theme, AgentUiEvent, ChatView, DesktopState, SecretRequest, ThreadId};
    use bruhs_models::vault::SavedSource;
    use gpui::{AppContext as _, TestAppContext};
    use std::{cell::RefCell, rc::Rc};

    #[gpui::test]
    fn the_saved_source_shows_and_forget_is_project_only(cx: &mut TestAppContext) {
        cx.update(|cx| {
            bruhs_ui::init(cx);
            theme::apply_bruhs_theme(cx);
            cx.set_global(DesktopState::new().unwrap());
        });
        let captured = Rc::new(RefCell::new(None));
        let output = captured.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let chat = cx.new(|cx| ChatView::new(window, cx));
            *output.borrow_mut() = Some(chat.clone());
            bruhs_ui::Root::new(chat, window, cx)
        });
        let chat = captured.borrow().clone().unwrap();
        let thread = ThreadId::new_v4();
        chat.update(cx, |chat, cx| {
            chat.start_thread(thread.as_str(), "Credential fixture", cx)
        });
        for (source, label, selector, forget) in [
            (
                SavedSource::Global,
                "Saved for all projects",
                "secret-source-Saved for all projects",
                false,
            ),
            (
                SavedSource::OnePassword,
                "From 1Password",
                "secret-source-From 1Password",
                false,
            ),
            (
                SavedSource::Project,
                "Saved for this project",
                "secret-source-Saved for this project",
                true,
            ),
        ] {
            chat.update(cx, |chat, cx| {
                chat.pending_secret = Some(SecretRequest {
                    id: format!("saved-{label}"),
                    thread_id: thread.clone(),
                    operation: bruhs_models::secrets::SecretOperation {
                        name: "DEPLOY_KEY".into(),
                        reason: "Deploy".into(),
                        env_var: "DEPLOY_KEY".into(),
                        command: vec!["deploy".into()],
                    },
                    cwd: "/tmp/x".into(),
                    saved_available: true,
                    storage_unavailable: false,
                    connection: None,
                    saved_source: Some(source),
                });
                cx.notify();
            });
            cx.run_until_parked();
            assert!(cx.debug_bounds("secret-saved").is_some(), "{label}");
            assert!(cx.debug_bounds(selector).is_some(), "{label}");
            assert_eq!(
                cx.debug_bounds("secret-forget").is_some(),
                forget,
                "{label}"
            );
        }
    }

    #[gpui::test]
    fn the_smoke_scene_presses_use_saved_only_on_an_open_form(cx: &mut TestAppContext) {
        cx.update(|cx| {
            bruhs_ui::init(cx);
            theme::apply_bruhs_theme(cx);
            cx.set_global(DesktopState::new().unwrap());
        });
        let captured = Rc::new(RefCell::new(None));
        let output = captured.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let chat = cx.new(|cx| ChatView::new(window, cx));
            *output.borrow_mut() = Some(chat.clone());
            bruhs_ui::Root::new(chat, window, cx)
        });
        let chat = captured.borrow().clone().unwrap();
        let thread = ThreadId::new_v4();
        chat.update(cx, |chat, cx| {
            chat.start_thread(thread.as_str(), "Credential fixture", cx)
        });
        let pressed =
            cx.update(|window, cx| chat.update(cx, |chat, cx| chat.smoke_use_saved(window, cx)));
        assert!(!pressed, "no form is open");
        chat.update(cx, |chat, cx| {
            chat.pending_secret = Some(SecretRequest {
                id: "smoke-saved".into(),
                thread_id: thread.clone(),
                operation: bruhs_models::secrets::SecretOperation {
                    name: "DEPLOY_KEY".into(),
                    reason: "Deploy".into(),
                    env_var: "DEPLOY_KEY".into(),
                    command: vec!["deploy".into()],
                },
                cwd: "/tmp/x".into(),
                saved_available: true,
                storage_unavailable: false,
                connection: None,
                saved_source: Some(SavedSource::Global),
            });
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("secret-saved").is_some());
        let pressed =
            cx.update(|window, cx| chat.update(cx, |chat, cx| chat.smoke_use_saved(window, cx)));
        assert!(pressed, "the open form's Use saved is pressed");
    }

    #[gpui::test]
    fn private_input_is_disposable_and_never_enters_the_composer_or_messages(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            bruhs_ui::init(cx);
            theme::apply_bruhs_theme(cx);
            cx.set_global(DesktopState::new().unwrap());
        });
        let captured = Rc::new(RefCell::new(None));
        let output = captured.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let chat = cx.new(|cx| ChatView::new(window, cx));
            *output.borrow_mut() = Some(chat.clone());
            bruhs_ui::Root::new(chat, window, cx)
        });
        let chat = captured.borrow().clone().unwrap();
        chat.update_in(cx, |chat, window, cx| {
            let thread = ThreadId::new_v4();
            chat.start_thread(thread.as_str(), "Credential fixture", cx);
            chat.pending_secret = Some(SecretRequest {
                id: "private-input".into(),
                thread_id: thread,
                operation: bruhs_models::secrets::SecretOperation {
                    name: "Key".into(),
                    reason: "Deploy".into(),
                    env_var: "API_KEY".into(),
                    command: vec!["deploy".into()],
                },
                cwd: "/tmp".into(),
                saved_available: false,
                storage_unavailable: false,
                connection: None,
                saved_source: None,
            });
            drop(chat.render_secret_prompt(window, cx));
            let prompt = chat.secret_prompt.clone().unwrap();
            prompt.update(cx, |prompt, cx| {
                prompt.input.update(cx, |input, cx| {
                    input.set_value("synthetic-private-canary", window, cx)
                })
            });
            assert!(chat.input_state.read(cx).value().is_empty());
            assert!(chat.messages.is_empty());
            chat.clear_private_input();
            assert!(!chat.has_private_input());
            drop(chat.render_secret_prompt(window, cx));
            assert!(chat
                .secret_prompt
                .as_ref()
                .unwrap()
                .read(cx)
                .input
                .read(cx)
                .value()
                .is_empty());
            chat.apply_agent_event(AgentUiEvent::SecretResolved("private-input".into()));
            assert!(chat.pending_secret.is_none());
            assert!(!chat.has_private_input());
        });
    }
}
