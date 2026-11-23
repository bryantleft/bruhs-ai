use super::super::updates_ui::icon_svg;
use super::*;

impl Workspace {
    fn generator_credential_field(
        &self,
        form: &Form,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let sources = form.sources.read(cx).sources();
        let literal = Input::new(&form.key)
            .aria_label("API key")
            .disabled(self.generators.busy);
        if sources.is_empty() && form.source.is_none() {
            return literal.into_any_element();
        }
        let mut choices = vec![("value", "Value")];
        for (id, source) in sources {
            choices.push((
                saved_option_value(&format!("source:{id}")),
                saved_option_value(&source.name),
            ));
        }
        let selected = form
            .source
            .as_ref()
            .map_or("value", |id| saved_option_value(&format!("source:{id}")));
        let missing = form
            .source
            .as_ref()
            .is_some_and(|id| !sources.contains_key(id));
        if missing {
            choices.push((selected, "Unavailable source"));
        }
        let id = "generator-key-source";
        let icon = if missing {
            assets::ALERT_TRIANGLE_ICON_PATH
        } else if form.source.is_some() {
            "icons/shield.svg"
        } else {
            assets::TOOL_PENCIL_ICON_PATH
        };
        let (toggle, select) = self.settings_dropdown_handlers(id, cx);
        let options = choices
            .into_iter()
            .map(|(value, label)| DropdownOption {
                value,
                label: label.to_owned(),
                description: None,
            })
            .collect();
        let picker = bruhs_ui::dropdown::Dropdown::new(
            id,
            "API key source",
            selected,
            vec![DropdownGroup {
                label: None,
                mark: None,
                options,
            }],
            self.settings_listbox(id),
            toggle,
            select,
        )
        .icon_trigger(icon);
        let source = if self.generators.busy {
            web_button(
                id,
                WebButtonVariant::Outline,
                WebButtonSize::Small,
                ButtonState::Disabled,
            )
            .aria_label("API key source")
            .child(icon_svg(icon, 14., theme::rambutan_400()))
            .into_any_element()
        } else {
            picker.into_any_element()
        };
        let mut controls = div()
            .flex()
            .items_start()
            .gap_2()
            .child(div().flex_shrink_0().child(source));
        let mut field = div().flex_1().min_w_0().flex().flex_col().gap_1();
        if form.source.is_some() {
            field = field.child(
                Input::new(&form.reference)
                    .aria_label("API key reference")
                    .disabled(self.generators.busy),
            );
            let value = form.reference.read(cx).value();
            let message = if missing {
                Some("Choose an available secret source.".to_owned())
            } else if (form.show_reference_error
                || !form.reference.focus_handle(cx).is_focused(window))
                && !value.is_empty()
            {
                validate_reference(value.trim())
                    .err()
                    .map(|e| e.to_string())
            } else {
                None
            };
            if let Some(message) = message {
                field = field.child(
                    div()
                        .id("generator-invalid-reference")
                        .debug_selector(|| "generator-invalid-reference".into())
                        .role(Role::Alert)
                        .text_xs()
                        .text_color(theme::persimmon_400())
                        .child(message),
                );
            }
        } else {
            field = field.child(literal);
        }
        controls = controls.child(field);
        controls.into_any_element()
    }
    pub(in crate::views::workspace) fn render_generator_dialog(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        let page = self.generators.dialog.as_ref()?;
        let mut body = div().flex().flex_col().gap_3();
        let mut foot = div().flex().justify_end().gap_2();
        let close = |ws: &mut Self, w: &mut Window, cx: &mut Context<'_, Self>| {
            ws.dismiss_generator();
            ws.focus_modal_dialog(w, cx);
            cx.notify();
        };
        let field = |label: &'static str, control: AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .text_color(theme::rambutan_300())
                        .child(label),
                )
                .child(control)
        };
        let title = match page {
            Page::Edit(f) => {
                if f.previous.is_none() {
                    let mut buttons = div().flex().gap_2();
                    for (preset, id, icon) in [
                        (
                            Preset::OpenAi,
                            "generator-openai",
                            assets::provider_logo_path(AgentProvider::Codex),
                        ),
                        (
                            Preset::OpenRouter,
                            "generator-openrouter",
                            assets::GIT_BRANCH_ICON_PATH,
                        ),
                        (
                            Preset::Anthropic,
                            "generator-anthropic",
                            assets::provider_logo_path(AgentProvider::Claude),
                        ),
                        (
                            Preset::Custom,
                            "generator-custom",
                            assets::TOOL_TERMINAL_ICON_PATH,
                        ),
                    ] {
                        let selected = preset == f.preset;
                        let button = web_button(
                            id,
                            WebButtonVariant::Outline,
                            WebButtonSize::Small,
                            ButtonState::disabled_if(self.generators.busy),
                        )
                        .aria_label(preset.label())
                        .debug_selector(move || id.into())
                        .flex_1()
                        .min_w_0()
                        .h(px(72.))
                        .flex_col()
                        .gap_2()
                        .when(selected, |el| {
                            el.border_color(theme::persimmon_400())
                                .bg(theme::persimmon_400().opacity(0.08))
                        })
                        .child(icon_svg(
                            icon,
                            18.,
                            if selected {
                                theme::persimmon_400()
                            } else {
                                theme::rambutan_400()
                            },
                        ))
                        .child(preset.label());
                        buttons = buttons.child(if self.generators.busy {
                            button
                        } else {
                            on_press_view(button, cx, move |ws, w, cx| {
                                ws.select_generator_option(
                                    "generator-preset",
                                    preset.label(),
                                    w,
                                    cx,
                                );
                            })
                        });
                    }
                    body = body.child(field("Service", buttons.into_any_element()));
                }
                if f.preset == Preset::Custom {
                    body = body.child(field(
                        "Base URL",
                        Input::new(&f.url)
                            .disabled(self.generators.busy)
                            .aria_label("Base URL")
                            .into_any_element(),
                    ));
                }
                body = body
                    .child(field(
                        if f.preset == Preset::Custom {
                            "API key (optional)"
                        } else {
                            "API key"
                        },
                        self.generator_credential_field(f, window, cx),
                    ))
                    .child(field(
                        "Name (optional)",
                        Input::new(&f.name)
                            .disabled(self.generators.busy)
                            .aria_label("Generator name")
                            .into_any_element(),
                    ));
                foot = foot
                    .child(self.gen_button("generator-cancel", "Cancel", false, cx, close))
                    .child(self.gen_button(
                        "generator-connect",
                        if self.generators.busy {
                            "Connecting…"
                        } else if f.previous.is_some() {
                            "Save and continue"
                        } else {
                            "Connect"
                        },
                        true,
                        cx,
                        |ws, _, cx| ws.save_generator(cx),
                    ));
                f.previous.as_ref().map_or_else(
                    || "Add a text generator".into(),
                    |g| format!("Edit {}", g.label),
                )
            }
            Page::Connected(g) => {
                body = body.child(format!(
                    "{} is connected and lists {} {}. Choose where to use it.",
                    g.label,
                    g.models.len(),
                    if g.models.len() == 1 {
                        "model"
                    } else {
                        "models"
                    }
                ));
                let writer =
                    self.loaded_delivery_preferences()
                        .and_then(|p| match &p.writer_model {
                            bruhs_models::delivery::WriterModel::Separate { model, .. } => {
                                Some(model.as_str())
                            }
                            _ => None,
                        });
                for (id, label, current) in [
                    (
                        "generator-assign-title",
                        "Thread titles and other generated text",
                        self.text_generation_model.as_deref(),
                    ),
                    (
                        "generator-assign-writer",
                        "Commit messages, pull requests, and branch names",
                        writer,
                    ),
                    (
                        "generator-assign-voice",
                        "Voice transcript cleanup",
                        self.dictation_prefs.cleanup.model.as_deref(),
                    ),
                ] {
                    let defaults = if id == "generator-assign-title" {
                        &[
                            ("default", "Automatic"),
                            ("local", "Local first-message text"),
                        ][..]
                    } else {
                        &[("default", "Use text generation model")][..]
                    };
                    let current = current
                        .filter(|model| {
                            api::split_selection(model).is_none() || self.generator_model(model)
                        })
                        .unwrap_or("default");
                    let mut control = div().flex().items_center().gap_2().child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(self.text_model_control(id, label, current, defaults, cx)),
                    );
                    let effective = if current == "default" {
                        self.text_generation_model.as_deref().unwrap_or("automatic")
                    } else {
                        current
                    };
                    if let Some((uuid, slug)) = api::split_selection(effective) {
                        if let Some(row) = self
                            .generators
                            .rows
                            .iter()
                            .find(|row| row.enabled && row.id == uuid)
                        {
                            let row = row.clone();
                            let slug = slug.to_owned();
                            let test_id = saved_option_value(&format!("{id}-test"));
                            control = control.child(self.gen_button(
                                test_id,
                                "Test",
                                false,
                                cx,
                                move |ws, _, cx| {
                                    ws.test_generator(row.clone(), slug.clone(), Some(id), cx)
                                },
                            ));
                        }
                    }
                    let mut assignment = field(label, control.into_any_element());
                    if let Some((_, _, note)) = self
                        .generators
                        .assignment_test
                        .as_ref()
                        .filter(|(key, model, _)| *key == id && model == effective)
                    {
                        assignment = assignment.child(
                            div()
                                .id(saved_option_value(&format!("{id}-result")))
                                .role(Role::Status)
                                .text_sm()
                                .text_color(theme::rambutan_400())
                                .child(note.clone()),
                        );
                    }
                    body = body.child(assignment);
                }
                if let Some(error) = &self.delivery_preferences.error {
                    body = body.child(
                        div()
                            .id("generator-writer-status")
                            .role(Role::Status)
                            .text_sm()
                            .child(format!("Source control writer: {error}")),
                    );
                }
                let back = g.clone();
                foot = foot
                    .child(self.gen_button(
                        "generator-back",
                        "Back",
                        false,
                        cx,
                        move |ws, w, cx| ws.edit_generator(Some(back.clone()), w, cx),
                    ))
                    .child(self.gen_button("generator-done", "Done", true, cx, close));
                "Choose models".to_owned()
            }
            Page::Models(g, slug) => {
                let choices = g
                    .models
                    .iter()
                    .map(|m| (saved_option_value(&m.id), saved_option_value(&m.name)))
                    .collect::<Vec<_>>();
                if choices.is_empty() {
                    body = body.child("No models available.");
                } else {
                    body = body.child(field(
                        "Model",
                        self.choice_control(
                            "generator-test-model",
                            "Generator model",
                            slug,
                            &choices,
                            cx,
                        ),
                    ));
                }
                let g1 = g.clone();
                foot = foot.child(self.gen_button(
                    "generator-refresh",
                    "Refresh",
                    false,
                    cx,
                    move |ws, _, cx| {
                        let g = g1.clone();
                        ws.generator_job(
                            async move {
                                let g = api::refresh(g).await?;
                                let slug =
                                    g.models.first().map(|m| m.id.clone()).unwrap_or_default();
                                Ok(Some(Page::Models(g, slug)))
                            },
                            cx,
                        )
                    },
                ));
                if !slug.is_empty() && g.enabled {
                    let row = g.clone();
                    let slug = slug.clone();
                    foot = foot.child(self.gen_button(
                        "generator-test",
                        "Test model",
                        false,
                        cx,
                        move |ws, _, cx| {
                            let row = row.clone();
                            let slug = slug.clone();
                            ws.test_generator(row, slug, None, cx)
                        },
                    ));
                }
                foot = foot.child(self.gen_button("generator-done", "Done", true, cx, close));
                format!("{} models", g.label)
            }
            Page::Remove(g) => {
                body=body.child(if g.reference.is_some() {
                    "This removes the link to the 1Password field. Settings that use this generator fall back to their default model."
                } else {
                    "This removes the saved API key. Settings that use this generator fall back to their default model."
                });
                let row = g.clone();
                foot = foot
                    .child(self.gen_button("generator-cancel", "Cancel", false, cx, close))
                    .child(self.gen_button(
                        "generator-remove",
                        "Remove",
                        true,
                        cx,
                        move |ws, _, cx| {
                            let row = row.clone();
                            ws.generator_job(
                                async move { api::remove(row).await.map(|_| None) },
                                cx,
                            )
                        },
                    ));
                format!("Remove {}?", g.label)
            }
        };
        if let Some(note) = &self.generators.notice {
            body = body.child(
                div()
                    .id("generator-dialog-status")
                    .role(Role::Status)
                    .text_sm()
                    .text_color(theme::rambutan_300())
                    .child(note.clone()),
            );
        }
        let mut header = div()
            .flex()
            .flex_col()
            .gap_2()
            .px_5()
            .py_4()
            .border_b_1()
            .border_color(theme::ring_modal());
        let close_button = web_button(
            "generator-close",
            WebButtonVariant::Ghost,
            WebButtonSize::IconSmall,
            ButtonState::disabled_if(self.generators.busy),
        )
        .aria_label("Close dialog")
        .child(icon_svg(
            assets::CLOSE_ICON_PATH,
            14.,
            theme::rambutan_400(),
        ));
        header = header.child(
            div()
                .flex()
                .justify_between()
                .items_center()
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title.clone()),
                )
                .child(if self.generators.busy {
                    close_button
                } else {
                    on_press_view(close_button, cx, close)
                }),
        );
        if matches!(page, Page::Edit(_) | Page::Connected(_)) {
            header=header.child(div().text_sm().text_color(theme::rambutan_400()).child("Use a model API for short text. Enter a key stored on this device or use a configured secret source."));
            let mut steps = div()
                .flex()
                .gap_1()
                .rounded_md()
                .p_1()
                .bg(theme::longan_950());
            for (id, label, first) in [
                ("generator-step-connect", "1  Connect", true),
                ("generator-step-use", "2  Use it", false),
            ] {
                let current = matches!(page, Page::Edit(_)) == first;
                let step = div()
                    .id(id)
                    .flex_1()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_sm()
                    .when(current, |el| el.bg(theme::longan_800()))
                    .text_color(if current {
                        theme::rambutan_100()
                    } else {
                        theme::rambutan_400()
                    })
                    .child(label);
                steps = steps.child(
                    if let Some(g) = match page {
                        Page::Connected(g) if first && !self.generators.busy => Some(g),
                        _ => None,
                    } {
                        let g = g.clone();
                        on_press_view(
                            step.role(Role::Button).tab_index(0).cursor_pointer(),
                            cx,
                            move |ws, w, cx| ws.edit_generator(Some(g.clone()), w, cx),
                        )
                    } else {
                        step
                    },
                );
            }
            header = header.child(steps);
        }
        Some(
            div()
                .id("generator-overlay")
                .absolute()
                .inset_0()
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::longan_950().opacity(0.72))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .id("generator-dialog")
                        .debug_selector(|| "generator-dialog".into())
                        .role(Role::Dialog)
                        .aria_label(title)
                        .track_focus(&self.generators.start)
                        .tab_group()
                        .on_key_down(cx.listener(Self::trap_modal_tab_focus))
                        .w(px(576.).min(window.viewport_size().width - px(28.)))
                        .max_h(window.viewport_size().height - px(40.))
                        .overflow_y_scroll()
                        .rounded_xl()
                        .border_1()
                        .border_color(theme::longan_700())
                        .bg(theme::longan_900())
                        .flex()
                        .flex_col()
                        .child(header)
                        .child(body.px_5().py_4())
                        .child(
                            foot.px_5()
                                .py_3()
                                .border_t_1()
                                .border_color(theme::ring_modal()),
                        )
                        .child(div().size(px(0.)).track_focus(&self.generators.end)),
                )
                .into_any_element(),
        )
    }
}
