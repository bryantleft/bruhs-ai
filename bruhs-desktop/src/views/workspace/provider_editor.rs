//! Local ACP instances share their identity and configuration with Workbench and chat.
mod accent;
mod account;
mod environment;
mod kiro;
mod registry;
use super::*;
use bruhs_models::agent_extensions::AcpInstance;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Command,
    Details,
    Config,
    Edit,
    Account,
    Remove,
}

pub(super) struct ProviderEditor {
    original: Option<AcpInstance>,
    page: Page,
    registry: registry::RegistryPicker,
    kiro: kiro::KiroPicker,
    accent: accent::AccentPicker,
    account: account::SignIn,
    name: Entity<InputState>,
    id: Entity<InputState>,
    command: Entity<InputState>,
    args: Vec<Entity<InputState>>,
    variables: Vec<environment::VariableRow>,
    variable_source_menu: Option<gpui::EntityId>,
    models: Vec<Entity<InputState>>,
    show_modes: bool,
    modes: Vec<(PermissionMode, Entity<InputState>)>,
    mode_option: Entity<InputState>,
    model_option: Entity<InputState>,
    busy: bool,
    error: Option<String>,
    pub(super) start: FocusHandle,
    pub(super) end: FocusHandle,
    trigger: Option<FocusHandle>,
    _follow_name: Option<Subscription>,
    _source_changes: Option<Subscription>,
    input_events: std::collections::HashMap<gpui::EntityId, Subscription>,
    suggested_id: String,
}

fn field(value: &str, window: &mut Window, cx: &mut App) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx).default_value(value.to_owned()))
}

fn suggested_id(name: &str) -> String {
    name.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
        .to_ascii_lowercase()
}

impl ProviderEditor {
    fn new(original: Option<AcpInstance>, window: &mut Window, cx: &mut App) -> Self {
        let instance = original.as_ref();
        Self {
            page: if instance.is_some() {
                Page::Edit
            } else {
                Page::Command
            },
            account: Default::default(),
            registry: registry::RegistryPicker::new(instance, window, cx),
            kiro: kiro::KiroPicker::new(instance, window, cx),
            accent: accent::AccentPicker::new(
                instance.and_then(|i| i.accent_color.as_deref()),
                window,
                cx,
            ),
            name: field(instance.map_or("", |i| &i.name), window, cx),
            id: field(instance.map_or("", |i| &i.id), window, cx),
            command: field(instance.map_or("", |i| &i.command), window, cx),
            args: instance
                .into_iter()
                .flat_map(|i| &i.args)
                .map(|arg| field(arg, window, cx))
                .collect(),
            variables: instance
                .into_iter()
                .flat_map(|i| &i.environment)
                .map(|(name, value)| environment::VariableRow::new(name, Some(value), window, cx))
                .collect(),
            variable_source_menu: None,
            models: instance
                .into_iter()
                .flat_map(|i| &i.models)
                .map(|model| field(model, window, cx))
                .collect(),
            show_modes: false,
            modes: [
                PermissionMode::Default,
                PermissionMode::Plan,
                PermissionMode::AcceptEdits,
                PermissionMode::BypassPermissions,
            ]
            .into_iter()
            .map(|mode| {
                let value = instance
                    .and_then(|agent| agent.modes.get(mode.as_str()))
                    .map_or("", String::as_str);
                (mode, field(value, window, cx))
            })
            .collect(),
            mode_option: field(
                instance
                    .and_then(|agent| agent.mode_config_id.as_deref())
                    .unwrap_or(""),
                window,
                cx,
            ),
            model_option: field(
                instance
                    .and_then(|agent| agent.model_config_id.as_deref())
                    .unwrap_or(""),
                window,
                cx,
            ),
            _follow_name: None,
            _source_changes: None,
            input_events: Default::default(),
            suggested_id: String::new(),
            original,
            busy: false,
            error: None,
            start: cx.focus_handle(),
            end: cx.focus_handle(),
            trigger: window.focused(cx),
        }
    }

