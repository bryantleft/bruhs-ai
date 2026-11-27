//! Configure the host’s named 1Password sources and check their resolution status.
use super::updates_ui::icon_svg;
use super::*;
use bruhs_core::secret_sources::{self, Binding, Change, SecretSources, Source};
use bruhs_models::agent::AgentProvider;

mod check;
mod dialog;
pub(super) use dialog::SecretProviderDialog;

#[derive(Clone, Copy, PartialEq, Eq)]
enum VariableMenu {
    Runtime,
    Source,
}

enum Editor {
    Source {
        id: String,
        name: Entity<InputState>,
        account: Entity<InputState>,
        use_default: bool,
    },
    Variable {
        source: String,
        runtime: AgentProvider,
        instance: Entity<InputState>,
        original: Option<secret_sources::SecretReference>,
        menu: Option<VariableMenu>,
        variable: Entity<InputState>,
        reference: Entity<InputState>,
    },
    Remove {
        id: String,
        consumers: Option<Vec<String>>,
    },
}
pub(super) struct SourceSettings {
    start: FocusHandle,
    end: FocusHandle,
    cancel: FocusHandle,
    trigger: FocusHandle,
    trigger_id: String,
    restore_focus: bool,
    submitted: bool,
    form_events: Vec<Subscription>,
    revealed: std::collections::BTreeSet<String>,
    data: SecretSources,
    editor: Option<Editor>,
    busy: bool,
    error: Option<String>,
}
impl SourceSettings {
    #[cfg(test)]
    pub(super) fn add_test_source(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        self.data.sources.insert(
            id.to_owned(),
            Source {
                name: "Test vault".into(),
                account: "test.example".into(),
            },
        );
        cx.notify();
    }
    pub(super) fn bindings(&self) -> &[bruhs_core::secret_sources::Binding] {
        &self.data.bindings
    }

    pub(super) fn sources(&self) -> &BTreeMap<String, Source> {
        &self.data.sources
    }

