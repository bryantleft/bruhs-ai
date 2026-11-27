use super::*;
use crate::views::terminal_view::TerminalView;
use bruhs_core::agent::health::account::{self, Account, Action, Event, Kind, Method};

#[derive(Default)]
pub(super) struct SignIn {
    state: Option<Account>,
    selected: Option<String>,
    menu: bool,
    pending: Option<Pending>,
    message: Option<&'static str>,
    browser: Option<(String, tokio::sync::oneshot::Sender<bool>)>,
    terminal: Option<Entity<TerminalView>>,
    from_edit: bool,
    pub(super) return_after_save: bool,
    pub(super) advanced: bool,
}
struct Pending {
    discover: bool,
    id: uuid::Uuid,
    task: tokio::task::AbortHandle,
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl SignIn {
    #[expect(
        clippy::wrong_self_convention,
        reason = "reports whether sign-in was opened from the edit page, not a constructor"
    )]
    pub(super) fn from_edit(&self) -> bool {
        self.from_edit
    }
    fn method(&self, original: &AcpInstance) -> Option<&Method> {
        let list = &self.state.as_ref()?.methods;
        self.selected
            .as_deref()
            .or_else(|| original.registry.as_ref()?.auth_method_id.as_deref())
            .and_then(|id| list.iter().find(|m| m.id == id))
            .or_else(|| list.first())
    }
    fn clear_job(&mut self) {
        self.pending = None;
        self.browser = None;
        self.terminal = None;
        self.menu = false;
    }
}

