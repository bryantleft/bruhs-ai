//! Configure and check model APIs used for short generated text.
use super::*;
use bruhs_core::secret_sources::{validate_reference, SecretReference};
mod dialog;
use crate::app_state::spawn_tokio;
use bruhs_core::text_generators::{self as api, Draft, Generator, Preset};
use bruhs_ui::dropdown::dropdown;
use bruhs_ui::dropdown::saved_option_value;
use uuid::Uuid;

pub(super) struct GeneratorSettings {
    start: FocusHandle,
    end: FocusHandle,
    session: u64,
    form_events: Vec<Subscription>,
    pub rows: Vec<Generator>,
    pub loaded: bool,
    revision: u64,
    busy: bool,
    notice: Option<String>,
    assignment_test: Option<(&'static str, String, String)>,
    dialog: Option<Page>,
}
impl GeneratorSettings {
    pub(super) fn new(cx: &App) -> Self {
        Self {
            start: cx.focus_handle(),
            end: cx.focus_handle(),
            session: 0,
            form_events: Vec::new(),
            rows: Vec::new(),
            loaded: false,
            revision: 0,
            busy: false,
            notice: None,
            assignment_test: None,
            dialog: None,
        }
    }
    pub(super) fn focus_bounds(&self) -> Option<(FocusHandle, FocusHandle)> {
        self.dialog
            .as_ref()
            .map(|_| (self.start.clone(), self.end.clone()))
    }
}
struct Form {
    previous: Option<Generator>,
    preset: Preset,
    url: Entity<InputState>,
    key: Entity<InputState>,
    name: Entity<InputState>,
    sources: Entity<super::secret_source_settings::SourceSettings>,
    source: Option<String>,
    reference: Entity<InputState>,
    reference_edited: bool,
    show_reference_error: bool,
}
impl Form {
    fn can_connect(&self, cx: &App) -> bool {
        let credential_ready = match &self.source {
            Some(source) => {
                self.sources.read(cx).sources().contains_key(source)
                    && validate_reference(self.reference.read(cx).value().trim()).is_ok()
            }
            None => {
                self.preset == Preset::Custom
                    || !self.key.read(cx).value().trim().is_empty()
                    || self
                        .previous
                        .as_ref()
                        .is_some_and(|row| row.has_key() && row.reference.is_none())
            }
        };
        !self.url.read(cx).value().trim().is_empty() && credential_ready
    }