    fn track_edits(&mut self, input: &Entity<InputState>, cx: &mut Context<'_, Workspace>) {
        let sub = cx.subscribe(input, |ws, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                if let Some(editor) = &mut ws.provider_editor {
                    if editor.error.take().is_some()
                        || editor.registry.query == input
                        || editor.variables.iter().any(|r| r.value == input)
                    {
                        cx.notify();
                    }
                }
            }
        });
        self.input_events.insert(input.entity_id(), sub);
    }

    fn value(&self, cx: &App) -> Result<AcpInstance, String> {
        let name = self.name.read(cx).value().trim().to_owned();
        if name.is_empty() {
            return Err("Enter a label.".into());
        }
        let raw_id = self.id.read(cx).value();
        let id = if let Some(previous) = &self.original {
            previous.id.clone()
        } else if raw_id.trim().is_empty() {
            if self.kiro.active {
                kiro::instance_id(&name)
            } else {
                suggested_id(&name)
            }
        } else {
            raw_id.trim().to_owned()
        };
        if self.kiro.active {
            let mut agent = AcpInstance::kiro(id, name, &self.kiro.binary.read(cx).value());
            agent.accent_color = self.accent.selected.map(|rgb| format!("#{rgb:06x}"));
            if let Some(original) = &self.original {
                agent.enabled = original.enabled;
                agent.environment = self.environment(cx)?;
                agent.models = self
                    .models
                    .iter()
                    .map(|field| field.read(cx).value().trim().to_owned())
                    .collect();
                if agent.models.iter().any(String::is_empty) {
                    return Err("Enter a model ID or remove its row.".into());
                }
            }
            agent.validate()?;
            return Ok(agent);
        }
        let args = self
            .args
            .iter()
            .map(|a| a.read(cx).value().to_string())
            .collect();
        let models = self
            .models
            .iter()
            .map(|m| m.read(cx).value().trim().to_owned())
            .collect::<Vec<_>>();
        if models.iter().any(String::is_empty) {
            return Err("Enter a model ID or remove its row.".into());
        }
        let mut agent = self.original.clone().unwrap_or_else(|| AcpInstance {
            driver: None,
            id: String::new(),
            name: String::new(),
            registry: None,
            accent_color: None,
            command: String::new(),
            args: vec![],
            environment: Default::default(),
            models: vec![],
            modes: Default::default(),
            mode_config_id: None,
            model_config_id: None,
            enabled: true,
        });
        agent.id = id;
        agent.name = name;
        agent.accent_color = self.accent.selected.map(|rgb| format!("#{rgb:06x}"));
        agent.command = self.command.read(cx).value().trim().to_owned();
        if self.registry.active {
            agent.registry = self.registry.origin.clone();
        } else if self
            .original
            .as_ref()
            .is_some_and(|old| old.command != agent.command)
        {
            agent.registry = None;
        }
        if let Some(origin) = &mut agent.registry {
            if self.registry.entry == registry::Entry::ById {
                let optional = |field: &Entity<InputState>| {
                    let value = field.read(cx).value();
                    (!value.trim().is_empty()).then(|| value.trim().to_owned())
                };
                origin.executable_override = optional(&self.registry.executable_override);
                origin.auth_method_id = optional(&self.registry.auth_method);
            }
        }
        agent.args = args;
        agent.environment = self.environment(cx)?;
        agent.models = models;
        for (mode, input) in &self.modes {
            let value = input.read(cx).value();
            if value.trim().is_empty() {
                agent.modes.remove(mode.as_str());
            } else {
                agent
                    .modes
                    .insert(mode.as_str().into(), value.trim().to_owned());
            }
        }
        let option = |input: &Entity<InputState>| {
            let value = input.read(cx).value();
            (!value.trim().is_empty()).then(|| value.trim().to_owned())
        };
        agent.mode_config_id = option(&self.mode_option);
        agent.model_config_id = option(&self.model_option);
        agent.validate()?;
        Ok(agent)
    }
}

