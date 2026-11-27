use super::*;
use bruhs_models::delivery::{
    generation::OptionKind,
    writer_catalog::{WriterCliModel, WriterOption},
};
use bruhs_ui::dropdown::{
    Dropdown, DropdownGroup, DropdownOption, DropdownTriggerVariant, Listbox,
};
use bruhs_ui::press::on_press_view;
use std::path::PathBuf;

impl ChatView {
    pub(super) fn native_model(&self) -> Option<&WriterCliModel> {
        self.model_catalog.model(&self.model_id)
    }
    pub(super) fn effort_spec(&self) -> Option<&WriterOption> {
        self.native_model()?
            .options
            .iter()
            .find(|spec| spec.kind == OptionKind::Effort)
    }
    pub(super) fn refresh_model_catalog(&mut self, cx: &mut Context<'_, Self>) {
        if cfg!(test) {
            return;
        }
        let project = self.project_path.clone();
        let path = PathBuf::from(project.as_str());
        let listing = crate::app_state::spawn_tokio(cx, async move {
            bruhs_core::agent::writer_catalog_for_project(&path).await
        });
        self.model_catalog_load = Some(cx.spawn(async move |chat, cx| {
            let catalog = listing.await;
            let _ = chat.update(cx, |chat, cx| {
                if chat.project_path == project {
                    chat.replace_model_catalog(std::sync::Arc::new(catalog), cx);
                }
            });
        }));
    }

    pub(super) fn render_native_options(&self, cx: &Context<'_, Self>) -> AnyElement {
        let menu = div().id("desktop-effort-menu").flex().flex_col().gap_2();
        let Some(model) = self.native_model() else {
            return menu.into_any_element();
        };
        let summary = model
            .options
            .iter()
            .filter_map(|spec| {
                let value = self.option_value(spec);
                let choice = spec.choices.iter().find(|c| c.value == value)?;
                if spec.kind == OptionKind::Thinking {
                    Some(format!("Thinking {}", choice.label))
                } else if value == "off" || spec.kind == OptionKind::CyberAccessProgram {
                    None
                } else if value == "on" {
                    Some(spec.label().to_owned())
                } else {
                    Some(choice.label.clone())
                }
            })
            .collect::<Vec<_>>()
            .join(" · ");
        let button = bruhs_ui::press::focusable_control(
            div()
                .id("composer-model-options")
                .debug_selector(|| "composer-model-options".into())
                .min_h(px(theme::CONTROL_PX))
                .px_2()
                .rounded(px(5.))
                .bg(theme::longan_800())
                .flex()
                .items_center()
                .justify_between()
                .child(if summary.is_empty() {
                    "Model options".into()
                } else {
                    summary
                })
                .child(
                    svg()
                        .path(assets::CHEVRON_DOWN_ICON_PATH)
                        .size(px(12.))
                        .text_color(theme::rambutan_400()),
                ),
            Role::Button,
            "Model options",
            bruhs_ui::press::Interactivity::Enabled,
        )
        .aria_expanded(false)
        .track_focus(&self.composer_effort_focus);
        menu.child(on_press_view(button, cx, |chat, window, cx| {
            chat.toggle_effort_menu(window, cx)
        }))
        .into_any_element()
    }

    fn option_value(&self, spec: &WriterOption) -> String {
        let configured = if spec.kind == OptionKind::Effort {
            self.claude_send_effort()
        } else {
            self.model_options.configured(spec.kind).map(str::to_owned)
        };
        configured
            .filter(|v| spec.accepts(v))
            .unwrap_or_else(|| spec.default.clone())
    }

    pub(super) fn has_picker_options(&self) -> bool {
        self.has_picker_options_for(&self.model_id)
    }

    pub(super) fn has_picker_options_for(&self, id: &str) -> bool {
        self.model_catalog
            .model(id)
            .is_some_and(|model| !model.options.is_empty())
            && self
                .picker_availability
                .as_ref()
                .is_ok_and(|p| p.enabled(AgentProvider::for_model(id)))
    }

