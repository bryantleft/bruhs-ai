use super::*;

#[derive(IntoElement)]
pub(in crate::views::workspace) struct SecretProviderDialog(pub Entity<SourceSettings>);

impl RenderOnce for SecretProviderDialog {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.0.update(cx, |view, cx| view.render_dialog(window, cx))
    }
}

impl SourceSettings {
    pub(in crate::views::workspace) fn dialog_open(&self) -> bool {
        matches!(
            self.editor,
            Some(Editor::Source { .. } | Editor::Remove { .. })
        )
    }
    pub(in crate::views::workspace) fn focus_bounds(&self) -> (FocusHandle, FocusHandle) {
        (self.start.clone(), self.end.clone())
    }
    pub(in crate::views::workspace) fn needs_reset(&self) -> bool {
        self.editor.is_some() || !self.revealed.is_empty() || self.restore_focus
    }
    pub(in crate::views::workspace) fn leave_settings(&mut self) {
        self.close_dialog();
        self.revealed.clear();
        self.restore_focus = false;
    }
    pub(super) fn close_dialog(&mut self) {
        self.editor = None;
        self.form_events.clear();
        self.error = None;
        self.restore_focus = true;
    }
    pub(in crate::views::workspace) fn dismiss(&mut self, cx: &mut Context<'_, Self>) {
        if self.can_dismiss() {
            self.close_dialog();
            cx.notify();
        }
    }

    fn can_dismiss(&self) -> bool {
        !self.busy
            || matches!(
                self.editor,
                Some(Editor::Remove {
                    consumers: None,
                    ..
                })
            )
    }

    pub(super) fn account_display(
        &self,
        id: &str,
        source: &Source,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let row = div()
            .flex()
            .items_center()
            .gap_1()
            .min_w_0()
            .text_xs()
            .text_color(theme::rambutan_400())
            .child("1Password ·");
        if source.account.is_empty() {
            return row.child("Default account").into_any_element();
        }
        let revealed = self.revealed.contains(id);
        let key = id.to_owned();
        let selector = format!("toggle-secret-account-{id}");
        let toggle = div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .min_w_0()
            .truncate()
            .role(Role::Button)
            .tab_index(0)
            .cursor_pointer()
            .aria_label(if revealed {
                "Hide account"
            } else {
                "Reveal account"
            })
            .border_1()
            .rounded_sm()
            .border_color(gpui::transparent_black())
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .child(if revealed {
                source.account.clone()
            } else {
                "••••••••".into()
            });
        row.child(on_press_view(toggle, cx, move |this, _, cx| {
            if !this.revealed.remove(&key) {
                this.revealed.insert(key.clone());
            }
            cx.notify();
        }))
        .into_any_element()
    }

    pub(super) fn source_errors(&self, cx: &App) -> (Option<&'static str>, Option<&'static str>) {
        let Some(Editor::Source {
            id,
            name,
            account,
            use_default,
        }) = &self.editor
        else {
            return (None, None);
        };
        let name = name.read(cx).value();
        let account = account.read(cx).value();
        let name = name.trim();
        let account = account.trim();
        let existing = self.data.sources.get(id);
        let name_error = if name.is_empty() {
            Some("Enter a name.")
        } else if existing.is_some_and(|s| s.name == name) {
            None
        } else if name.chars().count() > 64 {
            Some("Use 64 characters or fewer.")
        } else if !name.bytes().any(|c| c.is_ascii_alphanumeric()) {
            Some("Include at least one letter or digit.")
        } else if self
            .data
            .sources
            .iter()
            .any(|(other, s)| other != id && s.name.eq_ignore_ascii_case(name))
        {
            Some("A provider with this name already exists.")
        } else {
            None
        };
        // Editing the label of an older provider preserves its original account.
        let account_error = if *use_default
            || (!account.is_empty() && existing.is_some_and(|s| s.account == account))
        {
            None
        } else if account.is_empty() {
            Some("Enter the 1Password account.")
        } else if account.len() > 253
            || !account.as_bytes()[0].is_ascii_alphanumeric()
            || !account
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
        {
            Some("Use an account shorthand, sign-in address without https://, or ID.")
        } else {
            None
        };
        (name_error, account_error)
    }