impl Workspace {
    pub(super) fn open_provider_editor(
        &mut self,
        id: Option<&str>,
        w: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.provider_editor.is_some() || self.provider_change_pending {
            return;
        }
        let original = match id {
            Some(id) => match bruhs_core::agent_extensions::load_config().and_then(|config| {
                config
                    .acp
                    .into_iter()
                    .find(|a| a.id == id)
                    .ok_or_else(|| std::io::Error::other("This ACP instance was removed."))
            }) {
                Ok(agent) => Some(agent),
                Err(err) => {
                    self.provider_change_error = Some(err.to_string());
                    self.check_provider_health(cx);
                    cx.notify();
                    return;
                }
            },
            None => None,
        };
        let mut editor = ProviderEditor::new(original, w, cx);
        editor.accent.observe_hex(w, cx);
        let sources = self.credential_settings.read(cx).secret_sources();
        editor._source_changes = Some(cx.observe(&sources, |_, _, cx| cx.notify()));
        sources.update(cx, |s, cx| s.reload(cx));
        if editor.original.is_none() {
            editor._follow_name = Some(cx.subscribe_in(
                &editor.name,
                w,
                |ws, name, event: &InputEvent, w, cx| {
                    if !matches!(event, InputEvent::Change) {
                        return;
                    }
                    let Some(e) = ws.provider_editor.as_mut().filter(|e| &e.name == name) else {
                        return;
                    };
                    if e.registry.active
                        && e.registry.origin.is_some()
                        && e.registry
                            .choice()
                            .is_some_and(|a| a.name == name.read(cx).value().as_str())
                    {
                        return;
                    }
                    let id = e.id.read(cx).value();
                    if id.trim().is_empty() || id.as_ref() == e.suggested_id {
                        e.suggested_id = if e.kiro.active {
                            kiro::instance_id(&name.read(cx).value())
                        } else {
                            suggested_id(&name.read(cx).value())
                        };
                        let next = e.suggested_id.clone();
                        e.id.update(cx, |f, cx| f.set_value(next, w, cx));
                    }
                },
            ));
        }
        let fields = [
            &editor.registry.query,
            &editor.registry.agent_id,
            &editor.registry.executable_override,
            &editor.registry.auth_method,
            &editor.kiro.binary,
            &editor.command,
            &editor.name,
            &editor.id,
            &editor.mode_option,
            &editor.model_option,
        ]
        .into_iter()
        .chain(editor.variables.iter().flat_map(|r| [&r.name, &r.value]))
        .chain(&editor.args)
        .chain(&editor.models)
        .chain(editor.modes.iter().map(|(_, field)| field))
        .cloned()
        .collect::<Vec<_>>();
        for field in fields {
            editor.track_edits(&field, cx);
        }
        let adding = editor.page == Page::Command;
        let target = if adding {
            &editor.registry.query
        } else {
            &editor.name
        };
        target.update(cx, |f, cx| f.focus(w, cx));
        self.provider_editor = Some(editor);
        if adding {
            self.load_provider_registry(false, cx);
        }
        cx.notify();
    }

    pub(super) fn dismiss_provider_editor(&mut self, w: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(e) = self.provider_editor.as_mut() else {
            return;
        };
        if e.page == Page::Account {
            self.finish_provider_account(w, cx);
            return;
        }
        if let Some(task) = e.registry.install.take() {
            task.abort();
            e.busy = false;
        }
        if e.busy {
            return;
        }
        if std::mem::take(&mut e.registry.source_menu_open)
            || e.variable_source_menu.take().is_some()
        {
            cx.notify();
            return;
        }
        if e.accent.open {
            e.accent.close(w, cx);
            cx.notify();
            return;
        }
        if e.page == Page::Edit && e.account.return_after_save {
            if let Some(id) = e.original.as_ref().map(|i| i.id.clone()) {
                let from_edit = e.account.from_edit();
                let trigger = e.trigger.clone();
                self.provider_editor = None;
                self.open_provider_editor(Some(&id), w, cx);
                if let Some(editor) = &mut self.provider_editor {
                    editor.trigger = trigger;
                }
                self.show_provider_account(from_edit, w, cx);
                return;
            }
        }
        if e.page == Page::Remove {
            e.page = Page::Edit;
            e.error = None;
            e.name.update(cx, |f, cx| f.focus(w, cx));
        } else {
            let trigger = e.trigger.clone();
            self.provider_editor = None;
            if let Some(focus) = trigger {
                focus.focus(w, cx);
            } else {
                self.focus_modal_dialog(w, cx);
            }
        }
        cx.notify();
    }