    pub(super) fn focus_model_choices(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        cx.defer_in(window, |chat, window, cx| {
            if chat.model_picker_open && chat.has_picker_options() {
                if let Some(spec) = chat.native_model().and_then(primary_column_option) {
                    let selected = chat.option_value(spec);
                    if let Some(index) = spec.choices.iter().position(|c| c.value == selected) {
                        if spec.kind == OptionKind::Effort {
                            chat.model_option_scroll.scroll_to_item(index);
                        }
                    }
                    chat.composer_effort_focus.focus(window, cx);
                    cx.notify();
                }
            }
        });
    }

    fn picker_option_dropdown(
        &self,
        id: &'static str,
        label: &str,
        value: &str,
        choices: Vec<DropdownOption>,
        select: impl Fn(&mut Self, &str, &mut Context<'_, Self>) + 'static,
        cx: &Context<'_, Self>,
    ) -> Dropdown {
        let toggle = cx.weak_entity();
        let view = toggle.clone();
        Dropdown::new(
            id,
            label,
            value,
            vec![DropdownGroup {
                label: None,
                mark: None,
                options: choices,
            }],
            if self.model_picker_dropdown == Some(id) {
                Listbox::Popover
            } else {
                Listbox::Closed
            },
            std::rc::Rc::new(move |_, cx| {
                let _ = toggle.update(cx, |chat, cx| {
                    chat.model_picker_dropdown = if chat.model_picker_dropdown == Some(id) {
                        None
                    } else {
                        Some(id)
                    };
                    cx.notify();
                });
            }),
            std::rc::Rc::new(move |value, _, cx| {
                let _ = view.update(cx, |chat, cx| {
                    chat.model_picker_dropdown = None;
                    select(chat, value, cx);
                    cx.notify();
                });
            }),
        )
    }

    pub(super) fn picker_permission_control(&self, cx: &Context<'_, Self>) -> AnyElement {
        let modes = [
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::BypassPermissions,
        ];
        let choices = modes
            .iter()
            .map(|m| DropdownOption {
                value: m.as_str().into(),
                label: m.label().into(),
                description: None,
            })
            .collect();
        self.picker_option_dropdown(
            "model-picker-permissions",
            "Permissions",
            self.composer_access_mode().as_str(),
            choices,
            |chat, value, cx| {
                if let Ok(mode) = value.parse() {
                    chat.choose_permission_mode(mode, cx);
                }
            },
            cx,
        )
        .compact()
        .end_aligned_popover(px(170.))
        .trigger_variant(DropdownTriggerVariant::Toolbar)
        .leading_icon("icons/shield.svg")
        .into_any_element()
    }

    fn choose_picker_option(&mut self, kind: OptionKind, value: &str, cx: &mut Context<'_, Self>) {
        let Some(spec) = self
            .native_model()
            .and_then(|m| m.options.iter().find(|s| s.kind == kind))
            .cloned()
        else {
            return;
        };
        if self.model_options.set_choice(&spec, value) {
            cx.emit(ChatViewEvent::ModelOptionsChanged);
            cx.notify();
        }
    }

    pub(super) fn picker_model_footer(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        let model = self.native_model()?;
        let mut rows = Vec::new();
        for spec in model.options.iter().filter(|spec| footer_option(spec)) {
            let kind = spec.kind;
            let selected = self.option_value(spec);
            if kind == OptionKind::Ultracode
                && spec.accepts("on")
                && spec.accepts("off")
                && spec.choices.len() == 2
            {
                let enabled = selected == "on";
                let label = spec.label();
                let control = bruhs_ui::press::focusable_control(
                    div()
                        .id(SharedString::from(format!("picker-{}", kind.key())))
                        .h(px(28.))
                        .px_2()
                        .rounded(px(4.))
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_size(px(12.))
                        .text_color(theme::rambutan_300())
                        .when(enabled, |el| {
                            el.bg(theme::longan_800())
                                .text_color(theme::rambutan_100())
                                .child(svg().path(assets::CHECK_ICON_PATH).size(px(12.)))
                        })
                        .child(label)
                        .hover(|el| el.bg(theme::longan_800())),
                    Role::Switch,
                    label,
                    bruhs_ui::press::Interactivity::Enabled,
                )
                .aria_toggled(if enabled {
                    Toggled::True
                } else {
                    Toggled::False
                });
                rows.push(
                    on_press_view(control, cx, move |chat, _, cx| {
                        chat.choose_picker_option(kind, if enabled { "off" } else { "on" }, cx);
                    })
                    .into_any_element(),
                );
                continue;
            }
            let mut chips = div().flex().flex_wrap().items_center().gap(px(2.)).when(
                matches!(kind, OptionKind::ServiceTier | OptionKind::FastMode),
                |el| el.p(px(2.)).rounded(px(6.)).bg(theme::longan_900()),
            );
            let mut choices: Vec<_> = spec.choices.iter().collect();
            if kind == OptionKind::FastMode {
                choices.sort_by_key(|choice| choice.value != "off");
            }
            for choice in choices {
                let value = choice.value.clone();
                let checked = value == selected;
                let selector = format!("desktop-model-{}-{value}", kind.key());
                let label = match (kind, value.as_str()) {
                    (OptionKind::FastMode, "off") => "Standard".to_owned(),
                    (OptionKind::FastMode, "on") => "Fast".to_owned(),
                    _ => choice.label.clone(),
                };
                let fast = matches!(kind, OptionKind::FastMode | OptionKind::ServiceTier)
                    && matches!(label.as_str(), "Fast" | "Ultrafast");
                let button = div()
                    .id(SharedString::from(selector.clone()))
                    .debug_selector(move || selector)
                    .flex_shrink_0()
                    .min_h(px(24.))
                    .px_2()
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(px(12.))
                    .text_color(theme::rambutan_300())
                    .border_1()
                    .border_color(transparent_black())
                    .when(checked, |el| {
                        el.text_color(theme::rambutan_100()).bg(theme::longan_800())
                    })
                    .hover(|el| el.bg(theme::longan_800()))
                    .tab_index(0)
                    .role(Role::RadioButton)
                    .aria_toggled(if checked {
                        Toggled::True
                    } else {
                        Toggled::False
                    })
                    .aria_label(format!("{}: {label}", spec.label()))
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .when(fast, |row| {
                                row.child(
                                    svg()
                                        .path(bruhs_ui::icons::ZAP)
                                        .size(px(12.))
                                        .flex_none()
                                        .text_color(if checked {
                                            theme::persimmon_400()
                                        } else {
                                            theme::rambutan_400()
                                        }),
                                )
                            })
                            .child(label),
                    );
                let control = on_press_view(button, cx, move |chat, _, cx| {
                    chat.choose_picker_option(kind, &value, cx);
                });
                chips = chips.child(control);
            }
            rows.push(
                chips
                    .id(SharedString::from(format!("picker-{}", kind.key())))
                    .role(Role::RadioGroup)
                    .aria_label(spec.label())
                    .into_any_element(),
            );
        }
        (!rows.is_empty()).then(|| {
            div()
                .flex()
                .flex_wrap()
                .gap_1()
                .children(rows)
                .into_any_element()
        })
    }

