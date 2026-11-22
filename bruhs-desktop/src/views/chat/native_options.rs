use super::*;
use bruhs_models::delivery::{
    generation::OptionKind,
    writer_catalog::{WriterCliModel, WriterOption},
};
use bruhs_ui::dropdown::{Dropdown, DropdownGroup, DropdownOption, Listbox, saved_option_value};
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
                    chat.model_catalog = std::sync::Arc::new(catalog);
                    cx.notify();
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
        self.native_model()
            .is_some_and(|model| !model.options.is_empty())
            && self
                .picker_availability
                .as_ref()
                .is_ok_and(|p| p.enabled(AgentProvider::for_model(&self.model_id)))
    }

    pub(super) fn focus_model_choices(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        cx.defer_in(window, |chat, window, cx| {
            if chat.model_picker_open && chat.has_picker_options() {
                if let Some(spec) = chat
                    .native_model()
                    .and_then(|model| model.options.iter().find(|spec| !footer_option(spec)))
                {
                    let selected = chat.option_value(spec);
                    if let Some(index) = spec.choices.iter().position(|c| c.value == selected) {
                        // The first child is the heading. Reveal the selected row after layout.
                        chat.model_option_scroll.scroll_to_item(index + 1);
                    }
                }
                chat.composer_effort_focus.focus(window, cx);
                cx.notify();
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
        .compact()
        .end_aligned_popover(px(170.))
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
                value: m.as_str(),
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

    pub(super) fn picker_model_options(
        &self,
        footer: bool,
        stacked: bool,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        let model = self.native_model()?;
        let primary = model
            .options
            .iter()
            .find(|s| !footer_option(s))
            .or_else(|| model.options.first())?
            .kind;
        let specs: Vec<_> = model
            .options
            .iter()
            .filter(|s| footer_option(s) == footer)
            .collect();
        if specs.is_empty() {
            return None;
        }
        let scroll = self.model_option_scroll.clone();
        let mut rows = Vec::new();
        for spec in specs {
            let kind = spec.kind;
            let selected = self.option_value(spec);
            let mut chips = div().flex().flex_wrap().items_center().gap_1();
            if !footer {
                rows.push(
                    div()
                        .px_2()
                        .py_1()
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_between()
                        .text_size(px(12.))
                        .text_color(theme::rambutan_400())
                        .child(spec.label())
                        .when(primary == kind, |el| {
                            el.child(div().min_w_0().ml_1().truncate().child(model.label.clone()))
                        })
                        .into_any_element(),
                );
            }
            if !footer && kind != primary {
                let choices = spec
                    .choices
                    .iter()
                    .map(|choice| DropdownOption {
                        value: saved_option_value(&choice.value),
                        label: choice.label.clone(),
                        description: choice.description.clone(),
                    })
                    .collect();
                rows.push(
                    self.picker_option_dropdown(
                        kind.key(),
                        spec.label(),
                        &selected,
                        choices,
                        move |chat, value, cx| chat.choose_picker_option(kind, value, cx),
                        cx,
                    )
                    .into_any_element(),
                );
                continue;
            }
            for (i, choice) in spec.choices.iter().enumerate() {
                let value = choice.value.clone();
                let checked = value == selected;
                let selector = if kind == OptionKind::Effort {
                    format!("desktop-effort-option-{value}")
                } else {
                    format!("desktop-model-{}-{value}", kind.key())
                };
                let index = rows.len();
                let first = i == 0;
                let last = i + 1 == spec.choices.len();
                let scroll = scroll.clone();
                let label = choice.label.clone();
                let button = div()
                    .id(SharedString::from(selector.clone()))
                    .debug_selector(move || selector)
                    .flex_shrink_0()
                    .min_h(px(26.))
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
                        el.bg(theme::longan_800()).text_color(theme::rambutan_100())
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
                    .when(primary == kind && checked, |el| {
                        el.track_focus(&self.composer_effort_focus)
                    })
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .child(label)
                    .when(!footer && checked, |el| {
                        el.child(
                            svg()
                                .path(assets::CHECK_ICON_PATH)
                                .size(px(12.))
                                .text_color(theme::persimmon_400()),
                        )
                    })
                    .on_key_down(cx.listener(move |chat, event: &KeyDownEvent, window, cx| {
                        if event.keystroke.modifiers.modified() {
                            return;
                        }
                        match event.keystroke.key.as_str() {
                            "left" => chat
                                .model_picker_query
                                .update(cx, |input, cx| input.focus(window, cx)),
                            "down" if !footer => {
                                if !last {
                                    window.focus_next(cx);
                                    scroll.scroll_to_item(index + 1);
                                }
                            }
                            "up" if !footer => {
                                if !first {
                                    window.focus_prev(cx);
                                    scroll.scroll_to_item(index - 1);
                                }
                            }
                            _ => return,
                        }
                        window.prevent_default();
                        cx.stop_propagation();
                        cx.notify();
                    }));
                let control = on_press_view(button, cx, move |chat, _, cx| {
                    chat.choose_picker_option(kind, &value, cx);
                });
                if footer {
                    chips = chips.child(control);
                } else {
                    rows.push(control.into_any_element());
                }
            }
            if footer {
                rows.push(
                    chips
                        .id(SharedString::from(format!("picker-{}", kind.key())))
                        .role(Role::RadioGroup)
                        .aria_label(spec.label())
                        .into_any_element(),
                );
            }
        }
        if footer {
            return Some(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .children(rows)
                    .into_any_element(),
            );
        }
        Some(
            div()
                .id("model-options-column")
                .debug_selector(|| "model-options-column".into())
                .min_h_0()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_px()
                .p_1()
                .when(stacked, |el| el.w_full().flex_1().border_t_1())
                .when(!stacked, |el| el.w(px(190.)).flex_shrink_0().border_l_1())
                .border_color(theme::ring_floating())
                .bg(theme::longan_900())
                .overflow_y_scroll()
                .track_scroll(&scroll)
                .children(rows)
                .vertical_scrollbar(&scroll)
                .into_any_element(),
        )
    }
}

fn footer_option(spec: &WriterOption) -> bool {
    matches!(
        spec.kind,
        OptionKind::ServiceTier | OptionKind::FastMode | OptionKind::Ultracode
    )
}