    fn submit_provider_editor(&mut self, w: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(e) = self.provider_editor.as_mut() else {
            return;
        };
        if e.busy {
            return;
        }
        e.error = None;
        e.registry.source_menu_open = false;
        if e.page == Page::Account {
            return;
        }
        if e.page == Page::Command && e.kiro.active {
            e.page = Page::Details;
            e.name.update(cx, |f, cx| f.focus(w, cx));
            cx.notify();
            return;
        }
        if e.page == Page::Command && e.registry.active {
            if e.registry.entry == registry::Entry::ById {
                self.submit_registry_id(w, cx);
            }
            return;
        }
        if e.page == Page::Details && e.kiro.active {
            let label = e.name.read(cx).value();
            let raw_id = e.id.read(cx).value();
            let id = if raw_id.trim().is_empty() {
                kiro::instance_id(&label)
            } else {
                raw_id.trim().to_owned()
            };
            if let Err(error) = AcpInstance::kiro(id, label.trim().to_owned(), "").validate() {
                e.error = Some(error);
            } else {
                e.page = Page::Config;
                e.kiro.binary.update(cx, |f, c| f.focus(w, c));
            }
            cx.notify();
            return;
        }
        if e.page == Page::Command {
            let raw = e.command.read(cx).value();
            let path = raw.trim();
            if path.is_empty() || path.starts_with('-') || path.contains('\0') {
                e.error = Some("Enter an executable name or path.".into());
            } else if let Err(err) = e.environment(cx) {
                e.error = Some(err);
            } else {
                e.variable_source_menu = None;
                e.page = Page::Details;
                if e.name.read(cx).value().trim().is_empty() {
                    let default = std::path::Path::new(path)
                        .file_name()
                        .and_then(|v| v.to_str())
                        .unwrap_or(path);
                    e.name
                        .update(cx, |f, cx| f.set_value(default.to_owned(), w, cx));
                }
                if e.id.read(cx).value().is_empty() {
                    e.suggested_id = suggested_id(&e.name.read(cx).value());
                    let next = e.suggested_id.clone();
                    e.id.update(cx, |f, cx| f.set_value(next, w, cx));
                }
                e.name.update(cx, |f, cx| f.focus(w, cx));
            }
            cx.notify();
            return;
        }
        let next = if e.page == Page::Remove {
            None
        } else {
            match e.value(cx) {
                Ok(next) => Some(next),
                Err(err) => {
                    e.error = Some(err);
                    cx.notify();
                    return;
                }
            }
        };
        let values = if next.is_some() && (!e.kiro.active || e.original.is_some()) {
            e.device_values(cx)
        } else {
            Default::default()
        };
        let expected = e.original.clone();
        let registry_changed = [&expected, &next]
            .into_iter()
            .flatten()
            .any(|i| i.registry.is_some());
        let session = e.name.clone();
        let account_target = next
            .as_ref()
            .filter(|i| (expected.is_none() && i.registry.is_some()) || e.account.return_after_save)
            .map(|i| i.id.clone());
        let return_to_edit = e.account.from_edit();
        let removing = e.page == Page::Remove;
        e.busy = true;
        let save = cx.background_spawn(async move {
            bruhs_core::agent_extensions::instances::edit_with_values(
                expected.as_ref(),
                next.as_ref(),
                values,
            )
        });
        cx.spawn_in(w, async move |this, cx| {
            let result = save.await;
            let _ = this.update_in(cx, |this, w, cx| {
                if let Some(e) = this.provider_editor.as_mut().filter(|e| e.name == session) {
                    e.busy = false;
                    match result {
                        Ok(()) => {
                            let trigger = e.trigger.clone();
                            this.provider_editor = None;
                            if let Some(id) = &account_target {
                                this.open_provider_editor(Some(id), w, cx);
                                if let Some(editor) = &mut this.provider_editor {
                                    editor.trigger = trigger;
                                }
                                this.show_provider_account(return_to_edit, w, cx);
                            } else if removing {
                                this.provider_add_focus.focus(w, cx);
                            } else if let Some(focus) = trigger {
                                focus.focus(w, cx);
                            } else {
                                this.focus_modal_dialog(w, cx);
                            }
                        }
                        Err(ref error) => {
                            e.error = Some(error.to_string());
                        }
                    }
                }
                if result.is_ok() {
                    if registry_changed {
                        this.updates
                            .update(cx, |u, cx| u.registry_config_changed(cx));
                    } else {
                        this.health_check = None;
                        this.check_provider_health(cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_provider_editor(
        &self,
        w: &Window,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        let e = self.provider_editor.as_ref()?;
        let wide = w.viewport_size().width >= px(620.);
        let edit_field = |label, input: &Entity<InputState>, disabled| {
            editor_field(label, input, disabled, wide)
        };
        let mut body = div()
            .id("acp-editor-body")
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_3()
            .p_4();
        let mut footer = div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_end()
            .gap_2()
            .px_4()
            .py_3()
            .border_t_1()
            .border_color(theme::ring_modal());
        let title = match e.page {
            Page::Command | Page::Details | Page::Config => "Add provider",
            Page::Account if !e.account.from_edit() => "Add provider",
            Page::Account => "Provider sign-in",
            Page::Edit => "Edit provider",
            Page::Remove => "Remove provider",
        };
        let adding = matches!(e.page, Page::Command | Page::Details | Page::Config)
            || (e.page == Page::Account && !e.account.from_edit());
        let mut header = div()
            .flex()
            .flex_col()
            .gap_2()
            .px_5()
            .py_4()
            .border_b_1()
            .border_color(theme::ring_modal());
        let close = web_button(
            "provider-editor-close",
            WebButtonVariant::Ghost,
            WebButtonSize::Small,
            ButtonState::disabled_if(e.busy && e.registry.install.is_none()),
        )
        .aria_label("Close provider dialog")
        .child(
            svg()
                .path(assets::CLOSE_ICON_PATH)
                .size(px(15.))
                .text_color(theme::rambutan_400()),
        );
        header = header.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                .child(on_press_view(close, cx, Self::dismiss_provider_editor)),
        );
        if adding {
            header = header.child(hint(
                "Add an account or configure a provider on this computer.",
            ));
            let mut steps = div()
                .flex()
                .items_center()
                .gap_1()
                .p_1()
                .rounded_md()
                .bg(theme::longan_950());
            for (index, (page, label)) in [
                (Page::Command, "Provider"),
                (Page::Details, "Identity"),
                if e.registry.active {
                    (Page::Account, "Sign in")
                } else {
                    (Page::Config, "Config")
                },
            ]
            .into_iter()
            .enumerate()
            {
                if page == Page::Config && !e.registry.active && !e.kiro.active {
                    continue;
                }
                let current = e.page == page;
                let completed = matches!(
                    (page, e.page),
                    (Page::Command, Page::Details | Page::Config | Page::Account)
                        | (Page::Details, Page::Config | Page::Account)
                );
                let step = web_button(
                    format!("provider-step-{index}"),
                    WebButtonVariant::Ghost,
                    WebButtonSize::Small,
                    ButtonState::disabled_if(
                        e.busy || !(current || completed) || (e.page == Page::Account && completed),
                    ),
                )
                .aria_label(format!("Step {}. {label}", index + 1))
                .flex_1()
                .min_w_0()
                .justify_start()
                .gap_2()
                .when(current, |step| step.bg(theme::longan_800()))
                .child(
                    div()
                        .size(px(20.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .border_1()
                        .border_color(if current || completed {
                            theme::persimmon_400()
                        } else {
                            theme::ring_modal()
                        })
                        .when(completed, |dot| {
                            dot.bg(theme::persimmon_400()).child(
                                svg()
                                    .path(assets::CHECK_ICON_PATH)
                                    .size(px(12.))
                                    .text_color(theme::longan_950()),
                            )
                        })
                        .when(!completed, |dot| {
                            dot.text_xs().child((index + 1).to_string())
                        }),
                )
                .child(label);
                steps = steps.child(step.when(
                    completed && !e.busy && e.page != Page::Account,
                    |button| {
                        on_press_view(button, cx, move |ws, w, cx| {
                            let Some(e) = ws.provider_editor.as_mut() else {
                                return;
                            };
                            e.page = page;
                            e.error = None;
                            if page == Page::Details {
                                e.name.update(cx, |f, c| f.focus(w, c));
                            } else {
                                e.start.focus(w, cx);
                            }
                            cx.notify();
                        })
                    },
                ));
            }
            header = header.child(steps);
        }
        if e.page == Page::Account {
            if e.account.from_edit() {
                header = header.child(
                    div()
                        .text_sm()
                        .text_color(theme::rambutan_400())
                        .child(e.original.as_ref()?.name.clone()),
                );
            }
            body = body.child(self.account_body(wide, cx));
        } else if e.page == Page::Remove {
            let name = &e.original.as_ref()?.name;
            body = body.child(format!(
                "Remove {name} from this computer? Its saved configuration will be deleted."
            ));
        } else {
            if e.page == Page::Details && e.registry.active {
                if let Some(agent) = e.registry.choice() {
                    body = body.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .pb_2()
                            .border_b_1()
                            .border_color(theme::ring_modal())
                            .child(agent.name.clone())
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::rambutan_400())
                                    .child(format!("v{} · {}", agent.version, agent.method())),
                            ),
                    );
                }
            }
            if matches!(e.page, Page::Details | Page::Edit) {
                body = body
                    .child(described_field(
                        "Label",
                        Some("Shown in the provider list."),
                        &e.name,
                        e.busy,
                        wide,
                    ))
                    .child(described_field(
                        "Instance ID",
                        Some("Letters, digits, hyphens, or underscores."),
                        &e.id,
                        e.busy || e.original.is_some(),
                        wide,
                    ));
                body = body.child(self.provider_accent_row(wide, cx));
            }
            if e.page == Page::Command {
                body = body.child(self.provider_source_choices(wide, cx));
            }
            if e.page == Page::Edit && !e.kiro.active {
                body = body.child(
                    div()
                        .flex()
                        .gap_2()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child("Account")
                                .child(hint("Uses the saved provider configuration.")),
                        )
                        .child(editor_button(
                            "acp-account-open",
                            "Manage sign-in",
                            WebButtonVariant::Outline,
                            e.busy || !e.original.as_ref().is_some_and(|i| i.enabled),
                            cx,
                            |ws, w, cx| ws.show_provider_account(true, w, cx),
                        )),
                );
            }
            if e.page == Page::Edit {
                if let Some(origin) = e.original.as_ref().and_then(|i| i.registry.as_ref()) {
                    body = body.child(div().text_sm().child(format!(
                        "ACP Registry · {} · v{}",
                        origin.agent_id, origin.version
                    )));
                }
            }
            if e.page == Page::Edit && e.original.as_ref().is_some_and(|i| i.registry.is_some()) {
                body = body.child(self.registry_override_fields(wide));
            }
            if e.kiro.active && matches!(e.page, Page::Config | Page::Edit) {
                body = body.child(described_field(
                    "Binary path",
                    Some("Path to the Kiro CLI binary."),
                    &e.kiro.binary,
                    e.busy,
                    wide,
                ));
                if e.page == Page::Edit {
                    body = body
                        .child(self.provider_list_fields(true, cx))
                        .child(self.provider_variables(wide, cx));
                }
            }
            let registry_identity = e.page == Page::Details && e.registry.active;
            if registry_identity {
                body = body.child(div().flex().child(editor_button(
                    "acp-registry-advanced",
                    if e.account.advanced {
                        "Hide advanced configuration"
                    } else {
                        "Advanced configuration"
                    },
                    WebButtonVariant::Ghost,
                    e.busy,
                    cx,
                    |ws, _, cx| {
                        if let Some(e) = &mut ws.provider_editor {
                            e.account.advanced = !e.account.advanced;
                        }
                        cx.notify();
                    },
                )));
            }
            let registry_fields = registry_identity && e.account.advanced;
            if !e.kiro.active
                && (registry_fields
                    || matches!(e.page, Page::Config | Page::Edit)
                    || (e.page == Page::Command && !e.registry.active))
            {
                body = body
                    .child(edit_field(
                        "Executable",
                        &e.command,
                        e.busy
                            || e.page == Page::Config
                            || registry_fields
                            || e.original.as_ref().is_some_and(|a| a.registry.is_some()),
                    ))
                    .child(hint("Executable name or path on this computer."))
                    .child(self.provider_list_fields(false, cx))
                    .child(self.provider_variables(wide, cx));
            }
            if !e.kiro.active
                && (registry_fields
                    || matches!(e.page, Page::Config | Page::Edit)
                    || (e.page == Page::Details && !e.registry.active))
            {
                body = body
                    .child(self.provider_list_fields(true, cx))
                    .when(e.page == Page::Config, |body|body.child(hint("Map Default, Plan, or Accept edits to a native mode ID before using that access level in chat.")))
                    .child(div().flex().child(editor_button(
                        "acp-modes",
                        if e.show_modes {
                            "Hide access modes"
                        } else {
                            "Configure access modes"
                        },
                        WebButtonVariant::Outline,
                        e.busy,
                        cx,
                        |this, _, cx| {
                            if let Some(e) = &mut this.provider_editor {
                                e.show_modes = !e.show_modes;
                            }
                            cx.notify();
                        },
                    )));
                if e.show_modes {
                    body = body.child(hint(
                        "Enter the mode ID this agent advertises for each access level you use.",
                    ));
                    for (mode, input) in &e.modes {
                        body = body.child(edit_field(mode.label(), input, e.busy));
                    }
                    body = body
                        .child(edit_field("Mode config option ID", &e.mode_option, e.busy))
                        .child(edit_field(
                            "Model config option ID",
                            &e.model_option,
                            e.busy,
                        ))
                        .child(hint("Leave blank to use the standard ACP selectors."));
                }
            }
        }
        if let Some(error) = &e.error {
            body = body.child(
                div()
                    .id("acp-editor-error")
                    .role(Role::Alert)
                    .text_sm()
                    .text_color(theme::lychee_400())
                    .child(error.clone()),
            );
        }
        if e.page == Page::Edit {
            footer = footer
                .child(editor_button(
                    "acp-request-remove",
                    "Remove",
                    WebButtonVariant::Ghost,
                    e.busy,
                    cx,
                    |this, w, cx| {
                        if let Some(e) = &mut this.provider_editor {
                            e.page = Page::Remove;
                            e.error = None;
                            e.start.focus(w, cx);
                        }
                        cx.notify();
                    },
                ))
                .child(div().flex_1());
        }
        if matches!(e.page, Page::Details | Page::Config) {
            footer = footer.child(editor_button(
                "acp-back",
                "Back",
                WebButtonVariant::Outline,
                e.busy,
                cx,
                |this, w, cx| {
                    if let Some(e) = &mut this.provider_editor {
                        let focus = if e.page == Page::Config {
                            e.page = Page::Details;
                            &e.name
                        } else {
                            e.page = Page::Command;
                            if e.kiro.active {
                                e.start.focus(w, cx);
                                cx.notify();
                                return;
                            } else if e.registry.entry == registry::Entry::ById {
                                &e.registry.agent_id
                            } else if e.registry.active {
                                &e.registry.query
                            } else {
                                &e.command
                            }
                        };
                        e.error = None;
                        focus.update(cx, |f, c| f.focus(w, c));
                    }
                    cx.notify();
                },
            ));
        }
        if !matches!(e.page, Page::Details | Page::Config | Page::Account) {
            footer = footer.child(editor_button(
                "acp-cancel",
                "Cancel",
                WebButtonVariant::Outline,
                e.busy && e.registry.install.is_none(),
                cx,
                Self::dismiss_provider_editor,
            ));
        }
        if e.page == Page::Account {
            footer = footer.child(self.account_footer(cx));
        }
        footer = footer.when(
            e.page != Page::Account
                && (e.page != Page::Command
                    || !e.registry.active
                    || e.registry.entry == registry::Entry::ById),
            |footer| {
                footer.child(editor_button(
                    "acp-submit",
                    match e.page {
                        Page::Command => "Next",
                        Page::Details if e.registry.active => "Continue to sign-in",
                        Page::Details if e.kiro.active => "Next",
                        Page::Config => "Add instance",
                        Page::Details => "Add instance",
                        Page::Account => unreachable!(),
                        Page::Edit => "Save",
                        Page::Remove => "Remove provider",
                    },
                    if e.page == Page::Remove {
                        WebButtonVariant::Destructive
                    } else {
                        WebButtonVariant::Primary
                    },
                    e.busy || (e.page == Page::Command && e.registry.active && e.registry.loading),
                    cx,
                    Self::submit_provider_editor,
                ))
            },
        );
        Some(
            div()
                .id("acp-editor-overlay")
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
                        .id("acp-editor-dialog")
                        .role(Role::Dialog)
                        .aria_label(title)
                        .track_focus(&e.start)
                        .tab_group()
                        .on_key_down(cx.listener(Self::trap_modal_tab_focus))
                        .w(px(768.))
                        .max_w(w.viewport_size().width - px(32.))
                        .max_h(w.viewport_size().height - px(40.))
                        .rounded_xl()
                        .border_1()
                        .border_color(theme::longan_700())
                        .bg(theme::longan_900())
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .child(header)
                        .child(body)
                        .child(footer)
                        .child(div().size(px(0.)).track_focus(&e.end)),
                )
                .into_any_element(),
        )
    }

    fn provider_list_fields(&self, models: bool, cx: &Context<'_, Self>) -> AnyElement {
        let Some(e) = self.provider_editor.as_ref() else {
            return div().into_any_element();
        };
        let (label, key, add, fields) = if models {
            ("Models", "model", "Add model", &e.models)
        } else {
            ("Arguments", "argument", "Add argument", &e.args)
        };
        let mut list = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().text_sm().child(label));
        for (index, f) in fields.iter().enumerate() {
            let target = f.clone();
            let remove = editor_button(
                format!("acp-remove-{key}-{index}"),
                "Remove",
                WebButtonVariant::Ghost,
                e.busy,
                cx,
                move |this, w, cx| {
                    if let Some(e) = &mut this.provider_editor {
                        let fields = if models { &mut e.models } else { &mut e.args };
                        let Some(index) = fields.iter().position(|field| field == &target) else {
                            return;
                        };
                        fields.remove(index);
                        e.input_events.remove(&target.entity_id());
                        e.error = None;
                        if let Some(f) = fields.get(index.min(fields.len().saturating_sub(1))) {
                            f.update(cx, |f, cx| f.focus(w, cx));
                        } else {
                            e.start.focus(w, cx);
                        }
                    }
                    cx.notify();
                },
            );
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(f)
                                .aria_label(format!("{label} {}", index + 1))
                                .disabled(e.busy),
                        ),
                    )
                    .child(remove),
            );
        }
        list.child(div().flex().child(editor_button(
            format!("acp-add-{key}"),
            add,
            WebButtonVariant::Outline,
            e.busy,
            cx,
            move |this, w, cx| {
                let f = field("", w, cx);
                f.update(cx, |f, cx| f.focus(w, cx));
                if let Some(e) = &mut this.provider_editor {
                    e.track_edits(&f, cx);
                    if models {
                        e.models.push(f)
                    } else {
                        e.args.push(f)
                    }
                }
                cx.notify();
            },
        )))
        .into_any_element()
    }
}