    pub(super) fn picker_option_column(
        &self,
        stacked: bool,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        let model = self.native_model()?;
        let primary = primary_column_option(model)?;
        let short_label = match model.label.strip_prefix("GPT-") {
            Some(name) => name.replace('-', " "),
            None => model
                .label
                .strip_prefix("Claude ")
                .unwrap_or(&model.label)
                .to_owned(),
        };
        let heading = div()
            .debug_selector(|| "desktop-model-picker-options-heading".into())
            .h(px(crate::views::model_browser::HEADER_HEIGHT_PX))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_between()
            .gap_1()
            .px_2()
            .border_b_1()
            .border_color(theme::ring_card())
            .text_size(px(theme::APP_UI_TEXT_SIZE_PX))
            .text_color(theme::rambutan_400())
            .child(if primary.kind == OptionKind::Effort {
                primary.label()
            } else {
                "Options"
            })
            .child(div().min_w_0().truncate().child(short_label));
        let mut rows = Vec::new();
        let mut navigation = Vec::new();
        let specs = std::iter::once(primary).chain(
            model
                .options
                .iter()
                .filter(|spec| !footer_option(spec) && spec.kind != primary.kind),
        );
        for spec in specs {
            let kind = spec.kind;
            let selected = self.option_value(spec);
            if kind == OptionKind::Effort {
                for (index, choice) in spec.choices.iter().enumerate() {
                    let checked = choice.value == selected;
                    let focus = self.picker_column_focus(kind, index, checked, cx);
                    let value = choice.value.clone();
                    let selector = format!("desktop-effort-option-{value}");
                    let label = choice.label.clone();
                    navigation.push((label.to_lowercase(), focus.clone()));
                    let button = div()
                        .id(SharedString::from(selector.clone()))
                        .debug_selector(move || selector)
                        .flex_shrink_0()
                        .h(px(34.))
                        .px_2()
                        .rounded(px(4.))
                        .flex()
                        .items_center()
                        .justify_between()
                        .text_size(px(12.))
                        .text_color(if checked {
                            theme::rambutan_100()
                        } else {
                            theme::rambutan_300()
                        })
                        .border_1()
                        .border_color(transparent_black())
                        .hover(|el| el.bg(theme::longan_800()))
                        .focus_visible(|el| el.border_color(theme::persimmon_400()))
                        .tab_index(0)
                        .tab_stop(checked)
                        .track_focus(&focus)
                        .role(Role::RadioButton)
                        .aria_toggled(if checked {
                            Toggled::True
                        } else {
                            Toggled::False
                        })
                        .aria_label(format!("{}: {label}", spec.label()))
                        .cursor_pointer()
                        .child(div().flex().items_center().gap_2().child(label).when(
                            value != "default" && value == spec.default && !checked,
                            |el| el.child(bruhs_ui::kbd::kbd("Default")),
                        ))
                        .when(checked, |el| {
                            el.child(
                                svg()
                                    .path(assets::CHECK_ICON_PATH)
                                    .size(px(12.))
                                    .text_color(theme::persimmon_400()),
                            )
                        });
                    rows.push(
                        on_press_view(button, cx, move |chat, window, cx| {
                            chat.choose_picker_option(kind, &value, cx);
                            chat.focus_model_choices(window, cx);
                        })
                        .into_any_element(),
                    );
                }
                continue;
            }

            let entry = kind == primary.kind;
            let focus = self.picker_column_focus(kind, 0, entry, cx);
            navigation.push((spec.label().to_lowercase(), focus.clone()));
            if spec.choices.len() == 2 && spec.accepts("on") && spec.accepts("off") {
                let enabled = selected == "on";
                let switch = bruhs_ui::switch::switch(
                    kind.key(),
                    spec.label(),
                    enabled,
                    bruhs_ui::press::Interactivity::Enabled,
                )
                .track_focus(&focus)
                .tab_stop(entry);
                rows.push(
                    div()
                        .debug_selector(move || format!("{}-switch-row", kind.key()))
                        .min_h(px(theme::CONTROL_DETAIL_PX))
                        .flex_shrink_0()
                        .px(px(theme::FIELD_PADDING_X_PX))
                        .border_1()
                        .border_color(transparent_black())
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_1()
                        .font_family(theme::FontFace::Interface)
                        .text_size(px(12.))
                        .text_color(theme::rambutan_300())
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .debug_selector(move || format!("{}-caption", kind.key()))
                                        .truncate()
                                        .text_color(theme::rambutan_400())
                                        .child(spec.label()),
                                )
                                .child(if enabled { "On" } else { "Off" }),
                        )
                        .child(on_press_view(switch, cx, move |chat, _, cx| {
                            chat.choose_picker_option(kind, if enabled { "off" } else { "on" }, cx);
                        }))
                        .into_any_element(),
                );
            } else {
                let choices = spec
                    .choices
                    .iter()
                    .map(|choice| DropdownOption {
                        value: choice.value.clone().into(),
                        label: choice.label.clone(),
                        description: choice.description.clone(),
                    })
                    .collect();
                let current_label = spec
                    .choices
                    .iter()
                    .find(|choice| choice.value == selected)
                    .map_or_else(|| selected.clone(), |choice| choice.label.clone());
                let dropdown = self
                    .picker_option_dropdown(
                        kind.key(),
                        spec.label(),
                        &selected,
                        choices,
                        move |chat, value, cx| chat.choose_picker_option(kind, value, cx),
                        cx,
                    )
                    .roving_focus(focus, entry)
                    .group_selection(current_label, vec![], vec![spec.default.clone().into()]);
                if kind == OptionKind::ContextWindow {
                    rows.push(
                        div()
                            .flex_shrink_0()
                            .child(dropdown.trigger_variant(DropdownTriggerVariant::Submenu))
                            .into_any_element(),
                    );
                    continue;
                }
                rows.push(
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .flex_shrink_0()
                        .pt_2()
                        .gap_1()
                        .child(
                            div()
                                .debug_selector(move || format!("{}-caption", kind.key()))
                                .px(px(bruhs_ui::theme::FIELD_PADDING_X_PX + 1.0))
                                .text_size(px(12.))
                                .text_color(theme::rambutan_400())
                                .child(spec.label()),
                        )
                        .child(dropdown)
                        .into_any_element(),
                );
            }
        }
        let scroll = self.model_option_scroll.clone();
        let nav_scroll = scroll.clone();
        Some(
            self.model_options_column(stacked, cx)
                .child(heading)
                .child(
                    div()
                        .id("model-options-list")
                        .min_h_0()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap_px()
                        .pt_1()
                        .overflow_y_scroll()
                        .track_scroll(&scroll)
                        .on_key_down(cx.listener(move |chat, event: &KeyDownEvent, window, cx| {
                            if chat.model_picker_dropdown.is_some()
                                || event.keystroke.modifiers.modified()
                            {
                                return;
                            }
                            let Some(current) = navigation
                                .iter()
                                .position(|(_, focus)| focus.is_focused(window))
                            else {
                                return;
                            };
                            let next = match event.keystroke.key.as_str() {
                                "up" => Some(current.saturating_sub(1)),
                                "down" => Some((current + 1).min(navigation.len() - 1)),
                                key if key.chars().count() == 1 && !key.trim().is_empty() => {
                                    let key = key.to_lowercase();
                                    (1..=navigation.len())
                                        .map(|offset| (current + offset) % navigation.len())
                                        .find(|&index| navigation[index].0.starts_with(&key))
                                }
                                _ => None,
                            };
                            if let Some(next) = next {
                                navigation[next].1.focus(window, cx);
                                nav_scroll.scroll_to_item(next);
                                window.prevent_default();
                                cx.stop_propagation();
                            }
                        }))
                        .children(rows),
                )
                .into_any_element(),
        )
    }

    fn picker_column_focus(
        &self,
        kind: OptionKind,
        index: usize,
        entry: bool,
        cx: &App,
    ) -> FocusHandle {
        if entry {
            return self.composer_effort_focus.clone();
        }
        // a11y-exempt: This sets up a roving focus handle. The option renders the focus ring.
        self.model_option_focus
            .borrow_mut()
            .entry((kind, index))
            .or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(false))
            .clone()
    }

    pub(super) fn model_options_column(
        &self,
        stacked: bool,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        div()
            .id("model-options-column")
            .debug_selector(|| "model-options-column".into())
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_px()
            .px_2()
            .pb_2()
            .when(stacked, |el| el.w_full().flex_1().border_t_1())
            .when(!stacked, |el| {
                el.w(px(super::model_picker_ui::MODEL_PICKER_OPTIONS_WIDTH_PX))
                    .flex_shrink_0()
                    .border_l_1()
            })
            .border_color(theme::ring_floating())
            .bg(theme::longan_950())
            .on_key_down(cx.listener(|chat, event: &KeyDownEvent, window, cx| {
                if chat.model_picker_dropdown.is_none()
                    && !event.keystroke.modifiers.modified()
                    && event.keystroke.key == "left"
                {
                    chat.model_picker_query
                        .update(cx, |input, cx| input.focus(window, cx));
                    window.prevent_default();
                    cx.stop_propagation();
                }
            }))
    }
}

fn primary_column_option(model: &WriterCliModel) -> Option<&WriterOption> {
    model
        .options
        .iter()
        .find(|spec| spec.kind == OptionKind::Effort)
        .or_else(|| model.options.iter().find(|spec| !footer_option(spec)))
}

fn footer_option(spec: &WriterOption) -> bool {
    matches!(
        spec.kind,
        OptionKind::ServiceTier | OptionKind::FastMode | OptionKind::Ultracode
    )
}
