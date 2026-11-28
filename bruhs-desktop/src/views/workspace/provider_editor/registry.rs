use super::*;
use bruhs_core::agent_extensions::registry::{self as catalog, Agent, Catalog, Prepared};
use bruhs_models::agent_extensions::{AcpVariable, RegistryOrigin};

struct CommandDraft {
    command: Entity<InputState>,
    args: Vec<Entity<InputState>>,
    variables: Vec<environment::VariableRow>,
    name: String,
    id: String,
    suggested_id: String,
    models: Vec<Entity<InputState>>,
    modes: Vec<String>,
    mode_option: String,
    model_option: String,
    show_modes: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Entry {
    Browse,
    ById,
    Local,
}

pub(super) struct RegistryPicker {
    pub(super) entry: Entry,
    pub(super) source_menu_open: bool,
    pub(super) agent_id: Entity<InputState>,
    pub(super) executable_override: Entity<InputState>,
    pub(super) auth_method: Entity<InputState>,
    pub(super) active: bool,
    pub(super) query: Entity<InputState>,
    catalog: Option<Catalog>,
    added: std::collections::HashSet<String>,
    pub(super) loading: bool,
    failure: Option<String>,
    pub(super) selected: Option<String>,
    pub(super) origin: Option<RegistryOrigin>,
    pub(super) install: Option<tokio::task::AbortHandle>,
    manual: Option<CommandDraft>,
}

impl RegistryPicker {
    pub(super) fn new(original: Option<&AcpInstance>, w: &mut Window, cx: &mut App) -> Self {
        let origin = original.and_then(|i| i.registry.as_ref());
        let mut input = |placeholder: &'static str, value: Option<&str>| {
            cx.new(|cx| {
                InputState::new(w, cx)
                    .placeholder(placeholder)
                    .default_value(value.unwrap_or_default().to_owned())
            })
        };
        Self {
            source_menu_open: false,
            active: original.is_none() || origin.is_some(),
            entry: if origin.is_some() {
                Entry::ById
            } else {
                Entry::Browse
            },
            agent_id: input("devin", origin.map(|o| o.agent_id.as_str())),
            executable_override: input(
                "Registry default",
                origin.and_then(|o| o.executable_override.as_deref()),
            ),
            auth_method: input("auto", origin.and_then(|o| o.auth_method_id.as_deref())),
            query: cx.new(|cx| InputState::new(w, cx).placeholder("Search agents…")),
            catalog: None,
            added: Default::default(),
            loading: false,
            failure: None,
            selected: None,
            origin: origin.cloned(),
            install: None,
            manual: None,
        }
    }
    pub(super) fn choice(&self) -> Option<&Agent> {
        self.catalog
            .as_ref()?
            .agents
            .iter()
            .find(|a| Some(&a.id) == self.selected.as_ref())
    }
    pub(super) fn prepared_selection(&self) -> bool {
        self.choice()
            .zip(self.origin.as_ref())
            .is_some_and(|(a, o)| {
                a.id == o.agent_id && a.version == o.version && a.method() == o.distribution
            })
    }
}

impl Workspace {
    pub(super) fn choose_registry_entry(
        &mut self,
        entry: Entry,
        w: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.select_kiro_provider(false, w, cx);
        let Some(e) = self.provider_editor.as_mut() else {
            return;
        };
        if e.busy {
            return;
        }
        let active = entry != Entry::Local;
        e.registry.entry = entry;
        e.registry.source_menu_open = false;
        e.registry.active = active;
        e.error = None;
        if active {
            let input = if entry == Entry::ById {
                &e.registry.agent_id
            } else {
                &e.registry.query
            };
            input.update(cx, |f, c| f.focus(w, c));
            if e.registry.catalog.is_none() {
                self.load_provider_registry(false, cx);
            }
        } else {
            if let Some(draft) = e.registry.manual.take() {
                for input in e
                    .args
                    .iter()
                    .chain(&e.models)
                    .chain(e.variables.iter().flat_map(|v| [&v.name, &v.value]))
                {
                    e.input_events.remove(&input.entity_id());
                }
                e.command = draft.command;
                e.args = draft.args;
                e.variables = draft.variables;
                e.name.update(cx, |f, c| f.set_value(draft.name, w, c));
                e.id.update(cx, |f, c| f.set_value(draft.id, w, c));
                e.suggested_id = draft.suggested_id;
                e.models = draft.models;
                for ((_, field), value) in e.modes.iter().zip(draft.modes) {
                    field.update(cx, |f, c| f.set_value(value, w, c));
                }
                e.mode_option
                    .update(cx, |f, c| f.set_value(draft.mode_option, w, c));
                e.model_option
                    .update(cx, |f, c| f.set_value(draft.model_option, w, c));
                e.show_modes = draft.show_modes;
                e.registry.origin = None;
            }
            e.command.update(cx, |f, c| f.focus(w, c));
        }
        cx.notify();
    }