    fn render_dialog(&self, window: &Window, cx: &Context<'_, Self>) -> AnyElement {
        let mut body = div()
            .id("secret-provider-dialog-body")
            .min_h_0()
            .overflow_y_scroll()
            .px_6()
            .pb_4()
            .flex()
            .flex_col()
            .gap_3();
        let muted = |text: String| {
            div()
                .text_sm()
                .text_color(theme::rambutan_400())
                .child(text)
        };
        let errors = self.source_errors(cx);
        let (title, verb, removal) = match &self.editor {
            Some(Editor::Source {
                id,
                name,
                account,
                use_default,
            }) => {
                let existing = self.data.sources.get(id);
                let field = |label: &'static str,
                             input: &Entity<InputState>,
                             error: Option<&'static str>,
                             selector: &'static str| {
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().text_sm().child(label))
                        .child(Input::new(input).aria_label(label).disabled(self.busy))
                        .when_some(error.filter(|_| self.submitted), |el, message| {
                            el.child(
                                div()
                                    .id(selector)
                                    .debug_selector(move || selector.into())
                                    .role(Role::Alert)
                                    .aria_label(message)
                                    .text_xs()
                                    .text_color(theme::lychee_300())
                                    .child(message),
                            )
                        })
                };
                let source = cx.entity().downgrade();
                let host_default = bruhs_ui::checkbox::Checkbox::new("secret-provider-use-default")
                    .label("Use this host’s default account")
                    .checked(*use_default)
                    .disabled(self.busy)
                    .on_click(move |enabled, _, cx| {
                        let _ = source.update(cx, |this, cx| {
                            if let Some(Editor::Source { use_default, .. }) = &mut this.editor {
                                *use_default = *enabled;
                                this.error = None;
                                cx.notify();
                            }
                        });
                    });
                let account_field = div().flex().flex_col().gap_1()
                    .when(!*use_default, |el| el.child(field("Account", account, errors.1, "secret-provider-account-error"))
                        .child(div().text_xs().text_color(theme::rambutan_400()).child(
                            "Use the shorthand, sign-in address without https://, or ID from op account list.")))
                    .child(host_default);
                body = body
                    .child(muted("This host uses the 1Password CLI to read fields for agent variables and text generator API keys.".into()))
                    .child(field("Name", name, errors.0, "secret-provider-name-error"))
                    .child(account_field);
                (
                    existing
                        .map(|s| format!("Edit {}", s.name))
                        .unwrap_or_else(|| "Add a 1Password provider".into()),
                    if existing.is_some() {
                        "Save changes"
                    } else {
                        "Add provider"
                    },
                    false,
                )
            }
            Some(Editor::Remove { id, consumers }) => {
                let name = self
                    .data
                    .sources
                    .get(id)
                    .map_or(id.as_str(), |s| s.name.as_str());
                let uses: Vec<_> = self
                    .data
                    .bindings
                    .iter()
                    .filter(|b| b.source == *id)
                    .map(|b| {
                        let target = b
                            .target
                            .parse::<AgentProvider>()
                            .map(|p| p.label())
                            .unwrap_or(&b.target);
                        format!("{target} · {}", b.variable)
                    })
                    .chain(consumers.iter().flatten().cloned())
                    .collect();
                if consumers.is_none() && self.busy {
                    body = body.child(muted("Checking linked settings…".into()));
                } else if consumers.is_some() {
                    body = body.child(muted(if uses.is_empty() {
                        format!("No settings read from {name}. Its fields stay in 1Password.")
                    } else {
                        format!("Removing {name} will unset these settings. The fields stay in 1Password.")
                    }));
                    if !uses.is_empty() {
                        body = body.child(
                            div()
                                .id("secret-provider-dependents")
                                .role(Role::List)
                                .max_h(px(192.))
                                .overflow_y_scroll()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .children(uses.into_iter().enumerate().map(|(index, label)| {
                                    div()
                                        .id(("secret-consumer", index))
                                        .role(Role::ListItem)
                                        .text_sm()
                                        .child(format!("•  {label}"))
                                })),
                        );
                    }
                }
                (format!("Remove {name}?"), "Remove provider", true)
            }
            _ => return div().into_any_element(),
        };
        if let Some(error) = &self.error {
            body = body.child(
                div()
                    .id("secret-dialog-save-error")
                    .role(Role::Alert)
                    .text_sm()
                    .text_color(theme::lychee_300())
                    .child(error.clone()),
            );
        }
        let disabled = self.busy
            || if removal {
                matches!(
                    self.editor,
                    Some(Editor::Remove {
                        consumers: None,
                        ..
                    })
                )
            } else {
                self.submitted && errors != (None, None)
            };
        let submit = web_button(
            "save-secret-source",
            if removal {
                WebButtonVariant::Destructive
            } else {
                WebButtonVariant::Primary
            },
            WebButtonSize::Small,
            ButtonState::disabled_if(disabled),
        )
        .debug_selector(|| "save-secret-source".into())
        .aria_label(verb)
        .child(verb);
        let cancel = web_button(
            "cancel-secret-source",
            WebButtonVariant::Outline,
            WebButtonSize::Small,
            ButtonState::disabled_if(!self.can_dismiss()),
        )
        .track_focus(&self.cancel)
        .debug_selector(|| "cancel-secret-source".into())
        .child("Cancel");
        let footer = div()
            .flex_shrink_0()
            .flex()
            .justify_end()
            .gap_2()
            .px_6()
            .py_4()
            .when(removal, |el| {
                el.border_t_1()
                    .border_color(theme::ring_modal())
                    .bg(theme::longan_950())
            })
            .child(on_press_view(cancel, cx, |this, _, cx| this.dismiss(cx)))
            .child(if disabled {
                submit
            } else {
                on_press_view(submit, cx, |this, _, cx| this.save(cx))
            });
        let header = div()
            .flex_shrink_0()
            .flex()
            .items_start()
            .justify_between()
            .gap_2()
            .px_6()
            .pt_5()
            .pb_2()
            .child(
                div()
                    .min_w_0()
                    .font_family(theme::HEADING_FONT_FAMILY)
                    .text_size(px(18.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title.clone()),
            )
            .when(!removal, |el| {
                el.child(on_press_view(
                    web_button(
                        "close-secret-source",
                        WebButtonVariant::Ghost,
                        WebButtonSize::IconSmall,
                        ButtonState::disabled_if(self.busy),
                    )
                    .aria_label("Close dialog")
                    .child(icon_svg(
                        assets::CLOSE_ICON_PATH,
                        14.,
                        theme::rambutan_400(),
                    )),
                    cx,
                    |this, _, cx| this.dismiss(cx),
                ))
            });
        let dialog = div()
            .id("secret-provider-dialog")
            .debug_selector(|| "secret-provider-dialog".into())
            .role(if removal {
                Role::AlertDialog
            } else {
                Role::Dialog
            })
            .aria_label(title)
            .track_focus(&self.start)
            .tab_group()
            .on_action(cx.listener(|this, _: &bruhs_ui::input::Enter, _, cx| {
                cx.stop_propagation();
                this.save(cx);
            }))
            .flex()
            .flex_col()
            .overflow_hidden()
            .w(px(if removal { 512. } else { 448. }).min(window.viewport_size().width - px(24.)))
            .max_h(window.viewport_size().height - px(32.))
            .rounded_xl()
            .border_1()
            .border_color(theme::ring_modal())
            .bg(theme::longan_900())
            .text_color(theme::rambutan_100())
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(header)
            .child(body)
            .child(footer)
            .child(div().size(px(0.)).track_focus(&self.end));
        div()
            .id("secret-provider-backdrop")
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::longan_950().opacity(0.72))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                if !removal {
                    this.dismiss(cx);
                }
            }))
            .child(dialog)
            .into_any_element()
    }
}
