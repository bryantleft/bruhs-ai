//! Draft values belong to the open editor. Opening a saved device variable
//! displays its configured state without reading its value from the keychain.
use super::*;
use bruhs_models::agent_extensions::{valid_environment_name, AcpEnvironment, AcpVariable};
use bruhs_ui::dropdown::saved_option_value;
use uuid::Uuid;

#[derive(Clone, PartialEq, Eq)]
enum Source {
    Device,
    Plain,
    Vault(String),
}

pub(super) struct VariableRow {
    pub(super) name: Entity<InputState>,
    pub(super) value: Entity<InputState>,
    source: Source,
    saved: Option<Uuid>,
    revision: Uuid,
}

impl VariableRow {
    pub(super) fn new(
        name: &str,
        variable: Option<&AcpVariable>,
        w: &mut Window,
        cx: &mut App,
    ) -> Self {
        let (source, value, saved) = match variable {
            Some(AcpVariable::Plain { value }) => (Source::Plain, value.as_str(), None),
            Some(AcpVariable::OnePassword { source, reference }) => {
                (Source::Vault(source.clone()), reference.as_str(), None)
            }
            Some(AcpVariable::Device { revision }) => (Source::Device, "", Some(*revision)),
            None => (Source::Device, "", None),
        };
        let hint = if saved.is_some() {
            "Saved value · type to replace"
        } else if matches!(source, Source::Vault(_)) {
            "op://vault/item/field"
        } else {
            "Value"
        };
        Self {
            name: cx.new(|cx| {
                InputState::new(w, cx)
                    .placeholder("VARIABLE_NAME")
                    .default_value(name.to_owned())
            }),
            value: cx.new(|cx| {
                InputState::new(w, cx)
                    .placeholder(hint)
                    .masked(source == Source::Device)
                    .default_value(value.to_owned())
            }),
            source,
            saved,
            revision: Uuid::new_v4(),
        }
    }

    fn redacted(&self, cx: &App) -> bool {
        self.saved.is_some() && self.value.read(cx).value().is_empty()
    }
}

impl ProviderEditor {
    pub(super) fn environment(&self, cx: &App) -> Result<AcpEnvironment, String> {
        let mut result = AcpEnvironment::new();
        for row in &self.variables {
            let name = row.name.read(cx).value();
            let name = name.trim();
            let value = row.value.read(cx).value();
            if name.is_empty()
                && value.is_empty()
                && row.source == Source::Device
                && row.saved.is_none()
            {
                continue;
            }
            if !valid_environment_name(name) {
                return Err("Enter a variable name beginning with a letter or underscore.".into());
            }
            if value.contains('\0') {
                return Err(format!("The value for {name} contains a NUL character."));
            }
            let var = match &row.source {
                Source::Plain => AcpVariable::Plain {
                    value: value.to_string(),
                },
                Source::Device => AcpVariable::Device {
                    revision: row
                        .saved
                        .filter(|_| row.redacted(cx))
                        .unwrap_or(row.revision),
                },
                Source::Vault(source) => {
                    bruhs_core::secret_sources::validate_reference(value.trim())
                        .map_err(|e| e.to_string())?;
                    AcpVariable::OnePassword {
                        source: source.clone(),
                        reference: value.trim().to_owned(),
                    }
                }
            };
            if result.keys().any(|key: &String| {
                bruhs_models::agent_extensions::same_environment_name(key, name)
            }) {
                return Err(format!("Variable {name} appears twice."));
            }
            result.insert(name.to_owned(), var);
        }
        Ok(result)
    }

    pub(super) fn device_values(
        &self,
        cx: &App,
    ) -> bruhs_core::agent_extensions::environment::DeviceValues {
        self.variables
            .iter()
            .filter(|r| {
                r.source == Source::Device
                    && !r.redacted(cx)
                    && !r.name.read(cx).value().trim().is_empty()
            })
            .map(|r| (r.revision, r.value.read(cx).value().to_string().into()))
            .collect()
    }
}