    pub(super) fn load_provider_registry(&mut self, refresh: bool, cx: &mut Context<'_, Self>) {
        let Some(e) = self.provider_editor.as_mut() else {
            return;
        };
        if e.registry.loading || e.busy {
            return;
        }
        if cfg!(test) {
            // Editor tests run on GPUI's deterministic scheduler and use a local catalog.
            e.registry.catalog = Some(Catalog {
                agents: Vec::new(),
                cached: false,
            });
            cx.notify();
            return;
        }
        e.registry.loading = true;
        e.registry.failure = None;
        let session = e.registry.query.clone();
        let job = cx.global::<DesktopState>().runtime.spawn(async move {
            let added = bruhs_core::agent_extensions::load_config()?
                .acp
                .into_iter()
                .filter_map(|a| a.registry.map(|r| r.agent_id))
                .collect();
            catalog::catalog(refresh).await.map(|list| (list, added))
        });
        cx.spawn(async move |this, cx| {
            let result = job.await.map_err(std::io::Error::other).and_then(|v| v);
            let _ = this.update(cx, |ws, cx| {
                let Some(e) = ws
                    .provider_editor
                    .as_mut()
                    .filter(|e| e.registry.query == session)
                else {
                    return;
                };
                e.registry.loading = false;
                match result {
                    Ok((list, added)) => {
                        e.registry.added = added;
                        e.registry.catalog = Some(list);
                        if e.registry.choice().is_none() {
                            e.registry.selected = None
                        }
                    }
                    Err(err) => {
                        e.registry.failure = Some(format!("Could not load the ACP Registry. {err}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn prepare_registry_provider(&mut self, w: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(e) = self.provider_editor.as_mut() else {
            return;
        };
        if e.registry.prepared_selection() {
            e.page = Page::Details;
            e.name.update(cx, |f, c| f.focus(w, c));
            cx.notify();
            return;
        }
        let Some(agent) = e.registry.choice().cloned() else {
            return;
        };
        let task_agent = agent.clone();
        let session = e.registry.query.clone();
        let task = cx
            .global::<DesktopState>()
            .runtime
            .spawn(async move { catalog::prepare(&task_agent).await });
        e.registry.install = Some(task.abort_handle());
        e.busy = true;
        e.error = None;
        cx.spawn_in(w, async move |this, w| {
            let result = task.await.map_err(std::io::Error::other).and_then(|v| v);
            let _ = this.update_in(w, |ws, w, cx| {
                let Some(e) = ws
                    .provider_editor
                    .as_mut()
                    .filter(|e| e.registry.query == session)
                else {
                    return;
                };
                e.busy = false;
                e.registry.install = None;
                match result {
                    Ok(prepared) => e.use_registry_command(&agent, prepared, w, cx),
                    Err(err) => e.error = Some(format!("Could not install {}. {err}", agent.name)),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn submit_registry_id(&mut self, w: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(e) = self.provider_editor.as_mut() else {
            return;
        };
        let id = e.registry.agent_id.read(cx).value();
        let id = id.trim();
        let error = if id.is_empty() {
            Some("Enter an official registry agent ID.".to_owned())
        } else if e.registry.loading {
            Some("The registry is still loading.".to_owned())
        } else {
            match &e.registry.catalog {
                None => Some("Load the ACP Registry and try again.".to_owned()),
                Some(catalog) if !catalog.agents.iter().any(|a| a.id == id) => Some(format!(
                    "No compatible agent with ID “{id}” was found in the ACP Registry."
                )),
                Some(_) => None,
            }
        };
        if let Some(error) = error {
            e.error = Some(error);
            cx.notify();
            return;
        }
        e.registry.selected = Some(id.to_owned());
        self.prepare_registry_provider(w, cx);
    }

    fn select_builtin_instance_preset(
        &mut self,
        label: &'static str,
        id: &'static str,
        command: &'static str,
        env_var: &'static str,
        w: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.choose_registry_entry(Entry::Local, w, cx);
        let Some(e) = self.provider_editor.as_mut() else {
            return;
        };
        if e.busy || e.original.is_some() {
            return;
        }
        e.name.update(cx, |f, cx| f.set_value(label, w, cx));
        e.id.update(cx, |f, cx| f.set_value(id, w, cx));
        e.suggested_id = id.to_owned();
        e.command.update(cx, |f, cx| f.set_value(command, w, cx));
        if !e
            .variables
            .iter()
            .any(|row| row.name.read(cx).value().trim() == env_var)
        {
            let row = environment::VariableRow::new(
                env_var,
                Some(&AcpVariable::Plain {
                    value: String::new(),
                }),
                w,
                cx,
            );
            let name_input = row.name.clone();
            let value_input = row.value.clone();
            e.variables.push(row);
            e.track_edits(&name_input, cx);
            e.track_edits(&value_input, cx);
        }
        e.error = None;
        cx.notify();
    }

    pub(super) fn provider_source_choices(&self, wide: bool, cx: &Context<'_, Self>) -> AnyElement {
        let Some(e) = self.provider_editor.as_ref() else {
            return div().into_any_element();
        };
        let mut presets = div().flex().flex_wrap().gap_2();
        for (key, title, default_label, default_id, command, env_var) in [
            (
                "claude",
                "Claude",
                "Claude Work",
                "claude-work",
                "claude",
                "CLAUDE_CONFIG_DIR",
            ),
            (
                "codex",
                "Codex",
                "Codex Work",
                "codex-work",
                "codex",
                "CODEX_HOME",
            ),
            (
                "cursor",
                "Cursor",
                "Cursor Work",
                "cursor-work",
                "cursor-agent",
                "CURSOR_CONFIG_DIR",
            ),
            (
                "grok",
                "Grok",
                "Grok Work",
                "grok-work",
                "grok",
                "XAI_API_KEY",
            ),
            (
                "opencode",
                "OpenCode",
                "OpenCode Work",
                "opencode-work",
                "opencode",
                "OPENCODE_CONFIG_DIR",
            ),
        ] {
            let selector = format!("acp-preset-{key}");
            presets = presets.child(editor_button(
                selector,
                title,
                WebButtonVariant::Outline,
                e.busy,
                cx,
                move |ws, w, cx| {
                    ws.select_builtin_instance_preset(
                        default_label,
                        default_id,
                        command,
                        env_var,
                        w,
                        cx,
                    );
                },
            ));
        }
        let mut body = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_sm().child("Provider"))
            .child(hint(
                "Add a named account or isolated instance for a built-in provider.",
            ))
            .child(presets);
        body = body.child(div().flex().child(editor_button(
            "acp-more-providers",
            if e.kiro.expanded {
                "Show less"
            } else {
                "Show more"
            },
            WebButtonVariant::Ghost,
            e.busy,
            cx,
            |ws, _, cx| {
                if let Some(editor) = ws.provider_editor.as_mut() {
                    editor.kiro.expanded = !editor.kiro.expanded;
                }
                cx.notify();
            },
        )));
        if e.kiro.expanded || e.kiro.active {
            body = body.child(self.kiro_provider_tile(cx));
        }
        body = body.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .py_1()
                .child(div().flex_1().h(px(1.)).bg(theme::ring_modal()))
                .child(hint("Or choose from ACP Registry"))
                .child(div().flex_1().h(px(1.)).bg(theme::ring_modal())),
        );
        if e.registry.entry == Entry::Browse || e.kiro.active {
            return body
                .child(self.provider_registry_results(wide, cx))
                .into_any_element();
        }
        let by_id = e.registry.entry == Entry::ById;
        let (title, description) = if by_id {
            (
                "Enter manually",
                "Enter an official registry ID and any executable or authentication override.",
            )
        } else {
            (
                "Local ACP command",
                "Run an installed ACP executable on this computer.",
            )
        };
        let back = editor_button(
            "acp-source-registry",
            "Search registry",
            WebButtonVariant::Ghost,
            e.busy,
            cx,
            |ws, w, cx| ws.choose_registry_entry(Entry::Browse, w, cx),
        );
        body = body.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(title)
                        .child(hint(description)),
                )
                .child(back),
        );
        let mut fields = div()
            .flex()
            .flex_col()
            .gap_4()
            .py_2()
            .when(wide, |form| form.px_3())
            .child(self.registry_source_field(wide, cx));
        if by_id {
            fields = fields
                .child(described_field(
                    "Registry agent ID",
                    Some("Agent identifier from the official ACP Registry."),
                    &e.registry.agent_id,
                    e.busy,
                    wide,
                ))
                .child(self.registry_override_fields(wide));
            if let Some(error) = &e.registry.failure {
                fields = fields
                    .child(
                        div()
                            .id("registry-id-load-error")
                            .role(Role::Status)
                            .text_sm()
                            .child(error.clone()),
                    )
                    .child(self.registry_refresh(cx));
            }
        }
        body = body.child(fields);
        body.into_any_element()
    }

    fn registry_source_field(&self, wide: bool, cx: &Context<'_, Self>) -> Div {
        let Some(editor) = self.provider_editor.as_ref() else {
            return div();
        };
        let local = editor.registry.entry == Entry::Local;
        let value = if local { "local" } else { "registry" };
        let entity = cx.weak_entity();
        let toggle = std::rc::Rc::new(move |_: &mut Window, cx: &mut App| {
            let _ = entity.update(cx, |ws, cx| {
                if let Some(e) = ws.provider_editor.as_mut().filter(|e| !e.busy) {
                    e.registry.source_menu_open = !e.registry.source_menu_open;
                    cx.notify();
                }
            });
        });
        let entity = cx.weak_entity();
        let select = std::rc::Rc::new(move |value: &str, w: &mut Window, cx: &mut App| {
            let entry = if value == "local" {
                Entry::Local
            } else {
                Entry::ById
            };
            let _ = entity.update(cx, |ws, cx| ws.choose_registry_entry(entry, w, cx));
        });
        let options = [("registry", "ACP Registry"), ("local", "Local command")]
            .into_iter()
            .map(|(value, label)| DropdownOption {
                value: value.into(),
                label: label.into(),
                description: None,
            })
            .collect();
        let picker = bruhs_ui::dropdown::Dropdown::new(
            "acp-source-select",
            "ACP source",
            value,
            vec![DropdownGroup {
                label: None,
                mark: None,
                options,
            }],
            if editor.registry.source_menu_open {
                Listbox::Popover
            } else {
                Listbox::Closed
            },
            toggle,
            select,
        );
        let control = if editor.busy {
            div()
                .text_sm()
                .child(if local {
                    "Local command"
                } else {
                    "ACP Registry"
                })
                .into_any_element()
        } else {
            picker.into_any_element()
        };
        div()
            .flex()
            .gap_2()
            .when(wide, |row| row.items_center().justify_between())
            .when(!wide, |row| row.flex_col())
            .child(div().text_sm().child("ACP source"))
            .child(div().when(wide, |field| field.w(px(248.))).child(control))
    }

    pub(super) fn registry_override_fields(&self, wide: bool) -> Div {
        let Some(e) = self.provider_editor.as_ref() else {
            return div();
        };
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(described_field(
                "Executable override",
                Some("Use a local executable with the registry arguments and environment."),
                &e.registry.executable_override,
                e.busy,
                wide,
            ))
            .child(described_field(
                "Authentication method",
                Some("Defaults to the first agent-managed method when sign-in is required."),
                &e.registry.auth_method,
                e.busy,
                wide,
            ))
    }

    fn registry_refresh(&self, cx: &Context<'_, Self>) -> AnyElement {
        let Some(e) = self.provider_editor.as_ref() else {
            return div().into_any_element();
        };
        editor_button(
            "acp-registry-refresh",
            "Refresh",
            WebButtonVariant::Ghost,
            e.registry.loading || e.busy,
            cx,
            |ws, _, cx| ws.load_provider_registry(true, cx),
        )
    }

    fn provider_registry_results(&self, wide: bool, cx: &Context<'_, Self>) -> AnyElement {
        let Some(e) = self.provider_editor.as_ref() else {
            return div().into_any_element();
        };
        let picker = &e.registry;
        let form = div()
            .flex()
            .gap_2()
            .when(wide, |form| form.items_center())
            .when(!wide, |form| form.flex_col())
            .child(
                div().min_w_0().when(wide, |el| el.flex_1()).child(
                    Input::new(&picker.query)
                        .aria_label("Search ACP Registry")
                        .disabled(e.busy),
                ),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .flex_wrap()
                    .child(editor_button(
                        "acp-source-id",
                        "Enter manually",
                        WebButtonVariant::Ghost,
                        e.busy,
                        cx,
                        |ws, w, cx| ws.choose_registry_entry(Entry::ById, w, cx),
                    ))
                    .child(editor_button(
                        "acp-source-local",
                        "Local ACP command",
                        WebButtonVariant::Outline,
                        e.busy,
                        cx,
                        |ws, w, cx| ws.choose_registry_entry(Entry::Local, w, cx),
                    )),
            );
        let mut body = div().flex().flex_col().gap_2().child(form);
        if picker.loading {
            body = body.child(
                hint("Searching the ACP Registry…")
                    .id("acp-registry-loading")
                    .role(Role::Status),
            )
        }
        if let Some(err) = &picker.failure {
            body = body.child(
                div()
                    .id("acp-registry-error")
                    .role(Role::Status)
                    .text_sm()
                    .child(err.clone()),
            )
        }
        if picker.failure.is_some() || picker.catalog.as_ref().is_some_and(|c| c.cached) {
            body = body.child(self.registry_refresh(cx));
        }
        if let Some(catalog) = &picker.catalog {
            if catalog.cached {
                body = body.child(hint("Showing the last available registry index."));
            }
            let query = picker.query.read(cx).value();
            let mut matches = catalog
                .agents
                .iter()
                .filter(|a| {
                    !matches!(
                        a.id.as_str(),
                        "codex-acp"
                            | "claude-acp"
                            | "cursor"
                            | "antigravity-acp"
                            | "grok-build"
                            | "opencode"
                            | "opencode-v2"
                    )
                })
                .filter_map(|a| Some((a.search_rank(&query)?, a)))
                .collect::<Vec<_>>();
            matches.sort_by_key(|(rank, _)| *rank);
            if matches.is_empty() {
                body = body.child(
                    hint("No compatible agents found.")
                        .id("acp-registry-empty")
                        .role(Role::Status),
                )
            }
            if !matches.is_empty() {
                let count = matches.len();
                let status = match count {
                    1 => "1 compatible agent found.".to_owned(),
                    2..=20 => format!("{count} compatible agents found."),
                    _ => format!(
                        "Showing 20 of {count} compatible agents. Refine your search to see more."
                    ),
                };
                body = body.child(
                    div()
                        .id("acp-registry-count")
                        .role(Role::Status)
                        .text_xs()
                        .text_color(theme::rambutan_400())
                        .child(status),
                );
            }
            let mut rows = div()
                .id("acp-registry-results")
                .role(Role::List)
                .aria_label(format!(
                    "ACP Registry results, {} compatible agents",
                    matches.len().min(20)
                ))
                .max_h(px(256.))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_1();
            for (_, a) in matches.into_iter().take(20) {
                let added = picker.added.contains(&a.id);
                let preparing = e.busy && picker.selected.as_ref() == Some(&a.id);
                let label = if added {
                    "Added"
                } else if preparing && a.method() == "Binary" {
                    "Downloading…"
                } else if preparing {
                    "Preparing…"
                } else {
                    "Add"
                };
                let disabled = e.busy || added;
                let id = a.id.clone();
                let button = web_button(
                    SharedString::from(format!("acp-registry-{}", a.id)),
                    WebButtonVariant::Outline,
                    WebButtonSize::Small,
                    ButtonState::disabled_if(disabled),
                )
                .aria_label(format!("{label} {}", a.name))
                .child(label)
                .when(!disabled, |button| {
                    on_press_view(button, cx, move |ws, w, cx| {
                        ws.choose_registry_entry(Entry::Browse, w, cx);
                        if let Some(e) = &mut ws.provider_editor {
                            e.registry.selected = Some(id.clone());
                        }
                        ws.prepare_registry_provider(w, cx);
                    })
                });
                let about = a.about.as_ref().map(|url| {
                    let url = url.clone();
                    let button = web_button(
                        SharedString::from(format!("acp-about-{}", a.id)),
                        WebButtonVariant::Ghost,
                        WebButtonSize::IconSmall,
                        ButtonState::Enabled,
                    )
                    .aria_label(format!("About {}", a.name))
                    .child(svg().path("icons/external-link.svg").size(px(14.)));
                    on_press_view(button, cx, move |_, _, cx| cx.open_url(&url))
                });
                rows = rows.child(
                    div()
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .gap_2()
                        .py_2()
                        .child(
                            svg()
                                .path("icons/bot.svg")
                                .size(px(22.))
                                .flex_shrink_0()
                                .text_color(theme::rambutan_400()),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(div().truncate().child(a.name.clone()))
                                        .child(
                                            div()
                                                .flex_shrink_0()
                                                .text_xs()
                                                .text_color(theme::rambutan_400())
                                                .child(format!("v{} · {}", a.version, a.method())),
                                        ),
                                )
                                .child(
                                    div()
                                        .truncate()
                                        .text_xs()
                                        .text_color(theme::rambutan_400())
                                        .child(a.description.clone()),
                                ),
                        )
                        .children(about)
                        .child(button),
                );
            }
            body = body.child(rows);
        }
        body.into_any_element()
    }
}

impl ProviderEditor {
    fn use_registry_command(
        &mut self,
        a: &Agent,
        p: Prepared,
        w: &mut Window,
        cx: &mut Context<'_, Workspace>,
    ) {
        let command = field(&p.command.to_string_lossy(), w, cx);
        let args = p.args.iter().map(|v| field(v, w, cx)).collect();
        let variables = p
            .environment
            .into_iter()
            .map(|(k, value)| {
                environment::VariableRow::new(&k, Some(&AcpVariable::Plain { value }), w, cx)
            })
            .collect();
        let draft = CommandDraft {
            command: std::mem::replace(&mut self.command, command),
            args: std::mem::replace(&mut self.args, args),
            variables: std::mem::replace(&mut self.variables, variables),
            name: self.name.read(cx).value().to_string(),
            id: self.id.read(cx).value().to_string(),
            suggested_id: self.suggested_id.clone(),
            models: std::mem::take(&mut self.models),
            modes: self
                .modes
                .iter()
                .map(|(_, f)| f.read(cx).value().to_string())
                .collect(),
            mode_option: self.mode_option.read(cx).value().to_string(),
            model_option: self.model_option.read(cx).value().to_string(),
            show_modes: std::mem::take(&mut self.show_modes),
        };
        for f in self
            .modes
            .iter()
            .map(|(_, f)| f)
            .chain([&self.mode_option, &self.model_option])
        {
            f.update(cx, |f, c| f.set_value("", w, c));
        }
        if self.registry.manual.is_none() {
            self.registry.manual = Some(draft);
        } else {
            for f in std::iter::once(draft.command)
                .chain(draft.args)
                .chain(draft.models)
                .chain(draft.variables.into_iter().flat_map(|v| [v.name, v.value]))
            {
                self.input_events.remove(&f.entity_id());
            }
        }
        let fields: Vec<_> = self
            .args
            .iter()
            .chain(self.variables.iter().flat_map(|v| [&v.name, &v.value]))
            .cloned()
            .collect();
        for f in fields {
            self.track_edits(&f, cx)
        }
        self.registry.origin = Some(a.origin());
        self.name
            .update(cx, |f, c| f.set_value(a.name.clone(), w, c));
        self.suggested_id = suggested_id(&a.id);
        self.id
            .update(cx, |f, c| f.set_value(self.suggested_id.clone(), w, c));
        self.page = Page::Details;
        self.name.update(cx, |f, c| f.focus(w, c));
    }
}