    pub(super) fn new(cx: &mut App) -> Self {
        Self {
            start: cx.focus_handle(),
            end: cx.focus_handle(),
            cancel: cx.focus_handle().tab_stop(true),
            trigger: cx.focus_handle().tab_stop(true),
            trigger_id: "add-secret-provider".into(),
            restore_focus: false,
            submitted: false,
            form_events: Vec::new(),
            revealed: Default::default(),
            data: SecretSources::default(),
            editor: None,
            busy: false,
            error: None,
        }
    }
    pub(super) fn reload(&mut self, cx: &mut Context<'_, Self>) {
        if !self.busy {
            self.request(None, cx);
        }
    }
    fn request(&mut self, edit: Option<Change>, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let editing = edit.is_some();
        let result = cx.background_spawn(async move {
            match edit {
                Some(edit) => secret_sources::change(edit),
                None => secret_sources::load(),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = result.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(data) => {
                        if editing && this.dialog_open() {
                            if matches!(this.editor, Some(Editor::Remove { .. })) {
                                this.trigger_id = "add-secret-provider".into();
                            }
                            this.close_dialog();
                        }
                        this.revealed.retain(|id| data.sources.contains_key(id));
                        this.data = data;
                        if editing {
                            this.form_events.clear();
                            this.editor = None;
                        }
                    }
                    Err(e) => this.error = Some(e.to_string()),
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn field(
        label: &'static str,
        value: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Entity<InputState> {
        cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(label)
                .default_value(value.to_owned())
        })
    }
    fn source(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        self.submitted = false;
        self.trigger_id = id.as_ref().map_or_else(
            || "add-secret-provider".into(),
            |id| format!("edit-source-{id}"),
        );
        let id = id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let source = self.data.sources.get(&id);
        let name = Self::field(
            "Personal",
            source.map_or("", |s| s.name.as_str()),
            window,
            cx,
        );
        let account = Self::field(
            "my.1password.com",
            source.map_or("", |s| s.account.as_str()),
            window,
            cx,
        );
        let use_default = source.is_some_and(|s| s.account.is_empty());
        self.form_events = [&name, &account]
            .into_iter()
            .map(|field| {
                cx.subscribe(field, |this, _, event: &InputEvent, cx| {
                    if *event == InputEvent::Change {
                        this.error = None;
                        cx.notify();
                    }
                })
            })
            .collect();
        name.update(cx, |input, cx| input.focus(window, cx));
        self.editor = Some(Editor::Source {
            id,
            name,
            account,
            use_default,
        });
        self.error = None;
        cx.notify();
    }
    fn variable(
        &mut self,
        source: String,
        existing: Option<&Binding>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let target = existing.map_or("claude", |b| b.target.as_str());
        let runtime = target
            .split('/')
            .next()
            .and_then(|p| p.parse().ok())
            .unwrap_or(AgentProvider::Claude);
        let instance = Self::field(
            "ACP instance ID",
            target.strip_prefix("acp/").unwrap_or(""),
            window,
            cx,
        );
        let variable = Self::field(
            "Variable name",
            existing.map_or("", |b| b.variable.as_str()),
            window,
            cx,
        );
        let reference = Self::field(
            "op://vault/item/field",
            existing.map_or("", |b| b.reference.as_str()),
            window,
            cx,
        );
        self.submitted = false;
        self.form_events = vec![cx.subscribe(&reference, |_, _, event: &InputEvent, cx| {
            if matches!(
                event,
                InputEvent::Change | InputEvent::Blur | InputEvent::Focus
            ) {
                cx.notify();
            }
        })];
        reference.update(cx, |input, cx| input.focus(window, cx));
        self.editor = Some(Editor::Variable {
            source,
            runtime,
            instance,
            original: existing.map(|binding| secret_sources::SecretReference {
                source: binding.source.clone(),
                reference: binding.reference.clone(),
            }),
            menu: None,
            variable,
            reference,
        });
        self.error = None;
        cx.notify();
    }
    fn save(&mut self, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        if matches!(self.editor, Some(Editor::Source { .. })) {
            self.submitted = true;
            if self.source_errors(cx) != (None, None) {
                cx.notify();
                return;
            }
        }
        if let Some(Editor::Variable {
            source, reference, ..
        }) = &self.editor
        {
            self.submitted = true;
            let field = secret_sources::SecretReference {
                source: source.clone(),
                reference: reference.read(cx).value().trim().to_owned(),
            };
            if self.data.can_check_reference(&field).is_err() {
                cx.notify();
                return;
            }
        }
        let value = |input: &Entity<InputState>| input.read(cx).value().trim().to_owned();
        let change = match &self.editor {
            Some(Editor::Source {
                id,
                name,
                account,
                use_default,
            }) => Change::SaveSource(
                id.clone(),
                Source {
                    name: value(name),
                    account: if *use_default {
                        String::new()
                    } else {
                        value(account)
                    },
                },
            ),
            Some(Editor::Variable {
                source,
                runtime,
                instance,
                variable,
                reference,
                ..
            }) => Change::Bind(Binding {
                source: source.clone(),
                target: if *runtime == AgentProvider::Acp {
                    format!("acp/{}", value(instance))
                } else {
                    runtime.as_str().into()
                },
                variable: value(variable),
                reference: value(reference),
            }),
            Some(Editor::Remove {
                id,
                consumers: Some(_),
            }) => Change::RemoveSource(id.clone()),
            Some(Editor::Remove { .. }) => return,
            None => return,
        };
        self.request(Some(change), cx)
    }
    fn button(
        &self,
        id: String,
        label: &str,
        caption: &'static str,
        variant: WebButtonVariant,
        cx: &Context<'_, Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<'_, Self>) + 'static,
    ) -> AnyElement {
        let track = self.trigger_id == id;
        let selector = id.clone();
        let el = web_button(
            SharedString::from(id),
            variant,
            WebButtonSize::Small,
            ButtonState::disabled_if(self.busy),
        )
        .debug_selector(move || selector)
        .when(track, |el| el.track_focus(&self.trigger))
        .aria_label(label.to_owned())
        .child(caption);
        if self.busy {
            el.into_any_element()
        } else {
            on_press_view(el, cx, action).into_any_element()
        }
    }
    fn controls(&self, id: &str, source: &Source, cx: &Context<'_, Self>) -> AnyElement {
        let edit = id.to_owned();
        let add = edit.clone();
        let remove = edit.clone();
        div()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(self.button(
                format!("edit-source-{id}"),
                &format!("Edit {}", source.name),
                "Edit",
                WebButtonVariant::Ghost,
                cx,
                move |this, window, cx| this.source(Some(edit.clone()), window, cx),
            ))
            .child(self.button(
                format!("bind-source-{id}"),
                &format!("Add variable from {}", source.name),
                "Add variable",
                WebButtonVariant::Ghost,
                cx,
                move |this, window, cx| this.variable(add.clone(), None, window, cx),
            ))
            .child(self.button(
                format!("remove-source-{id}"),
                &format!("Remove {}", source.name),
                "Remove",
                WebButtonVariant::Ghost,
                cx,
                move |this, window, cx| this.confirm_remove(remove.clone(), window, cx),
            ))
            .into_any_element()
    }
    fn confirm_remove(&mut self, id: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        self.trigger_id = format!("remove-source-{id}");
        self.editor = Some(Editor::Remove {
            id: id.clone(),
            consumers: None,
        });
        self.cancel.focus(window, cx);
        let source = id.clone();
        let query = cx.background_spawn(async move {
            let mut uses = bruhs_core::text_generators::secret_source_consumers(&source)?
                .into_iter()
                .map(|g| format!("{g} · API key"))
                .collect::<Vec<_>>();
            uses.extend(
                bruhs_core::agent_extensions::environment::source_consumers(&source)
                    .map_err(|e| e.to_string())?,
            );
            uses.extend(
                bruhs_core::usage_hubs::source_consumers(&source).map_err(|e| e.to_string())?,
            );
            Ok::<_, String>(uses)
        });
        cx.spawn(async move |this, cx| {
            let result = query.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                if let Some(Editor::Remove {
                    id: current,
                    consumers,
                }) = &mut this.editor
                {
                    if *current == id {
                        match result {
                            Ok(found) => *consumers = Some(found),
                            Err(error) => this.error = Some(error),
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify()
    }
    fn variable_menu(
        &self,
        kind: VariableMenu,
        selected: &str,
        open: Option<VariableMenu>,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let (id, label, options) = match kind {
            VariableMenu::Runtime => (
                "secret-variable-runtime",
                "Agent runtime",
                AgentProvider::ALL
                    .into_iter()
                    .map(|p| DropdownOption {
                        value: p.as_str().into(),
                        label: p.label().into(),
                        description: None,
                    })
                    .collect(),
            ),
            VariableMenu::Source => (
                "secret-variable-source",
                "1Password reference source",
                self.data
                    .sources
                    .iter()
                    .map(|(id, s)| DropdownOption {
                        value: id.clone().into(),
                        label: s.name.clone(),
                        description: None,
                    })
                    .collect(),
            ),
        };
        let view = cx.entity().downgrade();
        let select = view.clone();
        let dropdown = bruhs_ui::dropdown::Dropdown::new(
            id,
            label,
            selected,
            vec![DropdownGroup {
                label: None,
                mark: None,
                options,
            }],
            if open == Some(kind) {
                Listbox::Popover
            } else {
                Listbox::Closed
            },
            Rc::new(move |_, cx| {
                let _ = view.update(cx, |this, cx| {
                    if this.busy {
                        return;
                    }
                    if let Some(Editor::Variable { menu, .. }) = &mut this.editor {
                        *menu = (*menu != Some(kind)).then_some(kind);
                        cx.notify();
                    }
                });
            }),
            Rc::new(move |value, window, cx| {
                let _ = select.update(cx, |this, cx| {
                    if this.busy {
                        return;
                    }
                    let Some(Editor::Variable {
                        menu,
                        source,
                        runtime,
                        reference,
                        original,
                        ..
                    }) = &mut this.editor
                    else {
                        return;
                    };
                    *menu = None;
                    match kind {
                        VariableMenu::Runtime => {
                            if let Ok(provider) = value.parse() {
                                *runtime = provider;
                            }
                        }
                        VariableMenu::Source => {
                            if !this.data.sources.contains_key(value) {
                                return;
                            }
                            if value != source.as_str() {
                                *source = value.to_owned();
                                let saved = original
                                    .as_ref()
                                    .filter(|r| r.source == value)
                                    .map(|r| r.reference.clone())
                                    .unwrap_or_default();
                                reference
                                    .update(cx, |input, cx| input.set_value(saved, window, cx));
                                this.submitted = false;
                                this.error = None;
                            }
                            let focus = reference.focus_handle(cx);
                            cx.defer_in(window, move |_, window, cx| focus.focus(window, cx));
                        }
                    }
                    cx.notify();
                });
            }),
        );
        if kind == VariableMenu::Source {
            dropdown.icon_trigger("icons/shield.svg").into_any_element()
        } else {
            dropdown.into_any_element()
        }
    }
    fn editor(&self, editor: &Editor, window: &Window, cx: &Context<'_, Self>) -> AnyElement {
        let wrap = div().flex().flex_col().gap_2().py_2().min_w_0();
        let input = |e: &Entity<InputState>, label: &'static str| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_xs().child(label))
                .child(Input::new(e).aria_label(label))
        };
        let contents = match editor {
            Editor::Variable {
                runtime,
                instance,
                original,
                menu,
                variable,
                reference,
                source,
            } => {
                let field = secret_sources::SecretReference {
                    source: source.clone(),
                    reference: reference.read(cx).value().trim().to_owned(),
                };
                let validation = self.data.can_check_reference(&field);
                let feedback = if self.submitted
                    || (!field.reference.is_empty()
                        && !reference.focus_handle(cx).is_focused(window))
                {
                    validation.as_ref().err().map(ToString::to_string)
                } else {
                    None
                };
                let check = self
                    .data
                    .sources
                    .get(source)
                    .filter(|_| validation.is_ok())
                    .map(|s| check::CheckButton {
                        id: "secret-variable-check".into(),
                        label: "Check 1Password reference".into(),
                        account: s.account.clone(),
                        reference: field,
                        disabled: self.busy,
                    });
                let field = div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_xs().child("1Password reference"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(self.variable_menu(VariableMenu::Source, source, *menu, cx))
                            .child(
                                div().min_w_0().flex_1().child(
                                    Input::new(reference)
                                        .aria_label("1Password reference")
                                        .disabled(self.busy),
                                ),
                            )
                            .children(check),
                    )
                    .when_some(feedback, |el, msg| {
                        el.child(
                            div()
                                .id("secret-variable-reference-error")
                                .debug_selector(|| "secret-variable-reference-error".into())
                                .role(Role::Alert)
                                .aria_label(msg.clone())
                                .text_xs()
                                .text_color(theme::lychee_300())
                                .child(msg),
                        )
                    });
                if original.is_some() {
                    let target = if *runtime == AgentProvider::Acp {
                        format!("ACP · {}", instance.read(cx).value())
                    } else {
                        runtime.label().into()
                    };
                    wrap.child(format!("{target} · {}", variable.read(cx).value()))
                        .child(field)
                } else {
                    wrap.child(self.variable_menu(
                        VariableMenu::Runtime,
                        runtime.as_str(),
                        *menu,
                        cx,
                    ))
                    .when(*runtime == AgentProvider::Acp, |el| {
                        el.child(input(instance, "ACP instance ID"))
                    })
                    .child(input(variable, "Environment variable"))
                    .child(field)
                }
            }
            Editor::Source { .. } | Editor::Remove { .. } => return div().into_any_element(),
        };

        contents
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(self.button(
                        "save-secret-source".into(),
                        "Save variable",
                        "Save",
                        WebButtonVariant::Primary,
                        cx,
                        |this, _, cx| this.save(cx),
                    ))
                    .child(self.button(
                        "cancel-secret-source".into(),
                        "Cancel secret provider changes",
                        "Cancel",
                        WebButtonVariant::Outline,
                        cx,
                        |this, _, cx| {
                            this.form_events.clear();
                            this.editor = None;
                            this.error = None;
                            cx.notify();
                        },
                    )),
            )
            .into_any_element()
    }
}
impl Render for SourceSettings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        if std::mem::take(&mut self.restore_focus) {
            self.trigger.focus(window, cx);
        }
        let mut body = div()
            .id("named-secret-sources")
            .flex()
            .flex_col()
            .min_w_0()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme::rambutan_100())
                            .child("Secret providers"),
                    )
                    .child(self.button(
                        "add-secret-provider".into(),
                        "Add a 1Password provider",
                        "+ Add provider",
                        WebButtonVariant::Outline,
                        cx,
                        |this, window, cx| this.source(None, window, cx),
                    )),
            );
        if self.data.sources.is_empty() && !self.busy {
            body = body.child(div().text_sm().child("No secret providers configured."))
                .child(div().text_xs().text_color(theme::rambutan_400()).child("Add a 1Password account to use its fields for provider variables and text generator API keys."));
        }
        for (id, s) in &self.data.sources {
            body = body.child(
                div()
                    .border_t_1()
                    .border_color(theme::longan_800())
                    .py_2()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(icon_svg("icons/shield.svg", 16., theme::rambutan_400()))
                                    .child(div().min_w_0().child(s.name.clone())),
                            )
                            .child(self.account_display(id, s, cx)),
                    )
                    .child(self.controls(id, s, cx)),
            );
            for b in self.data.bindings.iter().filter(|b| &b.source == id) {
                let binding = b.clone();
                let edit = b.clone();
                body = body.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(format!("{} · {}", b.target, b.variable))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme::rambutan_400())
                                        .child(b.reference.clone()),
                                ),
                        )
                        .child(check::CheckButton {
                            id: format!("check-{}-{}", b.target, b.variable),
                            label: format!("Check {} for {}", b.variable, b.target),
                            account: s.account.clone(),
                            reference: secret_sources::SecretReference {
                                source: b.source.clone(),
                                reference: b.reference.clone(),
                            },
                            disabled: self.busy,
                        })
                        .child(self.button(
                            format!("edit-{}-{}", b.target, b.variable),
                            &format!("Edit {} for {}", b.variable, b.target),
                            "Edit",
                            WebButtonVariant::Ghost,
                            cx,
                            move |this, window, cx| {
                                this.variable(edit.source.clone(), Some(&edit), window, cx)
                            },
                        ))
                        .child(self.button(
                            format!("unbind-{}-{}", b.target, b.variable),
                            &format!("Unset {} for {}", b.variable, b.target),
                            "Unset",
                            WebButtonVariant::Ghost,
                            cx,
                            move |this, _, cx| {
                                this.request(
                                    Some(Change::Unbind {
                                        target: binding.target.clone(),
                                        variable: binding.variable.clone(),
                                    }),
                                    cx,
                                )
                            },
                        )),
                );
            }
        }
        body.when_some(
            self.editor.as_ref().filter(|_| !self.dialog_open()),
            |e, editor| e.child(self.editor(editor, window, cx)),
        )
        .when_some(
            self.error.clone().filter(|_| !self.dialog_open()),
            |e, error| {
                e.child(
                    div()
                        .id("secret-source-error")
                        .role(Role::Alert)
                        .aria_label(error.clone())
                        .child(error),
                )
            },
        )
    }
}