impl Workspace {
    pub(super) fn show_provider_account(
        &mut self,
        from_edit: bool,
        w: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(e) = self.provider_editor.as_mut() else {
            return;
        };
        if e.busy || e.original.is_none() {
            return;
        }
        e.page = Page::Account;
        e.error = None;
        e.account = SignIn {
            from_edit,
            ..Default::default()
        };
        e.start.focus(w, cx);
        self.provider_account_job(Action::Discover, w, cx);
    }
    pub(super) fn finish_provider_account(&mut self, w: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(e) = self.provider_editor.as_mut() else {
            return;
        };
        if e.account.menu {
            e.account.menu = false;
            cx.notify();
            return;
        }
        if e.account.pending.as_ref().is_some_and(|p| !p.discover) {
            self.cancel_provider_account(w, cx);
            return;
        }
        if e.account.from_edit {
            e.page = Page::Edit;
            e.error = None;
            e.account = SignIn::default();
            e.name.update(cx, |f, c| f.focus(w, c));
            cx.notify();
        } else {
            let trigger = e.trigger.clone();
            self.provider_editor = None;
            if let Some(f) = trigger {
                f.focus(w, cx)
            } else {
                self.focus_modal_dialog(w, cx)
            }
            cx.notify();
        }
    }
    fn cancel_provider_account(&mut self, _w: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(e) = &mut self.provider_editor {
            e.account.clear_job();
            e.account.message = Some("Cancelled.");
            cx.notify();
        }
    }
    fn provider_account_job(&mut self, action: Action, w: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(e) = self.provider_editor.as_mut() else {
            return;
        };
        let Some(original) = &e.original else { return };
        if e.page != Page::Account || e.account.pending.is_some() {
            return;
        }
        let instance = original.id.clone();
        let attempt = uuid::Uuid::new_v4();
        e.account.menu = false;
        e.error = None;
        e.account.message = Some(match action {
            Action::Discover => "Discovering sign-in methods…",
            Action::SignIn(_) => "Signing in…",
            Action::SignOut => "Signing out…",
        });
        let runtime = cx.global::<DesktopState>().runtime.clone();
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let discover = matches!(action, Action::Discover);
        let job = runtime.spawn(async move { account::run(&instance, action, tx).await });
        e.account.pending = Some(Pending {
            discover,
            id: attempt,
            task: job.abort_handle(),
        });
        cx.spawn_in(w, async move |ws, w| {
            while let Some(event) = rx.recv().await {
                if ws
                    .update_in(w, |ws, w, cx| {
                        let Some(e) = ws.provider_editor.as_mut().filter(|e| {
                            e.account.pending.as_ref().is_some_and(|j| j.id == attempt)
                        }) else {
                            return;
                        };
                        match event {
                            Event::Verifying => {
                                e.account.message = Some("Verifying sign-in…");
                                e.account.browser = None;
                                e.account.terminal = None;
                            }
                            Event::Browser { url, response } => {
                                e.account.message =
                                    Some("This provider wants to open a sign-in page.");
                                e.account.browser = Some((url, response));
                            }
                            Event::Terminal(manager, info) => {
                                let terminal = cx.new(|cx| {
                                    TerminalView::new(info, manager, runtime.clone(), cx)
                                });
                                terminal.read(cx).focus_handle(cx).focus(w, cx);
                                e.account.terminal = Some(terminal);
                                e.account.message = Some("Complete sign-in in the terminal.");
                            }
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
            }
            let result = job.await.map_err(std::io::Error::other).and_then(|r| r);
            let _ = ws.update_in(w, |ws, _, cx| {
                let Some(e) = ws
                    .provider_editor
                    .as_mut()
                    .filter(|e| e.account.pending.as_ref().is_some_and(|j| j.id == attempt))
                else {
                    return;
                };
                e.account.clear_job();
                e.account.message = None;
                match result {
                    Ok(state) => {
                        e.error = state.notice.clone();
                        e.account.state = Some(state);
                    }
                    Err(err) => e.error = Some(err.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn edit_account_variables(&mut self, w: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(e) = self.provider_editor.as_mut() else {
            return;
        };
        if e.account.pending.is_some() {
            return;
        }
        let Some(original) = e.original.as_ref() else {
            return;
        };
        let names = e
            .account
            .method(original)
            .map(|m| m.variables.clone())
            .unwrap_or_default();
        let sources = self.credential_settings.read(cx).secret_sources();
        let target = format!("acp/{}", original.id);
        let mut first = None;
        for name in names {
            if e.variables
                .iter()
                .any(|row| row.name.read(cx).value().as_str() == name)
                || sources
                    .read(cx)
                    .bindings()
                    .iter()
                    .any(|b| b.target == target && b.variable == name)
            {
                continue;
            }
            let row = environment::VariableRow::new(&name, None, w, cx);
            e.track_edits(&row.name, cx);
            e.track_edits(&row.value, cx);
            first.get_or_insert(row.value.clone());
            e.variables.push(row);
        }
        e.account.return_after_save = true;
        e.page = Page::Edit;
        e.error = None;
        if let Some(field) = first {
            field.update(cx, |f, c| f.focus(w, c));
        } else if let Some(row) = e.variables.first() {
            row.name.update(cx, |f, c| f.focus(w, c))
        } else {
            e.name.update(cx, |f, c| f.focus(w, c))
        }
        cx.notify();
    }
    fn account_method_picker(&self, cx: &Context<'_, Self>) -> AnyElement {
        let Some(e) = self.provider_editor.as_ref() else {
            return div().into_any_element();
        };
        let Some(list) = e.account.state.as_ref().map(|s| &s.methods) else {
            return div().into_any_element();
        };
        let selected = e
            .account
            .selected
            .as_ref()
            .and_then(|s| list.iter().position(|m| &m.id == s));
        let value = |i: usize| SharedString::from(format!("method-{i}"));
        let mut options = vec![DropdownOption {
            value: "default".into(),
            label: "Provider default".into(),
            description: None,
        }];
        options.extend(list.iter().enumerate().map(|(i, m)| DropdownOption {
            value: value(i),
            label: m.name.clone(),
            description: None,
        }));
        let entity = cx.weak_entity();
        let toggle = std::rc::Rc::new(move |_: &mut Window, cx: &mut App| {
            let _ = entity.update(cx, |ws, cx| {
                if let Some(e) = &mut ws.provider_editor {
                    e.account.menu = !e.account.menu;
                    cx.notify()
                }
            });
        });
        let entity = cx.weak_entity();
        let select = std::rc::Rc::new(move |choice: &str, _: &mut Window, cx: &mut App| {
            let choice = choice.to_owned();
            let _ = entity.update(cx, |ws, cx| {
                if let Some(e) = &mut ws.provider_editor {
                    e.account.menu = false;
                    e.account.selected = choice
                        .strip_prefix("method-")
                        .and_then(|s| s.parse::<usize>().ok())
                        .and_then(|i| e.account.state.as_ref()?.methods.get(i))
                        .map(|m| m.id.clone());
                    e.error = None;
                    e.account.message = None;
                    cx.notify()
                }
            });
        });
        let selected_value = selected.map(value);
        div()
            .w(px(182.))
            .max_w_full()
            .child(bruhs_ui::dropdown::Dropdown::new(
                "acp-account-method",
                "Sign-in method",
                selected_value.as_deref().unwrap_or("default"),
                vec![DropdownGroup {
                    label: None,
                    mark: None,
                    options,
                }],
                if e.account.menu {
                    Listbox::Popover
                } else {
                    Listbox::Closed
                },
                toggle,
                select,
            ))
            .into_any_element()
    }
    pub(super) fn account_body(&self, wide: bool, cx: &Context<'_, Self>) -> AnyElement {
        let Some(e) = self.provider_editor.as_ref() else {
            return div().into_any_element();
        };
        let a = &e.account;
        let Some(instance) = e.original.as_ref() else {
            return div().into_any_element();
        };
        let busy = a.pending.is_some();
        let confirmed = a.state.as_ref().is_some_and(|s| s.confirmed);
        let method = a.method(instance);
        let has_methods = method.is_some();
        let status = a.message.unwrap_or_else(|| {
            if confirmed {
                "Signed in"
            } else if a.state.is_none() && e.error.is_some() {
                "Sign-in discovery failed."
            } else if !has_methods {
                "No in-app sign-in advertised. Follow the provider’s documentation to finish setup."
            } else if a.state.as_ref().is_some_and(|s| s.ready) {
                "This provider can open a session."
            } else {
                "Sign in to use this provider."
            }
        });
        let mut controls = div().flex().items_center().flex_wrap().gap_2();
        if a.pending.as_ref().is_some_and(|p| p.discover) {
            controls = controls.child(editor_button(
                "acp-account-loading",
                "Sign in",
                WebButtonVariant::Outline,
                true,
                cx,
                |_, _, _| {},
            ));
        } else if busy {
            controls = controls.child(editor_button(
                "acp-account-cancel",
                "Cancel",
                WebButtonVariant::Outline,
                false,
                cx,
                Self::cancel_provider_account,
            ));
        } else if has_methods {
            if a.state.as_ref().is_some_and(|s| s.methods.len() > 1) {
                controls = controls.child(self.account_method_picker(cx));
            }
            controls = controls.child(editor_button(
                "acp-account-sign-in",
                if e.error.is_some() {
                    "Retry sign-in"
                } else if confirmed {
                    "Change account"
                } else if method.is_some_and(|m| m.kind == Kind::Credentials) {
                    "Verify credentials"
                } else {
                    "Sign in"
                },
                WebButtonVariant::Outline,
                false,
                cx,
                |ws, w, cx| {
                    if let Some(id) = ws
                        .provider_editor
                        .as_ref()
                        .and_then(|e| e.account.method(e.original.as_ref()?))
                        .map(|m| m.id.clone())
                    {
                        ws.provider_account_job(Action::SignIn(id), w, cx)
                    }
                },
            ));
        } else {
            controls = controls.child(editor_button(
                "acp-account-refresh",
                "Check again",
                WebButtonVariant::Outline,
                false,
                cx,
                |ws, w, cx| ws.provider_account_job(Action::Discover, w, cx),
            ));
        }
        if !busy && a.state.as_ref().is_some_and(|s| s.ready && s.logout) {
            controls = controls.child(editor_button(
                "acp-account-logout",
                "Sign out",
                WebButtonVariant::Ghost,
                false,
                cx,
                |ws, w, cx| ws.provider_account_job(Action::SignOut, w, cx),
            ));
        }
        let description = div()
            .min_w_0()
            .flex_1()
            .flex()
            .flex_col()
            .gap_1()
            .child("Account")
            .child(
                div()
                    .id("acp-account-status")
                    .role(Role::Status)
                    .text_xs()
                    .text_color(theme::rambutan_400())
                    .child(status),
            );
        let mut body = div().flex().flex_col().gap_3().min_h(px(268.)).child(
            div()
                .flex()
                .gap_3()
                .when(wide, |r| r.items_center().justify_between())
                .when(!wide, |r| r.flex_col())
                .child(description)
                .child(controls),
        );
        if !busy {
            if let Some(link) = a.state.as_ref().and_then(|s| s.website.clone()) {
                body = body.child(editor_button(
                    "acp-account-docs",
                    "Open provider website",
                    WebButtonVariant::Outline,
                    false,
                    cx,
                    move |_, _, cx| cx.open_url(&link),
                ));
            }
            if let Some(m) = method {
                if !m.description.is_empty() {
                    body = body.child(div().text_sm().child(m.description.clone()));
                }
                if m.kind == Kind::Credentials {
                    if !m.variables.is_empty() {
                        body = body.child(
                            div()
                                .text_sm()
                                .child(format!("Variables: {}", m.variables.join(", "))),
                        );
                    }
                    let mut row = div().flex().gap_2().child(editor_button(
                        "acp-account-variables",
                        "Edit variables",
                        WebButtonVariant::Outline,
                        false,
                        cx,
                        Self::edit_account_variables,
                    ));
                    if let Some(link) = &m.link {
                        let link = link.clone();
                        row = row.child(editor_button(
                            "acp-account-key-link",
                            "Open provider website",
                            WebButtonVariant::Ghost,
                            false,
                            cx,
                            move |_, _, cx| cx.open_url(&link),
                        ));
                    }
                    body = body.child(row);
                }
            }
        }
        if let Some((url, _)) = &a.browser {
            body = body.child(div().text_sm().child(url.clone())).child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(editor_button(
                        "acp-account-browser",
                        "Open browser",
                        WebButtonVariant::Primary,
                        false,
                        cx,
                        |ws, _, cx| {
                            if let Some(e) = &mut ws.provider_editor {
                                if let Some((url, reply)) = e.account.browser.take() {
                                    cx.open_url(&url);
                                    let _ = reply.send(true);
                                    e.account.message = Some("Waiting for the provider…");
                                    cx.notify();
                                }
                            }
                        },
                    ))
                    .child(editor_button(
                        "acp-account-copy",
                        "Copy sign-in link",
                        WebButtonVariant::Ghost,
                        false,
                        cx,
                        |ws, _, cx| {
                            if let Some((url, _)) = ws
                                .provider_editor
                                .as_ref()
                                .and_then(|e| e.account.browser.as_ref())
                            {
                                cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
                            }
                        },
                    ))
                    .child(editor_button(
                        "acp-account-decline",
                        "Decline",
                        WebButtonVariant::Ghost,
                        false,
                        cx,
                        |ws, _, cx| {
                            if let Some(e) = &mut ws.provider_editor {
                                if let Some((_, reply)) = e.account.browser.take() {
                                    let _ = reply.send(false);
                                    e.account.message = Some("Sign-in link declined.");
                                    cx.notify();
                                }
                            }
                        },
                    )),
            );
        }
        if let Some(terminal) = &a.terminal {
            body = body.child(
                div()
                    .w_full()
                    .h(px(220.))
                    .overflow_hidden()
                    .child(terminal.clone()),
            );
        }
        body.into_any_element()
    }
    pub(super) fn account_footer(&self, cx: &Context<'_, Self>) -> AnyElement {
        let Some(e) = self.provider_editor.as_ref() else {
            return div().into_any_element();
        };
        let a = &e.account;
        let done = a.from_edit || a.state.as_ref().is_some_and(|s| s.confirmed);
        editor_button(
            "acp-account-done",
            if done { "Done" } else { "Skip for now" },
            if done {
                WebButtonVariant::Primary
            } else {
                WebButtonVariant::Outline
            },
            a.pending.as_ref().is_some_and(|p| !p.discover),
            cx,
            Self::finish_provider_account,
        )
    }
}