    fn credential(&self, cx: &App) -> api::CredentialEdit {
        if let Some(source) = &self.source {
            let reference = SecretReference {
                source: source.clone(),
                reference: self.reference.read(cx).value().trim().to_owned(),
            };
            if !self.reference_edited
                && self.previous.as_ref().and_then(|g| g.reference.as_ref()) == Some(&reference)
            {
                return api::CredentialEdit::Keep;
            }
            return api::CredentialEdit::Reference(reference);
        }
        let value = self.key.read(cx).value();
        if value.trim().is_empty() && self.previous.as_ref().is_none_or(|g| g.reference.is_none()) {
            api::CredentialEdit::Keep
        } else {
            api::CredentialEdit::Value(value.trim().to_owned().into())
        }
    }
}
enum Page {
    Edit(Form),
    Connected(Generator),
    Models(Generator, String),
    Remove(Generator),
}

impl Workspace {
    pub(super) fn load_generators(&mut self, cx: &mut Context<'_, Self>) {
        self.generators.revision += 1;
        let revision = self.generators.revision;
        let job = spawn_tokio(cx, api::list());
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |ws, cx| {
                if ws.generators.revision != revision {
                    return;
                }
                match result {
                    Ok(rows) => {
                        ws.generators.rows = rows;
                        ws.generators.loaded = true;
                        ws.stamp_run_prefs_on_threads(cx);
                    }
                    Err(e) => ws.generators.notice = Some(e),
                }
                cx.notify()
            });
        })
        .detach();
    }
    pub(super) fn generator_model(&self, model: &str) -> bool {
        api::split_selection(model).is_some_and(|(id, slug)| {
            self.generators
                .rows
                .iter()
                .any(|g| g.id == id && g.offers(slug))
        })
    }
    pub(super) fn generator_unavailable(&self, selection: &str) -> bool {
        self.generators.loaded
            && api::split_selection(selection).is_some()
            && !self.generator_model(selection)
    }
    pub(super) fn generator_groups(&self) -> Vec<DropdownGroup> {
        self.generators
            .rows
            .iter()
            .filter(|g| g.enabled && !g.models.is_empty())
            .map(|g| DropdownGroup {
                label: Some(saved_option_value(&g.label)),
                mark: None,
                options: g
                    .models
                    .iter()
                    .map(|model| DropdownOption {
                        value: saved_option_value(&g.selection(&model.id)),
                        label: model.name.clone(),
                        description: Some(model.id.clone()),
                    })
                    .collect(),
            })
            .collect()
    }
    pub(super) fn text_model_control(
        &self,
        id: &'static str,
        label: &'static str,
        current: &str,
        defaults: &[(&'static str, &'static str)],
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let mut groups = vec![DropdownGroup {
            label: None,
            mark: None,
            options: defaults
                .iter()
                .map(|&(value, label)| DropdownOption {
                    value,
                    label: label.to_owned(),
                    description: None,
                })
                .collect(),
        }];
        groups.extend(Self::catalog_model_groups(cx, |p| !p.is_external()));
        groups.extend(self.generator_groups());
        let (toggle, select) = self.settings_dropdown_handlers(id, cx);
        dropdown(
            id,
            label,
            current,
            groups,
            self.settings_listbox(id),
            toggle,
            select,
        )
    }
    fn edit_generator(
        &mut self,
        previous: Option<Generator>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let preset = previous.as_ref().map(|g| g.preset).unwrap_or_default();
        let url = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(
                    previous
                        .as_ref()
                        .map_or(preset.url(), |g| g.url.as_str())
                        .to_owned(),
                )
                .placeholder("http://localhost:11434/v1")
        });
        let key = cx.new(|cx| {
            InputState::new(window, cx).masked(true).placeholder(
                if previous
                    .as_ref()
                    .is_some_and(|g| g.has_key() && g.reference.is_none())
                {
                    "Leave blank to keep the saved key"
                } else {
                    "API key"
                },
            )
        });
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(
                    previous
                        .as_ref()
                        .map_or("", |g| g.label.as_str())
                        .to_owned(),
                )
                .placeholder("Name (optional)")
        });
        let sources = self.vault_settings.read(cx).secret_sources();
        sources.update(cx, |s, cx| s.reload(cx));
        let saved = previous.as_ref().and_then(|g| g.reference.as_ref());
        let source = saved.map(|r| r.source.clone());
        let reference = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("op://vault/item/field")
                .default_value(saved.map(|r| r.reference.clone()).unwrap_or_default())
        });
        self.generators.notice = None;
        if preset == Preset::Custom {
            url.focus_handle(cx).focus(window, cx);
        } else if source.is_some() {
            reference.focus_handle(cx).focus(window, cx);
        } else {
            key.focus_handle(cx).focus(window, cx);
        }
        self.generators.form_events = [&url, &key, &name]
            .into_iter()
            .map(|input| {
                cx.subscribe(input, |this, _, event: &InputEvent, cx| match event {
                    InputEvent::Change => cx.notify(),
                    InputEvent::PressEnter { .. } => this.save_generator(cx),
                    _ => {}
                })
            })
            .collect();
        self.generators
            .form_events
            .push(cx.observe(&sources, |_, _, cx| cx.notify()));
        self.generators.form_events.push(cx.subscribe(
            &reference,
            |this, _, event: &InputEvent, cx| {
                let Some(Page::Edit(form)) = &mut this.generators.dialog else {
                    return;
                };
                match event {
                    InputEvent::Change => {
                        form.reference_edited = true;
                        form.show_reference_error = false;
                        this.generators.notice = None;
                    }
                    InputEvent::Blur => form.show_reference_error = true,
                    InputEvent::PressEnter { .. } => {
                        form.show_reference_error = true;
                        this.save_generator(cx);
                    }
                    _ => {}
                }
                cx.notify()
            },
        ));
        self.generators.dialog = Some(Page::Edit(Form {
            sources,
            source,
            reference,
            reference_edited: false,
            show_reference_error: false,
            previous,
            preset,
            url,
            key,
            name,
        }));
        cx.notify()
    }
    pub(super) fn dismiss_generator(&mut self) -> bool {
        if self.generators.dialog.is_none() {
            return false;
        }
        if !self.generators.busy {
            self.reset_generator_dialog();
            self.generators.notice = None;
        }
        true
    }
    pub(super) fn reset_generator_dialog(&mut self) {
        self.generators.session = self.generators.session.wrapping_add(1);
        self.generators.form_events.clear();
        self.generators.assignment_test = None;
        self.generators.dialog = None;
    }
    fn generator_job(
        &mut self,
        job: impl std::future::Future<Output = Result<Option<Page>, String>> + Send + 'static,
        cx: &mut Context<'_, Self>,
    ) {
        if self.generators.busy {
            return;
        }
        self.generators.busy = true;
        self.generators.notice = None;
        let session = self.generators.session;
        let task = spawn_tokio(cx, job);
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |ws, cx| {
                ws.generators.busy = false;
                match result {
                    Ok(page) => {
                        if ws.settings_visible() && ws.generators.session == session {
                            if let Some(Page::Connected(g) | Page::Models(g, _)) = &page {
                                ws.generators.rows.retain(|row| row.id != g.id);
                                ws.generators.rows.push(g.clone());
                            }
                            ws.generators.form_events.clear();
                            ws.generators.dialog = page;
                        }
                        ws.load_generators(cx);
                    }
                    Err(e) => {
                        ws.generators.notice = Some(e);
                        ws.load_generators(cx);
                    }
                }
                cx.notify()
            });
        })
        .detach();
        cx.notify()
    }
    fn save_generator(&mut self, cx: &mut Context<'_, Self>) {
        let Some(Page::Edit(form)) = &self.generators.dialog else {
            return;
        };
        if !form.can_connect(cx) || self.generators.busy {
            return;
        }
        let previous = form.previous.clone();
        let draft = Draft {
            preset: form.preset,
            label: form.name.read(cx).value().to_string(),
            url: form.url.read(cx).value().to_string(),
            credential: form.credential(cx),
        };
        self.generator_job(
            async move {
                api::connect(previous, draft)
                    .await
                    .map(|g| Some(Page::Connected(g)))
            },
            cx,
        );
    }
    pub(super) fn select_generator_option(
        &mut self,
        id: &str,
        value: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if let Some(raw) = id.strip_prefix("generator-actions-") {
            let Some(g) = Uuid::parse_str(raw)
                .ok()
                .and_then(|id| self.generators.rows.iter().find(|g| g.id == id))
                .cloned()
            else {
                return true;
            };
            self.generators.notice = None;
            match value {
                "edit" => self.edit_generator(Some(g), window, cx),
                "models" => {
                    self.generators.start.focus(window, cx);
                    let selected = g.models.first().map(|m| m.id.clone()).unwrap_or_default();
                    self.generators.dialog = Some(Page::Models(g, selected));
                }
                "remove" => {
                    self.generators.start.focus(window, cx);
                    self.generators.dialog = Some(Page::Remove(g));
                }
                _ => {}
            }
        } else {
            match id {
                "generator-key-source" => {
                    if self.generators.busy {
                        return true;
                    }
                    let Some(Page::Edit(form)) = &mut self.generators.dialog else {
                        return true;
                    };
                    let source = value.strip_prefix("source:");
                    if source.is_some_and(|id| !form.sources.read(cx).sources().contains_key(id)) {
                        return true;
                    }
                    if source != form.source.as_deref() {
                        form.source = source.map(str::to_owned);
                        let saved = form
                            .previous
                            .as_ref()
                            .and_then(|g| g.reference.as_ref())
                            .filter(|r| Some(r.source.as_str()) == source);
                        form.reference.update(cx, |f, cx| {
                            f.set_value(
                                saved.map(|r| r.reference.clone()).unwrap_or_default(),
                                window,
                                cx,
                            )
                        });
                        form.reference_edited = false;
                        form.show_reference_error = false;
                        self.generators.notice = None;
                    }
                    let focus = if source.is_some() {
                        form.reference.focus_handle(cx)
                    } else {
                        form.key.focus_handle(cx)
                    };
                    cx.defer_in(window, move |_, w, cx| focus.focus(w, cx));
                }
                "generator-preset" => {
                    let Some(Page::Edit(form)) = &mut self.generators.dialog else {
                        return true;
                    };
                    let Some(preset) = Preset::ALL.into_iter().find(|p| p.label() == value) else {
                        return true;
                    };
                    if preset != form.preset {
                        form.preset = preset;
                        form.url
                            .update(cx, |f, cx| f.set_value(preset.url(), window, cx));
                        form.key.update(cx, |f, cx| f.set_value("", window, cx));
                        if preset == Preset::Custom {
                            form.url.focus_handle(cx).focus(window, cx);
                        } else if form.source.is_some() {
                            form.reference.focus_handle(cx).focus(window, cx);
                        } else {
                            form.key.focus_handle(cx).focus(window, cx);
                        }
                        self.generators.notice = None;
                    }
                }
                "generator-test-model" => {
                    if let Some(Page::Models(_, slug)) = &mut self.generators.dialog {
                        *slug = value.to_owned();
                        self.generators.notice = None;
                    }
                }
                "generator-assign-title" | "generator-assign-writer" | "generator-assign-voice" => {
                    if id == "generator-assign-writer"
                        && self.loaded_delivery_preferences().is_none()
                    {
                        return true;
                    }
                    if !self.generator_model(value)
                        && !bruhs_models::agent::model_info(value)
                            .is_some_and(|m| !m.provider.is_external())
                        && !(id == "generator-assign-title" && value == "local")
                        && value != "default"
                    {
                        return true;
                    }
                    let model = (value != "default").then(|| value.to_owned());
                    match id {
                        "generator-assign-title" => self.text_generation_model = model,
                        "generator-assign-writer" => {
                            use bruhs_models::delivery::{DeliveryPreferenceChange, WriterModel};
                            self.queue_delivery_change(
                                "",
                                DeliveryPreferenceChange::WriterModel(Some(
                                    model
                                        .map(|model| WriterModel::Separate {
                                            model,
                                            options: Default::default(),
                                        })
                                        .unwrap_or(WriterModel::TextGeneration),
                                )),
                                cx,
                            );
                        }
                        _ => self.dictation_prefs.cleanup.model = model,
                    }
                    self.stamp_run_prefs_on_threads(cx);
                    self.persist(cx);
                }
                _ => return false,
            }
        }
        cx.notify();
        true
    }
    fn gen_button(
        &self,
        id: &'static str,
        label: &'static str,
        primary: bool,
        cx: &Context<'_, Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<'_, Self>) + 'static,
    ) -> AnyElement {
        let disabled = self.generators.busy
            || (id == "generator-connect"
                && matches!(&self.generators.dialog,Some(Page::Edit(f)) if !f.can_connect(cx)));
        let button = web_button(
            id,
            if primary {
                WebButtonVariant::Primary
            } else {
                WebButtonVariant::Outline
            },
            WebButtonSize::Small,
            ButtonState::disabled_if(disabled),
        )
        .aria_label(label)
        .debug_selector(move || id.into())
        .child(label);
        if disabled {
            button.into_any_element()
        } else {
            on_press_view(button, cx, action).into_any_element()
        }
    }
    pub(super) fn generators_card(&self, cx: &Context<'_, Self>) -> AnyElement {
        let mut rows = vec![self.settings_row(
            "settings-row-text-generators",
            "Model API connections",
            "Connect a model API for thread titles, source control writing and voice cleanup.",
            self.gen_button("generator-add", "Add generator", false, cx, |ws, w, cx| {
                ws.edit_generator(None, w, cx)
            }),
        )];
        if self.generators.loaded && self.generators.rows.is_empty() {
            rows.push(
                div()
                    .px_4()
                    .py_3()
                    .text_color(theme::rambutan_400())
                    .child("No text generators yet")
                    .into_any_element(),
            );
        }
        for g in &self.generators.rows {
            let row = g.clone();
            let action = saved_option_value(&format!("generator-actions-{}", g.id));
            let enabled = row.enabled;
            let control = div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().w(px(106.)).child(self.choice_control(
                    action,
                    "Generator actions",
                    "Actions",
                    &[
                        ("edit", "Edit"),
                        ("models", "Show models"),
                        ("remove", "Remove"),
                    ],
                    cx,
                )))
                .child(settings_ui::switch(
                    saved_option_value(&format!("generator-enabled-{}", g.id)),
                    enabled,
                    self.generators.busy,
                    saved_option_value(&format!("Enable {}", g.label)),
                    cx,
                    move |ws, _, cx| {
                        let row = row.clone();
                        ws.generator_job(
                            async move { api::enable(row, !enabled).await.map(|_| None) },
                            cx,
                        )
                    },
                ));
            rows.push(self.settings_row(
                saved_option_value(&format!("generator-row-{}", g.id)),
                g.label.clone(),
                g.url.clone(),
                control.into_any_element(),
            ));
        }
        if self.generators.dialog.is_none() {
            if let Some(note) = &self.generators.notice {
                rows.push(
                    div()
                        .id("generator-list-status")
                        .px_4()
                        .py_2()
                        .role(Role::Status)
                        .child(note.clone())
                        .into_any_element(),
                );
            }
        }
        self.settings_card("Text generators", "", rows)
    }
    fn test_generator(
        &mut self,
        row: Generator,
        slug: String,
        assignment: Option<&'static str>,
        cx: &mut Context<'_, Self>,
    ) {
        if self.generators.busy {
            return;
        }
        self.generators.busy = true;
        let model = row.selection(&slug);
        let session = self.generators.session;
        if let Some(id) = assignment {
            self.generators.assignment_test = Some((id, model.clone(), "Testing model…".into()));
        } else {
            self.generators.notice = Some("Testing model…".into());
        }
        let task = spawn_tokio(cx, api::test(row, slug.clone()));
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |ws, cx| {
                ws.generators.busy = false;
                if ws.generators.session == session {
                    if let Some(id) = assignment {
                        ws.generators.assignment_test = Some((
                            id,
                            model,
                            result
                                .map(|_| format!("{slug} answered."))
                                .unwrap_or_else(|error| error),
                        ));
                    } else {
                        ws.generators.notice = Some(
                            result
                                .map(|_| "Model responded successfully.".to_owned())
                                .unwrap_or_else(|error| error),
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