impl Workspace {
    pub(super) fn provider_variables(&self, wide: bool, cx: &Context<'_, Self>) -> AnyElement {
        let Some(editor) = self.provider_editor.as_ref() else {
            return div().into_any_element();
        };
        let mut section = div().flex().flex_col().gap_2().child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .child(div().text_sm().child("Variables"))
                        .child(hint(
                            "API keys, base URLs, and other settings for this provider.",
                        )),
                )
                .child(editor_button(
                    "acp-add-variable",
                    "Add variable",
                    WebButtonVariant::Outline,
                    editor.busy,
                    cx,
                    |ws, w, cx| {
                        if let Some(e) = &mut ws.provider_editor {
                            let row = VariableRow::new("", None, w, cx);
                            e.track_edits(&row.name, cx);
                            e.track_edits(&row.value, cx);
                            row.name.update(cx, |f, cx| f.focus(w, cx));
                            e.variables.push(row);
                        }
                        cx.notify();
                    },
                )),
        );
        let sources = self.credential_settings.read(cx).secret_sources();
        let instance_id = editor.id.read(cx).value();
        let target = format!("acp/{}", instance_id.trim());
        for binding in sources
            .read(cx)
            .bindings()
            .iter()
            .filter(|binding| binding.target == target)
        {
            section = section.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().text_sm().child(binding.variable.clone()))
                    .child(hint("Managed in Secrets")),
            );
        }
        if editor.kiro.active && !editor.variables.iter().any(|r| r.name.read(cx).value().as_str() == "KIRO_API_KEY")
            && !sources.read(cx).bindings().iter().any(|b| b.target == target && b.variable == "KIRO_API_KEY") {
            section = section.child(editor_button("kiro-add-api-key", "Add Kiro API key",
                WebButtonVariant::Outline, editor.busy, cx, |ws, w, cx| {
                    if let Some(e) = &mut ws.provider_editor {
                        let row = VariableRow::new("KIRO_API_KEY", None, w, cx);
                        e.track_edits(&row.name, cx);
                        e.track_edits(&row.value, cx);
                        row.value.update(cx, |f, cx| f.focus(w, cx));
                        e.variables.push(row);
                    }
                    cx.notify();
                }));
        }
        for (i, row) in editor.variables.iter().enumerate() {
            let target = row.name.clone();
            let key = target.entity_id();
            let name = row.name.read(cx).value();
            let description = if name.trim().is_empty() {
                (i + 1).to_string()
            } else {
                name.trim().to_owned()
            };
            let remove = web_button(
                SharedString::from(format!("acp-remove-variable-{i}")),
                WebButtonVariant::Ghost,
                WebButtonSize::Small,
                ButtonState::disabled_if(editor.busy),
            )
            .aria_label(format!("Remove environment variable {description}"))
            .child(
                svg()
                    .path(assets::CLOSE_ICON_PATH)
                    .size(px(14.))
                    .text_color(theme::rambutan_400()),
            );
            let remove = remove.when(!editor.busy, |el| {
                on_press_view(el, cx, move |ws, w, cx| {
                    if let Some(e) = &mut ws.provider_editor {
                        if let Some(index) = e.variables.iter().position(|r| r.name == target) {
                            let removed = e.variables.remove(index);
                            e.input_events.remove(&key);
                            e.input_events.remove(&removed.value.entity_id());
                            e.error = None;
                            if e.variable_source_menu == Some(key) {
                                e.variable_source_menu = None;
                            }
                            if let Some(r) = e.variables.get(index).or_else(|| e.variables.last()) {
                                r.name.update(cx, |f, cx| f.focus(w, cx));
                            } else {
                                e.command.update(cx, |f, cx| f.focus(w, cx));
                            }
                        }
                    }
                    cx.notify();
                })
            });
            let redacted = row.redacted(cx);
            let mut value = div()
                .flex()
                .gap_2()
                .when(wide, |d| d.items_center())
                .when(!wide, |d| d.flex_col())
                .child(
                    div().min_w_0().flex_1().child(
                        Input::new(&row.value)
                            .aria_label(format!("Environment variable value {}", i + 1))
                            .disabled(editor.busy),
                    ),
                );
            if !matches!(row.source, Source::Vault(_)) {
                let sensitive = row.source == Source::Device;
                let toggle = bruhs_ui::switch::switch(
                    format!("acp-env-sensitive-{i}"),
                    format!("Mark environment variable {} as sensitive", i + 1),
                    sensitive,
                    bruhs_ui::press::Interactivity::disabled_if(editor.busy || redacted),
                );
                value = value.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(hint("Sensitive"))
                        .child(toggle.when(!editor.busy && !redacted, |el| {
                            on_press_view(el, cx, move |ws, w, cx| {
                                if let Some(r) = ws.provider_editor.as_mut().and_then(|e| {
                                    e.variables.iter_mut().find(|r| r.name.entity_id() == key)
                                }) {
                                    r.source = if sensitive {
                                        Source::Plain
                                    } else {
                                        Source::Device
                                    };
                                    r.saved = None;
                                    r.value.update(cx, |v, cx| v.set_masked(!sensitive, w, cx));
                                }
                                cx.notify();
                            })
                        })),
                );
            }
            let mut header = div().flex().items_center().gap_2().child(
                div().min_w_0().flex_1().child(
                    Input::new(&row.name)
                        .aria_label(format!("Environment variable name {}", i + 1))
                        .disabled(editor.busy || redacted),
                ),
            );
            if row.source != Source::Plain {
                header = header.children(self.variable_source(i, row, cx));
            }
            section = section.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .border_1()
                    .border_color(theme::ring_modal())
                    .rounded_lg()
                    .p_2()
                    .child(header.child(remove))
                    .child(value),
            );
        }
        section
            .when(!editor.variables.is_empty(), |el| {
                el.child(hint(
                    "Sensitive device values are stored in the system credential store.",
                ))
            })
            .into_any_element()
    }

    fn variable_source(
        &self,
        index: usize,
        row: &VariableRow,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        let source_settings = self.credential_settings.read(cx).secret_sources();
        let sources = source_settings.read(cx).sources();
        let selected = match &row.source {
            Source::Vault(id) => saved_option_value(&format!("source:{id}")),
            _ => "value",
        };
        if sources.is_empty() && selected == "value" {
            return None;
        }
        let mut options = vec![DropdownOption {
            value: "value",
            label: "Value".into(),
            description: None,
        }];
        for (id, source) in sources {
            options.push(DropdownOption {
                value: saved_option_value(&format!("source:{id}")),
                label: source.name.clone(),
                description: None,
            });
        }
        if !options.iter().any(|option| option.value == selected) {
            options.push(DropdownOption {
                value: selected,
                label: "Unavailable source".into(),
                description: None,
            });
        }
        let key = row.name.entity_id();
        let entity = cx.weak_entity();
        let toggle = std::rc::Rc::new(move |_: &mut Window, cx: &mut App| {
            let _ = entity.update(cx, |ws, cx| {
                if let Some(e) = ws.provider_editor.as_mut().filter(|e| !e.busy) {
                    e.variable_source_menu = if e.variable_source_menu == Some(key) {
                        None
                    } else {
                        Some(key)
                    };
                }
                cx.notify();
            });
        });
        let entity = cx.weak_entity();
        let select = std::rc::Rc::new(move |value: &'static str, w: &mut Window, cx: &mut App| {
            let _ = entity.update(cx, |ws, cx| {
                if let Some(e) = ws.provider_editor.as_mut().filter(|e| !e.busy) {
                    e.variable_source_menu = None;
                    let source = value
                        .strip_prefix("source:")
                        .map_or(Source::Device, |s| Source::Vault(s.to_owned()));
                    if let Some(r) = e
                        .variables
                        .iter_mut()
                        .find(|row| row.name.entity_id() == key && row.source != source)
                    {
                        r.source = source;
                        r.saved = None;
                        r.value.update(cx, |f, cx| {
                            f.set_value("", w, cx);
                            f.set_masked(r.source == Source::Device, w, cx);
                            f.set_placeholder(
                                if r.source == Source::Device {
                                    "Value"
                                } else {
                                    "op://vault/item/field"
                                },
                                w,
                                cx,
                            );
                            f.focus(w, cx);
                        });
                        e.error = None;
                    }
                }
                cx.notify();
            });
        });
        let open = self.provider_editor.as_ref()?.variable_source_menu == Some(key);
        let picker = bruhs_ui::dropdown::Dropdown::new(
            saved_option_value(&format!("acp-variable-source-{index}")),
            "Variable value source",
            selected,
            vec![DropdownGroup {
                label: None,
                mark: None,
                options,
            }],
            if open {
                Listbox::Popover
            } else {
                Listbox::Closed
            },
            toggle,
            select,
        )
        .icon_trigger(if selected == "value" {
            assets::TOOL_PENCIL_ICON_PATH
        } else {
            "icons/shield.svg"
        });
        Some(picker.into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::{AcpVariable, Source, Uuid, VariableRow};
    use crate::views::workspace::{tests::thread_menu_workspace, OpenSettings};
    #[gpui::test]
    fn saved_sensitive_value_stays_redacted_and_replacement_gets_a_fresh_key(
        cx: &mut gpui::TestAppContext,
    ) {
        let (workspace, cx, _, _) = thread_menu_workspace(cx);
        workspace.update_in(cx, |ws, w, cx| {
            ws.open_settings(&OpenSettings, w, cx);
            ws.open_provider_editor(None, w, cx);
            let original = AcpVariable::Device {
                revision: Uuid::new_v4(),
            };
            let editor = ws.provider_editor.as_mut().unwrap();
            editor
                .variables
                .push(VariableRow::new("TOKEN", Some(&original), w, cx));
            assert!(editor.variables[0].redacted(cx));
            assert_eq!(editor.environment(cx).unwrap()["TOKEN"], original);
            assert!(editor.device_values(cx).is_empty());
            editor.variables[0]
                .value
                .update(cx, |v, cx| v.replace_all("new test value", w, cx));
            let changed = editor.environment(cx).unwrap();
            let AcpVariable::Device { revision } = changed["TOKEN"] else {
                panic!("device variable")
            };
            assert_ne!(changed["TOKEN"], original);
            assert_eq!(
                editor.device_values(cx)[&revision].as_str(),
                "new test value"
            );
            let mut duplicate = VariableRow::new("TOKEN", None, w, cx);
            duplicate.source = Source::Plain;
            editor.variables.push(duplicate);
            assert!(editor.environment(cx).unwrap_err().contains("twice"));
        });
    }
}