fn hint(text: &'static str) -> Div {
    div()
        .text_xs()
        .text_color(theme::rambutan_400())
        .child(text)
}

fn editor_field(
    label: &'static str,
    field: &Entity<InputState>,
    disabled: bool,
    wide: bool,
) -> Div {
    described_field(label, None, field, disabled, wide)
}

fn described_field(
    label: &'static str,
    description: Option<&'static str>,
    field: &Entity<InputState>,
    disabled: bool,
    wide: bool,
) -> Div {
    div()
        .flex()
        .gap_2()
        .when(wide, |row| row.items_center().justify_between())
        .when(!wide, |row| row.flex_col())
        .child(
            div()
                .min_w_0()
                .when(wide, |label| label.flex_1())
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_sm().child(label))
                .children(description.map(hint)),
        )
        .child(
            div()
                .when(wide, |el| el.w(px(248.)).flex_shrink_0())
                .child(Input::new(field).aria_label(label).disabled(disabled)),
        )
}

pub(super) fn editor_button(
    id: impl Into<SharedString>,
    label: &'static str,
    variant: WebButtonVariant,
    disabled: bool,
    cx: &Context<'_, Workspace>,
    action: impl Fn(&mut Workspace, &mut Window, &mut Context<'_, Workspace>) + 'static,
) -> AnyElement {
    let id = id.into();
    let button = web_button(
        id.clone(),
        variant,
        WebButtonSize::Small,
        ButtonState::disabled_if(disabled),
    )
    .debug_selector(move || id.to_string())
    .aria_label(label)
    .child(label);
    button
        .when(!disabled, |el| on_press_view(el, cx, action))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{OpenSettings, Page};

    #[gpui::test]
    fn clearing_registry_overrides_restores_the_managed_launcher(cx: &mut gpui::TestAppContext) {
        let (ws, cx, _, _) = super::super::tests::thread_menu_workspace(cx);
        ws.update_in(cx, |_, w, cx| {
            let instance = serde_json::from_value(serde_json::json!({
                "id":"work","name":"Work","command":"/managed/launcher","args":["--acp"],
                "registry":{"agent_id":"fixture","distribution":"npm","version":"1",
                    "executable_override":"my-agent","auth_method_id":"account"}
            }))
            .unwrap();
            let editor = super::ProviderEditor::new(Some(instance), w, cx);
            assert_eq!(editor.value(cx).unwrap().executable(), "my-agent");
            editor
                .registry
                .executable_override
                .update(cx, |f, c| f.replace_all("", w, c));
            editor
                .registry
                .auth_method
                .update(cx, |f, c| f.replace_all("", w, c));
            let value = editor.value(cx).unwrap();
            assert_eq!(value.executable(), "/managed/launcher");
            assert_eq!(value.args, ["--acp"]);
            let origin = value.registry.unwrap();
            assert_eq!(origin.agent_id, "fixture");
            assert!(origin.auth_method_id.is_none() && origin.executable_override.is_none());
        });
    }

    #[gpui::test]
    fn new_instance_id_follows_the_label_until_the_user_enters_one(cx: &mut gpui::TestAppContext) {
        let (ws, cx, _, _) = super::super::tests::thread_menu_workspace(cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.open_settings(&OpenSettings, w, cx);
            ws.open_provider_editor(None, w, cx);
            assert!(
                ws.provider_editor.as_ref().unwrap().registry.entry
                    == super::registry::Entry::Browse
            );
            ws.choose_registry_entry(super::registry::Entry::ById, w, cx);
            ws.submit_provider_editor(w, cx);
            assert_eq!(
                ws.provider_editor.as_ref().unwrap().error.as_deref(),
                Some("Enter an official registry agent ID.")
            );
            ws.choose_registry_entry(super::registry::Entry::Local, w, cx);
            ws.submit_provider_editor(w, cx);
            assert!(ws.provider_editor.as_ref().unwrap().error.is_some());
            ws.provider_editor
                .as_ref()
                .unwrap()
                .command
                .update(cx, |f, cx| f.replace_all("example-acp", w, cx));
            ws.submit_provider_editor(w, cx);
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            let e = ws.provider_editor.as_ref().unwrap();
            assert!(e.page == Page::Details);
            e.name.update(cx, |f, cx| f.replace_all("Work Lab", w, cx));
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            let e = ws.provider_editor.as_ref().unwrap();
            assert_eq!(e.id.read(cx).value().as_ref(), "work-lab");
            e.id.update(cx, |f, cx| f.replace_all("my_agent", w, cx));
            e.name
                .update(cx, |f, cx| f.replace_all("Renamed lab", w, cx));
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            let value = ws.provider_editor.as_ref().unwrap().value(cx).unwrap();
            assert_eq!(value.id, "my_agent");
            assert_eq!(value.name, "Renamed lab");
            ws.dismiss_provider_editor(w, cx);
            assert!(ws.provider_editor.is_none() && ws.settings_visible());
        });
    }
}
